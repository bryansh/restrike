//! §4 conformance suite (`docs/design/vm-scriptmemory.md`): micro-ECL
//! programs, hand-authored via `EclBuilder` (D10 — nothing here is derived
//! from real game data), asserting on the yielded step stream, memory
//! traffic (`TestHost::calls`), service-call recordings, flag state, and pc
//! trajectory. Every opcode this session implements gets at least one test
//! citing its coab handler; the mandatory cross-cutting test classes (skip
//! semantics, string-register staleness, suspension, effect/request
//! ordering, `ChainTo`, `VmError` legality) each get their own module.

use crate::dialect::COTAB;
use crate::host::{MissingData, MonsterHandle, PlayerId, RecordedCall};
use crate::test_support::{EclBuilder, TestHost};
use crate::{BlockId, EclMachine, Effect, Exit, Reply, Request, VmError, VmStep, VmString};

fn machine_from(b: &EclBuilder, entry: u16) -> EclMachine {
    let mut m = EclMachine::load_block(b.build(), &COTAB).unwrap();
    m.enter(entry);
    m
}

/// Runs `step()` until the activation completes or a non-`Continue` result
/// appears, panicking on `Effect`/`Request`/`Err` — for fixtures built
/// entirely from opcodes that never suspend or emit effects.
fn run_until_done(m: &mut EclMachine, h: &mut TestHost) -> Exit {
    loop {
        match m.step(h).expect("step should not error") {
            VmStep::Continue => continue,
            VmStep::Done(exit) => return exit,
            other => panic!("expected Continue or Done, got {other:?}"),
        }
    }
}

fn assert_continue(r: Result<VmStep, VmError>) {
    assert_eq!(r, Ok(VmStep::Continue));
}

mod opcodes {
    use super::*;

    /// EXIT (0x00), `CMD_Exit` ovr003.cs:9-42.
    #[test]
    fn exit_ends_the_activation() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x00); // EXIT
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended)));
        assert!(m.is_idle());
    }

    /// EXIT clears the shared GOSUB call stack (`vmCallStack.Clear()`,
    /// ovr003.cs:37) — observed indirectly: a GOSUB pushes a return site,
    /// the callee EXITs (clearing it), and a *later, independent*
    /// activation's RETURN then sees an empty stack (falls through to its
    /// own EXIT) rather than popping the stale entry.
    #[test]
    fn exit_clears_shared_call_stack() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x02).imm_word_label("sub"); // GOSUB sub
        b.label("sub");
        b.op(0x00); // EXIT — clears vmCallStack (which has 1 entry)
        b.label("return_probe");
        b.op(0x13); // RETURN, run later as its own activation

        let entry = b.addr_of("entry");
        let return_probe = b.addr_of("return_probe");
        let mut m = EclMachine::load_block(b.build(), &COTAB).unwrap();
        let mut h = TestHost::new();

        m.enter(entry);
        assert_continue(m.step(&mut h)); // GOSUB: call_stack now [after-gosub addr]
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended))); // EXIT: clears it
        assert!(m.is_idle());

        m.enter(return_probe);
        // If the call stack still held the GOSUB's return site, this would
        // jump there (Continue) instead of falling through to EXIT.
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended)));
    }

    /// GOTO (0x01), `CMD_Goto` ovr003.cs:45-53: jumps to the operand's raw
    /// `.Word`, no fall-through successor.
    #[test]
    fn goto_jumps_to_raw_target_word() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x01).imm_word_label("target"); // GOTO target
        b.label("dead");
        b.op(0x00); // never reached
        b.label("target");
        b.op(0x00); // EXIT, reached only via the jump

        let entry = b.addr_of("entry");
        let target = b.addr_of("target");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(target));
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended)));
    }

    /// GOSUB (0x02) / RETURN (0x13), `CMD_Gosub` ovr003.cs:56-65 /
    /// `CMD_Return` ovr003.cs:420-435: GOSUB pushes the fall-through address
    /// (coab's already-advanced `ecl_offset`) as RETURN's landing site.
    #[test]
    fn gosub_then_return_lands_on_the_fallthrough_address() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x02).imm_word_label("sub"); // GOSUB sub
        b.label("after_gosub");
        b.op(0x00); // EXIT — the return site
        b.label("sub");
        b.op(0x13); // RETURN

        let entry = b.addr_of("entry");
        let after_gosub = b.addr_of("after_gosub");
        let sub = b.addr_of("sub");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(sub));
        assert_continue(m.step(&mut h)); // RETURN
        assert_eq!(m.current_pc(), Some(after_gosub));
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended)));
    }

    /// RETURN (0x13) on an empty call stack silently becomes EXIT, full
    /// side effects included (`ovr003.cs:430-433`).
    #[test]
    fn return_with_empty_call_stack_becomes_exit() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x13); // RETURN, no prior GOSUB
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended)));
    }

    /// COMPARE (0x03), `CMD_Compare` ovr003.cs:68-87, numeric path: flags
    /// derive from `operand1 OP operand2` (natural order, once
    /// `compare_variables(value_b, value_a)`'s swapped argument names are
    /// unwound). Observed via a subsequent `IF >` executing (not skipping)
    /// its next instruction.
    #[test]
    fn compare_sets_relational_flags_true_branch_executes() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(5).imm_byte(3); // COMPARE 5, 3 -> 5>3 true
        b.op(0x19); // IF > : true -> does not skip
        b.op(0x09).imm_byte(1).imm_word(0x4B00); // SAVE 1 -> 0x4B00 (probe)
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(m.flags(), [false, true, false, true, false, true]);
        assert_eq!(h.word(0x4B00), Some(1));
    }

    /// Same COMPARE, but probing the false branch: `IF <` on `5, 3` is
    /// false, so it skips the probing SAVE (size 2, non-divergent) and the
    /// memory cell is never written.
    #[test]
    fn compare_sets_relational_flags_false_branch_skips() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(5).imm_byte(3); // COMPARE 5, 3
        b.op(0x18); // IF < : false -> skips next (SAVE, skip_size 2)
        b.op(0x09).imm_byte(1).imm_word(0x4B00); // SAVE probe (skipped)
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), None);
    }

    /// ADD (0x04), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 4. Destination
    /// is the raw `.Word` of operand 3.
    #[test]
    fn add_writes_sum_to_destination() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x04).imm_byte(5).imm_byte(3).imm_word(0x4B00); // ADD 5,3 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(8));
    }

    /// SUBTRACT (0x05), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 5: result
    /// is operand2 minus operand1 (B−A), not A−B.
    #[test]
    fn subtract_result_is_operand2_minus_operand1() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x05).imm_byte(3).imm_byte(10).imm_word(0x4B00); // SUBTRACT 3,10 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(7)); // 10 - 3, not 3 - 10
    }

    /// MULTIPLY (0x07), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 7.
    #[test]
    fn multiply_writes_product_to_destination() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x07).imm_byte(6).imm_byte(7).imm_word(0x4B00); // MULTIPLY 6,7 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(42));
    }

    /// DIVIDE (0x06), `CMD_AddSubDivMulti` ovr003.cs:90-130 case 6: the
    /// quotient writes to the operand-3 destination; the remainder writes
    /// through the ordinary `ScriptMemory` facade at the Party-window alias
    /// address `0x7F3F` (opcode-classification.md docket item 2).
    #[test]
    fn divide_writes_quotient_to_destination_and_remainder_to_0x7f3f() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x06).imm_byte(17).imm_byte(5).imm_word(0x4B00); // DIVIDE 17,5 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(3)); // 17 / 5
        assert_eq!(h.word(0x7F3F), Some(2)); // 17 % 5
    }

    /// DIVIDE by zero: coab's `val_a / val_b` uses C#'s integer division,
    /// which throws `DivideByZeroException` uncaught anywhere up the
    /// `RunEclVm` call chain — the original crashes. Modeled as a defined
    /// `VmError`, not a Rust panic.
    #[test]
    fn divide_by_zero_is_a_defined_error_not_a_panic() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x06).imm_byte(9).imm_byte(0).imm_word(0x4B00); // DIVIDE 9,0 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let err = m
            .step(&mut h)
            .expect_err("division by zero must be a defined error");
        assert!(matches!(
            err,
            VmError::DivisionByZero {
                pc,
                opcode: 0x06
            } if pc == entry
        ));
    }

    /// The shipped pattern this opcode's implementation exists to unblock:
    /// `ECL2.DAX` block 1's per-step script executes `0x8295: DIVIDE
    /// mem=0x7F7B, imm=0x08 -> mem=0x7F80` immediately followed by `0x829E:
    /// GETTABLE base=0x9DB8 index=mem[0x7F3F] -> 0x7E7A` — a modulo-8 table
    /// lookup whose index is DIVIDE's out-of-band remainder, read back
    /// through the ordinary Party-window address (docket item 2's confirmed
    /// live example). Mirrors every real address except GETTABLE's base:
    /// the real `0x9DB8` sits *inside* the ECL window (`0x8000..=0x9DFF`),
    /// which the VM intercepts against its own resident block bytes before
    /// ever reaching `ScriptMemory` (`host.rs`'s module doc) — the real
    /// table lives in the shipped block's own data, not a `TestHost`-mocked
    /// window. This fixture uses a Table-window base (`0x9E00`, just past
    /// the ECL window) instead, so the test exercises the DIVIDE→0x7F3F→
    /// GETTABLE data flow through the same host-visible facade a resident
    /// mock can assert on, without fabricating in-block table bytes.
    #[test]
    fn divide_then_gettable_via_0x7f3f_mirrors_the_shipped_pattern() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x06).mem(0x7F7B).imm_byte(8).mem(0x7F80); // DIVIDE mem[0x7F7B],8 -> mem[0x7F80]
        b.op(0x2A).imm_word(0x9E00).mem(0x7F3F).mem(0x7E7A); // GETTABLE base=0x9E00 idx=mem[0x7F3F] -> mem[0x7E7A]
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x7F7B, 19); // dividend
        h.set_word(0x9E00 + 3, 0xBEEF); // table[remainder] sentinel, 19 % 8 == 3

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x7F80), Some(2)); // 19 / 8
        assert_eq!(h.word(0x7F3F), Some(3)); // 19 % 8, the alias GETTABLE indexes with
        assert_eq!(h.word(0x7E7A), Some(0xBEEF));
    }

    /// RANDOM (0x08), `CMD_Random` ovr003.cs:132-151: the inclusive-bound
    /// adjustment increments the operand unless it's already `0xFF`.
    #[test]
    fn random_applies_inclusive_bound_adjustment_and_writes_roll() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x08).imm_byte(0x0A).imm_word(0x4B00); // RANDOM max=10 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.roll_replies.push_back(7);

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::Roll { max: 0x0B }));
        assert_eq!(h.word(0x4B00), Some(7));
    }

    #[test]
    fn random_does_not_increment_an_already_maximal_bound() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x08).imm_byte(0xFF).imm_word(0x4B00);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.roll_replies.push_back(1);

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::Roll { max: 0xFF }));
    }

    /// SAVE (0x09), `CMD_Save` ovr003.cs:153-172, numeric branch.
    #[test]
    fn save_numeric_writes_through_memory() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x09).imm_byte(9).imm_word(0x4B00); // SAVE 9 -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(9));
    }

    /// SAVE (0x09), string branch: writes the register slot operand 1 just
    /// filled (`ovr003.cs:166-169`), not a stale one.
    #[test]
    fn save_string_writes_the_freshly_filled_register() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x09).inline_str(b"HI").imm_word(0x4B00); // SAVE "HI" -> 0x4B00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::MemWriteString {
            addr: 0x4B00,
            value: VmString::from_bytes(*b"HI"),
            origin: crate::Origin { pc: entry },
        }));
    }

    /// SAVE (0x09) with a `mem_str` (mode `0x81`)-encoded *destination* —
    /// real shipped content (`ECL2.DAX` block 1, `0x8328`/`0x833D`): a
    /// destination operand's raw word must resolve regardless of its own
    /// encoded mode (docket item 3), and `MemStr` carries one just like
    /// `Mem`/`ImmWord` (`Arg::raw_word`'s doc comment) — this used to be
    /// `VmError::UnresolvedOperand` before that fix.
    #[test]
    fn save_string_to_a_mem_str_encoded_destination_resolves() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x09).inline_str(b"HI").mem_str(0x7B89); // SAVE "HI" -> mem_str 0x7B89
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::MemWriteString {
            addr: 0x7B89,
            value: VmString::from_bytes(*b"HI"),
            origin: crate::Origin { pc: entry },
        }));
    }

    /// LOAD MONSTER (0x0B), `CMD_LoadMonster` ovr003.cs:238-297: all 3
    /// operands bundle into one `EngineServices` call (`host.rs`'s trait
    /// doc comment explains the departure from the classification draft).
    #[test]
    fn load_monster_bundles_all_three_operands() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0B).imm_byte(5).imm_byte(2).imm_byte(9); // LOAD MONSTER 5,2,9
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.load_monster_replies.push_back(Ok(MonsterHandle(1)));

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::LoadMonster {
            monster_id: 5,
            num_copies: 2,
            icon_block_id: 9,
        }));
    }

    /// LOAD MONSTER (0x0B) with a missing `.dax` asset: the original's hard
    /// `print_and_exit()` (docket item 4) is modeled as a halting
    /// `VmError::MissingAsset`, not a panic and not a silent continue.
    #[test]
    fn load_monster_missing_asset_halts_the_machine() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0B).imm_byte(200).imm_byte(1).imm_byte(1);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.load_monster_replies.push_back(Err(MissingData));

        let err = m.step(&mut h).unwrap_err();
        assert_eq!(
            err,
            VmError::MissingAsset {
                pc: entry,
                opcode: 0x0B
            }
        );
        // Halted: the pc didn't move, so stepping again reproduces exactly
        // the same error (D-VM6's "the machine is halted" contract).
        h.load_monster_replies.push_back(Err(MissingData));
        assert_eq!(m.step(&mut h).unwrap_err(), err);
    }

    /// SETUP MONSTER (0x0C), `CMD_SetupMonster` ovr003.cs:215-236: the three
    /// stores, the ray, the clamp into `area2_ptr.encounter_distance`, and the
    /// encounter-visual dispatch — in that order, with the draw travelling as
    /// an effect.
    #[test]
    fn setup_monster_stores_clamps_and_dispatches_the_encounter_visual() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0C).imm_byte(1).imm_byte(2).imm_byte(3); // sprite,max_dist,pic
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.approach_distance_replies.push_back(10);

        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Effect(Effect::EncounterVisual)),
            "`:235`'s sub_30580 draw travels as an effect"
        );
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));

        let want = [
            RecordedCall::SetupMonster {
                sprite_id: 1,
                max_distance: 2,
                pic_id: 3,
            },
            RecordedCall::ApproachDistance,
            // Clamped by max_distance (2), matching `if (max_distance <
            // encounter_distance) encounter_distance = max_distance;`.
            RecordedCall::SetEncounterDistance { value: 2 },
            RecordedCall::LoadEncounterVisual,
        ];
        assert_eq!(h.calls, want);
    }

    /// The clamp is ONE-sided (`ovr003.cs:231-234`): a `max_distance` operand
    /// larger than the ray leaves the ray's own value standing.
    #[test]
    fn setup_monster_clamp_is_one_sided() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0C).imm_byte(1).imm_byte(9).imm_byte(3); // max_dist 9 > ray 1
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.approach_distance_replies.push_back(1);

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::EncounterVisual)));
        assert!(h
            .calls
            .contains(&RecordedCall::SetEncounterDistance { value: 1 }));
    }

    /// APPROACH (0x0D), `CMD_Approach` ovr003.cs:300-310: decrement and
    /// re-dispatch.
    #[test]
    fn approach_decrements_the_encounter_distance_and_redispatches() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0D); // APPROACH
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.encounter_distance = 2;

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::EncounterVisual)));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(h.encounter_distance, 1);
        assert_eq!(
            h.calls,
            [
                RecordedCall::EncounterDistance,
                RecordedCall::SetEncounterDistance { value: 1 },
                RecordedCall::LoadEncounterVisual,
            ]
        );
    }

    /// At distance 0 the whole body is skipped — but the pc still advances
    /// (`ecl_offset++` sits OUTSIDE the `if`, `ovr003.cs:309`), and no visual
    /// is dispatched.
    #[test]
    fn approach_at_distance_zero_is_a_pure_advance() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0D); // APPROACH
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.encounter_distance = 0;

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(h.calls, [RecordedCall::EncounterDistance]);
    }

    /// SPRITE OFF (0x31), `CMD_SpriteOff` ovr003.cs:1707-1717: the guarded
    /// `RedrawView()` when a sprite is up…
    #[test]
    fn sprite_off_with_a_sprite_up_yields_a_redraw() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x31); // SPRITE OFF
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.display_player_sprite = true;

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::RedrawView)));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert!(!h.display_player_sprite, "the service clears it");
    }

    /// …and nothing at all when none is.
    #[test]
    fn sprite_off_without_a_sprite_is_a_pure_advance() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x31);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(h.calls, [RecordedCall::SpriteOff]);
    }

    /// PARTYSTRENGTH (0x1D), `CMD_PartyStrength` ovr003.cs:772-810: the
    /// service's power value written to the operand's RAW `.Word`
    /// (`gbl.cmd_opps[1].Word`, `:808`) — a destination address, not a
    /// resolved value. `ECL6#66 @0x827B` writes `0x7F7A`.
    #[test]
    fn party_strength_writes_the_power_value_to_the_operand_word() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x1D).mem(0x7F7A);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.party_strength_replies.push_back(0x2A);

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(h.word(0x7F7A), Some(0x2A));
        assert!(h.calls.contains(&RecordedCall::PartyStrength));
    }

    // --- ENCOUNTER MENU (0x29) ---------------------------------------------

    /// `area_ptr.inDungeon`'s window address, which the opcode reads to pick
    /// PARLAY-vs-ADVANCE and to decide whether the text region is cleared.
    const IN_DUNGEON_ADDR: u16 = 0x4BE6;
    const RESULT_CELL: u16 = 0x7F79;

    /// A 14-operand ENCOUNTER MENU shaped like the shipped ones: sprite/max/pic
    /// ids, the result cell, five outcome classes, three approach lines, and
    /// the two movement thresholds.
    fn encounter_menu_block(
        max_distance: u8,
        outcomes: [u8; 5],
        texts: [&[u8]; 3],
        party_flee: u8,
        monster_flee: u8,
    ) -> EclBuilder {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x29)
            .imm_byte(0x22) // 1: sprite block
            .imm_byte(max_distance) // 2: max encounter distance
            .imm_byte(0x22) // 3: pic block
            .mem(RESULT_CELL); // 4: result cell
        for o in outcomes {
            b.imm_byte(o); // 5..9
        }
        for t in texts {
            b.inline_str(t); // 10..12
        }
        b.imm_byte(party_flee).imm_byte(monster_flee); // 13, 14
        b.label("after");
        b.op(0x00);
        b
    }

    /// Drives the opcode to its first suspension, returning the effects it
    /// emitted on the way and the request it parked on.
    fn run_to_request(m: &mut EclMachine, h: &mut TestHost) -> (Vec<Effect>, Request) {
        let mut effects = Vec::new();
        for _ in 0..16 {
            match m.step(h).expect("step should not error") {
                VmStep::Effect(e) => effects.push(e),
                VmStep::Request(r) => return (effects, r),
                VmStep::Continue => continue,
                other => panic!("expected Effect/Request, got {other:?}"),
            }
        }
        panic!("the menu never opened");
    }

    fn words(request: &Request) -> Vec<String> {
        let Request::HorizontalMenu { options } = request else {
            panic!("expected a horizontal menu, got {request:?}")
        };
        options
            .iter()
            .map(|s| String::from_utf8_lossy(&s.0).into_owned())
            .collect()
    }

    /// The preamble (`ovr003.cs:1245-1279`): the menu flag goes up first, the
    /// movement pair is sampled once, the three ids are stashed, the ray is
    /// clamped into `encounter_distance`, and one encounter visual is
    /// dispatched — then the approach line prints and the menu opens.
    #[test]
    fn encounter_menu_preamble_arms_the_flag_clamps_the_ray_and_opens_the_menu() {
        let b = encounter_menu_block(2, [2, 1, 2, 3, 4], [b"THEY APPROACH", b"", b""], 0x32, 0x32);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((9, 14));

        let (effects, request) = run_to_request(&mut m, &mut h);

        assert_eq!(
            h.calls[0],
            RecordedCall::SetEncounterMenuActive { active: true },
            "`:1245` runs before anything else, so `sub_30580` sees it"
        );
        assert_eq!(h.calls[1], RecordedCall::CalcGroupMovement, "`:1250`");
        assert!(h.calls.contains(&RecordedCall::SetupMonster {
            sprite_id: 0x22,
            max_distance: 2,
            pic_id: 0x22,
        }));
        assert_eq!(h.encounter_distance, 2, "the ray, clamped by max");
        assert_eq!(
            effects,
            vec![
                Effect::EncounterVisual, // `:1279`
                Effect::Print {
                    text: VmString(b"THEY APPROACH".to_vec()),
                    clear_first: true, // inDungeon != 0 (`:1294`)
                },
            ]
        );
        assert_eq!(words(&request), ["COMBAT", "WAIT", "FLEE", "ADVANCE"]);
    }

    /// `:1348-1355` — the fourth word is PARLAY once the monsters are adjacent,
    /// and `:1363-1368` then resolves that word to slot **4**, not slot 3.
    /// `ECL4#32 @0x98A9` is exactly this shape (`max_distance = 0`,
    /// `var_6 = [0,3,0,0,3]`), and its slot-4 class 3 writes the parlay
    /// outcome.
    #[test]
    fn at_distance_zero_the_fourth_word_is_parlay_and_resolves_to_slot_four() {
        let b = encounter_menu_block(0, [0, 3, 0, 0, 3], [b"A VOICE CALLS", b"", b""], 0x0C, 0x0C);
        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((12, 12));

        let (_, request) = run_to_request(&mut m, &mut h);
        assert_eq!(h.encounter_distance, 0, "max_distance 0 clamps the ray");
        assert_eq!(words(&request), ["COMBAT", "WAIT", "FLEE", "PARLAY"]);

        // Selecting the fourth word (index 3) resolves to slot 4 -> class 3,
        // whose distance-0 arm writes 3.
        assert_continue(m.resume(Reply::Selection(3), &mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(h.word(RESULT_CELL), Some(3));
        assert_eq!(
            *h.calls.last().unwrap(),
            RecordedCall::SetEncounterMenuActive { active: false },
            "`:1536`"
        );
    }

    /// Class 0's FLEE arm (`:1382-1392`): the party's SLOWEST member is the one
    /// that has to make the operand-13 threshold.
    #[test]
    fn class_zero_flee_gates_on_the_partys_slowest_member() {
        for (slowest, want) in [(0x0Cu8, 2u16), (0x0B, 1)] {
            let b = encounter_menu_block(0, [0, 3, 0, 0, 3], [b"HALT", b"", b""], 0x0C, 0x0C);
            let entry = b.addr_of("entry");
            let mut m = machine_from(&b, entry);
            let mut h = TestHost::new();
            h.set_word(IN_DUNGEON_ADDR, 1);
            h.approach_distance_replies.push_back(0);
            h.calc_group_movement_replies.push_back((slowest, 99));

            run_to_request(&mut m, &mut h);
            assert_continue(m.resume(Reply::Selection(2), &mut h)); // FLEE
            assert_eq!(
                h.word(RESULT_CELL),
                Some(want),
                "slowest {slowest:#04X} vs threshold 0x0C"
            );
        }
    }

    /// Class 2's COMBAT arm (`:1441-1454`): the monsters break off when their
    /// operand-14 rating beats the party's FASTEST member — and say so.
    #[test]
    fn class_two_combat_lets_the_monsters_outrun_the_partys_fastest() {
        let b = encounter_menu_block(0, [2, 2, 2, 2, 2], [b"GOBLINS", b"", b""], 0, 20);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(0);
        h.calc_group_movement_replies.push_back((5, 19)); // fastest 19 < 20

        run_to_request(&mut m, &mut h);
        assert_eq!(
            m.resume(Reply::Selection(0), &mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString(b"The monsters flee.".to_vec()),
                clear_first: true,
            }))
        );
        assert_eq!(h.word(RESULT_CELL), Some(0), "written before the print");
        assert_continue(m.step(&mut h));

        // …and stand their ground when the party is faster.
        let b = encounter_menu_block(0, [2, 2, 2, 2, 2], [b"GOBLINS", b"", b""], 0, 20);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(0);
        h.calc_group_movement_replies.push_back((5, 20)); // fastest 20, not < 20
        run_to_request(&mut m, &mut h);
        assert_continue(m.resume(Reply::Selection(0), &mut h));
        assert_eq!(h.word(RESULT_CELL), Some(1), "fight");
    }

    /// Class 1's WAIT arm (`:1402-1406`): "Both sides wait." and the menu
    /// re-opens — the `init_max = 1` loop, the thing that makes this opcode
    /// structurally unlike every other menu.
    #[test]
    fn class_one_wait_reopens_the_menu_after_saying_so() {
        let b = encounter_menu_block(2, [1, 1, 1, 1, 1], [b"ORCS", b"", b""], 0, 0);
        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((9, 9));

        run_to_request(&mut m, &mut h);
        assert_eq!(
            m.resume(Reply::Selection(1), &mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString(b"Both sides wait.".to_vec()),
                clear_first: true,
            }))
        );
        // The next iteration's own approach line, then the menu again.
        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString(b"ORCS".to_vec()),
                clear_first: true,
            }))
        );
        let VmStep::Request(request) = m.step(&mut h).unwrap() else {
            panic!("the menu must re-open")
        };
        assert_eq!(words(&request), ["COMBAT", "WAIT", "FLEE", "ADVANCE"]);
        assert_eq!(h.encounter_distance, 2, "WAIT does not close the distance");

        // COMBAT ends it.
        assert_continue(m.resume(Reply::Selection(0), &mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(h.word(RESULT_CELL), Some(1));
    }

    /// Class 1's ADVANCE arm (`:1408-1423`): the monsters step one band closer
    /// — the same decrement-and-re-dispatch APPROACH performs — and the menu
    /// re-opens, now offering PARLAY because they are adjacent.
    #[test]
    fn advance_walks_the_monsters_in_and_the_menu_reopens_offering_parlay() {
        let b = encounter_menu_block(1, [1, 1, 1, 1, 1], [b"KOBOLDS", b"", b""], 0, 0);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((9, 9));

        let (_, request) = run_to_request(&mut m, &mut h);
        assert_eq!(h.encounter_distance, 1, "max_distance 1 clamps the ray");
        assert_eq!(words(&request), ["COMBAT", "WAIT", "FLEE", "ADVANCE"]);

        assert_eq!(
            m.resume(Reply::Selection(3), &mut h),
            Ok(VmStep::Effect(Effect::EncounterVisual)),
            "the step-in re-dispatches the visual, exactly as APPROACH does"
        );
        assert_eq!(h.encounter_distance, 0);
        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString(b"KOBOLDS".to_vec()),
                clear_first: true,
            }))
        );
        let VmStep::Request(request) = m.step(&mut h).unwrap() else {
            panic!("the menu must re-open")
        };
        assert_eq!(words(&request), ["COMBAT", "WAIT", "FLEE", "PARLAY"]);
    }

    /// The approach line is picked by a CYCLIC scan starting at the current
    /// band (`:1298-1339`): band 2 reads `2,0,1`, so a script that fills only
    /// slot 0 still has something to say at range.
    #[test]
    fn the_approach_line_scans_cyclically_from_the_current_band() {
        // Only slot 0 filled, party at distance 2 -> scan 2,0,1 -> slot 0.
        let b = encounter_menu_block(2, [1, 1, 1, 1, 1], [b"FAR OFF", b"", b""], 0, 0);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((9, 9));
        let (effects, _) = run_to_request(&mut m, &mut h);
        assert_eq!(
            effects[1],
            Effect::Print {
                text: VmString(b"FAR OFF".to_vec()),
                clear_first: true,
            }
        );

        // All three filled: band 2 takes its own line.
        let b = encounter_menu_block(2, [1, 1, 1, 1, 1], [b"NEAR", b"MID", b"FAR"], 0, 0);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((9, 9));
        let (effects, _) = run_to_request(&mut m, &mut h);
        assert_eq!(
            effects[1],
            Effect::Print {
                text: VmString(b"FAR".to_vec()),
                clear_first: true,
            }
        );
    }

    /// An empty approach line suppresses the region clear (`:1341-1344`) — and
    /// outside a dungeon there is no clear to begin with (`:1294`), which is
    /// also what puts PARLAY on the menu at any distance (`:1348`).
    #[test]
    fn outside_a_dungeon_the_menu_offers_parlay_and_never_clears_the_region() {
        let b = encounter_menu_block(2, [1, 1, 1, 1, 1], [b"", b"", b""], 0, 0);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new(); // inDungeon = 0
        h.approach_distance_replies.push_back(2);
        h.calc_group_movement_replies.push_back((9, 9));

        let (effects, request) = run_to_request(&mut m, &mut h);
        assert_eq!(
            effects[1],
            Effect::Print {
                text: VmString(Vec::new()),
                clear_first: false,
            }
        );
        assert_eq!(words(&request), ["COMBAT", "WAIT", "FLEE", "PARLAY"]);
    }

    /// A suspended ENCOUNTER MENU round-trips through a snapshot with its whole
    /// operand set intact — the loop state lives in the `Completion`, so a save
    /// taken on the menu resumes into the same iteration.
    #[test]
    fn a_parked_encounter_menu_survives_a_snapshot_round_trip() {
        let b = encounter_menu_block(0, [0, 3, 0, 0, 3], [b"A VOICE", b"", b""], 0x0C, 0x0C);
        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(IN_DUNGEON_ADDR, 1);
        h.approach_distance_replies.push_back(0);
        h.calc_group_movement_replies.push_back((12, 12));
        run_to_request(&mut m, &mut h);

        let snap = m.snapshot();
        let mut restored = EclMachine::restore(snap, &COTAB).expect("restore");
        assert!(matches!(
            restored.pending(),
            Some(Request::HorizontalMenu { .. })
        ));

        assert_continue(restored.resume(Reply::Selection(3), &mut h));
        assert_eq!(restored.current_pc(), Some(after));
        assert_eq!(h.word(RESULT_CELL), Some(3), "the parlay outcome survived");
    }

    /// PICTURE (0x0E), `CMD_Picture` ovr003.cs:312-358: a real block id
    /// yields `Effect::Picture`, then the instruction completes on the next
    /// `step()`.
    #[test]
    fn picture_with_real_block_id_yields_picture_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0E).imm_byte(0x50); // PICTURE 0x50
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::Picture(0x50))));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// PICTURE (0x0E)'s `blockId == 0xFF` sentinel (`ovr003.cs:343-356`).
    #[test]
    fn picture_with_0xff_yields_clear_picture_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0E).imm_byte(0xFF);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::ClearPicture)));
    }

    /// PRINT (0x11), `CMD_Print` ovr003.cs:389-417, numeric operand path:
    /// stringified exactly like `.ToString()`.
    #[test]
    fn print_numeric_operand_is_stringified() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x11).imm_byte(42); // PRINT 42
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString::from_bytes(*b"42"),
                clear_first: false,
            }))
        );
    }

    /// PRINT (0x11), string operand path.
    #[test]
    fn print_string_operand_uses_the_register() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x11).inline_str(b"HELLO");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString::from_bytes(*b"HELLO"),
                clear_first: false,
            }))
        );
    }

    /// PRINTCLEAR (0x12): same handler as PRINT, `clear_first = true`
    /// (`ovr003.cs:404-414`).
    #[test]
    fn printclear_sets_clear_first() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x12).inline_str(b"HI");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Effect(Effect::Print {
                text: VmString::from_bytes(*b"HI"),
                clear_first: true,
            }))
        );
    }

    /// COMPARE AND (0x14), `CMD_CompareAnd` ovr003.cs:438-461: only ever
    /// sets flags `[0]`/`[1]`, never the relational four.
    #[test]
    fn compare_and_true_case_sets_only_flag_zero() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x14).imm_byte(1).imm_byte(1).imm_byte(2).imm_byte(2); // 1==1 && 2==2
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(m.flags(), [true, false, false, false, false, false]);
    }

    #[test]
    fn compare_and_false_case_sets_only_flag_one() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x14).imm_byte(1).imm_byte(2).imm_byte(2).imm_byte(2); // 1!=2
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(m.flags(), [false, true, false, false, false, false]);
    }

    /// COMPARE AND fed a string-mode operand: the original's unconditional
    /// `GetCmdValue()` throws (docket item 5) — surfaced as a defined
    /// `VmError`, not a panic.
    #[test]
    fn compare_and_string_operand_is_a_defined_error_not_a_panic() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x14)
            .inline_str(b"X")
            .imm_byte(1)
            .imm_byte(2)
            .imm_byte(2);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(
            m.step(&mut h),
            Err(VmError::StringOperandTypeMismatch {
                pc: entry,
                opcode: 0x14
            })
        );
    }

    /// CLEARMONSTERS (0x1C), `CMD_ClearMonsters` ovr003.cs:758-769: no
    /// operands.
    #[test]
    fn clearmonsters_calls_the_service_with_no_operands() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x1C);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::ClearMonsters));
    }

    /// NEWECL (0x20), `CMD_NewECL` ovr003.cs:480-498: reports the chain and
    /// stops — no further resets happen here (those live in `load_block`).
    #[test]
    fn newecl_yields_chain_to_with_the_decoded_block_id() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x20).imm_byte(7); // NEWECL block 7

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::ChainTo(BlockId(7)))));
    }

    /// LOAD FILES (0x21), `CMD_LoadFiles` ovr003.cs:501-604 (`0x21` branch):
    /// `var_3 != 0xFF/0x7F` gates `load_3d_map`, gated further on `inDungeon`
    /// (`0x4BE6`, a documented Area-window cell).
    #[test]
    fn load_files_loads_3d_map_when_in_dungeon() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x21).imm_byte(3).imm_byte(0xFF).imm_byte(0xFF); // var_3=3
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x4BE6, 1); // inDungeon = 1

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::Load3dMap { block_id: 3 }));
    }

    #[test]
    fn load_files_loads_bigpic_when_not_in_dungeon() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x21).imm_byte(0xFF).imm_byte(0xFF).imm_byte(5); // var_1=5
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x4BE6, 0); // inDungeon = 0

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::LoadBigpic { id: 0x79 }));
    }

    /// ★ LOAD FILES' third conjunct (`ovr003.cs:530`), landed in roll-credits
    /// slice 7 (D-S7e): `lastDaxBlockId != 0x50`. While `ECL1#80`'s city scene
    /// owns the viewport, the overland map underneath it is not reloaded.
    #[test]
    fn load_files_does_not_reload_the_bigpic_under_a_city_scene() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x21).imm_byte(0xFF).imm_byte(0xFF).imm_byte(5);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x4BE6, 0); // inDungeon = 0
        h.last_dax_block = 0x50; // the city scene is up

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(!h.calls.contains(&RecordedCall::LoadBigpic { id: 0x79 }));
    }

    /// LOAD PIECES (0x37), `CMD_LoadFiles` ovr003.cs:501-604 (shared with
    /// 0x21; the `0x37` branch): `var_3 == 0x7F` is the fixed-walldef
    /// shortcut, `LoadWalldef(1, 0)`.
    #[test]
    fn load_pieces_var_3_0x7f_loads_a_fixed_walldef() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x37).imm_byte(0x7F).imm_byte(0xFF).imm_byte(0xFF);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h
            .calls
            .contains(&RecordedCall::LoadWalldef { set: 1, id: 0 }));
    }

    /// LOAD PIECES (0x37), the general branch: each of the 3 operands
    /// either loads a walldef (`!= 0xFF`) or resets that wall-set slot
    /// (`== 0xFF`) — the `area_ptr.field_1CE`/`field_1D0` gate has no
    /// `ScriptMemory` address (documented simplification, `machine.rs`'s
    /// doc comment), so this is the only path this session's interpreter
    /// takes when `var_3 != 0x7F`.
    #[test]
    fn load_pieces_general_branch_loads_or_resets_each_slot() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x37).imm_byte(5).imm_byte(0xFF).imm_byte(9); // var_3=5, var_2=0xFF, var_1=9
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h
            .calls
            .contains(&RecordedCall::LoadWalldef { set: 1, id: 5 }));
        assert!(h.calls.contains(&RecordedCall::ResetWallSet { index: 1 }));
        assert!(h
            .calls
            .contains(&RecordedCall::LoadWalldef { set: 3, id: 9 }));
    }

    /// COMBAT (0x24), `CMD_Combat` ovr003.cs:971-1029: the design doc's
    /// coarse request — no operands, suspends, then completes on reply.
    #[test]
    fn combat_suspends_then_resumes_to_continue() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x24); // COMBAT
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Combat)));
        assert_eq!(m.pending(), Some(&Request::Combat));
        assert_continue(m.resume(Reply::Combat, &mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// ON GOTO (0x25), `CMD_OnGotoGoSub` ovr003.cs:1032-1064 (`0x25`
    /// branch): selector and count are both `GetCmdValue`-resolved; an
    /// in-range selector jumps to the matching tail entry.
    #[test]
    fn on_goto_in_range_selector_jumps_to_the_matching_entry() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x25)
            .imm_byte(1) // selector = 1
            .imm_byte(2) // count = 2
            .imm_word_label("entry0")
            .imm_word_label("entry1");
        b.label("entry0");
        b.op(0x00); // would be wrong if selector routing were off
        b.label("entry1");
        b.op(0x00); // correct target for selector 1

        let entry = b.addr_of("entry");
        let entry1 = b.addr_of("entry1");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(entry1));
    }

    /// ON GOTO (0x25): an out-of-range selector is a confirmed fall-through
    /// to `next` — no `else`-branch jump in the original
    /// (`ovr003.cs:1038-1059`).
    #[test]
    fn on_goto_out_of_range_selector_falls_through() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x25)
            .imm_byte(5) // selector = 5, out of range
            .imm_byte(2) // count = 2
            .imm_word_label("entry0")
            .imm_word_label("entry1");
        b.label("after");
        b.op(0x00);
        b.label("entry0");
        b.op(0x00);
        b.label("entry1");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// ON GOSUB (0x26): same shape as ON GOTO, plus a call-stack push of the
    /// fall-through address on the in-range branch — a later RETURN lands
    /// back after the whole decoded tail, not falling straight through to
    /// EXIT (which is what an *empty* call stack's RETURN would do, 0x13's
    /// own note) — the distinguishing signal this test checks for.
    #[test]
    fn on_gosub_in_range_selector_jumps_and_pushes_the_fallthrough() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x26)
            .imm_byte(1) // selector = 1
            .imm_byte(2) // count = 2
            .imm_word_label("entry0")
            .imm_word_label("entry1");
        b.label("fallthrough");
        b.op(0x00); // landing site for the pushed return address
        b.label("entry0");
        b.op(0x00);
        b.label("entry1");
        b.op(0x13); // RETURN — pops the pushed fall-through address

        let entry = b.addr_of("entry");
        let entry1 = b.addr_of("entry1");
        let fallthrough = b.addr_of("fallthrough");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(entry1));
        // If the push happened, RETURN pops it and lands back at
        // `fallthrough`, still Continue — not Done(Ended), which is what an
        // empty call stack's RETURN would produce instead.
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(fallthrough));
    }

    /// ON GOSUB (0x26): an out-of-range selector neither jumps nor pushes —
    /// indistinguishable from ON GOTO's own out-of-range fall-through
    /// (opcode-classification.md's 0x26 row).
    #[test]
    fn on_gosub_out_of_range_selector_falls_through_without_pushing() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x26)
            .imm_byte(5) // selector = 5, out of range
            .imm_byte(2) // count = 2
            .imm_word_label("entry0")
            .imm_word_label("entry1");
        b.label("after");
        b.op(0x13); // RETURN — empty call stack silently becomes EXIT (0x13's own note)
        b.label("entry0");
        b.op(0x00);
        b.label("entry1");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        // RETURN with nothing pushed falls through to EXIT, not a jump back
        // into the tail — proving the out-of-range branch never pushed.
        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
    }

    /// GETTABLE (0x2A), `CMD_GetTable` ovr003.cs:635-648: operand 1 is a raw
    /// base address, added to operand 2's *resolved* index — a computed
    /// address (docket item 12).
    #[test]
    fn gettable_reads_from_a_computed_base_plus_index_address() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2A).imm_word(0x4B00).imm_byte(3).imm_word(0x4C00); // base=0x4B00, idx=3, dest=0x4C00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x4B03, 77);

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4C00), Some(77));
    }

    /// HORIZONTAL MENU (0x2B), `CMD_HorizontalMenu` ovr003.cs:698-753:
    /// variable tail, suspends with the decoded options, writes the reply
    /// selection to the destination on resume.
    #[test]
    fn horizontal_menu_suspends_then_writes_selection_on_resume() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2B)
            .imm_word(0x4B00) // dest
            .imm_byte(2) // string_count
            .inline_str(b"YES")
            .inline_str(b"NO");
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let expected_options = vec![VmString::from_bytes(*b"YES"), VmString::from_bytes(*b"NO")];
        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::HorizontalMenu {
                options: expected_options.clone()
            }))
        );
        assert_eq!(
            m.pending(),
            Some(&Request::HorizontalMenu {
                options: expected_options
            })
        );
        assert_continue(m.resume(Reply::Selection(1), &mut h));
        assert_eq!(h.word(0x4B00), Some(1));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// VERTICAL MENU (0x15), `CMD_VertMenu` ovr003.cs:663-694: three fixed
    /// operands (dest, PROMPT string, entry count), then the count reloaded
    /// tail strings. Suspends with the prompt and the entries and writes
    /// `VertMenuSelect`'s 0-based index to the destination cell on resume.
    #[test]
    fn vertical_menu_suspends_with_prompt_and_entries_then_writes_the_index() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x15)
            .imm_word(0x4B00) // mem_loc
            .inline_str(b"PICK ONE") // gbl.unk_1D972[1], press_any_key's text
            .imm_byte(3) // menuCount
            .inline_str(b"ALE")
            .inline_str(b"MEAD")
            .inline_str(b"WATER");
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let request = Request::VerticalMenu {
            prompt: VmString::from_bytes(*b"PICK ONE"),
            options: vec![
                VmString::from_bytes(*b"ALE"),
                VmString::from_bytes(*b"MEAD"),
                VmString::from_bytes(*b"WATER"),
            ],
        };
        assert_eq!(m.step(&mut h), Ok(VmStep::Request(request.clone())));
        assert_eq!(m.pending(), Some(&request));
        assert_continue(m.resume(Reply::Selection(2), &mut h));
        assert_eq!(h.word(0x4B00), Some(2), "the index is written 0-based");
        assert_eq!(m.current_pc(), Some(after));
    }

    /// The reload trick's own edge: a zero-entry VERTICAL MENU decodes its
    /// three fixed operands, suspends with no entries, and still lands on the
    /// instruction after them.
    #[test]
    fn vertical_menu_with_no_entries_still_suspends_and_advances() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x15)
            .imm_word(0x4B00)
            .inline_str(b"NOTHING")
            .imm_byte(0);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::VerticalMenu {
                prompt: VmString::from_bytes(*b"NOTHING"),
                options: vec![],
            }))
        );
        assert_continue(m.resume(Reply::Selection(0), &mut h));
        assert_eq!(h.word(0x4B00), Some(0));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// A `Reply` of the wrong kind is refused for VERTICAL MENU exactly as it
    /// is for every other request (`resume`'s kind check).
    #[test]
    fn vertical_menu_refuses_a_mismatched_reply() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x15)
            .imm_word(0x4B00)
            .inline_str(b"P")
            .imm_byte(1)
            .inline_str(b"ALE");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        assert!(matches!(m.step(&mut h), Ok(VmStep::Request(_))));
        assert_eq!(m.resume(Reply::Delay, &mut h), Err(VmError::ReplyMismatch));
    }

    /// CALL (0x2D) case `0xAE11`, `CMD_Call` ovr003.cs:1843-1866.
    #[test]
    fn call_0xae11_queries_wall_roof_and_wall_type() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(0xAE11)); // CALL key 0xAE11
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::WallRoof));
        assert!(h.calls.contains(&RecordedCall::WallType));
        // The gate was consulted and, unarmed, drew nothing.
        assert!(h
            .calls
            .contains(&RecordedCall::RedrawViewGate { armed: false }));
    }

    /// `0xAE11`'s consolidated redraw gate (`ovr003.cs:1848-1860`): with the
    /// dirty flags armed, the CALL yields `Effect::RedrawView` — the draw
    /// rides the effect queue so it presents before any text the script
    /// prints next (the amnesia intro's page-1 view: `vm_init_ecl` arms
    /// `byte_1EE91`, `ovr008.cs:94`). The check-and-clear is the host's, so
    /// a second `CALL 0xAE11` with nothing re-arming the flags draws nothing.
    #[test]
    fn call_0xae11_armed_gate_yields_one_redraw_view_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(0xAE11)); // armed → redraw
        b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(0xAE11)); // cleared → silent
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.redraw_flags_armed = true;

        let step = m.step(&mut h);
        assert!(
            matches!(step, Ok(VmStep::Effect(Effect::RedrawView))),
            "the armed gate's draw travels the effect queue: {step:?}"
        );
        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        let gates: Vec<bool> = h
            .calls
            .iter()
            .filter_map(|c| match c {
                RecordedCall::RedrawViewGate { armed } => Some(*armed),
                _ => None,
            })
            .collect();
        assert_eq!(gates, vec![true, false], "check-and-clear is one-shot");
    }

    /// CALL (0x2D) cases `1`/`2`: `SetupDuel(bool)`.
    #[test]
    fn call_case_1_and_2_setup_duel() {
        for (key, expect) in [(1u16, true), (2u16, false)] {
            let mut b = EclBuilder::new();
            b.label("entry");
            b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(key));
            b.op(0x00);

            let entry = b.addr_of("entry");
            let mut m = machine_from(&b, entry);
            let mut h = TestHost::new();

            assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
            assert!(h
                .calls
                .contains(&RecordedCall::SetupDuel { is_duel: expect }));
        }
    }

    /// CALL (0x2D) case `0x3201`: sound selection is a service call, but
    /// playback is a buffered `Effect::Sound`.
    #[test]
    fn call_case_0x3201_plays_the_selected_sound_as_an_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFF + 0x3201);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.call_sound_variant_replies.push_back(9);

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::Sound(9))));
        assert!(h.calls.contains(&RecordedCall::CallSoundVariant));
    }

    /// CALL (0x2D) case `0x401F`: `MovePositionForward`.
    #[test]
    fn call_case_0x401f_moves_position_forward() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFF + 0x401F);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::MovePositionForward));
    }

    /// CALL (0x2D) case `0x4019`: `wall_type` only queried when not in a
    /// dungeon (`gbl.area_ptr.inDungeon == 0`).
    #[test]
    fn call_case_0x4019_queries_wall_type_only_outside_dungeon() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFF + 0x4019);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x4BE6, 1); // inDungeon = 1: gate should suppress the query

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(!h.calls.contains(&RecordedCall::WallType));
    }

    /// CALL (0x2D) case `0xE804`: draws one animation frame (`Effect`) then
    /// requests the trailing pause — the effects-before-request ordering
    /// test lives in its own module below; this just checks the case wiring.
    #[test]
    fn call_case_0xe804_draws_a_frame_then_delays() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(0xE804));
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::AnimationFrame)));
        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay)));
    }

    /// CALL (0x2D): an unrecognized key is a silent no-op (no `default` arm
    /// in the original's switch).
    #[test]
    fn call_unrecognized_key_is_a_silent_noop() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x0001); // resolves to a key with no case
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// AND (0x2F), `CMD_AndOr` ovr003.cs:607-632 (`0x2F` branch): flags
    /// derive from `compare_variables(resultant, 0)`, which unwinds to
    /// `set_compare_flags(0, resultant)` — the relational flags test the
    /// result against zero.
    #[test]
    fn and_writes_bitwise_and_and_sets_flags_against_zero() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2F)
            .imm_byte(0b1100)
            .imm_byte(0b1010)
            .imm_word(0x4B00);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(0b1000));
        // set_compare_flags(0, 8): 0<8 is true ("<"), 0!=8 true, etc.
        assert_eq!(m.flags(), [false, true, true, false, true, false]);
    }

    /// OR (0x30), `CMD_AndOr` ovr003.cs:607-632 (`0x30` branch): identical
    /// structure to AND, bitwise OR instead of AND.
    #[test]
    fn or_writes_bitwise_or_and_sets_flags_against_zero() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x30)
            .imm_byte(0b1100)
            .imm_byte(0b1010)
            .imm_word(0x4B00);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(0b1110));
        // set_compare_flags(0, 14): 0<14 is true ("<"), 0!=14 true, etc.
        assert_eq!(m.flags(), [false, true, true, false, true, false]);
    }

    /// PRINT RETURN (0x33), `CMD_PrintReturn` ovr003.cs:1730-1738: cursor
    /// bookkeeping only, a payload-less effect.
    #[test]
    fn print_return_yields_a_payloadless_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x33);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::PrintReturn)));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// DELAY (0x3A), `CMD_Delay` ovr003.cs:1588-1592: no operands, suspends
    /// on a bare `Request::Delay`.
    #[test]
    fn delay_suspends_then_resumes_to_continue() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3A);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay)));
        assert_continue(m.resume(Reply::Delay, &mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    // --- roll-credits slice 3: the items/roster/mechanics tail ------------

    /// ★ PARLAY's five tones are a fixed code-segment menu, and the operands
    /// are the OUTCOME TABLE the reply indexes — the selection itself is
    /// never written (`ovr003:2837-284D`).
    #[test]
    fn parlay_writes_the_table_entry_under_the_chosen_tone() {
        // `ECL3#16 @0x8B15`'s shape: five outcome bytes and a destination.
        for (choice, expected) in [(0u8, 1u16), (2, 3), (4, 9)] {
            let mut b = EclBuilder::new();
            b.label("entry");
            b.op(0x2C)
                .imm_byte(1)
                .imm_byte(2)
                .imm_byte(3)
                .imm_byte(4)
                .imm_byte(9)
                .mem(0x7F80);
            b.label("after");
            b.op(0x00);

            let entry = b.addr_of("entry");
            let after = b.addr_of("after");
            let mut m = machine_from(&b, entry);
            let mut h = TestHost::new();

            let step = m.step(&mut h).expect("step");
            let VmStep::Request(Request::HorizontalMenu { options }) = step else {
                panic!("expected the tone menu, got {step:?}");
            };
            let words: Vec<String> = options
                .iter()
                .map(|o| String::from_utf8_lossy(&o.0).into_owned())
                .collect();
            assert_eq!(words, ["HAUGHTY", "SLY", "NICE", "MEEK", "ABUSIVE"]);

            assert_continue(m.resume(Reply::Selection(choice), &mut h));
            assert_eq!(h.word(0x7F80), Some(expected));
            assert_eq!(m.current_pc(), Some(after));
        }
    }

    /// PARLAY never touches the PRNG — the draw-neutrality claim, asserted
    /// rather than argued.
    #[test]
    fn parlay_draws_nothing() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2C)
            .imm_byte(0)
            .imm_byte(0)
            .imm_byte(0)
            .imm_byte(0)
            .imm_byte(0)
            .mem(0x7F80);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::HorizontalMenu { .. }))
        ));
        assert_continue(m.resume(Reply::Selection(1), &mut h));
        assert!(
            !h.calls
                .iter()
                .any(|c| matches!(c, RecordedCall::Roll { .. } | RecordedCall::RollDice { .. })),
            "PARLAY is draw-free start to finish"
        );
    }

    /// A parked PARLAY survives a snapshot round-trip with its outcome table
    /// intact — the table lives in the completion, not in the block bytes the
    /// operand cursor has already passed.
    #[test]
    fn a_parked_parlay_survives_a_snapshot_round_trip() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2C)
            .imm_byte(7)
            .imm_byte(8)
            .imm_byte(9)
            .imm_byte(10)
            .imm_byte(11)
            .mem(0x7F84);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::HorizontalMenu { .. }))
        ));

        let snap = m.snapshot();
        let mut restored = EclMachine::restore(snap, &COTAB).expect("restore");
        assert!(matches!(
            restored.pending(),
            Some(Request::HorizontalMenu { .. })
        ));

        assert_continue(restored.resume(Reply::Selection(3), &mut h));
        assert_eq!(h.word(0x7F84), Some(10));
    }

    /// LOAD CHARACTER passes the operand byte through **raw**, high bit and
    /// all: the mask is the service's business (`ovr003:0318-0325`).
    #[test]
    fn load_character_hands_the_service_the_raw_operand_byte() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0A).imm_byte(0x83);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert!(h
            .calls
            .contains(&RecordedCall::RetargetSelectedPlayer { index: 0x83 }));
    }

    /// FIND ITEM's flag convention (`ovr003.cs:1566-1581`): a hit is `=`, a
    /// miss is `<>`, and **no other relation is left true** — the original
    /// clears all six and then arms exactly one.
    #[test]
    fn find_item_sets_only_the_equal_flag_on_a_hit() {
        for (found, expect_hit) in [(true, true), (false, false)] {
            let mut b = EclBuilder::new();
            b.label("entry");
            b.op(0x32).imm_byte(0x5E);
            b.op(0x16); // IF =
            b.op(0x01).imm_word_label("hit"); // GOTO hit
            b.label("miss");
            b.op(0x00);
            b.label("hit");
            b.op(0x00);

            let entry = b.addr_of("entry");
            let hit = b.addr_of("hit");
            let miss = b.addr_of("miss");
            let mut m = machine_from(&b, entry);
            let mut h = TestHost::new();
            h.party_has_item_replies.push_back(found);

            assert_continue(m.step(&mut h)); // FIND ITEM
            assert!(h
                .calls
                .contains(&RecordedCall::PartyHasItem { item_type: 0x5E }));
            assert_continue(m.step(&mut h)); // IF =
            if expect_hit {
                assert_continue(m.step(&mut h)); // the guarded GOTO runs
                assert_eq!(m.current_pc(), Some(hit));
            } else {
                // The IF skipped the GOTO outright: the false arm is the very
                // next instruction, which is the shape the census gap hid.
                assert_eq!(m.current_pc(), Some(miss));
            }
        }
    }

    /// FIND SPECIAL is the same shape over `HasAffect` (`ovr003.cs:2021-2039`).
    #[test]
    fn find_special_sets_the_same_two_flags() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3F).imm_byte(0x27);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.find_special_replies.push_back(true);

        assert_continue(m.step(&mut h));
        assert!(h
            .calls
            .contains(&RecordedCall::FindSpecial { affect_type: 0x27 }));
    }

    /// DESTROY ITEMS is one service call and no flags (`ovr003.cs:2042-2055`).
    #[test]
    fn destroy_items_calls_the_service_and_advances() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x40).imm_byte(0x60);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert!(h
            .calls
            .contains(&RecordedCall::DestroyItems { item_type: 0x60 }));
    }

    /// ★ ROB (0x28), `CMD_Rob` (`ovr003.cs:1202-1225`) with `allParty == 0`:
    /// the two halves fire once, against `gbl.SelectedPlayer`, and the scale
    /// is the fraction KEPT.
    #[test]
    fn rob_with_all_party_clear_hits_only_the_selected_player() {
        let mut b = EclBuilder::new();
        b.label("entry");
        // `ECL3#16 @0x93AC` verbatim: ROB 0x00, 0x14, 0x00 — take 20%.
        b.op(0x28).imm_byte(0x00).imm_byte(0x14).imm_byte(0x00);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.selected_player = PlayerId(2);
        h.team_size_replies.push_back(6);

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));

        let rob_calls: Vec<_> = h
            .calls
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    RecordedCall::RobMoney { .. } | RecordedCall::RobItems { .. }
                )
            })
            .cloned()
            .collect();
        assert_eq!(
            rob_calls,
            vec![
                RecordedCall::RobMoney {
                    player: PlayerId(2),
                    scale: 0.8,
                },
                RecordedCall::RobItems {
                    player: PlayerId(2),
                    chance: 0,
                },
            ]
        );
        // `team_size` is never consulted on this arm.
        assert!(!h.calls.contains(&RecordedCall::TeamSize));
    }

    /// ★ The all-party arm walks `TeamList` and **interleaves** the two
    /// halves per member (`ovr003.cs:1216-1222`) — which is the draw order,
    /// since `rob_items` rolls once per item.
    #[test]
    fn rob_with_all_party_set_interleaves_money_and_items_per_member() {
        let mut b = EclBuilder::new();
        b.label("entry");
        // `ECL5#51 @0x8B55` verbatim: ROB 0x01, 0x28, 0x7D.
        b.op(0x28).imm_byte(0x01).imm_byte(0x28).imm_byte(0x7D);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.team_size_replies.push_back(3);

        assert_continue(m.step(&mut h));

        let rob_calls: Vec<_> = h
            .calls
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    RecordedCall::RobMoney { .. } | RecordedCall::RobItems { .. }
                )
            })
            .cloned()
            .collect();
        assert_eq!(
            rob_calls,
            vec![
                RecordedCall::RobMoney {
                    player: PlayerId(0),
                    scale: 0.6,
                },
                RecordedCall::RobItems {
                    player: PlayerId(0),
                    chance: 0x7D,
                },
                RecordedCall::RobMoney {
                    player: PlayerId(1),
                    scale: 0.6,
                },
                RecordedCall::RobItems {
                    player: PlayerId(1),
                    chance: 0x7D,
                },
                RecordedCall::RobMoney {
                    player: PlayerId(2),
                    scale: 0.6,
                },
                RecordedCall::RobItems {
                    player: PlayerId(2),
                    chance: 0x7D,
                },
            ]
        );
    }

    /// ROB is three operand batches wide (`vm_LoadCmdSets(3)`), so an `IF`
    /// skipping over one lands on the instruction after it.
    #[test]
    fn rob_consumes_three_operand_batches() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x28).mem(0x7F79).imm_byte(0x4B).imm_byte(0x00);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.set_word(0x7F79, 0);
        h.team_size_replies.push_back(1);

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// ★ SPELL (0x3B), `CMD_Spell` (`ovr003:2E33-2F22`): three operands, and
    /// the two results are written **spell index first, player index second**
    /// (`ovr003:2F03-2F1A`).
    #[test]
    fn spell_writes_the_slot_then_the_player_to_its_two_destinations() {
        let mut b = EclBuilder::new();
        b.label("entry");
        // `ECL4#34 @0x879F` verbatim: SPELL 0x16, [0x7F79], [0x7F7A].
        b.op(0x3B).imm_byte(0x16).mem(0x7F79).mem(0x7F7A);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.find_spell_in_party_replies.push_back((7, 2));

        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
        assert!(h
            .calls
            .contains(&RecordedCall::FindSpellInParty { spell_id: 0x16 }));
        assert_eq!(h.word(0x7F79), Some(7), "slot into the FIRST cell");
        assert_eq!(h.word(0x7F7A), Some(2), "player into the SECOND");
    }

    /// The not-found answer reaches the cells unchanged — `0xFF` is what the
    /// shipped `COMPARE <cell>, 0xFF` right after each site tests.
    #[test]
    fn spell_passes_the_not_found_sentinel_straight_through() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3B).imm_byte(0x16).mem(0x7F79).mem(0x7F7A);
        b.op(0x03).mem(0x7F79).imm_byte(0xFF); // COMPARE [0x7F79], 0xFF
        b.op(0x17); // IF <>
        b.op(0x00); // (skipped when equal)
        b.label("not_taken");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let not_taken = b.addr_of("not_taken");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        // ★ (0xFF, Count - 1), not (0xFF, 0xFF).
        h.find_spell_in_party_replies.push_back((0xFF, 5));

        assert_continue(m.step(&mut h)); // SPELL
        assert_continue(m.step(&mut h)); // COMPARE
        assert_continue(m.step(&mut h)); // IF <> -> false, skip the EXIT
        assert_eq!(m.current_pc(), Some(not_taken));
        assert_eq!(
            h.word(0x7F7A),
            Some(5),
            "the finder's index is still written"
        );
    }

    /// ★ INPUT STRING (0x10), `CMD_InputString` (`ovr003.cs:372-388`): two
    /// operand batches, the destination is the SECOND, and the typed line
    /// lands there.
    #[test]
    fn input_string_writes_the_typed_line_to_the_second_operand() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x10).imm_byte(0x0C).mem_str(0x7B90);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let step = m.step(&mut h).expect("INPUT STRING decodes");
        let VmStep::Request(Request::InputString { max_len }) = step else {
            panic!("expected the editor, got {step:?}");
        };
        // ★ 40, not the operand's 12 — the handler's own hardcoded `0x28`.
        assert_eq!(max_len, 0x28);

        m.resume(Reply::Text(VmString::from_bytes(&b"PASSWORD"[..])), &mut h)
            .expect("the editor resumes");
        assert_eq!(m.current_pc(), Some(after));
        assert_eq!(
            h.string(0x7B90),
            Some(&VmString::from_bytes(&b"PASSWORD"[..]))
        );
    }

    /// ★ An empty line becomes a single space (`ovr003.cs:379-382`, the
    /// `asc_269A2` literal at `ovr003:09A2`) — so a destination cell never
    /// holds the empty string, and the shipped `COMPARE [cell], "<word>"`
    /// gates always compare against something.
    #[test]
    fn input_string_substitutes_a_space_for_an_empty_line() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x10).imm_byte(0x08).mem_str(0x7F79);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::InputString { .. }))
        ));
        m.resume(Reply::Text(VmString::default()), &mut h)
            .expect("an empty line is a legal answer");
        assert_eq!(h.string(0x7F79), Some(&VmString::from_bytes(&b" "[..])));
    }

    /// INPUT STRING consumes two operand batches, so an `IF` skipping over
    /// one lands on the instruction after it.
    #[test]
    fn input_string_consumes_two_operand_batches() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x10).imm_byte(0x2D).mem_str(0x7B00);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::InputString { .. }))
        ));
        m.resume(Reply::Text(VmString::from_bytes(&b"X"[..])), &mut h)
            .unwrap();
        assert_eq!(m.current_pc(), Some(after));
    }

    /// ★ WHO (0x39), `CMD_Who` (`ovr003.cs:1757-1765`): one operand batch,
    /// the picker's prompt taken from string register 1, and a reply that
    /// carries nothing (`selectAPlayer` mutates `gbl.SelectedPlayer`, not
    /// memory).
    #[test]
    fn who_opens_a_player_picker_with_the_inline_prompt() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x39).inline_str(b"WHO PICKS?");
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let step = m.step(&mut h).expect("WHO decodes");
        let VmStep::Request(Request::SelectPlayer { prompt }) = step else {
            panic!("expected the picker, got {step:?}");
        };
        assert_eq!(prompt, VmString::from_bytes(&b"WHO PICKS?"[..]));

        m.resume(Reply::PlayerSelected, &mut h)
            .expect("the picker resumes");
        assert_eq!(m.current_pc(), Some(after));
    }

    /// The reply kind is checked: a menu selection is not an answer to a
    /// player picker.
    #[test]
    fn who_rejects_a_mismatched_reply() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x39).inline_str(b"WHO?");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::SelectPlayer { .. }))
        ));
        assert!(matches!(
            m.resume(Reply::Selection(0), &mut h),
            Err(VmError::ReplyMismatch)
        ));
    }

    /// ★ `CMD_Who` reads `gbl.unk_1D972[1]` with no `Code < 0x80` guard
    /// (`ovr003.cs:1759`), so a numeric operand leaves the register holding
    /// whatever the last string-mode operand put there. Transcribed, not
    /// corrected — no shipped site does it.
    #[test]
    fn who_with_a_numeric_operand_presents_the_stale_string_register() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x11).inline_str(b"EARLIER"); // PRINT fills register 1
        b.op(0x39).imm_byte(0x07); // WHO, numeric operand
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let prompt = loop {
            match m.step(&mut h).expect("the fixture runs") {
                VmStep::Request(Request::SelectPlayer { prompt }) => break prompt,
                VmStep::Continue | VmStep::Effect(Effect::Print { .. }) => continue,
                other => panic!("expected the picker, got {other:?}"),
            }
        };
        assert_eq!(prompt, VmString::from_bytes(&b"EARLIER"[..]));
    }

    /// ★ SAVE TABLE's operand roles are NOT GETTABLE's mirror image
    /// (`ovr003.cs:651-660` against `:635-647`): the value comes first, the
    /// table base second (raw, never dereferenced) and the index third.
    #[test]
    fn save_table_writes_value_to_base_plus_index() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x35).imm_byte(0x2A).mem(0x7A00).imm_byte(5);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_eq!(h.word(0x7A05), Some(0x2A));
    }

    /// SAVE TABLE and GETTABLE round-trip through the same cell — the pairing
    /// the shipped quest-flag tables are built on.
    #[test]
    fn save_table_then_gettable_round_trips_one_cell() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x35).imm_byte(0x07).mem(0x7A80).imm_byte(3); // SAVE TABLE 7 -> 0x7A83
        b.op(0x2A).mem(0x7A80).imm_byte(3).mem(0x7F00); // GETTABLE 0x7A83 -> 0x7F00
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_continue(m.step(&mut h));
        assert_eq!(h.word(0x7F00), Some(7));
    }

    /// CLEAR BOX: no operands, one effect, one byte of pc (`ovr003.cs:1743`).
    #[test]
    fn clear_box_yields_one_effect_and_advances_one_byte() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3D);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::ClearBox)));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// DUMP: the service, then the summary repaint (`ovr003.cs:2007-2018`).
    #[test]
    fn dump_frees_the_selected_member_then_repaints() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3E);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::PartySummary)));
        assert!(h.calls.contains(&RecordedCall::DumpSelectedPlayer));
    }

    /// ★ ADD NPC decodes **two** operands even though its `skip_size` is 1 —
    /// the fixed-arity divergence the dialect records. Both shipped pairs use
    /// morale 0x64.
    #[test]
    fn add_npc_decodes_two_operands_and_repaints() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x36).imm_byte(0x16).imm_byte(0x64);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::PartySummary)));
        assert!(h.calls.contains(&RecordedCall::AddNpc {
            monster_id: 0x16,
            morale: 0x64
        }));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(after), "both operands were consumed");
    }

    /// A missing `MON{area}CHA` block is `load_mob`'s hard stop
    /// (`ovr017.cs:836`), surfaced like LOAD MONSTER's.
    #[test]
    fn add_npc_missing_asset_halts_the_machine() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x36).imm_byte(0x16).imm_byte(0x64);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.add_npc_replies.push_back(Err(MissingData));

        assert_eq!(
            m.step(&mut h),
            Err(VmError::MissingAsset {
                pc: entry,
                opcode: 0x36
            })
        );
    }

    /// PROGRAM 3 sets the party-killed flag and ends the activation
    /// (`ovr003.cs:1982-1986`); every other case returns to the next
    /// instruction.
    #[test]
    fn program_exits_or_continues_per_the_engines_verdict() {
        for (outcome, expect_exit) in [
            (crate::ProgramOutcome::Exit, true),
            (crate::ProgramOutcome::Continue, false),
        ] {
            let mut b = EclBuilder::new();
            b.label("entry");
            b.op(0x38).imm_byte(3);
            b.label("after");
            b.op(0x00);

            let entry = b.addr_of("entry");
            let after = b.addr_of("after");
            let mut m = machine_from(&b, entry);
            let mut h = TestHost::new();
            h.program_replies.push_back(outcome);

            let step = m.step(&mut h);
            assert!(h.calls.contains(&RecordedCall::Program { code: 3 }));
            if expect_exit {
                assert_eq!(step, Ok(VmStep::Done(Exit::Ended)));
            } else {
                assert_eq!(step, Ok(VmStep::Continue));
                assert_eq!(m.current_pc(), Some(after));
            }
        }
    }

    /// ★ DAMAGE's draw ORDER, which is the whole of its fidelity: the damage
    /// roll first, then the victim roll (taken whenever `var_1 & 0x40` is
    /// clear, **before** the mode test), then the arm's own checks.
    #[test]
    fn damage_rolls_damage_then_victim_then_saves() {
        let mut b = EclBuilder::new();
        b.label("entry");
        // var_1 = 0x80 (save mode, single random victim), 2d6+1, save type 3.
        b.op(0x2E)
            .imm_byte(0x80)
            .imm_byte(2)
            .imm_byte(6)
            .imm_byte(1)
            .imm_byte(3);
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.party_size_replies.push_back(6);
        h.roll_dice_replies.push_back(9); // the damage roll
        h.roll_dice_replies.push_back(4); // the victim roll -> index 3
        h.roll_saving_throw_replies.push_back(false); // failed save -> damage

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::PressAnyKey { .. }))
        ));

        let rolls: Vec<_> = h
            .calls
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    RecordedCall::RollDice { .. }
                        | RecordedCall::RollSavingThrow { .. }
                        | RecordedCall::ApplyDamage { .. }
                )
            })
            .cloned()
            .collect();
        assert_eq!(
            rolls,
            vec![
                RecordedCall::RollDice { size: 6, count: 2 },
                RecordedCall::RollDice { size: 6, count: 1 },
                RecordedCall::RollSavingThrow {
                    player: crate::PlayerId(3),
                    bonus: 0,
                    save_type: 3
                },
                RecordedCall::ApplyDamage {
                    player: crate::PlayerId(3),
                    damage: 10
                },
            ]
        );

        assert_continue(m.resume(Reply::PressAnyKey, &mut h));
        assert_eq!(m.current_pc(), Some(after));
    }

    /// The `& 0x80`-clear arm: `var_1` separate hits, each re-rolling its own
    /// victim, each gated by `CanHitTarget` — and the damage used by hit *n*
    /// is the value rolled at the end of hit *n-1* (`ovr003:2BDE`), so the
    /// last roll is drawn and thrown away.
    #[test]
    fn damage_hit_count_arm_rerolls_victim_and_trails_its_damage_roll() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2E)
            .imm_byte(2) // two hits, no save mode
            .imm_byte(1)
            .imm_byte(4)
            .imm_byte(0)
            .imm_byte(11); // to-hit bonus
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.party_size_replies.extend([6, 6, 6]);
        h.roll_dice_replies.extend([3, 2, 5, 1, 7]);
        h.can_hit_target_replies.extend([true, true]);

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::PressAnyKey { .. }))
        ));

        let damages: Vec<_> = h
            .calls
            .iter()
            .filter_map(|c| match c {
                RecordedCall::ApplyDamage { player, damage } => Some((player.0, *damage)),
                _ => None,
            })
            .collect();
        // Draw order: damage=3, victim=2 (pre-loop, discarded by the loop's
        // own re-roll), then hit 1: victim=5 -> index 4 with damage 3, the
        // trailing roll 1; hit 2: victim=7 -> index 6 with damage 1, the
        // trailing roll 7 discarded.
        assert_eq!(damages, vec![(4, 3), (6, 1)]);
        let dice = h
            .calls
            .iter()
            .filter(|c| matches!(c, RecordedCall::RollDice { .. }))
            .count();
        assert_eq!(dice, 6, "1 damage + 1 pre-loop victim + 2x(victim+damage)");
    }

    /// The whole-party arm walks the roster and offers each member a save.
    #[test]
    fn damage_whole_party_arm_walks_every_member() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2E)
            .imm_byte(0xC0) // 0x80 save mode | 0x40 whole party
            .imm_byte(1)
            .imm_byte(8)
            .imm_byte(0)
            .imm_byte(2);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.team_size_replies.push_back(3);
        h.roll_dice_replies.push_back(6);
        h.roll_saving_throw_replies.extend([false, true, false]);

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::PressAnyKey { .. }))
        ));

        let hit: Vec<_> = h
            .calls
            .iter()
            .filter_map(|c| match c {
                RecordedCall::ApplyDamage { player, .. } => Some(player.0),
                _ => None,
            })
            .collect();
        assert_eq!(hit, vec![0, 2], "the member who saved took nothing");
        assert!(
            !h.calls.iter().any(|c| matches!(c, RecordedCall::PartySize)),
            "the whole-party arm never rolls a victim, so it never reads party_size"
        );
    }

    /// `0x10` means "damage anyway on a successful save" — not half damage.
    #[test]
    fn damage_bit_0x10_damages_through_a_successful_save() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2E)
            .imm_byte(0xD0) // save mode | whole party | damage-through-save
            .imm_byte(1)
            .imm_byte(8)
            .imm_byte(0)
            .imm_byte(2);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.team_size_replies.push_back(2);
        h.roll_dice_replies.push_back(6);
        h.roll_saving_throw_replies.extend([true, true]);

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::PressAnyKey { .. }))
        ));

        let hit = h
            .calls
            .iter()
            .filter(|c| matches!(c, RecordedCall::ApplyDamage { .. }))
            .count();
        assert_eq!(hit, 2, "both saved, both still took the damage");
    }

    /// DAMAGE brackets its body with a save/restore of `SelectedPlayer`
    /// (`ovr003:295E`, `:2C8B`) and ends with the wipe scan.
    #[test]
    fn damage_restores_the_selection_and_runs_the_wipe_scan() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2E)
            .imm_byte(0xC0)
            .imm_byte(1)
            .imm_byte(4)
            .imm_byte(0)
            .imm_byte(0);
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.selected_player = crate::PlayerId(4);
        h.team_size_replies.push_back(1);
        h.roll_dice_replies.push_back(2);

        assert!(matches!(
            m.step(&mut h),
            Ok(VmStep::Request(Request::PressAnyKey { .. }))
        ));
        assert!(h.calls.contains(&RecordedCall::PartyWipeCheck));
        assert!(h.calls.contains(&RecordedCall::SetSelectedPlayer {
            player: crate::PlayerId(4)
        }));
        let wipe = h
            .calls
            .iter()
            .position(|c| matches!(c, RecordedCall::PartyWipeCheck))
            .unwrap();
        let restore = h
            .calls
            .iter()
            .position(|c| matches!(c, RecordedCall::SetSelectedPlayer { .. }))
            .unwrap();
        assert!(wipe < restore, "the scan runs before the restore (`:2C04`)");
    }
}

/// Skip-semantics tests (§4): IF-false over every opcode class the design
/// doc calls out — size-0 one-byte advance, the `0x34`/`0x36` fixed-arity
/// mismatches, and skip's side effects (string-register fills, `0x81`
/// memory reads).
mod skip_semantics {
    use super::*;

    /// A false IF over a size-0 opcode (EXIT) advances exactly one byte
    /// (`CmdItem.Skip`, `ovr003.cs:2431-2434`) — verified by landing exactly
    /// on a probing SAVE right after the size-0 opcode's single byte.
    #[test]
    fn if_false_over_size_zero_opcode_advances_one_byte() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(2); // COMPARE 1,2 -> "=" false
        b.op(0x16); // IF = : false -> skip next
        b.op(0x00); // EXIT, skip_size 0 (the maybe-skipped instruction)
        b.op(0x09).imm_byte(0xAA).imm_word(0x4B00); // SAVE probe
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(0xAA));
    }

    /// IF true never skips: the maybe-skipped instruction executes
    /// normally.
    #[test]
    fn if_true_does_not_skip() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(1); // COMPARE 1,1 -> "=" true
        b.op(0x16); // IF = : true -> no skip
        b.op(0x09).imm_byte(0xAA).imm_word(0x4B00); // SAVE, executes normally
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(0xAA));
    }

    /// The confirmed fixed-arity mismatch: ECL CLOCK (`0x34`) declares skip
    /// size 1 but its handler decodes 2 operands via one `vm_LoadCmdSets(2)`
    /// call. Skip must transcribe the *declared* size (1 batch), landing
    /// mid-operand relative to a normal decode — reproduced here by
    /// asserting the exact landing pc, not by relying on what happens to be
    /// there.
    #[test]
    fn if_false_over_ecl_clock_uses_the_declared_skip_size_not_real_consumption() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(2); // false
        b.op(0x16); // IF =
        b.label("clock");
        b.op(0x34); // ECL CLOCK opcode byte
        b.imm_byte(9); // operand 1 — the only batch the declared skip_size=1 consumes
        b.label("landing_if_declared_size_used"); // clock+1 (opcode) + 2 (1st batch)
        b.imm_byte(9); // operand 2 — real execution decodes this too; skip must NOT
        b.label("landing_if_real_consumption_used"); // where skip would land if it (wrongly) used the real 2-operand length
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let expected_landing = b.addr_of("landing_if_declared_size_used");
        let real_decode_end = b.addr_of("landing_if_real_consumption_used");
        assert_ne!(
            expected_landing, real_decode_end,
            "test fixture must actually exercise the divergence"
        );

        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h)); // COMPARE
        assert_continue(m.step(&mut h)); // IF =, false -> skip using ECL CLOCK's declared size (1)
        assert_eq!(m.current_pc(), Some(expected_landing));
    }

    /// Same divergence shape for ADD NPC (`0x36`).
    #[test]
    fn if_false_over_add_npc_uses_the_declared_skip_size_not_real_consumption() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(2); // false
        b.op(0x16); // IF =
        b.label("addnpc");
        b.op(0x36); // ADD NPC opcode byte
        b.imm_byte(9); // operand 1 — the only batch the declared skip_size=1 consumes
        b.label("landing_if_declared_size_used");
        b.imm_byte(9); // operand 2 — real execution decodes this too
        b.label("landing_if_real_consumption_used");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let expected_landing = b.addr_of("landing_if_declared_size_used");
        let real_decode_end = b.addr_of("landing_if_real_consumption_used");
        assert_ne!(expected_landing, real_decode_end);

        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h));
        assert_continue(m.step(&mut h));
        assert_eq!(m.current_pc(), Some(expected_landing));
    }

    /// Skip's side effects: string-mode operands in the skipped
    /// instruction's operand stream still fill the string registers,
    /// exactly like normal decode (`vm_LoadCmdSets`'s side effects run
    /// during `Skip()` too, `docs/design/vm-scriptmemory.md` §1). The
    /// skipped instruction here is a COMPARE with *two* string operands
    /// (skip_size 2, non-divergent — chosen only as a vehicle for two
    /// string fills in one call, not for its own semantics, since it's
    /// never executed); a later mixed-mode COMPARE's string path then
    /// reads slot 2, which only the *skipped* instruction could have set.
    #[test]
    fn skip_fills_string_registers_as_a_side_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(2); // priming COMPARE: 1==2 false
        b.op(0x16); // IF = : false -> skip next (COMPARE, skip_size 2)
        b.op(0x03).inline_str(b"SLOT1").inline_str(b"SLOT2"); // skipped;
                                                              // side effect alone should set slot1="SLOT1", slot2="SLOT2".
                                                              // Mixed compare: operand2's string refills slot 1 (always the
                                                              // *first* string slot, str_index-based); slot 2 is untouched by
                                                              // this instruction, so this only passes if the skip really set it.
        b.op(0x03).imm_byte(9).inline_str(b"SLOT2");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h)); // priming COMPARE
        assert_continue(m.step(&mut h)); // IF =, false -> skip (side effect fills both slots)
        assert_continue(m.step(&mut h)); // mixed COMPARE: slot2("SLOT2") vs slot1("SLOT2")
        assert_eq!(m.flags(), [true, false, false, false, true, true]);
    }

    /// Skip's `0x81` side effect: a memory-addressed string operand in a
    /// skipped instruction still performs a `ScriptMemory::read_string`
    /// call (`vm_CopyStringFromMemory`, `ovr008.cs:57-71`), even though the
    /// decoded value is discarded.
    #[test]
    fn skip_reads_through_memory_for_0x81_operand_as_a_side_effect() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(2); // false
        b.op(0x16); // IF =
        b.op(0x11).mem_str(0x4B00); // PRINT via mem_str, skip_size 1 (skipped)
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h
            .calls
            .iter()
            .any(|c| matches!(c, RecordedCall::MemReadString { addr: 0x4B00, .. })));
    }

    /// Skipping over an opcode the dialect doesn't know tolerates it (just
    /// a 1-byte advance, `ovr003.cs:2139-2143`) — unlike *executing* an
    /// unknown opcode, which is fatal (D-VM6).
    #[test]
    fn if_false_over_a_truly_unknown_opcode_is_tolerated() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).imm_byte(1).imm_byte(2); // false
        b.op(0x16); // IF =
        b.raw(&[0x41]); // 0x41 has no dialect entry at all
        b.label("landing");
        b.op(0x09).imm_byte(0xAA).imm_word(0x4B00); // SAVE probe
        b.op(0x00);

        let entry = b.addr_of("entry");
        let landing = b.addr_of("landing");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h)); // COMPARE
        assert_continue(m.step(&mut h)); // IF =, tolerated unknown skip target
        assert_eq!(m.current_pc(), Some(landing));
        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert_eq!(h.word(0x4B00), Some(0xAA));
    }
}

/// String-register persistence/staleness tests (§4): the 15-slot register
/// file is never bulk-cleared between instructions
/// (`docs/design/vm-scriptmemory.md` §1).
mod string_register_staleness {
    use super::*;

    /// The canonical staleness hazard (`ovr003.cs:72-77`): a mixed-mode
    /// COMPARE (one operand string, one numeric) always refills string slot
    /// 1 (`strIndex` starts at 0 each call and increments on the *first*
    /// string operand encountered, regardless of its cmd_opps position) but
    /// never touches slot 2 unless *both* operands are string-mode — so a
    /// mixed compare's slot 2 is whatever an earlier, unrelated instruction
    /// left there. Primed here by a COMPARE with two string operands (its
    /// own flags are irrelevant — it's only a vehicle to fill both slots at
    /// once); a later mixed compare's result then hinges entirely on that
    /// earlier slot 2, which contains a string ("CCC") that doesn't appear
    /// anywhere in the mixed compare's own operands.
    #[test]
    fn mixed_mode_compare_reads_a_stale_slot_from_a_prior_instruction() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).inline_str(b"AAA").inline_str(b"CCC"); // primes slot1="AAA", slot2="CCC"
                                                          // Mixed compare: operand1 numeric (no string effect), operand2
                                                          // string "AAA" -> refills slot 1 to "AAA" (already was). Slot 2 is
                                                          // untouched by *this* instruction, yet the string path still reads
                                                          // it: "CCC", stale from the priming COMPARE above.
        b.op(0x03).imm_byte(9).inline_str(b"AAA");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h)); // priming COMPARE
        assert_continue(m.step(&mut h)); // mixed COMPARE
                                         // compare_strings(string_a=slot2="CCC", string_b=slot1="AAA"):
                                         // "AAA" < "CCC" lexicographically.
        assert_eq!(m.flags(), [false, true, true, false, true, false]);
    }

    /// Registers are never bulk-cleared: slot 2, filled by the priming
    /// COMPARE, is still read stale by a mixed compare several unrelated
    /// instructions later.
    #[test]
    fn string_registers_persist_across_several_intervening_instructions() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x03).inline_str(b"AAA").inline_str(b"CCC"); // primes slot1="AAA", slot2="CCC"
        b.op(0x1C); // CLEARMONSTERS — unrelated, doesn't touch string regs
        b.op(0x1C); // another unrelated instruction
        b.op(0x03).imm_byte(9).inline_str(b"AAA"); // mixed compare, same as above
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_continue(m.step(&mut h)); // priming COMPARE
        assert_continue(m.step(&mut h)); // CLEARMONSTERS
        assert_continue(m.step(&mut h)); // CLEARMONSTERS
        assert_continue(m.step(&mut h)); // mixed COMPARE, 3 instructions after slot 2 was set
        assert_eq!(m.flags(), [false, true, true, false, true, false]);
    }
}

/// Suspension tests (§4): scripted replies, nested `enter()` while an outer
/// activation sits suspended, and serialize/restore round-tripping a
/// suspended machine.
mod suspension {
    use super::*;

    /// Nested `enter()` while an outer activation is suspended
    /// mid-instruction (the PROGRAM-9 camp case's shape, D-VM3): the outer
    /// DELAY suspends; while it's pending, `enter()` pushes an inner vector
    /// that runs to `Done`; `step()` on the (now again top) outer activation
    /// still correctly reports `StepWhilePending` until `resume()` supplies
    /// the reply.
    #[test]
    fn nested_enter_while_outer_activation_is_suspended() {
        let mut outer = EclBuilder::new();
        outer.label("outer_entry");
        outer.op(0x3A); // DELAY — suspends
        outer.label("outer_after");
        outer.op(0x00);
        outer.label("inner_entry");
        outer.op(0x00); // a trivial inner vector: immediately EXITs

        let outer_entry = outer.addr_of("outer_entry");
        let outer_after = outer.addr_of("outer_after");
        let inner_entry = outer.addr_of("inner_entry");

        let mut m = EclMachine::load_block(outer.build(), &COTAB).unwrap();
        let mut h = TestHost::new();
        m.enter(outer_entry);

        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay)));

        // Push the inner activation on top of the suspended outer one.
        m.enter(inner_entry);
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended))); // inner completes and pops

        // The outer activation is back on top, still awaiting its reply.
        assert_eq!(m.step(&mut h), Err(VmError::StepWhilePending));
        assert_eq!(m.pending(), Some(&Request::Delay));

        assert_continue(m.resume(Reply::Delay, &mut h));
        assert_eq!(m.current_pc(), Some(outer_after));
    }

    /// Serialize a suspended machine, restore it, and confirm `pending()`
    /// re-presents the outstanding request verbatim before `resume()`
    /// completes it — save-anywhere insurance (D-VM3).
    #[test]
    fn serialize_restore_then_resume_a_suspended_machine() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2B).imm_word(0x4B00).imm_byte(1).inline_str(b"OK");
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let expected_request = Request::HorizontalMenu {
            options: vec![VmString::from_bytes(*b"OK")],
        };
        assert_eq!(
            m.step(&mut h),
            Ok(VmStep::Request(expected_request.clone()))
        );

        let snapshot = m.snapshot();
        let restored = EclMachine::restore(snapshot, &COTAB).unwrap();
        assert_eq!(restored.pending(), Some(&expected_request));

        let mut restored = restored;
        assert_continue(restored.resume(Reply::Selection(0), &mut h));
        assert_eq!(h.word(0x4B00), Some(0));
        assert_eq!(restored.current_pc(), Some(after));
    }

    /// `restore` rejects an unknown snapshot version rather than migrating
    /// it (D-VM3).
    #[test]
    fn restore_rejects_unknown_snapshot_version() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x00);
        let entry = b.addr_of("entry");
        let m = machine_from(&b, entry);

        let mut snapshot = m.snapshot();
        snapshot.version = 9999;

        assert_eq!(
            EclMachine::restore(snapshot, &COTAB).unwrap_err(),
            crate::RestoreError::UnknownVersion(9999)
        );
    }

    /// A snapshot taken with the current version restores successfully.
    #[test]
    fn restore_accepts_the_current_snapshot_version() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x00);
        let entry = b.addr_of("entry");
        let m = machine_from(&b, entry);

        let snapshot = m.snapshot();
        assert!(EclMachine::restore(snapshot, &COTAB).is_ok());
    }
}

/// Effects-before-a-request ordering (D-VM3's MUST): CALL (0x2D) case
/// `0xE804` yields one `Effect` then one `Request` from the *same*
/// instruction — the effect must be observable before the request is ever
/// issued.
mod effects_then_request_ordering {
    use super::*;

    #[test]
    fn call_0xe804_yields_effect_before_request() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2D).imm_word(0x7FFFu16.wrapping_add(0xE804));
        b.label("after");
        b.op(0x00);

        let entry = b.addr_of("entry");
        let after = b.addr_of("after");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        // step() is legal here (mid-instruction "more effects" phase, not
        // awaiting-reply yet) and yields the frame draw first.
        assert_eq!(m.step(&mut h), Ok(VmStep::Effect(Effect::AnimationFrame)));
        // The *next* step() (still no resume() call) transitions into the
        // trailing request.
        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay)));
        assert_eq!(m.pending(), Some(&Request::Delay));
        // Now truly suspended: step() is illegal.
        assert_eq!(m.step(&mut h), Err(VmError::StepWhilePending));

        assert_continue(m.resume(Reply::Delay, &mut h));
        assert_eq!(m.current_pc(), Some(after));
    }
}

/// `Exit::ChainTo` tests: NEWECL abandons the *entire* activation stack, not
/// just the top frame (D-VM3: "no VM context ever resumes across a chain").
mod chain_to {
    use super::*;

    #[test]
    fn newecl_abandons_the_whole_activation_stack() {
        let mut b = EclBuilder::new();
        b.label("outer");
        b.op(0x3A); // DELAY — suspends, parking a frame mid-instruction
        b.label("inner_with_newecl");
        b.op(0x20).imm_byte(3); // NEWECL 3

        let outer = b.addr_of("outer");
        let inner = b.addr_of("inner_with_newecl");
        let mut m = EclMachine::load_block(b.build(), &COTAB).unwrap();
        let mut h = TestHost::new();

        m.enter(outer);
        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay))); // outer parked

        m.enter(inner); // nested activation on top of the parked outer
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::ChainTo(BlockId(3)))));

        // Both the inner *and* the parked outer are gone.
        assert!(m.is_idle());
        assert_eq!(m.pending(), None);
    }
}

/// `VmError` call-legality tests (§3's table): every illegal call shape.
mod vm_error_legality {
    use super::*;

    #[test]
    fn step_on_an_idle_machine_is_idle_error() {
        let mut b = EclBuilder::new();
        b.op(0x00);
        let mut m = EclMachine::load_block(b.build(), &COTAB).unwrap();
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Err(VmError::Idle));
    }

    #[test]
    fn resume_on_an_idle_machine_is_resume_without_pending() {
        let mut b = EclBuilder::new();
        b.op(0x00);
        let mut m = EclMachine::load_block(b.build(), &COTAB).unwrap();
        let mut h = TestHost::new();

        assert_eq!(
            m.resume(Reply::Delay, &mut h),
            Err(VmError::ResumeWithoutPending)
        );
    }

    #[test]
    fn resume_without_an_outstanding_request_is_resume_without_pending() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x00); // EXIT: never suspends
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        // The activation is already Done/popped, so this is also Idle-ish,
        // but per the call-legality table resume's own failure is
        // ResumeWithoutPending regardless of *why* nothing is pending.
        assert_eq!(m.step(&mut h), Ok(VmStep::Done(Exit::Ended)));
        assert_eq!(
            m.resume(Reply::Delay, &mut h),
            Err(VmError::ResumeWithoutPending)
        );
    }

    #[test]
    fn step_while_awaiting_reply_is_step_while_pending() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3A); // DELAY
        b.op(0x00);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay)));
        assert_eq!(m.step(&mut h), Err(VmError::StepWhilePending));
    }

    #[test]
    fn resume_with_a_mismatched_reply_kind_is_reply_mismatch() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x3A); // DELAY expects Reply::Delay
        b.op(0x00);
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(m.step(&mut h), Ok(VmStep::Request(Request::Delay)));
        assert_eq!(
            m.resume(Reply::Selection(0), &mut h),
            Err(VmError::ReplyMismatch)
        );
    }

    /// `0x1F` is dialect-known (coab's own null-handler entry) but has no
    /// Restrike handler — `VmError::Unimplemented`, distinct from a byte
    /// with no dialect entry at all (D-VM6 audit note, M1 run-script task).
    #[test]
    fn executing_a_dialect_known_but_unimplemented_opcode_halts_as_unimplemented() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.raw(&[0x1F, 0x00, 0x00]); // 0x1F: known to the dialect table, not to this interpreter
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        let err = m.step(&mut h).unwrap_err();
        assert_eq!(
            err,
            VmError::Unimplemented {
                pc: entry,
                opcode: 0x1F
            }
        );
        // Halted: the pc never moved, so stepping again reproduces it.
        assert_eq!(m.step(&mut h).unwrap_err(), err);
    }

    #[test]
    fn executing_a_completely_out_of_table_opcode_halts_the_machine() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.raw(&[0x41]); // no dialect entry at all
        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();

        assert_eq!(
            m.step(&mut h),
            Err(VmError::UnknownOpcode {
                pc: entry,
                opcode: 0x41
            })
        );
    }
}
