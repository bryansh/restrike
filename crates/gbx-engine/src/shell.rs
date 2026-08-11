//! The UI shell state machine (D-UI2, task deliverable 3) driven by the
//! real `EclMachine` (M2 step 4, task deliverables 1-3): `Shell`, `VmPhase`,
//! the flow plans (`BootFlow`/`LookFlow`/`StepFlow`) with chain checkpoints
//! and resume-after-chain, the persistent `chained`/`party_killed` engine
//! state, and the walk-loop's world-menu dispatch.
//!
//! **Fable review finding, addressed explicitly (binding since M2 step 3):**
//! the design doc's prose says every blocking site is a Widget parked in
//! `VmPhase::Gate` or `WorldMenu` — but the locked-door menu lives in a
//! [`StepFlow`] stage (`StepStage::DoorInteraction`), which is neither: no
//! VM vector is running during a door prompt at all. The fix applied here is
//! the doc's own suggested alternative, "Gate generalizes to flows":
//! [`VmPhase::Gate`] is not exclusive to `VectorRun`s — any flow stage may
//! park a `Widget` in it directly (`StepStage::DoorInteraction` does exactly
//! this, with no VM involvement whatsoever). There is nowhere left for a
//! blocking interaction to hide outside this one mechanism.
//!
//! Derived by reading coab for behavior (D11, never copied) — see
//! `movement.rs`'s citations for `ovr015.cs`/`ovr031.cs`, `vmhost.rs`'s for
//! the ScriptMemory/EngineServices/`load_ecl_dax` research pass; this
//! module's own citations are to `engine/ovr003.cs` `sub_29758` (the walk
//! loop, `:2230-2396`) and `sub_29677` (the chain runner, `:2180-2227`).

use crate::combat_host::{CombatHost, HostTick};
use crate::framebuffer::Framebuffer;
use crate::input::InputQueue;
use crate::movement::{
    attempt_bash, attempt_knock, attempt_pick, build_door_hotbar, move_party_forward,
    position_time_text, try_step_forward, wall_door_flags, DoorState, DoorStepFlags, Facing,
    GameClock, PartyPredicates, WorldMenuCommand,
};
use crate::rng::EngineRng;
use crate::text::{JobStatus, TextCursor, TextJob, TextPacer, NORMAL_BOTTOM};
use crate::vmhost::{describe_halt, load_ecl_block, EngineVmHost, HaltRecord, VmMemoryState};
use crate::widgets::{Delay, Hotbar, PressAnyKey, Widget, WidgetOutcome};
use gbx_formats::font::Font;
use gbx_formats::game_data::GameData;
use gbx_formats::geo::GeoBlock;
use gbx_vm::{BlockId, EclMachine, Effect, Exit, Reply, Request, VmStep, COTAB};
use std::collections::VecDeque;

/// One audio cue this tick (D-UI1's `Frame::sounds` — M8 synthesizes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SoundEvent(pub u8);

/// The Fuel watchdog (D-UI2's obligations table): a vector run steps at
/// most this many times per tick before yielding, so a `GOTO`-self script
/// can't hang the app.
const STEP_BUDGET: u32 = 10_000;

/// The dialect's vector-table indices this session's flows fire
/// (`docs/design/vm-scriptmemory.md` §1's table, 0-indexed as
/// `EclMachine::vector` takes it — confirmed by `frontends/cli/run_script.rs`'s
/// own `vector.unwrap_or(4)` default).
const VECTOR_RUN_ADDR_1: usize = 0;
const VECTOR_SEARCH_LOCATION: usize = 1;
/// `CampInterruptedAddr` — `TryEncamp` runs it when `MakeCamp` returns
/// "interrupted" (`ovr003.cs:1920`), i.e. when the rest-encounter check fired.
const VECTOR_CAMP_INTERRUPTED: usize = 3;
const VECTOR_ENTRY_POINT: usize = 4;

/// What a flow's cursor is doing right now (D-UI2, generalized — see this
/// module's top doc comment). `Pump`/`Present` only ever occur inside a
/// [`VectorRun`]; `Gate` is the shared park point for both VM-sourced and
/// purely engine-owned widgets.
/// The `Combat` arm is `combat-visualizer.md` §8.1's parking shape: a live
/// fight is an **interaction the vector is waiting on**, at the same level as
/// a parked `Widget` — not a top-level [`Shell`] variant. The `VectorRun`, and
/// the flow that owns it (Boot, Step, Look, or a chain round), stay exactly
/// where they were with their stage cursors intact, so every flow kind resumes
/// into the identical pre-fight cursor (§8.3 rule 1).
///
/// `PartialEq` is derived on the enum but a [`CombatHost`] is not comparable
/// (a whole `CombatState`), so the arm compares by discriminant only — enough
/// for the `matches!` uses in this module.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum VmPhase {
    Pump,
    Present,
    Gate(Widget),
    Combat(Box<CombatHost>),
    /// ★ **Roll-credits slice 6 (G8)**: `CMD_Combat`'s non-monster branch took
    /// the `EnterTemple` arm (`ovr003.cs:985-990`) and `ovr005.temple_shop()`
    /// is on screen. Appended last so postcard keeps every earlier variant's
    /// index and no committed `.rsav` moves.
    Temple(Box<crate::temple_screen::TempleHost>),
}

impl VmPhase {
    /// One-word state summary for frontend debug logs (`RESTRIKE_DEBUG_LOG`).
    fn probe(&self) -> String {
        match self {
            VmPhase::Pump => "pump".to_string(),
            VmPhase::Present => "present".to_string(),
            VmPhase::Gate(w) => format!(
                "gate({})",
                match w {
                    Widget::Hotbar(_) => "hotbar",
                    Widget::ListMenu(_) => "list",
                    Widget::TextEntry(_) => "text-entry",
                    Widget::PressAnyKey(_) => "press-any-key",
                    Widget::Delay(_) => "delay",
                }
            ),
            VmPhase::Combat(h) => format!("combat({:?})", h.stage()),
            VmPhase::Temple(_) => "temple".to_string(),
        }
    }
}

/// The shell's parked-`Hotbar` line — delegates to the shared
/// `sub_6C1E9` transcription (`scene::render::draw_menu_line`).
fn draw_hotbar_prompt(
    fb: &mut crate::framebuffer::Framebuffer,
    font: &gbx_formats::font::Font,
    hotbar: &Hotbar,
) {
    crate::combat::scene::render::draw_menu_line(fb, font, &hotbar.text, hotbar.selected_span());
}

/// A parked [`ListMenu`] with its own on-screen box — `sub_6C897`'s page paint
/// (`ovr027.cs:355-376`) plus `ListItemHighlighted` (`:384-398`), then
/// `sl_select_item`'s own grown prompt line (`:585-604`).
///
/// Colors are `defaultMenuColors` (`Gbl.cs:189`): foreground 10 for a normal
/// row, prompt 13 for a heading, highlight 15 as the selected row's background
/// with foreground 0. Both paint functions `Trim()`/offset by the row's leading
/// spaces, so an indented entry keeps its indent and the inverse-video block
/// covers only the text.
pub fn draw_list_menu(
    fb: &mut crate::framebuffer::Framebuffer,
    font: &gbx_formats::font::Font,
    list: &crate::widgets::ListMenu,
) {
    use crate::widgets::ListItem;
    const NORMAL: u8 = 10;
    const HEADING: u8 = 13;
    const HIGHLIGHT: u8 = 15;
    let Some(layout) = list.layout else {
        return;
    };
    // `draw8x8_clear_area(yEnd, xEnd, yStart, xStart)` (`ovr027.cs:357`).
    crate::draw::cell_rect_fill(
        fb,
        0,
        layout.start_row,
        layout.end_row,
        layout.start_col,
        layout.end_col,
    );
    let screen = list.screen_index();
    let visible = list.page_size.min(list.items.len().saturating_sub(screen));
    for offset in 0..visible {
        let i = screen + offset;
        let row = layout.start_row + offset;
        let (text, heading) = match &list.items[i] {
            ListItem::Heading(t) => (t.as_str(), true),
            ListItem::Entry(t) => (t.as_str(), false),
        };
        let indent = text.len() - text.trim_start_matches(' ').len();
        let trimmed = text.trim();
        let selected = i == list.index();
        let (bg, fg) = match (selected, heading) {
            (true, _) => (HIGHLIGHT, 0),
            (false, true) => (0, HEADING),
            (false, false) => (0, NORMAL),
        };
        crate::text::draw_string(fb, font, trimmed, row, layout.start_col + indent, bg, fg);
    }
    // The prompt line the list's own `displayInput` shows: `inputString` is
    // empty for `CMD_VertMenu` (`ovr008.cs:1218`), so this is just the
    // `" Next"`/`" Prev"` growth — highlighted by the same `sub_6C1E9` painter
    // as any other menu.
    let words = list.prompt_words();
    let span = crate::widgets::build_words(&words).first().copied();
    crate::combat::scene::render::draw_menu_line(fb, font, &words, span);
}

/// The vector/chain half of a flow's probe line.
fn run_probe(run: &Option<VectorRun>, chain: &Option<ChainRunner>) -> String {
    match (run, chain) {
        (Some(r), _) => r.phase.probe(),
        (None, Some(c)) => format!("chain:{}", c.run.phase.probe()),
        (None, None) => "idle".to_string(),
    }
}

impl Clone for VmPhase {
    fn clone(&self) -> Self {
        match self {
            VmPhase::Pump => VmPhase::Pump,
            VmPhase::Present => VmPhase::Present,
            VmPhase::Gate(w) => VmPhase::Gate(w.clone()),
            VmPhase::Combat(h) => VmPhase::Combat(h.clone()),
            VmPhase::Temple(h) => VmPhase::Temple(h.clone()),
        }
    }
}

impl PartialEq for VmPhase {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (VmPhase::Pump, VmPhase::Pump) => true,
            (VmPhase::Present, VmPhase::Present) => true,
            (VmPhase::Gate(a), VmPhase::Gate(b)) => a == b,
            (VmPhase::Combat(_), VmPhase::Combat(_)) => true,
            (VmPhase::Temple(_), VmPhase::Temple(_)) => true,
            _ => false,
        }
    }
}

/// Whatever a vector run yielded once its activation stack stops (a
/// `Request` or `Done`) — remembered across the `Present` drain so the run
/// knows what to do once the presentation queue is empty.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum PendingOutcome {
    Request(Request),
    Exit(Exit),
    /// ★ VERTICAL MENU's two-beat shape (`CMD_VertMenu`, `ovr003.cs:676-691`):
    /// `press_any_key` prints the prompt into the bottom text region FIRST, and
    /// only then does `VertMenuSelect` open its list — on the row after
    /// wherever that text stopped (`gbl.textYCol + 1`). This variant is the
    /// state between the two: the prompt is a live [`TextJob`], the request is
    /// held for the list about to open. Appended last so the `.rsav` encoding
    /// of every existing variant is untouched.
    VerticalPrompt(Request),
}

/// One buffered [`Effect`] plus the sliver of engine state its
/// *presentation* needs but which the VM has already moved past by the time
/// the effect is drained.
///
/// D-VM3 buffers effects: `step()` yields them and the VM keeps running, so
/// by the time [`VectorRun::tick_present`] draws a `PICTURE` the script has
/// typically already reset the cell that decides *how* to draw it. Real
/// CotAB content does exactly that — `SAVE <head>, 0x7EE1` / `PICTURE <body>`
/// / `SAVE 0xFF, 0x7EE1` is the shape of every picture in Tilverton's `ECL2`
/// block 1. The original has no such gap (`CMD_Picture` draws inline, reading
/// `area2_ptr.HeadBlockId` at `ovr003.cs:322`), so the faithful thing is to
/// capture the cell at *queue* time, which is execution time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QueuedEffect {
    effect: Effect,
    /// `area2_ptr.HeadBlockId` as of the step that yielded `effect`.
    head_block_id: u8,
}

/// One vector's execution against the real [`EclMachine`]: pumps steps,
/// buffers `Effect`s into an ordered presentation queue, drains that queue
/// (pacing text through [`TextJob`], gating on pagination) before any
/// `Request`'s Widget opens — the D-VM3 ordering obligation, mechanically
/// enforced by this struct's own phase order.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VectorRun {
    phase: VmPhase,
    queue: VecDeque<QueuedEffect>,
    current_job: Option<TextJob>,
    pending: Option<PendingOutcome>,
    /// Set by [`VectorRun::tick_gate`] once a parked widget resolves;
    /// consumed by the next [`VectorRun::tick_pump`] call, which then calls
    /// `machine.resume(reply, ...)` instead of `machine.step(...)` exactly
    /// once.
    pending_reply: Option<Reply>,
    /// The parked gate's animation timer (`displayInput`'s `timeStart`,
    /// `ovr027.cs:150,196` — a wait-loop stack local, so transient here too:
    /// a restored save simply restarts the current frame's dwell).
    #[serde(skip)]
    anim_wait: u32,
}

/// One tick's result from [`VectorRun::tick`].
#[derive(Debug, Clone, PartialEq)]
pub enum RunTick {
    Working,
    Done(Exit),
}

/// [`VectorRun::tick_present`]'s internal result.
enum PresentTick {
    Working,
    OpenedGate,
    Done(Exit),
}

/// Everything a flow stage needs for one tick — bundled so `Shell`/flow
/// methods don't thread a dozen parameters individually.
pub struct FlowCtx<'a> {
    pub machine: &'a mut EclMachine,
    pub vm_memory: &'a mut VmMemoryState,
    pub data: &'a GameData,
    pub input: &'a mut InputQueue,
    pub dt_ticks: u32,
    pub state: &'a mut EngineState,
    /// The resident 3D map (`gbl.geo_ptr`). **Mutable since the
    /// area-generalization slice:** `LOAD FILES` → `Load3DMap` replaces the
    /// whole block (`ovr031.cs:690-705`), which is how a party crosses from
    /// one map to another — the gap FD-19 recorded.
    pub geo: &'a mut GeoBlock,
    pub party: &'a mut dyn PartyPredicates,
    /// The real party roster (M3 step 6): the party-facing screens
    /// (character sheet, camp, training, shops) read and mutate it. Distinct
    /// from [`Self::party`], which is the M2 combat/door-predicate abstraction.
    pub roster: &'a mut crate::party::Party,
    /// The loaded rules pack (M3 step 6): derived numbers, training XP
    /// thresholds, spell slots, prices — the flavor's tables.
    pub rules: &'a gbx_rules::pack::RuleSet,
    /// Host-injected view of the save slots (M3 step 6 deliverable 3): the
    /// save/load screen renders from this, never the filesystem (D8).
    pub slots: &'a crate::saveload::SlotDirectory,
    /// Where the save/load screen deposits its chosen action for the host to
    /// fulfill after the tick (D8: the core does no file I/O itself).
    pub io_request: &'a mut Option<crate::saveload::SaveLoadRequest>,
    pub rng: &'a mut EngineRng,
    pub fb: &'a mut Framebuffer,
    pub font: &'a Font,
    pub cursor: &'a mut TextCursor,
    pub pacer: &'a mut TextPacer,
    pub sounds: &'a mut Vec<SoundEvent>,
    /// Resident 8×8 symbol sets + wallset slots (step 5, task deliverable
    /// 1): `load_walldef`'s real target, and `crate::corridor`'s texture
    /// source.
    pub symbols: &'a mut crate::symbols::SymbolSets,
    /// The three boot-loaded `SKY` blocks (moon/sun/horizon, `boot.rs`'s
    /// `BootAssets::sky`) — read-only after boot, `crate::corridor`'s
    /// backdrop source (step 5, task deliverable 2).
    pub sky: &'a [gbx_formats::image::ImageBlock; 3],
    /// Boot's 26-slot combat icon store (`BootAssets::combat_icons`) with the
    /// thirteen `COMSPR` missile/effect/focus-box icons already in it
    /// (`seg001.cs:312-317`). The combat host copies it at fight entry and
    /// fills the party/monster slots over the top, which is `BattleSetup`'s own
    /// division of labour (M6 slice 6).
    pub combat_icons: &'a crate::combat_art::CombatIcons,
    /// The resident decoded PICTURE/BIGPIC/HEAD/BODY assets
    /// (`crate::picture::PictureCache`) — the cache half of the original's
    /// `byte_1D556`/`bigpic_dax`/`headX_dax`/`bodyX_dax` globals. Derivable
    /// from [`EngineState::picture`] + `data`, so it is never serialized.
    pub pictures: &'a mut crate::picture::PictureCache,
}

impl FlowCtx<'_> {
    /// ★ The live `gbl.game_area` (roll-credits D-S1b).
    ///
    /// A **method**, not the field it used to be: the original reads
    /// `gbl.game_area` at each asset load's own call time (`load_ecl_dax`
    /// interpolates it into the file name at `ovr008.cs:148`, `Load3DMap` at
    /// `ovr031.cs:695`, and so on), which is precisely what lets a
    /// `SAVE <n>, 0x7F12` earlier in the *same* script run redirect the
    /// `NEWECL` that follows it. A snapshot taken when the tick's context was
    /// built would be stale by exactly the one instruction that matters.
    pub fn game_area(&self) -> u8 {
        self.state.game_area
    }
}

/// `VertMenuSelect`'s box for `CMD_VertMenu` (`ovr003.cs:689`): `endY = 0x16`,
/// `endX = 0x26`, `startX = 1`. Only `startY` is dynamic — `gbl.textYCol + 1`,
/// the row after wherever `press_any_key` left the prompt's own text.
const VERT_MENU_END_ROW: usize = 0x16;
const VERT_MENU_END_COL: usize = 0x26;
const VERT_MENU_START_COL: usize = 1;

/// `Request::VerticalMenu`'s widget: the entries as a [`ListMenu`] in
/// `VertMenuSelect`'s box (`ovr008.cs:1217-1226` → `sl_select_item`, which is
/// the SAME routine [`ListMenu`] already transcribes — the two are not
/// separate originals, `VertMenuSelect` is a five-argument wrapper).
///
/// Every entry is a plain `MenuItem(string)` (`Classes/MenuItem.cs:19-24` sets
/// `Heading = false`), so no row is ever a heading here.
fn vertical_menu_widget(options: &[gbx_vm::VmString], start_row: usize) -> Widget {
    let items = options
        .iter()
        .map(|s| crate::widgets::ListItem::Entry(String::from_utf8_lossy(&s.0).into_owned()))
        .collect();
    Widget::ListMenu(crate::widgets::ListMenu::boxed(
        items,
        crate::widgets::ListLayout {
            start_row: start_row.min(VERT_MENU_END_ROW),
            start_col: VERT_MENU_START_COL,
            end_row: VERT_MENU_END_ROW,
            end_col: VERT_MENU_END_COL,
        },
    ))
}

/// `Request` -> `Widget` (design doc's table, M2 slice). Engine-owned
/// interactions (world menu, door menu, pagination) never go through this —
/// only a real `VectorRun`'s `Request` does. Only the `Request` variants
/// `gbx-vm`'s interpreter can currently emit are handled
/// (`HorizontalMenu`/`VerticalMenu`/`Delay`/`Combat`) — `InputNumber`/
/// `InputString`/`SelectPlayer` await their opcodes (`0x0F`/`0x10`/`0x39`)
/// landing in the interpreter; `TextEntry` already exists and is ready
/// (step 3), this is purely a `gbx-vm` coverage gap, docketed.
/// `CMD_HorizontalMenu`'s single-option prompt rewrite (`ovr003.cs:711-721`):
/// when `string_count == 1` and the sole option is EXACTLY
/// `"PRESS BUTTON OR RETURN TO CONTINUE."` (trailing period included), the
/// original swaps in `"PRESS <ENTER>/<RETURN> TO CONTINUE"` (no trailing
/// period) *before* `buildMenuStrings` ever sees the text — the shipped ECL
/// still carries the mouse-era prompt, and the engine canonicalizes it to
/// the keyboard one at display time. A census of the shipped scripts finds
/// this firing in every one-option HORIZONTAL MENU in the game, the amnesia
/// intro's two pages included, so it is the prompt a player actually reads.
///
/// The match is byte-exact and the option count is part of the condition: a
/// multi-option menu that happens to contain the same string is untouched.
///
/// **NIT, docketed not implemented:** the single-option arm also picks the
/// colour set `(15, 15, 13)` instead of `gbl.defaultMenuColors`
/// (`ovr003.cs:713-716`). This engine's prompt painter carries no per-menu
/// colours yet; plumbing them is its own slice (see `fidelity-docket.md`).
const PRESS_BUTTON_RAW: &str = "PRESS BUTTON OR RETURN TO CONTINUE.";
const PRESS_BUTTON_CANONICAL: &str = "PRESS <ENTER>/<RETURN> TO CONTINUE";

/// The option strings a HORIZONTAL MENU actually presents — raw script text
/// with [`PRESS_BUTTON_RAW`]'s canonicalization applied. Shared by the
/// widget builder and the transcript label, because the original applies it
/// once, in `CMD_HorizontalMenu`, upstream of both the painter and anything
/// that could observe the text.
fn horizontal_menu_option_texts(options: &[gbx_vm::VmString]) -> Vec<String> {
    let mut texts: Vec<String> = options
        .iter()
        .map(|s| String::from_utf8_lossy(&s.0).into_owned())
        .collect();
    if texts.len() == 1 && texts[0] == PRESS_BUTTON_RAW {
        texts[0] = PRESS_BUTTON_CANONICAL.to_string();
    }
    texts
}

/// `Request` -> `Widget`, with the caller's persistent `gbl.menuSelectedWord`
/// (`EngineState::menu_selected_word`) seeded into whatever menu opens.
fn widget_for_request(request: &Request, menu_selected_word: usize) -> Widget {
    match request {
        Request::HorizontalMenu { options } => {
            // ★ `buildMenuStrings` (`ovr008.cs:1131-1165`): `CMD_HorizontalMenu`
            // marks each option's first character with `~`
            // (`ovr003.cs:741-745`), and this transform lowercases every
            // letter EXCEPT those marks (which it uppercases and collects as
            // the menu's hotkeys). One capital per option means
            // `BuildInputKeys`' capital-delimited grouping lands on WHOLE
            // options ("Punch barkeep" inverts as a unit, not per letter),
            // and the `sub_6C1E9` painter's `[0-9A-Z]`-in-highlight rule
            // lights exactly the hotkeys. The raw all-caps join this
            // replaced degenerated to per-letter groups — Enter resolved to
            // "P" and any stray key fell back to option 0, which at the bar
            // meant PUNCH BARKEEP: an accidental brawl (Bryan, 2026-08-08).
            //
            // (`unk_31673` includes digits, whose `+= 0x20` would garble —
            // a census of all 156 shipped HORIZONTAL MENU sites finds no
            // digit at ANY option position; letters-only here.)
            let mut text = String::new();
            let mut keys: Vec<u8> = Vec::new();
            for (i, opt) in horizontal_menu_option_texts(options).iter().enumerate() {
                if i > 0 {
                    text.push(' ');
                }
                let mut chars = opt.chars();
                if let Some(first) = chars.next() {
                    let hot = first.to_ascii_uppercase();
                    keys.push(hot as u8);
                    text.push(hot);
                    text.extend(chars.map(|c| c.to_ascii_lowercase()));
                }
            }
            let mut hotbar = Hotbar::new(text);
            // `gbl.menuSelectedWord` is global and NEVER reset between menus
            // (`ovr027.cs:142-145` only clamps it) — a looping script menu
            // re-opens with the option the player last picked still
            // highlighted. The Tilverton bar is the case that matters: press
            // 'H', drink, the menu re-opens, and Enter must order another
            // drink rather than start the brawl option 0 sits on.
            hotbar.seed_selected_word(menu_selected_word);
            hotbar.accept_ext = true;
            hotbar.ext_scrolls_party = true;
            // Only the menu's own hotkeys resolve it. `displayInput`'s
            // letter scan runs over the transformed string, where the sole
            // capitals ARE the hotkeys — any other letter never matches and
            // the loop keeps waiting (`ovr027.cs:267-292`), so `sub_317AA`'s
            // `-1` not-found arm is unreachable in practice.
            hotbar.valid_keys = Some(keys);
            Widget::Hotbar(hotbar)
        }
        // The real site is `tick_present`, which knows where the prompt's text
        // stopped; this fallback opens the list on the first row under an
        // unprinted prompt.
        Request::VerticalMenu { options, .. } => {
            vertical_menu_widget(options, NORMAL_BOTTOM.y_start + 1)
        }
        // `game_speed_var`-scaled duration is engine-owned and not on the
        // wire (host.rs's doc comment); placeholder tick count pending the
        // real value (docketed, same spirit as the "Not Here" 24-tick wait).
        Request::Delay => Widget::Delay(Delay::new(24)),
        // M2 stub (design doc's Request table): paint a stub + wait for any
        // key. The real paint is step 5's rendering scope; the flow-control
        // shape (park, resolve, resume) is what this session proves.
        Request::Combat => Widget::PressAnyKey(PressAnyKey),
        // `DisplayAndPause`: the prompt line is painted by `tick_present`
        // (which owns the framebuffer at that point); the widget is the
        // blocking `GetInputKey` that follows it.
        Request::PressAnyKey { .. } => Widget::PressAnyKey(PressAnyKey),
    }
}

/// ★ Which arm `CMD_Combat`'s non-monster branch takes (`ovr003.cs:974-992`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatBranch {
    /// `area2_ptr.EnterShop == 1` → `ovr007.CityShop()`.
    Shop,
    /// `area2_ptr.EnterTemple == 1` → `ovr005.temple_shop()`.
    Temple,
    /// Neither flag → `ovr006.AfterCombatExpAndTreasure()` — the "COMBAT with
    /// no monsters and no shop" idiom a script uses to open the treasure
    /// screen on a pile it just laid down with `TREASURE`.
    AfterCombat,
}

/// Reads the two flags and **clears the one it found**, in the original's own
/// order (Shop tested first, `ovr003.cs:978`).
fn combat_branch(ctx: &mut FlowCtx) -> CombatBranch {
    if ctx.state.enter_shop == 1 {
        ctx.state.enter_shop = 0;
        return CombatBranch::Shop;
    }
    if ctx.state.enter_temple == 1 {
        ctx.state.enter_temple = 0;
        return CombatBranch::Temple;
    }
    CombatBranch::AfterCombat
}

/// The transcript line for a non-monster `COMBAT`.
///
/// ★ **Where the shipped temples are.** The flag write is a plain
/// `SAVE 1 → 0x7EE2` (`EnterTemple`), so no opcode census could ever surface
/// one; a scan of every `ECL*.DAX` block for the address finds exactly four,
/// each of them the same three-instruction idiom:
///
/// | block | address | shape |
/// |---|---|---|
/// | `ECL1#80` | `0x8829` | `CLEARMONSTERS; SAVE 1 → 0x7EE2; COMBAT` |
/// | `ECL1#81` | `0x8677` | the same |
/// | ★ `ECL2#1` | `0x91DF` | `SAVE 0xFF → 0x7EE1` (HeadBlockId); `SAVE 1 → 0x7EE2`; `CLEARMONSTERS; COMBAT` |
/// | `ECL5#49` | `0x8E0C` | `CLEARMONSTERS; SAVE 1 → 0x7EE2; COMBAT` |
///
/// `ECL2#1` is Tilverton — **the block the playthrough starts in** (§2's "where
/// the playthrough begins"), which is why it is the slice's live acceptance
/// site. `EnterShop` (`0x7F6C`) appears in nine places across `ECL1#80/81`,
/// `ECL2#1`, `ECL4#32/37` and `ECL5#49`.
pub fn describe_combat_branch(branch: CombatBranch) -> String {
    match branch {
        CombatBranch::Shop => "combat: EnterShop → CityShop (not wired)".to_string(),
        CombatBranch::Temple => "combat: EnterTemple → temple_shop".to_string(),
        CombatBranch::AfterCombat => "combat: no monsters, no shop → AfterCombat".to_string(),
    }
}

/// Transcript-mode's (M2 step 8) request label — content, not widget shape:
/// a `HorizontalMenu`'s joined option text (the same text a player reads),
/// or a fixed descriptive label for the non-textual requests.
fn describe_request(request: &Request) -> String {
    match request {
        // The canonicalized text, not the raw script bytes: the original
        // rewrites in `CMD_HorizontalMenu` (`ovr003.cs:711-721`) and the
        // player only ever sees the result, so a transcript meant to be
        // diffed against a DOSBox side-by-side must carry it too.
        Request::HorizontalMenu { options } => {
            let text = horizontal_menu_option_texts(options).join(" ");
            format!("menu: {text}")
        }
        Request::VerticalMenu { prompt, options } => {
            let text = options
                .iter()
                .map(|s| String::from_utf8_lossy(&s.0).into_owned())
                .collect::<Vec<_>>()
                .join(" / ");
            format!(
                "vertical menu: {} [{text}]",
                String::from_utf8_lossy(&prompt.0)
            )
        }
        Request::Delay => "delay".to_string(),
        // Reached only for the DEFERRED non-combat COMBAT branch (no monsters
        // loaded → shop/temple/AfterCombat dispatch, `CMD_Combat` ovr003:974);
        // the real-combat branch resolves in `tick_present` before a widget is
        // ever built, so it never reaches here.
        Request::Combat => "combat (non-combat branch: deferred)".to_string(),
        Request::PressAnyKey { text, .. } => {
            format!("pause: {}", String::from_utf8_lossy(&text.0))
        }
    }
}

/// The party's world facing → coab's `mapDirection` (0/2/4/6 = N/E/S/W), the
/// axis `place_combatants` offsets the monster team along and `sub_304B4`
/// casts its LoS ray down.
pub(crate) fn facing_to_map_dir(facing: crate::movement::Facing) -> u8 {
    use crate::movement::Facing;
    match facing {
        Facing::North => 0,
        Facing::East => 2,
        Facing::South => 4,
        Facing::West => 6,
    }
}

// `DEFAULT_PARTY_WEAPON_DIE`, `party_combat_stats` and `run_pending_combat`
// were the shell's synchronous inline fight (M4 combat #6): a documented 1d8
// for every party member, `provisional_combat_map` for terrain, the whole
// fight run headlessly inside one `tick`, and `party_killed` set the instant
// the outcome was known.
//
// All three retired with `combat-visualizer.md` §8.4 (M6 slice 6). The
// assembly logic moved rather than died — `combat_host::CombatHost::assemble`
// is its successor, with the two placeholders replaced by
// `combat::floor`'s faithful `SetupDungeonFloor` and `combat::kits`' real
// party kits, and the sequencing obligations of §8.2 in place.

/// The inverse of [`widget_for_request`]'s `HorizontalMenu` case: maps a
/// resolved Hotbar key back to a `Reply::Selection` index. Implementation
/// note (flagged): finds the first option whose leading byte (uppercased)
/// matches the resolved key — exact for every menu this session's flows
/// construct (each option is its own hotkey-selectable word, per real
/// HORIZONTAL MENU option text), but not a byte-exact replication of
/// `sub_317AA`'s own index bookkeeping (out of scope — the original tracks
/// the option index directly rather than re-deriving it from the key).
fn resolve_horizontal_menu_reply(options: &[gbx_vm::VmString], key: u8) -> Reply {
    let upper = key.to_ascii_uppercase();
    // Over the canonicalized texts, matching `sub_317AA`'s own index scan
    // (`ovr008.cs:1196-1203`), which runs over `buildMenuStrings`' output —
    // i.e. downstream of `CMD_HorizontalMenu`'s rewrite. (Both prompt spellings
    // start with 'P', so this is exactness, not a behaviour change.)
    let idx = horizontal_menu_option_texts(options)
        .iter()
        .position(|opt| opt.bytes().next().map(|b| b.to_ascii_uppercase()) == Some(upper))
        // Not-found is `sub_317AA`'s `-1`, which `CMD_HorizontalMenu` writes
        // as the `(byte)-1 = 0xFF` sentinel (`ovr003.cs:748-750`) — ON GOTO's
        // bound check then falls through. Unreachable through the widget's
        // own `valid_keys`; kept as the faithful sentinel rather than a
        // silent option 0 (which made any stray key start the bar brawl).
        .unwrap_or(0xFF);
    Reply::Selection(idx as u8)
}

/// `machine.vector(index)` + `machine.enter(addr)`, or `None` if that vector
/// is unresolved in the resident block (an empty/malformed block — treated
/// as "nothing to run," not a panic; real CotAB data never hits this per
/// M1's census).
fn enter_vector(machine: &mut EclMachine, index: usize) -> Option<VectorRun> {
    let addr = machine.vector(index)?;
    machine.enter(addr);
    Some(VectorRun {
        phase: VmPhase::Pump,
        queue: VecDeque::new(),
        current_job: None,
        pending: None,
        pending_reply: None,
        anim_wait: 0,
    })
}

/// ★ The **vector-level drive** seam (roll-credits §5 acceptance item 2).
///
/// Enters the resident block at an arbitrary address and returns a shell
/// parked on it, so a test can exercise one script path against REAL data
/// without first satisfying every gate upstream of it. The overland exits are
/// the case that needs it: `ECL5#48`'s entry vector reaches its
/// `SAVE 1, 0x7F12` + `NEWECL 0x50` only past a `FIND ITEM` and a
/// `LOAD CHARACTER` scan, both of which belong to roll-credits slice 3 — so
/// slice 1 drives the transition itself and leaves the approach to the slice
/// that owns those opcodes.
///
/// Everything downstream is the ordinary machinery: the chain, the
/// destination block's own entry vector, its `LOAD FILES`, its menus.
#[cfg(test)]
pub(crate) fn boot_at_address(machine: &mut EclMachine, addr: u16) -> Shell {
    machine.enter(addr);
    Shell::Boot(BootFlow {
        stage: BootStage::EntryVector,
        run: Some(VectorRun {
            phase: VmPhase::Pump,
            queue: VecDeque::new(),
            current_job: None,
            pending: None,
            pending_reply: None,
            anim_wait: 0,
        }),
        chain: None,
    })
}

impl VectorRun {
    /// Advances by one tick — internally loops through phase transitions
    /// (Pump -> Present -> Gate -> Pump -> ...) making maximal progress,
    /// per D-UI1's "bounded state advance" model: a tick only *actually*
    /// pauses at a genuine wait (a parked widget needing input, or a
    /// `TextJob` that's spent this tick's character budget), never on an
    /// artificial one-phase-per-tick rule.
    ///
    /// The character-pacing budget (D-UI1's fractional accumulator) is
    /// drawn from `ctx.pacer` **exactly once** here, regardless of how many
    /// phase transitions this call makes — `tick_present` spends it on at
    /// most one `TextJob::advance` call, so a cascade of same-tick effects
    /// (e.g. an instant `PrintReturn` immediately followed by a `Print`)
    /// can never double-dip the per-tick character rate.
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> RunTick {
        let tick_ms = 1000.0 / crate::input::TICK_HZ as f64;
        let mut budget = Some(ctx.pacer.tick(tick_ms));
        loop {
            match &self.phase {
                VmPhase::Pump => {
                    if !self.tick_pump(ctx) {
                        return RunTick::Working; // exhausted the step budget without resolving
                    }
                    // pending is now set; phase became Present — keep going.
                }
                VmPhase::Present => match self.tick_present(ctx, &mut budget) {
                    PresentTick::Working => return RunTick::Working,
                    PresentTick::Done(exit) => return RunTick::Done(exit),
                    PresentTick::OpenedGate => {} // loop: let the Gate arm run this same tick
                },
                VmPhase::Combat(_) => {
                    if !self.tick_combat(ctx) {
                        return RunTick::Working; // the fight is still on screen
                    }
                    // ExitStage completed — resumed to Pump this same tick.
                }
                VmPhase::Temple(_) => {
                    if !self.tick_temple(ctx) {
                        return RunTick::Working; // the temple is still on screen
                    }
                }
                VmPhase::Gate(_) => {
                    if !self.tick_gate(ctx) {
                        return RunTick::Working; // still gated, or paginating
                    }
                    // resumed to Pump/Present — keep going this same tick.
                }
            }
        }
    }

    /// Pumps up to [`STEP_BUDGET`] steps against the real `EclMachine`.
    /// Returns `true` once a `Request` or `Done` is pending (phase advances
    /// to `Present`), `false` if the budget ran out first (the Fuel
    /// watchdog, D-UI2's obligations table). A `VmError` invokes the M2 halt
    /// policy (task deliverable 4): logged to `vm_memory.halts`, the run
    /// treated as `Done(Ended)` for flow purposes — never a hard failure.
    fn tick_pump(&mut self, ctx: &mut FlowCtx) -> bool {
        for _ in 0..STEP_BUDGET {
            // Constructed inline (not via a helper function) so the borrow
            // checker sees these as disjoint field reborrows of `*ctx` —
            // `ctx.machine` stays reachable alongside `host` only because
            // this happens within the same scope, not across a call
            // boundary (a `&mut FlowCtx`-taking helper would opaquely
            // borrow the whole struct from the caller's view).
            let result = {
                let mut host = EngineVmHost {
                    state: &mut *ctx.state,
                    vm: &mut *ctx.vm_memory,
                    geo: &mut *ctx.geo,
                    party: &mut *ctx.party,
                    roster: &mut *ctx.roster,
                    rng: &mut *ctx.rng,
                    sounds: &mut *ctx.sounds,
                    data: ctx.data,
                    symbols: &mut *ctx.symbols,
                };
                if let Some(reply) = self.pending_reply.take() {
                    ctx.machine.resume(reply, &mut host)
                } else {
                    ctx.machine.step(&mut host)
                }
            };
            match result {
                Ok(VmStep::Continue) => continue,
                // The `head_block_id` snapshot is taken here, after the step
                // that yielded the effect — see [`QueuedEffect`].
                Ok(VmStep::Effect(effect)) => self.queue.push_back(QueuedEffect {
                    effect,
                    head_block_id: ctx.state.head_block_id,
                }),
                Ok(VmStep::Request(r)) => {
                    self.pending = Some(PendingOutcome::Request(r));
                    break;
                }
                Ok(VmStep::Done(exit)) => {
                    self.pending = Some(PendingOutcome::Exit(exit));
                    break;
                }
                Err(err) => {
                    ctx.vm_memory.halts.push(describe_halt(&err));
                    self.pending = Some(PendingOutcome::Exit(Exit::Ended));
                    break;
                }
            }
        }
        if self.pending.is_some() {
            self.phase = VmPhase::Present;
            true
        } else {
            false
        }
    }

    /// `budget` is this external tick's character allowance, spent on at
    /// most one `TextJob::advance` call (see [`VectorRun::tick`]'s doc
    /// comment) — once taken, later jobs started the same tick get `0`
    /// characters and simply wait for the next external tick.
    fn tick_present(&mut self, ctx: &mut FlowCtx, budget: &mut Option<u32>) -> PresentTick {
        loop {
            if let Some(job) = &mut self.current_job {
                let this_budget = budget.take().unwrap_or(0);
                match job.advance(this_budget, ctx.fb, ctx.font, ctx.cursor) {
                    JobStatus::Continuing => return PresentTick::Working,
                    JobStatus::NeedsKey => {
                        self.phase = VmPhase::Gate(Widget::PressAnyKey(PressAnyKey));
                        return PresentTick::OpenedGate;
                    }
                    JobStatus::Done => {
                        self.current_job = None;
                        continue;
                    }
                }
            }
            if let Some(queued) = self.queue.pop_front() {
                self.present_effect(ctx, queued);
                continue;
            }

            match self
                .pending
                .take()
                .expect("Present entered with no pending outcome")
            {
                PendingOutcome::Exit(exit) => return PresentTick::Done(exit),
                // The prompt finished printing: `VertMenuSelect(0, true, false,
                // menuList, 0x16, 0x26, gbl.textYCol + 1, 1)` (`ovr003.cs:689`)
                // — the list opens on the row after the text, wherever it
                // stopped.
                PendingOutcome::VerticalPrompt(request) => {
                    let Request::VerticalMenu { options, .. } = &request else {
                        unreachable!("VerticalPrompt only ever holds a VerticalMenu")
                    };
                    let widget = vertical_menu_widget(options, ctx.cursor.row + 1);
                    self.pending = Some(PendingOutcome::Request(request));
                    self.phase = VmPhase::Gate(widget);
                    return PresentTick::OpenedGate;
                }
                PendingOutcome::Request(request) => {
                    // COMBAT (0x24) real-combat branch (`CMD_Combat` else,
                    // monsters loaded): **park** the vector on a live fight
                    // (`combat-visualizer.md` §8.1) instead of running it
                    // headlessly inside this tick. The `VmHost` borrow was
                    // released when `step()` yielded the request, so the host
                    // can own the tick loop from here (D8). The run resumes
                    // with `Reply::Combat` only once the fight's ExitStage
                    // completes — §8.2's whole point.
                    if matches!(request, Request::Combat)
                        && ctx.state.pending_combat.monsters_loaded
                    {
                        self.phase = VmPhase::Combat(Box::new(CombatHost::open(ctx)));
                        return PresentTick::OpenedGate;
                    }
                    // ★ **Roll-credits slice 6 (G8)**: `CMD_Combat`'s
                    // non-monster branch (`ovr003.cs:974-992`). Two `Area2`
                    // flags a script has just set decide which shop opens; the
                    // handler clears the flag it found, exactly here, so the
                    // NEXT flagless `COMBAT` in the same block takes the
                    // AfterCombat arm rather than re-entering.
                    if matches!(request, Request::Combat) {
                        let branch = combat_branch(ctx);
                        ctx.vm_memory
                            .transcript
                            .push(crate::vmhost::TranscriptEntry::Request(
                                describe_combat_branch(branch),
                            ));
                        if branch == CombatBranch::Temple {
                            self.phase = VmPhase::Temple(Box::new(
                                crate::temple_screen::TempleHost::open(ctx),
                            ));
                            return PresentTick::OpenedGate;
                        }
                    }
                    ctx.vm_memory
                        .transcript
                        .push(crate::vmhost::TranscriptEntry::Request(describe_request(
                            &request,
                        )));
                    // ★ VERTICAL MENU's first beat (`ovr003.cs:676-681`):
                    // `textXCol = 1; textYCol = 0x11;` then
                    // `press_any_key(delay_text, true, 10, 22, 38, 17, 1)` —
                    // which is [`TextJob`] over [`NORMAL_BOTTOM`] with
                    // `clear_first`, pagination included. The list opens on the
                    // next pass, once the text is on screen.
                    if let Request::VerticalMenu { prompt, .. } = &request {
                        ctx.cursor.col = NORMAL_BOTTOM.x_start;
                        ctx.cursor.row = NORMAL_BOTTOM.y_start;
                        let text = String::from_utf8_lossy(&prompt.0).into_owned();
                        self.current_job = Some(TextJob::new(
                            &text,
                            10,
                            NORMAL_BOTTOM,
                            true,
                            ctx.cursor,
                            ctx.fb,
                        ));
                        self.pending = Some(PendingOutcome::VerticalPrompt(request));
                        continue;
                    }
                    // `DisplayAndPause` (`seg041.cs:297-303`):
                    // `ClearPromptAreaNoUpdate`, the message on the prompt
                    // line in its own colour, then a blocking `GetInputKey`.
                    // Drawn here rather than as an `Effect` because the
                    // message and the key are one interaction.
                    if let Request::PressAnyKey { text, color } = &request {
                        crate::combat::scene::render::clear_prompt_line(ctx.fb);
                        crate::text::draw_string(
                            ctx.fb,
                            ctx.font,
                            &String::from_utf8_lossy(&text.0),
                            0x18,
                            0,
                            0,
                            *color,
                        );
                    }
                    let widget = widget_for_request(&request, ctx.state.menu_selected_word);
                    self.pending = Some(PendingOutcome::Request(request));
                    self.phase = VmPhase::Gate(widget);
                    return PresentTick::OpenedGate;
                }
            }
        }
    }

    /// `PartySummary(gbl.SelectedPlayer)` (`ovr025.cs:1430`) — the roster panel,
    /// painted from the live roster with the current selection highlighted. The
    /// same three lines `screens.rs`'s camp draw and `Engine::build` already use.
    fn draw_party_summary(ctx: &mut FlowCtx) {
        let rows: Vec<_> = ctx
            .roster
            .members
            .iter()
            .map(crate::charsheet::sheet_view)
            .collect();
        let selected =
            (!rows.is_empty()).then(|| (ctx.state.selected_player as usize) % rows.len());
        crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, selected);
    }

    /// One buffered [`Effect`] presented (D-VM3's ordered drain).
    fn present_effect(&mut self, ctx: &mut FlowCtx, queued: QueuedEffect) {
        let QueuedEffect {
            effect,
            head_block_id,
        } = queued;
        match effect {
            Effect::Print { text, clear_first } => {
                let text = String::from_utf8_lossy(&text.0).into_owned();
                ctx.vm_memory
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Print {
                        text: text.clone(),
                        clear_first,
                    });
                self.current_job = Some(TextJob::new(
                    &text,
                    10,
                    NORMAL_BOTTOM,
                    clear_first,
                    ctx.cursor,
                    ctx.fb,
                ));
            }
            Effect::PrintReturn => {
                ctx.cursor.row += 1;
                ctx.cursor.col = NORMAL_BOTTOM.x_start;
            }
            Effect::Sound(variant) => ctx.sounds.push(SoundEvent(variant)),
            // `CMD_Picture`'s two halves and the ANIMATION call
            // (`crate::picture`): these draw for real as of the
            // scene-pictures slice. M2 drained them into redraw flags
            // only — and set `can_draw_bigpic = true` on every
            // `Picture`, having read the `0xFF` clear branch's line
            // (`ovr003.cs:348`) into the non-`0xFF` branch, which
            // actually sets it *false* after a BIGPIC draw (`:332`).
            Effect::Picture(block) => crate::picture::cmd_picture(ctx, block, head_block_id),
            Effect::ClearPicture => crate::picture::cmd_clear_picture(ctx),
            Effect::AnimationFrame => crate::picture::animation_frame(ctx),
            // `0xAE11`'s guarded `RedrawView()` (`ovr003.cs:1848-1860`): the
            // gate already checked-and-cleared the dirty flags at execution
            // time (`EngineServices::redraw_view_gate`); this is the draw it
            // authorized, mid-vector — the walk-loop's own unconditional
            // recompose only ever runs at world-menu entry, which an intro
            // vector never reaches. Its `display_map_position_time()` pair
            // is covered by the engine's per-tick status line.
            Effect::RedrawView => crate::corridor::redraw_view(ctx),
            // ★ `sub_30580`'s draw half (FD-34), authorized by the
            // execution-time state pass that emitted this effect.
            Effect::EncounterVisual => crate::picture::encounter_visual(ctx),
            // `PartySummary(SelectedPlayer)` — the roster panel alone. Every
            // frame this engine composes already redraws that panel from the
            // live roster (`Engine::build`'s party summary step), so the
            // effect's job is to make sure a *frame happens* at this point in
            // the queue; the panel it asks for is repainted with it.
            Effect::PartySummary => Self::draw_party_summary(ctx),
            // ★ CLEAR BOX (`ovr003.cs:1741-1754`): `draw8x8_03` +
            // `PartySummary` + `display_map_position_time` + the picture
            // window's frame-0 redraw + the status line again. The frame
            // rebuild is `redraw_view`'s own clear→frame→viewport idiom (the
            // one `combat_host`'s Restore documents), so this reuses it rather
            // than half-painting.
            Effect::ClearBox => {
                let _ = crate::frames::draw8x8_03(ctx.fb, ctx.symbols);
                crate::corridor::redraw_view(ctx);
                Self::draw_party_summary(ctx);
            }
        }
    }

    /// ★ Ticks the parked fight (`combat-visualizer.md` §8.2). Returns `true`
    /// once ExitStage completed and the run resumed pumping — `false` while the
    /// fight is still on screen.
    ///
    /// **The deferred writes happen here, and only here.** `Shell::tick`
    /// unconditionally replaces the shell with `GameOver` at top-of-tick when
    /// `party_killed` is set, so a wipe that set the flag at outcome-known time
    /// would annihilate the fight's final beats mid-playback. Setting it at
    /// ExitStage completion means the unwind fires on the NEXT tick, after the
    /// player has seen the fight end — §8.2's MUST, and the property
    /// `a_wiped_partys_final_beats_all_play_before_the_game_over_unwind` pins.
    /// ★ Ticks the parked temple (roll-credits slice 6 / G8). `temple_shop`
    /// returns to `CMD_Combat`, which then runs the branch's shared tail
    /// (`ovr003.cs:1016-1026`: the game state goes back to the map, the search
    /// flags mask down, the encounter flags clear and `LoadPic` rebuilds the
    /// screen). Returns `true` once the visit closed and the run resumed.
    fn tick_temple(&mut self, ctx: &mut FlowCtx) -> bool {
        let VmPhase::Temple(host) = &mut self.phase else {
            unreachable!("tick_temple called outside Temple phase")
        };
        if !matches!(host.tick(ctx), crate::screens::ScreenTransition::Exit) {
            return false;
        }
        ctx.vm_memory
            .transcript
            .push(crate::vmhost::TranscriptEntry::Request(
                "temple: closed".to_string(),
            ));
        // `CMD_Combat`'s tail, shared with the fight arm.
        ctx.state.search_flags &= 1;
        ctx.state.encounter_flags = [false; 2];
        ctx.vm_memory.sprite_changed = false;
        // `LoadPic` (`ovr025.cs:1435-1441`) — the exploration screen comes back
        // whole, the same rebuild the fight's Restore stage performs.
        ctx.fb.clear(0);
        let _ = crate::frames::draw8x8_03(ctx.fb, ctx.symbols);
        crate::corridor::redraw_view(ctx);
        self.pending_reply = Some(Reply::Combat);
        self.phase = VmPhase::Pump;
        true
    }

    fn tick_combat(&mut self, ctx: &mut FlowCtx) -> bool {
        let VmPhase::Combat(host) = &mut self.phase else {
            unreachable!("tick_combat called outside Combat phase")
        };
        let HostTick::Finished {
            outcome,
            rounds,
            dropped_keys,
            verdict,
        } = host.tick(ctx)
        else {
            return false;
        };

        let label = match outcome {
            crate::combat::CombatOutcome::PartyWins => "party wins",
            crate::combat::CombatOutcome::MonstersWin => "party wiped",
            crate::combat::CombatOutcome::Stalemate => "stalemate",
        };
        let dropped = if dropped_keys > 0 {
            format!(", {dropped_keys} key(s) dropped")
        } else {
            String::new()
        };
        ctx.vm_memory
            .transcript
            .push(crate::vmhost::TranscriptEntry::Request(format!(
                "combat: {label} ({rounds} round(s){dropped})"
            )));
        // ★ Slice 6 deliverable E: the game-over trigger is
        // `CleanupPlayersStateAfterCombat`'s `gbl.party_killed`
        // (`ovr006.cs:216-226`) computed over the roster, not the fight's own
        // `CountCombatTeamMembers` verdict. A party that FLED comes out
        // `running` — alive by the original's liveness set, and a rout is not
        // a game over.
        if verdict.party_killed {
            ctx.state.party_killed = true;
            ctx.state.wipe_cause = crate::shell::WipeCause::Combat;
        }
        // ★ `CMD_Combat`'s own tail (`ovr003.cs:1024-1026`): the encounter is
        // over, so the two-flag visual state machine resets and the sprite-dirty
        // flag clears. Without this a second encounter in the same block would
        // find `encounter_flags[1]` latched from the first and never dispatch
        // its own visual — exactly what the real content does back-to-back
        // (`ECL2#2 @0x8780` and `@0x880F` are two approaches in one vector).
        ctx.state.encounter_flags = [false; 2];
        ctx.vm_memory.sprite_changed = false;
        // `Reply::Combat` with today's outcome semantics (§8.3 rule 2): a script
        // cannot tell the rendered fight from the headless one.
        self.pending_reply = Some(Reply::Combat);
        self.phase = VmPhase::Pump;
        true
    }

    /// Ticks the parked Gate widget. Returns `true` once it resumed pumping
    /// (or resumed presenting, for a nested pagination gate) — `false` while
    /// still waiting on input.
    fn tick_gate(&mut self, ctx: &mut FlowCtx) -> bool {
        let VmPhase::Gate(widget) = &mut self.phase else {
            unreachable!("tick_gate called outside Gate phase")
        };
        let outcome = widget.tick(ctx.input, ctx.dt_ticks);
        // `VertMenuSelect` returns `index` however its loop ended
        // (`ovr008.cs:1217-1226`), so the highlighted row is read here — after
        // the key, before the widget is dropped — and used for a cancel too.
        let list_index = match widget {
            Widget::ListMenu(l) => l.index(),
            _ => 0,
        };
        // `gbl.menuSelectedWord`'s write-back. The original's `','`/`'.'`/
        // letter-scan arms assign the global *inside* `displayInput`'s loop
        // (`ovr027.cs:244-292`), so it is already current whether this tick
        // cycled the highlight, resolved the menu, or neither — mirroring
        // that here, once, covers all three.
        if let Widget::Hotbar(h) = widget {
            ctx.state.menu_selected_word = h.selected_word();
        }

        // A PressAnyKey gate nested under a paginating TextJob: release the
        // job and resume presenting, rather than resuming the VM.
        if self.current_job.is_some() {
            if matches!(outcome, WidgetOutcome::Done) {
                if let Some(job) = &mut self.current_job {
                    job.release(ctx.fb);
                }
                // The original drains the whole keyboard buffer right after
                // the pagination keypress (`clear_keyboard`, seg041.cs:211;
                // design doc §1.4/D-UI3 named this the caller's obligation —
                // it was never wired in steps 3/4). Without it, keys typed
                // behind the gating keypress leak to the next widget, where
                // Enter selects the highlighted first word ("Area" in the
                // world menu). Found via the step-5 four-facings demo.
                ctx.input.clear();
                self.phase = VmPhase::Present;
                return true;
            }
            return false;
        }

        if matches!(outcome, WidgetOutcome::Pending) {
            // The `displayInput` wait-loop's animation beat (FD-33,
            // `ovr027.cs:184-198`): script menus are the loop's callers with
            // `useOverlay` in play — the world/door menus never park here.
            if matches!(widget, Widget::Hotbar(_)) {
                crate::picture::menu_wait_animation(ctx, &mut self.anim_wait);
            }
            return false;
        }
        if let WidgetOutcome::PartyScroll(code) = outcome {
            // `sub_317AA`'s special-key arm (`ovr008.cs:1181-1187`): an
            // extended key scrolls the team list and RE-PROMPTS — it never
            // resolves the menu. Before this arm existed the outcome fell
            // into the resolution match's `_ =>` fallback and replied
            // option 0, which at the bar menu was PUNCH BARKEEP — Bryan's
            // left-arrow brawl (2026-08-08).
            crate::screens::scroll_team_list(ctx, code);
            return false;
        }
        self.anim_wait = 0;

        // Any resolution: build the real Reply matching the pending
        // Request, then resume pumping. `displayInput` ends with
        // `ClearPromptArea` (`ovr027.cs:344-354`) — clear the menu line so
        // nothing lingers under whatever the script prints next.
        crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, "");
        let Some(PendingOutcome::Request(request)) = self.pending.take() else {
            unreachable!("Gate phase without a pending Request")
        };
        let reply = match (&request, outcome) {
            (Request::HorizontalMenu { options }, WidgetOutcome::Hotbar(key)) => {
                resolve_horizontal_menu_reply(options, key)
            }
            // `vm_SetMemoryValue((ushort)index, mem_loc)` (`ovr003.cs:691`)
            // takes whatever `VertMenuSelect` returned — and it returns the
            // highlighted `index` on EVERY exit, commit or ESC/`'E'` alike
            // (`ovr027.cs:653-658` never touches `index_ptr` on the cancel
            // arm). So a cancelled drink menu still orders the highlighted
            // drink; transcribed, not corrected.
            (Request::VerticalMenu { .. }, _) => Reply::Selection(list_index as u8),
            (Request::Delay, _) => Reply::Delay,
            (Request::Combat, _) => Reply::Combat,
            (Request::PressAnyKey { .. }, _) => Reply::PressAnyKey,
            // Unreachable: Hotbar yields only Hotbar(key)/PartyScroll (both
            // handled), the list arm is exhaustive above. Kept as a quiet
            // fallback, not a panic — but note option 0 is NOT "safe" at a
            // script menu (the bar's option 0 is the brawl), which is why
            // PartyScroll is consumed before this match.
            _ => Reply::Selection(0),
        };
        self.pending_reply = Some(reply);
        self.phase = VmPhase::Pump;
        true
    }
}

/// The chain runner (`sub_29677`, `ovr003.cs:2180-2227`): re-entered at
/// every `ChainCheckpoint` while `chained` stays set, running the newly
/// resident block's entry vector each round.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChainRunner {
    run: VectorRun,
}

pub enum ChainRunnerOutcome {
    ChainedAgain(BlockId),
    Finished,
}

impl ChainRunner {
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<ChainRunnerOutcome> {
        match self.run.tick(ctx) {
            RunTick::Working => None,
            RunTick::Done(Exit::Ended) => Some(ChainRunnerOutcome::Finished),
            RunTick::Done(Exit::ChainTo(id)) => Some(ChainRunnerOutcome::ChainedAgain(id)),
        }
    }
}

/// `Exit::ChainTo` bookkeeping shared by every checkpoint (§1.6): commits
/// `LastEclBlockId` (NEWECL's own old-id write, `ovr003.cs:488`), sets
/// `chained`, loads the new block via `load_ecl_dax`'s mapping
/// (`vmhost.rs`), and starts its entry vector. Returns `None` if the chain
/// already fully resolved this same tick (a load failure or an unresolved
/// entry vector — both loudly diagnosed via `vm_memory.halts`, never a
/// silent stall or a panic on bad/missing content).
fn begin_chain(ctx: &mut FlowCtx, id: BlockId) -> Option<ChainRunner> {
    ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
    ctx.state.ecl_block_id = id.0;
    ctx.state.chained = true;

    // ★ `load_ecl_dax` interpolates `gbl.game_area` at call time
    // (`ovr008.cs:148`) — a `SAVE <n>, 0x7F12` earlier in the *same* script
    // run is exactly how a NEWECL becomes a CROSS-FILE transition
    // (`ECL4#37 @0x8225`/`ECL5#48 @0x8092`, both `SAVE 1` → `NEWECL 0x50`).
    let bytes = match load_ecl_block(ctx.data, ctx.game_area(), id.0) {
        Ok(bytes) => bytes,
        Err(err) => {
            ctx.vm_memory.halts.push(HaltRecord {
                pc: 0,
                opcode: 0,
                description: format!("NEWECL to block {} failed to load: {err:?}", id.0),
            });
            ctx.state.chained = false;
            return None;
        }
    };
    *ctx.machine = EclMachine::load_block(bytes, &COTAB).unwrap_or_else(|never| match never {});
    // `CMD_NewECL` runs `vm_init_ecl` right after the load
    // (`ovr003.cs:491-492`) — the whole engine half, not just the redraw
    // flags: the fresh block starts with no portrait head armed, an armed
    // redraw gate, a zeroed rest schedule, `inDungeon` poked back to 1, and
    // (in normal play) both script scratch tables wiped. See
    // [`crate::vmhost::vm_init_ecl`] for the cell-by-cell derivation.
    crate::vmhost::vm_init_ecl(ctx.state, ctx.vm_memory);
    // `CMD_NewECL`'s own two extra clears, after the call (`ovr003.cs:496-497`)
    // — `encounter_flags[0..1]` has no engine cell yet (FD-34/G4), so there is
    // nothing to clear here beyond what `vm_init_ecl` already did.

    match enter_vector(ctx.machine, VECTOR_ENTRY_POINT) {
        Some(run) => Some(ChainRunner { run }),
        None => {
            ctx.state.chained = false;
            None
        }
    }
}

/// Runs `chain`, handling the "chained again" re-entry loop; on `Finished`,
/// commits the bookkeeping every checkpoint shares (`chained` clears,
/// `LastEclBlockId` commits, `LastSelectedPlayer` saves — §1.6) and reports
/// completion so the owning flow can resume its suspended plan. `None`
/// means still working (call again next tick); `Some(())` means resume.
fn drive_chain(chain: &mut Option<ChainRunner>, ctx: &mut FlowCtx) -> Option<()> {
    let runner = chain.as_mut()?;
    match runner.tick(ctx) {
        None => None,
        Some(ChainRunnerOutcome::ChainedAgain(id)) => {
            ctx.state.last_selected_player = ctx.state.selected_player;
            *chain = begin_chain(ctx, id);
            if chain.is_none() {
                ctx.state.chained = false;
                ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
                return Some(());
            }
            None
        }
        Some(ChainRunnerOutcome::Finished) => {
            ctx.state.chained = false;
            ctx.state.last_ecl_block_id = ctx.state.ecl_block_id;
            *chain = None;
            Some(())
        }
    }
}

/// The persistent, serializable engine state carried across ticks (D-UI2's
/// "Engine state carried" list, M2 slice — rendering caches/redraw flags
/// are step-5 scope and not modeled here).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EngineState {
    pub pos: (u8, u8),
    pub facing: Facing,
    /// bit 0 = search mode; bit 1 = transient "Look in progress" marker
    /// (`search_flags |= 2` on `'L'`, cleared by [`LookFlow`]'s restore).
    pub search_flags: u8,
    /// `block_area_view == 0`-equivalent: whether the area map is available
    /// at all in the resident block (a per-area config, M6 scope in full —
    /// defaults `true`).
    pub area_view_allowed: bool,
    /// `mapAreaDisplay`: whether the area map is currently being shown.
    pub area_map_shown: bool,
    /// The persistent `vmFlag01` equivalent (D-UI2): survives across
    /// `WorldMenu`, suppresses `LastEclBlockId` commits while set.
    pub chained: bool,
    pub party_killed: bool,
    /// Which death screen the wipe flow shows — the combat one, or the ECL
    /// DAMAGE opcode's own (`ovr003:2C43-2C86`, different words, different
    /// box, a hard 3-second beat, a different prompt colour). Set by whatever
    /// killed the party; read by [`GameOverFlow::start`].
    pub wipe_cause: WipeCause,
    pub selected_player: u8,
    pub last_selected_player: u8,
    /// ★ `gbl.player_not_found` (`byte_1EE97`, `Classes/Gbl.cs:417`): LOAD
    /// CHARACTER's miss flag. Its one reader is the Party-window cell
    /// `0x7D00` (`get_player_values`' `arg_4 == 0x100` arm,
    /// `ovr008.cs:425-441`), which returns **0** when it is set — and clears
    /// it on the way past. That read-and-clear is what makes the shipped
    /// slot-scan idiom work: `LOAD CHARACTER n` then a compare against
    /// `0x7D00` distinguishes "no such slot" (0) from "in combat" (1) and
    /// "out of combat" (0x80).
    pub player_not_found: bool,
    /// `gbl.restore_player_ptr` (`byte_1AB0A`, `Gbl.cs:290`): set by LOAD
    /// CHARACTER (`ovr003:02EF`), consumed by EXIT (`:14-18`) and PROGRAM
    /// (`:1934-1938`), which put `LastSelectedPlayer` back when it is armed —
    /// so a script that retargets the selection mid-vector does not leak that
    /// retarget into the world menu.
    pub restore_player_ptr: bool,
    /// `gbl.redrawPartySummary1`/`2` (`byte_1EE7C`/`byte_1EE7D`,
    /// `Gbl.cs:400-401`), armed by two Party-window writes of zero
    /// (`alter_character`'s `0x7C00` and `0x7D00` arms, `ovr008.cs:568-571`,
    /// `:628-631`) and cleared by `vm_init_ecl` (`:92-93`). Both armed **plus**
    /// LOAD CHARACTER's high operand bit is what turns that opcode into a
    /// remove-this-member instruction (`ovr003:0377-03C0`).
    pub redraw_party_summary: [bool; 2],
    /// `area2_ptr.party_size` (`area2.field_67C`) — the count of *real* party
    /// members. Joined NPCs are appended to the roster past it and are not
    /// counted (`load_npc` gates on `party_size <= 7` without incrementing,
    /// `ovr017.cs:880`), which is exactly the distinction combat's
    /// `nonTeamMember` split already relies on. DAMAGE rolls its victim over
    /// this, not over the roster length.
    pub party_size: u8,
    /// ★ `gbl.pooled_money` (`Classes/Gbl.cs:528`) — the treasure pool
    /// TREASURE (0x27) writes, dead monsters pay into
    /// (`calc_battle_exp`, `ovr006.cs:29`), CLEARMONSTERS clears
    /// (`ovr003.cs:767`) and the post-fight screen shares out.
    pub pooled_money: crate::money::MoneySet,
    /// `gbl.items_pointer` (`Gbl.cs:526`) — the treasure pool's item half,
    /// as opaque `0x3F`-byte `.swg`-shaped records, the same representation
    /// [`crate::party::Character::items`] uses.
    pub treasure_items: Vec<Vec<u8>>,
    /// `gbl.exp_to_add` — the per-survivor experience `calc_battle_exp`
    /// computed for the last fight, kept because `displayCombatResults` prints
    /// it after `addExp` has already spent it (`ovr006.cs:251-252`, `:795`).
    pub exp_to_add: i32,
    pub ecl_block_id: u8,
    pub last_ecl_block_id: u8,
    pub tried_to_exit_map: bool,
    /// ★ `gbl.game_area` — **real state since the area-generalization slice**
    /// (roll-credits D-RC1/D-S1a), not the `engine::GAME_AREA` constant it
    /// used to be.
    ///
    /// Every `{area}`-keyed asset family reads this at load time:
    /// `ECL{area}.DAX` (`load_ecl_dax`, `ovr008.cs:148`), `GEO{area}.DAX`
    /// (`Load3DMap`, `ovr031.cs:695`), `WALLDEF{area}`/`8X8D{area}`
    /// (`LoadWalldef`, `ovr031.cs:646`; `ovr038.cs:12`), `MON{area}CHA`
    /// (`load_mob`, `ovr017.cs:826`), `PIC{area}`/`BIGPIC{area}`/
    /// `HEAD{area}`/`BODY{area}` (`ovr030.cs:58,170,237`), `CPIC{area}`
    /// (`ovr034.cs:80`) and `ITEM{area}.DAX` (`ovr003.cs:1085`). The block-id
    /// namespace is partitioned across areas, so the same numeric block id in
    /// a different area is a different block entirely — which is exactly what
    /// makes `SAVE <n> → 0x7F12` followed by a `NEWECL` a cross-file
    /// transition.
    ///
    /// Written only through [`EngineState::set_game_area`] (the script hook)
    /// or [`EngineState::restore_game_area`]; boot/import seed it from
    /// [`crate::engine::GAME_AREA`] / the imported save's own byte.
    pub game_area: u8,
    /// `gbl.game_area_backup` (`Classes/Gbl.cs:511`) — the shadow
    /// [`EngineState::set_game_area`] pushes before overwriting the live cell
    /// (`seg042.cs:126`), and [`EngineState::restore_game_area`] pops
    /// (`seg042.cs:133`).
    pub game_area_backup: u8,
    /// `area_ptr.lastXPos`/`lastYPos` (`Classes/Area1.cs:72-75`, DataOffsets
    /// `0x1E0`/`0x1E2` → window addresses `0x4BF0`/`0x4BF1`): the walk loop's
    /// record of where the party stood when this step's per-step script
    /// finished, written immediately before `locked_door()`
    /// (`ovr003.cs:2371-2372`) and compared against the live position right
    /// after it (`:2377-2381`, the "you were moved" sound).
    ///
    /// Load-bearing content state, not bookkeeping: Tilverton's own
    /// (7,12)-North refusal script copies these two cells straight back into
    /// `mapPosX`/`mapPosY` (`ECL2#1 @0x9444`/`@0x944B`) to bounce the party
    /// off the wrong entrance — see `docs/fidelity-docket.md` FD-19.
    pub last_pos: (u8, u8),
    /// `area_ptr.can_cast_spells` (`Classes/Area1.cs:89-90`, DataOffset
    /// `0x1FF`): cleared by `vm_init_ecl` at every block entry
    /// (`ovr008.cs:113`).
    ///
    /// **Deliberately has no reader yet.** `0x1FF` is an ODD DataOffset, and
    /// the Area window's mapping is `DataOffset = (addr - 0x4B00) * 2` —
    /// always even — so no script can address this cell at all; its only
    /// consumers are engine-side spell gates (`ovr009.cs:333`,
    /// `ovr010.cs:190`, `ovr016.cs:122`) that belong to roll-credits G3/G7.
    /// Modeled here so the reset FD-37 names is real rather than skipped,
    /// exactly like the redraw flags were before their gate landed.
    pub can_cast_spells: bool,
    /// `area2_ptr.field_592`: `< 0xFF` gates `locked_door`'s whole
    /// interaction; zeroed at every world-menu entry.
    pub field_592: u8,
    pub door_flags: DoorStepFlags,
    pub clock: GameClock,
    pub reload_ecl_and_pictures: bool,
    /// `gbl.game_state`/`gbl.last_game_state` (`inDungeon`'s write hook,
    /// this session's research): only `DungeonMap`/`WildernessMap` are
    /// modeled (M2 slice — the other `game_state` values are M3+ screens).
    pub game_state: GameState,
    pub last_game_state: GameState,
    /// `area2_ptr.HeadBlockId`: `0xFF` = no specific portrait head (reset by
    /// `vm_init_ecl`). Written both by the encounter-visual path
    /// (`sub_30580`) and directly by scripts through the Area2 window cell
    /// `0x7EE1` (`vmhost.rs`'s `write_party`) — `CMD_Picture` reads it to
    /// pick between its head/body arm and its plain-picture arm
    /// (`ovr003.cs:322`).
    pub head_block_id: u8,
    /// ★ `area2_ptr.encounter_distance` (`Classes/Area2.cs:42-43`, DataOffset
    /// `0x582` → Party-window address `0x7EC1`): how many cells ahead the
    /// monster team deploys, and the approach cluster's whole subject.
    ///
    /// SETUP MONSTER computes it from `sub_304B4`'s ray and clamps it down to
    /// its own `max_distance` operand (`ovr003.cs:229-233`); APPROACH
    /// decrements it one band at a time (`:302-304`); ENCOUNTER MENU's
    /// ADVANCE arms do the same; and `CMD_Combat` clamps it once more against
    /// a *fresh* ray immediately before placement (`:997-1001`) — which is
    /// what makes a party that walked into a corridor between the approach
    /// and the fight start adjacent rather than at the old range.
    pub encounter_distance: u8,
    /// `area2_ptr.max_encounter_distance` (DataOffset `0x580` → `0x7EC0`):
    /// SETUP MONSTER / ENCOUNTER MENU operand 2, the ceiling the ray is
    /// clamped to. A `ushort` in the original, and script-writable through
    /// its own explicit `field_800_Set` case (`Classes/Area2.cs:207-209`).
    pub max_encounter_distance: u16,
    /// `gbl.sprite_block_id` (`byte_1D92B`) / `gbl.pic_block_id`
    /// (`byte_1D92C`): the `SPRIT{area}` and `PIC{area}` block ids SETUP
    /// MONSTER (`ovr003.cs:225,227`) and ENCOUNTER MENU (`:1253,1255`) stash
    /// for `sub_30580` to re-read at every later dispatch. Neither is reset by
    /// `vm_init_ecl`, so they carry across blocks exactly as the original's
    /// globals do.
    ///
    /// `pic_block_id` doubles as the **body** id when `HeadBlockId != 0xFF`
    /// (`ovr008.cs:270`), the arm essentially every shipped encounter takes.
    pub sprite_block_id: u8,
    pub pic_block_id: u8,
    /// `gbl.encounter_flags` (`byte_1EE72`, `Classes/Gbl.cs:399`) — the
    /// two-flag state machine `sub_30580` runs on: `[0]` = the `SPRIT`
    /// approach-sprite set is loaded and on screen, `[1]` = a close-up (PIC or
    /// head+body) owns the picture window. Cleared by `vm_init_ecl`
    /// (`ovr008.cs:96-97`), by `CMD_Picture`'s `0xFF` arm (`ovr003.cs:354-355`),
    /// by `CMD_NewECL` (`:496-497`) and after every fight (`:1025-1026`).
    pub encounter_flags: [bool; 2],
    /// What the viewport's picture layer shows, and the running animation's
    /// frame cursor (D-SAVE3's "active animation's frame index").
    pub picture: crate::picture::PictureLayer,
    /// The pending-combat monster roster the `LOAD MONSTER`/`CLEARMONSTERS`
    /// opcodes accumulate and `COMBAT` consumes (coab `gbl.TeamList` monster
    /// half + `monstersLoaded`/`monster_icon_id`, M4 combat #6). Transient
    /// combat-setup state — **not serialized** (`#[serde(skip)]`): a save is
    /// never taken mid-setup, so the `.rsav` golden is unaffected (the field
    /// deserializes back to [`crate::monster::PendingCombat::default`]).
    #[serde(skip)]
    pub pending_combat: crate::monster::PendingCombat,
    /// ★ `area2_ptr.EnterTemple` (`Area2.cs:65`) — set by a `SAVE 1 → 0x7EE2`
    /// two instructions before a `COMBAT`, read and cleared by `CMD_Combat`'s
    /// non-monster branch (`ovr003.cs:985-990`). Roll-credits slice 6 / G8.
    ///
    /// `#[serde(default)]`, so no `.rsav` golden moves: the flag is only ever
    /// live across the two instructions between its write and the `COMBAT`
    /// that consumes it, and no save can be taken in between.
    #[serde(default)]
    pub enter_temple: u16,
    /// `area2_ptr.EnterShop` (`Area2.cs:79`) — the same shape, one branch over
    /// (`ovr003.cs:978-982`).
    #[serde(default)]
    pub enter_shop: u16,
    /// ★ `gbl.menuSelectedWord` (`byte_1D5BE`, `Classes/Gbl.cs:375`): the
    /// highlighted word index, GLOBAL in the original and never reset —
    /// `displayInput` only clamps it on entry (`ovr027.cs:142-145`) and its
    /// `','`/`'.'`/letter arms write it (`:244-292`); a handful of callers
    /// preset it (`ovr015.cs:384` 'C' → 1, `ovr027.cs:548`/`:680`,
    /// `ovr009.cs:204`). So a looping script menu re-opens on the option the
    /// player last chose.
    ///
    /// **Not serialized, and that is the faithful choice:** `SaveGame`
    /// (`ovr017.cs:1109-1156`) writes `game_area`, `area_ptr`, `area2_ptr`,
    /// `stru_1B2CA`, `ecl_ptr`, the position bytes, the game states,
    /// `setBlocks` and the party — `byte_1D5BE` is in none of them. It is
    /// transient process state, zeroed at process init (`seg001.cs`'s
    /// `menuScreenIndex` neighbourhood, and `ovr007.cs:23`), so a
    /// `#[serde(skip)]` field that deserializes to 0 matches a freshly
    /// launched original loading the same save — and costs no
    /// `SAVE_FORMAT_VERSION` bump.
    ///
    /// Combat's menus model the same global separately
    /// (`combat_host.rs`'s `menu_selected`); unifying the two is docketed.
    #[serde(skip)]
    pub menu_selected_word: usize,
    /// ★ `gbl.rest_incounter_count` (`Classes/Gbl.cs:440`) — the camp rest
    /// loop's encounter counter. See [`crate::rest`] for the check itself and
    /// for why this is `#[serde(skip)]`: the original's own `SaveGame` does not
    /// write it either (it is a `gbl` cell, not an `Area2` one), so a reloaded
    /// game starts counting from zero exactly as a fresh one does.
    ///
    /// **Deliberately has no driver yet.** `resting`'s loop — the sole caller,
    /// `ovr021.cs:586-604` — belongs to roll-credits G3/slice 4, which owns
    /// Rest's commit, clock and healing. The cell and the arithmetic are real
    /// and tested now so that slice wires a transcription rather than
    /// discovering one; FD-44 records the seam.
    #[serde(skip)]
    pub rest_encounter: crate::rest::RestEncounterSchedule,
}

/// `gbl.game_state`'s M2 slice (`Classes/Gbl.cs`'s `GameState` enum —
/// `WildernessMap`/`DungeonMap` are the only two this session's `inDungeon`
/// write hook can produce).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum GameState {
    WildernessMap,
    DungeonMap,
}

impl EngineState {
    pub fn new() -> Self {
        EngineState {
            pos: (0, 0),
            facing: Facing::North,
            search_flags: 0,
            area_view_allowed: true,
            area_map_shown: false,
            chained: false,
            party_killed: false,
            wipe_cause: WipeCause::Combat,
            selected_player: 0,
            last_selected_player: 0,
            player_not_found: false,
            restore_player_ptr: false,
            redraw_party_summary: [false; 2],
            party_size: 0,
            pooled_money: crate::money::MoneySet::default(),
            treasure_items: Vec::new(),
            exp_to_add: 0,
            ecl_block_id: 1,
            last_ecl_block_id: 0,
            tried_to_exit_map: false,
            // The boot/import default (`crate::engine::GAME_AREA`); a real
            // boot or import overwrites it immediately, and from then on only
            // `SAVE <n> -> 0x7F12` moves it.
            game_area: crate::engine::GAME_AREA,
            game_area_backup: crate::engine::GAME_AREA,
            last_pos: (0, 0),
            can_cast_spells: false,
            field_592: 0,
            door_flags: DoorStepFlags::all_true(),
            clock: GameClock::default(),
            reload_ecl_and_pictures: false,
            game_state: GameState::DungeonMap,
            last_game_state: GameState::DungeonMap,
            head_block_id: 0xFF,
            encounter_distance: 0,
            max_encounter_distance: 0,
            sprite_block_id: 0,
            pic_block_id: 0,
            encounter_flags: [false; 2],
            picture: crate::picture::PictureLayer::default(),
            pending_combat: crate::monster::PendingCombat::default(),
            enter_temple: 0,
            enter_shop: 0,
            menu_selected_word: 0,
            rest_encounter: crate::rest::RestEncounterSchedule::default(),
        }
    }

    pub(crate) fn search_mode(&self) -> bool {
        self.search_flags & 1 != 0
    }

    /// ★ `seg042.set_game_area` (`seg042.cs:124-128`) — the whole of the
    /// original's body, in order: **backup ← live, then live ← value**.
    ///
    /// The one script route in is `SAVE <n>, 0x7F12`
    /// (`vm_SetMemoryValue` → `alter_character`'s `switch_var == 0x312` arm,
    /// `ovr008.cs:654-657`). Both shipped uses (`ECL4#37 @0x8225`,
    /// `ECL5#48 @0x8092`) are `SAVE 1` immediately followed by `NEWECL 0x50`,
    /// which is what makes the *next* `load_ecl_dax` read `ECL1.DAX` rather
    /// than the file the running block came from.
    pub fn set_game_area(&mut self, area: u8) {
        self.game_area_backup = self.game_area;
        self.game_area = area;
    }

    /// `seg042.restore_game_area` (`seg042.cs:131-134`): backup → live.
    ///
    /// **Correction to the slice-1 door, which expected no reached caller:**
    /// the original has exactly one, and it *is* reached — `LoadPlayerCombatIcon`
    /// brackets its work with `set_game_area(1)` … `restore_game_area()`
    /// (`ovr017.cs:88,120`), and `loadSaveGame` calls it for every non-NPC
    /// party member (`ovr017.cs:1058`), as does combat setup throughout
    /// `ovr018`. The bracket is nonetheless *vestigial for asset selection*:
    /// everything it wraps takes `chead_cbody_comspr_icon`'s `CHEAD`/`CBODY`
    /// branch (`ovr034.cs:57-66`), which never appends `gbl.game_area` to the
    /// file name — only the `else` branch (`CPIC`, `:80`) does. Our own party
    /// icon loader (`combat_art::load_party_icon`) takes no area argument at
    /// all, so there is nothing here for a bracket to protect; the method
    /// exists because the cell pair it pops is real, and a caller lands with
    /// whatever first needs one.
    pub fn restore_game_area(&mut self) {
        self.game_area = self.game_area_backup;
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}

// --- BootFlow (block-entry preamble, `sub_29758`'s header, `:2230-2313`) ---

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum BootStage {
    EntryVector,
    PostChainResume,
    Done,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BootFlow {
    stage: BootStage,
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
}

impl BootFlow {
    pub fn start(
        machine: &mut EclMachine,
        state: &mut EngineState,
        vm: &mut VmMemoryState,
    ) -> Self {
        state.last_selected_player = state.selected_player; // `:2232`
                                                            // The block-entry preamble calls `vm_init_ecl` unconditionally before
                                                            // running the entry vector (`ovr003.cs:2278`). Its `HeadBlockId = 0xFF`
                                                            // (`ovr008.cs:109`) is load-bearing for an imported original save: the
                                                            // save's own Area2 bytes carry a stale head id (the bundled GOG save
                                                            // says `0x00`), and without the reset the intro's first `PICTURE 0x0a`
                                                            // takes the head/body arm (HEAD2 block 0's face + a BODY2 block that
                                                            // does not exist) instead of the plain-PIC sword arm — Bryan's
                                                            // 2026-08-08 playtest find. The rest of the reset is
                                                            // [`crate::vmhost::vm_init_ecl`]'s table.
        crate::vmhost::vm_init_ecl(state, vm);
        let run = enter_vector(machine, VECTOR_ENTRY_POINT);
        BootFlow {
            stage: BootStage::EntryVector,
            run,
            chain: None,
        }
    }

    /// `Some(())` once the flow is done (caller transitions to `WorldMenu`).
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            self.stage = BootStage::PostChainResume;
        }

        match self.stage {
            BootStage::EntryVector => {
                let Some(run) = self.run.as_mut() else {
                    // The entry vector was unresolved at construction — an
                    // immediate no-op, matching an empty block.
                    self.stage = BootStage::PostChainResume;
                    return None;
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        ctx.state.last_ecl_block_id = ctx.state.ecl_block_id; // `:2292-2294`
                        self.run = None;
                        self.stage = BootStage::PostChainResume;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = BootStage::PostChainResume,
                        }
                        None
                    }
                }
            }
            // The `LoadPic`/`RedrawView`-equivalent + `reload_ecl_and_pictures`
            // clear (`:2298-2313`) — rendering itself is step-5 scope; this
            // session's observable contract is the flag clear happening
            // strictly after any chain resolves (the resume-after-chain
            // shape D-UI2 calls mandatory).
            BootStage::PostChainResume => {
                ctx.state.reload_ecl_and_pictures = false;
                self.stage = BootStage::Done;
                Some(())
            }
            BootStage::Done => Some(()),
        }
    }
}

// --- LookFlow (the 'L' sub-loop's Look branch, `ovr003.cs`'s
// `search_flags>1` while-loop body) ---

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum LookStage {
    RunVector2,
    RestoreSearchFlags,
    Done,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LookFlow {
    stage: LookStage,
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
    search_flags_backup: u8,
}

impl LookFlow {
    /// Caller (the world-menu dispatch) has already set `search_flags |= 2`
    /// and advanced the clock (`'L'` handler, §1.6) before calling this.
    pub fn start(machine: &mut EclMachine, state: &mut EngineState) -> Self {
        let backup = state.search_flags & 1;
        state.search_flags = 1;
        let run = enter_vector(machine, VECTOR_SEARCH_LOCATION);
        LookFlow {
            stage: LookStage::RunVector2,
            run,
            chain: None,
            search_flags_backup: backup,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            self.stage = LookStage::RestoreSearchFlags;
        }

        match self.stage {
            LookStage::RunVector2 => {
                let Some(run) = self.run.as_mut() else {
                    self.stage = LookStage::RestoreSearchFlags;
                    return None;
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.stage = LookStage::RestoreSearchFlags;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = LookStage::RestoreSearchFlags,
                        }
                        None
                    }
                }
            }
            LookStage::RestoreSearchFlags => {
                // Bit 1 ("Look pending") never survives a Look, regardless
                // of what vector 2 did — confirmed by this session's
                // research (only bit 0 is ever backed up/restored).
                ctx.state.search_flags = self.search_flags_backup;
                self.stage = LookStage::Done;
                Some(())
            }
            LookStage::Done => Some(()),
        }
    }
}

// --- CampInterruptFlow (★ FD-44: `TryEncamp`'s interrupted arm) ---

/// ★ The camp ambush (`TryEncamp`, `ovr003.cs:1920`).
///
/// `MakeCamp` returns `actionInterrupted` when `resting`'s rest-encounter check
/// fires (`ovr021.cs:594-602`); `TryEncamp` answers it by running the resident
/// ECL block's header **vector 3**, `CampInterruptedAddr` — the block's own
/// camp-ambush script, which is ordinary script content from there on (its
/// `COMBAT`, its text, its `NEWECL` if it has one).
///
/// The interruption is deliberately **not** a flag the script polls: nothing
/// writes a "you were ambushed" cell. The engine simply runs the vector.
/// A block whose vector 3 is unresolved runs nothing and the party walks on,
/// which is the same tolerance every other vector site here has.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CampInterruptFlow {
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
}

impl CampInterruptFlow {
    pub fn start(machine: &mut EclMachine) -> Self {
        CampInterruptFlow {
            run: enter_vector(machine, VECTOR_CAMP_INTERRUPTED),
            chain: None,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            return Some(());
        }
        let Some(run) = self.run.as_mut() else {
            return Some(());
        };
        match run.tick(ctx) {
            RunTick::Working => None,
            RunTick::Done(Exit::Ended) => {
                self.run = None;
                Some(())
            }
            RunTick::Done(Exit::ChainTo(id)) => {
                self.run = None;
                match begin_chain(ctx, id) {
                    Some(runner) => {
                        self.chain = Some(runner);
                        None
                    }
                    None => Some(()),
                }
            }
        }
    }
}

// --- StepFlow (the forward/step sequence, `sub_29758`'s tail) ---

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum StepStage {
    RunVector1,
    DoorInteraction,
    RunVector2,
    Done,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StepFlow {
    stage: StepStage,
    run: Option<VectorRun>,
    chain: Option<ChainRunner>,
    /// The Fable-review fix in concrete form: a Widget parked directly by
    /// this flow stage, no VM involved. `None` once resolved or if the
    /// door menu never opened (no options / solid / already open).
    door_widget: Option<Widget>,
}

/// ★ `area_ptr.lastXPos`/`lastYPos = mapPosX`/`mapPosY` (`ovr003.cs:2371-2372`)
/// — written at exactly one point in the walk loop: after the per-step script
/// (`vm_run_addr_1`) has run, before `locked_door()`.
///
/// It reads like bookkeeping for the sound check three lines later, and that
/// is one of its two jobs. The other is content: a script can copy these cells
/// back into `mapPosX`/`mapPosY` to put the party where it was, which is how
/// Tilverton refuses the (7,12)-North entrance (`ECL2#1 @0x9444`/`@0x944B`).
/// With nothing maintaining them, that refusal read two zeroes and teleported
/// the party to (0,0) — the symptom FD-19 recorded and blamed on the map swap.
fn record_last_pos(ctx: &mut FlowCtx) {
    ctx.state.last_pos = ctx.state.pos;
}

impl StepFlow {
    pub fn start(machine: &mut EclMachine, _state: &mut EngineState) -> Self {
        let run = enter_vector(machine, VECTOR_RUN_ADDR_1);
        StepFlow {
            stage: StepStage::RunVector1,
            run,
            chain: None,
            door_widget: None,
        }
    }

    /// Whether the Bash/Pick/Knock/Exit menu is currently parked — a
    /// test/demo introspection seam (the Fable-review fix is otherwise
    /// invisible from outside `shell.rs`: no VM run is active while this is
    /// `true`).
    pub fn door_widget_is_some(&self) -> bool {
        self.door_widget.is_some()
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if self.chain.is_some() {
            drive_chain(&mut self.chain, ctx)?;
            // The chained-mid-flow rule: the pending step (door interaction)
            // is abandoned entirely (§1.6) — resume goes straight past it.
            self.stage = StepStage::Done;
            return Some(());
        }

        match self.stage {
            StepStage::RunVector1 => {
                let Some(run) = self.run.as_mut() else {
                    record_last_pos(ctx);
                    self.stage = StepStage::DoorInteraction;
                    return None;
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        record_last_pos(ctx);
                        self.stage = StepStage::DoorInteraction;
                        None
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = StepStage::Done,
                        }
                        None
                    }
                }
            }
            StepStage::DoorInteraction => self.tick_door_interaction(ctx),
            StepStage::RunVector2 => {
                let Some(run) = self.run.as_mut() else {
                    self.stage = StepStage::Done;
                    return Some(());
                };
                match run.tick(ctx) {
                    RunTick::Working => None,
                    RunTick::Done(Exit::Ended) => {
                        self.run = None;
                        self.stage = StepStage::Done;
                        Some(())
                    }
                    RunTick::Done(Exit::ChainTo(id)) => {
                        self.run = None;
                        match begin_chain(ctx, id) {
                            Some(runner) => self.chain = Some(runner),
                            None => self.stage = StepStage::Done,
                        }
                        None
                    }
                }
            }
            StepStage::Done => Some(()),
        }
    }

    /// `locked_door` (`ovr015.cs:468-593`) — no VM run of any kind; a
    /// direct Widget park (the Fable review fix, see module doc comment).
    fn tick_door_interaction(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if let Some(widget) = &mut self.door_widget {
            match widget.tick(ctx.input, ctx.dt_ticks) {
                WidgetOutcome::Pending => return None,
                WidgetOutcome::Hotbar(key) => {
                    let state_flag = wall_door_flags(
                        ctx.geo
                            .square(ctx.state.pos.0 as usize, ctx.state.pos.1 as usize),
                        ctx.state.facing,
                    );
                    let door_state = DoorState::from_flag(state_flag);
                    let moved = match key.to_ascii_uppercase() {
                        b'B' => {
                            attempt_bash(door_state, ctx.party, &mut ctx.state.door_flags, ctx.rng)
                        }
                        b'P' => {
                            attempt_pick(door_state, ctx.party, &mut ctx.state.door_flags, ctx.rng)
                        }
                        b'K' => attempt_knock(ctx.party),
                        _ => false, // Exit (or anything else): no effect
                    };
                    if moved {
                        let facing = ctx.state.facing;
                        let search = ctx.state.search_mode();
                        move_party_forward(
                            &mut ctx.state.pos,
                            facing,
                            search,
                            &mut ctx.state.door_flags,
                            &mut ctx.state.clock,
                        );
                    }
                    self.door_widget = None;
                    // `ClearPromptArea` on resolution (`ovr027.cs:344-354`).
                    crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, "");
                }
                _ => {
                    self.door_widget = None; // any other widget outcome: treat as exit
                    crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, "");
                }
            }
        } else if ctx.state.field_592 < 0xFF {
            let square = ctx
                .geo
                .square(ctx.state.pos.0 as usize, ctx.state.pos.1 as usize);
            let flag = wall_door_flags(square, ctx.state.facing);
            match DoorState::from_flag(flag) {
                DoorState::Open => {
                    let facing = ctx.state.facing;
                    let search = ctx.state.search_mode();
                    move_party_forward(
                        &mut ctx.state.pos,
                        facing,
                        search,
                        &mut ctx.state.door_flags,
                        &mut ctx.state.clock,
                    )
                }
                DoorState::Solid => {}
                DoorState::Locked | DoorState::Unpickable => {
                    if let Some(hotbar) = build_door_hotbar(&ctx.state.door_flags, ctx.party) {
                        self.door_widget = Some(Widget::Hotbar(hotbar));
                        return None;
                    }
                    // No option available at all: silent no-op (research
                    // finding — the original shows no menu here either).
                }
            }
        } else {
            ctx.state.field_592 = 0;
        }

        // `:2377-2381`, the "you were moved" sound — compared against the same
        // two cells `record_last_pos` just wrote, which is what makes a script
        // that teleports the party (or bounces it back off a refused entrance)
        // announce itself.
        if ctx.state.pos != ctx.state.last_pos {
            ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
        }
        self.run = enter_vector(ctx.machine, VECTOR_SEARCH_LOCATION);
        self.stage = StepStage::RunVector2;
        None
    }
}

// --- The total party kill (roll-credits slice 0, G0) ---

/// `AfterCombatExpAndTreasure`'s wipe branch (`ovr006.cs:801-809`), verbatim.
/// 36 columns of box (`xStart = 2 .. xEnd = 0x25`) wrap it onto two lines.
/// Which of the original's two party-wipe screens the [`GameOverFlow`] shows.
/// They are genuinely different presentations, not one screen with two
/// strings, so the choice is engine state rather than a caller argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WipeCause {
    /// `AfterCombatExpAndTreasure`'s `else` arm (`ovr006.cs:803-808`).
    Combat,
    /// The ECL `DAMAGE` opcode's own arm (`ovr003:2C43-2C86`), whose
    /// party-killed scan runs inside the opcode.
    EclDamage,
}

const WIPE_TEXT: &str = "The monsters rejoice for the party has been destroyed";

/// `press_any_key(..., yEnd = 0x16, xEnd = 0x25, yStart = 5, xStart = 2)`
/// (`ovr006.cs:807`) — the box the death message prints into.
const WIPE_TEXT_REGION: crate::text::TextRegion = crate::text::TextRegion {
    y_start: 5,
    y_end: 0x16,
    x_start: 2,
    x_end: 0x25,
};

/// `DisplayAndPause("Press any key to continue", 13)` (`ovr006.cs:808`) —
/// `ClearPromptAreaNoUpdate`, then this at row 0x18 col 0 in colour 13, then
/// a blocking `GetInputKey`.
const WIPE_PROMPT: &str = "Press any key to continue";
const WIPE_PROMPT_COLOR: u8 = 13;

/// ★ The ECL DAMAGE variant (`ovr003:2C4A-2C86`), transcribed from the binary
/// rather than from coab's paraphrase. Four differences from the combat
/// screen, all of them visible: the words, the box (`press_any_key(text,
/// clear, 10, yEnd 0x16, xEnd 0x26, yStart 1, xStart 1)` — a *wider* box
/// starting one row higher), the cursor it is placed from (`textXCol = 2`,
/// `textYCol = 2` at `:2C4F-2C54`, against the combat screen's `2, 6`), and a
/// hard `DELAY(3000)` (`:2C82`) before anything else may happen.
const DAMAGE_WIPE_TEXT: &str = "The entire party is killed!";
const DAMAGE_WIPE_TEXT_REGION: crate::text::TextRegion = crate::text::TextRegion {
    y_start: 1,
    y_end: 0x16,
    x_start: 1,
    x_end: 0x26,
};
/// `SysDelay(3000)` at 60 Hz (`crate::input::TICK_HZ`).
const DAMAGE_WIPE_DELAY_TICKS: u32 = 3 * crate::input::TICK_HZ;
/// The opcode's own trailing prompt is `DisplayAndPause("press
/// &lt;enter&gt;/&lt;return&gt; to continue", 15)` (`:2C98-2CAD`) — colour 15, and a
/// different sentence from the combat screen's colour-13 one.
const DAMAGE_WIPE_PROMPT: &str = "press <enter>/<return> to continue";
const DAMAGE_WIPE_PROMPT_COLOR: u8 = 15;

/// ★ **The party wipe** (`ovr006.cs:801-809` for the screen; `seg001.cs:133-153`
/// for what happens next).
///
/// The original's death path is not a special state at all — it is three
/// nested loops unwinding on one global. `CleanupPlayersStateAfterCombat`
/// (`ovr006.cs:169-231`) assumes the party dead and lets any living, non-NPC
/// member of `CombatTeam.Ours` disprove it; on a real wipe
/// `AfterCombatExpAndTreasure` takes its `else` branch (`ovr006.cs:784`) —
/// **no experience, no treasure, no combat-results screen** — draws the outer
/// frame, prints the message and waits for a key. Then `party_killed` fails
/// `RunEclVm`'s loop condition (`ovr003.cs:2154-2155`), fails the exploration
/// loop's `while` (`ovr003.cs:2392`), and `sub_29758` returns into
/// `seg001.Main`'s `while (true)`, which calls `InitAgain()` — clearing
/// `TeamList` outright (`seg001.cs:364`) — and re-enters `startGameMenu`. With
/// a null party that menu offers exactly four things (`ovr018.cs:103-114`):
/// Create, Add, **Load Saved Game**, Exit; `BEGIN Adventuring` and `Save` are
/// disabled. `loadGameMenu` is reached only if the player presses `L`
/// (`ovr018.cs:223-228`).
///
/// **Our mapping.** We have no character creation and no start menu (M7+
/// territory), so the branch of `startGameMenu` that actually matters here —
/// the one and only way back into the game with a dead party — is the load
/// list, and that is what we open: the save/load screen, in the same
/// host-injected slot directory the camp Save screen uses. `ReturnTo::GameOver`
/// keeps the loop honest in the original's own terms: declining the load puts
/// the player back on the death screen, never into the world with a dead party
/// (a state the original cannot express, since `InitAgain` already threw the
/// party away). A load replaces the whole engine, which is where the recovery
/// completes — the host's job, exactly as in `frontends/desktop`.
///
/// Deliberately NOT transcribed: `InitAgain`'s own resets. Every one of them
/// (`TeamList.Clear()`, `SelectedPlayer = null`, the position/area defaults) is
/// preparation for a menu we do not have, and clearing our roster before the
/// load would only make a failed load unrecoverable. `field_58E = 0x80`
/// (`ovr006.cs:803`) is the ECL-readable "party was destroyed" cell — no
/// shipped script reads it on this path, and Area2's cell modelling is slice
/// 1's business.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GameOverFlow {
    /// The message being printed (`press_any_key`'s own paced, wrapping,
    /// paginating job). `None` once it has finished.
    job: Option<TextJob>,
    /// `DisplayAndPause`'s blocking `GetInputKey`, armed once the message is
    /// fully on screen.
    gate: Option<Widget>,
    /// Which screen this is — the two differ in words, box, cursor, pacing and
    /// prompt colour (see [`WipeCause`]).
    #[serde(default = "wipe_cause_combat")]
    cause: WipeCause,
    /// The ECL variant's `SysDelay(3000)` (`ovr003:2C82`), counted down before
    /// the prompt is even drawn. Zero for the combat screen, which has none.
    #[serde(default)]
    delay_ticks: u32,
}

fn wipe_cause_combat() -> WipeCause {
    WipeCause::Combat
}

impl GameOverFlow {
    /// Paints the death screen and starts the message printing — the combat
    /// arm (`ovr006.cs:802-807`) or the ECL DAMAGE arm (`ovr003:2C4A-2C86`),
    /// whichever [`EngineState::wipe_cause`] says killed the party.
    fn start(ctx: &mut FlowCtx) -> Self {
        let cause = ctx.state.wipe_cause;
        // A fixture engine with no symbol set 4 gets no border, exactly as
        // the screens' own `draw_frame_outer` calls already tolerate.
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        // `gbl.textXCol/textYCol` (`ovr006.cs:805-806` / `ovr003:2C4F-2C54`) —
        // then `press_any_key`'s `clear_first` resets the cursor to the box
        // origin anyway (`seg041.cs:151-158`), which is what actually places
        // the text.
        let (text, color, region, delay_ticks) = match cause {
            WipeCause::Combat => {
                ctx.cursor.col = 2;
                ctx.cursor.row = 6;
                (WIPE_TEXT, 10, WIPE_TEXT_REGION, 0)
            }
            WipeCause::EclDamage => {
                ctx.cursor.col = 2;
                ctx.cursor.row = 2;
                (
                    DAMAGE_WIPE_TEXT,
                    10,
                    DAMAGE_WIPE_TEXT_REGION,
                    DAMAGE_WIPE_DELAY_TICKS,
                )
            }
        };
        let job = TextJob::new(text, color, region, true, ctx.cursor, ctx.fb);
        GameOverFlow {
            job: Some(job),
            gate: None,
            cause,
            delay_ticks,
        }
    }

    /// The state a death screen is in once its message has finished printing
    /// and only the keypress is outstanding — constructible without a
    /// `FlowCtx` (which [`GameOverFlow::start`] needs in order to *draw*), for
    /// fixtures and serde round-trips.
    pub fn awaiting_key() -> Self {
        GameOverFlow {
            job: None,
            gate: Some(Widget::PressAnyKey(PressAnyKey)),
            cause: WipeCause::Combat,
            delay_ticks: 0,
        }
    }

    /// `Some(())` once the player has acknowledged the death screen and the
    /// caller should open the recovery (load) screen.
    fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if let Some(job) = &mut self.job {
            let tick_ms = 1000.0 / crate::input::TICK_HZ as f64;
            let budget = ctx.pacer.tick(tick_ms);
            match job.advance(budget, ctx.fb, ctx.font, ctx.cursor) {
                JobStatus::Continuing => return None,
                // The message is two lines in an eighteen-row box, so this is
                // unreachable in practice — handled rather than assumed.
                JobStatus::NeedsKey => {
                    if matches!(
                        Widget::PressAnyKey(PressAnyKey).tick(ctx.input, ctx.dt_ticks),
                        WidgetOutcome::Done
                    ) {
                        job.release(ctx.fb);
                        ctx.input.clear();
                    }
                    return None;
                }
                JobStatus::Done => {
                    self.job = None;
                }
            }
        }
        // ★ The ECL variant's `SysDelay(3000)` (`ovr003:2C82`) sits BETWEEN
        // the message and the prompt: the original really does hold the
        // "entire party is killed!" screen for three seconds with nothing
        // else on it before it will take a key.
        if self.delay_ticks > 0 {
            self.delay_ticks = self.delay_ticks.saturating_sub(ctx.dt_ticks);
            ctx.input.clear(); // the delay is not interruptible
            return None;
        }
        if self.gate.is_none() {
            // `DisplayAndPause` (`seg041.cs:297-303`).
            let (prompt, color) = match self.cause {
                WipeCause::Combat => (WIPE_PROMPT, WIPE_PROMPT_COLOR),
                WipeCause::EclDamage => (DAMAGE_WIPE_PROMPT, DAMAGE_WIPE_PROMPT_COLOR),
            };
            crate::combat::scene::render::clear_prompt_line(ctx.fb);
            crate::text::draw_string(ctx.fb, ctx.font, prompt, 0x18, 0, 0, color);
            self.gate = Some(Widget::PressAnyKey(PressAnyKey));
        }
        let gate = self.gate.as_mut()?;
        match gate.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Done => {
                ctx.input.clear(); // `clear_keyboard` after the acknowledgement
                Some(())
            }
            _ => None,
        }
    }
}

/// The UI shell (D-UI2): `Boot`/`WorldMenu`/`Step`/`GameOver`, plus `Look`
/// as its own explicit variant (the `'L'` sub-loop, distinct from
/// `WorldMenu` and `Step` — both this session's required resume-after-chain
/// sites live here and in [`BootFlow`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Shell {
    Boot(BootFlow),
    WorldMenu {
        menu: Widget,
    },
    Look(LookFlow),
    Step(StepFlow),
    GameOver(GameOverFlow),
    /// The M3 step-6 party-facing menu screens (character sheet, camp,
    /// save/load, training, shops) — additive, no VM vector runs here; each
    /// is a parked-widget screen (`crate::screens`).
    Screen(crate::screens::Screen),
    /// ★ The camp ambush: camp ended interrupted, so the resident block's
    /// vector 3 runs (roll-credits §8, FD-44). Appended last so postcard keeps
    /// every existing variant's encoding.
    CampInterrupt(CampInterruptFlow),
}

impl Shell {
    /// The Widget currently holding the keyboard, wherever it is parked — the
    /// same walk [`Shell::gate_open`] does, plus `WorldMenu`'s own menu and
    /// the door menu. `None` while a fight owns the screen (combat draws its
    /// own prompts).
    fn parked_widget(&self) -> Option<&Widget> {
        fn from_phase(phase: Option<&VmPhase>) -> Option<&Widget> {
            match phase {
                Some(VmPhase::Gate(w)) => Some(w),
                _ => None,
            }
        }
        fn from_run<'a>(
            run: &'a Option<VectorRun>,
            chain: &'a Option<ChainRunner>,
        ) -> Option<&'a Widget> {
            from_phase(run.as_ref().map(|r| &r.phase))
                .or_else(|| from_phase(chain.as_ref().map(|c| &c.run.phase)))
        }
        match self {
            Shell::Boot(f) => from_run(&f.run, &f.chain),
            Shell::WorldMenu { menu } => Some(menu),
            Shell::Look(f) => from_run(&f.run, &f.chain),
            Shell::Step(f) => f
                .door_widget
                .as_ref()
                .or_else(|| from_run(&f.run, &f.chain)),
            // The death screen's own gate draws nothing on the prompt row
            // (`DisplayAndPause` already put its line there), so it is not a
            // `parked_widget` for drawing purposes — see [`GameOverFlow`].
            Shell::GameOver(_) | Shell::Screen(_) => None,
            Shell::CampInterrupt(f) => from_run(&f.run, &f.chain),
        }
    }

    /// [`Shell::parked_widget`] as a test seam (which widget currently holds
    /// the keyboard, and in what state).
    #[cfg(test)]
    pub(crate) fn parked_widget_for_tests(&self) -> Option<&Widget> {
        self.parked_widget()
    }

    /// Draws the parked widget's prompt-row line — called every tick after the
    /// flows have run, skipped while a fight owns the screen. Idempotent (the
    /// row is cleared and redrawn), and the resolution paths call
    /// `ClearPromptArea`'s analogue so nothing lingers.
    ///
    /// A `Hotbar` draws with `display_highlighed_text`'s own three-way
    /// coloring (`sub_6C1E9`, `ovr027.cs:89-120`): the SELECTED word in
    /// inverse video (black on the highlight color), every hotkey-capable
    /// `[0-9A-Z]` character in the highlight color, separators/lowercase in
    /// the foreground color — `defaultMenuColors` = highlight 15 / foreground
    /// 10 (`Gbl.cs:189`). Other widgets draw their plain line.
    pub fn draw_parked_widget(&self, ctx: &mut FlowCtx) {
        if self.combat_host().is_some() {
            return;
        }
        let Some(widget) = self.parked_widget() else {
            return;
        };
        match widget {
            Widget::Hotbar(h) => draw_hotbar_prompt(ctx.fb, ctx.font, h),
            // A VERTICAL MENU's list paints its own box AND its prompt row.
            Widget::ListMenu(l) if l.layout.is_some() => draw_list_menu(ctx.fb, ctx.font, l),
            other => {
                if let Some(line) = other.display_line() {
                    crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, &line);
                }
            }
        }
    }

    /// A one-line state summary for frontend debug logs
    /// (`RESTRIKE_DEBUG_LOG`): the shell variant plus, where a VM vector or
    /// chain is live, its phase — `step/gate(hotbar)`,
    /// `step/combat(Fighting)`, `world-menu`, …
    pub fn probe(&self) -> String {
        match self {
            Shell::Boot(f) => format!("boot/{}", run_probe(&f.run, &f.chain)),
            Shell::WorldMenu { .. } => "world-menu".to_string(),
            Shell::Look(f) => format!("look/{}", run_probe(&f.run, &f.chain)),
            Shell::Step(f) => format!("step/{}", run_probe(&f.run, &f.chain)),
            Shell::GameOver(f) => match (&f.job, &f.gate) {
                (Some(_), _) => "game-over/message".to_string(),
                (None, Some(_)) => "game-over/press-any-key".to_string(),
                _ => "game-over".to_string(),
            },
            Shell::Screen(_) => "screen".to_string(),
            Shell::CampInterrupt(f) => format!("camp-interrupt/{}", run_probe(&f.run, &f.chain)),
        }
    }
}

impl Shell {
    pub fn boot(machine: &mut EclMachine, state: &mut EngineState, vm: &mut VmMemoryState) -> Self {
        Shell::Boot(BootFlow::start(machine, state, vm))
    }

    /// ★ Leaving a full-screen [`Shell::Screen`] recomposes the exploration
    /// screen whole.
    ///
    /// Every screen paints a WHOLE screen (camp's `LoadPic`, the character
    /// sheet's `render_sheet`, the save/training/shop backdrops), so putting
    /// the 3D viewport back is not enough: the border, the right panel and both
    /// text areas are still screen pixels until something paints over them.
    /// Bryan's 2026-08-08 playtest symptom — a stale "Camp" title, torn camp
    /// text and a frameless viewport after leaving camp — is exactly that.
    ///
    /// This is the combat host's `Restore` idiom (`combat_host.rs:490-494`,
    /// itself `free_combat_stuff` + `CMD_Combat`'s `LoadPic` rebuild), minus
    /// the palette swap: **no screen touches the palette registers** — only
    /// combat does (`ovr033.Color_0_8_inverse` on entry / `palette_normal` on
    /// exit), which is why `MakeCamp`'s own exit tail has no palette step. The
    /// three steps that remain are `LoadPic`'s DungeonMap arm
    /// (`ovr025.cs:1435-1441`): clear, `draw8x8_03`, redraw the view — the
    /// caller's [`Shell::enter_world_menu`] supplies the third.
    ///
    /// `MakeCamp`'s tail (`ovr016.cs:1148-1158`) additionally runs
    /// `ClearPlayerTextArea` (`ovr025.cs:822`: rows 0x12-0x16, cols 1-0x26) and
    /// `ClearPromptArea` (`ovr027.cs:350-353`: row 0x18) — both subsumed by the
    /// full clear here. The text CURSOR is deliberately left where it was: the
    /// original's clears are plain rect fills that never touch
    /// `textXCol`/`textYCol` either. The status line comes back on its own,
    /// from the engine tick (`display_map_position_time()` at `:1157`), the
    /// instant the shell is no longer a `Screen`.
    ///
    /// **Deliberately NOT transplanted:** that `LoadPic` arm's fourth step,
    /// `PartySummary(gbl.SelectedPlayer)` (`ovr025.cs:1438`). Our walk loop has
    /// never drawn the right-panel roster at *any* of its recompose sites, so
    /// painting it only on a screen exit would leave a panel nothing else ever
    /// refreshes — worse than the honest blank. Wiring `PartySummary` into
    /// [`Shell::enter_world_menu`] (where it belongs) moves the committed walk
    /// goldens; docketed as its own change.
    fn rebuild_exploration_screen(ctx: &mut FlowCtx) {
        ctx.fb.clear(0);
        // A fixture engine with no symbol set 4 simply gets no border, exactly
        // as the screens' own `draw_frame_outer` calls already tolerate.
        let _ = crate::frames::draw8x8_03(ctx.fb, ctx.symbols);
    }

    /// `main_3d_world_menu`'s entry bookkeeping (`ovr015.cs:352`): zeroes
    /// `field_592` on *every* entry, no exceptions — the required
    /// "field_592 zeroing at menu entry" test target. Also recomposes the
    /// viewport (`crate::corridor::redraw_view`) — a deliberate, documented
    /// simplification of the original's sparser, flag-gated `RedrawView`
    /// call sites (step 5, task deliverable 4's design note): since the
    /// composited result is deterministic and immediate-mode redraws are
    /// idempotent (D-UI4), redrawing every time the player can see the
    /// world menu again is behaviorally equivalent to the original's own
    /// call-site choreography without needing to model the save-load-only
    /// `reload_ecl_and_pictures` gate this session's research found the
    /// original's boot-recompose path actually depends on.
    ///
    /// The recompose destroys any picture on the viewport, which is exactly
    /// the original's behavior — every picture-bearing vector exits through
    /// `PICTURE 0xFF` and/or the `CALL 0xAE11` redraw gate, so a picture
    /// never outlives its scene (`crate::picture`'s module doc has the
    /// evidence). `crate::corridor::redraw_view` clears the picture layer
    /// itself; this call site needs no special handling.
    fn enter_world_menu(ctx: &mut FlowCtx) -> Shell {
        ctx.state.field_592 = 0;
        crate::corridor::redraw_view(ctx);
        let mut hotbar = Hotbar::new("Area Cast View Encamp Search Look");
        hotbar.accept_ext = true;
        Shell::WorldMenu {
            menu: Widget::Hotbar(hotbar),
        }
    }

    /// The D-UI7 mechanical property: true whenever a Widget is parked
    /// anywhere in the current state (a `Gate`, `WorldMenu`'s own menu, or a
    /// `StepFlow`'s door menu) — no vector may be pumped while this holds.
    pub fn gate_open(&self) -> bool {
        fn parked(phase: Option<&VmPhase>) -> bool {
            // A parked fight (`combat-visualizer.md` §8.1) is an interaction
            // exactly as a Widget is: no vector may pump while one is running.
            matches!(
                phase,
                Some(VmPhase::Gate(_)) | Some(VmPhase::Combat(_)) | Some(VmPhase::Temple(_))
            )
        }
        fn run_gated(run: &Option<VectorRun>) -> bool {
            parked(run.as_ref().map(|r| &r.phase))
        }
        fn chain_gated(chain: &Option<ChainRunner>) -> bool {
            parked(chain.as_ref().map(|c| &c.run.phase))
        }
        match self {
            Shell::Boot(b) => run_gated(&b.run) || chain_gated(&b.chain),
            Shell::WorldMenu { .. } => true,
            Shell::Look(l) => run_gated(&l.run) || chain_gated(&l.chain),
            Shell::Step(s) => s.door_widget.is_some() || run_gated(&s.run) || chain_gated(&s.chain),
            // The death screen is an interaction like any other: a message
            // pacing out, then a keypress gate. No vector may pump under it —
            // and none would, since `party_killed` unwound them all.
            Shell::GameOver(_) => true,
            // A screen always has a parked widget (its command bar/list); no
            // VM vector ever runs while one is open.
            Shell::Screen(_) => true,
            // The camp-ambush vector is ordinary script: gated exactly like
            // Look's and Step's.
            Shell::CampInterrupt(c) => run_gated(&c.run) || chain_gated(&c.chain),
        }
    }

    /// Advances the whole shell by one tick.
    pub fn tick(&mut self, ctx: &mut FlowCtx) {
        if ctx.state.party_killed {
            *self = Shell::GameOver(GameOverFlow::start(ctx));
            // `ovr003.cs:2394`: the flag is consumed the moment the loop it
            // broke has exited — the death *screen* is a separate state, not
            // the flag's continued life.
            ctx.state.party_killed = false;
            return;
        }

        match self {
            Shell::Boot(flow) => {
                if flow.tick(ctx).is_some() {
                    ctx.state.last_selected_player = ctx.state.selected_player;
                    *self = Self::enter_world_menu(ctx);
                }
            }
            Shell::WorldMenu { menu } => {
                let outcome = menu.tick(ctx.input, ctx.dt_ticks);
                let WidgetOutcome::Hotbar(key) = outcome else {
                    return; // Pending, or a party-scroll outcome — handled below
                };
                // `ClearPromptArea` on resolution (`ovr027.cs:344-354`).
                crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, "");
                ctx.state.last_selected_player = ctx.state.selected_player; // `:2319`/`:2353`
                if !ctx.state.chained {
                    ctx.state.last_ecl_block_id = ctx.state.ecl_block_id; // `:2321-2324`
                }
                self.dispatch_world_menu_key(key, ctx);
            }
            Shell::Look(flow) => {
                if flow.tick(ctx).is_some() {
                    ctx.state.last_selected_player = ctx.state.selected_player; // `:2353`
                    *self = Self::enter_world_menu(ctx);
                }
            }
            Shell::Step(flow) => {
                if flow.tick(ctx).is_some() {
                    *self = Self::enter_world_menu(ctx);
                }
            }
            Shell::GameOver(flow) => {
                if flow.tick(ctx).is_some() {
                    // The original's `startGameMenu`-with-a-dead-party offers
                    // one way back into the game: Load. That is this screen.
                    *self = Shell::Screen(crate::screens::Screen::SaveLoad(
                        crate::screens::SaveLoad::new_recovery(ctx),
                    ));
                }
            }
            Shell::Screen(screen) => {
                use crate::screens::ScreenTransition;
                match screen.tick(ctx) {
                    ScreenTransition::Stay => {}
                    ScreenTransition::Exit => {
                        Self::rebuild_exploration_screen(ctx);
                        *self = Self::enter_world_menu(ctx);
                    }
                    ScreenTransition::To(next) => *self = Shell::Screen(next),
                    // Declined the recovery load: back to the death screen,
                    // repainted from scratch (the load list overwrote it).
                    ScreenTransition::ToGameOver => {
                        *self = Shell::GameOver(GameOverFlow::start(ctx))
                    }
                    // ★ FD-44. `MakeCamp` returned interrupted: run
                    // `cancel_spells` (its exit tail, `ovr016.cs:1154`),
                    // recompose the exploration screen, and hand the resident
                    // block's vector 3 the party.
                    ScreenTransition::CampInterrupted => {
                        let scrolls = crate::camp_magic::camp_scrolls(ctx);
                        crate::magic::cancel_spells(ctx.roster, &scrolls);
                        Self::rebuild_exploration_screen(ctx);
                        *self = Shell::CampInterrupt(CampInterruptFlow::start(ctx.machine));
                    }
                }
            }
            Shell::CampInterrupt(flow) => {
                if flow.tick(ctx).is_some() {
                    ctx.state.last_selected_player = ctx.state.selected_player;
                    *self = Self::enter_world_menu(ctx);
                }
            }
        }
    }

    fn dispatch_world_menu_key(&mut self, key: u8, ctx: &mut FlowCtx) {
        use WorldMenuCommand::*;
        let cmd = crate::movement::world_menu_command(key, ctx.state.area_view_allowed);
        match cmd {
            ToggleAreaView => {
                ctx.state.area_map_shown = !ctx.state.area_map_shown;
                *self = Self::enter_world_menu(ctx);
            }
            NotHere => {
                // A timed status wait inside the menu (§1.6): parked as a
                // Delay widget, same interaction layer as everything else.
                *self = Shell::WorldMenu {
                    menu: Widget::Delay(Delay::new(24)),
                };
            }
            View => {
                // The character sheet / party view (`ovr020.viewPlayer`),
                // returning to the walk loop on Exit (M3 step 6 deliverable 1).
                *self = Shell::Screen(crate::screens::Screen::PartyView(
                    crate::screens::PartyView::new(ctx, crate::screens::ReturnTo::World),
                ));
            }
            Encamp => {
                // The camp menu (`ovr016.MakeCamp`) — M3 step 6 deliverable 2.
                // (TryEncamp's vector 3/4 area-script dance is out of scope;
                // this enters the menu directly.)
                *self = Shell::Screen(crate::screens::Screen::Camp(crate::screens::Camp::new(ctx)));
            }
            Cast => {
                // M3 stub: casting is M5. Status text only, stays in the menu.
                *self = Self::enter_world_menu(ctx);
            }
            ToggleSearch => {
                ctx.state.search_flags ^= 1;
                *self = Self::enter_world_menu(ctx);
            }
            Look => {
                ctx.state.search_flags |= 2;
                ctx.state.clock.advance(true);
                *self = Shell::Look(LookFlow::start(ctx.machine, ctx.state));
            }
            Forward => {
                ctx.state.tried_to_exit_map =
                    try_step_forward(ctx.geo, ctx.state.pos, ctx.state.facing);
                *self = Shell::Step(StepFlow::start(ctx.machine, ctx.state));
            }
            TurnLeft => {
                ctx.state.facing = ctx.state.facing.turn_left();
                ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
                *self = Self::enter_world_menu(ctx);
            }
            TurnRight => {
                ctx.state.facing = ctx.state.facing.turn_right();
                ctx.sounds.push(SoundEvent(crate::movement::SOUND_A));
                *self = Self::enter_world_menu(ctx);
            }
            TurnAround => {
                ctx.state.facing = ctx.state.facing.turn_around(); // no sound (research finding)
                *self = Self::enter_world_menu(ctx);
            }
            ScrollParty(_) | None => {
                *self = Self::enter_world_menu(ctx);
            }
        }
    }

    /// The fight currently on screen, if any — the parked [`CombatHost`] in
    /// whichever flow's vector run owns it (`combat-visualizer.md` §8.1).
    ///
    /// A read-only seam for the inspector's live pane and for the state-chart
    /// tests; the host is otherwise invisible from outside, which is the point
    /// of parking at interaction level.
    pub fn combat_host(&self) -> Option<&CombatHost> {
        fn in_run(run: &Option<VectorRun>) -> Option<&CombatHost> {
            match run.as_ref().map(|r| &r.phase) {
                Some(VmPhase::Combat(h)) => Some(h),
                _ => None,
            }
        }
        fn in_chain(chain: &Option<ChainRunner>) -> Option<&CombatHost> {
            match chain.as_ref().map(|c| &c.run.phase) {
                Some(VmPhase::Combat(h)) => Some(h),
                _ => None,
            }
        }
        match self {
            Shell::Boot(b) => in_run(&b.run).or_else(|| in_chain(&b.chain)),
            Shell::Look(l) => in_run(&l.run).or_else(|| in_chain(&l.chain)),
            Shell::Step(s) => in_run(&s.run).or_else(|| in_chain(&s.chain)),
            // A camp ambush's own `COMBAT` parks here like any other.
            Shell::CampInterrupt(c) => in_run(&c.run).or_else(|| in_chain(&c.chain)),
            Shell::WorldMenu { .. } | Shell::GameOver(_) | Shell::Screen(_) => None,
        }
    }

    /// ★ The temple currently on screen, if any (roll-credits slice 6 / G8) —
    /// the same read-only seam [`Shell::combat_host`] is, one variant over.
    pub fn temple_host(&self) -> Option<&crate::temple_screen::TempleHost> {
        fn in_run(run: &Option<VectorRun>) -> Option<&crate::temple_screen::TempleHost> {
            match run.as_ref().map(|r| &r.phase) {
                Some(VmPhase::Temple(h)) => Some(h),
                _ => None,
            }
        }
        fn in_chain(chain: &Option<ChainRunner>) -> Option<&crate::temple_screen::TempleHost> {
            match chain.as_ref().map(|c| &c.run.phase) {
                Some(VmPhase::Temple(h)) => Some(h),
                _ => None,
            }
        }
        match self {
            Shell::Boot(b) => in_run(&b.run).or_else(|| in_chain(&b.chain)),
            Shell::Look(l) => in_run(&l.run).or_else(|| in_chain(&l.chain)),
            Shell::Step(s) => in_run(&s.run).or_else(|| in_chain(&s.chain)),
            Shell::CampInterrupt(c) => in_run(&c.run).or_else(|| in_chain(&c.chain)),
            Shell::WorldMenu { .. } | Shell::GameOver(_) | Shell::Screen(_) => None,
        }
    }

    /// The status line every command refreshes (§1.6): `"X,Y DIR HH:MM"`
    /// (+ `" search"`).
    pub fn status_line(state: &EngineState) -> String {
        position_time_text(
            state.pos,
            state.facing,
            &state.clock,
            state.search_mode(),
            false,
        )
    }

    /// Whether the engine-level tick should paint the position/time line over
    /// whatever this shell state just drew.
    ///
    /// The original never draws `display_map_position_time` "every frame": it
    /// runs at explicit recomposition sites only — `LoadPic`'s per-`game_state`
    /// arms (`ovr025.cs:1398-1455`), the walk loop (`ovr003.cs:600,1749-1752`),
    /// `main_3d_world_menu`/`locked_door` (`ovr015.cs:452,579`) and `MakeCamp`'s
    /// exit tail (`ovr016.cs:1157`). None of those fire while a full-screen
    /// state owns the screen, so the unconditional per-tick line landed
    /// mid-layout on every [`Shell::Screen`].
    ///
    /// A fight is excluded for the same reason it always was
    /// (`combat-visualizer.md` §1.1: combat replaces the exploration screen and
    /// row 15 col 17 is inside its right panel). Screens whose *original*
    /// layout does include the line — Camping and Shop, the two `LoadPic` arms
    /// at `ovr025.cs:1425`/`:1432` — draw it themselves, as part of their own
    /// composition (`crate::screens::draw_position_time`).
    /// The death screen is excluded for the same reason: `DrawFrame_Outer`
    /// wiped the whole interior and `AfterCombatExpAndTreasure`'s wipe branch
    /// draws nothing but the message and the prompt (`ovr006.cs:801-809`) —
    /// there is no map position to report when there is no party.
    pub fn draws_engine_status_line(&self) -> bool {
        self.combat_host().is_none()
            && self.temple_host().is_none()
            && !matches!(self, Shell::Screen(_) | Shell::GameOver(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::DefaultPartyPredicates;
    use crate::test_support::{ecl_game_data, exit_only_block, labeled_block, simple_block};
    use gbx_formats::font;
    use gbx_vm::test_support::EclBuilder;

    fn open_geo() -> GeoBlock {
        GeoBlock::parse(&vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE]).unwrap()
    }

    fn marker_font() -> Font {
        let data = vec![0xFFu8; font::GLYPH_COUNT * font::GLYPH_BYTES];
        font::decode(&data)
    }

    /// A trivial single-item 8×8 `ImageBlock` — a fixture stand-in for a
    /// `SKY` block (moon/sun/horizon) this module's flow-control tests
    /// never actually render.
    fn empty_sky_block() -> gbx_formats::image::ImageBlock {
        gbx_formats::image::ImageBlock {
            height: 8,
            width_cols: 1,
            x_pos: 0,
            y_pos: 0,
            field_9: [0; 8],
            items: vec![gbx_formats::image::DecodedItem {
                pixels: vec![0; 64],
            }],
        }
    }

    const GAME_AREA: u8 = 2;

    struct Harness {
        machine: EclMachine,
        vm_memory: VmMemoryState,
        data: GameData,
        input: InputQueue,
        state: EngineState,
        geo: GeoBlock,
        party: DefaultPartyPredicates,
        roster: crate::party::Party,
        rules: gbx_rules::pack::RuleSet,
        slots: crate::saveload::SlotDirectory,
        io_request: Option<crate::saveload::SaveLoadRequest>,
        rng: EngineRng,
        fb: Framebuffer,
        font: Font,
        cursor: TextCursor,
        pacer: TextPacer,
        sounds: Vec<SoundEvent>,
        symbols: crate::symbols::SymbolSets,
        sky: [gbx_formats::image::ImageBlock; 3],
        combat_icons: crate::combat_art::CombatIcons,
        pictures: crate::picture::PictureCache,
    }

    impl Harness {
        /// `blocks`' id `1` becomes the initial resident block; every id
        /// (including `1`) is also reachable via NEWECL/chaining through
        /// `data` (`"ECL2.DAX"`, matching `GAME_AREA`).
        fn with_blocks(blocks: Vec<(u8, EclBuilder)>) -> Self {
            let data = ecl_game_data(GAME_AREA, blocks);
            let initial = load_ecl_block(&data, GAME_AREA, 1).expect("block 1 must load");
            let machine = EclMachine::load_block(initial, &COTAB).unwrap_or_else(|e| match e {});
            let mut state = EngineState::new();
            state.game_area = GAME_AREA;
            state.game_area_backup = GAME_AREA;
            Harness {
                machine,
                vm_memory: VmMemoryState::new(),
                data,
                input: InputQueue::new(),
                state,
                geo: open_geo(),
                party: DefaultPartyPredicates::default(),
                roster: crate::party::Party::default(),
                rules: gbx_rules::pack::RuleSet::load(),
                slots: crate::saveload::SlotDirectory::new(),
                io_request: None,
                rng: EngineRng::new(1),
                fb: Framebuffer::new(),
                font: marker_font(),
                cursor: TextCursor::new(),
                pacer: TextPacer::new(4),
                sounds: Vec::new(),
                symbols: crate::symbols::SymbolSets::new(),
                sky: [empty_sky_block(), empty_sky_block(), empty_sky_block()],
                combat_icons: crate::combat_art::CombatIcons::new(),
                pictures: crate::picture::PictureCache::new(),
            }
        }

        fn new() -> Self {
            Self::with_blocks(vec![(1, exit_only_block())])
        }

        fn ctx(&mut self) -> FlowCtx<'_> {
            FlowCtx {
                machine: &mut self.machine,
                vm_memory: &mut self.vm_memory,
                data: &self.data,
                input: &mut self.input,
                dt_ticks: 1,
                state: &mut self.state,
                geo: &mut self.geo,
                party: &mut self.party,
                roster: &mut self.roster,
                rules: &self.rules,
                slots: &self.slots,
                io_request: &mut self.io_request,
                rng: &mut self.rng,
                fb: &mut self.fb,
                font: &self.font,
                cursor: &mut self.cursor,
                pacer: &mut self.pacer,
                sounds: &mut self.sounds,
                symbols: &mut self.symbols,
                sky: &self.sky,
                combat_icons: &self.combat_icons,
                pictures: &mut self.pictures,
            }
        }
    }

    /// Ticks at least once, then up to `max_ticks` times total, stopping as
    /// soon as `done` holds — always ticks first so a call starting already
    /// in the target state (e.g. queued input meant to move `shell` *out*
    /// of `WorldMenu` and back) still gets a chance to consume that input,
    /// rather than returning immediately without ticking at all.
    fn tick_until(
        shell: &mut Shell,
        h: &mut Harness,
        max_ticks: u32,
        done: impl Fn(&Shell) -> bool,
    ) {
        for _ in 0..max_ticks {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
            if done(shell) {
                return;
            }
        }
        assert!(done(shell), "did not converge within {max_ticks} ticks");
    }

    /// ★ `buildMenuStrings`' case transform (`ovr008.cs:1131-1165`) applied
    /// at `widget_for_request`: one capital per option, so the highlight
    /// groups are whole options, only real hotkeys resolve, Enter commits
    /// the highlighted option, and a stray key is ignored — pressing 'X' at
    /// the bar no longer starts the brawl (Bryan, 2026-08-08).
    #[test]
    fn a_script_menu_groups_whole_options_and_ignores_stray_keys() {
        use crate::input::InputEvent;
        use gbx_vm::VmString;
        let options = vec![
            VmString(b"PUNCH BARKEEP".to_vec()),
            VmString(b"HAVE A DRINK".to_vec()),
            VmString(b"LEAVE".to_vec()),
        ];
        let Widget::Hotbar(mut h) = widget_for_request(
            &Request::HorizontalMenu {
                options: options.clone(),
            },
            0,
        ) else {
            panic!("a HORIZONTAL MENU parks a Hotbar");
        };
        assert_eq!(h.text, "Punch barkeep Have a drink Leave");
        assert_eq!(
            h.valid_keys.as_deref(),
            Some(&b"PHL"[..]),
            "only the ~-marked hotkeys resolve (sub_317AA via the letter scan)"
        );
        let span = h.selected_span().expect("option 0 opens highlighted");
        assert_eq!(&h.text[span.0..span.1], "Punch barkeep");

        let mut q = crate::input::InputQueue::new();

        // A stray letter is ignored (`displayInput` keeps waiting,
        // `ovr027.cs:267-292`) — the pre-fix fallback resolved it to
        // option 0.
        q.push_all(&[InputEvent::Char(b'X')]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Pending);

        // ',' wraps the highlight back to LEAVE; Enter commits it as its
        // own hotkey (`ovr027.cs:226-240`, the `,`/`.`/Enter loop).
        q.push_all(&[InputEvent::Char(b',')]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Pending);
        q.push_all(&[InputEvent::Enter]);
        assert_eq!(h.tick(&mut q, 1), WidgetOutcome::Hotbar(b'L'));

        // The reply maps hotkeys to option indexes, with sub_317AA's
        // not-found -1 -> 0xFF sentinel instead of a silent option 0.
        assert_eq!(
            resolve_horizontal_menu_reply(&options, b'H'),
            Reply::Selection(1)
        );
        assert_eq!(
            resolve_horizontal_menu_reply(&options, b'X'),
            Reply::Selection(0xFF)
        );
    }

    /// ★ `CMD_HorizontalMenu`'s single-option prompt rewrite
    /// (`ovr003.cs:711-721`): the shipped ECL's mouse-era
    /// `"PRESS BUTTON OR RETURN TO CONTINUE."` is canonicalized to
    /// `"PRESS <ENTER>/<RETURN> TO CONTINUE"` before `buildMenuStrings`, so
    /// the displayed text (and the transcript that mirrors it) carries the
    /// keyboard prompt the player actually reads.
    #[test]
    fn the_single_option_continue_prompt_is_canonicalized() {
        use gbx_vm::VmString;
        let request = Request::HorizontalMenu {
            options: vec![VmString(PRESS_BUTTON_RAW.as_bytes().to_vec())],
        };
        let Widget::Hotbar(h) = widget_for_request(&request, 0) else {
            panic!("a HORIZONTAL MENU parks a Hotbar");
        };
        // `buildMenuStrings` then lowercases everything but the one `~` mark.
        assert_eq!(h.text, "Press <enter>/<return> to continue");
        assert_eq!(
            describe_request(&request),
            "menu: PRESS <ENTER>/<RETURN> TO CONTINUE",
            "the transcript shows what the player saw, not the raw script bytes"
        );

        // The rewrite is exact-match and single-option only.
        let two = Request::HorizontalMenu {
            options: vec![
                VmString(PRESS_BUTTON_RAW.as_bytes().to_vec()),
                VmString(b"QUIT".to_vec()),
            ],
        };
        assert_eq!(
            describe_request(&two),
            format!("menu: {PRESS_BUTTON_RAW} QUIT"),
            "a multi-option menu is untouched (string_count == 1 is part of the condition)"
        );
        let near_miss = Request::HorizontalMenu {
            options: vec![VmString(b"PRESS BUTTON OR RETURN TO CONTINUE".to_vec())],
        };
        assert_eq!(
            describe_request(&near_miss),
            "menu: PRESS BUTTON OR RETURN TO CONTINUE",
            "the match includes the trailing period"
        );
    }

    /// ★ An extended key at a parked script menu scrolls the team list and
    /// re-prompts (`sub_317AA`'s special-key arm, `ovr008.cs:1181-1187`) —
    /// it must NEVER resolve the menu. The left arrow used to fall into the
    /// resolution match's fallback and reply option 0, which at the bar was
    /// PUNCH BARKEEP: Bryan's left-arrow brawl (2026-08-08).
    #[test]
    fn an_arrow_key_at_a_script_menu_reprompts_instead_of_resolving() {
        use crate::input::{ExtKey, InputEvent};
        let menu = simple_block(|b| {
            b.op(0x2B) // HORIZONTAL MENU
                .mem(0x5000)
                .imm_byte(2)
                .inline_str(b"FIGHT")
                .inline_str(b"NO");
            b.op(0x00);
        });
        let mut h = Harness::with_blocks(vec![(1, menu)]);
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(shell.gate_open(), "the menu parked");

        // Every arrow/nav key: still parked afterwards.
        for ext in [
            ExtKey::Left,
            ExtKey::Right,
            ExtKey::Up,
            ExtKey::Down,
            ExtKey::Home,
            ExtKey::End,
        ] {
            {
                let ctx = h.ctx();
                ctx.input.push_all(&[InputEvent::Ext(ext)]);
            }
            {
                let mut ctx = h.ctx();
                shell.tick(&mut ctx);
            }
            assert!(
                shell.gate_open(),
                "{ext:?} must scroll-and-reprompt, not resolve the menu"
            );
        }

        // The real hotkey still resolves it.
        {
            let ctx = h.ctx();
            ctx.input.push_all(&[InputEvent::Char(b'N')]);
        }
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            matches!(shell, Shell::WorldMenu { .. }),
            "'N' resolved the menu and the script ran out"
        );
    }

    /// ★ `gbl.menuSelectedWord` is GLOBAL and never reset — the Tilverton
    /// bar's own scenario. The script menu loops (press 'H', drink, the menu
    /// re-opens); the original re-opens with HAVE A DRINK still highlighted
    /// (`displayInput` only clamps the global on entry, `ovr027.cs:142-145`,
    /// and its letter-scan arm wrote it at `:289`), so Enter orders another
    /// drink. Ours used to re-open on option 0 — PUNCH BARKEEP — and Enter
    /// after a re-open started the brawl.
    #[test]
    fn a_looping_script_menu_reopens_on_the_last_chosen_option() {
        use crate::input::InputEvent;
        let bar = |b: &mut EclBuilder| {
            b.op(0x2B) // HORIZONTAL MENU
                .mem(0x5000)
                .imm_byte(3)
                .inline_str(b"PUNCH BARKEEP")
                .inline_str(b"HAVE A DRINK")
                .inline_str(b"LEAVE");
        };
        let block = simple_block(|b| {
            bar(b);
            bar(b); // the loop's second pass
            b.op(0x00);
        });
        let mut h = Harness::with_blocks(vec![(1, block)]);
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);

        let selected_text = |shell: &Shell| -> String {
            let Some(Widget::Hotbar(hb)) = shell.parked_widget_for_tests() else {
                panic!("a script menu parks a Hotbar");
            };
            let span = hb.selected_span().expect("something is highlighted");
            hb.text[span.0..span.1].to_string()
        };

        tick_until(&mut shell, &mut h, 10, |s| s.gate_open());
        assert_eq!(
            selected_text(&shell),
            "Punch barkeep",
            "a fresh global (0) opens on option 0"
        );

        // 'H' resolves the menu AND moves the highlight (`ovr027.cs:279-292`).
        h.input.push_all(&[InputEvent::Char(b'H')]);
        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(
            h.state.menu_selected_word, 1,
            "the resolving hotkey wrote the global"
        );

        // The menu re-opens: HAVE A DRINK is still highlighted.
        tick_until(&mut shell, &mut h, 10, |s| s.gate_open());
        assert_eq!(selected_text(&shell), "Have a drink");

        // ...so Enter orders another drink instead of starting the brawl.
        h.input.push_all(&[InputEvent::Enter]);
        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.menu_selected_word, 1);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_eq!(
            h.vm_memory.raw_word(0x5000),
            Some(1),
            "Enter after the re-open selected HAVE A DRINK (index 1), not PUNCH BARKEEP"
        );
    }

    /// `displayInput`'s entry clamp (`ovr027.cs:142-145`) resets to ZERO when
    /// the stored word is at or past the new menu's group count — it does not
    /// clamp to the last group.
    #[test]
    fn a_stored_word_past_the_new_menus_group_count_clamps_to_zero() {
        use gbx_vm::VmString;
        let request = Request::HorizontalMenu {
            options: vec![VmString(b"YES".to_vec()), VmString(b"NO".to_vec())],
        };
        let Widget::Hotbar(h) = widget_for_request(&request, 1) else {
            panic!("a HORIZONTAL MENU parks a Hotbar");
        };
        assert_eq!(h.selected_word(), 1, "an in-range stored word survives");

        let Widget::Hotbar(h) = widget_for_request(&request, 5) else {
            panic!("a HORIZONTAL MENU parks a Hotbar");
        };
        assert_eq!(h.selected_word(), 0, "out of range resets to 0, not to 1");
    }

    #[test]
    fn boot_reaches_world_menu_with_no_chain() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    /// ★ `vm_init_ecl`'s engine-state half at the block-entry preamble
    /// (`ovr003.cs:2278` → `ovr008.cs:109`): booting resets `HeadBlockId` to
    /// the no-portrait sentinel `0xFF`, no matter what an imported original
    /// save left in the Area2 cell. The bundled GOG save carries `0x00` there,
    /// which sent the intro's first `PICTURE` down the head/body arm (a wrong
    /// face + a `BODY2` block that does not exist) instead of the plain-PIC
    /// sword arm — Bryan's 2026-08-08 playtest find.
    #[test]
    fn boot_resets_the_imported_head_block_id_to_none() {
        let mut h = Harness::new();
        h.state.head_block_id = 0x00; // what the bundled save's Area2 bytes say
        let _shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        assert_eq!(
            h.state.head_block_id, 0xFF,
            "the block-entry preamble must arm the no-portrait sentinel"
        );
    }

    /// The same reset at `CMD_NewECL`'s own `vm_init_ecl` call
    /// (`ovr003.cs:491-492`): a chained-to block starts with no portrait head
    /// armed, whatever the previous block's scripts wrote to the cell.
    #[test]
    fn newecl_chain_resets_head_block_id() {
        let newecl = simple_block(|b| {
            b.op(0x20).imm_byte(2); // NEWECL block 2
        });
        let mut h = Harness::with_blocks(vec![(1, newecl), (2, exit_only_block())]);
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        // Poke stale values AFTER boot's own reset so the chain site is the
        // one under test. Assert right after the tick that fires the chain —
        // the world-menu entry's own recompose clears the redraw flags again
        // (`corridor::redraw_view` → `clear_redraw_flags`), so the armed
        // gate is only observable mid-chain.
        h.state.head_block_id = 0x05;
        h.vm_memory.byte_1ee91 = false;
        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.ecl_block_id, 2, "the chain fired");
        assert_eq!(
            h.state.head_block_id, 0xFF,
            "NEWECL's vm_init_ecl must re-arm the no-portrait sentinel"
        );
        assert!(
            h.vm_memory.byte_1ee91,
            "NEWECL's vm_init_ecl must arm byte_1EE91 (`ovr008.cs:94`)"
        );
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_eq!(h.state.head_block_id, 0xFF, "the sentinel survives");
    }

    #[test]
    fn boot_resume_after_chain_clears_reload_flag_only_after_the_chain_finishes() {
        let newecl = simple_block(|b| {
            b.op(0x20).imm_byte(2); // NEWECL block 2
        });
        let mut h = Harness::with_blocks(vec![(1, newecl), (2, exit_only_block())]);
        h.state.reload_ecl_and_pictures = true;
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);

        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            h.state.reload_ecl_and_pictures,
            "must not clear before the chain resolves"
        );
        assert!(h.state.chained);

        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert!(
            !h.state.reload_ecl_and_pictures,
            "must clear once resumed post-chain"
        );
        assert!(!h.state.chained);
        assert_eq!(h.state.ecl_block_id, 2);
    }

    #[test]
    fn world_menu_forward_into_open_square_moves_and_returns_to_world_menu() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        // Forward is driven by the extended "up" key (resolves through
        // accept_ext's ctrl-code table to 'H'), not a literal typed 'h'.
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Up)]);
        let start_pos = h.state.pos;
        tick_until(&mut shell, &mut h, 20, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_ne!(
            h.state.pos, start_pos,
            "an open square must let the party step forward"
        );
    }

    #[test]
    fn party_killed_unwinds_to_game_over_and_resets_the_flag() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        h.state.party_killed = true;
        let mut ctx = h.ctx();
        shell.tick(&mut ctx);
        assert!(matches!(shell, Shell::GameOver(_)));
        assert!(!h.state.party_killed, "the flag resets on unwind");
    }

    #[test]
    fn world_menu_with_chained_set_is_a_reachable_valid_state() {
        // The M3-camp-case invariant (D-UI2): WorldMenu can run with
        // `chained` still set — the flag survives the menu, only cleared at
        // the next step's checkpoint.
        let mut h = Harness::new();
        h.state.chained = true;
        let shell = Shell::enter_world_menu(&mut h.ctx());
        assert!(matches!(shell, Shell::WorldMenu { .. }));
        assert!(
            h.state.chained,
            "WorldMenu entry must not itself clear chained"
        );
    }

    #[test]
    fn field_592_zeroes_on_every_world_menu_entry() {
        let mut h = Harness::new();
        h.state.field_592 = 0xFF;
        let _ = Shell::enter_world_menu(&mut h.ctx());
        assert_eq!(h.state.field_592, 0);
    }

    #[test]
    fn no_vector_pumps_while_a_gate_is_open() {
        // Mechanical D-UI7 property: a widget requiring several ticks to
        // resolve must keep `gate_open()` true and never advance the
        // machine on its own (proven here by ticking many times with no
        // input and observing the gate never silently closes).
        let combat = simple_block(|b| {
            b.op(0x24); // COMBAT — a Request the interpreter can really emit
            b.op(0x00); // EXIT (after the reply resumes)
        });
        let mut h = Harness::with_blocks(vec![(1, combat)]);
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            shell.gate_open(),
            "Combat's PressAnyKey stub must be parked"
        );
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            shell.gate_open(),
            "must still be gated with no input supplied"
        );
    }

    #[test]
    fn shell_state_round_trips_through_serde_mid_boot() {
        let mut h = Harness::new();
        let shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        let json = serde_json::to_string(&shell).expect("Shell must serialize");
        let restored: Shell = serde_json::from_str(&json).expect("Shell must deserialize");
        assert!(matches!(restored, Shell::Boot(_)));
    }

    #[test]
    fn widget_round_trips_through_serde() {
        let widget = Widget::Hotbar(Hotbar::new("Yes No"));
        let json = serde_json::to_string(&widget).unwrap();
        let restored: Widget = serde_json::from_str(&json).unwrap();
        assert_eq!(widget, restored);
    }

    #[test]
    fn look_flow_restores_search_flags_after_resolving() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        h.input.push_all(&[crate::input::InputEvent::Char(b'l')]);
        tick_until(&mut shell, &mut h, 15, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_eq!(h.state.search_flags, 0, "bit 1 must never survive a Look");
    }

    #[test]
    fn door_menu_parks_directly_in_step_flow_not_via_vmphase_gate_over_a_vector() {
        // The Fable review finding, proven structurally: a locked door with
        // no VM run active still reports a parked widget.
        let mut geo_data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        // Square (0,0): North wall present, door state 2 (locked).
        geo_data[2] = 5 << 4;
        geo_data[2 + 3 * 256] = 0b10; // door_north = 2
        let geo = GeoBlock::parse(&geo_data).unwrap();

        let mut h = Harness::new();
        h.geo = geo;
        h.party.can_pick = false;
        h.party.can_knock = false;
        let mut flow = StepFlow::start(&mut h.machine, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx);
            if flow.door_widget_is_some() {
                break;
            }
        }
        assert!(
            flow.door_widget_is_some(),
            "the Bash/Exit menu must be parked directly"
        );
    }

    #[test]
    fn combat_request_maps_to_press_any_key_stub() {
        let options = vec![gbx_vm::VmString::from_bytes(*b"Yes")];
        let w = widget_for_request(&Request::HorizontalMenu { options }, 0);
        assert!(matches!(w, Widget::Hotbar(_)));
        let w = widget_for_request(&Request::Combat, 0);
        assert!(matches!(w, Widget::PressAnyKey(_)));
        let w = widget_for_request(&Request::Delay, 0);
        assert!(matches!(w, Widget::Delay(_)));
    }

    fn round_trip_shell(shell: &Shell) -> Shell {
        let json = serde_json::to_string(shell).expect("Shell must serialize");
        serde_json::from_str(&json).expect("Shell must deserialize")
    }

    #[test]
    fn every_shell_variant_round_trips_through_serde() {
        let mut h = Harness::new();

        assert!(matches!(
            round_trip_shell(&Shell::GameOver(GameOverFlow::awaiting_key())),
            Shell::GameOver(_)
        ));
        // The death screen mid-message carries a live `TextJob` — the state a
        // save taken during the wipe would have to reconstruct.
        let dying = Shell::GameOver(GameOverFlow::start(&mut h.ctx()));
        assert!(matches!(round_trip_shell(&dying), Shell::GameOver(_)));

        let world_menu = Shell::enter_world_menu(&mut h.ctx());
        assert!(matches!(
            round_trip_shell(&world_menu),
            Shell::WorldMenu { .. }
        ));

        let step = Shell::Step(StepFlow::start(&mut h.machine, &mut h.state));
        assert!(matches!(round_trip_shell(&step), Shell::Step(_)));

        let look = Shell::Look(LookFlow::start(&mut h.machine, &mut h.state));
        assert!(matches!(round_trip_shell(&look), Shell::Look(_)));

        let boot = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        assert!(matches!(round_trip_shell(&boot), Shell::Boot(_)));
    }

    #[test]
    fn every_widget_variant_round_trips_through_serde() {
        fn round_trip(w: &Widget) -> Widget {
            let json = serde_json::to_string(w).unwrap();
            serde_json::from_str(&json).unwrap()
        }
        let variants = vec![
            Widget::Hotbar(Hotbar::new("Yes No")),
            Widget::ListMenu(crate::widgets::ListMenu::new(
                vec![crate::widgets::ListItem::Entry("x".into())],
                3,
            )),
            Widget::TextEntry(crate::widgets::TextEntry::new("Name?", 10, false)),
            Widget::PressAnyKey(PressAnyKey),
            Widget::Delay(Delay::new(5)),
        ];
        for w in variants {
            assert_eq!(round_trip(&w), w);
        }
    }

    #[test]
    fn forward_at_the_grid_edge_sets_tried_to_exit_map() {
        let mut h = Harness::new();
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        // Facing North at y=0: stepping forward would exit the 16x16 grid.
        assert_eq!(h.state.pos, (0, 0));
        assert_eq!(h.state.facing, Facing::North);
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Up)]);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            h.state.tried_to_exit_map,
            "stepping off the grid must set the flag"
        );
    }

    #[test]
    fn solid_wall_blocks_movement_with_no_menu() {
        let mut geo_data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        geo_data[2] = 5 << 4; // North wall present, door state 0 = solid
        let geo = GeoBlock::parse(&geo_data).unwrap();

        let mut h = Harness::new();
        h.geo = geo;
        let mut flow = StepFlow::start(&mut h.machine, &mut h.state);
        let start_pos = h.state.pos;
        for _ in 0..5 {
            let mut ctx = h.ctx();
            if flow.tick(&mut ctx).is_some() {
                break;
            }
        }
        assert_eq!(h.state.pos, start_pos, "a solid wall must block the step");
    }

    #[test]
    fn unpickable_door_pick_never_succeeds_via_the_full_shell_path() {
        let mut geo_data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        geo_data[2] = 5 << 4; // North wall present
        geo_data[2 + 3 * 256] = 0b11; // door_north = 3 (unpickable)
        let geo = GeoBlock::parse(&geo_data).unwrap();

        let mut h = Harness::new();
        h.geo = geo;
        h.party.can_pick = true;
        h.party.pick_succeeds = true; // would succeed if it ever rolled
        let mut flow = StepFlow::start(&mut h.machine, &mut h.state);
        for _ in 0..5 {
            let mut ctx = h.ctx();
            let _ = flow.tick(&mut ctx);
            if flow.door_widget_is_some() {
                break;
            }
        }
        assert!(flow.door_widget_is_some());
        h.input.push_all(&[crate::input::InputEvent::Char(b'p')]);
        let start_pos = h.state.pos;
        for _ in 0..5 {
            let mut ctx = h.ctx();
            if flow.tick(&mut ctx).is_some() {
                break;
            }
        }
        assert_eq!(
            h.state.pos, start_pos,
            "an unpickable door never opens via Pick"
        );
        assert!(
            !h.state.door_flags.can_pick,
            "Pick is disabled after the attempt regardless"
        );
    }

    #[test]
    fn chain_during_look_resumes_at_restore_search_flags_not_abandoned() {
        // Block 1: vector[4] (entry) is a trivial EXIT so Boot reaches
        // WorldMenu normally; vector[1] (SearchLocationAddr, the one Look's
        // vector 2 fires) is a separate label that NEWECLs to block 9 —
        // proving the chain fires specifically from the Look site.
        let block1 = labeled_block(["entry", "search", "entry", "entry", "entry"], |b| {
            b.label("entry");
            b.op(0x00); // EXIT
            b.label("search");
            b.op(0x20).imm_byte(9); // NEWECL block 9
        });
        let mut h = Harness::with_blocks(vec![(1, block1), (9, exit_only_block())]);

        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        h.input.push_all(&[crate::input::InputEvent::Char(b'l')]);
        for _ in 0..15 {
            if matches!(shell, Shell::WorldMenu { .. }) && h.state.ecl_block_id == 9 {
                break;
            }
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(matches!(shell, Shell::WorldMenu { .. }));
        assert_eq!(h.state.ecl_block_id, 9);
        assert!(
            !h.state.chained,
            "the chain runner must finish and clear the flag"
        );
        assert_eq!(
            h.state.search_flags, 0,
            "search_flags restore must still run after the chain resolves (resume-after-chain)"
        );
    }

    // --- M3 step 6: party-facing menu screens (View/Camp/Magic) ---

    use crate::screens::Screen;

    fn test_char(name: &str) -> crate::party::Character {
        use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE};
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0] = name.len() as u8;
        bytes[1..1 + name.len()].copy_from_slice(name.as_bytes());
        let rec = decode_char_record(&bytes).unwrap();
        crate::party::character_from_record(&rec, vec![], vec![])
    }

    fn char_key(c: u8) -> crate::input::InputEvent {
        crate::input::InputEvent::Char(c)
    }

    /// Boots to the world menu with a two-member roster.
    fn boot_with_party() -> (Shell, Harness) {
        let mut h = Harness::new();
        h.roster.members = vec![test_char("Aran"), test_char("Bink")];
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        (shell, h)
    }

    #[test]
    fn world_menu_view_opens_the_party_view_and_exit_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'v')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::PartyView(_)))
        });
        // The command bar for a no-money character is just "Exit"; 'E' resolves
        // it and returns to the walk-loop world menu.
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    #[test]
    fn world_menu_encamp_opens_camp_and_exit_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'e')]); // camp Exit ("Exit" is the sole 'E' word)
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    /// ★ Bryan's 2026-08-08 playtest find: leaving camp left a stale "Camp"
    /// title, torn camp text and a frameless viewport. Every screen paints a
    /// WHOLE screen, so the exit has to recompose one (`ovr025.cs:1435-1441`'s
    /// DungeonMap `LoadPic` arm; `combat_host.rs:490-494`'s same idiom).
    #[test]
    fn leaving_a_screen_rebuilds_the_exploration_screen_instead_of_leaving_stale_pixels() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        // One more tick so camp actually paints its layout.
        {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        // "The party makes camp..." at row 18, col 1 (`ovr016.cs:1093`).
        assert_ne!(
            h.fb.get_pixel(8, 18 * 8),
            0,
            "camp must have painted its own text area"
        );

        h.input.push_all(&[char_key(b'e')]); // camp Exit
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        assert_eq!(
            h.fb.get_pixel(8, 18 * 8),
            0,
            "the screen's own pixels must not survive its exit"
        );
    }

    /// `MakeCamp`'s `special_key` arm (`ovr016.cs:1105-1109`) →
    /// `scroll_team_list` (`ovr020.cs:1349-1368`): End/numpad-1 next,
    /// Home/numpad-7 previous, both wrapping.
    #[test]
    fn camp_extended_keys_scroll_the_team_list() {
        use crate::input::{ExtKey, InputEvent};
        let (mut shell, mut h) = boot_with_party(); // two members
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        assert_eq!(h.state.selected_player, 0);

        h.input.push_all(&[InputEvent::Ext(ExtKey::End)]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.selected_player, 1, "End selects the next member");

        h.input.push_all(&[InputEvent::Ext(ExtKey::Home)]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.selected_player, 0, "Home selects the previous");

        // Still in camp: a special key never resolves the bar.
        assert!(matches!(shell, Shell::Screen(Screen::Camp(_))));

        // FD-18: the arrow keys do nothing in a Gold Box list.
        h.input.push_all(&[InputEvent::Ext(ExtKey::Down)]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.selected_player, 0, "Down is not a scroll key");
    }

    /// ★ **FD-44's acceptance** (roll-credits §8, D-S4e): a rest interruption
    /// runs the resident block's header **vector 3**, `CampInterruptedAddr`.
    ///
    /// Proved the way the Look-vector test proves its own site: vector 3 is the
    /// only vector that `NEWECL`s to block 9, so reaching block 9 can only have
    /// come from the camp-ambush path. The schedule is armed to fire on the
    /// first rest iteration (period 1 / percentage 100) through the same
    /// Party-window cells `ECL4#37 @0x822E` writes.
    #[test]
    fn an_interrupted_rest_runs_the_blocks_camp_ambush_vector() {
        // vectors: [run_addr_1, search, pre_camp, CAMP INTERRUPTED, entry]
        let block1 = labeled_block(["entry", "entry", "entry", "ambush", "entry"], |b| {
            b.label("entry");
            b.op(0x00); // EXIT
            b.label("ambush");
            b.op(0x20).imm_byte(9); // NEWECL block 9
        });
        let mut h = Harness::with_blocks(vec![(1, block1), (9, exit_only_block())]);
        h.roster.members = vec![test_char("Aran"), test_char("Bink")];
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        // Arm the schedule: every iteration fires, and always interrupts.
        h.vm_memory
            .poke_raw(crate::rest::REST_INCOUNTER_PERIOD_ADDR, 1);
        h.vm_memory
            .poke_raw(crate::rest::REST_INCOUNTER_PERCENTAGE_ADDR, 100);

        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'r')]); // Rest
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Rest(_)))
        });
        // Nothing is staged, so the countdown is zero — add eight hours so the
        // loop has something to be interrupted during.
        h.input.push_all(&[char_key(b'h')]);
        for _ in 0..8 {
            h.input.push_all(&[char_key(b'a')]);
        }
        h.input.push_all(&[char_key(b'r')]); // commit

        let mut saw_interrupt = false;
        for _ in 0..80 {
            if matches!(shell, Shell::CampInterrupt(_)) {
                saw_interrupt = true;
            }
            if saw_interrupt && matches!(shell, Shell::WorldMenu { .. }) {
                break;
            }
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            saw_interrupt,
            "the fired schedule ended camp into the vector-3 run: {}",
            shell.probe()
        );
        assert_eq!(
            h.state.ecl_block_id, 9,
            "vector 3 — and only vector 3 — chains to block 9"
        );
        assert!(matches!(shell, Shell::WorldMenu { .. }), "and it resumed");
    }

    /// The rest that is *not* interrupted goes back to camp, never through the
    /// ambush vector — the same fixture, schedule disarmed.
    #[test]
    fn an_uninterrupted_rest_never_touches_the_ambush_vector() {
        let block1 = labeled_block(["entry", "entry", "entry", "ambush", "entry"], |b| {
            b.label("entry");
            b.op(0x00);
            b.label("ambush");
            b.op(0x20).imm_byte(9);
        });
        let mut h = Harness::with_blocks(vec![(1, block1), (9, exit_only_block())]);
        h.roster.members = vec![test_char("Aran")];
        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'r')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Rest(_)))
        });
        h.input
            .push_all(&[char_key(b'h'), char_key(b'a'), char_key(b'r')]);
        tick_until(&mut shell, &mut h, 200, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        assert_eq!(h.state.ecl_block_id, 1, "the ambush vector never ran");
    }

    /// ★ Camp exit runs `cancel_spells` (`ovr016.cs:1154`): staged spells do
    /// not survive walking out, memorized ones do.
    #[test]
    fn leaving_camp_cancels_staged_spells_but_keeps_memorized_ones() {
        let (mut shell, mut h) = boot_with_party();
        h.roster.members[0].magic.spell_list = vec![0u8; crate::magic::SPELL_LIST_SIZE];
        crate::magic::add_learnt(&mut h.roster.members[0].magic.spell_list, 0x03);
        crate::magic::add_learn(&mut h.roster.members[0].magic.spell_list, 0x0F);
        h.roster.members[0].magic.spell_to_learn_count = 4;

        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'e')]); // Exit camp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        let list = &h.roster.members[0].magic.spell_list;
        assert_eq!(crate::magic::learning(list).count(), 0, "staging is gone");
        let left: Vec<u8> = crate::magic::learnt(list).map(|e| e.id).collect();
        assert_eq!(left, vec![0x03], "memory survives");
        assert_eq!(h.roster.members[0].magic.spell_to_learn_count, 0);
    }

    /// ★ The Magic submenu's leaves reach their screens (D-S4d): Memorize and
    /// Scribe open, Display lists the party's effects, and each returns.
    #[test]
    fn the_magic_menu_leaves_open_their_screens() {
        let (mut shell, mut h) = boot_with_party();
        h.roster.members[0].magic.spell_list = vec![0u8; crate::magic::SPELL_LIST_SIZE];
        h.roster.members[0].magic.spell_book = vec![0u8; 100];
        h.roster.members[0].magic.spell_book[0x0F - 1] = 1; // knows Magic Missile
        h.roster.members[0].magic.cast_count[2][0] = 2; // …and has MU level-1 slots
        h.roster.members[0].stats.int.original = 16;
        h.roster.members[0].class_level[crate::party::SKILL_MAGIC_USER] = 3;
        h.roster.members[0].status.in_combat = true;

        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'm')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Magic(_)))
        });

        // Display: `DisplayMagicEffects` — every member listed, no effects.
        h.input.push_all(&[char_key(b'd')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SpellEffects(_)))
        });
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Magic(_)))
        });

        // Memorize: nothing staged, so it opens straight on the grimoire
        // picker; Enter stages the highlighted spell.
        h.input.push_all(&[char_key(b'm')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Memorize(_)))
        });
        h.input.push_all(&[crate::input::InputEvent::Enter]);
        for _ in 0..4 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(
            crate::magic::learning(&h.roster.members[0].magic.spell_list)
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![0x0F],
            "the picker staged Magic Missile"
        );

        // Scribe with no scrolls refuses and bounces back with its line.
        h.input.push_all(&[char_key(b'e')]); // out of the picker
        for _ in 0..4 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        h.input.push_all(&[char_key(b'y')]); // keep the staging
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Magic(_)))
        });
        h.input.push_all(&[char_key(b's')]);
        for _ in 0..6 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert!(
            matches!(shell, Shell::Screen(Screen::Magic(_)))
                || matches!(shell, Shell::Screen(Screen::Scribe(_))),
            "Scribe resolved one way or the other: {}",
            shell.probe()
        );
    }

    /// ★ `MakeCamp`'s View/Magic/Rest/Alter arms each set
    /// `gbl.menuSelectedWord = 1` **before** dispatching
    /// (`ovr016.cs:1123`/`:1128`/`:1133`/`:1142`), so the camp bar comes back
    /// from a sub-action with its FIRST word highlighted — not the word that
    /// was pressed. (This corrects the pre-slice-4 expectation, which came from
    /// Rest being a no-op that never left the bar.)
    #[test]
    fn camp_resets_its_highlight_when_a_sub_action_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'r')]); // Rest → the rest screen
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Rest(_)))
        });
        h.input.push_all(&[char_key(b'e')]); // Exit the rest menu
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        let Shell::Screen(Screen::Camp(camp)) = &shell else {
            panic!("back in camp");
        };
        let (start, _) = camp.selected_span().expect("a word is highlighted");
        assert_eq!(
            &"Save View Magic Rest Alter Fix Exit"[start..start + 4],
            "Save",
            "menuSelectedWord = 1 puts the highlight back on the first word"
        );
    }

    /// The status-line decision, per shell state ((a)'s coab derivation lives
    /// on the method).
    #[test]
    fn the_engine_status_line_is_suppressed_only_over_screens_and_fights() {
        let (mut shell, mut h) = boot_with_party();
        assert!(shell.draws_engine_status_line(), "the walk loop draws it");
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        assert!(
            !shell.draws_engine_status_line(),
            "a parked full-screen Screen composes its own line (or none)"
        );
    }

    #[test]
    fn camp_view_returns_to_camp_not_the_world_menu() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'v')]); // camp View
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::PartyView(_)))
        });
        // Escape leaves the sheet — but back to camp, not the world menu.
        h.input.push_all(&[crate::input::InputEvent::Escape]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
    }

    #[test]
    fn camp_magic_opens_the_magic_submenu_and_returns() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b'm')]); // Magic
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Magic(_)))
        });
        h.input.push_all(&[char_key(b'e')]); // Magic Exit → camp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
    }

    #[test]
    /// ★ Camp Rest opens `rest_menu`'s own screen (`ovr016.cs:1134`), and
    /// resting it through **never** touches `spellCastCount` — FD-25's finding,
    /// re-pinned here at the shell level as well as in `crate::rest`. With
    /// nothing staged the required time is zero, so the loop ends immediately
    /// and the party comes back exactly as it went in.
    fn camp_rest_opens_the_rest_screen_and_never_resets_cast_count() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        let before = h.roster.members[0].magic.clone();
        h.input.push_all(&[char_key(b'r')]); // Rest
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Rest(_)))
        });
        h.input.push_all(&[char_key(b'r')]); // Rest — commit the (zero) time
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        assert_eq!(
            h.roster.members[0].magic, before,
            "rest is not a spell-slot restoration (FD-25)"
        );
    }

    fn feed_and_settle(shell: &mut Shell, h: &mut Harness, key: u8) {
        h.input.push_all(&[char_key(key)]);
        for _ in 0..4 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
    }

    #[test]
    fn camp_save_opens_saveload_and_emits_a_save_request() {
        use crate::saveload::SaveLoadRequest;
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'e')]); // Encamp
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b's')]); // Save → SaveLoad
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SaveLoad(_)))
        });
        feed_and_settle(&mut shell, &mut h, b's'); // choose Save action
        h.input.push_all(&[char_key(b'a')]); // pick slot A
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        assert_eq!(h.io_request, Some(SaveLoadRequest::Save('A')));
    }

    #[test]
    fn saveload_load_emits_load_for_restrike_and_import_for_original() {
        use crate::saveload::{SaveLoadRequest, SlotStatus};
        let (mut shell, mut h) = boot_with_party();
        h.slots.set('B', SlotStatus::RestrikeSave);
        h.slots.set('C', SlotStatus::OriginalSave);

        h.input.push_all(&[char_key(b'e')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::Camp(_)))
        });
        h.input.push_all(&[char_key(b's')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SaveLoad(_)))
        });
        feed_and_settle(&mut shell, &mut h, b'l'); // Load action
                                                   // A restrike slot → Load; an original slot → ImportOriginal.
        feed_and_settle(&mut shell, &mut h, b'b');
        assert_eq!(h.io_request, Some(SaveLoadRequest::Load('B')));
        // Emitting returned us to camp (ReturnTo::Camp).
        assert!(matches!(shell, Shell::Screen(Screen::Camp(_))));

        // Re-open save/load from camp and test the original-import path.
        h.io_request = None;
        h.input.push_all(&[char_key(b's')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::SaveLoad(_)))
        });
        feed_and_settle(&mut shell, &mut h, b'l');
        feed_and_settle(&mut shell, &mut h, b'c');
        assert_eq!(h.io_request, Some(SaveLoadRequest::ImportOriginal('C')));
    }

    #[test]
    fn training_screen_levels_up_an_eligible_member() {
        use crate::screens::{ReturnTo, Training};
        let mut h = Harness::new();
        let mut fighter = test_char("Gareth");
        fighter.class_level = [0; 8];
        fighter.class_level[2] = 1; // fighter level 1
        fighter.exp = 3000; // > the 2001 needed for L1→L2
        fighter.hit_dice = 1;
        fighter.multiclass_level = 0;
        fighter.stats.con.current = 12; // a valid CON (no HP adjustment)
        fighter.money.gold = 2000;
        fighter.hit_point_max = 20;
        fighter.hit_point_current = 20;
        h.roster.members = vec![fighter];

        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
        shell = Shell::Screen(Screen::Training(Training::new(0, ReturnTo::World)));

        h.input.push_all(&[char_key(b't')]); // Train
        for _ in 0..4 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.roster.members[0].class_level[2], 2, "fighter leveled up");
        assert_eq!(h.roster.members[0].exp, 3000, "exp not consumed");

        h.input.push_all(&[char_key(b'e')]); // Exit → world menu
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });
    }

    #[test]
    fn shop_screen_buys_an_item_and_updates_money_and_weight() {
        use crate::screens::{Screen, Shop as ShopScreen};
        use crate::shop::{Shop, ShopItem};

        let mut h = Harness::new();
        let mut buyer = test_char("Rich");
        buyer.money.gold = 100;
        buyer.combat.weight = 0;
        h.roster.members = vec![buyer];
        h.state.selected_player = 0;

        let mut shell = Shell::boot(&mut h.machine, &mut h.state, &mut h.vm_memory);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::WorldMenu { .. })
        });

        let shop = Shop::new(vec![ShopItem::synthetic("Dagger", 2, 10)], 0x00);
        shell = Shell::Screen(Screen::Shop(ShopScreen::new(shop)));

        // Buy → enter the item list.
        h.input.push_all(&[char_key(b'b')]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        // Select the first (highlighted) item.
        h.input.push_all(&[crate::input::InputEvent::Enter]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.roster.members[0].items.len(), 1, "item bought");
        assert_eq!(h.roster.members[0].combat.weight, 10, "encumbrance updated");
        assert_eq!(
            crate::money::gold_worth(&h.roster.members[0].money, &h.rules),
            98,
            "paid 2 gp"
        );
    }

    #[test]
    fn party_view_scrolls_between_members() {
        let (mut shell, mut h) = boot_with_party();
        h.input.push_all(&[char_key(b'v')]);
        tick_until(&mut shell, &mut h, 10, |s| {
            matches!(s, Shell::Screen(Screen::PartyView(_)))
        });
        assert_eq!(h.state.selected_player, 0);
        // Down (ctrl 'P') advances to the next member.
        h.input
            .push_all(&[crate::input::InputEvent::Ext(crate::input::ExtKey::Down)]);
        for _ in 0..3 {
            let mut ctx = h.ctx();
            shell.tick(&mut ctx);
        }
        assert_eq!(h.state.selected_player, 1);
    }
}
