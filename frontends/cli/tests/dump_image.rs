use std::path::PathBuf;
use std::process::Command;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/synthetic-unknown-game")
}

fn restrike() -> Command {
    Command::new(env!("CARGO_BIN_EXE_restrike"))
}

#[test]
fn dump_image_requires_dax_flag() {
    let output = restrike()
        .arg("dump-image")
        .arg(fixture_dir())
        .args(["--block", "1", "--out", "/dev/null"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--dax"));
}

#[test]
fn dump_image_requires_block_flag() {
    let output = restrike()
        .arg("dump-image")
        .arg(fixture_dir())
        .args(["--dax", "SOMETHING.DAX", "--out", "/dev/null"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--block"));
}

#[test]
fn dump_image_requires_out_flag() {
    let output = restrike()
        .arg("dump-image")
        .arg(fixture_dir())
        .args(["--dax", "SOMETHING.DAX", "--block", "1"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--out"));
}

#[test]
fn dump_image_fails_cleanly_on_a_missing_dax_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.ppm");
    let output = restrike()
        .arg("dump-image")
        .arg(fixture_dir())
        .args(["--dax", "NOSUCHFILE.DAX", "--block", "1"])
        .args(["--out", out.to_str().unwrap()])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read"));
}

/// Local-only tier (pattern from `map.rs`): a known real BIGPIC block dumps
/// to a valid, non-trivial-sized binary PPM without error. No image content
/// is asserted on beyond the PPM header (per D10, no game-data-derived
/// pixels get hard-coded into this repo).
#[test]
fn dump_image_writes_a_valid_ppm_for_a_real_bigpic_block() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    if !dir.join("BIGPIC1.DAX").is_file() {
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("bigpic.ppm");
    let output = restrike()
        .arg("dump-image")
        .arg(&dir)
        .args(["--dax", "BIGPIC1.DAX", "--block", "121", "--mask", "0"])
        .args(["--out", out.to_str().unwrap()])
        .output()
        .expect("failed to run restrike");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let bytes = std::fs::read(&out).expect("PPM must have been written");
    assert!(
        bytes.starts_with(b"P6\n"),
        "not a binary PPM: {:?}",
        &bytes[..bytes.len().min(16)]
    );
    assert!(
        bytes.len() > 100,
        "PPM suspiciously small: {} bytes",
        bytes.len()
    );
}

/// Local-only tier: every real image container format (8X8D/BIGPIC/HEAD/
/// BODY/SKY/PIC/SPRIT) dumps at least one block through the CLI without
/// error — the plumbing half of "decode+write a PPM"; the decoders'
/// own local-only tests (`gbx-formats`) cover exhaustive per-block sweeps.
#[test]
fn dump_image_handles_every_container_kind_present_in_real_data() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);

    let candidates = [
        ("8X8D1.DAX", 202u8, false),
        ("BIGPIC1.DAX", 121, false),
        ("HEAD2.DAX", 0, false),
        ("BODY2.DAX", 0, false),
        ("SKY.DAX", 251, false),
        ("PIC1.DAX", 1, true),
        ("SPRIT1.DAX", 2, true),
    ];

    let mut checked = 0usize;
    for (file, block, is_anim) in candidates {
        if !dir.join(file).is_file() {
            continue;
        }
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.ppm");
        let mut cmd = restrike();
        cmd.arg("dump-image")
            .arg(&dir)
            .args(["--dax", file, "--block", &block.to_string()])
            .args(["--out", out.to_str().unwrap()]);
        if is_anim {
            cmd.args(["--frame", "0"]);
        }
        let output = cmd.output().expect("failed to run restrike");
        assert!(
            output.status.success(),
            "{file}#{block}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        checked += 1;
    }

    assert!(
        checked > 0,
        "GBX_DATA_DIR is set but none of the expected files were found"
    );
    eprintln!("dump-image handled {checked} real container(s) without error");
}
