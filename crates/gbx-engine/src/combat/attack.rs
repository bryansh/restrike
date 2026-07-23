use super::*;

impl CombatState {
    /// The range layer's view of the roster (`size = 0` for the dead, so they drop
    /// out of target lists — matching coab's `combatantMap.size > 0` gate).
    fn range_combatants(&self) -> Vec<RangeCombatant> {
        self.fighters
            .iter()
            .map(|f| RangeCombatant {
                pos: f.pos,
                size: if f.in_combat { f.size } else { 0 },
                team: f.team,
            })
            .collect()
    }

    /// `BuildNearTargets(max_range, actor)` over the live roster.
    pub(super) fn build_near(
        &self,
        actor: usize,
        max_range: i32,
        ignore_walls: bool,
    ) -> Vec<NearTarget> {
        build_near_targets(
            &self.map,
            &self.range_combatants(),
            actor,
            max_range,
            ignore_walls,
        )
    }

    /// `find_target(clear, arg_2, max_range, actor)` (`ovr014.cs:2238`): keep a
    /// still-valid target (**0 draws**), else pick a random near-target
    /// (`roll_dice(nearTargets.Count, 1)` per retry, `:2275`). With no invisibility
    /// modeled, `CanSeeTargetA` is always true, so the first pick succeeds — exactly
    /// **1 draw** when a target is found from scratch, 0 when none exist or the old
    /// target survives. Two passes (the second `ignoreWalls`) as coab.
    pub(super) fn find_target(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        clear: bool,
        arg_2: u8,
        max_range: i32,
    ) -> bool {
        let team = self.fighters[actor].team;
        let invalidate = clear
            || match self.fighters[actor].target {
                Some(t) => {
                    let tf = &self.fighters[t];
                    tf.team == team || !tf.in_combat || !self.can_see_target(t)
                }
                None => false,
            };
        if invalidate {
            self.fighters[actor].target = None;
        }
        if self.fighters[actor].target.is_some() {
            return true;
        }

        let mut found = false;
        let mut second_pass = false;
        let mut var_5 = false;
        while !found && !var_5 {
            var_5 = second_pass;
            let ignore_walls = second_pass && !clear;
            let mut near = self.build_near(actor, max_range, ignore_walls);
            let mut try_count = 20;
            while try_count > 0 && !found && !near.is_empty() {
                try_count -= 1;
                let roll = roll_dice(rng, near.len() as u16, 1); // ovr014.cs:2275
                let epi = near[(roll - 1) as usize];
                if (arg_2 != 0 && ignore_walls) || self.can_see_target(epi.idx) {
                    found = true;
                    self.fighters[actor].target = Some(epi.idx);
                } else {
                    near.retain(|n| n.idx != epi.idx);
                }
            }
            if !second_pass {
                second_pass = true;
            }
        }
        found
    }

    /// `damage_player` (`ovr025:23D5`, coab ovr025.cs:1183-1242) — apply melee
    /// damage and run the health-status ladder (§26.1). `neg_hp` is the overkill
    /// (`damage − hp`, else 0); `new_hp` the survivor's HP (`hp − damage`, else 0):
    /// - overkill `> 9`, or a `new_hp == 0` hit on an `animated` combatant → **dead**;
    /// - else overkill `1..=9` → **dying**, and `actions.bleeding = neg_hp`;
    /// - else an exact drop to 0 (`new_hp == 0`) → **unconscious**.
    ///
    /// A combatant left `okey`/`animated` keeps `new_hp` and stays in combat; any
    /// other status flips `in_combat = false`, zeroes HP and `actions.delay`
    /// (`ovr025:24BB` — the corpse can never win a `FindNextCombatant` pass, bug
    /// #9), and frees its occupancy footprint immediately (`CombatantKilled`,
    /// bug #10). `gbl.game_state == GameState.Combat` holds on this path, so the
    /// `bleeding` and `delay = 0` writes are unconditional here.
    pub(super) fn apply_damage(&mut self, target: usize, amount: i32) {
        let t = &mut self.fighters[target];
        let (neg_hp, new_hp) = if t.hp_current >= amount {
            (0, t.hp_current - amount)
        } else {
            (amount - t.hp_current, 0)
        };

        // The ladder (ovr025.cs:1197-1216).
        if neg_hp > 9 || (new_hp == 0 && t.health_status == HealthStatus::Animated) {
            t.health_status = HealthStatus::Dead;
        } else if neg_hp > 0 {
            t.health_status = HealthStatus::Dying;
            t.bleeding = neg_hp as u8;
        } else if new_hp == 0 {
            t.health_status = HealthStatus::Unconscious;
        }

        // Survivor (ovr025.cs:1218): status stayed okey/animated → keep the
        // reduced HP, stay in combat.
        if t.health_status.is_conscious() {
            t.hp_current = new_hp;
            return;
        }

        // Removed from combat (ovr025.cs:1220-1240). Site 6 (death path) —
        // `CombatantKilled` (`sub_74E6F` @`ovr033:550`) FIRST scrolls the
        // camera to the victim if it is off-screen: `if (PlayerOnScreen(true,
        // victim) == false) redrawCombatArea(8, 3, victim.pos)`, evaluated while
        // the victim is still present (before `size = 0`). **Deviation:** the
        // spec's site 6 cites `RemoveFromCombat`'s FOCUS-gated scroll, but the
        // damage-death path is `CombatantKilled`, ON-SCREEN-gated (bring an
        // off-screen death into view) — a distinct gate (binary `sub_74E6F`).
        let pos = self.fighters[target].pos;
        if !self.on_screen_pos(pos) {
            self.redraw_combat_area(8, 3, pos);
        }
        let t = &mut self.fighters[target];
        t.hp_current = 0;
        t.in_combat = false;
        t.delay = 0;
        let downed_party = t.team == Team::Party;
        // §39.5 site 9/10: the weapon death tail (`DisplayAttackMessage`, coab
        // ovr014.cs:209-210) runs `RemoveCombatAffects(target)` (`sub_645AB` call
        // @`ovr014:0622`) then `CheckAffectsEffect(target, Death)` (`mov al,0Dh`
        // @`ovr014:0630`) once `in_combat == false`, before `CombatantKilled`.
        // (PreDamage/FireShield are NOT here — they live in `damage_person`, the
        // spell/effect-damage entry; the weapon path is DisplayAttackMessage →
        // damage_player, doc §40.) Draw-free.
        self.remove_combat_affects(target);
        self.check_affects_effect(target, CheckType::Death);
        // `CombatantKilled` (`sub_74E6F`, `ovr033:534`→coab): the removal path the
        // damage caller reaches whenever `in_combat == false` (`ovr014.cs:214`),
        // so it fires for dying/unconscious/dead alike. §26.5 — for a downed
        // party member (`nonTeamMember == false`, modeled as `team == Party`),
        // stamp `Tile_DownPlayer` at its cell unless a `Tile_StinkingCloud`
        // already occupies it (`ovr033.cs:579-590`). Movement-/reach-neutral on a
        // cost-1 floor (the tile constants match a floor's) — fidelity, and it
        // must precede the occupancy repaint, matching coab's order.
        if downed_party && self.map.ground_tile(pos) != TILE_STINKING_CLOUD {
            self.map.set_tile(pos, TILE_DOWN_PLAYER);
        }
        // `CombatantKilled` then zeroes `CombatMap[idx].size` + calls `sub_743E7`
        // (`setup_mapToPlayerIndex_and_playerScreen`): the occupancy repaint
        // happens AT removal, so a corpse's cells free up immediately (a later
        // mover's `CanMove` must see them empty), not at the next position change.
        self.rebuild_occupancy();
    }

    /// `AttackTarget`'s direction bookkeeping (`sub_3F9DB` @`ovr014:19FE-1AD2`,
    /// coab ovr014.cs:913-936, §36.1). Draw-free bookkeeping — the camera scroll
    /// never enters the PRNG stream, and the target-side draw fires only when the
    /// target is on-screen so its off-screen recenter can't run.
    ///
    /// The **target's** facing (`attack_type_nonzero` = the caller's `attackType
    /// != 0`):
    /// - **Branch 1** (@`19FE-1A39`) — `AttacksReceived < 2 && attackType == 0`:
    ///   `var_9 = getTargetDirection(attacker, target)` = `target_direction(target,
    ///   attacker)` = the bearing from the target toward its attacker; store the
    ///   **face-away** `(var_9 + 4) % 8` (@`1A35`, unconditional).
    /// - **Branch 2** (`loc_3FA3B` @`1A3B-1A79`) — else: only touch facing if the
    ///   target is on-screen; then `var_9 = direction`, and if `attackType == 0`
    ///   store the 180° flip `(var_9 + 4) % 8` (@`1A79`).
    /// - **Shared tail** (`loc_3FA7D` @`1A7D-1A9F`): if the target is on-screen,
    ///   `draw_74B3F(false, Normal, var_9, target)` stores `var_9`
    ///   **unconditionally** — the on-screen **draw overwrite**. Branch 1 → the
    ///   target ends up FACING its attacker (`var_9` = bearing, overwrites the
    ///   face-away store: the §35 crack). Branch 2 → `var_9` = the old direction,
    ///   so the flip is restored → net no-op.
    ///
    /// Then the **attacker ALWAYS faces its target** (`loc_3FAA4` @`1AA4-1AD2`):
    /// `draw_74B3F(false, Attack, getTargetDirection(target, attacker), attacker)`
    /// = `target_direction(attacker, target)`, an unconditional store.
    ///
    /// Net (melee `attackType == 0`): 1st attack on-screen → target faces the
    /// attacker; 1st off-screen → faces away; 2nd+ → unchanged; `attackType != 0`
    /// → unchanged. The facing-equality reads (flanking/backstab) therefore see a
    /// target FACING its attacker in melee — which is why the §35 face-away-only
    /// transliterations over-fired.
    pub(super) fn attack_target_facing(
        &mut self,
        target: usize,
        attacker: usize,
        attack_type_nonzero: bool,
    ) {
        let tgt_on_screen = self.on_screen(target);
        let var_9: u8 = if self.fighters[target].attacks_received < 2 && !attack_type_nonzero {
            // Branch 1 (@1A0B-1A39): var_9 = bearing target→attacker; store
            // face-away unconditionally (the tail draw overwrites it on-screen).
            let bearing = target_direction(self.fighters[target].pos, self.fighters[attacker].pos);
            self.fighters[target].direction = (bearing + 4) % 8; // @1A35
            bearing
        } else {
            // Branch 2 (loc_3FA3B @1A3B): the binary reads `direction` only after
            // the on-screen gate; reading it unconditionally is harmless because
            // `var_9` feeds only the on-screen-gated tail draw.
            let old = self.fighters[target].direction; // @1A55
            if tgt_on_screen && !attack_type_nonzero {
                self.fighters[target].direction = (old + 4) % 8; // @1A79 flip
            }
            old
        };
        // Shared tail (loc_3FA7D @1A7D): the on-screen draw overwrite (@1A9F).
        if tgt_on_screen {
            self.draw_74b3f(target, var_9);
        }
        // loc_3FAA4 @1AA4-1AD2: the attacker always faces its target.
        let face = target_direction(self.fighters[attacker].pos, self.fighters[target].pos);
        self.draw_74b3f(attacker, face);
    }

    /// The flanking heuristic (`AttackTarget01` @`ovr014:16AD-16E9`, coab
    /// ovr014.cs:782-784, §36.4): a swarmed target whose back is turned to this
    /// attacker is hit on its **behind** AC. All three must hold:
    /// - `AttacksReceived > 1` (@`16B5`, `jbe` skips ≤ 1) — the target has taken
    ///   more than one swing since its last move (swarmed this turn);
    /// - `getTargetDirection(target, attacker) == direction` (@`16C9-16D4`) =
    ///   `target_direction(attacker, target) == target.direction` — the attacker's
    ///   bearing toward the target equals the target's facing, i.e. the target
    ///   faces AWAY from the attacker (the attacker is behind it);
    /// - `directionChanges > 4` (@`16E2`, `jbe` skips ≤ 4) — the target has been
    ///   spun enough this turn.
    ///
    /// Guarded by `!CanBackStabTarget` in the binary (the `else` at `loc_3F6AD`);
    /// backstab preempts with `ac_behind − 4` and lands next commit, so here
    /// CanBackStab is treated as false and the gate is vacuously satisfied.
    pub(super) fn is_flanking(&self, target: usize, attacker: usize) -> bool {
        self.fighters[target].attacks_received > 1
            && target_direction(self.fighters[attacker].pos, self.fighters[target].pos)
                == self.fighters[target].direction
            && self.fighters[target].direction_changes > 4
    }

    /// `CanBackStabTarget(target, attacker)` (`sub_408D7` @`ovr014:28D7-29B9`, coab
    /// ovr014.cs:1433-1457, §36.4). All must hold:
    /// - **class** (@`291C-293C`): `attacker.SkillLevel(Thief) > 0` — our decoded
    ///   `thief_skill_level` (the inlined `field_10F`/`field_117`/`sub_6B3D1`
    ///   dual-class fold, `Player.cs:492`);
    /// - **weapon** (@`293E-2962`): the attacker's `primaryWeapon` (`field_151`)
    ///   is null (bare hands — an unreadied loadout, `weapon_readied == false`) OR
    ///   its type ∈ {Club 7, Dagger 8, BroadSword 35, LongSword 36, ShortSword 37,
    ///   DrowLongSword 97};
    /// - **swarm** (@`2976`, `jbe ≤1`): `target.AttacksReceived > 1`;
    /// - **size** (@`2980-2989`): `(target.field_DE & 0x7F) <= 1` (man-sized);
    /// - **facing** (@`298B-29A3`): `getTargetDirection(target, attacker) ==
    ///   target.direction` = `target_direction(attacker, target) ==
    ///   target.direction` (the target's back is to the attacker — same test as
    ///   flanking).
    ///
    /// Fires `ac_behind − 4` (@`169E-16A5`) and the damage multiplier
    /// `((SkillLevel(Thief) − 1) / 4) + 2` ([`backstab_multiplier`], `sub_3E192`
    /// @`ovr014.cs:96`). Preempts the flanking heuristic (the binary's `else`).
    pub(super) fn can_backstab(&self, target: usize, attacker: usize) -> bool {
        if self.fighters[attacker].thief_skill_level <= 0 {
            return false;
        }
        // `weapon == null` ⟺ the primary is not readied (a depleted/unreadied
        // loadout → bare hands, or no loadout at all → `weapon_readied == false`).
        let weapon_ok = if self.fighters[attacker].weapon_readied {
            match &self.fighters[attacker].loadout {
                Some(l) => matches!(l.primary_type, 7 | 8 | 35 | 36 | 37 | 97),
                None => false, // readied with no loadout is unreachable, but be safe
            }
        } else {
            true // null primaryWeapon → bare hands → backstab-capable
        };
        weapon_ok
            && self.fighters[target].attacks_received > 1
            && (self.fighters[target].field_de & 0x7F) <= 1
            && target_direction(self.fighters[attacker].pos, self.fighters[target].pos)
                == self.fighters[target].direction
    }

    /// `AttackTarget → AttackTarget01` (`ovr014.cs:904/724`), melee core: for
    /// `attackIdx` counting down from `attack_idx`, drain `AttacksLeft(attackIdx)`
    /// swings — each **one d20** to-hit ([`pc_can_hit_target`]); **on a hit only**,
    /// profile-1 damage ([`roll_damage`]). A hit that kills the target sets
    /// `targetNotInCombat` and stops the remaining swings (no further draws). Sets
    /// `delay = 0` (via `clear_actions`) when the turn's attacks are spent, and
    /// returns `turnComplete`. Backstab/behind AC and the held-slay path are
    /// deferred (raw AC used).
    /// `behind`: `AttackTarget`'s `attackType` arg ≠ 0 (`BehindAttack`,
    /// ovr014.cs:728). The departure opportunity attack passes 1
    /// (ovr014.cs:407); the into-reach and normal turn attacks pass 0. The
    /// `AttacksReceived>1 && facing && directionChanges>4` flanking heuristic
    /// (`ovr014:16BA-16E9`) and backstab's `ac_behind − 4` (`:169E`) are
    /// cited-deferred (M5) — no capture exercises them yet.
    pub(super) fn attack_target(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        target: usize,
        behind: bool,
        ranged_item: AttackItemRef,
    ) -> bool {
        // AttackTarget (`sub_3F9DB` @`ovr014:19DB`) head: focus the camera on
        // the attacker (`ovr014.cs:908`).
        self.focus = true;
        // §36.1 direction bookkeeping (`sub_3F9DB` @`ovr014:19FE-1AD2`): the
        // target-side facing store + the on-screen draw overwrite, then the
        // attacker ALWAYS faces its target. `behind` here is the caller's
        // `attackType != 0` (departure/behind attacks pass 1) — `attackType != 0`
        // leaves the target's facing untouched. Draw-free (the camera scroll
        // never enters the PRNG stream; the target-side draw fires only on-screen
        // so its recenter can't). The AC-select `behind` decision is derived from
        // this same flag today; the flanking/backstab heuristics diverge it later.
        self.attack_target_facing(target, actor, behind);
        // §19: `AttackTarget` (`sub_3F9DB`, ovr014.cs:939) sets
        // `attacker.actions.target = target` — the attacked (possibly re-picked)
        // combatant becomes the persistent target, so next round's `find_target`
        // keeps it draw-free. Draw-free; only the *held target* carried into later
        // rounds changes (the §18 re-pick correctly writes only a local `chosen`).
        self.fighters[actor].target = Some(target);
        // Site 5 — the ranged missile camera (`ovr014.cs:945` → `draw_missile_attack`,
        // `sub_67AA4`): a bow/thrown shot animates the missile across the board
        // and scrolls the camera toward the target. Draw-free; only its
        // `mapScreenTopLeft` effect is ported ([`draw_missile_camera`]). A plain
        // melee swing (null item) fires no missile.
        if matches!(ranged_item, AttackItemRef::Ammo | AttackItemRef::SelfWeapon) {
            self.draw_missile_camera(actor, target);
        }
        // ...and `sub_3F9DB` fires `sub_40BF1` a SECOND time, with the readied
        // primary itself as the missile, when that primary is a **Sling (0x2F)**
        // or **StaffSling (0x65)** (@`ovr014:1B14-1B4C`). This is the branch that
        // gives a sling its missile: `GetCurrentAttackItem` returns a
        // found-but-NULL item for flags `0x0A` (§34.2), so the item-gated call
        // above does not fire for it. Draw-free like the first — but the camera
        // scroll is exactly the state the §36.1 on-screen facing branch reads, so
        // "draw-free" is not a reason to skip it. The binary dereferences
        // `field_151` here with no null check (UB for bare hands); we gate on the
        // primary actually being readied. No capture carries a sling loadout.
        const ITEM_SLING: u8 = 0x2F;
        const ITEM_STAFF_SLING: u8 = 0x65;
        let sling_primary = self.fighters[actor].weapon_readied
            && matches!(
                self.fighters[actor].loadout.map(|l| l.primary_type),
                Some(ITEM_SLING) | Some(ITEM_STAFF_SLING)
            );
        if sling_primary {
            self.draw_missile_camera(actor, target);
        }
        // `AttackTarget01` sets `actions.field_8 = true` (`ovr014.cs:738`) — the
        // "attacked this round" flag `reclac_attacks`'s write-back gate reads
        // (§34.3); `CalculateInitiative` resets it each round.
        self.fighters[actor].field_8 = true;
        if self.fighters[actor].attack1_left == 0 && self.fighters[actor].attack2_left == 0 {
            self.clear_actions(actor);
            return true;
        }
        // §39.5 site 6: `CheckAffectsEffect(target, Type_11)` — after
        // `reclac_player_values(target)` and before the AC selection, once per
        // attack (`mov al,0Bh; call work_on_00` @`ovr014:167E`, coab
        // ovr014.cs:774). Draw-free (empty lists).
        self.check_affects_effect(target, CheckType::Type11);
        // AttackTarget01's AC selection (`sub_3F4EB` @`ovr014:1683-1708`, §36.4).
        // Backstab preempts (the binary's `if CanBackStabTarget` @`1694`, `else`
        // the flanking/behind path @`16AD`): `ac_behind − 4` (@`169E-16A5`).
        // Otherwise the AC byte is record[0x19A + behindIdx] — front @0x19A,
        // behind @0x19B — with behindIdx set when `var_13 != 0` (@`16ED-16F3`):
        // the caller's `attackType != 0` (`behind`) OR the flanking heuristic.
        // Then `target_ac += RangedDefenseBonus` on EVERY path (`ovr014.cs:799`).
        let can_backstab = self.can_backstab(target, actor);
        let base_ac = if can_backstab {
            self.fighters[target].ac_behind as i32 - 4
        } else {
            let behind_attack = behind || self.is_flanking(target, actor);
            (if behind_attack {
                self.fighters[target].ac_behind
            } else {
                self.fighters[target].ac
            }) as i32
        };
        let target_ac = (base_ac + self.ranged_defense_bonus(actor, target)).clamp(0, 255) as u8;
        let hit_bonus = self.fighters[actor].hit_bonus;
        let mut target_gone = false;
        // `bytes_1D900[1]` — the attack-1 swing count (each swing, hit or miss),
        // the ammo the write-back subtracts (§34.6, coab≠binary #16).
        let mut swings_attack1: i32 = 0;

        let start = self.fighters[actor].attack_idx;
        for attack_idx in (1..=start).rev() {
            loop {
                let left = if attack_idx == 1 {
                    self.fighters[actor].attack1_left
                } else {
                    self.fighters[actor].attack2_left
                };
                if left == 0 || target_gone {
                    break;
                }
                if attack_idx == 1 {
                    self.fighters[actor].attack1_left -= 1;
                    swings_attack1 += 1; // bytes_1D900[1] += 1
                } else {
                    self.fighters[actor].attack2_left -= 1;
                }
                self.fighters[actor].attack_idx = attack_idx;

                // §39.5 site 6/10: `PC_CanHitTarget` (`sub_64245`) opens with
                // `remove_invisibility(attacker)` (coab ovr024.cs:519) then rolls
                // the d20; if `roll > 1` it runs `CheckAffectsEffect(attacker,
                // Type_10)` (`mov al,0Ah` @`ovr024:1283`) and `(target, Type_16)`
                // (`mov al,10h` @`ovr024:1290`). Our `pc_can_hit_target` is the
                // pure d20; host those affect ops around it, per swing. Draw-free.
                self.remove_invisibility(actor);
                let th = pc_can_hit_target(rng, target_ac, hit_bonus, 0); // one d20
                if th.d20 > 1 {
                    self.check_affects_effect(actor, CheckType::Type10);
                    self.check_affects_effect(target, CheckType::Type16);
                }
                if th.hit {
                    // `sub_3E192(idx)` damage cells (§34.6): idx 1 = @0x19E/0x1A0/
                    // 0x1A2 (our decoded profile-1), idx 2 = @0x19F/0x1A1/0x1A3
                    // (`attack2_dice`, all zero in this party).
                    let (dc, ds, db) = if attack_idx == 1 {
                        (
                            self.fighters[actor].dice_count,
                            self.fighters[actor].dice_size,
                            self.fighters[actor].damage_bonus,
                        )
                    } else {
                        self.fighters[actor].attack2_dice
                    };
                    // sub_3E192 @ovr014.cs:94-96: on a backstab, `damage *=
                    // ((SkillLevel(Thief)−1)/4)+2`. CanBackStabTarget's inputs
                    // (facing/AttacksReceived/direction) are stable across the
                    // swing loop, so the AC-time result carries.
                    let backstab = if can_backstab {
                        Some(backstab_multiplier(self.fighters[actor].thief_skill_level))
                    } else {
                        None
                    };
                    let dmg = roll_damage(rng, ds, dc, db, backstab);
                    // §39.5 site 6: the tail of `sub_3E192` (the damage function),
                    // after the roll/backstab and `damage_flags = 0`, before the
                    // caller applies it: `CheckAffectsEffect(attacker,
                    // SpecialAttacks)` (`mov al,4` @`ovr014:023A`, coab
                    // ovr014.cs:100) then `(target, Type_5)` (`mov al,5`
                    // @`ovr014:0248`, coab :101). Fires only on a hit (sub_3E192
                    // runs only then). Draw-free.
                    self.check_affects_effect(actor, CheckType::SpecialAttacks);
                    self.check_affects_effect(target, CheckType::Type5);
                    self.apply_damage(target, dmg.amount);
                    if !self.fighters[target].in_combat {
                        target_gone = true;
                    }
                }
            }
        }

        // Ammo write-back (`sub_3F9DB` @`ovr014:1BB3-1BC7`, coab≠binary #16 —
        // the binary SUBTRACTS `byte_1D901`, coab assigns): `if (item.count > 0)
        // item.count -= swings_attack1`. The decremented count is the FOUND
        // item's own (@item+0x39): a launcher's arrows/quarrels, or a
        // self-launching weapon's own count (our single `ammo` cell serves
        // both). A null item (sling / opportunity attack) skips it.
        if matches!(ranged_item, AttackItemRef::Ammo | AttackItemRef::SelfWeapon) {
            if self.fighters[actor].ammo > 0 {
                self.fighters[actor].ammo -= swings_attack1;
            }
            // Depletion (`:1BC7-`): count hits 0 → the item is lost. For plain
            // ammo (arrows/quarrels) that is a straight `lose_item` — modeled
            // (capture-proven by TRAVIS's quiver). For a SELF-LAUNCHING weapon
            // the lost item IS the primary (`field_151` nulls at once; ours
            // keeps the ready flag but the found-gates treat it as lost, so it
            // degrades exactly like arrows), and a ranged-melee one
            // additionally clone-drops an unreadied copy (`ovr014:1BD4-1C54`) —
            // that drop is unmodeled: the tripwire names the territory. Gated
            // on the edge so the trip fires ONCE at depletion, not on every
            // later re-observation of the already-lost item.
            if self.fighters[actor].ammo <= 0 && !self.fighters[actor].ammo_item_lost {
                self.fighters[actor].ammo = 0;
                self.fighters[actor].ammo_item_lost = true;
                if ranged_item == AttackItemRef::SelfWeapon {
                    self.emit(ActionEvent::StubTripped {
                        combatant_id: actor,
                        stub: "self-weapon-depleted",
                    });
                }
            }
        }

        let complete =
            self.fighters[actor].attack1_left == 0 && self.fighters[actor].attack2_left == 0;
        if complete || !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return true;
        }
        false
    }

    /// `RangedDefenseBonus(target, attacker)` (`sub_3FCED` @`ovr014:1CED`, coab
    /// `ovr014.cs:1012`; doc §34.6): a to-hit AC penalty that grows with distance
    /// for a ranged attacker. `oneThird = (table[type].range − 1) / 3`; the
    /// current `getTargetRange` climbs two bands — `> oneThird` adds +2, again
    /// adds +3 (LongBow: +2 beyond 7, +5 beyond 14). `0` for a non-ranged
    /// attacker (the `else` return). Draw-free.
    pub(super) fn ranged_defense_bonus(&self, attacker: usize, target: usize) -> i32 {
        if !self.is_weapon_ranged(attacker) {
            return 0;
        }
        let one_third = (self.primary_item(attacker).expect("ranged ⇒ item").range as i32 - 1) / 3;
        let mut range = get_target_range(
            &self.map,
            self.fighters[target].pos,
            self.fighters[attacker].pos,
        ) as i32;
        let mut adj = 0;
        if range > one_third {
            range -= one_third;
            adj += 2;
        }
        if range > one_third {
            adj += 3;
        }
        adj
    }

    /// `RecalcAttacksReceived` (`sub_3F94D` @`ovr014:194D-19D8`, coab
    /// ovr014.cs:887-901) — bump the target's received-attack counter and accumulate
    /// its facing-swing count. Draw-free. Called immediately before `AttackTarget`
    /// on every attack path (AI turn, guard into-reach, sweep per-target).
    ///
    /// `AttacksReceived++` (@`195B`); then `dirDiff = ((getTargetDirection(attacker,
    /// target) − direction) + 8) % 8` (@`1987-1993`) — the bearing from the target
    /// toward its attacker minus the target's current facing — folded `> 4 → 8 −
    /// dirDiff` (@`1996-19A8`, `jbe 4` keeps ≤4 unchanged); then `directionChanges =
    /// (directionChanges + dirDiff) % 8` (@`19C0-19D1`). Mod 8, so values only ever
    /// 0..7 and the accumulator wraps.
    pub(super) fn recalc_attacks_received(&mut self, target: usize, attacker: usize) {
        self.fighters[target].attacks_received =
            self.fighters[target].attacks_received.saturating_add(1);
        // getTargetDirection(attacker, target) = target_direction(target, attacker)
        // = bearing from the target toward its attacker (§36.2).
        let bearing = target_direction(self.fighters[target].pos, self.fighters[attacker].pos);
        let mut dir_diff =
            (bearing as i32 - self.fighters[target].direction as i32 + 8).rem_euclid(8);
        if dir_diff > 4 {
            dir_diff = 8 - dir_diff;
        }
        self.fighters[target].direction_changes =
            ((self.fighters[target].direction_changes as i32 + dir_diff) % 8) as u8;
    }

    /// `TrySweepAttack` (`ovr014.cs:530`): a melee sweep vs. `HitDice == 0` targets.
    /// **Draw-free and returns `false` for a normal (`hit_dice > 0`) target** — the
    /// only case this slice's fights use. The 0-HD sweep (extra swings per victim)
    /// is deferred with 0-HD monsters flagged.
    pub(super) fn try_sweep_attack(&mut self, target: usize, actor: usize) -> bool {
        // Guard `target.HitDice == 0` fails for hit_dice > 0 → no sweep, no draws.
        // Tripwire: a 0-HD target means the binary WOULD enter the sweep path
        // (extra swings + their draws) that this stub skips (M5).
        if self.fighters[target].hit_dice == 0 {
            self.emit(ActionEvent::StubTripped {
                combatant_id: actor,
                stub: "0-hd-sweep",
            });
        }
        false
    }

    /// `sub_3E748(direction, actor)` (`ovr014.cs:252`): step one tile, deduct the
    /// move cost, repaint occupancy, then run opportunity attacks by *guarding*
    /// enemies at the new cell (`move_step_into_attack`). The position updates
    /// unconditionally (coab), but `CanMove` already guaranteed the cost is
    /// affordable.
    pub(super) fn sub_3e748(&mut self, rng: &mut EngineRng, actor: usize, direction: u8) {
        let old = self.fighters[actor].pos;
        let new = old.stepped(direction);
        if !new.in_bounds() {
            return;
        }
        let base = self.map.move_cost(new) as i32;
        let cost = if direction & 1 != 0 {
            base * 3
        } else {
            base * 2
        };
        if cost > self.fighters[actor].move_left {
            self.fighters[actor].move_left = 0;
        } else {
            self.fighters[actor].move_left -= cost;
        }
        // Site 7 (movement step) — sub_3E748's camera (`ovr014.cs:285-310`). In
        // QuickFight (radius 3): if the destination is off-screen and focus is
        // on, first scroll to the OLD cell (`redrawCombatArea(8, 2, oldPos)`,
        // @294) using the pre-move window; then, after the pos write, scroll to
        // the NEW cell (`redrawCombatArea(8, 3, newPos)`, @309) if focus.
        if !self.on_screen_pos(new) && self.focus {
            self.redraw_combat_area(8, 2, old);
        }
        self.fighters[actor].pos = new;
        self.rebuild_occupancy();
        if self.focus {
            self.redraw_combat_area(8, 3, new);
        }
        self.emit(ActionEvent::Move {
            combatant_id: actor,
            from_x: old.x,
            from_y: old.y,
            to_x: new.x,
            to_y: new.y,
            cost,
        });
        // sub_3E748 @`ovr014:0902-090F`: the mover's own swarm state zeroes after
        // the pos write — `AttacksReceived = 0` (@`0902`) and `directionChanges = 0`
        // (@`090F`). Swarm/facing bookkeeping is per-position.
        self.fighters[actor].attacks_received = 0;
        self.fighters[actor].direction_changes = 0;
        self.move_step_into_attack(rng, actor);
        if !self.fighters[actor].in_combat {
            self.fighters[actor].move_left = 0;
        }
    }

    /// `move_step_into_attack(mover)` (`ovr014.cs:226`): every adjacent enemy that
    /// is **guarding** attacks the mover entering its reach (`AttackTarget(null,0)`).
    /// In a fresh melee no one guards, so this is draw-free; it becomes draw-bearing
    /// only once a combatant has fallen back to guard.
    fn move_step_into_attack(&mut self, rng: &mut EngineRng, mover: usize) {
        if !self.fighters[mover].in_combat {
            return;
        }
        let near = self.build_near(mover, 1, false);
        for n in near {
            let att = n.idx;
            if self.fighters[att].guarding {
                // Site 7 (guard fire) — `move_step_into_attack` scrolls to the
                // entering mover before the swing: `redrawCombatArea(8, 2,
                // target.pos)` (`ovr014.cs:239`).
                let mp = self.fighters[mover].pos;
                self.redraw_combat_area(8, 2, mp);
                self.fighters[att].guarding = false;
                self.recalc_attacks_received(mover, att);
                // AttackTarget(null,0) — the guard's into-reach swing carries no
                // ranged item (ovr014.cs:245).
                self.attack_target(rng, att, mover, false, AttackItemRef::None);
            }
        }
    }

    /// `move_step_away_attack(direction, mover)` (`ovr014.cs:326`): every enemy the
    /// mover **leaves** melee adjacency with (adjacent now, not adjacent at the
    /// destination) gets a free `AttackTarget(null,1)`. In a clean open-ground
    /// approach the mover isn't adjacent to anyone, so this is draw-free; it fires
    /// once melee is joined and a combatant steps out.
    pub(super) fn move_step_away_attack(
        &mut self,
        rng: &mut EngineRng,
        mover: usize,
        direction: u8,
    ) {
        let origin = self.build_near(mover, 1, false);
        if origin.is_empty() {
            return;
        }
        // Peek the destination's adjacent enemies (move, measure, move back).
        let orig_pos = self.fighters[mover].pos;
        self.fighters[mover].pos = orig_pos.stepped(direction);
        self.rebuild_occupancy();
        let dest = self.build_near(mover, 1, false);
        self.fighters[mover].pos = orig_pos;
        self.rebuild_occupancy();
        if !self.fighters[mover].in_combat {
            return;
        }
        let dest_ids: std::collections::HashSet<usize> = dest.iter().map(|n| n.idx).collect();
        let departed: Vec<usize> = origin
            .iter()
            .map(|n| n.idx)
            .filter(|i| !dest_ids.contains(i))
            .collect();
        for att in departed {
            // `sub_3E954` re-tests the MOVER's `in_combat` at the top of every
            // candidate iteration (`ovr014:0AD2-0ADD`): dead → `jmp loc_3ECEF`,
            // the loop continuation, skipping the swing AND the focus set. A
            // mover dropped by an earlier departure swing therefore takes no
            // further swings. Every remaining iteration skips identically
            // (nothing revives mid-loop), so `break` is draw- and
            // state-equivalent to the binary's skip-to-end scan.
            if !self.fighters[mover].in_combat {
                break;
            }
            // Site 7 (departure attack) — `sub_3E954` @`ovr014:0AE0-0AE5` sets
            // `byte_1D90F = 1` and `byte_1D910 = 1` (`focusCombatAreaOnPlayer`)
            // at the TOP of each candidate iteration: after the loop's re-test of
            // the MOVER's `in_combat` (@`0AD2-0ADD`, above), but BEFORE the
            // candidate is even fetched (@`0AF5-0B0B`) and before every per-candidate filter
            // (`sub_66BDB` @`0B14`, `sub_3F143` @`0B2D`, the two `find_affect`s).
            // So a candidate that is later skipped STILL leaves focus on — which
            // is why this is not folded into the `continue` below. The camera is
            // then live for the step that follows (`sub_3E748`'s focus-gated
            // scrolls) even for an off-screen monster mover.
            self.focus = true;
            if !self.fighters[att].in_combat || !self.can_see_target(mover) {
                continue;
            }
            // The tmpDir visibility scan (ovr014.cs:374-380): an attacker that
            // hasn't acted (delay>0) or hasn't been attacked qualifies immediately.
            let base = self.fighters[att].direction as i32 + 6;
            let qualifies = (base..=base + 4).any(|tmp| {
                self.fighters[att].delay > 0
                    || self.fighters[att].attacks_received == 0
                    || can_see_combatant(
                        (tmp % 8) as u8,
                        self.fighters[mover].pos,
                        self.fighters[att].pos,
                    )
            });
            if qualifies {
                let idx = if self.fighters[att].attack1_left > 0 {
                    1
                } else if self.fighters[att].attack2_left > 0 {
                    2
                } else {
                    1
                };
                self.fighters[att].attack_idx = idx;
                if idx == 1 && self.fighters[att].attack1_left == 0 {
                    self.fighters[att].attack1_left = 1;
                } else if idx == 2 && self.fighters[att].attack2_left == 0 {
                    self.fighters[att].attack2_left = 1;
                }
                // AttackTarget(null, 1, mover, att) — ovr014.cs:407: the
                // departure swing is ALWAYS a BehindAttack (the mover has
                // turned its back), so it hits `ac_behind`@0x19B. This is the
                // draw-2707 layer: same d20, rear AC — the bar-rout fleer is
                // hit where front-AC math missed.
                //
                // §31 bug #14: the departure attack does NOT retarget the
                // attacker — `sub_3E954` saves `actions.target` before the
                // `AttackTarget` call (`ovr014:0C83-0C8E`) and restores it
                // after (`:0CB3-0CC5`; coab's `backupTarget`, ovr014.cs:405/
                // 410), so `attack_target`'s §19 write-back is transient
                // here. Without the restore the attacker permanently switches
                // to the fleer it punished, and its held target silently
                // diverges for the rest of the fight.
                let backup_target = self.fighters[att].target;
                // AttackTarget(null,1) — the departure opportunity attack carries
                // no ranged item (ovr014.cs:407).
                self.attack_target(rng, att, mover, true, AttackItemRef::None);
                self.fighters[att].target = backup_target;
            }
        }
    }

    // === the ranged predicates + weapon table (M5 armed slice, doc §34.2/34.3) ===

    /// `is_weapon_ranged` (`offset_above_1` @`ovr025:2FE4`, coab `ovr025.cs:1578`):
    /// the readied primary weapon (`field_151`) is non-null AND its table range
    /// is `> 1` (`jbe` → false on `<= 1`). Without a loadout / item table a
    /// combatant is never ranged — today's melee behaviour.
    pub(super) fn is_weapon_ranged(&self, actor: usize) -> bool {
        let f = &self.fighters[actor];
        match (f.weapon_readied, f.loadout, self.item_data.as_ref()) {
            (true, Some(l), Some(items)) => items.get(l.primary_type).range as i32 > 1,
            _ => false,
        }
    }

    /// The ranged-melee FLAG test for a candidate weapon TYPE (readied or not):
    /// its table flags carry both `flag_10 | melee` — a thrown weapon also
    /// usable in hand (HandAxe 0x14 yes; Dart 0x1A no). The type-level half of
    /// `offset_equals_20`, shared by [`Self::is_weapon_ranged_melee`] (the
    /// readied-actor predicate) and `ai_items_selection` (which evaluates the
    /// candidate before it is readied).
    fn candidate_ranged_melee(&self, item_type: u8) -> bool {
        const RANGED_MELEE: u8 =
            gbx_formats::items::flags::FLAG_10 | gbx_formats::items::flags::MELEE;
        match self.item_data.as_ref() {
            Some(items) => (items.get(item_type).flags & RANGED_MELEE) == RANGED_MELEE,
            None => false,
        }
    }

    /// `is_weapon_ranged_melee` (`offset_equals_20` @`ovr025:3027`, coab
    /// `ovr025.cs:1570`): [`is_weapon_ranged`] AND [`Self::candidate_ranged_melee`]
    /// on the readied primary. None of armed-bar's bows qualify.
    pub(super) fn is_weapon_ranged_melee(&self, actor: usize) -> bool {
        if !self.is_weapon_ranged(actor) {
            return false;
        }
        let l = self.fighters[actor].loadout.expect("ranged ⇒ loadout");
        self.candidate_ranged_melee(l.primary_type)
    }

    /// The readied primary weapon's [`gbx_formats::items::ItemData`], or `None`
    /// when no loadout weapon is readied. A convenience over the `(loadout,
    /// item_data)` pair the predicates share.
    fn primary_item(&self, actor: usize) -> Option<gbx_formats::items::ItemData> {
        let f = &self.fighters[actor];
        match (f.weapon_readied, f.loadout, self.item_data.as_ref()) {
            (true, Some(l), Some(items)) => Some(items.get(l.primary_type)),
            _ => None,
        }
    }

    /// `GetCurrentAttackItem(out item, player)` (`sub_6906C` @`ovr025:306C`, coab
    /// `ovr025.cs:1590`): from the readied primary's flags, resolve which item
    /// the attack draws (arrows/quarrels slot for a launcher `flag_08`, the
    /// weapon itself for a self-launcher `flag_10`), and whether one was
    /// "found" (`item != null` OR `flags == flag_08|flag_02` == 0x0A — a
    /// Sling/StaffSling finds a null item and still shoots, no ammo consumed).
    pub(super) fn get_current_attack_item(&self, actor: usize) -> CurrentAttackItem {
        let Some(item) = self.primary_item(actor) else {
            // primaryWeapon == null → item stays null, flags None → not found.
            return CurrentAttackItem {
                found: false,
                item: AttackItemRef::None,
            };
        };
        let flags = item.flags;
        let f = &self.fighters[actor];
        let mut found_item = AttackItemRef::None;
        // A depleted self-launching weapon is LOST in the binary (`field_151`
        // nulls at depletion), so it no longer finds itself — same degradation
        // as a depleted ammo slot.
        if flags & gbx_formats::items::flags::FLAG_10 != 0 && !f.ammo_item_lost {
            found_item = AttackItemRef::SelfWeapon;
        }
        if flags & gbx_formats::items::flags::FLAG_08 != 0 {
            // The arrows / quarrels ammo slot — null once depleted (`lose_item`).
            let ammo_slot = if f.ammo_item_lost {
                AttackItemRef::None
            } else {
                AttackItemRef::Ammo
            };
            if flags & gbx_formats::items::flags::ARROWS != 0 {
                found_item = ammo_slot;
            }
            if flags & gbx_formats::items::flags::QUARRELS != 0 {
                found_item = ammo_slot;
            }
        }
        // item_found = (found_item != null) || flags == (flag_08 | flag_02).
        let found = !matches!(found_item, AttackItemRef::None)
            || flags == (gbx_formats::items::flags::FLAG_08 | gbx_formats::items::flags::FLAG_02);
        CurrentAttackItem {
            found,
            item: found_item,
        }
    }

    /// The ammo `count` of the `GetCurrentAttackItem` result (item+0x39), or
    /// `None` when the item is null (a Sling's found-but-null item — no ammo
    /// cap). A launcher counts the combatant's `ammo`; a self-launching weapon's
    /// own count is unmodeled (armed-bar has none) and treated as `ammo`.
    pub(super) fn attack_item_count(&self, actor: usize, item: &CurrentAttackItem) -> Option<i32> {
        match item.item {
            AttackItemRef::None => None,
            AttackItemRef::Ammo | AttackItemRef::SelfWeapon => Some(self.fighters[actor].ammo),
        }
    }

    /// The AI turn's attack range (`ovr010.cs:562-572`, doc §34.4): `range =
    /// table[primary.type].range - 1` when a primary weapon is readied
    /// (`field_151` non-null), else 1; sanitize to 1. The binary sanitizes the
    /// BYTE values `{0, 0xFF}` — table range 1 and table range 0 (whose `0 − 1`
    /// wraps to `0xFF`) — which in i32 space are exactly `r == 0` and `r == -1`;
    /// an i32 `r == 0xFF` arm would instead catch table range 255, which the
    /// binary leaves at 254 (review finding #4). LongBow (22) → 21, ShortBow
    /// (16) → 15.
    pub(super) fn weapon_range(&self, actor: usize) -> i32 {
        match self.primary_item(actor) {
            Some(it) => {
                let r = it.range as i32 - 1;
                if r == 0 || r == -1 {
                    1
                } else {
                    r
                }
            }
            None => 1,
        }
    }

    /// `reclac_attacks(player)` (`sub_3EDD4` @`ovr014:0DD4`, coab `ovr014.cs:462`;
    /// doc §34.3). Sets `attack1_left` for the round: `attacksCount` half-actions
    /// for melee, or — with a readied ranged weapon whose ammo is found —
    /// `max(2, table[type].numberAttacks)` (LongBow 4 → 2 shots/round), capped by
    /// remaining ammo. The write-back is gated so a mid-turn recompute cannot
    /// inflate the count. Draw-free; called by `CalculateInitiative` and the
    /// cornered weapon-selection AI.
    pub(super) fn reclac_attacks(&mut self, actor: usize) {
        let orig = self.fighters[actor].attack1_left as i32;
        // rec[0x19C] = rec[0x11C] (attack1_left := attacksCount).
        self.fighters[actor].attack1_left = self.fighters[actor].attacks_count;

        let ranged = self.is_weapon_ranged(actor);
        let item = self.get_current_attack_item(actor);
        let found_ranged = ranged && item.found;

        let half = if found_ranged {
            let natk = self
                .primary_item(actor)
                .map(|it| it.number_attacks as i32)
                .unwrap_or(0);
            natk.max(2)
        } else {
            self.fighters[actor].attack1_left as i32
        };

        // §39.5 site 4: `CheckAffectsEffect(Movement)` inside `reclac_attacks`,
        // after `halfActionsLeft` is set and before `ThisRoundActionCount` —
        // `mov al,12h; call work_on_00` @`ovr014:0E66` (coab ovr014.cs:488). The
        // haste/slow/clear_movement effects it reads are the spell slice's;
        // draw-free (empty lists).
        self.check_affects_effect(actor, CheckType::Movement);

        let mut attacks = this_round_action_count(half, self.combat_round);

        // Ammo cap (only for a found ranged item that is non-null — a Sling's
        // null item is skipped): cap = max(1, count); if cap < attacks &&
        // count > 0 → attacks = cap.
        if found_ranged {
            if let Some(count) = self.attack_item_count(actor, &item) {
                let cap = count.max(1);
                if cap < attacks && count > 0 {
                    attacks = cap;
                }
            }
        }

        // Write-back gate (`ovr014:0EBE-0EFC`, coab `ovr014.cs:508`): !field_8
        // || attacks < orig || (field_8 && attacks < orig*2 && !foundRanged).
        // The third clause tests `var_5` = **foundRanged** (`:0EF6`, ranged AND
        // the attack item found), not mere is_weapon_ranged — the two differ for
        // a readied launcher whose ammo item is gone (audit fix).
        let field_8 = self.fighters[actor].field_8;
        if !field_8 || attacks < orig || (field_8 && attacks < orig * 2 && !found_ranged) {
            self.fighters[actor].attack1_left = attacks as u8;
        }
    }

    /// `CalcItemPowerRating(item, player)` (`sub_36535` @`ovr010:1535`, coab
    /// `ovr010.cs:817`; doc §34.5) for the loadout's primary weapon type:
    /// `rating = dsN*dcN + item.plus*8(if>0) + bonusN*2(if>0) +
    /// (flag_08 ? (natk−1)*2 : 0) + (hands ≤ 1 ? 3 : 0)`. The loadout carries no
    /// magic plus (mundane weapons → `plus = 0`); the cursed / affect / hands+used
    /// zeroing branches are cited-deferred (a single non-cursed weapon). LongBow:
    /// `6 + 6 = 12`.
    fn calc_item_power_rating(&self, item_type: u8) -> i32 {
        let it = self
            .item_data
            .as_ref()
            .expect("rating ⇒ items")
            .get(item_type);
        let mut rating = it.dice_size_normal as i32 * it.dice_count_normal as i32;
        // item.plus not modeled (mundane loadout weapons) → the +plus*8 term is 0.
        if it.bonus_normal > 0 {
            rating += it.bonus_normal as i32 * 2;
        }
        if it.flags & gbx_formats::items::flags::FLAG_08 != 0 {
            rating += (it.number_attacks as i32 - 1) * 2;
        }
        if it.hands_count <= 1 {
            rating += 3;
        }
        rating
    }

    /// Whether the loadout's primary weapon would "find" an attack item — the
    /// `var_1F` ammo-availability test in `AI_items_selection` (coab
    /// `ovr010.cs:943-970`): the ammo slot present (not depleted) for a launcher,
    /// the weapon itself for a self-launcher, or the `flag_08|flag_02` (0x0A)
    /// sling special. Evaluated for the CANDIDATE weapon regardless of whether it
    /// is currently readied (unlike [`Self::get_current_attack_item`]).
    fn candidate_attack_found(&self, actor: usize, item_type: u8) -> bool {
        let Some(items) = self.item_data.as_ref() else {
            return false;
        };
        let flags = items.get(item_type).flags;
        let mut found = false;
        // Mirrors `get_current_attack_item`: a depleted self-launcher is a
        // lost item in the binary and cannot be re-found by the selection scan.
        if flags & gbx_formats::items::flags::FLAG_10 != 0 {
            found = !self.fighters[actor].ammo_item_lost;
        }
        if flags & gbx_formats::items::flags::FLAG_08 != 0
            && flags & (gbx_formats::items::flags::ARROWS | gbx_formats::items::flags::QUARRELS)
                != 0
        {
            found = !self.fighters[actor].ammo_item_lost;
        }
        found || flags == (gbx_formats::items::flags::FLAG_08 | gbx_formats::items::flags::FLAG_02)
    }

    /// `AI_items_selection(player)` (`sub_36673` @`ovr010:1673`, coab
    /// `ovr010.cs:875`; doc §34.5) — the cornered weapon swap, faithful over the
    /// loadout's single weapon (the secondary/shield/multi-item branches are
    /// cited-deferred, tripwired). The primary candidate `var_4` = the loadout
    /// bow (`rating = var_15`); the melee candidate `var_8` = bare hands here
    /// (`None`). The bow wins iff `rating > (var_16 >> 1)` (`var_16` = the base
    /// profile rating) AND ammo is available AND (ranged-melee OR no adjacent
    /// enemy). Otherwise bare hands. The observable swap (§34.5): unready → the
    /// attack-1 profile becomes the unarmed profile; re-ready → the saved entry
    /// profile; attacks recomputed via [`Self::reclac_attacks`] both ways.
    /// Inert without a loadout (weapon-only no-op). Draw-free.
    pub(super) fn ai_items_selection(&mut self, actor: usize) {
        let Some(l) = self.fighters[actor].loadout else {
            return; // no loadout → nothing to select (today's melee no-op).
        };
        if self.item_data.is_none() {
            return;
        }
        // var_15 = CalcItemPowerRating(bow); var_16 = the base profile rating
        // (dsB*dcB (+2*bonusB if >0)).
        let var_15 = self.calc_item_power_rating(l.primary_type);
        let (dcb, dsb, dbb) = self.fighters[actor].base_dice;
        let mut var_16 = dsb as i32 * dcb as i32;
        if dbb as i32 > 0 {
            var_16 += dbb as i32 * 2;
        }
        // var_1F = the bow's ammo is available.
        let ammo_avail = self.candidate_attack_found(actor, l.primary_type);
        // ranged_melee(var_4) — a thrown weapon usable in hand (the candidate
        // may be unreadied, so this is the type-level test, not the actor one).
        let ranged_of_bow = self.item_data.as_ref().unwrap().get(l.primary_type).range as i32 > 1;
        let ranged_melee = ranged_of_bow && self.candidate_ranged_melee(l.primary_type);
        let no_adjacent = self.build_near(actor, 1, false).is_empty();

        // The bow wins the primary slot iff rating dominates the base, ammo is
        // available, and (ranged-melee or no adjacent enemy).
        let use_bow = var_15 > (var_16 >> 1) && ammo_avail && (ranged_melee || no_adjacent);

        let currently_readied = self.fighters[actor].weapon_readied;
        if use_bow && !currently_readied {
            // Re-ready the bow: primaryWeapon := bow, attack-1 profile := the
            // saved entry profile.
            self.fighters[actor].weapon_readied = true;
            let (dc, ds, db) = self.fighters[actor].entry_dice;
            self.fighters[actor].dice_count = dc;
            self.fighters[actor].dice_size = ds;
            self.fighters[actor].damage_bonus = db;
        } else if !use_bow && currently_readied {
            // Unready the bow: primaryWeapon := null, attack-1 profile := the
            // bare-hands profile.
            self.fighters[actor].weapon_readied = false;
            let (dc, ds, db) = l.unarmed_profile;
            self.fighters[actor].dice_count = dc;
            self.fighters[actor].dice_size = ds;
            self.fighters[actor].damage_bonus = db;
        }
        // The tail (`ovr010:1AB0-1AC6`, coab ovr010.cs:1018-1020) runs
        // `reclac_player_values` + `reclac_attacks` UNCONDITIONALLY — both the
        // replace path and the `replace_weapon = false` skip land on the same
        // merge point (audit fix: the recompute is not gated on a swap; the
        // §34.3 write-back gate is what makes the always-recompute safe).
        self.reclac_attacks(actor);
    }
}
