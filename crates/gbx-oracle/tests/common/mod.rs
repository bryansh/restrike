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
/// behaviour. `armed-bar` carries the PC loadouts: MATHEW a long bow (43) with
/// a `1d2+6` fist, TRAVIS a short bow (44) with a `1d2+3` fist. MARK/LEDERA's
/// swords act through their record profile exactly as in the closed fist
/// captures. `sewer-fight-1` arms the FIRE KNIVES from real game data (doc
/// §45): `MON2ITM.DAX` block 1 (the `load_mob` item load, `ovr017:3298/3498`
/// — the `'ITM'` DAX-name suffix) lists ShortBow (44, unreadied at load) +
/// **7 Arrows** (readied — the data-derived ammo count) + LongSword/Shield/
/// LeatherArmor; the sword is the record's own `1d8+0` attack-1 profile, so
/// the unready fallback equals the entry profile and `ai_items_selection`
/// models the binary's sword-vs-bow re-ready exactly (bow rating 12 vs base 8).
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
            // FITTED, not derived: 10 CLOSES the capture (2749/2749) — the only
            // loadout entry that is neither binary- nor record-derivable. The
            // facing slice (§36) did NOT move his shot count: re-fitted after
            // backstab landed, 9 still → diverge @1575, 11 → @1910, 10 → CLOSED.
            ammo_count: 10,
            unarmed_profile: (1, 2, 3),
        }),
        // Sewer monster kits are DATA, not per-fight staging: `load_mob` reads
        // the same `MON2ITM.DAX` template block for every copy of a basename
        // in every sewer encounter (CMD_LoadMonster clones the item list per
        // copy), so the kits key by NAME across all sewer captures.
        (c, n) if c.starts_with("sewer-fight-") => sewer_monster_kit(n),
        _ => None,
    }
}

/// The per-basename sewer monster kits (`MON2ITM.DAX`, doc §45/§47 — blocks
/// keyed by `MON2CHA.DAX` name): only rosters with a RANGED option need a row.
/// - `FIRE KNIFE` (ITM block 1): 7 Arrows readied + ShortBow (44, unreadied)
///   over LongSword (36, readied)/Shield/Leather — the §45 kit. The sword is
///   the record's own `1d8+0` attack-1 profile, so the unready fallback equals
///   the entry profile.
/// - `THIEF` (ITM block 2): Dagger (8, readied) + LeatherArmor only — NO
///   ranged option, so `ai_items_selection` never swaps and the entry-record
///   profile stands: no row, melee-identical by construction.
/// - `TROLL`/`CROCODILE` (CHA blocks 7/8): no ITM block at all — natural
///   attacks straight from the record: no row.
fn sewer_monster_kit(name: &str) -> Option<Loadout> {
    match name {
        "FIRE KNIFE" => Some(Loadout {
            primary_type: 44,
            ammo_count: 7,
            unarmed_profile: (1, 8, 0),
        }),
        _ => None,
    }
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
/// `ovr014:0156-0166`) is [`gbx_engine::combat::calc_moves`]'s, already
/// faithful in the i32 domain for every value whose word sum stays under 256.
pub fn area_6e4_from_word(raw: i32) -> i32 {
    i32::from((raw & 0xFF) as u8 as i8)
}

/// True if any combatant in this capture carries a loadout — lets a harness
/// skip a ranged capture when the `ITEMS` file is absent (it cannot replay
/// ranged combat without the weapon table). `sewer-fight-3` (trolls +
/// crocodiles, both itemless) deliberately stays OFF this list — it replays
/// without the `ITEMS` file.
pub fn capture_has_loadout(capture: &str) -> bool {
    matches!(
        capture,
        "armed-bar.gbxtrace" | "sewer-fight-1.gbxtrace" | "sewer-fight-2.gbxtrace"
    )
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
