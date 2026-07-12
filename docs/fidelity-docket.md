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

- **Status:** open
- **Question:** In CotAB's AD&D 1e-derived combat, does rolling a natural 20
  on the attack die always hit regardless of AC/THAC0, and does a natural 1
  always miss regardless of AC/THAC0? PLAN.md §1 notes the brief and
  Jzatopa's notes disagree — at least one is wrong.
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

- **Status:** narrowed
- **Question:** Does VM address `0x7F3F` actually read back DIVIDE's (0x06)
  remainder through the ordinary Party window, as the struct-offset
  arithmetic (`Area2.field_800_Get` mapping `field_67E`) implies?
- **Evidence so far:** Derived from static arithmetic only, not a traced
  live example (opcode-classification.md item 2). Census could not confirm
  it either way (`cotab-v1.3.md` §6: "unconfirmable from this census by
  construction" — hard-coded/computed addresses baked into a handler aren't
  visible as decoded operands).
- **Settled by:** H2 — a conformance test once DIVIDE (0x06) is implemented
  in `EclMachine` (not yet, per `docs/census/cotab-v1.3.md` §8's top-25 list
  — DIVIDE ranks below the top 25 at 4 uses) that writes a division with a
  nonzero remainder and reads back `0x7F3F` through a Party-window mock.

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

## 4. How new entries get added

Any session that surfaces a behavioral hypothesis not derivable purely from
static code reading — a coab claim that needs oracle confirmation, a
brief/Jzatopa disagreement, a census-observed pattern whose *cause* is
unclear — adds an entry here with the same shape: id, question, status,
evidence, settling rung. Update existing entries in place as evidence
accumulates; move `open` → `narrowed` → `resolved`/`deferred` rather than
duplicating entries.
