//! `EclMachine`: the resumable ECL interpreter (`docs/design/vm-scriptmemory.md`
//! §2 D-VM3, §3 API sketch). One machine holds exactly what the original
//! holds globally — the resident block, the parsed vector table, shared
//! compare flags / string registers / GOSUB call stack, and an activation
//! stack of `{pc, pending}` frames — and never blocks: `step()`/`resume()`
//! each execute (or continue) one instruction and return.
//!
//! Implements the census's (`docs/census/cotab-v1.3.md` §8) top-25 opcodes
//! plus the ride-alongs the task's docket calls out explicitly (PRINT
//! RETURN, COMPARE AND, LOAD FILES, NEWECL). Every other opcode is a loud,
//! poisoning halt (D-VM6), but the *reason* is now distinguished
//! (M1 run-script audit note): `VmError::UnknownOpcode` is the original's
//! own "no dialect entry" wedge (e.g. `0x41`, or any byte the CotAB
//! `CommandTable` never populated), while `VmError::Unimplemented` is a
//! Restrike-side gap — the dialect table knows the opcode (including
//! `0x1F`, which even coab leaves as a null handler) but this interpreter
//! hasn't grown a handler for it yet. Both halt identically from `step()`'s
//! perspective; the split exists so `restrike run-script`'s diagnostic can
//! tell "the original game would have wedged here too" apart from "our
//! interpreter's opcode coverage stops here."

use std::collections::VecDeque;

use crate::decode::{decode_operand, Arg, BlockBytes, ECL_BLOCK_BASE, ECL_BLOCK_SIZE};
use crate::dialect::Dialect;
use crate::host::{Effect, Origin, PlayerId, ProgramOutcome, Reply, Request, VmHost, VmString};

/// PARLAY (0x2C)'s five tones — a code-segment literal in the original
/// (`aHaughtySlyNice`, `ovr003:2806`), not script data, so the words are the
/// same at every site. Passed as plain options: the engine's own
/// `buildMenuStrings` transform re-marks the first letter of each, which
/// reproduces the original's `~`-prefixed literal exactly.
const PARLAY_TONES: [&[u8]; 5] = [b"HAUGHTY", b"SLY", b"NICE", b"MEEK", b"ABUSIVE"];

/// Identifies a script block for `Exit::ChainTo` (NEWECL/PROGRAM-8's target).
/// A raw `.dax`-file-relative block id, exactly as coab's `CMD_NewECL`
/// decodes it (`(byte)ovr008.vm_GetCmdValue(1)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BlockId(pub u8);

/// `load_block`'s failure type. Currently uninhabited: nothing about the
/// current CotAB header parse can fail (`read_header_vectors` never errors —
/// unresolved vectors decode to `None`, not an `Err`). Reserved for a future
/// dialect that needs to reject a malformed header outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {}

/// `EclMachine::restore`'s failure type (D-VM3: "unknown versions are
/// rejected, not migrated").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreError {
    UnknownVersion(u32),
}

/// The call-legality table's failure modes (`docs/design/vm-scriptmemory.md`
/// §3), plus two opcode-execution hazards this session's opcodes can hit:
/// `StringOperandTypeMismatch` (COMPARE AND / CHECKPARTY-class operand-mode
/// hazard, opcode-classification.md docket item 5) and `UnresolvedOperand`
/// (a destination/target operand with no resolvable raw word — the
/// original's `.Word` getter throwing on `highSet == false`). Both are
/// modeled as halting errors, matching `UnknownOpcode`'s "the machine is
/// halted" contract — after any of these, the offending activation's `pc`
/// does not move, so a repeated `step()` call reproduces the same error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmError {
    /// No dialect entry for this opcode byte at all — the original's own
    /// wedge (D-VM6). `dialect.lookup(opcode)` returned `None`.
    UnknownOpcode {
        pc: u16,
        opcode: u8,
    },
    /// The dialect table has an entry for this opcode (including `0x1F`,
    /// coab's own null-handler case), but this interpreter has no handler
    /// for it yet — a Restrike coverage gap, not an original-engine wedge.
    Unimplemented {
        pc: u16,
        opcode: u8,
    },
    StepWhilePending,
    ResumeWithoutPending,
    ReplyMismatch,
    Idle,
    /// The original's `Opperation.GetCmdValue()` throws when called on a
    /// string-mode (`Code>=0x80`) operand outside `COMPARE`'s own
    /// `Code>=0x80`-guarded string path (`Classes/Opperation.cs:98-130`).
    /// COMPARE AND (`ovr003.cs:438-461`) and CHECKPARTY call it
    /// unconditionally on every operand — opcode-classification.md docket
    /// item 5.
    StringOperandTypeMismatch {
        pc: u16,
        opcode: u8,
    },
    /// A destination/target operand's `Arg::raw_word()` was `None` — the
    /// original's `.Word` getter throws in the same situation.
    UnresolvedOperand {
        pc: u16,
        opcode: u8,
    },
    /// LOAD MONSTER (0x0B) with a missing `.dax` asset: the original's hard
    /// `print_and_exit()` (`ovr017.cs:836-838`, opcode-classification.md
    /// docket item 4), modeled as a halting `VmError` rather than aborting
    /// the host process.
    MissingAsset {
        pc: u16,
        opcode: u8,
    },
    /// DIVIDE (0x06) with a zero divisor: coab's `CMD_AddSubDivMulti` (case 6,
    /// `ovr003.cs:111-114`) computes `val_a / val_b` with C#'s integer `/`,
    /// which throws `DivideByZeroException` uncaught by any handler up the
    /// `RunEclVm` call chain (`ovr003.cs:2147-2227` has no `try`/`catch`) —
    /// the original crashes. Modeled as a halting `VmError`, the same
    /// non-aborting analogue used for LOAD MONSTER's missing-asset crash.
    DivisionByZero {
        pc: u16,
        opcode: u8,
    },
}

/// How an activation ends (D-VM3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Exit {
    Ended,
    ChainTo(BlockId),
}

/// One `step()`/`resume()` result.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VmStep {
    Continue,
    Effect(Effect),
    Request(Request),
    Done(Exit),
}

/// What happens once a `Pending`'s effect queue (and optional trailing
/// request) is fully drained.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Completion {
    Advance(u16),
    WriteWordThenAdvance {
        dest: u16,
        next: u16,
    },
    /// ★ ENCOUNTER MENU (0x29)'s `do { … } while (init_max != 0)` loop
    /// (`ovr003.cs:1281-1531`): the only shipped opcode whose own body re-opens
    /// a menu. The reply is resolved against the operand-borne outcome table
    /// and either ends the instruction or arms the next iteration, so this
    /// carries the decoded operands with it — the original keeps them in stack
    /// locals across the loop, and `vm_LoadCmdSets` has long since advanced
    /// past the bytes.
    EncounterMenu(Box<EncounterMenuState>),
    /// ★ PARLAY (0x2C)'s write (`talk_style`, `ovr003:2837-284D`): the reply
    /// **indexes an operand-borne table** and the table's entry is what
    /// reaches memory, so the selection itself is never written. Five entries
    /// for five tones.
    WriteToneOutcomeThenAdvance {
        dest: u16,
        values: [u8; 5],
        next: u16,
    },
}

/// ENCOUNTER MENU (0x29)'s decoded operands, held across the menu loop. Field
/// names follow `CMD_EncounterMenu`'s own locals where the original has no
/// better name (`ovr003.cs:1227-1269`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct EncounterMenuState {
    /// `var_43D` — `gbl.cmd_opps[4].Word` (`:1256`), the result cell's raw
    /// address, taken like every other destination operand.
    dest: u16,
    /// `var_6[0..5]` (`:1258-1261`), operands 5..=9: the outcome CLASS for each
    /// of the five menu slots. Indexed by the *resolved* selection, and it is
    /// the class — not the slot — that decides what happens, so the same word
    /// means different things in different encounters.
    outcomes: [u8; 5],
    /// `strings[0..3]` (`:1263-1266`) — string registers 1..=3, i.e. operands
    /// 10, 11 and 12. One approach line per distance band, scanned cyclically
    /// from the current band (`:1298-1339`).
    texts: [VmString; 3],
    /// `var_407` (`:1268`, operand 13): the party's SLOWEST member must reach
    /// this to get away (`init_min >= var_407`, `:1384`).
    party_flee_movement: u8,
    /// `var_408` (`:1269`, operand 14): the monsters break off if this beats
    /// the party's FASTEST member (`var_408 > var_40A`, `:1442`).
    monster_flee_movement: u8,
    /// `init_min` / `var_40A` — `calc_group_movement`'s (slowest, fastest),
    /// sampled ONCE before the loop (`:1250`), so a party hasted mid-menu would
    /// not benefit. Verbatim.
    slowest: u8,
    fastest: u8,
    next: u16,
}

/// Per-opcode continuation state (`docs/design/vm-scriptmemory.md` §3):
/// which phase of a multi-step instruction, and what completes it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum PendingState {
    /// Mid-instruction: more `Effect`s (and optionally one trailing
    /// `Request`) before this instruction completes. `step()` remains legal
    /// here — distinguishable from `AwaitingReply`.
    Effects {
        queue: VecDeque<Effect>,
        request_after: Option<Request>,
        completion: Completion,
    },
    /// Suspended awaiting a reply. `step()` is illegal (`StepWhilePending`);
    /// only `resume()` with a matching reply completes the instruction.
    AwaitingReply {
        request: Request,
        completion: Completion,
    },
}

/// Per-opcode continuation: which phase, plus the originating instruction's
/// address (for `Origin` on any memory access the completion performs).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Pending {
    pc: u16,
    state: PendingState,
}

/// One activation frame: `{pc, pending}` (`docs/design/vm-scriptmemory.md`
/// §3 API sketch, verbatim).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Activation {
    pc: u16,
    pending: Option<Pending>,
}

/// The 15-slot persistent string register file (`gbl.unk_1D972`) — never
/// bulk-cleared between instructions (`docs/design/vm-scriptmemory.md` §1).
/// 1-indexed to match coab's `strIndex`/`cmd_opps` convention directly.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StringRegs {
    slots: [VmString; 15],
}

impl Default for StringRegs {
    fn default() -> Self {
        StringRegs {
            slots: std::array::from_fn(|_| VmString::default()),
        }
    }
}

impl StringRegs {
    fn get(&self, index: u8) -> &VmString {
        &self.slots[(index - 1) as usize]
    }

    fn set(&mut self, index: u8, value: VmString) {
        self.slots[(index - 1) as usize] = value;
    }
}

fn in_block(addr: u16) -> bool {
    let block_end = ECL_BLOCK_BASE.wrapping_add(ECL_BLOCK_SIZE as u16);
    (ECL_BLOCK_BASE..block_end).contains(&addr)
}

/// A save-anywhere snapshot of one `EclMachine` (D-VM3): the resident block,
/// parsed vectors, shared flags/strings/call-stack, and the full activation
/// stack including any suspended `Pending`s — re-presented verbatim by
/// `pending()` after `restore`, never re-derived. Carries an explicit
/// version tag; unknown versions are rejected outright, not migrated.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    // `pub(crate)` rather than fully private: the conformance suite
    // (`conformance.rs`, a sibling module) constructs a deliberately
    // corrupted version tag to exercise `RestoreError::UnknownVersion`
    // without a public "poke the version" API surface.
    pub(crate) version: u32,
    block: BlockBytes,
    vectors: Vec<Option<u16>>,
    flags: [bool; 6],
    strings: StringRegs,
    call_stack: Vec<u16>,
    runs: Vec<Activation>,
}

const SNAPSHOT_VERSION: u32 = 1;

/// The resumable ECL interpreter (`docs/design/vm-scriptmemory.md` §3).
///
/// Holds a `'static` dialect reference (set at `load_block`/`restore` time)
/// so `step()`/`resume()` — whose signatures are fixed by the API sketch to
/// take only a host, no dialect — can still consult per-opcode `skip_size`
/// for the IF family's skip path without threading a dialect through every
/// call. Every dialect this crate ships (`crate::dialect::COTAB`) is a
/// `'static` table, so this costs nothing in practice.
#[derive(Debug)]
pub struct EclMachine {
    dialect: &'static Dialect,
    block: BlockBytes,
    vectors: Vec<Option<u16>>,
    flags: [bool; 6],
    strings: StringRegs,
    call_stack: Vec<u16>,
    runs: Vec<Activation>,
}

impl EclMachine {
    /// Loads (or switches to) a resident block: parses the dialect's header
    /// vectors, and resets the shared call stack + compare flags exactly as
    /// coab's `vm_init_ecl` does on every block load/switch
    /// (`docs/design/vm-scriptmemory.md` §1) — string registers are *not*
    /// reset here (process-global, they persist across block switches too).
    /// The activation stack is left as-is: callers driving a fresh machine
    /// start with an empty stack; callers chaining after `Exit::ChainTo`
    /// abandon the old stack themselves before calling this (D-VM3).
    pub fn load_block(block: BlockBytes, dialect: &'static Dialect) -> Result<Self, HeaderError> {
        let (vectors, _) = crate::decode::read_header_vectors(&block, dialect.vector_count);
        Ok(EclMachine {
            dialect,
            block,
            vectors,
            flags: [false; 6],
            strings: StringRegs::default(),
            call_stack: Vec::new(),
            runs: Vec::new(),
        })
    }

    /// Walk-loop re-entry without a reload (`docs/design/vm-scriptmemory.md`
    /// §1): re-parses vectors from the (possibly self-modified) resident
    /// bytes and clears flags + call stack, but keeps the same block bytes
    /// and leaves the activation stack untouched.
    pub fn reinit(&mut self) {
        let (vectors, _) =
            crate::decode::read_header_vectors(&self.block, self.dialect.vector_count);
        self.vectors = vectors;
        self.flags = [false; 6];
        self.call_stack.clear();
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: SNAPSHOT_VERSION,
            block: self.block.clone(),
            vectors: self.vectors.clone(),
            flags: self.flags,
            strings: self.strings.clone(),
            call_stack: self.call_stack.clone(),
            runs: self.runs.clone(),
        }
    }

    /// Restores a machine from a snapshot. The dialect is re-bound here,
    /// never embedded in the snapshot (`docs/design/vm-scriptmemory.md` §2).
    pub fn restore(snapshot: Snapshot, dialect: &'static Dialect) -> Result<Self, RestoreError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(RestoreError::UnknownVersion(snapshot.version));
        }
        Ok(EclMachine {
            dialect,
            block: snapshot.block,
            vectors: snapshot.vectors,
            flags: snapshot.flags,
            strings: snapshot.strings,
            call_stack: snapshot.call_stack,
            runs: snapshot.runs,
        })
    }

    /// Pushes a fresh activation (a vector run, or a nested run) — always
    /// legal, even while an outer activation sits suspended mid-instruction
    /// (the PROGRAM-9 camp case, D-VM3).
    pub fn enter(&mut self, addr: u16) {
        self.runs.push(Activation {
            pc: addr,
            pending: None,
        });
    }

    /// The dialect-defined header vector at `index`, or `None` if out of
    /// range or unresolved at load time.
    pub fn vector(&self, index: usize) -> Option<u16> {
        self.vectors.get(index).copied().flatten()
    }

    /// The top activation's outstanding request, if it's suspended awaiting
    /// a reply. `None` if the machine is idle or mid-instruction (more
    /// effects coming, not yet a `Request`).
    pub fn pending(&self) -> Option<&Request> {
        let pending = self.runs.last()?.pending.as_ref()?;
        match &pending.state {
            PendingState::AwaitingReply { request, .. } => Some(request),
            PendingState::Effects { .. } => None,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.runs.is_empty()
    }

    /// The top activation's program counter, for conformance tests asserting
    /// on pc trajectory (`docs/design/vm-scriptmemory.md` §4). `None` if the
    /// machine is idle.
    pub fn current_pc(&self) -> Option<u16> {
        self.runs.last().map(|a| a.pc)
    }

    /// The six relation flags (`==, !=, <, >, <=, >=`), for conformance
    /// tests asserting on flag state directly (§4) instead of only through
    /// an `IF`'s branch behavior.
    pub fn flags(&self) -> [bool; 6] {
        self.flags
    }

    /// Executes (or continues) one instruction of the top activation.
    pub fn step(&mut self, host: &mut dyn VmHost) -> Result<VmStep, VmError> {
        let Some(mut activation) = self.runs.pop() else {
            return Err(VmError::Idle);
        };
        let result = self.run_activation(&mut activation, host);
        self.reconcile(activation, &result);
        result
    }

    /// Completes a suspended instruction with `reply`.
    pub fn resume(&mut self, reply: Reply, host: &mut dyn VmHost) -> Result<VmStep, VmError> {
        match self.runs.last() {
            None => return Err(VmError::ResumeWithoutPending),
            Some(top) => match &top.pending {
                Some(Pending {
                    state: PendingState::AwaitingReply { request, .. },
                    ..
                }) => {
                    if !reply.matches(request) {
                        return Err(VmError::ReplyMismatch);
                    }
                }
                _ => return Err(VmError::ResumeWithoutPending),
            },
        }

        let mut activation = self.runs.pop().expect("checked above");
        let pending = activation.pending.take().expect("checked above");
        let PendingState::AwaitingReply { completion, .. } = pending.state else {
            unreachable!("checked above");
        };
        let result =
            self.apply_completion(&mut activation, completion, Some(reply), host, pending.pc);
        self.reconcile(activation, &result);
        result
    }

    /// Pushes `activation` back unless the instruction ended the run
    /// (`Done`): `Exit::Ended` simply pops it; `Exit::ChainTo` abandons the
    /// *entire* stack (D-VM3: "no VM context ever resumes across a chain").
    /// An `Err` also leaves the activation off the stack only if it was
    /// never popped in the first place — errors reproduce deterministically
    /// by construction, since the pc never advances past a failing
    /// instruction (see each opcode handler).
    fn reconcile(&mut self, activation: Activation, result: &Result<VmStep, VmError>) {
        match result {
            Ok(VmStep::Done(Exit::ChainTo(_))) => self.runs.clear(),
            Ok(VmStep::Done(Exit::Ended)) => {}
            _ => self.runs.push(activation),
        }
    }

    fn run_activation(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
    ) -> Result<VmStep, VmError> {
        if activation.pending.is_some() {
            self.drain_pending(activation, host)
        } else {
            self.dispatch(activation, host)
        }
    }

    fn drain_pending(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
    ) -> Result<VmStep, VmError> {
        let pending = activation.pending.as_mut().expect("checked by caller");
        match &mut pending.state {
            PendingState::AwaitingReply { .. } => Err(VmError::StepWhilePending),
            PendingState::Effects {
                queue,
                request_after,
                completion,
            } => {
                if let Some(effect) = queue.pop_front() {
                    return Ok(VmStep::Effect(effect));
                }
                if let Some(request) = request_after.take() {
                    pending.state = PendingState::AwaitingReply {
                        request: request.clone(),
                        completion: completion.clone(),
                    };
                    return Ok(VmStep::Request(request));
                }
                let completion = completion.clone();
                let origin_pc = pending.pc;
                activation.pending = None;
                self.apply_completion(activation, completion, None, host, origin_pc)
            }
        }
    }

    fn apply_completion(
        &mut self,
        activation: &mut Activation,
        completion: Completion,
        reply: Option<Reply>,
        host: &mut dyn VmHost,
        origin_pc: u16,
    ) -> Result<VmStep, VmError> {
        match completion {
            Completion::Advance(next) => {
                activation.pc = next;
                Ok(VmStep::Continue)
            }
            Completion::WriteWordThenAdvance { dest, next } => {
                let value = match reply {
                    Some(Reply::Selection(v)) => v as u16,
                    _ => 0,
                };
                self.mem_write(dest, value, host, Origin { pc: origin_pc });
                activation.pc = next;
                Ok(VmStep::Continue)
            }
            Completion::WriteToneOutcomeThenAdvance { dest, values, next } => {
                // `ovr003:2837-2841` — `var_2 = var_8[selection]`. A reply
                // outside 0..5 cannot happen (the menu has five words), but
                // the original would have read a stack byte past its array;
                // clamping to the last entry is the closest safe analogue.
                let selection = match reply {
                    Some(Reply::Selection(v)) => (v as usize).min(values.len() - 1),
                    _ => 0,
                };
                let value = values[selection] as u16;
                self.mem_write(dest, value, host, Origin { pc: origin_pc });
                activation.pc = next;
                Ok(VmStep::Continue)
            }
            Completion::EncounterMenu(state) => {
                let selection = match reply {
                    Some(Reply::Selection(v)) => v,
                    _ => 0,
                };
                self.encounter_menu_reply(activation, host, origin_pc, *state, selection)
            }
        }
    }

    // --- ScriptMemory routing (D-VM5): the VM intercepts its own Ecl
    // window (read-only — self-modifying writes are out of scope this
    // session, see `mem_write`'s doc comment) before delegating to the host.

    /// ★ **Script-space reads are BYTE-wide** (roll-credits slice 7's
    /// correction).
    ///
    /// `vm_GetMemoryValueType` gives `0x8000..=0x9DFF` its own memory class
    /// (`ovr008.cs:319-322`) and `vm_GetMemoryValue`'s arm for that class
    /// returns `gbl.ecl_ptr[…]` (`:848`) — and `EclBlock`'s indexer is
    /// `public byte this[int index]` (`Classes/EclBlock.cs:31`). One byte,
    /// widened. (coab's own expression there is `ecl_ptr[loc + 0x8000]`, which
    /// would index past the 0x1E00-byte buffer; the `// When does this
    /// happen?` beside it says the decompiler did not resolve this arm. The
    /// *width* is not in doubt — the indexer's return type settles it.)
    ///
    /// M2 read a little-endian WORD here, which no shipped table survives:
    /// `ECL1#80`'s overland route tables are 56 consecutive bytes each, and a
    /// word read of `0x9C02` returns `0x0201` where the script needs `1`
    /// (`@0x8FCE GETTABLE 0x9C02,[0x7F79],[0x4C02]`, whose result feeds an
    /// `ON GOTO … #0x0E`). Every table read in the game is byte-indexed for
    /// the same reason: overlapping word reads at consecutive indices could
    /// not encode a table at all.
    fn mem_read(&self, addr: u16, host: &mut dyn VmHost, origin: Origin) -> u16 {
        if in_block(addr) {
            u16::from(self.block.get(addr))
        } else {
            host.read(addr, origin)
        }
    }

    /// Writes never intercept the Ecl window: `BlockBytes` is intentionally
    /// read-only (self-modifying scripts are documented original behavior,
    /// `docs/design/vm-scriptmemory.md` §1, but implementing a mutable
    /// resident block is out of this session's scope — no opcode this
    /// session's conformance suite exercises targets a script-address
    /// destination, and the census found no self-modification in reachable
    /// CotAB regions either). A write to an in-block address currently just
    /// reaches the host like any other window; flagged here for a future
    /// docket entry rather than silently "handled."
    fn mem_write(&mut self, addr: u16, value: u16, host: &mut dyn VmHost, origin: Origin) {
        host.write(addr, value, origin);
    }

    fn mem_write_string(&mut self, addr: u16, s: &VmString, host: &mut dyn VmHost, origin: Origin) {
        host.write_string(addr, s, origin);
    }

    /// The original's `Opperation.GetCmdValue()`: immediate operands resolve
    /// to their literal value, `Mem`/`MemAlt` resolve through `ScriptMemory`,
    /// and string-mode operands (`InlineStr`/`UnknownMode`) throw in the
    /// original (`Classes/Opperation.cs:98-130`) — surfaced here as
    /// `VmError::StringOperandTypeMismatch`. `MemStr` (mode `0x81`) *does*
    /// set `highSet` in the original (`ovr008.cs:57-71`), so `GetCmdValue`
    /// returns its raw address rather than throwing — included for
    /// completeness even though no opcode this session calls it that way.
    fn resolve_numeric(
        &self,
        arg: &Arg,
        pc: u16,
        opcode: u8,
        host: &mut dyn VmHost,
    ) -> Result<u16, VmError> {
        match arg {
            Arg::ImmByte(b) => Ok(*b as u16),
            Arg::ImmWord(w) => Ok(*w),
            Arg::Mem(addr) | Arg::MemAlt(addr) => Ok(self.mem_read(*addr, host, Origin { pc })),
            Arg::MemStr(addr) => Ok(*addr),
            Arg::InlineStr(_) | Arg::UnknownMode { .. } => {
                Err(VmError::StringOperandTypeMismatch { pc, opcode })
            }
        }
    }

    fn resolve_target(&self, arg: &Arg, pc: u16, opcode: u8) -> Result<u16, VmError> {
        arg.raw_word()
            .ok_or(VmError::UnresolvedOperand { pc, opcode })
    }

    fn is_string_mode(arg: &Arg) -> bool {
        matches!(arg, Arg::InlineStr(_) | Arg::MemStr(_))
    }

    /// Decodes `count` operand batches starting at `cursor`, performing the
    /// same side effects coab's `vm_LoadCmdSets` performs regardless of
    /// whether a real handler ends up using the decoded values: string-mode
    /// operands fill the string registers (`0x81` additionally reads through
    /// `ScriptMemory`); numeric modes have no decode-time side effect (the
    /// original only resolves them later, per-operand, via `GetCmdValue`).
    /// `strIndex` resets to 0 for each call, exactly like the original's
    /// local variable — callers doing a fixed-prefix-then-tail decode
    /// (variable-tail opcodes) call this twice, each with its own reset.
    ///
    /// `0x80` inline strings are decompressed here (task 1, ECL
    /// inline-string decompression — coab `LoadCompressedEclString` runs
    /// this at the exact same decode-time point, `ovr008.cs:39-56`) via
    /// `gbx_formats::ecl_text::decompress`. `0x81` memory strings are
    /// *never* decompressed — they're already plain ASCII on the wire
    /// (`gbx-formats/src/ecl_text.rs`'s module doc); `host.read_string`
    /// returns them as-is.
    fn load_cmd_sets(
        &mut self,
        mut cursor: u16,
        count: u8,
        host: &mut dyn VmHost,
        origin_pc: u16,
    ) -> (Vec<Arg>, u16) {
        let mut str_index: u8 = 0;
        let mut args = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let (arg, next) = decode_operand(&self.block, cursor);
            cursor = next;
            match &arg {
                Arg::InlineStr(raw) => {
                    str_index += 1;
                    self.strings
                        .set(str_index, VmString(gbx_formats::ecl_text::decompress(raw)));
                }
                Arg::MemStr(addr) => {
                    str_index += 1;
                    let s = host.read_string(*addr, Origin { pc: origin_pc });
                    self.strings.set(str_index, s);
                }
                _ => {}
            }
            args.push(arg);
        }
        (args, cursor)
    }

    fn yield_effect(activation: &mut Activation, pc: u16, effect: Effect, next: u16) -> VmStep {
        activation.pending = Some(Pending {
            pc,
            state: PendingState::Effects {
                queue: VecDeque::new(),
                request_after: None,
                completion: Completion::Advance(next),
            },
        });
        VmStep::Effect(effect)
    }

    /// [`Self::yield_effect`] for an ordered batch: the first effect is
    /// returned now, the rest queue behind it, and an optional trailing
    /// request follows them. An empty batch with no request completes
    /// immediately — but every caller here has at least one effect.
    fn yield_effects(
        activation: &mut Activation,
        pc: u16,
        mut queue: VecDeque<Effect>,
        request_after: Option<Request>,
        completion: Completion,
    ) -> VmStep {
        let first = queue.pop_front();
        activation.pending = Some(Pending {
            pc,
            state: PendingState::Effects {
                queue,
                request_after,
                completion,
            },
        });
        match first {
            Some(effect) => VmStep::Effect(effect),
            // Nothing to present: fall through to the pending drain on the
            // next step, which will surface the request (or the completion).
            None => VmStep::Continue,
        }
    }

    fn yield_effect_then_request(
        activation: &mut Activation,
        pc: u16,
        effect: Effect,
        request: Request,
        completion: Completion,
    ) -> VmStep {
        activation.pending = Some(Pending {
            pc,
            state: PendingState::Effects {
                queue: VecDeque::new(),
                request_after: Some(request),
                completion,
            },
        });
        VmStep::Effect(effect)
    }

    fn yield_request(
        activation: &mut Activation,
        pc: u16,
        request: Request,
        completion: Completion,
    ) -> VmStep {
        activation.pending = Some(Pending {
            pc,
            state: PendingState::AwaitingReply {
                request: request.clone(),
                completion,
            },
        });
        VmStep::Request(request)
    }

    /// Sets all six relation flags from `left OP right` — the natural
    /// operand-order convention every implemented flag-setting opcode
    /// reduces to once coab's double-swapped `compare_variables(arg_0,
    /// arg_2)` argument order is unwound (see `op_compare`/`op_and_or`'s doc
    /// comments for the specific call-site derivation).
    fn set_compare_flags(&mut self, left: u16, right: u16) {
        self.flags = [
            left == right,
            left != right,
            left < right,
            left > right,
            left <= right,
            left >= right,
        ];
    }

    fn set_compare_flags_bytes(&mut self, left: &[u8], right: &[u8]) {
        use std::cmp::Ordering;
        let ord = left.cmp(right);
        self.flags = [
            ord == Ordering::Equal,
            ord != Ordering::Equal,
            ord == Ordering::Less,
            ord == Ordering::Greater,
            ord != Ordering::Greater,
            ord != Ordering::Less,
        ];
    }

    fn dispatch(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
    ) -> Result<VmStep, VmError> {
        let pc = activation.pc;
        let opcode = self.block.get(pc);
        match opcode {
            0x00 => self.op_exit(activation),
            0x01 => self.op_goto(activation, host, pc, opcode),
            0x02 => self.op_gosub(activation, host, pc, opcode),
            0x03 => self.op_compare(activation, host, pc, opcode),
            0x04 => self.op_add(activation, host, pc, opcode),
            0x05 => self.op_subtract(activation, host, pc, opcode),
            0x06 => self.op_divide(activation, host, pc, opcode),
            0x07 => self.op_multiply(activation, host, pc, opcode),
            0x08 => self.op_random(activation, host, pc, opcode),
            0x09 => self.op_save(activation, host, pc, opcode),
            0x0A => self.op_load_character(activation, host, pc, opcode),
            0x0B => self.op_load_monster(activation, host, pc, opcode),
            0x0C => self.op_setup_monster(activation, host, pc, opcode),
            0x0D => self.op_approach(activation, host, pc),
            0x0E => self.op_picture(activation, host, pc, opcode),
            0x11 => self.op_print(activation, host, pc, opcode, false),
            0x12 => self.op_print(activation, host, pc, opcode, true),
            0x13 => self.op_return(activation),
            0x14 => self.op_compare_and(activation, host, pc, opcode),
            0x15 => self.op_vertical_menu(activation, host, pc, opcode),
            0x16..=0x1B => self.op_if(activation, host, pc, opcode),
            0x1C => self.op_clearmonsters(activation, host),
            0x1D => self.op_party_strength(activation, host, pc, opcode),
            0x20 => self.op_newecl(activation, host, pc, opcode),
            0x21 => self.op_load_files(activation, host, pc, opcode, false),
            0x24 => self.op_combat(activation),
            0x25 => self.op_on_goto(activation, host, pc, opcode),
            0x26 => self.op_on_gosub(activation, host, pc, opcode),
            0x29 => self.op_encounter_menu(activation, host, pc, opcode),
            0x2A => self.op_gettable(activation, host, pc, opcode),
            0x2B => self.op_horizontal_menu(activation, host, pc, opcode),
            0x27 => self.op_treasure(activation, host, pc, opcode),
            0x2C => self.op_parlay(activation, host, pc, opcode),
            0x2D => self.op_call(activation, host, pc, opcode),
            0x2E => self.op_damage(activation, host, pc, opcode),
            0x2F => self.op_and(activation, host, pc, opcode),
            0x30 => self.op_or(activation, host, pc, opcode),
            0x31 => self.op_sprite_off(activation, host, pc),
            0x32 => self.op_find_item(activation, host, pc, opcode),
            0x33 => self.op_print_return(activation),
            0x34 => self.op_ecl_clock(activation, host, pc, opcode),
            0x35 => self.op_save_table(activation, host, pc, opcode),
            0x36 => self.op_add_npc(activation, host, pc, opcode),
            0x37 => self.op_load_files(activation, host, pc, opcode, true),
            0x38 => self.op_program(activation, host, pc, opcode),
            0x3A => self.op_delay(activation),
            0x3D => self.op_clear_box(activation, pc),
            0x3E => self.op_dump(activation, host, pc),
            0x3F => self.op_find_special(activation, host, pc, opcode),
            0x40 => self.op_destroy_items(activation, host, pc, opcode),
            _ if self.dialect.lookup(opcode).is_some() => {
                Err(VmError::Unimplemented { pc, opcode })
            }
            _ => Err(VmError::UnknownOpcode { pc, opcode }),
        }
    }

    // --- Opcode implementations, ordered per the census's frequency list
    // (`docs/census/cotab-v1.3.md` §8) plus the docket ride-alongs. Each
    // handler's citation is to the coab source read for this session
    // (`engine/ovr003.cs` unless noted).

    /// EXIT (0x00), `CMD_Exit` ovr003.cs:9-42. `SelectedPlayer` restoration
    /// and the text-cursor reset are engine-owned presentation state with no
    /// `ScriptMemory` address — out of `gbx-vm`'s model. `vmCallStack.Clear()`
    /// is the one piece of *our* state EXIT actually touches.
    fn op_exit(&mut self, _activation: &mut Activation) -> Result<VmStep, VmError> {
        self.call_stack.clear();
        Ok(VmStep::Done(Exit::Ended))
    }

    /// GOTO (0x01), `CMD_Goto` ovr003.cs:45-53. The target is the operand's
    /// raw `.Word` — coab reads `cmd_opps[1].Word` directly, never through
    /// `GetCmdValue()` (docket item 3: destination/target operands never
    /// dereference).
    fn op_goto(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, _next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let target = self.resolve_target(&args[0], pc, opcode)?;
        activation.pc = target;
        Ok(VmStep::Continue)
    }

    /// GOSUB (0x02), `CMD_Gosub` ovr003.cs:56-65. Pushes the fall-through
    /// address (`next`, i.e. coab's already-advanced `ecl_offset` at push
    /// time) as the eventual RETURN's landing site.
    fn op_gosub(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let target = self.resolve_target(&args[0], pc, opcode)?;
        self.call_stack.push(next);
        activation.pc = target;
        Ok(VmStep::Continue)
    }

    /// COMPARE (0x03), `CMD_Compare` ovr003.cs:68-87. String path compares
    /// slots `[1]`/`[2]` whenever *either* operand is string-mode — a mixed
    /// compare reads one stale slot by construction (`docs/design/
    /// vm-scriptmemory.md` §1). Flag order derived from
    /// `compare_variables(value_b, value_a)`'s double-swapped argument names
    /// (`arg_0=value_b, arg_2=value_a`, flags set from `arg_2 OP arg_0`) —
    /// unwinds to the natural `operand1 OP operand2` convention.
    fn op_compare(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        if Self::is_string_mode(&args[0]) || Self::is_string_mode(&args[1]) {
            let a = self.strings.get(1).0.clone();
            let b = self.strings.get(2).0.clone();
            self.set_compare_flags_bytes(&a, &b);
        } else {
            let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
            let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
            self.set_compare_flags(a, b);
        }
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// ADD (0x04), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 4. Destination
    /// is the raw `.Word` of operand 3 (never `GetCmdValue`'d).
    fn op_add(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        let value = a.wrapping_add(b);
        self.mem_write(dest, value, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// SUBTRACT (0x05), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 5. Result
    /// is `operand2 - operand1` (B−A), not A−B (`ovr003.cs:107`:
    /// `value = (ushort)(val_b - val_a)`).
    fn op_subtract(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        let value = b.wrapping_sub(a);
        self.mem_write(dest, value, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// DIVIDE (0x06), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 6:
    /// `value = val_a / val_b; gbl.area2_ptr.field_67E = val_a % val_b`. A
    /// zero divisor throws uncaught in coab (`VmError::DivisionByZero`, see
    /// its doc comment). The remainder bypasses `vm_SetMemoryValue` in the
    /// original (a direct `field_67E` struct write) but `Area2.field_800_Get`
    /// maps that same struct offset back onto Party-window address
    /// **`0x7F3F`** (opcode-classification.md docket item 2, confirmed by a
    /// live example: `ECL2.DAX` block 1's `0x8295: DIVIDE mem=0x7F7B, imm=0x08
    /// -> mem=0x7F80` feeds `0x829E: GETTABLE base=0x9DB8 index=mem[0x7F3F]`).
    /// Writing the remainder through the ordinary `mem_write` facade at
    /// `0x7F3F` reproduces that alias for any host without a special case.
    fn op_divide(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        if b == 0 {
            return Err(VmError::DivisionByZero { pc, opcode });
        }
        let value = a / b;
        let remainder = a % b;
        self.mem_write(dest, value, host, Origin { pc });
        self.mem_write(0x7F3F, remainder, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// MULTIPLY (0x07), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 7.
    fn op_multiply(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        let value = a.wrapping_mul(b);
        self.mem_write(dest, value, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// RANDOM (0x08), `CMD_Random` ovr003.cs:132-151. The inclusive-bound
    /// adjustment (`rand_max` incremented unless already `0xFF`) happens
    /// here, in the opcode, not inside `EngineServices::roll`.
    fn op_random(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let rand_max = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        // `if (rand_max < 0xff) rand_max++;` (ovr003.cs:138-141) — a
        // saturating increment.
        let rand_max = rand_max.saturating_add(1);
        let dest = self.resolve_target(&args[1], pc, opcode)?;
        let val = host.roll(rand_max);
        self.mem_write(dest, val as u16, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// SAVE (0x09), `CMD_Save` ovr003.cs:153-172. Branches on operand 1's
    /// mode: numeric writes through `vm_SetMemoryValue`, string writes the
    /// register slot operand 1 itself just filled (not stale — it's this
    /// instruction's own operand).
    fn op_save(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let loc = self.resolve_target(&args[1], pc, opcode)?;
        if Self::is_string_mode(&args[0]) {
            let s = self.strings.get(1).clone();
            self.mem_write_string(loc, &s, host, Origin { pc });
        } else {
            let val = self.resolve_numeric(&args[0], pc, opcode, host)?;
            self.mem_write(loc, val, host, Origin { pc });
        }
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// LOAD MONSTER (0x0B), `CMD_LoadMonster` ovr003.cs:238-297. Bundles all
    /// 3 operands into one `EngineServices` call (see `host.rs`'s trait doc
    /// comment); a missing `.dax` asset halts the machine
    /// (`VmError::MissingAsset`) rather than silently continuing.
    fn op_load_monster(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let monster_id = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let num_copies = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let icon_block_id = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;
        host.load_monster(monster_id, num_copies, icon_block_id)
            .map_err(|_| VmError::MissingAsset { pc, opcode })?;
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// SETUP MONSTER (0x0C), `CMD_SetupMonster` ovr003.cs:215-236, in full:
    /// the three ids are stashed engine-side (`:225-227`), the approach ray is
    /// cast and clamped into `area2_ptr.encounter_distance` (`:229-233`), and
    /// the encounter visual is dispatched (`:235`).
    ///
    /// The clamp is the original's `if (max < dist) dist = max`, i.e. `min` —
    /// note it is one-sided, so a `max_distance` operand *larger* than the ray
    /// leaves the ray's own value standing.
    fn op_setup_monster(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let sprite_id = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let max_distance = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let pic_id = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;
        host.setup_monster(sprite_id, max_distance as u16, pic_id);
        let distance = host.approach_distance().min(max_distance);
        host.set_encounter_distance(distance);
        host.load_encounter_visual();
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::EncounterVisual,
            next,
        ))
    }

    /// APPROACH (0x0D), `CMD_Approach` ovr003.cs:300-310. One step closer:
    /// decrement `area2_ptr.encounter_distance` and re-dispatch the encounter
    /// visual at the new band. At distance 0 the whole body is skipped — but
    /// `ecl_offset++` runs either way (`:309`, outside the `if`).
    fn op_approach(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
    ) -> Result<VmStep, VmError> {
        let next = pc.wrapping_add(1);
        let distance = host.encounter_distance();
        if distance == 0 {
            activation.pc = next;
            return Ok(VmStep::Continue);
        }
        host.set_encounter_distance(distance - 1);
        host.load_encounter_visual();
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::EncounterVisual,
            next,
        ))
    }

    /// SPRITE OFF (0x31), `CMD_SpriteOff` ovr003.cs:1707-1717. The service
    /// does the check-and-clear at execution time (like the `0xAE11` gate);
    /// the `RedrawView()` it guards travels as an effect.
    fn op_sprite_off(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
    ) -> Result<VmStep, VmError> {
        let next = pc.wrapping_add(1);
        if host.sprite_off() {
            Ok(Self::yield_effect(activation, pc, Effect::RedrawView, next))
        } else {
            activation.pc = next;
            Ok(VmStep::Continue)
        }
    }

    /// ★ ENCOUNTER MENU (0x29), `CMD_EncounterMenu` ovr003.cs:1227-1538 — the
    /// classic Gold Box COMBAT/WAIT/FLEE/PARLAY decision, and the one shipped
    /// opcode that loops around its own menu.
    ///
    /// **Fourteen operands** (`vm_LoadCmdSets(0x0e)`, `:1251`):
    ///
    /// | # | `:line` | meaning |
    /// |---|---|---|
    /// | 1 | `:1253` | `sprite_block_id` (`SPRIT{area}` block) |
    /// | 2 | `:1254` | `max_encounter_distance` — **not** byte-cast here, unlike SETUP MONSTER's `:220` |
    /// | 3 | `:1255` | `pic_block_id` (also the BODY id when a head is set) |
    /// | 4 | `:1256` | the result cell, raw `.Word` |
    /// | 5-9 | `:1258-1261` | `var_6[0..5]`, the per-slot outcome class |
    /// | 10-12 | `:1263-1266` | the three approach lines (string registers 1..3) |
    /// | 13 | `:1268` | party-flee movement threshold |
    /// | 14 | `:1269` | monster-flee movement threshold |
    ///
    /// **The preamble runs once** (`:1245-1279`): `byte_1EE95 = true` — which
    /// is what suppresses `sub_30580`'s close-up for the whole menu, keeping
    /// the 3D approach sprite on screen — then `calc_group_movement`, the
    /// operand decode, the `sub_304B4` ray clamped into
    /// `area2_ptr.encounter_distance`, and one encounter-visual dispatch.
    ///
    /// **The loop body** (`:1281-1531`) prints one approach line, opens the
    /// menu, and resolves the reply; see [`Self::encounter_menu_reply`] for the
    /// outcome table.
    ///
    /// Presentation the original does here and this engine does not model, each
    /// for a reason already established elsewhere: `bottomTextHasBeenCleared`
    /// and `DelayBetweenCharacters` (`:1246-1247`, `:1535`) are the teletype
    /// pacing pair — [`crate::Effect::Print`]'s `TextJob` always paces;
    /// `useOverlay` (`:1283-1292`) is recomputed per tick by the parked widget
    /// (FD-33's discharge); `textXCol`/`textYCol` (`:1296-1297`) are
    /// `NORMAL_BOTTOM`'s own origin; and `ClearPromptArea` (`:1534`) is the
    /// same call `CMD_HorizontalMenu` ends with and this engine has never
    /// modelled (the widget closes itself).
    fn op_encounter_menu(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        // `:1245` — set BEFORE anything else, so the dispatch at `:1279` and
        // every later one already sees it.
        host.set_encounter_menu_active(true);
        // `:1250` — sampled once, outside the loop.
        let (slowest, fastest) = host.calc_group_movement();

        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 14, host, pc);
        let sprite_id = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let max_distance = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let pic_id = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;
        let dest = self.resolve_target(&args[3], pc, opcode)?;
        let mut outcomes = [0u8; 5];
        for (i, slot) in outcomes.iter_mut().enumerate() {
            *slot = self.resolve_numeric(&args[4 + i], pc, opcode, host)? as u8;
        }
        // `:1263-1266` — the string REGISTERS, not the operands: `strIndex`
        // counts only string-mode operands, so these are registers 1..=3
        // whatever positions the strings occupied.
        let texts = [
            self.strings.get(1).clone(),
            self.strings.get(2).clone(),
            self.strings.get(3).clone(),
        ];
        let party_flee_movement = self.resolve_numeric(&args[12], pc, opcode, host)? as u8;
        let monster_flee_movement = self.resolve_numeric(&args[13], pc, opcode, host)? as u8;

        // `:1253-1255` — the same three stores SETUP MONSTER makes.
        host.setup_monster(sprite_id, max_distance, pic_id);
        // `:1271-1276` — the ray, clamped.
        let distance = host.approach_distance();
        let distance = if max_distance < distance as u16 {
            max_distance as u8
        } else {
            distance
        };
        host.set_encounter_distance(distance);
        host.load_encounter_visual(); // `:1279`

        let state = EncounterMenuState {
            dest,
            outcomes,
            texts,
            party_flee_movement,
            monster_flee_movement,
            slowest,
            fastest,
            next,
        };
        let mut effects = VecDeque::new();
        effects.push_back(Effect::EncounterVisual);
        let request = self.encounter_menu_iteration(host, pc, &state, &mut effects);
        Ok(Self::yield_effects(
            activation,
            pc,
            effects,
            Some(request),
            Completion::EncounterMenu(Box::new(state)),
        ))
    }

    /// One turn of ENCOUNTER MENU's loop body (`ovr003.cs:1283-1361`): the
    /// approach line, then the menu.
    ///
    /// The line is chosen by a **cyclic scan starting at the current distance
    /// band** (`:1298-1339`) — band 0 scans `0,1,2` and stops at the end; bands
    /// 1 and 2 wrap (`1,2,0` and `2,0,1`) and stop when the cursor returns to
    /// where it began. So a script that fills only one of the three strings
    /// still has something to say at every range, and the empty-string case
    /// additionally suppresses the region clear (`:1341-1344`).
    ///
    /// The fourth menu word is PARLAY when the monsters are already adjacent
    /// (or the party is outdoors) and ADVANCE otherwise (`:1348-1355`) — the
    /// words a player sees differ, while the slot they resolve to does not,
    /// which is the presentation/execution split the combat menus already use.
    fn encounter_menu_iteration(
        &mut self,
        host: &mut dyn VmHost,
        pc: u16,
        state: &EncounterMenuState,
        effects: &mut VecDeque<Effect>,
    ) -> Request {
        let distance = host.encounter_distance();
        let in_dungeon = self.in_dungeon(host, pc);

        // `:1294` — `clearTextArea = (area_ptr.inDungeon != 0)`.
        let mut clear_first = in_dungeon;
        let text = Self::encounter_menu_text(state, distance);
        if text.0.is_empty() {
            clear_first = false; // `:1341-1344`
        }
        // `:1346` — `press_any_key(text, clearTextArea, 10, NormalBottom)`.
        effects.push_back(Effect::Print { text, clear_first });

        Request::HorizontalMenu {
            options: Self::encounter_menu_words(distance, in_dungeon),
        }
    }

    /// `:1298-1339`'s cyclic scan, as one expression: try the band's own
    /// string first, then each following band in turn, wrapping, and take the
    /// first non-empty. Falls back to the last one visited when all three are
    /// empty — which is what the original's `do`/`while` leaves in `text`.
    fn encounter_menu_text(state: &EncounterMenuState, distance: u8) -> VmString {
        // Only 0..=2 are reachable (the clamp caps the ray at 2); a wilder
        // value would fall through the original's `switch` and reuse the
        // PREVIOUS iteration's `text`, a stack local it never resets. Band 0's
        // scan is the closest honest stand-in and cannot be reached anyway.
        let start = (distance as usize).min(2);
        let order: [usize; 3] = if start == 0 {
            [0, 1, 2] // `:1300-1307` — no wrap; this arm stops at index 3
        } else {
            [start, (start + 1) % 3, (start + 2) % 3] // `:1309-1338`
        };
        let mut last = VmString::default();
        for i in order {
            last = state.texts[i].clone();
            if !last.0.is_empty() {
                return last;
            }
        }
        last
    }

    /// `:1348-1355`. The menu is built as a literal `"~COMBAT ~WAIT ~FLEE
    /// ~PARLAY"`/`"…~ADVANCE"` string in the original and handed to the same
    /// `sub_317AA` `CMD_HorizontalMenu` uses, so it presents identically —
    /// `buildMenuStrings` lowercases everything but the `~`-marked initials,
    /// leaving "Combat Wait Flee Parlay" with C/W/F/P as the hotkeys.
    fn encounter_menu_words(distance: u8, in_dungeon: bool) -> Vec<VmString> {
        let fourth: &[u8] = if distance == 0 || !in_dungeon {
            b"PARLAY"
        } else {
            b"ADVANCE"
        };
        vec![
            VmString(b"COMBAT".to_vec()),
            VmString(b"WAIT".to_vec()),
            VmString(b"FLEE".to_vec()),
            VmString(fourth.to_vec()),
        ]
    }

    /// ★ ENCOUNTER MENU's outcome table (`ovr003.cs:1363-1531`), resolved.
    ///
    /// First, the **slot remap** (`:1363-1368`): with the monsters adjacent (or
    /// outdoors) the fourth word is PARLAY, and selecting it resolves to slot
    /// **4**, not 3 — so `var_6` really does have five entries for four words,
    /// and slot 3 (ADVANCE) is simply unreachable at distance 0.
    ///
    /// Then `var_43A = var_6[selection]` picks one of five outcome classes, and
    /// the class × slot pair decides:
    ///
    /// | class | COMBAT (0) | WAIT (1) | FLEE (2) | ADVANCE (3) | PARLAY (4) |
    /// |---|---|---|---|---|---|
    /// | 0 | write 1 | write 1 | flee check | write 1 | write 1 |
    /// | 1 | write 1 | "Both sides wait." + loop | write 2 | step in (or wait) + loop | step in + loop, else write 3 |
    /// | 2 | monster-flee check | monsters flee | monsters flee | monsters flee | monsters flee |
    /// | 3 | write 1 | step in (or wait) + loop | write 2 | step in (or wait) + loop | step in + loop, else write 3 |
    /// | 4 | write 1 | step in + loop, else write 3 | write 2 | same as WAIT | same as WAIT |
    ///
    /// The written values are the script's language: **0** = the monsters fled,
    /// **1** = fight, **2** = the party got away, **3** = parlay/advance
    /// exhausted. `ECL6#64 @0x8519` feeds its cell straight to an `ON GOTO`
    /// with four targets; `ECL4#32 @0x98A9` compares it against 3.
    ///
    /// Two checks read `calc_group_movement`'s sampled pair. The party's flee
    /// succeeds when its **slowest** member makes the operand-13 threshold
    /// (`:1384`), and the monsters break off when operand 14 beats the party's
    /// **fastest** (`:1442`) — slowest for running away, fastest for being run
    /// away from.
    ///
    /// Every "step in" arm decrements the distance and re-dispatches the
    /// encounter visual, exactly as APPROACH does; at distance 0 the same arms
    /// print "Both sides wait." instead, except class 1/3/4's PARLAY slot,
    /// which is where the write-3 parlay outcome comes from.
    fn encounter_menu_reply(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        state: EncounterMenuState,
        selection: u8,
    ) -> Result<VmStep, VmError> {
        const WAIT_TEXT: &[u8] = b"Both sides wait.";
        const MONSTERS_FLEE_TEXT: &[u8] = b"The monsters flee.";

        let distance = host.encounter_distance();
        let in_dungeon = self.in_dungeon(host, pc);

        // `:1363-1368` — PARLAY resolves to slot 4, not slot 3.
        let mut slot = selection as usize;
        if (distance == 0 || !in_dungeon) && slot == 3 {
            slot = 4;
        }
        let class = state.outcomes[slot.min(4)];

        let mut effects = VecDeque::new();
        // `Some(value)` = write and end; `None` = loop again.
        let mut result: Option<u16> = None;
        let mut loop_again = false;

        // Every "the monsters walk a band closer" arm is this: decrement and
        // re-dispatch, or say nothing happened.
        let step_in = |host: &mut dyn VmHost, effects: &mut VecDeque<Effect>| {
            if distance != 0 {
                host.set_encounter_distance(distance - 1);
                host.load_encounter_visual();
                effects.push_back(Effect::EncounterVisual);
            } else {
                effects.push_back(Effect::Print {
                    text: VmString(WAIT_TEXT.to_vec()),
                    clear_first: true,
                });
            }
        };

        match (class, slot) {
            // --- class 0 (`:1372-1394`) ---
            (0, 2) => {
                // `:1384` — the party's SLOWEST must make the threshold.
                result = Some(if state.slowest >= state.party_flee_movement {
                    2
                } else {
                    1
                });
            }
            (0, _) => result = Some(1),

            // --- class 1 (`:1396-1437`) ---
            (1, 0) => result = Some(1),
            (1, 1) => {
                effects.push_back(Effect::Print {
                    text: VmString(WAIT_TEXT.to_vec()),
                    clear_first: true,
                });
                loop_again = true;
            }
            (1, 2) => result = Some(2),
            (1, 3) => {
                step_in(host, &mut effects);
                loop_again = true;
            }
            (1, _) => {
                if distance > 0 {
                    step_in(host, &mut effects);
                    loop_again = true;
                } else {
                    result = Some(3);
                }
            }

            // --- class 2 (`:1439-1462`) ---
            (2, 0) => {
                // `:1442` — the monsters outrun the party's FASTEST.
                if state.monster_flee_movement > state.fastest {
                    result = Some(0);
                    effects.push_back(Effect::Print {
                        text: VmString(MONSTERS_FLEE_TEXT.to_vec()),
                        clear_first: true,
                    });
                } else {
                    result = Some(1);
                }
            }
            (2, _) => {
                result = Some(0);
                effects.push_back(Effect::Print {
                    text: VmString(MONSTERS_FLEE_TEXT.to_vec()),
                    clear_first: true,
                });
            }

            // --- class 3 (`:1464-1505`) ---
            (3, 0) => result = Some(1),
            (3, 1) | (3, 3) => {
                step_in(host, &mut effects);
                loop_again = true;
            }
            (3, 2) => result = Some(2),
            (3, _) => {
                if distance == 0 {
                    result = Some(3);
                } else {
                    step_in(host, &mut effects);
                    loop_again = true;
                }
            }

            // --- class 4 (`:1507-1530`) ---
            (4, 0) => result = Some(1),
            (4, 2) => result = Some(2),
            (4, _) => {
                if distance == 0 {
                    result = Some(3);
                } else {
                    step_in(host, &mut effects);
                    loop_again = true;
                }
            }

            // No `default` in the original's `switch` — an outcome class
            // outside 0..=4 falls straight through, writes nothing, and (with
            // `init_max` still 0) ends the instruction. Verbatim.
            _ => {}
        }

        if loop_again {
            let request = self.encounter_menu_iteration(host, pc, &state, &mut effects);
            return Ok(Self::yield_effects(
                activation,
                pc,
                effects,
                Some(request),
                Completion::EncounterMenu(Box::new(state)),
            ));
        }

        // The write happens at execution time, where the original's
        // `vm_SetMemoryValue` calls sit — ahead of the trailing
        // "The monsters flee." (`:1444-1450`), which is why that print is
        // queued behind it rather than folded into a completion.
        if let Some(value) = result {
            self.mem_write(state.dest, value, host, Origin { pc });
        }
        host.set_encounter_menu_active(false); // `:1536`
        if effects.is_empty() {
            // Nothing left to present (the common case: the outcome was just a
            // write) — complete here rather than parking an empty drain.
            activation.pc = state.next;
            return Ok(VmStep::Continue);
        }
        Ok(Self::yield_effects(
            activation,
            pc,
            effects,
            None,
            Completion::Advance(state.next),
        ))
    }

    /// `gbl.area_ptr.inDungeon` — the RAW cell, the same address
    /// [`Self::op_call`]'s `0xAE11` arm reads (`Classes/Area1.cs:495-496`,
    /// DataOffset `0x1CC`). Read rather than serviced because the VM already
    /// knows this address.
    fn in_dungeon(&self, host: &mut dyn VmHost, pc: u16) -> bool {
        const IN_DUNGEON_ADDR: u16 = 0x4BE6;
        self.mem_read(IN_DUNGEON_ADDR, host, Origin { pc }) != 0
    }

    /// PARTYSTRENGTH (0x1D), `CMD_PartyStrength` ovr003.cs:772-810. One
    /// operand, the destination cell — taken as the raw `.Word`
    /// (`gbl.cmd_opps[1].Word`, `:808`) exactly like HORIZONTAL MENU's, not as
    /// a resolved value. The arithmetic itself is engine-side
    /// ([`EngineServices::party_strength`]); it is draw-free.
    fn op_party_strength(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let dest = self.resolve_target(&args[0], pc, opcode)?;
        let power = host.party_strength() as u16;
        self.mem_write(dest, power, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// PICTURE (0x0E), `CMD_Picture` ovr003.cs:312-358. `blockId == 0xFF` is
    /// the "clear picture" sentinel.
    fn op_picture(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let block_id = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let effect = if block_id == 0xFF {
            Effect::ClearPicture
        } else {
            Effect::Picture(block_id)
        };
        Ok(Self::yield_effect(activation, pc, effect, next))
    }

    /// PRINT (0x11) / PRINTCLEAR (0x12), `CMD_Print` ovr003.cs:389-417
    /// (shared handler, `clear` keyed on the opcode). Numeric operands are
    /// stringified and stashed into register slot 1 exactly like the
    /// original (`gbl.unk_1D972[1] = val.ToString()`); string-mode operands
    /// already landed there via this instruction's own `load_cmd_sets` side
    /// effect.
    fn op_print(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
        clear_first: bool,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let text = if Self::is_string_mode(&args[0]) {
            self.strings.get(1).clone()
        } else {
            let val = self.resolve_numeric(&args[0], pc, opcode, host)?;
            let text = VmString::from_bytes(val.to_string().into_bytes());
            self.strings.set(1, text.clone());
            text
        };
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::Print { text, clear_first },
            next,
        ))
    }

    /// RETURN (0x13), `CMD_Return` ovr003.cs:420-435. An empty call stack
    /// silently becomes EXIT, full side effects included.
    fn op_return(&mut self, activation: &mut Activation) -> Result<VmStep, VmError> {
        if let Some(target) = self.call_stack.pop() {
            activation.pc = target;
            Ok(VmStep::Continue)
        } else {
            self.op_exit(activation)
        }
    }

    /// COMPARE AND (0x14), `CMD_CompareAnd` ovr003.cs:438-461. Only ever
    /// sets flags `[0]`/`[1]` (`==`/`!=`) — never the relational four. Every
    /// operand goes through `GetCmdValue` with no `Code<0x80` guard, so a
    /// string-mode operand here is the docket-item-5 hazard, surfaced as
    /// `VmError::StringOperandTypeMismatch`.
    fn op_compare_and(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 4, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let c = self.resolve_numeric(&args[2], pc, opcode, host)?;
        let d = self.resolve_numeric(&args[3], pc, opcode, host)?;
        self.flags = [false; 6];
        if a == b && c == d {
            self.flags[0] = true;
        } else {
            self.flags[1] = true;
        }
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// The six IF opcodes (0x16-0x1B), `CMD_If` ovr003.cs:464-477 +
    /// `SkipNextCommand` ovr003.cs:2130-2144. Skip is table-driven, not
    /// decode: it advances by the *following* opcode's declared `skip_size`
    /// (running the same side-effecting operand loader used everywhere
    /// else), one byte only for size-0 opcodes, and tolerates an unknown
    /// following opcode by advancing one byte with no error (unlike
    /// executing an unknown opcode directly, which is fatal).
    fn op_if(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let index = (opcode - 0x16) as usize;
        let next = pc.wrapping_add(1);
        if self.flags[index] {
            activation.pc = next;
            return Ok(VmStep::Continue);
        }

        let skip_opcode = self.block.get(next);
        let skip_target = match self.dialect.lookup(skip_opcode) {
            None => next.wrapping_add(1),
            Some(info) if info.skip_size == 0 => next.wrapping_add(1),
            Some(info) => {
                let (_args, cursor) =
                    self.load_cmd_sets(next.wrapping_add(1), info.skip_size, host, pc);
                cursor
            }
        };
        activation.pc = skip_target;
        Ok(VmStep::Continue)
    }

    /// CLEARMONSTERS (0x1C), `CMD_ClearMonsters` ovr003.cs:758-769. No
    /// operands.
    fn op_clearmonsters(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
    ) -> Result<VmStep, VmError> {
        host.clear_monsters();
        activation.pc = activation.pc.wrapping_add(1);
        Ok(VmStep::Continue)
    }

    /// NEWECL (0x20), `CMD_NewECL` ovr003.cs:480-498. The interpreter's job
    /// ends at reporting the chain: block-swap + `vm_init_ecl`-equivalent
    /// resets happen via a subsequent `load_block` call (D-VM3 — "no VM
    /// context ever resumes across a chain," and string registers
    /// deliberately survive the switch, so they're not touched here either).
    fn op_newecl(
        &mut self,
        _activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, _next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let block_id = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        Ok(VmStep::Done(Exit::ChainTo(BlockId(block_id))))
    }

    /// LOAD FILES (0x21) / LOAD PIECES (0x37), `CMD_LoadFiles`
    /// ovr003.cs:501-604 — one shared handler, keyed on `gbl.command`
    /// (`load_pieces` here). Operand decode/order is identical for both
    /// (`var_3, var_2, var_1` from operands 1-3, matching the original's own
    /// quirky reversed naming).
    ///
    /// 0x21 (`load_pieces == false`): ★ the `lastDaxBlockId != 0x50` gate is
    /// real as of roll-credits D-S7e — it reads through
    /// [`VmHost::last_dax_block`], the field having no `ScriptMemory` address
    /// of its own. (M2 dropped it as a documented simplification; the door
    /// asked for the cell modeled once and all three of its guards threaded
    /// from it.)
    ///
    /// 0x37 (`load_pieces == true`, added for `restrike run-script`'s M1
    /// task 3 real-block demo — under-traced by the original M1 step-0
    /// classification pass, which stopped at `Load3DMap`/`LoadWalldef`/
    /// `load_bigpic` without reading this branch's body): `var_3 == 0x7F`
    /// loads a fixed walldef; otherwise a gate on `area_ptr.field_1CE`/
    /// `field_1D0` (both engine-internal, no `ScriptMemory` address) picks
    /// between a 2-call and a 3-call `LoadWalldef` sequence — modeled here
    /// as always false (documented simplification, same spirit as 0x21's:
    /// prefer the branch that exercises the full mapped service surface —
    /// the 3-way load-or-`reset_wall_set` sequence — over guessing at
    /// unmodeled state). None of these calls feed a value back into the VM,
    /// so the simplification cannot affect control flow, only which
    /// `EngineServices` calls are observed.
    fn op_load_files(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
        load_pieces: bool,
    ) -> Result<VmStep, VmError> {
        const IN_DUNGEON_ADDR: u16 = 0x4BE6;
        /// `gbl.lastDaxBlockId == 0x50`, the city-scene guard
        /// (`crate::gbx_engine::picture::CITY_SCENE_PIC_BLOCK` in the engine —
        /// this crate has no dependency on it, so the literal is repeated with
        /// its citation).
        const CITY_SCENE_PIC_BLOCK: u8 = 0x50;
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let var_3 = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let var_2 = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let var_1 = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;

        if !load_pieces {
            let in_dungeon = self.mem_read(IN_DUNGEON_ADDR, host, Origin { pc });
            if var_3 != 0xFF && var_3 != 0x7F && in_dungeon != 0 {
                host.load_3d_map(var_3);
            }
            // `ovr003.cs:528-533`. Note the block id is the HARDCODED `0x79`,
            // not `var_1`: the third operand only decides *whether* to reload
            // the overland map, never which one.
            if var_1 != 0xFF && in_dungeon == 0 && host.last_dax_block() != CITY_SCENE_PIC_BLOCK {
                host.load_bigpic(0x79);
            }
        } else if var_3 == 0x7F {
            host.load_walldef(1, 0);
        } else {
            if var_3 != 0xFF {
                host.load_walldef(1, var_3);
            } else {
                host.reset_wall_set(0);
            }
            if var_2 != 0xFF {
                host.load_walldef(2, var_2);
            } else {
                host.reset_wall_set(1);
            }
            if var_1 != 0xFF {
                host.load_walldef(3, var_1);
            } else {
                host.reset_wall_set(2);
            }
        }
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// COMBAT (0x24), `CMD_Combat` ovr003.cs:971-1029. The design doc's
    /// preferred coarse request: the engine owns `MainCombatLoop`/
    /// `CityShop`/`temple_shop` entirely (opcode-classification.md docket
    /// item 10) — out of scope for this interpreter's fidelity beyond
    /// suspending and resuming.
    fn op_combat(&mut self, activation: &mut Activation) -> Result<VmStep, VmError> {
        let pc = activation.pc;
        let next = pc.wrapping_add(1);
        Ok(Self::yield_request(
            activation,
            pc,
            Request::Combat,
            Completion::Advance(next),
        ))
    }

    /// ON GOTO (0x25), `CMD_OnGotoGoSub` ovr003.cs:1032-1064 (`gbl.command
    /// == 0x25` branch). Both the selector and the tail-entry count are
    /// `GetCmdValue`-resolved (can be memory-mode, not just immediate).
    /// Out-of-range selector is a confirmed fall-through to `next` — no
    /// `else`-branch jump in the original (`ovr003.cs:1038-1059`).
    fn op_on_goto(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (mut args, cursor) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let selector = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let count = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let (tail, next) = self.load_cmd_sets(cursor, count, host, pc);
        args.extend(tail);

        if selector < count {
            let target = self.resolve_target(&args[2 + selector as usize], pc, opcode)?;
            activation.pc = target;
        } else {
            activation.pc = next;
        }
        Ok(VmStep::Continue)
    }

    /// ON GOSUB (0x26), `CMD_OnGotoGoSub` ovr003.cs:1032-1064 (`gbl.command
    /// == 0x26` branch). Identical decode/dispatch shape to ON GOTO, plus a
    /// call-stack push — but ONLY on the in-range branch
    /// (opcode-classification.md's 0x26 row): the push at `ovr003.cs:1055`
    /// sits inside the `if (var_1 < var_2)` body, so an out-of-range
    /// selector neither jumps nor pushes, indistinguishable from ON GOTO's
    /// own out-of-range fall-through. The pushed return address is `next`
    /// (the fall-through landing after the full decoded tail), matching
    /// GOSUB's own convention.
    fn op_on_gosub(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (mut args, cursor) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let selector = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let count = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let (tail, next) = self.load_cmd_sets(cursor, count, host, pc);
        args.extend(tail);

        if selector < count {
            let target = self.resolve_target(&args[2 + selector as usize], pc, opcode)?;
            self.call_stack.push(next);
            activation.pc = target;
        } else {
            activation.pc = next;
        }
        Ok(VmStep::Continue)
    }

    /// GETTABLE (0x2A), `CMD_GetTable` ovr003.cs:635-648. Operand 1 is a raw
    /// base address (never `GetCmdValue`'d) added to operand 2's resolved
    /// index — a computed address that can address any window despite the
    /// "table" name (docket item 12).
    fn op_gettable(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let base = self.resolve_target(&args[0], pc, opcode)?;
        let index = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        let addr = base.wrapping_add(index);
        let value = self.mem_read(addr, host, Origin { pc });
        self.mem_write(dest, value, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// HORIZONTAL MENU (0x2B), `CMD_HorizontalMenu` ovr003.cs:698-753.
    /// Variable tail: 2 fixed operands (dest, string count), then that many
    /// more string-mode tail operands via a *second*, independently
    /// `strIndex`-reset `load_cmd_sets` call — exactly like the original's
    /// rewind-and-reload (`ecl_offset--; vm_LoadCmdSets(string_count)`).
    fn op_horizontal_menu(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, cursor) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let dest = self.resolve_target(&args[0], pc, opcode)?;
        let count = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let (_tail, next) = self.load_cmd_sets(cursor, count, host, pc);

        let options = (1..=count).map(|i| self.strings.get(i).clone()).collect();
        Ok(Self::yield_request(
            activation,
            pc,
            Request::HorizontalMenu { options },
            Completion::WriteWordThenAdvance { dest, next },
        ))
    }

    /// VERTICAL MENU (0x15), `CMD_VertMenu` ovr003.cs:663-694. The same
    /// variable-tail shape HORIZONTAL MENU has, one fixed operand wider:
    /// `mem_loc`, an inline PROMPT string, the entry count, then that many
    /// reloaded tail strings via the `ecl_offset--; vm_LoadCmdSets(count)`
    /// rewind (`:672-673`).
    ///
    /// The prompt is `gbl.unk_1D972[1]` (`:668`) — the string register the
    /// fixed batch's one string-mode operand filled, exactly as
    /// `vm_LoadCmdSets`'s own `strIndex` counts. `vm_SetMemoryValue(index,
    /// mem_loc)` (`:691`) is [`Completion::WriteWordThenAdvance`]'s job, so the
    /// reply's 0-based index reaches the destination cell unchanged.
    ///
    /// Presentation — `textXCol`/`textYCol`, `press_any_key`'s wrap into the
    /// bottom region, `VertMenuSelect`'s list box and the closing
    /// `draw8x8_clear_area(NormalBottom)` (`:676-693`) — is the engine's, like
    /// every other `Request`: none of it is `ScriptMemory`-addressable.
    fn op_vertical_menu(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, cursor) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let dest = self.resolve_target(&args[0], pc, opcode)?;
        let prompt = self.strings.get(1).clone();
        let count = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;
        let (_tail, next) = self.load_cmd_sets(cursor, count, host, pc);

        let options = (1..=count).map(|i| self.strings.get(i).clone()).collect();
        Ok(Self::yield_request(
            activation,
            pc,
            Request::VerticalMenu { prompt, options },
            Completion::WriteWordThenAdvance { dest, next },
        ))
    }

    /// CALL (0x2D), `CMD_Call` ovr003.cs:1832-1910. The hidden second
    /// dispatch table, fully enumerated in opcode-classification.md §3 (7
    /// keys, no `default` — an unrecognized key is a silent no-op). Case
    /// `0xAE11`'s "redraw dirty flags" gate is engine-internal presentation
    /// state with no `ScriptMemory` address, so both wall queries always run
    /// here (a documented over-approximation — the queries are pure reads,
    /// so calling them unconditionally can't corrupt state, just makes an
    /// extra idempotent call relative to the original).
    fn op_call(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        const IN_DUNGEON_ADDR: u16 = 0x4BE6;
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let raw = self.resolve_target(&args[0], pc, opcode)?;
        let key = raw.wrapping_sub(0x7FFF);

        match key {
            0xAE11 => {
                host.wall_roof();
                host.wall_type();
                // The consolidated redraw gate (`ovr003.cs:1848-1860`): the
                // flag check-and-clear happens here at execution time, the
                // guarded draw travels the effect queue (D-VM3) so it lands
                // before any text the script prints next.
                if host.redraw_view_gate() {
                    return Ok(Self::yield_effect(activation, pc, Effect::RedrawView, next));
                }
            }
            1 => host.setup_duel(true),
            2 => host.setup_duel(false),
            0x3201 => {
                let variant = host.call_sound_variant();
                return Ok(Self::yield_effect(
                    activation,
                    pc,
                    Effect::Sound(variant),
                    next,
                ));
            }
            0x401F => host.move_position_forward(),
            0x4019 => {
                if self.mem_read(IN_DUNGEON_ADDR, host, Origin { pc }) == 0 {
                    host.wall_type();
                }
            }
            0xE804 => {
                return Ok(Self::yield_effect_then_request(
                    activation,
                    pc,
                    Effect::AnimationFrame,
                    Request::Delay,
                    Completion::Advance(next),
                ));
            }
            _ => {}
        }
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// AND (0x2F), `CMD_AndOr` ovr003.cs:607-632 (`gbl.command == 0x2F`
    /// branch; shared with OR/0x30's `op_or`, `:621-624`). Flags derive
    /// from `compare_variables(resultant, 0)` — unwinding the same
    /// `arg_0`/`arg_2` swap as COMPARE gives `set_compare_flags(0,
    /// resultant)`: the relational flags effectively test the result
    /// against zero.
    fn op_and(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        let resultant = (a as u8) & (b as u8);
        self.set_compare_flags(0, resultant as u16);
        self.mem_write(dest, resultant as u16, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// OR (0x30), `CMD_AndOr` ovr003.cs:607-632 (`gbl.command == 0x30`
    /// branch, `:621-624`) — identical structure to AND (0x2F), bitwise OR
    /// instead of AND.
    fn op_or(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let a = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let b = self.resolve_numeric(&args[1], pc, opcode, host)?;
        let dest = self.resolve_target(&args[2], pc, opcode)?;
        let resultant = (a as u8) | (b as u8);
        self.set_compare_flags(0, resultant as u16);
        self.mem_write(dest, resultant as u16, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// PRINT RETURN (0x33), `CMD_PrintReturn` ovr003.cs:1730-1738. Cursor
    /// bookkeeping only — the VM doesn't own `textXCol`/`textYCol` (no
    /// `ScriptMemory` address), so the effect carries no payload.
    fn op_print_return(&mut self, activation: &mut Activation) -> Result<VmStep, VmError> {
        let pc = activation.pc;
        let next = pc.wrapping_add(1);
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::PrintReturn,
            next,
        ))
    }

    /// ★ ECL CLOCK (0x34), `CMD_EclClock` (`ovr003.cs:1720-1727`).
    ///
    /// `vm_LoadCmdSets(2)`, then `timeStep = GetCmdValue(1) & 0xFF` and
    /// `timeSlot = GetCmdValue(2) & 0xFF` — note the ORDER: the *step* is the
    /// first operand and the *slot* the second, which is the reverse of
    /// `step_game_time`'s own parameter order it then calls with.
    ///
    /// One of the census's two uses is the overland's own: `ECL1#80 @0x8EA7
    /// ECL CLOCK [0x4C06], #0x04` spends a journey's route cost in DAYS
    /// (`crate::movement::GameClock::step` has the slot table). The other is
    /// `ECL2#1 @0x8E56`.
    ///
    /// The dialect already records this opcode's confirmed skip≠run
    /// divergence (`skip_size` 1, two operands decoded), so an `IF`-guarded
    /// ECL CLOCK skips the right distance.
    fn op_ecl_clock(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let time_step = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let time_slot = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        host.step_game_time(time_slot, time_step);
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// DELAY (0x3A), `CMD_Delay` ovr003.cs:1588-1592. `game_speed_var` (the
    /// real tick multiplier) has no `ScriptMemory` address, so the request
    /// carries no tick count — the engine decides the real duration.
    fn op_delay(&mut self, activation: &mut Activation) -> Result<VmStep, VmError> {
        let pc = activation.pc;
        let next = pc.wrapping_add(1);
        Ok(Self::yield_request(
            activation,
            pc,
            Request::Delay,
            Completion::Advance(next),
        ))
    }

    // --- roll-credits slice 3: the items/roster/mechanics tail -------------

    /// LOAD CHARACTER (0x0A), `CMD_LoadCharacter` (`sub_262E9`,
    /// `ovr003:02E9`). One operand, byte-cast, passed through **raw** — the
    /// high bit is part of it. Nothing in this handler draws or writes
    /// `ScriptMemory`; every effect is roster state, so the whole body is one
    /// service call (see [`crate::EngineServices::retarget_selected_player`],
    /// which carries the binary transcription and coab's slot-0 correction).
    ///
    /// The dump arm's `PartySummary` repaint is not emitted here: the arm is
    /// conditional on engine-side flags the VM cannot see
    /// (`redrawPartySummary1/2`), so the service owns both the decision and
    /// its redraw.
    fn op_load_character(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let index = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        host.retarget_selected_player(index);
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// FIND ITEM (0x32), `CMD_FindItem` (`ovr003.cs:1560-1585`). Scans every
    /// roster member's inventory for an item of the operand's type.
    ///
    /// The flag convention is the original's, verbatim: all six cleared, then
    /// `compare_flags[1] = true` **before** the scan, and only a hit flips
    /// `[0]` on and `[1]` off. So "not found" reads as `!=` and "found" as
    /// `=`, and the other four relations are left false — a `IF <` after a
    /// FIND ITEM tests nothing, which is exactly what the original leaves
    /// behind.
    fn op_find_item(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let item_type = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let found = host.party_has_item(item_type);
        self.flags = [found, !found, false, false, false, false];
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// FIND SPECIAL (0x3F), `CMD_FindSpecial` (`ovr003.cs:2021-2039`) — FIND
    /// ITEM's twin over `SelectedPlayer.HasAffect`, with the same two-flag
    /// convention (and the same clear-all-six preamble, which here happens
    /// *before* the operand load, `:2023-2028`).
    fn op_find_special(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let affect_type = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let found = host.find_special(affect_type);
        self.flags = [found, !found, false, false, false, false];
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// DESTROY ITEMS (0x40), `CMD_DestroyItems` (`ovr003.cs:2042-2055`):
    /// remove every item of the operand's type from every roster member, then
    /// recompute each member's derived values (the removal can drop a readied
    /// weapon or armour). Sets no flags.
    fn op_destroy_items(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let item_type = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        host.destroy_items(item_type);
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// SAVE TABLE (0x35), `CMD_SaveTable` (`ovr003.cs:651-660`) — GETTABLE's
    /// mirror, and note the operand roles are **not** mirrored: GETTABLE is
    /// `(base, index, dest)` while SAVE TABLE is `(value, base, index)`. The
    /// base is the raw destination `.Word` (never dereferenced, docket item
    /// 3); the index is a resolved value added to it.
    fn op_save_table(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let value = self.resolve_numeric(&args[0], pc, opcode, host)?;
        let base = self.resolve_target(&args[1], pc, opcode)?;
        let index = self.resolve_numeric(&args[2], pc, opcode, host)?;
        let dest = base.wrapping_add(index);
        self.mem_write(dest, value, host, Origin { pc });
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// ★ ADD NPC (0x36), `CMD_AddNPC` (`ovr003.cs:1769-1782`) — the game's one
    /// join mechanism (roll-credits §7.1).
    ///
    /// **`skip_size` 1, arity 2** — the confirmed divergence the dialect table
    /// already records (`Fixed(2)` with `skip_size: 1`): the handler really
    /// does `vm_LoadCmdSets(2)` while an `IF` skipping over it advances by one
    /// batch. Both shipped pairs (`ADD NPC 0x16,0x64` then `ADD NPC 0x17,0x64`)
    /// sit on a straight path, so the divergence is unreachable in practice;
    /// the arity here follows the handler, as everywhere else.
    ///
    /// A missing `MON{area}CHA` block is `load_mob`'s hard stop, surfaced as
    /// [`VmError::MissingAsset`] exactly like LOAD MONSTER's.
    fn op_add_npc(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 2, host, pc);
        let monster_id = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let morale = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        host.add_npc(monster_id, morale)
            .map_err(|_| VmError::MissingAsset { pc, opcode })?;
        // `:1780-1781` — `reclac_player_values` is inside the service (roster
        // state); the summary repaint is the presented half.
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::PartySummary,
            next,
        ))
    }

    /// DUMP (0x3E), `CMD_Dump` (`ovr003.cs:2007-2018`) — ADD NPC's mirror.
    /// Zero operands.
    fn op_dump(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
    ) -> Result<VmStep, VmError> {
        host.dump_selected_player();
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::PartySummary,
            pc.wrapping_add(1),
        ))
    }

    /// CLEAR BOX (0x3D), `CMD_ClearBox` (`ovr003.cs:1741-1754`). Zero
    /// operands, pure presentation — the whole exploration frame is rebuilt.
    /// **Not demo-only** (roll-credits §7.2 withdraws that claim): 17 shipped
    /// uses, concentrated in the wilderness blocks.
    fn op_clear_box(&mut self, activation: &mut Activation, pc: u16) -> Result<VmStep, VmError> {
        Ok(Self::yield_effect(
            activation,
            pc,
            Effect::ClearBox,
            pc.wrapping_add(1),
        ))
    }

    /// ★ TREASURE (0x27), `CMD_Treasure` (`load_item`, `ovr003:1B9D`;
    /// `ovr003.cs:1068-1199`) — the script's own treasure drop.
    ///
    /// Eight operands: seven coin counts **assigned** into `gbl.pooled_money`
    /// (`SetCoins`, not `AddCoins` — a second TREASURE replaces the pool's
    /// coins rather than topping them up) and one block selector:
    ///
    /// - `< 0x80`: a **fixed** drop — every item record in block `id` of
    ///   `ITEM{game_area}.DAX`. Draw-free.
    /// - `0x80 < id < 0xFF`: `id - 0x80` **random** items, each rolled off the
    ///   ladder below.
    /// - `== 0x80` or `== 0xFF`: coins only. (`0x80` falls through the
    ///   `else if` with a zero count and generates nothing, which is the same
    ///   outcome by a different route.)
    ///
    /// **The draws, and their neutrality.** The random arm is genuinely
    /// reachable in shipped content — `ECL5#49 @0x94B3` is
    /// `TREASURE 0,0,0,0,0,6,3,0x82`, two random items, immediately before a
    /// `COMBAT`. Every capture in the frontier manifest is a *combat* draw
    /// stream that begins at `BattleSetup`, so a TREASURE preceding the fight
    /// draws entirely before the capture's first recorded draw. The guard
    /// confirms this the only way that counts: 16/16 still closed.
    ///
    /// The ladder (`ovr003.cs:1102-1195`, thresholds re-read off
    /// `ovr003:1D53-1DF9`), per item:
    ///
    /// | outer d100 | result |
    /// |---|---|
    /// | 1..=60 | a weapon/armour roll (a second d100, below) |
    /// | 61..=85 | magic-user scroll |
    /// | 86..=92 | clerical scroll |
    /// | 91..=98 | potion/wand — **but 91 and 92 are unreachable**, taken by the arm above |
    /// | 99, 100 | shield |
    ///
    /// and the weapon roll: `1..=47` and `50..=59` are the item type
    /// *directly* (with 45 special-cased to a shield), `60..=90` is a third
    /// roll over the five sword types, `91..=94` arrows, `95..=97` a ring of
    /// protection, `98..=100` bracers, and the 48/49 gap falls to a shield.
    /// The overlapping range and the two dead values are the original's, kept.
    fn op_treasure(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 8, host, pc);
        for coin in 0..7u8 {
            let value = self.resolve_numeric(&args[coin as usize], pc, opcode, host)?;
            host.set_pooled_coin(coin, value); // `:1078`
        }
        let block_id = self.resolve_numeric(&args[7], pc, opcode, host)? as u8;

        if block_id < 0x80 {
            host.load_treasure_items(block_id)
                .map_err(|_| VmError::MissingAsset { pc, opcode })?;
        } else if block_id != 0xFF {
            for _ in 0..(block_id - 0x80) {
                let item_type = Self::roll_random_item_type(host);
                host.create_item(item_type);
            }
        }
        activation.pc = next;
        Ok(VmStep::Continue)
    }

    /// One random treasure item's type (`ovr003.cs:1104-1193`). Kept out of
    /// [`Self::op_treasure`]'s body only for readability — the draws are the
    /// opcode's own, in the opcode's own order.
    fn roll_random_item_type(host: &mut dyn VmHost) -> u8 {
        // `ItemType` (`Classes/ItemData.cs:120`), only the values this ladder
        // can produce.
        const BASTARD_SWORD: u8 = 34;
        const BROAD_SWORD: u8 = 35;
        const LONG_SWORD: u8 = 36;
        const SHORT_SWORD: u8 = 37;
        const TWO_HANDED_SWORD: u8 = 38;
        const SHIELD: u8 = 59;
        const MU_SCROLL: u8 = 61;
        const CLERIC_SCROLL: u8 = 62;
        const POTION: u8 = 71;
        const ARROW: u8 = 73;
        const BRACERS: u8 = 77;
        const WAND_B: u8 = 79;
        const TYPE_84: u8 = 84;
        const RING_OF_PROT: u8 = 93;

        let outer = host.roll_dice(100, 1); // `:1104`
        match outer {
            1..=60 => {
                let inner = host.roll_dice(100, 1); // `:1108`
                match inner {
                    45 => SHIELD,                    // `:1113`
                    1..=47 | 50..=59 => inner as u8, // `:1110-1120`
                    60..=90 => match host.roll_dice(10, 1) {
                        // `:1124`
                        1..=4 => LONG_SWORD,
                        5..=7 => BROAD_SWORD,
                        8 => BASTARD_SWORD,
                        9 => SHORT_SWORD,
                        // `:1142` tests `== 10` and leaves the type at its
                        // previous value otherwise — unreachable for a d10.
                        _ => TWO_HANDED_SWORD,
                    },
                    91..=94 => ARROW,        // `:1147`
                    95..=97 => RING_OF_PROT, // `:1151`
                    98..=100 => BRACERS,     // `:1155`
                    // 48 and 49 — the gap the `else` catches (`:1159`).
                    _ => SHIELD,
                }
            }
            61..=85 => MU_SCROLL,     // `:1164`
            86..=92 => CLERIC_SCROLL, // `:1168`
            // `:1172`'s range is `0x5B..=0x62` (91..=98), but 91 and 92 were
            // already taken by the arm above: only 93..=98 reach here.
            93..=98 => match host.roll_dice(15, 1) {
                1..=9 => POTION,
                10 => TYPE_84,
                _ => WAND_B, // 11..=15
            },
            99 | 100 => SHIELD, // `:1189`
            // `roll_dice(100,1)` cannot leave this range; the original's own
            // fall-through leaves `item_type` at its initializer, 0.
            _ => 0,
        }
    }

    /// ★ PARLAY (0x2C), `CMD_Parlay` (`talk_style`, `ovr003:27B7-2855`).
    ///
    /// Not a dialogue tree (review E6 was right): six operands, of which the
    /// first five are a **five-entry outcome table** and the sixth is a
    /// destination cell. The player picks a tone from a fixed, hard-coded
    /// menu — `"~HAUGHTY ~SLY ~NICE ~MEEK ~ABUSIVE"` lives in the code
    /// segment (`ovr003:2806`), not in the script — and the table entry under
    /// that tone is written to the cell, where a COMPARE picks it up.
    ///
    /// So the script decides what each tone *means* at this encounter, and
    /// the same word can be the right answer in one conversation and the
    /// wrong one in the next. That is the whole mechanism.
    ///
    /// **Draw-free**, start to finish: `sub_317AA` is a menu wait and
    /// `cmd_table01` is `vm_SetMemoryValue`. Nothing in `talk_style` touches
    /// the PRNG, so PARLAY is draw-neutral by construction rather than by
    /// argument.
    ///
    /// The operand loop is `for i in 0..=4 { values[i] = GetCmdValue(i + 1) }`
    /// (`:27D0-27EF`) and the destination is `cmd_opps[6].Word` (`:2827-2834`)
    /// — the raw word, never dereferenced, like every destination operand.
    fn op_parlay(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 6, host, pc);
        let mut values = [0u8; 5];
        for (i, slot) in values.iter_mut().enumerate() {
            *slot = self.resolve_numeric(&args[i], pc, opcode, host)? as u8;
        }
        let dest = self.resolve_target(&args[5], pc, opcode)?;
        Ok(Self::yield_request(
            activation,
            pc,
            Request::HorizontalMenu {
                options: PARLAY_TONES
                    .iter()
                    .map(|w| VmString::from_bytes(&w[..]))
                    .collect(),
            },
            Completion::WriteToneOutcomeThenAdvance { dest, values, next },
        ))
    }

    /// ★ DAMAGE (0x2E), `CMD_Damage` (`sub_28958`, `ovr003:2958-2CB5`) — the
    /// script's own damage primitive, and the one opcode in this slice that
    /// draws. Five operands:
    ///
    /// | # | name | meaning |
    /// |---|---|---|
    /// | 1 | `var_1` | mode bits **and**, in the `& 0x80 == 0` arm, the hit COUNT |
    /// | 2 | `var_2` | dice count |
    /// | 3 | `var_3` | dice size |
    /// | 4 | `var_7` | flat damage bonus |
    /// | 5 | `var_6` | save type / to-hit bonus |
    ///
    /// `var_1`'s bits: `0x80` = "saving throw mode" (else: `var_1` hits, each
    /// gated by `CanHitTarget`), `0x40` = whole party, `0x20` = no save at all,
    /// `0x10` = **damage anyway on a successful save**, `0x1F` = the save
    /// bonus. `var_6`'s: `0x80` = target `SelectedPlayer` (with the save type
    /// taken as `type - 1`), low 3 bits = the save type.
    ///
    /// **Draw order, exactly** (`:29BF`, `:29F1`, then per-arm): the damage
    /// roll first; then, iff `var_1 & 0x40 == 0`, one `roll_dice(party_size, 1)`
    /// victim roll — **even in the `0x80`-clear arm, which immediately
    /// re-rolls and discards it**; then each arm's saving throws / to-hit
    /// checks. In the `0x80`-clear loop the damage for iteration *n* is the
    /// value rolled at the END of iteration *n-1* (`:2BDE`), so the pre-loop
    /// roll is the first hit's damage and the last roll is thrown away.
    ///
    /// Draw-neutral for every capture: `DAMAGE` is an ECL opcode and the
    /// captures are combat streams — no capture's script ever executes one.
    fn op_damage(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        // `:295E` — `SelectedPlayer` is saved and restored around the body.
        let selected_backup = host.selected_player();

        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 5, host, pc);
        let var_1 = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let dice_count = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let dice_size = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;
        let dam_plus = self.resolve_numeric(&args[3], pc, opcode, host)?;
        let var_6 = self.resolve_numeric(&args[4], pc, opcode, host)? as u8;

        // `:29BF-29D7` — the first draw, unconditionally.
        let mut damage = host.roll_dice(dice_size, dice_count).wrapping_add(dam_plus);

        let damage_even_on_save = var_1 & 0x10 != 0; // `var_1B`, `:29DA`
        let whole_party = var_1 & 0x40 != 0; // `var_1A`, `:29E2`

        // `:29F1` — the victim roll, taken here (before the 0x80 test) so its
        // draw lands in the original's position even on the arm that re-rolls.
        let mut victim = 0u16;
        if !whole_party {
            let party_size = host.party_size();
            victim = host.roll_dice(party_size, 1);
        }

        if var_1 & 0x80 != 0 {
            let save_bonus = var_1 & 0x1F; // `:2A12`
            let save_type = var_6 & 7; // `:2A1A`
            if whole_party {
                // `:2A28-2A97` — every roster member, walking to the list end.
                let members = host.team_size();
                for index in 0..members {
                    let target = PlayerId(index);
                    // `:2A30` — no save offered at all; `:2A49-2A5E` — a
                    // failed save; `:2A70` — a successful one, which still
                    // hurts under the 0x10 bit. The first two arms have the
                    // same outcome but NOT the same cost: the 0x20 arm must
                    // not draw the save at all.
                    let hit = if var_1 & 0x20 != 0 {
                        true
                    } else {
                        !host.roll_saving_throw(target, save_bonus, save_type)
                            || damage_even_on_save
                    };
                    if hit {
                        host.apply_damage(target, damage);
                    }
                }
            } else if var_6 & 0x80 != 0 {
                // `:2A9C-2AEF` — the selected member, with the save type
                // taken one lower (and type 0 meaning "no save").
                let target = host.selected_player();
                let hit = if save_type == 0
                    || !host.roll_saving_throw(target, save_bonus, save_type - 1)
                {
                    true
                } else {
                    damage_even_on_save
                };
                if hit {
                    host.apply_damage(target, damage);
                }
            } else {
                // `:2AF1-2B5C` — the rolled victim. The original walks a
                // linked list `victim - 1` times with no bounds check; the
                // roll's own range (`1..=party_size`) is what keeps it in
                // bounds, so the saturation here can never bite.
                let target = PlayerId(victim.saturating_sub(1) as u8);
                let hit = if !host.roll_saving_throw(target, save_bonus, save_type) {
                    true
                } else {
                    damage_even_on_save
                };
                if hit {
                    host.apply_damage(target, damage);
                }
            }
        } else {
            // `:2B5F-2C01` — `var_1` separate hits, each on its own freshly
            // rolled victim, each gated by `CanHitTarget(var_6, target)`.
            for _ in 0..var_1 {
                let party_size = host.party_size();
                let rolled = host.roll_dice(party_size, 1);
                let target = PlayerId(rolled.saturating_sub(1) as u8);
                if host.can_hit_target(target, var_6) {
                    host.apply_damage(target, damage);
                }
                // `:2BDE` — the NEXT hit's damage, rolled at the tail. The
                // final iteration's roll is drawn and discarded.
                damage = host.roll_dice(dice_size, dice_count).wrapping_add(dam_plus);
            }
        }

        // `:2C04-2C43` — `party_killed` iff nobody is left `in_combat`. The
        // death screen the original prints inline is the engine's wipe flow
        // here (it swaps the shell at top-of-tick once `party_killed` is set),
        // which is why this opcode emits only the trailing prompt: printing
        // the message twice is the one thing the two models must not do.
        host.party_wipe_check();
        // `:2C8B` — restore, before the prompt.
        host.set_selected_player(selected_backup);

        Ok(Self::yield_request(
            activation,
            pc,
            Request::PressAnyKey {
                text: VmString::from_bytes(&b"press <enter>/<return> to continue"[..]),
                color: 15,
            },
            Completion::Advance(next),
        ))
    }

    /// PROGRAM (0x38), `CMD_Program` (`ovr003.cs:1929-1987`). One operand
    /// selecting one of four engine-level behaviours; the engine performs it
    /// and reports whether the activation continues (case 3 and case 9 both
    /// end in `CMD_Exit()`).
    ///
    /// `ConservativeFallthrough` in the dialect stays honest here: the
    /// disassembler cannot know the operand's case, so it keeps walking; the
    /// interpreter, which *does* know, ends the activation when the engine
    /// says so.
    fn op_program(
        &mut self,
        activation: &mut Activation,
        host: &mut dyn VmHost,
        pc: u16,
        opcode: u8,
    ) -> Result<VmStep, VmError> {
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 1, host, pc);
        let code = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        match host.program(code) {
            ProgramOutcome::Continue => {
                activation.pc = next;
                Ok(VmStep::Continue)
            }
            ProgramOutcome::Exit => {
                // `CMD_Exit`'s own body (`ovr003.cs:9-42`), reached through
                // `CMD_Program`'s tail call.
                self.call_stack.clear();
                Ok(VmStep::Done(Exit::Ended))
            }
        }
    }
}
