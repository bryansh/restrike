//! The `ITEMS` game-file parser — the resident weapon/armor data table
//! (`ItemDataTable`, coab `Classes/ItemData.cs`; doc §34.1, M5 armed slice).
//!
//! The game ships a flat `ITEMS` file next to the executable: a 2-byte header
//! followed by fixed-width 16-byte entries, one per item type. At boot the
//! original loads it into the resident table `seg600:5D10` (`unk_1C020`) and
//! indexes it by a readied item's `type` byte to answer every combat question
//! about that weapon — its dice, attack count, range, and behaviour flags.
//!
//! **Faithful load (coab `ItemData.cs:42-59`).** coab seeks past the 2-byte
//! header, reads `0x810` bytes into a **zeroed** `0x810`-byte buffer, and
//! builds `0x81` ([`ITEM_TYPE_COUNT`]) entries. A short file therefore
//! zero-fills the tail entries rather than failing — CotAB's shipped `ITEMS`
//! is `0x802` bytes (2-byte header + `0x80` entries), so the last entry
//! (`Type_128` @ index `0x80`) reads all-zero. [`ItemDataTable::parse`]
//! reproduces that exactly: it always yields `0x81` entries, taking each from
//! the file where present and zero-filling beyond the file's end.
//!
//! Pure over bytes — no filesystem access (`gbx-formats`' convention); the
//! caller supplies the file contents.

use std::fmt;

/// Entries in the resident item table (`table = new ItemData[0x81]`,
/// coab `ItemData.cs:52`).
pub const ITEM_TYPE_COUNT: usize = 0x81;

/// Each `ITEMS` entry is 16 bytes (`new ItemData(data, i * 0x10)`,
/// coab `ItemData.cs:55`).
pub const ITEM_ENTRY_SIZE: usize = 0x10;

/// The `ITEMS` file's leading header, skipped before the entries
/// (`stream.Seek(2, …)`, coab `ItemData.cs:48`).
pub const ITEMS_HEADER_SIZE: usize = 2;

/// The `ItemData.field_E` behaviour flags (coab `enum ItemDataFlags`,
/// `Classes/ItemData.cs:7`). The names mirror coab's; the two suggestive ones
/// are documented from their combat use (doc §34.2/§34.6).
pub mod flags {
    /// Consumes arrows from the arrows ammo slot (`GetCurrentAttackItem`).
    pub const ARROWS: u8 = 0x01;
    pub const FLAG_02: u8 = 0x02;
    /// A hand-to-hand weapon.
    pub const MELEE: u8 = 0x04;
    /// A launcher (bow/crossbow) — draws ammo from an ammo slot.
    pub const FLAG_08: u8 = 0x08;
    /// Self-launching (thrown / sling) — the item itself is the missile.
    pub const FLAG_10: u8 = 0x10;
    pub const FLAG_20: u8 = 0x20;
    pub const FLAG_40: u8 = 0x40;
    /// Consumes quarrels from the quarrels ammo slot (`GetCurrentAttackItem`).
    pub const QUARRELS: u8 = 0x80;
}

/// One 16-byte `ITEMS` entry (coab `class ItemData`, `Classes/ItemData.cs:71`).
/// Every field is a direct byte read at the offset coab documents; the two
/// damage bonuses are signed (`sbyte`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemData {
    /// `[0]` `item_slot` — inventory slot class (`ItemSlot`).
    pub item_slot: u8,
    /// `[1]` `handsCount` — hands required to wield.
    pub hands_count: u8,
    /// `[2]` `diceCountLarge` — damage dice count vs large targets.
    pub dice_count_large: u8,
    /// `[3]` `diceSizeLarge` — damage die size vs large targets.
    pub dice_size_large: u8,
    /// `[4]` `bonusLarge` (sbyte) — flat damage bonus vs large targets.
    pub bonus_large: i8,
    /// `[5]` `numberAttacks` — HALF-attacks per round (folded through
    /// `ThisRoundActionCount`; doc §34.3).
    pub number_attacks: u8,
    /// `[6]` `field_6`.
    pub field_6: u8,
    /// `[7]` `field_7`.
    pub field_7: u8,
    /// `[8]` `field_8`.
    pub field_8: u8,
    /// `[9]` `diceCountNormal` — damage dice count vs man-sized targets.
    pub dice_count_normal: u8,
    /// `[0xA]` `diceSizeNormal` — damage die size vs man-sized targets.
    pub dice_size_normal: u8,
    /// `[0xB]` `bonusNormal` (sbyte) — flat damage bonus vs man-sized targets.
    pub bonus_normal: i8,
    /// `[0xC]` `range` — weapon range (1 = melee reach; `>1` = ranged).
    pub range: u8,
    /// `[0xD]` `classFlags` — which classes may wield it.
    pub class_flags: u8,
    /// `[0xE]` `field_E` — the [`flags`] behaviour bitfield.
    pub flags: u8,
    /// `[0xF]` `field_F`.
    pub field_f: u8,
}

impl ItemData {
    /// Decode one 16-byte entry (`ItemData(byte[] data, int offset)`,
    /// coab `ItemData.cs:91`). Shorter slices zero-fill the missing tail.
    fn from_bytes(bytes: &[u8]) -> Self {
        let b = |o: usize| bytes.get(o).copied().unwrap_or(0);
        ItemData {
            item_slot: b(0),
            hands_count: b(1),
            dice_count_large: b(2),
            dice_size_large: b(3),
            bonus_large: b(4) as i8,
            number_attacks: b(5),
            field_6: b(6),
            field_7: b(7),
            field_8: b(8),
            dice_count_normal: b(9),
            dice_size_normal: b(0xA),
            bonus_normal: b(0xB) as i8,
            range: b(0xC),
            class_flags: b(0xD),
            flags: b(0xE),
            field_f: b(0xF),
        }
    }

    /// `field_E & arrows` — draws from the arrows ammo slot.
    pub fn is_arrows(&self) -> bool {
        self.flags & flags::ARROWS != 0
    }
    /// `field_E & quarrels` — draws from the quarrels ammo slot.
    pub fn is_quarrels(&self) -> bool {
        self.flags & flags::QUARRELS != 0
    }
    /// `field_E & melee` — usable in hand-to-hand.
    pub fn is_melee(&self) -> bool {
        self.flags & flags::MELEE != 0
    }
    /// `field_E & flag_08` — a launcher (bow/crossbow), draws ammo.
    pub fn is_launcher(&self) -> bool {
        self.flags & flags::FLAG_08 != 0
    }
    /// `field_E & flag_10` — self-launching (thrown / sling): the item itself
    /// is the missile.
    pub fn is_self_launching(&self) -> bool {
        self.flags & flags::FLAG_10 != 0
    }
}

/// The resident item table — `0x81` [`ItemData`] entries indexed by item type
/// (`ItemDataTable`, coab `Classes/ItemData.cs:38`).
#[derive(Debug, Clone)]
pub struct ItemDataTable {
    entries: Vec<ItemData>,
}

impl ItemDataTable {
    /// Parse an `ITEMS` file image (coab `ItemDataTable(string fileName)`,
    /// `ItemData.cs:42`). Skips the 2-byte header and builds exactly
    /// [`ITEM_TYPE_COUNT`] entries, taking each from the file where present and
    /// **zero-filling beyond the file's end** — the faithful equivalent of
    /// coab reading `0x810` bytes into a zeroed buffer. Errors only when the
    /// file is too short to even hold the header.
    pub fn parse(bytes: &[u8]) -> Result<Self, ItemsParseError> {
        if bytes.len() < ITEMS_HEADER_SIZE {
            return Err(ItemsParseError::TooShort { len: bytes.len() });
        }
        let mut entries = Vec::with_capacity(ITEM_TYPE_COUNT);
        for i in 0..ITEM_TYPE_COUNT {
            let off = ITEMS_HEADER_SIZE + i * ITEM_ENTRY_SIZE;
            // Zero-fill entries the file does not cover (coab's zeroed buffer).
            let slice = bytes.get(off..off + ITEM_ENTRY_SIZE).unwrap_or(&[]);
            entries.push(ItemData::from_bytes(slice));
        }
        Ok(ItemDataTable { entries })
    }

    /// The entry for `item_type` (`this[ItemType index]`, coab `ItemData.cs:61`).
    /// Types `>= ITEM_TYPE_COUNT` (only `Type_128 = 0x80` is defined, and it is
    /// zero) return the zeroed entry, matching coab's array bound.
    pub fn get(&self, item_type: u8) -> ItemData {
        self.entries
            .get(item_type as usize)
            .copied()
            .unwrap_or_default()
    }

    /// The full entry slice (`0x81` entries), for tooling / inspection.
    pub fn entries(&self) -> &[ItemData] {
        &self.entries
    }
}

impl std::ops::Index<u8> for ItemDataTable {
    type Output = ItemData;
    fn index(&self, item_type: u8) -> &ItemData {
        // Types beyond the table are never readied in practice; a debug build
        // still bounds-checks, so index inside the fixed 0x81 window.
        &self.entries[item_type as usize]
    }
}

/// A typed `ITEMS` parse failure (untrusted-input posture, D-SAVE10 — never
/// panics on malformed input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemsParseError {
    /// The file is shorter than the 2-byte header.
    TooShort { len: usize },
}

impl fmt::Display for ItemsParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemsParseError::TooShort { len } => {
                write!(
                    f,
                    "ITEMS file too short: {len} bytes (need >= {ITEMS_HEADER_SIZE})"
                )
            }
        }
    }
}

impl std::error::Error for ItemsParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic `ITEMS` image: a 2-byte header then the given entries.
    fn synth(header: [u8; 2], entries: &[[u8; 16]]) -> Vec<u8> {
        let mut v = header.to_vec();
        for e in entries {
            v.extend_from_slice(e);
        }
        v
    }

    #[test]
    fn parses_header_and_first_entry() {
        // A LongBow-shaped entry (doc §34.1 row 43): hands 2, 1d6 large,
        // natk 4, 1d6 normal, range 22, flags 0x0B.
        let long_bow = [
            0x00, 0x02, 0x01, 0x06, 0x00, 0x04, 0x00, 0x01, 0x80, 0x01, 0x06, 0x00, 0x16, 0xC8,
            0x0B, 0x00,
        ];
        let img = synth([0x00, 0x76], &[long_bow]);
        let table = ItemDataTable::parse(&img).unwrap();
        let e = table.get(0);
        assert_eq!(e.item_slot, 0);
        assert_eq!(e.hands_count, 2);
        assert_eq!((e.dice_count_large, e.dice_size_large), (1, 6));
        assert_eq!(e.number_attacks, 4);
        assert_eq!((e.dice_count_normal, e.dice_size_normal), (1, 6));
        assert_eq!(e.bonus_normal, 0);
        assert_eq!(e.range, 22);
        assert_eq!(e.class_flags, 0xC8);
        assert_eq!(e.flags, 0x0B);
        assert!(e.is_arrows());
        assert!(e.is_launcher());
        assert!(!e.is_self_launching());
        assert!(!e.is_melee());
    }

    #[test]
    fn signed_bonus_and_flags_helpers() {
        // A Sling (row 47): 0x0A flags = flag_08 (launcher) | flag_02, 1d4+1
        // normal, +1 large, range 21. GetCurrentAttackItem "finds" a null item
        // for it (doc §34.2) — a launcher that draws no ammo slot.
        let sling = [
            0x00, 0x01, 0x01, 0x06, 0x01, 0x02, 0x00, 0x80, 0x80, 0x01, 0x04, 0x01, 0x15, 0xDC,
            0x0A, 0x00,
        ];
        // A HandAxe-shaped entry: 0x14 = flag_10 (self-launching) | melee — a
        // thrown weapon also usable in hand (ranged-melee, doc §34.2).
        let mut hand_axe = [0u8; 16];
        hand_axe[0xE] = 0x14;
        // A negative-bonus entry to exercise the sbyte cast.
        let mut neg = [0u8; 16];
        neg[0xB] = 0xFF; // bonusNormal = -1
        neg[4] = 0xFE; // bonusLarge = -2
        let img = synth([0x00, 0x00], &[sling, hand_axe, neg]);
        let table = ItemDataTable::parse(&img).unwrap();
        let s = table.get(0);
        assert_eq!(s.flags, 0x0A);
        assert!(s.is_launcher());
        assert!(!s.is_self_launching());
        assert_eq!(
            (s.dice_count_normal, s.dice_size_normal, s.bonus_normal),
            (1, 4, 1)
        );
        assert_eq!(s.range, 21);
        let h = table.get(1);
        assert!(h.is_self_launching());
        assert!(h.is_melee());
        assert_eq!(h.flags & 0x14, 0x14);
        let n = table.get(2);
        assert_eq!(n.bonus_normal, -1);
        assert_eq!(n.bonus_large, -2);
    }

    #[test]
    fn always_yields_full_table_zero_filling_the_tail() {
        // One entry only; the rest of the 0x81-entry table zero-fills, exactly
        // like coab reading a short file into its zeroed 0x810 buffer.
        let one = [7u8; 16];
        let img = synth([1, 2], &[one]);
        let table = ItemDataTable::parse(&img).unwrap();
        assert_eq!(table.entries().len(), ITEM_TYPE_COUNT);
        assert_eq!(table.get(0).item_slot, 7);
        assert_eq!(table.get(1), ItemData::default());
        assert_eq!(table.get(0x80), ItemData::default());
    }

    #[test]
    fn rejects_a_file_shorter_than_the_header() {
        assert!(matches!(
            ItemDataTable::parse(&[0x00]),
            Err(ItemsParseError::TooShort { len: 1 })
        ));
    }

    /// Local-tier (D10): parse Bryan's real `ITEMS` and verify the doc §34.1
    /// rows. Gated on the file's presence (`GBX_ITEMS_FILE` or the default
    /// cotab path) so plain CI, which lacks the game data, skips it — the file
    /// itself never enters the repo.
    #[test]
    fn parses_the_real_items_file() {
        let path = std::env::var_os("GBX_ITEMS_FILE")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| std::path::Path::new(&h).join("goldbox-data/cotab/ITEMS"))
            });
        let Some(path) = path.filter(|p| p.exists()) else {
            eprintln!("SKIPPED (D10): real ITEMS file absent");
            return;
        };
        let bytes = std::fs::read(&path).expect("ITEMS readable");
        let table = ItemDataTable::parse(&bytes).expect("ITEMS parses");

        let long_bow = table.get(43);
        assert_eq!(long_bow.hands_count, 2);
        assert_eq!(long_bow.number_attacks, 4);
        assert_eq!(long_bow.range, 22);
        assert_eq!(long_bow.flags, 0x0B);

        let short_bow = table.get(44);
        assert_eq!(short_bow.number_attacks, 4);
        assert_eq!(short_bow.range, 16);
        assert_eq!(short_bow.flags, 0x0B);

        let arrow = table.get(73);
        assert_eq!(arrow.item_slot, 10);
        assert_eq!(arrow.range, 0);
        assert_eq!(arrow.flags, 0x00);

        let sling = table.get(47);
        assert_eq!(sling.hands_count, 1);
        assert_eq!(sling.range, 21);
        assert_eq!(sling.flags, 0x0A);
        assert_eq!(
            (
                sling.dice_count_normal,
                sling.dice_size_normal,
                sling.bonus_normal
            ),
            (1, 4, 1)
        );

        let long_sword = table.get(36);
        assert_eq!(long_sword.range, 0);
        assert_eq!(long_sword.flags, 0x04);
        assert_eq!(
            (long_sword.dice_count_normal, long_sword.dice_size_normal),
            (1, 8)
        );
    }
}
