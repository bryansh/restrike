//! The combat art stores and their loaders (`docs/design/combat-visualizer.md`
//! §1.2–1.3, D-CV4): the 48-slot 24×24 ground-tile atlas (`gbl.dax24x24Set`)
//! and the 26-slot combatant icon store (`gbl.combat_icons`), plus the four
//! loads that fill them — COMSPR at boot, ground tiles / party icons /
//! monster pictures at combat entry.
//!
//! This is the engine-side companion to [`gbx_formats::combat_art`], which
//! owns the pure decode and the pixel transforms. Nothing here draws: slice 3
//! consumes [`TileAtlas`] and [`CombatIcon`] to build the `CombatScene`.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/seg001.cs:214` — the atlas is one `DaxBlock(0, 0x30, 3, 0x18)`:
//!   **48 slots of 24×24**, allocated zeroed at boot and refilled per fight.
//! - `engine/seg001.cs:225-228` — `combat_icons = new CombatIcon[26]`.
//! - `engine/seg001.cs:312-317` — boot's COMSPR loads: blocks `0..=0x0B`
//!   into icon slots `0x0D..=0x18`, then block `0x19` into slot `0x19` (the
//!   grey focus box). This is the load `boot.rs` used to declare stubbed.
//! - `engine/ovr034.cs:9-27` (`Load24x24Set`) and `engine/ovr011.cs:757-768`
//!   (`SetupGroundTiles`) — the two atlas fills per fight.
//! - `engine/ovr034.cs:50-88` (`chead_cbody_comspr_icon`) — file/block
//!   resolution, the `+0x40` size-2 and `+0x80` Attack offsets, and the
//!   CPIC path's identity `Recolor` (a functional no-op).
//! - `engine/ovr017.cs:86-122` (`LoadPlayerCombatIcon`) — head-into-body
//!   merge order and the optional party recolor.
//! - `Classes/Combat/CombatIcon.cs:36-66` — the four-frame store
//!   (Normal/Attack × base/mirrored) and the `direction > 3` mirror rule.

use gbx_formats::combat_art::{
    self, CombatArtError, Sprite, TileSet, ATTACK_BLOCK_OFFSET, CELL_PX, TALL_BLOCK_OFFSET,
};
use gbx_formats::game_data::{GameData, GameDataError};

use crate::party::IconInfo;

/// Pixels in one 24×24 atlas cell.
pub const TILE_PIXELS: usize = CELL_PX * CELL_PX;

/// Atlas capacity (`DaxBlock(0, 0x30, 3, 0x18)`, `seg001.cs:214`).
pub const ATLAS_SLOTS: usize = 0x30;

/// Combatant icon store capacity (`seg001.cs:225`).
pub const COMBAT_ICON_SLOTS: usize = 26;

/// Icon slots `0..8` are the party (`ovr003.cs`'s per-player `icon_id`);
/// `8..` are assigned per monster type at LOADMONSTER time.
pub const PARTY_ICON_SLOTS: usize = 8;

/// First icon slot boot's COMSPR loop fills (`seg001.cs:314`'s `i + 0x0D`).
pub const COMSPR_FIRST_SLOT: usize = 0x0D;

/// How many COMSPR missile/effect blocks boot loads (`i <= 0x0b`).
pub const COMSPR_EFFECT_COUNT: usize = 0x0C;

/// The grey focus-box cursor: COMSPR block `0x19` in icon slot `0x19`
/// (`seg001.cs:317`).
pub const FOCUS_BOX_SLOT: usize = 0x19;
pub const FOCUS_BOX_BLOCK: u8 = 0x19;

/// `SetupGroundTiles`' three fills (`ovr011.cs:761-768`): `(cell_count,
/// dest_offset)` for the dungeon set, the wilderness set, and RANDCOM.
///
/// Note `WILDCOM.DAX` block 1 actually holds **34** items; the original
/// copies only the first `0x21` = 33, and slot 33 is left as whatever the
/// previous fill (or boot's zeroing) put there — a real gap between the
/// wilderness tiles and RANDCOM's base at `0x22`. Transliterated as read.
pub const DUNGEON_TILE_LOAD: (usize, usize) = (0x19, 0);
pub const WILDERNESS_TILE_LOAD: (usize, usize) = (0x21, 0);
pub const RANDCOM_TILE_LOAD: (usize, usize) = (6, 0x22);

/// Everything that can go wrong loading combat art out of a [`GameData`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatArtLoadError {
    /// The file or block isn't in the data set (or failed to decompress).
    GameData(GameDataError),
    /// The block's bytes failed to decode or transform.
    Art(CombatArtError),
    /// A derived block id (`+0x40` size-2, `+0x80` Attack) ran past `u8`.
    /// Real ids never come close; a bad LOADMONSTER operand or a corrupt
    /// character record could, and that should be loud.
    BlockIdOverflow {
        file: String,
        block_id: u8,
        offset: u8,
    },
    /// `icon_size` outside `0..=2`. The original indexes a 3-entry
    /// `sizeToken` array with it (`ovr017.cs:95`) and would throw.
    BadIconSize { icon_size: u8 },
    /// A tile-set copy would run past the 48-slot atlas. (coab's own guard
    /// is `destCellOffset > 0x30` on the offset alone — it under-checks,
    /// since it ignores `cellCount`; this checks the whole span.)
    AtlasOverflow {
        dest_offset: usize,
        cell_count: usize,
    },
    /// The decoded tile set holds fewer cells than the load asked for.
    TileSetShort {
        file: String,
        wanted: usize,
        available: usize,
    },
}

impl From<GameDataError> for CombatArtLoadError {
    fn from(e: GameDataError) -> Self {
        CombatArtLoadError::GameData(e)
    }
}

impl From<CombatArtError> for CombatArtLoadError {
    fn from(e: CombatArtError) -> Self {
        CombatArtLoadError::Art(e)
    }
}

/// The 48-slot 24×24 ground-tile atlas — `gbl.dax24x24Set`, the store
/// `DrawIsoTile` indexes with `BackGroundTiles[cell].tile_index`
/// (`ovr034.cs:30-40`).
///
/// Boot allocates it zeroed and every `BattleSetup` overwrites a prefix of
/// it, so unfilled slots read as all-palette-0 (black), not as an error —
/// [`TileAtlas::tile`] returns the raw cell either way. `is_loaded` reports
/// whether a slot was ever written, for slice 3's diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileAtlas {
    cells: Vec<u8>,
    loaded: [bool; ATLAS_SLOTS],
}

impl Default for TileAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl TileAtlas {
    /// A zeroed atlas, as boot allocates it.
    pub fn new() -> Self {
        TileAtlas {
            cells: vec![0; ATLAS_SLOTS * TILE_PIXELS],
            loaded: [false; ATLAS_SLOTS],
        }
    }

    /// Copies `cell_count` tiles from `set`'s front into slots
    /// `dest_offset..dest_offset + cell_count` (`Load24x24Set`).
    pub fn load_set(
        &mut self,
        set: &TileSet,
        cell_count: usize,
        dest_offset: usize,
        file: &str,
    ) -> Result<(), CombatArtLoadError> {
        if dest_offset + cell_count > ATLAS_SLOTS {
            return Err(CombatArtLoadError::AtlasOverflow {
                dest_offset,
                cell_count,
            });
        }
        if set.len() < cell_count {
            return Err(CombatArtLoadError::TileSetShort {
                file: file.to_string(),
                wanted: cell_count,
                available: set.len(),
            });
        }
        for (i, tile) in set.tiles[..cell_count].iter().enumerate() {
            let slot = dest_offset + i;
            let start = slot * TILE_PIXELS;
            self.cells[start..start + TILE_PIXELS].copy_from_slice(tile);
            self.loaded[slot] = true;
        }
        Ok(())
    }

    /// One slot's 24×24 pixels, row-major; `None` when `index` is past the
    /// atlas. (`DrawIsoTile`'s `tileIndex > 0x7f` overlay path is a separate
    /// stub in coab — doc §6 item 2 — and is not this store's business.)
    pub fn tile(&self, index: usize) -> Option<&[u8]> {
        if index >= ATLAS_SLOTS {
            return None;
        }
        let start = index * TILE_PIXELS;
        Some(&self.cells[start..start + TILE_PIXELS])
    }

    /// Whether `index` was ever written by a [`load_set`](Self::load_set).
    pub fn is_loaded(&self, index: usize) -> bool {
        self.loaded.get(index).copied().unwrap_or(false)
    }
}

/// The two poses every combat icon carries (`Classes/Combat/CombatIcon.cs`'s
/// `Icon` enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconPose {
    Normal,
    Attack,
}

/// The first facing that renders mirrored art (`GetIcon`'s `direction > 3`).
pub const FIRST_MIRRORED_DIRECTION: u8 = 4;

/// One combatant icon: two poses, each with its pre-computed horizontal
/// mirror. 8-way facing renders the base art for directions 0–3 and the
/// mirror for 4–7 — there is no per-direction art.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatIcon {
    normal: Sprite,
    attack: Sprite,
    normal_mirrored: Sprite,
    attack_mirrored: Sprite,
}

impl CombatIcon {
    /// Builds the four frames from a Normal/Attack pair, mirroring each
    /// (`LoadIcons`, `CombatIcon.cs:36-45`).
    pub fn from_poses(normal: Sprite, attack: Sprite) -> Self {
        let normal_mirrored = normal.mirrored();
        let attack_mirrored = attack.mirrored();
        CombatIcon {
            normal,
            attack,
            normal_mirrored,
            attack_mirrored,
        }
    }

    /// The frame to blit for `pose` facing `direction` (0–7).
    pub fn frame(&self, pose: IconPose, direction: u8) -> &Sprite {
        let mirrored = direction >= FIRST_MIRRORED_DIRECTION;
        match (pose, mirrored) {
            (IconPose::Normal, false) => &self.normal,
            (IconPose::Normal, true) => &self.normal_mirrored,
            (IconPose::Attack, false) => &self.attack,
            (IconPose::Attack, true) => &self.attack_mirrored,
        }
    }

    /// This icon's footprint in 24×24 cells, when it is a whole cell grid.
    /// `None` only for a bare `CHEAD` half, which is never a standalone
    /// combatant icon.
    pub fn cell_footprint(&self) -> Option<(usize, usize)> {
        self.normal.cell_footprint()
    }

    /// Overlays every frame of `src` onto the matching frame of `self` —
    /// the head-into-body blend (`MergeIcon`, `CombatIcon.cs:68-74`). The
    /// mirrors are merged as mirrors, matching the original's flip-then-
    /// merge order.
    pub fn merge_from(&mut self, src: &CombatIcon) -> Result<(), CombatArtError> {
        self.normal.merge_from(&src.normal)?;
        self.normal_mirrored.merge_from(&src.normal_mirrored)?;
        self.attack.merge_from(&src.attack)?;
        self.attack_mirrored.merge_from(&src.attack_mirrored)?;
        Ok(())
    }

    /// Applies one recolor table pair to all four frames (`Recolor`,
    /// `CombatIcon.cs:47-53`).
    pub fn recolor(&mut self, new_colors: &[u8; 16], old_colors: &[u8; 16]) {
        for frame in [
            &mut self.normal,
            &mut self.normal_mirrored,
            &mut self.attack,
            &mut self.attack_mirrored,
        ] {
            frame.recolor(new_colors, old_colors);
        }
    }
}

/// The 26-slot combatant icon store (`gbl.combat_icons`). Slots 0–7 are the
/// party, 8+ are per-monster-type, 13–24 are COMSPR missiles/effects, and 25
/// is the focus box — a layout the original overlaps deliberately (a fight
/// with many monster types eats into the effect slots), so this store keeps
/// the slots and no opinion about who owns which.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CombatIcons {
    slots: Vec<Option<CombatIcon>>,
}

impl CombatIcons {
    pub fn new() -> Self {
        CombatIcons {
            slots: vec![None; COMBAT_ICON_SLOTS],
        }
    }

    /// The icon in `slot`, if one was loaded.
    pub fn get(&self, slot: usize) -> Option<&CombatIcon> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    /// Installs `icon` in `slot`. Returns `false` (and stores nothing) when
    /// `slot` is past the store — the caller's slot arithmetic is wrong.
    pub fn set(&mut self, slot: usize, icon: CombatIcon) -> bool {
        match self.slots.get_mut(slot) {
            Some(entry) => {
                *entry = Some(icon);
                true
            }
            None => false,
        }
    }

    /// Clears `slot` (`ReleaseCombatIcon`, `ovr034.cs:42-45`).
    pub fn release(&mut self, slot: usize) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = None;
        }
    }

    /// How many slots hold an icon — used by the boot/entry tests.
    pub fn loaded_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

/// Adds `offset` to `block_id`, reporting the overflow rather than wrapping.
fn derived_block(file: &str, block_id: u8, offset: u8) -> Result<u8, CombatArtLoadError> {
    block_id
        .checked_add(offset)
        .ok_or_else(|| CombatArtLoadError::BlockIdOverflow {
            file: file.to_string(),
            block_id,
            offset,
        })
}

/// Loads one icon's Normal (`block_id`) and Attack (`block_id + 0x80`)
/// frames from `file` and mirrors both — `LoadIcons(0, 1, file, block_id,
/// block_id + 0x80)`, the shape every combat icon load goes through.
pub fn load_icon(
    data: &GameData,
    file: &str,
    block_id: u8,
) -> Result<CombatIcon, CombatArtLoadError> {
    let attack_id = derived_block(file, block_id, ATTACK_BLOCK_OFFSET)?;
    let normal = combat_art::decode_icon(&data.block(file, block_id)?)?;
    let attack = combat_art::decode_icon(&data.block(file, attack_id)?)?;
    Ok(CombatIcon::from_poses(normal, attack))
}

/// Boot's COMSPR slice (`seg001.cs:312-317`): missile/effect blocks
/// `0x00..=0x0B` into icon slots `0x0D..=0x18`, then the grey focus box
/// (block `0x19`) into slot `0x19`. Thirteen icons in all.
pub fn load_comspr_icons(data: &GameData) -> Result<CombatIcons, CombatArtLoadError> {
    let mut icons = CombatIcons::new();
    for block in 0..COMSPR_EFFECT_COUNT {
        let icon = load_icon(data, "COMSPR.DAX", block as u8)?;
        icons.set(COMSPR_FIRST_SLOT + block, icon);
    }
    let focus = load_icon(data, "COMSPR.DAX", FOCUS_BOX_BLOCK)?;
    icons.set(FOCUS_BOX_SLOT, focus);
    Ok(icons)
}

/// `SetupGroundTiles` (`ovr011.cs:757-768`): the dungeon or wilderness set
/// at slot 0, then RANDCOM's six at slot `0x22` (table, chair, cloudkill,
/// **blank**, stinking cloud, dead body — the fourth was doc §6 item 3's
/// "unnamed" tile; it is an all-palette-0 cell, identified in slice 3 and
/// written up on [`crate::combat::BACKGROUND_TILE_INDEX`]).
pub fn load_ground_tiles(
    data: &GameData,
    in_dungeon: bool,
) -> Result<TileAtlas, CombatArtLoadError> {
    let mut atlas = TileAtlas::new();
    let (file, (count, offset)) = if in_dungeon {
        ("DUNGCOM.DAX", DUNGEON_TILE_LOAD)
    } else {
        ("WILDCOM.DAX", WILDERNESS_TILE_LOAD)
    };
    let set = combat_art::decode_tile_set(&data.block(file, 1)?)?;
    atlas.load_set(&set, count, offset, file)?;

    let rand = combat_art::decode_tile_set(&data.block("RANDCOM.DAX", 1)?)?;
    let (count, offset) = RANDCOM_TILE_LOAD;
    atlas.load_set(&rand, count, offset, "RANDCOM.DAX")?;

    Ok(atlas)
}

/// The `+0x40` size-2 block offset (`chead_cbody_comspr_icon:59-62`, driven
/// by `LoadPlayerCombatIcon`'s `sizeToken[icon_size]`: `'\0'`/`'S'` for
/// sizes 0 and 1, `'T'` for size 2).
fn size_block_offset(icon_size: u8) -> Result<u8, CombatArtLoadError> {
    match icon_size {
        0 | 1 => Ok(0),
        2 => Ok(TALL_BLOCK_OFFSET),
        _ => Err(CombatArtLoadError::BadIconSize { icon_size }),
    }
}

/// A party member's combat icon (`LoadPlayerCombatIcon`, `ovr017.cs:86-122`):
/// `CHEAD[head_icon]` merged **into** `CBODY[weapon_icon]` (both shifted
/// `+0x40` for size 2), optionally recolored by the character's
/// `icon_colours`.
///
/// The head is the source and the body the destination, so the head's 10 (or
/// 8) rows land over the body's top rows and the rest of the body survives.
/// `recolor: false` reproduces the original's non-recoloring call sites; a
/// default-coloured character recolors to a no-op either way.
pub fn load_party_icon(
    data: &GameData,
    icon: &IconInfo,
    recolor: bool,
) -> Result<CombatIcon, CombatArtLoadError> {
    let size_offset = size_block_offset(icon.icon_size)?;
    let head_id = derived_block("CHEAD.DAX", icon.head_icon, size_offset)?;
    let body_id = derived_block("CBODY.DAX", icon.weapon_icon, size_offset)?;

    let head = load_icon(data, "CHEAD.DAX", head_id)?;
    let mut merged = load_icon(data, "CBODY.DAX", body_id)?;
    merged.merge_from(&head)?;

    if recolor {
        let (new_colors, old_colors) = combat_art::party_recolor_tables(&icon.colours);
        merged.recolor(&new_colors, &old_colors);
    }
    Ok(merged)
}

/// A monster type's combat icon: `CPIC{area}.DAX` block `block_id` (the
/// LOADMONSTER third operand), Attack at `+0x80`.
///
/// The original runs a `Recolor` on this path too, but with two identity
/// tables (`ovr034.cs:80-81`) — a functional no-op, so monsters render in
/// their stored colors and this loader skips the pass entirely.
pub fn load_monster_icon(
    data: &GameData,
    area: u8,
    block_id: u8,
) -> Result<CombatIcon, CombatArtLoadError> {
    load_icon(data, &format!("CPIC{area}.DAX"), block_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_formats::combat_art::TRANSPARENT;

    /// Hand-authored (D10): a 4bpp image block of `items`, each a flat
    /// row-major nibble list — the same shape `gbx_formats::combat_art`'s
    /// own tests build.
    fn build_block(height: u16, width_cols: u16, items: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&width_cols.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.push(items.len() as u8);
        out.extend_from_slice(&[0u8; 8]);
        for item in items {
            for pair in item.chunks(2) {
                out.push((pair[0] << 4) | *pair.get(1).unwrap_or(&0));
            }
        }
        out
    }

    /// Wraps raw block bytes in a one-block DAX container.
    fn dax_file(blocks: &[(u8, Vec<u8>)]) -> Vec<u8> {
        let header_bytes = (blocks.len() * 9) as u16;
        let mut header = Vec::new();
        let mut payload = Vec::new();
        for (id, raw) in blocks {
            // A single literal run per block: control byte = len - 1.
            let mut comp = Vec::new();
            for chunk in raw.chunks(128) {
                comp.push((chunk.len() - 1) as u8);
                comp.extend_from_slice(chunk);
            }
            header.push(*id);
            header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            header.extend_from_slice(&(raw.len() as u16).to_le_bytes());
            header.extend_from_slice(&(comp.len() as u16).to_le_bytes());
            payload.extend_from_slice(&comp);
        }
        let mut out = header_bytes.to_le_bytes().to_vec();
        out.extend_from_slice(&header);
        out.extend_from_slice(&payload);
        out
    }

    fn tile_block(fills: &[u8]) -> Vec<u8> {
        let items: Vec<Vec<u8>> = fills.iter().map(|&f| vec![f; TILE_PIXELS]).collect();
        let refs: Vec<&[u8]> = items.iter().map(Vec::as_slice).collect();
        build_block(24, 3, &refs)
    }

    fn icon_block(fill: u8) -> Vec<u8> {
        build_block(24, 3, &[&vec![fill; TILE_PIXELS]])
    }

    /// A CHEAD-shaped half: 24 px wide, `rows` tall.
    fn head_block(rows: u16, fill: u8) -> Vec<u8> {
        build_block(rows, 3, &[&vec![fill; CELL_PX * rows as usize]])
    }

    fn tile_set(fills: &[u8]) -> TileSet {
        combat_art::decode_tile_set(&tile_block(fills)).unwrap()
    }

    fn icon(fill: u8) -> CombatIcon {
        let sprite = combat_art::decode_icon(&icon_block(fill)).unwrap();
        CombatIcon::from_poses(sprite.clone(), sprite)
    }

    // --- TileAtlas ---------------------------------------------------------

    #[test]
    fn new_atlas_is_zeroed_and_unloaded() {
        let atlas = TileAtlas::new();
        assert_eq!(atlas, TileAtlas::default());
        for slot in 0..ATLAS_SLOTS {
            assert_eq!(atlas.tile(slot).unwrap(), &[0u8; TILE_PIXELS][..]);
            assert!(!atlas.is_loaded(slot));
        }
        assert!(atlas.tile(ATLAS_SLOTS).is_none());
    }

    #[test]
    fn load_set_copies_a_prefix_at_the_destination_offset() {
        let mut atlas = TileAtlas::new();
        atlas
            .load_set(&tile_set(&[1, 2, 3]), 2, 5, "T.DAX")
            .unwrap();
        assert_eq!(atlas.tile(5).unwrap()[0], 1);
        assert_eq!(atlas.tile(6).unwrap()[0], 2);
        // The third tile was not asked for, and slot 4 was never touched.
        assert!(!atlas.is_loaded(7));
        assert_eq!(atlas.tile(7).unwrap()[0], 0);
        assert!(!atlas.is_loaded(4));
        assert!(atlas.is_loaded(5) && atlas.is_loaded(6));
    }

    #[test]
    fn load_set_rejects_a_copy_past_the_atlas() {
        let mut atlas = TileAtlas::new();
        let err = atlas
            .load_set(&tile_set(&[1, 2]), 2, ATLAS_SLOTS - 1, "T.DAX")
            .unwrap_err();
        assert_eq!(
            err,
            CombatArtLoadError::AtlasOverflow {
                dest_offset: ATLAS_SLOTS - 1,
                cell_count: 2
            }
        );
        // Nothing was written before the check.
        assert!(!atlas.is_loaded(ATLAS_SLOTS - 1));
    }

    #[test]
    fn load_set_rejects_a_short_tile_set() {
        let mut atlas = TileAtlas::new();
        let err = atlas
            .load_set(&tile_set(&[1, 2]), 3, 0, "SHORT.DAX")
            .unwrap_err();
        assert_eq!(
            err,
            CombatArtLoadError::TileSetShort {
                file: "SHORT.DAX".to_string(),
                wanted: 3,
                available: 2
            }
        );
    }

    // --- CombatIcon --------------------------------------------------------

    #[test]
    fn frame_mirrors_for_directions_four_through_seven() {
        let normal = Sprite {
            width: 2,
            height: 1,
            pixels: vec![1, 2],
        };
        let attack = Sprite {
            width: 2,
            height: 1,
            pixels: vec![3, 4],
        };
        let icon = CombatIcon::from_poses(normal, attack);
        for dir in 0..4u8 {
            assert_eq!(
                icon.frame(IconPose::Normal, dir).pixels,
                vec![1, 2],
                "{dir}"
            );
            assert_eq!(
                icon.frame(IconPose::Attack, dir).pixels,
                vec![3, 4],
                "{dir}"
            );
        }
        for dir in 4..8u8 {
            assert_eq!(
                icon.frame(IconPose::Normal, dir).pixels,
                vec![2, 1],
                "{dir}"
            );
            assert_eq!(
                icon.frame(IconPose::Attack, dir).pixels,
                vec![4, 3],
                "{dir}"
            );
        }
    }

    #[test]
    fn merge_applies_to_all_four_frames_including_the_mirrors() {
        let body_sprite = Sprite {
            width: 2,
            height: 2,
            pixels: vec![TRANSPARENT, TRANSPARENT, 5, 5],
        };
        let head_sprite = Sprite {
            width: 2,
            height: 1,
            pixels: vec![9, TRANSPARENT],
        };
        let mut body = CombatIcon::from_poses(body_sprite.clone(), body_sprite);
        let head = CombatIcon::from_poses(head_sprite.clone(), head_sprite);
        body.merge_from(&head).unwrap();

        // Base frames: the head's opaque 9 lands in the top-left.
        assert_eq!(
            body.frame(IconPose::Normal, 0).pixels,
            vec![9, TRANSPARENT, 5, 5]
        );
        // Mirrored frames merged as mirrors: the head's 9 lands top-RIGHT,
        // which is the same as mirroring the merged base frame.
        assert_eq!(
            body.frame(IconPose::Normal, 4).pixels,
            vec![TRANSPARENT, 9, 5, 5]
        );
        assert_eq!(
            body.frame(IconPose::Attack, 4).pixels,
            vec![TRANSPARENT, 9, 5, 5]
        );
    }

    #[test]
    fn recolor_applies_to_all_four_frames() {
        let mut icon = icon(3);
        let old: [u8; 16] = std::array::from_fn(|i| i as u8);
        let mut new = old;
        new[3] = 11;
        icon.recolor(&new, &old);
        for dir in [0u8, 5] {
            for pose in [IconPose::Normal, IconPose::Attack] {
                assert!(icon.frame(pose, dir).pixels.iter().all(|&p| p == 11));
            }
        }
    }

    #[test]
    fn cell_footprint_reads_through_to_the_normal_frame() {
        assert_eq!(icon(1).cell_footprint(), Some((1, 1)));
    }

    // --- CombatIcons store -------------------------------------------------

    #[test]
    fn icon_store_holds_twenty_six_slots() {
        let mut icons = CombatIcons::new();
        assert_eq!(icons.loaded_count(), 0);
        assert!(icons.get(0).is_none());
        assert!(icons.set(COMBAT_ICON_SLOTS - 1, icon(1)));
        assert!(icons.get(COMBAT_ICON_SLOTS - 1).is_some());
        assert_eq!(icons.loaded_count(), 1);
        assert!(
            !icons.set(COMBAT_ICON_SLOTS, icon(1)),
            "a slot past the store must be refused, not grow it"
        );
        icons.release(COMBAT_ICON_SLOTS - 1);
        assert_eq!(icons.loaded_count(), 0);
    }

    // --- loaders -----------------------------------------------------------

    fn comspr_data() -> GameData {
        let mut blocks = Vec::new();
        for id in 0..=0x0Bu8 {
            blocks.push((id, icon_block(id + 1)));
            blocks.push((id + ATTACK_BLOCK_OFFSET, icon_block(id + 2)));
        }
        blocks.push((FOCUS_BOX_BLOCK, icon_block(15)));
        blocks.push((FOCUS_BOX_BLOCK + ATTACK_BLOCK_OFFSET, icon_block(15)));
        GameData::from_files([("COMSPR.DAX".to_string(), dax_file(&blocks))])
    }

    #[test]
    fn comspr_boot_load_fills_thirteen_slots() {
        let icons = load_comspr_icons(&comspr_data()).unwrap();
        assert_eq!(icons.loaded_count(), 13);
        for block in 0..COMSPR_EFFECT_COUNT {
            let stored = icons.get(COMSPR_FIRST_SLOT + block).expect("effect slot");
            assert_eq!(
                stored.frame(IconPose::Normal, 0).pixels[0],
                block as u8 + 1,
                "slot {}",
                COMSPR_FIRST_SLOT + block
            );
            assert_eq!(stored.frame(IconPose::Attack, 0).pixels[0], block as u8 + 2);
        }
        assert!(icons.get(FOCUS_BOX_SLOT).is_some());
        // Nothing lands in the party or monster ranges at boot.
        for slot in 0..COMSPR_FIRST_SLOT {
            assert!(icons.get(slot).is_none(), "slot {slot}");
        }
    }

    #[test]
    fn missing_attack_frame_is_a_loud_error() {
        let data =
            GameData::from_files([("COMSPR.DAX".to_string(), dax_file(&[(0u8, icon_block(1))]))]);
        assert!(matches!(
            load_icon(&data, "COMSPR.DAX", 0).unwrap_err(),
            CombatArtLoadError::GameData(_)
        ));
    }

    #[test]
    fn attack_block_id_overflow_is_reported_not_wrapped() {
        let data = GameData::from_files([("COMSPR.DAX".to_string(), dax_file(&[]))]);
        assert_eq!(
            load_icon(&data, "COMSPR.DAX", 0x80).unwrap_err(),
            CombatArtLoadError::BlockIdOverflow {
                file: "COMSPR.DAX".to_string(),
                block_id: 0x80,
                offset: ATTACK_BLOCK_OFFSET,
            }
        );
    }

    #[test]
    fn ground_tile_load_places_the_floor_set_and_randcom() {
        let dung = tile_block(&(0..0x19u8).map(|i| i % 16).collect::<Vec<_>>());
        let wild = tile_block(&(0..0x21u8).map(|_| 9u8).collect::<Vec<_>>());
        let rand = tile_block(&[1, 2, 3, 4, 5, 6]);
        let data = GameData::from_files([
            ("DUNGCOM.DAX".to_string(), dax_file(&[(1, dung)])),
            ("WILDCOM.DAX".to_string(), dax_file(&[(1, wild)])),
            ("RANDCOM.DAX".to_string(), dax_file(&[(1, rand)])),
        ]);

        let atlas = load_ground_tiles(&data, true).unwrap();
        assert!(atlas.is_loaded(0) && atlas.is_loaded(0x18));
        assert!(!atlas.is_loaded(0x19), "dungeon set stops at 0x19 tiles");
        for slot in 0x22..0x28 {
            assert!(atlas.is_loaded(slot), "RANDCOM slot {slot}");
        }
        assert_eq!(atlas.tile(0x27).unwrap()[0], 6, "the dead-body tile");
        assert!(!atlas.is_loaded(0x28));

        let atlas = load_ground_tiles(&data, false).unwrap();
        assert!(atlas.is_loaded(0x20), "wilderness set runs to 0x20");
        assert!(
            !atlas.is_loaded(0x21),
            "slot 0x21 is the gap between the wilderness set and RANDCOM"
        );
        assert!(atlas.is_loaded(0x22));
    }

    fn party_icon_data() -> GameData {
        // Head halves are 10 rows (8 for size 2) of code 2; bodies are full
        // cells of code 4. The merged head region is 2|4 = 6, so the two
        // regions land on two different template codes (6 and 4).
        let heads = dax_file(&[
            (0u8, head_block(10, 2)),
            (ATTACK_BLOCK_OFFSET, head_block(10, 2)),
            (TALL_BLOCK_OFFSET, head_block(8, 2)),
            (TALL_BLOCK_OFFSET + ATTACK_BLOCK_OFFSET, head_block(8, 2)),
        ]);
        let bodies = dax_file(&[
            (0u8, icon_block(4)),
            (ATTACK_BLOCK_OFFSET, icon_block(4)),
            (TALL_BLOCK_OFFSET, icon_block(4)),
            (TALL_BLOCK_OFFSET + ATTACK_BLOCK_OFFSET, icon_block(4)),
        ]);
        GameData::from_files([
            ("CHEAD.DAX".to_string(), heads),
            ("CBODY.DAX".to_string(), bodies),
        ])
    }

    fn icon_info(icon_size: u8, colours: [u8; 6]) -> IconInfo {
        IconInfo {
            head_icon: 0,
            weapon_icon: 0,
            icon_id: 0,
            icon_size,
            colours,
        }
    }

    /// The stock `icon_colours` a freshly rolled character carries
    /// (`ovr018.cs:341`) — the pair that recolors to the identity table.
    fn default_colours() -> [u8; 6] {
        std::array::from_fn(|i| {
            let d = gbx_formats::combat_art::DEFAULT_ICON_COLOURS[i];
            ((d + 8) << 4) | d
        })
    }

    #[test]
    fn party_icon_merges_the_head_over_the_body_top_rows() {
        let data = party_icon_data();
        let merged = load_party_icon(&data, &icon_info(1, default_colours()), false).unwrap();
        let frame = merged.frame(IconPose::Normal, 0);
        assert_eq!(
            (frame.width, frame.height),
            (24, 24),
            "the body's shape wins"
        );
        // Top 10 rows carry head(2) | body(4) = 6; the rest stay body(4).
        assert!(frame.pixels[..24 * 10].iter().all(|&p| p == (2 | 4)));
        assert!(frame.pixels[24 * 10..].iter().all(|&p| p == 4));
    }

    #[test]
    fn party_icon_size_two_uses_the_plus_0x40_art_and_its_shorter_head() {
        let data = party_icon_data();
        let merged = load_party_icon(&data, &icon_info(2, default_colours()), false).unwrap();
        let frame = merged.frame(IconPose::Normal, 0);
        assert!(frame.pixels[..24 * 8].iter().all(|&p| p == (2 | 4)));
        assert!(frame.pixels[24 * 8..].iter().all(|&p| p == 4));
    }

    #[test]
    fn party_icon_recolor_rewrites_the_template_codes() {
        let data = party_icon_data();
        // Start from the identity colours and repaint two templates only:
        // code 4 (the body) -> 10, code 6 (the merged head region) -> 5.
        // Both bright twins keep their default values so no later table
        // slot re-catches a just-rewritten pixel.
        let mut colours = default_colours();
        colours[3] = (12 << 4) | 10; // template 4: low nibble 4 -> 10
        colours[4] = (14 << 4) | 5; //  template 6: low nibble 6 -> 5
        let plain = load_party_icon(&data, &icon_info(1, colours), false).unwrap();
        let painted = load_party_icon(&data, &icon_info(1, colours), true).unwrap();

        assert_eq!(plain.frame(IconPose::Normal, 0).pixels[0], 6);
        assert_eq!(plain.frame(IconPose::Normal, 0).pixels[24 * 10], 4);
        assert_eq!(painted.frame(IconPose::Normal, 0).pixels[0], 5);
        assert_eq!(painted.frame(IconPose::Normal, 0).pixels[24 * 10], 10);
    }

    #[test]
    fn party_icon_with_default_colours_recolors_to_a_no_op() {
        let data = party_icon_data();
        let info = icon_info(1, default_colours());
        assert_eq!(
            load_party_icon(&data, &info, false).unwrap(),
            load_party_icon(&data, &info, true).unwrap()
        );
    }

    #[test]
    fn party_icon_rejects_an_out_of_range_size() {
        let data = party_icon_data();
        assert_eq!(
            load_party_icon(&data, &icon_info(3, [0; 6]), false).unwrap_err(),
            CombatArtLoadError::BadIconSize { icon_size: 3 }
        );
    }

    #[test]
    fn monster_icon_resolves_the_area_suffixed_file() {
        let data = GameData::from_files([(
            "CPIC3.DAX".to_string(),
            dax_file(&[
                (7u8, icon_block(6)),
                (7 + ATTACK_BLOCK_OFFSET, icon_block(8)),
            ]),
        )]);
        let mon = load_monster_icon(&data, 3, 7).unwrap();
        assert_eq!(mon.frame(IconPose::Normal, 0).pixels[0], 6);
        assert_eq!(mon.frame(IconPose::Attack, 0).pixels[0], 8);
        assert!(matches!(
            load_monster_icon(&data, 2, 7).unwrap_err(),
            CombatArtLoadError::GameData(GameDataError::UnknownFile { .. })
        ));
    }

    /// Local-only tier: the real combat-entry loads all succeed against the
    /// real data set, at the shapes §1.3 describes. Loud-skips without
    /// `GBX_DATA_DIR`.
    #[test]
    fn real_combat_entry_art_loads() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
            .expect("GBX_DATA_DIR must be readable");

        let icons = load_comspr_icons(&data).expect("boot COMSPR load");
        assert_eq!(icons.loaded_count(), 13);
        assert_eq!(
            icons.get(FOCUS_BOX_SLOT).unwrap().cell_footprint(),
            Some((1, 1)),
            "the focus box is one cell"
        );

        for in_dungeon in [true, false] {
            let atlas = load_ground_tiles(&data, in_dungeon).expect("ground tiles");
            assert!(atlas.is_loaded(0));
            assert!(atlas.is_loaded(0x27), "RANDCOM's dead-body tile");
        }

        // Every real head/weapon icon id pairs into a mergeable icon.
        for icon_size in [1u8, 2] {
            for head_icon in 0..14u8 {
                for weapon_icon in [0u8, 15, 31] {
                    let info = icon_info_for(head_icon, weapon_icon, icon_size);
                    let merged = load_party_icon(&data, &info, true)
                        .unwrap_or_else(|e| panic!("head {head_icon} body {weapon_icon}: {e:?}"));
                    assert_eq!(merged.cell_footprint(), Some((1, 1)));
                }
            }
        }

        // Every CPIC block in every area file loads as a monster icon.
        let mut monsters = 0usize;
        for area in 1..=6u8 {
            let file = format!("CPIC{area}.DAX");
            let Some(bytes) = data.raw_file(&file) else {
                continue;
            };
            let archive = gbx_formats::dax::DaxArchive::parse(bytes).unwrap();
            let ids: Vec<u8> = archive.entries().iter().map(|e| e.id).collect();
            for id in ids.into_iter().filter(|id| *id < ATTACK_BLOCK_OFFSET) {
                let mon = load_monster_icon(&data, area, id)
                    .unwrap_or_else(|e| panic!("{file} block {id}: {e:?}"));
                let (cols, rows) = mon.cell_footprint().expect("whole cell grid");
                assert!((1..=2).contains(&cols) && (1..=2).contains(&rows));
                monsters += 1;
            }
        }
        assert!(monsters > 0, "no CPIC monster icons found");
        eprintln!("loaded 13 COMSPR icons, both tile atlases, {monsters} monster icons");
    }

    #[cfg(test)]
    fn icon_info_for(head_icon: u8, weapon_icon: u8, icon_size: u8) -> IconInfo {
        IconInfo {
            head_icon,
            weapon_icon,
            icon_id: 0,
            icon_size,
            colours: [0x91, 0xA2, 0xB3, 0xC4, 0xE6, 0xF7],
        }
    }
}
