//! The `.gbxtrace` format (D-OR3): types, the **canonical** writer, and a
//! **liberal** parser.
//!
//! A `.gbxtrace` file is JSON-lines: line 1 is a [`TraceHeader`]; every
//! subsequent non-blank line is a [`TraceEvent`]. The order of events is the
//! draw/emission order and is **part of the format contract** (D-OR3) — the
//! comparator and the chain-continuity check both depend on it.
//!
//! **Canonical writer, liberal reader.** Our writer emits one compact JSON
//! object per line — fixed field order (serde struct declaration order), no
//! insignificant whitespace, integers only, optional fields omitted when
//! absent — so a trace file is **byte-hashable** (the H1 hashes-only pattern,
//! D-OR3). The reader ignores unknown/extra fields (serde's default): the
//! step-3 staging hook emits additional *diagnostic* fields beyond `caller`
//! (e.g. `ss_sp_words`), and they must not break us. We never enable
//! `deny_unknown_fields`.
//!
//! Integers only is deliberate: floats would introduce formatting
//! nondeterminism and defeat byte-hashing. The float `Random` path (M5) will
//! carry its Turbo-Pascal 6-byte real as a fixed-point integer when it lands.

use std::fmt;

/// The trace profile. `prng` is the H3/H4-gate-carrying stream profile built
/// this session; `action` is the semantic-combat profile whose vocabulary is
/// pinned as combat systems land (D-OR3, step 5) — the *mechanism* exists here,
/// the event fields do not (deliberately not speculatively invented).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Prng,
    Action,
}

/// The `.gbxtrace` header (D-OR3). Field order here **is** the canonical
/// on-disk order: `gbxtrace`, `profile`, `game`, `seed`, `encounter`,
/// `source`, `notes`.
///
/// `gbxtrace` is the format-major version (currently `1`); a rename or semantic
/// change bumps it and the comparator rejects a version mismatch (mirroring
/// D-SAVE2). `source` and `notes` are provenance only — **ignored** by the
/// comparator's validity gate (the whole point is comparing a `restrike` trace
/// against a `staging-hook` trace). `source` is a free-form string on purpose:
/// the writer only ever emits the known values (`restrike`/`staging-hook`/
/// `coab-fork`), but the reader accepts any so a future emitter can't break it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TraceHeader {
    /// Format-major version. `1` for every trace this session emits.
    pub gbxtrace: u32,
    pub profile: Profile,
    /// The game + version the trace was produced against, e.g. `"cotab-v1.3"`.
    pub game: String,
    /// The seed poked at the synchronization point (D-OR4 part B).
    pub seed: u32,
    /// A label identifying the captured window/encounter (e.g.
    /// `"creation-rerolls"`); `""` is allowed for a bare stream.
    pub encounter: String,
    /// `"restrike"` | `"staging-hook"` | `"coab-fork"` | … — provenance,
    /// excluded from the comparator's validity gate.
    pub source: String,
    /// Free-form provenance note — excluded from the validity gate. Omitted
    /// from the canonical form when absent.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub notes: Option<String>,
}

/// One `.gbxtrace` event line, internally tagged by `"e"` (which serde emits
/// **first**, matching the doc's `{"e":"rng",…}` shape).
///
/// Two profiles' event types live here. The `prng`-profile types are
/// [`RngEvent`] and the bare `randomize` marker. The `action` profile's
/// vocabulary is pinned as combat systems land (D-OR3, step 5): the **initiative
/// slice** pinned [`InitEvent`] and [`PickEvent`] (`init`/`pick`); the **attack
/// slice** pins [`AttackEvent`], [`DmgEvent`], and [`SaveEvent`]
/// (`attack`/`dmg`/`save`); the **melee-AI slice** pins [`MoveEvent`], [`AiEvent`],
/// and [`MoraleEvent`] (`move`/`ai`/`morale`); the remaining action types
/// (`status`/`award`) are still absent and their sessions add them. An unknown `e`
/// value is rejected loudly by the reader (a foreign
/// event type), distinct from tolerating unknown *fields*, which it does — so
/// pinning a vocabulary means adding a variant here, moving it out of "unknown".
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum TraceEvent {
    /// A single PRNG draw (`prng` profile).
    Rng(RngEvent),
    /// `{"e":"randomize"}` — the original's boot-time `Randomize` re-seed. In a
    /// post-boot capture this is a **loud finding**, not a curiosity (D-OR4
    /// part B): it means the seed dword was re-written mid-session. The
    /// chain-continuity check reports it.
    Randomize,
    /// `init` (`action` profile) — one per combatant in `CalculateInitiative`,
    /// bracketing its one d6.
    Init(InitEvent),
    /// `pick` (`action` profile) — one per `FindNextCombatant` selection.
    Pick(PickEvent),
    /// `attack` (`action` profile) — one per to-hit resolution, bracketing its
    /// one d20.
    Attack(AttackEvent),
    /// `dmg` (`action` profile) — one per damage roll (emitted only on a hit),
    /// bracketing its `dice_count` damage dice.
    Dmg(DmgEvent),
    /// `save` (`action` profile) — one per saving throw, bracketing its one d20.
    Save(SaveEvent),
    /// `move` (`action` profile) — one per movement step (`sub_3E748`). Draw-free.
    Move(MoveEvent),
    /// `ai` (`action` profile) — one per melee AI turn (`PlayerQuickFight`): its
    /// resolved target-mode + target.
    Ai(AiEvent),
    /// `morale` (`action` profile) — one per morale/advance decision, bracketing
    /// its 0-or-1 `random(100)`.
    Morale(MoraleEvent),
    /// `combat_entry` — the **combat entry-state snapshot** (D-OR5(b), H4): the
    /// seed + full roster (team/position/0x1A6 record) captured live at the moment
    /// a fight begins. It is the replay **input**, not a draw: the comparator and
    /// the chain-continuity check both **ignore** it (it carries no PRNG state and
    /// must never count as an event mismatch). A capture emits exactly one, ahead
    /// of the fight's `rng` stream.
    CombatEntry(CombatEntryEvent),
    /// `round_snapshot` — a capture-side **observation** (H4 localizer): the full
    /// board (`team`/`x`/`y`/`hp` per roster slot) at a round boundary. Like
    /// `combat_entry` it carries no PRNG state and is not a draw — the comparator
    /// and the chain-continuity check both ignore it. The board-diff harnesses
    /// (`h4_turndiff`) are its consumers.
    RoundSnapshot(RoundSnapshotEvent),
    /// `turn_snapshot` — a capture-side observation: the board (plus each slot's
    /// `target`) recorded by the staging hook on in-turn state writes
    /// (position/hp/target). Ignored by comparator + chain like `combat_entry`.
    TurnSnapshot(TurnSnapshotEvent),
}

/// A `prng`-profile draw event (D-OR3): `{"e":"rng","before":u32,"after":u32}`
/// plus the optional operand extension and the optional diagnostic `caller`.
///
/// Field order is the canonical order. `before`/`after` are the equality core;
/// `n`/`result` extend equality **only when both compared traces carry them**;
/// `caller` is **diagnostic only, excluded from equality** (a runtime seg:ofs
/// can never equal restrike's synthetic tag, and overlay return addresses
/// aren't stable run-to-run — D-OR3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RngEvent {
    /// The state dword before the LCG step (the original's `DS:0x47F0`).
    pub before: u32,
    /// The state dword after the step. Independent of `before` on the capture
    /// side (the staging hook reads it back from memory, not by re-computing),
    /// which is what makes chain continuity a real check.
    pub after: u32,
    /// The `Random(n)` wrapper operand, when the emitter knows it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub n: Option<u16>,
    /// The value `Random(n)` returned, when the emitter knows it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<u16>,
    /// Diagnostic call-site tag — **never** part of equality. Held as an
    /// arbitrary JSON value so the reader tolerates whatever the staging hook
    /// emits (a synthetic string tag from restrike, a `{"seg":…,"ofs":…}`
    /// object or a raw offset from the hook). See [`crate::compare`] for the
    /// image-offset normalization used in diagnostics.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub caller: Option<serde_json::Value>,
}

/// An `action`-profile `init` event (D-OR3; `combat-study.md` §9, pinned by the
/// initiative slice — `gbx-engine`'s `combat::CalculateInitiative`). Emitted per
/// combatant, bracketing its one `random(6)`.
///
/// Field order **is** the canonical on-disk order: `combatant_id`, `delay`,
/// `dex_adj`, `surprise`. All integers (D-OR3 canonical encoding): `surprise` is
/// `0`/`1`, not a JSON bool. `combatant_id` is the stable per-encounter roster
/// index; `delay` the final assigned initiative value (`0..=20`); `dex_adj` the
/// DEX reaction adjustment added (`-4..=5`); `surprise` whether the team `-6`
/// fired for this combatant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InitEvent {
    pub combatant_id: u32,
    pub delay: i16,
    pub dex_adj: i16,
    pub surprise: u8,
}

/// An `action`-profile `pick` event (D-OR3; `combat-study.md` §9, pinned by the
/// initiative slice — `gbx-engine`'s `combat::FindNextCombatant`). Emitted per
/// selection (one per yielded combatant).
///
/// Field order **is** the canonical on-disk order: `pass`, `combatant_id`,
/// `delay`, `roll`. All integers. `pass` is the 0-based selection-pass index
/// within the round; `combatant_id` the chosen roster index; `delay` the winning
/// combatant's delay at selection time; `roll` the winning `random(100)`+1
/// (`1..=100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PickEvent {
    pub pass: u32,
    pub combatant_id: u32,
    pub delay: i16,
    pub roll: u16,
}

/// An `action`-profile `attack` event (D-OR3; `combat-study.md` §9, pinned by the
/// attack slice — `gbx-engine`'s `combat::resolve_attack` → `PC_CanHitTarget`).
/// Emitted per to-hit resolution, bracketing its one `random(20)`.
///
/// Field order **is** the canonical on-disk order: `attacker_id`, `target_id`,
/// `roll`, `hit`. All integers (`hit` is `0`/`1`, not a JSON bool). `attacker_id`
/// / `target_id` are stable per-encounter roster indices; `roll` is the **raw d20
/// (1..=20, before the natural-20 promotion to 100)** — the honest observable
/// die, from which nat-1/nat-20 are visible; `hit` the resolved outcome. (The §9
/// strawman also listed `attack_idx`/`bonus`/`target_ac`; the session brief trims
/// the pinned event to the observable roll + outcome, as it did for `dmg`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttackEvent {
    pub attacker_id: u32,
    pub target_id: u32,
    pub roll: u8,
    pub hit: u8,
}

/// An `action`-profile `dmg` event (D-OR3; `combat-study.md` §9, pinned by the
/// attack slice — `gbx-engine`'s `combat::roll_damage` → `sub_3E192`). Emitted
/// per damage roll (**only on a hit**), bracketing its `dice_count` damage dice.
///
/// Field order **is** the canonical on-disk order: `attacker_id`, `target_id`,
/// `amount`, `backstab`. All integers (`backstab` is `0`/`1`). `amount` is the
/// final damage (dice + bonus, clamped `>= 0`, times the backstab multiplier);
/// `backstab` whether that multiplier was applied. (Trimmed from the §9 strawman's
/// `dice_count`/`dice_size`/`bonus`/`backstab_mult`/`total` to the resolved
/// amount + flag, per the session brief.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DmgEvent {
    pub attacker_id: u32,
    pub target_id: u32,
    pub amount: i32,
    pub backstab: u8,
}

/// An `action`-profile `save` event (D-OR3; `combat-study.md` §9, pinned by the
/// attack slice — `gbx-engine`'s `combat::roll_saving_throw` → `RollSavingThrow`).
/// Emitted per saving throw, bracketing its one `random(20)`.
///
/// Field order **is** the canonical on-disk order: `combatant_id`, `save_type`,
/// `roll`, `made`. All integers (`made` is `0`/`1`). `save_type` is the
/// `SaveVerseType` index (`0..=4`); `roll` the raw d20 (1..=20); `made` the
/// outcome. (Trimmed from the §9 strawman's `bonus`/`target`, which are non-drawn
/// inputs, not observables.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SaveEvent {
    pub combatant_id: u32,
    pub save_type: u8,
    pub roll: u8,
    pub made: u8,
}

/// An `action`-profile `move` event (D-OR3; `combat-study.md` §9, pinned by the
/// melee-AI slice — `gbx-engine`'s `combat` `sub_3E748`). Emitted per movement
/// step; **draw-free** (movement rolls no dice — the per-step monster morale d100
/// is the separate `morale` event that precedes the step).
///
/// Field order **is** the canonical on-disk order: `combatant_id`, `from_x`,
/// `from_y`, `to_x`, `to_y`, `cost`. The `{x,y}` pairs of the §9 strawman are
/// flattened to integers (D-OR3 canonical encoding); `cost` is the half-move cost
/// deducted (diagonal ×3 / orthogonal ×2 of the destination tile's move_cost).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MoveEvent {
    pub combatant_id: u32,
    pub from_x: i32,
    pub from_y: i32,
    pub to_x: i32,
    pub to_y: i32,
    pub cost: i32,
}

/// An `action`-profile `ai` event (D-OR3; `combat-study.md` §9, pinned by the
/// melee-AI slice — `gbx-engine`'s `combat::CombatState::melee_ai_turn`). Emitted
/// once per melee AI turn, after its target is resolved.
///
/// Field order **is** the canonical on-disk order: `combatant_id`, `field_15`,
/// `target_id`. `field_15` is the (post-gate) target-mode scratch (`1..=6`);
/// `target_id` the chosen target's roster index, or **`-1` when none** (guarding /
/// no reachable enemy) — integer-encoded per D-OR3 (no `Option`/null on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AiEvent {
    pub combatant_id: u32,
    pub field_15: u8,
    pub target_id: i64,
}

/// An `action`-profile `morale` event (D-OR3; `combat-study.md` §9, pinned by the
/// melee-AI slice — `gbx-engine`'s `combat` `moralFailureEscape`/`FleeCheck_001`).
/// Emitted per morale/advance decision, bracketing its **0-or-1** `random(100)`.
///
/// Field order **is** the canonical on-disk order: `combatant_id`,
/// `monster_morale`, `enemy_hp_pct`, `roll`, `failed`. `roll` is the advance d100
/// (`1..=100`) when drawn — a **monster** draws it; **`0` when none was drawn**
/// (a PC short-circuits the gate). `failed` (`0`/`1`) is `moral_failure`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MoraleEvent {
    pub combatant_id: u32,
    pub monster_morale: i32,
    pub enemy_hp_pct: i32,
    pub roll: u16,
    pub failed: u8,
}

/// The byte length of one combat record — a full `Player`/monster record
/// (`0x1A6` = 422 bytes). Named here so the `.gbxtrace` format layer stays
/// self-describing (it deliberately does not depend on `gbx-formats`); the
/// engine-side decoder validates the same length independently.
pub const COMBAT_RECORD_LEN: usize = 0x1A6;

/// The `combat_entry` snapshot event (D-OR5(b), H4): the replay seed plus the
/// full combat roster captured live at fight start. **Input, not a draw** — the
/// [`crate::compare`] comparator and chain-check both skip it (see the
/// [`TraceEvent::CombatEntry`] doc).
///
/// `rng_state` is the seed the replay pokes into `gbx-prng`; it equals the
/// `before` of the first following `rng` event (chain-continuous across the
/// snapshot). `combatants` is in `TeamList` order — party then monsters — which
/// **is** the initiative draw order, so the harness must preserve it verbatim.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CombatEntryEvent {
    /// The replay seed (`DS:0x47F0` at combat entry) == the first following draw's
    /// `before`.
    pub rng_state: u32,
    /// The ground-tile grid (`mapToBackGroundTile`, 50×25 row-major) as
    /// lowercase hex, when the staging hook captured it — terrain is
    /// load-bearing for movement (§14), so an H4 replay must build its
    /// `CombatMap` from this. Optional: pre-terrain and synthetic captures omit
    /// it (and the canonical writer then omits the field).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub terrain: Option<String>,
    /// `area2.field_58C` — the morale threshold the faithful `FleeCheck_001`
    /// gate 2 reads (`sub_3637F` @`ovr010:1473`, doc §28). Captured live by the
    /// staging hook (which can also poke it via `RESTRIKE_58C`). Optional and
    /// additive: pre-`field_58C` captures omit it (and the canonical writer then
    /// omits the field, keeping existing goldens byte-identical); the replay
    /// harnesses default a missing value to **99** — the measured bar value (§28)
    /// under which the natural rout is mathematically impossible.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub area2_field_58c: Option<u16>,
    /// The roster in `TeamList` (== initiative draw) order.
    pub combatants: Vec<CombatEntryCombatant>,
}

/// The `round_snapshot` observation event (H4): the capture-side board at a
/// round boundary. See [`TraceEvent::RoundSnapshot`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoundSnapshotEvent {
    /// The round counter at the snapshot.
    pub round: u16,
    /// One row per roster slot, roster order.
    pub combatants: Vec<SnapshotCombatant>,
}

/// The `turn_snapshot` observation event (H4): the capture-side board recorded
/// on an in-turn state write. See [`TraceEvent::TurnSnapshot`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TurnSnapshotEvent {
    /// The hook's monotonically increasing snapshot sequence number.
    pub seq: u32,
    /// One row per roster slot, roster order.
    pub combatants: Vec<SnapshotCombatant>,
}

/// One board row in a [`RoundSnapshotEvent`]/[`TurnSnapshotEvent`]: team,
/// position, hp, and (turn snapshots only) the slot's `actions.target`
/// (`255` = none on the wire).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotCombatant {
    /// `0` = party, `1` = monsters (as [`CombatEntryCombatant::team`]).
    pub team: u8,
    pub x: u8,
    pub y: u8,
    pub hp: u8,
    /// `actions.target` as a roster index, `255` = none. Emitted by
    /// `turn_snapshot` only; absent on `round_snapshot` rows.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub target: Option<u8>,
}

/// One combatant in a [`CombatEntryEvent`]: its team, grid position, and the
/// raw `0x1A6` record bytes (a full `Player`/monster record). The record is
/// carried as a `2·0x1A6`-char lowercase-hex string on the wire (integers-only /
/// byte-hashable canonical encoding, D-OR3) and decoded to a fixed array by the
/// reader. **Real record bytes are local-only (D10)** — a `combat_entry` line
/// never lands in the repo/CI; only synthetic records exercise this in CI.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CombatEntryCombatant {
    /// `0` = party (`CombatTeam.Ours`), `1` = monsters (`CombatTeam.Enemy`).
    pub team: u8,
    pub x: u8,
    pub y: u8,
    /// The full `0x1A6` combat record, hex-encoded on the wire.
    #[serde(with = "hex_record")]
    pub record: [u8; COMBAT_RECORD_LEN],
}

impl fmt::Debug for CombatEntryCombatant {
    /// Compact: the 422-byte record would swamp any diagnostic, so it is elided.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CombatEntryCombatant")
            .field("team", &self.team)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("record", &format_args!("[{} bytes]", COMBAT_RECORD_LEN))
            .finish()
    }
}

/// Fixed-length-array hex serde for the combat record. Serializes to lowercase
/// hex (canonical, deterministic); deserializes with a strict length + hex-digit
/// check (a trace is tooling input — garbage is a loud, located error, D-OR3).
mod hex_record {
    use super::COMBAT_RECORD_LEN;
    use serde::de::{Deserialize, Deserializer, Error as _};
    use serde::Serializer;

    pub fn serialize<S: Serializer>(
        bytes: &[u8; COMBAT_RECORD_LEN],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(COMBAT_RECORD_LEN * 2);
        for &b in bytes.iter() {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
        s.serialize_str(&out)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<[u8; COMBAT_RECORD_LEN], D::Error> {
        let s = String::deserialize(d)?;
        let want = COMBAT_RECORD_LEN * 2;
        if s.len() != want {
            return Err(D::Error::custom(format!(
                "combat_entry record must be {want} hex chars ({COMBAT_RECORD_LEN} bytes), got {}",
                s.len()
            )));
        }
        let raw = s.as_bytes();
        let mut out = [0u8; COMBAT_RECORD_LEN];
        for (i, slot) in out.iter_mut().enumerate() {
            let hi =
                hex_digit(raw[2 * i]).ok_or_else(|| D::Error::custom("non-hex char in record"))?;
            let lo = hex_digit(raw[2 * i + 1])
                .ok_or_else(|| D::Error::custom("non-hex char in record"))?;
            *slot = (hi << 4) | lo;
        }
        Ok(out)
    }

    fn hex_digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
}

/// A fully parsed trace: its header plus its events in file (draw) order.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub header: TraceHeader,
    pub events: Vec<TraceEvent>,
}

/// A line-numbered parse failure. A trace is tooling input, not user data —
/// garbage is a loud, located error, never silently tolerated (the `walk.rs`
/// convention).
#[derive(Debug)]
pub enum ParseError {
    /// The file had no non-blank lines at all.
    Empty,
    /// The first non-blank line didn't parse as a header.
    Header(serde_json::Error),
    /// An event line (1-based line number in the file) didn't parse.
    Event {
        line_no: usize,
        err: serde_json::Error,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "trace file has no lines"),
            ParseError::Header(err) => write!(f, "header (line 1): {err}"),
            ParseError::Event { line_no, err } => write!(f, "event (line {line_no}): {err}"),
        }
    }
}

impl std::error::Error for ParseError {}

impl Trace {
    /// Builds a trace from a header and events (the sink/replay path).
    pub fn new(header: TraceHeader, events: Vec<TraceEvent>) -> Self {
        Trace { header, events }
    }

    /// Parses a `.gbxtrace` (JSON-lines). The first non-blank line is the
    /// header; every subsequent non-blank line is an event. Blank lines are
    /// skipped (hand-authoring convenience). Unknown/extra fields on any line
    /// are ignored (liberal reader); an unparseable line is a located error.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut header: Option<TraceHeader> = None;
        let mut events = Vec::new();

        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            if header.is_none() {
                header = Some(serde_json::from_str(line).map_err(ParseError::Header)?);
                continue;
            }
            let event = serde_json::from_str(line).map_err(|err| ParseError::Event {
                line_no: i + 1,
                err,
            })?;
            events.push(event);
        }

        match header {
            Some(header) => Ok(Trace { header, events }),
            None => Err(ParseError::Empty),
        }
    }

    /// Serializes to the **canonical** `.gbxtrace` string: one compact JSON
    /// object per line, header first, terminated by a trailing newline. This is
    /// the byte-hashable form — two traces with equal contents produce
    /// identical bytes (no map iteration, no floats, no insignificant
    /// whitespace). `serde_json::to_string` cannot fail for these types (no
    /// custom `Serialize`, no non-string map keys), but we surface any error
    /// rather than `unwrap` for robustness.
    pub fn to_canonical_string(&self) -> String {
        // Pre-size roughly: header + ~48 bytes/event.
        let mut out = String::with_capacity(64 + self.events.len() * 48);
        out.push_str(&serde_json::to_string(&self.header).expect("header serializes"));
        out.push('\n');
        for event in &self.events {
            out.push_str(&serde_json::to_string(event).expect("event serializes"));
            out.push('\n');
        }
        out
    }

    /// The `prng`-profile draw events, in order — the equality/chain surface.
    /// Skips the `randomize` marker (which the chain check treats specially).
    pub fn rng_events(&self) -> impl Iterator<Item = &RngEvent> {
        self.events.iter().filter_map(|e| match e {
            TraceEvent::Rng(r) => Some(r),
            TraceEvent::Randomize
            | TraceEvent::Init(_)
            | TraceEvent::Pick(_)
            | TraceEvent::Attack(_)
            | TraceEvent::Dmg(_)
            | TraceEvent::Save(_)
            | TraceEvent::Move(_)
            | TraceEvent::Ai(_)
            | TraceEvent::Morale(_)
            | TraceEvent::CombatEntry(_)
            | TraceEvent::RoundSnapshot(_)
            | TraceEvent::TurnSnapshot(_) => None,
        })
    }

    /// The single `combat_entry` snapshot, if the trace carries one (a live
    /// combat capture does; a synthetic prng stream does not). The replay harness
    /// reads its `rng_state` + roster to build the `CombatState`.
    pub fn combat_entry(&self) -> Option<&CombatEntryEvent> {
        self.events.iter().find_map(|e| match e {
            TraceEvent::CombatEntry(c) => Some(c),
            _ => None,
        })
    }

    /// The number of `rng` draw events (excludes the `randomize` marker) — the
    /// count a human bisects against.
    pub fn rng_event_count(&self) -> usize {
        self.rng_events().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> TraceHeader {
        TraceHeader {
            gbxtrace: 1,
            profile: Profile::Prng,
            game: "cotab-v1.3".to_string(),
            seed: 0x1234_5678,
            encounter: "creation-rerolls".to_string(),
            source: "restrike".to_string(),
            notes: None,
        }
    }

    #[test]
    fn header_field_order_and_omitted_notes_are_canonical() {
        let line = serde_json::to_string(&sample_header()).unwrap();
        assert_eq!(
            line,
            r#"{"gbxtrace":1,"profile":"prng","game":"cotab-v1.3","seed":305419896,"encounter":"creation-rerolls","source":"restrike"}"#
        );
    }

    #[test]
    fn rng_event_tag_first_and_optionals_omitted() {
        let e = TraceEvent::Rng(RngEvent {
            before: 0,
            after: 1,
            n: None,
            result: None,
            caller: None,
        });
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"e":"rng","before":0,"after":1}"#
        );

        let e = TraceEvent::Rng(RngEvent {
            before: 10,
            after: 20,
            n: Some(6),
            result: Some(4),
            caller: None,
        });
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"e":"rng","before":10,"after":20,"n":6,"result":4}"#
        );
    }

    #[test]
    fn init_and_pick_events_are_canonical_and_tag_first() {
        let init = TraceEvent::Init(InitEvent {
            combatant_id: 3,
            delay: 5,
            dex_adj: -2,
            surprise: 1,
        });
        assert_eq!(
            serde_json::to_string(&init).unwrap(),
            r#"{"e":"init","combatant_id":3,"delay":5,"dex_adj":-2,"surprise":1}"#
        );

        let pick = TraceEvent::Pick(PickEvent {
            pass: 0,
            combatant_id: 3,
            delay: 5,
            roll: 87,
        });
        assert_eq!(
            serde_json::to_string(&pick).unwrap(),
            r#"{"e":"pick","pass":0,"combatant_id":3,"delay":5,"roll":87}"#
        );
    }

    #[test]
    fn attack_dmg_save_events_are_canonical_and_tag_first() {
        let attack = TraceEvent::Attack(AttackEvent {
            attacker_id: 2,
            target_id: 7,
            roll: 14,
            hit: 1,
        });
        assert_eq!(
            serde_json::to_string(&attack).unwrap(),
            r#"{"e":"attack","attacker_id":2,"target_id":7,"roll":14,"hit":1}"#
        );

        let dmg = TraceEvent::Dmg(DmgEvent {
            attacker_id: 2,
            target_id: 7,
            amount: 11,
            backstab: 0,
        });
        assert_eq!(
            serde_json::to_string(&dmg).unwrap(),
            r#"{"e":"dmg","attacker_id":2,"target_id":7,"amount":11,"backstab":0}"#
        );

        let save = TraceEvent::Save(SaveEvent {
            combatant_id: 3,
            save_type: 4,
            roll: 9,
            made: 0,
        });
        assert_eq!(
            serde_json::to_string(&save).unwrap(),
            r#"{"e":"save","combatant_id":3,"save_type":4,"roll":9,"made":0}"#
        );
    }

    #[test]
    fn move_ai_morale_events_are_canonical_and_tag_first() {
        let mv = TraceEvent::Move(MoveEvent {
            combatant_id: 0,
            from_x: 25,
            from_y: 8,
            to_x: 25,
            to_y: 9,
            cost: 2,
        });
        assert_eq!(
            serde_json::to_string(&mv).unwrap(),
            r#"{"e":"move","combatant_id":0,"from_x":25,"from_y":8,"to_x":25,"to_y":9,"cost":2}"#
        );

        let ai = TraceEvent::Ai(AiEvent {
            combatant_id: 0,
            field_15: 5,
            target_id: -1,
        });
        assert_eq!(
            serde_json::to_string(&ai).unwrap(),
            r#"{"e":"ai","combatant_id":0,"field_15":5,"target_id":-1}"#
        );

        let morale = TraceEvent::Morale(MoraleEvent {
            combatant_id: 0,
            monster_morale: 40,
            enemy_hp_pct: 100,
            roll: 73,
            failed: 0,
        });
        assert_eq!(
            serde_json::to_string(&morale).unwrap(),
            r#"{"e":"morale","combatant_id":0,"monster_morale":40,"enemy_hp_pct":100,"roll":73,"failed":0}"#
        );

        // The reader round-trips them (accepted, tag-first) and they are not draws.
        for line in [
            r#"{"e":"move","combatant_id":0,"from_x":25,"from_y":8,"to_x":25,"to_y":9,"cost":2}"#,
            r#"{"e":"ai","combatant_id":0,"field_15":5,"target_id":-1}"#,
            r#"{"e":"morale","combatant_id":0,"monster_morale":40,"enemy_hp_pct":100,"roll":73,"failed":0}"#,
        ] {
            let parsed: TraceEvent = serde_json::from_str(line).unwrap();
            assert_eq!(serde_json::to_string(&parsed).unwrap(), line);
        }
    }

    #[test]
    fn attack_dmg_save_are_accepted_by_the_reader_and_are_not_draws() {
        let header = serde_json::to_string(&TraceHeader {
            profile: Profile::Action,
            encounter: "attack-slice".to_string(),
            ..sample_header()
        })
        .unwrap();
        let text = format!(
            "{header}\n{}\n{}\n{}\n",
            r#"{"e":"attack","attacker_id":2,"target_id":7,"roll":14,"hit":1}"#,
            r#"{"e":"dmg","attacker_id":2,"target_id":7,"amount":11,"backstab":0}"#,
            r#"{"e":"save","combatant_id":3,"save_type":4,"roll":9,"made":0}"#,
        );
        let trace = Trace::parse(&text).expect("attack/dmg/save are accepted event types");
        assert_eq!(trace.events.len(), 3);
        assert!(matches!(trace.events[0], TraceEvent::Attack(_)));
        assert!(matches!(trace.events[1], TraceEvent::Dmg(_)));
        assert!(matches!(trace.events[2], TraceEvent::Save(_)));
        assert_eq!(trace.rng_event_count(), 0, "action events are not draws");
        assert_eq!(Trace::parse(&trace.to_canonical_string()).unwrap(), trace);
    }

    #[test]
    fn init_and_pick_are_no_longer_unknown_to_the_reader() {
        // Previously an `init`/`pick` `e` value would be a loud parse error;
        // pinning the vocabulary means the reader accepts them.
        let header = serde_json::to_string(&TraceHeader {
            profile: Profile::Action,
            encounter: "init-slice".to_string(),
            ..sample_header()
        })
        .unwrap();
        let text = format!(
            "{header}\n{}\n{}\n",
            r#"{"e":"init","combatant_id":0,"delay":4,"dex_adj":0,"surprise":0}"#,
            r#"{"e":"pick","pass":0,"combatant_id":0,"delay":4,"roll":51}"#,
        );
        let trace = Trace::parse(&text).expect("init/pick are accepted event types");
        assert_eq!(trace.events.len(), 2);
        assert!(matches!(trace.events[0], TraceEvent::Init(_)));
        assert!(matches!(trace.events[1], TraceEvent::Pick(_)));
        // They are not draws.
        assert_eq!(trace.rng_event_count(), 0);
        // Canonical round-trip is a fixed point.
        assert_eq!(Trace::parse(&trace.to_canonical_string()).unwrap(), trace);
    }

    /// D1: the `combat_entry` snapshot parses into a typed struct, round-trips
    /// through the canonical form, and is **not** a draw (its record is
    /// hex-decoded to the fixed 0x1A6 array). Synthetic records only (D10).
    #[test]
    fn combat_entry_parses_round_trips_and_is_not_a_draw() {
        // Two synthetic records: byte i = i (mod 256) shifted, distinct per member.
        let rec = |seed: u8| {
            let mut r = [0u8; COMBAT_RECORD_LEN];
            for (i, b) in r.iter_mut().enumerate() {
                *b = (i as u8).wrapping_add(seed);
            }
            r
        };
        let event = TraceEvent::CombatEntry(CombatEntryEvent {
            rng_state: 0xdead_beef,
            terrain: None,
            area2_field_58c: None,
            combatants: vec![
                CombatEntryCombatant {
                    team: 0,
                    x: 26,
                    y: 12,
                    record: rec(0),
                },
                CombatEntryCombatant {
                    team: 1,
                    x: 34,
                    y: 13,
                    record: rec(7),
                },
            ],
        });

        // Tag-first, `combat_entry`, records as 2·0x1A6 lowercase-hex chars.
        // `terrain` and `area2_field_58c` are both absent → omitted (existing
        // goldens stay byte-identical).
        let line = serde_json::to_string(&event).unwrap();
        assert!(line.starts_with(r#"{"e":"combat_entry","rng_state":3735928559,"combatants":[{"team":0,"x":26,"y":12,"record":"00010203"#));
        assert!(
            !line.contains("area2_field_58c"),
            "the field is omitted when absent"
        );
        // The two records serialize to exactly 2·0x1A6 hex chars each.
        assert_eq!(line.matches("\"record\":\"").count(), 2);

        // The reader accepts it, decodes the array, and it is not a draw.
        let header = serde_json::to_string(&TraceHeader {
            encounter: "combat-entry-slice".to_string(),
            ..sample_header()
        })
        .unwrap();
        let text = format!("{header}\n{line}\n");
        let trace = Trace::parse(&text).expect("combat_entry is an accepted event type");
        assert_eq!(trace.events.len(), 1);
        assert_eq!(trace.rng_event_count(), 0, "combat_entry is not a draw");
        let ce = trace
            .combat_entry()
            .expect("combat_entry accessor finds it");
        assert_eq!(ce.rng_state, 0xdead_beef);
        assert_eq!(ce.combatants.len(), 2);
        assert_eq!(ce.combatants[0].record, rec(0));
        assert_eq!(ce.combatants[1].record, rec(7));
        assert_eq!((ce.combatants[1].team, ce.combatants[1].x), (1, 34));

        // Canonical round-trip is a fixed point.
        assert_eq!(Trace::parse(&trace.to_canonical_string()).unwrap(), trace);

        // When present, `area2_field_58c` is emitted (between `terrain` and
        // `combatants`) and round-trips.
        let with_58c = TraceEvent::CombatEntry(CombatEntryEvent {
            rng_state: 1,
            terrain: None,
            area2_field_58c: Some(50),
            combatants: vec![CombatEntryCombatant {
                team: 0,
                x: 1,
                y: 2,
                record: rec(0),
            }],
        });
        let line = serde_json::to_string(&with_58c).unwrap();
        assert!(line.contains(r#""area2_field_58c":50,"combatants":"#));
        assert_eq!(serde_json::from_str::<TraceEvent>(&line).unwrap(), with_58c);
    }

    /// A `combat_entry` record of the wrong hex length is a loud, located error.
    #[test]
    fn combat_entry_wrong_record_length_is_a_located_error() {
        let header = serde_json::to_string(&sample_header()).unwrap();
        // A record with only 4 hex chars (2 bytes), not 0x1A6.
        let text = format!(
            "{header}\n{}\n",
            r#"{"e":"combat_entry","rng_state":1,"combatants":[{"team":0,"x":1,"y":2,"record":"dead"}]}"#
        );
        match Trace::parse(&text) {
            Err(ParseError::Event { line_no, .. }) => assert_eq!(line_no, 2),
            other => panic!("expected a located event error, got {other:?}"),
        }
    }

    #[test]
    fn randomize_marker_serializes_bare() {
        assert_eq!(
            serde_json::to_string(&TraceEvent::Randomize).unwrap(),
            r#"{"e":"randomize"}"#
        );
    }

    #[test]
    fn canonical_string_round_trips_through_the_parser() {
        let trace = Trace::new(
            sample_header(),
            vec![
                TraceEvent::Rng(RngEvent {
                    before: 0x1234_5678,
                    after: 0xcb5b_9059,
                    n: Some(6),
                    result: Some(1),
                    caller: None,
                }),
                TraceEvent::Rng(RngEvent {
                    before: 0xcb5b_9059,
                    after: 0x79ff_b5be,
                    n: Some(100),
                    result: Some(0x79),
                    caller: None,
                }),
            ],
        );
        let text = trace.to_canonical_string();
        assert!(text.ends_with('\n'));
        let reparsed = Trace::parse(&text).unwrap();
        assert_eq!(reparsed, trace);
        // Canonical form is a fixed point.
        assert_eq!(reparsed.to_canonical_string(), text);
    }

    /// The liberal-reader contract: the staging hook's extra diagnostic fields
    /// (beyond `caller`) and a rich `caller` object must parse without error
    /// and without disturbing the equality surface.
    #[test]
    fn reader_tolerates_unknown_fields_and_rich_caller() {
        let text = concat!(
            r#"{"gbxtrace":1,"profile":"prng","game":"cotab-v1.3","seed":1,"encounter":"e","source":"staging-hook","hook_build":"0.82.2","extra":42}"#,
            "\n",
            r#"{"e":"rng","before":0,"after":1,"n":6,"result":0,"caller":{"seg":38135,"ofs":5610},"ss_sp_words":[1,2],"foo":"bar"}"#,
            "\n",
        );
        let trace = Trace::parse(text).expect("liberal reader must accept extra fields");
        assert_eq!(trace.header.source, "staging-hook");
        let ev = trace.rng_events().next().unwrap();
        assert_eq!(
            (ev.before, ev.after, ev.n, ev.result),
            (0, 1, Some(6), Some(0))
        );
        assert!(ev.caller.is_some());
    }

    #[test]
    fn blank_lines_skipped_and_empty_file_errors() {
        assert!(matches!(Trace::parse("   \n\n"), Err(ParseError::Empty)));
        let text = format!("\n{}\n\n", serde_json::to_string(&sample_header()).unwrap());
        let trace = Trace::parse(&text).unwrap();
        assert!(trace.events.is_empty());
    }

    #[test]
    fn a_malformed_event_line_is_located() {
        let text = format!(
            "{}\n{{\"e\":\"rng\",\"before\":0}}\n",
            serde_json::to_string(&sample_header()).unwrap()
        );
        // Missing `after` — a required field.
        match Trace::parse(&text) {
            Err(ParseError::Event { line_no, .. }) => assert_eq!(line_no, 2),
            other => panic!("expected a located event error, got {other:?}"),
        }
    }
}
