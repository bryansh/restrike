# Design: ECL VM & ScriptMemory

> M1 architecture pass per PLAN.md §9 operating rule 3 (one design review before each
> one-way door). Status: **draft for review**. Written 2026-07-11 from a read of coab's
> VM internals (read-for-behavior per D11 — no code copied; see SOURCES.md).
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
ends in a modal `VertMenuSelect(...)` call (`ovr003.cs`, `sub_26EE9`). The loop exits
when a handler sets `stopVM` (EXIT) or externally when `party_killed`.

**Program structure.** A script block is a `0x1E00`-byte buffer conceptually mapped at
VM address `0x8000` (`Classes/EclBlock.cs`; `gbl.ecl_offset = 0x8000` on init). Code
addresses stored in scripts (GOTO targets, event vectors) are `0x8000`-based 16-bit VM
addresses; coab reaches block bytes via a deliberate 16-bit wrap
(`ecl_ptr[addr + 0x8000]` with `index & 0xFFFF`). The block header is five decoded
operands read at init (`ovr008.cs vm_init_ecl`), an event-vector table:

| # | Vector | Fired |
|---|--------|-------|
| 1 | `vm_run_addr_1` | every step (post-move handler) |
| 2 | `SearchLocationAddr` | search at current square |
| 3 | `PreCampCheckAddr` | before camping |
| 4 | `CampInterruptedAddr` | camp interrupted |
| 5 | `ecl_initial_entryPoint` | block entry (move/load) |

The engine's walk loop (`ovr003.cs sub_29677`) fires these vectors in sequence with a
`vmFlag01` short-circuit between them — that orchestration belongs to `gbx-engine`,
not the VM.

**Instruction encoding.** Opcode byte, then per-opcode operands. Each operand is a
mode byte + payload (`ovr008.cs vm_LoadCmdSets`, `Classes/Opperation.cs`):

| Mode | Payload | Meaning |
|------|---------|---------|
| `0x00` | 1 byte | immediate byte |
| `0x01`, `0x03` | 2-byte LE word | address; value read through ScriptMemory |
| `0x02` | 2-byte LE word | immediate word (used for code addresses) |
| `0x80` | length byte + packed bytes | inline compressed string → string register |
| `0x81` | 2-byte LE word | address; string copied from memory → string register |

String operands fill a register file of 15 slots (`gbl.unk_1D972`), reset at each
decode batch — registers are meaningful only within one instruction. Why `0x01` and
`0x03` both exist is unresolved (coab treats them identically on read) → docket.

**Variable-length instructions.** Operand counts are per-opcode (0–14 in the CotAB
table), but menus are data-dependent: `CMD_VertMenu` decodes 3 operands, uses operand
3 as a count, rewinds one operand, and re-decodes that many string operands. If a
count operand were a memory reference, static disassembly of that instruction would be
impossible; expectation is that shipped scripts always use immediates — **verify via
census on real data**, docket if violated.

**Branching model.** COMPARE / COMPARE AND / AND / OR set six relation flags
(`==, !=, <, >, <=, >=`) at once; string compare is used when either operand is a
string mode (≥ `0x80`). The six IF opcodes (`0x16`–`0x1B`) take no operands: each
tests one flag and, when false, **skips exactly the next instruction** via
`SkipNextCommand` (every opcode knows how to skip itself — free for us since decoding
is separate from execution). GOSUB/RETURN use an unbounded `Stack<ushort>`
(`vmCallStack`); ON GOTO / ON GOSUB are computed jumps.

**Opcode set.** CotAB uses 65 opcodes, `0x00`–`0x40`, enumerated with names in
`ovr003.cs SetupCommandTable` (EXIT, GOTO, GOSUB, COMPARE, ADD…, RANDOM, SAVE,
LOAD/SETUP MONSTER, PICTURE, INPUT NUMBER/STRING, PRINT/PRINTCLEAR, VERTICAL/
HORIZONTAL/ENCOUNTER MENU, IF-family, NEWECL, COMBAT, TREASURE, PARLAY, SPELL, …).
`0x1F` is unknown even to coab (`"notsure 0x1f"`, null handler) → docket. On an
unknown opcode the original effectively wedges (no offset advance); see D-VM6 for our
policy.

**ScriptMemory.** Scripts address engine state by raw 16-bit address. Addresses are
classified into windows (`ovr008.cs vm_GetMemoryValueType`, `:300`):

| Window | Range | Backing (coab) |
|--------|-------|----------------|
| Area | `0x4B00`–`0x4EFF` | per-area persistent words (`area_ptr`, `0x6A00 + loc*2`) — includes ECL clock (`0x4BC6..`), `inDungeon` (`0x4BE6`) |
| Table | `0x7A00`–`0x7BFF` | `stru_1B2CA` words (`0xC00 + loc*2`) |
| Party | `0x7C00`–`0x7FFF` | `area2_ptr` words (`0x800 + loc*2`) **plus** read/write-through to the selected character's fields (`get_player_values` / `alter_character`) |
| Ecl | `0x8000`–`0x9DFF` | the running script block itself — **self-modifiable** |
| Global | everything else | a sparse set of named globals (`mapPosX/Y` at `0xC04B/0xC04C`, facing, wall info, …); unknown reads return 0, unknown writes are dropped |

Two windows have **write side effects**: writing `inDungeon` flips
`game_state` between dungeon and wilderness (`ovr008.cs:704`); position/facing writes
set `positionChanged`. Party-window writes route into character records. The facade
must support per-cell hooks, not just storage.

## 2. Decisions

**D-VM1 — One decoder, three consumers.** A single `decode(block, addr) → Instr`
function feeds the interpreter, the disassembler, and the census tool. Operand shapes
(including the menu variable-tail) are described per-opcode in a static table, next to
the opcode's name and dialect membership.

**D-VM2 — The interpreter decodes live bytes at the pc; there is no pre-decoded IR.**
Scripts can write their own block through the Ecl window, and jumps target raw
byte addresses — a decoded-ahead representation would be both unfaithful and unsound.
Blocks are ≤ 7.5 KB; decode-per-step costs nothing at 2026 speeds. The pc is a `u16`
VM address (`0x8000`-based), exactly as scripts store it.

**D-VM3 — Resumable, non-blocking execution (the D8 door).** The VM never waits.
`step()` executes one instruction and returns:

- `Continue` — instruction complete, call `step()` again;
- `Effect(e)` — same as Continue, but the engine should present something
  (print text, show picture, clear box, sprite off…). Fire-and-forget, ordered;
- `Request(r)` — the VM is **suspended** mid-instruction awaiting a reply
  (menu selection, number/string input, combat outcome, block switch, delay, save).
  The engine services it over as many ticks as needed, then calls `resume(reply)`;
- `Done(exit)` — EXIT reached (or vector ran to completion).

All VM state (pc, call stack, flags, string registers, pending request) lives in the
`EclVm` value — suspending is just returning; save-anywhere and replay fall out.
The engine may also abandon a suspended VM outright (the original's `party_killed`
abort is an engine-side loop guard, not VM state).

**D-VM4 — Synchronous services are context arguments, not requests.** ScriptMemory
and the PRNG are passed into `step(ctx)` by `&mut` — RANDOM and memory reads return
values immediately rather than generating suspension chatter. Requests are reserved
for interactions that genuinely take time (user input, combat, block loads).
Determinism (D9): the VM owns no RNG and no clock; DELAY yields a request carrying
tick counts.

**D-VM5 — ScriptMemory is a trait defined in `gbx-vm`, implemented in `gbx-engine`.**
The VM sees `read(addr, origin) → u16`, `write(addr, value, origin)`, plus string
read/write; `origin` is `(block_id, instruction_addr)` so the access log can say
*which script, where*. The engine implementation carries the window map per game
generation: known cells route to named engine state (with write hooks for the
side-effecting cells), unknown cells fall through to a raw word store **so scripts
still round-trip values they stash there**, and every unknown access is logged once
per (addr, kind). The unknown-access log is the discovery backlog (PLAN §2.2); the
map is seeded from coab's `Gbl.cs` names and the `ovr008.cs` switch tables.

**D-VM6 — Unknown opcode = halt the block with a diagnostic.** Skipping is impossible
(length unknown) and the original wedges. The census (M1) exists precisely so this
never fires on real data unnoticed; conformance tests assert it fires loudly.

**D-VM7 — Dialects are data plus small code.** The opcode table is registered per
game flavor (name, operand shape, handler); operand modes and window ranges are
shared until real Buck Rogers data proves otherwise (census delta, M7). Block size
(`0x1E00`) and vector-header shape are per-generation parameters supplied by
`gbx-formats`.

## 3. API sketch

```rust
// gbx-vm — shapes only; names bikesheddable at implementation time.

pub struct EclVm {
    pc: u16,                       // 0x8000-based VM address
    block: EclBlock,               // owned, mutable copy (self-modification)
    call_stack: Vec<u16>,
    flags: CompareFlags,           // [bool; 6]
    strings: StringRegs,           // 15 slots, reset per decode batch
    pending: Option<Pending>,      // Some(_) while suspended
}

pub enum VmStep {
    Continue,
    Effect(Effect),                // Print(String), Picture(u16), ClearBox, ...
    Request(Request),              // Menu{items, dest}, InputNumber{dest}, Combat{..},
    Done(Exit),                    //   NewEcl{id}, Delay{ticks}, Save, ...
}

impl EclVm {
    pub fn from_block(block: EclBlock) -> Self;      // parses the 5-vector header
    pub fn vector(&self, v: Vector) -> u16;          // Entry | Step | Search | PreCamp | CampInterrupted
    pub fn jump_to(&mut self, addr: u16);
    pub fn step(&mut self, ctx: &mut ScriptCtx) -> VmStep;
    pub fn resume(&mut self, reply: Reply);          // error if nothing pending / wrong kind
}

pub struct ScriptCtx<'a> {
    pub mem: &'a mut dyn ScriptMemory,
    pub rng: &'a mut dyn VmRng,
}

pub trait ScriptMemory {
    fn read(&mut self, addr: u16, origin: Origin) -> u16;
    fn write(&mut self, addr: u16, value: u16, origin: Origin);
    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString;
    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin);
}

// decoder — shared by interpreter, disassembler, census
pub fn decode(block: &EclBlock, addr: u16) -> Result<Instr, DecodeError>;
pub struct Instr { pub op: Op, pub args: Vec<Arg>, pub next: u16 }
pub enum Arg { ImmByte(u8), Mem(u16), MemAlt(u16), ImmWord(u16), InlineStr(..), MemStr(u16) }
```

The engine's run loop is then:

```rust
loop {
    match vm.step(&mut ctx) {
        VmStep::Continue => continue,
        VmStep::Effect(e) => present(e),               // and continue
        VmStep::Request(r) => return AwaitingUser(r),  // resume() on a later tick
        VmStep::Done(exit) => return Finished(exit),
    }
}
```

## 4. Conformance testing (H2)

- **Micro-ECL builder** (test-only assembler in `gbx-vm`): hand-construct synthetic
  blocks opcode-by-opcode — fully legal to ship. Every implemented opcode gets
  conformance programs asserting on the yielded step stream, memory traffic (via a
  mock ScriptMemory), flag state, and pc trajectory.
- **Suspension tests**: menus/inputs driven by scripted replies; assert state
  round-trips through suspend/resume (serialize the suspended VM, restore, resume —
  save-anywhere insurance from day one).
- **Unknown-access log** asserted empty for conformance programs; asserted *non-empty
  and precise* for deliberately-unknown-address programs.
- **Disassembler goldens**: same decoder, static sweep; later validated against
  ECLDump output on real data (H1, local-only).
- **Real-script replays** (later, data-gated): expected text/branch outcomes captured
  from DOSBox/instrumented coab.

## 5. Open questions → fidelity docket seeds

1. `0x1F` opcode semantics (unknown to coab). Census first; if unused in shipped
   CotAB scripts, no-op with rationale.
2. Operand mode `0x01` vs `0x03` — identical on read in coab; is there a write-path
   or width distinction? Pin against ECLDump + binary before the disassembler stabilizes.
3. Menu counts: are they ever non-immediate in shipped scripts? (Breaks static
   disassembly if so.)
4. Does PRINT ever block (pagination/"MORE")? Determines whether text stays an
   Effect or needs a Request variant. (M2 UI-shell design will inherit this.)
5. Exact inline-string compression (bit-packing) — `gbx-formats` work, verified
   against ECLDump text output.
6. Byte-exact operand/offset accounting in `vm_LoadCmdSets` (the +1/+2/wrap dance) —
   pinned by implementing against ECLDump goldens rather than derived from coab prose.

## 6. What this unblocks (M1 build order)

1. `decode()` + opcode table (names from coab) → **disassembler** → validated against
   EclDump.exe on real data when it lands (H1);
2. **census tool** on the same decoder (the project dashboard);
3. `EclVm::step()` + the ~15–25 most frequent opcodes (control flow, arithmetic,
   compare/IF, print, menus, memory ops) against micro-ECL conformance tests;
4. ScriptMemory facade with the window table above, raw fallback store, and
   unknown-access logging — engine-side implementation grows as M2/M3 name more cells.
