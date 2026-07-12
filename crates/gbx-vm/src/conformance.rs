//! §4 conformance suite (`docs/design/vm-scriptmemory.md`): micro-ECL
//! programs, hand-authored via `EclBuilder` (D10 — nothing here is derived
//! from real game data), asserting on the yielded step stream, memory
//! traffic (`TestHost::calls`), service-call recordings, flag state, and pc
//! trajectory. Every opcode this session implements gets at least one test
//! citing its coab handler; the mandatory cross-cutting test classes (skip
//! semantics, string-register staleness, suspension, effect/request
//! ordering, `ChainTo`, `VmError` legality) each get their own module.

use crate::dialect::COTAB;
use crate::host::{MissingData, MonsterHandle, RecordedCall};
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

    /// SETUP MONSTER (0x0C), `CMD_SetupMonster` ovr003.cs:215-236.
    #[test]
    fn setup_monster_calls_its_three_services_in_order() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x0C).imm_byte(1).imm_byte(2).imm_byte(3); // sprite,max_dist,pic
        b.op(0x00);

        let entry = b.addr_of("entry");
        let mut m = machine_from(&b, entry);
        let mut h = TestHost::new();
        h.approach_distance_replies.push_back(10);

        assert_eq!(run_until_done(&mut m, &mut h), Exit::Ended);
        assert!(h.calls.contains(&RecordedCall::SetupMonster {
            sprite_id: 1,
            max_distance: 2,
            pic_id: 3
        }));
        assert!(h.calls.contains(&RecordedCall::ApproachDistance));
        // Clamped by max_distance (2), matching `if (max_distance <
        // encounter_distance) encounter_distance = max_distance;`.
        assert!(h.calls.contains(&RecordedCall::LoadEncounterVisual {
            flags: 0,
            distance: 2,
            pic_id: 3,
            sprite_id: 1,
        }));
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
