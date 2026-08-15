//! ★ **The Items screen** — `PlayerItemsMenu` (`ovr020.cs:432-623`) and
//! `UseMagicItem` (`:980-1086`), roll-credits §12 / G6's interactive half.
//!
//! Derived by reading coab for behavior (D11, never copied). The model half —
//! names, `reclac_player_values`, the verbs' effects — is [`crate::items`];
//! this is the parked state machine over it, opened from the character sheet's
//! `Items` word.
//!
//! ```text
//! viewPlayer  "Items Spells Trade Drop … Exit"
//!   └─ I → PlayerItemsMenu: sl_select_item over the inventory,
//!          "Ready [View] [Use] [Trade] Drop [Halve] Join [Sell Id]"
//!            ├─ R → ready_Item                     (+ reclac, every action)
//!            ├─ U → UseMagicItem  ─┬─ scroll  → spell_menu2(SpellLoc.scroll)
//!            │                     └─ charged → affect_2 & 0x7F
//!            │        └─ sub_5D2E1 ─┬─ SpellTargets.Combat → "is a combat-only
//!            │                      │   item…" / "Use it? "  ← burns a charge
//!            │                      └─ else → NonCombatSpellCast → the effect
//!            ├─ T → CanSellDropTradeItem → selectAPlayer("Trade with Whom?")
//!            ├─ D → CanSellDropTradeItem → "…will be gone forever" / "Drop It?"
//!            ├─ H → halve_items
//!            └─ J → join_items
//! ```
//!
//! ## The two things that surprise people
//!
//! **A wand in camp burns a charge for nothing.** Every wand CotAB ships maps
//! to a `SpellTargets.Combat` row, so out of combat `sub_5D2E1` takes its first
//! branch — and because `gbl.spell_from_item` is set, that branch is not the
//! familiar "can't be cast here… Lose it?" but `"That Item"` /
//! `"is a combat-only item..."` / **`"Use it? "`**, whose Yes answer sets
//! `arg_0` (`ovr023.cs:704-707`). `UseMagicItem`'s tail then reads `arg_0` and
//! spends the charge (`ovr020.cs:1064-1085`) even though `stillCast` was
//! cleared and nothing was ever cast. Reproduced.
//!
//! **A successful camp use also spends.** `PlayerItemsMenu` clears `arg_0`
//! after `UseMagicItem` returns (`ovr020.cs:545-548`) — but that is only to
//! stop the *combat turn* from ending; the consumption already happened inside
//! `UseMagicItem`, before the return.

use crate::camp_cast::{self, NonCombatTargets};
use crate::camp_magic::{paint_confirm, yes_no, ConfirmWidget};
use crate::items::{self, DisposeCheck, ReadyOutcome, ReadyRefusal};
use crate::magic::{self, SpellListing, SpellLoc};
use crate::screens::{ReturnTo, Screen, ScreenTransition};
use crate::shell::FlowCtx;
use crate::spells;
use crate::widgets::{Hotbar, ListItem, ListLayout, ListMenu, Widget, WidgetOutcome};
use gbx_formats::save_orig as rec;

/// `sl_select_item(out …, true, menulist, 0x16, 0x26, 5, 1, …)`
/// (`ovr020.cs:516-517`) — rows 5..=0x16, columns 1..=0x26.
const LIST_LAYOUT: ListLayout = ListLayout {
    start_row: 5,
    start_col: 1,
    end_row: 0x16,
    end_col: 0x26,
};

/// `sl_select_item` opens with `gbl.menuSelectedWord = 1` (`ovr027.cs:548`) —
/// the **second** word of the composed bar, not the first. On the items menu
/// that is `Use` when Use is available and `Trade` when it is not.
const SL_SELECTED_WORD: usize = 1;

/// Which disposal verb a "…was going to scribe from that scroll" confirm is
/// standing in front of (`CanSellDropTradeItem`'s single yes/no serves all of
/// Trade, Drop and Sell — `ovr020.cs:557-607`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Disposal {
    Trade,
    Drop,
    /// ★ Roll-credits slice 9a — `CanSellDropTradeItem` serves Sell too
    /// (`ovr020.cs:600-607`). Appended for postcard.
    Sell,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum Stage {
    /// The `sl_select_item` loop.
    Browse,
    /// `CanSellDropTradeItem`'s `"is it Okay to lose it? "` (`ovr020.cs:362`).
    ScribedScroll { item: usize, then: Disposal },
    /// `press_any_key("Your <name> will be gone forever")` (`:574`).
    DropWarn { item: usize },
    /// `yes_no("Drop It? ")` (`:576`).
    DropConfirm { item: usize },
    /// `selectAPlayer(…, "Trade with Whom?")` (`:897`).
    TradeWhom { item: usize },
    /// `spell_menu2(…, SpellLoc.scroll)` — which spell to read off the scroll
    /// (`:991`).
    ScrollPick { item: usize },
    /// `sub_5D2E1`'s item arm of the can't-cast-here branch: `"Use it? "`
    /// (`ovr023.cs:702-707`).
    CombatOnly { item: usize, spell: u8 },
    /// `NonCombatSpellCast`'s `SpellTargets.PartyMember` selector
    /// (`ovr023.cs:635`).
    ChooseTarget { item: usize, spell: u8 },
    /// ★ Roll-credits slice 9a. `ShopSellItem`'s offer line
    /// (`ovr020.cs:1113`, `press_any_key(..., 14, TextRegion.Normal2)`).
    /// Appended after `ChooseTarget` for postcard's sake.
    SellOffer { item: usize, value: i64 },
    /// `ShopSellItem`'s `yes_no("Is It a Deal? ")` (`:1115`).
    SellConfirm { item: usize, value: i64 },
    /// `IdentifyItem`'s offer line (`:1160`).
    IdOffer { item: usize },
    /// `IdentifyItem`'s `yes_no("Is It a Deal? ")` (`:1162`).
    IdConfirm { item: usize },
    /// `IdentifyItem`'s closing `press_any_key` (`:1186`/`:1192`).
    IdResult { line: String },
}

/// The Items screen.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ItemsScreen {
    member: usize,
    stage: Stage,
    list: ListMenu,
    /// The verb bar — carried as a [`Hotbar`] so the highlight tracks
    /// `gbl.menuSelectedWord` the way every other menu in this shell does; the
    /// *input* goes through [`Self::list`], which is what `sl_select_item`
    /// does too (its own `displayInput` only paints and moves the highlight).
    bar: Hotbar,
    status: Option<String>,
    confirm: Option<ConfirmWidget>,
    /// The scroll picker's rows, when one is open.
    spell_list: Option<ListMenu>,
    spell_listing: Option<SpellListing>,
    /// `gbl.tradeWith` / `gbl.lastSelectetSpellTarget` — the roster cursor the
    /// Trade and target selectors open on.
    target: usize,
    return_to: ReturnTo,
}

impl ItemsScreen {
    /// `viewPlayer`'s `'I'` case (`ovr020.cs:299-301`). Refuses (by never
    /// opening) when the member carries nothing, which is also the condition
    /// the sheet's own bar uses to hide the word.
    pub fn open(ctx: &mut FlowCtx, return_to: ReturnTo) -> ScreenTransition {
        let member =
            (ctx.state.selected_player as usize).min(ctx.roster.members.len().saturating_sub(1));
        let Some(ch) = ctx.roster.members.get(member) else {
            return ScreenTransition::Exit;
        };
        if ch.items.is_empty() {
            return ScreenTransition::Stay;
        }
        // `reclac_player_values` runs at the bottom of every menu iteration
        // (`ovr020.cs:615`); running it on entry as well makes the first paint
        // agree with the sheet the player just left.
        let table = items::load_table(ctx.data);
        let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(ctx.rules);
        items::reclac_player_values(&mut ctx.roster.members[member], &table, &flavor);

        let mut screen = ItemsScreen {
            member,
            stage: Stage::Browse,
            list: ListMenu::boxed(Vec::new(), LIST_LAYOUT),
            bar: Hotbar::new("Ready"),
            status: None,
            confirm: None,
            spell_list: None,
            spell_listing: None,
            target: member,
            return_to,
        };
        screen.rebuild(ctx);
        ScreenTransition::To(Screen::Items(Box::new(screen)))
    }

    /// Rebuild the row list and the verb bar (`redraw_items` /
    /// `player.items.ForEach(ItemDisplayNameBuild)`, `ovr020.cs:496`), keeping
    /// the cursor where `dummy_index` would have left it.
    fn rebuild(&mut self, ctx: &mut FlowCtx) {
        let detect_magic = items::party_has_detect_magic(ctx.roster);
        let Some(ch) = ctx.roster.members.get(self.member) else {
            return;
        };
        let rows: Vec<ListItem> = ch
            .items
            .iter()
            .map(|it| ListItem::Entry(items::display_name(it, detect_magic, true)))
            .collect();
        let cursor = self.list.index();
        self.list = ListMenu::boxed(rows, LIST_LAYOUT);
        self.list.seed_cursor(cursor);
        // ★ `gbl.area_ptr.field_1CA` — the per-area item ban. Threaded from the
        // resident area block the same way the combat menu's Cast word reads it.
        let words = items::items_menu_words(ch, items::area_bans_items(ctx.state), self.in_shop());
        let mut bar = Hotbar::new(words.text());
        bar.seed_selected_word(SL_SELECTED_WORD);
        self.bar = bar;
    }

    /// ★ `gbl.game_state == GameState.Shop` (`ovr020.cs:484`) — the ONE
    /// condition that puts `Sell` and `Id` on this bar. In this shell the
    /// Items leaf only ever reaches a shop through `CityShop`'s own
    /// `viewPlayer()` call (`ovr007.cs:190`), so
    /// [`ReturnTo::Shop`](crate::screens::ReturnTo::Shop) IS that state.
    fn in_shop(&self) -> bool {
        matches!(self.return_to, ReturnTo::Shop)
    }

    fn exit(&self, ctx: &FlowCtx) -> ScreenTransition {
        ScreenTransition::To(Screen::PartyView(crate::screens::PartyView::new(
            ctx,
            self.return_to,
        )))
    }

    /// The full bar `sl_select_item` composes each iteration: the caller's
    /// verbs, then `" Next"`/`" Prev"` as the window allows, then `" Exit"`
    /// (`showExit == true` here, `ovr020.cs:516`).
    fn bar_text(&self) -> String {
        format!("{}{} Exit", self.bar.text, self.list.prompt_words())
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        // `while (… && player.items.Count > 0)` (`ovr020.cs:441-443`): the loop
        // ends the moment the last item leaves.
        let empty = ctx
            .roster
            .members
            .get(self.member)
            .is_none_or(|c| c.items.is_empty());
        if empty {
            return self.exit(ctx);
        }
        self.paint(ctx);
        match self.stage.clone() {
            Stage::Browse => self.tick_browse(ctx),
            Stage::ScribedScroll { item, then } => self.tick_scribed(ctx, item, then),
            Stage::DropWarn { item } => {
                if ctx.input.read_key().is_some() {
                    self.stage = Stage::DropConfirm { item };
                    self.confirm = Some(yes_no("Drop It? "));
                }
                ScreenTransition::Stay
            }
            Stage::DropConfirm { item } => self.tick_drop(ctx, item),
            Stage::TradeWhom { item } => self.tick_trade(ctx, item),
            Stage::ScrollPick { item } => self.tick_scroll_pick(ctx, item),
            Stage::CombatOnly { item, spell } => self.tick_combat_only(ctx, item, spell),
            Stage::ChooseTarget { item, spell } => self.tick_choose_target(ctx, item, spell),
            // ★ Roll-credits slice 9a: the shop-only pair.
            Stage::SellOffer { item, value } => {
                // `press_any_key(offer, true, 14, TextRegion.Normal2)` (`:1113`).
                if ctx.input.read_key().is_some() {
                    self.stage = Stage::SellConfirm { item, value };
                    self.confirm = Some(yes_no("Is It a Deal? "));
                }
                ScreenTransition::Stay
            }
            Stage::SellConfirm { item, value } => self.tick_sell(ctx, item, value),
            Stage::IdOffer { item } => {
                if ctx.input.read_key().is_some() {
                    self.stage = Stage::IdConfirm { item };
                    self.confirm = Some(yes_no("Is It a Deal? "));
                }
                ScreenTransition::Stay
            }
            Stage::IdConfirm { item } => self.tick_identify(ctx, item),
            Stage::IdResult { .. } => {
                if ctx.input.read_key().is_some() {
                    self.stage = Stage::Browse;
                    self.rebuild(ctx);
                }
                ScreenTransition::Stay
            }
        }
    }

    // --- Sell / Id (shop only) -------------------------------------------

    /// `ShopSellItem`'s offer for one row ([`crate::shop::sell_offer`]).
    fn sell_value(&self, ctx: &FlowCtx, item: usize) -> i64 {
        ctx.roster
            .members
            .get(self.member)
            .and_then(|c| c.items.get(item))
            .map(|r| crate::shop::sell_offer(r))
            .unwrap_or(0)
    }

    fn begin_sell(&mut self, ctx: &mut FlowCtx, item: usize) {
        let value = self.sell_value(ctx, item);
        self.stage = Stage::SellOffer { item, value };
    }

    /// `ShopSellItem`'s Yes arm (`ovr020.cs:1117-1147`): `"Sold!"`, the item is
    /// lost, and the money arrives as [`crate::shop::sell_payout`]'s
    /// platinum-and-gold change — with the overflow pooled when it would
    /// overload the seller.
    fn tick_sell(&mut self, ctx: &mut FlowCtx, item: usize, value: i64) -> ScreenTransition {
        let Some(confirm) = &mut self.confirm else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        match confirm.tick(ctx) {
            None => return ScreenTransition::Stay,
            Some(false) => {
                self.confirm = None;
                self.stage = Stage::Browse;
                return ScreenTransition::Stay;
            }
            Some(true) => {}
        }
        self.confirm = None;
        let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(ctx.rules);
        let overloaded = {
            let Some(ch) = ctx.roster.members.get_mut(self.member) else {
                self.stage = Stage::Browse;
                return ScreenTransition::Stay;
            };
            items::lose_item(ch, item);
            crate::shop::sell_payout(ch, &mut ctx.state.pooled_money, value, &flavor)
        };
        self.status = Some(if overloaded {
            "Overloaded. Money will be put in pool.".into()
        } else {
            "Sold!".into()
        });
        self.stage = Stage::Browse;
        self.reclac(ctx);
        self.rebuild(ctx);
        ScreenTransition::Stay
    }

    /// `IdentifyItem` (`ovr020.cs:1153-1200`): 200 gold from the purse or the
    /// pool, then `hidden_names_flag = 0` — which is what makes the generated
    /// name grow its `+N`/`of X` words.
    fn tick_identify(&mut self, ctx: &mut FlowCtx, item: usize) -> ScreenTransition {
        let Some(confirm) = &mut self.confirm else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        match confirm.tick(ctx) {
            None => return ScreenTransition::Stay,
            Some(false) => {
                self.confirm = None;
                self.stage = Stage::Browse;
                return ScreenTransition::Stay;
            }
            Some(true) => {}
        }
        self.confirm = None;
        let cost = crate::shop::IDENTIFY_COST;
        let Some(ch) = ctx.roster.members.get_mut(self.member) else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        // Purse first, then the pool (`ovr020.cs:1164-1181`).
        if crate::money::gold_worth(&ch.money, ctx.rules) >= cost {
            crate::money::subtract_gold_worth(&mut ch.money, cost, ctx.rules);
        } else if ctx.state.pooled_money.gold_worth() >= cost {
            ctx.state.pooled_money.subtract_gold_worth(cost);
        } else {
            self.status = Some("Not Enough Money".into());
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        }
        let Some(record) = ch.items.get_mut(item) else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        let line = if rec::item_hidden_names_flag(record) == 0 {
            format!(
                "I can't tell anything new about your {}",
                items::display_name(record, false, false)
            )
        } else {
            rec::set_item_hidden_names_flag(record, 0);
            format!(
                "It looks like some sort of {}",
                items::display_name(record, false, false)
            )
        };
        self.stage = Stage::IdResult { line };
        ScreenTransition::Stay
    }

    fn tick_browse(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let mut widget = Widget::ListMenu(self.list.clone());
        let outcome = widget.tick(ctx.input, ctx.dt_ticks);
        if let Widget::ListMenu(list) = widget {
            self.list = list;
        }
        match outcome {
            // `sl_select_item`'s `default:` arm — the highlighted row plus the
            // key that resolved it (`ovr027.cs:655-658`).
            WidgetOutcome::ListSelected { index, key } => {
                // `displayInput`'s own letter scan moves the highlight before
                // the loop stops (`ovr027.cs:279-292`).
                self.bar.select_word_starting_with(key.to_ascii_uppercase());
                self.dispatch(ctx, index, key.to_ascii_uppercase())
            }
            // Esc / `'\0'` / `'E'` all leave (`ovr027.cs:650-656`).
            WidgetOutcome::ListCancelled => self.exit(ctx),
            _ => ScreenTransition::Stay,
        }
    }

    /// `PlayerItemsMenu`'s switch (`ovr020.cs:523-612`), followed by its
    /// unconditional `reclac_player_values` (`:615`).
    fn dispatch(&mut self, ctx: &mut FlowCtx, item: usize, key: u8) -> ScreenTransition {
        let words = {
            let ch = &ctx.roster.members[self.member];
            items::items_menu_words(ch, items::area_bans_items(ctx.state), self.in_shop())
        };
        self.status = None;
        match key {
            b'R' => self.ready(ctx, item),
            b'U' if words.use_item => return self.use_item(ctx, item),
            b'T' if words.trade => match self.dispose_check(ctx, item) {
                DisposeCheck::MustBeUnreadied => {
                    self.status = Some("Must be unreadied".into());
                }
                DisposeCheck::ConfirmScribedScroll => {
                    self.stage = Stage::ScribedScroll {
                        item,
                        then: Disposal::Trade,
                    };
                    self.confirm = Some(yes_no("is it Okay to lose it? "));
                    return ScreenTransition::Stay;
                }
                DisposeCheck::Allowed => {
                    self.stage = Stage::TradeWhom { item };
                    return ScreenTransition::Stay;
                }
            },
            b'D' => match self.dispose_check(ctx, item) {
                DisposeCheck::MustBeUnreadied => {
                    self.status = Some("Must be unreadied".into());
                }
                DisposeCheck::ConfirmScribedScroll => {
                    self.stage = Stage::ScribedScroll {
                        item,
                        then: Disposal::Drop,
                    };
                    self.confirm = Some(yes_no("is it Okay to lose it? "));
                    return ScreenTransition::Stay;
                }
                DisposeCheck::Allowed => {
                    self.stage = Stage::DropWarn { item };
                    return ScreenTransition::Stay;
                }
            },
            b'H' if words.halve => {
                if !items::halve_items(&mut ctx.roster.members[self.member], item) {
                    self.status = Some("Can't halve that".into());
                }
            }
            b'J' => {
                items::join_items(&mut ctx.roster.members[self.member], item);
            }
            // ★ Shop-only, and the same `CanSellDropTradeItem` gate Trade and
            // Drop use (`ovr020.cs:600-607`).
            b'S' if words.sell => match self.dispose_check(ctx, item) {
                DisposeCheck::MustBeUnreadied => {
                    self.status = Some("Must be unreadied".into());
                }
                DisposeCheck::ConfirmScribedScroll => {
                    self.stage = Stage::ScribedScroll {
                        item,
                        then: Disposal::Sell,
                    };
                }
                DisposeCheck::Allowed => self.begin_sell(ctx, item),
            },
            b'I' if words.id => {
                self.stage = Stage::IdOffer { item };
            }
            _ => {}
        }
        self.reclac(ctx);
        self.rebuild(ctx);
        ScreenTransition::Stay
    }

    fn dispose_check(&self, ctx: &FlowCtx, item: usize) -> DisposeCheck {
        let table = items::load_table(ctx.data);
        let ch = &ctx.roster.members[self.member];
        match ch.items.get(item) {
            Some(record) => items::dispose_check(&table, record),
            None => DisposeCheck::MustBeUnreadied,
        }
    }

    fn reclac(&self, ctx: &mut FlowCtx) {
        let table = items::load_table(ctx.data);
        let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(ctx.rules);
        if let Some(ch) = ctx.roster.members.get_mut(self.member) {
            items::reclac_player_values(ch, &table, &flavor);
        }
    }

    fn ready(&mut self, ctx: &mut FlowCtx, item: usize) {
        let table = items::load_table(ctx.data);
        let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(ctx.rules);
        let ch = &mut ctx.roster.members[self.member];
        match items::ready_item(ch, item, &table, &flavor) {
            Ok(ReadyOutcome::Readied) | Ok(ReadyOutcome::Unreadied) => {}
            Ok(ReadyOutcome::RiderDeferred {
                readied,
                masked_affect,
            }) => {
                // The named residual (roll-credits §12.3): the ready/unready and
                // the reclac happened; `calc_items_effects`' stat/spell-slot
                // rider did not. Loud, not silent.
                self.status = Some(format!(
                    "{} — magic-item effect {masked_affect} not wired",
                    if readied { "Readied" } else { "Unreadied" }
                ));
            }
            Err(ReadyRefusal::Cursed) => self.status = Some("It's Cursed".into()),
            Err(ReadyRefusal::WrongClass) => self.status = Some("Wrong Class".into()),
            Err(ReadyRefusal::HandsFull) => self.status = Some("Your hands are full!".into()),
            Err(ReadyRefusal::AlreadyUsing(name)) => {
                self.status = Some(format!("already using {name}"))
            }
        }
    }

    // --- Use -------------------------------------------------------------

    /// `PlayerItemsMenu`'s `'U'` case (`ovr020.cs:535-555`): a **readied**
    /// scroll or charged item only. A readied mundane item says nothing at all
    /// — the original's `else if` simply does not match, and no message is
    /// printed. Faithful.
    fn use_item(&mut self, ctx: &mut FlowCtx, item: usize) -> ScreenTransition {
        let table = items::load_table(ctx.data);
        let ch = &mut ctx.roster.members[self.member];
        let Some(record) = ch.items.get(item) else {
            return ScreenTransition::Stay;
        };
        if !rec::item_readied(record) {
            self.status = Some("Must be Readied".into());
            return ScreenTransition::Stay;
        }
        if items::is_scroll(&table, record) {
            // `gbl.currentScroll = item` then `spell_menu2(SpellLoc.scroll)`
            // (`ovr020.cs:987-991`). The read-magic sweep runs first, exactly
            // as `scroll_5C912` would on the way into the list.
            items::apply_read_magic(ch, &table);
            let scrolls = crate::camp_magic::camp_scrolls(ctx);
            let ch = &ctx.roster.members[self.member];
            let listing = magic::build_spell_list(SpellLoc::Scroll, ch, &scrolls, Some(item));
            if listing.is_empty() {
                // A still-hidden scroll lists nothing, and `spell_menu2`
                // returns 0 without drawing (`ovr020.cs:1442-1445`).
                self.status = Some("You can't read it".into());
                return ScreenTransition::Stay;
            }
            self.spell_list = Some(ListMenu::boxed(
                listing.items.clone(),
                magic::spell_list_layout(magic::SpellSource::Cast),
            ));
            self.spell_listing = Some(listing);
            self.stage = Stage::ScrollPick { item };
            return ScreenTransition::Stay;
        }
        // `affect_2 > 0 && affect_3 < 0x80` — a charged item (`ovr020.cs:542`,
        // `:993-997`); the spell is `affect_2 & 0x7F`.
        if rec::item_affect(record, 2) > 0 && rec::item_affect(record, 3) < 0x80 {
            let spell = rec::item_affect(record, 2) & 0x7F;
            return self.begin_cast(ctx, item, spell, false);
        }
        ScreenTransition::Stay
    }

    fn tick_scroll_pick(&mut self, ctx: &mut FlowCtx, item: usize) -> ScreenTransition {
        let Some(list) = &mut self.spell_list else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        let mut widget = Widget::ListMenu(list.clone());
        let outcome = widget.tick(ctx.input, ctx.dt_ticks);
        if let Widget::ListMenu(l) = widget {
            self.spell_list = Some(l);
        }
        match outcome {
            WidgetOutcome::ListSelected { index, .. } => {
                let spell = self
                    .spell_listing
                    .as_ref()
                    .and_then(|l| l.id_at_row(index))
                    .unwrap_or(0);
                self.spell_list = None;
                self.spell_listing = None;
                if spell == 0 {
                    self.stage = Stage::Browse;
                    return ScreenTransition::Stay;
                }
                self.begin_cast(ctx, item, spell, true)
            }
            WidgetOutcome::ListCancelled => {
                self.spell_list = None;
                self.spell_listing = None;
                self.stage = Stage::Browse;
                ScreenTransition::Stay
            }
            _ => ScreenTransition::Stay,
        }
    }

    /// `UseMagicItem`'s body from the spell id onward (`ovr020.cs:999-1062`),
    /// out of combat.
    fn begin_cast(
        &mut self,
        ctx: &mut FlowCtx,
        item: usize,
        spell: u8,
        from_scroll: bool,
    ) -> ScreenTransition {
        // The scroll literacy gate (`ovr020.cs:1032-1047`): a cleric or
        // magic-user reads it; a thief above level 9 reads it 75% of the time;
        // anyone else gets "oops!" and the scroll is NOT consumed.
        if from_scroll && !self.can_read_scroll(ctx) {
            self.status = Some(format!(
                "{} oops!",
                ctx.roster.members[self.member].name.clone()
            ));
            self.stage = Stage::Browse;
            self.rebuild(ctx);
            return ScreenTransition::Stay;
        }

        let Some(entry) = spells::spell_entry(spell) else {
            // §9.1's lazy-transcription rule: a pruned row is a loud refusal,
            // and nothing is consumed.
            self.status = Some(format!(
                "{} is not implemented yet",
                magic::spell_name(spell)
            ));
            self.stage = Stage::Browse;
            self.rebuild(ctx);
            return ScreenTransition::Stay;
        };

        if camp_cast::cant_be_cast_here(&entry) {
            self.stage = Stage::CombatOnly { item, spell };
            self.confirm = Some(yes_no("Use it? "));
            self.status = Some("That Item is a combat-only item...".into());
            return ScreenTransition::Stay;
        }

        match NonCombatTargets::of(&entry) {
            NonCombatTargets::ChooseOne => {
                self.stage = Stage::ChooseTarget { item, spell };
                self.target = self.member;
                ScreenTransition::Stay
            }
            NonCombatTargets::Caster => {
                let targets = vec![self.member];
                self.resolve(ctx, item, spell, &targets, from_scroll)
            }
            NonCombatTargets::WholeParty => {
                let targets = camp_cast::whole_party_targets(self.member, ctx.roster);
                self.resolve(ctx, item, spell, &targets, from_scroll)
            }
            NonCombatTargets::None => {
                self.stage = Stage::Browse;
                ScreenTransition::Stay
            }
        }
    }

    /// `ovr020.cs:1034-1043`'s three-way literacy test. The thief roll is
    /// **draw-bearing** — a d100 out of camp, on the same `crate::rest::roll_dice`
    /// path every other out-of-combat roll takes.
    fn can_read_scroll(&self, ctx: &mut FlowCtx) -> bool {
        let ch = &ctx.roster.members[self.member];
        if ch.skill_level(crate::party::SKILL_MAGIC_USER) > 0
            || ch.skill_level(crate::party::SKILL_CLERIC) > 0
        {
            return true;
        }
        const SKILL_THIEF: usize = 6;
        if ch.skill_level(SKILL_THIEF) > 9 {
            return i32::from(crate::rest::roll_dice(ctx.rng, 100, 1)) <= 75;
        }
        false
    }

    fn tick_combat_only(&mut self, ctx: &mut FlowCtx, item: usize, _spell: u8) -> ScreenTransition {
        let Some(confirm) = &mut self.confirm else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        match confirm.tick(ctx) {
            None => ScreenTransition::Stay,
            Some(answer) => {
                self.confirm = None;
                self.status = None;
                if answer {
                    // ★ `arg_0 = true` (`ovr023.cs:706`) with `stillCast =
                    // false`: the charge is spent, and nothing is cast.
                    let ch = &mut ctx.roster.members[self.member];
                    let name = items::display_name(&ch.items[item], false, false);
                    items::consume_charge(ch, item);
                    self.status = Some(format!("{name}: a charge is spent"));
                }
                self.stage = Stage::Browse;
                self.reclac(ctx);
                self.rebuild(ctx);
                ScreenTransition::Stay
            }
        }
    }

    fn tick_choose_target(
        &mut self,
        ctx: &mut FlowCtx,
        item: usize,
        spell: u8,
    ) -> ScreenTransition {
        let count = ctx.roster.members.len();
        match ctx.input.read_key().and_then(camp_cast::select_key) {
            Some(camp_cast::SelectKey::Next) => {
                self.target = (self.target + 1) % count;
                ScreenTransition::Stay
            }
            Some(camp_cast::SelectKey::Prev) => {
                self.target = (self.target + count - 1) % count;
                ScreenTransition::Stay
            }
            Some(camp_cast::SelectKey::Select) => {
                let targets = vec![self.target];
                let from_scroll = self.spell_listing.is_some() || self.was_scroll(ctx, item);
                self.resolve(ctx, item, spell, &targets, from_scroll)
            }
            Some(camp_cast::SelectKey::Exit) => {
                // `castSpell = false` — nothing cast, nothing consumed.
                self.stage = Stage::Browse;
                self.rebuild(ctx);
                ScreenTransition::Stay
            }
            None => ScreenTransition::Stay,
        }
    }

    fn was_scroll(&self, ctx: &FlowCtx, item: usize) -> bool {
        let table = items::load_table(ctx.data);
        ctx.roster.members[self.member]
            .items
            .get(item)
            .is_some_and(|r| items::is_scroll(&table, r))
    }

    /// `sub_5D2E1`'s success tail plus `UseMagicItem`'s consumption
    /// (`ovr023.cs:769-782`, `ovr020.cs:1064-1085`).
    fn resolve(
        &mut self,
        ctx: &mut FlowCtx,
        item: usize,
        spell: u8,
        targets: &[usize],
        from_scroll: bool,
    ) -> ScreenTransition {
        // `remove_invisibility(caster)` runs on every successful cast; the
        // memorized-slot `ClearSpell` does **not** (`gbl.spell_from_item`).
        crate::affects::remove_all(
            &mut ctx.roster.members[self.member],
            camp_cast::AFF_INVISIBILITY,
        );
        let reports = camp_cast::cast(ctx.roster, ctx.rng, ctx.rules, self.member, spell, targets);
        let item_name = ctx.roster.members[self.member]
            .items
            .get(item)
            .map(|r| items::display_name(r, false, false))
            .unwrap_or_default();
        self.status = Some(match reports.first() {
            Some(r) => format!(
                "{} {}",
                ctx.roster
                    .members
                    .get(r.member)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
                r.text
            ),
            None => format!(
                "{} uses an item: {item_name}",
                ctx.roster.members[self.member].name
            ),
        });
        if from_scroll {
            items::remove_spell_from_scroll(&mut ctx.roster.members[self.member], item, spell);
        } else {
            items::consume_charge(&mut ctx.roster.members[self.member], item);
        }
        self.stage = Stage::Browse;
        self.reclac(ctx);
        self.rebuild(ctx);
        ScreenTransition::Stay
    }

    // --- Trade / Drop ----------------------------------------------------

    fn tick_scribed(&mut self, ctx: &mut FlowCtx, item: usize, then: Disposal) -> ScreenTransition {
        let Some(confirm) = &mut self.confirm else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        match confirm.tick(ctx) {
            None => ScreenTransition::Stay,
            Some(true) => {
                self.confirm = None;
                self.stage = match then {
                    Disposal::Trade => Stage::TradeWhom { item },
                    Disposal::Drop => Stage::DropWarn { item },
                    Disposal::Sell => {
                        let value = self.sell_value(ctx, item);
                        Stage::SellOffer { item, value }
                    }
                };
                ScreenTransition::Stay
            }
            Some(false) => {
                self.confirm = None;
                self.stage = Stage::Browse;
                ScreenTransition::Stay
            }
        }
    }

    fn tick_drop(&mut self, ctx: &mut FlowCtx, item: usize) -> ScreenTransition {
        let Some(confirm) = &mut self.confirm else {
            self.stage = Stage::Browse;
            return ScreenTransition::Stay;
        };
        match confirm.tick(ctx) {
            None => ScreenTransition::Stay,
            Some(answer) => {
                self.confirm = None;
                if answer {
                    items::lose_item(&mut ctx.roster.members[self.member], item);
                }
                self.stage = Stage::Browse;
                self.reclac(ctx);
                self.rebuild(ctx);
                ScreenTransition::Stay
            }
        }
    }

    /// `trade_item` (`ovr020.cs:892-913`): pick a recipient, check `canCarry`,
    /// then move the record. `canCarry`'s inverted sense is preserved in
    /// [`items::cannot_carry`].
    fn tick_trade(&mut self, ctx: &mut FlowCtx, item: usize) -> ScreenTransition {
        let count = ctx.roster.members.len();
        match ctx.input.read_key().and_then(camp_cast::select_key) {
            Some(camp_cast::SelectKey::Next) => {
                self.target = (self.target + 1) % count;
                ScreenTransition::Stay
            }
            Some(camp_cast::SelectKey::Prev) => {
                self.target = (self.target + count - 1) % count;
                ScreenTransition::Stay
            }
            Some(camp_cast::SelectKey::Select) => {
                self.trade(ctx, item);
                self.stage = Stage::Browse;
                self.reclac(ctx);
                self.rebuild(ctx);
                ScreenTransition::Stay
            }
            Some(camp_cast::SelectKey::Exit) => {
                self.stage = Stage::Browse;
                ScreenTransition::Stay
            }
            None => ScreenTransition::Stay,
        }
    }

    fn trade(&mut self, ctx: &mut FlowCtx, item: usize) {
        if self.target == self.member {
            return;
        }
        let table = items::load_table(ctx.data);
        let flavor = gbx_rules::adnd1::flavor_impl::Adnd1::new(ctx.rules);
        let Some(record) = ctx.roster.members[self.member].items.get(item).cloned() else {
            return;
        };
        let recipient = &mut ctx.roster.members[self.target];
        if items::cannot_carry(recipient, &record, &table, &flavor) {
            self.status = Some("Overloaded".into());
            return;
        }
        recipient.items.push(record);
        items::reclac_player_values(recipient, &table, &flavor);
        items::lose_item(&mut ctx.roster.members[self.member], item);
        let name = ctx.roster.members[self.target].name.clone();
        self.status = Some(format!("Traded to {name}"));
    }

    // --- paint -----------------------------------------------------------

    /// `PlayerItemsMenu`'s own header (`ovr020.cs:499-511`): `draw8x8_07`, the
    /// player's name at row 1 col 1, `"Items"` beside it, and the `"Ready
    /// Item"` column heading on row 3 — the label over the Yes/No column
    /// `ItemDisplayNameBuild(displayReadied = true)` writes into every row.
    fn paint(&self, ctx: &mut FlowCtx) {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        let name = ctx
            .roster
            .members
            .get(self.member)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        crate::text::draw_string(ctx.fb, ctx.font, &name, 1, 1, 0, 0x0B);
        crate::text::draw_string(ctx.fb, ctx.font, "Items", 1, name.len() + 4, 0, 10);
        crate::text::draw_string(ctx.fb, ctx.font, "Ready Item", 3, 1, 0, 0x0F);

        match &self.stage {
            Stage::ScrollPick { .. } => {
                if let Some(list) = &self.spell_list {
                    crate::shell::draw_list_menu(ctx.fb, ctx.font, list);
                }
                crate::text::draw_string(ctx.fb, ctx.font, "Choose Spell: ", 0x17, 1, 0, 13);
            }
            Stage::TradeWhom { .. } | Stage::ChooseTarget { .. } => {
                let rows: Vec<_> = ctx
                    .roster
                    .members
                    .iter()
                    .map(crate::charsheet::sheet_view)
                    .collect();
                crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, Some(self.target));
                let prompt = if matches!(self.stage, Stage::TradeWhom { .. }) {
                    "Trade with Whom?  Select Exit"
                } else {
                    "Use on whom  Select Exit"
                };
                crate::text::draw_string(ctx.fb, ctx.font, prompt, 0x17, 1, 0, 13);
            }
            _ => {
                crate::shell::draw_list_menu(ctx.fb, ctx.font, &self.list);
                let bar = self.bar_text();
                crate::combat::scene::render::draw_menu_line(
                    ctx.fb,
                    ctx.font,
                    &bar,
                    self.bar.selected_span(),
                );
            }
        }

        // ★ Roll-credits slice 9a: the two shop-only offer lines, both
        // `press_any_key(..., 14, TextRegion.Normal2)` (`ovr020.cs:1113`,
        // `:1160`), plus `IdentifyItem`'s verdict (`:1186`/`:1192`).
        let offer = match &self.stage {
            Stage::SellOffer { item, value } | Stage::SellConfirm { item, value } => ctx
                .roster
                .members
                .get(self.member)
                .and_then(|c| c.items.get(*item))
                .map(|r| {
                    format!(
                        "I'll give you {value} gold pieces for your {}",
                        items::display_name(r, false, false)
                    )
                }),
            Stage::IdOffer { item } | Stage::IdConfirm { item } => ctx
                .roster
                .members
                .get(self.member)
                .and_then(|c| c.items.get(*item))
                .map(|r| {
                    format!(
                        "For {} gold pieces I'll identify your {}",
                        crate::shop::IDENTIFY_COST,
                        items::display_name(r, false, false)
                    )
                }),
            Stage::IdResult { line } => Some(line.clone()),
            _ => None,
        };
        if let Some(line) = offer {
            crate::text::draw_string(ctx.fb, ctx.font, &line, 0x15, 1, 0, 14);
            if matches!(self.stage, Stage::SellOffer { .. } | Stage::IdOffer { .. })
                || matches!(self.stage, Stage::IdResult { .. })
            {
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    "press <enter>/<return> to continue",
                    0x18,
                    0,
                    0,
                    13,
                );
            }
        }
        if let Stage::DropWarn { item } = &self.stage {
            let ch = &ctx.roster.members[self.member];
            if let Some(record) = ch.items.get(*item) {
                let line = format!(
                    "Your {} will be gone forever",
                    items::display_name(record, false, false)
                );
                crate::text::draw_string(ctx.fb, ctx.font, &line, 0x15, 1, 0, 14);
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    "press <enter>/<return> to continue",
                    0x18,
                    0,
                    0,
                    13,
                );
            }
        }
        // `string_print01` puts its line on the prompt row itself
        // (`ovr025.cs:775-784`: `ClearPromptAreaNoUpdate`, draw at `0x18`,
        // `GameDelay`, clear again) — a *transient* the bar reappears behind.
        // This shell has no blocking delay to hang that on, so the line goes on
        // row `0x17`, the free row directly above the bar, and the bar stays
        // legible. Flagged rather than absorbed: it is a placement divergence,
        // not a behavioural one.
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 0x17, 1, 0, 10);
        }
        if let Some(confirm) = &self.confirm {
            let confirm = confirm.clone();
            paint_confirm(ctx, &confirm);
        }
    }
}

#[cfg(test)]
mod tests;
