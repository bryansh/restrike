# Combat study — reading the original's combat engine (M4 step 5, opening)

> **This is a study, not a door.** It locks no decisions. It maps the original
> (via coab, read-for-behavior per D11 — never copied) so the implement-to-parity
> sessions transliterate instead of spelunking. Where a number or a semantic is
> unverifiable from a static read, it gets a docket pointer, not an assertion.
> coab is a hypothesis; the binary and the game validate (divergence count is at
> seven-plus — `docs/fidelity-docket.md`). Every claim below cites a coab
> `file:line`; combat globals additionally carry the original's data-segment
> address where coab records one (its `// byte_1XXXX` / `seg600:XXXX` comments).
>
> **Scope guardrails (from the session brief and `oracle-rig.md` D-OR3/D-OR5):**
> this session implements **no** combat mechanics, no state machine, no AI. The
> action-profile event vocabulary in §9 is **PROPOSED only** — pinned when the
> systems land, per D-OR3's rule. Bootstrap order is D-OR5(a)'s: Phase 0
> observe-only → Phase 1 replay-to-parity → Phase 2 scripted turns. Nothing here
> presupposes that order.
>
> Status: opened 2026-07-16, M4 step 5. H3 is CLOSED (`gbx-prng` bit-exact,
> live-chain-verified); this is the groundwork for H4's combat half.

---

## 0. The one-screen map

A fixed encounter runs entirely inside `CMD_Combat` (`ovr003.cs:971`,
`sub_277E4`), which — when monsters are loaded — calls `MainCombatLoop`
(`ovr009.cs:22`, `sub_33100`) and then `AfterCombatExpAndTreasure`
(`ovr006.cs:763`, `sub_2E7A2`). The loop is:

```
CMD_Combat (ovr003.cs:971)
├─ BattleSetup            (ovr011.cs:1169)  battlefield + placement, combat_round=0
└─ MainCombatLoop         (ovr009.cs:22)
   while (!end_combat):
     CountCombatTeamMembers                 (ovr025.cs:1268) → friends_count/foe_count
     for each player: CalculateInitiative   (ovr014.cs:8)    → d6+DexReact → action.delay
     for each player in FindNextCombatant(): (ovr009.cs:59)   → d100 draw-order pick
        DoPlayerCombatTurn(player)           (ovr009.cs:103)
          └─ QuickFight AI or combat_menu
     end_combat = BattleRoundChecks()        (ovr009.cs:363)  combat_round++, bleed, win/loss
   free_combat_stuff                         (ovr009.cs:9)
AfterCombatExpAndTreasure                    (ovr006.cs:763)   XP + treasure
```

The rest of this document expands each box.

---

## 1. The round loop and its state (feeds D-OR5(b)'s structure-walk prerequisites)

> **PARTIALLY IMPLEMENTED — initiative slice (2026-07-16).** The round skeleton
> (`count → initiative → turns → BattleRoundChecks`) is realized as a tick-based
> `CombatState` in `gbx-engine`'s `combat` module (D8: `step()` returns control,
> no blocking loop). Only the draw-bearing parts land this slice: initiative
> (§2), the `combat_round` counter + stalemate-cap termination, and the
> surprise-mask clear (`ovr009.cs:44`). The turn body, `step_game_time`, affect
> ticks, bleed/bandage, death/counts, and the map are stubs/out of scope. State
> variables in §1.1 that this slice touches: `TeamList` (roster order),
> `combat_round` (`byte_1D8B7`), `combat_round_no_action_limit` (=15),
> `area2_ptr.field_596` (surprise mask).

`MainCombatLoop` (`ovr009.cs:22`) is a `while (end_combat == false)` loop. Each
iteration is one **combat round**:

1. `CountCombatTeamMembers()` (`ovr025.cs:1268`) recomputes `gbl.friends_count`
   / `gbl.foe_count` (`Gbl.cs:697-698`).
2. **Initiative:** `foreach (Player player in gbl.TeamList) CalculateInitiative(player)`
   — every combatant rolls its `action.delay` for this round (§2).
3. `gbl.area2_ptr.field_596 = 0` — clears the per-round surprise/bonus flag that
   `CalculateInitiative` reads (`ovr009.cs:44`, `ovr014.cs:38`).
4. **Turns:** `foreach (Player player in FindNextCombatant()) DoPlayerCombatTurn(player)`
   — the iterator yields combatants in draw-order until none remain (§2).
5. `end_combat = BattleRoundChecks()` (`ovr009.cs:363`, `battle01`).

On exit: `free_combat_stuff()` (`ovr009.cs:9`, `sub_3304B`) clears gas-cloud
lists and the spell-cast delegate; `gbl.DelayBetweenCharacters = true`.

### 1.1 State variables the loop touches (with original addresses)

| Global | coab decl | Original addr | Role in the loop |
|---|---|---|---|
| `game_state` | `Gbl.cs` | — | set to `GameState.Combat` at entry (`ovr009.cs:24`) |
| `TeamList` | `Gbl.cs:496` | **heap list**, `player_next_ptr` | the combatant roster (party + ≤63 monsters), iteration order |
| `friends_count` | `Gbl.cs:697` | — | live allied count; 0 ⇒ battle over |
| `foe_count` | `Gbl.cs:698` | — | live enemy count; 0 ⇒ battle over |
| `combat_round` | `Gbl.cs:382` | `byte_1D8B7` | round counter; `++` in `BattleRoundChecks` (`ovr009.cs:366`) |
| `combat_round_no_action_limit` | `Gbl.cs:383` | `byte_1D8B8` | stalemate cap; init = `combat_round_no_action_value` = **15** (`Gbl.cs:384`) |
| `enemyHealthPercentage` | `Gbl.cs:388` | `byte_1D903` | `((20·ΣcurHP)/ΣmaxHP)·5` over enemies (`ovr014.cs:1674`); morale + AI input |
| `monster_morale` | `Gbl.cs:348` | `byte_1D2CC` | per-combatant morale scratch (§6) |
| `numLoadedMonsters` | `Gbl.cs:294` | `byte_1AB0E` | monsters spawned this encounter (cap 63, `ovr003.cs:243`) |
| `CombatMap[]` | `Gbl.cs:506` | `seg600:66BD` (`stru_1C9CD`) | **grid geometry only** — `CombatantMap{pos,size,screenPos}` (`Combat/CombatantMap.cs`) |
| `CombatantCount` | `Gbl.cs:505` | `stru_1C9CD[0].field_3` | live cell count |
| `attack_roll` | `Gbl.cs` | — | last d20 to-hit (`ovr024.cs:490`), reset 0 in `BattleSetup` |
| `SelectedPlayer` | `Gbl.cs` | — | whose turn is being processed (`ovr009.cs:117`) |
| `area2_ptr.field_596` | `Area2` | — | per-round team surprise/init-bonus flag |

**Per-combatant turn state is the `Action` struct** (`Classes/Action.cs`), which
the door (D-OR5(b)) notes hangs off `Player+0x18d` and carries **no
`[DataOffset]`s** — coab reconstructs it, so its runtime layout is an RE
deliverable, not a known offset table. Its fields (coab's own byte comments):

| Field | off | Meaning |
|---|---|---|
| `spell_id` | 0x00 | queued spell for this turn (0 = none) |
| `can_cast` / `can_use` | 0x01 / 0x02 | may cast / may use item this round |
| `delay` | 0x03 | **initiative value** — the draw-order key (§2) |
| `attackIdx` | 0x04 | which attack profile (1 or 2) is active |
| `maxSweapTargets` | 0x05 | sweep-attack cap = `attackLevel` |
| `move` | 0x06 | movement points left (half-moves; §3) |
| `guarding` | 0x07 | set by `TryGuarding` when no target |
| `direction` | 0x09 | facing |
| `target` | 0x0A | current target `Player*` |
| `bleeding` | 0x0E | dying-bleed counter (→dead at >9, `ovr009.cs:378`) |
| `AttacksReceived` | 0x0F | attacks taken this turn (rear/multi tracking) |
| `fleeing` / `moral_failure` | 0x10 / 0x14 | morale outcomes (§6) |
| `hasTurnedUndead` | 0x11 | one turn-undead attempt per battle |
| `directionChanges` | 0x12 | facing-change count this turn |
| `field_15` | 0x15 | QuickFight target-mode scratch (§4) |

> **For D-OR5(b):** the checkpoint projection the door specifies
> (`{combatant_id, hp_current, hp_max, status, grid_pos, attacks_left, ac}` +
> turn-order list) maps to: `hp_current`=`Player@0x1a4`, `hp_max`=`@0x78`,
> `status`=`health_status@0x195`, `ac`=`@0x19a`, `attacks_left`=`attack1_AttacksLeft@0x19c`,
> `grid_pos` from the `CombatMap` cell, turn-order = the `action.delay` values +
> the `FindNextCombatant` draw sequence. The **walk** to reach them still needs
> the `TeamList` head address and the `Action` runtime layout — unchanged from
> the door's §5 open list.

### 1.2 `BattleRoundChecks` (`ovr009.cs:363`) — end-of-round

```
step_game_time(1,1)                 advance clock one tick
combat_round++                      (ovr009.cs:366) — the byte_1D8B7 increment
calc_enemy_health_percentage()      recompute enemyHealthPercentage
for each player:
    CheckAffectsEffect(Type_19)     per-round affect ticks
    in_poison_cloud(0, player)      cloud damage
    if health_status == dying: bleeding++; if bleeding > 9 → dead
bandage(false)                      auto-bandage check
CountCombatTeamMembers()
redrawCombatArea(...)
battleOver = (friends_count==0 || foe_count==0 || combat_round >= no_action_limit)
if (friends_count>1 && foe_count==0 && !inDemo && yes_no("Continue Battle:")=='Y') battleOver=false
return battleOver
```

The stalemate cap (`combat_round >= 15`) guarantees termination even if neither
side can finish the other. `step_game_time` is a **non-RNG** clock advance.

---

## 2. Initiative — the draw-order signature (settles FD-2 via D-OR5(a))

> **IMPLEMENTED — initiative slice (2026-07-16, D-OR5(a) Phase 1, first slice).**
> Both routines below are transliterated in `gbx-engine`'s `combat` module
> (`CalculateInitiative` → `calculate_initiative`, `FindNextCombatant` →
> `select_or_end` + the pure `select_combatant`), with synthetic draw-sequence
> tests. The coab re-read for this slice **matched §2.1/§2.2 exactly** (the
> clamp-then-`-6` ordering, the two-`if` tie-break with its `>`-only `max_roll`
> reset, and the `(A+1)·K` d100 count). Not yet parity-verified against a live
> trace — that closes FD-2. Turn resolution beyond initiative (the actual turn)
> is a later slice; here the turn slot is a zero-draw stub.

Two functions produce the round's turn order. Their combined RNG-stream shape is
what a Phase-0 trace matches.

### 2.1 `CalculateInitiative` (`ovr014.cs:8`, `sub_3E000`) — one d6 per combatant

Called once per combatant at round start. Resets the `Action` (spell_id=0,
can_cast/can_use=true, attackIdx=2), calls `reclac_attacks` (§3), sets movement
half-actions, then:

```
if (player.in_combat):
    action.delay = (sbyte)(roll_dice(6,1) + DexReactionAdj(player))   ← ONE d6 draw
    if (action.delay < 1) action.delay = 1
    if (((combat_team+1) & area2_ptr.field_596) != 0) action.delay -= 6   ← surprise/team penalty
    if (action.delay < 0 || action.delay > 20) action.delay = 0
else:
    action.delay = 0
action.move = CalcMoves(player)
```

So **initiative = d6 + Dexterity reaction adjustment**, clamped to `[1,20]` then
possibly `-6` for a team-flagged surprise, with out-of-range collapsing to 0
(0 = "never acts this round"). `DexReactionAdj` (`ovr025.DexReactionAdj`) is a
table lookup (no draw). **RNG cost: exactly one `roll_dice(6,1)` = one `random(6)`
draw per combatant with `in_combat==true`.**

> `roll_dice(6,1)` = `seg051.Random(6)+1` = one `gbx_prng` `random(6)` draw
> yielding 1..6 (`ovr024.cs:586`; migration ledger row 3). `DexReactionAdj` is
> a pack table, not a roll.

### 2.2 `FindNextCombatant` (`ovr009.cs:59`, `sub_331BC`) — d100 per pick pass

A C# iterator (`yield return`). Each pass over the **whole** `TeamList` rolls a
fresh d100 for every member, then yields the member with the highest `delay`,
ties broken by the highest d100 this pass:

```
do:
    output_player = null; max_delay = 0; max_roll = 0
    foreach (player in TeamList):
        roll = roll_dice(100,1)                      ← ONE d100 draw PER MEMBER, EVERY pass
        if (player.actions.delay > max_delay) max_roll = roll
        if (player.actions.delay >= max_delay && roll >= max_roll):
            max_roll = roll; max_delay = player.actions.delay; output_player = player
    if (max_delay == 0) output_player = null
    if (output_player != null) yield return output_player
while (output_player != null)
```

A yielded combatant takes its turn (`DoPlayerCombatTurn`), and the turn sets its
`action.delay = 0` on completion (`ovr010.cs:521`, the AI move/attack path;
`ovr025.cs:1240` / `ovr033.cs:605` on other exits) so it is not re-picked. The
loop ends on the first pass where every remaining `delay == 0`.

**Draw-stream shape of one round** (the signature D-OR5(a) matches):

- **K** = `TeamList.Count` (party + monsters, all iterated regardless of alive).
- **A** = number of combatants that actually act (delay stayed > 0).
- Initiative phase: **K_c** `random(6)` draws, where K_c = combatants with
  `in_combat==true` (`CalculateInitiative`).
- Selection phase: one `random(100)` per member per pass; there are **A+1**
  passes (A yielding passes + one final empty pass) ⇒ **(A+1)·K** `random(100)`
  draws.
- Interleaved with selection: each acted turn's own draws (§4/§5) fall *between*
  the pass that selected it and the next pass.

This ordering — d6×K_c, then blocks of d100×K separated by each actor's turn
draws — is the precise per-round fingerprint. **Because the original consumes
turn order and never persists it (`FindNextCombatant` re-rolls every pass, stores
nothing), draw-order parity settles FD-2 by itself** (D-OR5(a)): two orderings
can share an endstate, so only the draw stream distinguishes them.

---

## 3. Action economy (attacks & movement per round)

### 3.1 Attacks per round (feeds FD-3)

`reclac_attacks` (`ovr014.cs:462`, `sub_3EDD4`) + `ThisRoundActionCount`
(`ovr014.cs:519`, `sub_3EF0D`):

```
ThisRoundActionCount(halfActions):
    if ((combat_round & 1) == 1) halfActions++      ← odd rounds get +1 half-action
    return halfActions / 2
```

This is the AD&D **3/2-attacks** mechanism: a combatant whose `attacksCount`
yields 3 half-actions gets `(3+1)/2 = 2` attacks on odd rounds and `3/2 = 1` on
even rounds — 3 attacks per 2 rounds. `attack1_AttacksLeft` (`Player@0x19c`) is
set from `attacksCount` (`ovr014.cs:467`) then reduced to
`ThisRoundActionCount(halfActionsLeft)` (`:514`). Ranged weapons override the
half-action count from the item's `numberAttacks` (min 2, `:473-480`) and clamp
to available ammo (`:501-505`). `attack2_AttacksLeft` (`@0x19d`) holds movement
half-actions (`ovr014.cs:25`).

- **`attacksCount`** (the base) is a `Player` field derived from class/level and
  the equipped weapon — not itself a per-round roll.
- **Multi-attack monsters:** the two attack profiles (attack1 / attack2, §5.1)
  each carry their own dice; `attackIdx` (`Action@0x04`) selects which is live.

### 3.2 Movement

`CalcMoves` (`ovr014.cs:58`, `sub_3E124`) = `player.movement` (+ a wilderness
bonus when out of combat), clamped `[1,96]`, doubled into `halfActionsLeft`
(half-move granularity). No RNG. `action.move` (`Action@0x06`) is decremented as
the combatant steps; diagonal steps cost `move_cost·3`, orthogonal `move_cost·2`
(`ovr009.cs:525-528`). Movement resets each round via `CalculateInitiative`.

---

## 4. The QuickFight AI (`ovr010.cs:8`, `sub_3504B`) — the Phase-1 spec

`PlayerQuickFight(player)` is the whole-turn AI, run for any combatant with
`quick_fight == QuickFight.True` (`ovr009.cs:134`). Both sides run it in Phase 0
(observe-only). The decision tree, top to bottom (first satisfied branch ends the
turn):

```
PlayerQuickFight(player):
 1. process_input_in_monsters_turn(player)          poll for human interrupt (space/'2'/'-') (ovr010.cs:705)
 2. if !in_combat: clear_actions; done
 3. TARGET-MODE scratch (field_15):                 ← RNG
       if (field_15 ∈ {0,4}  ||  roll_dice(4,1)==1):
           v = roll_dice(8,1)
           field_15 = (v==8) ? roll_dice(4,1) : roll_dice(2,1)+4
    (a d4 gate; on fire, a d8 then either a d4 or d2 — sets a movement/approach mode)
 4. FleeCheck_001(player)                            morale (§6); may set moral_failure/flee
 5. if moral_failure && !fleeing: "flees in panic"
 6. if (var_2) return                                (input or flee ended the turn)
 7. sub_354AA(player)   → wand/magic-item use        ← RNG: roll_dice(7,1) priority scan (ovr010.cs:183)
 8. if actions.spell_id > 0: cast queued spell; done (ovr010.cs:60)
 9. turn_undead(player): if cleric & undead present; done (ovr010.cs:99)
10. sub_3560B(player)  → cast a memorized spell      AI spell selection from LearntList (ovr010.cs:232)
11. AI_items_selection(player)                       ready best weapon
12. LOOP until acted:
       if (find_target(...) && delay>0 && in_combat):
            sub_35DB1(player)   → approach + melee/ranged attack   (sets delay=0 at ovr010.cs:521)
       else:
            TryGuarding(player); done
```

**Priority order (the implementable spec):** interrupt → morale/flee → **wand** →
**queued spell** → **turn undead** → **memorized spell** → **weapon (move+attack)**
→ **guard**. Depth notes:

- **Spell casting decisions** (`ShouldCastSpellX`, `ovr010.cs:143`;
  `ShouldCastSpellX_sub1`, `:117`) score candidate spells by a
  `spellCastingTable[id].priority` threshold and whether targets would fail their
  save (`RollSavingThrow`, a d20 per candidate target — RNG-bearing). `sub_354AA`
  scans item spells across `roll_dice(7,1)` priority bands (`:192`).
- **`find_target`** (`ovr014.find_target`) selects a target and, on the melee
  path, the approach direction; `sub_35DB1` performs the move + one or more
  attacks (each attack = `CanHitTarget`/`PC_CanHitTarget` d20 + a damage
  `roll_dice`, §5). It sets `action.delay = 0` when the turn is spent.
- **`TryGuarding`** is the fallback when no target is reachable — sets
  `action.guarding`.

> Depth caveat: this is a one-read outline. `sub_35DB1`, `find_target`,
> `AI_items_selection`, and the `sub_3560B` spell-selection loop each have
> internal RNG and target-scan logic that the Phase-1 implementation session must
> read in full against a live Phase-0 trace. The **branch structure** above is
> firm; the per-branch draw counts are not yet enumerated (they depend on target
> geometry) — which is exactly why D-OR5(a) mandates observe-only capture first.

---

## 4.1 The melee AI turn — the complete draw-sequence map (M4 combat #4)

> **Implementation status (2026-07-16):** the **whole melee AI turn is landed**
> in `gbx-engine`'s `combat` module. Deliverable 2 (the range ray §4.1.3):
> `reach_ray`/`can_reach`/`get_target_range`/`build_near_targets`. Deliverable 3
> (the turn): `field_15_mode_gate` (§4.1.2), `flee_check` (draw-free morale), the
> two behavior-guard d7s (`wand_scan_d7` site C + the unconditional `sub_3560B`
> d7 site F), `find_target` (pick+retry), and `sub_35db1` (the move-attack loop
> with the per-step monster d100, opportunity attacks, and `attack_target`) — all
> on a `CombatWorld`/`Fighter` model, faithful and draw-exact. The **parity
> artifact** proves it: `melee_turn_adjacent_draws_the_exact_sequence` asserts the
> full turn's operand stream against an independent replay (§4.1.7's worked
> example), `monster_approach_…` proves the PC/NPC d100 asymmetry, and
> `all_ai_1v1_fight_…` runs a fight to a victor with a Prng-consistent,
> deterministic draw stream. **Still remaining:** pin the `ai`/`morale`/`move`
> action events (D4), wire the turn into `CombatState::step` replacing the stub
> (D5), and update the ASCII demos to the real AI (D6). Spell/wand/turn-undead
> **effects**, backstab detection, ranged weapons, sweep (0-HD) attacks, and
> affects are stubbed (guards+draws faithful; effects deferred to M5).

> **THIS IS THE PARITY SPEC.** The full melee call chain was read leaf-to-leaf
> for the AI slice (2026-07-16). Every `roll_dice`/`Random` site a melee
> combatant's `PlayerQuickFight` turn can reach is listed below **in the exact
> order the original draws them**, each with its `file:line` and the guard that
> gates it. The audit re-derives one turn's draw sequence by hand against this
> table; the implementation is checked against it. Draw ORDER is the whole game
> (D9) — one mis-ordered/missing/extra draw diverges the fight.
>
> **Scope of "the canonical melee combatant"** used to classify each site:
> **non-cleric, spell-less, item-less-but-weapon-equipped**, in a **normal area**
> (spells allowed), **not fleeing**, **passing morale**. Sites a *pure fighter*
> never reaches are marked **guarded-off (proof: …)**; sites it *does* reach are
> **reached — reproduce**. Spell/wand/turn-undead **effects** are stubbed for the
> slice, but any **draw** the fighter reaches on the way to those guards is
> faithful.
>
> **Draw-free helpers (verified by read, so they never appear below):** `ovr025`
> and `ovr032` contain **zero** `roll_dice`/`Random`/`Randomize` calls — so
> `BuildNearTargets`, `getTargetRange`, `canReachTarget`, `CanSeeCombatant`,
> `Rebuild_SortedCombatantList`, `bandage`, `reclac_player_values`,
> `is_weapon_ranged*`, `GetCurrentAttackItem` are all draw-free. Also draw-free by
> read: `reclac_attacks`/`ThisRoundActionCount` (`ovr014.cs:462/519`),
> `CalcMoves` (`:58`), `MaxOppositionMoves` (`:1699`), `CanBackStabTarget`
> (`:1433`), `RecalcAttacksReceived` (`:887`), `getTargetDirection` (`:1460`),
> `RemoveAttackersAffects` (`ovr024.cs:694`), `AI_items_selection`
> (`ovr010.cs:875` — no `roll_dice`; `CalcItemPowerRating` is table math),
> `process_input_in_monsters_turn` (`ovr010.cs:705` — keyboard only; headless =
> draw-free), `CanSeeTargetA` (`ovr014.cs:571` — invisibility affect check).
> `CheckAffectsEffect` is draw-free **only when the combatant has no draw-bearing
> affect** — true for the affect-less synthetic rosters this slice uses; monster
> innate affects (`MON*SPC`) may add draws inside `CheckAffectsEffect`, which is
> M5 affect-system territory (deferred, flagged at each call site).

### 4.1.1 The turn, top to bottom (`PlayerQuickFight`, `ovr010.cs:8`)

| # | Site | coab | Draw(s) | Guard / when |
|---|---|---|---|---|
| A | **`field_15` mode-gate** | `ovr010.cs:20-36` | **see §4.1.2** | reached — reproduce |
| B | **`FleeCheck_001`** | `ovr010.cs:40`→`:760` | **0** (normal) | morale is draw-free here; the d100/d2 live in the *flee-move* path (§4.1.4) |
| C | **`sub_354AA`** (wand scan) | `ovr010.cs:54`→`:183`; d7 at **`:192`** | **1 × d7** | reached — reproduce. Fires iff `can_use && oppTeamCount>0 && area.can_cast_spells==false`; in a normal area `can_cast_spells==false` (see note), `can_use=true` (set by `CalculateInitiative`, `ovr014.cs:14`) ⇒ **fires**. Item scan is draw-free for a weapon-only combatant (no readied spell-item). |
| D | queued-spell cast | `ovr010.cs:60` | 0 | guarded-off (`spell_id==0` for a fighter) |
| E | **`turn_undead`** | `ovr010.cs:68`→`:99` | **0** | guarded-off (proof: `cleric_lvl==0 && !(cleric_old_lvl>multiclassLevel)` short-circuits **before** `FindLowestE9Target`, `ovr010.cs:103-105`) |
| F | **`sub_3560B`** (memorized spell) | `ovr010.cs:74`→`:232`; d7 at **`:248`** | **1 × d7** | reached — reproduce. The `var_5B = roll_dice(7,1)` at `:248` is **UNCONDITIONAL** — computed before the `if (spells_count>0 && …)` guard. The inner `roll_dice(spells_count,1)` at `:261` is guarded-off (proof: `spells_count==0` ⇒ the `while` never runs). |
| G | `AI_items_selection` | `ovr010.cs:79`→`:875` | 0 | reached — draw-free |
| H | `process_input` | `ovr010.cs:80` | 0 | headless — draw-free |
| I | **`find_target(false,1,0xff)`** | `ovr010.cs:84`→`ovr014.cs:2238` | **0 or ≥1 × d(nearCount)** | reached — see §4.1.3 |
| J | **`sub_35DB1`** (move+attack) | `ovr010.cs:88`→`:511` | **many** | reached — see §4.1.4 |
| K | `TryGuarding` | `ovr010.cs:93`→`:685` | 0 | fallback when `find_target` fails — draw-free |

> **`can_cast_spells` polarity (caller read — the field is inverted vs its name):**
> the "Cast" combat-menu option is shown when `area_ptr.can_cast_spells == false`
> (`ovr009.cs:331-333`), and area init sets it to `false` (`ovr008.cs:113`). So
> **`false` = casting ALLOWED (the normal state)**, `true` = a silence/anti-magic
> zone. Therefore in a normal fight site C's guard **passes** and the d7 fires.
> (Same subtlety as slice 2's `PC_CanHitTarget` mislabel: verify by caller, not by
> name.) The strawman map in the session brief called site C's d7 area-blocked;
> the read shows the opposite — it fires in ordinary combat.

### 4.1.2 The `field_15` mode-gate — the C# short-circuit correction (LANDMINE)

```csharp
int var_1 = player.actions.field_15;                       // persistent per-combatant
if (var_1 == 0 || var_1 == 4 || roll_dice(4,1) == 1) {     // ovr010.cs:22
    var_1 = roll_dice(8,1);                                 // ovr010.cs:24
    if (var_1 != 8) var_1 = roll_dice(2,1) + 4;             // ovr010.cs:28  → 5..6
    else            var_1 = roll_dice(4,1);                 // ovr010.cs:32  → 1..4
}
player.actions.field_15 = var_1;
```

The `||` **short-circuits**, so the d4 gate at `:22` is **not always drawn**:

| `field_15` on entry | d4 gate (`:22`)? | body (`:24-32`)? | **draws** |
|---|---|---|---|
| **0 or 4** | **skipped** (short-circuit) | yes | **2** (d8, then d4 if d8==8 else d2) |
| ∉{0,4}, gate==1 | 1 × d4 | yes | **3** (d4, d8, then d4|d2) |
| ∉{0,4}, gate≠1 | 1 × d4 | no | **1** (d4 only) |

**`field_15` starts at 0** (the `Action` default; `CalculateInitiative` resets
`spell_id`/`can_cast`/`can_use`/`attackIdx` but **not** `field_15`,
`ovr014.cs:12-16`). So **every combatant's first turn takes the 2-draw path, not
3** — the brief's "1 draw always (the d4 gate)" is wrong for `field_15∈{0,4}`.
After a reroll `field_15∈{1..6}`; it can land on 4, re-triggering the
short-circuit next turn. `field_15` is only otherwise written in the flee path
(`ovr010.cs:400,443`). The value is later used as a **row index into `data_2B8`**
(`ovr010.cs:290`, the approach-direction table) — never re-clamped, so it must be
reproduced exactly.

### 4.1.3 `find_target(clear_target, arg_2, max_range, player)` (`ovr014.cs:2238`)

```
if (clear_target || existing target is same-team / not-in-combat / invisible)  → target = null
if (target != null)  → target_found = true               // 0 DRAWS (keeps a still-valid target)
while (!target_found && !var_5):                          // pass 0, then pass 1 (ignoreWalls)
    nearTargets = BuildNearTargets(max_range, player)     // draw-free
    tryCount = 20
    while (tryCount>0 && !target_found && nearTargets.Count>0):
        tryCount--
        roll = roll_dice(nearTargets.Count, 1)            // ovr014.cs:2275 — ONE d(count) PER RETRY
        target = nearTargets[roll-1].player
        if ((arg_2 && ignoreWalls) || CanSeeTargetA(target,player))  → target_found  // draw-free test
        else  nearTargets.Remove(target)                  // and retry
```

Draw cost: **0** if `player.actions.target` is still a valid enemy (very common
on turns after the first — the target persists across turns until cleared);
otherwise **1 × d(nearTargets.Count)** to pick a *visible* target on the first
try (`CanSeeTargetA` = not-invisible, true for ordinary enemies). Only invisible
picks force extra retries (each a fresh `d(count)`, count shrinking as candidates
are removed). Two passes exist (the second sets `ignoreWalls=true`); the second
runs only if pass 0 found nothing. Called with `max_range=0xff` from the top loop.

### 4.1.4 `sub_35DB1(player)` — the move-then-attack loop (`ovr010.cs:511`)

```
CheckAffectsEffect(Type_14)                               // draw-free (no affects)
if (combat_team==Ours && bandage(true)) delay=0           // party only; bandage draw-free
delayed = (delay != 0)
while (!stop && delayed):
  if (moral_failure): while(move>0 && 0<delay<20) moralFailureEscape(player)   // FLEE path, §below
  if (delay==0 || delay==20) delayed=false
  if (!stop && delayed):
    counter++; if (counter>20){ stop; TryGuarding }        // 20-iteration safety cap
    range = (primaryWeapon ? ItemDataTable[weapon].range-1 : 1); clamp 0/0xff/-1 → 1
    target = actions.target
    // (1) reachability probe — DRAW-FREE:
    if (target valid && CanSeeTargetA) and canReachTarget(steps,…) and steps/2<=range → byte_1D90E=true
    // (2) if not yet reachable:
    if (!byte_1D90E):
        nearTargets = BuildNearTargets(range, player)       // draw-free
        if (count==0):                                       // no adjacent target → approach
            if (find_target(false,0,0xff,player)) moralFailureEscape(player)   // move a step (§)
            else { stop; TryGuarding }
        else:
            roll = roll_dice(nearTargets.Count,1)            // ovr010.cs:618 — ONE d(count)
            target = nearTargets[roll-1].player              //   (re-pick among adjacent)
            if (ranged && !ranged_melee && BuildNearTargets(1).Count>0){ AI_items_selection; stop } // draw-free
            else if (getTargetRange(target)==1 || CanSeeTargetA) byte_1D90E=true
    // (3) attack if in range:
    if (byte_1D90E):
        if (TrySweepAttack(target,player)) { stop; clear_actions }   // draw-free unless target.HitDice==0 (§)
        else:
            RecalcAttacksReceived(target,player)             // draw-free
            (ranged item selection — draw-free)
            stop = AttackTarget(item,0,target,player)         // §4.1.5 — the d20s + damage
```

**The `roll_dice(nearTargets.Count,1)` at `:618`** fires only when the
reachability probe (1) failed **and** an adjacent target exists (count>0) — i.e.
a re-pick among in-range foes when the primary target wasn't directly reachable.
In the clean "target already adjacent/reachable" case, (1) sets `byte_1D90E`
and `:618` is **skipped**.

**Approach movement — `moralFailureEscape` (`sub_359D1`, `ovr010.cs:369`)** (also
the normal step-toward-target routine despite the name):

```
if (move/2>0 && delay>0):
  if ( control_morale<NPC_Base                                   // A: player-controlled
     || (control_morale>=NPC_Base && enemyHealthPercentage <= roll_dice(100,1)+monster_morale)  // B&C — ovr010.cs:387
     || combat_team==Enemy ):                                    // D
    if (moral_failure==false)  dir = getTargetDirection(target,player)   // draw-free
    else { field_15 = roll_dice(2,1);  … }                       // ovr010.cs:400 — FLEE only
    while (dirStep<6 && !var_5 && !CanMove(dir,dirStep,player)):  // CanMove draw-free unless in a cloud (§)
        …
    move_step_away_attack(direction,player)                       // §4.1.6 opportunity attacks
    if (move>0) sub_3E748(direction,player)                       // §4.1.6 opportunity attacks
    in_poison_cloud(1,player)                                     // draw-free (no cloud)
  else TryGuarding
```

**The per-step morale-advance d100 (`ovr010.cs:387`) is asymmetric by control:**
by C# `||` short-circuit, operand **A** (`control_morale < NPC_Base`) is true for
a **player-controlled** combatant ⇒ the d100 is **not drawn**; for an
**NPC/monster** (A false, B true) operand **C** is evaluated ⇒ **1 × d100 per
approach step**. So **each monster approach step draws a d100; each PC approach
step draws none.** (This is the §6.2 "second morale gate" — it lives here in the
move path, *not* in `FleeCheck_001`.)

### 4.1.5 `AttackTarget → AttackTarget01` (`ovr014.cs:904/724`) — the swings

Reached via `sub_35DB1` (in-range) or the opportunity-attack sites. Per swing, in
the `for(attackIdx = actions.attackIdx; attackIdx>=1; attackIdx--)` /
`while(AttacksLeft(attackIdx)>0)` loop (`ovr014.cs:811-847`):

- **1 × d20** to-hit — `PC_CanHitTarget` (`:821`); this is slice 2's `>=` weapon
  path (already implemented).
- **on a hit only:** `sub_3E192` (`:828`) → `roll_dice_save(diceSize,diceCount)` =
  **`diceCount × random(diceSize)`** damage draws + bonus, byte-truncated
  (slice 2's `roll_damage`).

So the attack draws **N_attacks × d20**, plus damage dice on each hit, where
`N_attacks = attack1_AttacksLeft (+ attack2_AttacksLeft)` — a **data-driven,
non-RNG** count from `reclac_attacks`/`ThisRoundActionCount` (round-parity 3/2
rule, §3.1). The held-target *slay* branch (`:740`) and backstab AC/mult are
affect/positioning-gated (deferred). `TrySweepAttack` (`ovr014.cs:530`) is
**draw-free unless `target.HitDice==0`** (0-HD sweep victims), in which case it
issues `AttackTarget(null,0,…)` per swept target (extra d20s+damage) — deferred
with 0-HD monsters flagged.

### 4.1.6 Opportunity attacks — movement is NOT unconditionally draw-free

Two sites make *movement* draw-bearing once combatants are adjacent:

- **`move_step_away_attack` (`ovr014.cs:326`, from `moralFailureEscape:477`):**
  every enemy the mover **leaves** melee adjacency with gets a free
  `AttackTarget(null,1,player,attacker)` (`:407`) — the classic
  attack-of-opportunity. **Not** guard-gated. Draw cost = one full attack
  (§4.1.5) per departed adjacent enemy.
- **`move_step_into_attack` (`ovr014.cs:226`, from `sub_3E748:316`):** every
  adjacent **guarding** enemy (`actions.guarding==true`) attacks the mover
  entering its reach (`AttackTarget(null,0,…)`, `:245`). Guard-gated.

Early in a fight (teams `encounter_distance` apart, no one adjacent, no one
guarding) both lists are empty ⇒ approach steps are draw-free apart from the
monster d100. They become draw-bearing only once melee is joined. **These are the
subtlest draw sites in the whole turn** — they depend on live positions and the
`guarding` flag, so the implementation models both and the parity test exercises
a fight that reaches adjacency.

### 4.1.7 One-turn worked example (the audit's hand-check target)

A **monster** melee fighter, its **first** turn, target not yet adjacent, one
approach step to reach the target, then a single-attack swing that hits — normal
area, no clouds, no guards, no affects:

| order | draws | site |
|---|---|---|
| 1 | d8, then (d2 or d4) | field_15 gate, `field_15==0` path (§4.1.2) |
| 2 | d7 | `sub_354AA:192` (wand scan; normal area) |
| 3 | d7 | `sub_3560B:248` (unconditional) |
| 4 | d(nearCount) | `find_target:2275` (first target pick) |
| 5 | d100 | `moralFailureEscape:387` (monster approach step) |
| 6 | d20 | `AttackTarget01:821` (to-hit) |
| 7 | diceCount × d(diceSize) | `sub_3E192:86` (damage, on the hit) |

= for that turn: `{d8, d2|d4, d7, d7, d(n), d100, d20, dmg…}`. A **PC** fighter's
same turn drops the d100 at step 5 (operand A short-circuit). A turn where the
target is already adjacent drops steps 5 (no approach) and, if the reachability
probe succeeds, keeps step 4 only if the target wasn't already set. This is the
sequence the audit re-derives against the implementation.

---

## 5. To-hit, damage, saves (feeds FD-1 / FD-4)

> **IMPLEMENTED — attack slice (2026-07-16, D-OR5(a) Phase 1, second slice).**
> §5.2/§5.3/§5.4 are transliterated in `gbx-engine`'s `combat` module
> (`can_hit_target`/`pc_can_hit_target`, `roll_damage`+`backstab_multiplier`,
> `roll_saving_throw`, tied together by `resolve_attack`), draw-faithful through
> the one `EngineRng` seam, with synthetic draw-sequence tests vs an independent
> `gbx-prng` replay. **One correction from the caller read (coab wins over this
> study's §5.2 labels):** §5.2 below calls `CanHitTarget` "monster/generic" and
> `PC_CanHitTarget` "PC" — the caller read shows the real split is
> **weapon-attack vs scripted-effect**, not monster vs PC. `PC_CanHitTarget`
> (the `>=` path) is the **standard weapon-attack path for _any_ combatant**
> (both PCs and monsters): its only live caller is `AttackTarget01`
> (`ovr014.cs:821`, `sub_3F4EB`), the per-turn weapon body reached from the
> QuickFight AI / combat menu. `CanHitTarget` (the `>` path) is the scripted
> `DAMAGE`-opcode / area-effect path: its live caller is `CMD_Damage`
> (`ovr003.cs:1673`), rolling to hit a random party member. Backstab **detection**
> is deferred (needs facing/positioning); the multiplier math is faithful. Saves
> were implemented (clean read — the save target is on the record, not a rules
> pack). Not yet live-parity-verified — that closes FD-1/FD-4 at H4.

### 5.1 Attack profiles

Each `Player`/monster carries **two** attack profiles (`Player.cs:646-703`):

| Field | off | | Field | off |
|---|---|---|---|---|
| `attack1_AttacksLeft` | 0x19c | | `attack2_AttacksLeft` | 0x19d |
| `attack1_DiceCount` | 0x19e | | `attack2_DiceCount` | 0x19f |
| `attack1_DiceSize` | 0x1a0 | | `attack2_DiceSize` | 0x1a1 |
| `attack1_DamageBonus` | 0x1a2 (i8) | | `attack2_DamageBonus` | 0x1a3 |

`attackDiceCount(idx)` / `attackDiceSize(idx)` / `attackDamageBonus(idx)`
(`Player.cs:651/668/685`) select by `attackIdx`.

### 5.2 To-hit (settles FD-1)

Two paths, both roll one d20 and both promote a natural 20 and reject a natural 1:

- **`CanHitTarget(bonus, target)`** (`ovr024.cs:487`, `sub_641DD`) — monster/generic:
  ```
  attack_roll = roll_dice(20,1)
  if (attack_roll > 1):                    ← natural 1 = automatic miss
      if (attack_roll == 20) attack_roll = 100   ← natural 20 = auto-hit (forced huge)
      hit = (attack_roll + bonus) > target.ac
  ```
- **`PC_CanHitTarget(target_ac, target, attacker)`** (`ovr024.cs:515`, `sub_64245`) — PC:
  ```
  attack_roll = roll_dice(20,1)
  if (attack_roll > 1):
      if (attack_roll == 20) attack_roll = 100
      team_bonus = (attacker on Ours) ? area2.field_6E2 : area2.field_6E0
      hit = (attack_roll + attacker.hitBonus + team_bonus) >= target_ac
  ```

> **FD-1 resolved on the coab side:** *both* auto-rules exist — natural 1
> auto-misses (`attack_roll > 1` gate) and natural 20 auto-hits (promoted to 100,
> which beats any AC). The brief-vs-Jzatopa disagreement resolves in favor of
> "both exist." Two comparator subtleties worth carrying to H4: `CanHitTarget`
> uses strict `>` while `PC_CanHitTarget` uses `>=`, and the two consume the AC /
> bonus terms differently (target.ac + attacker bonus vs. explicit `target_ac` +
> `hitBonus` + team bonus). AC on disk is `ac@0x19a` with **display AC =
> `0x3C - ac`** (`Player.cs:598`). Settles fully only via H4 traces on edge rolls.

### 5.3 Damage (`sub_3E192`, `ovr014.cs:84`)

```
damage = roll_dice_save(attackDiceSize(idx), attackDiceCount(idx))   ← N dice of size S
damage += attackDamageBonus(idx)
if (damage < 0) damage = 0
if (CanBackStabTarget): damage *= ((thiefLevel-1)/4) + 2             ← backstab multiplier
CheckAffectsEffect(SpecialAttacks) / (Type_5)                         ← special-attack affects
```

`roll_dice_save` (`ovr024.cs:601`) just records `gbl.dice_count` and calls
`roll_dice`. **RNG cost of one damage roll = `dice_count` `random(dice_size)`
draws.**

### 5.4 Saving throws (`RollSavingThrow`, `ovr024.cs:554`)

```
savingThrowRoll = roll_dice(20,1)
if (roll == 1) fail
elif (roll == 20) succeed
else: roll += saveBonus + field_186; made = roll >= saveVerse[saveType]
```

Natural-1/natural-20 auto rules apply to saves too. `saveVerse[5]` = `@0xdf`
(paralyze/petrify/rod/breath/spell); `field_186` = `@0x186` (signed save bonus).

### 5.5 Sleep / held auto-kill (FD-4)

Not a single "coup-de-grace" branch: the read so far shows sleep/held are
`Affect`s (§7) that gate normal attack resolution via `CheckAffectsEffect`
(e.g. `CheckType.Type_16` on the target inside `CanHitTarget`, `ovr024.cs:500`).
`TrySweepAttack` (`ovr014.cs:530`) is a distinct mechanism: a melee attacker with
spare attacks vs. a **`HitDice == 0`** target sweeps multiple 1-HD targets. The
specific "auto-hit/auto-kill a sleeping/held target" resolution lives in the
`CheckAffectsEffect` handlers and the melee path (`sub_35DB1`) not yet read to the
leaf. **FD-4 stays narrowed** — the affect-gated structure is identified; the
exact auto-kill condition needs the melee-leaf read + H4 curated-encounter traces
(one attacker, one sleeping target) per D-OR5's FD-1/FD-4 note.

---

## 6. Morale, health%, and status

### 6.1 Enemy health percentage (`calc_enemy_health_percentage`, `ovr014.cs:1674`)

```
enemyHealthPercentage = ((20 · Σ enemy.hit_point_current) / Σ enemy.hit_point_max) · 5
```

A 0..100 value quantized to multiples of 5. Recomputed at `BattleSetup` and every
`BattleRoundChecks`.

### 6.2 Morale (`FleeCheck_001`, `ovr010.cs:760`)

Only NPC-controlled combatants (`control_morale >= Control.NPC_Base`) check
morale; player-controlled ones don't. `control_morale` = `Player@0xf7`
(`>= 0x80` ⇒ NPC, `Control.cs`).

```
monster_morale = (control_morale & Control.PC_Mask) << 1
if (monster_morale > 102) monster_morale = 0
CheckAffectsEffect(Morale)
if (monster_morale < (100 - hp%) || monster_morale == 0):
    monster_morale = enemyHealthPercentage
    CheckAffectsEffect(Morale)
    if (monster_morale < (100 - area2.field_58C) || monster_morale == 0 || team==Ours):
        if (MaxOppositionMoves(player) <= CalcMoves(player)/2):
            moral_failure = true      ← flees
```

A second morale gate appears in the AI move path (`ovr010.cs:387`): an NPC advances
only if `enemyHealthPercentage <= roll_dice(100,1) + monster_morale` — **one d100
draw** guarding the advance decision. This is a live RNG site inside the AI turn.

### 6.3 Status / `health_status` (`Player@0x195`)

`health_status` is a small enum: **okay / … / dying / dead / gone / animated**
(the values used in the loop: `dying`, `dead` at `ovr009.cs:374-381`; `gone` at
`ovr014.cs:661`; `animated` at `ovr010.cs:736`). Transitions seen:
`dying → dead` when `bleeding > 9` (`BattleRoundChecks`); `→ gone` on turn-undead
destruction; `→ dead/gone` on damage-to-0 in the melee path. Tier-1 status effects
(sleep/held/poison/unconscious/dying/dead per PLAN) are a mix of `health_status`
values and `Affect`s (§7). Exact enum integer values are **not yet pinned** — an
implementation-session read of the `Status` enum + `CheckAffectsEffect` handlers.

---

## 7. Affects (status-effect carrier)

Combat status effects are `Affect` records (`Classes/Affect.cs`, 9-byte on-disk
record — `AFFECT_RECORD_SIZE = 9`, matching `gbx-formats`). A monster's innate
affects load from `MON<area>SPC.dax` (§8). During combat they are applied and
queried through `CheckAffectsEffect(player, CheckType)` (`ovr024.cs`), which the
loop calls at many `CheckType.*` sites (movement, to-hit, morale, per-round tick).
The affect **vocabulary and byte layout** are deliberately out of scope for this
study — they land with status-tier-1 implementation (PLAN M4) and M5.

---

## 8. Monster / encounter data (the deliverable-2 trail, traced)

### 8.1 Where monster records come from

`CMD_LoadMonster` (`ovr003.cs:238`, the `MONSTER` opcode handler) reads three ECL
operands — monster id, copy count, CPIC block — and calls **`ovr017.load_mob(mod_id)`**
(`ovr003.cs:247`), then `ShallowClone`s the master up to `num_copies` times into
`gbl.TeamList` (cap 63, `ovr003.cs:243/268`). `load_mob` (`ovr017.cs:824`) is the
loader:

```
area_text = gbl.game_area.ToString()                                    // "1".."6"
load_decode_dax(out data, ..., monster_id, "MON"+area_text+"CHA.dax")   // ← the record
player = new Player(data, 0)                                            // decode 0x1A6 record
load_decode_dax(..., monster_id, "MON"+area_text+"SPC.dax")            // innate Affects (9B each)
    → for offset in 0..size step 9: player.affects.Add(new Affect(data, offset))
load_decode_dax(..., monster_id, "MON"+area_text+"ITM.dax")           // carried Items (Item.StructSize each)
    → for offset in 0..size step Item.StructSize: player.items.Add(new Item(data, offset))
```

**Finding: a monster is stored as a full `Player` record** — `new Player(data, 0)`,
`StructSize = 0x1A6` (`Player.cs:708/715`). There is no separate "monster struct":
the CHA block **is** a character record, identical in layout to a `CHRDAT` save
record. The real files are present as `MON{1..6}{CHA,SPC,ITM}.DAX` (six areas ×
three files), each a standard DAX archive whose block `id` = the monster id.

This is a clean reuse point: `gbx_formats::save_orig::decode_char_record`
(`CHAR_RECORD_SIZE = 0x1A6`) already decodes this exact record, including every
combat field (`ac@0x19a`, `thac0_base@0x73`, `hit_dice@0xe5`, `field_e9@0xe9`,
`monster_type@0x11a`, `control_morale@0xf7`, `attack_profile_current@0x19c` =
the 8-byte attack1/2 [left,count,size,bonus] run). The monster loader decodes CHA
blocks with the same function; SPC/ITM reuse the existing 9-byte / `Item.StructSize`
blob splitters.

### 8.2 FD-20 (turn-undead type ≥ 11) — the field and the census target

`turns_undead` (`ovr014.cs:608+`) indexes a flat table
`unk_16679[(target.field_E9 * 10) + clericBand]` (`ovr014.cs:642`), where
`field_E9` = the target monster's `@0xe9` byte and `clericBand` ∈ ~[1,10]. The
image stores exactly **11** undead-type rows (types 0–10) at `0xaf4a` (FD-20,
already pinned); row 11 would be string-table bytes. So the open half of FD-20 is
purely a data question: **does any real `MON*CHA` record carry `field_e9` ≥ 11?**
(For CotAB monsters this is read directly from `@0xe9`; the `ovr017.cs:286`
"`field_76`" the docket mentioned is the *PoolRad-import* conversion
`player.field_E9 = poolRad.field_76`, a different source record — not the CotAB
monster read.) **Deliverable-2's local-tier census reads byte `0xe9` of every
`MON{1..6}CHA` block** and reports the max, resolving FD-20 against real data.

### 8.3 FD-29 (data-driven `roll_dice` extents) — what to enumerate

The migration ledger's open FD-29 clause is the **data-driven** `roll_dice`
extents (its literal-site census is done). From the monster data the enumerable
extents are:

- **Monster damage dice:** `roll_dice(attackDiceSize(idx), attackDiceCount(idx))`
  (`ovr014.cs:86`), i.e. per attack profile `count = @0x19e/@0x19f`,
  `size = @0x1a0/@0x1a1`. Max single-roll total = `count · size`; truncation to a
  byte (coab `(byte)roll_total`, `ovr024.cs:595`) is observable only if any
  `count · size > 255`.
- **Monster hit dice:** `hit_dice = @0xe5`. (HP itself is stored as
  `hit_point_max@0x78`, a byte, so HP isn't re-rolled from data at load; the
  class-indexed `unk_1A8C4/unk_1A8C3` hit-dice roll the ledger names is a
  creation-time table path, not per-record data.)

**Deliverable-2's census computes, across all real `MON*CHA` records,
`max(count·size)` over both attack profiles and the observed `hit_dice` range,
and states whether any exceeds 255.** That closes FD-29's data-driven clause for
monsters (weapon damage dice — the `ItemData` path — remain for the item-table
session, M5-adjacent).

---

## 9. Action-profile event vocabulary (`init`/`pick` pinned; rest PROPOSED — D-OR3)

> **`init`/`pick` PINNED (2026-07-16, initiative slice); `attack`/`dmg`/`save`
> PINNED (2026-07-16, attack slice).** D-OR3 leaves the `action`-profile
> *vocabulary* for the combat sessions to pin as each system lands; its
> *mechanism* (profile tag, canonical field order, same-tick emission order)
> already exists in `gbx-oracle`. The `move`/`ai`/`status`/`morale`/`award` rows
> are still a strawman. All values are integers (D-OR3 canonical encoding —
> `surprise`/`hit`/`backstab`/`made` are `0`/`1`, not bools); `combatant_id` is a
> stable per-encounter index into the roster. Equality over action events is
> **not** yet a gate, even for the pinned rows — pinning fixes the field
> names/order, not a comparison. **The pinned `attack`/`dmg`/`save` field sets
> were trimmed from the strawman below** (per the attack-slice brief) to the
> observable roll + outcome — the pinned canonical forms are: `attack` =
> `{attacker_id, target_id, roll, hit}` (`roll` is the raw d20, 1..=20, *before*
> the nat-20→100 promotion), `dmg` = `{attacker_id, target_id, amount, backstab}`,
> `save` = `{combatant_id, save_type, roll, made}`. The strawman rows kept below
> record the fuller field ideas for reference; `gbx-oracle`'s
> `AttackEvent`/`DmgEvent`/`SaveEvent` are the canonical pinned forms.

| `e` | Fields | Emitted when | Draws it brackets |
|---|---|---|---|
| `init` **✓ pinned** | `combatant_id, delay, dex_adj, surprise` (canonical order) | per combatant in `CalculateInitiative` | the one `random(6)` |
| `pick` **✓ pinned** | `pass, combatant_id, delay, roll` (canonical order) | **per `FindNextCombatant` selection** (one per yielded combatant, not per member) | brackets a whole pass's `random(100)`s |
| `move` | `combatant_id, from{x,y}, to{x,y}, cost` | each step in the AI/menu move | none (movement is RNG-free) |
| `attack` **✓ pinned** | pinned: `attacker_id, target_id, roll, hit` (strawman: `+attack_idx, bonus, target_ac`) | each `PC_CanHitTarget` (weapon) / `CanHitTarget` (effect) | one `random(20)` |
| `dmg` **✓ pinned** | pinned: `attacker_id, target_id, amount, backstab` (strawman: `dice_count, dice_size, bonus, backstab_mult, total`) | each `sub_3E192`, on a hit | `dice_count` × `random(dice_size)` |
| `save` **✓ pinned** | pinned: `combatant_id, save_type, roll, made` (strawman: `+bonus, target`) | each `RollSavingThrow` | one `random(20)` |
| `ai` | `combatant_id, branch, spell_id?, item?` | each QuickFight branch taken | branch-dependent |
| `status` | `combatant_id, from, to` | each `health_status` transition | none |
| `morale` | `combatant_id, monster_morale, enemy_hp_pct, roll, failed` | each `FleeCheck`/advance gate | 0 or 1 `random(100)` |
| `award` | `xp_total, treasure_ids[]` | `AfterCombatExpAndTreasure` | (see §10) |

Emission-order contract to honor when pinned: same-tick events emit in the order
the original consumes their draws (init before picks; a turn's `ai`→`attack`→`dmg`
in resolution order), so the `action` stream and the `prng` stream stay index-
alignable.

---

## 10. XP and treasure award path

`AfterCombatExpAndTreasure` (`ovr006.cs:763`, `sub_2E7A2`), reached from
`CMD_Combat` (`ovr003.cs:992/1006`):

```
CleanupPlayersStateAfterCombat / DeallocateNonTeamMembers
if (!party_killed || duel):
    if (party_fled) items_pointer.Clear()
    distributeNpcTreasure()
    displayCombatResults(gbl.exp_to_add)         ← XP shown/awarded
    distributeCombatTreasure()
    items_pointer.Clear()
else: "party destroyed" path
```

`gbl.exp_to_add = calc_battle_exp()` (`ovr006.cs:251/359`) — XP is computed **at
combat end** from the defeated roster (not accumulated per-kill), then distributed.
Treasure distribution (`distributeCombatTreasure` / `distributeNpcTreasure`) is the
TREASURE-table path (FD-5) and the monster `ITM` carry — its RNG (item rolls) is
out of scope here and lands with the item/treasure session.

---

## 11. Combat-map generation (battlefield from where the party was)

> **IMPLEMENTED — the tactical-battlefield slice (2026-07-16, D-OR5(a) Phase 1,
> third slice; algorithm faithful, real-area wiring deferred).** The map,
> placement, and movement geometry are transliterated in `gbx-engine`'s `combat`
> module: a 50×25 `CombatMap` with per-tile `TilePassability`
> (Passable/Wall/Void) from the `BackGroundTiles` `move_cost` table; the full
> `PlaceCombatants`/`place_combatant`/`try_place_combatant` fan-out (team origins,
> `half_team_count`, the `unk_16620` mask, the tri-state left/right walk, the iso
> transform, occupancy rebuild) with **exact-position tests** (party member 0
> hand-derives to (27,13)); and `CalcMoves`, the `sub_3E748` step-cost model
> (diagonal ×3 / orthogonal ×2), `getTargetDirection` (the 8-octant classifier),
> and `grid_distance`/`is_adjacent`. **The whole path is draw-free as this study
> scoped it** — `SetupGroundTiles`/`PlaceCombatants`/`CalcMoves`/`sub_3E748` make
> zero `Random` calls (a test asserts zero draws over setup, D9); no `gbx-prng`
> call was added. **Two caller-read corrections (coab over names):**
> `CanSeeTargetA` (`ovr014.cs:571`) is an **invisibility affect check, not
> geometric LoS** (documented; no fake LoS added — it belongs with affects); and
> the engine's authoritative combat *range* is the **wall-respecting flood**
> `Rebuild_SortedCombatantList` (`ovr032.cs:228`, `getTargetRange`=`steps/2`),
> which is target-selection's core and is **deferred to the AI slice** — this
> slice exposes the open-ground king-move `grid_distance` as the geometric
> primitive. **Deferred real-area hook (below):** the *derivation of the combat
> floor from the source area's wall topology* (`SetupGroundTiles` →
> `build_background_tiles_*` → `get_dir_flags`) and the `COMBAT`-opcode →
> `BattleSetup` roster assembly are the later encounter-trigger slice; here the
> map is built from a provided terrain descriptor and the area→wall-flags input is
> a caller `dir_flags` hook defaulting to open ground.

`BattleSetup` (`ovr011.cs:1169`, `battle_begins`) builds the battlefield:

```
combat_round = 0; combat_round_no_action_limit = 15; attack_roll = 0
SetupGroundTiles()        ← battlefield terrain sampled around the party's map cell
SetupCombatActions()
PlaceCombatants()         ← team_start offsets + tri-state fan-out placement
missile_dax = new DaxBlock(1,4,3,0x18)
mapToBackGroundTile.mapScreenTopLeft = PlayerMapPos(TeamList[0]) - ScreenCenter
calc_enemy_health_percentage()
```

**The battlefield derives from the party's world position + facing** (PLAN's
"terrain based on where the party was"):

- Team origins: `team_start_x/y[0] = 0` (party); the enemy team is offset by
  **`encounter_distance · MapDirection{X,Y}Delta[mapDirection]`** (`ovr011.cs:1063-1068`)
  — i.e. the enemies start `encounter_distance` tiles ahead of the party along the
  facing direction. `encounter_distance` (`area2_ptr`) is the approach range
  (decremented by `CMD_Approach`, `ovr003.cs:300`; clamped in `CMD_Combat`,
  `ovr003.cs:997-1002`).
- Absolute cell = `team_start[team] + {mapPosX, mapPosY}` (`ovr011.cs:984-985`).
- Within a team, `PlaceCombatants` fans combatants out with a left/right
  tri-state walk (`ovr011.cs:942-966`) bounded to the combat grid
  (`0 ≤ x ≤ 10, 0 ≤ y ≤ 5`, `ovr011.cs:969`), rows scaling outward
  (`half_team_count`, `ovr011.cs:974`).
- `CombatMap[]` (`Gbl.cs:506`, `seg600:66BD`) holds only `{pos, size, screenPos}`
  per cell — geometry, not combatant identity (D-OR5(b): the fixed-address combat
  array is grid geometry only).

The full `PlaceCombatants` body (the tri-state fan-out, the `unk_16620` mask, the
iso transform, `size_footprint`/occupancy) is now **transliterated and tested**
(see the §11 status banner); the **derivation** (position + facing +
encounter_distance → team origins → fan-out) this study pinned held on the read.
Two pieces remain a *later* read: the `SetupGroundTiles` wall-painting from the
source area (`build_background_tiles_*` ← `get_dir_flags`), deferred with the
`COMBAT`-opcode → `BattleSetup` real-area wiring; and mounted/large-monster sizing
beyond the `field_DE & 7` footprint (`size_footprint` supports sizes 0–4 already,
but the record fields that *set* a size > 1 are an item/mount-session read).
Note `encounter_distance` is clamped to the forward line-of-sight ray
(`sub_304B4`, `ovr003.cs:997-1002`), so the iso diamond fits the 50×25 field only
for the small distances a real approach yields — large synthetic distances push a
team off-map, which is expected, not a bug.

---

## 12. FD-1..FD-4 — coab-evidence, filled

| FD | Question | coab evidence (this study) | Status | Settles via |
|---|---|---|---|---|
| **FD-1** | nat-20 auto-hit / nat-1 auto-miss? | **Both exist.** `CanHitTarget` `ovr024.cs:487-508` + `PC_CanHitTarget` `:515-548`: `attack_roll > 1` gate (nat-1 miss), `==20 → 100` (nat-20 hit). Saves too (`RollSavingThrow` `:564-571`). **Implemented** (attack slice); caller read confirms the `>=` path is the weapon path (`AttackTarget01`), the `>` path the scripted-effect path (`CMD_Damage`). | narrowed → coab settled → **implemented** | H4 edge-roll traces |
| **FD-2** | initiative formula | `d6 + DexReactionAdj`, clamp[1,20], `-6` team surprise (`CalculateInitiative` `ovr014.cs:31-47`); draw-order = per-pass d100 (`FindNextCombatant` `ovr009.cs:72`). Consumed, never persisted. | narrowed → **settles by draw-order parity (D-OR5(a))** | Phase-0/1 draw stream |
| **FD-3** | attacks-per-round | `ThisRoundActionCount` `ovr014.cs:519-527` = `(halfActions + oddRound)/2` (3/2 mechanic); ranged from item `numberAttacks` (`:473-480`); two attack profiles. | narrowed | H4 round-count + `attacks_left` checkpoint |
| **FD-4** | sleep/held auto-kill | Affect-gated (`CheckType.Type_16` in `CanHitTarget` `ovr024.cs:500`); `TrySweepAttack` vs `HitDice==0` (`ovr014.cs:530-534`). Exact auto-kill leaf not yet read. | narrowed | melee-leaf read + curated 1v1 H4 |

---

## 13. Phase-0 capture runbook (the session's most-consumed output)

**Goal:** capture the original's `prng` draw stream over one **observe-only,
all-AI** fixed encounter, per D-OR5(a) Phase 0 — the only scripted input is the
keystroke(s) that start the fight and hand it to the AI, so `AUTOTYPE` cannot
desync. The rig, hook, and reader already exist (H3 is closed); this is the
combat-specific invocation.

### 13.1 Preconditions (all in place)

- **Hook branch:** `~/src/goldbox-refs/dosbox-staging`, branch `restrike-hook`
  (`@ e7138cb` after the part-B fix), triggers on RandNext entry/exit + Randomize,
  emits the `.gbxtrace` `prng` profile (`before` pre-instruction, `after`
  read-back from `DS:0x47F0`). Never pushed, never in the restrike repo (D10).
- **Reader:** `restrike trace-compare … --chain` (`gbx-oracle`) validates
  chain continuity and flags any post-poke `randomize`.
- **Seed control:** `RESTRIKE_SEED=<u32>` arms the one-shot poke at first draw and
  suppresses pre-poke events; `RESTRIKE_ENCOUNTER=<label>` sets the header label.
- **Save:** GOG bundled slot A — a 6-member party at Tilverton (area 2, pos (7,13)),
  imported clean by restrike's real-save path.

### 13.2 Which encounter

**Prefer the Tilverton-sewers fight** (PLAN's M4 exit-gate encounter; area 2, so
its monsters are `MON2CHA/SPC/ITM.DAX`). Reachability from the bundled slot-A save
(the walk route):

- The party starts in Tilverton City (GEO block 1, which also packs the Thieves'
  Guild side-by-side — FD-16). The Guild's bottom edge has **two** "exits to
  Tilverton Sewers" at absolute columns **10 and 14** (FD-16, confirmed by the
  border-opening topology match).
- Route: reach the Thieves' Guild half of block 1, take one of the two sewer exits
  at the bottom edge into the sewers, and trigger the scripted sewers encounter.

> **Confirm at the display.** restrike does not yet implement the cross-area GEO
> transition that the Guild→Sewers exit performs (FD-19), so the exact in-game
> step sequence must be walked in DOSBox and recorded during the session — this
> runbook names the geography (block 1, bottom-edge exits at cols 10/14), not a
> keystroke-exact path. **Fallback if the sewers fight is awkward to reach
> unarmed:** any fixed (scripted, non-random) area-2 encounter reachable early
> serves Phase 0 equally — the requirement is *fixed monsters + observe-only*, not
> that specific fight. Log which encounter was used in `RESTRIKE_ENCOUNTER`.

### 13.3 Engaging QuickFight in-game (all-AI)

From the `SetPlayerQuickFight` read (`ovr009.cs:707`) and the combat menu
(`ovr009.cs:149-306`):

- When combat starts, each PC's turn shows the combat menu ending in **"Quick
  Done"**. Pressing **`Q`** (the `'Q'` case, `ovr009.cs:177`) calls
  `SetPlayerQuickFight(player)` then `PlayerQuickFight(player)` — that PC and its
  subsequent turns run under AI.
- To hand the **whole party** to the AI at once, the special-key path at
  `ovr009.cs:267` (`(char)0x10`) sets `delay = 20` and calls
  `SetPlayerQuickFight` for **every** member — a party-wide "everyone quick-fights"
  toggle. (Monsters already run `PlayerQuickFight` unconditionally, `ovr009.cs:134`.)
- **Simplest Phase-0 procedure:** enter the fight, press `Q` on each PC's first
  prompt (or the party-wide key) so all PCs are quick-fighting; from then on every
  turn — both teams — is AI, and no further input is needed until the fight ends.

### 13.4 Armed rig invocation

Launch under the harness (background) so the window appears on Bryan's display and
the harness is notified on quit (the part-B lesson: a `!`-prefix launch of the
compound env-var command silently no-ops):

```
cd ~/src/goldbox-refs/dosbox-staging
RESTRIKE_TRACE=~/goldbox-data/traces/phase0-sewers.gbxtrace \
RESTRIKE_SEED=0x0C0FFEE0 \
RESTRIKE_ENCOUNTER=tilverton-sewers \
./build/dosbox -conf restrike-rigs/rig-partb.conf
# (rig-partb boots past copy protection with a blind 'Z'; then: load slot A,
#  walk to the encounter, start it, press Q for all PCs, let it run to the end.)
```

Then validate and archive (traces stay under `~/goldbox-data`, D10 — never the repo):

```
restrike trace-compare ~/goldbox-data/traces/phase0-sewers.gbxtrace --chain
# expect: OK (N draw event(s)), exit 0, zero post-poke randomize
```

### 13.5 Expected draw-stream shape (what a good capture looks like)

Per §2, one round should read as: a block of **`random(6)`** draws (one per
in-combat combatant, initiative) followed by **`random(100)`** picks in groups of
`K = TeamList.Count` (one selection pass), each group separated by the selected
combatant's own turn draws — for an AI turn typically a morale `random(100)` (§6.2),
then per attack a `random(20)` to-hit (§5.2) and `dice_count × random(dice_size)`
damage (§5.3), plus any spell/save `random(20)`s. `n` (the wrapper operand) is
recoverable per draw from the hook's `ss_sp_words[3]` diagnostic, so the draw
*kinds* (d6 vs d100 vs d20 vs dN) are visible in the capture — a first sanity check
that the stream matches this shape before any Restrike replay exists.

Chain continuity (`--chain`) must hold on every link and there must be **zero**
post-poke `randomize` events; a break there means a dropped `AUTOTYPE` keystroke or
a foreign write, i.e. a *detected re-run*, not silent corruption (D-OR4 part B's
guarantee, now reused for combat).

### 13.6 What Phase 0 feeds

The captured `.gbxtrace` is the Phase-1 oracle: the implement-to-parity session
builds the round loop (§1), initiative (§2), action economy (§3), AI tree (§4),
and to-hit/damage (§5) until Restrike's headless replay reproduces this draw order
for ≥10 seeds (D-OR5(a)). AI-decision parity closes there; only then does Phase 2
(scripted player turns) begin.

---

## 14. Surprises worth flagging (H4 landmines)

1. **`FindNextCombatant` re-rolls d100 for the *entire* `TeamList` on every pass,
   including delay-0 and dead members** (`ovr009.cs:70-72`). The per-round d100
   count is `(A+1)·K`, not `A` — a naive "roll once per actor" implementation
   desyncs the stream immediately. This is the single most important draw-count
   fact for Phase 1.
2. **A monster is a full 0x1A6 `Player` record** — no separate monster struct. The
   save-record decoder already covers it; combat should not invent a parallel
   layout.
3. **`ThisRoundActionCount` folds the 3/2-attacks rule into a `combat_round`
   parity test** (`ovr014.cs:521`), so attack counts depend on round number — a
   replay that resets round parity will diverge on multi-attack combatants.
4. **`CanHitTarget` uses `>` but `PC_CanHitTarget` uses `>=`** (`ovr024.cs:504`
   vs `:544`) — the two hit tests are not symmetric; an off-by-one here is
   invisible until an exact-AC edge case in H4. **Which is which (caller read,
   attack slice):** the `>=` path (`PC_CanHitTarget`) is the **weapon-attack path
   for both teams** (`AttackTarget01`, `ovr014.cs:821`); the `>` path
   (`CanHitTarget`) is the scripted `DAMAGE`-opcode / area-effect path
   (`CMD_Damage`, `ovr003.cs:1673`). So a normal melee/ranged swing is `>=`; the
   strict `>` only bites on scripted damage effects. (Supersedes §5.2's
   "monster/generic vs PC" framing.)
5. **The QuickFight `field_15` target-mode is itself RNG-gated** (`ovr010.cs:22`)
   — a d4 gate, then d8→(d4|d2) — *before* any target selection, so the AI turn's
   first draws are mode-selection, not the attack. Easy to miss when counting
   draws. **AI-slice correction:** the d4 gate `||`-**short-circuits** when
   `field_15∈{0,4}` (which includes every combatant's *first* turn, since
   `field_15` starts 0) — so that turn draws **d8+(d2|d4) = 2**, not 3
   (§4.1.2). And two *unconditional-in-a-normal-area* d7s precede target
   selection that the strawman map missed: `sub_354AA:192` (the wand-scan
   priority roll — fires because `area.can_cast_spells==false` means casting is
   *allowed*, §4.1.1) and `sub_3560B:248` (the memorized-spell priority roll,
   drawn *before* the `spells_count>0` guard). A spell-less item-less fighter
   still draws **both**. See §4.1 for the full ordered map.

7. **Movement is not unconditionally draw-free.** Each **monster** approach step
   draws a d100 (the `moralFailureEscape:387` morale-advance gate; PCs
   short-circuit it), and once melee is joined, stepping away from / into
   adjacency triggers opportunity attacks (`move_step_away_attack` /
   `move_step_into_attack`, each a full attack's worth of draws) — §4.1.4/§4.1.6.
   The slice-3 claim that "movement geometry is draw-free" holds only for the
   *geometry primitives*, not the AI move path.
6. **XP is computed at end-of-combat, not per kill** (`calc_battle_exp`,
   `ovr006.cs:251`) — the `award` event fires once, not incrementally.

---

*Sources: coab (read-for-behavior, D11, never copied) — `ovr003.cs`, `ovr006.cs`,
`ovr009.cs`, `ovr010.cs`, `ovr011.cs`, `ovr014.cs`, `ovr017.cs`, `ovr024.cs`,
`Classes/Action.cs`, `Classes/Gbl.cs`, `Classes/Player.cs`,
`Classes/Combat/CombatantMap.cs`. Cited inline by `file:line`. Restrike:
`gbx-formats/src/save_orig.rs` (the shared 0x1A6 record decoder),
`gbx-oracle` (trace mechanism), `docs/design/oracle-rig.md` (D-OR3/D-OR5),
`docs/fidelity-docket.md` (FD-1..4/20/29). See `SOURCES.md`.*

## 15. Live confirmation — first Phase-0 combat capture (2026-07-16)

First combat draw-stream captured from the real game (a Tilverton brawl, all-AI
via Quick; armed rig, seed `0x0C0FFEE0`; 3,577 draws, chain-continuity OK, zero
post-poke reseed; trace archived local-only per D10, sha256 `388347e2…`). Each
draw's operand `N` was recovered from the hook's `ss_sp_words[3]` diagnostic
(the wrapper reads `N` at `ss:[bx+4]`), so the operand histogram is directly
readable. Two behavioral claims from this study are now empirically confirmed
against the running original, *before* any combat code exists:

- **§4 initiative loop — CONFIRMED.** `N=100` dominates at **2,503 draws**,
  and run-length encoding the consecutive-`N=100` bursts gives **152 bursts of
  length exactly 16** (of 168 total). That is `FindNextCombatant` rolling one
  d100 **per roster member, every selection pass**, over a stable
  **16-combatant** roster — the `(picks+1)×roster` blow-up §14's landmine
  predicted from `ovr009.cs:59-99`, seen in the wild. The handful of short
  bursts (len 1–12) are other d100 consumers (morale/flee/percentage checks) or
  round-boundary passes, not the initiative loop. A combat implementation that
  rolls initiative any other way desyncs on round 1.

- **The `N=101` draws are the ECL `RANDOM` opcode — CONFIRMED, and it
  corroborates the step-1 off-by-one fix.** All 96 `N=101` draws sit **outside**
  the combat region (a tight pre-combat cluster, boot/exploration), and cluster
  next to each other. `RANDOM` (`CMD_Random`, `ovr003.cs:132-151`)
  pre-increments its operand (`rand_max++` unless `0xFF`) before calling
  `seg051.Random`, so a script `RANDOM x,100` becomes `Random(101)` — exactly
  the increment `machine.rs` `op_random` implements and the §6 migration ledger
  (rows 1/2) fixed. So the RANDOM-opcode operand handling is now validated on
  real data too, not just against coab.

**Caveat (why this is first-light, not the canonical Phase-0 golden):** the
brawl's roster/config is not a known fixed encounter, and full *replay* parity
needs the combat entry state (RNG state at combat start — recoverable from the
trace — **plus** the combatant roster + delays, which need the D-OR5(b)
structure walk). This capture validates the initiative *shape* and the RANDOM
operand, and is a real target for the first combat session's synthetic parity
tests; a canonical fixed-encounter capture (known roster) is still wanted before
a full-fight replay golden is locked.
