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
    /// Neither the buyer nor pooled money covers the price (`ovr007.cs:147`
    /// "Not enough Money.").
    NotEnoughMoney,
}

/// A successful purchase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyOutcome {
    pub item_name: String,
    pub price: i64,
}

/// Buys `shop.items[index]` for `buyer` (`shop_buy` → `PlayerAddItem`,
/// `ovr007.cs:106-149/85-103`): computes the price, checks affordability, adds
/// a clone of the item to the buyer's inventory, deducts the money, and bumps
/// encumbrance by the item's weight.
///
/// **Deferrals (M4, documented not silent):** the `canCarry` overload check
/// (`ovr020.canCarry`) needs the STR-based max-encumbrance table, and full
/// `reclac_player_values` re-sums weight from *all* items + coins (so spending
/// coins would also change weight). Here weight is incremented by the item's
/// own weight — the "encumbrance updated" the deliverable asks for — with the
/// overload cap and coin-weight delta left to M4's reclac.
pub fn buy(
    shop: &Shop,
    index: usize,
    buyer: &mut Character,
    rules: &RuleSet,
) -> Result<BuyOutcome, BuyError> {
    let item = shop.items.get(index).ok_or(BuyError::NoSuchItem)?;
    let price = items_value(item.base_value(), shop.price_class);

    if !money::can_afford(&buyer.money, price, rules) {
        return Err(BuyError::NotEnoughMoney);
    }

    // Clone the item into the buyer's inventory (ovr007.cs:97 ShallowClone).
    buyer.items.push(item.record.clone());
    // Encumbrance bump (the reclac weight re-sum's item contribution).
    buyer.combat.weight = buyer.combat.weight.saturating_add(item.weight());
    // Pay (ovr007.cs:131 SubtractGoldWorth).
    money::subtract_gold_worth(&mut buyer.money, price, rules);

    Ok(BuyOutcome {
        item_name: item.name(),
        price,
    })
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

    #[test]
    fn buying_adds_the_item_deducts_money_and_bumps_weight() {
        let r = rules();
        let shop = Shop::new(vec![ShopItem::synthetic("Dagger", 2, 10)], 0x00);
        let mut buyer = buyer_with_gold(5);
        let before = money::gold_worth(&buyer.money, &r);

        let outcome = buy(&shop, 0, &mut buyer, &r).expect("affordable");
        assert_eq!(outcome.price, 2);
        assert_eq!(outcome.item_name, "Dagger");
        assert_eq!(buyer.items.len(), 1, "item landed in inventory");
        assert_eq!(buyer.combat.weight, 10, "encumbrance updated");
        assert_eq!(
            money::gold_worth(&buyer.money, &r),
            before - 2,
            "paid exactly the price"
        );
    }

    #[test]
    fn buying_what_you_cannot_afford_is_refused() {
        let r = rules();
        let shop = Shop::new(vec![ShopItem::synthetic("Plate Mail", 400, 500)], 0x00);
        let mut buyer = buyer_with_gold(50); // 50 gp < 400
        assert_eq!(buy(&shop, 0, &mut buyer, &r), Err(BuyError::NotEnoughMoney));
        assert!(buyer.items.is_empty(), "nothing bought on refusal");
    }

    #[test]
    fn buying_a_missing_index_errors() {
        let r = rules();
        let shop = Shop::new(vec![], 0x00);
        let mut buyer = buyer_with_gold(100);
        assert_eq!(buy(&shop, 0, &mut buyer, &r), Err(BuyError::NoSuchItem));
    }
}
