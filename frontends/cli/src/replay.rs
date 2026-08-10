//! `restrike replay <session.log> [--slot X|--bare] [--checkpoint-every N]
//! [--digests-out FILE] [--verify FILE]` — the **H5 capture vehicle**
//! (`docs/design/roll-credits.md` D-RC3).
//!
//! ## Why this exists next to `restrike walk`
//!
//! `walk` is the H2-era driver: hand-authored JSON traces, **frame-hash**
//! checkpoints, a **bare** boot. All three are wrong for a playthrough. A run
//! of Curse of the Azure Bonds starts from an imported party (a bare engine
//! cannot even fight — "no living party"); nobody hand-authors a hundred hours
//! of input; and frame hashes die to our own renderer work, as the whole M6 arc
//! demonstrated. `walk`'s checkpoint mechanism is deliberately left exactly as
//! it is — this is a second, differently-shaped tool, not a replacement.
//!
//! ## The pipeline
//!
//! **Record** with the desktop, which already does it:
//! `RESTRIKE_DEBUG_LOG=/path/session.log restrike-desktop`. Nothing new to
//! learn, and every log ever captured is already a valid input here.
//!
//! **Replay** with this, which boots the way that desktop booted (imported
//! slot A by default), feeds the recorded input schedule tick for tick, plays
//! the host's part for save/load — against a *copy* of the saves directory,
//! never the real one — and emits `tick<TAB>digest` lines.
//!
//! **Verify** by re-running against a previously emitted file: the first
//! differing checkpoint is reported with both digests and the run fails. That
//! is an H5 trace: state, not pixels, so a slice that repaints the world does
//! not invalidate it.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_engine::debug_log::{self, Boot, Session};
use gbx_engine::input::InputEvent;

/// The desktop's own default seed — a replay of a log written without
/// `--seed` must use it or the PRNG stream differs from tick one.
const DEFAULT_SEED: u32 = 1;
/// Checkpoint cadence when `--checkpoint-every` is not given. Fine enough to
/// localize a divergence to a couple of seconds of play, coarse enough that an
/// hour of session is a few thousand lines.
const DEFAULT_CHECKPOINT_EVERY: u64 = 60;

pub fn cmd_replay(args: Vec<String>) -> ExitCode {
    let opts = match Args::parse(args) {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("restrike: {msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let Some(dir) = opts
        .dir
        .clone()
        .or_else(|| std::env::var_os("GBX_DATA_DIR").map(PathBuf::from))
    else {
        eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
        print_usage();
        return ExitCode::FAILURE;
    };

    let text = match std::fs::read_to_string(&opts.log) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("restrike: cannot read '{}': {err}", opts.log.display());
            return ExitCode::FAILURE;
        }
    };
    let session = Session::parse(&text);
    for unknown in &session.unrecognized {
        eprintln!("restrike: unrecognized event {unknown:?} in the log — skipped");
    }
    if session.schedule.is_empty() {
        eprintln!(
            "restrike: '{}' carries no input events — is it a RESTRIKE_DEBUG_LOG?",
            opts.log.display()
        );
    }

    let expected = match &opts.verify {
        Some(path) => match read_digests(path) {
            Ok(map) => Some(map),
            Err(err) => {
                eprintln!("restrike: cannot read '{}': {err}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let mut engine = match debug_log::boot(&dir, opts.boot, opts.seed) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("restrike: {err}");
            return ExitCode::FAILURE;
        }
    };

    // The saves directory the recording desktop used, copied somewhere
    // disposable: a session that saved must save here, and a replay must never
    // write over the evidence.
    let saves_dir = debug_log::sandbox_path("replay");
    if let Err(err) = debug_log::sandbox_saves(&dir.join("SAVE"), &saves_dir) {
        eprintln!("restrike: saves sandbox at {}: {err}", saves_dir.display());
        return ExitCode::FAILURE;
    }
    engine.set_slot_directory(gbx_engine::saveload_fs::scan_slot_directory(&saves_dir));

    let last_tick = session.last_input_tick() + opts.tail;
    eprintln!(
        "-- replay: {} input batch(es), {last_tick} tick(s), boot {:?}, seed {} --",
        session.schedule.len(),
        opts.boot,
        opts.seed
    );

    let mut out: Box<dyn Write> = match &opts.digests_out {
        Some(path) => match std::fs::File::create(path) {
            Ok(f) => Box::new(std::io::BufWriter::new(f)),
            Err(err) => {
                eprintln!("restrike: cannot create '{}': {err}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => Box::new(std::io::stdout()),
    };

    let mut checkpoints = 0u64;
    let mut mismatch: Option<(u64, String, String)> = None;
    for tick in 1..=last_tick {
        let batch: Vec<InputEvent> = session.inputs_at(tick).to_vec();
        engine.tick(&batch);
        if let Some((request, outcome)) =
            debug_log::fulfill_pending_io(&mut engine, &saves_dir, opts.seed)
        {
            match outcome {
                Ok(()) => eprintln!("   tick {tick}: {request:?} ok"),
                // Loud, not fatal: the recorded session may itself have been a
                // failed load, and the replay's job is to reproduce it.
                Err(err) => eprintln!("   tick {tick}: {request:?} FAILED: {err}"),
            }
        }
        // The final tick is always a checkpoint: a run whose length is not a
        // multiple of the cadence still gets its ending compared.
        if tick % opts.every == 0 || tick == last_tick {
            let digest = engine.state_digest();
            checkpoints += 1;
            if let Err(err) = writeln!(out, "{tick}\t{digest}") {
                eprintln!("restrike: writing digests: {err}");
                return ExitCode::FAILURE;
            }
            if let Some(expected) = &expected {
                if let Some(want) = expected.get(&tick) {
                    if want != &digest && mismatch.is_none() {
                        mismatch = Some((tick, want.clone(), digest));
                    }
                }
            }
        }
    }
    if let Err(err) = out.flush() {
        eprintln!("restrike: writing digests: {err}");
        return ExitCode::FAILURE;
    }
    let _ = std::fs::remove_dir_all(&saves_dir);

    eprintln!("-- replay: {checkpoints} checkpoint(s) over {last_tick} tick(s) --");
    match (mismatch, &opts.verify) {
        (Some((tick, want, got)), _) => {
            eprintln!("DIVERGED at tick {tick}");
            eprintln!("  expected {want}");
            eprintln!("  actual   {got}");
            ExitCode::FAILURE
        }
        (None, Some(path)) => {
            eprintln!("VERIFIED against {}", path.display());
            ExitCode::SUCCESS
        }
        (None, None) => ExitCode::SUCCESS,
    }
}

/// Reads a `tick<TAB>digest` file back. Blank lines and `#` comments are
/// skipped so a trace can carry a provenance header.
fn read_digests(
    path: &std::path::Path,
) -> std::io::Result<std::collections::BTreeMap<u64, String>> {
    let text = std::fs::read_to_string(path)?;
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((tick, digest)) = line.split_once('\t') {
            if let Ok(tick) = tick.trim().parse::<u64>() {
                map.insert(tick, digest.trim().to_string());
            }
        }
    }
    Ok(map)
}

struct Args {
    dir: Option<PathBuf>,
    log: PathBuf,
    boot: Boot,
    every: u64,
    tail: u64,
    seed: u32,
    digests_out: Option<PathBuf>,
    verify: Option<PathBuf>,
}

impl Args {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut dir = None;
        let mut log = None;
        let mut boot = Boot::default();
        let mut every = DEFAULT_CHECKPOINT_EVERY;
        let mut tail = 120;
        let mut seed = DEFAULT_SEED;
        let mut digests_out = None;
        let mut verify = None;

        let mut iter = args.into_iter().peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--data" => dir = Some(PathBuf::from(next_val(&mut iter, "--data")?)),
                "--slot" => {
                    let v = next_val(&mut iter, "--slot")?;
                    let letter = v
                        .chars()
                        .next()
                        .ok_or("--slot needs a letter A-J")?
                        .to_ascii_uppercase();
                    boot = Boot::ImportedSlot(letter);
                }
                "--bare" => boot = Boot::Bare,
                "--checkpoint-every" => {
                    let v = next_val(&mut iter, "--checkpoint-every")?;
                    every = v
                        .parse::<u64>()
                        .ok()
                        .filter(|&n| n > 0)
                        .ok_or_else(|| format!("invalid --checkpoint-every '{v}'"))?;
                }
                "--tail" => {
                    let v = next_val(&mut iter, "--tail")?;
                    tail = v.parse().map_err(|_| format!("invalid --tail '{v}'"))?;
                }
                "--seed" => {
                    let v = next_val(&mut iter, "--seed")?;
                    seed = v.parse().map_err(|_| format!("invalid --seed '{v}'"))?;
                }
                "--digests-out" => {
                    digests_out = Some(PathBuf::from(next_val(&mut iter, "--digests-out")?))
                }
                "--verify" => verify = Some(PathBuf::from(next_val(&mut iter, "--verify")?)),
                other if other.starts_with("--") => {
                    return Err(format!("unknown replay flag '{other}'"))
                }
                other if log.is_none() => log = Some(PathBuf::from(other)),
                other => dir = Some(PathBuf::from(other)),
            }
        }

        Ok(Args {
            dir,
            log: log.ok_or("replay requires a <session.log> argument")?,
            boot,
            every,
            tail,
            seed,
            digests_out,
            verify,
        })
    }
}

fn next_val(
    iter: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
    flag: &str,
) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

pub fn print_usage() {
    eprintln!(
        "usage: restrike replay <session.log> [DIR] [--slot X|--bare] [--checkpoint-every N] \
         [--tail N] [--seed N] [--digests-out FILE] [--verify FILE]"
    );
    eprintln!();
    eprintln!(
        "Replays a RESTRIKE_DEBUG_LOG recorded by the desktop and emits H5 state-digest \
         checkpoints (roll-credits D-RC3) as 'tick<TAB>digest' lines — to --digests-out, else \
         stdout. Boots from an imported save slot (default A, the desktop's own default); \
         --bare boots partyless. Save/load requests the session made are fulfilled against a \
         throwaway COPY of DIR/SAVE, so a replay never writes over real saves."
    );
    eprintln!();
    eprintln!(
        "--verify FILE re-runs and compares against a previously emitted digest file, failing \
         at the first differing checkpoint with both hashes. Digests hash engine STATE — \
         position/area/block/clock/party/PRNG/search_flags plus the ScriptMemory windows where \
         quest flags live — never pixels, so renderer work does not invalidate a trace."
    );
    eprintln!();
    eprintln!("Record one with: RESTRIKE_DEBUG_LOG=/path/session.log restrike-desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn the_defaults_are_the_desktops_own() {
        let a = parse(&["session.log"]).expect("a bare log path is enough");
        assert_eq!(a.log, PathBuf::from("session.log"));
        assert_eq!(a.boot, Boot::ImportedSlot('A'));
        assert_eq!(a.seed, DEFAULT_SEED);
        assert_eq!(a.every, DEFAULT_CHECKPOINT_EVERY);
        assert!(a.digests_out.is_none() && a.verify.is_none());
    }

    #[test]
    fn every_flag_lands_where_it_should() {
        let a = parse(&[
            "s.log",
            "/data",
            "--slot",
            "c",
            "--checkpoint-every",
            "5",
            "--tail",
            "7",
            "--seed",
            "9",
            "--digests-out",
            "out.tsv",
            "--verify",
            "want.tsv",
        ])
        .expect("flags parse");
        assert_eq!(a.dir, Some(PathBuf::from("/data")));
        assert_eq!(a.boot, Boot::ImportedSlot('C'), "--slot uppercases");
        assert_eq!((a.every, a.tail, a.seed), (5, 7, 9));
        assert_eq!(a.digests_out, Some(PathBuf::from("out.tsv")));
        assert_eq!(a.verify, Some(PathBuf::from("want.tsv")));

        assert_eq!(parse(&["s.log", "--bare"]).unwrap().boot, Boot::Bare);
    }

    #[test]
    fn bad_arguments_are_refused_not_guessed() {
        assert!(parse(&[]).is_err(), "a log path is required");
        assert!(parse(&["s.log", "--nope"]).is_err());
        assert!(parse(&["s.log", "--seed"]).is_err(), "flag needs a value");
        assert!(
            parse(&["s.log", "--checkpoint-every", "0"]).is_err(),
            "a zero cadence would checkpoint every tick by accident"
        );
    }

    #[test]
    fn digest_files_round_trip_through_the_reader() {
        let path =
            std::env::temp_dir().join(format!("restrike-digests-{}.tsv", std::process::id()));
        std::fs::write(
            &path,
            "# recorded 2026-08-09\n\n60\tabc123\n120\tdef456\ngarbage line\n",
        )
        .expect("temp file");
        let map = read_digests(&path).expect("readable");
        assert_eq!(map.len(), 2, "comments, blanks and junk are skipped");
        assert_eq!(map[&60], "abc123");
        assert_eq!(map[&120], "def456");
        let _ = std::fs::remove_file(&path);
    }
}
