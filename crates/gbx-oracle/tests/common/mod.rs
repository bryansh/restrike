//! Shared test support for the H4 replay harnesses (M5 armed slice, doc §34).
//!
//! The **one shared place** the per-capture ranged loadouts and the `ITEMS`
//! table loader live, so `h4_replay`, `h4_frontier_guard`, and `h4_turndiff`
//! feed the engine the SAME ranged inputs (the §30 lesson: every replay must
//! share input knobs, or an instrument replays a different fight). D10: the
//! `ITEMS` file and capture bytes are local-only; nothing here enters the repo.

#![allow(dead_code)] // each test binary uses a different subset of these.

use gbx_engine::combat::{CombatState, Loadout};
use gbx_formats::items::ItemDataTable;
use gbx_formats::save_orig::decode_char_record;
use std::path::{Path, PathBuf};

/// Load the resident `ITEMS` table from the local game dir (D10). `GBX_ITEMS_FILE`
/// overrides the default `~/goldbox-data/cotab/ITEMS`. `None` when the file is
/// absent — a caller then replays melee-only (and should skip loadout-bearing
/// captures via [`capture_has_loadout`]).
pub fn load_item_data() -> Option<ItemDataTable> {
    let path = std::env::var_os("GBX_ITEMS_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| Path::new(&h).join("goldbox-data/cotab/ITEMS"))
        })?;
    let bytes = std::fs::read(path).ok()?;
    ItemDataTable::parse(&bytes).ok()
}

/// The per-capture ranged loadout table (doc §34.1), keyed by capture basename
/// and combatant name. `None` for every combatant not listed = today's melee
/// behaviour. Only `armed-bar` carries loadouts: MATHEW a long bow (43) with a
/// `1d2+6` fist, TRAVIS a short bow (44) with a `1d2+3` fist. MARK/LEDERA's
/// swords act through their record profile exactly as in the closed fist
/// captures.
///
/// **Ammo (deviation from §34.1).** §34.1 called ammo "a free parameter — any
/// count ≥ shots-fired replays identically." The capture disproves it for
/// TRAVIS: he empties a **10-arrow** quiver mid-fight, and the depletion path
/// (`lose_item` → `GetCurrentAttackItem` false → `AI_items_selection` unreadies
/// the bow, `var_1F` false) switches him to fists and CHANGES the draw stream —
/// with ammo 40 (no depletion) the replay diverges at draw 1910 (TRAVIS shoots
/// where the capture shows him out of arrows and approaching); ammo 10 (the
/// empirically-pinned quiver — 9 depletes a turn early → diverge @1575, 11 never
/// depletes in time → diverge @1910) carries it to 2019. MATHEW fires few enough
/// (§34.1: 6) that his count is genuinely free; 40 holds.
pub fn loadout_for(capture: &str, name: &str) -> Option<Loadout> {
    match (capture, name) {
        ("armed-bar.gbxtrace", "MATHEW") => Some(Loadout {
            primary_type: 43,
            ammo_count: 40,
            unarmed_profile: (1, 2, 6),
        }),
        ("armed-bar.gbxtrace", "TRAVIS") => Some(Loadout {
            primary_type: 44,
            ammo_count: 10, // the capture-pinned quiver — TRAVIS depletes and punches
            unarmed_profile: (1, 2, 3),
        }),
        _ => None,
    }
}

/// True if any combatant in this capture carries a loadout (only `armed-bar`
/// today) — lets a harness skip a ranged capture when the `ITEMS` file is
/// absent (it cannot replay ranged combat without the weapon table).
pub fn capture_has_loadout(capture: &str) -> bool {
    capture == "armed-bar.gbxtrace"
}

/// Apply the `ITEMS` table and the per-capture loadouts to a freshly-built
/// state (doc §34.1). Sets `state.item_data`, then decodes each roster record
/// for its name and applies its loadout (if any). `records` is the roster's raw
/// `0x1A6` bytes in roster order. `None`-loadout combatants are left exactly as
/// today's engine — draw-identical.
pub fn apply_loadouts(
    state: &mut CombatState,
    capture: &str,
    records: &[&[u8]],
    items: Option<ItemDataTable>,
) {
    state.item_data = items;
    for (id, rec) in records.iter().enumerate() {
        if let Ok(r) = decode_char_record(rec) {
            if let Some(l) = loadout_for(capture, &r.name) {
                state.set_loadout(id, l);
            }
        }
    }
}
