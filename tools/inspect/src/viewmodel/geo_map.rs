//! GEO automap geometry: flattens a parsed `GeoBlock` into cell fills and
//! wall-edge segments a painter can draw directly — graphical, not the
//! CLI's ASCII `restrike map` output (task brief deliverable 2). Pure over
//! `gbx-formats::geo` types, no rendering.

use gbx_formats::geo::{GeoBlock, GEO_GRID_SIZE};

/// Which edge of a cell a [`WallEdge`] sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    North,
    East,
    South,
    West,
}

/// One drawable wall edge: cell `(x, y)`'s `side`, its raw wall-type nibble
/// (`0` would mean "no wall" — callers only ever get a [`WallEdge`] for a
/// nonzero type, see [`build_geometry`]) and door-state field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallEdge {
    pub x: usize,
    pub y: usize,
    pub side: Side,
    pub wall_type: u8,
    pub door: u8,
}

/// One drawable cell fill: `(x, y)` plus the flags an automap colors by
/// (`indoor`/`floor_flag`/`low7`, per `gbx-formats::geo::Square`'s own
/// docket on what `low7` might mean — surfaced here, not interpreted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellFill {
    pub x: usize,
    pub y: usize,
    pub indoor: bool,
    pub floor_flag: bool,
    pub low7: u8,
}

/// The full drawable geometry of a [`GeoBlock`]: every cell's fill data, and
/// one [`WallEdge`] per nonzero wall-type nibble (open edges are omitted —
/// nothing to draw there).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoGeometry {
    pub grid_size: usize,
    pub cells: Vec<CellFill>,
    pub edges: Vec<WallEdge>,
}

/// Builds the drawable geometry for `geo`: `GEO_GRID_SIZE * GEO_GRID_SIZE`
/// cells (always present, in `(y, x)` row-major order to match the on-disk
/// plane layout `geo.rs` documents) plus every nonzero wall edge, at most 4
/// per cell.
pub fn build_geometry(geo: &GeoBlock) -> GeoGeometry {
    let mut cells = Vec::with_capacity(GEO_GRID_SIZE * GEO_GRID_SIZE);
    let mut edges = Vec::new();

    for y in 0..GEO_GRID_SIZE {
        for x in 0..GEO_GRID_SIZE {
            let sq = geo.square(x, y);
            cells.push(CellFill {
                x,
                y,
                indoor: sq.indoor,
                floor_flag: sq.floor_flag,
                low7: sq.low7,
            });

            let sides = [
                (Side::North, sq.wall_north, sq.door_north),
                (Side::East, sq.wall_east, sq.door_east),
                (Side::South, sq.wall_south, sq.door_south),
                (Side::West, sq.wall_west, sq.door_west),
            ];
            for (side, wall_type, door) in sides {
                if wall_type != 0 {
                    edges.push(WallEdge {
                        x,
                        y,
                        side,
                        wall_type,
                        door,
                    });
                }
            }
        }
    }

    GeoGeometry {
        grid_size: GEO_GRID_SIZE,
        cells,
        edges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::geo::GEO_BLOCK_SIZE;

    fn open_geo() -> GeoBlock {
        GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap()
    }

    #[test]
    fn every_cell_is_present_even_when_fully_open() {
        let geo = open_geo();
        let geometry = build_geometry(&geo);
        assert_eq!(geometry.cells.len(), GEO_GRID_SIZE * GEO_GRID_SIZE);
        assert!(geometry.edges.is_empty(), "an all-open map has no edges");
    }

    #[test]
    fn cells_are_ordered_row_major_y_then_x() {
        let geo = open_geo();
        let geometry = build_geometry(&geo);
        assert_eq!(
            geometry.cells[0],
            CellFill {
                x: 0,
                y: 0,
                indoor: false,
                floor_flag: false,
                low7: 0
            }
        );
        assert_eq!(geometry.cells[1].x, 1);
        assert_eq!(geometry.cells[1].y, 0);
        assert_eq!(geometry.cells[GEO_GRID_SIZE].x, 0);
        assert_eq!(geometry.cells[GEO_GRID_SIZE].y, 1);
    }

    fn geo_with_wall(x: usize, y: usize, nibble_byte: usize, hi: bool, val: u8) -> GeoBlock {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        let idx = 2 + nibble_byte * 256 + x + 16 * y;
        data[idx] |= if hi { val << 4 } else { val };
        GeoBlock::parse(&data).unwrap()
    }

    #[test]
    fn a_single_north_wall_produces_exactly_one_edge() {
        let geo = geo_with_wall(3, 4, 0, true, 5); // plane 0 hi nibble = North
        let geometry = build_geometry(&geo);
        assert_eq!(geometry.edges.len(), 1);
        let edge = geometry.edges[0];
        assert_eq!((edge.x, edge.y), (3, 4));
        assert_eq!(edge.side, Side::North);
        assert_eq!(edge.wall_type, 5);
    }

    #[test]
    fn door_state_is_carried_onto_the_edge() {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        let idx = 2 + 5 + 16 * 6;
        data[idx] = 3 << 4; // North wall type 3
        data[idx + 3 * 256] = 0b10; // door_north = 2 (locked)
        let geo = GeoBlock::parse(&data).unwrap();
        let geometry = build_geometry(&geo);
        let edge = geometry
            .edges
            .iter()
            .find(|e| e.x == 5 && e.y == 6 && e.side == Side::North)
            .unwrap();
        assert_eq!(edge.door, 2);
    }

    #[test]
    fn all_four_sides_of_one_cell_produce_four_edges() {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        let idx_ne = 2 + 8 + 16 * 8;
        let idx_sw = idx_ne + 256;
        data[idx_ne] = (1 << 4) | 2; // North=1, East=2
        data[idx_sw] = (3 << 4) | 4; // South=3, West=4
        let geo = GeoBlock::parse(&data).unwrap();
        let geometry = build_geometry(&geo);
        let edges: Vec<_> = geometry
            .edges
            .iter()
            .filter(|e| e.x == 8 && e.y == 8)
            .collect();
        assert_eq!(edges.len(), 4);
    }
}
