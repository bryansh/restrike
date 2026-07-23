use crate::combat::spells::{spell_entry, DamageOnSave, SpellClass, SpellTargets, SpellWhen};

// --- the SpellEntry row + the lazy-transcription rule (doc §41.2) -----------

/// Magic Missile (id 0x0F) decodes to the doc §41.2 row — every cell the
/// selection/cast path reads, pinned against an accidental edit to
/// `gbl.spellCastingTable[0xf]` (`Gbl.cs:583`).
#[test]
fn magic_missile_row_matches_the_binary_table() {
    let mm = spell_entry(0x0F).expect("Magic Missile is transcribed");
    assert_eq!(mm.id, 0x0F);
    assert_eq!(mm.spell_class, SpellClass::MagicUser);
    assert_eq!(mm.fixed_range, 6);
    assert_eq!(mm.per_lvl_range, 4);
    assert_eq!(mm.field_6, 4);
    assert_eq!(mm.target_type, SpellTargets::Combat);
    assert_eq!(mm.damage_on_save, DamageOnSave::Normal);
    assert_eq!(mm.save_verse, 4); // SaveVerseType.Spell
    assert_eq!(mm.affect_id, 0); // Affects.none
    assert_eq!(mm.when_cast, SpellWhen::Combat);
    assert_eq!(mm.casting_delay, 1);
    assert_eq!(mm.priority, 4);
    assert_eq!(mm.field_e, 1);
    assert_eq!(mm.field_f, 0);
}

/// `DamageOnSave.Normal == 0` — the value `DoSpellCastingWork` compares against
/// to decide "no save draw" (`ovr023.cs:587`). Pinning it guards the enum
/// discriminant the cast path depends on.
#[test]
fn damage_on_save_normal_is_zero() {
    assert_eq!(DamageOnSave::Normal as u8, 0);
    assert_eq!(DamageOnSave::Zero as u8, 1);
}

/// The lazy-transcription rule: only Magic Missile is transcribed. Every other
/// id — a neighbouring Magic-User row (Shield 0x13, Sleep 0x15), the Cleric
/// heal (id 3 `find_healing_target` special), and an out-of-range id — returns
/// `None`, so the selection AI trips `spell-entry` and rejects it (doc §41.2).
#[test]
fn only_magic_missile_is_transcribed() {
    assert!(spell_entry(0x0F).is_some());
    for id in [0u8, 1, 3, 0x0E, 0x10, 0x13, 0x15, 0x24, 0xFF] {
        assert!(
            spell_entry(id).is_none(),
            "id {id:#x} must be untranscribed (spell-entry trip)"
        );
    }
}
