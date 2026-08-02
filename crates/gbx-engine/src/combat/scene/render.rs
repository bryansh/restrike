//! The combat screen's draws (`docs/design/combat-visualizer.md` §1.1–1.3,
//! D-CV4: faithful-first). Every function here is one coab site, takes the
//! framebuffer plus the art it blits, and reads nothing but the presented
//! board.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr033.cs` `Color_0_8_inverse`/`Color_0_8_normal` (`:43-55`) —
//!   the palette swap that makes combat's backdrop grey.
//! - `engine/ovr025.cs` `RedrawCombatScreen` (`:1514-1519`) — swap →
//!   `DrawFrame_Combat` → `redrawCombatArea(8, 0xff, centre)`.
//! - `engine/ovr033.cs` `ScreenMapCheck` (`:316-334`) — the 7×7 tile paint.
//! - `engine/ovr033.cs` `redrawCombatArea` (`:340-370`) — the icon repaint
//!   pass over every on-screen combatant, `Icon.Normal`, its own facing.
//! - `engine/ovr033.cs` `sub_7416E` (`:58-83`) — the focus box: ground tile,
//!   then the grey cursor (icon slot 0x19) per footprint cell, then the
//!   occupant's icon on top.
//! - `engine/ovr034.cs` `DrawIsoTile` (`:30-40`) — the `> 0x7f` overlay fork
//!   (doc §6 item 2, unknown) and the plain atlas blit.
//! - `engine/ovr034.cs` `draw_combat_icon` (`:90-98`) — `if (icon != null)`,
//!   i.e. an unloaded icon slot draws **nothing**, silently.
//! - `engine/ovr025.cs` `CombatDisplayPlayerSummary` (`:291-335`),
//!   `display_hp` (`:270-289`), `display_AC` (`:263-267`),
//!   `DisplayPlayerStatusString` (`:787-811`), `displayPlayerName`
//!   (`:827-847`), `ClearPlayerTextArea` (`:815-825`).
//! - `engine/ovr020.cs:36-38` — `statusString`.
//! - `engine/ovr027.cs` `ClearPromptAreaNoUpdate` (`:352-355`) and
//!   `engine/ovr014.cs:1904` — the prompt and status rows' clears.

use super::layout::*;
use super::missile::SpriteRef;
use super::{PresentedBoard, PresentedCombatant, SceneArt, SceneError};
use crate::combat::{HealthStatus, Team, BACKGROUND_TILE_INDEX};
use crate::combat_art::{CombatIcon, FOCUS_BOX_SLOT};
use crate::draw::{blit_image, cell_rect_fill};
use crate::framebuffer::Framebuffer;
use crate::frames::draw_frame_combat;
use crate::symbols::SymbolSets;
use crate::text::{draw_string, JobStatus, TextCursor, TextJob};
use gbx_formats::combat_art::{Sprite, CELL_PX};
use gbx_formats::font::Font;

/// `Color_0_8_inverse` (`ovr033.cs:43-47`): EGA registers 0 and 8 swap for
/// the whole fight — `SetPaletteColor(8, 0)` points slot 0 at canon colour 8
/// (dark grey) and `SetPaletteColor(0, 8)` points slot 8 at canon colour 0.
pub fn palette_combat(fb: &mut Framebuffer) {
    fb.set_ega_palette(0, 8);
    fb.set_ega_palette(8, 0);
}

/// `Color_0_8_normal` (`ovr033.cs:49-53`): put both registers back. Runs at
/// fight exit and around the mid-combat View sheet (`ovr020.cs:240,334`).
pub fn palette_normal(fb: &mut Framebuffer) {
    fb.set_ega_palette(0, 0);
    fb.set_ega_palette(8, 8);
}

/// The atlas slot a map cell paints with: `BackGroundTiles[value].tile_index`
/// (`ovr033.cs:327`).
///
/// Two loud failures instead of guesses:
/// - a ground value past the 74-row table (`BackGroundTiles` is indexed
///   unguarded in the original — a value that big is a corrupt map, not a
///   render decision);
/// - `tile_index > 0x7f`, which forks into `DrawIsoTile`'s
///   `OverlayUnbounded(gbl.dword_1C8FC, …)` path — **stubbed in coab, actual
///   behavior unknown** (doc §6 item 2). Four sentinel rows of the table
///   (66, 67, 69, 71) carry `0xFF`, so this is reachable by data, not just by
///   a bug.
fn atlas_slot(ground_value: u8) -> Result<usize, SceneError> {
    let tile_index = BACKGROUND_TILE_INDEX
        .get(ground_value as usize)
        .copied()
        .ok_or(SceneError::BackgroundTileOutOfRange { ground_value })?;
    if tile_index > 0x7F {
        return Err(SceneError::IsoTileOverlayPath {
            ground_value,
            tile_index,
        });
    }
    Ok(tile_index as usize)
}

/// `DrawIsoTile(tile_index, cy*3, cx*3)` for one **screen** cell: blits the
/// 24×24 atlas slot at that cell, under the viewport clip.
fn draw_iso_tile(
    fb: &mut Framebuffer,
    art: &SceneArt,
    slot: usize,
    cx: usize,
    cy: usize,
) -> Result<(), SceneError> {
    let tile = art
        .tiles
        .tile(slot)
        .ok_or(SceneError::AtlasSlotOutOfRange { slot })?;
    let (row, col) = screen_cell_text_origin(cx, cy);
    blit_image(
        fb,
        tile,
        CELL_PX,
        CELL_PX,
        row,
        col,
        VIEWPORT_CLIP,
        None,
        None,
    );
    Ok(())
}

/// The 7×7 ground paint inside `ScreenMapCheck` (`ovr033.cs:316-334`):
/// row-major from `mapScreenTopLeft`, each cell's `BackGroundTiles` slot.
pub fn draw_ground(
    fb: &mut Framebuffer,
    board: &PresentedBoard,
    art: &SceneArt,
) -> Result<(), SceneError> {
    let top_left = board.camera_top_left();
    for cy in 0..VIEWPORT_CELLS {
        for cx in 0..VIEWPORT_CELLS {
            let cell = crate::combat::GridPos::new(top_left.x + cx as i32, top_left.y + cy as i32);
            let slot = atlas_slot(board.map().ground_tile(cell))?;
            draw_iso_tile(fb, art, slot, cx, cy)?;
        }
    }
    Ok(())
}

/// The icon blit, at a **signed** pixel origin.
///
/// [`blit_image`] anchors at an unsigned *cell*, which is right for
/// everything drawn from the exploration screen but not here: a multi-cell
/// monster's anchor cell can sit left of or above the window while its body
/// reaches into it, and the original blits from that negative origin and
/// lets `draw_combat_picture`'s clip keep the visible part
/// (`seg040.cs:73-113` walks `minY..maxY` unconditionally and tests each
/// pixel against the clip). Same loop, same order, same transparency rule —
/// only the origin's type differs, so this stays beside the icon path
/// instead of widening a primitive every other caller uses unsigned.
fn blit_sprite_px(fb: &mut Framebuffer, sprite: &Sprite, x0: i32, y0: i32) {
    for row in 0..sprite.height {
        for col in 0..sprite.width {
            let px = x0 + col as i32;
            let py = y0 + row as i32;
            if px < VIEWPORT_CLIP.x0 as i32
                || px >= VIEWPORT_CLIP.x1 as i32
                || py < VIEWPORT_CLIP.y0 as i32
                || py >= VIEWPORT_CLIP.y1 as i32
            {
                continue;
            }
            // `set_pixel`'s own `< 16` guard is the transparency skip.
            fb.set_pixel(
                px as usize,
                py as usize,
                sprite.pixels[row * sprite.width + col],
            );
        }
    }
}

/// `draw_combat_icon(icon_id, iconState, direction, tileY, tileX)`
/// (`ovr034.cs:90-98`) at a **screen** cell, which may be partly (or wholly)
/// off-window — the viewport clip takes care of that.
fn draw_icon_at(
    fb: &mut Framebuffer,
    icon: &CombatIcon,
    pose: crate::combat_art::IconPose,
    direction: u8,
    screen_x: i32,
    screen_y: i32,
) {
    let frame = icon.frame(pose, direction);
    // `(tileX*3 + 1) * 8` = `24·tileX + 8`, the same origin
    // `screen_cell_pixel_origin` computes for a non-negative cell.
    let x0 = screen_x * CELL_PX as i32 + VIEWPORT_ORIGIN_PX.0 as i32;
    let y0 = screen_y * CELL_PX as i32 + VIEWPORT_ORIGIN_PX.1 as i32;
    blit_sprite_px(fb, frame, x0, y0);
}

/// One combatant's icon, at its own anchor, facing and pose — the body of
/// both `redrawCombatArea`'s repaint loop and `DrawPlayerIconIfOnScreen`.
/// An unloaded icon slot draws nothing at all, matching the original's
/// `if (icon != null)` (`ovr034.cs:92`).
fn draw_combatant(
    fb: &mut Framebuffer,
    board: &PresentedBoard,
    art: &SceneArt,
    c: &PresentedCombatant,
) {
    let Some(icon) = art.icons.get(c.icon_slot) else {
        return;
    };
    let (sx, sy) = board.screen_pos(c.pos);
    draw_icon_at(fb, icon, c.pose, c.direction, sx, sy);
}

/// `redrawCombatArea`'s icon pass (`ovr033.cs:349-359`): every combatant that
/// is in combat, has a footprint, and is on screen draws its Normal frame at
/// its own facing. Roster order — a later combatant's icon overlaps an
/// earlier one, exactly as the original stacks them.
pub fn draw_combatants(fb: &mut Framebuffer, board: &PresentedBoard, art: &SceneArt) {
    for c in board.combatants() {
        if c.on_board() && board.combatant_on_screen(c) && !board.icon_suppressed(c.id) {
            draw_combatant(fb, board, art, c);
        }
    }
}

/// `DrawPlayerIconIfOnScreen(PlayerIndexAtMapXY(pos))` (`ovr033.cs:84-95`):
/// redraw **only** the combatant occupying `pos`, at its own anchor. This is
/// what `sub_7416E` puts back over the focus box — not the whole board.
pub fn draw_cell_occupant(
    fb: &mut Framebuffer,
    board: &PresentedBoard,
    art: &SceneArt,
    pos: crate::combat::GridPos,
) {
    if let Some(c) = board.occupant_at(pos) {
        if board.combatant_on_screen(c) && !board.icon_suppressed(c.id) {
            draw_combatant(fb, board, art, c);
        }
    }
}

/// `sub_7416E(pos)` (`ovr033.cs:58-83`) — the turn/aim indicator. For every
/// cell of a `size`-shaped footprint anchored at `pos` that is on screen:
/// repaint the ground tile, then (when the cursor is up) blit the grey focus
/// box from icon slot `0x19`; finally put that cell's occupant back **over**
/// the box.
///
/// `size` is the original's `gbl.mapToBackGroundTile.size`, parked by
/// `RedrawCombatIfFocusOn` from the acting/targeted combatant's footprint
/// (`ovr033.cs:670-672`) — a box under a 2×2 monster covers all four cells.
pub fn draw_focus_box(
    fb: &mut Framebuffer,
    board: &PresentedBoard,
    art: &SceneArt,
    pos: crate::combat::GridPos,
    size: u8,
    draw_cursor: bool,
) -> Result<(), SceneError> {
    for cell in crate::combat::size_footprint(size, pos) {
        if !board.on_screen(cell) {
            continue;
        }
        let (sx, sy) = board.screen_pos(cell);
        let slot = atlas_slot(board.map().ground_tile(cell))?;
        draw_iso_tile(fb, art, slot, sx as usize, sy as usize)?;
        if draw_cursor {
            if let Some(icon) = art.icons.get(FOCUS_BOX_SLOT) {
                draw_icon_at(fb, icon, crate::combat_art::IconPose::Normal, 0, sx, sy);
            }
        }
    }
    // `sub_7416E`'s tail: the cell's occupant goes back on top of the box.
    draw_cell_occupant(fb, board, art, pos);
    Ok(())
}

/// `RedrawCombatScreen` (`ovr025.cs:1514-1519`) minus its interactive tail:
/// palette swap, `DrawFrame_Combat`, then the full map repaint (ground +
/// icons). The status and prompt rows are cleared too — `DrawFrame_Combat`'s
/// own `draw8x8_clear_area(0x17, 0x27, 0, 0)` already covers them, so this is
/// a statement of intent rather than extra pixels.
pub fn redraw_combat_screen(
    fb: &mut Framebuffer,
    board: &PresentedBoard,
    art: &SceneArt,
    sets: &SymbolSets,
) -> Result<(), SceneError> {
    palette_combat(fb);
    draw_frame_combat(fb, sets)?;
    draw_ground(fb, board, art)?;
    draw_combatants(fb, board, art);
    Ok(())
}

/// `ClearPlayerTextArea` in combat (`ovr025.cs:817-819`): the panel's message
/// rows, 10 down. Slice 4 fills them; slice 3 owns the surface.
pub fn clear_message_region(fb: &mut Framebuffer) {
    cell_rect_fill(
        fb,
        0,
        MESSAGE_REGION.y_start,
        MESSAGE_REGION.y_end,
        MESSAGE_REGION.x_start,
        MESSAGE_REGION.x_end,
    );
}

/// The status line's clear — `draw8x8_clear_area(0x17, 0x27, 0x17, 0)`
/// (`ovr014.cs:1904`, `ovr023.cs:810`).
pub fn clear_status_line(fb: &mut Framebuffer) {
    cell_rect_fill(fb, 0, STATUS_ROW, STATUS_ROW, 0, LAST_COL);
}

/// `ClearPromptAreaNoUpdate` (`ovr027.cs:352-355`) — row 0x18, all columns.
pub fn clear_prompt_line(fb: &mut Framebuffer) {
    cell_rect_fill(fb, 0, PROMPT_ROW, PROMPT_ROW, 0, LAST_COL);
}

/// `ovr020.statusString[health_status]` (`ovr020.cs:36-38`) — indexed by the
/// raw `Status` value, so the unmodeled rungs keep their slots.
pub fn status_string(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Okey => "Okay",
        HealthStatus::Animated => "Animated",
        HealthStatus::Running => "Running",
        HealthStatus::Unconscious => "Unconscious",
        HealthStatus::Dying => "Dying",
        HealthStatus::Dead => "Dead",
    }
}

/// `displayPlayerName`'s colour choice (`ovr025.cs:827-838`): removed first,
/// then enemy, then party.
pub fn name_color(in_combat: bool, team: Team) -> u8 {
    if !in_combat {
        NAME_COLOR_REMOVED
    } else if team == Team::Monster {
        NAME_COLOR_ENEMY
    } else {
        NAME_COLOR_PARTY
    }
}

/// What the right panel prints for one combatant — the boundary-read half of
/// [`CombatScene::draw_panel_summary`](super::CombatScene::draw_panel_summary),
/// separated so the draw itself is a pure function of plain data (and so the
/// goldens can drive it without a `CombatState`).
///
/// `readied_weapon` is the display **name** of `activeItems.primaryWeapon`;
/// `None` means "no weapon readied", which is exactly the original's
/// `primaryWeapon == null` branch — it prints no weapon line and the status
/// line moves up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelSummary {
    pub name: String,
    pub team: Team,
    pub in_combat: bool,
    pub hp_current: i32,
    pub hp_max: i32,
    /// Raw AC; the panel prints `0x3C - ac` (`Player.DisplayAc`).
    pub ac: u8,
    pub health_status: HealthStatus,
    pub readied_weapon: Option<String>,
    /// `player.IsHeld()` — printed as "(Helpless)" for a combatant still in
    /// combat.
    pub held: bool,
}

/// `CombatDisplayPlayerSummary(player)` (`ovr025.cs:291-335`), drawn at every
/// turn start (`ovr009.cs:124`).
///
/// The `display_hitpoints_ac` gate that gets cleared on entry is the caller's
/// (it is "has anything changed since the last summary?", not geometry).
pub fn draw_panel_summary(fb: &mut Framebuffer, font: &Font, summary: &PanelSummary) {
    // draw8x8_clear_area(TextRegion.CombatSummary).
    cell_rect_fill(
        fb,
        0,
        PANEL_REGION.y_start,
        PANEL_REGION.y_end,
        PANEL_REGION.x_start,
        PANEL_REGION.x_end,
    );

    // DisplayPlayerStatusString(false, 1, " ", player): re-clear from the
    // name row down, print the name, then run a one-space wrapped print that
    // parks the text cursor at (2, 0x17).
    cell_rect_fill(
        fb,
        0,
        PANEL_NAME_ROW,
        PANEL_REGION.y_end,
        PANEL_REGION.x_start,
        PANEL_REGION.x_end,
    );
    draw_string(
        fb,
        font,
        &summary.name,
        PANEL_NAME_ROW,
        PANEL_COL,
        0,
        name_color(summary.in_combat, summary.team),
    );
    let mut cursor = TextCursor {
        col: PANEL_COL,
        row: PANEL_STATUS_TEXT_ROW,
    };
    let status_text_region = crate::text::TextRegion {
        y_start: PANEL_STATUS_TEXT_ROW,
        y_end: PANEL_REGION.y_end,
        x_start: PANEL_COL,
        x_end: PANEL_REGION.x_end,
    };
    run_text(fb, font, " ", LABEL_COLOR, status_text_region, &mut cursor);

    draw_string(
        fb,
        font,
        "Hitpoints",
        PANEL_HP_ROW,
        PANEL_COL,
        0,
        LABEL_COLOR,
    );
    let hp_color = if summary.hp_current < summary.hp_max {
        HP_COLOR_DAMAGED
    } else {
        HP_COLOR_FULL
    };
    draw_string(
        fb,
        font,
        &summary.hp_current.to_string(),
        PANEL_HP_ROW,
        PANEL_HP_COL,
        0,
        hp_color,
    );

    draw_string(fb, font, "AC", PANEL_AC_ROW, PANEL_COL, 0, LABEL_COLOR);
    draw_string(
        fb,
        font,
        &(0x3C - summary.ac as i32).to_string(),
        PANEL_AC_ROW,
        PANEL_AC_COL,
        0,
        LABEL_COLOR,
    );

    // `gbl.textYCol = line + 1` — the cursor row the status line is measured
    // from, whether or not a weapon prints.
    cursor.row = PANEL_AC_ROW;
    cursor.col = PANEL_COL;
    if let Some(weapon) = &summary.readied_weapon {
        cursor.row = PANEL_WEAPON_REGION.y_start;
        run_text(
            fb,
            font,
            weapon,
            LABEL_COLOR,
            PANEL_WEAPON_REGION,
            &mut cursor,
        );
    }

    // `line = gbl.textYCol + 1`, then the status prints at `line + 1`.
    let status_row = cursor.row + 2;
    if !summary.in_combat {
        draw_string(
            fb,
            font,
            status_string(summary.health_status),
            status_row,
            PANEL_COL,
            0,
            STATUS_COLOR,
        );
    } else if summary.held {
        draw_string(
            fb,
            font,
            "(Helpless)",
            status_row,
            PANEL_COL,
            0,
            STATUS_COLOR,
        );
    }
}

/// One `press_any_key(text, true, colour, region)` run to completion.
///
/// Combat text is **not** per-character paced (`BattleSetup` clears
/// `DelayBetweenCharacters`, `ovr011.cs:1171`, §1.4), so the whole string
/// draws in one go; the pagination gate can't fire for panel-sized strings,
/// and if it ever did, releasing it is the faithful "keep going".
fn run_text(
    fb: &mut Framebuffer,
    font: &Font,
    text: &str,
    color: u8,
    region: crate::text::TextRegion,
    cursor: &mut TextCursor,
) {
    let mut job = TextJob::new(text, color, region, true, cursor, fb);
    loop {
        match job.advance(u32::MAX, fb, font, cursor) {
            JobStatus::Done => break,
            JobStatus::NeedsKey => job.release(fb),
            JobStatus::Continuing => unreachable!("the budget is unbounded"),
        }
    }
}

/// The presented combatant a panel summary is built from — everything except
/// the two fields only a boundary `CombatState` read can supply
/// (`readied_weapon`, `held`).
pub(super) fn summary_skeleton(c: &PresentedCombatant) -> PanelSummary {
    PanelSummary {
        name: c.name.clone(),
        team: c.team,
        in_combat: c.in_combat,
        hp_current: c.hp_current,
        hp_max: c.hp_max,
        ac: c.ac,
        health_status: c.health_status,
        readied_weapon: None,
        held: false,
    }
}

// ===========================================================================
// The text surfaces and the overlay (M6 slice 4, §1.4–1.5)
// ===========================================================================

/// Where a panel print lands.
///
/// `DisplayAttackMessage` derives its tail rows from the wrap cursor
/// (`line = gbl.textYCol + 1`, `ovr014.cs:180`) rather than from constants, so
/// the row a message lands on depends on how many rows the *previous* wrapped
/// print consumed. Resolving that needs the font and the region, i.e. it can
/// only happen at draw time — which is why a message beat is a little script
/// of [`PanelOp`]s replayed here rather than a list of finished lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A literal row (10 and 12 are the attack sequence's two constants).
    At(usize),
    /// `line + delta`, where `line` is the row [`PanelOp::Mark`] captured.
    FromMark(usize),
}

/// One print into the panel's message region — the calls §1.5's beats are
/// built from, in the original's own vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelOp {
    /// `ClearPlayerTextArea()` (`ovr025.cs:817`) — rows 10..=0x15.
    Clear,
    /// `DisplayPlayerStatusString(false, row, text, who)`
    /// (`ovr025.cs:786-800`): clear from `row` down, print `who`'s name on
    /// `row`, wrap `text` from `row + 1` to the region's foot.
    Status { row: Row, who: usize, text: String },
    /// `displayPlayerName(false, row, 0x17, who)` (`ovr014.cs:132`) — the
    /// target's name on its own row, with no clear and no text.
    Name { row: Row, who: usize },
    /// `press_any_key(text, true, 10, row + 3, 0x26, row, 0x17)`
    /// (`ovr014.cs:177`) — the damage/miss line, wrapped into four rows.
    Wrapped { row: Row, text: String },
    /// `displayString(text, 0, 10, row, 0x17)` (`ovr014.cs:196`) — one
    /// unwrapped line, no clear.
    Line { row: Row, text: String },
    /// `line = gbl.textYCol + 1` (`ovr014.cs:180`) — remember where the last
    /// wrapped print ended, so the removal tail can hang off it.
    Mark,
}

/// Replays a message script into the panel's message region.
///
/// Stateless between calls: the whole script is replayed from a clean region
/// every frame, so what the screen shows is a pure function of the script the
/// timeline has accumulated so far. That is what makes a fast-drain and a
/// played-out beat land on identical pixels.
pub fn draw_panel_messages(
    fb: &mut Framebuffer,
    font: &Font,
    board: &PresentedBoard,
    ops: &[PanelOp],
) {
    let mut cursor = TextCursor {
        col: PANEL_COL,
        row: MESSAGE_REGION.y_start,
    };
    let mut mark = MESSAGE_REGION.y_start;
    let resolve = |row: &Row, mark: usize| match row {
        Row::At(r) => *r,
        Row::FromMark(delta) => mark + delta,
    };
    for op in ops {
        match op {
            PanelOp::Clear => clear_message_region(fb),
            PanelOp::Status { row, who, text } => {
                let row = resolve(row, mark);
                if row > MESSAGE_REGION.y_end {
                    continue;
                }
                // draw8x8_clear_area(0x15, 0x26, lineY, 0x17).
                cell_rect_fill(
                    fb,
                    0,
                    row,
                    MESSAGE_REGION.y_end,
                    MESSAGE_REGION.x_start,
                    MESSAGE_REGION.x_end,
                );
                draw_name(fb, font, board, *who, row);
                cursor = TextCursor {
                    col: PANEL_COL,
                    row: row + 1,
                };
                let region = crate::text::TextRegion {
                    y_start: row + 1,
                    y_end: MESSAGE_REGION.y_end,
                    x_start: PANEL_COL,
                    x_end: MESSAGE_REGION.x_end,
                };
                run_text(fb, font, text, LABEL_COLOR, region, &mut cursor);
            }
            PanelOp::Name { row, who } => {
                let row = resolve(row, mark);
                if row <= MESSAGE_REGION.y_end {
                    draw_name(fb, font, board, *who, row);
                }
            }
            PanelOp::Wrapped { row, text } => {
                let row = resolve(row, mark);
                if row > MESSAGE_REGION.y_end {
                    continue;
                }
                cursor = TextCursor {
                    col: PANEL_COL,
                    row,
                };
                let region = crate::text::TextRegion {
                    y_start: row,
                    y_end: (row + 3).min(MESSAGE_REGION.y_end),
                    x_start: PANEL_COL,
                    x_end: MESSAGE_REGION.x_end,
                };
                run_text(fb, font, text, LABEL_COLOR, region, &mut cursor);
            }
            PanelOp::Line { row, text } => {
                let row = resolve(row, mark);
                if row <= MESSAGE_REGION.y_end {
                    draw_string(fb, font, text, row, PANEL_COL, 0, LABEL_COLOR);
                }
            }
            PanelOp::Mark => mark = cursor.row + 1,
        }
    }
}

/// `displayPlayerName(false, row, 0x17, who)` off the presented roster — an
/// id the board doesn't hold prints nothing, which is the original's own
/// null-player behavior.
fn draw_name(fb: &mut Framebuffer, font: &Font, board: &PresentedBoard, who: usize, row: usize) {
    if let Some(c) = board.combatant(who) {
        draw_string(
            fb,
            font,
            &c.name,
            row,
            PANEL_COL,
            0,
            name_color(c.in_combat, c.team),
        );
    }
}

/// One overlay sprite placement — `OverlayBounded(missile_dax, 5, frame,
/// cur.y, cur.x)` (`seg040.cs:29-31`), whose cell coordinates are 8-pixel
/// units off the viewport origin rather than 24-pixel combat cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayDraw {
    pub sprite: SpriteRef,
    pub cell_x: i32,
    pub cell_y: i32,
}

/// Blits the overlay sprites currently up, over everything else in the
/// viewport and under the same hard clip (`draw_combat_picture`'s
/// `[8,176)²`). An unloaded icon slot draws nothing, as everywhere else.
pub fn draw_overlays(fb: &mut Framebuffer, art: &SceneArt, overlays: &[OverlayDraw]) {
    for o in overlays {
        let Some(icon) = art.icons.get(o.sprite.icon_slot) else {
            continue;
        };
        let frame = icon.frame(o.sprite.pose, o.sprite.direction());
        blit_sprite_px(
            fb,
            frame,
            o.cell_x * 8 + VIEWPORT_ORIGIN_PX.0 as i32,
            o.cell_y * 8 + VIEWPORT_ORIGIN_PX.1 as i32,
        );
    }
}

/// `string_print01`'s print half (`ovr025.cs:775-784`): the prompt row, all 40
/// columns, colour 10 — cleared first, exactly as `ClearPromptAreaNoUpdate`
/// leaves it.
pub fn draw_prompt(fb: &mut Framebuffer, font: &Font, text: &str) {
    clear_prompt_line(fb);
    draw_string(fb, font, text, PROMPT_ROW, 0, 0, LABEL_COLOR);
}

/// The status row (`ovr023.cs:3122`'s "Spell:<name>", `ovr014.cs:1760`'s
/// "Range = N") — row 0x17 from column 0, cleared first.
pub fn draw_status_line(fb: &mut Framebuffer, font: &Font, text: &str) {
    clear_status_line(fb);
    draw_string(fb, font, text, STATUS_ROW, 0, 0, LABEL_COLOR);
}
