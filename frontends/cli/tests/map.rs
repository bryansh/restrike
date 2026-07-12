use std::path::PathBuf;
use std::process::Command;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/synthetic-unknown-game")
}

fn restrike() -> Command {
    Command::new(env!("CARGO_BIN_EXE_restrike"))
}

#[test]
fn map_requires_block_flag() {
    let output = restrike()
        .arg("map")
        .arg(fixture_dir())
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--block"));
}

#[test]
fn map_fails_cleanly_on_a_missing_dax_file() {
    let output = restrike()
        .arg("map")
        .arg(fixture_dir())
        .args(["--dax", "NOSUCHFILE.DAX", "--block", "1"])
        .output()
        .expect("failed to run restrike");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read"));
}

/// Local-only tier (pattern from `detect.rs`): every GEO block in the real
/// data set renders without error — the task brief's "every GEO block in
/// the real data parses and renders without error" requirement (rendering
/// half; parsing is `gbx-formats::geo`'s own local-only test). No map
/// content is asserted on, per D10.
#[test]
fn map_renders_every_real_geo_block_without_error() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);

    // The three known CotAB block ids in GEO2.DAX (Tilverton City, Sewers,
    // Fire Knife Hideout) plus GEO3/4/5/6's contents are exercised via a
    // direct scan below rather than hard-coding ids, so this stays correct
    // if the data set ever changes.
    let mut blocks_checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("GBX_DATA_DIR must be readable") {
        let path = entry.unwrap().path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_uppercase();
        if !(name.starts_with("GEO") && name.ends_with(".DAX")) {
            continue;
        }
        let file_bytes = std::fs::read(&path).unwrap();
        let archive = gbx_formats::dax::DaxArchive::parse(&file_bytes)
            .unwrap_or_else(|e| panic!("{}: failed to parse: {e:?}", path.display()));
        for block_entry in archive.entries() {
            let output = restrike()
                .arg("map")
                .arg(&dir)
                .args(["--dax", path.file_name().unwrap().to_str().unwrap()])
                .args(["--block", &block_entry.id.to_string()])
                .output()
                .expect("failed to run restrike");
            assert!(
                output.status.success(),
                "{}#{}: {}",
                path.display(),
                block_entry.id,
                String::from_utf8_lossy(&output.stderr)
            );
            blocks_checked += 1;
        }
    }

    assert!(
        blocks_checked > 0,
        "GBX_DATA_DIR is set but no GEO*.DAX files were found"
    );
    eprintln!("rendered {blocks_checked} real GEO block(s) without error");
}
