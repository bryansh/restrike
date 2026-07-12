# Opcode Census — Curse of the Azure Bonds v1.3

> Produced by `restrike census ~/goldbox-data/cotab` (M1 §6 build-order item 2,
> `docs/design/vm-scriptmemory.md`). This is the project dashboard per
> PLAN.md §2.6: opcode frequencies and hazard statistics are uncopyrightable
> facts about a legally-owned data set, not the data itself (D10) — no game
> bytes are reproduced here, only counts, addresses, and offsets.
>
> **Data set.** GOG *Forgotten Realms: The Archives — Collection Two*, CotAB
> engine v1.3 (matches `crates/gbx-formats/src/detect.rs`'s `DETECTION_TABLE`
> entry). All six shipped ECL area files — `ECL1.DAX`–`ECL6.DAX`, the
> complete set present in the data directory — parsed via
> `gbx_formats::dax::DaxArchive`, yielding **25 ECL blocks** (block IDs
> 80–82, 1–4, 16–18/21, 32–35/37, 48–51/53, 64/66/67/69 across the six
> files), **192,000 bytes** (`25 × 0x1E00`) after stripping each block's
> 2-byte container prefix (`gbx_formats::dax::ecl_block_payload` — see
> `dax.rs`'s module doc). Every block's flow-following disassembly starts
> from its 5 decoded header vectors (`gbx_vm::read_header_vectors`) via
> `gbx_vm::disassemble` (D-VM8). CSV: `opcode,name,count,pct_of_total`,
> reproducible via the command above (`--out` to save it; the CSV isn't
> checked in here since it's regenerable from the same command against the
> same legally-owned data — only this report, its citations, and its
> aggregate numbers are).
>
> **H1 status.** DaxDump.exe/EclDump.exe golden comparisons are Windows
> binaries not runnable in this environment yet — deferred to the
> oracle-rig milestone, per the task brief. Everything below is validated by
> internal consistency (clean flow-following traversal, zero decode
> desyncs — see §7) and by cross-checking the DAX container algorithm
> against two independent implementations (coab C# and ssi-engine Java —
> `dax.rs` module doc), not against an external golden yet.

## Summary

| | |
|---|---|
| Files | 6 (`ECL1.DAX`–`ECL6.DAX`) |
| Blocks | 25 |
| Total block bytes | 192,000 (`25 × 0x1E00`) |
| Reached instructions | 3,582 |
| Distinct opcodes reached | 52 of 65 |
| Code bytes (reached instructions) | 25,868 (13.5%) |
| Data/unreached bytes | 166,132 (86.5%) |
| Decode-error bytes | 0 |
| Skip-divergence hazards | 0 |
| Decode desyncs (see §7) | 0 |
| Out-of-block traversal targets | 0 |

The 86.5% "data" share is expected, not a coverage problem: it's
overwhelmingly PRINT/menu prose text (inline `0x80` strings and `0x81`
in-block string targets), GETTABLE/SAVETABLE data tables, and — per D-VM8's
stated limitation — regions only reachable after self-modification, which a
static traversal cannot follow. See the per-block table in §7 for the full
breakdown; some blocks (e.g. `ECL2.DAX#1` at 38.8% code) are dense event
logic, others (e.g. `ECL1.DAX#80` at 4.9%) are mostly narrative text.

## §1 — Does opcode `0x1F` appear anywhere (reachable or skipped-over)?

**No.** Zero occurrences, in either normal traversal or the skip-divergence
quarantine bucket, across all 25 blocks. This confirms
`opcode-classification.md` §5 item 1's expectation ("shipped scripts likely
never emit it directly since the skip path is only exercised if `0x1F` is
skipped over by a preceding false IF"): `0x1F` is entirely unused in the
shipped CotAB v1.3 data. **Docket item 1 (design doc §5): closeable** —
`0x1F` can be a no-op in the interpreter with this as its evidence, pending
the same result from a second data set (Buck Rogers, M7) if one ever
justifies revisiting the CotAB-specific claim.

## §2 — Any IF preceding a skip-divergent opcode (`0x15`/`0x25`/`0x26`/`0x2B`/`0x34`/`0x36`)?

**No.** Zero `SkipDivergence` hazards and zero `UnknownSkipTargetOpcode`
hazards. Shipped CotAB scripts never place a conditional IF immediately
before VERTICAL MENU, HORIZONTAL MENU, ON GOTO, ON GOSUB, ECL CLOCK, or ADD
NPC. **Docket item 2: answered for CotAB v1.3** — the skip/run mismatch is a
confirmed *machine* hazard (opcode-classification.md's citations are exact),
but it is never *exercised* by this data set. Our `step()`/skip
implementation still needs the table-driven skip-size semantics (per D-VM6),
since a future dialect or a differently-compiled CotAB build could still
trigger it — this result only says *this* data never does.

## §3 — Any non-immediate (memory-mode) menu counts (`UnresolvedVariableTail`)?

**No.** Zero occurrences. Every VERTICAL MENU / HORIZONTAL MENU / ON GOTO /
ON GOSUB count operand in the shipped data is an immediate (`ImmByte` or
`ImmWord`), matching `vm-scriptmemory.md` §1's stated expectation exactly.
**Docket item in vm-scriptmemory.md §1** ("expectation is that shipped
scripts always use immediates — verify via census on real data, docket if
violated"): **confirmed, no violation found.**

## §4 — Any unknown mode bytes in reached code?

**No.** Zero `Arg::UnknownMode` operands across 3,582 reached instructions.
Every operand in the shipped data uses one of the six documented modes
(`0x00`, `0x01`, `0x02`, `0x03`, `0x80`, `0x81`).

## §5 — CALL (`0x2D`): every distinct operand key seen vs. the 7 known cases

52 CALL instructions reached, resolving to **4 of the 7 known dispatch
keys** (`opcode-classification.md` §3), zero unrecognized keys:

| Key | Case | Uses | Blocks |
|---|---|---:|---|
| `0xAE11` | wall-roof/wall-type query + conditional redraw | 38 | `ECL2#1,2,3,4`, `ECL3#16,17,18`, `ECL4#32,33,34,35,37`, `ECL5#49,50,51,53`, `ECL6#64,66,67` |
| `0xE804` | sprite/animation frame advance + timed delay | 11 | `ECL1#82` (all 11) |
| `0x3201` | sound-effect variant selection | 2 | `ECL4#35` |
| `0x401F` | move party one cell forward | 1 | `ECL4#35` |
| `1` / `2` | duel setup (`SetupDuel`) | 0 | — |
| `0x4019` | wall-type query, non-dungeon gated | 0 | — |

**Docket item 9 (design doc, resolved by the earlier classification pass):**
the enumeration holds — no key outside the documented 7 appears anywhere in
the shipped data. The two unobserved cases (duel setup, the
`!inDungeon`-gated wall query) are plausible absences for a corpus that is
entirely dungeon/town area scripts (`0xAE11`'s dominance — 38 of 52 calls —
is exactly the "every world-menu command" per-step wall/redraw check the
design doc's vector-table row 1/2 describes); their absence here isn't
evidence they're unused in the full game, just unused in this data.

## §6 — Operand-mode usage stats (`0x01` vs `0x03`) and Global-window writes

**`0x01` (Mem): 2,787 of 2,787 memory-mode operands (100.0%). `0x03`
(MemAlt): 0 (0.0%).** Every single memory-mode operand in the shipped data
uses `0x01`; `0x03` never appears once. This is a strong empirical
confirmation of `vm-scriptmemory.md` §1's claim ("modes `0x01` and `0x03`
are treated identically on both read and write paths in coab... the
encoding distinction, if any, is cosmetic") — cosmetic *and*, in this data,
entirely unused. **Docket item 3: the `0x01`/`0x03` half is closeable**
(low priority, as already flagged, now with zero-use evidence behind it).

The *write-destination-operand* half of this docket item is **not**
answered here by design: distinguishing which operand index a given opcode
treats as its destination requires per-opcode operand-role semantics (e.g.
"ADD's 3rd operand is the destination") — that's interpreter-shaped
knowledge (which `EclMachine`/`ScriptMemory` would encode), explicitly out
of the census tool's scope per the task brief. What the census *can* and
does report instead: every memory-mode operand whose address falls in the
**Global** window (outside Area/Table/Party/Ecl —
`vm-scriptmemory.md` §1's window table):

**199 of 2,787 memory-mode operands (7.1%) address the Global window.**
Two clusters, both explainable:

1. **`0xC04B`–`0xC04F`** (facing/position-adjacent cells, matching the
   design doc's named globals `mapPosX`/`mapPosY` at `0xC04B`/`0xC04C`) —
   the large majority of Global hits, e.g. `ECL2.DAX#1 @ 0x81F5 -> 0xC04B`.
2. **CALL's own dispatch-selector operand.** CALL (`0x2D`) decodes its one
   operand in mode `0x01` (Mem) — e.g. `ECL2.DAX#1 @ 0x8C7E -> 0x2E10`
   (Global) is the *same instruction* as the CALL-key citation
   `ECL2.DAX#1 @ 0x8C7E` in §5 (`0x2E10 + 0x8001 = 0xAE11` mod `0x10000`,
   confirming the wraparound arithmetic). This isn't a genuine memory
   read — `opcode-classification.md`'s CALL row already establishes the
   operand's raw `.Word` is only ever used as a dispatch key, never
   dereferenced — so every CALL citation in this Global-hit list is a
   second, independent, real-data confirmation of docket item 3's broader
   claim ("destination/target operands never trigger a ScriptMemory read
   regardless of encoded mode").

**The SURPRISE (`0x23`) `0x2CB` pattern: 0 hits, as expected.** SURPRISE
itself never appears in this data set (see §8's opcode list), and even if
it did, `0x2CB` is a hard-coded literal address inside `CMD_Surprise`
(`opcode-classification.md`'s SURPRISE row: "not operand-addressed!") — it
would never show up as a decoded operand regardless. **Docket item 8
(design doc §5, new candidate list item 8): unconfirmable from this
census by construction** — a future disassembler pass that recognizes
fixed-address writes baked into specific opcodes (not just operand-decoded
addresses) would be needed to verify `0x7F3F`/`0x2CB`-style hard-coded
cells empirically; static bytecode disassembly alone can't see them.

## §7 — Per-block coverage and decode desyncs

| Block | Code bytes | Code % | Data bytes | Vectors (`vm_run_addr_1`, `SearchLocationAddr`, `PreCampCheckAddr`, `CampInterruptedAddr`, `ecl_initial_entryPoint`) |
|---|---:|---:|---:|---|
| ECL1.DAX#80 | 380 | 4.9% | 7,300 | 0x806A, 0x806B, 0x9BF4, 0x9C01, 0x8014 |
| ECL1.DAX#81 | 577 | 7.5% | 7,103 | 0x814A, 0x814B, 0x9C53, 0x9C60, 0x8014 |
| ECL1.DAX#82 | 901 | 11.7% | 6,779 | 0x8397, 0x8398, 0x8395, 0x8396, 0x8014 |
| ECL2.DAX#1 | 2,980 | 38.8% | 4,700 | 0x8137, 0x8286, 0x81EF, 0x8225, 0x8014 |
| ECL2.DAX#2 | 1,439 | 18.7% | 6,241 | 0x8093, 0x8133, 0x80DB, 0x8122, 0x8014 |
| ECL2.DAX#3 | 743 | 9.7% | 6,937 | 0x80F2, 0x82CC, 0x824C, 0x82B0, 0x8014 |
| ECL2.DAX#4 | 1,025 | 13.3% | 6,655 | 0x8058, 0x80E1, 0x8072, 0x80D0, 0x8014 |
| ECL3.DAX#16 | 2,195 | 28.6% | 5,485 | 0x8198, 0x81BB, 0x8275, 0x82FA, 0x8014 |
| ECL3.DAX#17 | 566 | 7.4% | 7,114 | 0x82E1, 0x8523, 0x84F6, 0x851E, 0x8014 |
| ECL3.DAX#18 | 2,127 | 27.7% | 5,553 | 0x80AB, 0x80B7, 0x8066, 0x80A6, 0x8014 |
| ECL3.DAX#21 | 573 | 7.5% | 7,107 | 0x8113, 0x82C8, 0x80F5, 0x810F, 0x8014 |
| ECL4.DAX#32 | 2,127 | 27.7% | 5,553 | 0x81F7, 0x82A4, 0x826F, 0x8297, 0x8014 |
| ECL4.DAX#33 | 1,460 | 19.0% | 6,220 | 0x8424, 0x8479, 0x84A7, 0x84E3, 0x8014 |
| ECL4.DAX#34 | 1,040 | 13.5% | 6,640 | 0x8465, 0x850A, 0x8482, 0x84D4, 0x8014 |
| ECL4.DAX#35 | 1,080 | 14.1% | 6,600 | 0x8030, 0x8042, 0x8041, 0x8040, 0x8014 |
| ECL4.DAX#37 | 1,564 | 20.4% | 6,116 | 0x815C, 0x823F, 0x822E, 0x823B, 0x8014 |
| ECL5.DAX#48 | 210 | 2.7% | 7,470 | 0x8242, 0x8245, 0x8243, 0x8244, 0x8014 |
| ECL5.DAX#49 | 363 | 4.7% | 7,317 | 0x8397, 0x84A9, 0x80CD, 0x80FE, 0x8014 |
| ECL5.DAX#50 | 559 | 7.3% | 7,121 | 0x811D, 0x85D4, 0x859E, 0x85C7, 0x8014 |
| ECL5.DAX#51 | 708 | 9.2% | 6,972 | 0x87A3, 0x8EB4, 0x8E84, 0x8EB0, 0x8014 |
| ECL5.DAX#53 | 1,086 | 14.1% | 6,594 | 0x820D, 0x86B2, 0x81EF, 0x8209, 0x8014 |
| ECL6.DAX#64 | 723 | 9.4% | 6,957 | 0x80AA, 0x8231, 0x8208, 0x822D, 0x8014 |
| ECL6.DAX#66 | 594 | 7.7% | 7,086 | 0x8081, 0x81F2, 0x81B8, 0x81EE, 0x8014 |
| ECL6.DAX#67 | 265 | 3.5% | 7,415 | 0x8104, 0x81AD, 0x817F, 0x818C, 0x8014 |
| ECL6.DAX#69 | 583 | 7.6% | 7,097 | 0x8102, 0x82B1, 0x80E4, 0x80FE, 0x8014 |

Every block's `ecl_initial_entryPoint` vector (the 5th) is `0x8014` — the
same address in every single block, always resolvable, always in-block.
That's a real, consistent pattern worth a docket note: it suggests block
entry always lands at a fixed offset from the block base regardless of
content, i.e. the CotAB compiler/editor tool always emits the 5-vector
header plus (evidently) a small fixed prologue before the "real" entry
code — 20 bytes (`0x8014 - 0x8000`) exactly matches this census's own
corrected header layout (5 vectors × 4 bytes each — 1 wasted anchor byte +
3 payload bytes per word-mode vector — `read_header_vectors`'s doc comment
in `decode.rs`). **New docket candidate**: confirm this holds for every
ECL area across the full game (not just this 25-block sample) once a
second data set (or a save-game/area-table cross-reference) is available.

**Decode desyncs: zero.** No `DecodeError::UnknownOpcode` reached via
normal (non-quarantine) traversal, and no traversal target resolved outside
a block's own `0x8000..=0x9DFF` window. This is a strong positive
signal for H1: the decoder's byte-accounting model
(`docs/design/vm-scriptmemory.md` D-VM1, `crates/gbx-vm/src/decode.rs`) is
internally consistent across 3,582 real, independently-decoded
instructions with zero contradictions. See §9 for the one real bug this
census development *did* find and fix (in the census tool's own header-vector
reader, not in `decode.rs`).

## §8 — Frequency-ordered opcode list (interpreter implementation order)

52 of 65 opcodes appear in the shipped data; **13 never appear**: `0x0F`
INPUT NUMBER, `0x10` INPUT STRING, `0x1E` CHECKPARTY, `0x1F` (unknown, §1),
`0x22` PARTY SURPRISE, `0x23` SURPRISE, `0x28` ROB, `0x34` ECL CLOCK, `0x39`
WHO, `0x3B` SPELL, `0x3C` PROTECTION, `0x3E` DUMP, `0x3F` FIND SPECIAL.
These are real gaps in *this* corpus (all 6 shipped `ECLn.DAX` files, not a
sample) — plausible for content-dependent opcodes (SPELL/PROTECTION/DUMP
are all situational), but each is still worth an interpreter conformance
test built from the opcode table alone (D-VM6/H2), since "never observed"
isn't "never shipped anywhere in the game" (character-file-driven or
training-hall-only scripts, if any exist outside these blocks, aren't
covered here).

Top 25 by frequency (marked `*`), the recommended interpreter build order
per `vm-scriptmemory.md` §6 item 3 ("the ~15–25 most frequent opcodes"):

| Rank | Op | Name | Count | % |
|---:|---|---|---:|---:|
| 1 | `0x09` | SAVE | 463 | 12.93% |
| 2 | `0x03` | COMPARE | 411 | 11.47% |
| 3 | `0x01` | GOTO | 379 | 10.58% |
| 4 | `0x00` | EXIT | 278 | 7.76% |
| 5 | `0x16` | IF = | 269 | 7.51% |
| 6 | `0x17` | IF <> | 203 | 5.67% |
| 7 | `0x02` | GOSUB | 201 | 5.61% |
| 8 | `0x12` | PRINTCLEAR | 184 | 5.14% |
| 9 | `0x11` | PRINT | 176 | 4.91% |
| 10 | `0x2F` | AND | 129 | 3.60% |
| 11 | `0x13` | RETURN | 74 | 2.07% |
| 12 | `0x0B` | LOAD MONSTER | 72 | 2.01% |
| 13 | `0x0E` | PICTURE | 69 | 1.93% |
| 14 | `0x04` | ADD | 58 | 1.62% |
| 15 | `0x08` | RANDOM | 53 | 1.48% |
| 16 | `0x2D` | CALL | 52 | 1.45% |
| 17 | `0x2A` | GETTABLE | 48 | 1.34% |
| 18 | `0x0C` | SETUP MONSTER | 39 | 1.09% |
| 19 | `0x1C` | CLEARMONSTERS | 38 | 1.06% |
| 20 | `0x2B` | HORIZONTAL MENU | 38 | 1.06% |
| 21 | `0x3A` | DELAY | 36 | 1.01% |
| 22 | `0x24` | COMBAT | 34 | 0.95% |
| 23 | `0x25` | ON GOTO | 33 | 0.92% |
| 24 | `0x19` | IF > | 31 | 0.87% |
| 25 | `0x18` | IF < | 30 | 0.84% |

These 25 opcodes cover **2,974 of 3,582 reached instructions (83.0%)** —
implementing just this list gets the interpreter to over four-fifths
real-script coverage by observed frequency. The full ranked list (all 52
observed opcodes) is reproducible via `restrike census`'s CSV output
(`opcode,name,count,pct_of_total`); it isn't checked in here (D10 — derive
it from the same command against the same legally-owned data), but the
ranking beyond rank 25 is, for reference: `0x1B` IF >= (28), `0x33` PRINT
RETURN (24), `0x14` COMPARE AND (19), `0x21` LOAD FILES (15), `0x05`
SUBTRACT (12), `0x37` LOAD PIECES (12), `0x20` NEWECL (9), `0x0D` APPROACH
(8), `0x07` MULTIPLY (7), `0x1A` IF <= (6), `0x40` DESTROY ITEMS (5),
`0x06` DIVIDE (4), `0x0A` LOAD CHARACTER (4), `0x26` ON GOSUB (4), `0x27`
TREASURE (4), `0x30` OR (4), `0x31` SPRITE OFF (3), `0x36` ADD NPC (3),
`0x3D` CLEAR BOX (3), `0x29` ENCOUNTER MENU (2), `0x32` FIND ITEM (2),
`0x15` VERTICAL MENU (1), `0x1D` PARTYSTRENGTH (1), `0x2C` PARLAY (1),
`0x2E` DAMAGE (1), `0x35` SAVE TABLE (1), `0x38` PROGRAM (1).

## §9 — Contradictions and fixes found during this session

**No open contradictions against `decode.rs`/`dialect.rs`/`disasm.rs` as
shipped.** Every hazard category the disassembler is built to detect
(skip-divergence, unresolved variable tails, unknown opcodes/modes reached
normally, out-of-block traversal targets) came back at **zero** across the
full real-data corpus — a clean, positive H1 result for the decode model as
it currently stands.

**One real bug was found and fixed in the census tool's own supporting
code**, not in the shared decoder: the first implementation of
`gbx_vm::read_header_vectors` (added this session, to decode a block's
5-vector header before the census could pick disassembly entry points)
assumed the 5 vectors are decoded contiguously, as if by one
`vm_LoadCmdSets(5)` call. Tracing `vm_init_ecl` (`ovr008.cs:115-124`)
byte-for-byte against `vm_LoadCmdSets`'s exact addressing (`ovr008.cs:9-80`)
showed the real behavior is **5 separate `vm_LoadCmdSets(1)` calls**, and —
because `vm_LoadCmdSets` reads its first operand at `ecl_offset+1` and
unconditionally advances `ecl_offset` once more *after* its loop — each
separate call wastes one unread "anchor" byte before its vector's real mode
byte. The contiguous-decode bug corrupted every vector after the first,
which cascaded into near-total garbage for the rest of every block (the
first (buggy) census run showed near-0% code coverage, out-of-block jump
targets in the tens-of-thousands-of-bytes range, and dozens of spurious
unknown-mode/decode-error hazards — all symptoms of a progressively
compounding one-block-header misalignment, not real findings about the
game). Fixed in `crates/gbx-vm/src/decode.rs` (`read_header_vectors`'s doc
comment has the full byte-for-byte trace); the corrected function is what
produced every number in this report. This is exactly the outcome D-VM8
anticipates a census run should be capable of catching — in this case, in
its own supporting code rather than in the shared decoder, which is why it
was fixed rather than reported as an open contradiction: it was a
straightforward, unambiguous mistracing of `vm_LoadCmdSets`'s addressing,
confirmed byte-for-byte against source, not a case where the real game's
behavior is ambiguous or surprising.

## Provenance

New reference reads for this session, beyond the rows already in
`SOURCES.md`: `Classes/DaxFiles/{DaxCache,DaxFileCache,DaxHeaderEntry,
DaxArray,EclBlock}.cs`, `engine/seg040.cs` (`LoadDax`), `engine/seg042.cs`
(`load_decode_dax`), `engine/ovr008.cs` (`load_ecl_dax`,
`LoadCompressedEclString`, and a full re-read of `vm_LoadCmdSets` beyond
the design doc's earlier citations — see §9), ssi-engine (Java, GPL-3)
`data/{DAXFile,ContentFile}.java` as an independent cross-check of the DAX
container algorithm. See the `SOURCES.md` row added alongside this
document.
