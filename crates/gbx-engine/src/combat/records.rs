use super::*;
use gbx_formats::save_orig::{decode_char_record, CharRecord, SaveParseError};

// --- the combat entry-state replay harness (H4, D-OR5(b)) ------------------

/// One combatant of a captured combat **entry-state snapshot** (`combat_entry`,
/// D-OR5(b)): its team, grid position, and the raw `0x1A6` record bytes (a full
/// `Player`/monster record). The replay harness decodes the record and places
/// the combatant **at `pos`** — the snapshot supplies the position, so
/// `PlaceCombatants` is deliberately *not* run in the replay path (one fewer
/// variable between our draw stream and the capture's).
pub struct RecordCombatant<'a> {
    pub team: Team,
    pub pos: GridPos,
    /// The full `0x1A6` combat record (`decode_char_record`'s input).
    pub record: &'a [u8],
}

/// Map one decoded `0x1A6` record onto a combat [`Combatant`] for a faithful
/// replay (H4). Built on top of [`Combatant::new_melee`] (the accepted real-fight
/// constructor) with the record-derived fields patched in. **Which record field
/// feeds which combat input** (the load-bearing mapping — every one of these is
/// read by some part of the draw stream, except where noted):
///
/// - **team / pos** — from the snapshot, not the record.
/// - **npc** — `control_morale@0xf7 >= 0x80` (gates the per-step morale d100 and
///   the `FleeCheck` block; a PC short-circuits both).
/// - **hp** — `hit_point_current@0x1a4` / `hit_point_max@0x78` (deaths change the
///   live counts → who is targetable → the draw stream; `enemy_health_pct` reads
///   the monster team's cur/max for morale).
/// - **ac** — raw `ac@0x19a` (the to-hit compare target; whether an attack hits
///   decides whether damage dice are rolled).
/// - **hit_bonus** — `hitBonus@0x199` (the current THAC0-derived to-hit number —
///   the field [`Combatant::hit_bonus`] itself names).
/// - **hit_dice** — `hit_dice@0xe5` (the `TrySweepAttack` 0-HD gate).
/// - **movement** — `movement@0x1a5` → [`calc_moves`] (half-move budget → how far
///   an actor steps → per-step monster d100 count).
/// - **reaction_adj** — `DexReactionAdj(stats2.Dex.full)` via the [`Flavor`]
///   (`full` == the record's `original` DEX byte); the initiative `delay = clamp(d6
///   + reaction_adj)`, so it drives selection order.
/// - **attacks_count** — `attacksCount@0x11c` (`attack_profile_base[0]`) →
///   [`this_round_action_count`] → `attack1_left` → number of to-hit d20s/round.
/// - **melee dice** — attack-1 `dice_count@0x19e` / `dice_size@0x1a0` /
///   `dmg_bonus@0x1a2` (`attack_profile_current[2/4/6]`). The readied-weapon
///   `ItemData` dice are not decoded yet (FD-29); the record's carried attack-1
///   dice are used directly, per the session brief.
///
/// **`field_186@0x186` (the save bonus) is intentionally not threaded:** the
/// [`Combatant`] model has no save-bonus cell because saving throws only fire for
/// spell/affect effects (stubbed to M5). A plain-melee replay rolls no saves, so
/// `field_186` feeds no draw here — it becomes load-bearing only once effects land.
fn combatant_from_record(
    id: usize,
    team: Team,
    pos: GridPos,
    rec: &CharRecord,
    raw: &[u8],
    flavor: &dyn Flavor,
) -> Combatant {
    let npc = rec.control_morale >= 0x80;
    let dice = (
        rec.attack_profile_current[2], // a1 dice_count @0x19e
        rec.attack_profile_current[4], // a1 dice_size  @0x1a0
        rec.attack_profile_current[6], // a1 dmg_bonus  @0x1a2
    );
    // stats2.Dex.full == the record's `original` DEX byte (coab reads .full).
    let reaction_adj = flavor.dex_reaction_bonus(rec.stats.dex.original) as i8;

    let mut c = Combatant::new_melee(
        id,
        team,
        npc,
        pos,
        rec.hit_point_current as i32,
        rec.ac as u8,
        rec.hit_bonus as i32,
        rec.movement as i32,
        dice,
        0, // delay — CalculateInitiative sets it each round
        1, // attack1_left — CalculateInitiative overwrites it from attacks_count
    );
    // Fields new_melee cannot carry from the record: max HP (may differ from
    // current), real hit dice, the DEX reaction adj, and the base attack count.
    c.hp_max = rec.hit_point_max as i32;
    c.ac_behind = rec.ac_behind as u8; // @0x19b — the behind-AC index target
    c.hit_dice = rec.hit_dice;
    c.reaction_adj = reaction_adj;
    c.attacks_count = rec.attack_profile_base[0]; // attacksCount @0x11c
                                                  // §15 bug #4 (the mage hold): class @0x75 and field_159 @0x159 (a 4-byte
                                                  // runtime far-pointer; null == all-zero). The QuickFight approach guards a
                                                  // non-fleeing class-5 (pure Magic-User) with a null field_159.
    c.class = rec.class;
    c.field_159_null = match raw.get(0x159..0x15D) {
        Some(p) => p.iter().all(|&b| b == 0),
        None => true, // full 0x1A6 records always carry it; missing → treat as null
    };
    // `sub_3560B`'s memorized candidate list (doc §41.1). The collection loop
    // (`ovr010:062A-065D`) reads `record[0x1E + i]` for i = 1..=0x53 (bytes
    // 0x1F..0x71): slot 0 @0x1E is NEVER read, and the list packs from the BACK
    // (`SpellList.Save` fills from index 83 down — the first memorized spell
    // lands @0x71; doc §33's save-diff). ANY non-zero byte collects (`cmp
    // ..,0`/`jbe` ≡ `jz` @`ovr010:0637-063C`), IN slot order, so high-bit
    // "learning" entries collect too — coab's `LearntList()` filters them, a
    // cited coab≠binary nuance no capture exercises. caster-bar PHILIPPE →
    // `[0x0F]`; bar-fists-2 PHILIPPE → `[0x0F, 0x0F]`.
    c.memorized_list = rec.spell_list[1..]
        .iter()
        .copied()
        .filter(|&b| b != 0)
        .collect();
    // `spellMaxTargetCount` inputs (doc §41.2, `ovr025.cs:1342`): the caster's
    // MagicUser/Ranger skill levels (`SkillLevel`, `Player.cs:494`, same shape as
    // `skill_level_thief`) and the no-caster fallback predicate
    // (`ovr025.cs:1351`). PHILIPPE: MU 5, single-class → SkillLevel(MU) 5,
    // castingLvl 5.
    c.skill_level_magic_user = skill_level(rec, SKILL_MAGIC_USER);
    c.skill_level_ranger = skill_level(rec, SKILL_RANGER);
    c.caster_no_class = rec.class_level[SKILL_CLERIC] == 0
        && rec.class_level[SKILL_MAGIC_USER] == 0
        && rec.class_level[SKILL_PALADIN] < 9
        && rec.class_level[SKILL_RANGER] < 8;
    // §26.1 the downed-PC ladder: the entry `health_status@0x195` (okey in a
    // fresh combat snapshot). `bleeding` starts 0; `damage_player` seeds it.
    c.health_status = decode_health_status(rec.health_status);
    c.bleeding = 0;
    // §28 the faithful FleeCheck ladder: the raw `control_morale@0xF7` (for the
    // per-actor morale reseed `(control_morale & 0x7F) << 1`) and `Int@0x13`
    // (`stats2.Int.original` — the `.original`/`.full` byte, as DEX above; the
    // surrender branch's `Int > 5` gate). `npc` already folds control_morale.
    c.control_morale = rec.control_morale;
    c.int_score = rec.stats.int.original;
    // §34 the armed/ranged slice. The saved readied attack-1 profile (for the
    // cornered unready→re-ready swap, §34.5) is the record's decoded `dice`;
    // the attack-2 profile is @0x19F/0x1A1/0x1A3 (idx-2 damage, §34.6 — all
    // zero in this party); `baseHalfMoves`@0x11D folds into `attack2_left`
    // (§34.3); `field_DE`@0xde drives the large-target and backstab size gates;
    // and `SkillLevel(Thief)` is precomputed for the backstab multiplier (§34.6).
    c.entry_dice = dice;
    c.attack2_dice = (
        rec.attack_profile_current[3], // a2 dice_count @0x19f
        rec.attack_profile_current[5], // a2 dice_size  @0x1a1
        rec.attack_profile_current[7], // a2 dmg_bonus  @0x1a3
    );
    c.base_half_moves = rec.attack_profile_base[1]; // baseHalfMoves @0x11d
    c.base_dice = (
        rec.attack_profile_base[2], // attack1_DiceCountBase @0x11e
        rec.attack_profile_base[4], // attack1_DiceSizeBase  @0x120
        rec.attack_profile_base[6], // attack1_DamageBonusBase @0x122
    );
    c.field_de = rec.field_de; // @0xde
    c.thief_skill_level = skill_level(rec, SKILL_THIEF);
    c
}

// `SkillType` (`Classes/Enums.cs:57`) — the `ClassLevel`/`ClassLevelsOld` index.
const SKILL_CLERIC: usize = 0;
const SKILL_PALADIN: usize = 3;
const SKILL_RANGER: usize = 4;
const SKILL_MAGIC_USER: usize = 5;
const SKILL_THIEF: usize = 6;

/// `SkillLevel(skill)` (coab `Player.cs:492`): `ClassLevel[skill] +
/// ClassLevelsOld[skill] * DualClassExceedsPreviousLevel()`. The binary reads
/// `rec[0x109 + skill]` (`ClassLevel[skill]`) and `rec[0x111 + skill]`
/// (`ClassLevelsOld[skill]`) and multiplies the latter by `sub_6B3D1`
/// (`ovr014:01F9-021F`, verified this session). `DualClassExceedsPreviousLevel`
/// (`sub_6B3D1`, `Player.cs:800`) = `DuelClassCurrentLevel() > multiclassLevel ?
/// 1 : 0`, where `DuelClassCurrentLevel` (`Player.cs:812`) returns 0 for
/// non-humans, else the first non-zero `ClassLevel[0..7]` (or `ClassLevel[7]` if
/// `0..7` are all 0). The dual term is class-independent, so it is computed once
/// per record. Constant during a fight — precomputed at decode.
fn skill_level(rec: &CharRecord, skill: usize) -> i32 {
    const HUMAN: u8 = 7; // Race.human (Classes/Enums.cs:54)
    let dual = {
        let current = if rec.race != HUMAN {
            0
        } else {
            let mut i = 0;
            while i < 7 && rec.class_level[i] == 0 {
                i += 1;
            }
            rec.class_level[i] as i32
        };
        i32::from(current > rec.multiclass_level as i32)
    };
    rec.class_level[skill] as i32 + rec.class_levels_old[skill] as i32 * dual
}

/// Build a [`CombatState`] from a captured combat entry-state snapshot (H4,
/// D-OR5(b)) — the replay harness. Decodes each `0x1A6` record, maps it onto a
/// [`Combatant`] ([`combatant_from_record`]), and assembles the roster **in the
/// snapshot's order** (== `TeamList` == the initiative draw order — load-bearing)
/// **at the snapshot's positions** (no `PlaceCombatants`). The result is a full
/// melee fight ([`CombatState::new`], `TurnDriver::MeleeAi`) over `map`.
///
/// The caller owns the RNG: seed a [`EngineRng`] with the snapshot's `rng_state`,
/// attach an `RngSink`, then drive `state.step(&mut rng)` (or `run_combat`) to
/// `Ended`. A record that fails to decode is a loud [`SaveParseError`] (tooling
/// input, never silently tolerated).
pub fn combat_state_from_records(
    entries: &[RecordCombatant],
    map: CombatMap,
    flavor: &dyn Flavor,
) -> Result<CombatState, SaveParseError> {
    let mut fighters = Vec::with_capacity(entries.len());
    for (id, e) in entries.iter().enumerate() {
        let rec = decode_char_record(e.record)?;
        fighters.push(combatant_from_record(
            id, e.team, e.pos, &rec, e.record, flavor,
        ));
    }
    Ok(CombatState::new(map, fighters))
}
