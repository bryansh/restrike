//! Combat — the **initiative subsystem** (M4 D-OR5(a) Phase 1, first slice;
//! `docs/design/combat-study.md` §2/§4, `docs/design/oracle-rig.md`
//! D-OR5(a)/D-OR1).
//!
//! This is deliberately *only* the round scaffold plus the two initiative
//! routines — the single most draw-critical, most-landmine-prone part of combat
//! (study §14). **No attacks, no AI, no damage, no movement, no map.** The turn
//! slot is a documented stub that consumes **zero** PRNG draws (it just zeroes
//! the picked combatant's `delay` so it isn't re-picked), which makes this
//! session's draw stream pure initiative — the cleanest possible parity target.
//!
//! The two routines, transliterated from coab (read-for-behavior, D11):
//!
//! - **`CalculateInitiative`** (`ovr014.cs:8`, `sub_3E000`): one `roll_dice(6,1)`
//!   per in-combat combatant plus its DEX reaction adjustment, clamped, with a
//!   team-surprise `-6`. Exactly one d6 draw per in-combat combatant, in roster
//!   order (`ovr009.cs:39-42` drives it over `gbl.TeamList`).
//! - **`FindNextCombatant`** (`ovr009.cs:59`, `sub_331BC`): a selection loop that
//!   rolls **one d100 per roster member on *every* pass** (study §14 landmine 1:
//!   the per-round d100 count is `(A+1)·K`, not `A`) and yields the highest-delay
//!   member, ties broken by the highest roll — the exact two-`if` shape at
//!   `ovr009.cs:74-86`.
//!
//! Draw discipline (D9/D-OR1): every draw flows through the engine's single
//! `EngineRng` seam, so an attached [`crate::rng::RngSink`] observes it. Dice use
//! the `roll_dice` shape `1 + random(size)` per die (`ovr024.cs:586-598`) — the
//! same formula the vmhost roller uses, over the same PRNG; not a second path.
//!
//! Combat is entered from a **caller-provided roster** ([`CombatState::new`]);
//! wiring it to the ECL `COMBAT` opcode / `BattleSetup` is a later session.

use crate::monster::LoadedMonster;
use crate::rng::EngineRng;
use gbx_formats::affects::AffectRecord;
use gbx_formats::geo::GeoBlock;
use gbx_rules::flavor::Flavor;

mod records;
pub use records::{combat_state_from_records, RecordCombatant};

mod affects;
use affects::CheckType;
mod ai;
mod attack;
pub use ai::field_15_mode_gate;
mod facing;

/// One `roll_dice(size, count)` (`ovr024.cs:586-598`): `count` dice, each
/// `1 + random(size)`, through the engine's one PRNG seam so an attached
/// `RngSink` sees every draw. This mirrors the vmhost roller (`vmhost.rs`
/// `roll_dice`) exactly — same formula, same `EngineRng` — rather than opening a
/// second RNG path (D9/D-OR1). `size == 0` still draws (`random(0)` advances then
/// returns 0 → die value 1), the faithful binary behavior.
///
/// **Byte truncation (`(byte)roll_total`, `ovr024.cs:595`):** the original sums
/// as an `int` then truncates the total to a byte. Observable only when
/// `count * size > 255` (FD-29 — the data-driven clause). For d6/d100 initiative
/// the sum never reaches 256, so the truncation is a no-op there; it matters for
/// weapon/monster damage dice, so it is applied here faithfully. The `u32`
/// accumulator avoids intermediate overflow before the truncation.
fn roll_dice(rng: &mut EngineRng, size: u16, count: u16) -> u16 {
    let mut total = 0u32;
    for _ in 0..count {
        total += 1 + rng.random(size) as u32;
    }
    (total as u8) as u16 // (byte)roll_total — ovr024.cs:595
}

/// The stalemate cap: `combat_round_no_action_value` (`Classes/Gbl.cs:384`),
/// the initial value of `combat_round_no_action_limit` (`byte_1D8B8`).
/// `BattleRoundChecks` ends the fight once `combat_round >= this`
/// (`ovr009.cs:399`), guaranteeing termination even when neither side can finish
/// the other — the only terminator in this slice, since the stub kills no one.
pub const DEFAULT_NO_ACTION_LIMIT: u16 = 15;

/// Which side a combatant fights on. The discriminants mirror coab's
/// `CombatTeam` (`Classes/Enums.cs:91` — `Ours = 0`, `Enemy = 1`) because the
/// surprise test is bit `(team + 1)` of the per-round mask (`ovr014.cs:38`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    /// `CombatTeam.Ours` — the player party.
    Party = 0,
    /// `CombatTeam.Enemy` — the loaded monsters.
    Monster = 1,
}

/// One combatant in a fight — **the single, unified combatant record** (M4
/// combat #5, model unification). Carries everything any slice of the engine
/// reads: the initiative inputs (`team`, `reaction_adj`, `in_combat`, `delay`),
/// the tactical state (`pos`, `facing`/`direction`, footprint `size`), the
/// combat stats (`hp`, `ac`, `hit_bonus`, the readied melee attack profile), and
/// the persistent per-combatant `Action` scratch the QuickFight AI mutates
/// (`field_15`, `target`, morale flags). Before this slice the engine carried
/// *two* records — a lightweight initiative-only `Combatant` and a rich
/// `Fighter` — which is why the fields split into an initiative core and an
/// AI/tactical remainder; the merge folds them onto one struct so the one
/// tick-based engine ([`CombatState`]) works over one type.
///
/// **The former `Fighter` name is preserved as [`Fighter`] (a type alias)** so
/// every audit-accepted slice-4 test and both demos keep constructing it by that
/// name, byte-for-byte unchanged — the unification changed the *type*, not the
/// call sites.
///
/// The lightweight initiative harness ([`CombatState::initiative_only`]) builds
/// these with [`Combatant::new`] / [`Combatant::from_dex`], leaving the tactical
/// fields at inert defaults (it never runs a real turn); a real fight builds them
/// with [`Combatant::new_melee`]. Real construction from a party `Player` / a
/// `LoadedMonster` lands with the `COMBAT`-opcode wiring; the caller assembling
/// the roster owns the records.
///
/// **Scope:** a single **melee** attack profile (profile 1). The second attack
/// form (`attack2_*` dice) and the `ThisRoundActionCount` 3/2 derivation of
/// `attack{1,2}_left` are the initiative/`BattleSetup` concern (§3.1, FD-3) — the
/// turn faithfully consumes whatever `attack1_left`/`attack2_left` the combatant
/// carries (`attack2_left` defaults 0, so the `AttackTarget01` loop makes exactly
/// `attack1_left` swings with the profile-1 dice).
/// `Player.health_status@0x195` (`Status`, `Classes/Enums.cs`) reduced to the
/// values `damage_player` / the bandage / bleed paths key on (§26). The original
/// `Status` enum runs `okey=0 … gone=8`; a melee replay only ever moves a
/// combatant through **okey → {unconscious, dying, dead}**, and reads `animated`
/// in `damage_player`'s special-case (`new_hp == 0 && animated → dead`). The
/// other original values (`tempgone`/`running`/`stoned`/`gone`) are set only by
/// spell/affect paths (M5), so they are not modeled — an entry record carrying
/// one decodes to [`HealthStatus::Okey`] (documented on [`decode_health_status`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    /// `okey` (0) — conscious and fighting. Entry records are all okey.
    Okey,
    /// `animated` (1) — an animated-dead combatant; `damage_player` treats a
    /// `new_hp == 0` hit on an animated combatant as an outright kill.
    Animated,
    /// `unconscious` (4) — dropped to exactly 0 HP (no overkill); out of combat,
    /// not bleeding.
    Unconscious,
    /// `dying` (5) — dropped past 0 with 1..=9 overkill; out of combat and
    /// bleeding (`actions.bleeding`), bandageable, bleeds to `Dead` if untended.
    Dying,
    /// `dead` (6) — overkill > 9, or a `new_hp == 0` hit on an `animated`, or a
    /// bleed-out (`bleeding > 9`).
    Dead,
    /// `running` (3) — a combatant that fled and **Got Away** (`flee_battle` →
    /// `RemoveFromCombat(..., Status.running, ...)`, `ovr014:0D90`/`sub_644A7`).
    /// Out of combat; unlike every other removal, `RemoveFromCombat` **skips** the
    /// `hp_current = 0` write for a `running` combatant (`sub_644A7:151A`). Never
    /// present on an entry record — [`decode_health_status`] folds a raw `3` to
    /// [`HealthStatus::Okey`] as with the other non-entry states.
    Running,
}

impl HealthStatus {
    /// `damage_player`'s survivor test: a combatant whose status is `okey` or
    /// `animated` after the ladder keeps its HP and stays in combat; any other
    /// status flips `in_combat = false` (`ovr025.cs:1218`).
    fn is_conscious(self) -> bool {
        matches!(self, HealthStatus::Okey | HealthStatus::Animated)
    }
}

/// Decode the `health_status@0x195` byte onto the minimal [`HealthStatus`]. Entry
/// records are `okey` (0); the unmodeled `Status` values (`tempgone`/`running`/
/// `stoned`/`gone`, and any out-of-range byte) fold to [`HealthStatus::Okey`]
/// since a plain-melee replay never enters those states (they are spell/affect
/// outcomes, M5). `animated=1`/`unconscious=4`/`dying=5`/`dead=6` map through.
pub fn decode_health_status(byte: u8) -> HealthStatus {
    match byte {
        1 => HealthStatus::Animated,
        4 => HealthStatus::Unconscious,
        5 => HealthStatus::Dying,
        6 => HealthStatus::Dead,
        _ => HealthStatus::Okey,
    }
}

#[derive(Debug, Clone)]
pub struct Combatant {
    /// Stable per-encounter roster index (the D-OR3 `combatant_id`).
    pub id: usize,
    /// Party or monster (`Player.combat_team`, `@0x18b`-ish runtime cell).
    pub team: Team,
    /// `control_morale >= Control.NPC_Base` — an NPC/monster. **Only NPCs draw the
    /// per-step morale-advance d100** (`moralFailureEscape:387`); PCs short-circuit
    /// it. Also gates the `FleeCheck_001` morale block.
    pub npc: bool,
    /// `control_morale@0xF7` (the raw byte). `FleeCheck_001` reseeds
    /// `monster_morale = (control_morale & 0x7F) << 1` **per actor, every call**
    /// (`sub_3637F` @`ovr010:13F1`, §28) — the deviation slice-2 replaces (the old
    /// stub used a process-lifetime scratch). [`Combatant::npc`] is
    /// `control_morale >= 0x80`, but the ladder needs the raw byte for the seed.
    pub control_morale: u8,
    /// `Intelligence@0x13` (`stats2.Int.original`, the record byte the FleeCheck
    /// surrender branch reads: `sub_3637F` @`ovr010:14FA`, `cmp es:[di+13h], 5`).
    /// A combatant reaching the surrender fork **surrenders only when `Int > 5`**
    /// (§28 item 7). Default 0 (never surrenders) for synthetic combatants.
    pub int_score: u8,
    /// Footprint size (`field_DE & 7`); combat uses 1 for single-cell combatants.
    pub size: u8,
    pub pos: GridPos,
    /// `player.in_combat` (`ovr014.cs:29`): a not-in-combat combatant gets
    /// `delay = 0` and rolls **no** d6; a killed combatant flips this false and is
    /// excluded from target lists / occupancy.
    pub in_combat: bool,
    pub hp_current: i32,
    pub hp_max: i32,
    /// Raw on-disk AC (`Player.ac@0x19a`; display AC = `0x3C - ac`).
    pub ac: u8,
    /// `Player.ac_behind@0x19b` — the rear armor class. `AttackTarget01`
    /// (`sub_3F4EB` @`ovr014:16F7-1700`) selects the to-hit AC by INDEXING
    /// `record[0x19A + behind]` (`add di, ax; mov al, es:[di+19Ah]`): front
    /// 0x19A, behind 0x19B. A departure opportunity attack is always behind
    /// (`AttackTarget(null, 1, …)`, coab ovr014.cs:407). Backstab reads
    /// `[0x19B] − 4` (`ovr014:169E-16A5`) — deferred (M5) with backstab.
    pub ac_behind: u8,
    /// `attacker.hitBonus@0x199` (THAC0-derived to-hit number).
    pub hit_bonus: i32,
    /// `HitDice` — `TrySweepAttack` only sweeps `HitDice == 0` targets.
    pub hit_dice: u8,
    /// Base movement (`player.movement`) → [`calc_moves`] at initiative.
    pub movement: i32,
    /// `DexReactionAdj(player)` (`ovr025.cs:537`) — a table lookup, no draw —
    /// precomputed via `gbx-rules`' `Flavor::dex_reaction_bonus`. Range `-4..=5`.
    pub reaction_adj: i8,
    /// `Player.class@0x75`. The QuickFight approach guards a **pure Magic-User**
    /// (`class == 5`): §15 bug #4, `sub_359D1` @`ovr010:0AA3` — a non-fleeing
    /// class-5 combatant with a null [`Combatant::field_159`] does **not** advance
    /// (PHILIPPE the mage holds his corner all fight). Default 0 (no guard) for
    /// synthetically-built combatants.
    pub class: u8,
    /// `Player.field_159@0x159` (a runtime far-pointer, 4 bytes) is **null** here.
    /// The mage-hold guard (§15 bug #4) only fires when this is null; a mage with a
    /// readied `field_159` (a ranged option) advances instead. In the entry-state
    /// snapshot it is whatever the capture recorded (null in the bar brawl). Like
    /// the §1.7 pointer fields it is not otherwise decoded. Default `true` (null).
    pub field_159_null: bool,
    /// Base attack half-actions (`attacksCount@0x11c`) — `reclac_attacks`/
    /// `ThisRoundActionCount` fold this into `attack1_left` each round (the 3/2
    /// rule, §3.1). `2` = one attack per round.
    pub attacks_count: u8,
    // --- readied melee attack profile 1 ---
    pub dice_count: u8,
    pub dice_size: u8,
    pub damage_bonus: u8,
    // --- Action scratch (per-round / persistent) ---
    /// `action.delay@0x03` — the initiative/turn-order key. Reset each round by
    /// [`CombatState`]; zeroed when the combatant's turn completes.
    pub delay: i8,
    /// `action.move@0x06` — half-move budget this round ([`calc_moves`]).
    pub move_left: i32,
    /// `attack1_AttacksLeft@0x19c` — profile-1 swings left this round.
    pub attack1_left: u8,
    /// `attack2_AttacksLeft@0x19d` — profile-2 swings (0 for single-form melee).
    pub attack2_left: u8,
    /// `action.attackIdx@0x04` — starts 2 (`CalculateInitiative`), the profile the
    /// `AttackTarget01` loop counts down from.
    pub attack_idx: u8,
    /// `action.field_15@0x15` — the **persistent** target-mode scratch
    /// ([`field_15_mode_gate`]); `Action.Clear` does NOT reset it.
    pub field_15: u8,
    /// `action.target@0x0A` — the current target roster index; persists across
    /// turns (`Action.Clear` doesn't reset it) until invalidated.
    pub target: Option<usize>,
    pub moral_failure: bool,
    pub fleeing: bool,
    /// `action.guarding@0x07` — set by `TryGuarding`; consumed by opportunity
    /// attacks (`move_step_into_attack`).
    pub guarding: bool,
    /// `action.can_use@0x02` — may use an item this round (set true at initiative);
    /// the `sub_354AA` wand-scan guard.
    pub can_use: bool,
    /// `action.direction@0x09` — facing; set to the move heading by each step.
    pub direction: u8,
    /// `action.AttacksReceived@0x0F` — attacks taken since the last move.
    pub attacks_received: u8,
    /// `action.directionChanges@0x12` — the accumulated facing-swing count, mod 8.
    /// `RecalcAttacksReceived` (`sub_3F94D` @`ovr014:19C2-19D1`) folds each swing's
    /// `dirDiff` in `(direction_changes + dirDiff) % 8`; reset to 0 at the turn head
    /// (`ovr009:029C`) and every movement step (`ovr014:090F`). Read ONLY by the
    /// flanking heuristic (`> 4`, `ovr014:16BA`). Values only ever 0..7.
    pub direction_changes: u8,
    /// The count of non-zero `spellList`@0x1E slots on the source record — an
    /// approximation of coab's `player.spells.Count`, decoded ONLY to drive the
    /// `memorized-spells` stub tripwire (`sub_3560B`'s inner spell-selection
    /// draws are unmodeled, M5). `0` for synthetic combatants.
    pub memorized_spells: u8,
    /// `Player.health_status@0x195` — the downed-PC ladder (§26). Entry records
    /// are [`HealthStatus::Okey`]; `damage_player` moves a downed combatant to
    /// `dying`/`unconscious`/`dead` (`apply_damage`), the bleed tick advances
    /// `dying → dead`, and a bandage turn advances `dying → unconscious`.
    pub health_status: HealthStatus,
    /// `action.bleeding@0x13` (offset within the `Action` struct) — the overkill
    /// carried into `dying` by `damage_player` (`bleeding = neg_hp`); the bleed
    /// tick adds 1/round and kills at `> 9`; a bandage zeroes it. `0` for a
    /// combatant that is not dying.
    pub bleeding: u8,

    // --- the armed/ranged loadout (M5 armed slice, doc §34) ----------------
    /// The additive per-combatant ranged loadout (doc §34.1). `None` = today's
    /// behaviour — range-1 melee, the record's readied profile as-is, weapon
    /// selection inert. `Some` supplies the readied primary-weapon type
    /// (`field_151`), the launcher's ammo, and the bare-hands profile the AI
    /// swaps to when cornered.
    pub loadout: Option<Loadout>,
    /// `player.activeItems.primaryWeapon != null` — is the loadout's primary
    /// weapon currently readied (`field_151` non-null)? Starts `true` when a
    /// loadout is applied; the cornered weapon-selection AI toggles it (unready
    /// → bare hands, re-ready → the bow). Always `false` without a loadout, so
    /// the ranged predicates read melee (doc §34.2).
    pub weapon_readied: bool,
    /// The launcher's ammo count (`item.count`@item+0x39, doc §34.3/§34.6) — the
    /// arrows/quarrels remaining. Decremented by the swing count each ranged
    /// attack (coab≠binary #16: the binary SUBTRACTS). `0` without a loadout.
    pub ammo: i32,
    /// `false` once the launcher's ammo item has been lost to depletion
    /// (`item.count == 0` → `lose_item`, doc §34.6) — `GetCurrentAttackItem`
    /// then finds no ammo. Unexercised by armed-bar (ammo ≥ usage); cheap.
    pub ammo_item_lost: bool,
    /// The saved readied attack-1 profile (`dice_count`, `dice_size`,
    /// `damage_bonus` @0x19E/0x1A0/0x1A2 at entry) — what re-readying the bow
    /// restores after a cornered unready swapped in the bare-hands profile
    /// (doc §34.5). Set to the record's decoded profile at construction.
    pub entry_dice: (u8, u8, u8),
    /// `action.field_8@0x08` — set `true` by `AttackTarget01` (`ovr014.cs:738`),
    /// reset by `CalculateInitiative` (`sub_3E000`, §32). Gates the
    /// `reclac_attacks` write-back (doc §34.3). `false` at entry.
    pub field_8: bool,
    /// `field_DE@0xde` (raw) — icon dimensions / footprint. The large-target
    /// dice-substitution gate (`> 0x80 || (&7) > 1`, deferred) and
    /// `CanBackStabTarget`'s size gate (`(& 0x7F) <= 1`, doc §34.6) read it.
    /// `0x01` (man-sized single cell) for synthetic combatants. UNREAD until
    /// the facing slice (both consumers were reverted, §35) — decoded now
    /// because it is record-derived and the next slice consumes it.
    pub field_de: u8,
    /// The attack-2 profile (`dice_count`, `dice_size`, `damage_bonus`
    /// @0x19F/0x1A1/0x1A3) — `sub_3E192`'s idx-2 damage cells (doc §34.6). All
    /// zero in this party (attack-2 never swings); decoded for fidelity.
    pub attack2_dice: (u8, u8, u8),
    /// `baseHalfMoves@0x11D` — the attack-2 half-count `CalculateInitiative`
    /// folds through `ThisRoundActionCount` into `attack2_left` (doc §34.3).
    /// `0` in this party (so attack-2 stays 0).
    pub base_half_moves: u8,
    /// `SkillLevel(SkillType.Thief)` precomputed from the record — the sum
    /// `ClassLevel[6] + ClassLevelsOld[6] * DualClassExceedsPreviousLevel`
    /// (coab `Player.cs:492` / `sub_6B3D1`) — the backstab-multiplier and
    /// `CanBackStabTarget` input (doc §34.6). Constant during a fight. `0` for
    /// synthetic combatants. UNREAD until the facing slice (backstab was
    /// reverted, §35) — decoded now because it is record-derived and the next
    /// slice consumes it.
    pub thief_skill_level: i32,
    /// The **base** attack-1 profile (`attack1_DiceCountBase`@0x11E /
    /// `attack1_DiceSizeBase`@0x120 / `attack1_DamageBonusBase`@0x122) — the
    /// raw dice with no STR adjustment. `CalcItemPowerRating`'s baseline
    /// (`var_16`, doc §34.5) reads it: `dsB*dcB (+2*bonusB if >0)`. Distinct
    /// from the loadout's `unarmed_profile` (which folds in the STR adj).
    pub base_dice: (u8, u8, u8),

    // --- the affect substrate (M5 Phase 2, doc §39) ------------------------
    /// The combatant's active affects (`charStruct.affect_ptr`@0xF2 — a runtime
    /// heap list; doc §39.1/§39.6). **List order is load-bearing**: `add_affect`
    /// appends at the TAIL (`ovr024:13F0-14A4`), `find_affect` returns the FIRST
    /// match (`ovr025:2345`), and `remove_affect` drops ONE instance. A capture's
    /// record image cannot carry this list (@0xF2 is heap linkage), so every
    /// entry-state replay builds it **empty** — bit-for-bit today's behaviour, and
    /// the reason the substrate is draw-neutral for all eight guard pins (doc
    /// §39.2/§39.6). Real-play population (save `.FX` import, `MON<area>SPC` innate
    /// affects) is wired by their own slices.
    pub affects: Vec<AffectRecord>,
}

/// Which item a ranged swing draws from — the `out item` of
/// `GetCurrentAttackItem` (`sub_6906C`, doc §34.2), mapped onto our single-ammo
/// model. `None` = the item is null (nothing found, or a Sling's found-but-null
/// item — no ammo decrement); `Ammo` = the launcher's arrows/quarrels slot
/// (decrement the combatant's `ammo`); `SelfWeapon` = a self-launching weapon
/// (its own count, unmodeled in armed-bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttackItemRef {
    None,
    Ammo,
    SelfWeapon,
}

/// The `GetCurrentAttackItem` result: whether an attack item was `found`
/// (`item_found`, the `reclac_attacks` gate) and which [`AttackItemRef`] the
/// swing draws / decrements.
#[derive(Debug, Clone, Copy)]
struct CurrentAttackItem {
    found: bool,
    item: AttackItemRef,
}

/// The additive per-combatant ranged loadout (doc §34.1) — the entry-state
/// snapshot cannot recover item identity/ammo (they live behind runtime far
/// pointers the capture does not chase), so a fight with readied ranged weapons
/// supplies them here, committed per capture in the harness like the guard's
/// pins. `None` reproduces today's melee behaviour exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loadout {
    /// The readied primary weapon's item type (`field_151` weapon), indexing
    /// the [`crate::combat::CombatState`]'s `ItemDataTable`.
    pub primary_type: u8,
    /// The launcher's initial ammo count (a free parameter — any count ≥
    /// shots-fired replays identically; doc §34.1).
    pub ammo_count: i32,
    /// The bare-hands attack-1 profile (`dice_count`, `dice_size`,
    /// `damage_bonus`) the AI swaps to when cornered — base dice @0x11E/0x120
    /// plus the STR damage adjustment, pinned empirically (doc §34.1).
    pub unarmed_profile: (u8, u8, u8),
}

impl Combatant {
    /// An **initiative-harness** combatant with a directly-supplied reaction
    /// adjustment (the primitive [`CombatState::initiative_only`] and the D-OR3
    /// oracle tests use with hand-built rosters). Initiative reads only
    /// `in_combat`, `reaction_adj`, `team`, and `delay`; the tactical/AI fields are
    /// left at inert defaults (`pos (0,0)`, no hp, `field_15 = 0`, no target) since
    /// this construction never drives a real turn. Starts with `delay = 0`.
    pub fn new(id: usize, team: Team, reaction_adj: i8, in_combat: bool) -> Self {
        Combatant {
            id,
            team,
            npc: false,
            control_morale: 0,
            int_score: 0,
            size: 1,
            pos: GridPos::new(0, 0),
            in_combat,
            hp_current: 0,
            hp_max: 0,
            ac: 0,
            ac_behind: 0,
            hit_bonus: 0,
            hit_dice: 0,
            movement: 0,
            reaction_adj,
            class: 0,
            field_159_null: true,
            attacks_count: 0,
            dice_count: 0,
            dice_size: 0,
            damage_bonus: 0,
            delay: 0,
            move_left: 0,
            attack1_left: 0,
            attack2_left: 0,
            attack_idx: 2,
            field_15: 0,
            target: None,
            moral_failure: false,
            fleeing: false,
            guarding: false,
            can_use: true,
            direction: 0,
            attacks_received: 0,
            direction_changes: 0,
            memorized_spells: 0,
            health_status: HealthStatus::Okey,
            bleeding: 0,
            loadout: None,
            weapon_readied: false,
            ammo: 0,
            ammo_item_lost: false,
            entry_dice: (0, 0, 0),
            field_8: false,
            field_de: 0x01,
            attack2_dice: (0, 0, 0),
            base_half_moves: 0,
            thief_skill_level: 0,
            base_dice: (0, 0, 0),
            affects: Vec::new(),
        }
    }

    /// An initiative-harness combatant whose reaction adjustment is derived from
    /// its Dexterity through the rules flavor (`DexReactionAdj`, `ovr025.cs:537` —
    /// the mapping lives in `gbx-rules`, not hardcoded here). coab reads
    /// `stats2.Dex.full`.
    pub fn from_dex(id: usize, team: Team, dex: u8, in_combat: bool, flavor: &dyn Flavor) -> Self {
        Combatant::new(id, team, flavor.dex_reaction_bonus(dex) as i8, in_combat)
    }

    /// A single-cell **melee** combatant with a fresh turn state (`delay`/
    /// `move_left`/`attack1_left` supplied by the caller — normally from
    /// initiative). `field_15` starts 0, `attack_idx` 2, `can_use` true, no target —
    /// the `CalculateInitiative` reset state. This is the constructor a real fight
    /// (and both demos) uses; the former `Fighter::new_melee`.
    #[allow(clippy::too_many_arguments)]
    pub fn new_melee(
        id: usize,
        team: Team,
        npc: bool,
        pos: GridPos,
        hp: i32,
        ac: u8,
        hit_bonus: i32,
        movement: i32,
        dice: (u8, u8, u8),
        delay: i8,
        attack1_left: u8,
    ) -> Self {
        Combatant {
            id,
            team,
            npc,
            // A synthetic melee combatant has no raw morale/Int decode; the
            // faithful FleeCheck reseeds from `control_morale` (npc → 0x80 folds
            // to seed 0, PCs stay 0), and `int_score` 0 never surrenders. Tests
            // that exercise the ladder set these explicitly.
            control_morale: if npc { 0x80 } else { 0 },
            int_score: 0,
            size: 1,
            pos,
            in_combat: true,
            hp_current: hp,
            hp_max: hp,
            ac,
            ac_behind: ac,
            hit_bonus,
            hit_dice: 1,
            movement,
            reaction_adj: 0,
            class: 0,
            field_159_null: true,
            attacks_count: 2,
            dice_count: dice.0,
            dice_size: dice.1,
            damage_bonus: dice.2,
            delay,
            move_left: calc_moves(movement),
            attack1_left,
            attack2_left: 0,
            attack_idx: 2,
            field_15: 0,
            target: None,
            moral_failure: false,
            fleeing: false,
            guarding: false,
            can_use: true,
            direction: 0,
            attacks_received: 0,
            direction_changes: 0,
            memorized_spells: 0,
            health_status: HealthStatus::Okey,
            bleeding: 0,
            loadout: None,
            weapon_readied: false,
            ammo: 0,
            ammo_item_lost: false,
            entry_dice: (0, 0, 0),
            field_8: false,
            field_de: 0x01,
            attack2_dice: (0, 0, 0),
            base_half_moves: 0,
            thief_skill_level: 0,
            base_dice: (0, 0, 0),
            affects: Vec::new(),
        }
    }
}

/// **`Fighter` is the former name of the now-unified [`Combatant`].** Kept as a
/// type alias so the audit-accepted slice-4 tests and both demos construct the
/// record by the name they always used, unchanged by the merge.
pub type Fighter = Combatant;

/// A combat-action-profile event (D-OR3 `action` profile; study §9, pinned this
/// session for the initiative slice). Engine-local plain data emitted through
/// [`ActionSink`]; `gbx-oracle` translates these into canonical `.gbxtrace`
/// events, so `gbx-engine` never depends on `gbx-oracle` (the [`crate::rng`]
/// `RngSink` pattern, mirrored).
///
/// Emission order honors the D-OR3 same-tick contract: within a round, each
/// combatant's `Init` is emitted right after its d6; each `Pick` right after the
/// pass that selected it — so the `action` stream stays index-alignable with the
/// `prng` stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionEvent {
    /// Per combatant in `CalculateInitiative`, bracketing its one `random(6)`.
    /// `delay` is the final assigned value; `dex_adj` the reaction adjustment
    /// added; `surprise` whether the team `-6` fired for this combatant.
    Init {
        combatant_id: usize,
        delay: i8,
        dex_adj: i8,
        surprise: bool,
    },
    /// Per `FindNextCombatant` selection (one per yielded combatant). `pass` is
    /// the 0-based selection-pass index within the round; `roll` the winning
    /// d100; `delay` the winning combatant's delay at selection time.
    Pick {
        pass: u32,
        combatant_id: usize,
        delay: i8,
        roll: u16,
    },
    /// Per to-hit resolution ([`resolve_attack`] → `PC_CanHitTarget`,
    /// `ovr024.cs:515`), bracketing the one `random(20)`. `roll` is the **raw
    /// d20 (1..=20, before the natural-20 promotion to 100)** — the honest
    /// observable die, from which nat-1 (auto-miss) and nat-20 (auto-hit) are
    /// both visible; `hit` is the resolved outcome.
    Attack {
        attacker_id: usize,
        target_id: usize,
        roll: u8,
        hit: bool,
    },
    /// Per damage roll ([`roll_damage`] → `sub_3E192`, `ovr014.cs:84`), emitted
    /// only on a hit (the original rolls damage only inside the hit branch).
    /// `amount` is the final damage (dice + bonus, clamped `>= 0`, times the
    /// backstab multiplier); `backstab` whether that multiplier was applied. It
    /// brackets the `dice_count` `random(dice_size)` draws.
    Dmg {
        attacker_id: usize,
        target_id: usize,
        amount: i32,
        backstab: bool,
    },
    /// Per saving throw ([`roll_saving_throw`] → `RollSavingThrow`,
    /// `ovr024.cs:554`), bracketing its one `random(20)`. `roll` is the raw d20
    /// (1..=20); `save_type` the `SaveVerseType` index; `made` the outcome.
    Save {
        combatant_id: usize,
        save_type: u8,
        roll: u8,
        made: bool,
    },
    /// Per melee AI turn (`PlayerQuickFight`, `ovr010.cs:8`), emitted once the
    /// turn's target is resolved. Pins the study §9 `ai` vocabulary now that the AI
    /// lands: `field_15` is the (post-gate) target-mode scratch, `target_id` the
    /// chosen target (`-1` = none/guarding). Draw-bracketing is loose here (the
    /// turn's draws are the mode-gate + behavior d7s + find_target + the swing);
    /// the event marks *which* combatant acted with what mode/target.
    Ai {
        combatant_id: usize,
        field_15: u8,
        /// Roster index of the target, or `-1` when none (guarding / no reachable
        /// enemy). Integer-encoded per D-OR3 (no `Option` on the wire).
        target_id: i64,
    },
    /// Per morale/advance decision — the `FleeCheck_001` outcome and the
    /// `moralFailureEscape:387` advance gate (§6.2). `roll` is the advance d100 (a
    /// monster draws it; `0` when none was drawn, e.g. a PC or a draw-free
    /// `FleeCheck`); `failed` is `moral_failure`. Brackets the 0-or-1 `random(100)`.
    Morale {
        combatant_id: usize,
        monster_morale: i32,
        enemy_hp_pct: i32,
        roll: u16,
        failed: bool,
    },
    /// Per movement step (`sub_3E748`, `ovr014.cs:252`): the from/to cells and the
    /// half-move `cost`. Draw-free (movement rolls no dice; the per-step monster
    /// d100 is the separate `Morale` event that precedes the step).
    Move {
        combatant_id: usize,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        cost: i32,
    },
    /// A deliberately-stubbed original mechanic was **reached** this fight (the
    /// M5 ledger, doc §24): the engine took its modeled path, but the binary
    /// would have consulted a subsystem we have not built — so from this point
    /// the replay is in unproven territory even if the draw stream still
    /// matches. **Diagnostic only**: never part of the `.gbxtrace` vocabulary
    /// (the oracle collector drops it); the replay harnesses report it so a
    /// capture that wanders into a stub names itself instead of silently
    /// diverging. `stub` is a short stable name: `"memorized-spells"`,
    /// `"0-hd-sweep"`, `"surrender-int5"` (the `"downed-pc"` wire was retired
    /// once the downed-PC path was built, §26/§27).
    StubTripped {
        combatant_id: usize,
        stub: &'static str,
    },
}

/// The engine's action-trace seam (D-OR3, task deliverable 4), mirroring
/// [`crate::rng::RngSink`]: the core stays pure, and an observer is attached only
/// when a differential run wants one. The trait lives here, on the engine side;
/// `gbx-oracle` provides the `.gbxtrace`-writing implementation. Inert when
/// unattached ([`CombatState::emit`] pays a single `Option::is_some` branch).
pub trait ActionSink {
    /// Called once per emitted event, in emission order (D-OR3 contract).
    fn on_action(&mut self, event: ActionEvent);
}

/// What one [`CombatState::step`] produced. The round advances tick-by-tick (D8:
/// no blocking loop; control returns each step), so a caller drives combat with a
/// `loop { match state.step(rng) { … } }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatStep {
    /// A new round began: `CountCombatTeamMembers` ran and initiative was rolled
    /// for every combatant (one d6 per in-combat member, roster order).
    RoundStarted { round: u16 },
    /// The next combatant to act, in draw order. Its turn is the stub for this
    /// slice: `delay` is already zeroed (zero draws) so it is not re-picked. A
    /// later slice drives a real turn here.
    Turn { combatant_id: usize },
    /// End-of-round checks ran (`BattleRoundChecks`, `ovr009.cs:363`). The
    /// terminating empty selection pass consumed its K d100 draws first (study
    /// §14 landmine 1). `battle_over` is the loop-exit decision.
    RoundEnded { round: u16, battle_over: bool },
    /// Combat is over; further `step` calls stay `Ended`.
    Ended,
}

/// Where the tick machine is between [`CombatState::step`] calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Next step counts teams and rolls initiative.
    RoundStart,
    /// Next step runs one `FindNextCombatant` selection pass.
    Selecting,
    /// Terminal.
    Ended,
}

/// How the `Turn` phase of [`CombatState::step`] resolves the acting combatant's
/// turn — the faithful "turn dispatcher" `MainCombatLoop` runs (`ovr009.cs:59`):
/// coab dispatches each picked combatant to a turn handler (the interactive
/// player menu, `DoPlayerCombatTurn`, or the QuickFight AI). This engine models
/// two of those:
///
/// - **`MeleeAi`** — the real `PlayerQuickFight` melee turn ([`CombatState::melee_ai_turn`]),
///   drawing the turn's dice. A full fight ([`CombatState::new`]).
/// - **`Stub`** — a zero-draw turn that just zeroes the picked combatant's `delay`
///   so it isn't re-picked. This exposes the initiative/selection subsystem in
///   isolation — the cleanest possible parity target (study §2/§14) — and is what
///   [`CombatState::initiative_only`] configures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnDriver {
    /// Zero-draw turn (initiative/selection harness).
    Stub,
    /// The `PlayerQuickFight` melee AI turn.
    MeleeAi,
}

/// The round loop's state (`MainCombatLoop`, `ovr009.cs:22`): the roster (in
/// `TeamList` order — draw order depends on iteration order, so this ordering is
/// load-bearing), the round counter, the stalemate cap, and the per-round
/// surprise mask. Runs `count → initiative → turns → BattleRoundChecks` as a
/// tick-based skeleton.
pub struct CombatState {
    /// `gbl.TeamList` (`Classes/Gbl.cs:496`) — party then monsters, iteration
    /// order preserved. (The former `CombatWorld.fighters`; draw order depends on
    /// this ordering, so it is load-bearing.)
    pub fighters: Vec<Combatant>,
    /// The combat battlefield: terrain + occupancy (`SetupGroundTiles` →
    /// `PlaceCombatants`, §11). The [`initiative_only`](CombatState::initiative_only)
    /// harness leaves this an all-void placeholder (initiative never reads it).
    pub map: CombatMap,
    /// `gbl.combat_round` (`Classes/Gbl.cs:382` = `byte_1D8B7`); `++` in
    /// `BattleRoundChecks` (`ovr009.cs:366`). Held as `u16`; the byte never
    /// overflows because the fight ends at `no_action_limit` (15).
    combat_round: u16,
    /// `gbl.combat_round_no_action_limit` (`byte_1D8B8`), initialized to
    /// [`DEFAULT_NO_ACTION_LIMIT`].
    no_action_limit: u16,
    /// `gbl.area2_ptr.field_596` — the per-round team surprise/init-bonus mask
    /// read by `CalculateInitiative` (`ovr014.cs:38`) and cleared each round
    /// after initiative (`ovr009.cs:44`). Bit `(team + 1)`: bit 0 = party
    /// surprised, bit 1 = monsters surprised.
    surprise_mask: u8,
    /// The tick machine's position.
    phase: Phase,
    /// How the `Turn` phase resolves — the QuickFight AI or the zero-draw stub.
    turn: TurnDriver,
    /// 0-based `FindNextCombatant` pass index within the current round.
    pass: u32,
    /// `area_ptr.can_cast_spells` — **`false` in a normal area = casting allowed**
    /// (inverted-name field; §4.1.1). `false` ⇒ the `sub_354AA` wand-scan d7 fires.
    pub area_can_cast_spells: bool,
    /// `gbl.enemyHealthPercentage` — the morale/advance input (0..100).
    pub enemy_health_pct: i32,
    /// `gbl.monster_morale` scratch (set by `FleeCheck_001`).
    pub monster_morale: i32,
    /// `area2.field_58C` — a morale threshold (default 0).
    pub area_field_58c: i32,
    /// `gbl.mapDirection` — the party's world facing, read only by the flee-move
    /// direction (`moralFailureEscape:401`).
    pub map_direction: u8,
    /// `gbl.AutoPCsCastMagic` (`byte_1D904`) — the mid-combat "Magic On" toggle
    /// ('2' key, `ovr010.cs:718-730` / `ovr009.cs:255`). `BattleSetup` resets it
    /// **false** (`ovr011.cs:1186`), so `false` is the faithful entry state; a
    /// PARTY caster's `sub_3560B` spell-selection draws are gated on it
    /// (`ovr010:068D`) — an NPC's (`control_morale >= 0x80`) are not. Input-only
    /// (the toggle key is UI, not modeled); replay harnesses set the entry value
    /// and any mid-fight presses ([`auto_cast_toggles`](Self::auto_cast_toggles))
    /// per capture.
    pub auto_pcs_cast_magic: bool,
    /// Scheduled mid-combat '2' presses (doc §38): 0-based global turn ordinals
    /// (the running count of turns started — one per `Pick`) at whose head
    /// [`auto_pcs_cast_magic`](Self::auto_pcs_cast_magic) flips. Models the
    /// buffered keypress the binary consumes at the in-turn keyboard poll
    /// (`sub_36269` @`ovr010:1269-12A9`, called from the AI turn's head
    /// `sub_3504B+D` — before `sub_3560B`'s gate read @`ovr010:068D`). The only
    /// flag readers are the per-turn spell gates, so a head-of-turn flip is
    /// gate-equivalent to any poll instant between the two gate checks that
    /// bracket the real press (doc §38). Input-only; empty = no presses (the
    /// staging hook does not yet emit toggle events — pins carry the schedule).
    pub auto_cast_toggles: Vec<u32>,
    /// The toggle schedule's clock: turns started so far (== `Pick` events;
    /// incremented at every [`take_turn`](Self::take_turn) head).
    turns_started: u32,
    /// The resident `ITEMS` data table (`gbl.ItemDataTable`, doc §34.1) — the
    /// weapon dice/range/attack-count/flags the ranged mechanics index by a
    /// readied weapon's type. `None` = no ranged loadouts in play (every
    /// combatant fights melee exactly as before); a harness with a ranged
    /// capture loads it and applies per-combatant [`Loadout`]s.
    pub item_data: Option<gbx_formats::items::ItemDataTable>,
    /// `gbl.mapToBackGroundTile.mapScreenTopLeft` — the combat camera (doc
    /// §36.3): the map cell at the top-left of the 7×7 combat window, which
    /// spans `[topLeft, topLeft + (6,6)]`. Initialized at [`combat_setup`] to
    /// `TeamList[0].pos − (3,3)` (`ovr011.cs:1209`) and moved by the census
    /// scroll sites. Read ONLY through [`on_screen`](CombatState::on_screen) /
    /// [`on_screen_pos`](CombatState::on_screen_pos); its sole draw-affecting
    /// consumer is `AttackTarget`'s on-screen facing branch (§36.1) — the
    /// camera is state, not draws.
    map_screen_top_left: GridPos,
    /// `gbl.focusCombatAreaOnPlayer` (`byte_1D910`) — the camera-follow flag
    /// (doc §36.3). Gates the focus-dependent scrolls (turn head, movement, the
    /// `draw_74B3F` recenter, `RemoveFromCombat`). Written at census sites 2/4/7.
    focus: bool,
    /// One-time `BattleSetup` guard: entry-init facing (`ovr011.cs:803`) + the
    /// setup camera (`ovr011.cs:1209`) run once, at the first [`step`], after
    /// the harness has set [`map_direction`](CombatState::map_direction).
    combat_setup_done: bool,
    /// The optional action-trace observer (D-OR3). `None` in normal play.
    sink: Option<Box<dyn ActionSink>>,
}

impl CombatState {
    /// Enters a **full fight** over a battlefield (`map`) and a caller-provided
    /// roster (`fighters`, party then monsters in `TeamList` order). The `Turn`
    /// phase drives the real `PlayerQuickFight` melee AI ([`TurnDriver::MeleeAi`]).
    /// `combat_round` starts at 0 (`BattleSetup`, `ovr011.cs:1170`); the stalemate
    /// cap defaults to [`DEFAULT_NO_ACTION_LIMIT`]. Occupancy is painted from the
    /// initial placements. This is the former `CombatWorld::new`.
    pub fn new(map: CombatMap, fighters: Vec<Combatant>) -> Self {
        let mut s = CombatState {
            fighters,
            map,
            combat_round: 0,
            no_action_limit: DEFAULT_NO_ACTION_LIMIT,
            surprise_mask: 0,
            phase: Phase::RoundStart,
            turn: TurnDriver::MeleeAi,
            pass: 0,
            area_can_cast_spells: false,
            enemy_health_pct: 100,
            monster_morale: 0,
            area_field_58c: 0,
            map_direction: 0,
            auto_pcs_cast_magic: false,
            auto_cast_toggles: Vec::new(),
            turns_started: 0,
            item_data: None,
            map_screen_top_left: GridPos::new(0, 0),
            focus: false,
            combat_setup_done: false,
            sink: None,
        };
        s.rebuild_occupancy();
        s
    }

    /// Enters the **initiative/selection harness** over a caller-provided roster —
    /// the `Turn` phase is the zero-draw stub ([`TurnDriver::Stub`]), so the draw
    /// stream is pure initiative + selection, the cleanest parity target (study
    /// §2/§14). No battlefield is needed (initiative never reads the map), so an
    /// all-void placeholder map is used. This is the former one-argument
    /// `CombatState::new(roster)`.
    pub fn initiative_only(roster: Vec<Combatant>) -> Self {
        CombatState {
            fighters: roster,
            map: CombatMap::uniform(0),
            combat_round: 0,
            no_action_limit: DEFAULT_NO_ACTION_LIMIT,
            surprise_mask: 0,
            phase: Phase::RoundStart,
            turn: TurnDriver::Stub,
            pass: 0,
            area_can_cast_spells: false,
            enemy_health_pct: 100,
            monster_morale: 0,
            area_field_58c: 0,
            map_direction: 0,
            auto_pcs_cast_magic: false,
            auto_cast_toggles: Vec::new(),
            turns_started: 0,
            item_data: None,
            map_screen_top_left: GridPos::new(0, 0),
            focus: false,
            combat_setup_done: false,
            sink: None,
        }
    }

    /// Applies a ranged [`Loadout`] to one combatant (doc §34.1) — records the
    /// primary weapon type, marks it readied (`field_151` non-null), seeds the
    /// ammo count, and saves the combatant's entry attack-1 profile as the
    /// re-ready target. Without a loadout a combatant fights melee unchanged, so
    /// this is the only entry point that arms the ranged path; the harness calls
    /// it per capture, like the guard's pins. `entry_dice` is already the
    /// record's decoded profile ([`combatant_from_record`]).
    ///
    /// **Setup-time only**: call before any combat turn runs. It snapshots the
    /// combatant's *current* attack-1 profile as the re-ready target, so calling
    /// it after a turn has unreadied the weapon would snapshot the fist profile
    /// instead.
    pub fn set_loadout(&mut self, id: usize, loadout: Loadout) {
        let f = &mut self.fighters[id];
        f.loadout = Some(loadout);
        f.weapon_readied = true;
        f.ammo = loadout.ammo_count;
        f.ammo_item_lost = false;
        // Snapshot the readied attack-1 profile as the re-ready target HERE, so
        // a hand-built combatant (whose constructors default `entry_dice` to
        // zeros) survives an unready→re-ready round trip; for the capture path
        // this equals the record profile `combatant_from_record` already set.
        f.entry_dice = (f.dice_count, f.dice_size, f.damage_bonus);
    }

    /// Sets the initial per-round surprise mask (`area2_ptr.field_596`) — a
    /// builder for setup/tests. It is read during round 1's initiative and
    /// cleared afterward (`ovr009.cs:44`), so it affects only the first round
    /// unless set again.
    pub fn with_surprise_mask(mut self, mask: u8) -> Self {
        self.surprise_mask = mask;
        self
    }

    /// Attaches an action-trace observer (D-OR3). Replaces any existing sink and
    /// returns it. Observing never changes the draw stream or the outcome.
    pub fn attach_action_sink(&mut self, sink: Box<dyn ActionSink>) -> Option<Box<dyn ActionSink>> {
        self.sink.replace(sink)
    }

    /// Detaches and returns the current observer, if any.
    pub fn take_action_sink(&mut self) -> Option<Box<dyn ActionSink>> {
        self.sink.take()
    }

    /// The current round counter (`byte_1D8B7`).
    pub fn combat_round(&self) -> u16 {
        self.combat_round
    }

    /// The roster in iteration order (read-only; draw order depends on it). An
    /// accessor alias for the public [`fighters`](Self::fighters) field.
    pub fn roster(&self) -> &[Combatant] {
        &self.fighters
    }

    /// Advances combat by one tick and returns what happened (D8: control
    /// returns each step). The `Turn` phase resolves the acting combatant's whole
    /// turn *inside* this call (via the [`TurnDriver`]), so a headless caller can
    /// drive an entire fight with `while state.step(rng) != CombatStep::Ended {}`
    /// — that is exactly what [`run_combat`](Self::run_combat) is. See
    /// [`CombatStep`].
    pub fn step(&mut self, rng: &mut EngineRng) -> CombatStep {
        if !self.combat_setup_done {
            self.combat_setup();
            self.combat_setup_done = true;
        }
        match self.phase {
            Phase::Ended => CombatStep::Ended,
            Phase::RoundStart => self.begin_round(rng),
            Phase::Selecting => self.select_or_end(rng),
        }
    }

    /// `BattleSetup`'s once-per-fight state seeding (`sub_380E0`/`ovr011.cs`),
    /// the parts the QuickFight draw stream reads — run at the first [`step`],
    /// after the harness has set [`map_direction`](CombatState::map_direction)
    /// and the loadouts. Draw-free. Rendering (`RedrawCombatScreen`) is stubbed.
    fn combat_setup(&mut self) {
        // Entry-init facing (`sub_380E0` @`ovr011:1162-118E`, coab ovr011.cs:803):
        // each combatant faces `HalfDirToIso[mapDirection / 2]` (`unk_1660C =
        // {7,2,3,6}`), and an ENEMY additionally turns to face back at the party
        // (`+4 % 8`, @`ovr011:1185-118E`). Fresh `Action`s start with
        // `AttacksReceived`/`directionChanges` 0 (already the constructor state).
        // md = 2 (every capture) ⇒ HalfDirToIso[1] = 2 → party faces 2, enemies
        // face 6. Uses the harness-set `map_direction`.
        //
        // `mapDirection` is half-encoded {0 N, 2 E, 4 S, 6 W}, so `/2` is always
        // 0..3 for a well-formed heading and the binary's `unk_1660C[md/2]` is an
        // unbounded table read. The `% 4` is a guard, not a semantic: it keeps a
        // malformed capture field or a mistyped `RESTRIKE_MAP_DIR` (the §29/§30
        // heading-sweep knob, which accepts any `u8`) from indexing out of the
        // 4-entry table and panicking. Same idiom as the other three
        // `HALF_DIR_TO_ISO` sites in this file.
        let party_dir = HALF_DIR_TO_ISO[(self.map_direction as usize / 2) % 4] as u8;
        for f in &mut self.fighters {
            f.direction = if f.team == Team::Monster {
                (party_dir + 4) % 8
            } else {
                party_dir
            };
        }
        // Site 1 — the setup camera (`ovr011.cs:1208-1209`): centre the window
        // on `TeamList[0]` (roster index 0), no clamp. An empty roster can't
        // enter combat, so index 0 is always present here.
        if let Some(first) = self.fighters.first() {
            let p = first.pos;
            self.map_screen_top_left = GridPos::new(p.x - SCREEN_HALF, p.y - SCREEN_HALF);
        }
    }

    /// `MainCombatLoop`'s per-round head (`ovr009.cs:29-44`): the emptiness guard,
    /// `calc_enemy_health_percentage` (draw-free, the morale input), initiative
    /// over the whole roster, then clear the surprise mask.
    fn begin_round(&mut self, rng: &mut EngineRng) -> CombatStep {
        // CountCombatTeamMembers + the pre-loop / round-top emptiness guard
        // (ovr009.cs:29-33). Counts LIVE (in_combat) members — with a real death
        // model this ends the fight when a side is wiped; with no deaths (the
        // stub harness) live == all, so it reduces to the whole-roster count.
        let (party, monsters) = self.live_counts();
        if party == 0 || monsters == 0 {
            self.phase = Phase::Ended;
            return CombatStep::Ended;
        }

        // calc_enemy_health_percentage (ovr014.cs:1674) — draw-free; the morale/
        // advance input read by the AI turn.
        self.recompute_enemy_health();

        // Initiative: foreach player in TeamList → CalculateInitiative (one d6 per
        // in-combat member, roster order).
        for i in 0..self.fighters.len() {
            self.calculate_initiative(rng, i, self.combat_round, self.surprise_mask);
        }

        // ovr009.cs:44 — clear the per-round surprise mask AFTER initiative read
        // it.
        self.surprise_mask = 0;
        self.pass = 0;
        self.phase = Phase::Selecting;
        CombatStep::RoundStarted {
            round: self.combat_round,
        }
    }

    /// One `FindNextCombatant` pass (`ovr009.cs:63-99`): roll one d100 per roster
    /// member, pick per the two-`if` tie-break, and either take the pick's turn or
    /// — on the terminating empty pass (`max_delay == 0`) — run `BattleRoundChecks`.
    /// The terminating pass **still draws its K d100s** (study §14 landmine 1)
    /// before ending the round.
    ///
    /// The turn itself resolves here, via the [`TurnDriver`]: `Stub` zeroes the
    /// picked combatant's `delay` with **zero draws**; `MeleeAi` runs the real
    /// `PlayerQuickFight` turn ([`melee_ai_turn`](Self::melee_ai_turn)), whose
    /// dice follow the K d100 of this pass — the exact order `MainCombatLoop`'s
    /// `while (FindNextCombatant) DoTurn` produced.
    fn select_or_end(&mut self, rng: &mut EngineRng) -> CombatStep {
        // One d100 per roster member, EVERY pass (dead/zero-delay members
        // included). Draw first, into roster order, so the seam sees exactly K
        // draws for this pass.
        let rolls: Vec<u16> = (0..self.fighters.len())
            .map(|_| roll_dice(rng, 100, 1))
            .collect();

        let delays: Vec<i8> = self.fighters.iter().map(|c| c.delay).collect();
        let picked = select_combatant(&delays, &rolls);

        let pass = self.pass;
        self.pass += 1;

        match picked {
            Some((idx, roll)) => {
                let id = self.fighters[idx].id;
                let delay = self.fighters[idx].delay;
                self.emit(ActionEvent::Pick {
                    pass,
                    combatant_id: id,
                    delay,
                    roll,
                });
                self.take_turn(rng, idx);
                CombatStep::Turn { combatant_id: id }
            }
            None => self.battle_round_checks(),
        }
    }

    /// Resolve the picked combatant's turn per the active [`TurnDriver`] — the
    /// dispatch `MainCombatLoop`'s `while (FindNextCombatant) { … }` body performs
    /// (`ovr009.cs:59-95`).
    fn take_turn(&mut self, rng: &mut EngineRng, idx: usize) {
        // The in-turn keyboard poll (`sub_36269` @`ovr010:1269`, called at the
        // AI turn's head `sub_3504B+D`): a buffered '2' press flips
        // `AutoPCsCastMagic` ("Magic On"/"Magic Off" @`ovr010:129C-12A9`). The
        // schedule entry for ordinal N takes effect at the head of turn N,
        // before the turn body's `sub_3560B` gate read (doc §38).
        if self.auto_cast_toggles.contains(&self.turns_started) {
            self.auto_pcs_cast_magic = !self.auto_pcs_cast_magic;
        }
        self.turns_started += 1;
        match self.turn {
            // Zero-draw stub: DoPlayerCombatTurn eventually sets action.delay = 0
            // (ovr010.cs:521 etc.). The harness zeroes it immediately, consuming
            // ZERO draws, so it is not re-picked and the stream stays pure
            // initiative/selection.
            TurnDriver::Stub => self.fighters[idx].delay = 0,
            // The real melee AI turn. coab's guard: only a live, un-delayed
            // combatant acts; otherwise clear_actions (draw-free) drops it so it
            // isn't re-picked (`run_combat_observed`'s old `if in_combat && delay>0`).
            TurnDriver::MeleeAi => {
                // Turn head (`sub_33281` @`ovr009:028F-02A9`, coab ovr009.cs:105):
                // the acting combatant's OWN `AttacksReceived` (@`028F`),
                // `directionChanges` (@`029C`), and `guarding` (@`02A9`) reset to 0
                // — UNCONDITIONALLY, before the `delay > 0` turn body. A parked
                // guard therefore clears at ITS OWN next turn (§32 bug #15: guards
                // survive initiative, clear here), and the swarm/facing counts
                // restart each turn.
                self.fighters[idx].attacks_received = 0;
                self.fighters[idx].direction_changes = 0;
                self.fighters[idx].guarding = false;
                // §39.5 site 1a: `CheckAffectsEffect(PlayerRestrained)` — the
                // turn-head restrained/held check, UNCONDITIONAL (before the
                // `delay > 0` gate), `mov al,7; call work_on_00` @`ovr009:02B7`
                // (coab ovr009.cs:108). The held-turn BEHAVIOUR stays unmodeled;
                // a found affect trips via the dispatch (draw-free, empty lists).
                self.check_affects_effect(idx, CheckType::PlayerRestrained);
                if self.fighters[idx].in_combat && self.fighters[idx].delay > 0 {
                    // Site 2 — the turn-head camera (`sub_33281` @`ovr009:02FA-0318`):
                    // the camera follows the acting combatant — `focus = (team ==
                    // Ours) || PlayerOnScreen(actor)` — and a focus-on turn scrolls
                    // to it (`RedrawCombatIfFocusOn(true, 2, actor)` =
                    // focus-gated `redrawCombatArea(8, 2, actor.pos)`).
                    self.focus = self.fighters[idx].team == Team::Party || self.on_screen(idx);
                    if self.focus {
                        let p = self.fighters[idx].pos;
                        self.redraw_combat_area(8, 2, p);
                    }
                    // §39.5 site 1b/1c, after the reclac/display position: `Type_15`
                    // (`mov al,0Fh; call work_on_00` @`ovr009:0352`, coab :125) then
                    // `Confusion` (`mov al,15h` @`ovr009:036E`, coab :129) which the
                    // binary gates on `spell_id == 0` — always true in the no-spell
                    // model (spell_id is the spell slice's), so run unconditionally.
                    self.check_affects_effect(idx, CheckType::Type15);
                    self.check_affects_effect(idx, CheckType::Confusion);
                    self.melee_ai_turn(rng, idx);
                } else {
                    self.clear_actions(idx);
                }
            }
        }
    }

    /// `BattleRoundChecks` (`ovr009.cs:363`, `battle01`) reduced to its
    /// non-stubbed parts: increment the round counter, run the per-member
    /// `CheckAffectsEffect(Type_19)`, run the bleed tick, and decide the loop
    /// exit. `step_game_time`, cloud damage (`in_poison_cloud`), the display-only
    /// `bandage(false)` "Your Teammate is Dying" scan, and
    /// `calc_enemy_health_percentage` (recomputed at `begin_round` instead, both
    /// draw-free) are gated on systems not in this slice.
    fn battle_round_checks(&mut self) -> CombatStep {
        // ovr009.cs:366 — the byte_1D8B7 increment.
        self.combat_round += 1;

        // §39.5 site 2: `CheckAffectsEffect(Type_19)` per TeamList member — the
        // per-round affect re-evaluation (`mov al,13h; call work_on_00`
        // @`ovr009:09EF`, coab ovr009.cs:371), run for EVERY member (before the
        // per-member dying check in the binary's one loop). Draw-free; the
        // `in_poison_cloud` companion is its own (cloud) slice.
        for ci in 0..self.fighters.len() {
            self.check_affects_effect(ci, CheckType::Type19);
        }

        // The bleed tick (§26.4, `ovr009:0A05-0A2B`, coab ovr009.cs:369-382;
        // binary-verified against coab_new.lst this session): per round end, each
        // TeamList member that is `dying` bleeds one more, and dies once
        // `bleeding > 9` (the `cmp bleeding, 9; jbe` — dead only past 9). A dead
        // (vs still-dying) ally is no longer bandageable, so this feeds §26.3.
        // Draw-free.
        for f in &mut self.fighters {
            if f.health_status == HealthStatus::Dying {
                f.bleeding += 1;
                if f.bleeding > 9 {
                    f.health_status = HealthStatus::Dead;
                }
            }
        }
        let (party, monsters) = self.live_counts();
        let battle_over = party == 0 || monsters == 0 || self.combat_round >= self.no_action_limit;
        let round = self.combat_round;
        self.phase = if battle_over {
            Phase::Ended
        } else {
            Phase::RoundStart
        };
        CombatStep::RoundEnded { round, battle_over }
    }

    /// The fight's decision from the live team counts — `PartyWins` if the
    /// monsters are gone (checked first, as `MainCombatLoop` does), `MonstersWin`
    /// if the party is gone, else `Stalemate`. Read by [`run_combat`](Self::run_combat)
    /// once the tick loop ends.
    fn outcome(&self) -> CombatOutcome {
        let (party, monsters) = self.live_counts();
        if monsters == 0 {
            CombatOutcome::PartyWins
        } else if party == 0 {
            CombatOutcome::MonstersWin
        } else {
            CombatOutcome::Stalemate
        }
    }

    fn emit(&mut self, event: ActionEvent) {
        if let Some(sink) = self.sink.as_mut() {
            sink.on_action(event);
        }
    }
}

/// The `FindNextCombatant` per-pass pick, factored pure for exact testability
/// (`ovr009.cs:74-86`, transliterated). Given each roster member's `delay` and
/// this pass's d100 `roll`, returns the yielded `(index, winning_roll)`, or
/// `None` when every remaining `delay` is 0 (the round-ending pass,
/// `ovr009.cs:89-92`).
///
/// The two `if`s are load-bearing and must not be collapsed:
/// - `if (delay > max_delay) max_roll = roll;` — a strictly-higher delay **resets**
///   `max_roll` to that member's roll, so it wins regardless of a prior high roll.
/// - `if (delay >= max_delay && roll >= max_roll) { … pick }` — among equal delays,
///   the highest roll wins (equal rolls: later member, `>=`).
pub fn select_combatant(delays: &[i8], rolls: &[u16]) -> Option<(usize, u16)> {
    debug_assert_eq!(delays.len(), rolls.len(), "one d100 per roster member");

    let mut output: Option<(usize, u16)> = None;
    let mut max_delay: i32 = 0;
    let mut max_roll: u16 = 0;

    for (i, (&delay_i8, &roll)) in delays.iter().zip(rolls).enumerate() {
        let delay = delay_i8 as i32;
        if delay > max_delay {
            max_roll = roll;
        }
        if delay >= max_delay && roll >= max_roll {
            max_roll = roll;
            max_delay = delay;
            output = Some((i, roll));
        }
    }

    // if (max_delay == 0) output_player = null;
    if max_delay == 0 {
        return None;
    }
    output
}

// ===========================================================================
// Attack resolution — to-hit + damage (M4 combat #2; study §5, D-OR5(a) Phase 1)
// ===========================================================================
//
// Draw discipline (D9/D-OR1): every roll flows through `roll_dice` (the single
// `EngineRng` seam). One `random(20)` per to-hit; `dice_count` `random(dice_size)`
// per damage roll; one `random(20)` per saving throw. `roll_dice`'s `1+random(n)`
// shape and byte truncation are already the faithful `ovr024.cs:586-598` roller.

/// The result of one to-hit roll. `d20` is the **raw** die (1..=20, *before* the
/// natural-20 promotion to 100) — the value the `attack` event records; `hit` is
/// the resolved outcome (nat-1 auto-miss, nat-20 auto-hit, else the AC compare).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToHit {
    /// The raw d20, 1..=20.
    pub d20: u8,
    /// Whether the attack connected.
    pub hit: bool,
}

/// `CanHitTarget(bonus, target)` (`ovr024.cs:487`, `sub_641DD`) — the strict-`>`
/// to-hit path.
///
/// **This is NOT the weapon-attack path.** Its only live caller is `CMD_Damage`
/// (the ECL `DAMAGE` opcode, `ovr003.cs:1673`): a scripted/area effect rolling to
/// hit a *random* party member (`rnd_player_id = roll_dice(party_size,1)`), with
/// a script-supplied `bonus`. Per-combatant weapon swings use
/// [`pc_can_hit_target`] (the `>=` path) instead. (Study §5.2 labels this
/// "monster/generic" — the caller read shows the real split is scripted-effect vs
/// weapon-attack, not monster vs PC. Flagged; the study is annotated.)
///
/// One d20; natural 1 auto-misses (the `attack_roll > 1` gate); natural 20
/// promotes to 100 (auto-hit); hit iff `(effective_roll + bonus) > target_ac`
/// (**strict `>`**). `target_ac` is the raw on-disk AC (`Player.ac@0x19a`;
/// display AC = `0x3C - ac`).
pub fn can_hit_target(rng: &mut EngineRng, bonus: i32, target_ac: u8) -> ToHit {
    let d20 = roll_dice(rng, 20, 1) as u8; // 1..=20
    let mut hit = false;
    if d20 > 1 {
        // natural 20 → 100 (beats any AC); else the raw die.
        let effective = if d20 == 20 { 100 } else { d20 as i32 };
        // The original's `attack_roll >= 0` guard is always true here
        // (effective ∈ {2..=19, 100}); the AC compare is strict `>`.
        hit = (effective + bonus) > target_ac as i32;
    }
    ToHit { d20, hit }
}

/// `PC_CanHitTarget(target_ac, target, attacker)` (`ovr024.cs:515`, `sub_64245`)
/// — the `>=` to-hit path, and **the standard weapon-attack path for ANY
/// combatant** (both PCs and monsters).
///
/// Confirmed by the caller read: its only live caller is `AttackTarget01`
/// (`ovr014.cs:821`, `sub_3F4EB`), the per-turn weapon-attack body reached from
/// the QuickFight AI / combat menu for whichever combatant is acting — so monster
/// and PC melee both resolve through this `>=` path. (`DoSpellCastingWork`,
/// `ovr023.cs:602`, also uses it for spell attacks.)
///
/// One d20; natural 1 auto-misses; natural 20 promotes to 100; hit iff
/// `(effective_roll + hit_bonus + team_bonus) >= target_ac` (**`>=`**).
///
/// - `hit_bonus` = `attacker.hitBonus@0x199` — a THAC0-derived to-hit number
///   (higher = better; `hitBonus = thac0 + DexReactionAdj + strengthHitBonus`,
///   `ovr025.cs:16-29`).
/// - `team_bonus` = the caller-selected team modifier: `area2.field_6E2` when the
///   attacker is on `Ours`, else `area2.field_6E0` (`ovr024.cs:533-540`). Passed
///   in because the combat-area team-bonus fields are not modeled this slice
///   (default 0).
///
/// `remove_invisibility` (`ovr024.cs:519`) and both `CheckAffectsEffect` calls in
/// the original are **draw-free** (verified by read, slice-1 discipline:
/// `remove_invisibility` only walks the affect list removing invisibility
/// affects — `ovr024.cs:650-658` — no `Random`), so this is exactly one d20.
pub fn pc_can_hit_target(
    rng: &mut EngineRng,
    target_ac: u8,
    hit_bonus: i32,
    team_bonus: i32,
) -> ToHit {
    let d20 = roll_dice(rng, 20, 1) as u8; // 1..=20
    let mut hit = false;
    if d20 > 1 {
        let effective = if d20 == 20 { 100 } else { d20 as i32 };
        hit = (effective + hit_bonus + team_bonus) >= target_ac as i32;
    }
    ToHit { d20, hit }
}

/// The result of one damage roll (`sub_3E192`, `ovr014.cs:84`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    /// Final damage: byte-truncated dice total + bonus, clamped `>= 0`, times the
    /// backstab multiplier when backstabbing.
    pub amount: i32,
    /// Whether the backstab multiplier was applied.
    pub backstab: bool,
}

/// The backstab damage multiplier (`sub_3E192`, `ovr014.cs:96`):
/// `((thiefSkillLevel - 1) / 4) + 2`, C-style truncating integer division. A real
/// backstab requires `SkillLevel(Thief) > 0` (`CanBackStabTarget`,
/// `ovr014.cs:1435`), so the argument is `>= 1` and the division has no
/// negative-operand ambiguity. Level 1-4 → ×2, 5-8 → ×3, 9-12 → ×4, …
pub fn backstab_multiplier(thief_level: i32) -> i32 {
    ((thief_level - 1) / 4) + 2
}

/// `sub_3E192` (`ovr014.cs:84`) reduced to its draw-bearing damage core:
/// `roll_dice_save(dice_size, dice_count)` + damage bonus, clamped `>= 0`, then
/// the backstab multiplier.
///
/// `roll_dice_save` (`ovr024.cs:601`) is just `roll_dice` after recording
/// `gbl.dice_count` (a scratch global we don't model) — so the **draw cost is
/// exactly `dice_count` `random(dice_size)` draws**, byte-truncated as a total
/// ([`roll_dice`]). The dice come from the readied attack profile
/// (`attackDiceSize/Count(idx)` = `@0x1a0/0x19e` for profile 1, `@0x1a1/0x19f`
/// for profile 2).
///
/// `damage_bonus` is `attackDamageBonus(idx)`. **Faithful quirk:** profile 1's
/// on-disk bonus is an `sbyte@0x1a2` but the accessor reinterprets it as a
/// **byte** (`(byte)attack1_DamageBonus`, `Player.cs:690`), so a *negative*
/// attack1 bonus reads as `256 + bonus` (e.g. -1 → 255); profile 2's is already a
/// byte. Callers pass the byte the accessor yields, preserving that (H4 should
/// confirm the `(byte)` cast is real 8086 behavior, not a coab artifact — the
/// `if (damage < 0)` clamp below hints the original expected it could go
/// negative, but with a byte bonus it never does).
///
/// **Backstab detection is DEFERRED** — `backstab` carries the resolved
/// multiplier or `None`. `CanBackStabTarget` (`ovr014.cs:1433`) needs facing
/// (`getTargetDirection` over map positions), `AttacksReceived`, `field_DE`, and
/// the target's `direction` — the positioning/facing system, not modeled until a
/// later slice. The multiplier math itself is faithful ([`backstab_multiplier`]).
pub fn roll_damage(
    rng: &mut EngineRng,
    dice_size: u8,
    dice_count: u8,
    damage_bonus: u8,
    backstab: Option<i32>,
) -> Damage {
    // roll_dice_save == roll_dice (byte-truncated dice total), ovr024.cs:601.
    let dice = roll_dice(rng, dice_size as u16, dice_count as u16) as i32;
    let mut amount = dice + damage_bonus as i32;
    // if (gbl.damage < 0) gbl.damage = 0;  — faithful; unreachable with a byte
    // bonus (both terms >= 0), kept for transliteration fidelity.
    if amount < 0 {
        amount = 0;
    }
    let applied = match backstab {
        Some(mult) => {
            amount *= mult;
            true
        }
        None => false,
    };
    Damage {
        amount,
        backstab: applied,
    }
}

/// The result of one saving throw (`RollSavingThrow`, `ovr024.cs:554`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavingThrow {
    /// The raw d20, 1..=20.
    pub d20: u8,
    /// Whether the save was made.
    pub made: bool,
}

/// `RollSavingThrow(saveBonus, saveType, player)` (`ovr024.cs:554`). One d20:
/// natural 1 always fails, natural 20 always succeeds; otherwise
/// `roll += save_bonus + field_186` and the save is **made iff
/// `roll >= save_target`**.
///
/// `save_target` is `player.saveVerse[saveType]@0xdf` — a per-record 5-entry table
/// read directly off the character/monster record (NOT a class/level rules-pack
/// computation), so the roll + comparison is a clean read and `save_target` /
/// `field_186` (`@0x186`, a signed per-record save bonus) are provided by the
/// caller. `CheckAffectsEffect(player, SavingThrow)` (affect-based save modifiers)
/// is draw-free and not modeled (no affects yet). The `Cheats.player_always_saves`
/// branch (`ovr024.cs:559`) is a coab dev cheat, omitted (not original behavior).
pub fn roll_saving_throw(
    rng: &mut EngineRng,
    save_bonus: i32,
    field_186: i32,
    save_target: i32,
) -> SavingThrow {
    let d20 = roll_dice(rng, 20, 1) as u8; // 1..=20
    let made = if d20 == 1 {
        false
    } else if d20 == 20 {
        true
    } else {
        (d20 as i32 + save_bonus + field_186) >= save_target
    };
    SavingThrow { d20, made }
}

/// The inputs of one weapon swing — the readied attack profile plus the target's
/// raw AC and the roster ids for the emitted events. Mirrors what `AttackTarget01`
/// (`ovr014.cs:724`) feeds `PC_CanHitTarget` + `sub_3E192` for a single attack.
#[derive(Debug, Clone, Copy)]
pub struct AttackProfile {
    /// Attacker roster id (the `attack`/`dmg` event `attacker_id`).
    pub attacker_id: usize,
    /// Target roster id.
    pub target_id: usize,
    /// The target's raw on-disk AC (`Player.ac@0x19a`; display AC = `0x3C - ac`).
    pub target_ac: u8,
    /// `attacker.hitBonus@0x199` (THAC0-derived to-hit number).
    pub hit_bonus: i32,
    /// Team to-hit modifier (`area2.field_6E2`/`field_6E0`); 0 when unmodeled.
    pub team_bonus: i32,
    /// Damage dice size (`attackDiceSize(idx)`).
    pub dice_size: u8,
    /// Damage dice count (`attackDiceCount(idx)`).
    pub dice_count: u8,
    /// Damage bonus (`attackDamageBonus(idx)`, the byte the accessor yields —
    /// see [`roll_damage`]'s quirk note).
    pub damage_bonus: u8,
    /// The backstab multiplier to apply on a hit, or `None` for no backstab
    /// (detection deferred — see [`roll_damage`]).
    pub backstab: Option<i32>,
}

/// What one [`resolve_attack`] produced: the to-hit result, and the damage on a
/// hit (`None` on a miss).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackOutcome {
    pub to_hit: ToHit,
    /// `Some` iff the attack hit.
    pub damage: Option<Damage>,
}

/// One faithful weapon attack — `AttackTarget01`'s per-swing body
/// (`ovr014.cs:811-829`): roll to hit via the `>=` path ([`pc_can_hit_target`]);
/// **on a hit only**, roll damage ([`roll_damage`]). Emits the `attack` event
/// (always) then, on a hit, the `dmg` event — in resolution order (D-OR3
/// same-tick contract).
///
/// **Draw-faithful:** exactly one d20, plus `dice_count` `random(dice_size)`
/// draws *only on a hit* (the original calls `sub_3E192` only inside the hit
/// branch, `ovr014.cs:821-828`; a miss draws nothing further).
///
/// The `|| target.IsHeld()` auto-hit (`ovr014.cs:821`) and the held-slay path
/// (`ovr014.cs:740`) are affect-gated and not modeled here (no affects yet); this
/// is the un-held single-swing core. `sink` is the optional action-trace
/// observer (D-OR3) — pass `None` in plain play; the events are draw-free
/// bookkeeping either way.
pub fn resolve_attack(
    rng: &mut EngineRng,
    p: AttackProfile,
    mut sink: Option<&mut dyn ActionSink>,
) -> AttackOutcome {
    let to_hit = pc_can_hit_target(rng, p.target_ac, p.hit_bonus, p.team_bonus);
    if let Some(s) = sink.as_mut() {
        s.on_action(ActionEvent::Attack {
            attacker_id: p.attacker_id,
            target_id: p.target_id,
            roll: to_hit.d20,
            hit: to_hit.hit,
        });
    }

    let damage = if to_hit.hit {
        let dmg = roll_damage(rng, p.dice_size, p.dice_count, p.damage_bonus, p.backstab);
        if let Some(s) = sink.as_mut() {
            s.on_action(ActionEvent::Dmg {
                attacker_id: p.attacker_id,
                target_id: p.target_id,
                amount: dmg.amount,
                backstab: dmg.backstab,
            });
        }
        Some(dmg)
    } else {
        None
    };

    AttackOutcome { to_hit, damage }
}

// ===========================================================================
// The tactical battlefield — map, placement, movement (M4 combat #3; study §11,
// D-OR5(a) Phase 1, third slice)
// ===========================================================================
//
// **This whole subsystem is draw-free.** The coab read confirms
// `SetupGroundTiles` → `PlaceCombatants` (`ovr011.cs:757-1166`) and `CalcMoves`
// / the step primitives (`ovr014.cs:58-83`, `ovr014.cs:252`) make **zero**
// `Random` calls — it is pure, deterministic geometry. So nothing here touches
// `EngineRng`/`gbx-prng`; correctness is measured against coab's layout math, not
// a draw stream (D9: no draws added). Every routine is transliterated
// read-for-behavior from coab (D11), cited by `file:line`.
//
// What the original models and this slice mirrors:
//   - a 50×25 grid of ground-tile indices (`mapToBackGroundTile`,
//     `Struct_1D1BC` — 1250 cells, `pos.y*50 + pos.x`), each tile's passability
//     read through the `BackGroundTiles` `move_cost` table (`Gbl.cs:193`);
//   - a parallel 50×25 occupancy grid (`mapToPlayerIndex`, `ovr033.cs:111`)
//     rebuilt after each placement;
//   - per-combatant `{pos, size}` cells (`CombatMap[]`, `CombatantMap.cs`);
//   - the deterministic fan-out that assigns each roster member a cell
//     (`PlaceCombatants`/`place_combatant`/`try_place_combatant`).
//
// **Deferred real-area hook (documented, not wired):** the original *derives* the
// battlefield terrain from the area the party stood in — `SetupGroundTiles`
// (`ovr011.cs:757`) calls `SetupDungeonFloor`/`SetupWildernessFloor`, which paint
// the combat diamond via `build_background_tiles_*` (`ovr011.cs:149-...`) reading
// the source area's wall topology through `get_dir_flags` (`ovr011.cs:136`). That
// wiring — like the `COMBAT`-opcode → `BattleSetup` roster assembly — belongs to
// the later encounter-trigger slice; here the map is built from a **provided
// terrain descriptor** (synthetic in tests), and the *derivation algorithm* (grid
// dimensions, tile → passability, the placement geometry) is what this slice
// implements and tests. The area→wall-flags input is surfaced as a caller
// `dir_flags` hook that defaults to "no walls" (the wilderness / open-ground
// path).

/// Combat-map width in cells (`Point.MapMaxX`, `Gbl.cs:111`). The playable
/// isometric diamond sits inside this 50×25 field.
pub const MAP_W: i32 = 50;
/// Combat-map height in cells (`Point.MapMaxY`, `Gbl.cs:112`).
pub const MAP_H: i32 = 25;
/// `Point.MapMinX`/`MapMinY` (`Gbl.cs:113-114`) — the low map bound.
pub const MAP_MIN: i32 = 0;
/// `Point.ScreenMaxX`/`ScreenMaxY` (`Gbl.cs:116-117`) — the combat window is
/// `0..=6` on both axes (a 7×7 icon grid).
pub const SCREEN_MAX: i32 = 6;
/// `Point.ScreenHalfX`/`ScreenHalfY` (`Gbl.cs:118-119`) = `ScreenMax / 2` — the
/// window's centre offset (`Point.ScreenCenter = (3, 3)`, `Gbl.cs:120`).
pub const SCREEN_HALF: i32 = SCREEN_MAX / 2;

/// A cell in the 50×25 combat map (coab's `Point`, `Gbl.cs:106`). `y` increases
/// **downward** (screen space), which the facing/octant math below depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPos {
    pub x: i32,
    pub y: i32,
}

impl GridPos {
    pub fn new(x: i32, y: i32) -> Self {
        GridPos { x, y }
    }

    /// `Point.MapInBounds()` (`Gbl.cs:170`): inside the 50×25 field.
    pub fn in_bounds(self) -> bool {
        self.x >= 0 && self.x < MAP_W && self.y >= 0 && self.y < MAP_H
    }

    /// This cell stepped one tile in iso `direction` (`+ MapDirectionDelta[dir]`).
    pub fn stepped(self, direction: u8) -> GridPos {
        let (dx, dy) = map_dir_delta(direction);
        GridPos {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

/// The 8 iso movement directions plus index 8 = "no move" (`(0,0)`), matching
/// coab's `MapDirectionDelta` (`Gbl.cs:690`). Index = iso direction: 0=N, 1=NE,
/// 2=E, 3=SE, 4=S, 5=SW, 6=W, 7=NW, 8=none. Odd indices are the diagonals.
pub const MAP_DIRECTION_DELTA: [(i32, i32); 9] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, 0),
];

/// `MapDirectionDelta[dir]` (`Gbl.cs:690`) — the (dx, dy) step for an iso
/// direction 0..=8. Panics only on an out-of-range index (a program bug).
pub fn map_dir_delta(direction: u8) -> (i32, i32) {
    MAP_DIRECTION_DELTA[direction as usize]
}

/// `Point.MapInBounds()` for a raw (x, y) — the guard `sub_3E748` applies before
/// costing a step (`ovr014.cs:260`).
pub fn map_in_bounds(p: GridPos) -> bool {
    p.in_bounds()
}

// --- ground tiles & passability -------------------------------------------

/// `BackGroundTiles[tile].move_cost` (`Struct_189B4.field_0`, the `Gbl.cs:193`
/// `unk_189B4` table, 74 entries transliterated). `0xFF` = impassable (wall);
/// `0` = a degenerate/sentinel tile; `1` = normal floor; `2`/`4` = heavier
/// terrain. This is engine-constant behavior data (like the other combat tables
/// in this module), not game *content* — D10/D11 clean.
pub const BACKGROUND_MOVE_COST: [u8; 74] = [
    1, 255, 255, 255, 255, 1, 255, 255, 255, 1, // 0..9
    255, 1, 255, 1, 255, 1, 255, 1, 255, 255, // 10..19
    255, 255, 255, 1, 1, 255, 2, 1, 1, 1, // 20..29
    1, 1, 255, 255, 255, 255, 255, 1, 1, 1, // 30..39
    1, 1, 255, 255, 1, 1, 1, 1, 2, 2, // 40..49
    2, 2, 2, 2, 1, 1, 1, 1, 2, 2, // 50..59
    4, 4, 4, 4, 1, 1, 0, 255, 0, 255, // 60..69
    0, 255, 0, 0, // 70..73
];

/// The three placement/movement-relevant states of a combat-map cell. The
/// original doesn't name an enum — it reads `move_cost` and treats groundTile 0
/// specially (`getGroundInformation`, `ovr033.cs:433`; `AtMapXY`,
/// `ovr033.cs:191`) — but this trichotomy is the faithful decode:
/// - **`Void`**: tile index 0. `AtMapXY` returns 0 for out-of-bounds, and
///   `getGroundInformation` short-circuits the whole footprint to `groundTile = 0`
///   on any 0 cell (`ovr033.cs:460`), which fails the `groundTile > 0` placement
///   gate. Unpainted map cells default to 0 (`Struct_1D1BC` `new int[1250]`).
/// - **`Wall`**: `move_cost == 0xFF`. Blocks placement (the `move_cost < 0xFF`
///   gate, `ovr011.cs:865`) and makes a step cost `0xFF·{2,3}` ≫ any budget.
/// - **`Passable`**: `move_cost` in `1..=0xFE` — walkable, at that cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilePassability {
    Passable { move_cost: u8 },
    Wall,
    Void,
}

/// Decode a ground-tile index to its passability (see [`TilePassability`]). Tile
/// index 0 is the void sentinel *regardless* of `BACKGROUND_MOVE_COST[0]`, because
/// the engine special-cases groundTile 0 upstream of the table lookup.
pub fn tile_passability(tile: u8) -> TilePassability {
    if tile == 0 {
        return TilePassability::Void;
    }
    match BACKGROUND_MOVE_COST.get(tile as usize).copied() {
        Some(0xFF) | None => TilePassability::Wall,
        Some(mc) => TilePassability::Passable { move_cost: mc },
    }
}

/// The combat battlefield: the 50×25 ground-tile grid (`mapToBackGroundTile`) plus
/// the parallel occupancy grid (`mapToPlayerIndex`, `ovr033.cs:111`). Combatant
/// `{pos, size}` cells live in [`Battlefield`] alongside this.
///
/// Built from a provided terrain descriptor (a `MAP_W*MAP_H` tile-index buffer,
/// row-major `y*50 + x`); the real "derive tiles from the area GEO block the party
/// stood on" wiring is the deferred hook documented at the top of this section.
#[derive(Debug, Clone)]
pub struct CombatMap {
    /// `mapToBackGroundTile.field_7` (`Struct_1D1BC`): ground-tile index per cell,
    /// row-major `y*MAP_W + x`, length `MAP_W*MAP_H`.
    ground: Vec<u8>,
    /// `mapToPlayerIndex` (`ovr033.cs`): 1-based combatant index occupying each
    /// cell, 0 = empty. Rebuilt by [`CombatMap::rebuild_occupancy`].
    occupancy: Vec<u16>,
}

impl CombatMap {
    /// A map with every cell set to `tile` (and empty occupancy). `tile = 0`
    /// yields an all-void map; a passable floor tile (e.g. `0x17`) yields open
    /// ground. Panics never — the buffers are always `MAP_W*MAP_H`.
    pub fn uniform(tile: u8) -> Self {
        let n = (MAP_W * MAP_H) as usize;
        CombatMap {
            ground: vec![tile; n],
            occupancy: vec![0; n],
        }
    }

    /// A map from an explicit ground-tile buffer (row-major `y*MAP_W + x`). The
    /// buffer must be exactly `MAP_W*MAP_H` long.
    pub fn from_ground(ground: Vec<u8>) -> Self {
        let n = (MAP_W * MAP_H) as usize;
        assert_eq!(ground.len(), n, "ground buffer must be {n} cells");
        CombatMap {
            ground,
            occupancy: vec![0; n],
        }
    }

    fn index(p: GridPos) -> usize {
        (p.y * MAP_W + p.x) as usize
    }

    /// Set one cell's ground tile (a terrain-descriptor builder for tests /
    /// synthetic maps). Out-of-bounds is ignored.
    pub fn set_tile(&mut self, p: GridPos, tile: u8) {
        if p.in_bounds() {
            let i = Self::index(p);
            self.ground[i] = tile;
        }
    }

    /// The ground-tile index at `p`; 0 (void) for out-of-bounds — matching
    /// `AtMapXY` returning `groundTile = 0` outside the field (`ovr033.cs:191`).
    pub fn ground_tile(&self, p: GridPos) -> u8 {
        if p.in_bounds() {
            self.ground[Self::index(p)]
        } else {
            0
        }
    }

    /// Passability of the cell at `p` ([`tile_passability`] of its ground tile;
    /// out-of-bounds → `Void`).
    pub fn passability(&self, p: GridPos) -> TilePassability {
        tile_passability(self.ground_tile(p))
    }

    /// `BackGroundTiles[mapToBackGroundTile[p]].move_cost` — the raw movement cost
    /// the step primitive multiplies (`ovr014.cs:269-273`). Out-of-bounds → `0xFF`
    /// (a step there is guarded out by `MapInBounds` first). Note the faithful
    /// quirk: an in-bounds void tile (index 0) costs `move_cost 1` here (the table
    /// value), even though placement treats it as `Void` — the engine's two paths
    /// read tile 0 differently, and both are mirrored.
    pub fn move_cost(&self, p: GridPos) -> u8 {
        if !p.in_bounds() {
            return 0xFF;
        }
        let tile = self.ground[Self::index(p)];
        BACKGROUND_MOVE_COST
            .get(tile as usize)
            .copied()
            .unwrap_or(0xFF)
    }

    /// The 1-based combatant index occupying cell `p`, or 0 (`PlayerIndexAtMapXY`,
    /// `ovr033.cs:139`; out-of-bounds → 0).
    pub fn occupant(&self, p: GridPos) -> u16 {
        if p.in_bounds() {
            self.occupancy[Self::index(p)]
        } else {
            0
        }
    }

    /// `setup_mapToPlayerIndex_and_playerScreen` (`ovr033.cs:111`): clear the
    /// occupancy grid, then paint each placed combatant's footprint
    /// (`BuildSizeMap(size, pos)`, `ovr033.cs:23`) with its 1-based index. Only
    /// `size > 0` combatants are painted (`ovr033.cs:123`). Indices are 1-based to
    /// match `player_array`/`CombatMap` (0 = empty).
    fn rebuild_occupancy(&mut self, placements: &[Placement]) {
        for c in self.occupancy.iter_mut() {
            *c = 0;
        }
        for (i, pl) in placements.iter().enumerate() {
            if !pl.placed || pl.size == 0 {
                continue;
            }
            let index = (i + 1) as u16;
            for cell in size_footprint(pl.size, pl.pos) {
                if cell.in_bounds() {
                    let idx = Self::index(cell);
                    self.occupancy[idx] = index;
                }
            }
        }
    }
}

/// `Steps[size]` (`ovr033.cs:10`) — the footprint deltas for a combatant of the
/// given size (`field_DE & 7`). Size 0 has an **empty** footprint (occupies no
/// map cell); size 1 is a single cell; 2/3 are 1×2 / 2×1; 4 is 2×2 (large
/// monsters). `BuildSizeMap(size, pos)` = these deltas offset by `pos`
/// (`ovr033.cs:23`).
pub fn size_footprint(size: u8, pos: GridPos) -> Vec<GridPos> {
    const STEPS: [&[(i32, i32)]; 5] = [
        &[],                               // 0: no footprint
        &[(0, 0)],                         // 1: single cell
        &[(0, 0), (0, 1)],                 // 2: 1×2 (tall)
        &[(0, 0), (1, 0)],                 // 3: 2×1 (wide)
        &[(0, 0), (1, 0), (0, 1), (1, 1)], // 4: 2×2
    ];
    STEPS
        .get(size as usize)
        .copied()
        .unwrap_or(&[])
        .iter()
        .map(|&(dx, dy)| GridPos::new(pos.x + dx, pos.y + dy))
        .collect()
}

// --- provisional area terrain (the deferred SetupGroundTiles hook) --------

/// The passable floor tile the provisional derivation lays down (`move_cost`
/// 1 — see `BACKGROUND_MOVE_COST`). Matches the `watch_a_real_data_fight`
/// demo's overlay so the two agree.
pub const PROVISIONAL_FLOOR: u8 = 0x17;
/// A rock/obstacle tile (`move_cost 0xFF` → [`TilePassability::Wall`]).
pub const PROVISIONAL_ROCK: u8 = 1;

/// `gbl.Tile_DownPlayer` (`Gbl.cs:680`) — the ground tile `CombatantKilled`
/// stamps at a downed party member's cell (§26.5). `BACKGROUND_MOVE_COST[0x1F]`,
/// `TILE_HEIGHT[0x1F]`, `TILE_WALL_HEIGHT[0x1F]` all equal a cost-1 floor's
/// (`1/1/0`), so the swap is movement- and reach-neutral on a cost-1 floor (the
/// bar) — fidelity, not a divergence driver.
pub const TILE_DOWN_PLAYER: u8 = 0x1F;
/// `gbl.Tile_StinkingCloud` (`Gbl.cs:679`) — a cell already carrying a stinking
/// cloud is **not** overwritten by the downed-player swap (`ovr033.cs:587`).
pub const TILE_STINKING_CLOUD: u8 = 0x1E;

/// **PROVISIONAL, draw-free combat terrain from an area's GEO wall topology**
/// (M4 combat #6, the ECL `COMBAT`-opcode wiring's map hook).
///
/// ## Why this is provisional, not the faithful `SetupGroundTiles`
///
/// The real battlefield floor is painted by `SetupGroundTiles`
/// (`ovr011.cs:757`) → `SetupDungeonFloor`/`SetupWildernessFloor`
/// (`ovr011.cs:500`/`:746`) → `build_background_tiles_1..4`
/// (`ovr011.cs:149-497`) driven by `get_dir_flags` (`ovr011.cs:136`) /
/// `sub_37306` (`ovr011.cs:90`): for each of a 13×5 band of source map cells
/// around where the party stood, it samples the four directional wall flags
/// (0=open / 1=wall / 3=door) and stamps a **rotated iso "diamond"** of
/// specific ground-tile indices via `set_background_tile`. That derivation is
/// deferred here for three compounding reasons — landing a *wrong* faithful
/// map would be worse than a flagged provisional one (this slice's stated
/// boundary):
///
/// 1. **It is a large, intricate transliteration** — four dense
///    `build_background_tiles_*` switch tables of magic tile indices plus the
///    iso `set_background_tile` transform and the `dir_*_flags` sampling.
/// 2. **There is no map oracle to verify it against.** The staging hook
///    (`docs/design/oracle-rig.md` D-OR2) dumps the PRNG *draw* stream, not
///    the `CombatMap` grid, so a transliterated diamond could only be checked
///    by re-derivation — exactly the un-cross-checkable state the boundary
///    warns against.
/// 3. **The wilderness/city floor path DRAWS from the PRNG** — a finding this
///    slice made reading the chain: `SetupWildernessFloor01/02/03` and
///    `SetGroupMapStepped` (`ovr011.cs:551-743`) call `roll_dice(100,1)`,
///    `roll_dice(2,1)`, `roll_dice(4,5)`, `roll_dice(20,1)`, `roll_dice(5,1)`
///    to scatter grass/rock decoration. Only `SetupDungeonFloor`
///    (`get_dir_flags`/`build_background_tiles_*`) is genuinely draw-free.
///    (This corrects M4 combat #3's "SetupGroundTiles is draw-free" claim,
///    which held only for the dungeon path.) So a faithful wilderness terrain
///    would have to reproduce those draws **in exact order** or desync every
///    subsequent draw in an oracle replay — another reason it belongs in its
///    own carefully-verified slice, not this wiring one.
///
/// ## What this does instead (draw-free, deterministic)
///
/// Stamps every fully-enclosed (all-four-walls-nonzero) GEO square as a rock
/// obstacle onto an otherwise-open field, then re-clears the deployment core
/// (where `place_combatants` fans the roster out, party origin `(0,0)` → iso
/// centre ≈ `(27,13)`) so everyone always finds a cell. It is *real* GEO data
/// shaping the fight — just not the faithful iso diamond. Identical to the
/// `watch_a_real_data_fight` demo's overlay (which predates this shared fn).
pub fn provisional_combat_map(geo: &GeoBlock) -> CombatMap {
    let mut ground = vec![PROVISIONAL_FLOOR; (MAP_W * MAP_H) as usize];
    for gy in 0..gbx_formats::geo::GEO_GRID_SIZE {
        for gx in 0..gbx_formats::geo::GEO_GRID_SIZE {
            let s = geo.square(gx, gy);
            let walls = [s.wall_north, s.wall_east, s.wall_south, s.wall_west]
                .iter()
                .filter(|&&w| w != 0)
                .count();
            if walls == 4 {
                let (cx, cy) = (gx as i32 + 17, gy as i32 + 3);
                if (0..MAP_W).contains(&cx) && (0..MAP_H).contains(&cy) {
                    ground[(cy * MAP_W + cx) as usize] = PROVISIONAL_ROCK;
                }
            }
        }
    }
    let mut map = CombatMap::from_ground(ground);
    // Keep the deployment diamond clear so the roster always places (the
    // faithful diamond derivation is deferred — see this fn's doc comment).
    for y in 6..=16 {
        for x in 20..=30 {
            map.set_tile(GridPos::new(x, y), PROVISIONAL_FLOOR);
        }
    }
    map
}

// --- placement (PlaceCombatants) ------------------------------------------

/// The per-combatant inputs `PlaceCombatants` reads (`ovr011.cs:1110-1118`): which
/// team, the footprint size (`field_DE & 7`), and whether it is a live combatant.
/// The full `Player`/monster record is *not* needed for placement geometry — only
/// these three fields drive the fan-out.
#[derive(Debug, Clone, Copy)]
pub struct PlacementInput {
    pub team: Team,
    /// `player.field_DE & 7` — footprint size for [`size_footprint`]. Normal
    /// single-cell combatants are size 1; large monsters 2/3/4.
    pub size: u8,
    /// `player.in_combat` — a downed member still consumes a slot but gets
    /// `size = 0` (`ovr011.cs:1122-1124`).
    pub in_combat: bool,
}

/// One placed combatant's cell (`CombatMap[i]`, `CombatantMap.cs`): its map
/// position, footprint size, and whether the fan-out found it a spot. `placed ==
/// false` means the walk exhausted the team's region (`place_combatant` returned
/// `var_4 == true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub pos: GridPos,
    pub size: u8,
    pub placed: bool,
}

/// The battlefield state: the terrain/occupancy map plus every combatant's placed
/// cell, indexed 0-based in roster order (roster index `i` ↔ coab's 1-based
/// `player_array[i+1]`). Produced by [`place_combatants`].
#[derive(Debug, Clone)]
pub struct Battlefield {
    pub map: CombatMap,
    pub placements: Vec<Placement>,
}

impl Battlefield {
    /// The placed cell of roster member `id`, or `None` if it wasn't placed.
    pub fn position(&self, id: usize) -> Option<GridPos> {
        self.placements.get(id).filter(|p| p.placed).map(|p| p.pos)
    }
}

// Placement tables (all `seg600:*` constants from `ovr011.cs`, transliterated).

/// `unk_16620[dir][row][{minCol,maxCol}]` (`ovr011.cs:885`) — per-direction,
/// per-row inclusive column range of the valid-cell mask. A row `[min,max]` with
/// `min > max` (e.g. `[1,0]`) is an empty row.
const UNK_16620: [[[u8; 2]; 6]; 5] = [
    [[1, 0], [1, 0], [1, 0], [2, 9], [3, 10], [4, 10]],
    [[0, 2], [0, 3], [1, 4], [2, 5], [3, 6], [4, 7]],
    [[0, 6], [0, 7], [1, 8], [1, 0], [1, 0], [1, 0]],
    [[3, 6], [4, 7], [5, 8], [6, 9], [7, 10], [8, 10]],
    [[0, 6], [0, 7], [1, 8], [2, 9], [3, 10], [4, 10]],
];
/// `unk_165EC[team_dir][k]` (`ovr011.cs:877`) — the direction-retry probe order.
const DIRECTION_165EC: [[i32; 4]; 4] = [[8, 4, 6, 2], [8, 6, 4, 0], [8, 0, 6, 2], [8, 2, 0, 4]];
/// `unk_165FC[team_dir][var_14]` (`ovr011.cs:878`) — the half-direction the fan-out
/// walk uses for retry index `var_14`.
const DIRECTION_165FC: [[i32; 4]; 4] = [[0, 0, 2, 6], [2, 2, 0, 4], [4, 4, 2, 6], [6, 6, 4, 0]];
/// `HalfDirToIso` / `unk_1660C` (`ovr011.cs:880`) — half-direction (0..3) → iso
/// direction.
const HALF_DIR_TO_ISO: [i32; 4] = [7, 2, 3, 6];
/// `unk_16610` (`ovr011.cs:882`) — the row-0 base column per `(var_14>0?4:0)+half_dir`.
const UNK_16610: [i32; 8] = [5, 4, 5, 6, 3, 8, 7, 2];
/// `unk_16618` (`ovr011.cs:883`) — the row-0 base row per `(var_14>0?4:0)+half_dir`.
const UNK_16618: [i32; 8] = [3, 2, 2, 3, 0, 2, 5, 3];

/// `MapDirectionXDelta` / `MapDirectionYDelta` (`Gbl.cs:691-692`) — the signed
/// per-axis deltas the placement math uses directly (kept separate from
/// [`MAP_DIRECTION_DELTA`] because coab indexes them independently).
const MAP_DIR_X_DELTA: [i32; 9] = [0, 1, 1, 1, 0, -1, -1, -1, 0];
const MAP_DIR_Y_DELTA: [i32; 9] = [-1, -1, 0, 1, 1, 1, 0, -1, 0];

/// Per-team fan-out scratch, mirroring the `gbl.team_*` / `unk_1AB1C` globals
/// `place_combatant` reads (`ovr011.cs:900-1050`). Rebuilt once per battle by
/// [`place_combatants`].
struct PlaceCtx {
    /// `team_start_x/y[team]` (`Gbl.cs:297-298`).
    team_start: [GridPos; 2],
    /// `team_direction[team]` (`Gbl.cs`), a half-direction 0..3.
    team_direction: [i32; 2],
    /// `half_team_count[team]` (`Gbl.cs:299`).
    half_team_count: [i32; 2],
    /// `unk_1AB1C[team][var_14][row][col]` — the valid-cell mask, consumed as
    /// combatants take cells.
    valid: [[[[u8; 11]; 6]; 4]; 2],
    /// `gbl.mapPosX/mapPosY` — the party's world cell, only read by the deferred
    /// `dir_flags` branches.
    map_pos: GridPos,
    /// The current team being placed (`gbl.currentTeam`).
    current_team: usize,
}

/// The deferred area-wall hook: `get_dir_flags(dir, mapX, mapY)` (`ovr011.cs:136`),
/// which reads the *source area's* wall topology. Placement calls it in two retry
/// branches; the default "open ground" impl returns `0` (no wall), which makes the
/// `get_dir_flags(...) != 1` guards behave exactly like the wilderness path
/// (`game_state == WildernessMap`). The real area-derived flags land with the
/// encounter-trigger wiring slice.
pub type DirFlags<'a> = dyn Fn(i32, i32, i32) -> i32 + 'a;

fn open_ground_dir_flags(_dir: i32, _map_x: i32, _map_y: i32) -> i32 {
    0
}

/// `PlaceCombatants` (`ovr011.cs:1053`): assign each roster member a battlefield
/// cell, deterministically, in `TeamList` order. Draw-free.
///
/// The geometry, step by step:
/// - **Team origins** (`ovr011.cs:1063-1069`): the party (`Team::Party`, coab team
///   0) starts at `(0,0)`; the monsters (team 1) start `encounter_distance` tiles
///   ahead along the party's facing —
///   `encounter_distance · MapDirectionDelta[map_direction]`. Each team's
///   half-direction is `map_direction/2` (party) / `((map_direction+4)%8)/2`
///   (facing back at the party).
/// - **Half-team counts** (`ovr011.cs:1071-1072`): `(count+1)/2`, the row the
///   fan-out fills before spilling to the next.
/// - **Valid-cell mask** (`ovr011.cs:1074-1104`): built per team from
///   [`UNK_16620`]'s per-row column ranges.
/// - **Per member** (`ovr011.cs:1110-1160`): [`place_combatant`] walks a
///   left/right tri-state fan-out from the team origin to the first cell that is
///   (a) mask-valid, (b) on passable ground (`move_cost < 0xFF`, non-void), and
///   (c) unoccupied. On success the cell's map position is the iso transform
///   `pos.x = cur_x + team_x·6 + team_y·5 + 22`, `pos.y = cur_y + team_y·5 + 10`
///   (`ovr011.cs:856-857`). Occupancy is rebuilt after each placement.
///
/// `map_direction` is the party's iso facing (0..7); `encounter_distance` is the
/// approach range (`area2.encounter_distance`); `map_pos` is the party's world
/// cell (only the deferred `dir_flags` branch reads it). `dir_flags` defaults to
/// open ground when `None`.
pub fn place_combatants(
    map: &mut CombatMap,
    roster: &[PlacementInput],
    map_direction: u8,
    encounter_distance: i32,
    map_pos: GridPos,
    dir_flags: Option<&DirFlags<'_>>,
) -> Vec<Placement> {
    let default_flags = open_ground_dir_flags;
    let dir_flags: &DirFlags<'_> = dir_flags.unwrap_or(&default_flags);

    let friends = roster.iter().filter(|r| r.team == Team::Party).count() as i32;
    let foes = roster.iter().filter(|r| r.team == Team::Monster).count() as i32;

    // Team origins + directions (ovr011.cs:1063-1069).
    let (edx, edy) = map_dir_delta(map_direction);
    let mut ctx = PlaceCtx {
        team_start: [
            GridPos::new(0, 0),
            GridPos::new(encounter_distance * edx, encounter_distance * edy),
        ],
        team_direction: [
            (map_direction as i32) / 2,
            (((map_direction as i32) + 4) % 8) / 2,
        ],
        half_team_count: [(friends + 1) / 2, (foes + 1) / 2],
        valid: [[[[0u8; 11]; 6]; 4]; 2],
        map_pos,
        current_team: 0,
    };

    // Build the valid-cell mask per team (ovr011.cs:1074-1104). Indexed loops
    // mirror coab's nested `for (var_C; var_2; var_1)` exactly.
    #[allow(clippy::needless_range_loop)]
    for team in 0..2usize {
        for var_c in 0..4usize {
            let direction = if var_c == 1 {
                4usize
            } else {
                ctx.team_direction[team] as usize
            };
            for row in 0..6usize {
                for col in 0..11usize {
                    let lo = UNK_16620[direction][row][0] as usize;
                    let hi = UNK_16620[direction][row][1] as usize;
                    ctx.valid[team][var_c][row][col] = if lo > col || hi < col { 0 } else { 1 };
                }
            }
        }
    }

    // Per-member fan-out, in roster (TeamList) order. `placements[i]` ↔ coab's
    // 1-based `player_array[i+1]`.
    let mut placements: Vec<Placement> = roster
        .iter()
        .map(|r| Placement {
            pos: GridPos::new(0, 0),
            size: r.size & 7,
            placed: false,
        })
        .collect();

    for i in 0..roster.len() {
        ctx.current_team = match roster[i].team {
            Team::Party => 0,
            Team::Monster => 1,
        };
        // CombatMap[i].size = field_DE & 7 (ovr011.cs:1118).
        placements[i].size = roster[i].size & 7;

        let ok = place_combatant(&mut ctx, map, &mut placements, i, dir_flags);
        placements[i].placed = ok;

        if ok && !roster[i].in_combat {
            // A downed member keeps its cell but drops to size 0 (ovr011.cs:1122).
            placements[i].size = 0;
        }
        // setup_mapToPlayerIndex_and_playerScreen after each placement
        // (ovr011.cs:1143) so later members see this one's footprint.
        map.rebuild_occupancy(&placements);
    }

    placements
}

/// `row_column_both_out_of_range` (`ovr011.cs:832`): true only when the cell is
/// outside **both** the column band `[0,10]` and the row band `[0,5]`.
fn row_column_both_out_of_range(row: i32, column: i32) -> bool {
    !((0..=10).contains(&column) || (0..=5).contains(&row))
}

/// `try_place_combatant` (`ovr011.cs:846`): if cell `(cur_x, cur_y)` is mask-valid
/// for `(team, var_14)`, tentatively write its iso map position, then accept iff
/// the footprint is on passable, unoccupied ground. On accept the mask cell is
/// consumed. Returns whether the cell was taken.
#[allow(clippy::too_many_arguments)]
fn try_place_combatant(
    ctx: &mut PlaceCtx,
    map: &CombatMap,
    placements: &mut [Placement],
    var_14: usize,
    team_y: i32,
    team_x: i32,
    cur_y: i32,
    cur_x: i32,
    player_index: usize,
) -> bool {
    if !(0..=10).contains(&cur_x)
        || !(0..=5).contains(&cur_y)
        || ctx.valid[ctx.current_team][var_14][cur_y as usize][cur_x as usize] == 0
    {
        return false;
    }

    // The iso transform (ovr011.cs:856-857).
    let pos = GridPos::new(
        cur_x + (team_x * 6) + (team_y * 5) + 22,
        cur_y + (team_y * 5) + 10,
    );
    placements[player_index].pos = pos;

    // getGroundInformation(...,8,player): scan the footprint at the just-written
    // position for the "worst" ground tile and any occupant (ovr033.cs:433).
    let (ground_tile, occupant) = ground_information(map, placements, player_index);

    if occupant == 0 && ground_tile > 0 && ground_tile_move_cost(ground_tile) < 0xFF {
        ctx.valid[ctx.current_team][var_14][cur_y as usize][cur_x as usize] = 0;
        true
    } else {
        false
    }
}

/// `BackGroundTiles[tile].move_cost` for a ground-tile index already known to be
/// in-range (the placement gate reads it straight, `ovr011.cs:865`).
fn ground_tile_move_cost(tile: i32) -> u8 {
    BACKGROUND_MOVE_COST
        .get(tile as usize)
        .copied()
        .unwrap_or(0xFF)
}

/// `getGroundInformation(out groundTile, out playerIndex, 8, player)`
/// (`ovr033.cs:433`) reduced to what placement needs: over the combatant's
/// footprint (`BuildSizeMap(size, pos)`), return the highest-move_cost ground tile
/// (or 0 if any cell is void) and the index of any *other* occupant. Direction 8's
/// delta is `(0,0)`, so it scans the footprint in place.
fn ground_information(
    map: &CombatMap,
    placements: &[Placement],
    player_index: usize,
) -> (i32, u16) {
    let current = (player_index + 1) as u16;
    let pl = &placements[player_index];

    let mut ground_tile: i32 = 0x17; // default (ovr033.cs:436)
    let mut player_out: u16 = 0;
    let mut max_move_cost: u8 = 1;

    for cell in size_footprint(pl.size, pl.pos) {
        // AtMapXY: out-of-bounds → (0, 0) (ovr033.cs:191).
        let (at_ground, at_player) = if cell.in_bounds() {
            (map.ground_tile(cell) as i32, map.occupant(cell))
        } else {
            (0, 0)
        };
        let at_player = if at_player == current { 0 } else { at_player };
        if at_player > 0 {
            player_out = at_player;
        }
        if at_ground == 0 {
            ground_tile = 0;
        } else if ground_tile != 0 {
            let mc = ground_tile_move_cost(at_ground);
            if mc >= max_move_cost {
                max_move_cost = mc;
                ground_tile = at_ground;
            }
        }
    }
    (ground_tile, player_out)
}

/// `place_combatant` (`ovr011.cs:900`): the left/right tri-state fan-out that walks
/// outward from the team origin, one candidate cell per iteration, until
/// [`try_place_combatant`] takes one or the team's region is exhausted. Returns
/// `true` on placement (coab's `var_4 == false`).
///
/// Transliterated literally — the two direction tables ([`DIRECTION_165FC`] /
/// [`DIRECTION_165EC`]), the `row_scale`/`col_scale` outward growth, the
/// `var_13`/`half_team_count` row-fill limits, and the `var_14` direction-retry
/// that shifts the team origin when a whole direction is blocked. The two
/// `dir_flags`-gated branches are the deferred area-wall hook (open-ground default
/// makes them behave as the wilderness path).
fn place_combatant(
    ctx: &mut PlaceCtx,
    map: &CombatMap,
    placements: &mut [Placement],
    player_index: usize,
    dir_flags: &DirFlags<'_>,
) -> bool {
    let team = ctx.current_team;

    let mut cur_x: i32;
    let mut cur_y: i32;
    let mut base_x: i32 = 0;
    let mut base_y: i32 = 0;
    let mut var_13: i32 = 0;

    let mut placed = false;
    let mut first_row = true;
    let mut var_4 = false;

    // tri_state: 1 = start, 2 = right, 3 = left (ovr011.cs:893).
    let mut state: i32 = 1;
    let mut row_scale: i32 = 0;
    let mut col_scale: i32 = 0;
    let mut var_14: usize = 0;

    let mut team_x = ctx.team_start[team].x;
    let mut team_y = ctx.team_start[team].y;

    loop {
        let half_dir = (DIRECTION_165FC[ctx.team_direction[team] as usize][var_14] / 2) as usize;

        match state {
            1 => {
                // start
                let iso_dir = HALF_DIR_TO_ISO[(half_dir + 2) % 4] as usize;
                let delta_x = MAP_DIR_X_DELTA[iso_dir];
                let delta_y = MAP_DIR_Y_DELTA[iso_dir];
                let base_idx = (if var_14 > 0 { 4 } else { 0 }) + half_dir;
                base_x = UNK_16610[base_idx] + (row_scale * delta_x);
                base_y = UNK_16618[base_idx] + (row_scale * delta_y);
                cur_x = base_x;
                cur_y = base_y;
                col_scale = 1;
                state = 2; // right
                var_13 = 1;
            }
            2 => {
                // right
                let iso = HALF_DIR_TO_ISO[(half_dir + 1) % 4] as usize;
                let delta_x = MAP_DIR_X_DELTA[iso];
                let delta_y = MAP_DIR_Y_DELTA[iso];
                cur_x = base_x + (delta_x * col_scale);
                cur_y = base_y + (delta_y * col_scale);
                state = 3; // left
                var_13 += 1;
            }
            _ => {
                // left (3)
                let iso = HALF_DIR_TO_ISO[(half_dir + 3) % 4] as usize;
                let delta_x = MAP_DIR_X_DELTA[iso];
                let delta_y = MAP_DIR_Y_DELTA[iso];
                cur_x = base_x + (delta_x * col_scale);
                cur_y = base_y + (delta_y * col_scale);
                state = 2; // right
                col_scale += 1;
                var_13 += 1;
            }
        }

        let any_cur_invalid = cur_x < 0 || cur_y < 0 || cur_x > 10 || cur_y > 5;

        // coab nests `if (state > start) { if (row-full) {…} }`; kept nested to
        // mirror the transliteration source.
        #[allow(clippy::collapsible_if)]
        if state > 1 {
            if (any_cur_invalid && !row_column_both_out_of_range(cur_y, cur_x))
                || (first_row && var_13 >= ctx.half_team_count[team])
                || (!first_row && var_13 > 11)
            {
                row_scale += 1;

                // Deferred dir_flags branch (ovr011.cs:979-1003): party team, odd
                // half-direction, first retry — peek 3 probe directions and bump
                // row_scale again if the source area is open there.
                if team == 0 && (ctx.team_direction[0] & 1) == 1 && var_14 == 0 && row_scale == 1 {
                    let tmp_x = ctx.team_start[team].x + ctx.map_pos.x;
                    let tmp_y = ctx.team_start[team].y + ctx.map_pos.y;
                    let mut found = false;
                    #[allow(clippy::needless_range_loop)] // faithful `for (var_A=1; var_A<=3)`
                    for var_a in 1..=3usize {
                        let tmp_dir = DIRECTION_165EC[ctx.team_direction[team] as usize][var_a];
                        // game_state == WildernessMap || get_dir_flags(...) != 1.
                        // Open-ground default returns 0 → != 1 → found.
                        if dir_flags(tmp_dir, tmp_y, tmp_x) != 1 {
                            found = true;
                        }
                    }
                    if found {
                        row_scale += 1;
                    }
                }
                state = 1; // start
                first_row = false;
            }
        }

        if any_cur_invalid && row_column_both_out_of_range(cur_y, cur_x) {
            placed = false;
            state = 0;

            // var_14 direction-retry (ovr011.cs:1016-1034): advance to the next
            // probe direction that the source area leaves open, shifting the team
            // origin one tile that way and resetting the walk.
            while var_14 < 3 && state != 1 {
                var_14 += 1;
                let tmp_x = ctx.team_start[team].x + ctx.map_pos.x;
                let tmp_y = ctx.team_start[team].y + ctx.map_pos.y;
                let tmp_dir = DIRECTION_165EC[ctx.team_direction[team] as usize][var_14];
                if dir_flags(tmp_dir, tmp_y, tmp_x) != 1 {
                    team_x = ctx.team_start[team].x + MAP_DIR_X_DELTA[tmp_dir as usize];
                    team_y = ctx.team_start[team].y + MAP_DIR_Y_DELTA[tmp_dir as usize];
                    row_scale = 0;
                    state = 1; // start
                }
            }

            if state != 1 {
                var_4 = true;
            }
        }

        if !any_cur_invalid {
            placed = try_place_combatant(
                ctx,
                map,
                placements,
                var_14,
                team_y,
                team_x,
                cur_y,
                cur_x,
                player_index,
            );
        }

        if placed || var_4 {
            break;
        }
    }

    !var_4
}

// --- movement, facing, adjacency, distance --------------------------------

/// `CalcMoves(player)` (`ovr014.cs:58`), in-combat core: clamp the base movement
/// to `[1, 96]` — note the faithful quirk that a value **> 96 also collapses to 1**
/// (the `moves < 1 || moves > 96` test, `ovr014.cs:67`), not to 96 — then double
/// into half-move granularity (`halfActionsLeft = moves * 2`, `:72`). The returned
/// value is the round's half-move budget (`action.move`, `Action@0x06`).
///
/// The out-of-combat wilderness bonus (`+ area2.field_6E4`, `:64`) and the
/// `CheckAffectsEffect(Movement)` pass (`:76`, draw-free, no affects modeled) are
/// omitted — this is the in-combat, no-affects budget.
pub fn calc_moves(movement: i32) -> i32 {
    let moves = if !(1..=96).contains(&movement) {
        1
    } else {
        movement
    };
    moves * 2
}

/// The cost of stepping one tile in iso `direction` from `pos`, per `sub_3E748`
/// (`ovr014.cs:252`): `None` if the destination is off the map (the `MapInBounds`
/// guard, `:260`); otherwise `(destination, cost)` where cost is the destination
/// tile's `move_cost` times **3 for a diagonal** (odd direction) or **2 for an
/// orthogonal** step (`:266-273`). Draw-free.
///
/// The move accounting `sub_3E748` then does — `if cost > move { move = 0 } else {
/// move -= cost }` (`:276-283`) — is [`deduct_move`]; the rest of `sub_3E748`
/// (redraw, sound, `move_step_into_attack`) is UI / a later slice.
pub fn step_cost(map: &CombatMap, pos: GridPos, direction: u8) -> Option<(GridPos, i32)> {
    let dest = pos.stepped(direction);
    if !dest.in_bounds() {
        return None;
    }
    let base = map.move_cost(dest) as i32;
    let cost = if direction & 1 != 0 {
        base * 3
    } else {
        base * 2
    };
    Some((dest, cost))
}

/// The move-point deduction of `sub_3E748` (`ovr014.cs:276-283`): spending more
/// than is left zeroes the budget (you can't half-finish a step), otherwise it
/// subtracts. Returns the remaining half-moves.
pub fn deduct_move(remaining: i32, cost: i32) -> i32 {
    if cost > remaining {
        0
    } else {
        remaining - cost
    }
}

/// The `SteppingPath` iteration count (`var_AF`) for a missile from `attacker`
/// to `target` over the **×3 pixel grid** (`ovr025.cs:896-908`): `Step()`
/// (`sub_7324C`) is called until it takes no step (the current cell reaches the
/// target on both axes), counting each call — including the terminal no-step
/// one, since `var_AF` post-increments past it. Draw-free; used only by the
/// missile camera (site 5, [`CombatState::draw_missile_camera`]).
fn missile_path_pixel_steps(attacker: GridPos, target: GridPos) -> usize {
    let (ax, ay) = (attacker.x * 3, attacker.y * 3);
    let (tx, ty) = (target.x * 3, target.y * 3);
    let diff_x = (tx - ax).abs();
    let diff_y = (ty - ay).abs();
    let sign_x = (tx - ax).signum();
    let sign_y = (ty - ay).signum();
    let (mut cx, mut cy) = (ax, ay);
    let mut delta_count = 0i32;
    let mut count = 0usize;
    loop {
        // one Step() (SteppingPath.cs:38-88).
        let mut step_made = false;
        if diff_x >= diff_y {
            if cx != tx {
                cx += sign_x;
                delta_count += diff_y * 2;
                if delta_count >= diff_x {
                    cy += sign_y;
                    delta_count -= diff_x * 2;
                }
                step_made = true;
            }
        } else if cy != ty {
            cy += sign_y;
            delta_count += diff_x * 2;
            if delta_count >= diff_y {
                cx += sign_x;
                delta_count -= diff_y * 2;
            }
            step_made = true;
        }
        count += 1; // var_AF++ (ovr025.cs:907)
        if !step_made {
            break;
        }
    }
    count
}

/// `getTargetDirection(playerB, playerA)` (`ovr014.cs:1460`, `sub_409BC`): the iso
/// heading (0..7) **from `from` toward `to`**, an octant classifier over the cell
/// vector. Pure geometry, draw-free.
///
/// The original scans directions 0,1,2,… returning the first whose octant test
/// passes. Even directions (N/E/S/W) test one axis dominance; odd (diagonals) test
/// both. The slope thresholds are fixed-point tangents: `0x26A/256 ≈ 2.414`
/// (tan 67.5°) and `0x6A/256 ≈ 0.414` (tan 22.5°) — the 22.5°/67.5° octant
/// boundaries. `diff_x`/`diff_y` are absolute; the sign tests disambiguate
/// quadrant. Recall `y` grows downward, so "north" is `to.y < from.y`.
pub fn target_direction(from: GridPos, to: GridPos) -> u8 {
    // plyr_a = from, plyr_b = to.
    let diff_x = (to.x - from.x).abs();
    let diff_y = (to.y - from.y).abs();
    let hi = |d: i32| (0x26A * d) / 0x100; // tan 67.5°
    let lo = |d: i32| (0x6A * d) / 0x100; // tan 22.5°

    let mut direction: u8 = 0;
    loop {
        let solved = match direction {
            0 => !(to.y > from.y || hi(diff_x) > diff_y),
            2 => !(to.x < from.x || lo(diff_x) < diff_y),
            4 => !(to.y < from.y || hi(diff_x) > diff_y),
            6 => !(to.x > from.x || lo(diff_x) < diff_y),
            1 => !(to.y > from.y || to.x < from.x || hi(diff_x) < diff_y || lo(diff_x) > diff_y),
            3 => !(to.y < from.y || to.x < from.x || hi(diff_x) < diff_y || lo(diff_x) > diff_y),
            5 => !(to.y < from.y || to.x > from.x || hi(diff_x) < diff_y || lo(diff_x) > diff_y),
            _ => !(to.y > from.y || to.x > from.x || hi(diff_x) < diff_y || lo(diff_x) > diff_y), // 7
        };
        if solved {
            return direction;
        }
        direction += 1;
        // One octant always solves; the guard keeps a pathological input bounded.
        if direction > 7 {
            return 0;
        }
    }
}

/// The open-ground king-move distance between two single-cell combatants:
/// `max(|dx|, |dy|)` (Chebyshev), i.e. the number of iso steps with diagonals
/// allowed. This is the **geometric** distance, exact on open ground.
///
/// **Not the engine's authoritative combat range.** The original measures range as
/// a *wall-respecting* BFS step count — `Rebuild_SortedCombatantList`
/// (`ovr032.cs:228`) fills a flood from the attacker and `getTargetRange`
/// (`ovr025.cs:1309`) returns `steps / 2`. That flood is the core of target
/// *selection* (the AI's `BuildNearTargets`, `ovr025.cs:1290`), which consumes the
/// next slice; it is draw-free but out of this geometry slice's scope. Around
/// walls this Chebyshev underestimates the real path length — callers needing the
/// authoritative range must use the pathfinder the AI slice will add.
pub fn grid_distance(a: GridPos, b: GridPos) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

/// Melee reach for single-cell combatants: the two cells are king-adjacent
/// (`grid_distance == 1`) — the geometric form of `BuildNearTargets(1, …)`
/// (`ovr025.cs:1290`) on open ground. Same wall/size caveat as [`grid_distance`]:
/// the engine's near-target list is the wall-respecting flood, and multi-cell
/// (size > 0) footprints widen reach; those land with the AI slice. `false` for a
/// cell against itself.
pub fn is_adjacent(a: GridPos, b: GridPos) -> bool {
    a != b && grid_distance(a, b) == 1
}

/// `CanSeeTargetA` is **not** geometric line-of-sight — it is an *invisibility*
/// check. Documented here to prevent a future slice from wiring it as LoS.
///
/// The caller read (`ovr014.cs:571`, `sub_3F143`) shows it returns
/// `!gbl.targetInvisible` after running `CheckAffectsEffect(Visibility)` on the
/// target and `CheckType.None` on the seer — purely the affect system's
/// invisible/see-invisible resolution, no cell geometry at all (it never reads a
/// position). Geometric visibility in combat is instead handled by the
/// wall-respecting flood's wall checks (`mapToBackGroundTile.ignoreWalls`,
/// `ovr025.cs:1311`). Since affects aren't modeled yet, `CanSeeTargetA` has no
/// analog this slice; when affects land it belongs with them, not with the map.
/// (This mirrors the slice-2 `PC_CanHitTarget` mislabel correction — verify by
/// caller, not by name.)
pub const CAN_SEE_TARGET_A_IS_INVISIBILITY_NOT_LOS: () = ();

// ===========================================================================
// The wall-respecting range — the Bresenham reach ray (M4 combat #4; study
// §4.1.3; deliverable 2, deferred from slice 3)
// ===========================================================================
//
// **This is a straight-line reach RAY, not a BFS flood.** Both the slice-3 study
// and the AI-slice brief describe the engine's combat range as a "wall-respecting
// flood-fill"; the coab read (`ovr032.cs` `canReachTargetCalc:92`,
// `Classes/SteppingPath.cs`) shows it is a **Bresenham line march** from attacker
// to target. It accumulates a step cost of **2 per orthogonal step, +1 more for a
// diagonal** (`SteppingPath.Step:38-89`) and — unless walls are ignored — blocks
// if any tile on the line presents a wall taller than the *attacker's* tile
// height (`BackGroundTiles[tile].field_2 > attackerTile.field_1`,
// `canReachTargetCalc:124`). `getTargetRange` = `steps / 2` (`ovr025.cs:1305-1316`,
// with `ignoreWalls=true` so it is pure geometry); `BuildNearTargets` = the
// opposite-team members reachable within `max_range`, sorted nearest-first
// (`ovr025.cs:1290`, `ovr032.cs` `Rebuild_SortedCombatantList:221`). **Draw-free**
// (both `ovr025` and `ovr032` contain zero `Random` calls — verified by read).
//
// This corrects the slice-3 `grid_distance` note: the authoritative combat range
// is this ray's `steps/2`, which on open ground is the move-cost of the straight
// path (diagonals discounted), *not* the Chebyshev king-move `grid_distance`.
//
// **Faithful-but-degenerate quirk (transliterated as coab wrote it):** the height
// "budget" path (`var_31`, `canReachTargetCalc:103-116`) is built flat — both its
// endpoints take the *attacker* tile's `field_1` — so the wall test reduces to the
// constant `tile.field_2 > attackerTile.field_1`. Whether coab's flat `var_31` is
// the real binary behavior or a decompiler artifact is unverifiable statically; on
// the uniform-height terrain this slice's fights use (`field_1` is 1 for every
// floor tile) the test never fires anyway, so it is inert for the parity artifact.

/// `BackGroundTiles[tile].field_1` (`Struct_189B4.field_1`, `Gbl.cs:193-268`) —
/// the tile's "floor height", the attacker-tile value the reach ray uses as its
/// wall-clearance budget. 74 entries, parallel to [`BACKGROUND_MOVE_COST`].
pub const TILE_HEIGHT: [u8; 74] = [
    0, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0..9
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 10..19
    1, 1, 1, 1, 1, 1, 2, 1, 1, 1, // 20..29
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 30..39
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 40..49
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 50..59
    0, 0, 0, 0, 1, 1, 0, 0xFF, 0, 0xFF, // 60..69
    0, 0xFF, 0, 1, // 70..73
];

/// `BackGroundTiles[tile].field_2` (`Struct_189B4.field_2`, `Gbl.cs:193-268`) —
/// the tile's "wall height". The reach ray blocks on a tile whose `field_2`
/// exceeds the attacker tile's [`TILE_HEIGHT`] (`canReachTargetCalc:124`). Walls
/// (`move_cost 0xFF`) carry `field_2 = 2` (> the floor height 1); the void
/// sentinels carry `0xFF`. 74 entries.
pub const TILE_WALL_HEIGHT: [u8; 74] = [
    0xFF, 2, 2, 2, 2, 0, 2, 2, 2, 0, // 0..9
    2, 0, 2, 0, 2, 0, 2, 0, 2, 2, // 10..19
    2, 2, 2, 0, 0, 2, 0, 0, 0, 0, // 20..29
    0, 0, 2, 2, 2, 2, 2, 0, 0, 0, // 30..39
    0, 0, 2, 2, 0, 0, 0, 0, 0, 0, // 40..49
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 50..59
    0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0xFF, // 60..69
    1, 0xFF, 1, 1, // 70..73
];

/// The result of one reach ray (`MapReach`, `ovr032.cs:9`): whether the line was
/// unobstructed, and the accumulated `steps` (2·orthogonal + 3·diagonal, i.e.
/// `2·max(|dx|,|dy|) + min(|dx|,|dy|)`). Range in half-steps; `steps / 2` is the
/// tile range (`getTargetRange`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReachRay {
    pub reach: bool,
    pub steps: u16,
}

/// `canReachTargetCalc` (`ovr032.cs:92`, `sub_733F1`): march the Bresenham line
/// from `attacker` to `target`, accumulating [`ReachRay::steps`], returning early
/// (`reach = false`) at the first tile whose wall height exceeds the attacker
/// tile's height (skipped when `ignore_walls`). The `steps > 511` guard is dead on
/// a 50×25 map (coab's own comment, `:129`) and omitted. `steps` never wraps here
/// (max ≈ 147 < 256) though coab stores it in a byte.
///
/// The line stepping is `SteppingPath.Step` (`SteppingPath.cs:38`) verbatim: along
/// the dominant axis, each major step adds 2, and a Bresenham minor step adds a
/// further 1 (the diagonal). Draw-free.
pub fn reach_ray(
    map: &CombatMap,
    attacker: GridPos,
    target: GridPos,
    ignore_walls: bool,
) -> ReachRay {
    let attacker_height = TILE_HEIGHT
        .get(map.ground_tile(attacker) as usize)
        .copied()
        .unwrap_or(0);

    let diff_x = (target.x - attacker.x).abs();
    let diff_y = (target.y - attacker.y).abs();
    let sign_x = (target.x - attacker.x).signum();
    let sign_y = (target.y - attacker.y).signum();

    let mut cur = attacker;
    let mut steps: u16 = 0;
    let mut delta_count: i32 = 0;

    loop {
        // Wall test on the current cell (attacker cell first, target cell last).
        if !ignore_walls {
            let gt = map.ground_tile(cur);
            let wall = TILE_WALL_HEIGHT.get(gt as usize).copied().unwrap_or(0xFF);
            if wall > attacker_height {
                return ReachRay {
                    reach: false,
                    steps,
                };
            }
        }

        // SteppingPath.Step (ovr032/SteppingPath.cs:38-89).
        let made = if diff_x >= diff_y {
            if cur.x != target.x {
                cur.x += sign_x;
                delta_count += diff_y * 2;
                steps += 2;
                if delta_count >= diff_x {
                    cur.y += sign_y;
                    delta_count -= diff_x * 2;
                    steps += 1;
                }
                true
            } else {
                false
            }
        } else if cur.y != target.y {
            cur.y += sign_y;
            delta_count += diff_x * 2;
            steps += 2;
            if delta_count >= diff_y {
                cur.x += sign_x;
                delta_count -= diff_y * 2;
                steps += 1;
            }
            true
        } else {
            false
        };

        if !made {
            return ReachRay { reach: true, steps };
        }
    }
}

/// `canReachTarget(ref range, target, attacker)` (`ovr032.cs:77`): the reach test
/// with a `range_budget` (in tiles). Returns `Some(steps)` iff the line is
/// unobstructed **and** `steps <= range_budget·2 + 1`; `None` otherwise. Mirrors
/// coab's `if (mr.range > range*2+1) return false; else return mr.reach;` — note
/// the `+1` slack lets a diagonal-adjacent (steps 3) satisfy a `range_budget` of 1.
pub fn can_reach(
    map: &CombatMap,
    attacker: GridPos,
    target: GridPos,
    range_budget: i32,
    ignore_walls: bool,
) -> Option<u16> {
    let ray = reach_ray(map, attacker, target, ignore_walls);
    if ray.steps as i32 > range_budget * 2 + 1 {
        return None;
    }
    if ray.reach {
        Some(ray.steps)
    } else {
        None
    }
}

/// `getTargetRange(target, attacker)` (`ovr025.cs:1305`): the tile range from
/// `attacker` to `target` — `steps / 2` of the wall-**ignoring** ray (coab sets
/// `ignoreWalls = true`, `:1307`, so this is pure geometry). Adjacent = 1 (an
/// orthogonal neighbour is steps 2, a diagonal 3, both `/2 = 1`). coab returns
/// `0xFF` when the target isn't in the combatant list; that case doesn't arise for
/// a real live target, so the geometric value is returned directly.
pub fn get_target_range(map: &CombatMap, target: GridPos, attacker: GridPos) -> u16 {
    reach_ray(map, attacker, target, true).steps / 2
}

/// `CanSeeCombatant(direction, playerA, playerB)` (`ovr032.cs:145`, `sub_7354A`):
/// whether `playerB`, facing iso `direction`, can see `playerA` — an octant
/// half-plane test (NOT the same as `CanSeeTargetA`, which is the invisibility
/// affect). Pure geometry, draw-free. Used only as the [`build_near_targets`]
/// sort tiebreak via [`find_combatant_direction`]; transliterated for fidelity.
pub fn can_see_combatant(direction: u8, player_a: GridPos, player_b: GridPos) -> bool {
    if !player_a.in_bounds() || !player_b.in_bounds() {
        return false;
    }
    if direction == 0xFF || direction == 8 {
        return true;
    }
    let facing_x = player_b.x + MAP_DIR_X_DELTA[direction as usize];
    let facing_y = player_b.y + MAP_DIR_Y_DELTA[direction as usize];
    if player_b == player_a || (facing_x == player_a.x && facing_y == player_a.y) {
        return true;
    }
    let (ax, ay) = (player_a.x, player_a.y);
    let (fx, fy) = (facing_x, facing_y);
    match direction {
        0 => (ax >= fx && ay <= (fx - ax) + fy) || (ax <= fx && ay <= (ax - fx) + fy),
        1 => (ax >= fx && ay <= (fx - ax) + fy) || (ax >= (fx - fy) + ay && ay <= fy),
        2 => (ax >= (fx + fy - ay) && ay <= fy) || (ax >= (fx + ay - fy) && ay >= fy),
        3 => (ax >= (fx + ay) - fy && ay >= fy) || (ax >= fx && ay >= (ax - fx) + fy),
        4 => (ax >= fx && ay >= (ax - fx) + fy) || (ax <= fx && ay >= (fx - ax) + fy),
        5 => (ax <= fx && ay >= (fx - ax) + fy) || (ax <= (fx + fy) - ay && ay >= fy),
        6 => (ax <= (fx + fy) - ay && ay >= fy) || (ax <= (fx + ay) - fy && ay <= fy),
        _ => (ax <= (fx + ay) - fy && ay <= fy) || (ax <= fx && ay <= (ax - fx) + fy), // 7
    }
}

/// `FindCombatantDirection(target, attacker)` (`ovr032.cs:283`): the first iso
/// direction 0..=8 in which `attacker` can see `target` ([`can_see_combatant`]).
/// The [`build_near_targets`] sort's secondary key.
pub fn find_combatant_direction(target: GridPos, attacker: GridPos) -> u8 {
    let mut dir: u8 = 0;
    while dir < 8 && !can_see_combatant(dir, target, attacker) {
        dir += 1;
    }
    dir
}

/// A combatant as the range layer sees it: its footprint origin, footprint size
/// (`field_DE & 7`), and team. The full record isn't needed — reach only reads
/// positions and the team filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeCombatant {
    pub pos: GridPos,
    /// Footprint size (`CombatMap[i].size`); `0` = downed/absent (skipped, matching
    /// coab's `combatantMap.size > 0` gate).
    pub size: u8,
    pub team: Team,
}

/// One entry of a near-target list (`CombatPlayerIndex` + `SortedCombatant.steps`):
/// the roster index, cell, and the reach `steps` coab stored for the sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NearTarget {
    /// Roster index into the `combatants` slice given to [`build_near_targets`].
    pub idx: usize,
    pub pos: GridPos,
    /// `SortedCombatant.steps` — the REAL minimum path steps over the footprint
    /// cell pairs (binary `sub_738D8` stores actual steps; see §20 note below).
    pub steps: u16,
}

/// `BuildNearTargets(max_range, player)` → `Rebuild_SortedCombatantList`
/// (`ovr025.cs:1290`, `ovr032.cs:221-280`): the opposite-team combatants reachable
/// from `attacker_idx` within `max_range` tiles, **sorted nearest-first** (the
/// `SortedCombatant.CompareTo` order: `steps` asc, then `direction` asc). Draw-free.
///
/// **§20 bug #8 — the best-pair accumulator init (`sub_738D8` @`ovr032:097B`):**
/// the binary initializes the per-candidate best range `var_1F` to **0xFF (255)**,
/// NOT to `max_range` as coab wrote (`found_range = max_range`, ovr032.cs:243).
/// So the first successful reach ALWAYS fires the `steps < best` update: the
/// winning attacker/target footprint cells (`var_5`/`var_6`) and the REAL step
/// count are recorded for every entry, the entry stores real `steps` (@`+1` of
/// the stride-3 record at `DS:0x6EAE`), and the direction (@`+2`) is computed by
/// the `FindCombatantDirection` scan from the real winning cells. coab's
/// `max_range` init happens to COINCIDE with the binary at `max_range == 0xff`
/// (`find_target`'s lists were therefore correct) but degenerates at range 1:
/// every entry got `(steps=max_range, dir=0-from-(0,0))` and the sort collapsed
/// to roster order — the draw-747 re-pick divergence.
///
/// (The binary's `sub_738D8` also takes a direction arg (`arg_6`): if `< 8` it is
/// stored verbatim instead of scanned, and it pre-filters candidate cell pairs via
/// `sub_7354A`. Every path we model passes 0xFF — scan + no-op filter — so it is
/// not a parameter here.)
///
/// **Tie order:** `SortedCombatant.CompareTo` returns 0 for equal `(steps,
/// direction)` and coab's `List.Sort` is unstable, so the live order of exact ties
/// is statically unspecified; this uses a stable sort (roster order on ties) — a
/// documented micro-divergence that only a binary trace could pin.
pub fn build_near_targets(
    map: &CombatMap,
    combatants: &[RangeCombatant],
    attacker_idx: usize,
    max_range: i32,
    ignore_walls: bool,
) -> Vec<NearTarget> {
    let attacker = combatants[attacker_idx];
    let attacker_map = size_footprint(attacker.size, attacker.pos);

    let mut out: Vec<(NearTarget, u8)> = Vec::new();

    for (i, c) in combatants.iter().enumerate() {
        // combatantMap.size > 0 && filter(p.combat_team != attacker.combat_team).
        if c.size == 0 || c.team == attacker.team {
            continue;
        }
        let target_map = size_footprint(c.size, c.pos);

        let mut found = false;
        // Binary `ovr032:097B`: `mov [bp+var_1F], 0FFh` — 255, NOT `max_range`.
        let mut found_range: i32 = 0xFF;
        let mut found_target = GridPos::new(0, 0);
        let mut found_attacker = GridPos::new(0, 0);

        for &tp in &target_map {
            for &ap in &attacker_map {
                if let Some(steps) = can_reach(map, ap, tp, max_range, ignore_walls) {
                    found = true;
                    if (steps as i32) < found_range {
                        found_range = steps as i32;
                        found_target = tp;
                        found_attacker = ap;
                    }
                }
            }
        }

        if found {
            let dir = find_combatant_direction(found_target, found_attacker);
            out.push((
                NearTarget {
                    idx: i,
                    pos: c.pos,
                    steps: found_range as u16,
                },
                dir,
            ));
        }
    }

    // SortedCombatant.CompareTo: steps asc, then direction asc (the `direction%2`
    // tertiary key is 0 whenever directions are equal). Stable → roster order on
    // full ties.
    // §15 bug #5 — the near-target sort is the binary's `sub_73033` (ovr032:0033):
    // an EXCHANGE sort (swap-on-every-improvement) whose swap predicate is a
    // PARTIAL order, not a clean key. Element `j` sorts before element `i` when
    // `steps[j] < steps[i]`, OR (`steps` equal AND `dir[j] < dir[i]` AND
    // `dir[j]%2 <= dir[i]%2`). Incomparable pairs keep build (roster) order —
    // e.g. a `dir 1` (diagonal) and a `dir 2` (orthogonal) at equal steps are
    // never swapped, so the binary keeps the roster-earlier one first.
    //
    // The swap PLACEMENT is load-bearing under a non-transitive predicate
    // (exchange-in-inner-loop vs find-min-then-swap-once can order ties
    // differently), and it is confirmed from the disassembly: the 3-byte triple
    // swap at `ovr032:011A-0186` (temp←[i], [i]←[j], [j]←temp on the stride-3
    // entries @6EAE) runs IMMEDIATELY inside the inner loop and falls into the
    // inner-loop increment (`loc_7318B`); no min-index is tracked and the outer
    // loop closure swaps nothing. `out.swap(i, j)` inside the inner loop below
    // is therefore exact, not merely equivalent-on-total-orders.
    //
    // coab's `SortedCombatant.CompareTo` mis-orders this as a clean
    // `(steps, direction)` key (it has the `%2` term only as an unreachable
    // innermost tie-break); that gave the wrong `find_target` pick and the
    // round-0 movement cascade.
    let n = out.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let (si, di) = (out[i].0.steps, out[i].1 as i32);
            let (sj, dj) = (out[j].0.steps, out[j].1 as i32);
            let swap = sj < si || (sj == si && dj < di && (dj % 2) <= (di % 2));
            if swap {
                out.swap(i, j);
            }
        }
    }

    out.into_iter().map(|(nt, _)| nt).collect()
}

/// **`CombatWorld` is the former name of the now-unified [`CombatState`].** Kept
/// as a type alias so the audit-accepted slice-4 tests and both demos build the
/// fight by the name they always used, unchanged by the merge. `CombatWorld::new`
/// resolves to [`CombatState::new`] — the `(map, fighters)` full-fight constructor.
pub type CombatWorld = CombatState;

// The melee-AI turn and the round loop, on the one unified `CombatState`. These
// were the former `CombatWorld` methods; the model merge moved them onto the
// single state type. `new(map, fighters)`, the `sink` field, `attach_action_sink`/
// `take_action_sink`, and `emit` already live on the `CombatState` impl above (the
// former `CombatWorld::new`/`emit_action` were duplicates and were dropped), so
// they are not repeated here.
impl CombatState {
    /// `setup_mapToPlayerIndex_and_playerScreen` (`ovr033.cs:111`): repaint the
    /// occupancy grid from live fighter footprints (1-based index). Called after
    /// every position change, exactly as `sub_3E748` does.
    fn rebuild_occupancy(&mut self) {
        let placements: Vec<Placement> = self
            .fighters
            .iter()
            .map(|f| Placement {
                pos: f.pos,
                size: if f.in_combat { f.size } else { 0 },
                placed: true,
            })
            .collect();
        self.map.rebuild_occupancy(&placements);
    }

    /// `CanSeeTargetA` (`ovr014.cs:571`) — the **invisibility** affect check, not
    /// geometry. No affects are modeled, so a live target is always "seen".
    fn can_see_target(&self, target: usize) -> bool {
        self.fighters[target].in_combat
    }

    /// `clear_actions` → `Action.Clear` (`Classes/Action.cs`): zero `delay`,
    /// `guarding`, and `move` — but **keep** `field_15`/`target`/morale (persistent).
    fn clear_actions(&mut self, actor: usize) {
        let f = &mut self.fighters[actor];
        f.delay = 0;
        f.guarding = false;
        f.move_left = 0;
    }

    // --- the round loop (MainCombatLoop, ovr009.cs:22) ---------------------

    /// `(live party, live monsters)`.
    fn live_counts(&self) -> (usize, usize) {
        let mut party = 0;
        let mut monsters = 0;
        for f in &self.fighters {
            if f.in_combat {
                match f.team {
                    Team::Party => party += 1,
                    Team::Monster => monsters += 1,
                }
            }
        }
        (party, monsters)
    }

    /// `calc_enemy_health_percentage` (`sub_40E00` @`ovr014:2E00`, coab
    /// `ovr014.cs:1674`): `((20·ΣcurHP)/ΣmaxHP)·5` over the **monster** team —
    /// the morale/flee input (`byte_1D903`). Draw-free.
    ///
    /// **The denominator counts DEAD monsters** (`maxTotal += hit_point_max`
    /// runs for every enemy at `:2E4B`, reached whether or not `in_combat`),
    /// while the numerator only sums live enemies (`currentTotal +=
    /// hit_point_current` gated on `in_combat` at `:2E28`). So as a fight wears
    /// on, `enemyHealthPercentage` decays past what the surviving fraction alone
    /// would give — which is what drops it below `FleeCheck`'s gate-2 threshold
    /// and triggers the rout (the previous `in_combat`-only denominator kept it
    /// too high, so the faithful gate never fired). Binary-verified this session;
    /// safe for the closed captures because a monster's advance short-circuits on
    /// `|| team == Monster` (`moralFailureEscape`), so this value only ever moves
    /// the flee gate, which is closed at `field_58C = 99`.
    fn recompute_enemy_health(&mut self) {
        let (mut cur, mut max) = (0i32, 0i32);
        for f in &self.fighters {
            if f.team == Team::Monster {
                max += f.hp_max; // ALL enemies, dead included (:2E4B)
                if f.in_combat {
                    cur += f.hp_current; // live enemies only (:2E28)
                }
            }
        }
        self.enemy_health_pct = if max > 0 {
            (((20 * cur) / max) * 5).clamp(0, 100)
        } else {
            0
        };
    }

    /// `CalculateInitiative(i)` (`sub_3E000` @`ovr014.cs:8`) on the rich model:
    /// reset the Action scalars (`can_use`, `attack_idx = 2`, `field_8`; NOT
    /// `guarding`, §32), refresh the per-round attack counts (`reclac_attacks`
    /// for attack-1, `ThisRoundActionCount(baseHalfMoves)` for attack-2) and the
    /// move budget, and roll `delay = clamp(d6 + reaction_adj)` with the surprise
    /// `-6`. One d6 per in-combat fighter — the exact initiative draw of the
    /// audit-accepted [`CombatState`] slice.
    fn calculate_initiative(
        &mut self,
        rng: &mut EngineRng,
        i: usize,
        round: u16,
        surprise_mask: u8,
    ) {
        // The draw-free Action reset (can_use, attack_idx = 2, the 3/2 attack
        // count, the move budget). Scoped so its &mut borrow ends before the d6
        // draw and the Init emit.
        //
        // §32 bug #15: `guarding` is NOT reset here. `sub_3E000` writes only
        // `spell_id`/`can_cast`/`field_2`/`field_8`/`field_4`/`field_5`/
        // `delay`/`move` (`ovr014:0017-011A`) — the guard flag survives the
        // round boundary until the guard fires (`sub_3E65D`) or `Action.Clear`
        // runs. Clearing it here disarmed every cross-round guard: a parked
        // fleer's into-reach attack on an arriving PC never fired.
        // The draw-free head (`sub_3E000`, `ovr014.cs:12-16`): reset the Action
        // scalars. `field_8` (set by `AttackTarget01`) resets false HERE, so the
        // `reclac_attacks` write-back gate below sees a clean `!field_8` on the
        // per-round recompute (doc §34.3).
        {
            let f = &mut self.fighters[i];
            f.can_use = true;
            f.attack_idx = 2;
            f.field_8 = false;
        }
        // `reclac_attacks(player)` (`ovr014.cs:18`) sets `attack1_left` — the
        // ranged-aware per-round count (§34.3): a readied bow yields
        // `max(2, table[type].numberAttacks)` half-actions (LongBow 4 → 2
        // shots/round), a melee combatant its `attacksCount`. Draw-free.
        self.reclac_attacks(i);
        // §39.5 site 4: `CheckAffectsEffect(Movement)` in `CalculateInitiative`,
        // after `reclac_attacks` and before the attack-2 count — `mov al,12h;
        // call work_on_00` @`ovr014:005E` (coab ovr014.cs:23). A SECOND Movement
        // pass this call (the first is inside `reclac_attacks` above, @0E66);
        // both are draw-free no-ops on empty lists. (The third binary site, the
        // one inside `CalcMoves` @`ovr014:0179`/coab :76, lives in our pure
        // `calc_moves` helper — no emit seam; doc §40.)
        self.check_affects_effect(i, CheckType::Movement);
        // CalcInit tail (`ovr014.cs:19-27`): attack-2 = ThisRoundActionCount of
        // `baseHalfMoves`@0x11D (0 in this party → attack-2 never swings). The
        // `maxSweapTargets = attackLevel` write is deferred with the 0-HD sweep.
        let in_combat = {
            let f = &mut self.fighters[i];
            f.attack2_left = this_round_action_count(f.base_half_moves as i32, round) as u8;
            f.move_left = calc_moves(f.movement);
            f.in_combat
        };

        let team = self.fighters[i].team;
        let reaction_adj = self.fighters[i].reaction_adj;
        let (delay, surprise) = if in_combat {
            // action.delay = (sbyte)(roll_dice(6,1) + DexReactionAdj(player))
            let d6 = roll_dice(rng, 6, 1) as i32;
            let mut delay = d6 + reaction_adj as i32;
            // if (action.delay < 1) action.delay = 1;   ← BEFORE the -6
            if delay < 1 {
                delay = 1;
            }
            // if (((combat_team+1) & area2_ptr.field_596) != 0) action.delay -= 6;
            let surprise = ((team as i32 + 1) & surprise_mask as i32) != 0;
            if surprise {
                delay -= 6;
            }
            // if (action.delay < 0 || action.delay > 20) action.delay = 0;
            if !(0..=20).contains(&delay) {
                delay = 0;
            }
            (delay as i8, surprise)
        } else {
            (0, false)
        };

        let id = self.fighters[i].id;
        self.fighters[i].delay = delay;
        self.emit(ActionEvent::Init {
            combatant_id: id,
            delay,
            dex_adj: reaction_adj,
            surprise,
        });
    }

    /// `MainCombatLoop` (`ovr009.cs:22`) as a **thin driver over
    /// [`step`](Self::step)** (D8): pump the one tick machine to completion —
    /// `while step(rng) != Ended {}` — then read the [`CombatOutcome`] from the live
    /// team counts. The engine core is the tick machine; this is just the headless
    /// caller that runs it start to finish, so the whole all-AI fight (initiative
    /// d6s, then d100 selection passes interleaved with each actor's turn draws,
    /// study §2) flows through the single `step` path — no separate blocking loop.
    /// Returns the [`CombatOutcome`].
    pub fn run_combat(&mut self, rng: &mut EngineRng, max_rounds: u16) -> CombatOutcome {
        self.run_combat_observed(rng, max_rounds, |_, _| {})
    }

    /// [`run_combat`](Self::run_combat) with a per-round observer — `on_round(state,
    /// round)` fires after each round's turns resolve (when `step` reports the
    /// round ended), for transcripts/rendering, with the 0-based round index.
    /// Observation never touches the draw stream. This is the thin `step`-pumping
    /// driver; `max_rounds` is applied as the stalemate cap.
    pub fn run_combat_observed<F: FnMut(&CombatState, u16)>(
        &mut self,
        rng: &mut EngineRng,
        max_rounds: u16,
        mut on_round: F,
    ) -> CombatOutcome {
        self.no_action_limit = max_rounds;
        loop {
            match self.step(rng) {
                CombatStep::RoundEnded { round, battle_over } => {
                    // `round` is post-increment (1-based); the observer wants the
                    // 0-based index the old MainCombatLoop passed. A `round` of 0 is
                    // impossible here (battle_round_checks incremented it), so the
                    // subtraction never underflows.
                    on_round(self, round - 1);
                    if battle_over {
                        break;
                    }
                }
                CombatStep::Ended => break,
                _ => {}
            }
        }
        self.outcome()
    }
}

/// `ThisRoundActionCount` (`ovr014.cs:519`): `(halfActions + oddRound) / 2` — the
/// AD&D 3/2-attacks rule folded into a `combat_round`-parity test (§3.1). Odd
/// rounds get the `+1`.
pub fn this_round_action_count(half_actions: i32, round: u16) -> i32 {
    (half_actions + (round as i32 & 1)) / 2
}

/// The result of a full [`CombatState::run_combat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatOutcome {
    /// The monster team was wiped out.
    PartyWins,
    /// The party was wiped out.
    MonstersWin,
    /// Neither side could finish the other within the stalemate cap.
    Stalemate,
}

// --- the ECL COMBAT-opcode encounter runner (D3) --------------------------

/// One party member's combat-relevant stats, as team 0 of a script-triggered
/// encounter (M4 combat #6). The engine maps a `crate::party::Character` into
/// this at the `COMBAT` opcode; kept a plain struct so [`run_encounter`] is
/// unit-testable without the full party model.
///
/// `dice` is the equipped-weapon damage die. Real weapon dice live in the
/// `.swg` `ItemData` records, which are **not decoded yet** (FD-29's weapon
/// clause, M5-adjacent) — the caller passes a documented default until then.
/// DEX-reaction / strength folding into the initiative adjustment and to-hit
/// bonus (`hitBonus@0x199`, a `BattleSetup` concern) is likewise deferred, so
/// `reaction_adj` starts 0 here exactly as the accepted `watch_a_real_data_fight`
/// demo has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartyCombatStats {
    pub hp: i32,
    /// Raw on-disk AC (`@0x19a`); displayed AC = `0x3C - ac`.
    pub raw_ac: u8,
    /// To-hit bonus in the raw-AC compare space (the record's stored THAC0,
    /// matching the monster path).
    pub hit_bonus: i32,
    pub movement: i32,
    /// `(dice_count, dice_size, damage_bonus)` for the equipped weapon.
    pub dice: (u8, u8, u8),
    pub npc: bool,
}

/// The result of a script-triggered encounter: the fight's [`CombatOutcome`]
/// plus the rounds it ran (for transcripts/logging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncounterOutcome {
    pub outcome: CombatOutcome,
    pub rounds: u16,
}

/// `sub_304B4` (`ovr008.cs:157`): the forward line-of-sight distance that sets
/// how far ahead the monster team deploys — **draw-free** (a wall ray, no
/// `roll_dice`; verified by reading it, and the reason this slice's
/// opcode→combat path adds no draw before the first initiative d6).
///
/// Wilderness/city (`inDungeon == 0`) is always 2. In a dungeon it casts a ray
/// up to 2 cells forward along `map_dir` (0/2/4/6 = N/E/S/W), stepping while
/// the wall in the facing direction is open (`getMap_wall_type == 0`) and
/// stopping at the first wall. Out-of-grid steps stop the ray (treated as
/// blocked). Note: coab also clamps this against `SETUP MONSTER`'s
/// `max_encounter_distance` (`ovr003.cs:231`) and any prior value
/// (`CMD_Combat`, `ovr003.cs:999`) — those upper clamps are deferred (we don't
/// yet carry `area2_ptr.encounter_distance` across opcodes), so the raw ray
/// result stands; realistically 1-2.
pub fn encounter_distance(
    geo: &GeoBlock,
    map_dir: u8,
    map_x: i32,
    map_y: i32,
    in_dungeon: bool,
) -> u8 {
    if !in_dungeon {
        return 2;
    }
    let grid = gbx_formats::geo::GEO_GRID_SIZE as i32;
    let (mut x, mut y) = (map_x, map_y);
    let mut dist = 0u8;
    for _ in 0..2 {
        if !(0..grid).contains(&x) || !(0..grid).contains(&y) {
            break;
        }
        let s = geo.square(x as usize, y as usize);
        let wall = match map_dir {
            0 => s.wall_north,
            2 => s.wall_east,
            4 => s.wall_south,
            6 => s.wall_west,
            _ => s.wall_north,
        };
        if wall != 0 {
            break; // a wall blocks the ray
        }
        dist += 1;
        match map_dir {
            0 => y -= 1,
            2 => x += 1,
            4 => y += 1,
            6 => x -= 1,
            _ => {}
        }
    }
    dist
}

/// Run the `COMBAT` opcode's **real-combat branch** — `CMD_Combat`'s `else`
/// (monsters were loaded), `ovr003.cs:1004` → `MainCombatLoop` (M4 combat #6).
/// The party is team 0, the script-loaded monsters are team 1, placed
/// `encounter_distance` tiles ahead along `map_dir`; the whole all-AI melee
/// fight then runs through the one unified tick engine
/// ([`CombatState::run_combat`]) to a victor.
///
/// **Draw discipline:** everything before the first initiative d6 — placement
/// ([`place_combatants`]), the [`provisional_combat_map`] terrain, and
/// [`encounter_distance`] — is draw-free, so the returned fight's draw stream
/// begins exactly with the §2 initiative fingerprint (asserted by combat #6's
/// draw-parity test). `AfterCombatExpAndTreasure` (XP/treasure) and the
/// non-combat (shop/temple) branch are **deferred** (handled by the caller /
/// out of scope).
pub fn run_encounter(
    party: &[PartyCombatStats],
    monsters: &[LoadedMonster],
    mut map: CombatMap,
    map_dir: u8,
    encounter_distance: u8,
    rng: &mut EngineRng,
) -> EncounterOutcome {
    let inputs: Vec<PlacementInput> = party
        .iter()
        .map(|_| PlacementInput {
            team: Team::Party,
            size: 1,
            in_combat: true,
        })
        .chain(monsters.iter().map(|_| PlacementInput {
            team: Team::Monster,
            size: 1,
            in_combat: true,
        }))
        .collect();
    let placements = place_combatants(
        &mut map,
        &inputs,
        map_dir,
        encounter_distance as i32,
        GridPos::new(0, 0),
        None,
    );

    let mut fighters: Vec<Combatant> = Vec::with_capacity(party.len() + monsters.len());
    for (i, p) in party.iter().enumerate() {
        fighters.push(Combatant::new_melee(
            i,
            Team::Party,
            p.npc,
            placements[i].pos,
            p.hp,
            p.raw_ac,
            p.hit_bonus,
            p.movement,
            p.dice,
            0, // delay — CalculateInitiative sets it each round
            1, // one swing/round
        ));
    }
    for (k, m) in monsters.iter().enumerate() {
        let a1 = m.attacks[0];
        let id = party.len() + k;
        fighters.push(Combatant::new_melee(
            id,
            Team::Monster,
            m.is_npc(),
            placements[id].pos,
            m.hit_point_max as i32,
            m.ac as u8,
            m.thac0 as i32,
            m.movement as i32,
            (a1.dice_count, a1.dice_size, a1.damage_bonus as u8),
            0,
            1,
        ));
    }

    let mut state = CombatState::new(map, fighters);
    state.map_direction = map_dir;
    let mut rounds = 0u16;
    let outcome = state.run_combat_observed(rng, DEFAULT_NO_ACTION_LIMIT, |_, r| {
        rounds = r + 1;
    });
    EncounterOutcome { outcome, rounds }
}

#[cfg(test)]
mod tests;
