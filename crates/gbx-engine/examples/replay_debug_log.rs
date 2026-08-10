//! Replay a desktop `RESTRIKE_DEBUG_LOG` headlessly — the forensics
//! round-trip: an interactive bug is logged in play, then reproduced
//! deterministically here (same data, same fixed default seed 1, same input
//! schedule ⇒ the identical run), where it can be instrumented at will.
//!
//! Usage:
//!   GBX_DATA_DIR=~/goldbox-data/cotab cargo run -p gbx-engine \
//!     --example replay_debug_log -- /tmp/restrike-debug.log [max_extra_ticks]
//!
//! Prints the same probe/transcript stream the desktop logger wrote, so the
//! two can be diffed directly.
//!
//! **Save/load replays too** (roll-credits slice 0): the desktop is the host
//! that fulfills the save/load screen's `SaveLoadRequest`, so a replay that
//! didn't do the same would diverge the moment a recorded session saved. It
//! fulfills against a **throwaway copy** of the desktop's saves directory
//! (`<data>/SAVE`) made per run — every slot the live session could see is
//! there, the writes land in the copy, and the player's own saves are never
//! touched by a forensics replay.

use gbx_engine::engine::Engine;
use gbx_engine::input::{ExtKey, InputEvent};
use gbx_formats::game_data::load_dir;

fn parse_event(s: &str) -> Option<InputEvent> {
    let s = s.trim();
    Some(match s {
        "Enter" => InputEvent::Enter,
        "Escape" => InputEvent::Escape,
        "Backspace" => InputEvent::Backspace,
        "Ext(Up)" => InputEvent::Ext(ExtKey::Up),
        "Ext(Down)" => InputEvent::Ext(ExtKey::Down),
        "Ext(Left)" => InputEvent::Ext(ExtKey::Left),
        "Ext(Right)" => InputEvent::Ext(ExtKey::Right),
        "Ext(Home)" => InputEvent::Ext(ExtKey::Home),
        "Ext(End)" => InputEvent::Ext(ExtKey::End),
        _ => {
            if let Some(rest) = s.strip_prefix("Char(") {
                let byte = rest.trim_end_matches(')');
                // Debug prints `Char(98)` (a u8) — accept both that and 'b'.
                if let Ok(n) = byte.parse::<u8>() {
                    return Some(InputEvent::Char(n));
                }
                let ch = byte.trim_matches('\'');
                return ch.bytes().next().map(InputEvent::Char);
            }
            eprintln!("replay: unrecognized event {s:?} — skipped");
            return None;
        }
    })
}

/// Copies every regular file in `from` into a fresh `to` (no recursion — a
/// save directory is flat). A missing source is fine: the replay just has no
/// slots, exactly like a first launch.
fn copy_dir_shallow(from: &std::path::Path, to: &std::path::Path) {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to).expect("replay: saves sandbox must be creatable");
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_file() {
            let _ = std::fs::copy(entry.path(), to.join(entry.file_name()));
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let log_path = args.next().expect("usage: replay_debug_log <debug.log>");
    let extra: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);

    let text = std::fs::read_to_string(&log_path).expect("log readable");
    let mut schedule: Vec<(u64, Vec<InputEvent>)> = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("tick ") else {
            continue;
        };
        let Some((tick, sent)) = rest.split_once(" | sent [") else {
            continue;
        };
        let Some((events, _)) = sent.split_once(']') else {
            continue;
        };
        if events.is_empty() {
            continue;
        }
        let tick: u64 = tick.trim().parse().expect("tick number");
        let events: Vec<InputEvent> = events.split(',').filter_map(parse_event).collect();
        if !events.is_empty() {
            schedule.push((tick, events));
        }
    }
    let last_tick = schedule.last().map(|(t, _)| *t).unwrap_or(0) + extra;
    eprintln!(
        "replay: {} input batches over {} ticks (+{extra} tail)",
        schedule.len(),
        last_tick
    );

    let dir = std::path::PathBuf::from(
        std::env::var_os("GBX_DATA_DIR").expect("GBX_DATA_DIR must be set"),
    );
    let data = load_dir(&dir).expect("data dir readable");
    // The replay must boot the way the desktop that WROTE the log booted:
    // slot-A import is the desktop default; RESTRIKE_REPLAY_BARE=1 replays
    // logs from a `--slot none` (or pre-import-era) desktop.
    let mut engine = if std::env::var_os("RESTRIKE_REPLAY_BARE").is_some() {
        Engine::new(data, 1).expect("bare boot") // the desktop's DEFAULT_SEED
    } else {
        let saves = load_dir(&dir.join("SAVE")).expect("SAVE dir readable");
        let master = saves.raw_file("SAVGAMA.DAT").expect("slot A present");
        let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n))
            .expect("slot A parses");
        gbx_engine::import::import_original(&set, data, 1).expect("slot A imports")
    };

    // The saves directory the desktop would have used, copied somewhere
    // disposable so a replay can save/load without mutating the real one.
    let saves_dir =
        std::env::temp_dir().join(format!("restrike-replay-saves-{}", std::process::id()));
    copy_dir_shallow(&dir.join("SAVE"), &saves_dir);
    engine.set_slot_directory(gbx_engine::saveload_fs::scan_slot_directory(&saves_dir));
    eprintln!("replay: saves sandbox at {}", saves_dir.display());

    let dump_at: Vec<u64> = std::env::var("RESTRIKE_REPLAY_DUMP")
        .ok()
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_default();

    let mut cursor = 0usize;
    let mut last_probe = String::new();
    for tick in 1..=last_tick {
        let batch: &[InputEvent] = if cursor < schedule.len() && schedule[cursor].0 == tick {
            cursor += 1;
            &schedule[cursor - 1].1
        } else {
            &[]
        };
        let frame = engine.tick(batch);
        if dump_at.contains(&tick) {
            let path = std::env::temp_dir().join(format!("restrike-replay-{tick:05}.ppm"));
            let mut out = format!("P6\n{} {}\n255\n", 320, 200).into_bytes();
            for &idx in frame.pixels {
                out.extend_from_slice(&frame.palette[idx as usize]);
            }
            std::fs::write(&path, &out).expect("dump writable");
            eprintln!("frame {tick} -> {}", path.display());
        }
        if let Some(request) = engine.take_io_request() {
            let data = engine.game_data().clone();
            let outcome = gbx_engine::saveload_fs::fulfill(
                &mut engine,
                request,
                &saves_dir,
                data,
                1, // the desktop's DEFAULT_SEED, as above
            );
            engine.set_slot_directory(gbx_engine::saveload_fs::scan_slot_directory(&saves_dir));
            match outcome {
                Ok(()) => {
                    engine.recompose_world_screen();
                    println!("tick {tick} | io {request:?} ok");
                }
                Err(err) => println!("tick {tick} | io {request:?} FAILED: {err:?}"),
            }
        }
        let probe = engine.probe();
        let transcript = engine.take_transcript();
        if !batch.is_empty() || probe != last_probe || !transcript.is_empty() {
            println!("tick {tick} | sent {batch:?} | {probe}");
            for entry in transcript {
                println!("    {entry:?}");
            }
            last_probe = probe;
        }
    }
}
