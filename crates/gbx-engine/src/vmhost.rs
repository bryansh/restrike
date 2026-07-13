//! The real `VmHost` implementation (D-VM4/D-VM5, task deliverables 1-2):
//! `ScriptMemory`'s window dispatch (named cells + raw fallback + the
//! unknown-access log) and `EngineServices` (the M2 subset gets real
//! implementations; everything else is a logged M3/M4 stub).
//!
//! Derived by reading coab for behavior (D11, never copied) — a dedicated
//! research pass this session read `engine/ovr008.cs`'s
//! `vm_GetMemoryValueType`/`vm_GetMemoryValue`/`vm_SetMemoryValue` in full,
//! `load_ecl_dax` (`:136-154`), `seg001.cs`'s `game_area` boot init, and
//! `sub_30580` (`:220-276`); every address/behavior below cites that pass.
//! Per the M2 step 4 task brief's scope note, `load_3d_map`/`load_bigpic`
//! **record** resident-block state; they do not draw (3D/area-map rendering
//! is step 5). `load_walldef` graduated to a real implementation in step 5
//! (task deliverable 1): it now actually loads the walldef's tile-id table
//! and its paired 8×8 pixel data into [`crate::symbols::SymbolSets`], which
//! `crate::corridor`'s renderer reads from.

use crate::movement::Facing;
use crate::shell::{EngineState, GameState};
use crate::symbols::SymbolSets;
use gbx_formats::game_data::{GameData, GameDataError};
use gbx_formats::geo::GeoBlock;
use gbx_formats::walldef::WalldefBlock;
use gbx_vm::{
    BlockBytes, ItemHandle, MissingData, MonsterHandle, NotFound, Origin, PlayerId, RecordedCall,
    ScriptMemory, VmRng, VmString, ECL_BLOCK_SIZE,
};
use std::collections::{HashMap, HashSet};

/// The color code every wallset's paired 8×8 symbol data is loaded masked
/// against — the same convention as boot's `Load8x8D` (`boot.rs`'s
/// `BOOT_MASK`, design doc §1.3).
const WALLSET_MASK: u8 = 13;

/// `load_ecl_dax` (`ovr008.cs:136-154`, this session's research §2/§3):
/// `block_id` within `"ECL{game_area}.DAX"` — the file name embeds
/// `game_area`, so the same numeric `block_id` in a different area is a
/// different block entirely. This session's `Engine` fixes `game_area = 2`
/// (matching the already-validated M1/step-3 precedent — real Tilverton
/// data lives in `GEO2.DAX`/`ECL2.DAX`); the research pass also found the
/// literal boot-time default is `1` (`InitFirst`/`InitAgain`,
/// `seg001.cs:276-277,369-370`, with a same-file `game_area = 2` branch for
/// non-demo play seemingly clobbered right after — flagged as UNSURE/a
/// possible transliteration quirk, docketed rather than resolved).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadEclError {
    GameData(GameDataError),
    /// The decoded payload doesn't leave enough bytes after the 2-byte
    /// prefix `load_ecl_dax` skips (`ovr008.cs:151`) to be a real block.
    TooShort,
}

impl From<GameDataError> for LoadEclError {
    fn from(e: GameDataError) -> Self {
        LoadEclError::GameData(e)
    }
}

/// Loads block `block_id` from `"ECL{game_area}.DAX"` and prepares it as a
/// resident [`BlockBytes`] (the `ecl_block_payload` 2-byte-prefix skip,
/// `dax.rs`'s own citation to `ovr008.cs:151`), oversize-truncated to
/// [`ECL_BLOCK_SIZE`] exactly like `frontends/cli/run_script.rs`'s loader.
pub fn load_ecl_block(
    data: &GameData,
    game_area: u8,
    block_id: u8,
) -> Result<BlockBytes, LoadEclError> {
    let raw = data.block(&format!("ECL{game_area}.DAX"), block_id)?;
    let payload = gbx_formats::dax::ecl_block_payload(&raw);
    if payload.is_empty() {
        return Err(LoadEclError::TooShort);
    }
    let payload = &payload[..payload.len().min(ECL_BLOCK_SIZE)];
    Ok(BlockBytes::from_bytes(payload))
}

// --- Window ranges (D-VM5 / this session's research §1.0) ---

const AREA_WINDOW: std::ops::RangeInclusive<u16> = 0x4B00..=0x4EFF;
const TABLE_WINDOW: std::ops::RangeInclusive<u16> = 0x7A00..=0x7BFF;
const PARTY_WINDOW: std::ops::RangeInclusive<u16> = 0x7C00..=0x7FFF;

/// The Area window's ECL-clock word cluster (this session's research §1.5):
/// 7 consecutive words at `0x4BC6..=0x4BD2`, two unlabeled bracketing words
/// around minutes-ones/minutes-tens/hour/day/year.
const CLOCK_BASE: u16 = 0x4BC6;
const IN_DUNGEON_ADDR: u16 = 0x4BE6;
/// Both addresses set the same `byte_1EE94` redraw-dirty flag on write
/// (research §1.5) — recorded, never meaningfully consumed this session.
const FORCE_REDRAW_ADDRS: [u16; 2] = [0x4BFD, 0x4BFE];

/// One access kind, for the unknown-access log's `(addr, kind)` dedup key
/// (D-VM5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AccessKind {
    Read,
    Write,
    ReadByte,
    WriteByte,
    ReadString,
    WriteString,
}

/// One first-seen unknown access — the discovery backlog (D-VM5, PLAN §2.2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UnknownAccess {
    pub addr: u16,
    pub kind: AccessKind,
    pub origin: Origin,
}

/// Dedups per `(addr, kind)`, keeping only the first-seen `Origin` — the
/// unknown-access log is a discovery backlog, not a full trace.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct UnknownAccessLog {
    seen: HashSet<(u16, AccessKind)>,
    entries: Vec<UnknownAccess>,
}

impl UnknownAccessLog {
    fn record(&mut self, addr: u16, kind: AccessKind, origin: Origin) {
        if self.seen.insert((addr, kind)) {
            self.entries.push(UnknownAccess { addr, kind, origin });
        }
    }

    pub fn entries(&self) -> &[UnknownAccess] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A halted vector run's diagnostic (the M2 halt policy, task deliverable
/// 4): every `VmError` is downgraded to a loud, counted status-line-visible
/// event rather than propagating a hard failure — the flow treats the run
/// as ended. Decoupled from `gbx_vm::VmError`'s own shape so this struct can
/// stay serde-derivable without changing that crate.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HaltRecord {
    pub pc: u16,
    pub opcode: u8,
    pub description: String,
}

pub fn describe_halt(err: &gbx_vm::VmError) -> HaltRecord {
    use gbx_vm::VmError;
    match *err {
        VmError::UnknownOpcode { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: format!(
                "opcode {opcode:#04X} has no dialect entry (the original engine would wedge here too)"
            ),
        },
        VmError::Unimplemented { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: format!(
                "opcode {opcode:#04X} is known to the dialect but not yet implemented by this interpreter"
            ),
        },
        VmError::StringOperandTypeMismatch { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: "a string-mode operand was fed to a numeric-only opcode".to_string(),
        },
        VmError::UnresolvedOperand { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: "a destination/target operand had no resolvable raw word".to_string(),
        },
        VmError::MissingAsset { pc, opcode } => HaltRecord {
            pc,
            opcode,
            description: "a required .dax asset was missing".to_string(),
        },
        VmError::StepWhilePending
        | VmError::ResumeWithoutPending
        | VmError::ReplyMismatch
        | VmError::Idle => HaltRecord {
            pc: 0,
            opcode: 0,
            description: format!("{err:?} (engine-orchestration bug, not a content issue)"),
        },
    }
}

/// Resident-asset bookkeeping (§1.3's `setBlocks[0..2]`): recorded so the
/// state is observable/serializable, never drawn (step 5's job).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResidentAssets {
    pub map_3d_block: Option<u8>,
    /// `(set, id)` per `LOAD FILES`/`LOAD PIECES`' up-to-3 walldef slots.
    pub walldefs: [Option<(u8, u8)>; 3],
    pub bigpic_block: Option<u8>,
}

/// Everything `ScriptMemory`/`EngineServices` needs beyond `EngineState`
/// (D-VM5's raw fallback store + log, the M2-slice named-but-inert Global
/// cells, resident-asset bookkeeping, the service-call log, and halt
/// diagnostics). Persists across ticks in [`crate::engine::Engine`].
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VmMemoryState {
    raw_words: HashMap<u16, u16>,
    raw_bytes: HashMap<u16, u8>,
    raw_strings: HashMap<u16, VmString>,
    pub unknown_log: UnknownAccessLog,
    /// A diagnostic trace, not save-relevant state — `RecordedCall` doesn't
    /// derive serde (it's `gbx-vm`'s own H4-oracle-trace type), and this log
    /// is meant for the demo's/inspector's read, not round-tripping.
    #[serde(skip)]
    pub calls: Vec<RecordedCall>,
    pub halts: Vec<HaltRecord>,
    pub assets: ResidentAssets,
    /// `0x3DE` (`word_1EE76`): the CALL `0x3201` sound-variant selector —
    /// the one "dead-ish" Global cell with a real consumer
    /// ([`call_sound_variant`]).
    word_1ee76: u16,
    /// `0xB8`/`0xB9` (`word_1EE78`/`word_1EE7A`): write-only, no consumer
    /// found anywhere in the reference source (research §1.3) — stored
    /// verbatim anyway (cheap, and preserves round-trip if a consumer
    /// surfaces later), never read back through this facade.
    word_1ee78: u16,
    word_1ee7a: u16,
    /// `byte_1EE91`/`byte_1EE94` (redraw-dirty flags, `vm_SetMemoryValue`
    /// locations `0xBF68+0xF1`/`0xF7` and `0x4B00`-relative `0xFD`/`0xFE`)
    /// and `gbl.positionChanged` (`mapPosX`/`mapPosY`/`mapDirection`
    /// writes, `MovePositionForward`): the three flags a dedicated step 5
    /// research pass found consolidated at a single real gate, `CMD_Call`
    /// case `0xAE11` (`ovr003.cs:1844-1860`) — `if (spriteChanged ||
    /// displayPlayerSprite || byte_1EE91 || positionChanged || byte_1EE94)
    /// { RedrawView(); display_map_position_time(); <clear all five> }`.
    pub byte_1ee91: bool,
    pub byte_1ee94: bool,
    pub position_changed: bool,
    /// `gbl.spriteChanged`: set by `sub_30580` (encounter-visual dispatch)
    /// and `CMD_Picture` — same consolidated gate above.
    pub sprite_changed: bool,
    /// `gbl.can_draw_bigpic`: set at many command/init sites and
    /// unconditionally by `LoadPic`; read only by `RedrawView`'s own
    /// non-dungeon (wilderness/bigpic, M6) branch — recorded for state
    /// fidelity, no M2 consumer.
    pub can_draw_bigpic: bool,
}

impl VmMemoryState {
    /// `CMD_Call` case `0xAE11`'s consolidated redraw gate
    /// (`ovr003.cs:1848-1860`): this session's `crate::corridor::redraw_view`
    /// runs unconditionally at every world-menu-visible point instead of
    /// gating on these flags (`shell.rs`'s `enter_world_menu` doc comment
    /// explains why that's a safe, documented simplification for a
    /// deterministic immediate-mode renderer) — but the flags themselves
    /// are still cleared here, at the same logical point the original
    /// clears them, so their *state* stays faithful even though nothing
    /// currently reads them to decide *whether* to redraw.
    pub(crate) fn clear_redraw_flags(&mut self) {
        self.byte_1ee91 = false;
        self.byte_1ee94 = false;
        self.position_changed = false;
        self.sprite_changed = false;
    }
}

impl VmMemoryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// The raw fallback word store's current value at `addr` (D-VM5's
    /// round-trip guarantee for any cell without a named cell above) — a
    /// read-only seam for tests and the eventual inspector (D-UI8), not
    /// used by `ScriptMemory` dispatch itself (which owns its own
    /// insert/lookup calls directly).
    pub fn raw_word(&self, addr: u16) -> Option<u16> {
        self.raw_words.get(&addr).copied()
    }

    /// The raw fallback byte store's current value at `addr` — [`raw_word`](Self::raw_word)'s
    /// counterpart for `ScriptMemory::read_byte`/`write_byte` traffic, added
    /// for `tools/inspect`'s ScriptMemory watch pane (D-UI8) so it can show
    /// the raw-store contents across all three access widths, not just
    /// words.
    pub fn raw_byte(&self, addr: u16) -> Option<u8> {
        self.raw_bytes.get(&addr).copied()
    }

    /// The raw fallback string store's current value at `addr` —
    /// [`raw_word`](Self::raw_word)'s counterpart for
    /// `ScriptMemory::read_string`/`write_string` traffic.
    pub fn raw_string(&self, addr: u16) -> Option<&VmString> {
        self.raw_strings.get(&addr)
    }
}

/// The real `VmHost`: borrows engine state fresh each pump (`shell.rs`
/// constructs one per `EclMachine::step`/`resume` call).
pub struct EngineVmHost<'a> {
    pub state: &'a mut EngineState,
    pub vm: &'a mut VmMemoryState,
    pub geo: &'a GeoBlock,
    pub party: &'a mut dyn crate::movement::PartyPredicates,
    pub rng: &'a mut crate::rng::EngineRng,
    pub sounds: &'a mut Vec<crate::shell::SoundEvent>,
    /// `load_walldef`'s real data source (step 5, task deliverable 1):
    /// `"WALLDEF{game_area}.DAX"`/`"8X8D{game_area}.DAX"`, the same
    /// `game_area`-embeds-the-filename convention `load_ecl_dax`/boot's
    /// `Load8x8D` already use.
    pub data: &'a GameData,
    pub game_area: u8,
    pub symbols: &'a mut SymbolSets,
}

impl EngineVmHost<'_> {
    fn wall_square(&self) -> &gbx_formats::geo::Square {
        self.geo
            .square(self.state.pos.0 as usize, self.state.pos.1 as usize)
    }

    /// `mapWallType` (`getMap_wall_type`): the facing edge's raw wall-type
    /// nibble (`0` = no wall).
    fn wall_type_value(&self) -> u16 {
        let sq = self.wall_square();
        let v = match self.state.facing {
            Facing::North => sq.wall_north,
            Facing::East => sq.wall_east,
            Facing::South => sq.wall_south,
            Facing::West => sq.wall_west,
        };
        v as u16
    }

    /// `mapWallRoof` (`get_wall_x2`): the current square's reconstructed
    /// `x2` byte (`indoor<<7 | floor_flag<<6 | low7`, `gbx_formats::geo`'s
    /// own decomposition, undone here).
    fn wall_roof_value(&self) -> u16 {
        let sq = self.wall_square();
        let mut x2 = sq.low7 & 0x7F;
        if sq.indoor {
            x2 |= 0x80;
        }
        if sq.floor_flag {
            x2 |= 0x40;
        }
        x2 as u16
    }
}

impl ScriptMemory for EngineVmHost<'_> {
    fn read(&mut self, addr: u16, origin: Origin) -> u16 {
        if AREA_WINDOW.contains(&addr) {
            return self.read_area(addr, origin);
        }
        if TABLE_WINDOW.contains(&addr) || PARTY_WINDOW.contains(&addr) {
            self.vm.unknown_log.record(addr, AccessKind::Read, origin);
            return self.vm.raw_words.get(&addr).copied().unwrap_or(0);
        }
        self.read_global(addr, origin)
    }

    fn write(&mut self, addr: u16, value: u16, origin: Origin) {
        if AREA_WINDOW.contains(&addr) {
            return self.write_area(addr, value, origin);
        }
        if TABLE_WINDOW.contains(&addr) || PARTY_WINDOW.contains(&addr) {
            self.vm.unknown_log.record(addr, AccessKind::Write, origin);
            self.vm.raw_words.insert(addr, value);
            return;
        }
        self.write_global(addr, value, origin);
    }

    fn read_byte(&mut self, addr: u16, origin: Origin) -> u8 {
        self.vm
            .unknown_log
            .record(addr, AccessKind::ReadByte, origin);
        self.vm.raw_bytes.get(&addr).copied().unwrap_or(0)
    }

    fn write_byte(&mut self, addr: u16, value: u8, origin: Origin) {
        self.vm
            .unknown_log
            .record(addr, AccessKind::WriteByte, origin);
        self.vm.raw_bytes.insert(addr, value);
    }

    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString {
        self.vm
            .unknown_log
            .record(addr, AccessKind::ReadString, origin);
        self.vm.raw_strings.get(&addr).cloned().unwrap_or_default()
    }

    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin) {
        self.vm
            .unknown_log
            .record(addr, AccessKind::WriteString, origin);
        self.vm.raw_strings.insert(addr, s.clone());
    }
}

impl EngineVmHost<'_> {
    /// The Area window (`0x4B00..=0x4EFF`): the ECL clock cluster + the two
    /// named flags, everything else raw+logged (research §1.5).
    fn read_area(&mut self, addr: u16, origin: Origin) -> u16 {
        if (CLOCK_BASE..=CLOCK_BASE + 12).contains(&addr) && (addr - CLOCK_BASE).is_multiple_of(2) {
            let idx = ((addr - CLOCK_BASE) / 2) as usize;
            return self.state.clock.raw_clock_words()[idx];
        }
        if addr == IN_DUNGEON_ADDR {
            return u16::from(self.state.game_state == GameState::DungeonMap);
        }
        self.vm.unknown_log.record(addr, AccessKind::Read, origin);
        self.vm.raw_words.get(&addr).copied().unwrap_or(0)
    }

    fn write_area(&mut self, addr: u16, value: u16, origin: Origin) {
        if addr == IN_DUNGEON_ADDR {
            let new_in_dungeon = value != 0;
            let cur_in_dungeon = self.state.game_state == GameState::DungeonMap;
            if new_in_dungeon != cur_in_dungeon {
                self.state.last_game_state = self.state.game_state;
                self.state.game_state = if value == 0 {
                    GameState::WildernessMap
                } else {
                    GameState::DungeonMap
                };
            }
            self.vm.raw_words.insert(addr, value);
            return;
        }
        if FORCE_REDRAW_ADDRS.contains(&addr) {
            self.vm.byte_1ee94 = true;
            self.vm.raw_words.insert(addr, value);
            return;
        }
        self.vm.unknown_log.record(addr, AccessKind::Write, origin);
        self.vm.raw_words.insert(addr, value);
    }

    /// The Global window's named cells (research §1.1/§1.2) — everything
    /// unmatched round-trips through the raw store + unknown-access log
    /// (D-VM5's deliberate design choice: scripts still get back what they
    /// stash, even at an address the original's own switch silently drops —
    /// see `0x2CB`'s docket note below).
    fn read_global(&mut self, addr: u16, origin: Origin) -> u16 {
        match addr {
            // Confirmed dead cells: write no-op, field never assigned
            // elsewhere in the reference source — always reads 0.
            0x00B1 | 0x00FB | 0x00FC => 0,
            // The raw (unhalved) facing read, distinct from 0xC04D below.
            0x033D => self.state.facing.raw_code() as u16,
            0x035F => 0, // stub case in the original, no assignment
            0xC04B => self.state.pos.0 as u16,
            0xC04C => self.state.pos.1 as u16,
            0xC04D => (self.state.facing.raw_code() / 2) as u16,
            0xC04E => self.wall_type_value(),
            0xC04F => self.wall_roof_value(),
            0xC059 => 0, // stub read case (write sets byte_1EE91; read here never reflects it)
            _ => {
                self.vm.unknown_log.record(addr, AccessKind::Read, origin);
                self.vm.raw_words.get(&addr).copied().unwrap_or(0)
            }
        }
    }

    fn write_global(&mut self, addr: u16, value: u16, origin: Origin) {
        match addr {
            // Confirmed no-op writes (research §1.1) — dropped, not stored.
            0x00B1 | 0x00FB | 0x00FC => {}
            0x03DE => self.vm.word_1ee76 = value,
            0x00B8 => self.vm.word_1ee78 = value,
            0x00B9 => self.vm.word_1ee7a = value,
            0xC04B => {
                self.state.pos.0 = value as u8;
                self.vm.position_changed = true;
            }
            0xC04C => {
                self.state.pos.1 = value as u8;
                self.vm.position_changed = true;
            }
            0xC04D => {
                // The original's do-while normalizes any input to `%4`
                // before expanding to the raw facing code (research §1.1).
                let normalized = (value % 4) as u8;
                self.state.facing = Facing::from_raw(normalized * 2);
                self.vm.position_changed = true;
            }
            0xC059 | 0xC05F => self.vm.byte_1ee91 = true,
            // 0xC04E/0xC04F (wall type/roof) are read-only through this
            // dispatch in the original (no write case) — silently dropped,
            // matching that exactly (not even round-tripped via raw store).
            0xC04E | 0xC04F => {}
            // `0x2CB` (SURPRISE's write target, `CMD_Surprise`,
            // `ovr003.cs:967`): this session's research found no matching
            // case in either the original's read or write switch — the
            // write appears to be a genuine no-op in the reference source
            // (flagged, not resolved; see design doc docket). Falling
            // through to the raw store here is *more* functional than the
            // original (round-trips the value instead of dropping it),
            // which is D-VM5's own explicit "unknown cells still
            // round-trip" design choice — a deliberate, documented
            // divergence in the safe direction, not a fidelity miss.
            _ => {
                self.vm.unknown_log.record(addr, AccessKind::Write, origin);
                self.vm.raw_words.insert(addr, value);
            }
        }
    }
}

// --- EngineServices (D-VM4's placement rule; M2 subset real, rest logged M3/M4 stubs) ---

impl gbx_vm::EngineServices for EngineVmHost<'_> {
    fn retarget_selected_player(&mut self, index: u8) -> Result<(), NotFound> {
        self.vm
            .calls
            .push(RecordedCall::RetargetSelectedPlayer { index });
        Ok(())
    }

    fn free_current_player(&mut self, free_icon: bool, leave_party_size: bool) -> PlayerId {
        self.vm.calls.push(RecordedCall::FreeCurrentPlayer {
            free_icon,
            leave_party_size,
        });
        PlayerId(0)
    }

    fn party_strength(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::PartyStrength);
        0
    }

    fn check_party(&mut self, query: u16) -> u16 {
        self.vm.calls.push(RecordedCall::CheckParty { query });
        0
    }

    fn party_has_item(&mut self, item_type: u8) -> bool {
        self.vm.calls.push(RecordedCall::PartyHasItem { item_type });
        false
    }

    fn find_special(&mut self, affect_type: u8) -> bool {
        self.vm
            .calls
            .push(RecordedCall::FindSpecial { affect_type });
        false
    }

    fn destroy_items(&mut self, item_type: u8) {
        self.vm.calls.push(RecordedCall::DestroyItems { item_type });
    }

    fn rob_money(&mut self, pct: u8) {
        self.vm.calls.push(RecordedCall::RobMoney { pct });
    }

    fn rob_items(&mut self, chance: u8) {
        self.vm.calls.push(RecordedCall::RobItems { chance });
    }

    fn party_surprise_check(&mut self) -> (u8, u8) {
        self.vm.calls.push(RecordedCall::PartySurpriseCheck);
        (0, 0)
    }

    fn load_monster(
        &mut self,
        monster_id: u8,
        num_copies: u8,
        icon_block_id: u8,
    ) -> Result<MonsterHandle, MissingData> {
        self.vm.calls.push(RecordedCall::LoadMonster {
            monster_id,
            num_copies,
            icon_block_id,
        });
        Ok(MonsterHandle(monster_id as u16))
    }

    fn setup_monster(&mut self, sprite_id: u8, max_distance: u8, pic_id: u8) {
        self.vm.calls.push(RecordedCall::SetupMonster {
            sprite_id,
            max_distance,
            pic_id,
        });
    }

    fn clear_monsters(&mut self) {
        self.vm.calls.push(RecordedCall::ClearMonsters);
    }

    fn add_npc(&mut self, monster_id: u8, morale: u8) {
        self.vm
            .calls
            .push(RecordedCall::AddNpc { monster_id, morale });
    }

    fn setup_duel(&mut self, is_duel: bool) {
        self.vm.calls.push(RecordedCall::SetupDuel { is_duel });
    }

    fn calc_group_movement(&mut self) -> (u8, u8) {
        self.vm.calls.push(RecordedCall::CalcGroupMovement);
        (0, 0)
    }

    /// `sub_304B4`'s approach-distance calc: the exact wall-type-driven
    /// formula wasn't in the material read this session (opcode-
    /// classification.md's own docket names the same gap) — a documented
    /// neutral placeholder pending that read.
    fn approach_distance(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::ApproachDistance);
        0
    }

    fn load_encounter_visual(&mut self, flags: u8, distance: u8, pic_id: u8, sprite_id: u8) {
        self.vm.calls.push(RecordedCall::LoadEncounterVisual {
            flags,
            distance,
            pic_id,
            sprite_id,
        });
        // `sub_30580`'s state effects (research §4) — recorded, not drawn.
        if distance == 0 {
            self.state.head_block_id = 0xFF;
        }
    }

    fn create_item(&mut self, item_type: u8) -> ItemHandle {
        self.vm.calls.push(RecordedCall::CreateItem { item_type });
        ItemHandle(0)
    }

    fn load_item_from_table(&mut self, block_id: u8) -> ItemHandle {
        self.vm
            .calls
            .push(RecordedCall::LoadItemFromTable { block_id });
        ItemHandle(0)
    }

    fn find_spell_in_party(&mut self, spell_id: u8) -> (u8, u8) {
        self.vm
            .calls
            .push(RecordedCall::FindSpellInParty { spell_id });
        (0xFF, 0xFF) // the original's own not-found sentinel (byte underflow), replicated verbatim
    }

    fn roll(&mut self, max: u8) -> u8 {
        self.vm.calls.push(RecordedCall::Roll { max });
        self.rng.roll_uniform(max as u16) as u8
    }

    fn roll_dice(&mut self, size: u8, count: u8) -> u16 {
        self.vm.calls.push(RecordedCall::RollDice { size, count });
        let mut total = 0u16;
        for _ in 0..count {
            total += 1 + self.rng.roll_uniform(size.saturating_sub(1) as u16);
        }
        total
    }

    fn roll_saving_throw(&mut self, bonus: u8, save_type: u8) -> bool {
        self.vm
            .calls
            .push(RecordedCall::RollSavingThrow { bonus, save_type });
        false
    }

    fn can_hit_target(&mut self, bonus: u8) -> bool {
        self.vm.calls.push(RecordedCall::CanHitTarget { bonus });
        false
    }

    fn apply_damage(&mut self, player: PlayerId, damage: u16) {
        self.vm
            .calls
            .push(RecordedCall::ApplyDamage { player, damage });
    }

    fn load_3d_map(&mut self, block_id: u8) {
        self.vm.calls.push(RecordedCall::Load3dMap { block_id });
        self.vm.assets.map_3d_block = Some(block_id);
    }

    /// `LoadWalldef` (`ovr031.cs:642-687`, step 5 task deliverable 1) — a
    /// dedicated research pass this session read the function (plus
    /// `Classes/GeoBlock.cs`'s `WallDefs`/`WallDefBlock.Offset`) in full and
    /// found a load call can populate *multiple consecutive* wallset slots,
    /// not just `set` itself: it loads the walldef block's raw tile-id data
    /// from `"WALLDEF{game_area}.DAX"` block `id`, which may hold several
    /// internal 780-byte sub-blocks (`WalldefBlock::wallset_count`); for
    /// each sub-block `n` (`0`-indexed), the *target* slot is `set + n`, and
    /// only sub-blocks landing in `1..=3` are kept (`idx = symbolSet + block`,
    /// `:664-682`) — so `LoadWalldef(1, id)` with a 3-sub-block walldef
    /// populates sets 1, 2, *and* 3 in one call. Each kept sub-block's
    /// paired 8×8 pixel data loads from `"8X8D{game_area}.DAX"` at `id`
    /// (single sub-block) or `id*10 + n + 1` (multiple, 1-based, `:673-679`)
    /// into `SymbolSets`' matching pixel slot. The `>=0x2D` rebase (`var_A =
    /// symbol_set_fix[set] - symbol_set_fix[1]`, computed once from the
    /// call's *original* `set` parameter, `:658`) is applied to every
    /// touched sub-block's tile ids (wrapping byte add, `GeoBlock.cs:84`)
    /// before storing — baked in, not reapplied at lookup time. Bookkeeping
    /// (`vm.assets.walldefs`) records only the original `set` slot's
    /// `(set, id)` pair, matching a real asymmetry this research pass found
    /// in the original itself (`:669` vs `:684-685` — every touched slot
    /// gets real texture data, only the one matching the call's own `set`
    /// gets its `setBlocks` entry written). Any load failure (missing
    /// block, malformed data) is a silent no-op for that sub-block beyond
    /// the call log — real CotAB data never hits this path (this session's
    /// demo/tests load every wallset the walk exercises without error).
    fn load_walldef(&mut self, set: u8, id: u8) {
        self.vm.calls.push(RecordedCall::LoadWalldef { set, id });
        let slot = (set.saturating_sub(1)) as usize;
        if let Some(entry) = self.vm.assets.walldefs.get_mut(slot) {
            *entry = Some((set, id));
        }
        if !(1..=3).contains(&set) {
            return;
        }

        let Ok(raw) = self
            .data
            .block(&format!("WALLDEF{}.DAX", self.game_area), id)
        else {
            return;
        };
        let Ok(walldef) = WalldefBlock::parse(&raw) else {
            return;
        };
        let block_count = walldef.wallset_count();
        if block_count == 0 {
            return;
        }

        let rebase = (crate::symbols::SYMBOL_SET_FIX[set as usize] as i32
            - crate::symbols::SYMBOL_SET_FIX[1] as i32) as u8;
        let sym_file = format!("8X8D{}.DAX", self.game_area);

        for block in 0..block_count {
            let idx = set as usize + block;
            if !(1..=3).contains(&idx) {
                continue;
            }

            let mut tiles = [0u8; gbx_formats::walldef::WALLSET_SIZE];
            for style in 0..gbx_formats::walldef::STYLES_PER_WALLSET {
                for i in 0..gbx_formats::walldef::TILE_IDS_PER_STYLE {
                    let raw_id = walldef.tile_id(block, style, i).unwrap_or(0);
                    tiles[style * gbx_formats::walldef::TILE_IDS_PER_STYLE + i] = if raw_id >= 0x2D
                    {
                        raw_id.wrapping_add(rebase)
                    } else {
                        raw_id
                    };
                }
            }

            let sym_block_id = if block_count > 1 {
                id.wrapping_mul(10).wrapping_add(block as u8 + 1)
            } else {
                id
            };
            let Ok(bytes) = self.data.block(&sym_file, sym_block_id) else {
                continue;
            };
            let Ok(decoded) = gbx_formats::image::decode(&bytes, Some(WALLSET_MASK)) else {
                continue;
            };

            self.symbols.load(idx, decoded);
            self.symbols
                .load_wallset(idx - 1, crate::symbols::WallsetSlot::from_tiles(tiles));
        }
    }

    fn load_bigpic(&mut self, id: u8) {
        self.vm.calls.push(RecordedCall::LoadBigpic { id });
        self.vm.assets.bigpic_block = Some(id);
    }

    fn reset_wall_set(&mut self, index: u8) {
        self.vm.calls.push(RecordedCall::ResetWallSet { index });
        if let Some(entry) = self.vm.assets.walldefs.get_mut(index as usize) {
            *entry = None;
        }
        if (index as usize) < crate::symbols::WALLSET_SLOT_COUNT {
            self.symbols.reset_wallset(index as usize);
        }
    }

    fn step_game_time(&mut self, time_slot: u8, amount: u8) {
        self.vm
            .calls
            .push(RecordedCall::StepGameTime { time_slot, amount });
        self.state.clock.step(time_slot, amount);
    }

    /// `CALL 0x401F` (`MovePositionForward`, research §4's summary table):
    /// advances one cell in the facing direction, wrapping map coords —
    /// the raw advance, with none of the walk loop's door gating
    /// (`movement::move_party_forward` is the higher-level function for
    /// that; this mirrors the original's own lower-level primitive).
    fn move_position_forward(&mut self) {
        self.vm.calls.push(RecordedCall::MovePositionForward);
        let (dx, dy) = self.state.facing.delta();
        self.state.pos.0 = (self.state.pos.0 as i32 + dx).rem_euclid(16) as u8;
        self.state.pos.1 = (self.state.pos.1 as i32 + dy).rem_euclid(16) as u8;
        self.vm.position_changed = true;
    }

    fn wall_roof(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::WallRoof);
        self.wall_roof_value() as u8
    }

    fn wall_type(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::WallType);
        self.wall_type_value() as u8
    }

    /// CALL `0x3201`'s variant selector (research §1.3/§1.5): `word_1EE76
    /// == 8` -> `sound_a`-class, `== 10` -> `sound_b`-class, else
    /// `sound_a`-class. Real sound-catalog ids are a documented placeholder
    /// (`movement::SOUND_A`'s doc comment) pending a `seg044.cs` read.
    fn call_sound_variant(&mut self) -> u8 {
        self.vm.calls.push(RecordedCall::CallSoundVariant);
        if self.vm.word_1ee76 == 10 {
            1
        } else {
            crate::movement::SOUND_A
        }
    }
}

impl VmRng for EngineVmHost<'_> {
    fn roll_uniform(&mut self, inclusive_max: u16) -> u16 {
        self.rng.roll_uniform(inclusive_max)
    }
}

impl gbx_vm::VmHost for EngineVmHost<'_> {
    fn rng(&mut self) -> &mut dyn VmRng {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::DefaultPartyPredicates;
    use crate::rng::EngineRng;
    use crate::shell::SoundEvent;
    use gbx_formats::geo::GEO_BLOCK_SIZE;
    use gbx_vm::EngineServices;

    const GAME_AREA: u8 = 2;

    struct Fixture {
        state: EngineState,
        vm: VmMemoryState,
        geo: GeoBlock,
        party: DefaultPartyPredicates,
        rng: EngineRng,
        sounds: Vec<SoundEvent>,
        data: GameData,
        symbols: SymbolSets,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_data(GameData::from_files(Vec::<(String, Vec<u8>)>::new()))
        }

        fn with_data(data: GameData) -> Self {
            Fixture {
                state: EngineState::new(),
                vm: VmMemoryState::new(),
                geo: GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap(),
                party: DefaultPartyPredicates::default(),
                rng: EngineRng::new(1),
                sounds: Vec::new(),
                data,
                symbols: SymbolSets::new(),
            }
        }

        fn host(&mut self) -> EngineVmHost<'_> {
            EngineVmHost {
                state: &mut self.state,
                vm: &mut self.vm,
                geo: &self.geo,
                party: &mut self.party,
                rng: &mut self.rng,
                sounds: &mut self.sounds,
                data: &self.data,
                game_area: GAME_AREA,
                symbols: &mut self.symbols,
            }
        }
    }

    fn origin() -> Origin {
        Origin { pc: 0x8100 }
    }

    #[test]
    fn map_pos_round_trips_and_sets_position_changed() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0xC04B, 7, origin());
        host.write(0xC04C, 13, origin());
        assert_eq!(host.read(0xC04B, origin()), 7);
        assert_eq!(host.read(0xC04C, origin()), 13);
        assert!(host.vm.position_changed);
        assert_eq!(host.state.pos, (7, 13));
    }

    #[test]
    fn facing_write_at_0xc04d_uses_the_halved_encoding_and_normalizes_mod_4() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0xC04D, 2, origin()); // halved 2 -> raw South (4)
        assert_eq!(host.state.facing, Facing::South);
        host.write(0xC04D, 5, origin()); // 5 % 4 = 1 -> raw East (2)
        assert_eq!(host.state.facing, Facing::East);
    }

    #[test]
    fn facing_reads_differ_between_0xc04d_halved_and_0x033d_raw() {
        let mut f = Fixture::new();
        f.state.facing = Facing::South; // raw 4, halved 2
        let mut host = f.host();
        assert_eq!(host.read(0xC04D, origin()), 2);
        assert_eq!(host.read(0x033D, origin()), 4);
    }

    #[test]
    fn dead_cells_b1_fb_fc_are_write_no_ops_and_always_read_zero() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0x00B1, 0xFFFF, origin());
        host.write(0x00FB, 0xFFFF, origin());
        host.write(0x00FC, 0xFFFF, origin());
        assert_eq!(host.read(0x00B1, origin()), 0);
        assert_eq!(host.read(0x00FB, origin()), 0);
        assert_eq!(host.read(0x00FC, origin()), 0);
    }

    #[test]
    fn wall_type_and_roof_are_read_only_through_scriptmemory() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0xC04E, 5, origin());
        host.write(0xC04F, 5, origin());
        // Both must stay whatever the GEO block says (0 on an all-open
        // fixture), never the attempted write.
        assert_eq!(host.read(0xC04E, origin()), 0);
        assert_eq!(host.read(0xC04F, origin()), 0);
    }

    #[test]
    fn in_dungeon_write_flips_game_state_only_on_actual_change() {
        let mut f = Fixture::new();
        f.state.game_state = GameState::DungeonMap;
        let mut host = f.host();
        host.write(IN_DUNGEON_ADDR, 0, origin()); // -> WildernessMap
        assert_eq!(host.state.game_state, GameState::WildernessMap);
        assert_eq!(host.state.last_game_state, GameState::DungeonMap);
        // Writing the same value again must not touch last_game_state.
        host.state.last_game_state = GameState::DungeonMap; // poke to detect a spurious re-save
        host.write(IN_DUNGEON_ADDR, 0, origin());
        assert_eq!(host.state.last_game_state, GameState::DungeonMap);
    }

    #[test]
    fn clock_cells_reflect_game_clock_after_step_game_time() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.step_game_time(1, 100); // 100 units, normal rate
        let hour_addr = CLOCK_BASE + 2 * 3;
        assert!(host.read(hour_addr, origin()) > 0 || host.read(CLOCK_BASE + 2, origin()) > 0);
    }

    #[test]
    fn party_and_table_windows_go_to_the_raw_store_and_log_unknown_access() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0x7C10, 42, origin());
        assert_eq!(host.read(0x7C10, origin()), 42);
        host.write(0x7A10, 7, origin());
        assert_eq!(host.read(0x7A10, origin()), 7);
        assert_eq!(host.vm.unknown_log.entries().len(), 4); // write+read for each
    }

    #[test]
    fn unmatched_global_address_round_trips_via_the_raw_store() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.write(0x2CB, 99, origin()); // SURPRISE's target — no case in the original
        assert_eq!(host.read(0x2CB, origin()), 99);
        assert_eq!(host.vm.unknown_log.entries().len(), 2);
    }

    #[test]
    fn move_position_forward_wraps_and_sets_position_changed() {
        let mut f = Fixture::new();
        f.state.pos = (15, 0);
        f.state.facing = Facing::East;
        let mut host = f.host();
        host.move_position_forward();
        assert_eq!(host.state.pos, (0, 0));
        assert!(host.vm.position_changed);
    }

    #[test]
    fn call_sound_variant_selects_by_word_1ee76() {
        let mut f = Fixture::new();
        {
            let mut host = f.host();
            host.write(0x03DE, 10, origin());
        }
        let mut host = f.host();
        assert_eq!(host.call_sound_variant(), 1);
    }

    #[test]
    fn service_calls_are_logged() {
        let mut f = Fixture::new();
        let mut host = f.host();
        host.clear_monsters();
        assert_eq!(host.vm.calls, vec![RecordedCall::ClearMonsters]);
    }
}
