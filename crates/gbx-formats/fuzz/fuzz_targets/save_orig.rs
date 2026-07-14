//! `cargo fuzz run save_orig` — see `crates/gbx-formats/fuzz/README.md`.
//!
//! Original-save parsing (`docs/design/save-formats.md` D-SAVE10/§4) reads
//! untrusted user files (a hand-edited or corrupt `savgam?.dat`/`CHRDAT`
//! record); the contract is "typed error or success, never a panic". Splits
//! the input in two so both `parse_master`-sized and record-sized inputs get
//! exercised without needing two separate corpora.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = gbx_formats::save_orig::parse_master(data);
    let _ = gbx_formats::save_orig::decode_char_record(data);
    let _ = gbx_formats::save_orig::read_items(data);
    let _ = gbx_formats::save_orig::read_affects(data);
});
