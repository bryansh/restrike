//! Monster / encounter data layer (M4 step 5, `docs/design/combat-study.md` §8).
//!
//! **Provenance (traced this session, read-for-behavior per D11):** the ECL
//! `MONSTER` opcode handler `CMD_LoadMonster` (coab `ovr003.cs:238-296`) reads a
//! monster id operand and calls `ovr017.load_mob(mod_id)` (`ovr003.cs:247`).
//! `load_mob` (`ovr017.cs:824`) loads the record from a DAX archive named
//! `MON<area>CHA.dax` (area = `gbl.game_area`, 1..6) and decodes it with
//! `new Player(data, 0)` — i.e. **a monster is a full `Player` record**,
//! `StructSize = 0x1A6` (`Classes/Player.cs:708/715`), byte-identical in layout
//! to an on-disk `CHRDAT` save record. Two companion archives carry per-id
//! extras: `MON<area>SPC.dax` (innate `Affect`s, 9-byte records) and
//! `MON<area>ITM.dax` (carried `Item`s, `Item.StructSize` records).
//!
//! Consequence: the record decode is exactly [`crate::save_orig::decode_char_record`]
//! (`CHAR_RECORD_SIZE = 0x1A6`). This module is a thin monster-facing layer over
//! it — it locates the DAX blocks, decodes each as a [`crate::save_orig::CharRecord`],
//! and exposes the combat-relevant view (`ac`/`thac0`/hit dice/two attack
//! profiles/turn-undead type/monster type/morale). **Data only, no behavior** —
//! combat resolution lives in `gbx-engine` once the implement-to-parity sessions
//! land (D-OR5(a)).
//!
//! Pure over bytes (no filesystem access — the crate convention): callers pull
//! the `MON*.DAX` files out of a loaded directory and hand the bytes in.

use crate::dax::{DaxArchive, DaxError};
use crate::save_orig::{
    decode_char_record, read_affects, read_items, CharRecord, CHAR_RECORD_SIZE,
};

/// Which of the three per-area monster DAX files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonsterFile {
    /// `MON<area>CHA.DAX` — the 0x1A6 character/monster records.
    Cha,
    /// `MON<area>SPC.DAX` — innate affects (9-byte records).
    Spc,
    /// `MON<area>ITM.DAX` — carried items.
    Itm,
}

impl MonsterFile {
    fn suffix(self) -> &'static str {
        match self {
            MonsterFile::Cha => "CHA",
            MonsterFile::Spc => "SPC",
            MonsterFile::Itm => "ITM",
        }
    }
}

/// The on-disk file name for one per-area monster file, e.g.
/// `monster_filename(2, MonsterFile::Cha)` → `"MON2CHA.DAX"`. Uppercase `.DAX`
/// to match the real GOG files (coab's source string is lowercase `.dax`, but
/// `load_decode_dax` resolves case-insensitively; the shipped files are
/// uppercase — `MON{1..6}{CHA,SPC,ITM}.DAX`).
pub fn monster_filename(area: u8, kind: MonsterFile) -> String {
    format!("MON{area}{}.DAX", kind.suffix())
}

/// Everything that can go wrong reading a monster archive. DAX-container and
/// record-decode failures are wrapped; the monster-specific case is a CHA block
/// too small to hold a `Player` record (a real find worth surfacing, per the
/// session brief, not a silent skip).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonsterError {
    /// The `MON*.DAX` container itself failed to parse or a block failed to
    /// decompress.
    Dax(DaxError),
    /// A CHA block decompressed to fewer than `0x1A6` bytes — not a full
    /// `Player` record. `load_mob` reads a fixed `new Player(data, 0)`, so a
    /// short block is malformed data, not a variant record.
    ChaBlockTooShort { id: u8, len: usize },
    /// A CHA block's record body failed to decode (should not happen once the
    /// length check passes — `decode_char_record` only rejects on wrong size).
    RecordDecode { id: u8 },
    /// An SPC/ITM block's length is not a whole number of fixed records.
    CompanionRecordMisaligned {
        id: u8,
        len: usize,
        record_size: usize,
    },
}

impl From<DaxError> for MonsterError {
    fn from(e: DaxError) -> Self {
        MonsterError::Dax(e)
    }
}

/// One monster's attack profile. A `Player`/monster carries **two**
/// (`Classes/Player.cs:646-703`); the live (current) values sit in the 8-byte
/// run at record offset `0x19c` — `[a1_left, a2_left, a1_count, a2_count,
/// a1_size, a2_size, a1_bonus, a2_bonus]`. A damage attack rolls
/// `roll_dice(dice_size, dice_count)` then adds `damage_bonus`
/// (`sub_3E192`, `ovr014.cs:86-87`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackProfile {
    pub attacks: u8,
    pub dice_count: u8,
    pub dice_size: u8,
    pub damage_bonus: i8,
}

impl AttackProfile {
    /// A profile with a real damage die (both dimensions non-zero).
    pub fn is_present(&self) -> bool {
        self.dice_count > 0 && self.dice_size > 0
    }

    /// Maximum `roll_dice(dice_size, dice_count)` total **before** the bonus —
    /// `dice_count · dice_size`. The FD-29 truncation is observable only when
    /// this exceeds 255 (coab returns `(byte)roll_total`, `ovr024.cs:595`).
    /// Widened to `u16` so the product itself cannot wrap.
    pub fn max_roll(&self) -> u16 {
        self.dice_count as u16 * self.dice_size as u16
    }
}

/// A loaded monster record — the full 0x1A6 `Player` record ([`CharRecord`])
/// plus a monster-facing accessor surface. Named `MonsterRecord` because that is
/// how the engine consumes it, but the evidence is explicit that the underlying
/// bytes are a character record (`load_mob` → `new Player(data, 0)`).
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterRecord {
    /// The full decoded character record. Every combat field is here; the
    /// accessors below name the combat-relevant subset.
    pub record: CharRecord,
}

impl MonsterRecord {
    /// Decodes one CHA block. `bytes` must be at least `0x1A6` long; only the
    /// first `0x1A6` bytes are read (`load_mob` reads `new Player(data, 0)` and
    /// ignores any trailing bytes).
    pub fn from_cha_block(id: u8, bytes: &[u8]) -> Result<Self, MonsterError> {
        if bytes.len() < CHAR_RECORD_SIZE {
            return Err(MonsterError::ChaBlockTooShort {
                id,
                len: bytes.len(),
            });
        }
        let record = decode_char_record(&bytes[..CHAR_RECORD_SIZE])
            .map_err(|_| MonsterError::RecordDecode { id })?;
        Ok(MonsterRecord { record })
    }

    /// The monster's name (`@0x00`). Game data — never emitted into CI output or
    /// a committed golden; present for the engine's in-combat display.
    pub fn name(&self) -> &str {
        &self.record.name
    }

    /// Hit dice (`@0xe5`).
    pub fn hit_dice(&self) -> u8 {
        self.record.hit_dice
    }

    /// Maximum hit points (`@0x78`, a byte — HP is stored, not re-rolled at load).
    pub fn hit_point_max(&self) -> u8 {
        self.record.hit_point_max
    }

    /// Raw armor class (`@0x19a`). Lower is better; see [`Self::display_ac`].
    pub fn ac(&self) -> i8 {
        self.record.ac
    }

    /// Displayed armor class = `0x3C - ac` (`Classes/Player.cs:598`).
    pub fn display_ac(&self) -> i16 {
        0x3C - self.record.ac as i16
    }

    /// Base THAC0 (`@0x73`).
    pub fn thac0(&self) -> i8 {
        self.record.thac0_base
    }

    /// Turn-undead type index (`field_E9` `@0xe9`) — the value
    /// `turns_undead` multiplies by 10 to index `unk_16679`
    /// (`ovr014.cs:642`). The image table holds 11 rows (types 0..10); this is
    /// the byte FD-20 censuses for values ≥ 11.
    pub fn turn_undead_type(&self) -> u8 {
        self.record.field_e9
    }

    /// Monster type (`@0x11a`) — the family enum used by spell/turn special
    /// cases (`MonsterType`, e.g. troll/animated-dead branches, `ovr013.cs`).
    pub fn monster_type(&self) -> u8 {
        self.record.monster_type
    }

    /// Control/morale byte (`@0xf7`); `>= 0x80` marks an NPC/AI combatant
    /// (`Control.cs`), which is what gates the morale checks (§6).
    pub fn control_morale(&self) -> u8 {
        self.record.control_morale
    }

    /// Whether this record is AI-controlled (`control_morale >= 0x80`).
    pub fn is_npc(&self) -> bool {
        self.record.control_morale >= 0x80
    }

    /// Movement/initiative base (`@0x1a5`).
    pub fn movement(&self) -> u8 {
        self.record.movement
    }

    /// The two attack profiles, decoded from the live 8-byte run at `@0x19c`.
    pub fn attacks(&self) -> [AttackProfile; 2] {
        let a = &self.record.attack_profile_current;
        [
            AttackProfile {
                attacks: a[0],
                dice_count: a[2],
                dice_size: a[4],
                damage_bonus: a[6] as i8,
            },
            AttackProfile {
                attacks: a[1],
                dice_count: a[3],
                dice_size: a[5],
                damage_bonus: a[7] as i8,
            },
        ]
    }

    /// Largest single-attack `roll_dice` total (pre-bonus) across both
    /// profiles — the FD-29 data-driven extent for this monster.
    pub fn max_damage_roll(&self) -> u16 {
        self.attacks()
            .iter()
            .map(AttackProfile::max_roll)
            .max()
            .unwrap_or(0)
    }
}

/// One decoded entry from a `MON<area>CHA.DAX` archive: the DAX block id (the
/// monster id the `MONSTER` opcode passes) and the decoded record.
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterEntry {
    pub id: u8,
    pub monster: MonsterRecord,
}

/// Parses a `MON<area>CHA.DAX` archive into its monster records, sorted by
/// block id. Each block is decoded as a 0x1A6 `Player` record.
pub fn parse_cha_archive(cha_bytes: &[u8]) -> Result<Vec<MonsterEntry>, MonsterError> {
    let archive = DaxArchive::parse(cha_bytes)?;
    let mut ids: Vec<u8> = archive.entries().iter().map(|e| e.id).collect();
    ids.sort_unstable();

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let block = archive.block_data(id)?;
        let monster = MonsterRecord::from_cha_block(id, &block)?;
        out.push(MonsterEntry { id, monster });
    }
    Ok(out)
}

/// One companion (SPC or ITM) archive entry: the block id and its fixed-size
/// records, kept **opaque** (affect/item interiors are decoded by their own
/// layers, not here — the same posture `save_orig` takes for `.fx`/`.swg`).
#[derive(Debug, Clone, PartialEq)]
pub struct CompanionEntry {
    pub id: u8,
    pub records: Vec<Vec<u8>>,
}

/// Parses a `MON<area>SPC.DAX` archive into per-id opaque 9-byte affect records.
pub fn parse_spc_archive(spc_bytes: &[u8]) -> Result<Vec<CompanionEntry>, MonsterError> {
    parse_companion(spc_bytes, MonsterFile::Spc)
}

/// Parses a `MON<area>ITM.DAX` archive into per-id opaque item records.
pub fn parse_itm_archive(itm_bytes: &[u8]) -> Result<Vec<CompanionEntry>, MonsterError> {
    parse_companion(itm_bytes, MonsterFile::Itm)
}

fn parse_companion(bytes: &[u8], kind: MonsterFile) -> Result<Vec<CompanionEntry>, MonsterError> {
    let archive = DaxArchive::parse(bytes)?;
    let mut ids: Vec<u8> = archive.entries().iter().map(|e| e.id).collect();
    ids.sort_unstable();

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let block = archive.block_data(id)?;
        let split = match kind {
            MonsterFile::Spc => read_affects(&block),
            MonsterFile::Itm => read_items(&block),
            MonsterFile::Cha => unreachable!("parse_companion is SPC/ITM only"),
        };
        let records = split.map_err(|_| MonsterError::CompanionRecordMisaligned {
            id,
            len: block.len(),
            record_size: match kind {
                MonsterFile::Spc => 9,
                _ => crate::save_orig::ITEM_RECORD_SIZE,
            },
        })?;
        out.push(CompanionEntry { id, records });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // Synthetic CI fixtures (D10-clean — self-authored bytes, no game data).
    // -----------------------------------------------------------------

    const HEADER_ENTRY_SIZE: usize = 9;

    /// Builds a DAX container from `(id, raw_bytes)` blocks, RLE-encoding each
    /// as literal runs (≤128 bytes/run). Mirrors the dax.rs test helper.
    fn build_dax(blocks: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let header_bytes = blocks.len() * HEADER_ENTRY_SIZE;
        let mut data_area = Vec::new();
        let mut entries = Vec::new();
        for (id, raw) in blocks {
            let offset = data_area.len() as u32;
            let mut comp = Vec::new();
            for chunk in raw.chunks(128) {
                comp.push((chunk.len() - 1) as u8);
                comp.extend_from_slice(chunk);
            }
            entries.push((*id, offset, raw.len() as u16, comp.len() as u16));
            data_area.extend_from_slice(&comp);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&(header_bytes as u16).to_le_bytes());
        for (id, offset, raw_size, comp_size) in entries {
            out.push(id);
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&raw_size.to_le_bytes());
            out.extend_from_slice(&comp_size.to_le_bytes());
        }
        out.extend_from_slice(&data_area);
        out
    }

    /// A synthetic 0x1A6 monster record with known values at the combat offsets.
    fn synthetic_monster(
        name: &str,
        turn_type: u8,
        ac: i8,
        a1: (u8, u8, u8, i8),
        a2: (u8, u8, u8, i8),
    ) -> Vec<u8> {
        let mut b = vec![0u8; CHAR_RECORD_SIZE];
        b[0] = name.len() as u8;
        b[1..1 + name.len()].copy_from_slice(name.as_bytes());
        b[0x73] = 0x0Au8; // thac0 base (arbitrary)
        b[0xe5] = 3; // hit dice
        b[0x78] = 24; // hp max
        b[0xe9] = turn_type; // field_E9 (FD-20)
        b[0xf7] = 0x80; // control_morale → NPC
        b[0x11a] = 5; // monster type
        b[0x19a] = ac as u8; // ac
        b[0x1a5] = 9; // movement
                      // Live attack run @0x19c: [a1_left,a2_left,a1_cnt,a2_cnt,a1_size,a2_size,a1_bonus,a2_bonus]
        b[0x19c] = a1.0; // a1 attacks
        b[0x19d] = a2.0; // a2 attacks
        b[0x19e] = a1.1; // a1 count
        b[0x19f] = a2.1; // a2 count
        b[0x1a0] = a1.2; // a1 size
        b[0x1a1] = a2.2; // a2 size
        b[0x1a2] = a1.3 as u8; // a1 bonus
        b[0x1a3] = a2.3 as u8; // a2 bonus
        b
    }

    #[test]
    fn filename_matches_the_shipped_pattern() {
        assert_eq!(monster_filename(2, MonsterFile::Cha), "MON2CHA.DAX");
        assert_eq!(monster_filename(6, MonsterFile::Spc), "MON6SPC.DAX");
        assert_eq!(monster_filename(1, MonsterFile::Itm), "MON1ITM.DAX");
    }

    #[test]
    fn decodes_cha_archive_and_combat_fields() {
        let m0 = synthetic_monster("ORC", 0, 6, (1, 1, 8, 1), (0, 0, 0, 0));
        let m1 = synthetic_monster("WIGHT", 4, 4, (1, 2, 4, 0), (1, 1, 6, 2));
        let cha = build_dax(&[(0, m0), (1, m1)]);

        let entries = parse_cha_archive(&cha).unwrap();
        assert_eq!(entries.len(), 2);

        let orc = &entries[0].monster;
        assert_eq!(entries[0].id, 0);
        assert_eq!(orc.name(), "ORC");
        assert_eq!(orc.turn_undead_type(), 0);
        assert_eq!(orc.ac(), 6);
        assert_eq!(orc.display_ac(), 0x3C - 6);
        assert_eq!(orc.hit_dice(), 3);
        assert!(orc.is_npc());
        let a = orc.attacks();
        assert_eq!(
            a[0],
            AttackProfile {
                attacks: 1,
                dice_count: 1,
                dice_size: 8,
                damage_bonus: 1
            }
        );
        assert!(a[0].is_present());
        assert!(!a[1].is_present());
        assert_eq!(orc.max_damage_roll(), 8); // 1×8

        let wight = &entries[1].monster;
        assert_eq!(wight.turn_undead_type(), 4);
        assert_eq!(
            wight.attacks()[1],
            AttackProfile {
                attacks: 1,
                dice_count: 1,
                dice_size: 6,
                damage_bonus: 2
            }
        );
        assert_eq!(wight.max_damage_roll(), 8); // max(2×4, 1×6) = 8
    }

    #[test]
    fn short_cha_block_is_a_loud_error_not_a_skip() {
        let cha = build_dax(&[(0, vec![0u8; 0x100])]); // < 0x1A6
        let err = parse_cha_archive(&cha).unwrap_err();
        assert_eq!(err, MonsterError::ChaBlockTooShort { id: 0, len: 0x100 });
    }

    #[test]
    fn trailing_bytes_after_the_record_are_ignored() {
        // A block longer than 0x1A6 decodes from offset 0, like load_mob.
        let mut raw = synthetic_monster("KOBOLD", 0, 7, (1, 1, 4, 0), (0, 0, 0, 0));
        raw.extend_from_slice(&[0xFF; 16]);
        let cha = build_dax(&[(3, raw)]);
        let entries = parse_cha_archive(&cha).unwrap();
        assert_eq!(entries[0].monster.name(), "KOBOLD");
        assert_eq!(entries[0].monster.attacks()[0].dice_size, 4);
    }

    #[test]
    fn companion_archives_split_into_opaque_records() {
        // SPC: two 9-byte affect records in one block.
        let spc = build_dax(&[(0, vec![7u8; 18])]);
        let spc_entries = parse_spc_archive(&spc).unwrap();
        assert_eq!(spc_entries[0].records.len(), 2);
        assert_eq!(spc_entries[0].records[0].len(), 9);

        // ITM: one item record.
        let itm = build_dax(&[(0, vec![1u8; crate::save_orig::ITEM_RECORD_SIZE])]);
        let itm_entries = parse_itm_archive(&itm).unwrap();
        assert_eq!(itm_entries[0].records.len(), 1);
    }

    #[test]
    fn misaligned_companion_block_is_a_loud_error() {
        let spc = build_dax(&[(0, vec![7u8; 10])]); // not a multiple of 9
        let err = parse_spc_archive(&spc).unwrap_err();
        assert_eq!(
            err,
            MonsterError::CompanionRecordMisaligned {
                id: 0,
                len: 10,
                record_size: 9
            }
        );
    }

    // -----------------------------------------------------------------
    // Local-tier real-data census (GBX_DATA_DIR-gated, loud-skip when absent).
    // Settles FD-20 (field_E9 ≥ 11?) and FD-29's data-driven `roll_dice`
    // extent clause against the real `MON{1..6}CHA.DAX`, and pins a SHA-256 of
    // the *derived numeric facts* (D10 — never raw bytes, never monster names).
    // -----------------------------------------------------------------

    /// SHA-256 of the canonical derived-facts summary over all six areas.
    /// Re-derived from the user's own `MON*CHA.DAX`; loud-fails on mismatch so a
    /// different data version is re-pinned, not silently trusted.
    const DERIVED_FACTS_SHA256: &str =
        "728ebfb6bcdc51051bffe5b151e7ae03c06045c599e382a85b4f6970f97c1931";

    #[test]
    fn local_tier_monster_census() {
        use sha2::{Digest, Sha256};

        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!(
                "SKIPPED: local tier needs GBX_DATA_DIR (monster::local_tier_monster_census)"
            );
            return;
        };
        let dir = std::path::PathBuf::from(dir);

        // Canonical, numbers-only fact stream. No names, no raw bytes (D10).
        let mut facts = String::new();
        let mut global_max_turn_type: u8 = 0;
        let mut global_max_damage_roll: u16 = 0;
        let mut global_max_hit_dice: u8 = 0;
        let mut global_max_monster_type: u8 = 0;
        let mut nonzero_e9_count = 0usize;
        let mut total_monsters = 0usize;

        for area in 1u8..=6 {
            let path = dir.join(monster_filename(area, MonsterFile::Cha));
            let bytes =
                std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let entries = parse_cha_archive(&bytes)
                .unwrap_or_else(|e| panic!("parse {}: {e:?}", path.display()));

            facts.push_str(&format!("area {area}: {} monsters\n", entries.len()));
            total_monsters += entries.len();

            for e in &entries {
                let m = &e.monster;
                let [a1, a2] = m.attacks();
                global_max_turn_type = global_max_turn_type.max(m.turn_undead_type());
                global_max_damage_roll = global_max_damage_roll.max(m.max_damage_roll());
                global_max_hit_dice = global_max_hit_dice.max(m.hit_dice());
                global_max_monster_type = global_max_monster_type.max(m.monster_type());
                if m.turn_undead_type() != 0 {
                    nonzero_e9_count += 1;
                }
                // Per-monster derived facts — numbers only, deterministic order.
                facts.push_str(&format!(
                    "  id={} hd={} hp={} ac={} thac0={} e9={} mtype={} morale={} mv={} \
a1={},{}x{}+{} a2={},{}x{}+{}\n",
                    e.id,
                    m.hit_dice(),
                    m.hit_point_max(),
                    m.ac(),
                    m.thac0(),
                    m.turn_undead_type(),
                    m.monster_type(),
                    m.control_morale(),
                    m.movement(),
                    a1.attacks,
                    a1.dice_count,
                    a1.dice_size,
                    a1.damage_bonus,
                    a2.attacks,
                    a2.dice_count,
                    a2.dice_size,
                    a2.damage_bonus,
                ));
            }
        }

        facts.push_str(&format!(
            "TOTAL monsters={total_monsters} max_turn_type={global_max_turn_type} \
max_damage_roll={global_max_damage_roll} max_hit_dice={global_max_hit_dice}\n"
        ));

        // FD-20: does any monster's field_E9 reach the image table's out-of-range?
        // The turn-undead table holds 11 rows (types 0..10).
        eprintln!(
            "FD-20 census: max field_E9 across all MON*CHA = {global_max_turn_type} \
(image table covers 0..10; ≥11 would read out of range)"
        );
        // FD-29: does any single monster damage roll exceed the 255 byte-truncation threshold?
        eprintln!(
            "FD-29 census: max monster damage roll (count×size) = {global_max_damage_roll} \
(byte truncation observable only if >255)"
        );
        eprintln!(
            "sanity: {total_monsters} monsters, {nonzero_e9_count} with field_E9≠0, \
max monster_type={global_max_monster_type}, max hit_dice={global_max_hit_dice} \
(records decode with variety, so field_E9=0 is a real data fact, not a decode bug)"
        );
        eprintln!("monster census: {total_monsters} monsters across 6 areas");

        let digest = Sha256::digest(facts.as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        eprintln!("derived-facts SHA-256 = {hex}");

        assert_eq!(
            hex, DERIVED_FACTS_SHA256,
            "monster derived-facts digest changed — re-pin if the data version changed"
        );
    }
}
