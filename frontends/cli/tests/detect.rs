use std::path::PathBuf;
use std::process::Command;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/synthetic-unknown-game")
}

#[test]
fn detect_reports_unknown_game_on_synthetic_fixture() {
    let output = Command::new(env!("CARGO_BIN_EXE_restrike"))
        .arg("detect")
        .arg(fixture_dir())
        .output()
        .expect("failed to run restrike");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unknown game"));
    assert!(stdout.contains("blob.bin"));
}

#[test]
fn detect_falls_back_to_gbx_data_dir_env_var() {
    let output = Command::new(env!("CARGO_BIN_EXE_restrike"))
        .arg("detect")
        .env("GBX_DATA_DIR", fixture_dir())
        .output()
        .expect("failed to run restrike");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unknown game"));
}

#[test]
fn detect_fails_cleanly_with_no_dir_and_no_env_var() {
    let output = Command::new(env!("CARGO_BIN_EXE_restrike"))
        .arg("detect")
        .env_remove("GBX_DATA_DIR")
        .output()
        .expect("failed to run restrike");

    assert!(!output.status.success());
}
