//! Roll-credits **slice 9c**'s acceptance suite (`roll-credits.md` §13): a
//! whole party created through the real screens, the `.guy` round trips, and
//! the fresh-party `BEGIN` posture.
//!
//! `GBX_DATA_DIR`-gated and loud-skipping without it (D10 — no game data
//! enters this repo). `RESTRIKE_CREATION_DUMP=<dir>` writes one `.ppm` per
//! step of one creation, for eyeballing.
//!
//! ★ **Draw parity, stated once.** Nothing here can move a captured fight's
//! draw stream. Character creation is reachable only from `startGameMenu`,
//! which no capture passes through: every `.gbxtrace` is a combat capture
//! taken from an imported save (`--slot A`, the power-user shortcut that
//! skips the whole preamble, §14.5), and the reel replays those captures
//! through `gbx-oracle::replay` without a shell at all. The PRNG draws
//! creation makes — one `Random(256)` for `mod_id`, the age dice, 36 `3d6`
//! per reroll, a d100 for exceptional strength, and the hit-point rolls —
//! all happen on a code path a capture cannot enter. The guard (16/16
//! operand-exact pins) and the reel smoke (16/16 with live draw equality) are
//! the referees, and both are run green for every commit in this slice.

#![cfg(test)]

use crate::chr_file::{CharFileDirectory, CharFileRequest};
use crate::creation::{self, Picks};
use crate::engine::Engine;
use crate::input::InputEvent;
use crate::party::Character;
use crate::rng::EngineRng;
use crate::shell::Shell;

fn data_dir(who: &str) -> Option<std::path::PathBuf> {
    match std::env::var_os("GBX_DATA_DIR") {
        Some(d) => Some(std::path::PathBuf::from(d)),
        None => {
            eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (creation_tests::{who})");
            None
        }
    }
}

/// A bare front-door engine over real data — no party, no imported save.
fn bare_engine(who: &str) -> Option<Engine> {
    let dir = data_dir(who)?;
    let data = gbx_formats::game_data::load_dir(&dir).expect("GBX_DATA_DIR must be readable");
    Some(Engine::new_front_door(data, 0x9C_9C_9C_9C).expect("the front door must boot"))
}

fn press(engine: &mut Engine, key: u8) {
    engine.tick(&[InputEvent::Char(key)]);
}

fn dump(engine: &mut Engine, name: &str) {
    let frame = engine.tick(&[]);
    let dir = std::env::var_os("RESTRIKE_CREATION_DUMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("creation-{name}.ppm"));
    let mut out = format!(
        "P6\n{} {}\n255\n",
        crate::framebuffer::WIDTH,
        crate::framebuffer::HEIGHT
    )
    .into_bytes();
    for y in 0..crate::framebuffer::HEIGHT {
        for x in 0..crate::framebuffer::WIDTH {
            let idx = frame.pixels[y * crate::framebuffer::WIDTH + x];
            out.extend_from_slice(&frame.palette[idx as usize]);
        }
    }
    let _ = std::fs::create_dir_all(&dir);
    if std::fs::write(&path, &out).is_ok() {
        eprintln!("creation '{name}': dumped {}", path.display());
    }
}

/// Walks one creation from the start menu to the `.guy` write, pressing what a
/// player presses. `rows` are the 1-based picker rows for race/sex/class/
/// alignment; `dumps` writes a frame at every step.
///
/// The key-count arithmetic is the original's own cursor behaviour: the race
/// picker opens with the cursor on the **last** row (`index = 0`), the other
/// three on the **first** (`index = 1`), and End (`menu_scroll_in_page`) wraps
/// within the page — so from the last row one End lands on row 1.
fn create_one(engine: &mut Engine, name: &str, rows: [usize; 4], dumps: bool) {
    // `startGameMenu`'s `'C'` (`ovr018.cs:161-166`).
    press(engine, b'C');
    assert_eq!(engine.probe(), "screen", "Create opened a screen");
    if dumps {
        dump(engine, "1-pick-race");
    }
    for (step, row) in rows.iter().enumerate() {
        let presses = if step == 0 { *row } else { *row - 1 };
        for _ in 0..presses {
            engine.tick(&[InputEvent::Ext(crate::input::ExtKey::End)]);
        }
        // Only `'S'` commits (`while (input_key != 'S')`, `ovr018.cs:376`).
        press(engine, b'S');
        if dumps && step < 3 {
            dump(
                engine,
                &format!(
                    "{}-pick-{}",
                    step + 2,
                    ["gender", "class", "alignment"][step]
                ),
            );
        }
    }
    if dumps {
        dump(engine, "5-rolled-sheet");
    }
    // `yes_no("Reroll stats? ")` — 'N' accepts the roll.
    press(engine, b'N');
    if dumps {
        dump(engine, "6-name-prompt");
    }
    for byte in name.bytes() {
        engine.tick(&[InputEvent::Char(byte)]);
    }
    engine.tick(&[InputEvent::Enter]);
    if dumps {
        dump(engine, "7-icon-editor");
    }
    // The icon editor: `Exit` at the top menu, then `Is this icon ok? Y`.
    press(engine, b'E');
    press(engine, b'Y');
    if dumps {
        dump(engine, "8-save-prompt");
    }
    // `yes_no("Save <name>? ")`.
    press(engine, b'Y');
}

/// ★ **A full six-character party, created end to end through the screens.**
///
/// Every step is a keypress a player makes: the four pickers (only `'S'`
/// commits), the reroll confirmation, the name, the icon editor, the save
/// prompt. One creation is frame-dumped at every step.
///
/// The party is assembled the way the original assembles one — creation
/// writes a `.guy`, and `Add Character to Party` brings it in
/// (`createPlayer` has no `TeamList.Add`; see [`crate::creation`]).
#[test]
fn a_full_six_character_party_is_created_through_the_screens() {
    let Some(mut engine) = bare_engine("a_full_six_character_party...") else {
        return;
    };
    let temp = std::env::temp_dir().join("restrike-9c-party");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    engine.park_at_start_menu();
    engine.tick(&[]);
    assert_eq!(engine.probe(), "front-door/start-menu");

    // Six characters, deliberately spread across the pickers. Rows are
    // 1-based into each list; the expected (race, class_id) is asserted after
    // the party is assembled, so a navigation slip cannot pass unnoticed.
    // Race rows: 1 Dwarf, 2 Elf, 3 Gnome, 4 Half-Elf, 5 Halfling, 6 Human.
    let roster: [(&str, [usize; 4], u8, u8); 6] = [
        // human, male, paladin (RaceClasses[7] row 5), Lawful Good
        ("MATHEW", [6, 1, 5, 1], 7, 3),
        // dwarf, male, fighter (RaceClasses[1] row 1)
        ("BRUENOR", [1, 1, 1, 1], 1, 2),
        // elf, female, magic-user (RaceClasses[2] row 2)
        ("LEDERA", [2, 2, 2, 1], 2, 5),
        // half-elf, female, cleric (RaceClasses[4] row 1)
        ("SHARA", [4, 2, 1, 1], 4, 0),
        // halfling, male, thief (RaceClasses[5] row 2)
        ("REGIS", [5, 1, 2, 1], 5, 6),
        // gnome, male, fighter/thief (RaceClasses[3] row 3)
        ("STEVE", [3, 1, 3, 1], 3, 14),
    ];
    for (i, (name, rows, _, _)) in roster.iter().enumerate() {
        create_one(&mut engine, name, *rows, i == 0);
        // The save prompt's `SavePlayer` reaches the host.
        let request = engine
            .take_char_file_request()
            .unwrap_or_else(|| panic!("{name}: no .guy write was requested"));
        assert!(matches!(request, CharFileRequest::Save(_)));
        crate::saveload_fs::fulfill_char_file(&mut engine, request, &temp)
            .expect("the .guy write must succeed");
        engine.tick(&[]);
        assert_eq!(
            engine.probe(),
            "front-door/start-menu",
            "{name}: creation returned to the menu"
        );
    }

    // Six `.guy` files on disk, each the 0x1A6-byte record.
    let mut guys: Vec<String> = std::fs::read_dir(&temp)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".guy"))
        .collect();
    guys.sort();
    assert_eq!(guys.len(), 6, "six characters saved: {guys:?}");
    for g in &guys {
        let bytes = std::fs::read(temp.join(g)).unwrap();
        assert_eq!(bytes.len(), gbx_formats::save_orig::CHAR_RECORD_SIZE, "{g}");
    }

    // ...and `Add Character to Party` brings every one of them in.
    engine.set_char_file_directory(crate::saveload_fs::scan_char_files(&temp));
    assert_eq!(engine.char_file_directory().entries.len(), 6);
    for _ in 0..6 {
        add_first_available(&mut engine, &temp);
    }
    assert_eq!(engine.party().members.len(), 6, "a six-strong party");
    // ★ Every picker landed where it was aimed: the race and class the rows
    // name, on the character the name prompt named.
    for (name, _, race, class_id) in roster {
        let m = engine
            .party()
            .members
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} joined the party"));
        assert_eq!(m.race, race, "{name}'s race");
        assert_eq!(m.class_id, class_id, "{name}'s class");
    }
    // Every member is a real, trained character, not a blank record.
    for m in &engine.party().members {
        assert!(m.hit_point_max >= 1, "{} has hit points", m.name);
        assert!(m.class_level.iter().any(|&l| l > 0));
        assert_eq!(m.money.platinum, 300);
        assert!(m.hit_dice >= 1);
    }
    dump(&mut engine, "9-party-assembled");
    let _ = std::fs::remove_dir_all(&temp);
}

/// Presses `A`, picks Curse, takes the first untaken row, and fulfills the
/// load — the `AddPlayer` loop, one character.
fn add_first_available(engine: &mut Engine, dir: &std::path::Path) {
    engine.set_char_file_directory(
        crate::saveload_fs::scan_char_files(dir).without_party_members(engine.party()),
    );
    press(engine, b'A');
    assert_eq!(engine.probe(), "screen", "Add opened its screen");
    press(engine, b'C'); // `"Curse Pool Hillsfar Exit"`
    press(engine, b'A'); // `input_key == 'A'` commits a row (`ovr018.cs:1474`)
    let request = engine
        .take_char_file_request()
        .expect("Add asked the host to load a .guy");
    let refusal =
        crate::saveload_fs::fulfill_char_file(engine, request, dir).expect("the load must succeed");
    assert!(refusal.is_none(), "unexpected refusal: {refusal:?}");
    // Leave the picker.
    press(engine, b'E');
    engine.tick(&[]);
}

/// ★ **The `.guy` round trip**: a member removed writes a file, and adding it
/// back produces the identical record.
///
/// This is `startGameMenu`'s `'R'` arm (`SavePlayer` then `FreeCurrentPlayer`,
/// `ovr018.cs:207-221`) followed by `AddPlayer` — the two halves of §14.8's
/// residual, now closed.
#[test]
fn remove_writes_a_guy_and_add_reads_the_same_record_back() {
    let Some(mut engine) = bare_engine("remove_writes_a_guy...") else {
        return;
    };
    let temp = std::env::temp_dir().join("restrike-9c-roundtrip");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // Build one character headlessly and put them in the party.
    let mut rng = EngineRng::new(0x0C0F_FEE0);
    let mut ch = creation::create(
        engine.rules(),
        &mut rng,
        Picks {
            race: 7,
            sex: 0,
            class_id: 6,
            alignment: 7,
        },
    );
    creation::finish(&mut ch, "ROUNDTRIP");
    // Give them an item and an affect, so the `.swg`/`.fx` siblings are
    // exercised too.
    let mut item = vec![7u8; gbx_formats::save_orig::ITEM_RECORD_SIZE];
    gbx_formats::save_orig::set_item_readied(&mut item, true);
    ch.items.push(item);
    ch.readied_items.insert(0);
    ch.affects.push(vec![0x27, 0, 0, 0xFF, 0, 0, 0, 0, 0]); // a haste affect
    engine
        .add_character(ch.clone())
        .expect("joins an empty party");
    let before = engine.party().members[0].clone();

    engine.park_at_start_menu();
    engine.tick(&[]);
    // `'R'` — Remove.
    press(&mut engine, b'R');
    assert!(engine.party().members.is_empty(), "the member left");
    let request = engine
        .take_char_file_request()
        .expect("Remove asked the host to write the .guy");
    crate::saveload_fs::fulfill_char_file(&mut engine, request, &temp).unwrap();
    // `clean_string` lowercases and cuts at 8 — "ROUNDTRIP" is nine.
    for ext in ["guy", "swg", "fx"] {
        assert!(
            temp.join(format!("roundtri.{ext}")).is_file(),
            "the .{ext} sibling was written"
        );
    }

    // ...and Add reads it back.
    engine.set_char_file_directory(crate::saveload_fs::scan_char_files(&temp));
    add_first_available(&mut engine, &temp);
    let after = &engine.party().members[0];

    assert_eq!(after.name, before.name);
    assert_eq!(after.stats, before.stats);
    assert_eq!(after.class_level, before.class_level);
    assert_eq!(after.skills, before.skills);
    assert_eq!(after.magic, before.magic);
    assert_eq!(after.money, before.money);
    assert_eq!(after.combat, before.combat);
    assert_eq!(after.hit_point_max, before.hit_point_max);
    assert_eq!(after.hit_point_rolled, before.hit_point_rolled);
    assert_eq!(after.age, before.age);
    assert_eq!(after.icon, before.icon);
    assert_eq!(after.items, before.items, "the .swg survived");
    assert_eq!(after.affects, before.affects, "the .fx survived");
    assert_eq!(
        after.readied_items, before.readied_items,
        "readiness is rebuilt from the item flags, not from a stored pointer"
    );
    let _ = std::fs::remove_dir_all(&temp);
}

/// ★ **Save → Load → identical**, with a created party in the roster.
#[test]
fn a_created_party_survives_a_save_and_load() {
    let Some(mut engine) = bare_engine("a_created_party_survives...") else {
        return;
    };
    let dir = data_dir("a_created_party_survives...").unwrap();
    let temp = std::env::temp_dir().join("restrike-9c-saveload");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    let mut rng = EngineRng::new(0x5EED_5EED);
    for (i, class_id) in [0u8, 2, 5, 6].into_iter().enumerate() {
        let mut ch = creation::create(
            engine.rules(),
            &mut rng,
            Picks {
                race: 7,
                sex: (i % 2) as u8,
                class_id,
                alignment: creation::alignment_choices(engine.rules(), class_id)[0],
            },
        );
        creation::finish(&mut ch, &format!("HERO{i}"));
        engine.add_character(ch).expect("joins");
    }
    let before: Vec<Character> = engine.party().members.clone();

    crate::saveload_fs::save_to_slot(&engine, &temp, 'A').expect("save");
    let data = gbx_formats::game_data::load_dir(&dir).expect("data");
    let restored = crate::saveload_fs::load_from_slot(&temp, 'A', data).expect("load");

    assert_eq!(
        restored.party().members,
        before,
        "the party is byte-identical"
    );
    let _ = std::fs::remove_dir_all(&temp);
}

/// ★ **The fresh-party `BEGIN` posture** (`sub_29758`'s `LastEclBlockId == 0`
/// arm, `ovr003.cs:2243-2261`).
///
/// A virgin game has `area_ptr.LastEclBlockId == 0`, and that arm reads it as
/// "use the area's boot block": `EclBlockId = 1`, `PartySummary` paints the
/// roster, and `byte_1EE98 = false` suppresses the `LoadPic` a continuing game
/// would take. `BEGIN`'s own paint agrees from the other side — with
/// `LastEclBlockId == 0` it draws the exploration frame and nothing else
/// (`ovr018.cs:262-266`), because `sub_29758` is about to paint the roster
/// itself.
///
/// So a freshly created party lands at **area 2, block 1** — the Tilverton
/// intro — and NOT wherever an imported save left off. Digest-pinned so the
/// posture cannot silently drift.
#[test]
fn begin_with_a_fresh_party_lands_in_the_fresh_game_posture() {
    let Some(mut engine) = bare_engine("begin_with_a_fresh_party...") else {
        return;
    };
    assert_eq!(
        engine.state().last_ecl_block_id,
        0,
        "a virgin game's LastEclBlockId is the 0 sentinel"
    );

    let mut rng = EngineRng::new(0xFEED_FACE);
    let mut ch = creation::create(
        engine.rules(),
        &mut rng,
        Picks {
            race: 7,
            sex: 0,
            class_id: 2,
            alignment: 4,
        },
    );
    creation::finish(&mut ch, "PIONEER");
    engine.add_character(ch).expect("joins");

    engine.park_at_start_menu();
    engine.tick(&[]);
    dump(&mut engine, "10-fresh-start-menu");

    // `BEGIN Adventuring`.
    press(&mut engine, b'B');
    assert_ne!(
        engine.probe(),
        "front-door/start-menu",
        "BEGIN left the menu"
    );
    assert_eq!(
        engine.state().ecl_block_id,
        1,
        "the fresh game enters block 1, not a continuation block"
    );

    // Run to the intro's first parked interaction.
    let parked = |e: &Engine| e.shell().gate_open() && !matches!(e.shell(), Shell::FrontDoor(_));
    let mut ticks = 0;
    while ticks < 4000 && !parked(&engine) {
        engine.tick(&[]);
        ticks += 1;
    }
    assert!(parked(&engine), "the fresh game reached its first prompt");
    dump(&mut engine, "11-fresh-begin");

    // The posture, pinned. A digest change here means the fresh-game entry
    // moved — which is exactly the drift this test exists to catch.
    assert_eq!(engine.state().game_area, 2, "Tilverton");
    assert_eq!(engine.state().ecl_block_id, 1);
    assert_eq!(engine.party().members.len(), 1);
    let digest = engine.state_digest();

    // The same engine built the same way must land on the same digest —
    // creation is deterministic given the seed, and BEGIN adds nothing.
    let Some(mut twin) = bare_engine("begin_with_a_fresh_party... (twin)") else {
        return;
    };
    let mut rng = EngineRng::new(0xFEED_FACE);
    let mut ch = creation::create(
        twin.rules(),
        &mut rng,
        Picks {
            race: 7,
            sex: 0,
            class_id: 2,
            alignment: 4,
        },
    );
    creation::finish(&mut ch, "PIONEER");
    twin.add_character(ch).expect("joins");
    twin.park_at_start_menu();
    twin.tick(&[]);
    press(&mut twin, b'B');
    let mut ticks = 0;
    while ticks < 4000 && !parked(&twin) {
        twin.tick(&[]);
        ticks += 1;
    }
    assert_eq!(
        twin.state_digest(),
        digest,
        "the fresh-game posture is deterministic"
    );
}

/// ★ **`PROGRAM 0` opens the start menu mid-game** (`ovr003.cs:1941-1948`) —
/// §14.8's last residual.
///
/// Driven through the state the opcode handler raises, because no shipped
/// script reaches the opcode (the one `PROGRAM` use is `ECL1#82`'s, inside
/// the deferred attract mode). `BEGIN` here resumes the walk loop rather than
/// re-entering the block, which is what `startGameMenu` returning into
/// `sub_29758`'s own loop does.
#[test]
fn program_zero_opens_the_start_menu_and_begin_resumes_the_walk_loop() {
    let Some(mut engine) = bare_engine("program_zero_opens...") else {
        return;
    };
    let mut rng = EngineRng::new(4);
    let mut ch = creation::create(
        engine.rules(),
        &mut rng,
        Picks {
            race: 7,
            sex: 0,
            class_id: 2,
            alignment: 4,
        },
    );
    creation::finish(&mut ch, "WALKER");
    engine.add_character(ch).expect("joins");

    // Get into the walk loop the ordinary way.
    engine.park_at_start_menu();
    engine.tick(&[]);
    press(&mut engine, b'B');
    let mut ticks = 0;
    while ticks < 4000 && engine.probe() != "world-menu" {
        // Acknowledge whatever the intro puts up — but only while something
        // is actually asking, so no keystroke is left in the buffer for the
        // walk loop to resolve itself with.
        if engine.shell().gate_open() {
            engine.tick(&[InputEvent::Enter]);
        } else {
            engine.tick(&[]);
        }
        ticks += 1;
    }
    assert_eq!(engine.probe(), "world-menu", "reached the walk loop");
    let before = engine.state_digest();

    // The opcode's request.
    engine.raise_program_start_menu();
    engine.tick(&[]);
    assert_eq!(
        engine.probe(),
        "front-door/start-menu",
        "PROGRAM 0 opened startGameMenu"
    );
    dump(&mut engine, "12-program-0-menu");

    // BEGIN returns to the walk loop, not into a fresh block entry.
    press(&mut engine, b'B');
    engine.tick(&[]);
    assert_eq!(engine.probe(), "world-menu", "the walk loop resumed");
    assert_eq!(
        engine.state_digest(),
        before,
        "a menu that was only looked at changes nothing"
    );
}

/// The character-file directory hides members already in the party — the
/// original's own filter, and what stops `Add` offering a duplicate.
#[test]
fn the_add_picker_hides_characters_already_in_the_party() {
    let Some(mut engine) = bare_engine("the_add_picker_hides...") else {
        return;
    };
    let temp = std::env::temp_dir().join("restrike-9c-dupes");
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    let mut rng = EngineRng::new(77);
    let mut ch = creation::create(
        engine.rules(),
        &mut rng,
        Picks {
            race: 7,
            sex: 0,
            class_id: 2,
            alignment: 4,
        },
    );
    creation::finish(&mut ch, "SOLO");
    crate::saveload_fs::fulfill_char_file(
        &mut engine,
        CharFileRequest::Save(Box::new(ch.clone())),
        &temp,
    )
    .unwrap();

    let all = crate::saveload_fs::scan_char_files(&temp);
    assert_eq!(all.entries.len(), 1);
    assert_eq!(all.entries[0].name, "SOLO");

    engine.add_character(ch).expect("joins");
    let filtered = crate::saveload_fs::scan_char_files(&temp).without_party_members(engine.party());
    assert!(
        filtered.entries.is_empty(),
        "a character in the party is not offered again"
    );
    let _ = std::fs::remove_dir_all(&temp);
    let _: CharFileDirectory = filtered;
}
