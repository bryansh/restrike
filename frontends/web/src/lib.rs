//! The web frontend for Restrike (D-UI6): a wasm-bindgen + canvas 2D
//! presenter. Same crate graph as `restrike-desktop`, compiled to
//! `wasm32-unknown-unknown` instead of a native binary. One page, one
//! canvas, keyboard only -- the D8 proof (nothing blocks in `gbx-engine`,
//! not even across a JS event loop), not a product.
//!
//! Data is supplied by the user at runtime (a directory picker or a zip's
//! extracted files, from `index.html`'s JS) and handed to wasm as
//! `(name, bytes)` pairs via [`GameDataBuilder`] -- never bundled into the
//! wasm binary (D10). [`App::frame`] is driven by `requestAnimationFrame`
//! with a millisecond timestamp; it keeps its own accumulator and calls
//! `Engine::tick` at a fixed [`gbx_engine::input::TICK_HZ`], exactly like
//! the desktop frontend's `WaitUntil` loop.

mod keymap;

use gbx_engine::engine::Engine;
use gbx_engine::framebuffer::{HEIGHT, WIDTH};
use gbx_engine::input::{InputEvent, TICK_HZ};
use gbx_formats::game_data::GameData;
use wasm_bindgen::prelude::*;
use wasm_bindgen::Clamped;
use web_sys::{console, CanvasRenderingContext2d, HtmlCanvasElement, ImageData};

/// One tick's worth of game-presentation time, in milliseconds --
/// `performance.now()`'s unit, matching `TICK_HZ` (D-UI1).
const TICK_MS: f64 = 1000.0 / TICK_HZ as f64;

/// Accumulates `(file name, bytes)` pairs from JS (a directory picker or an
/// unzipped archive) ahead of [`App::new`]'s `GameData::from_files` call.
#[wasm_bindgen]
#[derive(Default)]
pub struct GameDataBuilder {
    files: Vec<(String, Vec<u8>)>,
}

#[wasm_bindgen]
impl GameDataBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one file's raw bytes, as read by JS's `File.arrayBuffer()`.
    pub fn add_file(&mut self, name: String, bytes: Vec<u8>) {
        self.files.push((name, bytes));
    }
}

/// D-RP4's boot plumbing: the web frontend has no real verify-report
/// surface yet (the design doc's own placeholder — "the web frontend logs
/// it to the console until a real surface exists"), so this just prints one
/// line per table to the browser console via `console.log`.
fn log_verify_report(engine: &Engine) {
    for (id, status) in &engine.verify_report().entries {
        console::log_1(&JsValue::from_str(&format!("verify: {id}: {status:?}")));
    }
}

/// The tick-driving presenter: owns the `Engine`, the canvas 2D context,
/// and the RGBA expansion buffer `ImageData` is built from each changed
/// frame.
#[wasm_bindgen]
pub struct App {
    engine: Engine,
    ctx: CanvasRenderingContext2d,
    rgba: Vec<u8>,
    last_serial: u64,
    pending_input: Vec<InputEvent>,
    accumulator_ms: f64,
    last_time_ms: Option<f64>,
}

#[wasm_bindgen]
impl App {
    /// Boots the `Engine` from `builder`'s collected files and grabs the
    /// canvas's 2D context. `canvas` must already be sized `320x200`
    /// (`index.html` sets this; CSS handles the D-UI6 display scale
    /// separately, so the backing pixel buffer stays native resolution).
    #[wasm_bindgen(constructor)]
    pub fn new(
        canvas: HtmlCanvasElement,
        builder: GameDataBuilder,
        seed: u32,
    ) -> Result<App, JsValue> {
        let data = GameData::from_files(builder.files);
        let engine = Engine::new(data, seed)
            .map_err(|err| JsValue::from_str(&format!("Engine::new failed to boot: {err:?}")))?;
        log_verify_report(&engine);
        let ctx = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("canvas has no 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;
        Ok(App {
            engine,
            ctx,
            rgba: vec![0u8; WIDTH * HEIGHT * 4],
            last_serial: 0,
            pending_input: Vec::new(),
            accumulator_ms: 0.0,
            last_time_ms: None,
        })
    }

    pub fn title(&self) -> String {
        self.engine.title().to_string()
    }

    /// `index.html`'s `keydown` listener calls this with the browser
    /// `KeyboardEvent`'s `key` (layout-resolved) and `code` (physical,
    /// layout-independent) fields -- see `keymap.rs`.
    pub fn key_down(&mut self, key: &str, code: &str) {
        if let Some(mapped) = keymap::map_key(key, code) {
            self.pending_input.push(mapped);
        }
    }

    /// Drives the fixed-timestep loop from `requestAnimationFrame`'s
    /// `DOMHighResTimeStamp` (`performance.now()`-relative milliseconds):
    /// ticks at `TICK_HZ` regardless of display refresh rate, and
    /// `putImageData`s only when a tick actually changed the frame.
    pub fn frame(&mut self, now_ms: f64) -> Result<(), JsValue> {
        let elapsed = now_ms - self.last_time_ms.unwrap_or(now_ms);
        self.last_time_ms = Some(now_ms);
        self.accumulator_ms += elapsed.max(0.0);

        let mut changed = false;
        while self.accumulator_ms >= TICK_MS {
            let f = self.engine.tick(&self.pending_input);
            self.pending_input.clear();
            if f.serial != self.last_serial {
                self.last_serial = f.serial;
                for (i, &idx) in f.pixels.iter().enumerate() {
                    let [r, g, b] = f.palette[idx as usize];
                    self.rgba[i * 4] = r;
                    self.rgba[i * 4 + 1] = g;
                    self.rgba[i * 4 + 2] = b;
                    self.rgba[i * 4 + 3] = 0xFF;
                }
                changed = true;
            }
            self.accumulator_ms -= TICK_MS;
        }

        if changed {
            let image = ImageData::new_with_u8_clamped_array_and_sh(
                Clamped(&self.rgba),
                WIDTH as u32,
                HEIGHT as u32,
            )?;
            self.ctx.put_image_data(&image, 0.0, 0.0)?;
        }
        Ok(())
    }
}
