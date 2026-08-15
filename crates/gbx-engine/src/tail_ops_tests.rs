//! ★ Roll-credits **slice 9a**'s acceptance suite (`roll-credits.md` §13):
//! the visible tail — ROB (0x28), WHO (0x39), INPUT STRING (0x10) and
//! SPELL (0x3B) — each driven at a **shipped site**, against real CotAB data,
//! through the real engine host.
//!
//! The vehicle is slice 1's own: [`crate::area_transition_tests::real_data_engine`]
//! imports a one-member synthetic save against the user's real `.DAX` files
//! and [`crate::shell::boot_at_address`] starts a `VectorRun` at any address
//! in the resident block — so a drive can begin on the instruction itself
//! rather than replaying everything upstream of it.
//!
//! Local tier throughout: no `GBX_DATA_DIR`, loud skip (D10).

#![cfg(test)]

use crate::area_transition_tests::{real_data_engine, run_until};
use crate::engine::Engine;

/// Runs the machine until the pc leaves `from` — one shipped instruction,
/// no further.
fn step_past(engine: &mut Engine, from: u16) -> bool {
    run_until(engine, 200, |e| e.machine.current_pc() != Some(from))
}

/// Ticks until the run parks on a gate (or gives up).
fn run_to_gate(engine: &mut Engine, max: u32) -> bool {
    for _ in 0..max {
        if engine.shell().gate_open() {
            return true;
        }
        engine.tick(&[]);
    }
    engine.shell().gate_open()
}

fn item_weighing(weight: i16) -> Vec<u8> {
    let mut rec = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
    rec[0x37..0x39].copy_from_slice(&weight.to_le_bytes());
    rec
}

// --- ROB (0x28) ----------------------------------------------------------

/// ★ **The shipped ROB, driven live.** `ECL2#3 @0x88DB` — `ROB 0x01, 0x4B,
/// 0x7D`: every `TeamList` member, 75% of the money taken, and a per-item
/// d100 threshold of **125**, i.e. certain unless the weight ladder buys the
/// item out.
///
/// The instruction after it is `GOTO 0x9B52`.
#[test]
fn the_shipped_ecl2_rob_takes_three_quarters_and_the_light_items() {
    const SITE: u16 = 0x88DB;
    let Some(mut engine) = real_data_engine(2, 3, 3, true) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (tail_ops_tests::the_shipped_ecl2_rob_takes_three_quarters_and_the_light_items)"
        );
        return;
    };

    // A purse and four items: one heavy enough to knock 90 off the chance,
    // then three featherweights that inherit the reduced chance.
    engine.party.members[0].money = crate::party::Money {
        copper: 100,
        silver: 50,
        electrum: 33,
        gold: 20,
        platinum: 7,
        gems: 9,
        jewelry: 3,
    };
    engine.party.members[0].items = vec![
        item_weighing(300),
        item_weighing(1),
        item_weighing(1),
        item_weighing(1),
    ];

    engine.shell = crate::shell::boot_at_address(&mut engine.machine, SITE);
    assert!(step_past(&mut engine, SITE), "the ROB executed");

    let m = &engine.party().members[0];
    // `(100 - 0x4B) / 100.0` = 0.25, truncated, over all SEVEN denominations.
    assert_eq!(
        [
            m.money.copper,
            m.money.silver,
            m.money.electrum,
            m.money.gold,
            m.money.platinum,
            m.money.gems,
            m.money.jewelry,
        ],
        [25, 12, 8, 5, 1, 2, 0],
        "★ gems and jewelry were taken too — coab's ScaleAll stops at platinum"
    );
    assert!(
        m.items.len() < 4,
        "chance 125 took at least one item; kept {:?}",
        m.items
            .iter()
            .map(|it| gbx_formats::save_orig::item_weight(it))
            .collect::<Vec<_>>()
    );
    assert!(
        engine.vm_memory().halts.is_empty(),
        "no halt: {:?}",
        engine.vm_memory().halts
    );
}

// --- WHO (0x39) ----------------------------------------------------------

/// ★ **The shipped WHO, driven live.** `ECL4#35 @0x8A15` — the shortest of
/// the seven prompts, followed by `SAVE [0x7EB1], [0x7F7A]` and
/// `SAVE [0x7EB1], [0x7F7B]`, which only mean anything once somebody is
/// selected.
///
/// `CMD_Who` (`ovr003.cs:1757-1765`) calls `selectAPlayer(ref SelectedPlayer,
/// showExit: false, prompt)`, whose loop only ends on one of the four
/// elements of `unk_68DFA` — `{0x0D, 0x1B, 'E', 'S'}`. The drive proves both
/// halves: a letter outside the set re-prompts, and Enter commits.
#[test]
fn the_shipped_ecl4_who_reprompts_on_a_stray_key_and_commits_on_enter() {
    const SITE: u16 = 0x8A15;
    // `GEO4.DAX` holds {32, 33, 37}; block 35 is an ECL id with no map of its
    // own, so the drive boots on the area's first real one.
    let Some(mut engine) = real_data_engine(4, 35, 33, true) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (tail_ops_tests::the_shipped_ecl4_who_reprompts_on_a_stray_key_and_commits_on_enter)"
        );
        return;
    };

    engine.shell = crate::shell::boot_at_address(&mut engine.machine, SITE);
    assert!(run_to_gate(&mut engine, 100), "WHO opened a gate");
    let line = engine
        .vm_memory()
        .transcript
        .iter()
        .map(|e| format!("{e:?}"))
        .find(|s| s.contains("who:"))
        .expect("the transcript names the picker");
    eprintln!("  {line}");

    // A key outside `unk_68DFA` re-prompts: the gate stays open and the pc
    // has not moved.
    engine.tick(&[crate::input::InputEvent::Char(b'X')]);
    assert!(engine.shell().gate_open(), "'X' re-prompted");
    assert_eq!(engine.machine.current_pc(), Some(SITE));

    // Enter resolves on the highlighted word, `Select`.
    assert!(
        run_until(&mut engine, 200, |e| e.machine.current_pc() != Some(SITE)),
        "Enter committed the pick"
    );
    assert_eq!(engine.state().selected_player, 0);
    assert!(
        engine.vm_memory().halts.is_empty(),
        "no halt: {:?}",
        engine.vm_memory().halts
    );
}

/// ★ The cell every WHO site tests on its very next instruction:
/// `COMPARE [0x7D00], 1` (`ECL2#3 @0x952A`, `ECL4#32 @0x8B3A`,
/// `ECL5#50 @0x8394`, `ECL5#51 @0x8C67`/`@0x91C4`).
///
/// `ECL5#50 @0x8388` is the drive:
///
/// ```text
/// 0x8388  WHO "<9 chars>"
/// 0x8394  COMPARE [0x7D00], 1
/// 0x839A  IF =
/// 0x839B  GOTO 0x83BE          <- the selected member is there: get on with it
/// 0x839F  PRINTCLEAR "<24 chars>"
/// 0x83BA  GOTO 0x8388          <- they are not: say so and ASK AGAIN
/// ```
///
/// The drive starts on the COMPARE, so the very next gate names the arm: the
/// equal arm runs into `@0x83C2 DAMAGE`, whose closing `press_any_key` is a
/// `pause:` in the transcript, while the not-equal arm prints its refusal and
/// comes straight back to the `who:` picker.
#[test]
fn the_shipped_ecl5_who_gates_on_the_selected_players_state_cell() {
    const COMPARE_SITE: u16 = 0x8394;
    for (in_combat, expect_first_gate) in [(true, "pause:"), (false, "who:")] {
        let Some(mut engine) = real_data_engine(5, 50, 50, true) else {
            eprintln!(
                "SKIPPED: local tier needs GBX_DATA_DIR \
                 (tail_ops_tests::the_shipped_ecl5_who_gates_on_the_selected_players_state_cell)"
            );
            return;
        };
        engine.party.members[0].status.in_combat = in_combat;
        engine.shell = crate::shell::boot_at_address(&mut engine.machine, COMPARE_SITE);
        assert!(run_to_gate(&mut engine, 400), "the arm reached a gate");
        let first = engine
            .vm_memory()
            .transcript
            .iter()
            .map(|e| format!("{e:?}"))
            .find(|s| s.contains("who:") || s.contains("pause:"))
            .unwrap_or_default();
        eprintln!("  in_combat={in_combat} -> first gate {first}");
        assert!(
            first.contains(expect_first_gate),
            "in_combat={in_combat}: [0x7D00] reads {}, expected {expect_first_gate}, got {first}",
            if in_combat { 1 } else { 0x80 }
        );
    }
}

// --- INPUT STRING (0x10) -------------------------------------------------

/// Reads an inline-string operand straight out of a shipped block — the
/// password the drive below has to type.
fn inline_string(file: &str, block_id: u8, addr: u16, operand: usize) -> Option<String> {
    let dir = std::env::var_os("GBX_DATA_DIR")?;
    let bytes = std::fs::read(std::path::Path::new(&dir).join(file)).ok()?;
    let archive = gbx_formats::dax::DaxArchive::parse(&bytes).ok()?;
    let raw = archive.block_data(block_id).ok()?;
    let block = gbx_vm::BlockBytes::from_bytes(gbx_formats::dax::ecl_block_payload(&raw));
    let instr = gbx_vm::decode(&block, addr, &gbx_vm::COTAB).ok()?;
    match instr.args.get(operand)? {
        gbx_vm::Arg::InlineStr(packed) => {
            String::from_utf8(gbx_formats::ecl_text::decompress(packed)).ok()
        }
        _ => None,
    }
}

/// ★ **The shipped INPUT STRING, driven live.** `ECL6#64 @0x8425` —
/// `INPUT STRING 0x0C, [0x7B90]`, immediately followed by
/// `COMPARE [0x7B90], "<9 characters>"` and `IF <>`: a password gate, and
/// the machinery slice 9c's name entry inherits.
///
/// Both arms are driven, and each is identified by the line it prints:
///
/// ```text
/// 0x843A  IF <>
/// 0x843B  GOTO 0x846A          <- wrong: 0x846A's line, then the ambush
/// 0x843F  PRINTCLEAR "<...>"   <- right: this line
/// ```
#[test]
fn the_shipped_ecl6_password_gate_reads_both_ways() {
    const SITE: u16 = 0x8425;
    const CELL: u16 = 0x7B90;
    let Some(answer) = inline_string("ECL6.DAX", 64, 0x842B, 1) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (tail_ops_tests::the_shipped_ecl6_password_gate_reads_both_ways)"
        );
        return;
    };
    eprintln!("  ECL6#64's password: {answer:?}");
    // ★ `TYRANTHRAXUS` — twelve characters, and the INPUT STRING's own
    // (dead) first operand is `0x0C`. The authorial intent and the answer
    // agree exactly; the engine's hardcoded 40 is simply more generous.
    assert_eq!(answer.len(), 12);
    let right_line = inline_string("ECL6.DAX", 64, 0x843F, 0).expect("the accept line");
    let wrong_line = inline_string("ECL6.DAX", 64, 0x846A, 0).expect("the refuse line");
    eprintln!("  accept: {right_line:?}\n  refuse: {wrong_line:?}");

    for (typed, correct) in [(answer.clone(), true), ("WRONG".to_string(), false)] {
        let mut engine = real_data_engine(6, 64, 64, true).expect("data is present");
        engine.shell = crate::shell::boot_at_address(&mut engine.machine, SITE);
        assert!(run_to_gate(&mut engine, 200), "the editor opened");

        for ch in typed.bytes() {
            engine.tick(&[crate::input::InputEvent::Char(ch)]);
        }
        engine.tick(&[crate::input::InputEvent::Enter]);
        assert!(
            run_until(&mut engine, 200, |e| e.machine.current_pc() != Some(SITE)),
            "the entry resumed the script"
        );

        assert_eq!(
            engine
                .vm_memory()
                .raw_string(CELL)
                .map(|s| String::from_utf8_lossy(&s.0).into_owned()),
            Some(typed.clone()),
            "the typed line landed in the destination cell"
        );

        let printed: Vec<String> = engine
            .vm_memory()
            .transcript
            .iter()
            .filter_map(|e| match e {
                crate::vmhost::TranscriptEntry::Print { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        eprintln!("  typed {typed:?} -> printed {printed:?}");
        assert_eq!(
            printed.contains(&right_line),
            correct,
            "typed {typed:?}: the compare took the wrong arm"
        );
        assert_eq!(
            printed.contains(&wrong_line),
            !correct,
            "typed {typed:?}: the compare took the wrong arm"
        );
        assert!(
            engine.vm_memory().halts.is_empty(),
            "no halt: {:?}",
            engine.vm_memory().halts
        );
    }
}

/// ★ Esc is not a cancel (`seg041.cs:270`) and an empty line becomes a single
/// space (`ovr003.cs:379-382`) — driven at the same shipped site, with
/// nothing typed.
#[test]
fn the_shipped_input_string_commits_on_escape_and_an_empty_line_becomes_a_space() {
    const SITE: u16 = 0x8425;
    const CELL: u16 = 0x7B90;
    let Some(mut engine) = real_data_engine(6, 64, 64, true) else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR (tail_ops_tests::\
             the_shipped_input_string_commits_on_escape_and_an_empty_line_becomes_a_space)"
        );
        return;
    };
    engine.shell = crate::shell::boot_at_address(&mut engine.machine, SITE);
    assert!(run_to_gate(&mut engine, 200), "the editor opened");
    engine.tick(&[crate::input::InputEvent::Escape]);
    assert!(
        run_until(&mut engine, 200, |e| e.machine.current_pc() != Some(SITE)),
        "Esc ended the editor rather than parking forever"
    );
    assert_eq!(
        engine
            .vm_memory()
            .raw_string(CELL)
            .map(|s| s.0.clone())
            .as_deref(),
        Some(&b" "[..]),
        "the empty line became a single space"
    );
}
