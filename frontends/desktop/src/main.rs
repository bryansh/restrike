//! `restrike-desktop [DIR] [--seed N] [--square-pixels] [--watch CAPTURE
//! [--turbo N]]` -- the winit + softbuffer presenter (D-UI6). Loads
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

mod keymap;
mod scale;

use std::env;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gbx_engine::engine::Engine;
use gbx_engine::framebuffer::{HEIGHT, WIDTH};
use gbx_engine::input::{InputEvent, TICK_HZ};
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
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
            other => dir_arg = Some(PathBuf::from(other)),
        }
    }
    let dir = dir_arg
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
        .expect("restrike-desktop: pass a data directory or set GBX_DATA_DIR");
    let data = load_dir(&dir).expect("restrike-desktop: failed to read the data directory");
    let engine = match watch {
        Some(capture) => open_reel(data, &capture, turbo),
        None => {
            Engine::new(data, seed).expect("restrike-desktop: Engine::new failed to boot this data")
        }
    };

    let event_loop = EventLoop::new().expect("failed to create the winit event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(engine, square_pixels);
    event_loop
        .run_app(&mut app)
        .expect("the event loop exited with an error");
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
}

impl App {
    fn new(engine: Engine, square_pixels: bool) -> Self {
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
        }
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
