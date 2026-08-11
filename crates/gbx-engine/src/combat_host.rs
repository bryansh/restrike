//! ★ **The Shell's combat host** (`docs/design/combat-visualizer.md` §8, D-CV1
//! item 1) — the live fight, on screen, inside the walk loop.
//!
//! What it replaces: `shell.rs`'s `run_pending_combat`, which ran the whole
//! fight headlessly inside one `tick` and resumed the VM on the same tick. Its
//! *assembly* logic lives on here ([`CombatHost::assemble`]) — §8.3 rule 3: the
//! function retires from the shell path, its logic moves rather than dies.
//!
//! ## The parking shape (§8.1)
//!
//! A [`CombatHost`] is parked in [`VmPhase::Combat`](crate::shell::VmPhase), an
//! **interaction-level** variant beside `Gate` — not a top-level `Shell`
//! variant. The `VectorRun` that yielded `Request::Combat`, and the flow that
//! owns it (Boot, Step, Look, or a chain round), stay exactly where they were,
//! suspended mid-`Present` with their stage cursors intact. The fight is an
//! interaction the vector is waiting on, morally identical to a menu.
//!
//! ## The three stages (§8.2)
//!
//! ```text
//! Pump/Present ──Request::Combat(+monsters)──▶ Entry ──▶ Fighting ──▶ ExitStage ──▶ (reply) Pump
//! ```
//!
//! - **[`Stage::Entry`]** follows `BattleSetup`'s own order
//!   (`ovr011.cs:1169-1220`): the prompt clear + `GameDelay`, "A battle
//!   begins...", then `SetupGroundTiles` (the D-CV6 floor, **draw-bearing**),
//!   `PlaceCombatants`, the fight's art, the camera, and the first full draw.
//! - **[`Stage::Fighting`]** is the D-CV2 lockstep loop verbatim: `step()`
//!   once, buffer its `ActionEvent` batch, play that batch to completion over
//!   ticks, reconcile at the boundary, repeat. Live input lands at step heads
//!   only.
//! - **[`Stage::ExitStage`]** holds the last frame a beat, restores the
//!   exploration screen, and **only at completion** writes the transcript line,
//!   `party_killed`, `Reply::Combat` and `phase = Pump`. That ordering is §8.2's
//!   MUST: `Shell::tick` replaces the shell with `GameOver` at top-of-tick when
//!   `party_killed` is set (`shell.rs`), so setting it at outcome-known time
//!   would annihilate the fight's final beats mid-playback.
//!
//! ## What is NOT here
//!
//! The headless paths — `run_encounter`, `CombatState::run_combat*`, the
//! `h4_*` harnesses, the frontier guard — never construct a `CombatHost`
//! (D-CV1's standing rule, §8.3 rule 3). And presentation never touches the
//! PRNG: every draw this module is responsible for comes from the floor dice at
//! assembly time or from `CombatState::step` inside `Fighting`, which is what
//! the shell-path draw-parity test asserts.

use crate::combat::floor::{self, FloorError};
use crate::combat::kits;
use crate::combat::manual::{TurnCmd, TurnOutcome};
use crate::combat::scene::{
    menu::{ManualUi, MenuAction},
    strings,
    time::BeatClock,
    CombatScene, CombatantIdentity, EntrySnapshot, SceneArt,
};
use crate::combat::{
    place_combatants, ActionEvent, ActionSink, CombatOutcome, CombatState, CombatStep, Combatant,
    GridPos, PlacementInput, Team,
};
use crate::combat_art::{self, CombatIcons, COMBAT_ICON_SLOTS};
use crate::monster::LoadedMonster;
use crate::shell::{FlowCtx, GameState};
use gbx_formats::items::ItemDataTable;
use gbx_rules::adnd1::flavor_impl::Adnd1;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The resident `ITEMS` file's name in a CotAB data directory — a flat file
/// beside the executable, not a DAX (`ItemData.cs:42`).
pub const ITEMS_FILE: &str = "ITEMS";

/// Buffers one `step()`'s [`ActionEvent`]s — D-CV2's render feed. The same
/// shape the reel uses (`combat::reel::BatchSink`).
struct BatchSink(Rc<RefCell<Vec<ActionEvent>>>);
impl ActionSink for BatchSink {
    fn on_action(&mut self, event: ActionEvent) {
        self.0.borrow_mut().push(event);
    }
}

/// Where a parked fight is in the §8.2 chart.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Stage {
    /// `BattleSetup`'s prompt beat, still on the exploration screen.
    Announce { ticks_left: u32 },
    /// The D-CV2 lockstep loop.
    Fighting,
    /// ★ **M6c**: a manual turn is open ([`CombatStep::AwaitPlayerTurn`]) and
    /// the menus have the keyboard. No `step()` runs until a command ends the
    /// turn — here the suspension *is* the lockstep.
    PlayerTurn,
    /// ★ **M6c**: the mid-combat View sheet (`viewPlayer`, `ovr020.cs:236`),
    /// palette swapped back to normal around it. Returns to
    /// [`Stage::PlayerTurn`].
    Sheet { member: usize },
    /// ★ **M6c**: the round-end `yes_no("Continue Battle:")`
    /// ([`CombatStep::AwaitContinueBattle`]).
    ContinuePrompt,
    /// The last frame, held one `GameDelay` beat before the screen restores.
    FinalBeats { ticks_left: u32 },
    /// ★ **roll-credits slice 3**: `displayCombatResults` (`ovr006.cs:381`) —
    /// the headline, the experience the party just earned, and a blocking
    /// keypress. Reached only for a fight that was not a party wipe (the wipe
    /// arm is `AfterCombatExpAndTreasure`'s own `else`, which is the
    /// [`crate::shell::GameOverFlow`]'s screen).
    Results,
    /// ★ `distributeCombatTreasure` (`ovr006.cs:564`) — the pool screen. Its
    /// word list is composed exactly as the original composes it, from what is
    /// actually on the ground.
    Treasure,
    /// The one-tick screen restore + the deferred writes.
    Restore,
}

/// The art pins one fight draws with — enough to rebuild [`SceneArt`] after a
/// deserialize without re-deriving it from a roster that has since moved.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FightArt {
    /// `gbl.game_area` — the `CPIC{area}.DAX` suffix.
    pub cpic_area: u8,
    /// Icon slot (`>= 8`) → the CPIC block LOADMONSTER put there.
    pub monster_blocks: BTreeMap<u8, u8>,
    /// `SetupGroundTiles`' fork: DUNGCOM or WILDCOM.
    pub in_dungeon: bool,
    /// Per party roster index, the `IconInfo` its CHEAD/CBODY merge needs.
    pub party_icons: Vec<crate::party::IconInfo>,
}

/// Everything that can stop a fight before its first frame.
#[derive(Debug, Clone, PartialEq)]
pub enum CombatHostError {
    /// The roster placed nobody — an empty party or an empty monster list.
    NothingToFight,
    /// A record in the live party did not decode. Cannot happen for a party
    /// that came through import (it decoded once already), so this names a
    /// party-model bug rather than bad data.
    Record(gbx_formats::save_orig::SaveParseError),
}

impl std::fmt::Display for CombatHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CombatHostError::NothingToFight => {
                write!(f, "the encounter has no living party or no monsters")
            }
            CombatHostError::Record(e) => write!(f, "a party record did not decode: {e:?}"),
        }
    }
}

/// ★ **A live fight, parked inside a suspended `VectorRun`.**
///
/// Serde per D-CV7 (§8.1): the whole host derives, with the scene and the event
/// buffer `#[serde(skip)]` + rebuilt-on-load — both are presentation state
/// reconstructible from the `CombatState` the host *does* carry, and both are
/// default-inert. A parked fight therefore snapshots by construction (D-UI2),
/// while the player-facing Save word stays faithfully absent from the combat
/// menus (`ovr009.cs:313-360,616-631`).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CombatHost {
    stage: Stage,
    state: CombatState,
    /// Roster order names + icon slots — the two facts `Combatant` has no room
    /// for (doc §2's gap table).
    identities: Vec<CombatantIdentity>,
    art: FightArt,
    /// Readied-weapon display names for the right panel, by `ITEMS` type.
    weapon_names: BTreeMap<u8, String>,
    /// `gbl.combat_round` as of the last `RoundEnded` — the transcript's count.
    rounds: u16,
    /// Set once `step()` returns `Ended`; the final batch may still be playing.
    ended: bool,
    outcome: Option<CombatOutcome>,
    /// ★ **M6c**: the open manual turn's menus, if one is suspended.
    manual: Option<ManualUi>,
    /// `gbl.menuSelectedWord` — global in the original: the last-resolved
    /// menu word stays selected into the next menu, across turns (the D13
    /// side-by-side showed QUICK still selected a turn after Quick was
    /// picked). Seeded into each `ManualUi` at open, persisted after keys.
    #[serde(default)]
    menu_selected: usize,
    /// Keys pressed during AI turns that no path honours — none, as of M6c:
    /// SPACE and '2' are both real now. Kept because the *reporting* is the
    /// §8.2 rule, and the next unmodeled key should land here rather than
    /// vanish.
    dropped_keys: Vec<u8>,
    /// ★ roll-credits slice 3: what the fight paid out, computed once at the
    /// FinalBeats boundary and printed by [`Stage::Results`].
    #[serde(default)]
    award: Option<crate::award::AwardOutcome>,
    /// `distributeNpcTreasure`'s message lines, if a joined NPC took a cut.
    #[serde(default)]
    npc_share: Vec<String>,
    /// Presentation, rebuilt on load (D-CV7).
    #[serde(skip)]
    scene: Option<CombatScene>,
    #[serde(skip)]
    batch: Rc<RefCell<Vec<ActionEvent>>>,
}

/// A clone is a **snapshot**, identical in content to what serde writes: the
/// scene and the event buffer are dropped, and the next tick rebuilds both
/// ([`CombatHost::rebuild_scene_if_missing`]). `Engine::save` clones the whole
/// `Shell` to encode it, so the two paths must agree — and this is the shape
/// that makes a restored fight resume correctly rather than play empty
/// schedules into a detached sink.
impl Clone for CombatHost {
    fn clone(&self) -> Self {
        CombatHost {
            stage: self.stage.clone(),
            state: self.state.clone(),
            identities: self.identities.clone(),
            art: self.art.clone(),
            weapon_names: self.weapon_names.clone(),
            rounds: self.rounds,
            ended: self.ended,
            outcome: self.outcome,
            manual: self.manual.clone(),
            menu_selected: self.menu_selected,
            dropped_keys: self.dropped_keys.clone(),
            award: self.award,
            npc_share: self.npc_share.clone(),
            scene: None,
            batch: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

/// What one [`CombatHost::tick`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostTick {
    /// Still fighting; call again next tick.
    Working,
    /// ExitStage completed. The caller performs §8.2's deferred writes.
    Finished {
        outcome: CombatOutcome,
        rounds: u16,
        /// SPACE presses this slice queued and dropped (slice 7's TurnCmd).
        dropped_keys: usize,
    },
}

impl CombatHost {
    /// ★ **`BattleSetup`'s setup half** (`ovr011.cs:1186-1210`), plus the roster
    /// assembly `run_pending_combat` used to do inline.
    ///
    /// Runs at the moment the announce beat drains, in the original's order:
    ///
    /// 1. `SetupGroundTiles` — the D-CV6 faithful [`floor`] (**draw-bearing**;
    ///    this is where a live fight's draw stream now opens);
    /// 2. `PlaceCombatants` — now with the area's real wall flags, not the
    ///    open-ground stub ([`floor::dir_flags_hook`]);
    /// 3. the roster: the party through [`kits`] (real equipment, readied
    ///    weapons, §49's readied-ammo gate) and the monsters straight off
    ///    `LOAD MONSTER`;
    /// 4. the `ITEMS` table, then the per-member loadouts (§34.1's order).
    ///
    /// The `Request::Combat` reply the VM eventually gets is unchanged (§8.3
    /// rule 2) — a script cannot tell this fight from the headless one.
    fn assemble(
        ctx: &mut FlowCtx,
    ) -> Result<(CombatState, Vec<CombatantIdentity>, FightArt), CombatHostError> {
        let monsters: Vec<LoadedMonster> = std::mem::take(&mut ctx.state.pending_combat.monsters);
        let living = kits::living_count(&ctx.roster.members);
        if living == 0 || monsters.is_empty() {
            // Consume the roster anyway (`CMD_Combat` does): leaving
            // `monstersLoaded` up would send the script's *next* COMBAT down
            // the real-combat branch with nothing to fight.
            ctx.state.pending_combat.clear();
            return Err(CombatHostError::NothingToFight);
        }

        let map_dir = crate::shell::facing_to_map_dir(ctx.state.facing);
        let in_dungeon = matches!(ctx.state.game_state, GameState::DungeonMap);
        let party_pos = (ctx.state.pos.0 as i32, ctx.state.pos.1 as i32);
        // ★ `CMD_Combat`'s own re-clamp (`ovr003.cs:997-1001`): cast a FRESH
        // `sub_304B4` ray and lower `area2_ptr.encounter_distance` to it if the
        // ray is shorter — `if (var_2 < encounter_distance) encounter_distance
        // = var_2;`. One-sided, so an approach that already walked the
        // monsters in close stays close even where the corridor is open.
        // Placement then reads the CELL (`ovr011.cs:1067-1068`), not the ray,
        // which is what makes APPROACH mechanically real: `SETUP MONSTER
        // s,1,p` + `APPROACH` starts the fight adjacent.
        let ray = crate::combat::encounter_distance(
            ctx.geo,
            map_dir,
            party_pos.0,
            party_pos.1,
            in_dungeon,
        );
        if ray < ctx.state.encounter_distance {
            ctx.state.encounter_distance = ray;
        }
        let dist = ctx.state.encounter_distance;

        // (1) The floor. Wilderness is deferred whole (doc §7) — rather than
        // lay a dungeon floor outdoors, fall back to the flagged provisional
        // derivation and SAY so in the transcript.
        let mut map = match floor::setup_ground_tiles(
            ctx.geo,
            party_pos,
            ctx.state.ecl_block_id,
            in_dungeon,
            ctx.rng,
        ) {
            Ok(map) => map,
            Err(FloorError::WildernessFloorDeferred) => {
                ctx.vm_memory
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Request(
                        "combat: wilderness floor deferred — provisional terrain \
                         (combat-visualizer.md §7)"
                            .to_string(),
                    ));
                crate::combat::provisional_combat_map(ctx.geo)
            }
        };

        // (2) Placement, with the area's real walls in play.
        let party_sizes: Vec<u8> = ctx
            .roster
            .members
            .iter()
            .filter(|c| c.hit_point_current > 0)
            .map(|c| c.opaque.field_de & 7)
            .collect();
        let inputs: Vec<PlacementInput> = party_sizes
            .iter()
            .map(|&size| PlacementInput {
                team: Team::Party,
                size,
                in_combat: true,
            })
            .chain(monsters.iter().map(|_| PlacementInput {
                // Monster footprints ride `field_DE & 7` in the original, which
                // `LoadedMonster` does not carry yet — every monster places as
                // a single cell exactly as `run_encounter` has always done.
                team: Team::Monster,
                size: 1,
                in_combat: true,
            }))
            .collect();
        let hook = floor::dir_flags_hook(ctx.geo, party_pos.1);
        let placements = place_combatants(
            &mut map,
            &inputs,
            map_dir,
            dist as i32,
            GridPos::new(party_pos.0, party_pos.1),
            Some(&hook),
        );

        // (3) The roster, party first (TeamList order is load-bearing).
        let item_data = load_item_data(ctx);
        let rules = ctx.rules;
        let flavor = Adnd1::new(rules);
        let party_positions: Vec<GridPos> = placements[..living].iter().map(|p| p.pos).collect();
        let party = kits::party_kits(
            &ctx.roster.members,
            &party_positions,
            item_data.as_ref(),
            &flavor,
        );

        let mut identities = Vec::with_capacity(party.len() + monsters.len());
        let mut party_icons = Vec::with_capacity(party.len());
        let mut fighters: Vec<Combatant> = Vec::with_capacity(party.len() + monsters.len());
        let mut loadouts = Vec::new();
        for kit in party {
            let member = &ctx.roster.members[kit.member_index];
            identities.push(CombatantIdentity::new(
                member.name.clone(),
                (member.icon.icon_id as usize).min(COMBAT_ICON_SLOTS - 1),
            ));
            party_icons.push(member.icon);
            if let Some(l) = kit.loadout {
                loadouts.push((fighters.len(), l));
            }
            fighters.push(kit.combatant);
        }

        let mut monster_blocks = BTreeMap::new();
        for m in &monsters {
            let a1 = m.attacks[0];
            let id = fighters.len();
            identities.push(CombatantIdentity::new(
                m.name.clone(),
                (m.icon_slot as usize).min(COMBAT_ICON_SLOTS - 1),
            ));
            monster_blocks.insert(m.icon_slot, m.icon_block);
            let mut combatant = Combatant::new_melee(
                id,
                Team::Monster,
                m.is_npc(),
                placements[id].pos,
                m.hit_point_max as i32,
                m.ac as u8,
                m.thac0 as i32,
                m.movement as i32,
                (a1.dice_count, a1.dice_size, a1.damage_bonus as u8),
                0, // delay — CalculateInitiative sets it each round
                1, // one swing/round
            );
            // ★ What this one is worth dead (roll-credits slice 3). Carried
            // here because `calc_battle_exp` runs against the fight's final
            // roster, after the original has already deallocated the monster
            // records. Nothing in combat reads it, so it cannot move a draw.
            combatant.award = m.award.clone();
            fighters.push(combatant);
        }

        let mut state = CombatState::new(map, fighters);
        state.map_direction = map_dir;
        // §34.1's order: the table first, then the rows that read it.
        state.item_data = item_data;
        for (id, l) in loadouts {
            state.set_loadout(id, l);
        }

        ctx.state.pending_combat.clear();
        Ok((
            state,
            identities,
            FightArt {
                cpic_area: ctx.game_area(),
                monster_blocks,
                in_dungeon,
                party_icons,
            },
        ))
    }

    /// Opens a fight: the announce beat, still on the exploration screen.
    ///
    /// Nothing is assembled yet — `BattleSetup` prints "A battle begins..."
    /// *before* `SetupGroundTiles`, and the floor dice are draw-bearing, so the
    /// order is kept even though only the message is visible at this point.
    pub fn open(ctx: &mut FlowCtx) -> Self {
        // `ClearPromptArea(); GameDelay(); displayString("A battle begins...")`
        // (`ovr011.cs:1176-1179`).
        crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, strings::A_BATTLE_BEGINS);
        ctx.vm_memory
            .transcript
            .push(crate::vmhost::TranscriptEntry::Print {
                text: strings::A_BATTLE_BEGINS.to_string(),
                clear_first: false,
            });
        CombatHost {
            stage: Stage::Announce {
                // The original holds this message for however long the combat
                // art takes to load off disk; one `GameDelay` beat stands in
                // for that (a presentation choice, draw-free).
                ticks_left: BeatClock::default().game_delay(),
            },
            state: CombatState::new(crate::combat::CombatMap::uniform(0), Vec::new()),
            identities: Vec::new(),
            art: FightArt {
                cpic_area: ctx.game_area(),
                monster_blocks: BTreeMap::new(),
                in_dungeon: true,
                party_icons: Vec::new(),
            },
            weapon_names: BTreeMap::new(),
            rounds: 0,
            ended: false,
            outcome: None,
            manual: None,
            menu_selected: 0,
            dropped_keys: Vec::new(),
            award: None,
            npc_share: Vec::new(),
            scene: None,
            batch: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// One tick of the §8.2 chart.
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> HostTick {
        // D-CV7's rebuild-on-load, before anything reads the presenter: every
        // stage past `Announce` owns a scene, and a fight restored from a
        // snapshot has none.
        if !matches!(self.stage, Stage::Announce { .. }) {
            let had_scene = self.scene.is_some();
            self.rebuild_scene_if_missing(ctx);
            // A fight restored *on a player's turn* rebuilds its menus' three
            // surfaces too — the words, the status row and the focus box are
            // presentation, so the snapshot did not carry them.
            if !had_scene && self.manual.is_some() {
                self.refresh_manual_surfaces();
            }
        }
        self.drain_input(ctx);
        match self.stage {
            Stage::Announce { ticks_left } => {
                let left = ticks_left.saturating_sub(ctx.dt_ticks);
                if left > 0 {
                    self.stage = Stage::Announce { ticks_left: left };
                    return HostTick::Working;
                }
                self.begin_fight(ctx);
                HostTick::Working
            }
            Stage::Fighting => {
                self.tick_fighting(ctx);
                HostTick::Working
            }
            // Both suspensions and the sheet are input-driven: `drain_input`
            // above did this tick's work, and the frame is already composed.
            Stage::PlayerTurn | Stage::ContinuePrompt => {
                self.render(ctx);
                HostTick::Working
            }
            Stage::Sheet { member } => {
                self.render_sheet(ctx, member);
                HostTick::Working
            }
            Stage::FinalBeats { ticks_left } => {
                let left = ticks_left.saturating_sub(ctx.dt_ticks);
                if left > 0 {
                    self.stage = Stage::FinalBeats { ticks_left: left };
                    self.render(ctx);
                } else {
                    // ★ `AfterCombatExpAndTreasure` (`ovr006.cs:763`): the
                    // award is computed here, once, and the screens that show
                    // it follow. A wiped party takes the `else` arm instead —
                    // and that arm is the wipe flow, which `Shell::tick` opens
                    // from `party_killed` after this host finishes.
                    self.settle_and_award(ctx);
                    self.stage = if self.outcome == Some(CombatOutcome::MonstersWin) {
                        Stage::Restore
                    } else {
                        self.draw_results(ctx);
                        Stage::Results
                    };
                }
                HostTick::Working
            }
            Stage::Results | Stage::Treasure => {
                // Input-driven; `drain_input` did this tick's work and the
                // screen is already composed.
                HostTick::Working
            }
            Stage::Restore => {
                // `free_combat_stuff` (`ovr009.cs:9`) + `CMD_Combat`'s `LoadPic`
                // rebuild (`ovr003.cs:971`): the palette goes back and the
                // exploration screen is recomposed whole — combat replaced it
                // (§1.1), so putting the 3D viewport back is not enough; the
                // frame, the panel and the text areas are all combat pixels
                // until they are painted over. These are `Engine::build`'s own
                // three steps for a fresh screen.
                crate::combat::scene::render::palette_normal(ctx.fb);
                ctx.fb.clear(0);
                crate::frames::draw8x8_03(ctx.fb, ctx.symbols)
                    .expect("symbol set 4 is resident whenever a fight can start");
                // `LoadPic`'s `DungeonMap` arm (`ovr025.cs:1435-1441`) is
                // `draw8x8_03` + `RedrawView` + party summary + status line —
                // it never puts a pre-combat picture back, and `redraw_view`
                // clears the picture layer for exactly that reason.
                crate::corridor::redraw_view(ctx);
                HostTick::Finished {
                    outcome: self.outcome.unwrap_or(CombatOutcome::Stalemate),
                    rounds: self.rounds,
                    dropped_keys: self.dropped_keys.len(),
                }
            }
        }
    }

    /// The `Entry` stage's second half: assemble, load the art, take D-CV2's
    /// entry snapshot after the first `step()`, and draw the combat screen.
    fn begin_fight(&mut self, ctx: &mut FlowCtx) {
        let (mut state, identities, art) = match Self::assemble(ctx) {
            Ok(parts) => parts,
            Err(e) => {
                // Nothing to fight: report it and end the stage immediately, so
                // the VM still gets its `Reply::Combat` and the script goes on.
                // The refusal is drawn on the prompt line too — a transcript
                // no frontend shows is not a report (the partyless-boot hunt,
                // 2026-08-03: ten loaded patrons met an empty roster and the
                // only witness was RESTRIKE_DEBUG_LOG).
                crate::combat::scene::render::draw_prompt(
                    ctx.fb,
                    ctx.font,
                    &format!("combat: {e}"),
                );
                ctx.vm_memory
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Request(format!(
                        "combat: {e}"
                    )));
                self.outcome = Some(CombatOutcome::Stalemate);
                self.ended = true;
                self.stage = Stage::Restore;
                return;
            }
        };
        state.attach_action_sink(Box::new(BatchSink(Rc::clone(&self.batch))));
        // ★ D-CV5: there is a human at the keyboard. This is the one call that
        // arms both suspensions — every headless path leaves it off, which is
        // what makes the whole manual surface invisible to the guard.
        state.set_interactive(true);

        // D-CV2: the camera initializes lazily inside `combat_setup` on the
        // first `step()`, so the entry snapshot is read after it.
        let first = state.step(ctx.rng);
        self.note_round(first);
        if first == CombatStep::Ended {
            self.ended = true;
            self.outcome = Some(state.outcome());
        }

        self.weapon_names = weapon_display_names(ctx);
        let scene_art = self.load_art(ctx, &art);
        let mut scene = CombatScene::new(EntrySnapshot::from_state(&state, &identities), scene_art);
        scene.set_weapon_names(self.weapon_names.clone());
        scene.refresh_panels(&state);
        scene
            .reconcile(&state)
            .expect("the entry snapshot was just read from this very state");
        scene.begin_step(&self.take_batch());

        self.state = state;
        self.identities = identities;
        self.art = art;
        self.scene = Some(scene);
        self.stage = Stage::Fighting;
        // The first `step()` is `RoundStarted` (initiative), never a
        // suspension — but say so rather than assume it.
        debug_assert!(
            !matches!(
                first,
                CombatStep::AwaitPlayerTurn { .. } | CombatStep::AwaitContinueBattle
            ),
            "the entry step cannot suspend: it is the round head"
        );
        self.render(ctx);
    }

    /// D-CV2's lockstep loop, verbatim: present step N to completion **before**
    /// calling `step()` for N+1. There is no pipelining and no rollback.
    fn tick_fighting(&mut self, ctx: &mut FlowCtx) {
        let scene = self.scene.as_mut().expect("`tick` rebuilds it first");
        if scene.is_playing() {
            let cues = scene.tick(ctx.dt_ticks.max(1));
            ctx.sounds.extend_from_slice(cues);
            self.render(ctx);
            return;
        }

        // The step boundary — the only two `CombatState` reads the scene gets.
        scene
            .reconcile(&self.state)
            .unwrap_or_else(|e| panic!("the presented board drifted from the fight: {e:?}"));
        scene.refresh_panels(&self.state);

        if self.ended {
            self.stage = Stage::FinalBeats {
                ticks_left: scene.clock().game_delay(),
            };
            self.render(ctx);
            return;
        }

        let step = self.state.step(ctx.rng);
        self.note_round(step);
        if step == CombatStep::Ended {
            self.ended = true;
            self.outcome = Some(self.state.outcome());
        }
        let events = self.take_batch();
        self.scene
            .as_mut()
            .expect("just rebuilt")
            .begin_step(&events);
        // ★ M6c: a suspension parks the host on the player. Its batch (the
        // `Pick` and the turn head's camera) plays out first — the stage change
        // is what stops `step()` being called again, and playback drains under
        // `Stage::PlayerTurn` exactly as it does here.
        match step {
            CombatStep::AwaitPlayerTurn { combatant_id } => self.open_manual_turn(combatant_id),
            CombatStep::AwaitContinueBattle => self.stage = Stage::ContinuePrompt,
            _ => {}
        }
        self.render(ctx);
    }

    /// ★ **M6c**: park on a manual turn — build the menus, put the focus box on
    /// the actor and its summary in the right panel (`DoPlayerCombatTurn`'s own
    /// `RedrawCombatIfFocusOn(true, 2, player)` + `CombatDisplayPlayerSummary`,
    /// `ovr009.cs:122-124`).
    fn open_manual_turn(&mut self, actor: usize) {
        let mut ui = ManualUi::open(&mut self.state, actor);
        ui.set_selected(self.menu_selected);
        if let Some(scene) = self.scene.as_mut() {
            ui.set_game_speed(scene.clock().game_speed());
            scene.set_panel_focus(Some(actor));
        }
        self.manual = Some(ui);
        self.stage = Stage::PlayerTurn;
        self.refresh_manual_surfaces();
    }

    /// Push the open UI's three surfaces into the scene: the prompt row, the
    /// status row and the grey focus box.
    fn refresh_manual_surfaces(&mut self) {
        let Some(ui) = self.manual.as_ref() else {
            return;
        };
        let size = ui
            .aim_target()
            .and_then(|t| self.state.roster().get(t))
            .map(|c| c.size)
            .unwrap_or_else(|| self.state.roster()[ui.actor()].size);
        let (prompt, status) = (ui.prompt(), ui.status());
        let aim_cell = ui.aim_focus_cell();
        let actor = ui.actor();
        let panel = ui.aim_target().or(Some(actor));
        let ui_span = ui.selected_span();
        if let Some(scene) = self.scene.as_mut() {
            // The actor's box rides the **presented** board, so it follows the
            // icon through a walk instead of jumping to where the fight has
            // already put it (D-CV2's whole point). The aim cursor is
            // presentation's own and has no board twin.
            let pos = aim_cell.or_else(|| scene.board().combatant(actor).map(|c| c.pos));
            scene.set_prompt(Some(prompt));
            scene.set_prompt_selection(ui_span);
            scene.set_status(status);
            scene.set_focus(pos.map(|pos| crate::combat::scene::FocusCursor { pos, size }));
            scene.set_panel_focus(panel);
        }
    }

    /// ★ **M6c**: run one accepted [`TurnCmd`] and fold the result back into
    /// the UI, the scene and the stage.
    ///
    /// The events a command emits are played through the same timeline a
    /// `step()` batch is: a swing animates, a walk step walks. The lockstep
    /// invariant holds trivially — the next key is not read until this drains,
    /// because `drain_input` only feeds the UI at a quiet moment.
    fn issue(&mut self, ctx: &mut FlowCtx, cmd: TurnCmd) {
        let outcome = self.state.issue(ctx.rng, cmd.clone());
        let events = self.take_batch();
        if let Some(scene) = self.scene.as_mut() {
            if !events.is_empty() {
                scene.begin_step(&events);
            }
        }
        match outcome {
            Ok(TurnOutcome::Speed(n)) => {
                if let Some(scene) = self.scene.as_mut() {
                    scene.set_game_speed(n);
                }
                if let Some(ui) = self.manual.as_mut() {
                    ui.set_game_speed(n);
                }
            }
            Ok(outcome) => {
                if let Some(ui) = self.manual.as_mut() {
                    ui.note(outcome);
                }
                if outcome.turn_ended() {
                    self.close_manual_turn();
                    return;
                }
            }
            Err(refusal) => {
                // §9's rule: a refusal is loud. Two of them are the original's
                // own player-facing lines; the rest name a driver bug, and the
                // transcript is where this host says so.
                if let Some(ui) = self.manual.as_mut() {
                    ui.note_refusal(&cmd, &refusal);
                }
                ctx.vm_memory
                    .transcript
                    .push(crate::vmhost::TranscriptEntry::Request(format!(
                        "combat: {cmd:?} refused: {refusal:?}"
                    )));
            }
        }
        if let Some(ui) = self.manual.as_mut() {
            ui.refresh(&mut self.state);
        }
        self.refresh_manual_surfaces();
    }

    /// The turn is over: drop the menus, clear their surfaces, and go back to
    /// the lockstep loop.
    fn close_manual_turn(&mut self) {
        self.manual = None;
        self.stage = Stage::Fighting;
        if let Some(scene) = self.scene.as_mut() {
            scene.set_prompt(None);
            scene.set_status(None);
        }
    }

    /// Live input.
    ///
    /// Two régimes, exactly as the original has: while the **AI** is fighting,
    /// the only keys are `sub_36269`'s head polls ('2' and SPACE), and they land
    /// at **step heads only** (the D-CV2 lockstep rule). While a **player's**
    /// turn is open, the menus have the keyboard and every key goes through
    /// [`ManualUi`].
    fn drain_input(&mut self, ctx: &mut FlowCtx) {
        // Both suspensions let the *last* batch finish playing before they read
        // a key — the original is inside its own animation there, and the D-CV2
        // lockstep forbids composing a batch over a running one.
        if matches!(self.stage, Stage::PlayerTurn | Stage::ContinuePrompt) {
            if self.tick_playback(ctx) {
                return;
            }
            // Playback just drained (or none is running): bring the manual
            // surfaces up to date BEFORE reading keys. `drain_menu_input`'s
            // accept path deliberately returns early while a step's batch is
            // playing, so this is the one place the focus box catches up to
            // where the walk actually put the icon (Bryan's playtest find,
            // 2026-08-03: the box trailed the actor by a cell per step).
            self.refresh_manual_surfaces();
        }
        match self.stage {
            Stage::PlayerTurn => self.drain_menu_input(ctx),
            Stage::ContinuePrompt => self.drain_continue_input(ctx),
            Stage::Sheet { .. } => self.drain_sheet_input(ctx),
            Stage::Results => self.drain_results_input(ctx),
            Stage::Treasure => self.drain_treasure_input(ctx),
            _ => self.drain_ai_turn_input(ctx),
        }
    }

    // --- roll-credits slice 3: the award screens --------------------------

    /// ★ `CleanupPlayersStateAfterCombat` + `calc_battle_exp` + `addExp` +
    /// `distributeNpcTreasure` (`ovr006.cs:169-253`, `:713`), in the
    /// original's order — which matters twice over: `partyAnimatedCount` is
    /// counted **before** the survivor ladder wakes anybody up (so a member
    /// who was down when the last monster fell does not dilute the share),
    /// and the NPC's cut comes off the pool **before** the results screen
    /// prints the number the party will see.
    ///
    /// Every call here is draw-free (`crate::award`'s module doc has the
    /// proof), so this runs with the fight's RNG untouched.
    fn settle_and_award(&mut self, ctx: &mut FlowCtx) {
        self.carry_affects_home(ctx);
        let animated = crate::award::animated_count(ctx.roster);
        let party_size = if ctx.state.party_size == 0 {
            ctx.roster.members.len() as u8
        } else {
            ctx.state.party_size
        };
        let outcome = crate::award::calc_battle_exp(
            &self.state,
            &mut ctx.state.pooled_money,
            &mut ctx.state.treasure_items,
            party_size,
            animated,
            false,
        );
        // `:249-253` — only a won battle pays.
        if self.outcome == Some(CombatOutcome::PartyWins) {
            crate::award::add_exp(ctx.roster, outcome.exp_each);
            ctx.state.exp_to_add = outcome.exp_each;
        } else {
            ctx.state.exp_to_add = 0;
        }
        crate::award::settle_survivors(ctx.roster);
        self.npc_share =
            crate::award::distribute_npc_treasure(ctx.roster, &mut ctx.state.pooled_money);
        self.award = Some(outcome);
        ctx.vm_memory
            .transcript
            .push(crate::vmhost::TranscriptEntry::Request(format!(
                "award: {} xp each, pool {} gp, {} item(s)",
                ctx.state.exp_to_add,
                ctx.state.pooled_money.gold_worth(),
                ctx.state.treasure_items.len()
            )));
    }

    /// `displayCombatResults` (`ovr006.cs:381-440`): the outer frame, the
    /// headline at row 3, the two experience lines at rows 5 and 7, and
    /// `displayInput`'s colour-15 prompt.
    fn draw_results(&mut self, ctx: &mut FlowCtx) {
        crate::combat::scene::render::palette_normal(ctx.fb);
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        let award = self.award.unwrap_or_default();
        let won = self.outcome == Some(CombatOutcome::PartyWins);
        let fled = self.outcome == Some(CombatOutcome::Stalemate) && !won;
        let headline = crate::award::results_headline(&award, fled, won);
        crate::text::draw_string(ctx.fb, ctx.font, headline, 3, 1, 0, 10); // `:390`
        let [line_a, line_b] = crate::award::results_exp_lines(ctx.state.exp_to_add);
        crate::text::draw_string(ctx.fb, ctx.font, &line_a, 5, 1, 0, 10); // `:436`
        crate::text::draw_string(ctx.fb, ctx.font, &line_b, 7, 1, 0, 10); // `:437`
        for (i, line) in self.npc_share.iter().enumerate() {
            // `distributeNpcTreasure`'s own lines (`ovr006.cs:752`).
            crate::text::draw_string(ctx.fb, ctx.font, line, 9 + i * 2, 2, 0, 10);
        }
        crate::combat::scene::render::draw_prompt(
            ctx.fb,
            ctx.font,
            "press <enter>/<return> to continue",
        );
    }

    fn drain_results_input(&mut self, ctx: &mut FlowCtx) {
        if ctx.input.read_key().is_some() {
            let (items, money) = crate::award::treasure_on_ground(
                &ctx.state.pooled_money,
                &ctx.state.treasure_items,
            );
            self.stage = if items || money {
                self.draw_treasure(ctx);
                Stage::Treasure
            } else {
                // `distributeCombatTreasure`'s Exit arm with an empty ground
                // (`ovr006.cs:656-659`) — nothing to claim, so it closes.
                Stage::Restore
            };
        }
    }

    /// `distributeCombatTreasure`'s menu line (`ovr006.cs:577-607`), composed
    /// from what is actually on the ground. Detect is omitted (its spell scan
    /// belongs to G7); View is the mid-combat sheet, already docketed.
    fn draw_treasure(&mut self, ctx: &mut FlowCtx) {
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        let (items, money) =
            crate::award::treasure_on_ground(&ctx.state.pooled_money, &ctx.state.treasure_items);
        let line = if money {
            "View Take Pool Share Exit"
        } else if items {
            "View Take Pool Exit"
        } else {
            "View Pool Exit"
        };
        let summary = format!(
            "Treasure: {} gp worth, {} item(s)",
            ctx.state.pooled_money.gold_worth(),
            ctx.state.treasure_items.len()
        );
        crate::text::draw_string(ctx.fb, ctx.font, &summary, 3, 1, 0, 10);
        crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, line);
    }

    /// The pool screen's keys. `S`/`P` are `share_pooled`/`poolMoney`
    /// verbatim; `T` takes one pooled item per press onto the selected member
    /// (the original opens `sl_select_item`'s scrolling list here — that
    /// widget is the named residual, not the arithmetic); `E`/Esc leaves,
    /// with the original's own "There is still treasure left" note in the
    /// transcript rather than a second modal.
    fn drain_treasure_input(&mut self, ctx: &mut FlowCtx) {
        while let Some(event) = ctx.input.read_key() {
            let key = match event {
                crate::input::InputEvent::Char(k) => k.to_ascii_uppercase(),
                crate::input::InputEvent::Escape => b'E',
                _ => continue,
            };
            match key {
                b'S' => crate::award::share_pooled(ctx.roster, &mut ctx.state.pooled_money),
                b'P' => crate::award::pool_money(ctx.roster, &mut ctx.state.pooled_money),
                b'T' => {
                    let member = ctx.state.selected_player as usize;
                    crate::award::take_item(ctx.roster, member, &mut ctx.state.treasure_items, 0);
                }
                b'E' => {
                    let (items, money) = crate::award::treasure_on_ground(
                        &ctx.state.pooled_money,
                        &ctx.state.treasure_items,
                    );
                    if items || money {
                        ctx.vm_memory
                            .transcript
                            .push(crate::vmhost::TranscriptEntry::Request(
                                "There is still treasure left.".to_string(),
                            ));
                    }
                    self.stage = Stage::Restore;
                    return;
                }
                _ => continue,
            }
            self.draw_treasure(ctx);
        }
    }

    /// Advances a running playback by this tick; `true` while one is running.
    fn tick_playback(&mut self, ctx: &mut FlowCtx) -> bool {
        let Some(scene) = self.scene.as_mut() else {
            return false;
        };
        if !scene.is_playing() {
            return false;
        }
        let cues = scene.tick(ctx.dt_ticks.max(1));
        ctx.sounds.extend_from_slice(cues);
        true
    }

    /// `process_input_in_monsters_turn` (`ovr010.cs:703-754`), as keys: '2'
    /// flips auto-magic, SPACE revokes auto-fight — and the engine consumes the
    /// SPACE at the original's own poll site, which is what hands the turn back.
    fn drain_ai_turn_input(&mut self, ctx: &mut FlowCtx) {
        let at_step_head = matches!(self.stage, Stage::Fighting)
            && self.scene.as_ref().is_some_and(|s| !s.is_playing());
        while let Some(event) = ctx.input.read_key() {
            let crate::input::InputEvent::Char(key) = event else {
                continue;
            };
            if !at_step_head {
                continue;
            }
            match key {
                b'2' => {
                    self.state.auto_pcs_cast_magic = !self.state.auto_pcs_cast_magic;
                    let label = if self.state.auto_pcs_cast_magic {
                        strings::MAGIC_ON
                    } else {
                        strings::MAGIC_OFF
                    };
                    ctx.vm_memory
                        .transcript
                        .push(crate::vmhost::TranscriptEntry::Print {
                            text: label.to_string(),
                            clear_first: false,
                        });
                }
                // ★ M6c: a real key at last. The flag is queued here and
                // consumed inside the next AI turn's own poll, so the revoke
                // takes effect exactly where the original's does.
                b' ' => self.state.queue_quick_fight_revoke(),
                _ => {}
            }
        }
    }

    /// The manual turn's keyboard (§9): one key per tick's worth of input,
    /// through the menus, with each accepted command executed immediately.
    fn drain_menu_input(&mut self, ctx: &mut FlowCtx) {
        while let Some(event) = ctx.input.read_key() {
            let Some(ui) = self.manual.as_mut() else {
                return;
            };
            match ui.key(event) {
                // ★ A key that changed only the UI's own state still needs the
                // boundary reads refreshed. Found live, 2026-08-08: `A` opens
                // Aim with an EMPTY scan list (`ManualUi::key` builds the
                // `AimState` but only `refresh` fills `aim.list` from
                // `copy_sorted_players`), so `Next`/`Prev` had nothing to walk
                // and the cursor sat on the actor forever — the list half of
                // §9.4's aim menu was unusable from the keyboard, while its
                // unit tests passed because they call `refresh` themselves.
                // `MenuAction::Issue` refreshes inside `issue`; this is the
                // other arm.
                MenuAction::None => {
                    if let Some(ui) = self.manual.as_mut() {
                        ui.refresh(&mut self.state);
                    }
                }
                MenuAction::OpenSheet => {
                    let actor = ui.actor();
                    // The party member behind the roster index — the fight's
                    // party run is the party's living prefix, in order.
                    let member = self.party_member_of(ctx, actor);
                    self.stage = Stage::Sheet { member };
                    return;
                }
                MenuAction::Issue(cmd) => {
                    self.issue(ctx, cmd);
                    // One keypress can owe a second command: a direction key at
                    // the main menu opens the loop *and* steps
                    // (`ovr009.cs:243`), and a `Y` to `"Attack Ally: "` re-runs
                    // the commit the core just refused (`ovr014.cs:1725-1746`).
                    let pending = self.manual.as_mut().and_then(|ui| ui.take_follow_up());
                    if let Some(step) = pending {
                        self.issue(ctx, step);
                    }
                    if !matches!(self.stage, Stage::PlayerTurn) {
                        return;
                    }
                    if self.scene.as_ref().is_some_and(|s| s.is_playing()) {
                        return;
                    }
                }
            }
            if let Some(ui) = self.manual.as_ref() {
                self.menu_selected = ui.selected_index();
            }
            self.refresh_manual_surfaces();
        }
    }

    /// `yes_no("Continue Battle:")` (`ovr009.cs:407`) — Y or N, nothing else.
    fn drain_continue_input(&mut self, ctx: &mut FlowCtx) {
        if let Some(scene) = self.scene.as_mut() {
            scene.set_prompt(Some(format!(
                "{} {}",
                strings::CONTINUE_BATTLE,
                strings::YES_NO
            )));
        }
        while let Some(event) = ctx.input.read_key() {
            let crate::input::InputEvent::Char(key) = event else {
                continue;
            };
            let yes = match key.to_ascii_uppercase() {
                b'Y' => true,
                b'N' => false,
                _ => continue,
            };
            let step = self.state.answer_continue_battle(yes);
            self.note_round(step);
            if let CombatStep::RoundEnded { battle_over, .. } = step {
                if battle_over {
                    self.ended = true;
                    self.outcome = Some(self.state.outcome());
                }
            }
            let events = self.take_batch();
            if let Some(scene) = self.scene.as_mut() {
                scene.set_prompt(None);
                scene.begin_step(&events);
            }
            self.stage = Stage::Fighting;
            return;
        }
    }

    /// `viewPlayer`'s exit set (`unk_54B03 = {0, 'E'}`, `ovr020.cs:249`).
    fn drain_sheet_input(&mut self, ctx: &mut FlowCtx) {
        while let Some(event) = ctx.input.read_key() {
            let exit = match event {
                crate::input::InputEvent::Escape => true,
                crate::input::InputEvent::Char(key) => key.eq_ignore_ascii_case(&b'E'),
                _ => false,
            };
            if exit {
                self.stage = Stage::PlayerTurn;
                // `Color_0_8_inverse()` + `LoadPic()` (`ovr020.cs:332-336`):
                // the combat palette and the whole combat screen come back.
                crate::combat::scene::render::palette_combat(ctx.fb);
                self.issue(ctx, TurnCmd::ViewSheet);
                return;
            }
        }
    }

    /// ★ **Roll-credits slice 5**: the fight's final affect chains go back onto
    /// the roster.
    ///
    /// In the original there is nothing to do here — combat mutates the same
    /// `Player.affects` list camp reads, so a bless cast in round 1 is simply
    /// still on the character when the results screen closes. Our split model
    /// has to copy it, and the copy is the *whole* list rather than a merge:
    /// the fight is the authority on what a party member is carrying by the
    /// time it ends, having already run `RemoveCombatAffects`'s strip table on
    /// anybody who died or fled (`ovr024.cs:48`, `:1270`).
    ///
    /// So the surviving set is exactly the original's: combat-scoped affects
    /// (`paralyze`, `sleep`, `stinking_cloud`, …) are in that strip table and
    /// are gone; `bless`, `prayer`, `protection_from_evil` and the racial
    /// affects are **not**, and walk out of the fight still running — until
    /// the next camp ticks them down ([`crate::affects`]).
    ///
    /// Draw-free, and unreachable from a replay (no capture runs the host).
    fn carry_affects_home(&mut self, ctx: &mut FlowCtx) {
        let carried: Vec<(usize, Vec<Vec<u8>>)> = self
            .state
            .roster()
            .iter()
            .enumerate()
            .filter(|(_, f)| f.team == Team::Party && !f.non_team_member)
            .map(|(actor, f)| {
                (
                    actor,
                    f.affects.iter().map(|a| a.encode().to_vec()).collect(),
                )
            })
            .collect();
        for (actor, chain) in carried {
            let member = self.party_member_of(ctx, actor);
            if let Some(ch) = ctx.roster.members.get_mut(member) {
                ch.affects = chain;
            }
        }
    }

    /// The `party.members` index behind a roster index — the fight's party run
    /// is the living members in order ([`kits::party_kits`]).
    fn party_member_of(&self, ctx: &FlowCtx, actor: usize) -> usize {
        ctx.roster
            .members
            .iter()
            .enumerate()
            .filter(|(_, c)| c.hit_point_current > 0)
            .nth(actor)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// The mid-combat character sheet (`viewPlayer`, `ovr020.cs:236-339`), with
    /// the palette swapped back to normal for it (`:240`).
    fn render_sheet(&self, ctx: &mut FlowCtx, member: usize) {
        crate::combat::scene::render::palette_normal(ctx.fb);
        ctx.fb.clear(0);
        let Some(character) = ctx.roster.members.get(member) else {
            return;
        };
        let view = crate::charsheet::sheet_view(character);
        crate::charsheet::render_sheet(ctx.fb, ctx.font, ctx.symbols, &view);
        crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, "Exit");
    }

    fn note_round(&mut self, step: CombatStep) {
        if let CombatStep::RoundEnded { round, .. } = step {
            self.rounds = round;
        }
    }

    fn take_batch(&self) -> Vec<ActionEvent> {
        std::mem::take(&mut *self.batch.borrow_mut())
    }

    fn render(&self, ctx: &mut FlowCtx) {
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        scene
            .render_frame(ctx.fb, ctx.symbols, ctx.font)
            .unwrap_or_else(|e| panic!("the combat scene failed to render: {e:?}"));
    }

    /// D-CV7's rebuild-on-load: a fight restored from a snapshot has no scene
    /// (it is `#[serde(skip)]`), so the first tick after a restore rebuilds one
    /// from the `CombatState` the host carried. Playback resumes at a step
    /// boundary rather than mid-beat — the presented board is reconciled by
    /// construction, so nothing downstream can tell.
    fn rebuild_scene_if_missing(&mut self, ctx: &mut FlowCtx) {
        if self.scene.is_some() {
            return;
        }
        // The sink is `#[serde(skip)]` on `CombatState` too, so a restored
        // fight has none — re-attach before the next `step()` buffers a batch.
        self.state
            .attach_action_sink(Box::new(BatchSink(Rc::clone(&self.batch))));
        let art = self.art.clone();
        let scene_art = self.load_art(ctx, &art);
        let mut scene = CombatScene::new(
            EntrySnapshot::from_state(&self.state, &self.identities),
            scene_art,
        );
        scene.set_weapon_names(self.weapon_names.clone());
        scene.refresh_panels(&self.state);
        let _ = scene.reconcile(&self.state);
        self.scene = Some(scene);
    }

    /// `SetupGroundTiles`' art half + `BattleSetup`'s icon loads: ground tiles
    /// for the area kind, `CHEAD`/`CBODY` per party member, `CPIC` per monster
    /// type, over boot's already-resident COMSPR store.
    ///
    /// Art failures are **not** fatal to a fight: an unloaded icon slot simply
    /// draws nothing (`ovr034.cs:92`), which is the same outcome a fixture
    /// engine (no real data) has. The failure is reported, then the fight goes
    /// on — losing a picture must never lose the game.
    fn load_art(&self, ctx: &mut FlowCtx, art: &FightArt) -> SceneArt {
        let mut icons: CombatIcons = ctx.combat_icons.clone();
        let note = |ctx: &mut FlowCtx, what: String| {
            ctx.vm_memory
                .transcript
                .push(crate::vmhost::TranscriptEntry::Request(what));
        };

        for info in &art.party_icons {
            let slot = info.icon_id as usize;
            if slot >= COMBAT_ICON_SLOTS {
                continue;
            }
            match combat_art::load_party_icon(ctx.data, info, true) {
                Ok(icon) => {
                    icons.set(slot, icon);
                }
                Err(e) => note(ctx, format!("combat art: party icon slot {slot}: {e:?}")),
            }
        }
        for (&slot, &block) in &art.monster_blocks {
            if slot as usize >= COMBAT_ICON_SLOTS {
                continue;
            }
            match combat_art::load_monster_icon(ctx.data, art.cpic_area, block) {
                Ok(icon) => {
                    icons.set(slot as usize, icon);
                }
                Err(e) => note(
                    ctx,
                    format!("combat art: CPIC{} block {block}: {e:?}", art.cpic_area),
                ),
            }
        }
        let tiles = match combat_art::load_ground_tiles(ctx.data, art.in_dungeon) {
            Ok(t) => t,
            Err(e) => {
                note(ctx, format!("combat art: ground tiles: {e:?}"));
                Default::default()
            }
        };
        SceneArt::new(tiles, icons)
    }

    // --- introspection (tests, the inspector) ------------------------------

    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    /// A **boundary** read of the fight — never call it mid-playback (D-CV2's
    /// end-of-STEP trap).
    pub fn state(&self) -> &CombatState {
        &self.state
    }

    pub fn scene(&self) -> Option<&CombatScene> {
        self.scene.as_ref()
    }

    /// The open manual turn's UI, if one is suspended — what a scripted player
    /// (a demo, a test) reads to decide its next key.
    pub fn manual(&self) -> Option<&ManualUi> {
        self.manual.as_ref()
    }

    pub fn outcome(&self) -> Option<CombatOutcome> {
        self.outcome
    }

    pub fn rounds(&self) -> u16 {
        self.rounds
    }

    /// Keys this slice queued and dropped (SPACE — slice 7's TurnCmd).
    pub fn dropped_keys(&self) -> &[u8] {
        &self.dropped_keys
    }
}

/// The resident `ITEMS` table, parsed from the data set at fight entry.
///
/// coab loads it once at boot into `gbl.ItemDataTable`; parsing it here instead
/// is observationally identical (the file is immutable game data) and keeps
/// boot's asset set — and every `.rsav` golden that depends on it — unchanged.
/// `None` when the data set carries no `ITEMS` file, which is exactly the CI
/// fixture case: no loadouts, and every combatant fights from its record.
fn load_item_data(ctx: &FlowCtx) -> Option<ItemDataTable> {
    ItemDataTable::parse(ctx.data.raw_file(ITEMS_FILE)?).ok()
}

/// Readied-weapon display names for the right panel (`ovr025.cs:292-335`'s
/// weapon line), keyed by `ITEMS` type — read from the party's own inventory,
/// which is the only place item *names* exist (`ItemDisplayNameBuild`'s data).
fn weapon_display_names(ctx: &FlowCtx) -> BTreeMap<u8, String> {
    let mut names = BTreeMap::new();
    for member in &ctx.roster.members {
        for item in &member.items {
            let Some(&item_type) = item.get(0x2E) else {
                continue;
            };
            let name = gbx_formats::save_orig::item_name(item);
            if !name.is_empty() {
                names.entry(item_type).or_insert(name);
            }
        }
    }
    names
}
