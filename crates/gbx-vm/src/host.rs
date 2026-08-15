//! The VM's one and only borrow into engine state (D-VM4): `ScriptMemory`
//! (16-bit-addressed reads/writes through the window map), `EngineServices`
//! (synchronous calls into engine entities that aren't raw memory cells), and
//! a raw RNG accessor, unified behind a single `VmHost` trait so `step()`
//! never needs more than one `&mut` borrow into the host engine.
//!
//! `ScriptMemory` and `EngineServices` are declared here in full — this
//! module ships the *complete* `EngineServices` surface derived from
//! `docs/design/opcode-classification.md` §3 in one shot (D-VM4's explicit
//! anti-goal: "no grow-a-method-per-opcode treadmill"). Several methods are
//! not called by any opcode this session implements; they exist so the trait
//! (and any `TestHost`/engine implementation of it) never needs to regrow
//! underneath already-shipped conformance tests.
//!
//! Where a coab handler's real call shape didn't fit cleanly into the
//! classification doc's dedup listing, this module makes the concrete call —
//! documented inline — rather than leaving a vague signature. See individual
//! doc comments for the specific citations.

/// The VM address (instruction pointer at the time of the access) a
/// `ScriptMemory`/string-register access is attributed to. The engine-side
/// `ScriptMemory` implementation supplies block identity; the VM only knows
/// the pc (D-VM5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Origin {
    pub pc: u16,
}

/// A script string, exactly as captured off the wire: raw, undecoded bytes.
/// Real CotAB scripts pack strings with a compression scheme this crate
/// doesn't implement yet (`docs/design/vm-scriptmemory.md` §5 docket item 5,
/// `gbx-formats` work) — `decode.rs`'s `Arg::InlineStr` already captures raw
/// bytes for the same reason. Conformance tests author their own raw byte
/// fixtures (D10); nothing here decompresses or interprets script text.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VmString(pub Vec<u8>);

impl VmString {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }
}

/// The 16-bit-addressed memory facade scripts touch (`docs/design/vm-scriptmemory.md`
/// §3/D-VM5). Implemented by `gbx-engine` for real windows; this crate ships
/// only a mock (`test_support::MockMemory`). The VM itself intercepts the Ecl
/// window (`0x8000..=0x9DFF`) against its own resident block *before*
/// delegating here — a `ScriptMemory` impl never sees Ecl-window traffic.
pub trait ScriptMemory {
    fn read(&mut self, addr: u16, origin: Origin) -> u16;
    fn write(&mut self, addr: u16, value: u16, origin: Origin);
    fn read_byte(&mut self, addr: u16, origin: Origin) -> u8;
    fn write_byte(&mut self, addr: u16, value: u8, origin: Origin);
    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString;
    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin);
}

/// An opaque handle to an engine-tracked monster/NPC record. `gbx-vm` never
/// looks inside one — the VM only round-trips whatever `gbx-engine` hands
/// back (for call recording / future memory writes that store handles).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct MonsterHandle(pub u16);

/// An opaque handle to an engine-instantiated item record (TREASURE 0x27).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ItemHandle(pub u16);

/// A party roster index (`TeamList` in coab).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct PlayerId(pub u8);

/// PROGRAM (0x38)'s per-case outcome (`CMD_Program`, `ovr003.cs:1929-1987`).
/// The opcode's cases differ in whether the *script* keeps running, so the
/// engine-side service reports back rather than the VM guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProgramOutcome {
    /// Cases `0` and `8`: the handler returns and the script continues at the
    /// next instruction. (Case 8 never really returns in the original — it
    /// ends the process — but the engine decides that, not the VM.)
    Continue,
    /// Cases `3` and `9`: the handler ends with `CMD_Exit()`
    /// (`ovr003.cs:1980`, `:1985`), so the activation ends here.
    Exit,
}

/// LOAD MONSTER (0x0B)'s missing-`.dax`-asset path. The original hits a hard
/// `print_and_exit()` here (`ovr017.cs:836-838`) — a fatal, non-recoverable
/// modal with no graceful-degradation branch (opcode-classification.md
/// docket item 4). We model it as a recoverable `Result` at the
/// `EngineServices` boundary, but the *interpreter* surfaces it as a poisoning
/// `VmError` (see `machine.rs`) rather than silently continuing — the closest
/// faithful analogue to "the original engine cannot proceed here" without
/// actually aborting the host process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingData;

/// Synchronous calls into engine entities that aren't raw `ScriptMemory`
/// cells (D-VM4's placement rule). Declared in one shot from
/// `docs/design/opcode-classification.md` §3's draft surface; implemented by
/// `gbx-engine`, mocked by `test_support::TestHost`.
///
/// A few signatures deliberately depart from that draft's literal wording,
/// each noted below:
/// - `approach_distance`/`load_encounter_visual` drop the `(dir, y, x)`
///   params the draft listed: `sub_304B4`'s real inputs are the engine's
///   *ambient* current map position/direction (`gbl.mapDirection`,
///   `gbl.mapPosY/X`), not values any of its three callers (APPROACH, SETUP
///   MONSTER, ENCOUNTER MENU) decode from their own operands. Since the
///   engine already owns that position state (D-VM5's window map lives in
///   `gbx-engine`), round-tripping it through the VM would be pure plumbing;
///   the service reads its own ambient state.
/// - The draft's `setup_monster(dir, y, x) -> ApproachDistance` is the *same*
///   coab call (`sub_304B4`) as `approach_distance` under a second name — a
///   redundancy in the classification doc's own dedup listing, not two
///   distinct behaviors. Collapsed to the one method here; SETUP MONSTER
///   (0x0C)'s own 3 operands (sprite/max-distance/pic id) are carried by
///   `setup_monster` below instead.
/// - `roll(max) -> u8` is kept distinct from `roll_dice(size, count) -> u16`:
///   RANDOM (0x08) calls coab's `seg051.Random(max)` directly
///   (`ovr003.cs:145`), a different shape than the multi-die
///   `roll_dice`-style calls SURPRISE/TREASURE/ROB/DAMAGE make. The
///   classification doc's "(wraps VmRng per D9 — not raw RNG calls)" note
///   describes how `gbx-engine` implements both internally, not that they
///   share one signature.
pub trait EngineServices {
    // --- Character / party ---
    /// ★ LOAD CHARACTER (0x0A) in full — `sub_262E9` (`ovr003:02E9-03C8`),
    /// which is a **party-slot selector**, not a join op.
    ///
    /// `index` is the operand byte **raw**, high bit included: the mask is
    /// engine-side because everything the mask decides is engine state (there
    /// is no draw and no `ScriptMemory` write anywhere in this handler).
    ///
    /// **Correction to coab** (`ovr003.cs:186`), from the binary: coab reads
    /// `player_index > 0 && player_index < TeamList.Count ? TeamList[index] :
    /// null`, so slot **0 is never found**. The binary seeds its cursor with
    /// the list HEAD and then walks it `index` times
    /// (`ovr003:030B` `player_ptr = player_next_ptr`, `:0328`
    /// `cmp var_6,0 / jbe loc_26356`) — index 0 skips the walk and selects
    /// `TeamList[0]`, which is exactly what makes `ECL5#48 @0x809B`'s
    /// `SAVE 0, 0x7F79` … `LOAD CHARACTER 0x7F79` … `< 8` scan cover all eight
    /// slots instead of seven. Out of range still lands on a null cursor and
    /// sets `player_not_found` (`:0372`).
    fn retarget_selected_player(&mut self, index: u8);
    /// DUMP (0x3E), `CMD_Dump` (`ovr003.cs:2007-2018`): drop the selected
    /// member from the roster, re-seat the selection, and mirror it into
    /// `LastSelectedPlayer`. ADD NPC's mirror image — this is how `ECL5#48`
    /// says goodbye to Akabar.
    fn dump_selected_player(&mut self);
    fn free_current_player(&mut self, free_icon: bool, leave_party_size: bool) -> PlayerId;
    /// `area2_ptr.party_size` (`area2.field_67C`) — the count of real party
    /// members, which is *not* `TeamList.Count`: joined NPCs sit past it.
    /// DAMAGE rolls its victim over this (`roll_dice(party_size, 1)`,
    /// `ovr003:29F4-29FE`).
    fn party_size(&mut self) -> u8;
    /// `gbl.TeamList.Count` — party members **plus** joined NPCs. DAMAGE's
    /// `var_1 & 0x40` arm walks the whole list, and its victim walk stops at
    /// a null cursor rather than at `party_size`.
    fn team_size(&mut self) -> u8;
    /// `gbl.SelectedPlayer`, as a roster index. DAMAGE brackets its whole body
    /// with a save/restore of this (`ovr003:295E`, `:2C8B`).
    fn selected_player(&mut self) -> PlayerId;
    fn set_selected_player(&mut self, player: PlayerId);
    /// DAMAGE's closing scan (`ovr003:2C04-2C43`): `party_killed` is set iff
    /// **no** roster member is `in_combat`. Sets the engine cell and reports
    /// it, because the opcode's own death screen is gated on the answer.
    fn party_wipe_check(&mut self) -> bool;
    fn party_strength(&mut self) -> u8;
    /// CHECKPARTY (0x1E)'s query dispatch is a partial function in the
    /// original (docket item 7): unrecognized `query` codes silently no-op.
    fn check_party(&mut self, query: u16) -> u16;
    fn party_has_item(&mut self, item_type: u8) -> bool;
    fn find_special(&mut self, affect_type: u8) -> bool;
    fn destroy_items(&mut self, item_type: u8);
    /// ★ ROB (0x28)'s money half — `sub_31DEF` (`ovr008:1DEF-1F19`), called
    /// from `CMD_Rob` at `sub_27F76+6E`. `scale` is the **fraction kept**,
    /// which `CMD_Rob` computes as `(100 - operand2) / 100.0`
    /// (`ovr003.cs:1206`); each denomination is multiplied by it and
    /// truncated toward zero.
    ///
    /// **Correction to coab** (`ovr003.cs:1211`/`MoneySet.cs:150-160`): coab
    /// routes this through `MoneySet.ScaleAll`, whose loop runs
    /// `Copper..=Platinum` — five denominations. The binary's `sub_31DEF` has
    /// **seven** inline `Real` multiply/`Trunc` pairs, one per word of the
    /// `charStruct` money run (`copper 0xFB`, `electrum 0xFD`, `silver 0xFF`,
    /// `gold 0x101`, `platinum 0x103`, `field_105` = gems, `field_107` =
    /// jewelry). A robbed party loses its gems and jewelry too.
    ///
    /// Target-explicit like [`EngineServices::roll_saving_throw`]: `CMD_Rob`
    /// passes `gbl.SelectedPlayer` or each `TeamList` member in turn, never
    /// by implication.
    fn rob_money(&mut self, player: PlayerId, scale: f64);
    /// ★ ROB (0x28)'s item half — `sub_31F1C` (`ovr008:1F1C-1FCF`), called
    /// from `CMD_Rob` at `sub_27F76+7F`.
    ///
    /// Walks the member's item list; for each item, `chance` is first
    /// **reduced in place** by the weight ladder (`weight > 255` → `-90`,
    /// else `weight > 24` → `-50`, each clamped at 0 — `ovr008:1F43-1F81`),
    /// then `roll_dice(100, 1)` is drawn **unconditionally** and the item is
    /// lost when the roll is `<= chance`. The reduction is written back to
    /// the stack parameter (`mov [bp+arg_4], al`), so it is **cumulative down
    /// the list** within one member's call and resets for the next member —
    /// coab's closure-capturing `RemoveAll` lambda reproduces this exactly.
    fn rob_items(&mut self, player: PlayerId, chance: u8);
    fn party_surprise_check(&mut self) -> (u8, u8);

    // --- Monsters / NPCs / combat setup ---
    /// LOAD MONSTER (0x0B): bundles all 3 decoded operands (monster id,
    /// requested copy count, icon block id) — the copy-loop and icon
    /// assignment are engine-side roster bookkeeping (`ovr003.cs:238-297`),
    /// not something the VM should orchestrate one call at a time.
    fn load_monster(
        &mut self,
        monster_id: u8,
        num_copies: u8,
        icon_block_id: u8,
    ) -> Result<MonsterHandle, MissingData>;
    /// The three encounter ids SETUP MONSTER (`ovr003.cs:225-227`) and
    /// ENCOUNTER MENU (`:1253-1255`) both stash for `sub_30580` to re-read;
    /// see the trait doc comment for why this doesn't return the approach
    /// distance (that's `approach_distance`).
    ///
    /// `max_distance` is a `u16` because the two opcodes genuinely differ:
    /// SETUP MONSTER byte-casts its operand (`byte max_distance =
    /// (byte)vm_GetCmdValue(2)`, `:220`) and ENCOUNTER MENU does not
    /// (`:1254`). Each caller applies its own cast; the cell itself is the
    /// `ushort` `area2_ptr.max_encounter_distance`.
    fn setup_monster(&mut self, sprite_id: u8, max_distance: u16, pic_id: u8);
    fn clear_monsters(&mut self);
    /// ★ ADD NPC (0x36), `CMD_AddNPC` (`ovr003.cs:1769-1782`) — **the game's
    /// only join mechanism**, and (roll-credits §7.1) genuinely shipped
    /// content, not attract-mode-only: six sites recruit Alias, Dragonbait,
    /// Akabar and area 6's Rakshasa.
    ///
    /// `load_npc` (`ovr017.cs:878-890`) is gated on `party_size <= 7` and
    /// loads the record through `load_mob` — `MON{area}CHA` plus its `SPC`
    /// (innate affects) and `ITM` (carried items) companions
    /// (`ovr017.cs:824-873`) — then `AssignPlayerIconId` (`:892-896`) is what
    /// actually appends to `TeamList` and re-seats `SelectedPlayer`. The
    /// handler then overwrites the record's own morale byte with
    /// `(morale >> 1) + Control.NPC_Base` (`:1778`).
    ///
    /// A missing `.dax` is `load_mob`'s `print_and_exit` (`ovr017.cs:836`),
    /// surfaced the same way LOAD MONSTER's is.
    fn add_npc(&mut self, monster_id: u8, morale: u8) -> Result<(), MissingData>;
    /// CALL (0x2D) cases `1`/`2` — `SetupDuel(bool)`.
    fn setup_duel(&mut self, is_duel: bool);
    /// ENCOUNTER MENU (0x29)'s own first act (`ovr003.cs:1250`):
    /// `calc_group_movement(out mov_min, out mov_max)` (`ovr008.cs:1370-1398`)
    /// walks `TeamList` for each member's `movement`, doubled under `haste`
    /// and halved under `slow`, and reports **(slowest, fastest)** — in that
    /// order, matching the original's out-parameter order. Draw-free.
    ///
    /// The slowest gates the party's FLEE (`init_min >= var_407`,
    /// `ovr003.cs:1384`); the fastest gates the monsters' own flight
    /// (`var_408 > var_40A`, `:1442`).
    fn calc_group_movement(&mut self) -> (u8, u8);
    /// `sub_304B4` (`ovr008.cs:156-203`): the forward line-of-sight ray, 0..2.
    /// Reads the engine's *ambient* map position/direction (see this trait's
    /// doc comment); its wilderness arm also writes
    /// `area2_ptr.encounter_distance = 2` directly (`ovr008.cs:164`), which is
    /// the engine's business, not the VM's. Draw-free.
    fn approach_distance(&mut self) -> u8;
    /// `area2_ptr.encounter_distance` (`Classes/Area2.cs:42-43`, DataOffset
    /// `0x582` → Party-window address `0x7EC1`) — the approach range the
    /// encounter cluster carries between opcodes: SETUP MONSTER computes and
    /// clamps it (`ovr003.cs:229-233`), APPROACH decrements it
    /// (`:302-304`), ENCOUNTER MENU recomputes and re-decrements it, and
    /// `CMD_Combat` clamps it once more before placement (`:997-1001`).
    ///
    /// Exposed as a service rather than left to `ScriptMemory` because the
    /// original's handlers touch the struct field directly — a script write
    /// to `0x7EC1` lands on the same cell, so the two views agree.
    fn encounter_distance(&mut self) -> u8;
    fn set_encounter_distance(&mut self, value: u8);
    /// `sub_30580` (`ovr008.cs:220-276`), the encounter-visual dispatch —
    /// FD-34. Takes no parameters for the same reason
    /// [`EngineServices::approach_distance`] takes none: all four of the
    /// original's arguments are engine globals at every one of its eight call
    /// sites (`gbl.encounter_flags`, `area2_ptr.encounter_distance`,
    /// `gbl.pic_block_id`, `gbl.sprite_block_id`), never operand-derived.
    ///
    /// **State half only.** This mutates the flags the redraw gate reads
    /// (`encounter_flags`, `displayPlayerSprite`, `spriteChanged`,
    /// `byte_1EE96`, `can_draw_bigpic`, `mapAreaDisplay`) at *execution* time
    /// and records what to paint; the paint itself travels as
    /// [`Effect::EncounterVisual`] so it presents in queue order (D-VM3),
    /// exactly as [`EngineServices::redraw_view_gate`] pairs with
    /// [`Effect::RedrawView`].
    fn load_encounter_visual(&mut self);
    /// SPRITE OFF (0x31), `CMD_SpriteOff` (`ovr003.cs:1707-1717`): if the
    /// encounter sprite is on screen, arm `can_draw_bigpic`, clear
    /// `displayPlayerSprite`/`spriteChanged`, and report `true` so the caller
    /// emits the [`Effect::RedrawView`] that erases it. A no-op (and `false`)
    /// when no sprite is up.
    fn sprite_off(&mut self) -> bool;
    /// `gbl.byte_1EE95` — ENCOUNTER MENU sets it at its head and clears it at
    /// its tail (`ovr003.cs:1245`, `:1536`). Its only reader is `sub_30580`'s
    /// close-up gate (`ovr008.cs:257`), which is what keeps the 3D approach
    /// sprite on screen for the whole menu instead of cutting to the
    /// encounter's face the moment the distance reaches 0.
    fn set_encounter_menu_active(&mut self, active: bool);

    // --- Items / treasure ---
    /// TREASURE (0x27)'s random arm: `create_item` (`ovr022.cs:443`), which
    /// **draws** (its `randomBonus`/charge rolls) — so the interpreter calls it
    /// in the original's own order and never batches it.
    fn create_item(&mut self, item_type: u8) -> ItemHandle;
    /// TREASURE (0x27)'s table arm (`ovr003.cs:1083-1099`): decode block
    /// `block_id` of `ITEM{game_area}.DAX` and append **every** `Item`-sized
    /// record in it to the treasure pool. Draw-free. A missing file is
    /// `Logger.LogAndExit` in the original (`:1090`), surfaced as
    /// [`MissingData`] like every other hard asset stop.
    fn load_treasure_items(&mut self, block_id: u8) -> Result<(), MissingData>;
    /// TREASURE (0x27)'s first seven operands (`ovr003.cs:1076-1079`):
    /// `gbl.pooled_money.SetCoins(coin, value)` — an assignment, not an add.
    fn set_pooled_coin(&mut self, coin: u8, value: u16);
    /// SPELL (0x3B)'s not-found sentinel is a deliberate byte-underflow pair
    /// in the original (`0xFF`, `0xFF`) — replicate exactly, don't "fix" it.
    fn find_spell_in_party(&mut self, spell_id: u8) -> (u8, u8);

    // --- Combat math ---
    /// RANDOM (0x08)'s `seg051.Random(max)`. The inclusive-bound adjustment
    /// (`rand_max` incremented before rolling when `<0xFF`) happens in the
    /// opcode handler, not here — see `machine.rs`'s RANDOM implementation.
    fn roll(&mut self, max: u8) -> u8;
    fn roll_dice(&mut self, size: u8, count: u8) -> u16;
    /// `do_saving_throw(bonus, save_type, player)` — the target is an explicit
    /// argument in the original at every DAMAGE call site (`ovr003:2A57`,
    /// `:2ABE`, `:2B2F`), never `SelectedPlayer` by implication.
    fn roll_saving_throw(&mut self, player: PlayerId, bonus: u8, save_type: u8) -> bool;
    /// `sub_641DD(bonus, player)` — DAMAGE's per-victim to-hit check
    /// (`ovr003:2BC7`), likewise target-explicit.
    fn can_hit_target(&mut self, player: PlayerId, bonus: u8) -> bool;
    fn apply_damage(&mut self, player: PlayerId, damage: u16);

    // --- World / map / files ---
    fn load_3d_map(&mut self, block_id: u8);
    fn load_walldef(&mut self, set: u8, id: u8);
    fn load_bigpic(&mut self, id: u8);
    /// LOAD PIECES (0x37)'s `else` sub-branches: `gbl.setBlocks[index].Reset()`
    /// (`ovr003.cs:560-576`) — under-documented in the original M1 step-0
    /// classification pass (which traced LOAD PIECES only as far as
    /// `Load3DMap`/`LoadWalldef`/`load_bigpic`); added when `run-script`
    /// (M1 task 3) needed the full `CMD_LoadFiles` branch read to cover a
    /// real demo block.
    fn reset_wall_set(&mut self, index: u8);
    /// ★ `gbl.lastDaxBlockId` (`byte_1D5B4`) — the block `load_pic_final` last
    /// decoded (`ovr030.cs:51`), `0xFF` after a free (`:164`).
    ///
    /// LOAD FILES' bigpic reload is gated on it not being `0x50`
    /// (`ovr003.cs:528-533`). It has no `ScriptMemory` address, so unlike that
    /// gate's `inDungeon` conjunct it cannot come through `mem_read` — hence a
    /// service call. `0x50` is the city-scene PIC (`ECL1#80 @0x85E5`): while
    /// one is up, reloading the overland map underneath it is exactly what the
    /// original refuses to do.
    fn last_dax_block(&mut self) -> u8;
    fn step_game_time(&mut self, time_slot: u8, amount: u8);
    fn move_position_forward(&mut self);
    /// CALL (0x2D) case `0xAE11`/`0x4019` — `get_wall_x2`.
    fn wall_roof(&mut self) -> u8;
    /// CALL (0x2D) case `0xAE11`/`0x4019` — `getMap_wall_type`.
    fn wall_type(&mut self) -> u8;
    /// CALL (0x2D) case `0xAE11`'s consolidated redraw gate
    /// (`ovr003.cs:1848-1860`): checks the redraw-dirty flags
    /// (`spriteChanged || displayPlayerSprite || byte_1EE91 ||
    /// positionChanged || byte_1EE94`), **clears them**, and reports whether
    /// they were armed — the original's check-and-clear happens right here at
    /// execution time, while the *draw* it guards travels as
    /// [`Effect::RedrawView`] so it presents in queue order (D-VM3).
    fn redraw_view_gate(&mut self) -> bool;

    /// PROGRAM (0x38), `CMD_Program` (`ovr003.cs:1929-1987`). Every case is
    /// engine-level flow (`startGameMenu`, the end-game text, `TryEncamp`,
    /// the party-killed flag), so the operand goes over as-is and the engine
    /// reports whether the activation continues.
    fn program(&mut self, code: u8) -> ProgramOutcome;

    // --- CALL (0x2D) case 0x3201 ---
    /// Selects which sound effect CALL's `0x3201` case plays, from
    /// engine-internal state (`word_1EE76`) the VM doesn't own. Playback
    /// itself is an ordinary buffered `Effect::Sound`, not a service.
    fn call_sound_variant(&mut self) -> u8;
}

/// The binary's integer `Random(n)` primitive (D9: the VM never owns a clock
/// or RNG itself; `VmHost` exposes the engine's one PRNG through this accessor).
/// `EngineServices::roll`/`roll_dice` are the methods the interpreter's
/// opcodes actually call; `rng()` is exposed on `VmHost` per the design
/// doc's API sketch for engine-side service implementations (and any future
/// opcode) to reach the same underlying generator directly.
pub trait VmRng {
    /// The recovered integer `Random(n)` wrapper (oracle-rig §1, image
    /// `0xa55a`): draws over `0..n` — the bound is **exclusive** — and
    /// **always advances the PRNG**, including `n == 0`, which returns 0 after
    /// drawing (never a short-circuit; D-OR1(b)'s draw-parity contract). The
    /// engine implements this over `gbx-prng`, the one and only RNG (D-OR1).
    fn random(&mut self, n: u16) -> u16;
}

/// The interpreter's single host borrow (D-VM4): memory, services, and RNG
/// are views over the same engine state, so `step()`/`resume()` take exactly
/// one `&mut dyn VmHost` rather than three simultaneous borrows.
pub trait VmHost: ScriptMemory + EngineServices {
    fn rng(&mut self) -> &mut dyn VmRng;
}

/// Buffered presentation output (D-VM3): yielded by `step()`, never blocks
/// the VM. The engine must present these in yield order, fully, before any
/// subsequent `Request`'s interaction is shown.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Effect {
    /// PRINT (0x11) / PRINTCLEAR (0x12). `clear_first` distinguishes the two
    /// (`ovr003.cs:404-414`): PRINTCLEAR forces an unconditional region clear
    /// ahead of the text.
    Print { text: VmString, clear_first: bool },
    /// PRINT RETURN (0x33): cursor-newline bookkeeping only — "not itself a
    /// draw call" (opcode-classification.md 0x33 notes). The VM doesn't own
    /// text-cursor state (`textXCol`/`textYCol` are engine-internal, not a
    /// `ScriptMemory` cell), so this carries no payload; the engine applies
    /// its own cursor advance.
    PrintReturn,
    /// PICTURE (0x0E) with a real block id (`blockId != 0xFF`).
    Picture(u8),
    /// PICTURE (0x0E)'s `blockId == 0xFF` sentinel (`ovr003.cs:343-356`).
    ClearPicture,
    /// CALL (0x2D) case `0x3201`: plays whichever `SoundId` `call_sound_variant`
    /// selected.
    Sound(u8),
    /// CALL (0x2D) case `0xE804`: draw+advance one frame of the engine-owned
    /// running sprite/picture animation (`byte_1D556`) — genuinely
    /// payload-less from the VM's perspective, since that animation object
    /// isn't `ScriptMemory`-addressable state.
    AnimationFrame,
    /// CALL (0x2D) case `0xAE11`'s `RedrawView()` +
    /// `display_map_position_time()` pair (`ovr003.cs:1848-1860`), emitted
    /// only when [`EngineServices::redraw_view_gate`] reported the dirty
    /// flags armed. This is what puts the 3D view on screen *mid-vector* —
    /// `vm_init_ecl` arms `byte_1EE91` (`ovr008.cs:94`), so the first
    /// `CALL 0xAE11` a fresh block runs repaints the world before the
    /// script's own text (the amnesia intro's page 1, Bryan's 2026-08-08
    /// DOSBox side-by-side).
    ///
    /// Also carries SPRITE OFF (0x31)'s own guarded `RedrawView()`
    /// (`ovr003.cs:1712`) — same draw, same authorization shape
    /// ([`EngineServices::sprite_off`] does the check-and-clear at execution
    /// time and reports whether the draw happens).
    RedrawView,
    /// `sub_30580`'s draw half (`ovr008.cs:220-276`), authorized by
    /// [`EngineServices::load_encounter_visual`]'s execution-time state pass:
    /// the encounter view gets repainted, the `SPRIT` approach sprite blitted
    /// at its distance band, and — at distance 0 — the close-up PIC or
    /// head+body put up. Payload-less for the same reason [`Effect::RedrawView`]
    /// is: the plan lives in engine state the VM does not own.
    EncounterVisual,
    /// `PartySummary(SelectedPlayer)` (`ovr025`) — the roster panel repaint
    /// LOAD CHARACTER's dump arm (`ovr003.cs:208`), DUMP (`:2017`) and
    /// ADD NPC (`:1781`) each end with. Payload-less: the panel reads the
    /// engine's own roster.
    PartySummary,
    /// CLEAR BOX (0x3D), `CMD_ClearBox` (`ovr003.cs:1741-1754`): the whole
    /// exploration frame is rebuilt — `draw8x8_03`, the party summary, the
    /// map/position/time line, the picture window's frame-0 redraw, then the
    /// line again. One effect because the original does it as one
    /// unconditional sequence with nothing the VM can influence between the
    /// steps.
    ClearBox,
}

/// Interactions that suspend the activation awaiting a reply (D-VM3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Request {
    /// HORIZONTAL MENU (0x2B): `options` are the decoded tail strings, in
    /// script order (1-indexed register fill order, but presented 0-indexed
    /// here for the reply to select by).
    HorizontalMenu { options: Vec<VmString> },
    /// VERTICAL MENU (0x15), `CMD_VertMenu` (`ovr003.cs:663-694`): a prompt
    /// printed into the bottom text region followed by a vertical list.
    ///
    /// `prompt` is the fixed batch's single inline string (`gbl.unk_1D972[1]`,
    /// `:670`) — `press_any_key`'s text, not a menu word. `options` are the
    /// reloaded tail strings, in script order, 0-indexed like
    /// [`Request::HorizontalMenu`]'s. Answered by [`Reply::Selection`]: the
    /// engine writes `VertMenuSelect`'s returned index to the destination cell
    /// (`vm_SetMemoryValue(index, mem_loc)`, `:691`), and the index is
    /// **0-based** — the sole real occurrence (`ECL2.DAX` block 1 `@0x886A`,
    /// the tavern's drink list) feeds it straight to an `ON GOTO` with four
    /// targets.
    VerticalMenu {
        prompt: VmString,
        options: Vec<VmString>,
    },
    /// DELAY (0x3A) and CALL (0x2D) case `0xE804`'s trailing pause — both
    /// wrap `GameDelay()`/`SysDelay(game_speed_var*100)`. `game_speed_var`
    /// is an engine pacing setting the VM doesn't have a `ScriptMemory`
    /// address for, so this carries no tick count; the engine decides the
    /// real duration.
    Delay,
    /// COMBAT (0x24): the coarse request the design doc calls for — the
    /// engine owns `MainCombatLoop`/`CityShop`/`temple_shop` entirely
    /// (opcode-classification.md docket item 10, explicitly out of scope for
    /// M1 step 0).
    Combat,
    /// `seg041.DisplayAndPause(text, color)` — the prompt-line message plus a
    /// blocking `GetInputKey`. DAMAGE (0x2E) ends with one unconditionally
    /// (`ovr003:2C98-2CAD`: "press \<enter\>/\<return\> to continue", colour
    /// 15), which is distinct from the death screen's own prompt (colour 13,
    /// different words) — that one belongs to the wipe flow, not the opcode.
    PressAnyKey { text: VmString, color: u8 },
    /// ★ WHO (0x39), `CMD_Who` (`sub_28D7F`, `ovr003.cs:1757-1765`): the
    /// "who does this?" picker. The handler clears the bottom text region and
    /// calls `selectAPlayer(ref gbl.SelectedPlayer, showExit: false, prompt)`
    /// (`ovr025.cs:1527-1567`), whose loop re-prompts until the key is one of
    /// `{0x0D, 0x1B, 'E', 'S'}` — the four bits set in the Pascal set
    /// `unk_68DFA` (`ovr025:2DFA`, bytes `00 20 00 08 … 20 … 08`, i.e.
    /// elements 13, 27, 69 and 83).
    ///
    /// `showExit == false` means there is **no cancel**: the picker cannot
    /// return "nobody", it can only leave `SelectedPlayer` where the last
    /// `G`/`O` scroll put it. Whatever the script tests afterwards, it tests
    /// through `0x7D00` (`in_combat`), not through a returned index — which
    /// is why the reply carries nothing.
    ///
    /// `prompt` is string register 1, taken **unconditionally**
    /// (`gbl.unk_1D972[1]`, `:1759`) with no `Code < 0x80` guard, exactly as
    /// the handler reads it. Presented as `displayInput`'s
    /// `displayExtraString` — drawn at row `0x18` column 0 with the word
    /// `Select` starting at column `prompt.len()` (`ovr027.cs:155-163`).
    ///
    /// Appended last so the existing variant indices — which postcard
    /// encodes positionally inside `SaveState` — are untouched.
    SelectPlayer { prompt: VmString },
}

/// Replies to a suspended `Request`. `resume()` checks the reply kind
/// matches the outstanding request's kind (`VmError::ReplyMismatch` if not).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    /// A 0-indexed selection into a `Request::HorizontalMenu`'s or
    /// `Request::VerticalMenu`'s `options`.
    Selection(u8),
    Delay,
    Combat,
    /// Acknowledges a [`Request::PressAnyKey`]. Carries no key: the original's
    /// `DisplayAndPause` discards whatever was pressed.
    PressAnyKey,
    /// Acknowledges a [`Request::SelectPlayer`]. Carries nothing: `CMD_Who`
    /// writes no memory and `selectAPlayer` has already moved
    /// `gbl.SelectedPlayer` — engine state the VM does not own.
    PlayerSelected,
}

impl Reply {
    /// The `Request` kind this reply answers, for `resume()`'s legality check.
    pub(crate) fn matches(&self, request: &Request) -> bool {
        matches!(
            (self, request),
            (Reply::Selection(_), Request::HorizontalMenu { .. })
                | (Reply::Selection(_), Request::VerticalMenu { .. })
                | (Reply::Delay, Request::Delay)
                | (Reply::Combat, Request::Combat)
                | (Reply::PressAnyKey, Request::PressAnyKey { .. })
                | (Reply::PlayerSelected, Request::SelectPlayer { .. })
        )
    }
}

/// A single recorded `EngineServices`/`ScriptMemory` call, method-tagged for
/// test assertions. Built up by `test_support::TestHost`. One variant per
/// trait method (mechanical, but lets a conformance test match exactly which
/// call happened and with what arguments, rather than a stringly-typed
/// catch-all).
#[derive(Debug, Clone, PartialEq)]
pub enum RecordedCall {
    MemRead {
        addr: u16,
        origin: Origin,
    },
    MemWrite {
        addr: u16,
        value: u16,
        origin: Origin,
    },
    MemReadByte {
        addr: u16,
        origin: Origin,
    },
    MemWriteByte {
        addr: u16,
        value: u8,
        origin: Origin,
    },
    MemReadString {
        addr: u16,
        origin: Origin,
    },
    MemWriteString {
        addr: u16,
        value: VmString,
        origin: Origin,
    },

    RetargetSelectedPlayer {
        index: u8,
    },
    DumpSelectedPlayer,
    FreeCurrentPlayer {
        free_icon: bool,
        leave_party_size: bool,
    },
    PartySize,
    TeamSize,
    SelectedPlayer,
    SetSelectedPlayer {
        player: PlayerId,
    },
    PartyWipeCheck,
    PartyStrength,
    CheckParty {
        query: u16,
    },
    PartyHasItem {
        item_type: u8,
    },
    FindSpecial {
        affect_type: u8,
    },
    DestroyItems {
        item_type: u8,
    },
    RobMoney {
        player: PlayerId,
        scale: f64,
    },
    RobItems {
        player: PlayerId,
        chance: u8,
    },
    PartySurpriseCheck,

    LoadMonster {
        monster_id: u8,
        num_copies: u8,
        icon_block_id: u8,
    },
    SetupMonster {
        sprite_id: u8,
        max_distance: u16,
        pic_id: u8,
    },
    ClearMonsters,
    AddNpc {
        monster_id: u8,
        morale: u8,
    },
    SetupDuel {
        is_duel: bool,
    },
    CalcGroupMovement,
    ApproachDistance,
    EncounterDistance,
    SetEncounterDistance {
        value: u8,
    },
    LoadEncounterVisual,
    SpriteOff,
    SetEncounterMenuActive {
        active: bool,
    },

    CreateItem {
        item_type: u8,
    },
    LoadTreasureItems {
        block_id: u8,
    },
    SetPooledCoin {
        coin: u8,
        value: u16,
    },
    FindSpellInParty {
        spell_id: u8,
    },

    Roll {
        max: u8,
    },
    RollDice {
        size: u8,
        count: u8,
    },
    RollSavingThrow {
        player: PlayerId,
        bonus: u8,
        save_type: u8,
    },
    CanHitTarget {
        player: PlayerId,
        bonus: u8,
    },
    ApplyDamage {
        player: PlayerId,
        damage: u16,
    },

    Load3dMap {
        block_id: u8,
    },
    LoadWalldef {
        set: u8,
        id: u8,
    },
    LoadBigpic {
        id: u8,
    },
    ResetWallSet {
        index: u8,
    },
    StepGameTime {
        time_slot: u8,
        amount: u8,
    },
    MovePositionForward,
    WallRoof,
    WallType,
    RedrawViewGate {
        armed: bool,
    },
    Program {
        code: u8,
    },

    CallSoundVariant,
}
