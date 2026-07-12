//! The door-bash STR-to-success table (design doc `renderer-ui-shell.md`
//! §1.6's `bash_door` reference; M2 step 3 deliverable 5) — evidence-tagged
//! data, used through `gbx-engine`'s M3 party-predicate seam (the party
//! model itself doesn't exist until M3, so this table is exercised via a
//! test-configurable STR input this session, not a real roster).
//!
//! Derived by reading coab for behavior (D11, never copied):
//! - coab `engine/ovr015.cs` `bash_door` (`:49-224`): iterates the party in
//!   list order, stops at the first success (`if (bash_worked) break`,
//!   `:55-58`), and picks a roll from one of two tables keyed on the
//!   *door's* strength (`WallDoorFlagsGet == 3` = reinforced/unpickable,
//!   else = normal locked) and the rolling player's `stats2.Str.full` (+
//!   `Str00.cur`, the 18-percentile-strength field, for `str == 18`).
//! - coab `engine/ovr024.cs` `roll_dice` (`:586`) — the `1dN <= max` shape
//!   every table entry resolves to.
//!
//! **Table asymmetry, confirmed by direct read (flagged, not obvious from
//! either table's shape alone):** an out-of-table STR (16, or anything
//! outside `3..=25`) disables `can_bash_door` on a **reinforced** door
//! (`ovr015.cs:118-121` and the same-shaped catch-all) but leaves it enabled
//! on a **normal** locked door (no `can_bash_door = false` anywhere in that
//! branch) — the original lets a normal-door bash attempt keep re-offering
//! "Bash" forever for an out-of-range player, while a reinforced door's
//! attempt permanently removes the option. [`BashOutcome::NoEffect`]'s
//! `disables_can_bash` carries this exactly.

/// The door's resistance, matching `WallDoorFlagsGet`'s door-state field
/// (design doc §1.6): state `2` = normal locked, `3` = reinforced/unpickable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorStrength {
    Normal,
    Reinforced,
}

/// One player's bash attempt against one door, per [`bash_outcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashOutcome {
    /// No roll happens — an out-of-table STR. `disables_can_bash` is the
    /// table asymmetry this module's doc comment flags.
    NoEffect { disables_can_bash: bool },
    /// Success, no roll needed.
    Auto,
    /// Success iff `roll_dice(die_size, 1) <= max_success` (`1dN <= max`).
    Roll { die_size: u8, max_success: u8 },
}

/// `bash_door`'s STR-to-outcome table (`ovr015.cs:49-224`). `str_full` is
/// `stats2.Str.full` (`3..=25`, `18` meaning "18/xx exceptional strength");
/// `str_percentile` is `Str00.cur` (`0..=100`), meaningful only when
/// `str_full == 18`.
pub fn bash_outcome(strength: DoorStrength, str_full: u8, str_percentile: u8) -> BashOutcome {
    match strength {
        DoorStrength::Reinforced => reinforced(str_full, str_percentile),
        DoorStrength::Normal => normal(str_full, str_percentile),
    }
}

/// Table A — reinforced/unpickable door (`ovr015.cs:~90-141`).
fn reinforced(str_full: u8, pct: u8) -> BashOutcome {
    match str_full {
        18 => match pct {
            91..=99 => BashOutcome::Roll {
                die_size: 6,
                max_success: 1,
            },
            100 => BashOutcome::Roll {
                die_size: 6,
                max_success: 2,
            },
            _ => BashOutcome::NoEffect {
                disables_can_bash: true,
            },
        },
        19 | 20 => BashOutcome::Roll {
            die_size: 6,
            max_success: 3,
        },
        21 | 22 => BashOutcome::Roll {
            die_size: 6,
            max_success: 4,
        },
        23 => BashOutcome::Roll {
            die_size: 6,
            max_success: 5,
        },
        24 => BashOutcome::Roll {
            die_size: 8,
            max_success: 7,
        },
        25 => BashOutcome::Auto,
        _ => BashOutcome::NoEffect {
            disables_can_bash: true,
        },
    }
}

/// Table B — normal locked door (`ovr015.cs:~144-224`). STR `16` and any
/// value outside `3..=25` is the documented asymmetry: no roll, and
/// (unlike the reinforced table) `can_bash_door` is left untouched.
fn normal(str_full: u8, pct: u8) -> BashOutcome {
    match str_full {
        3..=7 => BashOutcome::Roll {
            die_size: 6,
            max_success: 1,
        },
        8..=15 => BashOutcome::Roll {
            die_size: 6,
            max_success: 2,
        },
        17 => BashOutcome::Roll {
            die_size: 6,
            max_success: 3,
        },
        18 => match pct {
            0..=50 => BashOutcome::Auto,
            51..=99 => BashOutcome::Roll {
                die_size: 6,
                max_success: 4,
            },
            _ => BashOutcome::Roll {
                die_size: 6,
                max_success: 5,
            },
        },
        19 | 20 => BashOutcome::Roll {
            die_size: 8,
            max_success: 7,
        },
        21 => BashOutcome::Roll {
            die_size: 10,
            max_success: 9,
        },
        22 | 23 => BashOutcome::Roll {
            die_size: 12,
            max_success: 11,
        },
        24 => BashOutcome::Roll {
            die_size: 20,
            max_success: 19,
        },
        25 => BashOutcome::Auto,
        _ => BashOutcome::NoEffect {
            disables_can_bash: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reinforced_door_str_25_is_automatic() {
        assert_eq!(
            bash_outcome(DoorStrength::Reinforced, 25, 0),
            BashOutcome::Auto
        );
    }

    #[test]
    fn reinforced_door_str_18_low_percentile_disables_bash() {
        assert_eq!(
            bash_outcome(DoorStrength::Reinforced, 18, 50),
            BashOutcome::NoEffect {
                disables_can_bash: true
            }
        );
    }

    #[test]
    fn reinforced_door_str_18_high_percentile_rolls() {
        assert_eq!(
            bash_outcome(DoorStrength::Reinforced, 18, 95),
            BashOutcome::Roll {
                die_size: 6,
                max_success: 1
            }
        );
        assert_eq!(
            bash_outcome(DoorStrength::Reinforced, 18, 100),
            BashOutcome::Roll {
                die_size: 6,
                max_success: 2
            }
        );
    }

    #[test]
    fn reinforced_door_out_of_table_str_disables_bash() {
        assert_eq!(
            bash_outcome(DoorStrength::Reinforced, 16, 0),
            BashOutcome::NoEffect {
                disables_can_bash: true
            }
        );
        assert_eq!(
            bash_outcome(DoorStrength::Reinforced, 3, 0),
            BashOutcome::NoEffect {
                disables_can_bash: true
            }
        );
    }

    #[test]
    fn normal_door_str_16_does_not_disable_bash() {
        assert_eq!(
            bash_outcome(DoorStrength::Normal, 16, 0),
            BashOutcome::NoEffect {
                disables_can_bash: false
            }
        );
    }

    #[test]
    fn normal_door_low_str_rolls_1d6_for_1() {
        assert_eq!(
            bash_outcome(DoorStrength::Normal, 5, 0),
            BashOutcome::Roll {
                die_size: 6,
                max_success: 1
            }
        );
    }

    #[test]
    fn normal_door_str_17_rolls_1d6_for_3() {
        assert_eq!(
            bash_outcome(DoorStrength::Normal, 17, 0),
            BashOutcome::Roll {
                die_size: 6,
                max_success: 3
            }
        );
    }

    #[test]
    fn normal_door_str_18_low_percentile_is_automatic() {
        assert_eq!(
            bash_outcome(DoorStrength::Normal, 18, 30),
            BashOutcome::Auto
        );
    }

    #[test]
    fn normal_door_str_25_is_automatic() {
        assert_eq!(bash_outcome(DoorStrength::Normal, 25, 0), BashOutcome::Auto);
    }

    #[test]
    fn normal_door_str_24_rolls_1d20_for_19() {
        assert_eq!(
            bash_outcome(DoorStrength::Normal, 24, 0),
            BashOutcome::Roll {
                die_size: 20,
                max_success: 19
            }
        );
    }
}
