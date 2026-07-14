//! The `adnd1` implementation of [`crate::flavor::Flavor`] (D-RP5, M3's
//! slice) — every method cites its coab source and consumes the typed
//! accessors built in M3 steps 1-2. AD&D vocabulary (class names, THAC0,
//! spell traditions) lives entirely in this module's comments and internal
//! helpers, never in the trait it implements.

use crate::adnd1::{creation, creation_limits, hp_hd, progression, spell_slots, thief_skills};
use crate::flavor::{
    AbilityStat, ClassLevel, CreationHp, Flavor, Roller, SpellSlots as EngineSpellSlots, StatBlock,
};
use crate::pack::RuleSet;

/// coab's base `ClassId` order (`Classes/Enums.cs:69`) — cleric=0,
/// druid=1, fighter=2, paladin=3, ranger=4, magic_user=5, thief=6, monk=7.
/// Multi-class characters are represented to this module as a list of
/// these base ids with independent levels (never coab's combined
/// `ClassId` 8..=16 combo values — those are a coab implementation detail
/// this module never needs).
mod class_id {
    pub const CLERIC: usize = 0;
    pub const FIGHTER: usize = 2;
    pub const PALADIN: usize = 3;
    pub const RANGER: usize = 4;
    pub const MAGIC_USER: usize = 5;
}

/// The `adnd1` flavor, borrowing the loaded [`RuleSet`] its accessors read.
pub struct Adnd1<'a> {
    pub rules: &'a RuleSet,
}

impl<'a> Adnd1<'a> {
    pub fn new(rules: &'a RuleSet) -> Self {
        Adnd1 { rules }
    }

    fn level_of(classes: &[ClassLevel], class: usize) -> u32 {
        classes
            .iter()
            .find(|c| c.class == class)
            .map(|c| c.level)
            .unwrap_or(0)
    }

    /// `ovr018.cs:894-929`'s `con_bonus` — an independent, non-identical
    /// second CON-to-HP formula used only by [`Flavor::max_hp_ceiling`]
    /// (`calc_max_hp`'s display path); [`Flavor::con_hp_adjustment`] uses
    /// the separate `con_hp_adj`/`get_con_hp_adj` path instead. Both exist
    /// verbatim in coab — a real duplication, not a mistake to reconcile.
    fn con_bonus_display(class: usize, con: u8) -> i32 {
        match con {
            3 => -2,
            4..=6 => -1,
            7..=14 => 0,
            15 | 16 => 1,
            _ if matches!(
                class,
                class_id::FIGHTER | class_id::PALADIN | class_id::RANGER
            ) =>
            {
                con as i32 - 14
            }
            _ => 2,
        }
    }
}

impl<'a> Flavor for Adnd1<'a> {
    /// `RaceClasses[race]` membership (`ovr018.cs:455-464`).
    fn class_admissible(&self, race: usize, class: usize) -> bool {
        creation_limits::race_classes(self.rules, race).contains(&(class as u8))
    }

    /// `class_alignments[class]` membership (`ovr018.cs:590-618`).
    fn alignment_admissible(&self, class: usize, alignment: usize) -> bool {
        creation::allowed_alignments(self.rules, class).contains(&(alignment as u8))
    }

    /// coab applies no race/class restriction on sex (`ovr018.cs:423-450`)
    /// — sex only selects a column in the race/sex stat-limit tables.
    fn sex_admissible(&self, _race: usize, sex: usize) -> bool {
        sex == 0 || sex == 1
    }

    /// `StatValue.EnforceRaceSexLimits` + `EnforceClassLimits`
    /// (`Classes/Player.cs:47-64`), in that order: clamp into the
    /// race/sex range, then raise to the class minimum.
    fn clamp_stat_for_creation(
        &self,
        stat: AbilityStat,
        race: usize,
        sex: usize,
        class: usize,
        value: u8,
    ) -> u8 {
        let table_stat = match stat {
            AbilityStat::Str => creation_limits::Stat::Str,
            AbilityStat::Int => creation_limits::Stat::Int,
            AbilityStat::Wis => creation_limits::Stat::Wis,
            AbilityStat::Dex => creation_limits::Stat::Dex,
            AbilityStat::Con => creation_limits::Stat::Con,
            AbilityStat::Cha => creation_limits::Stat::Cha,
        };
        let (min, max) = creation_limits::race_sex_min_max(self.rules, table_stat, race, sex);
        let mut v = value.min(max).max(min);

        let class_min = creation_limits::class_stats_min(self.rules, class);
        let floor = match stat {
            AbilityStat::Str => class_min.str,
            AbilityStat::Int => class_min.int,
            AbilityStat::Wis => class_min.wis,
            AbilityStat::Dex => class_min.dex,
            AbilityStat::Con => class_min.con,
            AbilityStat::Cha => class_min.cha,
        };
        v = v.max(floor);
        v
    }

    /// Best-of-6 rerolls of `3d6+1` (`ovr018.cs:675-683`, exact).
    fn roll_ability_score(&self, roller: &mut dyn Roller) -> u8 {
        let mut best = 0u32;
        for _ in 0..6 {
            best = best.max(roller.roll(6, 3) + 1);
        }
        best as u8
    }

    /// `ovr018.cs:699-701`'s gate: any of fighter, ranger, or paladin among
    /// the character's classes.
    fn exceptional_strength_eligible(&self, classes: &[ClassLevel]) -> bool {
        classes.iter().any(|c| {
            matches!(
                c.class,
                class_id::FIGHTER | class_id::RANGER | class_id::PALADIN
            )
        })
    }

    /// `seg051.Random(100) + 1` (`ovr018.cs:703`) — a single d100.
    fn roll_exceptional_strength(&self, roller: &mut dyn Roller) -> u8 {
        roller.roll(100, 1) as u8
    }

    /// `ovr018.cs:622-651`. Single-class characters roll `starting_age`'s
    /// dice directly. Multi-class characters take an UN-ROLLED maximum
    /// (`base + dice_count * dice_size`) of one *representative* base
    /// class's row — coab's own quirk, not a simplification: the
    /// cleric/magic-user combo (`mc_c_mu`) has no covered `case` in the
    /// original switch at all, so age is transcribed as 0 for it here
    /// rather than inventing a value coab itself never assigns.
    fn starting_age(&self, race: usize, classes: &[ClassLevel], roller: &mut dyn Roller) -> u16 {
        let mut ids: Vec<usize> = classes.iter().map(|c| c.class).collect();
        ids.sort_unstable();
        ids.dedup();

        let representative = match ids.as_slice() {
            [single] => {
                let sa = creation_limits::starting_age(self.rules, race, *single);
                return sa
                    .base_age
                    .saturating_add(roller.roll(sa.dice_size as u32, sa.dice_count as u32) as u16);
            }
            [0, 2] | [0, 2, 5] | [0, 4] | [0, 6] => Some(0),
            [2, 5] | [2, 5, 6] | [5, 6] => Some(6),
            [2, 6] => Some(2),
            _ => None,
        };

        let Some(representative) = representative else {
            return 0;
        };
        let sa = creation_limits::starting_age(self.rules, race, representative);
        sa.base_age + sa.dice_count as u16 * sa.dice_size as u16
    }

    /// `player.Money.SetCoins(Money.Platinum, 300)` (`ovr018.cs:807`).
    fn starting_money(&self) -> u32 {
        300
    }

    /// `ovr018.cs:483-561`. `exp = 25000` is the unconditional default,
    /// overridden by the ten explicit multi-class combinations below (each
    /// transcribed verbatim from its own `if`/`else if` branch — the
    /// pattern is NOT a simple function of class count: `mc_mu_t`
    /// (magic-user + thief, two classes) gets the three-class rate 8333,
    /// not the two-class rate 12500.
    fn starting_experience(&self, classes: &[ClassLevel]) -> u32 {
        let mut ids: Vec<usize> = classes.iter().map(|c| c.class).collect();
        ids.sort_unstable();
        ids.dedup();
        match ids.as_slice() {
            [0, 2] | [0, 4] | [0, 5] | [0, 6] | [2, 5] | [2, 6] => 12_500,
            [0, 2, 5] | [2, 5, 6] | [5, 6] => 8_333,
            _ => 25_000,
        }
    }

    /// `sub_509E0` (`ovr018.cs:2127-2174`) — used both at creation (all
    /// active classes trained) and at level-up (one class trained). Below
    /// each trained class's hit-dice cap: roll-twice-take-higher
    /// (`:2145-2151`) via [`hp_hd::level_up_dice_count`] (already capping
    /// to a single die past character level 1) and [`hp_hd::hit_die_size`].
    /// At/above the cap: a fixed per-class-group value that OVERWRITES the
    /// running total rather than adding to it (`:2157-2168`) — a real
    /// shared-accumulator quirk (a multi-class character with one
    /// below-cap and one at-cap class can lose the below-cap roll
    /// entirely, depending on iteration order 0..=7). Druid and monk have
    /// no fixed value at all in that branch, so an at-cap druid/monk
    /// leaves the total untouched — transcribed exactly, not patched.
    fn hp_die_roll(
        &self,
        classes: &[ClassLevel],
        trained: &[usize],
        roller: &mut dyn Roller,
    ) -> u32 {
        let mut total: u32 = 0;
        for class_index in 0..=7usize {
            let Some(cl) = classes.iter().find(|c| c.class == class_index) else {
                continue;
            };
            if cl.level == 0 || !trained.contains(&class_index) {
                continue;
            }
            let max_hd = hp_hd::max_class_hit_dice(self.rules, class_index) as u32;
            if cl.level < max_hd {
                let dice_count =
                    hp_hd::level_up_dice_count(self.rules, class_index, cl.level) as u32;
                let die_size = hp_hd::hit_die_size(self.rules, class_index) as u32;
                let roll_a = roller.roll(die_size, dice_count);
                let roll_b = roller.roll(die_size, dice_count);
                total += roll_a.max(roll_b);
            } else {
                total = match class_index {
                    2 | 3 => 3,
                    4 | 0 | 6 => 2,
                    5 => 1,
                    _ => total,
                };
            }
        }
        total
    }

    /// `get_con_hp_adj` (`ovr018.cs:1982-2030`). The CON-17+ warrior
    /// extras (`:1999-2018`) only ever fire for a single-classed
    /// fighter/paladin/ranger: coab gates them on `player._class`, the
    /// character's single "active class" field, which for any multi-class
    /// character always holds one of coab's combo ids (never exactly
    /// fighter/paladin/ranger's own id) — `classes.len() == 1` is the
    /// faithful equivalent. The ranger-level-1 doubling (`:2021-2025`)
    /// doubles the ENTIRE running total, including any earlier class's
    /// already-added contribution in iteration order — also transcribed
    /// exactly.
    fn con_hp_adjustment(&self, classes: &[ClassLevel], con: u8) -> i32 {
        let mut hp_adj: i32 = 0;
        let single_warrior = classes.len() == 1
            && matches!(
                classes[0].class,
                class_id::FIGHTER | class_id::PALADIN | class_id::RANGER
            );

        for class_index in 0..=7usize {
            let Some(cl) = classes.iter().find(|c| c.class == class_index) else {
                continue;
            };
            if cl.level == 0 {
                continue;
            }
            let max_hd = hp_hd::max_class_hit_dice(self.rules, class_index) as u32;
            if cl.level >= max_hd {
                continue;
            }
            hp_adj += hp_hd::con_hp_adj(self.rules, con) as i32;

            if single_warrior {
                hp_adj += match con {
                    17 => 1,
                    18 => 2,
                    19 | 20 => 3,
                    21..=23 => 4,
                    24 | 25 => 5,
                    _ => 0,
                };
            }

            if class_index == class_id::RANGER && cl.level == 1 {
                hp_adj *= 2;
            }
        }
        hp_adj
    }

    /// `ovr018.cs:807-830`'s creation-time flow: roll every active class,
    /// apply the CON adjustment (averaging across classes on either
    /// branch, matching `var_1E < 0`'s divide-or-floor-to-1 split versus
    /// the positive branch's plain divide), then also average the raw
    /// roll (`:830`).
    fn hp_gain_at_creation(
        &self,
        classes: &[ClassLevel],
        con: u8,
        roller: &mut dyn Roller,
    ) -> CreationHp {
        let trained: Vec<usize> = classes.iter().map(|c| c.class).collect();
        let rolled = self.hp_die_roll(classes, &trained, roller);
        let con_adj = self.con_hp_adjustment(classes, con);
        let class_count = classes.len().max(1) as i32;

        let max = if con_adj < 0 {
            if rolled as i32 > con_adj.abs() + class_count {
                (rolled as i32 + con_adj) / class_count
            } else {
                1
            }
        } else {
            (rolled as i32 + con_adj) / class_count
        };

        CreationHp {
            rolled: rolled / class_count as u32,
            max: max.max(0) as u32,
        }
    }

    /// `calc_max_hp` (`ovr018.cs:2089-2120`) — an alternate, non-rolled
    /// display formula using `hp_calc_table` and the independent
    /// [`Self::con_bonus_display`] formula (NOT `get_con_hp_adj`). Below a
    /// class's hit-dice cap the per-class contributions are summed; at/
    /// above the cap the running total is OVERWRITTEN (`hpt.max_base +
    /// over_count * hpt.max_mult`), the same shared-accumulator quirk as
    /// [`Flavor::hp_die_roll`] — transcribed exactly.
    fn max_hp_ceiling(&self, classes: &[ClassLevel], con: u8) -> u32 {
        let mut max_hp: i32 = 0;
        let mut class_count: i32 = 0;
        for class_index in 0..=7usize {
            let Some(cl) = classes.iter().find(|c| c.class == class_index) else {
                continue;
            };
            if cl.level == 0 {
                continue;
            }
            let hpt = hp_hd::hp_calc(self.rules, class_index);
            let bonus = Self::con_bonus_display(class_index, con);
            let max_hd = hp_hd::max_class_hit_dice(self.rules, class_index) as u32;
            class_count += 1;
            if cl.level < max_hd {
                max_hp += (bonus + hpt.dice as i32) * (cl.level as i32 + hpt.lvl_bonus as i32);
            } else {
                let over_count = (cl.level - max_hd) + 1;
                max_hp = hpt.max_base as i32 + (over_count as i32 * hpt.max_mult as i32);
            }
        }
        if class_count == 0 {
            return 0;
        }
        (max_hp / class_count).max(0) as u32
    }

    /// `train_player`'s per-class eligibility gate (`ovr018.cs:2226-2228`):
    /// a stored threshold of 0 (no further training via this table, e.g.
    /// druid/monk or beyond a class's last stored level) is never
    /// eligible, matching coab's `exp_table[...] > 0` guard. Race-level-
    /// limit gating (`Limits.RaceClassLimit`, `Limits.cs:106-290`) is out
    /// of this session's M3 slice — no race-level-limit pack exists yet.
    fn eligible_to_train(&self, class: usize, level: u32, experience: u32) -> bool {
        match progression::exp_threshold(self.rules, class, level as usize) {
            Some(threshold) if threshold > 0 => experience as i64 >= threshold as i64,
            _ => false,
        }
    }

    /// `reclac_thief_skills` (`ovr026.cs:482-544`), the base/race/DEX
    /// composition only — item-driven scroll-learning overrides
    /// (`var_A`/`var_B`, level-override branches) are omitted: M3 has no
    /// inventory/item model yet, and those overrides only ever fire when a
    /// specific readied scroll item is present.
    fn skill_percentages(&self, race: usize, dex: u8, level: u32) -> [u8; 8] {
        let mut out = [0u8; 8];
        let thief_level = (level.clamp(1, 12)) as usize;
        for skill in 1..=8usize {
            let race_adj = thief_skills::race_adj(self.rules, race, skill) as i32;
            let base = thief_skills::base_chance(self.rules, thief_level, skill) as i32;
            if race_adj < 0 && base < race_adj.unsigned_abs() as i32 {
                out[skill - 1] = 0;
                continue;
            }
            let mut v = base + race_adj;
            if skill < 6 {
                v += thief_skills::dex_adj(self.rules, dex as usize, skill) as i32;
            }
            out[skill - 1] = v.max(0) as u8;
        }
        out
    }

    /// `sub_6A00F` (`ovr026.cs:55-166`), consuming the corrected
    /// [`spell_slots::cleric_spell_slots`]/[`spell_slots::mu_spell_slots`]
    /// (see their doc comments for the M3-session-3 correction), plus the
    /// WIS bonus block (`ovr026.cs:291-319`, via `calc_cleric_spells`'s
    /// `ResetSpellLevels=false` call at `:83`). The WIS bonus applies ONCE,
    /// immediately after the cleric contribution, because `SkillType`
    /// shares `ClassId`'s numbering and the loop processes class 0
    /// (cleric) before 3 (paladin) — paladin's later addition into the
    /// same slot array is therefore never WIS-bonused, an order-dependent
    /// quirk transcribed exactly. Ranger's low-level "MU-side" track
    /// (`unk_1A758` columns 3-4, `:137-140`) accumulates into
    /// `spellCastCount[2, sp_lvl-3]` — the SAME array indices 0-1 that
    /// magic-user's own slots use (`:139`) — hence `arcane[0..2]` sums
    /// both contributors, matching coab's shared array exactly.
    fn spell_slots(&self, classes: &[ClassLevel], wis: u8) -> EngineSpellSlots {
        let mut divine = [0u8; 5];
        let mut hybrid = [0u8; 3];
        let mut arcane = [0u8; 5];

        let cleric_lvl = Self::level_of(classes, class_id::CLERIC) as i32;
        if cleric_lvl > 0 {
            let base = spell_slots::cleric_spell_slots(self.rules, cleric_lvl);
            for i in 0..5 {
                divine[i] += base[i];
            }
            if wis > 12 && divine[0] > 0 {
                divine[0] += 1;
            }
            if wis > 13 && divine[0] > 0 {
                divine[0] += 1;
            }
            if wis > 14 && divine[1] > 0 {
                divine[1] += 1;
            }
            if wis > 15 && divine[1] > 0 {
                divine[1] += 1;
            }
            if wis > 16 && divine[2] > 0 {
                divine[2] += 1;
            }
            if wis > 17 && divine[3] > 0 {
                divine[3] += 1;
            }
        }

        let paladin_lvl = Self::level_of(classes, class_id::PALADIN) as i32;
        if paladin_lvl > 8 {
            let base = spell_slots::paladin_spell_slots(self.rules, paladin_lvl);
            for i in 0..5 {
                divine[i] += base[i];
            }
        }

        let ranger_lvl = Self::level_of(classes, class_id::RANGER) as i32;
        if ranger_lvl > 7 {
            let (druid_track, mu_track) = spell_slots::ranger_spell_slots(self.rules, ranger_lvl);
            for i in 0..3 {
                hybrid[i] += druid_track[i];
            }
            for i in 0..2 {
                arcane[i] += mu_track[i];
            }
        }

        let mu_lvl = Self::level_of(classes, class_id::MAGIC_USER) as i32;
        if mu_lvl > 0 {
            let base = spell_slots::mu_spell_slots(self.rules, mu_lvl);
            for i in 0..5 {
                arcane[i] += base[i];
            }
        }

        EngineSpellSlots {
            divine,
            hybrid,
            arcane,
        }
    }

    /// `SecondClassAllowed` (`ovr026.cs:558-599`). The `class_stats_min`
    /// table doubles as a prime-requisite marker here: a stat with a
    /// declared minimum of 9 or more is treated as "relevant" to that
    /// class, and ALL of the current class's relevant stats must exceed
    /// 14 (i.e. reach 15+) while ALL of the new class's relevant stats
    /// must exceed 16 (reach 17+) — the classic 1e dual-class thresholds,
    /// encoded as reused minimum-to-enter data rather than a separate
    /// table.
    fn dual_class_eligible(
        &self,
        current_class: usize,
        new_class: usize,
        stats: StatBlock,
        alignment: usize,
    ) -> bool {
        if current_class == new_class {
            return false;
        }
        let vals = [
            stats.str, stats.int, stats.wis, stats.dex, stats.con, stats.cha,
        ];

        let prime_reqs_clear = |class: usize, threshold: u8| -> bool {
            let min = creation_limits::class_stats_min(self.rules, class);
            let mins = [min.str, min.int, min.wis, min.dex, min.con, min.cha];
            for i in 0..6 {
                if mins[i] >= 9 && vals[i] <= threshold {
                    return false;
                }
            }
            true
        };

        if !prime_reqs_clear(current_class, 14) {
            return false;
        }
        if !prime_reqs_clear(new_class, 16) {
            return false;
        }
        self.alignment_admissible(new_class, alignment)
    }

    /// `StatValue.AgeEffects` (`Classes/Player.cs:66-75`), per stat, via
    /// [`creation_limits::total_age_effect`].
    fn age_effect_deltas(&self, race: usize, age: u16) -> [i32; 6] {
        core::array::from_fn(|i| creation_limits::total_age_effect(self.rules, i, race, age))
    }

    /// `strengthHitBonus` (`ovr025.cs:628-670`), via
    /// [`Self::strength_group`] (`player_strength_group`,
    /// `ovr025.cs:576-625`, the 18/xx exceptional bands). coab gates the
    /// whole formula on `player.field_125 != 0`; every player character
    /// has that flag set unconditionally at creation (`ovr018.cs:753`) and
    /// M3 has no monster model yet (the only entities `field_125` ever
    /// excludes), so this omits the gate rather than threading an always-
    /// true flag through the trait signature.
    fn strength_hit_bonus(&self, str_score: u8, str_exceptional: u8) -> i32 {
        match strength_group(str_score, str_exceptional) {
            1..=3 => -3,
            4 | 5 => -2,
            6 | 7 => -1,
            17..=19 => 1,
            20..=22 => 2,
            23..=25 => 3,
            26 | 27 => 4,
            g @ 28..=30 => g - 23,
            _ => 0,
        }
    }

    /// `strengthDamBonus` (`ovr025.cs:673-708`).
    fn strength_damage_bonus(&self, str_score: u8, str_exceptional: u8) -> i32 {
        match strength_group(str_score, str_exceptional) {
            1 | 2 => -2,
            3..=5 => -1,
            16 => 1,
            g @ 17..=19 => g - 16,
            g @ 20..=29 => g - 17,
            30 => 14,
            _ => 0,
        }
    }

    /// `DexAcBonus`/`stat_bonus` (`ovr025.cs:498-534`).
    fn dex_ac_bonus(&self, dex: u8) -> i32 {
        match dex {
            1..=3 => -4,
            4..=6 => dex as i32 - 7,
            15..=18 => dex as i32 - 14,
            19 | 20 => 4,
            21..=23 => 5,
            24 | 25 => 6,
            _ => 0,
        }
    }

    /// `DexReactionAdj` (`ovr025.cs:537-573`).
    fn dex_reaction_bonus(&self, dex: u8) -> i32 {
        match dex {
            0..=2 => -4,
            3..=5 => dex as i32 - 6,
            16..=18 => dex as i32 - 15,
            19 | 20 => 3,
            21..=23 => 4,
            24 | 25 => 5,
            _ => 0,
        }
    }

    /// `ConHitPointBonus`/`sub_647BE` (`ovr024.cs:782-831`).
    fn con_hp_total_bonus(
        &self,
        class: usize,
        class_level: u32,
        con: u8,
        multiclass_level: u32,
        ranger_old_level: u32,
    ) -> i32 {
        let max_hd = hp_hd::max_class_hit_dice(self.rules, class) as u32;
        let mut lvl = if max_hd <= class_level {
            max_hd.saturating_sub(1)
        } else {
            class_level
        } as i32;

        if class == class_id::RANGER
            && (multiclass_level == 0 || ranger_old_level == multiclass_level)
        {
            lvl += 1;
        }

        if matches!(
            class,
            class_id::FIGHTER | class_id::PALADIN | class_id::RANGER
        ) {
            match con {
                15..=19 => lvl * (con as i32 - 14),
                20 => lvl * 5,
                21..=23 => lvl * 6,
                24 | 25 => lvl * 7,
                _ => 0,
            }
        } else if con > 15 {
            lvl * 2
        } else if con == 15 {
            lvl
        } else {
            0
        }
    }

    /// `ovr018.cs:385-421`'s race switch, restricted to the genuinely
    /// racial affects (halfling/dwarf/gnome/elf/half_elf); the class-
    /// granted affects appearing later in the same function (paladin's
    /// `protection_from_evil`, ranger's `ranger_vs_giant`) are not
    /// racial and are out of this method's scope. Ids are coab's
    /// `Classes/Affect.cs` `Affects` enum discriminants.
    fn racial_traits(&self, race: usize) -> Vec<u16> {
        const CON_SAVING_BONUS: u16 = 0x61;
        const DWARF_VS_ORC: u16 = 0x1a;
        const DWARF_AND_GNOME_VS_GIANTS: u16 = 0x2f;
        const GNOME_VS_MAN_SIZED_GIANT: u16 = 0x12;
        const AFFECT_30: u16 = 0x30;
        const ELF_RESIST_SLEEP: u16 = 0x6b;
        const HALFELF_RESISTANCE: u16 = 0x7c;

        match race {
            1 => vec![CON_SAVING_BONUS, DWARF_VS_ORC, DWARF_AND_GNOME_VS_GIANTS], // dwarf
            2 => vec![ELF_RESIST_SLEEP],                                          // elf
            3 => vec![
                CON_SAVING_BONUS,
                GNOME_VS_MAN_SIZED_GIANT,
                DWARF_AND_GNOME_VS_GIANTS,
                AFFECT_30,
            ], // gnome
            4 => vec![HALFELF_RESISTANCE],                                        // half_elf
            5 => vec![CON_SAVING_BONUS],                                          // halfling
            _ => vec![], // monster, half_orc, human
        }
    }
}

/// `player_strength_group` (`ovr025.cs:576-625`), the 18/xx exceptional-
/// strength band mapping shared by [`Adnd1::strength_hit_bonus`] and
/// [`Adnd1::strength_damage_bonus`].
fn strength_group(str_score: u8, str_exceptional: u8) -> i32 {
    match str_score {
        0..=17 => str_score as i32,
        18 => match str_exceptional {
            0 => 18,
            1..=50 => 19,
            51..=75 => 20,
            76..=90 => 21,
            91..=99 => 22,
            _ => 23,
        },
        19..=25 => str_score as i32 + 5,
        _ => str_score as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedRolls {
        values: std::collections::VecDeque<u32>,
    }

    impl FixedRolls {
        fn new(values: impl IntoIterator<Item = u32>) -> Self {
            FixedRolls {
                values: values.into_iter().collect(),
            }
        }
    }

    impl Roller for FixedRolls {
        fn roll(&mut self, _size: u32, _count: u32) -> u32 {
            self.values
                .pop_front()
                .expect("test ran out of scripted rolls")
        }
    }

    #[test]
    fn class_admissible_matches_race_classes() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert!(flavor.class_admissible(7, 0)); // human cleric
        assert!(!flavor.class_admissible(0, 0)); // monster has no classes
    }

    #[test]
    fn alignment_admissible_matches_class_alignments() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert!(flavor.alignment_admissible(3, 0)); // paladin: LG only
        assert!(!flavor.alignment_admissible(3, 1));
    }

    #[test]
    fn sex_admissible_accepts_only_binary_values() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert!(flavor.sex_admissible(7, 0));
        assert!(flavor.sex_admissible(7, 1));
        assert!(!flavor.sex_admissible(7, 2));
    }

    #[test]
    fn clamp_stat_for_creation_reproduces_player_cs_order() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // dwarf (race1) male (sex0) STR range is 8..=18 (creation_limits
        // test fixture); a roll of 25 clamps down to 18, then paladin's
        // (class3) STR minimum of 12 doesn't raise it further.
        assert_eq!(
            flavor.clamp_stat_for_creation(AbilityStat::Str, 1, 0, 3, 25),
            18
        );
        // a roll of 3 clamps up to the race/sex min (8), then paladin's
        // class minimum (12) raises it further.
        assert_eq!(
            flavor.clamp_stat_for_creation(AbilityStat::Str, 1, 0, 3, 3),
            12
        );
    }

    #[test]
    fn roll_ability_score_takes_the_best_of_six_3d6_plus_1() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // six rolls of 3d6 (roller returns the pre-summed total per call)
        // -- best-of-6 must pick 15, then +1 = 16.
        let mut roller = FixedRolls::new([10, 15, 8, 12, 9, 14]);
        assert_eq!(flavor.roll_ability_score(&mut roller), 16);
    }

    #[test]
    fn exceptional_strength_eligible_requires_a_warrior_component() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert!(flavor.exceptional_strength_eligible(&[ClassLevel { class: 2, level: 1 }]));
        assert!(flavor.exceptional_strength_eligible(&[
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 4, level: 1 },
        ]));
        assert!(!flavor.exceptional_strength_eligible(&[ClassLevel { class: 5, level: 1 }]));
    }

    #[test]
    fn starting_age_single_class_rolls_the_dice() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // human (race7) cleric (class0): base 18, 1d4 -- roll returns 3.
        let mut roller = FixedRolls::new([3]);
        assert_eq!(
            flavor.starting_age(7, &[ClassLevel { class: 0, level: 1 }], &mut roller),
            21
        );
    }

    #[test]
    fn starting_age_multiclass_uses_the_unrolled_ceiling() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // human (race7) cleric+fighter (mc_c_f): representative row 0
        // (cleric), base 18 + 1*4 = 22, no roll consumed.
        let mut roller = FixedRolls::new([]);
        let classes = [
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 2, level: 1 },
        ];
        assert_eq!(flavor.starting_age(7, &classes, &mut roller), 22);
    }

    #[test]
    fn starting_age_cleric_magic_user_is_the_transcribed_coab_gap() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut roller = FixedRolls::new([]);
        let classes = [
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 5, level: 1 },
        ];
        assert_eq!(flavor.starting_age(7, &classes, &mut roller), 0);
    }

    #[test]
    fn starting_money_is_300_platinum() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert_eq!(flavor.starting_money(), 300);
    }

    #[test]
    fn starting_experience_single_class_is_25000() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert_eq!(
            flavor.starting_experience(&[ClassLevel { class: 2, level: 1 }]),
            25_000
        );
    }

    #[test]
    fn starting_experience_two_class_is_12500() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let classes = [
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 2, level: 1 },
        ];
        assert_eq!(flavor.starting_experience(&classes), 12_500);
    }

    #[test]
    fn starting_experience_mu_thief_quirk_is_8333_not_12500() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // mc_mu_t: only 2 classes, but coab gives it the 3-class rate.
        let classes = [
            ClassLevel { class: 5, level: 1 },
            ClassLevel { class: 6, level: 1 },
        ];
        assert_eq!(flavor.starting_experience(&classes), 8_333);
    }

    #[test]
    fn hp_die_roll_takes_the_higher_of_two_rolls() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // fighter (class2) level 1: d10, 1 die. First roll 4, second 7 --
        // must take 7.
        let mut roller = FixedRolls::new([4, 7]);
        let classes = [ClassLevel { class: 2, level: 1 }];
        assert_eq!(flavor.hp_die_roll(&classes, &[2], &mut roller), 7);
    }

    #[test]
    fn hp_die_roll_past_hit_dice_cap_uses_the_fixed_group_value() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // fighter's max_class_hit_dice is 10 (hp_hd fixture); level 10 is
        // at the cap -- fixed value 3, no dice consumed.
        let mut roller = FixedRolls::new([]);
        let classes = [ClassLevel {
            class: 2,
            level: 10,
        }];
        assert_eq!(flavor.hp_die_roll(&classes, &[2], &mut roller), 3);
    }

    #[test]
    fn hp_die_roll_untrained_class_contributes_nothing() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut roller = FixedRolls::new([5, 5]);
        let classes = [
            ClassLevel { class: 2, level: 1 },
            ClassLevel { class: 0, level: 1 },
        ];
        // only training class 0 (cleric): fighter's slot must not roll.
        assert_eq!(flavor.hp_die_roll(&classes, &[0], &mut roller), 5);
    }

    #[test]
    fn con_hp_adjustment_matches_the_con_minus_3_table_for_multiclass() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // cleric+fighter at CON 15 (con_hp_adj(15) = 1 per hp_hd fixture),
        // both below their hit-dice caps: 1 + 1 = 2. Neither is a
        // single-classed warrior, so no CON17+ extra applies.
        let classes = [
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 2, level: 1 },
        ];
        assert_eq!(flavor.con_hp_adjustment(&classes, 15), 2);
    }

    #[test]
    fn con_hp_adjustment_single_class_warrior_gets_the_17_plus_extra() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // single-class fighter, CON 18: con_hp_adj(18) contributes its
        // table value, plus the warrior extra of +2 for CON 18.
        let classes = [ClassLevel { class: 2, level: 1 }];
        let base = hp_hd::con_hp_adj(&rules, 18) as i32;
        assert_eq!(flavor.con_hp_adjustment(&classes, 18), base + 2);
    }

    #[test]
    fn con_hp_adjustment_ranger_level_1_doubles_the_running_total() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // cleric+ranger (mc_c_r) both level 1, CON 15: cleric contributes
        // con_hp_adj(15) first, then ranger's level==1 branch doubles the
        // WHOLE running total (including cleric's contribution).
        let classes = [
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 4, level: 1 },
        ];
        let base = hp_hd::con_hp_adj(&rules, 15) as i32;
        let expected = (base + base) * 2;
        assert_eq!(flavor.con_hp_adjustment(&classes, 15), expected);
    }

    #[test]
    fn hp_gain_at_creation_averages_rolled_and_max_across_classes() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let classes = [
            ClassLevel { class: 0, level: 1 },
            ClassLevel { class: 2, level: 1 },
        ];
        // cleric: d8 best-of-two(6,6)=6; fighter: d10 best-of-two(6,6)=6.
        let mut roller = FixedRolls::new([6, 6, 6, 6]);
        let hp = flavor.hp_gain_at_creation(&classes, 10, &mut roller);
        // CON 10 -> con_hp_adj is in the flat-zero band for both classes,
        // so con_adj sums to 0 and max = (12+0)/2 = 6; rolled = 12/2 = 6.
        assert_eq!(hp, CreationHp { rolled: 6, max: 6 });
    }

    #[test]
    fn max_hp_ceiling_is_the_class_count_average() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let classes = [ClassLevel { class: 2, level: 1 }];
        // fighter hp_calc: dice 10 lvl_bonus 0 (per hp_hd's fixture);
        // con_bonus_display(10) is in the flat-zero band -> (0+10)*1 = 10.
        assert_eq!(flavor.max_hp_ceiling(&classes, 10), 10);
    }

    #[test]
    fn eligible_to_train_requires_the_threshold_to_be_met() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert!(flavor.eligible_to_train(0, 1, 1501)); // cleric level1->2
        assert!(!flavor.eligible_to_train(0, 1, 1500));
        assert!(!flavor.eligible_to_train(1, 1, 999_999)); // druid: no table
    }

    #[test]
    fn skill_percentages_applies_race_and_dex_only_to_skills_1_through_5() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let out = flavor.skill_percentages(1, 20, 1); // dwarf, dex20, thief lvl1
        let base2 = thief_skills::base_chance(&rules, 1, 2) as i32;
        let race2 = thief_skills::race_adj(&rules, 1, 2) as i32;
        let dex2 = thief_skills::dex_adj(&rules, 20, 2) as i32;
        assert_eq!(out[1], (base2 + race2 + dex2).max(0) as u8);

        // skill 7 (>=6): no dex adjustment applied, only base + race.
        let base7 = thief_skills::base_chance(&rules, 1, 7) as i32;
        let race7 = thief_skills::race_adj(&rules, 1, 7) as i32;
        assert_eq!(out[6], (base7 + race7).max(0) as u8);
    }

    #[test]
    fn skill_percentages_floors_at_zero_when_race_penalty_exceeds_base() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // dwarf (race1) skill8: race_adj = -5, thief level1 base_chance
        // for skill8 is 0 -- 0 < abs(-5), so the floor-to-zero branch fires.
        let out = flavor.skill_percentages(1, 0, 1);
        assert_eq!(out[7], 0);
    }

    #[test]
    fn spell_slots_cleric_gets_wis_bonus_paladin_does_not() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // cleric level 4 alone at WIS 16: base [3,2,0,0,0] (see
        // spell_slots's corrected test), plus +1 (wis>12) and +1 (wis>13)
        // to col0, plus +1 (wis>14) and +1 (wis>15) to col1.
        let classes = [ClassLevel { class: 0, level: 4 }];
        let slots = flavor.spell_slots(&classes, 16);
        assert_eq!(slots.divine, [5, 4, 0, 0, 0]);
    }

    #[test]
    fn spell_slots_ranger_mu_track_shares_arcane_with_magic_user() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let classes = [
            ClassLevel { class: 4, level: 9 },
            ClassLevel { class: 5, level: 1 },
        ];
        let slots = flavor.spell_slots(&classes, 10);
        let (_, ranger_mu) = spell_slots::ranger_spell_slots(&rules, 9);
        let mu_base = spell_slots::mu_spell_slots(&rules, 1);
        assert_eq!(slots.arcane[0], ranger_mu[0] + mu_base[0]);
        assert_eq!(slots.arcane[1], ranger_mu[1] + mu_base[1]);
    }

    #[test]
    fn dual_class_eligible_requires_old_and_new_prime_reqs() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // fighter (class2) -> paladin (class3, LG only). class_stats_min
        // fixture: fighter's only >=9 column is STR (9); paladin's are
        // STR/INT/WIS/CON/CHA (12,9,13,9,17) -- DEX (0) is never relevant
        // to either.
        let strong_stats = StatBlock {
            str: 18,
            int: 18,
            wis: 18,
            dex: 10,
            con: 18,
            cha: 18,
            str_exceptional: 0,
        };
        assert!(flavor.dual_class_eligible(2, 3, strong_stats, 0));

        // paladin's CHA prime req needs >16; 15 fails it.
        let weak_stats = StatBlock {
            cha: 15,
            ..strong_stats
        };
        assert!(!flavor.dual_class_eligible(2, 3, weak_stats, 0));
    }

    #[test]
    fn dual_class_eligible_rejects_same_class() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let stats = StatBlock {
            str: 18,
            int: 18,
            wis: 18,
            dex: 18,
            con: 18,
            cha: 18,
            str_exceptional: 0,
        };
        assert!(!flavor.dual_class_eligible(2, 2, stats, 0));
    }

    #[test]
    fn age_effect_deltas_matches_human_str_at_45() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert_eq!(flavor.age_effect_deltas(7, 45)[0], 1);
    }

    #[test]
    fn strength_hit_and_damage_bonuses_cross_the_18_xx_bands() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // str 18 with no percentile (exceptional 0) is group 18 -> the
        // 17..=19 hit-bonus band -> +1.
        assert_eq!(flavor.strength_hit_bonus(18, 0), 1);
        // exceptional 51 (51..=75) is group 20 -> the 20..=22 band -> +2.
        assert_eq!(flavor.strength_hit_bonus(18, 51), 2);
        // exceptional 100 is group 23 -> damage's 20..=29 band -> 23-17=6.
        assert_eq!(flavor.strength_damage_bonus(18, 100), 6);
    }

    #[test]
    fn dex_ac_and_reaction_bonuses_match_ovr025() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert_eq!(flavor.dex_ac_bonus(18), 4);
        assert_eq!(flavor.dex_reaction_bonus(18), 3);
    }

    #[test]
    fn con_hp_total_bonus_matches_the_ranger_first_level_extra() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // ranger (class4) level1, CON17, multiclassLevel 0 -> lvl bumped
        // to 2 by the ranger-specific +1, then (17-14)*2 = 6.
        assert_eq!(flavor.con_hp_total_bonus(4, 1, 17, 0, 0), 6);
    }

    #[test]
    fn racial_traits_matches_the_race_switch() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        assert_eq!(flavor.racial_traits(7), Vec::<u16>::new()); // human
        assert_eq!(flavor.racial_traits(2), vec![0x6b]); // elf
        assert_eq!(flavor.racial_traits(1), vec![0x61, 0x1a, 0x2f]); // dwarf
    }
}
