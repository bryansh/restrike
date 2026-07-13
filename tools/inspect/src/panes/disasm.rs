//! The disassembly pane (task deliverable 3): a picker over `ECL*.DAX`
//! blocks -> the flow-following listing (`gbx_vm::disasm`) with hazard
//! annotations visible, plus `Summary`'s per-opcode/data-region stats.
//!
//! Selection/copy/goto ergonomics pass: the listing renders as a selectable
//! (drag-select + Cmd-C) monospace text box instead of the old
//! `.interactive(false)` read-only `TextEdit` (which explicitly disabled
//! all selection — the concrete bug behind "had to screenshot the
//! listing"), a "Copy listing" button copies the whole plain-text blob, and
//! a goto-address box (`0x8295` or `8295`) scrolls to and highlights that
//! address's line by setting the `TextEdit`'s own cursor/selection range —
//! the exact field-find workflow the task brief names.

use eframe::egui::{self, Color32};
use gbx_formats::game_data::GameData;
use gbx_vm::dialect::{Dialect, COTAB, COTAB_VECTOR_COUNT};
use gbx_vm::{decode, disassemble};

use crate::viewmodel::goto;
use crate::widgets;

#[derive(Default)]
pub struct DisasmState {
    selected_file: Option<String>,
    selected_block: Option<u8>,
    goto_input: String,
    pending_scroll_addr: Option<u16>,
    goto_not_found: bool,
}

impl DisasmState {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &GameData) {
        egui::SidePanel::left("ecl_block_picker")
            .resizable(true)
            .default_width(240.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.picker(ui, data);
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            match (self.selected_file.clone(), self.selected_block) {
                (Some(file), Some(block)) => self.show_listing(ui, data, &file, block),
                _ => {
                    ui.label("Select an ECL*.DAX block from the list on the left.");
                }
            }
        });
    }

    fn picker(&mut self, ui: &mut egui::Ui, data: &GameData) {
        for file in data
            .file_names()
            .filter(|f| f.to_ascii_uppercase().starts_with("ECL"))
            .map(str::to_string)
            .collect::<Vec<_>>()
        {
            let entries = data
                .archive(&file)
                .map(|a| a.entries().to_vec())
                .unwrap_or_default();
            egui::CollapsingHeader::new(format!("{file} ({} blocks)", entries.len()))
                .id_salt(&file)
                .show(ui, |ui| {
                    for entry in &entries {
                        let selected = self.selected_file.as_deref() == Some(file.as_str())
                            && self.selected_block == Some(entry.id);
                        if ui
                            .selectable_label(selected, format!("block {}", entry.id))
                            .clicked()
                        {
                            self.selected_file = Some(file.clone());
                            self.selected_block = Some(entry.id);
                        }
                    }
                });
        }
    }

    fn show_listing(&mut self, ui: &mut egui::Ui, data: &GameData, file: &str, block_id: u8) {
        ui.heading(format!("{file} block {block_id}"));
        let raw = match data.block(file, block_id) {
            Ok(bytes) => bytes,
            Err(err) => {
                ui.colored_label(Color32::RED, format!("failed to extract block: {err:?}"));
                return;
            }
        };
        let payload = gbx_formats::dax::ecl_block_payload(&raw);
        let payload = &payload[..payload.len().min(decode::ECL_BLOCK_SIZE)];
        let block = decode::BlockBytes::from_bytes(payload);

        let dialect: &Dialect = &COTAB;
        let (vectors, _) = decode::read_header_vectors(&block, COTAB_VECTOR_COUNT);
        let mut entries: Vec<u16> = vectors.into_iter().flatten().collect();
        if entries.is_empty() {
            entries.push(decode::ECL_BLOCK_BASE);
        }
        ui.label(format!(
            "entry points (header vectors): {}",
            entries
                .iter()
                .map(|a| format!("{a:#06X}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));

        let listing = disassemble(&block, dialect, &entries);
        let summary = listing.summary();
        ui.separator();
        ui.label(format!(
            "{} opcode(s) reached, {} hazard(s), {} data region(s)",
            summary.opcode_reached_counts.len(),
            summary.hazards.len(),
            summary.data_region_spans.len()
        ));

        if !summary.hazards.is_empty() {
            egui::CollapsingHeader::new(format!("hazards ({})", summary.hazards.len()))
                .default_open(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("hazards")
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for hazard in &summary.hazards {
                                ui.colored_label(
                                    Color32::from_rgb(255, 180, 0),
                                    format!("{hazard:?}"),
                                );
                            }
                        });
                });
        }

        egui::CollapsingHeader::new("opcode counts")
            .default_open(false)
            .show(ui, |ui| {
                for (op, count) in &summary.opcode_reached_counts {
                    let name = dialect.lookup(*op).map(|i| i.name).unwrap_or("<unknown>");
                    ui.monospace(format!("{op:#04X}  {name:<20}  x{count}"));
                }
            });

        ui.separator();
        let text = listing.render(dialect);

        ui.horizontal(|ui| {
            widgets::copy_text_button(ui, "Copy listing", || text.clone());
            ui.label("goto address:");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.goto_input)
                    .desired_width(80.0)
                    .hint_text("0x8295"),
            );
            let submitted = ui.button("Go").clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if submitted {
                match goto::parse_address(&self.goto_input) {
                    Some(addr) => {
                        self.pending_scroll_addr = Some(addr);
                        self.goto_not_found = false;
                    }
                    None => self.goto_not_found = true,
                }
            }
            if self.goto_not_found {
                ui.colored_label(Color32::RED, "not a valid/reachable address");
            }
        });

        let id = ui.make_persistent_id(("disasm_listing", file, block_id));
        egui::ScrollArea::both()
            .id_salt("listing")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut buf = text.clone();
                let output = egui::TextEdit::multiline(&mut buf)
                    .id(id)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .show(ui);

                if let Some(addr) = self.pending_scroll_addr.take() {
                    match goto::find_line_for_address(&text, addr) {
                        Some(line_idx) => {
                            let start = goto::char_offset_for_line(&text, line_idx);
                            let line_len = text.lines().nth(line_idx).map(str::len).unwrap_or(0);
                            let mut state = output.state;
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(start),
                                    egui::text::CCursor::new(start + line_len),
                                )));
                            state.store(ui.ctx(), id);

                            let cursor =
                                output.galley.from_ccursor(egui::text::CCursor::new(start));
                            let rect = output
                                .galley
                                .pos_from_cursor(&cursor)
                                .translate(output.galley_pos.to_vec2());
                            ui.scroll_to_rect(rect, Some(egui::Align::Center));
                        }
                        None => self.goto_not_found = true,
                    }
                }
            });
    }
}
