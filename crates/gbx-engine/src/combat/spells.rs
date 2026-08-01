use super::*;

// ===========================================================================
// The spell subsystem (M5 caster peel doc §41; the cleric slice doc §48)
// ===========================================================================
//
// The `SpellEntry` row type + the transcribed rows (Magic Missile, Cure Light
// Wounds, Hold Person), the selection AI (`sub_3560B`/`ShouldCastSpellX`), and
// the cast (`spell_menu3` → `sub_5D2E1` → the per-spell functions) — including
// the QUEUED delayed cast ("Begins Casting", doc §48), the multi-target loop,
// per-target saving throws, and the healing-target scan. Rows are verified
// against `gbl.spellCastingTable` (`Classes/Gbl.cs:569+`, struct field↔offset
// map `Classes/Spells.cs:153-204`, `seg600:37DC` stride-16).
//
// **Lazy-transcription rule (doc §41.2).** Only capture-proven rows are
// transcribed (MM 0x0F; CLW 0x03 + hold person 0x17, doc §48). Any OTHER id
// reaching [`spell_entry`] returns `None`, and every caller treats `None` as a
// `spell-entry` StubTripped + reject — capture-safe. A future capture that
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
    /// `fixedDuration@+4` / `perLvlDuration@+5` — the affect timeout terms
    /// (`GetSpellAffectTimeout`: `fixed + perLvl × castingLvl`). Combat never
    /// ticks durations (`sub_5801E` decrements only while Camping, doc §40),
    /// so these only seed the stored affect's `minutes` on a landing.
    pub fixed_duration: i32,
    pub per_lvl_duration: i32,
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
    fixed_duration: 0,
    per_lvl_duration: 0,
    casting_delay: 1,
    priority: 4,
    field_e: 1,
    field_f: 0,
};

/// **Cure Light Wounds** (id 3) — `gbl.spellCastingTable[3]` (`Gbl.cs:572`):
/// `new SpellEntry(3, Cleric, 1, 0, 0, 0, 0, 4, PartyMember, Normal, Spell,
/// none, Both, 5, 1, 0, 0)`. Range `0 + 0×lvl` → the `field_6 != 0` clamp
/// makes it 1 (touch); castingDelay 5 → `5/3 = 1` — a QUEUED one-slot delayed
/// cast (doc §48); priority 1 (the LAST priority pass — cleric-fk round 2's
/// 19-draw selection: 6 full reject passes then the pass-7 accept);
/// field_E 0 with the id-3 `find_healing_target` special (doc §48).
const CURE_LIGHT_WOUNDS: SpellEntry = SpellEntry {
    id: 0x03,
    spell_class: SpellClass::Cleric,
    fixed_range: 0,
    per_lvl_range: 0,
    field_6: 4,
    target_type: SpellTargets::PartyMember,
    damage_on_save: DamageOnSave::Normal,
    save_verse: 4, // SaveVerseType.Spell
    affect_id: 0,  // Affects.none
    when_cast: SpellWhen::Both,
    fixed_duration: 0,
    per_lvl_duration: 0,
    casting_delay: 5,
    priority: 1,
    field_e: 0,
    field_f: 0,
};

/// **Hold Person** (cleric, id 0x17) — `gbl.spellCastingTable[0x17]`
/// (`Gbl.cs:592`): `new SpellEntry(0x17, Cleric, 2, 6, 0, 4, 1, 6, Combat,
/// Zero, Spell, paralyze, Combat, 5, 6, 1, 0)`. Range 6 (no per-level term);
/// `field_6 & 0xF = 6` → the tail targeting branch with `max_targets =
/// (6 & 3) + 1 = 3` (cleric-fk round 0: three find_target d4 picks);
/// `DamageOnSave::Zero` → a REAL save per target (`MultiTargetedSpell`, the
/// 3×d20); on a failed save `ApplyAttackSpellAffect` adds `paralyze` (0x34 —
/// a held-affect id); castingDelay 5 → queued one slot; priority 6.
const HOLD_PERSON: SpellEntry = SpellEntry {
    id: 0x17,
    spell_class: SpellClass::Cleric,
    fixed_range: 6,
    per_lvl_range: 0,
    field_6: 6,
    target_type: SpellTargets::Combat,
    damage_on_save: DamageOnSave::Zero,
    save_verse: 4,   // SaveVerseType.Spell
    affect_id: 0x34, // Affects.paralyze
    when_cast: SpellWhen::Combat,
    fixed_duration: 4,
    per_lvl_duration: 1,
    casting_delay: 5,
    priority: 6,
    field_e: 1,
    field_f: 0,
};

/// `Affects.affect_4a` (0x4A) — the miscast affect. `sub_5D2E1`'s miscast gate
/// (`ovr023.cs:714`) draws a d2 only when the caster `HasAffect(0x4A)`.
const AFF_4A: u8 = 0x4A;

/// `unk_18ADB[1..=4]` (`ovr014.cs:1093`, `seg600:27CB`; index 0 = `bless` filler)
/// == `held_affects` (`Player.cs:845`): snake_charm 0x33, paralyze 0x34, sleep
/// 0x35, helpless 0x1F. `sub_4001C`'s held-target filter rejects a pick whose
/// target `IsHeld()` when the spell's `affect_id` is one of these (doc §41.3).
const HELD_AFFECT_IDS: [u8; 4] = [0x33, 0x34, 0x35, 0x1F];

/// `gbl.spellCastingTable[id]` for the transcribed rows — **Magic Missile only**
/// (doc §41.2's lazy-transcription rule). Any other id returns `None`; callers
/// treat that as a `spell-entry` StubTripped + reject (capture-safe: pinned
/// captures memorize only Magic Missile).
pub(super) fn spell_entry(id: u8) -> Option<SpellEntry> {
    match id {
        0x03 => Some(CURE_LIGHT_WOUNDS),
        0x0F => Some(MAGIC_MISSILE),
        0x17 => Some(HOLD_PERSON),
        _ => None,
    }
}

impl CombatState {
    /// `sub_3560B(player)` (`ovr010:060B-0738`, coab `ovr010.cs:232`) — the
    /// memorized-spell selection loop, the replacement for step 6's old
    /// `memorized-spells` tripwire. The candidate list is already decoded
    /// ([`Combatant::memorized_list`], doc §41.1). Draws, in order:
    ///
    /// - the **unconditional** `var_5B = roll_dice(7,1)` bound (`@066D`) — drawn
    ///   before the gate, so a gate-off turn still spends this one d7 (the draw
    ///   step 6 already carried);
    /// - then, only when the gate passes (`@0679-06A7`): while `pass <= bound`
    ///   and nothing picked, up to **3×** `roll_dice(spells_count,1)` per priority
    ///   pass (`@06BB-0705`), each pick `list[roll−1]` fed to
    ///   [`should_cast_spell_x`](Self::should_cast_spell_x); an accept stops both
    ///   loops. `priority` counts down from 7 (`@0663`), `pass` up from 1.
    ///
    /// The gate (`@0679-06A7`): `spells_count > 0` **and** (`control_morale >=
    /// NPC_Base` **or** `AutoPCsCastMagic`) **and** a live opponent
    /// (`friends_count`/`foe_count`). Returns whether a spell was cast (the AI
    /// turn returns on `true`, `ovr010.cs:74-77`).
    pub(super) fn sub_3560b(&mut self, rng: &mut EngineRng, actor: usize) -> bool {
        // The collection itself is gated on `actions.can_cast` (coab
        // `ovr010.cs:238`; doc §45) — a caster disrupted by damage this round
        // collects NOTHING, so the gate below fails on `spells_count` and the
        // turn draws only the unconditional d7. Capture-proven by
        // sewer-fight-1: PHILIPPE arrow-hit in round 0, selection d1s only
        // from round 1 on.
        let spells_count = if self.fighters[actor].can_cast {
            self.fighters[actor].memorized_list.len()
        } else {
            0
        };
        // `var_5B = roll_dice(7,1)` (@066D) — UNCONDITIONAL, before the gate.
        // This is the d7 step 6 already drew (`ovr010.cs:248`).
        let bound = roll_dice(rng, 7, 1) as i32;
        let mut priority: i32 = 7; // var_5A (@0663)
        let mut pass: i32 = 1; // var_5D
        let mut spell_id: u8 = 0; // var_62

        // Gate (@0679-06A7): slots exist, NPC-controlled or magic toggled on, and
        // a live opponent — the ROUND-STALE `friends_count`/`foe_count`
        // globals (ovr010.cs:255), refreshed only at round boundaries (doc
        // §48): cleric-fk round 3 draws SHARA's selection d2s after both Fire
        // Knives escaped mid-round, round 4 draws none.
        let magic_on = self.fighters[actor].npc || self.auto_pcs_cast_magic;
        let live_opponent = match self.fighters[actor].team {
            Team::Party => self.foe_count > 0,
            Team::Monster => self.friends_count > 0,
        };
        if spells_count > 0 && magic_on && live_opponent {
            // The pass loop (@06A9-070D).
            while pass <= bound && spell_id == 0 {
                // Up to 3 inner picks (var_5E 1..4, @06BB).
                for _ in 0..3 {
                    if spell_id != 0 {
                        break;
                    }
                    // roll_dice(spells_count,1) − 1 indexes the candidate list
                    // (@06CE-06E0).
                    let idx = roll_dice(rng, spells_count as u16, 1) as usize - 1;
                    let id = self.fighters[actor].memorized_list[idx];
                    if self.should_cast_spell_x(priority, id, actor) {
                        spell_id = id; // var_62 = var_61 (@06FF)
                    }
                }
                priority -= 1; // @0707
                pass += 1; // @070A
            }
        }

        if spell_id > 0 {
            // On accept: spell_menu3 (@070F-0726). Returns casting_spell.
            return self.spell_menu3(rng, actor, spell_id);
        }
        false
    }

    /// `spell_menu3(out casting_spell, quick_fight, spell_id)` (`ovr014.cs:1373`)
    /// for a QuickFight, already-chosen spell (doc §41.3/§48): the `whenCast ==
    /// Camp` abort (unreachable for the transcribed rows — cited), then `delay
    /// = castingDelay / 3`. Magic Missile: `1 / 3 == 0` ⇒ the immediate cast
    /// [`sub_5d2e1`](Self::sub_5d2e1) + `clear_actions` (`ovr014.cs:1406-1411`).
    /// A `delay > 0` spell (CLW/hold: `5 / 3 == 1`) QUEUES — "Begins Casting"
    /// (`ovr014.cs:1414-1427`): `actions.spell_id := id` and the scheduler
    /// `delay` clamps to `min(current, cast_delay)` with a floor of 1, so the
    /// caster stays in the pick pool and the cast resolves at its NEXT pick
    /// (the capture's d4+d7 mini-turn, doc §48). Returns `casting_spell`.
    /// (`pub(super)` so the D-CV2 emission-order tests can drive the queue and
    /// immediate arms directly, without steering the selection loop by seed.)
    pub(super) fn spell_menu3(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) -> bool {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        // Camp-only spell reached in combat (@1385) — coab zeroes spell_id, so
        // casting_spell stays false. Unreachable for the transcribed rows.
        if entry.when_cast == SpellWhen::Camp {
            let id = self.fighters[actor].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "spell-entry",
            });
            return false;
        }
        // delay = castingDelay / 3 (@1404, sbyte). Magic Missile: 1/3 == 0.
        let delay = entry.casting_delay / 3;
        if delay == 0 {
            // Immediate cast (@1406-1411): sub_5D2E1 then clear_actions.
            self.sub_5d2e1(rng, actor, spell_id);
            self.clear_actions(actor);
            true
        } else {
            // "Begins Casting" (`ovr014:2866-28CC`): queue the cast
            // (`actions.spell_id := id` @2898) and adjust the scheduler
            // delay. ★ coab≠binary #22 (doc §48): with `actions.delay >
            // castDelay` the binary **SUBTRACTS** the cast delay
            // (`sub es:[di+3], al` @28BE) — coab ASSIGNS it. The subtract
            // keeps the caster near the top of the max-delay pick order, so
            // the cast resolves at the very next pick (cleric-fk: SHARA's
            // delay 8−1 = 7 ties the two remaining 7s and the pick-scan
            // d100s break the tie her way — the capture's immediate d4+d7
            // mini-turn; the coab assign parks her at 1, acting LAST). The
            // `delay <= castDelay` arm floors at 1 (@28CC). NO clear_actions
            // — the caster stays in the pool.
            let f = &mut self.fighters[actor];
            f.pending_spell = Some(spell_id);
            if f.delay as i32 > delay {
                f.delay -= delay as i8;
            } else {
                f.delay = 1;
            }
            // D-CV2 `BeginsCasting` — the queued-cast message. No spell id: the
            // original's "Begins Casting" does not name the spell (§1.5); the id
            // shows at resolution, with `Cast`, one or more picks later.
            self.emit(ActionEvent::BeginsCasting { caster_id: actor });
            true
        }
    }

    /// `sub_5D2E1(showCastingText, quick_fight, spell_id)` (`ovr023.cs:674-812`),
    /// the combat cast (doc §41.3). In draw order:
    /// 1. the miscast gate — `HasAffect(affect_4a 0x4A)` would draw a d2 (1 =
    ///    miscast); with empty affect lists no draw fires (§39 substrate);
    /// 2. `SpellCastFunction = ovr014.target` in combat (`ovr009.cs:25`) — the
    ///    targeting, [`spell_target`](Self::spell_target), which draws the
    ///    `find_target` **d10**;
    /// 3. on a target: the missile camera (`draw_missile_attack(0x1E, 4)` + the
    ///    `draw_74B3F` attack-icon pair, PlayerOnScreen-gated) — draw-free (§36
    ///    machinery, `MagicAttackDisplay` = §36.3 site 8);
    /// 4. `remove_invisibility(caster)` (§39 substrate, draw-free);
    /// 5. `spellList.ClearSpell(spell_id)` — slot consumption
    ///    ([`clear_spell`](Self::clear_spell)); every later PHILIPPE turn then
    ///    draws zero selection d1s (the capture's post-cast observable);
    /// 6. `SpellMagicMissile` (`gbl.spellTable[0x0F]`) — the damage d4s + apply.
    ///
    /// A QuickFight cast that finds no target aborts (`ovr023.cs:792` — "Spell
    /// Aborted", ClearSpell); the turn still ends. Magic Missile always finds a
    /// target in the pinned captures (its selection gate needed a near enemy).
    pub(super) fn sub_5d2e1(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) {
        // Miscast gate (@0714): HasAffect(affect_4a) → d2, 1 = miscast. The read
        // is draw-free on an empty list, and no capture carries the affect, so
        // the miscast never fires; the d2 is drawn only when the affect is
        // present (wired through the substrate for a future capture).
        if self.fighters[actor].has_affect(AFF_4A) && roll_dice(rng, 2, 1) == 1 {
            return; // "miscasts" — showCastingText/stillCast false, no cast.
        }

        // SpellCastFunction = target(quick_fight, spell_id) (@0733) — fills
        // gbl.spellTargets (the multi-target loop draws its find_target picks
        // here, doc §48).
        let targets = self.spell_target(rng, actor, spell_id);
        // D-CV2 `Cast` + one `SpellTarget` per pick, emitted once the targeting
        // pass has run (its `find_target` d10s are already drawn) and before the
        // missile camera below — message, then the targets it highlights, then
        // the projectile. The abort path emits `Cast` with an empty run: the
        // original shows the casting text and then "Spell Aborted"
        // (`ovr023.cs:792`). One event per pick keeps `ActionEvent: Copy`.
        self.emit(ActionEvent::Cast {
            caster_id: actor,
            spell_id,
        });
        for &t in &targets {
            self.emit(ActionEvent::SpellTarget { target_id: t });
        }
        if targets.is_empty() {
            // QuickFight abort (@0792): "Spell Aborted" — ClearSpell (the slot
            // is STILL consumed), turn ends, no cast.
            self.clear_spell(actor, spell_id);
            return;
        }
        // gbl.targetPos after ovr014.target = the LAST added target's pos.
        let target = *targets.last().expect("nonempty");

        // The missile camera (@0741-0768, doc §41.3 step 4). Draw-free — only the
        // persistent mapScreenTopLeft/direction effects are ported.
        let caster_pos = self.fighters[actor].pos;
        let target_pos = self.fighters[target].pos;
        let direction = find_combatant_direction(target_pos, caster_pos);
        self.focus = true; // focusCombatAreaOnPlayer = true (@0746)
        self.draw_74b3f(actor, direction); // draw_74B3F(false, Attack, dir, caster)
        self.draw_missile_camera(actor, target); // draw_missile_attack(0x1E, 4, ...)
        if self.on_screen(actor) {
            // The on-screen attack-icon pair (@0764-0768): direction re-stores
            // (no-ops, same value) + recenter checks (caster on-screen → no-op).
            let d = self.fighters[actor].direction;
            self.draw_74b3f(actor, d);
            self.draw_74b3f(actor, d);
        }

        // remove_invisibility(caster) (@0771) — §39 substrate, draw-free.
        self.remove_invisibility(actor);

        // ClearSpell(spell_id) (@0775) — consume the memorized slot.
        self.clear_spell(actor, spell_id);

        // gbl.spellTable[spell_id] (@0780-0781) — the per-spell function.
        match spell_id {
            0x03 => self.spell_cure_light(rng, actor, &targets),
            0x0F => self.spell_magic_missile(rng, actor, spell_id, target),
            0x17 => self.spell_hold_x(rng, actor, spell_id, &targets),
            _ => unreachable!("spell_entry gated"),
        }
    }

    /// `ovr014.target(quick_fight, spell_id)` (`ovr014.cs:1164`) for the
    /// TAIL targeting branch (doc §41.3 step 2/§48): a nibble NOT in
    /// {0, 5, 8..=0xF} runs the `max_targets = (field_6 & 3) + 1` loop
    /// (`ovr014.cs:1322-1358`) — Magic Missile (4) makes 1
    /// [`sub_4001c`](Self::sub_4001c) pick, hold person (6) makes 3. Each
    /// successful pick draws its own `find_target` d(count); a DUPLICATE pick
    /// still decrements `max_targets` in QuickFight (`:1345-1352`) but adds no
    /// entry; a failed pick zeroes the loop. Every other shape nibble is
    /// cited + tripped (`spell-target-shape`): `0` self, `5` budgeted-multi
    /// (a 2d4 draw), `8..=E` area, `0xF` held/area. Returns `gbl.spellTargets`
    /// (empty = no cast).
    fn spell_target(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) -> Vec<usize> {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let nibble = entry.field_6 & 0x0F;
        let tail_branch = !(nibble == 0 || nibble == 5 || (8..=0x0F).contains(&nibble));
        if !tail_branch {
            let id = self.fighters[actor].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "spell-target-shape",
            });
            return Vec::new();
        }
        // The max_targets loop (@1327-1358): MM 1 pick, hold person 3.
        let mut max_targets = (entry.field_6 & 3) as i32 + 1;
        let mut targets: Vec<usize> = Vec::new();
        while max_targets > 0 {
            match self.sub_4001c(rng, actor, spell_id) {
                Some(t) => {
                    if !targets.contains(&t) {
                        targets.push(t);
                        max_targets -= 1;
                    } else {
                        // "Already been targeted" — QuickFight decrements
                        // anyway (`:1345-1352`), no duplicate entry.
                        max_targets -= 1;
                    }
                }
                None => max_targets = 0,
            }
        }
        targets
    }

    /// `sub_4001C(arg_0, canTargetEmptyGround, quick_fight, spellId)`
    /// (`ovr014.cs:1095`) for QuickFight (doc §41.3 step 3/§48):
    ///
    /// - `field_E == 0` (Cure Light Wounds): the target is the CASTER, unless
    ///   `spellId == 3` where `find_healing_target` overrides it with the
    ///   most-wounded adjacent teammate (`ovr014.cs:1105-1112`) — **no
    ///   draw**; a failed healing scan aborts the cast;
    /// - `field_E != 0` (MM, hold): `find_target(clear=true, arg_2=0,
    ///   max_range=SpellRange(id))` — the d(count) pick — then the
    ///   held-target filter: a pick that `IsHeld()` when the spell's
    ///   `affect_id` is one of [`HELD_AFFECT_IDS`] (`unk_18ADB[1..=4]`) is
    ///   rejected and the `var_9` loop runs once → no target. Hold person's
    ///   paralyze (0x34) IS in the table, so a held pick is skipped — inert
    ///   while nothing is held (all three cleric-fk saves passed).
    fn sub_4001c(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) -> Option<usize> {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        if entry.field_e == 0 {
            // Self-target, with the id-3 healing override (`ovr014.cs:1105`).
            if spell_id == 3 {
                return self.find_healing_target(actor);
            }
            return Some(actor);
        }
        let range = self.spell_range(actor, spell_id);
        let affect_id = entry.affect_id;
        // var_9 = 1: a single find_target attempt (@1117-1148).
        // find_target(true, 0, SpellRange, caster) — the capture's d(count).
        if self.find_target(rng, actor, true, 0, range) {
            let target = self.fighters[actor].target.expect("find_target set it");
            // The held-target filter (@1128-1137): IsHeld && affect_id ∈
            // unk_18ADB[1..=4] → reject (var_3 = false).
            let held_rejected = self.is_held(target) && HELD_AFFECT_IDS.contains(&affect_id);
            if !held_rejected {
                return Some(target);
            }
        }
        None
    }

    /// `find_healing_target(out target, healer)` (`sub_3FDFE` @`ovr014:1DFE`,
    /// coab `ovr014.cs:1041`; instruction-verified doc §48): scan the 9
    /// `MapDirectionDelta` cells (8 neighbors, then SELF at index 8) for
    /// same-team combatants below max hp; keep the LOWEST current hp
    /// (strictly — first found wins ties), with the healer additionally
    /// qualifying below half max (`@1EF4-1F07`). The downed-ally override
    /// (`Tile_DownPlayer` + `lowest_hp >= 8`, `@1E7B`/tail) is cited — no
    /// downed teammate is adjacent in any pinned cast. Draw-free.
    pub(super) fn find_healing_target(&self, healer: usize) -> Option<usize> {
        let hpos = self.fighters[healer].pos;
        let mut lowest: Option<usize> = None;
        let mut lowest_hp = 0xFF_i32;
        for dir in 0..=8u8 {
            let (dx, dy) = MAP_DIRECTION_DELTA[dir as usize];
            let cell = GridPos::new(hpos.x + dx, hpos.y + dy);
            // AtMapXY (`ovr033.cs:191`): the occupancy map, 1-based (0 = none).
            let occ = self.map.occupant(cell);
            if occ == 0 {
                continue;
            }
            let t = (occ - 1) as usize;
            if self.fighters[t].team != self.fighters[healer].team {
                continue;
            }
            if self.fighters[t].hp_current >= self.fighters[t].hp_max {
                continue; // not wounded (`cmp hp_cur, hp_max; jnb skip`)
            }
            let hp = self.fighters[t].hp_current;
            if hp < lowest_hp || (t == healer && hp < self.fighters[t].hp_max / 2) {
                lowest = Some(t);
                lowest_hp = hp;
            }
        }
        lowest
    }

    /// `SpellMagicMissile` (`gbl.spellTable[0x0F]` = `sub_5E221`, `ovr023.cs:1166`,
    /// doc §41.3 steps 6-8): `n = spellMaxTargetCount + 1 = castingLvl + 1`;
    /// `damage = n/2 + roll_dice(4, n/2)` (`roll_dice_save ≡ roll_dice`,
    /// `ovr024.cs:601` — **(lvl+1)/2 separate d4 draws**; PHILIPPE lvl 5 → 3 d4s).
    /// Then `DoSpellCastingWork`: `damageOnSave == Normal(0)` ⇒ **no save draw**;
    /// `damage_person(false, Normal, damage, target)` routes through our
    /// [`apply_damage`](Self::apply_damage) ladder (draw-free); `affect_id == 0`
    /// ⇒ no `ApplyAttackSpellAffect`.
    fn spell_magic_missile(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        target: usize,
    ) {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let n = self.spell_max_target_count(actor, entry.spell_class) + 1; // var_1
        let half = n / 2;
        // damage = n/2 + roll_dice_save(4, n/2). roll_dice(4, half) draws `half`
        // separate d4s (byte-summed) — for PHILIPPE half = 3 → three d4s.
        let damage = half + roll_dice(rng, 4, half as u16) as i32;
        // DoSpellCastingWork (@sub_5CF7F): damageOnSave Normal → saved = false, NO
        // save draw; damage > 0 → damage_person → damage_player == apply_damage.
        // affect_id 0 → no ApplyAttackSpellAffect.
        if damage > 0 {
            self.apply_damage(rng, target, damage);
        }
    }

    /// `SpellCureLight` (`sub_5DDBC` @`ovr023:1DBC`, listing-verified doc §48):
    /// gated on a nonempty `spellTargets`, ONE `roll_dice(8,1)` then
    /// `heal_player(0, roll, spellTargets[0])` — the hp write caps at max
    /// (`heal_player`, `ovr024.cs:1336`; status gate okey/animated/unconscious/
    /// dying). cleric-fk round 2: SHARA 51 → 55 on a d8 roll of 4.
    fn spell_cure_light(&mut self, rng: &mut EngineRng, actor: usize, targets: &[usize]) {
        let target = targets[0];
        let heal = roll_dice(rng, 8, 1) as i32;
        // heal_player status gate: dead/gone targets don't heal. All modeled
        // statuses short of Dead qualify (okey/unconscious/dying; `animated`
        // is undecoded — no pinned roster carries it).
        if self.fighters[target].health_status == HealthStatus::Dead {
            return;
        }
        let f = &mut self.fighters[target];
        f.hp_current = (f.hp_current + heal).min(f.hp_max);
        // D-CV2 `Healed`, after the write the gate above let through. `amount` is
        // the **rolled** d8, which is what the original reports — not the
        // post-cap delta (SHARA's 51 → 55 on a 4 is a full-value heal; a heal
        // that caps still says its roll).
        self.emit(ActionEvent::Healed {
            healer_id: actor,
            target_id: target,
            amount: heal,
            kind: HealKind::Cure,
        });
    }

    /// `SpellHoldX` (`is_held` @`ovr023:2444`, listing-verified doc §48): the
    /// save bonus scales with the UNIQUE target count — 1 target → −2 (hold
    /// person 0x17; −3 for the MU twin), 2 → −1, 3/4 → 0 — then
    /// [`Self::multi_targeted_spell`].
    fn spell_hold_x(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8, targets: &[usize]) {
        let save_bonus = match targets.len() {
            1 => {
                if spell_id == 0x17 {
                    -2
                } else {
                    -3
                }
            }
            2 => -1,
            _ => 0, // 3 or 4 (the loop can't build more)
        };
        self.multi_targeted_spell(rng, actor, spell_id, targets, save_bonus);
    }

    /// `MultiTargetedSpell(text, save_bonus)` (`sub_5DB24` @`ovr023:1B24`,
    /// listing-verified doc §48) — per `spellTargets` entry, in order:
    ///
    /// 1. every entry after the first draws the missile camera
    ///    (`draw_missile_attack(0x1E, 4, target, caster)` @`1B8D-1BD8`) —
    ///    draw-free, camera state only;
    /// 2. `RollSavingThrow(save_bonus, saveVerse, target)` — **one d20**
    ///    (`@1C18`; the 0x4F/0x51 first-target skip is cited — not our ids);
    /// 3. the type/size override (`@1C32-1C46`): `monsterType@0x11A > 1 ||
    ///    field_DE > 1` → `saved := true` AFTER the roll (large/monster-class
    ///    targets can't be held; the roll is spent regardless). The listing
    ///    compares the skip-spell id against 0x5E where coab writes 0x53
    ///    (`@1C48` — neither id is transcribed; recorded, not modeled);
    /// 4. `ApplyAttackSpellAffect` (`is_unaffected`, `ovr024.cs:1303`):
    ///    `saved && DamageOnSave::Zero` → "is Unaffected" (no state change);
    ///    else remove-existing + `add_affect(affect_id)` — the HELD landing.
    ///    No pinned capture lands one (all three cleric-fk saves passed), so
    ///    the landing is tripwired `hold-landed` on top of the affect add.
    fn multi_targeted_spell(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
        save_bonus: i32,
    ) {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        for (i, &target) in targets.iter().enumerate() {
            if i > 0 {
                self.draw_missile_camera(actor, target);
            }
            let saved_roll = self.do_saving_throw(rng, save_bonus, entry.save_verse, target);
            let f = &self.fighters[target];
            let saved = saved_roll || f.monster_type > 1 || f.field_de > 1;
            // ApplyAttackSpellAffect: MagicResistance affect hook first
            // (draw-free on empty lists), then the saved/Zero gate.
            self.check_affects_effect(target, CheckType::MagicResistance);
            if entry.affect_id == 0 || (saved && entry.damage_on_save == DamageOnSave::Zero) {
                continue; // "is Unaffected" — display only.
            }
            // The landing (`is_unaffected` else-arm, ovr024.cs:1316-1329):
            // remove an existing `minutes > 0` instance, then
            // `add_affect(call_affect_table = false, data = castingLvl,
            // time = GetSpellAffectTimeout)`. CAPTURE-PROVEN (§49,
            // cleric-guildwar): the held-target behavior is now modeled —
            // the PlayerRestrained turn skip (`sub_3A071` = clear_actions)
            // and the melee SLAY (`sub_3F4EB` @`ovr014:152C-15E0`) — so the
            // `hold-landed` tripwire is RETIRED.
            if self.fighters[target]
                .affects
                .iter()
                .any(|a| a.kind == entry.affect_id && a.minutes > 0)
            {
                self.remove_affect(target, entry.affect_id);
            }
            let casting_lvl = self.spell_max_target_count(actor, entry.spell_class);
            let timeout =
                (entry.fixed_duration + entry.per_lvl_duration * casting_lvl).max(0) as u16;
            self.fighters[target].add_affect(entry.affect_id, timeout, casting_lvl as u8, false);
        }
    }

    /// `RollSavingThrow(saveBonus, saveType, player)` (`do_saving_throw`
    /// @`ovr024:12F1`, listing-verified doc §48): one d20; natural 1 fails,
    /// natural 20 saves; else `roll += saveBonus + field_186`, the
    /// `CheckAffectsEffect(SavingThrow)` hook (TRAVIS's dwarf con-save affect
    /// lives there — draw-free, unexercised), then `made = roll >=
    /// saveVerse[type]@0xDF`.
    fn do_saving_throw(
        &mut self,
        rng: &mut EngineRng,
        save_bonus: i32,
        save_verse: u8,
        target: usize,
    ) -> bool {
        let d20 = roll_dice(rng, 20, 1) as i32;
        if d20 == 1 {
            return false;
        }
        if d20 == 20 {
            return true;
        }
        // The `saving_throw` accumulator (doc §50): the binary seeds the
        // global with the roll (`mov saving_throw, al` @`ovr024:1306`), adds
        // `field_186 + saveBonus` (@`1322-133C`), runs the affect hook
        // (`work_on_00(target, 12)` @`134F`) — whose handlers adjust the LIVE
        // accumulator (prot-evil +2; dwarf 0x61 still trips) — then compares
        // `saves[verse] > saving_throw` (@`135B-1364`, made on equality).
        self.saving_throw = d20 + save_bonus + self.fighters[target].field_186;
        self.check_affects_effect(target, CheckType::SavingThrow);
        let f = &self.fighters[target];
        let target_num = f.saves[(save_verse as usize).min(4)] as i32;
        self.saving_throw >= target_num
    }

    /// `IsHeld()` (`Player.cs:847`): the target carries any `held_affects`
    /// {snake_charm 0x33, paralyze 0x34, sleep 0x35, helpless 0x1F}. Draw-free;
    /// false on the empty affect lists every capture carries (§39).
    pub(super) fn is_held(&self, actor: usize) -> bool {
        HELD_AFFECT_IDS
            .iter()
            .any(|&a| self.fighters[actor].has_affect(a))
    }

    /// `SpellList.ClearSpell(spellId)` (`Classes/SpellList.cs:30`): remove the
    /// **first** memorized entry whose id matches (one instance). The engine's
    /// `memorized_list` is the collected candidate list, so removing one `spell_id`
    /// from it drops the caster's `spells_count` — PHILIPPE's one Magic Missile →
    /// empty → his later turns draw zero selection d1s (doc §41.3 step 6).
    fn clear_spell(&mut self, actor: usize, spell_id: u8) {
        if let Some(pos) = self.fighters[actor]
            .memorized_list
            .iter()
            .position(|&s| s == spell_id)
        {
            self.fighters[actor].memorized_list.remove(pos);
        }
    }

    /// `ShouldCastSpellX(minPriority, spellId, attacker)` (`sub_353B1`
    /// @`ovr010:03B1-04A7`, coab `ovr010.cs:143`) — **draw-free for every
    /// transcribed row** (field_F 0). The verdict chain (doc §41.2/§48):
    ///
    /// 1. an untranscribed id (lazy-transcription rule) → `spell-entry`
    ///    StubTripped + reject;
    /// 2. priority gate: `entry.priority >= minPriority` else reject;
    /// 3. `(id != 3 && field_E == 0)` → **accept** (self/buff spells need no
    ///    target scan); `(id == 3 && find_healing_target(attacker))` — the
    ///    Cure Light Wounds special (`ovr010.cs:149-150`) — → **accept**;
    /// 4. else `BuildNearTargets(SpellRange(id))` (`near_enermy`, our enemy-team
    ///    near-list flood); count == 0 → reject — the gate that rejects hold
    ///    person once the Fire Knives are out of its 6-step range (doc §48);
    ///    an id-3 with no healing target also lands here (adjacent-enemy
    ///    fallback, faithful to the coab `||` structure);
    /// 5. `field_F == 0` → **accept**; else the `sub_352AF` per-candidate
    ///    `RollSavingThrow` scan (`ovr010.cs:117`) — **DRAW-BEARING, not modeled**
    ///    → `spell-ff-scan` StubTripped + reject (no pinned capture reaches it;
    ///    all transcribed rows carry field_F 0).
    pub(super) fn should_cast_spell_x(
        &mut self,
        min_priority: i32,
        spell_id: u8,
        actor: usize,
    ) -> bool {
        let Some(entry) = spell_entry(spell_id) else {
            // Untranscribed id — cite + reject (capture-safe: pinned captures
            // memorize only transcribed rows).
            let id = self.fighters[actor].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "spell-entry",
            });
            return false;
        };
        // Priority gate (@03B8): `priority >= minPriority`.
        if entry.priority < min_priority {
            return false;
        }
        // (id != 3 && field_E == 0) || (id == 3 && find_healing_target) → accept
        // (`ovr010.cs:148-152`; the id-3 special @`ovr010:03D5`).
        if (spell_id != 3 && entry.field_e == 0)
            || (spell_id == 3 && self.find_healing_target(actor).is_some())
        {
            return true;
        }
        // near_enermy(SpellRange(id)) — BuildNearTargets over the enemy team
        // (@03F6, ovr025.cs:1290 = Rebuild_SortedCombatantList w/ the
        // enemy-team filter = our build_near). Count 0 → reject.
        let range = self.spell_range(actor, spell_id);
        if self.build_near(actor, range, false).is_empty() {
            return false;
        }
        // field_F == 0 → accept (@0435). Every transcribed row lands here.
        if entry.field_f == 0 {
            return true;
        }
        // field_F != 0 → the sub_352AF per-target RollSavingThrow scan
        // (@0442-0489, DRAW-BEARING) — not modeled.
        let id = self.fighters[actor].id;
        self.emit(ActionEvent::StubTripped {
            combatant_id: id,
            stub: "spell-ff-scan",
        });
        false
    }

    /// `SpellRange(spellId)` (`sub_5CDE5` @`ovr023.cs:515`): `fixedRange +
    /// perLvlRange × castingLvl`, then the clamps — `range == 0 && field_6 != 0`
    /// → 1, and `range ∈ {−1, 0xFF}` → 1. `castingLvl = spellMaxTargetCount(id)`
    /// (`spell_from_item` is never set on a memorized cast, so the item-branch
    /// `6` is unreachable here). Magic Missile: `6 + 4×5 = 26` for PHILIPPE (doc
    /// §41.2). Draw-free.
    pub(super) fn spell_range(&self, actor: usize, spell_id: u8) -> i32 {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let casting_lvl = self.spell_max_target_count(actor, entry.spell_class);
        let mut range = entry.fixed_range + entry.per_lvl_range * casting_lvl;
        if range == 0 && entry.field_6 != 0 {
            range = 1;
        }
        if range == -1 || range == 0xff {
            range = 1;
        }
        range
    }

    /// `spellMaxTargetCount(spell_id)` (`sub_6886F` @`ovr025.cs:1342`) for the
    /// caster `actor` — the spell's per-level scaling (= `castingLvl`, doc §41.2).
    /// The no-caster fallback ([`Combatant::caster_no_class`], `@1351`) → 6; else
    /// by `spellClass`: MagicUser → `max(SkillLevel(MU), SkillLevel(Ranger) − 8)`
    /// (`@1376`); Monster → 12 (`@1382`, cited — no capture). The Cleric/Druid
    /// branches are untranscribed (Magic Missile is MagicUser); a spell needing
    /// them arrives with its own row. `spell_from_item → 6` is unmodeled (never
    /// set on a memorized cast). Draw-free.
    pub(super) fn spell_max_target_count(&self, actor: usize, spell_class: SpellClass) -> i32 {
        let f = &self.fighters[actor];
        if f.caster_no_class {
            return 6;
        }
        match spell_class {
            SpellClass::MagicUser => f.skill_level_magic_user.max(f.skill_level_ranger - 8),
            SpellClass::Monster => 12,
            // Cleric: `max(SkillLevel(Cleric), SkillLevel(Paladin) − 8)`
            // (`sub_6886F` @`ovr025.cs:1363`, doc §48 — SHARA cleric 5).
            SpellClass::Cleric => f.skill_level_cleric.max(f.skill_level_paladin - 8),
            // Druid: untranscribed (no Druid row). Reached only if one is
            // transcribed without its casting-level decode — a loud 0.
            SpellClass::Druid => 0,
        }
    }
}
