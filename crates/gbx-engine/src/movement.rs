//! Movement, facing, and door interaction (§1.6, task deliverable 4).
//!
//! Derived by reading coab for behavior (D11, never copied) — this session's
//! research pass pinned exact line numbers and several details the design
//! doc's prose only summarized; each is cited at its point of use below:
//! - coab `engine/ovr015.cs` `TryStepForward` (`:278-315`) and
//!   `MovePartyForward` (`:318-345`).
//! - coab `engine/ovr015.cs` `locked_door` (`:468-593`) and `bash_door`
//!   (`:49-224`)/`pick_lock` (`:227-254`).
//! - coab `engine/ovr031.cs` `WallDoorFlagsGet` (`:181-219`).
//! - coab `engine/ovr015.cs` `main_3d_world_menu` (`:348-465`).
//! - coab `engine/ovr025.cs` `display_map_position_time` (`:1476-1511`).

use crate::widgets::Hotbar;
use gbx_formats::geo::{GeoBlock, Square};
use gbx_rules::bash_door::{bash_outcome, BashOutcome, DoorStrength};
use gbx_vm::VmRng;

/// `mapDirection`'s cardinal values (`0,2,4,6` of the original's 8-dir
/// encoding — the walk loop only ever turns in 90/180 steps, §1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Facing {
    North,
    East,
    South,
    West,
}

impl Facing {
    /// `(dir + 6) % 8` (`ovr015.cs` extended-key `'K'` handler).
    pub fn turn_left(self) -> Self {
        match self {
            Facing::North => Facing::West,
            Facing::West => Facing::South,
            Facing::South => Facing::East,
            Facing::East => Facing::North,
        }
    }

    /// `(dir + 2) % 8` (`'M'` handler).
    pub fn turn_right(self) -> Self {
        match self {
            Facing::North => Facing::East,
            Facing::East => Facing::South,
            Facing::South => Facing::West,
            Facing::West => Facing::North,
        }
    }

    /// `(dir + 4) % 8` (`'P'` handler) — plays **no** turn sound, unlike
    /// left/right (research finding, refining the design doc's "sound
    /// (L/R)" phrasing).
    pub fn turn_around(self) -> Self {
        self.turn_left().turn_left()
    }

    /// Implementation note (flagged): North/South screen-axis sign is an
    /// unconfirmed convention pick (the material read this session didn't
    /// pin compass-vs-grid-Y orientation) — internally consistent with this
    /// module's own `GeoBlock` fixtures either way.
    pub(crate) fn delta(self) -> (i32, i32) {
        match self {
            Facing::North => (0, -1),
            Facing::East => (1, 0),
            Facing::South => (0, 1),
            Facing::West => (-1, 0),
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Facing::North => "N",
            Facing::East => "E",
            Facing::South => "S",
            Facing::West => "W",
        }
    }

    /// `gbl.mapDirection`'s raw encoding (`0=N,2=E,4=S,6=W`, `0x033D`'s
    /// read-only ScriptMemory cell — this session's research).
    pub fn raw_code(self) -> u8 {
        match self {
            Facing::North => 0,
            Facing::East => 2,
            Facing::South => 4,
            Facing::West => 6,
        }
    }

    /// Inverse of [`Facing::raw_code`]. Panics on an odd/out-of-range code —
    /// a caller bug (every write site normalizes first, matching
    /// `0xC04D`'s write handler, this session's research).
    pub fn from_raw(code: u8) -> Self {
        match code {
            0 => Facing::North,
            2 => Facing::East,
            4 => Facing::South,
            6 => Facing::West,
            other => panic!("Facing::from_raw: {other} is not a valid raw facing code"),
        }
    }
}

/// `WallDoorFlagsGet` (`ovr031.cs:181-219`): `1` when the edge has no wall
/// recorded at all; otherwise the edge's raw 2-bit door field (`0`=solid,
/// `1`=open, `2`=locked, `3`=unpickable). Diagonal facings can't occur here
/// (this module's [`Facing`] is cardinal-only) — the original's `switch`
/// falls through to the default `1` for them, per this session's research.
pub fn wall_door_flags(square: &Square, facing: Facing) -> u8 {
    let (wall, door) = match facing {
        Facing::North => (square.wall_north, square.door_north),
        Facing::East => (square.wall_east, square.door_east),
        Facing::South => (square.wall_south, square.door_south),
        Facing::West => (square.wall_west, square.door_west),
    };
    if wall == 0 {
        1
    } else {
        door
    }
}

/// The four door states `wall_door_flags` can resolve to (once a wall is
/// actually present — a `0` flag with no wall is "open", not "solid").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorState {
    Solid,
    Open,
    Locked,
    Unpickable,
}

impl DoorState {
    pub fn from_flag(v: u8) -> Self {
        match v {
            0 => DoorState::Solid,
            1 => DoorState::Open,
            2 => DoorState::Locked,
            _ => DoorState::Unpickable,
        }
    }
}

/// `TryStepForward` (`ovr015.cs:278-315`): a pure query. Returns the new
/// `tried_to_exit_map` value — `false` and a full no-op when the facing
/// edge is solid (confirmed: the original does nothing at all in that
/// case, not even touching the flag's *reset*, which the caller already
/// performs unconditionally per `sub_29758`'s bookkeeping — see
/// [`crate::shell`]). The actual position never changes here; a confirmed
/// step commits later via [`move_party_forward`].
pub fn try_step_forward(geo: &GeoBlock, pos: (u8, u8), facing: Facing) -> bool {
    let square = geo.square(pos.0 as usize, pos.1 as usize);
    if wall_door_flags(square, facing) == 0 {
        return false;
    }
    let (dx, dy) = facing.delta();
    let nx = pos.0 as i32 + dx;
    let ny = pos.1 as i32 + dy;
    !(0..16).contains(&nx) || !(0..16).contains(&ny)
}

/// `MovePartyForward` (`ovr015.cs:318-345`): position advances by facing's
/// delta, wrapped `& 0x0F` (confirmed masking, not clamping — distinct from
/// `TryStepForward`'s edge-clamp query above), the three door step-flags
/// reset to `true`, and the clock advances (slot 2/search vs slot 1/normal).
/// Caller owns the sound event and the 3-tick step delay (§1.10).
pub fn move_party_forward(
    pos: &mut (u8, u8),
    facing: Facing,
    search_mode: bool,
    flags: &mut DoorStepFlags,
    clock: &mut GameClock,
) {
    let (dx, dy) = facing.delta();
    pos.0 = (pos.0 as i32 + dx).rem_euclid(16) as u8;
    pos.1 = (pos.1 as i32 + dy).rem_euclid(16) as u8;
    *flags = DoorStepFlags::all_true();
    clock.advance(search_mode);
}

/// `can_bash_door`/`can_pick_door`/`can_knock_door` (plain fields per this
/// session's research, not gating functions — `ovr015.cs`/`seg001.cs`):
/// reset `true` at boot and on every successful [`move_party_forward`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DoorStepFlags {
    pub can_bash: bool,
    pub can_pick: bool,
    pub can_knock: bool,
}

impl DoorStepFlags {
    pub fn all_true() -> Self {
        DoorStepFlags {
            can_bash: true,
            can_pick: true,
            can_knock: true,
        }
    }
}

impl Default for DoorStepFlags {
    fn default() -> Self {
        Self::all_true()
    }
}

/// The M2/M3 seam (task deliverable 4): party-dependent door predicates —
/// thief presence, knock-spell memorization, and per-player STR — are a
/// named M3 concern (the party model doesn't exist yet). Movement/door
/// logic only ever calls through this trait; [`DefaultPartyPredicates`] is
/// a test-configurable stand-in exercising the same call shape M3's real
/// roster will fill in.
pub trait PartyPredicates {
    /// `(stats2.Str.full, Str00.cur)` for every player able to attempt a
    /// bash, in party (roll) order — `bash_door` stops at the first success
    /// (`ovr015.cs:55-58`, this session's research).
    fn bash_candidates(&self) -> Vec<(u8, u8)>;
    /// `AnyPlayerHasSkill(Thief)` — gates whether "Pick" appears at all.
    fn can_attempt_pick(&self) -> bool;
    /// `pick_lock`'s roll (`d100 <= thief_skills[1]`, M3 internals) — the
    /// seam only needs the resolved outcome.
    fn attempt_pick(&mut self, rng: &mut dyn VmRng) -> bool;
    /// `TeamMemberHasSpell(knock)` — gates whether "Knock" appears at all.
    fn can_attempt_knock(&self) -> bool;
    /// `RemoveKnockSpell()`'s success (consumes the spell, M3 internals).
    fn attempt_knock(&mut self) -> bool;
}

/// A fixed-answer [`PartyPredicates`] for tests exercising the movement/door
/// seam ahead of M3's real party model.
#[derive(Debug, Clone, Default)]
pub struct DefaultPartyPredicates {
    pub bash_candidates: Vec<(u8, u8)>,
    pub can_pick: bool,
    pub pick_succeeds: bool,
    pub can_knock: bool,
    pub knock_succeeds: bool,
}

impl PartyPredicates for DefaultPartyPredicates {
    fn bash_candidates(&self) -> Vec<(u8, u8)> {
        self.bash_candidates.clone()
    }
    fn can_attempt_pick(&self) -> bool {
        self.can_pick
    }
    fn attempt_pick(&mut self, _rng: &mut dyn VmRng) -> bool {
        self.pick_succeeds
    }
    fn can_attempt_knock(&self) -> bool {
        self.can_knock
    }
    fn attempt_knock(&mut self) -> bool {
        self.knock_succeeds
    }
}

/// `sound_a` (§1.10): played on a committed step and on left/right turns
/// (not 180, per this session's research). Implementation note (flagged):
/// the exact sound-catalog id wasn't in the material read this session
/// (`seg044.cs`'s `Sound` enum) — placeholder pending that read.
pub const SOUND_A: u8 = 0;

/// `1dN`, `N = die_size` (`roll_dice(size, 1)`, `ovr024.cs:586`).
fn roll_die(rng: &mut dyn VmRng, die_size: u8) -> u8 {
    rng.roll_uniform((die_size - 1) as u16) as u8 + 1
}

/// `locked_door`'s Bash/Pick/Knock/Exit menu build (`ovr015.cs:491-510`,
/// `534-552`): `None` when no option is available at all — the original
/// shows **no menu** and the attempt silently fails (research finding,
/// flagged: refines the design doc's "options gated on step-flags" into an
/// explicit reachable empty-menu state).
pub fn build_door_hotbar(flags: &DoorStepFlags, party: &dyn PartyPredicates) -> Option<Hotbar> {
    let mut words: Vec<&str> = Vec::new();
    if flags.can_bash {
        words.push("Bash");
    }
    if flags.can_pick && party.can_attempt_pick() {
        words.push("Pick");
    }
    if flags.can_knock && party.can_attempt_knock() {
        words.push("Knock");
    }
    if words.is_empty() {
        return None;
    }
    words.push("Exit");
    Some(Hotbar::new(words.join(" ")))
}

/// `bash_door` (`ovr015.cs:49-224`): iterates `party.bash_candidates()` in
/// order, stopping at the first success; a `NoEffect` outcome may disable
/// `flags.can_bash` per [`gbx_rules::bash_door::BashOutcome`]'s documented
/// table asymmetry, but never breaks the loop early.
///
/// Scope note (flagged, not silently absorbed): a success here does not
/// persist an "unlocked" state back into the resident `GeoBlock` — the
/// original calls `MapSetDoorUnlocked` on both tile sides so the door stays
/// open on a later approach (`ovr015.cs:212-224`, this session's research).
/// `gbx-formats::geo::GeoBlock` has no mutation API yet (a parsed map is
/// treated as immutable this session); this crossing succeeds for the
/// party's current step, but a later re-approach to the same edge would
/// re-roll the bash. Persisting door state is deferred to whichever session
/// adds resident-map mutation (naturally step 4/5, alongside real
/// `ScriptMemory` writes to the map window).
pub fn attempt_bash(
    state: DoorState,
    party: &dyn PartyPredicates,
    flags: &mut DoorStepFlags,
    rng: &mut dyn VmRng,
) -> bool {
    let strength = if state == DoorState::Unpickable {
        DoorStrength::Reinforced
    } else {
        DoorStrength::Normal
    };
    for (str_full, pct) in party.bash_candidates() {
        match bash_outcome(strength, str_full, pct) {
            BashOutcome::Auto => return true,
            BashOutcome::Roll {
                die_size,
                max_success,
            } => {
                if roll_die(rng, die_size) <= max_success {
                    return true;
                }
            }
            BashOutcome::NoEffect { disables_can_bash } => {
                if disables_can_bash {
                    flags.can_bash = false;
                }
            }
        }
    }
    false
}

/// `pick_lock` (`ovr015.cs:227-254`): `can_pick_door` is disabled
/// unconditionally after one attempt (success or not — a confirmed
/// asymmetry vs. bash's per-outcome disabling); an unpickable door
/// (`state == Unpickable`) never rolls at all (`'P'` on `al==3`,
/// `ovr015.cs:564` — flag-clear only, no roll, no success).
pub fn attempt_pick(
    state: DoorState,
    party: &mut dyn PartyPredicates,
    flags: &mut DoorStepFlags,
    rng: &mut dyn VmRng,
) -> bool {
    flags.can_pick = false;
    if state == DoorState::Unpickable {
        return false;
    }
    party.attempt_pick(rng)
}

/// `RemoveKnockSpell()` via `locked_door`'s `'K'` handler. Implementation
/// note (flagged): this session's research pinned bash's/pick's exact
/// `can_bash_door`/`can_pick_door` clearing rules but didn't trace
/// `RemoveKnockSpell()` itself (an M3 spell-memorization concern) — whether
/// `can_knock_door` also unconditionally clears after one attempt (mirroring
/// pick) is left to the M3 seam's real implementation; this function does
/// not touch `flags.can_knock`, docketed pending that read.
pub fn attempt_knock(party: &mut dyn PartyPredicates) -> bool {
    party.attempt_knock()
}

/// The game-world clock (§1.6's "clock slot 2 in search mode else 1").
/// Implementation note (flagged): the exact original per-step minute value
/// wasn't in the material read this session (`step_game_time`'s unit
/// definition lives outside `ovr015.cs`/`ovr031.cs`) — `MINUTES_PER_UNIT`
/// is a placeholder pending that read; only the 2x-in-search relative rate
/// is confirmed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GameClock {
    pub total_units: u32,
}

impl GameClock {
    const MINUTES_PER_UNIT: u32 = 10;

    /// `step_game_time(2, 1)` in search mode, `step_game_time(1, 1)`
    /// otherwise (`MovePartyForward`, confirmed exact) — sugar over
    /// [`GameClock::step`].
    pub fn advance(&mut self, search_mode: bool) {
        self.step(if search_mode { 2 } else { 1 }, 1);
    }

    /// `EngineServices::step_game_time(time_slot, amount)` — ECL CLOCK
    /// (0x34)'s general form: `time_slot == 2` runs at double rate (the
    /// same search-mode multiplier `MovePartyForward` uses), any other
    /// slot at normal rate. This session's research confirmed the field
    /// *identities* the raw clock cells back (§ScriptMemory `0x4BC6`+) but
    /// not the original's exact minutes-per-tick/calendar constants;
    /// `MINUTES_PER_UNIT` and [`GameClock::raw_clock_words`]'s day/year
    /// derivation are documented placeholders pending that read.
    pub fn step(&mut self, time_slot: u8, amount: u8) {
        let multiplier = if time_slot == 2 { 2 } else { 1 };
        self.total_units += amount as u32 * multiplier;
    }

    /// The inverse of [`GameClock::raw_clock_words`]' inner 5 words
    /// (minutes-ones, minutes-tens, hour, day, year) — original-save import
    /// (task deliverable 4, `docs/design/save-formats.md` §1.4's clock
    /// cells). `day`/`year` are 1-based on the wire (clamped to `>= 1`
    /// here so a zeroed/malformed save can't underflow); reconstructs
    /// `total_units` exactly for any value this module itself produced.
    pub fn from_raw_clock_words(words: [u16; 5]) -> Self {
        let [minutes_ones, minutes_tens, hour, day, year] = words;
        let minutes = (minutes_tens as u32) * 10 + minutes_ones as u32;
        let day = (day as u32).max(1);
        let year = (year as u32).max(1);
        let total_minutes =
            minutes + hour as u32 * 60 + (day - 1) * 60 * 24 + (year - 1) * 360 * 24 * 60;
        GameClock {
            total_units: total_minutes / Self::MINUTES_PER_UNIT,
        }
    }

    pub fn hh_mm(&self) -> (u8, u8) {
        let total_minutes = self.total_units * Self::MINUTES_PER_UNIT;
        (
            ((total_minutes / 60) % 24) as u8,
            (total_minutes % 60) as u8,
        )
    }

    /// The 7 raw ScriptMemory clock words at `0x4BC6..=0x4BD2` (this
    /// session's research, `Classes/Area1.cs:41-61`): two unlabeled
    /// bracketing words (kept `0`, matching the original's own "never
    /// separately assigned" fields), minutes-ones, minutes-tens, hour, day,
    /// year.
    pub fn raw_clock_words(&self) -> [u16; 7] {
        let total_minutes = self.total_units * Self::MINUTES_PER_UNIT;
        let minutes = total_minutes % 60;
        let hour = (total_minutes / 60) % 24;
        let day = (total_minutes / 60 / 24) % 30 + 1;
        let year = 1 + total_minutes / 60 / 24 / 360;
        [
            0,
            (minutes % 10) as u16,
            (minutes / 10) as u16,
            hour as u16,
            day as u16,
            year as u16,
            0,
        ]
    }
}

/// `display_map_position_time` (`ovr025.cs:1476-1511`): `"X,Y DIR HH:MM"`,
/// plus `" search"`/`" camping"` when applicable.
pub fn position_time_text(
    pos: (u8, u8),
    facing: Facing,
    clock: &GameClock,
    search_mode: bool,
) -> String {
    let (hh, mm) = clock.hh_mm();
    let mut s = format!("{},{} {} {:02}:{:02}", pos.0, pos.1, facing.glyph(), hh, mm);
    if search_mode {
        s.push_str(" search");
    }
    s
}

/// `main_3d_world_menu`'s dispatch table (`ovr015.cs:348-465`, §1.6),
/// factored out of the raw resolved Hotbar key so [`crate::shell`] doesn't
/// need to know the ctrl-code table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldMenuCommand {
    ToggleAreaView,
    NotHere,
    Cast,
    View,
    Encamp,
    ToggleSearch,
    Look,
    Forward,
    TurnLeft,
    TurnRight,
    TurnAround,
    ScrollParty(u8),
    /// Unrecognized: the world menu simply redraws the status line and
    /// stays parked (`main_3d_world_menu` reprompts unconditionally).
    None,
}

/// Maps a resolved world-menu Hotbar key (§1.6's table) to a command. `area`
/// is `block_area_view == 0` — when false, `'A'` shows the timed "Not Here"
/// status instead of toggling the area view.
pub fn world_menu_command(key: u8, area_view_available: bool) -> WorldMenuCommand {
    match key {
        b'A' if area_view_available => WorldMenuCommand::ToggleAreaView,
        b'A' => WorldMenuCommand::NotHere,
        b'C' => WorldMenuCommand::Cast,
        b'V' => WorldMenuCommand::View,
        b'E' => WorldMenuCommand::Encamp,
        b'S' => WorldMenuCommand::ToggleSearch,
        b'L' => WorldMenuCommand::Look,
        b'H' => WorldMenuCommand::Forward,
        b'K' => WorldMenuCommand::TurnLeft,
        b'M' => WorldMenuCommand::TurnRight,
        b'P' => WorldMenuCommand::TurnAround,
        // Any other resolved ctrl-code (Home/End/PgUp/PgDn/Kp*) scrolls the
        // party panel instead (§1.6's "other extended" row).
        code @ (b'G' | b'O' | b'I' | b'Q' | b' ') => WorldMenuCommand::ScrollParty(code),
        _ => WorldMenuCommand::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::geo::GeoBlock;

    #[test]
    fn game_clock_from_raw_words_round_trips_through_to_the_same_words() {
        let clock = GameClock { total_units: 54321 }; // arbitrary nonzero value
        let words = clock.raw_clock_words();
        let rebuilt =
            GameClock::from_raw_clock_words([words[1], words[2], words[3], words[4], words[5]]);
        assert_eq!(rebuilt.raw_clock_words(), words);
    }

    #[test]
    fn game_clock_from_raw_words_at_zero_is_the_default_clock() {
        let rebuilt = GameClock::from_raw_clock_words([0, 0, 0, 1, 1]);
        assert_eq!(rebuilt.total_units, 0);
    }

    const GEO_BLOCK_SIZE: usize = gbx_formats::geo::GEO_BLOCK_SIZE;

    fn synthetic_geo() -> GeoBlock {
        // A minimal all-open block (every plane zeroed): every edge has no
        // wall recorded, so wall_door_flags is 1 (open) everywhere.
        GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap()
    }

    fn geo_with_wall_at(x: usize, y: usize, dir_nibble_hi: bool, wall: u8, door: u8) -> GeoBlock {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        let i = x + 16 * y;
        // planes: NE at offset 2, SW at 2+256, door at 2+3*256.
        if dir_nibble_hi {
            data[2 + i] = wall << 4; // North (hi nibble)
            data[2 + 3 * 256 + i] = door; // door_north = bits 0-1
        } else {
            data[2 + i] |= wall; // East (lo nibble)
            data[2 + 3 * 256 + i] = door << 2; // door_east = bits 2-3
        }
        GeoBlock::parse(&data).unwrap()
    }

    #[test]
    fn turn_left_right_around_match_the_8dir_deltas() {
        assert_eq!(Facing::North.turn_left(), Facing::West);
        assert_eq!(Facing::North.turn_right(), Facing::East);
        assert_eq!(Facing::North.turn_around(), Facing::South);
        assert_eq!(Facing::West.turn_left(), Facing::South);
        assert_eq!(Facing::East.turn_right(), Facing::South);
    }

    #[test]
    fn wall_door_flags_defaults_to_open_with_no_wall_recorded() {
        let geo = synthetic_geo();
        let sq = geo.square(5, 5);
        assert_eq!(wall_door_flags(sq, Facing::North), 1);
    }

    #[test]
    fn wall_door_flags_reads_the_door_field_when_a_wall_is_present() {
        let geo = geo_with_wall_at(5, 5, true, 3, 2); // North wall type 3, door state 2 (locked)
        let sq = geo.square(5, 5);
        assert_eq!(wall_door_flags(sq, Facing::North), 2);
    }

    #[test]
    fn try_step_forward_is_a_full_no_op_against_a_solid_wall() {
        let geo = geo_with_wall_at(5, 5, true, 3, 0); // wall present, door state 0 = solid
        let tried = try_step_forward(&geo, (5, 5), Facing::North);
        assert!(!tried);
    }

    #[test]
    fn try_step_forward_flags_exiting_the_grid() {
        let geo = synthetic_geo(); // open everywhere
        assert!(try_step_forward(&geo, (0, 0), Facing::North)); // y would go to -1
        assert!(!try_step_forward(&geo, (5, 5), Facing::North)); // interior step, no exit
    }

    #[test]
    fn move_party_forward_wraps_via_masking_not_clamping() {
        let mut pos = (15, 0);
        let mut flags = DoorStepFlags {
            can_bash: false,
            can_pick: false,
            can_knock: false,
        };
        let mut clock = GameClock::default();
        move_party_forward(&mut pos, Facing::East, false, &mut flags, &mut clock);
        assert_eq!(pos, (0, 0), "must wrap 15+1 -> 0, not clamp at 15");
        assert!(flags.can_bash && flags.can_pick && flags.can_knock);
    }

    #[test]
    fn move_party_forward_advances_clock_twice_as_fast_searching() {
        let mut pos = (5, 5);
        let mut flags = DoorStepFlags::all_true();
        let mut clock = GameClock::default();
        move_party_forward(&mut pos, Facing::East, true, &mut flags, &mut clock);
        assert_eq!(clock.total_units, 2);
        move_party_forward(&mut pos, Facing::East, false, &mut flags, &mut clock);
        assert_eq!(clock.total_units, 3);
    }

    #[test]
    fn build_door_hotbar_is_none_when_every_option_is_unavailable() {
        let flags = DoorStepFlags {
            can_bash: false,
            can_pick: false,
            can_knock: false,
        };
        let party = DefaultPartyPredicates::default();
        assert!(build_door_hotbar(&flags, &party).is_none());
    }

    #[test]
    fn build_door_hotbar_lists_only_available_options_plus_exit() {
        let flags = DoorStepFlags::all_true();
        let party = DefaultPartyPredicates {
            can_pick: true,
            can_knock: false,
            ..Default::default()
        };
        let hb = build_door_hotbar(&flags, &party).unwrap();
        assert_eq!(hb.text, "Bash Pick Exit");
    }

    #[test]
    fn build_door_hotbar_gates_pick_on_thief_presence_even_if_flag_allows() {
        let flags = DoorStepFlags::all_true();
        let party = DefaultPartyPredicates::default(); // can_pick=false (no thief)
        let hb = build_door_hotbar(&flags, &party).unwrap();
        assert_eq!(hb.text, "Bash Exit");
    }

    #[test]
    fn attempt_bash_stops_at_the_first_success() {
        let party = DefaultPartyPredicates {
            bash_candidates: vec![(25, 0), (3, 0)], // first is automatic success
            ..Default::default()
        };
        let mut flags = DoorStepFlags::all_true();
        let mut rng = crate::rng::EngineRng::new(1);
        assert!(attempt_bash(
            DoorState::Locked,
            &party,
            &mut flags,
            &mut rng
        ));
    }

    #[test]
    fn attempt_bash_reinforced_out_of_table_str_disables_can_bash() {
        let party = DefaultPartyPredicates {
            bash_candidates: vec![(3, 0)], // out of reinforced table
            ..Default::default()
        };
        let mut flags = DoorStepFlags::all_true();
        let mut rng = crate::rng::EngineRng::new(1);
        assert!(!attempt_bash(
            DoorState::Unpickable,
            &party,
            &mut flags,
            &mut rng
        ));
        assert!(!flags.can_bash);
    }

    #[test]
    fn attempt_pick_always_disables_can_pick_even_on_success() {
        let mut party = DefaultPartyPredicates {
            pick_succeeds: true,
            ..Default::default()
        };
        let mut flags = DoorStepFlags::all_true();
        let mut rng = crate::rng::EngineRng::new(1);
        assert!(attempt_pick(
            DoorState::Locked,
            &mut party,
            &mut flags,
            &mut rng
        ));
        assert!(!flags.can_pick);
    }

    #[test]
    fn attempt_pick_on_unpickable_door_never_succeeds_and_never_rolls() {
        let mut party = DefaultPartyPredicates {
            pick_succeeds: true, // would succeed if rolled
            ..Default::default()
        };
        let mut flags = DoorStepFlags::all_true();
        let mut rng = crate::rng::EngineRng::new(1);
        assert!(!attempt_pick(
            DoorState::Unpickable,
            &mut party,
            &mut flags,
            &mut rng
        ));
    }

    #[test]
    fn world_menu_command_maps_the_dispatch_table() {
        assert_eq!(world_menu_command(b'H', true), WorldMenuCommand::Forward);
        assert_eq!(world_menu_command(b'K', true), WorldMenuCommand::TurnLeft);
        assert_eq!(world_menu_command(b'M', true), WorldMenuCommand::TurnRight);
        assert_eq!(world_menu_command(b'P', true), WorldMenuCommand::TurnAround);
        assert_eq!(world_menu_command(b'L', true), WorldMenuCommand::Look);
        assert_eq!(
            world_menu_command(b'S', true),
            WorldMenuCommand::ToggleSearch
        );
        assert_eq!(
            world_menu_command(b'A', true),
            WorldMenuCommand::ToggleAreaView
        );
        assert_eq!(world_menu_command(b'A', false), WorldMenuCommand::NotHere);
        assert_eq!(
            world_menu_command(b'G', true),
            WorldMenuCommand::ScrollParty(b'G')
        );
    }

    #[test]
    fn position_time_text_includes_search_suffix_only_when_searching() {
        let clock = GameClock::default();
        let plain = position_time_text((3, 4), Facing::North, &clock, false);
        assert!(!plain.contains("search"));
        let searching = position_time_text((3, 4), Facing::North, &clock, true);
        assert!(searching.ends_with(" search"));
    }
}
