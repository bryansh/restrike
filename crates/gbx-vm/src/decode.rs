//! The bytecode decoder: one instruction's worth of bytes in, an [`Instr`]
//! out. Shared by the interpreter, the disassembler, and the census tool
//! (D-VM1) — none of which exist yet this session. This module is decode
//! only: no interpreter loop, no flow-following traversal, no census.
//!
//! Byte accounting is derived precisely from coab's `vm_LoadCmdSets`
//! (`ovr008.cs:9-80`), traced operand-by-operand against
//! `Classes/Opperation.cs`'s `Code`/`Low`/`High`/`Word` fields. The one
//! genuinely unverified byte count — how many raw bytes an inline compressed
//! string (mode `0x80`) consumes beyond its length byte — is called out
//! explicitly below and in its own test; it's pinned against ECLDump goldens
//! later (`docs/design/vm-scriptmemory.md` §5 docket item 6), not derived
//! from coab prose we've actually read.

use crate::dialect::{Dialect, OperandShape};

/// A script block is a fixed `0x1E00`-byte buffer conceptually mapped at VM
/// address `0x8000` (`docs/design/vm-scriptmemory.md` §1). This is a
/// decode-only view over such a buffer — it has no relationship to
/// `gbx-formats`' eventual on-disk block type, and no self-modification
/// support (nothing here writes).
pub const ECL_BLOCK_SIZE: usize = 0x1E00;
pub const ECL_BLOCK_BASE: u16 = 0x8000;

/// Read-only bytes for one resident ECL block, addressed by `0x8000`-based
/// VM address per [`ECL_BLOCK_BASE`].
#[derive(Debug, Clone)]
pub struct BlockBytes {
    data: [u8; ECL_BLOCK_SIZE],
}

impl BlockBytes {
    /// Builds a block from `data`, zero-padding up to [`ECL_BLOCK_SIZE`].
    /// Panics if `data` is longer than a real block can ever be — that's a
    /// test-setup bug, not a runtime condition to handle gracefully.
    pub fn from_bytes(data: &[u8]) -> Self {
        assert!(
            data.len() <= ECL_BLOCK_SIZE,
            "block data ({} bytes) exceeds the 0x1E00-byte ECL block size",
            data.len()
        );
        let mut buf = [0u8; ECL_BLOCK_SIZE];
        buf[..data.len()].copy_from_slice(data);
        Self { data: buf }
    }

    /// Reads the byte at `addr`. Addresses outside `0x8000..=0x9DFF` wrap
    /// (mod `ECL_BLOCK_SIZE`) rather than panicking — self-modified or wild
    /// addresses must never crash the decoder; the original tolerates
    /// arbitrary byte content at any reachable position.
    pub fn get(&self, addr: u16) -> u8 {
        let local = addr.wrapping_sub(ECL_BLOCK_BASE) as usize % ECL_BLOCK_SIZE;
        self.data[local]
    }
}

/// A decoded opcode's raw byte value, dialect-relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Op(pub u8);

/// One decoded operand. Variant names follow the design doc's operand-mode
/// table (`docs/design/vm-scriptmemory.md` §1):
///
/// | Mode | Variant | Meaning |
/// |------|---------|---------|
/// | `0x00` | [`Arg::ImmByte`] | immediate byte |
/// | `0x01` | [`Arg::Mem`] | address; value read through ScriptMemory |
/// | `0x03` | [`Arg::MemAlt`] | same as `0x01` on both read and write paths in coab; kept distinct only because the encoding is (cosmetically) distinct — docket item 3 |
/// | `0x02` | [`Arg::ImmWord`] | immediate word (code addresses, small counts) |
/// | `0x80` | [`Arg::InlineStr`] | inline compressed string → string register; **raw packed bytes only, not decompressed** (docket item 5, `gbx-formats`) |
/// | `0x81` | [`Arg::MemStr`] | address; string copied from memory → string register |
/// | other | [`Arg::UnknownMode`] | tolerated: consumed like an immediate byte, flagged |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    ImmByte(u8),
    Mem(u16),
    MemAlt(u16),
    ImmWord(u16),
    InlineStr(Vec<u8>),
    MemStr(u16),
    UnknownMode { mode: u8, byte: u8 },
}

impl Arg {
    /// The operand's raw 16-bit word, for the destination/target operands
    /// that coab always reads via `.Word` rather than `.GetCmdValue()`
    /// (`docs/design/vm-scriptmemory.md` §1 docket item 3 — GOTO/GOSUB
    /// targets, ON GOTO/ON GOSUB tail entries, and every write-destination
    /// operand). `None` for operand kinds that never carry a `.Word` in the
    /// original (an immediate-byte-moded operand never sets coab's `high`
    /// field, so `.Word` there would throw) — used by the disassembler to
    /// flag an unresolvable jump target rather than guessing one.
    pub fn raw_word(&self) -> Option<u16> {
        match *self {
            Arg::Mem(w) | Arg::MemAlt(w) | Arg::ImmWord(w) => Some(w),
            Arg::ImmByte(_) | Arg::InlineStr(_) | Arg::MemStr(_) | Arg::UnknownMode { .. } => None,
        }
    }
}

/// One decoded instruction: which opcode, its operands in stream order, and
/// the VM address of the next instruction (for non-branching fallthrough).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instr {
    pub op: Op,
    pub args: Vec<Arg>,
    pub next: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// No dialect entry for this opcode byte. The original wedges here
    /// (D-VM6); decode reports it instead of guessing a length.
    UnknownOpcode { addr: u16, opcode: u8 },
    /// A variable-tail opcode's count operand wasn't an immediate
    /// (`ImmByte`/`ImmWord`), so the tail length can't be determined from
    /// bytes alone — matches the design doc's `VariableTailUnresolved`
    /// census hazard (D-VM8). Shipped scripts are expected to always use an
    /// immediate here; this is the "verify via census" case if they don't.
    UnresolvedVariableTail { addr: u16, opcode: u8 },
}

/// Decodes one instruction starting at `addr`. Shared by the interpreter,
/// disassembler, and census (D-VM1) — this crate ships none of those yet.
pub fn decode(bytes: &BlockBytes, addr: u16, dialect: &Dialect) -> Result<Instr, DecodeError> {
    let opcode = bytes.get(addr);
    let info = dialect
        .lookup(opcode)
        .ok_or(DecodeError::UnknownOpcode { addr, opcode })?;

    let mut cursor = addr.wrapping_add(1);
    let mut args = Vec::new();

    let fixed_prefix = match info.shape {
        OperandShape::Fixed(n) => n,
        OperandShape::VariableTail { fixed_prefix } => fixed_prefix,
    };

    for _ in 0..fixed_prefix {
        let (arg, next) = decode_operand(bytes, cursor);
        cursor = next;
        args.push(arg);
    }

    if let OperandShape::VariableTail { .. } = info.shape {
        let count = match args.last() {
            Some(Arg::ImmByte(b)) => *b as u16,
            Some(Arg::ImmWord(w)) => *w & 0x00FF, // the original's count locals are `byte`-typed
            _ => {
                return Err(DecodeError::UnresolvedVariableTail { addr, opcode });
            }
        };
        for _ in 0..count {
            let (arg, next) = decode_operand(bytes, cursor);
            cursor = next;
            args.push(arg);
        }
    }

    Ok(Instr {
        op: Op(opcode),
        args,
        next: cursor,
    })
}

/// Decodes `count` raw operand batches starting at `start` (the first
/// batch's mode byte), returning the address just past the last one.
///
/// This mirrors the original's `vm_LoadCmdSets(count)` — the *same*
/// batch decoder both normal decode and `CmdItem.Skip` call, with `count` a
/// **batch count, not a byte count** (each batch's byte length depends on
/// its own mode byte, exactly like [`decode`]'s `Fixed(n)` loop). The
/// disassembler uses this to compute an IF's skip successor from a
/// dialect's declared `skip_size` (`docs/design/vm-scriptmemory.md` §1,
/// "Skip is not decode": skip advances by *this*, never by how many bytes
/// the opcode's operands actually occupy).
pub(crate) fn skip_batches(bytes: &BlockBytes, start: u16, count: u8) -> u16 {
    let mut cursor = start;
    for _ in 0..count {
        let (_, next) = decode_operand(bytes, cursor);
        cursor = next;
    }
    cursor
}

/// Decodes one operand batch starting at `addr` (the mode byte's position),
/// returning the operand and the address of the next unread byte.
fn decode_operand(bytes: &BlockBytes, addr: u16) -> (Arg, u16) {
    let mode = bytes.get(addr);
    let payload = addr.wrapping_add(1);

    match mode {
        0x00 => {
            let b = bytes.get(payload);
            (Arg::ImmByte(b), payload.wrapping_add(1))
        }
        0x01..=0x03 => {
            let word = read_le_word(bytes, payload);
            let next = payload.wrapping_add(2);
            let arg = match mode {
                0x01 => Arg::Mem(word),
                0x02 => Arg::ImmWord(word),
                0x03 => Arg::MemAlt(word),
                _ => unreachable!(),
            };
            (arg, next)
        }
        0x80 => {
            // Length byte, then that many raw packed bytes. This byte count
            // is an assumption (docket item 6, not verified against
            // `LoadCompressedEclString`'s body) — captured raw, undecoded
            // (docket item 5).
            let len = bytes.get(payload);
            let str_start = payload.wrapping_add(1);
            let mut raw = Vec::with_capacity(len as usize);
            for i in 0..u16::from(len) {
                raw.push(bytes.get(str_start.wrapping_add(i)));
            }
            let next = str_start.wrapping_add(u16::from(len));
            (Arg::InlineStr(raw), next)
        }
        0x81 => {
            let word = read_le_word(bytes, payload);
            (Arg::MemStr(word), payload.wrapping_add(2))
        }
        other => {
            let b = bytes.get(payload);
            (
                Arg::UnknownMode {
                    mode: other,
                    byte: b,
                },
                payload.wrapping_add(1),
            )
        }
    }
}

/// Reads a little-endian word: low byte at `addr`, high byte at `addr+1`
/// (`Classes/Opperation.cs` `High` setter: `word = low + (high << 8)`).
fn read_le_word(bytes: &BlockBytes, addr: u16) -> u16 {
    let lo = bytes.get(addr);
    let hi = bytes.get(addr.wrapping_add(1));
    u16::from_le_bytes([lo, hi])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::COTAB;

    /// All fixtures here are hand-authored synthetic byte sequences (D10) —
    /// nothing derived from real game data.
    fn block_at(addr: u16, bytes: &[u8]) -> BlockBytes {
        let mut data = vec![0u8; (addr - ECL_BLOCK_BASE) as usize];
        data.extend_from_slice(bytes);
        BlockBytes::from_bytes(&data)
    }

    #[test]
    fn zero_operand_opcode_consumes_only_the_opcode_byte() {
        // EXIT (0x00), skip_size 0, Fixed(0).
        let block = block_at(0x8000, &[0x00]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.op, Op(0x00));
        assert!(instr.args.is_empty());
        assert_eq!(instr.next, 0x8001);
    }

    #[test]
    fn immediate_byte_operand_mode_0x00() {
        // GOTO (0x01) is Fixed(1); feed it a mode-0x00 operand for coverage
        // of every mode byte, not because GOTO realistically uses it.
        let block = block_at(0x8000, &[0x01, 0x00, 0x2A]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::ImmByte(0x2A)]);
        assert_eq!(instr.next, 0x8003);
    }

    #[test]
    fn mem_operand_mode_0x01() {
        let block = block_at(0x8000, &[0x01, 0x01, 0x34, 0x12]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::Mem(0x1234)]);
        assert_eq!(instr.next, 0x8004);
    }

    #[test]
    fn mem_alt_operand_mode_0x03() {
        let block = block_at(0x8000, &[0x01, 0x03, 0x34, 0x12]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::MemAlt(0x1234)]);
        assert_eq!(instr.next, 0x8004);
    }

    #[test]
    fn imm_word_operand_mode_0x02() {
        let block = block_at(0x8000, &[0x01, 0x02, 0x00, 0x90]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::ImmWord(0x9000)]);
        assert_eq!(instr.next, 0x8004);
    }

    #[test]
    fn mem_str_operand_mode_0x81() {
        let block = block_at(0x8000, &[0x01, 0x81, 0x00, 0x7C]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::MemStr(0x7C00)]);
        assert_eq!(instr.next, 0x8004);
    }

    #[test]
    fn inline_str_operand_mode_0x80_captures_raw_bytes_undecoded() {
        // Assumption under test (docket item 6): the length byte is
        // immediately followed by exactly that many raw packed bytes.
        let block = block_at(0x8000, &[0x01, 0x80, 0x03, 0xAA, 0xBB, 0xCC]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::InlineStr(vec![0xAA, 0xBB, 0xCC])]);
        assert_eq!(instr.next, 0x8006);
    }

    #[test]
    fn inline_str_zero_length_consumes_no_packed_bytes() {
        let block = block_at(0x8000, &[0x01, 0x80, 0x00]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args, vec![Arg::InlineStr(vec![])]);
        assert_eq!(instr.next, 0x8003);
    }

    #[test]
    fn unknown_mode_is_tolerated_as_immediate_byte() {
        let block = block_at(0x8000, &[0x01, 0x99, 0x42]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(
            instr.args,
            vec![Arg::UnknownMode {
                mode: 0x99,
                byte: 0x42
            }]
        );
        assert_eq!(instr.next, 0x8003);
    }

    #[test]
    fn multi_operand_fixed_opcode_mixed_modes() {
        // ADD (0x04), Fixed(3): imm byte, mem word, imm word destination.
        let block = block_at(
            0x8000,
            &[
                0x04, // opcode
                0x00, 0x05, // operand 1: imm byte 5
                0x01, 0x00, 0x4B, // operand 2: mem 0x4B00
                0x02, 0x10, 0x7C, // operand 3: imm word 0x7C10 (destination)
            ],
        );
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(
            instr.args,
            vec![Arg::ImmByte(5), Arg::Mem(0x4B00), Arg::ImmWord(0x7C10)]
        );
        assert_eq!(instr.next, 0x8009);
    }

    #[test]
    fn variable_tail_resolved_via_immediate_byte_count() {
        // VERTICAL MENU (0x15): 3 fixed operands, the 3rd (count) an
        // ImmByte, then that many more operand batches.
        let block = block_at(
            0x8000,
            &[
                0x15, // opcode
                0x01, 0x00, 0x4B, // operand 1: mem_loc
                0x80, 0x00, // operand 2: header string, empty
                0x00, 0x02, // operand 3: count = 2 (ImmByte)
                0x80, 0x01, 0xAA, // tail operand 1: 1-byte string
                0x80, 0x02, 0xBB, 0xCC, // tail operand 2: 2-byte string
            ],
        );
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args.len(), 5);
        assert_eq!(instr.args[2], Arg::ImmByte(2));
        assert_eq!(instr.args[3], Arg::InlineStr(vec![0xAA]));
        assert_eq!(instr.args[4], Arg::InlineStr(vec![0xBB, 0xCC]));
        assert_eq!(instr.next, 0x800F);
    }

    #[test]
    fn variable_tail_resolved_via_immediate_word_count() {
        // ON GOTO (0x25): 2 fixed operands, the 2nd (count) as ImmWord,
        // truncated to its low byte per the original's `byte` cast.
        let block = block_at(
            0x8000,
            &[
                0x25, // opcode
                0x00, 0x00, // operand 1: selector = 0 (ImmByte)
                0x02, 0x01, 0x00, // operand 2: count = ImmWord(0x0001) -> 1
                0x02, 0x00, 0x90, // tail operand 1: jump target 0x9000
            ],
        );
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args.len(), 3);
        assert_eq!(instr.args[2], Arg::ImmWord(0x9000));
        assert_eq!(instr.next, 0x8009);
    }

    #[test]
    fn variable_tail_with_zero_count_has_no_tail_operands() {
        let block = block_at(
            0x8000,
            &[
                0x2B, /* HORIZONTAL MENU */
                0x01, 0x00, 0x4B, 0x00, 0x00,
            ],
        );
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args.len(), 2);
        assert_eq!(instr.args[1], Arg::ImmByte(0));
        assert_eq!(instr.next, 0x8006);
    }

    #[test]
    fn variable_tail_with_memory_mode_count_is_unresolved() {
        // HORIZONTAL MENU (0x2B) with its count operand encoded as mode
        // 0x01 (memory address) rather than an immediate — the exact
        // "VariableTailUnresolved" census hazard from D-VM8.
        let block = block_at(0x8000, &[0x2B, 0x01, 0x00, 0x4B, 0x01, 0x00, 0x4C]);
        let err = decode(&block, 0x8000, &COTAB).unwrap_err();
        assert_eq!(
            err,
            DecodeError::UnresolvedVariableTail {
                addr: 0x8000,
                opcode: 0x2B
            }
        );
    }

    #[test]
    fn unknown_opcode_is_reported_not_guessed() {
        let block = block_at(0x8000, &[0x41]);
        let err = decode(&block, 0x8000, &COTAB).unwrap_err();
        assert_eq!(
            err,
            DecodeError::UnknownOpcode {
                addr: 0x8000,
                opcode: 0x41
            }
        );
    }

    #[test]
    fn ecl_clock_decodes_two_operands_despite_skip_size_one() {
        // The confirmed skip≠run divergence (docs/design/opcode-classification.md).
        // decode() follows run-time shape (Fixed(2)), independent of skip_size.
        assert_eq!(COTAB.lookup(0x34).unwrap().skip_size, 1);

        let block = block_at(
            0x8000,
            &[
                0x34, 0x00, 0x01, /* time slot */ 0x00, 0x05, /* amount */
            ],
        );
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args.len(), 2);
    }

    #[test]
    fn add_npc_decodes_two_operands_despite_skip_size_one() {
        assert_eq!(COTAB.lookup(0x36).unwrap().skip_size, 1);

        let block = block_at(0x8000, &[0x36, 0x00, 0x03, 0x00, 0x07]);
        let instr = decode(&block, 0x8000, &COTAB).unwrap();
        assert_eq!(instr.args.len(), 2);
    }

    #[test]
    fn block_bytes_wraps_rather_than_panics_outside_the_ecl_window() {
        let block = BlockBytes::from_bytes(&[0xAB]);
        // Address far outside 0x8000..=0x9DFF must not panic.
        let _ = block.get(0x0000);
        let _ = block.get(0xFFFF);
    }
}
