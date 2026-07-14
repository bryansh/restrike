//! `cargo fuzz run exepack` — see `crates/gbx-formats/fuzz/README.md`.
//!
//! `exepack::decode` is exercised on every boot against user-supplied
//! `.EXE` bytes (PLAN.md M1's fuzz-roster convention), so this target just
//! throws arbitrary bytes at it: the module's contract is "typed error or
//! success, never a panic" (bounds-checked arithmetic throughout), and this
//! is what checks that contract holds on inputs no unit test thought of.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = gbx_formats::exepack::decode(data);
});
