//! The boot slice of resident assets (§1.3): the mono font, symbol set 4,
//! symbol set 0, the thirteen `COMSPR` combat icons, and the three `SKY`
//! backdrop blocks. Sets 1-3 (wallsets) are step-5 scope: the slots exist
//! ([`SymbolSets`]) but nothing loads them here.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/seg001.cs:305-321` — the exact boot call sequence:
//!   `Load8x8Tiles()` (font, `:305`), `Load8x8D(4, 0xCA)` and
//!   `Load8x8D(0, 0xCB)` (`:309-310`, color-13-masked), the `COMSPR` icon
//!   loop (`:312-317`), then `LoadDax(13, 1, {250,251,252}, "SKY")`
//!   (`:319-321`). Boot's `ItemDataTable` construction (`:323`) is an M3
//!   inventory surface — still declared stubbed here, not loaded, so this
//!   module isn't mistaken for complete (design doc §1.3's closing note).
//! - `ovr038.Load8x8D` (`:8-22`) resolves its file as `"8x8d" +
//!   gbl.game_area`; at boot `game_area` names the same `8X8D1.DAX` file
//!   the font's block 201 lives in (design doc §1.3's own citation groups
//!   these three blocks together).
//! - The `COMSPR` slice's own citations live with its loader,
//!   [`crate::combat_art::load_comspr_icons`]
//!   (`docs/design/combat-visualizer.md` §1.3, M6 slice 1 — this discharges
//!   the stub the previous revision of this comment declared, and corrects
//!   its line cites: the COMSPR loop is `:312-317`, not `:308-311`, and
//!   `ITEMS` is `:323`, not `:321`).

use crate::combat_art::{self, CombatArtLoadError, CombatIcons};
use crate::symbols::SymbolSets;
use gbx_formats::font::{self, Font};
use gbx_formats::game_data::{GameData, GameDataError};
use gbx_formats::image::{self, ImageBlock, ImageError};

/// The color code every boot-loaded 8×8/SKY asset is masked against
/// (`Load8x8D`'s `LoadDax(13, 1, ...)`, §1.3).
const BOOT_MASK: u8 = 13;

/// The boot slice's resident assets.
#[derive(Debug, Clone)]
pub struct BootAssets {
    pub font: Font,
    pub symbol_sets: SymbolSets,
    /// `SKY` blocks 250 (moon), 251 (sun), 252 (horizon backdrop).
    pub sky: [ImageBlock; 3],
    /// The 26-slot combat icon store with boot's thirteen `COMSPR` icons in
    /// it (slots `0x0D..=0x18` missiles/effects, slot `0x19` the grey focus
    /// box). The party and per-monster-type slots below `0x0D` fill at
    /// combat entry, not here.
    pub combat_icons: CombatIcons,
}

/// [`boot`]'s failure mode. [`BootError::Geo`] is unused by [`boot`] itself
/// — it's here so `engine.rs`'s `Engine::new` (which also loads the M2
/// session's hardcoded resident GEO block alongside the boot slice, D-UI1)
/// can report both failure kinds through the one `Result<Self, BootError>`
/// the design doc's `Engine::new` signature specifies, without a second
/// error enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootError {
    GameData(GameDataError),
    Image(ImageError),
    Geo(gbx_formats::geo::GeoError),
    CombatArt(CombatArtLoadError),
}

impl From<GameDataError> for BootError {
    fn from(e: GameDataError) -> Self {
        BootError::GameData(e)
    }
}

impl From<ImageError> for BootError {
    fn from(e: ImageError) -> Self {
        BootError::Image(e)
    }
}

impl From<gbx_formats::geo::GeoError> for BootError {
    fn from(e: gbx_formats::geo::GeoError) -> Self {
        BootError::Geo(e)
    }
}

impl From<CombatArtLoadError> for BootError {
    fn from(e: CombatArtLoadError) -> Self {
        BootError::CombatArt(e)
    }
}

/// Loads the boot slice from `data` (`seg001.cs:305-321`'s font/set-4/set-0/
/// COMSPR/SKY portion).
pub fn boot(data: &GameData) -> Result<BootAssets, BootError> {
    let font_bytes = data.block("8X8D1.DAX", 201)?;
    let font = font::decode(&font_bytes);

    let mut symbol_sets = SymbolSets::new();
    let set4_bytes = data.block("8X8D1.DAX", 0xCA)?;
    symbol_sets.load(4, image::decode(&set4_bytes, Some(BOOT_MASK))?);
    let set0_bytes = data.block("8X8D1.DAX", 0xCB)?;
    symbol_sets.load(0, image::decode(&set0_bytes, Some(BOOT_MASK))?);

    let combat_icons = combat_art::load_comspr_icons(data)?;

    let mut sky_blocks = Vec::with_capacity(3);
    for block_id in [250u8, 251, 252] {
        let bytes = data.block("SKY.DAX", block_id)?;
        sky_blocks.push(image::decode(&bytes, Some(BOOT_MASK))?);
    }
    let sky: [ImageBlock; 3] = sky_blocks
        .try_into()
        .unwrap_or_else(|_| unreachable!("exactly 3 sky blocks were pushed"));

    Ok(BootAssets {
        font,
        symbol_sets,
        sky,
        combat_icons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Local-only tier (pattern from `gbx-formats`): boots from the real
    /// `GBX_DATA_DIR` data set without error, and every loaded asset has
    /// sane shape.
    #[test]
    fn boots_from_real_game_data() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let data = gbx_formats::game_data::load_dir(dir).expect("GBX_DATA_DIR must be readable");

        let assets = boot(&data).expect("boot must succeed against real CotAB data");

        assert!(assets.symbol_sets.get(4).is_some());
        assert!(assets.symbol_sets.get(0).is_some());
        assert!(
            assets.symbol_sets.get(1).is_none(),
            "wallsets are step-5 scope"
        );
        assert!(assets.symbol_sets.get(2).is_none());
        assert!(assets.symbol_sets.get(3).is_none());
        for sky in &assets.sky {
            assert!(sky.height > 0 && sky.width_cols > 0);
        }
        // The COMSPR slice: twelve missile/effect icons plus the focus box.
        assert_eq!(assets.combat_icons.loaded_count(), 13);
        for slot in 0..combat_art::COMSPR_EFFECT_COUNT {
            assert!(
                assets
                    .combat_icons
                    .get(combat_art::COMSPR_FIRST_SLOT + slot)
                    .is_some(),
                "COMSPR effect slot {slot} must be loaded at boot"
            );
        }
        assert!(assets
            .combat_icons
            .get(combat_art::FOCUS_BOX_SLOT)
            .is_some());
        assert!(
            assets.combat_icons.get(0).is_none(),
            "party icon slots fill at combat entry, not boot"
        );
        eprintln!(
            "boot: font ok, set4 items={}, set0 items={}, sky[0..3] loaded, \
             combat icons={}",
            assets.symbol_sets.get(4).unwrap().items.len(),
            assets.symbol_sets.get(0).unwrap().items.len(),
            assets.combat_icons.loaded_count(),
        );
    }
}
