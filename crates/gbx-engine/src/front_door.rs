//! ★ **The front door** (roll-credits slice 9b): everything `seg001.PROGRAM`
//! does before the game starts — the TITLE.DAX sequence, the "Play Demo"
//! prompt, the copy-protection challenge, and `startGameMenu`.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/seg001.cs:109-177` — `PROGRAM`'s own order: `title_screen()`,
//!   then a 30-second `displayInput` defaulting to `'D'`, then
//!   `copy_protection()` (unless the demo was chosen), then the
//!   `while (true)` loop whose body is `startGameMenu()` + `sub_29758()` +
//!   `InitAgain()`.
//! - `engine/ovr002.cs:8-97` — `title_screen`/`credits`/`delay_or_key`.
//! - `engine/ovr004.cs:16-111` — `copy_protection` (the wheel itself is
//!   [`crate::copy_wheel`]).
//! - `engine/ovr018.cs:24-306` — `startGameMenu`, its twelve menu strings and
//!   the conditional flag table.
//!
//! **Shape.** The original is four blocking loops in a row; this is one
//! [`Shell::FrontDoor`](crate::shell::Shell) state advancing a stage per tick
//! (D-UI1). `Begin` hands the party to `BootFlow` — the same entry the
//! `--slot` shortcut takes — so nothing downstream can tell which door the
//! session came through, which is exactly what the digest-compare acceptance
//! checks.

use crate::copy_wheel::{Challenge, DETHEK_BASE};
use crate::draw::{blit_image, Clip};
use crate::framebuffer::Framebuffer;
use crate::input::TICK_HZ;
use crate::shell::FlowCtx;
use crate::widgets::{Hotbar, PressAnyKey, TextEntry, Widget, WidgetOutcome};
use gbx_formats::game_data::GameData;
use gbx_formats::image::{self, ImageBlock};

/// `delay_or_key(5)` after the first title screen (`ovr002.cs:76`).
const TITLE_DELAY_SHORT: u32 = 5 * TICK_HZ;
/// `delay_or_key(10)` after each of the remaining three (`:83,90,94`).
const TITLE_DELAY_LONG: u32 = 10 * TICK_HZ;
/// `gbl.displayInputSecondsToWait = 30` at the first Play-Demo prompt
/// (`seg001.cs:114`).
const DEMO_PROMPT_SECONDS: u32 = 30;
/// …and 10 at every prompt after a demo (`seg001.cs:159`).
const DEMO_PROMPT_SECONDS_AGAIN: u32 = 10;
/// `gbl.displayInputTimeoutValue = 'D'` (`seg001.cs:115`) — an unattended
/// title screen plays the demo.
const DEMO_TIMEOUT_KEY: u8 = b'D';
/// `displayInput(..., "Play Demo", "Curse of the Azure Bonds v1.3 ")`
/// (`seg001.cs:117`). The trailing space is the original's own: it is what
/// separates the version banner from the menu word at
/// `displayInputXOffset = displayExtraString.Length`.
const DEMO_PROMPT_BANNER: &str = "Curse of the Azure Bonds v1.3 ";
const DEMO_PROMPT_WORDS: &str = "Play Demo";
/// `seg044.PlaySound(Sound.sound_d)` before the fourth title block
/// (`ovr002.cs:87`).
const TITLE_SOUND: u8 = 0x0D;

/// The four `TITLE.DAX` blocks and where `title_screen` puts them
/// (`ovr002.cs:73-90`): `(block, cell row, cell col)`. `draw_picture`'s
/// `(rowY, colX)` are cell coordinates — `minY = rowY * 8`, `minX = colX * 8`
/// (`seg040.cs:76-80`).
const TITLE_BLOCKS: [(u8, usize, usize); 4] = [
    (1, 0, 0),    // SSI / AD&D product screen, 320×200
    (2, 0, 0),    // the Alias cover art, 320×200
    (3, 0x0B, 6), // "CURSE of the AZURE BONDS", 240×112, over the art
    (4, 0x0B, 0), // "A FORGOTTEN REALMS Fantasy Role-Playing Epic Vol. II"
];

/// `ovr002.credits` (`ovr002.cs:24-66`), transcribed as its own table:
/// `(text, color, row, col)` in the original's own draw order. The strings are
/// lower-case in the original too — the 8×8 font has no lower case, so they
/// render as capitals either way ([`crate::text::draw_char`] uppercases).
const CREDITS: [(&str, u8, usize, usize); 34] = [
    ("based on the tsr novel 'azure bonds'", 10, 1, 2),
    ("by:", 10, 2, 6),
    ("kate novak", 11, 2, 9),
    ("and", 10, 2, 0x14),
    ("jeff grubb", 11, 2, 0x18),
    ("scenario created by:", 10, 4, 0x0A),
    ("tsr, inc.", 0x0E, 5, 0x0B),
    ("and", 0x0A, 5, 0x15),
    ("ssi", 0x0E, 5, 0x19),
    ("jeff grubb", 0x0B, 6, 0x0E),
    ("george mac donald", 0x0B, 7, 0x0B),
    ("game created by:", 0x0A, 9, 1),
    ("ssi special projects", 0x0E, 9, 0x12),
    ("project leader:", 0x0E, 0x0B, 2),
    ("george mac donald", 0x0B, 0x0C, 2),
    ("programming:", 0x0E, 0x0E, 2),
    ("scot bayless", 0x0B, 0x0F, 2),
    ("russ brown", 0x0B, 0x10, 2),
    ("michael mancuso", 0x0B, 0x11, 2),
    ("development:", 0x0E, 0x13, 2),
    ("david shelley", 0x0B, 0x14, 2),
    ("michael mancuso", 0x0B, 0x15, 2),
    ("oran kangas", 0x0B, 0x16, 2),
    ("graphic arts:", 0x0E, 0x0B, 0x16),
    ("tom wahl", 0x0B, 0x0C, 0x16),
    ("fred butts", 0x0B, 0x0D, 0x16),
    ("susan manley", 0x0B, 0x0E, 0x16),
    ("mark johnson", 0x0B, 0x0F, 0x16),
    ("cyrus lum", 0x0B, 0x10, 0x16),
    ("playtesting:", 0x0E, 0x12, 0x16),
    ("jim jennings", 0x0B, 0x13, 0x16),
    ("james kucera", 0x0B, 0x14, 0x16),
    ("rick white", 0x0B, 0x15, 0x16),
    ("robert daly", 0x0B, 0x16, 0x16),
];

// --- The title sequence (ovr002.title_screen) ---

/// Which beat of `title_screen` is on screen. The four art beats are indices
/// into [`TITLE_BLOCKS`]; `Credits` is `ClearScreen(); credits();`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum TitleStage {
    /// Block 1 alone, `delay_or_key(5)`.
    Logo,
    /// Blocks 2 **and** 3 — the original draws both with no delay between
    /// them (`ovr002.cs:78-83`), so the cover art and the logo are one beat.
    Cover,
    /// Block 4, preceded by `sound_d`.
    Banner,
    /// `credits()` on a cleared screen.
    Credits,
}

impl TitleStage {
    fn delay(self) -> u32 {
        match self {
            TitleStage::Logo => TITLE_DELAY_SHORT,
            _ => TITLE_DELAY_LONG,
        }
    }

    fn next(self) -> Option<TitleStage> {
        match self {
            TitleStage::Logo => Some(TitleStage::Cover),
            TitleStage::Cover => Some(TitleStage::Banner),
            TitleStage::Banner => Some(TitleStage::Credits),
            TitleStage::Credits => None,
        }
    }
}

/// `title_screen` (`ovr002.cs:69-97`) as a parked stage machine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TitleSequence {
    stage: TitleStage,
    /// `delay_or_key`'s remaining wall time, in ticks.
    ticks_left: u32,
    /// Whether this stage's art is already on the glass. Not serialized: a
    /// restored save simply repaints, which is cheaper to reason about than
    /// trusting a framebuffer we did not draw.
    #[serde(skip)]
    painted: bool,
}

impl Default for TitleSequence {
    fn default() -> Self {
        TitleSequence::new()
    }
}

impl TitleSequence {
    pub fn new() -> Self {
        TitleSequence {
            stage: TitleStage::Logo,
            ticks_left: TitleStage::Logo.delay(),
            painted: false,
        }
    }

    /// `Some(())` once the whole sequence (including the trailing
    /// `ClearScreen`) is done.
    fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        if !self.painted {
            self.paint(ctx);
            self.painted = true;
            // `delay_or_key` opens with `clear_keyboard()` (`ovr002.cs:10`),
            // so the key that skipped the previous beat cannot skip this one.
            ctx.input.clear();
        }
        // `while (KEYPRESSED() == false && DateTime.Now < timeEnd)`
        // (`ovr002.cs:14-18`) — any key ends the beat early.
        let keyed = ctx.input.read_key().is_some();
        self.ticks_left = self.ticks_left.saturating_sub(ctx.dt_ticks);
        if !keyed && self.ticks_left > 0 {
            return None;
        }
        ctx.input.clear(); // `delay_or_key`'s trailing `clear_keyboard()`
        match self.stage.next() {
            Some(next) => {
                self.stage = next;
                self.ticks_left = next.delay();
                self.painted = false;
                None
            }
            None => {
                // `seg041.ClearScreen()` (`ovr002.cs:96`).
                ctx.fb.clear(0);
                Some(())
            }
        }
    }

    fn paint(&self, ctx: &mut FlowCtx) {
        match self.stage {
            TitleStage::Logo => draw_title_block(ctx.fb, ctx.data, TITLE_BLOCKS[0]),
            TitleStage::Cover => {
                draw_title_block(ctx.fb, ctx.data, TITLE_BLOCKS[1]);
                draw_title_block(ctx.fb, ctx.data, TITLE_BLOCKS[2]);
            }
            TitleStage::Banner => {
                // `PlaySound(sound_d)` is issued between the load and the draw
                // (`ovr002.cs:85-89`).
                ctx.sounds.push(crate::shell::SoundEvent(TITLE_SOUND));
                draw_title_block(ctx.fb, ctx.data, TITLE_BLOCKS[3]);
            }
            TitleStage::Credits => {
                // `seg041.ClearScreen(); credits();` (`ovr002.cs:92-93`).
                ctx.fb.clear(0);
                let _ = crate::frames::draw8x8_02(ctx.fb, ctx.symbols);
                for (text, color, row, col) in CREDITS {
                    crate::text::draw_string(ctx.fb, ctx.font, text, row, col, 0, color);
                }
            }
        }
    }
}

/// `LoadDax(0, 0, block, "Title")` + `draw_picture(dax, rowY, colX, 0)`
/// (`ovr002.cs:73-89`): mask colour 0 with `masked == 0`, i.e. **no** mask, and
/// the full-canvas clip.
///
/// A missing/undecodable block draws nothing rather than failing the tick —
/// the same tolerance every other art site here has, and a data set without
/// `TITLE.DAX` is a fixture, not a player.
fn draw_title_block(fb: &mut Framebuffer, data: &GameData, (block, row, col): (u8, usize, usize)) {
    let Some(img) = load_title_block(data, block) else {
        return;
    };
    let Some(item) = img.items.first() else {
        return;
    };
    blit_image(
        fb,
        &item.pixels,
        img.width_px(),
        img.height as usize,
        row,
        col,
        Clip::FULL,
        None,
        None,
    );
}

fn load_title_block(data: &GameData, block: u8) -> Option<ImageBlock> {
    let bytes = data.block("TITLE.DAX", block).ok()?;
    image::decode(&bytes, None).ok()
}

// --- The copy-protection challenge (ovr004.copy_protection) ---

/// `ovr004.cs:27-29` — the three instruction lines, colour 10, rows 2-4 col 3.
const PROTECTION_LINES: [&str; 3] = [
    "Align the espruar and dethek runes",
    "shown below, on translation wheel",
    "like this:",
];
/// `getUserInputString(1, 0, 13, …)` (`ovr004.cs:87`).
const PROTECTION_PROMPT: &str = "type character and press return: ";
const PROTECTION_PROMPT_COLOR: u8 = 13;
/// `DisplayStatusText(0, 14, …)` (`ovr004.cs:94`).
const PROTECTION_WRONG: &str = "Sorry, that's incorrect.";
const PROTECTION_WRONG_COLOR: u8 = 14;
/// `ovr004.cs:107` — the third failure.
const PROTECTION_DOOM: &str = "An unseen force hurls you into the abyss!";
/// `SysDelay(0x3E8)` before `print_and_exit()` (`ovr004.cs:108`).
const PROTECTION_DOOM_TICKS: u32 = TICK_HZ;
/// `attempt < 3` (`ovr004.cs:100`).
const PROTECTION_MAX_ATTEMPTS: u8 = 3;
/// `DrawIsoTile(var_6, 3, 0x11)` → `OverlayUnbounded(set, 0, tile, 3, 0x11)` →
/// `draw_combat_picture(set, 4, 0x12, tile)` (`ovr034.cs`/`seg040.cs:22-25`):
/// the `-1`/`+1` round trip lands the espruar rune at cell (4, 0x12).
const ESPRUAR_CELL: (usize, usize) = (4, 0x12);
/// `DrawIsoTile(var_7 + 0x1A, 7, 0x11)` — the dethek rune, four rows down.
const DETHEK_CELL: (usize, usize) = (8, 0x12);

/// ★ **The copy-protection prompt** (`copy_protection`, `ovr004.cs:16-111`),
/// neutralized per **D-RC4**: the screen is the original's, and the answer is
/// pre-filled into the input line so `<Enter>` accepts it.
///
/// Everything else is transcribed as written — three attempts, a re-rolled
/// challenge after each miss, the "Sorry, that's incorrect." status line, and
/// the third failure's `print_and_exit`. The neutralization is *only* the
/// prefilled character (and [`CopyProtection::faithful`] takes even that away,
/// D4), because a player who owns the wheel should be able to backspace and
/// answer for themselves.
///
/// Used twice, exactly as the original uses `copy_protection()` twice: the
/// boot prompt ([`FrontDoor`]) and the `PROTECTION` opcode's one shipped site
/// (`ECL1#0x50 @0x9B6F`, the sixth journey's bridge keeper).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CopyProtection {
    challenge: Challenge,
    entry: TextEntry,
    /// `attempt` (`ovr004.cs:30,90`).
    attempts: u8,
    /// The status line under the prompt, once a miss has put one there.
    status: Option<String>,
    /// The third-failure path: `PlaySound`s, the message, `SysDelay(1000)`,
    /// `print_and_exit()`. Counts down before the session ends.
    doom_ticks: Option<u32>,
    /// D4: with the answer hidden, this is the original's own challenge.
    faithful: bool,
    #[serde(skip)]
    painted: bool,
}

/// How a [`CopyProtection`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionOutcome {
    /// The answer was right (`return`, `ovr004.cs:98`).
    Passed,
    /// Three misses: `print_and_exit()` (`:109`). The host is expected to end
    /// the session; the engine parks on the message.
    Ejected,
}

impl CopyProtection {
    /// Poses the first challenge — four PRNG draws, in the original's order.
    pub fn pose(rng: &mut crate::rng::EngineRng, faithful: bool) -> Self {
        let challenge = Challenge::pose(rng);
        CopyProtection {
            challenge,
            entry: Self::entry_for(&challenge, faithful),
            attempts: 0,
            status: None,
            doom_ticks: None,
            faithful,
            painted: false,
        }
    }

    /// The input line: `getUserInputString(1, …)` takes exactly one character,
    /// and D-RC4 puts the right one in it up front.
    fn entry_for(challenge: &Challenge, faithful: bool) -> TextEntry {
        let mut entry = TextEntry::new(PROTECTION_PROMPT, 1, false);
        if !faithful {
            entry.buf = vec![challenge.answer() as u8];
        }
        entry
    }

    /// The posed challenge (a test/inspection seam — and what the "answer
    /// shown" presentation is computed from).
    pub fn challenge(&self) -> Challenge {
        self.challenge
    }

    /// `Some(outcome)` once the prompt is finished with.
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<ProtectionOutcome> {
        if let Some(left) = &mut self.doom_ticks {
            // `SysDelay(0x3E8)` then `print_and_exit()` (`ovr004.cs:108-109`).
            *left = left.saturating_sub(ctx.dt_ticks);
            ctx.input.clear();
            return (*left == 0).then_some(ProtectionOutcome::Ejected);
        }
        if !self.painted {
            self.paint(ctx);
            self.painted = true;
            // ★ `Load24x24Set` ends with `seg043.clear_keyboard()`
            // (`ovr034.cs`), and `copy_protection` calls it twice before it
            // draws anything — so a key already in the buffer when the
            // challenge is posed does NOT answer it. Load-bearing here: the
            // bridge keeper's `PROTECTION` is reached with the player leaning
            // on Enter through two joke questions, and without this the
            // challenge would be answered on the tick it appeared.
            ctx.input.clear();
        }
        // The prompt row is repainted every tick: it carries the live edit
        // buffer, which is the one thing on this screen that moves.
        self.draw_prompt_row(ctx);
        match self.entry.tick(ctx.input) {
            WidgetOutcome::TextSubmitted(text) => {
                // `input_key = (input == null || input.Length == 0) ? ' ' :
                // input[0]` (`ovr004.cs:89`).
                let key = text.chars().next().unwrap_or(' ');
                self.attempts += 1;
                if key == self.challenge.answer() {
                    return Some(ProtectionOutcome::Passed);
                }
                if self.attempts >= PROTECTION_MAX_ATTEMPTS {
                    // `game_speed_var = 9` (`:106`) — the original speeds the
                    // text up on the way out; our pacer is host-set, so the
                    // message simply lands.
                    self.status = Some(PROTECTION_DOOM.to_string());
                    self.doom_ticks = Some(PROTECTION_DOOM_TICKS);
                    self.draw_status(ctx);
                    return None;
                }
                self.status = Some(PROTECTION_WRONG.to_string());
                // The `do` loop re-enters at the top: a fresh challenge, fresh
                // runes, fresh box and path (`ovr004.cs:32-71`).
                self.challenge = Challenge::pose(ctx.rng);
                self.entry = Self::entry_for(&self.challenge, self.faithful);
                self.painted = false;
                None
            }
            _ => None,
        }
    }

    /// The static half of the screen (`ovr004.cs:25-71`): the outer frame, the
    /// three instruction lines, the two runes, and the box/path sentence.
    fn paint(&self, ctx: &mut FlowCtx) {
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        for (i, line) in PROTECTION_LINES.iter().enumerate() {
            crate::text::draw_string(ctx.fb, ctx.font, line, 2 + i, 3, 0, 10);
        }
        draw_rune(ctx.fb, ctx.data, 1, self.challenge.espruar, ESPRUAR_CELL);
        draw_rune(ctx.fb, ctx.data, 2, self.challenge.dethek, DETHEK_CELL);
        // `"Type the character in box number " + (6 - code_row)`
        // (`ovr004.cs:65-67`).
        let text = format!(
            "Type the character in box number {}",
            self.challenge.box_number()
        );
        crate::text::draw_string(ctx.fb, ctx.font, &text, 12, 3, 0, 10);
        crate::text::draw_string(ctx.fb, ctx.font, "under the ", 13, 3, 0, 10);
        crate::text::draw_string(
            ctx.fb,
            ctx.font,
            self.challenge.path_string(),
            13,
            14,
            0,
            15,
        );
        crate::text::draw_string(ctx.fb, ctx.font, "path.", 13, 0x19, 0, 10);
    }

    /// `getUserInputString`'s own two-tone line (`seg041.cs:236-257`): the
    /// prompt at column 0 in colour 13, the typed characters in colour 15.
    fn draw_prompt_row(&self, ctx: &mut FlowCtx) {
        if self.status.is_some() {
            return; // a status line owns the row until the next redraw
        }
        crate::combat::scene::render::clear_prompt_line(ctx.fb);
        crate::text::draw_string(
            ctx.fb,
            ctx.font,
            PROTECTION_PROMPT,
            0x18,
            0,
            0,
            PROTECTION_PROMPT_COLOR,
        );
        crate::text::draw_string(
            ctx.fb,
            ctx.font,
            &String::from_utf8_lossy(&self.entry.buf),
            0x18,
            PROTECTION_PROMPT.len(),
            0,
            15,
        );
    }

    /// `DisplayStatusText(0, 14, …)` (`seg041.cs:305-310`).
    fn draw_status(&self, ctx: &mut FlowCtx) {
        let Some(status) = &self.status else {
            return;
        };
        crate::combat::scene::render::clear_prompt_line(ctx.fb);
        crate::text::draw_string(ctx.fb, ctx.font, status, 0x18, 0, 0, PROTECTION_WRONG_COLOR);
    }
}

/// `DrawIsoTile` over the 24×24 set `Load24x24Set` filled from `TILES.DAX`
/// (`ovr004.cs:22-23,38-39`). `block` is the `TILES.DAX` block (1 espruar,
/// 2 dethek) and `index` the rune's item index inside it — the `+0x1A` the
/// original applies is a slot offset into one shared 48-cell set, which we do
/// not need because we address the two blocks directly.
fn draw_rune(
    fb: &mut Framebuffer,
    data: &GameData,
    block: u8,
    index: u8,
    (row, col): (usize, usize),
) {
    debug_assert!(block == 1 || index < DETHEK_BASE + 22);
    let Ok(bytes) = data.block("TILES.DAX", block) else {
        return;
    };
    let Ok(img) = image::decode(&bytes, None) else {
        return;
    };
    let Some(item) = img.items.get(index as usize) else {
        return;
    };
    // `OverlayUnbounded` → `draw_combat_picture` → the overlay clip window
    // (`seg040.cs:22-25,115-118`).
    blit_image(
        fb,
        &item.pixels,
        img.width_px(),
        img.height as usize,
        row,
        col,
        Clip::OVERLAY,
        None,
        None,
    );
}

// --- The front door itself ---

/// Which beat of `seg001.PROGRAM`'s preamble the shell is parked on.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum FrontDoor {
    /// `ovr002.title_screen()`.
    Title(TitleSequence),
    /// The `"Play Demo"` prompt (`seg001.cs:114-125`).
    DemoPrompt { menu: Box<Widget> },
    /// ★ The attract mode stays deferred (roll-credits G10): choosing Demo (or
    /// letting the prompt time out into it) lands here — a loud, acknowledged
    /// refusal — and then goes back to the title, which is where the original's
    /// own demo loop returns too.
    DemoStub {
        gate: PressAnyKey,
        #[serde(skip)]
        painted: bool,
    },
    /// `ovr004.copy_protection()` (`seg001.cs:127-131`).
    Protection(Box<CopyProtection>),
    /// `ovr018.startGameMenu()` (`seg001.cs:147`).
    StartMenu(Box<StartMenu>),
}

/// What the front door wants the shell to do next.
pub enum FrontDoorTick {
    /// Still on the front door.
    Stay,
    /// `BEGIN Adventuring` — hand off to `BootFlow`.
    Begin,
    /// `Load Saved Game` — open the slot picker (which returns here).
    OpenLoad,
    /// `Save Current Game` — open the save-slot picker.
    OpenSave,
    /// `View Character` — the character sheet, returning here.
    OpenView,
    /// `Train Character` — the training hall, returning here.
    OpenTrain,
    /// ★ Slice 9c. `Create New Character` — `createPlayer`.
    OpenCreate,
    /// ★ Slice 9c. `Modify Character` — `modifyPlayer`, for the selected
    /// member.
    OpenModify,
    /// ★ Slice 9c. `Human Change Classes` — `DuelClass`.
    OpenDualClass,
    /// ★ Slice 9c. `Add Character to Party` — `AddPlayer`'s `.guy` picker.
    OpenAdd,
    /// `Exit to DOS` — the host is asked to quit (`print_and_exit`).
    Quit,
}

impl FrontDoor {
    /// The whole preamble, from the first title screen.
    pub fn new() -> Self {
        FrontDoor::Title(TitleSequence::new())
    }

    /// Parked straight on `startGameMenu` — where a load returns, and where
    /// the party-wipe recovery lands (`InitAgain` → `startGameMenu`,
    /// `seg001.cs:152,147`).
    pub fn start_menu() -> Self {
        FrontDoor::StartMenu(Box::default())
    }

    /// A one-word probe for `RESTRIKE_DEBUG_LOG`.
    pub fn probe(&self) -> &'static str {
        match self {
            FrontDoor::Title(_) => "front-door/title",
            FrontDoor::DemoPrompt { .. } => "front-door/play-demo",
            FrontDoor::DemoStub { .. } => "front-door/demo-deferred",
            FrontDoor::Protection(_) => "front-door/copy-protection",
            FrontDoor::StartMenu(_) => "front-door/start-menu",
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> FrontDoorTick {
        match self {
            FrontDoor::Title(seq) => {
                if seq.tick(ctx).is_some() {
                    *self = FrontDoor::demo_prompt(ctx, DEMO_PROMPT_SECONDS_AGAIN);
                }
                FrontDoorTick::Stay
            }
            FrontDoor::DemoPrompt { menu } => {
                // `displayInput`'s own prompt+menu line, drawn every tick so
                // the highlight follows `','`/`'.'`.
                if let Widget::Hotbar(h) = menu.as_ref() {
                    crate::combat::scene::render::clear_prompt_line(ctx.fb);
                    crate::text::draw_string(ctx.fb, ctx.font, DEMO_PROMPT_BANNER, 0x18, 0, 0, 13);
                    crate::combat::scene::render::draw_menu_line_at(
                        ctx.fb,
                        ctx.font,
                        &h.text,
                        h.selected_span(),
                        DEMO_PROMPT_BANNER.len(),
                    );
                }
                match menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::Hotbar(key) => {
                        if let Widget::Hotbar(h) = menu.as_ref() {
                            ctx.state.menu_selected_word = h.selected_word();
                        }
                        // `if (inputKey == 'D') gbl.inDemo = true;`
                        // (`seg001.cs:122-125`).
                        if key.to_ascii_uppercase() == DEMO_TIMEOUT_KEY {
                            *self = FrontDoor::DemoStub {
                                gate: PressAnyKey,
                                painted: false,
                            };
                        } else {
                            *self = FrontDoor::protection(ctx);
                        }
                        FrontDoorTick::Stay
                    }
                    _ => FrontDoorTick::Stay,
                }
            }
            FrontDoor::DemoStub { gate, painted } => {
                if !*painted {
                    ctx.fb.clear(0);
                    let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
                    for (i, line) in DEMO_STUB_TEXT.iter().enumerate() {
                        // `draw_char` drops columns past 40 in silence, so an
                        // over-long line loses its tail rather than complaining
                        // (the audit caught exactly that here). Loud in debug,
                        // pinned by `the_demo_stub_text_fits_inside_the_frame`.
                        debug_assert!(line.len() <= DEMO_STUB_WIDTH);
                        crate::text::draw_string(
                            ctx.fb,
                            ctx.font,
                            line,
                            8 + i,
                            DEMO_STUB_COL,
                            0,
                            15,
                        );
                    }
                    crate::combat::scene::render::draw_prompt(
                        ctx.fb,
                        ctx.font,
                        "Press any key to continue",
                    );
                    ctx.vm_memory
                        .transcript
                        .push(crate::vmhost::TranscriptEntry::Request(
                            "DEMO: the attract mode is deferred (roll-credits G10) — \
                             returning to the title"
                                .to_string(),
                        ));
                    *painted = true;
                    ctx.input.clear();
                }
                if matches!(gate.tick(ctx.input), WidgetOutcome::Done) {
                    ctx.input.clear();
                    *self = FrontDoor::Title(TitleSequence::new());
                }
                FrontDoorTick::Stay
            }
            FrontDoor::Protection(prot) => {
                match prot.tick(ctx) {
                    Some(ProtectionOutcome::Passed) => {
                        *self = FrontDoor::start_menu();
                        FrontDoorTick::Stay
                    }
                    // `print_and_exit()` (`ovr004.cs:109`) — the host quits.
                    Some(ProtectionOutcome::Ejected) => FrontDoorTick::Quit,
                    None => FrontDoorTick::Stay,
                }
            }
            FrontDoor::StartMenu(menu) => menu.tick(ctx),
        }
    }

    /// The first prompt is 30 seconds (`seg001.cs:114`); every later one — the
    /// prompt after a demo — is 10 (`:159`).
    pub fn first_demo_prompt(ctx: &mut FlowCtx) -> Self {
        FrontDoor::demo_prompt(ctx, DEMO_PROMPT_SECONDS)
    }

    fn demo_prompt(ctx: &mut FlowCtx, seconds: u32) -> Self {
        let mut hotbar = Hotbar::new(DEMO_PROMPT_WORDS);
        hotbar.seed_selected_word(ctx.state.menu_selected_word);
        hotbar.timeout = Some((seconds * TICK_HZ, DEMO_TIMEOUT_KEY));
        FrontDoor::DemoPrompt {
            menu: Box::new(Widget::Hotbar(hotbar)),
        }
    }

    fn protection(ctx: &mut FlowCtx) -> Self {
        FrontDoor::Protection(Box::new(CopyProtection::pose(
            ctx.rng,
            ctx.state.copy_protection_faithful,
        )))
    }
}

impl Default for FrontDoor {
    fn default() -> Self {
        FrontDoor::new()
    }
}

/// The deferred-demo screen's words — loud on purpose (the M6 forensics rule:
/// a deferral the player cannot see is a silent failure).
///
/// This is OUR screen, not a transcription, so its layout is ours to get
/// right: the lines start at column [`DEMO_STUB_COL`] inside
/// `DrawFrame_Outer`'s border, and [`DEMO_STUB_WIDTH`] is what fits before the
/// right edge. `draw_char` drops anything past column 40 silently, so a line
/// that overflows simply loses its tail — the test below is what keeps that
/// from happening again.
const DEMO_STUB_TEXT: [&str; 4] = [
    "The demo (attract mode) is deferred.",
    "ECL1 block 82 is out of scope for",
    "this milestone (roll-credits G10).",
    "Returning to the title screen.",
];
/// The stub text's left column, inside the outer frame.
const DEMO_STUB_COL: usize = 3;
/// Columns available from [`DEMO_STUB_COL`] to the frame's right edge
/// inclusive (`DrawFrame_Outer` owns columns 0 and 0x27 and clears 1..=0x26).
const DEMO_STUB_WIDTH: usize = 0x26 - DEMO_STUB_COL + 1;

// --- startGameMenu (ovr018.cs:69-306) ---

/// `ovr018.menuStrings` (`ovr018.cs:24-37`) paired with `menuFlags`' own index
/// constants (`:40-51`). The first character is the hotkey: the menu paints it
/// in colour 15 at column 2 and the rest of the word in colour 10 at column 3
/// (`:121-124`).
const MENU_STRINGS: [&str; 12] = [
    "Create New Character",
    "Drop Character",
    "Modify Character",
    "Train Character",
    "Human Change Classes",
    "View Character",
    "Add Character to Party",
    "Remove Character from Party",
    "Load Saved Game",
    "Save Current Game",
    "BEGIN Adventuring",
    "Exit to DOS",
];

const ALLOW_CREATE: usize = 0;
const ALLOW_DROP: usize = 1;
const ALLOW_MODIFY: usize = 2;
const ALLOW_TRAINING: usize = 3;
const ALLOW_DUELCLASS: usize = 4;
const ALLOW_VIEW: usize = 5;
const ALLOW_ADD: usize = 6;
const ALLOW_REMOVE: usize = 7;
const ALLOW_LOAD: usize = 8;
const ALLOW_SAVE: usize = 9;
const ALLOW_BEGIN: usize = 10;
const ALLOW_EXIT: usize = 11;

/// The menu's own `displayInput` (`ovr018.cs:134`) — note the colours:
/// `MenuColorSet(0, 0, 13)` is highlight 0, foreground 0, prompt 13, so
/// `display_highlighed_text` draws every one of these letters in colour **0**.
/// The key string is real (it is what `displayInput` accepts) and completely
/// **invisible**; the visible menu is the vertical list at rows 12+.
const MENU_KEYS: &str = "C D M T H V A R L S B E J";
const MENU_PROMPT: &str = "Choose a function ";
/// The first row of the vertical list (`yCol + 12`, `ovr018.cs:121`).
const MENU_FIRST_ROW: usize = 12;

/// `ovr018.startGameMenu`'s flag table (`:80-114`), computed fresh on every
/// `reclac_menus` pass.
///
/// The party-less column is the one the party wipe lands in
/// (`seg001.InitAgain` clears `TeamList` before re-entering the menu): only
/// Create, Add, **Load** and Exit — `BEGIN Adventuring` and `Save` are gone,
/// which is why a dead party's single road back into the game is a load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MenuFlags([bool; 12]);

impl MenuFlags {
    /// `reclac_menus`' recomputation (`ovr018.cs:80-114`). `training` is
    /// `area2_ptr.training_class_mask > 0 || Cheats.free_training`, and
    /// `duel_class` the selected member's `CanDuelClass()`.
    pub fn compute(has_party: bool, training: bool, duel_class: bool) -> Self {
        let mut f = [false; 12];
        // `menuFlags`' three statically-true entries (`:54-67`) are never
        // recomputed by the loop — they are true in both columns.
        f[ALLOW_CREATE] = true;
        f[ALLOW_ADD] = true;
        f[ALLOW_EXIT] = true;
        if has_party {
            f[ALLOW_DROP] = true;
            f[ALLOW_MODIFY] = true;
            f[ALLOW_TRAINING] = training;
            f[ALLOW_DUELCLASS] = training && duel_class;
            f[ALLOW_VIEW] = true;
            f[ALLOW_REMOVE] = true;
            f[ALLOW_LOAD] = false;
            f[ALLOW_SAVE] = true;
            f[ALLOW_BEGIN] = true;
        } else {
            f[ALLOW_LOAD] = true;
        }
        MenuFlags(f)
    }

    pub fn allows(&self, index: usize) -> bool {
        self.0.get(index).copied().unwrap_or(false)
    }

    /// The visible entries, in table order — what the screen actually lists.
    pub fn visible(&self) -> Vec<(usize, &'static str)> {
        (0..12)
            .filter(|&i| self.0[i])
            .map(|i| (i, MENU_STRINGS[i]))
            .collect()
    }
}

/// ★ `ovr018.startGameMenu` (`:69-306`) — the front door's last room.
///
/// Every verb but Create and Modify is wired to machinery that already exists;
/// those two present per the flag table and report slice 9c loudly rather than
/// pretending (roll-credits §13).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StartMenu {
    menu: Widget,
    /// A one-line report under the list (a refusal, or what a verb did).
    status: Option<String>,
}

impl Default for StartMenu {
    fn default() -> Self {
        StartMenu::new()
    }
}

impl StartMenu {
    pub fn new() -> Self {
        let mut hotbar = Hotbar::new(MENU_KEYS);
        // `displayInput(out controlKey, false, 1, …)` — `accept_ctrlkeys = 1`,
        // and the control keys `'G'`/`'O'` scroll the party panel
        // (`ovr018.cs:138-151` → `scroll_team_list`, `ovr020.cs:1349-1369`).
        hotbar.accept_ext = true;
        hotbar.ext_scrolls_party = true;
        StartMenu {
            menu: Widget::Hotbar(hotbar),
            status: None,
        }
    }

    /// The live flag table for this engine state.
    pub fn flags(ctx: &FlowCtx) -> MenuFlags {
        // `gbl.SelectedPlayer != null` — our roster's emptiness is the same
        // question (`selectAPlayer` cannot leave a non-empty TeamList with a
        // null selection).
        let has_party = !ctx.roster.members.is_empty();
        // `area2_ptr.training_class_mask > 0` (`ovr018.cs:86`): the town ECL
        // writes it at a training hall. Nothing else in the menu reads it.
        let training = crate::vmhost::training_class_mask(ctx.vm_memory) > 0;
        // ★ Slice 9c: `gbl.SelectedPlayer.CanDuelClass()` (`ovr018.cs:95`) —
        // live, per selected member, which is why `'G'`/`'O'` recompute the
        // flag table when the answer changes (`:142-149`).
        let duel_class = selected_index(ctx)
            .map(|i| crate::modify_screen::can_dual_class(&ctx.roster.members[i]))
            .unwrap_or(false);
        MenuFlags::compute(has_party, training, duel_class)
    }

    fn render(&self, ctx: &mut FlowCtx, flags: MenuFlags) {
        // `reclac_menus`' own repaint (`ovr018.cs:79-82`): the outer frame and
        // the party panel, then the list.
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        let rows: Vec<_> = ctx
            .roster
            .members
            .iter()
            .map(crate::charsheet::sheet_view)
            .collect();
        if !rows.is_empty() {
            let selected = Some((ctx.state.selected_player as usize) % rows.len());
            crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, selected);
        }
        for (row, (_, text)) in flags.visible().iter().enumerate() {
            let (head, tail) = text.split_at(1);
            // `displayString(menuStrings[i][0], 0, 15, yCol + 12, 2)` then the
            // remainder at colour 10, column 3 (`ovr018.cs:121-124`).
            crate::text::draw_string(ctx.fb, ctx.font, head, MENU_FIRST_ROW + row, 2, 0, 15);
            crate::text::draw_string(ctx.fb, ctx.font, tail, MENU_FIRST_ROW + row, 3, 0, 10);
        }
        if let Some(status) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, status, 0x16, 2, 0, 14);
        }
        // The prompt row: `displayExtraString` in colour 13 and then the key
        // string in colour 0 — i.e. invisible, exactly as the original's
        // `MenuColorSet(0, 0, 13)` makes it (`ovr018.cs:134`).
        crate::combat::scene::render::clear_prompt_line(ctx.fb);
        crate::text::draw_string(ctx.fb, ctx.font, MENU_PROMPT, 0x18, 0, 0, 13);
    }

    fn tick(&mut self, ctx: &mut FlowCtx) -> FrontDoorTick {
        let flags = Self::flags(ctx);
        self.render(ctx, flags);
        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::PartyScroll(code) => {
                // `scroll_team_list` (`ovr020.cs:1349-1369`): `'O'` next,
                // `'G'` previous, wrapping.
                let count = ctx.roster.members.len();
                if count > 0 {
                    let cur = ctx.state.selected_player as usize % count;
                    let next = match code {
                        b'O' | b'P' => (cur + 1) % count,
                        b'G' | b'H' => (cur + count - 1) % count,
                        _ => cur,
                    };
                    ctx.state.selected_player = next as u8;
                }
                FrontDoorTick::Stay
            }
            WidgetOutcome::Hotbar(key) => self.dispatch(key.to_ascii_uppercase(), ctx, flags),
            _ => FrontDoorTick::Stay,
        }
    }

    /// `startGameMenu`'s `switch (inputkey)` (`ovr018.cs:159-301`) — every arm
    /// gated on its own flag, exactly as written.
    fn dispatch(&mut self, key: u8, ctx: &mut FlowCtx, flags: MenuFlags) -> FrontDoorTick {
        self.status = None;
        match key {
            b'C' if flags.allows(ALLOW_CREATE) => FrontDoorTick::OpenCreate,
            b'M' if flags.allows(ALLOW_MODIFY) => {
                // ★ `modifyPlayer`'s own gate is checked here so the refusal
                // reads on the menu the player is standing on
                // (`ovr018.cs:1004-1013` prints it and returns immediately).
                let index = selected_index(ctx);
                match index.map(|i| &ctx.roster.members[i]) {
                    Some(ch) if !crate::modify_screen::can_modify(ch) => {
                        self.status = Some(format!("{} can't be modified.", ch.name));
                        FrontDoorTick::Stay
                    }
                    Some(_) => FrontDoorTick::OpenModify,
                    None => FrontDoorTick::Stay,
                }
            }
            b'A' if flags.allows(ALLOW_ADD) => FrontDoorTick::OpenAdd,
            b'D' if flags.allows(ALLOW_DROP) => {
                self.drop_selected(ctx, true);
                FrontDoorTick::Stay
            }
            b'R' if flags.allows(ALLOW_REMOVE) => {
                // `ovr018.cs:207-221`: a non-NPC member is SAVED and freed;
                // an NPC (`control_morale >= NPC_Base`) is dropped outright.
                self.drop_selected(ctx, false);
                FrontDoorTick::Stay
            }
            b'V' if flags.allows(ALLOW_VIEW) => FrontDoorTick::OpenView,
            b'T' if flags.allows(ALLOW_TRAINING) => FrontDoorTick::OpenTrain,
            b'H' if flags.allows(ALLOW_DUELCLASS) => FrontDoorTick::OpenDualClass,
            b'L' if flags.allows(ALLOW_LOAD) => FrontDoorTick::OpenLoad,
            // `case 'S'`: `menuFlags[allow_save] && gbl.TeamList.Count > 0`
            // (`ovr018.cs:230-236`).
            b'S' if flags.allows(ALLOW_SAVE) && !ctx.roster.members.is_empty() => {
                FrontDoorTick::OpenSave
            }
            b'B' if flags.allows(ALLOW_BEGIN) => self.begin(ctx),
            b'E' if flags.allows(ALLOW_EXIT) => FrontDoorTick::Quit,
            // A disabled verb, or a key outside the table: the original's
            // `switch` simply falls through and re-prompts. Ours says so — a
            // silent refusal is the bug class M6 spent a day on.
            0 => FrontDoorTick::Stay,
            other => {
                self.status = Some(refusal(other, flags));
                FrontDoorTick::Stay
            }
        }
    }

    /// `BEGIN Adventuring` (`ovr018.cs:239-274`).
    ///
    /// The guard is `field_3FA == 0` — a cell written in exactly one place,
    /// `CMD_Program`'s "you won" arm (`ovr003.cs:1953`), which sets it to
    /// `0xFF`. So BEGIN is refused only after the game has been won, when the
    /// menu's job is the post-victory save prompt instead.
    ///
    /// The recompose that follows is the original's own
    /// (`reload_ecl_and_pictures == false && lastDaxBlockId != 0x50` picks
    /// between a full frame+roster repaint and the `LastEclBlockId == 0`
    /// fallback) — but every path through it ends in the same three things for
    /// us: the exploration frame, the roster, and a cleared prompt row. The
    /// boot flow the caller starts repaints the viewport itself.
    fn begin(&mut self, ctx: &mut FlowCtx) -> FrontDoorTick {
        if crate::vmhost::game_won_flag(ctx.vm_memory) != 0 {
            self.status = Some("The game is over. Save, or Exit to DOS.".to_string());
            return FrontDoorTick::Stay;
        }
        // `gbl.area2_ptr.training_class_mask = 0` (`ovr018.cs:269`): leaving
        // the menu spends the training permission the town script granted.
        crate::vmhost::clear_training_class_mask(ctx.vm_memory);
        FrontDoorTick::Begin
    }

    /// `dropPlayer`/the `'R'` arm's `FreeCurrentPlayer` (`ovr018.cs:207-221`)
    /// — both remove the selected member from the roster; Drop discards the
    /// character, Remove **saves it out first**.
    ///
    /// ★ Slice 9c closes §14.8's residual: Remove now emits the
    /// `SavePlayer(string.Empty, player)` the original makes (`:213`), which
    /// the host turns into the `<name>.guy`/`.swg`/`.fx` trio. An NPC is
    /// dropped outright instead (`:216-219`), never written.
    fn drop_selected(&mut self, ctx: &mut FlowCtx, discard: bool) {
        let count = ctx.roster.members.len();
        if count == 0 {
            return;
        }
        let index = (ctx.state.selected_player as usize).min(count - 1);
        let name = ctx.roster.members[index].name.clone();
        let is_npc = ctx.roster.members[index].is_npc();
        if !discard && !is_npc {
            *ctx.char_io_request = Some(crate::chr_file::CharFileRequest::Save(Box::new(
                ctx.roster.members[index].clone(),
            )));
        }
        ctx.roster.members.remove(index);
        ctx.state.selected_player = ctx
            .state
            .selected_player
            .min(ctx.roster.members.len().saturating_sub(1) as u8);
        // `area2_ptr.party_size` is the count of real (non-NPC) members
        // (`Area2.field_67C`), which is what the roster length tracks here.
        ctx.state.party_size = ctx.roster.members.len() as u8;
        self.status = Some(if discard || is_npc {
            format!("{name} dropped.")
        } else {
            format!("{name} removed and saved.")
        });
    }
}

/// `gbl.SelectedPlayer`'s roster index, clamped — `None` for an empty party.
fn selected_index(ctx: &FlowCtx) -> Option<usize> {
    let count = ctx.roster.members.len();
    (count > 0).then(|| (ctx.state.selected_player as usize).min(count - 1))
}

/// The refusal line for a key the flag table does not currently allow — named,
/// not silent.
fn refusal(key: u8, flags: MenuFlags) -> String {
    let named = MENU_STRINGS
        .iter()
        .enumerate()
        .find(|(_, s)| s.as_bytes()[0] == key);
    match named {
        Some((i, name)) if !flags.allows(i) => format!("{name}: not available now."),
        _ => format!("'{}' is not a choice here.", key as char),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The party-less column is the wipe-recovery column: Create, Add, Load,
    /// Exit and nothing else (`ovr018.cs:103-114`).
    #[test]
    fn a_partyless_menu_offers_exactly_create_add_load_and_exit() {
        let flags = MenuFlags::compute(false, false, false);
        let visible: Vec<&str> = flags.visible().iter().map(|(_, s)| *s).collect();
        assert_eq!(
            visible,
            vec![
                "Create New Character",
                "Add Character to Party",
                "Load Saved Game",
                "Exit to DOS",
            ]
        );
        assert!(!flags.allows(ALLOW_BEGIN), "BEGIN is gone with no party");
        assert!(!flags.allows(ALLOW_SAVE), "so is Save");
        assert!(flags.allows(ALLOW_LOAD), "Load is the only road back");
    }

    /// With a party, Load disappears and everything party-shaped arrives
    /// (`ovr018.cs:80-102`).
    #[test]
    fn a_party_hides_load_and_shows_begin() {
        let flags = MenuFlags::compute(true, false, false);
        assert!(!flags.allows(ALLOW_LOAD), "Load is FALSE with a party");
        assert!(flags.allows(ALLOW_BEGIN));
        assert!(flags.allows(ALLOW_SAVE));
        assert!(flags.allows(ALLOW_VIEW));
        assert!(flags.allows(ALLOW_DROP));
        assert!(flags.allows(ALLOW_MODIFY));
        assert!(flags.allows(ALLOW_REMOVE));
        assert!(!flags.allows(ALLOW_TRAINING), "no training hall granted it");
        assert!(!flags.allows(ALLOW_DUELCLASS));
    }

    /// Training and dual-class ride the same `training_class_mask` gate, and
    /// dual-class additionally needs the member to qualify (`:86-95`).
    #[test]
    fn training_gates_both_train_and_dual_class() {
        let no_mask = MenuFlags::compute(true, false, true);
        assert!(!no_mask.allows(ALLOW_TRAINING));
        assert!(!no_mask.allows(ALLOW_DUELCLASS));
        let mask_only = MenuFlags::compute(true, true, false);
        assert!(mask_only.allows(ALLOW_TRAINING));
        assert!(!mask_only.allows(ALLOW_DUELCLASS));
        let both = MenuFlags::compute(true, true, true);
        assert!(both.allows(ALLOW_TRAINING));
        assert!(both.allows(ALLOW_DUELCLASS));
    }

    /// The title sequence is four beats and 35 seconds of unattended wall
    /// time (`5 + 10 + 10 + 10`, `ovr002.cs:76-94`).
    #[test]
    fn the_title_sequence_is_four_beats() {
        let mut stage = TitleStage::Logo;
        let mut beats = 1;
        let mut total = stage.delay();
        while let Some(next) = stage.next() {
            stage = next;
            beats += 1;
            total += stage.delay();
        }
        assert_eq!(beats, 4);
        assert_eq!(total, 35 * TICK_HZ);
    }

    /// Our own deferral screen has to fit inside its own frame: `draw_char`
    /// drops columns past 40 without a word, so an over-long line loses its
    /// tail and the message stops making sense ("...IS NOT IMPLE").
    #[test]
    fn the_demo_stub_text_fits_inside_the_frame() {
        for line in DEMO_STUB_TEXT {
            assert!(
                line.len() <= DEMO_STUB_WIDTH,
                "{line:?} is {} columns, {DEMO_STUB_WIDTH} available",
                line.len()
            );
        }
    }

    /// The credits table is the original's, in its own draw order.
    #[test]
    fn the_credits_name_everyone_the_original_names() {
        assert_eq!(CREDITS.len(), 34);
        assert_eq!(CREDITS[0].0, "based on the tsr novel 'azure bonds'");
        assert_eq!(CREDITS[33].0, "robert daly");
        for (text, _, row, col) in CREDITS {
            assert!(row < 25 && col + text.len() <= 40, "{text} fits the screen");
        }
    }
}
