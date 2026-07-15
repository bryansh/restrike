//! `restrike walk [DIR] --trace <FILE> [--dump-at TICK]... [--out-dir DIR]
//! [--seed N]` — the headless trace driver (D-UI7's H5 seed, D-UI5's item
//! 6): replays a JSON-lines input trace against a real `Engine::tick` loop,
//! printing the frame hash at each declared checkpoint tick and on exit.
//!
//! `--record` (capturing a live session as a trace) is not needed yet —
//! traces are hand-authored or test-generated for now — but the seam is
//! this module's [`TraceEvent`]/[`TraceLine`] types: a future `--record`
//! mode would just serialize the same shapes it replays here, most likely
//! consumed by the DOSBox capture work (M2 step 8).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_engine::engine::{Engine, Frame};
use gbx_engine::input::InputEvent;
use gbx_formats::game_data::load_dir;

/// One line of a walk trace file: which tick the event belongs to, and
/// what happens at it (serde JSON lines: `{"tick": N, "event": ...}`).
/// Several lines may share a `tick` — e.g. multiple `input` events collected
/// for the same tick, applied in file order, matching D-UI1's "the frontend
/// pushes the events it collected since the last tick, in order".
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TraceLine {
    pub tick: u64,
    pub event: TraceEvent,
}

/// `{"input": <InputEvent>}` feeds that tick's `Engine::tick` input slice;
/// `"checkpoint"` (a JSON string, since it carries no data) requests a
/// printed frame hash for that tick — D-UI7's "checkpoints are explicit
/// `(trace, tick_index)` pairs, never named moments".
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceEvent {
    Input(InputEvent),
    Checkpoint,
}

/// A parsed trace: every tick's declared input events (ticks with none get
/// an empty slice at replay time), the sorted checkpoint ticks, and the
/// highest tick index mentioned anywhere in the file.
#[derive(Debug, Default)]
pub struct Trace {
    pub inputs: BTreeMap<u64, Vec<InputEvent>>,
    pub checkpoints: Vec<u64>,
    pub max_tick: u64,
}

#[derive(Debug)]
pub enum TraceError {
    Json {
        line_no: usize,
        err: serde_json::Error,
    },
    Empty,
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceError::Json { line_no, err } => write!(f, "line {line_no}: {err}"),
            TraceError::Empty => write!(f, "trace file has no trace lines"),
        }
    }
}

impl Trace {
    /// Parses a JSON-lines trace: one [`TraceLine`] per non-blank line.
    /// Blank lines are skipped (hand-authoring convenience); anything else
    /// that fails to parse as a `TraceLine` is a loud, line-numbered error —
    /// a trace is test/tooling input, not user-facing data worth silently
    /// tolerating garbage in.
    pub fn parse(text: &str) -> Result<Self, TraceError> {
        let mut trace = Trace::default();
        let mut saw_a_line = false;
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            saw_a_line = true;
            let parsed: TraceLine = serde_json::from_str(line).map_err(|err| TraceError::Json {
                line_no: i + 1,
                err,
            })?;
            trace.max_tick = trace.max_tick.max(parsed.tick);
            match parsed.event {
                TraceEvent::Input(event) => {
                    trace.inputs.entry(parsed.tick).or_default().push(event)
                }
                TraceEvent::Checkpoint => trace.checkpoints.push(parsed.tick),
            }
        }
        if !saw_a_line {
            return Err(TraceError::Empty);
        }
        trace.checkpoints.sort_unstable();
        Ok(trace)
    }
}

/// Replays `trace` against `engine`: one `Engine::tick` call per tick index
/// from 1 through `trace.max_tick`, in order — the tick model has no gaps
/// (D-UI1), so every intervening tick runs with an empty input slice even
/// if the trace never mentions it. `on_checkpoint` fires, in tick order,
/// for each declared checkpoint tick; `on_dump` fires for each tick named
/// in `dump_at`. Returns the final tick's frame hash (the "on exit" hash).
pub fn replay(
    engine: &mut Engine,
    trace: &Trace,
    dump_at: &[u64],
    mut on_checkpoint: impl FnMut(u64, &str),
    mut on_dump: impl FnMut(u64, &Frame<'_>),
    mut on_transcript: impl FnMut(u64, &gbx_engine::vmhost::TranscriptEntry),
) -> String {
    let empty: Vec<InputEvent> = Vec::new();
    let mut exit_hash = String::new();
    let last_tick = trace.max_tick.max(1);
    for tick in 1..=last_tick {
        let input = trace.inputs.get(&tick).unwrap_or(&empty);
        let frame = engine.tick(input);
        let hash = frame.hash_hex();
        if trace.checkpoints.binary_search(&tick).is_ok() {
            on_checkpoint(tick, &hash);
        }
        if dump_at.contains(&tick) {
            on_dump(tick, &frame);
        }
        if tick == last_tick {
            exit_hash = hash;
        }
        for entry in engine.take_transcript() {
            on_transcript(tick, &entry);
        }
    }
    exit_hash
}

pub fn cmd_walk(args: Vec<String>) -> ExitCode {
    let opts = match Args::parse(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("restrike: {msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let dir = match opts
        .dir
        .clone()
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
    {
        Some(dir) => dir,
        None => {
            eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let trace_text = match fs::read_to_string(&opts.trace) {
        Ok(text) => text,
        Err(err) => {
            eprintln!(
                "restrike: failed to read trace '{}': {err}",
                opts.trace.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let trace = match Trace::parse(&trace_text) {
        Ok(trace) => trace,
        Err(err) => {
            eprintln!(
                "restrike: failed to parse trace '{}': {err}",
                opts.trace.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let data = match load_dir(&dir) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("restrike: failed to read '{}': {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };
    let mut engine = match Engine::new(data, opts.seed) {
        Ok(engine) => engine,
        Err(err) => {
            eprintln!("restrike: Engine::new failed to boot this data: {err:?}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "-- walk: replaying {} tick(s) from '{}' (seed={}) --",
        trace.max_tick.max(1),
        opts.trace.display(),
        opts.seed
    );

    let mut transcript_file = match &opts.transcript {
        Some(path) => match fs::File::create(path) {
            Ok(f) => Some(std::io::BufWriter::new(f)),
            Err(err) => {
                eprintln!("restrike: failed to create '{}': {err}", path.display());
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let exit_hash = replay(
        &mut engine,
        &trace,
        &opts.dump_at,
        |tick, hash| println!("tick {tick}: checkpoint hash={hash}"),
        |tick, frame| {
            let path = opts.out_dir.join(format!("restrike-walk-dump-{tick}.ppm"));
            match write_ppm(&path, frame) {
                Ok(()) => println!("tick {tick}: dumped '{}'", path.display()),
                Err(err) => eprintln!("restrike: failed to write '{}': {err}", path.display()),
            }
        },
        |tick, entry| {
            let Some(w) = transcript_file.as_mut() else {
                return;
            };
            use std::io::Write;
            if let Err(err) = writeln!(w, "{}", format_transcript_line(tick, entry)) {
                eprintln!("restrike: failed to write transcript line: {err}");
            }
        },
    );
    if let Some(mut w) = transcript_file {
        use std::io::Write;
        if let Err(err) = w.flush() {
            eprintln!("restrike: failed to flush transcript: {err}");
        } else if let Some(path) = &opts.transcript {
            println!("transcript written to '{}'", path.display());
        }
    }
    println!("tick {}: exit hash={exit_hash}", trace.max_tick.max(1));
    ExitCode::SUCCESS
}

/// One transcript line's text, shared by `--transcript`'s file writer and
/// the local-only expected-transcript comparison test below, so the two
/// never drift out of the same format.
pub fn format_transcript_line(tick: u64, entry: &gbx_engine::vmhost::TranscriptEntry) -> String {
    use gbx_engine::vmhost::TranscriptEntry;
    match entry {
        TranscriptEntry::Print {
            text,
            clear_first: false,
        } => format!("tick {tick}: PRINT: {text}"),
        TranscriptEntry::Print {
            text,
            clear_first: true,
        } => format!("tick {tick}: PRINTCLEAR: {text}"),
        TranscriptEntry::Request(label) => format!("tick {tick}: REQUEST: {label}"),
    }
}

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
            fs::create_dir_all(parent)?;
        }
    }
    fs::File::create(path)?.write_all(&out)
}

struct Args {
    dir: Option<PathBuf>,
    trace: PathBuf,
    dump_at: Vec<u64>,
    out_dir: PathBuf,
    /// Determinism over novelty (the desktop/demo convention this session
    /// established, `engine.rs`'s test module and `gbx-engine/src/demo.rs`'s
    /// real-data walks all fix seed `1`): default to the same constant so a
    /// trace with no `--seed` reproduces the same PRNG stream everywhere.
    seed: u32,
    /// D10: transcripts contain real game text — a LOCAL-ONLY artifact. The
    /// caller is responsible for pointing this outside the repo (or at a
    /// gitignored path); this tool has no opinion on where, only that it
    /// never ships one.
    transcript: Option<PathBuf>,
}

const DEFAULT_SEED: u32 = 1;

impl Args {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut dir = None;
        let mut trace = None;
        let mut dump_at = Vec::new();
        let mut out_dir = PathBuf::from(".");
        let mut seed = DEFAULT_SEED;
        let mut transcript = None;

        let mut iter = args.into_iter().peekable();
        if let Some(first) = iter.peek() {
            if !first.starts_with("--") {
                dir = Some(PathBuf::from(iter.next().unwrap()));
            }
        }
        while let Some(flag) = iter.next() {
            match flag.as_str() {
                "--trace" => trace = Some(PathBuf::from(next_val(&mut iter, "--trace")?)),
                "--dump-at" => {
                    let v = next_val(&mut iter, "--dump-at")?;
                    dump_at.push(
                        v.parse::<u64>()
                            .map_err(|_| format!("invalid --dump-at tick '{v}'"))?,
                    );
                }
                "--out-dir" => out_dir = PathBuf::from(next_val(&mut iter, "--out-dir")?),
                "--transcript" => {
                    transcript = Some(PathBuf::from(next_val(&mut iter, "--transcript")?))
                }
                "--seed" => {
                    let v = next_val(&mut iter, "--seed")?;
                    seed = v
                        .parse::<u32>()
                        .map_err(|_| format!("invalid --seed '{v}'"))?;
                }
                other => return Err(format!("unknown walk flag '{other}'")),
            }
        }

        Ok(Args {
            dir,
            trace: trace.ok_or("walk requires --trace <FILE>")?,
            dump_at,
            out_dir,
            seed,
            transcript,
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

fn print_usage() {
    eprintln!(
        "usage: restrike walk [DIR] --trace <FILE> [--dump-at TICK]... [--out-dir DIR] \
         [--seed N] [--transcript <PATH>]"
    );
    eprintln!();
    eprintln!(
        "Replays a JSON-lines input trace ({{\"tick\": N, \"event\": {{\"input\": <InputEvent>}} \
         | \"checkpoint\"}}) against a real Engine::tick loop, one tick per index from 1 through \
         the trace's highest tick, in order. Prints 'tick N: checkpoint hash=...' for each \
         declared checkpoint and 'tick N: exit hash=...' for the final tick. --dump-at N (may \
         repeat) writes that tick's frame as a PPM into --out-dir (default '.'). --seed defaults \
         to 1 (this session's fixed-determinism convention)."
    );
    eprintln!();
    eprintln!(
        "--transcript <PATH> logs every PRINT/PRINTCLEAR text and VM-request label emitted \
         during the replay, one 'tick N: KIND: text' line per event. Transcripts contain real \
         game text (D10) — PATH must be outside the repo (or gitignored); this tool does not \
         enforce that."
    );
}

/// M2 step 8's task deliverable 3: a local-only test comparing the
/// committed circuit's live transcript against an expected-transcript file
/// the user maintains outside the repo (a DOSBox side-by-side capture of
/// the same walk, D10 — never committed, never auto-generated by this
/// test). Gated on GBX_DATA_DIR like every other real-data test in this
/// repo, AND on the expected file's own existence: until a human captures
/// one, this test documents the convention and skips rather than failing.
#[cfg(test)]
mod expected_transcript {
    use super::*;
    use gbx_engine::engine::Engine;
    use gbx_formats::game_data::load_dir;

    /// Where the human-maintained oracle transcript lives, documented (never
    /// committed — the directory itself is outside this repo entirely).
    const EXPECTED_TRANSCRIPT_PATH: &str = "expected/tilverton-circuit.transcript";

    #[test]
    fn live_transcript_matches_the_human_maintained_expected_transcript() {
        let Some(data_dir) = env::var_os("GBX_DATA_DIR") else {
            eprintln!("GBX_DATA_DIR not set — skipping (local-only test)");
            return;
        };
        // The expected-transcript directory is a sibling of GBX_DATA_DIR's
        // usual home (~/goldbox-data), not GBX_DATA_DIR itself (which points
        // at the game's own data files) — resolved from HOME per this
        // repo's documented convention (`docs/dosbox-capture.md`).
        let Some(home) = env::var_os("HOME") else {
            eprintln!("HOME not set — skipping (local-only test)");
            return;
        };
        let expected_path = PathBuf::from(home)
            .join("goldbox-data")
            .join(EXPECTED_TRANSCRIPT_PATH);
        let Ok(expected) = fs::read_to_string(&expected_path) else {
            eprintln!(
                "no expected transcript at '{p}' yet — this test documents the convention and \
                 skips until a human captures one from DOSBox (see docs/dosbox-capture.md). \
                 Reference (engine-only, not an oracle): run `restrike walk $GBX_DATA_DIR \
                 --trace fixtures/tilverton-circuit.jsonl --transcript {p}`",
                p = expected_path.display()
            );
            return;
        };

        let trace_text = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/tilverton-circuit.jsonl"
        ))
        .expect("fixtures/tilverton-circuit.jsonl must be readable");
        let trace = Trace::parse(&trace_text).expect("fixtures/tilverton-circuit.jsonl must parse");

        let data =
            load_dir(std::path::Path::new(&data_dir)).expect("GBX_DATA_DIR must be readable");
        let mut engine =
            Engine::new(data, DEFAULT_SEED).expect("Engine::new must boot against real CotAB data");

        let mut lines = Vec::new();
        replay(
            &mut engine,
            &trace,
            &[],
            |_, _| {},
            |_, _| {},
            |tick, entry| lines.push(format_transcript_line(tick, entry)),
        );
        let live = lines.join("\n");

        assert_eq!(
            live.trim_end(),
            expected.trim_end(),
            "live transcript diverged from the human-maintained expected transcript at '{}'",
            expected_path.display()
        );
    }
}
