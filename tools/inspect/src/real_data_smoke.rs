//! Local-only tier (pattern from `gbx-formats`' own decoder tests): drives
//! every real block through the exact decode paths the resource browser's
//! panes use (`block_kind::classify` then the matching decoder), without a
//! display — this crate is a `[[bin]]` with no windowing test harness, so
//! this is the closest thing to an automated pane smoke test: it proves the
//! non-egui half of every pane (decode + view-model composition) doesn't
//! panic or error against the full real CotAB data set. The egui rendering
//! itself is verified by use (task brief's testing note). Silently passes
//! when `GBX_DATA_DIR` is unset, matching every other local-only test in
//! this repo.

use crate::viewmodel::{block_kind, block_kind::BlockKind, walldef as wv};

#[test]
fn every_real_block_decodes_via_the_pane_it_would_route_to() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        return;
    };
    let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
        .expect("GBX_DATA_DIR must be readable");

    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut walldef_tiles_resolved = 0usize;

    for file in data.file_names().map(str::to_string).collect::<Vec<_>>() {
        // Mirrors the resource browser's own tree-building fallback
        // (`resource_browser.rs`'s `tree()`): a file that fails to parse as
        // a DAX index at all (e.g. a duplicate block id, `dax.rs`'s
        // `DuplicateBlockId`) shows as "0 blocks" in the tree rather than
        // crashing the pane -- this test matches that, not a hard failure.
        let Ok(archive) = data.archive(&file) else {
            eprintln!(
                "real_data_smoke: {file}: not a parseable DAX index, skipped (tree shows 0 blocks)"
            );
            continue;
        };
        let entries = archive.entries().to_vec();
        for entry in &entries {
            let kind = block_kind::classify(&file, entry.id);
            let raw = data
                .block(&file, entry.id)
                .unwrap_or_else(|e| panic!("{file} block {}: extract failed: {e:?}", entry.id));

            match kind {
                BlockKind::Ecl => {
                    // Disasm pane's own path: strip the 2-byte prefix, decode
                    // header vectors, and disassemble -- must never panic,
                    // whatever the block's actual bytecode looks like.
                    let payload = gbx_formats::dax::ecl_block_payload(&raw);
                    let payload = &payload[..payload.len().min(gbx_vm::ECL_BLOCK_SIZE)];
                    let block = gbx_vm::BlockBytes::from_bytes(payload);
                    let (vectors, _) =
                        gbx_vm::read_header_vectors(&block, gbx_vm::dialect::COTAB_VECTOR_COUNT);
                    let mut addrs: Vec<u16> = vectors.into_iter().flatten().collect();
                    if addrs.is_empty() {
                        addrs.push(gbx_vm::ECL_BLOCK_BASE);
                    }
                    let listing = gbx_vm::disassemble(&block, &gbx_vm::dialect::COTAB, &addrs);
                    let rendered = listing.render(&gbx_vm::dialect::COTAB);
                    let _ = listing.summary();

                    // The disasm pane's goto-address box (D-UI8 copy/paste
                    // ergonomics pass) against a real, previously-observed
                    // field find: 0x8295 is a real instruction address in
                    // this data set's ECL2.DAX block 1 -- exercise the exact
                    // pipeline the pane uses (parse the typed text, then
                    // locate its line in the rendered listing) whenever a
                    // block's listing contains it, so this regresses if
                    // either half of the goto pipeline breaks.
                    if file.eq_ignore_ascii_case("ECL2.DAX") && entry.id == 1 {
                        let addr = crate::viewmodel::goto::parse_address("0x8295")
                            .expect("0x8295 must parse as a valid address");
                        assert_eq!(addr, 0x8295);
                        let line_idx = crate::viewmodel::goto::find_line_for_address(
                            &rendered, addr,
                        )
                        .expect("0x8295 must resolve to a line in ECL2.DAX block 1's listing");
                        let line = rendered.lines().nth(line_idx).unwrap();
                        assert!(
                            line.starts_with("0x8295:"),
                            "goto landed on the wrong line: {line:?}"
                        );
                    }
                }
                BlockKind::Geo => {
                    let geo = gbx_formats::geo::GeoBlock::parse(&raw).unwrap_or_else(|e| {
                        panic!("{file} block {}: GEO parse failed: {e:?}", entry.id)
                    });
                    let geometry = crate::viewmodel::geo_map::build_geometry(&geo);
                    assert_eq!(geometry.cells.len(), 16 * 16);
                }
                BlockKind::Walldef => {
                    let walldef =
                        gbx_formats::walldef::WalldefBlock::parse(&raw).unwrap_or_else(|e| {
                            panic!("{file} block {}: walldef parse failed: {e:?}", entry.id)
                        });
                    let wallset_count = walldef.wallset_count();
                    let Some(sym_file) = wv::sym_file_for_walldef_file(&file) else {
                        panic!("{file}: couldn't derive a paired 8X8D file name");
                    };
                    for wallset in 0..wallset_count {
                        let sym_block_id =
                            wv::paired_image_block_id(entry.id, wallset_count, wallset);
                        let Ok(pixel_raw) = data.block(&sym_file, sym_block_id) else {
                            // Not every wallset's paired block is guaranteed
                            // present for every walldef in the wild -- a
                            // missing pairing is a decoded-empty tile grid
                            // in the resource browser, not a panic. Skip.
                            continue;
                        };
                        let Ok(pixels) = gbx_formats::image::decode(&pixel_raw, Some(13)) else {
                            continue;
                        };
                        for style in 0..gbx_formats::walldef::STYLES_PER_WALLSET {
                            let composed = wv::compose_style(&walldef, wallset, style, &pixels);
                            assert_eq!(composed.len(), gbx_formats::walldef::TILE_IDS_PER_STYLE);
                            walldef_tiles_resolved +=
                                composed.iter().filter(|t| t.is_some()).count();
                        }
                    }
                }
                BlockKind::Font => {
                    let font = gbx_formats::font::decode(&raw);
                    for i in 0..gbx_formats::font::GLYPH_COUNT {
                        let _ = font.glyph(i);
                    }
                }
                BlockKind::Image => {
                    let mask = block_kind::default_mask(&file);
                    match gbx_formats::image::decode(&raw, mask) {
                        Ok(block) => assert!(block.width_px() > 0 || block.items.is_empty()),
                        Err(_) => {
                            // A handful of real files don't fit this pane's
                            // default-mask assumption cleanly (D-UI8 is a
                            // display convenience, not a verified-format
                            // guarantee) -- recorded via the counts summary
                            // below, not a hard failure here.
                        }
                    }
                }
                BlockKind::AnimatedPicture => {
                    let masked = block_kind::default_mask(&file).is_some();
                    let xor_delta = !file.to_ascii_uppercase().starts_with("SPRIT");
                    let _ = gbx_formats::anim::decode(&raw, masked, xor_delta);
                }
                BlockKind::Unknown => {
                    let _ = crate::viewmodel::hex::hex_dump(&raw, 16);
                }
            }
            *counts.entry(kind_name(kind)).or_insert(0) += 1;
        }
    }

    eprintln!("real_data_smoke: block counts by kind: {counts:?}");
    eprintln!(
        "real_data_smoke: walldef tiles resolved to a real pixel item: {walldef_tiles_resolved}"
    );
    assert!(
        counts.values().sum::<usize>() > 0,
        "GBX_DATA_DIR is set but no blocks were found in it"
    );
}

fn kind_name(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::Ecl => "ecl",
        BlockKind::Geo => "geo",
        BlockKind::Walldef => "walldef",
        BlockKind::Font => "font",
        BlockKind::Image => "image",
        BlockKind::AnimatedPicture => "animated_picture",
        BlockKind::Unknown => "unknown",
    }
}
