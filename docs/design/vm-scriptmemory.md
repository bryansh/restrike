# Design: ECL VM & ScriptMemory

> M1 architecture pass per PLAN.md §9 operating rule 3 (one design review before each
> one-way door). Status: **v3, draft for review**. v1 was written 2026-07-11 from a
> read of coab's VM internals (read-for-behavior per D11 — no code copied; see
> SOURCES.md), then subjected to two independent adversarial reviews the same day;
> v2 folded in the verified findings (nested script runs, block ownership, resume
> semantics, the engine-services channel, skip fidelity, string-register persistence,
> PRINT pagination). A second, bounded review round scoped to v2's new surface
> produced v3: no architectural changes, but two §1 machine-behavior corrections
> (PROGRAM-9 terminates rather than resumes; skip/run mismatches exist on fixed-arity
> opcodes too), a single-borrow host fix in the ctx API, and tightened
> suspension/serialization/census contracts. All load-bearing citations were
> re-verified against coab directly. Review stops here: remaining risk is carried by
> M1's scheduled falsifiers (step-0 opcode classification, §4 conformance tests,
> census hazard reports). *Post-review note:* step 0 has since run
> ([opcode-classification.md](opcode-classification.md)) — it re-confirmed the
> load-bearing claims with exact citations and caught one example-level
> miscitation (PARLAY/TREASURE in D-VM3, corrected in place, adjudicated against
> coab before editing).
>
> Scope: the `gbx-vm` crate — bytecode decoding, the interpreter's execution and
> suspension model, and the ScriptMemory facade through which scripts touch engine
> state. Out of scope: the renderer/UI shell state machine (M2 design pass), save
> format (M3), and the exact byte-level operand accounting (pinned during
> implementation against ECLDump goldens, H1/H2).

## 1. The original machine, as verified in coab

Everything below was read directly from coab (CotAB dialect). File references are to
`~/src/goldbox-refs/coab/`.

**Execution loop.** `RunEclVm(offset)` (`engine/ovr003.cs:2147`) is a fetch/dispatch
loop over a global instruction pointer `ecl_offset`: read one opcode byte, look it up
in a `CommandTable`, run the handler. Handlers decode their own operands (advancing
`ecl_offset`) and call **directly into blocking UI routines** — e.g. `CMD_VertMenu`
ends in a modal `VertMenuSelect(...)` call. The loop exits when a handler sets
`stopVM` (EXIT, NEWECL) or externally when `party_killed`.

**Nested runs are real — and the parked script is terminated, not resumed.**
`RunEclVm` is re-entrant in the original at exactly one site: PROGRAM case 9 saves
`ecl_offset`, calls `TryEncamp()` — which itself runs `RunEclVm(PreCampCheckAddr)`
and possibly `RunEclVm(CampInterruptedAddr)` (`ovr003.cs:1913–1926`) — restores the
offset, then immediately performs **`CMD_Exit()`** (`ovr003.cs:1975–1981`): the outer
script never executes another instruction (the offset restore is dead code — every
`RunEclVm` entry reassigns it), and EXIT's side effects apply (set `stopVM`, clear
`vmCallStack`, clear encounter flags, restore `SelectedPlayer`). No other nested
call sites exist (combat/rest/training never re-enter the VM). So the original
parks a frame mid-instruction — it is genuinely live state, and a save taken inside
the camp menu happens above it — but never resumes one. Our activation stack must
represent the parked frame (for the save case and for PROGRAM-9's post-camp EXIT
semantics); *resumability* of outer frames beyond that is generality insurance, not
observed original behavior. The original saves and restores *only the instruction
pointer*: `compare_flags`, `vmCallStack`, and the string registers are process
globals shared (and clobberable) across nested runs.

**Program structure.** A script block is a `0x1E00`-byte buffer conceptually mapped at
VM address `0x8000` (`Classes/EclBlock.cs`; code addresses stored in scripts are
`0x8000`-based 16-bit VM addresses; coab reaches block bytes via a 16-bit wrap). There
is **one resident block at a time**, shared by all nested runs. The block header is
five decoded operands read at load (`ovr008.cs vm_init_ecl`) — five *separate*
one-operand batches, each preceded by an unread anchor byte, so the vectors sit at
block bytes 1–3, 5–7, 9–11, 13–15, 17–19 (see docket item 6) — an event-vector table:

| # | Vector | Fired |
|---|--------|-------|
| 1 | `vm_run_addr_1` | after every world-menu command (per-step handler) |
| 2 | `SearchLocationAddr` | after **every world-menu command** (search mode distinguished via `search_flags`, forced to 1 per firing) |
| 3 | `PreCampCheckAddr` | before camping |
| 4 | `CampInterruptedAddr` | camp interrupted |
| 5 | `ecl_initial_entryPoint` | block entry (move/load) |

The engine's **walk loop** is `sub_29758` (`ovr003.cs:2230+`); the post-NEWECL
**chain runner** is `sub_29677` (`ovr003.cs:2180+`). Both fire vectors in sequence
with a `vmFlag01` short-circuit after every run — that orchestration belongs to
`gbx-engine`, not the VM, but the VM contract must expose what it needs (see D-VM3).

**Block switching (NEWECL 0x20 / PROGRAM 0x38).** NEWECL loads the new block
*immediately, mid-flow*, re-parses vectors, **clears the call stack and flags**
(`ovr008.cs vm_init_ecl:102–107` — which also resets engine state: `inDungeon=1`,
encounter flags, rest-encounter params, `HeadBlockId`), sets `stopVM` + `vmFlag01`,
and the old script **never resumes** (`ovr003.cs:480–498`). When a *nested* run
triggers NEWECL, the interrupted engine flow **completes against the new block**:
`TryEncamp` has no `vmFlag01` check, so after a NEWECL in the pre-camp script the
camp UI still runs and `CampInterruptedAddr` — now the *new* block's vector — can
still fire; the chain runner only takes over when control unwinds to the walk loop
(`ovr003.cs:2342, 2363, 2386`). The M2 engine contract inherits this: a request in
progress finishes (with the new block resident) before the engine chains. Block bytes
are reloaded fresh from disk on every switch (`ovr008.cs load_ecl_dax`), while the
walk loop skips reload when re-entering the resident block — but **`vm_init_ecl`
runs on every walk-loop entry regardless** (`ovr003.cs:2268–2278`): call stack and
flags are cleared and the five vectors re-parsed from the (possibly self-modified)
resident bytes, plus conditional area-field resets (`RestField200Values`/
`RestField6F2Values` when `reload_ecl_and_pictures == false`). So self-modified
*data* persists across vector runs, self-modified *vectors* take effect on re-entry,
and nothing survives a block switch. PROGRAM is a grab-bag: case 0 = the full game
menu (save/load/training/party changes, then the script *continues*), case 3 =
party-kill (then EXIT), case 8 = endgame (prints, saves, exits), case 9 = run the
camp flow, then EXIT (the nested-run case above).

**Instruction encoding.** Opcode byte, then per-opcode operands. Each operand is a
mode byte + payload (`ovr008.cs vm_LoadCmdSets`, `Classes/Opperation.cs`):

| Mode | Payload | Meaning |
|------|---------|---------|
| `0x00` | 1 byte | immediate byte |
| `0x01`, `0x03` | 2-byte LE word | address; value read through ScriptMemory |
| `0x02` | 2-byte LE word | immediate word (used for code addresses) |
| `0x80` | length byte + packed bytes | inline compressed string → string register |
| `0x81` | 2-byte LE word | address; string copied from memory → string register |
| other | 1 byte | tolerated: consumed like an immediate byte (`vm_LoadCmdSets` else-branch), no error |

Modes `0x01` and `0x03` are treated identically on both read and write paths in coab
(destinations always use the raw `.Word`); the encoding distinction, if any, is
cosmetic → docket, low priority.

**String registers persist.** The 15-slot register file (`gbl.unk_1D972`,
`Gbl.cs:562`) is **never cleared between instructions** — only individual slots are
overwritten as string operands decode (`strIndex` restarts per batch). Staleness is
*observable original behavior*: `CMD_VertMenu` reads slot 1 as its header text even
when its own operands supplied no string; `CMD_Compare`'s string path compares slots
[2] vs [1] whenever *either* operand is a string, so a mixed compare reads one stale
slot by construction (`ovr003.cs:68–77`). The registers are genuine, serializable VM
state.

**Variable-length instructions.** Operand counts are per-opcode (0–14 in the CotAB
table), but menus have a data-dependent tail: `CMD_VertMenu` decodes 3 operands, uses
operand 3 as a count, rewinds one byte (the batch-final increment), and decodes that
many further string operands. If a count operand were a memory reference, static
disassembly of that instruction would be impossible; expectation is that shipped
scripts always use immediates — **verify via census on real data**, docket if
violated.

**Branching and skipping.** COMPARE / COMPARE AND / AND / OR set six relation flags
(`==, !=, <, >, <=, >=`) at once; string compare is used when either operand is a
string mode (≥ `0x80`). The six IF opcodes (`0x16`–`0x1B`) take no operands: each
tests one flag and, when false, skips the next instruction via `SkipNextCommand`.
**Skip is not decode**: `CmdItem.Skip` (`ovr003.cs:2424+`) advances by the opcode's
*static size column* by running `vm_LoadCmdSets(size)` — which fills string registers
and performs `0x81` memory reads as side effects — and for size-**0** opcodes
(EXIT, RETURN, APPROACH, the IFs, both MENUs, ON GOTO/GOSUB, COMBAT, …) it advances
**one byte only**. An IF-false over a variable-tail opcode (VERTICAL MENU…) therefore
lands the pc *inside operand bytes*. And the divergence is **not limited to
variable-tail opcodes**: a full sweep of the 65-entry table against every handler's
operand consumption found two fixed-arity mismatches — ECL CLOCK (`0x34`) and ADD NPC
(`0x36`) declare size 1 but their handlers decode **two operands in a single
`vm_LoadCmdSets(2)` call** (`ovr003.cs:2115/2117` vs `CMD_EclClock`/`CMD_AddNPC`),
while skip runs `vm_LoadCmdSets(1)` — so skip sizes must be
*transcribed from the original's size column, never derived from operand counts*.
Whether shipped scripts ever skip across a divergent opcode is a census question; our
skip must reproduce the original's table-driven behavior either way.
GOSUB/RETURN use an unbounded `Stack<ushort>`; ON GOTO / ON GOSUB are computed jumps.

**Text output blocks on pagination.** PRINT/PRINTCLEAR route through
`seg041 press_any_key`; on text-window overflow it issues a modal
`DisplayAndPause("Press any key to continue")` before clearing and continuing
(`seg041.cs:204–216`). This resolves v1's open question: text presentation includes
user-paced gates, and those keypresses are replay-trace inputs (H5).

**Opcode set.** CotAB uses 65 opcodes, `0x00`–`0x40`, enumerated with names in
`ovr003.cs SetupCommandTable`. `0x1F` is unknown even to coab (`"notsure 0x1f"`,
null handler) → docket. On an unknown opcode the original wedges (no offset advance);
see D-VM6. Note `SAVE` (0x09) is a plain memory/string write — **no game-save opcode
exists**; saving enters via PROGRAM(0)'s menu and the camp menu.

**ScriptMemory.** Scripts address engine state by raw 16-bit address. Addresses are
classified into windows (`ovr008.cs vm_GetMemoryValueType:300`, verified exact):

| Window | Range | Granularity | Backing (coab) |
|--------|-------|-------------|----------------|
| Area | `0x4B00`–`0x4EFF` | word | per-area persistent words (`area_ptr`, `0x6A00 + loc*2`) — includes ECL clock (`0x4BC6..`), `inDungeon` (`0x4BE6`) |
| Table | `0x7A00`–`0x7BFF` | word | `stru_1B2CA` words (`0xC00 + loc*2`) |
| Party | `0x7C00`–`0x7FFF` | word | `area2_ptr` words (`0x800 + loc*2`) **plus** read/write-through to the selected character's fields (`get_player_values` / `alter_character`) |
| Ecl | `0x8000`–`0x9DFF` | **byte** | the resident script block itself — self-modifiable, shared with instruction fetch |
| Global | everything else | word | a sparse set of named globals (`mapPosX/Y` at `0xC04B/0xC04C`, facing, wall info, the SURPRISE result cell `0x2CB`, …); unknown reads return 0, unknown writes are dropped |

Two windows have **write side effects**: writing `inDungeon` flips `game_state`
(`ovr008.cs:704`); position/facing writes set `positionChanged`. Party-window writes
route into character records. The facade must support per-cell hooks, not just
storage.

## 2. Decisions

**D-VM1 — One decoder, three consumers.** A single `decode(bytes, addr) → Instr`
function feeds the interpreter, the disassembler, and the census tool. The per-opcode
static table carries: name, dialect membership, **skip size** (the original's size
column — distinct from full decoded length), and the operand-tail shape (fixed count
or menu-style variable tail). Unknown mode bytes decode as immediate-byte operands
(matching the original's tolerance) and are flagged, not fatal.

**D-VM2 — The interpreter decodes live bytes at the pc; there is no pre-decoded IR.**
Scripts can write their own block through the Ecl window, and jumps target raw byte
addresses — a decoded-ahead representation would be both unfaithful and unsound.
Blocks are ≤ 7.5 KB; decode-per-step costs nothing. The pc is a `u16` VM address
(`0x8000`-based), exactly as scripts store it.

**D-VM3 — One machine, an activation stack, resumable execution (the D8 door).**
The unit of script execution is an `EclMachine` holding exactly what the original
holds globally:

- the **resident block** (owned here — one copy, shared by fetch, the Ecl window,
  and all activations; reloaded fresh on block switch, retained across vector runs);
- the parsed **vector table** (per-dialect count, not a hard-coded five);
- **shared mutable state**: compare flags, the 15 persistent string registers, the
  GOSUB call stack — deliberately *not* per-activation, matching the original's
  globals (nested runs clobber them; that's faithful);
- an **activation stack**: each activation is `{pc, pending}`. The engine pushes an
  activation to fire a vector (`enter(addr)`) — including *while an outer activation
  sits suspended mid-instruction* (the PROGRAM-9 camp case). `Done` pops it.

The machine never waits. `step(host)` executes (or continues) one instruction of the
top activation and returns:

- `Continue` — call again;
- `Effect(e)` — presentation output (text, picture, clear-box, sprite…). Effects are
  **engine-buffered** with one normative rule: effects yielded before a Request
  **must** be presented (or enqueued to presentation) in yield order, none dropped,
  before that Request's interaction is shown. The engine may stop stepping until
  presentation drains (that's where PRINT pagination and its "press any key" inputs
  live — engine-side, in the input trace, without a VM suspension);
- `Request(r)` — the activation is **suspended** awaiting a reply (menu selection,
  number/string input, combat outcome, camp flow, delay…). The engine services it
  over as many ticks as needed — possibly running *other activations* meanwhile —
  then calls `resume(reply, host)`, which completes the instruction (post-input
  memory write-backs happen here, inside the VM, with correct Origin) and returns
  the next `VmStep`. Instructions may legitimately yield **several**
  Effects/Requests before completing — ENCOUNTER MENU is an interactive loop
  (three Effect classes plus one Request *per iteration*), and DAMAGE's
  per-target loop has the same shape via data-dependent pagination. (v3
  originally miscited PARLAY/TREASURE here; the step-0 classification corrected
  it: PARLAY is single-shot — one Request, zero Effects — and TREASURE has no
  presentation at all.) `pending` therefore carries per-opcode continuation
  state (phase + decoded operands), not just the request kind. Coarse requests are preferred where the original's loop state is all engine
  state anyway (ENCOUNTER MENU's approach-distance dance lives in `area2_ptr` —
  the engine owns the loop, one memory write exits it);
- `Done(exit)` — `Exit::Ended` (EXIT/vector completion, pops the activation; if the
  activation below is suspended, the engine consults `pending()` — it does not blindly
  run the next vector) or `Exit::ChainTo(block_id)` (NEWECL/PROGRAM-8): **no VM
  context ever resumes across a chain** — the engine swaps the block immediately
  (applying the documented `vm_init_ecl` resets), *completes any engine flow already
  in progress against the new block* (the TryEncamp behavior in §1), then abandons
  the activation stack and enters via the chain-runner protocol (`vmFlag01`
  semantics, owned by the M2 engine loop).

Suspending is just returning; the whole `EclMachine` — block, shared state,
activation stack with pendings — is the save-anywhere unit (M3). Three snapshot
commitments now, because deep pendings (camp save above a parked PROGRAM-9 frame)
*will* land in user saves: snapshots carry a **version tag** and unknown versions are
**rejected, not migrated** (revisit at M3 if saves need more longevity); a restored
machine re-presents its **stored Request verbatim** via `pending()` — never
re-derived from memory or string registers; and restore is its own entry point
(`EclMachine::restore(snapshot, &Dialect)`) — the dialect is re-bound at restore,
never embedded in the snapshot. Fuel is **engine policy, not VM API**: `step()`
executes one instruction per call and cannot hang; the engine counts steps per tick
and raises the loud diagnostic itself (same for a GOSUB-depth watchdog — the
`call_stack` is faithfully unbounded, so a GOSUB-self loop grows it forever while
staying responsive; the engine flags depth past a threshold).

**D-VM4 — Four channels, chosen per opcode by a stated rule; one host borrow.**
Synchronous, value-returning services are context arguments; presentation is Effects;
user/time-scale interaction is Requests; everything the VM keeps is machine state.
The context is a **single borrow** — `step(&mut self, host: &mut dyn VmHost)` with
`trait VmHost: ScriptMemory + EngineServices { fn rng(&mut self) -> &mut dyn VmRng; }`
— because in `gbx-engine` the memory facade and the services are views over the
*same* state (the Party window reads the selected character that LOAD CHARACTER
retargets; services roll the engine's one PRNG per D9): three simultaneous `&mut`
borrows into one engine would be unconstructible in safe Rust. ScriptMemory and
EngineServices stay separate *traits* for classification and testing; `gbx-vm` ships
one composable `TestHost` (mock memory + scripted services + fixed rng) so
conformance tests aren't repriced every time the surface grows. The services are the
load-bearing addition: a large
fraction of the opcode set synchronously touches engine entities that are *not*
16-bit memory cells — LOAD CHARACTER retargets the selected player (redirecting
subsequent Party-window reads), LOAD/SETUP/CLEAR MONSTER, CHECKPARTY, PARTYSTRENGTH,
FIND ITEM/SPECIAL, DESTROY ITEMS, ADD NPC, WHO, ECL CLOCK (game time), SPELL
(memorized-spell queries), TREASURE's item instantiation, DAMAGE's saving throws.
These are `EngineServices` calls: defined in `gbx-vm`, implemented in `gbx-engine`,
deterministic, mockable in conformance tests. The **seam is fixed now**: placement
rule — returns a value or mutates game entities synchronously → service; paced or
user-facing → Effect/Request. **M1 step 0 produces the 65-opcode channel
classification table** (checked against each coab handler) before `step()` is
written; the classification lands in
[docs/design/opcode-classification.md](opcode-classification.md), and the
**full `EngineServices` trait is declared in one shot from it** — no
grow-a-method-per-opcode treadmill breaking every mock. Call recording for oracle
traces (H4) comes from a wrapper impl around the trait, not from the trait's shape.

**D-VM5 — ScriptMemory is a trait defined in `gbx-vm`, implemented in `gbx-engine`,
with the Ecl window intercepted VM-side.** The VM sees `read/write` (word),
`read_byte/write_byte`, and string read/write, each carrying an `Origin` (instruction
address; the engine implementation supplies block identity, which it knows). Address
resolution order: the VM intercepts `0x8000`–`0x9DFF` against its own resident block
*before* delegating — the engine never sees Ecl-window traffic (it couldn't service
it; the block lives in the machine). All other windows go to the engine
implementation, which carries the window map per game generation: known cells route
to named engine state (with write hooks for the side-effecting cells), unknown cells
fall through to a raw word store **so scripts still round-trip values they stash
there**, and every unknown access is logged once per (addr, kind). The unknown-access
log is the discovery backlog (PLAN §2.2); the map is seeded from coab's `Gbl.cs`
names and the `ovr008.cs` switch tables.

**D-VM6 — Unknown opcode = halt the block with a diagnostic.** Skipping is impossible
(length unknown) and the original wedges. The census (M1) exists precisely so this
never fires on real data unnoticed; conformance tests assert it fires loudly.
(Unknown *mode bytes*, by contrast, are tolerated exactly as the original tolerates
them — D-VM1.)

**D-VM7 — Dialects are data plus small code.** The opcode table (including skip
sizes and operand shapes) is registered per game flavor; operand modes and window
ranges are shared until real Buck Rogers data proves otherwise (census delta, M7).
Block size (`0x1E00`), vector-table length, and vector meanings are per-generation
parameters supplied by `gbx-formats`; vectors are accessed by dialect-defined index,
not a hard-coded enum.

**D-VM8 — Disassembly is flow-following, not linear sweep.** Blocks embed non-code
bytes (in-block strings addressed by `0x81` operands, GETTABLE/SAVETABLE tables,
self-modified regions); a linear sweep desynchronizes at the first data byte. The
disassembler starts from the vector table plus statically-known targets (GOTO/GOSUB/
ON-GOTO immediate tails, IF fall-through *and* table-driven skip successor), marks
unreached bytes as data, and resynchronizes only at known targets. Decode errors are
diagnostics in disassembly (mark and continue from the next known target) but halts
in execution (D-VM6). The census runs on the same traversal and **must report,
not assume**: memory-mode menu counts (`VariableTailUnresolved`), IF-preceding any
opcode whose skip size ≠ run consumption (the skip-divergence hazard — variable-tail
opcodes plus the `0x34`/`0x36` class), unknown modes, unreached regions. Divergent
skip successors (pc landing mid-operand) are **reported and quarantined** — decoded
into a tagged bucket excluded from headline census counts, never silently traversed —
because D-VM1's mode tolerance means mid-operand garbage usually decodes as plausible
instructions, and the census is the M6 gate metric. Known limitation, stated in
census output: regions reachable only after self-modification are reported as data.

## 3. API sketch

```rust
// gbx-vm — shapes only; names bikesheddable at implementation time.

pub struct EclMachine {
    block: EclBlock,               // resident block: fetch + Ecl window share it
    vectors: Vec<u16>,             // parsed per-dialect header
    flags: CompareFlags,           // [bool; 6] — shared across activations
    strings: StringRegs,           // 15 slots, PERSISTENT (never bulk-cleared)
    call_stack: Vec<u16>,
    runs: Vec<Activation>,         // top = executing; empty = idle
}
struct Activation { pc: u16, pending: Option<Pending> }
// Pending = per-opcode continuation: which phase of a multi-step instruction, its
// decoded operands, and — when the phase is awaiting-reply — the outstanding Request
// stored verbatim (pending() returns exactly this; awaiting-reply is a Pending
// phase, so "mid-instruction, more Effects coming" and "suspended on a Request" are
// distinguishable states). Serializable, like everything above.

pub enum VmStep {
    Continue,
    Effect(Effect),        // Print(String), Picture(u16), ClearBox, ... buffered by engine
    Request(Request),      // Menu{header, items, dest}, InputNumber{dest},
    Done(Exit),            //   Combat{..}, Camp, Delay{ticks}, ...
}
pub enum Exit { Ended, ChainTo(BlockId) }

impl EclMachine {
    pub fn load_block(block: EclBlock, dialect: &Dialect) -> Result<Self, HeaderError>;
    pub fn reinit(&mut self);          // walk-loop re-entry without reload (§1): clears
                                       //   flags + call stack, re-parses vectors from the
                                       //   (possibly self-modified) resident bytes
    pub fn restore(snapshot: Snapshot, dialect: &Dialect) -> Result<Self, RestoreError>;
    pub fn enter(&mut self, addr: u16);                  // push activation (vector or nested run)
    pub fn vector(&self, index: usize) -> Option<u16>;   // dialect-defined meaning
    pub fn step(&mut self, host: &mut dyn VmHost) -> Result<VmStep, VmError>;
    pub fn resume(&mut self, reply: Reply, host: &mut dyn VmHost) -> Result<VmStep, VmError>;
    pub fn pending(&self) -> Option<&Request>;           // top-of-stack; re-present after load
}

// Call legality (machine state × call → result); everything else is a VmError:
//   step:   top has no pending, or pending mid-instruction (more Effects) → runs
//           top is awaiting-reply → Err(StepWhilePending)
//           runs empty → Err(Idle)
//   resume: top is awaiting-reply + matching reply kind → completes instruction
//           otherwise → Err(ResumeWithoutPending) / Err(ReplyMismatch)
//   enter:  always legal (that's the nested-run case); pushes a fresh activation
// After Err(UnknownOpcode) (D-VM6) the machine is halted: only restore/load/reinit
// or abandoning it are meaningful.
pub enum VmError { UnknownOpcode { pc: u16, opcode: u8 },
                   StepWhilePending, ResumeWithoutPending, ReplyMismatch, Idle }

// ONE host borrow (D-VM4): memory + services are views over the same engine state.
pub trait VmHost: ScriptMemory + EngineServices {
    fn rng(&mut self) -> &mut dyn VmRng;
}
// EngineServices: declared in full from the M1-step-0 classification appendix.
// gbx-vm test support ships a composable TestHost (mock memory + scripted services
// + fixed rng); trace recording wraps a VmHost, it doesn't shape the trait.

pub trait ScriptMemory {
    fn read(&mut self, addr: u16, origin: Origin) -> u16;
    fn write(&mut self, addr: u16, value: u16, origin: Origin);
    fn read_byte(&mut self, addr: u16, origin: Origin) -> u8;
    fn write_byte(&mut self, addr: u16, value: u8, origin: Origin);
    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString;
    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin);
}

// decoder — shared by interpreter, disassembler, census (D-VM1, D-VM8)
pub fn decode(bytes: &BlockBytes, addr: u16, dialect: &Dialect) -> Result<Instr, DecodeError>;
pub struct Instr { pub op: Op, pub args: Vec<Arg>, pub next: u16 }
pub enum Arg { ImmByte(u8), Mem(u16), MemAlt(u16), ImmWord(u16), InlineStr(..), MemStr(u16),
               UnknownMode { mode: u8, byte: u8 } }
// skip uses the dialect table's skip_size (≠ full length for variable-tail opcodes)
// and performs operand side effects (string-register fills, 0x81 reads), per §1.
```

The engine's run loop (shape only — the M2 design owns its final form):

```rust
match machine.step(&mut host)? {
    VmStep::Continue => { /* step again, within the engine's per-tick step budget */ }
    VmStep::Effect(e) => buffer(e),          // MUST present in order before next Request UI
    VmStep::Request(r) => park(r),           // resume(reply, host) on a later tick;
                                             // engine may machine.enter(..) meanwhile
    VmStep::Done(Exit::Ended) => {           // activation popped; if machine.pending()
                                             //   is Some, an outer frame awaits — else
                                             //   next vector or idle
    }
    VmStep::Done(Exit::ChainTo(id)) => chain_to(id),  // swap block now; finish any
                                             // in-progress engine flow against it,
                                             // then abandon stack and chain (§1)
}
```

## 4. Conformance testing (H2)

- **Micro-ECL builder** (test-only assembler in `gbx-vm`): hand-construct synthetic
  blocks opcode-by-opcode — fully legal to ship. Every implemented opcode gets
  conformance programs asserting on the yielded step stream, memory traffic (mock
  ScriptMemory), service calls (mock EngineServices), flag state, and pc trajectory.
- **Skip-semantics tests**: IF-false over every opcode class, asserting table-driven
  advance (the size-0 one-byte case, the fixed-arity mismatches `0x34`/`0x36`, and
  skip's string-register/`0x81` side effects) — divergence here is silent plot
  corruption later.
- **Staleness tests**: string-register persistence across instructions (mixed
  COMPARE, menu header from a prior instruction) — behavior the original exhibits.
- **Suspension tests**: menus/inputs driven by scripted replies; nested-activation
  tests (suspend mid-instruction, `enter()` a vector, run it to `Done`, resume the
  outer); serialize the suspended machine, restore, `pending()`, resume —
  save-anywhere insurance from day one.
- **Runaway-script protection** is engine policy, not H2 surface: `step()` executes
  one instruction per call and cannot hang; the per-tick step budget and the
  GOSUB-depth watchdog are tested in `gbx-engine` (a `GOTO`-self and a `GOSUB`-self
  block trip their diagnostics).
- **Unknown-access log** asserted empty for conformance programs; asserted *non-empty
  and precise* for deliberately-unknown-address programs.
- **Disassembler goldens**: flow-following traversal on synthetic blocks with
  embedded data regions; later validated against EclDump.exe on real data (H1,
  local-only).
- **Real-script replays** (later, data-gated): expected text/branch outcomes captured
  from DOSBox/instrumented coab.

## 5. Open questions → fidelity docket seeds

1. ~~`0x1F` opcode semantics~~ **Resolved by census
   ([cotab-v1.3](../census/cotab-v1.3.md) §1): `0x1F` never appears anywhere in
   shipped CotAB scripts** — reachable or skipped-over. It therefore stays in the
   D-VM6 unknown-opcode class (loud halt): if it ever fires, that signals a
   corrupted pc or bad extraction, and a no-op would mask it.
2. ~~IF before skip-divergent opcodes / memory-mode menu counts~~ **Resolved for
   CotAB by census (§2, §3): zero occurrences of either.** The skip-divergence
   hazard is theoretical in this title and static disassembly is fully sound;
   the quarantine machinery stays for other dialects (re-run per title).
3. ~~Operand mode `0x01` vs `0x03`~~ **Resolved for CotAB by census (§6): mode
   `0x03` appears zero times in 2,787 shipped memory-mode operands.** The
   distinction is moot on real data; both stay decoded (`Arg::MemAlt` may matter
   in other titles — recheck in the M7/M9 census delta).
4. Nested `ChainTo` mechanism is now recorded from coab (§1: block swaps mid-flow,
   the interrupted engine flow completes against the new block, chaining happens at
   walk-loop unwind). Data-gated remainder: does shipped content actually exercise a
   NEWECL inside camp scripts, and does our composite flow match the oracle end-to-end?
5. Exact inline-string compression (bit-packing) — `gbx-formats` work, verified
   against ECLDump text output.
6. Byte-exact operand/offset accounting in `vm_LoadCmdSets` (the +1/+2/wrap dance) —
   **substantially validated on real data: zero decode desyncs across all 824 blocks
   of CotAB v1.3 (census §7).** Implementation also pinned the header subtlety: the
   five header vectors are five *separate* one-operand batches, each preceded by an
   unread anchor byte (`vm_LoadCmdSets` always behaves as if positioned on an opcode
   byte), so vectors live at block bytes 1–3, 5–7, 9–11, 13–15, 17–19 — not
   contiguously. EclDump golden cross-check remains open until the oracle rig lands.
7. Version skew: coab transliterates a specific CotAB build (its demo string says
   v1.3); our canonical target is the GOG build (PLAN §2.3). On data arrival, confirm
   the GOG binary's version and treat any behavioral delta as a detection-table +
   per-version quirk entry, not a coab error. More broadly: every coab-derived claim
   in this doc is a *hypothesis* until an H1/H2 check against real data or the
   running game confirms it — coab bootstraps, the game itself validates.

Resolved since v1: PRINT pagination (it blocks; handled engine-side as buffered
Effects + input-trace keypresses — §1, D-VM3).

## 6. What this unblocks (M1 build order)

0. **The 65-opcode channel classification** (Effect / Request / services / machine-
   internal, from each coab handler) — see
   [docs/design/opcode-classification.md](opcode-classification.md), before
   `step()` exists;
1. `decode()` + dialect table (names from coab, skip sizes from the original size
   column) → **disassembler** (flow-following, D-VM8) → validated against EclDump.exe
   on real data when it lands (H1);
2. **census tool** on the same traversal (the project dashboard, plus the D-VM8
   hazard reports);
3. `EclMachine::step()/resume()` + the ~15–25 most frequent opcodes (control flow,
   arithmetic, compare/IF/skip, print, menus, memory ops) against micro-ECL
   conformance tests;
4. ScriptMemory facade with the window table above, raw fallback store, and
   unknown-access logging — engine-side implementation grows as M2/M3 name more
   cells.
