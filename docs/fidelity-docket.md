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
  `PC_CanHitTarget` :515–545) treat natural 1 as an automatic miss (the
  `attack_roll > 1` gate) and promote a natural 20 to a roll of 100
  (guaranteeing the comparison) — i.e. BOTH auto-rules exist in the engine.
  `RollSavingThrow` (:564–571) applies the same nat-1/nat-20 auto rules to
  saves. Jzatopa's contrary note is presumptively wrong for CotAB. Re-read in
  full 2026-07-16 (M4 step 5) — see `docs/design/combat-study.md` §5.2, which
  also flags two H4-relevant asymmetries: `CanHitTarget` compares with strict
  `>` while `PC_CanHitTarget` uses `>=`, and the two combine the AC/bonus terms
  differently. H4 (M4) confirms against oracle traces on edge rolls.
- **Implemented + caller-confirmed (2026-07-16, M4 combat #2):** both auto-rules
  are now transliterated in `gbx-engine`'s `combat` module
  (`can_hit_target`/`pc_can_hit_target`) with the nat-1 gate and the nat-20→100
  promotion, and the `>`/`>=` asymmetry is exercised at the equality point
  (`gt_path_and_ge_path_disagree_at_the_equality_point`). **The caller read
  resolved which path is which** (the study §5.2 "monster/generic vs PC" labels
  were imprecise): the `>=` path (`PC_CanHitTarget`) is the STANDARD weapon-attack
  path for **any** combatant — its only live caller is `AttackTarget01`
  (`ovr014.cs:821`, `sub_3F4EB`), the per-turn weapon body both PCs and monsters
  reach via the AI/menu; the `>` path (`CanHitTarget`) is the scripted
  DAMAGE-opcode / area-effect path (`CMD_Damage`, `ovr003.cs:1673`, hitting a
  random party member), not a weapon swing. Status stays *narrowed* — the coab
  read + implementation settle the mechanism; H4 curated edge-roll traces settle
  it fully.
- **Evidence so far:** None gathered yet from this project's own reading;
  the disagreement is inherited from the brief vs. Jzatopa's corpus (treat
  the latter as unverified candidate data per PLAN.md D11/§6 rule 4).
- **Settled by:** H4 (combat trace equality, M4) — read coab's
  `RollSavingThrow`/`CanHitTarget`/attack-roll code directly (already
  partially read for `opcode-classification.md`'s DAMAGE row — `CanHitTarget`
  is `ovr024.cs:487`, not yet read for this specific question) and confirm
  against instrumented-oracle traces for edge-roll cases.

### FD-2: Exact initiative formula

- **Status:** narrowed (coab read 2026-07-16, M4 step 5; settles by draw-order
  parity per D-OR5(a))
- **Question:** What determines turn order each combat round — a single
  d10 per side, per-combatant rolls, DEX modifiers, weapon speed factors,
  spell casting-time penalties?
- **coab evidence:** per-combatant, **not** per-side. `CalculateInitiative`
  (`ovr014.cs:8`, `sub_3E000`): `action.delay = roll_dice(6,1) +
  DexReactionAdj(player)`, clamped to `[1,20]`, with a `-6` team-surprise
  adjustment when `area2.field_596` flags the team, out-of-range collapsing to 0
  (`ovr014.cs:31-47`). Turn order is then resolved by `FindNextCombatant`
  (`ovr009.cs:59`), which each pass rolls a fresh d100 for **every** `TeamList`
  member and yields the highest-`delay` member, ties broken by the highest d100
  (`ovr009.cs:70-87`). The order is **consumed and never persisted** — so per
  D-OR5(a) draw-order parity settles this docket by itself (two orderings can
  share an endstate; only the draw stream distinguishes them). Full read +
  per-round draw-stream shape: `docs/design/combat-study.md` §2.
- **Settled by:** H4 (M4) — Phase-0 QuickFight capture + Phase-1 replay matching
  draw order for ≥10 seeds (`oracle-rig.md` D-OR5(a)).
- **Progress (2026-07-16, D-OR5(a) Phase 1 first slice):** the formula is now
  **implemented** in `gbx-engine`'s `combat` module — `CalculateInitiative`
  (one d6 + `Flavor::dex_reaction_bonus`, clamp-to-1, team `-6`, out-of-range →
  0, the clamp-then-subtract ordering transliterated) and `FindNextCombatant`
  (one d100 per roster member per pass, the two-`if` tie-break, `max_delay==0`
  termination), with synthetic draw-sequence tests. This does **not** settle the
  docket: settlement still requires the live Phase-0 capture replayed to
  draw-order parity for ≥10 seeds. The `init`/`pick` action-profile events are
  now pinned (`combat-study.md` §9; `gbx-oracle` `InitEvent`/`PickEvent`) to
  bracket those draws for the parity check.

### FD-3: Attacks-per-round schedule

- **Status:** narrowed (coab read 2026-07-16, M4 step 5)
- **Question:** How does the engine grant multiple attacks per round (high-
  level fighters, specialization, monster multi-attacks, weapon speed)?
- **coab evidence:** the 3/2-attacks rule is folded into a round-parity test:
  `ThisRoundActionCount(halfActions)` (`ovr014.cs:519`, `sub_3EF0D`) =
  `(halfActions + (combat_round & 1)) / 2` — a combatant with 3 half-actions
  gets 2 attacks on odd rounds, 1 on even (3 per 2 rounds). `attack1_AttacksLeft`
  (`@0x19c`) is set from `attacksCount` then reduced to this
  (`reclac_attacks`, `ovr014.cs:462-514`); ranged weapons override the count from
  the item's `numberAttacks` (min 2) and clamp to ammo. Two attack profiles per
  combatant (attack1/attack2, `Player.cs:646-703`) carry monster multi-attacks.
  Full read: `docs/design/combat-study.md` §3.1. **Landmine:** attack counts
  depend on `combat_round` parity — a replay that resets round parity diverges on
  multi-attack combatants.
- **Settled by:** H4 (M4) — round-count + round-start `attacks_left` checkpoint
  (D-OR5(b)).

### FD-4: Sleep/held auto-kill rules

- **Status:** narrowed (structure identified 2026-07-16, M4 step 5; leaf still
  unread)
- **Question:** Does a coup-de-grace against a sleeping or held target
  auto-hit, auto-crit, or auto-kill? What conditions gate it (melee only?
  any weapon? specific spell interactions?)
- **coab evidence:** sleep/held are `Affect`s (§7 of the study), not a single
  coup-de-grace branch — the to-hit path gates on them via
  `CheckAffectsEffect(target, CheckType.Type_16)` inside `CanHitTarget`
  (`ovr024.cs:500`). A distinct multi-target mechanism exists,
  `TrySweepAttack` (`ovr014.cs:530`): a melee attacker with spare attacks vs a
  **`HitDice == 0`** target. The exact auto-hit/auto-kill *condition* lives in the
  `CheckAffectsEffect` handlers + the melee-attack leaf (`sub_35DB1`), not yet
  read to the leaf. See `docs/design/combat-study.md` §5.5.
- **Settled by:** H4 (M4) — the melee-leaf read + a **curated 1v1 encounter**
  (one attacker, one sleeping/held target) so an HP delta or kill is attributable
  to a specific draw (`oracle-rig.md` D-OR5 FD-1/FD-4 note; the M4 exit-gate
  encounter set must include these).

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

- **Status:** RESOLVED (2026-07-16, M4 step 5 — real `MON*CHA.DAX` census:
  the out-of-range read is unreachable from shipped monster data)
- **Resolution:** a census of all six real `MON{1..6}CHA.DAX` archives (81
  monsters total; `gbx_formats::monster::parse_cha_archive`, decoding each block
  as the 0x1A6 `Player` record — see below on why the monster record IS a
  character record) found **`field_E9` (byte `@0xe9`) == 0 for every one of the
  81 records** (max = 0; `crates/gbx-formats/src/monster.rs`
  `local_tier_monster_census`, run under `GBX_DATA_DIR`). So `turns_undead`'s
  index `unk_16679[field_E9 * 10 + band]` (`ovr014.cs:642`) never exceeds the
  11-row (types 0..10) image table from shipped data — the behavioral risk is a
  **non-issue**. Cross-check confirming the decode is real and not a stuck-zero
  bug: across the same 81 records `monster_type` (`@0x11a`) ranges to 19 and
  `hit_dice` (`@0xe5`) to 20 — the records vary; only `field_E9` is uniformly 0.
  **Sub-finding (for the combat sessions):** `field_E9` is not a stored monster
  attribute at all — it is a **runtime combat flag**. The only writers are
  runtime: an Animate-Dead-style spell sets it to exactly 1 (`ovr023.cs:1550`),
  it is reset to 0 (`ovr013.cs:439`), and `FindLowestE9Target` (`ovr014.cs:697`)
  only considers combatants with `field_E9 > 0`. Shipped monster data leaves it
  0; CotAB's inherently-undead monsters are keyed by `monster_type` (`@0x11a`,
  the `MonsterType` enum — troll/animated_dead/etc. branches in `ovr013.cs`), not
  by `field_E9`. (The docket's earlier "copied from monster data's `field_76`,
  `ovr017.cs:286`" was the *PoolRad-import* conversion path
  `player.field_E9 = poolRad.field_76`, a different source record — not the CotAB
  monster read, which is `@0xe9` direct.)
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
  oracle-rig adversarial round; **the implementation exists** as
  `crates/gbx-prng` (`Prng::next`/`random`), M4 step 1 2026-07-15;
  **D-OR4 part A is now DONE** (M4 step 2, 2026-07-15) — a purpose-built
  8086 stepper executed the real wrapper+`RandNext` bytes and matched
  `gbx-prng` bit-for-bit over 10,000 (K,N) pairs, and the real bytes
  *empirically refuted* the v1 "scaled high word" claim (first at k=1, n=3:
  real bytes = 1, v1 scaled = 0 — a project first). Executable confirmation
  now = **part A done**; part B (the live staging session) remains)
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
- **Settled by:** D-OR4 part A (**DONE** — `crates/gbx-prng/tests/stepper.rs`,
  a purpose-built 8086 stepper executing the pinned routine bytes vs
  `gbx-prng`, 10,000 (K,N) pairs, 0 mismatches; asserts the `[0xa55a,0xa5ee)`
  pin before stepping; the `N==0` draw-always contract confirmed *by
  execution* across every edge K; teeth tests prove the acceptance test can
  fail. Round 2 replaced v2's unicorn-engine pick, which does not build on
  this toolchain) + part B (one live staging-hook session with
  chain-continuity checks — the last remaining piece).
- **Cross-reference:** `docs/design/oracle-rig.md` §1/D-OR1/D-OR4;
  `crates/gbx-prng/tests/stepper.rs`.

### FD-27: Seed lifecycle — answered statically: one boot-time `Randomize`, no overlay RNG copies

- **Status:** RESOLVED (2026-07-16 — dynamic half confirmed by D-OR4 part B:
  across a 2,096-draw live session, **zero** post-poke `Randomize` firings and
  zero foreign writes to `DS:0x47F0` (chain continuity held on every link);
  the boot capture separately showed the one `InitFirst` seeding firing before
  the first draw, exactly as the static census predicted)
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
- **Interim posture (D-OR1 draw-parity contract) — IMPLEMENTED (M4 step 1):**
  `apply_recolor_dithered` (`crates/gbx-engine/src/draw.rs`) no longer takes a
  `VmRng` at all; it dithers via the deterministic position hash `dither_hit`
  (Knuth multiplicative hash of the flat pixel index, ~1-in-4). It touches
  `gbx-prng` zero times, so the traced draw stream is dither-free regardless of
  the answer. Dither pixels are already declared non-comparable by the renderer
  doc.
- **Settled by:** a tier-1 staging-hook trace captured across a
  fade/recolor transition — if RandNext fires per pixel, revisit; if not,
  the position-hash divergence is documented and permanent.
- **Cross-reference:** `docs/design/oracle-rig.md` D-OR1(c)/§5.

### FD-29: `roll_dice` truncates its total to a byte in the original; ours returns `u16`

- **Status:** open (candidate, filed M4 step 1 — no observable divergence at
  any current call site)
- **Question:** coab's `roll_dice` returns `(byte)roll_total`
  (`ovr024.cs:595`) — it truncates the summed total to 8 bits (the original's
  return is `AL`). Our `EngineServices::roll_dice`
  (`crates/gbx-engine/src/vmhost.rs`, `frontends/cli/src/run_script.rs`) and
  its `host.rs:171` trait signature return `u16` and never truncate. Should
  our `roll_dice` truncate to a byte to match?
- **Evidence / reachability:** truncation is only observable when a single
  `roll_dice(size, count)` total exceeds 255, i.e. `count * size > 255`.
  A full census of coab's call sites (2026-07-15 orchestrator audit,
  `grep -rhoE "roll_dice\([^)]*\)" engine/ Classes/`) splits them in two:
  - **Literal-argument sites — all provably inert.** The largest single-call
    maximum across every literal site is **100** (`roll_dice(100, 1)`, 26
    sites); the largest *multi-die* literals are `roll_dice(4, 5)` = 20,
    `roll_dice(8, 3)` = 24, and `roll_dice(6, 3)` = 18 (`ovr018.cs:675-683`,
    creation stats). Door bash uses `count == 1` (`ovr015.cs:180-215`).
    Nothing literal comes within 2× of 255.
  - **Data-driven sites — the MONSTER half now enumerated against real data
    (2026-07-16, M4 step 5); provably inert.** `roll_dice(attackDiceSize(idx),
    attackDiceCount(idx))` (monster damage dice, `ovr014.cs:86`) reads the live
    attack run at `MON*CHA` record `@0x19e..0x1a1`. Census of all 81 shipped
    monsters (`gbx_formats::monster`, `local_tier_monster_census`):
    **max `count · size` = 45** — far under 255, so monster damage never
    truncates. Monster HP is stored as a byte (`hit_point_max@0x78`), not rolled
    at load, so no monster-HP `roll_dice`; observed monster `hit_dice@0xe5` maxes
    at 20 (a hypothetical `roll_dice(8, 20)` = 160 is still inert). **Still not
    enumerated (data not yet read):** *weapon* damage dice (the `ItemData` table,
    an item-session read) and the class hit-die table `unk_16B32`/`unk_1A8C4`
    (creation-time, a character-creation read). The other named data-driven sites
    (`roll_dice(6, spellMaxTargetCount)`, `roll_dice(nearTargets.Count, 1)`,
    `roll_dice(party_size, 1)`) are small by construction.
  So the `u16` return and coab's `(byte)` return are bit-identical for every
  input reachable *today*, and the divergence is inert at every call site our
  engine currently has (monster damage now *proven* so, not merely likely).
  Changing the signature to `u8` would churn the `VmHost` trait and all impls
  for no behavioral gain yet.
- **Progress (2026-07-16, M4 combat #2 — the truncation is now implemented in the
  combat damage path):** the attack slice's `roll_dice`
  (`crates/gbx-engine/src/combat.rs`) — the roller used by `roll_damage`
  (`sub_3E192`) and the to-hit/initiative draws — **now faithfully truncates its
  total to a byte** (`(total as u8) as u16`, the `(byte)roll_total`
  `ovr024.cs:595`), matching coab exactly regardless of `count · size`. So combat
  damage rolls are byte-truncation-faithful today (with a synthetic
  `roll_dice_truncates_the_total_to_a_byte` test exercising `count · size > 255`).
  This does **not** close the docket: the `VmHost`/CLI `roll_dice` (`vmhost.rs`,
  `run_script.rs`) — the ECL-opcode roller — still returns `u16` without
  truncating; its sites remain inert (census above), so the signature churn is
  still deferred. The open **weapon** clause is unchanged: `attackDiceSize/Count`
  for a PC's readied weapon come from the `ItemData` table (`Classes/ItemData.cs`
  `diceCountLarge/Normal`), which is **not decoded yet** (the item-table session,
  M5-adjacent) — so real weapon-dice extents still can't be censused here. coab
  wins where read: monster damage is inert (max 45), weapon extents TBD by data.
- **Settled by:** the remaining data-driven sites — the item session (weapon
  damage dice, `ItemData`) and character creation (class hit-die table). Check
  their real data extents against `count * size > 255` and decide then whether to
  narrow the `VmHost`/CLI return to a byte (the combat roller already truncates).
  The **monster** clause is closed: inert (`docs/design/combat-study.md` §8.3).
- **Cross-reference:** `docs/design/oracle-rig.md` §6 (migration ledger),
  `crates/gbx-engine/src/vmhost.rs` `roll_dice`.

### FD-30: Creation-roll draw ORDER — per-stat API cannot express the original's interleave

- **Status:** open (candidate, filed M4 step 1 — no production caller today,
  so nothing is broken; a live blocker for D-OR4 part B when creation lands)
- **Question:** the original rolls the six ability scores **interleaved within
  each of six reroll iterations** — `Str, Int, Wis, Dex, Con, Cha` per
  iteration, best-of-six per stat (`ovr018.cs:675-683`, each
  `Math.Max(prev, roll_dice(6,3) + 1)`). Our `Flavor::roll_ability_score`
  (`crates/gbx-rules/src/flavor.rs:108`, impl `adnd1/flavor_impl.rs:121`) is
  **per-stat**: any caller looping stats produces `Str×6, Int×6, …` — the same
  draw *multiset* in a **different order**. Against a per-draw oracle trace that
  is a draw-parity desync.
- **Evidence:** there is **no production caller** of `roll_ability_score` yet
  (only `flavor_impl.rs`'s test module, `:746`), so nothing is currently wrong.
  But D-OR4 part B's live acceptance window is *precisely* character-creation
  stat rerolls (the roll-heavy, boot-reachable, zero-prerequisite window the
  door picked), so when creation rolls land they **must** interleave, and the
  current per-stat API shape cannot express that.
- **Settled by:** the session that implements character creation — reshape the
  flavor API to roll all six stats per iteration (or otherwise emit draws in
  the original's interleaved order) *before* wiring it to the part-B trace. Do
  not redesign the flavor API speculatively before then.
- **Cross-reference:** `docs/design/oracle-rig.md` D-OR4 part B / §6,
  `crates/gbx-rules/src/flavor.rs`.

## 5. How new entries get added

Any session that surfaces a behavioral hypothesis not derivable purely from
static code reading — a coab claim that needs oracle confirmation, a
brief/Jzatopa disagreement, a census-observed pattern whose *cause* is
unclear — adds an entry here with the same shape: id, question, status,
evidence, settling rung. Update existing entries in place as evidence
accumulates; move `open` → `narrowed` → `resolved`/`deferred` rather than
duplicating entries.
