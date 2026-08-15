//! `restrike-desktop [DIR] [--seed N] [--square-pixels] [--slot X|none]
//! [--watch CAPTURE [--turbo N]]` -- the winit + softbuffer presenter (D-UI6). Loads
//! `GBX_DATA_DIR` (or a positional dir argument) into a `GameData`, boots the
//! `Engine`, and runs a fixed 60 Hz tick loop: `ControlFlow::WaitUntil` plus an
//! accumulator calls `tick` regardless of display refresh rate, collecting winit
//! keyboard events since the last tick into that tick's input slice. Presents on
//! `frame.serial` change only. Knows nothing about what a key *means* or
//! what's on screen -- see `keymap.rs`/`scale.rs` for the two things it
//! does know: platform key -> `InputEvent`, and D-UI6 scaling.
//!
//! ## `--watch <capture>` -- the M6a reel
//!
//! Plays a closed `.gbxtrace` combat capture on screen, with draw equality
//! asserted live (`combat-visualizer.md` D-CV1 item 2). The frontend stays a
//! **thin presenter**: it parses the capture with `gbx-oracle`, hands the
//! assembled `ReelInput` to `Engine::new_reel`, and presents Frames exactly as
//! in normal play. It owns no framebuffer, no atlas and no timeline -- v1's
//! frontend-pumped design was rejected in review, and this is why the reel is an
//! `Engine` constructor rather than a second app.
//!
//! `--turbo N` multiplies the reel's tick rate (D-CV3's open speed door: frames
//! are unchanged, only wall time). The long captures want it -- sewer-fight-2 is
//! 18,185 draws over 49 rounds.
//!
//! ## `--slot` -- the front door and the shortcut past it (slice 9b)
//!
//! With no `--slot`, launching lands where the original lands: the TITLE.DAX
//! sequence, the 30-second "Play Demo" prompt, the copy-protection challenge
//! (answer shown, D-RC4 -- `RESTRIKE_COPY_PROTECTION=faithful` hides it), then
//! `startGameMenu`, where `L` loads a slot and `B` begins.
//!
//! `--slot X` is the **power-user shortcut**: import that original save and
//! start playing immediately, skipping the whole preamble -- what every
//! capture/demo flow and most development launches want. `--slot none` keeps
//! the bare boot (no party) for engine archaeology.
//!
//! ## Save slots -- `--saves <DIR>` (roll-credits slice 0, G0)
//!
//! The engine's save/load screen is pure (D8): a slot pick sets a
//! `SaveLoadRequest` and the *host* performs the file I/O. This frontend is
//! that host -- after every tick it takes the request, fulfills it with
//! `gbx_engine::saveload_fs`, re-scans the directory so the screen shows real
//! slots, and reports the verdict on screen via `Engine::report_host_notice`.
//!
//! **The saves directory is `<data dir>/SAVE` by default** -- the same
//! directory the boot import already reads, and the one `saveload.rs`'s
//! filename convention was designed for: our `SAVGAM{L}.RSAV` snapshots sit
//! *beside* the originals' `SAVGAM{L}.DAT`, never overwriting them (D-SAVE12),
//! so a single scan sees both and the Load list can offer either. Override with
//! `--saves <DIR>` or `RESTRIKE_SAVE_DIR=<dir>` (an install you would rather
//! not write into); the cost is that the original slots, which live with the
//! game data, stop appearing in the Load list. The resolved path is printed at
//! boot.

mod keymap;
mod scale;

use std::env;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gbx_engine::debug_log;
use gbx_engine::engine::Engine;
use gbx_engine::framebuffer::{HEIGHT, WIDTH};
use gbx_engine::input::{InputEvent, TICK_HZ};
use gbx_engine::saveload_fs;
use gbx_formats::game_data::load_dir;
use gbx_oracle::replay;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Determinism over novelty (task brief): a fixed default seed, so two
/// launches with no `--seed` reproduce the same PRNG stream. Override with
/// `--seed N` for anything else.
const DEFAULT_SEED: u32 = 1;
const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICK_HZ as u64);

fn main() {
    let mut dir_arg = None;
    let mut seed = DEFAULT_SEED;
    let mut square_pixels = false;
    let mut watch: Option<PathBuf> = None;
    let mut turbo: u32 = 1;
    // ★ Roll-credits slice 9b: the default launch is the FRONT DOOR
    // (`seg001.PROGRAM`'s title / Play-Demo prompt / copy protection /
    // `startGameMenu`). `--slot X` is the power-user shortcut past all of it,
    // straight into the imported game; `--slot none` is the bare boot.
    let mut slot: SlotArg = SlotArg::FrontDoor;
    let mut saves_arg: Option<PathBuf> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--saves" => {
                let v = args.next().expect("--saves requires a directory");
                saves_arg = Some(PathBuf::from(v));
            }
            "--seed" => {
                let v = args.next().expect("--seed requires a value");
                seed = v.parse().expect("--seed must be a u32");
            }
            "--square-pixels" => square_pixels = true,
            "--watch" => {
                let v = args.next().expect("--watch requires a capture path");
                watch = Some(PathBuf::from(v));
            }
            "--turbo" => {
                let v = args.next().expect("--turbo requires a value");
                turbo = v.parse().expect("--turbo must be a positive integer");
            }
            "--slot" => {
                let v = args
                    .next()
                    .expect("--slot requires a save letter (A-J) or 'none'");
                slot = match v.as_str() {
                    "none" => SlotArg::Bare,
                    s => SlotArg::Import(
                        s.chars()
                            .next()
                            .expect("--slot letter")
                            .to_ascii_uppercase(),
                    ),
                };
            }
            other => dir_arg = Some(PathBuf::from(other)),
        }
    }
    let dir = dir_arg
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
        .expect("restrike-desktop: pass a data directory or set GBX_DATA_DIR");
    let data = load_dir(&dir).expect("restrike-desktop: failed to read the data directory");
    let saves_dir = resolve_saves_dir(saves_arg, &dir);
    let watching = watch.is_some();
    let mut engine = match watch {
        Some(capture) => open_reel(data, &capture, turbo),
        None => boot_with_party(data, &dir, slot, seed),
    };
    if !watching {
        eprintln!("restrike-desktop: save slots in {}", saves_dir.display());
        engine.set_slot_directory(saveload_fs::scan_slot_directory(&saves_dir));
        // ★ Slice 9c: the `.guy` character files `Add Character to Party`
        // lists, filtered against whoever is already in the party.
        let files = saveload_fs::scan_char_files(&saves_dir).without_party_members(engine.party());
        eprintln!(
            "restrike-desktop: {} saved character(s) available to add",
            files.entries.len()
        );
        engine.set_char_file_directory(files);
    }

    let event_loop = EventLoop::new().expect("failed to create the winit event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(engine, square_pixels, saves_dir, seed);
    event_loop
        .run_app(&mut app)
        .expect("the event loop exited with an error");
}

/// What `--slot` asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotArg {
    /// No `--slot` at all: the front door (the default a player gets).
    FrontDoor,
    /// `--slot none`: bare boot, no party — engine archaeology and fixtures.
    Bare,
    /// `--slot X`: import that original slot and start playing immediately,
    /// skipping the front door. The power-user shortcut.
    Import(char),
}

/// The saves directory (module doc): `--saves DIR`, else `RESTRIKE_SAVE_DIR`,
/// else `<data dir>/SAVE` — where the originals already live, so one scan sees
/// our `.rsav` slots and the importable `savgam{letter}.dat` sets together.
fn resolve_saves_dir(arg: Option<PathBuf>, data_dir: &std::path::Path) -> PathBuf {
    arg.or_else(|| env::var_os("RESTRIKE_SAVE_DIR").map(PathBuf::from))
        .unwrap_or_else(|| data_dir.join("SAVE"))
}

/// `--watch`: parse the capture with `gbx-oracle`, assemble the engine-side
/// `ReelInput`, and boot a watch-mode `Engine` over it.
///
/// Everything specific to captures lives on this side of the call; from
/// `App`'s point of view the returned engine is just an engine.
fn open_reel(data: gbx_formats::game_data::GameData, capture: &PathBuf, turbo: u32) -> Engine {
    let name = capture
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = std::fs::read_to_string(capture).unwrap_or_else(|e| {
        panic!(
            "restrike-desktop: cannot read the capture {}: {e}",
            capture.display()
        )
    });
    // The `ITEMS` table is needed by any capture with a ranged loadout; the
    // assembly refuses loudly rather than replaying a different fight.
    let mut input = replay::reel_input_from_capture(&name, &text, replay::load_item_data())
        .unwrap_or_else(|e| panic!("restrike-desktop: {e}"));
    input.tick_multiplier = turbo.max(1);
    if replay::sidecar_for(&name).is_none() {
        eprintln!(
            "restrike-desktop: {name} has no committed sidecar row — replaying with the \
             documented defaults (heading 2, magic off, no schedules) and no monster icons. \
             Expect it to refuse if the roster has monsters."
        );
    }
    eprintln!(
        "restrike-desktop: watching {name} — {} combatants, {} captured draws{}",
        input.combatants.len(),
        input.expected_draws.len(),
        if turbo > 1 {
            format!(", {turbo}x")
        } else {
            String::new()
        }
    );
    Engine::new_reel(data, input).unwrap_or_else(|e| panic!("restrike-desktop: {e}"))
}

/// Normal play boots WITH a party: import the save slot (default `A`, the
/// GOG-bundled one — the "continue where the box left you" experience, and
/// the same state every demo/capture flow uses). The original never lets a
/// partyless session adventure; an engine booted bare can walk but any
/// COMBAT refuses with "no living party" — exactly the trap this default
/// closes. `--slot none` keeps the bare boot for engine archaeology.
fn boot_with_party(
    data: gbx_formats::game_data::GameData,
    dir: &std::path::Path,
    slot: SlotArg,
    seed: u32,
) -> Engine {
    let letter = match slot {
        SlotArg::FrontDoor => {
            eprintln!(
                "restrike-desktop: front door — title, Play Demo, copy protection, start menu \
                 (pass --slot <letter> to skip straight into an imported game)"
            );
            let mut engine =
                Engine::new_front_door(data, seed).expect("restrike-desktop: front door failed");
            // D4/D-RC4: `RESTRIKE_COPY_PROTECTION=faithful` hides the answer.
            if std::env::var("RESTRIKE_COPY_PROTECTION").as_deref() == Ok("faithful") {
                engine.set_copy_protection_faithful(true);
            }
            apply_game_speed(&mut engine);
            return engine;
        }
        SlotArg::Bare => {
            eprintln!("restrike-desktop: --slot none — bare boot, NO PARTY (fights will refuse)");
            return Engine::new(data, seed).expect("restrike-desktop: bare boot failed");
        }
        SlotArg::Import(letter) => letter,
    };
    let saves = load_dir(&dir.join("SAVE"))
        .unwrap_or_else(|e| panic!("restrike-desktop: {}/SAVE unreadable: {e}", dir.display()));
    let master_name = format!("SAVGAM{letter}.DAT");
    let master = saves.raw_file(&master_name).unwrap_or_else(|| {
        panic!(
            "restrike-desktop: save slot {letter} not found ({master_name}); \
             pass --slot <letter> or --slot none"
        )
    });
    let set = gbx_formats::save_orig::load_from_lookup(master, letter, |n| saves.raw_file(n))
        .unwrap_or_else(|e| panic!("restrike-desktop: slot {letter} did not parse: {e:?}"));
    let party_size = set.chars.len();
    let mut engine = gbx_engine::import::import_original(&set, data, seed)
        .unwrap_or_else(|e| panic!("restrike-desktop: slot {letter} did not import: {e:?}"));
    eprintln!("restrike-desktop: imported save slot {letter} — party of {party_size}");
    apply_game_speed(&mut engine);
    engine
}

/// `RESTRIKE_GAME_SPEED=1..9`: the original's own speed setting (camp
/// Alter ▸ Speed territory; 1 = fastest text, default 4) as an env knob until
/// that screen lands. Re-applied after every engine replacement — a `.rsav`
/// carries its own pacer, but an import starts at the boot default.
fn apply_game_speed(engine: &mut Engine) {
    if let Some(speed) = std::env::var("RESTRIKE_GAME_SPEED")
        .ok()
        .and_then(|s| s.parse::<u8>().ok())
    {
        engine.set_game_speed(speed);
    }
}

/// One line a player can act on for each `SlotIoError` — the whole point of
/// routing this back to the screen rather than only to stderr.
fn describe_slot_error(err: &saveload_fs::SlotIoError) -> String {
    use saveload_fs::SlotIoError as E;
    match err {
        E::Io(e) if e.kind() == std::io::ErrorKind::NotFound => "no save file there.".to_string(),
        E::Io(e) => format!("file error — {e}"),
        E::Restore(e) => format!("{e}"),
        E::Import(e) => format!("original save did not import ({e:?})"),
        E::OriginalParse(msg) => format!("original save unreadable ({msg})"),
    }
}

struct App {
    engine: Engine,
    square_pixels: bool,
    window: Option<Rc<Window>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    /// The last-presented frame, expanded to RGBA -- updated only when
    /// `frame.serial` changes (D-UI1: palette expansion is a frontend
    /// concern, never engine state).
    rgba: Vec<[u8; 4]>,
    last_serial: u64,
    pending_input: Vec<InputEvent>,
    accumulator: Duration,
    last_instant: Instant,
    /// `RESTRIKE_DEBUG_LOG=<path>`: per-tick state/input/transcript log for
    /// interactive-bug forensics. `None` in normal play.
    debug_log: Option<std::fs::File>,
    debug_tick: u64,
    debug_last_probe: String,
    /// Where `SaveLoadRequest`s are fulfilled (module doc).
    saves_dir: PathBuf,
    /// The PRNG seed a fresh import gets (the original save format carries
    /// none) — the same one the boot import used, so re-importing a slot
    /// mid-session reproduces the launch-time stream.
    seed: u32,
}

impl App {
    fn new(engine: Engine, square_pixels: bool, saves_dir: PathBuf, seed: u32) -> Self {
        let debug_log = std::env::var_os("RESTRIKE_DEBUG_LOG")
            .map(|p| std::fs::File::create(&p).expect("RESTRIKE_DEBUG_LOG path must be writable"));
        App {
            engine,
            square_pixels,
            window: None,
            surface: None,
            rgba: vec![[0, 0, 0, 0xFF]; WIDTH * HEIGHT],
            last_serial: 0,
            pending_input: Vec::new(),
            accumulator: Duration::ZERO,
            last_instant: Instant::now(),
            debug_log,
            debug_tick: 0,
            debug_last_probe: String::new(),
            saves_dir,
            seed,
        }
    }

    /// Fulfills this tick's `SaveLoadRequest`, if there is one, and tells the
    /// player what happened.
    ///
    /// The fulfilment itself is `debug_log::fulfill_pending_io` — the same call
    /// `restrike replay` and the forensics example make, deliberately, so a
    /// replay of a recorded session performs the host's part exactly as the
    /// desktop did. What belongs to *this* frontend is the two things a
    /// headless replay has no use for: the game-speed env knob (a Load/Import
    /// replaces the whole engine, and an import starts at the boot default),
    /// and the on-screen verdict.
    fn fulfill_io(&mut self) {
        // ★ Roll-credits slice 9c: the character-file half — creation's
        // `Save <name>?`, `Remove`'s `SavePlayer`, and `Add`'s load. Same
        // shape, same place, and its verdict reaches the player too.
        if let Some(notice) =
            debug_log::fulfill_pending_char_file(&mut self.engine, &self.saves_dir)
        {
            eprintln!("restrike-desktop: {notice} ({})", self.saves_dir.display());
            if let Some(log) = &mut self.debug_log {
                use std::io::Write;
                let _ = writeln!(log, "io: {notice}");
            }
            self.engine.report_host_notice(notice);
        }
        let Some((request, result)) =
            debug_log::fulfill_pending_io(&mut self.engine, &self.saves_dir, self.seed)
        else {
            return;
        };
        use gbx_engine::saveload::SaveLoadRequest as Req;
        let (verb, letter) = match request {
            Req::Save(l) => ("saved to", l),
            Req::Load(l) => ("loaded", l),
            Req::ImportOriginal(l) => ("imported", l),
        };
        apply_game_speed(&mut self.engine);
        let notice = match result {
            Ok(()) => format!("Slot {letter} {verb}."),
            Err(err) => format!("Slot {letter}: {}", describe_slot_error(&err)),
        };
        eprintln!("restrike-desktop: {notice} ({})", self.saves_dir.display());
        if let Some(log) = &mut self.debug_log {
            use std::io::Write;
            let _ = writeln!(log, "io: {notice}");
        }
        self.engine.report_host_notice(notice);
    }

    /// Advances the accumulator and calls `tick` at `TICK_HZ` regardless of
    /// display refresh, draining `pending_input` into the tick it belongs
    /// to. Requests a redraw only if a tick actually ran.
    fn advance(&mut self, window: &Window) {
        let now = Instant::now();
        self.accumulator += now.duration_since(self.last_instant);
        self.last_instant = now;

        let mut ticked = false;
        while self.accumulator >= TICK {
            let sent = std::mem::take(&mut self.pending_input);
            {
                let frame = self.engine.tick(&sent);
                if frame.serial != self.last_serial {
                    self.last_serial = frame.serial;
                    for (dst, &idx) in self.rgba.iter_mut().zip(frame.pixels.iter()) {
                        let [r, g, b] = frame.palette[idx as usize];
                        *dst = [r, g, b, 0xFF];
                    }
                }
            }
            // D8's other half: the tick core deposited a save/load request,
            // the host performs it. Between ticks, never during one.
            self.fulfill_io();
            // ★ `seg043.print_and_exit()` (slice 9b): `Exit to DOS`, or copy
            // protection's third failure. The core cannot end the process, so
            // it raises the request and this frontend honors it.
            if self.engine.quit_requested() {
                eprintln!("restrike-desktop: exit requested — goodbye");
                std::process::exit(0);
            }
            self.debug_tick += 1;
            if let Some(log) = &mut self.debug_log {
                let probe = self.engine.probe();
                let transcript = self.engine.take_transcript();
                if !sent.is_empty() || probe != self.debug_last_probe || !transcript.is_empty() {
                    use std::io::Write;
                    let _ = writeln!(log, "tick {} | sent {sent:?} | {probe}", self.debug_tick);
                    for entry in transcript {
                        let _ = writeln!(log, "    {entry:?}");
                    }
                    self.debug_last_probe = probe;
                }
            }
            ticked = true;
            self.accumulator -= TICK;
        }
        if ticked {
            window.request_redraw();
        }
    }

    /// Expands `self.rgba` onto the softbuffer surface at the current D-UI6
    /// scale, letterboxed on black.
    fn present(&mut self) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else {
            return;
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        surface.resize(w, h).expect("failed to resize the surface");

        let mut buffer = surface
            .buffer_mut()
            .expect("failed to lock the surface buffer");
        buffer.fill(0);
        let s = scale::compute(w.get(), h.get(), self.square_pixels);
        for sy in 0..HEIGHT as u32 {
            let dst_y0 = s.offset_y + sy * s.scale_y;
            for sx in 0..WIDTH as u32 {
                let [r, g, b, _] = self.rgba[sy as usize * WIDTH + sx as usize];
                let pixel = (r as u32) << 16 | (g as u32) << 8 | b as u32;
                let dst_x0 = s.offset_x + sx * s.scale_x;
                for dy in 0..s.scale_y {
                    let row = (dst_y0 + dy) as usize * w.get() as usize;
                    for dx in 0..s.scale_x {
                        buffer[row + (dst_x0 + dx) as usize] = pixel;
                    }
                }
            }
        }
        buffer
            .present()
            .expect("failed to present the surface buffer");
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title(self.engine.title())
            .with_inner_size(winit::dpi::LogicalSize::new(
                (WIDTH * 5) as u32,
                (HEIGHT * 6) as u32,
            ));
        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create the window"),
        );
        let context =
            Context::new(window.clone()).expect("failed to create the softbuffer context");
        let surface = Surface::new(&context, window.clone())
            .expect("failed to create the softbuffer surface");
        self.window = Some(window);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.present(),
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(log) = &mut self.debug_log {
                    use std::io::Write;
                    let _ = writeln!(
                        log,
                        "key: {:?} state={:?} repeat={}",
                        event.logical_key, event.state, event.repeat
                    );
                }
                if event.state == ElementState::Pressed && !event.repeat {
                    if let Some(mapped) = keymap::map_key(&event) {
                        self.pending_input.push(mapped);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.advance(&window);
        let next = self.last_instant + TICK.saturating_sub(self.accumulator);
        event_loop.set_control_flow(ControlFlow::WaitUntil(next));
    }
}
