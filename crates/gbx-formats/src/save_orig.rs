//! Original CotAB save import (`docs/design/save-formats.md` v2, task
//! deliverable 1): the `savgam<X>.dat` master-container parser (§1.1) and
//! the `CHRDAT` 0x1A6 character-record decoder (§1.3), plus the `.swg`
//! (items) / `.fx` (affects) sized-blob readers. Pure over bytes — no
//! filesystem access (`gbx-formats`' established convention, `game_data.rs`);
//! callers pull the relevant files out of an already-loaded [`crate::game_data::GameData`]
//! (the same directory a real save was written into holds both the game
//! assets and, if DOSBox was pointed there, the save files).
//!
//! Import is one-way (D-SAVE12): these types only decode. `gbx-engine`
//! assembles the decoded data into a live `Engine` (D-SAVE5). Untrusted-input
//! posture (D-SAVE10/§4): every parse function returns a typed
//! [`SaveParseError`], never panics — this module joins the fuzz roster.
//!
//! **The three pinned cells (§1.7 items 1, 2, 5, D-SAVE6).** Each is a named
//! constant with a single flip point, defaulting per the task brief (all
//! three independently corroborated — item 2 by Fable's GBC-editor
//! cross-check against real save data, per this project's M3 save-format
//! design-door review):
//! - [`STAT_BYTE_ORDER`] — coab's `data[+0]=cur, data[+1]=full`
//!   (`Player.cs:77-88`), not GBC-doc's opposite reading.
//! - [`SPELL_CAST_COUNT_STRIDE`] — `5` (GBC-doc's clean 3×5 layout), not
//!   coab's self-overlapping `i*i` transcription bug (`Player.cs:727,769`).
//! - [`STR_EXCEPTIONAL_RANGE`] — `0..=100`, never coab's `Math.Min(_, 25)`
//!   read-time corruption (`Player.cs:86-87`) applied to the other six
//!   stats' `0..=25` domain.

use std::fmt;

// ---------------------------------------------------------------------
// The master container (`savgam<X>.dat`, §1.1)
// ---------------------------------------------------------------------

/// The flat `savgam<X>.dat` size (§1.1): sum of all 11 fixed sections,
/// `ovr017.cs:1150-1191`.
pub const SAVGAM_SIZE: usize = 13149;

const SEC_GAME_AREA: usize = 1;
const SEC_AREA_PTR: usize = 0x800;
const SEC_AREA2_PTR: usize = 0x800;
const SEC_STRU_1B2CA: usize = 0x400;
const SEC_ECL_PTR: usize = 0x1E00;
const SEC_POSITION: usize = 5;
const SEC_LAST_GAME_STATE: usize = 1;
const SEC_GAME_STATE: usize = 1;
const SEC_SET_BLOCKS: usize = 12;
const SEC_PARTY_COUNT: usize = 1;
const SEC_CHAR_NAMES: usize = 0x148;

/// One `"CHRDAT<X><n>"` name slot (§1.1 row 11): a Pascal-style
/// length-prefixed string, `Sys.StringToArray(data, 0x29*i, 0x29, name)`
/// (`ovr017.cs:1184`) — the length byte is always the slot width (`0x29`),
/// not the name's real length (a genuine coab quirk; harmless, since
/// [`Sys::ArrayToString`]-equivalent decoding skips zero bytes regardless).
const CHAR_NAME_SLOT: usize = 0x29;
const CHAR_NAME_SLOTS: usize = 8;

// Byte offsets *within* the 0x800-byte `area_ptr` blob (coab `Area1.cs`
// `[DataOffset]` attributes — verified directly against source for
// `current_3DMap_block_id`/the clock cluster; the remainder transcribed from
// `docs/design/save-formats.md` §1.4, which cites the same class).
const AREA_CURRENT_3D_MAP_BLOCK_ID: usize = 0x18A;
const AREA_CLOCK_MINUTES_ONES: usize = 0x18E;
const AREA_CLOCK_MINUTES_TENS: usize = 0x190;
const AREA_CLOCK_HOUR: usize = 0x192;
const AREA_CLOCK_DAY: usize = 0x194;
const AREA_CLOCK_YEAR: usize = 0x196;
const AREA_IN_DUNGEON: usize = 0x1CC;
const AREA_LAST_XPOS: usize = 0x1E0;
const AREA_LAST_YPOS: usize = 0x1E2;
const AREA_LAST_ECL_BLOCK_ID: usize = 0x1E4;

// Byte offsets within the 0x800-byte `area2_ptr` blob (coab `Area2.cs`,
// transcribed from `docs/design/save-formats.md` §1.4).
const AREA2_TRAINING_CLASS_MASK: usize = 0x550;
const AREA2_SEARCH_FLAGS: usize = 0x594;
const AREA2_GAME_AREA: usize = 0x624;
const AREA2_HEAD_BLOCK_ID: usize = 0x5C2;
const AREA2_ENTER_TEMPLE: usize = 0x5C4;
const AREA2_TRIED_TO_EXIT_MAP: usize = 0x5AA;
const AREA2_ENTER_SHOP: usize = 0x6D8;
const AREA2_PARTY_SIZE: usize = 0x67C;

fn le16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

/// [`parse_master`]/[`decode_char_record`]/[`read_items`]/[`read_affects`]'s
/// failure mode. Malformed original-save bytes are always user data (a
/// hand-edited or corrupt save file) — never a panic (D-SAVE10/§4 fuzz
/// posture).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveParseError {
    /// `savgam<X>.dat` isn't exactly [`SAVGAM_SIZE`] bytes.
    WrongMasterSize { got: usize },
    /// A `CHRDAT<X><n>.sav` record isn't exactly `0x1A6` bytes.
    WrongRecordSize { got: usize },
    /// A `.swg`/`.fx` side file's length isn't a whole multiple of its
    /// record size.
    TruncatedBlobFile { len: usize, record_size: usize },
    /// `party_count` in section 10 exceeds the 8 name slots section 11 holds.
    PartyCountTooLarge { party_count: u8 },
}

impl fmt::Display for SaveParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveParseError::WrongMasterSize { got } => write!(
                f,
                "savgam?.dat must be exactly {SAVGAM_SIZE} bytes, got {got}"
            ),
            SaveParseError::WrongRecordSize { got } => {
                write!(f, "CHRDAT record must be exactly 0x1A6 bytes, got {got:#x}")
            }
            SaveParseError::TruncatedBlobFile { len, record_size } => write!(
                f,
                "blob file length {len} is not a multiple of record size {record_size}"
            ),
            SaveParseError::PartyCountTooLarge { party_count } => write!(
                f,
                "party_count {party_count} exceeds the 8 stored character-name slots"
            ),
        }
    }
}

impl std::error::Error for SaveParseError {}

/// One `setBlocks[i]` entry (§1.1 row 9): `{blockId, setId}`, both LE `i16`
/// (`Sys.ShortToArray`, `ovr017.cs:1168-1173`). `LoadWalldef(setId, blockId)`
/// is the reload call order (§1.5) — note the *write* order on disk is
/// `blockId` then `setId` (this struct's field order), the *reload call*
/// takes them in the opposite order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SetBlock {
    pub block_id: i16,
    pub set_id: i16,
}

/// The decoded `savgam<X>.dat` master container (§1.1's 11 sections, a flat
/// 13149-byte read, no decompression). Section 5 (`ecl_ptr`, the resident
/// ECL block bytes) is parsed for structural completeness only — its
/// content is never consulted: `loadSaveGame` discards it too (§1.5), always
/// reloading a pristine block by `last_ecl_block_id` from `GameData`
/// (D-SAVE5).
#[derive(Debug, Clone)]
pub struct MasterSave {
    pub game_area: u8,
    pub area_ptr: Box<[u8; SEC_AREA_PTR]>,
    pub area2_ptr: Box<[u8; SEC_AREA2_PTR]>,
    pub stru_1b2ca: Box<[u8; SEC_STRU_1B2CA]>,
    /// Discarded on import (§1.5) — retained so a round-trip test can assert
    /// this section was read at the right offset/size (D-SAVE10 tier 1);
    /// never consulted by [`crate::save_orig`] consumers otherwise.
    pub ecl_ptr_discarded: Box<[u8; SEC_ECL_PTR]>,
    pub map_pos_x: u8,
    pub map_pos_y: u8,
    pub map_direction: u8,
    pub map_wall_type: u8,
    pub map_wall_roof: u8,
    pub last_game_state: u8,
    pub game_state: u8,
    pub set_blocks: [SetBlock; 3],
    pub party_count: u8,
    /// The 8 `"CHRDAT<X><n>"` name slots (§1.1 row 11) — only the first
    /// `party_count` are meaningful; a fresh save zeroes the rest.
    pub char_file_names: [String; CHAR_NAME_SLOTS],
}

impl MasterSave {
    /// `current_3DMap_block_id` (`Area1.cs:45`, byte, offset `0x18A`) — the
    /// resident GEO block id (§1.5).
    pub fn current_3d_map_block_id(&self) -> u8 {
        self.area_ptr[AREA_CURRENT_3D_MAP_BLOCK_ID]
    }

    /// `LastEclBlockId` (word, offset `0x1E4`) — which script block is
    /// resident; import reloads pristine bytes by this id, never the
    /// discarded section-5 bytes (§1.5, D-SAVE5).
    pub fn last_ecl_block_id(&self) -> u16 {
        le16(&self.area_ptr[..], AREA_LAST_ECL_BLOCK_ID)
    }

    /// `inDungeon` (word, offset `0x1CC`) — drives `game_state`
    /// (Dungeon/Wilderness) on import, matching the live engine's own
    /// write-hook semantics (`vmhost.rs`'s `IN_DUNGEON_ADDR`).
    pub fn in_dungeon(&self) -> bool {
        le16(&self.area_ptr[..], AREA_IN_DUNGEON) != 0
    }

    /// `lastXPos`/`lastYPos` (words, offsets `0x1E0`/`0x1E2`) — a shadow
    /// copy of the position the section-6 `mapPosX`/`mapPosY` bytes already
    /// carry authoritatively (§1.4: "duplicates lastXPos/Y semantics at the
    /// engine level"); exposed for round-trip/consistency assertions, not
    /// used as the import's primary position source.
    pub fn last_pos(&self) -> (u16, u16) {
        (
            le16(&self.area_ptr[..], AREA_LAST_XPOS),
            le16(&self.area_ptr[..], AREA_LAST_YPOS),
        )
    }

    /// The 5 ECL-clock words (`time_minutes_ones/tens`, `time_hour`,
    /// `time_day`, `time_year`; offsets `0x18E..=0x196`) — matches
    /// `GameClock::raw_clock_words()`'s inner 5 (excluding its two always-0
    /// bracketing words).
    pub fn clock_words(&self) -> [u16; 5] {
        [
            le16(&self.area_ptr[..], AREA_CLOCK_MINUTES_ONES),
            le16(&self.area_ptr[..], AREA_CLOCK_MINUTES_TENS),
            le16(&self.area_ptr[..], AREA_CLOCK_HOUR),
            le16(&self.area_ptr[..], AREA_CLOCK_DAY),
            le16(&self.area_ptr[..], AREA_CLOCK_YEAR),
        ]
    }

    /// `search_flags` (word, offset `0x594` in `area2_ptr`) — bit 0
    /// searching / bit 1 looking (§1.4).
    pub fn search_flags(&self) -> u16 {
        le16(&self.area2_ptr[..], AREA2_SEARCH_FLAGS)
    }

    /// `HeadBlockId` (word, offset `0x5C2` in `area2_ptr`).
    pub fn head_block_id(&self) -> u16 {
        le16(&self.area2_ptr[..], AREA2_HEAD_BLOCK_ID)
    }

    /// `tried_to_exit_map` (word, offset `0x5AA` in `area2_ptr`).
    pub fn tried_to_exit_map(&self) -> bool {
        le16(&self.area2_ptr[..], AREA2_TRIED_TO_EXIT_MAP) != 0
    }

    /// `training_class_mask` (word, offset `0x550` in `area2_ptr`).
    pub fn training_class_mask(&self) -> u16 {
        le16(&self.area2_ptr[..], AREA2_TRAINING_CLASS_MASK)
    }

    /// `EnterTemple`/`EnterShop` (words, offsets `0x5C4`/`0x6D8` in
    /// `area2_ptr`).
    pub fn enter_temple_shop(&self) -> (u16, u16) {
        (
            le16(&self.area2_ptr[..], AREA2_ENTER_TEMPLE),
            le16(&self.area2_ptr[..], AREA2_ENTER_SHOP),
        )
    }

    /// `area2_ptr.game_area` (word, offset `0x624`) and `.party_size` (word,
    /// offset `0x67C`) — exposed for cross-checks against section 1's
    /// `game_area` / section 10's `party_count`, which import treats as
    /// authoritative (they're the simpler, direct reads).
    pub fn area2_game_area_and_party_size(&self) -> (u16, u16) {
        (
            le16(&self.area2_ptr[..], AREA2_GAME_AREA),
            le16(&self.area2_ptr[..], AREA2_PARTY_SIZE),
        )
    }
}

/// Reads a Pascal-style length-prefixed string at `data[offset..]`:
/// `len = min(data[offset], cap)`, then `len` bytes, skipping zero bytes
/// (`Sys::ArrayToString`, `Classes/Sys.cs:46-63`) — used for both the
/// character-file-name slots and the character record's `name` field.
fn read_pstring(data: &[u8], offset: usize, cap: usize) -> String {
    let len = (data[offset] as usize).min(cap);
    let mut s = String::with_capacity(len);
    for i in 1..=len {
        let c = data[offset + i];
        if c > 0 {
            s.push(c as char);
        }
    }
    s
}

/// Parses a flat `savgam<X>.dat` buffer into its 11 sections (§1.1's write
/// order is the read order — a straight sequential transcription, no
/// decompression).
pub fn parse_master(bytes: &[u8]) -> Result<MasterSave, SaveParseError> {
    if bytes.len() != SAVGAM_SIZE {
        return Err(SaveParseError::WrongMasterSize { got: bytes.len() });
    }
    let mut off = 0usize;
    let mut take = |len: usize| {
        let slice = &bytes[off..off + len];
        off += len;
        slice
    };

    let game_area = take(SEC_GAME_AREA)[0];
    let area_ptr: Box<[u8; SEC_AREA_PTR]> = Box::new(take(SEC_AREA_PTR).try_into().unwrap());
    let area2_ptr: Box<[u8; SEC_AREA2_PTR]> = Box::new(take(SEC_AREA2_PTR).try_into().unwrap());
    let stru_1b2ca: Box<[u8; SEC_STRU_1B2CA]> = Box::new(take(SEC_STRU_1B2CA).try_into().unwrap());
    let ecl_ptr_discarded: Box<[u8; SEC_ECL_PTR]> = Box::new(take(SEC_ECL_PTR).try_into().unwrap());

    let position = take(SEC_POSITION);
    let (map_pos_x, map_pos_y, map_direction, map_wall_type, map_wall_roof) = (
        position[0],
        position[1],
        position[2],
        position[3],
        position[4],
    );

    let last_game_state = take(SEC_LAST_GAME_STATE)[0];
    let game_state = take(SEC_GAME_STATE)[0];

    let set_blocks_bytes = take(SEC_SET_BLOCKS);
    let mut set_blocks = [SetBlock::default(); 3];
    for (i, sb) in set_blocks.iter_mut().enumerate() {
        sb.block_id = le16(set_blocks_bytes, i * 4) as i16;
        sb.set_id = le16(set_blocks_bytes, i * 4 + 2) as i16;
    }

    let party_count = take(SEC_PARTY_COUNT)[0];
    if party_count as usize > CHAR_NAME_SLOTS {
        return Err(SaveParseError::PartyCountTooLarge { party_count });
    }

    let names_bytes = take(SEC_CHAR_NAMES);
    let char_file_names =
        std::array::from_fn(|i| read_pstring(names_bytes, i * CHAR_NAME_SLOT, CHAR_NAME_SLOT - 1));

    debug_assert_eq!(off, SAVGAM_SIZE);

    Ok(MasterSave {
        game_area,
        area_ptr,
        area2_ptr,
        stru_1b2ca,
        ecl_ptr_discarded,
        map_pos_x,
        map_pos_y,
        map_direction,
        map_wall_type,
        map_wall_roof,
        last_game_state,
        game_state,
        set_blocks,
        party_count,
        char_file_names,
    })
}

// ---------------------------------------------------------------------
// The character record (`CHRDAT<X><n>.sav`, 0x1A6 bytes, §1.3)
// ---------------------------------------------------------------------

/// The 0x1A6-byte record size (`Player.StructSize`, `Player.cs:708`).
pub const CHAR_RECORD_SIZE: usize = 0x1A6;

/// One ability score's two on-disk bytes, decoded per [`STAT_BYTE_ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawStat {
    pub current: u8,
    pub original: u8,
}

/// §1.7 item 1's flip point: which on-disk byte is `current` vs `original`
/// (`full`) for every `Cust1Array` `StatValue` pair (`stats2` @ 0x10, 7
/// entries incl. Str00's exceptional-percentile pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatByteOrder {
    /// coab: `data[+0] = cur`, `data[+1] = full` (`Player.cs:77-88`,
    /// `DataIO.cs`'s `Cust1Array` read/write). The default — coab
    /// transliterates the binary's own field I/O directly, the stronger
    /// authority per D11 (§1.7 item 1).
    CoabCurFull,
    /// GBC-doc's independently-documented opposite reading ("str original"
    /// @0x10, "str current" @0x11) — the flip target if real-data pinning
    /// (D-SAVE10 tier 3, a drained-stat character) contradicts the default.
    GbcOriginalCurrent,
}

/// §1.7 item 1's single flip point (D-SAVE6) — defaults to coab's own
/// transliterated read order. Flip to `GbcOriginalCurrent` only if a real
/// drained-stat save (D-SAVE10 tier 3 procedure, §5.2) contradicts it.
pub const STAT_BYTE_ORDER: StatByteOrder = StatByteOrder::CoabCurFull;

fn read_stat(pair: [u8; 2]) -> RawStat {
    match STAT_BYTE_ORDER {
        StatByteOrder::CoabCurFull => RawStat {
            current: pair[0],
            original: pair[1],
        },
        StatByteOrder::GbcOriginalCurrent => RawStat {
            current: pair[1],
            original: pair[0],
        },
    }
}

/// §1.7 item 2's single flip point (D-SAVE6): the `spellCastCount[3,5]`
/// grid's row stride. `5` (GBC-doc's clean, non-overlapping 3×5 layout) —
/// **not** coab's self-overlapping `data[0x12d + j + i*i]` transcription bug
/// (`Player.cs:727,769`; row `i=1` at `i*i=1` collides with row `i=0`'s own
/// tail). Already corroborated: Fable's M3 save-format review cross-checked
/// this against GBC's editor operating on real saves at stride 5 — this is
/// the strongest-evidence of the three pinned cells, not merely a default.
pub const SPELL_CAST_COUNT_STRIDE: usize = 5;

/// §1.7 item 5's flip point (D-SAVE6): the valid range for the
/// exceptional-strength percentile (`Str00`, the 7th `stats2` entry). coab's
/// `StatValue.Read` wrongly applies `Math.Min(_, 25)` to every stat
/// including this one (`Player.cs:86-87`); the six *main* ability scores
/// legitimately live in `0..=25`, but Str00 is `Load(Random(100)+1)` in play
/// (`ovr018.cs:703`) and must not be clamped. This module never applies
/// coab's clamp at decode time (that's the actual fix, not something this
/// constant toggles) — the constant instead names the *validation* range a
/// bounds-check (D-SAVE10 tier 2) should use, with a single flip point back
/// to `0..=25` should real-data pinning ever contradict the default.
pub const STR_EXCEPTIONAL_RANGE: std::ops::RangeInclusive<u8> = 0..=100;
/// The six main ability scores' validation range (unaffected by §1.7 item 5
/// — always `0..=25`, coab's clamp and GBC-doc agree here).
pub const MAIN_STAT_RANGE: std::ops::RangeInclusive<u8> = 0..=25;

/// The seven ability scores, in storage order (`Player.cs:107-116`): STR,
/// INT, WIS, DEX, CON, CHA, then Str00 (exceptional-strength percentile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawStatBlock {
    pub str: RawStat,
    pub int: RawStat,
    pub wis: RawStat,
    pub dex: RawStat,
    pub con: RawStat,
    pub cha: RawStat,
    pub str_exceptional: RawStat,
}

/// The decoded 0x1A6-byte character record (§1.3's table, transcribed
/// exactly). Pointer fields (§1.7 item 3: the affects-list pointer @0xf2,
/// items-list pointer @0x14d, the 13-pointer `activeItems` array @0x151,
/// next-char pointer @0x189, actions pointer @0x18d) are **not** represented
/// here at all — they are disk garbage on any real save (`DataIO` never
/// attributes them) and are simply skipped during decode, per D-SAVE6.
/// Equipment is reconstructed post-item-load from readied flags, not from
/// these bytes.
///
/// Fields with no established sub-byte layout (icon nibble-packs, the
/// current/base attack-profile byte runs) are carried as opaque raw arrays
/// rather than guessed apart — D-SAVE11's "every remaining record byte held
/// as an opaque named field" discipline, applied at the format layer too.
#[derive(Debug, Clone, PartialEq)]
pub struct CharRecord {
    pub name: String,
    pub stats: RawStatBlock,
    /// `spellList` @0x1e, 84 bytes: the per-slot memorized-spell list.
    /// Interpretation (which byte = which spell) is a rules/party-model
    /// concern (§5 item 7), not pinned here.
    pub spell_list: Vec<u8>,
    pub spell_to_learn_count: u8,
    pub thac0_base: i8,
    pub race: u8,
    pub class: u8,
    pub age: i16,
    pub hit_point_max: u8,
    /// `spellBook[100]` @0x79: per-spell known flags.
    pub spell_book: Vec<u8>,
    pub attack_level: u8,
    /// `field_DE` @0xde: icon dimensions.
    pub field_de: u8,
    /// `saveVerse[5]` @0xdf: paralyze/petrify/rod/breath/spell.
    pub save_verse: [u8; 5],
    pub base_movement: u8,
    pub hit_dice: u8,
    pub multiclass_level: u8,
    pub lost_lvls: u8,
    pub lost_hp: u8,
    /// `field_E9` @0xe9: turn-undead type index.
    pub field_e9: u8,
    /// `thief_skills[8]` @0xea: pick/locks/traps/silent/hide/hear/climb/read.
    pub thief_skills: [u8; 8],
    /// `field_F6` @0xf6.
    pub field_f6: u8,
    /// `control_morale` @0xf7: `>= 0x80` means NPC (`Control.cs:322`).
    pub control_morale: u8,
    pub npc_treasure_share_count: u8,
    /// `field_F9`/`field_FA` @0xf9-0xfa.
    pub field_f9_fa: [u8; 2],
    /// `Money` @0xfb, 7 × i16 LE: copper/silver/electrum/gold/plat/gems/jewelry.
    pub money: [i16; 7],
    /// `ClassLevel[8]` @0x109: current per-class levels.
    pub class_level: [u8; 8],
    /// `ClassLevelsOld[8]` @0x111: former (dual-class) levels.
    pub class_levels_old: [u8; 8],
    pub sex: u8,
    pub monster_type: u8,
    pub alignment: u8,
    /// `attacksCount, baseHalfMoves, attack1/2 dice-base ×6` @0x11c, 8 bytes:
    /// opaque (no established sub-byte layout, §1.3 notes).
    pub attack_profile_base: [u8; 8],
    pub base_ac: i8,
    /// `field_125` @0x125.
    pub field_125: u8,
    /// `mod_id` @0x126: monster index.
    pub mod_id: u8,
    pub exp: i32,
    /// `classFlags` @0x12b: item limits.
    pub class_flags: u8,
    pub hit_point_rolled: u8,
    /// `spellCastCount[3,5]` @0x12d: cleric/druid/mage memorized-cast
    /// counts, decoded with [`SPELL_CAST_COUNT_STRIDE`] (§1.7 item 2).
    pub spell_cast_count: [[u8; 5]; 3],
    /// `field_13C` @0x13c: xp award.
    pub field_13c: i16,
    /// `field_13E/13F/140` @0x13e: xp bonus/hp, ??? ×2.
    pub field_13e_140: [u8; 3],
    pub head_icon: u8,
    pub weapon_icon: u8,
    pub icon_id: u8,
    pub icon_size: u8,
    /// `icon_colours[6]` @0x145: nibble-packed pairs, unpacked by a future
    /// consumer (`&0x0F`/`>>4`, `LoadPlayerCombatIcon`).
    pub icon_colours: [u8; 6],
    /// `field_14B` @0x14b: flags 1.
    pub field_14b: u8,
    /// `weaponsHandsUsed` @0x185.
    pub weapons_hands_used: u8,
    /// `field_186` @0x186: save bonus (signed).
    pub field_186: i8,
    pub weight: i16,
    /// `paladinCuresLeft` @0x191.
    pub paladin_cures_left: u8,
    /// `field_192/193/194` @0x192, 3 bytes.
    pub field_192_194: [u8; 3],
    /// `health_status` @0x195: 0=okay..8=gone.
    pub health_status: u8,
    pub in_combat: bool,
    pub combat_team: u8,
    pub quick_fight: u8,
    /// `hitBonus` @0x199: current THAC0.
    pub hit_bonus: u8,
    /// `ac` @0x19a: display AC = `0x3C - ac`.
    pub ac: i8,
    pub ac_behind: i8,
    /// `attack1/2 left/dice-count/dice-size/dmg-bonus` @0x19c, 8 bytes:
    /// opaque (current-attack counterpart of [`Self::attack_profile_base`]).
    pub attack_profile_current: [u8; 8],
    pub hit_point_current: u8,
    /// `movement` @0x1a5: current movement/initiative.
    pub movement: u8,
}

/// Decodes a `CHRDAT<X><n>.sav` record — a straight transcription of §1.3's
/// offset table, with the three pinned cells resolved per
/// [`STAT_BYTE_ORDER`]/[`SPELL_CAST_COUNT_STRIDE`]/[`STR_EXCEPTIONAL_RANGE`]
/// (no clamping applied at decode time) and every pointer field (§1.7 item
/// 3) skipped rather than stored.
pub fn decode_char_record(bytes: &[u8]) -> Result<CharRecord, SaveParseError> {
    if bytes.len() != CHAR_RECORD_SIZE {
        return Err(SaveParseError::WrongRecordSize { got: bytes.len() });
    }
    let u8_at = |o: usize| bytes[o];
    let i8_at = |o: usize| bytes[o] as i8;
    let i16_at = |o: usize| le16(bytes, o) as i16;
    let i32_at =
        |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let arr_at = |o: usize, n: usize| bytes[o..o + n].to_vec();
    let fixed2 = |o: usize| [bytes[o], bytes[o + 1]];
    let fixed3 = |o: usize| [bytes[o], bytes[o + 1], bytes[o + 2]];
    let fixed5 = |o: usize| {
        [
            bytes[o],
            bytes[o + 1],
            bytes[o + 2],
            bytes[o + 3],
            bytes[o + 4],
        ]
    };
    let fixed6 = |o: usize| {
        [
            bytes[o],
            bytes[o + 1],
            bytes[o + 2],
            bytes[o + 3],
            bytes[o + 4],
            bytes[o + 5],
        ]
    };
    let fixed8 = |o: usize| {
        let mut a = [0u8; 8];
        a.copy_from_slice(&bytes[o..o + 8]);
        a
    };

    let stats = RawStatBlock {
        str: read_stat(fixed2(0x10)),
        int: read_stat(fixed2(0x12)),
        wis: read_stat(fixed2(0x14)),
        dex: read_stat(fixed2(0x16)),
        con: read_stat(fixed2(0x18)),
        cha: read_stat(fixed2(0x1a)),
        str_exceptional: read_stat(fixed2(0x1c)),
    };

    let mut spell_cast_count = [[0u8; 5]; 3];
    for (i, row) in spell_cast_count.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = bytes[0x12d + j + i * SPELL_CAST_COUNT_STRIDE];
        }
    }

    Ok(CharRecord {
        name: read_pstring(bytes, 0x00, 15),
        stats,
        spell_list: arr_at(0x1e, 84),
        spell_to_learn_count: u8_at(0x72),
        thac0_base: i8_at(0x73),
        race: u8_at(0x74),
        class: u8_at(0x75),
        age: i16_at(0x76),
        hit_point_max: u8_at(0x78),
        spell_book: arr_at(0x79, 100),
        attack_level: u8_at(0xdd),
        field_de: u8_at(0xde),
        save_verse: fixed5(0xdf),
        base_movement: u8_at(0xe4),
        hit_dice: u8_at(0xe5),
        multiclass_level: u8_at(0xe6),
        lost_lvls: u8_at(0xe7),
        lost_hp: u8_at(0xe8),
        field_e9: u8_at(0xe9),
        thief_skills: fixed8(0xea),
        // 0xf2..0xf6: affects-list pointer (§1.7 item 3) — skipped.
        field_f6: u8_at(0xf6),
        control_morale: u8_at(0xf7),
        npc_treasure_share_count: u8_at(0xf8),
        field_f9_fa: fixed2(0xf9),
        money: {
            let mut m = [0i16; 7];
            for (i, v) in m.iter_mut().enumerate() {
                *v = i16_at(0xfb + i * 2);
            }
            m
        },
        class_level: fixed8(0x109),
        class_levels_old: fixed8(0x111),
        sex: u8_at(0x119),
        monster_type: u8_at(0x11a),
        alignment: u8_at(0x11b),
        attack_profile_base: fixed8(0x11c),
        base_ac: i8_at(0x124),
        field_125: u8_at(0x125),
        mod_id: u8_at(0x126),
        exp: i32_at(0x127),
        class_flags: u8_at(0x12b),
        hit_point_rolled: u8_at(0x12c),
        spell_cast_count,
        field_13c: i16_at(0x13c),
        field_13e_140: fixed3(0x13e),
        head_icon: u8_at(0x141),
        weapon_icon: u8_at(0x142),
        icon_id: u8_at(0x143),
        icon_size: u8_at(0x144),
        icon_colours: fixed6(0x145),
        field_14b: u8_at(0x14b),
        // 0x14c: item count (commented out in coab); 0x14d..0x151: items-list
        // pointer; 0x151..0x185: activeItems (13 pointers) — all §1.7 item 3,
        // skipped.
        weapons_hands_used: u8_at(0x185),
        field_186: i8_at(0x186),
        weight: i16_at(0x187),
        // 0x189..0x18d: next-char pointer; 0x18d..0x191: actions pointer —
        // §1.7 item 3, skipped.
        paladin_cures_left: u8_at(0x191),
        field_192_194: fixed3(0x192),
        health_status: u8_at(0x195),
        in_combat: u8_at(0x196) != 0,
        combat_team: u8_at(0x197),
        quick_fight: u8_at(0x198),
        hit_bonus: u8_at(0x199),
        ac: i8_at(0x19a),
        ac_behind: i8_at(0x19b),
        attack_profile_current: fixed8(0x19c),
        hit_point_current: u8_at(0x1a4),
        movement: u8_at(0x1a5),
    })
}

// ---------------------------------------------------------------------
// `.swg` (items) / `.fx` (affects) sized-blob readers — opaque per §5.5
// ---------------------------------------------------------------------

/// `Item.StructSize` (`Classes/Item.cs:87`) — each `.swg` record's fixed
/// size. Record interiors are opaque this session (§5.5, deferred to the
/// party model/M4).
pub const ITEM_RECORD_SIZE: usize = 0x3F;

/// `Affect.StructSize` (`Classes/Affect.cs:164`) — each `.fx` record's fixed
/// size.
pub const AFFECT_RECORD_SIZE: usize = 9;

/// Splits a `.swg` file's bytes into fixed-size opaque item records.
pub fn read_items(bytes: &[u8]) -> Result<Vec<Vec<u8>>, SaveParseError> {
    read_fixed_records(bytes, ITEM_RECORD_SIZE)
}

/// Splits a `.fx` file's bytes into fixed-size opaque affect records.
pub fn read_affects(bytes: &[u8]) -> Result<Vec<Vec<u8>>, SaveParseError> {
    read_fixed_records(bytes, AFFECT_RECORD_SIZE)
}

/// One item record's `readied` flag (`Item.cs:30,132`: `bool readied`,
/// on-disk at byte offset `0x34`) — the *only* field this module extracts
/// from an otherwise-opaque `.swg` record (§5.5 defers full item-interior
/// decoding), because it's exactly what the party model needs to
/// reconstruct the readied-equipment set the record's own pointer-based
/// `activeItems` array can't provide (§1.7 item 3). `false` for a
/// short/malformed record rather than panicking.
pub fn item_readied(record: &[u8]) -> bool {
    record.get(0x34).is_some_and(|&b| b != 0)
}

fn read_fixed_records(bytes: &[u8], record_size: usize) -> Result<Vec<Vec<u8>>, SaveParseError> {
    if !bytes.len().is_multiple_of(record_size) {
        return Err(SaveParseError::TruncatedBlobFile {
            len: bytes.len(),
            record_size,
        });
    }
    Ok(bytes.chunks(record_size).map(<[u8]>::to_vec).collect())
}

// ---------------------------------------------------------------------
// The full file-set (D-SAVE5's `OriginalSaveSet`)
// ---------------------------------------------------------------------

/// One party member's on-disk record plus optional side files (§1.1: `.sav`
/// is required whenever the name slot is used; `.swg`/`.fx` are written only
/// when non-empty).
#[derive(Debug, Clone)]
pub struct OriginalChar {
    pub record: CharRecord,
    pub items: Vec<Vec<u8>>,
    pub affects: Vec<Vec<u8>>,
}

/// The full original-save file-set (D-SAVE5): the master container plus one
/// character (with optional side files) per `party_count`.
#[derive(Debug, Clone)]
pub struct OriginalSaveSet {
    pub master: MasterSave,
    pub chars: Vec<OriginalChar>,
}

/// [`load_from_lookup`]'s failure mode: parse errors plus a missing-required-
/// file case (a `.sav` record named in section 11 but absent from the file
/// set — malformed/incomplete user data, not a panic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportSetError {
    Parse(SaveParseError),
    MissingFile(String),
}

impl fmt::Display for ImportSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportSetError::Parse(e) => write!(f, "{e}"),
            ImportSetError::MissingFile(name) => write!(f, "missing required save file: {name}"),
        }
    }
}

impl std::error::Error for ImportSetError {}

impl From<SaveParseError> for ImportSetError {
    fn from(e: SaveParseError) -> Self {
        ImportSetError::Parse(e)
    }
}

/// Builds an [`OriginalSaveSet`] from a case-insensitive file lookup (e.g.
/// [`crate::game_data::GameData::raw_file`]) plus the master container's own
/// bytes and save slot letter — the crate stays filesystem-free (`game_data.rs`'s
/// convention: a save directory's files are just more entries in the same
/// in-memory file set a `GameData` already holds, since DOSBox can be
/// pointed at `GBX_DATA_DIR` for its save path too).
pub fn load_from_lookup<'a>(
    master_bytes: &[u8],
    slot: char,
    lookup: impl Fn(&str) -> Option<&'a [u8]>,
) -> Result<OriginalSaveSet, ImportSetError> {
    let master = parse_master(master_bytes)?;
    let mut chars = Vec::with_capacity(master.party_count as usize);
    for n in 1..=master.party_count {
        let base = format!("CHRDAT{slot}{n}");
        let sav_name = format!("{base}.SAV");
        let record_bytes = lookup(&sav_name).ok_or(ImportSetError::MissingFile(sav_name))?;
        let record = decode_char_record(record_bytes)?;
        let items = lookup(&format!("{base}.SWG"))
            .map(read_items)
            .transpose()?
            .unwrap_or_default();
        let affects = lookup(&format!("{base}.FX"))
            .map(read_affects)
            .transpose()?
            .unwrap_or_default();
        chars.push(OriginalChar {
            record,
            items,
            affects,
        });
    }
    Ok(OriginalSaveSet { master, chars })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a structurally-valid, hand-authored (D10-clean, self-authored
    /// bytes) 13149-byte `savgam?.dat` buffer with one party member, so
    /// decode tests don't need real game data.
    fn synthetic_master_bytes(party_count: u8) -> Vec<u8> {
        let mut buf = vec![0u8; SAVGAM_SIZE];
        let mut off = 0usize;
        buf[off] = 2; // game_area
        off += SEC_GAME_AREA;

        // area_ptr: set current_3DMap_block_id, LastEclBlockId, inDungeon, clock.
        let area = &mut buf[off..off + SEC_AREA_PTR];
        area[AREA_CURRENT_3D_MAP_BLOCK_ID] = 1;
        area[AREA_LAST_ECL_BLOCK_ID..AREA_LAST_ECL_BLOCK_ID + 2]
            .copy_from_slice(&7u16.to_le_bytes());
        area[AREA_IN_DUNGEON..AREA_IN_DUNGEON + 2].copy_from_slice(&1u16.to_le_bytes());
        area[AREA_CLOCK_MINUTES_ONES..AREA_CLOCK_MINUTES_ONES + 2]
            .copy_from_slice(&3u16.to_le_bytes());
        area[AREA_CLOCK_HOUR..AREA_CLOCK_HOUR + 2].copy_from_slice(&14u16.to_le_bytes());
        off += SEC_AREA_PTR;

        let area2 = &mut buf[off..off + SEC_AREA2_PTR];
        area2[AREA2_SEARCH_FLAGS..AREA2_SEARCH_FLAGS + 2].copy_from_slice(&1u16.to_le_bytes());
        area2[AREA2_HEAD_BLOCK_ID..AREA2_HEAD_BLOCK_ID + 2].copy_from_slice(&0xFFu16.to_le_bytes());
        off += SEC_AREA2_PTR;

        off += SEC_STRU_1B2CA;
        off += SEC_ECL_PTR;

        buf[off] = 7; // mapPosX
        buf[off + 1] = 13; // mapPosY
        buf[off + 2] = 0; // mapDirection (North)
        buf[off + 3] = 0;
        buf[off + 4] = 0;
        off += SEC_POSITION;

        buf[off] = 4; // last_game_state (DungeonMap, coab numbering)
        off += SEC_LAST_GAME_STATE;
        buf[off] = 4; // game_state
        off += SEC_GAME_STATE;

        let sb = &mut buf[off..off + SEC_SET_BLOCKS];
        sb[0..2].copy_from_slice(&5i16.to_le_bytes()); // setBlocks[0].blockId
        sb[2..4].copy_from_slice(&1i16.to_le_bytes()); // setBlocks[0].setId
        off += SEC_SET_BLOCKS;

        buf[off] = party_count;
        off += SEC_PARTY_COUNT;

        let names = &mut buf[off..off + SEC_CHAR_NAMES];
        for i in 0..party_count as usize {
            let name = format!("CHRDATA{}", i + 1);
            names[i * CHAR_NAME_SLOT] = CHAR_NAME_SLOT as u8; // coab's own quirk
            names[i * CHAR_NAME_SLOT + 1..i * CHAR_NAME_SLOT + 1 + name.len()]
                .copy_from_slice(name.as_bytes());
        }
        off += SEC_CHAR_NAMES;

        assert_eq!(off, SAVGAM_SIZE);
        buf
    }

    fn synthetic_char_bytes(name: &str) -> Vec<u8> {
        let mut buf = vec![0u8; CHAR_RECORD_SIZE];
        buf[0] = name.len() as u8;
        buf[1..1 + name.len()].copy_from_slice(name.as_bytes());
        // str current=17, original=18 (coab order: [+0]=cur, [+1]=full).
        buf[0x10] = 17;
        buf[0x11] = 18;
        // Str00 exceptional %, e.g. 91 (would corrupt to 25 under coab's clamp).
        buf[0x1c] = 91;
        buf[0x1d] = 91;
        buf[0x74] = 7; // race = human
        buf[0x75] = 2; // class
        buf[0x76..0x78].copy_from_slice(&25i16.to_le_bytes()); // age
        buf[0x78] = 42; // hp max
        buf[0x127..0x12b].copy_from_slice(&12345i32.to_le_bytes()); // exp
                                                                    // spellCastCount: cleric row [1,2,3,4,5], druid row all 9s, mage row zero.
        for (j, v) in [1u8, 2, 3, 4, 5].into_iter().enumerate() {
            buf[0x12d + j] = v;
        }
        for j in 0..5 {
            buf[0x12d + 5 + j] = 9;
        }
        buf[0x1a4] = 40; // hp current
        buf
    }

    #[test]
    fn parse_master_rejects_wrong_size() {
        let err = parse_master(&[0u8; 100]).unwrap_err();
        assert_eq!(err, SaveParseError::WrongMasterSize { got: 100 });
    }

    #[test]
    fn parse_master_reads_every_section_at_the_right_offset() {
        let bytes = synthetic_master_bytes(1);
        let m = parse_master(&bytes).unwrap();
        assert_eq!(m.game_area, 2);
        assert_eq!(m.current_3d_map_block_id(), 1);
        assert_eq!(m.last_ecl_block_id(), 7);
        assert!(m.in_dungeon());
        assert_eq!(m.clock_words(), [3, 0, 14, 0, 0]);
        assert_eq!(m.search_flags(), 1);
        assert_eq!(m.head_block_id(), 0xFF);
        assert_eq!((m.map_pos_x, m.map_pos_y, m.map_direction), (7, 13, 0));
        assert_eq!(m.last_game_state, 4);
        assert_eq!(m.game_state, 4);
        assert_eq!(
            m.set_blocks[0],
            SetBlock {
                block_id: 5,
                set_id: 1
            }
        );
        assert_eq!(m.party_count, 1);
        assert_eq!(m.char_file_names[0], "CHRDATA1");
    }

    #[test]
    fn ecl_ptr_section_is_parsed_but_never_interpreted() {
        let mut bytes = synthetic_master_bytes(1);
        // Poison section 5 with non-zero bytes -- structurally present,
        // never consulted by anything downstream of this module.
        let ecl_off = SEC_GAME_AREA + SEC_AREA_PTR + SEC_AREA2_PTR + SEC_STRU_1B2CA;
        bytes[ecl_off] = 0xAA;
        let m = parse_master(&bytes).unwrap();
        assert_eq!(m.ecl_ptr_discarded[0], 0xAA);
        assert_eq!(m.ecl_ptr_discarded.len(), SEC_ECL_PTR);
    }

    #[test]
    fn party_count_too_large_is_a_typed_error_not_a_panic() {
        let mut bytes = synthetic_master_bytes(1);
        let pc_off = SEC_GAME_AREA
            + SEC_AREA_PTR
            + SEC_AREA2_PTR
            + SEC_STRU_1B2CA
            + SEC_ECL_PTR
            + SEC_POSITION
            + SEC_LAST_GAME_STATE
            + SEC_GAME_STATE
            + SEC_SET_BLOCKS;
        bytes[pc_off] = 9;
        let err = parse_master(&bytes).unwrap_err();
        assert_eq!(err, SaveParseError::PartyCountTooLarge { party_count: 9 });
    }

    #[test]
    fn decode_char_record_rejects_wrong_size() {
        let err = decode_char_record(&[0u8; 10]).unwrap_err();
        assert_eq!(err, SaveParseError::WrongRecordSize { got: 10 });
    }

    #[test]
    fn decode_char_record_reads_authored_values() {
        let bytes = synthetic_char_bytes("Fenwick");
        let rec = decode_char_record(&bytes).unwrap();
        assert_eq!(rec.name, "Fenwick");
        assert_eq!(
            rec.stats.str,
            RawStat {
                current: 17,
                original: 18
            }
        );
        assert_eq!(
            rec.stats.str_exceptional,
            RawStat {
                current: 91,
                original: 91
            }
        );
        assert_eq!(rec.race, 7);
        assert_eq!(rec.class, 2);
        assert_eq!(rec.age, 25);
        assert_eq!(rec.hit_point_max, 42);
        assert_eq!(rec.hit_point_current, 40);
        assert_eq!(rec.exp, 12345);
        assert_eq!(rec.spell_cast_count[0], [1, 2, 3, 4, 5]);
        assert_eq!(rec.spell_cast_count[1], [9, 9, 9, 9, 9]);
        assert_eq!(rec.spell_cast_count[2], [0, 0, 0, 0, 0]);
    }

    #[test]
    fn str_exceptional_is_never_clamped_to_25() {
        let bytes = synthetic_char_bytes("Test");
        let rec = decode_char_record(&bytes).unwrap();
        // coab's StatValue.Read would have corrupted this to 25.
        assert_eq!(rec.stats.str_exceptional.current, 91);
        assert!(STR_EXCEPTIONAL_RANGE.contains(&rec.stats.str_exceptional.current));
        assert!(!MAIN_STAT_RANGE.contains(&rec.stats.str_exceptional.current));
    }

    #[test]
    fn spell_cast_count_stride_is_five_not_overlapping() {
        // With coab's i*i bug, row 1 (druid) would start at byte offset
        // 0x12d + 0 + 1*1 = 0x12e, overlapping row 0's tail. Assert our
        // decode reads druid's row starting at the clean stride-5 offset.
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0x12d + 5] = 42; // druid row, slot 0, under stride 5
        let rec = decode_char_record(&bytes).unwrap();
        assert_eq!(rec.spell_cast_count[1][0], 42);
        assert_eq!(rec.spell_cast_count[0][0], 0); // cleric row untouched
    }

    #[test]
    fn pointer_fields_are_not_represented_in_char_record() {
        // Structural check: CharRecord has no field for 0xf2/0x14d/0x151 etc.
        // — compile-time enforced by the struct shape; this test just
        // confirms decode succeeds even with those bytes poisoned, proving
        // they're never read as anything meaningful.
        let mut bytes = synthetic_char_bytes("Ptr");
        bytes[0xf2..0xf6].fill(0xFF);
        bytes[0x151..0x185].fill(0xFF);
        let rec = decode_char_record(&bytes).unwrap();
        assert_eq!(rec.name, "Ptr");
    }

    #[test]
    fn read_items_splits_fixed_size_records() {
        let bytes = vec![7u8; ITEM_RECORD_SIZE * 3];
        let items = read_items(&bytes).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].len(), ITEM_RECORD_SIZE);
    }

    #[test]
    fn read_items_rejects_truncated_files() {
        let err = read_items(&[0u8; ITEM_RECORD_SIZE + 1]).unwrap_err();
        assert_eq!(
            err,
            SaveParseError::TruncatedBlobFile {
                len: ITEM_RECORD_SIZE + 1,
                record_size: ITEM_RECORD_SIZE
            }
        );
    }

    #[test]
    fn item_readied_reads_byte_0x34() {
        let mut item = vec![0u8; ITEM_RECORD_SIZE];
        assert!(!item_readied(&item));
        item[0x34] = 1;
        assert!(item_readied(&item));
    }

    #[test]
    fn item_readied_on_short_record_is_false_not_a_panic() {
        assert!(!item_readied(&[0u8; 2]));
    }

    #[test]
    fn read_affects_splits_fixed_size_records() {
        let bytes = vec![3u8; AFFECT_RECORD_SIZE * 2];
        let affects = read_affects(&bytes).unwrap();
        assert_eq!(affects.len(), 2);
    }

    #[test]
    fn load_from_lookup_assembles_the_full_set() {
        let master_bytes = synthetic_master_bytes(1);
        let char_bytes = synthetic_char_bytes("Solo");
        let items_bytes = vec![1u8; ITEM_RECORD_SIZE * 2];
        let lookup = |name: &str| -> Option<&[u8]> {
            match name {
                "CHRDATA1.SAV" => Some(char_bytes.as_slice()),
                "CHRDATA1.SWG" => Some(items_bytes.as_slice()),
                _ => None,
            }
        };
        let set = load_from_lookup(&master_bytes, 'A', lookup).unwrap();
        assert_eq!(set.chars.len(), 1);
        assert_eq!(set.chars[0].record.name, "Solo");
        assert_eq!(set.chars[0].items.len(), 2);
        assert!(set.chars[0].affects.is_empty());
    }

    #[test]
    fn load_from_lookup_errors_on_missing_required_sav() {
        let master_bytes = synthetic_master_bytes(1);
        let err = load_from_lookup(&master_bytes, 'A', |_| None).unwrap_err();
        assert_eq!(err, ImportSetError::MissingFile("CHRDATA1.SAV".to_string()));
    }
}
