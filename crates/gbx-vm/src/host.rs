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

/// LOAD CHARACTER (0x0A) with an out-of-range/absent index —
/// `gbl.player_not_found`, not an exception, in the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotFound;

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
    fn retarget_selected_player(&mut self, index: u8) -> Result<(), NotFound>;
    fn free_current_player(&mut self, free_icon: bool, leave_party_size: bool) -> PlayerId;
    fn party_strength(&mut self) -> u8;
    /// CHECKPARTY (0x1E)'s query dispatch is a partial function in the
    /// original (docket item 7): unrecognized `query` codes silently no-op.
    fn check_party(&mut self, query: u16) -> u16;
    fn party_has_item(&mut self, item_type: u8) -> bool;
    fn find_special(&mut self, affect_type: u8) -> bool;
    fn destroy_items(&mut self, item_type: u8);
    fn rob_money(&mut self, pct: u8);
    fn rob_items(&mut self, chance: u8);
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
    /// SETUP MONSTER (0x0C)'s own operands; see the trait doc comment for why
    /// this doesn't return the approach distance (that's `approach_distance`).
    fn setup_monster(&mut self, sprite_id: u8, max_distance: u8, pic_id: u8);
    fn clear_monsters(&mut self);
    fn add_npc(&mut self, monster_id: u8, morale: u8);
    /// CALL (0x2D) cases `1`/`2` — `SetupDuel(bool)`.
    fn setup_duel(&mut self, is_duel: bool);
    fn calc_group_movement(&mut self) -> (u8, u8);
    fn approach_distance(&mut self) -> u8;
    fn load_encounter_visual(&mut self, flags: u8, distance: u8, pic_id: u8, sprite_id: u8);

    // --- Items / treasure ---
    fn create_item(&mut self, item_type: u8) -> ItemHandle;
    fn load_item_from_table(&mut self, block_id: u8) -> ItemHandle;
    /// SPELL (0x3B)'s not-found sentinel is a deliberate byte-underflow pair
    /// in the original (`0xFF`, `0xFF`) — replicate exactly, don't "fix" it.
    fn find_spell_in_party(&mut self, spell_id: u8) -> (u8, u8);

    // --- Combat math ---
    /// RANDOM (0x08)'s `seg051.Random(max)`. The inclusive-bound adjustment
    /// (`rand_max` incremented before rolling when `<0xFF`) happens in the
    /// opcode handler, not here — see `machine.rs`'s RANDOM implementation.
    fn roll(&mut self, max: u8) -> u8;
    fn roll_dice(&mut self, size: u8, count: u8) -> u16;
    fn roll_saving_throw(&mut self, bonus: u8, save_type: u8) -> bool;
    fn can_hit_target(&mut self, bonus: u8) -> bool;
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
    fn step_game_time(&mut self, time_slot: u8, amount: u8);
    fn move_position_forward(&mut self);
    /// CALL (0x2D) case `0xAE11`/`0x4019` — `get_wall_x2`.
    fn wall_roof(&mut self) -> u8;
    /// CALL (0x2D) case `0xAE11`/`0x4019` — `getMap_wall_type`.
    fn wall_type(&mut self) -> u8;

    // --- CALL (0x2D) case 0x3201 ---
    /// Selects which sound effect CALL's `0x3201` case plays, from
    /// engine-internal state (`word_1EE76`) the VM doesn't own. Playback
    /// itself is an ordinary buffered `Effect::Sound`, not a service.
    fn call_sound_variant(&mut self) -> u8;
}

/// Raw dice/uniform-random primitive (D9: the VM never owns a clock or RNG
/// itself; `VmHost` exposes the engine's one PRNG through this accessor).
/// `EngineServices::roll`/`roll_dice` are the methods the interpreter's
/// opcodes actually call; `rng()` is exposed on `VmHost` per the design
/// doc's API sketch for engine-side service implementations (and any future
/// opcode) to reach the same underlying generator directly.
pub trait VmRng {
    fn roll_uniform(&mut self, inclusive_max: u16) -> u16;
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
}

/// Interactions that suspend the activation awaiting a reply (D-VM3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Request {
    /// HORIZONTAL MENU (0x2B): `options` are the decoded tail strings, in
    /// script order (1-indexed register fill order, but presented 0-indexed
    /// here for the reply to select by).
    HorizontalMenu { options: Vec<VmString> },
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
}

/// Replies to a suspended `Request`. `resume()` checks the reply kind
/// matches the outstanding request's kind (`VmError::ReplyMismatch` if not).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    /// A 0-indexed selection into a `Request::HorizontalMenu`'s `options`.
    Selection(u8),
    Delay,
    Combat,
}

impl Reply {
    /// The `Request` kind this reply answers, for `resume()`'s legality check.
    pub(crate) fn matches(&self, request: &Request) -> bool {
        matches!(
            (self, request),
            (Reply::Selection(_), Request::HorizontalMenu { .. })
                | (Reply::Delay, Request::Delay)
                | (Reply::Combat, Request::Combat)
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
    FreeCurrentPlayer {
        free_icon: bool,
        leave_party_size: bool,
    },
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
        pct: u8,
    },
    RobItems {
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
        max_distance: u8,
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
    LoadEncounterVisual {
        flags: u8,
        distance: u8,
        pic_id: u8,
        sprite_id: u8,
    },

    CreateItem {
        item_type: u8,
    },
    LoadItemFromTable {
        block_id: u8,
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
        bonus: u8,
        save_type: u8,
    },
    CanHitTarget {
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

    CallSoundVariant,
}
