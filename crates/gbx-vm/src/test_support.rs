//! Test-only synthetic ECL block assembler (`docs/design/vm-scriptmemory.md`
//! §4: "hand-construct synthetic blocks opcode-by-opcode"). Gated behind the
//! `test-support` Cargo feature rather than `#[cfg(test)]` so it can be
//! reused as a dev-dependency by other crates' conformance tests (the
//! interpreter's, later) without duplicating it per-crate.
//!
//! Every fixture built with [`EclBuilder`] is hand-authored (D10) — nothing
//! here is derived from real game data, and nothing here ships in a release
//! binary (the feature is off by default).

use crate::decode::{BlockBytes, ECL_BLOCK_BASE, ECL_BLOCK_SIZE};
use crate::host::{
    EngineServices, ItemHandle, MissingData, MonsterHandle, NotFound, Origin, PlayerId,
    RecordedCall, ScriptMemory, VmHost, VmRng, VmString,
};
use std::collections::{HashMap, VecDeque};

/// One pending word-sized fixup: the two bytes at `bytes[offset..offset+2]`
/// (little-endian) get patched with the resolved address of `label` once
/// [`EclBuilder::build`] runs — this is what lets labels be referenced
/// before they're defined (forward jumps).
struct Fixup {
    offset: usize,
    label: String,
}

/// Hand-assembles a synthetic ECL block byte-by-byte: opcode, operand mode +
/// payload, inline/data bytes, and labels with fixups for jump/call targets.
///
/// Method names mirror the operand-mode table in
/// `docs/design/vm-scriptmemory.md` §1 (`mem` = mode `0x01`, `imm_word` =
/// mode `0x02`, etc.) so a test reads like the instruction it's building.
#[derive(Default)]
pub struct EclBuilder {
    bytes: Vec<u8>,
    labels: HashMap<String, u16>,
    fixups: Vec<Fixup>,
}

impl EclBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The VM address the next byte pushed will land at.
    pub fn here(&self) -> u16 {
        ECL_BLOCK_BASE.wrapping_add(self.bytes.len() as u16)
    }

    /// Records `name` as a label bound to the current (not-yet-written)
    /// address. Panics on a duplicate label — a test-authoring bug, not a
    /// runtime condition to handle gracefully.
    pub fn label(&mut self, name: &str) -> &mut Self {
        let addr = self.here();
        assert!(
            self.labels.insert(name.to_string(), addr).is_none(),
            "EclBuilder: duplicate label {name:?}"
        );
        self
    }

    /// The resolved VM address of `name`. Panics if undefined.
    pub fn addr_of(&self, name: &str) -> u16 {
        *self
            .labels
            .get(name)
            .unwrap_or_else(|| panic!("EclBuilder: undefined label {name:?}"))
    }

    fn push_byte(&mut self, b: u8) -> &mut Self {
        self.bytes.push(b);
        self
    }

    fn push_word(&mut self, value: u16) -> &mut Self {
        let [lo, hi] = value.to_le_bytes();
        self.push_byte(lo);
        self.push_byte(hi)
    }

    fn push_word_fixup(&mut self, label: &str) -> &mut Self {
        let offset = self.bytes.len();
        self.fixups.push(Fixup {
            offset,
            label: label.to_string(),
        });
        self.push_byte(0);
        self.push_byte(0)
    }

    /// Pushes a raw opcode byte.
    pub fn op(&mut self, opcode: u8) -> &mut Self {
        self.push_byte(opcode)
    }

    /// mode `0x00`: immediate byte operand.
    pub fn imm_byte(&mut self, value: u8) -> &mut Self {
        self.push_byte(0x00);
        self.push_byte(value)
    }

    /// mode `0x01`: memory-address operand (ScriptMemory-resolved read).
    pub fn mem(&mut self, addr: u16) -> &mut Self {
        self.push_byte(0x01);
        self.push_word(addr)
    }

    /// mode `0x03`: the read/write-identical alt memory-address operand.
    pub fn mem_alt(&mut self, addr: u16) -> &mut Self {
        self.push_byte(0x03);
        self.push_word(addr)
    }

    /// mode `0x02`: immediate word operand (jump/call targets, small counts).
    pub fn imm_word(&mut self, value: u16) -> &mut Self {
        self.push_byte(0x02);
        self.push_word(value)
    }

    /// mode `0x02` whose word is a forward/backward reference to `label`,
    /// resolved at [`build`](Self::build). The usual way to encode a
    /// GOTO/GOSUB/ON-GOTO-tail target in a test fixture.
    pub fn imm_word_label(&mut self, label: &str) -> &mut Self {
        self.push_byte(0x02);
        self.push_word_fixup(label)
    }

    /// mode `0x01` whose word is a label reference — for exercising a
    /// destination operand encoded in the "address" mode rather than
    /// `0x02` (docket item 3: both behave identically as raw-word targets).
    pub fn mem_label(&mut self, label: &str) -> &mut Self {
        self.push_byte(0x01);
        self.push_word_fixup(label)
    }

    /// mode `0x81`: string-from-memory operand.
    pub fn mem_str(&mut self, addr: u16) -> &mut Self {
        self.push_byte(0x81);
        self.push_word(addr)
    }

    /// mode `0x81` whose address is a label reference — for building an
    /// in-block string operand that points at a data region built with
    /// [`raw`](Self::raw)/[`label`](Self::label) elsewhere in the same block.
    pub fn mem_str_label(&mut self, label: &str) -> &mut Self {
        self.push_byte(0x81);
        self.push_word_fixup(label)
    }

    /// mode `0x80`: inline packed-string operand, packed through the real
    /// 6-bit ECL compression (`gbx_formats::ecl_text::compress` — task 1,
    /// ECL inline-string decompression) so conformance tests exercise the
    /// on-wire format the interpreter actually decodes, not a raw escape
    /// hatch. `text` must be plain ASCII in the representable domain
    /// (`0x20..=0x5F`, no lowercase); panics on a non-representable byte —
    /// a test-authoring bug, matching this builder's other panics
    /// (undefined/duplicate labels).
    pub fn inline_str(&mut self, text: &[u8]) -> &mut Self {
        let packed = gbx_formats::ecl_text::compress(text).unwrap_or_else(|e| {
            panic!("EclBuilder::inline_str: byte {:#04X} at index {} isn't representable in the 6-bit ECL text scheme (use inline_str_packed for raw bytes)", e.byte, e.index)
        });
        self.inline_str_packed(&packed)
    }

    /// mode `0x80`: inline packed-string operand, raw bytes verbatim
    /// (undecoded escape hatch — `decode.rs` docket item 5 — the length
    /// byte plus exactly that many raw bytes, no compression applied). For
    /// tests that need to exercise decode-time byte accounting itself
    /// rather than realistic packed text (e.g. arbitrary non-ASCII payload
    /// bytes to prove operand-length handling is content-agnostic).
    pub fn inline_str_packed(&mut self, raw: &[u8]) -> &mut Self {
        self.push_byte(0x80);
        self.push_byte(raw.len() as u8);
        self.bytes.extend_from_slice(raw);
        self
    }

    /// An operand whose mode byte is outside the known set — exercises
    /// `decode()`'s tolerated-as-immediate-byte fallback.
    pub fn unknown_mode(&mut self, mode: u8, byte: u8) -> &mut Self {
        self.push_byte(mode);
        self.push_byte(byte)
    }

    /// Raw bytes with no operand-mode structure — for data regions (e.g.
    /// bytes an in-block `0x81`/`0x80` string operand targets, or
    /// filler after an unconditional GOTO).
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    /// Resolves all label fixups and returns the finished block. Panics if a
    /// referenced label was never defined, or the assembled block would
    /// exceed the `0x1E00`-byte ECL block size — both test-authoring bugs.
    pub fn build(&self) -> BlockBytes {
        let mut bytes = self.bytes.clone();
        assert!(
            bytes.len() <= ECL_BLOCK_SIZE,
            "EclBuilder: synthetic block ({} bytes) exceeds the 0x1E00-byte ECL block size",
            bytes.len()
        );
        for fixup in &self.fixups {
            let addr = *self
                .labels
                .get(&fixup.label)
                .unwrap_or_else(|| panic!("EclBuilder: undefined label {:?}", fixup.label));
            let [lo, hi] = addr.to_le_bytes();
            bytes[fixup.offset] = lo;
            bytes[fixup.offset + 1] = hi;
        }
        BlockBytes::from_bytes(&bytes)
    }
}

/// A fixed-sequence RNG for deterministic conformance tests
/// (`docs/design/vm-scriptmemory.md` §4: "fixed rng"). Cycles through
/// `values` in order once `values` is non-empty; returns 0 if never seeded.
#[derive(Debug, Clone, Default)]
pub struct FixedRng {
    values: VecDeque<u16>,
}

impl FixedRng {
    pub fn new(values: impl IntoIterator<Item = u16>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub fn push(&mut self, value: u16) {
        self.values.push_back(value);
    }
}

impl VmRng for FixedRng {
    fn roll_uniform(&mut self, inclusive_max: u16) -> u16 {
        self.values.pop_front().unwrap_or(0).min(inclusive_max)
    }
}

/// The composable mock `VmHost` (`docs/design/vm-scriptmemory.md` §4): a
/// `HashMap`-backed `ScriptMemory`, scripted `EngineServices` replies (one
/// `VecDeque` per reply-bearing method — pop the front when called, fall
/// back to a fixed default if the test never scripted one), a [`FixedRng`],
/// and a single ordered `calls` log covering every `ScriptMemory`/
/// `EngineServices` invocation (D-VM4: "full call recording").
///
/// Every `EngineServices` method is implemented, even the ones no opcode
/// this session's interpreter calls — the trait was declared in one shot
/// from `docs/design/opcode-classification.md` §3, and this mock must never
/// need to regrow underneath an already-shipped conformance test.
#[derive(Default)]
pub struct TestHost {
    words: HashMap<u16, u16>,
    bytes: HashMap<u16, u8>,
    strings: HashMap<u16, VmString>,
    pub calls: Vec<RecordedCall>,
    pub rng: FixedRng,

    pub retarget_selected_player_replies: VecDeque<Result<(), NotFound>>,
    pub free_current_player_replies: VecDeque<PlayerId>,
    pub party_strength_replies: VecDeque<u8>,
    pub check_party_replies: VecDeque<u16>,
    pub party_has_item_replies: VecDeque<bool>,
    pub find_special_replies: VecDeque<bool>,
    pub party_surprise_check_replies: VecDeque<(u8, u8)>,

    pub load_monster_replies: VecDeque<Result<MonsterHandle, MissingData>>,
    pub calc_group_movement_replies: VecDeque<(u8, u8)>,
    pub approach_distance_replies: VecDeque<u8>,

    pub create_item_replies: VecDeque<ItemHandle>,
    pub load_item_from_table_replies: VecDeque<ItemHandle>,
    pub find_spell_in_party_replies: VecDeque<(u8, u8)>,

    pub roll_replies: VecDeque<u8>,
    pub roll_dice_replies: VecDeque<u16>,
    pub roll_saving_throw_replies: VecDeque<bool>,
    pub can_hit_target_replies: VecDeque<bool>,

    pub wall_roof_replies: VecDeque<u8>,
    pub wall_type_replies: VecDeque<u8>,
    pub call_sound_variant_replies: VecDeque<u8>,
}

impl TestHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seeds a word-addressable memory cell (Area/Table/Party/Global
    /// windows — anything the engine's real `ScriptMemory` impl would
    /// resolve to a stored word). Ecl-window addresses never reach a
    /// `ScriptMemory` impl (the VM intercepts them, D-VM5); seeding one here
    /// has no effect on interpreter behavior, only on direct test assertions.
    pub fn set_word(&mut self, addr: u16, value: u16) {
        self.words.insert(addr, value);
    }

    pub fn word(&self, addr: u16) -> Option<u16> {
        self.words.get(&addr).copied()
    }

    fn next_or_default<T: Default>(queue: &mut VecDeque<T>) -> T {
        queue.pop_front().unwrap_or_default()
    }
}

impl ScriptMemory for TestHost {
    fn read(&mut self, addr: u16, origin: Origin) -> u16 {
        self.calls.push(RecordedCall::MemRead { addr, origin });
        self.words.get(&addr).copied().unwrap_or(0)
    }

    fn write(&mut self, addr: u16, value: u16, origin: Origin) {
        self.calls.push(RecordedCall::MemWrite {
            addr,
            value,
            origin,
        });
        self.words.insert(addr, value);
    }

    fn read_byte(&mut self, addr: u16, origin: Origin) -> u8 {
        self.calls.push(RecordedCall::MemReadByte { addr, origin });
        self.bytes.get(&addr).copied().unwrap_or(0)
    }

    fn write_byte(&mut self, addr: u16, value: u8, origin: Origin) {
        self.calls.push(RecordedCall::MemWriteByte {
            addr,
            value,
            origin,
        });
        self.bytes.insert(addr, value);
    }

    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString {
        self.calls
            .push(RecordedCall::MemReadString { addr, origin });
        self.strings.get(&addr).cloned().unwrap_or_default()
    }

    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin) {
        self.calls.push(RecordedCall::MemWriteString {
            addr,
            value: s.clone(),
            origin,
        });
        self.strings.insert(addr, s.clone());
    }
}

impl EngineServices for TestHost {
    fn retarget_selected_player(&mut self, index: u8) -> Result<(), NotFound> {
        self.calls
            .push(RecordedCall::RetargetSelectedPlayer { index });
        self.retarget_selected_player_replies
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn free_current_player(&mut self, free_icon: bool, leave_party_size: bool) -> PlayerId {
        self.calls.push(RecordedCall::FreeCurrentPlayer {
            free_icon,
            leave_party_size,
        });
        Self::next_or_default(&mut self.free_current_player_replies)
    }

    fn party_strength(&mut self) -> u8 {
        self.calls.push(RecordedCall::PartyStrength);
        Self::next_or_default(&mut self.party_strength_replies)
    }

    fn check_party(&mut self, query: u16) -> u16 {
        self.calls.push(RecordedCall::CheckParty { query });
        Self::next_or_default(&mut self.check_party_replies)
    }

    fn party_has_item(&mut self, item_type: u8) -> bool {
        self.calls.push(RecordedCall::PartyHasItem { item_type });
        Self::next_or_default(&mut self.party_has_item_replies)
    }

    fn find_special(&mut self, affect_type: u8) -> bool {
        self.calls.push(RecordedCall::FindSpecial { affect_type });
        Self::next_or_default(&mut self.find_special_replies)
    }

    fn destroy_items(&mut self, item_type: u8) {
        self.calls.push(RecordedCall::DestroyItems { item_type });
    }

    fn rob_money(&mut self, pct: u8) {
        self.calls.push(RecordedCall::RobMoney { pct });
    }

    fn rob_items(&mut self, chance: u8) {
        self.calls.push(RecordedCall::RobItems { chance });
    }

    fn party_surprise_check(&mut self) -> (u8, u8) {
        self.calls.push(RecordedCall::PartySurpriseCheck);
        Self::next_or_default(&mut self.party_surprise_check_replies)
    }

    fn load_monster(
        &mut self,
        monster_id: u8,
        num_copies: u8,
        icon_block_id: u8,
    ) -> Result<MonsterHandle, MissingData> {
        self.calls.push(RecordedCall::LoadMonster {
            monster_id,
            num_copies,
            icon_block_id,
        });
        self.load_monster_replies
            .pop_front()
            .unwrap_or(Ok(MonsterHandle::default()))
    }

    fn setup_monster(&mut self, sprite_id: u8, max_distance: u8, pic_id: u8) {
        self.calls.push(RecordedCall::SetupMonster {
            sprite_id,
            max_distance,
            pic_id,
        });
    }

    fn clear_monsters(&mut self) {
        self.calls.push(RecordedCall::ClearMonsters);
    }

    fn add_npc(&mut self, monster_id: u8, morale: u8) {
        self.calls.push(RecordedCall::AddNpc { monster_id, morale });
    }

    fn setup_duel(&mut self, is_duel: bool) {
        self.calls.push(RecordedCall::SetupDuel { is_duel });
    }

    fn calc_group_movement(&mut self) -> (u8, u8) {
        self.calls.push(RecordedCall::CalcGroupMovement);
        Self::next_or_default(&mut self.calc_group_movement_replies)
    }

    fn approach_distance(&mut self) -> u8 {
        self.calls.push(RecordedCall::ApproachDistance);
        Self::next_or_default(&mut self.approach_distance_replies)
    }

    fn load_encounter_visual(&mut self, flags: u8, distance: u8, pic_id: u8, sprite_id: u8) {
        self.calls.push(RecordedCall::LoadEncounterVisual {
            flags,
            distance,
            pic_id,
            sprite_id,
        });
    }

    fn create_item(&mut self, item_type: u8) -> ItemHandle {
        self.calls.push(RecordedCall::CreateItem { item_type });
        Self::next_or_default(&mut self.create_item_replies)
    }

    fn load_item_from_table(&mut self, block_id: u8) -> ItemHandle {
        self.calls
            .push(RecordedCall::LoadItemFromTable { block_id });
        Self::next_or_default(&mut self.load_item_from_table_replies)
    }

    fn find_spell_in_party(&mut self, spell_id: u8) -> (u8, u8) {
        self.calls.push(RecordedCall::FindSpellInParty { spell_id });
        self.find_spell_in_party_replies
            .pop_front()
            .unwrap_or((0xFF, 0xFF))
    }

    fn roll(&mut self, max: u8) -> u8 {
        self.calls.push(RecordedCall::Roll { max });
        self.roll_replies.pop_front().unwrap_or(0)
    }

    fn roll_dice(&mut self, size: u8, count: u8) -> u16 {
        self.calls.push(RecordedCall::RollDice { size, count });
        Self::next_or_default(&mut self.roll_dice_replies)
    }

    fn roll_saving_throw(&mut self, bonus: u8, save_type: u8) -> bool {
        self.calls
            .push(RecordedCall::RollSavingThrow { bonus, save_type });
        Self::next_or_default(&mut self.roll_saving_throw_replies)
    }

    fn can_hit_target(&mut self, bonus: u8) -> bool {
        self.calls.push(RecordedCall::CanHitTarget { bonus });
        Self::next_or_default(&mut self.can_hit_target_replies)
    }

    fn apply_damage(&mut self, player: PlayerId, damage: u16) {
        self.calls
            .push(RecordedCall::ApplyDamage { player, damage });
    }

    fn load_3d_map(&mut self, block_id: u8) {
        self.calls.push(RecordedCall::Load3dMap { block_id });
    }

    fn load_walldef(&mut self, set: u8, id: u8) {
        self.calls.push(RecordedCall::LoadWalldef { set, id });
    }

    fn load_bigpic(&mut self, id: u8) {
        self.calls.push(RecordedCall::LoadBigpic { id });
    }

    fn reset_wall_set(&mut self, index: u8) {
        self.calls.push(RecordedCall::ResetWallSet { index });
    }

    fn step_game_time(&mut self, time_slot: u8, amount: u8) {
        self.calls
            .push(RecordedCall::StepGameTime { time_slot, amount });
    }

    fn move_position_forward(&mut self) {
        self.calls.push(RecordedCall::MovePositionForward);
    }

    fn wall_roof(&mut self) -> u8 {
        self.calls.push(RecordedCall::WallRoof);
        Self::next_or_default(&mut self.wall_roof_replies)
    }

    fn wall_type(&mut self) -> u8 {
        self.calls.push(RecordedCall::WallType);
        Self::next_or_default(&mut self.wall_type_replies)
    }

    fn call_sound_variant(&mut self) -> u8 {
        self.calls.push(RecordedCall::CallSoundVariant);
        Self::next_or_default(&mut self.call_sound_variant_replies)
    }
}

impl VmHost for TestHost {
    fn rng(&mut self) -> &mut dyn VmRng {
        &mut self.rng
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{decode, Arg, Op};
    use crate::dialect::COTAB;

    #[test]
    fn builds_a_linear_goto_with_a_forward_label() {
        let mut b = EclBuilder::new();
        b.op(0x01).imm_word_label("target"); // GOTO target
        b.label("target");
        b.op(0x00); // EXIT

        let block = b.build();
        let instr = decode(&block, ECL_BLOCK_BASE, &COTAB).unwrap();
        assert_eq!(instr.op, Op(0x01));
        assert_eq!(instr.args, vec![Arg::ImmWord(b.addr_of("target"))]);
    }

    #[test]
    fn here_tracks_the_next_write_address() {
        let mut b = EclBuilder::new();
        assert_eq!(b.here(), ECL_BLOCK_BASE);
        b.op(0x00);
        assert_eq!(b.here(), ECL_BLOCK_BASE + 1);
    }

    #[test]
    #[should_panic(expected = "duplicate label")]
    fn duplicate_labels_panic() {
        let mut b = EclBuilder::new();
        b.label("x");
        b.op(0x00);
        b.label("x");
    }

    #[test]
    #[should_panic(expected = "undefined label")]
    fn unresolved_label_panics_on_build() {
        let mut b = EclBuilder::new();
        b.op(0x01).imm_word_label("nowhere");
        let _ = b.build();
    }
}
