# Fidelity Docket

> Created in M1 per PLAN.md §1 and `docs/README.md`. This is the running list
> of behavioral hypotheses about the original game(s) that Restrike must
> eventually settle against an oracle (H1-H5, PLAN.md §3) before the engine
> can claim fidelity on the corresponding behavior. Every entry gets an id,
> a question, a status, the evidence gathered so far, and which harness rung
> will settle it. Timeboxed per PLAN.md §7: an item that blows its timebox
> twice gets documented and deferred, not endlessly re-investigated.
>
> This docket is *not* a duplicate of the "docket candidate" lists scattered
> through `docs/design/vm-scriptmemory.md` and
> `docs/design/opcode-classification.md` — those are implementation-detail
> hazards discovered while building the VM. This docket cross-references them
> (§3 below) but its primary content is *game-behavior* hypotheses: things a
> player would notice, not things only a VM implementer would notice.

## Status legend

- **open** — no oracle evidence yet; a hypothesis only.
- **narrowed** — some evidence gathered (reading two sources that disagree,
  a partial census result, etc.) but not yet settled against a running oracle.
- **resolved** — settled with cited evidence; the resolution and its source
  are recorded in the entry, not just "done."
- **deferred** — timeboxed out per PLAN §7; documented divergence, revisit
  post-M6 (or per its own note).

---

## 1. Combat/rules hypotheses (PLAN.md §1 seed list)

### FD-1: Does a natural 20 auto-hit / natural 1 auto-miss?

- **Status:** narrowed (coab side settled 2026-07-13, during the rules-pack
  design pass's adversarial round)
- **Question:** In CotAB's AD&D 1e-derived combat, does rolling a natural 20
  on the attack die always hit regardless of AC/THAC0, and does a natural 1
  always miss regardless of AC/THAC0? PLAN.md §1 notes the brief and
  Jzatopa's notes disagree — at least one is wrong.
- **coab evidence:** both attack paths (`CanHitTarget` ovr024.cs:487–512,
  `PC_CanHitTarget` :515–545) treat natural 1 as an automatic miss and
  promote a natural 20 to a roll of 100 (guaranteeing the comparison) —
  i.e. BOTH auto-rules exist in the engine. Jzatopa's contrary note is
  presumptively wrong for CotAB. H4 (M4) confirms against oracle traces.
- **Evidence so far:** None gathered yet from this project's own reading;
  the disagreement is inherited from the brief vs. Jzatopa's corpus (treat
  the latter as unverified candidate data per PLAN.md D11/§6 rule 4).
- **Settled by:** H4 (combat trace equality, M4) — read coab's
  `RollSavingThrow`/`CanHitTarget`/attack-roll code directly (already
  partially read for `opcode-classification.md`'s DAMAGE row — `CanHitTarget`
  is `ovr024.cs:487`, not yet read for this specific question) and confirm
  against instrumented-oracle traces for edge-roll cases.

### FD-2: Exact initiative formula

- **Status:** open
- **Question:** What determines turn order each combat round — a single
  d10 per side, per-combatant rolls, DEX modifiers, weapon speed factors,
  spell casting-time penalties?
- **Evidence so far:** None gathered yet. `MainCombatLoop` (`ovr009.cs:22`)
  was named but explicitly not traced during M1 step 0
  (opcode-classification.md docket item 10 — "COMBAT's deep call chain was
  not traced... out of scope for M1 step 0").
- **Settled by:** H4 (M4) — read `MainCombatLoop` in full when M4 opens
  combat work; cross-check against oracle traces for N seeds.

### FD-3: Attacks-per-round schedule

- **Status:** open
- **Question:** How does the engine grant multiple attacks per round (high-
  level fighters, specialization, monster multi-attacks, weapon speed)?
- **Evidence so far:** None gathered yet; same `MainCombatLoop` scope note
  as FD-2 applies.
- **Settled by:** H4 (M4).

### FD-4: Sleep/held auto-kill rules

- **Status:** open
- **Question:** Does a coup-de-grace against a sleeping or held target
  auto-hit, auto-crit, or auto-kill? What conditions gate it (melee only?
  any weapon? specific spell interactions?)
- **Evidence so far:** None gathered yet. Status effects are explicitly M4
  tier-1 scope (PLAN.md M4 bullet list: "Status effects tier 1 (sleep, held,
  poison, unconscious/dying/dead)").
- **Settled by:** H4 (M4) — read the relevant `CMD_Damage`/status-effect
  interaction code (`DAMAGE`'s row in opcode-classification.md already
  touches `sub_32200`/`damage_player`, `ovr008.cs:1401`, but the
  sleep/held-specific auto-kill branch, if any, was not traced this session).

### FD-5: Treasure-table behavior

- **Status:** narrowed
- **Question:** How does TREASURE (0x27) select and roll items — is it a
  flat random table, tiered by dungeon level/encounter, or fully
  script-authored per instance?
- **Evidence so far:** `opcode-classification.md`'s TREASURE (0x27) row
  documents the *opcode's* two paths precisely: `load_decode_dax(...)`
  (file-table path, `block_id<0x80`, `seg042.cs:115`) vs. `create_item`
  (random-roll path, `block_id 0x80-0xFE`, `ovr022.cs:443`, with `roll_dice`
  item-type-table rolls). What's still open is the *content* question: which
  path shipped scripts actually use, and what the item-type tables contain
  (uncopyrightable facts once extracted, per PLAN.md D6, but not yet
  extracted). Census note: TREASURE appears only 4 times across all 25
  CotAB blocks (`docs/census/cotab-v1.3.md` §8) — low volume, cheap to fully
  enumerate by hand later.
- **Settled by:** H1/H2 local-only content read of the 4 real TREASURE call
  sites (M5-adjacent, when item/spell tables are built in `gbx-rules`).

### FD-6: Which titles actually use TLB

- **Status:** open
- **Question:** Of the Gold Box catalog, which titles use the later
  TLB/MicroMagic engine generation vs. the DAX-era engine CotAB/CTD/MC use?
  PLAN.md M9 already assumes DQK/FRUA are TLB-era and "a separate
  compatibility effort," but the exact title boundary isn't pinned down.
- **Evidence so far:** None gathered directly by this project; PLAN.md §1's
  seed note and M9's framing are the only sources so far, both are
  project-authored expectations, not verified facts.
- **Settled by:** Deferred — M9 (catalog expansion), when TLB-era titles are
  actually approached. No harness rung needed before then; this is a
  scoping question, not a behavioral one this engine's fidelity depends on
  before M9.

---

## 2. Open items carried from `docs/design/vm-scriptmemory.md` §5

These are VM/engine-architecture hypotheses (not player-visible behavior
directly, but they gate fidelity of the execution model). Full detail lives
in the source document; summarized here with cross-references so this docket
is the one place that shows the *complete* open-hypothesis picture.

### FD-7: Nested `ChainTo` in shipped content

- **Status:** open
- **Question:** Does any shipped CotAB script actually trigger a NEWECL
  (block switch) from *inside* a nested run — e.g. a PROGRAM-9 camp
  pre-check/interrupted-camp script that itself chains to a different block
  mid-flow? vm-scriptmemory.md §1 documents the *mechanism* precisely (block
  swap mid-flow, interrupted engine flow completes against the new block,
  chaining happens at walk-loop unwind) but not whether real data exercises
  it.
- **Evidence so far:** Not census-checked yet — the M1 census
  (`docs/census/cotab-v1.3.md`) counted NEWECL occurrences (9, rank 20-ish
  per §8's tail list) but did not cross-reference which of those sit inside
  a `PreCampCheckAddr`/`CampInterruptedAddr` vector specifically vs. a
  regular walk-loop vector.
- **Settled by:** H2 (a targeted census query: for each block, is a NEWECL
  reachable from vector index 3 or 4?) plus, if any are found, H4/H5 oracle
  comparison of the resulting composite flow.
- **Cross-reference:** `docs/design/vm-scriptmemory.md` §5 item 4.

### FD-8: EclDump goldens

- **Status:** open
- **Question:** Does this project's disassembler produce byte-identical
  output to EclDump.exe (the reference Windows disassembler) on the same
  real blocks?
- **Evidence so far:** Deferred by design — `docs/census/cotab-v1.3.md`'s
  header states H1 status explicitly: "DaxDump.exe/EclDump.exe golden
  comparisons are Windows binaries not runnable in this environment yet —
  deferred to the oracle-rig milestone." Internal-consistency validation
  (zero decode desyncs across 3,582 real instructions, `cotab-v1.3.md` §7)
  is a strong positive signal but not the same as an external golden match.
- **Settled by:** H1, once the oracle-rig milestone (PLAN.md M0 checklist:
  UTM Windows VM or CrossOver) makes EclDump.exe runnable.
- **Cross-reference:** `docs/design/vm-scriptmemory.md` §5 item 6.

---

## 3. Unresolved candidates from `docs/design/opcode-classification.md` §5

Carried over verbatim in spirit, condensed here; full citations live in the
source document (§5 "New docket candidates").

### FD-9: DIVIDE remainder addressability at `0x7F3F`

- **Status:** resolved (2026-07-13, M2 step 8)
- **Question:** Does VM address `0x7F3F` actually read back DIVIDE's (0x06)
  remainder through the ordinary Party window, as the struct-offset
  arithmetic (`Area2.field_800_Get` mapping `field_67E`) implies?
- **Evidence so far:** Derived from static arithmetic only, not a traced
  live example (opcode-classification.md item 2). Census could not confirm
  it either way (`cotab-v1.3.md` §6: "unconfirmable from this census by
  construction" — hard-coded/computed addresses baked into a handler aren't
  visible as decoded operands).
- **Settled by:** DIVIDE (0x06) is now implemented in `EclMachine`
  (`crates/gbx-vm/src/machine.rs`'s `op_divide`), writing the remainder
  through the ordinary `mem_write` facade at `0x7F3F` alongside the
  quotient at the operand-3 destination — the H2 conformance suite includes
  `divide_then_gettable_via_0x7f3f_mirrors_the_shipped_pattern`
  (`crates/gbx-vm/src/conformance.rs`), which replicates the real
  `ECL2.DAX` block 1 instruction shapes and addresses (`0x7F7B`/`0x7F80`/
  `0x7F3F`) exactly, substituting only GETTABLE's base (the real `0x9DB8`
  falls inside the VM-intercepted ECL window and isn't `ScriptMemory`-mockable
  — see the test's own doc comment). Division-by-zero also settled: coab's
  `val_a / val_b` throws an uncaught C# exception with no `try`/`catch`
  anywhere up the `RunEclVm` call chain, so the original crashes; modeled as
  `VmError::DivisionByZero`, exercised by
  `divide_by_zero_is_a_defined_error_not_a_panic`.

### FD-10: COMPARE AND / CHECKPARTY string-mode operand hazard

- **Status:** resolved (as a machine-level hazard; census-quiet on real
  data)
- **Question:** Do shipped CotAB scripts ever feed a string-mode operand to
  COMPARE AND (0x14) or CHECKPARTY (0x1E), which would throw in the
  original (`Opperation.GetCmdValue()`'s `highSet` guard)?
- **Evidence so far:** The hazard itself is confirmed as a genuine original-
  engine behavior (opcode-classification.md item 5, exact citations to
  `Classes/Opperation.cs:98-130`). `EclMachine::op_compare_and` implements
  the guard as `VmError::StringOperandTypeMismatch`
  (`crates/gbx-vm/src/machine.rs`), with a conformance test proving it's a
  defined error, not a panic
  (`compare_and_string_operand_is_a_defined_error_not_a_panic`,
  `crates/gbx-vm/src/conformance.rs`). Whether shipped CotAB v1.3 data ever
  actually hits it: CHECKPARTY never appears in the census at all (0 of
  3,582 reached instructions, `cotab-v1.3.md` §8's "never appear" list);
  COMPARE AND appears 19 times (tail list beyond the top 25) but the census
  does not yet classify operand modes per-opcode beyond the aggregate
  0x01-vs-0x03 count (§6), so whether any of those 19 uses is string-moded
  is still open.
- **Settled by:** A future census refinement (per-opcode operand-mode
  breakdown for COMPARE AND specifically) would fully resolve the "does real
  data hit this" half; the machine-level behavior is already resolved and
  tested.

### FD-11: CHECKPARTY partial dispatch

- **Status:** open
- **Question:** Is CHECKPARTY's query-code dispatch (values outside
  `{0x8001, 0xA5-0xAC, 0x9F}` silently no-op) intentional original design,
  or an unhandled-case bug in the original engine?
- **Evidence so far:** Behavior confirmed exactly as coded
  (opcode-classification.md item 7); CHECKPARTY is unimplemented in
  `EclMachine` (never observed in the CotAB census, `cotab-v1.3.md` §8) so
  there's no conformance test yet either way.
- **Settled by:** Not player-visible unless a script relies on the silent
  no-op for correctness — low priority; revisit if/when CHECKPARTY is
  implemented for a later title's census (M7/M9) where it's actually used.

### FD-12: PROTECTION's dead operand

- **Status:** open
- **Question:** Is PROTECTION's (0x3C) decoded-but-never-read operand
  vestigial in every CotAB build, or does some other Gold Box title's
  dialect actually read it (making it a real per-dialect parameter rather
  than dead code)?
- **Evidence so far:** Confirmed dead in coab's CotAB transliteration
  (opcode-classification.md item 13, `ovr003.cs:1997`). Not implemented in
  `EclMachine` (never observed in the census, `cotab-v1.3.md` §8's
  "never appear" list — copy-protection scripts are presumably outside the
  6 shipped `ECLn.DAX` area files, e.g. a startup-specific block).
- **Settled by:** Deferred — revisit only if a Buck Rogers or other-title
  census (M7/M9) shows the operand actually consumed.

### FD-13: SURPRISE's `0x2CB` hard-coded cell

- **Status:** narrowed
- **Question:** Should VM address `0x2CB` (SURPRISE's hard-coded result
  cell, bypassing normal operand addressing) be added to the named-global
  address map as a first-class documented cell?
- **Evidence so far:** The write itself is confirmed
  (opcode-classification.md item 8, `ovr003.cs:967`). The census's
  `surprise_cell_hits` check (`frontends/cli/src/census.rs`) found zero
  operand-decoded references to `0x2CB` in CotAB v1.3 — expected, since
  SURPRISE (0x23) itself never appears in this data set at all
  (`cotab-v1.3.md` §6/§8), and the census can't see hard-coded writes baked
  into a handler by construction.
- **Settled by:** Should still be added to the engine-side named-global map
  (`gbx-engine`, when it implements `ScriptMemory`) proactively, since the
  behavior is confirmed even without a live example — low-risk, cheap
  insurance. Not blocking; revisit if a future title's data exercises
  SURPRISE directly.

### FD-14: CALL unknown-key silent no-op

- **Status:** resolved
- **Question:** Is CALL's (0x2D) 7-case hidden dispatch table (keyed on
  `operand.Word - 0x7FFF`) fully enumerated, and do shipped scripts ever
  emit a key outside those 7?
- **Evidence so far:** Fully resolved.
  `opcode-classification.md` §3 enumerates all 7 cases with proposed
  `EngineServices` signatures (implemented in `crates/gbx-vm/src/host.rs` /
  `machine.rs`'s `op_call`). The census (`cotab-v1.3.md` §5) found exactly
  4 of the 7 keys actually used in CotAB v1.3 (`0xAE11`, `0xE804`, `0x3201`,
  `0x401F` — 52 total CALL instructions, zero unrecognized keys); the other
  3 (`1`, `2` duel setup, `0x4019` non-dungeon wall query) are unobserved in
  this corpus but implemented and conformance-tested anyway
  (`crates/gbx-vm/src/conformance.rs`: `call_case_1_and_2_setup_duel`,
  `call_case_0x4019_queries_wall_type_only_outside_dungeon`,
  `call_unrecognized_key_is_a_silent_noop`).
- **Settled by:** Already settled — kept here as the docket's worked example
  of what "resolved" looks like, per the task brief's instruction to show
  settled items, not just open questions.

---

### FD-16: GEO2.DAX block 1's columns 8-15 don't match any printed map

- **Status:** resolved (2026-07-12 audit)
- **Question:** Block 1 of `GEO2.DAX` (Tilverton City, per Gold Box
  Explorer's per-game GEO-id table — confirmed to be the correct block by
  a real wall-topology match, see evidence) decodes to a 16x16 square grid,
  but *Cluebook.pdf*'s printed Tilverton City map is only 8 columns wide
  (labeled `0`-`7`). Columns 0-7 of the decoded grid match the printed map
  closely (see evidence); columns 8-15 contain real, structured wall/door
  data — not blank padding or noise — that doesn't correspond to anything
  printed for Tilverton City. What is it?
- **Evidence so far:** Two independent structural matches confirm columns
  0-7 are genuinely Tilverton City: (1) a solid wall at the column 4/5
  boundary in row 0, matching the printed map's boundary between room "9"
  and room "8"; (2) a dense cluster of door markers at rows 11-14, columns
  5-7, matching the printed map's Tilverton Inn room cluster (locations
  "1"/"10", famous for many interior doors) in the same relative position.
  Columns 8-15 were hypothesized to be the Thieves' Guild sub-map (printed
  separately on the facing page, also 8 columns x 16 rows — plausible if
  the two locations are packed side-by-side in one GEO block, and Gold Box
  Explorer's table names block 1 "Tilverton City, Thieves' Guild" as one
  combined entry), but the wall *density* doesn't match: the printed
  Thieves' Guild map is far denser (many small rooms/doors) than what
  `restrike map`'s columns 8-15 show. Not disproven, just not confirmed —
  a hasty visual density comparison, not a cell-by-cell check.
- **Resolution:** columns 8-15 **are the Thieves' Guild**, confirmed by a
  discriminating topology landmark rather than density: the printed Guild
  map (*Cluebook.pdf* p.8, 8x16) has exactly two exits — "Exits to
  Tilverton Sewers" — on its bottom edge at its columns ~2 and ~6. Our
  rendering of GEO block 1 has exactly two openings in the entire bottom
  border, at absolute columns **10 and 14** = 2+8 and 6+8. Together with
  both printed maps being 8x16 and Gold Box Explorer naming block 1
  "Tilverton City, Thieves' Guild" as one entry, the side-by-side packing
  is confirmed. (The earlier density mismatch was a red herring: printed-map
  visual density includes location labels and annotations; unique boundary
  topology is the right fingerprint for map identification.) Consequence
  worth keeping: one GEO block can hold multiple logical locations packed
  side by side — automap and event work (M2/M8) must not assume
  block == single map.

### FD-17: Keyboard type-ahead — does the original drain the buffer to the newest key?

- **Status:** RESOLVED (2026-07-14, live DOSBox mash test by Bryan —
  dosbox-staging 0.82.2, 3000 cycles)
- **Question:** coab's `GetInputKey` discards the entire keyboard buffer
  after reading any nonzero key, keeping only the newest
  (`~/src/goldbox-refs/coab/engine/seg043.cs:55-62`) — so mashing forward
  five times during a slow redraw commits **one** step, and type-ahead is
  largely lost. Is this the original binary's behavior, or a coab
  transliteration/anti-key-repeat artifact?
- **Resolution: the original BUFFERS type-ahead — coab's drain-to-newest is
  a transliteration/anti-key-repeat artifact, not original behavior.**
  Bryan mashed forward repeatedly in the real game's 3D view and the party
  committed multiple queued steps, not one. Fifth confirmed
  coab-vs-original divergence (after monk hit dice, thief base_chance, the
  spellCastCount stride, and the Str00 clamp).
- **Consequence for D-UI1:** the engine's input queue keeps FIFO type-ahead
  for movement (per the design doc, the queue read is a single function —
  flip it off coab's drain semantics). This coexists with the *documented*
  pagination-release queue-clear (`seg041.cs:211`, wired in the M2 step-5
  stale-Enter fix): that clear is a specific, sourced behavior at page
  release, not a general drain policy.
- **Cross-reference:** `docs/design/renderer-ui-shell.md` §1.5, §1.11
  item 9 (found in the 2026-07-12 M2 adversarial review round).

### FD-18: Do Up/Down arrows move list-menu highlights?

- **Status:** RESOLVED (2026-07-14, live DOSBox check by Bryan during CotAB
  character creation — the race-selection list, `sl_select_item` via
  `ovr018.cs:368`)
- **Question:** coab's `sl_select_item` special-key switch handles only the
  Home/End/PgUp/PgDn ctrl-codes (`'G'/'O'/'I'/'Q'`) and ignores Up/Down
  arrows entirely (`~/src/goldbox-refs/coab/engine/ovr027.cs:617-640`),
  which contradicts common memory of navigating Gold Box menus with the
  arrow keys. Which is right?
- **Resolution: coab is right — arrow keys do NOT move the highlight.**
  Bryan confirmed on the real game: arrows did nothing on the race list;
  the numpad **`1` key (End, `0x4F`→`'O'`) moves the highlight DOWN**, and
  by symmetry **`7` (Home, `0x47`→`'G'`) moves UP** —
  `menu_scroll_in_page(true/false)`, `ovr027.cs:620-627`. The ctrl-code
  letter is just the ASCII of the extended scancode byte (Home `0x47`=`G`,
  End `0x4F`=`O`, Up `0x48`=`H`, Down `0x50`=`P`); Up/Down (`H`/`P`) are
  absent from the special-key switch, so they are ignored. (Note the
  collision: the *letter* `P` typed non-special pages down, but the Down
  *arrow* — special `P` — does nothing.)
- **Consequence for D-UI6:** the desktop/web key map must route Home/End
  (and numpad 7/1) to list-highlight up/down, and must NOT bind Up/Down
  arrows to list movement (faithful default). A QoL toggle could add arrow
  support later per D4, default-off.
- **Cross-reference:** `docs/design/renderer-ui-shell.md` §1.5, §1.11
  item 10 (both can drop the "contradicts common memory / verify" caveat).

### FD-19: The (7,12)-North door is a real area transition, not a plain locked door

- **Status:** narrowed (mechanism understood; cross-area GEO-block swapping
  deferred to M3+)
- **Question:** Tilverton's per-step script (`ECL2.DAX` block 1 vector 1,
  the DIVIDE-unblocked logic FD-9 fixed) prints a short in-fiction refusal
  when the party approaches (7,12)'s North edge from most positions — the
  gist (not a verbatim quote, D10): this is the wrong entrance, use the
  other one, and the party is turned away — and the M2 step 4 demo
  (`gbx-engine/src/demo.rs::walk_tilverton_and_bash_a_real_door`) forces a
  bash success there anyway (a party-predicate stub) — what actually happens
  when the bash mechanically succeeds and the party steps through?
- **Evidence so far:** the engine's own service-call log shows
  `Load3dMap { block_id: 1 }` immediately after the forced bash, with zero
  halts — the edge is scripted to load a *different resident area* (an
  interior; the refusal text's framing matches: this edge is the wrong/side
  entrance), not to just flip a GEO door bit. M2's engine deliberately keeps
  one fixed resident GEO/ECL block for the whole session
  (`gbx-engine/src/engine.rs`'s doc comment: block *selection* logic is
  step 5+/M3+ scope) — `load_3d_map`/`load_walldef` update wallset assets but
  never swap the resident `GeoBlock` movement/wall-query source, so the
  party's position and the still-Tilverton wall/door geometry go out of
  sync. Observed result: position lands at `(0, 0)` (the raw-store default,
  not a real new-area spawn point) — a consequence of the gap, not a new
  bug. `demo.rs`'s test was updated to assert this real (if imperfect)
  outcome rather than the pre-DIVIDE-fix assumption it originally shipped
  with (bashing through to `(7,11)`), which turned out to be an artifact of
  vector 1 silently halting before this session (FD-9's finding).
- **Settled by:** M3+/whenever cross-area GEO/ECL-block selection is wired
  (the natural place to also decide what a mid-session `Load3dMap` should do
  to party position — likely reading the new area's own boot-vector spawn,
  the same way `INITIAL_ECL_BLOCK`'s spawn is read today). Not blocking M2's
  exit gate: the circuit trace (`fixtures/tilverton-circuit.jsonl`) routes
  around this door entirely, using a different, transition-free path to its
  event squares.

### FD-20: Turn-undead types 11-12 — does any shipped monster actually use them?

- **Status:** narrowed (image extent settled 2026-07-14; real-monster-data
  question still open)
- **Question:** `turns_undead` (`ovr014.cs:642`) indexes `unk_16679` by
  `target.field_E9` (copied straight from monster data's `field_76`,
  `ovr017.cs:286`, an unclamped byte read) with no visible upper-bound
  check in the code read this session. The rules-pack design doc originally
  hypothesized the image-stored table would need to cover types up to 12.
  Does any monster actually shipped in CotAB's data carry `field_76` >= 11?
- **Evidence so far:** the *table* question is settled: the decompressed
  `START.EXE` image (v1.3) unambiguously stores exactly 11 rows (undead
  types 0-10) at offset `0xaf4a` — type 11's would-be row is ASCII menu
  text ("!Area Cast..."), not further table data, confirmed by direct
  byte inspection. `packs/adnd1/progression.toml`'s `turn_undead` table
  and `gbx-rules::adnd1::progression::turn_undead_entry` both encode this
  (the accessor returns `None` for `undead_type >= 11`, never reading past
  the confirmed extent). What's still unknown: whether the monster data
  files (`MON*`, Group C — never pack material, not parsed this session)
  contain any `field_76` value of 11 or higher. If none do, this is a
  closed non-issue; if one does, `turns_undead` would read into the
  string table's bytes as if they were turn-difficulty values — a real
  behavioral bug in the *original*, or evidence this session's index-
  formula reading has a gap (e.g. an unseen clamp elsewhere).
- **Settled by:** M4 (monster data loading lands) — grep all shipped
  `MON*` records' `field_76` byte for its observed value range.

### FD-21: `thief_skill_base_chance`'s levels 6-11 diverge from a naive reading of coab's declaration

- **Status:** open
- **Question:** `base_chance`/`unk_1A1D0` (`ovr026.cs:465-477`) declares 13
  rows (thief levels 0-12, row 0 dead) x 9 columns (skill 0-8, column 0
  dead). Dropping the dead row/column, thief levels 1-4 and 12 byte-match
  the image exactly at offset `0xeaa9`. Levels 6-11 do not: the image's
  flattened byte stream runs exactly one position ahead of a naive
  per-row reading starting partway through level 6, and self-corrects
  precisely by level 12. What causes the one-byte local divergence?
- **Evidence so far:** re-verified the coab source transcription
  character-by-character twice (ruling out a transcription slip this
  session); the image bytes are unambiguous and were used verbatim in
  `packs/adnd1/thief_skills.toml` (byte-exact anchor, `restrike verify`
  reports `Verified`). No working theory yet for *why* — candidates not
  investigated this session: a genuine second coab transcription error
  (like `max_class_hit_dice`'s monk entry, FD-adjacent but not yet its
  own confirmed case), a column/row semantic this session's reading of
  `reclac_thief_skills` didn't fully capture, or an intentional original-
  engine quirk.
- **Settled by:** a future session with `restrike extract-table` and more
  forensic time — try reconstructing the exact VM-level access pattern
  (disassemble `sub_6AAEA`'s real addressing rather than trusting coab's
  transliteration) rather than pattern-matching bytes.

### FD-22: `thief_skill_race_adj` (`unk_1A230`) has no confirmed image location

- **Status:** open
- **Question:** Where, if anywhere, does the real image store the race
  adjustment table `reclac_thief_skills` reads via `unk_1A230[race, skill]`
  (`ovr026.cs:426-439`, `:530`)?
- **Evidence so far:** actively disproven at coab's declared shape (13
  rows x 9 columns = 117 bytes), not just unlocated: the confirmed gap
  between `thief_skill_base_chance`'s anchor end (`0xeb09`) and
  `thief_skill_dex_adj`'s independently byte-verified start (`0xeb13`) is
  only 10 bytes. The first 8 of those 10 bytes do match coab's row 1
  (dwarf) minus a presumed dead column exactly, and the next 2 bytes
  match the start of row 2 (elf) before being cut off — i.e. coab's own
  transcribed extent for this array is provably too long for wherever it
  actually lives. Ships `coab-only` in `packs/adnd1/thief_skills.toml`,
  transcribed verbatim from coab with no guessed truncation.
- **Settled by:** a future session — search elsewhere in the image (this
  session only checked the gap between its two confirmed neighbors, on
  the assumption of proximity that turned out false); `restrike
  extract-table --table thief_skill_race_adj` is ready to confirm a
  candidate offset once one is found.

## 4. Open items carried from `docs/design/save-formats.md` §5

Full detail lives in the source document; summarized here so this docket
stays the one place showing the complete open-hypothesis picture.

### FD-23: The three pinned original-save record cells

- **Status:** narrowed → item 5 PINNED, item 2 corroborated, item 1 still open
  (2026-07-14 Fable audit, against GOG's bundled save)
- **BREAKTHROUGH (2026-07-14):** GOG's Collection Two ships a **complete
  bundled save** at `GBX_DATA_DIR/SAVE/SAVGAMA.DAT` (+ `CHRDATA{1..6}.SAV`/
  `.FX`), present since install — 13149 + 422-byte files, exactly the design
  doc's predicted sizes. Our real importer (`load_from_lookup` +
  `import_original`) parsed it clean on the first try: a 6-char party
  (MATHEW/MARK/TRAVIS/LEDERA/SHARA/PHILIPPE) at pos (7,13) area 2 (Tilverton).
  - **Item 5 (Str00 range) PINNED to `0..=100` — and display-confirmed:**
    MATHEW decodes to exceptional strength **100** (18/00); coab's
    `Math.Min(_,25)` clamp would read 25. Bryan then loaded slot A in the
    real game and the character sheet displays **STR 18(00)** — the exact
    field-by-field-against-DOSBox's-display criterion this entry's
    "settled by" clause demanded. Same screen also display-matched our
    decode of LEVEL 5 / EXP 25000 / HP 49 / PALADIN / LAWFUL GOOD
    (alignment byte 0 = LG). Fully closed.
  - **Item 2 (spell stride) corroborated:** all 6 records decode fully and
    sanely at stride 5; combined with the GBC-editor cross-check, effectively
    settled (a memorized-spell golden would fully close it).
  - **Item 1 (stat byte order) STILL OPEN:** the bundled party has no
    stat-*drained* character (every stat shows current==original), so `cur`
    vs `full` ordering is not disambiguated by this save. Needs a save with a
    drained stat (current≠max) — a self-made DOSBox save, or an in-game
    stat-drain, is the remaining pin.
- **Also found:** the GOG build reads/writes saves in a **`SAVE/`
  subdirectory** of the game dir, not the root — the local-tier import test
  and any save-locating code must look in `GBX_DATA_DIR/SAVE/` (design-doc
  §1.1 assumed the game root; correct at implementation).
- **Question:** §1.7 items 1/2/5 — is coab's `stats2` byte order (`cur`,
  `full`) or GBC-doc's (`original`, `current`) correct for the on-disk
  `CHRDAT` record? Is `spellCastCount`'s row stride really 5 (GBC-doc), not
  coab's transliterated `i*i` bug? Does exceptional-strength `Str00` really
  need `0..=100` unclamped, against coab's own buggy `Math.Min(_, 25)` read?
- **Question:** §1.7 items 1/2/5 — is coab's `stats2` byte order (`cur`,
  `full`) or GBC-doc's (`original`, `current`) correct for the on-disk
  `CHRDAT` record? Is `spellCastCount`'s row stride really 5 (GBC-doc), not
  coab's transliterated `i*i` bug? Does exceptional-strength `Str00` really
  need `0..=100` unclamped, against coab's own buggy `Math.Min(_, 25)` read?
- **Evidence so far:** all three ship at their doc-specified defaults
  (`gbx_formats::save_orig::STAT_BYTE_ORDER`/`SPELL_CAST_COUNT_STRIDE`/
  `STR_EXCEPTIONAL_RANGE`, each a single named flip-point constant, task
  deliverable 1). Item 2 (the stride) already has stronger-than-default
  evidence: Fable's M3 save-format design review cross-checked it against
  GBC's own character editor operating on real save data. Items 1 and 5
  are un-pinned code decisions, not yet checked against a real save.
- **Settled by:** D-SAVE10 tier 3 — a real DOSBox save with a stat-drained
  character (item 1) and an 18/xx-strength fighter (item 5), compared
  field-by-field against DOSBox's own display. This is also the M3 exit
  gate's blocking precondition (no real save exists under `GBX_DATA_DIR`
  yet as of the step-4 session, 2026-07-14) — see the step-4 session
  handoff for the exact DOSBox procedure.
- **Cross-reference:** `docs/design/save-formats.md` §1.7 items 1/2/5, §5
  item 2, D-SAVE6.

### FD-24: ScriptMemory window byte-offset alignment for original-save import

- **Status:** resolved (2026-07-14 Fable audit — by derivation, one
  confirming unit test still owed)
- **Resolution:** the two numbering schemes are linearly related:
  **origData byte offset = 2 × (vm_address − window_base)**, for both
  windows, via the same 16-bit-wrap idiom as the ECL window (its third
  appearance). Derivation: `vm_GetMemoryValue` calls
  `field_6A00_Get(0x6A00 + vm×2)` and the dispatch masks `& 0xFFFF` —
  `(0x6A00 + (0x4B00+n)×2) mod 0x10000 = 2n`; Party likewise
  `((0x7C00+n)×2 + 0x800) mod 0x10000 = 2n`. Confirmed against two
  independent evidence eras: `Area1.cs` case `0x18E = time_minutes_ones`
  → VM `0x4BC7` (the M1 clock cluster), and `Area2.cs` case
  `0x67e = field_67E` → VM `0x7F3F` — **the DIVIDE-remainder alias
  field-verified in M2**. Consequence: the step-4 importer's raw-blob
  packing (blob word n → VM address base+n) is the correct mapping;
  the two-path importer can be unified in a later pass. Remaining task:
  one unit test asserting a known blob cell reads back through the live
  facade (e.g. blob word 0xE6 ↔ facade read at 0x4BE6/inDungeon).
- **Question:** Do the named Area/Party-window engine fields import derives
  from `Area1.cs`/`Area2.cs`'s `[DataOffset]` byte offsets, and the *raw*
  window content import packs into restrike's own VM word-addressed
  `ScriptMemory` store, actually refer to the same real memory cells a
  resident script would read post-import? The step-4 implementation session
  found these are two different, not-provably-related numbering schemes
  (one from the save-file's own C# struct layout, one reverse-engineered
  from real ECL operand addresses) and did not reconcile them — see the
  implementation note this session added to save-formats.md §5 item 8 for
  the full reasoning.
- **Evidence so far:** none beyond the two independently-sourced numbering
  schemes agreeing on nothing checked directly against each other. Low
  practical risk noted: no M2/M3 opcode reads an unnamed Area/Table/Party
  cell that matters yet.
- **Settled by:** either reading `Area1.field_6A00_Get`/`Area2.field_800_Get`'s
  full dispatch bodies to derive the true mapping, or empirical pinning
  (D-SAVE10 tier 3): import a real save with a known script-stashed value at
  a specific unnamed cell, then observe whether the resident script reads it
  back correctly.
- **Cross-reference:** `docs/design/save-formats.md` §1.4, §5 item 8.

### FD-25: Rest does not "restore spell slots" — it commits a staged pending list

- **Status:** narrowed (coab read; game-oracle confirmation deferred to M5).
- **Question:** The M3 step-6 task framed camp/Rest as "spell-slot
  restoration through `MagicState` + `flavor.spell_slots`" — the common
  tabletop mental model of "rest → spells back to full." A coab read
  (`ovr016.cs:274` `rest_menu`, `ovr021.cs:516` `resting`, `:393`
  `rest_memorize`) shows the original does **not** work that way:
  `spellCastCount[3,5]` (`Player.cs:536`) is a **fixed per-level capacity**
  written only at creation/level-up/item-effects, *never* reset by rest.
  Memorized spells live in a two-list `SpellList` (`Classes/SpellList.cs`):
  each `SpellItem` carries a `Learning` flag; `LearntList()` (castable) vs
  `LearningList()` (pending). Magic ▸ Memorize *stages* a spell
  (`spellList.AddLearn`, `Learning=true`); casting removes it
  (`ClearSpell`); **Rest merely commits** pending → memorized
  (`spellList.MarkLearnt`, `ovr021.cs:403`, flips `Learning=false`). There
  is no "everything back to full" shortcut anywhere.
- **Impact on M3:** restrike's `party::MagicState` carries `spell_list` (the
  84-byte `spellList`) and `cast_count` (`spellCastCount`) **raw/undecoded**
  by deliberate design (party.rs: "Slot→spell interpretation is a rules
  concern"). The `SpellItem` Learning-flag encoding inside those 84 bytes is
  not decoded, and staging (Magic ▸ Memorize) is Vancian-memorization work
  the PLAN explicitly schedules for **M5** ("Vancian memorization/slots/
  scribing"). So M3's camp Rest cannot faithfully mutate spell state: with
  no staging path, the pending list is always empty and a faithful
  `MarkLearnt`-all-pending commits zero — which is *correct* for the bundled
  save (it carries no staged memorizations). M3's camp Rest therefore wires
  the menu action and documents this, deferring the real commit + the
  time/healing halves (per PLAN's "minus time effects") to M5/M4.
- **Evidence so far:** coab `ovr016.cs:274-298`, `ovr021.cs:393-413/516-606`,
  `Classes/SpellList.cs:54-86`, `Classes/Player.cs:536`. No game-oracle run
  yet.
- **Settled by:** M5's Vancian memorization work — decode the `SpellList`
  Learning-flag layout, implement Magic ▸ Memorize staging + Rest commit,
  and confirm against a DOSBox rest that a staged spell becomes castable
  only after resting.
- **Cross-reference:** `crates/gbx-engine/src/screens.rs` (camp Rest/Magic),
  PLAN.md M5.

### FD-26: Integer `Random(N)` — modulo over the TP LCG (v1's "scaled" claim refuted in review)

- **Status:** narrowed (semantics settled statically 2026-07-15 by the
  oracle-rig adversarial round; executable confirmation = D-OR4 A/B)
- **Question → answer:** The original's RNG is the Borland TP LCG
  (`RandNext`, image `0xa5a9`, cs `0x8F7:0x1639`): `state = state*0x08088405
  + 1`, state dword `DS:0x47F0`. The integer wrapper (image `0xa55a`,
  `0x8F7:0x15EA`) computes **`hi16(new_state) DIV N` and returns the
  remainder** — TP 5.x modulo, NOT TP6+'s scaled high word (the door's v1
  claim, refuted by wrapper disassembly: `div bx`). Draw consumption:
  `Random(0)` calls `RandNext` **before** the N==0 test — a draw is consumed
  and 0 returned; coab short-circuits without drawing (`seg051.cs:35-38`) —
  a one-draw desync hazard for any transcribed call site that can pass 0.
  coab's `% N` reduction *shape* was faithful; its generator and
  short-circuit are not.
- **Evidence:** adversarial re-derivation (capstone decode + 200k-state
  instruction-semantics simulation, 0 mismatches vs the LCG formula); caller
  census — 29 GAME.OVR + 5 START.EXE far calls to the wrapper, matching
  coab's 29 `seg051.Random(` sites 1:1; hash pin `[0xa55a,0xa5ee)` in
  `docs/design/oracle-rig.md` §1.
- **Settled by:** D-OR4 part A (unicorn-engine execution of the pinned
  routine vs `gbx-prng`, 10k (K,N) pairs) + part B (one live staging-hook
  session with chain-continuity checks).
- **Cross-reference:** `docs/design/oracle-rig.md` §1/D-OR1/D-OR4.

### FD-27: Seed lifecycle — answered statically: one boot-time `Randomize`, no overlay RNG copies

- **Status:** narrowed (static census complete 2026-07-15; dynamic
  single-writer confirmation = D-OR4 part B)
- **Answer:** `Randomize` (image `0xa5e1`; seeds `DS:0x47F0` from DOS wall
  clock, low word ← CX hour:min, high ← DX sec:centisec — the dword is
  DX:CX) has **exactly one call site**: GAME.OVR `0xf5f6` = coab
  `seg001.InitFirst` — boot only, never per battle. GAME.OVR contains **no
  local copy** of any RNG routine (full-body + distinctive-subsequence
  scans: zero hits); all overlay randomness far-calls the resident cluster
  (segment word `0x08F7` in all 34 far calls). Float `Random`: 4 sites =
  coab `ovr019.cs`'s `Random__Real` calls. Poke-once seed control is
  statically sound.
- **Settled by:** D-OR4 part B's chain-continuity verification (detects any
  mid-session reseed or foreign write to `DS:0x47F0` rather than assuming).
- **Cross-reference:** `docs/design/oracle-rig.md` §1/D-OR2/D-OR4.

### FD-28: Does the original's fade dither draw from the game RNG?

- **Status:** open
- **Question:** Our fade dither currently draws `roll_uniform(3)` per
  changed pixel from the one engine PRNG (`crates/gbx-engine/src/draw.rs`,
  `apply_recolor_dithered`) — a framebuffer-content-dependent draw count
  that would desync any traced window if the original's dither does NOT
  consume `DS:0x47F0` draws (coab uses a separate time-seeded RNG for it,
  `docs/design/renderer-ui-shell.md` §D-UI4 region — unfaithful either
  way). Which stream does the real binary's dither use, if any?
- **Evidence so far:** none from the binary; both references disagree with
  each other by construction.
- **Interim posture (D-OR1 draw-parity contract):** our dither moves OFF
  `gbx-prng` to a deterministic position-hash pattern (dither pixels are
  already declared non-comparable by the renderer doc), so the traced draw
  stream is dither-free regardless of the answer.
- **Settled by:** a tier-1 staging-hook trace captured across a
  fade/recolor transition — if RandNext fires per pixel, revisit; if not,
  the position-hash divergence is documented and permanent.
- **Cross-reference:** `docs/design/oracle-rig.md` D-OR1(c)/§5.

## 5. How new entries get added

Any session that surfaces a behavioral hypothesis not derivable purely from
static code reading — a coab claim that needs oracle confirmation, a
brief/Jzatopa disagreement, a census-observed pattern whose *cause* is
unclear — adds an entry here with the same shape: id, question, status,
evidence, settling rung. Update existing entries in place as evidence
accumulates; move `open` → `narrowed` → `resolved`/`deferred` rather than
duplicating entries.
