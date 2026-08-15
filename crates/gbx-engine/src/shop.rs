//! Shop purchases (`ovr007.cs` `CityShop`/`shop_buy`/`ItemsValue`, M3 step 6
//! deliverable 5). The transaction logic; the interactive screen is
//! `crate::screens::Shop`.
//!
//! **Inventory/price trace (the deliverable's required writeup).** A shop's
//! stock is NOT a shop-id table or an item-type range: it is `gbl.items_pointer`
//! (`Gbl.cs:526`), a plain item list the **ECL script fills before entry** via
//! the `TREASURE` opcode 0x27 (`ovr003.cs:1068-1198`) — either a fixed authored
//! block from `ITEM{game_area}.dax` (`block_id < 0x80`) or a d100-loot random
//! roll (`0x80 <= block_id`). The shop is entered flag-based: the ECL sets area
//! var `0x6D8` (`EnterShop`, `Area2.cs:249`) then runs `COMBAT` 0x24, whose
//! handler dispatches to `CityShop()` when that flag is set (`ovr003.cs:978-982`).
//! Price lives on the *item instance*: `Item._value` (`Item.cs:35`, on-disk
//! `0x3A`), floored to 1, scaled by the shop's price class `area2.field_6DA`
//! (`ItemsValue`, `ovr007.cs:44-82`) — a bit-shift markup/discount. Payment is
//! `MoneySet.SubtractGoldWorth` from the buyer (or pooled money), the item is
//! cloned into `player.items`, and `reclac_player_values` re-sums encumbrance.
//!
//! **M3 scope.** The `TREASURE` opcode (item-data-file decode) and the ECL
//! shop-entry flow are M6; here a [`Shop`] is populated by the host (the demo
//! stocks Tilverton's arms shop). The transaction — price arithmetic against
//! the 7-coin money model, inventory add, encumbrance bump — is faithful and
//! pack-backed.

use crate::money;
use crate::party::Character;
use gbx_rules::pack::RuleSet;

/// One item for sale — the raw `.swg`-format item record (cloned into the
/// buyer's inventory on purchase, exactly as coab's `ShallowClone`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShopItem {
    pub record: Vec<u8>,
}

impl ShopItem {
    /// Wraps a raw item record.
    pub fn from_record(record: Vec<u8>) -> Self {
        ShopItem { record }
    }

    /// Builds a synthetic item record with the fields a shop reads — for
    /// tests/demos (self-authored, D10-clean; no game bytes).
    pub fn synthetic(name: &str, value: i16, weight: i16) -> Self {
        let mut record = vec![0u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
        let name_bytes = name.as_bytes();
        let n = name_bytes.len().min(0x2A);
        record[0..n].copy_from_slice(&name_bytes[..n]);
        record[0x37..0x39].copy_from_slice(&weight.to_le_bytes());
        record[0x3A..0x3C].copy_from_slice(&value.to_le_bytes());
        ShopItem { record }
    }

    pub fn name(&self) -> String {
        gbx_formats::save_orig::item_name(&self.record)
    }

    pub fn base_value(&self) -> i16 {
        gbx_formats::save_orig::item_value(&self.record)
    }

    pub fn weight(&self) -> i16 {
        gbx_formats::save_orig::item_weight(&self.record)
    }
}

/// A shop: its stock (`gbl.items_pointer`) and price class (`area2.field_6DA`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Shop {
    pub items: Vec<ShopItem>,
    /// `area2_ptr.field_6DA` (`Area2.cs:81`): the price-class bitflag scaling
    /// `ItemsValue`. `0` (default) = list price unchanged.
    pub price_class: u8,
}

impl Shop {
    pub fn new(items: Vec<ShopItem>, price_class: u8) -> Self {
        Shop { items, price_class }
    }

    /// What the player pays for `index` (`ItemsValue`, `ovr007.cs:44-82`).
    pub fn price(&self, index: usize) -> Option<i64> {
        self.items
            .get(index)
            .map(|it| items_value(it.base_value(), self.price_class))
    }
}

/// `ItemsValue` (`ovr007.cs:44-82`): the item's `_value` floored to 1
/// (`ShopChooseItem`, `ovr007.cs:13-16`) then bit-shifted by the shop's price
/// class — discounts `0x01..0x08` (>>4..>>1), markups `0x20..0x80` (<<1..<<3),
/// anything else the list price unchanged.
pub fn items_value(base_value: i16, price_class: u8) -> i64 {
    let v = (base_value as i64).max(1);
    match price_class {
        0x01 => v >> 4,
        0x02 => v >> 3,
        0x04 => v >> 2,
        0x08 => v >> 1,
        0x20 => v << 1,
        0x40 => v << 2,
        0x80 => v << 3,
        _ => v,
    }
}

/// Why a purchase failed (`shop_buy`, `ovr007.cs:106-149`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyError {
    /// No such item in the shop.
    NoSuchItem,
    /// Neither the buyer's purse nor the pool covers the price
    /// (`ovr007.cs:147` `"Not enough Money."`).
    NotEnoughMoney,
    /// ★ `PlayerAddItem`'s `canCarry` refusal (`ovr007.cs:85-89`,
    /// `"Overloaded"`) — sixteen items, or over
    /// `max_encumberance + 1500`. **The money is not taken**: `shop_buy` only
    /// pays when `PlayerAddItem` reported no overload (`:126-137`).
    Overloaded,
}

/// A successful purchase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyOutcome {
    pub item_name: String,
    pub price: i64,
    /// Whether the pool paid rather than the buyer's own purse
    /// (`ovr007.cs:139-146`).
    pub paid_from_pool: bool,
}

/// ★ `shop_buy`'s transaction (`ovr007.cs:106-149`), in the original's own
/// order: price, then **the buyer's purse**, then — only if the purse cannot
/// cover it — **the pool**, then the refusal.
///
/// `PlayerAddItem` (`:85-103`) runs `canCarry` FIRST and adds a
/// `ShallowClone`; the money is only taken when it reported no overload, so an
/// overloaded buyer keeps their coins and gets nothing.
///
/// ★ **The shop never runs out.** Nothing removes the item from
/// `gbl.items_pointer` — the clone is what the player walks away with, and the
/// same sword can be bought all day. Transcribed, not "fixed".
pub fn buy(
    shop: &Shop,
    index: usize,
    buyer: &mut Character,
    pool: &mut crate::money::MoneySet,
    table: &gbx_formats::items::ItemDataTable,
    flavor: &dyn gbx_rules::flavor::Flavor,
    rules: &RuleSet,
) -> Result<BuyOutcome, BuyError> {
    let item = shop.items.get(index).ok_or(BuyError::NoSuchItem)?;
    let price = items_value(item.base_value(), shop.price_class);

    let purse_covers = money::gold_worth(&buyer.money, rules) >= price;
    let pool_covers = pool.gold_worth() >= price;
    if !purse_covers && !pool_covers {
        return Err(BuyError::NotEnoughMoney);
    }

    // `PlayerAddItem` (`ovr007.cs:85-103`): `canCarry` then the clone, then
    // `reclac_player_values` — which is what really re-sums the weight.
    if crate::items::cannot_carry(buyer, &item.record, table, flavor) {
        return Err(BuyError::Overloaded);
    }
    buyer.items.push(item.record.clone());
    crate::items::reclac_player_values(buyer, table, flavor);

    if purse_covers {
        money::subtract_gold_worth(&mut buyer.money, price, rules);
    } else {
        pool.subtract_gold_worth(price);
    }

    Ok(BuyOutcome {
        item_name: crate::items::display_name(&item.record, false, false),
        price,
        paid_from_pool: !purse_covers,
    })
}

/// ★ `ShopSellItem`'s offer (`ovr020.cs:1089-1110`): half the item's `_value`
/// (integer division, and `0` for a worthless item — the shop's price class
/// does **not** apply), then the stack adjustment.
///
/// A stack of more than one is worth `count * half / 20` — a *twentieth* —
/// unless it is arrows or quarrels, which are worth `count * half`.
pub fn sell_offer(record: &[u8]) -> i64 {
    let base = gbx_formats::save_orig::item_value(record);
    let mut value = if base > 0 { (base / 2) as i64 } else { 0 };
    let count = gbx_formats::save_orig::item_count(record) as i64;
    if count > 1 {
        let ty = gbx_formats::save_orig::item_type(record);
        if ty == crate::items::TYPE_ARROW || ty == crate::items::TYPE_QUARREL {
            value *= count;
        } else {
            value = (count * value) / 20;
        }
    }
    value
}

/// `IdentifyItem`'s fee (`ovr020.cs:1163`).
pub const IDENTIFY_COST: i64 = 200;

/// `get_max_load` (`ovr022.cs:8-11`): `1500 + max_encumberance(player)`.
pub fn max_load(ch: &Character, flavor: &dyn gbx_rules::flavor::Flavor) -> i32 {
    1500 + flavor.max_encumbrance(
        ch.stats.str_score.original,
        ch.stats.str_exceptional.current,
    )
}

/// `willOverload` (`ovr022.cs:20-36`): `(weight, would_overload)` — the
/// **spare capacity** when it would, `0` when it would not.
pub fn will_overload(
    ch: &Character,
    added_weight: i32,
    flavor: &dyn gbx_rules::flavor::Flavor,
) -> (i32, bool) {
    let max = max_load(ch, flavor);
    if ch.combat.weight as i32 + added_weight > max {
        (max - ch.combat.weight as i32, true)
    } else {
        (0, false)
    }
}

/// ★ `ShopSellItem`'s payout (`ovr020.cs:1119-1147`): the item is lost, and
/// the money arrives as `value / 5` **platinum** plus `value % 5` gold — the
/// original's own change-making, which is why selling a 12-gp item hands back
/// 2 pl + 2 gp and not 12 gp.
///
/// `willOverload` is tested against `plat + gold` (the *coin count*, since a
/// coin weighs 1), and the overflow goes to the pool with the line
/// `"Overloaded. Money will be put in pool."` — note the gold is added
/// **unconditionally** either way (`:1141`), outside the overload branch.
pub fn sell_payout(
    ch: &mut Character,
    pool: &mut crate::money::MoneySet,
    value: i64,
    flavor: &dyn gbx_rules::flavor::Flavor,
) -> bool {
    let plat = (value / 5) as i32;
    let gold = (value % 5) as i32;
    let (overflow, overloaded) = will_overload(ch, plat + gold, flavor);
    if overloaded {
        if overflow > plat {
            ch.money.platinum = ch.money.platinum.saturating_add(plat as i16);
        } else {
            ch.money.platinum = ch.money.platinum.saturating_add(overflow as i16);
            pool.add(4, plat - overflow);
        }
    } else {
        ch.money.platinum = ch.money.platinum.saturating_add(plat as i16);
    }
    ch.money.gold = ch.money.gold.saturating_add(gold as i16);
    overloaded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::Money;
    use gbx_formats::save_orig::{decode_char_record, CHAR_RECORD_SIZE};

    fn rules() -> RuleSet {
        RuleSet::load()
    }

    fn buyer_with_gold(gold: i16) -> Character {
        let mut bytes = vec![0u8; CHAR_RECORD_SIZE];
        bytes[0] = 4;
        bytes[1..5].copy_from_slice(b"Buyr");
        let rec = decode_char_record(&bytes).unwrap();
        let mut ch = crate::party::character_from_record(&rec, vec![], vec![]);
        ch.money = Money {
            gold,
            ..Default::default()
        };
        ch.combat.weight = 0;
        ch
    }

    #[test]
    fn synthetic_item_round_trips_its_fields() {
        let it = ShopItem::synthetic("Long Sword", 10, 60);
        assert_eq!(it.name(), "Long Sword");
        assert_eq!(it.base_value(), 10);
        assert_eq!(it.weight(), 60);
    }

    #[test]
    fn items_value_applies_the_price_class_shifts() {
        assert_eq!(items_value(16, 0x00), 16); // list price
        assert_eq!(items_value(16, 0x08), 8); // half off
        assert_eq!(items_value(16, 0x40), 64); // 4x markup
                                               // Floor of 1 (ShopChooseItem forces _value>=1).
        assert_eq!(items_value(0, 0x00), 1);
    }

    fn table() -> gbx_formats::items::ItemDataTable {
        gbx_formats::items::ItemDataTable::parse(&[0, 0]).unwrap()
    }

    fn flavor(r: &RuleSet) -> gbx_rules::adnd1::flavor_impl::Adnd1<'_> {
        gbx_rules::adnd1::flavor_impl::Adnd1::new(r)
    }

    #[test]
    fn buying_adds_the_item_deducts_money_and_bumps_weight() {
        let r = rules();
        let shop = Shop::new(vec![ShopItem::synthetic("Dagger", 2, 10)], 0x00);
        let mut buyer = buyer_with_gold(5);
        let mut pool = crate::money::MoneySet::default();
        let before = money::gold_worth(&buyer.money, &r);

        let outcome =
            buy(&shop, 0, &mut buyer, &mut pool, &table(), &flavor(&r), &r).expect("affordable");
        assert_eq!(outcome.price, 2);
        assert!(!outcome.paid_from_pool);
        assert_eq!(buyer.items.len(), 1, "item landed in inventory");
        // ★ `reclac_player_values` re-sums items AND coins — and it runs
        // inside `PlayerAddItem`, i.e. BEFORE the money is taken
        // (`ovr007.cs:99`, `:131`). So the dagger's 10 plus the five gold
        // pieces still in the purse at that moment.
        assert_eq!(buyer.combat.weight, 15, "encumbrance re-summed by reclac");
        assert_eq!(
            money::gold_worth(&buyer.money, &r),
            before - 2,
            "paid exactly the price"
        );
        assert_eq!(shop.items.len(), 1, "★ the stock is not consumed");
    }

    /// ★ `shop_buy`'s second arm (`ovr007.cs:139-146`): a purse that cannot
    /// cover the price falls through to the POOL before the refusal.
    #[test]
    fn a_purse_that_cannot_cover_it_falls_through_to_the_pool() {
        let r = rules();
        let shop = Shop::new(vec![ShopItem::synthetic("Plate Mail", 400, 500)], 0x00);
        let mut buyer = buyer_with_gold(50);
        let mut pool = crate::money::MoneySet::default();
        pool.set(3, 500); // 500 gold in the pool

        let outcome = buy(&shop, 0, &mut buyer, &mut pool, &table(), &flavor(&r), &r)
            .expect("the pool covers it");
        assert!(outcome.paid_from_pool);
        assert_eq!(
            money::gold_worth(&buyer.money, &r),
            50,
            "the buyer's own coins are untouched"
        );
        assert_eq!(pool.gold_worth(), 100);
    }

    #[test]
    fn buying_what_neither_purse_nor_pool_covers_is_refused() {
        let r = rules();
        let shop = Shop::new(vec![ShopItem::synthetic("Plate Mail", 400, 500)], 0x00);
        let mut buyer = buyer_with_gold(50); // 50 gp < 400
        let mut pool = crate::money::MoneySet::default();
        assert_eq!(
            buy(&shop, 0, &mut buyer, &mut pool, &table(), &flavor(&r), &r),
            Err(BuyError::NotEnoughMoney)
        );
        assert!(buyer.items.is_empty(), "nothing bought on refusal");
    }

    /// ★ `PlayerAddItem`'s overload refusal takes NO money (`ovr007.cs:126-137`).
    #[test]
    fn an_overloaded_buyer_keeps_their_coins() {
        let r = rules();
        let shop = Shop::new(vec![ShopItem::synthetic("Anvil", 1, 30_000)], 0x00);
        let mut buyer = buyer_with_gold(500);
        let mut pool = crate::money::MoneySet::default();
        assert_eq!(
            buy(&shop, 0, &mut buyer, &mut pool, &table(), &flavor(&r), &r),
            Err(BuyError::Overloaded)
        );
        assert!(buyer.items.is_empty());
        assert_eq!(money::gold_worth(&buyer.money, &r), 500);
    }

    #[test]
    fn buying_a_missing_index_errors() {
        let r = rules();
        let shop = Shop::new(vec![], 0x00);
        let mut buyer = buyer_with_gold(100);
        let mut pool = crate::money::MoneySet::default();
        assert_eq!(
            buy(&shop, 0, &mut buyer, &mut pool, &table(), &flavor(&r), &r),
            Err(BuyError::NoSuchItem)
        );
    }

    /// ★ `ShopSellItem`'s offer (`ovr020.cs:1091-1110`): half the value, and a
    /// stack of anything but arrows/quarrels is worth a TWENTIETH of that
    /// times its count.
    #[test]
    fn the_sell_offer_halves_and_then_divides_a_stack_by_twenty() {
        let mut rec = ShopItem::synthetic("Long Sword", 100, 60).record;
        assert_eq!(sell_offer(&rec), 50, "half of 100");

        gbx_formats::save_orig::set_item_count(&mut rec, 10);
        assert_eq!(sell_offer(&rec), 25, "10 * 50 / 20");

        // Arrows keep the full per-item price.
        let mut arrows = ShopItem::synthetic("Arrow", 2, 1).record;
        arrows[0x2E] = crate::items::TYPE_ARROW;
        gbx_formats::save_orig::set_item_count(&mut arrows, 20);
        assert_eq!(sell_offer(&arrows), 20, "20 * (2 / 2)");

        // A worthless item is worth nothing — `ItemsValue`'s floor of 1 is a
        // BUYING rule, not a selling one.
        let free = ShopItem::synthetic("Rag", 0, 1).record;
        assert_eq!(sell_offer(&free), 0);
    }

    /// ★ The payout's change-making (`ovr020.cs:1123-1147`): `value / 5`
    /// platinum plus `value % 5` gold.
    #[test]
    fn the_sell_payout_is_platinum_and_gold_change() {
        let r = rules();
        let mut seller = buyer_with_gold(0);
        let mut pool = crate::money::MoneySet::default();
        let overloaded = sell_payout(&mut seller, &mut pool, 12, &flavor(&r));
        assert!(!overloaded);
        assert_eq!(seller.money.platinum, 2);
        assert_eq!(seller.money.gold, 2);
    }
}
