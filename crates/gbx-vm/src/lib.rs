//! ECL bytecode decoder, disassembler, and interpreter, plus the ScriptMemory
//! facade that maps VM operand addresses to named engine state.
//!
//! This crate is platform-pure: no windowing, audio, or async runtime dependencies.

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert_eq!(2 + 2, 4);
    }
}
