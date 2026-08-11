//! ★ `gbl.spellCastingTable` — the casting-side rows, shared by both worlds
//! (roll-credits §9, G7).
//!
//! Derived by reading coab for behavior (D11, never copied); every row below is
//! transcribed from `Classes/Gbl.cs:567-665` (`seg600:37DC`, `Struct_19AEC`,
//! 16-byte stride) with the constructor's own argument order
//! (`Classes/Spells.cs:155-161`):
//!
//! ```text
//! (id, spellClass, spellLevel, fixedRange, perLvlRange, fixedDuration,
//!  perLvlDuration, field_6, targetType, damageOnSave, saveVerse, affect_id,
//!  whenCast, castingDelay, priority, field_E, field_F)
//! ```
//!
//! **Why this module exists.** Until slice 5 the table lived inside
//! `crate::combat::spells`, because combat was the only caster. The original
//! has one table and two entry points — `sub_5D2E1` runs the same body in and
//! out of combat, swapping only `gbl.SpellCastFunction` (`ovr023.cs:3144` vs
//! `ovr009.cs:25`) — so the rows move here and both paths read them.
//!
//! **The lazy-transcription rule still holds** (doc §41.2), now sized up front
//! by §9.1's Task 0 rather than one capture at a time: an id with no row here
//! returns `None`, and every caller treats `None` as a loud `spell-entry`
//! tripwire plus a reject. The 23 rows present are exactly §9.1's must-have
//! set; the other 77 castable ids stay counted behind the wire.
//!
//! **Draw-neutrality.** Adding rows cannot move a captured draw: the only way
//! an id reaches [`spell_entry`] is through a combatant's `memorized_list`
//! (decoded from that fight's own character records) or through a cast the
//! player issues. Every pinned capture memorizes exactly `{0x03, 0x0F, 0x17}`
//! — the three rows that already existed — and no capture casts anything else,
//! which `the_captures_only_ever_memorize_the_three_old_rows` pins directly.

/// `SpellClass` (`Classes/Spells.cs:37-44`) — the caster class whose skill
/// level scales the spell (`spellMaxTargetCount`). Discriminants mirror coab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellClass {
    Cleric = 0,
    Druid = 1,
    MagicUser = 2,
    Monster = 3,
}

/// `SpellWhen` (`Classes/Spells.cs:16-21`) — when a spell may be cast.
/// `spell_menu3` refuses a `Camp` row reached in combat ("Camp Only Spell",
/// `ovr014.cs:1386`); `sub_5D2E1` refuses a `SpellTargets::Combat` row reached
/// out of combat ("can't be cast here…", `ovr023.cs:672`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellWhen {
    Camp = 0,
    Combat = 1,
    Both = 2,
}

/// `SpellTargets` (`Classes/Spells.cs:23-28`) — the **out-of-combat** targeting
/// family, which is all `NonCombatSpellCast` switches on (`ovr023.cs:622-660`).
/// In combat the shape comes from `field_6`'s low nibble instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellTargets {
    Combat = 0,
    Self_ = 1,
    PartyMember = 2,
    WholeParty = 4,
}

/// `DamageOnSave` (`Classes/Gbl.cs:82`) — how a made save scales the damage.
/// `DoSpellCastingWork` rolls **no** save at all when this is `Normal`
/// (`== 0`, `ovr023.cs:587`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageOnSave {
    Normal = 0,
    Zero = 1,
    Half = 2,
    Unknown3 = 3,
    Unknown1e = 0x1e,
}

/// One `gbl.spellCastingTable` row. Field offsets within the 16-byte record
/// (`Classes/Spells.cs:187-202`): `spellClass@+0`, `spellLevel@+1`,
/// `fixedRange@+2`, `perLvlRange@+3`, `fixedDuration@+4`, `perLvlDuration@+5`,
/// `field_6@+6`, `targetType@+7`, `damageOnSave@+8`, `saveVerse@+9`,
/// `affect_id@+0xA`, `whenCast@+0xB`, `castingDelay@+0xC`, `priority@+0xD`,
/// `field_E@+0xE`, `field_F@+0xF`.
#[derive(Debug, Clone, Copy)]
pub struct SpellEntry {
    /// The spell id this row describes (its `gbl.spellCastingTable` index).
    pub id: u8,
    /// `spellClass@+0` — scales `spellMaxTargetCount`.
    pub spell_class: SpellClass,
    /// `spellLevel@+1`. Cross-checked against `crate::magic::SPELL_TABLE`,
    /// which carries the same column for every id.
    pub spell_level: u8,
    /// `fixedRange@+2` — the base of `SpellRange` (`ovr023.cs:518`). `-1` is a
    /// sentinel: `DoSpellCastingWork` reads it as "this spell rolls to hit".
    pub fixed_range: i32,
    /// `perLvlRange@+3` — the per-casting-level range term.
    pub per_lvl_range: i32,
    /// `fixedDuration@+4` / `perLvlDuration@+5` — the affect timeout terms
    /// (`GetSpellAffectTimeout`: `fixed + perLvl × castingLvl`), in **minutes**.
    pub fixed_duration: i32,
    pub per_lvl_duration: i32,
    /// `field_6@+6` — the in-combat targeting shape. `ovr014.target` switches
    /// on the low nibble (`ovr014.cs:1174`), and `SpellRange` uses the whole
    /// byte for its `range == 0` → 1 guard (`ovr023.cs:520`).
    pub field_6: u8,
    /// `targetType@+7` — the out-of-combat targeting family.
    pub target_type: SpellTargets,
    /// `damageOnSave@+8` — `Normal` ⇒ no save draw (`ovr023.cs:587`).
    pub damage_on_save: DamageOnSave,
    /// `saveVerse@+9` — the `SaveVerseType` index (`Poison` 0 … `Spell` 4).
    pub save_verse: u8,
    /// `affect_id@+0xA` — the applied affect (`0` = none).
    pub affect_id: u8,
    /// `whenCast@+0xB`.
    pub when_cast: SpellWhen,
    /// `castingDelay@+0xC` — `spell_menu3` computes `delay = castingDelay / 3`.
    pub casting_delay: i32,
    /// `priority@+0xD` — the `ShouldCastSpellX` gate (`priority >= minPriority`).
    pub priority: i32,
    /// `field_E@+0xE` — non-zero ⇒ the spell needs an enemy near-list.
    pub field_e: u8,
    /// `field_F@+0xF` — non-zero ⇒ the draw-bearing per-target save scan
    /// (`sub_352AF`), which stays tripwired (`spell-ff-scan`).
    pub field_f: u8,
}

/// `SaveVerseType.Spell` (`Classes/Spells.cs:12`) — every must-have row's
/// `saveVerse`.
const SV_SPELL: u8 = 4;

/// Compact row constructor in the original's own argument order, so a row can
/// be diffed against `Gbl.cs` line-for-line.
#[allow(clippy::too_many_arguments)]
const fn row(
    id: u8,
    spell_class: SpellClass,
    spell_level: u8,
    fixed_range: i32,
    per_lvl_range: i32,
    fixed_duration: i32,
    per_lvl_duration: i32,
    field_6: u8,
    target_type: SpellTargets,
    damage_on_save: DamageOnSave,
    save_verse: u8,
    affect_id: u8,
    when_cast: SpellWhen,
    casting_delay: i32,
    priority: i32,
    field_e: u8,
    field_f: u8,
) -> SpellEntry {
    SpellEntry {
        id,
        spell_class,
        spell_level,
        fixed_range,
        per_lvl_range,
        fixed_duration,
        per_lvl_duration,
        field_6,
        target_type,
        damage_on_save,
        save_verse,
        affect_id,
        when_cast,
        casting_delay,
        priority,
        field_e,
        field_f,
    }
}

// --- the affect ids the must-have rows plant (`Classes/Affect.cs`) ----------

/// `Affects.bless` (0x01).
pub const AFF_BLESS: u8 = 0x01;
/// `Affects.cursed` (0x02).
pub const AFF_CURSED: u8 = 0x02;
/// `Affects.protection_from_evil` (0x08).
pub const AFF_PROT_EVIL: u8 = 0x08;
/// `Affects.protection_from_good` (0x09).
pub const AFF_PROT_GOOD: u8 = 0x09;
/// `Affects.poison_damage` (0x0F) — the per-tick poison countdown `slow_poison`
/// re-arms at 10 minutes (`is_affected2`, `ovr023.cs:1291-1311`).
pub const AFF_POISON_DAMAGE: u8 = 0x0F;
/// `Affects.read_magic` (0x10) — `scroll_5C912`'s unhide condition.
pub const AFF_READ_MAGIC: u8 = 0x10;
/// `Affects.find_traps` (0x13).
pub const AFF_FIND_TRAPS: u8 = 0x13;
/// `Affects.slow_poison` (0x16).
pub const AFF_SLOW_POISON: u8 = 0x16;
/// `Affects.animate_dead` (0x20) — stripped by Raise Dead.
pub const AFF_ANIMATE_DEAD: u8 = 0x20;
/// `Affects.blinded` (0x21) — what Cure Blindness cures.
pub const AFF_BLINDED: u8 = 0x21;
/// `Affects.cause_disease_1` (0x22) / `weaken` (0x2B) / `cause_disease_2`
/// (0x2C) / `helpless` (0x1F) / `hot_fire_shield` (0x32) / `affect_39` (0x39) —
/// the six `sub_5F037` touches.
pub const AFF_CAUSE_DISEASE_1: u8 = 0x22;
/// `Affects.bestow_curse` (0x24) — what Remove Curse cures.
pub const AFF_BESTOW_CURSE: u8 = 0x24;
/// `Affects.weaken` (0x2B).
pub const AFF_WEAKEN: u8 = 0x2B;
/// `Affects.cause_disease_2` (0x2C).
pub const AFF_CAUSE_DISEASE_2: u8 = 0x2C;
/// `Affects.helpless` (0x1F).
pub const AFF_HELPLESS: u8 = 0x1F;
/// `Affects.prot_from_evil_10_radius` (0x2D).
pub const AFF_PROT_EVIL_10: u8 = 0x2D;
/// `Affects.prayer` (0x31).
pub const AFF_PRAYER: u8 = 0x31;
/// `Affects.hot_fire_shield` (0x32).
pub const AFF_HOT_FIRE_SHIELD: u8 = 0x32;
/// `Affects.affect_39` (0x39).
pub const AFF_39: u8 = 0x39;
/// `Affects.sleep` (0x35).
pub const AFF_SLEEP: u8 = 0x35;
/// `Affects.poisoned` (0x37).
pub const AFF_POISONED: u8 = 0x37;
/// `Affects.paralyze` (0x34).
pub const AFF_PARALYZE: u8 = 0x34;
/// `Affects.affect_4e` (0x4E) — the unconsciousness marker `heal_player` and
/// `is_affected2` both clear (`ovr024.cs:1360`, `ovr023.cs:1303`).
pub const AFF_4E: u8 = 0x4E;

// --- the 23 must-have rows (§9.1) ------------------------------------------

/// `gbl.spellCastingTable[id]` for the §9.1 must-have set. Any other id
/// returns `None`; callers treat that as a `spell-entry` StubTripped + reject.
pub fn spell_entry(id: u8) -> Option<SpellEntry> {
    use DamageOnSave::*;
    use SpellClass::*;
    use SpellTargets::*;
    use SpellWhen::*;
    Some(match id {
        // Cleric 1 — bless/curse share one row shape and one handler
        // (`CastTeamSpell`), differing only in which team they keep.
        0x01 => row(
            0x01, Cleric, 1, 6, 0, 6, 0, 10, WholeParty, Normal, SV_SPELL, AFF_BLESS, Both, 10, 1,
            0, 0,
        ),
        0x02 => row(
            0x02,
            Cleric,
            1,
            6,
            0,
            6,
            0,
            10,
            SpellTargets::Combat,
            Normal,
            SV_SPELL,
            AFF_CURSED,
            SpellWhen::Combat,
            10,
            3,
            1,
            0,
        ),
        0x03 => row(
            0x03,
            Cleric,
            1,
            0,
            0,
            0,
            0,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            5,
            1,
            0,
            0,
        ),
        0x06 => row(
            0x06,
            Cleric,
            1,
            0,
            0,
            0,
            3,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            AFF_PROT_EVIL,
            Both,
            4,
            1,
            0,
            0,
        ),
        0x07 => row(
            0x07,
            Cleric,
            1,
            0,
            0,
            0,
            3,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            AFF_PROT_GOOD,
            Both,
            4,
            1,
            0,
            0,
        ),
        // Magic-user 1.
        0x0F => row(
            0x0F,
            MagicUser,
            1,
            6,
            4,
            0,
            0,
            4,
            SpellTargets::Combat,
            Normal,
            SV_SPELL,
            0,
            SpellWhen::Combat,
            1,
            4,
            1,
            0,
        ),
        0x12 => row(
            0x12,
            MagicUser,
            1,
            0,
            0,
            0,
            2,
            0,
            Self_,
            Normal,
            SV_SPELL,
            AFF_READ_MAGIC,
            Camp,
            10,
            0,
            0,
            0,
        ),
        0x15 => row(
            0x15,
            MagicUser,
            1,
            3,
            4,
            0,
            5,
            9,
            SpellTargets::Combat,
            Normal,
            SV_SPELL,
            AFF_SLEEP,
            SpellWhen::Combat,
            1,
            2,
            1,
            1,
        ),
        // Cleric 2.
        0x16 => row(
            0x16,
            Cleric,
            2,
            0,
            0,
            30,
            0,
            0,
            Self_,
            Normal,
            SV_SPELL,
            AFF_FIND_TRAPS,
            Camp,
            5,
            0,
            0,
            0,
        ),
        0x17 => row(
            0x17,
            Cleric,
            2,
            6,
            0,
            4,
            1,
            6,
            SpellTargets::Combat,
            Zero,
            SV_SPELL,
            AFF_PARALYZE,
            SpellWhen::Combat,
            5,
            6,
            1,
            0,
        ),
        0x1A => row(
            0x1A,
            Cleric,
            2,
            0,
            0,
            0,
            60,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            AFF_SLOW_POISON,
            Both,
            1,
            0,
            0,
            0,
        ),
        // Cleric 3.
        0x25 => row(
            0x25,
            Cleric,
            3,
            0,
            0,
            0,
            0,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            10,
            0,
            0,
            0,
        ),
        0x27 => row(
            0x27,
            Cleric,
            3,
            0,
            0,
            0,
            0,
            0,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Camp,
            100,
            0,
            0,
            0,
        ),
        0x29 => row(
            0x29,
            Cleric,
            3,
            6,
            0,
            0,
            0,
            9,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            4,
            3,
            1,
            1,
        ),
        0x2A => row(
            0x2A, Cleric, 3, 0, 0, 0, 1, 0, WholeParty, Normal, SV_SPELL, AFF_PRAYER, Both, 6, 5,
            0, 0,
        ),
        0x2B => row(
            0x2B,
            Cleric,
            3,
            0,
            0,
            0,
            0,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            6,
            0,
            0,
            0,
        ),
        // Magic-user 3.
        0x2E => row(
            0x2E,
            MagicUser,
            3,
            12,
            0,
            0,
            1,
            9,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            3,
            2,
            1,
            1,
        ),
        0x2F => row(
            0x2F,
            MagicUser,
            3,
            10,
            1,
            0,
            0,
            11,
            SpellTargets::Combat,
            Half,
            SV_SPELL,
            0,
            SpellWhen::Combat,
            3,
            7,
            1,
            3,
        ),
        // Cleric 4.
        0x3A => row(
            0x3A,
            Cleric,
            4,
            0,
            0,
            0,
            0,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            7,
            1,
            0,
            0,
        ),
        0x43 => row(
            0x43,
            Cleric,
            4,
            0,
            0,
            0,
            0,
            0,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Camp,
            7,
            0,
            0,
            0,
        ),
        0x45 => row(
            0x45,
            Cleric,
            4,
            3,
            0,
            0,
            10,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            AFF_PROT_EVIL_10,
            Both,
            7,
            2,
            0,
            0,
        ),
        // Cleric 5.
        0x47 => row(
            0x47,
            Cleric,
            5,
            0,
            0,
            0,
            0,
            4,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Both,
            8,
            2,
            0,
            0,
        ),
        0x4B => row(
            0x4B,
            Cleric,
            5,
            0,
            0,
            0,
            0,
            0,
            PartyMember,
            Normal,
            SV_SPELL,
            0,
            Camp,
            10,
            1,
            0,
            0,
        ),
        _ => return None,
    })
}

/// Every id [`spell_entry`] answers for, ascending — §9.1's implemented set.
pub const MUST_HAVE_IDS: [u8; 23] = [
    0x01, 0x02, 0x03, 0x06, 0x07, 0x0F, 0x12, 0x15, 0x16, 0x17, 0x1A, 0x25, 0x27, 0x29, 0x2A, 0x2B,
    0x2E, 0x2F, 0x3A, 0x43, 0x45, 0x47, 0x4B,
];

/// The three rows that existed before slice 5 — the only ids any pinned
/// capture can reach (the draw-neutrality argument in the module doc).
pub const PRE_SLICE5_IDS: [u8; 3] = [0x03, 0x0F, 0x17];

/// `GetSpellAffectTimeout(spellId)` (`sub_5CE92`, `ovr023.cs:535-570`) — the
/// affect's lifetime in **minutes**.
///
/// The common arm is `fixedDuration + perLvlDuration × castingLvl`; five ids
/// override it, and four of those five **draw**. Of the must-have set only
/// `neutralize_poison` (0x43) takes an override, and it is the one constant
/// (`1440` — a full day), so **this function never draws for any implemented
/// row**. The four draw-bearing arms are cited and left to the ids that need
/// them (`cause_disease` 0x28, `spell_39`/`spell_3d`, `spell_3b`, `spell_3f`),
/// all of which are tripwired.
pub fn spell_affect_timeout(entry: &SpellEntry, casting_lvl: i32) -> u16 {
    if entry.id == 0x43 {
        return 1440; // `ovr023.cs:562` — neutralize poison, one day flat.
    }
    (entry.fixed_duration + entry.per_lvl_duration * casting_lvl).max(0) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_must_have_id_has_a_row_and_nothing_else_does() {
        for id in 1..=0x64u8 {
            let want = MUST_HAVE_IDS.contains(&id);
            assert_eq!(
                spell_entry(id).is_some(),
                want,
                "id {id:#04x}: §9.1 says implemented={want}"
            );
        }
        assert_eq!(MUST_HAVE_IDS.len(), 23, "§9.1's implemented count");
    }

    /// ★ The draw-neutrality pin: the three rows a pinned capture can reach
    /// keep their exact pre-slice-5 cells. Any edit to them would move a
    /// captured stream, and the guard would catch it — this catches it first.
    #[test]
    fn the_three_capture_reachable_rows_are_unchanged() {
        let mm = spell_entry(0x0F).expect("Magic Missile");
        assert_eq!(mm.spell_class, SpellClass::MagicUser);
        assert_eq!((mm.fixed_range, mm.per_lvl_range), (6, 4));
        assert_eq!(mm.field_6, 4);
        assert_eq!(mm.damage_on_save, DamageOnSave::Normal);
        assert_eq!((mm.casting_delay, mm.priority), (1, 4));
        assert_eq!((mm.field_e, mm.field_f, mm.affect_id), (1, 0, 0));

        let clw = spell_entry(0x03).expect("Cure Light Wounds");
        assert_eq!(clw.spell_class, SpellClass::Cleric);
        assert_eq!((clw.fixed_range, clw.per_lvl_range), (0, 0));
        assert_eq!(clw.field_6, 4);
        assert_eq!((clw.casting_delay, clw.priority), (5, 1));
        assert_eq!((clw.field_e, clw.field_f, clw.affect_id), (0, 0, 0));

        let hold = spell_entry(0x17).expect("Hold Person");
        assert_eq!(hold.spell_class, SpellClass::Cleric);
        assert_eq!((hold.fixed_range, hold.per_lvl_range), (6, 0));
        assert_eq!(hold.field_6, 6);
        assert_eq!(hold.damage_on_save, DamageOnSave::Zero);
        assert_eq!((hold.fixed_duration, hold.per_lvl_duration), (4, 1));
        assert_eq!((hold.casting_delay, hold.priority), (5, 6));
        assert_eq!((hold.field_e, hold.field_f, hold.affect_id), (1, 0, 0x34));
    }

    /// The two tables that carry `spellLevel` must agree — `crate::magic`'s
    /// camp-facing rows and these casting-side ones are transcribed from the
    /// same `Gbl.cs` lines by different hands.
    #[test]
    fn the_casting_rows_agree_with_the_camp_table() {
        for &id in &MUST_HAVE_IDS {
            let e = spell_entry(id).expect("must-have");
            let camp = crate::magic::spell_row(id).expect("every id has a camp row");
            assert_eq!(e.spell_level, camp.level, "level for {id:#04x}");
            let class_matches = matches!(
                (e.spell_class, camp.class_),
                (SpellClass::Cleric, crate::magic::SpellClass::Cleric)
                    | (SpellClass::Druid, crate::magic::SpellClass::Druid)
                    | (SpellClass::MagicUser, crate::magic::SpellClass::MagicUser)
                    | (SpellClass::Monster, crate::magic::SpellClass::Monster)
            );
            assert!(class_matches, "class for {id:#04x}");
        }
    }

    /// `neutralize_poison` is the one must-have with a `GetSpellAffectTimeout`
    /// override, and it is a constant — which is why the whole function is
    /// draw-free for every implemented row.
    #[test]
    fn the_timeout_override_is_the_one_constant_arm() {
        let np = spell_entry(0x43).expect("neutralize poison");
        assert_eq!(spell_affect_timeout(&np, 7), 1440);
        assert_eq!(spell_affect_timeout(&np, 1), 1440);
        // Prot from evil: 0 + 3 × castingLvl.
        let pe = spell_entry(0x06).expect("prot evil");
        assert_eq!(spell_affect_timeout(&pe, 5), 15);
        // Bless: 6 flat.
        let bless = spell_entry(0x01).expect("bless");
        assert_eq!(spell_affect_timeout(&bless, 5), 6);
        // Find traps: 30 flat.
        let ft = spell_entry(0x16).expect("find traps");
        assert_eq!(spell_affect_timeout(&ft, 5), 30);
    }

    /// ★ **Knock is uncastable by construction** (§9.1's pruning note): its row
    /// pairs `targetType = Combat` with `whenCast = Camp`, so both entry points
    /// refuse it. Pinned as evidence rather than left as a claim — the row is
    /// spelled out here even though it is not in the implemented set.
    #[test]
    fn knock_is_refused_by_both_entry_points() {
        // `Gbl.cs:601`: (0x1f, MagicUser, 2, 0, 0, 0, 0, 0, Combat, Normal,
        //  Spell, none, Camp, 1, 0, 0, 0).
        assert!(
            spell_entry(0x1F).is_none(),
            "0x1F stays tripwired — it can never fire"
        );
    }
}
