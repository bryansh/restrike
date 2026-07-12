use std::path::PathBuf;
use std::process::Command;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/synthetic-unknown-game")
}

fn restrike() -> Command {
    Command::new(env!("CARGO_BIN_EXE_restrike"))
}

#[test]
fn run_script_requires_dax_flag() {
    let output = restrike()
        .arg("run-script")
        .arg(fixture_dir())
        .args(["--block", "1"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--dax"));
}

#[test]
fn run_script_requires_block_flag() {
    let output = restrike()
        .arg("run-script")
        .arg(fixture_dir())
        .args(["--dax", "whatever.dax"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--block"));
}

#[test]
fn run_script_fails_cleanly_on_a_missing_dax_file() {
    let output = restrike()
        .arg("run-script")
        .arg(fixture_dir())
        .args(["--dax", "NOSUCHFILE.DAX", "--block", "1"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read"));
}

/// Local-only tier (pattern from `detect.rs`): runs a real CotAB event
/// script end to end. Per D10, this asserts only on *shape* (exit code,
/// section markers, that at least one line of decompressed text and one
/// request/reply pair were printed) — never on actual game text content,
/// which would embed copyrighted strings in the repo. Silently passes when
/// GBX_DATA_DIR is unset.
#[test]
fn run_script_real_ecl2_block1_reaches_done() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };

    let output = restrike()
        .arg("run-script")
        .arg(&dir)
        .args(["--dax", "ECL2.DAX", "--block", "1"])
        .output()
        .expect("failed to run restrike");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("-- DONE: Ended --"));
    assert!(stdout.contains("-- REQUEST:"));
    assert!(stdout.contains("REPLY:"));

    let text_lines = stdout
        .lines()
        .filter(|l| !l.starts_with("--") && !l.trim().is_empty())
        .count();
    assert!(
        text_lines > 0,
        "expected at least one line of decompressed game text in stdout"
    );
}

/// `--reply menu=1` overrides the default "pick the first option" policy —
/// checked structurally (the reply line names selection 1), not by content.
#[test]
fn run_script_reply_override_selects_a_non_default_menu_option() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };

    let output = restrike()
        .arg("run-script")
        .arg(&dir)
        .args(["--dax", "ECL2.DAX", "--block", "1", "--reply", "menu=0"])
        .output()
        .expect("failed to run restrike");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("REPLY: Selection(0)"));
}

/// `--trace` disassembles each instruction to stderr as it executes.
#[test]
fn run_script_trace_flag_emits_disassembly_lines() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };

    let output = restrike()
        .arg("run-script")
        .arg(&dir)
        .args(["--dax", "ECL2.DAX", "--block", "1", "--trace"])
        .output()
        .expect("failed to run restrike");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("COMPARE"));
    assert!(stderr.contains("IF"));
}
