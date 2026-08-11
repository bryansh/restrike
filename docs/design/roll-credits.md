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

> ⚠ **SUPERSEDED 2026-08-10 by §7.** Every count in this section came from a
> census whose flow-follower silently dropped the false arm of the shipped
> `IF <cmp>` + `GOTO` idiom. With that fixed, the same command reports
> **14,183 reached instructions, not 3,582** — and the tail below is wrong in
> both directions (every "unimplemented" count grows; `ADD NPC`, `CLEAR BOX`
> and `PROGRAM` are *not* demo-only). §7 carries the corrected dashboard and
> the evidence. The section is kept verbatim because the review that produced
> it, and D-RC1/D-RC2's sequencing, were reasoned from these numbers.

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
  is nearly done; the milestone's real work is in §2. **(Slice 6 landed the temple and
  AfterCombat arms of that dispatch — §10.3; `CityShop` is the one still reported-not-run.)**

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
scheduling. **LANDED 2026-08-10 — see §6.** All five opcodes implemented and FD-34 resolved;
the scheduling half is transcribed and tested but has no loop to drive it until G3 lands
Rest (FD-44).

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
D-RC7's tripwires with the count stated. **LANDED 2026-08-11 — see §9.**
**23 implemented, 77 tripwired**; the set was sized from the slot-A party's own two
spell books (§9.1), and *Stone to Flesh does not exist in CotAB* — the medusa
answer is a temple service and belongs to G8.

**G8 — Death recovery (D-RC6, review M4).** Temple raise-dead (the non-monster COMBAT
branch's temple dispatch), the `stoned`/`gone` health-status decode fix, and whatever the
poison/petrification arcs need to be survivable-and-recoverable. **LANDED 2026-08-11 —
see §10.** All ten temple services, the whole nine-rung `Status` ladder, the poison clock
(tick, lapse and cure), and — outside the door's own scope — **the post-fight writeback**,
without which every wound healed itself the moment the results screen closed.

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
| 2 | Encounter cluster (G4) — **landed, §6** | Opus @ high | No | 1 |
| 3 | Items/roster/mechanics tail + TREASURE/XP + PARLAY (G5) | Opus @ high | No | 1 (rebases on 2's enum batch) |
| 4 | Vancian camp magic (G3) | Opus @ high | **Yes — short** (the SpellList staging model on the character record) | 1 |
| 5 | Spell tail must-haves (G7) — **landed, §9** | Opus @ high | No — G7's enumeration IS the spec | 4 |
| 6 | Death recovery + temple services (G8) — **landed, §10** | Opus @ high | No | 4 (shares the record), 5 (clerical spells) |
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

**Status: LANDED 2026-08-10.** Implementation notes and the corrections the
code forced on this door:

- **D-S1a** — landed as written. The read side does return the live value
  (`get_player_values`' own `arg_4 == 0x312` arm, `ovr008.cs:545-548`, sets its
  found-flag so the Area2 struct shadow is never consulted). **Correction:**
  `restore_game_area` *does* have a reached caller — `LoadPlayerCombatIcon`
  brackets its work `set_game_area(1)` … `restore_game_area()`
  (`ovr017.cs:88,120`), reached from `loadSaveGame` for every non-NPC party
  member (`:1058`) and from `ovr018` throughout. It is nonetheless vestigial
  for asset selection: everything it wraps takes `chead_cbody_comspr_icon`'s
  `CHEAD`/`CBODY` branch (`ovr034.cs:57-66`), which never appends
  `gbl.game_area`. Our own party-icon loader takes no area argument, so the
  method exists without a caller here for a *reason*, not by omission.
- **D-S1b** — landed, but `FlowCtx.game_area`/`EngineVmHost.game_area` became
  **methods**, not fields. A `u8` snapshot taken at context construction is
  stale by exactly the instruction that matters: the original interpolates
  `gbl.game_area` at each load's own call time, which is what lets a `SAVE`
  earlier in the same run redirect the `NEWECL` after it.
- **D-S1c** — landed; `SAVE_FORMAT_VERSION` 3 → 4, one golden recompute. Two
  additions the door did not name but the mechanism needs: `EngineState`
  also gained `last_pos` (`area_ptr.lastXPos`/`lastYPos` — see FD-19) and
  `can_cast_spells` (FD-37), so the slice takes exactly one save break. Both
  load paths also picked up `loadSaveGame`'s own `inDungeon` gate on the
  3D-map reload (`ovr017.cs:1076-1095`) — area 1 ships no `GEO1.DAX`, so an
  unconditional load would make every wilderness save unrestorable.
- **D-S1d** — FD-37 closed. Two findings: `compare_flags` needed **no**
  `gbx-vm` seam (`EclMachine::load_block` starts `flags: [false; 6]` with an
  empty call stack, and every `vm_init_ecl` site rebuilds the machine), and
  `can_cast_spells` sits at an **odd** DataOffset (`0x1FF`), so no script can
  address it at all. `:126`'s direct `inDungeon = 1` also forced the *read*
  side to move to the raw cell (`Classes/Area1.cs:495-496`) — otherwise a
  block entered from the overland refuses its own `LOAD FILES` map load.
- **D-S1e** — replicated, with FD-43 documenting the quirk and its concrete
  repeating ids (`PIC` block 1 exists in all six area files).
- **Also required, and not in the door:** `load_3d_map` had to actually swap
  the resident `GeoBlock` (`Load3DMap`, `ovr031.cs:690-705`) — it recorded an
  id and nothing else from M2 until now — and three Area cells had to be named
  (`lastXPos`/`lastYPos`/`LastEclBlockId`, `0x4BF0`/`0x4BF1`/`0x4BF2`) before
  any destination block's arrival branch could work. **FD-19 is resolved and
  its headline was wrong**: the (7,12)-North door is not an area transition,
  it is a `lastXPos`/`lastYPos` bounce-back (the (0,0) landing was those two
  unmaintained cells), with a guarded fight and a *same-area* `NEWECL 2` on
  the other arm.

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

## 6. Slice 2: the encounter cluster (G4) — LANDED 2026-08-10

No door was written for this one (§3: "No"), so this section is the record of
what the code found. Five opcodes and FD-34, plus the scheduling half of G4.

**What was actually missing.** `area2_ptr.encounter_distance` was never carried
between opcodes — SETUP MONSTER computed a ray and threw it away, so APPROACH
had nothing to decrement and `CMD_Combat` placed monsters from a fresh ray
instead of from the cell. That one gap is why the whole cluster reads as
presentation-only until you wire it: with the cell real, `SETUP MONSTER s,1,p`
+ `APPROACH` genuinely starts the fight adjacent, which is the mechanical
content of an approach.

**The save break** (the second and, per §3's churn rule, the last one this
milestone should need before slice 3 rebases on it): `SAVE_FORMAT_VERSION`
4 → 5. `EngineState` gained `encounter_distance` / `max_encounter_distance`
(`Area2` `0x582`/`0x580`, named at Party-window `0x7EC1`/`0x7EC0`),
`sprite_block_id` / `pic_block_id`, and `encounter_flags`; `PictureLayer`
gained the `SPRIT` pair and `Shown::Sprite`; `VmMemoryState` gained
`display_player_sprite`, `byte_1EE95`, `byte_1EE96`. One golden recompute, in
the same commit, as the discipline requires.

**FD-34 resolved, and it completes the redraw gate's inner disjunction.**
`sub_30580` splits on the seam `redraw_view_gate` already established: flags at
execution time, pixels as `Effect::EncounterVisual` at presentation time. That
is not a stylistic choice — `PICTURE 0xFF` immediately followed by
`CALL 0xAE11` is real shipped content (`ECL2#2 @0x8307`), and drawing at
execution time would paint the sprite *before* the queued clear wiped it.
`displayPlayerSprite` is now the gate's fifth flag; only FD-35's outer
`byte_1AB0B` conjunct remains.

**Four corrections the code forced, none of which the opcode names imply:**

- **The distance bands are pre-rendered art, not scaling.** `Show3DSprite`'s
  second argument is a 1-based frame index with a hard 1..=3 range check
  (`ovr030.cs:215-226`), and `sub_30580` passes `encounter_distance + 1`. Every
  real `SPRIT` block carries exactly three frames, largest first — `SPRIT2`
  block 1 is 32×80, 24×65, 16×57 at cells (4,1), (4,1), (5,1) — each blitted at
  its own header anchor, `(y_pos + 3, x_pos + 3)`.
- **The masked load's recolor is not a second transparency.** `load_pic_final`
  masks colour 0 to transparency-16 at decode (`:127`) and only *then* folds
  13 → 0 (`:129-132`), so colour-13 pixels are opaque black. Reversing the
  order would punch holes through every approach sprite.
- **ENCOUNTER MENU's fourth word resolves to slot 4, not slot 3.** PARLAY and
  ADVANCE share a position but not an index (`ovr003.cs:1363-1368`) — which is
  why `var_6` has five entries for four words, and why slot 3 is simply
  unreachable at distance 0. The outcome is chosen by *class*
  (`var_6[selection]`, one of five tables), not by slot, so the same word means
  different things in different encounters: `ECL4#32` ships `[0,3,0,0,3]`,
  `ECL6#64` ships `[2,1,2,3,4]`.
- **The two flee checks read opposite ends of the party.** The party gets away
  on its SLOWEST member (`:1384`); the monsters break off on the party's
  FASTEST (`:1442`). The pair is sampled once, before the loop.

**`byte_1EE95` is the flag that makes the menu look right.** Its only reader is
`sub_30580`'s close-up gate. Without it, an encounter menu at distance 0 would
cut to the portrait the moment it opened; with it, the 3D approach sprite stays
in the viewport for the whole decision — which the `ECL4#32` acceptance frame
shows directly.

**Random-encounter scheduling is transcribed, tested, and deliberately
unwired** (`crate::rest`, FD-44). Two facts settle the question this slice was
asked: it is **rest-only** — the walk loop has no engine-side encounter roll at
all, wandering monsters being entirely script-driven (`RANDOM` → `IF` →
`COMBAT`, the census's 53 `RANDOM` uses) — and its `period` counts *rest-loop
iterations*, each worth five units of clock slot 1, not minutes. The check
draws exactly one `Random(100) + 1` on firing iterations only, compared `<=`
against the percentage, so a percentage of 0 still burns the draw. Its sole
caller is `resting`'s loop, which belongs to G3/slice 4; the cell, the
arithmetic and its tests are landed now so that slice wires a transcription
rather than discovering one. The draw is unreachable from every capture path
(captures never camp) and, today, from every path at all.

**Acceptance.** (1) Micro-ECL conformance per opcode, VM-side, plus a live
engine drive of the ENCOUNTER MENU loop. (2) Real data: `ECL2#2 @0x8780`'s
approach driven end to end — the masked `SPRIT2` band standing in the corridor,
then one `APPROACH` later the `HEAD2`/`BODY2` close-up filling the viewport,
both dumped and eyeballed. (3) Real data: `ECL4#32 @0x98A9`'s ENCOUNTER MENU
live, with "COMBAT WAIT FLEE PARLAY" on the menu line under the encounter's own
text and the sprite still up behind it, answered both ways and checked against
the outcome table.

**Residual, named:** `CMD_Combat`'s non-monster branch is still a stub (§0's
caveat, unchanged); `CMD_Picture`'s `0xFF` arm still mutates its flags at
presentation time rather than execution time — harmless for everything the
goldens and the shipped scripts reach, and noted here rather than fixed inside
a slice that did not own it.

## 7. Slice 3: the items/roster/mechanics tail, TREASURE/XP, PARLAY (G5)

### 7.1 ★ Task 0 answered: the quest NPCs join through `ADD NPC`, and the census was blind to it

**Verdict: `ADD NPC` (0x36) is the join mechanism, it is used in shipped
playthrough content, and it gets a full implementation.** The premise that its
only uses were the attract-mode demo's was an artifact of the census tool, not
a fact about the game.

**The tool bug.** `disassemble()` gives `IF` a `SuccessorKind::Branch` that
enqueued only the *guarded* instruction — the skip successor (the IF-FALSE
path) was reached incidentally, through that instruction's own fall-through.
That works while the guarded instruction is `Sequential`. It fails completely
for `IF <cmp>` + `GOTO`, because `GOTO` is a `Jump` and has no fall-through —
and `IF <cmp>` + `GOTO` **is** the conditional branch every ECL block is built
out of. `IF` + `EXIT`/`RETURN` (the early-return idiom) failed the same way.
`handle_branch_skip` now enqueues the skip target whenever it agrees with the
guarded instruction's real decoded length (the divergent case still
quarantines, unchanged — that decode is untrusted by construction).

**What it cost.** Reached instructions went **3,582 → 14,183** on the same 25
blocks, with **zero decode hazards** and per-block code coverage rising to
70–98% for most blocks. Roughly three quarters of the shipped scripts were
being reported as inert data.

**The six join sites** (found by the corrected traversal, monster ids resolved
against `MON{area}CHA.DAX`):

| site | operands | who |
|---|---|---|
| `ECL3#17 @0x8D0F` | `ADD NPC 0x16, 0x64` | ALIAS (`MON3CHA` block 22) |
| `ECL3#17 @0x8D14` | `ADD NPC 0x17, 0x64` | DRAGONBAIT (`MON3CHA` block 23) |
| `ECL3#18 @0x9010` | `ADD NPC 0x16, 0x64` | ALIAS (the second quest path) |
| `ECL3#18 @0x9015` | `ADD NPC 0x17, 0x64` | DRAGONBAIT |
| `ECL5#49 @0x8BCE` | `ADD NPC 0x3B, 0x64` | AKABAR BEL AKAS (`MON5CHA` block 59) |
| `ECL6#66 @0x8A04` | `ADD NPC 0x43, 0x64` | RAKSHASA (`MON6CHA` block 67) |

`ECL3#17`'s site reads, in order: `PRINTCLEAR` (the introduction text) →
`ADD NPC 0x16` → `ADD NPC 0x17` → `SAVE 0x80, 0x4C2E` (the quest flag) →
three more lines. The decoded block text at that offset is
"THE FIGHTER INTRODUCES HERSELF AS ALIAS AND HER COMPANION AS DRAGONBAIT."
followed by "ALIAS AND DRAGONBAIT JOIN YOUR PARTY."

The last row is worth its own line: **area 6 recruits a RAKSHASA into the
party**. The record carries `control_morale = 0xB2`, which `CMD_AddNPC`
recomputes to the same value from its own operand (`(0x64 >> 1) + 0x80`).

**The corroborating negatives**, all independently checked this session:
`gbl.TeamList.Add`/`.Insert` has exactly five call sites in the whole
reference source — `CMD_LoadMonster`'s two monster spawns (`ovr003.cs:263`,
`:290`), `SetupDuel`'s cloned opponent (`ovr008.cs:1336`), camp's
reorder-in-place (`ovr016.cs:657`/`:661`/`:676`/`:680`, a `RemoveAt` +
re-`Insert` of a member already on the list), and `AssignPlayerIconId`
(`ovr017.cs:896`), whose only caller is `load_npc`, whose only caller is
`CMD_AddNPC`. The shipped GOG save's nine slots all hold the same six
player-generated characters with `control_morale == 0` — **no NPC arrives
pre-joined**. `Nacacia` and `Olive Ruskettle` have no `MON*CHA` record in any
area and therefore can never occupy a roster slot at all: the cluebook's
"joinable NPC" list is looser than the engine's. The engine's real list is
Alias, Dragonbait, Akabar — plus the area-6 impostor.

**Leaving is the mirror, and it is also shipped**: `ECL5#48 @0x809B` walks
slots 0..7 with `LOAD CHARACTER`, tests `control_morale >= 0x80` at `0x7CB8`,
name-matches at `0x7C00`, and calls `DUMP` (0x3E → `FreeCurrentPlayer`) — the
Akabar farewell.

### 7.2 The corrected dashboard

Same command, same data, after the traversal fix — **6 files, 25 blocks,
14,183 reached instructions, zero decode hazards**. The tail this slice
inherited is much bigger than §0 claimed, and G10's "demo-only" carve-out for
CLEAR BOX/PROGRAM/ADD NPC is **withdrawn**: all three are playthrough content.

| op | name | §0 said | really | this slice |
|---|---|---|---|---|
| 0x0A | LOAD CHARACTER | 4 | **42** | implemented |
| 0x27 | TREASURE | 4 | **63** | implemented |
| 0x2C | PARLAY | 1 | **15** | implemented |
| 0x2E | DAMAGE | 1 | **24** | implemented |
| 0x32 | FIND ITEM | 2 | **8** | implemented |
| 0x35 | SAVE TABLE | 1 | **8** | implemented |
| 0x36 | ADD NPC | 3 *(demo)* | **9** | implemented |
| 0x38 | PROGRAM | 1 *(demo)* | **13** | implemented (cases 0/3/8/9) |
| 0x3D | CLEAR BOX | 3 *(demo)* | **17** | implemented |
| 0x3E | DUMP | — | **5** | implemented (ADD NPC's mirror) |
| 0x3F | FIND SPECIAL | — | **2** | implemented (FIND ITEM's twin) |
| 0x40 | DESTROY ITEMS | 5 | **13** | implemented |

**Newly visible and still open** (named here rather than discovered later):
`ROB` (0x28, 10 uses), `WHO` (0x39, 7), `INPUT STRING` (0x10, 5), `SPELL`
(0x3B, 2). None are in this slice's brief; all four have their
`EngineServices` seams already declared. `CHECKPARTY` (0x1E, 1) and
`PROTECTION` (0x3C, 1) are likewise reached and stubbed.

### 7.3 What landed, and the corrections the code forced

**LOAD CHARACTER's slot 0 — coab is wrong.** `sub_262E9` (`ovr003:030B`,
`:0328`) seeds its cursor with the `TeamList` **head** and then walks it
`index` links, so `LOAD CHARACTER 0` selects member 0 and is *found*. coab's
`player_index > 0 && player_index < Count ? TeamList[index] : null`
(`ovr003.cs:186`) never finds slot 0, which would make `ECL5#48`'s own
`0..8` scan skip the first party member. The high-bit arm additionally needs
**both** `redrawPartySummary` flags armed (`:0377-0389`) — the flags are set
by two Party-window writes of zero, so a script arms them deliberately before
turning LOAD CHARACTER into a remove-this-member instruction.

**DAMAGE's draw order is its whole fidelity.** The damage roll first
(`:29BF`), then — whenever bit `0x40` is clear — one `roll_dice(party_size, 1)`
victim roll (`:29F1`), **including on the arm that immediately re-rolls and
discards it**, then the arm's own saves. In the hit-count arm the damage used
by hit *n* is the value rolled at the END of hit *n−1* (`:2BDE`), so the last
roll is drawn and thrown away. Bit `0x10` is *damage anyway on a successful
save*, not half damage. Bit `0x20` skips the save entirely — same outcome as a
failed save, but one fewer draw.

**SAVE TABLE is not GETTABLE mirrored**: `(value, base, index)` against
GETTABLE's `(base, index, dest)`.

**FIND ITEM / FIND SPECIAL leave only two flags meaningful.** All six are
cleared and exactly one armed, so `=` and `<>` mean something after them and
the other four relations are simply false.

**The award path is draw-free — completely.** The brief asked for a proof that
no award draw lands inside a captured stream; the real answer is stronger.
`calc_battle_exp`, `addExp`, `CleanupPlayersStateAfterCombat`,
`DeallocateNonTeamMemebers`, `distributeNpcTreasure`, `displayCombatResults`,
`poolMoney`, `share_pooled`, `TakePoolMoney` and `treasureOnGround` contain no
`Random`/`roll_dice` call at all; every draw in that neighbourhood of
`ovr006`/`ovr007`/`ovr022` is inside `randomBonus`, `create_item` or
`appraiseGemsJewels`, none of which the award reaches. There is nothing on the
path to perturb a capture with. The one draw-bearing neighbour is the script
opcode TREASURE's `>= 0x80` arm, and its shipped uses precede their `COMBAT`
(`ECL5#49 @0x94B3`), so those draws land before a capture's first recorded one.
Guard 16/16 and reel 16/16 confirm both.

**`distributeNpcTreasure` scales by an integer ratio.** `npcParts /
totalParts` is `int` division (`ovr006.cs:736`) *before* it reaches
`ScaleAll`, so one NPC among several PCs scales the pooled coins by **zero**.
Replicated verbatim; it is why "takes and hides his share" is remembered as
larceny.

**`calc_battle_exp`'s ordering is load-bearing**: the corpses pay into the
pool first, and only then is `GetExpWorth()` taken — so the coins the party
just won are themselves worth experience (gold-worth + 250/gem +
2200/jewelry), as are `+N` items at 400 per plus, *including* items a script
TREASURE dropped before the fight.

**Acceptance.** (1) Real data, live: the M6b bar brawl now ends on
`displayCombatResults` — "THE PARTY HAS WON. / EACH CHARACTER RECEIVES 35 /
EXPERIENCE POINTS." — with ten BAR PATRONs' twelve items in the pool, frame
dumped and eyeballed, and the party's records really gained the experience
(the demo asserts it). (2) Real data, live: `ECL3#16 @0x92C6`'s shipped
PARLAY driven through all five tones, each writing its operand-table outcome
into `0x7F79`, with the following `COMPARE` asserted — the site the old census
could not reach. (3) The six `ADD NPC` sites pinned by address and operands,
plus a test that the corrected traversal reaches them from the block's own
header vectors. (4) Both death screens rendered side by side and eyeballed:
different words, different box, and the ECL variant's un-skippable
three-second hold measured in ticks.

**Residual, named:** `distributeCombatTreasure`'s per-item `sl_select_item`
list (Take moves the first pooled item), `TakePoolMoney`'s per-coin dialog,
the Detect-Magic word, encumbrance in `share_pooled`, and PROGRAM's cases
0/8/9 (start menu, end-game, `TryEncamp` — G9 and G3 own those). Each reports
itself in the transcript rather than failing silently.

## 8. Slice-4 door: Vancian camp magic (Fable, 2026-08-10)

G3/D-RC5's implementation shape. FD-25 closes here; FD-44 (the rest-encounter wiring)
lands here too.

**D-S4a — the record model.** `MagicState.spell_list`'s raw bytes decode to the original's
`SpellList` (`Classes/SpellList.cs:19-110`): up to 84 entries of `(id, learning)`, whose
on-wire byte is `id | 0x80` while learning (`AddLearnt`: `id & 0x7F`, `Learning = id >
0x7F`). Model staged-vs-memorized exactly that way on `Character` (decoded form + the
byte round-trip); if `Character`'s serde shape changes, this slice owns ONE
`SAVE_FORMAT_VERSION` bump with the golden discipline.

**D-S4b — capacity.** `HowManySpellsPlayerCanLearn(spellClass, spellLevel)` — transcribe
its exact formula (the slot table minus memorized-plus-staged at that level; the rules
pack already carries the slot tables the training path uses). `gbl.spellCastingTable`'s
`spellClass`/`spellLevel` rows are the same table our combat casting reads.

**D-S4c — the four flows**, each from its coab site, words-per-presentation /
commands-per-core as always:
- **Memorize** (`ovr016.cs:301-375`): the staged-review pass (`SpellLoc.memorize`) with
  the "Memorize These Spells?" confirm and `cancel_memorize` on N; the grimoire picker
  loop (`SpellLoc.grimoire`, `spell_menu2`) gated per-pick by capacity; `AddLearn` stages.
- **Scribe** (`ovr016.cs:377+`): scroll → grimoire, with its level/knowability gates —
  read the full handler; the scroll item consumption is part of it.
- **Rest** (`ovr016.cs:274+` `rest_menu` + `ovr021.cs:516+` `resting`): the required-time
  computation from the staged list, the interactive countdown, and the commit —
  `MarkLearnt` per spell as its time elapses (`ovr021.cs:390-410`). ★ FD-44 wires here:
  `resting`'s loop calls the slice-2 `crate::rest` encounter schedule; an interruption
  runs `CampInterruptedAddr` (the ECL header's vector 3) via the real VM. `cast_count` is
  NEVER reset by rest (FD-25's core finding — re-pin it in a test).
- **Fix** (`FixTeam`, `ovr016.cs`): the cure-spell auto-heal loop over the party, which
  becomes real once clerics can memorize cures (slice 5 provides the cast; Fix's loop and
  its arithmetic land now against the existing Cure Light effect).
- `cancel_spells` at `MakeCamp`'s entry and exit (`ovr016.cs:1117,1150-region` — verify
  both sites): staged-but-uncommitted spells do not survive leaving camp.

**D-S4d — UI.** The camp Magic submenu's leaves stop reporting deferrals: Memorize/
Scribe/Display drive `spell_menu2`'s list presentation through the existing list-widget
machinery (the vertical-menu slice's `ListMenu`/painter — check `spell_menu2`'s own
layout before assuming which); Display renders the grimoire + memorized sets.

**D-S4e — acceptance.** On the real slot-A party: stage spells on a caster → Rest → the
staged list commits at the right elapsed time and casting capacity reflects it; a
mid-staging save round-trips (staging is record state); the rest interruption fires the
schedule and runs vector 3 (synthetic fixture; ECL5#48's own vector 3 if reachable);
`cast_count` untouched by rest, pinned; camp-exit cancels staging, pinned. Frame dumps of
the Memorize list and the Rest countdown, eyeballed.

### 8.1 What landed, and the corrections the code forced

**No save break.** The door budgeted one; the slice needed none. coab models
`SpellList` as an object with `Load`/`Save` over the record's 84 bytes at
`0x1E`, but the **original has no such object** — the array *is* the state
(`charStruct.spell_list db 84 dup(?)`), and "learning" is the high bit of the
stored id, which is exactly what `AddLearnt` decodes (`SpellList.cs:83`). So
`crate::magic` is a **view over `MagicState.spell_list`'s raw bytes**:
`Character`'s serde shape is untouched, mid-staging saves round-trip for free,
and `SAVE_FORMAT_VERSION` stays at 6. The four new `Screen` variants and the new
`Shell::CampInterrupt` are appended last, and postcard encodes a variant as its
index, so no committed golden moved either.

**The layout is proven three ways.** Adds fill from index 83 downward
(`SpellList.Save`'s own descending `idx`, plus doc §33's save-diff catching the
first memorized Magic Missile at record offset `0x71`); slot 0 is never used
(the binary's combat collector reads `0x1F..=0x71` only, `ovr010:062A-065D`);
reads run ascending (same collector, and coab's `Load` agrees after any
save/load cycle). The consequence is stated rather than tidied away: the
**most recently staged spell commits first**.

**Cited lines re-verified, with three corrections.**

| door said | actually |
|---|---|
| `cancel_spells` at `ovr016.cs:1117` and `:1150-region` | `:1095` (entry) and `:1154` (exit) |
| `MarkLearnt` per spell at `ovr021.cs:390-410` | `rest_memorize` is `:393-413`; `MarkLearnt` is `:403` |
| D-S4d: "Display renders the grimoire + memorized sets" | `magic_menu`'s `'D'` is `DisplayMagicEffects` (`:632`) — the party's running **affects**, not spells |

Everything else checked out: `SpellList.cs:19-110`, `HowManySpellsPlayerCanLearn`
at `ovr016.cs:99-113`, `memorize_spell` `:301-374`, `scribe_spell` `:377-499`,
`rest_menu` `:274-298`, `resting` `ovr021.cs:516-612`, `FixTeam` `:1035-1073`.

**The capacity formula**, transcribed: `spellCastCount[class, level-1]` minus
the count of entries in **`IdList`** — every entry at that (class, level),
**memorized and staged alike** (`:103`) — which is why a caster who wakes with
spells still in memory cannot re-fill those slots until they are cast. The
result is **signed**: an over-full level reads negative and every caller tests
`> 0`. The `spellCastCount` read is kept **flat** (`0x12D + class*5 + level-1`)
so the casting table's level-6/7 rows alias into the next class's row exactly as
the original's own indexing does, rather than panicking.

**The rest-time computation**, transcribed (`sub_44032`, `ovr016.cs:8-64`):

```text
count = 4 if anything is staged (spells OR scroll scribes)
count = 6 if any of them is level 3 or higher      (sequential ifs, not else-if)
minutes = count * 60 + (total_scribe_levels + total_spell_levels) * 15
```

i.e. **four hours of study, six for level 3+, plus fifteen minutes per spell
level**. `rest_menu` takes the party maximum and splits it (`:287-289`);
`count` is *hours*, ticked down one per twelve loop iterations by `sub_58C03`,
and twelve iterations is sixty minutes. The learning rate has an asymmetry that
is the original's: the **first** spell after the study period is armed at
`level * 2` iterations (ten minutes per level, `sub_58C03`), every subsequent
one at `level * 3` (fifteen, `CheckForSpellLearning`).

**★ FD-44 wired.** `RestSession::step`'s seventh action is slice 2's
`RestEncounterSchedule::check`, reading the live Party-window cells. An
interruption returns `ScreenTransition::CampInterrupted`, which runs
`cancel_spells`, rebuilds the exploration screen and enters
`Shell::CampInterrupt` — a real vector run on the resident block's header
**vector 3**, `CampInterruptedAddr`. Acceptance proves the site the way the
Look-vector test proves its own: the fixture block's vector 3 is the only vector
that `NEWECL`s to block 9. Draw-neutrality is unchanged and now argued through
the real caller: a test rests to completion with the schedule disarmed and
asserts the PRNG never moves; captures never camp; guard 16/16 and reel 16/16.

**★ FD-25 re-pinned, twice.** Nothing in `resting`, `CheckForSpellLearning`,
`sub_58C03`, `rest_memorize` or `rest_scribe` writes `spellCastCount`. It is
capacity, not a per-rest pool — pinned in `crate::rest` and again at shell level.

**Five more things the code forced, none of which the flow names imply:**

- **camp Fix prices its rest from capacity, then divides it.** When the party
  has lost *less* than the estimated healing, the whole rest is divided by the
  **integer ratio** `maxHealing / lost` (`:993-998`) — a scratched party's Fix
  takes minutes, a battered one's takes hours. `TotalHitpointsLost` has no
  status filter at all, so a corpse's full maximum is in the total.
- **`rest_scribe` gates on `> 0x80`** (`ovr021.cs:425`) while `sub_44032`'s own
  scroll scan uses `> 0x7f` (`ovr016.cs:34`). They disagree about exactly one
  byte — spell id 0 staged — which no real scroll can carry. Both as written.
- **`resting`'s opening `Array.Clear` is off by one against its own indexing**:
  players occupy `spellLaernTimeout[1..=Count]` but the clear covers
  `0..Count-1`, so the last member's timeout survives into the next rest.
  Replicated (it is the shape a `memset(base, 0, party_size * 2)` produces) and
  flagged for the listing.
- **`Scribe`'s third gate reads capacity, not free capacity**: a caster whose
  slots are all spoken for may still scribe, because scribing writes the
  grimoire, not the memorized list. And its staging write walks **every** item,
  not only scrolls (`:455-470`).
- **the spell list's initial highlight is not row 1.** `sl_select_item` runs
  `index_ptr++` then `menu_scroll_in_page(false, …)` before its first draw
  (`ovr027.cs:572-573`), and `skipHeadings` walking backwards off the leading
  `"1st Level"` heading wraps to the **bottom of the visible page**
  (`:443-455`) — row 10 in the Memorize box's 11 rows, i.e. the first
  *second*-level spell. Our `ListMenu` already reproduced the arithmetic; the
  acceptance drive confirmed it against real data.

**Acceptance.** (1) Real data, live: the bundled slot-A party's SHARA stages
three spells from her grimoire, the capacity table drops `5 5 2` → `5 2 2`, the
closing review reads `" FIND TRAPS (3)"`, and `REST TIME: 00:05:30`
(= 240 + 3×2×15) commits all three — frames dumped and eyeballed. (2) A
mid-staging save round-trips. (3) `cast_count` untouched by rest, pinned.
(4) Camp exit cancels staging and keeps memory, pinned. (5) The FD-44
interruption runs vector 3, with its uninterrupted mirror.

**Residual, named:** `crate::movement::GameClock`'s `MINUTES_PER_UNIT = 10` is
now provably wrong — `timeScales` plus `display_resting_time`'s own Days/Hours/
Mins mapping make slot 1 **one** minute and slot 2 **ten**, and coab's
`time_year` at `Area1` offset `0x196` is really **months** (slot 4 carries at 30,
slot 5 at 12) with the year in `field_198`. Correcting it moves the walk goldens
and the ScriptMemory clock cells, so it is docketed rather than folded into this
slice; the rest loop calls `step_game_time(1, 5)` with the original's own
arguments meanwhile. Also open: out-of-combat casting (`cast_spell`,
`ovr016.cs:159-200`) stays G7's; `scroll_5C912`'s read-magic *unhiding*
conditions and `CheckAffectsTimingOut` both need the out-of-combat affect system
(G7's tail) and are named at their sites; `FixTeam` never removes the memorized
cures it rolls, so coab's Fix can bank the same ones again — read as written,
flagged for the listing.

## 9. Slice 5: the spell must-haves (G7)

### 9.1 ★ Task 0 answered: the party's own two spell books size the set

D-RC7 asked for the must-have set to be **enumerated from evidence before
implementation** rather than discovered one tripwire at a time. The evidence is
the slot-A party itself, read straight off the bundled GOG save
(`demo::slice5_the_spell_books_that_size_the_set` prints it):

| member | class(es) | `spellCastCount` | grimoire (`spellBook[id-1]`) |
|---|---|---|---|
| MATHEW | Paladin 5 | — | — |
| MARK | Paladin 5 | — | — |
| TRAVIS | Fighter 4 / Thief 5 | — | — |
| LEDERA | Fighter 4 / **Magic-User 4** | MU `3 2 0 0 0` | Charm Person, Detect Magic, Enlarge, **Magic Missile**, Read Magic, **Sleep**, Knock, Stinking Cloud |
| SHARA | **Cleric 5** | CL `5 5 2 0 0` | **every** cleric spell at levels 1-3 (8 + 7 + 8 = 23) |
| PHILIPPE | **Magic-User 5** | MU `4 2 1 0 0` | LEDERA's eight, plus **Fireball** |

Three findings fall straight out of that table and they are what bound the set:

1. **`spellBook` is indexed `id − 1`** (`Player.cs:363`, `KnowsSpell`) — our
   `magic::knows_spell` already had it right, but reading the array by id gives
   a plausible-looking off-by-one list (SHARA appears to know `sleep` and
   `animate_dead`, and not `resist_cold`), so it is pinned here.
2. **The cleric's book is not a choice.** `calc_cleric_spells` + its caller
   (`ovr026.cs:83-98`, `ovr018.cs:781-793`) grant **every** cleric-class row at
   a level the character now has slots for, `animate_dead` excepted. So SHARA's
   L4 and L5 lists arrive automatically at cleric 7 and 9 — the cleric side of
   the set must be sized for the *whole run*, not for today's book.
3. **The magic-user's book is scarce and fixed** until Scribe adds to it, and
   `scroll_5C912`'s scribe gate needs a **`read_magic` affect** to unhide an
   unknown scroll (`ovr023.cs:351-356`) — slice 4 named that as G7's. Read Magic
   is therefore a *play-loop dependency*, not a nicety.

**The threat profile** (D-RC6) contributes the recovery path: save-or-die poison
(giant/phase spiders, wyvern), petrification (hooded medusa), death rays
(beholder), and **no level drain**. Of these, **only poison has a spell answer**
— *Stone to Flesh does not exist in CotAB*: the `Spells` enum runs `0x01..0x65`
and carries no such row (`Classes/Spells.cs:47-151`), so the medusa answer is a
temple service and belongs to G8, not here. Raise Dead (`0x4B`) is the one
clerical row G8 consumes, so its **effect** lands in this slice.

#### The must-have table

23 implemented, 77 left counted behind D-RC7's tripwires (the casting table has
100 real rows, `0x01..0x64`; `0x65` is the `Unknown10` terminator).

| id | spell | class/lvl | why it earns its place | coab handler |
|---|---|---|---|---|
| `0x01` | Bless | C1 | the party-buff staple; SHARA's book; affect `0x01`'s handler is already live (§47.7) | `cleric_bless` → `CastTeamSpell` `ovr023.cs:990-1006` |
| `0x02` | Curse | C1 | the same function with the opposite team — free once Bless lands | `cleric_curse` `:1008` |
| `0x03` | Cure Light Wounds | C1 | **exists**; this slice adds the out-of-combat path | `SpellCureLight` `:1014` |
| `0x06` | Protection from Evil | C1 | buff staple; affect `0x08`'s handler is live; both paladins already carry it permanently | `SpellProtectionFromX` `:1036` |
| `0x07` | Protection from Good | C1 | the pair, same handler, one mirrored alignment gate | `SpellProtectionFromX` |
| `0x0F` | Magic Missile | MU1 | **exists** | `SpellMagicMissile` `:1166` |
| `0x12` | Read Magic | MU1 | ★ the Scribe gate — without it an unknown scroll lists nothing | `is_affected` `:1030` |
| `0x15` | Sleep | MU1 | the MU staple; both magic-users know it | `SpellSleep` `:1187` |
| `0x16` | Find Traps | C2 | SHARA's book, a dungeon crawl's utility spell | `is_affected` |
| `0x17` | Hold Person | C2 | **exists** | `SpellHoldX` `:1247` |
| `0x1A` | Slow Poison | C2 | ★ the poison arc, part 1 — the field answer to a spider bite | `is_affected2` `:1291` |
| `0x25` | Cure Blindness | C3 | recovery path | `SpellCureBlindness` `:1587` |
| `0x27` | Cure Disease | C3 | recovery path (and the `weaken`/`cause_disease_2` cascade) | `SpellCureDisease` `:1633` → `sub_5F037` `:1602` |
| `0x29` | Dispel Magic (cleric) | C3 | G7 names it | `SpellDispelMagic` `:1667` |
| `0x2A` | Prayer | C3 | the party-wide combat buff, via the radius-carrier scan | `SpellPrayer` `:1823` |
| `0x2B` | Remove Curse | C3 | G7 names it — the cursed-item release | `SpellRemoveCurse` `:1831` |
| `0x2E` | Dispel Magic (MU) | MU3 | one table row onto an implemented handler | `SpellDispelMagic` |
| `0x2F` | Fireball | MU3 | PHILIPPE's book; the MU damage staple | `sub_5F782` `:1878` |
| `0x3A` | Cure Serious Wounds | C4 | recovery; arrives with SHARA's cleric 7 | `SpellCureSeriousWounds` `:2177` |
| `0x43` | Neutralize Poison | C4 | ★ G7 names it — the poison arc, part 2 | `SpellNeutralizePoison` `:2242` |
| `0x45` | Protection from Evil, 10' Radius | C4 | the party-wide version, same handler + the carrier scan | `SpellProtectionFromX` |
| `0x47` | Cure Critical Wounds | C5 | recovery | `SpellCureCriticalWounds` `:2312` |
| `0x4B` | Raise Dead | C5 | ★ G8's critical path; the effect lands here, the temple service there | `SpellRaiseDead` `:2341` |

**Pruned, with the reason** (each stays a loud `spell-entry` tripwire): the
`cause_*` mirrors of every cure (a cleric who wants damage swings a mace);
`resist_cold` / `resist_fire` (a `PreDamage` damage-scaling hook nothing else
needs yet); `silence_15_radius`, `snake_charm`, `spiritual_hammer`,
`stinking_cloud`, `cloud_kill` (gas-cloud and summoned-item subsystems);
`charm_person` / `charm_monsters` / `confusion` / `fear` (runtime team flips);
`enlarge` / `strength` (`CalcStatBonuses` re-entry); `detect_magic` /
`detect_invisibility` (their payoff is the item-identify UI, G6); every Druid and
every Monster row (nobody in the party is a druid, and monster casts arrive with
their own captures). ★ **`Knock` (`0x1F`) is pruned because it is uncastable**:
its row is `targetType = Combat` *and* `whenCast = Camp`, so the camp path takes
`sub_5D2E1`'s "can't be cast here…" arm (`ovr023.cs:672`) and the combat path
takes `spell_menu3`'s "Camp Only Spell" arm (`ovr014.cs:1386`) — the original
ships it unreachable from either side.

### 9.2 What landed, and the corrections the code forced

**No save break.** The door budgeted one for the affect-record shape; the slice
needed none. `Character::affects` was already `Vec<Vec<u8>>` — the raw `.fx`
chain — so `crate::affects` operates on it in place, exactly as slice 4's
`crate::magic` does with `spell_list`. `AffectRecord` gained an `encode()` to
match its `decode()`, and that is the whole format change.
`SAVE_FORMAT_VERSION` stays at **6**. The one new `Screen` variant (`Cast`) and
the one new `RestSession` field (`affects_timed_out`, `#[serde(default)]`) are
both append-only, and postcard encodes a variant as its index, so no committed
golden moved.

**One table, two worlds.** The casting rows moved out of
`crate::combat::spells` into a new public `crate::spells`, because the original
casts from **one** `gbl.spellCastingTable` through **two** entry points —
`sub_5D2E1` with `gbl.SpellCastFunction` swapped (`ovr014.target` in combat,
`ovr023.NonCombatSpellCast` out of it). Combat keeps the combat half of the
machinery; `crate::camp_cast` is the other arm.

**Counts, restated for the §4 gate:** **23 implemented, 77 tripwired** of the
100 real rows (`0x01..0x64`). Of the 23, **18 are combat-castable** and **12 are
camp-castable** (seven overlap; five rows are `SpellTargets::Combat` and are
refused in camp, and five are `whenCast = Camp` and are refused in combat).

**Draw-neutrality, argued once.** A new row can only be reached two ways: an id
pulled from a combatant's `memorized_list` (decoded from that fight's own
character records) or a cast the player issues. No replay issues casts, and
**every pinned capture memorizes exactly `{0x03, 0x0F, 0x17}`** — the three rows
that already existed. That is not a claim in prose: `gbx-oracle`'s
`spell_rows.rs` re-reads all sixteen capture files and asserts it, printing the
ids it found. Guard 16/16 and reel 16/16 (62,108 draws checked live) then
confirm it end to end.

**Eight things the code forced.**

- ★ **The `SpellDamage` event was carrying the *unscaled* damage.** It rode at
  the `DoSpellCastingWork` call site, before `damage_person`'s save halving —
  invisible while Magic Missile (`DamageOnSave::Normal`, never scaled) was the
  only damage spell, and it drifted the presented board by half a fireball the
  instant one landed. The M6a scene's own `reconcile` caught it on the very
  first render of the new demo (`BoardDrift { hp_current, presented: -2, actual:
  11 }`). The event now comes from inside `damage_person` after the scaling,
  which is also the number the original prints (`ovr024.cs:1204-1208` reads
  `gbl.damage` *after* the halve). Draw-neutral, and Magic Missile's event is
  byte-identical.
- ★ **`spellBook` is indexed `id − 1`** (`Player.cs:363`). Reading it by id
  yields a plausible-looking but wrong grimoire (SHARA appears to know `sleep`
  and `animate_dead`, and not `resist_cold`). `magic::knows_spell` already had
  it right; §9.1 pins it so the next reader does not re-derive it.
- ★ **Buff durations only tick in camp.** `CheckAffectsTimingOut`'s first branch
  (`ovr021.cs:13-19`) fires when `game_state != Camping` and marks every
  `affects_timed_out` slot dirty *without decrementing anything*. So a Bless cast
  in camp survives an arbitrary amount of walking and expires during the **next**
  rest. Replicated, dirty flags and all, and pinned by
  `walking_never_ages_a_buff`.
- ★ **`NonCombatSpellCast`'s `WholeParty` arm puts the caster in the list
  twice** (`ovr023.cs:647-650`): `spellTargets` opens as `[SelectedPlayer]` at
  `:625` and the `WholeParty` case calls `AddRange` **without clearing** where
  every other case clears. Kept as written; the second pass finds the affect the
  first just planted, removes it and re-adds it, so the observable result is
  unchanged — but the message prints twice, which is the original's.
- ★ **Hold Person cannot be cast in camp** even though its `whenCast` is
  `Combat`: what refuses it is `targetType == SpellTargets::Combat`, so it takes
  the "can't be cast here… Lose it?" arm. A cleric who wakes with one and finds
  no fight can only burn the slot. Five of §9.1's rows behave this way
  (`0x02`, `0x0F`, `0x15`, `0x17`, `0x2F`).
- ★ **`Knock` is uncastable from either side** (§9.1's pruning note) — a shipped
  row the original can never fire.
- ★ **Dispel Magic in combat strips the *enemy's* buffs, not an ally's.** Its row
  pairs `targetType = PartyMember` with `field_E = 1`, and `field_E` is what
  `sub_4001C` reads: `find_target`'s list is the enemy near-list. The
  `targetType` only steers the out-of-combat cast. The `affect_data == 0xFF`
  marker every racial and item affect carries is what makes a dwarf's
  `dwarf_vs_orc` undispellable — the loop never rolls for it.
- ★ **`TryLooseSpell` now runs on the spell-damage path too.** `damage_person`'s
  tail (`ovr024.cs:1244` → `:1288-1300`) clears the target's `can_cast` and
  loses any queued cast, exactly as the melee swing already did (§45). Our Magic
  Missile used to call `apply_damage` directly and skip it, along with the
  `PreDamage` and `FireShield` affect dispatches; routing it through the real
  `damage_person` closed all three at once, and the guard held.

**What each half implements.** In combat: the `field_6` low-nibble targeting
shapes `0` (self) and `8..=0xE` (area, radius `field_6 & 7`, via a new
`build_sorted_from` anchored on a map point rather than a combatant) joined the
tail loop that was already there; `5` and `0xF` stay tripwired because no
must-have row uses them. `DoSpellCastingWork`, `ApplyAttackSpellAffect`,
`damage_person` and `GetSpellAffectTimeout` are transcribed; the
`fixedRange == -1` touch-attack arm and Dispel Magic's nine-cell ground sweep
are cited and tripwired, both unreachable from §9.1's set. Out of combat,
`crate::camp_cast` is `sub_5D2E1`'s non-combat arm: the "can't be cast here"
gate, `NonCombatSpellCast`'s three-way `targetType` switch (including
`selectAPlayer`'s "Cast Spell on whom"), and twelve effects — of which
**Remove Curse's item arm is one combat structurally cannot have**, because a
combatant carries no inventory.

**Two named residuals closed.** Slice 4 left `scroll_5C912`'s read-magic
unhiding and `CheckAffectsTimingOut` both waiting on "the out-of-combat affect
system (G7's tail)". Both land here: **Read Magic** (`0x12`) plants the affect
the scribe gate reads, and the timing-out runs at `RestSession::step`'s clock
call with the original's own `(slot 1, 5)` arguments.

**The radius-carrier range gate landed too.** M5's §39 modelled
`calc_affect_effect`'s carrier scan but **tripped** on any carrier it found,
because the range test and the handlers were "the spell slice's". They are
here: a carrier counts only when the dispatched combatant is inside
`Rebuild_SortedCombatantList(carrier, max_range, p => p == player)` — **6 for
prayer, 1 for the three radius blessings** (`ovr024.cs:119-126`). That gate is
the whole of Prayer's radius: `SpellPrayer`'s own targeting is `field_6 = 0`,
the caster alone, and everyone else is reached from here. The handler is passed
the **found** record, which for a radius kind is the *carrier's* — `AffectPrayer`
reads its `affect_data` for the team bit, and the original hands the same object
down (`:132`).

**Affects now cross the combat boundary.** `kits::party_kits` carries each
member's decoded chain into the fight and `combat_host::carry_affects_home`
carries the final one back. The surviving set is the original's by
construction: `RemoveCombatAffects`'s strip table already ran per-combatant on
anyone who died or fled, so `paralyze`/`sleep`/`stinking_cloud` are gone while
`bless`, `prayer` and `protection_from_evil` walk out of the fight still
running. `DisplayMagicEffects` (slice 4's Magic ▸ D) consequently shows
something real for the first time.

**Acceptance.** (1) Real data, live: `slice5_the_spell_books_that_size_the_set`
prints the two grimoires §9.1 is built from. (2)
`slice5_the_cleric_casts_in_camp` boots the bundled slot-A party, camps, and
casts three spells covering all three shapes — Bless lands on all six, Cure
Light Wounds opens "Cast Spell on whom" and heals the member the cursor walked
to, Hold Person is refused with "HOLD PERSON CAN'T BE CAST HERE…" + "LOSE IT?".
Five frames dumped and eyeballed, plus a sixth of Magic ▸ Display listing
"PROTECTION FROM EVIL / BLESS" per member. (3)
`slice5_a_bless_and_a_fireball_on_screen` plays both casts through the M6a scene
over the real art — 124 frames, eyeballed: "SHARA / CASTS A SPELL",
"SPELL:BLESS" on the prompt row, "PHILIPPE / IS BLESSED" from the new
`AffectApplied` event, then "SPELL:FIREBALL" with the missile mid-flight. (4)
`a_bless_and_a_fireball_in_one_scripted_fight` asserts the whole draw sequence
against the arithmetic: bless spends nothing, fireball spends one
`find_target` pick + five d6s + one d20 per target, and every survivor's loss is
the volley or exactly half of it.

**Gates, at every one of the eight commits.** Guard **16/16**, reel smoke
**16/16** (62,108 draws checked live), clippy 0, `fmt --check` clean, and the
workspace grown from 1,519 to **1,569** (plus four `#[ignore]` local demos:
the spell-book dump, the camp-cast drive and the on-screen casting beat). No
`.gbxtrace`, no game data, and no golden moved.

**Residuals, named.**

- ★ **The fight's HP and status never reach the roster.** `combat_host` writes
  back experience, treasure and (as of this slice) affects, but nothing syncs
  `hit_point_current` or `health_status` — so wounds do not persist past a
  fight. This is pre-existing and outside G7, but it is on the playthrough's
  critical path and should be its own small slice before D-RC2's loop starts.
  **CLOSED by slice 6 (§10.1)** — which also found that *spell expenditure* was
  never written back either.
- `gbl.damage_flags` (fire/cold/electricity/acid/magic) is not carried into
  `damage_person`. It is read only by the `resist_*` affect handlers, every one
  of which §9.1 pruned, so the visible consequence is "our fireball does not
  respect a resist-fire ring" — and it lands with whichever slice implements the
  first resist row.
- Fireball's `inDungeon == 0` re-target (a radius-**2** blast outdoors,
  `ovr023.cs:1894-1902`) is cited but not wired: `CombatState` carries no
  dungeon/wilderness flag yet. It belongs to G2.
- `AffectSlowPoison`'s kill-on-timeout (`ovr013.cs:305-317`) is transcribed in
  the citation but not fired: the out-of-combat `remove_affect` does not run
  `CallAffectTable(Remove)` handlers. Slow Poison therefore buys its five hours
  and then simply lapses. The handler wants the same dispatch table combat has,
  which is a G8-sized job alongside the poison arc itself. **CLOSED by slice 6
  (§10.5)** — and the same seam turned on `AffectPoisonDamage`'s ten-minute
  re-planting tick, which is the whole poison clock.
- The affect-effect handlers for the 77 tripwired rows remain tripwired. The
  ones **this slice's own rows plant** are all landed, because a spell whose
  affect trips `affect-effect` on every dispatch is not implemented: `cursed`
  0x02, `protection_from_good` 0x09, `prot_from_evil_10_radius` 0x2D /
  `prot_from_good_10_radius` 0x2E, `prayer` 0x31 and `blinded` 0x21 joined
  Bless's and Protection from Evil's, and `read_magic` 0x10 / `find_traps` 0x13
  / `slow_poison` 0x16 are explicit **no-ops** (the original's table maps the
  first two to `ovr013.empty` and the third to a timeout-only handler).

## 10. Slice 6: death recovery + temple services (G8)

**LANDED 2026-08-11.** Five deliverables, four commits, `SAVE_FORMAT_VERSION`
6 → 7 (one break, budgeted).

### 10.1 The post-fight writeback — and what the brief got wrong

Slice 5's flagged residual, closed. In the original there is **no** post-fight
sync function at all: `gbl.TeamList` holds the same `Player` objects the combat
overlays mutate, so `damage_person`'s hit points, the `Status` ladder,
`RemoveFromCombat`'s `in_combat`, the Quick word's `quick_fight`
(`ovr009.cs:709`) and `SpellList.ClearSpell` are already on the record when
`CleanupPlayersStateAfterCombat` (`ovr006.cs:169`) runs over it. Our split
roster/`Combatant` model has to copy them, and until this slice it copied only
the affect chain — so **wounds vanished after every fight**.

`combat_host::carry_state_home` carries the complete set:

| record cell | where combat writes it |
|---|---|
| `hit_point_current` @0x1A3 | `damage_person`/`heal_player` |
| `health_status` @0x195 | the `damage_player` ladder + `KillPlayer` |
| `in_combat` @0x196 | `RemoveFromCombat`, the ladder's non-conscious arm |
| `quick_fight` @0x198 | the Quick menu word (`ovr009.cs:709`), SPACE's revoke (`:233`) |
| `spell_list` @0x1E | `SpellList.ClearSpell` per cast |
| `affects` (`.fx`) | every add/remove in the fight (slice 5) |

★ **The brief assumed spell expenditure was already handled; it was not.**
Nothing wrote the fight's `memorized_list` back, so a cleric who spent every
Cure Light Wounds in a bar brawl walked out with them all still memorized. The
mapping is a multiset difference against the record's own collected entry list
(`records.rs`' non-zero sweep of `spell_list[1..]`), resolved one
`magic::clear_spell` per spent id — `ClearSpell`'s remove-the-first-match
semantics.

★ **The roster index map has to be taken before the first write.** `party_kits`
picks members by `hit_point_current > 0`, and the writeback is about to zero
that predicate for a casualty; resolving lazily (as the affect-only version
could afford to) shifts every actor past the first one who fell.

**Named, not carried:** the ready/unready cells (`ac`, `hit_bonus`, the current
attack profile, each item's `readied` flag). The original's per-turn
`AI_items_selection` really does re-ready a weapon and `reclac_player_values`
really does rewrite those cells; ours models the swap inside the fight's own
`Loadout` and never touches `Character::readied_items`, so writing the derived
cells back would desync the record from the inventory.

### 10.2 The health-status ladder

`decode_health_status` folded `tempgone` (2), `running` (3), **`stoned`** (7)
and **`gone`** (8) onto `Okey`, so a petrified or disintegrated record read back
as a healthy one. All nine `Status` values (`Classes/Enums.cs:7-18`) now
round-trip; `HealthStatus` gained `TempGone`/`Stoned`/`Gone` (appended **last**,
so postcard keeps every committed `.rsav`'s encoding), plus two predicates that
turn out to be the spine of deliverables D and E:

- `is_terminal` — `KillPlayer`'s refusal set `{dead, stoned, gone}`
  (`ovr024.cs:39-42`);
- `counts_as_alive` — `CleanupPlayersStateAfterCombat`'s liveness set
  `{running, animated, okey}` (`:221-223`). Note what is **not** in it:
  `unconscious` and `dying`.

**A correction kept rather than smoothed:** `damage_player` has **no**
terminal-status guard (`ovr025.cs:1183` opens straight on the arithmetic),
unlike `KillPlayer` and the DAMAGE opcode's `sub_32200` (`ovr008.cs:1403`). The
asymmetry is the original's. It is unreachable anyway, because `KillPlayer`
cleared the statue's `in_combat` and every target list filters on that.

**The roster tells the truth now.** The character sheet's `statusString` table
was already all nine rows; what was missing was the byte. And `PartySummary`
finally paints `displayPlayerName`'s three arms (`ovr025.cs:827-838`): M3 argued
the removed-colour arm could not fire on a walk-loop roster because a live
member's `in_combat` is true — which was only true because nothing wrote it
back. A member who fell in the last fight is now dark red in the panel.

### 10.3 The temple, and where the shipped ones are

`CMD_Combat`'s non-monster branch (`ovr003.cs:974-992`) is a three-way dispatch
on two `Area2` flags a script has just set: `EnterShop` (`0x6D8`) →
`ovr007.CityShop()`, `EnterTemple` (`0x5C4`) → `ovr005.temple_shop()`, neither →
`ovr006.AfterCombatExpAndTreasure()`. **That is why no opcode census could ever
show a temple: there is no temple opcode.** The flag write is a plain
`SAVE 1 → 0x7EE2`.

A scan of every `ECL*.DAX` block for the address finds exactly four shipped
temples, each the same three-instruction idiom:

| block | address | shape |
|---|---|---|
| `ECL1#80` | `0x8829` | `CLEARMONSTERS; SAVE 1 → 0x7EE2; COMBAT` |
| `ECL1#81` | `0x8677` | the same |
| ★ `ECL2#1` | `0x91DF` | `SAVE 0xFF → 0x7EE1` (HeadBlockId); `SAVE 1 → 0x7EE2`; `CLEARMONSTERS; COMBAT` |
| `ECL5#49` | `0x8E0C` | `CLEARMONSTERS; SAVE 1 → 0x7EE2; COMBAT` |

`ECL2#1` is Tilverton — the block the playthrough opens in (§2's "where the
playthrough begins"). `EnterShop` appears in nine places (`ECL1#80/81`,
`ECL2#1` ×2, `ECL4#32` ×3, `ECL4#37`, `ECL5#49`); its branch is reported and
left for the shop slice.

**The service table** (`temple_sl`, `ovr005.cs:13`; `temple_heal` builds **ten**
rows from an eleven-entry array, so the trailing `"Exit"` is never a list row
and the `case 10` in the dispatch switch is dead code):

| # | service | gp | effect |
|---|---|---|---|
| 0 | Cure Blindness | 1000 | `blinded` off |
| 1 | Cure Disease | 1000 | all six `disease_types` off |
| 2 | Cure Light Wounds | 100 | 1d8 |
| 3 | Cure Serious Wounds | 350 | 2d8+1 |
| 4 | Cure Critical Wounds | 600 | 3d8+3 |
| 5 | Heal | 5000 | full **minus 1d4**, + blindness/disease/`feeblemind` |
| 6 | Neutralize Poison | 1000 | `poisoned`/`slow_poison`/`poison_damage` off |
| 7 | Raise Dead | 5500 | §10.4 |
| 8 | Remove Curse | 3500 | `SpellRemoveCurse` |
| 9 | ★ Stone to Flesh | 2000 | `stoned` → `okey`, 1 hit point |

★ **Stone to Flesh is delivered here and only here** — slice 5 proved the spell
does not exist in CotAB (§9.1), so the temple is the whole medusa answer.

Three details worth keeping:

- `buy_cure` charges **the selected member's own purse first** and the **pooled
  money** second, never another member's coins (`:39-50`). `temple_shop` empties
  the pool on entry (`:406`), so `P` is the only way to fill it — and a dead,
  broke member can only be raised out of the pool.
- `gbl.SelectedPlayer` is **both the payer and the patient**. `G`/`O`
  (`scroll_team_list`, `:483-489`) is how the original picks them.
- Raise Dead and Stone to Flesh **re-test the condition after taking the
  money** (`:174`, `:290-291`). The temple keeps the gold either way, which is
  why the "cast cure anyway" prompt exists at all.

### 10.4 ★ Raise Dead's real mechanics, and two corrections with evidence

The temple's gate is `dead || animated`, with **no elf clause and no `Con > 0`
clause** — unlike the *spell* (`SpellRaiseDead`, `ovr023.cs:2343-2345`). So in
CotAB **an elf who dies can be raised at a temple and never by a cleric.** Then
`animate_dead` and `poisoned` come off under `cureSpell`, and the member stands
at exactly one hit point.

Two things the decompilation cannot be read literally on, both resolved against
the spell — which is the same routine, hand-inlined:

1. The Constitution guard reads `if (player.stats2.Con.full <= 0) {
   player.stats2.Con.full--; }` — a branch that can never fire for any character
   with a Constitution at all, and whose body would drive the score further
   negative if it did. The spell is unambiguous at the same point: `Con.cur > 0`
   as a precondition, `Con.cur--` unconditional. Read as a flipped comparison
   (`jle` for `jg`, one bit) the two agree, and that is how it is transcribed:
   **the raise costs a point of Constitution.**
2. The hit-point recompute (`var_107 = hp_max − hp_rolled; … var_107 /= var_108;
   hit_point_max = var_107`) sets a Constitution-16 fighter 5's maximum hit
   points to **1**. Its shape is an inlined `CalcStatBonuses(Stat.CON)` —
   `var_108`'s per-class loop is `ConHitPointBonus` (`ovr024.cs:782-831`) term
   for term — and *that* function is readable, is what the spell calls at exactly
   this point (`ovr023.cs:2358`), and does the job correctly. So the recompute is
   `temple::recalc_hp_for_con`, a transcription of `ovr024.cs:1053-1105`. The
   literal is preserved in the doc comment for a future reader with the
   overlay's disassembly. **`ovr005` is not in `coab_new.lst`** — the C#
   decompilation is the only source for it, which is why this is written down.

The member ends at exactly one hit point either way: the temple's inline writes
`hit_point_max` only (it has no counterpart to `CalcStatBonuses`'
`hit_point_current` delta), and the spell reaches the same place by ordering.

**The spell's own Raise Dead gained the recompute too** — slice 5 landed the
Constitution loss without `CalcStatBonuses`, so raising cost a stat point but no
hit points.

### 10.5 ★ The poison clock

Slice 5 named the seam: out-of-combat `remove_affect` never ran
`CallAffectTable(Remove)`, so `AffectSlowPoison`'s kill-on-timeout could not
fire. It fires now, gated on the record's own `callAffectTable` flag exactly as
`ovr024.cs:75-79` gates it, and `CheckAffectsTimingOut` routes its expiries
through `remove_affect` rather than dropping them in place (`ovr021.cs:79-81`).

What that turns on is the whole arc, and **both handlers ignore their `Effect`
argument**, so both run on the way *out*:

1. A failed save against a spider bite is **save-or-die**: `PoisonAttack`
   (`ovr013.cs:848-860`) plants a permanent `poisoned` and calls `KillPlayer`
   immediately. The affect marks the corpse.
2. **Slow Poison** (`is_affected2`, `ovr023.cs:1291-1310`) lifts a member at 0
   hit points to 1, plants `slow_poison` for the row's timeout and
   `poison_damage` for ten minutes — both with `callAffectTable`.
3. Every ten minutes of **camp** time (§9.2's quirk: the clock only runs in
   camp), `poison_damage` lapses → `AffectPoisonDamage` **re-plants it for
   another ten minutes** and takes one hit point off anybody above 1. The tick
   alone can never kill.
4. When `slow_poison` finally lapses → `AffectSlowPoison` finds `poisoned` still
   on the chain → **`KillPlayer("dies from poison")`**.
5. **Neutralize Poison** (spell or temple) removes all three under
   `gbl.cureSpell`.

★ `cureSpell` does **not** suppress the handler — it suppresses the handler's
*re-plant* (`ovr013.addAffect` returns `false` and adds nothing, `:25-36`). That
one indirection is the whole difference between a cure and a slow, expensive way
to poison somebody again.

★ **The removal order inside Neutralize Poison is load-bearing.** `poisoned`
leaves the chain **before** `slow_poison` (`ovr023.cs:2259-2261`,
`ovr005.cs:255-257`). Reverse those two lines and the cure kills the patient,
because `AffectSlowPoison` would still find the poison. There is a test named
exactly that.

**Petrification** needs no clock: `KillPlayer`'s guard means a `stoned` member is
never killed again (`ovr024.cs:39-42`), so the party carries the statue — not
dead, not walking, `in_combat == false`, dark red in the roster panel — until a
temple pays 2000 gp for Stone to Flesh. **Nothing in the game restores `gone`**:
Raise Dead's gate is `dead || animated`.

### 10.6 The wipe check, aligned

`tick_combat` keyed the game over off the fight's own
`CountCombatTeamMembers` verdict. The original keys it off
`CleanupPlayersStateAfterCombat`'s scan over the roster (`ovr006.cs:216-232`),
and three cases separate them — all real:

- ★ **a party that FLED is `running`** — in the liveness set, so a rout is not a
  game over. This was a live bug: fleeing raised the death screen.
- **a surviving joined NPC does not save the party** (`control_morale <
  Control.NPC_Base` is part of the falsifying predicate).
- ★ **a petrified party is a wipe** — `stoned` is not in the set. The case D-RC6
  named, and the one §10.2's decode fix had to unblock.

`battleWon` comes with it, and it is **not** "the monsters are dead": it is "at
least one member came out `okey` or `animated`", and it is what gates the
experience award (`:249-253`). So does the scan's own nineteen-affect strip
(`affects_array`, `:147-167`) — a **different** table from
`RemoveCombatAffects`': `charm_person`, `confuse`, `fumbling`, `fear` and
`spiritual_hammer` are on this one only.

### 10.7 The save break

`SAVE_FORMAT_VERSION` **6 → 7**, one golden recomputed. `EngineState` gained
`enter_temple`/`enter_shop`. They are **persisted** rather than
`#[serde(skip)]`ped (the shape `pending_combat` takes) because the original's own
`SaveGame` writes `area2_ptr` whole (`ovr017.cs:1109-1156`), flags included.
`VmPhase::Temple` and the three `HealthStatus` rungs are both append-only, so
neither moved an encoding on its own.

### 10.8 Acceptance

1. **The writeback**: `a_wound_survives_the_fight_and_reaches_the_roster` drives
   a fixture fight to `FinalBeats`, snapshots the fight's own view of the party,
   finishes, and asserts the roster matches hit point for hit point — then
   round-trips it through `.rsav`. `the_dead_stay_dead_on_the_roster` does the
   wiped-party case.
2. **The temple at a shipped site**:
   `the_enter_temple_flag_opens_the_temple_from_a_shipped_combat_opcode` drives
   `ECL2#1`'s four instructions byte for byte through the real VM;
   `a_petrified_member_walks_out_of_the_temple` buys Stone to Flesh with them.
3. **The poison clock**: `the_poison_clock_ticks_and_then_kills`,
   `neutralize_poison_saves_the_patient_because_of_the_order` and
   `curing_in_the_wrong_order_would_kill_the_patient`.
4. **Frames** (`slice6_a_visit_to_the_temple`, real data + the bundled slot-A
   party): six dumped and eyeballed — the temple menu with MATHEW's name in
   `displayPlayerName`'s removed colour, the ten services with their prices, the
   wrapped price line, "MATHEW IS CURED.", the priest noticing the leftover pool,
   and the rebuilt exploration screen with the script's own post-`COMBAT` PRINT
   running. MATHEW: status `okey`, 1 hit point, Constitution 17 → 16, maximum hit
   points 49 → 44 — a paladin 5 losing exactly `+1/level`.

### 10.9 Residuals, named

- **`CityShop` is still the reported-but-unwired arm.** The dispatch names it and
  the transcript says so; nine shipped sites wait on the shop slice.
- **The temple's View and Appraise words** report rather than act (`viewPlayer`
  and `appraiseGemsJewels` are their own screens; the latter is the only
  draw-bearing thing in `ovr022` the award path deliberately avoids).
- **`Take` on the temple's menu** reports rather than acting: `TakePoolMoney`
  (`ovr022.cs:350-400`) is a sub-screen of its own — a coin-type
  `sl_select_item` plus an `AskNumberValue` prompt per denomination — and
  aliasing it onto Share would move different coins.
- **The stoned state has no script route in yet.** `alter_character`'s
  `switch_var == 0x100` arm (`ovr008.cs:622-628`: `set_value >= 0x80` clears
  `in_combat`, `0x87` sets `Status.stoned`) is the cell a script writes; the
  acceptance test stages the state on the record directly. The medusa's own gaze
  is a monster special attack and arrives with the bestiary work.
- **`CheckAffectsEffect(Death)`** is not run by the out-of-combat `KillPlayer`:
  its only non-trivial rows are monster handlers (`troll_fire_or_acid`'s 3d6 and
  friends), none of which a party member carries.
- **The wiped-party arm of `CleanupPlayersStateAfterCombat`** frees every team
  member and sets `party_size = 0` (`ovr006.cs:326-346`). Ours leaves the roster
  intact and lets the `GameOver` flow reload a save, which is D-RC0's shape.
