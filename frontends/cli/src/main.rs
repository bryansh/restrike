//! `restrike` — headless CLI frontend for the Restrike engine.

use gbx_formats::detect::{self, Detection};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("detect") => cmd_detect(args.next()),
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
    eprintln!();
    eprintln!("If DIR is omitted, falls back to the GBX_DATA_DIR environment variable.");
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
