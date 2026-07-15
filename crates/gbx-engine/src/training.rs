//! The training hall's level-up logic (`train_player`, `ovr018.cs:2189-2483`,
//! M3 step 6 deliverable 4). Pure logic over a [`Character`] + the rules pack;
//! the interactive screen (`crate::screens::Training`) wraps it.
//!
//! The numbers are pack-correct (D-RP3: prefer the binary-verified packs over
//! coab transliteration): eligibility from `progression::exp_threshold`, the
//! fee from the money model, HP from `flavor.hp_die_roll`/`con_hp_adjustment`,
//! THAC0 from `progression::thac0_stored`, the training-class masks from
//! `constants::class_training_mask`.
//!
//! Read for behavior from coab (D11). Deferrals (documented at their sites):
//! spell learning on a caster level-up (M5), `Limits.RaceClassLimit` race/
//! class caps (no race-limit pack yet), and the exact `spellCastCount[3,5]`
//! row layout (M5 — see [`recompute_spell_caps`]).

use crate::money;
use crate::party::Character;
use crate::rng::EngineRng;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::adnd1::{constants, progression};
use gbx_rules::flavor::{ClassLevel, Flavor, Roller};

/// The flat training fee, in gold (`ovr018.cs:2203`).
pub const TRAINING_FEE_GP: i64 = 1000;

/// A trainer that trains every class — the default when we're not modeling a
/// specific hall's `area2_ptr.training_class_mask` (which is set by the town's
/// ECL script; wiring that is M6 scope).
pub const TRAINS_ALL_CLASSES: u8 = 0xFF;

/// Why training was refused (`train_player`'s early returns).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainError {
    /// `health_status != Okay` (`ovr018.cs:2191`).
    NotConscious,
    /// `Money.GetGoldWorth() < 1000` (`ovr018.cs:2198`).
    NotEnoughGold,
    /// The trainer doesn't train any of this character's classes
    /// (`ovr018.cs:2296` "We don't train that class here").
    WrongClassHere,
    /// No class has enough XP to advance (`ovr018.cs:2304` "Not Enough
    /// Experience").
    NotEnoughExperience,
}

/// What a successful [`train`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainOutcome {
    /// (class id, new level) for each class that advanced.
    pub advanced: Vec<(usize, u8)>,
    /// The HP added to `hit_point_max`.
    pub hp_gained: u8,
}

/// A [`Roller`] over the engine's one PRNG (D9): `roll(size, count)` sums
/// `count` dice each on `1..=size`, drawn from [`EngineRng`]. As of M4 step 1
/// this is the binary-exact stream: `1 + random(size)` per die mirrors
/// `roll_dice`'s `Random(dice_size) + 1` (`ovr024.cs:586-598`), so RNG-stream
/// parity is live here, not deferred (the old H3/M4 deferral ends this session
/// — oracle-rig §6 ledger).
struct EngineRoller<'a>(&'a mut EngineRng);

impl Roller for EngineRoller<'_> {
    fn roll(&mut self, size: u32, count: u32) -> u32 {
        // `random(size)` is exclusive `0..size` and draws even at `size == 0`
        // (returns 0 -> die value 1), so the old `size.max(1)` underflow guard
        // is gone — the draw-always behavior is the faithful one.
        (0..count)
            .map(|_| self.0.random(size as u16) as u32 + 1)
            .sum()
    }
}

/// The character's live per-class levels as [`ClassLevel`]s (base classes with
/// a nonzero level).
fn class_levels(ch: &Character) -> Vec<ClassLevel> {
    ch.class_levels()
}

/// The `train_player` eligibility scan (`ovr018.cs:2217-2257`): the class
/// masks with a nonzero level, and the subset with enough XP to advance.
/// Returns `(classes_present_mask, classes_exp_eligible_mask)`.
fn eligibility_masks(ch: &Character, rules: &gbx_rules::pack::RuleSet, flavor: &Adnd1) -> (u8, u8) {
    let mut present = 0u8;
    let mut eligible = 0u8;
    for cl in class_levels(ch) {
        let mask = constants::class_training_mask(rules, cl.class);
        present |= mask;
        // eligible_to_train gates on progression::exp_threshold (FD: pack
        // wins over coab). RaceClassLimit caps are deferred (no pack yet).
        if flavor.eligible_to_train(cl.class, cl.level, ch.exp.max(0) as u32) {
            eligible |= mask;
        }
    }
    (present, eligible)
}

/// Which classes this character could train right now at `trainer_mask`
/// (enough XP *and* trained by this hall) — for the screen's "will become"
/// preview and the demo's eligibility probe.
pub fn trainable_classes(
    ch: &Character,
    rules: &gbx_rules::pack::RuleSet,
    trainer_mask: u8,
) -> Vec<usize> {
    let flavor = Adnd1::new(rules);
    let (_, eligible) = eligibility_masks(ch, rules, &flavor);
    let actual = eligible & trainer_mask;
    class_levels(ch)
        .into_iter()
        .filter(|cl| constants::class_training_mask(rules, cl.class) & actual != 0)
        .map(|cl| cl.class)
        .collect()
}

/// The descending display THAC0 = `0x3C - hitBonus`, recomputed after a
/// level-up by adding the pack-driven base-THAC0 delta to the stored current
/// value — so the strength/weapon bonus baked into `thac0_current` at import
/// is preserved (full `reclac_player_values` re-derivation is M4).
fn thac0_base_from(rules: &gbx_rules::pack::RuleSet, classes: &[ClassLevel]) -> i32 {
    classes
        .iter()
        .map(|cl| {
            progression::thac0_stored(rules, cl.class, (cl.level as usize).clamp(1, 12)) as i32
        })
        .max()
        .unwrap_or(0)
}

/// Recomputes the memorized-spell capacity caps from the packs
/// (`flavor.spell_slots`) after a level-up. **Provisional row mapping (M5):**
/// `cast_count[3,5]` is cleric/druid/mage per spell level 1-5; the pack's
/// `SpellSlots` splits into divine (cleric+paladin), a hybrid nature track
/// (ranger), and arcane (mage + ranger's MU track). We map divine → row 0,
/// arcane → row 2, hybrid → row 1's first three — the natural tradition
/// mapping. The exact original `spellCastCount` layout (and multi-class
/// interleaving) is Vancian-memorization work for M5; for a non-caster or a
/// sub-9 paladin (the bundled party) this is all zeros, a no-op. Documented
/// rather than guessed silently.
fn recompute_spell_caps(
    ch: &mut Character,
    rules: &gbx_rules::pack::RuleSet,
    classes: &[ClassLevel],
) {
    let flavor = Adnd1::new(rules);
    let slots = flavor.spell_slots(classes, ch.stats.wis.current);
    ch.magic.cast_count[0] = slots.divine;
    ch.magic.cast_count[2] = slots.arcane;
    let mut druid_row = [0u8; 5];
    druid_row[..3].copy_from_slice(&slots.hybrid);
    ch.magic.cast_count[1] = druid_row;
}

/// Trains `ch` one level (`train_player`, `ovr018.cs:2189-2483`) at
/// `trainer_mask` (use [`TRAINS_ALL_CLASSES`] for an unrestricted hall). On
/// success: pays the 1000 gp fee from the money model, advances every eligible
/// trained class by one level, recovers lost levels/HP, adds HP via the packs,
/// and updates THAC0 + spell caps. `rng` drives the HP roll (D9). The
/// character's `exp` is **not** consumed (coab leaves it, `ovr018.cs:2293`).
pub fn train(
    ch: &mut Character,
    rules: &gbx_rules::pack::RuleSet,
    rng: &mut EngineRng,
    trainer_mask: u8,
) -> Result<TrainOutcome, TrainError> {
    // Preconditions (ovr018.cs:2191-2205).
    if ch.status.health_status != 0 {
        return Err(TrainError::NotConscious);
    }
    if !money::can_afford(&ch.money, TRAINING_FEE_GP, rules) {
        return Err(TrainError::NotEnoughGold);
    }

    let flavor = Adnd1::new(rules);
    let (present_mask, eligible_mask) = eligibility_masks(ch, rules, &flavor);
    if present_mask & trainer_mask == 0 {
        return Err(TrainError::WrongClassHere);
    }
    let actual_mask = eligible_mask & trainer_mask;
    if actual_mask == 0 {
        return Err(TrainError::NotEnoughExperience);
    }

    let old_classes = class_levels(ch);
    let old_thac0_base = thac0_base_from(rules, &old_classes);

    // Pay the fee (ovr018.cs:2380).
    money::subtract_gold_worth(&mut ch.money, TRAINING_FEE_GP, rules);

    // Advance every eligible-and-trained class (ovr018.cs:2389-2405).
    let class_count = old_classes.len().max(1) as i32;
    let mut advanced = Vec::new();
    for cl in &old_classes {
        let mask = constants::class_training_mask(rules, cl.class);
        if mask & actual_mask != 0 {
            ch.class_level[cl.class] += 1;
            advanced.push((cl.class, ch.class_level[cl.class]));
            // Lost-level recovery (ovr018.cs:2398-2402): checked_div is None
            // only when lost_levels == 0, i.e. nothing to recover.
            if let Some(recover) = ch.lost_hp.checked_div(ch.lost_levels) {
                ch.lost_hp -= recover;
                ch.lost_levels -= 1;
            }
        }
    }

    let new_classes = class_levels(ch);

    // THAC0: add the pack base-delta to the current value (ovr026.cs:188-193's
    // max()-over-classes, applied incrementally).
    let new_thac0_base = thac0_base_from(rules, &new_classes);
    ch.combat.thac0_current =
        (ch.combat.thac0_current as i32 + (new_thac0_base - old_thac0_base)).clamp(0, 255) as u8;
    ch.combat.thac0_base = new_thac0_base as i8;

    // Spell caps (flavor.spell_slots) — see recompute_spell_caps's M5 note.
    recompute_spell_caps(ch, rules, &new_classes);

    // HP gain (ovr018.cs:2453-2481). Skipped past the multiclass HP cap.
    let hp_gained = if ch.hit_dice as u32 > ch.multiclass_level as u32 {
        let trained: Vec<usize> = advanced.iter().map(|(c, _)| *c).collect();
        let var_f = {
            let mut roller = EngineRoller(rng);
            flavor.hp_die_roll(&new_classes, &trained, &mut roller) as i32
        };
        // hit_point_rolled tracks the pre-CON roll average (ovr018.cs:2460-2467).
        let mut rolled_inc = var_f / class_count;
        if rolled_inc == 0 {
            rolled_inc = 1;
        }
        ch.hit_point_rolled = ch.hit_point_rolled.saturating_add(rolled_inc as u8);

        let con_adj = flavor.con_hp_adjustment(&new_classes, ch.stats.con.current);
        let mut max_inc = (var_f + con_adj) / class_count;
        if max_inc < 1 {
            max_inc = 1;
        }
        // Preserve current damage taken across the max bump (ovr018.cs:2478-2481).
        let hp_lost = ch.hit_point_max.saturating_sub(ch.hit_point_current);
        ch.hit_point_max = ch.hit_point_max.saturating_add(max_inc as u8);
        ch.hit_point_current = ch.hit_point_max.saturating_sub(hp_lost);
        max_inc as u8
    } else {
        0
    };

    Ok(TrainOutcome {
        advanced,
        hp_gained,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::Money;
    use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE};

    fn rules() -> gbx_rules::pack::RuleSet {
        gbx_rules::pack::RuleSet::load()
    }

    /// A single-class character at a given class/level/exp, with plenty of
    /// money and a nonzero hit-dice so HP gain runs.
    fn char_at(class: usize, level: u8, exp: i32) -> Character {
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0] = 4;
        bytes[1..5].copy_from_slice(b"Test");
        let rec = decode_char_record(&bytes).unwrap();
        let mut ch = crate::party::character_from_record(&rec, vec![], vec![]);
        ch.class_level = [0; 8];
        ch.class_level[class] = level;
        ch.exp = exp;
        ch.hit_dice = level.max(1);
        ch.multiclass_level = 0;
        ch.stats.con.current = 12; // no CON HP adjustment
        ch.hit_point_max = 30;
        ch.hit_point_current = 30;
        ch.money = Money {
            gold: 2000,
            ..Default::default()
        };
        ch.status.health_status = 0;
        ch
    }

    #[test]
    fn refuses_an_unconscious_character() {
        let r = rules();
        let mut ch = char_at(2, 1, 5000);
        ch.status.health_status = 5; // Dying
        let mut rng = EngineRng::new(1);
        assert_eq!(
            train(&mut ch, &r, &mut rng, TRAINS_ALL_CLASSES),
            Err(TrainError::NotConscious)
        );
    }

    #[test]
    fn refuses_without_the_fee() {
        let r = rules();
        let mut ch = char_at(2, 1, 5000);
        ch.money = Money::default(); // broke
        let mut rng = EngineRng::new(1);
        assert_eq!(
            train(&mut ch, &r, &mut rng, TRAINS_ALL_CLASSES),
            Err(TrainError::NotEnoughGold)
        );
    }

    #[test]
    fn refuses_without_enough_experience() {
        let r = rules();
        // Fighter level 1 needs 2001 XP to reach level 2; give 1000.
        let mut ch = char_at(2, 1, 1000);
        let mut rng = EngineRng::new(1);
        assert_eq!(
            train(&mut ch, &r, &mut rng, TRAINS_ALL_CLASSES),
            Err(TrainError::NotEnoughExperience)
        );
    }

    #[test]
    fn trains_a_fighter_with_pack_correct_numbers() {
        let r = rules();
        // Fighter level 1 → 2 needs 2001 XP (exp_thresholds_fighter[0]).
        let mut ch = char_at(2, 1, 3000);
        let gold_before = money::gold_worth(&ch.money, &r);
        let mut rng = EngineRng::new(42);
        let outcome = train(&mut ch, &r, &mut rng, TRAINS_ALL_CLASSES).expect("should train");

        assert_eq!(ch.class_level[2], 2, "fighter advanced to level 2");
        assert_eq!(outcome.advanced, vec![(2, 2)]);
        // Fee paid (1000 gp of worth gone).
        assert_eq!(money::gold_worth(&ch.money, &r), gold_before - 1000);
        // exp is not consumed (ovr018.cs:2293).
        assert_eq!(ch.exp, 3000);
        // HP gained: fighter is a d10 class (hp_hd), so 1..=10, CON 12 = +0.
        assert!(
            (1..=10).contains(&outcome.hp_gained),
            "fighter HP gain in 1..=10, got {}",
            outcome.hp_gained
        );
        assert_eq!(ch.hit_point_max, 30 + outcome.hp_gained as u16 as u8);
        // THAC0 base: fighter L1 stored 40 → L2 stored 40 (no change at this
        // step per the fighter row), so thac0_current is unchanged here.
        assert_eq!(ch.combat.thac0_base, 40);
    }

    #[test]
    fn thac0_improves_when_the_class_table_steps() {
        let r = rules();
        // Fighter L3→L4: stored 42 → 43 (fighter row [39,40,40,42,43,...]),
        // a +1 improvement to hitBonus. Needs L3→L4 XP = 8001.
        let mut ch = char_at(2, 3, 10000);
        ch.combat.thac0_current = 47;
        let mut rng = EngineRng::new(7);
        train(&mut ch, &r, &mut rng, TRAINS_ALL_CLASSES).expect("should train");
        assert_eq!(ch.class_level[2], 4);
        assert_eq!(ch.combat.thac0_base, 43);
        assert_eq!(
            ch.combat.thac0_current, 48,
            "hitBonus +1 as the table steps"
        );
    }

    #[test]
    fn trainable_classes_reflects_xp_eligibility() {
        let r = rules();
        let eligible = char_at(2, 1, 3000); // fighter with enough XP
        assert_eq!(
            trainable_classes(&eligible, &r, TRAINS_ALL_CLASSES),
            vec![2]
        );
        let not_eligible = char_at(2, 1, 100); // too little XP
        assert!(trainable_classes(&not_eligible, &r, TRAINS_ALL_CLASSES).is_empty());
    }
}
