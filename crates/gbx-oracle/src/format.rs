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
/// (`attack`/`dmg`/`save`); the remaining action types
/// (`move`/`ai`/`status`/`morale`/`award`) are still absent and their sessions
/// add them. An unknown `e` value is rejected loudly by the reader (a foreign
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
            | TraceEvent::Save(_) => None,
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
