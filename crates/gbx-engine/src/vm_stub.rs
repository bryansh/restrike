//! The M2 step-3 VM stand-in (task deliverable 3): every flow stage that
//! would run a real ECL vector instead pulls from a test-scripted sequence
//! of [`gbx_vm::VmStep`]s. Step 4 swaps the real `EclMachine` (+ a real
//! `VmHost`) in behind the same `enter`/`advance` call shape, so
//! `shell.rs`'s flow-stage logic doesn't need to change at that boundary —
//! only what sits behind [`StubVm`] does.
//!
//! Simplification (flagged, in scope for step 3 only): a real `EclMachine`
//! distinguishes `step()` (illegal while a request is pending) from
//! `resume()` (illegal otherwise, `docs/design/vm-scriptmemory.md` §3);
//! this stub doesn't need that distinction, since its "instruction stream"
//! is a pre-authored sequence a test wrote in one shot — [`StubVm::advance`]
//! always just pops the next scripted step, whatever the caller most
//! recently did with a parked widget/reply.

use gbx_vm::VmStep;
use std::collections::VecDeque;

/// A scripted vector-run stand-in.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StubVm {
    calls: VecDeque<VecDeque<VmStep>>,
    active: Option<VecDeque<VmStep>>,
}

impl StubVm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test setup: scripts one vector-run's full outcome in advance. Must
    /// end in a `VmStep::Done(..)` — [`StubVm::advance`] panics if a run is
    /// exhausted without one (a fixture bug, not a runtime condition).
    pub fn script_call(&mut self, steps: Vec<VmStep>) {
        self.calls.push_back(steps.into());
    }

    /// Begins the next scripted call ("enters" a vector, `machine.enter`'s
    /// analogue). Panics if the test forgot to script one.
    pub fn enter(&mut self) {
        let call = self.calls.pop_front().unwrap_or_else(|| {
            panic!(
                "StubVm::enter: no scripted call queued — the test fixture is missing a script_call"
            )
        });
        self.active = Some(call);
    }

    /// Pops the next step of the active call — after `enter`, or after a
    /// parked widget resolved and the flow keeps pumping the *same* run.
    /// Panics if nothing is active, or the active run is exhausted without a
    /// `Done` (both fixture bugs).
    pub fn advance(&mut self) -> VmStep {
        let steps = self
            .active
            .as_mut()
            .expect("StubVm::advance: no active run (call enter() first)");
        steps.pop_front().unwrap_or_else(|| {
            panic!("StubVm::advance: scripted call ran out of steps without a Done — fixture bug")
        })
    }

    /// How many *unstarted* calls remain scripted — a test-authoring
    /// convenience for asserting a fixture's script was fully consumed.
    pub fn calls_remaining(&self) -> usize {
        self.calls.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_vm::Exit;

    #[test]
    fn advances_through_a_scripted_call_in_order() {
        let mut vm = StubVm::new();
        vm.script_call(vec![VmStep::Continue, VmStep::Done(Exit::Ended)]);
        vm.enter();
        assert_eq!(vm.advance(), VmStep::Continue);
        assert_eq!(vm.advance(), VmStep::Done(Exit::Ended));
    }

    #[test]
    #[should_panic(expected = "no scripted call queued")]
    fn enter_without_a_script_panics() {
        StubVm::new().enter();
    }

    #[test]
    #[should_panic(expected = "ran out of steps without a Done")]
    fn advance_past_the_end_of_a_call_panics() {
        let mut vm = StubVm::new();
        vm.script_call(vec![VmStep::Continue]);
        vm.enter();
        vm.advance();
        vm.advance();
    }

    #[test]
    fn calls_remaining_counts_unstarted_scripted_calls() {
        let mut vm = StubVm::new();
        vm.script_call(vec![VmStep::Done(Exit::Ended)]);
        vm.script_call(vec![VmStep::Done(Exit::Ended)]);
        assert_eq!(vm.calls_remaining(), 2);
        vm.enter();
        assert_eq!(vm.calls_remaining(), 1);
    }
}
