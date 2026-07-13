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
    );
    println!("tick {}: exit hash={exit_hash}", trace.max_tick.max(1));
    ExitCode::SUCCESS
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
    seed: u64,
}

const DEFAULT_SEED: u64 = 1;

impl Args {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut dir = None;
        let mut trace = None;
        let mut dump_at = Vec::new();
        let mut out_dir = PathBuf::from(".");
        let mut seed = DEFAULT_SEED;

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
                "--seed" => {
                    let v = next_val(&mut iter, "--seed")?;
                    seed = v
                        .parse::<u64>()
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
         [--seed N]"
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
}
