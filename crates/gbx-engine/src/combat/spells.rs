use super::*;

/// Which arm of `ovr014.target` a cast takes — the `quick_fight` argument
/// `sub_5D2E1` threads from `spell_menu3` down to `sub_4001C`
/// (`ovr023.cs:674-733`, `ovr014.cs:1095-1103`).
///
/// `QuickFight.True` picks its targets with `find_target`'s dice; `False` opens
/// the aim menu, which draws nothing. Same cast, different targeting — the one
/// place a player's spell and the AI's diverge.
#[derive(Debug, Clone, Copy)]
pub(super) enum Targeting<'a> {
    /// `QuickFight.True` — the AI's auto-pick (draw-bearing).
    Auto,
    /// `QuickFight.False` — the aim menu's picks, in pick order (draw-free).
    Manual(&'a [usize]),
}

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

// ★ **Roll-credits slice 5**: the rows themselves now live in
// [`crate::spells`], because the original casts from one table through two
// entry points (`sub_5D2E1` with `SpellCastFunction` swapped, `ovr023.cs:3144`
// vs `ovr009.cs:25`). This module keeps the *combat* half of the machinery and
// reads its rows from there; §9.1's Task 0 sized the set.
#[allow(unused_imports)]
pub(super) use crate::spells::{
    spell_affect_timeout, spell_entry, DamageOnSave, SpellClass, SpellEntry, SpellTargets,
    SpellWhen,
};

/// `Affects.affect_4a` (0x4A) — the miscast affect. `sub_5D2E1`'s miscast gate
/// (`ovr023.cs:714`) draws a d2 only when the caster `HasAffect(0x4A)`.
const AFF_4A: u8 = 0x4A;

/// What `ovr014.target` leaves behind: `gbl.spellTargets` **and**
/// `gbl.targetPos`. They are separate globals and they genuinely differ for an
/// area spell — the list is whoever the blast caught, the position is where it
/// was aimed — so the caller's missile camera must read the second, not the
/// last entry of the first.
struct SpellAim {
    targets: Vec<usize>,
    target_pos: GridPos,
}

/// `unk_18ADB[1..=4]` (`ovr014.cs:1093`, `seg600:27CB`; index 0 = `bless` filler)
/// == `held_affects` (`Player.cs:845`): snake_charm 0x33, paralyze 0x34, sleep
/// 0x35, helpless 0x1F. `sub_4001C`'s held-target filter rejects a pick whose
/// target `IsHeld()` when the spell's `affect_id` is one of these (doc §41.3).
const HELD_AFFECT_IDS: [u8; 4] = [0x33, 0x34, 0x35, 0x1F];

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
        self.spell_menu3_with(rng, actor, spell_id, Targeting::Auto)
    }

    /// ★ **M6c**: `spell_menu3` with `quick_fight == False` (`ovr009.cs:219`) —
    /// the player's Cast word. Same function, same delay arithmetic, same queue;
    /// only the targeting differs, and `targets` is what the aim menu picked.
    pub(super) fn spell_menu3_manual(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
    ) -> bool {
        self.spell_menu3_with(rng, actor, spell_id, Targeting::Manual(targets))
    }

    fn spell_menu3_with(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targeting: Targeting<'_>,
    ) -> bool {
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
            self.sub_5d2e1_with(rng, actor, spell_id, targeting);
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
        self.sub_5d2e1_with(rng, actor, spell_id, Targeting::Auto)
    }

    /// [`sub_5d2e1`](Self::sub_5d2e1) with the `quick_fight` argument the
    /// original threads all the way down to `sub_4001C` (`ovr023.cs:733` →
    /// `ovr014.target` → `ovr014.cs:1098`). Everything outside the targeting
    /// call is shared, which is the point: a player's Magic Missile and the
    /// AI's are the same cast.
    /// ★ **M6c**: the queued cast resolving on a manual turn
    /// (`combat_menu`'s head, `ovr009.cs:161` — `QuickFight.False`).
    pub(super) fn sub_5d2e1_manual(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
    ) {
        self.sub_5d2e1_with(rng, actor, spell_id, Targeting::Manual(targets))
    }

    fn sub_5d2e1_with(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targeting: Targeting<'_>,
    ) {
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
        let aim = self.spell_target_with(rng, actor, spell_id, targeting);
        let targets = aim.targets;
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

        // The missile camera (@0741-0768, doc §41.3 step 4). Draw-free — only the
        // persistent mapScreenTopLeft/direction effects are ported. ★ It flies
        // to `gbl.targetPos`, not to `spellTargets.Last()`: for an area spell
        // those differ (the aim point vs whoever the sorted list ended on), and
        // the original always uses the aim point.
        let caster_pos = self.fighters[actor].pos;
        let target_pos = aim.target_pos;
        let direction = find_combatant_direction(target_pos, caster_pos);
        self.focus = true; // focusCombatAreaOnPlayer = true (@0746)
        self.draw_74b3f(actor, direction); // draw_74B3F(false, Attack, dir, caster)
        self.draw_missile_camera_between(caster_pos, target_pos);
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
        self.spell_table(rng, actor, spell_id, &targets, target_pos);
    }

    /// `gbl.spellTable[spell_id]` (`ovr023.cs:3146-3253`) — the per-spell
    /// function, for §9.1's must-have set.
    ///
    /// Camp-only rows (`0x12` Read Magic, `0x16` Find Traps, `0x27` Cure
    /// Disease, `0x43` Neutralize Poison, `0x4B` Raise Dead) are unreachable
    /// from here: `spell_menu3` refuses them before the cast starts, which is
    /// where the "Camp Only Spell" line comes from (`ovr014.cs:1386`). They
    /// reach the roster instead, through [`crate::camp_cast`].
    fn spell_table(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
        target_pos: GridPos,
    ) {
        match spell_id {
            // `cleric_bless` / `cleric_curse` — one function, two teams
            // (`CastTeamSpell`, `ovr023.cs:990-1011`).
            0x01 => {
                let team = self.fighters[actor].team;
                self.cast_team_spell(rng, actor, spell_id, targets, team);
            }
            0x02 => {
                let team = match self.fighters[actor].team {
                    Team::Party => Team::Monster,
                    Team::Monster => Team::Party,
                }; // `OppositeTeam()` (`Player.cs`)
                self.cast_team_spell(rng, actor, spell_id, targets, team);
            }
            0x03 => self.spell_cure_light(rng, actor, targets),
            // `SpellProtectionFromX` / `is_affected` — `DoSpellCastingWork`
            // with no damage: the row's `affect_id` is the whole effect
            // (`ovr023.cs:1030`, `:1036`).
            0x06 | 0x07 | 0x45 => {
                self.do_spell_casting_work(rng, actor, spell_id, targets, 0, false, 0)
            }
            0x0F => self.spell_magic_missile(rng, actor, spell_id, targets),
            0x15 => self.spell_sleep(rng, actor, spell_id, targets),
            0x17 => self.spell_hold_x(rng, actor, spell_id, targets),
            // `is_affected2` — Slow Poison (`ovr023.cs:1291-1311`).
            0x1A => self.spell_slow_poison(rng, actor, spell_id, targets),
            // `SpellCureBlindness` (`:1587`).
            0x25 => self.spell_cure_blindness(actor, targets),
            0x29 | 0x2E => self.spell_dispel_magic(rng, actor, spell_id, targets, target_pos),
            // `SpellPrayer` (`:1823`): the affect's `data` byte encodes the
            // caster's team in bit 4 and the casting level in the low nibble.
            0x2A => {
                let team_bit = (self.fighters[actor].team as i32) * 16;
                let lvl = self.spell_max_target_count(actor, SpellClass::Cleric);
                self.do_spell_casting_work(rng, actor, spell_id, targets, 0, false, team_bit + lvl)
            }
            // `SpellRemoveCurse` (`:1831`).
            0x2B => self.spell_remove_curse(actor, targets),
            0x2F => self.spell_fireball(rng, actor, spell_id, targets, target_pos),
            // `SpellCureSeriousWounds` (`:2177`) / `SpellCureCriticalWounds`
            // (`:2312`) — `heal_player` with a bigger roll, same shape as CLW.
            0x3A => {
                let heal = i32::from(roll_dice(rng, 8, 2)) + 1;
                self.heal_first_target(actor, targets, heal);
            }
            0x47 => {
                let heal = i32::from(roll_dice(rng, 8, 3)) + 3;
                self.heal_first_target(actor, targets, heal);
            }
            _ => {
                // A transcribed row with no combat handler — the camp-only set.
                // `spell_menu3` should already have refused it; reaching here
                // means the refusal was bypassed, which is a real surprise.
                let id = self.fighters[actor].id;
                self.emit(ActionEvent::StubTripped {
                    combatant_id: id,
                    stub: "spell-camp-only",
                });
            }
        }
    }

    /// `ovr014.target(quick_fight, spell_id)` (`ovr014.cs:1164-1362`) — the
    /// four targeting shapes the low nibble of `field_6` selects, plus
    /// `gbl.targetPos`, which is a separate output the caller's missile camera
    /// reads (doc §41.3 step 2/§48).
    ///
    /// | nibble | shape | must-have rows |
    /// |---|---|---|
    /// | `0` | **self** — clear, add the caster, no pick at all (`:1176-1180`) | Prayer `0x2A`; the camp-only rows |
    /// | `5` | budgeted-multi: a `2d4` power pool spent against each pick's hit dice (`:1182-1268`) | none — **tripwired** |
    /// | `8..=0xE` | **area**: one pick (empty ground allowed), then everyone within `field_6 & 7` of it (`:1294-1312`) | Sleep `0x15`, Dispel Magic `0x29`/`0x2E`, Fireball `0x2F` |
    /// | `0xF` | the held/area hybrid (`:1270-1292`) | none — **tripwired** |
    /// | else | the `max_targets = (field_6 & 3) + 1` loop (`:1314-1358`) | MM `0x0F` (1 pick), CLW `0x03` (1), Hold `0x17` (3), the buffs/cures (1) |
    ///
    /// ★ Roll-credits slice 5 landed the `0` and `8..=0xE` arms; before it,
    /// every non-tail nibble tripped `spell-target-shape`. The tail arm is
    /// byte-for-byte the one the captures ride.
    fn spell_target_with(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targeting: Targeting<'_>,
    ) -> SpellAim {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let nibble = entry.field_6 & 0x0F;
        // `gbl.targetPos = PlayerMapPos(SelectedPlayer)` (@1172) — the default
        // every arm may overwrite.
        let mut aim = SpellAim {
            targets: Vec::new(),
            target_pos: self.fighters[actor].pos,
        };
        match nibble {
            // The self arm (@1176-1180): no pick, no draw, no aim menu.
            0 => {
                aim.targets.push(actor);
                aim
            }
            // The two shapes no must-have row uses, cited and tripped.
            5 | 0x0F => {
                let id = self.fighters[actor].id;
                self.emit(ActionEvent::StubTripped {
                    combatant_id: id,
                    stub: "spell-target-shape",
                });
                aim.targets.clear();
                aim
            }
            // The area arm (@1294-1312): `sub_4001C(canTargetEmptyGround =
            // true)` picks the centre — in QuickFight that is still
            // `find_target`'s d(count), because the AI has no cursor — and then
            // `Rebuild_SortedCombatantList(1, field_6 & 7, targetPos, all)`
            // replaces the list with **everyone** in the blast, both teams and
            // the caster included. A failed pick means no cast.
            8..=0x0E => {
                let Some(centre) = self.pick_one(rng, actor, spell_id, targeting, 0) else {
                    aim.targets.clear();
                    return aim;
                };
                aim.target_pos = self.fighters[centre].pos;
                let radius = (entry.field_6 & 7) as i32;
                aim.targets = self.build_sorted_at(aim.target_pos, radius);
                aim
            }
            // The tail arm (@1314-1358): MM 1 pick, hold person 3.
            _ => {
                let mut max_targets = (entry.field_6 & 3) as i32 + 1;
                let mut index = 0usize;
                while max_targets > 0 {
                    match self.pick_one(rng, actor, spell_id, targeting, index) {
                        Some(t) => {
                            index += 1;
                            if !aim.targets.contains(&t) {
                                aim.targets.push(t);
                                aim.target_pos = self.fighters[t].pos; // @1349
                                max_targets -= 1;
                            } else {
                                // "Already been targeted" — QuickFight
                                // decrements anyway (`:1345-1352`), no
                                // duplicate entry.
                                max_targets -= 1;
                            }
                        }
                        None => max_targets = 0,
                    }
                }
                if aim.targets.is_empty() {
                    // `castSpell = false; gbl.targetPos = new Point()` (@1360).
                    aim.target_pos = GridPos::new(0, 0);
                }
                aim
            }
        }
    }

    /// One `sub_4001C` pick, from whichever side of the `quick_fight` fork this
    /// cast is on.
    ///
    /// ★ **M6c**: with `QuickFight.False` the call opened the **aim menu**
    /// (`ovr014.cs:1098-1103`) rather than drawing, so the picks are already
    /// made and this whole path is draw-free. `index` is which of them this
    /// call consumes — running off the end is the aborted-aim case, which ends
    /// the loop exactly as a failed `find_target` does.
    fn pick_one(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targeting: Targeting<'_>,
        index: usize,
    ) -> Option<usize> {
        match targeting {
            Targeting::Auto => self.sub_4001c(rng, actor, spell_id),
            Targeting::Manual(picked) => picked.get(index).copied(),
        }
    }

    /// `Rebuild_SortedCombatantList(1, max_range, pos, sc => true)` over the
    /// live roster — the unfiltered sorted list anchored on a **map point**,
    /// which is what every spell area shape asks for. Draw-free.
    pub(super) fn build_sorted_at(&self, pos: GridPos, max_range: i32) -> Vec<usize> {
        build_sorted_from(
            &self.map,
            &self.range_combatants(),
            pos,
            1,
            max_range,
            false,
        )
        .into_iter()
        .map(|nt| nt.idx)
        .collect()
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
        targets: &[usize],
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
        self.do_spell_casting_work(rng, actor, spell_id, targets, damage, false, 0);
    }

    /// `DoSpellCastingWork(text, damageFlags, damage, call_affect_table,
    /// TargetCount, spell_id)` (`sub_5CF7F`, `ovr023.cs:573-620`) — the shared
    /// per-target loop almost every spell function ends in.
    ///
    /// Per `gbl.spellTargets` entry, in the original's order:
    /// 1. `saved`: **no draw at all** when `damageOnSave == Normal` (`:587`);
    ///    otherwise one `RollSavingThrow(0, saveVerse, target)` — a d20;
    /// 2. the `fixedRange == -1` to-hit arm (`:594-604`) — a spell that has to
    ///    beat AC (the `cause_*` touch spells). **No must-have row carries
    ///    `-1`**, so it is cited and tripwired rather than guessed at;
    /// 3. `damage > 0` → [`damage_person`](Self::damage_person);
    /// 4. `affect_id > 0` → [`apply_attack_spell_affect`](Self::apply_attack_spell_affect)
    ///    with `GetSpellAffectTimeout`'s minutes.
    ///
    /// `target_count` is the `data` byte the affect record carries — `0` means
    /// "use `spellMaxTargetCount`" (`:580`), which is what every row but Prayer
    /// wants.
    ///
    /// `gbl.damage_flags` (fire/cold/electricity/acid/magic) is set at `:575`
    /// and read by the resist-* affect handlers, none of which is implemented
    /// (§9.1 pruned them all); the flags would only ever scale damage down, so
    /// omitting them is visible as "our fireball does not respect a resist-fire
    /// ring" and is named in §9.2 rather than silently dropped.
    ///
    /// The argument list is the original's own, one for one — collapsing it
    /// into a struct would hide which call site passes what.
    #[allow(clippy::too_many_arguments)]
    fn do_spell_casting_work(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
        damage: i32,
        call_affect_table: bool,
        target_count: i32,
    ) {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        if targets.is_empty() {
            return; // `:577` — the whole body is inside `Count > 0`.
        }
        let data = if target_count > 0 {
            target_count
        } else {
            self.spell_max_target_count(actor, entry.spell_class)
        };
        let casting_lvl = self.spell_max_target_count(actor, entry.spell_class);
        let timeout = spell_affect_timeout(&entry, casting_lvl);
        for &target in targets {
            let saved = if entry.damage_on_save == DamageOnSave::Normal {
                false // `:587` — Normal never rolls
            } else {
                self.do_saving_throw(rng, 0, entry.save_verse, target)
            };
            if entry.fixed_range == -1 {
                // The to-hit arm (`:594-604`): `reclac_player_values` +
                // `CheckAffectsEffect(Type_11)` + `PC_CanHitTarget`. Reached by
                // no §9.1 row (every one has `fixedRange >= 0`).
                let id = self.fighters[actor].id;
                self.emit(ActionEvent::StubTripped {
                    combatant_id: id,
                    stub: "spell-touch-attack",
                });
                continue;
            }
            if damage > 0 {
                self.damage_person(rng, actor, target, saved, entry.damage_on_save, damage);
            }
            if entry.affect_id > 0 {
                self.apply_attack_spell_affect(
                    target,
                    saved,
                    entry.damage_on_save,
                    call_affect_table,
                    data,
                    timeout,
                    entry.affect_id,
                );
            }
        }
    }

    /// `damage_person(change_damage, arg_2, damage, player)`
    /// (`ovr024.cs:1180-1288`): the save scaling, then the damage.
    ///
    /// - `CheckAffectsEffect(PreDamage)` first (`:1186`) — draw-free, and every
    ///   handler on that list is still tripwired;
    /// - a made save scales: `Zero` → 0, `Half` → `damage / 2` (`:1188-1198`);
    ///   a **failed** save runs `CheckAffectsEffect(FireShield)` instead
    ///   (`:1201`) — the retaliation hook, also tripwired;
    /// - then `damage_player` (our [`apply_damage`](Self::apply_damage)) and
    ///   `TryLooseSpell` (`:1244`) — the disruption that costs a caster its
    ///   queued spell and its `can_cast` for the round (`ovr024.cs:1288-1300`),
    ///   the same tail the melee swing already runs (§45).
    ///
    /// ★ **`SpellDamage` is emitted from here, after the scaling** — a bug the
    /// M6a reel's own board reconcile caught the moment a `DamageOnSave::Half`
    /// row existed to catch it. The event used to ride at the `DoSpellCastingWork`
    /// call site with the *unscaled* number, which was invisible while Magic
    /// Missile (`Normal`, never scaled) was the only damage spell and drifted
    /// the presented board by half a fireball the instant one landed. The
    /// original prints the same number this event now carries: `damage_person`'s
    /// "takes N points of damage" reads `gbl.damage` **after** the halving
    /// (`ovr024.cs:1204-1208`). Draw-neutral — no draw moves, and Magic
    /// Missile's event is byte-identical because its damage is never scaled.
    fn damage_person(
        &mut self,
        rng: &mut EngineRng,
        caster: usize,
        target: usize,
        saved: bool,
        on_save: DamageOnSave,
        damage: i32,
    ) {
        self.check_affects_effect(target, CheckType::PreDamage);
        let mut dealt = damage;
        if saved {
            match on_save {
                DamageOnSave::Zero => dealt = 0,
                DamageOnSave::Half => dealt /= 2,
                _ => {}
            }
        } else {
            self.check_affects_effect(target, CheckType::FireShield);
        }
        if dealt <= 0 {
            return;
        }
        // D-CV2 `SpellDamage`, before the cascade its `Removed` comes out of —
        // the same head-of-branch placement `SlayHelpless` uses, and the order
        // the original displays: the effect, then the fall.
        self.emit(ActionEvent::SpellDamage {
            caster_id: caster,
            target_id: target,
            amount: dealt,
        });
        self.apply_damage(rng, target, dealt);
        // `TryLooseSpell` (`ovr024.cs:1244` → `:1288-1300`): any real damage
        // kills this round's casting, and a queued cast is lost outright —
        // "lost a spell", with `ClearSpell` taking the memorized entry with it.
        // The same tail the melee swing already runs (§45).
        self.fighters[target].can_cast = false;
        if let Some(queued) = self.fighters[target].pending_spell.take() {
            self.clear_spell(target, queued);
        }
    }

    /// `ApplyAttackSpellAffect(text, saved, can_save, call_affect_table, data,
    /// time, affect_id, target)` (`is_unaffected`, `ovr024.cs:1303-1332`).
    ///
    /// The `MagicResistance` hook runs first and can zero `gbl.current_affect`
    /// (`:1307`) — draw-free, and every handler on that list is tripwired, so
    /// today it never does. Then: a made save on a `DamageOnSave::Zero` row is
    /// "is Unaffected" and nothing happens; otherwise an existing instance with
    /// `minutes > 0` is removed and a fresh one added.
    #[allow(clippy::too_many_arguments)]
    fn apply_attack_spell_affect(
        &mut self,
        target: usize,
        saved: bool,
        on_save: DamageOnSave,
        call_affect_table: bool,
        data: i32,
        minutes: u16,
        affect_id: u8,
    ) {
        self.check_affects_effect(target, CheckType::MagicResistance);
        if saved && on_save == DamageOnSave::Zero {
            return; // "is Unaffected" — display only.
        }
        if self.fighters[target]
            .affects
            .iter()
            .any(|a| a.kind == affect_id && a.minutes > 0)
        {
            self.remove_affect(target, affect_id);
        }
        self.fighters[target].add_affect(affect_id, minutes, data as u8, call_affect_table);
        self.emit(ActionEvent::AffectApplied {
            target_id: target,
            affect_id,
        });
    }

    /// `CastTeamSpell(text, team)` (`sub_5DCA0`, `ovr023.cs:989-997`) — Bless
    /// and Curse, one function.
    ///
    /// The area shape has already put **everyone** within two squares of the
    /// caster into `spellTargets`; this filter keeps the named team and then —
    /// for Bless in combat only — drops anybody who already has an enemy
    /// adjacent (`BuildNearTargets(1, target).Count > 0`, `:994`). That is the
    /// AD&D rule that Bless cannot be cast on troops already engaged in melee,
    /// and it is why a bless is worth casting *before* the lines meet.
    /// Draw-free (the near-list builder does not roll).
    fn cast_team_spell(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
        team: Team,
    ) {
        let kept: Vec<usize> = targets
            .iter()
            .copied()
            .filter(|&t| {
                if self.fighters[t].team != team {
                    return false;
                }
                if spell_id == 0x01 && !self.build_near(t, 1, false).is_empty() {
                    return false; // engaged in melee — Bless only (`:994`)
                }
                true
            })
            .collect();
        self.do_spell_casting_work(rng, actor, spell_id, &kept, 0, false, 0);
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

    // === roll-credits slice 5: the rest of §9.1's combat-castable set =======

    /// `SpellSleep` (`falls_asleep`, `ovr023.cs:1187-1209`) — the area list the
    /// `field_6 = 9` shape built, spent against a **4d4 power pool**.
    ///
    /// One `roll_dice(4, 4)` (four d4s) up front, then the list is filtered in
    /// order: a target that is not `animated`, does not already carry `sleep`,
    /// and whose hit-dice cost fits the remaining pool is **kept** and pays its
    /// cost; everybody else is dropped. The cost ladder is
    /// [`Self::calc_sleep_cost`]. The survivors then go through
    /// `DoSpellCastingWork`, whose `damageOnSave == Normal` means **no saving
    /// throw at all** — Sleep in this engine is a resource contest, not a save.
    ///
    /// The pool is spent in `spellTargets` order, which the area shape sorted
    /// nearest-first, so a big monster standing closest can eat the whole
    /// spell.
    fn spell_sleep(&mut self, rng: &mut EngineRng, actor: usize, spell_id: u8, targets: &[usize]) {
        let mut power = i32::from(roll_dice(rng, 4, 4)); // `:1190`
        let mut kept = Vec::with_capacity(targets.len());
        for &t in targets {
            let cost = self.calc_sleep_cost(t);
            let f = &self.fighters[t];
            if f.health_status != HealthStatus::Animated
                && !f.has_affect(crate::spells::AFF_SLEEP)
                && power >= cost
            {
                power -= cost;
                kept.push(t);
            }
        }
        self.do_spell_casting_work(rng, actor, spell_id, &kept, 0, false, 0);
    }

    /// `CalcSleepCost(target)` (`ovr023.cs:1211-1245`): 0-1 HD → 1, 2 → 2,
    /// 3 → 4, 4 → 6, 5 → **10 for a monster, 20 for anything else**, 6+ → 20.
    /// The race check on the 5-HD rung is the original's own (`race ==
    /// Race.monster`, id 8 in `Classes/Enums.cs`).
    pub(super) fn calc_sleep_cost(&self, target: usize) -> i32 {
        const RACE_MONSTER: u8 = 8;
        let f = &self.fighters[target];
        match f.hit_dice {
            0 | 1 => 1,
            2 => 2,
            3 => 4,
            4 => 6,
            5 if f.race == RACE_MONSTER => 10,
            _ => 20, // 5 HD non-monster, and everything above 5
        }
    }

    /// `is_affected2` — Slow Poison (`ovr023.cs:1291-1311`).
    ///
    /// Three arms, and the middle one is the point of the spell: an `animated`
    /// target clears the list and nothing happens; a target carrying `poisoned`
    /// is **pulled off the floor at 1 hit point** if it was at 0, gets the
    /// `slow_poison` affect through `DoSpellCastingWork` (with
    /// `call_affect_table = true` and `data = 0xFF`), loses `affect_4e`, and
    /// is re-armed with a fresh 10-minute `poison_damage` countdown; a target
    /// that is not poisoned at all gets nothing.
    ///
    /// So Slow Poison does not cure — it buys ten minutes and a heartbeat, and
    /// when `slow_poison` finally times out its own handler
    /// (`AffectSlowPoison`, `ovr013.cs:305-317`) kills anybody still poisoned.
    /// That timeout only runs in camp ([`crate::affects`]), which is where the
    /// spell is worth casting.
    fn spell_slow_poison(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
    ) {
        let Some(&target) = targets.first() else {
            return;
        };
        if self.fighters[target].health_status == HealthStatus::Animated {
            return; // `:1296` — spellTargets.Clear()
        }
        if !self.fighters[target].has_affect(crate::spells::AFF_POISONED) {
            return;
        }
        if self.fighters[target].hp_current == 0 {
            self.fighters[target].hp_current = 1; // `:1301`
        }
        self.do_spell_casting_work(rng, actor, spell_id, targets, 0, true, 0xFF);
        self.remove_affect(target, crate::spells::AFF_4E);
        self.fighters[target].add_affect(crate::spells::AFF_POISON_DAMAGE, 10, 0xFF, true);
    }

    /// `SpellCureBlindness` (`can_see`, `ovr023.cs:1587-1593`): `cure_affect`
    /// on `blinded`, and the "can see" line only when something was cured.
    /// Draw-free.
    fn spell_cure_blindness(&mut self, actor: usize, targets: &[usize]) {
        let Some(&target) = targets.first() else {
            return;
        };
        if self.fighters[target].has_affect(crate::spells::AFF_BLINDED) {
            self.remove_affect(target, crate::spells::AFF_BLINDED);
            self.emit(ActionEvent::AffectCured {
                caster_id: actor,
                target_id: target,
                affect_id: crate::spells::AFF_BLINDED,
            });
        }
    }

    /// `SpellRemoveCurse` (`uncurse`, `ovr023.cs:1831-1864`): cure
    /// `bestow_curse` if it is there; **otherwise** find the first cursed item
    /// and un-ready it.
    ///
    /// The item arm needs the inventory, which a combatant does not carry (the
    /// record image cannot hold a heap list — the same reason `has_items` is a
    /// bool). In combat it is therefore cited and tripped; the real one lives
    /// on the roster side, in [`crate::camp_cast`], where the items are.
    fn spell_remove_curse(&mut self, actor: usize, targets: &[usize]) {
        let Some(&target) = targets.first() else {
            return;
        };
        if self.fighters[target].has_affect(crate::spells::AFF_BESTOW_CURSE) {
            self.remove_affect(target, crate::spells::AFF_BESTOW_CURSE);
            self.emit(ActionEvent::AffectCured {
                caster_id: actor,
                target_id: target,
                affect_id: crate::spells::AFF_BESTOW_CURSE,
            });
            return;
        }
        let id = self.fighters[actor].id;
        self.emit(ActionEvent::StubTripped {
            combatant_id: id,
            stub: "uncurse-item",
        });
    }

    /// `SpellDispelMagic` (`is_affected3`, `ovr023.cs:1667-1822`).
    ///
    /// Per affect on the first target whose `affect_data < 0xFF`, a **d100**
    /// against a level-difference ladder (`:1690-1704`): the caster's casting
    /// level versus the affect's own stored level (`affect_data & 0x0F`) —
    /// equal is 50%, each level above adds 5, each level below subtracts 2.
    /// Every affect that beats its roll is removed. `affect_data == 0xFF` is
    /// the "not from a spell" marker every racial and item affect carries, and
    /// it is what makes a dwarf's `dwarf_vs_orc` undispellable.
    ///
    /// The tail (`:1719-1822`) sweeps the nine cells around `targetPos` for gas
    /// clouds and summoned tiles to dispel as well. No gas-cloud subsystem
    /// exists (§9.1 pruned every cloud spell), so that half is cited and
    /// tripped rather than half-built.
    ///
    /// ★ **Draw-bearing**, one d100 per dispellable affect — reachable only by
    /// casting Dispel Magic, which no capture does.
    fn spell_dispel_magic(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
        _target_pos: GridPos,
    ) {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let max_target_count = self.spell_max_target_count(actor, entry.spell_class);
        if let Some(&target) = targets.first() {
            let candidates: Vec<(u8, i32)> = self.fighters[target]
                .affects
                .iter()
                .filter(|a| a.data < 0xFF)
                .map(|a| (a.kind, i32::from(a.data & 0x0F)))
                .collect();
            for (kind, level) in candidates {
                let needed = if max_target_count > level {
                    50 + (max_target_count - level) * 5
                } else if max_target_count < level {
                    50 - (level - max_target_count) * 2
                } else {
                    50
                };
                // `roll_dice(100, 1)` — `Random(100) + 1`, compared with `<=`.
                if i32::from(roll_dice(rng, 100, 1)) <= needed {
                    self.remove_affect(target, kind);
                    self.emit(ActionEvent::AffectCured {
                        caster_id: actor,
                        target_id: target,
                        affect_id: kind,
                    });
                }
            }
        }
        // The nine-cell cloud/summon sweep (`:1719-1822`).
        let id = self.fighters[actor].id;
        self.emit(ActionEvent::StubTripped {
            combatant_id: id,
            stub: "dispel-ground-sweep",
        });
    }

    /// `sub_5F782` — Fireball (`ovr023.cs:1878-1907`).
    ///
    /// `dice_count = spellMaxTargetCount` (the caster's magic-user level), then
    /// **one `roll_dice_save(6, dice_count)`** — `dice_count` separate d6s,
    /// byte-summed — and `DoSpellCastingWork` spreads that single number over
    /// every target with `DamageOnSave::Half`, i.e. one save each.
    ///
    /// The `inDungeon == 0` re-target (`:1894-1902`) rebuilds `spellTargets`
    /// from a **radius-2** list around `targetPos` instead of the row's own
    /// radius 3 — the outdoor blast is smaller. **Still cited, not
    /// implemented**: roll-credits slice 7 landed the wilderness *floor*
    /// (`crate::combat::floor::setup_wilderness_floor`) but `CombatState`
    /// still carries no dungeon/wilderness flag of its own, and inventing one
    /// for a spell no capture casts is the wrong slice's work. Named in
    /// slice 7's residuals.
    ///
    /// ★ **Draw-bearing** (the d6 volley plus a d20 per target) and reachable
    /// only by casting Fireball, which no capture does.
    fn spell_fireball(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        spell_id: u8,
        targets: &[usize],
        _target_pos: GridPos,
    ) {
        let entry = spell_entry(spell_id).expect("caller guarantees a transcribed id");
        let dice_count = self.spell_max_target_count(actor, entry.spell_class);
        let damage = i32::from(roll_dice(rng, 6, dice_count.max(0) as u16));
        self.do_spell_casting_work(rng, actor, spell_id, targets, damage, false, 0);
    }

    /// `heal_player(0, roll, spellTargets[0])` (`ovr024.cs:1335-1370`) — the
    /// shared tail of Cure Serious (`2d8+1`) and Cure Critical (`3d8+3`), which
    /// are `SpellCureLight` with a bigger roll. The caller does the rolling so
    /// the draw sits at the original's own site.
    fn heal_first_target(&mut self, actor: usize, targets: &[usize], heal: i32) {
        let Some(&target) = targets.first() else {
            return;
        };
        if self.fighters[target].health_status == HealthStatus::Dead {
            return;
        }
        let f = &mut self.fighters[target];
        f.hp_current = (f.hp_current + heal).min(f.hp_max);
        self.emit(ActionEvent::Healed {
            healer_id: actor,
            target_id: target,
            amount: heal,
            kind: HealKind::Cure,
        });
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
