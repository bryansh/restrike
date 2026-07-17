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
