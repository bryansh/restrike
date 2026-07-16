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

use crate::rng::EngineRng;
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

/// One combatant in the round: a stable roster id, a team tag, the (precomputed)
/// DEX reaction adjustment the initiative roll adds, whether it acts this combat,
/// and its live `delay` — the `Action.delay` initiative key (`Action@0x03`).
///
/// This slice carries the *derived combat inputs* the initiative loop needs
/// rather than a full decoded record: initiative reads only `in_combat`, the DEX
/// reaction adjustment, `team`, and `delay` (`ovr014.cs:29-52`). Real
/// construction from a party `Player` / a `LoadedMonster` lands with the
/// `COMBAT`-opcode wiring; the caller assembling the roster owns the records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combatant {
    /// Stable per-encounter roster index (the D-OR3 `combatant_id`).
    pub id: usize,
    /// Party or monster (`Player.combat_team`, `@0x18b`-ish runtime cell).
    pub team: Team,
    /// `DexReactionAdj(player)` (`ovr025.cs:537`) — a table lookup, no draw —
    /// precomputed via `gbx-rules`' `Flavor::dex_reaction_bonus`. Range `-4..=5`.
    pub reaction_adj: i8,
    /// `player.in_combat` (`ovr014.cs:29`): a not-in-combat combatant gets
    /// `delay = 0` and rolls **no** d6.
    pub in_combat: bool,
    /// `action.delay` (`Action@0x03`) — the initiative/turn-order key. Reset each
    /// round by [`CombatState`]; zeroed when the combatant's turn completes.
    pub delay: i8,
}

impl Combatant {
    /// A combatant with a directly-supplied reaction adjustment (the primitive
    /// used by tests with hand-built rosters). Starts with `delay = 0`.
    pub fn new(id: usize, team: Team, reaction_adj: i8, in_combat: bool) -> Self {
        Combatant {
            id,
            team,
            reaction_adj,
            in_combat,
            delay: 0,
        }
    }

    /// A combatant whose reaction adjustment is derived from its Dexterity
    /// through the rules flavor (`DexReactionAdj`, `ovr025.cs:537` — the mapping
    /// lives in `gbx-rules`, not hardcoded here). coab reads `stats2.Dex.full`.
    pub fn from_dex(id: usize, team: Team, dex: u8, in_combat: bool, flavor: &dyn Flavor) -> Self {
        Combatant::new(id, team, flavor.dex_reaction_bonus(dex) as i8, in_combat)
    }
}

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

/// The round loop's state (`MainCombatLoop`, `ovr009.cs:22`): the roster (in
/// `TeamList` order — draw order depends on iteration order, so this ordering is
/// load-bearing), the round counter, the stalemate cap, and the per-round
/// surprise mask. Runs `count → initiative → turns → BattleRoundChecks` as a
/// tick-based skeleton.
pub struct CombatState {
    /// `gbl.TeamList` (`Classes/Gbl.cs:496`) — party then monsters, iteration
    /// order preserved.
    roster: Vec<Combatant>,
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
    /// 0-based `FindNextCombatant` pass index within the current round.
    pass: u32,
    /// The optional action-trace observer (D-OR3). `None` in normal play.
    sink: Option<Box<dyn ActionSink>>,
}

impl CombatState {
    /// Enters combat with a caller-provided roster (party then monsters, in
    /// `TeamList` order). `combat_round` starts at 0 (`BattleSetup`,
    /// `ovr011.cs:1170`); the stalemate cap defaults to
    /// [`DEFAULT_NO_ACTION_LIMIT`].
    pub fn new(roster: Vec<Combatant>) -> Self {
        CombatState {
            roster,
            combat_round: 0,
            no_action_limit: DEFAULT_NO_ACTION_LIMIT,
            surprise_mask: 0,
            phase: Phase::RoundStart,
            pass: 0,
            sink: None,
        }
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

    /// The roster in iteration order (read-only; draw order depends on it).
    pub fn roster(&self) -> &[Combatant] {
        &self.roster
    }

    /// Advances combat by one tick and returns what happened (D8: control
    /// returns each step). See [`CombatStep`].
    pub fn step(&mut self, rng: &mut EngineRng) -> CombatStep {
        match self.phase {
            Phase::Ended => CombatStep::Ended,
            Phase::RoundStart => self.begin_round(rng),
            Phase::Selecting => self.select_or_end(rng),
        }
    }

    /// `MainCombatLoop`'s per-round head (`ovr009.cs:37-44`): count teams, roll
    /// initiative over the whole roster, then clear the surprise mask.
    fn begin_round(&mut self, rng: &mut EngineRng) -> CombatStep {
        // CountCombatTeamMembers + the pre-loop / round-top emptiness guard
        // (ovr009.cs:29-33). With no death model the counts are static, so this
        // only ever short-circuits a roster missing a whole side.
        let (friends, foe) = self.team_counts();
        if friends == 0 || foe == 0 {
            self.phase = Phase::Ended;
            return CombatStep::Ended;
        }

        // Initiative: foreach player in TeamList → CalculateInitiative.
        for i in 0..self.roster.len() {
            self.calculate_initiative(i, rng);
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

    /// `CalculateInitiative(player)` (`ovr014.cs:8`) reduced to its draw-bearing
    /// core: one d6 + DEX reaction adjustment, clamp-to-1, team `-6`, then
    /// out-of-range → 0. (The attack/movement recalculation `CalculateInitiative`
    /// also does is RNG-free — study §3 — and out of scope for this slice.)
    fn calculate_initiative(&mut self, i: usize, rng: &mut EngineRng) {
        let Combatant {
            id,
            team,
            reaction_adj,
            in_combat,
            ..
        } = self.roster[i];

        let (delay, surprise) = if in_combat {
            // action.delay = (sbyte)(roll_dice(6,1) + DexReactionAdj(player))
            let d6 = roll_dice(rng, 6, 1) as i32;
            let mut delay = d6 + reaction_adj as i32;
            // if (action.delay < 1) action.delay = 1;   ← BEFORE the -6
            if delay < 1 {
                delay = 1;
            }
            // if (((combat_team+1) & area2_ptr.field_596) != 0) action.delay -= 6;
            let surprise = ((team as i32 + 1) & self.surprise_mask as i32) != 0;
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

        self.roster[i].delay = delay;
        self.emit(ActionEvent::Init {
            combatant_id: id,
            delay,
            dex_adj: reaction_adj,
            surprise,
        });
    }

    /// One `FindNextCombatant` pass (`ovr009.cs:63-99`): roll one d100 per roster
    /// member, pick per the two-`if` tie-break, and either yield the pick (its
    /// turn) or — on the terminating empty pass (`max_delay == 0`) — run
    /// `BattleRoundChecks`. The terminating pass **still draws its K d100s**
    /// (study §14 landmine 1) before ending the round.
    fn select_or_end(&mut self, rng: &mut EngineRng) -> CombatStep {
        // One d100 per roster member, EVERY pass (dead/zero-delay members
        // included). Draw first, into roster order, so the seam sees exactly K
        // draws for this pass.
        let rolls: Vec<u16> = (0..self.roster.len())
            .map(|_| roll_dice(rng, 100, 1))
            .collect();

        let delays: Vec<i8> = self.roster.iter().map(|c| c.delay).collect();
        let picked = select_combatant(&delays, &rolls);

        let pass = self.pass;
        self.pass += 1;

        match picked {
            Some((idx, roll)) => {
                let id = self.roster[idx].id;
                let delay = self.roster[idx].delay;
                self.emit(ActionEvent::Pick {
                    pass,
                    combatant_id: id,
                    delay,
                    roll,
                });
                // Turn slot (stub): DoPlayerCombatTurn eventually sets
                // action.delay = 0 (ovr010.cs:521 etc.). With no real turn yet we
                // zero it immediately, consuming ZERO draws — so it is not
                // re-picked and the draw stream stays pure initiative.
                self.roster[idx].delay = 0;
                CombatStep::Turn { combatant_id: id }
            }
            None => self.battle_round_checks(),
        }
    }

    /// `BattleRoundChecks` (`ovr009.cs:363`) reduced to its non-stubbed parts:
    /// increment the round counter and decide the loop exit. `step_game_time`,
    /// affect ticks, cloud damage, bleed, and bandage are RNG-free and gated on
    /// systems not in this slice.
    fn battle_round_checks(&mut self) -> CombatStep {
        self.combat_round += 1; // ovr009.cs:366 — the byte_1D8B7 increment
        let (friends, foe) = self.team_counts();
        let battle_over = friends == 0 || foe == 0 || self.combat_round >= self.no_action_limit;
        let round = self.combat_round;
        self.phase = if battle_over {
            Phase::Ended
        } else {
            Phase::RoundStart
        };
        CombatStep::RoundEnded { round, battle_over }
    }

    /// `CountCombatTeamMembers` (`ovr025.cs:1268`) → `(friends_count, foe_count)`.
    /// No death model in this slice, so every roster member counts toward its
    /// team.
    fn team_counts(&self) -> (usize, usize) {
        let mut friends = 0;
        let mut foe = 0;
        for c in &self.roster {
            match c.team {
                Team::Party => friends += 1,
                Team::Monster => foe += 1,
            }
        }
        (friends, foe)
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
        let mut state = CombatState::new(roster);

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
        let mut state = CombatState::new(vec![party(0, -3), monster(9)]).with_surprise_mask(0b01);
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
        let mut state = CombatState::new(roster);

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
        let mut state = CombatState::new(roster);

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
        let mut state = CombatState::new(roster);
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
        let mut state = CombatState::new(vec![party(0, 0), monster(1)]);

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
        let mut state = CombatState::new(vec![party(0, 0), party(1, 0)]);
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
}
