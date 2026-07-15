//! The 7-denomination money model's gold-worth arithmetic (`MoneySet.cs`,
//! M3 step 6) — shared by the training fee (deliverable 4) and shop purchases
//! (deliverable 5). Transliterated from coab (D11), pack-backed for the
//! per-denomination copper values (`coin_conversion_rates`,
//! `constants.toml`).

use crate::party::Money;
use gbx_rules::adnd1::constants::coin_conversion_rate;
use gbx_rules::pack::RuleSet;

/// coab's `Money.Gold` coin index — the denomination gold-worth normalizes to.
const GOLD: usize = 3;

/// `MoneySet.GetGoldWorth` (`MoneySet.cs:70-79`): the copper-equivalent sum of
/// Copper..Platinum (gems/jewelry excluded — variable-value items), divided by
/// the gold rate. `per_copper[coin]` is the pack's `coin_conversion_rates`.
pub fn gold_worth(money: &Money, rules: &RuleSet) -> i64 {
    let mut copper = 0i64;
    for coin in 0..=4usize {
        copper += money.get_coin(coin) as i64 * coin_conversion_rate(rules, coin) as i64;
    }
    copper / coin_conversion_rate(rules, GOLD) as i64
}

/// `MoneySet.SubtractGoldWorth` (`MoneySet.cs:93-128`): spend `gold` gp,
/// consuming the lowest denominations first (the `+1` over-pay so the loop
/// terminates) then making change back from the highest — the exact
/// coin-optimizing algorithm, for coin-distribution parity. **Assumes
/// affordability** (`gold_worth >= gold`); coab guards every call site the
/// same way, and this stops at Platinum rather than indexing past it if a
/// caller violates that (a defensive divergence from coab's unchecked
/// `per_copper[5]`, never reached in practice).
pub fn subtract_gold_worth(money: &mut Money, gold: i64, rules: &RuleSet) {
    let per_copper = |coin: usize| coin_conversion_rate(rules, coin) as i64;
    let mut coppers = gold * per_copper(GOLD);

    // Greedy pass Copper → Platinum, over-paying by one coin each step.
    let mut coin = 0usize;
    while coppers > 0 && coin <= 4 {
        let rate = per_copper(coin);
        let mut sub = coppers / rate + 1;
        let have = money.get_coin(coin) as i64;
        if have < sub {
            sub = have;
        }
        coppers -= rate * sub;
        money.set_coin(coin, (money.get_coin(coin) as i64 - sub) as i16);
        coin += 1;
    }

    // Make change back Platinum → Copper for the over-payment.
    if coppers < 0 {
        coppers = coppers.abs();
        let mut coin = 4i64;
        while coppers > 0 && coin >= 0 {
            let rate = per_copper(coin as usize);
            let add = coppers / rate;
            coppers -= rate * add;
            let idx = coin as usize;
            money.set_coin(idx, (money.get_coin(idx) as i64 + add) as i16);
            coin -= 1;
        }
    }
}

/// True if `money` can cover `gold` gp (the affordability guard every spend
/// site checks first).
pub fn can_afford(money: &Money, gold: i64, rules: &RuleSet) -> bool {
    gold_worth(money, rules) >= gold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> RuleSet {
        RuleSet::load()
    }

    #[test]
    fn gold_worth_matches_the_per_copper_conversions() {
        let r = rules();
        // 1 platinum = 1000 copper = 5 gold; 3 gold = 3 gold.
        let m = Money {
            platinum: 1,
            gold: 3,
            ..Default::default()
        };
        assert_eq!(gold_worth(&m, &r), 8); // (1000 + 600)/200 = 8
                                           // Gems/jewelry are excluded from gold worth.
        let m2 = Money {
            gems: 5,
            jewelry: 5,
            ..Default::default()
        };
        assert_eq!(gold_worth(&m2, &r), 0);
    }

    #[test]
    fn subtract_preserves_total_worth_minus_the_cost() {
        let r = rules();
        let mut m = Money {
            gold: 10,
            platinum: 2,
            ..Default::default()
        };
        let before = gold_worth(&m, &r); // (2000 + 2000)/200 = 20
        assert_eq!(before, 20);
        subtract_gold_worth(&mut m, 7, &r);
        assert_eq!(gold_worth(&m, &r), 13, "spent exactly 7 gp of worth");
    }

    #[test]
    fn subtract_makes_change_from_a_single_large_coin() {
        let r = rules();
        // Only a platinum (=5 gp worth); pay 1 gp, expect 4 gp of change back.
        let mut m = Money {
            platinum: 1,
            ..Default::default()
        };
        subtract_gold_worth(&mut m, 1, &r);
        assert_eq!(gold_worth(&m, &r), 4);
    }

    #[test]
    fn can_afford_reflects_gold_worth() {
        let r = rules();
        let m = Money {
            gold: 1000,
            ..Default::default()
        };
        assert!(can_afford(&m, 1000, &r));
        assert!(!can_afford(&m, 1001, &r));
    }
}
