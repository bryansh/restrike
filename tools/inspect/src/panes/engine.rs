//! The live engine pane (task deliverable 4, the heart of v0): an embedded
//! `Engine` over the same `GameData`, driven by inspector-owned ticks —
//! run/pause/step/speed controls, a zoomable framebuffer texture, keyboard
//! passthrough while the pane has focus, `Shell`/`VmPhase` + engine-state
//! display, and the ScriptMemory watch (the unknown-access log front and
//! center, joined against the raw store's current values), halt records,
//! and the service-call log tail.
//!
//! **Focus model:** "while the pane has focus" (task brief) is implemented
//! as an explicit capture toggle rather than egui's low-level widget focus
//! chain — a checkbox the user switches on to play the engine and off to
//! type into the ScriptMemory filter box below without leaking keystrokes.
//! Simpler and more robust than fighting egui's focus system for a debug
//! tool where exact input-routing polish isn't the point.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use gbx_engine::engine::Engine;
use gbx_engine::input::InputEvent;
use gbx_engine::vmhost::{AccessKind, UnknownAccess, VmMemoryState};
use gbx_formats::game_data::GameData;

use crate::keymap;
use crate::viewmodel::{call_table, copy, halt_table, log_table};
use crate::widgets;

pub struct EnginePaneState {
    engine: Option<Engine>,
    seed: u64,
    running: bool,
    speed: u32,
    capture_keyboard: bool,
    texture: Option<TextureHandle>,
    /// The same RGBA bytes just uploaded to `texture`, kept around for the
    /// framebuffer's "Copy image"/"Save .ppm" buttons (task brief
    /// deliverable 3) — `show_framebuffer` runs after the texture is
    /// (re)built each tick and has no other way to get the raw pixels back.
    last_rgba: Vec<u8>,
    last_serial: u64,
    tick_count: u64,
    /// Parallel to `engine.vm_memory().unknown_log.entries()`: the tick each
    /// entry first appeared at (the log itself carries no timestamp, per
    /// `viewmodel::log_table`'s doc comment — this pane is the one place
    /// that can observe "new since last tick").
    log_first_seen: Vec<u64>,
    log_kind_filter: Option<AccessKind>,
    log_addr_filter: String,
    /// Substring filter on the service-call log (task brief deliverable 4;
    /// [`log_kind_filter`]/[`log_addr_filter`] above are the unknown-access
    /// log's own existing filter).
    call_filter: String,
    /// Shared directory every "Save .ppm" button in this pane writes into.
    save_dir: String,
}

impl Default for EnginePaneState {
    fn default() -> Self {
        EnginePaneState {
            engine: None,
            seed: 1,
            running: false,
            speed: 1,
            capture_keyboard: false,
            texture: None,
            last_rgba: Vec::new(),
            last_serial: 0,
            tick_count: 0,
            log_first_seen: Vec::new(),
            log_kind_filter: None,
            log_addr_filter: String::new(),
            call_filter: String::new(),
            save_dir: ".".to_string(),
        }
    }
}

impl EnginePaneState {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &GameData) {
        if self.engine.is_none() {
            ui.horizontal(|ui| {
                ui.label("seed:");
                ui.add(egui::DragValue::new(&mut self.seed));
                if ui.button("Boot engine").clicked() {
                    match Engine::new(data.clone(), self.seed) {
                        Ok(e) => self.engine = Some(e),
                        Err(err) => eprintln!("restrike-inspect: engine boot failed: {err:?}"),
                    }
                }
            });
            ui.label("No engine booted yet.");
            return;
        }

        let mut reset = false;
        let mut step = false;
        ui.horizontal(|ui| {
            if ui
                .button(if self.running { "Pause" } else { "Run" })
                .clicked()
            {
                self.running = !self.running;
            }
            step = ui
                .add_enabled(!self.running, egui::Button::new("Step"))
                .clicked();
            ui.label("speed (ticks/frame):");
            ui.add(egui::Slider::new(&mut self.speed, 1..=60));
            reset = ui.button("Reset").clicked();
            ui.checkbox(&mut self.capture_keyboard, "capture keyboard");
            ui.label(format!("tick {}", self.tick_count));
        });
        if reset {
            *self = EnginePaneState {
                seed: self.seed,
                ..Default::default()
            };
            return;
        }

        let mut input_this_frame: Vec<InputEvent> = Vec::new();
        if self.capture_keyboard {
            ui.input(|i| {
                for event in &i.events {
                    match event {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            repeat: false,
                            ..
                        } => {
                            if let Some(ev) = keymap::map_key(*key) {
                                input_this_frame.push(ev);
                            }
                        }
                        egui::Event::Text(text) => {
                            if let Some(ev) = keymap::map_text(text) {
                                input_this_frame.push(ev);
                            }
                        }
                        _ => {}
                    }
                }
            });
        }

        if self.running {
            for i in 0..self.speed {
                let input = if i == 0 {
                    input_this_frame.as_slice()
                } else {
                    &[]
                };
                self.advance_one_tick(ui.ctx(), input);
            }
            ui.ctx().request_repaint();
        } else if step {
            self.advance_one_tick(ui.ctx(), &input_this_frame);
        }

        egui::SidePanel::right("engine_state")
            .resizable(true)
            .default_width(380.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.show_state(ui);
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.show_framebuffer(ui);
        });
    }

    fn advance_one_tick(&mut self, ctx: &egui::Context, input: &[InputEvent]) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        self.tick_count += 1;
        let frame = engine.tick(input);
        if frame.serial != self.last_serial {
            self.last_serial = frame.serial;
            let mut rgba = Vec::with_capacity(frame.pixels.len() * 4);
            for &idx in frame.pixels {
                let [r, g, b] = frame.palette[idx as usize];
                rgba.extend_from_slice(&[r, g, b, 0xFF]);
            }
            let image = ColorImage::from_rgba_unmultiplied(
                [
                    gbx_engine::framebuffer::WIDTH,
                    gbx_engine::framebuffer::HEIGHT,
                ],
                &rgba,
            );
            self.texture =
                Some(ctx.load_texture("engine_framebuffer", image, TextureOptions::NEAREST));
            self.last_rgba = rgba;
        }
        let entries_len = engine.vm_memory().unknown_log.entries().len();
        while self.log_first_seen.len() < entries_len {
            self.log_first_seen.push(self.tick_count);
        }
    }

    fn show_framebuffer(&mut self, ui: &mut egui::Ui) {
        let Some(texture) = &self.texture else {
            ui.label("(no frame yet — press Step or Run)");
            return;
        };
        let size = egui::vec2(
            gbx_engine::framebuffer::WIDTH as f32 * 3.0,
            gbx_engine::framebuffer::HEIGHT as f32 * 3.0,
        );
        ui.add(egui::Image::new(texture).fit_to_exact_size(size));
        ui.label(if self.capture_keyboard {
            "keyboard capture ON — typing plays the engine"
        } else {
            "keyboard capture OFF — check the box above to play"
        });

        ui.horizontal(|ui| {
            ui.label("save directory:");
            ui.text_edit_singleline(&mut self.save_dir);
        });
        widgets::image_actions(
            ui,
            "framebuffer",
            gbx_engine::framebuffer::WIDTH,
            gbx_engine::framebuffer::HEIGHT,
            &self.last_rgba,
            &self.save_dir,
            &["framebuffer", &format!("tick{}", self.tick_count)],
        );
    }

    fn show_state(&mut self, ui: &mut egui::Ui) {
        let Some(engine) = &self.engine else { return };

        ui.heading("Shell / VmPhase");
        let shell_json = serde_json::to_string_pretty(engine.shell())
            .unwrap_or_else(|e| format!("<serialize error: {e}>"));
        widgets::copy_text_button(ui, "Copy JSON", || shell_json.clone());
        egui::ScrollArea::vertical()
            .id_salt("shell_json")
            .max_height(220.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(&shell_json).monospace()).selectable(true),
                );
            });

        ui.separator();
        ui.heading("Engine state");
        let state = engine.state();
        let (hh, mm) = state.clock.hh_mm();
        ui.monospace(format!("pos: {:?}", state.pos));
        ui.monospace(format!("facing: {:?}", state.facing));
        ui.monospace(format!(
            "search_flags: {:#04b}  area_map_shown: {}  area_view_allowed: {}",
            state.search_flags, state.area_map_shown, state.area_view_allowed
        ));
        ui.monospace(format!("clock: {hh:02}:{mm:02}"));
        ui.monospace(format!(
            "ecl_block_id: {}  last_ecl_block_id: {}",
            state.ecl_block_id, state.last_ecl_block_id
        ));
        ui.monospace(format!(
            "selected_player: {}  last_selected_player: {}",
            state.selected_player, state.last_selected_player
        ));
        ui.monospace(format!(
            "chained: {}  party_killed: {}  game_state: {:?}",
            state.chained, state.party_killed, state.game_state
        ));

        ui.separator();
        ui.heading("VM memory flags / resident assets");
        let vm = engine.vm_memory();
        ui.monospace(format!(
            "byte_1ee91: {}  byte_1ee94: {}  position_changed: {}  sprite_changed: {}  \
             can_draw_bigpic: {}",
            vm.byte_1ee91,
            vm.byte_1ee94,
            vm.position_changed,
            vm.sprite_changed,
            vm.can_draw_bigpic
        ));
        ui.monospace(format!(
            "3d map block: {:?}  bigpic block: {:?}  walldefs: {:?}",
            vm.assets.map_3d_block, vm.assets.bigpic_block, vm.assets.walldefs
        ));
        widgets::copy_text_button(ui, "Copy summary (key=value)", || {
            copy::to_key_value(&[
                ("pos", format!("{:?}", state.pos)),
                ("facing", format!("{:?}", state.facing)),
                ("search_flags", format!("{:#04b}", state.search_flags)),
                ("area_map_shown", state.area_map_shown.to_string()),
                ("area_view_allowed", state.area_view_allowed.to_string()),
                ("clock", format!("{hh:02}:{mm:02}")),
                ("ecl_block_id", state.ecl_block_id.to_string()),
                ("last_ecl_block_id", state.last_ecl_block_id.to_string()),
                ("selected_player", state.selected_player.to_string()),
                (
                    "last_selected_player",
                    state.last_selected_player.to_string(),
                ),
                ("chained", state.chained.to_string()),
                ("party_killed", state.party_killed.to_string()),
                ("game_state", format!("{:?}", state.game_state)),
                ("byte_1ee91", vm.byte_1ee91.to_string()),
                ("byte_1ee94", vm.byte_1ee94.to_string()),
                ("position_changed", vm.position_changed.to_string()),
                ("sprite_changed", vm.sprite_changed.to_string()),
                ("can_draw_bigpic", vm.can_draw_bigpic.to_string()),
                ("map_3d_block", format!("{:?}", vm.assets.map_3d_block)),
                ("bigpic_block", format!("{:?}", vm.assets.bigpic_block)),
                ("walldefs", format!("{:?}", vm.assets.walldefs)),
            ])
        });

        ui.separator();
        ui.heading(format!("Halt records ({})", vm.halts.len()));
        widgets::copy_text_button(ui, "Copy halts (TSV)", || {
            let rows: Vec<Vec<String>> = vm.halts.iter().map(halt_table::to_tsv_row).collect();
            copy::to_tsv(&halt_table::TSV_HEADERS, &rows)
        });
        egui::ScrollArea::vertical()
            .id_salt("halts")
            .max_height(120.0)
            .show(ui, |ui| {
                for halt in vm.halts.iter().rev().take(50) {
                    ui.colored_label(
                        Color32::RED,
                        format!(
                            "pc={:#06X} op={:#04X} {}",
                            halt.pc, halt.opcode, halt.description
                        ),
                    );
                }
            });

        ui.separator();
        ui.heading(format!("Service-call log tail ({} total)", vm.calls.len()));
        ui.horizontal(|ui| {
            ui.label("filter:");
            ui.text_edit_singleline(&mut self.call_filter);
            widgets::copy_text_button(ui, "Copy calls (TSV)", || {
                let filtered = call_table::filter_calls(&vm.calls, &self.call_filter);
                let rows: Vec<Vec<String>> = filtered
                    .iter()
                    .enumerate()
                    .map(|(i, call)| call_table::to_tsv_row(i, call))
                    .collect();
                copy::to_tsv(&call_table::TSV_HEADERS, &rows)
            });
        });
        let filtered_calls = call_table::filter_calls(&vm.calls, &self.call_filter);
        egui::ScrollArea::vertical()
            .id_salt("calls")
            .max_height(150.0)
            .show(ui, |ui| {
                for call in filtered_calls.iter().rev().take(50) {
                    ui.monospace(format!("{call:?}"));
                }
            });

        ui.separator();
        ui.heading(format!(
            "ScriptMemory unknown-access log ({} entries)",
            vm.unknown_log.entries().len()
        ));
        ui.horizontal(|ui| {
            ui.label("filter addr:");
            ui.text_edit_singleline(&mut self.log_addr_filter);
            egui::ComboBox::from_label("kind")
                .selected_text(
                    self.log_kind_filter
                        .map(log_table::kind_label)
                        .unwrap_or("any"),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.log_kind_filter, None, "any");
                    for kind in [
                        AccessKind::Read,
                        AccessKind::Write,
                        AccessKind::ReadByte,
                        AccessKind::WriteByte,
                        AccessKind::ReadString,
                        AccessKind::WriteString,
                    ] {
                        ui.selectable_value(
                            &mut self.log_kind_filter,
                            Some(kind),
                            log_table::kind_label(kind),
                        );
                    }
                });
        });

        let entries = vm.unknown_log.entries();
        let filtered =
            log_table::filter_entries(entries, self.log_kind_filter, &self.log_addr_filter);
        let filtered_idx: Vec<usize> = filtered.iter().map(|e| index_of(entries, e)).collect();
        widgets::copy_text_button(ui, "Copy log (TSV)", || {
            let rows: Vec<Vec<String>> = filtered_idx
                .iter()
                .map(|&idx| {
                    let entry = &entries[idx];
                    let first_seen = self.log_first_seen.get(idx).copied();
                    let current_value = current_value_label(vm, entry);
                    log_table::to_tsv_row(entry, first_seen, &current_value)
                })
                .collect();
            copy::to_tsv(&log_table::TSV_HEADERS, &rows)
        });
        egui::ScrollArea::vertical()
            .id_salt("unknown_log")
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("unknown_log_grid")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("addr");
                        ui.strong("kind");
                        ui.strong("origin pc");
                        ui.strong("first seen");
                        ui.strong("current value");
                        ui.end_row();
                        for idx in filtered_idx {
                            let entry = &entries[idx];
                            let first_seen = self.log_first_seen.get(idx).copied();
                            let row = log_table::format_row(entry, first_seen);
                            ui.monospace(&row.addr_hex);
                            ui.monospace(row.kind);
                            ui.monospace(&row.origin_pc_hex);
                            ui.monospace(
                                row.first_seen_tick
                                    .map(|t| t.to_string())
                                    .unwrap_or_else(|| "?".to_string()),
                            );
                            ui.monospace(current_value_label(vm, entry));
                            ui.end_row();
                        }
                    });
            });
    }
}

/// Index of `target` within `entries` by address identity (a plain linear
/// scan — the log is capped by real access-site diversity, never large
/// enough in practice for this to matter for a debug tool).
fn index_of(entries: &[UnknownAccess], target: &UnknownAccess) -> usize {
    entries
        .iter()
        .position(|e| e.addr == target.addr && e.kind == target.kind)
        .unwrap_or(0)
}

/// Looks up the raw store's current value for one log entry, by access
/// width — the "raw-store contents" half of the ScriptMemory watch (task
/// deliverable 4), joined onto the unknown-access log rather than needing a
/// separate full-store enumeration getter (`vmhost.rs` intentionally
/// exposes only point lookups, D-UI8's minimal-getter seam).
fn current_value_label(vm: &VmMemoryState, entry: &UnknownAccess) -> String {
    match entry.kind {
        AccessKind::Read | AccessKind::Write => vm
            .raw_word(entry.addr)
            .map(|v| format!("{v:#06X}"))
            .unwrap_or_else(|| "?".to_string()),
        AccessKind::ReadByte | AccessKind::WriteByte => vm
            .raw_byte(entry.addr)
            .map(|v| format!("{v:#04X}"))
            .unwrap_or_else(|| "?".to_string()),
        AccessKind::ReadString | AccessKind::WriteString => vm
            .raw_string(entry.addr)
            .map(|s| String::from_utf8_lossy(&s.0).into_owned())
            .unwrap_or_else(|| "?".to_string()),
    }
}
