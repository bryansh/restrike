//! Roll-credits slice 9b's acceptance suite (`roll-credits.md` §13): the whole
//! front door driven headlessly against real CotAB data, with frame dumps.
//!
//! Everything here is `GBX_DATA_DIR`-gated and loud-skips without it (D10 — no
//! game data enters this repo). Set `RESTRIKE_FRONT_DOOR_DUMP=<dir>` to write
//! one `.ppm` per beat: the title screens, the Play-Demo prompt, the
//! copy-protection challenge with its visible answer, and the start menu with
//! the party listed.
//!
//! The synthetic halves live with the code they exercise — the wheel
//! arithmetic in [`crate::copy_wheel`], the menu flag table and the title
//! timings in [`crate::front_door`].

#![cfg(test)]

use crate::engine::Engine;
use crate::input::InputEvent;
use crate::shell::Shell;

/// Boots the front door over real data (the same call the desktop's default
/// launch makes).
fn front_door(who: &str) -> Option<Engine> {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (front_door_tests::{who})");
        return None;
    };
    let data = gbx_formats::game_data::load_dir(std::path::Path::new(&dir))
        .expect("GBX_DATA_DIR must be readable");
    Some(Engine::new_front_door(data, 0x5A1E_5A1E).expect("the front door must boot"))
}

/// One frame to `RESTRIKE_FRONT_DOOR_DUMP` (or the temp dir), for eyeballing.
fn dump(engine: &mut Engine, name: &str) {
    let frame = engine.tick(&[]);
    let dir = std::env::var_os("RESTRIKE_FRONT_DOOR_DUMP")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("front-door-{name}.ppm"));
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
        eprintln!("front door '{name}': dumped {}", path.display());
    }
}

/// Ticks until `done`, up to `max` ticks. Returns whether it happened.
fn run_until(engine: &mut Engine, max: u32, done: impl Fn(&Engine) -> bool) -> bool {
    for _ in 0..max {
        if done(engine) {
            return true;
        }
        engine.tick(&[]);
    }
    done(engine)
}

/// Presses one key.
fn press(engine: &mut Engine, key: u8) {
    engine.tick(&[InputEvent::Char(key)]);
}

/// Skips the whole title sequence with keypresses.
///
/// Not simply four presses: `delay_or_key` opens with `clear_keyboard()`
/// (`ovr002.cs:10`), so a key that arrives on the very tick a beat paints is
/// discarded — faithfully, and exactly as in the original, where the buffer is
/// flushed after the picture goes up. Pressing until the stage changes is what
/// a player does anyway.
fn skip_title(engine: &mut Engine) {
    for _ in 0..40 {
        if engine.probe() != "front-door/title" {
            return;
        }
        press(engine, b' ');
    }
    panic!("the title sequence never ended; probe={}", engine.probe());
}

/// ★ **The boot sequence**: title → Play-Demo prompt → copy protection →
/// start menu, each beat reached by the key the original takes, with a frame
/// dumped at every one.
#[test]
fn the_front_door_runs_title_prompt_protection_and_start_menu() {
    let Some(mut engine) = front_door("the_front_door_runs_...") else {
        return;
    };

    // Beat 1: the SSI / AD&D product screen.
    engine.tick(&[]);
    assert_eq!(engine.probe(), "front-door/title");
    dump(&mut engine, "1-title-logo");

    // A key skips the beat (`delay_or_key`, `ovr002.cs:8-21`) — four of them
    // walk the whole sequence: logo, cover+logo, banner, credits.
    press(&mut engine, b' ');
    dump(&mut engine, "2-title-cover");
    press(&mut engine, b' ');
    dump(&mut engine, "3-title-banner");
    press(&mut engine, b' ');
    dump(&mut engine, "4-title-credits");
    assert_eq!(
        engine.probe(),
        "front-door/title",
        "still in the sequence at the credits"
    );

    // Beat 2: the Play-Demo prompt.
    press(&mut engine, b' ');
    assert_eq!(engine.probe(), "front-door/play-demo");
    dump(&mut engine, "5-play-demo-prompt");

    // Anything but 'D' means "play the game" (`seg001.cs:122-125`).
    press(&mut engine, b'P');
    assert_eq!(engine.probe(), "front-door/copy-protection");
    dump(&mut engine, "6-copy-protection");

    // Beat 3: the answer is pre-filled (D-RC4), so <Enter> passes.
    engine.tick(&[InputEvent::Enter]);
    assert_eq!(engine.probe(), "front-door/start-menu");
    dump(&mut engine, "7-start-menu-partyless");

    // A partyless start menu (this boot imported nobody) offers Load, and
    // BEGIN does nothing.
    press(&mut engine, b'B');
    assert_eq!(
        engine.probe(),
        "front-door/start-menu",
        "BEGIN is not on a partyless menu"
    );
}

/// The copy-protection prompt really is answerable: the shown answer is the
/// wheel's, and typing it (rather than accepting the prefill) passes too.
#[test]
fn the_shown_answer_is_the_wheel_answer() {
    let Some(mut engine) = front_door("the_shown_answer_is...") else {
        return;
    };
    skip_title(&mut engine);
    press(&mut engine, b'P');
    let Shell::FrontDoor(door) = engine.shell() else {
        panic!("the front door must be parked; probe={}", engine.probe());
    };
    let crate::front_door::FrontDoor::Protection(prot) = door.as_ref() else {
        panic!("copy protection must be up; probe={}", engine.probe());
    };
    let answer = prot.challenge().answer();
    assert!(answer.is_ascii_alphanumeric());

    // Backspace clears the prefill; typing the same character passes.
    engine.tick(&[InputEvent::Backspace]);
    engine.tick(&[InputEvent::Char(answer as u8)]);
    engine.tick(&[InputEvent::Enter]);
    assert_eq!(engine.probe(), "front-door/start-menu");
}

/// A wrong answer re-rolls the challenge and says so; three of them eject the
/// session (`ovr004.cs:92-110`).
#[test]
fn three_wrong_answers_eject_the_session() {
    let Some(mut engine) = front_door("three_wrong_answers...") else {
        return;
    };
    // Before the challenge is posed: the prefill is baked into the input
    // line at construction (D-RC4), so flipping it afterwards would leave a
    // correct answer already typed.
    engine.set_copy_protection_faithful(true);
    skip_title(&mut engine);
    press(&mut engine, b'P');

    let mut seen = Vec::new();
    for attempt in 0..3 {
        let Shell::FrontDoor(door) = engine.shell() else {
            panic!("front door expected");
        };
        let crate::front_door::FrontDoor::Protection(prot) = door.as_ref() else {
            panic!("copy protection expected at attempt {attempt}");
        };
        let answer = prot.challenge().answer();
        seen.push(prot.challenge());
        // Any character that is not the answer.
        let wrong = if answer == 'A' { b'B' } else { b'A' };
        engine.tick(&[InputEvent::Char(wrong)]);
        engine.tick(&[InputEvent::Enter]);
    }
    assert!(
        seen[0] != seen[1] || seen[1] != seen[2],
        "each miss re-rolls the challenge (`ovr004.cs:32-71`)"
    );
    // `SysDelay(0x3E8)` then `print_and_exit()`.
    assert!(run_until(&mut engine, 120, |e| e.quit_requested()));
}

/// Choosing Demo (or letting the 30-second prompt time out into it) lands on
/// a loud deferral and returns to the title — the attract mode stays out of
/// scope (roll-credits G10).
#[test]
fn the_demo_is_a_loud_stub_that_returns_to_the_title() {
    let Some(mut engine) = front_door("the_demo_is_a_loud_stub...") else {
        return;
    };
    skip_title(&mut engine);
    assert_eq!(engine.probe(), "front-door/play-demo");
    press(&mut engine, b'D');
    assert_eq!(engine.probe(), "front-door/demo-deferred");
    dump(&mut engine, "8-demo-deferred"); // the paint tick, which is loud
    let notes = engine.take_transcript();
    assert!(
        notes
            .iter()
            .any(|n| format!("{n:?}").contains("attract mode")),
        "the deferral is in the transcript: {notes:?}"
    );
    press(&mut engine, b' ');
    assert_eq!(engine.probe(), "front-door/title");
}

/// The prompt's 30-second timeout defaults to `'D'` (`seg001.cs:114-115`) —
/// an unattended title screen plays the demo, which for us is the stub.
#[test]
fn the_play_demo_prompt_times_out_into_the_demo() {
    let Some(mut engine) = front_door("the_play_demo_prompt_times_out...") else {
        return;
    };
    skip_title(&mut engine);
    assert_eq!(engine.probe(), "front-door/play-demo");
    // 10 seconds at 60 Hz, plus slack: the prompt reached from the title
    // sequence is the post-demo one (`seg001.cs:159`).
    assert!(
        run_until(&mut engine, 11 * 60, |e| e.probe()
            == "front-door/demo-deferred"),
        "the prompt timed out into 'D'; probe={}",
        engine.probe()
    );
}

/// ★ **The digest-compare**: BEGIN from the start menu lands in exactly the
/// state today's `--slot A` boot does.
///
/// The front door changed presentation, not state — the whole point of
/// routing BEGIN through `BootFlow::start`, the same entry `import_original`
/// has always used. Both engines are ticked the same number of times past the
/// intro's first parked menu and their H5 state digests (D-RC3) compared.
#[test]
fn begin_lands_where_the_slot_shortcut_lands() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (front_door_tests::begin_lands...)");
        return;
    };
    let dir = std::path::Path::new(&dir);
    let Some(direct) = imported(dir) else {
        eprintln!("SKIPPED: no importable slot A under GBX_DATA_DIR/SAVE");
        return;
    };
    let Some(mut through_door) = imported(dir) else {
        return;
    };
    // The same engine, parked on the start menu instead of the walk loop —
    // which is exactly what a load asked for by the START MENU produces
    // (`saveload_fs::fulfill` carries the bit across the replacement).
    through_door.park_at_start_menu();
    assert_eq!(through_door.probe(), "front-door/start-menu");

    // BEGIN.
    let mut direct = direct;
    press(&mut through_door, b'B');
    assert_ne!(
        through_door.probe(),
        "front-door/start-menu",
        "BEGIN left the menu"
    );

    // Both run to the intro's first parked interaction.
    let parked = |e: &Engine| e.shell().gate_open() && !matches!(e.shell(), Shell::FrontDoor(_));
    assert!(run_until(&mut direct, 4000, parked), "direct boot parked");
    assert!(
        run_until(&mut through_door, 4000, parked),
        "front-door boot parked"
    );

    assert_eq!(
        through_door.state_digest(),
        direct.state_digest(),
        "the front door changed presentation, not state\n  through-door probe={}\n  direct probe={}",
        through_door.probe(),
        direct.probe()
    );
    dump(&mut through_door, "9-begin-tilverton-intro");
}

/// The bundled GOG slot A, imported the way the desktop's `--slot` shortcut
/// does — `None` when the data set has no `SAVE/SAVGAMA.DAT`.
fn imported(dir: &std::path::Path) -> Option<Engine> {
    let data = gbx_formats::game_data::load_dir(dir).ok()?;
    let saves = gbx_formats::game_data::load_dir(&dir.join("SAVE")).ok()?;
    let master = saves.raw_file("SAVGAMA.DAT")?;
    let set = gbx_formats::save_orig::load_from_lookup(master, 'A', |n| saves.raw_file(n)).ok()?;
    crate::import::import_original(&set, data, 0x5A1E_5A1E).ok()
}

/// ★ The wipe recovery now returns to the START MENU, party-less — the
/// original's `InitAgain()` → `startGameMenu()` path (`seg001.cs:147-152`),
/// whose flag table then offers Load and nothing else that leads anywhere.
#[test]
fn a_party_wipe_recovers_through_the_start_menu() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (front_door_tests::a_party_wipe...)");
        return;
    };
    let Some(mut engine) = imported(std::path::Path::new(&dir)) else {
        eprintln!("SKIPPED: no importable slot A under GBX_DATA_DIR/SAVE");
        return;
    };
    for _ in 0..20 {
        engine.tick(&[]);
    }
    assert!(!engine.party().members.is_empty(), "a party to lose");

    engine.state.party_killed = true;
    engine.tick(&[]);
    assert!(matches!(engine.shell(), Shell::GameOver(_)));
    assert!(run_until(&mut engine, 900, |e| e.probe() == "game-over/press-any-key"));
    press(&mut engine, b' ');

    assert_eq!(engine.probe(), "front-door/start-menu");
    assert!(
        engine.party().members.is_empty(),
        "`InitAgain` clears TeamList (`seg001.cs:364`)"
    );
    dump(&mut engine, "10-wipe-recovery-start-menu");

    // The only verb that leads anywhere is Load.
    press(&mut engine, b'B');
    assert_eq!(engine.probe(), "front-door/start-menu", "no BEGIN");
    press(&mut engine, b'S');
    assert_eq!(engine.probe(), "front-door/start-menu", "no Save");
    press(&mut engine, b'L');
    assert_eq!(engine.probe(), "screen", "Load opens the slot list");
}

/// The start menu with a party: the roster is on the panel, the flag table
/// shows the party column, and View/Save/Begin all reach their screens.
#[test]
fn the_start_menu_with_a_party_offers_the_party_column() {
    let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (front_door_tests::the_start_menu...)");
        return;
    };
    let Some(mut engine) = imported(std::path::Path::new(&dir)) else {
        eprintln!("SKIPPED: no importable slot A under GBX_DATA_DIR/SAVE");
        return;
    };
    engine.park_at_start_menu();
    engine.tick(&[]);
    dump(&mut engine, "11-start-menu-with-party");

    // View opens the character sheet and Exit comes back here.
    press(&mut engine, b'V');
    assert_eq!(engine.probe(), "screen");
    press(&mut engine, b'E');
    engine.tick(&[]);
    assert_eq!(engine.probe(), "front-door/start-menu");

    // Save opens the ten-letter slot picker directly (no chooser).
    press(&mut engine, b'S');
    assert_eq!(engine.probe(), "screen");
    press(&mut engine, b'A');
    assert_eq!(
        engine.take_io_request(),
        Some(crate::saveload::SaveLoadRequest::Save('A')),
        "the save request reaches the host"
    );

    // Create/Modify present but report the slice that owns them.
    engine.park_at_start_menu();
    engine.tick(&[]);
    press(&mut engine, b'C');
    assert_eq!(engine.probe(), "front-door/start-menu");
}

/// `Exit to DOS` asks the host to quit (`print_and_exit`, `ovr018.cs:296`).
#[test]
fn exit_to_dos_raises_the_quit_request() {
    let Some(mut engine) = front_door("exit_to_dos...") else {
        return;
    };
    skip_title(&mut engine);
    press(&mut engine, b'P');
    engine.tick(&[InputEvent::Enter]);
    assert_eq!(engine.probe(), "front-door/start-menu");
    assert!(!engine.quit_requested());
    press(&mut engine, b'E');
    assert!(engine.quit_requested(), "the host is asked to quit");
}
