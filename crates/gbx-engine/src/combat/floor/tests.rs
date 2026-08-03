//! Table-exact tests for the faithful `SetupDungeonFloor` (D-CV6 item 1).
//!
//! These prove the **transliteration**, not the original's behaviour: no
//! capture has ever been staged on a floor this code generated (§D-CV6's
//! honesty rule). Each test names the coab site it pins, and the furniture
//! tests pin the draw ORDER specifically — that is the shape the first
//! live-fight capture will check against.

use super::*;
use crate::combat::{tile_passability, TilePassability, BACKGROUND_TILE_INDEX};
use crate::rng::{EngineRng, RngDraw, RngSink};
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use std::cell::RefCell;
use std::rc::Rc;

const PLANE_NE: usize = 2;
const PLANE_SW: usize = 2 + 256;
const PLANE_X2: usize = 2 + 2 * 256;
const PLANE_DOOR: usize = 2 + 3 * 256;

/// A hand-authored GEO block (D10) whose squares this module's tests set
/// individually.
#[derive(Default)]
struct GeoBuilder {
    data: Vec<u8>,
}

impl GeoBuilder {
    fn new() -> Self {
        GeoBuilder {
            data: vec![0u8; GEO_BLOCK_SIZE],
        }
    }
    /// Wall-type nibbles: any nonzero value is "a wall" to every consumer here.
    fn walls(mut self, x: usize, y: usize, n: u8, e: u8, s: u8, w: u8) -> Self {
        self.data[PLANE_NE + x + 16 * y] = (n << 4) | e;
        self.data[PLANE_SW + x + 16 * y] = (s << 4) | w;
        self
    }
    /// Door 2-bit fields (only meaningful where the matching wall is nonzero).
    fn doors(mut self, x: usize, y: usize, n: u8, e: u8, s: u8, w: u8) -> Self {
        self.data[PLANE_DOOR + x + 16 * y] = n | (e << 2) | (s << 4) | (w << 6);
        self
    }
    /// The `x2` byte — bit `0x40` is `byte_1AD3D`, the furniture gate.
    fn x2(mut self, x: usize, y: usize, value: u8) -> Self {
        self.data[PLANE_X2 + x + 16 * y] = value;
        self
    }
    fn build(self) -> GeoBlock {
        GeoBlock::parse(&self.data).expect("the synthetic block is well-formed")
    }
}

fn open_geo() -> GeoBlock {
    GeoBuilder::new().build()
}

fn cell(map: &CombatMap, x: i32, y: i32) -> u8 {
    map.ground_tile(GridPos::new(x, y))
}

/// A PRNG that records every draw's `Random(n)` operand — the furniture tests
/// assert on the operand SEQUENCE, which is what a capture would pin.
#[derive(Clone, Default)]
struct Tap(Rc<RefCell<Vec<RngDraw>>>);
struct TapSink(Rc<RefCell<Vec<RngDraw>>>);
impl RngSink for TapSink {
    fn on_draw(&mut self, d: RngDraw) {
        self.0.borrow_mut().push(d);
    }
}
impl Tap {
    fn rng(&self, seed: u32) -> EngineRng {
        let mut rng = EngineRng::new(seed);
        rng.attach_sink(Box::new(TapSink(Rc::clone(&self.0))));
        rng
    }
    fn count(&self) -> usize {
        self.0.borrow().len()
    }
    fn operands(&self) -> Vec<Option<u16>> {
        self.0.borrow().iter().map(|d| d.n).collect()
    }
}

// --- the shear -----------------------------------------------------------

#[test]
fn set_background_tile_writes_the_id_plus_one_at_the_sheared_cell() {
    // `sub_37046` (`ovr011.cs:11-23`): combatX = dx*6 + dy*5 + 21 + x,
    // combatY = dy*5 + 10 + y, and the stored value is `tileId + 1`.
    for (dx, dy, x, y) in [(0, 0, 0, 0), (1, 0, 3, 2), (-1, 2, 5, 4), (6, -2, 2, 1)] {
        let geo = open_geo();
        let mut pass = FloorPass {
            ground: vec![0u8; (MAP_W * MAP_H) as usize],
            geo: &geo,
            ecl_block_id: 1,
            party_y: 0,
            dx,
            dy,
            dir_0: 0,
            dir_2: 0,
            dir_4: 0,
            dir_6: 0,
            byte_1ad3d: 0,
        };
        pass.set_tile(22, y, x);
        let cx = dx * 6 + dy * 5 + 21 + x;
        let cy = dy * 5 + 10 + y;
        assert_eq!(
            pass.ground[(cy * MAP_W + cx) as usize],
            23,
            "tile id 22 stores 0x17 at ({cx},{cy})"
        );
    }
}

#[test]
fn the_floor_tile_the_builders_lay_is_the_cost_one_floor_the_furniture_test_looks_for() {
    // Two constants that must agree or the furniture dice never fire:
    // `set_background_tile(22, …)` writes ground 0x17, and
    // `BackGroundTiles[0x17].tile_index` is 0x16.
    assert_eq!(BACKGROUND_TILE_INDEX[0x17], FLOOR_TILE_INDEX);
    assert_eq!(
        tile_passability(0x17),
        TilePassability::Passable { move_cost: 1 }
    );
}

#[test]
fn the_band_covers_every_cell_of_the_combat_grid() {
    // A consequence of the shear worth pinning: `dx` steps the patch by 6 while
    // each source square paints 7 columns, and `dy` steps it by 5 while each
    // paints 5 rows — so the 13x5 band tiles the whole 50x25 field with no
    // seams and no leftovers. Nothing is left at the zero-fill
    // (`gbl.mapToBackGroundTile = new Struct_1D1BC()`, `ovr011.cs:768`), and
    // the party's world position never enters the transform, so this holds
    // wherever the fight starts.
    for party in [(0, 0), (8, 8), (15, 15), (3, 12)] {
        let mut rng = EngineRng::new(1);
        let map = setup_dungeon_floor(&open_geo(), party, 1, &mut rng);
        for y in 0..MAP_H {
            for x in 0..MAP_W {
                assert_ne!(
                    tile_passability(cell(&map, x, y)),
                    TilePassability::Void,
                    "party {party:?}: cell ({x},{y}) was never painted"
                );
            }
        }
    }
}

#[test]
fn an_all_open_area_paints_a_solid_floor_and_nothing_else() {
    // With no walls anywhere every builder falls to its "plain floor" arm —
    // ids 22 (`build_tiles_1`/`_2`) and 0x16 (`_3`/`_4`), which are the SAME
    // ground value 0x17. A wall-free dungeon fights on open ground.
    let mut rng = EngineRng::new(1);
    let map = setup_dungeon_floor(&open_geo(), (8, 8), 1, &mut rng);
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            assert_eq!(cell(&map, x, y), 0x17, "cell ({x},{y})");
        }
    }
}

// --- the wall runs -------------------------------------------------------

#[test]
fn a_north_wall_becomes_a_horizontal_run_of_5_over_10() {
    // `build_background_tiles_2` (`ovr011.cs:169-192`): dir_0 == 1 lays tile ids
    // 5 (row 0) and 10 (row 1) at x = 3,4 — ground values 6 and 11.
    let geo = GeoBuilder::new().walls(8, 8, 1, 0, 0, 0).build();
    let mut rng = EngineRng::new(1);
    let map = setup_dungeon_floor(&geo, (8, 8), 1, &mut rng);
    let o = party_square_origin();
    assert_eq!(cell(&map, o.x + 3, o.y), 6);
    assert_eq!(cell(&map, o.x + 4, o.y), 6);
    assert_eq!(cell(&map, o.x + 3, o.y + 1), 11);
    assert_eq!(cell(&map, o.x + 4, o.y + 1), 11);
}

#[test]
fn a_west_wall_becomes_a_diagonal_run_stepping_one_one_per_row() {
    // `build_background_tiles_1` (`ovr011.cs:143-166`): dir_6 == 1 lays ids
    // 4/3/13 at (v-1, v), (v, v), (v+1, v) for v = 2..=4 — the shear's
    // signature, a diagonal, not a column.
    let geo = GeoBuilder::new().walls(8, 8, 0, 0, 0, 1).build();
    let mut rng = EngineRng::new(1);
    let map = setup_dungeon_floor(&geo, (8, 8), 1, &mut rng);
    let o = party_square_origin();
    for v in 2..=4i32 {
        assert_eq!(cell(&map, o.x + v - 1, o.y + v), 5, "row {v} tile id 4");
        assert_eq!(cell(&map, o.x + v, o.y + v), 4, "row {v} tile id 3");
        assert_eq!(cell(&map, o.x + v + 1, o.y + v), 14, "row {v} tile id 13");
    }
}

#[test]
fn a_west_door_lays_the_two_jamb_tiles_instead_of_the_wall_run() {
    // dir_6 == 3 (a door): `set_background_tile(8, 2, 1)` + `(0, 4, 5)`.
    let geo = GeoBuilder::new()
        .walls(8, 8, 0, 0, 0, 1)
        .doors(8, 8, 0, 0, 0, 2) // west door state 2 (locked) — any nonzero
        .build();
    let mut rng = EngineRng::new(1);
    let map = setup_dungeon_floor(&geo, (8, 8), 1, &mut rng);
    let o = party_square_origin();
    assert_eq!(cell(&map, o.x + 1, o.y + 2), 9, "tile id 8");
    assert_eq!(cell(&map, o.x + 5, o.y + 4), 1, "tile id 0");
    // and the wall run is absent
    assert_ne!(cell(&map, o.x + 3, o.y + 3), 4);
}

#[test]
fn dir_flags_is_the_or_of_both_sides_of_the_shared_wall() {
    // `get_dir_flags`: the wall between (8,8) and (8,7) is declared on the
    // SOUTH side of (8,7) only; probing NORTH from (8,8) must still see it.
    let geo = GeoBuilder::new().walls(8, 7, 0, 0, 1, 0).build();
    assert_eq!(dir_flags(&geo, 8, 0, 8, 8), 1, "north from (8,8)");
    assert_eq!(dir_flags(&geo, 8, 4, 8, 7), 1, "south from (8,7)");
    assert_eq!(dir_flags(&geo, 8, 2, 8, 8), 0, "east is open");

    // A door on either side makes the shared flag a door (3 dominates 1).
    let geo = GeoBuilder::new()
        .walls(8, 7, 0, 0, 1, 0)
        .doors(8, 7, 0, 0, 1, 0)
        .build();
    assert_eq!(dir_flags(&geo, 8, 0, 8, 8), 3);
}

#[test]
fn off_grid_squares_read_open_only_along_the_partys_own_row() {
    // `sub_37306`'s off-grid arm (`ovr011.cs:113-124`).
    let geo = open_geo();
    // Party row 8; probing EAST from (16,8) — off grid, same row → open.
    assert_eq!(wall_flags_at(&geo, 8, 2, 16, 8), 0);
    assert_eq!(wall_flags_at(&geo, 8, 6, 16, 8), 0);
    // North/south off-grid, or a different row → wall.
    assert_eq!(wall_flags_at(&geo, 8, 0, 16, 8), 1);
    assert_eq!(wall_flags_at(&geo, 8, 2, 16, 9), 1);
}

#[test]
fn the_x2_read_wraps_a_single_step_like_the_original() {
    // `get_wall_x2` (`ovr031.cs:296-312`): > 0x0F → 0, < 0 → 0x0F. Not modular.
    let geo = GeoBuilder::new().x2(0, 0, 0x40).x2(15, 15, 0x40).build();
    let pass = FloorPass {
        ground: Vec::new(),
        geo: &geo,
        ecl_block_id: 1,
        party_y: 0,
        dx: 0,
        dy: 0,
        dir_0: 0,
        dir_2: 0,
        dir_4: 0,
        dir_6: 0,
        byte_1ad3d: 0,
    };
    assert_eq!(pass.wall_x2(16, 16) & 0x40, 0x40, "wraps to (0,0)");
    assert_eq!(pass.wall_x2(-1, -1) & 0x40, 0x40, "wraps to (15,15)");
    assert_eq!(
        pass.wall_x2(-7, -7) & 0x40,
        0x40,
        "a single step, not mod 16"
    );
}

// --- the furniture dice (the draw-bearing half) ---------------------------

/// A square that passes every furniture gate: walls on all four sides (a room,
/// no doors) and the `0x40` x2 bit set.
fn furnished_room_geo() -> GeoBlock {
    GeoBuilder::new()
        .walls(8, 8, 1, 1, 1, 1)
        .x2(8, 8, 0x40)
        .build()
}

#[test]
fn an_open_area_rolls_no_furniture_dice_at_all() {
    // `byte_1AD3E` false (no walls anywhere) AND `byte_1AD3D` zero — either
    // alone suppresses the roll, so a wall-free area's floor is draw-FREE.
    let tap = Tap::default();
    let mut rng = tap.rng(0x0C0F_FEE0);
    setup_dungeon_floor(&open_geo(), (8, 8), 1, &mut rng);
    assert_eq!(tap.count(), 0);
}

#[test]
fn the_x2_bit_alone_gates_every_die() {
    // Same room, x2 bit cleared: `byte_1AD3D == 0` short-circuits before the
    // first roll_dice, so not one die is spent.
    let no_bit = GeoBuilder::new().walls(8, 8, 1, 1, 1, 1).build();
    let tap = Tap::default();
    let mut rng = tap.rng(0x0C0F_FEE0);
    setup_dungeon_floor(&no_bit, (8, 8), 1, &mut rng);
    assert_eq!(tap.count(), 0, "no x2 0x40 bit → no dice");

    let tap = Tap::default();
    let mut rng = tap.rng(0x0C0F_FEE0);
    setup_dungeon_floor(&furnished_room_geo(), (8, 8), 1, &mut rng);
    assert!(tap.count() > 0, "with the bit set the dice fire");
}

#[test]
fn a_corridor_is_never_furnished() {
    // `byte_1AD3E`'s two corridor arms (`ovr011.cs:35-43`): N+S walls without
    // both E+W, or E+W without both N+S.
    for (n, e, s, w) in [(1, 0, 1, 0), (0, 1, 0, 1)] {
        let geo = GeoBuilder::new()
            .walls(8, 8, n, e, s, w)
            .x2(8, 8, 0x40)
            .build();
        let tap = Tap::default();
        let mut rng = tap.rng(7);
        setup_dungeon_floor(&geo, (8, 8), 1, &mut rng);
        assert_eq!(tap.count(), 0, "corridor {n}{e}{s}{w} rolls nothing");
    }
}

#[test]
fn a_doorway_is_never_furnished() {
    // `ovr011.cs:44-48`: any dir flag of 3 (a door) suppresses furniture.
    let geo = GeoBuilder::new()
        .walls(8, 8, 1, 1, 1, 1)
        .doors(8, 8, 1, 0, 0, 0)
        .x2(8, 8, 0x40)
        .build();
    let tap = Tap::default();
    let mut rng = tap.rng(7);
    setup_dungeon_floor(&geo, (8, 8), 1, &mut rng);
    assert_eq!(tap.count(), 0);
}

#[test]
fn every_furniture_die_is_a_d10() {
    // `roll_dice(10, 1)` both times (`ovr011.cs:68` and `:82`) — the operand a
    // capture records. A future live-fight trace matches on exactly this.
    let tap = Tap::default();
    let mut rng = tap.rng(0x0C0F_FEE0);
    setup_dungeon_floor(&furnished_room_geo(), (8, 8), 1, &mut rng);
    assert!(tap.count() >= 3, "the eligible candidates rolled");
    assert!(
        tap.operands().iter().all(|&n| n == Some(10)),
        "every floor die is a d10: {:?}",
        tap.operands()
    );
}

#[test]
fn only_the_candidate_cells_this_square_has_already_painted_are_eligible() {
    // A real consequence of the `dy`-outer/`dx`-inner iteration order, worth
    // naming because it halves the dice a room spends: the six candidates sit
    // at x offsets 4..=7, but a source square only paints x 0..=5 on rows
    // 2..=4 (`build_tiles_1`). Offsets 6 and 7 belong to the NEXT `dx`, which
    // has not run yet — so they are still zero-fill when the furniture test
    // reads them, fail the `tile_index == 0x16` gate, and roll nothing.
    //
    // Three candidates therefore survive in a four-walled room: (4,2), (5,2),
    // (5,3) — the cells at combat (25,12), (26,12), (26,13).
    let tap = Tap::default();
    let mut rng = tap.rng(1);
    setup_dungeon_floor(&furnished_room_geo(), (8, 8), 1, &mut rng);
    assert_eq!(
        tap.count(),
        3,
        "three table rolls, no chair rolls (all failed)"
    );
}

#[test]
fn the_furniture_draw_sequence_is_pinned_for_a_fixed_seed() {
    // ★ The §D-CV8 pin-shape, ready for the first live-fight capture: a table
    // roll <= 5 is followed by up to four neighbour rolls (in
    // `dir_{x,y}_offset` order), a failed one goes straight to the next
    // candidate. Any reordering of the two loops, or of the `&&` chain's
    // short-circuits, moves these numbers.
    //
    // Recorded from this transliteration, NOT from a capture (the honesty
    // rule): these are a regression pin, not evidence.
    /// `(seed, draws spent, furnished cells)`.
    type Pin = (u32, usize, &'static [(i32, i32, u8)]);
    let expected: &[Pin] = &[
        (1, 3, &[]),
        (7, 5, &[(26, 12, TILE_CHAIR)]),
        (
            0x0C0F_FEE0,
            5,
            &[(25, 12, TILE_CHAIR), (26, 12, TILE_CHAIR)],
        ),
        (
            0xDEAD_BEEF,
            5,
            &[(26, 12, TILE_CHAIR), (26, 13, TILE_CHAIR)],
        ),
    ];
    for &(seed, draws, furniture) in expected {
        let tap = Tap::default();
        let mut rng = tap.rng(seed);
        let map = setup_dungeon_floor(&furnished_room_geo(), (8, 8), 1, &mut rng);
        assert_eq!(tap.count(), draws, "seed {seed:#x}: draw count");
        let got: Vec<(i32, i32, u8)> = (0..MAP_H)
            .flat_map(|y| (0..MAP_W).map(move |x| (x, y)))
            .filter(|&(x, y)| matches!(cell(&map, x, y), TILE_TABLE | TILE_CHAIR))
            .map(|(x, y)| (x, y, cell(&map, x, y)))
            .collect();
        assert_eq!(got, furniture, "seed {seed:#x}: furnished cells");
    }
}

#[test]
fn furniture_lands_only_on_the_six_candidate_cells() {
    // `posX = dx*6 + dy*5 + 0x15 + var_1 + var_2`, `posY = dy*5 + 0x0A + var_2`
    // for var_1 in 2..=3, var_2 in 2..=4 — i.e. x offsets 4..=7, y offsets 2..=4
    // relative to the party square's origin, and never the same cell twice for
    // the same (var_1, var_2).
    let mut furnished = Vec::new();
    for seed in 0..40u32 {
        let mut rng = EngineRng::new(seed.wrapping_mul(2_654_435_761).wrapping_add(1));
        let map = setup_dungeon_floor(&furnished_room_geo(), (8, 8), 1, &mut rng);
        for y in 0..MAP_H {
            for x in 0..MAP_W {
                if matches!(cell(&map, x, y), TILE_TABLE | TILE_CHAIR) {
                    furnished.push((x, y));
                }
            }
        }
    }
    assert!(!furnished.is_empty(), "40 seeds produced some furniture");
    let o = party_square_origin();
    for (x, y) in furnished {
        let (ox, oy) = (x - o.x, y - o.y);
        assert!(
            (4..=7).contains(&ox) && (2..=4).contains(&oy),
            "furniture at offset ({ox},{oy}) is outside the candidate window"
        );
    }
}

#[test]
fn a_furnished_cell_is_a_table_or_a_chair_and_stays_walkable() {
    // `BackGroundTiles[0x1A]` is move-cost 2, `[0x1B]` cost 1 — furniture
    // slows a fighter down, it does not wall the room off.
    assert_eq!(
        tile_passability(TILE_TABLE),
        TilePassability::Passable { move_cost: 2 }
    );
    assert_eq!(
        tile_passability(TILE_CHAIR),
        TilePassability::Passable { move_cost: 1 }
    );
}

#[test]
fn the_same_seed_and_area_always_lay_the_same_floor() {
    // Determinism (D9): the floor is a pure function of (geo, party, seed).
    let geo = furnished_room_geo();
    let mut a = EngineRng::new(0x1234_5678);
    let mut b = EngineRng::new(0x1234_5678);
    assert_eq!(
        setup_dungeon_floor(&geo, (8, 8), 1, &mut a),
        setup_dungeon_floor(&geo, (8, 8), 1, &mut b)
    );
}

// --- the fork ------------------------------------------------------------

#[test]
fn a_wilderness_fight_refuses_loudly_rather_than_laying_a_dungeon_floor() {
    let geo = open_geo();
    let mut rng = EngineRng::new(1);
    assert_eq!(
        setup_ground_tiles(&geo, (8, 8), 1, false, &mut rng).unwrap_err(),
        FloorError::WildernessFloorDeferred
    );
    assert!(setup_ground_tiles(&geo, (8, 8), 1, true, &mut rng).is_ok());
}

#[test]
fn the_placement_hook_reports_the_same_flags_the_floor_reads() {
    // One `get_dir_flags`, two consumers (the floor and `place_combatants`) —
    // the property that keeps combatants out of the walls the floor draws.
    let geo = GeoBuilder::new().walls(8, 7, 0, 0, 1, 0).build();
    let hook = dir_flags_hook(&geo, 8);
    // The hook's argument order is coab's call shape: (dir, mapY, mapX).
    assert_eq!(hook(0, 8, 8), dir_flags(&geo, 8, 0, 8, 8) as i32);
    assert_eq!(hook(0, 8, 8), 1);
    assert_eq!(hook(2, 8, 8), 0);
}
