//! **The per-capture ranged loadouts** (doc §34.1/§45/§48) — the kits a
//! `combat_entry` snapshot cannot recover.
//!
//! Item identity and ammo counts live behind runtime far pointers the capture
//! does not chase, so a fight with readied ranged weapons supplies them here.
//! This was `gbx-oracle/tests/common/mod.rs` through M5; M6a's reel needs the
//! same rows outside a test binary, so the table moved into the library
//! unchanged (doc §4 M6a's library-ification). The **one shared place** rule is
//! the point (the §30 lesson: every replay must share input knobs, or an
//! instrument replays a different fight) — `h4_replay`, `h4_turndiff`, the
//! frontier guard and the reel all read these rows.
//!
//! D10: the `ITEMS` file and the capture bytes stay local-only; nothing here is
//! game data — only the identifications a research session earned.

use gbx_engine::combat::Loadout;

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
            ranged: Some((43, 0)),
            ammo_count: 40,
            ammo_readied: true, // game-readied (slot-B lineage, bow+arrows readied in-record)
            melee: None,
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: true,
        }),
        ("armed-bar.gbxtrace", "TRAVIS") => Some(Loadout {
            ranged: Some((44, 0)),
            ammo_readied: true, // game-readied quiver (the fitted 10)
            // FITTED, not derived: 10 CLOSES the capture (2749/2749) — the only
            // loadout entry that is neither binary- nor record-derivable. The
            // facing slice (§36) did NOT move his shot count: re-fitted after
            // backstab landed, 9 still → diverge @1575, 11 → @1910, 10 → CLOSED.
            ammo_count: 10,
            melee: None,
            unarmed_profile: (1, 2, 3),
            entry_ranged_readied: true,
        }),
        // The campaign-2 captures (doc §48/§49) carry the SAME slot-H party
        // (charup.py's kits, one game-validated save) plus sewer monsters —
        // the party kits key by PC name across both basenames, and the
        // monster kits by NAME exactly as in the sewer captures: `load_mob`
        // reads the same `MON2ITM.DAX` template block for every copy of a
        // basename in every sewer encounter (CMD_LoadMonster clones the item
        // list per copy). `cleric-fk` is sewer-fight-1's 5-FIRE-KNIFE ambush
        // script; `cleric-guildwar` is sewer-fight-2's guild-war brawl (both
        // allied and enemy THIEFs are the same rowless ITM block 2).
        // `buffed-otyugh` (campaign 3, doc §50) is the SAME slot-H party —
        // staged from slot I, a position-only teleport clone of the same
        // records/gear — so the kits key across all three basenames. The
        // OTYUGH roster is itemless (natural attacks straight from the
        // records, exactly as in sewer-fight-4): no monster row.
        ("cleric-fk.gbxtrace" | "cleric-guildwar.gbxtrace" | "buffed-otyugh.gbxtrace", n) => {
            slot_h_party_kit(n).or_else(|| sewer_monster_kit(n))
        }
        (c, n) if c.starts_with("sewer-fight-") => sewer_monster_kit(n),
        _ => None,
    }
}

/// The slot-H upgraded party kits (doc §48): from `charup.py`'s PARTY table,
/// validated in-game and re-saved (the game-written slot H is canonical). The
/// records serialize NOTHING readied (bare-hands profiles + armor AC), so
/// `entry_ranged_readied` is false and the round-0 `AI_items_selection`
/// readies each PC's best weapon — the captures' sword turns (d8 + str +
/// plus) prove the melee candidates. Class-ineligible items never enter a row
/// (the `classFlags` gate, §48): SHARA's plain sling is 1e-cleric-forbidden,
/// so her only candidates are the mace +1 and bare hands. Shields/armor are
/// slot-1/Armor items — AC rides the serialized record, no row entry.
///
/// **The arrows are UNREADIED too** (`ammo_readied: false`, §49): charup.py
/// ships every item `@0x34 = 0` and the operator readied only weapons/armor,
/// so the binary's `var_1F` (the READIED-ammo-slot test, `ovr010:1939-1952`)
/// stays false all fight — the bows can never win `AI_items_selection` and
/// MATHEW/TRAVIS fight as SWORDSMEN. Capture-proven: cleric-guildwar TRAVIS's
/// first turn readies the long sword +1 with no adjacent enemy (d8+str+plus =
/// 12 damage on [21]) where a readied-arrows model stands and shoots d6.
fn slot_h_party_kit(name: &str) -> Option<Loadout> {
    match name {
        "MATHEW" => Some(Loadout {
            ranged: Some((43, 0)), // long bow + 40 plain arrows (unreadied)
            ammo_count: 40,
            ammo_readied: false,
            melee: Some((36, 1)), // long sword +1
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        }),
        "MARK" => Some(Loadout {
            ranged: None,
            ammo_count: 0,
            ammo_readied: false,
            melee: Some((36, 2)), // long sword +2
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        }),
        "TRAVIS" => Some(Loadout {
            ranged: Some((44, 0)), // short bow + 40 plain arrows (unreadied)
            ammo_count: 40,
            ammo_readied: false,
            melee: Some((36, 1)), // long sword +1
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        }),
        "LEDERA" => Some(Loadout {
            ranged: None,
            ammo_count: 0,
            ammo_readied: false,
            melee: Some((36, 2)), // long sword +2 (elf: +1 to-hit rider)
            unarmed_profile: (1, 2, 6),
            entry_ranged_readied: false,
        }),
        "SHARA" => Some(Loadout {
            ranged: None, // plain sling (47) is cleric-forbidden — classFlags gate
            ammo_count: 0,
            ammo_readied: false,
            melee: Some((23, 1)), // mace +1
            unarmed_profile: (1, 2, 2),
            entry_ranged_readied: false,
        }),
        "PHILIPPE" => Some(Loadout {
            ranged: None,
            ammo_count: 0,
            ammo_readied: false,
            melee: Some((8, 1)), // dagger +1
            unarmed_profile: (1, 2, 1),
            entry_ranged_readied: false,
        }),
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
            ranged: Some((44, 0)),
            ammo_count: 7,
            ammo_readied: true, // MON2ITM block 1: the 7 Arrows load READIED
            // The kit's LongSword rides as the bare-hands profile (§45's
            // sword-equivalence: the record's own 1d8+0 attack-1 IS the plain
            // sword's table profile, so `melee: None` is draw-equal).
            melee: None,
            unarmed_profile: (1, 8, 0),
            entry_ranged_readied: true,
        }),
        _ => None,
    }
}

/// True if any combatant in this capture carries a loadout — lets a harness
/// skip a ranged capture when the `ITEMS` file is absent (it cannot replay
/// ranged combat without the weapon table). `sewer-fight-3` (trolls +
/// crocodiles, both itemless) deliberately stays OFF this list — it replays
/// without the `ITEMS` file.
pub fn capture_has_loadout(capture: &str) -> bool {
    matches!(
        capture,
        "armed-bar.gbxtrace"
            | "sewer-fight-1.gbxtrace"
            | "sewer-fight-2.gbxtrace"
            | "cleric-fk.gbxtrace"
            | "cleric-guildwar.gbxtrace"
            | "buffed-otyugh.gbxtrace"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_armed_bar_pcs_carry_their_bows() {
        let m = loadout_for("armed-bar.gbxtrace", "MATHEW").expect("MATHEW is armed");
        assert_eq!(m.ranged, Some((43, 0)));
        assert!(m.entry_ranged_readied);
        let t = loadout_for("armed-bar.gbxtrace", "TRAVIS").expect("TRAVIS is armed");
        assert_eq!(t.ammo_count, 10, "the capture-fitted quiver");
        assert!(loadout_for("armed-bar.gbxtrace", "MARK").is_none());
    }

    #[test]
    fn the_slot_h_kits_key_across_all_three_campaign_basenames() {
        for capture in [
            "cleric-fk.gbxtrace",
            "cleric-guildwar.gbxtrace",
            "buffed-otyugh.gbxtrace",
        ] {
            let l = loadout_for(capture, "SHARA").expect("SHARA has a kit in every campaign fight");
            assert_eq!(l.melee, Some((23, 1)), "{capture}: mace +1");
            assert_eq!(l.ranged, None, "{capture}: the sling is cleric-forbidden");
            assert!(!l.entry_ranged_readied);
        }
    }

    #[test]
    fn fire_knives_are_archers_in_every_sewer_basename() {
        for capture in [
            "sewer-fight-1.gbxtrace",
            "sewer-fight-2.gbxtrace",
            "cleric-fk.gbxtrace",
            "cleric-guildwar.gbxtrace",
        ] {
            let l = loadout_for(capture, "FIRE KNIFE").expect("{capture}: the MON2ITM block-1 kit");
            assert_eq!(l.ranged, Some((44, 0)), "{capture}");
            assert_eq!(l.ammo_count, 7, "{capture}: the data-derived quiver");
        }
        // The rowless-by-data rosters stay rowless.
        assert!(loadout_for("sewer-fight-2.gbxtrace", "THIEF").is_none());
        assert!(loadout_for("sewer-fight-3.gbxtrace", "TROLL").is_none());
        assert!(loadout_for("sewer-fight-4.gbxtrace", "OTYUGH").is_none());
    }

    #[test]
    fn the_bar_matrix_carries_no_kits_at_all() {
        for capture in [
            "combat4.gbxtrace",
            "caster-bar.gbxtrace",
            "bar-fists-2.gbxtrace",
        ] {
            for name in ["MATHEW", "TRAVIS", "BAR PATRON"] {
                assert!(loadout_for(capture, name).is_none(), "{capture}/{name}");
            }
            assert!(!capture_has_loadout(capture));
        }
    }

    #[test]
    fn the_items_gate_lists_exactly_the_ranged_captures() {
        for capture in crate::replay::sidecar::pinned_captures() {
            let needs = capture_has_loadout(capture);
            // sewer-fight-3's itemless rosters are the documented exception:
            // it replays with no `ITEMS` file at all.
            if capture == "sewer-fight-3.gbxtrace" || capture == "sewer-fight-4.gbxtrace" {
                assert!(!needs, "{capture} is itemless");
            }
        }
        assert!(capture_has_loadout("armed-bar.gbxtrace"));
        assert!(capture_has_loadout("buffed-otyugh.gbxtrace"));
    }
}
