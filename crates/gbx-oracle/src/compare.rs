//! The projection comparator (D-OR3) and the chain-continuity check (D-OR4
//! part B).
//!
//! Two jobs, deliberately separate:
//!
//! 1. [`compare`] — is trace A equal to trace B on the declared projection?
//!    First a **validity gate** (are these even comparable?), then **exact
//!    equality** over the `prng`-profile surface. A header mismatch is
//!    *incomparable*, never merely "unequal" — the distinction matters
//!    (`restrike` vs `staging-hook` with different seeds is a rig error, not a
//!    fidelity finding).
//!
//! 2. [`check_chain`] — is a *single* trace internally consistent? `after_i ==
//!    step(before_i)` and `before_{i+1} == after_i`, with `step =
//!    gbx_prng::Prng::next`. This is what makes the live capture
//!    self-validating: a mid-session reseed, a missed hook hit, or a foreign
//!    write to `DS:0x47F0` becomes a **detected** failure rather than silent
//!    corruption — and it makes D-OR4 part B independent of FD-27 while
//!    confirming it. A `randomize` event in a post-boot trace is likewise a
//!    loud finding.
//!
//! Diagnostics are the deliverable, not a nicety (D-OR3): every failure carries
//! the **draw index**, both sides' values, and the first point of divergence —
//! bisecting a divergence is this profile's stated purpose.

use crate::format::{Profile, Trace, TraceEvent};
use std::fmt;

/// Why two traces cannot be compared at all (the validity gate). Distinct from
/// a value divergence: an incomparable pair is a rig/setup error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incomparable {
    /// Which header field disqualified the pair (the first mismatch found, in
    /// gate order: `gbxtrace`, `profile`, `game`, `seed`, `encounter`).
    pub field: &'static str,
    pub a: String,
    pub b: String,
}

impl fmt::Display for Incomparable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "traces are not comparable: header field `{}` differs (A={}, B={}); \
             `source`/`notes` are ignored, but every other header field must match",
            self.field, self.a, self.b
        )
    }
}

/// The point at which two comparable traces first diverge on the projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// 0-based draw index (position among `rng`-profile draws, matching what a
    /// human counts when bisecting).
    pub index: usize,
    /// The projection field that differs: `"before"`, `"after"`, `"n"`,
    /// `"result"`, `"event-kind"` (one side `rng`, the other `randomize`), or
    /// `"length"` (one side ran out of draws first).
    pub field: &'static str,
    pub a: String,
    pub b: String,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "draw #{}: `{}` differs — A={}, B={}",
            self.index, self.field, self.a, self.b
        )
    }
}

/// The result of a valid comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    Equal,
    Diverged(Divergence),
}

/// Compares two `prng`-profile traces on the declared projection (D-OR3).
///
/// **Validity gate (b):** `gbxtrace`, `profile`, `game`, `seed`, `encounter`
/// must match; `source` and `notes` are ignored. A mismatch returns `Err`.
///
/// **Equality surface (b):** `(before, after)` per draw, extended to
/// `(n, result)` **only when both sides carry them** (the default projection is
/// the intersection of what both emit — D-OR3(c)). `caller` and every other
/// diagnostic field are excluded from equality.
///
/// The traversal is over draw events (the `randomize` marker is not a draw and
/// is not compared here — it is [`check_chain`]'s concern); a `randomize` on
/// one side vs a draw on the other is not reachable through this iterator, so
/// event-kind divergence is reported only for the rare mixed stream and length
/// differences are reported at the first missing draw.
pub fn compare(a: &Trace, b: &Trace) -> Result<Comparison, Incomparable> {
    validity_gate(a, b)?;

    // The `combat_entry` snapshot is replay *input*, not a draw or an action
    // event (D-OR5(b)): it must never count as an event/length mismatch, so it is
    // filtered out before the positional walk (one side may carry it, the other
    // not). Every other event kind keeps its position.
    let a_events: Vec<&TraceEvent> = a
        .events
        .iter()
        .filter(|e| !matches!(e, TraceEvent::CombatEntry(_)))
        .collect();
    let b_events: Vec<&TraceEvent> = b
        .events
        .iter()
        .filter(|e| !matches!(e, TraceEvent::CombatEntry(_)))
        .collect();

    // Draw index counts `rng` events only, so diagnostics match how a human
    // numbers draws when bisecting.
    let mut draw_index = 0usize;
    let n = a_events.len().max(b_events.len());
    for i in 0..n {
        match (a_events.get(i), b_events.get(i)) {
            (Some(TraceEvent::Rng(ra)), Some(TraceEvent::Rng(rb))) => {
                if ra.before != rb.before {
                    return Ok(diverged(draw_index, "before", ra.before, rb.before));
                }
                if ra.after != rb.after {
                    return Ok(diverged(draw_index, "after", ra.after, rb.after));
                }
                // Operand extension: compared only when BOTH sides carry it.
                if let (Some(na), Some(nb)) = (ra.n, rb.n) {
                    if na != nb {
                        return Ok(diverged(draw_index, "n", na, nb));
                    }
                }
                if let (Some(resa), Some(resb)) = (ra.result, rb.result) {
                    if resa != resb {
                        return Ok(diverged(draw_index, "result", resa, resb));
                    }
                }
                draw_index += 1;
            }
            (Some(ea), Some(eb)) => {
                // At least one is a `randomize` marker and they differ in kind.
                if kind(ea) != kind(eb) {
                    return Ok(Comparison::Diverged(Divergence {
                        index: draw_index,
                        field: "event-kind",
                        a: kind(ea).to_string(),
                        b: kind(eb).to_string(),
                    }));
                }
                // Both `randomize` — equal, not a draw.
            }
            (Some(ea), None) => {
                return Ok(Comparison::Diverged(Divergence {
                    index: draw_index,
                    field: "length",
                    a: kind(ea).to_string(),
                    b: "<end of trace>".to_string(),
                }));
            }
            (None, Some(eb)) => {
                return Ok(Comparison::Diverged(Divergence {
                    index: draw_index,
                    field: "length",
                    a: "<end of trace>".to_string(),
                    b: kind(eb).to_string(),
                }));
            }
            (None, None) => unreachable!("i < max(len_a, len_b)"),
        }
    }
    Ok(Comparison::Equal)
}

fn kind(e: &TraceEvent) -> &'static str {
    match e {
        TraceEvent::Rng(_) => "rng",
        TraceEvent::Randomize => "randomize",
        TraceEvent::Init(_) => "init",
        TraceEvent::Pick(_) => "pick",
        TraceEvent::Attack(_) => "attack",
        TraceEvent::Dmg(_) => "dmg",
        TraceEvent::Save(_) => "save",
        TraceEvent::Move(_) => "move",
        TraceEvent::Ai(_) => "ai",
        TraceEvent::Morale(_) => "morale",
        TraceEvent::CombatEntry(_) => "combat_entry",
    }
}

fn diverged<T: fmt::Display>(index: usize, field: &'static str, a: T, b: T) -> Comparison {
    Comparison::Diverged(Divergence {
        index,
        field,
        a: a.to_string(),
        b: b.to_string(),
    })
}

fn validity_gate(a: &Trace, b: &Trace) -> Result<(), Incomparable> {
    let ha = &a.header;
    let hb = &b.header;
    if ha.gbxtrace != hb.gbxtrace {
        return Err(mismatch("gbxtrace", ha.gbxtrace, hb.gbxtrace));
    }
    if ha.profile != hb.profile {
        return Err(mismatch(
            "profile",
            profile_str(ha.profile),
            profile_str(hb.profile),
        ));
    }
    if ha.game != hb.game {
        return Err(mismatch("game", &ha.game, &hb.game));
    }
    if ha.seed != hb.seed {
        return Err(mismatch("seed", ha.seed, hb.seed));
    }
    if ha.encounter != hb.encounter {
        return Err(mismatch("encounter", &ha.encounter, &hb.encounter));
    }
    // `source` and `notes` are deliberately NOT gated (D-OR3): comparing a
    // `restrike` trace against a `staging-hook` trace is the whole point.
    Ok(())
}

fn profile_str(p: Profile) -> &'static str {
    match p {
        Profile::Prng => "prng",
        Profile::Action => "action",
    }
}

fn mismatch<T: fmt::Display>(field: &'static str, a: T, b: T) -> Incomparable {
    Incomparable {
        field,
        a: a.to_string(),
        b: b.to_string(),
    }
}

/// A detected break in a single trace's PRNG chain (D-OR4 part B). Every
/// variant carries the draw index and the values, so a bad live run is a
/// *located* failure, not a bare assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainBreak {
    /// `after_i != step(before_i)`: the recorded post-draw state is not the LCG
    /// step of the recorded pre-draw state — a corrupt or foreign-written
    /// `after` (or a wrong generator on the emitter side).
    StepMismatch {
        index: usize,
        before: u32,
        expected_after: u32,
        actual_after: u32,
    },
    /// `before_{i+1} != after_i`: consecutive draws don't chain — a missed hook
    /// hit (a dropped `AUTOTYPE` keystroke's draw), an interleaved foreign
    /// draw, or a mid-session reseed between the two.
    LinkMismatch {
        /// Index of the *later* draw whose `before` didn't match.
        index: usize,
        prev_after: u32,
        next_before: u32,
    },
    /// A `randomize` event appeared. In a post-boot capture this means the seed
    /// dword was re-written (the original `Randomize`s exactly once, at boot —
    /// oracle-rig §1); a loud finding by itself.
    Randomize { index: usize },
}

impl fmt::Display for ChainBreak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainBreak::StepMismatch {
                index,
                before,
                expected_after,
                actual_after,
            } => write!(
                f,
                "draw #{index}: after != step(before) — before={before:#010x}, \
                 expected after={expected_after:#010x}, recorded after={actual_after:#010x} \
                 (corrupt/foreign-written `after`, or a non-LCG emitter)"
            ),
            ChainBreak::LinkMismatch {
                index,
                prev_after,
                next_before,
            } => write!(
                f,
                "draw #{index}: before != previous after — previous after={prev_after:#010x}, \
                 this before={next_before:#010x} (missed draw, interleaved foreign draw, or \
                 mid-session reseed)"
            ),
            ChainBreak::Randomize { index } => write!(
                f,
                "event #{index}: `randomize` in a post-boot trace — the seed dword was \
                 re-written mid-capture (the original re-seeds only at boot)"
            ),
        }
    }
}

/// Verifies a single trace's PRNG chain (D-OR4 part B). Walks the events in
/// order; returns the **first** break, or `Ok(())` if the whole chain is
/// continuous. `step` is [`gbx_prng::Prng::next`] — the real generator, so the
/// check never re-implements the LCG it validates.
///
/// A `randomize` event returns immediately (it *is* the finding). Between two
/// draws separated only by non-draw events other than `randomize` (none exist
/// this session) the link check still applies to consecutive draws.
pub fn check_chain(trace: &Trace) -> Result<(), ChainBreak> {
    let mut prev_after: Option<u32> = None;
    let mut draw_index = 0usize;

    for (event_index, event) in trace.events.iter().enumerate() {
        match event {
            TraceEvent::Randomize => {
                return Err(ChainBreak::Randomize { index: event_index });
            }
            // Action-profile events carry no PRNG state — skip them (a mixed
            // combined-order trace can interleave `init`/`pick`/`attack`/`dmg`/
            // `save` with draws; the chain links only consecutive `rng` draws).
            TraceEvent::Init(_)
            | TraceEvent::Pick(_)
            | TraceEvent::Attack(_)
            | TraceEvent::Dmg(_)
            | TraceEvent::Save(_)
            | TraceEvent::Move(_)
            | TraceEvent::Ai(_)
            | TraceEvent::Morale(_)
            // The combat_entry snapshot carries no PRNG state; it links the draw
            // before it to the draw after it transparently (D-OR5(b)).
            | TraceEvent::CombatEntry(_) => {}
            TraceEvent::Rng(r) => {
                // Link: this draw's `before` must equal the previous `after`.
                if let Some(prev) = prev_after {
                    if r.before != prev {
                        return Err(ChainBreak::LinkMismatch {
                            index: draw_index,
                            prev_after: prev,
                            next_before: r.before,
                        });
                    }
                }
                // Step: `after` must be the LCG step of `before`.
                let expected = gbx_prng::Prng::new(r.before).next();
                if r.after != expected {
                    return Err(ChainBreak::StepMismatch {
                        index: draw_index,
                        before: r.before,
                        expected_after: expected,
                        actual_after: r.after,
                    });
                }
                prev_after = Some(r.after);
                draw_index += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{RngEvent, TraceHeader};

    fn header(seed: u32, source: &str) -> TraceHeader {
        TraceHeader {
            gbxtrace: 1,
            profile: Profile::Prng,
            game: "cotab-v1.3".to_string(),
            seed,
            encounter: "e".to_string(),
            source: source.to_string(),
            notes: None,
        }
    }

    /// Builds a genuine, chain-continuous trace by running `gbx_prng` — every
    /// `after` is the true step of its `before`, and draws link.
    fn genuine_trace(seed: u32, ns: &[u16], source: &str) -> Trace {
        let mut p = gbx_prng::Prng::new(seed);
        let mut events = Vec::new();
        for &n in ns {
            let before = p.state();
            let result = p.random(n);
            let after = p.state();
            events.push(TraceEvent::Rng(RngEvent {
                before,
                after,
                n: Some(n),
                result: Some(result),
                caller: None,
            }));
        }
        Trace::new(header(seed, source), events)
    }

    #[test]
    fn equal_traces_compare_equal() {
        let a = genuine_trace(42, &[6, 100, 0, 20], "restrike");
        let b = genuine_trace(42, &[6, 100, 0, 20], "staging-hook");
        assert_eq!(compare(&a, &b).unwrap(), Comparison::Equal);
    }

    #[test]
    fn caller_only_difference_still_compares_equal() {
        let a = genuine_trace(42, &[6, 100], "restrike");
        let mut b = genuine_trace(42, &[6, 100], "staging-hook");
        // Give B a rich diagnostic caller on every draw; equality ignores it.
        for e in &mut b.events {
            if let TraceEvent::Rng(r) = e {
                r.caller = Some(serde_json::json!({"seg": 0x8f7, "ofs": 0x15ea}));
            }
        }
        assert_eq!(compare(&a, &b).unwrap(), Comparison::Equal);
    }

    #[test]
    fn a_single_altered_after_fails_at_the_right_index() {
        let a = genuine_trace(42, &[6, 100, 20, 7], "restrike");
        let mut b = genuine_trace(42, &[6, 100, 20, 7], "staging-hook");
        if let TraceEvent::Rng(r) = &mut b.events[2] {
            r.after ^= 0xFF;
        }
        match compare(&a, &b).unwrap() {
            Comparison::Diverged(d) => {
                assert_eq!(d.index, 2);
                assert_eq!(d.field, "after");
            }
            other => panic!("expected divergence, got {other:?}"),
        }
    }

    #[test]
    fn n_and_result_compared_only_when_both_present() {
        let a = genuine_trace(42, &[6, 100], "restrike");
        // B carries no n/result at all (a state-only emitter).
        let mut b = genuine_trace(42, &[6, 100], "staging-hook");
        for e in &mut b.events {
            if let TraceEvent::Rng(r) = e {
                r.n = None;
                r.result = None;
            }
        }
        // Still equal: the intersection projection is (before, after) only.
        assert_eq!(compare(&a, &b).unwrap(), Comparison::Equal);

        // But when both carry n and it differs, it's caught.
        let a2 = genuine_trace(42, &[6], "restrike");
        let mut b2 = genuine_trace(42, &[6], "staging-hook");
        if let TraceEvent::Rng(r) = &mut b2.events[0] {
            r.n = Some(7); // lie about the operand; before/after still match
        }
        match compare(&a2, &b2).unwrap() {
            Comparison::Diverged(d) => assert_eq!(d.field, "n"),
            other => panic!("expected n divergence, got {other:?}"),
        }
    }

    #[test]
    fn header_mismatch_is_incomparable_not_unequal() {
        let a = genuine_trace(42, &[6], "restrike");
        let b = genuine_trace(43, &[6], "staging-hook"); // different seed
        let err = compare(&a, &b).unwrap_err();
        assert_eq!(err.field, "seed");
    }

    #[test]
    fn source_and_notes_do_not_block_comparison() {
        let mut a = genuine_trace(42, &[6, 100], "restrike");
        a.header.notes = Some("ours".to_string());
        let mut b = genuine_trace(42, &[6, 100], "staging-hook");
        b.header.notes = Some("theirs".to_string());
        assert_eq!(compare(&a, &b).unwrap(), Comparison::Equal);
    }

    #[test]
    fn a_length_difference_is_reported() {
        let a = genuine_trace(42, &[6, 100, 20], "restrike");
        let b = genuine_trace(42, &[6, 100], "staging-hook");
        match compare(&a, &b).unwrap() {
            Comparison::Diverged(d) => {
                assert_eq!(d.field, "length");
                assert_eq!(d.index, 2);
            }
            other => panic!("expected length divergence, got {other:?}"),
        }
    }

    #[test]
    fn genuine_trace_has_a_continuous_chain() {
        let t = genuine_trace(0xdead_beef, &[6, 100, 0, 1, 255, 20], "restrike");
        assert_eq!(check_chain(&t), Ok(()));
    }

    #[test]
    fn a_broken_step_is_caught() {
        let mut t = genuine_trace(42, &[6, 100, 20], "restrike");
        if let TraceEvent::Rng(r) = &mut t.events[1] {
            r.after = r.after.wrapping_add(1); // after no longer = step(before)
        }
        match check_chain(&t) {
            Err(ChainBreak::StepMismatch { index, .. }) => assert_eq!(index, 1),
            other => panic!("expected StepMismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_broken_link_is_caught() {
        // Two independently genuine draws that don't chain to each other: a
        // dropped draw between them. Each passes the step check, but draw 1's
        // `before` won't equal draw 0's `after`.
        let mut p = gbx_prng::Prng::new(42);
        let b0 = p.state();
        let r0 = p.random(6);
        let a0 = p.state();
        // skip a draw
        p.next();
        let b1 = p.state();
        let r1 = p.random(6);
        let a1 = p.state();
        let t = Trace::new(
            header(42, "staging-hook"),
            vec![
                TraceEvent::Rng(RngEvent {
                    before: b0,
                    after: a0,
                    n: Some(6),
                    result: Some(r0),
                    caller: None,
                }),
                TraceEvent::Rng(RngEvent {
                    before: b1,
                    after: a1,
                    n: Some(6),
                    result: Some(r1),
                    caller: None,
                }),
            ],
        );
        match check_chain(&t) {
            Err(ChainBreak::LinkMismatch { index, .. }) => assert_eq!(index, 1),
            other => panic!("expected LinkMismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_post_boot_randomize_is_a_loud_finding() {
        let mut t = genuine_trace(42, &[6, 100], "staging-hook");
        t.events.insert(1, TraceEvent::Randomize);
        match check_chain(&t) {
            Err(ChainBreak::Randomize { index }) => assert_eq!(index, 1),
            other => panic!("expected Randomize finding, got {other:?}"),
        }
    }
}
