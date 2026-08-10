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
//! The machinery itself lives in `gbx_engine::debug_log` and is shared with
//! `restrike replay` (roll-credits D-RC3) — this example is the frame-dumping
//! debugging front for it. Reach for `restrike replay` when you want digests.

use gbx_engine::debug_log::{self, Boot, Session};
use gbx_engine::input::InputEvent;

fn main() {
    let mut args = std::env::args().skip(1);
    let log_path = args.next().expect("usage: replay_debug_log <debug.log>");
    let extra: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);

    let text = std::fs::read_to_string(&log_path).expect("log readable");
    let session = Session::parse(&text);
    for unknown in &session.unrecognized {
        eprintln!("replay: unrecognized event {unknown:?} — skipped");
    }
    let last_tick = session.last_input_tick() + extra;
    eprintln!(
        "replay: {} input batches over {last_tick} ticks (+{extra} tail)",
        session.schedule.len()
    );

    let dir = std::path::PathBuf::from(
        std::env::var_os("GBX_DATA_DIR").expect("GBX_DATA_DIR must be set"),
    );
    // The replay must boot the way the desktop that WROTE the log booted:
    // slot-A import is the desktop default; RESTRIKE_REPLAY_BARE=1 replays
    // logs from a `--slot none` (or pre-import-era) desktop.
    let posture = if std::env::var_os("RESTRIKE_REPLAY_BARE").is_some() {
        Boot::Bare
    } else {
        Boot::default()
    };
    let seed = 1; // the desktop's DEFAULT_SEED
    let mut engine = debug_log::boot(&dir, posture, seed).unwrap_or_else(|e| panic!("replay: {e}"));

    // The desktop's saves directory, copied somewhere disposable: a recorded
    // session that saved must save here too, and never over the real thing.
    let saves_dir = debug_log::sandbox_path("replay");
    debug_log::sandbox_saves(&dir.join("SAVE"), &saves_dir).expect("saves sandbox");
    engine.set_slot_directory(gbx_engine::saveload_fs::scan_slot_directory(&saves_dir));
    eprintln!("replay: saves sandbox at {}", saves_dir.display());

    let dump_at: Vec<u64> = std::env::var("RESTRIKE_REPLAY_DUMP")
        .ok()
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_default();

    let mut last_probe = String::new();
    for tick in 1..=last_tick {
        let batch: Vec<InputEvent> = session.inputs_at(tick).to_vec();
        let frame = engine.tick(&batch);
        if dump_at.contains(&tick) {
            let path = std::env::temp_dir().join(format!("restrike-replay-{tick:05}.ppm"));
            let mut out = format!("P6\n{} {}\n255\n", 320, 200).into_bytes();
            for &idx in frame.pixels {
                out.extend_from_slice(&frame.palette[idx as usize]);
            }
            std::fs::write(&path, &out).expect("dump writable");
            eprintln!("frame {tick} -> {}", path.display());
        }
        if let Some((request, outcome)) =
            debug_log::fulfill_pending_io(&mut engine, &saves_dir, seed)
        {
            match outcome {
                Ok(()) => println!("tick {tick} | io {request:?} ok"),
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
