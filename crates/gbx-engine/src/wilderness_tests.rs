//! Roll-credits slice 7's acceptance suite (`roll-credits.md` §11): the
//! overland loop against real CotAB data.
//!
//! Everything here is `GBX_DATA_DIR`-gated and loud-skips without it (D10 —
//! no game data enters this repo). The synthetic halves of the same slice
//! live with the code they exercise: the presentation arms in
//! `crate::picture_tests`, the terrain generator in
//! `crate::combat::floor::tests`, the cursor table in `crate::mapcursor`.

#![cfg(test)]

use crate::area_transition_tests::{real_data_engine, run_until};
use crate::engine::Engine;
use crate::input::InputEvent;
use crate::shell::{GameState, Shell};

/// Boots the party out of `ECL5#48`'s overland exit (`@0x8086`, the tail of
/// the sequence roll-credits slice 1 pinned) and runs until `ECL1#80`'s own
/// travel menu is parked — "YOU ARE AT THE EDGE OF <city>. WILL YOU ENTER OR
/// CONTINUE YOUR JOURNEY?" over ENTER CITY / JOURNEY ON / CAMP [/ SEARCH
/// AREA].
fn at_the_edge_menu(who: &str) -> Option<Engine> {
    let Some(mut engine) = real_data_engine(5, 48, 50, true) else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (wilderness_tests::{who})");
        return None;
    };
    engine.shell = crate::shell::boot_at_address(&mut engine.machine, 0x8086);
    let parked = run_until_no_input(&mut engine, 4000, |e| menu_line(e).contains("JOURNEY ON"));
    assert!(
        parked,
        "the edge menu never parked; probe={} line={:?} halts={:?}",
        engine.shell().probe(),
        menu_line(&engine),
        engine.vm_memory().halts
    );
    Some(engine)
}

/// [`run_until`] without the Enter feed — the edge menu must be *reached*
/// without anything answering it.
fn run_until_no_input(engine: &mut Engine, max: u32, done: impl Fn(&Engine) -> bool) -> bool {
    for _ in 0..max {
        if done(engine) {
            return true;
        }
        engine.tick(&[]);
    }
    done(engine)
}

/// The parked widget's prompt line, or `""`.
fn menu_line(engine: &Engine) -> String {
    // Upper-cased: `CMD_HorizontalMenu` renders a script's all-caps words in
    // the original's own sentence case ("Enter city"), so the tests compare
    // against the ECL strings rather than the presentation.
    engine
        .shell()
        .parked_widget_for_tests()
        .and_then(|w| w.display_line())
        .unwrap_or_default()
        .to_ascii_uppercase()
}

/// Answers the parked hotbar by pressing the first letter of `word`.
fn press(engine: &mut Engine, word: &str) {
    let key = word.as_bytes()[0];
    engine.tick(&[InputEvent::Char(key)]);
}

/// ★ **The overland has no world menu** (D-S7d). Arriving in `ECL1#80` the
/// shell must never park "Area Cast View Encamp Search Look": outdoors
/// `main_3d_world_menu` returns `'\0'` without drawing anything
/// (`ovr015.cs:354`), and the only prompt the player ever sees is the
/// resident block's own.
#[test]
fn the_overland_parks_the_scripts_menu_and_never_the_world_menu() {
    let Some(engine) = at_the_edge_menu("the_overland_parks_the_scripts_menu...") else {
        return;
    };

    assert_eq!(engine.state().game_state, GameState::WildernessMap);
    assert!(
        !matches!(engine.shell(), Shell::WorldMenu { .. }),
        "the wilderness loop must not open the dungeon world menu"
    );
    let line = menu_line(&engine);
    assert!(
        line.contains("ENTER CITY") && line.contains("JOURNEY ON") && line.contains("CAMP"),
        "the edge menu's own words: {line:?}"
    );
    assert_eq!(
        engine.state().picture.bigpic_block,
        Some(0x79),
        "with the Dalelands map on the viewport"
    );
    assert!(
        engine.vm_memory().halts.is_empty(),
        "halts: {:?}",
        engine.vm_memory().halts
    );
}

/// ★ **D-S7b, live**: the cursor sits on the city the party is standing at.
/// `ECL1#80 @0x809F` copies `[0x4C9B]` into `current_city` (`0x4CA1`), and the
/// arrival branch that ran was `ECL5#48`'s — `@0x803F SAVE #0x09, [0x4C9C]`,
/// city 9 (HAP), whose table entry is `(0x1F, 0x0F)`.
#[test]
fn the_map_cursor_points_at_the_city_the_party_arrived_at() {
    let Some(engine) = at_the_edge_menu("the_map_cursor_points_at_the_city...") else {
        return;
    };

    let city = engine.vm_memory().current_city();
    assert_eq!(city, 9, "ECL5#48's arrival branch names city 9");
    assert_eq!(
        crate::mapcursor::position(city),
        Some((0x1F, 0x0F)),
        "and the table puts the cursor there"
    );
    assert!(
        crate::mapcursor::blinks(
            engine.state().game_state,
            engine.state().picture.bigpic_block,
            engine.state().picture.last_dax_block,
        ),
        "all three of the blink's conditions hold at the edge menu"
    );
}

/// ★ **ENTER CITY crosses back into an area** — the reverse of slice 1's
/// transition, and the door's second acceptance item.
///
/// City 9 (HAP) takes `@0x853C`'s `ON GOTO` to `@0x85B8`: `SAVE 1,[0x4BE6]`
/// (back indoors), `SAVE 5,[0x7F12]` (game_area = 5), `CLEAR BOX`,
/// `NEWECL 0x31` — `ECL5.DAX` block 49.
#[test]
fn enter_city_crosses_back_into_a_real_area() {
    let Some(mut engine) = at_the_edge_menu("enter_city_crosses_back_into_a_real_area") else {
        return;
    };

    press(&mut engine, "ENTER CITY");
    let crossed = run_until(&mut engine, 4000, |e| e.state().game_area == 5);

    assert!(
        crossed,
        "ENTER CITY never crossed; probe={} line={:?} halts={:?}",
        engine.shell().probe(),
        menu_line(&engine),
        engine.vm_memory().halts
    );
    assert_eq!(
        engine.state().ecl_block_id,
        0x31,
        "ECL5 block 49 is resident"
    );
    assert_eq!(
        engine.state().game_state,
        GameState::DungeonMap,
        "SAVE 1,[0x4BE6] put the party back indoors"
    );
    assert_eq!(
        engine.state().game_area_backup,
        1,
        "with the overland pushed to the backup shadow"
    );
}

/// ★ **JOURNEY ON runs its beat** — the door's third acceptance item, and
/// D-S7d's model in one drive.
///
/// From HAP the destination menu is `@0x8E10`'s two-entry VERTICAL MENU
/// ("HAP", "THE STANDING STONE"); the route tables then price the trip, the
/// travel-mode menu multiplies it, `ECL CLOCK` spends it, and `@0x9120`'s
/// `ON GOTO` fires the route's own scripted encounter.
#[test]
fn journey_on_prices_the_route_and_spends_the_clock() {
    let Some(mut engine) = at_the_edge_menu("journey_on_prices_the_route...") else {
        return;
    };
    let day_before = engine.state().clock.raw_clock_words()[4];

    // At the edge menu the route BASE is already `current_city * 4`
    // (`@0x80A6 GOSUB 0x8FBE`); the destination menu's selection is added to
    // it at `@0x8E72`, and `@0x8E7C GETTABLE 0x9C3A,[0x4C9D],[0x4C06]` prices
    // it. HAP is city 9, so the base is 36 and route 36's price is 2.
    assert_eq!(engine.vm_memory().raw_word(0x4C9D), Some(36));

    press(&mut engine, "JOURNEY ON");
    // The destination list, then the travel-mode hotbar, then whatever the
    // route's encounter puts up — Enter answers each in turn.
    let priced = run_until(&mut engine, 6000, |e| {
        e.vm_memory().raw_word(0x4C06).unwrap_or(0) != 0
    });
    assert!(
        priced,
        "the route was never priced; probe={} line={:?} halts={:?}",
        engine.shell().probe(),
        menu_line(&engine),
        engine.vm_memory().halts
    );
    // `@0x8E86 GETTABLE 0x9C72,…,[0x4C08]` is the route KIND — 2 = the
    // "TRAIL WILDERNESS EXIT" menu (`@0x8F3A`), which multiplies the price by
    // 2 for TRAIL and by 4 for WILDERNESS.
    assert_eq!(engine.vm_memory().raw_word(0x4C08), Some(2), "route kind");

    // `@0x8EA7 ECL CLOCK [0x4C06], #0x04` spends the price in DAYS.
    let spent = run_until(&mut engine, 6000, |e| {
        e.state().clock.raw_clock_words()[4] != day_before
    });
    assert!(
        spent,
        "ECL CLOCK never moved the day counter; halts={:?}",
        engine.vm_memory().halts
    );
    // ★ Eight days, not four — and that is `gbl.menuSelectedWord`'s global
    // persistence showing (M6's `eef62b6`): pressing `J` for JOURNEY ON left
    // the highlight on word 1, so the Enter that answered the travel-mode
    // menu took word 1 there too — WILDERNESS (`×4`), not TRAIL (`×2`).
    assert_eq!(engine.vm_memory().raw_word(0x4C9E), Some(1), "WILDERNESS");
    assert_eq!(
        engine.state().clock.raw_clock_words()[4],
        day_before + 8,
        "route 36's two days, quadrupled for going cross-country"
    );
    assert!(
        engine.vm_memory().halts.is_empty(),
        "halts: {:?}",
        engine.vm_memory().halts
    );
}

/// ★ **A wilderness COMBAT on a real WildCom floor** — the door's fourth and
/// last acceptance item, reached by playing the game rather than by poking
/// state.
///
/// The route the previous test priced is the one that carries it. Route 36's
/// encounter id is `2` (`@0x9116 GETTABLE 0x9CAA,[0x4C9D],[0x7F7A]`, table
/// byte `0x9CAA + 36`), and `@0x9120`'s `ON GOTO` sends `2` to `@0x9364`:
/// "SAILING ACROSS THE SKY ARE GREAT BLACK SHAPES… REVEALED AS FEARSOME BLACK
/// DRAGONS", then `SETUP MONSTER 4,2,4` / `CLEARMONSTERS` /
/// `LOAD MONSTER 0x35,3,0x35` / `COMBAT`.
///
/// What makes it a *wilderness* fight and not just a fight: `inDungeon` is 0,
/// so `SetupGroundTiles` takes `SetupWildernessFloor` (terrain from
/// `current_city`, no GEO) and `load_ground_tiles` takes `WILDCOM.DAX`.
#[test]
fn a_journey_encounter_starts_a_fight_on_a_wilderness_floor() {
    let Some(mut engine) = at_the_edge_menu("a_journey_encounter_starts_a_fight...") else {
        return;
    };

    press(&mut engine, "JOURNEY ON");
    // `Stage::Entry` builds the host a tick before `begin_fight` assembles it,
    // so wait for the assembled roster, not merely for a host to exist.
    let fighting = run_until(&mut engine, 20_000, |e| {
        e.shell()
            .combat_host()
            .is_some_and(|h| !h.state().fighters.is_empty())
    });
    assert!(
        fighting,
        "the journey never reached its encounter's COMBAT; probe={} line={:?} halts={:?}",
        engine.shell().probe(),
        menu_line(&engine),
        engine.vm_memory().halts
    );

    assert_eq!(
        engine.state().game_state,
        GameState::WildernessMap,
        "still outdoors — this is not a dungeon fight"
    );
    assert!(
        !engine.vm_memory().in_dungeon(),
        "and the RAW inDungeon cell agrees, which is what the floor fork reads"
    );

    // The floor: `SetField_7(23)` floods it, so no cell is the dungeon's void
    // — and the three scatter passes left something on it.
    let state = engine.shell().combat_host().expect("just checked").state();
    let tiles: Vec<u8> = (0..crate::combat::MAP_W * crate::combat::MAP_H)
        .map(|i| {
            state.map.ground_tile(crate::combat::GridPos::new(
                i % crate::combat::MAP_W,
                i / crate::combat::MAP_W,
            ))
        })
        .collect();
    assert!(
        tiles.iter().all(|&t| t != 0),
        "a wilderness floor has no void cells"
    );
    assert!(
        tiles.iter().any(|&t| t != 23),
        "and the scatter passes put something on it"
    );
    // Both sides are on it: the imported party and `LOAD MONSTER 0x35,3,0x35`.
    let monsters = state
        .fighters
        .iter()
        .filter(|f| f.team == crate::combat::Team::Monster)
        .count();
    let party = state.fighters.len() - monsters;
    assert!(party > 0 && monsters > 0, "{party} vs {monsters}");

    assert!(
        engine.vm_memory().halts.is_empty(),
        "halts: {:?}",
        engine.vm_memory().halts
    );
}

/// ★ `LoadPic`'s `WildernessMap` arm (`ovr025.cs:1443-1448`) on the way OUT of
/// that fight: `RedrawView()` and nothing else, so the Dalelands map comes
/// back whole.
///
/// The combat restore used to run the `DungeonMap` arm unconditionally —
/// `draw8x8_03` painted the 88×88 viewport box and the col-16 panel divider
/// over the full-width map, and nothing re-armed `can_draw_bigpic`, so the
/// overland returned as an empty frame. Found by watching the demo's frames.
#[test]
fn the_overland_map_comes_back_after_a_wilderness_fight() {
    let Some(mut engine) = at_the_edge_menu("the_overland_map_comes_back...") else {
        return;
    };

    press(&mut engine, "JOURNEY ON");
    assert!(
        run_until(&mut engine, 20_000, |e| e.shell().combat_host().is_some()),
        "no fight: {:?}",
        engine.shell().probe()
    );
    // `Q` (Quick) hands each turn to the AI — the bundled save ships
    // `quick_fight = 0`, so the fight parks on the manual menu otherwise.
    let mut back = false;
    for _ in 0..20_000 {
        engine.tick(&[InputEvent::Char(b'Q')]);
        if engine.shell().combat_host().is_none()
            && menu_line(&engine).contains("JOURNEY ON")
            && engine.state().picture.shown == crate::picture::Shown::BigPic
        {
            back = true;
            break;
        }
    }
    assert!(
        back,
        "the overland never came back; probe={} line={:?} shown={:?}",
        engine.shell().probe(),
        menu_line(&engine),
        engine.state().picture.shown
    );
    assert_eq!(engine.state().picture.bigpic_block, Some(0x79));
    assert_eq!(
        engine.vm_memory().current_city(),
        8,
        "and the party is at ESSEMBRA now, so the cursor moved with it"
    );
}
