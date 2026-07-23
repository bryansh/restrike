use super::*;
use crate::rng::{RngDraw, RngSink};
use gbx_prng::Prng;
use std::cell::RefCell;
use std::rc::Rc;

mod affects;
mod ai;
mod attack;
mod core;
mod facing;
mod records;

const FLOOR: u8 = 0x17; // a passable floor tile (move_cost 1)

// --- test doubles ------------------------------------------------------

/// Records the operand `n` and `result` of every PRNG draw at the engine
/// seam — lets a test assert the *exact* draw sequence (kinds and values).
#[derive(Clone, Default)]
struct DrawLog {
    draws: Rc<RefCell<Vec<RngDraw>>>,
}
struct DrawSink(Rc<RefCell<Vec<RngDraw>>>);
impl RngSink for DrawSink {
    fn on_draw(&mut self, draw: RngDraw) {
        self.0.borrow_mut().push(draw);
    }
}
impl DrawLog {
    fn sink(&self) -> Box<dyn RngSink> {
        Box::new(DrawSink(Rc::clone(&self.draws)))
    }
    fn ns(&self) -> Vec<u16> {
        self.draws.borrow().iter().map(|d| d.n.unwrap()).collect()
    }
    fn len(&self) -> usize {
        self.draws.borrow().len()
    }
}

/// Records every emitted action event.
#[derive(Clone, Default)]
struct ActionLog {
    events: Rc<RefCell<Vec<ActionEvent>>>,
}
struct ActionSinkImpl(Rc<RefCell<Vec<ActionEvent>>>);
impl ActionSink for ActionSinkImpl {
    fn on_action(&mut self, event: ActionEvent) {
        self.0.borrow_mut().push(event);
    }
}
impl ActionLog {
    fn sink(&self) -> Box<dyn ActionSink> {
        Box::new(ActionSinkImpl(Rc::clone(&self.events)))
    }
    fn events(&self) -> Vec<ActionEvent> {
        self.events.borrow().clone()
    }
}

/// An independent replay of the same seed — the by-hand oracle for what
/// `1 + random(size)` yields, so tests derive expected delays/rolls without
/// trusting the code under test.
struct Replay(Prng);
impl Replay {
    fn new(seed: u32) -> Self {
        Replay(Prng::new(seed))
    }
    fn roll(&mut self, size: u16) -> u16 {
        1 + self.0.random(size)
    }
}

const SEED: u32 = 0x0C0F_FEE0; // the §15 capture seed, reused

fn party(id: usize, reaction_adj: i8) -> Combatant {
    Combatant::new(id, Team::Party, reaction_adj, true)
}
fn monster(id: usize) -> Combatant {
    Combatant::new(id, Team::Monster, 0, true)
}

fn clamp_init(d6: u16, reaction_adj: i8) -> i8 {
    // The CalculateInitiative clamp with no surprise (surprise_mask == 0).
    let mut delay = d6 as i32 + reaction_adj as i32;
    if delay < 1 {
        delay = 1;
    }
    if !(0..=20).contains(&delay) {
        delay = 0;
    }
    delay as i8
}

/// A synthetic `ITEMS` table with the rows the ranged tests exercise (doc
/// §34.1) plus a natk-1 launcher (type 45) for the floor test and a range-1
/// weapon (type 30) for the range sanitize test.
fn synth_item_table() -> gbx_formats::items::ItemDataTable {
    let mut bytes = vec![0u8; 2 + 0x81 * 0x10];
    let mut set = |t: usize, e: [u8; 16]| {
        let off = 2 + t * 0x10;
        bytes[off..off + 16].copy_from_slice(&e);
    };
    // 43 LongBow: range 22, natk 4, 1d6 normal, flags 0x0B (arrows|02|08).
    set(
        43,
        [0, 2, 1, 6, 0, 4, 0, 1, 0x80, 1, 6, 0, 22, 0xC8, 0x0B, 0],
    );
    // 47 Sling: range 21, flags 0x0A (flag_08|flag_02), 1d4+1 normal.
    set(
        47,
        [0, 1, 1, 6, 1, 2, 0, 0x80, 0x80, 1, 4, 1, 21, 0xDC, 0x0A, 0],
    );
    // 45 (a natk-1 launcher): range 5, natk 1, flags 0x0B.
    set(
        45,
        [0, 2, 1, 8, 0, 1, 0, 1, 0x80, 1, 8, 0, 5, 0xC8, 0x0B, 0],
    );
    // 30 (a range-1 melee weapon): range 1, flags 0x04.
    set(
        30,
        [0, 1, 1, 8, 0, 0, 0, 0, 0x80, 1, 8, 0, 1, 0xCC, 0x04, 0],
    );
    gbx_formats::items::ItemDataTable::parse(&bytes).unwrap()
}

fn place_input(team: Team) -> PlacementInput {
    PlacementInput {
        team,
        size: 1,
        in_combat: true,
    }
}
