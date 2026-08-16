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

/// ★ The `PIC{area}` block whose presence suppresses the overland's own
/// presentation — `gbl.lastDaxBlockId == 0x50`, tested in four places:
/// `RedrawView` (`ovr029.cs:12`), `MapCursor`'s four blink gates
/// (`ovr027.cs:167,178,317,333`), `CMD_LoadFiles`' bigpic reload
/// (`ovr003.cs:530`), and `LoadPic`'s `Shop`/`WildernessMap` arms
/// (`ovr025.cs:1414,1444`).
///
/// **What it actually is** (roll-credits D-S7e, corrected from the door's
/// "demo-title guard"): `lastDaxBlockId` is set by `load_pic_final`
/// (`ovr030.cs:51`) to whatever `PIC`/`SPRIT` block was last decoded, and the
/// only shipped script that loads block `0x50` is `ECL1#80`'s own city
/// preamble — `@0x85D8` runs `SAVE 1,[0x4BE6]` / `CLEAR BOX` /
/// `SAVE 0,[0x4BE6]` / **`PICTURE 0x50`** and then prints "YOU ARE IN
/// <city>. WHAT PLACE WILL YOU VISIT?". So the guard reads "a city scene owns
/// the viewport": while it does, the Dalelands map underneath must not be
/// repainted and its cursor must not blink. `PICTURE 0x79` on the way out of
/// the city menu (`@0x86C7`) puts the map back and, by moving
/// `lastDaxBlockId` off `0x50`, re-arms all four guards.
pub const CITY_SCENE_PIC_BLOCK: u8 = 0x50;

/// `gbl.lastDaxBlockId`'s "nothing loaded" value — `DaxArrayFreeDaxBlocks`
/// (`ovr030.cs:164`) writes it, and `InitFirst` leaves the byte at 0 only
/// until the first load.
pub const NO_DAX_BLOCK: u8 = 0xFF;

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
/// (FD-31 resolved: `crate::corridor`'s sky cells and `vmhost.rs`'s clock
/// cluster now use this same halved mapping.)
pub(crate) const PICTURE_FADE_ADDR: u16 = 0x4B00 + 0x1FF;

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
    /// ★ `Show3DSprite` (`ovr030.cs:215-226`): the masked `SPRIT{area}`
    /// approach sprite overlaid on the 3D view, at the distance band
    /// [`PictureLayer::sprite_frame`] names. Not one of `CMD_Picture`'s arms —
    /// only `sub_30580` puts this up.
    Sprite,
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
    /// ★ The `SPRIT{area}` block `sub_30580` loaded into `byte_1D556`
    /// (`ovr008.cs:235`), and the **1-based** frame `Show3DSprite` blits —
    /// `encounter_distance + 1` (`ovr008.cs:248`), so `1..=3`.
    ///
    /// Real `SPRIT*` blocks carry exactly three frames, largest first: block 1
    /// of `SPRIT2` is 32×80, 24×65, 16×57 at cells (4,1), (4,1), (5,1). The
    /// "distance bands" are those three pre-rendered sizes — there is no
    /// runtime scaling anywhere in the original's draw path.
    pub sprite_block: Option<u8>,
    pub sprite_frame: u8,
    pub shown: Shown,
    /// ★ `gbl.lastDaxBlockId` itself (`byte_1D5B4`), the single cell
    /// [`CITY_SCENE_PIC_BLOCK`]'s four guards read (roll-credits D-S7e).
    ///
    /// Modeled once rather than derived, because in the original it is ONE
    /// byte shared by every `load_pic_final` call — `PIC` closeups and
    /// `SPRIT` approach sets alike (`ovr030.cs:35-51`) — while [`anim_block`]
    /// and [`sprite_block`] are this engine's deliberately *split* caches (see
    /// [`PictureCache::sprite`]'s note). Derivation from the pair would be
    /// wrong exactly when the two disagree, which is the encounter path.
    ///
    /// [`NO_DAX_BLOCK`] (`0xFF`) is "nothing loaded", written by every free
    /// (`ovr030.cs:164`).
    ///
    /// [`anim_block`]: PictureLayer::anim_block
    /// [`sprite_block`]: PictureLayer::sprite_block
    pub last_dax_block: u8,
    /// ★ **FD-32's mechanism.** How many `DaxBlock.Recolor` fade passes the
    /// currently-shown block has taken.
    ///
    /// The original has no such counter: `DrawMaybeOverlayed` recolors the
    /// **cached** block in place whenever `picture_fade > 0` (`ovr030.cs:19-22`),
    /// so the progress *is* the cache's own pixels and every redraw of the
    /// same block fades it further. Ours keeps the decoded asset pristine
    /// (D-UI4: composition is idempotent, and a save must never carry
    /// half-faded art), so the progress lives here instead and
    /// [`crate::draw::apply_recolor_dithered`] applies `step` passes in one
    /// go. `0` and `1` both mean "one pass" — the original always recolors at
    /// least once before a fade-armed blit.
    ///
    /// **Advanced by the ANIMATION cadence** — [`animation_frame`] (the
    /// `0xE804` opcode), [`menu_wait_animation`] (`displayInput`'s wait loop,
    /// whose fade arm advances every iteration, `ovr027.cs:185-198`), and the
    /// ending's own `ShowAnimation` loop (`crate::ending`) — which is exactly
    /// FD-33's precedent, one redraw per tick.
    ///
    /// **Reset when the decoded asset changes**, mirroring `load_pic_final`'s
    /// own cache guard (`ovr030.cs:37-38`): loading a *different* block
    /// re-decodes and the fresh pixels are unfaded; re-showing the *same*
    /// block keeps fading it, exactly as the original's retained `DaxArray`
    /// does.
    ///
    /// ★ **`#[serde(skip)]` — it resets on load, and that is the faithful
    /// answer, not a shortcut.** The original's fade progress lives in the
    /// decoded `DaxBlock`, and `SaveGame` writes none of it: a restored
    /// session re-decodes every picture from the DAX files, pristine, while
    /// `picture_fade` itself (an `Area1` cell) *is* saved. So the original
    /// restoring mid-dissolve resumes with an unfaded picture and a still-armed
    /// fade — which is precisely `fade_step == 0` with `picture_fade > 0`.
    /// Skipping it also keeps `SAVE_FORMAT_VERSION` where it is.
    #[serde(skip)]
    pub fade_step: u16,
}

impl Default for PictureLayer {
    fn default() -> Self {
        PictureLayer {
            anim_block: None,
            anim_frame: 0,
            bigpic_block: None,
            head_block: 0xFF,
            body_block: 0xFF,
            sprite_block: None,
            sprite_frame: 0,
            shown: Shown::Nothing,
            last_dax_block: NO_DAX_BLOCK,
            fade_step: 0,
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

    /// ★ FD-32: one more `DaxBlock.Recolor` pass on the shown block — what the
    /// original gets for free by recoloring its cache in place. Clamped at
    /// [`crate::draw::FADE_STEP_MAX`], past which nothing eligible is left.
    pub fn advance_fade(&mut self) {
        self.fade_step = self
            .fade_step
            .saturating_add(1)
            .min(crate::draw::FADE_STEP_MAX);
    }

    /// The step [`compose_into`] should apply: `0` still means one pass,
    /// because `DrawMaybeOverlayed` recolors before *every* fade-armed blit.
    fn effective_fade_step(&self) -> u16 {
        self.fade_step.max(1)
    }

    /// `load_pic_final`'s cache guard (`ovr030.cs:37-38`): a *different* block
    /// id re-decodes, and fresh pixels are unfaded. Same id, same (possibly
    /// already-faded) cache.
    fn reset_fade_if_block_changed(&mut self, was: Option<u8>, now: Option<u8>) {
        if was != now {
            self.fade_step = 0;
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
    /// ★ The masked `SPRIT{area}` approach-sprite set. In the original this
    /// shares `byte_1D556` with `pic` — one `DaxArray`, one
    /// `(lastDaxFile, lastDaxBlockId)` key — so loading a close-up `PIC`
    /// *destroys* the sprite frames. Ours keeps them in their own slot, which
    /// is strictly more persistent than the original and invisible: the only
    /// consumer of the sprite frames is `Show3DSprite`, and `sub_30580` gates
    /// that behind `encounter_flags[1] == false` — the same flag the close-up
    /// sets when it overwrites the array (`ovr008.cs:222,264,271`). Decoding
    /// is deterministic and draw-free, so an extra cache hit cannot move a
    /// pixel or a PRNG draw.
    sprite: Option<(u8, AnimatedPicture)>,
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

    /// ★ `load_pic_final(ref byte_1D556, 1, sprite_block_id, "SPRIT")`
    /// (`ovr008.cs:235`): **masked** (mask colour 0 → transparency-16,
    /// `DaxBlock.SetMaskedColor`) and **not** XOR-delta'd —
    /// `is_pic_or_final` is false for `SPRIT`, so every frame is independent
    /// (`ovr030.cs:53,109`).
    ///
    /// The masked load also runs a second pass the `PIC` load does not:
    /// `Recolor(false, transparentNewColors, transparentOldColors)`
    /// (`ovr030.cs:129-132`), whose only non-identity entry is **13 → 0**
    /// ([`crate::draw::TRANSPARENT_RECOLOR`]). Order matters and is easy to
    /// get backwards: the mask is applied first, at decode
    /// (`DaxToPicture(0, masked, …)`, `:127`), so colour 0 has already become
    /// transparency-16 by the time 13 is folded onto 0 — the recoloured
    /// pixels are opaque black, **not** transparent.
    fn sprite(
        &mut self,
        data: &GameData,
        area: u8,
        block: u8,
    ) -> Result<&AnimatedPicture, PictureLoadError> {
        if self.sprite.as_ref().map(|(id, _)| *id) != Some(block) {
            let file = format!("SPRIT{area}.DAX");
            let raw = data
                .block(&file, block)
                .map_err(|e| PictureLoadError(format!("{file} block {block}: {e:?}")))?;
            let mut decoded = anim::decode(&raw, true, false)
                .map_err(|e| PictureLoadError(format!("{file} block {block}: {e:?}")))?;
            if decoded.frames.is_empty() {
                return Err(PictureLoadError(format!(
                    "{file} block {block}: zero frames"
                )));
            }
            for frame in decoded.frames.iter_mut() {
                crate::draw::apply_recolor(&mut frame.pixels, &crate::draw::TRANSPARENT_RECOLOR);
            }
            self.sprite = Some((block, decoded));
        }
        Ok(&self.sprite.as_ref().expect("just populated").1)
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
    /// `load_bigpic` does first thing (`ovr030.cs:230`) and `locked_door`'s
    /// ungated tail does once per walk-loop iteration (`ovr015.cs:587`).
    pub(crate) fn free_pic(&mut self) {
        self.pic = None;
    }

    /// The loaded animation's frame count (`DaxArray.numFrames`), or 0.
    fn pic_frame_count(&self) -> u8 {
        self.pic
            .as_ref()
            .map_or(0, |(_, a)| a.frames.len().min(255) as u8)
    }

    /// The resident PIC animation's per-frame `delay` for the 1-based frame
    /// cursor (`DaxArray.CurrentDelay()`) — original units, ×100 ms each.
    fn pic_frame_delay(&self, frame: u8) -> Option<u32> {
        let (_, anim) = self.pic.as_ref()?;
        anim.frames
            .get(frame.saturating_sub(1) as usize)
            .map(|f| f.delay)
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
///   `FADE_RECOLOR` with `DaxBlock.Recolor`'s 1-in-4 dither, `fade_step` times
///   (★ FD-32, resolved). The original recolors the *cached* block in place so
///   repeated draws fade it progressively; ours applies the accumulated pass
///   count to a scratch copy, which keeps [`compose`] idempotent (D-UI4) and
///   the decoded asset pristine, while still *progressing* —
///   [`PictureLayer::fade_step`] is what moves, driven by the ANIMATION
///   cadence. `apply_recolor_dithered`'s dither is a position hash, not a PRNG
///   draw (FD-28), so the whole model is draw-neutral, which the in-place
///   version could not be. The exercisers in the shipped game are the ending
///   sequence (`ovr019.cs:510-514`) and `ECL3` block 18's own
///   `SAVE 0xFF, 0x4CFF`.
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
    fade_step: u16,
) {
    let (pixels, width, height) = (&img.0, img.1, img.2);
    let clip = if picture_fade > 0 || use_overlay {
        Clip::OVERLAY
    } else {
        Clip::FULL
    };
    if picture_fade > 0 {
        let mut faded = pixels.clone();
        apply_recolor_dithered(&mut faded, &FADE_RECOLOR, fade_step);
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
        // That free is also what disarms the city-scene guard: it writes
        // `lastDaxBlockId = 0xFF` (`ovr030.cs:164`). This is exactly how
        // `ECL1#80`'s `@0x86C7 PICTURE 0x79` re-arms the overland's cursor
        // after a city menu closes.
        ctx.state.picture.last_dax_block = NO_DAX_BLOCK;
        let was = ctx.state.picture.bigpic_block;
        ctx.state.picture.bigpic_block = Some(block_id);
        ctx.state.picture.shown = Shown::BigPic;
        // ★ FD-32: a re-decode is a pristine block (`ovr030.cs:233`).
        ctx.state
            .picture
            .reset_fade_if_block_changed(was, Some(block_id));
        ctx.vm_memory.can_draw_bigpic = false; // `:332`
    } else {
        // `:335-337`
        let was = ctx.state.picture.anim_block;
        ctx.state.picture.anim_block = Some(block_id);
        ctx.state.picture.anim_frame = 1; // `load_pic_final`, `ovr030.cs:66`
        ctx.state.picture.last_dax_block = block_id; // `ovr030.cs:51`
        ctx.state.picture.shown = Shown::Pic;
        // ★ FD-32: `load_pic_final` re-decodes only when the block id moved
        // (`ovr030.cs:37-38`) — so only then is the fade progress discarded.
        ctx.state
            .picture
            .reset_fade_if_block_changed(was, Some(block_id));
    }
    compose(ctx);
}

/// `set_and_draw_head_body` (`ovr008.cs:208-217`).
fn set_and_draw_head_body(ctx: &mut FlowCtx, body_id: u8, head_id: u8) {
    ctx.vm_memory.byte_1ee8d = false; // `:210`
    let was = (ctx.state.picture.head_block, ctx.state.picture.body_block);
    ctx.state.picture.head_block = head_id; // `:212`
    ctx.state.picture.body_block = body_id; // `:213`
    ctx.state.picture.shown = Shown::HeadBody;
    // ★ FD-32: `head_body` re-loads each half only when its id changed
    // (`ovr030.cs:169,181`).
    if was != (head_id, body_id) {
        ctx.state.picture.fade_step = 0;
    }
    compose(ctx);
}

/// `CMD_Picture`'s `blockId == 0xFF` half (`ovr003.cs:343-356`).
///
/// The gate is the original's verbatim: a state pair plus "something is
/// actually on screen" — the latter now including `displayPlayerSprite`
/// (`:346`), the flag `sub_30580` raises, which is why an encounter's
/// `PICTURE 0xFF` erases the approach sprite even when no `CMD_Picture` ever
/// set `spriteChanged`. Both `encounter_flags` clear unconditionally
/// (`:354-355`), outside the gate.
pub fn cmd_clear_picture(ctx: &mut FlowCtx) {
    use crate::shell::GameState;
    let gate = (ctx.state.last_game_state != GameState::DungeonMap
        || ctx.state.game_state == GameState::DungeonMap)
        && (ctx.vm_memory.sprite_changed || ctx.vm_memory.display_player_sprite);
    if gate {
        ctx.vm_memory.can_draw_bigpic = true; // `:347`
                                              // `RedrawView()` (`:348`) repaints the viewport — and nothing puts
                                              // the picture back, so the layer stops showing first.
        ctx.state.picture.hide();
        crate::corridor::redraw_view(ctx);
        ctx.vm_memory.sprite_changed = false; // `:350`
        ctx.vm_memory.display_player_sprite = false; // `:351`
        ctx.vm_memory.byte_1ee8d = true; // `:352`
    }
    ctx.state.encounter_flags = [false; 2]; // `:354-355`
}

/// ★ One `sub_30580` dispatch's **draw** half, computed by
/// [`encounter_visual_state`] at execution time and replayed by
/// [`encounter_visual`] at present time (D-VM3's ordering rule — the flags the
/// `0xAE11` gate reads must move when the instruction runs, the pixels must
/// land in queue order behind whatever `PRINT`s preceded them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EncounterVisualPlan {
    /// `can_draw_bigpic = true; RedrawView();` — either the auto-map
    /// dismissal (`ovr008.cs:226-231`) or the "sprite already up, repaint
    /// before re-blitting it one band closer" arm (`:241-244`).
    pub redraw_view: bool,
    /// `Show3DSprite(byte_1D556, encounter_distance + 1)` (`:248`):
    /// `(SPRIT block id, 1-based frame)`.
    pub sprite: Option<(u8, u8)>,
    /// The distance-0 close-up: [`Shown::Pic`] (`:263-266`) or
    /// [`Shown::HeadBody`] (`:270`).
    pub close_up: Option<Shown>,
}

/// ★ **`sub_30580`** (`ovr008.cs:220-276`) — FD-34's state half.
///
/// A two-flag state machine over `gbl.encounter_flags`: `[0]` = "the `SPRIT`
/// approach-sprite set is loaded and on screen", `[1]` = "a close-up owns the
/// picture window". Both start false at every block entry (`vm_init_ecl`,
/// `:96-97`), so the first dispatch of an encounter loads the sprite set, and
/// each later one (APPROACH, ENCOUNTER MENU's ADVANCE) repaints the view and
/// re-blits one band closer — until the distance reaches 0, when the close-up
/// takes over the window and `[1]` latches.
///
/// Two guards on the close-up are easy to miss and both matter:
/// - `byte_1EE95` (`:257`) suppresses it for the whole of ENCOUNTER MENU, which
///   is what keeps the 3D sprite visible behind COMBAT/WAIT/FLEE/ADVANCE
///   instead of cutting to the face the instant the distance hits 0.
/// - `byte_1EE96 != HeadBlockId` (`:253`) is a change detector, not a
///   dirty flag: it lets a *second* close-up through when the script has
///   swapped `HeadBlockId` since the first, and it starts at 0 — a valid head
///   id — because the original never initializes it.
pub fn encounter_visual_state(
    state: &mut crate::shell::EngineState,
    vm: &mut crate::vmhost::VmMemoryState,
) {
    use crate::shell::GameState;
    let mut plan = EncounterVisualPlan::default();
    let distance = state.encounter_distance;

    if !state.encounter_flags[1] {
        // `:222`
        if !state.encounter_flags[0] {
            // `:224`
            if state.area_map_shown {
                // `:226` — an encounter cancels the auto-map
                state.area_map_shown = false; // `:228`
                vm.can_draw_bigpic = true; // `:229`
                plan.redraw_view = true; // `:230`
            }
            if vm.in_dungeon() {
                // `:233` — the RAW `area_ptr.inDungeon` cell, per FD-37
                state.picture.sprite_block = Some(state.sprite_block_id); // `:235`
                                                                          // `load_pic_final(…, "SPRIT")` moves the shared cell too
                                                                          // (`ovr030.cs:51`), which is why it is modeled once here and
                                                                          // not derived from the split caches.
                state.picture.last_dax_block = state.sprite_block_id;
                state.encounter_flags[0] = true; // `:236`
                vm.display_player_sprite = true; // `:237`
            }
        } else {
            vm.can_draw_bigpic = true; // `:242`
            plan.redraw_view = true; // `:243`
        }
        if state.game_state == GameState::DungeonMap {
            // `:246`
            plan.sprite = Some((state.sprite_block_id, distance + 1)); // `:248`
        }
    }

    // `:252-253`
    if !state.encounter_flags[1] || vm.byte_1ee96 != state.head_block_id {
        // `:255-257`
        if distance == 0 && state.game_state == GameState::DungeonMap && !vm.byte_1ee95 {
            vm.byte_1ee96 = state.head_block_id; // `:259`
            vm.sprite_changed = true; // `:260`
            if state.head_block_id == 0xFF {
                // `:263` load_pic_final(…, 0, pic_block_id, "PIC")
                state.picture.anim_block = Some(state.pic_block_id);
                state.picture.anim_frame = 1; // `load_pic_final`, `ovr030.cs:66`
                state.picture.last_dax_block = state.pic_block_id; // `ovr030.cs:51`
                state.encounter_flags[1] = true; // `:264`
                plan.close_up = Some(Shown::Pic); // `:266`
            } else {
                // `:270` set_and_draw_head_body(pic_block_id, HeadBlockId) —
                // note `pic_block_id` is the BODY id on this arm.
                vm.byte_1ee8d = false; // `set_and_draw_head_body`, `ovr008.cs:210`
                state.picture.head_block = state.head_block_id; // `:212`
                state.picture.body_block = state.pic_block_id; // `:213`
                state.encounter_flags[1] = true; // `:271`
                plan.close_up = Some(Shown::HeadBody);
                vm.byte_1ee8d = false; // `:272` — set twice, verbatim
            }
        }
    }

    vm.push_encounter_visual_plan(plan);
}

/// ★ [`gbx_vm::Effect::EncounterVisual`]: `sub_30580`'s draw half, replayed in
/// the order the original performs it — repaint, then sprite, then close-up
/// over the top.
///
/// The two composes are not redundant. At `SETUP MONSTER <sprite>, 0, <pic>`
/// (real content: `ECL2#2 @0x87D7`) a single dispatch both blits the nearest
/// sprite band and then paints the close-up over it, and the sprite is masked
/// while the close-up is not — so the intermediate frame is genuinely visible
/// in the original's retained framebuffer.
pub fn encounter_visual(ctx: &mut FlowCtx) {
    let Some(plan) = ctx.vm_memory.pop_encounter_visual_plan() else {
        return;
    };
    if plan.redraw_view {
        // `RedrawView()` repaints the viewport, which is what erases the
        // previous band's sprite. It hides the picture layer as its first act,
        // so everything below re-establishes what is shown.
        crate::corridor::redraw_view(ctx);
    }
    if let Some((block, frame)) = plan.sprite {
        ctx.state.picture.sprite_block = Some(block);
        ctx.state.picture.sprite_frame = frame;
        ctx.state.picture.shown = Shown::Sprite;
        compose(ctx);
    }
    if let Some(shown) = plan.close_up {
        ctx.state.picture.shown = shown;
        compose(ctx);
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
    // ★ FD-32: `DrawMaybeOverlayed` recolored the cache on the way past, so
    // the NEXT draw of this block is one pass further along.
    if picture_fade(ctx) > 0 {
        ctx.state.picture.advance_fade();
    }
    let count = ctx.pictures.pic_frame_count();
    ctx.state.picture.next_frame(count);
}

/// ★ FD-33 discharged for the menu wait: `displayInput`'s wait-loop
/// animation (`ovr027.cs:184-198`). While a prompt waits with `useOverlay`
/// armed — `CMD_HorizontalMenu` computes it as `spriteChanged && byte_1EE8D`
/// (`ovr003.cs:730-738`), both flags untouched while the VM is suspended, so
/// a per-tick recompute equals the original's at-open snapshot — and the
/// running PIC animation loaded (`byte_1D556.curFrame > 0`), each loop
/// iteration re-blits the current frame and advances it once
/// `CurrentDelay() * 100` ms have passed (per-frame delay from the DAX
/// data). At [`crate::input::TICK_HZ`] = 60, 100 ms = 6 ticks — the
/// GameDelay conversion combat already uses. `waited` is the loop's
/// transient `timeStart`, owned by the parked gate and never serialized
/// (the original's is a stack local).
///
/// ★ **FD-32's other half, now closed too.** The loop's gate is
/// `(picture_fade != 0 || useOverlay) && curFrame > 0` (`ovr027.cs:185-186`),
/// and its advance condition is `elapsed >= delay || picture_fade != 0`
/// (`:192-193`) — so while a fade is armed the wait loop steps the animation
/// **every iteration, ignoring the frame delay**, which is what dissolves a
/// picture behind a menu. Ours advances the fade step and the frame cursor
/// once per tick on that arm.
pub fn menu_wait_animation(ctx: &mut FlowCtx, waited: &mut u32) {
    let use_overlay = ctx.vm_memory.sprite_changed && ctx.vm_memory.byte_1ee8d;
    let fade = picture_fade(ctx);
    if (fade == 0 && !use_overlay) || ctx.state.picture.anim_frame == 0 {
        return;
    }
    // `DrawMaybeOverlayed(byte_1D556.CurrentPicture(), useOverlay, 3, 3)` —
    // straight from the animation object, the PIC arm's geometry.
    ctx.state.picture.shown = Shown::Pic;
    compose(ctx);
    if fade != 0 {
        // `:192-193` — no delay test at all on this arm.
        ctx.state.picture.advance_fade();
        let count = ctx.pictures.pic_frame_count();
        ctx.state.picture.next_frame(count);
        *waited = 0;
        return;
    }
    let Some(delay) = ctx.pictures.pic_frame_delay(ctx.state.picture.anim_frame) else {
        return;
    };
    *waited = waited.saturating_add(ctx.dt_ticks);
    if *waited >= delay.saturating_mul(6) {
        let count = ctx.pictures.pic_frame_count();
        ctx.state.picture.next_frame(count);
        *waited = 0;
    }
}

/// ★ `ShowAnimation`'s opening (`ovr019.cs:413-420`):
/// `load_pic_final(ref animation, 2, block_id, "PIC")`, then
/// `OverlayBounded(frames[0], …)` + `DrawOverlay()` — i.e. the block becomes
/// the running animation object at frame 1 and its first frame goes straight
/// on the glass. `crate::ending` is the only caller in the shipped game.
pub fn show_animation_begin(ctx: &mut FlowCtx, block: u8) {
    let was = ctx.state.picture.anim_block;
    ctx.state.picture.anim_block = Some(block);
    ctx.state.picture.anim_frame = 1; // `load_pic_final`, `ovr030.cs:66`
    ctx.state.picture.last_dax_block = block; // `ovr030.cs:51`
    ctx.state.picture.shown = Shown::Pic;
    ctx.state
        .picture
        .reset_fade_if_block_changed(was, Some(block));
    compose(ctx);
}

/// ★ One iteration of `ShowAnimation`'s `do … while (loop_count != num_loops)`
/// (`ovr019.cs:422-441`), tick-paced.
///
/// The original re-blits `CurrentPicture()` **every** iteration and advances
/// the frame cursor only once `CurrentDelay() * (game_speed_var + 3)` have
/// elapsed on `time01()`'s centisecond clock (`seg041.cs:309-314`) — so a
/// one-frame block (like the ending's dissolve, `PIC6` block `0x4D`) simply
/// redraws itself, which is exactly what makes a fade-armed loop progress.
/// `num_loops` counts wraps of the cursor.
///
/// Returns `true` when the loop has run out of wraps.
pub fn show_animation_tick(ctx: &mut FlowCtx, waited: &mut u32, loops_left: &mut u16) -> bool {
    // `DrawMaybeOverlayed(animation.CurrentPicture(), true, row_y, col_x)`.
    ctx.state.picture.shown = Shown::Pic;
    compose(ctx);
    // ★ FD-32: the original's redraw recolored the cache; ours steps the
    // counter that stands in for it.
    if picture_fade(ctx) > 0 {
        ctx.state.picture.advance_fade();
    }
    if *loops_left == 0 {
        return true;
    }
    let Some(delay) = ctx.pictures.pic_frame_delay(ctx.state.picture.anim_frame) else {
        return true; // nothing loaded — do not spin forever
    };
    *waited = waited.saturating_add(ctx.dt_ticks);
    if *waited < frame_ticks(delay, crate::vmhost::game_speed(ctx.vm_memory)) {
        return false;
    }
    *waited = 0;
    let before = ctx.state.picture.anim_frame;
    let count = ctx.pictures.pic_frame_count();
    ctx.state.picture.next_frame(count);
    if ctx.state.picture.anim_frame <= before {
        // `curFrame > numFrames` → back to 1, `loop_count++` (`:433-437`).
        *loops_left -= 1;
    }
    *loops_left == 0
}

/// `ShowAnimation`'s frame period in engine ticks: `CurrentDelay() *
/// (game_speed_var + 3)` **centiseconds** (`ovr019.cs:427` against
/// `time01()`'s 1/100 s clock), converted at [`crate::input::TICK_HZ`] = 60 —
/// 1 cs = 0.6 ticks. Never zero, or the loop would spin.
fn frame_ticks(delay_units: u32, game_speed: u8) -> u32 {
    let centiseconds = delay_units.saturating_mul(u32::from(game_speed) + 3);
    (centiseconds * 6 / 10).max(1)
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
        ctx.game_area(),
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
/// are the standing referees for the second half of that. Idempotency survives
/// FD-32's fade because the *step* is an input here
/// ([`PictureLayer::fade_step`]), never something this function advances.
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
    let step = layer.effective_fade_step();
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
            draw_maybe_overlayed(fb, &img, (PIC_ROW, PIC_COL), true, fade, step);
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
            draw_maybe_overlayed(fb, &img, (BIGPIC_ROW, BIGPIC_COL), false, 0, 0);
        }
        Shown::Sprite => {
            // ★ `Show3DSprite(byte_1D556, sprite_index)` (`ovr030.cs:215-226`).
            // The frame is the block's OWN pre-rendered distance band, blitted
            // at its own header anchor: `OverlayBounded(block, 1, 0,
            // y_pos + 3 - 1, x_pos + 3 - 1)` → `draw_combat_picture(block,
            // y_pos + 3, x_pos + 3, 0)` (`seg040.cs:29-31`, the -1/+1 round
            // trip), i.e. the overlay clip window ([`Clip::OVERLAY`]).
            let Some(block) = layer.sprite_block else {
                return Ok(());
            };
            let anim = cache
                .sprite(data, game_area, block)
                .map_err(|e| ("SPRITE", e))?;
            // `Show3DSprite`'s own two guards, in order: the 1..=3 range check
            // (`:217-220`, a `LogAndExit` in the original — loud here too, via
            // the caller's diagnose) and the null-frame skip (`:222`).
            let Some(f) = layer
                .sprite_frame
                .checked_sub(1)
                .and_then(|i| anim.frames.get(i as usize))
            else {
                return Err((
                    "SPRITE",
                    PictureLoadError(format!(
                        "SPRIT{game_area} block {block}: frame {} of {}",
                        layer.sprite_frame,
                        anim.frames.len()
                    )),
                ));
            };
            let img = (f.pixels.clone(), f.width_px(), f.height as usize);
            let (row, col) = (f.y_pos as usize + PIC_ROW, f.x_pos as usize + PIC_COL);
            // Masked: transparency-16 pixels leave the 3D view showing through
            // — [`Framebuffer::set_pixel`]'s universal `< 16` guard, which is
            // `Display.SetPixel3`'s own (`Classes/Display.cs:163-173`). No
            // `no_draw` colour is needed on top of it.
            blit_image(
                fb,
                &img.0,
                img.1,
                img.2,
                row,
                col,
                Clip::OVERLAY,
                None,
                None,
            );
        }
        Shown::HeadBody => {
            // `draw_head_and_body(true, 3, 3)` (`ovr030.cs:193-204`):
            // `useOverlay == false` for both halves, head first.
            if layer.head_block != 0xFF {
                let img = cache
                    .head(data, game_area, layer.head_block)
                    .map(first_item)
                    .map_err(|e| ("HEAD", e))?;
                draw_maybe_overlayed(fb, &img, (PIC_ROW, PIC_COL), false, fade, step);
            }
            if layer.body_block != 0xFF {
                let img = cache
                    .body(data, game_area, layer.body_block)
                    .map(first_item)
                    .map_err(|e| ("BODY", e))?;
                draw_maybe_overlayed(fb, &img, (BODY_ROW, PIC_COL), false, fade, step);
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

    /// ★ FD-32: the counter climbs one pass per redraw, clamps, and a step of
    /// 0 still composes ONE pass (the original recolors before every
    /// fade-armed blit).
    #[test]
    fn the_fade_counter_advances_clamps_and_never_composes_zero_passes() {
        let mut layer = PictureLayer::default();
        assert_eq!(layer.fade_step, 0);
        assert_eq!(layer.effective_fade_step(), 1, "0 still means one pass");
        for expect in 1..=5u16 {
            layer.advance_fade();
            assert_eq!(layer.fade_step, expect);
            assert_eq!(layer.effective_fade_step(), expect);
        }
        for _ in 0..500 {
            layer.advance_fade();
        }
        assert_eq!(layer.fade_step, crate::draw::FADE_STEP_MAX, "clamped");
    }

    /// ★ FD-32: the fade resets exactly when the original re-decodes — a
    /// different block id — and survives a redraw of the same one.
    #[test]
    fn the_fade_resets_only_when_the_decoded_block_changes() {
        let mut layer = PictureLayer {
            anim_block: Some(0x4D),
            fade_step: 9,
            ..PictureLayer::default()
        };
        layer.reset_fade_if_block_changed(Some(0x4D), Some(0x4D));
        assert_eq!(layer.fade_step, 9, "same block: the cache is still faded");
        layer.reset_fade_if_block_changed(Some(0x4D), Some(0x4A));
        assert_eq!(layer.fade_step, 0, "a re-decode is pristine");
    }

    /// ★ FD-32's serde decision, pinned: the counter is presentation state and
    /// **resets on load**. The original's fade progress lives in the decoded
    /// `DaxBlock`, which `SaveGame` never writes; a restored session
    /// re-decodes pristine art with `picture_fade` still armed. Being
    /// `#[serde(skip)]` is also what keeps `SAVE_FORMAT_VERSION` at 9.
    #[test]
    fn the_fade_step_is_presentation_state_and_does_not_survive_a_save() {
        let layer = PictureLayer {
            anim_block: Some(0x4D),
            anim_frame: 1,
            shown: Shown::Pic,
            fade_step: 17,
            ..PictureLayer::default()
        };
        let bytes = postcard::to_allocvec(&layer).expect("serialize");
        let back: PictureLayer = postcard::from_bytes(&bytes).expect("deserialize");
        assert_eq!(back.fade_step, 0, "a restore starts the dissolve over");
        assert_eq!(back.anim_block, Some(0x4D), "everything else round-trips");
        assert_eq!(back.anim_frame, 1);
        assert_eq!(back.shown, Shown::Pic);
        // And the encoding carries no bytes for it: a layer that differs ONLY
        // in `fade_step` serializes identically, which is why no golden moves.
        let pristine = PictureLayer {
            fade_step: 0,
            ..layer
        };
        assert_eq!(bytes, postcard::to_allocvec(&pristine).expect("serialize"));
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
