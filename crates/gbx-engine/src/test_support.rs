//! Shared test-only helpers for building synthetic `GameData` backed by
//! real `EclBuilder`-authored ECL blocks (M2 step 4's H2 conformance work,
//! task deliverable 5) — used by `shell.rs`, `engine.rs`, `walk_goldens.rs`,
//! and `demo.rs`'s test modules. Everything here is D10 synthetic (hand-
//! authored bytes), never derived from real game data.

#![cfg(test)]

use gbx_formats::game_data::GameData;
use gbx_vm::test_support::EclBuilder;

/// Authors a real, resolvable 5-slot vector header (`vm-scriptmemory.md`
/// §1's table: `0`=`vm_run_addr_1`, `1`=`SearchLocationAddr`,
/// `2`=`PreCampCheckAddr`, `3`=`CampInterruptedAddr`,
/// `4`=`ecl_initial_entryPoint`) ahead of `body`'s labeled code —
/// `EclBuilder` itself has no header-vector helper (its own conformance
/// tests always `machine.enter(addr)` a known label directly, bypassing the
/// header this session's `shell.rs` flows resolve through
/// `machine.vector(index)`). `vectors[i]` names which label header slot `i`
/// points at. Mirrors `read_header_vectors`'s exact layout: each slot is
/// one unread anchor byte + one `imm_word` operand (mode `0x02`) naming the
/// label.
pub fn labeled_block(vectors: [&str; 5], body: impl FnOnce(&mut EclBuilder)) -> EclBuilder {
    let mut b = EclBuilder::new();
    for label in vectors {
        b.raw(&[0]); // the unread anchor byte each header slot skips
        b.imm_word_label(label);
    }
    body(&mut b);
    b
}

/// All 5 header vectors point at the same `entry` label — the common case
/// where every flow site fires the same code path.
pub fn simple_block(body: impl FnOnce(&mut EclBuilder)) -> EclBuilder {
    labeled_block(["entry"; 5], |b| {
        b.label("entry");
        body(b);
    })
}

/// A one-instruction EXIT block — the default "vector resolved, does
/// nothing" fixture.
pub fn exit_only_block() -> EclBuilder {
    simple_block(|b| {
        b.op(0x00); // EXIT
    })
}

/// `load_ecl_dax`'s 2-byte prefix (`vmhost.rs`'s citation) — every
/// synthetic ECL block needs this ahead of its real bytecode.
pub(crate) fn ecl_dax_block(bytecode: &[u8]) -> Vec<u8> {
    let mut v = vec![0u8, 0u8];
    v.extend_from_slice(bytecode);
    v
}

/// RLE-encodes `raw` as a literal-run stream (`dax.rs`'s format, mirroring
/// `gbx-formats/src/game_data.rs`'s own test helper).
fn rle_encode_literal(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in raw.chunks(128) {
        out.push((chunk.len() - 1) as u8);
        out.extend_from_slice(chunk);
    }
    out
}

/// A synthetic multi-block DAX file (`dax.rs`'s layout) — `gbx-formats::dax`
/// ships no public encoder by design (D10: only test code ever needs to
/// *write* the format).
pub(crate) fn build_dax_file(blocks: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut entries = Vec::new();
    let mut data_area = Vec::new();
    for (id, raw) in blocks {
        let offset = data_area.len() as u32;
        let comp = rle_encode_literal(raw);
        entries.push((*id, offset, raw.len() as u16, comp.len() as u16));
        data_area.extend_from_slice(&comp);
    }
    let header_bytes = (entries.len() * 9) as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&header_bytes.to_le_bytes());
    for (id, offset, raw_size, comp_size) in &entries {
        out.push(*id);
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&raw_size.to_le_bytes());
        out.extend_from_slice(&comp_size.to_le_bytes());
    }
    out.extend_from_slice(&data_area);
    out
}

/// Builds a `GameData` holding one `"ECL{game_area}.DAX"` file with every
/// `(id, builder)` pair as a real, `load_ecl_dax`-shaped block.
pub fn ecl_game_data(game_area: u8, blocks: Vec<(u8, EclBuilder)>) -> GameData {
    let raw_blocks: Vec<(u8, Vec<u8>)> = blocks
        .iter()
        .map(|(id, b)| (*id, ecl_dax_block(&b.build_bytes())))
        .collect();
    let dax_bytes = build_dax_file(&raw_blocks);
    GameData::from_files([(format!("ECL{game_area}.DAX"), dax_bytes)])
}
