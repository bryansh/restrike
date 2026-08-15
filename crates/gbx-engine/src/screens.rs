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
    /// The death screen (`Shell::GameOver`) — the save/load screen opened as
    /// the party-wipe recovery. Declining the load goes back to the death
    /// screen, never into the world: the original's post-wipe `startGameMenu`
    /// has no `BEGIN Adventuring` to press (`ovr018.cs:103-114`).
    GameOver,
    /// ★ Roll-credits slice 9a: opened from inside `CityShop`'s own loop
    /// (`ovr007.cs:199` `viewPlayer()`). Appended last, so postcard keeps the
    /// three above at their indices.
    ///
    /// Load-bearing beyond "where does Exit go": it is also this shell's
    /// `gbl.game_state == GameState.Shop`, the condition that puts `Sell` and
    /// `Id` on the Items leaf's bar (`ovr020.cs:484-493`).
    Shop,
    /// ★ Roll-credits slice 9b: opened from `startGameMenu` — its `L`, `S`,
    /// `V` and `T` verbs all call a screen that plainly `return`s into the
    /// menu's own `while (true)` (`ovr018.cs:193-237`). Appended last so
    /// postcard keeps the four above at their indices.
    StartMenu,
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
    /// Back to the death screen ([`ReturnTo::GameOver`]).
    ToGameOver,
    /// ★ `MakeCamp` returned `actionInterrupted` (roll-credits §8, FD-44):
    /// the rest-encounter check fired, so camp ends and `TryEncamp` runs the
    /// resident block's **vector 3**, `CampInterruptedAddr`
    /// (`ovr003.cs:1920`) — the block's own camp-ambush script.
    CampInterrupted,
    /// ★ Roll-credits slice 9b: back to `startGameMenu` ([`ReturnTo::StartMenu`]).
    ToStartMenu,
}

/// The M3 party-facing screens. `Shell::Screen` holds one of these.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Screen {
    PartyView(PartyView),
    Camp(Camp),
    Magic(MagicMenu),
    SaveLoad(SaveLoad),
    Training(Training),
    Shop(Box<crate::shop_screen::ShopHost>),
    // ★ Roll-credits slice 4 (Vancian camp magic). Appended at the END on
    // purpose: postcard encodes an enum variant as its index, so the six
    // above keep their encodings and no committed `.rsav` moves.
    // Boxed: these four carry list rows and a whole rest session, and
    // `ScreenTransition::To(Screen)` is passed by value on every transition.
    Memorize(Box<crate::camp_magic::MemorizeScreen>),
    Scribe(Box<crate::camp_magic::ScribeScreen>),
    SpellEffects(Box<crate::camp_magic::SpellEffectsScreen>),
    Rest(Box<crate::camp_magic::RestScreen>),
    /// ★ Roll-credits slice 5 (out-of-combat casting). Appended last for the
    /// same reason the four above were: postcard encodes a variant as its
    /// index, so every committed `.rsav` keeps its encoding and no golden moves.
    Cast(Box<crate::camp_cast::CastScreen>),
    /// ★ Roll-credits slice 8 (out-of-combat item use, G6). Appended at the
    /// end for the same postcard reason — no committed `.rsav` moves, so no
    /// `SAVE_FORMAT_VERSION` bump.
    Items(Box<crate::items_screen::ItemsScreen>),
    /// ★ Roll-credits slice 9c: `startGameMenu`'s four character verbs.
    /// Appended at the end, for the same postcard reason every variant above
    /// them was — no committed `.rsav` moves.
    CreateCharacter(Box<crate::create_screen::CreateCharacter>),
    ModifyCharacter(Box<crate::modify_screen::ModifyCharacter>),
    DualClass(Box<crate::modify_screen::DualClass>),
    AddCharacter(Box<crate::create_screen::AddCharacter>),
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
            Screen::Memorize(s) => s.tick(ctx),
            Screen::Scribe(s) => s.tick(ctx),
            Screen::SpellEffects(s) => s.tick(ctx),
            Screen::Rest(s) => s.tick(ctx),
            Screen::Cast(s) => s.tick(ctx),
            Screen::Items(s) => s.tick(ctx),
            Screen::CreateCharacter(s) => s.tick(ctx),
            Screen::ModifyCharacter(s) => s.tick(ctx),
            Screen::DualClass(s) => s.tick(ctx),
            Screen::AddCharacter(s) => s.tick(ctx),
        }
    }
}

/// `display_map_position_time` (`ovr025.cs:1476-1509`) drawn by a screen that
/// composes it itself: `draw8x8_clear_area(15, 0x26, 15, 17)` then
/// `displayString(output, 0, 10, 15, 17)`.
///
/// The engine-level per-tick line is suppressed while any screen is parked
/// ([`crate::shell::Shell::draws_engine_status_line`]), so a screen whose
/// original layout includes the line calls this from inside its own paint.
/// `camping` picks the `" camping"` suffix the `GameState.Camping` arm shows
/// (`ovr025.cs:1500-1503`).
pub fn draw_position_time(ctx: &mut FlowCtx, camping: bool) {
    crate::draw::cell_rect_fill(ctx.fb, 0, 15, 15, 17, 0x26);
    let text = crate::movement::position_time_text(
        ctx.state.pos,
        ctx.state.facing,
        &ctx.state.clock,
        ctx.state.search_mode(),
        camping,
    );
    crate::text::draw_string(ctx.fb, ctx.font, &text, 15, 17, 0, 10);
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

    /// [`PartyView::new`] without a [`FlowCtx`] — the bar is rebuilt on the
    /// first tick anyway (`tick` re-derives it whenever the selection moves),
    /// so an entry point that has only an index can open the sheet too.
    pub fn from_index(selected_player: u8, return_to: ReturnTo) -> Self {
        let mut hotbar = Hotbar::new("Exit");
        hotbar.accept_ext = true;
        hotbar.ext_scrolls_party = true;
        PartyView {
            selected: selected_player as usize,
            menu: Widget::Hotbar(hotbar),
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
            // Unreachable by construction (only the save/load screen is ever
            // opened from the death screen), but "back where you came from" is
            // the right answer for any screen that ever is.
            ReturnTo::GameOver => ScreenTransition::ToGameOver,
            // ★ The shop host owns the sub-screen (`ShopHost::Stage::Sub`) and
            // turns any exit back into `CityShop`'s own loop — `viewPlayer`
            // returns to its caller, it does not leave the shop.
            ReturnTo::Shop => ScreenTransition::Exit,
            // ★ Slice 9b: back into `startGameMenu`'s own loop.
            ReturnTo::StartMenu => ScreenTransition::ToStartMenu,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let count = ctx.roster.members.len();
        if count == 0 {
            // Nothing to view (no party imported/created) — leave immediately.
            return self.exit(ctx);
        }
        self.selected = self.selected.min(count - 1);

        // Render the sheet fresh each tick (immediate-mode, D-UI4).
        let view = sheet_view(&ctx.roster.members[self.selected]);
        ctx.fb.clear(0);
        render_sheet(ctx.fb, ctx.font, ctx.symbols, &view);

        // ★ `viewPlayer` recomputes its bar **every loop iteration**
        // (`ovr020.cs:252-291`), so a leaf that changes what the character has
        // — Items dropping the last item, say — is reflected the moment the
        // sheet comes back. Rebuild on change, carrying `gbl.menuSelectedWord`
        // across (`ovr027.cs:142-145`'s own clamp does the rest).
        if let Widget::Hotbar(h) = &self.menu {
            if h.text != view.command_bar {
                let stored = h.selected_word();
                self.menu = Self::build_menu(ctx, self.selected);
                if let Widget::Hotbar(h) = &mut self.menu {
                    h.seed_selected_word(stored);
                }
            }
        }

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
                // ★ Items is real since roll-credits slice 8 (`PlayerItemsMenu`,
                // `ovr020.cs:300`). `viewPlayer` keeps `gbl.SelectedPlayer`
                // pointing at whoever the sheet is showing, and the items menu
                // reads the same cell — so the scroll keys above have to have
                // written it, which they do.
                b'I' => {
                    ctx.state.selected_player = self.selected as u8;
                    crate::items_screen::ItemsScreen::open(ctx, self.return_to)
                }
                // Spells/Trade/Drop/Heal/Cure: parked for later deliverables
                // (the memorized-spell viewer, the coin trade/drop UI, and
                // paladin heal/cure's `CanCastHeal` gates). Staying re-prompts,
                // matching viewPlayer's own re-display loop rather than
                // silently exiting.
                _ => ScreenTransition::Stay,
            },
            _ => ScreenTransition::Stay,
        }
    }
}

// --- Camp (ovr016.cs, MakeCamp) ---

/// `MakeCamp`'s command bar (`ovr016.cs:1103`).
const CAMP_MENU: &str = "Save View Magic Rest Alter Fix Exit";
/// `displayInput`'s `displayExtraString` for that bar (`ovr016.cs:1103`) —
/// drawn at column 0 in `defaultMenuColors.prompt` (13, `Gbl.cs:189`), with
/// the menu text starting at `displayExtraString.Length`.
const CAMP_PROMPT: &str = "Camp:";
/// `ovr016.cs:1093`: `displayString("The party makes camp...", 0, 10, 18, 1)`.
const CAMP_ENTRY_TEXT: &str = "The party makes camp...";
/// `LoadPic`'s Camping arm loads `PIC{area}` block `0x1d` — the campfire
/// (`load_pic_final(ref gbl.byte_1D556, 0, 0x1d, "PIC")`, `ovr025.cs:1429`;
/// the file name is `file_name + gbl.game_area`, `ovr030.cs:58`).
const CAMP_PIC_BLOCK: u8 = 0x1D;
/// `defaultMenuColors.prompt` (`Gbl.cs:189`).
const PROMPT_COLOR: u8 = 13;

/// The campfire picture's first frame, decoded once and kept for the screen's
/// lifetime. **Never serialized** — a `.rsav` carries no pixels, and a restored
/// camp re-decodes on its first tick.
#[derive(Debug, Clone)]
pub struct CampPicture {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

/// The camp screen (`MakeCamp`, `ovr016.cs:1081-1160`): `LoadPic`'s Camping
/// composition (`ovr025.cs:1428-1433`) — the standard bordered layout, the
/// campfire picture in the viewport, the party roster down the right panel and
/// the position/time line — with `"The party makes camp..."` in the text area
/// and the `Camp:` command bar re-displayed after each sub-action
/// (`ovr016.cs`'s own `while` loop).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Camp {
    menu: Widget,
    /// A one-line status shown under the camp text after an action —
    /// engine-owned, not game prose.
    status: Option<String>,
    /// Decoded campfire pixels, filled on the first paint. Transient.
    #[serde(skip)]
    picture: Option<CampPicture>,
}

impl Camp {
    pub fn new(_ctx: &FlowCtx) -> Self {
        Camp {
            menu: camp_menu_bar(),
            status: None,
            picture: None,
        }
    }

    /// Rebuilds camp carrying a post-action status line (`ovr016`'s
    /// re-display after each dispatch).
    pub fn with_status(status: impl Into<String>) -> Self {
        Camp {
            menu: camp_menu_bar(),
            status: Some(status.into()),
            picture: None,
        }
    }

    /// `LoadPic()`'s Camping arm plus `MakeCamp`'s own two writes, repainted
    /// every tick (immediate mode, D-UI4 — the original composes it once and
    /// loops `displayInput` over it, which is the same picture).
    fn render(&mut self, ctx: &mut FlowCtx) {
        // `seg037.draw8x8_03()` (`ovr025.cs:1428`) — the standard bordered
        // layout, whose own inset clear wipes the previous screen's body.
        let _ = crate::frames::draw8x8_03(ctx.fb, ctx.symbols);
        self.draw_campfire(ctx);

        // `PartySummary(gbl.SelectedPlayer)` (`ovr025.cs:1430`).
        let rows: Vec<_> = ctx.roster.members.iter().map(sheet_view).collect();
        let selected =
            (!rows.is_empty()).then(|| (ctx.state.selected_player as usize) % rows.len());
        crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, selected);

        // `display_map_position_time()` (`ovr025.cs:1432`), with the
        // `game_state == Camping` suffix (`ovr025.cs:1500-1503`).
        draw_position_time(ctx, true);

        // `draw8x8_clear_area(TextRegion.NormalBottom)` then the entry line
        // (`ovr016.cs:1091-1093`).
        let region = crate::text::NORMAL_BOTTOM;
        crate::draw::cell_rect_fill(
            ctx.fb,
            0,
            region.y_start,
            region.y_end,
            region.x_start,
            region.x_end,
        );
        crate::text::draw_string(ctx.fb, ctx.font, CAMP_ENTRY_TEXT, 18, 1, 0, 10);
        if let Some(status) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, status, 20, 1, 0, 0x0E);
        }

        // `displayInput(…, CAMP_MENU, "Camp:")` (`ovr016.cs:1103`): the prompt
        // string at column 0, then `display_highlighed_text` from column
        // `displayExtraString.Length` (`ovr027.cs:158-163`).
        crate::text::draw_string(ctx.fb, ctx.font, CAMP_PROMPT, 0x18, 0, 0, PROMPT_COLOR);
        let span = match &self.menu {
            Widget::Hotbar(h) => h.selected_span(),
            _ => None,
        };
        crate::combat::scene::render::draw_menu_line_at(
            ctx.fb,
            ctx.font,
            CAMP_MENU,
            span,
            CAMP_PROMPT.len(),
        );
    }

    /// The campfire, blitted at the viewport cell the camp menu's own
    /// `displayInput` draws it on: `DrawMaybeOverlayed(picture, useOverlay =
    /// true, 3, 3)` (`ovr027.cs:181-184`, with `MakeCamp` passing
    /// `useOverlay = true` at `ovr016.cs:1103`) → `OverlayBounded(block, 0, 0,
    /// 2, 2)` → `draw_combat_picture(block, 3, 3, 0)`, which clips to pixels
    /// x 8..176 / y 8..176 (`seg040.cs:29-31,115-118` = [`Clip::OVERLAY`]).
    /// `load_pic_final`'s `masked` argument is 0 here, so nothing is
    /// transparent and no recolor runs.
    ///
    /// ★ **Seam for the concurrent scene-pictures slice.** Only frame 0 is
    /// drawn: `byte_1D556`'s multi-frame loop — the one `displayInput` advances
    /// per `CurrentDelay()` (`ovr027.cs:180-193`) — belongs with
    /// `Effect::Picture`'s general plumbing and the `picture_fade` recolor,
    /// which that slice owns. This is the smallest thing that puts the right
    /// pixels on the camp screen; the audit reconciles the two at merge.
    fn draw_campfire(&mut self, ctx: &mut FlowCtx) {
        draw_camp_picture(ctx, &mut self.picture);
    }

    /// The highlighted word's span into [`CAMP_MENU`] — a test/introspection
    /// seam over the bar's `gbl.menuSelectedWord`.
    pub fn selected_span(&self) -> Option<crate::widgets::WordRange> {
        match &self.menu {
            Widget::Hotbar(h) => h.selected_span(),
            _ => None,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        self.render(ctx);
        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Pending => ScreenTransition::Stay,
            // `special_key == true` → `scroll_team_list` + a `PartySummary`
            // redraw (`ovr016.cs:1105-1109`); the redraw is free here because
            // the screen repaints every tick.
            WidgetOutcome::PartyScroll(code) => {
                scroll_team_list(ctx, code);
                ScreenTransition::Stay
            }
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
            // [`party_rest`] for the M4/M5 deferral (FD-25). The menu itself is
            // NOT rebuilt: `MakeCamp`'s loop keeps the one `displayInput` it
            // opened with, so the highlighted word survives the action.
            b'R' => ScreenTransition::To(crate::camp_magic::RestScreen::open(ctx, false)),
            // Save → the save/load menu (ovr016.cs:1114 → ovr017.SaveGame).
            b'S' => ScreenTransition::To(Screen::SaveLoad(SaveLoad::new(ReturnTo::Camp))),
            // Alter (Order/Drop/Speed/Icon, ovr016.cs:1141 → alter_menu) and
            // Fix (auto-heal via cure spells + rest, ovr016.cs:1137 →
            // FixTeam): stubbed. Alter's Order/Drop are simple roster edits
            // but need a select-player sub-UI; Fix couples cure-spell tallies
            // to the same deferred rest/healing machinery as Rest (FD-25).
            // TODO(M3+): Alter ▸ Order/Drop roster ops; TODO(M4/M5): Fix.
            b'A' => {
                self.status = Some("Alter: party order/drop — TODO".into());
                ScreenTransition::Stay
            }
            // Fix (`FixTeam`, `ovr016.cs:1035-1073`): roll the memorized cures,
            // price the rest, then rest with `resting(false)` — no time menu.
            // A party at full health short-circuits with nothing rolled.
            b'F' => match crate::rest::plan_fix(ctx.roster, ctx.rng) {
                Some(plan) => {
                    ScreenTransition::To(crate::camp_magic::RestScreen::open_for_fix(ctx, plan))
                }
                None => ScreenTransition::Stay,
            },
            // Exit ('E') or Escape ('\0') leaves camp for the walk loop
            // (ovr016.cs:1075's `Set(0, 69)` loop-exit set).
            // ★ `MakeCamp`'s tail runs `cancel_spells` a second time
            // (`ovr016.cs:1154`), so staged-but-uncommitted memorizations and
            // scribes do not survive leaving camp.
            b'E' | 0 => {
                let scrolls = crate::camp_magic::camp_scrolls(ctx);
                crate::magic::cancel_spells(ctx.roster, &scrolls);
                ScreenTransition::Exit
            }
            _ => ScreenTransition::Stay,
        }
    }
}

/// Camp's command bar: extended keys scroll the team list rather than
/// resolving the bar (`MakeCamp`'s `special_key` arm, `ovr016.cs:1105-1109`).
fn camp_menu_bar() -> Widget {
    let mut hotbar = Hotbar::new(CAMP_MENU);
    hotbar.accept_ext = true;
    hotbar.ext_scrolls_party = true;
    Widget::Hotbar(hotbar)
}

/// `scroll_team_list` (`ovr020.cs:1349-1368`), transcribed exactly: `'O'`
/// (End / numpad 1) selects the next member, `'G'` (Home / numpad 7) the
/// previous, both wrapping; **every other extended key is a no-op** — the
/// arrow keys included, which is FD-18's confirmed behavior for every Gold Box
/// list. (`crate::screens::PartyView`/`Training` still read `'H'`/`'P'`
/// instead; that pre-M6 divergence is docketed, not widened here.)
pub(crate) fn scroll_team_list(ctx: &mut FlowCtx, code: u8) {
    let count = ctx.roster.members.len();
    if count == 0 {
        return;
    }
    let index = (ctx.state.selected_player as usize) % count;
    let next = match code {
        b'O' => (index + 1) % count,
        b'G' => (index + count - 1) % count,
        _ => return,
    };
    ctx.state.selected_player = next as u8;
}

/// Decodes `PIC{area}.DAX` block [`CAMP_PIC_BLOCK`]'s first frame
/// (`load_pic_final`, `ovr030.cs:35-149`; the `PIC`/`FINAL` XOR-delta scheme is
/// [`gbx_formats::anim::decode`]'s `xor_delta`). `None` — no campfire, the rest
/// of the layout unaffected — whenever the archive isn't there, which is every
/// synthetic-fixture engine (D10).
/// The campfire, decoded once into `cache` and blitted — shared by [`Camp`] and
/// the rest screen, because `resting` never repaints the viewport at all
/// (`ovr021.cs:533-534` clears only the bottom text region): it inherits
/// whatever `MakeCamp`'s own `LoadPic` left there, campfire included.
pub(crate) fn draw_camp_picture(ctx: &mut FlowCtx, cache: &mut Option<CampPicture>) {
    if cache.is_none() {
        *cache = decode_camp_picture(ctx);
    }
    let Some(pic) = cache else {
        return;
    };
    crate::draw::blit_image(
        ctx.fb,
        &pic.pixels,
        pic.width,
        pic.height,
        3,
        3,
        crate::draw::Clip::OVERLAY,
        None,
        None,
    );
}

fn decode_camp_picture(ctx: &FlowCtx) -> Option<CampPicture> {
    let file = format!("PIC{}.DAX", ctx.game_area());
    let bytes = ctx.data.block(&file, CAMP_PIC_BLOCK).ok()?;
    let picture = gbx_formats::anim::decode(&bytes, false, true).ok()?;
    let frame = picture.frames.into_iter().next()?;
    Some(CampPicture {
        width: frame.width_px(),
        height: frame.height as usize,
        pixels: frame.pixels,
    })
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

    /// Rebuilt carrying a leaf's refusal or result line — `magic_menu`'s own
    /// re-display loop (`ovr016.cs:605-640`).
    pub fn with_status(status: impl Into<String>) -> Self {
        MagicMenu {
            menu: menu_bar("Cast Memorize Scribe Display Rest Exit"),
            status: Some(status.into()),
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
                // Rest here is the same `rest_menu` the camp bar calls
                // (ovr016.cs:636), returning to THIS menu (:605's own loop).
                b'R' => ScreenTransition::To(crate::camp_magic::RestScreen::open(ctx, true)),
                // Exit ('E') / Escape back to camp (ovr016.cs:831's Set(0,69)).
                b'E' | 0 => ScreenTransition::To(Screen::Camp(Camp::new(ctx))),
                // ★ Memorize / Scribe / Display are real (roll-credits §8).
                b'M' => crate::camp_magic::MemorizeScreen::open(ctx),
                b'S' => crate::camp_magic::ScribeScreen::open(ctx),
                b'D' => ScreenTransition::To(crate::camp_magic::SpellEffectsScreen::open(ctx)),
                // ★ Cast is real too (roll-credits §9): `cast_spell`
                // (`ovr016.cs:158-197`) → `sub_5D2E1`'s non-combat arm.
                b'C' => crate::camp_cast::CastScreen::open(ctx),
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

    /// ★ `ovr017.loadGameMenu()` (`:929`) opened **directly** — no
    /// `Save Load Exit` chooser, because there is no chooser in the original
    /// either: `startGameMenu`'s `'L'` arm calls `loadGameMenu` and its `'S'`
    /// arm calls `SaveGame` (`ovr018.cs:223-237`), each of which IS a slot
    /// picker. The chooser is our own camp-screen convenience, and the start
    /// menu does not get it.
    pub fn new_load(ctx: &FlowCtx, return_to: ReturnTo) -> Self {
        SaveLoad {
            phase: SlPhase::Pick(SlMode::Load),
            menu: menu_bar(&load_bar_text(ctx)),
            return_to,
        }
    }

    /// `ovr017.SaveGame()` (`:1109`) opened directly — the ten-letter slot
    /// picker (`ovr017.cs:1117`), no chooser. See [`SaveLoad::new_load`].
    pub fn new_save(return_to: ReturnTo) -> Self {
        SaveLoad {
            phase: SlPhase::Pick(SlMode::Save),
            menu: menu_bar("A B C D E F G H I J"),
            return_to,
        }
    }

    fn exit(&self) -> ScreenTransition {
        match self.return_to {
            ReturnTo::World => ScreenTransition::Exit,
            // Rebuild camp fresh (its menu is stateless); a status hint would
            // need the outcome of the host's I/O, which isn't known here.
            ReturnTo::Camp => ScreenTransition::To(Screen::Camp(camp_after_saveload())),
            ReturnTo::GameOver => ScreenTransition::ToGameOver,
            // ★ The shop host owns the sub-screen (`ShopHost::Stage::Sub`) and
            // turns any exit back into `CityShop`'s own loop — `viewPlayer`
            // returns to its caller, it does not leave the shop.
            ReturnTo::Shop => ScreenTransition::Exit,
            // ★ Slice 9b: back into `startGameMenu`'s own loop.
            ReturnTo::StartMenu => ScreenTransition::ToStartMenu,
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
                // Escape / null returns to the chooser rather than leaving —
                // except on the screens that never had one: the wipe recovery
                // and (slice 9b) `startGameMenu`'s own `L`/`S` verbs, which
                // call `loadGameMenu`/`SaveGame` directly and simply `return`
                // to the menu when the player backs out (`ovr018.cs:223-237`).
                if key == 0 {
                    if matches!(self.return_to, ReturnTo::GameOver | ReturnTo::StartMenu) {
                        return self.exit();
                    }
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
            ReturnTo::GameOver => ScreenTransition::ToGameOver,
            // ★ The shop host owns the sub-screen (`ShopHost::Stage::Sub`) and
            // turns any exit back into `CityShop`'s own loop — `viewPlayer`
            // returns to its caller, it does not leave the shop.
            ReturnTo::Shop => ScreenTransition::Exit,
            // ★ Slice 9b: back into `startGameMenu`'s own loop.
            ReturnTo::StartMenu => ScreenTransition::ToStartMenu,
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
//
// ★ Roll-credits slice 9a replaced the M3 placeholder here with the real
// thing: `crate::shop_screen::ShopHost`, which `Screen::Shop` now carries.
// The placeholder's `Buy View Take Pool Share Appraise Exit` bar was
// unconditional and its Buy list was a plain stock listing; the shipped
// `CityShop` picks between two bars on `treasureOnGround` and reaches
// Sell/Id through `viewPlayer`, not from its own bar.

/// A horizontal command bar of first-letter-selectable words, extended-key
/// aware (the M2 `displayInput`/`HorizontalMenu` vocabulary).
fn menu_bar(text: &str) -> Widget {
    let mut hotbar = Hotbar::new(text);
    hotbar.accept_ext = true;
    Widget::Hotbar(hotbar)
}
