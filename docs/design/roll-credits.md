# Roll Credits — the CotAB-completable milestone

> Door opened 2026-08-09 (post-M6 ratification + the playtest-fix arc); **v2 after the
> adversarial design review of the same day** (independent Opus pass: 2 BLOCKERs, 8 MAJORs,
> 6 evidence corrections folded — the reviewer independently re-extracted and re-disassembled
> all 25 ECL blocks and reproduced the census's 3,582 instructions exactly); **v3 after the
> same reviewer's re-review of the fold** (verdict: ratify with 8 amendments, all folded —
> including its catch that v2 mis-folded one of its own findings: LOAD CHARACTER is a
> party-slot selector, ADD NPC is the original's only join mechanism, and "how do the quest
> NPCs join?" is now G5's named open research item). **RATIFIED by Bryan 2026-08-09**
> (D13, on the reviewer's ratify-with-amendments verdict with all amendments folded). This is the
> working plan for PLAN.md's "M6 Roll credits" (shifted right by the working-ledger
> renumbering): **finish Curse of the Azure Bonds start-to-end in our engine.** Decisions are
> locked on Bryan's ratification; per-slice research items are named as such.

## 0. Where we stand (the dashboard, census of 2026-08-09)

`restrike census` against the real data (v1.3) reports: **6 ECL files, 25 blocks, 3,582
reached instructions, 52 distinct opcodes in use, zero decode hazards** (dockets 1–4 and 7
all clean). The derived numbers below come from diffing that report against the
interpreter's dispatch table by hand — the tool does not compute implemented-ness itself
(§4 item 3 makes that mechanical).

- **Use-weighted opcode coverage: 98.9%** (3,543 of 3,582 reached uses on implemented
  opcodes; independently re-derived by the review).
- **The unimplemented tail: 14 opcodes, 39 uses** — of which **7 uses (ADD NPC ×3,
  CLEAR BOX ×3, PROGRAM ×1) sit exclusively in `ECL1` block 82, the attract-mode demo**
  a playthrough never executes. The playthrough-relevant tail is **11 opcodes, 32 uses**:

  | op | name | uses | where | cluster (§2) |
  |---|---|---|---|---|
  | 0x0D | APPROACH | 8 | ECL2#2, ECL4#32 | encounters |
  | 0x40 | DESTROY ITEMS | 5 | ECL4#37, ECL5#48 | items |
  | 0x0A | LOAD CHARACTER | 4 | ECL1#80, ECL2#2, ECL5#48 | roster |
  | 0x27 | TREASURE | 4 | ECL2#4, ECL3#21, ECL4#37, ECL6#69 | treasure/XP |
  | 0x31 | SPRITE OFF | 3 | ECL2#2/#3/#4 | encounters |
  | 0x29 | ENCOUNTER MENU | 2 | ECL4#32, ECL6#64 | encounters |
  | 0x32 | FIND ITEM | 2 | ECL4#37, ECL5#48 | items |
  | 0x1D | PARTYSTRENGTH | 1 | ECL6#66 | encounters |
  | 0x2C | PARLAY | 1 | ECL3#16 | dialogue |
  | 0x2E | DAMAGE | 1 | ECL4#34 | mechanics |
  | 0x35 | SAVE TABLE | 1 | ECL6#67 | mechanics |
  | *(demo-only)* | ADD NPC / CLEAR BOX / PROGRAM | 7 | ECL1#82 | attract mode |

- All seven `KNOWN_CALL_KEYS` are implemented; the four reached in shipped content
  (`0x3201`, `0x401F`, `0xAE11`, `0xE804`) are the only four reached.
- **Caveat the number honestly:** "implemented opcode" ≠ "implemented behavior". `COMBAT`
  (34 uses) counts as implemented while its non-monster branch (`CMD_Combat`'s shop/temple/
  `AfterCombatExpAndTreasure` dispatch, `ovr003.cs:971-1029`) is a deferred stub; the spell
  tail (§2 G7) is invisible to opcode counting entirely. The dashboard says the *interpreter*
  is nearly done; the milestone's real work is in §2.

## 1. Decisions

| # | Decision | Rationale |
|---|---|---|
| D-RC0 | **Save/load wiring + the party-wipe flow is slice 0.** `Engine::take_io_request` / `saveload_fs::fulfill` / the camp Save screen all exist and are tested; **no frontend consumes the request** and `Shell::GameOver`'s tick is empty. A multi-session playthrough is impossible without it, and D-RC3 wants traces from the first session. | Review B2: exit-gate item 1 is unreachable today. Small slice — frontend wiring plus the wipe/reload flow. |
| D-RC1 | **Area generalization is the foundation slice and goes second.** The mechanism (review B1, from the shipped scripts): the area switch is `SAVE <n> → 0x7F12` — the `Area2.game_area` cell, `DataOffset 0x624`, whose original write hook is `seg042.set_game_area` (`ovr008.cs:654-657`, `seg042.cs:124-128` incl. the `game_area_backup` shadow) — immediately followed by a **cross-file NEWECL** (`load_ecl_dax` interpolates `gbl.game_area` at call time, `ovr008.cs:148`). There is **no transition choreography to invent**: every block's `0x8014` header vector opens with `LOAD FILES`/`LOAD PIECES` naming its own assets. Today the idiom fails **silently** (the `0x7F12` write raw-logs; the NEWECL to a block the resident file lacks halts quietly and the flow continues) — the worst case for D-RC2's loop. | The block-id namespace is globally partitioned 16-per-area across every asset family (the review's table); `GAME_AREA = 2` is an M2 hardcode threaded through every `format!("…{area}.DAX")` site. |
| D-RC2 | **The playthrough is the discovery engine** — after slices 0–8 (item use must exist before the deep dungeons), Bryan plays; slice 9 (the ending) lands during the run, needed only at the finale. Each blocker becomes a slice with a `RESTRIKE_DEBUG_LOG` repro. | The playtest-fix arc proved the loop. The review's gap-map additions (G6–G10) shrink what discovery must carry. |
| D-RC3 | **H5 checkpoints are engine-state digests, not frame hashes**, defined BEFORE traces accumulate: a checkpoint hashes position/area/block/clock/party (HP/XP/levels/inventory counts)/PRNG state **plus the ScriptMemory windows (`WindowsSnapshot` — where every `SAVE`/`GETTABLE` quest flag lives; `SAVE` is the game's most-used opcode) and `search_flags`** — re-recordable across renderer work, and able to distinguish quest progress, not just position. The capture/replay vehicle is the debug-log pipeline promoted to a shipped subcommand that **boots from an imported save** (today's `restrike walk` bare-boots and cannot replay an imported run; `replay_debug_log` is an example binary). | Review M7: frame hashes would be invalidated by slices 2/4/7's own pixel changes — the M6 arc demonstrated exactly this failure. |
| D-RC4 | **Copy protection: neutralized, faithfully** — prompt shown with the answer (algorithm + 6×36 table in `docs/copy-protection.md`; verify row-0 length at implementation, per that doc's own flag). | Faithful-optional per D4. |
| D-RC5 | **Vancian camp magic is in scope (FD-25 closes here).** | The M5 deferral that hard-blocks casters. |
| D-RC6 | **Temple services are promoted to a named gap (G8): death recovery is critical-path.** The shipped bestiary carries save-or-die poison (giant/phase spiders, wyvern), petrification (hooded medusa), and death rays (beholder) — and the Beholder Corps is plot-critical (`ECL4#37 @0x80CF`). Raise-dead has zero implementation; camp Fix is a status string; `decode_health_status` folds `stoned`/`gone` to `Okey`. Other shop/temple services stay on-demand. | Review M4. Also banked from the same pass: **CotAB ships no level-drain undead** (no vampire/wight/wraith/spectre in any `MON*CHA`) — that worry is settled and need not be re-litigated. |
| D-RC7 | **The circle-back combat ledger stays behind its tripwires — with the spell tail stated honestly (G7):** the playthrough starts with **three** implemented spell effects (Cure Light Wounds, Magic Missile, Hold Person); every other castable id is a loud tripwire. G7 sizes the must-have set up front rather than discovering ~84 spells one at a time. | Review M3: D-RC7 unamended would convert the spell list into an unbounded discovery stream. |
| D-RC8 | **Cache fidelity across areas is a conscious D11 call in slice 1:** the original's PIC/head-body/bigpic caches are block-id-keyed and **area-unkeyed** (`ovr030.cs:35,173-194,237`) — after an area switch it serves stale art when ids repeat. Ours inherits the same shape by accident; slice 1 decides replicate-vs-diverge and documents it. | Review B1(d). |

## 2. The gap map (known blockers, evidence-backed)

**G0 — Save/load + party wipe (D-RC0).** Frontend consumption of `SaveLoadRequest`
(`saveload_fs::fulfill` is written and tested), the `GameOver` death screen + reload flow
(`shell.rs`'s GameOver tick is empty), and the desktop's session ergonomics around it.

**G1 — Area generalization (D-RC1).** Name `0x7F12` in the ScriptMemory dispatch (read +
write hooks, the `game_area_backup` shadow); move `game_area` from the `engine.rs` const
into `EngineState` and thread every `{area}`-keyed loader (ECL, GEO, WALLDEF/8X8D,
`MON*CHA/SPC/ITM`, PIC/BIGPIC/HEAD/BODY, CPIC, and `ITEM{area}.DAX` when TREASURE lands);
close **FD-37** here (begin_chain's missing `vm_init_ecl` engine-half resets: the direct
`inDungeon = 1` write, `rest_incounter_*`, `can_cast_spells`, the
`reload_ecl_and_pictures`-conditional table restores, `ovr008.cs:109-131`); the D-RC8 cache
call; **and the save break (review M1):** `.rsav` carries no area — `SaveState` gains it
(version bump, golden regen), `rebuild_engine` stops hardcoding, and `import_original`
starts honoring `MasterSave.game_area` instead of ignoring the parsed byte. Acceptance: the
FD-19 door crossed live, plus the `ECL4#37`/`ECL5#48` overland exits (`SAVE 1 → 0x7F12` +
`NEWECL 0x50`) reaching `ECL1#80`'s "YOU ARE AT THE EDGE OF…" menu.

**G2 — Wilderness/overworld.** The overland mode: `ECL1#80` (no GEO1 exists — area 1 is
bigpic-driven by construction), the four full-screen backdrops (BIGPIC1 blocks `0x79`/`0x7B`,
BIGPIC2 `0x78`, BIGPIC6 `0x7A`), `RedrawView`'s non-dungeon branch + `can_draw_bigpic`, the
`MapCursor` blink (`ovr027.cs:165-172,181,320,335`; class `ovr028.cs:5`), `op_load_files`'
missing `lastDaxBlockId != 0x50` condition (`ovr003.cs:526-531`), overland
movement/encounter scheduling, and wilderness combat floors (the M6b
`WildernessFloorDeferred` fallback). Own full door before implementation.

**G3 — Vancian camp magic (D-RC5, FD-25).** `SpellList` Learning-flag decode, Memorize/
Scribe staging, Rest's commit + clock + healing, camp Fix's real behavior.

**G4 — The encounter cluster.** APPROACH / ENCOUNTER MENU / PARTYSTRENGTH / SPRITE OFF +
`sub_30580` (FD-34, which also completes the redraw gate's fifth flag) + `rest_incounter_*`
scheduling.

**G5 — Items + roster + mechanics tail.** FIND ITEM / DESTROY ITEMS; LOAD CHARACTER —
a **party-slot selector**, not a join op (`CMD_LoadCharacter`, `ovr003.cs:174-210`: sets
`SelectedPlayer`/`player_not_found` from a TeamList index; the shipped idiom is `ECL5#48
@0x80A1`'s slot-scan loop; its `ECL1#80` site is not live until slice 7 — acceptance for
that one is synthetic until then); DAMAGE; SAVE TABLE; TREASURE + the deferred combat
XP/treasure award (one mechanism, M5 ledger). PARLAY is its own small item: its single use
(`ECL3#16 @0x8B15`) is a six-operand boolean-outcome negotiation feeding a COMPARE — not a
dialogue tree (review E6). **Open research item (re-review A1): how do the quest NPCs
join?** `CMD_AddNPC` is the original's ONLY join mechanism (`ovr003.cs:1769-1782` →
`load_npc` → `TeamList.Add`, `ovr017.cs:878-896`; `load_npc` has exactly one caller) — yet
its only shipped uses are the demo block's, and `ECL5#48` shows Akabar *leaving* plus a
`Control.NPC_Base` scan at `0x7CB8`. Leads: the imported roster, the NPC-flag scan.
Resolve before slice 3's spec is final.

**G6 — Out-of-combat item use (review M5).** Potions/scrolls/wands: combat `UseItem` is a
tripwire and the character sheet has no Use verb at all. The standard Gold Box survival
mechanism; must exist before the deep dungeons.

**G7 — The spell tail (review M3).** Three effects exist; Bless/Protection are
imported-affect handlers with no casting path. Slice = enumerate the must-have set for a
CotAB run (the party-buff staples + Dispel Magic, Remove Curse, Neutralize Poison, Stone to
Flesh, and G8's clerical services), implement those, and leave the exotic remainder to
D-RC7's tripwires with the count stated.

**G8 — Death recovery (D-RC6, review M4).** Temple raise-dead (the non-monster COMBAT
branch's temple dispatch), the `stoned`/`gone` health-status decode fix, and whatever the
poison/petrification arcs need to be survivable-and-recoverable.

**G9 — The ending sequence (review M9).** The endgame choreography, FD-32's progressive
fade, and the credits themselves — named deliverables with their transcription sources
identified during G2/G1's area work (the finale lives in area 6).

**G10 — The demo/attract mode: explicitly out of scope.** `ECL1#82` (the fake fight,
`PICTURE 0x7B`, CLEAR BOX, PROGRAM) is not on any playthrough path; CLEAR BOX and PROGRAM
are implemented only if trivial or consciously no-op'd with a docket entry (§4 item 3
accepts either). **ADD NPC is NOT descoped here**: it is the game's only join mechanism
(G5's open item) and holds its scope decision until that resolves.

**Where the playthrough begins (review M8, settled by the re-review):** the amnesia intro
in Tilverton — area 2, block 1 (`ECL2#1 @0x8051` carries the real intro text; `ECL1#82`'s
version is the attract-mode's separate narration) — which is exactly our current boot
posture with the imported GOG slot. The boot ordering is unambiguous: `seg001.cs:142` sets
`game_area = 2` for gameplay; `InitFirst`/`InitAgain`'s `game_area = 1` (`:276`, `:369`)
are the title/demo-loop resets around it, not the gameplay value.

## 3. Slice plan (per the working model: Fable doors/specs/audits, Opus implements)

Sequenced, not parallel-by-default — the review (M2) showed the tail slices collide in
`machine.rs`'s dispatch, the `VmHost` traits, and the `Request`/`Effect` enums whose serde
shapes live inside `SaveState` (each addition = a version bump + golden regen). **Slice 1
owns the version-bump churn**; later slices rebase onto it and batch their enum additions.

| # | Slice | Model | Door? | Depends on |
|---|---|---|---|---|
| 0 | Save/load wiring + GameOver/wipe flow (G0) | Opus @ high | No | — |
| 1 | Area generalization (G1, incl. the save bump + FD-37 + D-RC8) | Opus @ high | **Yes — short** (Fable: the `0x7F12` hook shape + `EngineState` threading + the cache call) | 0 |
| 2 | Encounter cluster (G4) | Opus @ high | No | 1 |
| 3 | Items/roster/mechanics tail + TREASURE/XP + PARLAY (G5) | Opus @ high | No | 1 (rebases on 2's enum batch) |
| 4 | Vancian camp magic (G3) | Opus @ high | **Yes — short** (the SpellList staging model on the character record) | 1 |
| 5 | Spell tail must-haves (G7) | Opus @ high | No — G7's enumeration IS the spec | 4 |
| 6 | Death recovery + temple services (G8) | Opus @ high | No | 4 (shares the record), 5 (clerical spells) |
| 7 | Wilderness/overworld (G2) | Opus @ high–xhigh | **Yes — full door** | 1 |
| 8 | Out-of-combat item use (G6) | Opus @ high | No | 1 |
| 9 | Ending sequence + FD-32 fade (G9) | sized during G2/G1 work | — | 7 |
| 10+ | D-RC2's playthrough loop | as shaped | per item | rolling |

The one genuinely safe parallel is **slice 8** (out-of-combat item use — screens/UI
territory) against the VM-side slices; everything else sequences as listed (re-review A3:
slices 3 and 4 both reshape `Character`'s serde, and slice 7 shares `machine.rs`'s
`op_load_files` and G4's encounter scheduling — the earlier parallel claim was wrong). Gates at every commit: the standing battery (guard 16/16, reel
smoke 16/16, workspace growing, clippy 0, fmt, draw-parity) plus, from slice 1 on, the
cross-area walk demo as the standing regression.

## 4. Exit gate

1. **Bryan finishes Curse of the Azure Bonds start-to-end in restrike** (imported party,
   desktop, multi-session via slice 0's save/load), from the Tilverton intro to the credits.
2. The run exists as **H5 state-digest checkpoint traces** (D-RC3's definition) that replay
   green from imported-save boots; local-tier, hashes-only in CI.
3. **`restrike census --implemented`** (a small tool addition: the dispatch table exported to
   the census) reports 100% of reached opcode uses implemented **excluding exactly `ECL1`
   block 82** (the demo — the tool hardcodes that one exclusion so the number is
   reproducible) — or consciously no-op'd with a docket entry. Mechanical, not hand-diffed.
4. A **docket sweep slice** (scheduled in the 10+ loop, before the gate closes) walks every
   open fidelity-docket item to resolved or explicitly deferred-with-rationale — the gate
   names the slice rather than assuming the state.
5. Guard + reel + battery green throughout; circle-back tripwires that fired during the run
   are closed (D-RC7), with G7's implemented-spell count restated at the gate.

The "it's real" moment, per PLAN D12: the repo has been public all along; whether to
announce anywhere is decided at this gate, not presumed.

## 5. Slice-1 door: area generalization (Fable, 2026-08-09)

The mechanism is settled (D-RC1/G1); this section fixes the implementation shape.

**D-S1a — the `0x7F12` hook.** `EngineVmHost::write`'s Party-window arm gains the named
case: `set_game_area`'s exact semantics (`seg042.cs:124-128`) — `game_area_backup = game_area;
game_area = value` — both cells on `EngineState` (`game_area: u8`, `game_area_backup: u8`).
The read side returns the live value. `restore_game_area` (`seg042.cs:131-134`, backup →
live) gets a method now and a caller when one is found in reached content — none is known;
its writes at `seg001.cs:277,370` are the title-loop resets.

**D-S1b — threading.** `GAME_AREA` the const survives only as the boot/import DEFAULT;
every `format!("…{area}.DAX")` site reads the state: `vmhost.rs` (ECL `:74`, GEO `:97`,
WALLDEF/8X8D `:126,:139`, `MON*` `:890`), `picture.rs` (PIC/BIGPIC/HEAD/BODY
`:557,:570,:586,:593`), combat art (CPIC), and `FlowCtx.game_area` becomes a read of the
state, not the const. (Line numbers are this week's; re-locate, don't trust.)

**D-S1c — the save break (owned here, once).** `SaveState` gains `game_area` +
`game_area_backup` → `SAVE_FORMAT_VERSION` bump + synthetic golden recompute in the same
commit (the documented discipline); `rebuild_engine` reads the saved value;
`import_original` honors `MasterSave.game_area` (already parsed, currently ignored) for
file loading AND the resident-block resolve. Later slices batch their own serde additions
per §3's churn rule — this is the bump they rebase onto.

**D-S1d — FD-37 closes here** (begin_chain's `vm_init_ecl` engine-half completion), with
the transcription details preserved: `inDungeon = 1` is a DIRECT struct write
(`ovr008.cs:126`) that bypasses `vm_SetMemoryValue`'s `game_state` hook — ours writes the
raw cell without touching `game_state`, exactly that asymmetry; `rest_incounter_*` and
`can_cast_spells` reset at their confirmed cells; the `reload_ecl_and_pictures == false`
arm's `RestField200Values`/`RestField6F2Values` (`ovr008.cs:128-131`) — read both bodies
and transcribe (research item inside the slice, not guessed here).

**D-S1e — caches: REPLICATE the original's area-unkeyed shape (D-RC8 resolved).**
Faithful-first (D4/D11): the original serves the previous area's art when a block id
repeats after a switch; ours keeps the same block-id keying and gains a docket entry
documenting the quirk with a repeating-id example, plus a note that an opt-in QoL
correction (keying by `(area, block)`) is available later. Rationale: diverging silently
would "fix" behavior we have never observed in DOSBox; if the quirk proves ugly in play,
the QoL toggle is a one-line key change behind a decision we will then make deliberately.

**Acceptance.** (1) Synthetic: a two-area fixture (two ECL files, distinct GEO/wallsets)
proving `SAVE → 0x7F12` + cross-file NEWECL end-to-end — assets swapped, block resident,
`vm_init_ecl` resets applied. (2) Real data: the FD-19 door — the cross-area transition
M2's circuit found and routed around — walked LIVE, arriving with the destination area's
map and art on screen; plus a targeted vector-level drive of one of the two overland exits
(`ECL5#48 @0x8092` or `ECL4#37 @0x8225`) reaching `ECL1#80`'s "YOU ARE AT THE EDGE OF…"
menu (scripted replies; full wilderness PRESENTATION is slice 7 — arrival at the menu with
the right resident block is the slice-1 bar). (3) The silent-failure regression: a
cross-file NEWECL that CANNOT resolve still halts loudly — but one that can, never
half-transitions.
