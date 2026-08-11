//! The full character sheet (`playerDisplayFull`/`display_player_stats01`/
//! `displayMoney`, `ovr020.cs:44/153/134`) and the compact party summary
//! (`PartySummary`, `ovr025.cs:216-261`) — M3 step 6 deliverable 1.
//!
//! Two layers, cleanly split:
//! - [`SheetView`]/[`sheet_view`] compute a fully-derived, serde-able snapshot
//!   of one character's sheet from the stored [`Character`] fields alone. Every
//!   display number the sheet shows is a *reclac'd* field the original already
//!   stored (`reclac_player_values`, `ovr025.cs:338-495`, runs before display
//!   and writes back into the same `player.*` cells the layout reads) — so a
//!   faithful sheet needs no combat-math re-derivation here, only the same
//!   presentation transforms coab applies at draw time (`0x3C - ac`, the
//!   `18(00)` percentile format, the `NdM+B` damage string). Combat re-derivation
//!   from equipment is M4 scope.
//! - [`render_sheet`]/[`render_party_summary`] paint a `SheetView` into the
//!   core framebuffer at coab's exact cell coordinates.
//!
//! **UI-string policy (D10 clarification):** the name tables below (class,
//! alignment, race, sex, status, coin) are functional interface vocabulary
//! documented in the freely distributed manual — reproduced verbatim from
//! coab (`ovr020.cs:19-41`, read-for-behavior per D11) for fidelity, exactly
//! as the stat labels (`STR`/`AC`/`THAC0`) already are. No multi-word game
//! prose is embedded.

use crate::framebuffer::Framebuffer;
use crate::party::{Character, Money};
use crate::symbols::SymbolSets;
use crate::text::draw_string;
use gbx_formats::font::Font;

// --- Name tables (ovr020.cs:19-41, verbatim functional vocabulary) ---

/// `sexString` (`ovr020.cs:19`).
const SEX: [&str; 2] = ["Male", "Female"];

/// `raceString` (`ovr020.cs:20-21`), index = raw `race` (0 = Monster).
const RACE: [&str; 8] = [
    "Monster", "Dwarf", "Elf", "Gnome", "Half-Elf", "Halfling", "Half-Orc", "Human",
];

/// `alignmentString` (`ovr020.cs:23-25`), index = raw `alignment` 0..=8,
/// row-major Law/Neutral/Chaos × Good/Neutral/Evil.
const ALIGNMENT: [&str; 9] = [
    "Lawful Good",
    "Lawful Neutral",
    "Lawful Evil",
    "Neutral Good",
    "True Neutral",
    "Neutral Evil",
    "Chaotic Good",
    "Chaotic Neutral",
    "Chaotic Evil",
];

/// `classString` (`ovr020.cs:27-32`), index = raw `class_id`: 0..=7
/// single-class, 8..=16 the pre-joined multiclass combos (so multiclass is a
/// table lookup, *not* assembled at display time — unlike the Level field).
const CLASS: [&str; 17] = [
    "Cleric",
    "Druid",
    "Fighter",
    "Paladin",
    "Ranger",
    "Magic-User",
    "Thief",
    "Monk",
    "Cleric/Fighter",
    "Cleric/Fighter/Magic-User",
    "Cleric/Ranger",
    "Cleric/Magic-User",
    "Cleric/Thief",
    "Fighter/Magic-User",
    "Fighter/Thief",
    "Fighter/Magic-User/Thief",
    "Magic-User/Thief",
];

/// `statusString` (`ovr020.cs:36-38`), index = `health_status` 0..=8.
const STATUS: [&str; 9] = [
    "Okay",
    "Animated",
    "tempgone",
    "Running",
    "Unconscious",
    "Dying",
    "Dead",
    "Stoned",
    "Gone",
];

/// `Money.names`/`moneyString` (`ovr020.cs:40-41`, `MoneySet.cs:17`), index =
/// coin type 0..=6 in [`Money`]'s own field order.
const COIN: [&str; 7] = [
    "Copper", "Silver", "Electrum", "Gold", "Platinum", "Gems", "Jewelry",
];

/// The six ability-row labels (`statShortString`, `ovr020.cs:34`) — the fixed
/// STR/INT/WIS/DEX/CON/CHA display order.
const STAT_LABELS: [&str; 6] = ["STR", "INT", "WIS", "DEX", "CON", "CHA"];

/// A table index or `?` for an out-of-range raw byte (untrusted save data
/// never panics the sheet — a corrupt index shows a visible placeholder).
fn lookup(table: &[&str], idx: u8) -> String {
    table
        .get(idx as usize)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Coins by coab coin-type index (0 = Copper .. 6 = Jewelry), matching
/// [`COIN`]'s order — `MoneySet.GetCoins(coinType)` (`ovr020.cs:142`).
fn coin_amount(m: &Money, coin_type: usize) -> i16 {
    match coin_type {
        0 => m.copper,
        1 => m.silver,
        2 => m.electrum,
        3 => m.gold,
        4 => m.platinum,
        5 => m.gems,
        _ => m.jewelry,
    }
}

// --- The computed sheet snapshot ---

/// One ability row: label plus the value as coab renders it (STR carries the
/// `(00)` exceptional-strength parenthetical inline).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatRow {
    pub label: String,
    /// The stat's `full` value as a string (`display_stat`, `ovr020.cs:210`).
    pub value: String,
    /// The `(00)`-style exceptional-strength suffix, present only on the STR
    /// row when `str == 18` and the percentile is nonzero (`ovr020.cs:213-229`).
    pub exceptional: Option<String>,
}

/// One shown coin row: name and amount (`displayMoney`, `ovr020.cs:140-147`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CoinRow {
    pub name: String,
    pub amount: i16,
}

/// A fully-derived, presentation-ready snapshot of one character's sheet —
/// every string/number the layout draws, computed once from [`Character`].
/// serde-able so `tools/inspect` (and any frontend) can consume it without
/// re-deriving.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SheetView {
    pub name: String,
    pub is_npc: bool,
    /// The row-3 identity line: "Male Human Age 20" (`ovr020.cs:57-68`).
    pub identity: String,
    pub alignment: String,
    pub class: String,
    pub stats: [StatRow; 6],
    /// Slash-joined per-class levels (`ovr020.cs:86-107`).
    pub level: String,
    pub exp: i32,
    /// Descending display AC = `0x3C - ac` (`Player.cs:598`).
    pub ac: i32,
    /// Descending display THAC0 = `0x3C - hitBonus` (`ovr020.cs:169`).
    pub thac0: i32,
    /// ★ `displayPlayerName`'s two inputs (`ovr025.cs:827-838`) — the roster
    /// panel paints a removed member's name in `0x0C` and an enemy's in `0x0E`.
    /// Carried on the view since roll-credits slice 6, because the post-fight
    /// writeback finally makes `in_combat == false` reachable on a walk-loop
    /// roster: a member who died in the last fight now really is one.
    pub in_combat: bool,
    /// `combat_team` (`0` ours, `1` enemy).
    pub combat_team: u8,
    pub hp_current: u8,
    pub hp_max: u8,
    /// `NdM+B` primary-attack damage (`ovr020.cs:172-173`).
    pub damage: String,
    /// Carried weight (`player.weight`, `ovr020.cs:180`).
    pub encumbrance: i16,
    /// `player.movement` (`ovr020.cs:182`); slow/haste display scaling is an
    /// affect-system concern deferred to M4 (see [`sheet_view`]).
    pub movement: u8,
    /// Coin rows in on-screen order (Jewelry → Copper), nonzero only.
    pub money: Vec<CoinRow>,
    pub status: String,
    /// The dynamic command bar (`viewPlayer`, `ovr020.cs:250-291`).
    pub command_bar: String,
}

/// True if `spell_list` holds any real memorized spell — `SpellList.HasSpells`
/// (`ovr020.cs:253`). Empty slots read as `0`; `0xFF` is the coab
/// end-of-list sentinel. Approximate (cosmetic — only gates the "Spells"
/// command word); exact slot semantics are M5 (Vancian) scope.
fn has_spells(list: &[u8]) -> bool {
    list.iter().any(|&b| b != 0 && b != 0xFF)
}

/// `Money.AnyMoney` (`ovr020.cs:254`).
fn any_money(m: &Money) -> bool {
    (0..7).any(|c| coin_amount(m, c) > 0)
}

/// The `18(00)` exceptional-strength suffix (`ovr020.cs:213-229`): only when
/// `str == 18` and the percentile is nonzero; `< 10` zero-pads, `100`
/// renders as `00`.
fn exceptional_suffix(str_full: u8, str00: u8) -> Option<String> {
    if str_full != 18 || str00 == 0 {
        return None;
    }
    let text = if str00 == 100 {
        "00".to_string()
    } else if str00 < 10 {
        format!("0{str00}")
    } else {
        str00.to_string()
    };
    Some(format!("({text})"))
}

/// `"{count}d{size}{+bonus}"` (`ovr020.cs:172-173`): a `+` only when the bonus
/// is strictly positive, no suffix when zero, a bare `-N` when negative
/// (`ToString` of a negative already carries its sign).
fn damage_string(dice_count: u8, dice_size: u8, damage_bonus: i8) -> String {
    let mut s = format!("{dice_count}d{dice_size}");
    if damage_bonus > 0 {
        s.push('+');
        s.push_str(&damage_bonus.to_string());
    } else if damage_bonus < 0 {
        s.push_str(&damage_bonus.to_string());
    }
    s
}

/// The slash-joined level field (`ovr020.cs:86-107`): each base class 0..=7
/// with a live level contributes `ClassLevel[i] + ClassLevelsOld[i]`.
///
/// **Simplification (flagged, not silently absorbed):** coab's inclusion test
/// (`ovr020.cs:93-94`) is `ClassLevel[i] > 0 || (ClassLevelsOld[i] > 0 &&
/// ClassLevelsOld[i] < HumanCurrentClassLevel_Zero(player))` — the second
/// clause keeps a *dual-classed* human's abandoned old class visible only
/// until the new class surpasses it. This reproduces the first clause exactly
/// and the second's `> 0` half, but not the `HumanCurrentClassLevel_Zero`
/// threshold (a `reclac`-side value, M4 scope). Correct for every
/// single-class and active-multiclass character; a fully-transitioned human
/// dual-class could show one extra `/N`. TODO(M4): thread the threshold once
/// `reclac_player_values` lands.
fn level_string(ch: &Character) -> String {
    let mut parts = Vec::new();
    for i in 0..8 {
        let cur = ch.class_level[i];
        let old = ch.class_levels_old[i];
        if cur > 0 || old > 0 {
            parts.push((cur as u16 + old as u16).to_string());
        }
    }
    parts.join("/")
}

/// The dynamic command bar (`viewPlayer`, `ovr020.cs:250-291`).
///
/// Faithfully includes Items (any items), Spells (any memorized), Trade/Drop
/// (any money), and always Exit. **Heal/Cure are deliberately omitted with a
/// TODO**, not stubbed as always-present: their guards `CanCastHeal`/
/// `CanCastCureDiseases` (`ovr020.cs:281-289`) depend on paladin spell/affect
/// state this milestone doesn't model (no spell casting, no affect system —
/// scope guardrails). Adding them unconditionally would misrepresent a
/// non-paladin's bar; omitting them is the honest subset. TODO(M5): gate
/// Heal/Cure on the real cast-ability checks.
fn command_bar(ch: &Character) -> String {
    let mut bar = String::new();
    if !ch.items.is_empty() {
        bar.push_str("Items ");
    }
    if has_spells(&ch.magic.spell_list) {
        bar.push_str("Spells ");
    }
    if any_money(&ch.money) {
        // Trade needs a second party member to trade *with*; the walk-loop
        // View path always has the roster, so both words appear together
        // here exactly as coab emits them for an out-of-combat sheet.
        bar.push_str("Trade ");
        bar.push_str("Drop ");
    }
    bar.push_str("Exit");
    bar
}

/// Builds the presentation snapshot for one character (`playerDisplayFull` +
/// `display_player_stats01` + `displayMoney`, `ovr020.cs`). Reads only stored
/// fields — see the module doc comment on why no combat re-derivation is
/// needed for a faithful sheet.
pub fn sheet_view(ch: &Character) -> SheetView {
    // Row-3 identity line: "Male Human Age 20" (ovr020.cs:57-68).
    let identity = format!(
        "{} {} Age {}",
        lookup(&SEX, ch.sex),
        lookup(&RACE, ch.race),
        ch.age
    );

    let s = &ch.stats;
    let stat_values = [
        s.str_score.current,
        s.int.current,
        s.wis.current,
        s.dex.current,
        s.con.current,
        s.cha.current,
    ];
    let stats = std::array::from_fn(|i| StatRow {
        label: STAT_LABELS[i].to_string(),
        value: stat_values[i].to_string(),
        exceptional: if i == 0 {
            exceptional_suffix(s.str_score.current, s.str_exceptional.current)
        } else {
            None
        },
    });

    // displayMoney iterates coin types 6 → 0 (Jewelry first), nonzero only
    // (ovr020.cs:140-147).
    let money = (0..7)
        .rev()
        .filter_map(|c| {
            let amount = coin_amount(&ch.money, c);
            (amount > 0).then_some(CoinRow {
                name: COIN[c].to_string(),
                amount,
            })
        })
        .collect();

    // Primary-attack damage decomposes the current 8-byte attack profile:
    // DiceCount @+2 (0x19e), DiceSize @+4 (0x1a0), DamageBonus @+6 (0x1a2)
    // within the 0x19c block (Player.cs:664/681/698 offsets vs save_orig's
    // fixed8(0x19c)). These are already strength/weapon-adjusted by the
    // original's own reclac at save time.
    let cur = &ch.combat.attacks.current;
    let damage = damage_string(cur[2], cur[4], cur[6] as i8);

    SheetView {
        name: ch.name.clone(),
        is_npc: ch.is_npc(),
        identity,
        alignment: lookup(&ALIGNMENT, ch.alignment),
        class: lookup(&CLASS, ch.class_id),
        stats,
        level: level_string(ch),
        exp: ch.exp,
        ac: 0x3C - ch.combat.ac as i32,
        thac0: 0x3C - ch.combat.thac0_current as i32,
        in_combat: ch.status.in_combat,
        combat_team: ch.status.combat_team,
        hp_current: ch.hit_point_current,
        hp_max: ch.hit_point_max,
        damage,
        encumbrance: ch.combat.weight,
        movement: ch.combat.movement,
        money,
        status: lookup(&STATUS, ch.status.health_status),
        command_bar: command_bar(ch),
    }
}

// --- Rendering (coab cell coordinates; colors from the reference capture) ---

/// EGA palette indices used by the sheet (coab's own color args, confirmed
/// against `charsheet-mathew-slotA.png`).
const WHITE: u8 = 0x0F;
const GREEN: u8 = 0x0A;
const CYAN: u8 = 0x0B;
/// `display_hp`'s wounded color (`ovr025.cs:276`) — yellow, drawn whenever
/// `hit_point_current < hit_point_max`.
const HP_WOUNDED: u8 = 0x0E;
/// `displayPlayerName`'s `in_combat == false` colour (`ovr025.cs:831`).
const NAME_REMOVED: u8 = 0x0C;
/// `displayPlayerName`'s `combat_team == Enemy` colour (`ovr025.cs:835`).
const NAME_ENEMY: u8 = 0x0E;
const BG: u8 = 0;

/// Paints `view` into `fb` as the full character sheet, at coab's exact
/// `displayString(_, color, row, col)` coordinates (`ovr020.cs`). Draws the
/// ornate outer frame first (`seg037.DrawFrame_Outer`, `ovr020.cs:46`); a
/// missing symbol set is non-fatal (the text still lands).
pub fn render_sheet(fb: &mut Framebuffer, font: &Font, sets: &SymbolSets, view: &SheetView) {
    let _ = crate::frames::draw_frame_outer(fb, sets);

    // Name (row 1, col 1) — cyan in the reference capture.
    draw_string(fb, font, &view.name, 1, 1, BG, CYAN);
    if view.is_npc {
        draw_string(fb, font, "(NPC)", 1, view.name.len() + 3, BG, GREEN);
    }

    // Identity / alignment / class (rows 3/4/5, col 1) — white.
    draw_string(fb, font, &view.identity, 3, 1, BG, WHITE);
    draw_string(fb, font, &view.alignment, 4, 1, BG, WHITE);
    draw_string(fb, font, &view.class, 5, 1, BG, WHITE);

    // Ability rows 7..12, green label + green value (display_stat color 0x0A).
    for (i, row) in view.stats.iter().enumerate() {
        let y = i + 7;
        draw_string(fb, font, &row.label, y, 1, BG, GREEN);
        // Value column: 5, shifted +1 for a single-digit value so 2-digit and
        // 1-digit both right-pad the label gap the same way (ovr020.cs:205-211).
        let col = if row.value.len() < 2 { 6 } else { 5 };
        draw_string(fb, font, &row.value, y, col, BG, GREEN);
        if let Some(exc) = &row.exceptional {
            // Fixed (row 7, col 7) — only ever the STR row (ovr020.cs:229).
            draw_string(fb, font, exc, 7, 7, BG, GREEN);
        }
    }

    // Money block: rows from 7, col 12, "{name:>8} {amount}" (ovr020.cs:144).
    for (i, coin) in view.money.iter().enumerate() {
        let text = format!("{:>8} {}", coin.name, coin.amount);
        draw_string(fb, font, &text, 7 + i, 12, BG, GREEN);
    }

    // Level / Exp (row 15) — white label + white value (ovr020.cs:84-110).
    draw_string(fb, font, "Level", 15, 1, BG, WHITE);
    draw_string(fb, font, &view.level, 15, 7, BG, WHITE);
    draw_string(fb, font, &format!("Exp {}", view.exp), 15, 17, BG, WHITE);

    // display_player_stats01: white labels, green values (ovr020.cs:160-195).
    draw_string(fb, font, "AC", 17, 1, BG, WHITE);
    draw_string(fb, font, &view.ac.to_string(), 17, 4, BG, GREEN);
    draw_string(fb, font, "HP", 18, 1, BG, WHITE);
    draw_string(fb, font, &view.hp_current.to_string(), 18, 4, BG, GREEN);

    draw_string(fb, font, "THAC0", 17, 9, BG, WHITE);
    draw_string(fb, font, &view.thac0.to_string(), 17, 15, BG, GREEN);
    draw_string(fb, font, "Damage", 18, 8, BG, WHITE);
    draw_string(fb, font, &view.damage, 18, 15, BG, GREEN);

    draw_string(fb, font, "Encumbrance", 17, 22, BG, WHITE);
    draw_string(fb, font, &view.encumbrance.to_string(), 17, 34, BG, GREEN);
    draw_string(fb, font, "Movement", 18, 25, BG, WHITE);
    draw_string(fb, font, &view.movement.to_string(), 18, 34, BG, GREEN);

    // Status (row 22): white label, green value (ovr020.cs:130-131). Weapon/
    // Armor rows (20/21) need decoded item names — deferred with items.
    draw_string(fb, font, "Status", 22, 1, BG, WHITE);
    draw_string(fb, font, &view.status, 22, 8, BG, GREEN);

    // The dynamic command bar along the bottom (row 24).
    draw_string(fb, font, &view.command_bar, 24, 1, BG, WHITE);
}

/// One row of the compact party summary (`PartySummary`, `ovr025.cs:216-261`):
/// Name | AC | HP. The full sheet's `SheetView` is overkill for this list, so
/// callers pass the three columns directly.
///
/// `selected` is `PartySummary`'s own `player` argument: that one row's name
/// draws white (`displayString(name, 0, 15, …)`, `ovr025.cs:238-239`), every
/// other goes through `displayPlayerName` (`ovr025.cs:827-846`), whose three
/// colors are `0x0C` (`in_combat == false`), `0x0E` (`combat_team ==
/// Enemy`), else `0x0B`.
///
/// ★ **Slice 6 made the first arm reachable.** M3 painted every row `0x0B` and
/// argued the removed-colour arm could not fire on a walk-loop roster, because
/// a live party member's stored `in_combat` is `true`. That was true only
/// because nothing ever wrote it back: since
/// `combat_host::carry_state_home`, a member who died or was knocked out in the
/// last fight walks around with `in_combat == false`, and the panel now says so
/// — dark red for the fallen, which together with the character sheet's own
/// nine-row `statusString` is the whole of "the roster tells the truth".
///
/// **Wilderness:** `PartySummary` early-returns outright when
/// `game_state == WildernessMap` (`ovr025.cs:218-221`) — the panel is not a
/// wilderness fixture. That predicate belongs to the caller (the engine's
/// per-tick paint), not here.
pub fn render_party_summary(
    fb: &mut Framebuffer,
    font: &Font,
    rows: &[SheetView],
    selected: Option<usize>,
) {
    // Header (row 2): "Name" at col 17, "AC  HP" at col 33 (ovr025.cs:226-227,
    // the non-StartGameMenu x origin). Body from row 4 (ovr025.cs:229).
    draw_string(fb, font, "Name", 2, 17, BG, WHITE);
    draw_string(fb, font, "AC  HP", 2, 33, BG, WHITE);
    for (i, r) in rows.iter().enumerate() {
        let y = 4 + i;
        // draw8x8_clear_area(y_pos, 0x26, y_pos, x_pos) per row (ovr025.cs:234)
        // — a shortened roster must not leave the old row behind.
        crate::draw::cell_rect_fill(fb, BG, y, y, 17, 0x26);
        // `displayPlayerName` (`ovr025.cs:827-838`): removed first, then enemy,
        // then the live team member's `0x0B` cyan.
        let name_color = if selected == Some(i) {
            WHITE
        } else if !r.in_combat {
            NAME_REMOVED
        } else if r.combat_team != 0 {
            NAME_ENEMY
        } else {
            CYAN
        };
        draw_string(fb, font, &r.name, y, 17, BG, name_color);
        // AC left-justified width 3 at col 31 (ovr025.cs:244, "{0,-3}").
        draw_string(fb, font, &format!("{:<3}", r.ac), y, 31, BG, GREEN);
        // HP right-aligned so the LAST digit cell is always 0x26 = 38
        // (ovr025.cs:246-256: `hpXPos` is 2 for a 1-digit value, 1 for 2
        // digits, 0 otherwise, and the draw goes to `hpXPos + 0x24`) — so
        // the string STARTS at 39 - len, not 38 - len.
        let hp = r.hp_current.to_string();
        let col = 39usize.saturating_sub(hp.len());
        // `display_hp` (ovr025.cs:270-289): 0x0E while wounded, 0x0A at full
        // health. (Its third color, 0x0D, is the `hightlighted` argument's,
        // which `PartySummary` always passes `false`.)
        let hp_color = if r.hp_current < r.hp_max {
            HP_WOUNDED
        } else {
            GREEN
        };
        draw_string(fb, font, &hp, y, col, BG, hp_color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party::{AbilityScorePair, AbilityScores, AttackProfiles, CombatStats};

    /// A MATHEW-like paladin matching the reference capture
    /// (`charsheet-mathew-slotA.png`): the exact stored fields that, run
    /// through the sheet's display transforms, must reproduce every value on
    /// screen. (Synthetic — self-authored, no game bytes; the real bundled
    /// save is exercised by the local-tier test in `import.rs`/the demo.)
    use crate::test_support::blank_character;

    fn mathew() -> Character {
        let mut ch = blank_character();
        ch.name = "MATHEW".into();
        ch.sex = 0; // Male
        ch.race = 7; // Human
        ch.age = 20;
        ch.alignment = 0; // Lawful Good
        ch.class_id = 3; // Paladin
        ch.stats = AbilityScores {
            str_score: AbilityScorePair {
                current: 18,
                original: 18,
            },
            str_exceptional: AbilityScorePair {
                current: 100,
                original: 100,
            },
            int: AbilityScorePair {
                current: 17,
                original: 17,
            },
            wis: AbilityScorePair {
                current: 16,
                original: 16,
            },
            dex: AbilityScorePair {
                current: 17,
                original: 17,
            },
            con: AbilityScorePair {
                current: 17,
                original: 17,
            },
            cha: AbilityScorePair {
                current: 17,
                original: 17,
            },
        };
        ch.class_level[3] = 5; // paladin level 5
        ch.exp = 25000;
        ch.hit_point_current = 49;
        ch.hit_point_max = 49;
        ch.money.platinum = 300;
        // AC 7 = 0x3C - 53; THAC0 13 = 0x3C - 47; weight 300; movement 12.
        ch.combat = CombatStats {
            ac: 53,
            thac0_current: 47,
            weight: 300,
            movement: 12,
            attacks: AttackProfiles {
                base: [0; 8],
                // DiceCount@+2=1, DiceSize@+4=2, DamageBonus@+6=6 → "1d2+6".
                current: [0, 0, 1, 0, 2, 0, 6, 0],
            },
            ..Default::default()
        };
        ch.status.health_status = 0; // Okay
        ch
    }

    #[test]
    fn mathew_sheet_reproduces_the_reference_capture() {
        let view = sheet_view(&mathew());
        // The acceptance-check values (M3 step 6 deliverable 1).
        assert_eq!(view.stats[0].value, "18");
        assert_eq!(view.stats[0].exceptional.as_deref(), Some("(00)"));
        assert_eq!(view.level, "5");
        assert_eq!(view.exp, 25000);
        assert_eq!(view.hp_current, 49);
        assert_eq!(view.class, "Paladin");
        assert_eq!(view.alignment, "Lawful Good");
        // The rest of the on-screen numbers.
        assert_eq!(view.identity, "Male Human Age 20");
        assert_eq!(view.ac, 7);
        assert_eq!(view.thac0, 13);
        assert_eq!(view.damage, "1d2+6");
        assert_eq!(view.encumbrance, 300);
        assert_eq!(view.movement, 12);
        assert_eq!(view.status, "Okay");
        assert_eq!(
            view.money,
            vec![CoinRow {
                name: "Platinum".into(),
                amount: 300
            }]
        );
        assert_eq!(
            view.stats
                .iter()
                .map(|r| r.value.as_str())
                .collect::<Vec<_>>(),
            vec!["18", "17", "16", "17", "17", "17"]
        );
    }

    #[test]
    fn exceptional_suffix_formats_percentiles_like_coab() {
        assert_eq!(exceptional_suffix(18, 100).as_deref(), Some("(00)"));
        assert_eq!(exceptional_suffix(18, 5).as_deref(), Some("(05)"));
        assert_eq!(exceptional_suffix(18, 76).as_deref(), Some("(76)"));
        // Only on an 18 with a nonzero percentile.
        assert_eq!(exceptional_suffix(18, 0), None);
        assert_eq!(exceptional_suffix(17, 50), None);
    }

    #[test]
    fn damage_string_signs_the_bonus_like_coab() {
        assert_eq!(damage_string(1, 2, 6), "1d2+6");
        assert_eq!(damage_string(2, 6, 0), "2d6");
        assert_eq!(damage_string(1, 8, -1), "1d8-1");
    }

    #[test]
    fn multiclass_level_joins_with_slashes() {
        let mut ch = blank_character();
        ch.class_level[2] = 6; // fighter
        ch.class_level[5] = 5; // magic-user
        assert_eq!(level_string(&ch), "6/5");
    }

    #[test]
    fn money_rows_are_reverse_order_nonzero_only() {
        let mut ch = blank_character();
        ch.money.copper = 12;
        ch.money.gold = 40;
        ch.money.jewelry = 1;
        let view = sheet_view(&ch);
        // Jewelry (6) first, then Gold (3), then Copper (0).
        assert_eq!(
            view.money,
            vec![
                CoinRow {
                    name: "Jewelry".into(),
                    amount: 1
                },
                CoinRow {
                    name: "Gold".into(),
                    amount: 40
                },
                CoinRow {
                    name: "Copper".into(),
                    amount: 12
                },
            ]
        );
    }

    #[test]
    fn command_bar_matches_the_available_actions() {
        let mut ch = blank_character();
        // No items, no spells, no money: just Exit.
        assert_eq!(command_bar(&ch), "Exit");
        ch.money.gold = 5;
        assert_eq!(command_bar(&ch), "Trade Drop Exit");
        ch.items.push(vec![0u8; 4]);
        assert_eq!(command_bar(&ch), "Items Trade Drop Exit");
    }

    /// Every glyph fully lit, so "which cells did this string land in" is
    /// readable straight off the framebuffer.
    fn solid_font() -> Font {
        gbx_formats::font::decode(&vec![
            0xFFu8;
            gbx_formats::font::GLYPH_COUNT
                * gbx_formats::font::GLYPH_BYTES
        ])
    }

    /// The columns a roster row's HP string occupies, and the colour it drew.
    fn hp_cells(rows: &[SheetView]) -> (Vec<usize>, u8) {
        let mut fb = Framebuffer::new();
        render_party_summary(&mut fb, &solid_font(), rows, Some(0));
        let y = 4 * 8; // row 4 = the first roster row
        let mut cols = Vec::new();
        let mut colour = 0;
        // From 35: the AC field's own `{:<3}` occupies 31..=33.
        for col in 35..40usize {
            let px = fb.get_pixel(col * 8, y);
            if px != BG {
                cols.push(col);
                colour = px;
            }
        }
        (cols, colour)
    }

    /// ★ `PartySummary`'s HP column is RIGHT-aligned on cell `0x26` = 38
    /// (`ovr025.cs:246-256`: `hpXPos` 2/1/0 by digit count, drawn at
    /// `hpXPos + 0x24`), so the string starts at `39 - len`. Ours started at
    /// `38 - len` — a whole cell left of the original, every row.
    #[test]
    fn roster_hp_is_right_aligned_on_column_38() {
        let mut ch = mathew();
        for (hp, expected) in [(7u8, vec![38]), (49, vec![37, 38]), (100, vec![36, 37, 38])] {
            ch.hit_point_current = hp;
            ch.hit_point_max = hp;
            let (cols, _) = hp_cells(&[sheet_view(&ch)]);
            assert_eq!(cols, expected, "hp {hp} must end on column 38");
        }
    }

    /// ★ `display_hp` (`ovr025.cs:270-289`) draws `0x0E` while
    /// `hit_point_current < hit_point_max` and `0x0A` at full health. Ours
    /// was always green, so a wounded party read as healthy.
    #[test]
    fn roster_hp_is_yellow_while_wounded_and_green_at_full() {
        let mut ch = mathew();
        ch.hit_point_max = 49;
        ch.hit_point_current = 49;
        assert_eq!(hp_cells(&[sheet_view(&ch)]).1, GREEN);

        ch.hit_point_current = 12;
        assert_eq!(hp_cells(&[sheet_view(&ch)]).1, HP_WOUNDED);
    }

    /// ★ Roll-credits slice 6: the sheet's Status line tells the truth for the
    /// whole ladder, `stoned` and `gone` included (`statusString`,
    /// `ovr020.cs:36-38`). Before the slice a petrified member's *combat*
    /// record read back as "Okay"; the sheet's own table was already complete,
    /// so this pins the pair together.
    #[test]
    fn the_status_line_names_every_rung_including_stoned_and_gone() {
        let mut ch = mathew();
        for (byte, text) in [
            (0u8, "Okay"),
            (1, "Animated"),
            (2, "tempgone"),
            (3, "Running"),
            (4, "Unconscious"),
            (5, "Dying"),
            (6, "Dead"),
            (7, "Stoned"),
            (8, "Gone"),
        ] {
            ch.status.health_status = byte;
            assert_eq!(sheet_view(&ch).status, text, "status byte {byte}");
            // The scene's own copy of the table has to agree with the sheet's.
            assert_eq!(
                crate::combat::scene::render::status_string(crate::combat::decode_health_status(
                    byte
                )),
                text
            );
        }
    }

    /// The colour of one roster row's name cell.
    fn name_colour(rows: &[SheetView], row: usize) -> u8 {
        let mut fb = Framebuffer::new();
        // `selected` is somebody else, so the row takes `displayPlayerName`'s
        // own choice rather than the white highlight.
        render_party_summary(&mut fb, &solid_font(), rows, None);
        fb.get_pixel(17 * 8, (4 + row) * 8)
    }

    /// ★ `displayPlayerName`'s three arms (`ovr025.cs:827-838`) — and the first
    /// of them is reachable on a walk-loop roster only since slice 6's
    /// post-fight writeback, which is what puts `in_combat == false` on a
    /// member who fell in the last fight.
    #[test]
    fn a_fallen_member_is_named_in_the_removed_colour() {
        let mut alive = mathew();
        alive.status.in_combat = true;
        alive.status.combat_team = 0;
        assert_eq!(name_colour(&[sheet_view(&alive)], 0), CYAN);

        let mut fallen = alive.clone();
        fallen.status.in_combat = false;
        fallen.status.health_status = crate::rest::status::DEAD;
        assert_eq!(name_colour(&[sheet_view(&fallen)], 0), NAME_REMOVED);

        let mut charmed = alive.clone();
        charmed.status.combat_team = 1;
        assert_eq!(name_colour(&[sheet_view(&charmed)], 0), NAME_ENEMY);
    }

    #[test]
    fn out_of_range_indices_show_a_placeholder_not_a_panic() {
        let mut ch = blank_character();
        ch.class_id = 200;
        ch.alignment = 99;
        ch.race = 250;
        let view = sheet_view(&ch);
        assert_eq!(view.class, "?");
        assert_eq!(view.alignment, "?");
        assert!(view.identity.contains('?'));
    }
}
