//! `restrike watch <CAPTURE> [--data DIR] [--turbo N] [--frames N]` — the M6a
//! reel, headless (`docs/design/combat-visualizer.md` D-CV1 item 2).
//!
//! The same `Engine::new_reel` the desktop `--watch` flag opens, ticked to
//! completion with nothing presenting the frames. It is the reel's terminal
//! counterpart: it answers "does this capture play, all the way, with draw
//! equality holding?" without a window — which is exactly what you want when
//! triaging a capture over ssh, and what the local-tier reel smoke automates
//! across all fifteen.
//!
//! A capture divergence panics inside the engine with `h4_replay`'s diagnostic
//! (the reel's whole point: it must not scroll past), so a clean exit here is
//! the same statement the frontier guard makes — with pixels having been drawn
//! for every beat along the way.
//!
//! `--dump-at N` writes that frame as a `.ppm` under `--out-dir` (default the
//! system temp dir, never the repo — D10, since a reel frame is real art). It is
//! how you eyeball the reel without a window; `restrike compare` takes the same
//! `.ppm` against a DOSBox screenshot.

use std::path::PathBuf;
use std::process::ExitCode;

use gbx_engine::engine::{Engine, Frame};
use gbx_formats::game_data::load_dir;
use gbx_oracle::replay;

/// Ticks the reel to its end, or until `--frames N` is reached.
pub fn cmd_watch(args: Vec<String>) -> ExitCode {
    let mut capture: Option<PathBuf> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut turbo: u32 = 1;
    let mut max_frames: Option<u64> = None;
    let mut dump_at: Vec<u64> = Vec::new();
    let mut out_dir: Option<PathBuf> = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--data" => match iter.next() {
                Some(d) => data_dir = Some(PathBuf::from(d)),
                None => {
                    eprintln!("restrike: --data requires a DIR argument");
                    return ExitCode::FAILURE;
                }
            },
            "--turbo" => match iter.next().and_then(|v| v.parse::<u32>().ok()) {
                Some(n) => turbo = n.max(1),
                None => {
                    eprintln!("restrike: --turbo requires a positive integer");
                    return ExitCode::FAILURE;
                }
            },
            "--frames" => match iter.next().and_then(|v| v.parse::<u64>().ok()) {
                Some(n) => max_frames = Some(n),
                None => {
                    eprintln!("restrike: --frames requires a positive integer");
                    return ExitCode::FAILURE;
                }
            },
            "--dump-at" => match iter.next().and_then(|v| v.parse::<u64>().ok()) {
                Some(n) => dump_at.push(n),
                None => {
                    eprintln!("restrike: --dump-at requires a frame number");
                    return ExitCode::FAILURE;
                }
            },
            "--out-dir" => match iter.next() {
                Some(d) => out_dir = Some(PathBuf::from(d)),
                None => {
                    eprintln!("restrike: --out-dir requires a DIR argument");
                    return ExitCode::FAILURE;
                }
            },
            other if other.starts_with("--") => {
                eprintln!("restrike: unknown watch flag '{other}'");
                return ExitCode::FAILURE;
            }
            other => capture = Some(PathBuf::from(other)),
        }
    }

    let Some(capture) = capture else {
        eprintln!("restrike: watch requires a .gbxtrace capture path");
        eprintln!(
            "  usage: restrike watch <CAPTURE> [--data DIR] [--turbo N] [--frames N] \
             [--dump-at FRAME]... [--out-dir DIR]"
        );
        return ExitCode::FAILURE;
    };
    let Some(dir) = data_dir.or_else(|| std::env::var_os("GBX_DATA_DIR").map(PathBuf::from)) else {
        eprintln!("restrike: watch needs the game data — pass --data DIR or set GBX_DATA_DIR");
        return ExitCode::FAILURE;
    };

    let name = capture
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = match std::fs::read_to_string(&capture) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("restrike: cannot read '{}': {e}", capture.display());
            return ExitCode::FAILURE;
        }
    };
    let data = match load_dir(&dir) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "restrike: cannot read the data dir '{}': {e:?}",
                dir.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let mut input = match replay::reel_input_from_capture(&name, &text, replay::load_item_data()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("restrike: {e}");
            return ExitCode::FAILURE;
        }
    };
    input.tick_multiplier = turbo;
    if replay::sidecar_for(&name).is_none() {
        eprintln!(
            "restrike: {name} has no committed sidecar row — using the documented defaults \
             and no monster icon pins"
        );
    }
    let mut engine = match Engine::new_reel(data, input) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("restrike: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut frames = 0u64;
    let mut sounds = 0usize;
    loop {
        let finished = engine.reel_progress().map(|p| p.finished).unwrap_or(true);
        if finished {
            break;
        }
        if max_frames.is_some_and(|m| frames >= m) {
            break;
        }
        let frame = engine.tick(&[]);
        sounds += frame.sounds.len();
        if dump_at.contains(&frames) {
            let dir = out_dir.clone().unwrap_or_else(std::env::temp_dir);
            let path = dir.join(format!("restrike-watch-{name}-{frames:05}.ppm"));
            match write_ppm(&path, &frame) {
                Ok(()) => eprintln!("frame {frames} -> {}", path.display()),
                Err(e) => {
                    eprintln!("restrike: cannot write '{}': {e}", path.display());
                    return ExitCode::FAILURE;
                }
            }
        }
        frames += 1;
    }

    let Some(p) = engine.reel_progress() else {
        eprintln!("restrike: the engine is not in watch mode (internal)");
        return ExitCode::FAILURE;
    };
    println!(
        "{}: {} frames, {} steps, {}/{} draws checked, {sounds} sound cues — {}",
        p.label,
        frames,
        p.steps,
        p.draws_checked,
        p.draws_expected,
        if p.finished {
            "PLAYED TO THE END, draw-equal"
        } else {
            "stopped early (--frames)"
        }
    );
    ExitCode::SUCCESS
}

/// D10: `.ppm` dumps land wherever `--out-dir` says (default the system temp
/// dir) and never in the repo — a reel frame is real game art.
fn write_ppm(path: &std::path::Path, frame: &Frame<'_>) -> std::io::Result<()> {
    use std::io::Write;
    let (w, h) = (
        gbx_engine::framebuffer::WIDTH,
        gbx_engine::framebuffer::HEIGHT,
    );
    let mut out = Vec::with_capacity(32 + w * h * 3);
    out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for &idx in frame.pixels {
        out.extend_from_slice(&frame.palette[idx as usize]);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::File::create(path)?.write_all(&out)
}
