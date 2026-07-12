//! ECL bytecode decoder, disassembler, and interpreter, plus the ScriptMemory
//! facade that maps VM operand addresses to named engine state.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//!
//! This session ships the dialect table and the bytecode decoder only
//! (`docs/design/vm-scriptmemory.md` §6 build-order items 0-1, decoder half).
//! No interpreter, no flow-following disassembler, no census tool yet.

pub mod decode;
pub mod dialect;
pub mod disasm;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use decode::{decode, Arg, BlockBytes, DecodeError, Instr, Op};
pub use dialect::{Channel, Dialect, OpcodeInfo, OperandShape, SuccessorKind, COTAB};
pub use disasm::disassemble;
