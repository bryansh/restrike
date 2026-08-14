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
