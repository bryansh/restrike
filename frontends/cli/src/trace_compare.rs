//! `restrike trace-compare <A> [B] [--chain]` — the thin CLI wrapper over
//! `gbx-oracle` (D-OR3: **thin** — argument parsing and file I/O only; all
//! comparison logic lives in the crate).
//!
//! Two files given: compares them on the D-OR3 projection (validity gate, then
//! `(before, after)` + `(n, result)`-when-both equality). `--chain`
//! additionally runs the D-OR4-part-B chain-continuity check on each file. One
//! file plus `--chain`: chain-checks that single trace (the self-validation a
//! live capture needs before it's trusted). Exits non-zero with a located diff
//! on any mismatch, incomparability, or chain break.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_oracle::{check_chain, compare, Comparison, Trace};

pub fn cmd_trace_compare(args: Vec<String>) -> ExitCode {
    let opts = match Args::parse(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("restrike: {msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let a = match read_trace(&opts.a) {
        Ok(t) => t,
        Err(code) => return code,
    };

    // Single-file mode: chain-check only.
    let Some(b_path) = opts.b.as_ref() else {
        // `Args::parse` guarantees `--chain` here (no B is only valid with it).
        return if run_chain("A", &opts.a, &a) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    };

    let b = match read_trace(b_path) {
        Ok(t) => t,
        Err(code) => return code,
    };

    let mut ok = true;

    // Equality comparison.
    match compare(&a, &b) {
        Err(incomparable) => {
            eprintln!("restrike: {incomparable}");
            ok = false;
        }
        Ok(Comparison::Equal) => {
            println!(
                "equal: {} draw event(s) match on the projection (before, after[, n, result])",
                a.rng_event_count()
            );
        }
        Ok(Comparison::Diverged(d)) => {
            eprintln!("restrike: traces diverge — {d}");
            ok = false;
        }
    }

    // Optional chain continuity on both sides.
    if opts.chain {
        ok &= run_chain("A", &opts.a, &a);
        ok &= run_chain("B", b_path, &b);
    }

    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Runs the chain-continuity check on `trace`, printing a located break.
/// Returns whether the chain held.
fn run_chain(label: &str, path: &std::path::Path, trace: &Trace) -> bool {
    match check_chain(trace) {
        Ok(()) => {
            println!(
                "chain {label} ({}): OK ({} draw event(s))",
                path.display(),
                trace.rng_event_count()
            );
            true
        }
        Err(brk) => {
            eprintln!(
                "restrike: chain {label} ({}) broken — {brk}",
                path.display()
            );
            false
        }
    }
}

fn read_trace(path: &std::path::Path) -> Result<Trace, ExitCode> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(err) => {
            eprintln!("restrike: failed to read '{}': {err}", path.display());
            return Err(ExitCode::FAILURE);
        }
    };
    Trace::parse(&text).map_err(|err| {
        eprintln!("restrike: failed to parse '{}': {err}", path.display());
        ExitCode::FAILURE
    })
}

struct Args {
    a: PathBuf,
    b: Option<PathBuf>,
    chain: bool,
}

impl Args {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut chain = false;
        for arg in args {
            match arg.as_str() {
                "--chain" => chain = true,
                other if other.starts_with("--") => {
                    return Err(format!("unknown trace-compare flag '{other}'"))
                }
                _ => positional.push(PathBuf::from(arg)),
            }
        }

        let mut it = positional.into_iter();
        let a = it
            .next()
            .ok_or("trace-compare requires at least one trace file")?;
        let b = it.next();
        if let Some(extra) = it.next() {
            return Err(format!("unexpected extra argument '{}'", extra.display()));
        }
        if b.is_none() && !chain {
            return Err(
                "give two trace files to compare, or one file with --chain to chain-check it"
                    .to_string(),
            );
        }
        Ok(Args { a, b, chain })
    }
}

fn print_usage() {
    eprintln!("usage: restrike trace-compare <A.gbxtrace> [B.gbxtrace] [--chain]");
    eprintln!();
    eprintln!(
        "Compares two .gbxtrace files on the D-OR3 projection: the header validity gate \
         (gbxtrace/profile/game/seed/encounter must match; source/notes ignored), then exact \
         equality over (before, after) per draw, extended to (n, result) when both sides carry \
         them. `caller` and other diagnostic fields are excluded from equality."
    );
    eprintln!();
    eprintln!(
        "--chain additionally verifies each trace's PRNG chain continuity (after == step(before), \
         and consecutive draws link) — the D-OR4-part-B self-validation. With a single file, \
         --chain runs that check alone. Exits non-zero with a located diff on any mismatch."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn two_files_compare_without_chain() {
        let a = parse(&["x.gbxtrace", "y.gbxtrace"]).unwrap();
        assert_eq!(a.a, PathBuf::from("x.gbxtrace"));
        assert_eq!(a.b, Some(PathBuf::from("y.gbxtrace")));
        assert!(!a.chain);
    }

    #[test]
    fn single_file_requires_chain() {
        assert!(parse(&["only.gbxtrace"]).is_err());
        let a = parse(&["only.gbxtrace", "--chain"]).unwrap();
        assert!(a.b.is_none() && a.chain);
    }

    #[test]
    fn chain_flag_position_is_free() {
        let a = parse(&["--chain", "x", "y"]).unwrap();
        assert_eq!(a.b, Some(PathBuf::from("y")));
        assert!(a.chain);
    }

    #[test]
    fn unknown_flag_and_extra_positional_are_errors() {
        assert!(parse(&["x", "y", "--nope"]).is_err());
        assert!(parse(&["x", "y", "z"]).is_err());
        assert!(parse(&[]).is_err());
    }
}
