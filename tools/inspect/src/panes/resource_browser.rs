//! The resource browser pane (task deliverable 2): a tree of `GameData`
//! (file -> block ids) and per-block decoded views chosen by
//! [`crate::viewmodel::block_kind::classify`].

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use gbx_formats::game_data::GameData;

use crate::viewmodel::{block_kind, geo_map, hex, palette, walldef as wv};

/// Which (file, block, params) a cached texture was built for — rebuilt
/// only when this changes, not every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TextureKey {
    Image {
        file: String,
        block: u8,
        mask: Option<u8>,
        item: usize,
    },
    AnimFrame {
        file: String,
        block: u8,
        frame: usize,
    },
    WalldefTile {
        file: String,
        block: u8,
        wallset: usize,
        style: usize,
        tile: usize,
    },
    Font {
        file: String,
        block: u8,
    },
}

pub struct ResourceBrowserState {
    selected_file: Option<String>,
    selected_block: Option<u8>,
    zoom: f32,
    anim_frame: usize,
    wallset: usize,
    style: usize,
    texture_cache: Vec<(TextureKey, TextureHandle)>,
}

impl Default for ResourceBrowserState {
    fn default() -> Self {
        ResourceBrowserState {
            selected_file: None,
            selected_block: None,
            zoom: 4.0,
            anim_frame: 0,
            wallset: 0,
            style: 0,
            texture_cache: Vec::new(),
        }
    }
}

impl ResourceBrowserState {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &GameData) {
        egui::SidePanel::left("resource_tree")
            .resizable(true)
            .default_width(240.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.tree(ui, data);
                });
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            match (self.selected_file.clone(), self.selected_block) {
                (Some(file), Some(block)) => self.show_block(ui, data, &file, block),
                _ => {
                    ui.label("Select a block from the tree on the left.");
                }
            }
        });
    }

    fn tree(&mut self, ui: &mut egui::Ui, data: &GameData) {
        for file in data.file_names().map(str::to_string).collect::<Vec<_>>() {
            let entries = data
                .archive(&file)
                .map(|a| a.entries().to_vec())
                .unwrap_or_default();
            egui::CollapsingHeader::new(format!("{file} ({} blocks)", entries.len()))
                .id_salt(&file)
                .show(ui, |ui| {
                    for entry in &entries {
                        let kind = block_kind::classify(&file, entry.id);
                        let label = format!("block {} — {kind:?}", entry.id);
                        let selected = self.selected_file.as_deref() == Some(file.as_str())
                            && self.selected_block == Some(entry.id);
                        if ui.selectable_label(selected, label).clicked() {
                            self.selected_file = Some(file.clone());
                            self.selected_block = Some(entry.id);
                            self.anim_frame = 0;
                            self.wallset = 0;
                            self.style = 0;
                        }
                    }
                });
        }
    }

    fn show_block(&mut self, ui: &mut egui::Ui, data: &GameData, file: &str, block_id: u8) {
        let kind = block_kind::classify(file, block_id);
        ui.horizontal(|ui| {
            ui.heading(format!("{file} block {block_id}"));
            ui.label(format!("({kind:?})"));
        });
        ui.separator();

        let raw = match data.block(file, block_id) {
            Ok(bytes) => bytes,
            Err(err) => {
                ui.colored_label(Color32::RED, format!("failed to extract block: {err:?}"));
                return;
            }
        };

        match kind {
            block_kind::BlockKind::Image => self.show_image(ui, file, block_id, &raw),
            block_kind::BlockKind::AnimatedPicture => self.show_anim(ui, file, block_id, &raw),
            block_kind::BlockKind::Walldef => self.show_walldef(ui, data, file, block_id, &raw),
            block_kind::BlockKind::Geo => self.show_geo(ui, &raw),
            block_kind::BlockKind::Font => self.show_font(ui, file, block_id, &raw),
            block_kind::BlockKind::Ecl | block_kind::BlockKind::Unknown => {
                self.show_hex(ui, &raw);
            }
        }
    }

    /// A tiny FIFO texture cache keyed by [`TextureKey`]: rebuilds only on a
    /// cache miss (a new block/item/frame/tile selection), capped so
    /// browsing hundreds of walldef tiles across a session doesn't leak GPU
    /// textures forever.
    fn get_texture(
        &mut self,
        ctx: &egui::Context,
        key: TextureKey,
        build: impl FnOnce() -> ColorImage,
    ) -> TextureHandle {
        if let Some((_, tex)) = self.texture_cache.iter().find(|(k, _)| *k == key) {
            return tex.clone();
        }
        let image = build();
        let tex = ctx.load_texture(format!("{key:?}"), image, TextureOptions::NEAREST);
        if self.texture_cache.len() >= 256 {
            let _ = self.texture_cache.remove(0);
        }
        self.texture_cache.push((key, tex.clone()));
        tex
    }

    fn show_image(&mut self, ui: &mut egui::Ui, file: &str, block_id: u8, raw: &[u8]) {
        let mask = block_kind::default_mask(file);
        ui.horizontal(|ui| {
            ui.label("zoom:");
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=16.0));
        });
        let decoded = match gbx_formats::image::decode(raw, mask) {
            Ok(d) => d,
            Err(err) => {
                ui.colored_label(Color32::RED, format!("decode error: {err:?}"));
                return;
            }
        };
        ui.label(format!(
            "{}x{} px, {} item(s), mask={mask:?}",
            decoded.width_px(),
            decoded.height,
            decoded.items.len()
        ));
        let (w, h) = (decoded.width_px(), decoded.height as usize);
        let zoom = self.zoom;
        egui::ScrollArea::both().show(ui, |ui| {
            for (i, item) in decoded.items.iter().enumerate() {
                let key = TextureKey::Image {
                    file: file.to_string(),
                    block: block_id,
                    mask,
                    item: i,
                };
                let pixels = item.pixels.clone();
                let tex = self.get_texture(ui.ctx(), key, || color_image(&pixels, w, h));
                ui.label(format!("item {i}"));
                ui.add(
                    egui::Image::new(&tex)
                        .fit_to_exact_size(egui::vec2(w as f32 * zoom, h as f32 * zoom)),
                );
            }
        });
    }

    fn show_anim(&mut self, ui: &mut egui::Ui, file: &str, block_id: u8, raw: &[u8]) {
        let mask = block_kind::default_mask(file).is_some();
        // PIC/FINAL are XOR-delta encoded against frame 0; SPRIT is not
        // (design doc §1.8). Guess by filename prefix, matching the same
        // convention `block_kind::classify` used to route here.
        let upper = file.to_ascii_uppercase();
        let xor_delta = !upper.starts_with("SPRIT");
        let anim = match gbx_formats::anim::decode(raw, mask, xor_delta) {
            Ok(a) => a,
            Err(err) => {
                ui.colored_label(Color32::RED, format!("decode error: {err:?}"));
                return;
            }
        };
        if anim.frames.is_empty() {
            ui.label("0 frames");
            return;
        }
        self.anim_frame = self.anim_frame.min(anim.frames.len() - 1);
        ui.horizontal(|ui| {
            ui.label("zoom:");
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=16.0));
            if ui.button("< prev").clicked() && self.anim_frame > 0 {
                self.anim_frame -= 1;
            }
            ui.label(format!(
                "frame {}/{}",
                self.anim_frame + 1,
                anim.frames.len()
            ));
            if ui.button("next >").clicked() && self.anim_frame + 1 < anim.frames.len() {
                self.anim_frame += 1;
            }
        });
        let frame = &anim.frames[self.anim_frame];
        ui.label(format!(
            "{}x{} px, delay={} ({}ms)",
            frame.width_px(),
            frame.height,
            frame.delay,
            frame.delay * 100
        ));
        let (w, h) = (frame.width_px(), frame.height as usize);
        let key = TextureKey::AnimFrame {
            file: file.to_string(),
            block: block_id,
            frame: self.anim_frame,
        };
        let pixels = frame.pixels.clone();
        let zoom = self.zoom;
        let tex = self.get_texture(ui.ctx(), key, || color_image(&pixels, w, h));
        ui.add(
            egui::Image::new(&tex).fit_to_exact_size(egui::vec2(w as f32 * zoom, h as f32 * zoom)),
        );
    }

    fn show_walldef(
        &mut self,
        ui: &mut egui::Ui,
        data: &GameData,
        file: &str,
        block_id: u8,
        raw: &[u8],
    ) {
        let walldef = match gbx_formats::walldef::WalldefBlock::parse(raw) {
            Ok(w) => w,
            Err(err) => {
                ui.colored_label(Color32::RED, format!("decode error: {err:?}"));
                return;
            }
        };
        let wallset_count = walldef.wallset_count();
        if wallset_count == 0 {
            ui.label("0 wallsets in this block");
            return;
        }
        let Some(sym_file) = wv::sym_file_for_walldef_file(file) else {
            ui.colored_label(Color32::RED, "couldn't derive a paired 8X8D file name");
            return;
        };

        ui.horizontal(|ui| {
            ui.label("wallset:");
            ui.add(egui::Slider::new(&mut self.wallset, 0..=wallset_count - 1));
            ui.label("style:");
            ui.add(egui::Slider::new(&mut self.style, 0..=4));
            ui.label("zoom:");
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=16.0));
        });

        let sym_block_id = wv::paired_image_block_id(block_id, wallset_count, self.wallset);
        ui.label(format!(
            "paired pixel data: {sym_file} block {sym_block_id} (mask 13, target symbol set {})",
            wv::target_symbol_set(self.wallset)
        ));

        let pixel_bytes = match data.block(&sym_file, sym_block_id) {
            Ok(b) => b,
            Err(err) => {
                ui.colored_label(
                    Color32::RED,
                    format!("failed to load paired pixel block: {err:?}"),
                );
                return;
            }
        };
        let pixel_block = match gbx_formats::image::decode(&pixel_bytes, Some(13)) {
            Ok(b) => b,
            Err(err) => {
                ui.colored_label(
                    Color32::RED,
                    format!("failed to decode pixel block: {err:?}"),
                );
                return;
            }
        };

        let composed = wv::compose_style(&walldef, self.wallset, self.style, &pixel_block);
        let (tile_w, tile_h) = (pixel_block.width_px().max(1), 8usize);
        let cols = 12usize;
        let zoom = self.zoom;
        egui::ScrollArea::both().show(ui, |ui| {
            egui::Grid::new("walldef_tiles").show(ui, |ui| {
                for (i, tile) in composed.iter().enumerate() {
                    match tile {
                        Some(item) => {
                            let key = TextureKey::WalldefTile {
                                file: file.to_string(),
                                block: block_id,
                                wallset: self.wallset,
                                style: self.style,
                                tile: i,
                            };
                            let pixels = item.pixels.clone();
                            let tex = self.get_texture(ui.ctx(), key, || {
                                color_image(&pixels, tile_w, tile_h)
                            });
                            ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(
                                tile_w as f32 * zoom,
                                tile_h as f32 * zoom,
                            )));
                        }
                        None => {
                            ui.add_sized(
                                [tile_w as f32 * zoom, tile_h as f32 * zoom],
                                egui::Label::new("·"),
                            );
                        }
                    }
                    if (i + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
        });
    }

    fn show_geo(&mut self, ui: &mut egui::Ui, raw: &[u8]) {
        let geo = match gbx_formats::geo::GeoBlock::parse(raw) {
            Ok(g) => g,
            Err(err) => {
                ui.colored_label(Color32::RED, format!("decode error: {err:?}"));
                return;
            }
        };
        ui.label(format!(
            "header: {:#04X} {:#04X}",
            geo.header[0], geo.header[1]
        ));
        let geometry = geo_map::build_geometry(&geo);
        let cell_size = 20.0f32;
        let size = egui::vec2(
            geometry.grid_size as f32 * cell_size,
            geometry.grid_size as f32 * cell_size,
        );
        let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
        let origin = response.rect.min;

        for cell in &geometry.cells {
            let fill = if cell.indoor {
                Color32::from_gray(60)
            } else {
                Color32::from_gray(30)
            };
            let rect = egui::Rect::from_min_size(
                origin + egui::vec2(cell.x as f32 * cell_size, cell.y as f32 * cell_size),
                egui::vec2(cell_size, cell_size),
            );
            painter.rect_filled(rect, 0.0, fill);
            if cell.low7 != 0 {
                painter.circle_filled(rect.center(), 2.0, Color32::YELLOW);
            }
        }

        for edge in &geometry.edges {
            let color = match edge.door {
                0 => Color32::GRAY,                  // solid wall, no door
                1 => Color32::GREEN,                 // open/unlocked door
                2 => Color32::RED,                   // locked door
                _ => Color32::from_rgb(255, 140, 0), // unpickable
            };
            let base = origin + egui::vec2(edge.x as f32 * cell_size, edge.y as f32 * cell_size);
            let (p1, p2) = match edge.side {
                geo_map::Side::North => (base, base + egui::vec2(cell_size, 0.0)),
                geo_map::Side::South => (
                    base + egui::vec2(0.0, cell_size),
                    base + egui::vec2(cell_size, cell_size),
                ),
                geo_map::Side::West => (base, base + egui::vec2(0.0, cell_size)),
                geo_map::Side::East => (
                    base + egui::vec2(cell_size, 0.0),
                    base + egui::vec2(cell_size, cell_size),
                ),
            };
            painter.line_segment([p1, p2], egui::Stroke::new(2.0_f32, color));
        }

        ui.separator();
        ui.label(
            "cell shade: darker=outdoor, lighter=indoor; yellow dot: nonzero low7 (hypothesized \
             event/trigger id, docketed in gbx-formats::geo). wall color: gray=solid, \
             green=open door, red=locked, orange=unpickable.",
        );
    }

    fn show_font(&mut self, ui: &mut egui::Ui, file: &str, block_id: u8, raw: &[u8]) {
        let font = gbx_formats::font::decode(raw);
        ui.horizontal(|ui| {
            ui.label("zoom:");
            ui.add(egui::Slider::new(&mut self.zoom, 1.0..=16.0));
        });
        let cols = 16usize;
        let rows = gbx_formats::font::GLYPH_COUNT.div_ceil(cols);
        let key = TextureKey::Font {
            file: file.to_string(),
            block: block_id,
        };
        let font_ref = &font;
        let zoom = self.zoom;
        let tex = self.get_texture(ui.ctx(), key, || font_grid_image(font_ref, cols, rows));
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(
                cols as f32 * 8.0 * zoom,
                rows as f32 * 8.0 * zoom,
            )));
        });
        ui.label(format!(
            "{} glyphs, {}x{} grid",
            gbx_formats::font::GLYPH_COUNT,
            cols,
            rows
        ));
    }

    fn show_hex(&mut self, ui: &mut egui::Ui, raw: &[u8]) {
        ui.label(format!("{} bytes (hex fallback view)", raw.len()));
        let rows = hex::hex_dump(raw, 16);
        egui::ScrollArea::vertical().show_rows(
            ui,
            ui.text_style_height(&egui::TextStyle::Monospace),
            rows.len(),
            |ui, range| {
                for row in &rows[range] {
                    ui.monospace(format!(
                        "{:#06X}  {:<47}  {}",
                        row.offset, row.hex, row.ascii
                    ));
                }
            },
        );
    }
}

/// Builds an egui `ColorImage` from row-major indexed pixels
/// (`gbx_formats::image`'s `0..=15`/`16`-transparent convention).
fn color_image(pixels: &[u8], width: usize, height: usize) -> ColorImage {
    let rgba = palette::expand_rgba(pixels);
    ColorImage::from_rgba_unmultiplied([width.max(1), height.max(1)], &rgba)
}

/// Composites every font glyph into one `ColorImage` grid, `cols` wide,
/// `rows` tall, each glyph an 8x8 1bpp block (set bits -> white, MSB-first
/// per row byte — `font.rs`'s own doc comment on `MonoBitMask`).
fn font_grid_image(font: &gbx_formats::font::Font, cols: usize, rows: usize) -> ColorImage {
    let (w, h) = (cols * 8, rows * 8);
    let mut pixels = vec![0u8; w * h];
    for g in 0..gbx_formats::font::GLYPH_COUNT {
        let gx = (g % cols) * 8;
        let gy = (g / cols) * 8;
        let glyph = font.glyph(g);
        for (row, &byte) in glyph.iter().enumerate() {
            for col in 0..8 {
                let bit = (byte >> (7 - col)) & 1;
                if bit != 0 {
                    pixels[(gy + row) * w + gx + col] = 15;
                }
            }
        }
    }
    color_image(&pixels, w, h)
}
