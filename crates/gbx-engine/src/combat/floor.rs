//! **`SetupDungeonFloor`** (`ovr011.cs:500-522`) — the faithful combat floor,
//! projected from the area's GEO wall topology (`docs/design/combat-visualizer.md`
//! D-CV6 item 1 / §8.4).
//!
//! This replaces [`provisional_combat_map`](super::provisional_combat_map) on the
//! live-fight path. Where the provisional derivation stamped fully-enclosed GEO
//! squares as rock onto an open field, this is the original's own transform: for
//! each of a 13×5 band of source map squares around the party (dx −6..=6,
//! dy −2..=2) it samples the four directional wall flags and stamps a sheared
//! 8×5 patch of ground-tile indices into the 50×25 combat grid.
//!
//! ## The shear
//!
//! `set_background_tile(tileId, y, x)` (`sub_37046`, `ovr011.cs:11-23`) writes
//! `tileId + 1` — note the `+1`, so a `22` lays ground tile `0x17`, the cost-1
//! floor — at
//!
//! ```text
//! combatX = dx*6 + dy*5 + 21 + x        (clipped to 0..=0x31)
//! combatY = dy*5 + 10 + y               (clipped to 0..=0x18)
//! ```
//!
//! an oblique shear: E–W walls come out as horizontal runs of tiles 5/10
//! (`build_background_tiles_2`), N–S walls as diagonal runs of 4/3/13 stepping
//! `(+1,+1)` per row (`build_background_tiles_1`), and the two corner quads
//! (`build_backgrould_tiles_3`/`_4`) resolve doors and junctions from lookup
//! ladders. The grid starts **all zero** (`gbl.mapToBackGroundTile = new
//! Struct_1D1BC()`, `ovr011.cs:768`), so any cell the band never reaches stays
//! void — impassable, and no combatant places there.
//!
//! ## Draw-bearing
//!
//! `sub_370D3` (`ovr011.cs:28-93`) rolls **`roll_dice(10,1) <= 5`** to drop a
//! table into a "building" cell and, per adjacent floor cell, **`roll_dice(10,1)
//! <= 9`** to turn it into a chair. Those dice come off the same PRNG the fight
//! draws from, and `SetupGroundTiles` runs inside `BattleSetup` **before**
//! `CalculateInitiative` (`ovr011.cs:1197` vs `ovr009.cs:37`) — so a live fight's
//! draw stream now opens with floor dice and only then the §2 initiative
//! fingerprint. Both the count and the ORDER of those dice are observable, which
//! is why [`furniture_dice`] is transcribed statement-for-statement including its
//! short-circuit order.
//!
//! ## Honesty rule (D-CV6)
//!
//! Everything here is **cited, not capture-proven**: no `.gbxtrace` in
//! `~/goldbox-data/traces` was staged on a floor this code generated (every
//! closed capture carries its terrain in `combat_entry`, so replays never call
//! this). The first live-fight capture converts it. Until then the unit tests
//! below pin the transform against the cited tables — they prove the
//! transliteration, not the original's behaviour.
//!
//! Wilderness floors (`SetupWildernessFloor`, `ovr011.cs:551-754`) stay deferred
//! with the wilderness circle-back item; [`setup_ground_tiles`] loud-fails rather
//! than silently laying a dungeon floor outdoors.

use super::{roll_dice, CombatMap, GridPos, BACKGROUND_TILE_INDEX, MAP_H, MAP_W};
use crate::rng::EngineRng;
use gbx_formats::geo::{GeoBlock, Square, GEO_GRID_SIZE};

/// `gbl.Tile_Table` (`Gbl.cs:676`) — `BackGroundTiles[0x1A]`, atlas slot 0x22.
pub const TILE_TABLE: u8 = 0x1A;
/// `gbl.Tile_Chair` (`Gbl.cs:677`) — `BackGroundTiles[0x1B]`, atlas slot 0x23.
pub const TILE_CHAIR: u8 = 0x1B;

/// The `tile_index` a plain dungeon floor cell resolves to — the value
/// [`furniture_dice`] tests a cell against before rolling (`ovr011.cs:65`).
/// `BACKGROUND_TILE_INDEX[0x17] == 0x16`, and `set_background_tile(22, …)`
/// writes exactly `0x17`.
const FLOOR_TILE_INDEX: u8 = 0x16;

/// `gbl.MapDirectionXDelta` / `MapDirectionYDelta` (`Gbl.cs:691-692`), the
/// 0..=8 direction tables — only the cardinals (0/2/4/6) are used here.
const MAP_DIR_X: [i32; 9] = [0, 1, 1, 1, 0, -1, -1, -1, 0];
const MAP_DIR_Y: [i32; 9] = [-1, -1, 0, 1, 1, 1, 0, -1, 0];

/// `dir_x_offset`/`dir_y_offset` (`ovr011.cs:24-25`, `unk_1665B`/`unk_1665F`) —
/// the four neighbour probes `sub_370D3`'s chair roll walks, in that order.
const FURNITURE_DIR_X: [i32; 4] = [0, 1, 0, -1];
const FURNITURE_DIR_Y: [i32; 4] = [-1, 0, 1, 0];

/// [`setup_ground_tiles`]'s one refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloorError {
    /// `SetupWildernessFloor` (`ovr011.cs:551-754`) scatters roads/rivers/grass
    /// with five different `roll_dice` calls, so a wrong transliteration would
    /// desync every subsequent draw rather than just look wrong. Deferred with
    /// the wilderness circle-back item (doc §7): loud-fail beats a guess (D11).
    WildernessFloorDeferred,
}

impl std::fmt::Display for FloorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FloorError::WildernessFloorDeferred => write!(
                f,
                "SetupWildernessFloor is deferred (combat-visualizer.md §7): a wilderness \
                 fight's floor draws from the PRNG and has no transliteration yet"
            ),
        }
    }
}

impl std::error::Error for FloorError {}

/// The mutable state one `SetupDungeonFloor` pass carries: the grid being
/// painted, the current source square's offsets (`gbl.byte_1AD34`/`byte_1AD35`),
/// the four directional flags, and `byte_1AD3D` (the x2 `0x40` bit).
struct FloorPass<'a> {
    ground: Vec<u8>,
    geo: &'a GeoBlock,
    /// `gbl.EclBlockId` — read only by the out-of-grid early-out in
    /// `get_wall_x2`/`MapCoordIsValid` (`ovr031.cs:289-293`).
    ecl_block_id: u8,
    /// `gbl.mapPosY` — the party's row, read by `sub_37306`'s off-grid arm.
    party_y: i32,
    dx: i32,
    dy: i32,
    dir_0: u8,
    dir_2: u8,
    dir_4: u8,
    dir_6: u8,
    /// `gbl.byte_1AD3D` — `get_wall_x2(mapY, mapX) & 0x40` for the current
    /// source square, the furniture gate's first condition.
    byte_1ad3d: u8,
}

/// `SetupGroundTiles` (`sub_38030`, `ovr011.cs:756-783`) — the fork the fight's
/// floor comes through. The art half (`Load24x24Set` of DUNGCOM/WILDCOM +
/// RANDCOM) is [`crate::combat_art::load_ground_tiles`]'s; this is the grid.
///
/// `party` is `(gbl.mapPosX, gbl.mapPosY)`; `ecl_block_id` is `gbl.EclBlockId`.
/// The PRNG is consumed by [`furniture_dice`] — see this module's draw-bearing
/// note.
pub fn setup_ground_tiles(
    geo: &GeoBlock,
    party: (i32, i32),
    ecl_block_id: u8,
    in_dungeon: bool,
    rng: &mut EngineRng,
) -> Result<CombatMap, FloorError> {
    if !in_dungeon {
        return Err(FloorError::WildernessFloorDeferred);
    }
    Ok(setup_dungeon_floor(geo, party, ecl_block_id, rng))
}

/// `SetupDungeonFloor` (`sub_378CD0`, `ovr011.cs:500-522`): the 13×5 band, in
/// the original's own iteration order (`dy` outer, `dx` inner) — load-bearing,
/// because the furniture dice fire inside it.
pub fn setup_dungeon_floor(
    geo: &GeoBlock,
    party: (i32, i32),
    ecl_block_id: u8,
    rng: &mut EngineRng,
) -> CombatMap {
    let (px, py) = party;
    let mut pass = FloorPass {
        // `new Struct_1D1BC()` — a zeroed grid; unpainted cells stay void.
        ground: vec![0u8; (MAP_W * MAP_H) as usize],
        geo,
        ecl_block_id,
        party_y: py,
        dx: 0,
        dy: 0,
        dir_0: 0,
        dir_2: 0,
        dir_4: 0,
        dir_6: 0,
        byte_1ad3d: 0,
    };

    for dy in -2..=2 {
        for dx in -6..=6 {
            pass.dx = dx;
            pass.dy = dy;
            let map_x = px + dx;
            let map_y = py + dy;

            pass.dir_0 = pass.dir_flags(0, map_x, map_y);
            pass.dir_6 = pass.dir_flags(6, map_x, map_y);
            pass.dir_2 = pass.dir_flags(2, map_x, map_y);
            pass.dir_4 = pass.dir_flags(4, map_x, map_y);

            pass.build_tiles_1();
            pass.build_tiles_2();
            pass.build_tiles_3(map_x, map_y);
            pass.build_tiles_4(map_x, map_y);
            pass.byte_1ad3d = pass.wall_x2(map_x, map_y) & 0x40;
            pass.furniture_dice(rng);
        }
    }

    CombatMap::from_ground(pass.ground)
}

// --- GEO sampling (`ovr031.cs` / `ovr011.cs`) -----------------------------
//
// Free functions rather than `FloorPass` methods because `place_combatants`
// takes the very same `get_dir_flags` as its (previously stubbed) area-wall
// hook — see [`dir_flags_hook`].

/// `WallDoorFlagsGet` (`ovr031.cs:181-219`) reduced to its in-grid arm (the only
/// one [`wall_flags_at`] reaches): `1` when the direction carries no wall at
/// all, else that direction's raw door 2-bit field.
fn wall_door_flags_get(sq: &Square, dir: u8) -> u8 {
    if wall_type(sq, dir) != 0 {
        door_field(sq, dir)
    } else {
        1
    }
}

/// `sub_37306` (`ovr011.cs:94-127`) — one square's flag for one direction:
/// `0` open, `1` wall, `3` door.
///
/// The off-grid arm is the original's own quirk: a square outside the 16×16 grid
/// reads as **open** when it shares the party's row and the probe is E/W, and as
/// a wall otherwise — which is what keeps a fight staged at the map edge from
/// being walled in along its own corridor.
fn wall_flags_at(geo: &GeoBlock, party_y: i32, dir: u8, x: i32, y: i32) -> u8 {
    let grid = GEO_GRID_SIZE as i32;
    if (0..grid).contains(&x) && (0..grid).contains(&y) {
        let sq = geo.square(x as usize, y as usize);
        if wall_door_flags_get(sq, dir) == 0 {
            1
        } else if wall_type(sq, dir) == 0 {
            0
        } else {
            3
        }
    } else if y == party_y && (dir == 2 || dir == 6) {
        0
    } else {
        1
    }
}

/// `get_dir_flags` (`sub_37388`, `ovr011.cs:128-141`): a wall is shared by two
/// squares, so the flag is `this square's | the neighbour's, seen from the
/// opposite side` — `0|0 = 0`, `1|1 = 1`, `3|anything = 3`.
///
/// (coab's parameter names read `mapX`/`mapY` but every call site passes
/// `(y, x)`; the deltas below are the un-swapped form.)
pub fn dir_flags(geo: &GeoBlock, party_y: i32, dir: u8, x: i32, y: i32) -> u8 {
    let opposite = (dir + 4) % 8;
    let nx = x + MAP_DIR_X[dir as usize];
    let ny = y + MAP_DIR_Y[dir as usize];
    wall_flags_at(geo, party_y, dir, x, y) | wall_flags_at(geo, party_y, opposite, nx, ny)
}

/// The same `get_dir_flags` in the shape [`place_combatants`] wants — coab calls
/// it `get_dir_flags(dir, mapY, mapX)` from both retry branches
/// (`ovr011.cs:979-1034`), which is why the closure's second argument is the row.
///
/// Placement has always defaulted this to open ground ("the real area-derived
/// flags land with the encounter-trigger wiring slice"). It lands here: leaving
/// the fan-out blind to the area's walls while the *floor* is projected from
/// them would put combatants through walls the floor draws.
///
/// [`place_combatants`]: super::place_combatants
pub fn dir_flags_hook<'a>(geo: &'a GeoBlock, party_y: i32) -> impl Fn(i32, i32, i32) -> i32 + 'a {
    move |dir, map_y, map_x| dir_flags(geo, party_y, dir as u8, map_x, map_y) as i32
}

impl FloorPass<'_> {
    fn dir_flags(&self, dir: u8, x: i32, y: i32) -> u8 {
        dir_flags(self.geo, self.party_y, dir, x, y)
    }

    /// `get_wall_x2` (`ovr031.cs:289-318`) — the per-square `x2` byte, read with
    /// the original's single-step coordinate wrap (`> 0x0F → 0`, `< 0 → 0x0F`).
    ///
    /// The early `return 0` needs `EclBlockId` 0 or 10 (the two wrap-exempt
    /// blocks) *and* a coordinate `MapCoordIsValid` rejects. coab's
    /// `MapCoordIsValid` (`ovr031.cs:175-178`) tests `mapX >= 0` twice and never
    /// `mapY >= 0` — transcribed as written, and inert for every other block id
    /// (the `&&` short-circuits), which is where CotAB's live fights happen.
    fn wall_x2(&self, x: i32, y: i32) -> u8 {
        let grid = GEO_GRID_SIZE as i32;
        let coord_valid = x < grid && x >= 0 && y < grid && x >= 0;
        if !coord_valid && (self.ecl_block_id == 0 || self.ecl_block_id == 10) {
            return 0;
        }
        let wrap = |v: i32| {
            if v > 0x0F {
                0
            } else if v < 0 {
                0x0F
            } else {
                v
            }
        };
        let sq = self.geo.square(wrap(x) as usize, wrap(y) as usize);
        let mut x2 = sq.low7;
        if sq.indoor {
            x2 |= 0x80;
        }
        x2
    }

    // --- the shear (`ovr011.cs:11-23`) ------------------------------------

    /// `set_background_tile(tileId, y, x)` (`sub_37046`) — the `+1` is the
    /// original's: the tile *ids* the build functions name are one less than the
    /// `BackGroundTiles` row they select.
    fn set_tile(&mut self, tile_id: u8, y: i32, x: i32) {
        let cx = self.dx * 6 + self.dy * 5 + 21 + x;
        let cy = self.dy * 5 + 10 + y;
        if (0..MAP_W).contains(&cx) && (0..MAP_H).contains(&cy) {
            self.ground[(cy * MAP_W + cx) as usize] = tile_id + 1;
        }
    }

    fn tile_at(&self, cx: i32, cy: i32) -> u8 {
        self.ground[(cy * MAP_W + cx) as usize]
    }

    // --- the four tile builders -------------------------------------------

    /// `build_background_tiles_1` (`sub_373FC`, `ovr011.cs:143-166`): lay the
    /// square's floor, then the WEST wall as a **diagonal** run of 4/3/13
    /// stepping `(+1,+1)` per row — the shear's signature. A door west
    /// (`dir_6 == 3`) instead drops the two jamb tiles 8 and 0.
    fn build_tiles_1(&mut self) {
        for y in 2..=4 {
            for x in 0..=5 {
                self.set_tile(22, y, x);
            }
        }
        if self.dir_6 == 1 {
            for v in 2..=4 {
                self.set_tile(4, v, v - 1);
                self.set_tile(3, v, v);
                self.set_tile(13, v, v + 1);
            }
        } else if self.dir_6 == 3 {
            self.set_tile(8, 2, 1);
            self.set_tile(0, 4, 5);
        }
    }

    /// `build_background_tiles_2` (`sub_374A1`, `ovr011.cs:169-192`): the NORTH
    /// wall as a **horizontal** pair of runs (5 over 10), or plain floor.
    fn build_tiles_2(&mut self) {
        if self.dir_0 == 1 {
            self.set_tile(5, 0, 3);
            self.set_tile(5, 0, 4);
            self.set_tile(10, 1, 3);
            self.set_tile(10, 1, 4);
        } else {
            self.set_tile(22, 0, 3);
            self.set_tile(22, 0, 4);
            self.set_tile(22, 1, 3);
            self.set_tile(22, 1, 4);
        }
    }

    /// `build_backgrould_tiles_3` (`sub_3751E`, `ovr011.cs:195-350`) — the
    /// NORTH-WEST corner quad. `var_6` asks whether the *outside* of the corner
    /// is open on both sides, which picks between the "inner" and "outer" art
    /// for every junction. coab's `for var_1 = 1..4` loop is a switch with one
    /// arm per output cell; it is written out here.
    fn build_tiles_3(&mut self, map_x: i32, map_y: i32) {
        let var_6 =
            self.dir_flags(6, map_x, map_y - 1) == 0 && self.dir_flags(0, map_x - 1, map_y) == 0;
        let (d0, d6) = (self.dir_0, self.dir_6);

        let mut var_2 = 0u8;
        if d0 == 0 {
            match d6 {
                0 => var_2 = 0x16,
                3 => var_2 = 0x0D,
                1 => var_2 = if var_6 { 0 } else { 0x0D },
                _ => {}
            }
        } else if d0 == 3 || d0 == 1 {
            if d6 == 0 {
                var_2 = if var_6 { 0x0F } else { 5 };
            } else {
                var_2 = if var_6 { 0x12 } else { 2 };
            }
        }

        let mut var_4 = 0u8;
        if d0 == 0 {
            var_4 = 0x16;
        } else if d0 == 3 {
            var_4 = 0x11;
        } else if d0 == 1 {
            var_4 = 5;
        }

        let mut var_3 = 0u8;
        match d6 {
            0 => {
                if d0 == 0 {
                    var_3 = 0x16;
                } else if var_6 {
                    var_3 = 0x10;
                } else {
                    var_3 = 0x0A;
                }
            }
            3 => var_3 = if var_6 { 0x14 } else { 7 },
            1 => var_3 = if var_6 { 1 } else { 3 },
            _ => {}
        }

        let mut var_5 = 0u8;
        if d6 == 0 || d6 == 3 {
            if d0 == 0 {
                var_5 = 0x16;
            } else if d0 == 3 {
                var_5 = 0x17;
            } else if d0 == 1 {
                var_5 = 0x0A;
            }
        } else if d6 == 1 {
            if d0 == 0 {
                var_5 = 0x0D;
            } else if d0 == 3 {
                var_5 = 0x15;
            } else if d0 == 1 {
                var_5 = 6;
            }
        }

        self.set_tile(var_2, 0, 1);
        self.set_tile(var_4, 0, 2);
        self.set_tile(var_3, 1, 1);
        self.set_tile(var_5, 1, 2);
    }

    /// `build_background_tiles_4` (`sub_376F6`, `ovr011.cs:353-497`) — the
    /// NORTH-EAST corner quad, the same shape with the east wall's flags
    /// (`var_7` north-of-east, `var_8` east-of-north) in play.
    fn build_tiles_4(&mut self, map_x: i32, map_y: i32) {
        let var_7 = self.dir_flags(2, map_x, map_y - 1);
        let var_8 = self.dir_flags(0, map_x + 1, map_y);
        let var_6 = var_7 == 0 && var_8 == 0;
        let (d0, d2) = (self.dir_0, self.dir_2);

        let var_2: u8 = if d0 == 0 {
            if var_7 == 1 {
                4 // bottom of a north-south wall
            } else {
                0x16 // bottom west edge of a west-east wall
            }
        } else if d0 == 3 {
            0x0F
        } else {
            5
        };

        let var_4: u8 = if d0 == 0 {
            if var_7 == 0 {
                0x16
            } else if var_7 == 3 {
                if d2 == 0 && var_8 != 0 {
                    0x18
                } else {
                    1
                }
            } else if d2 == 0 {
                if var_8 != 0 {
                    0x0B
                } else {
                    7
                }
            } else {
                3
            }
        } else if d2 != 0 {
            9
        } else if var_8 != 0 {
            5
        } else if var_6 {
            0x11
        } else {
            0x13
        };

        let var_3: u8 = if d0 == 0 {
            0x16
        } else if d0 == 3 {
            0x10
        } else {
            0x0A
        };

        let var_5: u8 = if d0 == 0 {
            if var_7 == 0 {
                0x16
            } else if d2 != 0 {
                4
            } else if var_8 == 0 {
                8
            } else {
                0x0C
            }
        } else if d2 != 0 {
            0x0E
        } else if var_8 == 0 {
            0x17
        } else {
            0x0A
        };

        self.set_tile(var_2, 0, 5);
        self.set_tile(var_4, 0, 6);
        self.set_tile(var_3, 1, 5);
        self.set_tile(var_5, 1, 6);
    }

    // --- the dice (`sub_370D3`, `ovr011.cs:27-92`) -------------------------

    /// `byte_1AD3E` — "this square looks like a room, not a corridor or a
    /// doorway": at least one wall, but not a plain N/S or E/W corridor, and no
    /// door on any side.
    fn looks_like_a_room(&self) -> bool {
        let (d0, d2, d4, d6) = (self.dir_0, self.dir_2, self.dir_4, self.dir_6);
        // coab's four `byte_1AD3E = false` arms in order (`ovr011.cs:30-53`):
        // no wall at all; a plain N/S corridor; a plain E/W corridor; a door on
        // any side. Written as one `||` chain — the arms are separate `if`s in
        // coab only because it assigns a variable.
        let not_a_room = (d0 != 1 && d2 != 1 && d4 != 1 && d6 != 1)
            || (d0 == 1 && d4 == 1 && (d2 != 1 || d6 != 1))
            || (d2 == 1 && d6 == 1 && (d0 != 1 || d4 != 1))
            || (d0 == 3 || d2 == 3 || d4 == 3 || d6 == 3);
        !not_a_room
    }

    /// ★ **The draw-bearing beat** (`sub_370D3`): six candidate cells per source
    /// square (`var_1` 2..=3 outer, `var_2` 2..=4 inner), each rolling
    /// `roll_dice(10,1) <= 5` for a table and then, per open neighbour in
    /// `dir_{x,y}_offset` order, `roll_dice(10,1) <= 9` to make it a chair.
    ///
    /// Two quirks are the original's and are kept: the chair roll **writes the
    /// candidate cell, not the neighbour** (`ovr011.cs:83`), so a table can be
    /// overwritten by a chair up to four times; and each neighbour's floor test
    /// short-circuits ahead of its roll, so the number of dice a square spends
    /// depends on its surroundings. Both are draw-observable — the future
    /// live-fight capture pins them.
    fn furniture_dice(&mut self, rng: &mut EngineRng) {
        let room = self.looks_like_a_room();
        for var_1 in 2..=3 {
            for var_2 in 2..=4 {
                let pos_x = self.dx * 6 + self.dy * 5 + 0x15 + var_1 + var_2;
                let pos_y = self.dy * 5 + 0x0A + var_2;
                if !((0..MAP_W).contains(&pos_x) && (0..MAP_H).contains(&pos_y)) {
                    continue;
                }
                if !(background_tile_index(self.tile_at(pos_x, pos_y)) == FLOOR_TILE_INDEX
                    && self.byte_1ad3d != 0
                    && room
                    && roll_dice(rng, 10, 1) <= 5)
                {
                    continue;
                }
                self.ground[(pos_y * MAP_W + pos_x) as usize] = TILE_TABLE;
                for k in 0..4 {
                    let nx = FURNITURE_DIR_X[k] + pos_x;
                    let ny = FURNITURE_DIR_Y[k] + pos_y;
                    if !((0..MAP_W).contains(&nx) && (0..MAP_H).contains(&ny)) {
                        continue;
                    }
                    if background_tile_index(self.tile_at(nx, ny)) == FLOOR_TILE_INDEX
                        && roll_dice(rng, 10, 1) <= 9
                    {
                        // Writes the CANDIDATE cell (coab `ovr011.cs:83`).
                        self.ground[(pos_y * MAP_W + pos_x) as usize] = TILE_CHAIR;
                    }
                }
            }
        }
    }
}

/// `BackGroundTiles[ground].tile_index`, guarded — a ground value past the
/// 74-row table can never be produced here, but the furniture test indexes it
/// unguarded in the original and a panic would be a poor way to learn that.
fn background_tile_index(ground: u8) -> u8 {
    BACKGROUND_TILE_INDEX
        .get(ground as usize)
        .copied()
        .unwrap_or(0)
}

fn wall_type(sq: &Square, dir: u8) -> u8 {
    match dir {
        0 => sq.wall_north,
        2 => sq.wall_east,
        4 => sq.wall_south,
        6 => sq.wall_west,
        _ => 0,
    }
}

fn door_field(sq: &Square, dir: u8) -> u8 {
    match dir {
        0 => sq.door_north,
        2 => sq.door_east,
        4 => sq.door_south,
        6 => sq.door_west,
        _ => 0,
    }
}

/// Where the party's own square lands on the combat grid — `dx = dy = 0`, so the
/// square's floor patch spans `x ∈ 21..=26`, `y ∈ 10..=14` and its centre is
/// where `place_combatants` fans the roster out from.
pub fn party_square_origin() -> GridPos {
    GridPos::new(21, 10)
}

#[cfg(test)]
mod tests;
