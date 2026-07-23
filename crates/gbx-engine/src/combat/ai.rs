use super::*;

// ===========================================================================
// The melee QuickFight AI (M4 combat #4; study §4.1, D-OR5(a) Phase 1)
// ===========================================================================
//
// **In progress — deliverable 3.** This section transliterates the draw-bearing
// pieces of `PlayerQuickFight` (`ovr010.cs:8`) in draw order per the §4.1 map. The
// `field_15` mode-gate lands first — the turn's *first* draw site and the study's
// #1 landmine (the `||` short-circuit). The behavior-guard d7s, `find_target`
// picks, and the `sub_35DB1` move-attack loop (with the per-step monster d100 and
// the opportunity attacks) are the remaining pieces; see the handoff. Every draw
// flows through the one `EngineRng`/`roll_dice` seam (D9).

/// The QuickFight `field_15` **target-mode gate** (`sub_3504B` @`ovr010:0090`;
/// study §4.1.2, corrected by the §15 binary RE) — the very first draws of a
/// melee AI turn, run before any target selection. Given the combatant's
/// persistent `field_15` (`Action@0x15`, which `CalculateInitiative` does **not**
/// reset), returns its new value and draws faithfully.
///
/// ```text
/// if (field_15 == 0 || field_15 > 4 || roll_dice(4,1) == 1) {   // d4 GATE (short-circuits)
///     v = roll_dice(8,1);                                       // d8
///     v = (v != 8) ? roll_dice(4,1) : roll_dice(2,1) + 4;       // d4 (→1..4) or d2+4 (→5..6)
/// }
/// ```
///
/// **§15 binary correction (bug #1).** This supersedes combat #4 D1's coab-derived
/// reading, which was wrong two ways against the binary at `ovr010:0090`:
/// - the entry short-circuit is `field_15 > 4` (`cmp 4; ja loc_350AB`), **not**
///   `== 4`; and
/// - the `d8` body branches are **swapped** — `d8 != 8` draws `roll_dice(4,1)`
///   (`loc_350D4` → 1..4) and `d8 == 8` draws `roll_dice(2,1)+4` (→ 5..6). coab/our
///   old code had these reversed, drawing a `d2` in the common `d8 != 8` case.
///
/// **The `||` short-circuit is still the landmine (D9):** when `field_15` is 0 or
/// `> 4` the `roll_dice(4,1)` gate is **not evaluated** — that turn draws only the
/// body's **2** dice (d8 then d4|d2), not 3. Since `field_15` starts at 0, *every
/// combatant's first turn* takes this 2-draw path. When `field_15 ∈ 1..=4`: one d4
/// gate draw always; then the 2-draw body only if the gate rolled 1 (so 1 or 3
/// draws). The result is always in `1..=6`.
pub fn field_15_mode_gate(rng: &mut EngineRng, field_15: u8) -> u8 {
    let mut v = field_15 as u16;
    // ovr010:0090 — `cmp 0; jz body` / `cmp 4; ja body` then the d4 gate. The `||`
    // short-circuits, so roll_dice(4,1) is skipped for field_15 ∈ {0} ∪ {>4}.
    let enter = v == 0 || v > 4 || roll_dice(rng, 4, 1) == 1;
    if enter {
        v = roll_dice(rng, 8, 1); // ovr010:00AB — d8
        if v != 8 {
            v = roll_dice(rng, 4, 1); // ovr010:00D4 — d8!=8 → 1..4
        } else {
            v = roll_dice(rng, 2, 1) + 4; // ovr010:00BF — d8==8 → 5..6
        }
    }
    v as u8
}

/// `data_2B8` (`seg600:02BD`) — the approach-angle table. Each entry is an
/// iso-direction *offset* added to the heading toward the target, so `field_15`
/// selects an "approach personality" (straight vs. weaving) and `dirStep` (1..=6)
/// is the retry index `CanMove`/`moralFailureEscape` walk. Value 8 = "no
/// direction". 11 rows materialized from coab's 6-wide windows.
///
/// **§15 binary correction (bug #2).** The binary (`CanMove`/`sub_3573B`
/// @`ovr010:076D`) indexes the *flat* table as `byte[0x2B8 + 5·field_15 + dirStep]`
/// = `T[5·(field_15−1) + dirStep]` (base `0x2BD`) — a **stride-5 sliding window**.
/// coab materialized the overlapping windows into these 6-wide rows and indexed
/// row `field_15`, an off-by-one: coab row `R` is `T[5R+1 ..= 5R+6]`, so binary
/// `field_15 = N` reads coab **row N−1**. Both call sites therefore index
/// [`DATA_2B8`]`[field_15 − 1]` (post-gate `field_15` is always 1..=6). Verified
/// `DATA_2B8[N−1][dirStep−1] == T[5·(N−1)+dirStep]` for `dirStep` 1..=6.
const DATA_2B8: [[i32; 6]; 11] = [
    [8, 7, 6, 1, 2, 8],
    [8, 1, 2, 7, 6, 7],
    [7, 1, 8, 6, 2, 1],
    [1, 7, 8, 2, 6, 8],
    [8, 7, 6, 5, 4, 8],
    [8, 1, 2, 3, 4, 8],
    [8, 4, 6, 2, 8, 6],
    [6, 4, 0, 8, 0, 6],
    [6, 2, 8, 2, 0, 4],
    [4, 0, 0, 2, 6, 2],
    [2, 2, 0, 4, 4, 4],
];

impl CombatState {
    /// `TryGuarding` (`sub_361F7` @`ovr010:11F7`, coab `ovr010.cs:685`): `IsHeld ||
    /// is_weapon_ranged || delay == 0` → `Action.Clear` (a ranged attacker NEVER
    /// parks a guard, §34.4); else `guarding()` = clear then set `guarding =
    /// true`. Either way `delay` ends 0, so it is not re-picked. `IsHeld` (a held
    /// affect) is not modeled → false. Draw-free.
    pub(super) fn try_guarding(&mut self, actor: usize) {
        if self.is_weapon_ranged(actor) || self.fighters[actor].delay == 0 {
            self.clear_actions(actor);
        } else {
            self.clear_actions(actor);
            self.fighters[actor].guarding = true;
        }
    }

    /// `RemoveFromCombat(name, status, player)` (`sub_644A7` @`ovr024:14A7`) — drop
    /// a combatant from combat with a given health status. A not-in-combat combatant
    /// is a no-op (`:14C0`). Else: display (draw-free); `in_combat = false`
    /// (`:1506`); `health_status = status` (`:1512`); and — **only when `status !=
    /// running`** (`:151A`) — `hit_point_current = 0` (`:1525`); then
    /// `CombatMap[idx].size = 0` + `sub_743E7` occupancy repaint (`:154A-154F`) and
    /// `clear_actions` (`:155A`). **No `Tile_DownPlayer` stamp** — that is
    /// `CombatantKilled` (the damage-death path) only. Draw-free.
    ///
    /// (Callers: the FleeCheck surrender branch with `Unconscious`, and
    /// [`flee_battle`]'s Got-Away removal with [`HealthStatus::Running`].)
    fn remove_from_combat(&mut self, actor: usize, status: HealthStatus) {
        if !self.fighters[actor].in_combat {
            return; // :14C0-14CB — already out of combat.
        }
        // Site 6 (flee/surrender path) — `RedrawCombatIfFocusOn(false, 3, player)`
        // (`ovr024.cs:624`, `sub_75356`): a focus-on removal scrolls the camera
        // to the leaver (radius 3) BEFORE `size = 0`.
        if self.focus {
            let p = self.fighters[actor].pos;
            self.redraw_combat_area(8, 3, p);
        }
        {
            let f = &mut self.fighters[actor];
            f.in_combat = false; // :1506
            f.health_status = status; // :1512
            if status != HealthStatus::Running {
                f.hp_current = 0; // :1525 — skipped for `running` (the Got-Away case)
            }
        }
        // :154A CombatMap[idx].size = 0 + :154F sub_743E7 occupancy repaint.
        self.rebuild_occupancy();
        // :155A clear_actions.
        self.clear_actions(actor);
        // §39.5 site 10: `RemoveFromCombat` (`sub_644A7`) ends with
        // `RemoveCombatAffects(player)` (coab ovr024.cs:645) — the strip on the
        // flee/surrender removal path. Draw-free (empty lists).
        self.remove_combat_affects(actor);
    }

    /// `FleeCheck_001` (`sub_3637F` @`ovr010:137F`, coab `ovr010.cs:760`) — the
    /// faithful morale ladder, **draw-free**. Sets `moral_failure`/`fleeing` (the
    /// flee outcome the move path acts on) and returns the surrender flag (`var_1`,
    /// the turn-ending `RemoveFromCombat("Surrenders")` path — §28 item 7, built in
    /// the next slice; here still the `surrender-int5` tripwire). Transliterated
    /// site-by-site from the IDA listing (each `ovr010:` cited); re-verified against
    /// `coab_new.lst` this session.
    pub(super) fn flee_check(&mut self, actor: usize) -> bool {
        // :1385 var_1 = 0 (the surrender return flag).
        // :1391 actions.field_14 = 0 → moral_failure = false.
        self.fighters[actor].moral_failure = false;
        // §39.5 site 10: `RemoveAttackersAffects(player)` (`sub_6460D` @:139C,
        // coab ovr010.cs:765) — strips reduce/clear_movement/affect_8b/
        // owlbear_hug_round_attack. Draw-free (empty lists).
        self.remove_attackers_affects(actor);
        // :13A9 fleeing (actions.field_10) → moral_failure = 1, "is forced to
        // flee", return false.
        if self.fighters[actor].fleeing {
            self.fighters[actor].moral_failure = true;
            return false;
        }
        // :13E3 control_morale@0xF7 > 0x7F (unsigned `ja`) else return false —
        // i.e. NPCs only (a PC short-circuits the whole block).
        if !self.fighters[actor].npc {
            return false;
        }
        // :13F1-13FC per-actor morale seed = (control_morale & 0x7F) << 1, recomputed
        // EVERY call (the deviation slice-2 replaces: the old stub used a process-
        // lifetime scratch stuck at 100). :13FF `> 0x66` (102) → 0.
        let mut morale = ((self.fighters[actor].control_morale & 0x7F) as i32) << 1;
        if morale > 0x66 {
            morale = 0;
        }
        self.monster_morale = morale;
        // §39.5 site 3: `CheckAffectsEffect(Morale)` (`mov al,11h; call work_on_00`
        // @`ovr010:1414`, coab ovr010.cs:780) — first of two, after the seed/clamp,
        // before Gate 1. Reads bless/cursed/charm_person; draw-free (empty lists).
        self.check_affects_effect(actor, CheckType::Morale);

        // Gate 1 (:143F-144D): morale < (100 − hp_cur·100/hp_max) SIGNED (`jl`)
        // OR morale == 0; else return false.
        let hp_pct = (self.fighters[actor].hp_current * 100) / self.fighters[actor].hp_max.max(1);
        if self.monster_morale < (100 - hp_pct) || self.monster_morale == 0 {
            // :1458 monster_morale = byte_1D903 (enemyHealthPercentage).
            self.monster_morale = self.enemy_health_pct;
            // §39.5 site 3: the second `CheckAffectsEffect(Morale)` (`mov al,11h`
            // @`ovr010:1467`, coab ovr010.cs:788), before Gate 2. Draw-free.
            self.check_affects_effect(actor, CheckType::Morale);

            // Gate 2 (:146C-1493): morale < (100 − area2.field_58C) — ★ bug #12:
            // UNSIGNED 16-bit `jb` at :1481 over a 16-bit `sub` at :1473, so a
            // `field_58C > 100` underflows `100 − field_58C` to ~0xFFxx and the gate
            // is ALWAYS true (coab's signed int makes it always false). Transliterate
            // as u16 wrapping subtraction + unsigned compare. — OR morale == 0 OR
            // combat_team == Party (`:148D cmp combat_team, 0`).
            let lhs = self.monster_morale as u16;
            let rhs = 100u16.wrapping_sub(self.area_field_58c as u16);
            if lhs < rhs || self.monster_morale == 0 || self.fighters[actor].team == Team::Party {
                // Speed fork (:1498-14BE): MaxOppositionMoves > CalcMoves/2 SIGNED
                // (`jg` at :14BE) → the surrender branch (loc_364F7); else (`<=`)
                // the flee fork: moral_failure = 1 (:14C8) + remove_affect(0x4A)/
                // remove_affect(0x4B) (:14DC/:14F0) — §39.5, wired below.
                let max_opp = self.max_opposition_moves(actor);
                if max_opp > calc_moves(self.fighters[actor].movement) / 2 {
                    // Surrender branch (loc_364F7, :14F7-1529, §28 item 7). The
                    // `surrender-int5` wire (kept, repurposed) fires whenever this
                    // implemented-but-capture-unproven branch executes — the rout
                    // capture never reaches it (its 12-vs-12 speed tie always takes
                    // the flee fork), so a firing marks an untested path.
                    self.emit(ActionEvent::StubTripped {
                        combatant_id: actor,
                        stub: "surrender-int5",
                    });
                    // :14FA `cmp byte es:[di+13h], 5; jbe → return false` — surrender
                    // only when `Int@0x13 > 5`.
                    if self.fighters[actor].int_score > 5 {
                        // :1501-1519 `RemoveFromCombat("Surrenders", status=4
                        // unconscious)`; :1524 clear_actions; return true (turn
                        // over — melee_ai_turn step 2 returns on it).
                        self.remove_from_combat(actor, HealthStatus::Unconscious);
                        return true;
                    }
                } else {
                    self.fighters[actor].moral_failure = true;
                    // §39.5: the flee fork's `remove_affect(affect_4a 0x4A)` (:14DC)
                    // and `remove_affect(weap_dragon_slayer 0x4B)` (:14F0). Draw-free.
                    self.remove_affect(actor, 0x4A);
                    self.remove_affect(actor, 0x4B);
                }
            }
        }
        false
    }

    /// `MaxOppositionMoves` (`ovr014.cs:1699`) — the largest half-move budget over
    /// the live opposite team. Draw-free.
    fn max_opposition_moves(&self, actor: usize) -> i32 {
        let team = self.fighters[actor].team;
        self.fighters
            .iter()
            .filter(|f| f.in_combat && f.team != team)
            .map(|f| calc_moves(f.movement) / 2)
            .max()
            .unwrap_or(0)
    }

    /// `sub_354AA` (`ovr010:04AA`) — the wand scan. The binary rolls the **d7
    /// unconditionally at proc entry** (`ovr010:04C6`: `call roll_dice(7,1)` into
    /// `var_3`) and only THEN checks `can_use` (`:04D6`), the opposite-team live
    /// count (`:04EE`, `friends_count[on_our_team(actor)]`), and
    /// `area.can_cast_spells` (`:04FC`) — those guards gate the **item scan**, not
    /// the roll. (coab ovr010.cs:188 hoisted the guard above the roll — coab ≠
    /// binary; the difference is only visible when a guard goes false, e.g. the
    /// last enemy died earlier this round.) The scan itself is draw-free for a
    /// weapon-only combatant (no readied spell-item), so this always returns
    /// `false` (no wand used). Wand *effects* are deferred (M5).
    fn wand_scan_d7(&mut self, rng: &mut EngineRng, _actor: usize) -> bool {
        let _priorities = roll_dice(rng, 7, 1); // ovr010:04C6 — before the guards
        false
    }

    /// `getGroundInformation(direction, actor)` (`ovr033.cs:433`) for a single-cell
    /// combatant: the destination cell (`pos + delta[direction]`), returning its
    /// ground-tile index (0 for void/OOB) and any *other* occupant (1-based; 0 =
    /// empty).
    fn ground_info_dir(&self, actor: usize, direction: u8) -> (i32, u16) {
        let dest = self.fighters[actor].pos.stepped(direction);
        let ground = self.map.ground_tile(dest) as i32;
        let occ = self.map.occupant(dest);
        let current = (actor + 1) as u16;
        let occ = if occ == current { 0 } else { occ };
        (ground, occ)
    }

    /// `CanMove(baseDirection, dirStep, actor)` (`ovr010.cs:295`): can the actor step
    /// in `(baseDirection + data_2B8[field_15][dirStep-1]) % 8`? Returns
    /// `(can_move, ground_clear)` where `ground_clear` is the void case. Draw-free
    /// (the cloud save at `:341` needs a poison/noxious cloud — none modeled).
    fn can_move(&self, actor: usize, base_dir: u8, dir_step: i32) -> (bool, bool) {
        let f15 = self.fighters[actor].field_15 as usize;
        // §15 bug #2: binary indexes coab row field_15−1 (stride-5 window).
        let offset = DATA_2B8[f15.saturating_sub(1)][(dir_step - 1) as usize];
        let player_dir = ((base_dir as i32 + offset) % 8) as u8;
        let (ground_tile, occ) = self.ground_info_dir(actor, player_dir);

        if ground_tile == 0 {
            return (false, true); // void → groundClear, can't move
        }
        let mc = ground_tile_move_cost(ground_tile);
        if mc == 0xFF {
            return (false, false); // wall
        }
        let cost = if player_dir & 1 != 0 {
            mc as i32 * 3
        } else {
            mc as i32 * 2
        };
        let can = occ == 0 && cost < self.fighters[actor].move_left;
        (can, false)
    }

    /// `moralFailureEscape(actor)` (`ovr010.cs:369`, `sub_359D1`) — one **approach**
    /// (or flee) step toward the target. For an **NPC** advancing, the morale gate
    /// draws **one d100** (`:387`); a **PC** short-circuits it (0 draws). Then a
    /// `CanMove` retry loop picks a step direction from [`DATA_2B8`], the mover
    /// faces it (`draw_74B3F` sets `direction`), leaving-adjacency enemies attack
    /// (`move_step_away_attack`), and the step lands (`sub_3E748`). The flee branch
    /// (`moral_failure`) draws the `:400` d2; only the non-flee approach is
    /// exercised by the parity fights.
    fn moral_failure_escape(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        b1ab18: &mut i32,
        b1ab19: &mut i32,
    ) {
        if !(self.fighters[actor].move_left / 2 > 0 && self.fighters[actor].delay > 0) {
            self.try_guarding(actor);
            return;
        }

        // The morale-advance gate (ovr010.cs:386-388). C# `||` short-circuit:
        // a PC (control<NPC_Base) makes the FIRST operand true → NO d100; an NPC
        // (control>=NPC_Base) evaluates operand C → draws the d100. `morale_roll`
        // stays 0 when no d100 is drawn.
        let mut morale_roll: u16 = 0;
        let advance = if !self.fighters[actor].npc {
            true
        } else {
            morale_roll = roll_dice(rng, 100, 1);
            self.enemy_health_pct <= morale_roll as i32 + self.monster_morale
                || self.fighters[actor].team == Team::Monster
        };
        self.emit(ActionEvent::Morale {
            combatant_id: actor,
            monster_morale: self.monster_morale,
            enemy_hp_pct: self.enemy_health_pct,
            roll: morale_roll,
            failed: self.fighters[actor].moral_failure,
        });

        if !advance {
            self.try_guarding(actor);
            return;
        }

        // §15 bug #4 — the Magic-User hold (`sub_359D1` @`ovr010:0AA3`, `loc_35AA3`,
        // the shared post-advance block both PCs and advancing NPCs reach). A
        // non-fleeing pure Magic-User (`class == 5`) with a null `field_159` does
        // **not** advance — it `jmp loc_35D9E` (guard). This is what pins PHILIPPE
        // to his corner the whole capture. The `sub_35DB1` caller then exits its
        // loop draw-free (`find_target` re-draws nothing once a target is held),
        // exactly like the binary.
        if !self.fighters[actor].moral_failure
            && self.fighters[actor].field_159_null
            && self.fighters[actor].class == 5
        {
            self.try_guarding(actor);
            return;
        }

        let dir = if !self.fighters[actor].moral_failure {
            let tp = self.fighters[self.fighters[actor].target.unwrap()].pos;
            target_direction(self.fighters[actor].pos, tp)
        } else {
            // Flee direction (ovr010.cs:400-408) — draws the d2, then a fixed
            // heading from mapDirection. Only reached when moral_failure is set.
            self.fighters[actor].field_15 = roll_dice(rng, 2, 1) as u8;
            let md = self.map_direction as i32;
            let mut d = md - (((md + 2) % 4) / 2) + 8;
            if self.fighters[actor].team == Team::Party {
                d += 4;
            }
            (d % 8) as u8
        };

        // CanMove retry loop (ovr010.cs:415-428): find the first dir_step whose
        // DATA_2B8-offset direction is walkable. flee_battle only in the flee case.
        let mut dir_step = 1i32;
        let mut var_5 = false;
        while dir_step < 6 && !var_5 {
            let (can, ground_clear) = self.can_move(actor, dir, dir_step);
            if can {
                break;
            }
            if self.fighters[actor].moral_failure && ground_clear {
                var_5 = true;
                self.flee_battle(rng, actor);
            } else {
                dir_step += 1;
            }
        }

        if var_5 {
            self.fighters[actor].move_left = 0;
            self.fighters[actor].moral_failure = false;
            self.clear_actions(actor);
            return;
        }

        let f15 = self.fighters[actor].field_15 as usize;
        // §15 bug #2: binary indexes coab row field_15−1 (stride-5 window).
        let offset = DATA_2B8[f15.saturating_sub(1)][(dir_step.min(6) - 1) as usize];
        let var_2 = (offset + dir as i32).rem_euclid(8);

        // Anti-oscillation (ovr010.cs:440-460): a 180° reversal or a failed step
        // rotates field_15 and (after 2) retargets — find_target here DRAWS.
        if dir_step == 6 || (var_2 + 4) % 8 == *b1ab18 {
            *b1ab19 += 1;
            self.fighters[actor].field_15 = (self.fighters[actor].field_15 % 6) + 1;
            if *b1ab19 > 1 {
                self.fighters[actor].target = None;
                if *b1ab19 > 2 {
                    self.fighters[actor].move_left = 0;
                    var_5 = true;
                } else if !self.find_target(rng, actor, false, 1, 0xff) {
                    var_5 = true;
                    self.try_guarding(actor);
                }
            }
        }

        if dir_step < 6 {
            *b1ab18 = var_2;
        } else {
            var_5 = true;
        }

        if var_5 {
            return;
        }

        // Site 7 (approach/flee step) — the camera follows the mover before it
        // steps (`ovr010.cs:474`): `focus = (byte_1D90E || PlayerOnScreen(mover)
        // || team == Ours)`, and `byte_1D90E` is provably false on this path
        // (reset @`ovr010:561`, only set true once a target is reached). Then
        // `draw_74B3F(false, Normal, var_2, mover)` (@476) recenters an
        // off-screen mover and sets `actions.direction = var_2` (the step
        // heading — the store our engine already carried).
        self.focus = self.on_screen(actor) || self.fighters[actor].team == Team::Party;
        self.draw_74b3f(actor, var_2 as u8);
        self.move_step_away_attack(rng, actor, var_2 as u8);
        if !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return;
        }
        if self.fighters[actor].move_left > 0 {
            self.sub_3e748(rng, actor, self.fighters[actor].direction);
        }
        // in_poison_cloud — draw-free (no cloud).
    }

    /// `flee_battle` (`ovr014.cs:426`): the escape check, drawing a `d2` tiebreak
    /// (`:443`) only when the fastest opponent exactly matches the fleer's speed.
    /// Reached only from the flee path; removes the fleer on success (**Got Away**).
    fn flee_battle(&mut self, rng: &mut EngineRng, actor: usize) {
        let gets_away = if self.build_near(actor, 0xff, false).is_empty() {
            true
        } else {
            let var_4 = calc_moves(self.fighters[actor].movement) / 2;
            let var_3 = self.max_opposition_moves(actor);
            if var_3 < var_4 {
                true
            } else {
                var_3 == var_4 && roll_dice(rng, 2, 1) == 1 // ovr014.cs:443
            }
        };
        if gets_away {
            // "Got Away" (`ovr014:0D90`): `RemoveFromCombat(..., Status.running=3,
            // ...)` — the fleer leaves with `health_status = Running`; hp is NOT
            // zeroed (the running special-case) and its footprint frees immediately
            // (`sub_743E7`, visible to every later `CanMove` this same round). No
            // downed-tile stamp.
            self.remove_from_combat(actor, HealthStatus::Running);
        }
        // `:0DBD func_end` — clear_actions unconditionally (idempotent after the
        // removal's own clear_actions on the Got-Away path).
        self.clear_actions(actor);
    }

    /// `bandage(applyBandage)` (`ovr025:335F`, coab ovr025.cs:1628) — scan the
    /// roster (`TeamList` order) for a bandageable ally: `nonTeamMember == false`
    /// AND `combat_team == Ours` AND `health_status == dying`. Returns whether any
    /// exists. When `apply_bandage`, the **first** such member is bandaged —
    /// `dying → unconscious`, `bleeding = 0` — and no further member is bandaged
    /// (one per call); the scan continues only to keep reporting `someoneBleeding`.
    ///
    /// `nonTeamMember == false && combat_team == Ours` is modeled as
    /// `team == Party` (§26 cited simplification: allied non-team NPCs on the
    /// party team are out of this slice's scope). Monsters are never bandaged.
    /// Draw-free (the "is bandaged" status string, `ovr025:33D6`, is display-only).
    fn bandage(&mut self, apply_bandage: bool) -> bool {
        let mut someone_bleeding = false;
        let mut apply = apply_bandage;
        for f in &mut self.fighters {
            if f.team == Team::Party && f.health_status == HealthStatus::Dying {
                someone_bleeding = true;
                if apply {
                    f.health_status = HealthStatus::Unconscious;
                    f.bleeding = 0;
                    apply = false; // one bandage per call (ovr025:33E5)
                }
            }
        }
        someone_bleeding
    }

    /// `sub_35DB1(actor)` (`ovr010.cs:511`) — the move-then-attack loop. Approaches
    /// the target one step per iteration (each NPC step drawing the morale d100)
    /// until adjacent, then attacks (`AttackTarget01`'s d20s + damage). Returns
    /// `delayed == false` (the turn is spent). The 20-iteration `counter` cap
    /// guarantees termination.
    fn sub_35db1(&mut self, rng: &mut EngineRng, actor: usize) -> bool {
        let mut b1ab18 = 8i32;
        let mut b1ab19 = 0i32;
        // §39.5 site 5: `CheckAffectsEffect(Type_14)` — the AI-specials check at
        // the head of `sub_35DB1` (`mov al,0Eh; call work_on_00` @`ovr010:0DDB`,
        // coab ovr010.cs:516). Draw-free (empty lists). Then the bandage turn
        // (§26.2, `ovr010:0DE3-0DFF`): a Party
        // actor whose team has a dying ally spends its whole turn bandaging —
        // `bandage(true)` zeroes `actions.delay`, so `delayed` starts false and
        // the move-attack loop below never runs (no movement, no attack, no draws
        // beyond the turn head the caller already rolled). Draw-free itself.
        self.check_affects_effect(actor, CheckType::Type14);
        if self.fighters[actor].team == Team::Party && self.bandage(true) {
            self.fighters[actor].delay = 0; // ovr010:0DFF — actions.delay = 0
        }
        let mut counter = 0;
        let mut stop = false;
        let mut delayed = self.fighters[actor].delay != 0;

        while !stop && delayed {
            if self.fighters[actor].moral_failure {
                while self.fighters[actor].move_left > 0
                    && self.fighters[actor].delay > 0
                    && self.fighters[actor].delay < 20
                {
                    self.moral_failure_escape(rng, actor, &mut b1ab18, &mut b1ab19);
                }
            }

            let d = self.fighters[actor].delay;
            if d == 0 || d == 20 {
                delayed = false;
            }

            if !stop && delayed {
                counter += 1;
                if counter > 20 {
                    stop = true;
                    delayed = false;
                    self.try_guarding(actor);
                }

                if !stop {
                    let mut reachable = false;
                    // Attack range (`ovr010.cs:562-572`, doc §34.4): the readied
                    // weapon's table range less one, sanitized. LongBow (22) →
                    // 21, ShortBow (16) → 15; a melee combatant (no loadout)
                    // stays range 1. The held-target reach test and every
                    // `BuildNearTargets` below use THIS range, so a bowman's near
                    // list spans the room.
                    let range = self.weapon_range(actor);

                    // The binary's `player01` local (ovr010:0F12-0F46): load
                    // actions.target, then null the LOCAL if the target is out
                    // of combat or on the PARTY team — `cmp combat_team, 0` is
                    // an immediate-0 compare (Team::Party), NOT the attacker's
                    // team, and actions.target itself is NOT cleared. A monster
                    // therefore never keeps a held party target here: it always
                    // falls through to the near-list re-pick.
                    let mut chosen: Option<usize> = self.fighters[actor].target;
                    if let Some(t) = chosen {
                        let tf = &self.fighters[t];
                        if !tf.in_combat || tf.team == Team::Party {
                            chosen = None;
                        }
                    }

                    // Reachability probe (ovr010.cs:583-598) — draw-free.
                    if let Some(t) = chosen {
                        if self.can_see_target(t) {
                            let ap = self.fighters[actor].pos;
                            let tp = self.fighters[t].pos;
                            if let Some(steps) = can_reach(&self.map, ap, tp, range, false) {
                                if steps as i32 / 2 <= range {
                                    reachable = true;
                                }
                            }
                        }
                    }

                    if !reachable {
                        let near = self.build_near(actor, range, false);
                        if near.is_empty() {
                            // No adjacent enemy → approach one step toward the target.
                            if self.find_target(rng, actor, false, 0, 0xff) {
                                self.moral_failure_escape(rng, actor, &mut b1ab18, &mut b1ab19);
                            } else {
                                stop = true;
                                self.try_guarding(actor);
                            }
                        } else {
                            // An adjacent enemy exists → re-pick among them (:618).
                            // Binary loc_36036: the pick lands in the LOCAL
                            // `player01` only — actions.target is not written.
                            let roll = roll_dice(rng, near.len() as u16, 1);
                            let picked = near[(roll - 1) as usize].idx;
                            chosen = Some(picked);
                            // §34.4 cornered re-pick: a still-ranged (non-ranged-
                            // melee) attacker with an adjacent enemy unreadies via
                            // items_selection and STOPS — no attack this turn
                            // (ovr010.cs:622-628). Step-7 usually unreadied
                            // already (so is_weapon_ranged is false here and the
                            // else-if fires the punch); this covers the case a
                            // bowman is still readied at the near-pick.
                            if self.is_weapon_ranged(actor)
                                && !self.is_weapon_ranged_melee(actor)
                                && !self.build_near(actor, 1, false).is_empty()
                            {
                                self.ai_items_selection(actor);
                                stop = true;
                            } else {
                                let tp = self.fighters[picked].pos;
                                if get_target_range(&self.map, tp, self.fighters[actor].pos) == 1
                                    || self.can_see_target(picked)
                                {
                                    reachable = true;
                                }
                            }
                        }
                    }

                    if reachable {
                        let t = chosen.unwrap();
                        // Site 3 — the AI pre-attack camera (`ovr010.cs:637-639`,
                        // gated on `byte_1D90E == reachable`): scroll one step
                        // from the actor toward the target, radius 2. Fires before
                        // both TrySweepAttack and RecalcAttacksReceived.
                        let cam_dir =
                            target_direction(self.fighters[actor].pos, self.fighters[t].pos);
                        let ap = self.fighters[actor].pos;
                        self.redraw_combat_area(cam_dir, 2, ap);
                        if self.try_sweep_attack(t, actor) {
                            stop = true;
                            self.clear_actions(actor);
                        } else {
                            self.recalc_attacks_received(t, actor);
                            // §34.4 attack execution: for a ranged attacker,
                            // resolve the ammo item (`GetCurrentAttackItem`); a
                            // ranged-melee weapon at reach 1 passes null (thrown
                            // as melee, `ovr010.cs:655-664`). coab≠binary #17: the
                            // `byte_1D90E = GetCurrentAttackItem` re-assign at
                            // `ovr010:1176` is dead (verified) — only the item is
                            // used, the attack proceeds unconditionally.
                            let ranged_item = if self.is_weapon_ranged(actor) {
                                let mut item = self.get_current_attack_item(actor).item;
                                if self.is_weapon_ranged_melee(actor)
                                    && get_target_range(
                                        &self.map,
                                        self.fighters[t].pos,
                                        self.fighters[actor].pos,
                                    ) == 1
                                {
                                    item = AttackItemRef::None;
                                }
                                item
                            } else {
                                AttackItemRef::None
                            };
                            stop = self.attack_target(rng, actor, t, false, ranged_item);
                            if stop {
                                delayed = false;
                            } else if !self.fighters[t].in_combat {
                                stop = true;
                            }
                        }
                    }
                }
            }
        }

        !delayed
    }

    /// `PlayerQuickFight(actor)` (`ovr010.cs:8`) — the whole melee AI turn, in draw
    /// order (study §4.1): the `field_15` mode-gate, `FleeCheck_001` (draw-free),
    /// the two normal-area behavior-guard d7s (`sub_354AA:192` + `sub_3560B:248`),
    /// then the `find_target` pick and the `sub_35DB1` move-attack loop. Spell/
    /// wand/turn-undead **effects** are stubbed; their **guards and draws** are
    /// faithful. Every draw flows through `rng`, so an attached `RngSink` sees the
    /// exact stream (D9).
    pub fn melee_ai_turn(&mut self, rng: &mut EngineRng, actor: usize) {
        // process_input_in_monsters_turn — headless, draw-free, returns false.
        if !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return;
        }

        // 1. field_15 mode-gate (ovr010.cs:20-36).
        self.fighters[actor].field_15 = field_15_mode_gate(rng, self.fighters[actor].field_15);

        // 2. FleeCheck_001 (ovr010.cs:40) — draw-free.
        let surrendered = self.flee_check(actor);
        if surrendered {
            return;
        }

        // 3. sub_354AA wand scan (ovr010.cs:54) — the normal-area d7.
        if self.wand_scan_d7(rng, actor) {
            self.clear_actions(actor);
            return;
        }

        // 4. queued spell (spell_id>0) — none for a fighter.
        // 5. turn_undead — non-cleric, short-circuit, draw-free.

        // 6. sub_3560B (ovr010.cs:74) — the memorized-spell selection loop
        // (doc §41.1). It always draws the unconditional d7 bound
        // (`ovr010:066D`, :248) — the draw this step already carried — and, only
        // when its gate passes (memorized slots exist, the caster is
        // NPC-controlled OR `AutoPCsCastMagic` is on, and an enemy is live,
        // `ovr010:0679-06A7`), the priority-pass selection draws. A PC with magic
        // OFF still draws only the d7 (capture-proven: bar-fists-2 closes
        // 3811/3811 with two memorized MM slots and no selection draws, doc §33).
        // On a modeled cast the AI turn RETURNS immediately (`ovr010.cs:74-77`) —
        // no items_selection, no melee targeting, no movement.
        if self.sub_3560b(rng, actor) {
            return;
        }

        // 7. AI_items_selection (ovr010.cs:79) — the cornered weapon swap
        // (§34.5): a bowman with an adjacent enemy unreadies to bare hands here,
        // at the TOP of the turn (before find_target / the move-attack loop), so
        // the swing below is a punch; the room clearing re-readies the bow.
        // Draw-free; inert without a loadout.
        self.ai_items_selection(actor);
        // 8. process_input again — draw-free.

        // 9. the target/move-attack loop (ovr010.cs:82-95).
        loop {
            let found = self.find_target(rng, actor, false, 1, 0xff);
            if found && self.fighters[actor].delay > 0 && self.fighters[actor].in_combat {
                if self.sub_35db1(rng, actor) {
                    break;
                }
            } else {
                self.try_guarding(actor);
                break;
            }
        }

        // The turn's `ai` action event (§9): its resolved mode + target.
        self.emit(ActionEvent::Ai {
            combatant_id: actor,
            field_15: self.fighters[actor].field_15,
            target_id: self.fighters[actor].target.map(|t| t as i64).unwrap_or(-1),
        });
    }
}
