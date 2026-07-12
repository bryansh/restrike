//! GEO map blocks: the 16x16 town/dungeon-square grid (walls, doors, and a
//! per-square flags byte) backing the automap and first-person wall
//! rendering. `docs/design/vm-scriptmemory.md`'s Global ScriptMemory window
//! already names the runtime-facing half of this data (`mapWallType`/
//! `mapWallRoof` at `0xC04B+`); this module parses the on-disk source of
//! truth those cells are queried from.
//!
//! Derived by reading coab for behavior (D11, never copied) and
//! cross-checked against three independent sources — the strongest
//! corroboration any format in this codebase has had:
//! - coab `Classes/GeoBlock.cs` (`:1-126`, the block/grid structure) and
//!   `engine/ovr031.cs` (`Load3DMap` `:690-705` — hard-asserts the decoded
//!   block is exactly `0x402` bytes; `getMap_wall_type` `:222-251`,
//!   `get_wall_x2` `:289-318`, `WallDoorFlagsGet` `:181-219` — the runtime
//!   query/door semantics this module's field layout must match).
//! - ssi-engine (Java, GPL-3) `src/main/java/data/dungeon/DungeonMap.java`
//!   (`:14-59`, `readMap`) — byte-for-byte agreement on all four plane
//!   offsets and nibble/bit extraction, independently derived.
//! - The Jzatopa corpus's Matrix Cubed raw-disassembly notes
//!   (`SSI-Engine-Full-Play-ability/scripts/geo_inspect.py:47-56`,
//!   `docs/load3dmap-cluster-resolution-pass-1.md:31-40`) — real retail
//!   game bytes (a different Gold Box title, same engine generation)
//!   confirming the same four-256-byte-plane structure via x86
//!   disassembly, independent of any C#/Java decompile.
//! - Confirmed against real CotAB data this session: every block in
//!   `GEO2.DAX` (ids 1/3/4 — Tilverton City, Tilverton Sewers, the Fire
//!   Knift Hideout, per Gold Box Explorer's per-game GEO-id table) has
//!   `raw_size == 0x402` exactly, matching this module's hard size check.
//!
//! **Explicitly NOT derived from `~/src/goldbox-refs/tools/hackdocs/GEO*.TXT`**
//! — those describe an unrelated format ("Unlimited Adventures", SSI's
//! separate level-editor toolkit): a flat 6-byte-per-square record with an
//! explicit per-square event-table index, numerically incompatible with
//! CotAB's four-plane/nibble+2-bit layout. See this module's docket note on
//! the `x2` byte's low 7 bits for where that contradiction matters.
//!
//! ## On-disk layout
//!
//! A GEO block, once extracted from its DAX container (`dax::block_data`,
//! no extra prefix-stripping — unlike `ECL*.DAX` blocks, a GEO block's
//! *entire* decompressed payload, header included, is exactly [`GEO_BLOCK_SIZE`]
//! bytes):
//!
//! ```text
//! offset 0x000-0x001 (2 bytes): header — meaning undetermined (docket)
//! offset 0x002-0x101 (256 bytes): plane 0 — N/E wall-type nibbles
//! offset 0x102-0x201 (256 bytes): plane 1 — S/W wall-type nibbles
//! offset 0x202-0x301 (256 bytes): plane 2 — the "x2" flags byte
//! offset 0x302-0x401 (256 bytes): plane 3 — door-state 2-bit fields
//! ```
//!
//! Each plane is indexed `x + 16*y` (row-major, `x`/`y` both `0..16`).
//! Plane 0's byte packs North in its high nibble, East in its low nibble;
//! plane 1 packs South (high) and West (low) the same way. A wall-type
//! nibble of `0` means "no wall on that edge" (open passage); `1..=15`
//! selects a wall-texture slot (`WallDef`) — irrelevant to wall/door
//! *presence*, which is all an automap needs. Plane 3's byte packs four
//! 2-bit door-state fields: bits `6-7` West, `4-5` South, `2-3` East,
//! `0-1` North (coab `Classes/GeoBlock.cs`'s `x3_dir_N` fields). A door
//! field is only meaningful when that direction's wall-type nibble is
//! nonzero (`WallDoorFlagsGet`, `ovr031.cs:181-219`): `0` = solid wall (no
//! door, blocks movement), `1` = open/unlocked door, `2` = locked door,
//! `3` = hard-locked ("unpickable") door.
//!
//! The `x2` byte (plane 2) is read by scripts through the Global
//! ScriptMemory window (`mapWallRoof`, `0xC04F`,
//! `docs/design/vm-scriptmemory.md` §1) and, per coab, bit `0x80` is a
//! confirmed indoor/has-roof flag (`ovr029.cs:21-29`: gates
//! indoor-vs-outdoor sky color). Bit `0x40`'s meaning is unresolved
//! (`ovr011.cs:518`, a dungeon floor-tile cosmetic flag — docket). The low
//! 7 bits (`x2 & 0x7F`) are a **narrowed, not resolved** hypothesis: the
//! Matrix Cubed corpus and Gold Box Explorer both treat this as a
//! per-square event/trigger id, which is architecturally plausible (CotAB's
//! own opcode census found the CALL `0xAE11` case — a wall-roof/wall-type
//! query — on 38 of 52 real CALL instructions, i.e. nearly every
//! world-menu step reads this exact byte) but was not independently
//! confirmed against CotAB's own ECL disassembly this session. Exposed as
//! [`Square::low7`] with that caveat; the automap marks nonzero squares
//! without asserting they mean "event" specifically.

/// A GEO block's total on-disk size (header + 4 planes), verified against
/// coab's own hard assert (`Load3DMap`, `ovr031.cs:690-705`) and confirmed
/// on every real CotAB `GEO2.DAX` block this session.
pub const GEO_BLOCK_SIZE: usize = 0x402;
/// The grid is always 16x16 (coab `Classes/GeoBlock.cs`'s `MapInfo[16,16]`).
pub const GEO_GRID_SIZE: usize = 16;

const PLANE_SIZE: usize = 256;
const PLANE_NE: usize = 2; // + header
const PLANE_SW: usize = 2 + PLANE_SIZE;
const PLANE_X2: usize = 2 + 2 * PLANE_SIZE;
const PLANE_DOOR: usize = 2 + 3 * PLANE_SIZE;

/// [`GeoBlock::parse`]'s failure mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeoError {
    /// The decompressed block isn't exactly [`GEO_BLOCK_SIZE`] bytes —
    /// matches coab's own hard assert on this exact condition
    /// (`Load3DMap`, `ovr031.cs:690-705`), reported here as a clean `Err`
    /// rather than aborting.
    WrongSize { expected: usize, actual: usize },
}

/// One 16x16 grid square's wall/door/flags state.
///
/// `wall_*` is the raw 0-15 wall-type nibble (`0` = open, `1..=15` = a
/// `WallDef` texture slot — texture identity doesn't matter for wall/door
/// *presence*, which is all this struct's consumers need). `door_*` is the
/// raw 0-3 door-state field, meaningful only when the matching `wall_*` is
/// nonzero (see this module's doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Square {
    pub wall_north: u8,
    pub wall_east: u8,
    pub wall_south: u8,
    pub wall_west: u8,
    pub door_north: u8,
    pub door_east: u8,
    pub door_south: u8,
    pub door_west: u8,
    /// `x2 & 0x80` — confirmed indoor/has-roof flag.
    pub indoor: bool,
    /// `x2 & 0x40` — meaning unresolved (docket candidate).
    pub floor_flag: bool,
    /// `x2 & 0x7F` — hypothesized per-square event/trigger id, unconfirmed
    /// for CotAB specifically (see this module's doc comment).
    pub low7: u8,
}

/// A parsed GEO block: the 2-byte header (meaning undetermined) and the
/// 16x16 square grid, indexed `[y][x]` (row-major, matching the on-disk
/// plane layout).
#[derive(Debug, Clone)]
pub struct GeoBlock {
    pub header: [u8; 2],
    squares: [[Square; GEO_GRID_SIZE]; GEO_GRID_SIZE],
}

impl GeoBlock {
    /// Parses a GEO block's full decompressed DAX payload (header
    /// included — see this module's doc comment on why GEO blocks need no
    /// separate prefix-stripping step, unlike `ECL*.DAX`).
    pub fn parse(data: &[u8]) -> Result<Self, GeoError> {
        if data.len() != GEO_BLOCK_SIZE {
            return Err(GeoError::WrongSize {
                expected: GEO_BLOCK_SIZE,
                actual: data.len(),
            });
        }
        let header = [data[0], data[1]];

        let mut squares = [[Square::default(); GEO_GRID_SIZE]; GEO_GRID_SIZE];
        for (y, row) in squares.iter_mut().enumerate() {
            for (x, square) in row.iter_mut().enumerate() {
                let i = x + GEO_GRID_SIZE * y;
                let ne = data[PLANE_NE + i];
                let sw = data[PLANE_SW + i];
                let x2 = data[PLANE_X2 + i];
                let door = data[PLANE_DOOR + i];

                *square = Square {
                    wall_north: ne >> 4,
                    wall_east: ne & 0x0F,
                    wall_south: sw >> 4,
                    wall_west: sw & 0x0F,
                    door_north: door & 0b11,
                    door_east: (door >> 2) & 0b11,
                    door_south: (door >> 4) & 0b11,
                    door_west: (door >> 6) & 0b11,
                    indoor: x2 & 0x80 != 0,
                    floor_flag: x2 & 0x40 != 0,
                    low7: x2 & 0x7F,
                };
            }
        }

        Ok(GeoBlock { header, squares })
    }

    /// The square at `(x, y)`, both `0..GEO_GRID_SIZE`. Panics out of
    /// range — a caller bug (the grid is always exactly 16x16), not a
    /// runtime condition.
    pub fn square(&self, x: usize, y: usize) -> &Square {
        &self.squares[y][x]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-authored (D10): builds a `0x402`-byte synthetic GEO payload
    /// with a handful of squares set to known wall/door/flag values, in
    /// exactly the plane layout this module's doc comment describes.
    fn synthetic_block() -> Vec<u8> {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        data[0] = 0xAB;
        data[1] = 0xCD; // header, opaque

        // Square (2, 3): North wall type 5, East wall type 0 (open).
        data[PLANE_NE + 2 + 16 * 3] = 5 << 4;
        // Square (2, 3): South wall type 0xF, West wall type 1.
        data[PLANE_SW + 2 + 16 * 3] = (0xF << 4) | 1;
        // Square (2, 3): x2 = indoor + floor_flag + low7=0x2A.
        data[PLANE_X2 + 2 + 16 * 3] = 0x80 | 0x40 | 0x2A;
        // Square (2, 3): door_north=2 (locked), door_west=3 (hard-locked).
        data[PLANE_DOOR + 2 + 16 * 3] = 0b11_00_00_10;

        data
    }

    #[test]
    fn rejects_wrong_size_payload() {
        let err = GeoBlock::parse(&[0u8; 10]).unwrap_err();
        assert_eq!(
            err,
            GeoError::WrongSize {
                expected: GEO_BLOCK_SIZE,
                actual: 10
            }
        );
    }

    #[test]
    fn parses_header_bytes_verbatim() {
        let block = GeoBlock::parse(&synthetic_block()).unwrap();
        assert_eq!(block.header, [0xAB, 0xCD]);
    }

    #[test]
    fn extracts_wall_nibbles_for_all_four_directions() {
        let block = GeoBlock::parse(&synthetic_block()).unwrap();
        let sq = block.square(2, 3);
        assert_eq!(sq.wall_north, 5);
        assert_eq!(sq.wall_east, 0);
        assert_eq!(sq.wall_south, 0xF);
        assert_eq!(sq.wall_west, 1);
    }

    #[test]
    fn extracts_door_fields_for_all_four_directions() {
        let block = GeoBlock::parse(&synthetic_block()).unwrap();
        let sq = block.square(2, 3);
        assert_eq!(sq.door_north, 2);
        assert_eq!(sq.door_east, 0);
        assert_eq!(sq.door_south, 0);
        assert_eq!(sq.door_west, 3);
    }

    #[test]
    fn extracts_x2_flags_and_low7() {
        let block = GeoBlock::parse(&synthetic_block()).unwrap();
        let sq = block.square(2, 3);
        assert!(sq.indoor);
        assert!(sq.floor_flag);
        // low7 = x2 & 0x7F necessarily includes bit 6 (floor_flag's own
        // bit) — the two hypotheses (bit 6 as a distinct flag, the full
        // low 7 bits as an event id) are competing interpretations of
        // overlapping bits, not disjoint fields (this module's doc
        // comment); 0x80|0x40|0x2A = 0xEA, masked to 0x7F = 0x6A.
        assert_eq!(sq.low7, 0x6A);
    }

    #[test]
    fn untouched_squares_default_to_all_open() {
        let block = GeoBlock::parse(&synthetic_block()).unwrap();
        let sq = block.square(0, 0);
        assert_eq!(sq, &Square::default());
        assert_eq!(sq.wall_north, 0);
        assert!(!sq.indoor);
    }

    #[test]
    fn grid_is_indexed_row_major_x_plus_16_y() {
        let mut data = vec![0u8; GEO_BLOCK_SIZE];
        // Square (15, 0): last column, first row.
        data[PLANE_NE + 15] = 0x30;
        let block = GeoBlock::parse(&data).unwrap();
        assert_eq!(block.square(15, 0).wall_north, 3);
        assert_eq!(block.square(15, 1).wall_north, 0);
        assert_eq!(block.square(0, 0).wall_north, 0);
    }

    /// Local-only tier (pattern from `detect.rs`/`dax.rs`): every GEO block
    /// in the real data set parses cleanly at exactly [`GEO_BLOCK_SIZE`]
    /// bytes — the task brief's "every GEO block in the real data parses
    /// and renders without error" requirement (parsing half; rendering is
    /// `restrike map`'s own local-only test in `frontends/cli`).
    #[test]
    fn every_real_geo_block_parses() {
        use crate::dax::DaxArchive;
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);

        let mut blocks_checked = 0usize;
        for entry in std::fs::read_dir(dir).expect("GBX_DATA_DIR must be readable") {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            if !(name.starts_with("GEO") && name.ends_with(".DAX")) {
                continue;
            }

            let bytes = std::fs::read(&path).unwrap();
            let archive = DaxArchive::parse(&bytes)
                .unwrap_or_else(|e| panic!("{}: failed to parse DAX: {e:?}", path.display()));
            for block_entry in archive.entries() {
                let raw = archive.block_data(block_entry.id).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to extract: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                GeoBlock::parse(&raw).unwrap_or_else(|e| {
                    panic!(
                        "{}: block {} failed to parse as GEO: {e:?}",
                        path.display(),
                        block_entry.id
                    )
                });
                blocks_checked += 1;
            }
        }

        assert!(
            blocks_checked > 0,
            "GBX_DATA_DIR is set but no *.DAX files starting with GEO were found"
        );
        eprintln!("checked {blocks_checked} real GEO block(s)");
    }
}
