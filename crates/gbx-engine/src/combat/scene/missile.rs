//! **Missile flight** (`docs/design/combat-visualizer.md` §1.4): the
//! weapon-class table `DrawRangedAttack` picks its animation from, and the
//! 8-pixel sub-cell flight `draw_missile_attack` walks it along.
//!
//! Both live scene-side by design. [`ActionEvent::Missile`] carries the raw
//! ITEMS **type** rather than a class (slice 2's payload note): the class is a
//! pure function of the type, but the table that maps one to the other picks
//! the COMSPR slot, the frame count, the per-step delay *and* the launch
//! sound — all four presentation — so it belongs here with the animation it
//! feeds, not in the combat core.
//!
//! The **camera** is the subtle half. `facing.rs`'s [`draw_missile_camera`]
//! models only the *persistent* `mapScreenTopLeft` effect, which is all the
//! engine may carry; the flight's transient window (the initial force-recentre
//! and the mid-flight re-scroll) never exists in engine state, so it is
//! recomputed here — presentation-locally, from the presented endpoints and
//! the presented camera. That is legal because it draws nothing from
//! `CombatState` and rolls no dice (doc §1.4, review finding).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr014.cs:1590-1671` (`DrawRangedAttack`) — the class switch: the
//!   unconditional `sound_c` at `:1592`, then per class the icon loads, frame
//!   count, delay and second sound.
//! - `engine/ovr025.cs:873-879` (`load_missile_icons`) — the four-frame buffer
//!   {Normal, Normal-flipped, Attack-flipped, Attack}.
//! - `engine/ovr025.cs:849-871` (`load_missile_dax`) — the single-frame loads
//!   the arrow class and the sling use.
//! - `engine/ovr025.cs:882-1115` (`draw_missile_attack`) — the path, the
//!   8-pixel step loop, the bounds test, and the phase-2 re-scroll.
//! - `Classes/SteppingPath.cs` (`sub_7324C`/`sub_731A5`) — the Bresenham
//!   stepper and its `directions` lookup.
//! - `Classes/ItemData.cs:120` — the `ItemType` ordinals the switch reads.
//! - `engine/seg040.cs:29-31` (`OverlayBounded`) — `rowY + 1`/`colX + 1`, i.e.
//!   the overlay's cell coordinates are 8-pixel units off the viewport origin.
//!
//! [`draw_missile_camera`]: crate::combat::CombatState

use crate::combat::{
    map_dir_delta, scrolled_top_left, GridPos, MAP_H, MAP_W, SCREEN_HALF, SCREEN_MAX,
};
use crate::combat_art::IconPose;

/// One frame of the missile buffer, as a reference into the icon store rather
/// than as pixels — `gbl.missile_dax` is filled by copying (and flipping)
/// frames out of `gbl.combat_icons`, so naming the source is lossless and
/// keeps the scene allocation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpriteRef {
    pub icon_slot: usize,
    pub pose: IconPose,
    /// `FlipIconLeftToRight` — the same cached mirror `CombatIcon::frame`
    /// hands out for directions 4–7.
    pub mirrored: bool,
}

impl SpriteRef {
    pub const fn new(icon_slot: usize, pose: IconPose, mirrored: bool) -> Self {
        SpriteRef {
            icon_slot,
            pose,
            mirrored,
        }
    }

    /// The `direction` argument that selects this frame's mirroring out of a
    /// [`CombatIcon`](crate::combat_art::CombatIcon).
    pub fn direction(&self) -> u8 {
        if self.mirrored {
            4
        } else {
            0
        }
    }
}

/// The first COMSPR icon slot (`iconId = 13` in `DrawRangedAttack`), i.e.
/// `combat_art::COMSPR_FIRST_SLOT`.
const MISSILE_BASE_SLOT: usize = 0x0D;

/// The unconditional launch sound at `DrawRangedAttack`'s head
/// (`ovr014.cs:1592`) — played for **every** class, before the switch picks a
/// second one. The arrow class's own sound is the same id, so an arrow really
/// does play `0x0C` twice; that is the original, not a transcription slip.
pub const LAUNCH_SOUND: u8 = 0x0C;

/// `ItemType` ordinals (`Classes/ItemData.cs:120`) the class switch names.
mod item {
    pub const HAND_AXE: u8 = 0x02;
    pub const CLUB: u8 = 0x07;
    pub const DART: u8 = 0x09;
    pub const GLAIVE: u8 = 0x0E;
    pub const JAVELIN: u8 = 0x15;
    pub const QUARREL: u8 = 0x1C;
    pub const SPEAR: u8 = 0x1F;
    pub const SLING: u8 = 0x2F;
    pub const ARROW: u8 = 0x49;
    pub const TYPE_85: u8 = 0x55;
    pub const FLASK_OF_OIL: u8 = 0x56;
    pub const SPINE: u8 = 0x62;
    pub const DART_OF_HORNETS_NEST: u8 = 0x64;
    pub const STAFF_SLING: u8 = 0x65;
}

/// One row of `DrawRangedAttack`'s switch — everything the flight needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissileClass {
    /// The missile buffer, in `load_missile_dax` offset order. Only the first
    /// [`frame_count`](Self::frame_count) entries are cycled.
    pub frames: Vec<SpriteRef>,
    /// `frameCount` — how many buffer slots the flight cycles through.
    pub frame_count: usize,
    /// `delay` — milliseconds held per 8-pixel step.
    pub step_ms: u32,
    /// The class's own `PlaySound`, after the unconditional [`LAUNCH_SOUND`].
    pub class_sound: u8,
}

/// `DrawRangedAttack`'s class switch (`ovr014.cs:1598-1665`), for a shot whose
/// heading is `dir` (`getTargetDirection(target, attacker)`).
///
/// The arrow class is the only one whose art depends on the heading: even
/// directions take a per-axis COMSPR block unflipped (Normal for 0–3, Attack
/// for 4–7), odd ones all take block `iconId + 1` with the Normal/Attack and
/// flip both chosen by the diagonal — so the four diagonals share one pair of
/// frames between them.
pub fn missile_class(item_type: u8, dir: u8) -> MissileClass {
    let base = MISSILE_BASE_SLOT;
    match item_type {
        item::DART
        | item::JAVELIN
        | item::DART_OF_HORNETS_NEST
        | item::QUARREL
        | item::SPEAR
        | item::ARROW => {
            // `ovr014.cs:1604-1630`.
            let frame = if dir & 1 == 1 {
                if dir == 3 || dir == 5 {
                    SpriteRef::new(base + 1, IconPose::Attack, dir == 5)
                } else {
                    SpriteRef::new(base + 1, IconPose::Normal, dir == 7)
                }
            } else if dir >= 4 {
                SpriteRef::new(base + (dir as usize % 4), IconPose::Attack, false)
            } else {
                SpriteRef::new(base + (dir as usize % 4), IconPose::Normal, false)
            };
            MissileClass {
                frames: vec![frame],
                frame_count: 1,
                step_ms: 10,
                class_sound: LAUNCH_SOUND,
            }
        }
        // The 4-frame spin (`ovr014.cs:1633-1640`).
        item::HAND_AXE | item::CLUB | item::GLAIVE => MissileClass {
            frames: spin_frames(base + 3),
            frame_count: 4,
            step_ms: 50,
            class_sound: 9,
        },
        // Same spin, different block and sound (`ovr014.cs:1642-1648`).
        item::TYPE_85 | item::FLASK_OF_OIL => MissileClass {
            frames: spin_frames(base + 4),
            frame_count: 4,
            step_ms: 50,
            class_sound: 6,
        },
        // The sling's two-frame tumble (`ovr014.cs:1650-1658`). Note the
        // `iconId++` **before** the `iconId + 7`: the sling reads block
        // `0x15`, one past the default arm's `0x14`.
        item::STAFF_SLING | item::SLING | item::SPINE => MissileClass {
            frames: two_frames(base + 1 + 7),
            frame_count: 2,
            step_ms: 10,
            class_sound: 6,
        },
        // The default arm (`ovr014.cs:1661-1666`) — a 20 ms two-frame tumble
        // off block `0x14`, with the axe class's sound.
        _ => MissileClass {
            frames: two_frames(base + 7),
            frame_count: 2,
            step_ms: 20,
            class_sound: 9,
        },
    }
}

/// `load_missile_icons(slot)` (`ovr025.cs:873-879`) — the four-frame spin
/// buffer {Normal, Normal-flipped, Attack-flipped, Attack}.
///
/// coab's `load_missile_dax` flip arm calls `FlipIconLeftToRight()` and
/// discards the result instead of copying it into the buffer — a
/// transliteration artifact of that C# port, not a behavior we can adopt (it
/// would leave two of the four frames stale). Transcribed here as the flip the
/// buffer's own comment describes, which is also what the doc's §1.4 note says
/// the buffer holds.
fn spin_frames(slot: usize) -> Vec<SpriteRef> {
    (0..4).map(|i| spin_frame(slot, i)).collect()
}

/// One slot of a [`spin_frames`] buffer, for callers that build the buffer
/// themselves (the on-target burst loads the same four frames off icon 0x16 or
/// 0x17, `ovr025.cs:1123`).
pub fn spin_frame(slot: usize, frame: usize) -> SpriteRef {
    match frame % 4 {
        0 => SpriteRef::new(slot, IconPose::Normal, false),
        1 => SpriteRef::new(slot, IconPose::Normal, true),
        2 => SpriteRef::new(slot, IconPose::Attack, true),
        _ => SpriteRef::new(slot, IconPose::Attack, false),
    }
}

/// The sling/default two-frame load (`ovr014.cs:1654-1655,1662-1663`): buffer
/// slot 0 = the block's Normal frame, slot 1 = its Attack frame, neither
/// flipped.
fn two_frames(slot: usize) -> Vec<SpriteRef> {
    vec![
        SpriteRef::new(slot, IconPose::Normal, false),
        SpriteRef::new(slot, IconPose::Attack, false),
    ]
}

/// The spell projectile's buffer (`load_missile_icons(0x12)`,
/// `ovr023.cs:741`) with its 30 ms step (`draw_missile_attack(0x1E, 4, …)`,
/// `ovr023.cs:762`).
pub fn spell_projectile_class() -> MissileClass {
    MissileClass {
        frames: spin_frames(0x12),
        frame_count: 4,
        step_ms: 30,
        // The cast sound is per-spell and rides with `Cast`'s `spell_id`
        // (see [`cast_sound`]); the projectile itself plays none of its own.
        class_sound: 0,
    }
}

/// The lightning bolt's buffer (`load_missile_icons(0x13)`,
/// `ovr023.cs:1996`) with its 50 ms step (`draw_missile_attack(0x32, 4, …)`,
/// `ovr023.cs:2052`). Unreached today — no modeled spell is a bolt — but the
/// row is §1.4's and costs one function to keep beside its sibling.
pub fn lightning_class() -> MissileClass {
    MissileClass {
        frames: spin_frames(0x13),
        frame_count: 4,
        step_ms: 50,
        class_sound: 0,
    }
}

/// The cast sound (`sub_5D2E1`, `ovr023.cs:749-758`): `0x0B` for Fireball
/// (0x2F), `8` for Lightning Bolt (0x33), `2` for everything else.
pub fn cast_sound(spell_id: u8) -> u8 {
    match spell_id {
        0x2F => 0x0B,
        0x33 => 8,
        _ => 2,
    }
}

/// One drawn moment of a flight, in emission order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightStep {
    /// `redrawCombatArea(8, 0xFF, centre)` — the window jumps (no scroll
    /// animation, §1.2) and the whole board repaints under the missile.
    Camera { top_left: GridPos },
    /// The buffer's `frame` is blitted at this **8-pixel cell** offset from
    /// the viewport origin, and held for the class's `step_ms`.
    Frame {
        cell_x: i32,
        cell_y: i32,
        frame: usize,
    },
}

/// The `SteppingPath` direction table (`SteppingPath.cs:87`), indexed by
/// `index_y * 3 + index_x` with each index `sign + 1`. `8` is "no step" and
/// [`map_dir_delta`] gives it `(0, 0)`.
const STEP_DIRECTIONS: [u8; 10] = [7, 0, 1, 6, 8, 2, 5, 4, 3, 8];

/// `SteppingPath` over the ×3 sub-cell grid (`CalculateDeltas` + `Step`), as
/// the per-step heading list `draw_missile_attack` fills `pathDir` with.
///
/// The returned vector has the original's `var_AF` length — one entry per
/// `Step()` call **including** the final non-stepping one, whose heading is
/// the table's `8`. The flight consumes only the first `len − 2` (`var_B0`).
pub fn path_directions(attacker: GridPos, target: GridPos) -> Vec<u8> {
    let (ax, ay) = (attacker.x * 3, attacker.y * 3);
    let (tx, ty) = (target.x * 3, target.y * 3);
    let diff_x = (tx - ax).abs();
    let diff_y = (ty - ay).abs();
    let sign_x = (tx - ax).signum();
    let sign_y = (ty - ay).signum();
    let (mut cx, mut cy) = (ax, ay);
    let mut delta_count = 0i32;
    let mut dirs = Vec::new();
    loop {
        let (mut index_x, mut index_y) = (1i32, 1i32);
        let mut step_made = false;
        if diff_x >= diff_y {
            if cx != tx {
                cx += sign_x;
                delta_count += diff_y * 2;
                index_x = sign_x + 1;
                if delta_count >= diff_x {
                    cy += sign_y;
                    delta_count -= diff_x * 2;
                    index_y = sign_y + 1;
                }
                step_made = true;
            }
        } else if cy != ty {
            cy += sign_y;
            delta_count += diff_x * 2;
            index_y = sign_y + 1;
            if delta_count >= diff_y {
                cx += sign_x;
                delta_count -= diff_y * 2;
                index_x = sign_x + 1;
            }
            step_made = true;
        }
        dirs.push(STEP_DIRECTIONS[(index_y * 3 + index_x) as usize]);
        if !step_made {
            break;
        }
    }
    dirs
}

/// The largest 8-pixel cell coordinate a missile may sit at before the flight
/// calls it off-screen (`cur.x > 0x12`, `ovr025.cs:970`): 6 cells × 3.
const MAX_SUBCELL: i32 = 0x12;

/// `draw_missile_attack(delay, frameCount, target, attacker)`
/// (`ovr025.cs:882-1115`) as a schedule: the window jumps and sprite
/// placements, in order, for a flight from `attacker` to `target` seen through
/// the window at `camera`.
///
/// The shape, faithfully:
///
/// 1. A path shorter than four sub-steps draws **nothing at all**
///    (`var_B0 < 2 || var_AF < 2`, `:912`) — and scrolls nothing either, which
///    is the early return `draw_missile_camera` already mirrors.
/// 2. One force-recentre (`:940`) to `center`: the current centre when both
///    endpoints are on screen (a no-op jump), the **midpoint** when one is off
///    but the span is ≤ 6 on both axes, and again the current centre when the
///    span is wider — in that last case the pan that matters comes at step 4.
/// 3. Phase 1 walks `var_B0` sub-steps over a **static** window, blitting the
///    next buffer frame at each and holding `delay`. (`attacker`/`center`
///    track which cell the missile is over, but nothing redraws from them
///    here; they are the state phase 2 restarts from.) The walk stops early if
///    the missile leaves the 0..0x12 box.
/// 4. If it stopped early, the window jumps to a **target-anchored** centre
///    (`:1030` — the same `var_CE`/`var_D0` clamp `draw_missile_camera`
///    ports) and the tail of the path is walked backwards from the target to
///    find where the missile re-enters; that backward walk draws nothing.
/// 5. Otherwise the missile lands: if the target is off screen the window
///    recentres on it (`radius 3`, `:1089`), and one last frame is held at the
///    target cell (`:1096-1105`).
///
/// The `var_B3` loop (`:1109`) can only re-run when the span was wider than 6
/// AND the flight completed anyway, which the geometry forbids — the path
/// stays inside the endpoints' bounding box, so a > 6 span always leaves the
/// 7-cell window. It is transcribed as the single pass it always takes.
pub fn plan_flight(
    attacker: GridPos,
    target: GridPos,
    camera: GridPos,
    class: &MissileClass,
) -> Vec<FlightStep> {
    let dirs = path_directions(attacker, target);
    let var_af = dirs.len() as i32;
    let var_b0 = var_af - 2;
    if var_b0 < 2 || var_af < 2 {
        return Vec::new();
    }

    let mut steps = Vec::new();
    let on_screen = |cam: GridPos, p: GridPos| {
        let (sx, sy) = (p.x - cam.x, p.y - cam.y);
        (0..=SCREEN_MAX).contains(&sx) && (0..=SCREEN_MAX).contains(&sy)
    };

    // Step 2 — `center` and its force-recentre.
    let diff = GridPos::new(target.x - attacker.x, target.y - attacker.y);
    let short_span = diff.x.abs() <= 6 && diff.y.abs() <= 6;
    let center = if !on_screen(camera, attacker) || !on_screen(camera, target) {
        if short_span {
            GridPos::new(diff.x / 2 + attacker.x, diff.y / 2 + attacker.y)
        } else {
            screen_centre(camera)
        }
    } else {
        screen_centre(camera)
    };
    let mut cam = camera;
    if let Some(top_left) = scrolled_top_left(cam, 0xFF, center) {
        if top_left != cam {
            cam = top_left;
            steps.push(FlightStep::Camera { top_left });
        }
    }

    // Step 3 — the phase-1 walk.
    let mut cur_x = (attacker.x - cam.x) * 3;
    let mut cur_y = (attacker.y - cam.y) * 3;
    let mut frame = 0usize;
    let mut idx = 0i32;
    let mut left_window = false;
    while idx < var_b0 && !left_window {
        let (dx, dy) = map_dir_delta(dirs[idx as usize]);
        cur_x += dx;
        cur_y += dy;
        // `delay > 0 || cur.x % 3 == 0 || cur.y % 3 == 0` (`:952`): every class
        // in the table has a positive delay, so the guard is always true — kept
        // as the original's, not folded away.
        if class.step_ms > 0 || cur_x % 3 == 0 || cur_y % 3 == 0 {
            steps.push(FlightStep::Frame {
                cell_x: cur_x,
                cell_y: cur_y,
                frame,
            });
            frame = (frame + 1) % class.frame_count.max(1);
        }
        idx += 1;
        if !(0..=MAX_SUBCELL).contains(&cur_x) || !(0..=MAX_SUBCELL).contains(&cur_y) {
            left_window = true;
        }
    }

    if idx < var_b0 {
        // Step 4 — the target-anchored re-scroll. The backward walk that
        // follows it draws nothing, so only its window jump is presentable.
        let center = target_anchored_centre(target);
        if let Some(top_left) = scrolled_top_left(cam, 0xFF, center) {
            if top_left != cam {
                steps.push(FlightStep::Camera { top_left });
            }
        }
        return steps;
    }

    // Step 5 — the landing.
    if !on_screen(cam, target) {
        if let Some(top_left) = scrolled_top_left(cam, 3, target) {
            if top_left != cam {
                cam = top_left;
                steps.push(FlightStep::Camera { top_left });
            }
        }
    }
    steps.push(FlightStep::Frame {
        cell_x: (target.x - cam.x) * 3,
        cell_y: (target.y - cam.y) * 3,
        frame,
    });
    steps
}

/// `mapScreenTopLeft + Point.ScreenCenter`.
fn screen_centre(camera: GridPos) -> GridPos {
    GridPos::new(camera.x + SCREEN_HALF, camera.y + SCREEN_HALF)
}

/// `center2` (`ovr025.cs:1010-1030`): the target, pushed back in-bounds so the
/// window that centres on it stays on the map. Identical arithmetic to
/// `draw_missile_camera`'s port — this is the presentation half of the same
/// site, and both must agree or the boundary reconcile fails.
fn target_anchored_centre(target: GridPos) -> GridPos {
    let mut var_ce = 0;
    if target.x + SCREEN_HALF > MAP_W {
        var_ce = target.x - MAP_W;
    } else if target.x < SCREEN_HALF {
        var_ce = SCREEN_HALF - target.x;
    }
    let mut var_d0 = 0;
    if target.y + SCREEN_HALF > MAP_H {
        var_d0 = target.y - MAP_H;
    } else if target.y < SCREEN_HALF {
        var_d0 = SCREEN_HALF - target.y;
    }
    GridPos::new(target.x + var_ce, target.y + var_d0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrow_class_picks_its_frame_from_the_heading() {
        // Even headings: one unflipped block per axis, Normal below 4 and
        // Attack at or above it (`ovr014.cs:1621-1629`).
        for (dir, slot, pose) in [
            (0u8, 0x0D, IconPose::Normal),
            (2, 0x0F, IconPose::Normal),
            (4, 0x0D, IconPose::Attack),
            (6, 0x0F, IconPose::Attack),
        ] {
            let c = missile_class(item::ARROW, dir);
            assert_eq!(c.frame_count, 1);
            assert_eq!(c.frames[0], SpriteRef::new(slot, pose, false), "dir {dir}");
        }
        // Odd headings all share block 0x0E, flipping the west-facing pair.
        for (dir, pose, mirrored) in [
            (1u8, IconPose::Normal, false),
            (3, IconPose::Attack, false),
            (5, IconPose::Attack, true),
            (7, IconPose::Normal, true),
        ] {
            let c = missile_class(item::QUARREL, dir);
            assert_eq!(
                c.frames[0],
                SpriteRef::new(0x0E, pose, mirrored),
                "dir {dir}"
            );
        }
    }

    #[test]
    fn each_class_carries_its_own_frames_delay_and_sound() {
        let axe = missile_class(item::HAND_AXE, 0);
        assert_eq!((axe.frame_count, axe.step_ms, axe.class_sound), (4, 50, 9));
        assert_eq!(axe.frames.len(), 4, "the spin buffer is filled whole");
        assert!(axe.frames[1].mirrored && axe.frames[2].mirrored);
        assert_eq!(axe.frames[0].pose, IconPose::Normal);
        assert_eq!(axe.frames[3].pose, IconPose::Attack);

        let oil = missile_class(item::FLASK_OF_OIL, 0);
        assert_eq!((oil.frame_count, oil.step_ms, oil.class_sound), (4, 50, 6));
        assert_eq!(oil.frames[0].icon_slot, 0x11, "block 0x0D + 4");

        // The sling's `iconId++` puts it one block past the default arm.
        let sling = missile_class(item::SLING, 0);
        assert_eq!(
            (sling.frame_count, sling.step_ms, sling.class_sound),
            (2, 10, 6)
        );
        assert_eq!(sling.frames[0].icon_slot, 0x15);
        let default = missile_class(0xEE, 0);
        assert_eq!(
            (default.frame_count, default.step_ms, default.class_sound),
            (2, 20, 9)
        );
        assert_eq!(default.frames[0].icon_slot, 0x14);

        // The arrow class's own sound is the same id the head already played.
        assert_eq!(missile_class(item::ARROW, 0).class_sound, LAUNCH_SOUND);
    }

    #[test]
    fn the_path_walks_the_sub_cell_grid_and_ends_with_the_no_step_heading() {
        let dirs = path_directions(GridPos::new(10, 10), GridPos::new(13, 10));
        // Nine sub-cells due east, plus the terminating non-step.
        assert_eq!(dirs.len(), 10);
        assert!(dirs[..9].iter().all(|&d| d == 2), "east: {dirs:?}");
        assert_eq!(dirs[9], 8, "the last Step() makes no step");
    }

    #[test]
    fn the_path_length_agrees_with_the_engines_own_step_count() {
        // `missile_path_pixel_steps` is `facing.rs`'s copy of the same walk —
        // the camera port counts `var_AF` with it, and the flight indexes
        // `pathDir` with this one. They must not drift apart.
        for (a, t) in [
            ((10, 10), (13, 10)),
            ((10, 10), (10, 14)),
            ((20, 12), (26, 15)),
            ((30, 20), (24, 9)),
            ((5, 5), (5, 5)),
        ] {
            let a = GridPos::new(a.0, a.1);
            let t = GridPos::new(t.0, t.1);
            assert_eq!(
                path_directions(a, t).len(),
                crate::combat::missile_path_pixel_steps(a, t),
                "{a:?} → {t:?}"
            );
        }
    }

    #[test]
    fn a_too_short_flight_draws_nothing() {
        // Adjacent cells are three sub-steps: `var_AF` 4, `var_B0` 2 — just
        // long enough. One sub-cell apart is not, and neither is a zero path.
        let class = missile_class(item::ARROW, 2);
        let same = plan_flight(
            GridPos::new(10, 10),
            GridPos::new(10, 10),
            GridPos::new(7, 7),
            &class,
        );
        assert!(same.is_empty(), "a zero-length path never draws");
    }

    #[test]
    fn a_flight_across_the_window_holds_one_frame_per_sub_step() {
        // Both endpoints on screen: the force-recentre is a no-op, so every
        // step is a frame and the last one is the landing.
        let camera = GridPos::new(17, 9);
        let attacker = GridPos::new(18, 12);
        let target = GridPos::new(22, 12);
        let class = missile_class(item::ARROW, 2);
        let steps = plan_flight(attacker, target, camera, &class);
        assert!(
            !steps.iter().any(|s| matches!(s, FlightStep::Camera { .. })),
            "an on-screen flight never scrolls: {steps:?}"
        );
        // `var_B0` = 12 sub-steps east, then the landing frame.
        let dirs = path_directions(attacker, target);
        assert_eq!(steps.len(), (dirs.len() - 2) + 1);
        // It starts one sub-cell east of the attacker's own cell and walks.
        assert_eq!(
            steps[0],
            FlightStep::Frame {
                cell_x: (attacker.x - camera.x) * 3 + 1,
                cell_y: (attacker.y - camera.y) * 3,
                frame: 0,
            }
        );
        // A one-frame class re-selects frame 0 every step.
        assert!(steps
            .iter()
            .all(|s| !matches!(s, FlightStep::Frame { frame, .. } if *frame != 0)));
        assert_eq!(
            steps.last(),
            Some(&FlightStep::Frame {
                cell_x: (target.x - camera.x) * 3,
                cell_y: (target.y - camera.y) * 3,
                frame: 0,
            }),
            "the landing sits on the target cell"
        );
    }

    #[test]
    fn a_spin_class_cycles_its_four_frames() {
        let camera = GridPos::new(17, 9);
        let class = missile_class(item::HAND_AXE, 2);
        let steps = plan_flight(GridPos::new(18, 12), GridPos::new(22, 12), camera, &class);
        let frames: Vec<usize> = steps
            .iter()
            .filter_map(|s| match s {
                FlightStep::Frame { frame, .. } => Some(*frame),
                _ => None,
            })
            .collect();
        assert_eq!(&frames[..8], &[0, 1, 2, 3, 0, 1, 2, 3]);
    }

    #[test]
    fn an_off_screen_endpoint_within_six_cells_recentres_on_the_midpoint() {
        // The attacker is on screen, the target two cells past the right edge.
        let camera = GridPos::new(10, 10);
        let attacker = GridPos::new(12, 13);
        let target = GridPos::new(18, 13);
        let class = missile_class(item::ARROW, 2);
        let steps = plan_flight(attacker, target, camera, &class);
        let midpoint = GridPos::new(15, 13);
        assert_eq!(
            steps[0],
            FlightStep::Camera {
                top_left: crate::combat::scrolled_top_left(camera, 0xFF, midpoint).unwrap(),
            },
            "the window jumps to the midpoint first: {steps:?}"
        );
        // ...and the same window the engine's own camera port lands on.
        assert!(
            steps
                .iter()
                .filter(|s| matches!(s, FlightStep::Camera { .. }))
                .count()
                == 1,
            "one jump for a flight that stays in the new window: {steps:?}"
        );
    }

    #[test]
    fn a_span_wider_than_six_pans_to_the_target_mid_flight() {
        let camera = GridPos::new(10, 10);
        let attacker = GridPos::new(11, 13);
        let target = GridPos::new(30, 13);
        let class = missile_class(item::ARROW, 2);
        let steps = plan_flight(attacker, target, camera, &class);
        // Phase 1 runs on the original window (its force-recentre is a no-op
        // jump to the current centre) and the missile flies off the right edge.
        let cameras: Vec<GridPos> = steps
            .iter()
            .filter_map(|s| match s {
                FlightStep::Camera { top_left } => Some(*top_left),
                _ => None,
            })
            .collect();
        assert_eq!(cameras.len(), 1, "only the phase-2 pan: {steps:?}");
        assert_eq!(
            cameras[0],
            crate::combat::scrolled_top_left(camera, 0xFF, target_anchored_centre(target)).unwrap(),
        );
        assert!(
            matches!(steps.last(), Some(FlightStep::Camera { .. })),
            "the backward walk draws nothing, so the pan ends the schedule"
        );
    }
}
