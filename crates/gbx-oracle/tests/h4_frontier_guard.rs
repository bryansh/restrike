//! **The frontier-pin regression guard.** A committed manifest of the exact H4
//! replay outcome for every local capture — FIFTEEN closed captures (the full
//! bar/terrain matrix, doc §44; the four campaign-1 sewer rosters, doc
//! §45/§47; the two campaign-2 cleric fights, doc §48/§49; and the campaign-3
//! buffed-party otyugh fight, doc §50) that cannot silently regress.
//!
//! ## The exact-pin rule (read before editing [`PINS`])
//!
//! Each entry is either [`Expect::Closed`] (our engine must replay the capture
//! operand-exact, draw-for-draw, equal length, with **zero** stub trips) or
//! [`Expect::Frontier`]`(N)` — the replay must diverge at **exactly** draw `N`
//! (not `>= N`, not `<= N`). A frontier moves **only** via a deliberate edit to
//! this manifest, made **in the same commit** as the engine fix that earned the
//! move. Both a regression (frontier shrinks / a closed capture diverges) and an
//! unexplained *forward* drift (frontier grows without a manifest edit) fail
//! loudly here. This is the tripwire that keeps "operand-exact" honest across
//! sessions.
//!
//! D10: the manifest holds only capture **basenames** and **draw indices** — no
//! record bytes, no `~/goldbox-data` content, ever. Like [`h4_replay`], the test
//! is local-tier: it loud-skips per-capture when a file is absent, so plain CI
//! (no `GBX_DATA_DIR`) stays green.
//!
//! ## Where the input knobs went (M6a, `combat-visualizer.md` §4)
//!
//! Each `Pin` used to carry this fight's input knobs (`map_direction`,
//! `auto_cast*`, `area_field_6e4`, `continue_battle`) beside its expectation.
//! They now live in **`gbx_oracle::replay::sidecar`**'s committed table, because
//! the M6a reel needs the identical inputs outside a test binary — and a second
//! copy of an input knob is exactly the §30 failure ("every replay must share
//! input knobs, or an instrument replays a different fight"). **The exact-pin
//! rule extends to that table verbatim**: a knob there moves only alongside the
//! fix that earns it, and this guard is still what fails when one drifts. Each
//! pin's comment below keeps the peel history that earned its value.
//!
//! The replay itself now runs through the library-ified path
//! (`replay::reel_input` → `gbx_engine::combat::reel::build_state`), the same
//! one `h4_replay` and the reel use; the equality surface is unchanged:
//! `(before, after)` plus the `Random(n)` **operand** whenever both sides carry
//! one.

use std::path::Path;

use gbx_engine::combat::reel::{self, draws_agree};
use gbx_engine::combat::DEFAULT_NO_ACTION_LIMIT;
use gbx_engine::rng::{EngineRng, RngDraw, RngSink};
use gbx_oracle::replay;
use gbx_oracle::Trace;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::pack::RuleSet;
use std::cell::RefCell;
use std::rc::Rc;

/// The pinned outcome for one capture.
enum Expect {
    /// Replays operand-exact, draw-for-draw, equal length, zero stub trips.
    Closed,
    /// Diverges at **exactly** this draw index (operand or `(before,after)`).
    /// Unconstructed since doc §50 closed the full 15-capture matrix — kept as
    /// the pin type every future capture with an open frontier re-enters
    /// through.
    #[allow(dead_code)]
    Frontier(usize),
}

/// One manifest row: a capture basename and its pinned outcome.
///
/// The fight's *input* knobs (flee heading, the `AutoPCsCastMagic` entry state
/// and '2'-press schedule, `area2.field_6E4`, the "Continue Battle:" answers)
/// live in `gbx_oracle::replay::sidecar`'s committed table — see the module doc.
/// Each row's comment keeps the peel history that earned both its expectation
/// and its knobs.
struct Pin {
    capture: &'static str,
    expect: Expect,
}

/// **The manifest.** Current truth (doc §29). Edit ONLY alongside the fix that
/// changes a frontier — see the module doc's exact-pin rule.
const PINS: &[Pin] = &[
    Pin {
        capture: "combat4.gbxtrace",
        expect: Expect::Closed,
    },
    Pin {
        capture: "combat3+terrain4.gbxtrace",
        expect: Expect::Closed,
    },
    Pin {
        capture: "combat2+terrain4.gbxtrace",
        expect: Expect::Closed,
    },
    Pin {
        // ★ CLOSED (doc §44): the oldest open frontier (@368 since the wire-gates
        // session, doc §29 finding 3) fell to coab≠binary #20 — the near-list
        // team filter runs AFTER the full-roster exchange sort (`near_enermy`
        // @`ovr025:2648-26B2`), so interleaved same-steps ties order differently
        // than coab's filter-during-build. The capture's @365 is a 6-way
        // find_target d6 (field_15 d4 @362 + the two d7 guards precede it) and
        // @368 sits in that turn's movement tail — the same silent-fork
        // signature as caster-bar's @2174/@2176, closed by the same fix.
        capture: "combat+terrain4.gbxtrace",
        expect: Expect::Closed, // was Frontier(368); near-list filter placement (#20)
    },
    Pin {
        // ★ CLOSED 3521/3521 (doc §32): the rout capture replays operand-exact
        // end to end — FleeCheck ladder, behind-AC, target restore (§31 bug
        // #14), and the cross-round guard (§32 bug #15) all landed.
        capture: "bar-rout-58c50.gbxtrace",
        expect: Expect::Closed, // was Frontier(2895); guarding-survives-initiative (§32 bug #15)
    },
    Pin {
        // The armed-slice driver (doc §25 runbook item 3): MATHEW's first turn
        // fires two long-bow shots from range (d20/d6/d20) where our range=1
        // stub (FD-29) walked him across the bar. §34.4/34.5 landed the cornered
        // weapon swap + ammo depletion (TRAVIS's 10-arrow quiver empties → he
        // punches), carrying the frontier to 2019. The facing subsystem (§36)
        // closed the rest: flanking (ovr014:16AD-16E9) landed the swarmed-target
        // behind-AC hits to 2517, and backstab (CanBackStabTarget, ovr014:28D7)
        // landed TRAVIS's cornered-punch kill at 2517 (ac_behind−4, ×3 damage).
        // CLOSED, operand-exact — the last H4 bar-fight frontier.
        capture: "armed-bar.gbxtrace",
        expect: Expect::Closed,
    },
    Pin {
        // The caster driver, fully peeled (doc §41). sub_3560B's selection loop
        // (round-2 3× d1, round-3 10× d1 + accept) AND the Magic Missile cast —
        // the `find_target` targeting d10, the 3× d4 damage (castingLvl 5 → 3+3d4,
        // no save), slot consumption, the AI-turn early return — all replay
        // operand-exact, carrying the frontier @453 → @2176 (a 1723-draw advance;
        // the cast's own d10+3×d4 and ~1150 further melee draws match). §38's
        // flip-window ruling stands: entry-false + a toggle at turn ordinal 16.
        //
        // ★ CLOSED 3517/3517 (doc §44): the @2176 residual was coab≠binary #20 —
        // [13]'s draw-2174 re-pick d5 fell on a 5-way party list whose tie order
        // depends on the near-list build's OTHER-team entries: the binary
        // exchange-sorts every `size > 0` combatant (attacker included, steps 0)
        // and `near_enermy` filters to the opposite team only AFTER the sort
        // (`ovr025:2648-26B2`), so the full-list dance orders the tied trio
        // `[4],[0],[3]` where our party-only sort gave `[0],[3],[4]` — same
        // roll 4, capture picks [3], we picked [4], forking the turn shape at
        // 2176. Sort-then-filter lands the pick on [3] and the remaining ~1341
        // draws replay operand-exact.
        capture: "caster-bar.gbxtrace",
        expect: Expect::Closed, // was Frontier(2176); near-list filter placement (#20)
    },
    Pin {
        // A third independent fist seed, free from the caster staging (doc
        // §33): two memorized-but-inert slots (magic off) — also the
        // capture-proof that the gated memorized-spells wire stays silent.
        capture: "bar-fists-2.gbxtrace",
        expect: Expect::Closed,
    },
    Pin {
        // The first post-matrix capture (doc §45): 5 Fire Knives jump the
        // 6-PC party in a sewer corridor (save-doctor staging, natural seed,
        // 7 rounds). md=0 and 58C=30 ride in-trace (hook channels), so the
        // pin fields mirror them for clarity only. @63 ours draws d100 where
        // the capture opens round-2 initiative d6 — our round 1 owes one
        // extra turn (first board fork: Fire Knife [9]'s approach move).
        // @63→65: the FIRE KNIFE loadout landed (MON2ITM.DAX block 1 — short
        // bow + 7 arrows; doc §45). The @63 fork was never terrain: back-rank
        // [9] THROWS twice from its entry square (the armed-bar d20/d6/d20
        // signature) where our melee-only decode walked it.
        // @65→381: the ready/unready recompute (`reclac_player_values` →
        // `CalculateAttackValues`, ovr025.cs:407-62) — readying installs the
        // weapon's TABLE profile (arrow d6, not the sword's record d8) and
        // rebuilds hitBonus from thac0 (bow = 41 + DexReactionAdj(18) = 44;
        // the constant-hit model let [10]'s capture-hit shot 2 miss @140).
        // armed-bar unshifted by both (its PC records serialized bow-readied).
        // @381→1152: the §38 toggle pin, DERIVED from the recorded
        // magic_toggle flip (doc §44.2 sampling places it between post-entry
        // draws 57 and 58 — the head of global turn 2, [9]'s). Not fitted:
        // round-0 PHILIPPE (ordinal 9) still draws no selection d1s because
        // the round-0 arrow hit dropped `can_cast` (the §45 disruption);
        // his round-1 turn draws the nine d1s at #381.
        //
        // ★ CLOSED 1526/1526 (doc §45): @1152 was coab≠binary #21 — the
        // area movement modifier `area2.field_6E4` is PARTY-gated
        // (`sub_3E124` @`ovr014:0138-014E` tests `combat_team == Ours`; coab
        // misread the gate as `in_combat == false`). The sewers run the party
        // at movement 12−3=9 (an 18-half budget), so the round-5 chase walks
        // stop a step short of the routing Fire Knives — three independent
        // PC walks pin −3 exactly; monsters keep 24 (their 23-cost flee
        // paths matched all along). The remaining ~374 draws replay
        // operand-exact — the first sewer roster proven draw-for-draw.
        capture: "sewer-fight-1.gbxtrace",
        expect: Expect::Closed, // was Frontier(1152); party-gated field_6E4 (#21)
    },
    Pin {
        // The campaign-1 thief brawl (doc §47): the biggest capture yet —
        // 18,185 post-entry draws / 49 rounds / 23 combatants, including the
        // first ALLIED team-0 NPCs (4 guild thieves fighting beside the 6 PCs
        // against 2 Fire Knives + 11 enemy thieves; 58C=60, 6E4=0 — the
        // per-SCRIPT values, all emitted in-trace). Intake frontier @308 =
        // the missing FIRE KNIFE loadout row for this basename (FK [6] stands
        // and shoots where our melee decode walked it) — re-keying the §45
        // MON2ITM kit by NAME across sewer captures carried it to 1336. The
        // THIEF kit is rowless by data (MON2ITM block 2: Dagger readied +
        // leather — no ranged option, entry profile stands).
        // @1336→7938: the §38 toggle pin, DERIVED (h4_toggle_ordinal): the
        // recorded magic_toggle flip lands between post-entry draws 619/620 =
        // exactly the head of global turn 17 ([9]'s pick d100 IS draw 619;
        // the sub_36269 poll sits between it and the turn body). @1336 was
        // PHILIPPE's selection d1s (capture d1 vs our plain-turn d100) —
        // toggles=[17] replays the casting turns and ~6,600 further draws.
        //
        // ★ CLOSED 18185/18185 (doc §47): @7938 was the bandage nonTeamMember
        // gate (`ovr025.cs:1634` — a dying ALLIED thief must not trigger a
        // PC's bandage turn; §26's team==Party simplification finally paid
        // for), and the @9716 length fork was the MOVING stalemate horizon
        // (`ovr014.cs:911`, listing ovr014:19EB-19F3: every AttackTarget sets
        // no_action_limit = combat_round + 15 — invisible in every fight
        // shorter than 15 rounds, load-bearing across this brawl's 49). The
        // fight ends BY the no-action rule, length-equal with the capture.
        capture: "sewer-fight-2.gbxtrace",
        expect: Expect::Closed, // was Frontier(7938); bandage gate + stalemate horizon
    },
    Pin {
        // The campaign-1 troll tussock (doc §47): 4 TROLLS + 7 CROCODILES,
        // 3,042 post-entry draws / 7 rounds (58C=75; the "trolls out walking
        // their crocodiles" random-pool scene, teleport-slot-E staging). Both
        // rosters are ITEMLESS (no MON2ITM blocks — natural attacks straight
        // from the records). The capture's emitted 6E4 = raw word 253 = 0xFD:
        // the ECL stores BYTE −3, bridged to the engine's i32 by
        // `common::area_6e4_from_word` (sub_3E124's word-add + AL-store,
        // listing-verified @ovr014:0140-014E) — the pin records the
        // engine-semantic value. Intake frontier @115: ours draws a d1 where
        // the capture draws a d100. Troll regeneration is UNMODELED — expect
        // the peel to surface it.
        // ★ CLOSED 3042/3042 (doc §47.6/§47.7): the intake @115 was the
        // SIZE/FOOTPRINT slice — trolls are field_DE 0x82 (size 2 = origin +
        // the cell SOUTH) and crocodiles 0x83 (size 3 = origin + EAST), the
        // first multi-cell combatants in any capture; our decode ran them
        // size-1, so every near list / reach / walk was computed against
        // wrong footprints (the §47.3 "impossible d1" = troll [9]'s southern
        // cell making it reach-1 from (26,13)). @937: the §38 toggle,
        // DERIVED toggles=[16] (flip between post-entry draws 513/514 = the
        // head of global turn 16). @978: the dwarf racial hit handlers —
        // dwarf_and_gnome_vs_giants 0x2F (−4 vs size-2 giants/trolls,
        // Type_16) and dwarf_vs_orc 0x1A (+1 vs field_14B&4 targets,
        // Type_10) — the first REAL CallAffectTable handlers, live inside
        // PC_CanHitTarget. @1982: TROLL REGENERATION — the on-hit 0x65
        // cascade (regenerate 0x3B + regen_3_hp 0x62 via the ADD-time
        // dispatch) heals +3 at every round end (Type_19): the capture's [9]
        // survives the punch that killed ours. The party is WIPED in 7
        // rounds — MonstersWin, length-equal, and no troll ever dies (the
        // death-3d6/rise machinery stays tripwired).
        capture: "sewer-fight-3.gbxtrace",
        expect: Expect::Closed, // was Frontier(115); size slice + racials + troll regen
    },
    Pin {
        // The campaign-1 otyugh attack, RESTAGED with QuickFight from turn 1
        // (the first staging had a manual first turn — doc §47.5): 4 OTYUGHS
        // + 1 NEO-OTYUGH (slot F, 1,361 post-entry draws; 58C=75 / 6E4=−3 —
        // the same per-SCRIPT sewer-area pair as the trolls; no '2' press).
        // ★ CLOSED 1361/1361 (doc §47.6): the intake @138 was the NEO-OTYUGH
        // — field_DE 0x82, the size-2 footprint — walking a different
        // anti-oscillation dance than our size-1 model; the size slice
        // closes it outright. The otyugh triple attack (d20/d8, d20/d8,
        // d20/d4) and the whole 5-monster brawl replay draw-for-draw.
        capture: "sewer-fight-4.gbxtrace",
        expect: Expect::Closed, // was Frontier(22) on the manual-turn staging; size slice
    },
    Pin {
        // The campaign-2 opener (doc §48): the slot-H UPGRADED party walks into
        // the 5-FIRE-KNIFE ambush (sewer-fight-1's script — 58C=30 / 6E4=0xFD
        // → −3 / md=4, all emitted in-trace; the monster side is the fully
        // modeled §45 kit, so every new draw is party-side novelty). 960
        // post-entry draws / 5 rounds. SHARA (cleric 5) memorized [3,3,23,23]
        // = 2× CLW + 2× HOLD PERSON; PHILIPPE [15]; the '2' press is recorded
        // between post-entry draws 21 and 22 (before the first turn's body).
        // Intake frontier @26: with no toggle pin PHILIPPE's round-0 turn
        // draws no selection d1s — capture d1 (his 10×d1 MM selection) vs our
        // post-head d100. Count-only runs 960/960 while operands fork here —
        // the purest §14 count-artifact yet.
        // @26→75: the §38 toggle pin, DERIVED (h4_toggle_ordinal): the
        // recorded flip sits between post-entry draws 21/22, and draw 22 IS
        // the head of global turn 0 — PHILIPPE's, whose MM selection + cast
        // (10×d1 + d5 + 3×d4, the §41/§42 machinery on a 5-FK list) then
        // replays through the round-0 Fire Knife sword turn. @75 = Fire
        // Knife [9] (a back-ranker): the capture fires its short bow from its
        // entry square (the §45 archer kit — not yet keyed to this basename)
        // where our kitless decode walked it.
        // @75→97: the §45 FIRE KNIFE kit keyed to this basename (the same
        // MON2ITM block-1 data — kits key by NAME, doc §48); both round-0
        // back-rank archer turns replay. @97 = MATHEW's first swing: the
        // capture readies his long sword +1 (items_selection over the slot-H
        // kit) and rolls d8 where our loadout-less decode punched d2 — the
        // item-plus / party-kit driver.
        // @97→233: the slot-H party kits + the item-plus recompute (doc §48):
        // the two-candidate AI_items_selection (ranged var_4 / melee var_8)
        // readies each PC's best weapon at the turn head, and the faithful
        // sub_66023 recompute installs table dice + str + item plus (damage)
        // and thac0 + dex/str + plus (+1 elf rider, to-hit only — LEDERA).
        // Every round-0 party sword turn replays (MATHEW/TRAVIS +1 hits,
        // MARK's 4-way pick + d1 retarget, LEDERA's walk-kill). @233 was the
        // tail of SHARA's selection: her cleric ids (3/23) were untranscribed,
        // so our loop rejected through every pass (9th d4) where the capture
        // ACCEPTS hold person on pick 8 and moves to the next-turn scan d100.
        //
        // ★ CLOSED 960/960 (doc §48): the cleric spell slice — SpellEntry
        // rows 3 (CLW, priority 1, delay 5/3=1) + 0x17 (hold person, priority
        // 6, 3 targets, DamageOnSave::Zero); the QUEUED delayed cast
        // ("Begins Casting" — ★ coab≠binary #22: the scheduler delay is
        // SUBTRACTED (`sub es:[di+3],al` @ovr014:28BE), not assigned, so the
        // caster's mini-turn wins the very next pick); the multi-target loop
        // (3× find_target d4) + per-target d20 saves (all three passed —
        // nobody held); find_healing_target's self-cure (51→55, one d8);
        // slot consumption visible as the d4→d3→d2 selection-die ladder;
        // the round-stale friends/foe counts (round-3 selection draws with
        // both Fire Knives escaped, round-4 draws none); and the round-3-end
        // "Continue Battle: Y" (occurrence schedule). The full 5-round fight
        // — hold, disruption, cure, double rout, escape, mop-up — replays
        // operand-exact.
        capture: "cleric-fk.gbxtrace",
        expect: Expect::Closed, // was Frontier(233); the cleric spell slice
                                // Bryan answered the round-3-end "Continue Battle:" prompt 'Y' once
                                // (round 4 exists in the capture), 'N' at round 4's end.
    },
    Pin {
        // The campaign-2 second capture (doc §48.5's hunt for a LANDED hold):
        // the REAL guild-war 23-brawl this time — 6 slot-H PCs + 4 ALLIED
        // team-0 THIEFs (roster [10]-[13]) vs 2 FIRE KNIVES + 11 THIEVES.
        // 5,659 post-entry draws / 11 rounds / 578 turn_snapshots; decisive
        // party win. 58C=60 / 6E4=0 / md=2 all emitted in-trace (the
        // per-SCRIPT guild-war values, same as sewer-fight-2); the '2' press
        // is recorded between post-entry draws 153 and 154 (raw draw 224).
        // Intake Frontier(193): the slot-H party kits keyed only to
        // `cleric-fk.gbxtrace` and the §38 toggle was unpinned. The kit
        // keying (slot_h_party_kit by NAME across both cleric basenames)
        // proved draw-neutral to 193; the toggle pin carried 193→228: draw
        // 153 IS the head of global turn 3 ([20]'s — the poll sees the
        // press), turn 4 (head @189) is PHILIPPE's, and @193 was his first
        // selection d1. @228 = TRAVIS's first turn (ordinal 5, head @223):
        // after the head f15 d8+d2, the two d7 guards, and the find_target
        // far pick d12 (both sides pick [9]), the capture WALKS him east
        // and swords blocking [21] (d1/d20/d8) where ours stood and shot.
        // @228→519: the READIED-AMMO gate (§49) — `AI_items_selection`'s
        // `var_1F` reads the readied-arrows SLOT (@0x17D), which the slot-H
        // records serialize EMPTY, so the party's bows can never win the
        // gate and MATHEW/TRAVIS fight as swordsmen. @519 = MARK's first
        // swing at the thief [21]: ours rolled a d20 where the capture
        // killed DRAW-FREE — because [21] was HELD (SHARA's round-0 hold
        // person; the 2×d20 saves at ~490 sit in the matched region and
        // one FAILED — the first landed hold in any capture).
        //
        // ★ CLOSED 5659/5659 (doc §49): the held-target slice — the melee
        // SLAY (`sub_3F4EB` @`ovr014:152C-15E0`: damage = hp+5, no dice,
        // both attack cells zeroed) and the restrained turn skip
        // (`sub_3A071` = clear_actions on the turn-head PlayerRestrained
        // dispatch) — plus the held gates on the departure/into-reach
        // opportunity passes. FOUR held thieves die draw-free ([21]@519,
        // [15]@1258 at FULL hp, [9]@1580, [16]@1801); both memorized CLWs
        // fire, each on an ALLIED thief ([11] 23→24 round 6, [12] 4→8
        // round 10 — find_healing_target's same-TEAM scan covers allies
        // unchanged); the 11-round decisive win replays operand-exact with
        // no Continue-Battle 'Y' (the fight never stalls to the prompt).
        capture: "cleric-guildwar.gbxtrace",
        expect: Expect::Closed, // was Frontier(228); the held-target slice (§49)
                                // The §38 toggle pin, DERIVED (h4_toggle_ordinal): the recorded
                                // magic_toggle flip sits between post-entry draws 153/154, and draw
                                // 153 IS the head of global turn 3 ([20]'s) — the poll at that head
                                // sees the press. Turn 4 (head @189) is PHILIPPE's: his selection
                                // d1s are the @193 fork the intake pin sat on.
    },
    Pin {
        // The campaign-3 capture (doc §46's last gate): the slot-H party —
        // staged from slot I, a position-only teleport clone of the same
        // records/gear — walks into 5 evil OTYUGHS (alignment 0x08, all
        // size-1; not the sewer-fight-4 pack) CAMP-BUFFED: bless (0x01) on
        // all six PCs, protection_from_evil (0x08) on MATHEW ×1 and MARK ×2
        // (a DUPLICATE node — the find-FIRST pin), TRAVIS's dwarf racials
        // (0x61/0x1A/0x2F) and LEDERA's 0x6B riding along. 1,089 post-entry
        // draws / 5 rounds; 58C=75 / 6E4=0xFD→−3 / md=6 all emitted
        // in-trace; the '2' press is recorded between post-entry draws 22
        // and 23. Intake Frontier(29): the slot-H party kits key only to
        // the cleric basenames — LEDERA's first swing (turn 0: walk + d5/d1
        // near-pick + d20 hit) punches d2 where the capture swords d8 (the
        // cleric-guildwar intake signature on a new basename).
        //
        // ★ CLOSED 1089/1089 (doc §50): the kit keying (slot_h_party_kit
        // across all three slot-H basenames — slot I is a position-only
        // teleport clone of the same records) + the derived toggle carried
        // 29 → draw-exact end-to-end; the last 21 stub trips fell to the
        // camp-buff handlers (bless dispatches on all 19 party swings — the
        // listing-verified +1 crosses no boundary roll here; prot-evil's
        // Type_11 writes stay stale with a byte-pinned evil attacker) and
        // to the SURRENDER proof: two slow Int-10 otyughs hit the §28
        // speed-fork branch and "Surrender" (removed unconscious), the
        // fight replaying draw-for-draw through both removals — the
        // `surrender-int5` wire retired capture-proven.
        capture: "buffed-otyugh.gbxtrace",
        expect: Expect::Closed, // was Frontier(29); kits + toggle + camp-buff handlers (§50)
                                // The §38 toggle pin, DERIVED (h4_toggle_ordinal): the recorded flip
                                // sits between post-entry draws 22/23 and draw 22 IS the head of
                                // global turn 0 (LEDERA's) — the head poll sees the press, exactly
                                // the cleric-fk shape.
    },
];

#[derive(Clone, Default)]
struct DrawTap {
    draws: Rc<RefCell<Vec<RngDraw>>>,
}
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.draws.borrow_mut().push(draw);
    }
}

/// Replay a capture and return `(first_divergence_index, trip_count)`.
/// `None` divergence == closed (all draws equal on `(before, after, operand)` and
/// equal length).
///
/// The assembly is the library's (`replay::reel_input` →
/// `reel::build_state`) — the same one `h4_replay` and the M6a reel use. This
/// function keeps only what is the *guard's* job: the two intake asserts, the
/// stub-trip count, and the comparison.
fn replay(path: &Path, capture: &str) -> (Option<usize>, usize) {
    let text = std::fs::read_to_string(path).expect("capture readable");
    let trace = Trace::parse(&text).expect("capture parses");
    let entry = trace
        .combat_entry()
        .expect("capture carries a combat_entry snapshot");
    let sidecar = replay::sidecar_for(capture)
        .unwrap_or_else(|| panic!("{capture}: every pinned capture has a committed sidecar row"));

    // coab≠binary #21 (doc §45): the party-only area movement modifier. The
    // sidecar stays the documented input, but when the capture carries the
    // field (post-cc0c9cd hooks, the doc §46 prep) the two must AGREE — a
    // silent capture-vs-sidecar mismatch would let a stale pin mask the real
    // input. Compared in ONE domain — the engine's — through the §47
    // byte-bridge: the ECL stores a byte (sewer-fight-3 records the raw word
    // 253 = 0xFD), the sidecar records the engine-semantic −3; comparing
    // raw-vs-semantic would fire spuriously on every byte-negative area.
    if let Some(cap_6e4) = entry.area2_field_6e4 {
        assert_eq!(
            replay::area_6e4_from_word(i32::from(cap_6e4)),
            sidecar.knobs.area_field_6e4,
            "{capture}: the sidecar's area_field_6e4 must record the capture's emitted \
             area2_field_6e4 (engine domain, §47 byte-bridge) — edit the sidecar row"
        );
    }
    // The 6E0/6E2 per-team to-hit pair is UNMODELED (parse-only; the team
    // gate needs listing verification first — the #21 lesson). A nonzero
    // pair cannot replay faithfully, so fail loudly at intake rather than
    // surfacing as a baffling d20 fork mid-fight.
    assert_eq!(
        (
            entry.area2_field_6e0.unwrap_or(0),
            entry.area2_field_6e2.unwrap_or(0)
        ),
        (0, 0),
        "{capture}: nonzero area2 to-hit pair (6E0/6E2) is not modeled (doc §46)"
    );

    // §34.1: the ITEMS table + per-capture ranged loadouts ride in the
    // assembly. `None` loadouts leave a combatant melee-identical, so the
    // un-armed pins are unshifted; armed-bar arms MATHEW/TRAVIS's bows.
    // NOTE: no `apply_env_overrides` here — the guard is deliberately hermetic,
    // so no exported RESTRIKE_* variable can quietly move a pin.
    let input = replay::reel_input(capture, entry, &sidecar, replay::load_item_data())
        .unwrap_or_else(|e| panic!("{capture}: {e}"));

    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let mut state = reel::build_state(&input, &flavor).expect("records decode");

    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    let mut rng = EngineRng::new(input.rng_state);
    rng.attach_sink(Box::new(tap));

    // Count stub trips (a closed capture must fire none).
    let trips: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    struct TripTap(Rc<RefCell<usize>>);
    impl gbx_engine::combat::ActionSink for TripTap {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            if let gbx_engine::combat::ActionEvent::StubTripped { combatant_id, stub } = e {
                eprintln!("GUARD-TRIP stub={stub} ci={combatant_id}");
                *self.0.borrow_mut() += 1;
            }
        }
    }
    state.attach_action_sink(Box::new(TripTap(trips.clone())));

    state.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |_, _| {});

    let trip_count = *trips.borrow();
    let ours = draws.borrow();
    let cap = replay::capture_draws(&text);
    let max = ours.len().max(cap.len());
    let mut frontier = None;
    for i in 0..max {
        match (ours.get(i), cap.get(i)) {
            (Some(o), Some(c)) if draws_agree(o, c) => {}
            _ => {
                frontier = Some(i);
                break;
            }
        }
    }
    (frontier, trip_count)
}

#[test]
fn h4_frontier_pins_hold() {
    let Some(dir) = replay::traces_dir() else {
        eprintln!(
            "SKIPPED: frontier guard needs the local traces dir \
             (set GBX_DATA_DIR or GBX_TRACES_DIR; captures are local-only, D10)"
        );
        return;
    };

    let mut checked = 0;
    for pin in PINS {
        let path = dir.join(pin.capture);
        if !path.exists() {
            eprintln!("SKIPPED (absent, D10): {}", pin.capture);
            continue;
        }
        // A ranged capture needs the ITEMS table; without it (D10) the replay
        // would fall back to melee and the pin cannot hold — skip loudly.
        if replay::capture_has_loadout(pin.capture) && replay::load_item_data().is_none() {
            eprintln!("SKIPPED (ITEMS absent, D10): {}", pin.capture);
            continue;
        }
        checked += 1;
        let (frontier, trips) = replay(&path, pin.capture);
        match pin.expect {
            Expect::Closed => {
                assert_eq!(
                    frontier, None,
                    "{}: pinned CLOSED but diverged at draw {frontier:?} \
                     (regression — do NOT edit the pin without the fix that earned it)",
                    pin.capture
                );
                assert_eq!(
                    trips, 0,
                    "{}: pinned CLOSED but {trips} stub trip(s) fired",
                    pin.capture
                );
                eprintln!("OK  {} — CLOSED (operand-exact, 0 trips)", pin.capture);
            }
            Expect::Frontier(n) => {
                assert_eq!(
                    frontier,
                    Some(n),
                    "{}: pinned frontier {n} but diverged at {frontier:?} \
                     (drift — a frontier moves ONLY via a manifest edit in the same \
                     commit as the fix that earned it)",
                    pin.capture
                );
                eprintln!("OK  {} — frontier @{n} (exact)", pin.capture);
            }
        }
    }

    if checked == 0 {
        eprintln!(
            "SKIPPED: no pinned captures present under {}",
            dir.display()
        );
    } else {
        eprintln!("frontier guard: {checked}/{} pins held", PINS.len());
    }
}
