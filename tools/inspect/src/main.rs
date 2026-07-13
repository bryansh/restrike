//! `restrike-inspect [DIR]` — the GBC-replacement seed (D-UI8, PLAN.md §2):
//! an eframe/egui app over a `GBX_DATA_DIR` data set. Read-only in v0: a
//! resource browser (DAX block tree -> decoded views), an ECL disassembly
//! pane, and a live engine pane (embedded `Engine`, inspector-owned ticks,
//! ScriptMemory watch front and center). Platform deps (`eframe`/`egui`)
//! live in this crate only — `gbx-engine` stays pure per D-UI8's seam rule.

mod app;
mod keymap;
mod panes;
#[cfg(test)]
mod real_data_smoke;
mod viewmodel;
mod widgets;

use std::env;
use std::path::PathBuf;

fn main() {
    let dir = match resolve_data_dir() {
        Some(dir) => dir,
        None => {
            eprintln!(
                "restrike-inspect: no data directory given and GBX_DATA_DIR is not set\n\n\
                 usage: restrike-inspect [DIR]\n\n\
                 Pass the CotAB data directory as a positional argument, or set \
                 GBX_DATA_DIR."
            );
            std::process::exit(1);
        }
    };
    if !dir.is_dir() {
        eprintln!("restrike-inspect: '{}' is not a directory", dir.display());
        std::process::exit(1);
    }

    let data = match gbx_formats::game_data::load_dir(&dir) {
        Ok(data) => data,
        Err(err) => {
            eprintln!(
                "restrike-inspect: failed to read data directory '{}': {err}",
                dir.display()
            );
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions::default();
    let result = eframe::run_native(
        "restrike-inspect",
        options,
        Box::new(move |_cc| Ok(Box::new(app::InspectApp::new(data)))),
    );
    if let Err(err) = result {
        eprintln!("restrike-inspect: {err}");
        std::process::exit(1);
    }
}

fn resolve_data_dir() -> Option<PathBuf> {
    env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
}
