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

/// Rasterizes `geometry` into an RGBA pixel buffer, `cell_size` pixels per
/// grid cell — the GEO automap's image copy/save target (task brief
/// deliverable 3). Colors mirror the resource browser's live painter-drawn
/// automap exactly (`panes/resource_browser.rs`'s `show_geo`): cell fill
/// gray-60/gray-30 (indoor/outdoor), a yellow square for a nonzero `low7`,
/// wall-edge color by door state (gray=solid, green=open, red=locked,
/// orange=unpickable). A from-scratch software rasterizer rather than a
/// screen capture: every edge is axis-aligned by construction ([`Side`]),
/// so each wall is just a thick filled rectangle along the cell boundary —
/// no line-drawing algorithm needed, and no antialiasing (the export target
/// is a debug-tool screenshot substitute, not a rendering). Returns
/// `(width_px, height_px, rgba)`.
pub fn rasterize(geometry: &GeoGeometry, cell_size: usize) -> (usize, usize, Vec<u8>) {
    let cell_size = cell_size.max(1);
    let width = geometry.grid_size * cell_size;
    let height = geometry.grid_size * cell_size;
    let mut buf = vec![0u8; width * height * 4];
    for px in buf.chunks_exact_mut(4) {
        px[3] = 255;
    }
    let mut canvas = Canvas {
        buf: &mut buf,
        width,
        height,
    };

    for cell in &geometry.cells {
        let fill = if cell.indoor {
            [60, 60, 60, 255]
        } else {
            [30, 30, 30, 255]
        };
        canvas.fill_rect(
            cell.x * cell_size,
            cell.y * cell_size,
            cell_size,
            cell_size,
            fill,
        );
        if cell.low7 != 0 {
            let r = (cell_size / 8).max(1);
            let cx = cell.x * cell_size + cell_size / 2;
            let cy = cell.y * cell_size + cell_size / 2;
            canvas.fill_rect(
                cx.saturating_sub(r),
                cy.saturating_sub(r),
                r * 2,
                r * 2,
                [255, 255, 0, 255],
            );
        }
    }

    let thickness = (cell_size / 10).max(1);
    for edge in &geometry.edges {
        let color = match edge.door {
            0 => [160, 160, 160, 255], // solid wall, no door
            1 => [0, 255, 0, 255],     // open/unlocked door
            2 => [255, 0, 0, 255],     // locked door
            _ => [255, 140, 0, 255],   // unpickable
        };
        canvas.draw_edge(edge, cell_size, thickness, color);
    }

    (width, height, buf)
}

/// A tiny RGBA pixel-buffer writer: bundles the buffer with its dimensions
/// so `rasterize`'s draw calls don't need to thread `width`/`height`
/// through every call (clippy's `too_many_arguments` threshold otherwise).
struct Canvas<'a> {
    buf: &'a mut [u8],
    width: usize,
    height: usize,
}

impl Canvas<'_> {
    fn set_pixel(&mut self, x: usize, y: usize, color: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = (y * self.width + x) * 4;
        self.buf[i..i + 4].copy_from_slice(&color);
    }

    fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: [u8; 4]) {
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.set_pixel(xx, yy, color);
            }
        }
    }

    fn draw_edge(&mut self, edge: &WallEdge, cell_size: usize, thickness: usize, color: [u8; 4]) {
        let bx = edge.x * cell_size;
        let by = edge.y * cell_size;
        let half = thickness / 2;
        match edge.side {
            Side::North => self.fill_rect(bx, by.saturating_sub(half), cell_size, thickness, color),
            Side::South => self.fill_rect(
                bx,
                (by + cell_size).saturating_sub(half),
                cell_size,
                thickness,
                color,
            ),
            Side::West => self.fill_rect(bx.saturating_sub(half), by, thickness, cell_size, color),
            Side::East => self.fill_rect(
                (bx + cell_size).saturating_sub(half),
                by,
                thickness,
                cell_size,
                color,
            ),
        }
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

    fn pixel(buf: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * width + x) * 4;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn rasterize_sizes_the_buffer_from_grid_size_and_cell_size() {
        let geo = open_geo();
        let geometry = build_geometry(&geo);
        let (w, h, buf) = rasterize(&geometry, 10);
        assert_eq!(w, GEO_GRID_SIZE * 10);
        assert_eq!(h, GEO_GRID_SIZE * 10);
        assert_eq!(buf.len(), w * h * 4);
    }

    #[test]
    fn rasterize_fills_indoor_cells_darker_gray_than_outdoor() {
        // plane 2 (PLANE_X2) byte 0x80 -> indoor=true (geo.rs: `x2 & 0x80`).
        let geo = geo_with_wall(3, 3, 2, false, 0x80);
        let geometry = build_geometry(&geo);
        let (w, _h, buf) = rasterize(&geometry, 10);
        let indoor_px = pixel(&buf, w, 3 * 10 + 5, 3 * 10 + 5);
        let outdoor_px = pixel(&buf, w, 5, 5);
        assert_eq!(indoor_px, [60, 60, 60, 255]);
        assert_eq!(outdoor_px, [30, 30, 30, 255]);
    }

    #[test]
    fn rasterize_marks_nonzero_low7_with_a_yellow_pixel_at_cell_center() {
        // plane 2 (PLANE_X2) low 7 bits -> low7 (geo.rs: `x2 & 0x7F`).
        let geo = geo_with_wall(4, 4, 2, false, 7);
        let geometry = build_geometry(&geo);
        let (w, _h, buf) = rasterize(&geometry, 20);
        let center = pixel(&buf, w, 4 * 20 + 10, 4 * 20 + 10);
        assert_eq!(center, [255, 255, 0, 255]);
    }

    #[test]
    fn rasterize_draws_a_north_wall_edge_in_its_door_color() {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        let idx = 2 + 5 + 16 * 6;
        data[idx] = 3 << 4; // North wall type 3 at (5,6)
        data[idx + 3 * 256] = 0b10; // door_north = 2 (locked) -> red
        let geo = GeoBlock::parse(&data).unwrap();
        let geometry = build_geometry(&geo);
        let (w, _h, buf) = rasterize(&geometry, 20);
        // The north edge of cell (5,6) sits at y = 6*20 = 120, spanning x
        // in [100, 120).
        let edge_px = pixel(&buf, w, 5 * 20 + 10, 6 * 20);
        assert_eq!(edge_px, [255, 0, 0, 255]);
    }

    #[test]
    fn rasterize_open_map_has_no_wall_colored_pixels() {
        let geo = open_geo();
        let geometry = build_geometry(&geo);
        let (w, h, buf) = rasterize(&geometry, 10);
        for y in 0..h {
            for x in 0..w {
                let px = pixel(&buf, w, x, y);
                assert!(px == [60, 60, 60, 255] || px == [30, 30, 30, 255]);
            }
        }
    }
}
