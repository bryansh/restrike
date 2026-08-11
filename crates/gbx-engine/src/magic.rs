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

// ---------------------------------------------------------------------------
// Scrolls — the Scribe path's half of the same `id | 0x80` staging encoding
// ---------------------------------------------------------------------------

/// `Item.IsScroll()` (`Classes/Item.cs:50-53`): the item's `ITEMS`-table entry
/// has `item_slot` in `11..=13` (`ItemSlot.slot_11`..`slot_13`, with `Quarrel`
/// = 12 sitting between them — the enum names are coab's guesses, the range is
/// the test).
///
/// The table is game data, so a fixture set without an `ITEMS` file simply has
/// no scrolls — the same D10 posture `combat_host::load_item_data` already
/// takes, and the honest answer rather than a guess from the record alone.
#[derive(Debug, Clone, Default)]
pub struct ScrollLookup {
    table: Option<gbx_formats::items::ItemDataTable>,
}

impl ScrollLookup {
    /// Parses the resident `ITEMS` file out of the data set, if it is there.
    pub fn load(data: &gbx_formats::game_data::GameData) -> Self {
        ScrollLookup {
            table: data
                .raw_file(crate::combat_host::ITEMS_FILE)
                .and_then(|b| gbx_formats::items::ItemDataTable::parse(b).ok()),
        }
    }

    /// True when the table is absent — every `is_scroll` answer is then `false`
    /// and the Scribe flows report having nothing to copy, which is what they
    /// would say for a party carrying no scrolls.
    pub fn is_empty(&self) -> bool {
        self.table.is_none()
    }

    /// `Item.IsScroll()` over one raw `.swg` record.
    pub fn is_scroll(&self, record: &[u8]) -> bool {
        let Some(t) = &self.table else { return false };
        let slot = t.get(gbx_formats::save_orig::item_type(record)).item_slot;
        (11..=13).contains(&slot)
    }
}

/// One spell written on a scroll: which of the three affect bytes holds it,
/// its masked id, and whether it is currently staged for scribing (the high
/// bit — `Item.ScrollLearning`, `Classes/Item.cs:45-48`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollSpell {
    /// 1-based, the way `Item.getAffect(i)` indexes (`Item.cs:55-66`).
    pub affect_index: usize,
    pub id: u8,
    pub scribing: bool,
}

/// The three affect bytes of one scroll record, decoded. Non-zero entries only.
pub fn scroll_spells(record: &[u8]) -> impl Iterator<Item = ScrollSpell> + '_ {
    (1..=3).filter_map(move |i| {
        let raw = gbx_formats::save_orig::item_affect(record, i);
        (raw & 0x7F != 0).then_some(ScrollSpell {
            affect_index: i,
            id: raw & 0x7F,
            scribing: raw > 0x7F,
        })
    })
}

/// `cancel_scribes` (`ovr016.cs:75-86`): clears the staging high bit from every
/// affect byte of every scroll the character carries. Camp entry and camp exit
/// both run it (via `cancel_spells`), so a scribe left un-rested is discarded.
pub fn cancel_scribes(items: &mut [Vec<u8>], scrolls: &ScrollLookup) {
    for item in items.iter_mut() {
        if scrolls.is_scroll(item) {
            for i in 1..=3 {
                let v = gbx_formats::save_orig::item_affect(item, i);
                gbx_formats::save_orig::set_item_affect(item, i, v & 0x7F);
            }
        }
    }
}

/// ★ `cancel_spells` (`ovr016.cs:89-96`) — `cancel_memorize` + `cancel_scribes`
/// for **every** party member. `MakeCamp` runs it twice: once on entry
/// (`ovr016.cs:1095`, right after "The party makes camp...") and once on exit
/// (`ovr016.cs:1154`, after the picture restore).
///
/// (The door cited `:1117`/`:1150-region` for the pair; the re-verified lines
/// are `1095` and `1154`.)
///
/// Staged-but-uncommitted spells therefore do not survive leaving camp — and,
/// because entry runs it too, they do not survive *re-entering* camp either.
pub fn cancel_spells(party: &mut crate::party::Party, scrolls: &ScrollLookup) {
    for member in &mut party.members {
        cancel_memorize(&mut member.magic);
        cancel_scribes(&mut member.items, scrolls);
    }
}

// ---------------------------------------------------------------------------
// The list presentation — `BuildSpellList` and its two row builders (D-S4d)
// ---------------------------------------------------------------------------

/// `SpellLoc` (`ovr020.cs:6-15`) — which set of spells a list shows, and the
/// heading `spell_menu2` writes next to the character's name (`:1375-1410`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpellLoc {
    /// What is memorized right now — the Cast picker.
    Memory,
    /// The grimoire: everything known and learnable — the Memorize picker.
    Grimoire,
    /// One scroll's spells.
    Scroll,
    /// Every scroll's un-staged spells — the Scribe picker.
    Scrolls,
    /// Learnable-but-unknown spells (the training "choose a new spell" flow).
    Choose,
    /// What is staged for memorization — Memorize's review pass.
    Memorize,
    /// What is staged for scribing — Scribe's review pass.
    Scribe,
}

impl SpellLoc {
    /// `spell_menu2`'s heading suffix (`ovr020.cs:1375-1410`), shown as
    /// `"Spells <text>"` beside the character's name.
    pub fn heading(self) -> &'static str {
        match self {
            SpellLoc::Memory => "in Memory",
            SpellLoc::Grimoire => "in Grimoire",
            SpellLoc::Scroll => "on Scroll",
            SpellLoc::Scrolls => "on Scrolls",
            SpellLoc::Choose => "to Choose",
            SpellLoc::Memorize => "to Memorize",
            SpellLoc::Scribe => "to Scribe",
        }
    }

    /// `BuildSpellList`'s `buildSpellList` flag (`ovr023.cs:395-472`): the
    /// scroll locations build their own headings inline and skip the
    /// level-heading post-pass.
    fn groups_by_level(self) -> bool {
        !matches!(
            self,
            SpellLoc::Scroll | SpellLoc::Scrolls | SpellLoc::Scribe
        )
    }
}

/// `SpellSource` (`Classes/Spells.cs:31-35`) — the verb in the list's prompt
/// (`spell_menu`, `ovr023.cs:177-198`) and the one that shrinks the Memorize
/// list's box (`:202`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SpellSource {
    None,
    Cast,
    Memorize,
    Scribe,
    Learn,
}

impl SpellSource {
    /// `spell_menu`'s `text` (`ovr023.cs:177-198`).
    pub fn verb(self) -> &'static str {
        match self {
            SpellSource::None => "",
            SpellSource::Cast => "Cast",
            SpellSource::Memorize => "Memorize",
            SpellSource::Scribe => "Scribe",
            SpellSource::Learn => "Learn",
        }
    }

    /// `spell_menu`'s `prompt_text` (`:200`) — present only when there is a
    /// verb.
    pub fn prompt(self) -> &'static str {
        if self.verb().is_empty() {
            ""
        } else {
            "Choose Spell: "
        }
    }

    /// `spell_menu`'s `end_y` (`ovr023.cs:202`): the Memorize picker's box
    /// stops at row `0x0F` so `BuildMemorizeSpellText`'s capacity table has
    /// room underneath; every other list runs to `0x16`.
    pub fn list_end_row(self) -> usize {
        if matches!(self, SpellSource::Memorize) {
            0x0F
        } else {
            0x16
        }
    }
}

/// `sl_select_item`'s box for every spell list (`ovr023.cs:223-224`:
/// `startY = 5`, `startX = 1`, `endX = 0x26`).
pub fn spell_list_layout(source: SpellSource) -> crate::widgets::ListLayout {
    crate::widgets::ListLayout {
        start_row: 5,
        start_col: 1,
        end_row: source.list_end_row(),
        end_col: 0x26,
    }
}

/// A built spell list: the rows `sl_select_item` shows, plus the parallel
/// `gbl.memorize_spell_id` / `gbl.scribeScrolls` arrays that turn a chosen row
/// back into a spell (and, for a scroll list, into the item it came from).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpellListing {
    pub items: Vec<crate::widgets::ListItem>,
    /// One id per **non-heading** row, in row order (`gbl.memorize_spell_id`).
    pub ids: Vec<u8>,
    /// One item index per non-heading row for scroll lists
    /// (`gbl.scribeScrolls`); empty for the others.
    pub scroll_items: Vec<usize>,
}

impl SpellListing {
    /// `spell_menu`'s row→id resolution (`ovr023.cs:236-237`): count the
    /// non-heading rows before the chosen one and index the parallel array.
    pub fn id_at_row(&self, row: usize) -> Option<u8> {
        let entry = self.entry_index(row)?;
        self.ids.get(entry).copied()
    }

    /// The scroll this row's spell is written on (`gbl.currentScroll`,
    /// `ovr023.cs:239-242`).
    pub fn scroll_at_row(&self, row: usize) -> Option<usize> {
        let entry = self.entry_index(row)?;
        self.scroll_items.get(entry).copied()
    }

    fn entry_index(&self, row: usize) -> Option<usize> {
        if row >= self.items.len() || self.items[row].is_heading_row() {
            return None;
        }
        Some(
            self.items[..row]
                .iter()
                .filter(|i| !i.is_heading_row())
                .count(),
        )
    }

    /// `BuildSpellList`'s return value (`ovr023.cs:474-511`): whether the list
    /// has anything in it. An empty list is what makes `spell_menu2` return 0
    /// without drawing, which every caller reads as "nothing here".
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// `add_spell_to_learning_list` / `sub_5C5B9` (`ovr023.cs:275-346`) — the
/// level-sorted, duplicate-collapsing insert the memory/memorize/grimoire/
/// choose lists are built with.
///
/// The scan (`:295-306`) walks the rows already placed and stops at the first
/// one whose spell level is **higher** than the incoming spell's, or at an
/// entry with the **same id**. So a new id lands at the end of its own level
/// run, and a repeat collapses onto the existing row, whose text grows a
/// ` (N)` multiplicity suffix (`:325-328`).
fn add_spell_to_learning_list(listing: &mut SpellListing, counts: &mut Vec<u32>, spell_id: u8) {
    let masked = spell_id & 0x7F;
    let level = spell_level(masked);
    let mut at = listing.ids.len();
    for (i, &existing) in listing.ids.iter().enumerate() {
        if spell_level(existing) > level || existing == masked {
            at = i;
            break;
        }
    }
    let collapsing = listing.ids.get(at) == Some(&masked);
    if collapsing {
        listing.items.remove(at);
        counts[at] += 1;
    } else {
        listing.ids.insert(at, masked);
        counts.insert(at, 1);
    }
    let suffix = if counts[at] > 1 {
        format!(" ({})", counts[at])
    } else {
        String::new()
    };
    let star = if spell_id > 0x7F { '*' } else { ' ' };
    listing.items.insert(
        at,
        crate::widgets::ListItem::Entry(format!(" {star}{}{suffix}", spell_name(masked))),
    );
}

/// `add_spell_to_list` / `sub_5C3ED` (`ovr023.cs:250-272`) — the scroll lists'
/// simpler builder: rows in the order the scrolls are carried, with a level
/// heading emitted whenever the level differs from the previous row's, and no
/// duplicate collapsing. The `*` marks a spell already staged for scribing.
fn add_spell_to_scroll_list(listing: &mut SpellListing, spell_id: u8, item_index: usize) {
    let masked = spell_id & 0x7F;
    let last_level = listing.ids.last().copied().map_or(0, spell_level);
    let level = spell_level(masked);
    if level != last_level {
        listing.items.push(crate::widgets::ListItem::Heading(
            level_string(level).into(),
        ));
    }
    let star = if spell_id > 0x7F { '*' } else { ' ' };
    listing.items.push(crate::widgets::ListItem::Entry(format!(
        " {star}{}",
        spell_name(masked)
    )));
    listing.ids.push(masked);
    listing.scroll_items.push(item_index);
}

/// ★ `BuildSpellList` / `sub_5CA74` (`ovr023.cs:395-512`) — every spell list
/// the camp magic menu shows, from one switch on [`SpellLoc`].
///
/// The level headings for the grouped locations are inserted in a **post-pass**
/// (`:474-509`) once the rows are placed, which is why they can be emitted in
/// list order without disturbing the sorted insert above.
pub fn build_spell_list(
    loc: SpellLoc,
    ch: &Character,
    scrolls: &ScrollLookup,
    current_scroll: Option<usize>,
) -> SpellListing {
    let mut listing = SpellListing::default();
    let mut counts: Vec<u32> = Vec::new();

    match loc {
        SpellLoc::Memory => {
            for e in learnt(&ch.magic.spell_list) {
                if can_learn_spell_in_camp(ch, e.id) {
                    add_spell_to_learning_list(&mut listing, &mut counts, e.id);
                }
            }
        }
        SpellLoc::Memorize => {
            for e in learning(&ch.magic.spell_list) {
                if can_learn_spell_in_camp(ch, e.id) {
                    add_spell_to_learning_list(&mut listing, &mut counts, e.id);
                }
            }
        }
        SpellLoc::Grimoire => {
            for id in known_spells(ch) {
                if can_learn_spell_in_camp(ch, id) {
                    add_spell_to_learning_list(&mut listing, &mut counts, id);
                }
            }
        }
        SpellLoc::Choose => {
            for (id, row) in SPELL_TABLE.iter().enumerate().skip(1) {
                let id = id as u8;
                if row.level > 5
                    || row.class_ == SpellClass::Monster
                    || row.class_ == SpellClass::Unknown10
                {
                    continue; // `:460-463`
                }
                if cast_count_at(&ch.magic, row.class_, row.level) > 0
                    && can_learn_spell_in_camp(ch, id)
                    && !knows_spell(ch, id)
                {
                    add_spell_to_learning_list(&mut listing, &mut counts, id);
                }
            }
        }
        SpellLoc::Scroll => {
            if let Some(idx) = current_scroll {
                add_scroll(&mut listing, ch, scrolls, idx, false);
            }
        }
        SpellLoc::Scrolls => build_scroll_lists(&mut listing, ch, scrolls, false),
        SpellLoc::Scribe => build_scroll_lists(&mut listing, ch, scrolls, true),
    }

    if loc.groups_by_level() && !listing.items.is_empty() {
        insert_level_headings(&mut listing);
    }
    listing
}

/// `BuildSpellList`'s heading post-pass (`ovr023.cs:478-506`).
fn insert_level_headings(listing: &mut SpellListing) {
    let mut inserts: Vec<(usize, u8)> = Vec::new();
    let mut level = 0;
    let mut insert = 0;
    for (idx, _) in listing.items.iter().enumerate() {
        let last = level;
        if let Some(&id) = listing.ids.get(idx) {
            if id != 0 {
                level = spell_level(id);
            }
        }
        if level > last {
            inserts.push((insert, level));
            insert += 1;
        }
        insert += 1;
    }
    for (pos, lvl) in inserts {
        listing.items.insert(
            pos,
            crate::widgets::ListItem::Heading(level_string(lvl).into()),
        );
    }
}

/// `scroll_5C912` / `sub_5C912` (`ovr023.cs:349-371`): one scroll's rows.
/// `learning == true` shows only the spells already staged for scribing
/// (`> 0x80`), `false` shows every spell on it.
///
/// The `hidden_names_flag` gate (`:358`) is the original's read-magic
/// requirement: a scroll whose names are still hidden lists nothing. Its
/// unhiding conditions (`:351-356` — a `read_magic` affect, or a cleric
/// holding a cleric scroll) need the out-of-combat affect system, so this
/// reads the stored flag and does not clear it. Named, not silently dropped.
fn add_scroll(
    listing: &mut SpellListing,
    ch: &Character,
    scrolls: &ScrollLookup,
    item_index: usize,
    learning_only: bool,
) {
    let Some(item) = ch.items.get(item_index) else {
        return;
    };
    if !scrolls.is_scroll(item) {
        return;
    }
    if gbx_formats::save_orig::item_hidden_names_flag(item) != 0 {
        return;
    }
    for i in 1..=3 {
        let raw = gbx_formats::save_orig::item_affect(item, i);
        let show = if learning_only { raw > 0x80 } else { raw > 0 };
        if show {
            add_spell_to_scroll_list(listing, raw, item_index);
        }
    }
}

/// `BuildScrollSpellLists` / `sub_5C9F4` (`ovr023.cs:374-392`).
fn build_scroll_lists(
    listing: &mut SpellListing,
    ch: &Character,
    scrolls: &ScrollLookup,
    learning_only: bool,
) {
    for idx in 0..ch.items.len() {
        add_scroll(listing, ch, scrolls, idx, learning_only);
    }
}

/// `player.KnowsSpell(spell)` (`Player.cs:363`): `spellBook[id - 1] != 0`.
pub fn knows_spell(ch: &Character, id: u8) -> bool {
    id >= 1
        && ch
            .magic
            .spell_book
            .get(id as usize - 1)
            .is_some_and(|&b| b != 0)
}

/// `player.LearnSpell(spell)` (`Player.cs:364`).
pub fn learn_spell(ch: &mut Character, id: u8) {
    if id < 1 {
        return;
    }
    let slot = id as usize - 1;
    if ch.magic.spell_book.len() <= slot {
        ch.magic.spell_book.resize(slot + 1, 0);
    }
    ch.magic.spell_book[slot] = 1;
}

/// Every id in the grimoire, ascending — `System.Enum.GetValues(typeof(Spells))`
/// order (`ovr023.cs:429`), which is ascending id.
fn known_spells(ch: &Character) -> Vec<u8> {
    (1..SPELL_TABLE.len() as u16)
        .map(|id| id as u8)
        .filter(|&id| knows_spell(ch, id))
        .collect()
}

/// ★ `BuildMemorizeSpellText` / `sub_445D4` (`ovr016.cs:203-271`) — the
/// capacity table under the Memorize picker: one row per class the character
/// has any slots in, five columns of "how many more at this level".
///
/// A level with **zero total slots** shows a blank rather than a number
/// (`:221-224`), so a level-3 cleric's row reads `2 1     ` and not `2 1 0 0 0`.
/// `found` is false — and the caller reports "cannot memorize any spells"
/// (`ovr016.cs:334-338`) — when the character has no slots at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorizeCapacityRow {
    pub class_: SpellClass,
    /// Five cells, level 1..=5, already rendered (`" "` for an unusable level).
    pub cells: [String; 5],
}

/// The capacity table, or an empty vec when the character has no slots at all.
pub fn memorize_capacity_table(ch: &Character) -> Vec<MemorizeCapacityRow> {
    let mut rows = Vec::new();
    for class_ in [SpellClass::Cleric, SpellClass::Druid, SpellClass::MagicUser] {
        let mut cells: [String; 5] = Default::default();
        let mut any = false;
        for level in 1..=5u8 {
            if cast_count_at(&ch.magic, class_, level) == 0 {
                cells[level as usize - 1] = " ".into();
            } else {
                any = true;
                cells[level as usize - 1] =
                    how_many_spells_player_can_learn(&ch.magic, class_, level).to_string();
            }
        }
        if any {
            rows.push(MemorizeCapacityRow { class_, cells });
        }
    }
    rows
}

/// `EffectNameMap` (`BuildEffectNameMap`, `ovr016.cs:503-553`) — the Magic ▸
/// Display screen's affect names.
///
/// The first 35 are *derived*: for each affect id in the listed set, scan
/// `spellCastingTable[1..=0x38]` for the first row carrying that `affect_id`
/// and take that spell's name (`:518-525`). Two of them are worth noticing —
/// `paralyze` (`0x34`) resolves to **"Hold Person"** and `blinded` (`0x21`) to
/// **"Cause Blindness"**, because that is the first spell that applies them —
/// and `animate_dead` (`0x20`) resolves to nothing at all in the `1..=0x38`
/// window, so it keeps coab's literal fallback string `"Funky--animate_dead"`
/// (`:529`). Kept verbatim: it is what the screen shows.
///
/// The remaining nineteen are the explicit `Add` calls at `:533-551`.
pub fn effect_name(affect_id: u8) -> Option<&'static str> {
    let name = match affect_id {
        0x01 => "Bless",
        0x02 => "Curse",
        0x04 => "Dispel Evil",
        0x05 => "Detect Magic",
        0x07 => "Faerie Fire",
        0x08 => "Protection from Evil",
        0x09 => "Protection from Good",
        0x0A => "Resist Cold",
        0x0B => "Charm Person",
        0x0C => "Enlarge",
        0x0E => "Friends",
        0x10 => "Read Magic",
        0x11 => "Shield",
        0x13 => "Find Traps",
        0x14 => "Resist Fire",
        0x15 => "Silence, 15' Radius",
        0x16 => "Slow Poison",
        0x17 => "Spiritual Hammer",
        0x18 => "Detect Invisibility",
        0x19 => "Invisibility",
        0x1B => "Fumbling",
        0x1C => "Mirror Image",
        0x1D => "Ray of Enfeeblement",
        0x1F => "Helpless",
        0x20 => "Funky--animate_dead",
        0x21 => "Cause Blindness",
        0x22 => "Cause Disease",
        0x23 => "Confused",
        0x24 => "Bestow Curse",
        0x25 => "Blink",
        0x26 => "Strength",
        0x27 => "Haste",
        0x29 => "Protection From Normal Missiles",
        0x2A => "Slow",
        0x2C => "Cause Disease",
        0x2D => "Protection From Evil, 10' Radius",
        0x2E => "Protection From Good, 10' Radius",
        0x31 => "Prayer",
        0x32 => "Hot Fire Shield",
        0x33 => "Snake Charm",
        0x34 => "Hold Person",
        0x35 => "Sleep",
        0x36 => "Cold Fire Shield",
        0x37 => "Poisoned",
        0x3B => "Regenerating",
        0x3D => "Fire Resistance",
        0x3F => "Minor Globe of Invulnerability",
        0x44 => "enfeebled",
        0x45 => "invisible to animals",
        0x47 => "Invisible",
        0x48 => "Camouflaged",
        0x49 => "protected from dragon breath",
        0x4D => "berserk",
        0x59 => "Displaced",
        _ => return None,
    };
    Some(name)
}

/// `sub_443A0` (`ovr016.cs:116-156`) — may this character do this right now?
/// `Err` carries the exact refusal line `DisplayPlayerStatusString` shows after
/// the character's name.
///
/// `learn_type` is the original's: `1` cast, `2` memorize, `3` scribe. The
/// `learn_type == 1` arm tests the area flag and is an **`else if`**, so the
/// health check below never runs for casting (`:120-127`).
///
/// ★ The area flag reads inverted, and is transcribed that way: coab's
/// `if (gbl.area_ptr.can_cast_spells == true) text = "cannot cast spells in
/// this area"` (`:122-125`). Whatever the cell is called, *set* means barred.
pub fn learn_gate(
    ch: &Character,
    learn_type: u8,
    area_bars_casting: bool,
) -> Result<(), &'static str> {
    if learn_type == 1 {
        if area_bars_casting {
            return Err("cannot cast spells in this area");
        }
    } else if ch.status.health_status == crate::rest::status::ANIMATED || !ch.status.in_combat {
        return Err(match learn_type {
            2 => "is in no condition to memorize spells",
            3 => "is in no condition to scribe any scrolls",
            _ => "is in no condition to ",
        });
    }
    Ok(())
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

    fn scroll_table() -> ScrollLookup {
        let mut bytes = vec![0u8; gbx_formats::items::ITEMS_HEADER_SIZE];
        bytes.extend_from_slice(&[0u8; gbx_formats::items::ITEM_ENTRY_SIZE]); // type 0
        let mut scroll = [0u8; gbx_formats::items::ITEM_ENTRY_SIZE];
        scroll[0] = 11;
        bytes.extend_from_slice(&scroll);
        let data = gbx_formats::game_data::GameData::from_files(vec![(
            crate::combat_host::ITEMS_FILE.to_string(),
            bytes,
        )]);
        ScrollLookup::load(&data)
    }

    /// Rows as the original writes them: `" {*|space}{name}"`
    /// (`ovr023.cs:269`/`:325-328`), so an unstaged row starts with two
    /// spaces and a staged one with a space then `*`. Headings are bracketed
    /// here purely to make the assertions readable.
    fn entry_texts(l: &SpellListing) -> Vec<String> {
        l.items
            .iter()
            .map(|i| match i {
                crate::widgets::ListItem::Heading(t) => format!("[{t}]"),
                crate::widgets::ListItem::Entry(t) => t.clone(),
            })
            .collect()
    }

    /// ★ `add_spell_to_learning_list` sorts by level, appends within a level,
    /// and collapses repeats into a ` (N)` row (`ovr023.cs:275-346`); the
    /// level headings arrive in `BuildSpellList`'s post-pass (`:478-506`).
    #[test]
    fn the_grimoire_list_is_level_sorted_with_headings_and_collapsed_repeats() {
        let mut ch = caster();
        ch.class_level[crate::party::SKILL_MAGIC_USER] = 5;
        for id in [0x0F, 0x15, 0x1E, 0x22] {
            learn_spell(&mut ch, id); // MM, Sleep (MU1); Invisibility, Stinking Cloud (MU2)
        }
        let l = build_spell_list(SpellLoc::Grimoire, &ch, &ScrollLookup::default(), None);
        assert_eq!(
            entry_texts(&l),
            vec![
                "[1st Level]",
                "  Magic Missile",
                "  Sleep",
                "[2nd Level]",
                "  Invisibility",
                "  Stinking Cloud",
            ]
        );
        assert_eq!(l.ids, vec![0x0F, 0x15, 0x1E, 0x22]);
        // Rows map back through the headings.
        assert_eq!(l.id_at_row(1), Some(0x0F));
        assert_eq!(l.id_at_row(4), Some(0x1E));
        assert_eq!(l.id_at_row(0), None, "a heading is not selectable");
    }

    /// The memorize review list collapses duplicates — two staged Magic
    /// Missiles are one row reading ` Magic Missile (2)`.
    #[test]
    fn the_memorize_review_list_collapses_duplicates() {
        let mut ch = caster();
        ch.class_level[crate::party::SKILL_MAGIC_USER] = 5;
        add_learn(&mut ch.magic.spell_list, 0x0F);
        add_learn(&mut ch.magic.spell_list, 0x0F);
        add_learn(&mut ch.magic.spell_list, 0x15);
        let l = build_spell_list(SpellLoc::Memorize, &ch, &ScrollLookup::default(), None);
        assert_eq!(
            entry_texts(&l),
            vec!["[1st Level]", "  Sleep", "  Magic Missile (2)"]
        );
        assert_eq!(l.ids, vec![0x15, 0x0F]);
    }

    /// The scroll lists build headings inline, keep carry order, do not
    /// collapse, and mark a staged scribe with `*`
    /// (`add_spell_to_list`, `ovr023.cs:250-272`; `scroll_5C912`, `:349-371`).
    #[test]
    fn the_scroll_lists_mark_staged_spells_with_a_star() {
        let scrolls = scroll_table();
        let mut ch = caster();
        let mut item = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        item[0x2E] = 1;
        gbx_formats::save_orig::set_item_affect(&mut item, 1, 0x0F); // not staged
        gbx_formats::save_orig::set_item_affect(&mut item, 2, 0x1E | 0x80); // staged
        ch.items.push(item);

        // `Scrolls` — everything on every scroll.
        let all = build_spell_list(SpellLoc::Scrolls, &ch, &scrolls, None);
        assert_eq!(
            entry_texts(&all),
            vec![
                "[1st Level]",
                "  Magic Missile",
                "[2nd Level]",
                " *Invisibility"
            ]
        );
        assert_eq!(all.scroll_at_row(1), Some(0));

        // `Scribe` — only the staged ones.
        let staged = build_spell_list(SpellLoc::Scribe, &ch, &scrolls, None);
        assert_eq!(entry_texts(&staged), vec!["[2nd Level]", " *Invisibility"]);
    }

    /// A scroll whose names are still hidden lists nothing (`ovr023.cs:358`).
    #[test]
    fn a_hidden_name_scroll_lists_nothing() {
        let scrolls = scroll_table();
        let mut ch = caster();
        let mut item = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        item[0x2E] = 1;
        item[0x35] = 1; // hidden_names_flag
        gbx_formats::save_orig::set_item_affect(&mut item, 1, 0x0F);
        ch.items.push(item);
        assert!(build_spell_list(SpellLoc::Scrolls, &ch, &scrolls, None).is_empty());
    }

    /// ★ `BuildMemorizeSpellText`: an unusable level is a blank, not a zero,
    /// and a character with no slots at all produces no table
    /// (`ovr016.cs:203-271`).
    #[test]
    fn the_capacity_table_blanks_levels_with_no_slots() {
        let mut ch = caster();
        assert!(memorize_capacity_table(&ch).is_empty());

        // SHARA's real cleric row: 5/5/2 at levels 1-3.
        ch.magic.cast_count[0] = [5, 5, 2, 0, 0];
        add_learnt(&mut ch.magic.spell_list, 0x03); // one level-1 already held
        let rows = memorize_capacity_table(&ch);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].class_, SpellClass::Cleric);
        assert_eq!(rows[0].cells, ["4", "5", "2", " ", " "]);
        assert_eq!(rows[0].class_.memorize_heading(), "    Cleric Spells:");
    }

    /// `spell_menu`'s box: the Memorize picker stops eight rows short so the
    /// capacity table fits underneath (`ovr023.cs:202`).
    #[test]
    fn the_memorize_picker_gets_the_short_box() {
        assert_eq!(spell_list_layout(SpellSource::Memorize).end_row, 0x0F);
        assert_eq!(spell_list_layout(SpellSource::Scribe).end_row, 0x16);
        assert_eq!(spell_list_layout(SpellSource::Memorize).start_row, 5);
        assert_eq!(SpellLoc::Memorize.heading(), "to Memorize");
        assert_eq!(SpellSource::Memorize.prompt(), "Choose Spell: ");
        assert_eq!(SpellSource::None.prompt(), "");
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
