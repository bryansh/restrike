//! ★ The inventory model — item names, `reclac_player_values`, and the verbs
//! the character sheet's **Items** leaf offers (roll-credits §12, G6).
//!
//! Derived by reading coab for behavior (D11, never copied). The flows are
//! `PlayerItemsMenu`/`ready_Item`/`halve_items`/`join_items`/`trade_item`/
//! `CanSellDropTradeItem` (`ovr020.cs:432-623`, `:787-977`, `:342-375`),
//! `reclac_player_values` and its five helpers (`ovr025.cs:10-168`, `:338-495`),
//! and `ItemDisplayNameBuild`/`Item.GenerateName` (`ovr025.cs:170-215`,
//! `Classes/Item.cs:264-303`).
//!
//! ## Where an item's name comes from — and where it does *not*
//!
//! An item record carries a 0x2A-byte name field at offset 0, and this engine
//! has been reading it since M6 ([`gbx_formats::save_orig::item_name`]). That
//! field is a **snapshot, not the name**: `ItemDisplayNameBuild` opens with
//! `item.name = string.Empty` and rebuilds the whole string from
//! `namenum1/2/3` every single time an item is drawn (`ovr025.cs:172`). Two
//! measurements from the shipped data settle it:
//!
//! - the party saves the staging rig writes carry an **all-zero** name field,
//!   so `item_name` returns `""` for every item a fight has ever seen — which
//!   is why `combat_host::weapon_display_names` quietly built an empty map;
//! - the authored `ITEM{area}.DAX` records *do* carry a name, and it is
//!   **stale**: `ITEM1.DAX#5[0]` (type 101, `namenum = (0, 47, 159)`) stores
//!   `"Small Raft Sling"` where the shipped word table spells
//!   `"Staff Sling"`, and `ITEM2.DAX#2[5]` stores `"Instrument Maul"` for what
//!   the game calls an `"Ioun Stone Deep Red"`. The authoring tool's table and
//!   the game's are not the same table.
//!
//! So the name is generated here, from [`ITEM_NAMES`].
//!
//! ★ **[`ITEM_NAMES`] is evidence-verified, not merely transcribed.** All 256
//! entries were checked, in order, against the length-prefixed string heap
//! inside the shipped `START.EXE` (the run beginning `0a "Battle Axe"` at file
//! offset `0xB521`): every non-empty coab entry appears there as
//! `<len><chars>`, in ascending order, with zero mismatches — including the
//! three empty slots at 62, 63 and 144. Same class of functional interface
//! vocabulary as [`crate::magic::SPELL_TABLE`]'s spell names and
//! `charsheet`'s class/race/status tables, and reproduced on the same D10
//! clarification.

use crate::party::Character;
use gbx_formats::items::ItemDataTable;
use gbx_formats::save_orig as rec;
use gbx_rules::flavor::Flavor;

/// `Player.MaxItems` (`Classes/Player.cs:561`) — the inventory cap that gates
/// the **Halve** word and `canCarry`.
pub const MAX_ITEMS: usize = 16;

/// `ItemType.Arrow` (`Classes/ItemData.cs:190`).
pub const TYPE_ARROW: u8 = 73;
/// `ItemType.Quarrel` (`:145`).
pub const TYPE_QUARREL: u8 = 28;
/// `ItemType.Dart` (`:126`).
pub const TYPE_DART: u8 = 9;
/// `ItemType.FlaskOfOil` (`:203`) — the one type the pluralizer special-cases.
pub const TYPE_FLASK_OF_OIL: u8 = 86;

/// `ItemSlot.Armor` (`Classes/ItemData.cs:24`).
pub const SLOT_ARMOR: u8 = 2;
/// `ItemSlot.slot_1` (`:23`) — the shield slot `sub_662A6` reads specially.
pub const SLOT_1: u8 = 1;
/// `ItemSlot.slot_9` (`:31`) — the two-pointer "held" slot.
pub const SLOT_9: u8 = 9;
/// `ItemSlot.slot_11`..`slot_13` (`:33-35`) — `Item.IsScroll()`'s range.
pub const SLOT_SCROLL_FIRST: u8 = 11;
/// The clerical-scroll slot (`ItemSlot.Quarrel = 12`), which `scroll_5C912`
/// names as the one a cleric can read without Read Magic (`ovr023.cs:351`).
pub const SLOT_CLERIC_SCROLL: u8 = 12;
/// The last scroll slot.
pub const SLOT_SCROLL_LAST: u8 = 13;

/// `Affects.detect_magic` (`Classes/Affect.cs`) — the party-wide flag that
/// makes `ItemDisplayNameBuild` prefix `"* "` on a magical item
/// (`ovr025.cs:189-195`).
pub const AFF_DETECT_MAGIC: u8 = 0x05;

// ---------------------------------------------------------------------------
// The word table
// ---------------------------------------------------------------------------

/// `Item.itemNames` (`Classes/Item.cs:199-262`; the game's own heap table at
/// `START.EXE:0xB521`, verified entry-for-entry — see the module doc).
/// `namenum1/2/3` index this; index `0` means "no word", which is how
/// `GenerateName`'s display-flag test spells an absent component.
#[rustfmt::skip]
pub const ITEM_NAMES: [&str; 256] = [
    "", "Battle Axe", "Hand Axe", "Bardiche", "Bec De Corbin", "Bill-Guisarme", "Bo Stick",
    "Club", "Dagger", "Dart", "Fauchard", "Fauchard-Fork", "Flail", "Military Fork", "Glaive",
    "Glaive-Guisarme", "Guisarme", "Guisarme-Voulge", "Halberd", "Lucern Hammer", "Hammer",
    "Javelin", "Jo Stick", "Mace", "Morning Star", "Partisan", "Military Pick", "Awl Pike",
    "Quarrel", "Ranseur", "Scimitar", "Spear", "Spetum", "Quarter Staff", "Bastard Sword",
    "Broad Sword", "Long Sword", "Short Sword", "Two-Handed Sword", "Trident", "Voulge",
    "Composite Long Bow", "Composite Short Bow", "Long Bow", "Short Bow", "Heavy Crossbow",
    "Light Crossbow", "Sling", "Mail", "Armor", "Leather", "Padded", "Studded", "Ring",
    "Scale", "Chain", "Splint", "Banded", "Plate", "Shield", "Woods", "Arrow", "", "",
    "Potion", "Scroll", "Ring", "Rod", "Stave", "Wand", "Jug", "Amulet", "Dragon Breath",
    "Bag", "Defoliation", "Ice Storm", "Book", "Boots", "Hornets Nest", "Bracers", "Piercing",
    "Brooch", "Elfin Chain", "Wizardry", "ac10", "Dexterity", "Fumbling", "Chime", "Cloak",
    "Crystal", "Cube", "Cubic", "The Dwarves", "Decanter", "Gloves", "Drums", "Dust",
    "Thievery", "Hat", "Flask", "Gauntlets", "Gem", "Girdle", "Helm", "Horn", "Stupidity",
    "Incense", "Stone", "Ioun Stone", "Javelin", "Jewel", "Ointment", "Pale Blue",
    "Scarlet And", "Manual", "Incandescent", "Deep Red", "Pink", "Mirror", "Necklace",
    "And Green", "Blue", "Pearl", "Powerlessness", "Vermin", "Pipes", "Hole", "Dragon Slayer",
    "Robe", "Rope", "Frost Brand", "Berserker", "Scarab", "Spade", "Sphere", "Blessed",
    "Talisman", "Tome", "Trident", "Grimoire", "Well", "Wings", "Vial", "Lantern", "",
    "Flask of Oil", "10 ft. Pole", "50 ft. Rope", "Iron", "Thf Prickly Tools", "Iron Rations",
    "Standard Rations", "Holy Symbol", "Holy Water vial", "Unholy Water vial", "Barding",
    "Dragon", "Lightning", "Saddle", "Staff", "Drow", "Wagon", "+1", "+2", "+3", "+4", "+5",
    "of", "Vulnerability", "Cloak", "Displacement", "Torches", "Oil", "Speed", "Tapestry",
    "Spine", "Copper", "Silver", "Electrum", "Gold", "Platinum", "Ointment", "Keoghtum's",
    "Sheet", "Strength", "Healing", "Holding", "Extra", "Gaseous Form", "Slipperiness",
    "Jewelled", "Flying", "Treasure Finding", "Fear", "Disappearance", "Statuette", "Fungus",
    "Chain", "Pendant", "Broach", "Of Seeking", "-1", "-2", "-3", "Lightning Bolt",
    "Fire Resistance", "Magic Missiles", "Save", "Clrc Scroll", "MU Scroll", "With 1 Spell",
    "With 2 Spells", "With 3 Spells", "Prot. Scroll", "Jewelry", "Fine", "Huge", "Bone",
    "Brass", "Key", "AC 2", "AC 6", "AC 4", "AC 3", "Of Prot.", "Paralyzation", "Ogre Power",
    "Invisibility", "Missiles", "Elvenkind", "Rotting", "Covered", "Efreeti", "Bottle",
    "Missile Attractor", "Of Maglubiyet", "Secr Door & Trap Det", "Gd Dragon Control",
    "Feather Falling", "Giant Strength", "Restoring Level(s)", "Flame Tongue", "Fireballs",
    "Spiritual", "Boulder", "Diamond", "Emerald", "Opal", "Saphire", "Of Tyr", "Of Tempus",
    "Of Sune", "Wooden", "+3 vs Undead", "Pass", "Cursed",
];

/// `Item.GenerateName(hidden_names_flag)` (`Classes/Item.cs:264-303`).
///
/// The three name words are emitted **3 → 2 → 1**, each present only when its
/// `namenum` is non-zero *and* its hide bit is clear — the bit order is
/// reversed against the word order (`namenum1` hides on `0x4`, `namenum2` on
/// `0x2`, `namenum3` on `0x1`), which is what makes a `hidden_names_flag` of
/// `6` show the third word alone ("Potion", "MU Scroll").
///
/// The pluralizer appends `"s "` to at most one word, chosen by the original's
/// own four-way disjunction; everything else gets a plain space, and the whole
/// string is trimmed.
pub fn generate_name(record: &[u8], hidden_names_flag: u8) -> String {
    let nn = |i: usize| rec::item_namenum(record, i);
    let count = rec::item_count(record);
    let item_type = rec::item_type(record);

    let mut display_flags = 0u8;
    if nn(1) != 0 && hidden_names_flag & 0x4 == 0 {
        display_flags |= 0x1;
    }
    if nn(2) != 0 && hidden_names_flag & 0x2 == 0 {
        display_flags |= 0x2;
    }
    if nn(3) != 0 && hidden_names_flag & 0x1 == 0 {
        display_flags |= 0x4;
    }

    let mut name = String::new();
    let mut plural_added = false;
    for v in (1..=3usize).rev() {
        if (display_flags >> (v - 1)) & 1 == 0 {
            continue;
        }
        name.push_str(word(nn(v)));
        if count < 2 || plural_added {
            name.push(' ');
        } else if (1 << (v - 1)) == display_flags
            || (v == 1 && display_flags > 4 && item_type != TYPE_FLASK_OF_OIL)
            || (v == 2 && display_flags & 1 == 0)
            || (v == 3 && item_type == TYPE_FLASK_OF_OIL)
            || (nn(3) != 0x87
                && matches!(item_type, TYPE_ARROW | TYPE_QUARREL | TYPE_DART)
                && nn(3) != 0xB1)
        {
            name.push_str("s ");
            plural_added = true;
        } else {
            name.push(' ');
        }
    }
    name.trim().to_string()
}

/// One [`ITEM_NAMES`] word. Out-of-range is impossible (the table covers every
/// `u8`), but the lookup stays total for the same reason `charsheet::lookup` is.
fn word(index: u8) -> &'static str {
    ITEM_NAMES.get(index as usize).copied().unwrap_or("")
}

/// `ItemDisplayNameBuild(display_new_name, displayReadied, …)`
/// (`ovr025.cs:170-215`) — the *list row*, which is the generated name with up
/// to three prefixes in front of it:
///
/// 1. `displayReadied` (the items list passes `true`) prepends the readied
///    column, `" Yes  "` or `" No   "`, with the original's own padding;
/// 2. any party member carrying `detect_magic` prepends `"* "` to an item with
///    a `plus`, a `plus_save` or a curse — the whole *party* is scanned, not
///    the owner (`TeamList.Exists`, `ovr025.cs:187`);
/// 3. a `count` above zero prepends the count and a space.
pub fn display_name(record: &[u8], detect_magic: bool, show_readied: bool) -> String {
    let mut name = String::new();
    if show_readied {
        name.push_str(if rec::item_readied(record) {
            " Yes  "
        } else {
            " No   "
        });
    }
    if detect_magic
        && (rec::item_plus(record) > 0
            || rec::item_plus_save(record) > 0
            || rec::item_is_cursed(record))
    {
        name.push_str("* ");
    }
    let count = rec::item_count(record);
    if count > 0 {
        name.push_str(&count.to_string());
        name.push(' ');
    }
    name.push_str(&generate_name(record, rec::item_hidden_names_flag(record)));
    name
}

/// `gbl.TeamList.Exists(pla => pla.HasAffect(Affects.detect_magic))`
/// (`ovr025.cs:187`).
pub fn party_has_detect_magic(party: &crate::party::Party) -> bool {
    party.members.iter().any(|m| m.has_affect(AFF_DETECT_MAGIC))
}

// ---------------------------------------------------------------------------
// Slot classification
// ---------------------------------------------------------------------------

/// `gbl.ItemDataTable[item.type].item_slot`.
pub fn slot_of(table: &ItemDataTable, record: &[u8]) -> u8 {
    table.get(rec::item_type(record)).item_slot
}

/// `Item.IsScroll()` (`Classes/Item.cs:50-53`) — slot `11..=13`.
pub fn is_scroll(table: &ItemDataTable, record: &[u8]) -> bool {
    (SLOT_SCROLL_FIRST..=SLOT_SCROLL_LAST).contains(&slot_of(table, record))
}

/// `item.HandsCount()` (`Classes/Item.cs:38-41`).
pub fn hands_count(table: &ItemDataTable, record: &[u8]) -> u8 {
    table.get(rec::item_type(record)).hands_count
}

/// The item's weight **as `reclac_player_values` counts it** — multiplied by
/// `count` when the stack is non-empty (`ovr025.cs:350-356`).
pub fn stack_weight(record: &[u8]) -> i32 {
    let w = rec::item_weight(record) as i32;
    let count = rec::item_count(record) as i32;
    if count > 0 {
        w * count
    } else {
        w
    }
}

// ---------------------------------------------------------------------------
// `reclac_player_values` (`ovr025.cs:338-495`)
// ---------------------------------------------------------------------------

/// The `activeItems` array `reclac_player_values` rebuilds from scratch every
/// call (`ovr025.cs:342`, `Classes/Player.cs:196-260`) — indices into
/// [`Character::items`], never the record's own dead pointer bytes (D-SAVE6).
///
/// `slot[0]` is `primaryWeapon`, `slot[1]` the shield slot, `slot[2]` armor
/// (★ the M6b RE correction: `field_159` is the readied ARMOR), and `held_1`/
/// `held_2` are the `slot_9` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActiveItems {
    /// `itemArray[0..=8]` — one index per `ItemSlot.slot_0`..`slot_8`.
    pub slots: [Option<usize>; 9],
    /// `Item_ptr_01` / `Item_ptr_02` — the `slot_9` pair, filled in order.
    pub held_1: Option<usize>,
    pub held_2: Option<usize>,
    /// `arrows` / `quarrels` — the ammo pointers, keyed on item *type*.
    pub arrows: Option<usize>,
    pub quarrels: Option<usize>,
}

impl ActiveItems {
    /// `player.activeItems.primaryWeapon` (`itemArray[0]`).
    pub fn primary_weapon(&self) -> Option<usize> {
        self.slots[0]
    }
    /// The item occupying `slot`, for the `AlreadyUsingX` refusal.
    pub fn at(&self, slot: u8) -> Option<usize> {
        match slot {
            0..=8 => self.slots[slot as usize],
            9 => self.held_1,
            _ => None,
        }
    }
}

/// ★ `reclac_player_values` / `sub_66C20` (`ovr025.cs:338-495`) — the recompute
/// the original runs **after every single items-menu action** (`ovr020.cs:615`),
/// which is what makes a Ready swap show up on the sheet at once.
///
/// Faithfully, in the original's own order:
///
/// 1. rebuild `activeItems` and re-sum `weight` from every item (×`count`) plus
///    all seven coin piles; readied items also add their `HandsCount` to
///    `weaponsHandsUsed` and their weight to the armor-effect subtotal;
/// 2. reset the attack profile to its `*Base` cells and `ac`/`movement`/
///    `hitBonus` to `base_ac`/`base_movement`/`thac0`;
/// 3. `stat_bonus[0] = DexAcBonus`; **bare-handed only**, fold the strength
///    to-hit/damage terms in (a readied weapon's own `CalculateAttackValues`
///    does it instead, and differently);
/// 4. `CalculateAttackValues` for the readied primary weapon;
/// 5. per readied item: `CalcArmorWeightEffect` then `sub_662A6`;
/// 6. `calc_movement`, then fold `stat_bonus[0..=4]` into `ac`, and
///    `ac_behind = (stat_bonus[4] + [2] + [3]) - 2`;
/// 7. `attackLevel` = the fighter skill level, or 1.
///
/// **The one thing left out, named:** the `affect_3 > 0x7F` magic-item riders
/// (`calc_items_effects`, `ovr020.cs:640-777`) are `ready_item`'s business, not
/// this function's, and are the slice's named residual — see
/// [`ready_item`]'s own note. The item's `plus` still reaches AC and to-hit
/// through step 5's `sub_662A6` and step 4, so magical armour, shields and
/// weapons all read correctly; what is missing is only the exotic riders
/// (Ring of Wizardry's doubled slots, the Ioun Stones' stat bonuses).
pub fn reclac_player_values(ch: &mut Character, table: &ItemDataTable, flavor: &dyn Flavor) {
    let mut active = ActiveItems::default();
    let mut weight = 0i32;
    let mut hands = 0u32;

    for (i, item) in ch.items.iter().enumerate() {
        weight += stack_weight(item);
        if !rec::item_readied(item) {
            continue;
        }
        // coab also accumulates `totalItemWeight` here, but its only consumer
        // is the `if (var_8)` block at `ovr025.cs:466-478` and `var_8` is
        // initialised `false` and never assigned — dead in the shipped build.
        let item_type = rec::item_type(item);
        let slot = table.get(item_type).item_slot;
        match slot {
            0..=8 => active.slots[slot as usize] = Some(i),
            9 => {
                if active.held_1.is_some() {
                    if active.held_2.is_none() {
                        active.held_2 = Some(i);
                    }
                } else {
                    active.held_1 = Some(i);
                }
            }
            _ => {}
        }
        if item_type == TYPE_ARROW {
            active.arrows = Some(i);
        }
        if item_type == TYPE_QUARREL {
            active.quarrels = Some(i);
        }
        hands += u32::from(table.get(item_type).hands_count);
    }
    for coin in 0..7 {
        weight += ch.money.get_coin(coin) as i32;
    }
    ch.combat.weapons_hands_used = hands.min(u8::MAX as u32) as u8;
    ch.combat.weight = weight.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

    // `attack1_*` / `attack2_*` reset to their `*Base` cells (`:415-421`). The
    // 8-byte profile blocks are laid out `[_, halfMoves, diceCount, _, diceSize,
    // _, damageBonus, _]` (doc §1.3 / `combat::records`).
    ch.combat.attacks.current[2] = ch.combat.attacks.base[2];
    ch.combat.attacks.current[3] = ch.combat.attacks.base[3];
    ch.combat.attacks.current[4] = ch.combat.attacks.base[4];
    ch.combat.attacks.current[5] = ch.combat.attacks.base[5];
    ch.combat.attacks.current[6] = ch.combat.attacks.base[6];
    ch.combat.attacks.current[7] = ch.combat.attacks.base[7];

    let mut stat_bonus = [0i32; 5];
    let mut armor_plus_seen = false;

    ch.status.save_bonus = 0;
    ch.combat.ac = ch.combat.base_ac;
    ch.combat.movement = ch.combat.base_movement;
    ch.combat.thac0_current = ch.combat.thac0_base as u8;

    stat_bonus[0] = flavor.dex_ac_bonus(ch.stats.dex.original);

    // `field_125` gates every strength term (`ovr025.cs:635`, `:679`), exactly
    // as `combat::records` already reads it.
    let str_hit = if ch.opaque.field_125 != 0 {
        flavor.strength_hit_bonus(
            ch.stats.str_score.original,
            ch.stats.str_exceptional.current,
        )
    } else {
        0
    };
    let str_dam = if ch.opaque.field_125 != 0 {
        flavor.strength_damage_bonus(
            ch.stats.str_score.original,
            ch.stats.str_exceptional.current,
        )
    } else {
        0
    };

    if active.primary_weapon().is_none() {
        ch.combat.thac0_current = add_u8(ch.combat.thac0_current, str_hit);
        ch.combat.attacks.current[6] = add_u8(ch.combat.attacks.current[6], str_dam);
    }

    calculate_attack_values(ch, &active, table, flavor, str_hit, str_dam);

    for i in 0..ch.items.len() {
        if !rec::item_readied(&ch.items[i]) {
            continue;
        }
        calc_armor_weight_effect(ch, i, table);
        sub_662a6(ch, i, table, &mut stat_bonus, &mut armor_plus_seen);
    }
    if armor_plus_seen {
        stat_bonus[3] = 0;
    }

    calc_movement(ch, flavor);

    if stat_bonus[4] < ch.combat.ac as i32 {
        stat_bonus[4] = ch.combat.ac as i32;
    }
    let ac: i32 = stat_bonus.iter().sum();
    ch.combat.ac = ac.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    ch.combat.ac_behind = (stat_bonus[4] + stat_bonus[2] + stat_bonus[3] - 2)
        .clamp(i8::MIN as i32, i8::MAX as i32) as i8;

    let fighter = ch.skill_level(SKILL_FIGHTER);
    ch.combat.attack_level = if fighter > 0 && ch.race > 0 {
        fighter.clamp(0, u8::MAX as i32) as u8
    } else {
        1
    };

    // The readied set is a derived view (§1.7 item 3); keep it in step.
    ch.readied_items = ch
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| rec::item_readied(it))
        .map(|(i, _)| i)
        .collect();
}

/// `SkillType.Fighter` (`Classes/Enums.cs:57-67`).
const SKILL_FIGHTER: usize = 2;

/// Byte arithmetic with the original's own wrap (`hitBonus`/`DamageBonus` are
/// a `byte` and an `sbyte` the binary adds into without checking).
fn add_u8(base: u8, delta: i32) -> u8 {
    (base as i32 + delta).rem_euclid(256) as u8
}

/// `CalculateAttackValues` / `sub_66023` (`ovr025.cs:10-64`) — the readied
/// weapon's own to-hit and damage, which **replace** the bare-handed terms
/// rather than adding to them.
fn calculate_attack_values(
    ch: &mut Character,
    active: &ActiveItems,
    table: &ItemDataTable,
    flavor: &dyn Flavor,
    str_hit: i32,
    str_dam: i32,
) {
    let Some(idx) = active.primary_weapon() else {
        return;
    };
    let record = ch.items[idx].clone();
    let item_type = rec::item_type(&record);
    let data = table.get(item_type);

    ch.combat.thac0_current = ch.combat.thac0_base as u8;
    if data.flags & gbx_formats::items::flags::FLAG_02 != 0 {
        ch.combat.thac0_current = add_u8(
            ch.combat.thac0_current,
            flavor.dex_reaction_bonus(ch.stats.dex.original),
        );
    }
    let mut damage_bonus = data.bonus_normal as i32;
    if data.is_melee() {
        ch.combat.thac0_current = add_u8(ch.combat.thac0_current, str_hit);
        damage_bonus += str_dam;
    }

    let mut bonus = rec::item_plus(&record) as i32;
    if data.is_quarrels() {
        if let Some(q) = active.quarrels {
            bonus += rec::item_plus(&ch.items[q]) as i32;
        }
    }
    if data.is_arrows() {
        if let Some(a) = active.arrows {
            bonus += rec::item_plus(&ch.items[a]) as i32;
        }
    }
    damage_bonus += bonus;

    // The elf rider (`ovr025.cs:47-57`): +1 to **hit only** — `bonus` is
    // incremented after it has already been folded into the damage total.
    const RACE_ELF: u8 = 2;
    if ch.race == RACE_ELF
        && matches!(
            item_type,
            41 | 42 | 43 | 44 | 37 | 36 // composite long/short bow, long/short bow, short/long sword
        )
    {
        bonus += 1;
    }
    ch.combat.thac0_current = add_u8(ch.combat.thac0_current, bonus);
    ch.combat.attacks.current[6] = (damage_bonus.rem_euclid(256)) as u8;
    ch.combat.attacks.current[2] = data.dice_count_normal;
    ch.combat.attacks.current[4] = data.dice_size_normal;
}

/// `CalcArmorWeightEffect` / `sub_6621E` (`ovr025.cs:67-89`): only an **armor**
/// slot item speaks, and it sets movement outright by its own weight band —
/// then adds 3 back to any band at or under 9, which is why the 151..=399 band
/// yields 12 and the heavy band 9.
fn calc_armor_weight_effect(ch: &mut Character, idx: usize, table: &ItemDataTable) {
    if table.get(rec::item_type(&ch.items[idx])).item_slot != SLOT_ARMOR {
        return;
    }
    let weight = rec::item_weight(&ch.items[idx]);
    ch.combat.movement = match weight {
        0..=150 => ch.combat.base_movement,
        151..=399 => 9,
        _ => 6,
    };
    if ch.combat.movement != 0 && ch.combat.movement <= 9 {
        ch.combat.movement += 3;
    }
}

/// `sub_662A6` (`ovr025.cs:92-134`) — the readied item's contribution to the
/// five AC/save accumulators, gated on `ItemData.field_6`'s high bit.
///
/// The four arms, in the original's order: a `slot_1` (shield) item writes
/// `bonus[1]` outright; `var_1 == 0` means a plain protective item, going to
/// `bonus[3]` (best-of, for `slot_9`) or `bonus[2]` (cumulative) and adding its
/// `plus_save` to `field_186`; anything else competes for `bonus[4]` as a
/// base-AC replacement, and a *magical* armour that wins also sets the flag
/// that later zeroes `bonus[3]`.
fn sub_662a6(
    ch: &mut Character,
    idx: usize,
    table: &ItemDataTable,
    bonus: &mut [i32; 5],
    armor_plus_seen: &mut bool,
) {
    let record = &ch.items[idx];
    let data = table.get(rec::item_type(record));
    let mut var_1 = data.field_6;
    if var_1 <= 0x7F {
        return;
    }
    var_1 &= 0x7F;
    let slot = data.item_slot;
    let plus = rec::item_plus(record) as i32;

    if slot == SLOT_1 {
        bonus[1] = plus + var_1 as i32;
        return;
    }
    if var_1 == 0 {
        if slot == SLOT_9 {
            if plus > bonus[3] {
                bonus[3] = plus;
            }
        } else {
            bonus[2] += plus;
        }
        ch.status.save_bonus = ch
            .status
            .save_bonus
            .saturating_add(rec::item_plus_save(record) as i8);
        return;
    }
    if plus + var_1 as i32 > bonus[4] {
        bonus[4] = plus + var_1 as i32;
        if plus > 0 && slot == SLOT_ARMOR {
            *armor_plus_seen = true;
        }
    }
}

/// `calc_movement` / `sub_663C4` (`ovr025.cs:137-168`): overload past
/// `max_encumberance` takes movement away in three steps, and the result only
/// ever *lowers* the current rate.
fn calc_movement(ch: &mut Character, flavor: &dyn Flavor) {
    let max = flavor.max_encumbrance(
        ch.stats.str_score.original,
        ch.stats.str_exceptional.current,
    );
    let overload = (ch.combat.weight as i32 - max).max(0);
    let moves = match overload {
        0..=0x200 => ch.combat.movement as i32,
        0x201..=0x300 => 9,
        0x301..=0x400 => 6,
        _ => 3,
    };
    if moves < ch.combat.movement as i32 {
        ch.combat.movement = moves as u8;
    }
}

/// `canCarry` (`ovr020.cs:1323-1346`) — **true means "cannot"**, which is the
/// original's own inverted sense (`if (canCarry(...) == true) "Overloaded"`).
/// The inventory cap and the `max_encumberance + 1500` weight ceiling.
pub fn cannot_carry(
    ch: &mut Character,
    record: &[u8],
    table: &ItemDataTable,
    flavor: &dyn Flavor,
) -> bool {
    reclac_player_values(ch, table, flavor);
    if ch.items.len() >= MAX_ITEMS {
        return true;
    }
    let ceiling = flavor.max_encumbrance(
        ch.stats.str_score.original,
        ch.stats.str_exceptional.current,
    ) + 1500;
    (ch.combat.weight as i32 + stack_weight(record)) > ceiling
}

// ---------------------------------------------------------------------------
// The verbs
// ---------------------------------------------------------------------------

/// Why `ready_item` refused (`ready_Item`'s four `Weld` outcomes plus the
/// cursed-removal refusal, `ovr020.cs:787-889`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyRefusal {
    /// `"It's Cursed"` — a cursed item cannot be taken off (`:798`).
    Cursed,
    /// `"Wrong Class"` (`:872`).
    WrongClass,
    /// `"already using {name}"` (`:877`).
    AlreadyUsing(String),
    /// `"Your hands are full!"` (`:884`).
    HandsFull,
}

/// What `ready_item` did, for the caller's status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyOutcome {
    Readied,
    Unreadied,
    /// The **named residual**: the item carries `affect_3 > 0x7F`, so the
    /// original would also have run `calc_items_effects` (`ovr020.cs:640-777`)
    /// — the stat/spell-slot riders. The ready/unready itself happened and
    /// `reclac_player_values` ran; only the rider is missing.
    RiderDeferred {
        readied: bool,
        masked_affect: u8,
    },
}

/// ★ `ready_Item` (`ovr020.cs:787-889`), transcribed **including its own
/// override**.
///
/// Taking an item *off* is two lines: a cursed item refuses, anything else
/// clears the flag (plus `calc_items_effects(false, …)` for a magic item).
///
/// Putting one *on* computes a `Weld` verdict from four tests — hands, the
/// occupied slot, the arrows/quarrels pointers, and the class mask — and then
/// **throws the verdict away**: `ovr020.cs:860` is a bare `result = Weld.Ok;`
/// sitting between the last test and the `switch`, so every one of those
/// refusals is dead code in the shipped build and *anything* can be readied by
/// *anyone*. That is not a coab transcription slip: coab transliterates the
/// binary statement-for-statement, the assignment has no condition attached,
/// and the three refusal strings it orphans are the ones players never see.
/// Reproduced as written — [`ReadyRefusal`] exists for the cursed case, which
/// is on the live path, and the computed verdict is returned to the caller in
/// `refused_by_dead_code` so a test can pin the dead branch without the screen
/// ever showing it.
pub fn ready_item(
    ch: &mut Character,
    idx: usize,
    table: &ItemDataTable,
    flavor: &dyn Flavor,
) -> Result<ReadyOutcome, ReadyRefusal> {
    let Some(record) = ch.items.get(idx) else {
        return Err(ReadyRefusal::WrongClass);
    };
    let magic_item = rec::item_affect(record, 3) > 0x7F;
    let masked_affect = rec::item_affect(record, 3) & 0x7F;

    if rec::item_readied(record) {
        if rec::item_is_cursed(record) {
            return Err(ReadyRefusal::Cursed);
        }
        rec::set_item_readied(&mut ch.items[idx], false);
        reclac_player_values(ch, table, flavor);
        return Ok(if magic_item {
            ReadyOutcome::RiderDeferred {
                readied: false,
                masked_affect,
            }
        } else {
            ReadyOutcome::Unreadied
        });
    }

    // The verdict the original computes and then discards (`:814-860`).
    let _ = weld_verdict(ch, idx, table);
    rec::set_item_readied(&mut ch.items[idx], true);
    reclac_player_values(ch, table, flavor);
    Ok(if magic_item {
        ReadyOutcome::RiderDeferred {
            readied: true,
            masked_affect,
        }
    } else {
        ReadyOutcome::Readied
    })
}

/// The `Weld` verdict `ready_Item` computes at `ovr020.cs:814-859` before
/// `:860` unconditionally overwrites it with `Ok`. Exposed so the dead branch
/// is *pinned* rather than merely asserted about in a comment.
pub fn weld_verdict(ch: &Character, idx: usize, table: &ItemDataTable) -> Option<ReadyRefusal> {
    let active = active_items(ch, table);
    let record = &ch.items[idx];
    let item_type = rec::item_type(record);
    let data = table.get(item_type);
    let mut result = None;

    if u32::from(ch.combat.weapons_hands_used) + u32::from(data.hands_count) > 2 {
        result = Some(ReadyRefusal::HandsFull);
    }
    let mut slot = data.item_slot;
    if (slot <= 8 && active.slots[slot as usize].is_some())
        || (slot == SLOT_9 && active.held_2.is_some())
    {
        result = Some(ReadyRefusal::AlreadyUsing(String::new()));
    }
    if item_type == TYPE_ARROW && active.arrows.is_some() {
        result = Some(ReadyRefusal::AlreadyUsing(String::new()));
        slot = 11;
    }
    if item_type == TYPE_QUARREL && active.quarrels.is_some() {
        result = Some(ReadyRefusal::AlreadyUsing(String::new()));
        slot = SLOT_CLERIC_SCROLL;
    }
    if ch.skills.class_flags & data.class_flags == 0 {
        result = Some(ReadyRefusal::WrongClass);
    }
    // Name the occupant, the way the refusal string would have.
    if let Some(ReadyRefusal::AlreadyUsing(_)) = &result {
        let occupant = active
            .at(slot)
            .map(|i| display_name(&ch.items[i], false, false))
            .unwrap_or_default();
        result = Some(ReadyRefusal::AlreadyUsing(occupant));
    }
    result
}

/// [`reclac_player_values`]' first pass alone, for callers that only need the
/// slot map (`weld_verdict`, the combat kit builder).
pub fn active_items(ch: &Character, table: &ItemDataTable) -> ActiveItems {
    let mut active = ActiveItems::default();
    for (i, item) in ch.items.iter().enumerate() {
        if !rec::item_readied(item) {
            continue;
        }
        let item_type = rec::item_type(item);
        let slot = table.get(item_type).item_slot;
        match slot {
            0..=8 => active.slots[slot as usize] = Some(i),
            9 => {
                if active.held_1.is_some() {
                    if active.held_2.is_none() {
                        active.held_2 = Some(i);
                    }
                } else {
                    active.held_1 = Some(i);
                }
            }
            _ => {}
        }
        if item_type == TYPE_ARROW {
            active.arrows = Some(i);
        }
        if item_type == TYPE_QUARREL {
            active.quarrels = Some(i);
        }
    }
    active
}

/// `CanSellDropTradeItem` / `sub_54EC1` (`ovr020.cs:342-375`)'s three answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisposeCheck {
    /// Go ahead.
    Allowed,
    /// `"Must be unreadied"` — a readied item is never disposable.
    MustBeUnreadied,
    /// A scroll with a spell staged for scribing: the original names the owner
    /// and asks `"is it Okay to lose it? "` before allowing it.
    ConfirmScribedScroll,
}

/// `CanSellDropTradeItem` (`ovr020.cs:342-375`).
pub fn dispose_check(table: &ItemDataTable, record: &[u8]) -> DisposeCheck {
    if rec::item_readied(record) {
        return DisposeCheck::MustBeUnreadied;
    }
    if !is_scroll(table, record) {
        return DisposeCheck::Allowed;
    }
    if (1..=3).any(|i| rec::item_affect(record, i) > 0x7F) {
        return DisposeCheck::ConfirmScribedScroll;
    }
    DisposeCheck::Allowed
}

/// `halve_items` (`ovr020.cs:916-936`): split the stack, the **larger** half
/// staying on the original row, the new one unreadied. `count / 2 == 0`
/// refuses with `"Can't halve that"`.
pub fn halve_items(ch: &mut Character, idx: usize) -> bool {
    let count = rec::item_count(&ch.items[idx]);
    let half = count / 2;
    if half == 0 {
        return false;
    }
    let mut clone = ch.items[idx].clone();
    rec::set_item_count(&mut ch.items[idx], count - half);
    rec::set_item_count(&mut clone, half);
    rec::set_item_readied(&mut clone, false);
    ch.items.push(clone);
    true
}

/// `join_items` / `sub_56285` (`ovr020.cs:939-977`): merge every *identical*
/// stack into this one.
///
/// "Identical" is the original's own thirteen-field compare — and it includes
/// the clause `affect_1 < 2`, so two items agreeing on everything else still
/// refuse to merge once either carries more than one charge. Overflow past 255
/// is handled the original's way: this stack saturates at 255, the donor keeps
/// `255 - (a + b)` (a **negative** remainder in the original's own arithmetic —
/// reproduced with the same wrap), and the walk continues from the donor.
pub fn join_items(ch: &mut Character, idx: usize) -> usize {
    let mut target = idx;
    let mut merged = 0usize;
    loop {
        let Some(donor) = ch
            .items
            .iter()
            .enumerate()
            .position(|(i, other)| i != target && joinable(&ch.items[target], other))
        else {
            return merged;
        };
        let a = rec::item_count(&ch.items[target]) as i32;
        let b = rec::item_count(&ch.items[donor]) as i32;
        if a + b <= 255 {
            rec::set_item_count(&mut ch.items[target], (a + b) as u8);
            ch.items.remove(donor);
            if donor < target {
                target -= 1;
            }
            merged += 1;
        } else {
            let temp = (255 - (a + b)).rem_euclid(256) as u8;
            rec::set_item_count(&mut ch.items[target], 255);
            rec::set_item_count(&mut ch.items[donor], temp);
            target = donor;
            merged += 1;
        }
    }
}

/// `join_items`' `FindAll` predicate (`ovr020.cs:943-959`), field for field.
fn joinable(a: &[u8], b: &[u8]) -> bool {
    rec::item_count(b) > 0
        && (1..=3).all(|i| rec::item_namenum(a, i) == rec::item_namenum(b, i))
        && rec::item_type(a) == rec::item_type(b)
        && rec::item_plus(a) == rec::item_plus(b)
        && rec::item_plus_save(a) == rec::item_plus_save(b)
        && rec::item_is_cursed(a) == rec::item_is_cursed(b)
        && rec::item_weight(a) == rec::item_weight(b)
        && rec::item_affect(a, 1) == rec::item_affect(b, 1)
        && rec::item_affect(a, 1) < 2
        && rec::item_affect(a, 2) == rec::item_affect(b, 2)
        && rec::item_affect(a, 3) == rec::item_affect(b, 3)
}

/// `lose_item` (`ovr025.cs:766-772`) — removing an item shifts every later
/// index, so the readied set is rebuilt rather than patched.
pub fn lose_item(ch: &mut Character, idx: usize) -> Option<Vec<u8>> {
    if idx >= ch.items.len() {
        return None;
    }
    let gone = ch.items.remove(idx);
    ch.readied_items = ch
        .items
        .iter()
        .enumerate()
        .filter(|(_, it)| rec::item_readied(it))
        .map(|(i, _)| i)
        .collect();
    Some(gone)
}

// ---------------------------------------------------------------------------
// The verb bar
// ---------------------------------------------------------------------------

/// Which words `PlayerItemsMenu` puts on its command bar this iteration
/// (`ovr020.cs:449-494`), and under what condition. `Ready`, `Drop` and `Join`
/// are unconditional; the rest each carry the original's own test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemsMenuWords {
    /// `Cheats.view_item_stats` — off in a shipped build, so never shown.
    pub view: bool,
    /// `in_combat && area.field_1CA == 0 && (state ∈ {Camping, Wilderness,
    /// Dungeon, Combat} || actions.can_use)`.
    pub use_item: bool,
    /// The NPC-control test, and never in combat.
    pub trade: bool,
    /// `items.Count < MaxItems`.
    pub halve: bool,
    /// Shop only.
    pub sell: bool,
    /// Shop only.
    pub id: bool,
}

/// `Control.NPC_Base` (`Classes/Control.cs:322`) — the threshold above which a
/// roster member is an NPC whose gear the party may not take.
pub const NPC_BASE: u8 = 0x80;

/// `PlayerItemsMenu`'s bar conditions (`ovr020.cs:449-494`), evaluated for one
/// character out of combat on a normal (non-shop) screen.
///
/// `area_bans_items` is `gbl.area_ptr.field_1CA != 0` — the per-area "no item
/// use here" flag the same `field_1CA` gates the combat menu's Cast word with.
pub fn items_menu_words(ch: &Character, area_bans_items: bool, in_shop: bool) -> ItemsMenuWords {
    let controllable =
        ch.control_morale < NPC_BASE || !ch.status.in_combat || ch.status.health_status == 1;
    ItemsMenuWords {
        // `Cheats.view_item_stats` (`ovr020.cs:451`) is a debug build flag.
        view: false,
        // Out of combat the game state is always one of the four the original
        // lists (Camping / WildernessMap / DungeonMap / Combat), so the whole
        // parenthesis is true and only `in_combat` and the area ban decide.
        use_item: ch.status.in_combat && !area_bans_items,
        trade: controllable,
        halve: ch.items.len() < MAX_ITEMS,
        sell: in_shop && controllable,
        id: in_shop,
    }
}

impl ItemsMenuWords {
    /// The bar text, in the original's own word order (`ovr020.cs:449-494`).
    pub fn text(&self) -> String {
        let mut t = String::from("Ready");
        if self.view {
            t.push_str(" View");
        }
        if self.use_item {
            t.push_str(" Use");
        }
        if self.trade {
            t.push_str(" Trade");
        }
        t.push_str(" Drop");
        if self.halve {
            t.push_str(" Halve");
        }
        t.push_str(" Join");
        if self.sell {
            t.push_str(" Sell");
        }
        if self.id {
            t.push_str(" Id");
        }
        t
    }
}

#[cfg(test)]
mod tests;
