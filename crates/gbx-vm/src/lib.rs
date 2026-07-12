//! ECL bytecode decoder, disassembler, and interpreter, plus the ScriptMemory
//! facade that maps VM operand addresses to named engine state.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.
//!
//! `docs/design/vm-scriptmemory.md` §6 build-order items 0-3 (channel
//! classification, decoder + disassembler, census, and now the
//! `EclMachine` interpreter over the census's top-25 opcodes) are shipped.

pub mod decode;
pub mod dialect;
pub mod disasm;
pub mod host;
pub mod machine;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
mod conformance;

pub use decode::{
    decode, read_header_vectors, Arg, BlockBytes, DecodeError, Instr, Op, ECL_BLOCK_BASE,
    ECL_BLOCK_SIZE,
};
pub use dialect::{
    Channel, Dialect, OpcodeInfo, OperandShape, SuccessorKind, COTAB, COTAB_VECTOR_COUNT,
};
pub use disasm::{disassemble, DataRegion, Hazard, Listing, Summary};
pub use host::{
    Effect, EngineServices, ItemHandle, MissingData, MonsterHandle, NotFound, Origin, PlayerId,
    RecordedCall, Reply, Request, ScriptMemory, VmHost, VmRng, VmString,
};
pub use machine::{
    BlockId, EclMachine, Exit, HeaderError, RestoreError, Snapshot, VmError, VmStep,
};
