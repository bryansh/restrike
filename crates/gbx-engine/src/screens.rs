//! The M3 step-6 party-facing menu screens — character sheet / party view
//! (`viewPlayer`/`selectAPlayer`, `ovr020`/`ovr025`), camp (`ovr016`),
//! save/load (`ovr017`), training (`ovr018`), and shops (`ovr007`).
//!
//! These are additive [`Shell`](crate::shell::Shell) states (M2's walk-loop
//! flows are untouched): each is a parked-widget screen that renders itself
//! every tick and advances one queued key at a time (D-UI1 — no blocking
//! loops), exactly like the M2 widgets. A single [`Shell::Screen`] variant
//! delegates here so `shell.rs` stays focused on the walk loop.
//!
//! Derived by reading coab for behavior (D11, never copied); citations are
//! per-screen below.

use crate::charsheet::{render_sheet, sheet_view};
use crate::shell::FlowCtx;
use crate::widgets::{Hotbar, Widget, WidgetOutcome};

/// Where a screen returns when the player exits it — the walk loop's world
/// menu, or the camp menu (camp sub-screens return to camp, `ovr016`'s own
/// re-display loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReturnTo {
    World,
    Camp,
}

/// One screen's result this tick.
pub enum ScreenTransition {
    /// Stay on this screen (still waiting on input / mid-animation).
    Stay,
    /// Leave the screen system entirely → the walk-loop world menu.
    Exit,
    /// Replace this screen with another (sub-menu entry, or a sub-screen
    /// returning to its parent).
    To(Screen),
}

/// The M3 party-facing screens. `Shell::Screen` holds one of these.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Screen {
    PartyView(PartyView),
    Camp(Camp),
    Magic(MagicMenu),
}

impl Screen {
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        match self {
            Screen::PartyView(s) => s.tick(ctx),
            Screen::Camp(s) => s.tick(ctx),
            Screen::Magic(s) => s.tick(ctx),
        }
    }
}

/// Paints a simple full-screen menu backdrop: clear, outer frame, a title in
/// the top-left, and the command bar along the bottom row. Shared by the camp
/// and magic menus (their own richer layouts are cosmetic polish; the
/// dispatch is the load-bearing part these deliverables prove).
fn draw_menu_backdrop(ctx: &mut FlowCtx, title: &str, bar: &str, status: Option<&str>) {
    ctx.fb.clear(0);
    let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
    crate::text::draw_string(ctx.fb, ctx.font, title, 1, 1, 0, 0x0F);
    if let Some(status) = status {
        crate::text::draw_string(ctx.fb, ctx.font, status, 3, 1, 0, 0x0A);
    }
    crate::text::draw_string(ctx.fb, ctx.font, bar, 24, 1, 0, 0x0F);
}

// --- Party view / character sheet (viewPlayer, ovr020.cs:236-339) ---

/// The character-sheet screen: renders the selected member's full sheet and
/// parks the dynamic command bar (`Items Spells Trade Drop … Exit`). Extended
/// keys scroll the selected player (`selectAPlayer`'s `O`/`G` next/prev,
/// `ovr025.cs:1540-1556`) rather than resolving the bar.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PartyView {
    selected: usize,
    menu: Widget,
    return_to: ReturnTo,
}

impl PartyView {
    /// Opens on the currently-selected player (`gbl.SelectedPlayer`), clamped
    /// into range. Empty roster is handled by [`PartyView::tick`] (immediate
    /// exit), not here.
    pub fn new(ctx: &FlowCtx, return_to: ReturnTo) -> Self {
        let count = ctx.roster.members.len();
        let selected = (ctx.state.selected_player as usize).min(count.saturating_sub(1));
        let menu = Self::build_menu(ctx, selected);
        PartyView {
            selected,
            menu,
            return_to,
        }
    }

    fn build_menu(ctx: &FlowCtx, selected: usize) -> Widget {
        let bar = ctx
            .roster
            .members
            .get(selected)
            .map(|m| sheet_view(m).command_bar)
            .unwrap_or_else(|| "Exit".to_string());
        let mut hotbar = Hotbar::new(bar);
        // Extended keys (arrows/numpad) scroll the party rather than resolving
        // — the same `ext_scrolls_party` behavior the M2 encounter menus use.
        hotbar.accept_ext = true;
        hotbar.ext_scrolls_party = true;
        Widget::Hotbar(hotbar)
    }

    fn exit(&self, ctx: &FlowCtx) -> ScreenTransition {
        match self.return_to {
            ReturnTo::World => ScreenTransition::Exit,
            ReturnTo::Camp => ScreenTransition::To(Screen::Camp(Camp::new(ctx))),
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let count = ctx.roster.members.len();
        if count == 0 {
            // Nothing to view (no party imported/created) — leave immediately.
            return match self.return_to {
                ReturnTo::World => ScreenTransition::Exit,
                ReturnTo::Camp => ScreenTransition::To(Screen::Camp(Camp::new(ctx))),
            };
        }
        self.selected = self.selected.min(count - 1);

        // Render the sheet fresh each tick (immediate-mode, D-UI4).
        let view = sheet_view(&ctx.roster.members[self.selected]);
        ctx.fb.clear(0);
        render_sheet(ctx.fb, ctx.font, ctx.symbols, &view);

        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Pending => ScreenTransition::Stay,
            WidgetOutcome::PartyScroll(code) => {
                // 'H'/up (and numpad-8) = previous, 'P'/down (numpad-2) = next,
                // wrapping over the roster (selectAPlayer's O/G, ovr025.cs).
                match code {
                    b'H' => self.selected = (self.selected + count - 1) % count,
                    b'P' => self.selected = (self.selected + 1) % count,
                    _ => {}
                }
                ctx.state.selected_player = self.selected as u8;
                self.menu = Self::build_menu(ctx, self.selected);
                ScreenTransition::Stay
            }
            WidgetOutcome::Hotbar(key) => match key.to_ascii_uppercase() {
                // Exit ('E') or Escape ('\0') leaves the sheet.
                b'E' | 0 => self.exit(ctx),
                // Items/Spells/Trade/Drop/Heal/Cure: parked for later
                // deliverables (item-name decode, spell casting, trade UI).
                // Staying re-prompts, matching viewPlayer's own re-display
                // loop rather than silently exiting. TODO(M3+/M5): wire the
                // items list (PlayerItemsMenu) and paladin heal/cure.
                _ => ScreenTransition::Stay,
            },
            _ => ScreenTransition::Stay,
        }
    }
}

// --- Camp (ovr016.cs, MakeCamp) ---

/// The camp menu (`MakeCamp`, `ovr016.cs:1080`): the command bar
/// `Save View Magic Rest Alter Fix Exit` (`ovr016.cs:1103`), re-displayed
/// after each sub-action (`ovr016.cs`'s own `while` loop). A parked command
/// bar dispatching to the party screens, the magic submenu, and the rest
/// action.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Camp {
    menu: Widget,
    /// A one-line status shown under the menu after an action — engine-owned,
    /// not game prose.
    status: Option<String>,
}

impl Camp {
    pub fn new(_ctx: &FlowCtx) -> Self {
        Camp {
            menu: menu_bar("Save View Magic Rest Alter Fix Exit"),
            status: None,
        }
    }

    /// Rebuilds camp carrying a post-action status line (`ovr016`'s
    /// re-display after each dispatch).
    pub fn with_status(status: impl Into<String>) -> Self {
        Camp {
            menu: menu_bar("Save View Magic Rest Alter Fix Exit"),
            status: Some(status.into()),
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        draw_menu_backdrop(
            ctx,
            "Camp",
            "Save View Magic Rest Alter Fix Exit",
            self.status.as_deref(),
        );
        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Pending => ScreenTransition::Stay,
            WidgetOutcome::Hotbar(key) => self.dispatch(key, ctx),
            _ => ScreenTransition::Stay,
        }
    }

    fn dispatch(&mut self, key: u8, ctx: &mut FlowCtx) -> ScreenTransition {
        match key.to_ascii_uppercase() {
            // View → the character sheet (ovr016.cs:1122 → ovr020.viewPlayer).
            b'V' => ScreenTransition::To(Screen::PartyView(PartyView::new(ctx, ReturnTo::Camp))),
            // Magic → the memorize/scribe submenu (ovr016.cs:1127 → magic_menu).
            b'M' => ScreenTransition::To(Screen::Magic(MagicMenu::new())),
            // Rest → the memorization commit (ovr016.cs:1132 → rest_menu). See
            // [`party_rest`] for the M4/M5 deferral (FD-25).
            b'R' => {
                *self = Camp::with_status(party_rest(ctx));
                ScreenTransition::Stay
            }
            // Save → the save/load menu (ovr016.cs:1114 → ovr017.SaveGame).
            // Wired to the save/load screen by deliverable 3; a placeholder
            // status until then keeps camp navigable.
            b'S' => {
                *self = Camp::with_status("Save/Load: menu lands in deliverable 3");
                ScreenTransition::Stay
            }
            // Alter (Order/Drop/Speed/Icon, ovr016.cs:1141 → alter_menu) and
            // Fix (auto-heal via cure spells + rest, ovr016.cs:1137 →
            // FixTeam): stubbed. Alter's Order/Drop are simple roster edits
            // but need a select-player sub-UI; Fix couples cure-spell tallies
            // to the same deferred rest/healing machinery as Rest (FD-25).
            // TODO(M3+): Alter ▸ Order/Drop roster ops; TODO(M4/M5): Fix.
            b'A' => {
                *self = Camp::with_status("Alter: party order/drop — TODO");
                ScreenTransition::Stay
            }
            b'F' => {
                *self = Camp::with_status("Fix: auto-heal needs rest/healing (M4/M5)");
                ScreenTransition::Stay
            }
            // Exit ('E') or Escape ('\0') leaves camp for the walk loop
            // (ovr016.cs:1075's `Set(0, 69)` loop-exit set).
            b'E' | 0 => ScreenTransition::Exit,
            _ => ScreenTransition::Stay,
        }
    }
}

/// The camp Rest action (`rest_menu`/`resting`, `ovr016.cs:274`/`ovr021.cs:516`).
///
/// **Faithfully a documented deferral this milestone (FD-25).** The original's
/// rest does three things: advance the clock (`step_game_time`), heal 1 HP per
/// time-tick (`rest_heal`), and commit each caster's *staged pending*
/// memorizations pending → memorized (`rest_memorize` → `SpellList.MarkLearnt`,
/// `ovr021.cs:403`). It never resets `spellCastCount` (a fixed capacity) — the
/// task brief's "spell-slot restoration" framing does not match the original
/// (FD-25). The clock/healing halves are the PLAN's deferred "time effects"
/// (M4); the memorization commit needs the `SpellList` Learning-flag decode +
/// the Magic ▸ Memorize staging path, both **M5 (Vancian)** scope, and our
/// `party::MagicState` carries `spell_list`/`cast_count` raw. With no staging
/// path yet, the pending list is always empty and a faithful commit is a
/// no-op — which is *correct* for the bundled save (it stages nothing). So
/// this reports the rest without mutating spell state, rather than faking a
/// restoration the original never performs.
fn party_rest(ctx: &FlowCtx) -> String {
    let casters = ctx
        .roster
        .members
        .iter()
        .filter(|m| m.magic.cast_count.iter().flatten().any(|&c| c > 0))
        .count();
    format!("The party rests. ({casters} caster(s); memorization: M5)")
}

// --- Magic submenu (ovr016.cs:600, magic_menu) ---

/// The Magic submenu (`magic_menu`, `ovr016.cs:600`): the command bar
/// `Cast Memorize Scribe Display Rest Exit` (`ovr016.cs:608`). Its leaves are
/// Vancian-memorization work scheduled for M5 (FD-25) — this milestone lands
/// the navigable menu structure with each leaf reporting the deferral, plus
/// Rest wired through to [`party_rest`], and Exit back to camp.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MagicMenu {
    menu: Widget,
    status: Option<String>,
}

impl MagicMenu {
    pub fn new() -> Self {
        MagicMenu {
            menu: menu_bar("Cast Memorize Scribe Display Rest Exit"),
            status: None,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        draw_menu_backdrop(
            ctx,
            "Magic",
            "Cast Memorize Scribe Display Rest Exit",
            self.status.as_deref(),
        );
        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Pending => ScreenTransition::Stay,
            WidgetOutcome::Hotbar(key) => match key.to_ascii_uppercase() {
                // Rest here calls the same rest action (ovr016.cs:635).
                b'R' => ScreenTransition::To(Screen::Camp(Camp::with_status(party_rest(ctx)))),
                // Exit ('E') / Escape back to camp (ovr016.cs:831's Set(0,69)).
                b'E' | 0 => ScreenTransition::To(Screen::Camp(Camp::new(ctx))),
                // Cast/Memorize/Scribe/Display: M5 (Vancian) — see FD-25.
                b'C' => {
                    self.status = Some("Cast: spell effects are M5".into());
                    ScreenTransition::Stay
                }
                b'M' => {
                    self.status = Some("Memorize: staging is M5 (FD-25)".into());
                    ScreenTransition::Stay
                }
                b'S' => {
                    self.status = Some("Scribe: scroll learning is M5".into());
                    ScreenTransition::Stay
                }
                b'D' => {
                    self.status = Some("Display: affect list needs the affect model (M4)".into());
                    ScreenTransition::Stay
                }
                _ => ScreenTransition::Stay,
            },
            _ => ScreenTransition::Stay,
        }
    }
}

impl Default for MagicMenu {
    fn default() -> Self {
        Self::new()
    }
}

/// A horizontal command bar of first-letter-selectable words, extended-key
/// aware (the M2 `displayInput`/`HorizontalMenu` vocabulary).
fn menu_bar(text: &str) -> Widget {
    let mut hotbar = Hotbar::new(text);
    hotbar.accept_ext = true;
    Widget::Hotbar(hotbar)
}
