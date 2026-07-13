//! Shared egui widgets for the copy/paste ergonomics pass (D-UI8 polish):
//! a one-click clipboard-text copy button, and the "Copy image"/"Save
//! .ppm" pair every rendered image gets. Thin — the actual formatting
//! (TSV/key=value/PPM bytes) lives in `crate::viewmodel`, pure and unit
//! tested; this module is just the egui/arboard glue, not itself tested.

use eframe::egui;

use crate::viewmodel::ppm;

/// A button that copies `text` to the OS clipboard via egui's own output
/// channel (`Context::copy_text`) — routed through eframe's clipboard
/// backend, no extra dependency needed for plain text (unlike image copy,
/// which egui's `Output` has no channel for — see [`image_actions`]).
pub fn copy_text_button(ui: &mut egui::Ui, label: &str, text: impl FnOnce() -> String) {
    if ui.button(label).clicked() {
        ui.ctx().copy_text(text());
    }
}

/// Renders "Copy image" + "Save .ppm" buttons for one RGBA image at its
/// native pixel size (task brief deliverable 3: "1x pixels; the OS scales
/// pastes fine"). `rgba` is row-major, `width*height*4` bytes. `save_dir`
/// is the pane's shared save-directory field; `name_parts` builds the
/// default filename (block id, item index, etc. — task brief: "default
/// filename with block id"). `id_salt` keeps multiple image-action rows in
/// the same scroll area (e.g. per-item in a multi-item image block) from
/// colliding on button interaction state.
#[allow(clippy::too_many_arguments)]
pub fn image_actions(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    width: usize,
    height: usize,
    rgba: &[u8],
    save_dir: &str,
    name_parts: &[&str],
) {
    ui.push_id(id_salt, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Copy image").clicked() {
                copy_image_to_clipboard(width, height, rgba);
            }
            if ui.button("Save .ppm").clicked() {
                save_ppm(save_dir, name_parts, width, height, rgba);
            }
        });
    });
}

fn copy_image_to_clipboard(width: usize, height: usize, rgba: &[u8]) {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("restrike-inspect: clipboard unavailable: {err}");
            return;
        }
    };
    let image = arboard::ImageData {
        width,
        height,
        bytes: rgba.into(),
    };
    if let Err(err) = clipboard.set_image(image) {
        eprintln!("restrike-inspect: clipboard image copy failed: {err}");
    }
}

fn save_ppm(save_dir: &str, name_parts: &[&str], width: usize, height: usize, rgba: &[u8]) {
    let path = std::path::Path::new(save_dir).join(ppm::ppm_filename(name_parts));
    let bytes = ppm::encode_ppm(width, height, rgba);
    match std::fs::write(&path, &bytes) {
        Ok(()) => eprintln!("restrike-inspect: saved {}", path.display()),
        Err(err) => eprintln!("restrike-inspect: failed to save {}: {err}", path.display()),
    }
}
