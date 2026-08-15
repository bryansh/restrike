//! ★ **Character creation** (`createPlayer`, `ovr018.cs:325-891`) — roll-credits
//! slice 9c's pure half: everything the flow does to the *record*, with the
//! screen ([`crate::create_screen`]) supplying only the four picks, the name and
//! the icon.
//!
//! Derived by reading coab for behavior (D11, never copied). The sites:
//!
//! - `ovr018.cs:325-891` — `createPlayer` itself: the identity constants, the
//!   race/sex/class/alignment pickers, the starting age roll, the reroll loop,
//!   the name prompt, the icon editor and the closing `Save <name>?`.
//! - `ovr017.cs:461-473` — `SilentTrainPlayer`: `training_class_mask = 0xFF`
//!   and `train_player()` until it says stop, which is how a brand-new
//!   character reaches level 6 on its 25,000 starting XP.
//! - `ovr026.cs:55-181,184-264,334-375` — `sub_6A00F` / `ReclacClassBonuses` /
//!   `reclac_saving_throws`: the derived-field recompute every level-up runs.
//!
//! ★ **The one thing the brief expected that the original does not do: a
//! created character is NOT added to the party.** `startGameMenu`'s `'C'` arm
//! is a bare `createPlayer()` (`ovr018.cs:161-166`), and `createPlayer` ends by
//! restoring `gbl.SelectedPlayer` and offering `SavePlayer` — there is no
//! `TeamList.Add` anywhere in it. Creation writes a character *file*; `Add
//! Character to Party` is what puts one in the party. That is the shipped Gold
//! Box loop, and it is why the bundle's own `SAVE/` directory has `.GUY` files
//! sitting next to the save games.

use crate::party::{AbilityScorePair, Character};
use crate::rng::EngineRng;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::adnd1::{constants, creation_limits, progression};
use gbx_rules::flavor::{AbilityStat, ClassLevel, Flavor, Roller};
use gbx_rules::pack::RuleSet;

/// A [`Roller`] over the engine PRNG — `roll_dice`'s `Random(size) + 1` per
/// die (`ovr024.cs:586-598`).
pub struct EngineRoller<'a>(pub &'a mut EngineRng);

impl Roller for EngineRoller<'_> {
    fn roll(&mut self, size: u32, count: u32) -> u32 {
        (0..count)
            .map(|_| self.0.random(size as u16) as u32 + 1)
            .sum()
    }
}

// --- The pickers' own tables (ovr018.cs:352-618) ---

/// ★ The **six** races `createPlayer`'s first picker offers
/// (`ovr018.cs:355-360`): `raceString[1..=5]` and then `raceString[7]`.
///
/// **Half-Orc (race 6) is missing, deliberately.** The list simply skips it,
/// and `:378-381`'s `if (index == 6) index++;` is what turns the sixth *menu
/// row* back into race id 7. A half-orc can only enter a CotAB party by
/// import (`RaceClasses[6]` exists for exactly that reason) — never by
/// creation.
pub const CREATABLE_RACES: [u8; 6] = [1, 2, 3, 4, 5, 7];

/// `raceString` (`ovr020.cs:20-21`).
pub const RACE_NAMES: [&str; 8] = [
    "Monster", "Dwarf", "Elf", "Gnome", "Half-Elf", "Halfling", "Half-Orc", "Human",
];
/// `sexString` (`ovr020.cs:19`).
pub const SEX_NAMES: [&str; 2] = ["Male", "Female"];
/// `classString` (`ovr020.cs:27-32`) — index is the raw combo `ClassId`.
pub const CLASS_NAMES: [&str; 17] = [
    "Cleric",
    "Druid",
    "Fighter",
    "Paladin",
    "Ranger",
    "Magic-User",
    "Thief",
    "Monk",
    "Cleric/Fighter",
    "Cleric/Fighter/Magic-User",
    "Cleric/Ranger",
    "Cleric/Magic-User",
    "Cleric/Thief",
    "Fighter/Magic-User",
    "Fighter/Thief",
    "Fighter/Magic-User/Thief",
    "Magic-User/Thief",
];
/// `alignmentString` (`ovr020.cs:23-25`).
pub const ALIGNMENT_NAMES: [&str; 9] = [
    "Lawful Good",
    "Lawful Neutral",
    "Lawful Evil",
    "Neutral Good",
    "True Neutral",
    "Neutral Evil",
    "Chaotic Good",
    "Chaotic Neutral",
    "Chaotic Evil",
];

/// `gbl.default_icon_colours` (`Classes/Gbl.cs:274`) — the six palette slots
/// the combat-icon recolour remaps.
pub const DEFAULT_ICON_COLOURS: [u8; 6] = [1, 2, 3, 4, 6, 7];

/// `player.icon_colours[i] = ((default + 8) << 4) + default` (`ovr018.cs:341`):
/// the low nibble is the "1st colour", the high nibble the "2nd", and a fresh
/// character starts with the palette's own pair. Pinned against every real
/// `.GUY`/`CHRDAT` record in the bundle, which all read `91 a2 b3 c4 e6 f7`.
pub fn default_icon_colours() -> [u8; 6] {
    DEFAULT_ICON_COLOURS.map(|c| ((c + 8) << 4) + c)
}

/// The base `ClassId`s a combo id covers — `createPlayer`'s own
/// `if/else if` ladder (`ovr018.cs:487-562`) read as a table. Index is the raw
/// combo id 0..=16.
pub fn component_classes(class_id: u8) -> &'static [usize] {
    match class_id {
        0..=7 => match class_id {
            0 => &[0],
            1 => &[1],
            2 => &[2],
            3 => &[3],
            4 => &[4],
            5 => &[5],
            6 => &[6],
            _ => &[7],
        },
        8 => &[0, 2],     // mc_c_f
        9 => &[0, 2, 5],  // mc_c_f_m
        10 => &[0, 4],    // mc_c_r
        11 => &[0, 5],    // mc_c_mu
        12 => &[0, 6],    // mc_c_t
        13 => &[2, 5],    // mc_f_mu
        14 => &[2, 6],    // mc_f_t
        15 => &[2, 5, 6], // mc_f_mu_t
        16 => &[5, 6],    // mc_mu_t
        _ => &[],
    }
}

/// The classes `race` may take at creation, as raw combo `ClassId`s
/// (`RaceClasses[race]`, `ovr018.cs:455`).
pub fn class_choices(rules: &RuleSet, race: u8) -> Vec<u8> {
    creation_limits::race_classes(rules, race as usize)
}

/// The alignments `class_id` may take (`class_alignments`, `ovr018.cs:590-597`).
pub fn alignment_choices(rules: &RuleSet, class_id: u8) -> Vec<u8> {
    gbx_rules::adnd1::creation::allowed_alignments(rules, class_id as usize)
}

// --- The record itself ---

/// An all-zero [`Character`] — `new Player()` (`ovr018.cs:337`), which in the
/// original is a freshly allocated, zero-filled `charStruct`.
fn blank_character() -> Character {
    let bytes = [0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
    let record = gbx_formats::save_orig::decode_char_record(&bytes)
        .expect("a zeroed buffer is exactly CHAR_RECORD_SIZE");
    crate::party::character_from_record(&record, Vec::new(), Vec::new())
}

/// `createPlayer`'s four picks, in the order its pickers ask for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Picks {
    /// A [`CREATABLE_RACES`] value (`Race` enum id).
    pub race: u8,
    /// 0 male, 1 female (`ovr018.cs:450`).
    pub sex: u8,
    /// The raw combo `ClassId` (`ovr018.cs:484`).
    pub class_id: u8,
    /// The raw alignment id (`ovr018.cs:618`).
    pub alignment: u8,
}

/// ★ `createPlayer`'s pre-reroll half (`ovr018.cs:337-651`): the identity
/// constants, the racial and class grants, the derived-field recompute, and
/// the starting-age roll.
///
/// Two PRNG draws happen here, in this order: `Random(256)` for `mod_id`
/// (`:349`, before the very first picker) and — for a single-class character
/// only — the starting-age dice (`:626`). A multi-class character's age is an
/// **un-rolled** `base + count * size` ceiling (`:638-650`), so it consumes no
/// draw at all.
pub fn begin(rules: &RuleSet, rng: &mut EngineRng, picks: Picks) -> Character {
    let flavor = Adnd1::new(rules);
    let mut ch = blank_character();

    // `ovr018.cs:339-350` — the constants a fresh record starts with.
    ch.icon.colours = default_icon_colours();
    ch.combat.base_ac = 50;
    ch.combat.thac0_base = 40; // recomputed at `:570-585`; carried faithfully
    ch.status.health_status = 0; // Status.okey
    ch.status.in_combat = true;
    ch.opaque.field_de = 1;
    ch.monster_index = rng.random(256) as u8; // `mod_id = Random(256)`
    ch.icon.icon_id = 0x0A;

    // `:385-421` — race: icon size, then the racial affects.
    ch.race = picks.race;
    ch.icon.icon_size = if matches!(picks.race, 1 | 3 | 5) {
        1
    } else {
        2
    };
    for kind in flavor.racial_traits(picks.race as usize) {
        push_permanent_affect(&mut ch, kind as u8);
    }

    ch.sex = picks.sex;
    ch.class_id = picks.class_id;

    // `:483-562` — starting XP, the per-class levels, and the two class
    // affects (`protection_from_evil` for a paladin, `ranger_vs_giant` for a
    // ranger — both granted at creation, not at first fight).
    let components = component_classes(picks.class_id);
    let classes: Vec<ClassLevel> = components
        .iter()
        .map(|&class| ClassLevel { class, level: 1 })
        .collect();
    ch.exp = flavor.starting_experience(&classes) as i32;
    ch.hit_dice = 1;
    for &class in components {
        ch.class_level[class] = 1;
    }
    if components.contains(&3) {
        ch.status.paladin_cures_left = 1;
        push_permanent_affect(&mut ch, AFFECT_PROTECTION_FROM_EVIL);
    }
    if components.contains(&4) {
        push_permanent_affect(&mut ch, AFFECT_RANGER_VS_GIANT);
    }

    // `:564-587` — thief skills, `classFlags`, THAC0 and the saving throws.
    if ch.class_level[6] > 0 {
        recalc_thief_skills(&mut ch, rules);
    }
    recalc_class_flags_and_thac0(&mut ch, rules);
    recalc_saving_throws(&mut ch, rules);

    ch.alignment = picks.alignment;

    // `:622-651` — the starting age.
    ch.age = {
        let mut roller = EngineRoller(rng);
        flavor.starting_age(picks.race as usize, &classes, &mut roller) as i16
    };

    ch
}

/// `Affects.protection_from_evil` (`Classes/Affect.cs`) — the paladin's
/// creation grant (`ovr018.cs:499`).
const AFFECT_PROTECTION_FROM_EVIL: u8 = 0x21;
/// `Affects.ranger_vs_giant` — the ranger's (`ovr018.cs:504,523`).
const AFFECT_RANGER_VS_GIANT: u8 = 0x13;

/// `add_affect(false, 0xff, 0, kind, player)` (`ovr024.cs`) — a permanent
/// affect: duration `0xff` means "never times out".
fn push_permanent_affect(ch: &mut Character, kind: u8) {
    let mut record = [0u8; gbx_formats::save_orig::AFFECT_RECORD_SIZE];
    record[0] = kind;
    record[3] = 0xFF; // duration
    ch.affects.push(record.to_vec());
}

/// ★ `createPlayer`'s reroll body (`ovr018.cs:657-865`) — everything between
/// `do {` and `while (input_key != 'N')`, so calling it again *is* answering
/// `Reroll stats? Y`.
///
/// In the original's own order: reset every active class to level 1 (a reroll
/// after [`silent_train`] has to undo it), zero the seven stats, roll the six
/// interleaved (`Flavor::roll_ability_scores`, FD-30), age/race-sex/class-clamp
/// each one, roll exceptional strength for a warrior who hit 18, floor a
/// multi-class cleric's WIS at 13, reset the attack profile, seed the caster
/// spell books, take the 300 platinum, roll and CON-adjust the hit points, and
/// then [`silent_train`] up to the starting XP.
pub fn reroll(ch: &mut Character, rules: &RuleSet, rng: &mut EngineRng) {
    let flavor = Adnd1::new(rules);

    // `:659-665` — back to level 1 before anything is recomputed.
    for level in ch.class_level.iter_mut() {
        if *level > 0 {
            *level = 1;
        }
    }
    ch.hit_dice = 1;

    // `:667-683` — the six stats, interleaved, best of six 3d6+1 each.
    let rolled = {
        let mut roller = EngineRoller(rng);
        flavor.roll_ability_scores(&mut roller)
    };
    ch.stats.str_exceptional = AbilityScorePair::default();

    let race = ch.race as usize;
    let sex = ch.sex as usize;
    let class = ch.class_id as usize;
    let age_deltas = flavor.age_effect_deltas(race, ch.age.max(0) as u16);
    let stats = [
        AbilityStat::Str,
        AbilityStat::Int,
        AbilityStat::Wis,
        AbilityStat::Dex,
        AbilityStat::Con,
        AbilityStat::Cha,
    ];
    let mut scores = [0u8; 6];
    for (i, stat) in stats.into_iter().enumerate() {
        // `AgeEffects` then `EnforceRaceSexLimits` then `EnforceClassLimits`
        // (`:692-744`, `Classes/Player.cs:47-75`).
        let aged = (rolled[i] as i32 + age_deltas[i]).clamp(0, 255) as u8;
        scores[i] = flavor.clamp_stat_for_creation(stat, race, sex, class, aged);
    }
    // `:697-706` — exceptional strength, a single d100, only for a warrior
    // who landed on exactly 18 (and only AFTER the clamps).
    let components = component_classes(ch.class_id);
    let class_levels: Vec<ClassLevel> = components
        .iter()
        .map(|&c| ClassLevel { class: c, level: 1 })
        .collect();
    if scores[0] == 18 && flavor.exceptional_strength_eligible(&class_levels) {
        let percentile = {
            let mut roller = EngineRoller(rng);
            flavor.roll_exceptional_strength(&mut roller)
        };
        let (min, max) = creation_limits::race_sex_min_max(
            rules,
            creation_limits::Stat::StrPercentile,
            race,
            sex,
        );
        let clamped = percentile.clamp(min, max);
        ch.stats.str_exceptional = AbilityScorePair {
            current: clamped,
            original: clamped,
        };
    }
    // `:720-725` — a multi-class cleric's WIS floor. Note it is applied to
    // `full` AFTER the class clamp, and only for combo ids `mc_c_f`..`mc_c_t`
    // (8..=12), which is why it is not just another `class_stats_min` row.
    if scores[2] < 13 && (8..=12).contains(&ch.class_id) {
        scores[2] = 13;
    }
    for (i, v) in scores.into_iter().enumerate() {
        let pair = AbilityScorePair {
            current: v,
            original: v,
        };
        match i {
            0 => ch.stats.str_score = pair,
            1 => ch.stats.int = pair,
            2 => ch.stats.wis = pair,
            3 => ch.stats.dex = pair,
            4 => ch.stats.con = pair,
            _ => ch.stats.cha = pair,
        }
    }

    // `:751-755` — the bare-handed attack profile. The 8-byte block is
    // `[left, halfMoves, diceCount, _, diceSize, _, damageBonus, _]`.
    ch.combat.attacks.base = [0; 8];
    ch.combat.attacks.base[0] = 2; // attacksCount
    ch.combat.attacks.base[2] = 1; // attack1_DiceCountBase
    ch.combat.attacks.base[4] = 2; // attack1_DiceSizeBase (1d2 fists)
    ch.opaque.field_125 = 1;
    ch.combat.base_movement = 12;

    // `:758-805` — spell state. Cleric gets one level-1 slot and every
    // level-1 cleric spell; magic-user gets one slot and its four openers.
    ch.magic.cast_count = [[0; 5]; 3];
    ch.magic.spell_book = vec![0u8; 100];
    ch.magic.spell_list = vec![0u8; crate::magic::SPELL_LIST_SIZE];
    if ch.class_level[0] > 0 {
        ch.magic.cast_count[0][0] = 1;
        for id in 1..crate::magic::SPELL_TABLE.len() as u8 {
            let row = crate::magic::SPELL_TABLE[id as usize];
            if row.class_ == crate::magic::SpellClass::Cleric && row.level == 1 {
                crate::magic::learn_spell(ch, id);
            }
        }
    }
    if ch.class_level[5] > 0 {
        ch.magic.cast_count[2][0] = 1;
        for id in MU_STARTING_SPELLS {
            crate::magic::learn_spell(ch, id);
        }
    }

    // `:807` — the starting purse.
    ch.money = crate::party::Money::default();
    ch.money.platinum = flavor.starting_money() as i16;

    // `:808-830` — hit points: roll every active class, apply the CON
    // adjustment, average across classes.
    let hp = {
        let mut roller = EngineRoller(rng);
        flavor.hp_gain_at_creation(&class_levels, ch.stats.con.current, &mut roller)
    };
    ch.hit_point_max = hp.max.min(255) as u8;
    ch.hit_point_current = ch.hit_point_max;
    ch.hit_point_rolled = hp.rolled.min(255) as u8;

    // `:831-835` — `SilentTrainPlayer` with the trainer mask forced open, then
    // the town's real mask restored (the menu's Train flag must not change
    // because somebody rolled a character).
    silent_train(ch, rules, rng);
}

/// The four spells a brand-new magic-user starts with (`ovr018.cs:797-800`):
/// detect magic, read magic, enlarge, sleep.
const MU_STARTING_SPELLS: [u8; 4] = [0x0B, 0x12, 0x0C, 0x15];

// --- SilentTrainPlayer (ovr017.cs:461-473) ---

/// ★ `SilentTrainPlayer` (`ovr017.cs:461-473`): `training_class_mask = 0xFF`,
/// then `train_player()` over and over until it reports `can_train_no_more`.
///
/// This is what turns a level-1 record with 25,000 XP into the level-6 thief
/// (or level-4/4 fighter/magic-user) the original's own `.GUY` files hold. The
/// loop is bounded here — the original's exit condition is a global the train
/// step sets, and a rules pack with a broken XP table would spin forever.
pub fn silent_train(ch: &mut Character, rules: &RuleSet, rng: &mut EngineRng) {
    for _ in 0..64 {
        if silent_train_step(ch, rules, rng).is_none() {
            break;
        }
    }
    reclac_class_bonuses(ch, rules);
}

/// One `train_player()` pass with `silent_training == true` — no fee, no
/// prompt, no spell menu (`ovr018.cs:2189-2482`). `None` once no class can
/// advance, which is `can_train_no_more`.
fn silent_train_step(ch: &mut Character, rules: &RuleSet, rng: &mut EngineRng) -> Option<()> {
    let flavor = Adnd1::new(rules);
    let classes: Vec<ClassLevel> = ch.class_levels();
    let advancing: Vec<usize> = classes
        .iter()
        .filter(|cl| flavor.eligible_to_train(cl.class, cl.level, ch.exp.max(0) as u32))
        .map(|cl| cl.class)
        .collect();
    if advancing.is_empty() {
        return None;
    }

    // `:2389-2405` — every eligible class gains a level.
    let class_count = classes.len().max(1) as i32;
    let old_mu = ch.class_level[5];
    for &class in &advancing {
        ch.class_level[class] += 1;
    }
    reclac_class_bonuses(ch, rules);

    // `:2430-2451` — the silent trainer's own magic-user spell grants, keyed
    // on the level just reached.
    for &spell in silent_mu_spells(ch.class_level[5], old_mu) {
        crate::magic::learn_spell(ch, spell);
    }

    // `:2453-2481` — the HP gain, skipped once the multiclass cap is reached.
    if ch.hit_dice as u32 > ch.multiclass_level as u32 {
        let new_classes = ch.class_levels();
        let var_f = {
            let mut roller = EngineRoller(rng);
            flavor.hp_die_roll(&new_classes, &advancing, &mut roller) as i32
        };
        let rolled_inc = (var_f / class_count).max(1);
        ch.hit_point_rolled = ch.hit_point_rolled.saturating_add(rolled_inc as u8);
        let con_adj = flavor.con_hp_adjustment(&new_classes, ch.stats.con.current);
        let max_inc = ((var_f + con_adj) / class_count).max(1);
        let lost = ch.hit_point_max.saturating_sub(ch.hit_point_current);
        ch.hit_point_max = ch.hit_point_max.saturating_add(max_inc as u8);
        ch.hit_point_current = ch.hit_point_max.saturating_sub(lost);
    }
    Some(())
}

/// ★ `train_player`'s `if (gbl.silent_training == true)` switch
/// (`ovr018.cs:2430-2451`) — the spells a *silently* trained magic-user is
/// handed, by the level just reached. The switch runs once per train step, so
/// a character that skips a level (impossible here: silent training advances
/// one at a time) would skip its grant too.
///
/// ★ **Correction to coab, with evidence.** coab has level 3 grant
/// `stinking_cloud` **and `protect_from_evil_MU` (0x10)**. Every magic-user
/// the original itself created disagrees: `LEDERA` (fighter 4 / magic-user 4,
/// the GOG bundle's own starting party) and `PHIL.GUY` (a fighter 4 /
/// magic-user 4 rolled in DOSBox) both hold exactly
/// `{0x0A, 0x0B, 0x0C, 0x0F, 0x12, 0x15, 0x1F, 0x22}`, and `PHILIPPE`
/// (magic-user 5) holds that set plus `0x2F` (fireball, the level-5 grant
/// coab gets right). Four of those eight are creation's own openers and three
/// more are the level-2/4/5 grants — which leaves level 3 granting
/// `stinking_cloud` (0x22) and **`charm_person` (0x0A)**, not
/// `protect_from_evil_MU` (0x10), which appears in no real character file at
/// all. Implemented per the data; pinned by
/// `the_silent_trainers_magic_user_spells_match_the_shipped_party`.
fn silent_mu_spells(level: u8, previous: u8) -> &'static [u8] {
    if level == previous {
        return &[];
    }
    match level {
        2 => &[0x0F],       // magic_missile
        3 => &[0x22, 0x0A], // stinking_cloud, charm_person (see above)
        4 => &[0x1F],       // knock
        5 => &[0x2F],       // fireball
        _ => &[],
    }
}

// --- ReclacClassBonuses (ovr026.cs:184-264) and its helpers ---

/// ★ `ReclacClassBonuses` (`ovr026.cs:184-264`) — the derived-field recompute
/// every level-up runs: THAC0 and hit dice from the class table, the third
/// attack for a high-level warrior, [`recalc_spell_state`], the saving throws,
/// the thief skills and `classFlags`.
///
/// The dual-class tail (`:225-263`) is applied too: once the new class exceeds
/// the banked `multiclassLevel`, the OLD class's THAC0 and attack count count
/// again.
pub fn reclac_class_bonuses(ch: &mut Character, rules: &RuleSet) {
    recalc_class_flags_and_thac0(ch, rules);
    // `:193` — HitDice is the highest single class level.
    ch.hit_dice = ch.class_level.iter().copied().max().unwrap_or(0);
    // `:196-201` — the extra attack.
    let extra = ch.class_level[2] >= 7 || ch.class_level[3] >= 7 || ch.class_level[4] >= 8;
    ch.combat.attacks.base[0] = if extra { 3 } else { 2 };

    recalc_spell_state(ch, rules);
    recalc_saving_throws(ch, rules);
    if ch.class_level[6] > 0 {
        recalc_thief_skills(ch, rules);
    }

    // `:225-263` — the dual-class tail.
    if dual_class_exceeds_previous(ch) {
        let mut thac0 = ch.combat.thac0_base as i32;
        for (class, &old) in ch.class_levels_old.iter().enumerate() {
            if old == 0 {
                continue;
            }
            if matches!(class, 2 | 3) && old > 6 {
                ch.combat.attacks.base[0] = 3;
            }
            if class == 4 && old > 7 {
                ch.combat.attacks.base[0] = 3;
            }
            thac0 = thac0
                .max(progression::thac0_stored(rules, class, (old as usize).clamp(1, 12)) as i32);
        }
        ch.combat.thac0_base = thac0.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        if ch.class_levels_old[6] > 0 {
            recalc_thief_skills(ch, rules);
        }
    }
}

/// `DualClassExceedLastLevel` (`ovr026.cs:739-742`): the human dual-class
/// character's *current* first class has passed the level they banked.
fn dual_class_exceeds_previous(ch: &Character) -> bool {
    const HUMAN: u8 = 7;
    if ch.race != HUMAN {
        return false;
    }
    let current = ch
        .class_level
        .iter()
        .take(7)
        .copied()
        .find(|&l| l > 0)
        .unwrap_or(ch.class_level[7]);
    current > ch.multiclass_level
}

/// `classFlags` + THAC0 (`ovr018.cs:569-585`, `ovr026.cs:186-220`).
/// `classFlags` counts a *banked* dual class too, while it is still ahead
/// (`ovr026.cs:215-219`) — which is what keeps a dual-classed fighter's armour
/// readied while the new class catches up.
fn recalc_class_flags_and_thac0(ch: &mut Character, rules: &RuleSet) {
    let mut thac0 = 0i32;
    let mut flags = 0u8;
    let hit_dice = ch.class_level.iter().copied().max().unwrap_or(0);
    for class in 0..8usize {
        let level = ch.class_level[class];
        if level > 0 {
            thac0 = thac0
                .max(progression::thac0_stored(rules, class, (level as usize).clamp(1, 12)) as i32);
        }
        let banked = ch.class_levels_old[class];
        if level > 0 || (banked > 0 && banked < hit_dice) {
            flags = flags.wrapping_add(constants::class_item_flag(rules, class));
        }
    }
    ch.combat.thac0_base = thac0.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
    ch.skills.class_flags = flags;
}

/// `reclac_saving_throws` (`ovr026.cs:334-375`): the best (lowest) target
/// across every active class, then the CON poison bonus.
///
/// The readied-item bonus (`item_affect_6`) is the caller's business — it only
/// exists once a character owns items, and creation's character owns none.
pub fn recalc_saving_throws(ch: &mut Character, rules: &RuleSet) {
    for save in 0..5usize {
        let mut best = 20u8;
        for class in 0..8usize {
            let level = ch.class_level[class];
            if level > 0 {
                best = best.min(progression::save_throw(
                    rules,
                    class,
                    (level as usize).clamp(1, 12),
                    save,
                ));
            }
        }
        // `:360-368` — the monk slot's own re-read against the banked level.
        if ch.class_level[7] > ch.class_levels_old[7] && ch.class_levels_old[7] > 0 {
            best = best.min(progression::save_throw(
                rules,
                7,
                (ch.class_levels_old[7] as usize).clamp(1, 12),
                save,
            ));
        }
        ch.skills.save_verse[save] = best;
    }
    // ★ `SaveVersePoisonBonus` (`ovr026.cs:377-423`) — TWO ladders on the
    // poison slot, and the first one is race-gated: a **dwarf or a halfling**
    // (or anyone with a readied `item_affect_6`) gets the low/middle CON
    // ladder as well as the high one everybody gets. `TRAVIS`, the bundle's
    // dwarf, is the pin: fighter 4 / thief 5 gives a bare 12, and his record
    // stores 16 — CON 16 lands in the `14..=17` band, +4.
    //
    // The readied-item half is the caller's business (creation owns no
    // items), so only the racial gate is applied here.
    let con = ch.stats.con.current;
    let poison = &mut ch.skills.save_verse[0];
    if matches!(ch.race, 1 | 5) {
        *poison = poison.saturating_add(match con {
            4..=6 => 1,
            7..=10 => 2,
            11..=13 => 3,
            14..=17 => 4,
            18 => 5,
            _ => 0,
        });
    }
    *poison = poison.saturating_add(match con {
        19 | 20 => 1,
        21 | 22 => 2,
        23 | 24 => 3,
        25 => 4,
        _ => 0,
    });
}

/// `reclac_thief_skills` (`ovr026.cs:500-555`) via the rules pack's
/// `skill_percentages` — base chance by thief level, race adjustment, and the
/// DEX adjustment on skills 1..=5 only.
pub fn recalc_thief_skills(ch: &mut Character, rules: &RuleSet) {
    let flavor = Adnd1::new(rules);
    let level = ch.skill_level(crate::party::SKILL_CLERIC + 6).max(1) as u32;
    ch.skills.thief_skills =
        flavor.skill_percentages(ch.race as usize, ch.stats.dex.current, level);
}

/// ★ `sub_6A00F` (`ovr026.cs:55-181`) — the spell-slot recompute, plus the two
/// "you just know them all" grants.
///
/// Slots come from the rules pack (`Flavor::spell_slots`), which already
/// carries the cleric/paladin (`divine`), ranger-nature (`hybrid`) and
/// magic-user (`arcane`) tracks. The **row mapping is now real-data pinned**
/// rather than provisional: `SHARA` (cleric 5) stores `[5,5,2,0,0]` in row 0,
/// `PHILIPPE` (magic-user 5) `[4,2,1,0,0]` in row 2, `LEDERA` (magic-user 4)
/// `[3,2,0,0,0]` in row 2.
///
/// The grants: a cleric learns **every** cleric spell of a level they have a
/// slot for, `animate_dead` alone excluded (`:92-97`); a ranger past level 7
/// learns every druid spell (`:143-149`); a paladin past level 8 learns the
/// cleric spells their slots reach (`:112-123`, and unlike the cleric arm this
/// one does *not* exclude `animate_dead`).
pub fn recalc_spell_state(ch: &mut Character, rules: &RuleSet) {
    use crate::magic::{SpellClass, SPELL_TABLE};
    let flavor = Adnd1::new(rules);
    let classes = ch.class_levels();
    let slots = flavor.spell_slots(&classes, ch.stats.wis.current);
    ch.magic.cast_count[0] = slots.divine;
    ch.magic.cast_count[1] = {
        let mut row = [0u8; 5];
        row[..3].copy_from_slice(&slots.hybrid);
        row
    };
    ch.magic.cast_count[2] = slots.arcane;

    let cleric_level = ch.skill_level(crate::party::SKILL_CLERIC);
    let paladin_level = ch.skill_level(crate::party::SKILL_PALADIN);
    let ranger_level = ch.skill_level(crate::party::SKILL_RANGER);

    const ANIMATE_DEAD: u8 = 0x24;
    if cleric_level > 0 || paladin_level > 8 {
        for id in 1..SPELL_TABLE.len() as u8 {
            let row = SPELL_TABLE[id as usize];
            if row.class_ != SpellClass::Cleric || row.level == 0 {
                continue;
            }
            // ★ `sp_class = (spellLevel - 1) / 5; sp_lvl = (spellLevel - 1) % 5;`
            // (`ovr026.cs:89-90,115-116`) — the table's `spellLevel` is a
            // COMBINED 1..15 encoding, so a cleric row above level 5 lands in
            // a different `spellCastCount` ROW, not just a different column.
            // The shipped game has exactly one such row (`Restoration`,
            // `spellLevel` 7 → row 1 column 1, the druid track a cleric never
            // has), and `SHARA`'s grimoire duly lacks it.
            let (sp_class, sp_lvl) = ((row.level as usize - 1) / 5, (row.level as usize - 1) % 5);
            if ch
                .magic
                .cast_count
                .get(sp_class)
                .and_then(|r| r.get(sp_lvl))
                .copied()
                .unwrap_or(0)
                == 0
            {
                continue;
            }
            if cleric_level > 0 && id == ANIMATE_DEAD {
                continue;
            }
            crate::magic::learn_spell(ch, id);
        }
    }
    if ranger_level > 7 {
        for id in 1..SPELL_TABLE.len() as u8 {
            if SPELL_TABLE[id as usize].class_ == SpellClass::Druid {
                crate::magic::learn_spell(ch, id);
            }
        }
    }
}

/// The whole of `createPlayer` up to the name prompt — [`begin`] then one
/// [`reroll`]. The screen calls the two halves separately (it has pickers
/// between them); this is the headless entry point.
pub fn create(rules: &RuleSet, rng: &mut EngineRng, picks: Picks) -> Character {
    let mut ch = begin(rules, rng, picks);
    reroll(&mut ch, rules, rng);
    ch
}

/// `player.stats2.Str00.full = player.stats2.Str00.cur;` (`ovr018.cs:881`) plus
/// the name — `createPlayer`'s last two writes before the save prompt.
pub fn finish(ch: &mut Character, name: &str) {
    ch.name = name.chars().take(15).collect();
    ch.stats.str_exceptional.original = ch.stats.str_exceptional.current;
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::save_orig::decode_char_record;

    fn rules() -> RuleSet {
        RuleSet::load()
    }

    /// The bundle's own character files, if they are there. Each is a record
    /// the ORIGINAL wrote — ground truth for everything creation derives.
    fn bundled(name: &str) -> Option<gbx_formats::save_orig::CharRecord> {
        let dir = std::env::var_os("GBX_DATA_DIR")?;
        let path = std::path::Path::new(&dir).join("SAVE").join(name);
        let bytes = std::fs::read(path).ok()?;
        decode_char_record(&bytes).ok()
    }

    #[test]
    fn the_race_picker_offers_six_races_and_skips_half_orc() {
        assert_eq!(CREATABLE_RACES, [1, 2, 3, 4, 5, 7]);
        assert!(
            !CREATABLE_RACES.contains(&6),
            "half-orc is import-only in CotAB"
        );
    }

    #[test]
    fn the_default_icon_colours_match_every_real_character_file() {
        assert_eq!(default_icon_colours(), [0x91, 0xA2, 0xB3, 0xC4, 0xE6, 0xF7]);
        if let Some(rec) = bundled("JOE.GUY") {
            assert_eq!(rec.icon_colours, default_icon_colours());
        }
    }

    #[test]
    fn a_fresh_character_carries_the_creation_constants() {
        let r = rules();
        let mut rng = EngineRng::new(7);
        let ch = begin(
            &r,
            &mut rng,
            Picks {
                race: 7,
                sex: 0,
                class_id: 6,
                alignment: 7,
            },
        );
        assert_eq!(ch.combat.base_ac, 50);
        assert_eq!(ch.opaque.field_de, 1);
        assert!(ch.status.in_combat);
        assert_eq!(ch.icon.icon_id, 0x0A);
        assert_eq!(ch.icon.icon_size, 2, "human is a large icon");
        assert_eq!(ch.exp, 25_000);
        assert_eq!(ch.class_level[6], 1);
    }

    #[test]
    fn a_small_race_gets_a_small_icon_and_its_racial_affects() {
        let r = rules();
        let mut rng = EngineRng::new(7);
        for (race, size) in [(1u8, 1u8), (3, 1), (5, 1), (2, 2), (4, 2), (7, 2)] {
            let class_id = class_choices(&r, race)[0];
            let ch = begin(
                &r,
                &mut rng,
                Picks {
                    race,
                    sex: 0,
                    class_id,
                    alignment: alignment_choices(&r, class_id)[0],
                },
            );
            assert_eq!(ch.icon.icon_size, size, "race {race}");
        }
        // dwarf: con_saving_bonus, dwarf_vs_orc, dwarf_and_gnome_vs_giants.
        let ch = begin(
            &r,
            &mut rng,
            Picks {
                race: 1,
                sex: 0,
                class_id: 2,
                alignment: 0,
            },
        );
        assert_eq!(ch.affects.len(), 3);
        assert!(ch.has_affect(0x61) && ch.has_affect(0x1a) && ch.has_affect(0x2f));
    }

    #[test]
    fn a_paladin_and_a_ranger_get_their_creation_affects() {
        let r = rules();
        let mut rng = EngineRng::new(3);
        let pal = begin(
            &r,
            &mut rng,
            Picks {
                race: 7,
                sex: 0,
                class_id: 3,
                alignment: 0,
            },
        );
        assert_eq!(pal.status.paladin_cures_left, 1);
        assert!(pal.has_affect(AFFECT_PROTECTION_FROM_EVIL));
        let ran = begin(
            &r,
            &mut rng,
            Picks {
                race: 7,
                sex: 0,
                class_id: 4,
                alignment: 0,
            },
        );
        assert!(ran.has_affect(AFFECT_RANGER_VS_GIANT));
    }

    #[test]
    fn multiclass_starting_experience_follows_the_combo_id() {
        let r = rules();
        let mut rng = EngineRng::new(1);
        let two = begin(
            &r,
            &mut rng,
            Picks {
                race: 2,
                sex: 0,
                class_id: 13,
                alignment: 4,
            },
        );
        assert_eq!(two.exp, 12_500);
        assert_eq!(two.class_level[2], 1);
        assert_eq!(two.class_level[5], 1);
        let three = begin(
            &r,
            &mut rng,
            Picks {
                race: 2,
                sex: 0,
                class_id: 15,
                alignment: 4,
            },
        );
        assert_eq!(three.exp, 8_333);
    }

    /// ★ The stat roller against the original's own arithmetic: 36 draws in
    /// the interleaved order, best of six per stat, `+1` included — so the
    /// pre-clamp domain is `4..=19`, not `3..=18`.
    #[test]
    fn the_stat_roller_consumes_thirty_six_draws_in_the_interleaved_order() {
        let r = rules();
        let mut rng = EngineRng::new(0x1234_5678);
        let mut ch = begin(
            &r,
            &mut rng,
            Picks {
                race: 7,
                sex: 0,
                class_id: 6,
                alignment: 7,
            },
        );
        // Snapshot the PRNG, roll the six stats through the flavor directly,
        // and count how many draws that took.
        let before = rng.state();
        let flavor = Adnd1::new(&r);
        let scores = {
            let mut roller = EngineRoller(&mut rng);
            flavor.roll_ability_scores(&mut roller)
        };
        let after_direct = rng.state();
        let mut count = 0;
        let mut probe = EngineRng::new(0);
        probe.set_state(before);
        while probe.state() != after_direct {
            probe.random(6);
            count += 1;
            assert!(count <= 200, "the roller never returned to the same state");
        }
        assert_eq!(count, 36 * 3, "36 rolls of 3d6");
        for s in scores {
            assert!((4..=19).contains(&s), "3d6+1 lands in 4..=19, got {s}");
        }
        reroll(&mut ch, &r, &mut rng);
        assert!(ch.hit_point_max >= 1);
    }

    /// The class minimum wins over a bad roll: a paladin's CHA floor is 17
    /// (`class_stats_min` row 3), so no paladin can ever roll below it.
    #[test]
    fn a_class_minimum_raises_a_low_roll() {
        let r = rules();
        for seed in 1..40u32 {
            let mut rng = EngineRng::new(seed);
            let ch = create(
                &r,
                &mut rng,
                Picks {
                    race: 7,
                    sex: 0,
                    class_id: 3,
                    alignment: 0,
                },
            );
            assert!(
                ch.stats.cha.current >= 17,
                "paladin CHA floor (seed {seed})"
            );
            assert!(ch.stats.str_score.current >= 12);
            assert!(ch.stats.wis.current >= 13);
        }
    }

    /// The race/sex ceiling wins over a good roll: a female dwarf's STR caps
    /// at 17 where a male dwarf's caps at 18.
    #[test]
    fn a_race_sex_ceiling_caps_a_high_roll() {
        let r = rules();
        let (min_m, max_m) =
            creation_limits::race_sex_min_max(&r, creation_limits::Stat::Str, 1, 0);
        let (min_f, max_f) =
            creation_limits::race_sex_min_max(&r, creation_limits::Stat::Str, 1, 1);
        assert_eq!((min_m, max_m, min_f, max_f), (8, 18, 8, 17));
        for seed in 1..40u32 {
            let mut rng = EngineRng::new(seed);
            let ch = create(
                &r,
                &mut rng,
                Picks {
                    race: 1,
                    sex: 1,
                    class_id: 2,
                    alignment: 0,
                },
            );
            assert!((8..=17).contains(&ch.stats.str_score.current));
            assert_eq!(
                ch.stats.str_exceptional.current, 0,
                "a female dwarf cannot reach 18, so never rolls a percentile"
            );
        }
    }

    /// A multi-class cleric's WIS never sits below 13 (`ovr018.cs:720-725`) —
    /// and that floor is NOT a `class_stats_min` row, so it has to be applied
    /// separately or it silently goes missing.
    #[test]
    fn a_multiclass_cleric_floors_wisdom_at_thirteen() {
        let r = rules();
        for seed in 1..30u32 {
            let mut rng = EngineRng::new(seed);
            let ch = create(
                &r,
                &mut rng,
                Picks {
                    race: 4,
                    sex: 0,
                    class_id: 12, // cleric/thief
                    alignment: 0,
                },
            );
            assert!(ch.stats.wis.current >= 13, "seed {seed}");
        }
    }

    /// ★ **The end-to-end pin.** `JOE.GUY` is a human thief the original
    /// created and silently trained on its 25,000 starting XP. Everything
    /// creation *derives* — level, THAC0, saves, thief skills, class flags,
    /// money, the attack profile, the icon defaults — must match, byte for
    /// byte, from nothing but the four picks and his stats.
    #[test]
    fn a_created_thief_reproduces_joe_guys_derived_fields() {
        let Some(joe) = bundled("JOE.GUY") else {
            return;
        };
        let r = rules();
        let mut rng = EngineRng::new(1);
        let mut ch = create(
            &r,
            &mut rng,
            Picks {
                race: joe.race,
                sex: joe.sex,
                class_id: joe.class,
                alignment: joe.alignment,
            },
        );
        // Force his rolled stats in and re-derive: the dice are his, the
        // arithmetic is ours.
        ch.stats.str_score.current = joe.stats.str.current;
        ch.stats.dex.current = joe.stats.dex.current;
        ch.stats.con.current = joe.stats.con.current;
        ch.stats.wis.current = joe.stats.wis.current;
        reclac_class_bonuses(&mut ch, &r);

        assert_eq!(ch.class_level, joe.class_level, "level 6 thief on 25000 XP");
        assert_eq!(ch.hit_dice, joe.hit_dice);
        assert_eq!(ch.combat.thac0_base, joe.thac0_base);
        assert_eq!(ch.skills.save_verse, joe.save_verse);
        assert_eq!(ch.skills.thief_skills, joe.thief_skills);
        assert_eq!(ch.skills.class_flags, joe.class_flags);
        assert_eq!(ch.money.platinum, joe.money[4]);
        assert_eq!(ch.combat.attacks.base, joe.attack_profile_base);
        assert_eq!(ch.icon.colours, joe.icon_colours);
        assert_eq!(ch.combat.base_movement, joe.base_movement);
        assert_eq!(ch.combat.base_ac, joe.base_ac);
        assert_eq!(ch.opaque.field_de, joe.field_de);
        assert_eq!(ch.opaque.field_125, joe.field_125);
        assert_eq!(ch.exp, joe.exp);
        assert!(ch.magic.spell_book.iter().all(|&b| b == 0));
    }

    /// The same, for a small race with a different thief-skill row —
    /// `STEVE.GUY` is a gnome thief, and gnome adjustments differ from human
    /// on every skill.
    #[test]
    fn a_created_gnome_thief_reproduces_steve_guys_skills() {
        let Some(steve) = bundled("STEVE.GUY") else {
            return;
        };
        let r = rules();
        let mut rng = EngineRng::new(2);
        let mut ch = create(
            &r,
            &mut rng,
            Picks {
                race: steve.race,
                sex: steve.sex,
                class_id: steve.class,
                alignment: steve.alignment,
            },
        );
        ch.stats.dex.current = steve.stats.dex.current;
        ch.stats.con.current = steve.stats.con.current;
        reclac_class_bonuses(&mut ch, &r);
        assert_eq!(ch.icon.icon_size, steve.icon_size, "gnome is a small icon");
        assert_eq!(ch.skills.thief_skills, steve.thief_skills);
        assert_eq!(ch.skills.save_verse, steve.save_verse);
        assert_eq!(ch.combat.thac0_base, steve.thac0_base);
    }

    /// ★ **The silent trainer's magic-user grants**, against the two
    /// independent characters that carry them: `PHIL.GUY` (rolled in DOSBox)
    /// and `LEDERA` (the GOG bundle's own fighter/magic-user), plus
    /// `PHILIPPE`'s level-5 fireball. This is the test that would fail if the
    /// level-3 grant were coab's `protect_from_evil_MU`.
    #[test]
    fn the_silent_trainers_magic_user_spells_match_the_shipped_party() {
        let Some(phil) = bundled("PHIL.GUY") else {
            return;
        };
        let r = rules();
        let mut rng = EngineRng::new(5);
        let mut ch = create(
            &r,
            &mut rng,
            Picks {
                race: phil.race,
                sex: phil.sex,
                class_id: phil.class,
                alignment: phil.alignment,
            },
        );
        ch.stats.wis.current = phil.stats.wis.current;
        reclac_class_bonuses(&mut ch, &r);
        let ours: Vec<usize> = ch
            .magic
            .spell_book
            .iter()
            .enumerate()
            .filter(|(_, &b)| b != 0)
            .map(|(i, _)| i + 1)
            .collect();
        let theirs: Vec<usize> = phil
            .spell_book
            .iter()
            .enumerate()
            .filter(|(_, &b)| b != 0)
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(ours, theirs, "PHIL.GUY's grimoire");
        assert_eq!(ch.class_level, phil.class_level);
        assert_eq!(ch.magic.cast_count[2], phil.spell_cast_count[2]);
    }

    /// A cleric knows every spell of every level they can cast, minus
    /// `animate_dead` — `SHARA`, the bundle's own level-5 cleric, is the pin.
    #[test]
    fn a_created_cleric_reproduces_sharas_grimoire_and_slots() {
        let Some(shara) = bundled("CHRDATA5.SAV") else {
            return;
        };
        let r = rules();
        let mut rng = EngineRng::new(11);
        let mut ch = create(
            &r,
            &mut rng,
            Picks {
                race: shara.race,
                sex: shara.sex,
                class_id: shara.class,
                alignment: shara.alignment,
            },
        );
        ch.stats.wis.current = shara.stats.wis.current;
        ch.stats.con.current = shara.stats.con.current;
        reclac_class_bonuses(&mut ch, &r);
        assert_eq!(ch.class_level, shara.class_level, "cleric 5 on 25000 XP");
        assert_eq!(ch.magic.cast_count[0], shara.spell_cast_count[0]);
        assert_eq!(ch.magic.spell_book, shara.spell_book);
        assert_eq!(ch.skills.save_verse, shara.save_verse);
        assert_eq!(ch.combat.thac0_base, shara.thac0_base);
        assert_eq!(ch.skills.class_flags, shara.class_flags);
    }

    /// A paladin's derived fields, against `MATHEW`/`MARK` — level 5 on
    /// 25,000 XP, `classFlags` 0x40, and no spells at all below level 9.
    #[test]
    fn a_created_paladin_reproduces_mathews_derived_fields() {
        let Some(m) = bundled("CHRDATA1.SAV") else {
            return;
        };
        let r = rules();
        let mut rng = EngineRng::new(13);
        let mut ch = create(
            &r,
            &mut rng,
            Picks {
                race: m.race,
                sex: m.sex,
                class_id: m.class,
                alignment: m.alignment,
            },
        );
        ch.stats.con.current = m.stats.con.current;
        reclac_class_bonuses(&mut ch, &r);
        assert_eq!(ch.class_level, m.class_level);
        assert_eq!(ch.combat.thac0_base, m.thac0_base);
        assert_eq!(ch.skills.save_verse, m.save_verse);
        assert_eq!(ch.skills.class_flags, m.class_flags);
        assert!(ch.magic.spell_book.iter().all(|&b| b == 0));
        assert_eq!(ch.status.paladin_cures_left, 1);
    }

    /// A dwarf fighter/thief — `TRAVIS`, the bundle's multi-class, whose
    /// saving throws are the per-save minimum ACROSS both classes and whose
    /// `classFlags` is the sum of both.
    #[test]
    fn a_created_multiclass_reproduces_travis_derived_fields() {
        let Some(t) = bundled("CHRDATA3.SAV") else {
            return;
        };
        let r = rules();
        let mut rng = EngineRng::new(17);
        let mut ch = create(
            &r,
            &mut rng,
            Picks {
                race: t.race,
                sex: t.sex,
                class_id: t.class,
                alignment: t.alignment,
            },
        );
        ch.stats.dex.current = t.stats.dex.current;
        ch.stats.con.current = t.stats.con.current;
        reclac_class_bonuses(&mut ch, &r);
        assert_eq!(ch.class_level, t.class_level, "fighter 4 / thief 5");
        assert_eq!(ch.combat.thac0_base, t.thac0_base);
        assert_eq!(ch.skills.save_verse, t.save_verse);
        assert_eq!(ch.skills.thief_skills, t.thief_skills);
        assert_eq!(ch.skills.class_flags, t.class_flags);
        assert_eq!(ch.exp, t.exp);
    }

    /// Every legal (race, class, alignment) triple creates a character with
    /// sane derived fields — the broad sweep behind the named pins above.
    #[test]
    fn every_legal_pick_triple_creates_a_usable_character() {
        let r = rules();
        let mut rng = EngineRng::new(0xDEAD_BEEF);
        let mut made = 0;
        for race in CREATABLE_RACES {
            for class_id in class_choices(&r, race) {
                for alignment in alignment_choices(&r, class_id) {
                    for sex in 0..2u8 {
                        let ch = create(
                            &r,
                            &mut rng,
                            Picks {
                                race,
                                sex,
                                class_id,
                                alignment,
                            },
                        );
                        assert!(ch.hit_point_max >= 1, "{race}/{class_id}");
                        assert!(ch.hit_point_current == ch.hit_point_max);
                        assert!(ch.hit_dice >= 1);
                        assert!(ch.class_level.iter().any(|&l| l > 0));
                        assert_eq!(ch.money.platinum, 300);
                        assert!(ch.skills.save_verse.iter().all(|&s| s > 0));
                        made += 1;
                    }
                }
            }
        }
        assert!(made > 100, "only {made} combinations exercised");
    }
}
