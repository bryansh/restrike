//! `restrike verify [DIR]` — the D6 user surface for D-RP4's verify-on-load
//! protocol: loads `GameData` from `DIR` (or `GBX_DATA_DIR`), runs
//! `RuleSet::verify`, and prints the per-table report. Advisory only (the
//! pack stays authoritative regardless of what this prints) — this command
//! never fails just because a table didn't verify.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_formats::game_data;
use gbx_rules::pack::{RuleSet, VerifyStatus};

pub fn cmd_verify(args: Vec<String>) -> ExitCode {
    let mut dir_arg = None;
    for arg in args {
        if dir_arg.is_none() && !arg.starts_with("--") {
            dir_arg = Some(PathBuf::from(arg));
        } else {
            eprintln!("restrike: unknown verify flag '{arg}'");
            print_usage();
            return ExitCode::FAILURE;
        }
    }

    let dir = match dir_arg.or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from)) {
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

    let data = match game_data::load_dir(&dir) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("restrike: failed to load '{}': {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };

    let report = RuleSet::load().verify(&data);
    for (id, status) in &report.entries {
        println!("{id}: {}", format_status(status));
    }

    ExitCode::SUCCESS
}

fn format_status(status: &VerifyStatus) -> String {
    match status {
        VerifyStatus::Verified => "Verified".to_string(),
        VerifyStatus::Moved { found_at } => {
            let offsets: Vec<String> = found_at.iter().map(|o| format!("{o:#x}")).collect();
            format!("Moved (found at {})", offsets.join(", "))
        }
        VerifyStatus::NotFound => "NotFound".to_string(),
        VerifyStatus::BinaryAbsent { file } => format!("BinaryAbsent ({file})"),
        VerifyStatus::ImageUndecodable { file, reason } => {
            format!("ImageUndecodable ({file}: {reason})")
        }
        VerifyStatus::Unanchored => "Unanchored".to_string(),
        VerifyStatus::NotAttempted { reason } => format!("NotAttempted ({reason})"),
    }
}

fn print_usage() {
    eprintln!("usage: restrike verify [DIR]");
    eprintln!();
    eprintln!(
        "Loads game data from DIR (or GBX_DATA_DIR if DIR is omitted), decompresses the \
         binaries the embedded rules packs anchor into, and prints each table's D-RP4 \
         verification status. Advisory only -- the pack stays authoritative regardless of \
         what this reports, and this command never fails solely because a table didn't verify."
    );
}
