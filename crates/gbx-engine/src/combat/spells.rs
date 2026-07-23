// ===========================================================================
// The spell subsystem (M5 caster peel, doc §41)
// ===========================================================================
//
// The `SpellEntry` row type + the one transcribed row (Magic Missile). The
// selection AI (`sub_3560B`/`ShouldCastSpellX`) and the cast (`spell_menu3` →
// `sub_5D2E1` → `SpellMagicMissile`) land in later commits of this slice; this
// commit is just the data + its lookup, so the row is verified against the
// `gbl.spellCastingTable` (`Classes/Gbl.cs:569+`, struct field↔offset map
// `Classes/Spells.cs:153-204`, `seg600:37DC` stride-16) before anything reads
// it.
//
// **Lazy-transcription rule (doc §41.2).** Only Magic Missile (id 0x0F) is
// transcribed. Any OTHER id reaching [`spell_entry`] returns `None`, and every
// caller treats `None` as a `spell-entry` StubTripped + reject — capture-safe,
// because the pinned captures memorize only Magic Missile. A future capture that
// memorizes another spell names the next row to transcribe through that wire.

/// `SpellClass` (`Classes/Spells.cs:39`) — the caster class whose skill level
/// scales the spell (`spellMaxTargetCount`, doc §41.2). Discriminants mirror
/// coab (`Cleric=0, Druid=1, MagicUser=2, Monster=3`).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpellClass {
    Cleric = 0,
    Druid = 1,
    MagicUser = 2,
    Monster = 3,
}

/// `SpellWhen` (`Classes/Spells.cs:16`) — when a spell may be cast. `spell_menu3`
/// aborts a `Camp`-only spell reached in combat (doc §41.3).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpellWhen {
    Camp = 0,
    Combat = 1,
    Both = 2,
}

/// `DamageOnSave` (`Classes/Gbl.cs:82`) — how a made save scales the damage.
/// `DoSpellCastingWork` (`ovr023.cs:587`) rolls **no** save when this is
/// `Normal` (`== 0`), the Magic Missile case (doc §41.3 step 8).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DamageOnSave {
    Normal = 0,
    Zero = 1,
    Half = 2,
    Unknown3 = 3,
    Unknown1e = 0x1e,
}

/// `SpellTargets` (`Classes/Spells.cs:23`) — the targeting family. Magic
/// Missile is `Combat`; `sub_5D2E1`'s "can't be cast here" gate compares this
/// against `game_state` (always Combat in a replay, so the gate never fires,
/// doc §41.3).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpellTargets {
    Combat = 0,
    Self_ = 1,
    PartyMember = 2,
    WholeParty = 4,
}

/// One row of `gbl.spellCastingTable` (`SpellEntry`, `Classes/Spells.cs:153`,
/// `Struct_19AEC` @`seg600:37DC`, 16-byte stride). Field offsets within the row
/// (verified against the `DataOffset`-style comments in `Spells.cs:187-202` and
/// the doc §41.2 map): `spellClass@+0`, `spellLevel@+1`, `fixedRange@+2`,
/// `perLvlRange@+3`, `fixedDuration@+4`, `perLvlDuration@+5`, `field_6@+6`,
/// `targetType@+7`, `damageOnSave@+8`, `saveVerse@+9`, `affect_id@+0xA`,
/// `whenCast@+0xB`, `castingDelay@+0xC`, `priority@+0xD`, `field_E@+0xE`,
/// `field_F@+0xF`. Only the cells the selection/cast path reads are carried.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(super) struct SpellEntry {
    /// The spell id this row describes (its `gbl.spellCastingTable` index).
    pub id: u8,
    /// `spellClass@+0` — scales `spellMaxTargetCount` (doc §41.2).
    pub spell_class: SpellClass,
    /// `fixedRange@+2` — the base of `SpellRange` (`ovr023.cs:518`).
    pub fixed_range: i32,
    /// `perLvlRange@+3` — the per-casting-level range term.
    pub per_lvl_range: i32,
    /// `field_6@+6` — the targeting-shape nibble (`field_6 & 0xF`, `ovr014.cs:1174`)
    /// and the `range == 0` → 1 guard (`ovr023.cs:520`).
    pub field_6: u8,
    /// `targetType@+7` — the `SpellTargets` family (cited; the combat-only gate).
    pub target_type: SpellTargets,
    /// `damageOnSave@+8` — `Normal` ⇒ no save draw (`ovr023.cs:587`).
    pub damage_on_save: DamageOnSave,
    /// `saveVerse@+9` — the `SaveVerseType` index (Magic Missile: `Spell` = 4).
    pub save_verse: u8,
    /// `affect_id@+0xA` — the applied affect (`0` = none for Magic Missile);
    /// gates `ApplyAttackSpellAffect` and the held-target filter (doc §41.3).
    pub affect_id: u8,
    /// `whenCast@+0xB` — `spell_menu3`'s Camp-only abort (doc §41.3).
    pub when_cast: SpellWhen,
    /// `castingDelay@+0xC` — `spell_menu3` computes `delay = castingDelay / 3`
    /// (Magic Missile: `1 / 3 == 0` ⇒ immediate, doc §41.3).
    pub casting_delay: i32,
    /// `priority@+0xD` — the `ShouldCastSpellX` gate (`priority >= minPriority`).
    pub priority: i32,
    /// `field_E@+0xE` — non-zero ⇒ the spell needs an enemy near-list
    /// (`ShouldCastSpellX`, doc §41.2 step 3).
    pub field_e: u8,
    /// `field_F@+0xF` — non-zero ⇒ the draw-bearing per-target save scan
    /// (`sub_352AF`); `0` for Magic Missile (doc §41.2 step 5).
    pub field_f: u8,
}

/// **Magic Missile** (id `0x0F`) — `gbl.spellCastingTable[0xf]` (`Gbl.cs:583`):
/// `new SpellEntry(0xf, MagicUser, 1, 6, 4, 0, 0, 4, Combat, Normal, Spell,
/// none, Combat, 1, 4, 1, 0)`. Priority 4, field_E 1, field_F 0, fixedRange 6,
/// perLvlRange 4, field_6 4, damageOnSave Normal(=0), saveVerse Spell, affect
/// none, whenCast Combat, castingDelay 1 — the doc §41.2 row.
const MAGIC_MISSILE: SpellEntry = SpellEntry {
    id: 0x0F,
    spell_class: SpellClass::MagicUser,
    fixed_range: 6,
    per_lvl_range: 4,
    field_6: 4,
    target_type: SpellTargets::Combat,
    damage_on_save: DamageOnSave::Normal,
    save_verse: 4, // SaveVerseType.Spell
    affect_id: 0,  // Affects.none
    when_cast: SpellWhen::Combat,
    casting_delay: 1,
    priority: 4,
    field_e: 1,
    field_f: 0,
};

/// `gbl.spellCastingTable[id]` for the transcribed rows — **Magic Missile only**
/// (doc §41.2's lazy-transcription rule). Any other id returns `None`; callers
/// treat that as a `spell-entry` StubTripped + reject (capture-safe: pinned
/// captures memorize only Magic Missile).
#[allow(dead_code)]
pub(super) fn spell_entry(id: u8) -> Option<SpellEntry> {
    match id {
        0x0F => Some(MAGIC_MISSILE),
        _ => None,
    }
}
