# Roll Credits — the CotAB-completable milestone

> Door opened 2026-08-09 (post-M6 ratification + the playtest-fix arc). This is the working
> plan for PLAN.md's "M6 Roll credits" (shifted right by the working-ledger renumbering):
> **finish Curse of the Azure Bonds start-to-end in our engine.** Decisions here are locked
> unless a slice's evidence forces a revisit; per-slice research items are named as such.

## 0. Where we stand (the dashboard, census of 2026-08-09)

`restrike census` against the real data (v1.3): **6 ECL files, 25 blocks, 3,582 reached
instructions, 52 distinct opcodes in use, zero decode hazards** (no desyncs, no unknown
modes, no unresolved tails, no out-of-block targets — dockets 1–4 and 7 all clean).

- **Use-weighted opcode coverage: 98.9%** — 3,543 of 3,582 reached instruction uses land on
  implemented opcodes.
- **The unimplemented tail: 14 opcodes, 39 uses total**, frequency-ordered:

  | op | name | uses | cluster (see §2) |
  |---|---|---|---|
  | 0x0D | APPROACH | 8 | encounters |
  | 0x40 | DESTROY ITEMS | 5 | items |
  | 0x0A | LOAD CHARACTER | 4 | roster |
  | 0x27 | TREASURE | 4 | treasure/XP |
  | 0x31 | SPRITE OFF | 3 | encounters |
  | 0x36 | ADD NPC | 3 | roster |
  | 0x3D | CLEAR BOX | 3 | presentation |
  | 0x29 | ENCOUNTER MENU | 2 | encounters |
  | 0x32 | FIND ITEM | 2 | items |
  | 0x1D | PARTYSTRENGTH | 1 | encounters |
  | 0x2C | PARLAY | 1 | dialogue |
  | 0x2E | DAMAGE | 1 | mechanics |
  | 0x35 | SAVE TABLE | 1 | mechanics |
  | 0x38 | PROGRAM | 1 | mechanics |

- CALL (0x2D) keys in use: `0x3201`, `0x401F`, `0xAE11`, `0xE804` — all four implemented.
  (The hidden dispatch table's other three keys are unreached in shipped content.)

The opcode grind PLAN budgeted a milestone for is, at this point, **one modest slice**. What
actually stands between here and the credits is a short list of subsystems.

## 1. Decisions

| # | Decision | Rationale |
|---|---|---|
| D-RC1 | **Area generalization is the foundation slice and goes first.** `GAME_AREA = 2` is a hardcoded const the whole engine leans on (`engine.rs`), and a playthrough traverses all six ECL/GEO areas. Every other slice builds on an engine that can switch areas. | The single largest structural assumption left from M2; everything downstream (wilderness, encounters, the later chapters) needs it. |
| D-RC2 | **The playthrough is the discovery engine.** After the foundation + known-blocker slices, Bryan plays; each blocker becomes a slice with a `RESTRIKE_DEBUG_LOG` repro attached. We do not try to enumerate every gap up front. | The playtest-fix arc proved the loop: seven finds, seven fixes, each traced to a cited line. Walkthrough-derived guesswork is weaker evidence than a live blocker with a log. |
| D-RC3 | **H5 checkpoints ride the playthrough from the first session.** Bryan's runs record input traces + checkpoint hashes (the existing debug-log/replay tooling is the capture mechanism); segments get promoted to committed local-tier replays as areas stabilize. | Replay protection for six areas of content we'll be fixing under our own feet (D9/D10). |
| D-RC4 | **Copy protection: neutralized, faithfully.** The prompt appears with the answer shown (PLAN's decision, `docs/copy-protection.md` carries the algorithm + table). | Faithful-optional per D4; blocking a playthrough on a codewheel is not a goal. |
| D-RC5 | **Vancian camp magic is in scope (FD-25 closes here).** Memorize/scribe/rest-commit and camp Fix — a playthrough's casters must re-memorize. Combat casting already exists (M5); this is the camp half. | The one M5 deferral that hard-blocks a playthrough. |
| D-RC6 | **Non-combat COMBAT branches (temple/shop services) come in on demand** (D-RC2's loop), not as an up-front slice — with the sole exception of anything the Tilverton→overland exit path needs. | The deferral has held since M4 combat #6; scope on evidence. |
| D-RC7 | **The circle-back combat ledger stays behind its tripwires** (`h4-entry-state-snapshot.md` §50.5/§50.6). A tripped stub during the playthrough = a slice, with a staged capture where the ledger calls for one. | Tripwires are working exactly as designed; don't pre-pay. |

## 2. The gap map (known blockers, evidence-backed)

**G1 — Area generalization (D-RC1).** `GAME_AREA`/`INITIAL_ECL_BLOCK` consts; `load_3d_map`
records-not-swaps (FD-19's cross-area door has been routed around since M2); GEO/walldef/
8x8D/PIC/HEAD/BODY/BIGPIC files are all `{area}`-keyed and the engine loads area 2's;
NEWECL switches blocks within a file, never files. Slice = make `game_area` live state with
the full asset-swap on transition (the original's `load_ecl_dax`/`reload` path), and walk
the FD-19 door across a real boundary as acceptance.

**G2 — Wilderness/overworld.** `GameState::WildernessMap` exists as a flag with no
presentation: `RedrawView`'s non-dungeon branch (bigpic + `can_draw_bigpic`) is unmodeled
(`vmhost.rs` module doc), the Dalelands overland map is `BIGPIC1` block `0x79` (rendered
once in a picture test, never as a mode) with its blinking city cursor (`MapCursor`,
`ovr027.cs:164-171,225-231` — the displayInput wait-loop's other job), wilderness combat
floors fall back to `provisional_combat_map` with a loud note (M6b §7 deferral), and
overland movement/encounter rules are unread. This is the one genuinely new subsystem;
it gets its own short Fable door before implementation.

**G3 — Vancian camp magic (D-RC5, FD-25).** `SpellList` Learning-flag decode, Magic ▸
Memorize/Scribe staging, Rest's commit + clock advance + healing, camp Fix. The rules-pack
spell tables and combat casting exist; this is staging + commit + UI.

**G4 — The encounter cluster.** APPROACH / ENCOUNTER MENU / PARTYSTRENGTH / SPRITE OFF
opcodes + `sub_30580`'s encounter visuals (FD-34, which also unblocks the redraw gate's
fifth flag `displayPlayerSprite`) + `rest_incounter_*` random-encounter scheduling. One
coherent slice: the original's encounter presentation/decision loop.

**G5 — The opcode tail minus the clustered ones.** Items (FIND ITEM, DESTROY ITEMS),
roster (LOAD CHARACTER, ADD NPC — NPCs joining is quest-critical), TREASURE (+ the
deferred combat XP/treasure award from the M5 ledger — one mechanism), DAMAGE, SAVE TABLE,
CLEAR BOX, PROGRAM, and PARLAY (one use, but a dialogue beat presumably on the critical
path — scope its real shape from its one coab site before sizing).

**G6 — H5 + the finish.** Checkpointed playthrough traces (D-RC3), the copy-protection
prompt (D-RC4), and whatever D-RC2's loop surfaces that this map missed.

## 3. Slice plan (per the working model: Fable doors/specs/audits, Opus implements)

| # | Slice | Model | Needs a Fable door first? |
|---|---|---|---|
| 1 | Area generalization (G1) | Opus @ high, worktree | **Yes — short**: the area-switch state shape (what moves into `EngineState`, what reloads, transition choreography from coab's `reload_ecl_and_pictures` path) |
| 2 | Opcode tail: items/roster/mechanics (G5 minus PARLAY/TREASURE) | Opus @ high | No — census sites + coab handlers, slice-1 pattern |
| 3 | Vancian camp magic (G3) | Opus @ high | **Yes — short**: the SpellList staging model (it touches the save-carried character record) |
| 4 | Encounter cluster (G4) | Opus @ high | No — FD-34 + the four opcodes' coab sites carry the spec |
| 5 | TREASURE + combat XP/treasure award | Opus @ high | No — the M5 ledger already cites the sites |
| 6 | PARLAY (scope first) | sized after its coab read | — |
| 7 | Wilderness/overworld (G2) | Opus @ high–xhigh | **Yes — full door** (new subsystem: travel model, encounter scheduling, bigpic mode, MapCursor) |
| 8+ | D-RC2 playthrough loop: Bryan plays, blockers become slices | as shaped | per item |

Slices 2/3/4/5 are mutually disjoint and can run as parallel worktrees once slice 1 merges
(they all want the generalized engine underneath). Gates at every commit are the standing
battery: guard 16/16, reel smoke 16/16, workspace tests growing, clippy 0, fmt, draw-parity
— plus, from slice 1 on, a cross-area walk demo as the area-generalization regression.

## 4. Exit gate

1. **Bryan finishes Curse of the Azure Bonds start-to-end in restrike**, importing his own
   party, playing on the desktop.
2. The run exists as H5 checkpoint traces (local-tier, hashes-only in CI) that replay green.
3. The census reports 100% of *reached* opcode uses implemented (or consciously no-op'd
   with a docket entry).
4. The fidelity docket's open items are each resolved or explicitly deferred-with-rationale.
5. Guard + reel + battery green throughout; the circle-back ledger's tripwires that fired
   during the run are closed (D-RC7).

The "it's real" moment, per PLAN D12: the repo has been public all along; whether to
announce anywhere is decided at this gate, not presumed.
