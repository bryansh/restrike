//! `restrike` — headless CLI frontend for the Restrike engine.

mod census;
mod dump_image;
mod map;
mod run_script;

use gbx_formats::detect::{self, Detection};
use gbx_vm::dialect::COTAB;
use gbx_vm::{decode, disassemble};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("detect") => cmd_detect(args.next()),
        Some("disasm") => cmd_disasm(args.collect()),
        Some("census") => census::cmd_census(args.collect()),
        Some("map") => map::cmd_map(args.collect()),
        Some("run-script") => run_script::cmd_run_script(args.collect()),
        Some("dump-image") => dump_image::cmd_dump_image(args.collect()),
        Some(other) => {
            eprintln!("restrike: unknown command '{other}'");
            print_usage();
            ExitCode::FAILURE
        }
        None => {
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!("usage: restrike detect [DIR]");
    eprintln!("       restrike disasm --raw-block <PATH> [--entry 0xNNNN[,0xNNNN...]]");
    eprintln!("       restrike census [DIR] [--out <PATH>]");
    eprintln!("       restrike map [DIR] --block <ID> [--dax <FILE>]");
    eprintln!(
        "       restrike run-script [DIR] --dax <FILE> --block <ID> [--vector N] [--trace] \
         [--reply k=v ...]"
    );
    eprintln!(
        "       restrike dump-image [DIR] --dax <FILE> --block <ID> [--frame N] [--mask N] \
         --out <path.ppm>"
    );
    eprintln!();
    eprintln!("If DIR is omitted, falls back to the GBX_DATA_DIR environment variable.");
    eprintln!();
    eprintln!(
        "census scans DIR for ECL*.DAX, extracts every block, and disassembles from each \
         block's header vectors, aggregating opcode/hazard statistics. CSV goes to --out (or \
         stdout by default); the human-readable report always goes to stderr."
    );
    eprintln!();
    eprintln!(
        "disasm reads a raw ECL block file (runtime input only — no sample blocks ship in \
         this repo) and runs the flow-following disassembler (D-VM8) over it. --entry defaults \
         to the block's base address (0x8000), not the block's real header vectors — unlike \
         census, disasm doesn't decode the header for you, so pass the true entry points with \
         --entry when you know them."
    );
}

fn cmd_detect(dir_arg: Option<String>) -> ExitCode {
    let dir = match resolve_data_dir(dir_arg) {
        Some(dir) => dir,
        None => {
            eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    if !dir.is_dir() {
        eprintln!("restrike: '{}' is not a directory", dir.display());
        return ExitCode::FAILURE;
    }

    match detect::detect_dir(&dir) {
        Ok(Detection::Known { game, files }) => {
            println!("detected game: {game}");
            print_file_report(&files);
            ExitCode::SUCCESS
        }
        Ok(Detection::Unknown { files }) => {
            println!(
                "unknown game ({} file(s) scanned, no signature match)",
                files.len()
            );
            print_file_report(&files);
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("restrike: failed to scan '{}': {err}", dir.display());
            ExitCode::FAILURE
        }
    }
}

fn resolve_data_dir(dir_arg: Option<String>) -> Option<PathBuf> {
    dir_arg
        .map(PathBuf::from)
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
}

fn print_file_report(files: &[detect::FileReport]) {
    for file in files {
        println!("  {file}");
    }
}

fn cmd_disasm(args: Vec<String>) -> ExitCode {
    let mut raw_block: Option<PathBuf> = None;
    let mut entries: Vec<u16> = Vec::new();

    let mut iter = args.into_iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--raw-block" => {
                let Some(path) = iter.next() else {
                    eprintln!("restrike: --raw-block requires a PATH argument");
                    return ExitCode::FAILURE;
                };
                raw_block = Some(PathBuf::from(path));
            }
            "--entry" => {
                let Some(list) = iter.next() else {
                    eprintln!("restrike: --entry requires a comma-separated address list");
                    return ExitCode::FAILURE;
                };
                for piece in list.split(',') {
                    match parse_addr(piece) {
                        Some(addr) => entries.push(addr),
                        None => {
                            eprintln!("restrike: invalid --entry address '{piece}'");
                            return ExitCode::FAILURE;
                        }
                    }
                }
            }
            other => {
                eprintln!("restrike: unknown disasm flag '{other}'");
                print_usage();
                return ExitCode::FAILURE;
            }
        }
    }

    let Some(raw_block) = raw_block else {
        eprintln!("restrike: disasm requires --raw-block <PATH>");
        print_usage();
        return ExitCode::FAILURE;
    };

    let bytes = match std::fs::read(&raw_block) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("restrike: failed to read '{}': {err}", raw_block.display());
            return ExitCode::FAILURE;
        }
    };

    if bytes.len() > decode::ECL_BLOCK_SIZE {
        eprintln!(
            "restrike: '{}' is {} bytes, exceeding the 0x{:X}-byte ECL block size",
            raw_block.display(),
            bytes.len(),
            decode::ECL_BLOCK_SIZE
        );
        return ExitCode::FAILURE;
    }
    let block = decode::BlockBytes::from_bytes(&bytes);

    if entries.is_empty() {
        entries.push(decode::ECL_BLOCK_BASE);
    }

    let listing = disassemble(&block, &COTAB, &entries);
    print!("{}", listing.render(&COTAB));

    let summary = listing.summary();
    eprintln!();
    eprintln!(
        "-- summary: {} opcode(s) reached, {} hazard(s), {} data region(s) --",
        summary.opcode_reached_counts.len(),
        summary.hazards.len(),
        summary.data_region_spans.len()
    );

    ExitCode::SUCCESS
}

fn parse_addr(s: &str) -> Option<u16> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u16::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}
