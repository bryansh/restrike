//! ★ **Roll-credits slice 5's draw-neutrality pin** (`docs/design/roll-credits.md`
//! §9.1/§9.2).
//!
//! Slice 5 grew `gbl.spellCastingTable`'s transcribed set from three rows to
//! twenty-three, and every new row can plant an affect, roll dice, or accept a
//! spell the selection AI would previously have rejected. The claim that none
//! of that can move a captured draw rests on one fact, and this test *is* that
//! fact, checked against the capture files themselves rather than asserted in
//! prose:
//!
//! > **No pinned capture's roster memorizes any spell outside `{0x03, 0x0F,
//! > 0x17}`** — the three rows that already existed.
//!
//! The only routes into `spell_entry` are (a) an id pulled from a combatant's
//! `memorized_list`, which `sub_3560B` collects from that combatant's own
//! character record (`record[0x1E..=0x71]`, doc §41.1), and (b) a `CastSpell`
//! the player issues, which no replay does. So if every record's memorized set
//! is a subset of the old three, no new row is reachable from any capture — the
//! guard's 16/16 and the reel's live draw equality then confirm it end to end.
//!
//! D10: reads only `~/goldbox-data` at runtime and asserts ids, never bytes;
//! loud-skips per capture when a file is absent, so plain CI stays green.

use gbx_formats::save_orig::decode_char_record;
use gbx_oracle::replay;
use gbx_oracle::Trace;

/// `Classes/SpellList.cs:54-63` (`LearntList`) over the raw 84-byte array at
/// record offset `0x1E`: every non-zero entry whose high bit is clear.
fn memorized(spell_list: &[u8]) -> Vec<u8> {
    spell_list
        .iter()
        .copied()
        .filter(|&b| b != 0 && b < 0x80)
        .collect()
}

/// `SpellList.LearningList` (`:65-74`) — staged, not yet committed. A capture
/// carrying one would mean a mid-rest snapshot; none does, and a staged id is
/// invisible to `sub_3560B` anyway (it collects `LearntList`).
fn staged(spell_list: &[u8]) -> Vec<u8> {
    spell_list
        .iter()
        .copied()
        .filter(|&b| b > 0x80)
        .map(|b| b & 0x7F)
        .collect()
}

#[test]
fn the_captures_only_ever_memorize_the_three_old_rows() {
    let Some(dir) = replay::traces_dir() else {
        eprintln!(
            "SKIPPED: needs the local traces dir (set GBX_DATA_DIR or \
             GBX_TRACES_DIR; captures are local-only, D10)"
        );
        return;
    };
    let old_rows = gbx_engine::spells::PRE_SLICE5_IDS;
    let mut checked = 0;
    let mut seen: Vec<u8> = Vec::new();

    for capture in replay::sidecar::pinned_captures() {
        let path = dir.join(capture);
        if !path.exists() {
            eprintln!("SKIPPED (absent, D10): {capture}");
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("capture readable");
        let trace = Trace::parse(&text).expect("capture parses");
        let entry = trace
            .combat_entry()
            .expect("capture carries a combat_entry snapshot");
        for (id, c) in entry.combatants.iter().enumerate() {
            let record = decode_char_record(&c.record).expect("record decodes");
            for spell in memorized(&record.spell_list) {
                assert!(
                    old_rows.contains(&spell),
                    "{capture}: combatant {id} ({}) memorizes {spell:#04x}, which is \
                     OUTSIDE the pre-slice-5 set {old_rows:?} — slice 5's \
                     draw-neutrality argument no longer holds and this capture must \
                     be re-peeled against the new row",
                    record.name
                );
                if !seen.contains(&spell) {
                    seen.push(spell);
                }
            }
            assert!(
                staged(&record.spell_list).is_empty(),
                "{capture}: combatant {id} ({}) carries STAGED spells — no capture \
                 is a mid-rest snapshot",
                record.name
            );
        }
        checked += 1;
    }

    if checked == 0 {
        eprintln!("SKIPPED: no capture files present (D10)");
        return;
    }
    seen.sort_unstable();
    eprintln!(
        "spell rows: {checked} captures checked, memorized ids in play = {seen:?} \
         (all inside the pre-slice-5 set)"
    );
    // The captures do exercise all three old rows — a vacuous pass (nobody
    // memorizes anything) would not be evidence of anything.
    assert!(
        !seen.is_empty(),
        "the captures memorize nothing at all — the pin proves nothing"
    );
}
