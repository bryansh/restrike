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
use gbx_formats::geo::GeoBlock;
use gbx_formats::save_orig::{decode_char_record, CharRecord, SaveParseError};
use gbx_rules::flavor::Flavor;

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
    /// (the toggle key is UI, not modeled); replay harnesses set it per capture.
    pub auto_pcs_cast_magic: bool,
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
                    self.melee_ai_turn(rng, idx);
                } else {
                    self.clear_actions(idx);
                }
            }
        }
    }

    /// `BattleRoundChecks` (`ovr009.cs:363`, `battle01`) reduced to its
    /// non-stubbed parts: increment the round counter, run the bleed tick, and
    /// decide the loop exit. `step_game_time`, the per-member affect ticks
    /// (`CheckAffectsEffect(Type_19)`), cloud damage (`in_poison_cloud`), the
    /// display-only `bandage(false)` "Your Teammate is Dying" scan, and
    /// `calc_enemy_health_percentage` (recomputed at `begin_round` instead, both
    /// draw-free) are gated on systems not in this slice.
    fn battle_round_checks(&mut self) -> CombatStep {
        // ovr009.cs:366 — the byte_1D8B7 increment.
        self.combat_round += 1;

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

// ===========================================================================
// The melee QuickFight AI (M4 combat #4; study §4.1, D-OR5(a) Phase 1)
// ===========================================================================
//
// **In progress — deliverable 3.** This section transliterates the draw-bearing
// pieces of `PlayerQuickFight` (`ovr010.cs:8`) in draw order per the §4.1 map. The
// `field_15` mode-gate lands first — the turn's *first* draw site and the study's
// #1 landmine (the `||` short-circuit). The behavior-guard d7s, `find_target`
// picks, and the `sub_35DB1` move-attack loop (with the per-step monster d100 and
// the opportunity attacks) are the remaining pieces; see the handoff. Every draw
// flows through the one `EngineRng`/`roll_dice` seam (D9).

/// The QuickFight `field_15` **target-mode gate** (`sub_3504B` @`ovr010:0090`;
/// study §4.1.2, corrected by the §15 binary RE) — the very first draws of a
/// melee AI turn, run before any target selection. Given the combatant's
/// persistent `field_15` (`Action@0x15`, which `CalculateInitiative` does **not**
/// reset), returns its new value and draws faithfully.
///
/// ```text
/// if (field_15 == 0 || field_15 > 4 || roll_dice(4,1) == 1) {   // d4 GATE (short-circuits)
///     v = roll_dice(8,1);                                       // d8
///     v = (v != 8) ? roll_dice(4,1) : roll_dice(2,1) + 4;       // d4 (→1..4) or d2+4 (→5..6)
/// }
/// ```
///
/// **§15 binary correction (bug #1).** This supersedes combat #4 D1's coab-derived
/// reading, which was wrong two ways against the binary at `ovr010:0090`:
/// - the entry short-circuit is `field_15 > 4` (`cmp 4; ja loc_350AB`), **not**
///   `== 4`; and
/// - the `d8` body branches are **swapped** — `d8 != 8` draws `roll_dice(4,1)`
///   (`loc_350D4` → 1..4) and `d8 == 8` draws `roll_dice(2,1)+4` (→ 5..6). coab/our
///   old code had these reversed, drawing a `d2` in the common `d8 != 8` case.
///
/// **The `||` short-circuit is still the landmine (D9):** when `field_15` is 0 or
/// `> 4` the `roll_dice(4,1)` gate is **not evaluated** — that turn draws only the
/// body's **2** dice (d8 then d4|d2), not 3. Since `field_15` starts at 0, *every
/// combatant's first turn* takes this 2-draw path. When `field_15 ∈ 1..=4`: one d4
/// gate draw always; then the 2-draw body only if the gate rolled 1 (so 1 or 3
/// draws). The result is always in `1..=6`.
pub fn field_15_mode_gate(rng: &mut EngineRng, field_15: u8) -> u8 {
    let mut v = field_15 as u16;
    // ovr010:0090 — `cmp 0; jz body` / `cmp 4; ja body` then the d4 gate. The `||`
    // short-circuits, so roll_dice(4,1) is skipped for field_15 ∈ {0} ∪ {>4}.
    let enter = v == 0 || v > 4 || roll_dice(rng, 4, 1) == 1;
    if enter {
        v = roll_dice(rng, 8, 1); // ovr010:00AB — d8
        if v != 8 {
            v = roll_dice(rng, 4, 1); // ovr010:00D4 — d8!=8 → 1..4
        } else {
            v = roll_dice(rng, 2, 1) + 4; // ovr010:00BF — d8==8 → 5..6
        }
    }
    v as u8
}

/// `data_2B8` (`seg600:02BD`) — the approach-angle table. Each entry is an
/// iso-direction *offset* added to the heading toward the target, so `field_15`
/// selects an "approach personality" (straight vs. weaving) and `dirStep` (1..=6)
/// is the retry index `CanMove`/`moralFailureEscape` walk. Value 8 = "no
/// direction". 11 rows materialized from coab's 6-wide windows.
///
/// **§15 binary correction (bug #2).** The binary (`CanMove`/`sub_3573B`
/// @`ovr010:076D`) indexes the *flat* table as `byte[0x2B8 + 5·field_15 + dirStep]`
/// = `T[5·(field_15−1) + dirStep]` (base `0x2BD`) — a **stride-5 sliding window**.
/// coab materialized the overlapping windows into these 6-wide rows and indexed
/// row `field_15`, an off-by-one: coab row `R` is `T[5R+1 ..= 5R+6]`, so binary
/// `field_15 = N` reads coab **row N−1**. Both call sites therefore index
/// [`DATA_2B8`]`[field_15 − 1]` (post-gate `field_15` is always 1..=6). Verified
/// `DATA_2B8[N−1][dirStep−1] == T[5·(N−1)+dirStep]` for `dirStep` 1..=6.
const DATA_2B8: [[i32; 6]; 11] = [
    [8, 7, 6, 1, 2, 8],
    [8, 1, 2, 7, 6, 7],
    [7, 1, 8, 6, 2, 1],
    [1, 7, 8, 2, 6, 8],
    [8, 7, 6, 5, 4, 8],
    [8, 1, 2, 3, 4, 8],
    [8, 4, 6, 2, 8, 6],
    [6, 4, 0, 8, 0, 6],
    [6, 2, 8, 2, 0, 4],
    [4, 0, 0, 2, 6, 2],
    [2, 2, 0, 4, 4, 4],
];

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
    /// The range layer's view of the roster (`size = 0` for the dead, so they drop
    /// out of target lists — matching coab's `combatantMap.size > 0` gate).
    fn range_combatants(&self) -> Vec<RangeCombatant> {
        self.fighters
            .iter()
            .map(|f| RangeCombatant {
                pos: f.pos,
                size: if f.in_combat { f.size } else { 0 },
                team: f.team,
            })
            .collect()
    }

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

    /// `BuildNearTargets(max_range, actor)` over the live roster.
    fn build_near(&self, actor: usize, max_range: i32, ignore_walls: bool) -> Vec<NearTarget> {
        build_near_targets(
            &self.map,
            &self.range_combatants(),
            actor,
            max_range,
            ignore_walls,
        )
    }

    // --- the combat camera (doc §36.3) -------------------------------------
    //
    // `mapScreenTopLeft` + `focusCombatAreaOnPlayer` are pure display state:
    // the ONLY consumer that changes a draw is `AttackTarget`'s on-screen
    // facing branch (§36.1), so these ports carry ONLY each display function's
    // persistent-state effect — the tile/icon/overlay rendering is stubbed.

    /// `CoordOnScreen(pos)` (`ovr033.cs:213`) for a screen-space cell (already
    /// `map − mapScreenTopLeft`): inside the 7×7 window `0..=6` on both axes.
    fn coord_on_screen(screen_x: i32, screen_y: i32) -> bool {
        (0..=SCREEN_MAX).contains(&screen_x) && (0..=SCREEN_MAX).contains(&screen_y)
    }

    /// Is map cell `p` inside the current combat window? (`CoordOnScreen(p −
    /// mapScreenTopLeft)`.) The size-1 form of `PlayerOnScreen` for a cell,
    /// independent of a combatant's `in_combat`/`size` — used at the
    /// `CombatantKilled` scroll, which tests the victim while it is still
    /// present (`ovr033.cs:550`, before `size = 0`).
    fn on_screen_pos(&self, p: GridPos) -> bool {
        Self::coord_on_screen(
            p.x - self.map_screen_top_left.x,
            p.y - self.map_screen_top_left.y,
        )
    }

    /// `PlayerOnScreen(_, combatant)` (`ovr033.cs:227`) for a **size-1**
    /// combatant (every combatant in every capture; a size>1 loadout tripwires
    /// elsewhere): `size == 0` (a removed combatant) ⇒ off-screen, else the
    /// single cell's [`on_screen_pos`]. The `AllOnScreen` arg is irrelevant for
    /// one cell, so both `PlayerOnScreen(false, …)` and `PlayerOnScreen(true,
    /// …)` map here.
    fn on_screen(&self, idx: usize) -> bool {
        self.fighters[idx].in_combat && self.on_screen_pos(self.fighters[idx].pos)
    }

    /// `ScreenMapCheck(radius, pos)` (`ovr033.cs:266`, `sub_749DD`'s scroll
    /// primitive) reduced to its persistent effect on `mapScreenTopLeft`: if
    /// forced (`radius == 0xFF`) or `pos` lies outside the ±`radius` box around
    /// the current screen centre, step the centre coordinate-wise toward `pos`
    /// — each axis clamped to `[MapMin + 3, MapMax − 3 − 1]` (`x ∈ [3, 46]`,
    /// `y ∈ [3, 21]`) — and rewrite `mapScreenTopLeft = centre − (3,3)`. Returns
    /// whether it scrolled. The 7×7 tile redraw + `calculatePlayerScreenPositions`
    /// are display (screenPos is derived live here). Binary-cited: the box test
    /// + clamp bounds, `ovr033.cs:278-314`.
    fn screen_map_check(&mut self, radius: i32, pos: GridPos) -> bool {
        let mut cx = self.map_screen_top_left.x + SCREEN_HALF;
        let mut cy = self.map_screen_top_left.y + SCREEN_HALF;
        let var2 = if radius == 0xFF { 0 } else { radius };
        let (min_x, max_x) = (cx - var2, cx + var2);
        let (min_y, max_y) = (cy - var2, cy + var2);
        if radius == 0xFF || pos.x < min_x || pos.x > max_x || pos.y < min_y || pos.y > max_y {
            if pos.x < min_x {
                while pos.x < cx && cx > MAP_MIN + SCREEN_HALF {
                    cx -= 1;
                }
            } else if pos.x > max_x {
                while pos.x > cx && cx < MAP_W - SCREEN_HALF - 1 {
                    cx += 1;
                }
            }
            if pos.y < min_y {
                while pos.y < cy && cy > MAP_MIN + SCREEN_HALF {
                    cy -= 1;
                }
            } else if pos.y > max_y {
                while pos.y > cy && cy < MAP_H - SCREEN_HALF - 1 {
                    cy += 1;
                }
            }
            self.map_screen_top_left = GridPos::new(cx - SCREEN_HALF, cy - SCREEN_HALF);
            return true;
        }
        false
    }

    /// `redrawCombatArea(dir, radius, map)` (`ovr033.cs:344`) reduced to its
    /// sole persistent effect: `ScreenMapCheck(radius, map + delta[dir])` (`dir
    /// == 8` ⇒ probe `map` in place). The per-icon repaint loop + `RedrawPosition`
    /// + the `MapBoundaryTrunc` local are display-only.
    fn redraw_combat_area(&mut self, dir: u8, radius: i32, map: GridPos) {
        let probe = map.stepped(dir);
        self.screen_map_check(radius, probe);
    }

    /// `draw_74B3F(arg0, iconState, direction, combatant)` (`ovr033.cs:376`)
    /// reduced to its two persistent effects (§36.1): (1) the focus-gated
    /// off-screen **recenter** — `redrawCombatArea(8, 3, combatant.pos)` when
    /// the combatant is not fully on-screen and `focus` is on (`ovr033.cs:380`)
    /// — and (2) the **unconditional** `combatant.direction = direction` store
    /// (`ovr033.cs:396`), which is why the on-screen draw overwrites the target's
    /// facing. The background/icon repaints (`arg0`/`iconState`-gated) are display.
    fn draw_74b3f(&mut self, idx: usize, direction: u8) {
        if !self.on_screen(idx) && self.focus {
            let p = self.fighters[idx].pos;
            self.redraw_combat_area(8, 3, p);
        }
        self.fighters[idx].direction = direction;
    }

    /// Site 5 — the persistent `mapScreenTopLeft` effect of a ranged shot's
    /// missile animation (`draw_missile_attack`, `sub_67AA4`, `ovr025.cs:882-1010`).
    /// The pixel-by-pixel overlay animation is display and draw-free, so only
    /// the scroll skeleton is ported:
    /// - the `SteppingPath` over the ×3 pixel grid gives `var_AF`; if `var_B0 =
    ///   var_AF − 2 < 2` (a very short path) the routine returns before any
    ///   scroll (`ovr025.cs:910-915`);
    /// - both endpoints on-screen ⇒ `center1 = current centre` ⇒
    ///   `redrawCombatArea(8, 0xFF, center1)` is a force-recenter no-op
    ///   (`ovr025.cs:934-940`);
    /// - either endpoint off-screen with `|Δ| ≤ 6` on both axes ⇒ force-scroll
    ///   to the midpoint `center1 = Δ/2 + attacker` (`ovr025.cs:922-926/940`);
    /// - either endpoint off-screen with a span > 6 ⇒ the missile leaves the
    ///   screen before reaching the target, so the animation force-scrolls to a
    ///   target-anchored centre `center2 = target + clamp` that brings the
    ///   target on-screen (`ovr025.cs:1010-1032`).
    fn draw_missile_camera(&mut self, attacker: usize, target: usize) {
        let a = self.fighters[attacker].pos;
        let t = self.fighters[target].pos;
        let var_af = missile_path_pixel_steps(a, t);
        let var_b0 = var_af as i32 - 2;
        if var_b0 < 2 || (var_af as i32) < 2 {
            return; // ovr025.cs:912 — `var_B0 < 2 || var_AF < 2` early return.
        }
        let a_on = self.on_screen_pos(a);
        let t_on = self.on_screen_pos(t);
        if a_on && t_on {
            return; // center1 = current centre → force-recenter is a no-op.
        }
        let diff = GridPos::new(t.x - a.x, t.y - a.y);
        if diff.x.abs() <= 6 && diff.y.abs() <= 6 {
            // center1 = midpoint (ovr025.cs:926).
            let center = GridPos::new(diff.x / 2 + a.x, diff.y / 2 + a.y);
            self.screen_map_check(0xFF, center);
        } else {
            // center2 (ovr025.cs:1010-1030): anchor the window on the target,
            // pushed back in-bounds by var_CE/var_D0.
            let mut var_ce = 0;
            if t.x + SCREEN_HALF > MAP_W {
                var_ce = t.x - MAP_W;
            } else if t.x < SCREEN_HALF {
                var_ce = SCREEN_HALF - t.x;
            }
            let mut var_d0 = 0;
            if t.y + SCREEN_HALF > MAP_H {
                var_d0 = t.y - MAP_H;
            } else if t.y < SCREEN_HALF {
                var_d0 = SCREEN_HALF - t.y;
            }
            let center = GridPos::new(t.x + var_ce, t.y + var_d0);
            self.screen_map_check(0xFF, center);
        }
    }

    /// `clear_actions` → `Action.Clear` (`Classes/Action.cs`): zero `delay`,
    /// `guarding`, and `move` — but **keep** `field_15`/`target`/morale (persistent).
    fn clear_actions(&mut self, actor: usize) {
        let f = &mut self.fighters[actor];
        f.delay = 0;
        f.guarding = false;
        f.move_left = 0;
    }

    /// `TryGuarding` (`sub_361F7` @`ovr010:11F7`, coab `ovr010.cs:685`): `IsHeld ||
    /// is_weapon_ranged || delay == 0` → `Action.Clear` (a ranged attacker NEVER
    /// parks a guard, §34.4); else `guarding()` = clear then set `guarding =
    /// true`. Either way `delay` ends 0, so it is not re-picked. `IsHeld` (a held
    /// affect) is not modeled → false. Draw-free.
    fn try_guarding(&mut self, actor: usize) {
        if self.is_weapon_ranged(actor) || self.fighters[actor].delay == 0 {
            self.clear_actions(actor);
        } else {
            self.clear_actions(actor);
            self.fighters[actor].guarding = true;
        }
    }

    /// `RemoveFromCombat(name, status, player)` (`sub_644A7` @`ovr024:14A7`) — drop
    /// a combatant from combat with a given health status. A not-in-combat combatant
    /// is a no-op (`:14C0`). Else: display (draw-free); `in_combat = false`
    /// (`:1506`); `health_status = status` (`:1512`); and — **only when `status !=
    /// running`** (`:151A`) — `hit_point_current = 0` (`:1525`); then
    /// `CombatMap[idx].size = 0` + `sub_743E7` occupancy repaint (`:154A-154F`) and
    /// `clear_actions` (`:155A`). **No `Tile_DownPlayer` stamp** — that is
    /// `CombatantKilled` (the damage-death path) only. Draw-free.
    ///
    /// (Callers: the FleeCheck surrender branch with `Unconscious`, and
    /// [`flee_battle`]'s Got-Away removal with [`HealthStatus::Running`].)
    fn remove_from_combat(&mut self, actor: usize, status: HealthStatus) {
        if !self.fighters[actor].in_combat {
            return; // :14C0-14CB — already out of combat.
        }
        // Site 6 (flee/surrender path) — `RedrawCombatIfFocusOn(false, 3, player)`
        // (`ovr024.cs:624`, `sub_75356`): a focus-on removal scrolls the camera
        // to the leaver (radius 3) BEFORE `size = 0`.
        if self.focus {
            let p = self.fighters[actor].pos;
            self.redraw_combat_area(8, 3, p);
        }
        {
            let f = &mut self.fighters[actor];
            f.in_combat = false; // :1506
            f.health_status = status; // :1512
            if status != HealthStatus::Running {
                f.hp_current = 0; // :1525 — skipped for `running` (the Got-Away case)
            }
        }
        // :154A CombatMap[idx].size = 0 + :154F sub_743E7 occupancy repaint.
        self.rebuild_occupancy();
        // :155A clear_actions.
        self.clear_actions(actor);
    }

    /// `FleeCheck_001` (`sub_3637F` @`ovr010:137F`, coab `ovr010.cs:760`) — the
    /// faithful morale ladder, **draw-free**. Sets `moral_failure`/`fleeing` (the
    /// flee outcome the move path acts on) and returns the surrender flag (`var_1`,
    /// the turn-ending `RemoveFromCombat("Surrenders")` path — §28 item 7, built in
    /// the next slice; here still the `surrender-int5` tripwire). Transliterated
    /// site-by-site from the IDA listing (each `ovr010:` cited); re-verified against
    /// `coab_new.lst` this session.
    fn flee_check(&mut self, actor: usize) -> bool {
        // :1385 var_1 = 0 (the surrender return flag).
        // :1391 actions.field_14 = 0 → moral_failure = false; RemoveAttackersAffects
        // (:139C) is draw-free, no affects modeled.
        self.fighters[actor].moral_failure = false;
        // :13A9 fleeing (actions.field_10) → moral_failure = 1, "is forced to
        // flee", return false.
        if self.fighters[actor].fleeing {
            self.fighters[actor].moral_failure = true;
            return false;
        }
        // :13E3 control_morale@0xF7 > 0x7F (unsigned `ja`) else return false —
        // i.e. NPCs only (a PC short-circuits the whole block).
        if !self.fighters[actor].npc {
            return false;
        }
        // :13F1-13FC per-actor morale seed = (control_morale & 0x7F) << 1, recomputed
        // EVERY call (the deviation slice-2 replaces: the old stub used a process-
        // lifetime scratch stuck at 100). :13FF `> 0x66` (102) → 0. Then
        // CheckAffectsEffect(Morale=0x11) at :140B — draw-free.
        let mut morale = ((self.fighters[actor].control_morale & 0x7F) as i32) << 1;
        if morale > 0x66 {
            morale = 0;
        }
        self.monster_morale = morale;

        // Gate 1 (:143F-144D): morale < (100 − hp_cur·100/hp_max) SIGNED (`jl`)
        // OR morale == 0; else return false.
        let hp_pct = (self.fighters[actor].hp_current * 100) / self.fighters[actor].hp_max.max(1);
        if self.monster_morale < (100 - hp_pct) || self.monster_morale == 0 {
            // :1458 monster_morale = byte_1D903 (enemyHealthPercentage); second
            // CheckAffectsEffect(Morale) at :145E — draw-free.
            self.monster_morale = self.enemy_health_pct;

            // Gate 2 (:146C-1493): morale < (100 − area2.field_58C) — ★ bug #12:
            // UNSIGNED 16-bit `jb` at :1481 over a 16-bit `sub` at :1473, so a
            // `field_58C > 100` underflows `100 − field_58C` to ~0xFFxx and the gate
            // is ALWAYS true (coab's signed int makes it always false). Transliterate
            // as u16 wrapping subtraction + unsigned compare. — OR morale == 0 OR
            // combat_team == Party (`:148D cmp combat_team, 0`).
            let lhs = self.monster_morale as u16;
            let rhs = 100u16.wrapping_sub(self.area_field_58c as u16);
            if lhs < rhs || self.monster_morale == 0 || self.fighters[actor].team == Team::Party {
                // Speed fork (:1498-14BE): MaxOppositionMoves > CalcMoves/2 SIGNED
                // (`jg` at :14BE) → the surrender branch (loc_364F7); else (`<=`)
                // moral_failure = 1 (:14C8) + remove_affect(0x4A)/remove_affect(0x4B)
                // (:14DC/:14F0 — both no-ops here, no affects modeled).
                let max_opp = self.max_opposition_moves(actor);
                if max_opp > calc_moves(self.fighters[actor].movement) / 2 {
                    // Surrender branch (loc_364F7, :14F7-1529, §28 item 7). The
                    // `surrender-int5` wire (kept, repurposed) fires whenever this
                    // implemented-but-capture-unproven branch executes — the rout
                    // capture never reaches it (its 12-vs-12 speed tie always takes
                    // the flee fork), so a firing marks an untested path.
                    self.emit(ActionEvent::StubTripped {
                        combatant_id: actor,
                        stub: "surrender-int5",
                    });
                    // :14FA `cmp byte es:[di+13h], 5; jbe → return false` — surrender
                    // only when `Int@0x13 > 5`.
                    if self.fighters[actor].int_score > 5 {
                        // :1501-1519 `RemoveFromCombat("Surrenders", status=4
                        // unconscious)`; :1524 clear_actions; return true (turn
                        // over — melee_ai_turn step 2 returns on it).
                        self.remove_from_combat(actor, HealthStatus::Unconscious);
                        return true;
                    }
                } else {
                    self.fighters[actor].moral_failure = true;
                }
            }
        }
        false
    }

    /// `MaxOppositionMoves` (`ovr014.cs:1699`) — the largest half-move budget over
    /// the live opposite team. Draw-free.
    fn max_opposition_moves(&self, actor: usize) -> i32 {
        let team = self.fighters[actor].team;
        self.fighters
            .iter()
            .filter(|f| f.in_combat && f.team != team)
            .map(|f| calc_moves(f.movement) / 2)
            .max()
            .unwrap_or(0)
    }

    /// `sub_354AA` (`ovr010:04AA`) — the wand scan. The binary rolls the **d7
    /// unconditionally at proc entry** (`ovr010:04C6`: `call roll_dice(7,1)` into
    /// `var_3`) and only THEN checks `can_use` (`:04D6`), the opposite-team live
    /// count (`:04EE`, `friends_count[on_our_team(actor)]`), and
    /// `area.can_cast_spells` (`:04FC`) — those guards gate the **item scan**, not
    /// the roll. (coab ovr010.cs:188 hoisted the guard above the roll — coab ≠
    /// binary; the difference is only visible when a guard goes false, e.g. the
    /// last enemy died earlier this round.) The scan itself is draw-free for a
    /// weapon-only combatant (no readied spell-item), so this always returns
    /// `false` (no wand used). Wand *effects* are deferred (M5).
    fn wand_scan_d7(&mut self, rng: &mut EngineRng, _actor: usize) -> bool {
        let _priorities = roll_dice(rng, 7, 1); // ovr010:04C6 — before the guards
        false
    }

    /// `find_target(clear, arg_2, max_range, actor)` (`ovr014.cs:2238`): keep a
    /// still-valid target (**0 draws**), else pick a random near-target
    /// (`roll_dice(nearTargets.Count, 1)` per retry, `:2275`). With no invisibility
    /// modeled, `CanSeeTargetA` is always true, so the first pick succeeds — exactly
    /// **1 draw** when a target is found from scratch, 0 when none exist or the old
    /// target survives. Two passes (the second `ignoreWalls`) as coab.
    fn find_target(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        clear: bool,
        arg_2: u8,
        max_range: i32,
    ) -> bool {
        let team = self.fighters[actor].team;
        let invalidate = clear
            || match self.fighters[actor].target {
                Some(t) => {
                    let tf = &self.fighters[t];
                    tf.team == team || !tf.in_combat || !self.can_see_target(t)
                }
                None => false,
            };
        if invalidate {
            self.fighters[actor].target = None;
        }
        if self.fighters[actor].target.is_some() {
            return true;
        }

        let mut found = false;
        let mut second_pass = false;
        let mut var_5 = false;
        while !found && !var_5 {
            var_5 = second_pass;
            let ignore_walls = second_pass && !clear;
            let mut near = self.build_near(actor, max_range, ignore_walls);
            let mut try_count = 20;
            while try_count > 0 && !found && !near.is_empty() {
                try_count -= 1;
                let roll = roll_dice(rng, near.len() as u16, 1); // ovr014.cs:2275
                let epi = near[(roll - 1) as usize];
                if (arg_2 != 0 && ignore_walls) || self.can_see_target(epi.idx) {
                    found = true;
                    self.fighters[actor].target = Some(epi.idx);
                } else {
                    near.retain(|n| n.idx != epi.idx);
                }
            }
            if !second_pass {
                second_pass = true;
            }
        }
        found
    }

    /// `damage_player` (`ovr025:23D5`, coab ovr025.cs:1183-1242) — apply melee
    /// damage and run the health-status ladder (§26.1). `neg_hp` is the overkill
    /// (`damage − hp`, else 0); `new_hp` the survivor's HP (`hp − damage`, else 0):
    /// - overkill `> 9`, or a `new_hp == 0` hit on an `animated` combatant → **dead**;
    /// - else overkill `1..=9` → **dying**, and `actions.bleeding = neg_hp`;
    /// - else an exact drop to 0 (`new_hp == 0`) → **unconscious**.
    ///
    /// A combatant left `okey`/`animated` keeps `new_hp` and stays in combat; any
    /// other status flips `in_combat = false`, zeroes HP and `actions.delay`
    /// (`ovr025:24BB` — the corpse can never win a `FindNextCombatant` pass, bug
    /// #9), and frees its occupancy footprint immediately (`CombatantKilled`,
    /// bug #10). `gbl.game_state == GameState.Combat` holds on this path, so the
    /// `bleeding` and `delay = 0` writes are unconditional here.
    fn apply_damage(&mut self, target: usize, amount: i32) {
        let t = &mut self.fighters[target];
        let (neg_hp, new_hp) = if t.hp_current >= amount {
            (0, t.hp_current - amount)
        } else {
            (amount - t.hp_current, 0)
        };

        // The ladder (ovr025.cs:1197-1216).
        if neg_hp > 9 || (new_hp == 0 && t.health_status == HealthStatus::Animated) {
            t.health_status = HealthStatus::Dead;
        } else if neg_hp > 0 {
            t.health_status = HealthStatus::Dying;
            t.bleeding = neg_hp as u8;
        } else if new_hp == 0 {
            t.health_status = HealthStatus::Unconscious;
        }

        // Survivor (ovr025.cs:1218): status stayed okey/animated → keep the
        // reduced HP, stay in combat.
        if t.health_status.is_conscious() {
            t.hp_current = new_hp;
            return;
        }

        // Removed from combat (ovr025.cs:1220-1240). Site 6 (death path) —
        // `CombatantKilled` (`sub_74E6F` @`ovr033:550`) FIRST scrolls the
        // camera to the victim if it is off-screen: `if (PlayerOnScreen(true,
        // victim) == false) redrawCombatArea(8, 3, victim.pos)`, evaluated while
        // the victim is still present (before `size = 0`). **Deviation:** the
        // spec's site 6 cites `RemoveFromCombat`'s FOCUS-gated scroll, but the
        // damage-death path is `CombatantKilled`, ON-SCREEN-gated (bring an
        // off-screen death into view) — a distinct gate (binary `sub_74E6F`).
        let pos = self.fighters[target].pos;
        if !self.on_screen_pos(pos) {
            self.redraw_combat_area(8, 3, pos);
        }
        let t = &mut self.fighters[target];
        t.hp_current = 0;
        t.in_combat = false;
        t.delay = 0;
        let downed_party = t.team == Team::Party;
        // `CombatantKilled` (`sub_74E6F`, `ovr033:534`→coab): the removal path the
        // damage caller reaches whenever `in_combat == false` (`ovr014.cs:214`),
        // so it fires for dying/unconscious/dead alike. §26.5 — for a downed
        // party member (`nonTeamMember == false`, modeled as `team == Party`),
        // stamp `Tile_DownPlayer` at its cell unless a `Tile_StinkingCloud`
        // already occupies it (`ovr033.cs:579-590`). Movement-/reach-neutral on a
        // cost-1 floor (the tile constants match a floor's) — fidelity, and it
        // must precede the occupancy repaint, matching coab's order.
        if downed_party && self.map.ground_tile(pos) != TILE_STINKING_CLOUD {
            self.map.set_tile(pos, TILE_DOWN_PLAYER);
        }
        // `CombatantKilled` then zeroes `CombatMap[idx].size` + calls `sub_743E7`
        // (`setup_mapToPlayerIndex_and_playerScreen`): the occupancy repaint
        // happens AT removal, so a corpse's cells free up immediately (a later
        // mover's `CanMove` must see them empty), not at the next position change.
        self.rebuild_occupancy();
    }

    /// `AttackTarget`'s direction bookkeeping (`sub_3F9DB` @`ovr014:19FE-1AD2`,
    /// coab ovr014.cs:913-936, §36.1). Draw-free bookkeeping — the camera scroll
    /// never enters the PRNG stream, and the target-side draw fires only when the
    /// target is on-screen so its off-screen recenter can't run.
    ///
    /// The **target's** facing (`attack_type_nonzero` = the caller's `attackType
    /// != 0`):
    /// - **Branch 1** (@`19FE-1A39`) — `AttacksReceived < 2 && attackType == 0`:
    ///   `var_9 = getTargetDirection(attacker, target)` = `target_direction(target,
    ///   attacker)` = the bearing from the target toward its attacker; store the
    ///   **face-away** `(var_9 + 4) % 8` (@`1A35`, unconditional).
    /// - **Branch 2** (`loc_3FA3B` @`1A3B-1A79`) — else: only touch facing if the
    ///   target is on-screen; then `var_9 = direction`, and if `attackType == 0`
    ///   store the 180° flip `(var_9 + 4) % 8` (@`1A79`).
    /// - **Shared tail** (`loc_3FA7D` @`1A7D-1A9F`): if the target is on-screen,
    ///   `draw_74B3F(false, Normal, var_9, target)` stores `var_9`
    ///   **unconditionally** — the on-screen **draw overwrite**. Branch 1 → the
    ///   target ends up FACING its attacker (`var_9` = bearing, overwrites the
    ///   face-away store: the §35 crack). Branch 2 → `var_9` = the old direction,
    ///   so the flip is restored → net no-op.
    ///
    /// Then the **attacker ALWAYS faces its target** (`loc_3FAA4` @`1AA4-1AD2`):
    /// `draw_74B3F(false, Attack, getTargetDirection(target, attacker), attacker)`
    /// = `target_direction(attacker, target)`, an unconditional store.
    ///
    /// Net (melee `attackType == 0`): 1st attack on-screen → target faces the
    /// attacker; 1st off-screen → faces away; 2nd+ → unchanged; `attackType != 0`
    /// → unchanged. The facing-equality reads (flanking/backstab) therefore see a
    /// target FACING its attacker in melee — which is why the §35 face-away-only
    /// transliterations over-fired.
    fn attack_target_facing(&mut self, target: usize, attacker: usize, attack_type_nonzero: bool) {
        let tgt_on_screen = self.on_screen(target);
        let var_9: u8 = if self.fighters[target].attacks_received < 2 && !attack_type_nonzero {
            // Branch 1 (@1A0B-1A39): var_9 = bearing target→attacker; store
            // face-away unconditionally (the tail draw overwrites it on-screen).
            let bearing = target_direction(self.fighters[target].pos, self.fighters[attacker].pos);
            self.fighters[target].direction = (bearing + 4) % 8; // @1A35
            bearing
        } else {
            // Branch 2 (loc_3FA3B @1A3B): the binary reads `direction` only after
            // the on-screen gate; reading it unconditionally is harmless because
            // `var_9` feeds only the on-screen-gated tail draw.
            let old = self.fighters[target].direction; // @1A55
            if tgt_on_screen && !attack_type_nonzero {
                self.fighters[target].direction = (old + 4) % 8; // @1A79 flip
            }
            old
        };
        // Shared tail (loc_3FA7D @1A7D): the on-screen draw overwrite (@1A9F).
        if tgt_on_screen {
            self.draw_74b3f(target, var_9);
        }
        // loc_3FAA4 @1AA4-1AD2: the attacker always faces its target.
        let face = target_direction(self.fighters[attacker].pos, self.fighters[target].pos);
        self.draw_74b3f(attacker, face);
    }

    /// The flanking heuristic (`AttackTarget01` @`ovr014:16AD-16E9`, coab
    /// ovr014.cs:782-784, §36.4): a swarmed target whose back is turned to this
    /// attacker is hit on its **behind** AC. All three must hold:
    /// - `AttacksReceived > 1` (@`16B5`, `jbe` skips ≤ 1) — the target has taken
    ///   more than one swing since its last move (swarmed this turn);
    /// - `getTargetDirection(target, attacker) == direction` (@`16C9-16D4`) =
    ///   `target_direction(attacker, target) == target.direction` — the attacker's
    ///   bearing toward the target equals the target's facing, i.e. the target
    ///   faces AWAY from the attacker (the attacker is behind it);
    /// - `directionChanges > 4` (@`16E2`, `jbe` skips ≤ 4) — the target has been
    ///   spun enough this turn.
    ///
    /// Guarded by `!CanBackStabTarget` in the binary (the `else` at `loc_3F6AD`);
    /// backstab preempts with `ac_behind − 4` and lands next commit, so here
    /// CanBackStab is treated as false and the gate is vacuously satisfied.
    fn is_flanking(&self, target: usize, attacker: usize) -> bool {
        self.fighters[target].attacks_received > 1
            && target_direction(self.fighters[attacker].pos, self.fighters[target].pos)
                == self.fighters[target].direction
            && self.fighters[target].direction_changes > 4
    }

    /// `CanBackStabTarget(target, attacker)` (`sub_408D7` @`ovr014:28D7-29B9`, coab
    /// ovr014.cs:1433-1457, §36.4). All must hold:
    /// - **class** (@`291C-293C`): `attacker.SkillLevel(Thief) > 0` — our decoded
    ///   `thief_skill_level` (the inlined `field_10F`/`field_117`/`sub_6B3D1`
    ///   dual-class fold, `Player.cs:492`);
    /// - **weapon** (@`293E-2962`): the attacker's `primaryWeapon` (`field_151`)
    ///   is null (bare hands — an unreadied loadout, `weapon_readied == false`) OR
    ///   its type ∈ {Club 7, Dagger 8, BroadSword 35, LongSword 36, ShortSword 37,
    ///   DrowLongSword 97};
    /// - **swarm** (@`2976`, `jbe ≤1`): `target.AttacksReceived > 1`;
    /// - **size** (@`2980-2989`): `(target.field_DE & 0x7F) <= 1` (man-sized);
    /// - **facing** (@`298B-29A3`): `getTargetDirection(target, attacker) ==
    ///   target.direction` = `target_direction(attacker, target) ==
    ///   target.direction` (the target's back is to the attacker — same test as
    ///   flanking).
    ///
    /// Fires `ac_behind − 4` (@`169E-16A5`) and the damage multiplier
    /// `((SkillLevel(Thief) − 1) / 4) + 2` ([`backstab_multiplier`], `sub_3E192`
    /// @`ovr014.cs:96`). Preempts the flanking heuristic (the binary's `else`).
    fn can_backstab(&self, target: usize, attacker: usize) -> bool {
        if self.fighters[attacker].thief_skill_level <= 0 {
            return false;
        }
        // `weapon == null` ⟺ the primary is not readied (a depleted/unreadied
        // loadout → bare hands, or no loadout at all → `weapon_readied == false`).
        let weapon_ok = if self.fighters[attacker].weapon_readied {
            match &self.fighters[attacker].loadout {
                Some(l) => matches!(l.primary_type, 7 | 8 | 35 | 36 | 37 | 97),
                None => false, // readied with no loadout is unreachable, but be safe
            }
        } else {
            true // null primaryWeapon → bare hands → backstab-capable
        };
        weapon_ok
            && self.fighters[target].attacks_received > 1
            && (self.fighters[target].field_de & 0x7F) <= 1
            && target_direction(self.fighters[attacker].pos, self.fighters[target].pos)
                == self.fighters[target].direction
    }

    /// `AttackTarget → AttackTarget01` (`ovr014.cs:904/724`), melee core: for
    /// `attackIdx` counting down from `attack_idx`, drain `AttacksLeft(attackIdx)`
    /// swings — each **one d20** to-hit ([`pc_can_hit_target`]); **on a hit only**,
    /// profile-1 damage ([`roll_damage`]). A hit that kills the target sets
    /// `targetNotInCombat` and stops the remaining swings (no further draws). Sets
    /// `delay = 0` (via `clear_actions`) when the turn's attacks are spent, and
    /// returns `turnComplete`. Backstab/behind AC and the held-slay path are
    /// deferred (raw AC used).
    /// `behind`: `AttackTarget`'s `attackType` arg ≠ 0 (`BehindAttack`,
    /// ovr014.cs:728). The departure opportunity attack passes 1
    /// (ovr014.cs:407); the into-reach and normal turn attacks pass 0. The
    /// `AttacksReceived>1 && facing && directionChanges>4` flanking heuristic
    /// (`ovr014:16BA-16E9`) and backstab's `ac_behind − 4` (`:169E`) are
    /// cited-deferred (M5) — no capture exercises them yet.
    fn attack_target(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        target: usize,
        behind: bool,
        ranged_item: AttackItemRef,
    ) -> bool {
        // AttackTarget (`sub_3F9DB` @`ovr014:19DB`) head: focus the camera on
        // the attacker (`ovr014.cs:908`).
        self.focus = true;
        // §36.1 direction bookkeeping (`sub_3F9DB` @`ovr014:19FE-1AD2`): the
        // target-side facing store + the on-screen draw overwrite, then the
        // attacker ALWAYS faces its target. `behind` here is the caller's
        // `attackType != 0` (departure/behind attacks pass 1) — `attackType != 0`
        // leaves the target's facing untouched. Draw-free (the camera scroll
        // never enters the PRNG stream; the target-side draw fires only on-screen
        // so its recenter can't). The AC-select `behind` decision is derived from
        // this same flag today; the flanking/backstab heuristics diverge it later.
        self.attack_target_facing(target, actor, behind);
        // §19: `AttackTarget` (`sub_3F9DB`, ovr014.cs:939) sets
        // `attacker.actions.target = target` — the attacked (possibly re-picked)
        // combatant becomes the persistent target, so next round's `find_target`
        // keeps it draw-free. Draw-free; only the *held target* carried into later
        // rounds changes (the §18 re-pick correctly writes only a local `chosen`).
        self.fighters[actor].target = Some(target);
        // Site 5 — the ranged missile camera (`ovr014.cs:945` → `draw_missile_attack`,
        // `sub_67AA4`): a bow/thrown shot animates the missile across the board
        // and scrolls the camera toward the target. Draw-free; only its
        // `mapScreenTopLeft` effect is ported ([`draw_missile_camera`]). A plain
        // melee swing (null item) fires no missile.
        if matches!(ranged_item, AttackItemRef::Ammo | AttackItemRef::SelfWeapon) {
            self.draw_missile_camera(actor, target);
        }
        // ...and `sub_3F9DB` fires `sub_40BF1` a SECOND time, with the readied
        // primary itself as the missile, when that primary is a **Sling (0x2F)**
        // or **StaffSling (0x65)** (@`ovr014:1B14-1B4C`). This is the branch that
        // gives a sling its missile: `GetCurrentAttackItem` returns a
        // found-but-NULL item for flags `0x0A` (§34.2), so the item-gated call
        // above does not fire for it. Draw-free like the first — but the camera
        // scroll is exactly the state the §36.1 on-screen facing branch reads, so
        // "draw-free" is not a reason to skip it. The binary dereferences
        // `field_151` here with no null check (UB for bare hands); we gate on the
        // primary actually being readied. No capture carries a sling loadout.
        const ITEM_SLING: u8 = 0x2F;
        const ITEM_STAFF_SLING: u8 = 0x65;
        let sling_primary = self.fighters[actor].weapon_readied
            && matches!(
                self.fighters[actor].loadout.map(|l| l.primary_type),
                Some(ITEM_SLING) | Some(ITEM_STAFF_SLING)
            );
        if sling_primary {
            self.draw_missile_camera(actor, target);
        }
        // `AttackTarget01` sets `actions.field_8 = true` (`ovr014.cs:738`) — the
        // "attacked this round" flag `reclac_attacks`'s write-back gate reads
        // (§34.3); `CalculateInitiative` resets it each round.
        self.fighters[actor].field_8 = true;
        if self.fighters[actor].attack1_left == 0 && self.fighters[actor].attack2_left == 0 {
            self.clear_actions(actor);
            return true;
        }
        // AttackTarget01's AC selection (`sub_3F4EB` @`ovr014:1683-1708`, §36.4).
        // Backstab preempts (the binary's `if CanBackStabTarget` @`1694`, `else`
        // the flanking/behind path @`16AD`): `ac_behind − 4` (@`169E-16A5`).
        // Otherwise the AC byte is record[0x19A + behindIdx] — front @0x19A,
        // behind @0x19B — with behindIdx set when `var_13 != 0` (@`16ED-16F3`):
        // the caller's `attackType != 0` (`behind`) OR the flanking heuristic.
        // Then `target_ac += RangedDefenseBonus` on EVERY path (`ovr014.cs:799`).
        let can_backstab = self.can_backstab(target, actor);
        let base_ac = if can_backstab {
            self.fighters[target].ac_behind as i32 - 4
        } else {
            let behind_attack = behind || self.is_flanking(target, actor);
            (if behind_attack {
                self.fighters[target].ac_behind
            } else {
                self.fighters[target].ac
            }) as i32
        };
        let target_ac = (base_ac + self.ranged_defense_bonus(actor, target)).clamp(0, 255) as u8;
        let hit_bonus = self.fighters[actor].hit_bonus;
        let mut target_gone = false;
        // `bytes_1D900[1]` — the attack-1 swing count (each swing, hit or miss),
        // the ammo the write-back subtracts (§34.6, coab≠binary #16).
        let mut swings_attack1: i32 = 0;

        let start = self.fighters[actor].attack_idx;
        for attack_idx in (1..=start).rev() {
            loop {
                let left = if attack_idx == 1 {
                    self.fighters[actor].attack1_left
                } else {
                    self.fighters[actor].attack2_left
                };
                if left == 0 || target_gone {
                    break;
                }
                if attack_idx == 1 {
                    self.fighters[actor].attack1_left -= 1;
                    swings_attack1 += 1; // bytes_1D900[1] += 1
                } else {
                    self.fighters[actor].attack2_left -= 1;
                }
                self.fighters[actor].attack_idx = attack_idx;

                let th = pc_can_hit_target(rng, target_ac, hit_bonus, 0); // one d20
                if th.hit {
                    // `sub_3E192(idx)` damage cells (§34.6): idx 1 = @0x19E/0x1A0/
                    // 0x1A2 (our decoded profile-1), idx 2 = @0x19F/0x1A1/0x1A3
                    // (`attack2_dice`, all zero in this party).
                    let (dc, ds, db) = if attack_idx == 1 {
                        (
                            self.fighters[actor].dice_count,
                            self.fighters[actor].dice_size,
                            self.fighters[actor].damage_bonus,
                        )
                    } else {
                        self.fighters[actor].attack2_dice
                    };
                    // sub_3E192 @ovr014.cs:94-96: on a backstab, `damage *=
                    // ((SkillLevel(Thief)−1)/4)+2`. CanBackStabTarget's inputs
                    // (facing/AttacksReceived/direction) are stable across the
                    // swing loop, so the AC-time result carries.
                    let backstab = if can_backstab {
                        Some(backstab_multiplier(self.fighters[actor].thief_skill_level))
                    } else {
                        None
                    };
                    let dmg = roll_damage(rng, ds, dc, db, backstab);
                    self.apply_damage(target, dmg.amount);
                    if !self.fighters[target].in_combat {
                        target_gone = true;
                    }
                }
            }
        }

        // Ammo write-back (`sub_3F9DB` @`ovr014:1BB3-1BC7`, coab≠binary #16 —
        // the binary SUBTRACTS `byte_1D901`, coab assigns): `if (item.count > 0)
        // item.count -= swings_attack1`. The decremented count is the FOUND
        // item's own (@item+0x39): a launcher's arrows/quarrels, or a
        // self-launching weapon's own count (our single `ammo` cell serves
        // both). A null item (sling / opportunity attack) skips it.
        if matches!(ranged_item, AttackItemRef::Ammo | AttackItemRef::SelfWeapon) {
            if self.fighters[actor].ammo > 0 {
                self.fighters[actor].ammo -= swings_attack1;
            }
            // Depletion (`:1BC7-`): count hits 0 → the item is lost. For plain
            // ammo (arrows/quarrels) that is a straight `lose_item` — modeled
            // (capture-proven by TRAVIS's quiver). For a SELF-LAUNCHING weapon
            // the lost item IS the primary (`field_151` nulls at once; ours
            // keeps the ready flag but the found-gates treat it as lost, so it
            // degrades exactly like arrows), and a ranged-melee one
            // additionally clone-drops an unreadied copy (`ovr014:1BD4-1C54`) —
            // that drop is unmodeled: the tripwire names the territory. Gated
            // on the edge so the trip fires ONCE at depletion, not on every
            // later re-observation of the already-lost item.
            if self.fighters[actor].ammo <= 0 && !self.fighters[actor].ammo_item_lost {
                self.fighters[actor].ammo = 0;
                self.fighters[actor].ammo_item_lost = true;
                if ranged_item == AttackItemRef::SelfWeapon {
                    self.emit(ActionEvent::StubTripped {
                        combatant_id: actor,
                        stub: "self-weapon-depleted",
                    });
                }
            }
        }

        let complete =
            self.fighters[actor].attack1_left == 0 && self.fighters[actor].attack2_left == 0;
        if complete || !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return true;
        }
        false
    }

    /// `RangedDefenseBonus(target, attacker)` (`sub_3FCED` @`ovr014:1CED`, coab
    /// `ovr014.cs:1012`; doc §34.6): a to-hit AC penalty that grows with distance
    /// for a ranged attacker. `oneThird = (table[type].range − 1) / 3`; the
    /// current `getTargetRange` climbs two bands — `> oneThird` adds +2, again
    /// adds +3 (LongBow: +2 beyond 7, +5 beyond 14). `0` for a non-ranged
    /// attacker (the `else` return). Draw-free.
    fn ranged_defense_bonus(&self, attacker: usize, target: usize) -> i32 {
        if !self.is_weapon_ranged(attacker) {
            return 0;
        }
        let one_third = (self.primary_item(attacker).expect("ranged ⇒ item").range as i32 - 1) / 3;
        let mut range = get_target_range(
            &self.map,
            self.fighters[target].pos,
            self.fighters[attacker].pos,
        ) as i32;
        let mut adj = 0;
        if range > one_third {
            range -= one_third;
            adj += 2;
        }
        if range > one_third {
            adj += 3;
        }
        adj
    }

    /// `RecalcAttacksReceived` (`sub_3F94D` @`ovr014:194D-19D8`, coab
    /// ovr014.cs:887-901) — bump the target's received-attack counter and accumulate
    /// its facing-swing count. Draw-free. Called immediately before `AttackTarget`
    /// on every attack path (AI turn, guard into-reach, sweep per-target).
    ///
    /// `AttacksReceived++` (@`195B`); then `dirDiff = ((getTargetDirection(attacker,
    /// target) − direction) + 8) % 8` (@`1987-1993`) — the bearing from the target
    /// toward its attacker minus the target's current facing — folded `> 4 → 8 −
    /// dirDiff` (@`1996-19A8`, `jbe 4` keeps ≤4 unchanged); then `directionChanges =
    /// (directionChanges + dirDiff) % 8` (@`19C0-19D1`). Mod 8, so values only ever
    /// 0..7 and the accumulator wraps.
    fn recalc_attacks_received(&mut self, target: usize, attacker: usize) {
        self.fighters[target].attacks_received =
            self.fighters[target].attacks_received.saturating_add(1);
        // getTargetDirection(attacker, target) = target_direction(target, attacker)
        // = bearing from the target toward its attacker (§36.2).
        let bearing = target_direction(self.fighters[target].pos, self.fighters[attacker].pos);
        let mut dir_diff =
            (bearing as i32 - self.fighters[target].direction as i32 + 8).rem_euclid(8);
        if dir_diff > 4 {
            dir_diff = 8 - dir_diff;
        }
        self.fighters[target].direction_changes =
            ((self.fighters[target].direction_changes as i32 + dir_diff) % 8) as u8;
    }

    /// `TrySweepAttack` (`ovr014.cs:530`): a melee sweep vs. `HitDice == 0` targets.
    /// **Draw-free and returns `false` for a normal (`hit_dice > 0`) target** — the
    /// only case this slice's fights use. The 0-HD sweep (extra swings per victim)
    /// is deferred with 0-HD monsters flagged.
    fn try_sweep_attack(&mut self, target: usize, actor: usize) -> bool {
        // Guard `target.HitDice == 0` fails for hit_dice > 0 → no sweep, no draws.
        // Tripwire: a 0-HD target means the binary WOULD enter the sweep path
        // (extra swings + their draws) that this stub skips (M5).
        if self.fighters[target].hit_dice == 0 {
            self.emit(ActionEvent::StubTripped {
                combatant_id: actor,
                stub: "0-hd-sweep",
            });
        }
        false
    }

    /// `getGroundInformation(direction, actor)` (`ovr033.cs:433`) for a single-cell
    /// combatant: the destination cell (`pos + delta[direction]`), returning its
    /// ground-tile index (0 for void/OOB) and any *other* occupant (1-based; 0 =
    /// empty).
    fn ground_info_dir(&self, actor: usize, direction: u8) -> (i32, u16) {
        let dest = self.fighters[actor].pos.stepped(direction);
        let ground = self.map.ground_tile(dest) as i32;
        let occ = self.map.occupant(dest);
        let current = (actor + 1) as u16;
        let occ = if occ == current { 0 } else { occ };
        (ground, occ)
    }

    /// `CanMove(baseDirection, dirStep, actor)` (`ovr010.cs:295`): can the actor step
    /// in `(baseDirection + data_2B8[field_15][dirStep-1]) % 8`? Returns
    /// `(can_move, ground_clear)` where `ground_clear` is the void case. Draw-free
    /// (the cloud save at `:341` needs a poison/noxious cloud — none modeled).
    fn can_move(&self, actor: usize, base_dir: u8, dir_step: i32) -> (bool, bool) {
        let f15 = self.fighters[actor].field_15 as usize;
        // §15 bug #2: binary indexes coab row field_15−1 (stride-5 window).
        let offset = DATA_2B8[f15.saturating_sub(1)][(dir_step - 1) as usize];
        let player_dir = ((base_dir as i32 + offset) % 8) as u8;
        let (ground_tile, occ) = self.ground_info_dir(actor, player_dir);

        if ground_tile == 0 {
            return (false, true); // void → groundClear, can't move
        }
        let mc = ground_tile_move_cost(ground_tile);
        if mc == 0xFF {
            return (false, false); // wall
        }
        let cost = if player_dir & 1 != 0 {
            mc as i32 * 3
        } else {
            mc as i32 * 2
        };
        let can = occ == 0 && cost < self.fighters[actor].move_left;
        (can, false)
    }

    /// `sub_3E748(direction, actor)` (`ovr014.cs:252`): step one tile, deduct the
    /// move cost, repaint occupancy, then run opportunity attacks by *guarding*
    /// enemies at the new cell (`move_step_into_attack`). The position updates
    /// unconditionally (coab), but `CanMove` already guaranteed the cost is
    /// affordable.
    fn sub_3e748(&mut self, rng: &mut EngineRng, actor: usize, direction: u8) {
        let old = self.fighters[actor].pos;
        let new = old.stepped(direction);
        if !new.in_bounds() {
            return;
        }
        let base = self.map.move_cost(new) as i32;
        let cost = if direction & 1 != 0 {
            base * 3
        } else {
            base * 2
        };
        if cost > self.fighters[actor].move_left {
            self.fighters[actor].move_left = 0;
        } else {
            self.fighters[actor].move_left -= cost;
        }
        // Site 7 (movement step) — sub_3E748's camera (`ovr014.cs:285-310`). In
        // QuickFight (radius 3): if the destination is off-screen and focus is
        // on, first scroll to the OLD cell (`redrawCombatArea(8, 2, oldPos)`,
        // @294) using the pre-move window; then, after the pos write, scroll to
        // the NEW cell (`redrawCombatArea(8, 3, newPos)`, @309) if focus.
        if !self.on_screen_pos(new) && self.focus {
            self.redraw_combat_area(8, 2, old);
        }
        self.fighters[actor].pos = new;
        self.rebuild_occupancy();
        if self.focus {
            self.redraw_combat_area(8, 3, new);
        }
        self.emit(ActionEvent::Move {
            combatant_id: actor,
            from_x: old.x,
            from_y: old.y,
            to_x: new.x,
            to_y: new.y,
            cost,
        });
        // sub_3E748 @`ovr014:0902-090F`: the mover's own swarm state zeroes after
        // the pos write — `AttacksReceived = 0` (@`0902`) and `directionChanges = 0`
        // (@`090F`). Swarm/facing bookkeeping is per-position.
        self.fighters[actor].attacks_received = 0;
        self.fighters[actor].direction_changes = 0;
        self.move_step_into_attack(rng, actor);
        if !self.fighters[actor].in_combat {
            self.fighters[actor].move_left = 0;
        }
    }

    /// `move_step_into_attack(mover)` (`ovr014.cs:226`): every adjacent enemy that
    /// is **guarding** attacks the mover entering its reach (`AttackTarget(null,0)`).
    /// In a fresh melee no one guards, so this is draw-free; it becomes draw-bearing
    /// only once a combatant has fallen back to guard.
    fn move_step_into_attack(&mut self, rng: &mut EngineRng, mover: usize) {
        if !self.fighters[mover].in_combat {
            return;
        }
        let near = self.build_near(mover, 1, false);
        for n in near {
            let att = n.idx;
            if self.fighters[att].guarding {
                // Site 7 (guard fire) — `move_step_into_attack` scrolls to the
                // entering mover before the swing: `redrawCombatArea(8, 2,
                // target.pos)` (`ovr014.cs:239`).
                let mp = self.fighters[mover].pos;
                self.redraw_combat_area(8, 2, mp);
                self.fighters[att].guarding = false;
                self.recalc_attacks_received(mover, att);
                // AttackTarget(null,0) — the guard's into-reach swing carries no
                // ranged item (ovr014.cs:245).
                self.attack_target(rng, att, mover, false, AttackItemRef::None);
            }
        }
    }

    /// `move_step_away_attack(direction, mover)` (`ovr014.cs:326`): every enemy the
    /// mover **leaves** melee adjacency with (adjacent now, not adjacent at the
    /// destination) gets a free `AttackTarget(null,1)`. In a clean open-ground
    /// approach the mover isn't adjacent to anyone, so this is draw-free; it fires
    /// once melee is joined and a combatant steps out.
    fn move_step_away_attack(&mut self, rng: &mut EngineRng, mover: usize, direction: u8) {
        let origin = self.build_near(mover, 1, false);
        if origin.is_empty() {
            return;
        }
        // Peek the destination's adjacent enemies (move, measure, move back).
        let orig_pos = self.fighters[mover].pos;
        self.fighters[mover].pos = orig_pos.stepped(direction);
        self.rebuild_occupancy();
        let dest = self.build_near(mover, 1, false);
        self.fighters[mover].pos = orig_pos;
        self.rebuild_occupancy();
        if !self.fighters[mover].in_combat {
            return;
        }
        let dest_ids: std::collections::HashSet<usize> = dest.iter().map(|n| n.idx).collect();
        let departed: Vec<usize> = origin
            .iter()
            .map(|n| n.idx)
            .filter(|i| !dest_ids.contains(i))
            .collect();
        for att in departed {
            // `sub_3E954` re-tests the MOVER's `in_combat` at the top of every
            // candidate iteration (`ovr014:0AD2-0ADD`): dead → `jmp loc_3ECEF`,
            // the loop continuation, skipping the swing AND the focus set. A
            // mover dropped by an earlier departure swing therefore takes no
            // further swings. Every remaining iteration skips identically
            // (nothing revives mid-loop), so `break` is draw- and
            // state-equivalent to the binary's skip-to-end scan.
            if !self.fighters[mover].in_combat {
                break;
            }
            // Site 7 (departure attack) — `sub_3E954` @`ovr014:0AE0-0AE5` sets
            // `byte_1D90F = 1` and `byte_1D910 = 1` (`focusCombatAreaOnPlayer`)
            // at the TOP of each candidate iteration: after the loop's re-test of
            // the MOVER's `in_combat` (@`0AD2-0ADD`, above), but BEFORE the
            // candidate is even fetched (@`0AF5-0B0B`) and before every per-candidate filter
            // (`sub_66BDB` @`0B14`, `sub_3F143` @`0B2D`, the two `find_affect`s).
            // So a candidate that is later skipped STILL leaves focus on — which
            // is why this is not folded into the `continue` below. The camera is
            // then live for the step that follows (`sub_3E748`'s focus-gated
            // scrolls) even for an off-screen monster mover.
            self.focus = true;
            if !self.fighters[att].in_combat || !self.can_see_target(mover) {
                continue;
            }
            // The tmpDir visibility scan (ovr014.cs:374-380): an attacker that
            // hasn't acted (delay>0) or hasn't been attacked qualifies immediately.
            let base = self.fighters[att].direction as i32 + 6;
            let qualifies = (base..=base + 4).any(|tmp| {
                self.fighters[att].delay > 0
                    || self.fighters[att].attacks_received == 0
                    || can_see_combatant(
                        (tmp % 8) as u8,
                        self.fighters[mover].pos,
                        self.fighters[att].pos,
                    )
            });
            if qualifies {
                let idx = if self.fighters[att].attack1_left > 0 {
                    1
                } else if self.fighters[att].attack2_left > 0 {
                    2
                } else {
                    1
                };
                self.fighters[att].attack_idx = idx;
                if idx == 1 && self.fighters[att].attack1_left == 0 {
                    self.fighters[att].attack1_left = 1;
                } else if idx == 2 && self.fighters[att].attack2_left == 0 {
                    self.fighters[att].attack2_left = 1;
                }
                // AttackTarget(null, 1, mover, att) — ovr014.cs:407: the
                // departure swing is ALWAYS a BehindAttack (the mover has
                // turned its back), so it hits `ac_behind`@0x19B. This is the
                // draw-2707 layer: same d20, rear AC — the bar-rout fleer is
                // hit where front-AC math missed.
                //
                // §31 bug #14: the departure attack does NOT retarget the
                // attacker — `sub_3E954` saves `actions.target` before the
                // `AttackTarget` call (`ovr014:0C83-0C8E`) and restores it
                // after (`:0CB3-0CC5`; coab's `backupTarget`, ovr014.cs:405/
                // 410), so `attack_target`'s §19 write-back is transient
                // here. Without the restore the attacker permanently switches
                // to the fleer it punished, and its held target silently
                // diverges for the rest of the fight.
                let backup_target = self.fighters[att].target;
                // AttackTarget(null,1) — the departure opportunity attack carries
                // no ranged item (ovr014.cs:407).
                self.attack_target(rng, att, mover, true, AttackItemRef::None);
                self.fighters[att].target = backup_target;
            }
        }
    }

    /// `moralFailureEscape(actor)` (`ovr010.cs:369`, `sub_359D1`) — one **approach**
    /// (or flee) step toward the target. For an **NPC** advancing, the morale gate
    /// draws **one d100** (`:387`); a **PC** short-circuits it (0 draws). Then a
    /// `CanMove` retry loop picks a step direction from [`DATA_2B8`], the mover
    /// faces it (`draw_74B3F` sets `direction`), leaving-adjacency enemies attack
    /// (`move_step_away_attack`), and the step lands (`sub_3E748`). The flee branch
    /// (`moral_failure`) draws the `:400` d2; only the non-flee approach is
    /// exercised by the parity fights.
    fn moral_failure_escape(
        &mut self,
        rng: &mut EngineRng,
        actor: usize,
        b1ab18: &mut i32,
        b1ab19: &mut i32,
    ) {
        if !(self.fighters[actor].move_left / 2 > 0 && self.fighters[actor].delay > 0) {
            self.try_guarding(actor);
            return;
        }

        // The morale-advance gate (ovr010.cs:386-388). C# `||` short-circuit:
        // a PC (control<NPC_Base) makes the FIRST operand true → NO d100; an NPC
        // (control>=NPC_Base) evaluates operand C → draws the d100. `morale_roll`
        // stays 0 when no d100 is drawn.
        let mut morale_roll: u16 = 0;
        let advance = if !self.fighters[actor].npc {
            true
        } else {
            morale_roll = roll_dice(rng, 100, 1);
            self.enemy_health_pct <= morale_roll as i32 + self.monster_morale
                || self.fighters[actor].team == Team::Monster
        };
        self.emit(ActionEvent::Morale {
            combatant_id: actor,
            monster_morale: self.monster_morale,
            enemy_hp_pct: self.enemy_health_pct,
            roll: morale_roll,
            failed: self.fighters[actor].moral_failure,
        });

        if !advance {
            self.try_guarding(actor);
            return;
        }

        // §15 bug #4 — the Magic-User hold (`sub_359D1` @`ovr010:0AA3`, `loc_35AA3`,
        // the shared post-advance block both PCs and advancing NPCs reach). A
        // non-fleeing pure Magic-User (`class == 5`) with a null `field_159` does
        // **not** advance — it `jmp loc_35D9E` (guard). This is what pins PHILIPPE
        // to his corner the whole capture. The `sub_35DB1` caller then exits its
        // loop draw-free (`find_target` re-draws nothing once a target is held),
        // exactly like the binary.
        if !self.fighters[actor].moral_failure
            && self.fighters[actor].field_159_null
            && self.fighters[actor].class == 5
        {
            self.try_guarding(actor);
            return;
        }

        let dir = if !self.fighters[actor].moral_failure {
            let tp = self.fighters[self.fighters[actor].target.unwrap()].pos;
            target_direction(self.fighters[actor].pos, tp)
        } else {
            // Flee direction (ovr010.cs:400-408) — draws the d2, then a fixed
            // heading from mapDirection. Only reached when moral_failure is set.
            self.fighters[actor].field_15 = roll_dice(rng, 2, 1) as u8;
            let md = self.map_direction as i32;
            let mut d = md - (((md + 2) % 4) / 2) + 8;
            if self.fighters[actor].team == Team::Party {
                d += 4;
            }
            (d % 8) as u8
        };

        // CanMove retry loop (ovr010.cs:415-428): find the first dir_step whose
        // DATA_2B8-offset direction is walkable. flee_battle only in the flee case.
        let mut dir_step = 1i32;
        let mut var_5 = false;
        while dir_step < 6 && !var_5 {
            let (can, ground_clear) = self.can_move(actor, dir, dir_step);
            if can {
                break;
            }
            if self.fighters[actor].moral_failure && ground_clear {
                var_5 = true;
                self.flee_battle(rng, actor);
            } else {
                dir_step += 1;
            }
        }

        if var_5 {
            self.fighters[actor].move_left = 0;
            self.fighters[actor].moral_failure = false;
            self.clear_actions(actor);
            return;
        }

        let f15 = self.fighters[actor].field_15 as usize;
        // §15 bug #2: binary indexes coab row field_15−1 (stride-5 window).
        let offset = DATA_2B8[f15.saturating_sub(1)][(dir_step.min(6) - 1) as usize];
        let var_2 = (offset + dir as i32).rem_euclid(8);

        // Anti-oscillation (ovr010.cs:440-460): a 180° reversal or a failed step
        // rotates field_15 and (after 2) retargets — find_target here DRAWS.
        if dir_step == 6 || (var_2 + 4) % 8 == *b1ab18 {
            *b1ab19 += 1;
            self.fighters[actor].field_15 = (self.fighters[actor].field_15 % 6) + 1;
            if *b1ab19 > 1 {
                self.fighters[actor].target = None;
                if *b1ab19 > 2 {
                    self.fighters[actor].move_left = 0;
                    var_5 = true;
                } else if !self.find_target(rng, actor, false, 1, 0xff) {
                    var_5 = true;
                    self.try_guarding(actor);
                }
            }
        }

        if dir_step < 6 {
            *b1ab18 = var_2;
        } else {
            var_5 = true;
        }

        if var_5 {
            return;
        }

        // Site 7 (approach/flee step) — the camera follows the mover before it
        // steps (`ovr010.cs:474`): `focus = (byte_1D90E || PlayerOnScreen(mover)
        // || team == Ours)`, and `byte_1D90E` is provably false on this path
        // (reset @`ovr010:561`, only set true once a target is reached). Then
        // `draw_74B3F(false, Normal, var_2, mover)` (@476) recenters an
        // off-screen mover and sets `actions.direction = var_2` (the step
        // heading — the store our engine already carried).
        self.focus = self.on_screen(actor) || self.fighters[actor].team == Team::Party;
        self.draw_74b3f(actor, var_2 as u8);
        self.move_step_away_attack(rng, actor, var_2 as u8);
        if !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return;
        }
        if self.fighters[actor].move_left > 0 {
            self.sub_3e748(rng, actor, self.fighters[actor].direction);
        }
        // in_poison_cloud — draw-free (no cloud).
    }

    /// `flee_battle` (`ovr014.cs:426`): the escape check, drawing a `d2` tiebreak
    /// (`:443`) only when the fastest opponent exactly matches the fleer's speed.
    /// Reached only from the flee path; removes the fleer on success (**Got Away**).
    fn flee_battle(&mut self, rng: &mut EngineRng, actor: usize) {
        let gets_away = if self.build_near(actor, 0xff, false).is_empty() {
            true
        } else {
            let var_4 = calc_moves(self.fighters[actor].movement) / 2;
            let var_3 = self.max_opposition_moves(actor);
            if var_3 < var_4 {
                true
            } else {
                var_3 == var_4 && roll_dice(rng, 2, 1) == 1 // ovr014.cs:443
            }
        };
        if gets_away {
            // "Got Away" (`ovr014:0D90`): `RemoveFromCombat(..., Status.running=3,
            // ...)` — the fleer leaves with `health_status = Running`; hp is NOT
            // zeroed (the running special-case) and its footprint frees immediately
            // (`sub_743E7`, visible to every later `CanMove` this same round). No
            // downed-tile stamp.
            self.remove_from_combat(actor, HealthStatus::Running);
        }
        // `:0DBD func_end` — clear_actions unconditionally (idempotent after the
        // removal's own clear_actions on the Got-Away path).
        self.clear_actions(actor);
    }

    /// `bandage(applyBandage)` (`ovr025:335F`, coab ovr025.cs:1628) — scan the
    /// roster (`TeamList` order) for a bandageable ally: `nonTeamMember == false`
    /// AND `combat_team == Ours` AND `health_status == dying`. Returns whether any
    /// exists. When `apply_bandage`, the **first** such member is bandaged —
    /// `dying → unconscious`, `bleeding = 0` — and no further member is bandaged
    /// (one per call); the scan continues only to keep reporting `someoneBleeding`.
    ///
    /// `nonTeamMember == false && combat_team == Ours` is modeled as
    /// `team == Party` (§26 cited simplification: allied non-team NPCs on the
    /// party team are out of this slice's scope). Monsters are never bandaged.
    /// Draw-free (the "is bandaged" status string, `ovr025:33D6`, is display-only).
    fn bandage(&mut self, apply_bandage: bool) -> bool {
        let mut someone_bleeding = false;
        let mut apply = apply_bandage;
        for f in &mut self.fighters {
            if f.team == Team::Party && f.health_status == HealthStatus::Dying {
                someone_bleeding = true;
                if apply {
                    f.health_status = HealthStatus::Unconscious;
                    f.bleeding = 0;
                    apply = false; // one bandage per call (ovr025:33E5)
                }
            }
        }
        someone_bleeding
    }

    /// `sub_35DB1(actor)` (`ovr010.cs:511`) — the move-then-attack loop. Approaches
    /// the target one step per iteration (each NPC step drawing the morale d100)
    /// until adjacent, then attacks (`AttackTarget01`'s d20s + damage). Returns
    /// `delayed == false` (the turn is spent). The 20-iteration `counter` cap
    /// guarantees termination.
    fn sub_35db1(&mut self, rng: &mut EngineRng, actor: usize) -> bool {
        let mut b1ab18 = 8i32;
        let mut b1ab19 = 0i32;
        // CheckAffectsEffect(Type_14) (`ovr010:0DDB`) — draw-free, no affects
        // modeled. Then the bandage turn (§26.2, `ovr010:0DE3-0DFF`): a Party
        // actor whose team has a dying ally spends its whole turn bandaging —
        // `bandage(true)` zeroes `actions.delay`, so `delayed` starts false and
        // the move-attack loop below never runs (no movement, no attack, no draws
        // beyond the turn head the caller already rolled). Draw-free itself.
        if self.fighters[actor].team == Team::Party && self.bandage(true) {
            self.fighters[actor].delay = 0; // ovr010:0DFF — actions.delay = 0
        }
        let mut counter = 0;
        let mut stop = false;
        let mut delayed = self.fighters[actor].delay != 0;

        while !stop && delayed {
            if self.fighters[actor].moral_failure {
                while self.fighters[actor].move_left > 0
                    && self.fighters[actor].delay > 0
                    && self.fighters[actor].delay < 20
                {
                    self.moral_failure_escape(rng, actor, &mut b1ab18, &mut b1ab19);
                }
            }

            let d = self.fighters[actor].delay;
            if d == 0 || d == 20 {
                delayed = false;
            }

            if !stop && delayed {
                counter += 1;
                if counter > 20 {
                    stop = true;
                    delayed = false;
                    self.try_guarding(actor);
                }

                if !stop {
                    let mut reachable = false;
                    // Attack range (`ovr010.cs:562-572`, doc §34.4): the readied
                    // weapon's table range less one, sanitized. LongBow (22) →
                    // 21, ShortBow (16) → 15; a melee combatant (no loadout)
                    // stays range 1. The held-target reach test and every
                    // `BuildNearTargets` below use THIS range, so a bowman's near
                    // list spans the room.
                    let range = self.weapon_range(actor);

                    // The binary's `player01` local (ovr010:0F12-0F46): load
                    // actions.target, then null the LOCAL if the target is out
                    // of combat or on the PARTY team — `cmp combat_team, 0` is
                    // an immediate-0 compare (Team::Party), NOT the attacker's
                    // team, and actions.target itself is NOT cleared. A monster
                    // therefore never keeps a held party target here: it always
                    // falls through to the near-list re-pick.
                    let mut chosen: Option<usize> = self.fighters[actor].target;
                    if let Some(t) = chosen {
                        let tf = &self.fighters[t];
                        if !tf.in_combat || tf.team == Team::Party {
                            chosen = None;
                        }
                    }

                    // Reachability probe (ovr010.cs:583-598) — draw-free.
                    if let Some(t) = chosen {
                        if self.can_see_target(t) {
                            let ap = self.fighters[actor].pos;
                            let tp = self.fighters[t].pos;
                            if let Some(steps) = can_reach(&self.map, ap, tp, range, false) {
                                if steps as i32 / 2 <= range {
                                    reachable = true;
                                }
                            }
                        }
                    }

                    if !reachable {
                        let near = self.build_near(actor, range, false);
                        if near.is_empty() {
                            // No adjacent enemy → approach one step toward the target.
                            if self.find_target(rng, actor, false, 0, 0xff) {
                                self.moral_failure_escape(rng, actor, &mut b1ab18, &mut b1ab19);
                            } else {
                                stop = true;
                                self.try_guarding(actor);
                            }
                        } else {
                            // An adjacent enemy exists → re-pick among them (:618).
                            // Binary loc_36036: the pick lands in the LOCAL
                            // `player01` only — actions.target is not written.
                            let roll = roll_dice(rng, near.len() as u16, 1);
                            let picked = near[(roll - 1) as usize].idx;
                            chosen = Some(picked);
                            // §34.4 cornered re-pick: a still-ranged (non-ranged-
                            // melee) attacker with an adjacent enemy unreadies via
                            // items_selection and STOPS — no attack this turn
                            // (ovr010.cs:622-628). Step-7 usually unreadied
                            // already (so is_weapon_ranged is false here and the
                            // else-if fires the punch); this covers the case a
                            // bowman is still readied at the near-pick.
                            if self.is_weapon_ranged(actor)
                                && !self.is_weapon_ranged_melee(actor)
                                && !self.build_near(actor, 1, false).is_empty()
                            {
                                self.ai_items_selection(actor);
                                stop = true;
                            } else {
                                let tp = self.fighters[picked].pos;
                                if get_target_range(&self.map, tp, self.fighters[actor].pos) == 1
                                    || self.can_see_target(picked)
                                {
                                    reachable = true;
                                }
                            }
                        }
                    }

                    if reachable {
                        let t = chosen.unwrap();
                        // Site 3 — the AI pre-attack camera (`ovr010.cs:637-639`,
                        // gated on `byte_1D90E == reachable`): scroll one step
                        // from the actor toward the target, radius 2. Fires before
                        // both TrySweepAttack and RecalcAttacksReceived.
                        let cam_dir =
                            target_direction(self.fighters[actor].pos, self.fighters[t].pos);
                        let ap = self.fighters[actor].pos;
                        self.redraw_combat_area(cam_dir, 2, ap);
                        if self.try_sweep_attack(t, actor) {
                            stop = true;
                            self.clear_actions(actor);
                        } else {
                            self.recalc_attacks_received(t, actor);
                            // §34.4 attack execution: for a ranged attacker,
                            // resolve the ammo item (`GetCurrentAttackItem`); a
                            // ranged-melee weapon at reach 1 passes null (thrown
                            // as melee, `ovr010.cs:655-664`). coab≠binary #17: the
                            // `byte_1D90E = GetCurrentAttackItem` re-assign at
                            // `ovr010:1176` is dead (verified) — only the item is
                            // used, the attack proceeds unconditionally.
                            let ranged_item = if self.is_weapon_ranged(actor) {
                                let mut item = self.get_current_attack_item(actor).item;
                                if self.is_weapon_ranged_melee(actor)
                                    && get_target_range(
                                        &self.map,
                                        self.fighters[t].pos,
                                        self.fighters[actor].pos,
                                    ) == 1
                                {
                                    item = AttackItemRef::None;
                                }
                                item
                            } else {
                                AttackItemRef::None
                            };
                            stop = self.attack_target(rng, actor, t, false, ranged_item);
                            if stop {
                                delayed = false;
                            } else if !self.fighters[t].in_combat {
                                stop = true;
                            }
                        }
                    }
                }
            }
        }

        !delayed
    }

    /// `PlayerQuickFight(actor)` (`ovr010.cs:8`) — the whole melee AI turn, in draw
    /// order (study §4.1): the `field_15` mode-gate, `FleeCheck_001` (draw-free),
    /// the two normal-area behavior-guard d7s (`sub_354AA:192` + `sub_3560B:248`),
    /// then the `find_target` pick and the `sub_35DB1` move-attack loop. Spell/
    /// wand/turn-undead **effects** are stubbed; their **guards and draws** are
    /// faithful. Every draw flows through `rng`, so an attached `RngSink` sees the
    /// exact stream (D9).
    pub fn melee_ai_turn(&mut self, rng: &mut EngineRng, actor: usize) {
        // process_input_in_monsters_turn — headless, draw-free, returns false.
        if !self.fighters[actor].in_combat {
            self.clear_actions(actor);
            return;
        }

        // 1. field_15 mode-gate (ovr010.cs:20-36).
        self.fighters[actor].field_15 = field_15_mode_gate(rng, self.fighters[actor].field_15);

        // 2. FleeCheck_001 (ovr010.cs:40) — draw-free.
        let surrendered = self.flee_check(actor);
        if surrendered {
            return;
        }

        // 3. sub_354AA wand scan (ovr010.cs:54) — the normal-area d7.
        if self.wand_scan_d7(rng, actor) {
            self.clear_actions(actor);
            return;
        }

        // 4. queued spell (spell_id>0) — none for a fighter.
        // 5. turn_undead — non-cleric, short-circuit, draw-free.

        // 6. sub_3560B (ovr010.cs:74) — the UNCONDITIONAL memorized-spell d7 (:248).
        let _spell_priority = roll_dice(rng, 7, 1);
        // (spells_count==0 → the inner roll_dice(spells_count,1) loop never runs.)
        // Tripwire: the binary's inner selection loop draws (3×
        // `roll_dice(spells_count,1)` per priority pass + the cast) only when ALL
        // its gates pass (`ovr010:0679-06A7`): memorized slots exist, the caster
        // is NPC-controlled (`control_morale >= 0x80`) **or** `AutoPCsCastMagic`
        // is on, and an enemy is live (`friends_count`/`foe_count`,
        // ovr010.cs:255). A PC with magic OFF draws NOTHING here —
        // capture-proven: bar-fists-2 closes 3811/3811 with two memorized slots
        // and zero spell draws (doc §33) — so the wire mirrors the binary's
        // draw condition, not mere possession.
        let live_opponent = {
            let (party, monsters) = self.live_counts();
            match self.fighters[actor].team {
                Team::Party => monsters > 0,
                Team::Monster => party > 0,
            }
        };
        if self.fighters[actor].memorized_spells > 0
            && (self.fighters[actor].npc || self.auto_pcs_cast_magic)
            && live_opponent
        {
            self.emit(ActionEvent::StubTripped {
                combatant_id: actor,
                stub: "memorized-spells",
            });
        }

        // 7. AI_items_selection (ovr010.cs:79) — the cornered weapon swap
        // (§34.5): a bowman with an adjacent enemy unreadies to bare hands here,
        // at the TOP of the turn (before find_target / the move-attack loop), so
        // the swing below is a punch; the room clearing re-readies the bow.
        // Draw-free; inert without a loadout.
        self.ai_items_selection(actor);
        // 8. process_input again — draw-free.

        // 9. the target/move-attack loop (ovr010.cs:82-95).
        loop {
            let found = self.find_target(rng, actor, false, 1, 0xff);
            if found && self.fighters[actor].delay > 0 && self.fighters[actor].in_combat {
                if self.sub_35db1(rng, actor) {
                    break;
                }
            } else {
                self.try_guarding(actor);
                break;
            }
        }

        // The turn's `ai` action event (§9): its resolved mode + target.
        self.emit(ActionEvent::Ai {
            combatant_id: actor,
            field_15: self.fighters[actor].field_15,
            target_id: self.fighters[actor].target.map(|t| t as i64).unwrap_or(-1),
        });
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

    // === the ranged predicates + weapon table (M5 armed slice, doc §34.2/34.3) ===

    /// `is_weapon_ranged` (`offset_above_1` @`ovr025:2FE4`, coab `ovr025.cs:1578`):
    /// the readied primary weapon (`field_151`) is non-null AND its table range
    /// is `> 1` (`jbe` → false on `<= 1`). Without a loadout / item table a
    /// combatant is never ranged — today's melee behaviour.
    fn is_weapon_ranged(&self, actor: usize) -> bool {
        let f = &self.fighters[actor];
        match (f.weapon_readied, f.loadout, self.item_data.as_ref()) {
            (true, Some(l), Some(items)) => items.get(l.primary_type).range as i32 > 1,
            _ => false,
        }
    }

    /// The ranged-melee FLAG test for a candidate weapon TYPE (readied or not):
    /// its table flags carry both `flag_10 | melee` — a thrown weapon also
    /// usable in hand (HandAxe 0x14 yes; Dart 0x1A no). The type-level half of
    /// `offset_equals_20`, shared by [`Self::is_weapon_ranged_melee`] (the
    /// readied-actor predicate) and `ai_items_selection` (which evaluates the
    /// candidate before it is readied).
    fn candidate_ranged_melee(&self, item_type: u8) -> bool {
        const RANGED_MELEE: u8 =
            gbx_formats::items::flags::FLAG_10 | gbx_formats::items::flags::MELEE;
        match self.item_data.as_ref() {
            Some(items) => (items.get(item_type).flags & RANGED_MELEE) == RANGED_MELEE,
            None => false,
        }
    }

    /// `is_weapon_ranged_melee` (`offset_equals_20` @`ovr025:3027`, coab
    /// `ovr025.cs:1570`): [`is_weapon_ranged`] AND [`Self::candidate_ranged_melee`]
    /// on the readied primary. None of armed-bar's bows qualify.
    fn is_weapon_ranged_melee(&self, actor: usize) -> bool {
        if !self.is_weapon_ranged(actor) {
            return false;
        }
        let l = self.fighters[actor].loadout.expect("ranged ⇒ loadout");
        self.candidate_ranged_melee(l.primary_type)
    }

    /// The readied primary weapon's [`gbx_formats::items::ItemData`], or `None`
    /// when no loadout weapon is readied. A convenience over the `(loadout,
    /// item_data)` pair the predicates share.
    fn primary_item(&self, actor: usize) -> Option<gbx_formats::items::ItemData> {
        let f = &self.fighters[actor];
        match (f.weapon_readied, f.loadout, self.item_data.as_ref()) {
            (true, Some(l), Some(items)) => Some(items.get(l.primary_type)),
            _ => None,
        }
    }

    /// `GetCurrentAttackItem(out item, player)` (`sub_6906C` @`ovr025:306C`, coab
    /// `ovr025.cs:1590`): from the readied primary's flags, resolve which item
    /// the attack draws (arrows/quarrels slot for a launcher `flag_08`, the
    /// weapon itself for a self-launcher `flag_10`), and whether one was
    /// "found" (`item != null` OR `flags == flag_08|flag_02` == 0x0A — a
    /// Sling/StaffSling finds a null item and still shoots, no ammo consumed).
    fn get_current_attack_item(&self, actor: usize) -> CurrentAttackItem {
        let Some(item) = self.primary_item(actor) else {
            // primaryWeapon == null → item stays null, flags None → not found.
            return CurrentAttackItem {
                found: false,
                item: AttackItemRef::None,
            };
        };
        let flags = item.flags;
        let f = &self.fighters[actor];
        let mut found_item = AttackItemRef::None;
        // A depleted self-launching weapon is LOST in the binary (`field_151`
        // nulls at depletion), so it no longer finds itself — same degradation
        // as a depleted ammo slot.
        if flags & gbx_formats::items::flags::FLAG_10 != 0 && !f.ammo_item_lost {
            found_item = AttackItemRef::SelfWeapon;
        }
        if flags & gbx_formats::items::flags::FLAG_08 != 0 {
            // The arrows / quarrels ammo slot — null once depleted (`lose_item`).
            let ammo_slot = if f.ammo_item_lost {
                AttackItemRef::None
            } else {
                AttackItemRef::Ammo
            };
            if flags & gbx_formats::items::flags::ARROWS != 0 {
                found_item = ammo_slot;
            }
            if flags & gbx_formats::items::flags::QUARRELS != 0 {
                found_item = ammo_slot;
            }
        }
        // item_found = (found_item != null) || flags == (flag_08 | flag_02).
        let found = !matches!(found_item, AttackItemRef::None)
            || flags == (gbx_formats::items::flags::FLAG_08 | gbx_formats::items::flags::FLAG_02);
        CurrentAttackItem {
            found,
            item: found_item,
        }
    }

    /// The ammo `count` of the `GetCurrentAttackItem` result (item+0x39), or
    /// `None` when the item is null (a Sling's found-but-null item — no ammo
    /// cap). A launcher counts the combatant's `ammo`; a self-launching weapon's
    /// own count is unmodeled (armed-bar has none) and treated as `ammo`.
    fn attack_item_count(&self, actor: usize, item: &CurrentAttackItem) -> Option<i32> {
        match item.item {
            AttackItemRef::None => None,
            AttackItemRef::Ammo | AttackItemRef::SelfWeapon => Some(self.fighters[actor].ammo),
        }
    }

    /// The AI turn's attack range (`ovr010.cs:562-572`, doc §34.4): `range =
    /// table[primary.type].range - 1` when a primary weapon is readied
    /// (`field_151` non-null), else 1; sanitize to 1. The binary sanitizes the
    /// BYTE values `{0, 0xFF}` — table range 1 and table range 0 (whose `0 − 1`
    /// wraps to `0xFF`) — which in i32 space are exactly `r == 0` and `r == -1`;
    /// an i32 `r == 0xFF` arm would instead catch table range 255, which the
    /// binary leaves at 254 (review finding #4). LongBow (22) → 21, ShortBow
    /// (16) → 15.
    fn weapon_range(&self, actor: usize) -> i32 {
        match self.primary_item(actor) {
            Some(it) => {
                let r = it.range as i32 - 1;
                if r == 0 || r == -1 {
                    1
                } else {
                    r
                }
            }
            None => 1,
        }
    }

    /// `reclac_attacks(player)` (`sub_3EDD4` @`ovr014:0DD4`, coab `ovr014.cs:462`;
    /// doc §34.3). Sets `attack1_left` for the round: `attacksCount` half-actions
    /// for melee, or — with a readied ranged weapon whose ammo is found —
    /// `max(2, table[type].numberAttacks)` (LongBow 4 → 2 shots/round), capped by
    /// remaining ammo. The write-back is gated so a mid-turn recompute cannot
    /// inflate the count. Draw-free; called by `CalculateInitiative` and the
    /// cornered weapon-selection AI.
    fn reclac_attacks(&mut self, actor: usize) {
        let orig = self.fighters[actor].attack1_left as i32;
        // rec[0x19C] = rec[0x11C] (attack1_left := attacksCount).
        self.fighters[actor].attack1_left = self.fighters[actor].attacks_count;

        let ranged = self.is_weapon_ranged(actor);
        let item = self.get_current_attack_item(actor);
        let found_ranged = ranged && item.found;

        let half = if found_ranged {
            let natk = self
                .primary_item(actor)
                .map(|it| it.number_attacks as i32)
                .unwrap_or(0);
            natk.max(2)
        } else {
            self.fighters[actor].attack1_left as i32
        };

        let mut attacks = this_round_action_count(half, self.combat_round);

        // Ammo cap (only for a found ranged item that is non-null — a Sling's
        // null item is skipped): cap = max(1, count); if cap < attacks &&
        // count > 0 → attacks = cap.
        if found_ranged {
            if let Some(count) = self.attack_item_count(actor, &item) {
                let cap = count.max(1);
                if cap < attacks && count > 0 {
                    attacks = cap;
                }
            }
        }

        // Write-back gate (`ovr014:0EBE-0EFC`, coab `ovr014.cs:508`): !field_8
        // || attacks < orig || (field_8 && attacks < orig*2 && !foundRanged).
        // The third clause tests `var_5` = **foundRanged** (`:0EF6`, ranged AND
        // the attack item found), not mere is_weapon_ranged — the two differ for
        // a readied launcher whose ammo item is gone (audit fix).
        let field_8 = self.fighters[actor].field_8;
        if !field_8 || attacks < orig || (field_8 && attacks < orig * 2 && !found_ranged) {
            self.fighters[actor].attack1_left = attacks as u8;
        }
    }

    /// `CalcItemPowerRating(item, player)` (`sub_36535` @`ovr010:1535`, coab
    /// `ovr010.cs:817`; doc §34.5) for the loadout's primary weapon type:
    /// `rating = dsN*dcN + item.plus*8(if>0) + bonusN*2(if>0) +
    /// (flag_08 ? (natk−1)*2 : 0) + (hands ≤ 1 ? 3 : 0)`. The loadout carries no
    /// magic plus (mundane weapons → `plus = 0`); the cursed / affect / hands+used
    /// zeroing branches are cited-deferred (a single non-cursed weapon). LongBow:
    /// `6 + 6 = 12`.
    fn calc_item_power_rating(&self, item_type: u8) -> i32 {
        let it = self
            .item_data
            .as_ref()
            .expect("rating ⇒ items")
            .get(item_type);
        let mut rating = it.dice_size_normal as i32 * it.dice_count_normal as i32;
        // item.plus not modeled (mundane loadout weapons) → the +plus*8 term is 0.
        if it.bonus_normal > 0 {
            rating += it.bonus_normal as i32 * 2;
        }
        if it.flags & gbx_formats::items::flags::FLAG_08 != 0 {
            rating += (it.number_attacks as i32 - 1) * 2;
        }
        if it.hands_count <= 1 {
            rating += 3;
        }
        rating
    }

    /// Whether the loadout's primary weapon would "find" an attack item — the
    /// `var_1F` ammo-availability test in `AI_items_selection` (coab
    /// `ovr010.cs:943-970`): the ammo slot present (not depleted) for a launcher,
    /// the weapon itself for a self-launcher, or the `flag_08|flag_02` (0x0A)
    /// sling special. Evaluated for the CANDIDATE weapon regardless of whether it
    /// is currently readied (unlike [`Self::get_current_attack_item`]).
    fn candidate_attack_found(&self, actor: usize, item_type: u8) -> bool {
        let Some(items) = self.item_data.as_ref() else {
            return false;
        };
        let flags = items.get(item_type).flags;
        let mut found = false;
        // Mirrors `get_current_attack_item`: a depleted self-launcher is a
        // lost item in the binary and cannot be re-found by the selection scan.
        if flags & gbx_formats::items::flags::FLAG_10 != 0 {
            found = !self.fighters[actor].ammo_item_lost;
        }
        if flags & gbx_formats::items::flags::FLAG_08 != 0
            && flags & (gbx_formats::items::flags::ARROWS | gbx_formats::items::flags::QUARRELS)
                != 0
        {
            found = !self.fighters[actor].ammo_item_lost;
        }
        found || flags == (gbx_formats::items::flags::FLAG_08 | gbx_formats::items::flags::FLAG_02)
    }

    /// `AI_items_selection(player)` (`sub_36673` @`ovr010:1673`, coab
    /// `ovr010.cs:875`; doc §34.5) — the cornered weapon swap, faithful over the
    /// loadout's single weapon (the secondary/shield/multi-item branches are
    /// cited-deferred, tripwired). The primary candidate `var_4` = the loadout
    /// bow (`rating = var_15`); the melee candidate `var_8` = bare hands here
    /// (`None`). The bow wins iff `rating > (var_16 >> 1)` (`var_16` = the base
    /// profile rating) AND ammo is available AND (ranged-melee OR no adjacent
    /// enemy). Otherwise bare hands. The observable swap (§34.5): unready → the
    /// attack-1 profile becomes the unarmed profile; re-ready → the saved entry
    /// profile; attacks recomputed via [`Self::reclac_attacks`] both ways.
    /// Inert without a loadout (weapon-only no-op). Draw-free.
    fn ai_items_selection(&mut self, actor: usize) {
        let Some(l) = self.fighters[actor].loadout else {
            return; // no loadout → nothing to select (today's melee no-op).
        };
        if self.item_data.is_none() {
            return;
        }
        // var_15 = CalcItemPowerRating(bow); var_16 = the base profile rating
        // (dsB*dcB (+2*bonusB if >0)).
        let var_15 = self.calc_item_power_rating(l.primary_type);
        let (dcb, dsb, dbb) = self.fighters[actor].base_dice;
        let mut var_16 = dsb as i32 * dcb as i32;
        if dbb as i32 > 0 {
            var_16 += dbb as i32 * 2;
        }
        // var_1F = the bow's ammo is available.
        let ammo_avail = self.candidate_attack_found(actor, l.primary_type);
        // ranged_melee(var_4) — a thrown weapon usable in hand (the candidate
        // may be unreadied, so this is the type-level test, not the actor one).
        let ranged_of_bow = self.item_data.as_ref().unwrap().get(l.primary_type).range as i32 > 1;
        let ranged_melee = ranged_of_bow && self.candidate_ranged_melee(l.primary_type);
        let no_adjacent = self.build_near(actor, 1, false).is_empty();

        // The bow wins the primary slot iff rating dominates the base, ammo is
        // available, and (ranged-melee or no adjacent enemy).
        let use_bow = var_15 > (var_16 >> 1) && ammo_avail && (ranged_melee || no_adjacent);

        let currently_readied = self.fighters[actor].weapon_readied;
        if use_bow && !currently_readied {
            // Re-ready the bow: primaryWeapon := bow, attack-1 profile := the
            // saved entry profile.
            self.fighters[actor].weapon_readied = true;
            let (dc, ds, db) = self.fighters[actor].entry_dice;
            self.fighters[actor].dice_count = dc;
            self.fighters[actor].dice_size = ds;
            self.fighters[actor].damage_bonus = db;
        } else if !use_bow && currently_readied {
            // Unready the bow: primaryWeapon := null, attack-1 profile := the
            // bare-hands profile.
            self.fighters[actor].weapon_readied = false;
            let (dc, ds, db) = l.unarmed_profile;
            self.fighters[actor].dice_count = dc;
            self.fighters[actor].dice_size = ds;
            self.fighters[actor].damage_bonus = db;
        }
        // The tail (`ovr010:1AB0-1AC6`, coab ovr010.cs:1018-1020) runs
        // `reclac_player_values` + `reclac_attacks` UNCONDITIONALLY — both the
        // replace path and the `replace_weapon = false` skip land on the same
        // merge point (audit fix: the recompute is not gated on a swap; the
        // §34.3 write-back gate is what makes the always-recompute safe).
        self.reclac_attacks(actor);
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

// --- the combat entry-state replay harness (H4, D-OR5(b)) ------------------

/// One combatant of a captured combat **entry-state snapshot** (`combat_entry`,
/// D-OR5(b)): its team, grid position, and the raw `0x1A6` record bytes (a full
/// `Player`/monster record). The replay harness decodes the record and places
/// the combatant **at `pos`** — the snapshot supplies the position, so
/// `PlaceCombatants` is deliberately *not* run in the replay path (one fewer
/// variable between our draw stream and the capture's).
pub struct RecordCombatant<'a> {
    pub team: Team,
    pub pos: GridPos,
    /// The full `0x1A6` combat record (`decode_char_record`'s input).
    pub record: &'a [u8],
}

/// Map one decoded `0x1A6` record onto a combat [`Combatant`] for a faithful
/// replay (H4). Built on top of [`Combatant::new_melee`] (the accepted real-fight
/// constructor) with the record-derived fields patched in. **Which record field
/// feeds which combat input** (the load-bearing mapping — every one of these is
/// read by some part of the draw stream, except where noted):
///
/// - **team / pos** — from the snapshot, not the record.
/// - **npc** — `control_morale@0xf7 >= 0x80` (gates the per-step morale d100 and
///   the `FleeCheck` block; a PC short-circuits both).
/// - **hp** — `hit_point_current@0x1a4` / `hit_point_max@0x78` (deaths change the
///   live counts → who is targetable → the draw stream; `enemy_health_pct` reads
///   the monster team's cur/max for morale).
/// - **ac** — raw `ac@0x19a` (the to-hit compare target; whether an attack hits
///   decides whether damage dice are rolled).
/// - **hit_bonus** — `hitBonus@0x199` (the current THAC0-derived to-hit number —
///   the field [`Combatant::hit_bonus`] itself names).
/// - **hit_dice** — `hit_dice@0xe5` (the `TrySweepAttack` 0-HD gate).
/// - **movement** — `movement@0x1a5` → [`calc_moves`] (half-move budget → how far
///   an actor steps → per-step monster d100 count).
/// - **reaction_adj** — `DexReactionAdj(stats2.Dex.full)` via the [`Flavor`]
///   (`full` == the record's `original` DEX byte); the initiative `delay = clamp(d6
///   + reaction_adj)`, so it drives selection order.
/// - **attacks_count** — `attacksCount@0x11c` (`attack_profile_base[0]`) →
///   [`this_round_action_count`] → `attack1_left` → number of to-hit d20s/round.
/// - **melee dice** — attack-1 `dice_count@0x19e` / `dice_size@0x1a0` /
///   `dmg_bonus@0x1a2` (`attack_profile_current[2/4/6]`). The readied-weapon
///   `ItemData` dice are not decoded yet (FD-29); the record's carried attack-1
///   dice are used directly, per the session brief.
///
/// **`field_186@0x186` (the save bonus) is intentionally not threaded:** the
/// [`Combatant`] model has no save-bonus cell because saving throws only fire for
/// spell/affect effects (stubbed to M5). A plain-melee replay rolls no saves, so
/// `field_186` feeds no draw here — it becomes load-bearing only once effects land.
fn combatant_from_record(
    id: usize,
    team: Team,
    pos: GridPos,
    rec: &CharRecord,
    raw: &[u8],
    flavor: &dyn Flavor,
) -> Combatant {
    let npc = rec.control_morale >= 0x80;
    let dice = (
        rec.attack_profile_current[2], // a1 dice_count @0x19e
        rec.attack_profile_current[4], // a1 dice_size  @0x1a0
        rec.attack_profile_current[6], // a1 dmg_bonus  @0x1a2
    );
    // stats2.Dex.full == the record's `original` DEX byte (coab reads .full).
    let reaction_adj = flavor.dex_reaction_bonus(rec.stats.dex.original) as i8;

    let mut c = Combatant::new_melee(
        id,
        team,
        npc,
        pos,
        rec.hit_point_current as i32,
        rec.ac as u8,
        rec.hit_bonus as i32,
        rec.movement as i32,
        dice,
        0, // delay — CalculateInitiative sets it each round
        1, // attack1_left — CalculateInitiative overwrites it from attacks_count
    );
    // Fields new_melee cannot carry from the record: max HP (may differ from
    // current), real hit dice, the DEX reaction adj, and the base attack count.
    c.hp_max = rec.hit_point_max as i32;
    c.ac_behind = rec.ac_behind as u8; // @0x19b — the behind-AC index target
    c.hit_dice = rec.hit_dice;
    c.reaction_adj = reaction_adj;
    c.attacks_count = rec.attack_profile_base[0]; // attacksCount @0x11c
                                                  // §15 bug #4 (the mage hold): class @0x75 and field_159 @0x159 (a 4-byte
                                                  // runtime far-pointer; null == all-zero). The QuickFight approach guards a
                                                  // non-fleeing class-5 (pure Magic-User) with a null field_159.
    c.class = rec.class;
    c.field_159_null = match raw.get(0x159..0x15D) {
        Some(p) => p.iter().all(|&b| b == 0),
        None => true, // full 0x1A6 records always carry it; missing → treat as null
    };
    // The `memorized-spells` tripwire input — `sub_3560B`'s spells_count. The
    // collection loop (`ovr010:062A-065D`) reads `record[0x1E + i]` for
    // i = 1..=0x53 (bytes 0x1F..0x71): slot 0 @0x1E is NEVER read, and the list
    // packs from the BACK (`SpellList.Save` fills from index 83 down — the first
    // memorized spell lands @0x71; doc §33's save-diff). ANY non-zero byte
    // counts (`cmp ..,0`/`jbe` ≡ `jz` @`ovr010:0637-063C`), so high-bit
    // "learning" entries collect too — coab's `LearntList()` filters them, a
    // cited coab≠binary nuance no capture exercises.
    c.memorized_spells = rec.spell_list[1..].iter().filter(|&&b| b != 0).count() as u8;
    // §26.1 the downed-PC ladder: the entry `health_status@0x195` (okey in a
    // fresh combat snapshot). `bleeding` starts 0; `damage_player` seeds it.
    c.health_status = decode_health_status(rec.health_status);
    c.bleeding = 0;
    // §28 the faithful FleeCheck ladder: the raw `control_morale@0xF7` (for the
    // per-actor morale reseed `(control_morale & 0x7F) << 1`) and `Int@0x13`
    // (`stats2.Int.original` — the `.original`/`.full` byte, as DEX above; the
    // surrender branch's `Int > 5` gate). `npc` already folds control_morale.
    c.control_morale = rec.control_morale;
    c.int_score = rec.stats.int.original;
    // §34 the armed/ranged slice. The saved readied attack-1 profile (for the
    // cornered unready→re-ready swap, §34.5) is the record's decoded `dice`;
    // the attack-2 profile is @0x19F/0x1A1/0x1A3 (idx-2 damage, §34.6 — all
    // zero in this party); `baseHalfMoves`@0x11D folds into `attack2_left`
    // (§34.3); `field_DE`@0xde drives the large-target and backstab size gates;
    // and `SkillLevel(Thief)` is precomputed for the backstab multiplier (§34.6).
    c.entry_dice = dice;
    c.attack2_dice = (
        rec.attack_profile_current[3], // a2 dice_count @0x19f
        rec.attack_profile_current[5], // a2 dice_size  @0x1a1
        rec.attack_profile_current[7], // a2 dmg_bonus  @0x1a3
    );
    c.base_half_moves = rec.attack_profile_base[1]; // baseHalfMoves @0x11d
    c.base_dice = (
        rec.attack_profile_base[2], // attack1_DiceCountBase @0x11e
        rec.attack_profile_base[4], // attack1_DiceSizeBase  @0x120
        rec.attack_profile_base[6], // attack1_DamageBonusBase @0x122
    );
    c.field_de = rec.field_de; // @0xde
    c.thief_skill_level = skill_level_thief(rec);
    c
}

/// `SkillLevel(SkillType.Thief)` (coab `Player.cs:492`): `ClassLevel[Thief] +
/// ClassLevelsOld[Thief] * DualClassExceedsPreviousLevel()`. The binary reads
/// `rec[0x10F]` (`ClassLevel[6]`) and `rec[0x117]` (`ClassLevelsOld[6]`) and
/// multiplies the latter by `sub_6B3D1` (`ovr014:01F9-021F`, verified this
/// session). `DualClassExceedsPreviousLevel` (`sub_6B3D1`, `Player.cs:800`) =
/// `DuelClassCurrentLevel() > multiclassLevel ? 1 : 0`, where
/// `DuelClassCurrentLevel` (`Player.cs:812`) returns 0 for non-humans, else the
/// first non-zero `ClassLevel[0..7]` (or `ClassLevel[7]` if `0..7` are all 0).
/// Constant during a fight — precomputed at decode.
fn skill_level_thief(rec: &CharRecord) -> i32 {
    const THIEF: usize = 6; // SkillType.Thief (Classes/Enums.cs:64)
    const HUMAN: u8 = 7; // Race.human (Classes/Enums.cs:54)
    let dual = {
        let current = if rec.race != HUMAN {
            0
        } else {
            let mut i = 0;
            while i < 7 && rec.class_level[i] == 0 {
                i += 1;
            }
            rec.class_level[i] as i32
        };
        i32::from(current > rec.multiclass_level as i32)
    };
    rec.class_level[THIEF] as i32 + rec.class_levels_old[THIEF] as i32 * dual
}

/// Build a [`CombatState`] from a captured combat entry-state snapshot (H4,
/// D-OR5(b)) — the replay harness. Decodes each `0x1A6` record, maps it onto a
/// [`Combatant`] ([`combatant_from_record`]), and assembles the roster **in the
/// snapshot's order** (== `TeamList` == the initiative draw order — load-bearing)
/// **at the snapshot's positions** (no `PlaceCombatants`). The result is a full
/// melee fight ([`CombatState::new`], `TurnDriver::MeleeAi`) over `map`.
///
/// The caller owns the RNG: seed a [`EngineRng`] with the snapshot's `rng_state`,
/// attach an `RngSink`, then drive `state.step(&mut rng)` (or `run_combat`) to
/// `Ended`. A record that fails to decode is a loud [`SaveParseError`] (tooling
/// input, never silently tolerated).
pub fn combat_state_from_records(
    entries: &[RecordCombatant],
    map: CombatMap,
    flavor: &dyn Flavor,
) -> Result<CombatState, SaveParseError> {
    let mut fighters = Vec::with_capacity(entries.len());
    for (id, e) in entries.iter().enumerate() {
        let rec = decode_char_record(e.record)?;
        fighters.push(combatant_from_record(
            id, e.team, e.pos, &rec, e.record, flavor,
        ));
    }
    Ok(CombatState::new(map, fighters))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{RngDraw, RngSink};
    use gbx_prng::Prng;
    use std::cell::RefCell;
    use std::rc::Rc;

    // --- test doubles ------------------------------------------------------

    /// Records the operand `n` and `result` of every PRNG draw at the engine
    /// seam — lets a test assert the *exact* draw sequence (kinds and values).
    #[derive(Clone, Default)]
    struct DrawLog {
        draws: Rc<RefCell<Vec<RngDraw>>>,
    }
    struct DrawSink(Rc<RefCell<Vec<RngDraw>>>);
    impl RngSink for DrawSink {
        fn on_draw(&mut self, draw: RngDraw) {
            self.0.borrow_mut().push(draw);
        }
    }
    impl DrawLog {
        fn sink(&self) -> Box<dyn RngSink> {
            Box::new(DrawSink(Rc::clone(&self.draws)))
        }
        fn ns(&self) -> Vec<u16> {
            self.draws.borrow().iter().map(|d| d.n.unwrap()).collect()
        }
        fn len(&self) -> usize {
            self.draws.borrow().len()
        }
    }

    /// Records every emitted action event.
    #[derive(Clone, Default)]
    struct ActionLog {
        events: Rc<RefCell<Vec<ActionEvent>>>,
    }
    struct ActionSinkImpl(Rc<RefCell<Vec<ActionEvent>>>);
    impl ActionSink for ActionSinkImpl {
        fn on_action(&mut self, event: ActionEvent) {
            self.0.borrow_mut().push(event);
        }
    }
    impl ActionLog {
        fn sink(&self) -> Box<dyn ActionSink> {
            Box::new(ActionSinkImpl(Rc::clone(&self.events)))
        }
        fn events(&self) -> Vec<ActionEvent> {
            self.events.borrow().clone()
        }
    }

    /// An independent replay of the same seed — the by-hand oracle for what
    /// `1 + random(size)` yields, so tests derive expected delays/rolls without
    /// trusting the code under test.
    struct Replay(Prng);
    impl Replay {
        fn new(seed: u32) -> Self {
            Replay(Prng::new(seed))
        }
        fn roll(&mut self, size: u16) -> u16 {
            1 + self.0.random(size)
        }
    }

    const SEED: u32 = 0x0C0F_FEE0; // the §15 capture seed, reused

    fn party(id: usize, reaction_adj: i8) -> Combatant {
        Combatant::new(id, Team::Party, reaction_adj, true)
    }
    fn monster(id: usize) -> Combatant {
        Combatant::new(id, Team::Monster, 0, true)
    }

    fn clamp_init(d6: u16, reaction_adj: i8) -> i8 {
        // The CalculateInitiative clamp with no surprise (surprise_mask == 0).
        let mut delay = d6 as i32 + reaction_adj as i32;
        if delay < 1 {
            delay = 1;
        }
        if !(0..=20).contains(&delay) {
            delay = 0;
        }
        delay as i8
    }

    // === the armed/ranged slice (doc §34) test support + units ==============

    /// A synthetic `ITEMS` table with the rows the ranged tests exercise (doc
    /// §34.1) plus a natk-1 launcher (type 45) for the floor test and a range-1
    /// weapon (type 30) for the range sanitize test.
    fn synth_item_table() -> gbx_formats::items::ItemDataTable {
        let mut bytes = vec![0u8; 2 + 0x81 * 0x10];
        let mut set = |t: usize, e: [u8; 16]| {
            let off = 2 + t * 0x10;
            bytes[off..off + 16].copy_from_slice(&e);
        };
        // 43 LongBow: range 22, natk 4, 1d6 normal, flags 0x0B (arrows|02|08).
        set(
            43,
            [0, 2, 1, 6, 0, 4, 0, 1, 0x80, 1, 6, 0, 22, 0xC8, 0x0B, 0],
        );
        // 47 Sling: range 21, flags 0x0A (flag_08|flag_02), 1d4+1 normal.
        set(
            47,
            [0, 1, 1, 6, 1, 2, 0, 0x80, 0x80, 1, 4, 1, 21, 0xDC, 0x0A, 0],
        );
        // 45 (a natk-1 launcher): range 5, natk 1, flags 0x0B.
        set(
            45,
            [0, 2, 1, 8, 0, 1, 0, 1, 0x80, 1, 8, 0, 5, 0xC8, 0x0B, 0],
        );
        // 30 (a range-1 melee weapon): range 1, flags 0x04.
        set(
            30,
            [0, 1, 1, 8, 0, 0, 0, 0, 0x80, 1, 8, 0, 1, 0xCC, 0x04, 0],
        );
        gbx_formats::items::ItemDataTable::parse(&bytes).unwrap()
    }

    /// A one-combatant state with `primary_type` readied over the synthetic
    /// table; `attacks_count` seeds the melee half-action count. `ammo` sets the
    /// launcher ammo.
    fn ranged_state(primary_type: u8, attacks_count: u8, ammo: i32) -> CombatState {
        let mut c = Combatant::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(0, 0),
            10,
            40,
            0,
            12,
            (1, 6, 0),
            5,
            2,
        );
        c.attacks_count = attacks_count;
        let mut state = CombatState::new(CombatMap::uniform(0x17), vec![c]);
        state.item_data = Some(synth_item_table());
        state.set_loadout(
            0,
            Loadout {
                primary_type,
                ammo_count: ammo,
                unarmed_profile: (1, 2, 6),
            },
        );
        state
    }

    #[test]
    fn ranged_predicate_and_current_attack_item() {
        let mut state = ranged_state(43, 2, 40); // LongBow
        assert!(state.is_weapon_ranged(0));
        assert!(!state.is_weapon_ranged_melee(0)); // bow has no melee/flag_10
        let it = state.get_current_attack_item(0);
        assert!(it.found);
        assert_eq!(it.item, AttackItemRef::Ammo);
        assert_eq!(state.attack_item_count(0, &it), Some(40));
        // Unreadying the bow → not ranged, no attack item found.
        state.fighters[0].weapon_readied = false;
        assert!(!state.is_weapon_ranged(0));
        assert!(!state.get_current_attack_item(0).found);
        // No loadout at all → melee.
        state.fighters[0].loadout = None;
        state.fighters[0].weapon_readied = true;
        assert!(!state.is_weapon_ranged(0));
    }

    #[test]
    fn ranged_predicate_sling_finds_null_item() {
        // Sling (flags 0x0A) "finds" a null item and still shoots (doc §34.2).
        let state = ranged_state(47, 2, 40);
        assert!(state.is_weapon_ranged(0)); // range 21 > 1
        let it = state.get_current_attack_item(0);
        assert!(it.found); // the flag_08|flag_02 == 0x0A special case
        assert_eq!(it.item, AttackItemRef::None); // no ammo item
        assert_eq!(state.attack_item_count(0, &it), None); // no ammo cap
    }

    #[test]
    fn weapon_range_sanitizes() {
        let mut state = ranged_state(43, 2, 40); // LongBow 22 → 21
        assert_eq!(state.weapon_range(0), 21);
        // A range-1 weapon → r = 0 → sanitized to 1.
        state.set_loadout(
            0,
            Loadout {
                primary_type: 30,
                ammo_count: 0,
                unarmed_profile: (1, 2, 6),
            },
        );
        assert_eq!(state.weapon_range(0), 1);
        // No readied weapon → 1.
        state.fighters[0].weapon_readied = false;
        assert_eq!(state.weapon_range(0), 1);
    }

    #[test]
    fn reclac_melee_matches_this_round_action_count() {
        // No loadout: attack1_left = ThisRoundActionCount(attacksCount) — the
        // pre-slice behaviour, both parities.
        let mut c = Combatant::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(0, 0),
            10,
            40,
            0,
            12,
            (1, 6, 0),
            5,
            2,
        );
        c.attacks_count = 3;
        let mut state = CombatState::new(CombatMap::uniform(0x17), vec![c]);
        state.combat_round = 0;
        state.fighters[0].field_8 = false;
        state.reclac_attacks(0);
        assert_eq!(state.fighters[0].attack1_left, 1); // (3+0)/2
        state.combat_round = 1;
        state.fighters[0].field_8 = false;
        state.reclac_attacks(0);
        assert_eq!(state.fighters[0].attack1_left, 2); // (3+1)/2
    }

    #[test]
    fn reclac_ranged_natk_floor_and_parity() {
        // LongBow natk 4 → 2 shots both parities ((4+0)/2, (4+1)/2 == 2).
        let mut state = ranged_state(43, 2, 40);
        state.combat_round = 0;
        state.fighters[0].field_8 = false;
        state.reclac_attacks(0);
        assert_eq!(state.fighters[0].attack1_left, 2);
        state.combat_round = 1;
        state.fighters[0].field_8 = false;
        state.reclac_attacks(0);
        assert_eq!(state.fighters[0].attack1_left, 2);
        // A natk-1 launcher floors to 2 half-actions → 1 shot even, 1 odd.
        let mut s2 = ranged_state(45, 2, 40);
        s2.combat_round = 0;
        s2.fighters[0].field_8 = false;
        s2.reclac_attacks(0);
        assert_eq!(s2.fighters[0].attack1_left, 1); // max(2,1)=2 → (2+0)/2
    }

    #[test]
    fn reclac_ranged_ammo_cap() {
        // Ammo 1 caps the 2-shot round to 1.
        let mut state = ranged_state(43, 2, 1);
        state.combat_round = 0;
        state.fighters[0].field_8 = false;
        state.reclac_attacks(0);
        assert_eq!(state.fighters[0].attack1_left, 1);
    }

    #[test]
    fn reclac_field_8_writeback_gate() {
        // With field_8 set (mid-turn recompute) and a ranged weapon, the gate
        // `attacks < orig` blocks a re-inflation: orig 1 < attacks 2, ranged, so
        // the count is NOT overwritten and stays at attacksCount.
        let mut state = ranged_state(43, 2, 40);
        state.combat_round = 0;
        state.fighters[0].attack1_left = 1; // orig
        state.fighters[0].field_8 = true;
        state.reclac_attacks(0);
        // gate: !field_8(F) || 2<1(F) || (T && 2<2 && !ranged=F) → F ⇒ keep the
        // attacksCount write (2) from the head of reclac.
        assert_eq!(state.fighters[0].attack1_left, 2);
    }

    #[test]
    fn ammo_subtracts_by_swing_count_not_assigned() {
        // coab≠binary #16: the binary SUBTRACTS the attack-1 swing count from
        // `item.count`; coab assigns. Two swings from ammo 40 → 38 (not 2).
        let bowman = Combatant::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(0, 0),
            30,
            40,
            40, // hit_bonus high — swings land, but the count is what matters
            12,
            (1, 6, 0),
            5,
            2, // attack1_left = 2
        );
        let target = Combatant::new_melee(
            1,
            Team::Monster,
            true,
            GridPos::new(3, 0),
            200, // survives both swings so the loop runs fully
            40,
            0,
            12,
            (1, 2, 0),
            5,
            1,
        );
        let mut state = CombatState::new(CombatMap::uniform(0x17), vec![bowman, target]);
        state.item_data = Some(synth_item_table());
        state.set_loadout(
            0,
            Loadout {
                primary_type: 43,
                ammo_count: 40,
                unarmed_profile: (1, 2, 6),
            },
        );
        assert_eq!(state.fighters[0].attack1_left, 2);
        let mut rng = EngineRng::new(SEED);
        state.attack_target(&mut rng, 0, 1, false, AttackItemRef::Ammo);
        assert_eq!(state.fighters[0].ammo, 38); // 40 − 2, SUBTRACT not assign
        assert!(!state.fighters[0].ammo_item_lost);
    }

    #[test]
    fn ranged_defense_bonus_bands() {
        // LongBow (range 22) → oneThird = 7: range ≤ 7 → 0, 8..14 → +2,
        // > 14 → +5. Validate the wiring reproduces the piecewise formula over
        // `get_target_range`, and that a far target actually reaches +5.
        let mk = |tx: i32| -> CombatState {
            let bowman = Combatant::new_melee(
                0,
                Team::Party,
                false,
                GridPos::new(0, 0),
                30,
                40,
                0,
                12,
                (1, 6, 0),
                5,
                2,
            );
            let target = Combatant::new_melee(
                1,
                Team::Monster,
                true,
                GridPos::new(tx, 0),
                30,
                40,
                0,
                12,
                (1, 2, 0),
                5,
                1,
            );
            let mut state = CombatState::new(CombatMap::uniform(0x17), vec![bowman, target]);
            state.item_data = Some(synth_item_table());
            state.set_loadout(
                0,
                Loadout {
                    primary_type: 43,
                    ammo_count: 40,
                    unarmed_profile: (1, 2, 6),
                },
            );
            state
        };
        let band = |r: i32| -> i32 {
            let one_third = 7;
            let mut adj = 0;
            let mut rr = r;
            if rr > one_third {
                rr -= one_third;
                adj += 2;
                if rr > one_third {
                    adj += 3;
                }
            }
            adj
        };
        let mut saw_plus5 = false;
        for tx in [1, 8, 20, 40] {
            let state = mk(tx);
            let r =
                get_target_range(&state.map, state.fighters[1].pos, state.fighters[0].pos) as i32;
            assert_eq!(state.ranged_defense_bonus(0, 1), band(r), "tx={tx} r={r}");
            if state.ranged_defense_bonus(0, 1) == 5 {
                saw_plus5 = true;
            }
        }
        assert!(saw_plus5, "a far target must reach the +5 band");
        // A non-ranged attacker (bow unreadied) → 0.
        let mut melee = mk(40);
        melee.fighters[0].weapon_readied = false;
        assert_eq!(melee.ranged_defense_bonus(0, 1), 0);
    }

    #[test]
    fn cornered_swap_unready_then_reready() {
        // A bowman with an adjacent enemy unreadies to the unarmed profile;
        // clearing the enemy re-readies the bow and restores the entry profile.
        let bowman = Combatant::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(0, 0),
            30,
            40,
            40,
            12,
            (1, 6, 0), // entry bow profile
            5,
            2,
        );
        let patron = Combatant::new_melee(
            1,
            Team::Monster,
            true,
            GridPos::new(1, 0), // adjacent
            16,
            40,
            0,
            12,
            (1, 2, 0),
            5,
            1,
        );
        let mut state = CombatState::new(CombatMap::uniform(0x17), vec![bowman, patron]);
        state.item_data = Some(synth_item_table());
        state.set_loadout(
            0,
            Loadout {
                primary_type: 43,
                ammo_count: 40,
                unarmed_profile: (1, 2, 6),
            },
        );
        // (`set_loadout` snapshots `entry_dice` from the live profile — no
        // hand-set needed; the re-ready below proves it.)
        assert!(state.is_weapon_ranged(0));

        // Adjacent enemy → unready to fists.
        state.ai_items_selection(0);
        assert!(!state.fighters[0].weapon_readied);
        assert_eq!(
            (
                state.fighters[0].dice_count,
                state.fighters[0].dice_size,
                state.fighters[0].damage_bonus
            ),
            (1, 2, 6)
        );
        assert!(!state.is_weapon_ranged(0));

        // Clear the enemy → re-ready the bow, restore the entry profile.
        state.fighters[1].in_combat = false;
        state.rebuild_occupancy();
        state.ai_items_selection(0);
        assert!(state.fighters[0].weapon_readied);
        assert_eq!(
            (
                state.fighters[0].dice_count,
                state.fighters[0].dice_size,
                state.fighters[0].damage_bonus
            ),
            (1, 6, 0)
        );
        assert!(state.is_weapon_ranged(0));
    }

    #[test]
    fn try_guarding_ranged_clears_never_guards() {
        // A ranged attacker never parks a guard (§34.4): clear, no guard flag.
        let mut state = ranged_state(43, 2, 40);
        state.fighters[0].delay = 5;
        state.try_guarding(0);
        assert!(!state.fighters[0].guarding);
        assert_eq!(state.fighters[0].delay, 0);
        // Unreadied (melee) with delay > 0 → guards as before.
        state.fighters[0].weapon_readied = false;
        state.fighters[0].delay = 5;
        state.try_guarding(0);
        assert!(state.fighters[0].guarding);
    }

    // --- pure selection logic (the two-if tie-break) -----------------------

    #[test]
    fn selection_picks_highest_delay() {
        // delay 8 beats delay 5 regardless of rolls.
        assert_eq!(select_combatant(&[5, 8, 3], &[99, 1, 50]), Some((1, 1)));
    }

    #[test]
    fn selection_breaks_ties_by_highest_roll() {
        // All delay 5: highest roll (index 2) wins.
        assert_eq!(select_combatant(&[5, 5, 5], &[30, 20, 50]), Some((2, 50)));
        // Equal rolls at the max: the later member wins (`>=` overwrite).
        assert_eq!(select_combatant(&[5, 5], &[40, 40]), Some((1, 40)));
    }

    #[test]
    fn selection_exercises_the_gt_only_branch_reset() {
        // The `>`-only branch (first if) resets max_roll so a strictly-higher
        // delay wins even with a LOWER roll than the running max. Without the
        // reset, index 1 (roll 10 < 90) would fail the second if and index 0
        // would wrongly win.
        assert_eq!(select_combatant(&[5, 8], &[90, 10]), Some((1, 10)));
        // Three-way: A(5,90) then B(8,10) then C(8,50) → C (delay 8, higher roll).
        assert_eq!(select_combatant(&[5, 8, 8], &[90, 10, 50]), Some((2, 50)));
    }

    #[test]
    fn selection_ends_when_all_delays_zero() {
        assert_eq!(select_combatant(&[0, 0, 0], &[99, 50, 1]), None);
        // A transient delay-0 pick is nulled out by the max_delay==0 guard.
        assert_eq!(select_combatant(&[0], &[100]), None);
    }

    // --- initiative draw sequence ------------------------------------------

    #[test]
    fn initiative_draws_one_d6_per_in_combat_combatant_in_roster_order() {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());

        // Mixed in_combat: members 0,1,3 in combat; member 2 out.
        let roster = vec![
            party(0, 3),                              // dex reaction +3
            party(1, -2),                             // dex reaction -2
            Combatant::new(2, Team::Party, 0, false), // not in combat → no d6
            monster(3),                               // reaction 0
        ];
        let mut state = CombatState::initiative_only(roster);

        let step = state.step(&mut rng);
        assert_eq!(step, CombatStep::RoundStarted { round: 0 });

        // Exactly three d6 draws, in order, for the three in-combat members.
        assert_eq!(log.ns(), vec![6, 6, 6]);

        // Delays match a by-hand replay of the same seed.
        let mut oracle = Replay::new(SEED);
        let d0 = oracle.roll(6);
        let d1 = oracle.roll(6);
        let d3 = oracle.roll(6);
        assert_eq!(state.roster()[0].delay, clamp_init(d0, 3));
        assert_eq!(state.roster()[1].delay, clamp_init(d1, -2));
        assert_eq!(state.roster()[2].delay, 0, "not in combat");
        assert_eq!(state.roster()[3].delay, clamp_init(d3, 0));
    }

    #[test]
    fn surprise_subtracts_six_after_the_min_one_clamp() {
        // reaction -3, d6 min 1 → pre-surprise delay clamps up to 1, then -6 →
        // -5 → out of range → 0. Prove the clamp-then-subtract ordering.
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let actions = ActionLog::default();

        // Party is bit (0+1)=1 → surprise_mask bit 0 set. A monster is needed
        // for the fight to actually start (the emptiness guard).
        let mut state =
            CombatState::initiative_only(vec![party(0, -3), monster(9)]).with_surprise_mask(0b01);
        state.attach_action_sink(actions.sink());
        state.step(&mut rng);

        // Whatever the d6 (1..6), with reaction -3 the pre-surprise value is in
        // 1..3 (after the min-1 clamp), minus 6 is negative → 0.
        assert_eq!(state.roster()[0].delay, 0);
        match actions.events()[0] {
            ActionEvent::Init {
                combatant_id,
                delay,
                dex_adj,
                surprise,
            } => {
                assert_eq!((combatant_id, delay, dex_adj, surprise), (0, 0, -3, true));
            }
            other => panic!("expected Init, got {other:?}"),
        }
    }

    #[test]
    fn dex_reaction_bonus_comes_from_the_flavor_not_a_hardcode() {
        use gbx_rules::adnd1::flavor_impl::Adnd1;
        use gbx_rules::pack::RuleSet;
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);
        // 18 DEX → +3 reaction (ovr025.cs:551-553), sourced through gbx-rules.
        let c = Combatant::from_dex(0, Team::Party, 18, true, &flavor);
        assert_eq!(c.reaction_adj, 3);
        let c = Combatant::from_dex(1, Team::Party, 3, true, &flavor);
        assert_eq!(c.reaction_adj, -3); // dex 3 → 3-6 = -3
    }

    // --- per-pass d100 burst = roster size ---------------------------------

    #[test]
    fn every_selection_pass_draws_exactly_one_d100_per_roster_member() {
        // A 16-combatant roster — the §15 live signature: bursts of exactly 16.
        // (In the real game turns interleave their own draws, splitting the raw
        // stream into separate 16-runs; here the stub draws nothing between
        // passes, so we assert the count PER pass directly.)
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());

        let mut roster = Vec::new();
        for id in 0..6 {
            roster.push(party(id, 0));
        }
        for id in 6..16 {
            roster.push(monster(id));
        }
        assert_eq!(roster.len(), 16);
        let mut state = CombatState::initiative_only(roster);

        // RoundStarted step: 16 d6 (all in combat).
        let mut before = log.len();
        assert_eq!(state.step(&mut rng), CombatStep::RoundStarted { round: 0 });
        assert_eq!(
            log.ns()[before..],
            [6u16; 16],
            "one d6 per in-combat member"
        );

        // Every subsequent step (each Turn, and the terminating RoundEnded)
        // consumes exactly 16 d100 draws.
        loop {
            before = log.len();
            let step = state.step(&mut rng);
            let burst = &log.ns()[before..];
            assert_eq!(
                burst.len(),
                16,
                "each selection pass rolls one d100 per member"
            );
            assert!(burst.iter().all(|&n| n == 100), "the burst is all d100s");
            match step {
                CombatStep::Turn { .. } => continue,
                CombatStep::RoundEnded { .. } => break,
                other => panic!("unexpected step {other:?}"),
            }
        }
    }

    // --- whole-round draw total --------------------------------------------

    #[test]
    fn a_round_draws_kc_d6_then_a_plus_one_times_k_d100() {
        // K = 4, all in combat, reaction 0 → every d6 gives delay 1..6 > 0, so
        // all A = 4 act: 4 d6 + (4+1)*4 = 4 + 20 = 24 draws.
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());

        let roster = vec![party(0, 0), party(1, 0), monster(2), monster(3)];
        let k = roster.len();
        let mut state = CombatState::initiative_only(roster);

        let mut turns = 0;
        loop {
            match state.step(&mut rng) {
                CombatStep::RoundStarted { .. } => {}
                CombatStep::Turn { .. } => turns += 1,
                CombatStep::RoundEnded { round, .. } => {
                    assert_eq!(round, 1);
                    break;
                }
                CombatStep::Ended => panic!("ended mid-round"),
            }
        }
        assert_eq!(turns, k, "every in-combat member with delay>0 acts once");
        // K_c d6 + (A+1)*K d100.
        assert_eq!(log.len(), k + (turns + 1) * k);
        assert_eq!(log.len(), 4 + 5 * 4);
    }

    // --- pick events + tie-break through the real state machine ------------

    #[test]
    fn pick_events_track_selection_order_and_zero_the_picked_delay() {
        let mut rng = EngineRng::new(SEED);
        let actions = ActionLog::default();
        let roster = vec![party(0, 0), party(1, 0), monster(2)];
        let mut state = CombatState::initiative_only(roster);
        state.attach_action_sink(actions.sink());

        let mut picks = Vec::new();
        loop {
            match state.step(&mut rng) {
                CombatStep::RoundStarted { .. } => {}
                CombatStep::Turn { combatant_id } => picks.push(combatant_id),
                CombatStep::RoundEnded { .. } => break,
                CombatStep::Ended => panic!("ended mid-round"),
            }
        }

        // Every in-combat member is picked exactly once (each acts, then zeroed).
        let mut sorted = picks.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2]);

        // The action log holds 3 Init then one Pick per selection, in order, and
        // the pass indices ascend from 0.
        let events = actions.events();
        let inits = events
            .iter()
            .filter(|e| matches!(e, ActionEvent::Init { .. }))
            .count();
        assert_eq!(inits, 3);
        let pick_events: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ActionEvent::Pick {
                    pass, combatant_id, ..
                } => Some((*pass, *combatant_id)),
                _ => None,
            })
            .collect();
        assert_eq!(pick_events.len(), 3);
        for (i, (pass, id)) in pick_events.iter().enumerate() {
            assert_eq!(*pass as usize, i, "pass index ascends from 0");
            assert_eq!(*id, picks[i], "pick event matches yielded combatant");
        }
    }

    // --- termination --------------------------------------------------------

    #[test]
    fn combat_terminates_at_the_stalemate_cap() {
        // Nobody dies in the stub, so the only terminator is combat_round >= 15.
        let mut rng = EngineRng::new(SEED);
        let mut state = CombatState::initiative_only(vec![party(0, 0), monster(1)]);

        let mut rounds_ended = 0;
        let final_step = loop {
            match state.step(&mut rng) {
                CombatStep::RoundEnded {
                    battle_over: true, ..
                } => {
                    rounds_ended += 1;
                    break CombatStep::Ended;
                }
                CombatStep::RoundEnded { .. } => rounds_ended += 1,
                CombatStep::Ended => break CombatStep::Ended,
                _ => {}
            }
        };
        assert_eq!(final_step, CombatStep::Ended);
        assert_eq!(rounds_ended, DEFAULT_NO_ACTION_LIMIT);
        assert_eq!(state.combat_round(), DEFAULT_NO_ACTION_LIMIT);
        assert_eq!(state.step(&mut rng), CombatStep::Ended, "stays ended");
    }

    #[test]
    fn empty_side_ends_before_any_draw() {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        // No monsters → no fight.
        let mut state = CombatState::initiative_only(vec![party(0, 0), party(1, 0)]);
        assert_eq!(state.step(&mut rng), CombatStep::Ended);
        assert_eq!(log.len(), 0, "the emptiness guard draws nothing");
    }

    // --- roll_dice byte truncation (FD-29) ---------------------------------

    #[test]
    fn roll_dice_truncates_the_total_to_a_byte() {
        // 100 dice of d100: the untruncated total blows past 255, so the
        // (byte)roll_total truncation (ovr024.cs:595) is observable — the
        // data-driven FD-29 clause. Our roll_dice must wrap mod 256.
        let mut rng = EngineRng::new(SEED);
        let got = roll_dice(&mut rng, 100, 100);

        let mut o = Replay::new(SEED);
        let mut full = 0u32;
        for _ in 0..100 {
            full += o.roll(100) as u32;
        }
        assert!(full > 255, "the untruncated total must exceed a byte");
        assert_eq!(got, (full as u8) as u16, "roll_dice truncates to a byte");
        // A total under 256 is unaffected (the initiative d6/d100 case).
        let mut rng = EngineRng::new(SEED);
        let small = roll_dice(&mut rng, 6, 3); // max 18
        let mut o = Replay::new(SEED);
        assert_eq!(small, o.roll(6) + o.roll(6) + o.roll(6));
    }

    // --- to-hit: both paths, the auto-rules, and the >/>= boundary ---------

    #[test]
    fn to_hit_natural_1_misses_and_natural_20_hits_via_the_100_promotion() {
        // AC 50 with 0 bonus: a plain roll (effective ≤ 19) can never reach it,
        // but a nat-20 promotes to 100 and clears it. A nat-1 misses (the gate).
        let mut rng = EngineRng::new(SEED);
        let (mut saw1, mut saw20, mut saw_plain) = (false, false, false);
        for _ in 0..2000 {
            let r = pc_can_hit_target(&mut rng, 50, 0, 0);
            match r.d20 {
                1 => {
                    assert!(!r.hit, "nat-1 auto-miss");
                    saw1 = true;
                }
                20 => {
                    assert!(r.hit, "nat-20 → 100 beats AC 50");
                    saw20 = true;
                }
                d => {
                    assert!((2..=19).contains(&d));
                    assert!(!r.hit, "a plain d20 can't reach AC 50 with 0 bonus");
                    saw_plain = true;
                }
            }
            if saw1 && saw20 && saw_plain {
                break;
            }
        }
        assert!(
            saw1 && saw20 && saw_plain,
            "expected a nat-1, a nat-20, and a plain roll within budget"
        );
    }

    #[test]
    fn natural_1_misses_even_when_it_would_otherwise_certainly_hit() {
        // AC 0, 0 bonus: every non-1 roll hits (>= path, effective ≥ 2 ≥ 0);
        // only the nat-1 gate produces a miss.
        let mut rng = EngineRng::new(SEED);
        let mut saw1 = false;
        for _ in 0..2000 {
            let r = pc_can_hit_target(&mut rng, 0, 0, 0);
            if r.d20 == 1 {
                assert!(!r.hit, "nat-1 overrides an otherwise-certain hit");
                saw1 = true;
                break;
            }
            assert!(r.hit, "any non-1 vs raw AC 0 hits under >=");
        }
        assert!(saw1, "expected a nat-1 within budget");
    }

    #[test]
    fn gt_path_and_ge_path_disagree_at_the_equality_point() {
        // The single load-bearing asymmetry (study §14.4): at the exact equality
        // point, the weapon path (PC_CanHitTarget, >=) HITS while the scripted
        // path (CanHitTarget, >) MISSES — for the *same* d20.
        let d20 = Replay::new(SEED).roll(20);
        assert!(
            (2..=19).contains(&d20),
            "this boundary test needs the seed's first d20 to be a plain roll (got {d20})"
        );
        // effective(=d20) + bonus(0) == target_ac exactly.
        let target_ac = d20 as u8;

        let mut rng = EngineRng::new(SEED);
        let ge = pc_can_hit_target(&mut rng, target_ac, 0, 0);
        assert_eq!(ge.d20 as u16, d20);
        assert!(ge.hit, "PC_CanHitTarget uses >=, so equality hits");

        let mut rng = EngineRng::new(SEED);
        let gt = can_hit_target(&mut rng, 0, target_ac);
        assert_eq!(gt.d20 as u16, d20);
        assert!(!gt.hit, "CanHitTarget uses strict >, so equality misses");
    }

    #[test]
    fn to_hit_draws_exactly_one_d20() {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        pc_can_hit_target(&mut rng, 40, 5, 1);
        can_hit_target(&mut rng, 3, 40);
        assert_eq!(log.ns(), vec![20, 20], "one d20 per to-hit, no more");
    }

    // --- damage: dice + bonus, clamp, backstab, exact draw count -----------

    #[test]
    fn damage_is_dice_plus_bonus_with_exact_draw_count() {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());

        let dmg = roll_damage(&mut rng, 8, 3, 2, None); // 3d8+2
        assert_eq!(log.ns(), vec![8, 8, 8], "exactly dice_count draws");

        let mut o = Replay::new(SEED);
        let base = o.roll(8) + o.roll(8) + o.roll(8) + 2;
        assert_eq!(dmg.amount, base as i32);
        assert!(!dmg.backstab);
    }

    #[test]
    fn damage_applies_the_backstab_multiplier() {
        let mut rng = EngineRng::new(SEED);
        let dmg = roll_damage(&mut rng, 4, 2, 1, Some(3)); // (2d4+1) × 3
        let mut o = Replay::new(SEED);
        let base = o.roll(4) + o.roll(4) + 1;
        assert_eq!(dmg.amount, base as i32 * 3);
        assert!(dmg.backstab);
    }

    #[test]
    fn backstab_multiplier_matches_the_thief_level_bands() {
        // ((level - 1) / 4) + 2, truncating.
        assert_eq!(backstab_multiplier(1), 2);
        assert_eq!(backstab_multiplier(4), 2);
        assert_eq!(backstab_multiplier(5), 3);
        assert_eq!(backstab_multiplier(8), 3);
        assert_eq!(backstab_multiplier(9), 4);
        assert_eq!(backstab_multiplier(13), 5);
    }

    #[test]
    fn damage_clamp_and_byte_bonus_quirk() {
        // The sbyte→byte reinterpret of attack1's bonus (Player.cs:690): a
        // "negative" bonus passed as the byte the accessor yields (e.g. -1 → 255)
        // is added as 255, never clamped — the faithful quirk. Damage stays >= 0.
        let mut rng = EngineRng::new(SEED);
        let dmg = roll_damage(&mut rng, 1, 1, 255, None); // d1 (=1) + 255
        assert_eq!(dmg.amount, 1 + 255);
    }

    // --- saving throws ------------------------------------------------------

    #[test]
    fn saving_throw_nat1_fails_nat20_succeeds_else_compares() {
        let mut rng = EngineRng::new(SEED);
        let (mut saw1, mut saw20, mut saw_plain) = (false, false, false);
        for _ in 0..2000 {
            let s = roll_saving_throw(&mut rng, 0, 0, 11); // target 11, no bonus
            match s.d20 {
                1 => {
                    assert!(!s.made, "nat-1 always fails");
                    saw1 = true;
                }
                20 => {
                    assert!(s.made, "nat-20 always succeeds");
                    saw20 = true;
                }
                d => {
                    assert_eq!(s.made, d as i32 >= 11, "plain roll compares vs target");
                    saw_plain = true;
                }
            }
            if saw1 && saw20 && saw_plain {
                break;
            }
        }
        assert!(saw1 && saw20 && saw_plain);
    }

    #[test]
    fn saving_throw_applies_bonus_and_field_186() {
        let mut rng = EngineRng::new(SEED);
        for _ in 0..200 {
            let s = roll_saving_throw(&mut rng, 3, -1, 15);
            if (2..=19).contains(&s.d20) {
                assert_eq!(s.made, (s.d20 as i32 + 3 - 1) >= 15);
            }
        }
    }

    // --- resolve_attack: the full to-hit → damage tie, draw-faithful -------

    #[test]
    fn resolve_attack_hit_draws_d20_then_damage_and_emits_both_events() {
        assert!(
            Replay::new(SEED).roll(20) > 1,
            "the hit case needs the seed's first d20 to not be a nat-1"
        );

        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let actions = ActionLog::default();
        let mut sink = actions.sink();

        // AC 0 + hitBonus 40: the first roll (>1) certainly hits.
        let p = AttackProfile {
            attacker_id: 2,
            target_id: 7,
            target_ac: 0,
            hit_bonus: 40,
            team_bonus: 0,
            dice_size: 6,
            dice_count: 2,
            damage_bonus: 1,
            backstab: None,
        };
        let out = resolve_attack(&mut rng, p, Some(&mut *sink));
        assert!(out.to_hit.hit);

        // Exactly: one d20, then two d6 (damage) — the hit-branch draw shape.
        assert_eq!(log.ns(), vec![20, 6, 6]);

        let mut o = Replay::new(SEED);
        let d20 = o.roll(20);
        let dmg = o.roll(6) + o.roll(6) + 1;
        assert_eq!(out.to_hit.d20 as u16, d20);
        assert_eq!(out.damage.unwrap().amount, dmg as i32);

        let ev = actions.events();
        assert_eq!(ev.len(), 2, "Attack then Dmg");
        assert!(matches!(
            ev[0],
            ActionEvent::Attack {
                attacker_id: 2,
                target_id: 7,
                hit: true,
                ..
            }
        ));
        assert!(matches!(
            ev[1],
            ActionEvent::Dmg {
                attacker_id: 2,
                target_id: 7,
                backstab: false,
                ..
            }
        ));
    }

    #[test]
    fn resolve_attack_miss_draws_only_the_d20_and_emits_no_dmg() {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let actions = ActionLog::default();
        let mut sink = actions.sink();

        // AC 200 is unreachable even by a nat-20 (→100), so every roll misses.
        let p = AttackProfile {
            attacker_id: 0,
            target_id: 1,
            target_ac: 200,
            hit_bonus: 0,
            team_bonus: 0,
            dice_size: 8,
            dice_count: 3,
            damage_bonus: 5,
            backstab: None,
        };
        let out = resolve_attack(&mut rng, p, Some(&mut *sink));
        assert!(!out.to_hit.hit);
        assert!(out.damage.is_none());
        assert_eq!(log.ns(), vec![20], "a miss draws no damage dice");

        let ev = actions.events();
        assert_eq!(ev.len(), 1);
        assert!(matches!(ev[0], ActionEvent::Attack { hit: false, .. }));
    }

    #[test]
    fn resolve_attack_works_without_a_sink() {
        let mut rng = EngineRng::new(SEED);
        let p = AttackProfile {
            attacker_id: 0,
            target_id: 1,
            target_ac: 0,
            hit_bonus: 40,
            team_bonus: 0,
            dice_size: 4,
            dice_count: 1,
            damage_bonus: 0,
            backstab: Some(backstab_multiplier(5)), // ×3
        };
        let out = resolve_attack(&mut rng, p, None);
        assert!(out.to_hit.hit);
        let mut o = Replay::new(SEED);
        let _d20 = o.roll(20);
        let dice = o.roll(4);
        assert_eq!(out.damage.unwrap().amount, dice as i32 * 3);
        assert!(out.damage.unwrap().backstab);
    }

    // === tactical battlefield (M4 combat #3) ==============================

    const FLOOR: u8 = 0x17; // a passable floor tile (move_cost 1)
    const WALL_TILE: u8 = 1; // BACKGROUND_MOVE_COST[1] == 0xFF

    fn place_input(team: Team) -> PlacementInput {
        PlacementInput {
            team,
            size: 1,
            in_combat: true,
        }
    }

    // --- map & passability -------------------------------------------------

    #[test]
    fn map_dimensions_are_50_by_25() {
        assert_eq!((MAP_W, MAP_H), (50, 25));
        assert_eq!(BACKGROUND_MOVE_COST.len(), 74);
    }

    #[test]
    fn tile_passability_decodes_move_cost_and_the_void_sentinel() {
        // Tile 0 is the void sentinel regardless of BACKGROUND_MOVE_COST[0].
        assert_eq!(tile_passability(0), TilePassability::Void);
        // Index 1 is move_cost 0xFF → wall.
        assert_eq!(BACKGROUND_MOVE_COST[1], 0xFF);
        assert_eq!(tile_passability(1), TilePassability::Wall);
        // A normal floor (0x17), heavy terrain (0x1A=26 → mc 2, 0x3C=60 → mc 4).
        assert_eq!(
            tile_passability(0x17),
            TilePassability::Passable { move_cost: 1 }
        );
        assert_eq!(
            tile_passability(26),
            TilePassability::Passable { move_cost: 2 }
        );
        assert_eq!(
            tile_passability(60),
            TilePassability::Passable { move_cost: 4 }
        );
        // Out-of-table index → wall (defensive).
        assert_eq!(tile_passability(200), TilePassability::Wall);
    }

    #[test]
    fn map_reads_are_bounds_safe() {
        let mut map = CombatMap::uniform(FLOOR);
        assert_eq!(
            map.passability(GridPos::new(10, 10)),
            TilePassability::Passable { move_cost: 1 }
        );
        // Out-of-bounds → void ground, 0xFF move cost, no occupant.
        assert_eq!(map.ground_tile(GridPos::new(-1, 0)), 0);
        assert_eq!(
            map.passability(GridPos::new(MAP_W, 0)),
            TilePassability::Void
        );
        assert_eq!(map.move_cost(GridPos::new(0, MAP_H)), 0xFF);
        assert_eq!(map.occupant(GridPos::new(-5, -5)), 0);
        // A stamped wall reads back as a wall.
        map.set_tile(GridPos::new(3, 3), WALL_TILE);
        assert_eq!(map.passability(GridPos::new(3, 3)), TilePassability::Wall);
    }

    #[test]
    fn size_footprint_matches_the_steps_table() {
        let p = GridPos::new(4, 7);
        assert!(size_footprint(0, p).is_empty(), "size 0 occupies no cell");
        assert_eq!(size_footprint(1, p), vec![GridPos::new(4, 7)]);
        assert_eq!(
            size_footprint(4, p),
            vec![
                GridPos::new(4, 7),
                GridPos::new(5, 7),
                GridPos::new(4, 8),
                GridPos::new(5, 8),
            ]
        );
    }

    // --- placement: exact positions ---------------------------------------

    /// The canonical layout: 3 party + 3 monsters, party facing north (dir 0),
    /// enemies 1 tile ahead, on all-floor ground. The exact cells below are the
    /// transliteration's output; member 0 is re-derived by hand in the doc comment
    /// as the worked example.
    ///
    /// **Worked example — party member 0** (`place_combatant`, team 0,
    /// `team_direction=0`, `team_start=(0,0)`):
    /// - iteration 1, tri-state `start`: `half_dir = DIRECTION_165FC[0][0]/2 = 0`;
    ///   `iso_dir = HALF_DIR_TO_ISO[2] = 3`, `delta=(1,1)`;
    ///   `base = (UNK_16610[0], UNK_16618[0]) = (5,3)`, `row_scale=0` → `cur=(5,3)`.
    /// - `cur=(5,3)` is in range; `valid[0][0][3][5]` is set (row 3 of `UNK_16620[0]`
    ///   is `[2,9]`, so col 5 is valid); ground is floor, unoccupied → placed.
    /// - iso transform: `pos.x = 5 + 0·6 + 0·5 + 22 = 27`,
    ///   `pos.y = 3 + 0·5 + 10 = 13` → **(27, 13)**.
    #[test]
    fn placement_exact_positions_party_north() {
        let mut map = CombatMap::uniform(FLOOR);
        let roster: Vec<PlacementInput> = (0..3)
            .map(|_| place_input(Team::Party))
            .chain((0..3).map(|_| place_input(Team::Monster)))
            .collect();
        let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);

        let cells: Vec<(i32, i32)> = p.iter().map(|c| (c.pos.x, c.pos.y)).collect();
        assert_eq!(
            cells,
            vec![
                (27, 13), // party 0 — hand-derived above
                (28, 13), // party 1
                (28, 14), // party 2
                (22, 7),  // monster 0
                (21, 7),  // monster 1
                (21, 6),  // monster 2
            ]
        );
        assert!(p.iter().all(|c| c.placed), "all six find a cell");
    }

    // --- provisional area terrain (D2) ------------------------------------

    /// A `0x402`-byte GEO payload with the named squares fully enclosed (all
    /// four wall nibbles nonzero); every other square is fully open. Mirrors
    /// the plane layout `gbx_formats::geo` documents (NE plane packs N high /
    /// E low at offset 2; SW plane packs S high / W low at offset 2+256).
    fn synthetic_geo_with_walled_squares(cells: &[(usize, usize)]) -> GeoBlock {
        const PLANE_NE: usize = 2;
        const PLANE_SW: usize = 2 + 256;
        let mut data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        for &(gx, gy) in cells {
            let i = gx + 16 * gy;
            data[PLANE_NE + i] = (3 << 4) | 3; // N=3, E=3
            data[PLANE_SW + i] = (3 << 4) | 3; // S=3, W=3
        }
        GeoBlock::parse(&data).unwrap()
    }

    #[test]
    fn provisional_map_stamps_fully_walled_squares_as_rock() {
        // (0,0) fully walled → rock at (17,3); (1,0) only partially walled →
        // stays floor.
        let mut data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        data[2] = (3 << 4) | 3; // sq (0,0): N=3,E=3
        data[2 + 256] = (3 << 4) | 3; // sq (0,0): S=3,W=3
        data[2 + 1] = 3 << 4; // sq (1,0): N=3 only (not enclosed)
        let geo = GeoBlock::parse(&data).unwrap();

        let map = provisional_combat_map(&geo);
        assert!(
            matches!(map.passability(GridPos::new(17, 3)), TilePassability::Wall),
            "a fully-walled GEO square becomes a rock obstacle"
        );
        assert!(
            matches!(
                map.passability(GridPos::new(18, 3)),
                TilePassability::Passable { .. }
            ),
            "a partially-walled square stays open floor"
        );
        // A cell nowhere near any wall is open floor.
        assert!(matches!(
            map.passability(GridPos::new(45, 20)),
            TilePassability::Passable { .. }
        ));
    }

    #[test]
    fn provisional_map_keeps_the_deployment_core_clear() {
        // Square (5,5) maps to (22,8), which lands INSIDE the deployment core
        // (x 20..=30, y 6..=16) — so even though it is fully walled, the core
        // re-clear stamps it back to floor and the roster can deploy there.
        let geo = synthetic_geo_with_walled_squares(&[(5, 5)]);
        let map = provisional_combat_map(&geo);
        assert!(
            matches!(
                map.passability(GridPos::new(22, 8)),
                TilePassability::Passable { .. }
            ),
            "the deployment core is re-cleared over any wall"
        );
        // And the whole party origin (27,13) region places.
        let roster: Vec<PlacementInput> = (0..3)
            .map(|_| place_input(Team::Party))
            .chain((0..3).map(|_| place_input(Team::Monster)))
            .collect();
        let mut map = map;
        let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);
        assert!(p.iter().all(|c| c.placed), "everyone finds a cell");
    }

    // --- encounter runner (D3) --------------------------------------------

    fn weak_goblin() -> LoadedMonster {
        use crate::monster::MonsterAttack;
        LoadedMonster {
            name: "GOB".to_string(),
            hit_dice: 1,
            hit_point_max: 3,
            ac: 10,
            thac0: 20,
            turn_undead_type: 0,
            monster_type: 3,
            control_morale: 0x80,
            movement: 6,
            attacks: [
                MonsterAttack {
                    attacks: 1,
                    dice_count: 1,
                    dice_size: 2,
                    damage_bonus: 0,
                },
                MonsterAttack {
                    attacks: 0,
                    dice_count: 0,
                    dice_size: 0,
                    damage_bonus: 0,
                },
            ],
        }
    }

    fn strong_party_member() -> PartyCombatStats {
        PartyCombatStats {
            hp: 40,
            raw_ac: 54, // displayed AC -18, near-untouchable
            hit_bonus: 50,
            movement: 12,
            dice: (2, 8, 5),
            npc: false,
        }
    }

    #[test]
    fn encounter_distance_wilderness_is_2() {
        let geo = synthetic_geo_with_walled_squares(&[]);
        assert_eq!(encounter_distance(&geo, 0, 5, 5, false), 2);
    }

    #[test]
    fn encounter_distance_dungeon_ray_walks_open_cells_and_stops_at_a_wall() {
        // Open everywhere: the ray walks its full 2 cells.
        let open = synthetic_geo_with_walled_squares(&[]);
        assert_eq!(encounter_distance(&open, 2, 5, 5, true), 2);
        // A wall on the east edge of the party's own cell blocks immediately.
        let mut data = vec![0u8; gbx_formats::geo::GEO_BLOCK_SIZE];
        data[2 + (5 + 16 * 5)] = 0x03; // sq (5,5): E nibble = 3 (wall)
        let walled = GeoBlock::parse(&data).unwrap();
        assert_eq!(encounter_distance(&walled, 2, 5, 5, true), 0);
    }

    #[test]
    fn run_encounter_party_beats_a_weak_monster() {
        let geo = synthetic_geo_with_walled_squares(&[]);
        let map = provisional_combat_map(&geo);
        let party = vec![strong_party_member(), strong_party_member()];
        let monsters = vec![weak_goblin()];
        let mut rng = EngineRng::new(0x0C0F_FEE0);
        let result = run_encounter(&party, &monsters, map, 0, 1, &mut rng);
        assert_eq!(result.outcome, CombatOutcome::PartyWins);
        assert!(result.rounds >= 1, "at least one round resolved");
    }

    /// Local-tier: the real Tilverton City block (`GEO2.DAX` block 1) derives
    /// a provisional field with the invariants the wiring relies on — the
    /// deployment core is fully passable, and it is real GEO data (at least
    /// one rock cell is stamped from the block's enclosed squares).
    #[test]
    fn provisional_map_from_real_geo2_block1_invariants() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!(
                "SKIPPED: provisional_map_from_real_geo2_block1_invariants needs GBX_DATA_DIR"
            );
            return;
        };
        let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
            .expect("GBX_DATA_DIR must be readable");
        let geo = GeoBlock::parse(&data.block("GEO2.DAX", 1).expect("GEO2.DAX block 1 loads"))
            .expect("GEO2 block 1 parses");
        let map = provisional_combat_map(&geo);

        for y in 6..=16 {
            for x in 20..=30 {
                assert!(
                    matches!(
                        map.passability(GridPos::new(x, y)),
                        TilePassability::Passable { .. }
                    ),
                    "deployment core cell ({x},{y}) must be passable"
                );
            }
        }
        let rocks = (0..MAP_H)
            .flat_map(|y| (0..MAP_W).map(move |x| GridPos::new(x, y)))
            .filter(|&p| matches!(map.passability(p), TilePassability::Wall))
            .count();
        assert!(rocks > 0, "real GEO2 block 1 stamps at least one rock cell");
        eprintln!("GEO2 block 1 → {rocks} rock cell(s) on the provisional field");
    }

    #[test]
    fn placement_offsets_monsters_along_the_facing_direction() {
        // East (dir 2): monsters end up at larger x than the party; south (dir 4):
        // larger y. The team origin shift is encounter_distance · facing.
        let roster: Vec<PlacementInput> = (0..3)
            .map(|_| place_input(Team::Party))
            .chain((0..3).map(|_| place_input(Team::Monster)))
            .collect();

        for (dir, enc, axis) in [(2u8, 2i32, 'x'), (4, 1, 'y')] {
            let mut map = CombatMap::uniform(FLOOR);
            let p = place_combatants(&mut map, &roster, dir, enc, GridPos::new(0, 0), None);
            assert!(p.iter().all(|c| c.placed), "dir {dir}: all placed");
            let party_mean: i32 = (0..3)
                .map(|i| if axis == 'x' { p[i].pos.x } else { p[i].pos.y })
                .sum::<i32>()
                / 3;
            let mon_mean: i32 = (3..6)
                .map(|i| if axis == 'x' { p[i].pos.x } else { p[i].pos.y })
                .sum::<i32>()
                / 3;
            assert!(
                mon_mean > party_mean,
                "dir {dir}: monsters should be ahead along {axis} (party {party_mean}, mon {mon_mean})"
            );
        }
    }

    #[test]
    fn placement_cells_are_distinct_and_on_passable_ground() {
        let mut map = CombatMap::uniform(FLOOR);
        let roster: Vec<PlacementInput> = (0..6)
            .map(|_| place_input(Team::Party))
            .chain((0..6).map(|_| place_input(Team::Monster)))
            .collect();
        let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);

        assert!(p.iter().all(|c| c.placed), "a 6v6 all fits");
        let mut seen = std::collections::HashSet::new();
        for c in &p {
            assert!(
                seen.insert((c.pos.x, c.pos.y)),
                "no two combatants share a cell"
            );
            assert!(
                matches!(map.passability(c.pos), TilePassability::Passable { .. }),
                "every combatant stands on passable ground: {:?}",
                c.pos
            );
        }
    }

    #[test]
    fn placement_skips_a_walled_cell() {
        // Wall off party member 0's natural cell (27,13); it must land elsewhere,
        // still on passable ground, and the fan-out still places everyone.
        let mut map = CombatMap::uniform(FLOOR);
        map.set_tile(GridPos::new(27, 13), WALL_TILE);
        let roster: Vec<PlacementInput> = (0..3)
            .map(|_| place_input(Team::Party))
            .chain((0..1).map(|_| place_input(Team::Monster)))
            .collect();
        let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);

        assert!(p.iter().all(|c| c.placed));
        assert_ne!(
            (p[0].pos.x, p[0].pos.y),
            (27, 13),
            "the walled cell is skipped"
        );
        assert!(matches!(
            map.passability(p[0].pos),
            TilePassability::Passable { .. }
        ));
    }

    #[test]
    fn placement_paints_occupancy_by_one_based_index() {
        let mut map = CombatMap::uniform(FLOOR);
        let roster: Vec<PlacementInput> = (0..3)
            .map(|_| place_input(Team::Party))
            .chain((0..3).map(|_| place_input(Team::Monster)))
            .collect();
        let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);
        for (i, c) in p.iter().enumerate() {
            assert_eq!(
                map.occupant(c.pos),
                (i + 1) as u16,
                "cell {:?} is owned by combatant {} (1-based)",
                c.pos,
                i + 1
            );
        }
    }

    // --- movement / facing / distance -------------------------------------

    #[test]
    fn calc_moves_clamps_then_doubles() {
        assert_eq!(calc_moves(12), 24); // in range → ×2
        assert_eq!(calc_moves(1), 2);
        assert_eq!(calc_moves(96), 192);
        assert_eq!(calc_moves(0), 2, "< 1 collapses to 1 → 2 half-moves");
        assert_eq!(
            calc_moves(97),
            2,
            "the faithful quirk: > 96 also collapses to 1"
        );
    }

    #[test]
    fn step_cost_diagonal_is_x3_orthogonal_x2_and_offmap_is_none() {
        let map = CombatMap::uniform(FLOOR); // move_cost 1 everywhere
        let from = GridPos::new(25, 12);
        // East (dir 2, even → orthogonal): dest (26,12), cost 1·2.
        assert_eq!(step_cost(&map, from, 2), Some((GridPos::new(26, 12), 2)));
        // NE (dir 1, odd → diagonal): dest (26,11), cost 1·3.
        assert_eq!(step_cost(&map, from, 1), Some((GridPos::new(26, 11), 3)));
        // Off the top edge → None (the MapInBounds guard).
        assert_eq!(step_cost(&map, GridPos::new(0, 0), 0), None);
    }

    #[test]
    fn step_cost_into_a_wall_is_huge() {
        let mut map = CombatMap::uniform(FLOOR);
        map.set_tile(GridPos::new(26, 12), WALL_TILE); // move_cost 0xFF
                                                       // Orthogonal into the wall: 0xFF · 2.
        assert_eq!(
            step_cost(&map, GridPos::new(25, 12), 2),
            Some((GridPos::new(26, 12), 0xFF * 2))
        );
    }

    #[test]
    fn deduct_move_zeroes_on_overspend() {
        assert_eq!(deduct_move(10, 3), 7);
        assert_eq!(deduct_move(2, 3), 0, "can't half-finish a step");
        assert_eq!(deduct_move(3, 3), 0);
    }

    #[test]
    fn target_direction_classifies_the_eight_octants() {
        let o = GridPos::new(10, 10);
        // y grows downward, so "north" is a smaller y.
        assert_eq!(target_direction(o, GridPos::new(10, 5)), 0, "N");
        assert_eq!(target_direction(o, GridPos::new(15, 5)), 1, "NE");
        assert_eq!(target_direction(o, GridPos::new(15, 10)), 2, "E");
        assert_eq!(target_direction(o, GridPos::new(15, 15)), 3, "SE");
        assert_eq!(target_direction(o, GridPos::new(10, 15)), 4, "S");
        assert_eq!(target_direction(o, GridPos::new(5, 15)), 5, "SW");
        assert_eq!(target_direction(o, GridPos::new(5, 10)), 6, "W");
        assert_eq!(target_direction(o, GridPos::new(5, 5)), 7, "NW");
    }

    // --- the combat camera (doc §36.3) -------------------------------------

    /// A minimal melee roster over an open floor for exercising camera state.
    fn camera_state(positions: &[(Team, GridPos)]) -> CombatState {
        let fighters = positions
            .iter()
            .enumerate()
            .map(|(i, (team, pos))| {
                Combatant::new_melee(
                    i,
                    *team,
                    *team == Team::Monster,
                    *pos,
                    10,
                    10,
                    0,
                    12,
                    (1, 6, 0),
                    5,
                    1,
                )
            })
            .collect();
        CombatState::new(CombatMap::uniform(FLOOR), fighters)
    }

    #[test]
    fn turn_head_resets_the_actors_own_swarm_and_guard_state() {
        // sub_33281 @028F-02A9: the acting combatant's own AttacksReceived (@028F),
        // directionChanges (@029C), and guarding (@02A9) zero at its turn head,
        // before the body.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(20, 12)),
            (Team::Monster, GridPos::new(21, 12)), // adjacent → attackable
        ]);
        s.combat_setup();
        s.combat_setup_done = true;
        s.fighters[0].attacks_received = 3; // stale swarm count
        s.fighters[0].direction_changes = 5; // stale facing accumulator
        s.fighters[0].guarding = true; // a parked guard
        s.fighters[0].delay = 5;
        s.fighters[0].attack1_left = 1;
        s.fighters[0].target = Some(1);
        let mut rng = EngineRng::new(0x1234_5678);
        s.take_turn(&mut rng, 0);
        // Cleared at the head; the actor attacked (RecalcAttacksReceived bumps
        // the TARGET, not the actor) so its own count stays 0 and it did not
        // re-guard.
        assert_eq!(s.fighters[0].attacks_received, 0);
        assert_eq!(s.fighters[0].direction_changes, 0);
        assert!(!s.fighters[0].guarding);
    }

    #[test]
    fn recalc_accumulates_direction_changes_mod_8() {
        // sub_3F94D @194D-19D8: each swing bumps AttacksReceived and folds a
        // dirDiff into directionChanges = (directionChanges + dirDiff) % 8.
        // Target at (20,12) faces EAST (direction 2); attacker adjacent due WEST
        // at (19,12). At distance 1 the fixed-point octant classifier floors due
        // west to SW=5 (`lo(1)` = 0), so bearing target→attacker = 5. dirDiff =
        // (5 − 2 + 8) % 8 = 3 (≤ 4, no fold). Three swings: 3, 6, then 9 % 8 = 1.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(19, 12)), // attacker (idx 0), due west
            (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
        ]);
        assert_eq!(
            target_direction(s.fighters[1].pos, s.fighters[0].pos),
            5,
            "adjacent-west classifies SW at distance 1"
        );
        s.fighters[1].direction = 2; // faces east
        s.fighters[1].direction_changes = 0;
        s.fighters[1].attacks_received = 0;

        s.recalc_attacks_received(1, 0);
        assert_eq!(s.fighters[1].attacks_received, 1);
        assert_eq!(s.fighters[1].direction_changes, 3, "first: (0 + 3) % 8");

        s.recalc_attacks_received(1, 0);
        assert_eq!(s.fighters[1].attacks_received, 2);
        assert_eq!(s.fighters[1].direction_changes, 6, "second: (3 + 3) % 8");

        s.recalc_attacks_received(1, 0);
        assert_eq!(
            s.fighters[1].direction_changes, 1,
            "third: (6 + 3) % 8 = 9 % 8 wraps"
        );
    }

    #[test]
    fn recalc_direction_diff_folds_above_four() {
        // dirDiff > 4 folds to 8 − dirDiff (@1996-19A8). Target faces N (0);
        // attacker at bearing SW (5) from the target → raw dirDiff = (5 − 0 + 8)
        // % 8 = 5, folded to 3.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(5, 15)), // attacker (idx 0): SW of target
            (Team::Monster, GridPos::new(10, 10)), // target (idx 1)
        ]);
        // Sanity: bearing target→attacker is SW (5).
        assert_eq!(
            target_direction(s.fighters[1].pos, s.fighters[0].pos),
            5,
            "SW"
        );
        s.fighters[1].direction = 0; // faces north
        s.fighters[1].direction_changes = 0;
        s.recalc_attacks_received(1, 0);
        assert_eq!(s.fighters[1].direction_changes, 3, "folded 5 → 8 − 5 = 3");
    }

    /// A roster for the AttackTarget facing table: attacker (idx 0) due NORTH of
    /// target (idx 1), two cells apart → bearing target→attacker = N (0), attacker
    /// faces target = S (4). `on_screen` picks whether the target is inside the
    /// 7×7 window.
    fn facing_state(target_on_screen: bool) -> CombatState {
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(20, 10)), // attacker (idx 0), due north
            (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
        ]);
        s.fighters[0].in_combat = true;
        s.fighters[1].in_combat = true;
        s.focus = false; // isolate the direction stores from any recenter
        s.map_screen_top_left = if target_on_screen {
            GridPos::new(17, 9) // window x17..23, y9..15 → both on-screen
        } else {
            GridPos::new(40, 20) // window x40..46 → target (20,12) off-screen
        };
        // Sanity on the geometry.
        assert_eq!(
            target_direction(GridPos::new(20, 12), GridPos::new(20, 10)),
            0
        );
        assert_eq!(
            target_direction(GridPos::new(20, 10), GridPos::new(20, 12)),
            4
        );
        s
    }

    #[test]
    fn attack_target_facing_first_attack_on_screen_faces_the_attacker() {
        // Table row 1 (§36.1): AttacksReceived<2, attackType 0, on-screen — the
        // face-away store is overwritten by the on-screen draw → target FACES its
        // attacker (bearing target→attacker = 0). Attacker faces target (4).
        let mut s = facing_state(true);
        assert!(s.on_screen(1));
        s.fighters[1].attacks_received = 1; // < 2 (post-Recalc bump)
        s.fighters[1].direction = 3; // arbitrary prior facing
        s.attack_target_facing(1, 0, false);
        assert_eq!(s.fighters[1].direction, 0, "target faces its attacker");
        assert_eq!(s.fighters[0].direction, 4, "attacker faces its target");
    }

    #[test]
    fn attack_target_facing_first_attack_off_screen_faces_away() {
        // Table row 2: AttacksReceived<2, attackType 0, off-screen — no draw, so
        // the face-away store stands → (bearing + 4) % 8 = 4.
        let mut s = facing_state(false);
        assert!(!s.on_screen(1));
        s.fighters[1].attacks_received = 1;
        s.fighters[1].direction = 3;
        s.attack_target_facing(1, 0, false);
        assert_eq!(s.fighters[1].direction, 4, "target faces away");
        assert_eq!(
            s.fighters[0].direction, 4,
            "attacker still faces its target"
        );
    }

    #[test]
    fn attack_target_facing_subsequent_attack_on_screen_is_a_no_op() {
        // Table row 3: AttacksReceived>=2, attackType 0, on-screen — the 180° flip
        // is stored then the draw restores the old value → unchanged.
        let mut s = facing_state(true);
        s.fighters[1].attacks_received = 2; // not < 2
        s.fighters[1].direction = 3;
        s.attack_target_facing(1, 0, false);
        assert_eq!(
            s.fighters[1].direction, 3,
            "subsequent on-screen: unchanged"
        );
        assert_eq!(s.fighters[0].direction, 4);
    }

    #[test]
    fn attack_target_facing_behind_attack_never_changes_the_target() {
        // Table row 5: attackType != 0 (a departure/behind swing) — branch 2, no
        // flip stored (attackType-gated), draw restores → target unchanged, even
        // on the first attack and on-screen.
        let mut s = facing_state(true);
        s.fighters[1].attacks_received = 1; // would be branch 1 if attackType 0
        s.fighters[1].direction = 3;
        s.attack_target_facing(1, 0, true);
        assert_eq!(
            s.fighters[1].direction, 3,
            "attackType != 0: target unchanged"
        );
        assert_eq!(
            s.fighters[0].direction, 4,
            "but the attacker still faces it"
        );
    }

    #[test]
    fn attack_target_facing_subsequent_attack_off_screen_is_a_no_op() {
        // Table row 4: AttacksReceived>=2, off-screen — no store, no draw.
        let mut s = facing_state(false);
        s.fighters[1].attacks_received = 2;
        s.fighters[1].direction = 3;
        s.attack_target_facing(1, 0, false);
        assert_eq!(
            s.fighters[1].direction, 3,
            "subsequent off-screen: unchanged"
        );
        assert_eq!(s.fighters[0].direction, 4);
    }

    #[test]
    fn flanking_fires_only_when_swarmed_turned_and_backs_the_attacker() {
        // §36.4 (ovr014:16AD-16E9): AttacksReceived>1 && the target's back is to
        // the attacker (target_direction(attacker,target) == target.direction) &&
        // directionChanges>4. Attacker (idx 0) due north of target (idx 1) → the
        // attacker's bearing toward the target is S (4); the target faces S (4) to
        // present its back.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(20, 10)), // attacker (idx 0), due north
            (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
        ]);
        assert_eq!(
            target_direction(GridPos::new(20, 10), GridPos::new(20, 12)),
            4
        );
        // All three satisfied → flanking.
        s.fighters[1].attacks_received = 2;
        s.fighters[1].direction = 4; // back turned to the attacker
        s.fighters[1].direction_changes = 5;
        assert!(s.is_flanking(1, 0));

        // Only one swing this turn → not swarmed.
        s.fighters[1].attacks_received = 1;
        assert!(!s.is_flanking(1, 0), "AttacksReceived must be > 1");
        s.fighters[1].attacks_received = 2;

        // Target faces the attacker (N=0), not away → not behind.
        s.fighters[1].direction = 0;
        assert!(
            !s.is_flanking(1, 0),
            "target must face away from the attacker"
        );
        s.fighters[1].direction = 4;

        // Not spun enough this turn.
        s.fighters[1].direction_changes = 4;
        assert!(!s.is_flanking(1, 0), "directionChanges must be > 4");
    }

    #[test]
    fn can_backstab_needs_a_thief_a_listed_weapon_a_swarmed_manzised_turned_target() {
        // §36.4 (ovr014:28D7-29B9): SkillLevel(Thief)>0 && weapon∈list &&
        // AttacksReceived>1 && (field_DE&0x7F)<=1 && back turned. Same geometry as
        // flanking: attacker (idx 0) due north → back-turned target faces S (4).
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(20, 10)),   // attacker (idx 0)
            (Team::Monster, GridPos::new(20, 12)), // target (idx 1)
        ]);
        s.fighters[0].thief_skill_level = 5; // TRAVIS's T5
        s.fighters[0].weapon_readied = false; // punching → null weapon → capable
        s.fighters[1].attacks_received = 2;
        s.fighters[1].field_de = 0x01; // man-sized
        s.fighters[1].direction = 4; // back turned
        assert!(s.can_backstab(1, 0), "thief, bare hands, swarmed, turned");

        // Not a thief.
        s.fighters[0].thief_skill_level = 0;
        assert!(!s.can_backstab(1, 0), "SkillLevel(Thief) must be > 0");
        s.fighters[0].thief_skill_level = 5;

        // Large target (field_DE & 0x7F > 1).
        s.fighters[1].field_de = 0x02;
        assert!(!s.can_backstab(1, 0), "(field_DE & 0x7F) must be <= 1");
        s.fighters[1].field_de = 0x01;

        // Only one swing this turn.
        s.fighters[1].attacks_received = 1;
        assert!(!s.can_backstab(1, 0), "AttacksReceived must be > 1");
        s.fighters[1].attacks_received = 2;

        // A readied weapon NOT in the list (short bow 44) fails; a dagger (8) is
        // in the list.
        s.fighters[0].weapon_readied = true;
        s.fighters[0].loadout = Some(Loadout {
            primary_type: 44, // short bow — not a backstab weapon
            ammo_count: 10,
            unarmed_profile: (1, 2, 3),
        });
        assert!(!s.can_backstab(1, 0), "a readied short bow can't backstab");
        s.fighters[0].loadout = Some(Loadout {
            primary_type: 8, // dagger — in the list
            ammo_count: 0,
            unarmed_profile: (1, 2, 3),
        });
        assert!(s.can_backstab(1, 0), "a readied dagger can backstab");

        // Back NOT turned (faces the attacker, N=0).
        s.fighters[1].direction = 0;
        assert!(
            !s.can_backstab(1, 0),
            "the target's back must be to the attacker"
        );
    }

    #[test]
    fn entry_init_facing_faces_the_party_heading_enemies_reversed() {
        // ovr011.cs:803-807: direction = HalfDirToIso[md/2]; enemies +4%8.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(26, 12)),
            (Team::Monster, GridPos::new(34, 13)),
        ]);
        s.map_direction = 2; // md/2 = 1 → HalfDirToIso[1] = 2.
        s.combat_setup();
        assert_eq!(s.fighters[0].direction, 2, "party faces the heading");
        assert_eq!(s.fighters[1].direction, 6, "enemy faces back (+4)");
        // md = 0 → HalfDirToIso[0] = 7; enemy 3.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(26, 12)),
            (Team::Monster, GridPos::new(34, 13)),
        ]);
        s.map_direction = 0;
        s.combat_setup();
        assert_eq!(s.fighters[0].direction, 7);
        assert_eq!(s.fighters[1].direction, 3);
    }

    #[test]
    fn entry_init_facing_survives_an_out_of_range_map_direction() {
        // `HalfDirToIso` has 4 entries and `map_direction` is a `pub u8` fed by a
        // capture field or `RESTRIKE_MAP_DIR` (which parses any u8), so the
        // `md / 2` index must be masked — an out-of-range heading must not panic.
        for md in [8u8, 9, 200, 255] {
            let mut s = camera_state(&[
                (Team::Party, GridPos::new(26, 12)),
                (Team::Monster, GridPos::new(34, 13)),
            ]);
            s.map_direction = md;
            s.combat_setup(); // must not panic
            let party = HALF_DIR_TO_ISO[(md as usize / 2) % 4] as u8;
            assert_eq!(s.fighters[0].direction, party, "md={md}");
            assert_eq!(s.fighters[1].direction, (party + 4) % 8, "md={md}");
        }
        // The four well-formed headings are unaffected by the mask.
        for (md, want) in [(0u8, 7u8), (2, 2), (4, 3), (6, 6)] {
            let mut s = camera_state(&[(Team::Party, GridPos::new(26, 12))]);
            s.map_direction = md;
            s.combat_setup();
            assert_eq!(s.fighters[0].direction, want, "md={md}");
        }
    }

    #[test]
    fn camera_setup_centres_the_window_on_teamlist0() {
        // BattleSetup (ovr011.cs:1209): mapScreenTopLeft = TeamList[0].pos − (3,3).
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(26, 12)),
            (Team::Monster, GridPos::new(34, 13)),
        ]);
        s.combat_setup();
        assert_eq!(s.map_screen_top_left, GridPos::new(23, 9));
        // The 7×7 window is [23,29]×[9,15]: the party member is on-screen; the
        // monster at x=34 (as in combat4) starts off-screen — the camera matters.
        assert!(s.on_screen(0));
        assert!(!s.on_screen(1));
    }

    #[test]
    fn coord_on_screen_is_the_seven_by_seven_window() {
        assert!(CombatState::coord_on_screen(0, 0));
        assert!(CombatState::coord_on_screen(6, 6));
        assert!(!CombatState::coord_on_screen(-1, 3));
        assert!(!CombatState::coord_on_screen(7, 3));
        assert!(!CombatState::coord_on_screen(3, 7));
    }

    #[test]
    fn screen_map_check_clamps_the_centre_to_the_map_interior() {
        // The clamp bounds (ovr033.cs:286-311): centre.x ∈ [3,46], centre.y ∈ [3,21].
        let mut s = camera_state(&[(Team::Party, GridPos::new(10, 10))]);
        s.map_screen_top_left = GridPos::new(0, 0); // centre (3,3)
        assert!(s.screen_map_check(0xFF, GridPos::new(90, 90)));
        let centre = GridPos::new(
            s.map_screen_top_left.x + SCREEN_HALF,
            s.map_screen_top_left.y + SCREEN_HALF,
        );
        assert_eq!(centre, GridPos::new(46, 21), "clamps to the far corner");
        assert!(s.screen_map_check(0xFF, GridPos::new(-50, -50)));
        let centre = GridPos::new(
            s.map_screen_top_left.x + SCREEN_HALF,
            s.map_screen_top_left.y + SCREEN_HALF,
        );
        assert_eq!(centre, GridPos::new(3, 3), "clamps to the near corner");
    }

    #[test]
    fn screen_map_check_box_test_gates_the_scroll() {
        let mut s = camera_state(&[(Team::Party, GridPos::new(20, 12))]);
        s.map_screen_top_left = GridPos::new(17, 9); // centre (20,12)
                                                     // Inside the radius-2 box → no scroll.
        assert!(!s.screen_map_check(2, GridPos::new(21, 13)));
        assert_eq!(s.map_screen_top_left, GridPos::new(17, 9));
        // Outside the box → the centre steps all the way to `pos` (the while
        // loops chase `pos`, not merely back within radius).
        assert!(s.screen_map_check(2, GridPos::new(24, 12)));
        assert_eq!(s.map_screen_top_left, GridPos::new(21, 9)); // centre (24,12)
    }

    #[test]
    fn draw_74b3f_recenters_an_offscreen_combatant_and_stores_direction() {
        let mut s = camera_state(&[(Team::Party, GridPos::new(40, 20))]);
        s.map_screen_top_left = GridPos::new(0, 0); // (40,20) is far off-screen
        s.focus = true;
        assert!(!s.on_screen(0));
        s.draw_74b3f(0, 5);
        assert!(s.on_screen(0), "the recenter brings it on-screen");
        assert_eq!(
            s.fighters[0].direction, 5,
            "direction stored unconditionally"
        );
        // With focus off, an off-screen combatant is NOT chased (only the store).
        s.map_screen_top_left = GridPos::new(0, 0);
        s.focus = false;
        s.draw_74b3f(0, 2);
        assert!(!s.on_screen(0));
        assert_eq!(s.fighters[0].direction, 2);
    }

    #[test]
    fn departure_attack_turns_focus_on_per_candidate() {
        // Site 7 (`sub_3E954` @`ovr014:0AE5`): each candidate iteration of the
        // departure-attack loop sets `focusCombatAreaOnPlayer = 1`. Without it an
        // off-screen monster mover keeps focus off and the step that follows
        // (`sub_3E748`) skips its focus-gated scrolls.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(20, 12)),   // mover (idx 0)
            (Team::Monster, GridPos::new(19, 12)), // adjacent, will be departed
        ]);
        s.map_screen_top_left = GridPos::new(40, 20); // both off-screen
        s.focus = false;
        assert!(!s.on_screen(0));
        let mut rng = EngineRng::new(SEED);
        // Step EAST (2) to (21,12): distance to (19,12) becomes 2 → departed.
        s.move_step_away_attack(&mut rng, 0, 2);
        assert!(s.focus, "a departure candidate turns the camera focus on");

        // Control: no adjacent enemy → the loop never runs → focus untouched.
        let mut s = camera_state(&[
            (Team::Party, GridPos::new(20, 12)),
            (Team::Monster, GridPos::new(30, 12)), // far away
        ]);
        s.focus = false;
        let mut rng = EngineRng::new(SEED);
        s.move_step_away_attack(&mut rng, 0, 2);
        assert!(!s.focus, "no candidates → no focus write");
    }

    /// Attacker (idx 0) on-screen at the window centre, target (idx 1) far
    /// off-screen to the east, window at (0,0). The attacker readies
    /// `primary_type`. Only the missile camera can move the window here: the
    /// attacker-side `draw_74b3f` recenter needs an OFF-screen attacker, the
    /// target-side draw needs an ON-screen target, and the target survives.
    fn sling_state(primary_type: u8) -> CombatState {
        let attacker = Combatant::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(3, 3),
            30,
            40,
            0,
            12,
            (1, 2, 0),
            5,
            1,
        );
        let target = Combatant::new_melee(
            1,
            Team::Monster,
            true,
            GridPos::new(30, 3),
            200, // survives the swing, so no CombatantKilled scroll
            40,
            0,
            12,
            (1, 2, 0),
            5,
            1,
        );
        let mut s = CombatState::new(CombatMap::uniform(FLOOR), vec![attacker, target]);
        s.item_data = Some(synth_item_table());
        s.set_loadout(
            0,
            Loadout {
                primary_type,
                ammo_count: 40,
                unarmed_profile: (1, 2, 0),
            },
        );
        s.combat_setup_done = true; // skip the lazy setup's camera/facing seed
        s.map_screen_top_left = GridPos::new(0, 0);
        s
    }

    #[test]
    fn a_sling_primary_fires_the_missile_camera_despite_its_null_item() {
        // `sub_3F9DB` @`ovr014:1B14-1B4C`: a Sling (0x2F) / StaffSling (0x65)
        // primary fires a SECOND `sub_40BF1` with the primary itself as the
        // missile. `GetCurrentAttackItem` hands a sling a found-but-NULL item
        // (flags 0x0A, §34.2), so the item-gated first call does not fire for it —
        // without this branch a sling would scroll no camera at all.
        let mut s = sling_state(47); // Sling
        assert!(s.on_screen(0), "attacker on-screen: no attacker recenter");
        assert!(!s.on_screen(1), "target off-screen: no target-side draw");
        let before = s.map_screen_top_left;
        let mut rng = EngineRng::new(SEED);
        // A sling resolves to a null attack item — the melee-shaped call.
        s.attack_target(&mut rng, 0, 1, false, AttackItemRef::None);
        assert_ne!(
            s.map_screen_top_left, before,
            "the sling's own missile scrolls the camera toward the target"
        );

        // Control: a plain range-1 melee primary (type 30) fires no missile, so
        // the window is untouched by an otherwise identical swing.
        let mut s = sling_state(30);
        let before = s.map_screen_top_left;
        let mut rng = EngineRng::new(SEED);
        s.attack_target(&mut rng, 0, 1, false, AttackItemRef::None);
        assert_eq!(
            s.map_screen_top_left, before,
            "a melee primary fires no missile camera"
        );
    }

    /// Two combatants at the given cells with the window at `top_left`, for
    /// exercising [`CombatState::draw_missile_camera`] directly.
    fn missile_state(a: GridPos, t: GridPos, top_left: GridPos) -> CombatState {
        let mut s = camera_state(&[(Team::Party, a), (Team::Monster, t)]);
        s.map_screen_top_left = top_left;
        s
    }

    #[test]
    fn missile_camera_returns_early_on_a_short_path() {
        // `ovr025.cs:910-915`: `var_B0 = var_AF − 2 < 2` (or `var_AF < 2`) →
        // return before any scroll. Adjacent cells span 3 pixels → var_AF = 4 →
        // var_B0 = 2, which does NOT early-return; the same cell gives var_AF = 1.
        // Use a same-cell shot for the guaranteed early return, and assert the
        // window is untouched even though one endpoint is off-screen.
        let mut s = missile_state(
            GridPos::new(30, 12),
            GridPos::new(30, 12),
            GridPos::new(0, 0),
        );
        assert!(
            !s.on_screen_pos(GridPos::new(30, 12)),
            "off-screen endpoint"
        );
        s.draw_missile_camera(0, 1);
        assert_eq!(
            s.map_screen_top_left,
            GridPos::new(0, 0),
            "a short path scrolls nothing"
        );
    }

    #[test]
    fn missile_camera_is_a_no_op_when_both_endpoints_are_on_screen() {
        // `ovr025.cs:934-940`: both on-screen ⇒ `center1` = the current centre, so
        // the force-recenter resolves to the window it already has.
        let mut s = missile_state(
            GridPos::new(18, 10),
            GridPos::new(22, 14),
            GridPos::new(17, 9),
        );
        assert!(s.on_screen(0) && s.on_screen(1));
        s.draw_missile_camera(0, 1);
        assert_eq!(s.map_screen_top_left, GridPos::new(17, 9));
    }

    #[test]
    fn missile_camera_force_scrolls_to_the_midpoint_on_a_short_span() {
        // `ovr025.cs:922-926/940`: an off-screen endpoint with |Δ| ≤ 6 on both
        // axes ⇒ force-scroll to `center1 = Δ/2 + attacker`. Attacker (10,12),
        // target (16,12) → Δ=(6,0) → centre (13,12) → top-left (10,9).
        let mut s = missile_state(
            GridPos::new(10, 12),
            GridPos::new(16, 12),
            GridPos::new(4, 9),
        );
        assert!(!s.on_screen(1), "the target starts off-screen");
        s.draw_missile_camera(0, 1);
        assert_eq!(s.map_screen_top_left, GridPos::new(10, 9), "centre (13,12)");
        assert!(s.on_screen(0) && s.on_screen(1), "both now in the window");
    }

    #[test]
    fn missile_camera_anchors_the_target_on_a_long_span() {
        // `ovr025.cs:1010-1032`: an off-screen endpoint with a span > 6 ⇒ the
        // missile leaves the screen first, so the animation force-scrolls to a
        // target-anchored centre that brings the TARGET into the window.
        let mut s = missile_state(
            GridPos::new(3, 12),
            GridPos::new(40, 12),
            GridPos::new(0, 9),
        );
        assert!(!s.on_screen(1));
        s.draw_missile_camera(0, 1);
        assert!(
            s.on_screen(1),
            "a long shot ends with the target on-screen, not the midpoint"
        );
        // Near the map's right edge the target anchor is pushed back in-bounds by
        // `var_CE`, and `ScreenMapCheck`'s clamp holds the centre at x ≤ 46.
        let mut s = missile_state(
            GridPos::new(3, 12),
            GridPos::new(49, 12),
            GridPos::new(0, 9),
        );
        s.draw_missile_camera(0, 1);
        assert!(
            s.map_screen_top_left.x + SCREEN_HALF < MAP_W - SCREEN_HALF,
            "the centre stays inside ScreenMapCheck's clamp (x <= 46)"
        );
        assert!(s.on_screen(1), "an edge target still lands in the window");
    }

    #[test]
    fn missile_path_pixel_steps_counts_stepping_path_iterations() {
        // A 2-cell horizontal shot spans 6 pixels; Step() moves x 0→6 (6 steps)
        // then the 7th call takes none → var_AF = 7 (var_B0 = 5).
        assert_eq!(
            missile_path_pixel_steps(GridPos::new(0, 0), GridPos::new(2, 0)),
            7
        );
        // A zero-length shot: the first Step() takes none → var_AF = 1 (var_B0 =
        // −1 < 2 ⇒ the missile camera early-returns).
        assert_eq!(
            missile_path_pixel_steps(GridPos::new(5, 5), GridPos::new(5, 5)),
            1
        );
    }

    #[test]
    fn distance_and_adjacency_are_king_moves() {
        assert_eq!(grid_distance(GridPos::new(0, 0), GridPos::new(3, 1)), 3);
        assert_eq!(grid_distance(GridPos::new(5, 5), GridPos::new(5, 5)), 0);
        // Adjacency: the 8 neighbours, not self, not distance 2.
        assert!(is_adjacent(GridPos::new(5, 5), GridPos::new(6, 6)));
        assert!(is_adjacent(GridPos::new(5, 5), GridPos::new(5, 4)));
        assert!(!is_adjacent(GridPos::new(5, 5), GridPos::new(5, 5)));
        assert!(!is_adjacent(GridPos::new(5, 5), GridPos::new(5, 7)));
    }

    #[test]
    fn setup_geometry_is_draw_free() {
        // The whole tactical subsystem must not touch the PRNG (D9). Attach a sink
        // to a shared EngineRng, run placement + movement + facing, assert zero
        // draws.
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());

        let mut map = CombatMap::uniform(FLOOR);
        let roster: Vec<PlacementInput> = (0..3)
            .map(|_| place_input(Team::Party))
            .chain((0..3).map(|_| place_input(Team::Monster)))
            .collect();
        let p = place_combatants(&mut map, &roster, 0, 1, GridPos::new(0, 0), None);
        let _ = calc_moves(12);
        let _ = step_cost(&map, p[0].pos, 2);
        let _ = target_direction(p[0].pos, p[3].pos);
        let _ = grid_distance(p[0].pos, p[3].pos);

        assert_eq!(log.len(), 0, "the setup path draws nothing (D9)");
        // (The rng binding exists only to hold the sink; silence unused warnings.)
        let _ = &mut rng;
    }

    // === wall-respecting range — the Bresenham reach ray (M4 combat #4) =====

    fn rc(team: Team, x: i32, y: i32) -> RangeCombatant {
        RangeCombatant {
            pos: GridPos::new(x, y),
            size: 1,
            team,
        }
    }

    #[test]
    fn reach_ray_open_ground_step_counts() {
        let map = CombatMap::uniform(FLOOR);
        let o = GridPos::new(20, 12);
        // Orthogonal neighbour: 1 step ×2 = 2.
        assert_eq!(reach_ray(&map, o, GridPos::new(21, 12), false).steps, 2);
        // Diagonal neighbour: 2 + 1 = 3.
        assert_eq!(reach_ray(&map, o, GridPos::new(21, 13), false).steps, 3);
        // Distance-2 orthogonal: 4.
        assert_eq!(reach_ray(&map, o, GridPos::new(22, 12), false).steps, 4);
        // 2·max + min: (dx=3,dy=1) → 6+1 = 7.
        assert_eq!(reach_ray(&map, o, GridPos::new(23, 13), false).steps, 7);
        // Symmetric in endpoint order (abs deltas).
        assert_eq!(
            reach_ray(&map, GridPos::new(23, 13), o, false).steps,
            reach_ray(&map, o, GridPos::new(23, 13), false).steps
        );
        // Self: zero steps, reachable.
        let r = reach_ray(&map, o, o, false);
        assert!(r.reach && r.steps == 0);
    }

    #[test]
    fn get_target_range_halves_steps_for_adjacency() {
        let map = CombatMap::uniform(FLOOR);
        let o = GridPos::new(20, 12);
        assert_eq!(
            get_target_range(&map, GridPos::new(21, 12), o),
            1,
            "ortho adj"
        );
        assert_eq!(
            get_target_range(&map, GridPos::new(21, 13), o),
            1,
            "diag adj"
        );
        assert_eq!(get_target_range(&map, GridPos::new(22, 12), o), 2, "dist 2");
        assert_eq!(get_target_range(&map, GridPos::new(24, 12), o), 4, "dist 4");
    }

    #[test]
    fn reach_ray_blocks_on_a_taller_wall_but_ignore_walls_passes() {
        let mut map = CombatMap::uniform(FLOOR); // floor height 1
                                                 // A wall tile (field_2 == 2 > floor height 1) mid-line blocks.
        map.set_tile(GridPos::new(12, 10), WALL_TILE);
        let a = GridPos::new(10, 10);
        let t = GridPos::new(14, 10);
        let blocked = reach_ray(&map, a, t, false);
        assert!(!blocked.reach, "the wall blocks the ray");
        assert_eq!(
            blocked.steps, 4,
            "blocked after reaching the wall cell (2 ortho steps)"
        );
        // Ignoring walls, the full line is traversed: 4 ortho steps ×2 = 8.
        let ignored = reach_ray(&map, a, t, true);
        assert!(ignored.reach);
        assert_eq!(ignored.steps, 8);
        // getTargetRange ignores walls, so it still measures the geometric range.
        assert_eq!(get_target_range(&map, t, a), 4);
        // can_reach reflects the block within budget.
        assert_eq!(can_reach(&map, a, t, 0xff, false), None, "blocked");
        assert_eq!(can_reach(&map, a, t, 0xff, true), Some(8), "wall ignored");
    }

    #[test]
    fn tile_height_tables_are_74_and_match_move_cost_walls() {
        assert_eq!(TILE_HEIGHT.len(), 74);
        assert_eq!(TILE_WALL_HEIGHT.len(), 74);
        // Every impassable wall tile presents a wall taller than the floor height 1.
        for t in 0..74u8 {
            if BACKGROUND_MOVE_COST[t as usize] == 0xFF && TILE_HEIGHT[t as usize] == 1 {
                assert!(
                    TILE_WALL_HEIGHT[t as usize] > 1,
                    "wall tile {t} should block a height-1 attacker"
                );
            }
        }
        // A floor tile (0x17) never blocks a height-1 attacker.
        assert!(TILE_WALL_HEIGHT[0x17] <= TILE_HEIGHT[0x17]);
    }

    #[test]
    fn build_near_targets_filters_team_and_sorts_nearest_first() {
        let map = CombatMap::uniform(FLOOR);
        let combatants = [
            rc(Team::Party, 25, 12),   // 0 = attacker (same team → excluded)
            rc(Team::Monster, 26, 12), // 1 = adjacent (steps 2)
            rc(Team::Monster, 28, 12), // 2 = dist 3 (steps 6)
            rc(Team::Monster, 25, 16), // 3 = dist 4 (steps 8)
            rc(Team::Party, 24, 12),   // 4 = ally (excluded by team filter)
        ];
        let near = build_near_targets(&map, &combatants, 0, 0xff, false);
        let idxs: Vec<usize> = near.iter().map(|n| n.idx).collect();
        assert_eq!(idxs, vec![1, 2, 3], "opposite team only, nearest-first");
        assert_eq!(near[0].steps, 2, "true min steps at large max_range");
        assert_eq!(near[1].steps, 6);
        assert_eq!(near[2].steps, 8);
    }

    #[test]
    fn build_near_targets_range_1_is_melee_adjacency() {
        let map = CombatMap::uniform(FLOOR);
        let combatants = [
            rc(Team::Party, 25, 12),   // attacker
            rc(Team::Monster, 26, 13), // diagonal-adjacent (steps 3 ≤ 1·2+1)
            rc(Team::Monster, 28, 12), // dist 3 (steps 6 > 3) — excluded at range 1
        ];
        let near = build_near_targets(&map, &combatants, 0, 1, false);
        assert_eq!(near.len(), 1, "only the adjacent enemy is near at range 1");
        assert_eq!(near[0].idx, 1);
        // §20 bug #8 (`ovr032:097B`): the binary's best-pair init is 0xFF, not
        // max_range, so the entry stores the REAL steps (3 for a diagonal step)
        // even at range 1 — this is what direction-sorts the range-1 re-pick.
        assert_eq!(near[0].steps, 3);
    }

    #[test]
    fn range_layer_is_draw_free() {
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let map = CombatMap::uniform(FLOOR);
        let combatants = [
            rc(Team::Party, 25, 12),
            rc(Team::Monster, 26, 12),
            rc(Team::Monster, 30, 15),
        ];
        let _ = reach_ray(&map, combatants[0].pos, combatants[1].pos, false);
        let _ = get_target_range(&map, combatants[1].pos, combatants[0].pos);
        let _ = build_near_targets(&map, &combatants, 0, 0xff, false);
        let _ = find_combatant_direction(combatants[1].pos, combatants[0].pos);
        assert_eq!(log.len(), 0, "the range layer draws nothing (D9)");
        let _ = &mut rng;
    }

    // === the field_15 mode-gate (M4 combat #4, deliverable 3 start) =========

    #[test]
    fn field_15_gate_short_circuits_on_0_and_over_4() {
        // §15 bug #1: the entry short-circuit is `field_15 == 0 || field_15 > 4`
        // (binary `cmp 4; ja`), NOT `== 4`. So field_15 ∈ {0} ∪ {5,6,…} skips the
        // d4 gate → exactly TWO draws (d8 then the swapped tail), never three.
        for start in [0u8, 5u8, 6u8, 7u8] {
            let mut oracle = Replay::new(SEED);
            let d8 = oracle.roll(8);
            let tail = if d8 != 8 { 4 } else { 2 }; // swapped branch: d8!=8→d4, d8==8→d2+4

            let log = DrawLog::default();
            let mut rng = EngineRng::new(SEED);
            rng.attach_sink(log.sink());
            let out = field_15_mode_gate(&mut rng, start);
            let ns = log.ns();
            assert_eq!(ns.len(), 2, "field_15={start}: no d4 gate, just d8 + tail");
            assert_eq!(ns[0], 8, "first body draw is the d8");
            assert_eq!(
                ns[1], tail,
                "field_15={start}: d8={d8} → tail d{tail} (d8!=8→d4, d8==8→d2+4)"
            );
            assert!((1..=6).contains(&out), "result in 1..=6, got {out}");
        }
    }

    #[test]
    fn field_15_gate_enters_the_body_when_over_4_gate_is_skipped() {
        // A concrete `field_15 > 4` start (5): the || short-circuits the d4 gate
        // and the body's swapped branch runs. Compare the exact stream + result to
        // an independent replay.
        let mut oracle = Replay::new(SEED);
        let d8 = oracle.roll(8);
        let expected = if d8 != 8 {
            oracle.roll(4)
        } else {
            oracle.roll(2) + 4
        };

        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let out = field_15_mode_gate(&mut rng, 5);
        assert_eq!(log.ns(), vec![8, if d8 != 8 { 4 } else { 2 }]);
        assert_eq!(out as u16, expected, "matches an independent replay");
    }

    #[test]
    fn field_15_gate_draws_the_d4_gate_for_1_through_4() {
        // §15 bug #1: field_15 ∈ 1..=4 evaluates the d4 gate (not short-circuited,
        // since it is neither 0 nor > 4). One d4 gate draw always; if it rolls 1 →
        // the 2-draw body follows (3 total); else just the gate (1 draw, value kept).
        let mut oracle = Replay::new(SEED);
        let gate = oracle.roll(4); // the first draw the gate will make

        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let out = field_15_mode_gate(&mut rng, 3);
        let ns = log.ns();
        assert_eq!(ns[0], 4, "the gate's d4 is the first draw");
        if gate == 1 {
            assert_eq!(ns.len(), 3, "gate==1 → body follows");
            assert_eq!(ns[1], 8);
            assert!((1..=6).contains(&out));
        } else {
            assert_eq!(ns.len(), 1, "gate!=1 → only the gate draws");
            assert_eq!(out, 3, "field_15 unchanged when the gate doesn't fire");
        }
    }

    #[test]
    fn field_15_gate_distribution_stays_in_range_and_respects_the_branch() {
        // Over many entries via the persistent field_15, every produced value is
        // 1..=6 and honors the §15-corrected branch: entry short-circuits on
        // `0 || >4`, and the body draws d4(1..4) when d8!=8 / d2+4(5..6) when d8==8.
        // Re-derive each gate with an independent replay to check the branch.
        let mut rng = EngineRng::new(SEED);
        let mut oracle = Replay::new(SEED);
        let mut field_15 = 0u8;
        for _ in 0..500 {
            let entered = field_15 == 0 || field_15 > 4 || {
                let g = oracle.roll(4);
                g == 1
            };
            let expected = if entered {
                let d8 = oracle.roll(8);
                if d8 != 8 {
                    oracle.roll(4)
                } else {
                    oracle.roll(2) + 4
                }
            } else {
                field_15 as u16
            };
            field_15 = field_15_mode_gate(&mut rng, field_15);
            assert_eq!(field_15 as u16, expected, "matches an independent replay");
            assert!((1..=6).contains(&field_15) || !entered);
        }
    }

    // === the melee AI turn — the parity artifact (M4 combat #4, D3/D6) =======

    #[test]
    fn melee_turn_adjacent_draws_the_exact_sequence() {
        // A monster (NPC) adjacent to a PC: mode-gate → the two behavior-guard d7s
        // → find_target pick (d1) → attack (d20 + damage on a hit). The exact
        // operand sequence AND values are hand-derived from an INDEPENDENT replay
        // (not the engine), so this is a real parity assertion (study §4.1.7).
        let dice = (2u8, 6u8, 1u8); // 2d6+1
        let mut world = CombatWorld::new(
            CombatMap::uniform(FLOOR),
            vec![
                Fighter::new_melee(
                    0,
                    Team::Monster,
                    true,
                    GridPos::new(25, 12),
                    20,
                    5,
                    20,
                    12,
                    dice,
                    5,
                    1,
                ),
                Fighter::new_melee(
                    1,
                    Team::Party,
                    false,
                    GridPos::new(26, 12),
                    20,
                    5,
                    0,
                    12,
                    (1, 4, 0),
                    5,
                    1,
                ),
            ],
        );

        // Independent replay → the expected (operand) stream, branch-following.
        let mut o = Replay::new(SEED);
        let mut expect: Vec<u16> = Vec::new();
        // field_15 gate: field_15 starts 0 → the || short-circuits the d4 gate.
        // §15 bug #1 swapped branch: d8!=8 → d4 (1..4); d8==8 → d2+4 (5..6).
        let d8 = o.roll(8);
        expect.push(8);
        if d8 != 8 {
            o.roll(4);
            expect.push(4);
        } else {
            o.roll(2);
            expect.push(2);
        }
        // wand-scan d7 (normal area), memorized-spell d7 (unconditional).
        o.roll(7);
        expect.push(7);
        o.roll(7);
        expect.push(7);
        // find_target: one target, d1 pick.
        o.roll(1);
        expect.push(1);
        // §18 bug #6: a monster attacker's held target is on the party team, so
        // the target-validity check drops it (ovr010:0F36 `cmp combat_team, 0`)
        // and it re-picks among adjacent PCs — one adjacent enemy → a d1 re-pick.
        o.roll(1);
        expect.push(1);
        // attack: one d20 to-hit; damage dice on a hit.
        let d20 = o.roll(20);
        expect.push(20);
        let effective = if d20 == 20 { 100 } else { d20 as i32 };
        let hit = d20 > 1 && effective + 20 >= 5; // hit_bonus 20 vs raw AC 5
        if hit {
            for _ in 0..dice.0 {
                o.roll(dice.1 as u16);
                expect.push(dice.1 as u16);
            }
        }

        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        world.melee_ai_turn(&mut rng, 0);

        assert_eq!(
            log.ns(),
            expect,
            "the melee turn's exact draw operand sequence"
        );
        assert_eq!(world.fighters[0].target, Some(1), "target was picked");
        assert_eq!(world.fighters[0].delay, 0, "turn spent (delay zeroed)");
        assert!(
            (1..=6).contains(&world.fighters[0].field_15),
            "field_15 updated"
        );
        if hit {
            assert!(
                world.fighters[1].hp_current < 20,
                "the PC took damage on a hit"
            );
        }
    }

    #[test]
    fn monster_approach_draws_a_d100_per_step_but_a_pc_does_not() {
        // The control asymmetry (§4.1.4): an NPC approaching a distant target draws
        // the morale-advance d100 on each step; a PC in the identical geometry
        // short-circuits it and draws none. Both still close and attack.
        for npc in [true, false] {
            let (a_team, t_team) = if npc {
                (Team::Monster, Team::Party)
            } else {
                (Team::Party, Team::Monster)
            };
            let mut world = CombatWorld::new(
                CombatMap::uniform(FLOOR),
                vec![
                    Fighter::new_melee(
                        0,
                        a_team,
                        npc,
                        GridPos::new(25, 8),
                        30,
                        5,
                        20,
                        12,
                        (1, 4, 2),
                        5,
                        1,
                    ),
                    Fighter::new_melee(
                        1,
                        t_team,
                        !npc,
                        GridPos::new(25, 12),
                        30,
                        5,
                        20,
                        12,
                        (1, 4, 2),
                        5,
                        1,
                    ),
                ],
            );
            let log = DrawLog::default();
            let mut rng = EngineRng::new(SEED);
            rng.attach_sink(log.sink());
            let start = world.fighters[0].pos;
            world.melee_ai_turn(&mut rng, 0);

            let d100s = log.ns().iter().filter(|&&n| n == 100).count();
            if npc {
                assert!(d100s >= 1, "an NPC draws a morale d100 per approach step");
            } else {
                assert_eq!(d100s, 0, "a PC never draws the morale-advance d100");
            }
            assert_ne!(
                world.fighters[0].pos, start,
                "the actor moved toward the target"
            );
            assert!(
                log.ns().contains(&20),
                "and eventually swung (a d20 to-hit)"
            );
        }
    }

    #[test]
    fn all_ai_1v1_fight_is_deterministic_terminates_and_is_prng_consistent() {
        // The D6 artifact (turn level): two adjacent all-AI combatants trade blows
        // over rounds until one falls. Same seed → byte-identical draw stream
        // (determinism); a victor emerges (termination); and every captured draw
        // reproduces through an independent `Prng` (before→result→after chain).
        fn run_fight(seed: u32) -> (Vec<RngDraw>, usize) {
            let log = DrawLog::default();
            let mut rng = EngineRng::new(seed);
            rng.attach_sink(log.sink());
            let mut world = CombatWorld::new(
                CombatMap::uniform(FLOOR),
                vec![
                    Fighter::new_melee(
                        0,
                        Team::Monster,
                        true,
                        GridPos::new(25, 12),
                        12,
                        5,
                        20,
                        12,
                        (1, 6, 1),
                        5,
                        1,
                    ),
                    Fighter::new_melee(
                        1,
                        Team::Party,
                        false,
                        GridPos::new(26, 12),
                        12,
                        5,
                        20,
                        12,
                        (1, 6, 1),
                        5,
                        1,
                    ),
                ],
            );
            let mut winner = usize::MAX;
            for _round in 0..100 {
                for actor in 0..2 {
                    if world.fighters[actor].in_combat && world.fighters[actor].delay > 0 {
                        world.melee_ai_turn(&mut rng, actor);
                    }
                }
                let alive: Vec<usize> = (0..2).filter(|&i| world.fighters[i].in_combat).collect();
                if alive.len() <= 1 {
                    winner = *alive.first().unwrap_or(&usize::MAX);
                    break;
                }
                // Initiative stub for the next round: re-arm each survivor's delay +
                // per-round attack (so multi-round trades occur).
                for i in 0..2 {
                    if world.fighters[i].in_combat {
                        world.fighters[i].delay = 5;
                        world.fighters[i].attack1_left = 1;
                        world.fighters[i].attack_idx = 2;
                    }
                }
            }
            let draws = log.draws.borrow().clone();
            (draws, winner)
        }

        let (draws1, w1) = run_fight(SEED);
        let (draws2, w2) = run_fight(SEED);
        assert_eq!(draws1, draws2, "same seed → identical draw stream");
        assert_eq!(w1, w2, "deterministic victor");
        assert_ne!(w1, usize::MAX, "the fight produced a victor");
        assert!(!draws1.is_empty(), "the fight drew from the PRNG");

        // Every draw reproduces through an independent Prng replay of the seed.
        let mut p = Prng::new(SEED);
        for (i, d) in draws1.iter().enumerate() {
            assert_eq!(
                d.before,
                p.state(),
                "draw {i}: before-state matches the replay"
            );
            let r = p.random(d.n.expect("operand recorded"));
            assert_eq!(Some(r), d.result, "draw {i}: result matches the replay");
            assert_eq!(
                d.after,
                p.state(),
                "draw {i}: after-state matches the replay"
            );
        }
    }

    #[test]
    fn run_combat_full_round_loop_is_a_parity_artifact() {
        // The real all-AI round loop (initiative → FindNextCombatant → melee turns):
        // a 2v2 fight run to a decision. Deterministic, terminating, Prng-consistent,
        // and it opens with the round-loop fingerprint — one initiative d6 per
        // combatant before any d100 selection (study §2).
        fn run(seed: u32) -> (Vec<RngDraw>, CombatOutcome, [bool; 4]) {
            let log = DrawLog::default();
            let mut rng = EngineRng::new(seed);
            rng.attach_sink(log.sink());
            let mut world = CombatWorld::new(
                CombatMap::uniform(FLOOR),
                vec![
                    Fighter::new_melee(
                        0,
                        Team::Party,
                        false,
                        GridPos::new(25, 14),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                    Fighter::new_melee(
                        1,
                        Team::Party,
                        false,
                        GridPos::new(26, 14),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                    Fighter::new_melee(
                        2,
                        Team::Monster,
                        true,
                        GridPos::new(25, 12),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                    Fighter::new_melee(
                        3,
                        Team::Monster,
                        true,
                        GridPos::new(26, 12),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                ],
            );
            let outcome = world.run_combat(&mut rng, DEFAULT_NO_ACTION_LIMIT);
            let alive = [
                world.fighters[0].in_combat,
                world.fighters[1].in_combat,
                world.fighters[2].in_combat,
                world.fighters[3].in_combat,
            ];
            let draws = log.draws.borrow().clone();
            (draws, outcome, alive)
        }

        let (draws1, o1, a1) = run(SEED);
        let (draws2, o2, a2) = run(SEED);
        assert_eq!(draws1, draws2, "same seed → identical draw stream");
        assert_eq!((o1, a1), (o2, a2), "deterministic outcome");
        assert!(!draws1.is_empty());

        // The round opens with one d6 per combatant (initiative), before selection.
        let ns: Vec<u16> = draws1.iter().map(|d| d.n.unwrap()).collect();
        assert_eq!(&ns[0..4], &[6, 6, 6, 6], "four initiative d6s open round 0");
        assert_eq!(ns[4], 100, "then the first FindNextCombatant d100");

        // A decisive fight ends with one side wiped; a stalemate leaves both alive.
        let party_alive = a1[0] || a1[1];
        let monsters_alive = a1[2] || a1[3];
        match o1 {
            CombatOutcome::PartyWins => assert!(party_alive && !monsters_alive),
            CombatOutcome::MonstersWin => assert!(!party_alive && monsters_alive),
            CombatOutcome::Stalemate => {}
        }

        // Prng-consistent across the whole fight.
        let mut p = Prng::new(SEED);
        for (i, d) in draws1.iter().enumerate() {
            assert_eq!(d.before, p.state(), "draw {i} before");
            assert_eq!(Some(p.random(d.n.unwrap())), d.result, "draw {i} result");
            assert_eq!(d.after, p.state(), "draw {i} after");
        }
    }

    #[test]
    fn run_combat_driver_matches_raw_step_pumping_draw_for_draw() {
        // Deliverable 3b — the model-unification proof: `run_combat` is now a THIN
        // DRIVER over `step()`, so the tick machine alone must produce the ENTIRE
        // fight. Drive one fight via `run_combat` and an identical one by pumping
        // `step()` straight to `Ended` (a bare `while step() != Ended {}`), and
        // assert the two whole-fight draw streams are byte-identical and the final
        // combatant state matches — the merge added nothing and hid nothing. (This
        // is the "whole-fight draw stream identical whether driven by the driver or
        // the raw tick loop" assertion the brief asks for.)
        fn build() -> CombatState {
            CombatState::new(
                CombatMap::uniform(FLOOR),
                vec![
                    Fighter::new_melee(
                        0,
                        Team::Party,
                        false,
                        GridPos::new(25, 14),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                    Fighter::new_melee(
                        1,
                        Team::Party,
                        false,
                        GridPos::new(26, 14),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                    Fighter::new_melee(
                        2,
                        Team::Monster,
                        true,
                        GridPos::new(25, 12),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                    Fighter::new_melee(
                        3,
                        Team::Monster,
                        true,
                        GridPos::new(26, 12),
                        8,
                        5,
                        20,
                        12,
                        (1, 6, 2),
                        0,
                        1,
                    ),
                ],
            )
        }

        // Path A: the run_combat driver.
        let log_a = DrawLog::default();
        let mut rng_a = EngineRng::new(SEED);
        rng_a.attach_sink(log_a.sink());
        let mut a = build();
        a.run_combat(&mut rng_a, DEFAULT_NO_ACTION_LIMIT);

        // Path B: pump step() directly to Ended (a headless `while step() != Ended`).
        // `new` already defaulted no_action_limit to DEFAULT_NO_ACTION_LIMIT — the
        // same cap run_combat applied — so the two fights share every parameter.
        let log_b = DrawLog::default();
        let mut rng_b = EngineRng::new(SEED);
        rng_b.attach_sink(log_b.sink());
        let mut b = build();
        while b.step(&mut rng_b) != CombatStep::Ended {}

        let draws_a = log_a.draws.borrow().clone();
        let draws_b = log_b.draws.borrow().clone();
        assert!(!draws_a.is_empty(), "the fight drew from the PRNG");
        assert_eq!(
            draws_a, draws_b,
            "run_combat and raw step() pumping draw the exact same whole-fight stream"
        );

        // …and reach the exact same fight (final HP + alive flags across the roster).
        let final_a: Vec<(i32, bool)> = a
            .fighters
            .iter()
            .map(|f| (f.hp_current, f.in_combat))
            .collect();
        let final_b: Vec<(i32, bool)> = b
            .fighters
            .iter()
            .map(|f| (f.hp_current, f.in_combat))
            .collect();
        assert_eq!(final_a, final_b, "identical final combatant state");
    }

    #[test]
    fn ai_action_events_emit_and_are_inert_on_the_draw_stream() {
        // D-OR3: attaching an ActionSink must NOT change the draw stream. Run the
        // same monster-approach turn with and without a sink — identical draws —
        // and confirm the sink saw the pinned ai/morale/move events.
        fn run(with_sink: bool) -> (Vec<u16>, Vec<ActionEvent>) {
            let log = DrawLog::default();
            let mut rng = EngineRng::new(SEED);
            rng.attach_sink(log.sink());
            let actions = ActionLog::default();
            let mut world = CombatWorld::new(
                CombatMap::uniform(FLOOR),
                vec![
                    Fighter::new_melee(
                        0,
                        Team::Monster,
                        true,
                        GridPos::new(25, 8),
                        30,
                        5,
                        20,
                        12,
                        (1, 4, 2),
                        5,
                        1,
                    ),
                    Fighter::new_melee(
                        1,
                        Team::Party,
                        false,
                        GridPos::new(25, 12),
                        30,
                        5,
                        20,
                        12,
                        (1, 4, 2),
                        5,
                        1,
                    ),
                ],
            );
            if with_sink {
                world.attach_action_sink(actions.sink());
            }
            world.melee_ai_turn(&mut rng, 0);
            (log.ns(), actions.events())
        }

        let (ns_plain, _) = run(false);
        let (ns_sunk, events) = run(true);
        assert_eq!(
            ns_plain, ns_sunk,
            "the action sink is inert on the draw stream"
        );

        // The monster resolved a target (ai), checked morale on each step, and moved.
        assert!(
            events.iter().any(|e| matches!(
                e,
                ActionEvent::Ai {
                    combatant_id: 0,
                    target_id: 1,
                    ..
                }
            )),
            "an ai event names the picked target"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                ActionEvent::Morale {
                    combatant_id: 0,
                    ..
                }
            )),
            "a morale event per approach step"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                ActionEvent::Move {
                    combatant_id: 0,
                    ..
                }
            )),
            "a move event per step"
        );
    }

    // --- the combat entry-state replay harness (H4, D-OR5(b)) --------------

    /// A synthetic `0x1A6` record with the combat fields the replay harness reads
    /// poked to the given values — the same offsets [`combatant_from_record`]
    /// reads (D10-clean self-authored bytes). `dice` is `(count, size, bonus)`.
    #[allow(clippy::too_many_arguments)]
    fn synthetic_record(
        name: &[u8],
        hp_cur: u8,
        hp_max: u8,
        raw_ac: u8,
        hit_bonus: u8,
        dex_full: u8,
        hit_dice: u8,
        movement: u8,
        npc: bool,
        attacks_count: u8,
        dice: (u8, u8, u8),
    ) -> Vec<u8> {
        let mut r = vec![0u8; 0x1A6];
        r[0] = name.len() as u8;
        r[1..1 + name.len()].copy_from_slice(name);
        r[0x17] = dex_full; // stats2.Dex.full (== read_stat's `original` byte)
        r[0xe5] = hit_dice; // hit_dice
        r[0xf7] = if npc { 0x80 } else { 0x00 }; // control_morale
        r[0x11c] = attacks_count; // attacksCount (attack_profile_base[0])
        r[0x78] = hp_max; // hit_point_max
        r[0x199] = hit_bonus; // hitBonus
        r[0x19a] = raw_ac; // ac (raw)
        r[0x19c] = 1; // a1 attacks-left (overwritten by initiative)
        r[0x19e] = dice.0; // a1 dice_count
        r[0x1a0] = dice.1; // a1 dice_size
        r[0x1a2] = dice.2; // a1 dmg_bonus
        r[0x1a4] = hp_cur; // hit_point_current
        r[0x1a5] = movement; // movement
        r
    }

    /// D2: `combat_state_from_records` decodes each record, maps the right field
    /// onto each combat input, preserves the snapshot's order + positions (no
    /// `PlaceCombatants`), and produces a full melee fight whose draw stream opens
    /// with exactly one initiative d6 per combatant — the §2 fingerprint, no setup
    /// draw ahead of it. Synthetic records only (D10); the live differential is
    /// the gated milestone test in `gbx-oracle`.
    #[test]
    fn replay_harness_maps_records_and_opens_with_one_d6_per_combatant() {
        use gbx_rules::adnd1::flavor_impl::Adnd1;
        use gbx_rules::pack::RuleSet;
        let rules = RuleSet::load();
        let flavor = Adnd1::new(&rules);

        // 2 party + 3 monsters, distinct positions, DEX 16 (party) / 10 (monsters).
        let p0 = synthetic_record(b"HERO", 20, 22, 54, 50, 16, 1, 12, false, 2, (1, 8, 0));
        let p1 = synthetic_record(b"MAGE", 12, 12, 48, 46, 15, 1, 12, false, 2, (1, 4, 0));
        let m0 = synthetic_record(b"THUG", 8, 8, 40, 12, 10, 1, 9, true, 2, (1, 6, 0));
        let entries = vec![
            RecordCombatant {
                team: Team::Party,
                pos: GridPos::new(25, 12),
                record: &p0,
            },
            RecordCombatant {
                team: Team::Party,
                pos: GridPos::new(24, 12),
                record: &p1,
            },
            RecordCombatant {
                team: Team::Monster,
                pos: GridPos::new(34, 13),
                record: &m0,
            },
            RecordCombatant {
                team: Team::Monster,
                pos: GridPos::new(35, 13),
                record: &m0,
            },
            RecordCombatant {
                team: Team::Monster,
                pos: GridPos::new(33, 13),
                record: &m0,
            },
        ];

        let state = combat_state_from_records(&entries, CombatMap::uniform(0x17), &flavor).unwrap();
        let roster = state.roster();
        assert_eq!(roster.len(), 5);
        // Order + positions preserved verbatim (no PlaceCombatants).
        assert_eq!(roster[0].pos, GridPos::new(25, 12));
        assert_eq!(roster[2].pos, GridPos::new(34, 13));
        // Field mapping (party member 0).
        assert_eq!(roster[0].team, Team::Party);
        assert!(!roster[0].npc);
        assert_eq!(roster[0].hp_current, 20);
        assert_eq!(roster[0].hp_max, 22);
        assert_eq!(roster[0].ac, 54);
        assert_eq!(roster[0].hit_bonus, 50);
        assert_eq!(roster[0].hit_dice, 1);
        assert_eq!(roster[0].movement, 12);
        assert_eq!(roster[0].attacks_count, 2);
        assert_eq!(roster[0].dice_size, 8);
        assert_eq!(
            roster[0].reaction_adj,
            flavor.dex_reaction_bonus(16) as i8,
            "reaction_adj derived from DEX 16 via the flavor"
        );
        // Monsters are NPCs (per control_morale).
        assert!(roster[2].npc);
        assert_eq!(roster[2].team, Team::Monster);

        // Drive the fight; the first five draws are the initiative d6s (one per
        // combatant), then the d100 selection pass begins — no setup draw leaks in.
        let log = DrawLog::default();
        let mut rng = EngineRng::new(SEED);
        rng.attach_sink(log.sink());
        let mut state = state;
        let _ = state.run_combat(&mut rng, DEFAULT_NO_ACTION_LIMIT);
        let ns = log.ns();
        assert!(ns.len() >= 6, "the fight drew from the PRNG");
        for (i, n) in ns.iter().take(5).enumerate() {
            assert_eq!(*n, 6, "draw #{i} must be an initiative d6");
        }
        assert_eq!(ns[5], 100, "the d100 selection pass follows the 5 d6s");
    }

    // --- stub tripwires (doc §24: the M5 ledger names itself) ---------------

    /// Every deliberately-stubbed original mechanic must EMIT when reached, so
    /// a replay that wanders into unmodeled territory produces a named finding
    /// instead of a silent divergence. Three wires: `0-hd-sweep`
    /// (try_sweep_attack vs hit_dice 0), `surrender-int5` (flee_check's omitted
    /// Int branch), `memorized-spells` (sub_3560B's unmodeled selection loop).
    /// The `downed-pc` wire was retired once the downed-PC path was built
    /// (§26/§27); this test also pins that downing a party member no longer trips.
    #[test]
    fn stub_tripwires_fire_when_unmodeled_mechanics_are_reached() {
        #[derive(Clone, Default)]
        struct Trips(Rc<RefCell<Vec<(usize, &'static str)>>>);
        impl ActionSink for Trips {
            fn on_action(&mut self, e: ActionEvent) {
                if let ActionEvent::StubTripped { combatant_id, stub } = e {
                    self.0.borrow_mut().push((combatant_id, stub));
                }
            }
        }

        let mk = |team, npc, pos, movement| {
            Fighter::new_melee(0, team, npc, pos, 30, 5, 20, movement, (1, 4, 2), 5, 1)
        };
        let mut world = CombatWorld::new(
            CombatMap::uniform(FLOOR),
            vec![
                {
                    let mut f = mk(Team::Party, false, GridPos::new(25, 12), 12);
                    f.id = 0;
                    f
                },
                {
                    let mut f = mk(Team::Monster, true, GridPos::new(26, 12), 12);
                    f.id = 1;
                    f
                },
                // A fast opposing monster so flee_check's `max_opp > own/2` else
                // branch (the surrender wire) is reachable for fighter 1.
                {
                    let mut f = mk(Team::Monster, true, GridPos::new(30, 12), 12);
                    f.id = 2;
                    f
                },
            ],
        );
        let trips = Trips::default();
        world.attach_action_sink(Box::new(trips.clone()));

        // 1. downing a party member: no longer trips (the downed-pc wire was
        // retired, §26/§27). Overkill 99 ≫ 9 → dead, out of combat, tile stamped.
        world.apply_damage(0, 99);
        assert!(!world.fighters[0].in_combat);
        assert_eq!(world.fighters[0].health_status, HealthStatus::Dead);
        assert_eq!(
            world.map.ground_tile(GridPos::new(25, 12)),
            TILE_DOWN_PLAYER
        );

        // 2. 0-hd-sweep: a 0-HD target reaches the stubbed sweep guard.
        world.fighters[2].hit_dice = 0;
        assert!(!world.try_sweep_attack(2, 1));

        // 3. surrender-int5: an NPC whose fastest opponent outruns half its own
        // moves lands in the binary's Int>5 surrender branch. Party fighter 0 is
        // down, so make the survivor fast via a fresh party opponent. fighter 1 is
        // an NPC (control_morale 0x80 → the faithful gate-2 seed is 0, so gate 1
        // passes via `== 0`); enemy_health_pct 5 < 100 − field_58C(0) → gate 2
        // passes; max_opp = calc_moves(48)/2 = 48 > calc_moves(12)/2 = 12 → the
        // surrender fork.
        world.fighters[0].in_combat = true; // revive the opponent for the ladder
        world.fighters[0].movement = 48;
        world.enemy_health_pct = 5;
        world.area_field_58c = 0;
        assert!(!world.flee_check(1));

        // 4. memorized-spells: an NPC caster with memorized slots runs a turn
        // (the `control_morale >= 0x80` arm of the sub_3560B gate).
        world.fighters[1].memorized_spells = 2;
        let mut rng = EngineRng::new(SEED);
        world.melee_ai_turn(&mut rng, 1);

        // 4b. the sub_3560B PC gates (`ovr010:0682-0692`): a PARTY caster with
        // memorized slots draws nothing while `AutoPCsCastMagic` is off
        // (capture-proven: bar-fists-2 closes with two memorized slots and zero
        // spell draws, doc §33) — the wire stays silent; the toggle arms it.
        let pc_trips = |trips: &Trips| {
            trips
                .0
                .borrow()
                .iter()
                .filter(|(id, s)| *id == 0 && *s == "memorized-spells")
                .count()
        };
        // Fighter 1's turn above re-killed the negative-hp fighter 0 — restore
        // him to a real live PC before running HIS turns.
        world.fighters[0].in_combat = true;
        world.fighters[0].hp_current = 30;
        world.fighters[0].health_status = HealthStatus::Okey;
        world.fighters[0].memorized_spells = 1;
        world.melee_ai_turn(&mut rng, 0);
        assert_eq!(pc_trips(&trips), 0, "PC + magic OFF must not trip");
        world.auto_pcs_cast_magic = true;
        world.melee_ai_turn(&mut rng, 0);
        assert_eq!(pc_trips(&trips), 1, "PC + magic ON must trip");

        let got: Vec<&'static str> = trips.0.borrow().iter().map(|(_, s)| *s).collect();
        assert!(
            !got.contains(&"downed-pc"),
            "the downed-pc wire was retired (§26/§27): {got:?}"
        );
        assert!(got.contains(&"0-hd-sweep"), "trips: {got:?}");
        assert!(got.contains(&"surrender-int5"), "trips: {got:?}");
        assert!(got.contains(&"memorized-spells"), "trips: {got:?}");
    }

    /// **Bug #12 pinned** — `FleeCheck_001`'s gate 2 is an UNSIGNED 16-bit `jb`
    /// over `100 − area2.field_58C` computed as a 16-bit `sub` (`sub_3637F`
    /// @`ovr010:1473`/`:1481`), so a `field_58C > 100` underflows the threshold to
    /// ~0xFFxx and the gate is **always true** — where coab's signed int makes it
    /// always false. This pins the always-true behavior: with a monster at 100%
    /// enemy-health (a morale that a *signed* threshold `100 − 150 = −50` would
    /// reject), a `field_58C = 150` still lets the ladder proceed to the speed fork
    /// and set `moral_failure`. The `field_58C = 50` contrast (signed==unsigned in
    /// range) rejects the same morale, proving it is the wrap, not the value.
    #[test]
    fn flee_check_gate2_field_58c_over_100_is_always_true_bug12() {
        // fighter 0: a slow party opponent (so the speed fork takes the flee
        // branch, not surrender). fighter 1: the acting NPC monster (control_morale
        // 0x80 → morale seed 0 → gate 1 passes via `== 0`; full HP).
        let slow = Fighter::new_melee(
            0,
            Team::Party,
            false,
            GridPos::new(25, 12),
            30,
            5,
            20,
            1,
            (1, 4, 2),
            5,
            1,
        );
        let fast_npc = Fighter::new_melee(
            1,
            Team::Monster,
            true,
            GridPos::new(26, 12),
            30,
            5,
            20,
            96,
            (1, 4, 2),
            5,
            1,
        );
        let mut world = CombatWorld::new(CombatMap::uniform(FLOOR), vec![slow, fast_npc]);
        // 100% enemy health → after gate 1, monster_morale = 100. A *signed*
        // `100 − field_58C` at field_58C > 100 is negative, so `100 < negative`
        // would be false; the unsigned wrap makes it true.
        world.enemy_health_pct = 100;

        // field_58C = 150 (> 100): gate 2 is always-true (the underflow) → the
        // speed fork sets moral_failure (max_opp = calc_moves(1)/2 = 1 ≤
        // calc_moves(96)/2 = 96 → the flee branch).
        world.area_field_58c = 150;
        assert!(
            !world.flee_check(1),
            "the flee fork returns false (not surrender)"
        );
        assert!(
            world.fighters[1].moral_failure,
            "field_58C > 100 underflows gate 2 to always-true (bug #12), so the ladder \
             proceeds and sets moral_failure even at 100% enemy health"
        );

        // Contrast: field_58C = 50 (≤ 100, signed == unsigned) rejects the same
        // 100% morale at gate 2 (`100 < 100 − 50 = 50` is false) → no flee.
        world.area_field_58c = 50;
        assert!(!world.flee_check(1));
        assert!(
            !world.fighters[1].moral_failure,
            "field_58C ≤ 100 gates normally: 100% enemy health does not rout"
        );
    }
}
