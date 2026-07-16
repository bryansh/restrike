//! Loaded-monster model (M4 step 5, `docs/design/combat-study.md` §8) — the
//! engine-side, **data-only** view a future combat state will consume.
//!
//! `gbx-formats::monster` decodes the on-disk `MON<area>CHA.DAX` records (a
//! monster is a full 0x1A6 `Player` record — coab `ovr017.cs:824` `load_mob`
//! → `new Player(data, 0)`). This module lifts the combat-relevant subset into
//! an owned engine struct so the combat systems don't reach back into the
//! format layer per field. **No behavior here** — combat resolution
//! (initiative, to-hit, damage, AI) lands only after Phase-0 captures exist
//! (D-OR5(a) bootstrap order). This is the roster payload, nothing more.

use gbx_formats::monster::{AttackProfile, MonsterRecord};

/// One of a monster's two attack profiles, lifted from the format layer.
/// Damage is `roll_dice(dice_size, dice_count) + damage_bonus` (coab
/// `sub_3E192`, `ovr014.cs:86-87`) — the roll itself is a combat-session
/// concern; this only carries the parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterAttack {
    pub attacks: u8,
    pub dice_count: u8,
    pub dice_size: u8,
    pub damage_bonus: i8,
}

impl From<AttackProfile> for MonsterAttack {
    fn from(p: AttackProfile) -> Self {
        MonsterAttack {
            attacks: p.attacks,
            dice_count: p.dice_count,
            dice_size: p.dice_size,
            damage_bonus: p.damage_bonus,
        }
    }
}

/// A monster ready to be placed on the combat map — the decoded combat stats,
/// owned by the engine. Populated from a [`MonsterRecord`]; the full underlying
/// `CharRecord` stays in the format layer (combat needs only this view).
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedMonster {
    /// Display name (`@0x00`).
    pub name: String,
    /// Hit dice (`@0xe5`).
    pub hit_dice: u8,
    /// Maximum hit points (`@0x78`; stored, not rolled at load).
    pub hit_point_max: u8,
    /// Raw armor class (`@0x19a`); displayed AC = `0x3C - ac`.
    pub ac: i8,
    /// Base THAC0 (`@0x73`).
    pub thac0: i8,
    /// Turn-undead type (`field_E9` `@0xe9`). Shipped CotAB monster data holds
    /// 0 for every record (FD-20, resolved) — it is a runtime combat flag.
    pub turn_undead_type: u8,
    /// Monster family (`@0x11a`, `MonsterType`).
    pub monster_type: u8,
    /// Control/morale byte (`@0xf7`); `>= 0x80` ⇒ AI-controlled.
    pub control_morale: u8,
    /// Movement/initiative base (`@0x1a5`).
    pub movement: u8,
    /// The two attack profiles (attack1, attack2).
    pub attacks: [MonsterAttack; 2],
}

impl LoadedMonster {
    /// Lifts the combat-relevant view out of a decoded format-layer record.
    pub fn from_record(record: &MonsterRecord) -> Self {
        let [a1, a2] = record.attacks();
        LoadedMonster {
            name: record.name().to_string(),
            hit_dice: record.hit_dice(),
            hit_point_max: record.hit_point_max(),
            ac: record.ac(),
            thac0: record.thac0(),
            turn_undead_type: record.turn_undead_type(),
            monster_type: record.monster_type(),
            control_morale: record.control_morale(),
            movement: record.movement(),
            attacks: [a1.into(), a2.into()],
        }
    }

    /// Displayed armor class = `0x3C - ac` (`Classes/Player.cs:598`).
    pub fn display_ac(&self) -> i16 {
        0x3C - self.ac as i16
    }

    /// Whether this monster is AI-controlled (`control_morale >= 0x80`).
    pub fn is_npc(&self) -> bool {
        self.control_morale >= 0x80
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::monster::parse_cha_archive;

    /// Minimal synthetic CHA archive (self-authored bytes, D10-clean) so the
    /// lift-from-record path is exercised in CI without game data.
    fn synthetic_cha() -> Vec<u8> {
        const CHAR_RECORD_SIZE: usize = 0x1A6;
        const HEADER_ENTRY_SIZE: usize = 9;
        let mut rec = vec![0u8; CHAR_RECORD_SIZE];
        let name = b"GOBLIN";
        rec[0] = name.len() as u8;
        rec[1..1 + name.len()].copy_from_slice(name);
        rec[0x73] = 0x0C; // thac0
        rec[0xe5] = 1; // hit dice
        rec[0x78] = 7; // hp max
        rec[0xe9] = 0; // field_E9
        rec[0xf7] = 0x80; // NPC
        rec[0x11a] = 3; // monster type
        rec[0x19a] = 7; // ac
        rec[0x1a5] = 6; // movement
        rec[0x19c] = 1; // a1 attacks
        rec[0x19e] = 1; // a1 count
        rec[0x1a0] = 6; // a1 size
        rec[0x1a2] = 0; // a1 bonus

        // Wrap the record in a one-block DAX container (single literal run).
        let mut comp = Vec::new();
        for chunk in rec.chunks(128) {
            comp.push((chunk.len() - 1) as u8);
            comp.extend_from_slice(chunk);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&(HEADER_ENTRY_SIZE as u16).to_le_bytes());
        out.push(0); // id
        out.extend_from_slice(&0u32.to_le_bytes()); // offset
        out.extend_from_slice(&(rec.len() as u16).to_le_bytes()); // raw size
        out.extend_from_slice(&(comp.len() as u16).to_le_bytes()); // comp size
        out.extend_from_slice(&comp);
        out
    }

    #[test]
    fn lifts_combat_view_from_a_decoded_record() {
        let cha = synthetic_cha();
        let entries = parse_cha_archive(&cha).unwrap();
        let m = LoadedMonster::from_record(&entries[0].monster);
        assert_eq!(m.name, "GOBLIN");
        assert_eq!(m.hit_dice, 1);
        assert_eq!(m.ac, 7);
        assert_eq!(m.display_ac(), 0x3C - 7);
        assert_eq!(m.thac0, 0x0C);
        assert_eq!(m.turn_undead_type, 0);
        assert_eq!(m.monster_type, 3);
        assert!(m.is_npc());
        assert_eq!(
            m.attacks[0],
            MonsterAttack {
                attacks: 1,
                dice_count: 1,
                dice_size: 6,
                damage_bonus: 0
            }
        );
        assert_eq!(
            m.attacks[1],
            MonsterAttack {
                attacks: 0,
                dice_count: 0,
                dice_size: 0,
                damage_bonus: 0
            }
        );
    }
}
