use super::*;

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
        0x0F => Some(MAGIC_MISSILE),
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
        let spells_count = self.fighters[actor].memorized_list.len();
        // `var_5B = roll_dice(7,1)` (@066D) — UNCONDITIONAL, before the gate.
        // This is the d7 step 6 already drew (`ovr010.cs:248`).
        let bound = roll_dice(rng, 7, 1) as i32;
        let mut priority: i32 = 7; // var_5A (@0663)
        let mut pass: i32 = 1; // var_5D
        let mut spell_id: u8 = 0; // var_62

        // Gate (@0679-06A7): slots exist, NPC-controlled or magic toggled on, and
        // a live opponent (`friends_count`/`foe_count` > 0, ovr010.cs:255).
        let magic_on = self.fighters[actor].npc || self.auto_pcs_cast_magic;
        let live_opponent = {
            let (party, monsters) = self.live_counts();
            match self.fighters[actor].team {
                Team::Party => monsters > 0,
                Team::Monster => party > 0,
            }
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
    /// for a QuickFight, already-chosen spell (doc §41.3): the `whenCast == Camp`
    /// abort (unreachable for the combat-only Magic Missile — cited), then `delay
    /// = castingDelay / 3`. Magic Missile: `1 / 3 == 0` ⇒ the immediate cast
    /// [`sub_5d2e1`](Self::sub_5d2e1) + `clear_actions` (`ovr014.cs:1406-1411`);
    /// a `delay > 0` spell queues (`ovr014.cs:1414-1427`) — not modeled
    /// (`spell-queued` StubTripped, the turn still ends). Returns `casting_spell`.
    fn spell_menu3(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) -> bool {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        // Camp-only spell reached in combat (@1385) — coab zeroes spell_id, so
        // casting_spell stays false. Unreachable for Magic Missile (Combat).
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
            // delay > 0: "Begins Casting" — the spell queues into actions.spell_id
            // with a delay clamp (@1414-1427). Not modeled; the turn still ends.
            let id = self.fighters[actor].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "spell-queued",
            });
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
    fn sub_5d2e1(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) {
        // Miscast gate (@0714): HasAffect(affect_4a) → d2, 1 = miscast. The read
        // is draw-free on an empty list, and no capture carries the affect, so
        // the miscast never fires; the d2 is drawn only when the affect is
        // present (wired through the substrate for a future capture).
        if self.fighters[actor].has_affect(AFF_4A) && roll_dice(rng, 2, 1) == 1 {
            return; // "miscasts" — showCastingText/stillCast false, no cast.
        }

        // SpellCastFunction = target(quick_fight, spell_id) (@0733) — the d10.
        let Some(target) = self.spell_target(rng, actor, spell_id) else {
            // QuickFight abort (@0792): ClearSpell, turn ends (no cast). Not
            // reached by a pinned Magic Missile cast.
            self.clear_spell(actor, spell_id);
            return;
        };

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

        // gbl.spellTable[0x0F] = SpellMagicMissile (@0780-0781).
        self.spell_magic_missile(rng, actor, spell_id, target);
    }

    /// `ovr014.target(quick_fight, spell_id)` (`ovr014.cs:1164`) for the
    /// **1-target** shape (doc §41.3 step 2). Magic Missile's `field_6 & 0xF = 4`
    /// falls into the tail branch (`ovr014.cs:1322`), `max_targets = (field_6 &
    /// 3) + 1 = 1`, so it makes one [`sub_4001c`](Self::sub_4001c) pick (the
    /// `find_target` d10). Every other shape nibble is cited + tripped
    /// (`spell-target-shape`): `0` self, `5` budgeted-multi (a 2d4 draw), `8..=E`
    /// area, `0xF` held/area. Returns the single target, or `None` (no cast).
    fn spell_target(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) -> Option<usize> {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let nibble = entry.field_6 & 0x0F;
        // The tail/default branch (@1322): nibbles NOT {0, 5, 8..=0xF}. Magic
        // Missile is 4. max_targets = (field_6 & 3) + 1; for MM = 1.
        let one_target = !(nibble == 0 || nibble == 5 || (8..=0x0F).contains(&nibble));
        if !one_target {
            let id = self.fighters[actor].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "spell-target-shape",
            });
            return None;
        }
        // The max_targets loop (@1327-1358). For MM (max_targets 1) one pick.
        self.sub_4001c(rng, actor, spell_id)
    }

    /// `sub_4001C(arg_0, canTargetEmptyGround, quick_fight, spellId)`
    /// (`ovr014.cs:1095`) for the QuickFight + `field_E != 0` case (Magic
    /// Missile, doc §41.3 step 3): `find_target(clear=true, arg_2=0,
    /// max_range=SpellRange(id))` — **the d10** — then the held-target filter.
    /// If the picked target `IsHeld()` **and** the spell's `affect_id` is one of
    /// the held-affect ids [`HELD_AFFECT_IDS`] (`unk_18ADB[1..=4]`), the pick is
    /// rejected and the `var_9` loop runs once → no cast. Magic Missile's
    /// `affect_id` is 0, never in that table, and with empty affect lists nothing
    /// is held — so the first pick always stands. Returns the target, or `None`.
    fn sub_4001c(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8) -> Option<usize> {
        let range = self.spell_range(actor, spell_id);
        let affect_id = spell_entry(spell_id)
            .expect("caller guarantees a transcribed id")
            .affect_id;
        // var_9 = 1: a single find_target attempt (@1117-1148).
        // find_target(true, 0, SpellRange, caster) — the capture's d10.
        if self.find_target(rng, actor, true, 0, range) {
            let target = self.fighters[actor].target.expect("find_target set it");
            // The held-target filter (@1128-1137): IsHeld && affect_id ∈
            // unk_18ADB[1..=4] → reject (var_3 = false). MM affect_id 0 is never
            // in the table; IsHeld is false on empty affect lists.
            let held_rejected = self.is_held(target) && HELD_AFFECT_IDS.contains(&affect_id);
            if !held_rejected {
                return Some(target);
            }
        }
        None
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
            self.apply_damage(target, damage);
        }
    }

    /// `IsHeld()` (`Player.cs:847`): the target carries any `held_affects`
    /// {snake_charm 0x33, paralyze 0x34, sleep 0x35, helpless 0x1F}. Draw-free;
    /// false on the empty affect lists every capture carries (§39).
    fn is_held(&self, actor: usize) -> bool {
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
    /// @`ovr010:03B1-04A7`, coab `ovr010.cs:143`) — **draw-free for Magic
    /// Missile**. The verdict chain (doc §41.2):
    ///
    /// 1. an untranscribed id (lazy-transcription rule, incl. the id-3
    ///    `find_healing_target` special the real proc branches to at `@03D5`) →
    ///    `spell-entry` StubTripped + reject;
    /// 2. priority gate: `entry.priority >= minPriority` else reject;
    /// 3. `field_E == 0` → **accept** (self/buff spells need no target scan);
    /// 4. else `BuildNearTargets(SpellRange(id))` (`near_enermy`, our enemy-team
    ///    near-list flood); count == 0 → reject;
    /// 5. `field_F == 0` → **accept**; else the `sub_352AF` per-candidate
    ///    `RollSavingThrow` scan (`ovr010.cs:117`) — **DRAW-BEARING, not modeled**
    ///    → `spell-ff-scan` StubTripped + reject (no pinned capture reaches it;
    ///    Magic Missile has field_F 0).
    pub(super) fn should_cast_spell_x(
        &mut self,
        min_priority: i32,
        spell_id: u8,
        actor: usize,
    ) -> bool {
        let Some(entry) = spell_entry(spell_id) else {
            // Untranscribed id (or id 3's find_healing_target special) — cite +
            // reject (capture-safe: pinned captures memorize only Magic Missile).
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
        // field_E == 0 → self/buff, accept without a target scan (@03CE).
        if entry.field_e == 0 {
            return true;
        }
        // near_enermy(SpellRange(id)) — BuildNearTargets over the enemy team
        // (@03F6, ovr025.cs:1290 = Rebuild_SortedCombatantList w/ the
        // enemy-team filter = our build_near). Count 0 → reject.
        let range = self.spell_range(actor, spell_id);
        if self.build_near(actor, range, false).is_empty() {
            return false;
        }
        // field_F == 0 → accept (@0435). Magic Missile lands here.
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
            // Cleric/Druid: untranscribed (Magic Missile is MagicUser). Reached
            // only if a Cleric/Druid row is transcribed without its casting-level
            // decode — a loud 0 (never a silent wrong range) until then.
            SpellClass::Cleric | SpellClass::Druid => 0,
        }
    }
}
