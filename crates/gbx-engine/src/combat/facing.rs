use super::*;

impl CombatState {
    // --- the combat camera (doc §36.3) -------------------------------------
    //
    // `mapScreenTopLeft` + `focusCombatAreaOnPlayer` are pure display state:
    // the ONLY consumer that changes a draw is `AttackTarget`'s on-screen
    // facing branch (§36.1), so these ports carry ONLY each display function's
    // persistent-state effect — the tile/icon/overlay rendering is stubbed.

    /// `CoordOnScreen(pos)` (`ovr033.cs:213`) for a screen-space cell (already
    /// `map − mapScreenTopLeft`): inside the 7×7 window `0..=6` on both axes.
    pub(super) fn coord_on_screen(screen_x: i32, screen_y: i32) -> bool {
        (0..=SCREEN_MAX).contains(&screen_x) && (0..=SCREEN_MAX).contains(&screen_y)
    }

    /// Is map cell `p` inside the current combat window? (`CoordOnScreen(p −
    /// mapScreenTopLeft)`.) The size-1 form of `PlayerOnScreen` for a cell,
    /// independent of a combatant's `in_combat`/`size` — used at the
    /// `CombatantKilled` scroll, which tests the victim while it is still
    /// present (`ovr033.cs:550`, before `size = 0`).
    pub(super) fn on_screen_pos(&self, p: GridPos) -> bool {
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
    pub(super) fn on_screen(&self, idx: usize) -> bool {
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
    pub(super) fn screen_map_check(&mut self, radius: i32, pos: GridPos) -> bool {
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
    pub(super) fn redraw_combat_area(&mut self, dir: u8, radius: i32, map: GridPos) {
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
    pub(super) fn draw_74b3f(&mut self, idx: usize, direction: u8) {
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
    pub(super) fn draw_missile_camera(&mut self, attacker: usize, target: usize) {
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
}
