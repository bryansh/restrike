//! D-RP5: the flavor-trait seam. `gbx-rules` defines this trait in
//! engine-vocabulary terms only — "may this character enter this class",
//! "hp gained on level-up" — never AD&D nouns (THAC0, cleric, paladin); a
//! flavor implementation (`adnd1`, later `xxvc`) supplies the meaning.
//!
//! Input/output types here are minimal value-like structs, not the engine's
//! future party/character model — that model wraps these later and is out
//! of scope for this module.

/// A character's six ability scores, plus the raw 1..=100 exceptional-
/// strength percentile (only meaningful when `str` is at a flavor's
/// non-exceptional cap — `adnd1` reads it at `str == 18`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatBlock {
    pub str: u8,
    pub str_exceptional: u8,
    pub int: u8,
    pub wis: u8,
    pub dex: u8,
    pub con: u8,
    pub cha: u8,
}

/// Which ability score an operation concerns — six generic RPG abilities
/// shared by any flavor built on this seam, not AD&D-specific vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityStat {
    Str,
    Int,
    Wis,
    Dex,
    Con,
    Cha,
}

/// One class's level for a (possibly multi-classed) character. `class` is
/// an opaque flavor-defined id — `adnd1` uses coab's base `ClassId` order
/// (0..=7); the trait itself attaches no meaning to the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassLevel {
    pub class: usize,
    pub level: u32,
}

/// Hit points determined at character creation — the rolled total (used for
/// display/reroll comparison) and the CON-adjusted max actually granted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreationHp {
    pub rolled: u32,
    pub max: u32,
}

/// Accumulated spell slots by casting tradition. Traditions are generic
/// engine vocabulary (not AD&D spell-class names): `divine` covers any
/// class sharing one slot pool from prayer-style casting (adnd1: cleric +
/// paladin), `arcane` any class sharing one slot pool from study-style
/// casting (adnd1: magic-user, plus the upper end of a hybrid caster's
/// pool), `hybrid` a third, independent pool for classes that split their
/// casting across two traditions (adnd1: ranger's low-level nature slots).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpellSlots {
    pub divine: [u8; 5],
    pub hybrid: [u8; 3],
    pub arcane: [u8; 5],
}

/// A source of dice rolls. The engine adapts its single seedable PRNG (D9)
/// to this trait; `gbx-rules` never depends on `gbx-vm`, so this stays a
/// small local abstraction rather than pulling in the engine's real RNG
/// type.
pub trait Roller {
    /// Sum of `count` dice, each uniform on `1..=size`.
    fn roll(&mut self, size: u32, count: u32) -> u32;
}

/// D-RP5's flavor-trait seam. M3's slice: creation legality, stat rolling,
/// starting age/money/XP, HP determination, XP-to-train and level-up
/// eligibility, thief skill recalculation, spell-slot accumulation, dual-
/// class eligibility, aging, and the ability-modifier chains the character
/// sheet needs. Combat-facing methods (to-hit, saves, turn undead) stay
/// table plumbing in `pack`'s typed accessors until M4's roll semantics.
pub trait Flavor {
    /// May a character of `race` select `class` at creation?
    fn class_admissible(&self, race: usize, class: usize) -> bool;

    /// May a character of `class` select `alignment` at creation?
    fn alignment_admissible(&self, class: usize, alignment: usize) -> bool;

    /// May a character of `race` select `sex`? (0 or 1 — the flavor
    /// attaches no further meaning here; `sex` only feeds stat-range
    /// lookups elsewhere.)
    fn sex_admissible(&self, race: usize, sex: usize) -> bool;

    /// Clamps a rolled ability score into the legal range for `race`/`sex`,
    /// then raises it to `class`'s minimum — the creation-time enforcement
    /// order, applied per stat.
    fn clamp_stat_for_creation(
        &self,
        stat: AbilityStat,
        race: usize,
        sex: usize,
        class: usize,
        value: u8,
    ) -> u8;

    /// Rolls one ability score: best-of-N rerolls of the flavor's base
    /// dice formula.
    fn roll_ability_score(&self, roller: &mut dyn Roller) -> u8;

    /// True if `classes` qualifies for an exceptional (fractional-above-18)
    /// strength roll.
    fn exceptional_strength_eligible(&self, classes: &[ClassLevel]) -> bool;

    /// Rolls the 1..=100 exceptional-strength percentile.
    fn roll_exceptional_strength(&self, roller: &mut dyn Roller) -> u8;

    /// Rolls a starting age for a character of `race` entering `classes`.
    fn starting_age(&self, race: usize, classes: &[ClassLevel], roller: &mut dyn Roller) -> u16;

    /// Starting money, in the flavor's largest coin denomination.
    fn starting_money(&self) -> u32;

    /// Starting experience for a character entering `classes` (multi-class
    /// combinations may start with less than a single-class character).
    fn starting_experience(&self, classes: &[ClassLevel]) -> u32;

    /// Rolls the hit-die gain for the classes named in `trained` (a subset
    /// of `classes`) — used both for every active class at creation and for
    /// a single class being trained at level-up.
    fn hp_die_roll(
        &self,
        classes: &[ClassLevel],
        trained: &[usize],
        roller: &mut dyn Roller,
    ) -> u32;

    /// The CON adjustment added to a hit-point roll for the given active
    /// classes.
    fn con_hp_adjustment(&self, classes: &[ClassLevel], con: u8) -> i32;

    /// The full creation-time HP determination: roll every active class,
    /// apply the CON adjustment, and average across classes.
    fn hp_gain_at_creation(
        &self,
        classes: &[ClassLevel],
        con: u8,
        roller: &mut dyn Roller,
    ) -> CreationHp;

    /// An alternate, non-rolled maximum-HP estimate used as a display
    /// ceiling.
    fn max_hp_ceiling(&self, classes: &[ClassLevel], con: u8) -> u32;

    /// True if a character of `class` at `level` with `experience` may
    /// train to the next level.
    fn eligible_to_train(&self, class: usize, level: u32, experience: u32) -> bool;

    /// Recalculates the skill-array percentage chances for a skill-based
    /// class (adnd1: thief) at `level`, for a character of `race` and `dex`.
    fn skill_percentages(&self, race: usize, dex: u8, level: u32) -> [u8; 8];

    /// Accumulated spell slots for a (possibly multi-classed) character.
    fn spell_slots(&self, classes: &[ClassLevel], wis: u8) -> SpellSlots;

    /// True if a single-classed character may add `new_class` as a second
    /// class.
    fn dual_class_eligible(
        &self,
        current_class: usize,
        new_class: usize,
        stats: StatBlock,
        alignment: usize,
    ) -> bool;

    /// Per-stat ability-score deltas applied once a character of `race`
    /// crosses `age`'s aging brackets — order matches [`StatBlock`]'s first
    /// six fields (str, int, wis, dex, con, cha).
    fn age_effect_deltas(&self, race: usize, age: u16) -> [i32; 6];

    /// To-hit bonus from strength.
    fn strength_hit_bonus(&self, str_score: u8, str_exceptional: u8) -> i32;

    /// Damage bonus from strength.
    fn strength_damage_bonus(&self, str_score: u8, str_exceptional: u8) -> i32;

    /// AC bonus from dexterity (more negative is better, matching the
    /// flavor's AC convention).
    fn dex_ac_bonus(&self, dex: u8) -> i32;

    /// Reaction/initiative adjustment from dexterity.
    fn dex_reaction_bonus(&self, dex: u8) -> i32;

    /// Total accumulated HP bonus from CON for one class's levels, as shown
    /// on the character sheet (distinct from [`Flavor::con_hp_adjustment`]'s
    /// per-roll adjustment — this is a display total, not a per-level
    /// delta). `multiclass_level`/`ranger_old_level` disambiguate a
    /// class-specific quirk in the source formula.
    fn con_hp_total_bonus(
        &self,
        class: usize,
        class_level: u32,
        con: u8,
        multiclass_level: u32,
        ranger_old_level: u32,
    ) -> i32;

    /// Opaque racial-trait ids granted at creation (the future engine
    /// affect/status system interprets them; this trait only enumerates
    /// which ones a race grants).
    fn racial_traits(&self, race: usize) -> Vec<u16>;
}
