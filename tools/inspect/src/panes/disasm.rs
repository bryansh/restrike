//! The disassembly pane (task deliverable 3): a picker over `ECL*.DAX`
//! blocks -> the flow-following listing (`gbx_vm::disasm`) with hazard
//! annotations visible, plus `Summary`'s per-opcode/data-region stats.

use eframe::egui::{self, Color32};
use gbx_formats::game_data::GameData;
use gbx_vm::dialect::{Dialect, COTAB, COTAB_VECTOR_COUNT};
use gbx_vm::{decode, disassemble};

#[derive(Default)]
pub struct DisasmState {
    selected_file: Option<String>,
    selected_block: Option<u8>,
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
        let mut text = listing.render(dialect);
        egui::ScrollArea::vertical()
            .id_salt("listing")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });
    }
}
