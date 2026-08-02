//! The **presented board** (`docs/design/combat-visualizer.md` D-CV2): the
//! roster/map/camera state *as of the playback cursor*, started from the entry
//! snapshot and advanced by [`ActionEvent`]s alone.
//!
//! The whole point is that nothing here reads a live [`CombatState`]:
//! `step()` has already run the entire turn by the time playback begins, so a
//! mid-playback read would show end-of-step state (the §49 "end-of-STEP state"
//! trap, now a design rule). The board therefore holds **no** reference to a
//! `CombatState` at all — the only place one appears is
//! [`PresentedBoard::reconcile`], which the host calls at a step boundary,
//! where a read is legal.
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - `engine/ovr033.cs` `CombatantKilled` (`:534-611`) — the downed-party
//!   `Tile_DownPlayer` stamp, its `nonTeamMember == false` gate, its
//!   `Tile_StinkingCloud` exception, and the `DownedPlayerTile` save list.
//! - `engine/ovr033.cs` `sub_7515A` (`:614-668`) — the restore half: the
//!   `FindLast(… originalBackgroundTile != Tile_DownPlayer)` pick, the
//!   `RemoveAll` for that combatant, and the "another body is still here"
//!   guard that suppresses the write.
//! - `engine/ovr024.cs` `RemoveFromCombat` (`sub_644A7`, coab `:600-647`) —
//!   `in_combat = false`, the status write, and the `running` special case
//!   that **skips** `hp_current = 0` (mirrored by
//!   [`crate::combat::CombatState::remove_from_combat`]).
//! - `engine/ovr025.cs:1197-1240` (`damage_player`) — the health-status
//!   ladder the `Removed` reason reports back.
//! - `engine/ovr014.cs:252-321` (`sub_3E748`) — a movement step stores the
//!   heading in `actions.direction`, which is why [`ActionEvent::Move`]'s
//!   delta is enough to turn the presented icon.

use super::SceneError;
use crate::combat::{
    map_dir_delta, ActionEvent, CombatMap, CombatState, GridPos, HealKind, HealthStatus,
    RemovalReason, Team, TILE_DOWN_PLAYER, TILE_STINKING_CLOUD,
};
use crate::combat_art::IconPose;

/// One combatant as the presented board sees it.
///
/// The **event-carried** fields — `pos`, `hp_current`, `health_status`,
/// `in_combat` — are the ones [`PresentedBoard::reconcile`] asserts against
/// the real roster. `direction`/`pose` are presentation-only (the original
/// stores facing from draws, and no event carries it), and `name`/`icon_slot`/
/// `ac`/`hp_max`/`team`/`size`/`non_team_member` are entry-snapshot constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedCombatant {
    /// Roster index (the D-OR3 `combatant_id`).
    pub id: usize,
    /// Display name — **host-supplied**: `Combatant` has no name field (doc
    /// §2), the harnesses decode it from the capture record.
    pub name: String,
    pub team: Team,
    /// `actions.nonTeamMember` — gates the downed-body tile stamp.
    pub non_team_member: bool,
    /// The `gbl.combat_icons` slot this combatant draws from
    /// (`player.icon_id`): 0–7 party, 8+ per monster type. Also host-supplied
    /// — captures carry no monster CPIC ids (doc §4/M6a's sidecar).
    pub icon_slot: usize,
    /// Footprint size (`CombatMap[i].size`, `field_DE & 7`).
    pub size: u8,
    pub pos: GridPos,
    /// `actions.direction` (0–7). Advanced by `Move` deltas during playback
    /// and refreshed from the boundary read in [`PresentedBoard::reconcile`].
    pub direction: u8,
    /// Which of the icon's two frames to draw. Playback (slice 4) flips this
    /// to [`IconPose::Attack`] for the 100 ms swing hold; slice 3 leaves it
    /// at Normal, which is what every full redraw uses (`redrawCombatArea`
    /// passes `Icon.Normal`).
    pub pose: IconPose,
    pub hp_current: i32,
    pub hp_max: i32,
    /// Raw on-disk AC (display AC = `0x3C - ac`).
    pub ac: u8,
    pub health_status: HealthStatus,
    pub in_combat: bool,
}

impl PresentedCombatant {
    /// Whether this combatant's icon draws at all — `redrawCombatArea`'s
    /// `in_combat == true && CombatMap[i].size > 0` gate (`ovr033.cs:349`).
    /// A removed combatant has its map size zeroed, so it vanishes; a downed
    /// *party* member leaves the body tile behind instead (see
    /// [`PresentedBoard::apply`]).
    pub fn on_board(&self) -> bool {
        self.in_combat && self.size > 0
    }
}

/// One saved body tile — `gbl.downedPlayers`' `DownedPlayerTile`
/// (`ovr033.cs:592-598`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownedTile {
    pub combatant_id: usize,
    pub map: GridPos,
    /// The ground tile that was there before the body was stamped.
    pub original_tile: u8,
}

/// The presented board: roster + map + camera at the playback cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentedBoard {
    combatants: Vec<PresentedCombatant>,
    map: CombatMap,
    camera_top_left: GridPos,
    downed: Vec<DownedTile>,
    /// `RedrawPlayerBackground(idx)` (`ovr033.cs:556`) with nothing painted
    /// back over it: the combatant is still on the board in every other sense,
    /// its cell just shows bare ground. `CombatantKilled` erases its victim
    /// this way before the death flash plays over the cleared cell.
    icon_suppressed: Option<usize>,
}

impl PresentedBoard {
    /// Starts a board from an entry snapshot. `combatants` must be in roster
    /// order (index == `combatant_id`) — the id is the index every event uses.
    pub fn new(
        combatants: Vec<PresentedCombatant>,
        map: CombatMap,
        camera_top_left: GridPos,
    ) -> Self {
        debug_assert!(
            combatants.iter().enumerate().all(|(i, c)| c.id == i),
            "the presented roster is indexed by combatant_id"
        );
        PresentedBoard {
            combatants,
            map,
            camera_top_left,
            downed: Vec::new(),
            icon_suppressed: None,
        }
    }

    pub fn combatants(&self) -> &[PresentedCombatant] {
        &self.combatants
    }

    pub fn combatant(&self, id: usize) -> Option<&PresentedCombatant> {
        self.combatants.get(id)
    }

    /// The presented map — the entry snapshot's terrain plus whatever body
    /// tiles playback has stamped into it.
    pub fn map(&self) -> &CombatMap {
        &self.map
    }

    /// The 7×7 window's top-left map cell (`mapScreenTopLeft`).
    pub fn camera_top_left(&self) -> GridPos {
        self.camera_top_left
    }

    /// The saved body tiles, in stamp order (`gbl.downedPlayers`).
    pub fn downed_tiles(&self) -> &[DownedTile] {
        &self.downed
    }

    /// Sets the pose and facing playback wants drawn for one combatant —
    /// `draw_74B3F(_, iconState, direction, combatant)`'s two presentation
    /// effects (`ovr033.cs:376-396`). Presentation-only: reconciliation
    /// refreshes both from the boundary read rather than asserting them.
    pub fn set_pose(&mut self, id: usize, pose: IconPose, direction: u8) {
        if let Some(c) = self.combatants.get_mut(id) {
            c.pose = pose;
            c.direction = direction;
        }
    }

    /// `RedrawPlayerBackground(idx)` — suppress one combatant's icon (or
    /// `None` to put every icon back). See [`Self::icon_suppressed`].
    pub fn set_icon_suppressed(&mut self, id: Option<usize>) {
        self.icon_suppressed = id;
    }

    /// Whether this combatant's icon is currently erased.
    pub fn icon_suppressed(&self, id: usize) -> bool {
        self.icon_suppressed == Some(id)
    }

    /// `combatantMap.screenPos = pos − mapScreenTopLeft`
    /// (`calculatePlayerScreenPositions`, `ovr033.cs:34-40`). May be negative
    /// or past the window — [`Self::on_screen`] is the test.
    pub fn screen_pos(&self, pos: GridPos) -> (i32, i32) {
        (
            pos.x - self.camera_top_left.x,
            pos.y - self.camera_top_left.y,
        )
    }

    /// `CoordOnScreen(pos − mapScreenTopLeft)` (`ovr033.cs:203`) for one cell.
    pub fn on_screen(&self, pos: GridPos) -> bool {
        let (sx, sy) = self.screen_pos(pos);
        let max = crate::combat::SCREEN_MAX;
        (0..=max).contains(&sx) && (0..=max).contains(&sy)
    }

    /// `PlayerOnScreen(false, combatant)` (`ovr033.cs:227-257`): **any** cell
    /// of the footprint inside the window. A removed combatant (`size == 0`)
    /// is off-screen by definition, which is the original's early return.
    pub fn combatant_on_screen(&self, c: &PresentedCombatant) -> bool {
        c.size > 0
            && crate::combat::size_footprint(c.size, c.pos)
                .into_iter()
                .any(|cell| self.on_screen(cell))
    }

    /// `PlayerIndexAtMapXY(pos)` (`ovr033.cs:135-148`) over the presented
    /// roster: whoever's footprint covers `pos`. The occupancy grid is
    /// painted in roster order (`sub_743E7`, `ovr033.cs:117-129`), so a later
    /// index overwrites an earlier one — hence the reverse scan.
    pub fn occupant_at(&self, pos: GridPos) -> Option<&PresentedCombatant> {
        self.combatants
            .iter()
            .rev()
            .find(|c| c.on_board() && crate::combat::size_footprint(c.size, c.pos).contains(&pos))
    }

    /// Applies one event to the board. Playback calls this for every event in
    /// the current step's batch, in emission order.
    ///
    /// Only the four D-CV2 scopes are mutated here — position, hp, health
    /// status, in-combat — plus the camera (also event-carried) and the
    /// presentation-only facing a `Move` implies. Every other variant is an
    /// explicit no-op arm: the match is **exhaustive on purpose**, mirroring
    /// the oracle collector's rule (a `_ =>` catch-all would silently swallow
    /// a future board-bearing event).
    pub fn apply(&mut self, event: ActionEvent) {
        match event {
            ActionEvent::Camera { top_left } => self.camera_top_left = top_left,

            ActionEvent::Move {
                combatant_id,
                from_x,
                from_y,
                to_x,
                to_y,
                ..
            } => {
                let to = GridPos::new(to_x, to_y);
                if let Some(c) = self.combatants.get_mut(combatant_id) {
                    debug_assert_eq!(
                        (c.pos.x, c.pos.y),
                        (from_x, from_y),
                        "presented position drifted from the event's `from` for combatant {combatant_id}"
                    );
                    if let Some(dir) = direction_of_step(from_x, from_y, to_x, to_y) {
                        c.direction = dir;
                    }
                    c.pos = to;
                }
            }

            ActionEvent::Dmg {
                target_id, amount, ..
            } => {
                if let Some(c) = self.combatants.get_mut(target_id) {
                    // `damage_player`'s `new_hp = hp_current − damage`
                    // (`ovr025.cs:1195`). The clamp to 0 is the removal's
                    // job, and arrives as `Removed`.
                    c.hp_current -= amount;
                }
            }

            ActionEvent::Healed {
                target_id,
                amount,
                kind,
                ..
            } => {
                if let Some(c) = self.combatants.get_mut(target_id) {
                    match kind {
                        // `spell_cure_light`: `hp = min(hp + roll, hp_max)`,
                        // and a `dead` target does not heal at all.
                        HealKind::Cure => {
                            if c.health_status != HealthStatus::Dead {
                                c.hp_current = (c.hp_current + amount).min(c.hp_max);
                            }
                        }
                        // `bandage(true)` (`ovr025.cs:1628`): no hp, just
                        // `dying → unconscious`.
                        HealKind::Bandage => {
                            if c.health_status == HealthStatus::Dying {
                                c.health_status = HealthStatus::Unconscious;
                            }
                        }
                    }
                }
            }

            // The spell-damage twin of `Dmg` — same board effect, its own
            // event because the wire's `Dmg` means a weapon roll (see the
            // variant's own doc).
            ActionEvent::SpellDamage {
                target_id, amount, ..
            } => {
                if let Some(c) = self.combatants.get_mut(target_id) {
                    c.hp_current -= amount;
                }
            }

            // `AffectRegen3Hp`'s silent round-end tick. `amount` is already
            // capped by the emitter, so this is a plain add.
            ActionEvent::Regenerated {
                combatant_id,
                amount,
            } => {
                if let Some(c) = self.combatants.get_mut(combatant_id) {
                    c.hp_current += amount;
                }
            }

            ActionEvent::Bled { combatant_id, died } => {
                if died {
                    if let Some(c) = self.combatants.get_mut(combatant_id) {
                        c.health_status = HealthStatus::Dead;
                    }
                }
            }

            ActionEvent::Removed {
                combatant_id,
                reason,
            } => self.apply_removal(combatant_id, reason),

            // --- draws nothing structural: beats, messages, dice ----------
            //
            // These drive slice 4's timeline (poses, missiles, text, sounds);
            // none of them moves the board. Listed one by one so a new
            // variant has to be considered here rather than silently ignored.
            ActionEvent::Init { .. }
            | ActionEvent::Pick { .. }
            | ActionEvent::Attack { .. }
            | ActionEvent::Save { .. }
            | ActionEvent::Ai { .. }
            | ActionEvent::Morale { .. }
            | ActionEvent::StubTripped { .. }
            | ActionEvent::ContinueBattlePrompt { .. }
            | ActionEvent::Missile { .. }
            | ActionEvent::BeginsCasting { .. }
            | ActionEvent::Cast { .. }
            | ActionEvent::SpellTarget { .. }
            | ActionEvent::SlayHelpless { .. }
            | ActionEvent::Sound { .. }
            | ActionEvent::Attacking { .. }
            | ActionEvent::Flees { .. } => {}
        }
    }

    /// The `Removed` half of [`Self::apply`], split out because it is the one
    /// event that touches the *map*.
    fn apply_removal(&mut self, id: usize, reason: RemovalReason) {
        let Some(c) = self.combatants.get_mut(id) else {
            return;
        };
        c.in_combat = false;
        match reason {
            // The damage path (`CombatantKilled`): the ladder already put the
            // status where it belongs and hp is zeroed at removal
            // (`ovr025.cs:1220-1222`). The reason reports which rung, so the
            // board can carry it without a second event.
            RemovalReason::Killed => {
                c.health_status = HealthStatus::Dead;
                c.hp_current = 0;
            }
            RemovalReason::Downed { dying } => {
                c.health_status = if dying {
                    HealthStatus::Dying
                } else {
                    HealthStatus::Unconscious
                };
                c.hp_current = 0;
            }
            // `RemoveFromCombat(_, running)` — the Got-Away case KEEPS its hp
            // (`sub_644A7:151A`, mirrored in `ai.rs:132`).
            RemovalReason::Fled => c.health_status = HealthStatus::Running,
            RemovalReason::Surrendered => {
                c.health_status = HealthStatus::Unconscious;
                c.hp_current = 0;
            }
        }

        // `CombatMap[idx].size = 0` (`ovr033.cs:598`, `sub_644A7:154A`) — the
        // footprint frees immediately, which is also what stops the icon from
        // drawing.
        c.size = 0;

        // The body tile: `CombatantKilled` only, and only for a real team
        // member (`ovr033.cs:579`'s `nonTeamMember == false`, modeled as
        // `team == Party && !non_team_member` exactly as `attack.rs:155`
        // does). Flee/surrender go through `RemoveFromCombat`, which stamps
        // nothing.
        let stamps_body = matches!(reason, RemovalReason::Killed | RemovalReason::Downed { .. })
            && c.team == Team::Party
            && !c.non_team_member;
        let pos = c.pos;
        if stamps_body {
            let original_tile = self.map.ground_tile(pos);
            // The save list records the tile that was there even when the
            // stamp is suppressed — coab pushes the entry and fills
            // `originalBackgroundTile` first, and only then tests the cloud
            // (`ovr033.cs:578-590`).
            self.downed.push(DownedTile {
                combatant_id: id,
                map: pos,
                original_tile,
            });
            if original_tile != TILE_STINKING_CLOUD {
                self.map.set_tile(pos, TILE_DOWN_PLAYER);
            }
        }
    }

    /// `sub_7515A`'s restore half (`ovr033.cs:637-655`): a combatant that is
    /// placed back on the board at `pos` puts the saved ground tile back.
    ///
    /// Faithfully: the tile written is the **last** saved entry for this
    /// combatant whose `original_tile` isn't itself a body tile (a body that
    /// fell on a body restores the floor, not another body); with no such
    /// entry the cell keeps what it already has (coab's `ground_tile` starts
    /// as `getGroundInformation`'s read at the new cell and is only
    /// *overwritten* by a saved tile). All of that combatant's entries are
    /// then dropped, and if some **other** body is still recorded at `pos`
    /// the write is suppressed so that body's tile stays. Returns the tile
    /// written, or `None` when suppressed.
    ///
    /// Unreachable inside a fight today (nothing re-places a downed member
    /// mid-combat), which is exactly why it is transcribed and tested rather
    /// than left to be re-derived when raise-dead lands.
    pub fn restore_downed(&mut self, id: usize, pos: GridPos) -> Option<u8> {
        let saved = self
            .downed
            .iter()
            .rev()
            .find(|d| d.combatant_id == id && d.original_tile != TILE_DOWN_PLAYER)
            .map(|d| d.original_tile);
        self.downed.retain(|d| d.combatant_id != id);
        let tile = saved.unwrap_or_else(|| self.map.ground_tile(pos));
        if self.downed.iter().any(|d| d.map == pos) {
            return None;
        }
        self.map.set_tile(pos, tile);
        Some(tile)
    }

    /// The D-CV2 **step-boundary reconciliation**: every event-carried field
    /// of the presented board must equal the real roster's, or playback has
    /// drifted from the fight it is presenting.
    ///
    /// Scope is exactly the event-carried set — position, hp, health status,
    /// in-combat, plus the camera ([`ActionEvent::Camera`]'s own field).
    /// Panel-only fields (`readied_weapon`, `ac`, …) are deliberately
    /// excluded: no event carries them and the panel reads them here, at the
    /// boundary, instead.
    ///
    /// Debug-asserted (the release build pays nothing) and additionally
    /// returned as a `Result` so a host that wants it loud in release can
    /// check. Facing is *refreshed* rather than asserted — the original
    /// stores `actions.direction` from draw sites no event models, so the
    /// boundary read is the scene's only truthful source for it.
    pub fn reconcile(&mut self, state: &CombatState) -> Result<(), SceneError> {
        let roster = state.roster();
        if roster.len() != self.combatants.len() {
            let err = SceneError::RosterSizeMismatch {
                presented: self.combatants.len(),
                actual: roster.len(),
            };
            debug_assert!(false, "{err:?}");
            return Err(err);
        }
        for (presented, actual) in self.combatants.iter_mut().zip(roster.iter()) {
            let mismatch = |field: &'static str, presented_value: String, actual_value: String| {
                SceneError::BoardDrift {
                    combatant_id: actual.id,
                    field,
                    presented: presented_value,
                    actual: actual_value,
                }
            };
            let err = if presented.pos != actual.pos {
                Some(mismatch(
                    "pos",
                    format!("{:?}", presented.pos),
                    format!("{:?}", actual.pos),
                ))
            } else if presented.hp_current != actual.hp_current {
                Some(mismatch(
                    "hp_current",
                    presented.hp_current.to_string(),
                    actual.hp_current.to_string(),
                ))
            } else if presented.health_status != actual.health_status {
                Some(mismatch(
                    "health_status",
                    format!("{:?}", presented.health_status),
                    format!("{:?}", actual.health_status),
                ))
            } else if presented.in_combat != actual.in_combat {
                Some(mismatch(
                    "in_combat",
                    presented.in_combat.to_string(),
                    actual.in_combat.to_string(),
                ))
            } else {
                None
            };
            if let Some(err) = err {
                debug_assert!(false, "{err:?}");
                return Err(err);
            }
            // Presentation-only refresh (not an assert — see the doc above).
            presented.direction = actual.direction;
            presented.pose = IconPose::Normal;
        }
        if self.camera_top_left != state.camera_top_left() {
            let err = SceneError::CameraDrift {
                presented: self.camera_top_left,
                actual: state.camera_top_left(),
            };
            debug_assert!(false, "{err:?}");
            return Err(err);
        }
        Ok(())
    }
}

/// The iso direction a one-cell step took, by looking its delta up in
/// `MapDirectionDelta` (`Gbl.cs:690`) — `sub_3E748` stores exactly this into
/// `actions.direction` before moving (`ovr014.cs:283`). `None` for a
/// zero-length or multi-cell delta (neither can come out of a `Move`, which
/// is emitted per cell).
fn direction_of_step(from_x: i32, from_y: i32, to_x: i32, to_y: i32) -> Option<u8> {
    let delta = (to_x - from_x, to_y - from_y);
    (0..8u8).find(|&d| map_dir_delta(d) == delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_of_step_covers_all_eight_headings() {
        for dir in 0..8u8 {
            let (dx, dy) = map_dir_delta(dir);
            assert_eq!(direction_of_step(10, 10, 10 + dx, 10 + dy), Some(dir));
        }
        assert_eq!(direction_of_step(10, 10, 10, 10), None, "no move");
        assert_eq!(direction_of_step(10, 10, 12, 10), None, "two cells");
    }
}
