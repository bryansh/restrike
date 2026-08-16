//! ★ **The ending** (roll-credits slice 9d, G9): `ovr019.end_game_text` and the
//! firework display it finishes on — everything `PROGRAM 8` does before the
//! start menu opens for the last time.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr003.cs:1951-1974` — `CMD_Program`'s `var_1 == 8` arm: the
//!   sequence below, then the win latch, the training mask, healing the
//!   survivors, `startGameMenu()`, the save prompt and `print_and_exit()`.
//! - `engine/ovr019.cs:474-538` — `end_game_text`, the twenty-four lines of
//!   prose and the six art beats they are interleaved with.
//! - `engine/ovr019.cs:411-444` — `ShowAnimation` (driven from
//!   [`crate::picture::show_animation_tick`]).
//! - `engine/ovr019.cs:25-30,69-159,162-274,277-353,356-408` — the fireworks:
//!   `sub_52068` (the palette flash), `sub_520B8` (seed the particles),
//!   `sub_524F7` (one burst frame), `sub_5279B` (run the burst),
//!   `endgame_5285E` (the rocket) and `endgame_529F4` (the loop itself).
//! - `engine/seg041.cs:125-231` — `press_any_key`, which is [`crate::text`].
//!
//! **Shape.** The original is a straight line of blocking calls; this is one
//! [`Shell::Ending`](crate::shell::Shell) state walking a static [`SCRIPT`]
//! table a step at a time (D-UI1 — the engine never reads a clock). The
//! sequence is pure presentation: it makes no rules decisions, changes no
//! character record, and the only state it touches outside the framebuffer is
//! the picture layer, the `picture_fade` cell it arms and disarms, and the
//! PRNG the fireworks draw from.
//!
//! **Draw parity.** No capture reaches here. Every `.gbxtrace` is a combat
//! capture taken from an imported save via `--slot A` (§14.5), replayed by
//! `gbx-oracle::replay` without a shell at all; `PROGRAM 8` is reachable only
//! from `ECL6#67 @0x93E7`, on the far side of the game's last fight. The
//! frontier guard (16/16) and the reel smoke (16/16) are the standing
//! referees.

use crate::framebuffer::{Framebuffer, HEIGHT, WIDTH};
use crate::picture::PICTURE_FADE_ADDR;
use crate::rng::EngineRng;
use crate::shell::FlowCtx;
use crate::text::{JobStatus, TextJob, NORMAL_BOTTOM};
use crate::widgets::{PressAnyKey, WidgetOutcome};

/// `press_any_key(text, …, 10, TextRegion.NormalBottom)` — the colour every
/// line of the ending prints in (`ovr019.cs:479-533`).
const PROSE_COLOR: u8 = 10;
/// `DisplayAndPause("Press any key to continue.", 13)` (`:483`, `:492`, `:502`,
/// `:524`). Note the full stop — the death screen's own prompt has none, and
/// the two really do differ in the original.
const PAUSE_PROMPT: &str = "Press any key to continue.";
const PAUSE_PROMPT_COLOR: u8 = 13;

/// One beat of `end_game_text`, in the original's own order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// `press_any_key(text, clear, 10, TextRegion.NormalBottom)` — `clear` is
    /// the original's `clearArea`, true only on the first line of a group.
    Say { text: &'static str, clear: bool },
    /// `DisplayAndPause("Press any key to continue.", 13)`.
    Pause,
    /// `ovr027.ClearPromptArea()`.
    ClearPrompt,
    /// `ShowAnimation(loops, block, 3, 3)`; `fade` brackets it with
    /// `picture_fade = 1` / `picture_fade = 0` (`:510-514`).
    Animate { block: u8, loops: u16, fade: bool },
    /// `head_body(head, body)` + `draw_head_and_body(true, 3, 3)` (`:516-517`).
    HeadBody { head: u8, body: u8 },
    /// `load_bigpic(block)` + `draw_bigpic()` (`:526-528`).
    BigPic { block: u8 },
    /// `endgame_529F4()` (`:534`) — fireworks until a key.
    Fireworks,
}

/// `PIC6` block 74 — the gauntlet shattering the Pool of Radiance.
const PIC_POOL: u8 = 0x4A;
/// `PIC6` block 75 — Tyranthraxus crumbling into nothingness.
const PIC_CRUMBLE: u8 = 0x4B;
/// `PIC6` block 77 — the bond fading. A **one-frame** block, which is why the
/// fade-armed loop over it is a pure dissolve.
const PIC_BOND: u8 = 0x4D;
/// `HEAD6`/`BODY6` block 65 — the Knights of Myth Drannor.
const KNIGHTS_HEAD: u8 = 0x41;
/// `BIGPIC6` block 122 — Shadowdale, the only block that file holds.
const SHADOWDALE_BIGPIC: u8 = 0x7A;
/// `(10 - game_speed_var) * 2` at `InitFirst`'s default speed of 4
/// (`seg001.cs:274`); recomputed from the live cell at entry.
const FADE_LOOPS_AT_DEFAULT_SPEED: u16 = 12;

/// ★ `end_game_text` (`ovr019.cs:474-538`), transcribed line for line. The
/// strings are the original's own, including the double space in "this  foul
/// place" and the unclosed quote in "The Knights of Myth Drannor rush in, '".
const SCRIPT: &[Step] = &[
    // `:479-483`
    Step::Say {
        text: "Tyranthraxus' spirit coalesces over the slain ",
        clear: true,
    },
    Step::Say {
        text: "storm giant. 'You have defeated me. Were it not for ",
        clear: false,
    },
    Step::Say {
        text: "the Amulet of Lythander, I could possess you and rob ",
        clear: false,
    },
    Step::Say {
        text: "you of your victory. Still I can escape through the pool.",
        clear: false,
    },
    Step::Pause,
    // `:485-493`
    Step::Say {
        text: "As you reach for the Pool of Radiance, he cries ",
        clear: true,
    },
    Step::Say {
        text: "out, 'Keep the Gauntlet of Moander away from there, you ",
        clear: false,
    },
    Step::Say {
        text: "will unleash dangerous energies. Stay back!' As the ",
        clear: false,
    },
    Step::Say {
        text: "gauntlet contacts the pool, it contracts and shatters it.",
        clear: false,
    },
    Step::Animate {
        block: PIC_POOL,
        loops: 1,
        fade: false,
    },
    Step::Pause,
    Step::ClearPrompt,
    // `:495-503`
    Step::Say {
        text: "'I am trapped without escape, you have succeeded ",
        clear: true,
    },
    Step::Say {
        text: "where armies have not. Gloat while you may, Tyranthraxus ",
        clear: false,
    },
    Step::Say {
        text: "is slain this day.' Before your eyes he crumbles into ",
        clear: false,
    },
    Step::Say {
        text: "nothingness.",
        clear: false,
    },
    Step::Animate {
        block: PIC_CRUMBLE,
        loops: 1,
        fade: false,
    },
    Step::Pause,
    Step::ClearPrompt,
    // `:505-514` — the dissolve
    Step::Say {
        text: "You are certain he is destroyed because your ",
        clear: true,
    },
    Step::Say {
        text: "final bond fades away. The Curse of the Azure Bonds ",
        clear: false,
    },
    Step::Say {
        text: "has finally been lifted from you! You are free at ",
        clear: false,
    },
    Step::Say {
        text: "last!",
        clear: false,
    },
    Step::Animate {
        block: PIC_BOND,
        loops: FADE_LOOPS_AT_DEFAULT_SPEED,
        fade: true,
    },
    // `:516-525`
    Step::HeadBody {
        head: KNIGHTS_HEAD,
        body: KNIGHTS_HEAD,
    },
    Step::Say {
        text: "The Knights of Myth Drannor rush in, '",
        clear: true,
    },
    Step::Say {
        text: "Congratulations, you have destroyed the Flamed One. ",
        clear: false,
    },
    Step::Say {
        text: "With the power of Elminster, let us take you from ",
        clear: false,
    },
    Step::Say {
        text: "this  foul place, to a fine feast.'",
        clear: false,
    },
    Step::Pause,
    Step::ClearPrompt,
    // `:526-534`
    Step::BigPic {
        block: SHADOWDALE_BIGPIC,
    },
    Step::Say {
        text: "You are teleported to Shadowdale, where festivities ",
        clear: true,
    },
    Step::Say {
        text: "have already begun. A huge cheer goes up at your arrival. ",
        clear: false,
    },
    Step::Say {
        text: "Gharri and Nacacia, arm in arm, yell congratulations ",
        clear: false,
    },
    Step::Say {
        text: "from the nearby stands. 'You have won!'",
        clear: false,
    },
    Step::Fireworks,
];

/// `ShowAnimation`'s per-loop state (`ovr019.cs:413-414`: `loop_count` and
/// `start_time`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AnimRun {
    loops_left: u16,
    waited: u32,
    fade: bool,
}

/// ★ `end_game_text` as a parked shell state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Ending {
    /// Index into [`SCRIPT`].
    step: usize,
    /// Whether the current step has run its one-time setup.
    entered: bool,
    /// The live `press_any_key` job.
    job: Option<TextJob>,
    /// `ShowAnimation`'s loop, while one is running.
    anim: Option<AnimRun>,
    /// `endgame_529F4`'s particle system, while it is running.
    fireworks: Option<Fireworks>,
}

impl Default for Ending {
    fn default() -> Self {
        Ending::new()
    }
}

impl Ending {
    pub fn new() -> Self {
        Ending {
            step: 0,
            entered: false,
            job: None,
            anim: None,
            fireworks: None,
        }
    }

    /// A one-word probe for `RESTRIKE_DEBUG_LOG`.
    pub fn probe(&self) -> &'static str {
        match SCRIPT.get(self.step) {
            Some(Step::Fireworks) => "ending/fireworks",
            Some(Step::Animate { fade: true, .. }) => "ending/dissolve",
            Some(_) => "ending/text",
            None => "ending/done",
        }
    }

    /// How far through the sequence this is — a test/inspection seam.
    pub fn step_index(&self) -> usize {
        self.step
    }

    /// Whether a rocket or a burst is currently on screen (as against the
    /// `Random(10000)` wait between them) — a test/inspection seam.
    pub fn fireworks_running(&self) -> bool {
        self.fireworks.as_ref().is_some_and(Fireworks::running)
    }

    /// The number of beats the whole ending has.
    pub fn beat_count() -> usize {
        SCRIPT.len()
    }

    /// `Some(())` once the whole sequence has run — the caller then performs
    /// `CMD_Program`'s own tail (`ovr003.cs:1953-1972`).
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> Option<()> {
        // Several steps resolve without consuming a tick (`ClearPrompt`, the
        // picture arms); the bound is the script length, so a step that never
        // completes cannot spin.
        for _ in 0..=SCRIPT.len() {
            let Some(step) = SCRIPT.get(self.step).copied() else {
                // `gbl.game_state = gbl.last_game_state` (`:536`).
                ctx.state.game_state = ctx.state.last_game_state;
                return Some(());
            };
            if !self.run(step, ctx) {
                return None;
            }
            self.step += 1;
            self.entered = false;
        }
        None
    }

    /// Runs one beat; `true` when it is finished.
    fn run(&mut self, step: Step, ctx: &mut FlowCtx) -> bool {
        match step {
            Step::Say { text, clear } => self.say(text, clear, ctx),
            Step::Pause => self.pause(ctx),
            Step::ClearPrompt => {
                // `ovr027.ClearPromptArea()`.
                crate::combat::scene::render::clear_prompt_line(ctx.fb);
                true
            }
            Step::Animate { block, loops, fade } => self.animate(block, loops, fade, ctx),
            Step::HeadBody { head, body } => {
                // `head_body(0x41, 0x41)` then `draw_head_and_body(true, 3, 3)`
                // (`ovr019.cs:516-517`) — the same pair `CMD_Picture`'s
                // head/body arm sets up.
                crate::picture::cmd_picture(ctx, body, head);
                true
            }
            Step::BigPic { block } => {
                // `load_bigpic(0x7A)` + `draw_bigpic()` (`:526-528`).
                crate::picture::cmd_picture(ctx, block, 0xFF);
                true
            }
            Step::Fireworks => self.fireworks(ctx),
        }
    }

    /// `press_any_key` (`seg041.cs:125-231`) — paced, wrapping, paginating,
    /// exactly as every other prose site in this engine drives it.
    fn say(&mut self, text: &'static str, clear: bool, ctx: &mut FlowCtx) -> bool {
        if !self.entered {
            self.job = Some(TextJob::new(
                text,
                PROSE_COLOR,
                NORMAL_BOTTOM,
                clear,
                ctx.cursor,
                ctx.fb,
            ));
            self.entered = true;
        }
        let Some(job) = self.job.as_mut() else {
            return true;
        };
        let tick_ms = 1000.0 / crate::input::TICK_HZ as f64;
        let budget = ctx.pacer.tick(tick_ms);
        match job.advance(budget, ctx.fb, ctx.font, ctx.cursor) {
            JobStatus::Continuing => false,
            JobStatus::NeedsKey => {
                // The window is six rows and the groups are four short lines,
                // so pagination is unreachable in practice — handled rather
                // than assumed, as `GameOverFlow` does.
                if matches!(PressAnyKey.tick(ctx.input), WidgetOutcome::Done) {
                    job.release(ctx.fb);
                    ctx.input.clear();
                }
                false
            }
            JobStatus::Done => {
                self.job = None;
                true
            }
        }
    }

    /// `DisplayAndPause` (`seg041.cs:297-303`): `ClearPromptAreaNoUpdate`, the
    /// message on the prompt line in colour 13, then a blocking `GetInputKey`.
    fn pause(&mut self, ctx: &mut FlowCtx) -> bool {
        if !self.entered {
            crate::combat::scene::render::clear_prompt_line(ctx.fb);
            crate::text::draw_string(
                ctx.fb,
                ctx.font,
                PAUSE_PROMPT,
                0x18,
                0,
                0,
                PAUSE_PROMPT_COLOR,
            );
            self.entered = true;
            ctx.input.clear();
            return false;
        }
        if matches!(PressAnyKey.tick(ctx.input), WidgetOutcome::Done) {
            ctx.input.clear(); // `clear_keyboard` after the acknowledgement
            return true;
        }
        false
    }

    /// `ShowAnimation(num_loops, block_id, 3, 3)` (`ovr019.cs:411-444`), with
    /// the fade beat's `picture_fade = 1` / `= 0` bracket (`:510-514`).
    fn animate(&mut self, block: u8, loops: u16, fade: bool, ctx: &mut FlowCtx) -> bool {
        if !self.entered {
            if fade {
                ctx.vm_memory.set_raw_word(PICTURE_FADE_ADDR, 1); // `:510`
            }
            crate::picture::show_animation_begin(ctx, block);
            // `(10 - game_speed_var) * 2` is computed at the call
            // (`:512`), so read the live cell rather than trusting the table.
            let loops_left = if fade {
                u16::from(10u8.saturating_sub(crate::vmhost::game_speed(ctx.vm_memory))) * 2
            } else {
                loops
            };
            self.anim = Some(AnimRun {
                loops_left,
                waited: 0,
                fade,
            });
            self.entered = true;
        }
        let Some(run) = self.anim.as_mut() else {
            return true;
        };
        let (mut waited, mut loops_left) = (run.waited, run.loops_left);
        let done = crate::picture::show_animation_tick(ctx, &mut waited, &mut loops_left);
        if let Some(run) = self.anim.as_mut() {
            run.waited = waited;
            run.loops_left = loops_left;
        }
        if done {
            if fade {
                ctx.vm_memory.set_raw_word(PICTURE_FADE_ADDR, 0); // `:514`
            }
            self.anim = None;
        }
        done
    }

    /// `endgame_529F4()` (`ovr019.cs:356-408`).
    fn fireworks(&mut self, ctx: &mut FlowCtx) -> bool {
        if !self.entered {
            self.fireworks = Some(Fireworks::new());
            self.entered = true;
            ctx.input.clear();
        }
        let Some(show) = self.fireworks.as_mut() else {
            return true;
        };
        if show.tick(ctx.fb, ctx.rng, ctx.input) {
            self.fireworks = None;
            return true;
        }
        false
    }
}

// --- endgame_529F4: the fireworks (ovr019.cs:25-408) ---

/// `gbl.dword_1ADF6` is 120 entries — three groups of forty (`:358`, and every
/// loop that walks it, e.g. `sub_524F7:166-170`).
const GROUPS: usize = 3;
const PER_GROUP: usize = 40;
const PARTICLES: usize = GROUPS * PER_GROUP;

/// `endgame_5285E`'s step count at both call sites (`:387`, `:396` pass
/// `0x3C`).
const ROCKET_STEPS: u16 = 0x3C;

/// The launch cell both passes start from — `word_1AE0F`/`word_1AE11` are set
/// to `65` before the invisible pass and reset to `0x41` before the visible
/// one (`:378-379`, `:393-394`).
const LAUNCH_COL: i32 = 65;
const LAUNCH_ROW: i32 = 65;

/// `sub_524F7`'s row clip (`:205-206`, `:222-223`, `:236-237`): a particle only
/// touches the screen while `8 < row < 0x41`. The column is unclipped in the
/// original (EGA `SetPixel` wraps the plane); ours drops out-of-canvas columns
/// instead, which is the same non-comparable renderer territory the dither
/// already lives in.
const ROW_MIN: u16 = 8;
const ROW_MAX: u16 = 0x41;

/// ★ `endgame_529F4`'s idle gate, `Random(10000) < 1` per loop iteration
/// (`:368`) — evaluated this many times per engine tick.
///
/// The original's outer loop is unthrottled: when the gate fails, the body is
/// one `Random` and a compare, so a period machine spins it tens of thousands
/// of times a second and a burst starts every second or so. We have 60 ticks a
/// second and no clock to read (D-UI1), so the loop's *rate* is a number we
/// have to choose; this is the one that reproduces the original's observed
/// cadence — an expected 10000/200 = 50 ticks of sky between bursts, against a
/// burst that itself lasts about a second and a half.
const IDLE_ROLLS_PER_TICK: u32 = 200;

/// One `gbl.dword_1ADF6` entry (`Classes/Struct_1ADF6.cs`), named.
///
/// The two coordinate pairs are the original's own: `field_08`/`field_0A` are
/// the position in 1/32-pixel fixed point, `field_0C`/`field_0E` the velocity
/// in the same units, and `field_00`/`field_02` the integer cell currently
/// painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Particle {
    /// `field_00` — the painted column.
    col: u16,
    /// `field_02` — the painted row.
    row: u16,
    /// `field_04`/`field_06` — where this frame's move is taking it.
    next_col: i16,
    next_row: i16,
    /// `field_08`/`field_0A` — position, 1/32 px.
    fx: i16,
    fy: i16,
    /// `field_0C`/`field_0E` — velocity, 1/32 px per frame.
    vx: i16,
    vy: i16,
    /// `field_10` — the colour-ramp stage, 1..=5.
    stage: u8,
    /// `field_11` — the pixel this particle is covering, restored on the way
    /// out so the Shadowdale bigpic survives the display.
    under: u8,
    /// `field_12`..`field_15` — the frame each stage advances on, read as
    /// `byteArray_11(field_10)`. `field_12` is the literal 1 (`:146`).
    stage_at: [u8; 4],
}

/// `unk_1ADFB[group]` (`Classes/Struct_1ADFB.cs` + `sub_520B8:89-105`): the
/// five-entry colour ramp a group's particles walk as they age.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Ramp([u8; 5]);

impl Ramp {
    /// `sub_520B8:91-105`, with `Reset()`'s all-ones defaults (`:36-40`).
    fn build(color: u8) -> Self {
        let bright = color > 1;
        Ramp([
            if bright { 15 } else { 1 },
            15,
            if bright { color + 8 } else { 1 },
            color,
            1,
        ])
    }

    /// `unk_1ADFB[group][field_10 - 1]` (`sub_524F7:239`).
    fn at(&self, stage: u8) -> u8 {
        self.0[(stage as usize).clamp(1, 5) - 1]
    }
}

/// ★ `endgame_529F4`'s state (`ovr019.cs:356-408`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fireworks {
    particles: Vec<Particle>,
    ramps: [Ramp; GROUPS],
    phase: Phase,
    /// `gbl.byte_1ADFA + 1` — how many frames this burst runs (`:254`).
    burst_frames: u16,
    /// `gbl.byte_1AE0A` — the key that ends the display, latched.
    keyed: bool,
    /// `sub_52068`'s one-`SysDelay(1)` palette flash, held for a tick.
    flash: bool,
}

/// Where the display currently is inside one burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Phase {
    /// The `Random(10000) < 1` wait (`:367-368`).
    Idle,
    /// `endgame_5285E(1, 0x3C, …)` — the visible rocket (`:396`).
    Rocket {
        step: u16,
        col: i32,
        row: i32,
        fx: i32,
        fy: i32,
        vx: i32,
        vy: i32,
        under: u8,
    },
    /// `sub_5279B` → `sub_524F7` × (`byte_1ADFA` + 1) (`:398`, `:254-259`).
    Burst { frame: u16, frames: u16 },
}

impl Default for Fireworks {
    fn default() -> Self {
        Fireworks::new()
    }
}

impl Fireworks {
    fn new() -> Self {
        Fireworks {
            particles: vec![Particle::default(); PARTICLES],
            ramps: [Ramp::default(); GROUPS],
            phase: Phase::Idle,
            burst_frames: 1,
            keyed: false,
            flash: false,
        }
    }

    /// Whether a rocket or a burst is on screen right now.
    fn running(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// One tick. `true` once the display is over — which, faithfully, is only
    /// ever tested at the end of a burst (`:400-403` sits inside the burst
    /// branch), so a key pressed mid-rocket ends the show when that rocket's
    /// burst finishes.
    fn tick(
        &mut self,
        fb: &mut Framebuffer,
        rng: &mut EngineRng,
        input: &mut crate::input::InputQueue,
    ) -> bool {
        if self.flash {
            // `sub_52068`'s second `SetPaletteColor(9, 9)` (`:29`).
            fb.set_ega_palette(9, 9);
            self.flash = false;
        }
        if input.read_key().is_some() {
            self.keyed = true; // `byte_1AE0A = READKEY()` (`:402`)
        }
        match self.phase {
            Phase::Idle => {
                if self.keyed {
                    // The original re-tests the gate forever once keyed, but
                    // never fires again (`byte_1AE0A == 0` guards the body), so
                    // an idle display with a key waiting is simply over.
                    return true;
                }
                for _ in 0..IDLE_ROLLS_PER_TICK {
                    if rng.random(10_000) < 1 {
                        self.launch(fb, rng);
                        return false;
                    }
                }
                false
            }
            Phase::Rocket { .. } => {
                self.rocket_step(fb, rng);
                false
            }
            Phase::Burst { frame, frames } => {
                self.burst_frame(fb, frame);
                if frame >= frames {
                    // `sub_5279B`'s tail: every particle puts back the pixel it
                    // was covering (`:261-273`).
                    self.erase(fb);
                    self.phase = Phase::Idle;
                    return self.keyed;
                }
                self.phase = Phase::Burst {
                    frame: frame + 1,
                    frames,
                };
                false
            }
        }
    }

    /// `endgame_529F4`'s burst set-up (`:370-397`): pick the colours, pick the
    /// launch velocity, fly the rocket **invisibly** to find the burst point,
    /// seed the particles there, then re-fly it visibly from the launch cell.
    fn launch(&mut self, fb: &mut Framebuffer, rng: &mut EngineRng) {
        // `FillChar(1, 3, unk_1AE0B)` then `Random(2)` of them get a real
        // colour (`:370-376`) — so most bursts are one coloured group and two
        // plain ones, and a `Random(2)` of 0 leaves all three plain.
        let mut colors = [1u8; GROUPS];
        let colored = rng.random(2) as usize;
        for c in colors.iter_mut().take(colored.min(GROUPS)) {
            *c = (rng.random(5) + 2) as u8;
        }

        // `:378-385`
        let mut vx = i32::from(rng.random(20)) + 35;
        let mut vy = -(i32::from(rng.random(5)) + 50);
        let (vx0, vy0) = (vx, vy);

        // `endgame_5285E(0, 0x3C, …)` — invisible: the pass exists only to
        // leave the burst point in `word_1AE0F`/`word_1AE11` (`:387`).
        //
        // **Timing deviation, named:** the original's shared routine still
        // sleeps `SysDelay(0x0F)` per step on this pass (`:331` is outside the
        // `arg_0 != 0` guard), so it holds a blank ~0.9 s before every rocket.
        // It draws nothing, and D-UI1 makes the tick budget ours to assign, so
        // we integrate the trajectory in one tick and spend the ticks on the
        // rocket the player can actually see.
        let (mut col, mut row) = (LAUNCH_COL, LAUNCH_ROW);
        let (mut fx, mut fy) = (col << 5, row << 5);
        for _ in 0..=ROCKET_STEPS {
            fx += vx;
            fy += vy + 1;
            col = fx / 32;
            row = fy / 32;
            vy += 1;
        }

        // `sub_520B8(unk_1AE0B, word_1AE15, word_1AE13, word_1AE11,
        // word_1AE0F)` — the burst point, and the velocity the rocket carried
        // into it (`:388`).
        self.burst_frames = self.seed(fb, rng, &colors, vy, vx, row, col);

        // `:390-394` — the originals come back and the launch cell is reset.
        vx = vx0;
        vy = vy0;
        // `:296-306` — the pixel under the launch cell, sampled only inside the
        // row clip. Row 65 is `0x41`, which the `< 0x41` test excludes, so the
        // original really does start with `var_B == 0`.
        let under = if LAUNCH_ROW > i32::from(ROW_MIN) && LAUNCH_ROW < i32::from(ROW_MAX) {
            read_pixel(fb, LAUNCH_COL, LAUNCH_ROW)
        } else {
            0
        };
        self.phase = Phase::Rocket {
            step: 0,
            col: LAUNCH_COL,
            row: LAUNCH_ROW,
            fx: LAUNCH_COL << 5,
            fy: LAUNCH_ROW << 5,
            vx,
            vy,
            under,
        };
        // `endgame_5285E(1, …)`'s own opening `sub_52068()` (`:287-290`).
        self.set_flash(fb);
    }

    /// `sub_520B8` (`ovr019.cs:69-159`): three groups of forty particles fanned
    /// out of the burst point. Returns `byte_1ADFA + 1` — the burst's frame
    /// count (`:109-112`, `:254`).
    #[allow(clippy::too_many_arguments)]
    fn seed(
        &mut self,
        fb: &Framebuffer,
        rng: &mut EngineRng,
        colors: &[u8; GROUPS],
        burst_vy: i32,
        burst_vx: i32,
        burst_row: i32,
        burst_col: i32,
    ) -> u16 {
        let mut longest = 0u8; // `gbl.byte_1ADFA`
        for (group, &color) in colors.iter().enumerate() {
            self.ramps[group] = Ramp::build(color);

            // `:107-115`
            let life = (rng.random(20) + 25) as u8;
            longest = longest.max(life);
            let stage2 = i32::from(rng.random(5)) + 5;
            let stage3 = stage2 + 15;

            // `:117-124` — the group's own direction off the rocket's momentum.
            // Two `Random__Real()` draws pick a point on a sphere; the two
            // `Random(10) + 24` scales are separate draws, one per axis. These
            // two casts are plain `(int)`, not the `(ushort)` the per-particle
            // pair uses.
            let a = rng.random_real() * std::f64::consts::TAU;
            let b = rng.random_real() * std::f64::consts::TAU;
            let spread = f64::from(rng.random(10) + 24);
            let group_vx = (spread * a.sin() * b.sin()) as i32 + burst_vx;
            let spread = f64::from(rng.random(10) + 24);
            let group_vy = (spread * a.cos() * b.sin()) as i32 + burst_vy;

            for i in 0..PER_GROUP {
                // `:130-131` — a fresh direction per particle.
                let a = rng.random_real() * std::f64::consts::TAU;
                let b = rng.random_real() * std::f64::consts::TAU;
                let p = &mut self.particles[group * PER_GROUP + i];
                p.col = burst_col as u16;
                p.row = burst_row as u16;
                p.fx = ((i32::from(p.col)) << 5) as i16;
                p.fy = ((i32::from(p.row)) << 5) as i16;
                // `:138-143` — the `(ushort)` casts are the original's own
                // 16-bit truncation of a negative double, kept verbatim.
                p.vx = (group_vx as i16).wrapping_add(trunc16(a.sin() * 16.0 * b.sin()) as i16);
                p.vy = (group_vy as i16).wrapping_add(trunc16(a.cos() * 16.0 * b.sin()) as i16);
                p.stage = 1; // `:145`
                p.stage_at = [
                    1,                                                  // `field_12 = 1` (`:146`)
                    (stage2 + i32::from(rng.random(7)) - 4) as u8,      // `:148`
                    (stage3 + i32::from(rng.random(11)) - 6) as u8,     // `:149`
                    (i32::from(life) + i32::from(rng.random(7))) as u8, // `:150`
                ];
                p.under = read_pixel(fb, i32::from(p.col), i32::from(p.row)); // `:156`
            }
        }
        u16::from(longest) + 1
    }

    /// One step of `endgame_5285E(1, 0x3C, …)` (`ovr019.cs:310-335`), one per
    /// tick — the original's own `SysDelay(0x0F)` pace, near enough (15 ms
    /// against our 16.7 ms tick).
    fn rocket_step(&mut self, fb: &mut Framebuffer, rng: &mut EngineRng) {
        let Phase::Rocket {
            step,
            col,
            row,
            fx,
            fy,
            vx,
            vy,
            under,
        } = self.phase
        else {
            return;
        };
        let (mut fx, mut fy, mut vy) = (fx, fy, vy);
        fx += vx;
        fy += vy + 1;
        let next_col = fx / 32;
        let next_row = fy / 32;
        vy += 1;

        let mut under = under;
        let visible = row > i32::from(ROW_MIN) && row < i32::from(ROW_MAX);
        if visible {
            write_pixel(fb, col, row, under); // put back what was there
            under = read_pixel(fb, next_col, next_row);
            write_pixel(fb, next_col, next_row, (rng.random(7) + 8) as u8);
        }

        if step + 1 >= ROCKET_STEPS {
            // `:337-342` — the last drawn pixel is put back, so the rocket
            // leaves no trail on the bigpic. (The original then performs one
            // more, undrawn, position update at `:344-352`; it lands on the
            // burst point the invisible pass already found, which is where the
            // particles are seeded, so nothing observable rides on it.)
            if row > i32::from(ROW_MIN) && row < i32::from(ROW_MAX) {
                write_pixel(fb, next_col, next_row, under);
            }
            self.phase = Phase::Burst {
                frame: 1,
                frames: self.burst_frames,
            };
            return;
        }
        self.phase = Phase::Rocket {
            step: step + 1,
            col: next_col,
            row: next_row,
            fx,
            fy,
            vx,
            vy,
            under,
        };
    }

    /// `sub_524F7(gbl.dword_1ADF6, frame)` (`ovr019.cs:162-248`) — one burst
    /// frame, in the original's own four passes.
    ///
    /// ★ **Correction, with evidence.** coab's `:172-173` reads
    /// `field_08 = field_0C; field_0A = field_0E;` — position *assigned* from
    /// velocity. That cannot be what the binary does: `sub_520B8` initialises
    /// `field_08 = field_00 << 5` (`:135-136`) only for it to be discarded on
    /// the very first frame, every particle would sit one or two pixels from
    /// the origin forever, and the sibling routine `endgame_5285E` integrates
    /// the identical fixed-point pair with `+=` (`:312-313`). It is a lost `+`
    /// in the decompilation; transcribed here as the accumulation it is.
    fn burst_frame(&mut self, fb: &mut Framebuffer, frame: u16) {
        let gravity = frame.is_multiple_of(6); // `:164`, `var_1 == 0`
        for p in self.particles.iter_mut() {
            p.fx = p.fx.wrapping_add(p.vx); // `:172` (see the note above)
            p.fy = p.fy.wrapping_add(p.vy); // `:173`
            p.next_col = p.fx / 32; // `:174`
            p.next_row = p.fy / 32; // `:175`
            if gravity {
                p.vy = p.vy.wrapping_add(1); // `:177-180`
            }
            // `:182-189` — the horizontal drift decays one unit a frame.
            match p.vx.cmp(&0) {
                std::cmp::Ordering::Greater => p.vx -= 1,
                std::cmp::Ordering::Less => p.vx += 1,
                std::cmp::Ordering::Equal => {}
            }
            // `:191-195` — `byteArray_11(field_10)` is `field_12..field_15`
            // for stages 1..4, so the ramp advances four times at most.
            if p.stage < 5 && u16::from(p.stage_at[(p.stage as usize) - 1]) < frame {
                p.stage += 1;
            }
        }
        // `:199-211` — restore what every particle was covering.
        for p in self.particles.iter() {
            if on_screen(p.row) {
                write_pixel(fb, i32::from(p.col), i32::from(p.row), p.under);
            }
        }
        // `:213-228` — move, then sample the new pixel underneath.
        for p in self.particles.iter_mut() {
            p.col = p.next_col as u16;
            p.row = p.next_row as u16;
            if on_screen(p.row) {
                p.under = read_pixel(fb, i32::from(p.col), i32::from(p.row));
            }
        }
        // `:230-242` — draw the ramp colour.
        let ramps = self.ramps;
        for (i, p) in self.particles.iter().enumerate() {
            if on_screen(p.row) {
                let color = ramps[i / PER_GROUP].at(p.stage);
                write_pixel(fb, i32::from(p.col), i32::from(p.row), color);
            }
        }
        // `:244-247` — the flash when the leading particle reaches stage 2.
        if self.particles[0].stage == 2 {
            self.set_flash(fb);
        }
    }

    /// `sub_5279B`'s tail (`:261-273`): the display leaves the picture as it
    /// found it.
    fn erase(&mut self, fb: &mut Framebuffer) {
        for p in self.particles.iter() {
            if on_screen(p.row) {
                write_pixel(fb, i32::from(p.col), i32::from(p.row), p.under);
            }
        }
    }

    /// `sub_52068` (`ovr019.cs:25-30`): palette slot 9 flashes white for one
    /// `SysDelay(1)`. At 60 Hz that is less than a tick, so it is held for
    /// exactly one and put back at the start of the next.
    fn set_flash(&mut self, fb: &mut Framebuffer) {
        fb.set_ega_palette(9, 15);
        self.flash = true;
    }
}

/// The original's `(ushort)someDouble` — truncate toward zero into 16 bits and
/// let it wrap, which is how a negative offset becomes a large unsigned one
/// that wraps back on the following add (`sub_520B8:140,143`).
fn trunc16(d: f64) -> i32 {
    i32::from(d as i32 as i16)
}

/// `sub_524F7`'s row clip (`:205-206`).
fn on_screen(row: u16) -> bool {
    row > ROW_MIN && row < ROW_MAX
}

/// `GetPixel(row, column)` (`ovr019.cs:19-22`), with our own canvas bound —
/// [`Framebuffer::get_pixel`] panics off-canvas and the original's EGA read
/// simply wraps the plane.
fn read_pixel(fb: &Framebuffer, col: i32, row: i32) -> u8 {
    if col < 0 || row < 0 || col as usize >= WIDTH || row as usize >= HEIGHT {
        return 0;
    }
    fb.get_pixel(col as usize, row as usize)
}

/// `SetPixel(colour, row, column)` (`ovr019.cs:13-16`).
fn write_pixel(fb: &mut Framebuffer, col: i32, row: i32, color: u8) {
    if col < 0 || row < 0 {
        return;
    }
    fb.set_pixel(col as usize, row as usize, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The script is the original's, in the original's order: twenty-four
    /// prose lines in six groups of four, four `DisplayAndPause`s, three
    /// `ShowAnimation`s (exactly one of them fade-armed), the head/body, the
    /// bigpic and the fireworks.
    #[test]
    fn the_script_is_end_game_texts_own_shape() {
        let says = SCRIPT
            .iter()
            .filter(|s| matches!(s, Step::Say { .. }))
            .count();
        assert_eq!(says, 24, "twenty-four press_any_key lines");
        let clears = SCRIPT
            .iter()
            .filter(|s| matches!(s, Step::Say { clear: true, .. }))
            .count();
        assert_eq!(clears, 6, "six groups, each opening with clearArea");
        assert_eq!(
            SCRIPT.iter().filter(|s| matches!(s, Step::Pause)).count(),
            4,
            "DisplayAndPause at :483, :492, :502, :524"
        );
        assert_eq!(
            SCRIPT
                .iter()
                .filter(|s| matches!(s, Step::ClearPrompt))
                .count(),
            3,
            "ClearPromptArea at :493, :503, :525"
        );
        let animations: Vec<_> = SCRIPT
            .iter()
            .filter_map(|s| match s {
                Step::Animate { block, fade, .. } => Some((*block, *fade)),
                _ => None,
            })
            .collect();
        assert_eq!(
            animations,
            vec![(PIC_POOL, false), (PIC_CRUMBLE, false), (PIC_BOND, true)]
        );
        assert!(matches!(SCRIPT.last(), Some(Step::Fireworks)));
        // The head/body arrives before its prose, the bigpic before its own —
        // the original draws the art and then talks over it.
        let head = SCRIPT
            .iter()
            .position(|s| matches!(s, Step::HeadBody { .. }))
            .unwrap();
        let bigpic = SCRIPT
            .iter()
            .position(|s| matches!(s, Step::BigPic { .. }))
            .unwrap();
        assert!(head < bigpic);
        assert!(matches!(SCRIPT[head + 1], Step::Say { clear: true, .. }));
        assert!(matches!(SCRIPT[bigpic + 1], Step::Say { clear: true, .. }));
    }

    /// Every line fits the six-row, 38-column window it prints into, so the
    /// pagination gate never opens mid-ending (which is why the original can
    /// get away with four `press_any_key`s and one `DisplayAndPause`).
    #[test]
    fn each_group_of_four_lines_fits_the_text_window() {
        let width = NORMAL_BOTTOM.x_end + 1 - NORMAL_BOTTOM.x_start;
        let rows = NORMAL_BOTTOM.y_end + 1 - NORMAL_BOTTOM.y_start;
        let mut group = String::new();
        let mut groups = 0;
        let check = |group: &str| {
            if group.is_empty() {
                return;
            }
            let mut used = 1;
            let mut col = 0;
            for word in group.split(' ').filter(|w| !w.is_empty()) {
                if col + word.len() + usize::from(col > 0) > width {
                    used += 1;
                    col = 0;
                }
                col += word.len() + usize::from(col > 0);
            }
            assert!(
                used <= rows,
                "{used} rows needed, {rows} available: {group}"
            );
        };
        for step in SCRIPT {
            match step {
                Step::Say { text, clear: true } => {
                    check(&group);
                    groups += 1;
                    group = (*text).to_string();
                }
                Step::Say { text, clear: false } => group.push_str(text),
                _ => {}
            }
        }
        check(&group);
        assert_eq!(groups, 6);
    }

    /// The colour ramp is `[15|1, 15, c+8|1, c, 1]` — a white flash into the
    /// group's own colour and out to black (`sub_520B8:91-105`).
    #[test]
    fn the_colour_ramp_is_the_originals() {
        assert_eq!(Ramp::build(4).0, [15, 15, 12, 4, 1]);
        // A group that never got a colour keeps `Reset()`'s ones everywhere
        // but the always-15 second entry.
        assert_eq!(Ramp::build(1).0, [1, 15, 1, 1, 1]);
        let r = Ramp::build(6);
        assert_eq!(r.at(1), 15);
        assert_eq!(r.at(5), 1);
    }

    /// ★ The display actually draws: a rocket climbs out of the launch cell
    /// and a burst puts particles in the sky, both inside the `8 < row < 0x41`
    /// clip, and every one of them puts back the pixel it borrowed.
    #[test]
    fn a_burst_draws_a_rocket_and_particles_and_restores_what_it_covered() {
        let mut fb = Framebuffer::new();
        // A recognisable "bigpic" underneath: colour 9 everywhere the display
        // is allowed to touch.
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                fb.set_pixel(x, y, 9);
            }
        }
        let pristine: Vec<u8> = fb.pixels().to_vec();
        let mut rng = EngineRng::new(0x5A1E_5A1E);
        let mut show = Fireworks::new();
        let mut input = crate::input::InputQueue::default();

        // Force a launch rather than waiting on `Random(10000)`.
        show.launch(&mut fb, &mut rng);
        assert!(matches!(show.phase, Phase::Rocket { .. }));

        let painted = |fb: &Framebuffer| {
            (0..HEIGHT)
                .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
                .filter(|&(x, y)| fb.get_pixel(x, y) != 9)
                .count()
        };

        let mut rocket_seen = 0usize;
        for _ in 0..ROCKET_STEPS {
            show.tick(&mut fb, &mut rng, &mut input);
            rocket_seen = rocket_seen.max(painted(&fb));
        }
        assert!(rocket_seen > 0, "the rocket never drew a pixel");
        assert!(
            matches!(show.phase, Phase::Burst { .. }),
            "60 steps and the rocket has burst"
        );

        let mut burst_seen = 0usize;
        for _ in 0..200 {
            if matches!(show.phase, Phase::Idle) {
                break;
            }
            show.tick(&mut fb, &mut rng, &mut input);
            burst_seen = burst_seen.max(painted(&fb));
        }
        assert!(
            burst_seen > 20,
            "the burst put only {burst_seen} pixels in the sky"
        );
        assert!(
            matches!(show.phase, Phase::Idle),
            "the burst ends and the sky goes quiet again"
        );
        // ★ The display gives the picture back — with the original's own two
        // small artifacts, transcribed rather than papered over:
        //   1. `endgame_5285E`'s `var_B` starts at 0 when the launch cell is
        //      outside the row clip (row 65 fails `< 0x41`), so the rocket's
        //      first in-range step "restores" a black pixel it never sampled
        //      (`ovr019.cs:296-306,320-329`).
        //   2. Two particles landing on the same pixel both sample it, and the
        //      second restores what the first drew.
        // Both are a handful of pixels against 120 particles over ~40 frames.
        let residue = pristine
            .iter()
            .zip(fb.pixels().iter())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            residue < 64,
            "the display left {residue} pixels behind — that is not aliasing, \
             that is a leak"
        );
        eprintln!("fireworks residue: {residue} pixels");
    }

    /// `(ushort)` of a negative double is the original's own 16-bit wrap, and
    /// the add that follows wraps it back — the pair is a signed offset.
    #[test]
    fn the_sixteen_bit_truncation_round_trips_a_negative_offset() {
        assert_eq!(trunc16(-3.7), -3);
        assert_eq!(trunc16(12.9), 12);
        let base: i16 = 40;
        assert_eq!(base.wrapping_add(trunc16(-3.7) as i16), 37);
    }
}
