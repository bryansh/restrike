//! **The frontier-pin regression guard.** A committed manifest of the exact H4
//! replay outcome for every local capture — NINE closed captures (the full
//! bar/terrain matrix, doc §44, plus `sewer-fight-1` 1526/1526, doc §45 — the
//! first sewer fight: Fire Knife archers, casting disruption, the party-gated
//! area move modifier #21) that cannot silently regress, plus the campaign-1
//! sewer captures under active peel (doc §47).
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
//! The replay+compare here mirrors `h4_replay`'s milestone machinery deliberately
//! (a compact copy — factoring the milestone test into a shared module would churn
//! it for little gain); the equality surface is identical: `(before, after)` plus
//! the `Random(n)` **operand** whenever both sides carry one.

use std::path::{Path, PathBuf};

mod common;

use gbx_engine::combat::{
    combat_state_from_records, CombatMap, GridPos, RecordCombatant, Team, DEFAULT_NO_ACTION_LIMIT,
};
use gbx_engine::rng::{EngineRng, RngDraw, RngSink};
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
    Frontier(usize),
}

/// One manifest row: capture basename, its pinned outcome, the flee heading
/// (`map_direction`) to apply in-process, and the `AutoPCsCastMagic` input
/// state — the entry value (`auto_cast`, doc §33) plus any mid-fight '2'
/// presses as a toggle schedule (`auto_cast_toggles`, global turn ordinals,
/// doc §38). Only `bar-rout-58c50` routs, so the heading is load-bearing there
/// (md=2, the geometry-matched value, doc §29); for the non-routing captures
/// it is inert but set uniformly for clarity. The magic flag today feeds only
/// the `memorized-spells` wire (no draws), but the manifest carries each
/// fight's true input state so the caster peel inherits it.
struct Pin {
    capture: &'static str,
    expect: Expect,
    map_direction: u8,
    auto_cast: bool,
    auto_cast_toggles: &'static [u32],
    /// `area2.field_6E4` — the PARTY-only area movement modifier (coab≠binary
    /// #21, doc §45). ECL-set per area, not in the capture snapshot (hook
    /// TODO): bar 0, sewers −3 (pinned by three independent chase walks).
    area_field_6e4: i32,
}

/// **The manifest.** Current truth (doc §29). Edit ONLY alongside the fix that
/// changes a frontier — see the module doc's exact-pin rule.
const PINS: &[Pin] = &[
    Pin {
        capture: "combat4.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
    },
    Pin {
        capture: "combat3+terrain4.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
    },
    Pin {
        capture: "combat2+terrain4.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
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
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
    },
    Pin {
        // ★ CLOSED 3521/3521 (doc §32): the rout capture replays operand-exact
        // end to end — FleeCheck ladder, behind-AC, target restore (§31 bug
        // #14), and the cross-round guard (§32 bug #15) all landed.
        capture: "bar-rout-58c50.gbxtrace",
        expect: Expect::Closed, // was Frontier(2895); guarding-survives-initiative (§32 bug #15)
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
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
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
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
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[16],
        area_field_6e4: 0,
    },
    Pin {
        // A third independent fist seed, free from the caster staging (doc
        // §33): two memorized-but-inert slots (magic off) — also the
        // capture-proof that the gated memorized-spells wire stays silent.
        capture: "bar-fists-2.gbxtrace",
        expect: Expect::Closed,
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: 0,
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
        map_direction: 0,
        auto_cast: false,
        auto_cast_toggles: &[2],
        area_field_6e4: -3,
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
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[17],
        area_field_6e4: 0,
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
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[16],
        area_field_6e4: -3,
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
        map_direction: 4,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: -3,
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
        capture: "cleric-fk.gbxtrace",
        expect: Expect::Frontier(26),
        map_direction: 4,
        auto_cast: false,
        auto_cast_toggles: &[],
        area_field_6e4: -3,
    },
];

/// Open-floor fallback tile (matches `h4_replay`) for pre-terrain captures.
const FLOOR: u8 = 0x17;

/// Resolve the traces directory, or `None` when the local tier is not active
/// (neither `GBX_TRACES_DIR` nor `GBX_DATA_DIR` set → plain CI skips, D10).
fn traces_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GBX_TRACES_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("GBX_DATA_DIR")?;
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("goldbox-data/traces"))
}

fn decode_terrain(hex: &str) -> Vec<u8> {
    let b = hex.as_bytes();
    let val = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    b.chunks_exact(2)
        .map(|p| (val(p[0]) << 4) | val(p[1]))
        .collect()
}

#[derive(Clone, Default)]
struct DrawTap {
    draws: Rc<RefCell<Vec<RngDraw>>>,
}
impl RngSink for DrawTap {
    fn on_draw(&mut self, draw: RngDraw) {
        self.draws.borrow_mut().push(draw);
    }
}

/// The capture's post-`combat_entry` draws with the diagnostic operand
/// (`ss_sp_words[3]`), pulled from the raw JSONL (mirrors `h4_replay`).
fn capture_draws(text: &str) -> Vec<(u32, u32, Option<u16>)> {
    let mut out = Vec::new();
    let mut seen = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        match v.get("e").and_then(|e| e.as_str()) {
            Some("combat_entry") => seen = true,
            Some("rng") if seen => {
                let before = v["before"].as_u64().unwrap() as u32;
                let after = v["after"].as_u64().unwrap() as u32;
                let operand = v
                    .get("ss_sp_words")
                    .and_then(|w| w.as_array())
                    .and_then(|w| w.get(3))
                    .and_then(|n| n.as_u64())
                    .map(|n| n as u16);
                out.push((before, after, operand));
            }
            _ => {}
        }
    }
    out
}

/// Replay a capture and return `(first_divergence_index, trip_count)`.
/// `None` divergence == closed (all draws equal on `(before, after, operand)` and
/// equal length). The comparison is `h4_replay`'s: `(before, after)` always, plus
/// the operand when both sides carry one.
fn replay(
    path: &Path,
    capture: &str,
    map_direction: u8,
    auto_cast: bool,
    auto_cast_toggles: &[u32],
    area_field_6e4: i32,
) -> (Option<usize>, usize) {
    let text = std::fs::read_to_string(path).expect("capture readable");
    let trace = Trace::parse(&text).expect("capture parses");
    let entry = trace
        .combat_entry()
        .expect("capture carries a combat_entry snapshot");

    let entries: Vec<RecordCombatant> = entry
        .combatants
        .iter()
        .map(|c| RecordCombatant {
            team: match c.team {
                0 => Team::Party,
                1 => Team::Monster,
                other => panic!("unknown team byte {other}"),
            },
            pos: GridPos::new(c.x as i32, c.y as i32),
            record: &c.record,
            affects: common::decode_affect_nodes(&c.affects),
        })
        .collect();

    let rules = RuleSet::load();
    let flavor = Adnd1::new(&rules);
    let map = match entry.terrain.as_deref() {
        Some(hex) => CombatMap::from_ground(decode_terrain(hex)),
        None => CombatMap::uniform(FLOOR),
    };
    let mut state = combat_state_from_records(&entries, map, &flavor).expect("records decode");
    state.area_field_58c = entry.area2_field_58c.map(|v| v as i32).unwrap_or(99);
    // The capture's own emitted heading wins (hooks from 8ab275e on); the pin's
    // value covers legacy captures without the field.
    state.map_direction = entry.map_direction.unwrap_or(map_direction);
    state.auto_pcs_cast_magic = auto_cast;
    state.auto_cast_toggles = auto_cast_toggles.to_vec();
    // coab≠binary #21 (doc §45): the party-only area movement modifier. The
    // pin stays the documented input, but when the capture carries the field
    // (post-cc0c9cd hooks, the doc §46 prep) the two must AGREE — a silent
    // capture-vs-pin mismatch would let a stale pin mask the real input.
    // Compared in ONE domain — the engine's — through the §47 byte-bridge:
    // the ECL stores a byte (sewer-fight-3 records the raw word 253 = 0xFD),
    // the pin records the engine-semantic −3; comparing raw-vs-semantic would
    // fire spuriously on every byte-negative area.
    if let Some(cap_6e4) = entry.area2_field_6e4 {
        assert_eq!(
            common::area_6e4_from_word(i32::from(cap_6e4)),
            area_field_6e4,
            "{capture}: pin's area_field_6e4 must record the capture's emitted \
             area2_field_6e4 (engine domain, §47 byte-bridge) — edit the manifest pin"
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
    state.area_field_6e4 = area_field_6e4;
    // §34.1: the ITEMS table + per-capture ranged loadouts (one shared place,
    // `common`). `None` loadouts leave a combatant melee-identical, so the six
    // non-armed pins are unshifted; armed-bar arms MATHEW/TRAVIS's bows.
    let records: Vec<&[u8]> = entries.iter().map(|e| e.record).collect();
    common::apply_loadouts(&mut state, capture, &records, common::load_item_data());

    let tap = DrawTap::default();
    let draws = tap.draws.clone();
    let mut rng = EngineRng::new(entry.rng_state);
    rng.attach_sink(Box::new(tap));

    // Count stub trips (a closed capture must fire none).
    let trips: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    struct TripTap(Rc<RefCell<usize>>);
    impl gbx_engine::combat::ActionSink for TripTap {
        fn on_action(&mut self, e: gbx_engine::combat::ActionEvent) {
            if matches!(e, gbx_engine::combat::ActionEvent::StubTripped { .. }) {
                *self.0.borrow_mut() += 1;
            }
        }
    }
    state.attach_action_sink(Box::new(TripTap(trips.clone())));

    state.run_combat_observed(&mut rng, DEFAULT_NO_ACTION_LIMIT, |_, _| {});

    let trip_count = *trips.borrow();
    let ours = draws.borrow();
    let cap = capture_draws(&text);
    let max = ours.len().max(cap.len());
    let mut frontier = None;
    for i in 0..max {
        match (ours.get(i), cap.get(i)) {
            (Some(o), Some(c)) => {
                let operand_ok = match (o.n, c.2) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                };
                if !(o.before == c.0 && o.after == c.1 && operand_ok) {
                    frontier = Some(i);
                    break;
                }
            }
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
    let Some(dir) = traces_dir() else {
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
        if common::capture_has_loadout(pin.capture) && common::load_item_data().is_none() {
            eprintln!("SKIPPED (ITEMS absent, D10): {}", pin.capture);
            continue;
        }
        checked += 1;
        let (frontier, trips) = replay(
            &path,
            pin.capture,
            pin.map_direction,
            pin.auto_cast,
            pin.auto_cast_toggles,
            pin.area_field_6e4,
        );
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
