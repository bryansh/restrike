//! ECL bytecode decoder, disassembler, and interpreter, plus the ScriptMemory
//! facade that maps VM operand addresses to named engine state.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//!
//! `docs/design/vm-scriptmemory.md` §6 build-order items 0-2 (channel
//! classification, decoder + disassembler, census) are shipped. No
//! interpreter yet.

pub mod decode;
pub mod dialect;
pub mod disasm;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use decode::{
    decode, read_header_vectors, Arg, BlockBytes, DecodeError, Instr, Op, ECL_BLOCK_BASE,
    ECL_BLOCK_SIZE,
};
pub use dialect::{
    Channel, Dialect, OpcodeInfo, OperandShape, SuccessorKind, COTAB, COTAB_VECTOR_COUNT,
};
pub use disasm::{disassemble, DataRegion, Hazard, Listing, Summary};
