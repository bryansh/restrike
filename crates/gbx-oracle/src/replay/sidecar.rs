//! **The versioned per-capture sidecar** (`docs/design/combat-visualizer.md`
//! §4 M6a) — the inputs a `.gbxtrace` cannot carry, pinned per capture.
//!
//! A capture records what the original *drew*. Two classes of input never made
//! it into the snapshot, and a replay is not reproducible without them:
//!
//! 1. **Knobs.** `map_direction` / `area2.field_58C` / `area2.field_6E4` ARE
//!    emitted by modern staging hooks (and the capture's value always wins), but
//!    the '2' auto-magic presses (doc §38) and the "Continue Battle:" answers
//!    (doc §48) are keypresses with no hook at all — they have been hand-pinned
//!    in the frontier guard's `PINS` manifest since M5, and this is where they
//!    now live.
//! 2. **Icon assignments.** `combat_entry` carries no monster CPIC ids:
//!    LOADMONSTER's third operand is read on the live path only
//!    (`format.rs:430-437`). Party icons need no pin — `head_icon` /
//!    `weapon_icon` / `icon_colours` all ride in the record — but a monster's
//!    picture does, so the 15 closed captures pin theirs exactly as their
//!    loadouts are pinned.
//!
//! **How the CPIC pins were derived** (they are not guesses). Every one of the
//! 15 captures is an area-2 fight. `CMD_LoadMonster` (`ovr003.cs:238-297`) reads
//! three ECL operands — monster id, copy count, CPIC block — loads the block
//! into `gbl.monster_icon_id` (which starts at 8, `ovr008.cs:98`) and stamps
//! that slot onto every copy's `icon_id`. So:
//!
//! - the **slot** comes from the record itself (`icon_id` @0x143 — the captures
//!   show 8 and 9 for their monster groups, and 0..5 for the party), and
//! - the **block** comes from the ECL, read straight off `ECL2.DAX`'s
//!   `LOAD MONSTER imm,imm,imm` instructions: every monster in these captures
//!   is loaded with `cpic == monster id` (FIRE KNIFE 1, THIEF 2, BAR PATRON 4,
//!   TROLL 7, CROCODILE 8, OTYUGH 9, NEO-OTYUGH 10 — the same ids their
//!   `MON2CHA.DAX` template blocks carry).
//!
//! Independently cross-checked against art shape: `CPIC2` block 7 (TROLL) and
//! block 10 (NEO-OTYUGH) are 48 rows tall and block 8 (CROCODILE) is 6 columns
//! wide — matching exactly the `field_DE` footprints (0x82 = origin+south,
//! 0x83 = origin+east) those three records carry (doc §47.6). A wrong pin would
//! have to coincide on both the id and the footprint.
//!
//! **Standing hook TODO** (dosbox-side, not this slice): future `combat_entry`
//! emissions should carry each combatant's `icon_id` **and** the CPIC block
//! LOADMONSTER used, at which point new captures need no icon pins at all and
//! this table stops growing. The knob half stays — keypresses have no snapshot.
//!
//! ## The format
//!
//! Same posture as the `.gbxtrace` reader/writer split (D-OR3): the writer is
//! **canonical** (fixed field order, compact, integers only — byte-hashable),
//! the reader is **liberal** (unknown fields are ignored, so a future field
//! doesn't break an old reader). The one thing that is NOT liberal is
//! [`SIDECAR_VERSION`]: a version this reader doesn't know is rejected loudly
//! rather than migrated (the D-SAVE2 reject-not-migrate rule — a silently
//! misread input knob is exactly the class of bug the guard exists to prevent).

use gbx_engine::combat::reel::MONSTER_FIRST_ICON_SLOT;
use std::collections::BTreeMap;
use std::fmt;

/// The sidecar format version. Bump it when a field's **meaning** changes;
/// additive optional fields do not need a bump (the reader is liberal).
pub const SIDECAR_VERSION: u32 = 1;

/// Everything wrong a sidecar can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarError {
    /// The JSON didn't parse, or a field had the wrong type.
    Malformed(String),
    /// A version this reader does not know (reject, never migrate).
    UnknownVersion { found: u32, expected: u32 },
    /// An icon pin named a slot below the monster range — party icons come
    /// from the record and must never be pinned.
    PartyIconPinned { slot: u8 },
    /// Two pins claim the same icon slot.
    DuplicateIconSlot { slot: u8 },
}

impl fmt::Display for SidecarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SidecarError::Malformed(m) => write!(f, "malformed capture sidecar: {m}"),
            SidecarError::UnknownVersion { found, expected } => write!(
                f,
                "capture sidecar version {found} is not readable by this build (expected {expected}) \
                 — sidecars are rejected, never migrated"
            ),
            SidecarError::PartyIconPinned { slot } => write!(
                f,
                "icon slot {slot} is a PARTY slot (0..8): party icons derive from the record's \
                 head_icon/weapon_icon/icon_colours and must not be pinned"
            ),
            SidecarError::DuplicateIconSlot { slot } => {
                write!(f, "two icon pins claim slot {slot}")
            }
        }
    }
}

impl std::error::Error for SidecarError {}

/// The knob half: the inputs no hook records.
///
/// `map_direction` is here as the **fallback** only — a capture that emits its
/// own heading always wins (see [`super::knobs_for`]).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarKnobs {
    /// `gbl.mapDirection` fallback for captures predating the emission (doc
    /// §29). Default 2 (E).
    #[serde(default = "default_map_direction")]
    pub map_direction: u8,
    /// `gbl.AutoPCsCastMagic` at combat entry (doc §33).
    #[serde(default)]
    pub auto_cast: bool,
    /// Mid-fight '2' presses as 0-based global turn ordinals (doc §38).
    #[serde(default)]
    pub auto_cast_toggles: Vec<u32>,
    /// "Continue Battle:" occurrences answered 'Y', 0-based (doc §48).
    #[serde(default)]
    pub continue_battle: Vec<u16>,
    /// `area2.field_6E4` in the ENGINE domain (doc §45/§47) — the fallback for
    /// captures predating the trio emission.
    #[serde(default)]
    pub area_field_6e4: i32,
}

fn default_map_direction() -> u8 {
    2
}

impl Default for SidecarKnobs {
    fn default() -> Self {
        SidecarKnobs {
            map_direction: default_map_direction(),
            auto_cast: false,
            auto_cast_toggles: Vec::new(),
            continue_battle: Vec::new(),
            area_field_6e4: 0,
        }
    }
}

/// One monster type's icon pin: which combat-icon slot LOADMONSTER filled, and
/// with which `CPIC{area}` block.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MonsterIconPin {
    /// `player.icon_id` — the slot, 8 or above. Read it out of any record in
    /// that monster group (@0x143).
    pub slot: u8,
    /// LOADMONSTER's third operand.
    pub cpic_block: u8,
    /// The monster, for whoever reads this table. Never load-bearing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

/// The art half.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarArt {
    /// `gbl.game_area` — picks `CPIC{area}.DAX`.
    #[serde(default = "default_cpic_area")]
    pub cpic_area: u8,
    /// `SetupGroundTiles`' fork: DUNGCOM (true) or WILDCOM.
    #[serde(default = "default_in_dungeon")]
    pub in_dungeon: bool,
    /// One pin per monster icon slot the roster uses.
    #[serde(default)]
    pub monster_icons: Vec<MonsterIconPin>,
}

fn default_cpic_area() -> u8 {
    2
}
fn default_in_dungeon() -> bool {
    true
}

impl Default for SidecarArt {
    fn default() -> Self {
        SidecarArt {
            cpic_area: default_cpic_area(),
            in_dungeon: default_in_dungeon(),
            monster_icons: Vec::new(),
        }
    }
}

/// One capture's sidecar.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CaptureSidecar {
    pub version: u32,
    /// The capture basename this pins, e.g. `"combat4.gbxtrace"`.
    pub capture: String,
    #[serde(default)]
    pub knobs: SidecarKnobs,
    #[serde(default)]
    pub art: SidecarArt,
}

impl CaptureSidecar {
    /// An all-defaults sidecar for a capture with no pinned row — the honest
    /// "nothing known" input: heading 2, magic off, no schedules, area 2,
    /// dungeon tiles, no monster icons (which will fail loudly at art-load time
    /// if the roster actually has monsters).
    pub fn defaults_for(capture: &str) -> Self {
        CaptureSidecar {
            version: SIDECAR_VERSION,
            capture: capture.to_string(),
            knobs: SidecarKnobs::default(),
            art: SidecarArt::default(),
        }
    }

    /// The **canonical** encoding: compact JSON, fixed field order, integers
    /// only — byte-deterministic and hashable, like a `.gbxtrace` line.
    pub fn to_canonical_json(&self) -> String {
        serde_json::to_string(self).expect("a sidecar is always serializable")
    }

    /// The **liberal** reader: unknown fields are ignored (so a newer emitter's
    /// extra diagnostics don't break this build), but an unknown
    /// [`SIDECAR_VERSION`] is rejected rather than migrated.
    pub fn parse(text: &str) -> Result<Self, SidecarError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| SidecarError::Malformed(e.to_string()))?;
        let version = value
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| SidecarError::Malformed("missing integer field `version`".into()))?
            as u32;
        if version != SIDECAR_VERSION {
            return Err(SidecarError::UnknownVersion {
                found: version,
                expected: SIDECAR_VERSION,
            });
        }
        let sidecar: CaptureSidecar =
            serde_json::from_value(value).map_err(|e| SidecarError::Malformed(e.to_string()))?;
        sidecar.validate()?;
        Ok(sidecar)
    }

    /// Slot → CPIC block, the shape the engine's `ReelArt` wants.
    pub fn monster_blocks(&self) -> BTreeMap<u8, u8> {
        self.art
            .monster_icons
            .iter()
            .map(|p| (p.slot, p.cpic_block))
            .collect()
    }

    /// Rejects pins that cannot be right: a party slot, or two claims on one
    /// slot. Cheap, and it turns a typo into a located error instead of a
    /// character wearing a crocodile.
    pub fn validate(&self) -> Result<(), SidecarError> {
        let mut seen: Vec<u8> = Vec::new();
        for pin in &self.art.monster_icons {
            if pin.slot < MONSTER_FIRST_ICON_SLOT {
                return Err(SidecarError::PartyIconPinned { slot: pin.slot });
            }
            if seen.contains(&pin.slot) {
                return Err(SidecarError::DuplicateIconSlot { slot: pin.slot });
            }
            seen.push(pin.slot);
        }
        Ok(())
    }
}

/// One row of the committed table, in a `const`-friendly shape.
struct Row {
    capture: &'static str,
    map_direction: u8,
    auto_cast: bool,
    auto_cast_toggles: &'static [u32],
    continue_battle: &'static [u16],
    area_field_6e4: i32,
    cpic_area: u8,
    in_dungeon: bool,
    /// `(icon slot, CPIC block, monster name)`.
    monster_icons: &'static [(u8, u8, &'static str)],
}

/// **The committed sidecar table** — the 15 closed captures (doc §44–§50).
///
/// The knob columns are the same values the frontier guard's `PINS` manifest
/// carried before this table existed; `PINS` now reads them from here, so the
/// guard remains the referee for every one of them. The icon columns are new
/// (M6a), derived as this module's header describes.
///
/// Every capture in this table is an area-2 (Tilverton / the sewers beneath it)
/// indoor fight, so `cpic_area` is 2 and `in_dungeon` true throughout.
const SIDECARS: &[Row] = &[
    // --- the bar/terrain matrix (doc §44): six PCs vs BAR PATRONs -----------
    Row {
        capture: "combat4.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        capture: "combat3+terrain4.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        capture: "combat2+terrain4.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        capture: "combat+terrain4.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        capture: "bar-rout-58c50.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        capture: "armed-bar.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        // The §38 toggle pin, DERIVED: entry-false + a '2' press at global
        // turn ordinal 16 (PHILIPPE's round-2 turn head).
        capture: "caster-bar.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[16],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    Row {
        capture: "bar-fists-2.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 4, "BAR PATRON")],
    },
    // --- campaign 1, the sewers (doc §45/§47) -------------------------------
    Row {
        // 5 FIRE KNIVES. The capture predates the 6E4 emission, so its −3
        // (three independent round-5 chase walks) rides here.
        capture: "sewer-fight-1.gbxtrace",
        map_direction: 0,
        auto_cast: false,
        auto_cast_toggles: &[2],
        continue_battle: &[],
        area_field_6e4: -3,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 1, "FIRE KNIFE")],
    },
    Row {
        // The 23-combatant guild war: 2 FIRE KNIVES + 11 enemy THIEFs + 4
        // ALLIED team-0 THIEFs. Both thief groups carry `icon_id` 9 in their
        // records — one loaded icon, one picture, exactly as the original drew
        // it; the panel's name colour is what tells friend from foe.
        capture: "sewer-fight-2.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[17],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 1, "FIRE KNIFE"), (9, 2, "THIEF")],
    },
    Row {
        // 4 TROLLS (size 2, the 48-row CPIC) + 7 CROCODILES (size 3, the
        // 6-column CPIC).
        capture: "sewer-fight-3.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[16],
        continue_battle: &[],
        area_field_6e4: -3,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 7, "TROLL"), (9, 8, "CROCODILE")],
    },
    Row {
        // 4 OTYUGHS + 1 NEO-OTYUGH (size 2).
        capture: "sewer-fight-4.gbxtrace",
        map_direction: 4,
        auto_cast: false,
        auto_cast_toggles: &[],
        continue_battle: &[],
        area_field_6e4: -3,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 9, "OTYUGH"), (9, 10, "NEO-OTYUGH")],
    },
    // --- campaign 2, the slot-H party (doc §48/§49) -------------------------
    Row {
        // Bryan answered the round-3-end "Continue Battle:" prompt 'Y' once.
        capture: "cleric-fk.gbxtrace",
        map_direction: 4,
        auto_cast: false,
        auto_cast_toggles: &[0],
        continue_battle: &[0],
        area_field_6e4: -3,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 1, "FIRE KNIFE")],
    },
    Row {
        capture: "cleric-guildwar.gbxtrace",
        map_direction: 2,
        auto_cast: false,
        auto_cast_toggles: &[3],
        continue_battle: &[],
        area_field_6e4: 0,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 1, "FIRE KNIFE"), (9, 2, "THIEF")],
    },
    // --- campaign 3, the camp-buffed otyugh fight (doc §50) -----------------
    Row {
        capture: "buffed-otyugh.gbxtrace",
        map_direction: 6,
        auto_cast: false,
        auto_cast_toggles: &[0],
        continue_battle: &[],
        area_field_6e4: -3,
        cpic_area: 2,
        in_dungeon: true,
        monster_icons: &[(8, 9, "OTYUGH")],
    },
];

impl From<&Row> for CaptureSidecar {
    fn from(row: &Row) -> Self {
        CaptureSidecar {
            version: SIDECAR_VERSION,
            capture: row.capture.to_string(),
            knobs: SidecarKnobs {
                map_direction: row.map_direction,
                auto_cast: row.auto_cast,
                auto_cast_toggles: row.auto_cast_toggles.to_vec(),
                continue_battle: row.continue_battle.to_vec(),
                area_field_6e4: row.area_field_6e4,
            },
            art: SidecarArt {
                cpic_area: row.cpic_area,
                in_dungeon: row.in_dungeon,
                monster_icons: row
                    .monster_icons
                    .iter()
                    .map(|&(slot, cpic_block, name)| MonsterIconPin {
                        slot,
                        cpic_block,
                        name: name.to_string(),
                    })
                    .collect(),
            },
        }
    }
}

/// The committed sidecar for `capture` (a basename), if one is pinned.
pub fn sidecar_for(capture: &str) -> Option<CaptureSidecar> {
    SIDECARS
        .iter()
        .find(|row| row.capture == capture)
        .map(CaptureSidecar::from)
}

/// The committed sidecar, or an all-defaults one — what a host wants when it
/// would rather show *something* than refuse an unpinned capture.
pub fn sidecar_or_default(capture: &str) -> CaptureSidecar {
    sidecar_for(capture).unwrap_or_else(|| CaptureSidecar::defaults_for(capture))
}

/// Every pinned capture basename, table order.
pub fn pinned_captures() -> impl Iterator<Item = &'static str> {
    SIDECARS.iter().map(|row| row.capture)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_holds_the_fifteen_closed_captures() {
        assert_eq!(pinned_captures().count(), 15);
        for capture in pinned_captures() {
            assert!(capture.ends_with(".gbxtrace"), "{capture} is a basename");
            sidecar_for(capture)
                .expect("every listed capture resolves")
                .validate()
                .expect("every committed row validates");
        }
    }

    #[test]
    fn every_row_pins_at_least_one_monster_icon() {
        // A capture with no monster icons would render its enemies as empty
        // slots — a silent, watchable-looking failure. There is no such fight
        // among the fifteen.
        for capture in pinned_captures() {
            let s = sidecar_for(capture).unwrap();
            assert!(
                !s.art.monster_icons.is_empty(),
                "{capture} pins no monster icon"
            );
        }
    }

    #[test]
    fn a_canonical_round_trip_is_byte_stable() {
        let s = sidecar_for("sewer-fight-3.gbxtrace").unwrap();
        let json = s.to_canonical_json();
        let back = CaptureSidecar::parse(&json).expect("our own writer parses");
        assert_eq!(back, s);
        assert_eq!(back.to_canonical_json(), json, "encoding is deterministic");
        // Integers only, no floats, no insignificant whitespace.
        assert!(!json.contains(' '), "canonical output is compact");
        assert!(json.contains("\"version\":1"));
    }

    #[test]
    fn the_reader_is_liberal_about_unknown_fields() {
        // Same posture as the `.gbxtrace` reader: a newer emitter's extra
        // diagnostics must not break an older build.
        let json = r#"{"version":1,"capture":"x.gbxtrace","tomorrows_field":[1,2],
            "knobs":{"map_direction":6,"whatever":true},
            "art":{"cpic_area":3,"monster_icons":[{"slot":8,"cpic_block":9,"extra":1}]}}"#;
        let s = CaptureSidecar::parse(json).expect("unknown fields are ignored");
        assert_eq!(s.knobs.map_direction, 6);
        assert_eq!(s.art.cpic_area, 3);
        assert_eq!(s.monster_blocks(), [(8u8, 9u8)].into_iter().collect());
    }

    #[test]
    fn absent_fields_take_the_documented_defaults() {
        let s = CaptureSidecar::parse(r#"{"version":1,"capture":"x.gbxtrace"}"#).unwrap();
        assert_eq!(s.knobs.map_direction, 2);
        assert_eq!(s.knobs.area_field_6e4, 0);
        assert!(!s.knobs.auto_cast);
        assert!(s.knobs.auto_cast_toggles.is_empty());
        assert!(s.knobs.continue_battle.is_empty());
        assert_eq!(s.art.cpic_area, 2);
        assert!(s.art.in_dungeon);
        assert!(s.art.monster_icons.is_empty());
        assert_eq!(s, CaptureSidecar::defaults_for("x.gbxtrace"));
    }

    #[test]
    fn an_unknown_version_is_rejected_not_migrated() {
        let err = CaptureSidecar::parse(r#"{"version":99,"capture":"x.gbxtrace"}"#).unwrap_err();
        assert_eq!(
            err,
            SidecarError::UnknownVersion {
                found: 99,
                expected: SIDECAR_VERSION
            }
        );
        assert!(err.to_string().contains("never migrated"));
    }

    #[test]
    fn a_missing_version_is_malformed() {
        let err = CaptureSidecar::parse(r#"{"capture":"x.gbxtrace"}"#).unwrap_err();
        assert!(matches!(err, SidecarError::Malformed(_)));
    }

    #[test]
    fn garbage_is_a_located_error_not_a_panic() {
        assert!(matches!(
            CaptureSidecar::parse("not json at all").unwrap_err(),
            SidecarError::Malformed(_)
        ));
        assert!(matches!(
            CaptureSidecar::parse(r#"{"version":1,"capture":7}"#).unwrap_err(),
            SidecarError::Malformed(_)
        ));
    }

    #[test]
    fn a_party_slot_or_a_duplicate_slot_is_refused() {
        let party =
            r#"{"version":1,"capture":"x","art":{"monster_icons":[{"slot":3,"cpic_block":4}]}}"#;
        assert_eq!(
            CaptureSidecar::parse(party).unwrap_err(),
            SidecarError::PartyIconPinned { slot: 3 }
        );
        let dup = r#"{"version":1,"capture":"x","art":{"monster_icons":
            [{"slot":8,"cpic_block":4},{"slot":8,"cpic_block":9}]}}"#;
        assert_eq!(
            CaptureSidecar::parse(dup).unwrap_err(),
            SidecarError::DuplicateIconSlot { slot: 8 }
        );
    }

    #[test]
    fn an_unpinned_capture_falls_back_to_defaults() {
        assert!(sidecar_for("never-staged.gbxtrace").is_none());
        let s = sidecar_or_default("never-staged.gbxtrace");
        assert_eq!(s.capture, "never-staged.gbxtrace");
        assert!(s.art.monster_icons.is_empty());
    }
}
