//! Block-kind detection: which decoded view a `(file, block)` pair gets in
//! the resource browser. Mirrors the filename-prefix convention `boot.rs`/
//! `vmhost.rs`/each `gbx-formats` decoder's own local-only tests already use
//! — there's no in-repo shared helper for this (each format module tests
//! itself against `GBX_DATA_DIR` independently), so this is `tools/inspect`'s
//! own dispatch table, kept in one place rather than duplicated per pane.

/// Which decoded view a block gets, chosen by filename convention (and, for
/// the mono font, a specific block id) — never by sniffing content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Ecl,
    Geo,
    Walldef,
    /// The mono font: `8X8D1.DAX` block 201 specifically — checked before
    /// the generic `Image` prefix rule since it lives in an `8X8D`-prefixed
    /// file despite being a different container format.
    Font,
    /// A static 4bpp image block (`8X8D*`, `BIGPIC*`, `HEAD*`, `BODY*`,
    /// `SKY*`).
    Image,
    /// An animated picture block (`PIC*`, `SPRIT*`, `FINAL*`).
    AnimatedPicture,
    /// No convention matched — the hex view fallback.
    Unknown,
}

const IMAGE_PREFIXES: [&str; 5] = ["8X8D", "BIGPIC", "HEAD", "BODY", "SKY"];
const ANIM_PREFIXES: [&str; 3] = ["PIC", "SPRIT", "FINAL"];

/// Classifies `(file_name, block_id)` by the on-disk naming convention.
/// `file_name` is matched case-insensitively (the DOS-era convention
/// `GameData` itself already normalizes to uppercase internally).
pub fn classify(file_name: &str, block_id: u8) -> BlockKind {
    let upper = file_name.to_ascii_uppercase();
    if !upper.ends_with(".DAX") {
        return BlockKind::Unknown;
    }

    if upper == "8X8D1.DAX" && block_id == 201 {
        return BlockKind::Font;
    }
    if upper.starts_with("ECL") {
        return BlockKind::Ecl;
    }
    if upper.starts_with("GEO") {
        return BlockKind::Geo;
    }
    if upper.starts_with("WALLDEF") {
        return BlockKind::Walldef;
    }
    if ANIM_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return BlockKind::AnimatedPicture;
    }
    if IMAGE_PREFIXES.iter().any(|p| upper.starts_with(p)) {
        return BlockKind::Image;
    }
    BlockKind::Unknown
}

/// A reasonable default mask color for `BlockKind::Image`/
/// `BlockKind::AnimatedPicture` blocks, by filename convention
/// (`docs/design/renderer-ui-shell.md` §1.3/§1.8): 8×8 symbol sets and the
/// `SKY` backdrop load masked against 13 (`boot.rs`'s `BOOT_MASK`);
/// everything else (pictures, portraits, animations) masks against 0
/// (`ovr030.load_pic_final`'s `DaxToPicture(0, masked, ...)`). A
/// resource-browser display convenience, not a verified-for-every-file
/// engine rule.
pub fn default_mask(file_name: &str) -> Option<u8> {
    let upper = file_name.to_ascii_uppercase();
    if upper.starts_with("8X8D") || upper.starts_with("SKY") {
        Some(13)
    } else {
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_block_takes_precedence_over_the_generic_image_rule() {
        assert_eq!(classify("8X8D1.DAX", 201), BlockKind::Font);
        assert_eq!(classify("8x8d1.dax", 201), BlockKind::Font);
    }

    #[test]
    fn other_8x8d1_blocks_are_plain_images() {
        assert_eq!(classify("8X8D1.DAX", 0xCA), BlockKind::Image);
        assert_eq!(classify("8X8D1.DAX", 0), BlockKind::Image);
    }

    #[test]
    fn ecl_geo_walldef_route_by_prefix() {
        assert_eq!(classify("ECL2.DAX", 1), BlockKind::Ecl);
        assert_eq!(classify("GEO2.DAX", 1), BlockKind::Geo);
        assert_eq!(classify("WALLDEF5.DAX", 14), BlockKind::Walldef);
    }

    #[test]
    fn animated_prefixes_route_before_bigpic() {
        assert_eq!(classify("PIC01.DAX", 1), BlockKind::AnimatedPicture);
        assert_eq!(classify("SPRIT3.DAX", 1), BlockKind::AnimatedPicture);
        assert_eq!(classify("FINAL1.DAX", 1), BlockKind::AnimatedPicture);
        // BIGPIC contains "PIC" but not as a prefix -- must stay an Image,
        // not get swept up by the "PIC" animated-prefix rule.
        assert_eq!(classify("BIGPIC1.DAX", 1), BlockKind::Image);
    }

    #[test]
    fn image_prefixes_cover_head_body_sky() {
        assert_eq!(classify("HEAD01.DAX", 1), BlockKind::Image);
        assert_eq!(classify("BODY01.DAX", 1), BlockKind::Image);
        assert_eq!(classify("SKY.DAX", 250), BlockKind::Image);
    }

    #[test]
    fn unmatched_names_and_non_dax_files_are_unknown() {
        assert_eq!(classify("TITLE.DAX", 1), BlockKind::Unknown);
        assert_eq!(classify("SAVE1.GAM", 1), BlockKind::Unknown);
    }

    #[test]
    fn matching_is_case_insensitive_throughout() {
        assert_eq!(classify("geo2.dax", 1), BlockKind::Geo);
        assert_eq!(classify("WaLlDeF2.DaX", 1), BlockKind::Walldef);
    }

    #[test]
    fn default_mask_uses_13_for_symbol_and_sky_assets() {
        assert_eq!(default_mask("8X8D1.DAX"), Some(13));
        assert_eq!(default_mask("SKY.DAX"), Some(13));
    }

    #[test]
    fn default_mask_uses_0_for_everything_else() {
        assert_eq!(default_mask("BIGPIC1.DAX"), Some(0));
        assert_eq!(default_mask("HEAD01.DAX"), Some(0));
        assert_eq!(default_mask("PIC01.DAX"), Some(0));
    }
}
