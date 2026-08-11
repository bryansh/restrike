//! ★ Vancian camp magic — the record model (roll-credits §8, D-S4a/D-S4b).
//!
//! Derived by reading coab for behavior (D11, never copied); every citation
//! below was re-verified against the reference tree this slice.
//!
//! ## The record model, and why it needs no save break
//!
//! coab models the memorized/staged list as a C# object (`Classes/SpellList.cs`)
//! with `Load`/`Save` methods over the character record's 84 bytes at `0x1E`.
//! The **original has no such object**: the 84-byte array *is* the state
//! (`charStruct.spell_list db 84 dup(?)` @`0x1E`, coab's own IDA listing), and
//! the "learning" flag is the high bit of the stored byte — which is exactly
//! what `AddLearnt` decodes (`SpellList.cs:81-84`: `id & 0x7F`, `Learning =
//! id > 0x7f`).
//!
//! So this module is a **view over [`crate::party::MagicState::spell_list`]'s
//! raw bytes**, not a parallel decoded model. `Character`'s serde shape is
//! untouched, staging round-trips through `.rsav` for free (it is record
//! state, D-S4e), and the slice spends no `SAVE_FORMAT_VERSION` bump.
//!
//! ### Layout, proven three ways
//!
//! - **Adds fill from the back.** `SpellList.Save` (`:129-137`) writes from
//!   `idx = 83` downward, and doc §33's save-diff caught the first memorized
//!   Magic Missile landing at record offset `0x71` — i.e. array index 83.
//! - **Slot 0 (`@0x1E`) is never used.** The binary's own combat collector
//!   reads `record[0x1E + i]` for `i = 1..=0x53` (`ovr010:062A-065D`, doc
//!   §41.1, transcribed in `crate::combat::records`) — index 0 is not in its
//!   range. [`add_learn`]/[`add_learnt`] therefore fill 83 down to **1**.
//! - **Reads run in ascending index order.** Same collector. coab's `Load`
//!   (`:111-120`) also appends ascending, so this ordering is what coab itself
//!   has after any save/load cycle.
//!
//! **The consequence, stated rather than tidied away:** because adds descend
//! and reads ascend, the *most recently staged* spell is the one at the lowest
//! index, so it is the first one `rest_memorize` commits (`crate::rest`).
//! coab's in-memory list keeps add order until its next `Load`, at which point
//! it agrees with us. The bytes are the authority.
//!
//! **One coab≠binary note carried forward:** coab's `SpellList.Save` writes
//! *only* non-learning entries (`:132`), silently dropping staged spells at
//! every save. The original cannot do that — the array is the live state and
//! `AddLearnt`'s high-bit decode exists precisely to read staged entries back.
//! We keep the staged bytes.

use crate::party::{Character, MagicState};

/// `SpellList.SpellListSize` (`Classes/SpellList.cs:21`) — the 84-byte field at
/// record offset `0x1E`.
pub const SPELL_LIST_SIZE: usize = 84;

/// The lowest array index [`add_learn`] will fill. Index 0 is skipped: the
/// binary's collector never reads it (see the module doc).
const FIRST_USABLE_SLOT: usize = 1;

/// `SpellClass` (`Classes/Spells.cs:37-44`). `Unknown10` is coab's name for the
/// discriminant the table's terminator row carries; it is not a caster class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpellClass {
    Cleric = 0,
    Druid = 1,
    MagicUser = 2,
    Monster = 3,
    Unknown10 = 10,
}

impl SpellClass {
    /// The `spellCastCount[class, level-1]` row index — only the three caster
    /// classes have one (`Player.cs:536`, `byte[3,5]` @`0x12D`).
    pub fn cast_count_row(self) -> Option<usize> {
        match self {
            SpellClass::Cleric => Some(0),
            SpellClass::Druid => Some(1),
            SpellClass::MagicUser => Some(2),
            SpellClass::Monster | SpellClass::Unknown10 => None,
        }
    }

    /// `BuildMemorizeSpellText`'s per-class heading (`ovr016.cs:243-256`),
    /// including its exact right-aligning leading spaces.
    pub fn memorize_heading(self) -> &'static str {
        match self {
            SpellClass::Cleric => "    Cleric Spells:",
            SpellClass::Druid => "     Druid Spells:",
            SpellClass::MagicUser => "Magic-User Spells:",
            _ => "",
        }
    }
}

/// One `gbl.spellCastingTable` row's camp-facing cells (`Classes/Gbl.cs:567+`,
/// `seg600:37DC`) plus its display name (`ovr023.SpellNames`, `:10-111`).
///
/// `crate::combat::spells::SpellEntry` carries the *casting* cells for the
/// handful of rows a fight has ever needed (its lazy-transcription rule, doc
/// §41.2). Camp needs `spellClass`/`spellLevel` for **every** id — capacity,
/// the grimoire list, and the rest-time arithmetic all walk the whole table —
/// so those two columns (and the name) are transcribed in full here. The two
/// tables are disjoint by column, not duplicates; the test
/// `the_camp_table_agrees_with_the_combat_rows` pins them together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellRow {
    pub class_: SpellClass,
    /// `spellLevel@+1`. `0` only on the null row and the terminator.
    pub level: u8,
    /// `ovr023.SpellNames[id]`. Empty for the ids the shipped name table
    /// leaves blank (`0x39`, `0x3B`..`0x41`, `0x5F`..`0x63`) — real rows in the
    /// casting table with no presentable name.
    pub name: &'static str,
}

pub const SPELL_TABLE: [SpellRow; 102] = [
    SpellRow {
        class_: SpellClass::Unknown10,
        level: 0,
        name: "",
    }, // 0 — the null row
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Bless",
    }, // 0x01
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Curse",
    }, // 0x02
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Cure Light Wounds",
    }, // 0x03
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Cause Light Wounds",
    }, // 0x04
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Detect Magic",
    }, // 0x05
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Protection from Evil",
    }, // 0x06
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Protection from Good",
    }, // 0x07
    SpellRow {
        class_: SpellClass::Cleric,
        level: 1,
        name: "Resist Cold",
    }, // 0x08
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Burning Hands",
    }, // 0x09
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Charm Person",
    }, // 0x0A
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Detect Magic",
    }, // 0x0B
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Enlarge",
    }, // 0x0C
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Reduce",
    }, // 0x0D
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Friends",
    }, // 0x0E
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Magic Missile",
    }, // 0x0F
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Protection From Evil",
    }, // 0x10
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Protection From Good",
    }, // 0x11
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Read Magic",
    }, // 0x12
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Shield",
    }, // 0x13
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Shocking Grasp",
    }, // 0x14
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 1,
        name: "Sleep",
    }, // 0x15
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Find Traps",
    }, // 0x16
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Hold Person",
    }, // 0x17
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Resist Fire",
    }, // 0x18
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Silence, 15' Radius",
    }, // 0x19
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Slow Poison",
    }, // 0x1A
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Snake Charm",
    }, // 0x1B
    SpellRow {
        class_: SpellClass::Cleric,
        level: 2,
        name: "Spiritual Hammer",
    }, // 0x1C
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Detect Invisibility",
    }, // 0x1D
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Invisibility",
    }, // 0x1E
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Knock",
    }, // 0x1F
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Mirror Image",
    }, // 0x20
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Ray of Enfeeblement",
    }, // 0x21
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Stinking Cloud",
    }, // 0x22
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 2,
        name: "Strength",
    }, // 0x23
    SpellRow {
        class_: SpellClass::Monster,
        level: 7,
        name: "Animate Dead",
    }, // 0x24
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Cure Blindness",
    }, // 0x25
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Cause Blindness",
    }, // 0x26
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Cure Disease",
    }, // 0x27
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Cause Disease",
    }, // 0x28
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Dispel Magic",
    }, // 0x29
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Prayer",
    }, // 0x2A
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Remove Curse",
    }, // 0x2B
    SpellRow {
        class_: SpellClass::Cleric,
        level: 3,
        name: "Bestow Curse",
    }, // 0x2C
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Blink",
    }, // 0x2D
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Dispel Magic",
    }, // 0x2E
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Fireball",
    }, // 0x2F
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Haste",
    }, // 0x30
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Hold Person",
    }, // 0x31
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Invisibility, 10' Radius",
    }, // 0x32
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Lightning Bolt",
    }, // 0x33
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Protection From Evil, 10' Radius",
    }, // 0x34
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Protection From Good, 10' Radius",
    }, // 0x35
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Protection From Normal Missiles",
    }, // 0x36
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 3,
        name: "Slow",
    }, // 0x37
    SpellRow {
        class_: SpellClass::Cleric,
        level: 7,
        name: "Restoration",
    }, // 0x38
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x39
    SpellRow {
        class_: SpellClass::Cleric,
        level: 4,
        name: "Cure Serious Wounds",
    }, // 0x3A
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x3B
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x3C
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x3D
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x3E
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x3F
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x40
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x41
    SpellRow {
        class_: SpellClass::Cleric,
        level: 4,
        name: "Cause Serious Wounds",
    }, // 0x42
    SpellRow {
        class_: SpellClass::Cleric,
        level: 4,
        name: "Neutralize Poison",
    }, // 0x43
    SpellRow {
        class_: SpellClass::Cleric,
        level: 4,
        name: "Poison",
    }, // 0x44
    SpellRow {
        class_: SpellClass::Cleric,
        level: 4,
        name: "Protection Evil, 10' Radius",
    }, // 0x45
    SpellRow {
        class_: SpellClass::Cleric,
        level: 4,
        name: "Sticks to Snakes",
    }, // 0x46
    SpellRow {
        class_: SpellClass::Cleric,
        level: 5,
        name: "Cure Critical Wounds",
    }, // 0x47
    SpellRow {
        class_: SpellClass::Cleric,
        level: 5,
        name: "Cause Critical Wounds",
    }, // 0x48
    SpellRow {
        class_: SpellClass::Cleric,
        level: 5,
        name: "Dispel Evil",
    }, // 0x49
    SpellRow {
        class_: SpellClass::Cleric,
        level: 5,
        name: "Flame Strike",
    }, // 0x4A
    SpellRow {
        class_: SpellClass::Cleric,
        level: 5,
        name: "Raise Dead",
    }, // 0x4B
    SpellRow {
        class_: SpellClass::Cleric,
        level: 5,
        name: "Slay Living",
    }, // 0x4C
    SpellRow {
        class_: SpellClass::Druid,
        level: 1,
        name: "Detect Magic",
    }, // 0x4D
    SpellRow {
        class_: SpellClass::Druid,
        level: 1,
        name: "Entangle",
    }, // 0x4E
    SpellRow {
        class_: SpellClass::Druid,
        level: 1,
        name: "Faerie Fire",
    }, // 0x4F
    SpellRow {
        class_: SpellClass::Druid,
        level: 1,
        name: "Invisibility to Animals",
    }, // 0x50
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Charm Monsters",
    }, // 0x51
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Confusion",
    }, // 0x52
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Dimension Door",
    }, // 0x53
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Fear",
    }, // 0x54
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Fire Shield",
    }, // 0x55
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Fumble",
    }, // 0x56
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Ice Storm",
    }, // 0x57
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Minor Globe Of Invulnerability",
    }, // 0x58
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Remove Curse",
    }, // 0x59
    SpellRow {
        class_: SpellClass::Monster,
        level: 5,
        name: "Animate Dead",
    }, // 0x5A
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 5,
        name: "Cloud Kill",
    }, // 0x5B
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 5,
        name: "Cone of Cold",
    }, // 0x5C
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 5,
        name: "Feeblemind",
    }, // 0x5D
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 5,
        name: "Hold Monsters",
    }, // 0x5E
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x5F
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x60
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x61
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x62
    SpellRow {
        class_: SpellClass::Monster,
        level: 6,
        name: "",
    }, // 0x63
    SpellRow {
        class_: SpellClass::MagicUser,
        level: 4,
        name: "Bestow Curse",
    }, // 0x64
    SpellRow {
        class_: SpellClass::Unknown10,
        level: 0,
        name: "",
    }, // 0x65
];

/// `gbl.spellCastingTable[id]`'s camp cells, or `None` for an id outside the
/// shipped table (the array is `1..=0x65`; index 0 is coab's `null` row).
pub fn spell_row(id: u8) -> Option<&'static SpellRow> {
    if id == 0 {
        return None;
    }
    SPELL_TABLE.get(id as usize)
}

/// `gbl.spellCastingTable[id].spellLevel`, `0` for an unknown id.
pub fn spell_level(id: u8) -> u8 {
    spell_row(id).map_or(0, |r| r.level)
}

/// `gbl.spellCastingTable[id].spellClass`, [`SpellClass::Unknown10`] for an
/// unknown id.
pub fn spell_class(id: u8) -> SpellClass {
    spell_row(id).map_or(SpellClass::Unknown10, |r| r.class_)
}

/// `ovr023.SpellNames[id]` — `""` for an unnamed or unknown id.
pub fn spell_name(id: u8) -> &'static str {
    spell_row(id).map_or("", |r| r.name)
}

/// `LevelStrings` (`ovr023.cs:113-124`) — the heading `add_spell_to_list`
/// inserts above each spell-level run.
pub fn level_string(level: u8) -> &'static str {
    const LEVELS: [&str; 10] = [
        "",
        "1st Level",
        "2nd Level",
        "3rd Level",
        "4th Level",
        "5th Level",
        "6th Level",
        "7th Level",
        "8th Level",
        "9th Level",
    ];
    LEVELS.get(level as usize).copied().unwrap_or("")
}

// ---------------------------------------------------------------------------
// The 84-byte list: the `SpellList` methods, as operations on the record bytes
// ---------------------------------------------------------------------------

/// One decoded entry — `SpellItem` (`Classes/SpellList.cs:7-14`) without the
/// object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagedSpell {
    /// The array index the byte lives at, so a caller can commit in place.
    pub slot: usize,
    /// `id & 0x7F` (`AddLearnt`, `SpellList.cs:83`).
    pub id: u8,
    /// `id > 0x7f` (same line) — staged for this rest, not yet memorized.
    pub learning: bool,
}

/// Every non-zero entry, ascending slot (`SpellList.IdList`, `:46-52`).
pub fn entries(list: &[u8]) -> impl Iterator<Item = StagedSpell> + '_ {
    list.iter()
        .enumerate()
        .filter(|&(_, &b)| b != 0)
        .map(|(slot, &b)| StagedSpell {
            slot,
            id: b & 0x7F,
            learning: b > 0x7F,
        })
}

/// `SpellList.LearntList` (`:54-63`) — the spells that are actually memorized
/// and castable right now.
pub fn learnt(list: &[u8]) -> impl Iterator<Item = StagedSpell> + '_ {
    entries(list).filter(|e| !e.learning)
}

/// `SpellList.LearningList` (`:65-74`) — the spells staged by Memorize and
/// waiting for Rest to commit them.
pub fn learning(list: &[u8]) -> impl Iterator<Item = StagedSpell> + '_ {
    entries(list).filter(|e| e.learning)
}

/// `SpellList.HasSpells` (`:96-99`).
pub fn has_spells(list: &[u8]) -> bool {
    entries(list).next().is_some()
}

/// `SpellList.HasSpell` (`:101-104`) — matches on the masked id, so a staged
/// entry counts.
pub fn has_spell(list: &[u8], id: u8) -> bool {
    entries(list).any(|e| e.id == id)
}

/// Grows a short/absent list to the record's own 84 bytes. Import always
/// supplies 84 (`save_orig::decode_char_record`); synthetic fixtures need not.
fn ensure_len(list: &mut Vec<u8>) {
    if list.len() < SPELL_LIST_SIZE {
        list.resize(SPELL_LIST_SIZE, 0);
    }
}

/// The highest free slot at or above [`FIRST_USABLE_SLOT`] — where the next
/// add goes (see the module doc's "adds fill from the back").
fn next_free_slot(list: &[u8]) -> Option<usize> {
    (FIRST_USABLE_SLOT..list.len().min(SPELL_LIST_SIZE))
        .rev()
        .find(|&i| list[i] == 0)
}

/// `SpellList.AddLearn` (`:76-79`) — stage `id` for memorization. Stores
/// `id | 0x80`. `false` if the list is full (84 entries; unreachable with real
/// slot counts, and a silent no-op in the original's own array walk).
pub fn add_learn(list: &mut Vec<u8>, id: u8) -> bool {
    add_byte(list, id | 0x80)
}

/// `SpellList.AddLearnt` (`:81-84`) called with a plain id — a spell that is
/// memorized immediately (the Learn/training path, and every test fixture).
pub fn add_learnt(list: &mut Vec<u8>, id: u8) -> bool {
    add_byte(list, id & 0x7F)
}

fn add_byte(list: &mut Vec<u8>, byte: u8) -> bool {
    if byte & 0x7F == 0 {
        return false; // id 0 is the array's own "empty" marker
    }
    ensure_len(list);
    match next_free_slot(list) {
        Some(slot) => {
            list[slot] = byte;
            true
        }
        None => false,
    }
}

/// `SpellList.MarkLearnt` (`:86-94`) — the Rest commit: the FIRST still-staged
/// entry with this id loses its high bit and becomes castable. `false` if no
/// staged entry matched.
pub fn mark_learnt(list: &mut [u8], id: u8) -> bool {
    let slot = list.iter().position(|&b| b > 0x7F && (b & 0x7F) == id);
    match slot {
        Some(slot) => {
            list[slot] &= 0x7F;
            true
        }
        None => false,
    }
}

/// `SpellList.CancelLearning` (`:106-109`) — drops every staged entry,
/// leaving memorized ones alone. This is half of `cancel_memorize`
/// (`ovr016.cs:67-72`).
pub fn cancel_learning(list: &mut [u8]) {
    for b in list.iter_mut() {
        if *b > 0x7F {
            *b = 0;
        }
    }
}

/// `SpellList.ClearSpell` (`:30-44`) — removes the first entry with this id
/// (staged or memorized). This is what casting will consume a slot with.
pub fn clear_spell(list: &mut [u8], id: u8) -> bool {
    match list.iter().position(|&b| b != 0 && (b & 0x7F) == id) {
        Some(slot) => {
            list[slot] = 0;
            true
        }
        None => false,
    }
}

/// `cancel_memorize` (`ovr016.cs:67-72`): drop the staged spells **and** zero
/// `spell_to_learn_count`. Both halves, always — the count is the rest loop's
/// study timer and a stale one would start learning at the wrong hour.
pub fn cancel_memorize(magic: &mut MagicState) {
    cancel_learning(&mut magic.spell_list);
    magic.spell_to_learn_count = 0;
}

// ---------------------------------------------------------------------------
// Capacity — `HowManySpellsPlayerCanLearn` (D-S4b)
// ---------------------------------------------------------------------------

/// `spellCastCount[class, level-1]` read the way the original reads it: one
/// **flat** byte at record offset `0x12D + class*5 + (level-1)`
/// (`Player.cs:536`; the stride is 5, pinned by
/// `save_orig::spell_cast_count_stride_is_five_not_overlapping` — coab's own
/// `Load` writes `i*i` there, a transcription bug this codebase already
/// corrected).
///
/// The flat read is deliberate. `HowManySpellsPlayerCanLearn` is called with
/// levels straight out of the casting table, which contains rows at level 6
/// and 7 (`0x24` Monster 7, `0x38` Cleric 7, the `0x39`+ Monster 6 run). C#
/// would throw; the original just reads the next byte along. Keeping the flat
/// index reproduces that exactly for the in-range cases and stays defined
/// (`0`) past the end of the 15-byte block rather than inventing a panic.
/// No shipped `spellBook` can hold a level-6/7 id, so this is provenance, not
/// behaviour.
pub fn cast_count_at(magic: &MagicState, class_: SpellClass, level: u8) -> u8 {
    let Some(row) = class_.cast_count_row() else {
        return 0;
    };
    if level == 0 {
        return 0;
    }
    let flat = row * 5 + (level as usize - 1);
    let (r, c) = (flat / 5, flat % 5);
    magic
        .cast_count
        .get(r)
        .and_then(|r| r.get(c))
        .copied()
        .unwrap_or(0)
}

/// ★ `HowManySpellsPlayerCanLearn(spellClass, spellLevel)` — `sub_4428E`,
/// `ovr016.cs:99-113`, transcribed:
///
/// ```text
/// alreadyLearning = count of gbl.SelectedPlayer.spellList.IdList() whose
///                   spellCastingTable[id] matches BOTH spellLevel and spellClass
/// return spellCastCount[spellClass, spellLevel - 1] - alreadyLearning
/// ```
///
/// Two details the name hides:
/// - The subtrahend walks **`IdList`**, not `LearningList` — memorized spells
///   occupy their slot just as staged ones do, which is why a caster who wakes
///   with spells still in memory cannot re-fill those slots until they are
///   cast (`ovr016.cs:103`).
/// - The result is **signed**. An over-full level (possible after a level
///   drain, or a save edited outside the game) returns a negative number, and
///   every caller tests `> 0`, so it simply reads as "no room" rather than
///   wrapping. Returned as `i32` for that reason.
pub fn how_many_spells_player_can_learn(magic: &MagicState, class_: SpellClass, level: u8) -> i32 {
    let already = entries(&magic.spell_list)
        .filter(|e| {
            let row = spell_row(e.id);
            row.is_some_and(|r| r.level == level && r.class_ == class_)
        })
        .count() as i32;
    i32::from(cast_count_at(magic, class_, level)) - already
}

/// `can_learn_spell` (`sub_5C01E`, `ovr023.cs:126-167`) — may this character
/// hold this spell at all? The grimoire/memory list builders filter on it
/// (`BuildSpellList`, `ovr023.cs:395-472`).
///
/// `in_combat_screen` is coab's `gbl.game_state != GameState.Combat` disjunct and
/// `armor_readied` its `activeItems.armor != null` one; every camp caller passes
/// `(false, false)` — see [`can_learn_spell_in_camp`]. (Combat has the readied-
/// armor bit already: `crate::combat::records`' `field_159_null`.)
///
/// ★ **A transcription note, not a correction.** The Magic-User arm is a
/// five-way **disjunction** in coab (`:150-155`), and one of its disjuncts is
/// "we are not in combat" — so in camp the whole arm collapses to `Int > 8`.
/// That is almost certainly not what the original's `and`/`or` chain means,
/// but it is unobservable here: every consumer of this predicate also requires
/// `KnowsSpell` (the grimoire list) or a non-zero `spellCastCount` (capacity),
/// and a non-caster has neither. Transcribed as coab has it, flagged as the
/// place to re-derive from the listing if a future flow ever depends on the
/// arm alone.
pub fn can_learn_spell(
    ch: &Character,
    id: u8,
    in_combat_screen: bool,
    armor_readied: bool,
) -> bool {
    use crate::party::{SKILL_CLERIC, SKILL_MAGIC_USER, SKILL_PALADIN, SKILL_RANGER};
    const HUMAN: u8 = 7; // `Race.human` (`Classes/Enums.cs:54`)
                         // `stats2.Wis.full` / `.Int.full` — the *second* stored byte of each pair
                         // (`StatValue.Read`, `Player.cs:83-88`: `cur` @+0, `full` @+1), which our
                         // model names `original` (`party::AbilityScorePair`).
    let wis = ch.stats.wis.original;
    let int = ch.stats.int.original;
    match spell_class(id & 0x7F) {
        SpellClass::Cleric => {
            wis > 8 && (ch.skill_level(SKILL_CLERIC) > 0 || ch.skill_level(SKILL_PALADIN) > 8)
        }
        SpellClass::Druid => wis > 8 && ch.skill_level(SKILL_RANGER) > 6,
        SpellClass::MagicUser => {
            int > 8
                && (ch.race != HUMAN
                    || !armor_readied
                    || !in_combat_screen
                    || ch.skill_level(SKILL_RANGER) > 8
                    || ch.skill_level(SKILL_MAGIC_USER) > 0)
        }
        SpellClass::Monster | SpellClass::Unknown10 => false,
    }
}

/// The camp callers' argument pair for [`can_learn_spell`]: not in combat, so
/// the Magic-User arm's `game_state != Combat` disjunct is already satisfied
/// and the readied-armor question never arises (`ovr023.cs:152-153`).
pub fn can_learn_spell_in_camp(ch: &Character, id: u8) -> bool {
    can_learn_spell(ch, id, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::{AbilityScorePair, Character};

    fn empty_list() -> Vec<u8> {
        vec![0u8; SPELL_LIST_SIZE]
    }

    fn caster() -> Character {
        let rec = gbx_formats::save_orig::decode_char_record(&vec![
            0u8;
            gbx_formats::save_orig::CHAR_RECORD_SIZE
        ])
        .unwrap();
        let mut ch = crate::party::character_from_record(&rec, vec![], vec![]);
        ch.stats.wis = AbilityScorePair {
            current: 16,
            original: 16,
        };
        ch.stats.int = AbilityScorePair {
            current: 16,
            original: 16,
        };
        ch.race = 7; // human
        ch.magic.spell_list = empty_list();
        ch
    }

    /// D-S4a's whole claim in one test: the on-wire byte is `id | 0x80` while
    /// learning, and `AddLearnt`'s decode (`SpellList.cs:83`) reads it back.
    #[test]
    fn the_learning_flag_is_the_high_bit_of_the_stored_byte() {
        let mut list = empty_list();
        assert!(add_learn(&mut list, 0x0F));
        assert_eq!(list[83], 0x8F, "Magic Missile staged == 0x0F | 0x80");
        let e: Vec<_> = entries(&list).collect();
        assert_eq!(
            e,
            vec![StagedSpell {
                slot: 83,
                id: 0x0F,
                learning: true
            }]
        );
        assert_eq!(learning(&list).count(), 1);
        assert_eq!(learnt(&list).count(), 0);
    }

    /// Adds descend from index 83 and stop at index 1 — the binary's combat
    /// collector reads `record[0x1E + 1 ..= 0x1E + 0x53]` and would never see
    /// a byte written at slot 0.
    #[test]
    fn adds_fill_from_the_back_and_never_touch_slot_zero() {
        let mut list = empty_list();
        for i in 0..83 {
            assert!(add_learnt(&mut list, 1 + (i % 8) as u8), "add {i}");
        }
        assert_eq!(list[0], 0, "slot 0 stays empty");
        assert!(list[1..].iter().all(|&b| b != 0), "slots 1..=83 are full");
        assert!(!add_learnt(&mut list, 3), "the 84th add has nowhere to go");
    }

    /// `MarkLearnt` (`SpellList.cs:86-94`) clears the flag **in place**: the
    /// byte keeps its slot, so nothing else in the record moves.
    #[test]
    fn mark_learnt_clears_the_high_bit_in_place() {
        let mut list = empty_list();
        add_learn(&mut list, 0x03);
        add_learn(&mut list, 0x03);
        assert_eq!(&list[82..], &[0x83, 0x83]);
        assert!(mark_learnt(&mut list, 0x03));
        assert_eq!(
            &list[82..],
            &[0x03, 0x83],
            "the FIRST staged entry commits, ascending"
        );
        assert!(mark_learnt(&mut list, 0x03));
        assert_eq!(&list[82..], &[0x03, 0x03]);
        assert!(!mark_learnt(&mut list, 0x03), "nothing staged is left");
    }

    /// `CancelLearning` (`:106-109`) is exactly "drop the high-bit bytes" —
    /// memorized spells survive camp-exit, staged ones do not.
    #[test]
    fn cancel_learning_drops_staged_and_keeps_memorized() {
        let mut list = empty_list();
        add_learnt(&mut list, 0x0F); // already in memory
        add_learn(&mut list, 0x03); // staged this camp
        assert_eq!(entries(&list).count(), 2);
        cancel_learning(&mut list);
        let left: Vec<_> = entries(&list).map(|e| e.id).collect();
        assert_eq!(left, vec![0x0F]);
    }

    /// `cancel_memorize` (`ovr016.cs:67-72`) zeroes the study timer too.
    #[test]
    fn cancel_memorize_also_zeroes_the_study_timer() {
        let mut ch = caster();
        add_learn(&mut ch.magic.spell_list, 0x0F);
        ch.magic.spell_to_learn_count = 6;
        cancel_memorize(&mut ch.magic);
        assert_eq!(learning(&ch.magic.spell_list).count(), 0);
        assert_eq!(ch.magic.spell_to_learn_count, 0);
    }

    /// ★ D-S4b. `HowManySpellsPlayerCanLearn` subtracts **`IdList`** — every
    /// entry at that (class, level), memorized *and* staged alike
    /// (`ovr016.cs:103`).
    #[test]
    fn capacity_subtracts_memorized_and_staged_alike() {
        let mut ch = caster();
        ch.magic.cast_count[2][0] = 4; // Magic-User level 1: four slots
        assert_eq!(
            how_many_spells_player_can_learn(&ch.magic, SpellClass::MagicUser, 1),
            4
        );
        add_learnt(&mut ch.magic.spell_list, 0x0F); // memorized Magic Missile
        assert_eq!(
            how_many_spells_player_can_learn(&ch.magic, SpellClass::MagicUser, 1),
            3,
            "a spell still in memory holds its slot"
        );
        add_learn(&mut ch.magic.spell_list, 0x15); // staged Sleep
        assert_eq!(
            how_many_spells_player_can_learn(&ch.magic, SpellClass::MagicUser, 1),
            2
        );
        // A different level is a different pool.
        add_learn(&mut ch.magic.spell_list, 0x1E); // Invisibility, MU 2
        assert_eq!(
            how_many_spells_player_can_learn(&ch.magic, SpellClass::MagicUser, 1),
            2
        );
        assert_eq!(
            how_many_spells_player_can_learn(&ch.magic, SpellClass::MagicUser, 2),
            -1,
            "over-full reads negative, never wraps — every caller tests `> 0`"
        );
    }

    /// The flat `0x12D + class*5 + (level-1)` read, and the aliasing it
    /// produces past a class's own five levels (the original's own behaviour
    /// for the casting table's level-6/7 rows).
    #[test]
    fn cast_count_reads_a_flat_stride_five_block() {
        let mut ch = caster();
        ch.magic.cast_count = [[1, 2, 3, 4, 5], [6, 7, 8, 9, 10], [11, 12, 13, 14, 15]];
        assert_eq!(cast_count_at(&ch.magic, SpellClass::Cleric, 1), 1);
        assert_eq!(cast_count_at(&ch.magic, SpellClass::Cleric, 5), 5);
        assert_eq!(cast_count_at(&ch.magic, SpellClass::Druid, 1), 6);
        assert_eq!(cast_count_at(&ch.magic, SpellClass::MagicUser, 5), 15);
        // Cleric level 7 (`0x38` Restoration) → flat index 6 → the druid row's
        // second cell. Unreachable in shipped content; defined, not a panic.
        assert_eq!(cast_count_at(&ch.magic, SpellClass::Cleric, 7), 7);
        // Past the 15-byte block: `0`, where the original would read
        // `field_13C`. Also unreachable.
        assert_eq!(cast_count_at(&ch.magic, SpellClass::MagicUser, 7), 0);
        assert_eq!(cast_count_at(&ch.magic, SpellClass::Monster, 1), 0);
    }

    /// The table's own shape: one row per shipped id, and the three rows the
    /// combat side already transcribed independently (`combat::spells`'
    /// `MAGIC_MISSILE`/`CURE_LIGHT_WOUNDS`/`HOLD_PERSON`, from the same
    /// `Gbl.cs:572,583,592` lines) agree on class and level.
    #[test]
    fn the_table_covers_the_shipped_ids_and_agrees_with_the_combat_rows() {
        assert_eq!(SPELL_TABLE.len(), 0x66, "ids 1..=0x65 plus the null row");
        assert!(spell_row(0).is_none(), "index 0 is coab's `null` row");
        assert_eq!(spell_row(0x66), None, "past the shipped table");

        assert_eq!(spell_class(0x0F), SpellClass::MagicUser);
        assert_eq!(spell_level(0x0F), 1);
        assert_eq!(spell_name(0x0F), "Magic Missile");

        assert_eq!(spell_class(0x03), SpellClass::Cleric);
        assert_eq!(spell_level(0x03), 1);
        assert_eq!(spell_name(0x03), "Cure Light Wounds");

        assert_eq!(spell_class(0x17), SpellClass::Cleric);
        assert_eq!(spell_level(0x17), 2);
        assert_eq!(spell_name(0x17), "Hold Person");

        // The three cure spells `FixTeam` counts (`ovr016.cs:886-896`) sit at
        // cleric levels 1/4/5 — which is why `CalculateTimeAndSpellNumbers`
        // reads `spellCastCount[0,0]`, `[0,3]` and `[0,4]`.
        assert_eq!(
            (spell_class(0x03), spell_level(0x03)),
            (SpellClass::Cleric, 1)
        );
        assert_eq!(
            (spell_class(0x3A), spell_level(0x3A)),
            (SpellClass::Cleric, 4)
        );
        assert_eq!(
            (spell_class(0x47), spell_level(0x47)),
            (SpellClass::Cleric, 5)
        );

        assert_eq!(level_string(1), "1st Level");
        assert_eq!(level_string(5), "5th Level");
    }

    /// `can_learn_spell`'s three caster arms (`ovr023.cs:131-164`), including
    /// the stat floor of 9 (`> 8`) and the flat "no" for monster spells.
    #[test]
    fn can_learn_spell_gates_on_class_and_the_stat_floor() {
        let mut ch = caster();
        ch.class_level[crate::party::SKILL_CLERIC] = 3;
        assert!(can_learn_spell_in_camp(&ch, 0x03), "cleric, Wis 16");
        ch.stats.wis.original = 8;
        assert!(!can_learn_spell_in_camp(&ch, 0x03), "Wis 8 is one short");
        ch.stats.wis.original = 9;
        assert!(can_learn_spell_in_camp(&ch, 0x03));

        // Magic-User: Int floor, and the id is masked first (`:128`), so a
        // staged byte answers the same as its plain id.
        let mut mu = caster();
        mu.class_level[crate::party::SKILL_MAGIC_USER] = 5;
        assert!(can_learn_spell_in_camp(&mu, 0x0F));
        assert!(can_learn_spell_in_camp(&mu, 0x8F), "0x8F masks to 0x0F");
        mu.stats.int.original = 8;
        assert!(!can_learn_spell_in_camp(&mu, 0x0F));

        // Druid needs Ranger > 6; Monster spells are never learnable.
        let mut rgr = caster();
        assert!(!can_learn_spell_in_camp(&rgr, 0x4D));
        rgr.class_level[crate::party::SKILL_RANGER] = 7;
        assert!(can_learn_spell_in_camp(&rgr, 0x4D));
        assert!(!can_learn_spell_in_camp(&rgr, 0x39), "0x39 is Monster 6");
    }

    /// The bytes a staged spell leaves behind are exactly what the binary's
    /// combat collector picks up (`crate::combat::records`, doc §41.1: every
    /// non-zero byte in `0x1F..=0x71`, high-bit entries included).
    #[test]
    fn staged_bytes_are_visible_to_the_combat_collectors_rule() {
        let mut list = empty_list();
        add_learnt(&mut list, 0x0F);
        add_learn(&mut list, 0x03);
        let collected: Vec<u8> = list[1..].iter().copied().filter(|&b| b != 0).collect();
        assert_eq!(collected, vec![0x83, 0x0F], "ascending, staged byte intact");
    }
}
