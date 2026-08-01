//! Pixel goldens for the combat scene (D-CV4/D-UI7), on **synthetic
//! hand-authored fixture art only** (D10 — the `hash_goldens`/`walk_goldens`
//! precedent; no real art bytes ever enter the repo). Each golden pins one
//! layer of §1.1–1.3:
//!
//! | Golden | Pins |
//! |---|---|
//! | `combat-frame` | the palette swap + `DrawFrame_Combat`'s borders |
//! | `combat-ground` | the 7×7 tile blit at 24×24 from the camera's top-left |
//! | `combat-icons` | icon placement, the dir-4..7 mirror, a 2×2 footprint, the merged+recoloured party icon |
//! | `combat-clip` | the hard `(8,8)-(175,175)` clip on an icon straddling the window edge |
//! | `combat-focus-box` | the grey box on an empty cell (the Aim cursor's home) |
//! | `combat-focus-box-large` | a 2×2 box under a 2×2 target, that target's icon back on top |
//! | `combat-downed` | the body tile a downed party member leaves; the monster that vanishes |
//! | `combat-downed-restored` | `sub_7515A`'s restore putting the saved tile back |
//! | `combat-panel` | the right panel's layout and colours, with a readied weapon |
//! | `combat-panel-removed` | the removed-name colour and the weapon-less status row |
//!
//! Regenerate with `RESTRIKE_REGEN_GOLDENS=1`; a `.ppm` lands in the system
//! temp dir on mismatch or during regen.

use super::tests::{board_with, floor_map, presented};
use super::*;
use crate::combat::{ActionEvent, CombatMap, GridPos, HealthStatus, RemovalReason, Team};
use crate::combat_art::{CombatIcon, TileAtlas, FOCUS_BOX_SLOT};
use crate::framebuffer::{Framebuffer, HEIGHT, WIDTH};
use gbx_formats::combat_art::{Sprite, TileSet, CELL_PX, TRANSPARENT};
use gbx_formats::font::{self, Font};
use gbx_formats::image::{DecodedItem, ImageBlock};

fn write_ppm(name: &str, fb: &Framebuffer) {
    let path = std::env::temp_dir().join(format!("restrike-scene-golden-{name}.ppm"));
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            out.extend_from_slice(&fb.palette()[fb.get_pixel(x, y) as usize]);
        }
    }
    if std::fs::write(&path, &out).is_ok() {
        eprintln!("scene golden '{name}': dumped {}", path.display());
    }
}

fn check_golden(name: &str, fb: &Framebuffer, expected_hex: &str) {
    let actual = fb.hash_hex();
    if std::env::var_os("RESTRIKE_REGEN_GOLDENS").is_some() {
        eprintln!("scene golden '{name}': {actual}");
        write_ppm(name, fb);
        return;
    }
    if actual != expected_hex {
        write_ppm(name, fb);
    }
    assert_eq!(
        actual, expected_hex,
        "scene golden '{name}' mismatched — see dumped .ppm (or rerun with RESTRIKE_REGEN_GOLDENS=1)"
    );
}

// --- synthetic fixture art (D10) ------------------------------------------

/// Symbol set 4 covering `0x100..=0x127`, each item a solid 8×8 of
/// `index % 16` — the same fixture `hash_goldens`/`walk_goldens` use, so a
/// border difference between the exploration and combat frames is visible as
/// a different pattern rather than as different art.
fn synthetic_set4() -> SymbolSets {
    let mut sets = SymbolSets::new();
    sets.load(
        4,
        ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: (0..40)
                .map(|i| DecodedItem {
                    pixels: vec![(i % 16) as u8; 64],
                })
                .collect(),
        },
    );
    sets
}

fn synthetic_font() -> Font {
    let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
    for j in 0..font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
    }
    font::decode(&data)
}

/// One 24×24 fixture tile: a solid field of `fill` with a one-pixel border of
/// `edge` — enough that a mis-strided blit shears visibly.
fn fixture_tile(fill: u8, edge: u8) -> Vec<u8> {
    let mut px = vec![fill; CELL_PX * CELL_PX];
    for i in 0..CELL_PX {
        px[i] = edge;
        px[(CELL_PX - 1) * CELL_PX + i] = edge;
        px[i * CELL_PX] = edge;
        px[i * CELL_PX + CELL_PX - 1] = edge;
    }
    px
}

/// A full 48-slot atlas: slot `n` is [`fixture_tile`]`(n % 16, (n + 7) % 16)`,
/// so every atlas index is distinguishable.
fn fixture_atlas() -> TileAtlas {
    let tiles: Vec<Vec<u8>> = (0..crate::combat_art::ATLAS_SLOTS)
        .map(|n| fixture_tile((n % 16) as u8, ((n + 7) % 16) as u8))
        .collect();
    let mut atlas = TileAtlas::new();
    atlas
        .load_set(
            &TileSet { tiles },
            crate::combat_art::ATLAS_SLOTS,
            0,
            "FIXTURE.DAX",
        )
        .expect("the fixture atlas is exactly atlas-sized");
    atlas
}

/// An icon sprite `cols`×`rows` cells: an opaque body of `body`, a
/// transparent hole in the top-left cell's centre (so transparency is
/// visible), an asymmetric marker column of `marker` two pixels wide at the
/// LEFT edge of the opaque region — which is what makes the dir-4..7 mirror
/// visible in a golden — and a transparent `margin` all the way round.
///
/// `margin == 0` is the deliberately hostile case (an icon that fills its
/// cell edge to edge, so anything drawn *under* it must be invisible);
/// `margin > 0` is what real monster art looks like, and is what lets the
/// grey focus box show around a combatant.
fn fixture_sprite(cols: usize, rows: usize, body: u8, marker: u8, margin: usize) -> Sprite {
    let (w, h) = (cols * CELL_PX, rows * CELL_PX);
    let mut pixels = vec![TRANSPARENT; w * h];
    for y in margin..h - margin {
        for x in margin..w - margin {
            pixels[y * w + x] = if x < margin + 2 { marker } else { body };
        }
    }
    for y in 8..16 {
        for x in 8..16 {
            pixels[y * w + x] = TRANSPARENT;
        }
    }
    Sprite {
        width: w,
        height: h,
        pixels,
    }
}

fn fixture_icon(cols: usize, rows: usize, body: u8, marker: u8, margin: usize) -> CombatIcon {
    CombatIcon::from_poses(
        fixture_sprite(cols, rows, body, marker, margin),
        fixture_sprite(cols, rows, body.wrapping_add(1) % 16, marker, margin),
    )
}

/// A **merged and recoloured** party icon, exercising the two slice-1 pixel
/// transforms the way `LoadPlayerCombatIcon` does: a 24×10 head half laid
/// over a 24×24 body, then the six-template recolor.
fn fixture_party_icon() -> CombatIcon {
    let mut body =
        CombatIcon::from_poses(fixture_sprite(1, 1, 4, 1, 0), fixture_sprite(1, 1, 4, 1, 0));
    let head_pixels = vec![2u8; CELL_PX * 10];
    let head_sprite = Sprite {
        width: CELL_PX,
        height: 10,
        pixels: head_pixels,
    };
    let head = CombatIcon::from_poses(head_sprite.clone(), head_sprite);
    body.merge_from(&head).expect("head over body");
    // Repaint template 6 (the merged head region, 2|4) -> 13 and template 4
    // (the bare body) -> 9, through the same table shape
    // `party_recolor_tables` builds.
    let old: [u8; 16] = std::array::from_fn(|i| i as u8);
    let mut new = old;
    new[6] = 13;
    new[4] = 9;
    body.recolor(&new, &old);
    body
}

/// The fixture art store: the full atlas plus icons in slots 0 (party,
/// merged+recoloured), 1 (single-cell), 2 (2×2 monster) and `0x19` (the grey
/// focus box, a hollow square so what is under it shows through).
fn fixture_art() -> SceneArt {
    let mut icons = crate::combat_art::CombatIcons::new();
    icons.set(0, fixture_party_icon());
    icons.set(1, fixture_icon(1, 1, 12, 15, 0));
    icons.set(2, fixture_icon(2, 2, 6, 14, 3));

    let mut box_pixels = vec![TRANSPARENT; CELL_PX * CELL_PX];
    for i in 0..CELL_PX {
        box_pixels[i] = 7;
        box_pixels[(CELL_PX - 1) * CELL_PX + i] = 7;
        box_pixels[i * CELL_PX] = 7;
        box_pixels[i * CELL_PX + CELL_PX - 1] = 7;
    }
    let focus = Sprite {
        width: CELL_PX,
        height: CELL_PX,
        pixels: box_pixels,
    };
    icons.set(FOCUS_BOX_SLOT, CombatIcon::from_poses(focus.clone(), focus));

    SceneArt::new(fixture_atlas(), icons)
}

/// A patterned terrain: ground value `(x + 2*y) % 26` over the whole field,
/// which keeps every cell inside the 74-row table's non-sentinel prefix and
/// gives the 7×7 window a diagonal pattern.
fn patterned_map() -> CombatMap {
    let mut ground = vec![0u8; (crate::combat::MAP_W * crate::combat::MAP_H) as usize];
    for y in 0..crate::combat::MAP_H {
        for x in 0..crate::combat::MAP_W {
            ground[(y * crate::combat::MAP_W + x) as usize] = ((x + 2 * y) % 26) as u8;
        }
    }
    CombatMap::from_ground(ground)
}

/// The scene every golden below starts from: three combatants on the
/// patterned floor, camera at (10, 5) so the window covers cells
/// (10,5)–(16,11).
fn fixture_scene() -> CombatScene {
    let mut roster = vec![
        presented(0, Team::Party, GridPos::new(11, 6)),
        presented(1, Team::Party, GridPos::new(12, 8)),
        presented(2, Team::Monster, GridPos::new(14, 7)),
    ];
    roster[0].icon_slot = 0;
    roster[0].direction = 1; // base art
    roster[1].icon_slot = 1;
    roster[1].direction = 5; // mirrored art (dirs 4..7)
    roster[2].icon_slot = 2;
    roster[2].size = 4; // 2×2
    roster[2].direction = 0;

    let snapshot = EntrySnapshot {
        roster,
        map: patterned_map(),
        camera_top_left: GridPos::new(10, 5),
    };
    CombatScene::new(snapshot, fixture_art())
}

// --- the goldens -----------------------------------------------------------

#[test]
fn golden_combat_frame() {
    let mut fb = Framebuffer::new();
    render::palette_combat(&mut fb);
    crate::frames::draw_frame_combat(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-frame",
        &fb,
        "a36980aad0d0fbf04b461e14fe8ac61d36dfb75f43a1b406f8cb5bb0359bc611",
    );
}

#[test]
fn golden_combat_ground() {
    let scene = fixture_scene();
    let mut fb = Framebuffer::new();
    render::palette_combat(&mut fb);
    crate::frames::draw_frame_combat(&mut fb, &synthetic_set4()).unwrap();
    render::draw_ground(&mut fb, scene.board(), scene.art()).unwrap();
    check_golden(
        "combat-ground",
        &fb,
        "cfd6d6531fc0edcff8c1b3994d5af7150b18df3eaec422ce68e94c81680dbf4e",
    );
}

#[test]
fn golden_combat_icons() {
    let scene = fixture_scene();
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-icons",
        &fb,
        "d8081aa7f845db48642372662f1013dba33df44136c0da26aca756c9441fee42",
    );
}

#[test]
fn golden_combat_clip() {
    // A 2×2 monster anchored on the window's bottom-right cell: half of it
    // lies outside (8,8)-(175,175) and must be clipped away, not wrapped.
    let mut scene = fixture_scene();
    scene.board_mut().apply(ActionEvent::Move {
        combatant_id: 2,
        from_x: 14,
        from_y: 7,
        to_x: 15,
        to_y: 8,
        cost: 2,
    });
    scene.board_mut().apply(ActionEvent::Move {
        combatant_id: 2,
        from_x: 15,
        from_y: 8,
        to_x: 16,
        to_y: 9,
        cost: 2,
    });
    scene.board_mut().apply(ActionEvent::Move {
        combatant_id: 2,
        from_x: 16,
        from_y: 9,
        to_x: 16,
        to_y: 10,
        cost: 2,
    });
    scene.board_mut().apply(ActionEvent::Move {
        combatant_id: 2,
        from_x: 16,
        from_y: 10,
        to_x: 16,
        to_y: 11,
        cost: 2,
    });
    // On a bare canvas — no frame — nothing the map draws may land outside
    // the viewport's `[8,176) × [8,176)` window. (On the framed screen the
    // border column starts at exactly x = 176, so the two are flush and the
    // check has to run without it.)
    let mut bare = Framebuffer::new();
    render::draw_ground(&mut bare, scene.board(), scene.art()).unwrap();
    render::draw_combatants(&mut bare, scene.board(), scene.art());
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let inside = (8..176).contains(&x) && (8..176).contains(&y);
            if !inside {
                assert_eq!(bare.get_pixel(x, y), 0, "pixel ({x},{y}) escaped the clip");
            }
        }
    }

    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-clip",
        &fb,
        "0d1b862a0cb039d30c23c47222b9a8c1a6bcd771c2363aed4c0f1bac3a530227",
    );
}

#[test]
fn golden_combat_focus_box() {
    // On an EMPTY cell — the Aim cursor's usual home, and the case where the
    // box's own pixels are what the golden pins (a box under an opaque
    // fixture icon is entirely hidden by it, which is correct but pins
    // nothing).
    let mut scene = fixture_scene();
    scene.set_focus(Some(FocusCursor {
        pos: GridPos::new(13, 9),
        size: 1,
    }));
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-focus-box",
        &fb,
        "15f2ccb3dcf15cb77cfc88a7e881a3976146d85d5d4225e5ec461a058ddb0470",
    );
}

#[test]
fn golden_combat_focus_box_under_a_large_target() {
    // The box takes the *targeted* combatant's footprint
    // (`mapToBackGroundTile.size`), so a 2×2 monster gets a 2×2 box — and the
    // monster's icon goes back on top of all four cells.
    let mut scene = fixture_scene();
    scene.set_focus(Some(FocusCursor {
        pos: GridPos::new(14, 7),
        size: 4,
    }));
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-focus-box-large",
        &fb,
        "9422662c7697efd65e4b43bdc030376a7ad4e17fcca0b3624ce12b29557b05c8",
    );
}

#[test]
fn golden_combat_downed() {
    let mut scene = fixture_scene();
    // The party member goes down (body tile) and the monster is killed
    // (vanishes without one).
    scene.apply_event(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Downed { dying: true },
    });
    scene.apply_event(ActionEvent::Removed {
        combatant_id: 2,
        reason: RemovalReason::Killed,
    });
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-downed",
        &fb,
        "bc40d3ee9bf8fddf4574f9a66874b6d8f6c8a4ece34a24508f9f6fe7a91345c3",
    );
}

#[test]
fn golden_combat_downed_restored() {
    let mut scene = fixture_scene();
    scene.apply_event(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Downed { dying: true },
    });
    scene
        .board_mut()
        .restore_downed(0, GridPos::new(11, 6))
        .expect("the saved tile goes back");
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    check_golden(
        "combat-downed-restored",
        &fb,
        "c5010e06fe2daf0a70e4b87307d7bd7c4824c2aac660e87f70719c0a065262d8",
    );
}

#[test]
fn golden_combat_panel() {
    let scene = fixture_scene();
    let font = synthetic_font();
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    scene.clear_text_surfaces(&mut fb);
    scene.draw_panel(
        &mut fb,
        &font,
        &PanelSummary {
            name: "KERMIT".to_string(),
            team: Team::Party,
            in_combat: true,
            hp_current: 7,
            hp_max: 12,
            ac: 0x32,
            health_status: HealthStatus::Okey,
            readied_weapon: Some("Long Sword +1".to_string()),
            held: false,
        },
    );
    check_golden(
        "combat-panel",
        &fb,
        "c20a46b1c508109ff874af1f7c706f1d2823315173224b60a8f8c0f149e1854d",
    );
}

#[test]
fn golden_combat_panel_removed_combatant() {
    // A removed combatant: the name takes the removed colour and the health
    // status prints where the weapon-less layout puts it.
    let scene = fixture_scene();
    let font = synthetic_font();
    let mut fb = Framebuffer::new();
    scene.render(&mut fb, &synthetic_set4()).unwrap();
    scene.draw_panel(
        &mut fb,
        &font,
        &PanelSummary {
            name: "KOBOLD".to_string(),
            team: Team::Monster,
            in_combat: false,
            hp_current: 0,
            hp_max: 6,
            ac: 0x37,
            health_status: HealthStatus::Dying,
            readied_weapon: None,
            held: false,
        },
    );
    check_golden(
        "combat-panel-removed",
        &fb,
        "1642d26c682c4c618e798ac7db9af209fb6f9b01993c1eac34b74ab306cb7e88",
    );
}

// --- non-hash assertions that make the goldens legible ---------------------

#[test]
fn the_palette_swap_exchanges_registers_0_and_8() {
    let mut fb = Framebuffer::new();
    let canon = gbx_rules::palette::EGA_PALETTE;
    render::palette_combat(&mut fb);
    assert_eq!(fb.palette()[0], canon[8]);
    assert_eq!(fb.palette()[8], canon[0]);
    render::palette_normal(&mut fb);
    assert_eq!(fb.palette()[0], canon[0]);
    assert_eq!(fb.palette()[8], canon[8]);
}

#[test]
fn ground_tiles_land_on_the_24_pixel_grid() {
    let scene = fixture_scene();
    let mut fb = Framebuffer::new();
    render::draw_ground(&mut fb, scene.board(), scene.art()).unwrap();
    // Cell (0,0) is map (10,5): ground (10 + 2*5) % 26 = 20 -> tile_index
    // 0x13 = 19 -> fixture tile fill 19 % 16 = 3, edge (19+7) % 16 = 10.
    assert_eq!(fb.get_pixel(8, 8), 10, "the tile's own top-left edge pixel");
    assert_eq!(fb.get_pixel(9 + 1, 9 + 1), 3, "the tile's interior");
    // Cell (1,0) is map (11,5): ground 21 -> tile_index 0x14 = 20 -> fill 4.
    assert_eq!(fb.get_pixel(8 + 24 + 2, 8 + 2), 4);
    // Nothing outside the viewport.
    assert_eq!(fb.get_pixel(7, 8), 0);
    assert_eq!(fb.get_pixel(176, 8), 0);
}

#[test]
fn a_mirrored_facing_flips_the_marker_column_to_the_right_edge() {
    let scene = fixture_scene();
    let mut fb = Framebuffer::new();
    render::draw_combatants(&mut fb, scene.board(), scene.art());
    // Combatant 1 sits at map (12,8) -> screen (2,3) -> pixels (56, 80), and
    // faces 5, so its LEFT marker column renders on the RIGHT edge.
    assert_eq!(fb.get_pixel(56 + 23, 80 + 20), 15, "mirrored marker");
    assert_eq!(
        fb.get_pixel(56, 80 + 20),
        12,
        "the body, where the marker was"
    );
}

#[test]
fn a_two_by_two_monster_covers_four_cells() {
    let scene = fixture_scene();
    let mut fb = Framebuffer::new();
    render::draw_combatants(&mut fb, scene.board(), scene.art());
    // Combatant 2 at map (14,7) -> screen (4,2) -> pixels (104, 56), 48×48
    // with a 3-pixel transparent margin.
    for (dx, dy) in [(5, 5), (28, 5), (5, 28), (28, 28)] {
        assert_eq!(
            fb.get_pixel(104 + dx, 56 + dy),
            6,
            "the 2×2 footprint's ({dx},{dy}) corner"
        );
    }
    assert_eq!(fb.get_pixel(104 + 48 + 2, 56 + 2), 0, "and nothing past it");
}

#[test]
fn the_panel_writes_only_inside_its_own_columns() {
    let scene = fixture_scene();
    let font = synthetic_font();
    let mut fb = Framebuffer::new();
    scene.draw_panel(
        &mut fb,
        &font,
        &PanelSummary {
            name: "KERMIT".to_string(),
            team: Team::Party,
            in_combat: true,
            hp_current: 12,
            hp_max: 12,
            ac: 0x32,
            health_status: HealthStatus::Okey,
            readied_weapon: None,
            held: false,
        },
    );
    for y in 0..HEIGHT {
        for x in 0..0x17 * 8 {
            assert_eq!(fb.get_pixel(x, y), 0, "the panel wrote at ({x},{y})");
        }
    }
}

#[test]
fn a_full_hit_point_total_prints_green_and_a_damaged_one_yellow() {
    let scene = fixture_scene();
    let font = synthetic_font();
    let summary = |hp: i32| PanelSummary {
        name: "A".to_string(),
        team: Team::Party,
        in_combat: true,
        hp_current: hp,
        hp_max: 12,
        ac: 0x32,
        health_status: HealthStatus::Okey,
        readied_weapon: None,
        held: false,
    };
    // The synthetic font's glyph rows are `index` repeated, so glyph '1'
    // (index 0x31) has bit 0 set only — pixel column 7 of the cell carries
    // the foreground colour.
    let hp_px =
        |fb: &Framebuffer| fb.get_pixel(layout::PANEL_HP_COL * 8 + 7, layout::PANEL_HP_ROW * 8);

    let mut full = Framebuffer::new();
    scene.draw_panel(&mut full, &font, &summary(12));
    let mut hurt = Framebuffer::new();
    scene.draw_panel(&mut hurt, &font, &summary(7));
    assert_eq!(hp_px(&full), layout::HP_COLOR_FULL);
    assert_eq!(hp_px(&hurt), layout::HP_COLOR_DAMAGED);
}

#[test]
fn the_message_status_and_prompt_surfaces_clear_to_black() {
    let scene = fixture_scene();
    let mut fb = Framebuffer::new();
    fb.clear(9);
    scene.clear_text_surfaces(&mut fb);
    // The message region: rows 10..=0x15, cols 0x17..=0x26.
    assert_eq!(fb.get_pixel(0x17 * 8, 10 * 8), 0);
    assert_eq!(fb.get_pixel(0x26 * 8 + 7, 0x15 * 8 + 7), 0);
    assert_eq!(
        fb.get_pixel(0x17 * 8, 9 * 8),
        9,
        "row 9 is not the message region"
    );
    // Status row 0x17 and prompt row 0x18, full width.
    assert_eq!(fb.get_pixel(0, layout::STATUS_ROW * 8), 0);
    assert_eq!(fb.get_pixel(319, layout::PROMPT_ROW * 8 + 7), 0);
}

#[test]
fn a_downed_party_member_repaints_its_cell_with_the_body_tile() {
    let mut scene = fixture_scene();
    let mut fb = Framebuffer::new();
    render::draw_ground(&mut fb, scene.board(), scene.art()).unwrap();
    let before = fb.get_pixel(8 + 24 + 2, 8 + 24 + 2); // cell (1,1) = map (11,6)

    scene.apply_event(ActionEvent::Removed {
        combatant_id: 0,
        reason: RemovalReason::Killed,
    });
    let mut after_fb = Framebuffer::new();
    render::draw_ground(&mut after_fb, scene.board(), scene.art()).unwrap();
    let after = after_fb.get_pixel(8 + 24 + 2, 8 + 24 + 2);
    // Ground tile 0x1F -> atlas slot 0x27 = 39 -> fixture fill 39 % 16 = 7.
    assert_eq!(after, 7, "the RANDCOM body slot");
    assert_ne!(before, after);
}

#[test]
fn the_focus_box_goes_under_the_cells_occupant() {
    // Ordering proof: the box is laid first and the occupant's icon is
    // redrawn over it, so an opaque icon pixel wins.
    let mut scene = fixture_scene();
    scene.set_focus(Some(FocusCursor {
        pos: GridPos::new(11, 6),
        size: 1,
    }));
    let mut with_box = Framebuffer::new();
    scene.render(&mut with_box, &synthetic_set4()).unwrap();

    scene.set_focus(None);
    let mut without_box = Framebuffer::new();
    scene.render(&mut without_box, &synthetic_set4()).unwrap();

    assert_eq!(
        with_box.hash(),
        without_box.hash(),
        "the fixture icon is opaque over its whole cell, so the box under it \
         must leave no pixel behind"
    );
    // And on an empty cell the box does show.
    scene.set_focus(Some(FocusCursor {
        pos: GridPos::new(13, 9),
        size: 1,
    }));
    let mut empty_cell = Framebuffer::new();
    scene.render(&mut empty_cell, &synthetic_set4()).unwrap();
    let (x, y) = layout::screen_cell_pixel_origin(3, 4); // map (13,9)
    assert_eq!(empty_cell.get_pixel(x, y), 7, "the box's own border pixel");
    assert_ne!(empty_cell.hash(), without_box.hash());
}

#[test]
fn an_icon_anchored_off_the_window_still_clips_in() {
    // A 2×2 monster whose anchor cell is one row ABOVE the window: the
    // original blits from the negative origin and lets the clip keep the
    // bottom half, so the two on-window cells must be painted.
    let mut roster = vec![presented(0, Team::Monster, GridPos::new(10, 4))];
    roster[0].icon_slot = 2;
    roster[0].size = 4;
    let snapshot = EntrySnapshot {
        roster,
        map: patterned_map(),
        camera_top_left: GridPos::new(10, 5),
    };
    let scene = CombatScene::new(snapshot, fixture_art());
    let mut fb = Framebuffer::new();
    render::draw_combatants(&mut fb, scene.board(), scene.art());
    // The anchor is at screen (0,-1), i.e. pixel y0 = -16: the sprite's rows
    // 16.. land in the window, so window pixel (18,18) is sprite (10,26) —
    // body, past both the 3-pixel margin and the transparent hole.
    assert_eq!(fb.get_pixel(8 + 10, 8 + 10), 6, "the clipped-in body");
    assert_eq!(fb.get_pixel(8 + 10, 7), 0, "nothing above the clip");
}

#[test]
fn a_board_test_helper_sanity_check() {
    // The shared fixtures the unit tests build on stay in step with the
    // goldens' expectations about them.
    let b = board_with(
        vec![presented(0, Team::Party, GridPos::new(1, 1))],
        floor_map(),
    );
    assert_eq!(b.map().ground_tile(GridPos::new(1, 1)), 0x17);
    assert!(b.on_screen(GridPos::new(6, 6)));
}
