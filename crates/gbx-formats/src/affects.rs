//! The typed affect-record decode (doc §39.1). One affect is 9 on-disk bytes
//! (`Affect.StructSize == 9`, `Classes/Affect.cs:164`; `affect_struct_size = 9`
//! in the listing). This layers a typed reader over the opaque 9-byte splitter
//! in [`crate::save_orig::read_affects`] — the same bytes, now with the four
//! meaningful fields pulled out and the trailing heap linkage discarded.
//!
//! Layout, confirmed three ways (coab's `DataOffset` attributes,
//! `Affect.cs:188-195`; `add_affect`'s field stores, `ovr024:13F0-14A4`; and
//! real `.FX` dumps):
//!
//! | off  | size | field               | notes                                    |
//! |------|------|---------------------|------------------------------------------|
//! | 0x00 | 1    | `kind`              | the `Affects` enum id (0x00-0x93)        |
//! | 0x01 | 2    | `minutes`           | game-time minutes; 0 = permanent         |
//! | 0x03 | 1    | `data`              | per-kind payload (e.g. bless amount)     |
//! | 0x04 | 1    | `call_affect_table` | bool: fire the effect handler on add/rm  |
//! | 0x05 | 4    | *(next far ptr)*    | heap linkage, NOT state — **ignored**    |
//!
//! Bytes 0x05-0x08 are a live `seg:off` far pointer that chained the affect on
//! the runtime heap (`charStruct.affect_ptr` walks it). In a real `.FX` dump
//! they are stale live-memory values with a NULL tail; coab zero-fills them on
//! write. They carry no state — [`AffectRecord::decode`] MUST ignore them, and
//! the tests below prove junk there does not change the decode.

/// One decoded affect — the four state fields of a 9-byte on-disk record
/// (`Classes/Affect.cs`), with the trailing heap `next` pointer dropped (doc
/// §39.1). List order is load-bearing at the combatant level, but a single
/// record is order-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AffectRecord {
    /// The `Affects` enum id (`type`@0x00, coab `Affect.type`). 0x00-0x93.
    pub kind: u8,
    /// `minutes`@0x01 (little-endian `ushort`). `0` = permanent / until-removed
    /// (combat never expires an affect by time — doc §39.3).
    pub minutes: u16,
    /// `affect_data`@0x03 — the per-kind payload byte (e.g. a bless amount).
    pub data: u8,
    /// `callAffectTable`@0x04 — whether adding/removing this affect fires the
    /// effect-handler jump table (`CallAffectTable`). Stored as a bool
    /// (byte != 0), as coab's `Affect(byte[], int)` ctor does.
    pub call_affect_table: bool,
}

impl AffectRecord {
    /// The smallest slice [`decode`](Self::decode) can read: the four fields end
    /// at offset 0x04, so five bytes suffice (bytes 0x05-0x08 are ignored heap
    /// linkage). A full on-disk record is
    /// [`AFFECT_RECORD_SIZE`](crate::save_orig::AFFECT_RECORD_SIZE) (9) bytes.
    pub const MIN_LEN: usize = 5;

    /// Decode one affect record from its on-disk bytes (doc §39.1). Reads bytes
    /// 0x00-0x04 and **ignores 0x05-0x08** (the stale heap `next` pointer).
    /// Returns `None` on a short slice (fewer than [`MIN_LEN`](Self::MIN_LEN)
    /// bytes — the fields cannot be read); a full 9-byte record always decodes,
    /// regardless of what its trailing four bytes hold.
    pub fn decode(bytes: &[u8]) -> Option<AffectRecord> {
        if bytes.len() < Self::MIN_LEN {
            return None;
        }
        Some(AffectRecord {
            kind: bytes[0x00],
            minutes: u16::from_le_bytes([bytes[0x01], bytes[0x02]]),
            data: bytes[0x03],
            call_affect_table: bytes[0x04] != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save_orig::AFFECT_RECORD_SIZE;

    /// A synthetic full 9-byte record with a distinct value in every field and
    /// a **junk** `next` pointer (bytes 0x05-0x08). Synthetic bytes only (D10 —
    /// never a real save's bytes).
    fn synthetic_record(next_junk: [u8; 4]) -> [u8; AFFECT_RECORD_SIZE] {
        [
            0x26, // kind = strength (0x26)
            0x2C,
            0x01, // minutes = 0x012C = 300 (little-endian)
            0x03, // data = 3
            0x01, // call_affect_table = true
            next_junk[0],
            next_junk[1],
            next_junk[2],
            next_junk[3],
        ]
    }

    #[test]
    fn decode_reads_the_four_state_fields() {
        let rec = AffectRecord::decode(&synthetic_record([0, 0, 0, 0])).unwrap();
        assert_eq!(rec.kind, 0x26);
        assert_eq!(rec.minutes, 300);
        assert_eq!(rec.data, 3);
        assert!(rec.call_affect_table);
    }

    #[test]
    fn decode_ignores_the_stale_next_pointer() {
        // The heap `next` far pointer (bytes 0x05-0x08) is live-memory garbage in
        // a real dump; a zero tail and a junk tail must decode identically.
        let zero_tail = AffectRecord::decode(&synthetic_record([0x00, 0x00, 0x00, 0x00])).unwrap();
        let junk_tail = AffectRecord::decode(&synthetic_record([0xDE, 0xAD, 0xBE, 0xEF])).unwrap();
        assert_eq!(zero_tail, junk_tail);
    }

    #[test]
    fn call_affect_table_is_byte_nonzero() {
        let mut bytes = synthetic_record([0, 0, 0, 0]);
        bytes[0x04] = 0x00;
        assert!(!AffectRecord::decode(&bytes).unwrap().call_affect_table);
        bytes[0x04] = 0xFF; // any non-zero byte → true (coab's `!= 0`)
        assert!(AffectRecord::decode(&bytes).unwrap().call_affect_table);
    }

    #[test]
    fn minutes_is_little_endian_and_zero_means_permanent() {
        let mut bytes = synthetic_record([0, 0, 0, 0]);
        bytes[0x01] = 0x00;
        bytes[0x02] = 0x00;
        assert_eq!(AffectRecord::decode(&bytes).unwrap().minutes, 0);
        bytes[0x01] = 0xFF;
        bytes[0x02] = 0xFF;
        assert_eq!(AffectRecord::decode(&bytes).unwrap().minutes, 0xFFFF);
    }

    #[test]
    fn short_input_is_none() {
        // Fewer than MIN_LEN bytes cannot carry the four fields.
        assert!(AffectRecord::decode(&[]).is_none());
        assert!(AffectRecord::decode(&[0x26, 0x2C, 0x01, 0x03]).is_none()); // 4 bytes
                                                                            // Exactly MIN_LEN decodes (the tail is optional).
        assert!(AffectRecord::decode(&[0x26, 0x2C, 0x01, 0x03, 0x01]).is_some());
    }

    #[test]
    fn a_full_record_from_read_affects_round_trips() {
        // The opaque splitter yields 9-byte chunks; the typed decode layers on
        // top. Two records back-to-back, second carrying junk linkage.
        let mut blob = Vec::new();
        blob.extend_from_slice(&synthetic_record([0x11, 0x22, 0x33, 0x44]));
        blob.extend_from_slice(&[0x07, 0x00, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD]);
        let chunks = crate::save_orig::read_affects(&blob).unwrap();
        assert_eq!(chunks.len(), 2);
        let a = AffectRecord::decode(&chunks[0]).unwrap();
        let b = AffectRecord::decode(&chunks[1]).unwrap();
        assert_eq!(a.kind, 0x26);
        assert_eq!(b.kind, 0x07); // faerie_fire
        assert_eq!(b.minutes, 0);
        assert!(!b.call_affect_table);
    }
}
