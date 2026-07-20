# H4 closure for melee: the combat entry-state snapshot (D-OR5(b))

Status: **v1 plan** (Fable-authored 2026-07-17). Implements D-OR5(b) from the
review-closed `oracle-rig.md` (this is an implementation plan, not a door
change). It is the *last* piece before H4 (combat trace equality) closes for
melee: every combat mechanic is built and self-consistent (a draw-parity
artifact through combat #1–#6), and a fight is now triggered by a running ECL
script. What remains is to prove our combat draw stream equals the **original's**
for a real encounter — which needs the original's combat entry state so we can
seed our engine identically.

## 0. Why a snapshot at all

Our combat is deterministic from (roster, positions, RNG state) — everything
after is faithful engine logic (initiative → AI → attacks), already
draw-verified against coab. So to replay an original fight we need exactly those
three inputs as the original had them at combat start. The RNG state our hook
already captures (`DS:0x47F0`). The roster + positions are the new snapshot.

## 1. The key simplification (from the coab structure read)

At `MainCombatLoop` entry (`ovr009.cs:22`), **before round 1**, the per-combatant
`Action` fields the AI keys on are still at their initial values:
- `Action.delay` (offset 0x03) = **0** — `CalculateInitiative` rolls it *inside*
  round 1 (our engine does the same d6+DexReact).
- `Action.field_15` (offset 0x15) = **0** — the AI mode-gate; the turn-1 gate
  `if (field_15 == 0 || …)` always rerolls precisely because it starts 0 (combat
  #4 finding). Our engine reproduces that reroll.

So **the snapshot does NOT need `delay` or `field_15`** — our faithful engine
regenerates them from the seed. The snapshot is only the *static* entry state:

  **snapshot = { for each combatant: team, grid position, the combat-relevant
  stats } + the RNG state dword at entry.**

## 2. What to read, and where (coab annotations — runtime addresses resolved live)

- **Roster:** `gbl.TeamList` (`Gbl.cs:496`, original `player_next_ptr` linked
  list) — party (team 0) then loaded monsters (team 1), in draw/iteration order
  (load-bearing: initiative and `FindNextCombatant` iterate this order).
- **Per-combatant stats:** each entry is a full **0x1A6 `Player` record** — the
  *same* format `gbx_formats::save_orig::decode_char_record` /
  `gbx_formats::monster` already parse. The combat engine reads: `team`
  (`combat_team`), `hit_point_current`/`max`, `ac` (@0x19a, descending), `hitBonus`
  (@0x199), the attack dice (@0x19e..0x1a1), DEX (for the reaction adj),
  `field_186` (save bonus @0x186), class/level (backstab, later). Dumping the
  whole record is simplest and future-proofs later slices.
- **Positions:** `gbl.CombatMap[index].pos` (`CombatMap`/`stru_1C9CD`, `Gbl.cs:506`,
  original `seg600:66BD`), `CombatantCount` = `stru_1C9CD[0].field_3`. `Player`
  carries its `actions` pointer at 0x18d; position is *not* in `Player`/`Action`
  — it is in `CombatMap`, indexed by `player_index`.
- **RNG state:** `DS:0x47F0` — already captured by the existing hook.

## 3. Trigger (resolve the code address live, like RandNext)

Dump the snapshot **once, at the transition into `MainCombatLoop`, after
`BattleSetup` (map + placement) but before the first initiative draw.** Two
candidate triggers (pick during the staging session):
- **A write-watch on `combat_round`** (`byte_1D8B7`): `BattleSetup` sets it to 0
  right before `MainCombatLoop`; snapshot on that write. Data-address trigger,
  no overlay-code-address needed.
- **The first `RandNext` hit after that `combat_round=0` write** — reuses the
  existing hook; the first combat draw is the round-1 initiative d6, so the
  state is fully placed and unrolled at that instant.
The write-watch is cleaner (independent of the RNG hook); prefer it if the
DOSBox build's memory-watch path is scriptable, else use the first-RandNext
approach.

*(Note: `MainCombatLoop` lives in an overlay (GAME.OVR), so its code address is
load-dependent — another reason to trigger on the resident data global
`byte_1D8B7` rather than an overlay code offset.)*

## 4. Trace format (extends `.gbxtrace`, additive per D-OR3)

A new event, emitted once before the fight's rng events:
```
{"e":"combat_entry","seed":<u32 rng state>,"combatants":[
   {"team":0|1,"x":<u8>,"y":<u8>,"record":"<hex of the 0x1A6 bytes>"}, … ]}
```
D10 note: the `record` hex is **real character/monster bytes** → this trace is
**local-only** (never in the repo/CI), same posture as the H3 captures. The
`gbx-oracle` reader learns this one additional event type; the comparator ignores
it for rng-stream equality (it is *input*, not a draw).

## 5. Capture target: a DUNGEON or CITY fight, NOT wilderness

Combat #6 finding: `SetupWildernessFloor` **draws** (terrain scatter) but
`SetupDungeonFloor` is **draw-free**. A wilderness fight's stream would begin with
terrain draws we haven't implemented (deferred); a dungeon/city fight's stream
begins cleanly with the round-1 initiative d6 — which is what our engine emits
first. So the canonical capture is a **dungeon/city** encounter: the Tilverton
**sewers** fight (PLAN's exit-gate encounter) or any scripted Tilverton **city**
fight (the bar-brawl Phase-0 capture already showed city combat opening with d6
initiative — consistent with the city using the draw-free floor).

## 6. Replay + the H4 assertion

1. Capture: original under the extended hook → `combat_entry` snapshot +
   the fight's `prng` stream (chain-continuity-validated).
2. Replay: seed `gbx-prng` with the snapshot's RNG state; build a `CombatState`
   from the snapshot roster (decode each 0x1A6 record) + positions + teams; run
   the unified tick engine to `Ended` with an `RngSink`.
3. Assert: our draw stream **equals** the captured `prng` stream, draw-for-draw
   (the D-OR3 comparator). That is H4 melee closure — for N seeds if the
   encounter can be re-entered with a poked seed.

## 7. Open items for the Bryan+Fable staging session

- **Runtime addresses (resolved live, like RandNext was):** the `byte_1D8B7`
  (`combat_round`) data address; the `CombatMap`/`stru_1C9CD` base and stride
  (to read `pos` + walk `CombatantCount`); the `TeamList` head + `player_next_ptr`
  offset **or** whether walking `CombatMap[0..CombatantCount]`'s player pointers
  is simpler than the `TeamList` chain.
- **The hook extension** (a bigger patch than the RandNext hook: a
  memory-walk that reads N records at the trigger) — a staging-branch change,
  drafted by a session, audited, then run by Bryan at a display.
- **Reaching a scripted dungeon/city fight** from the bundled save (FD-19: the
  cross-area Guild→Sewers transition our engine doesn't do yet — but for the
  *capture* we only need the original game to reach it; the sewers are reachable
  in-game).
- **Melee-only encounter:** pick a fight with no spellcasters/ranged/special
  attacks (combat #4's stubs) so the whole stream is within the implemented
  melee path. Kobolds/thieves/guards qualify; avoid clerics/mages for the first
  closure.

## 8. What this closes, and what it doesn't

Closes: **H4 for the melee combat path** — our combat is proven bit-exact
against the original for a real encounter. Leaves for later (each already
scoped): faithful wilderness terrain (draw-bearing), spell/ranged/item/backstab
effects (M5), XP/treasure (`AfterCombatExpAndTreasure`), and then the graphical
combat renderer (the visible payoff, safe to build once H4 proves the logic).

## 9. Result (implemented 2026-07-17 — the first live replay)

The snapshot was captured (`~/goldbox-data/traces/h4-combat-barbrawl-2026-07-17.gbxtrace`,
D10 local-only) and replayed. Structure realized as planned: one `combat_entry`
event (`rng_state` + 16 combatants in `TeamList` order, each `team`/`x`/`y`/0x1A6
record) then the fight's 3,162-draw `rng` stream. The encounter is a **Tilverton
bar brawl** — 6 party (MATHEW/MARK/TRAVIS/LEDERA/SHARA/PHILIPPE, unarmed 1d2+STR
fists, hd 4–5) vs **10 identical BAR PATRONs** (16 HP, hd 2, morale 0x80 = NPC,
1d6 fists). A pure melee fight, exactly the melee-only target §7 asked for.

**Built (this session):**
- **Reader** (`gbx-oracle::format`): the `combat_entry` event — typed struct with
  hex-decoded `[u8;0x1A6]` records; the comparator + chain-check **ignore it**
  (it is replay input, not a draw). Synthetic CI tests.
- **Replay harness** (`gbx-engine::combat::combat_state_from_records`): decodes
  each record and builds a `CombatState` in the captured order at the captured
  positions (no `PlaceCombatants`), seeded from `rng_state`. The record→combat
  field mapping is documented on `combatant_from_record` (hp/ac/`hitBonus@0x199`/
  attack-1 dice/`DexReactionAdj(full DEX)`/`attacksCount@0x11c`/hd). Synthetic CI
  test.
- **Differential milestone** (`gbx-oracle/tests/h4_replay.rs`, local-tier, gated
  on `GBX_DATA_DIR`/`GBX_H4_CAPTURE` so plain `cargo test` skips it): runs our
  engine to `Ended` with an `RngSink` and compares draw-for-draw on `(before,
  after)`.

**Outcome — 2,995 / 3,162 draws match bit-exactly (94.7%), then a tail
divergence — H4 melee is NOT yet fully closed.** Our stream is an *exact prefix*
of the capture's for 2,995 draws (every `before` **and** `after` identical), so
the RNG seam, multi-round initiative, `FindNextCombatant` selection, to-hit,
damage, saving-throw and QuickFight-AI-turn draw *structure*, and morale-draw
*timing* are all validated across ~9 rounds of a real 16-combatant fight. That is
the oracle effort paying off.

**The divergence is a *length* difference at the very end, not a wrong roll:**
our fight ends (`PartyWins`) at draw 2995 — our per-round monster survivors go
10→10→8→8→7→6→3→3→1→0 (rounds 0–9) — while the capture continues **167 more
draws** into a further round. The first missing capture draw (#2995) is a
QuickFight-AI **d7**, and the tail contains a fresh **7-combatant** (6 party + 1
monster) initiative d6 burst: **the original keeps its last BAR PATRON alive and
acting for ~1 more round than we do.** Because every draw up to that point
matched (identical rolls, identical damage) and all records enter at `hpC == hpM`
(so HP-entry mapping is *not* the cause), this is a **draw-free combat-tail
decision**: our engine removes/finishes the final low-morale NPC one round early.

Leading hypothesis (for the next session — do **not** fix in the harness):
end-of-fight **morale/flee handling** (`FleeCheck_001` / `moralFailureEscape`) —
a BAR PATRON is a low-morale NPC (0x80) and combat #4 explicitly stubbed the
`Surrenders`/flee-removal branch ("a morale-failing NPC flees rather than
surrenders here"). The original likely keeps a fled/surrendering patron in combat
(still taking turns) where our stub removes it (→ `monsters == 0` → `PartyWins`).
Alternative: a death/removal-threshold or opportunity-attack application detail.
The harness re-verifies draw-for-draw the moment that tail is corrected — a clean
`H4 MELEE CLOSED: N draws matched` line replaces the finding.

## 10. The flee hypothesis, tested and REFUTED (2026-07-17, session 2)

§9's leading hypothesis (end-of-fight morale/flee) was investigated directly
against `FleeCheck_001` (`ovr010.cs:760`) and the live capture, and **it is not
the cause.** The flee/surrender outcome was implemented faithfully and it makes
the match **worse**, so it was reverted (the tree is pristine — no engine change
this session). The evidence:

- **`FleeCheck_001` re-seeds `gbl.monster_morale = (control_morale & 0x7F) << 1`
  *per combatant* (`ovr010.cs:774`).** Every BAR PATRON in the capture decodes
  `control_morale == 0x80` (`@0xf7`), so that seed is `0` for all ten — the first
  morale gate is then always taken via `== 0`, `monster_morale` becomes
  `enemyHealthPercentage`, and the inner gate fires the moment a single monster
  dies (`enemyHealthPercentage < 100`, round 2+). Enemies and monsters are
  equal-speed (`CalcMoves/2 == MaxOppositionMoves == 12`), so the branch taken is
  **panic** (`moral_failure`), not surrender. Result: implementing the reseed
  **routs the entire monster team from round 2** and the replay diverges at draw
  **1549** — a 1,446-draw *regression* of the 2,995 prefix.
- **Identical `control_morale` ⇒ the flee decision is all-or-none.** It cannot
  selectively keep *only* the last patron acting while the prefix (nine rounds of
  the same monsters fighting) stays intact. Any faithful flee change perturbs
  draws long before 2995.
- **The capture shows no routing.** Operand histograms of our clean 2,995-draw
  fight vs. the capture's 3,162 are nearly identical — d20 to-hits 111 vs 114, d7
  mode-gates 230 vs 253, initiative d6 165 vs 187 — i.e. both are *attack*-heavy;
  the capture is simply **~1 round longer**. The tail (§9's draws 2994-3161) is
  ordinary attack turns (`field_15` gate → the two d7s → d20 to-hit → damage),
  **not** `moralFailureEscape` flee turns (which would draw the `:400` d2 flee
  direction). The party keeps hitting a surviving id 11 for one extra round.
- **coab's RNG ≠ the capture's.** coab's `seg051.Random` is C# `System.Random`;
  the capture is the DOS binary's Turbo-Pascal LCG (what `gbx-prng` implements).
  `FleeCheck_001` is draw-free, so this doesn't change the flee *decision* — but
  it is a standing reminder that coab is a control-flow oracle, not a draw oracle.

**Restated finding (for the next session).** The last monster (`id 11`, entering
the final round at 6/16 HP) is finished **one round early** by our party's
cumulative attacks; the capture's party takes an extra round to land the kill.
Because draws match bit-for-bit to 2994 and all records enter `hpC == hpM`, this
is a **draw-free, endgame kill-timing** divergence in the *attack* path, not
morale. The two most likely levers, both **outside** the combat-#4 flee scope:

1. **The terrain free variable.** The replay runs on a uniform open floor
   (`FLOOR = 0x17`); the real bar map (`SetupGroundTiles`, not snapshotted) shapes
   `find_target`'s random near-target pick and wall-flood reachability. Different
   target selection ⇒ our party concentrates fire and kills id 11 a round sooner.
   (Bar/dungeon floor is draw-free per combat #6, so faithful terrain here would
   change *which* target, hence the tail length, without adding draws — the exact
   draw-free lever this divergence needs.)
2. **The FD-3 attack-count derivation.** `ThisRoundActionCount` /
   `attack{1,2}_left` (the 3/2-attacks rule) is the acknowledged single-profile
   simplification; an over-count would let the party out-damage the original and
   finish monsters early.

Recommended next step: instrument per-round *target selection* (which enemy each
party member picks) under uniform-floor vs. a faithful bar map, and audit the
party's `attack1_left`/`attack2_left` per round against `ThisRoundActionCount`, to
localize which lever moves the final kill from round 10 to round 11. The flee
branch is genuinely stubbed and worth finishing for M5 completeness, but it is
**not** what closes this H4 replay.

## 11. The terrain hypothesis, tested and ALSO REFUTED (2026-07-17, session 3)

§10's leading hypothesis (the uniform-floor replay vs the real bar map) was
tested directly: the hook was extended to capture the terrain grid
(`mapToBackGroundTile`, far pointer at `DS:0x6EAC`, 50×25 byte grid — landed and
verified on the staging branch, `7fd558d`), a fresh terrain-carrying bar brawl
was captured, and the replay built its `CombatMap` from the real grid. **It is
not the fix** — and the A/B test is decisive.

On the *same* terrain-carrying capture (seed `0x4b7e9837`, 16 combatants, 4,260
draws):
- **uniform floor:** our fight matches **3,620** draws before ending early.
- **real captured terrain:** our fight matches only **3,385** draws.

Real terrain matches **worse**, not better. Two things follow:
1. **Our wall-respecting targeting/movement (combat #3's `reach_ray`/
   `build_near_targets`/`step_cost`, tested only on synthetic maps) is NOT
   faithful on real iso-diamond terrain** — using the real walls diverges the
   fight *sooner* than ignoring them does. Either the tile-index→passability
   mapping or the wall traversal differs from coab on real data.
2. **A wall-independent divergence remains:** even on a uniform floor the fight
   ends ~1 round early (3,620 < 4,260). So the core residual is not terrain at
   all — it is a **draw-free endgame targeting-ORDER** difference: same rolls,
   same damage amounts, but our attackers concentrate damage on interchangeable
   targets slightly differently than the original (`find_target` picks
   `nearTargets[roll-1]`; if our `build_near_targets` *ordering* differs from
   coab's `BuildNearTargets`, the same roll picks a different target), so our
   last monsters die a round early.

**Two hypotheses (flee, terrain) are now refuted by evidence.** The pattern is
consistent — a draw-free endgame kill-timing/targeting divergence — but its exact
lever is a targeting-order/`build_near_targets`-ordering detail, plus an
unfaithful real-terrain wall-handling on top. This is a **dedicated instrumented
investigation**, not another guess: it needs the original's *chosen target* per
`find_target` roll (the current trace logs the roll, not its result), i.e. a
further hook extension to log the picked target, then a per-round targeting diff.

**H4 status (honest):** the combat **mechanics** are validated bit-exact against a
real ~10–11-round 16-combatant fight — initiative, `FindNextCombatant` selection,
to-hit, damage, saves, the AI mode-gate, and the morale *rolls* all match
draw-for-draw (2,995 on the first capture; 3,620 on the second's uniform run).
The residual is a draw-free targeting-**order** fidelity gap (which interchangeable
monster dies in which round), affecting no roll and no mechanic. Full draw-for-draw
closure (`N/N`) awaits the targeting investigation above; the mechanics claim
stands on its own.

## 12. The targeting mechanism, fully traced (2026-07-17, session 4) — reach + sort key are FAITHFUL

The residual (draw-free endgame kill-timing, §10/§11) is a **targeting-order**
divergence: `find_target` (`ovr014.cs:2238`) picks `nearTargets[roll-1]`, and the
*order* of `nearTargets` decides which interchangeable monster is hit. The order
comes from `BuildNearTargets` (`ovr025.cs:1290`) → `Rebuild_SortedCombatantList`
(`ovr032.cs:221`): for each enemy, the minimum reach over the size-footprint cells,
then `sortedCombatants.Sort()`.

**The reach (`canReachTargetCalc`, `ovr032.cs:92`, `sub_733F1`) is NOT a flood or a
plain ray — it is a Bresenham line-walk with a 3D elevation LoS:** `SteppingPath`
walks attacker→target (`+2` per step, `+3` on a diagonal advance) while a second
path tracks a flat height line at the attacker's tile elevation (`BackGroundTiles
[tile].field_1`); a tile blocks when its wall-height (`field_2`) exceeds that
elevation.

**The sort comparator (`SortedCombatant.CompareTo`, `Classes/Combat.cs`):**
`steps` asc, then `direction` asc; the `(direction%2)` branch only fires when
directions are already equal, so it is a **no-op** — the effective key is
`(steps, direction)`.

**Verified faithful in our engine (checked line-by-line, not assumed):**
- `combat::reach_ray` — its Bresenham (`delta_count += diff_minor*2`, threshold
  `>= diff_major`, `+2`/`+1` counting) and its elevation block
  (`TILE_WALL_HEIGHT > TILE_HEIGHT[attacker]`) match `SteppingPath.Step()` /
  `canReachTargetCalc` exactly.
- `build_near_targets` sort key (`steps` then `direction`) matches the comparator.

**So the two biggest suspects are ruled out.** The residual is cornered into three
subtle, draw-free candidates, none distinguishable by code reading:
1. **Sort *stability* on exact `(steps, direction)` ties.** coab's `List.Sort` is
   **unstable**; ours (`sort_by`) is **stable** — and neither necessarily matches
   the *binary's* sort (`sub_738D8`), which is the capture's ground truth.
2. **Movement** — combatants move draw-free (`sub_35DB1`/`step_cost`); a different
   landing cell drifts positions and hence targeting.
3. **`find_combatant_direction`** octant edge cases.

**Next: instrument, don't guess (two hypotheses already refuted).** Extend the hook
to emit a per-round snapshot of every combatant's `{index, team, pos, hit_point_
current}` at each `combat_round` increment. The replay snapshots the same per round
and diffs; the first divergent round + combatant localizes it — a `pos` divergence
points at movement (#2), an `hp` divergence at targeting (#1/#3). That converts
three suspects into one measured fact.

## 13. Targeting subsystem verified faithful; residual cornered to movement-vs-sort-tie (2026-07-17, session 4 cont.)

The per-round `round_snapshot` instrumentation (§12) localized the first divergence
to **round 1**: the same damage roll lands on a different *equidistant* monster
(capture's #13 vs our #11 take an 8-damage hit), and positions drift across the
whole roster. Then, line-by-line against coab, the **entire targeting subsystem was
verified faithful**: `reach_ray` (Bresenham + elevation LoS) == `canReachTargetCalc`;
the sort key `(steps, direction)` == `SortedCombatant.CompareTo` (its `%2` branch a
no-op); `find_combatant_direction` == `FindCombatantDirection`; and **all 8 octant
cases** of `can_see_combatant` == `CanSeeCombatant`.

Since every targeting *input* is faithful and positions start identical
(`combat_entry`), a divergent target can only arise from (a) **movement** — a mover
lands on a different cell, so a later `find_target` sees different positions — or
(b) **sort *stability*** on an exact `(steps, direction)` tie (coab `List.Sort`
unstable, ours stable, neither necessarily the binary `sub_738D8`). Movement is the
prime suspect (the one unverified piece, `sub_35DB1` pathing), but it is **measured,
not assumed**, by the next step: a **per-turn** `turn_snapshot` adding each
combatant's `{pos, hp, target}` (target via `actions`@record `+0x18D` → `Action.target`
@`+0x0A` → `player_array` index). The first divergent turn names it — `target` differs
with matching positions ⇒ sort-tie; `pos` differs ⇒ movement.

## 14. NAMED: the residual is the QuickFight AI turn body (coab ≠ binary), NOT the PRNG (2026-07-17, session 5)

Bryan re-captured a full bar brawl with `combat_entry` (now carrying **terrain**),
`round_snapshot`, and `turn_snapshot` (per-turn `{pos,hp,target}`) →
`~/goldbox-data/traces/combat4.gbxtrace` (seed `0x80ee4cee`, 16 combatants, 3075
draws, 11 rounds, 198 turn snapshots). The repo-side localizer is
`crates/gbx-oracle/tests/h4_turndiff.rs` (local-tier, D10-gated). It diffs three
ways — draw stream (operands, not just before/after), per-round board, per-turn
board — and it named the divergence precisely. Findings, in order of certainty:

- **The PRNG is CORRECT — decisively ruled out.** The draw-stream "matches 3075/3075"
  is a *count-only* artifact (a pure LCG makes `(before,after)` trivially equal until
  the draw counts desync). The real signal is the **operand** (`Random(N)` die size,
  from `ss_sp_words[3]`), and it first diverges at **draw 33**. That draw is the
  `field_15` gate's second roll: ours `d2`, capture `d4`. Chasing it, I disassembled
  the wrapper at image `0xa55a`: `call RandNext; xor ax,ax; …; xchg ax,dx; div bx;
  xchg ax,dx; retf 2` — i.e. `(0:hi16) / N`, remainder → `hi16(new_state) mod N`.
  **Exactly what `gbx-prng` implements.** A full-state or lo16 reduction would
  overflow the 16-bit `div` (and, tested empirically, reshuffles initiative to the
  wrong first actor). So the RNG is right; the divergence is *logic*, not dice. (Same
  lesson as v1: the binary is the spec, and it exonerated the RNG here.)
- **Initiative + selection are CORRECT.** Both our engine and the capture pick
  **combatant 5 first** (PHILIPPE, `delay 8`). 16 d6 + 16 d100 match.
- **Terrain is REAL and load-bearing** (reverses §11's refutation, which used the
  buggy first terrain hook). The grid is a coherent bar room — party clustered left,
  monsters right, diagonal walls, every combatant on a passable tile. Using it drops
  our excess draws from 3668 (uniform) to 3232; the real fight is 3075.
- **The divergence is turn 1, combatant 5 = PHILIPPE, the party's Magic-User**
  (class 5; the others are Paladin/Paladin/Fighter-Thief/Fighter-Mage/Cleric). In the
  **capture PHILIPPE holds his corner the ENTIRE fight** — `(23,11)` hp27 in every one
  of the 11 `round_snapshot`s, never moving, never attacked, only re-targeting
  (11→13→8) as each enemy dies. **Our engine marches him into melee** (moves to
  `(32,13)`, swings a `d20`), which desyncs the whole board from round 1 and makes our
  fight run **157 draws longer** (3232 vs 3075).
- **The turn-body fork (draw-level):** capture's PHILIPPE turn = `[d8, d4, d7, d7,
  d10]` then **ends** (guards). Ours = `[d8, d2, d7, d7, d10, d1, d20]` then attacks.
  So two concrete coab-vs-binary gaps: (a) the `field_15` behavior-gate (draw 33: for
  the *same* `d8`=5, the binary draws `roll_dice(4,1)` where our coab-derived
  `field_15_mode_gate` draws `roll_dice(2,1)`), and (b) find_target picks a different
  target (7 vs 11 — same roll on an identical state, so the near-list **order** differs)
  and then the binary **guards** where ours enters `sub_35DB1` and swings.
- **coab is NOT the spec here.** coab's `find_target` (`ovr014.cs:2238`) is identical
  to ours and *also* returns a target for a far-off caster, and coab's turn body would
  also charge PHILIPPE in — so this is a genuine coab-vs-binary divergence in
  `PlayerQuickFight`, exactly like the PRNG, the `Random(0)` short-circuit, and the
  other ~7 confirmed classes. `field_15` in our engine only indexes `DATA_2B8`
  (movement approach angle), so fixing the gate corrects the *path*, not the
  hold-vs-charge — the hold is a separate turn-body behavior.

**Next (a real RE session, not a guess):** disassemble the binary's `PlayerQuickFight`
turn body in `GAME.OVR` (start at the `field_15` gate that first forks at draw 33,
then the target/move-attack loop), and model the caster/hold behavior the binary has
and coab lacks. Then re-run `h4_turndiff` toward `N/N` → **H4 MELEE CLOSED**. The
localizer + `combat4.gbxtrace` are the ground-truth harness for that work.

## 15. The binary RE: three coab≠binary bugs in the QuickFight turn body (2026-07-18, session 6)

Disassembled `PlayerQuickFight` and its callees directly from the IDA listing
`~/src/goldbox-refs/coab/coab_new.lst` (CP437; the `ovr010` segment starts at line
~94171; **ovr010 file offset = IDA-linear − 0x35000**, so `sub_3504B`=`ovr010:004B`,
`sub_35DB1`=`ovr010:0DB1`, `sub_359D1`=`ovr010:09D1`, `CanMove/sub_3573B`=`ovr010:073B`).
Three confirmed divergences, all where our engine faithfully copied **coab** and coab
diverges from the **binary** (the spec):

**Bug #1 — the `field_15` gate (`sub_3504B` @ovr010:0090). CONFIRMED + empirically
validated.** The binary:
```
cmp field_15,0 ; jz body        ; enter directly on 0
cmp field_15,4 ; ja body        ; enter directly on >4  (coab wrote "== 4")
  roll_dice(4,1); jnz skip      ; field_15 in 1..4: draw d4, enter iff ==1
body:
  roll_dice(8,1) → v
  v != 8 → field_15 = roll_dice(4,1)      (1..4)   ; coab draws d2+4 here
  v == 8 → field_15 = roll_dice(2,1)+4    (5..6)   ; coab draws d4 here
```
Two errors in coab/our `field_15_mode_gate`: (a) entry short-circuit `== 4` should be
`> 4`; (b) the `d8==8` branches are **swapped**. The common case (d8≠8) draws a **d4**,
not d2. Applying just (a)+(b) moved the first operand divergence **draw 33 → 37** —
proving the read. (This supersedes combat #4 D1's "short-circuits on {0,4}", derived
from coab.)

**Bug #2 — the `data_2B8` approach-direction table (`CanMove`/`sub_3573B` @ovr010:076D).
CONFIRMED from raw bytes.** The table lives at `seg600:0x2BD` =
`[0, 8,7,6,1,2,8, 1,2,7,6,7, 1,8,6,2,1,7,8,2,6,8, 7,6,5,4,8, …]`. The binary indexes
`byte[0x2B8 + 5·field_15 + dirStep]` = `T[5·(field_15−1) + dirStep]` — a **stride-5
sliding window**, so binary `field_15=N` reads coab **row N−1**. coab materialized the
overlapping windows into 6-wide rows and indexes `data_2B8[field_15][dirStep−1]` (row
**N**) — an **off-by-one on the approach-direction row**, which our `DATA_2B8` copies.
The fix is `DATA_2B8[field_15−1]`. (Verified it changes movement, but see below — it is
not the hold cause on its own.)

**Bug #3 — the attack range (`sub_35DB1` @ovr010:0ED1). Mechanism identified.** The
binary computes `var_4` (attack range) from the readied weapon: `field_151` (a weapon
struct ptr on the record) → `[field_2E]` → table `@0x5D1C` → `<<4 − 1`, defaulting to 1.
The reach/attack decision is then `steps/2 > var_4 → move, else attack` — **identical to
our engine** except we hardcode `var_4 = 1` ("no ranged weapon modeled"). This is the
ranged-weapon gap; it does not affect the unarmed bar brawl (range 1) but is needed for
armed fights.

**Bug #4 — the Magic-User guard (`sub_359D1` @loc_35AA3). PINNED + validated. THIS is the
hold** (Bryan confirmed live: PHILIPPE guards the whole fight, no magic, no attack).
`sub_359D1` **is** coab's `moralFailureEscape` (a coab misnomer; the "Move/Attack, Move
Left =" string proves it's the *approach* step, and it also handles flee — one function, as
in the binary). Its PC path has an explicit early exit:
```
loc_35AA3:
  cmp actions.moral_failure(+14h), 0 ; jnz →advance    ; fleeing → move
  mov ax,[player+159h]; or [player+15Bh]; jnz →advance ; field_159 ptr non-null → move
  cmp player.class(+75h), 5 ; jnz →advance             ; class != 5 → move
  jmp loc_35D9E                                         ; class 5 + not fleeing + field_159 null → GUARD
```
So **a non-fleeing pure Magic-User (`class == 5`, record `+0x75`) with a null `field_159`
does not advance in QuickFight — it guards.** PHILIPPE is class 5 → holds all fight; the
party's Paladins/Cleric/Fighter-multiclasses are not → they advance and fight. Our
`moral_failure_escape` has no class-5 guard, so it charges PHILIPPE in. (The near-list is
*faithful* — our `near[5]` = monster 11, same as the binary; the earlier "target 7" was a
**consequence** of charging + retargeting, not the cause. `field_159` @0x159 is a
far-pointer, null here — likely a readied ranged option; a mage with one would advance.)

**Empirical validation (all four, layered, over `combat4`).** Applying #1 + the class-5
guard moved the first *operand* divergence draw **33 → 129**; adding #2 → **153**; and the
round-1 board is now **near-exact** — PHILIPPE (5) and LEDERA (3) identical, most monsters
identical, only MATHEW (0)/TRAVIS (2)/SHARA (4) ~1 cell off. PHILIPPE holds at (23,11) with
target 11, exactly like the capture. Draw count closes from 3668 (baseline uniform) to 3146
(all-four, real terrain) vs the capture's 3075. The draw-153 fork is `combatant 12`'s turn:
every draw matches through its move `(34,12)→(33,13)`, then `roll_dice(near.len())` for the
adjacent re-pick draws `d1` (our 1 adjacent party member) vs `d2` (capture's 2) — a **cascade**
from a party member's round-0 move landing one cell off, not a fresh mechanic. So the last
knot is a **fine movement-step difference** in round 0 (a mover takes one extra/fewer step or
a 1-off direction), best localized empirically move-by-move (our per-turn positions vs the
capture's `turn_snapshot`s) rather than by more static reads. `dirStep`/`data_2B8`/base-dir
all check out (base dir = `sub_409BC`/`getTargetDirection`, our `target_direction`, matches;
the loop is `dir_step`/`var_3` 1..5 both sides).

**Plan.** The complete fix is: **#1** (`field_15` gate), **#2** (`data_2B8` row `field_15−1`),
and **#4** (decode `class`@0x75 + `field_159`@0x159 onto `Combatant`; guard a non-fleeing
class-5 mage with null `field_159` in the approach), plus updating the coab-based gate/parity
tests to the binary behavior; **#3** (weapon range) stays scoped to M5. Then close the draw-153
movement residual and re-run `h4_turndiff` → `N/N` = **H4 MELEE CLOSED**. All engine edits this
session were reverted (RE-validation only).

**Status/plan.** Bugs #1 and #2 are confirmed and ready to implement (with the coab-based
gate/parity tests updated to the binary behavior); #3 is scoped (ranged weapons, likely
M5). The remaining RE step is `sub_359D1`'s PC approach loop to pin the hold, then the
combined fix + `h4_turndiff` re-run toward `N/N` closes H4 melee. All engine edits this
session were reverted (RE-validation only); the repo carries only the localizer test,
its dev-dep, and this doc.

## 16. The four fixes IMPLEMENTED — draw match 33 → 153, residual = round-0 movement (2026-07-18, session 7)

§15's four findings were **implemented and landed** in `gbx-engine::combat`, each first
re-verified against the actual IDA listing `coab_new.lst` (`grep -a`; CP437) at its cited
`ovr010:` address before writing code:

- **#1 `field_15_mode_gate`** (`ovr010:0090`): entry `v == 0 || v > 4` (the `cmp 4; ja
  loc_350AB`, not `== 4`); body branches **swapped** so `d8 != 8` → `roll_dice(4,1)`
  (`loc_350D4`, 1..4) and `d8 == 8` → `roll_dice(2,1)+4` (`loc_350BF`, 5..6).
- **#2 `DATA_2B8`** (`CanMove`/`sub_3573B`): both call sites (`can_move`,
  `moral_failure_escape`) now index `DATA_2B8[field_15.saturating_sub(1)]` — the binary's
  stride-5 window reads coab row `N−1`; coab row `R` = `T[5R+1..=5R+6]` includes the 6th
  column, so `field_15−1` is faithful for `dir_step` 1..=6.
- **#4 the Magic-User hold** (`sub_359D1` @`loc_35AA3`): `class`@0x75 and `field_159`@0x159
  (a 4-byte far-pointer, null == all-zero) are decoded onto `Combatant`
  (`combatant_from_record`, from the raw record bytes). The guard sits at the shared
  post-advance block `loc_35AA3` (reached by **both** the PC path — `control_morale < 0x80`
  → `jb loc_35AA3`, skipping the d100 — and the advancing-NPC path): a **non-fleeing**
  combatant with `class == 5` and a null `field_159` calls `try_guarding` and returns
  (`jmp loc_35D9E`, which is `sub_361F7` = our `TryGuarding`). The `sub_35DB1` caller then
  exits its loop **draw-free** (once a target is held, `find_target` re-draws nothing).
- **#3 weapon range** left as a cited `TODO(M5, FD-29)` at the `range = 1` hardcode
  (`ovr010:0ED1`, `field_151` → table `@0x5D1C`) — unarmed brawl is range 1.

**Parity tests updated to the binary behavior (recomputed, not weakened):** the two
`field_15` gate unit tests (renamed `..._short_circuits_on_0_and_over_4` /
`..._draws_the_d4_gate_for_1_through_4`, plus a new `..._enters_the_body_when_over_4...`),
the distribution test's oracle, and `melee_turn_adjacent`'s hand-derived stream — each
re-derives its expected draws from an independent `gbx-prng` replay of the corrected logic.
The invariant-style parity tests (`monster_approach`, `all_ai_1v1`,
`run_combat_full_round_loop`, `run_combat_driver_matches_raw_step`) needed no change — they
self-derive from the actual draw stream. `.rsav`/save goldens untouched; both `watch_*`
demos assert only invariants (no committed transcript to re-bless).

**`h4_turndiff` result (real terrain, `combat4.gbxtrace`, seed `0x80ee4cee`):**
- first **operand** divergence moved **draw 33 → 153** (exactly §15's layered validation);
- our draw count closed **3971 (uniform) / 3146 (real terrain)** vs the capture's **3075**;
- round-1 board: **PHILIPPE (5)** holds `(23,11)` and **LEDERA (3)** `(31,12)` are
  **byte-identical** to the capture; most monsters identical.

**Residual (unchanged in character from §15 — a round-0 movement cascade, NOT a mechanic):**
the first divergent *round* is round 1, combatant 0 (MATHEW) at `(31,10)` vs capture
`(31,11)` — one cell off — with combatants 1/2/4 and monster 13 also ~1 cell off. The
draw-153 fork is `combatant 12`'s adjacent re-pick: `roll_dice(near.len())` draws `d1` (our
1 adjacent) vs `d2` (capture's 2), purely because a party member's round-0 step landed one
cell off. Movement is draw-free, so this shifts draw-free targeting without changing any
roll until draw 153. **This needs a dedicated RE of the `sub_35DB1`/`sub_3E748` approach
stepping** (a step-count or `CanMove` tie for the approaching party members — base
direction, `dir_step` loop, and `move_cost` gates already check out per §11–§13), so per the
brief the confirmed #1/#2/#4 fixes land as a reviewed slice and the residual is reported
with this localization rather than blocking on it. The localizer
(`h4_turndiff::h4_turndiff_localize`) gained a **per-turn POSITION-only** diff (cadence-
caveated) alongside the authoritative cadence-robust per-round diff. Gates 6/6 green
(build+wasm core/web, 324 workspace tests, clippy, fmt, guard); `.rsav` goldens untouched;
no new coab `Data/*.DAX` read.

## 17. Bug #5 — the near-target sort (`sub_73033`); the "movement residual" was targeting (2026-07-19)

The §15/§16 "round-0 movement cascade" turned out **not to be movement at all** — it was the
near-target **sort**. Instrumenting the first mover (SHARA, combatant 4) showed her drifting
north because she targeted monster **14** (`33,11`), while the capture targets **6**/then 7
(`34,13`). The draws match through her turn, so her `find_target` roll matches the binary —
which means her **near-list order** differed. Her near-list has monster 6 (dir 2) and monster
14 (dir 1) **tied on steps (18)**; our sort put 14 first, the binary keeps 6 first.

**The binary (`sub_73033` @`ovr032:0033`) is an exchange sort (swap-on-every-improvement,
confirmed at `ovr032:011A-0186`: the 3-byte triple swap runs inside the inner loop, no
min-index tracked — review callout settled 2026-07-20) with a PARTIAL-order predicate**,
not a clean key. Element `j` swaps before element `i` iff `steps[j] < steps[i]`, OR
(`steps` equal AND `dir[j] < dir[i]` AND `dir[j]%2 <= dir[i]%2`). For a diagonal-vs-orthogonal
tie (`dir 1` vs `dir 2`) **neither** swaps the other, so **build (roster) order is preserved**
— monster 6 (roster-earlier) stays before 14. coab's `SortedCombatant.CompareTo` collapsed
this into a clean `(steps, direction)` key with the `direction % 2` term as an *unreachable*
innermost tie-break (§12 dismissed it as a no-op) — wrong. The fix replaces
`build_near_targets`' `sort_by` with the exact `sub_73033` nested-loop predicate.

**Result:** first operand divergence **draw 153 → 358** (real terrain), and **MATHEW's round-1
position now matches the capture exactly** — the whole cell-off cascade is gone (it was
target-order the whole time, per the §13 sort-tie suspicion). 324 engine tests still pass (the
synthetic parity tests don't hit a tie, so the sort change is inert there). This also retires
the "per-step move capture" plan from §16 — no finer capture was needed; the disassembly of
`sub_73033` settled it.

**New residual: draw 358** — a `d20`-vs-`d2` (to-hit vs damage) split inside a round-0 turn
(after a `find_target` d6), i.e. an **attack-resolution** subtlety, not movement. Next onion
layer. (Method note: the metric switch from count-only `(before,after)` to the **operand**
stream — §16's "2995" was LCG-trivial count-matching — is what makes each of these layers
visible; the operand localizer is the load-bearing tool.)

## 18. Bug #6 — monster attack-spreading; the target-validity check (2026-07-19, Fable review)

Found by a Fable review pass when this session mis-called the draw-358 divergence a "murky
reach knot" and leaned toward banking. It was neither murky nor reach — it was **ours vs coab**
(our engine had "normalized" coab's correct-but-asymmetric code), three lines *above* the reach
probe I'd been re-reading.

The binary's target-validity check at the top of `sub_35DB1`'s loop body (`ovr010:0F12–0F46`)
loads `actions.target` into a **local** `player01` and nulls that local when the target is out
of combat **or** `cmp [combat_team], 0` — an **immediate-0 compare (Team::Party)**, NOT the
attacker's team, which is never loaded. coab is faithful (`target.combat_team == CombatTeam.Ours`,
ovr010.cs:578). Our engine had rewritten it as the "obvious" symmetric sanity check
`tf.team == attacker.team` — which is *always false* (targets are opposite-team), so we **never
dropped**, always took the attack-directly fast path.

Consequence: a **monster** attacker's held target is always a party member (`team == Party`), so
the binary always drops it here and falls through to the near-list **re-pick** — i.e. monsters
**spread attacks uniformly among adjacent PCs** (`roll_dice(near.count)`, the capture's extra
`d2`), the classic Gold Box behavior. A **party** attacker holds a monster target
(`team != Party`) and keeps the fast path. Two more faithful details: the drop nulls only the
**local**, not `actions.target`; and the re-pick stores to the **local** only (no write-back).
Fix: thread a local `chosen` through the loop body; `tf.team == Team::Party` for the drop.

**Result:** first operand divergence **358 → 459** (real terrain); our draw count 3744 → 3346
(capture 3075); the round-1 board now has **all 16 positions and all 10 monsters byte-identical**
to the capture (only two party hp cells differ — the draw-459 fork). One parity test recomputed
(`melee_turn_adjacent`: the monster's `d1` re-pick added via the independent oracle). 324 tests
pass.

**Method lesson (Fable's):** "clean domino vs murky knot" is a statement about *comprehension*,
not the code — a genuine contradiction always means a false premise (here: "our validity check
matches coab's"). Before declaring a knot, **diff the entire enclosing function against the Rust
from the listing, not from coab.** The banked claim would have been *wrong*: the divergent
mechanic was monster damage-allocation across the party, gameplay-visible every round — exactly
what H4 exists to catch.

**Next residual: draw 459** — SHARA (party), round 1: ours draws a `d3` (find_target near-count)
where the capture attacks a **held** target draw-free (`find_target`/`sub_41E44` early-outs on a
surviving held target). The residual family is the **`actions.target` lifecycle** (who writes/
clears it, at find_target / re-pick / TryGuarding / clear_actions / attack-cleanup) — a bounded,
named read, localizer already pointing at the exact actor and draw.

## 19. Bug #7 — the attack write-back to actions.target (2026-07-19)

The draw-459 residual was the `actions.target` lifecycle, as Fable predicted. Draw 459 is
SHARA (party, round 1): the capture attacks a **held** target draw-free while ours draws a `d3`
re-pick. Instrumenting showed our SHARA carries `actions.target = 6` into round 1 while the
capture carries **7** — the monster she actually *attacked* in round 0 after a reach re-pick.
The §18 fix correctly stopped the re-pick from writing `actions.target` (it writes only the
local `chosen`, per the binary) — but I'd missed the compensating write: **`AttackTarget`
(`sub_3F9DB`, ovr014.cs:939) sets `attacker.actions.target = target`** on every attack. So the
persistent target becomes the *attacked* combatant, and next round's `find_target` keeps it
draw-free (target 7 is adjacent → attack directly, no `d3`). Our `attack_target` never did this.

Fix: `attack_target` sets `self.fighters[actor].target = Some(target)` up front. Draw-free (only
the held target carried into later rounds changes), so round-0 draws are untouched; 324 tests
still pass (the parity test already asserts the post-attack target).

**Result:** first operand divergence **459 → 747** (real terrain, +288 — the biggest single jump
yet); draw count 3346 → 3342 vs capture 3075. The onion is yielding *more* per layer, not less.
This was a clean domino found by diffing the enclosing functions from the listing (§18's lesson).

**Next residual: draw 747.** Corroborating open thread (not yet the blocker): a guard turn should
**clear** `actions.target` — the capture shows PHILIPPE ending his guard with `tgt255` while ours
holds `tgt11` (`TryGuarding`/`clear_actions` → `actions.target = null`, cf. ovr010.cs:447 /
ovr014.cs:2357). Same `actions.target` lifecycle family.

## 20. Bug #8 — the near-list best-pair init; and a metric refinement (2026-07-19, Fable)

The draw-747 kill-cascade traced to combatant 14 (a monster) re-picking the wrong PC in
round 0 (SHARA in ours, MATHEW in the capture). Root: `build_near_targets`' `found_range`
accumulator is initialized to **`0xFF`** in the binary (`sub_738D8` @`ovr032:097B`:
`mov [bp+var_1F], 0FFh`), not `max_range` as coab wrote (`found_range = max_range`,
ovr032.cs:243) and we copied. With `0xFF`, the first reachable footprint pair *always* fires
the `steps < best` update, so every entry records the **real** min steps (2 orthogonal, 3
diagonal) and the direction from the **real** winning cells. coab's `max_range` init happens
to coincide with `0xFF` exactly when `max_range == 0xff` — which is why `find_target`'s lists
(range `0xff`) were always correct and **only the range-1 re-pick list degenerated**: every
entry got `(steps=1, dir=find_combatant_direction((0,0),(0,0)))`, so the sub_73033 sort
collapsed to roster order and `near[roll]` picked the wrong PC. (My earlier "coab shares this
bug" was the false premise — it's coab's alone. My `near_enermy`-uses-a-different-list
suspicion was also refuted: `near_enermy`/`ovr025:25E0` fills its table from the *same*
`sub_738D8` output, preserving order.) Fix: `found_range` init `max_range` → `0xFF`; the sort
key is then **(real steps, real direction)** — orthogonal-adjacent (2) sorts before
diagonal-adjacent (3), which a direction-only patch missed (hence its board regression).

**Result:** first divergent **round 1 → 3** — rounds 0–2 are now board-exact (MATHEW enters
round 1 at hp46, byte-identical; combatant 14 re-picks MATHEW). 324 tests pass (one range-1
adjacency assertion recomputed: a diagonal step now stores real steps 3, not the clamp).

**Metric refinement (important going forward).** The operand frontier stayed at **747** — and
that is *expected, not a failure*: a draw-free targeting fix (the re-pick draws the same `d2`
whichever PC it hits) can't move the operand frontier until the cascade reaches a
turn-*structure* change. With draw-free targeting/movement divergences now dominating, the
**first-divergent-round** (from the cadence-robust per-round board diff) is the **leading**
indicator; the operand frontier lags. Track both.

**Next residual: round 2 (draw-free).** At the round-3 snapshot, party damage concentration
differs (capture → monster 11 hp4, ours → monster 14 hp5) and monster 10's approach path
differs by ~3 cells. Same species as the six layers already peeled — a draw-free
targeting/movement order detail, localizer pointing at the round.

**Process note (Fable's):** both recent "murky knots" resolved to a single-line, binary-citable
fix (bug #6: one `cmp` operand; bug #8: one init byte), each found by transliterating the
*enclosing* binary function rather than re-reading the already-verified callees. When ours ==
coab but the capture disagrees, attack coab's fidelity at the enclosing frame first.

## 21. Bug #9 — death cancels pending initiative (`damage_player` @ovr025:24BB) (2026-07-19, session 8)

§20's "round-2 draw-free targeting" residual was neither targeting nor round 2 — it was round
1's **selection**. Reconstructing the capture's round-1 turn sequence from its draw-indexed
`turn_snapshot`s (a scratchpad script diffing consecutive snapshots, plus a d100-run-compressed
operand dump) showed two structural facts: every `FindNextCombatant` pass is **d100 ×16 even
after combatant 9 dies at draw 524** (the dead slot keeps drawing), and **every pass resolves an
acting turn** — 15 turns + the terminating empty pass, no double bursts. Ours instead had a
double burst at 731–762: pass 13 picked **dead combatant 9** (Pick: delay 3, roll 70 — killed at
draw 524 *before its turn*, still holding its round-1 initiative delay), dead-skipped, and burned
an extra 16-draw pass, displacing a live actor's 7-draw turn (capture 747–753: d4 gate → d8/d4,
d7, d7, d1 find, d20 miss).

coab's `FindNextCombatant` (ovr009.cs:59) is a pure `(delay, roll)` two-if with no alive check —
faithful, same as ours (the corpse keeps its d100 slot, matching the ×16 bursts). The false
premise was the **death path**: `damage_player`'s death branch (`ovr025:24BB`:
`mov byte ptr es:[di+3], 0` on the actions struct; coab ovr025.cs:1240) zeroes `actions.delay`
alongside `in_combat = false` and the team-count decrement — a combatant killed before acting
loses its pending initiative, so a corpse can never *win* a pass. Our `apply_damage` set
`in_combat = false` but left `delay` standing, so the corpse stayed the max-delay candidate.
(The flee path was already right: `flee_battle` → `clear_actions` zeroes delay.)

**Fix: one line** — `apply_damage`'s kill branch zeroes `delay` (cited `ovr025:24BB`).

**Result: first divergent round 3 → 6** (rounds 0–5 board-exact) **and operand frontier 747 →
1923** (+1176, the biggest single jump yet; both metrics moved because a selection bug is
turn-*structural*, not draw-free). Our draw count 3543 vs capture 3075. 324 engine tests pass
unchanged (no synthetic fight kills a pending-delay combatant that later wins a pass).

**Next residual: round 5, draw-free movement.** At the round-6 snapshot the ONLY divergence is
combatants [0] (MATHEW) and [3] with **swapped positions** — ours `[0]@(32,12), [3]@(32,11)`,
capture the reverse; every hp byte-identical. The operand fork at 1923 is the downstream
adjacent-count artifact (`d1` vs `d2` re-pick after a `d6` find inside a later turn). Same
species as §17/§20: a draw-free step/order detail, now in party movement into freed corpse
cells.

## 22. Bug #10 — leaving combat frees the occupancy footprint immediately (`sub_74E6F`/`sub_644A7` → `sub_743E7`) (2026-07-19, session 8)

§21's round-5 residual pinned to one grotesque turn: MATHEW's round-5 approach to monster 7.
The capture steps once, orthogonal E — `(31,11)→(32,11)` — and attacks; ours takes **three
diagonal steps in a spiral** — SW to `(30,12)`(!), SE to `(31,13)`, NE to `(32,12)` — before
attacking the same monster with the same draws (PC steps are draw-free, so the PRNG never sees
it; LEDERA then can't take `(32,12)`, and the pair land swapped).

The spiral decodes exactly as a **stale occupancy grid**: our `rebuild_occupancy` ran only on
position changes, so the corpses of 9/11 (both on `(32,11)`) and 14 (`(32,12)`) — dead since
rounds 3–4, during which nobody moved — still blocked `can_move` at MATHEW's step 1 (S is
LEDERA, E and SE are "occupied" corpses → dir_step 4 = SW), and then **his own first step's
repaint freed them mid-turn** (steps 2–3 walk back through the freed cells). The binary
repaints at the removal moment, in both paths:

- **damage kill**: the post-damage display path calls `CombatantKilled` (`sub_74E6F`,
  coab ovr033.cs:534), which ends `CombatMap[idx].size = 0` +
  `setup_mapToPlayerIndex_and_playerScreen()` (`sub_743E7`);
- **surrender/flee**: `RemoveFromCombat` (`sub_644A7` @`ovr024:154F`: `call sub_743E7`
  between the footprint zero and `clear_actions`).

Fix: `apply_damage`'s kill branch and `flee_battle`'s removal both call
`rebuild_occupancy()`. (Cited-deferred: `CombatantKilled` also swaps the ground tile to
`Tile_DownPlayer` (0x1F) for downed **party** members — `nonTeamMember` is true past
`party_size` (ovr011.cs:800), so it never fires for monsters and is out of combat4's scope;
goes with death UI.)

**Result: operand frontier 1923 → 2979 and the round frontier reached the fight's end — all
11 rounds match board-for-board.** Our draw count 3070 vs capture 3075: the whole residual
is one 5-draw tail divergence inside round 10.

## 23. Bug #11 — the sub_354AA d7 rolls BEFORE its guards; ★ H4 MELEE CLOSED: 3075/3075 ★ (2026-07-19, session 8)

The 5-draw tail: round 10, MARK (pass 0) kills the last patron (monster 8, hp3 — the capture
does too, seq 192), then PHILIPPE's turn draws `d4 + d7` in ours but `d4 + d7 + d7` in the
capture — the wand d7, **with zero live enemies**. Instrumentation showed our guard failing on
`opposite_count == 0`; coab agrees (`teamCount > 0` hoisted above the roll, ovr010.cs:188) —
ours == coab, capture disagrees → transliterate the enclosing binary function.

**The binary (`sub_354AA` @`ovr010:04AA`) rolls the d7 at proc entry, before any guard:**
`call roll_dice(7,1)` at `:04C6` into `var_3`; only then `can_use` (`:04D6`, `actions+2`), the
opposite-team live count (`:04EE`, `friends_count[on_our_team(actor)]` @`0x6FAA`), and
`area.can_cast_spells` (`:04FC`) — each `jmp`ing to exit past the **item scan**, which is what
the guards actually gate (and which is draw-free for a weapon-only combatant anyway). coab
hoisted the whole guard above the roll. Invisible until a guard goes false mid-fight — here,
the last enemy dying earlier in the round. Fix: `wand_scan_d7` rolls unconditionally; the
guards live in the doc comment until wand effects land (M5). (`opposite_count` lost its last
caller and is removed.)

**Result: `h4_turndiff` reports NO divergence — operand match 3075/3075, our draw count ==
the capture's 3075, all 11 rounds board-exact.** The `combat4` bar brawl — 16 combatants,
11 rounds, initiative, selection, the full QuickFight melee AI, movement, targeting, to-hit,
damage, deaths — replays **bit-exact, draw-for-draw, end to end**. (The per-turn `tgt11` vs
`tgt255` line the localizer still prints at snapshot 0 is the pre-turn/post-turn hook-cadence
artifact — capture `turn_snapshot`s fire on state writes, ours post-turn; §19's guard-clears-
target thread stays open as a state-fidelity note with zero draw impact in this capture.)

**The eleven coab-vs-binary bugs, in peel order:** #1 field_15 gate entry+branches
(`ovr010:0090`), #2 `DATA_2B8` stride-5 row (`ovr010:076D`), #3 weapon range (deferred M5),
#4 the class-5 mage guard (`ovr010:0AA3`), #5 the near-sort partial order (`ovr032:0033`),
#6 the monster attack-spread validity check (`ovr010:0F12`), #7 the attack write-back to
`actions.target` (ovr014.cs:939), #8 near-list best-pair init `0xFF` (`ovr032:097B`),
#9 death zeroes pending initiative (`ovr025:24BB`), #10 removal repaints occupancy
(`ovr024:154F`/`sub_74E6F`), #11 the pre-guard wand d7 (`ovr010:04C6`).

## 24. The milestone assert: `h4_replay` passes — H4 MELEE CLOSED on the asserting harness (2026-07-19, session 8)

`h4_replay` (the D-OR5(b) milestone differential, dormant since the capture format grew board
snapshots) is revived as the **asserting** proof. The typed `.gbxtrace` reader learned the two
capture-side observation events — `round_snapshot` (`{round, combatants[{team,x,y,hp}]}`) and
`turn_snapshot` (`{seq, combatants[{…,target}]}`) — treated exactly like `combat_entry`:
parse-typed, **ignored by the comparator and the chain-continuity check** (no draw, no PRNG
state). `combat_entry` gained the optional `terrain` field (lowercase hex, wire-ordered between
`rng_state` and `combatants`; the canonical writer omits it when absent, so all existing
goldens stay byte-identical). `h4_replay` now targets `combat4.gbxtrace`, builds its
`CombatMap` from the captured terrain (uniform-floor fallback only for pre-terrain captures),
and **passes**:

```
H4 replay: 16 combatants (6 party, 10 monster), seed 0x80ee4cee;
           our fight = 3075 draws (PartyWins), capture = 3075 draws
per-round survivors: (0,6,10) (1,6,9) (2,6,9) (3,6,7) (4,6,6) (5,6,5)
                     (6,6,4) (7,6,3) (8,6,2) (9,6,1) (10,6,0)
H4 MELEE CLOSED: 3075 draws matched draw-for-draw against the live bar-brawl capture.
```

The equality surface here is the full `(before, after)` **chain**, draw-for-draw, with equal
totals — strictly stronger than the localizer's operand view. CI still skips it without the
local capture (D10). Gates 6/6 (886 workspace tests, clippy 0, fmt, wasm core+web, guard).

**What this closes and what it doesn't (unchanged from §8's frame):** the initiative /
selection / QuickFight-melee / movement / targeting / to-hit / damage / death subsystems are
draw-stream-proven against one real 16-combatant fight. Stubbed-by-design and still open for
M5: spell/wand/turn-undead *effects*, ranged weapons (bug #3's `field_151` range table),
backstab, the 0-HD sweep, surrender's `Int>5` branch + `FleeCheck` morale ladder beyond what
this capture exercised (its patrons never rout: `control_morale 0x80` seeds morale 0 and the
area's `field_58C` keeps the ladder closed — a second capture in a rout-prone encounter would
exercise it), XP/treasure, and the wilderness draw-bearing `SetupGroundTiles`.

## 25. The four-capture matrix, stub tripwires, and the M5 capture runbook (2026-07-19, session 8 cont.)

**A second fight closes.** All four bar-brawl captures in `~/goldbox-data/traces/` are the SAME
encounter (verified: identical entry layout and party cells), so combat4's terrain — validated
by its own 3075/3075 closure — is the room's true grid. Grafting it into the three older
captures (local-only derived files `<name>+terrain4.gbxtrace`; combat2/combat3's own terrain
fields are the §14 buggy-hook output) and replaying:

| capture | seed | result |
|---|---|---|
| `combat4` | `0x80ee4cee` | **CLOSED 3075/3075** (§23) |
| `combat3` + terrain4 | `0xebb7e796` | **CLOSED 3218/3218** — a second complete fight, different kill order; the engine is not overfit to combat4 |
| `combat` + terrain4 | `0xb40d7505` | all 3,162 capture draws match (exact prefix), ours runs 218 longer — our replay downs a party member (round 6) |
| `combat2` + terrain4 | `0x4b7e9837` | all 3,772 draws match (exact prefix), capture runs 488 longer — our replay downs TWO party members |

The pattern is decisive: **both fights with zero party casualties close 100%; both fights
where a PC drops match perfectly until a length divergence.** The downed-PC path —
`damage_player`'s dying/unconscious + bleeding states, ally bandage turns, `CombatantKilled`'s
`Tile_DownPlayer` ground swap — is the confirmed next residual (Phase-1 target #1). The old
captures carry no board snapshots, so they cannot localize it; the next PC-down capture (with
the current hook) will.

**Stub tripwires.** Every deliberately-stubbed original mechanic now EMITS
`ActionEvent::StubTripped` when a replay reaches it, so a capture that wanders into unmodeled
territory names itself instead of silently diverging. Four wires:

- `downed-pc` — `apply_damage` kills a party member (dying/bleeding/bandage/`Tile_DownPlayer`).
- `memorized-spells` — a combatant with non-zero `spellList`@0x1E slots takes an AI turn
  (`sub_3560B`'s inner selection draws, M5).
- `0-hd-sweep` — `try_sweep_attack` meets a 0-HD target (the sweep path, M5).
- `surrender-int5` — `flee_check` reaches the binary's `Int > 5` →
  `RemoveFromCombat("Surrenders")` branch (coab ovr010.cs:803), which we neither decode Int for
  nor model.

Diagnostic-only: the oracle collector drops the event from `.gbxtrace` output; `h4_replay`
prints each trip with its draw index (before any divergence diagnostic) and words its final
line accordingly (`CLOSED` only when zero trips fired). Validated: combat3/combat4 close with
zero trips; `combat` names `downed-pc` @~2288 (combatant 4); `combat2` names it twice
(@~1904 c1, @~2884 c4) — the tripwires would have named the original §9 tail divergence
instantly.

**Capture runbook for the next staging session (Phase 1 — harden melee).** All fights
dungeon/city (draw-free terrain) until wilderness `SetupGroundTiles` lands; current hook
(terrain + `round_snapshot` + `turn_snapshot`) throughout:

1. **A PC-down fight** — any melee where at least one party member drops (the bar brawl played
   sloppy works). Localizes the downed-PC mechanics against snapshots. *Highest value: two
   existing captures already diverge on exactly this.*
2. **A rout-prone fight** — weak/low-morale enemies likely to flee or surrender. While in the
   area, **read `area2 + 0x58C` live** (the morale threshold `field_58C`; combat4 only bounds
   it ≥ 85) and note the value + the area in the capture notes. Drives the faithful
   `FleeCheck_001` transliteration (per-actor `control_morale` seed, >102 clamp, `Int>5`
   surrender) replacing our deviating stub.
3. **An armed fight** — enemies or party with readied ranged weapons (and ideally a 3/2-attacks
   fighter). Exercises bug #3's range table (`field_151` → `[field_2E]` → `@0x5D1C`), weapon
   dice, ammo, and the FD-3 `attack2` profile.
4. *(Optional, opens M5 proper)* **a caster fight** — a mage with memorized spells (and/or
   enemy casters). Trips `memorized-spells` today; becomes the spell-subsystem driver.

## 26. SPEC — the downed-PC path (M5 slice 1; Fable-scoped, implementer-built) (2026-07-20)

**Goal.** Replace the `downed-pc` stub with the faithful mechanics, and thereby (expected)
close the two length-diverging captures: `combat2+terrain4` (ours 3,772 vs capture 4,260 — the
real fight runs longer because party turns are spent bandaging, not attacking) and
`combat+terrain4` (ours 3,380 vs capture 3,162). This slice is fully self-validating against
existing local captures — no new staging needed.

**The mechanics (coab-cited; each site MUST be re-verified against `coab_new.lst` before
coding, per the session-7 discipline — the binary is the spec, coab the reference):**

1. **`damage_player` status ladder** (`ovr025:23D5`, binary-verified §21-era read; coab
   ovr025.cs:1160-1242). With `neg_hp = damage − hp_current` (0 when damage ≤ hp),
   `new_hp = hp_current − damage` (0 when overkill):
   - `neg_hp > 9` OR (`new_hp == 0` AND status == animated) → status **dead**;
   - else `neg_hp > 0` → status **dying**, and (in combat) `actions.bleeding = neg_hp`;
   - else `new_hp == 0` → status **unconscious**;
   - status ∉ {okey, animated} → `in_combat = false`, `hp = 0`, team-count decrement,
     `actions.delay = 0` (`ovr025:24BB`) — all as today, now with the status recorded.
   New `Combatant` state: `health_status` (okey/animated/dying/unconscious/dead — minimal
   enum; entry records are okey; decode from the record if the field exists there) and
   `bleeding: u8`.

2. **The bandage turn** (`sub_35DB1` head, coab ovr010.cs:516-522; binary `ovr010:0DB1`+):
   after `CheckAffectsEffect(Type_14)` (draw-free), **if the actor's `combat_team == Ours`
   AND `bandage(true)` → `actions.delay = 0`** — the turn is spent, the move-attack loop
   (`delayed = delay != 0`) never runs: no movement, no attack, no draws beyond the turn
   head (gate + two d7s + find_target). This is the draw-visible mechanic.

3. **`bandage(applyBandage)`** (coab ovr025.cs:1628): scan `TeamList` in order for members
   with `nonTeamMember == false && combat_team == Ours && health_status == dying`; return
   whether any exists; when applying, convert the FIRST one to **unconscious**, zero its
   `bleeding`, and stop applying (one bandage per call). Monsters never bandage and are
   never bandaged.

4. **The bleed tick** (`BattleRoundChecks`, coab ovr009.cs:369-382): per round end, for each
   TeamList member with status dying: `bleeding += 1; if bleeding > 9 → status = dead`.
   Draw-free. (The `bandage(false)` "Your Teammate is Dying" scan is display-only — skip.)

5. **The downed tile** (`CombatantKilled`, coab ovr033.cs:579-590): for a downed
   `nonTeamMember == false` member, swap the ground tile at its cell to `Tile_DownPlayer`
   (0x1F) unless the cell is `Tile_StinkingCloud` (0x1E). Tile 0x1F has move_cost 1
   (BackGroundTiles[31] = (1,1,0,0x27)) — movement-NEUTRAL on cost-1 floors (the bar), so
   this is fidelity, not the divergence driver. Model `nonTeamMember == false` as
   `team == Party` (cited simplification: allied non-team NPCs are out of this slice's
   scope). Tile restoration (heal/pickup) is M5-spells; cite, don't build.

**Retire the `downed-pc` tripwire** when these land (the remaining unmodeled piece —
restore-on-heal — is unreachable without spells, which have their own tripwire).

**Acceptance (all local-tier, run before AND after):**
- `combat3+terrain4` and `combat4` **must remain CLOSED** (3218/3218, 3075/3075) — zero-
  casualty fights are untouched by this slice (no one dies with 0 < overkill in them — if a
  regression appears, a mechanic leaked into the wrong path).
- `combat2+terrain4` — expected to **CLOSE 4260/4260**. If it does not, report the new
  operand frontier + trips honestly and STOP (the finding scopes the next session; do NOT
  weaken any assert or tune constants to force closure).
- `combat+terrain4` — expected to close at 3162/3162; if it instead stays exact-prefix with
  ours longer, report as a possible truncated capture — do not force.
- Full gates: workspace tests (parity tests recomputed ONLY from the independent gbx-prng
  oracle when a synthetic fight's stream legitimately changes — e.g. a fight where a party
  member drops and a teammate's turn follows now loses that turn's attack draws), clippy
  `-D warnings`, fmt, wasm core+web, no-game-data guard. D10 throughout: no capture bytes,
  no `~/goldbox-data` content, no derived graft files in the repo or tests' committed data.

## 27. LANDED — the downed-PC path; all four captures CLOSE (2026-07-20, M5 slice 1)

The §26 spec was implemented on branch `m5-downed-pc` (four commits, one mechanic each).
**Every §26 coab citation was re-verified against the IDA listing `coab_new.lst` before
coding** — the required (a)/(b)/(c) checks (`sub_35DB1` head @`ovr010:0DB1`, `bandage`
@`ovr025:335F`, `battle01` bleed @`ovr009:0A05`) plus `damage_player`/`CombatantKilled`,
and **no contradiction with §26 was found** at any point (the binary matches §26's rendering
exactly, including the `Status` enum values `okey=0/animated=1/unconscious=4/dying=5/dead=6`
from `Classes/Enums.cs`).

**What landed (four commits):**
- **#1 status ladder** — `HealthStatus{Okey,Animated,Unconscious,Dying,Dead}` + `bleeding` on
  `Combatant`; entry status decoded from record `@0x195`; `apply_damage` rewritten to the
  faithful `damage_player` ladder (`ovr025:23D5`). Behavior-neutral (nothing consumes the
  status yet).
- **#2 bandage turn** — `CombatState::bandage(apply)` (`ovr025:335F`) + the `sub_35DB1`-head
  guard (`ovr010:0DE3-0DFF`): a Party actor with a dying ally spends its turn bandaging
  (`delay = 0` → the move-attack loop never runs). **This is the mechanic that closes the
  length-diverging captures.**
- **#3 bleed tick** — `battle_round_checks` per-round-end `dying → bleeding+1 → dead@>9`
  (`ovr009:0A05-0A2B`). Draw-free; fidelity (not exercised past 9 rounds in these captures).
- **#4 downed tile + tripwire retirement** — `CombatantKilled`'s `Tile_DownPlayer` (0x1F)
  ground swap for a downed party member unless `Tile_StinkingCloud` (0x1E) (`ovr033.cs:579`),
  movement-/reach-neutral on a cost-1 floor; the `downed-pc` stub tripwire retired (the other
  three stay).

**Capture matrix (before → after):**

| capture | before | after |
|---|---|---|
| `combat4` | CLOSED 3075/3075 | **CLOSED 3075/3075** (unchanged) |
| `combat3+terrain4` | CLOSED 3218/3218 | **CLOSED 3218/3218** (unchanged) |
| `combat2+terrain4` | 3772/4260 (exact prefix, 2× `downed-pc`) | **CLOSED 4260/4260** |
| `combat+terrain4` | 3380 vs 3162 (exact prefix, ours longer, 1× `downed-pc`) | **CLOSED 3162/3162** |

`combat+terrain4` was **not** a truncated capture — with the bandage turn built, ours ends at
exactly the capture's 3162 draws (the pre-slice "ours runs longer" was the missing bandage
turns letting our party out-damage the original). **All four captures now report `H4 MELEE
CLOSED` with zero stub trips.** Gates 6/6 green (workspace tests 0 failed incl. the real-data
`watch_a_real_data_fight` demo, clippy `-D warnings`, fmt, wasm core+web, no-game-data guard);
no synthetic parity test needed recomputing (none exercises a dying-ally bandage). `.rsav`
goldens, the oracle format, and the other three tripwires untouched. D10 preserved.

**Left for M5 (cited, not built):** the downed-tile **restoration** on heal/pickup (spell
subsystem), and `bandage`'s allied-non-team-NPC case (modeled as `team == Party`).

## 28. SPEC — faithful FleeCheck_001 + surrender (M5 slice 2; Fable-scoped) (2026-07-20)

**Goal.** Replace the deviating `flee_check` stub with the faithful `sub_3637F` ladder and
close the rout capture `~/goldbox-data/traces/bar-rout-58c50.gbxtrace` (bar brawl, poked
`field_58C = 50` via the hook's new `RESTRIKE_58C`; seed `0x804aa4d4`, 3,521 draws, 12 rounds;
patrons rout from ~draw 2514, ≥2 escape at the map corner; two PCs go down — slice 1's
mechanics are in the matched prefix). D10: local-only, as ever.

**Context facts (measured live 2026-07-20):** the bar's real `field_58C` is **99** — with the
health pct quantized to multiples of 5, the natural bar rout is impossible (gate needs < 1),
which is why the four closed captures never exercised this ladder. The hook now emits
`area2_field_58c` in every `combat_entry` and accepts a `RESTRIKE_58C` poke (both committed on
the local `restrike-hook` branch).

**The binary (`sub_3637F` @`ovr010:137F`, read this session; re-verify each site before
coding):**

1. `moral_failure = 0`; `RemoveAttackersAffects` (draw-free). `fleeing` (`actions.field_10`)
   → `moral_failure = 1`, return false ("is forced to flee"). (`:1391-13DD`)
2. `control_morale`@0xF7 `> 0x7F` else return false. Morale seed
   `monster_morale = (control_morale & 0x7F) << 1` (`:13F1-13FC`); **`> 0x66` (102) → 0**
   (`:13FF-1406`). `CheckAffectsEffect(Morale)` (0x11; draw-free, no affects). Per-actor,
   EVERY call — our stub's process-lifetime scratch (stuck at 100 after the first turn) is
   the deviation being replaced.
3. **Gate 1** (`:143F-144D`): `morale < (100 − hp_cur·100/hp_max)` — **signed `jl`** — OR
   `morale == 0`; else return false.
4. `monster_morale = byte_1D903` (enemyHealthPercentage) (`:1458`); second
   `CheckAffectsEffect(Morale)`.
5. **Gate 2** (`:146C-1495`): `morale < (100 − area2.field_58C)` — ★ **UNSIGNED 16-bit `jb`
   (`:1481`): coab ≠ binary bug #12.** `100 − field_58C` is computed in AX and compared
   unsigned, so `field_58C > 100` underflows to ~0xFFxx and the gate is ALWAYS true; coab's
   signed int makes it always false. Transliterate as `u16` wrapping subtraction. ★ — OR
   `morale == 0` OR `combat_team == Party`; else return false.
6. **Speed fork** (`:1498-14BE`): `MaxOppositionMoves > CalcMoves/2` — signed `jg` → the
   surrender branch; **else** (`<=`) `moral_failure = 1` + `remove_affect(0x4A)` +
   `remove_affect(0x4B)` (both no-ops, no affects; cite) (`:14C0-14F5`).
7. **Surrender branch** (`:14F7-1529`): record byte **@0x13 (Int) `> 5`** else return false;
   `RemoveFromCombat("Surrenders", status=4 unconscious, player)` (`sub_644A7` — sets
   `in_combat = false`, hp 0 is NOT written here (health_status drives it), team-count
   decrement, `CombatMap[idx].size = 0` + `sub_743E7` occupancy repaint, `clear_actions`;
   **NO `Tile_DownPlayer` stamp** — that is `CombatantKilled` only, keep slice 1's stamp out
   of this path); return **true** (turn over; `melee_ai_turn` step 2 already returns on it).

**Flee outcome (already implemented, becomes capture-proven):** `moral_failure = 1` drives
the existing `moral_failure_escape` flee path — per-step `d100` + flee-direction `d2` (the
capture's visible rout signature from ~draw 2514) — and `flee_battle`'s escape ladder (the
12-vs-12 speed tie draws its `d2` tiebreak). "Got Away" removal (`ovr014.cs:451`,
`RemoveFromCombat(..., Status.running, ...)`): set `health_status` to a new `Running` variant
(verify the enum value in `Classes/Enums.cs`; decode folds it to Okey on entry records as
with the other non-entry states), `in_combat = false`, occupancy repaint, no tile stamp.

**Engine/harness plumbing:**
- Decode `control_morale` (raw byte, already decoded) and **Int @0x13** onto `Combatant`
  (verify against `decode_char_record`'s stats block; the DEX `.original` convention).
- `CombatEntryEvent` gains optional `area2_field_58c: Option<u16>` (additive; canonical
  writer omits when absent — existing goldens byte-identical). Both harnesses
  (`h4_replay`, `h4_turndiff`'s local parser) feed it into `CombatState.area_field_58c`;
  legacy captures without the field default to **99** (the measured bar value; cite this
  section).
- **`h4_replay` operand equality (harness debt, found this session):** the `(before, after)`
  chain advances identically whatever die is asked for, so chain equality is only
  draw-COUNT equality (the §14 lesson resurfaced). Extend the equality surface: when both
  sides carry an operand (`n` vs `ss_sp_words[3]`), a mismatch at draw i is a divergence.
  The four closed captures were already operand-verified by the localizer and must stay
  closed under the stricter assert.
- The `surrender-int5` wire: **keep it**, repurposed — it now fires when the *implemented*
  surrender branch executes, marking a capture that exercises a not-yet-capture-proven path
  (the rout capture never surrenders: the 12-vs-12 speed tie always takes the flee fork).
  Same for a new `got-away` reporting? No — the flee path IS exercised by the acceptance
  capture; no wire needed.

**Acceptance (all local-tier; before AND after):**
- The four closed captures stay CLOSED under the faithful ladder + the stricter operand
  assert (with `field_58C = 99` they mathematically cannot rout — a regression means a leak).
- `bar-rout-58c50.gbxtrace` **closes 3521/3521 operand-exact**. If it does not, report the
  frontier honestly and stop — no forcing, no assert-weakening, constants only from the
  listing.
- Full gates: workspace tests (parity recomputation only via the independent gbx-prng
  oracle), clippy `-D warnings`, fmt, wasm core+web, no-game-data guard. D10 throughout.

## 29. LANDED — the faithful FleeCheck ladder; the rout FIRES but does not yet close (M5 slice 2, 2026-07-20)

The §28 spec was implemented on branch `m5-fleecheck` (four commits). **Every §28
site was re-verified against the IDA listing `coab_new.lst` before coding**, plus one
site §28 did not name (`calc_enemy_health_percentage`) that the faithful gate-2 turned
out to depend on. The rout now fires — bar-rout's monsters flee to the correct SE
corner and the frontier moved from a stub that never routed to a real rout — but the
capture does **not** fully close: a downstream targeting/flee-movement-order residual
remains at draw ~2707, and the flee **heading** needs an input (`map_direction`) the
capture does not carry.

**What landed (four commits):**
- **#1 harness honesty** — `h4_replay` now asserts **operand** equality (`n` vs
  `ss_sp_words[3]`) on every draw both sides carry one, not just the `(before,after)`
  chain (which is draw-COUNT-only for a pure LCG). `CombatEntryEvent` gained optional
  `area2_field_58c: Option<u16>` (additive; writer omits when absent → goldens
  byte-identical); both harnesses feed it into `CombatState.area_field_58c`, legacy
  captures defaulting to 99.
- **#2 the faithful `FleeCheck_001` ladder** (`sub_3637F` @`ovr010:137F`) — per-actor
  morale reseed `(control_morale & 0x7F) << 1` every call (`:13F1`), `>0x66→0` (`:13FF`);
  gate 1 signed `jl` (`:1446`); **gate 2 UNSIGNED 16-bit `jb`/`sub` (`:1481`/`:1473`) =
  bug #12** (a unit test pins the `field_58C > 100` always-true underflow); speed fork
  signed `jg` (`:14BE`). Decodes `control_morale@0xF7` + `Int@0x13`
  (`stats2.Int.original`) onto `Combatant`. **Plus the `calc_enemy_health_percentage`
  denominator fix** (`sub_40E00` @`ovr014:2E00`, coab `ovr014.cs:1674`): `maxTotal` sums
  `hit_point_max` over **all** enemies incl. dead (`:2E4B`), `currentTotal` only over
  `in_combat` (`:2E28`). Our previous `in_combat`-only denominator kept
  `enemyHealthPercentage` too high, so the faithful gate 2 never crossed its threshold
  and the rout never fired. Safe for the closed captures (a monster's advance
  short-circuits on `|| team == Monster`, so this value only ever moves the flee gate,
  closed at `field_58C = 99`) — empirically confirmed (all three stay CLOSED).
- **#3 surrender + Got Away** (§28 item 7) — `remove_from_combat` (`sub_644A7`
  @`ovr024:14A7`): `in_combat=false`, `health_status=status`, `hp=0` **unless**
  `status==running` (`:151A`), occupancy repaint, `clear_actions`, no downed-tile stamp.
  Surrender branch `Int>5 → RemoveFromCombat(unconscious)` + return true; `flee_battle`'s
  Got-Away removal uses it with the new `HealthStatus::Running` (`Status.running=3`). The
  `surrender-int5` wire kept, repurposed (fires on the surrender branch — unexercised by
  the acceptance capture).
- **#4 map_direction plumbing** — the flee heading (`sub_359D1` @`ovr010:0B14`) derives
  from `gbl.mapDirection`; the capture omits it, so both harnesses read `RESTRIKE_MAP_DIR`
  (trial knob), defaulting to the geometry-matched **2 (E)**.

**Capture matrix (before → after, live, operand-exact assert):**

| capture | before (§27/§28) | after |
|---|---|---|
| `combat4` | CLOSED 3075/3075 | **CLOSED 3075/3075** |
| `combat3+terrain4` | CLOSED 3218/3218 | **CLOSED 3218/3218** |
| `combat2+terrain4` | CLOSED 4260/4260 | **CLOSED 4260/4260** |
| `combat+terrain4` | "CLOSED 3162/3162" (count-only) | **operand-diverges @368** (pre-existing) |
| `bar-rout-58c50` | operand @2514 (never routs, Stalemate) | operand @2707, **routs** (PartyWins) |

**The `map_direction` 4-way trial (live, bar-rout).** `gbl.mapDirection ∈ {0,2,4,6}`; the
monster flee heading is `dir = md − (((md+2)%4)/2)` `% 8` (no `+4` for enemies), verified
against `sub_359D1` @`ovr010:0B03-0B52`. **No value closes 3521/3521**, but the trial is
decisive that **md=2 (E) is the correct heading**:

| md | outcome | operand frontier | first divergent round |
|---|---|---|---|
| 0 | PartyWins | 2516 | round 8 (wrong flee corner) |
| **2** | **PartyWins** | **2707** | **round 8, round-8 rout positions MATCH the capture (SE corner)** |
| 4 | PartyWins | 2555 | round 1 (wrong) |
| 6 | Stalemate | 2516 | round 8 (wrong flee corner) |

Under md=2 the fleeing monsters land at the capture's exact SE cells (`[6]`→(39,17),
`[7]`→(38,16), `[13]`→(39,18), `[15]`→(37,16)); rounds 0–7 are board-exact. So md=2 is
pinned as the geometry-matched harness default, but **not** as a closure pin (per the
"pin only if it closes" rule — none does). The coordinator's `md=4` geometry guess did not
pan out empirically (md=4 diverges at round 1); the direction convention routes md=2's
`dir=2` to the SE corner through the `DATA_2B8`/`can_move` transform.

**The residual (draw ~2707, md=2) — a targeting/flee-movement-order divergence, NOT the
ladder.** At draw 2706 both sides draw the same d20 to-hit (chain-identical); the capture
**hits** (rolls damage) where ours **misses** — i.e. the same roll lands on a different
target (different AC). Root: accumulated round-8 flee differences (monster `[11]` flees to
(36,**14**) vs the capture's (36,**16**), and the party concentrates damage on `[6]`
hp4 vs the capture's `[8]` hp2). This is the same species as the §17–§22 onion layers
(near-target sort / movement-order) but exercised for the first time by the rout, and it
is downstream of the (correct) heading and the (correct) enemy-health gate. It needs the
same instrumented per-turn treatment those layers got; it is out of this slice's scope.

**Findings / contradictions with §28 (reported, not forced):**
1. **§28 missed `calc_enemy_health_percentage`.** The ladder alone is inert for the rout —
   gate 2's input (`enemyHealthPercentage`) must count dead monsters in the denominator or
   it never drops below the threshold. Binary-cited (`sub_40E00`), verified against coab,
   and shown safe for the closed captures. This was the difference between "stub never
   routs" and "rout fires at the right round/corner."
2. **§28 item 7 vs the listing (hp write).** §28 says the surrender `RemoveFromCombat`
   "hp 0 is NOT written here (health_status drives it)". The listing (`sub_644A7:1522-1525`)
   writes `hp_current = 0` for **every** non-`running` status — only the `running`
   (Got-Away) case skips it (`:151A cmp health_status, running; jz`). Implemented per the
   **binary** (hp=0 for the unconscious surrender, skipped for running). Immaterial to
   draws (a removed combatant feeds no draw) and the surrender branch is unexercised by
   the acceptance capture (its 12-vs-12 speed tie always takes the flee fork).
3. **§28's "the four closed captures were already operand-verified" is false for
   `combat+terrain4`.** Under the stricter operand assert it diverges at draw **368 with
   the engine unchanged** — a pre-existing targeting/terrain-graft residual in the oldest
   capture (no board snapshots, grafted terrain, `field_58C=99` so unrelated to flee),
   confirmed by the operand localizer (uniform floor @285, real terrain @368). It only
   ever count-matched. `combat4`/`combat2`/`combat3` are genuinely operand-exact.
4. **`Status.running = 3`** (`Classes/Enums.cs`), the Int byte at record `0x13`
   (`:14FA`), the pushed status `4` (`:1507`), and the `jb`-vs-`jl` gate semantics
   (`:1481` vs `:1446`) were all **confirmed** against the listing — no contradiction.

**TODO (staging hook, the lead patches it separately — do NOT touch the dosbox repo here):**
the hook should emit `map_direction` (`byte_1D53B`, half-encoded {0 N,2 E,4 S,6 W}) in
`combat_entry`, so a rout replay uses the captured heading instead of the `RESTRIKE_MAP_DIR`
default. Once emitted, drop the provisional md=2 default.

**Status.** The faithful FleeCheck ladder + surrender/Got-Away + the enemy-health gate are
landed and binary-cited; the four zero-rout captures stay CLOSED (combat+terrain4 excepted,
pre-existing and unrelated); bar-rout **routs to the correct corner** but does not close —
the residual at draw ~2707 is a downstream targeting/flee-movement-order layer, the next
onion peel. Gates green; `.rsav` goldens and the other tripwires untouched; D10 preserved.

**Addendum — the frontier-pin regression guard.** A committed manifest +
test (`crates/gbx-oracle/tests/h4_frontier_guard.rs`) pins every local capture's
exact H4 outcome: `combat4`/`combat3+terrain4`/`combat2+terrain4` **closed**
(operand-exact, zero trips), `combat+terrain4` **frontier @368**, `bar-rout-58c50`
**frontier @2707** (md=2 applied in-process). The **exact-pin rule**: a frontier
moves ONLY via a deliberate manifest edit made in the *same commit* as the engine
fix that earned it — both a regression (a closed capture diverging, a frontier
shrinking) and an unexplained forward drift (a frontier growing without a manifest
edit) fail the test loudly. It reuses the replay machinery and equality surface of
`h4_replay` (a compact copy), and is local-tier: it loud-skips per-capture when a
file is absent, so plain CI stays green. This is the tripwire that keeps
"operand-exact" honest as the next onion layers land.
