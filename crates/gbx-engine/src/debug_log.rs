//! The `RESTRIKE_DEBUG_LOG` forensics pipeline, replay half — promoted from
//! `examples/replay_debug_log.rs` to shipped machinery so the example and
//! `restrike replay` (roll-credits D-RC3) are one implementation, not two.
//!
//! **Recording is unchanged**: the desktop writes the log
//! (`frontends/desktop/src/main.rs`), one `tick N | sent [...] | probe` line
//! per interesting tick, and this module only ever *reads* that format. The
//! log format itself is deliberately not redefined here — every debug log ever
//! captured stays replayable.
//!
//! What lives here is the part a replay has to get right to be the same run:
//! the input schedule, the **boot posture**, and the **host's D8
//! obligations**, which since roll-credits slice 0 include fulfilling the
//! save/load screen's requests. A replay that skipped those would diverge the
//! instant a session saved.
//!
//! ★ **The boot posture tracks the desktop's default, whatever that is**
//! (roll-credits slice 9b). It moved once already — from the front door to an
//! imported slot A when slice 0 gave the desktop a party by default, and back
//! to the front door when slice 9b gave it a title screen — and each move is a
//! silent tick-0 divergence for any log recorded without flags. Two things
//! keep that honest: [`Boot::default`] is defined as "whatever
//! `restrike-desktop` does with no arguments", and [`Session::probes`] lets a
//! replay *check* rather than assume, by comparing the log's own probe line
//! against the engine's at the same tick ([`Session::probe_at`]).

use std::path::{Path, PathBuf};

use crate::engine::Engine;
use crate::input::{ExtKey, InputEvent};
use crate::saveload::SaveLoadRequest;
use gbx_formats::game_data::load_dir;

/// One `InputEvent` as the desktop's `{:?}` wrote it. `Char` is accepted in
/// both spellings (`Char(98)`, which is what `Debug` actually prints for a
/// `u8`, and `Char('b')` for hand-written logs).
pub fn parse_event(s: &str) -> Option<InputEvent> {
    let s = s.trim();
    Some(match s {
        "Enter" => InputEvent::Enter,
        "Escape" => InputEvent::Escape,
        "Backspace" => InputEvent::Backspace,
        "Ext(Up)" => InputEvent::Ext(ExtKey::Up),
        "Ext(Down)" => InputEvent::Ext(ExtKey::Down),
        "Ext(Left)" => InputEvent::Ext(ExtKey::Left),
        "Ext(Right)" => InputEvent::Ext(ExtKey::Right),
        "Ext(Home)" => InputEvent::Ext(ExtKey::Home),
        "Ext(End)" => InputEvent::Ext(ExtKey::End),
        _ => {
            let rest = s.strip_prefix("Char(")?;
            let byte = rest.trim_end_matches(')');
            if let Ok(n) = byte.parse::<u8>() {
                return Some(InputEvent::Char(n));
            }
            return byte.trim_matches('\'').bytes().next().map(InputEvent::Char);
        }
    })
}

/// A parsed session: every tick that carried input, in order.
#[derive(Debug, Clone, Default)]
pub struct Session {
    pub schedule: Vec<(u64, Vec<InputEvent>)>,
    /// Event strings the log carried that this build does not understand —
    /// surfaced rather than swallowed, because a silently dropped key is a
    /// replay that quietly stops being the recorded run.
    pub unrecognized: Vec<String>,
    /// ★ Every `tick N | … | <probe>` line's probe, in order — the recorded
    /// run's own account of what state it was in. A replay compares its engine
    /// against these ([`Session::probe_at`]): a wrong boot posture, a changed
    /// flow, a dropped key all show up as the first mismatch, at the tick it
    /// happened, instead of as a digest that quietly means something else.
    pub probes: Vec<(u64, String)>,
}

impl Session {
    /// Parses a debug log. Non-`tick` lines (key traces, transcripts, io
    /// lines) are ignored; a `tick` line with an empty `sent []` carries no
    /// input and needs no entry, since replay ticks every index anyway.
    pub fn parse(text: &str) -> Self {
        let mut session = Session::default();
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("tick ") else {
                continue;
            };
            let Some((tick, sent)) = rest.split_once(" | sent [") else {
                continue;
            };
            let Some((events, tail)) = sent.split_once(']') else {
                continue;
            };
            let Ok(tick) = tick.trim().parse::<u64>() else {
                continue;
            };
            if let Some(probe) = tail.strip_prefix(" | ") {
                session.probes.push((tick, probe.trim().to_string()));
            }
            if events.is_empty() {
                continue;
            }
            let mut parsed = Vec::new();
            for piece in events.split(',') {
                match parse_event(piece) {
                    Some(e) => parsed.push(e),
                    None => session.unrecognized.push(piece.trim().to_string()),
                }
            }
            if !parsed.is_empty() {
                session.schedule.push((tick, parsed));
            }
        }
        session
    }

    /// The probe the recorded session reported at `tick`, if it logged one.
    pub fn probe_at(&self, tick: u64) -> Option<&str> {
        match self.probes.binary_search_by_key(&tick, |(t, _)| *t) {
            Ok(i) => Some(self.probes[i].1.as_str()),
            Err(_) => None,
        }
    }

    /// The first probe the log carries, with its tick — the earliest point a
    /// replay can tell whether it booted the way the recording did.
    pub fn first_probe(&self) -> Option<(u64, &str)> {
        self.probes.first().map(|(t, p)| (*t, p.as_str()))
    }

    /// The last tick carrying input (0 for an inputless log).
    pub fn last_input_tick(&self) -> u64 {
        self.schedule.last().map(|(t, _)| *t).unwrap_or(0)
    }

    /// The input batch for `tick`, or an empty slice.
    pub fn inputs_at(&self, tick: u64) -> &[InputEvent] {
        match self.schedule.binary_search_by_key(&tick, |(t, _)| *t) {
            Ok(i) => &self.schedule[i].1,
            Err(_) => &[],
        }
    }
}

/// How a replay boots — it must match the desktop that wrote the log, or the
/// run is a different run.
///
/// The three variants are exactly the desktop's three `--slot` postures
/// (`frontends/desktop/src/main.rs`'s `SlotArg`), deliberately: "same flags on
/// both sides ⇒ same run" is only true if the two flag sets are the same set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    /// The desktop default: import `savgam{letter}.dat` from `<data>/SAVE`.
    ImportedSlot(char),
    /// `--slot none`: no party (engine archaeology only).
    Bare,
    /// ★ No `--slot` at all (roll-credits slice 9b): the **front door** —
    /// title screens, the Play-Demo prompt, copy protection, `startGameMenu`.
    FrontDoor,
}

impl Default for Boot {
    /// **Whatever `restrike-desktop` does with no arguments.** Since slice 9b
    /// that is the front door; before it, an imported slot A. Defined by that
    /// sentence rather than by a fixed variant, because a log recorded with no
    /// flags must replay with no flags — and the two defaults drifting apart
    /// is a tick-0 divergence nothing else would report.
    fn default() -> Self {
        Boot::FrontDoor
    }
}

/// Boots the engine a replay needs. `seed` must be the desktop's (its
/// `DEFAULT_SEED` is 1 unless `--seed` was passed).
pub fn boot(data_dir: &Path, boot: Boot, seed: u32) -> Result<Engine, String> {
    let data = load_dir(data_dir).map_err(|e| format!("{} unreadable: {e}", data_dir.display()))?;
    match boot {
        Boot::Bare => Engine::new(data, seed).map_err(|e| format!("bare boot failed: {e:?}")),
        Boot::FrontDoor => {
            Engine::new_front_door(data, seed).map_err(|e| format!("front-door boot failed: {e:?}"))
        }
        Boot::ImportedSlot(letter) => {
            let save_dir = data_dir.join("SAVE");
            let saves = load_dir(&save_dir)
                .map_err(|e| format!("{} unreadable: {e}", save_dir.display()))?;
            let name = crate::saveload::original_master_filename(letter);
            let master = saves
                .raw_file(&name)
                .ok_or_else(|| format!("no {name} in {}", save_dir.display()))?;
            let set =
                gbx_formats::save_orig::load_from_lookup(master, letter, |n| saves.raw_file(n))
                    .map_err(|e| format!("slot {letter} did not parse: {e:?}"))?;
            crate::import::import_original(&set, data, seed)
                .map_err(|e| format!("slot {letter} did not import: {e:?}"))
        }
    }
}

/// Copies every regular file of `from` into a fresh `to` (a save directory is
/// flat, so no recursion). A missing source is fine — the replay simply has no
/// slots, exactly like a first launch.
///
/// This is how a replay gets the live session's save slots without ever
/// writing to them: forensics must not mutate the evidence.
pub fn sandbox_saves(from: &Path, to: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_dir_all(to);
    std::fs::create_dir_all(to)?;
    let Ok(entries) = std::fs::read_dir(from) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// A per-process sandbox path under the system temp dir.
pub fn sandbox_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("restrike-{tag}-saves-{}", std::process::id()))
}

/// ★ **The host's post-tick obligation** (D8), in one place: take any pending
/// [`SaveLoadRequest`], fulfill it against `saves_dir`, re-inject the slot
/// directory the screen renders from, and — after a Load or Import, which
/// replace the engine wholesale — put the 3D viewport back.
///
/// Every host runs this: the desktop (which adds an on-screen notice for the
/// player), `restrike replay`, and the forensics example. Sharing it is not
/// tidiness — a replay whose host behaviour differs from the desktop's by one
/// missing step is not the recorded run, and nothing would say so.
///
/// ★ **The posture/flow referee** (roll-credits slice 9b): compares the live
/// engine against what the recording said it was doing at this tick.
///
/// Both replay fronts call this every tick. It answers `Some(recorded)` only
/// when the log logged a probe at `tick` **and** the engine disagrees — so a
/// caller can report the FIRST divergence loudly and keep going. The common
/// cause is a boot-posture mismatch, which shows up at the very first logged
/// tick (`front-door/title` against `boot/present`, say) rather than as a
/// digest that quietly means something else; but it catches any drift, which
/// is the point.
pub fn probe_divergence(session: &Session, tick: u64, engine: &Engine) -> Option<String> {
    let recorded = session.probe_at(tick)?;
    let live = engine.probe();
    (recorded != live).then_some(recorded.to_string())
}

/// Returns `None` when the tick asked for nothing.
pub fn fulfill_pending_io(
    engine: &mut Engine,
    saves_dir: &Path,
    seed: u32,
) -> Option<(SaveLoadRequest, Result<(), crate::saveload_fs::SlotIoError>)> {
    let request = engine.take_io_request()?;
    let replaced = !matches!(request, SaveLoadRequest::Save(_));
    let data = engine.game_data().clone();
    let result = crate::saveload_fs::fulfill(engine, request, saves_dir, data, seed);
    engine.set_slot_directory(crate::saveload_fs::scan_slot_directory(saves_dir));
    if result.is_ok() && replaced {
        engine.recompose_world_screen();
    }
    Some((request, result))
}

/// ★ The same shape for the **character-file** request (roll-credits slice
/// 9c): creation's `Save <name>?`, `Remove`'s own `SavePlayer`, and `Add`'s
/// load. Rescans the directory afterwards, so the Add picker sees a file the
/// moment it is written and stops offering a character who has just joined.
///
/// Returns a player-facing line — the `.guy` that was written, the character
/// who joined, or the original's own refusal words — for the host to show
/// (`Engine::report_host_notice`). D8 keeps the *outcome* on the host side,
/// and the M6 forensics rule says an outcome nothing shows is not an outcome.
#[cfg(not(target_arch = "wasm32"))]
pub fn fulfill_pending_char_file(engine: &mut Engine, saves_dir: &Path) -> Option<String> {
    use crate::chr_file::CharFileRequest;
    let request = engine.take_char_file_request()?;
    let described = match &request {
        CharFileRequest::Save(ch) => format!(
            "Saved {}.{}",
            crate::chr_file::clean_stem(&ch.name),
            crate::chr_file::CHAR_EXT
        ),
        CharFileRequest::Load(stem) => format!("Loaded {stem}"),
    };
    let notice = match crate::saveload_fs::fulfill_char_file(engine, request, saves_dir) {
        Ok(None) => described,
        Ok(Some(refusal)) => refusal,
        Err(e) => format!("character file failed: {e:?}"),
    };
    engine.set_char_file_directory(
        crate::saveload_fs::scan_char_files(saves_dir).without_party_members(engine.party()),
    );
    Some(notice)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_desktop_loggers_own_lines() {
        let log = "\
key: Character(\"e\") state=Pressed repeat=false
tick 5 | sent [Char(101)] | world-menu
tick 7 | sent [] | screen
tick 9 | sent [Enter, Escape] | screen
    Print { text: \"hi\", clear_first: false }
tick 11 | sent [Ext(Up), Char('b')] | step/gate(hotbar)
";
        let s = Session::parse(log);
        assert!(s.unrecognized.is_empty(), "{:?}", s.unrecognized);
        assert_eq!(s.schedule.len(), 3);
        assert_eq!(s.inputs_at(5), &[InputEvent::Char(b'e')]);
        assert_eq!(s.inputs_at(7), &[], "an empty batch carries no input");
        assert_eq!(s.inputs_at(9), &[InputEvent::Enter, InputEvent::Escape]);
        assert_eq!(
            s.inputs_at(11),
            &[InputEvent::Ext(ExtKey::Up), InputEvent::Char(b'b')]
        );
        assert_eq!(s.last_input_tick(), 11);
    }

    #[test]
    fn an_unknown_event_is_reported_not_swallowed() {
        let s = Session::parse("tick 3 | sent [Char(97), Wat] | world-menu\n");
        assert_eq!(s.inputs_at(3), &[InputEvent::Char(b'a')]);
        assert_eq!(s.unrecognized, vec!["Wat".to_string()]);
    }

    #[test]
    fn a_log_with_no_input_parses_to_nothing() {
        let s = Session::parse("nothing here\ntick 2 | sent [] | world-menu\n");
        assert!(s.schedule.is_empty());
        assert_eq!(s.last_input_tick(), 0);
    }

    // --- the H5 round trip (roll-credits D-RC3) ---

    const RUN_TICKS: u64 = 140;
    const CHECKPOINT_EVERY: u64 = 10;
    const SEED: u32 = 7;

    /// The scripted session both halves run: camp ▸ Save ▸ Save ▸ slot A ▸
    /// Exit, then the world menu's Search toggle so the state genuinely moves,
    /// then camp ▸ Save ▸ Load ▸ slot A, which must put it back. One key every
    /// other tick, which is what a human's keystrokes look like to a 60 Hz
    /// tick loop.
    fn script(tick: u64) -> &'static [InputEvent] {
        match tick {
            30 => &[InputEvent::Char(b'e')], // Encamp
            32 => &[InputEvent::Char(b's')], // Camp ▸ Save
            34 => &[InputEvent::Char(b's')], // Save mode
            36 => &[InputEvent::Char(b'A')], // slot A
            38 => &[InputEvent::Char(b'e')], // Camp ▸ Exit
            60 => &[InputEvent::Char(b's')], // world menu ▸ Search (toggles)
            90 => &[InputEvent::Char(b'e')], // Encamp
            92 => &[InputEvent::Char(b's')], // Camp ▸ Save
            94 => &[InputEvent::Char(b'l')], // Load mode
            96 => &[InputEvent::Char(b'A')], // slot A
            _ => &[],
        }
    }

    /// Runs the script against a fresh fixture engine, writing the desktop's
    /// own log format and collecting checkpoint digests. This is the
    /// *recording* side, standing in for a live desktop session.
    fn record(saves_dir: &Path) -> (String, Vec<(u64, String)>) {
        let mut engine = crate::save_roundtrip_tests::imported_engine();
        let _ = std::fs::remove_dir_all(saves_dir);
        engine.set_slot_directory(crate::saveload_fs::scan_slot_directory(saves_dir));

        let mut log = String::new();
        let mut digests = Vec::new();
        for tick in 1..=RUN_TICKS {
            let batch = script(tick);
            engine.tick(batch);
            if !batch.is_empty() {
                use std::fmt::Write;
                let _ = writeln!(log, "tick {tick} | sent {batch:?} | {}", engine.probe());
            }
            if let Some((request, outcome)) = fulfill_pending_io(&mut engine, saves_dir, SEED) {
                outcome.unwrap_or_else(|e| panic!("recording: {request:?} failed: {e:?}"));
            }
            if tick % CHECKPOINT_EVERY == 0 {
                digests.push((tick, engine.state_digest()));
            }
        }
        (log, digests)
    }

    /// Replays a parsed log against a fresh fixture engine — the *verify*
    /// side, i.e. exactly what `restrike replay` does around this module.
    fn replay(session: &Session, saves_dir: &Path) -> Vec<(u64, String)> {
        let mut engine = crate::save_roundtrip_tests::imported_engine();
        let _ = std::fs::remove_dir_all(saves_dir);
        engine.set_slot_directory(crate::saveload_fs::scan_slot_directory(saves_dir));

        let mut digests = Vec::new();
        for tick in 1..=RUN_TICKS {
            engine.tick(session.inputs_at(tick));
            fulfill_pending_io(&mut engine, saves_dir, SEED);
            if tick % CHECKPOINT_EVERY == 0 {
                digests.push((tick, engine.state_digest()));
            }
        }
        digests
    }

    /// ★ **The H5 round trip, CI tier**: a scripted session is recorded in the
    /// desktop's own log format, replayed from that log alone, and every
    /// checkpoint digest matches. Synthetic fixtures throughout — no game data,
    /// so this runs everywhere.
    ///
    /// The script deliberately saves and loads, because that is the one thing
    /// a replay could silently get wrong: the request is emitted by the tick
    /// core and fulfilled by the host, so a replay that skipped the host's half
    /// would diverge at the load with no error anywhere.
    #[test]
    fn a_recorded_fixture_session_replays_to_identical_digests() {
        let pid = std::process::id();
        let rec_dir = std::env::temp_dir().join(format!("restrike-h5-record-{pid}"));
        let rep_dir = std::env::temp_dir().join(format!("restrike-h5-replay-{pid}"));

        let (log, recorded) = record(&rec_dir);
        let session = Session::parse(&log);
        assert!(
            session.unrecognized.is_empty(),
            "{:?}",
            session.unrecognized
        );
        assert_eq!(
            session.schedule.len(),
            10,
            "every scripted keystroke survived the log round trip"
        );

        let replayed = replay(&session, &rep_dir);
        assert_eq!(recorded, replayed, "the replay is not the recorded run");

        // And the run genuinely moved: a schedule that did nothing would pass
        // the comparison above trivially.
        assert!(
            recorded
                .iter()
                .map(|(_, d)| d)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 1,
            "the session changed engine state at least once"
        );

        let _ = std::fs::remove_dir_all(&rec_dir);
        let _ = std::fs::remove_dir_all(&rep_dir);
    }

    /// The same round trip over a **real imported boot** (local tier,
    /// `GBX_DATA_DIR`): `Boot::ImportedSlot('A')` explicitly, because this is
    /// about the imported posture specifically — `Boot::default()` tracks the
    /// desktop, which since slice 9b opens on the front door instead.
    #[test]
    fn a_real_imported_boot_replays_to_identical_digests() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!("GBX_DATA_DIR not set — skipping (local tier)");
            return;
        };
        let dir = PathBuf::from(dir);
        let pid = std::process::id();

        // One "session": press Enter every 30 ticks to page the intro along.
        let script = |tick: u64| -> &'static [InputEvent] {
            if tick.is_multiple_of(30) {
                &[InputEvent::Enter]
            } else {
                &[]
            }
        };

        let run = |tag: &str| -> Vec<(u64, String)> {
            let saves = std::env::temp_dir().join(format!("restrike-h5-{tag}-{pid}"));
            sandbox_saves(&dir.join("SAVE"), &saves).expect("saves sandbox");
            let mut engine = boot(&dir, Boot::ImportedSlot('A'), 1).expect("slot A boots");
            engine.set_slot_directory(crate::saveload_fs::scan_slot_directory(&saves));
            let mut digests = Vec::new();
            for tick in 1..=300u64 {
                engine.tick(script(tick));
                fulfill_pending_io(&mut engine, &saves, 1);
                if tick % 50 == 0 {
                    digests.push((tick, engine.state_digest()));
                }
            }
            let _ = std::fs::remove_dir_all(&saves);
            digests
        };

        let first = run("real-a");
        let log: String = (1..=300u64)
            .filter(|t| !script(*t).is_empty())
            .map(|t| {
                format!(
                    "tick {t} | sent {:?} | boot/gate(press-any-key)\n",
                    script(t)
                )
            })
            .collect();
        let session = Session::parse(&log);
        assert_eq!(session.schedule.len(), 10);

        let saves = std::env::temp_dir().join(format!("restrike-h5-real-b-{pid}"));
        sandbox_saves(&dir.join("SAVE"), &saves).expect("saves sandbox");
        let mut engine = boot(&dir, Boot::ImportedSlot('A'), 1).expect("slot A boots");
        engine.set_slot_directory(crate::saveload_fs::scan_slot_directory(&saves));
        let mut second = Vec::new();
        for tick in 1..=300u64 {
            engine.tick(session.inputs_at(tick));
            fulfill_pending_io(&mut engine, &saves, 1);
            if tick % 50 == 0 {
                second.push((tick, engine.state_digest()));
            }
        }
        let _ = std::fs::remove_dir_all(&saves);

        assert_eq!(first, second, "the imported-boot replay diverged");
        eprintln!("  real imported boot: {} checkpoints", first.len());
        for (tick, digest) in &first {
            eprintln!("    {tick}\t{digest}");
        }
    }

    /// ★ **The front-door posture replays** (roll-credits slice 9b, local
    /// tier): a session recorded from a default-launch desktop — title beats
    /// skipped with keys, the demo declined, the challenge accepted, the start
    /// menu reached — replays to identical digests through `Boot::default()`.
    ///
    /// This is the case the desktop's default now produces, and the one that
    /// had no posture at all until this test existed: a log written by a
    /// no-flag desktop and replayed by a no-flag replayer.
    #[test]
    fn a_front_door_session_replays_to_identical_digests() {
        let Some(dir) = std::env::var_os("GBX_DATA_DIR") else {
            eprintln!("GBX_DATA_DIR not set — skipping (local tier)");
            return;
        };
        let dir = PathBuf::from(dir);

        // Space walks the four title beats (each key ends one, and the paint
        // tick flushes the buffer, so they are spread out); `P` declines the
        // demo; Enter accepts the pre-filled copy-protection answer.
        let script = |tick: u64| -> &'static [InputEvent] {
            match tick {
                10 | 20 | 30 | 40 | 50 | 60 | 70 | 80 => &[InputEvent::Char(b' ')],
                100 => &[InputEvent::Char(b'P')],
                140 => &[InputEvent::Enter],
                _ => &[],
            }
        };

        let run = |posture: Boot| -> (Vec<(u64, String)>, String) {
            let mut engine = boot(&dir, posture, 1).expect("the posture boots");
            let mut digests = Vec::new();
            for tick in 1..=200u64 {
                engine.tick(script(tick));
                if tick % 25 == 0 {
                    digests.push((tick, engine.state_digest()));
                }
            }
            (digests, engine.probe())
        };

        let (recorded, ended_at) = run(Boot::default());
        assert_eq!(
            ended_at, "front-door/start-menu",
            "the scripted session reaches `startGameMenu`"
        );

        // The log a desktop would have written for that session, replayed.
        let log: String = (1..=200u64)
            .filter(|t| !script(*t).is_empty())
            .map(|t| format!("tick {t} | sent {:?} | front-door/title\n", script(t)))
            .collect();
        let session = Session::parse(&log);
        assert_eq!(session.schedule.len(), 10);
        assert_eq!(session.probes.len(), 10, "probe lines are parsed too");

        let mut engine = boot(&dir, Boot::default(), 1).expect("the default posture boots");
        let mut replayed = Vec::new();
        for tick in 1..=200u64 {
            engine.tick(session.inputs_at(tick));
            if tick % 25 == 0 {
                replayed.push((tick, engine.state_digest()));
            }
        }
        assert_eq!(recorded, replayed, "the front-door replay diverged");

        // ★ And the referee works: replaying the same log against the WRONG
        // posture is caught, at the first logged tick, instead of silently
        // producing a different run's digests.
        let mut wrong = boot(&dir, Boot::ImportedSlot('A'), 1).expect("slot A boots");
        let mut caught = None;
        for tick in 1..=200u64 {
            wrong.tick(session.inputs_at(tick));
            if caught.is_none() {
                caught = probe_divergence(&session, tick, &wrong).map(|r| (tick, r));
            }
        }
        let (tick, recorded_probe) = caught.expect("a wrong boot posture must be reported");
        assert_eq!(tick, 10, "reported at the first tick the log recorded one");
        assert_eq!(recorded_probe, "front-door/title");
        assert_ne!(
            wrong.state_digest(),
            engine.state_digest(),
            "the wrong posture really is a different run"
        );
    }
}
