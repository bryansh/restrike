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

## 5. To-hit, damage, saves (feeds FD-1 / FD-4)

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

## 9. PROPOSED action-profile event vocabulary (NOT pinned — D-OR3)

> **PROPOSED ONLY.** D-OR3 leaves the `action`-profile *vocabulary* for the combat
> sessions; its *mechanism* (profile tag, canonical field order, same-tick
> emission order) already exists in `gbx-oracle`. These fields are a strawman for
> the Phase-1 implementer to pin when each system lands — they are **not** a
> format commitment, and equality over them is not yet a gate. All values are
> integers (D-OR3 canonical encoding); `combatant_id` is a stable per-encounter
> index into the roster.

| `e` | Proposed fields | Emitted when | Draws it should bracket |
|---|---|---|---|
| `init` | `combatant_id, delay, dex_adj, surprise` | per combatant in `CalculateInitiative` | the one `random(6)` |
| `pick` | `pass, combatant_id, delay, roll` | per member per `FindNextCombatant` pass | one `random(100)` |
| `move` | `combatant_id, from{x,y}, to{x,y}, cost` | each step in the AI/menu move | none (movement is RNG-free) |
| `attack` | `attacker_id, target_id, attack_idx, roll, bonus, target_ac, hit` | each `CanHit`/`PC_CanHit` | one `random(20)` |
| `dmg` | `attacker_id, target_id, dice_count, dice_size, bonus, backstab_mult, total` | each `sub_3E192` | `dice_count` × `random(dice_size)` |
| `save` | `combatant_id, save_type, roll, bonus, target, made` | each `RollSavingThrow` | one `random(20)` |
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

The full `SetupGroundTiles` / `PlaceCombatants` bodies (grid sampling, wall
handling, mounted/large-monster sizing) are an implementation-session read; the
**derivation** (position + facing + encounter_distance → team origins → fan-out)
is what this study pins.

---

## 12. FD-1..FD-4 — coab-evidence, filled

| FD | Question | coab evidence (this study) | Status | Settles via |
|---|---|---|---|---|
| **FD-1** | nat-20 auto-hit / nat-1 auto-miss? | **Both exist.** `CanHitTarget` `ovr024.cs:487-508` + `PC_CanHitTarget` `:515-548`: `attack_roll > 1` gate (nat-1 miss), `==20 → 100` (nat-20 hit). Saves too (`RollSavingThrow` `:564-571`). | narrowed → coab settled | H4 edge-roll traces |
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
   vs `:544`) — the monster and PC hit tests are not symmetric; an off-by-one here
   is invisible until an exact-AC edge case in H4.
5. **The QuickFight `field_15` target-mode is itself RNG-gated** (`ovr010.cs:22`)
   — a d4 gate, then d8→(d4|d2) — *before* any target selection, so the AI turn's
   first draws are mode-selection, not the attack. Easy to miss when counting
   draws.
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
