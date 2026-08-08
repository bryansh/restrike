//! PICTURE (0x0E) / BIGPIC / head+body / ANIMATION presentation: the
//! viewport's picture layer.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr003.cs` `CMD_Picture` (`:312-357`) — the opcode's four arms
//!   (head/body, BIGPIC, PIC, and the `0xFF` clear) and their flag writes.
//! - `engine/ovr003.cs` `CMD_Call` case `0xE804` (`:1902-1908`) — ANIMATION:
//!   draw the running animation object's current frame, then advance it.
//! - `engine/ovr030.cs` `DrawMaybeOverlayed` (`:13-31`), `load_pic_final`
//!   (`:35-149`), `head_body` (`:163-190`), `draw_head_and_body` (`:193-204`),
//!   `load_bigpic` (`:228-239`), `draw_bigpic` (`:243-247`).
//! - `engine/seg040.cs` `draw_picture`/`OverlayBounded`/`draw_combat_picture`
//!   (`:29-31,115-123`) — the two clip windows the two draw paths use.
//! - `Classes/DaxFiles/DaxArray.cs` `NextFrame`/`CurrentPicture` (`:34-47`) —
//!   the 1-based `curFrame` cursor and its wrap.
//! - `engine/ovr029.cs` `RedrawView` (`:10-49`) and `engine/ovr025.cs`
//!   `LoadPic` (`:1398-1455`) — who repaints the viewport, and who does not
//!   put the picture back afterwards.
//!
//! ## Where the picture lives, and when it goes away (the design decision)
//!
//! The original draws a picture *once*, into a retained EGA framebuffer, and
//! leaves it there until something repaints those cells. `RedrawView` is that
//! something: it calls `Draw3dWorld` (`ovr029.cs:39`), which paints the 88x88
//! viewport a picture exactly fills. So "when does the picture go away" is
//! really "when does the original redraw the view", and this slice answered
//! it from real content rather than from the flag algebra:
//!
//! **Every picture-bearing vector in the shipped scripts destroys its own
//! picture before it exits.** In Tilverton's `ECL2` block 1 — the amnesia
//! intro, the innkeeper, the tavern — each scene's tail is either `PICTURE
//! 0xFF` (`0x9441`, whose branch calls `RedrawView`) or the consolidated
//! redraw gate `CALL 0xAE11` (`0x9CB4`, `0x9452`, …), which fires because
//! `CMD_Picture` itself set `spriteChanged` (`ovr003.cs:320,1848`). A
//! disassembly renders that call as `CALL mem=0x2E10`; the operand is the raw
//! word and `0x2E10 - 0x7FFF == 0xAE11` (`ovr003.cs:1835`). A picture is
//! therefore visible for exactly as long as its scene, and the walk loop
//! always gets the 3D view back.
//!
//! That is precisely what this engine's *existing* unconditional
//! `crate::corridor::redraw_view` at every world-menu-visible point already
//! produces — so the documented simplification (`vmhost.rs`'s
//! `clear_redraw_flags` doc comment) needed no change, and the five-flag gate
//! did not need modeling. `redraw_view` calls [`PictureLayer::hide`] as its
//! first act, which is the truthful statement of what its own pixels do.
//!
//! What [`PictureLayer`] buys, then, is that the displayed picture is
//! **state**, not just pixels: serializable (D-SAVE3 names the active
//! animation's frame index as save-relevant), so `Engine::assemble` can put a
//! mid-scene picture back with [`compose_into`] after a restore, and the
//! ANIMATION opcode has a frame cursor to advance. [`compose`] is idempotent
//! and never mutates the decoded asset (see [`draw_maybe_overlayed`]'s fade
//! note), which D-UI4 requires and which the original itself leans on —
//! `displayInput`'s wait loop re-blits the running animation's current frame
//! on every iteration while a picture is up (`ovr027.cs:185-199`, with
//! `CMD_HorizontalMenu` passing `useOverlay = spriteChanged && byte_1EE8D`,
//! `ovr003.cs:730-738`).

use crate::draw::{apply_recolor_dithered, blit_image, Clip, FADE_RECOLOR};
use crate::framebuffer::Framebuffer;
use crate::shell::FlowCtx;
use crate::symbols::SymbolSets;
use crate::vmhost::HaltRecord;
use gbx_formats::anim::{self, AnimatedPicture};
use gbx_formats::game_data::GameData;
use gbx_formats::image::{self, ImageBlock};

/// `blockId >= 0x78` selects the BIGPIC arm (`ovr003.cs:328`).
pub const BIGPIC_FIRST_BLOCK: u8 = 0x78;

/// The PIC / head anchor cell, `DrawMaybeOverlayed(…, 3, 3)`
/// (`ovr003.cs:337`, `ovr008.cs:216`). An 88×88 `PIC` block lands exactly on
/// the 3D viewport (cells 3..=13, inside `draw8x8_03`'s inner frame at
/// 2..=14) — verified against the real `PIC*.DAX` set, every block 88×88.
const PIC_ROW: usize = 3;
const PIC_COL: usize = 3;
/// `draw_head_and_body(true, rowY, colX)` puts the body five cells below the
/// head (`ovr030.cs:199`). Real `HEAD*.DAX` blocks are 88×40 and `BODY*.DAX`
/// 88×48 — head at row 3 + body at row 8 tile the same 88×88 viewport a PIC
/// fills.
const BODY_ROW: usize = PIC_ROW + 5;

/// `draw_bigpic`'s anchor (`ovr030.cs:246`). Real `BIGPIC*.DAX` blocks are
/// 304×120, so cell (1,1) puts them at pixels (8,8)..(312,128) — inside the
/// outer frame, above `DrawFrame_WildernessMap`'s row-16 divider.
const BIGPIC_ROW: usize = 1;
const BIGPIC_COL: usize = 1;

/// `gbl.area_ptr.picture_fade` (`Classes/Area1.cs:197-198`, `DataOffset`
/// `0x3FE`).
///
/// The Area window's `DataOffset`→ECL-address mapping is
/// `DataOffset = (addr - 0x4B00) * 2` (`ovr008.cs:715` passes
/// `0x6A00 + location * 2` to `Area1::field_6A00_Set`, and `0x6A00 + 0x4B00 *
/// 2` is `0x10000`, i.e. zero in the `ushort` the setter masks to). Confirmed
/// three ways: `inDungeon` is `DataOffset 0x1CC` and coab's own check is
/// `(location - 0x4B00) == 0xE6` (`ovr008.cs:704`); the two addresses that set
/// `byte_1EE94` (`ovr008.cs:698`, `0xFD`/`0xFE`) land on `DataOffset`
/// `0x1FA`/`0x1FC` = `outdoor_sky_colour`/`indoor_sky_colour`; and real ECL
/// content writes this very cell — `ECL3` block 18 does `SAVE 0xFF, 0x4CFF`
/// immediately before a `PICTURE` (`0x95E5`), which is exactly a
/// "fade the next picture" script.
///
/// (`crate::corridor`'s `OUTDOOR_SKY_COLOUR_ADDR`/`INDOOR_SKY_COLOUR_ADDR`
/// predate this derivation and use the un-halved offsets; correcting them
/// changes the 3D view's sky colour and is deliberately **not** part of this
/// slice — docketed in `docs/fidelity-docket.md`.)
const PICTURE_FADE_ADDR: u16 = 0x4B00 + 0x1FF;

/// What the viewport's picture layer currently shows — one of `CMD_Picture`'s
/// three draw arms, or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Shown {
    /// No picture: the 3D view (or whatever else composes the viewport) owns
    /// the cells.
    #[default]
    Nothing,
    /// `blockId < 0x78`, `HeadBlockId == 0xFF` (`ovr003.cs:335-338`).
    Pic,
    /// `blockId >= 0x78` (`ovr003.cs:328-333`).
    BigPic,
    /// `HeadBlockId != 0xFF` (`ovr003.cs:340`).
    HeadBody,
}

/// The picture layer's persistent state: a faithful mirror of the original's
/// own picture globals, plus the one thing the original keeps in pixels
/// ([`Shown`]).
///
/// Mirrors `gbl.lastDaxBlockId` + `gbl.byte_1D556.curFrame` (the running
/// animation object), `gbl.bigpic_block_id`, and `gbl.head_block_id` /
/// `gbl.body_block_id`. D-SAVE3 names "the active animation's frame index"
/// as pixel-affecting state a save must carry — [`PictureLayer::anim_frame`]
/// is it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PictureLayer {
    /// `gbl.lastDaxBlockId` for `gbl.lastDaxFile == "PIC"`: the block loaded
    /// into the running animation object. `None` = freed
    /// (`DaxArrayFreeDaxBlocks`, `ovr030.cs:152-162`).
    pub anim_block: Option<u8>,
    /// `DaxArray.curFrame` — **1-based**, `0` when nothing is loaded
    /// (`DaxArray.cs:19-31`). `load_pic_final` sets it to 1 (`ovr030.cs:66`).
    pub anim_frame: u8,
    /// `gbl.bigpic_block_id`.
    pub bigpic_block: Option<u8>,
    /// `gbl.head_block_id` / `gbl.body_block_id` (`ovr008.cs:212-213`);
    /// `0xFF` = none loaded.
    pub head_block: u8,
    pub body_block: u8,
    pub shown: Shown,
}

impl Default for PictureLayer {
    fn default() -> Self {
        PictureLayer {
            anim_block: None,
            anim_frame: 0,
            bigpic_block: None,
            head_block: 0xFF,
            body_block: 0xFF,
            shown: Shown::Nothing,
        }
    }
}

impl PictureLayer {
    /// Stops showing anything, leaving the load bookkeeping alone — the
    /// original's picture-clearing paths repaint the viewport (`RedrawView`,
    /// `LoadPic`) without freeing `byte_1D556`/`bigpic_dax`.
    pub fn hide(&mut self) {
        self.shown = Shown::Nothing;
    }

    /// `DaxArray.NextFrame` (`DaxArray.cs:34-41`): `curFrame++`, wrapping to
    /// `1` past `numFrames`. A no-op when nothing is loaded.
    fn next_frame(&mut self, num_frames: u8) {
        if self.anim_frame == 0 || num_frames == 0 {
            return;
        }
        self.anim_frame += 1;
        if self.anim_frame > num_frames {
            self.anim_frame = 1;
        }
    }
}

/// The engine's resident decoded picture assets — the cache half of the
/// original's globals (`gbl.byte_1D556`, `gbl.bigpic_dax`, `gbl.headX_dax`,
/// `gbl.bodyX_dax`), keyed exactly as the original keys them
/// (`lastDaxFile`+`lastDaxBlockId`, `bigpic_block_id`, `current_head_id`,
/// `current_body_id` — each load is skipped when the key already matches).
///
/// Not serialized: it is derivable from [`PictureLayer`] + `GameData`, and
/// re-decodes on demand after a restore.
#[derive(Debug, Default)]
pub struct PictureCache {
    pic: Option<(u8, AnimatedPicture)>,
    bigpic: Option<(u8, ImageBlock)>,
    head: Option<(u8, ImageBlock)>,
    body: Option<(u8, ImageBlock)>,
}

/// Why a picture could not be put on screen. Loud, never silent: the original
/// stops on a modal `DisplayAndPause("PIC not found", 14)` (`ovr030.cs:60`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureLoadError(pub String);

impl PictureCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// `load_pic_final(ref byte_1D556, 0, block_id, "PIC")` (`ovr030.cs:35`):
    /// the animated-picture container, unmasked (`masked == 0`) with the
    /// `PIC`/`FINAL` XOR-delta scheme on. Re-decodes only when the block id
    /// changes, matching the original's `lastDaxFile`/`lastDaxBlockId` guard
    /// (`ovr030.cs:37-38`).
    ///
    /// `AnimationsOn == false` collapsing the block to one frame
    /// (`ovr030.cs:71-74`) is not modeled: this engine has no animations-off
    /// setting yet, and the original ships it on.
    fn pic(
        &mut self,
        data: &GameData,
        area: u8,
        block: u8,
    ) -> Result<&AnimatedPicture, PictureLoadError> {
        if self.pic.as_ref().map(|(id, _)| *id) != Some(block) {
            let file = format!("PIC{area}.DAX");
            let raw = data
                .block(&file, block)
                .map_err(|e| PictureLoadError(format!("{file} block {block}: {e:?}")))?;
            let decoded = anim::decode(&raw, false, true)
                .map_err(|e| PictureLoadError(format!("{file} block {block}: {e:?}")))?;
            if decoded.frames.is_empty() {
                return Err(PictureLoadError(format!(
                    "{file} block {block}: zero frames"
                )));
            }
            self.pic = Some((block, decoded));
        }
        Ok(&self.pic.as_ref().expect("just populated").1)
    }

    /// `load_bigpic` (`ovr030.cs:228-239`): `LoadDax(0, 0, block_id,
    /// "bigpic{area}")` — the plain 4bpp container, unmasked.
    fn bigpic(
        &mut self,
        data: &GameData,
        area: u8,
        block: u8,
    ) -> Result<&ImageBlock, PictureLoadError> {
        if self.bigpic.as_ref().map(|(id, _)| *id) != Some(block) {
            self.bigpic = Some((
                block,
                load_image(data, &format!("BIGPIC{area}.DAX"), block)?,
            ));
        }
        Ok(&self.bigpic.as_ref().expect("just populated").1)
    }

    /// `head_body`'s head half (`ovr030.cs:167-177`): `HEAD{area}.DAX`,
    /// re-loaded only when `current_head_id` changes.
    fn head(
        &mut self,
        data: &GameData,
        area: u8,
        block: u8,
    ) -> Result<&ImageBlock, PictureLoadError> {
        if self.head.as_ref().map(|(id, _)| *id) != Some(block) {
            self.head = Some((block, load_image(data, &format!("HEAD{area}.DAX"), block)?));
        }
        Ok(&self.head.as_ref().expect("just populated").1)
    }

    /// `head_body`'s body half (`ovr030.cs:179-189`): `BODY{area}.DAX`.
    fn body(
        &mut self,
        data: &GameData,
        area: u8,
        block: u8,
    ) -> Result<&ImageBlock, PictureLoadError> {
        if self.body.as_ref().map(|(id, _)| *id) != Some(block) {
            self.body = Some((block, load_image(data, &format!("BODY{area}.DAX"), block)?));
        }
        Ok(&self.body.as_ref().expect("just populated").1)
    }

    /// `DaxArrayFreeDaxBlocks(gbl.byte_1D556)` (`ovr030.cs:152-162`), which
    /// `load_bigpic` does first thing (`ovr030.cs:230`).
    fn free_pic(&mut self) {
        self.pic = None;
    }

    /// The loaded animation's frame count (`DaxArray.numFrames`), or 0.
    fn pic_frame_count(&self) -> u8 {
        self.pic
            .as_ref()
            .map_or(0, |(_, a)| a.frames.len().min(255) as u8)
    }
}

fn load_image(data: &GameData, file: &str, block: u8) -> Result<ImageBlock, PictureLoadError> {
    let raw = data
        .block(file, block)
        .map_err(|e| PictureLoadError(format!("{file} block {block}: {e:?}")))?;
    let decoded = image::decode(&raw, None)
        .map_err(|e| PictureLoadError(format!("{file} block {block}: {e:?}")))?;
    if decoded.items.is_empty() {
        return Err(PictureLoadError(format!(
            "{file} block {block}: zero items"
        )));
    }
    Ok(decoded)
}

/// `DrawMaybeOverlayed` (`ovr030.cs:13-31`), reduced to what it actually is:
/// the same blit under one of two clip windows, plus an optional fade
/// recolor.
///
/// - `picture_fade > 0` → the block is recolored toward palette 12 through
///   `FADE_RECOLOR` with `DaxBlock.Recolor`'s 1-in-4 dither.
///   **Divergence, deliberate:** the original recolors the *cached* block in
///   place, so repeated draws fade it progressively; this applies the pass to
///   a scratch copy so [`compose`] stays idempotent (D-UI4) and a save/restore
///   cannot land mid-fade. `apply_recolor_dithered`'s dither is a position
///   hash, not a PRNG draw (FD-28) — so it is also draw-neutral, which the
///   in-place version could not be. The only exerciser in the shipped game is
///   the ending sequence (`ovr019.cs:510-514`) plus `ECL3` block 18's own
///   `SAVE 0xFF, 0x4CFF`; docketed.
/// - `useOverlay` → `OverlayBounded` + `DrawOverlay`, whose net effect is
///   `draw_combat_picture`'s tighter clip ([`Clip::OVERLAY`],
///   `seg040.cs:29-31,115-118`); otherwise `draw_picture`'s full-canvas clip
///   (`seg040.cs:120-123`). `OverlayBounded(…, rowY - 1, colX - 1)` then
///   `draw_combat_picture(…, rowY, colX)` is a `-1`/`+1` round trip
///   (`ovr030.cs:26`), so the anchor cell is the same in both paths.
fn draw_maybe_overlayed(
    fb: &mut Framebuffer,
    img: &Ready,
    (row, col): (usize, usize),
    use_overlay: bool,
    picture_fade: u16,
) {
    let (pixels, width, height) = (&img.0, img.1, img.2);
    let clip = if picture_fade > 0 || use_overlay {
        Clip::OVERLAY
    } else {
        Clip::FULL
    };
    if picture_fade > 0 {
        let mut faded = pixels.clone();
        apply_recolor_dithered(&mut faded, &FADE_RECOLOR);
        blit_image(fb, &faded, width, height, row, col, clip, None, None);
    } else {
        blit_image(fb, pixels, width, height, row, col, clip, None, None);
    }
}

/// `gbl.area_ptr.picture_fade`'s current value (see [`PICTURE_FADE_ADDR`]).
fn picture_fade(ctx: &FlowCtx) -> u16 {
    ctx.vm_memory.raw_word(PICTURE_FADE_ADDR).unwrap_or(0)
}

/// Records an asset failure the way every other content failure in this
/// engine is recorded: loudly, in the halt log, never a panic and never a
/// silent blank (`shell.rs`'s `begin_chain` sets the precedent).
fn diagnose(ctx: &mut FlowCtx, what: &str, err: PictureLoadError) {
    ctx.vm_memory.halts.push(HaltRecord {
        pc: 0,
        opcode: 0x0E,
        description: format!("{what} failed to load: {}", err.0),
    });
}

/// `CMD_Picture`'s `blockId != 0xFF` half (`ovr003.cs:318-342`).
///
/// The three arms are chosen exactly as the original chooses them, and the
/// flag writes are the original's own — including `can_draw_bigpic = false`
/// *after* the BIGPIC draw (`ovr003.cs:332`), which M2 transcribed as
/// `can_draw_bigpic = true` on every `Picture` effect by reading the `0xFF`
/// branch's line (`:348`) into the non-`0xFF` branch.
pub fn cmd_picture(ctx: &mut FlowCtx, block_id: u8, head_block_id: u8) {
    ctx.vm_memory.sprite_changed = true; // `:320`

    if head_block_id != 0xFF {
        // `:340` — an encounter head is active, so the block id is a BODY id
        // and the pair draws together. This is the arm essentially every
        // CotAB script takes: of the 57 `PICTURE` ops across `ECL1..6`, every
        // one in Tilverton's `ECL2` block 1 (the amnesia intro, the tavern)
        // is immediately preceded by `SAVE <head>, 0x7EE1` — `HeadBlockId`.
        set_and_draw_head_body(ctx, block_id, head_block_id);
        return;
    }

    ctx.vm_memory.byte_1ee8d = true; // `:324`

    if block_id >= BIGPIC_FIRST_BLOCK {
        // `:328-332`
        ctx.pictures.free_pic(); // `load_bigpic`'s own first act, `ovr030.cs:230`
        ctx.state.picture.anim_block = None;
        ctx.state.picture.anim_frame = 0;
        ctx.state.picture.bigpic_block = Some(block_id);
        ctx.state.picture.shown = Shown::BigPic;
        ctx.vm_memory.can_draw_bigpic = false; // `:332`
    } else {
        // `:335-337`
        ctx.state.picture.anim_block = Some(block_id);
        ctx.state.picture.anim_frame = 1; // `load_pic_final`, `ovr030.cs:66`
        ctx.state.picture.shown = Shown::Pic;
    }
    compose(ctx);
}

/// `set_and_draw_head_body` (`ovr008.cs:208-217`).
fn set_and_draw_head_body(ctx: &mut FlowCtx, body_id: u8, head_id: u8) {
    ctx.vm_memory.byte_1ee8d = false; // `:210`
    ctx.state.picture.head_block = head_id; // `:212`
    ctx.state.picture.body_block = body_id; // `:213`
    ctx.state.picture.shown = Shown::HeadBody;
    compose(ctx);
}

/// `CMD_Picture`'s `blockId == 0xFF` half (`ovr003.cs:343-356`).
///
/// The gate is the original's verbatim: a state pair plus "something is
/// actually on screen". `encounter_flags` are not modeled here (they live in
/// the encounter-visual path, `sub_30580`, which is still record-not-draw) —
/// their clear at `:354-355` is a no-op for this slice and is noted, not
/// faked.
pub fn cmd_clear_picture(ctx: &mut FlowCtx) {
    use crate::shell::GameState;
    let gate = (ctx.state.last_game_state != GameState::DungeonMap
        || ctx.state.game_state == GameState::DungeonMap)
        && ctx.vm_memory.sprite_changed;
    if gate {
        ctx.vm_memory.can_draw_bigpic = true; // `:347`
                                              // `RedrawView()` (`:348`) repaints the viewport — and nothing puts
                                              // the picture back, so the layer stops showing first.
        ctx.state.picture.hide();
        crate::corridor::redraw_view(ctx);
        ctx.vm_memory.sprite_changed = false; // `:350`
        ctx.vm_memory.byte_1ee8d = true; // `:352`
    }
}

/// `CMD_Call` case `0xE804` (`ovr003.cs:1902-1908`): draw the running
/// animation object's **current** frame, then advance the cursor. The first
/// ANIMATION after a `PICTURE` therefore re-draws frame 0 (the frame
/// `CMD_Picture` already drew) before moving on — `load_pic_final` leaves
/// `curFrame == 1` and `CurrentPicture()` is `frames[curFrame - 1]`.
///
/// The trailing `GameDelay()` is the VM's own `Request::Delay`
/// (`gbx_vm::Effect::AnimationFrame` is emitted with it), not this
/// function's business.
pub fn animation_frame(ctx: &mut FlowCtx) {
    if ctx.state.picture.anim_frame == 0 {
        return; // nothing loaded: `CurrentPicture()` would be null
    }
    // The draw is `DrawMaybeOverlayed(CurrentPicture(), true, 3, 3)` — the
    // PIC arm's own geometry, regardless of what `shown` says, because the
    // original draws straight from `byte_1D556`.
    ctx.state.picture.shown = Shown::Pic;
    compose(ctx);
    let count = ctx.pictures.pic_frame_count();
    ctx.state.picture.next_frame(count);
}

/// One decoded picture ready to blit: pixels + geometry, lifted out of the
/// cache so the cache borrow ends before the framebuffer is touched.
type Ready = (Vec<u8>, usize, usize);

/// [`compose_into`] against a live [`FlowCtx`]; a failure stops the layer
/// showing (so the next compose does not re-fail) and is reported loudly.
pub fn compose(ctx: &mut FlowCtx) {
    let fade = picture_fade(ctx);
    let result = compose_into(
        ctx.fb,
        ctx.symbols,
        ctx.data,
        ctx.game_area,
        &ctx.state.picture,
        ctx.pictures,
        fade,
    );
    if let Err((what, err)) = result {
        ctx.state.picture.hide();
        diagnose(ctx, what, err);
    }
}

/// Paints whatever [`PictureLayer::shown`] says onto `fb`, without a
/// [`FlowCtx`] — `Engine::assemble` composes a restored save's picture this
/// way, before any tick has run.
///
/// Idempotent and free of PRNG draws: the frontier guard and the reel smoke
/// are the standing referees for the second half of that.
#[allow(clippy::too_many_arguments)]
pub fn compose_into(
    fb: &mut Framebuffer,
    symbols: &SymbolSets,
    data: &GameData,
    game_area: u8,
    layer: &PictureLayer,
    cache: &mut PictureCache,
    fade: u16,
) -> Result<(), (&'static str, PictureLoadError)> {
    match layer.shown {
        Shown::Nothing => {}
        Shown::Pic => {
            let Some(block) = layer.anim_block else {
                return Ok(());
            };
            let index = layer.anim_frame.saturating_sub(1) as usize;
            let anim = cache
                .pic(data, game_area, block)
                .map_err(|e| ("PICTURE", e))?;
            // A restored save can name a frame past the end if the asset
            // changed underneath it; clamp rather than panic.
            let f = anim.frames.get(index).unwrap_or(&anim.frames[0]);
            let img = (f.pixels.clone(), f.width_px(), f.height as usize);
            draw_maybe_overlayed(fb, &img, (PIC_ROW, PIC_COL), true, fade);
        }
        Shown::BigPic => {
            let Some(block) = layer.bigpic_block else {
                return Ok(());
            };
            let img = cache
                .bigpic(data, game_area, block)
                .map(first_item)
                .map_err(|e| ("BIGPIC", e))?;
            // `draw_bigpic` (`ovr030.cs:243-247`) draws
            // `DrawFrame_WildernessMap` first, then calls `draw_picture`
            // directly — never `DrawMaybeOverlayed`, so no fade and the full
            // clip. A missing symbol set is not a reason to skip the picture:
            // the frame is decoration, the picture is content.
            let _ = crate::frames::draw_frame_wilderness_map(fb, symbols);
            draw_maybe_overlayed(fb, &img, (BIGPIC_ROW, BIGPIC_COL), false, 0);
        }
        Shown::HeadBody => {
            // `draw_head_and_body(true, 3, 3)` (`ovr030.cs:193-204`):
            // `useOverlay == false` for both halves, head first.
            if layer.head_block != 0xFF {
                let img = cache
                    .head(data, game_area, layer.head_block)
                    .map(first_item)
                    .map_err(|e| ("HEAD", e))?;
                draw_maybe_overlayed(fb, &img, (PIC_ROW, PIC_COL), false, fade);
            }
            if layer.body_block != 0xFF {
                let img = cache
                    .body(data, game_area, layer.body_block)
                    .map(first_item)
                    .map_err(|e| ("BODY", e))?;
                draw_maybe_overlayed(fb, &img, (BODY_ROW, PIC_COL), false, fade);
            }
        }
    }
    Ok(())
}

fn first_item(img: &ImageBlock) -> Ready {
    (
        img.items[0].pixels.clone(),
        img.width_px(),
        img.height as usize,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_frame_wraps_one_based_like_daxarray() {
        let mut layer = PictureLayer {
            anim_block: Some(7),
            anim_frame: 1,
            ..PictureLayer::default()
        };
        layer.next_frame(3);
        assert_eq!(layer.anim_frame, 2);
        layer.next_frame(3);
        assert_eq!(layer.anim_frame, 3);
        layer.next_frame(3);
        assert_eq!(layer.anim_frame, 1, "wraps to 1, not 0");
    }

    #[test]
    fn next_frame_is_a_no_op_with_nothing_loaded() {
        let mut layer = PictureLayer::default();
        layer.next_frame(4);
        assert_eq!(layer.anim_frame, 0);
    }

    #[test]
    fn a_single_frame_animation_never_leaves_frame_one() {
        let mut layer = PictureLayer {
            anim_block: Some(1),
            anim_frame: 1,
            ..PictureLayer::default()
        };
        for _ in 0..5 {
            layer.next_frame(1);
        }
        assert_eq!(layer.anim_frame, 1);
    }

    #[test]
    fn hide_keeps_the_load_bookkeeping() {
        let mut layer = PictureLayer {
            anim_block: Some(9),
            anim_frame: 2,
            head_block: 4,
            body_block: 4,
            shown: Shown::HeadBody,
            ..PictureLayer::default()
        };
        layer.hide();
        assert_eq!(layer.shown, Shown::Nothing);
        assert_eq!(layer.anim_block, Some(9));
        assert_eq!(layer.anim_frame, 2);
        assert_eq!(layer.head_block, 4);
    }

    /// Local tier (D10 — no game data in the repo): every `PICTURE` operand
    /// the shipped ECL scripts can reach resolves to an asset that decodes,
    /// through the same cache the engine uses. The head/body arm is checked
    /// against `HEAD*`/`BODY*`, the plain arm against `PIC*`.
    #[test]
    fn local_tier_real_picture_assets_decode_at_the_documented_geometry() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!("GBX_DATA_DIR not set -- skipping the real-asset picture tier");
            return;
        };
        let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
            .expect("GBX_DATA_DIR must be readable");
        let mut cache = PictureCache::new();

        // Tilverton (area 2): the acceptance scenes' own ids.
        for (head, body) in [(3u8, 3u8), (4, 4), (5, 5), (9, 6), (2, 2), (8, 2)] {
            let head_dims = {
                let h = cache.head(&data, 2, head).expect("HEAD2 block");
                (h.width_px(), h.height as usize)
            };
            let body_dims = {
                let b = cache.body(&data, 2, body).expect("BODY2 block");
                (b.width_px(), b.height as usize)
            };
            assert_eq!(head_dims, (88, 40), "HEAD2 block {head} geometry");
            assert_eq!(body_dims, (88, 48), "BODY2 block {body} geometry");
            // Head + body tile the 88x88 viewport a PIC fills: 40 + 48 == 88,
            // and the body sits exactly five 8-px cells lower.
            assert_eq!(head_dims.1 + body_dims.1, 88);
            assert_eq!((BODY_ROW - PIC_ROW) * 8, head_dims.1);
        }

        // The plain PIC arm (`ECL4` block 33 reaches these without a head).
        for block in [0x25u8, 0x26, 0x27, 0x28, 0x29, 0x23, 0x09] {
            let anim = cache.pic(&data, 4, block).expect("PIC4 block");
            let f0 = &anim.frames[0];
            assert_eq!(
                (f0.width_px(), f0.height),
                (88, 88),
                "PIC4 block {block:#x}"
            );
        }

        // BIGPIC: 304x120 at cell (1,1) stays inside the 320x200 canvas.
        for (area, block) in [(1u8, 0x79u8), (1, 0x7B), (2, 0x78), (6, 0x7A)] {
            let img = cache.bigpic(&data, area, block).expect("BIGPIC block");
            assert_eq!((img.width_px(), img.height), (304, 120));
            assert!(BIGPIC_COL * 8 + img.width_px() <= crate::framebuffer::WIDTH);
            assert!(BIGPIC_ROW * 8 + img.height as usize <= crate::framebuffer::HEIGHT);
        }
    }

    /// The cache keys the way the original keys: a repeat load of the same
    /// block id must not re-decode (`ovr030.cs:37-38,233`).
    #[test]
    fn the_cache_reuses_a_block_and_replaces_a_different_one() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            return;
        };
        let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir)).unwrap();
        let mut cache = PictureCache::new();
        cache.head(&data, 2, 3).unwrap();
        let first = cache.head.as_ref().unwrap().1.items[0].pixels.as_ptr();
        cache.head(&data, 2, 3).unwrap();
        assert_eq!(
            cache.head.as_ref().unwrap().1.items[0].pixels.as_ptr(),
            first,
            "same block id must not re-decode"
        );
        cache.head(&data, 2, 4).unwrap();
        assert_eq!(cache.head.as_ref().unwrap().0, 4);
    }
}
