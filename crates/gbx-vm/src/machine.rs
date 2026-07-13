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
use crate::host::{Effect, Origin, Reply, Request, VmHost, VmString};

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
    WriteWordThenAdvance { dest: u16, next: u16 },
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
        }
    }

    // --- ScriptMemory routing (D-VM5): the VM intercepts its own Ecl
    // window (read-only — self-modifying writes are out of scope this
    // session, see `mem_write`'s doc comment) before delegating to the host.

    fn mem_read(&self, addr: u16, host: &mut dyn VmHost, origin: Origin) -> u16 {
        if in_block(addr) {
            let lo = self.block.get(addr);
            let hi = self.block.get(addr.wrapping_add(1));
            u16::from_le_bytes([lo, hi])
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
            0x0B => self.op_load_monster(activation, host, pc, opcode),
            0x0C => self.op_setup_monster(activation, host, pc, opcode),
            0x0E => self.op_picture(activation, host, pc, opcode),
            0x11 => self.op_print(activation, host, pc, opcode, false),
            0x12 => self.op_print(activation, host, pc, opcode, true),
            0x13 => self.op_return(activation),
            0x14 => self.op_compare_and(activation, host, pc, opcode),
            0x16..=0x1B => self.op_if(activation, host, pc, opcode),
            0x1C => self.op_clearmonsters(activation, host),
            0x20 => self.op_newecl(activation, host, pc, opcode),
            0x21 => self.op_load_files(activation, host, pc, opcode, false),
            0x24 => self.op_combat(activation),
            0x25 => self.op_on_goto(activation, host, pc, opcode),
            0x26 => self.op_on_gosub(activation, host, pc, opcode),
            0x2A => self.op_gettable(activation, host, pc, opcode),
            0x2B => self.op_horizontal_menu(activation, host, pc, opcode),
            0x2D => self.op_call(activation, host, pc, opcode),
            0x2F => self.op_and(activation, host, pc, opcode),
            0x30 => self.op_or(activation, host, pc, opcode),
            0x33 => self.op_print_return(activation),
            0x37 => self.op_load_files(activation, host, pc, opcode, true),
            0x3A => self.op_delay(activation),
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

    /// SETUP MONSTER (0x0C), `CMD_SetupMonster` ovr003.cs:215-236.
    /// `approach_distance`'s result is only used engine-side in the original
    /// (clamped into `area2_ptr.encounter_distance`, which has no
    /// `ScriptMemory` address) — called here for fidelity of the service
    /// call itself, result otherwise unused.
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
        host.setup_monster(sprite_id, max_distance, pic_id);
        let distance = host.approach_distance().min(max_distance);
        host.load_encounter_visual(0, distance, pic_id, sprite_id);
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
    /// 0x21 (`load_pieces == false`): drops the `lastDaxBlockId != 0x50`
    /// gate on the big-picture load — an engine-internal field with no
    /// documented `ScriptMemory` address — as a documented simplification
    /// (unconditionally allowed rather than silently suppressed).
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
        let (args, next) = self.load_cmd_sets(pc.wrapping_add(1), 3, host, pc);
        let var_3 = self.resolve_numeric(&args[0], pc, opcode, host)? as u8;
        let var_2 = self.resolve_numeric(&args[1], pc, opcode, host)? as u8;
        let var_1 = self.resolve_numeric(&args[2], pc, opcode, host)? as u8;

        if !load_pieces {
            let in_dungeon = self.mem_read(IN_DUNGEON_ADDR, host, Origin { pc });
            if var_3 != 0xFF && var_3 != 0x7F && in_dungeon != 0 {
                host.load_3d_map(var_3);
            }
            if var_1 != 0xFF && in_dungeon == 0 {
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
}
