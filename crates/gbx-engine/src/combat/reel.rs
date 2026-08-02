//! **The reel's input contract** (`docs/design/combat-visualizer.md` D-CV1
//! item 2, §4 M6a) — everything a host must hand over to replay one captured
//! fight, and the one place a `CombatState` is built from it.
//!
//! D-CV1 pins the dependency direction as a settled door: `gbx-oracle` →
//! `gbx-engine`, never the reverse. So `.gbxtrace` parsing stays in the oracle
//! and the **assembly product** lands here, engine-side, as plain data:
//! [`ReelInput`]. The oracle's `replay` module builds one from a capture (roster
//! records, terrain, the knob/icon sidecar); [`Engine::new_reel`] consumes it.
//!
//! [`Engine::new_reel`]: crate::engine::Engine::new_reel
//!
//! **One assembly path, not two.** The harnesses (`h4_replay`, `h4_turndiff`,
//! the frontier guard) and the reel all reach a live `CombatState` through
//! [`build_state`] — the same record decode, the same knob application, the same
//! loadout install, in the same order. That is what makes "the reel is h4_replay
//! with pixels" a structural claim rather than a hope: if the reel's fight ever
//! diverged from the harness's, the two would have had to build different
//! states, and there is only one builder.
//!
//! Nothing in this module draws, ticks, or renders — see [`crate::combat::scene`]
//! for the presenter and `engine.rs` for the host that drives both.

use super::{combat_state_from_records, CombatMap, CombatState, GridPos, Loadout, RecordCombatant};
use gbx_formats::affects::AffectRecord;
use gbx_formats::items::ItemDataTable;
use gbx_formats::save_orig::SaveParseError;
use gbx_rules::flavor::Flavor;
use std::collections::BTreeMap;

pub use super::Team;

/// The full `0x1A6` combat record every roster slot carries (`Player`'s
/// on-disk size — the oracle's `COMBAT_RECORD_LEN` names the same number from
/// the wire side, deliberately without a shared dependency).
pub const COMBAT_RECORD_LEN: usize = 0x1A6;

/// Open-floor fallback tile (`0x17` = passable floor, move cost 1).
///
/// Used only when a capture predates the `combat_entry.terrain` field. Terrain
/// is load-bearing for movement (doc §14), so every modern capture builds its
/// map from the captured ground grid instead.
pub const FALLBACK_FLOOR: u8 = 0x17;

/// Icon slots `0..8` are the party; `8..` are assigned per monster type at
/// LOADMONSTER time (`gbl.monster_icon_id` starts at 8, `ovr008.cs:98` /
/// `ovr003.cs:763`, and `CMD_LoadMonster` stamps it onto every copy before
/// incrementing, `ovr003.cs:259-293`).
pub const MONSTER_FIRST_ICON_SLOT: u8 = 8;

/// One roster slot's replay input: where it stood, whose side it was on, and
/// the bytes it was.
///
/// Owned rather than borrowed (unlike [`RecordCombatant`]) because a
/// [`ReelInput`] outlives the capture text it was parsed from — the reel host
/// keeps it for the length of the fight.
#[derive(Debug, Clone, PartialEq)]
pub struct ReelCombatant {
    pub team: Team,
    pub pos: GridPos,
    /// The full `0x1A6` combat record.
    pub record: Vec<u8>,
    /// The live affect chain as the staging hook captured it (doc §44.2/§47.7),
    /// order preserved — find-FIRST is order-observable.
    pub affects: Vec<AffectRecord>,
}

/// The per-fight input knobs a capture cannot always carry.
///
/// Three of these (`map_direction`, `area_field_58c`, `area_field_6e4`) ARE
/// emitted by modern staging hooks and the capture's value wins where present;
/// the rest (`auto_cast*`, `continue_battle`) are recordings of keypresses that
/// never made it into the snapshot and stay hand-pinned per capture (doc §38,
/// §48). Defaults are the documented pre-capture fallbacks the replay harnesses
/// have always used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReelKnobs {
    /// `gbl.mapDirection` — the flee HEADING (doc §29). Default 2 (E), the
    /// provisional geometry-matched value, capture-confirmed for the bar.
    pub map_direction: u8,
    /// `gbl.AutoPCsCastMagic` at entry (`BattleSetup` resets it false,
    /// `ovr011.cs:1186`).
    pub auto_cast: bool,
    /// Mid-fight '2' presses as 0-based global turn ordinals (doc §38).
    pub auto_cast_toggles: Vec<u32>,
    /// "Continue Battle:" occurrences answered 'Y', 0-based (doc §48).
    pub continue_battle: Vec<u16>,
    /// `area2.field_58C` — the `FleeCheck_001` gate-2 morale threshold (doc
    /// §28). Default 99, the measured bar value under which the natural rout
    /// is mathematically impossible.
    pub area_field_58c: i32,
    /// `area2.field_6E4` — the PARTY-gated area movement modifier, in the
    /// ENGINE domain (coab≠binary #21, doc §45; the §47 byte-bridge converts
    /// a capture's raw word).
    pub area_field_6e4: i32,
}

impl Default for ReelKnobs {
    fn default() -> Self {
        ReelKnobs {
            map_direction: 2,
            auto_cast: false,
            auto_cast_toggles: Vec::new(),
            continue_battle: Vec::new(),
            area_field_58c: 99,
            area_field_6e4: 0,
        }
    }
}

/// The art pins a capture cannot supply (doc §2's gap table, §4 M6a).
///
/// `combat_entry` carries no monster CPIC ids — LOADMONSTER's third operand is
/// read on the live path only (`format.rs:430-437`) — so the 15 closed captures
/// hand-pin them exactly as their loadouts are pinned. Party icons need no pin:
/// `CHEAD[head_icon]` + `CBODY[weapon_icon]` + `icon_colours` all ride in the
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReelArt {
    /// The `CPIC{area}.DAX` suffix — `gbl.game_area` at the fight
    /// (`chead_cbody_comspr_icon`'s `fileText + game_area`, `ovr034.cs:79`).
    pub cpic_area: u8,
    /// Icon slot (`>= 8`) → the CPIC block LOADMONSTER put there.
    pub monster_blocks: BTreeMap<u8, u8>,
    /// `SetupGroundTiles`' fork (`ovr011.cs:757-768`): DUNGCOM or WILDCOM.
    pub in_dungeon: bool,
}

impl Default for ReelArt {
    fn default() -> Self {
        ReelArt {
            cpic_area: 2,
            monster_blocks: BTreeMap::new(),
            in_dungeon: true,
        }
    }
}

/// One draw the capture recorded, as the reel checks it: `(before, after)`
/// always, plus the `Random(n)` operand when the capture carried one
/// (`ss_sp_words[3]`).
///
/// The equality surface is the harnesses' (doc §14's lesson: `(before, after)`
/// alone is draw-COUNT equality for a pure LCG, so the operand is part of the
/// comparison whenever both sides have it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpectedDraw {
    pub before: u32,
    pub after: u32,
    pub operand: Option<u16>,
}

/// Everything [`Engine::new_reel`] needs to replay one captured fight with
/// pixels.
///
/// [`Engine::new_reel`]: crate::engine::Engine::new_reel
#[derive(Debug, Clone)]
pub struct ReelInput {
    /// A human label for diagnostics — the capture basename.
    pub label: String,
    /// The replay seed (`combat_entry.rng_state`).
    pub rng_state: u32,
    /// The roster in `TeamList` (== initiative draw) order. Verbatim.
    pub combatants: Vec<ReelCombatant>,
    /// The captured ground grid (`mapToBackGroundTile`, 50×25 row-major);
    /// `None` falls back to [`FALLBACK_FLOOR`].
    pub terrain: Option<Vec<u8>>,
    pub knobs: ReelKnobs,
    /// Per-capture ranged loadouts by roster index (doc §34.1/§45/§48).
    pub loadouts: Vec<(usize, Loadout)>,
    /// The resident `ITEMS` table — required by any capture with a loadout.
    pub item_data: Option<ItemDataTable>,
    pub art: ReelArt,
    /// `game_speed_var` for playback (`seg001.cs:274`'s default is 4). Affects
    /// how long beats are held, never what a frame contains (D-CV3).
    pub game_speed: u8,
    /// D-CV3's host tick multiplier: how many scene ticks one engine tick
    /// advances. `1` is the faithful rate; a bigger number runs the reel fast
    /// without changing a single frame.
    pub tick_multiplier: u32,
    /// The capture's post-`combat_entry` draw stream. The reel asserts equality
    /// against this **live, while rendering** (D-CV1: "the reel is h4_replay
    /// with pixels"). Empty disables the check — for a reel over a fight that
    /// has no capture behind it.
    pub expected_draws: Vec<ExpectedDraw>,
}

impl ReelInput {
    /// A minimal input: a roster, no terrain, default knobs/art, no capture to
    /// check against. Hosts fill in the rest.
    pub fn new(label: impl Into<String>, rng_state: u32, combatants: Vec<ReelCombatant>) -> Self {
        ReelInput {
            label: label.into(),
            rng_state,
            combatants,
            terrain: None,
            knobs: ReelKnobs::default(),
            loadouts: Vec::new(),
            item_data: None,
            art: ReelArt::default(),
            game_speed: 4,
            tick_multiplier: 1,
            expected_draws: Vec::new(),
        }
    }

    /// The map this input replays on — the captured terrain, or open floor.
    pub fn map(&self) -> CombatMap {
        match &self.terrain {
            Some(ground) => CombatMap::from_ground(ground.clone()),
            None => CombatMap::uniform(FALLBACK_FLOOR),
        }
    }
}

/// **The one place a replay's `CombatState` is built.**
///
/// Decodes the roster records, lays the captured terrain, applies every knob,
/// installs the `ITEMS` table and the per-capture loadouts — in exactly the
/// order the H4 harnesses have always applied them, because they now call this.
///
/// The returned state has no sinks attached and has not stepped: the caller
/// owns the PRNG and the observation seams.
pub fn build_state(input: &ReelInput, flavor: &dyn Flavor) -> Result<CombatState, SaveParseError> {
    let entries: Vec<RecordCombatant> = input
        .combatants
        .iter()
        .map(|c| RecordCombatant {
            team: c.team,
            pos: c.pos,
            record: &c.record,
            affects: c.affects.clone(),
        })
        .collect();

    let mut state = combat_state_from_records(&entries, input.map(), flavor)?;
    state.area_field_58c = input.knobs.area_field_58c;
    state.map_direction = input.knobs.map_direction;
    state.auto_pcs_cast_magic = input.knobs.auto_cast;
    state.auto_cast_toggles = input.knobs.auto_cast_toggles.clone();
    state.continue_battle_yes = input.knobs.continue_battle.clone();
    state.area_field_6e4 = input.knobs.area_field_6e4;
    // §34.1: the `ITEMS` table first, then the rows that read it. A `None`
    // loadout list leaves every combatant exactly as today's engine — the
    // melee-identical path the un-armed captures ride.
    state.item_data = input.item_data.clone();
    for &(id, loadout) in &input.loadouts {
        state.set_loadout(id, loadout);
    }
    Ok(state)
}

/// Compare one of our draws against the capture's, on the harnesses' surface.
///
/// `(before, after)` always; the `Random(n)` operand additionally, but only
/// when **both** sides carry one — a capture from a hook that didn't record
/// `ss_sp_words` falls back to the chain alone for that draw.
pub fn draws_agree(ours: &crate::rng::RngDraw, capture: &ExpectedDraw) -> bool {
    let operand_ok = match (ours.n, capture.operand) {
        (Some(a), Some(b)) => a == b,
        _ => true,
    };
    ours.before == capture.before && ours.after == capture.after && operand_ok
}

/// Which combat mechanic drew, inferred from the `Random(n)` operand — the
/// honest die tells the mechanic (§2/§4/§9 draw map). Diagnostic only.
pub fn mechanic_for(operand: Option<u16>) -> &'static str {
    match operand {
        Some(6) => "initiative d6 (CalculateInitiative)",
        Some(100) => "d100 (FindNextCombatant selection, or FleeCheck/advance morale)",
        Some(20) => "d20 (to-hit PC_CanHitTarget, or a saving throw)",
        Some(7) => "d7 (QuickFight AI mode-gate / wand-scan / spell-priority)",
        Some(0) => "random(0) edge draw",
        Some(_) => "damage die (weapon/monster attack dice)",
        None => "unknown (operand not recorded)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::HealthStatus;
    use gbx_rules::adnd1::flavor_impl::Adnd1;
    use gbx_rules::pack::RuleSet;

    /// A hand-authored `0x1A6` record (D10 — no game bytes anywhere near
    /// this): a name, hp, AC, a 1d6 attack profile and an icon assignment.
    /// The field offsets are `save_orig::decode_char_record`'s.
    fn synthetic_record(name: &str, hp: u8, icon_slot: u8, monster: bool) -> Vec<u8> {
        let mut r = vec![0u8; COMBAT_RECORD_LEN];
        r[0] = name.len() as u8;
        r[1..1 + name.len()].copy_from_slice(name.as_bytes());
        r[0x78] = hp; // hit_point_max
        r[0x1a4] = hp; // hit_point_current
        r[0x19a] = 0x30; // ac
        r[0x199] = 40; // hitBonus
        r[0x11e] = 1; // attack-1 dice count
        r[0x120] = 6; // attack-1 dice size
        r[0xde] = 0x01; // size 1
        r[0x143] = icon_slot;
        r[0x144] = 1; // icon_size
        r[0xf7] = if monster { 0x80 } else { 0x00 }; // control_morale: NPC bit
        r
    }

    fn two_sided_input() -> ReelInput {
        let combatants = vec![
            ReelCombatant {
                team: Team::Party,
                pos: GridPos::new(20, 12),
                record: synthetic_record("ALPHA", 20, 0, false),
                affects: Vec::new(),
            },
            ReelCombatant {
                team: Team::Monster,
                pos: GridPos::new(24, 12),
                record: synthetic_record("BRUTE", 14, 8, true),
                affects: Vec::new(),
            },
        ];
        ReelInput::new("synthetic.gbxtrace", 0x0C0F_FEE0, combatants)
    }

    #[test]
    fn build_state_decodes_the_roster_in_order() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let input = two_sided_input();
        let state = build_state(&input, &flavor).expect("synthetic records decode");
        let roster = state.roster();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].team, Team::Party);
        assert_eq!(roster[0].pos, GridPos::new(20, 12));
        assert_eq!(roster[0].hp_current, 20);
        assert_eq!(roster[1].team, Team::Monster);
        assert_eq!(roster[1].hp_current, 14);
        assert_eq!(roster[1].health_status, HealthStatus::Okey);
    }

    #[test]
    fn build_state_applies_every_knob() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut input = two_sided_input();
        input.knobs = ReelKnobs {
            map_direction: 6,
            auto_cast: true,
            auto_cast_toggles: vec![3, 17],
            continue_battle: vec![0],
            area_field_58c: 75,
            area_field_6e4: -3,
        };
        let state = build_state(&input, &flavor).expect("records decode");
        assert_eq!(state.map_direction, 6);
        assert!(state.auto_pcs_cast_magic);
        assert_eq!(state.auto_cast_toggles, vec![3, 17]);
        assert_eq!(state.continue_battle_yes, vec![0]);
        assert_eq!(state.area_field_58c, 75);
        assert_eq!(state.area_field_6e4, -3);
    }

    #[test]
    fn the_default_knobs_are_the_documented_pre_capture_fallbacks() {
        // These four numbers are load-bearing history: every replay before the
        // hooks emitted them rode exactly these values.
        let k = ReelKnobs::default();
        assert_eq!(k.map_direction, 2);
        assert_eq!(k.area_field_58c, 99);
        assert_eq!(k.area_field_6e4, 0);
        assert!(!k.auto_cast);
        assert!(k.auto_cast_toggles.is_empty());
        assert!(k.continue_battle.is_empty());
    }

    #[test]
    fn terrain_builds_the_map_and_absence_falls_back_to_open_floor() {
        let mut input = two_sided_input();
        assert_eq!(input.map().ground_tile(GridPos::new(0, 0)), FALLBACK_FLOOR);

        let mut ground = vec![FALLBACK_FLOOR; 50 * 25];
        ground[12 * 50 + 22] = 1; // a wall between the two combatants
        input.terrain = Some(ground);
        assert_eq!(input.map().ground_tile(GridPos::new(22, 12)), 1);
    }

    #[test]
    fn a_loadout_row_lands_on_its_roster_index() {
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        let mut input = two_sided_input();
        input.loadouts = vec![(
            1,
            Loadout {
                ranged: Some((44, 0)),
                ammo_count: 7,
                ammo_readied: true,
                melee: None,
                unarmed_profile: (1, 8, 0),
                entry_ranged_readied: true,
            },
        )];
        let state = build_state(&input, &flavor).expect("records decode");
        assert!(
            state.roster()[0].loadout.is_none(),
            "index 0 carries no row"
        );
        assert_eq!(state.roster()[1].ammo, 7);
        assert_eq!(state.roster()[1].readied_weapon, Some((44, 0)));
    }

    #[test]
    fn draw_equality_uses_the_operand_only_when_both_sides_have_one() {
        use crate::rng::RngDraw;
        let ours = RngDraw {
            before: 1,
            after: 2,
            n: Some(20),
            result: Some(3),
        };
        assert!(draws_agree(
            &ours,
            &ExpectedDraw {
                before: 1,
                after: 2,
                operand: Some(20)
            }
        ));
        assert!(
            !draws_agree(
                &ours,
                &ExpectedDraw {
                    before: 1,
                    after: 2,
                    operand: Some(6)
                }
            ),
            "a differing operand is a divergence even when the chain matches"
        );
        assert!(
            draws_agree(
                &ours,
                &ExpectedDraw {
                    before: 1,
                    after: 2,
                    operand: None
                }
            ),
            "a capture with no recorded operand falls back to the chain"
        );
        assert!(!draws_agree(
            &ours,
            &ExpectedDraw {
                before: 1,
                after: 3,
                operand: Some(20)
            }
        ));
    }
}
