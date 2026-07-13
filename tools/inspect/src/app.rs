//! The top-level `eframe::App`: tab switching between the three D-UI8
//! panes, all sharing the one loaded `GameData`.

use eframe::egui;
use gbx_formats::game_data::GameData;

use crate::panes::{
    disasm::DisasmState, engine::EnginePaneState, resource_browser::ResourceBrowserState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    ResourceBrowser,
    Disasm,
    Engine,
}

pub struct InspectApp {
    data: GameData,
    pane: Pane,
    resource_browser: ResourceBrowserState,
    disasm: DisasmState,
    engine: EnginePaneState,
}

impl InspectApp {
    pub fn new(data: GameData) -> Self {
        InspectApp {
            data,
            pane: Pane::ResourceBrowser,
            resource_browser: ResourceBrowserState::default(),
            disasm: DisasmState::default(),
            engine: EnginePaneState::default(),
        }
    }
}

impl eframe::App for InspectApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Task brief deliverable 1: text selection everywhere. egui 0.29
        // already defaults this to `true`, but setting it explicitly here
        // documents the intent and survives a future egui upgrade that
        // might change that default.
        ctx.style_mut(|style| style.interaction.selectable_labels = true);

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.pane, Pane::ResourceBrowser, "Resource browser");
                ui.selectable_value(&mut self.pane, Pane::Disasm, "Disassembly");
                ui.selectable_value(&mut self.pane, Pane::Engine, "Live engine");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.pane {
            Pane::ResourceBrowser => self.resource_browser.ui(ui, &self.data),
            Pane::Disasm => self.disasm.ui(ui, &self.data),
            Pane::Engine => self.engine.ui(ui, &self.data),
        });
    }
}
