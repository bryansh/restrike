//! ★ **The shop on screen** — `CityShop`/`shop_buy`/`ShopChooseItem`
//! (`ovr007.cs:150-272`, `:106-149`, `:8-41`), roll-credits slice 9a.
//!
//! Parked as an **interaction**, not a top-level [`Screen`](crate::screens::Screen)
//! entered from the world: `CMD_Combat`'s non-monster branch calls
//! `CityShop()` inline and the script resumes on the next instruction, exactly
//! as it does around a fight or a temple. So a [`ShopHost`] lives in
//! [`VmPhase::Shop`](crate::shell::VmPhase) beside those two.
//!
//! ## The verb bar, as transcribed
//!
//! `CityShop` composes it fresh every iteration from **one** condition —
//! whether there is money on the ground (`ovr007.cs:176-186`):
//!
//! | condition | bar |
//! |---|---|
//! | `money_on_ground` | `Buy View Take Pool Share Appraise Exit` |
//! | otherwise | `Buy View Pool Appraise Exit` |
//!
//! ★ **There is no `Sell` and no `Id` on this bar.** Both are
//! `PlayerItemsMenu` words, gated on `gbl.game_state == GameState.Shop`
//! (`ovr020.cs:484-493`) — you reach them through `View` → the character sheet
//! → `Items`, which is exactly what [`crate::screens::ReturnTo::Shop`] carries
//! here. The brief's guess at a flat `Buy/Sell/Id/…` bar is not the shipped
//! one.
//!
//! Two keys are on the switch but never in the text: `'G'` and `'O'`, which
//! `scroll_team_list` uses to move `gbl.SelectedPlayer` (`ovr007.cs:248-254`)
//! — the shop's buyer, the sheet's subject, and the seller, all one cell.
//! `'P'` additionally requires `controlKey == false` (`:206-210`).
//!
//! ## Entry and exit
//!
//! Entry (`:154-172`): `game_state = Shop`, `LoadPic`, `PartySummary`, and
//! **`pooled_money.ClearAll()`** — so, as in the temple, `Pool` is the only
//! way to get coins into the pool and `shop_buy`'s pooled-money fallback can
//! only ever spend what this visit pooled.
//!
//! Exit (`:223-246`): with money still on the ground the shopkeeper says
//! `"As you Leave the Shopkeeper says, \"Excuse me but you have Left Some
//! Money here.\""`, then `"Do you want to go back and get your Money?"`, then
//! `~Yes ~No`. `menu_selected == 1` sets `exitShop = true`, and `sub_317AA`
//! returns the **0-based** index into the composed hotkey string
//! (`ovr008.cs:1196-1206`) — so 1 is **No**, and Yes puts the player back in
//! the shop. The temple's copy of this prompt (`ovr005.cs:455-471`) is the
//! same code with different words.
//!
//! ## `shop_buy`
//!
//! `ShopChooseItem` (`:8-41`) builds its list with `list.Insert(0, …)`, so the
//! rows are `gbl.items_pointer` **reversed**; each row is
//! `"{name,-21}{value,9}"`; `gbl.menuSelectedWord = 0`; and the loop only buys
//! while the resolving key is `'B'` or Enter — anything else leaves.
//!
//! The transaction itself is [`crate::shop::buy`].

use crate::money::MoneySet;
use crate::screens::{ReturnTo, Screen, ScreenTransition};
use crate::shell::FlowCtx;
use crate::shop::{self, BuyError, Shop};
use crate::widgets::{Hotbar, ListItem, ListLayout, ListMenu, Widget, WidgetOutcome};
use gbx_formats::items::ItemDataTable;

/// `ShopChooseItem`'s list box (`sl_select_item(…, 0x16, 0x26, 1, 1, …)`,
/// `ovr007.cs:28-29`) — rows 1..=0x16, columns 1..=0x26.
const BUY_LAYOUT: ListLayout = ListLayout {
    start_row: 1,
    start_col: 1,
    end_row: 0x16,
    end_col: 0x26,
};

/// ★ `area2_ptr.field_6DA` (`Classes/Area2.cs:81`, DataOffset `0x6DA`) — the
/// price class `ItemsValue` shifts by. Under the Party window's
/// `DataOffset = (addr - 0x7C00) * 2` mapping that is `0x7C00 + 0x36D`.
///
/// Left in the raw store rather than named in `vmhost.rs`: nothing but the
/// shop reads it, and reading it here keeps a script `SAVE` into the cell and
/// the shop looking at one place.
pub const PRICE_CLASS_ADDR: u16 = 0x7F6D;

/// Where a parked shop visit is.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum Stage {
    /// `CityShop`'s `displayInput` loop (`ovr007.cs:174-270`).
    Shop,
    /// `shop_buy` → `ShopChooseItem` (`:106-149`, `:8-41`).
    Buy,
    /// `View` → `viewPlayer`, and through it the Items leaf whose `Sell`/`Id`
    /// words only exist in a shop.
    Sub(Box<Screen>),
    /// The shopkeeper's leftover-money question (`:223-246`).
    LeavingWithMoney,
    /// Done: the reply goes out.
    Done,
}

/// ★ A shop visit, parked inside a suspended `VectorRun`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShopHost {
    stage: Stage,
    /// The stock, snapshotted from `gbl.items_pointer` on entry. Buying never
    /// removes a row (`ovr007.cs:97` clones), so nothing here changes.
    shop: Shop,
    list: ListMenu,
    status: Option<String>,
    /// Every completed purchase, in order — the acceptance seam.
    bought: Vec<(String, i64)>,
}

impl ShopHost {
    /// `CityShop`'s entry (`ovr007.cs:154-172`): the stock comes from
    /// `gbl.items_pointer` (which the script's own `TREASURE` filled), the
    /// price class from `area2.field_6DA`, and **the pool is emptied**.
    pub fn open(ctx: &mut FlowCtx) -> Self {
        ctx.state.pooled_money.clear();
        let price_class = ctx.vm_memory.raw_word(PRICE_CLASS_ADDR).unwrap_or_default() as u8;
        let items = ctx
            .state
            .treasure_items
            .iter()
            .cloned()
            .map(crate::shop::ShopItem::from_record)
            .collect();
        Self::new(Shop::new(items, price_class), ctx.roster)
    }

    /// A visit over a caller-supplied stock — [`crate::engine::Engine::enter_shop`]'s
    /// direct entry point, and the demos'.
    pub fn new(shop: Shop, roster: &crate::party::Party) -> Self {
        let list = Self::buy_list(&shop, roster);
        ShopHost {
            stage: Stage::Shop,
            shop,
            list,
            status: None,
            bought: Vec::new(),
        }
    }

    /// What the visit bought, for a test or demo to assert.
    pub fn bought(&self) -> &[(String, i64)] {
        &self.bought
    }

    /// One-word stage name, for `RESTRIKE_DEBUG_LOG` and test diagnostics —
    /// the same seam `CombatHost::stage` is.
    pub fn stage_name(&self) -> &'static str {
        match self.stage {
            Stage::Shop => "shop",
            Stage::Buy => "buy",
            Stage::Sub(_) => "sub",
            Stage::LeavingWithMoney => "leaving",
            Stage::Done => "done",
        }
    }

    /// The last line the shop said.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// The stock as `ShopChooseItem` lists it: `gbl.items_pointer` **reversed**
    /// (`list.Insert(0, …)`, `ovr007.cs:20`), each row
    /// `"{name,-21}{value,9}"` with `_value` floored to 1 (`:13-16`).
    fn buy_list(shop: &Shop, roster: &crate::party::Party) -> ListMenu {
        let detect_magic = crate::items::party_has_detect_magic(roster);
        let rows: Vec<ListItem> = shop
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let name = crate::items::display_name(&it.record, detect_magic, false);
                let value = shop.price(i).unwrap_or(1);
                ListItem::Entry(format!("{:<21}{:>9}", name.trim(), value))
            })
            .rev()
            .collect();
        ListMenu::boxed(rows, BUY_LAYOUT)
    }

    /// `ShopChooseItem`'s reversal, undone: display row → stock index.
    fn stock_index(&self, row: usize) -> usize {
        self.shop.items.len().saturating_sub(1).saturating_sub(row)
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        // The sheet/items leaf owns the whole frame while it is up, and
        // `viewPlayer` returns into `CityShop`'s loop when it closes.
        if matches!(self.stage, Stage::Sub(_)) {
            let Stage::Sub(mut screen) = std::mem::replace(&mut self.stage, Stage::Shop) else {
                unreachable!("just matched")
            };
            match screen.tick(ctx) {
                ScreenTransition::To(next) => self.stage = Stage::Sub(Box::new(next)),
                ScreenTransition::Stay => self.stage = Stage::Sub(screen),
                _ => self.stage = Stage::Shop,
            }
            return ScreenTransition::Stay;
        }

        self.paint(ctx);
        match self.stage.clone() {
            Stage::Shop => self.tick_shop(ctx),
            Stage::Buy => self.tick_buy(ctx),
            Stage::LeavingWithMoney => self.tick_leaving(ctx),
            Stage::Sub(_) => unreachable!("handled above"),
            Stage::Done => ScreenTransition::Exit,
        }
    }

    /// The word list `CityShop` composes from what is on the ground
    /// (`ovr007.cs:176-186`).
    fn shop_bar(pool: &MoneySet) -> &'static str {
        if pool.any() {
            "Buy View Take Pool Share Appraise Exit"
        } else {
            "Buy View Pool Appraise Exit"
        }
    }

    fn tick_shop(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let bar = Self::shop_bar(&ctx.state.pooled_money);
        let mut hotbar = Hotbar::new(bar);
        hotbar.accept_ext = true;
        let mut widget = Widget::Hotbar(hotbar);
        let WidgetOutcome::Hotbar(key) = widget.tick(ctx.input, ctx.dt_ticks) else {
            return ScreenTransition::Stay;
        };
        match key.to_ascii_uppercase() {
            b'B' => {
                self.status = None;
                if self.shop.items.is_empty() {
                    // `ShopChooseItem` on an empty list resolves immediately;
                    // saying so is louder than a blank box.
                    self.status = Some("Nothing for sale.".into());
                } else {
                    self.list = Self::buy_list(&self.shop, ctx.roster);
                    self.stage = Stage::Buy;
                }
            }
            // `ovr020.viewPlayer()` (`ovr007.cs:199`) — and `ReturnTo::Shop`
            // is what puts `Sell` and `Id` on the Items leaf's bar.
            b'V' => {
                self.status = None;
                self.stage = Stage::Sub(Box::new(Screen::PartyView(
                    crate::screens::PartyView::new(ctx, ReturnTo::Shop),
                )));
            }
            b'P' => crate::award::pool_money(ctx.roster, &mut ctx.state.pooled_money),
            b'S' => crate::award::share_pooled(ctx.roster, &mut ctx.state.pooled_money),
            // `TakePoolMoney` (`ovr022.cs:350-400`) is a coin-type picker plus
            // an `AskNumberValue` per denomination — slice 3's named residual,
            // reported rather than aliased onto Share.
            b'T' => self.status = Some("Take: the coin-type picker is not wired".into()),
            // `appraiseGemsJewels` (`ovr022.cs`) — the gem-valuation dialog,
            // the same residual the temple reports.
            b'A' => self.status = Some("Appraise: not wired".into()),
            b'G' | b'O' => crate::screens::scroll_team_list(ctx, key.to_ascii_uppercase()),
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

    fn tick_buy(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let mut widget = Widget::ListMenu(self.list.clone());
        let outcome = widget.tick(ctx.input, ctx.dt_ticks);
        if let Widget::ListMenu(list) = widget {
            self.list = list;
        }
        match outcome {
            // `if (input_key != 'B' && input_key != 0x0d) return;`
            // (`ovr007.cs:117-120`) — only those two keys buy.
            WidgetOutcome::ListSelected { index, key } if key == b'B' || key == b'\r' => {
                self.buy(ctx, self.stock_index(index));
            }
            WidgetOutcome::ListSelected { .. } | WidgetOutcome::ListCancelled => {
                self.stage = Stage::Shop;
            }
            _ => {}
        }
        ScreenTransition::Stay
    }

    fn buy(&mut self, ctx: &mut FlowCtx, index: usize) {
        let buyer =
            (ctx.state.selected_player as usize).min(ctx.roster.members.len().saturating_sub(1));
        let Some(member) = ctx.roster.members.get_mut(buyer) else {
            return;
        };
        let table: ItemDataTable = crate::items::load_table(ctx.data);
        let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(ctx.rules);
        let outcome = shop::buy(
            &self.shop,
            index,
            member,
            &mut ctx.state.pooled_money,
            &table,
            &flavor,
            ctx.rules,
        );
        self.status = Some(match outcome {
            Ok(o) => {
                let line = format!(
                    "Bought {} for {} gp{}.",
                    o.item_name,
                    o.price,
                    if o.paid_from_pool {
                        " from the pool"
                    } else {
                        ""
                    }
                );
                self.bought.push((o.item_name, o.price));
                line
            }
            // `string_print01("Not enough Money.")` (`ovr007.cs:147`).
            Err(BuyError::NotEnoughMoney) => "Not enough Money.".to_string(),
            // `string_print01("Overloaded")` (`:87`).
            Err(BuyError::Overloaded) => "Overloaded".to_string(),
            Err(BuyError::NoSuchItem) => "No such item.".to_string(),
        });
        // The rows carry prices, and a purchase can change nothing about them
        // — but the readied/identified name can move, so rebuild.
        self.list = {
            let cursor = self.list.index();
            let mut list = Self::buy_list(&self.shop, ctx.roster);
            list.seed_cursor(cursor);
            list
        };
    }

    fn tick_leaving(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        // `menu_selected == 1` -> `exitShop = true` (`ovr007.cs:233-236`) and
        // `sub_317AA` returns a 0-BASED index (`ovr008.cs:1196-1206`), so 1 is
        // `No`: Yes goes back into the shop for the money.
        match self.yes_no(ctx) {
            Some(true) => self.stage = Stage::Shop,
            Some(false) => self.stage = Stage::Done,
            None => {}
        }
        ScreenTransition::Stay
    }

    /// `sub_317AA(…, "~Yes ~No", "")` (`ovr007.cs:232`).
    fn yes_no(&self, ctx: &mut FlowCtx) -> Option<bool> {
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

    fn paint(&self, ctx: &mut FlowCtx) {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        match &self.stage {
            Stage::Shop | Stage::LeavingWithMoney => {
                crate::text::draw_string(ctx.fb, ctx.font, "Shop", 1, 1, 0, 0x0F);
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
                if matches!(self.stage, Stage::LeavingWithMoney) {
                    crate::temple_screen::draw_wrapped(
                        ctx,
                        "As you leave the shopkeeper says, \"Excuse me but you have left some \
                         money here.\" Do you want to go back and get your money?",
                    );
                    self.draw_prompt(ctx, "Yes No");
                } else {
                    self.draw_prompt(ctx, Self::shop_bar(&ctx.state.pooled_money));
                }
                // `PartySummary(gbl.SelectedPlayer)` at the bottom of every
                // iteration (`ovr007.cs:268`).
                let member = (ctx.state.selected_player as usize)
                    .min(ctx.roster.members.len().saturating_sub(1));
                let rows: Vec<crate::charsheet::SheetView> = ctx
                    .roster
                    .members
                    .iter()
                    .map(crate::charsheet::sheet_view)
                    .collect();
                crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, Some(member));
            }
            Stage::Buy => {
                // `sl_select_item(…, "Buy", "Items: ")` — the list box owns the
                // whole width, so no roster panel underneath it.
                crate::shell::draw_list_menu(ctx.fb, ctx.font, &self.list);
                self.draw_prompt(ctx, "Buy Exit");
            }
            Stage::Sub(_) | Stage::Done => {}
        }
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 0x17, 1, 0, 0x0E);
        }
    }

    fn draw_prompt(&self, ctx: &mut FlowCtx, text: &str) {
        crate::combat::scene::render::draw_menu_line(ctx.fb, ctx.font, text, None);
    }
}
