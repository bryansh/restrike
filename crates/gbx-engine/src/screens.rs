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
use crate::saveload::{SaveLoadRequest, SlotStatus};
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
    SaveLoad(SaveLoad),
    Training(Training),
    Shop(Shop),
}

impl Screen {
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        match self {
            Screen::PartyView(s) => s.tick(ctx),
            Screen::Camp(s) => s.tick(ctx),
            Screen::Magic(s) => s.tick(ctx),
            Screen::SaveLoad(s) => s.tick(ctx),
            Screen::Training(s) => s.tick(ctx),
            Screen::Shop(s) => s.tick(ctx),
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
            b'S' => ScreenTransition::To(Screen::SaveLoad(SaveLoad::new(ReturnTo::Camp))),
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

// --- Save / Load (ovr017.cs SaveGame/loadGameMenu) ---

/// Whether the slot picker is choosing a slot to save into or load from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SlMode {
    Save,
    Load,
}

/// The save/load screen (`SaveGame`/`loadGameMenu`, `ovr017.cs:1109`/`929`).
/// Two steps: a `Save Load Exit` chooser, then a lettered-slot picker (all ten
/// A-J for Save, `ovr017.cs:1117`; only occupied slots for Load,
/// `ovr017.cs:935-941`). Picking a slot sets a [`SaveLoadRequest`] the host
/// fulfills after the tick (D8 — the core never does file I/O).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SaveLoad {
    phase: SlPhase,
    menu: Widget,
    return_to: ReturnTo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum SlPhase {
    Choose,
    Pick(SlMode),
}

impl SaveLoad {
    pub fn new(return_to: ReturnTo) -> Self {
        SaveLoad {
            phase: SlPhase::Choose,
            menu: menu_bar("Save Load Exit"),
            return_to,
        }
    }

    fn exit(&self) -> ScreenTransition {
        match self.return_to {
            ReturnTo::World => ScreenTransition::Exit,
            // Rebuild camp fresh (its menu is stateless); a status hint would
            // need the outcome of the host's I/O, which isn't known here.
            ReturnTo::Camp => ScreenTransition::To(Screen::Camp(camp_after_saveload())),
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        // Backdrop: title, the per-slot status list, then the command bar.
        let bar = match self.phase {
            SlPhase::Choose => "Save Load Exit".to_string(),
            SlPhase::Pick(SlMode::Save) => "A B C D E F G H I J".to_string(),
            SlPhase::Pick(SlMode::Load) => load_bar_text(ctx),
        };
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        crate::text::draw_string(ctx.fb, ctx.font, "Save / Load", 1, 1, 0, 0x0F);
        for (i, (letter, status)) in ctx.slots.entries().enumerate() {
            let line = format!("{letter}  {}", status.label());
            crate::text::draw_string(ctx.fb, ctx.font, &line, 3 + i, 1, 0, 0x0A);
        }
        crate::text::draw_string(ctx.fb, ctx.font, &bar, 24, 1, 0, 0x0F);

        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Pending => ScreenTransition::Stay,
            WidgetOutcome::Hotbar(key) => self.dispatch(key, ctx),
            _ => ScreenTransition::Stay,
        }
    }

    fn dispatch(&mut self, key: u8, ctx: &mut FlowCtx) -> ScreenTransition {
        let key = key.to_ascii_uppercase();
        match self.phase {
            SlPhase::Choose => match key {
                b'S' => {
                    self.phase = SlPhase::Pick(SlMode::Save);
                    self.menu = menu_bar("A B C D E F G H I J");
                    ScreenTransition::Stay
                }
                b'L' => {
                    self.phase = SlPhase::Pick(SlMode::Load);
                    self.menu = menu_bar(&load_bar_text(ctx));
                    ScreenTransition::Stay
                }
                b'E' | 0 => self.exit(),
                _ => ScreenTransition::Stay,
            },
            SlPhase::Pick(mode) => {
                // Escape / null returns to the chooser rather than leaving.
                if key == 0 {
                    self.phase = SlPhase::Choose;
                    self.menu = menu_bar("Save Load Exit");
                    return ScreenTransition::Stay;
                }
                // A load screen with no games shows only "Exit".
                if key == b'E'
                    && matches!(mode, SlMode::Load)
                    && ctx.slots.occupied_letters().is_empty()
                {
                    return self.exit();
                }
                if !crate::saveload::SLOT_LETTERS.contains(&(key as char)) {
                    return ScreenTransition::Stay;
                }
                let letter = key as char;
                let request = match mode {
                    SlMode::Save => Some(SaveLoadRequest::Save(letter)),
                    SlMode::Load => match ctx.slots.status(letter) {
                        // A one-way import for an original slot (D-SAVE12).
                        SlotStatus::OriginalSave => Some(SaveLoadRequest::ImportOriginal(letter)),
                        SlotStatus::RestrikeSave => Some(SaveLoadRequest::Load(letter)),
                        // An empty slot in Load mode: re-prompt (ovr017 only
                        // lists occupied letters, so this is belt-and-braces).
                        SlotStatus::Empty => None,
                    },
                };
                match request {
                    Some(req) => {
                        *ctx.io_request = Some(req);
                        self.exit()
                    }
                    None => ScreenTransition::Stay,
                }
            }
        }
    }
}

/// The Load-mode command bar: the occupied slot letters (`loadGameMenu`'s
/// `games_list`), or just `Exit` when nothing is saved.
fn load_bar_text(ctx: &FlowCtx) -> String {
    let occupied = ctx.slots.occupied_letters();
    if occupied.is_empty() {
        "Exit".to_string()
    } else {
        occupied
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Camp rebuilt after leaving save/load — the host fulfills the request
/// between ticks, so the confirmation is left to the frontend (this keeps the
/// core free of the I/O outcome).
fn camp_after_saveload() -> Camp {
    Camp::with_status("Save/Load: request sent to host")
}

// --- Training hall (ovr018.cs train_player) ---

/// The training-hall screen (`train_player`, `ovr018.cs:2189`): shows the
/// selected character's sheet, a "will become" preview, and a `Train Exit`
/// command bar (`ovr018.cs:2371`'s "Do you wish to train?"). Training is an
/// unrestricted hall ([`crate::training::TRAINS_ALL_CLASSES`]) — a specific
/// hall's `training_class_mask` comes from the town ECL (M6).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Training {
    selected: usize,
    menu: Widget,
    status: Option<String>,
    return_to: ReturnTo,
}

impl Training {
    /// Opens on `selected_player` (clamped into range in [`Training::tick`]).
    /// Takes a plain index, not a `FlowCtx`, so a frontend can enter it from a
    /// town training-hall tile via [`Engine::open_training`](crate::engine::Engine::open_training).
    pub fn new(selected_player: u8, return_to: ReturnTo) -> Self {
        Training {
            selected: selected_player as usize,
            menu: menu_bar("Train Exit"),
            status: None,
            return_to,
        }
    }

    fn exit(&self, ctx: &FlowCtx) -> ScreenTransition {
        match self.return_to {
            ReturnTo::World => ScreenTransition::Exit,
            ReturnTo::Camp => ScreenTransition::To(Screen::Camp(Camp::new(ctx))),
        }
    }

    /// The "will become a level N {class}" preview, or the reason training is
    /// unavailable (`ovr018.cs:2346-2368`/the early-return messages).
    fn preview(&self, ctx: &FlowCtx) -> String {
        let ch = &ctx.roster.members[self.selected];
        let trainable = crate::training::trainable_classes(ch, ctx.rules, TRAINS_ALL);
        if !trainable.is_empty() {
            let parts: Vec<String> = trainable
                .iter()
                .map(|&c| format!("level {} {}", ch.class_level[c] + 1, class_name(c)))
                .collect();
            format!("{} will become: {}", ch.name, parts.join(", "))
        } else if !crate::money::can_afford(&ch.money, crate::training::TRAINING_FEE_GP, ctx.rules)
        {
            "Training costs 1000 gp.".to_string()
        } else {
            "Not enough experience.".to_string()
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let count = ctx.roster.members.len();
        if count == 0 {
            return self.exit(ctx);
        }
        self.selected = self.selected.min(count - 1);

        // Render the sheet, then the preview + status + command bar over it.
        let view = sheet_view(&ctx.roster.members[self.selected]);
        ctx.fb.clear(0);
        render_sheet(ctx.fb, ctx.font, ctx.symbols, &view);
        let preview = self.preview(ctx);
        crate::text::draw_string(ctx.fb, ctx.font, &preview, 20, 1, 0, 0x0A);
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 21, 1, 0, 0x0E);
        }
        crate::text::draw_string(ctx.fb, ctx.font, "Train Exit", 24, 1, 0, 0x0F);

        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Pending => ScreenTransition::Stay,
            WidgetOutcome::PartyScroll(code) => {
                match code {
                    b'H' => self.selected = (self.selected + count - 1) % count,
                    b'P' => self.selected = (self.selected + 1) % count,
                    _ => {}
                }
                ctx.state.selected_player = self.selected as u8;
                self.status = None;
                ScreenTransition::Stay
            }
            WidgetOutcome::Hotbar(key) => match key.to_ascii_uppercase() {
                b'T' => {
                    let member = &mut ctx.roster.members[self.selected];
                    self.status = Some(
                        match crate::training::train(member, ctx.rules, ctx.rng, TRAINS_ALL) {
                            Ok(o) => {
                                let (class, level) = o.advanced.first().copied().unwrap_or((0, 0));
                                format!(
                                    "Trained: level {} {} (+{} HP)",
                                    level,
                                    class_name(class),
                                    o.hp_gained
                                )
                            }
                            Err(e) => train_error_text(e),
                        },
                    );
                    ScreenTransition::Stay
                }
                b'E' | 0 => self.exit(ctx),
                _ => ScreenTransition::Stay,
            },
            _ => ScreenTransition::Stay,
        }
    }
}

const TRAINS_ALL: u8 = crate::training::TRAINS_ALL_CLASSES;

/// A single-word class name for the training preview (the class-name table is
/// engine-side in `charsheet`; this maps the base-class index to it).
fn class_name(class: usize) -> &'static str {
    const NAMES: [&str; 8] = [
        "Cleric",
        "Druid",
        "Fighter",
        "Paladin",
        "Ranger",
        "Magic-User",
        "Thief",
        "Monk",
    ];
    NAMES.get(class).copied().unwrap_or("?")
}

fn train_error_text(e: crate::training::TrainError) -> String {
    use crate::training::TrainError::*;
    match e {
        NotConscious => "We only train conscious people.".to_string(),
        NotEnoughGold => "Training costs 1000 gp.".to_string(),
        WrongClassHere => "We don't train that class here.".to_string(),
        NotEnoughExperience => "Not enough experience.".to_string(),
    }
}

// --- Shop (ovr007.cs CityShop) ---

/// The shop screen (`CityShop`, `ovr007.cs`): the command bar
/// `Buy View Take Pool Share Appraise Exit` (`ovr007.cs:181-185`), and a Buy
/// sub-list of stock with prices. The buyer is the currently-selected player.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Shop {
    shop: crate::shop::Shop,
    phase: ShopPhase,
    menu: Widget,
    status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ShopPhase {
    Menu,
    Buy,
}

impl Shop {
    pub fn new(shop: crate::shop::Shop) -> Self {
        Shop {
            shop,
            phase: ShopPhase::Menu,
            menu: menu_bar("Buy View Take Pool Share Appraise Exit"),
            status: None,
        }
    }

    fn buy_list(&self) -> Widget {
        let items: Vec<crate::widgets::ListItem> = self
            .shop
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let price = self.shop.price(i).unwrap_or(0);
                crate::widgets::ListItem::Entry(format!("{}  {} gp", it.name(), price))
            })
            .collect();
        Widget::ListMenu(crate::widgets::ListMenu::new(items, 8))
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        // Backdrop.
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        crate::text::draw_string(ctx.fb, ctx.font, "Shop", 1, 1, 0, 0x0F);
        // Stock listing for context.
        for (i, it) in self.shop.items.iter().enumerate() {
            let price = self.shop.price(i).unwrap_or(0);
            let line = format!("{}  {} gp", it.name(), price);
            crate::text::draw_string(ctx.fb, ctx.font, &line, 3 + i, 2, 0, 0x0A);
        }
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 22, 1, 0, 0x0E);
        }
        let bar = match self.phase {
            ShopPhase::Menu => "Buy View Take Pool Share Appraise Exit",
            ShopPhase::Buy => "Buy an item (Esc to cancel)",
        };
        crate::text::draw_string(ctx.fb, ctx.font, bar, 24, 1, 0, 0x0F);

        match self.phase {
            ShopPhase::Menu => match self.menu.tick(ctx.input, ctx.dt_ticks) {
                WidgetOutcome::Pending => ScreenTransition::Stay,
                WidgetOutcome::Hotbar(key) => self.dispatch_menu(key),
                _ => ScreenTransition::Stay,
            },
            ShopPhase::Buy => match self.menu.tick(ctx.input, ctx.dt_ticks) {
                WidgetOutcome::Pending => ScreenTransition::Stay,
                WidgetOutcome::ListSelected { index, .. } => {
                    self.status = Some(self.buy(index, ctx));
                    ScreenTransition::Stay
                }
                WidgetOutcome::ListCancelled => {
                    self.phase = ShopPhase::Menu;
                    self.menu = menu_bar("Buy View Take Pool Share Appraise Exit");
                    ScreenTransition::Stay
                }
                _ => ScreenTransition::Stay,
            },
        }
    }

    fn dispatch_menu(&mut self, key: u8) -> ScreenTransition {
        match key.to_ascii_uppercase() {
            b'B' => {
                if self.shop.items.is_empty() {
                    self.status = Some("Nothing for sale.".into());
                } else {
                    self.phase = ShopPhase::Buy;
                    self.menu = self.buy_list();
                }
                ScreenTransition::Stay
            }
            // View/Take/Pool/Share/Appraise: Pool/Share are trivial coin
            // aggregation but need a multi-member select UI; Take handles
            // on-ground treasure; Appraise runs a gem-valuation dialog. All
            // stubbed with a status. TODO(M3+): Pool/Share coin ops;
            // TODO(M4): View (char sheet from a shop), Take, Appraise.
            b'V' => {
                self.status = Some("View: character sheet — TODO".into());
                ScreenTransition::Stay
            }
            b'T' | b'P' | b'S' | b'A' => {
                self.status = Some("Take/Pool/Share/Appraise — TODO".into());
                ScreenTransition::Stay
            }
            b'E' | 0 => ScreenTransition::Exit,
            _ => ScreenTransition::Stay,
        }
    }

    /// Buys `index` for the selected player (`shop_buy`, `ovr007.cs:106`),
    /// returning a status line.
    fn buy(&mut self, index: usize, ctx: &mut FlowCtx) -> String {
        let buyer_idx =
            (ctx.state.selected_player as usize).min(ctx.roster.members.len().saturating_sub(1));
        let Some(buyer) = ctx.roster.members.get_mut(buyer_idx) else {
            return "No one to buy for.".to_string();
        };
        match crate::shop::buy(&self.shop, index, buyer, ctx.rules) {
            Ok(o) => format!("Bought {} for {} gp.", o.item_name, o.price),
            Err(crate::shop::BuyError::NotEnoughMoney) => "Not enough money.".to_string(),
            Err(crate::shop::BuyError::NoSuchItem) => "No such item.".to_string(),
        }
    }
}

/// A horizontal command bar of first-letter-selectable words, extended-key
/// aware (the M2 `displayInput`/`HorizontalMenu` vocabulary).
fn menu_bar(text: &str) -> Widget {
    let mut hotbar = Hotbar::new(text);
    hotbar.accept_ext = true;
    Widget::Hotbar(hotbar)
}
