//! **Capture → replay input** (`docs/design/combat-visualizer.md` §4 M6a's
//! library-ification): the one place a `.gbxtrace` becomes something the engine
//! can fight.
//!
//! Through M5 this lived in `tests/common/mod.rs` — test-only code three
//! harnesses shared by `mod common;`-ing the same file. M6a's reel needs the
//! identical assembly *outside* a test binary (a `--watch` flag on a frontend
//! cannot `mod common;`), so it moved into the library. The dependency
//! direction is unchanged and remains a settled door: **oracle → engine, never
//! the reverse** — the `.gbxtrace` types stay here, and the product is
//! `gbx_engine::combat::reel::ReelInput`, plain engine-side data.
//!
//! Layout:
//! - [`kits`] — the per-capture ranged loadouts (doc §34.1/§45/§48).
//! - [`sidecar`] — the versioned per-capture knob + icon pins (doc §4 M6a).
//! - this module — the assembly: [`reel_input`], [`capture_draws`],
//!   [`knobs_for`], and the local-tier file helpers the harnesses share.
//!
//! **What the harnesses lost by moving here: nothing.** `h4_replay`,
//! `h4_turndiff` and the frontier guard build their `CombatState` through
//! [`reel_input`] + `gbx_engine::combat::reel::build_state`, applying the same
//! knobs in the same order to the same records. The guard is the referee for
//! that claim — 15/15 exact, every commit.

pub mod kits;
pub mod sidecar;

use gbx_engine::combat::reel::{
    ExpectedDraw, ReelArt, ReelCombatant, ReelInput, ReelKnobs, Team, COMBAT_RECORD_LEN,
};
use gbx_engine::combat::Loadout;
use gbx_formats::items::ItemDataTable;
use gbx_formats::save_orig::decode_char_record;
use std::fmt;
use std::path::{Path, PathBuf};

pub use kits::{capture_has_loadout, loadout_for};
pub use sidecar::{sidecar_for, sidecar_or_default, CaptureSidecar, SidecarError};

use crate::format::{CombatEntryEvent, ParseError, Trace};

/// Everything that can stop a capture from becoming a replay.
///
/// Not `PartialEq` on the whole enum — [`ParseError`] isn't — so tests compare
/// with [`ReplayError::same_kind`] or `matches!`.
#[derive(Debug)]
pub enum ReplayError {
    /// The `.gbxtrace` itself didn't parse.
    Trace(ParseError),
    /// No `combat_entry` line — not a combat capture.
    NoCombatEntry,
    /// A roster row carried a team byte that isn't 0 (party) or 1 (monsters).
    UnknownTeam { index: usize, team: u8 },
    /// The capture carries a ranged loadout but the resident `ITEMS` table is
    /// absent, so the replay would silently fall back to melee and diverge at
    /// its first shot (the guard's own loud-skip condition, D10).
    ItemsRequired { capture: String },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::Trace(e) => write!(f, "capture did not parse: {e:?}"),
            ReplayError::NoCombatEntry => {
                write!(f, "capture carries no `combat_entry` snapshot")
            }
            ReplayError::UnknownTeam { index, team } => {
                write!(f, "combat_entry roster row {index} has team byte {team}")
            }
            ReplayError::ItemsRequired { capture } => write!(
                f,
                "{capture} carries a ranged loadout and needs the resident ITEMS table \
                 (~/goldbox-data/cotab/ITEMS or GBX_ITEMS_FILE); replaying without it \
                 falls back to melee and diverges"
            ),
        }
    }
}

impl ReplayError {
    /// Discriminant + payload equality, for tests. Two [`ReplayError::Trace`]s
    /// compare equal on their message text only.
    pub fn same_kind(&self, other: &ReplayError) -> bool {
        match (self, other) {
            (ReplayError::Trace(a), ReplayError::Trace(b)) => format!("{a:?}") == format!("{b:?}"),
            (ReplayError::NoCombatEntry, ReplayError::NoCombatEntry) => true,
            (
                ReplayError::UnknownTeam { index: a, team: t },
                ReplayError::UnknownTeam { index: b, team: u },
            ) => a == b && t == u,
            (
                ReplayError::ItemsRequired { capture: a },
                ReplayError::ItemsRequired { capture: b },
            ) => a == b,
            _ => false,
        }
    }
}

impl std::error::Error for ReplayError {}

impl From<ParseError> for ReplayError {
    fn from(e: ParseError) -> Self {
        ReplayError::Trace(e)
    }
}

// === the pure assembly ====================================================

/// Decode the `combat_entry.terrain` lowercase-hex ground grid.
///
/// Terrain is load-bearing for movement (doc §14), so a modern capture's replay
/// always builds its `CombatMap` from this. Odd trailing nibbles are dropped
/// (`chunks_exact`), which cannot happen in a well-formed capture.
pub fn decode_terrain(hex: &str) -> Vec<u8> {
    let b = hex.as_bytes();
    let val = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    b.chunks_exact(2)
        .map(|p| (val(p[0]) << 4) | val(p[1]))
        .collect()
}

/// Decode a capture combatant's raw affect chain (hex-encoded 9-byte nodes,
/// the §44.2 hook channel) into engine `AffectRecord`s, order preserved
/// (find-FIRST is order-observable, §39.2/§47.7). Nodes too short to decode
/// are skipped (defensive; real nodes are always 9 bytes).
pub fn decode_affect_nodes(hex_nodes: &[String]) -> Vec<gbx_formats::affects::AffectRecord> {
    hex_nodes
        .iter()
        .filter_map(|h| {
            let bytes: Vec<u8> = (0..h.len().saturating_sub(1))
                .step_by(2)
                .filter_map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok())
                .collect();
            gbx_formats::affects::AffectRecord::decode(&bytes)
        })
        .collect()
}

/// The engine-semantic `area2.field_6E4` from the captured raw word — the
/// **byte-bridge** (doc §47). The ECL stores a BYTE (sewer-fight-3 records the
/// word 253 = 0x00FD), and the binary's add (`sub_3E124` @`ovr014:0140-014E`,
/// listing-verified) is `mov al,movement; xor ah,ah; add ax,[6E4]word;
/// mov [movement],al` — a WORD add whose result is truncated to AL on store.
/// `(movement + word) & 0xFF ≡ (movement + sign_extended_low_byte) & 0xFF`
/// identically (the high byte cannot reach the low byte of a sum), so the
/// engine's i32 domain takes the low byte sign-extended: 0xFD → −3. The
/// mapping is also invariant to how the hook signed the word (+253 and −3
/// share a low byte). The subsequent clamp (`<1 || >0x60 → 1`,
/// `ovr014:0156-0166`) is `gbx_engine::combat::calc_moves`'s, already faithful
/// in the i32 domain for every value whose word sum stays under 256.
pub fn area_6e4_from_word(raw: i32) -> i32 {
    i32::from((raw & 0xFF) as u8 as i8)
}

/// Merge a capture's own emitted knobs with its sidecar pins.
///
/// **Precedence is the harnesses' existing one, unchanged**: what the staging
/// hook actually observed always beats a hand pin.
///
/// | knob | source |
/// |---|---|
/// | `map_direction` | capture's `map_direction` → sidecar → 2 |
/// | `area_field_58c` | capture's `area2_field_58c` → 99 |
/// | `area_field_6e4` | capture's `area2_field_6e4` (byte-bridged) → sidecar |
/// | `auto_cast`, `auto_cast_toggles`, `continue_battle` | sidecar only — these are keypresses no hook records |
pub fn knobs_for(entry: &CombatEntryEvent, sidecar: &CaptureSidecar) -> ReelKnobs {
    ReelKnobs {
        map_direction: entry.map_direction.unwrap_or(sidecar.knobs.map_direction),
        auto_cast: sidecar.knobs.auto_cast,
        auto_cast_toggles: sidecar.knobs.auto_cast_toggles.clone(),
        continue_battle: sidecar.knobs.continue_battle.clone(),
        // 99 is the measured bar value under which the natural rout is
        // mathematically impossible (doc §28) — the documented pre-emission
        // default every closed capture rode.
        area_field_58c: entry.area2_field_58c.map(|v| v as i32).unwrap_or(99),
        area_field_6e4: entry
            .area2_field_6e4
            .map(|v| area_6e4_from_word(i32::from(v)))
            .unwrap_or(sidecar.knobs.area_field_6e4),
    }
}

/// The per-roster-index loadout rows for one capture (doc §34.1): decode each
/// record for its name, look the name up in [`kits::loadout_for`].
///
/// Records that decode to no kit are simply absent from the result — that is
/// the melee-identical path.
pub fn loadouts_for(capture: &str, entry: &CombatEntryEvent) -> Vec<(usize, Loadout)> {
    entry
        .combatants
        .iter()
        .enumerate()
        .filter_map(|(id, c)| {
            let record = decode_char_record(&c.record).ok()?;
            loadout_for(capture, &record.name).map(|l| (id, l))
        })
        .collect()
}

/// Assemble the engine-side replay input from a parsed `combat_entry` + its
/// sidecar.
///
/// `expected_draws` is left empty: the caller supplies the capture's draw stream
/// with [`capture_draws`] (it needs the raw text, which the typed reader has
/// already consumed). [`reel_input_from_capture`] does both in one call.
pub fn reel_input(
    capture: &str,
    entry: &CombatEntryEvent,
    sidecar: &CaptureSidecar,
    item_data: Option<ItemDataTable>,
) -> Result<ReelInput, ReplayError> {
    if capture_has_loadout(capture) && item_data.is_none() {
        return Err(ReplayError::ItemsRequired {
            capture: capture.to_string(),
        });
    }

    let mut combatants = Vec::with_capacity(entry.combatants.len());
    for (index, c) in entry.combatants.iter().enumerate() {
        let team = match c.team {
            0 => Team::Party,
            1 => Team::Monster,
            team => return Err(ReplayError::UnknownTeam { index, team }),
        };
        debug_assert_eq!(c.record.len(), COMBAT_RECORD_LEN);
        combatants.push(ReelCombatant {
            team,
            pos: gbx_engine::combat::GridPos::new(c.x as i32, c.y as i32),
            record: c.record.to_vec(),
            affects: decode_affect_nodes(&c.affects),
        });
    }

    let mut input = ReelInput::new(capture, entry.rng_state, combatants);
    input.terrain = entry.terrain.as_deref().map(decode_terrain);
    input.knobs = knobs_for(entry, sidecar);
    input.loadouts = loadouts_for(capture, entry);
    input.item_data = item_data;
    // §9.6: the manual-turn schedule — sidecar-only, like every keypress knob.
    // Empty for the all-QuickFight captures, whose replay path is then
    // bit-identical to the pre-schedule one (`build_state` never sets the
    // interactive flag for an empty script).
    input.manual_script = sidecar.knobs.manual_turns.clone();
    input.art = ReelArt {
        cpic_area: sidecar.art.cpic_area,
        monster_blocks: sidecar.monster_blocks(),
        in_dungeon: sidecar.art.in_dungeon,
    };
    Ok(input)
}

/// The whole job in one call: parse the capture text, look up its committed
/// sidecar (falling back to all-defaults for an unpinned capture), assemble the
/// input, and attach the capture's own draw stream for the reel's live equality
/// assert.
pub fn reel_input_from_capture(
    capture: &str,
    text: &str,
    item_data: Option<ItemDataTable>,
) -> Result<ReelInput, ReplayError> {
    let trace = Trace::parse(text)?;
    let entry = trace.combat_entry().ok_or(ReplayError::NoCombatEntry)?;
    let sidecar = sidecar_or_default(capture);
    let mut input = reel_input(capture, entry, &sidecar, item_data)?;
    input.expected_draws = capture_draws(text);
    Ok(input)
}

/// The capture's post-`combat_entry` draws, `(before, after, operand)` per `rng`
/// event in file order.
///
/// Pulled straight from the raw JSONL rather than the typed reader because the
/// diagnostic operand lives in `ss_sp_words[3]`, an *unknown* field to the
/// reader (the writer never emits it; the staging hook does). Ordering matches
/// the typed reader's event order. This is the function `h4_replay` and the
/// frontier guard each carried a private copy of through M5.
pub fn capture_draws(text: &str) -> Vec<ExpectedDraw> {
    let mut out = Vec::new();
    let mut seen_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("e").and_then(|e| e.as_str()) {
            Some("combat_entry") => seen_entry = true,
            Some("rng") if seen_entry => {
                let (Some(before), Some(after)) = (
                    v.get("before").and_then(|n| n.as_u64()),
                    v.get("after").and_then(|n| n.as_u64()),
                ) else {
                    continue;
                };
                out.push(ExpectedDraw {
                    before: before as u32,
                    after: after as u32,
                    operand: v
                        .get("ss_sp_words")
                        .and_then(|w| w.as_array())
                        .and_then(|w| w.get(3))
                        .and_then(|n| n.as_u64())
                        .map(|n| n as u16),
                });
            }
            _ => {}
        }
    }
    out
}

// === local-tier host helpers ==============================================
//
// File and environment access, kept together and clearly labelled: these are
// the *host's* half, not the assembly's. The pure functions above never touch
// the filesystem, so a wasm or in-memory host uses them directly.

/// Load the resident `ITEMS` table from the local game dir (D10).
///
/// `GBX_ITEMS_FILE` overrides the default `~/goldbox-data/cotab/ITEMS`. `None`
/// when the file is absent — a caller then replays melee-only (and should skip
/// loadout-bearing captures via [`capture_has_loadout`], or let
/// [`reel_input`]'s `ItemsRequired` say so).
pub fn load_item_data() -> Option<ItemDataTable> {
    let path = std::env::var_os("GBX_ITEMS_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| Path::new(&h).join("goldbox-data/cotab/ITEMS"))
        })?;
    let bytes = std::fs::read(path).ok()?;
    ItemDataTable::parse(&bytes).ok()
}

/// The local traces directory, or `None` when the local tier is not active
/// (neither `GBX_TRACES_DIR` nor `GBX_DATA_DIR` set → plain CI skips, D10).
pub fn traces_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GBX_TRACES_DIR") {
        return Some(PathBuf::from(d));
    }
    std::env::var_os("GBX_DATA_DIR")?;
    let home = std::env::var_os("HOME")?;
    Some(Path::new(&home).join("goldbox-data/traces"))
}

/// Apply the `RESTRIKE_*` trial overrides to a knob set.
///
/// These are the **investigator's** knobs — an explicit override for a session
/// probing an open frontier (`RESTRIKE_MAP_DIR`, `RESTRIKE_AUTO_CAST`,
/// `RESTRIKE_AUTO_CAST_TOGGLES`, `RESTRIKE_CONTINUE_BATTLE`,
/// `RESTRIKE_AREA_6E4`). They sit ABOVE both the capture and the sidecar, which
/// is why the frontier guard deliberately does **not** call this: the guard must
/// be hermetic, or an exported variable could quietly move a pin.
pub fn apply_env_overrides(knobs: &mut ReelKnobs) {
    if let Some(md) = std::env::var("RESTRIKE_MAP_DIR")
        .ok()
        .and_then(|s| s.parse::<u8>().ok())
    {
        knobs.map_direction = md;
    }
    if let Ok(v) = std::env::var("RESTRIKE_AUTO_CAST") {
        knobs.auto_cast = v == "1";
    }
    if let Ok(v) = std::env::var("RESTRIKE_AUTO_CAST_TOGGLES") {
        knobs.auto_cast_toggles = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    }
    if let Ok(v) = std::env::var("RESTRIKE_CONTINUE_BATTLE") {
        knobs.continue_battle = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    }
    if let Some(v) = std::env::var("RESTRIKE_AREA_6E4")
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
    {
        knobs.area_field_6e4 = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::CombatEntryCombatant;

    /// A hand-authored `combat_entry` (D10 — synthetic records only).
    fn entry() -> CombatEntryEvent {
        let mut record = [0u8; COMBAT_RECORD_LEN];
        let name = b"MATHEW";
        record[0] = name.len() as u8;
        record[1..1 + name.len()].copy_from_slice(name);
        record[0x78] = 20;
        record[0x1a4] = 20;
        record[0x143] = 0; // icon slot 0 — a party member

        let mut monster = [0u8; COMBAT_RECORD_LEN];
        let mname = b"FIRE KNIFE";
        monster[0] = mname.len() as u8;
        monster[1..1 + mname.len()].copy_from_slice(mname);
        monster[0x78] = 12;
        monster[0x1a4] = 12;
        monster[0x143] = 8; // icon slot 8 — the first monster type

        CombatEntryEvent {
            rng_state: 0x1234_5678,
            terrain: None,
            area2_field_58c: None,
            area2_field_6e0: None,
            area2_field_6e2: None,
            area2_field_6e4: None,
            map_direction: None,
            combatants: vec![
                CombatEntryCombatant {
                    team: 0,
                    x: 20,
                    y: 12,
                    record,
                    affects: Vec::new(),
                },
                CombatEntryCombatant {
                    team: 1,
                    x: 24,
                    y: 12,
                    record: monster,
                    affects: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn the_capture_beats_the_sidecar_where_a_hook_recorded_it() {
        let sidecar = sidecar_for("sewer-fight-1.gbxtrace").unwrap();
        // sewer-fight-1 pins md 0 and 6E4 −3 for a capture that emits neither.
        let mut e = entry();
        let knobs = knobs_for(&e, &sidecar);
        assert_eq!(knobs.map_direction, 0, "the pin covers a silent capture");
        assert_eq!(knobs.area_field_6e4, -3);
        assert_eq!(knobs.area_field_58c, 99, "the documented default");

        // Now let the capture speak: it wins on all three.
        e.map_direction = Some(6);
        e.area2_field_6e4 = Some(0x00FD); // the §47 byte-bridge: −3
        e.area2_field_58c = Some(75);
        let knobs = knobs_for(&e, &sidecar);
        assert_eq!(knobs.map_direction, 6);
        assert_eq!(knobs.area_field_6e4, -3);
        assert_eq!(knobs.area_field_58c, 75);
        // …but never on the keypress knobs, which no hook records.
        assert_eq!(knobs.auto_cast_toggles, vec![2]);
    }

    #[test]
    fn the_byte_bridge_takes_the_low_byte_sign_extended() {
        assert_eq!(area_6e4_from_word(253), -3, "0x00FD → −3 (doc §47)");
        assert_eq!(area_6e4_from_word(-3), -3, "sign-invariant");
        assert_eq!(area_6e4_from_word(0), 0);
        assert_eq!(area_6e4_from_word(12), 12);
    }

    #[test]
    fn assembly_carries_the_roster_verbatim_and_pins_the_art() {
        // sewer-fight-3 is the itemless roster, so this exercises the assembly
        // without the `ITEMS` gate — its kit lookup still keys FIRE KNIFE by
        // name (every `sewer-fight-*` basename shares the MON2ITM template).
        let sidecar = sidecar_for("sewer-fight-3.gbxtrace").unwrap();
        let input = reel_input("sewer-fight-3.gbxtrace", &entry(), &sidecar, None)
            .expect("the fixture roster assembles");
        assert_eq!(input.rng_state, 0x1234_5678);
        assert_eq!(input.combatants.len(), 2);
        assert_eq!(input.combatants[0].team, Team::Party);
        assert_eq!(input.combatants[1].team, Team::Monster);
        assert_eq!(
            input.combatants[1].pos,
            gbx_engine::combat::GridPos::new(24, 12)
        );
        assert_eq!(input.art.cpic_area, 2);
        assert_eq!(input.art.monster_blocks.get(&8), Some(&7), "TROLL");
        assert_eq!(input.art.monster_blocks.get(&9), Some(&8), "CROCODILE");
        assert!(input.art.in_dungeon);
        // The FIRE KNIFE kit keys by NAME, decoded from the record — and lands
        // on that record's ROSTER INDEX.
        assert_eq!(input.loadouts.len(), 1);
        assert_eq!(input.loadouts[0].0, 1);
        assert_eq!(input.loadouts[0].1.ammo_count, 7);
    }

    #[test]
    fn a_ranged_capture_without_the_items_table_is_refused_loudly() {
        let sidecar = sidecar_for("armed-bar.gbxtrace").unwrap();
        let err = reel_input("armed-bar.gbxtrace", &entry(), &sidecar, None).unwrap_err();
        assert!(err.same_kind(&ReplayError::ItemsRequired {
            capture: "armed-bar.gbxtrace".into()
        }));
        assert!(err.to_string().contains("ITEMS"));
    }

    #[test]
    fn an_impossible_team_byte_is_located() {
        let sidecar = sidecar_for("combat4.gbxtrace").unwrap();
        let mut e = entry();
        e.combatants[1].team = 7;
        assert!(reel_input("combat4.gbxtrace", &e, &sidecar, None)
            .unwrap_err()
            .same_kind(&ReplayError::UnknownTeam { index: 1, team: 7 }));
    }

    #[test]
    fn terrain_decodes_row_major_hex() {
        assert_eq!(decode_terrain("00171aff"), vec![0x00, 0x17, 0x1A, 0xFF]);
        assert_eq!(decode_terrain("0017FF"), vec![0x00, 0x17, 0xFF]);
        assert!(decode_terrain("").is_empty());
    }

    #[test]
    fn capture_draws_reads_only_post_entry_rng_lines() {
        let text = concat!(
            r#"{"e":"rng","before":1,"after":2}"#,
            "\n",
            r#"{"e":"combat_entry","rng_state":5}"#,
            "\n",
            r#"{"e":"rng","before":5,"after":6,"ss_sp_words":[0,0,0,20]}"#,
            "\n",
            "\n",
            r#"not json"#,
            "\n",
            r#"{"e":"rng","before":6,"after":7}"#,
            "\n",
        );
        let draws = capture_draws(text);
        assert_eq!(draws.len(), 2, "the pre-entry draw is not a combat draw");
        assert_eq!(
            draws[0],
            ExpectedDraw {
                before: 5,
                after: 6,
                operand: Some(20)
            }
        );
        assert_eq!(draws[1].operand, None, "a hook that recorded no operand");
    }

    fn header() -> crate::format::TraceHeader {
        crate::format::TraceHeader {
            gbxtrace: 1,
            profile: crate::format::Profile::Prng,
            game: "synthetic".into(),
            seed: 0,
            encounter: "fixture".into(),
            source: "restrike".into(),
            notes: None,
        }
    }

    #[test]
    fn assembly_from_raw_text_attaches_the_draw_stream() {
        // The typed reader consumes the text; the raw pass gets the operands.
        // This is the seam `reel_input_from_capture` exists to hide.
        let mut e = entry();
        e.terrain = Some("1717".into());
        let mut text = Trace::new(header(), vec![crate::format::TraceEvent::CombatEntry(e)])
            .to_canonical_string();
        text.push_str(r#"{"e":"rng","before":5,"after":6,"ss_sp_words":[0,0,0,6]}"#);
        text.push('\n');

        let input = reel_input_from_capture("combat4.gbxtrace", &text, None)
            .expect("a well-formed capture assembles");
        assert_eq!(input.label, "combat4.gbxtrace");
        assert_eq!(input.terrain.as_deref(), Some(&[0x17u8, 0x17][..]));
        assert_eq!(input.expected_draws.len(), 1);
        assert_eq!(input.expected_draws[0].operand, Some(6));
        assert_eq!(input.art.monster_blocks.get(&8), Some(&4), "BAR PATRON");
    }

    #[test]
    fn a_capture_with_no_combat_entry_is_refused() {
        let text = Trace::new(header(), Vec::new()).to_canonical_string();
        assert!(reel_input_from_capture("x.gbxtrace", &text, None)
            .unwrap_err()
            .same_kind(&ReplayError::NoCombatEntry));
    }
}
