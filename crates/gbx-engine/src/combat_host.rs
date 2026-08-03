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
use crate::combat::scene::{
    strings, time::BeatClock, CombatScene, CombatantIdentity, EntrySnapshot, SceneArt,
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
    /// The last frame, held one `GameDelay` beat before the screen restores.
    FinalBeats { ticks_left: u32 },
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
    /// Keys pressed during AI turns that this slice cannot honour yet — SPACE
    /// (quick-fight revoke) needs D-CV5's suspensions. Recorded so a dropped
    /// key is *reported*, never silently eaten (§8.2).
    dropped_keys: Vec<u8>,
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
            dropped_keys: self.dropped_keys.clone(),
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
            return Err(CombatHostError::NothingToFight);
        }

        let map_dir = crate::shell::facing_to_map_dir(ctx.state.facing);
        let in_dungeon = matches!(ctx.state.game_state, GameState::DungeonMap);
        let party_pos = (ctx.state.pos.0 as i32, ctx.state.pos.1 as i32);
        let dist = crate::combat::encounter_distance(
            ctx.geo,
            map_dir,
            party_pos.0,
            party_pos.1,
            in_dungeon,
        );

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
                0, // delay — CalculateInitiative sets it each round
                1, // one swing/round
            ));
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
                cpic_area: ctx.game_area,
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
                cpic_area: ctx.game_area,
                monster_blocks: BTreeMap::new(),
                in_dungeon: true,
                party_icons: Vec::new(),
            },
            weapon_names: BTreeMap::new(),
            rounds: 0,
            ended: false,
            outcome: None,
            dropped_keys: Vec::new(),
            scene: None,
            batch: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// One tick of the §8.2 chart.
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> HostTick {
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
            Stage::FinalBeats { ticks_left } => {
                let left = ticks_left.saturating_sub(ctx.dt_ticks);
                self.stage = if left > 0 {
                    Stage::FinalBeats { ticks_left: left }
                } else {
                    Stage::Restore
                };
                self.render(ctx);
                HostTick::Working
            }
            Stage::Restore => {
                // `free_combat_stuff` (`ovr009.cs:9`) + `CMD_Combat`'s `LoadPic`
                // rebuild (`ovr003.cs:971`): the palette goes back and the
                // exploration screen is recomposed. Only after this does the
                // caller perform the deferred writes (§8.2's MUST).
                crate::combat::scene::render::palette_normal(ctx.fb);
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
        self.render(ctx);
    }

    /// D-CV2's lockstep loop, verbatim: present step N to completion **before**
    /// calling `step()` for N+1. There is no pipelining and no rollback.
    fn tick_fighting(&mut self, ctx: &mut FlowCtx) {
        self.rebuild_scene_if_missing(ctx);
        let scene = self.scene.as_mut().expect("just rebuilt");
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
        self.render(ctx);
    }

    /// Live input, applied at **step heads only** — the D-CV2 lockstep rule,
    /// mirroring the original's own `sub_36269` head polls.
    ///
    /// `'2'` (the auto-magic toggle, `ovr010.cs:718-730`) works from M6b: it is
    /// a plain flag flip the next turn's spell gate reads. SPACE (quick-fight
    /// revoke) needs D-CV5's suspensions and is **queued and dropped with a
    /// transcript note** until slice 7 — never silently eaten (§8.2).
    fn drain_input(&mut self, ctx: &mut FlowCtx) {
        let at_step_head = matches!(self.stage, Stage::Fighting)
            && self.scene.as_ref().is_some_and(|s| !s.is_playing());
        while let Some(event) = ctx.input.read_key() {
            let crate::input::InputEvent::Char(key) = event else {
                continue;
            };
            match key {
                b'2' => {
                    if at_step_head {
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
                }
                b' ' => {
                    self.dropped_keys.push(key);
                    ctx.vm_memory
                        .transcript
                        .push(crate::vmhost::TranscriptEntry::Request(
                            "combat: SPACE (quick-fight revoke) queued and dropped — \
                             manual turns land with the D-CV5 suspensions (slice 7)"
                                .to_string(),
                        ));
                }
                _ => {}
            }
        }
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
