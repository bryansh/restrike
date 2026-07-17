//! M4 combat #6 deliverables 4 & 5: the ECL `COMBAT`-opcode wiring, proven
//! end to end through the **real `gbx-vm` interpreter + engine shell**.
//!
//! A hand-authored `LOAD MONSTER; COMBAT; …` program is authored with the
//! real `EclBuilder`, decoded from a synthetic `MON2CHA.DAX`, and driven by
//! `Engine::tick` — so the fight is triggered by the game's own script (not a
//! demo caller) and the script resumes with the outcome. Everything here is
//! D10 synthetic (no game data), so it runs in CI.
//!
//! - `combat_from_a_running_script_resolves_and_resumes` (D5): the milestone —
//!   the script loads monsters, hits `COMBAT`, the fight runs through the one
//!   unified engine, and the post-`COMBAT` opcodes execute.
//! - `opcode_to_combat_path_adds_no_draw_before_initiative` (D4): the draw
//!   stream of a `LOAD MONSTER; COMBAT` program begins **exactly** with the
//!   §2 initiative fingerprint — one d6 per combatant, then the d100 selection
//!   pass — so the wiring injects/drops no setup draw before the first
//!   initiative roll (what keeps a future oracle replay honest).

#![cfg(test)]

use crate::engine::{Engine, GAME_AREA, INITIAL_ECL_BLOCK};
use crate::rng::{RngDraw, RngSink};
use crate::test_support::{build_dax_file, ecl_dax_block};
use crate::vmhost::TranscriptEntry;
use gbx_formats::font::{self, Font};
use gbx_formats::game_data::GameData;
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use gbx_formats::image::{DecodedItem, ImageBlock};
use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE};
use gbx_vm::test_support::EclBuilder;
use std::cell::RefCell;
use std::rc::Rc;

// --- fixture assets (mirrors h2_conformance's tiny synthetic set) ----------

fn synthetic_set4() -> ImageBlock {
    ImageBlock {
        height: 8,
        width_cols: 1,
        x_pos: 0,
        y_pos: 0,
        field_9: [0; 8],
        items: (0..40)
            .map(|i| DecodedItem {
                pixels: vec![(i % 16) as u8; 64],
            })
            .collect(),
    }
}

fn synthetic_font() -> Font {
    let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
    for j in 0..font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
    }
    font::decode(&data)
}

fn open_geo() -> GeoBlock {
    GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap()
}

// --- synthetic records -----------------------------------------------------

/// A raw 0x1A6 monster/character record with the combat fields this slice
/// reads poked to the given values (the same offsets `decode_char_record`
/// uses, `save_orig.rs`). A "monster" and a "party member" share this layout —
/// a monster *is* a `Player` record.
fn char_record(name: &[u8], hp: u8, raw_ac: i8, thac0: i8, movement: u8, npc: bool) -> Vec<u8> {
    let mut rec = vec![0u8; CHAR_RECORD_SIZE];
    rec[0] = name.len() as u8;
    rec[1..1 + name.len()].copy_from_slice(name);
    rec[0x73] = thac0 as u8; // thac0_base
    rec[0x78] = hp; // hit_point_max
    rec[0x1a4] = hp; // hit_point_current
    rec[0x19a] = raw_ac as u8; // ac
    rec[0x1a5] = movement; // movement
    rec[0xf7] = if npc { 0x80 } else { 0x00 }; // control_morale
                                               // attack profile 1 (a weak 1d2 fist for monsters; party uses the shell's
                                               // default weapon die regardless).
    rec[0x19c] = 1; // a1 attacks
    rec[0x19e] = 1; // a1 dice_count
    rec[0x1a0] = 2; // a1 dice_size
    rec[0x1a2] = 0; // a1 damage_bonus
    rec
}

/// A synthetic party member (`crate::party::Character`) with combat stats set,
/// via the record path so every derived field is populated exactly as an
/// imported save's would be.
fn party_member(name: &str, hp: u8, raw_ac: i8, thac0: i8) -> crate::party::Character {
    let rec = char_record(name.as_bytes(), hp, raw_ac, thac0, 12, false);
    let decoded = decode_char_record(&rec).unwrap();
    crate::party::character_from_record(&decoded, vec![], vec![])
}

/// A `GameData` with a synthetic `ECL2.DAX` (the given entry program at
/// [`INITIAL_ECL_BLOCK`]) and a `MON2CHA.DAX` whose block 0 is one weak
/// goblin record — the fixture the wiring drives.
fn combat_game_data(program: EclBuilder) -> GameData {
    let ecl_raw = vec![(INITIAL_ECL_BLOCK, ecl_dax_block(&program.build_bytes()))];
    let ecl_dax = build_dax_file(&ecl_raw);
    // A weak monster: raw AC 10 (easy to hit), THAC0 20 (near-useless vs a
    // well-armoured party), 3 HP. Block id 0 = the id `LOAD MONSTER` passes.
    let goblin = char_record(b"GOBLIN", 3, 10, 20, 6, true);
    let mon_dax = build_dax_file(&[(0u8, goblin)]);
    GameData::from_files([
        (format!("ECL{GAME_AREA}.DAX"), ecl_dax),
        (format!("MON{GAME_AREA}CHA.DAX"), mon_dax),
    ])
}

/// Builds an `Engine` running `program` as its resident block, with `party`
/// as the live roster.
fn engine_with_program(program: EclBuilder, party: Vec<crate::party::Character>) -> Engine {
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let data = combat_game_data(program);
    let mut e = Engine::new_fixture(synthetic_font(), sets, open_geo(), data, 1);
    e.party = crate::party::Party { members: party };
    e
}

// --- D5: the milestone -----------------------------------------------------

/// A `LOAD MONSTER 0, copies, 1; COMBAT; PRINT "<resume_text>"; EXIT` program.
fn load_then_combat_program(copies: u8, resume_text: &[u8]) -> EclBuilder {
    crate::test_support::simple_block(|b| {
        b.op(0x0B).imm_byte(0).imm_byte(copies).imm_byte(1); // LOAD MONSTER
        b.op(0x24); // COMBAT
        b.op(0x11).inline_str(resume_text); // PRINT — proves resume
        b.op(0x00); // EXIT
    })
}

#[test]
fn combat_from_a_running_script_resolves_and_resumes() {
    // A well-armoured party of two vs three weak goblins the script loads.
    let party = vec![
        party_member("Ravd", 40, 54, 50),
        party_member("Ilma", 38, 52, 48),
    ];
    let program = load_then_combat_program(3, b"AFTERWARD");
    let mut e = engine_with_program(program, party);

    let mut combat_line: Option<String> = None;
    let mut saw_resume_print = false;
    // Drive to completion (the block EXITs into the world menu).
    for _ in 0..3000 {
        e.tick(&[]);
        for entry in e.take_transcript() {
            match entry {
                TranscriptEntry::Request(label) if label.starts_with("combat:") => {
                    combat_line = Some(label);
                }
                TranscriptEntry::Print { text, .. } if text.contains("AFTERWARD") => {
                    // The PRINT only runs if COMBAT resumed the script.
                    if combat_line.is_some() {
                        saw_resume_print = true;
                    }
                }
                _ => {}
            }
        }
        if saw_resume_print {
            break;
        }
    }

    let combat_line = combat_line.expect("the COMBAT opcode ran a real fight (a `combat:` line)");
    assert!(
        combat_line.contains("party wins"),
        "the well-armoured party should win: got {combat_line:?}"
    );
    assert!(
        saw_resume_print,
        "the script resumed after COMBAT (the post-COMBAT PRINT executed)"
    );
    // The roster was consumed by the fight.
    assert!(
        !e.state().pending_combat.monsters_loaded,
        "monstersLoaded is cleared once the fight consumes the roster"
    );
    assert!(!e.state().party_killed, "the party survived");
}

// --- D4: draw parity -------------------------------------------------------

/// A trivial `RngSink` recording every draw's operand `n`.
#[derive(Clone, Default)]
struct DrawTap {
    draws: Rc<RefCell<Vec<RngDraw>>>,
}
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.draws.borrow_mut().push(draw);
    }
}

#[test]
fn opcode_to_combat_path_adds_no_draw_before_initiative() {
    // 2 party + 3 monsters = 5 in-combat combatants → the §2 fingerprint is
    // 5 leading d6 (initiative) then a d100 selection pass. If any setup step
    // (load_monster decode, terrain, encounter distance, placement) drew, it
    // would appear ahead of the first d6 and break this.
    let party = vec![
        party_member("Ravd", 40, 54, 50),
        party_member("Ilma", 38, 52, 48),
    ];
    let program = load_then_combat_program(3, b"AFTERWARD");
    let mut e = engine_with_program(program, party);

    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    e.attach_rng_sink(Box::new(tap));

    for _ in 0..3000 {
        e.tick(&[]);
        if !draws.borrow().is_empty()
            && e.take_transcript()
                .iter()
                .any(|t| matches!(t, TranscriptEntry::Request(l) if l.starts_with("combat:")))
        {
            break;
        }
    }

    let draws = draws.borrow();
    assert!(!draws.is_empty(), "the fight drew from the PRNG");
    let ops: Vec<Option<u16>> = draws.iter().map(|d| d.n).collect();
    // The first five draws are the initiative d6s — no setup draw precedes them.
    for (i, n) in ops.iter().take(5).enumerate() {
        assert_eq!(
            *n,
            Some(6),
            "draw #{i} must be an initiative d6 (n=6), got n={n:?}; a setup draw leaked in ahead of initiative"
        );
    }
    // The initiative phase is exactly 5 d6s, then the selection phase's d100s
    // begin (§2: K_c d6 then (A+1)·K d100).
    assert_eq!(
        ops[5],
        Some(100),
        "after the 5 initiative d6s the d100 selection pass begins, got n={:?}",
        ops[5]
    );
}
