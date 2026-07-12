//! Test-only synthetic ECL block assembler (`docs/design/vm-scriptmemory.md`
//! §4: "hand-construct synthetic blocks opcode-by-opcode"). Gated behind the
//! `test-support` Cargo feature rather than `#[cfg(test)]` so it can be
//! reused as a dev-dependency by other crates' conformance tests (the
//! interpreter's, later) without duplicating it per-crate.
//!
//! Every fixture built with [`EclBuilder`] is hand-authored (D10) — nothing
//! here is derived from real game data, and nothing here ships in a release
//! binary (the feature is off by default).

use crate::decode::{BlockBytes, ECL_BLOCK_BASE, ECL_BLOCK_SIZE};
use std::collections::HashMap;

/// One pending word-sized fixup: the two bytes at `bytes[offset..offset+2]`
/// (little-endian) get patched with the resolved address of `label` once
/// [`EclBuilder::build`] runs — this is what lets labels be referenced
/// before they're defined (forward jumps).
struct Fixup {
    offset: usize,
    label: String,
}

/// Hand-assembles a synthetic ECL block byte-by-byte: opcode, operand mode +
/// payload, inline/data bytes, and labels with fixups for jump/call targets.
///
/// Method names mirror the operand-mode table in
/// `docs/design/vm-scriptmemory.md` §1 (`mem` = mode `0x01`, `imm_word` =
/// mode `0x02`, etc.) so a test reads like the instruction it's building.
#[derive(Default)]
pub struct EclBuilder {
    bytes: Vec<u8>,
    labels: HashMap<String, u16>,
    fixups: Vec<Fixup>,
}

impl EclBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The VM address the next byte pushed will land at.
    pub fn here(&self) -> u16 {
        ECL_BLOCK_BASE.wrapping_add(self.bytes.len() as u16)
    }

    /// Records `name` as a label bound to the current (not-yet-written)
    /// address. Panics on a duplicate label — a test-authoring bug, not a
    /// runtime condition to handle gracefully.
    pub fn label(&mut self, name: &str) -> &mut Self {
        let addr = self.here();
        assert!(
            self.labels.insert(name.to_string(), addr).is_none(),
            "EclBuilder: duplicate label {name:?}"
        );
        self
    }

    /// The resolved VM address of `name`. Panics if undefined.
    pub fn addr_of(&self, name: &str) -> u16 {
        *self
            .labels
            .get(name)
            .unwrap_or_else(|| panic!("EclBuilder: undefined label {name:?}"))
    }

    fn push_byte(&mut self, b: u8) -> &mut Self {
        self.bytes.push(b);
        self
    }

    fn push_word(&mut self, value: u16) -> &mut Self {
        let [lo, hi] = value.to_le_bytes();
        self.push_byte(lo);
        self.push_byte(hi)
    }

    fn push_word_fixup(&mut self, label: &str) -> &mut Self {
        let offset = self.bytes.len();
        self.fixups.push(Fixup {
            offset,
            label: label.to_string(),
        });
        self.push_byte(0);
        self.push_byte(0)
    }

    /// Pushes a raw opcode byte.
    pub fn op(&mut self, opcode: u8) -> &mut Self {
        self.push_byte(opcode)
    }

    /// mode `0x00`: immediate byte operand.
    pub fn imm_byte(&mut self, value: u8) -> &mut Self {
        self.push_byte(0x00);
        self.push_byte(value)
    }

    /// mode `0x01`: memory-address operand (ScriptMemory-resolved read).
    pub fn mem(&mut self, addr: u16) -> &mut Self {
        self.push_byte(0x01);
        self.push_word(addr)
    }

    /// mode `0x03`: the read/write-identical alt memory-address operand.
    pub fn mem_alt(&mut self, addr: u16) -> &mut Self {
        self.push_byte(0x03);
        self.push_word(addr)
    }

    /// mode `0x02`: immediate word operand (jump/call targets, small counts).
    pub fn imm_word(&mut self, value: u16) -> &mut Self {
        self.push_byte(0x02);
        self.push_word(value)
    }

    /// mode `0x02` whose word is a forward/backward reference to `label`,
    /// resolved at [`build`](Self::build). The usual way to encode a
    /// GOTO/GOSUB/ON-GOTO-tail target in a test fixture.
    pub fn imm_word_label(&mut self, label: &str) -> &mut Self {
        self.push_byte(0x02);
        self.push_word_fixup(label)
    }

    /// mode `0x01` whose word is a label reference — for exercising a
    /// destination operand encoded in the "address" mode rather than
    /// `0x02` (docket item 3: both behave identically as raw-word targets).
    pub fn mem_label(&mut self, label: &str) -> &mut Self {
        self.push_byte(0x01);
        self.push_word_fixup(label)
    }

    /// mode `0x81`: string-from-memory operand.
    pub fn mem_str(&mut self, addr: u16) -> &mut Self {
        self.push_byte(0x81);
        self.push_word(addr)
    }

    /// mode `0x81` whose address is a label reference — for building an
    /// in-block string operand that points at a data region built with
    /// [`raw`](Self::raw)/[`label`](Self::label) elsewhere in the same block.
    pub fn mem_str_label(&mut self, label: &str) -> &mut Self {
        self.push_byte(0x81);
        self.push_word_fixup(label)
    }

    /// mode `0x80`: inline packed-string operand (raw bytes, undecoded —
    /// `decode.rs` docket item 5 — the length byte plus exactly that many
    /// raw bytes).
    pub fn inline_str(&mut self, raw: &[u8]) -> &mut Self {
        self.push_byte(0x80);
        self.push_byte(raw.len() as u8);
        self.bytes.extend_from_slice(raw);
        self
    }

    /// An operand whose mode byte is outside the known set — exercises
    /// `decode()`'s tolerated-as-immediate-byte fallback.
    pub fn unknown_mode(&mut self, mode: u8, byte: u8) -> &mut Self {
        self.push_byte(mode);
        self.push_byte(byte)
    }

    /// Raw bytes with no operand-mode structure — for data regions (e.g.
    /// bytes an in-block `0x81`/`0x80` string operand targets, or
    /// filler after an unconditional GOTO).
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Resolves all label fixups and returns the finished block. Panics if a
    /// referenced label was never defined, or the assembled block would
    /// exceed the `0x1E00`-byte ECL block size — both test-authoring bugs.
    pub fn build(&self) -> BlockBytes {
        let mut bytes = self.bytes.clone();
        assert!(
            bytes.len() <= ECL_BLOCK_SIZE,
            "EclBuilder: synthetic block ({} bytes) exceeds the 0x1E00-byte ECL block size",
            bytes.len()
        );
        for fixup in &self.fixups {
            let addr = *self
                .labels
                .get(&fixup.label)
                .unwrap_or_else(|| panic!("EclBuilder: undefined label {:?}", fixup.label));
            let [lo, hi] = addr.to_le_bytes();
            bytes[fixup.offset] = lo;
            bytes[fixup.offset + 1] = hi;
        }
        BlockBytes::from_bytes(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Arg, Op};
    use crate::dialect::COTAB;

    #[test]
    fn builds_a_linear_goto_with_a_forward_label() {
        let mut b = EclBuilder::new();
        b.op(0x01).imm_word_label("target"); // GOTO target
        b.label("target");
        b.op(0x00); // EXIT

        let block = b.build();
        let instr = decode(&block, ECL_BLOCK_BASE, &COTAB).unwrap();
        assert_eq!(instr.op, Op(0x01));
        assert_eq!(instr.args, vec![Arg::ImmWord(b.addr_of("target"))]);
    }

    #[test]
    fn here_tracks_the_next_write_address() {
        let mut b = EclBuilder::new();
        assert_eq!(b.here(), ECL_BLOCK_BASE);
        b.op(0x00);
        assert_eq!(b.here(), ECL_BLOCK_BASE + 1);
    }

    #[test]
    #[should_panic(expected = "duplicate label")]
    fn duplicate_labels_panic() {
        let mut b = EclBuilder::new();
        b.label("x");
        b.op(0x00);
        b.label("x");
    }

    #[test]
    #[should_panic(expected = "undefined label")]
    fn unresolved_label_panics_on_build() {
        let mut b = EclBuilder::new();
        b.op(0x01).imm_word_label("nowhere");
        let _ = b.build();
    }
}
