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

## 30. Bug #13 — the departure opportunity attack hits the BEHIND AC (`sub_3F4EB` @ovr014:16F7) (2026-07-20, Fable)

§29's draw-2707 residual named itself in one localization pass once `h4_locate_draw` gained
the same `map_direction` knob as the other harnesses (it had been replaying an md=0 fight —
NW flight — and misleading the peel; fixed here). With md=2, ours picks the same fleer ([8]),
walks the same SE cells, fires the same opportunity attack with the same d20 at draw 2706 —
and misses where the capture hits, with everything after identical shifted by one damage
draw. Same roll, different to-hit math.

**The binary:** `AttackTarget01` (`sub_3F4EB`) selects the to-hit AC by **indexing**
`record[0x19A + behind]` (`ovr014:16F7-1700`: `add di, ax; mov al, es:[di+19Ah]`) — front AC
@0x19A, `ac_behind`@0x19B — where `behind` = the `AttackTarget` `attackType` arg ≠ 0, OR the
flanking heuristic (`AttacksReceived > 1 && facing && directionChanges > 4`, `:16BA-16E9`),
with backstab reading `[0x19B] − 4` (`:169E-16A5`). **The departure opportunity attack is
always behind** (`AttackTarget(null, 1, …)`, coab ovr014.cs:407) — a fleeing patron is hit in
the back, where our engine used front AC everywhere and never decoded 0x19B. First exercised
by the rout capture, because fleeing is what turns a target's back mid-swing.

**Fix:** decode `ac_behind`@0x19B onto `Combatant` (synthetic constructors mirror `ac` —
behavior-neutral for every existing test); thread `behind: bool` through `attack_target`
(departure = true per ovr014.cs:407; into-reach and turn attacks = false per :245/normal);
select the AC by the flag. The flanking heuristic and backstab's −4 stay cited-deferred (M5)
— no capture exercises them.

**Result: bar-rout frontier 2707 → 2894 (+187)** — the fleer takes its hit and the whole
post-hit flee/chase sequence matches; the four closed captures are guard-verified unaffected
(no departure attack in them ever had its outcome flipped). Manifest pinned to 2894 in this
commit, per the guard's rule.

**Next residual: draw 2894** — MARK ([4])'s retarget after his dead target (10) invalidates:
ours draws `roll_dice(1)` (near-list of 1) where the capture draws `d6` (list of 6 = every
live monster). A find_target reach/near-list-size divergence from (35,16) — likely the reach
flood vs the binary's, or an upstream draw-free position difference. The next peel.

## 31. Bug #14 — the departure opportunity attack must RESTORE the attacker's target (`sub_3E954` @ovr014:0C83/0CB3) (2026-07-20, Fable)

§30's draw-2894 residual (`d1` vs `d6`) was not a reach or near-list-size bug at all — the
near-list machinery came through the RE clean end-to-end. `sub_733F1` (canReachTargetCalc)
was re-read from the listing: on success it writes back **raw steps** through the by-ref
range (`:0532-053A`), the budget test `steps > range·2+1` lives inside the walk loop
(`:04DD-04E5`), and `sub_738D8` stores min-steps at the stride-3 record's `+1` (`:0AD7-0ADA`,
`:0B1C-0B2C`) — §20's reading reconfirmed, ours == coab == binary.

**The localization** (the capture's `turn_snapshot`s carry per-combatant `actions.target` —
the first draw-free state channel this peel has had): capture-MARK holds target **10** from
draw 1798 all the way to 2894; 10 is dead by then, so his turn-start `find_target`
invalidates and draws the d6 over all six live monsters. Ours held **7** (alive) instead —
held target, no retarget, walk, adjacent re-pick `d1`. The 1-vs-6 was pure downstream
fallout of a *held-target* divergence.

**Where ours drifted:** draw 2613 — MARK's **departure opportunity attack** on the fleeing
[7] (d20 @2613 hit + d2 @2614, [7] hp 7→5, snapshot-confirmed). Our `attack_target` applies
the §19 write-back (`actions.target = target`) unconditionally, so the opportunity attack
permanently retargeted MARK onto the fleer. The capture's snapshots show the truth:
t10 → **t7** (transiently, at the attack) → **t10** (immediately after).

**The binary** (`sub_3E954`, the departure scan): `ovr014:0C83-0C8E` loads
`actions.target` (offset+seg) into locals **before** the `AttackTarget` (`sub_3F9DB`) call
at `:0CAC`, and `:0CB3-0CC5` writes it **back** after. coab renders it faithfully
(`backupTarget`, ovr014.cs:405/410) — this was a transliteration miss on our side, not a
coab≠binary bug. The §19 write-back is real but *transient* on this path.

**Fix:** save/restore `fighters[att].target` around the departure `attack_target` call in
`move_step_away_attack`. Draw-neutral at the attack itself; only the held target carried
forward changes.

**Result: bar-rout frontier 2894 → 2895.** MARK's retarget draws the capture's exact d6,
picks [13] with the same roll, and walks the capture's exact path (27,14)→(35,16). The
residual at 2895: the capture has [11] — parked at (36,16) since its rout turn — swing an
**into-reach d20** at MARK as he arrives; ours never fires it. [11] ends its rout turn
`guarding=true` in ours too, but the flag does not survive to MARK's next-round arrival:
the cross-round guard layer, the next peel (§32).

## 32. Bug #15 — `guarding` survives `CalculateInitiative`; ★ BAR-ROUT CLOSED 3521/3521 ★ (2026-07-20, Fable)

§31's residual named itself in one instrumented pass: [11] ends its rout turn via
`TryGuarding` (delay 1 → `guarding = true`), exactly as the binary must — but our
`calculate_initiative` cleared `guarding` at the next round boundary, so when MARK arrived
adjacent one pass later, the into-reach attack (`sub_3E65D`: `guarding && !IsHeld`) had
been disarmed.

**The binary:** `sub_3E000` (`CalculateInitiative`) resets exactly `actions.spell_id`,
`can_cast`, `field_2` (can_use), `field_8`, `field_4` (attackIdx = 2), `field_5`
(attack2_AttacksLeft), `delay`, and `move` (`ovr014:0017-011A`) — **the guarding byte is
never touched**. A guard armed in round N fires in round N+1 (or any later round) the
moment an enemy steps into reach; only the firing itself (`sub_3E65D` clears the flag) or
an `Action.Clear` disarms it. coab agrees (ovr014.cs:8-54 — no `guarding` write). Our
`guarding = false` in the reset was an over-transliteration, invisible until the rout
produced the first parked guard whose victim arrived in a later round.

**Fix:** delete the reset. One line.

**★ RESULT: `bar-rout-58c50` CLOSED — 3521/3521 operand-exact, equal length, zero stub
trips ★** — [11]'s into-reach d20 fires at 2895 (miss), MARK's adjacent re-pick d1 lands at
2896, his swing d20 at 2897 hits, the d2 damage at 2898 drops [11] to hp 10, and the
remaining 623 draws replay draw-for-draw through the PartyWins exit. Manifest pin flipped
to `Closed` in this commit; the guard holds 5/5 with the other four captures unshifted
(guarding never survived a round boundary in the zero-rout captures — every guard there
fired or was cleared within its own round).

**The five-capture matrix after this slice:**

| capture | status |
|---|---|
| `combat4` | CLOSED 3075/3075 |
| `combat3+terrain4` | CLOSED 3218/3218 |
| `combat2+terrain4` | CLOSED 4260/4260 |
| `combat+terrain4` | frontier @368 (pre-existing, separate low-priority thread) |
| `bar-rout-58c50` | **CLOSED 3521/3521** |

The full flee subsystem — FleeCheck ladder (§29), behind-AC departure attacks (§30),
departure-target restore (§31), cross-round guards (§32) — is now capture-proven end to
end. The M5 peel loop's next targets: the armed/ranged capture, then the caster capture
(poke-pattern staging as needed), then the affects substrate ahead of spells.

## 33. The memorized-spells wire, binary-verified — the "@0x71, not @0x1E" misread, and the real gates (2026-07-21, Fable)

The staging session's save diff (one memorized Magic Missile → a single byte `0x00→0x0F`
at record `0x71`) was read as "the memorized list is @0x71, NOT @0x1E — the tripwire reads
the wrong offset." The binary says otherwise. `sub_3560B`'s collection loop
(`ovr010:062A-065D`) reads `record[0x1E + i]` for `i = 1..=0x53`: **the memorized list IS
the 84-byte array @0x1E** — it just **packs from the back**. coab's `SpellList.Save`
(`Classes/SpellList.cs`) fills from index 83 down, so the FIRST memorized spell lands at
`0x1E + 83 = 0x71` = `spell_list[83]`. Slot 0 (@0x1E itself) is never read — the loop is
1-based — so the faithful `spells_count` window is `spell_list[1..]` (bytes `0x1F..0x71`).
Capture records confirm: caster-bar PHILIPPE carries `{0x71: 0x0F}`; bar-fists-2 PHILIPPE
carries `{0x70: 0x0F, 0x71: 0x0F}` — the "wrong save, no spells" capture actually has TWO
memorized Magic Missiles.

**The real defect was the wire's missing gates, not the offset.** The binary enters the
selection loop — the DRAWS (3× `roll_dice(spells_count,1)` per priority pass under the
unconditional d7 bound) — only when ALL of (`ovr010:0679-06A7`):

1. `spells_count > 0` (collected under `actions.can_cast`, reset true each round by
   `CalculateInitiative`);
2. `control_morale >= 0x80` (NPC-controlled) **or** `AutoPCsCastMagic` (`byte_1D904`,
   `ovr010:068D`; '2' toggles it, `BattleSetup` resets it false @ovr011.cs:1186);
3. a live opponent exists (`friends_count`/`foe_count`, ovr010.cs:255).

Capture-proof: **bar-fists-2 closes 3811/3811 with two memorized slots and zero spell
draws** — magic was never toggled on, so a PC caster's slots are inert. The ungated wire
fired 8× on that replay (a wolf-cry that would have blocked pinning it Closed); the gated
wire is silent there and fires on caster-bar exactly where the unmodeled draws live.

**Landed:** the gated wire + the `[1..]` slot window; `CombatState.auto_pcs_cast_magic`
(input-only, default false = the BattleSetup reset); `RESTRIKE_AUTO_CAST=1` knob in
`h4_replay`/`h4_turndiff`. Matrix: bar-fists-2 **CLOSED 3811/3811, zero trips**;
caster-bar knob-off silent, diverges @453 unchanged; knob-on trips at PHILIPPE's turns and
still diverges @453 (the flag feeds only the wire today); the four older captures carry
empty spell windows and are untouched.

**The toggle-window finding (matters for the future caster peel):** with the knob armed,
the wire trips at PHILIPPE's ROUND-1 turn (draw ~83) — but the capture's first selection
draws are @453, his ROUND-2 turn. So Bryan's '2' press landed BETWEEN PHILIPPE's round-1
and round-2 turns (the staging note "before his first turn" is corrected by the capture
itself). "On from entry" is draw-equivalent for the WIRE, but once the selection draws are
modeled, a from-entry flag would draw 3× d1 at his round-1 turn and diverge @~83 — the
caster slice must model the flip window (arm the flag after his round-1 turn), or the
staging hook must emit toggle events.

**Cited, not modeled (coab≠binary nuance):** the binary collects ANY non-zero slot byte
(`cmp ..,0`/`jbe` ≡ `jz` @`ovr010:0637-063C`) — including high-bit "learning" entries
(`id | 0x80`, memorization begun but rest not completed) — and would pass the raw byte to
`ShouldCastSpellX`; coab's `LearntList()` filters `Learning` entries and masks `0x7F`
(`SpellList.AddLearnt`). A caster who fights mid-memorization diverges between the two.
No capture exercises it; the wire's any-non-zero count matches the binary.

## 34. SPEC — faithful ranged combat (M5 armed slice; Fable-scoped, implementer-built) (2026-07-21)

**Goal: `armed-bar.gbxtrace` CLOSED 2749/2749** (guard pin flipped in the closing commit),
all other pins unshifted. The capture's fight: MATHEW (long bow) and TRAVIS (short bow)
shoot from range; patrons swarm; MATHEW is cornered rounds 1–6 and punches; the bows come
back out when the room clears (round 7+). Everything below is binary-cited; coab is
reference only. Two coab≠binary bugs found at spec time are flagged **(#16)** and **(#17)**.

### 34.1 The input model — per-combatant loadout + the ITEMS table

The capture's records carry runtime far pointers for the readied weapon (`field_151`
@0x151), the items list (`itemsPtr` @0x14D), and the ammo slots (`player_ptr_03` @0x17D =
arrows, `player_ptr_04` @0x181 = quarrels — `sub_6906C` reads exactly these two), so item
identity/ammo counts are NOT recoverable from a snapshot. Two additive inputs:

1. **`ItemDataTable`** — the game file `ITEMS` (`<gamedir>/ITEMS`, 2-byte header + 0x81
   entries × 16 bytes; resident copy `seg600:5D10` = `unk_1C020`). Entry layout (all
   binary-verified read sites): `[0]` item_slot, `[1]` handsCount, `[2]/[3]` diceCount/
   diceSizeLarge, `[4]` bonusLarge (sbyte), `[5]` numberAttacks (HALF-attacks),
   `[9]/[0xA]` diceCount/diceSizeNormal, `[0xB]` bonusNormal (sbyte), `[0xC]` range,
   `[0xD]` classFlags, `[0xE]` flags. Flags: `0x01` arrows, `0x02` flag_02, `0x04` melee,
   `0x08` launcher, `0x10` self-launching, `0x80` quarrels. New `gbx-formats` parser +
   harness loads it from the local game dir (D10: the file itself stays local; unit tests
   use synthetic entries). Rows in play (from Bryan's GOG `ITEMS`):

   | type | name | hands | large | natk | normal | range | flags |
   |---|---|---|---|---|---|---|---|
   | 43 | LongBow | 2 | 1d6+0 | **4** | 1d6+0 | **22** | 0x0B |
   | 44 | ShortBow | 2 | 1d6+0 | 4 | 1d6+0 | **16** | 0x0B |
   | 73 | Arrow | slot 10 | 1d6 | 0 | 1d6+0 | 0 | 0x00 |
   | 47 | Sling | 1 | 1d6+1 | 2 | 1d4+1 | 21 | **0x0A** |
   | 36 | LongSword | 1 | 1d12 | 0 | 1d8+0 | 0 | 0x04 |

2. **Per-combatant loadout** (committed per capture in the harness, like the guard's
   `PINS`; `None` = today's behavior — range-1 melee, record profile as-is, items_selection
   inert): `{ primary_type, ammo_count, unarmed_profile: (count,size,bonus) }`. armed-bar:
   MATHEW `{43, 40, (1,2,6)}`, TRAVIS `{44, 40, (1,2,3)}`, all others `None` (MARK/LEDERA's
   swords act through their record profile exactly as in the closed fist captures).
   The readied (entry) profile comes from the record (@0x19E/0x1A0/0x1A2); the unarmed
   profile = base dice @0x11E/0x120 + the STR damage adj — pinned empirically by the same
   characters' fist captures (MATHEW +6, TRAVIS/MARK/LEDERA +3, SHARA +1, PHILIPPE +2).
   Ammo 40 is a free parameter: no in-capture depletion (MATHEW fires 6, TRAVIS ≤20);
   any count ≥ shots-fired replays identically.

### 34.2 The predicates (`ovr025`)

- **`is_weapon_ranged`** (`offset_above_1` @`ovr025:2FE4`): `field_151 != null &&
  ItemDataTable[type].range > 1` (reads the TABLE range byte, `jbe` → false on ≤1).
- **`is_weapon_ranged_melee`** (`offset_equals_20` @`ovr025:3027`): the above AND
  `(flags & 0x14) == 0x14` (self-launching + melee: HandAxe 0x14 yes; Dart 0x1A no).
- **`GetCurrentAttackItem`** (`sub_6906C` @`ovr025:306C`): from the primary's flags:
  `0x10` → the item itself; `0x08` → `0x01`→arrows slot / `0x80`→quarrels slot; returns
  `found != null || flags == 0x0A` — **a Sling/StaffSling (flags 0x0A) "finds" a null
  item and still shoots** (no ammo consumed; the staging note's "slings need no ammo").

### 34.3 Attack counts (`sub_3EDD4` @`ovr014:0DD4`, called by CalcInit + items_selection)

Faithful transliteration (coab ovr014.cs:462 is accurate here):
`orig = rec[0x19C]; rec[0x19C] = rec[0x11C];` then if ranged && GetCurrentAttackItem:
`half = max(2, table[type].natk)` else `half = rec[0x19C]`. `attacks =
ThisRoundActionCount(half)` (`sub_3EF0D`: `(half + (combat_round & 1)) / 2`). Ammo cap:
`cap = max(1, item.count); if cap < attacks && item.count > 0 → attacks = cap` (item.count
@item+0x39; skipped entirely for a null item — slings). Write-back gate: write `rec[0x19C]
= attacks` iff `!field_8 || attacks < orig || (field_8 && attacks < orig*2 && !ranged)`.
LongBow natk 4 → 2 shots every round (`(4+parity)/2`). **CalcInit tail** (`sub_3E000`
@`:0041-0073`): `rec[0x19D] = ThisRoundActionCount(rec[0x11D])` — attack2's half-count is
record @0x11D (all zero in this party → attack2 never swings here); `actions.field_5
(maxSweapTargets) = rec[0xDD]`. The attacks-left cells are RECORD-resident: `rec[0x19B+idx]`
(idx 1 → 0x19C, idx 2 → 0x19D) — `sub_3F4EB`'s loop reads/decrements exactly those.

### 34.4 The AI turn (`sub_35DB1` @`ovr010:0DB1`) — range, near list, adjacency

- **Range** (@`:0EE0-0F0E`): `range = table[primary.type].range − 1` when `field_151`
  non-null, else 1; sanitize `{0, 0xFF} → 1`. LongBow → 21, ShortBow → 15.
- The held-target reach test and `BuildNearTargets` both use THIS range (a bowman's near
  list spans the room — the round-0 `d10` @57 is find_target's, and his re-pick lists are
  weapon-range wide).
- **The cornered re-pick block** (near-pick branch only): picked target + `is_weapon_ranged
  && !ranged_melee && BuildNearTargets(1).Count > 0` → `AI_items_selection` + stop (no
  attack this turn). A held-and-reachable target does NOT consult this block — the swap
  happens via step-7 items_selection the next turn.
- **Attack execution**: if ranged → `GetCurrentAttackItem(out item)`; if `ranged_melee &&
  targetRange == 1` → `item = null` (thrown weapon used as melee). Then
  `AttackTarget(item, 0, target, player)`.
- **`TryGuarding`** (`sub_361F7` @`ovr010:11F7`): `IsHeld || is_weapon_ranged ||
  delay == 0` → `clear_actions` (a ranged attacker NEVER parks a guard); else `guarding`.

### 34.5 The weapon-selection AI (`sub_36673` @`ovr010:1673` + `sub_36535`)

Runs every AI turn (step 7, ovr010.cs:79) and inside the cornered block. Faithful scope
for this slice = the PRIMARY path over the loadout (candidates: the loadout weapon vs
bare hands); the secondary/shield branches and multi-item lists are cited-deferred with a
tripwire (`items-selection-secondary`) since every loadout here has ≤1 weapon + ammo.

- `CalcItemPowerRating` (`sub_36535`): `rating = dsN*dcN + plus*8 (if >0) + bonusN*2 (if
  >0) + (flag_08 ? (natk−1)*2 : 0) + (hands ≤ 1 ? 3 : 0)`; zero if hands+used > 3 /
  cursed / (affect cases cited). LongBow: 6+6=12. Baseline `var_16` = base profile
  `dsB*dcB (+2*bonusB if >0)` = 2.
- Decision: ranged candidate wins iff `rating > var_16>>1 && ammo-available && (ranged_melee
  || BuildNearTargets(1).Count == 0)`; else best melee candidate (None here → bare hands).
- Ready/unready via `ready_Item` toggle + `reclac_player_values` + `reclac_attacks` at the
  tail — the observable: **cornered bowman unreadies the bow → attack-1 profile becomes
  the unarmed profile; clear again → re-readies, profile restored.** This is exactly
  armed-bar MATHEW: rounds 1–6 single d2+6 punches (`4 7 7 | 20 2` turns), round 7+
  double d6 shots again (@2350: `d3` retarget, `20 6 20 6` kills patron 8).
- Our engine models the swap as: profile1 := loadout.unarmed_profile on unready; := the
  saved entry profile on re-ready; attacks recomputed via §34.3 both times.
  (`reclac_player_values`/`sub_66C20` full transliteration stays deferred.)

### 34.6 The attack (`sub_3F9DB` @`ovr014:19DB` → `sub_3F4EB` @`ovr014:14EB` → `sub_3E192`)

- `sub_3F9DB`: missile animation (item, plus Sling 0x2F/StaffSling 0x65 drawing the
  primary, @`:1B14-1B4F` — draw-free); gate `rec[0x19C] > 0 || rec[0x19D] > 0`; call
  `sub_3F4EB`; then **ammo write-back @`:1BB3-1BC7`: `if (item.count > 0) item.count -=
  byte_1D901`** (the attack-1 swing count; punches never decrement) — **coab≠binary #16:
  coab ASSIGNS `count = bytes_1D900[1]` (ovr014.cs:968) where the binary SUBTRACTS.**
  Depletion (`count == 0`): ranged_melee && `affect_3 != 0x89` → clone-unreadied into the
  dropped-items list + `lose_item`; else plain `lose_item` (the arrows item vanishes);
  then `reclac_player_values(attacker)` — a depleted bowman punches from the next swing
  batch on. Unexercised by armed-bar (counts ≥ usage) — implement (it is cheap), no wire.
- `sub_3F4EB` (per doc §30 plus this session's full read): held-target auto-slay branch
  (@`:153E-15E0`, cited-deferred — no held targets here); large-target dice substitution
  (@`:15E3-1665`, `field_DE > 0x80 || (field_DE & 7) > 1` → table large dice/bonus swap,
  cited-deferred — patrons are man-sized); `CanBackStabTarget` (`sub_408D7`) → `target_ac
  = ac_behind − 4`; else flanking heuristic (§30) → BehindAttack; **`target_ac +=
  RangedDefenseBonus` BY REFERENCE on every path** (`sub_3FCED` @`ovr014:1CED`:
  `third = ranged ? (table.range−1)/3 : targetRange`; two bands: `range > third` → +2,
  again → +3; LongBow: +2 beyond 7, +5 beyond 14); the swing loop @`:1743-1878`: `for
  idx = actions.attackIdx down to 1: while rec[0x19B+idx] > 0 && !targetGone: dec cell,
  bytes_1D900[idx]++, PC_CanHitTarget(target_ac) → hit: sub_3E192(idx) + affects hooks`.
  The `bytes_1D900`/`bytes_1D2C9` counters are ZEROED in `sub_3F4EB`'s prologue
  (`ovr014:14FE-1512`: `byte_1D2CA/1D2CB/1D901/1D902 ← 0` every call), so the swing
  count the ammo write-back subtracts is per-`AttackTarget01`-call — a call-local
  counter is the faithful model (review finding #10, cited).
- `sub_3E192` (@`ovr014:0192`): **damage = `roll_dice(size@rec[0x19F+idx],
  count@rec[0x19D+idx]) + (sbyte)rec[0x1A1+idx]`, clamped ≥0** (idx 1 → the 0x19E/0x1A0/
  0x1A2 profile our decode already carries; idx 2 → 0x19F/0x1A1/0x1A3). Then the thief
  **backstab multiplier** (@`:01F3-0229`, exact factors from `sub_6B3D1` + rec[0x117]/
  rec[0x10F] — implementer reads both): armed-bar EXERCISES it — TRAVIS (Fighter/Thief)
  @d2496: `20 2` punch kills a 9-hp patron (d2+3 ≤ 5 unmultiplied — the ×2-at-his-level
  backstab is the only fit). `CanBackStabTarget` (`sub_408D7`) is therefore IN scope.
- **coab≠binary #17 (flag, verify during implementation):** coab re-assigns `byte_1D90E =
  GetCurrentAttackItem(...)` before `AttackTarget` in sub_35DB1 but never re-checks it —
  confirm the binary's exact use (an out-of-ammo bowman's swing path) when transliterating.

### 34.7 Capture walk (what closes when this lands)

Round 0 (@53): MATHEW `8 4 7 7 | 10 | 20 6 [11 −1] 20` — two shots, hit+miss, from
(26,12) with no move. Rounds 1–6: cornered — step-7 items_selection unreadies (adjacent
patron), one `20 2` punch per round on the held target ([11] 15→7→0 across rounds 1–2).
TRAVIS shoots 2/round from dist 2 (e.g. @709 `20 6 20 6`). Round 7 (@2344): room clear →
re-ready, `d3` retarget, `20 6 20 6` kills 8. Round 8 (@2514): `1` pick, `20 20 6`.
TRAVIS round 8 (@2492): cornered punch **backstab** kill. Patron paths (walk d100s, `d6`
picks, d6 punches) are unchanged from the closed fist captures.

### 34.8 Acceptance + discipline

1. `armed-bar` **CLOSED 2749/2749** (operand-exact, equal length, zero trips) — flip its
   pin in the same commit; guard 8/8 (all others unshifted — loadout `None` must be
   draw-identical to today's engine, which the 6 non-armed pins prove).
2. Workspace tests + clippy `-D warnings` + fmt + guard before every commit; one
   mechanic per commit, binary-cited; never weaken an assert; doc § notes ride along.
3. New tripwire `items-selection-secondary` (a loadout with a secondary/shield or >1
   candidate weapon reaches the deferred branches). Existing wires untouched.
4. Unit tests: reclac ranged counts (natk floor/parity/ammo cap/field_8 tail), predicates
   (incl. sling 0x0A), range sanitize, RangedDefenseBonus bands, ammo subtract-not-assign
   (#16), cornered swap (unready → punch profile → re-ready), TryGuarding ranged clear.
5. Localizer: `GBX_DRAW=<n> GBX_H4_TURNDIFF=.../armed-bar.gbxtrace cargo test -p
   gbx-oracle --test h4_turndiff h4_locate_draw -- --nocapture`.

## 35. LANDED — faithful ranged combat, armed-bar 58 → 2019; the facing subsystem is the residual (M5 armed slice, 2026-07-22)

The §34 spec was implemented on branch `m5-ranged` off `main` (5ff9cfb). **Every §34
site was re-verified against `coab_new.lst` before coding**; the three flagged
own-reads — `sub_408D7`/`sub_6B3D1` (backstab) and coab≠binary #17 — were read from
the listing and are settled below. `armed-bar.gbxtrace` moved from `Frontier(58)` to
**`Frontier(2019)`** (of 2749); the other seven pins held unshifted at every commit
(loadout `None` is draw-identical). It did **not** close — the residual is the
facing/direction subsystem (below), which regresses the closed captures under two
transliterations and needs its own slice.

**What landed (six commits, one mechanic each):**
- **#1 `7fe2326`** — `gbx-formats` `ITEMS` parser (`ItemDataTable`): 2-byte header +
  0x81 × 16-byte entries, zero-filling the tail; synthetic units + a local-tier test
  over Bryan's real `ITEMS` verifying the §34.1 rows.
- **#2 `4340624`** — plumbing: `Loadout` + the per-combatant ranged fields on
  `Combatant`, `CombatState.item_data` + `set_loadout`, `skill_level_thief`; the shared
  harness loadout table (`tests/common/mod.rs`) wired into all three harnesses (the §30
  shared-knobs rule). All 8 pins unchanged.
- **#3 `13ddf9a`** — predicates (`is_weapon_ranged`/`_melee`, `GetCurrentAttackItem`,
  incl. the Sling 0x0A null-item find), `weapon_range` (LongBow 21/ShortBow 15,
  sanitize), and `reclac_attacks` (natk floor → 2 shots/round, ammo cap, field_8 gate);
  `CalculateInitiative` now calls `reclac_attacks` + resets `field_8`. **58 → 493.**
- **#4 `78c9532`** — the ranged attack: `RangedDefenseBonus` on every path, ammo
  subtract + depletion, idx-indexed damage cells, `field_8 = true`. **493 (held —
  round-0 shots already correct; RangedDefenseBonus is exercised there).**
- **#5 `d1c4de0`** — `AI_items_selection` (the cornered swap: `CalcItemPowerRating` vs
  the base profile, ammo availability, adjacency → bow-vs-fists), wired at step-7 + the
  cornered re-pick block; `TryGuarding`'s ranged clear. **493 → 1910.**
- **#6 `39d876a`** — the TRAVIS ammo-depletion finding (below) + the `h4_locate_draw`
  diagnostic (prints our operands beside the capture's, dumps our roster at the divergent
  draw). **1910 → 2019.**

**Capture matrix (before → after):**

| capture | before | after |
|---|---|---|
| `combat4` | CLOSED 3075/3075 | **CLOSED** (unchanged) |
| `combat3+terrain4` | CLOSED 3218/3218 | **CLOSED** (unchanged) |
| `combat2+terrain4` | CLOSED 4260/4260 | **CLOSED** (unchanged) |
| `combat+terrain4` | frontier @368 | **@368** (unchanged) |
| `bar-rout-58c50` | CLOSED 3521/3521 | **CLOSED** (unchanged) |
| `armed-bar` | frontier @58 | **frontier @2019** |
| `caster-bar` | frontier @453 | **@453** (unchanged) |
| `bar-fists-2` | CLOSED 3811/3811 | **CLOSED** (unchanged) |

**Deviations found (binary-cited):**
1. **coab≠binary #16 CONFIRMED** — `ovr014:1BBD-1BC3` (`mov al, byte_1D901; sub
   es:[di+item.count], al`): the binary **subtracts** the attack-1 swing count from
   `item.count`; coab assigns (`count = bytes_1D900[1]`, ovr014.cs:970). Implemented as
   subtract.
2. **coab≠binary #17 CONFIRMED (dead)** — `ovr010:1176` re-assigns `byte_1D90E =
   GetCurrentAttackItem(...)` but nothing re-reads it before the unconditional
   `sub_3F9DB` at `:11BF` (and it is reset to 0 at each loop top), so the attack proceeds
   regardless. Our transliteration uses only the returned item, ignoring its boolean —
   draw-equivalent.
3. **ITEMS entry count** — the spec (and coab, `ItemData.cs:52`) build **0x81** entries;
   Bryan's shipped `ITEMS` is **0x802 bytes = 2-byte header + 0x80 entries**, so entry
   `0x80` (`Type_128`) is a zero-fill in both coab (reads 0x810 into a zeroed buffer) and
   our parser. No behavioural effect (types in play are ≤ 73).
4. **§34.1 "ammo is a free parameter" is WRONG for TRAVIS** — the capture proves he
   empties a **10-arrow** quiver mid-fight; depletion (`lose_item` →
   `GetCurrentAttackItem` false → `AI_items_selection` unreadies the bow, `var_1F` false)
   switches him to fists and **changes the draw stream**. Ammo 40 (no depletion) diverges
   at 1910 (TRAVIS shoots where the capture shows him out of arrows and approaching);
   ammo 10 carries to 2019 (9 depletes a turn early → @1575; 11 never depletes in time →
   @1910 — a sharp optimum **under the current model**; 10 is an empirical FIT, not a
   derived quantity, and if the facing slice moves TRAVIS's shot count by one, 10 moves
   with it — revisit then). MATHEW fires few enough (§34.1: 6) that 40 holds. The
   loadout table pins TRAVIS at 10 with the fit flagged at the constant.
5. **`field_DE = 0x01` for the patrons** (capture-decoded) — so the backstab size gate
   `(field_DE & 0x7F) <= 1` passes, and the large-target dice substitution
   (`> 0x80 || (& 7) > 1`) stays off (man-sized), as §34.6 assumed.
6. **The backstab factors settled** (own read, `ovr014:01F9-021F`): `multiplier =
   ((SkillLevel(Thief) − 1) / 4) + 2` with `SkillLevel(Thief) = rec[0x10F] (ClassLevel[6])
   + rec[0x117] (ClassLevelsOld[6]) * sub_6B3D1`; `sub_6B3D1 =
   DualClassExceedsPreviousLevel` (0/1). TRAVIS is Fighter 4 / Thief 5 → SkillLevel 5 →
   ×3. `CanBackStabTarget` (`sub_408D7`) weapon list = {null, Club 7, Dagger 8,
   BroadSword 35, LongSword 36, ShortSword 37, DrowLongSword 97}. Transliterated (see
   the deferred note) but not landed — it over-fires (below).

**The residual @2019 and the facing-subsystem blocker (STOP-and-report).** At draw 2019,
patron [14] attacks MARK: the capture draws `d6` (a hit → damage), ours draws the next
selection `d100` (a miss) — same `d20` @2018, different to-hit AC. MARK is swarmed
(`AttacksReceived > 1`); the capture hits his **behind AC 48** where ours uses front 53.
This is the **flanking heuristic** (`ovr014.cs:782`: `AttacksReceived > 1 &&
getTargetDirection(target, attacker) == direction && directionChanges > 4 → BehindAttack`)
— cited-deferred in §30, but armed-bar **exercises** it. The next residual (~2496) is
TRAVIS's cornered-punch **backstab** kill (§34.7). **Both need faithful `target.direction`
tracking**, and two transliterations of it — (a) the `sub_3F9DB` @913-927 attack-turn
update + `sub_3F94D` `directionChanges` + the flanking test + `CanBackStabTarget`; (b) the
same with the flanking test disabled — **each regresses the closed captures** (combat4
@618 with flanking, @1053 with only the backstab reading `direction`). The cause: the
post-attack "face away" update (`direction = (getTargetDirection(attacker,target)+4)%8`)
makes the facing check `getTargetDirection(target,attacker) == direction` pass on the very
next same-attacker hit — so the backstab/flanking fire where the real game (whose captures
close **without** them) did not. coab renders the same algorithm, so this is a
transliteration bug in the direction bookkeeping (candidate misses: the target-direction
update also fires on `draw_74B3F` at each icon redraw, not only the one AttackTarget site;
`PlayerMapPos` vs our grid pos; or the `AttacksReceived`-parity timing vs
`RecalcAttacksReceived`). **The facing subsystem is the next slice** — it must be built
and validated step-by-step against the five closed captures (the canary) BEFORE the
flanking/backstab land, since both read it. Until then the reverted engine holds all five
closed captures and armed-bar at the true `Frontier(2019)`.

**Review fixes (PR #6 review, 2026-07-22).** All ten findings addressed in one commit:
`set_loadout` now snapshots `entry_dice` from the live profile (a hand-built combatant
survives the unready→re-ready round trip; capture path unchanged — the values were
already equal); the panicking `Index<u8>` impl on `ItemDataTable` is deleted (zero
consumers; `get()` is the untrusted-input path); the write-only `direction_changes`
field is dropped (it described an accumulator nothing maintained — the facing slice
re-adds it WITH its maintainer), and `field_de`/`thief_skill_level` are marked
**unread until the facing slice** (record-derived, kept); `weapon_range` drops the
mis-ported i32 `0xFF` arm (the binary's byte-space sanitize is exactly `{0, −1}` in
i32; table range 255 would legitimately give 254); the ammo write-back now also
decrements a self-launching weapon's own count, with a new **`self-weapon-depleted`**
tripwire naming the unmodeled depletion (primary nulls at once; ranged-melee
clone-drop `ovr014:1BD4-1C54`); the §34.8(3) `items-selection-secondary` tripwire is
**closed out as moot** — `Loadout` holds one weapon by construction, so the deferred
secondary/shield branches are unreachable until a multi-weapon loadout exists (the
wire lands with that loadout); `h4_replay` loud-skips a loadout-bearing capture when
`ITEMS` is absent (mirroring the guard) instead of asserting bafflingly at draw ~58;
the duplicated `(flags & 0x14) == 0x14` collapses into `candidate_ranged_melee`
(type-level, shared by the actor predicate and the candidate scan); TRAVIS's
`ammo_count: 10` is flagged **FITTED, not derived** here and at the constant; and the
`bytes_1D900` per-call zeroing is now cited (`ovr014:14FE-1512`) — the call-local
swing counter was an assumption, now a citation. Guard 8/8 after all of it; armed-bar
exactly @2019.

## 36. SPEC — the facing subsystem: direction bookkeeping + the combat camera (M5 facing slice; Fable-scoped, implementer-built) (2026-07-22)

`armed-bar` sits at `Frontier(2019)`: patron [14] hits swarmed MARK's **behind AC 48**
(the flanking heuristic) where ours uses front 53; the next residual (~2496) is
TRAVIS's cornered-punch backstab kill. Both read the target's `actions.direction` —
and §35 records that two straight transliterations of the direction update REGRESS
combat4 (@618 flanking-on, @1053 backstab-only). This section is the RE that cracks
that contradiction, and the spec for the substrate that fixes it. Every site below
was read in `coab_new.lst` (binary = spec); coab agrees at every verified site — the
§35 failures were OUR transliteration losing a store, not coab≠binary.

### 36.1 The crack — why "face away" over-fires

`AttackTarget` (`sub_3F9DB`) updates the **target's** facing like this (binary
@`ovr014:19FE-1A9F`, coab ovr014.cs:913-931):

1. `AttacksReceived < 2 && attackType == 0` (@`19FE-1A09`): `var_9 =
   getTargetDirection(attacker, target)` — the bearing **from the target toward the
   attacker** — then store `direction = (var_9 + 4) % 8` (**face away**, @`1A35`).
2. else if target on-screen (@`1A3B-1A79`): `var_9 = direction`; if `attackType ==
   0`, store `(var_9 + 4) % 8` — a 180° flip.
3. **Shared tail** (@`1A7D-1A9F`): if target on-screen, `draw_74B3F(false, Normal,
   var_9, target)` — and `sub_74B3F` stores its direction argument
   **unconditionally** (@`ovr033:0BCC-0BD7`, after the focus-gated recenter). The
   draw is a state mutator.

Net semantics (melee `attackType == 0`):

| case | target's resulting `direction` |
|---|---|
| 1st attack since reset, target **on-screen** | bearing target→attacker (**faces the attacker** — the draw overwrites the face-away store) |
| 1st attack since reset, target **off-screen** | bearing+4 (**faces away** — the store stands, no draw) |
| 2nd+ attack, on-screen | unchanged (flip stored, then draw restores the old value) |
| 2nd+ attack, off-screen | unchanged (no store, no draw) |
| any `attackType != 0` (departure/behind) | unchanged |

Then the **attacker always faces its target**: `draw_74B3F(false, Attack,
getTargetDirection(target, attacker), attacker)` @`1AC2-1AD2` — unconditional call,
unconditional store.

The §35 attempts implemented only the face-away store. In melee the target is
almost always on-screen, so the real engine leaves the target **facing** its
attacker — and the flanking/backstab facing test `getTargetDirection(target,
attacker) == target.direction` ("the attacker is directly behind the target")
**fails** for that same attacker's next swing. With only face-away modeled, the
test passes — flanking/backstab fire where the binary's don't. combat4 @618/@1053
explained.

### 36.2 The facing state — fields, offsets, every write site

Action offsets (Classes/Action.cs, listing-confirmed): `direction`@0x09,
`AttacksReceived`@0x0F, `directionChanges`@0x12, actions ptr @`charStruct+0x18D`.

Writes/resets (QuickFight-exercised; manual-UI sites `sub_33B26`/`ovr009:608`/
`ovr014:1833` are out of scope):

- **Entry** (`sub_380E0` @`ovr011:1162-116E`, coab ovr011.cs:803-807): `direction =
  HalfDirToIso[map_direction/2]`, `HalfDirToIso = {7,2,3,6}` (`unk_1660C`); enemies
  `+4 % 8` (@`1185`+). md=2 → party 2, enemies 6. `AttacksReceived`/`directionChanges`
  start 0 (fresh Action).
- **Turn head** (`sub_33281` @`ovr009:028F-02A9`, coab ovr009.cs:105-107): the
  acting combatant's OWN `AttacksReceived = 0`, `directionChanges = 0`, `guarding =
  false` — before the delay>0 turn body. (Our engine has NO turn-head resets today —
  they land here. Consistent with §32/bug #15: guards clear at their own next turn.)
- **Every movement step** (`sub_3E748` @`ovr014:0902-090F`, coab ovr014.cs:312-313):
  the mover's `AttacksReceived = 0` and `directionChanges = 0`, after the pos write
  (ours has the first; the second lands now). Swarm state is per-position.
- **Every step's heading** (ovr010.cs:476): `draw_74B3F(false, Normal, step_dir,
  mover)` → `direction = step_dir` (already in our engine at the same point).
- **`RecalcAttacksReceived`** (`sub_3F94D` @`ovr014:194D-19D8`, coab 887-901):
  `AttacksReceived++` (@`195B`); `dirDiff = ((getTargetDirection(attacker, target) −
  direction) + 8) % 8` (bearing target→attacker vs current facing), folded `> 4 → 8 −
  dirDiff` (@`1996-19A8`); **`directionChanges = (directionChanges + dirDiff) % 8`**
  (@`19C2-19D1`) — the accumulator is mod 8, values only ever 0..7. Called
  IMMEDIATELY BEFORE `AttackTarget` on every path: AI turn (ovr010.cs:651), guard
  into-reach (ovr014.cs:243), sweep per-sweeptarget (ovr014.cs:556). Ours increments
  at the right times but ignores the attacker — the accumulate lands now
  (`_attacker` becomes used).
- **AttackTarget's update** — §36.1's table (both stores + the draw overwrites).
- **`clear_actions` does NOT touch facing** (`Action.Clear` = delay/spell_id/
  guarding/move only) — `direction`, `AttacksReceived`, `directionChanges` survive.
- `direction_changes` returns as a `Combatant` field WITH its maintainer (the §35
  review dropped the write-only field; this slice is the maintainer).

`getTargetDirection` = `sub_409BC` — already ported bit-exact as
`target_direction(from, to)` (combat.rs), octant scan with the 0x26A/0x6A
fixed-point tangents. `getTargetDirection(B, A)` = `target_direction(A.pos, B.pos)`
(bearing from A toward B); positions are `CombatMap[i].pos` = our grid pos (all
size-1 in every capture).

### 36.3 The combat camera — required by the on-screen branch

`PlayerOnScreen` = is the combatant's cell inside the 7×7 map window at
`mapScreenTopLeft` (`CoordOnScreen`: screen coords 0..6 both axes; map 50×25;
`ScreenCenter = (3,3)`). Two variants: any-cell (arg 0) and all-cells (arg 1) —
identical for size-1, which is every combatant in every capture (tripwire a size>1
loadout). The camera never draws into the PRNG stream — it enters the draw stream
ONLY through §36.1's on-screen branch — but it is path-dependent state and must be
replayed exactly.

**`ScreenMapCheck(radius, pos)`** (ovr033.cs:296-341, the scroll primitive under
`redrawCombatArea(dir, radius, map)` where the probe is `map +
MapDirectionDelta[dir]`, dir 8 = in place): if `radius == 0xff` (force) or `pos`
outside the ±radius box around the current screen centre → step the centre
coordinate-wise all the way to `pos`, clamped to `centre.x ∈ [MapMinX+3,
MapMaxX−4]`, `centre.y ∈ [MapMinY+3, MapMaxY−4]`; write `mapScreenTopLeft = centre
− (3,3)`. Port this exactly (both the box test and the clamp bounds).

**Camera event census (QuickFight paths)** — each with its trigger; port the
control flow of each site, stub the rendering:

1. **Combat setup** (ovr011.cs:1208-1209): `mapScreenTopLeft = TeamList[0].pos −
   (3,3)` after placement (no clamp — verify vs listing at implementation).
2. **Turn head** (ovr009.cs, sub_33281 tail): `focus := (team == Ours ||
   PlayerOnScreen(actor))` (@`02FA-0318` region); `RedrawCombatIfFocusOn(true, 2,
   actor)` (`sub_75356`) = focus-gated `redrawCombatArea(8, 2, actor.pos)`.
3. **AI pre-attack** (ovr010.cs:639, gated on the attack actually proceeding):
   `redrawCombatArea(getTargetDirection(target, actor), 2, actor.pos)` — probe one
   step from the actor toward the target, radius 2. NOT focus-gated.
4. **AttackTarget** (`sub_3F9DB` @`19E1`): `focus := true`; the target-side
   `draw_74B3F` is only CALLED when the target is on-screen → its internal recenter
   can't fire for size-1; the attacker-side `draw_74B3F(…, attacker)` @`1AC2` is
   unconditional → internal recenter `(8, 3, attacker.pos)` if attacker fully
   off-screen && focus (@`ovr033:0B52-0B87`). Post-attack pair (ovr014.cs:1004-1005):
   `draw_74B3F(true, Attack, dir, attacker)` + `(false, Normal, dir, attacker)` —
   same recenter check, direction stores are no-ops (same value).
5. **Ranged shots** (`sub_40BF1` → `draw_missile_attack` `sub_67AA4`, ovr025.cs:
   882-1010): port the control-flow skeleton exactly — early-return on short paths
   (`var_AF − 2 < 2`); if either endpoint off-screen: span ≤ 6 both axes →
   `redrawCombatArea(8, 0xff, midpoint)` (force-scroll), else animate until the
   missile exits the screen-pixel box then `redrawCombatArea(8, 3, target.pos)` if
   the target is off-screen; both-on-screen → force-redraw at current centre
   (no-op scroll). Fires for the primary `item != null` AND again for
   Sling/StaffSling primaries (@`1B14-1B4C`).
6. **Kill/flee removal** (`RemoveFromCombat` `sub_644A7`, ovr024.cs:624):
   `RedrawCombatIfFocusOn(false, 3, victim)` — focus-gated scroll to every
   combatant leaving combat, BEFORE `size = 0`.
7. **Movement**, per step (ovr010.cs:474-488 + `sub_3E748` @ovr014.cs:289-309):
   `focus := (byte_1D90E || PlayerOnScreen(mover) || team == Ours)`; the step's
   `draw_74B3F` internal recenter (mover fully off-screen && focus → `(8,3,mover)`);
   `move_step_away_attack` head sets `focus := true` (ovr014.cs:361-362) per
   candidate attacker; inside `sub_3E748`: QuickFight && new pos off-screen && focus
   → `(8, 2, old_pos)`; after the pos write, focus → `(8, 3, new_pos)` (QuickFight
   radius 3); `move_step_into_attack` on a guard firing: `(8, 2, mover.pos)`
   (ovr014.cs:239).
8. **End of round** (ovr009.cs:393): `(8, 0xff, current centre)` — scroll no-op.
   Flee/rout movement uses the shared step machinery (7). The caster path's
   `MagicAttackDisplay` scroll is the caster peel's, not this slice's.

`focusCombatAreaOnPlayer` (`byte_1D910`) writers in scope: sites 2, 4, 7 above.

### 36.4 The reads (flip on one at a time, in this order)

1. **Departure-attack gate** — already faithful in our engine (cone `direction+6..
   +10` + `AttacksReceived == 0` + `delay > 0` disjunction, `ovr014:0BC0`); it now
   reads the NEW bookkeeping. No code change; the canary validates it.
2. **Flanking heuristic** (`AttackTarget01` @`ovr014:16BA-16E9`, coab 782-784):
   `!CanBackStabTarget && AttacksReceived > 1 && getTargetDirection(target,
   attacker) == target.direction && directionChanges > 4` → to-hit AC =
   `ac_behind`. (`BehindAttack` init = `attackType != 0` @`ovr014:14EB+29`.)
3. **Backstab** (`CanBackStabTarget` `sub_408D7`, coab 1425-1454 + @`169E-16A5`):
   attacker is a thief-classed party member with weapon ∈ {null, Club, Dagger,
   BroadSword, LongSword, ShortSword, DrowLongSword} (§35), `AttacksReceived > 1 &&
   (field_DE & 0x7F) <= 1 && getTargetDirection(target, attacker) ==
   target.direction` → AC = `ac_behind − 4`, damage × `((SkillLevel(Thief)−1)/4)+2`
   (TRAVIS ×3). Re-verify the full head of `sub_408D7` (class/team gates) before
   coding — §35 settled the factors, not the whole predicate.

### 36.5 Build order + acceptance (the canary discipline)

The five closed captures close WITHOUT flanking/backstab — they pin the substrate.

1. **Substrate commit(s)**: camera model + entry-init facing + turn-head resets +
   step `directionChanges` reset + Recalc accumulate + AttackTarget update sites —
   flanking/backstab still OFF. Guard must hold **8/8 EXACTLY** (closed stay
   closed; @368, @2019, @453 exact). Any shift = a bookkeeping bug or a listing
   misread; localize with `h4_locate_draw` at the shifted draw before proceeding.
2. **Flanking ON** (one commit): closed captures unmoved; armed-bar MUST advance
   past 2019 (expect into the ~2496 backstab region). Manifest edit rides the
   commit.
3. **Backstab ON** (one commit): closed captures unmoved; armed-bar toward
   **2749/2749 CLOSED** (flip the pin in the closing commit). **Re-check TRAVIS's
   FITTED ammo_count 10** (§35: if facing changes his shot count by one, the fit
   moves — 9 → @1575, 11 → @1910 under the old model).
4. Bank at a named boundary if a residual appears; do not force it.

Discipline unchanged: re-verify every cited site against `coab_new.lst` before
coding (`LC_ALL=C grep -a`); one mechanic per commit, binary-cited; instrument
then revert; never weaken an assert; workspace tests + clippy `-D warnings` + fmt +
guard 8/8 per commit; doc §37 landing note + manifest edits ride their commits.

## 37. LANDED — the facing subsystem; armed-bar 2019 → CLOSED 2749/2749; the last H4 bar-fight frontier (M5 facing slice, 2026-07-22)

The §36 spec was implemented on branch `m5-facing-impl` off the spec commit
(`6ffb129`). **Every §36 site was re-verified against `coab_new.lst` before
coding**; the §36 prediction held — coab agrees at every verified site, so the §35
regressions were OUR transliteration losing the on-screen draw overwrite, not
coab≠binary. `armed-bar.gbxtrace` moved from `Frontier(2019)` to **CLOSED
2749/2749**; the other seven pins held unshifted at every commit. The two §35
blockers (combat4 @618 flanking-on, @1053 backstab-only) are resolved by the
substrate — the reads now fire ONLY where the binary's do.

**What cracked it (§36.1).** `AttackTarget` (`sub_3F9DB`) stores the target's
"face-away" (`direction = (bearing target→attacker + 4) % 8`, @`1A35`), then — on
the shared tail, when the target is on-screen — `draw_74B3F` **overwrites** it with
the raw bearing (@`1A9F`), so an on-screen (i.e. almost every melee) target ends up
**FACING its attacker**. The facing-equality read `getTargetDirection(target,
attacker) == target.direction` therefore FAILS for that attacker's next swing, and
flanking/backstab don't fire where §35's face-away-only ports made them. The draw
is a state mutator; the camera model (`797e09b`) supplies the on-screen test it is
gated on.

**What landed (seven commits, one mechanic each):**
- **`797e09b`** — the combat camera: `mapScreenTopLeft` + `focus`,
  `ScreenMapCheck`/`redrawCombatArea`/`draw_74B3F` persistent-state effects, all
  §36.3 census scroll sites, rendering stubbed. 8/8 (camera reads nothing yet).
- **`8bda3b0`** — entry-init facing: `direction = HalfDirToIso[md/2]`
  (`unk_1660C = {7,2,3,6}`), enemies `+4 % 8` (`ovr011:1162-118E`). 8/8.
- **`b7ecac5`** — turn-head resets: the actor's own `AttacksReceived`/`guarding`
  (and, next commit, `directionChanges`) zero at the turn head, before the
  `delay>0` body (`sub_33281` @`ovr009:028F-02A9`). 8/8.
- **`b4045f0`** — the `direction_changes` field WITH its maintainer:
  `RecalcAttacksReceived` (`sub_3F94D` @`ovr014:194D-19D8`) accumulates
  `directionChanges = (directionChanges + dirDiff) % 8`, and the field zeroes at
  the turn head + every movement step. 8/8 (read by nothing yet).
- **`0d4aad2`** — the AttackTarget direction update (§36.1, `sub_3F9DB`
  @`ovr014:19FE-1AD2`): the target-side face-away/flip store + the on-screen draw
  overwrite + the attacker-always-faces-target draw. 8/8 EXACTLY — the crux, still
  read by nothing.
- **`1ef5610`** — **flanking ON** (`AttackTarget01` @`ovr014:16AD-16E9`):
  `AttacksReceived>1 && target_direction(attacker,target)==direction &&
  directionChanges>4 → ac_behind`. **2019 → 2517** (manifest edit rode the commit).
- **`e5d0478`** — **backstab ON** (`CanBackStabTarget` `sub_408D7`
  @`ovr014:28D7-29B9`): thief + listed weapon + swarmed + man-sized + back-turned →
  `ac_behind − 4`, damage × `((SkillLevel(Thief)−1)/4)+2`. **2517 → CLOSED**.

**Capture matrix (§35 end → §37):**

| capture | before | after |
|---|---|---|
| `combat4` | CLOSED 3075/3075 | **CLOSED** (unchanged) |
| `combat3+terrain4` | CLOSED 3218/3218 | **CLOSED** (unchanged) |
| `combat2+terrain4` | CLOSED 4260/4260 | **CLOSED** (unchanged) |
| `combat+terrain4` | frontier @368 | **@368** (unchanged) |
| `bar-rout-58c50` | CLOSED 3521/3521 | **CLOSED** (unchanged) |
| `armed-bar` | frontier @2019 | **CLOSED 2749/2749** |
| `caster-bar` | frontier @453 | **@453** (unchanged) |
| `bar-fists-2` | CLOSED 3811/3811 | **CLOSED** (unchanged) |

**Findings (binary-cited):**
1. **No coab≠binary in §36** — as §36 predicted, coab renders every verified site
   faithfully (`ovr014.cs:913-936` facing, `782-784` flanking, `1433-1457`
   backstab, `887-901` recalc, `ovr009.cs:105-107` turn head). The §35 failures
   were the dropped draw overwrite alone.
2. **`sub_409BC` push-order convention settled** — by the operands at the known
   sites, `sub_409BC[A pushed first, B pushed second] = target_direction(A.pos,
   B.pos)` (bearing A→B). So the flanking/backstab facing test
   (`getTargetDirection(target, attacker) == direction`, @`16C9-16D4` /
   @`298B-29A3`, attacker pushed first) is
   `target_direction(attacker.pos, target.pos) == target.direction` (the target's
   back is to the attacker); the recalc bearing (`getTargetDirection(attacker,
   target)`, @`196C`) is `target_direction(target.pos, attacker.pos)` (bearing
   target→attacker) — the OPPOSITE argument order. Both coded from the stated
   semantics, then cross-checked against the push order.
3. **Distance-1 octant quirk (faithful, documented)** — the ported `target_direction`
   (`sub_409BC`) classifies a purely-west adjacent vector as **SW (5)**, not W (6),
   because `lo(1) = (0x6A·1)/0x100` floors to 0 so the SW octant test solves first.
   This is exact binary behaviour (the fixed-point tangents), and it is the common
   melee-adjacency case — the recalc/facing math consumes it as-is.
4. **`can_backstab` weapon = null ⟺ `!weapon_readied`** — the binary reads
   `attacker.primaryWeapon` (`field_151`); our model maps null (bare hands, e.g. a
   depleted/unreadied loadout) to `weapon_readied == false`. Only `armed-bar`
   carries loadouts, and the guard holds 8/8 (no closed capture shifts), so no
   no-loadout thief spuriously backstabs — the mapping is capture-validated.
   TRAVIS's kill at 2517 is his bare-handed punch (quiver empty), T5 → ×3.
5. **The 2517 divergence localized** — a `d2 → d?` damage-roll divergence on
   combatant `[14]` (`h4_turndiff`): the capture deals more (hp7 vs our hp11), the
   ×3 backstab multiplier; flanking alone stalled exactly there.
6. **TRAVIS ammo re-fit after backstab: still 10** — 9 → diverge @1575, 11 →
   @1910, 10 → CLOSED 2749/2749. The facing subsystem did **not** move his shot
   count; the §35 fit-may-move caveat is discharged. The loadout comment records
   the re-fit.

**Deferred / tripwired (unchanged territory):** the size>1 (`PlayerOnScreen`
all-cells) path, the ranged-melee clone-drop at depletion (`self-weapon-depleted`),
the 0-HD sweep, and the manual-UI facing sites (`sub_33B26`/`ovr009:608`/
`ovr014:1833`, out of §36 scope) all remain tripwired, none exercised by the eight
captures. The caster peel (§33 toggle-window) and the affects substrate are the
next slices; `caster-bar` @453 and `combat+terrain4` @368 are their frontiers.

With this slice **every H4 bar-fight capture that does not need magic or a ranged
size>1 loadout is CLOSED**: combat4, combat3, combat2, bar-rout, bar-fists-2, and
armed-bar — six operand-exact closes; the two open frontiers are the caster (@453)
and the terrain/wilderness driver (@368).

### 37.1 PR #7 review fixes — three landed, two cleared from the listing (2026-07-22)

Six review findings; **all five site questions were re-read in `coab_new.lst` before
acting**, and two of them turned out to be non-issues — recorded here with their
citations so they are not re-raised.

**Landed (four commits, one mechanic each; guard 8/8 verified at every one):**

1. **`HALF_DIR_TO_ISO` index bound.** `combat_setup` indexed the 4-entry table with
   `map_direction / 2` unmasked, so any heading ≥ 8 panicked (`index out of bounds: the
   len is 4 but the index is 4`) — reproduced with `RESTRIKE_MAP_DIR=8`. Since
   `map_direction` is a `pub u8` fed by the capture field or by the §29/§30 heading-sweep
   knob (which parses any `u8`), the trial knob that *found* md=2 had a landmine one step
   past the valid range. `% 4` added as a guard (the binary's `unk_1660C[md/2]` is an
   unbounded table read; md is always half-encoded {0,2,4,6}), matching the idiom at the
   other three `HALF_DIR_TO_ISO` sites.
2. **§36.3 site 7's departure-attack focus write.** `sub_3E954` sets `byte_1D90F = 1` and
   `byte_1D910 = 1` at `ovr014:0AE0-0AE5` — at the top of **each candidate iteration**,
   after the loop re-tests the MOVER's `in_combat` (@`0AD2-0ADD`) but **before** the
   candidate is fetched (@`0AF5-0B0B`) and before every per-candidate filter (`sub_66BDB`
   @`0B14`, `sub_3F143` @`0B2D`, the two `find_affect`s). A skipped candidate still leaves
   focus on, so it is not foldable into the `continue`. Without it an off-screen monster
   mover kept `focus == false` through its step, so `sub_3E748`'s post-write `(8, 3,
   new_pos)` scroll and `draw_74B3F`'s recenter were skipped where the binary's fire.
3. **§36.3 site 5's Sling/StaffSling second missile.** After the item-gated `sub_40BF1`
   (@`ovr014:1B11`), `sub_3F9DB` tests the readied primary's type at `:1B1C` (0x2F Sling)
   and `:1B2B` (0x65 StaffSling) and on either fires `sub_40BF1` **again** with the primary
   itself as the missile (`:1B32-1B4C`). That branch is the whole point for a sling:
   `GetCurrentAttackItem` hands flags `0x0A` a found-but-NULL item (§34.2), so the
   item-gated call never fires for one — a sling scrolled no camera at all. The old comment
   defended the omission with "the sling missile draw is itself draw-free"; draw-freeness is
   precisely *why* the camera was ported, since `mapScreenTopLeft` is what §36.1's on-screen
   branch reads. (The binary dereferences `field_151` with no null check here — UB for bare
   hands; we gate on the primary actually being readied.)
4. **`draw_missile_camera` branch tests** — the one camera site with nontrivial arithmetic
   (the `var_CE`/`var_D0` target anchor against `ScreenMapCheck`'s [3,46] clamp) had only
   its step-counting helper tested. Four tests now pin short-path early return, both-on-screen
   no-op, midpoint force-scroll, and long-span target anchoring incl. the map edge.

**Cleared — verified NOT defects (do not re-raise):**

5. **The post-attack `draw_74B3F` pair (`ovr014:1CAB-1CE7`) is correctly omitted.** §36.3
   site 4 lists it, and it is genuinely absent from our port — but it is gated on
   `sub_74761(0, attacker)` (@`1CA2`, `jz func_end`), and `sub_74761` is `PlayerOnScreen`
   (`ovr033:0761`: `size == 0 → false`, then the per-cell window scan). `draw_74B3F`'s only
   two persistent effects are the **off-screen** focus-gated recenter — unreachable under an
   on-screen gate — and the direction store, which passes the attacker's *current*
   `actions.field_9` (@`1CB9`/`1CD7`) and is therefore a no-op. Net camera and facing effect:
   nothing. Omitting it is exact, not an oversight.
6. **`CanBackStabTarget` has no team gate.** §36.4 specified "a thief-classed **party
   member**" and flagged the head for re-verification. The head (`ovr014:28DD-293C`) is
   purely: read `field_151`/`field_2E`; `ClassLevel[6] > 0` (@`291C`, `jg`) **or**
   (`ClassLevelsOld[6] > 0` (@`2927`, `jle` fail) **and** `sub_6B3D1` non-zero (@`293C`)).
   No `combat_team` compare anywhere in `sub_408D7`. Our `thief_skill_level > 0` is
   equivalent for every realistic level (the binary's compares are signed, so a ≥ 0x80 class
   level would differ — unreachable). The weapon test (@`293E-2962`) is confirmed as
   `{null, 0x61, 7, 8}` plus the range `0x23..0x25` ⇒ exactly `{null, 97, 7, 8, 35, 36, 37}`,
   and the facing test (@`298B-29A3`) pushes `arg_4` (attacker) first and `arg_0` (target)
   second ⇒ `target_direction(attacker.pos, target.pos) == target.direction`, matching the
   port. §36.4's "party member" was a spec overstatement; the code is right.

**Observed while verifying, NOT landed (for the next slice).** The departure-attack loop in
`sub_3E954` re-tests the **mover's** `in_combat` at the top of every candidate iteration
(@`0AD2-0ADD`, falling through to `loc_3ECEF` = loop exit); our `move_step_away_attack`
tests it once before the loop and then only tests each candidate's. So if a departure swing
kills the mover, the binary stops and we keep swinging at a dead mover. Pre-existing (it
predates §36), unexercised by the eight captures, and out of this slice's scope — flagged
rather than changed, since it alters attack counts and belongs in its own canary-checked
commit.

### 37.2 The deferred mover re-test, LANDED (post-merge, 2026-07-22)

The §37.1 deferred item, in its own canary-checked commit on main. One correction to the
§37.1 reading from the landing's own listing pass: the dead-mover jump at `ovr014:0ADD`
targets `loc_3ECEF`, which is the candidate loop's **continuation** (`var_18` vs `var_24`
compare → next iteration or `func_end`), not a hard exit — the binary skips the swing AND
the focus set (`@0AE0`, downstream of the test) and keeps scanning, with every remaining
iteration skipping identically since nothing revives the mover mid-loop. A `break` at our
loop top is therefore draw- and state-equivalent, and is what landed: the re-test sits
before the `focus = true` write, in the binary's order. Canary: all eight pins exact
(guard 8/8) — the six closed captures never drop a mover mid-departure-swarm, exactly as
§37.1 predicted.

## 38. RULING — §33's toggle window: a turn-ordinal toggle schedule (caster-bar pinned at ordinal 16) (2026-07-22)

**The question (§33, blocking all spell draws):** caster-bar's "Magic On" ('2' →
`AutoPCsCastMagic`/`byte_1D904`) was pressed BETWEEN PHILIPPE's round-1 and round-2 turns,
but the harness modeled the flag on-from-entry. Draw-equivalent while the flag armed only
the `memorized-spells` wire; the moment `sub_3560B`'s selection draws land, on-from-entry
draws 3× `roll_dice(1)` at his round-1 turn and moves the frontier from @453 to ~@83.
Options were (a) model the flip window as a per-capture input, or (b) extend the staging
hook to emit toggle events and restage.

**Ruling: (a) — a toggle schedule keyed by global turn ordinal, pinned per capture.**
(b) stays the long-term fix for FUTURE captures (hook TODO unchanged: an emitted toggle
event with its position would make the pin derived rather than fitted), but caster-bar is
already localized (§33's memorized-list decode, the md/58C fields, the 453 frontier) and
the flip point is recoverable from the capture itself to within a provably-equivalent
window — restaging would buy nothing this capture can't already prove.

**The listing grounds the model.** The mid-combat toggle is `sub_36269`
(`ovr010:1269-12DA`): `KEYPRESSED` → '2' → flip `byte_1D904` + print "Magic On"/"Magic
Off" (`@129C-12A9`; camp's own handler is `ovr009:0605-0647`). It is called from the AI
turn body — `sub_3504B+D` (the turn's head, BEFORE `sub_3560B`'s gate read
`@ovr010:068D`) and again later (`+19E`, …) — so the flag flips only at in-turn keyboard
polls, and the only draw-affecting readers are the per-turn spell gates (`ovr010:0679-06A7`).
A press is therefore observable ONLY through which gate checks see the flag on: any flip
instant between the same two bracketing gate checks is draw-identical, and a head-of-turn
flip can represent every equivalence class. The faithful input model is a schedule of
'2' presses keyed by **global turn ordinal** (0-based count of turns started = `Pick`
events) — general enough for multi-caster fights and mid-round presses, where a
round-boundary flip would not be (two casters' gate checks can bracket a press inside one
round).

**Landed:** `CombatState.auto_cast_toggles: Vec<u32>` (input-only; each listed ordinal
flips `auto_pcs_cast_magic` at that turn's head, the `sub_3504B+D` poll site);
`RESTRIKE_AUTO_CAST_TOGGLES=<n,...>` in `h4_replay`/`h4_turndiff`'s shared knobs; the
guard manifest gains `auto_cast_toggles` per pin. Unit tests pin the schedule mechanics
and that a flip is visible to the flipped turn's OWN gate (poll precedes gate).

**The caster-bar pin: entry `false`, toggles `[16]`.** Empirics (h4_replay trips, the
wire being the flag's only observable today): PHILIPPE's round-1 turn is ordinal 2 (pick
pass 2, round 1; gate check at draw 83 saw OFF — capture matches through his whole turn),
his round-2 turn is ordinal 16 (pick pass 0, round 2 — all 16 combatants act each round;
gate at draw 453 saw ON — the capture's first selection draws). Verified live: toggles
`[16]` and `[3]` produce identical trip sets (453, 987, 1366, … — exactly the turns where
the capture's unmodeled draws live) and the frontier stays @453; toggles `[2]` restores
the round-1 trip @83 (the overdraw a from-entry flag would commit once selection draws
land). The window is ordinals **[3, 16]**; 16 is pinned as the canonical representative —
the head of the turn whose gate PROVES the flag on, i.e. the only boundary the capture
itself names. The true press instant within the window is unknowable and irrelevant: all
representations in the class are draw-identical at every reachable observation point.

Guard after the pin edit: 8/8 exact, caster-bar frontier @453 unchanged.

## 39. SPEC — the affects substrate (M5 Phase 2 opener; Fable-scoped, implementer-built) (2026-07-23)

**Goal: the affect state machine + faithful check-site wiring, landed DRAW-NEUTRAL — all
eight guard pins EXACT at every commit, zero manifest edits.** Every combat path that
today says "draw-free, no affects modeled" becomes a real dispatch over (empty) affect
state, and a future capture that meets a LIVE affect names itself through a tripwire
instead of silently diverging. This is the platform every spell cast lands on (§25's
Phase-2 order: affects FIRST, then spells); it must not move a single draw today.

### 39.1 The data model (binary-verified)

One affect = 9 on-disk bytes (`Affect.StructSize` = 9, `Classes/Affect.cs:164`;
`affect_struct_size = 9` in the listing). Layout, confirmed THREE ways — coab's
`DataOffset` attributes (`Affect.cs:188-195`), `add_affect`'s field stores
(`ovr024:13F0-14A4`), and real `.FX` file dumps (`~/goldbox-data/cotab/SAVE/*.FX`):

| off | size | field | notes |
|---|---|---|---|
| 0x00 | 1 | `kind` | the `Affects` enum id (0x00-0x93) |
| 0x01 | 2 | `minutes` | game-time minutes; **0 = permanent/until-removed** |
| 0x03 | 1 | `data` | per-kind payload (e.g. bless amount) |
| 0x04 | 1 | `call_affect_table` | bool: fire the effect-handler jump table on add/remove |
| 0x05 | 4 | *(next far ptr)* | **heap linkage, NOT state** — stale in real `.FX` dumps (live seg:off values, NULL tail); coab zero-fills on write. Decode MUST ignore. |

Rust: `gbx_formats::affects::AffectRecord { kind: u8, minutes: u16, data: u8,
call_affect_table: bool }` + `decode(&[u8]) -> Option<AffectRecord>` (None on short
input; bytes 5-8 skipped). `save_orig::read_affects` (the opaque 9-byte splitter) stays;
the typed decode layers on top. Storage on the combatant:
`Combatant.affects: Vec<AffectRecord>` — **list order is load-bearing**: `add_affect`
appends at the TAIL (walks `next` to the end, `ovr024:13F0-14A4`), `FindAffect` returns
the FIRST match (`ovr025.cs:1175-1180`, binary `@find_affect` `ovr025:2345`), and
`remove_affect` removes ONE instance (the found one), not all.

### 39.2 The core API (all PRNG-free — verified)

The ONLY `@Random` consumer in ovr024 is `roll_dice` itself (`ovr024:13AC`); the check
dispatch, find, add, remove, and the expiry walk make **zero draws**. This is the
substrate's whole draw-neutrality argument, and it is why the six closed captures could
close with affects unmodeled.

- `find_affect(actor, kind) -> Option<&AffectRecord>` — first match in list order.
- `add_affect(actor, kind, minutes, data, call_table)` — append tail
  (`ovr024:13F0-14A4`). The `call_table=true` add-side handler (`CallAffectTable(Add)`,
  `ovr013`) is NOT modeled — no current caller adds affects yet; the spell slice will.
- `remove_affect(actor, kind)` — remove the first matching instance
  (`ovr024:010A-027A`, an UNHEADERED label — no proc header in the listing, reached via
  the `stub024` thunk). Side effects cited, tripwired, not modeled: the
  `CallAffectTable(Remove)` call when the removed record carries `call_affect_table`,
  and the `CalcStatBonuses` recompute for `resist_fire` (CHA) /
  `enlarge|strength|strength_spell` (STR).
- `remove_combat_affects(actor)` — the fixed strip table (`sub_645AB` @`ovr024:15AB`,
  coab `ovr024.cs:661-691`): faerie_fire, charm_person, reduce, silence_15_radius,
  spiritual_hammer, stinking_cloud, helpless, animate_dead, snake_charm, paralyze,
  sleep, clear_movement, regenerate, affect_5F, regen_3_hp, entangle, affect_89,
  affect_8b, owlbear_hug_round_attack — **transcribe the ids from the listing, not the
  names** — then the berserk quirk (`HasAffect(berserk) && control_morale == PC_Berzerk
  → combat_team = Ours`), tripwired. Companion `remove_attackers_affects`
  (`sub_6460D` @`ovr024:160D`): reduce, clear_movement, affect_8b,
  owlbear_hug_round_attack.
- `check_affects_effect(actor, CheckType)` — the 24-case dispatch (`work_on_00`
  @`ovr024:0414-0D02`): for each id in the case's ORDERED list →
  `calc_affect_effect(id, actor)`.
- `calc_affect_effect(id, actor)` (`ovr024:027A-0411`, coab `:99-136`): find on the
  actor; if absent AND id ∈ the radius set {prot_from_evil_10_radius 0x2D,
  prot_from_good_10_radius 0x2E, prayer 0x31, silence_15_radius 0x15} → scan the
  team lists for a CARRIER (any combatant holding id); a carrier found in combat gates
  on range (≤6 for prayer, ≤1 else, via the near-list builder) — model the scan, and
  TRIP on carrier-found (the range gate + handler are the spell slice's). If found on
  the actor → **tripwire** (below); the real effect handler (`CallAffectTable(Add)`)
  lands with the spells.

**The dispatch table.** Binary verified case-by-case against coab this session: id-for-id
and order-for-order IDENTICAL, all 24 cases (the listing's raw `affect_XX` names are the
same ids under coab's meaningful names — affect_0d=reduce, affect_88=entangle,
affect_03=sticks_to_snakes, affect_2f=dwarf_and_gnome_vs_giants, affect_3a=
clear_movement, affect_38=item_invisibility, affect_62=regen_3_hp, affect_61=
con_saving_bonus, affect_64=troll_fire_or_acid, affect_4b/4c=weap_dragon_slayer/
frost_brand — full enum in `Classes/Affect.cs:5-157`). Transcribe from coab
`ovr024.cs:140-375` verbatim (CheckType enum `ovr024.cs:6-32`); keep each case's list
ORDER (find-first semantics make order observable once handlers land).

### 39.3 What combat does NOT do: tick durations

`CheckAffectsTimingOut` (`sub_5801E` @`ovr021:001E`, coab `ovr021.cs:11-107`) decrements
`minutes` ONLY while Camping (converting rest time through `timeScales`); outside camp it
just flags `affects_timed_out` and returns. **Combat never expires an affect by time.**
The substrate therefore stores `minutes` verbatim and has NO tick machinery; per-round
re-evaluation happens through the `Type_19` dispatch at `BattleRoundChecks`, and combat
removal happens only via `remove_combat_affects`/`remove_affect` at the cited sites.
Camp/rest expiry lands with the camp systems, not here.

### 39.4 The tripwire strategy

One wire: `ActionEvent::StubTripped { combatant_id, stub: "affect-effect" }`, emitted by
`calc_affect_effect` when a matching affect is FOUND (= the point where the binary would
run a `CallAffectTable` handler we don't model). Do NOT extend the event shape (the
harness prints pattern-match it); the kind is recoverable by inspection. Secondary wires:
`"affect-remove-side"` in `remove_affect` when an ACTUAL removal carries
`call_affect_table` or a stat-recompute kind, and `"affect-berserk"` for the
`remove_combat_affects` berserk quirk. With empty lists — the state of every current
capture — no wire can fire; that plus the PRNG-free dispatch IS the draw-neutrality
proof, and the guard 8/8 run at every commit is its check.

### 39.5 The wiring census (comment-no-op → real dispatch)

Wire `check_affects_effect` at the already-modeled sites that today carry a "no affects"
comment — the exact CheckType per site, coab cite first, our anchor second:

1. Turn head: `PlayerRestrained` (`ovr009.cs:108`), `Type_15` (`:125`), `Confusion`
   (`:129`, gated `spell_id == 0`) — at `melee_ai_turn`'s head. The restrained/held TURN
   BEHAVIOR stays unmodeled (a found affect trips via the dispatch itself).
2. Round end: `Type_19` per member (`ovr009.cs:371`) — `battle_round_checks`, which
   already cites it as gated-out (combat.rs `battle_round_checks` doc).
3. FleeCheck: `Morale` twice (`ovr010.cs:780/788`) — our `flee_check` documents both.
4. Movement: `Movement` (`ovr014.cs:23/76/488`) — the calc-moves paths (our :76 anchor).
5. AI specials: `Type_14` (`ovr010.cs:516`) — our anchor cites `ovr010:0DDB`.
6. To-hit: `Type_10` on attacker + `Type_16` on target (`ovr024.cs:529-530`,
   PC_CanHitTarget); `Type_5` on target (`ovr014.cs:101`); `Type_11` on target
   (`ovr014.cs:774`); `SpecialAttacks` on attacker (`ovr014.cs:100`).
7. Visibility: `Visibility` on targetA + `None` on seer (`ovr014.cs:583/:591`) — our
   `CanSeeTargetA` anchor (the `None` case is a no-op by the dispatch's own case 0).
8. Saves: `SavingThrow` (`ovr024.cs:577`) — inside our save path.
9. Damage/death: `PreDamage` (`ovr024.cs:1186`), `FireShield` (`:1201`), `Death`
   (`:1272`; also `ovr014.cs:210` and KillPlayer `:49`) — our `apply_damage` ladder.
10. Removal paths: `remove_combat_affects` at our KillPlayer/RemoveFromCombat
    equivalents (`ovr024.cs:48/:645`, the sub_644A7 path we model);
    `remove_attackers_affects` at the flee path (`ovr010.cs:765`, our :3237 anchor);
    `remove_invisibility` (`ovr014.cs:752`, our :1248 anchor — loop
    `find_affect(invisibility)` + `remove_affect`).

Skip sites living in unmodeled subsystems (in_poison_cloud, spell resolution
`MagicResistance`, manual-UI): they arrive with their own slices. Every wired site gets
the binary citation in place of the old comment. **The implementer verifies each wired
site's CheckType argument against the listing call site** (the pattern: push player, push
type byte, call `work_on_00`), not just coab.

### 39.6 Entry population (why replays stay empty)

The record image's `affect_ptr` @0xF2 (`coab_new.lst` charStruct, occupies 0xF2-0xF5) is
HEAP LINKAGE — a capture's record bytes cannot carry the list, so capture replays build
every combatant with `affects: vec![]`, which is bit-for-bit today's behavior. That is
faithful for all eight pins (proven by their closure/frontier stability), NOT a general
truth: a future buffed-party or innate-affect capture needs the staging hook to walk the
`0xF2` chain and emit per-combatant affect records at `combat_entry` (**hook TODO #2**,
beside §38's toggle events; 9 bytes each, the `next` field ignored). Real-play population
sources, cited for their own slices: `.FX` decode at save import (`ovr017.cs:558-579`
fixed 9-byte blocks; our `SaveSet.chars[].affects` already carries the raw records) and
`MON<area>SPC.dax` innate affects cloned per spawn (`ovr003.cs:275-286` — per-copy
ShallowClone, our `gbx_formats::monster` already splits the records). This slice lands
the `gbx-formats` typed decode + tests; the import/spawn plumbing is NOT wired here.

### 39.7 Build order (canary discipline, §36.5 pattern)

1. `gbx-formats`: `AffectRecord` + decode + tests (incl. a real-layout fixture with a
   junk `next` field, synthetic bytes only — D10: no real save bytes in the repo).
2. Engine: storage + the API of §39.2 + unit tests (order semantics: add-tail,
   find-first, remove-one; the strip tables; dispatch → tripwire on a synthetic affect).
3. Wire the census sites in small groups, **guard 8/8 after every commit** — the pins
   must hold EXACTLY (no manifest edits; any shift = a bug in the slice, stop and report).
4. Doc landing note (§40) + memory: which sites wired, which cited-skipped.

One mechanic per commit, binary-cited; never weaken an assert; full gates (fmt, clippy,
`cargo test --workspace`, guard) per commit. Worktree, no push — Bryan reviews the PR
after Fable's audit.

## 40. LANDED — the affect substrate, draw-neutral; guard 8/8 exact at every commit (M5 Phase 2 opener, 2026-07-23)

The §39 spec was implemented on branch `m5-caster-prep` (folded from the implementer worktree) off the §39 spec commit
(`8d79537`, which itself carries §38's toggle-window code). **Every wired site's
`CheckType` was re-verified at the LISTING call site** (the `push player; mov al
<type>; push ax; call work_on_00` pattern) via
`LC_ALL=C grep -an "work_on_00" coab_new.lst` and a per-call-site scan of the
preceding `mov al,<type>` — the map is transcribed below. The substrate is
draw-neutral by construction (PRNG-free dispatch over the empty affect lists every
capture carries), and it held: **all eight guard pins EXACT at every one of the five
commits, zero manifest edits, zero stub trips on every closed capture.**

**Five commits (each fully gated):**

- `b33b154` §39.1 — `gbx-formats::affects::AffectRecord {kind, minutes, data,
  call_affect_table}` + `decode` (bytes 0x00-0x04, the heap `next` @0x05-0x08
  ignored). Synthetic-only tests (D10), incl. a junk-`next` fixture proving 0x05-0x08
  are discarded. **7 formats tests** (125 → 132).
- `d278d50` §39.2 — `Combatant.affects: Vec<AffectRecord>` (empty at both literal
  constructors; `combatant_from_record` inherits it via `new_melee`) + the PRNG-free
  API (`find_affect`/`has_affect`/`add_affect` on `Combatant`;
  `check_affects_effect`/`calc_affect_effect`/`remove_affect`/`remove_combat_affects`/
  `remove_attackers_affects`/`remove_invisibility` on `CombatState`) + the `CheckType`
  enum and the 24-case dispatch. **10 engine tests** (365 → 375).
- `ba92f3c` §39.5 (1/3) — turn/round/movement/AI-special check sites.
- `75fc757` §39.5 (2/3) — the to-hit / attack-path check sites.
- `2622ff8` §39.5 (3/3) — the flee / death / removal sites.

### 40.1 The dispatch table + the strip tables (LISTING-cited)

The 24-case `work_on_00` dispatch (`ovr024:0414-0D02`) was transcribed verbatim from
coab `ovr024.cs:140-375`, ids from `Classes/Affect.cs`, each case's ORDER preserved
(find-first makes order observable once handlers land). A unit test pins the 24 case
lengths `[0,4,7,7,6,16,21,7,5,12,10,8,16,3,11,5,7,3,3,5,2,1,1,1]` against an
accidental edit.

The two strip tables were transcribed **from the LISTING data** (not just coab) and
match coab id-for-id:

- `RemoveCombatAffects` (`sub_645AB` @`ovr024:15AB`): `unk_16D41[1..19]`
  @`seg600:0A32-0A44` = `07 0B 0D 15 17 1E 1F 20 33 34 35 3A 3B 5F 62 88 89 8B 90`
  (19 entries). The loop reads indices 1..19 (`mov al, unk_16D41[di]`, `cmp
  loop_var,13h; jnz`), index 0 (`0FFh`) unused. Then the berserk quirk (`@15DC-1601`):
  `find_affect(berserk 0x4D)` + `field_F7 == 0B3h` (`PC_Berzerk`) → `combat_team = 0`
  (Ours) — modeled as the `"affect-berserk"` tripwire, not a team flip.
- `RemoveAttackersAffects` (`sub_6460D` @`ovr024:160D`): `[0xA46..0xA49]` @`seg600`
  = `0D 3A 8B 90` (reduce, clear_movement, affect_8b, owlbear_hug_round_attack; loop
  reads `[di+0A45h]` for di 1..4, `cmp var_1,4; jnz`).

The radius-carrier set for `calc_affect_effect` is the `unk_6325A` **bitmask**
@`ovr024:025A` (`@Set@MemberOf` bit-test, `ptr[byte/8] & (byte&7)`), which decodes to
`{silence_15_radius 0x15, prot_from_evil_10_radius 0x2D, prot_from_good_10_radius
0x2E, prayer 0x31}` — verified bit-by-bit (0x15→byte2 bit5, 0x2D→byte5 bit5,
0x2E→byte5 bit6, 0x31→byte6 bit1). The carrier range gate (`prayer ? 6 : 1`,
`cmp affect_type,31h @031C`) is the spell slice's; the substrate models the scan and
trips on a carrier found.

### 40.2 The census sites wired (each `CheckType` verified at its listing call site)

| § | site (our anchor) | CheckType | listing call | coab |
|---|---|---|---|---|
| 1 | turn head (`TurnDriver::MeleeAi`) | PlayerRestrained 7 | `ovr009:02B7` (`mov al,7`) | ovr009.cs:108 |
| 1 | turn head, in `delay>0` | Type_15 0x0F | `ovr009:0352` | :125 |
| 1 | turn head, in `delay>0` | Confusion 0x15 | `ovr009:036E` | :129 (spell_id==0, trivially true) |
| 2 | `battle_round_checks` per member | Type_19 0x13 | `ovr009:09EF` | :371 |
| 5 | `sub_35db1` head | Type_14 0x0E | `ovr010:0DDB` | :516 |
| 4 | `reclac_attacks` | Movement 0x12 | `ovr014:0E66` | :488 |
| 4 | `calculate_initiative` | Movement 0x12 | `ovr014:005E` | :23 |
| 6 | `attack_target`, pre-AC-select | Type_11 0x0B | `ovr014:167E` | :774 |
| 6 | `attack_target` swing, roll>1 | Type_10 0x0A | `ovr024:1283` | PC_CanHitTarget :529 |
| 6 | `attack_target` swing, roll>1 | Type_16 0x10 | `ovr024:1290` | PC_CanHitTarget :530 |
| 6 | `attack_target` on-hit, post-`roll_damage` | SpecialAttacks 4 | `ovr014:023A` | sub_3E192 :100 |
| 6 | `attack_target` on-hit, post-`roll_damage` | Type_5 5 | `ovr014:0248` | sub_3E192 :101 |
| 3 | `flee_check`, after seed/clamp | Morale 0x11 | `ovr010:1414` | :780 |
| 3 | `flee_check`, after enemyHealth% | Morale 0x11 | `ovr010:1467` | :788 |
| 9 | `apply_damage` death tail | Death 0x0D | `ovr014:0630` | DisplayAttackMessage :210 |

Removal / list ops wired: `remove_invisibility(attacker)` per swing at the
PC_CanHitTarget head (coab `ovr024.cs:519`, our `attack_target`);
`remove_attackers_affects` at `flee_check`'s head (`sub_6460D`, coab :765);
`remove_affect(0x4A)`/`remove_affect(0x4B)` in `flee_check`'s flee fork
(`ovr010:14DC/14F0`); `remove_combat_affects` in `apply_damage`'s death tail
(`sub_645AB` call @`ovr014:0622`) and in `remove_from_combat` (`sub_644A7`, coab :645).

### 40.3 Census sites SKIPPED, with reasons (mechanic not modeled / no emit seam)

- **`CalcMoves`'s Movement check (`ovr014:0179`, coab :76).** Our `calc_moves` is a
  **pure free function** with no `&mut self`/emit seam, called from many read-only
  sites. The other two Movement checks (CalcInit, reclac_attacks) ARE wired at their
  self-bearing anchors, and both call `calc_moves` right after their own Movement pass,
  so the effect (an empty-list no-op) is covered in the modeled flow. Threading emit
  through the widely-used pure helper buys nothing draw-visible.
- **`CanSeeTargetA`'s Visibility + None checks (`ovr014:117C`/`11BD`, coab :583/:591).**
  Our `can_see_target` is a `&self` predicate (`return in_combat`) **inlined into
  self-borrowing read-only loops** (near-target building, mover/target filters at 5
  call sites). Hosting `CheckAffectsEffect(Visibility)` there needs `&mut self` to emit
  and would fire per scan-iteration, not per logical visibility check. The underlying
  invisibility RESOLUTION is explicitly unmodeled (the code comment: "no affects → a
  live target is always seen"); the `None` case is a dispatch no-op anyway (case 0).
  Belongs with the invisibility slice. Citation preserved at the anchor.
- **`RollSavingThrow`'s SavingThrow check (`ovr024:134F`, coab :577).** Our
  `roll_saving_throw` is a **pure free fn with no live combat caller** — its only
  callers are tests; nothing in a modeled combat flow rolls a save (no spell/effect
  resolution yet). No emit seam, no reachable site; lands with the effect slice.
- **`damage_person`'s PreDamage + FireShield (`ovr024:1FD1`/`2002`, coab :1186/:1201).**
  These live in `damage_person` — the **spell/effect-damage entry** — which our engine
  does not model. The weapon path is `DisplayAttackMessage → damage_player` (our
  `apply_damage`), which never enters `damage_person` (confirmed: coab `damage_player`,
  ovr025.cs:1184-1245, contains NO affect checks; they are all in the callers). Wiring
  them into the weapon path would run affect checks the binary does not run on a weapon
  hit. Only the Death + RemoveCombatAffects tail, which IS in the weapon caller
  (DisplayAttackMessage :209-210), was wired. Lands with the spell/effect-damage slice.
- **`remove_invisibility` at the held-slay auto-hit (`ovr014:0752`, coab :752).** In the
  `|| target.IsHeld()` auto-hit / held-slay branch, which is affect-gated (held IS an
  affect state) and **not modeled** (our `resolve_attack` doc already flags the held
  auto-hit as unmodeled). The modeled `remove_invisibility` is the PC_CanHitTarget one
  (:519), which IS wired.
- **`CanHitTarget`'s Type_16 (`ovr024:1211`, coab :493, `sub_641DD`).** A DIFFERENT
  function from PC_CanHitTarget — its only caller is `CMD_Damage` (the ECL `DAMAGE`
  opcode, a scripted/area effect), not the weapon path. Scripted effects are their own
  slice.
- **The on-hit `(CheckType)attackIdx+1` check (`ovr014:1839`, dynamic Type_2/Type_3).**
  The poison/special-attack-on-hit check inside AttackTarget01's hit branch — **not in
  the §39.5 census** (the census lists Type_5/SpecialAttacks/Type_10/16/11 for the
  attack path, not Type_2/3). Left unwired per the census.
- **`ovr011:1D7C`/`1D8A` (Type_8/Type_22), `ovr013:26B2` (Death, CallAffectTable),
  `ovr023:*` (Type_11/MagicResistance ×2), `ovr024:230A` (MagicResistance),
  `ovr024:22A8`/`008A` (Death in damage_person/KillPlayer).** Unmodeled subsystems
  (BattleSetup, the affect-effect jump table itself, spell resolution) — the spec's
  own "skip sites in unmodeled subsystems" clause. `KillPlayer` (`ovr024.cs:36`) is a
  distinct death entry we don't model as its own function; the weapon-death tail it
  shares (RemoveCombatAffects + Death) IS wired in `apply_damage`.

### 40.4 coab≠binary found (LISTING evidence)

**`remove_affect`'s CHA stat-recompute fires on `friends` (0x0E), not `resist_fire`
(0x14).** coab (`ovr024.cs:83`) reads `if (affect_id == Affects.resist_fire)
CalcStatBonuses(Stat.CHA, ...)`. The LISTING compares `[bp+0Ah] (affect_id), 0Eh`
(`ovr024:0222`) → `sub_648D9(al=5)` (CHA). `0x0E` is `friends`, not `resist_fire`
(`0x14`) — and semantically the binary is right: the AD&D **Friends** spell buffs
Charisma; `resist_fire → CHA` is nonsense. The STR set matches coab exactly:
`{enlarge 0x0C, strength 0x26, strength_spell 0x92}` → `sub_648D9(al=0)`
(`@0235-0245`). The substrate uses the binary set `{0x0E, 0x0C, 0x26, 0x92}` for the
`"affect-remove-side"` tripwire (draw-neutral either way — no capture removes an
affect from a non-empty list).

### 40.5 Result

`cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets` no warnings
(the sole remaining `#[allow(dead_code)]` is on `CheckType`, whose full 24-value set is
transcribed for dispatch fidelity though only the wired subset is constructed;
`add_affect` is `pub`, uncalled until the spell slice supplies the first affect-adding
caller). `cargo test --workspace` green (**132 formats, 375 engine**, all other crates
unchanged). Guard **8/8 EXACT** at every commit:

```
OK  combat4.gbxtrace — CLOSED (operand-exact, 0 trips)
OK  combat3+terrain4.gbxtrace — CLOSED (operand-exact, 0 trips)
OK  combat2+terrain4.gbxtrace — CLOSED (operand-exact, 0 trips)
OK  combat+terrain4.gbxtrace — frontier @368 (exact)
OK  bar-rout-58c50.gbxtrace — CLOSED (operand-exact, 0 trips)
OK  armed-bar.gbxtrace — CLOSED (operand-exact, 0 trips)
OK  caster-bar.gbxtrace — frontier @453 (exact)
OK  bar-fists-2.gbxtrace — CLOSED (operand-exact, 0 trips)
frontier guard: 8/8 pins held
```

Draw-neutrality proof discharged: the flee path (`bar-rout` CLOSED) runs
RemoveAttackersAffects + Morale ×2 + RemoveCombatAffects with zero trips; the
downed-PC death path (`combat`/`combat2` diverge for other reasons) runs
RemoveCombatAffects + Death unshifted; every closed capture stays 0 trips. The
substrate is the platform the spell slice lands on (§25 Phase-2 order: affects first).
**Not wired here** (their own slices): affect population at entry (the `0xF2` chain
walk / `.FX` import / SPC innate clone — hook TODO #2), the `CallAffectTable` add/remove
handlers, tick-duration expiry (camp-only, §39.3), and the skipped census sites in
§40.3.

## 41. SPEC — the caster peel, part 1: faithful spell selection + the Magic Missile cast (M5; Fable-scoped) (2026-07-23)

**Goal: caster-bar's frontier moved past 453 by faithful draws only — plausibly to
CLOSURE 3517/3517.** The capture's whole spell story is now RE-complete and
draw-accounted: round 2 (@453) draws 3× d1 (one priority-7 pass, all picks rejected —
the d7 rolled 1); round 3 (@1016-1029) is THE CAST — ten d1s (3+3+3 rejections at
priorities 7/6/5, then the accepted pick at priority 4) + one d10 (the `find_target`
near-list pick) + three d4s (Magic Missile damage 3+3d4) — and every later PHILIPPE
turn is selection-silent (the slot was consumed) and pure modeled melee. Post-cast the
fight contains no other stubbed territory, so closure is the realistic target; bank at
a named draw if a residual surfaces.

### 41.1 The selection loop (`sub_3560B` @`ovr010:060B-0738`) — the draws

Already modeled: the collection loop (`record[0x1E+i]`, `i=1..0x53`, §33), the gates
(§33 + the §38 toggle schedule), the unconditional d7. To land: the pass loop.

- `priority = 7`, `bound = roll_dice(7,1)` (the existing d7 — its RESULT becomes
  load-bearing), `pass = 1`.
- While `pass <= bound` and nothing picked: up to **3×** `roll_dice(spells_count, 1)`
  (each result −1 indexes the collected candidate list); each pick →
  `ShouldCastSpellX(priority, id)`; an accept stops the inner loop AND the outer.
  Then `priority -= 1`, `pass += 1`. (coab `ovr010.cs:255-273`, asm verified
  `ovr010:06A9-070D`.)
- On accept: `spell_menu3` (§41.3). On no pick: fall through to the normal turn
  (items_selection → find_target → move-attack). **On a modeled cast the AI turn
  RETURNS immediately after `sub_3560B`** (coab `ovr010.cs:74-77`) — no
  items_selection, no melee targeting, no movement.

### 41.2 `ShouldCastSpellX` (`sub_353B1` @`ovr010:03B1-04A7`) — draw-free for MM

Verdict chain (in order):
1. **Priority gate**: `SpellData[id].priority >= minPriority` else reject. The table =
   `gbl.spellCastingTable` @`seg600:37DC`, 16-byte stride (`Classes/Gbl.cs:567+`,
   struct field map `Classes/Spells.cs:153-204` — priority @+0xD, field_E @+0xE,
   field_F @+0xF, fixedRange @+0x2, perLvlRange @+0x3, field_6 @+0x6, damageOnSave
   @+0x8, affect_id @+0xA, whenCast @+0xB, castingDelay @+0xC).
2. `id == 3` special (`find_healing_target`) — cite, tripwire (no capture).
3. `field_E == 0` → **accept** (self/buff spells need no target scan).
4. Else `near_enermy(SpellRange(id), caster)` — `BuildNearTargets`
   (`ovr025.cs:1290`) = `Rebuild_SortedCombatantList(caster, range, enemy-team
   filter)` = OUR near-list flood; count == 0 → reject.
5. `field_F == 0` → **accept**. Else the `sub_352AF` per-target loop
   (`ovr010.cs:117-141`) — **DRAW-BEARING: `RollSavingThrow` per candidate** —
   tripwire this branch (`spell-ff-scan`); no pinned capture reaches it (MM ff=0).

`SpellRange` (`sub_5CDE5`, `ovr023.cs:515`): `fixedRange + perLvlRange × castingLvl`;
0-with-field_6 → 1; −1/0xFF → 1. `castingLvl = spellMaxTargetCount(id)`
(`sub_6886F`, `ovr025.cs:1342`): MagicUser class → `max(SkillLevel(MU),
SkillLevel(Ranger)−8)` (the §34 SkillLevel machinery); the no-caster fallback 6;
Monster 12; `spell_from_item` → 6 (cite only).

**Magic Missile row (id 0x0F)**: priority 4, field_E 1, field_F 0, fixedRange 6,
perLvlRange 4, field_6 4, targets Combat, damageOnSave Normal(=0), saveVerse Spell,
affect none, whenCast Combat, castingDelay 1. **Transcribe rows lazily**: MM now; any
OTHER id reaching ShouldCastSpellX → StubTripped `spell-entry` + reject (capture-safe:
pinned captures memorize only MM; a future capture names the next row to transcribe).

### 41.3 The cast (`spell_menu3` → `sub_5D2E1` → `SpellMagicMissile`)

- `spell_menu3` (`ovr014.cs:1373`): whenCast==Camp → "Camp Only" abort (cite);
  `delay = castingDelay/3` — MM: 0 → **immediate cast** `sub_5D2E1` + clear_actions.
  `delay > 0` spells queue `actions.spell_id` + delay clamp ("Begins Casting") —
  cite, tripwire (`spell-queued`).
- `sub_5D2E1` (`ovr023.cs:674-810`), combat path:
  1. Miscast: `HasAffect(affect_4a)` → d2, 1 = miscast — a §39 `find_affect` read;
     empty affects → no draw. Wire through the substrate.
  2. Targeting: `SpellCastFunction = ovr014.target` in combat (`ovr009.cs:25`).
     `ovr014.target` (`ovr014.cs:1164`): MM's `field_6 & 0xF = 4` → the tail branch,
     `max_targets = (4&3)+1 = 1` → one `sub_4001C` pick. Other field_6 shapes (0 self,
     5 budgeted multi w/ 2d4 draw, 8-0xE area, 0xF held/area) — cite, tripwire
     (`spell-target-shape`).
  3. `sub_4001C` (`ovr014.cs:1095`), QuickFight + field_E≠0: **`find_target(true, 0,
     SpellRange(id), caster)` — the d10** (our find_target, spell-mode args:
     clear_target=true, arg_2=0, max_range=range). Then the held filter: target
     `IsHeld()` && spell's affect_id ∈ `unk_18ADB[1..4]` (held-affect ids) → pick
     rejected (the var_9 loop runs ONCE → no cast this turn). MM affect none → never
     rejects. IsHeld = the §39 held-affects test (empty lists → false).
  4. On target success: the missile camera — `draw_missile_attack(0x1E, 4, targetPos,
     casterPos)` + the `draw_74B3F` attack-icon pair (PlayerOnScreen-gated) — the §36
     machinery, MagicAttackDisplay = §36.3 site 8. Draw-free.
  5. `remove_invisibility(caster)` — §39 API. Draw-free.
  6. **`spellList.ClearSpell(id)` — slot consumption**: clears ONE memorized slot
     (implementer verifies WHICH slot in `SpellList.ClearSpell` + the binary; the
     capture pins the observable: every post-cast PHILIPPE turn draws ZERO selection
     d1s — spells_count must hit 0).
  7. Dispatch `spellTable[0x0F] = SpellMagicMissile` (`ovr023.cs:1166`, `sub_5E221`):
     `n = spellMaxTargetCount + 1 = lvl+1`; `damage = n/2 + roll_dice_save(4, n/2)`;
     `roll_dice_save ≡ roll_dice` (`ovr024.cs:601` — sets `gbl.dice_count` only) →
     **(lvl+1)/2 separate d4 draws**. Capture: 3 d4s → PHILIPPE lvl 5-6 (the record's
     SkillLevel(MU) decides; verify the decode agrees).
  8. `DoSpellCastingWork` (`sub_5CF7F`): per target — `damageOnSave == Normal(0)` →
     `saved = false`, **NO save draw** (capture-confirmed: d100s follow the d4s);
     `fixedRange == -1` touch branch (Type_11 + PC_CanHitTarget) not MM — cite;
     `damage_person(false, Normal, damage, target)` → our existing apply_damage
     ladder (draw-free); `affect_id == 0` → no ApplyAttackSpellAffect.

### 41.4 Build order

1. The SpellEntry row type + MM's row (lazy-transcription rule) + unit tests.
2. Selection loop draws + ShouldCastSpellX (MM chain) — canary: guard 8/8 with
   caster-bar's pin EDITED IN THE SAME COMMIT once the frontier moves (the exact-pin
   rule). Expect 453 → 1016 region.
3. The cast: targeting d10 → damage d4s → consumption → early-return. Expect the
   frontier past 1029 — run to the next residual or closure; pin whatever is TRUE.
4. Tripwires: `spell-entry`, `spell-ff-scan`, `spell-queued`, `spell-target-shape`.
5. bar-fists-2 must stay CLOSED (two inert slots, magic off — the §33 proof), armed-bar
   and the four brawls untouched. Full gates per commit.

### Empirical anchors (this session's capture scan + record decode)

- Round-2 trip @453: 3× d1 (d7 rolled 1). Round-3 cast @1016-1029: 10× d1 + d10 +
  3× d4, then d100s (no save). Post-cast PHILIPPE turn heads draw d4/d7/d7 and no
  d1s. Other single-d1 runs in the stream are melee find_target re-picks (near-list
  size 1), not selection.
- PHILIPPE = combatant [5]: ClassLevels (rec 0x109+) = MU slot(5) = **5**,
  single-class, olds all 0 → SkillLevel(MU) = 5 → castingLvl 5, SpellRange
  6+4×5 = **26**, missiles (5+1)/2 = **3** ✓ the three d4s. Memorized:
  {0x71: 0x0F} only (§33). Combatant [3] is F4/MU4 with ZERO memorized slots —
  gate-1 inert, capture-consistent (no selection draws on its turns).
