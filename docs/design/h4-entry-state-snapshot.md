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
