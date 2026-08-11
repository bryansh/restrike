//! ★ **The temple on screen** (`temple_shop`/`temple_heal`/`buy_cure`,
//! `ovr005.cs:285-505`) — roll-credits slice 6 / G8's interactive half.
//!
//! Parked as an **interaction**, not a top-level [`Screen`](crate::screens::Screen):
//! `CMD_Combat` calls `temple_shop()` inline and the script resumes on the next
//! instruction, exactly as it does around a fight. So a [`TempleHost`] lives in
//! [`VmPhase::Temple`](crate::shell::VmPhase) beside the combat host, the
//! `VectorRun` that yielded `Request::Combat` stays suspended mid-`Present`,
//! and the reply goes out when the temple closes.
//!
//! ## The two menus
//!
//! ```text
//! temple_shop   "Heal View Take Pool Share Appraise Exit"   (money on the ground)
//!               "Heal View Pool Appraise Exit"              (none)
//!   └─ H → temple_heal: sl_select_item over the ten services, "Heal Exit"
//!            └─ a service → [is not X. → "cast cure anyway: "]
//!                           "<name> will only cost N gold pieces."  (press a key)
//!                           "pay for cure "  Yes/No
//!                           → paid: "<name> is cured." | "Not enough money."
//! ```
//!
//! `temple_shop` **clears the pool on entry** (`gbl.pooled_money.ClearAll()`,
//! `ovr005.cs:406`), so the Pool word is the only way to get coins into it —
//! and `buy_cure`'s fallback then finds them. Leaving with money still pooled
//! is the priest's "Excuse me but you have left some money here" prompt
//! (`:449-467`), whose Yes answer keeps you in the temple.

use crate::money::MoneySet;
use crate::screens::ScreenTransition;
use crate::shell::FlowCtx;
use crate::temple::{self, Payment};
use crate::widgets::{Hotbar, ListItem, ListLayout, ListMenu, Widget, WidgetOutcome};

/// Where a parked temple visit is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum Stage {
    /// `temple_shop`'s `displayInput` loop (`ovr005.cs:410-440`).
    Shop,
    /// `temple_heal`'s `sl_select_item` list (`:311-317`).
    Heal,
    /// `CastCureAnyway` (`:17-25`) — `"<name> is not blind."` + `"cast cure
    /// anyway: "` Yes/No.
    CastAnyway { service: usize },
    /// `buy_cure`'s price line (`:30`) — a blocking `press_any_key`.
    Price { service: usize },
    /// `buy_cure`'s `yes_no("pay for cure ")` (`:35`).
    Pay { service: usize },
    /// `press_any_key` on the priest's leftover-money question (`:449-467`).
    LeavingWithMoney,
    /// Done: the reply goes out.
    Done,
}

/// ★ A temple visit, parked inside a suspended `VectorRun`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TempleHost {
    stage: Stage,
    /// The `sl_select_item` cursor, which `temple_heal` keeps across services
    /// (`int sl_index = 0` lives outside the loop, `ovr005.cs:287`).
    list: ListMenu,
    /// The last line the temple said — the `DisplayPlayerStatusString` slot.
    status: Option<String>,
    /// Set the moment a paid service lands, so a test/demo can assert it.
    served: Vec<(usize, String)>,
}

/// `temple_heal`'s list box (`sl_select_item(..., 15, 0x26, 4, 2, ...)`,
/// `ovr005.cs:314`) — rows 4..=15, columns 2..=0x26.
const HEAL_LAYOUT: ListLayout = ListLayout {
    start_row: 4,
    start_col: 2,
    end_row: 15,
    end_col: 0x26,
};

impl Default for TempleHost {
    fn default() -> Self {
        Self::new()
    }
}

impl TempleHost {
    pub fn new() -> Self {
        TempleHost {
            stage: Stage::Shop,
            list: ListMenu::boxed(
                temple::SERVICES
                    .iter()
                    .map(|s| ListItem::Entry(format!("{}   {} gp", s.name, s.cost)))
                    .collect(),
                HEAL_LAYOUT,
            ),
            status: None,
            served: Vec::new(),
        }
    }

    /// `temple_shop`'s entry (`ovr005.cs:400-407`): the game state changes, the
    /// screen is rebuilt, and **the pool is emptied**.
    pub fn open(ctx: &mut FlowCtx) -> Self {
        ctx.state.pooled_money.clear();
        Self::new()
    }

    /// Which services were bought and applied, in order — the acceptance seam.
    pub fn served(&self) -> &[(usize, String)] {
        &self.served
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        self.paint(ctx);
        match self.stage.clone() {
            Stage::Shop => self.tick_shop(ctx),
            Stage::Heal => self.tick_heal(ctx),
            Stage::CastAnyway { service } => self.tick_cast_anyway(ctx, service),
            Stage::Price { service } => self.tick_price(ctx, service),
            Stage::Pay { service } => self.tick_pay(ctx, service),
            Stage::LeavingWithMoney => self.tick_leaving(ctx),
            Stage::Done => ScreenTransition::Exit,
        }
    }

    /// The word list `temple_shop` composes from what is on the ground
    /// (`ovr005.cs:415-422`).
    fn shop_bar(&self, pool: &MoneySet) -> &'static str {
        if pool.any() {
            "Heal View Take Pool Share Appraise Exit"
        } else {
            "Heal View Pool Appraise Exit"
        }
    }

    fn tick_shop(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let bar = self.shop_bar(&ctx.state.pooled_money);
        let mut hotbar = Hotbar::new(bar);
        hotbar.accept_ext = true;
        let mut widget = Widget::Hotbar(hotbar);
        let WidgetOutcome::Hotbar(key) = widget.tick(ctx.input, ctx.dt_ticks) else {
            return ScreenTransition::Stay;
        };
        match key.to_ascii_uppercase() {
            b'H' => {
                self.status = None;
                self.stage = Stage::Heal;
            }
            // `poolMoney` / `share_pooled` / `TakePoolMoney` — `ovr022`'s own,
            // already transcribed for the fight's treasure screen.
            b'P' => crate::award::pool_money(ctx.roster, &mut ctx.state.pooled_money),
            b'S' => crate::award::share_pooled(ctx.roster, &mut ctx.state.pooled_money),
            b'T' => crate::award::share_pooled(ctx.roster, &mut ctx.state.pooled_money),
            // View is `viewPlayer` and Appraise is `appraiseGemsJewels` — both
            // are their own screens elsewhere; the temple reports rather than
            // pretending (§9's loud-refusal rule).
            b'V' | b'A' => {
                self.status = Some("View/Appraise from the temple: not wired".into());
            }
            b'E' | 0 => {
                if ctx.state.pooled_money.any() {
                    self.stage = Stage::LeavingWithMoney;
                } else {
                    self.stage = Stage::Done;
                }
            }
            _ => {}
        }
        ScreenTransition::Stay
    }

    fn tick_heal(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let mut widget = Widget::ListMenu(self.list.clone());
        let outcome = widget.tick(ctx.input, ctx.dt_ticks);
        if let Widget::ListMenu(list) = widget {
            self.list = list;
        }
        match outcome {
            // `sl_output == 'H' || 0x0D` (`ovr005.cs:316`) — Enter or the Heal
            // word both commit the highlighted row.
            WidgetOutcome::ListSelected { index, .. } => {
                self.status = None;
                self.begin_service(ctx, index);
            }
            // `sl_output == 0` — `sl_select_item` answers Esc, `'\0'` and `'E'`
            // with `'\0'` (`ovr027.cs:652-657`), which is `end_shop = true`.
            WidgetOutcome::ListCancelled => {
                self.stage = Stage::Shop;
            }
            _ => {}
        }
        ScreenTransition::Stay
    }

    /// Enter a service: ask "cast cure anyway" when the member does not
    /// visibly need it, else go straight to the price.
    fn begin_service(&mut self, ctx: &mut FlowCtx, service: usize) {
        if service >= temple::SERVICES.len() {
            return;
        }
        let member = self.selected(ctx);
        let Some(ch) = ctx.roster.members.get(member) else {
            return;
        };
        match temple::not_needed_line(service, ch) {
            Some(line) => {
                self.status = Some(format!("{} {line}", ch.name));
                self.stage = Stage::CastAnyway { service };
            }
            None => self.stage = Stage::Price { service },
        }
    }

    fn tick_cast_anyway(&mut self, ctx: &mut FlowCtx, service: usize) -> ScreenTransition {
        match self.yes_no(ctx, "cast cure anyway: ") {
            Some(true) => self.stage = Stage::Price { service },
            Some(false) => {
                self.status = None;
                self.stage = Stage::Heal;
            }
            None => {}
        }
        ScreenTransition::Stay
    }

    fn tick_price(&mut self, ctx: &mut FlowCtx, service: usize) -> ScreenTransition {
        // `press_any_key(text, true, 10, TextRegion.NormalBottom)` (`:30`).
        if ctx.input.read_key().is_some() {
            self.stage = Stage::Pay { service };
        }
        ScreenTransition::Stay
    }

    fn tick_pay(&mut self, ctx: &mut FlowCtx, service: usize) -> ScreenTransition {
        match self.yes_no(ctx, "pay for cure ") {
            Some(true) => {
                self.buy(ctx, service);
                self.stage = Stage::Heal;
            }
            Some(false) => {
                self.status = None;
                self.stage = Stage::Heal;
            }
            None => {}
        }
        ScreenTransition::Stay
    }

    /// `buy_cure`'s money half, then the service's own effect
    /// (`ovr005.cs:37-58`).
    fn buy(&mut self, ctx: &mut FlowCtx, service: usize) {
        let member = self.selected(ctx);
        let cost = temple::SERVICES[service].cost;
        let Some(ch) = ctx.roster.members.get_mut(member) else {
            return;
        };
        let paid = temple::pay(ch, &mut ctx.state.pooled_money, cost, ctx.rules);
        if paid == Payment::NotEnough {
            // `string_print01("Not enough money.")` (`:49`).
            self.status = Some("Not enough money.".into());
            return;
        }
        // ★ `raise_dead`/`stone_to_flesh` re-test AFTER taking the money
        // (`:174`, `:290-291`): the temple keeps the gold either way.
        let still_qualifies = temple::not_needed_line(service, ch).is_none();
        if temple::requires_the_condition(service) && !still_qualifies {
            self.status = Some(format!("{} is not helped.", ch.name));
            return;
        }
        let name = ch.name.clone();
        let line = temple::apply(service, ch, ctx.rng, ctx.rules);
        self.status = Some(format!("{name} {line}"));
        self.served.push((service, name));
    }

    fn tick_leaving(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        // `press_any_key` twice, then `sub_317AA("~Yes ~No")` (`:452-465`):
        // Yes goes back into the temple, No leaves.
        match self.yes_no(ctx, "retrieve your money: ") {
            Some(true) => self.stage = Stage::Shop,
            Some(false) => self.stage = Stage::Done,
            None => {}
        }
        ScreenTransition::Stay
    }

    /// `yes_no(colors, prompt)` (`ovr027.cs:676-689`) — nothing but Y and N
    /// resolves it; Esc reads as No, as everywhere else in this shell.
    fn yes_no(&self, ctx: &mut FlowCtx, prompt: &str) -> Option<bool> {
        let _ = prompt;
        let mut widget = Widget::Hotbar(Hotbar::new("Yes No"));
        match widget.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Hotbar(key) => match key.to_ascii_uppercase() {
                b'Y' => Some(true),
                b'N' | 0 => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn selected(&self, ctx: &FlowCtx) -> usize {
        (ctx.state.selected_player as usize).min(ctx.roster.members.len().saturating_sub(1))
    }

    /// `DrawFrame_WildernessMap` + the "<name>, how can we help you?" heading
    /// (`ovr005.cs:307-312`), the list, the prompt row, and the party panel the
    /// temple keeps repainting (`PartySummary(gbl.SelectedPlayer)`, `:504`).
    fn paint(&self, ctx: &mut FlowCtx) {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        let member = self.selected(ctx);
        let name = ctx
            .roster
            .members
            .get(member)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        match &self.stage {
            Stage::Shop => {
                crate::text::draw_string(ctx.fb, ctx.font, "Temple", 1, 1, 0, 0x0F);
                let worth = ctx.state.pooled_money.gold_worth();
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    &format!("Pooled: {worth} gp worth"),
                    3,
                    1,
                    0,
                    10,
                );
                self.draw_prompt(ctx, self.shop_bar(&ctx.state.pooled_money));
            }
            Stage::Heal | Stage::CastAnyway { .. } | Stage::Price { .. } | Stage::Pay { .. } => {
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    &format!("{name}, how can we help you?"),
                    1,
                    1,
                    0,
                    0x0F,
                );
                crate::shell::draw_list_menu(ctx.fb, ctx.font, &self.list);
                match &self.stage {
                    Stage::Heal => self.draw_prompt(ctx, "Heal Exit"),
                    Stage::CastAnyway { .. } => self.draw_prompt(ctx, "cast cure anyway: Yes No"),
                    Stage::Price { service } => {
                        crate::text::draw_string(
                            ctx.fb,
                            ctx.font,
                            &temple::price_line(*service),
                            0x11,
                            1,
                            0,
                            10,
                        );
                        self.draw_prompt(ctx, "press <enter>/<return> to continue");
                    }
                    _ => self.draw_prompt(ctx, "pay for cure Yes No"),
                }
            }
            Stage::LeavingWithMoney => {
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    "As you leave a priest says, \"Excuse me but you",
                    3,
                    1,
                    0,
                    10,
                );
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    "have left some money here\"",
                    4,
                    1,
                    0,
                    10,
                );
                self.draw_prompt(ctx, "retrieve your money: Yes No");
            }
            Stage::Done => {}
        }
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 0x16, 1, 0, 0x0E);
        }
        let rows: Vec<crate::charsheet::SheetView> = ctx
            .roster
            .members
            .iter()
            .map(crate::charsheet::sheet_view)
            .collect();
        crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, Some(member));
    }

    fn draw_prompt(&self, ctx: &mut FlowCtx, text: &str) {
        crate::combat::scene::render::draw_menu_line(ctx.fb, ctx.font, text, None);
    }
}
