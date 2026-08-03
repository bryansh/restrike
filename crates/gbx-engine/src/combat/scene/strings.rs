//! **§1.5's string inventory**, transcribed from the cited sites — the words
//! the combat screen prints, and the two tables (`SpellNames`, the health
//! statuses) it prints them from.
//!
//! Everything here is a `&'static str` with the coab line that produced it.
//! The scene composes messages from these plus the roster's names (D-CV2: the
//! combat core stays presentation-free — no message text is ever built inside
//! `CombatState`).
//!
//! A few of these have no consumer *yet* — "Flee:", "Escape is blocked",
//! "Magic On"/"Magic Off", "Range = " belong to the M6c manual surface, and
//! "Your Teammate is Dying" belongs to the round-end `bandage(false)` display
//! scan D-CV2 leaves for whoever models it. They are transcribed together
//! because they were read together; a second pass over `ovr009`/`ovr014` to
//! re-find one string later is the waste this module exists to prevent.
//!
//! Derived by reading coab for behavior (D11, never copied) — sites inline.

// --- the attack sequence (`DisplayAttackMessage`, `ovr014.cs:113-223`) ------

/// `AttackType.Normal`/`Behind` (`ovr014.cs:127`).
pub const ATTACKS: &str = "Attacks";
/// `AttackType.Backstab` (`ovr014.cs:119`).
pub const BACKSTABS: &str = "-Backstabs-";
/// `AttackType.Slay` (`ovr014.cs:123`) — the held-target kill.
pub const SLAYS_HELPLESS: &str = "slays helpless";
/// The damage line's prefix for `AttackType.Behind` (`ovr014.cs:138`). The
/// trailing space is the original's.
pub const FROM_BEHIND: &str = "(from behind) ";
/// `AttackType.Slay`'s damage line (`ovr014.cs:149`) — it replaces the
/// "Hitting for …" text rather than prefixing it.
pub const ONE_CRUEL_BLOW: &str = "with one cruel blow";
/// The miss line (`ovr014.cs:172`), appended after any "(from behind) ".
pub const AND_MISSES: &str = "and Misses";
/// The removal line (`ovr014.cs:191`), printed under the target's name.
pub const GOES_DOWN: &str = "goes down";
/// `Status.dying`'s follow-up (`ovr014.cs:196`).
pub const AND_IS_DYING: &str = "and is Dying";
/// `Status.dead`/`stoned`/`gone`'s follow-up (`ovr014.cs:203`); also
/// `KillPlayer`'s own message (`ovr014.cs:2406`).
pub const IS_KILLED: &str = "is killed";

/// `"Hitting for " + N + (" point " | " points ") + "of damage"`
/// (`ovr014.cs:153-164`) — singular at exactly 1, and note the space *after*
/// the unit rather than before "of".
pub fn hitting_for(damage: i32) -> String {
    let unit = if damage == 1 { " point " } else { " points " };
    format!("Hitting for {damage}{unit}of damage")
}

// --- casting (`ovr014.cs:1416`, `ovr023.cs:3114-3123`) ---------------------

/// The queued-cast announcement (`ovr014.cs:1416`).
pub const BEGINS_CASTING: &str = "Begins Casting";
/// The resolution announcement (`DisplayCaseSpellText`, `ovr023.cs:3119`) —
/// the combat arm prints this, never the caller's "casts"/"miscasts" verb,
/// which is the exploration arm's.
pub const CASTS_A_SPELL: &str = "Casts a Spell";
/// The status line's prefix (`ovr023.cs:3122`); the name follows with no
/// separating space.
pub const SPELL_PREFIX: &str = "Spell:";
/// A QuickFight cast that found no target (`ovr023.cs:795`) — prompt line.
pub const SPELL_ABORTED: &str = "Spell Aborted";

// --- morale, flight, removal ----------------------------------------------

/// `FleeCheck_001`'s surrender branch (`ovr010.cs:804`), passed to
/// `RemoveFromCombat` as its message.
pub const SURRENDERS: &str = "Surrenders";
/// The post-check morale display (`ovr010.cs:46`).
pub const FLEES_IN_PANIC: &str = "flees in panic";
/// The already-fleeing branch inside the check (`ovr010.cs:770`).
pub const IS_FORCED_TO_FLEE: &str = "is forced to flee";
/// `flee_battle`'s successful escape (`ovr014.cs:451`), again a
/// `RemoveFromCombat` message.
pub const GOT_AWAY: &str = "Got Away";
/// The blocked-escape prompt-line message (`ovr014.cs:455`).
pub const ESCAPE_IS_BLOCKED: &str = "Escape is blocked";
/// The flee confirmation (`ovr009.cs:510`) — a `yes_no`, so it key-blocks
/// (M6c).
pub const FLEE_PROMPT: &str = "Flee:";

// --- the turn's small change ----------------------------------------------

/// `guarding(player)` (`ovr025.cs:1338`) — prompt line.
pub const GUARDING: &str = "Guarding";
/// `bandage(true)`'s panel message (`ovr025.cs:1645`).
pub const IS_BANDAGED: &str = "is bandaged";
/// A cure spell's `MagicAttackDisplay` text (`ovr023.cs:2219,2808`) — the
/// heal spells that use the stars burst, none of them modeled yet.
pub const IS_HEALED: &str = "is Healed";
/// `DescribeHealing` (`ovr025.cs:1246-1259`), the message `SpellCureLight`
/// actually prints: full at `hp_current == hp_max`, partial below it.
pub const IS_FULLY_HEALED: &str = "is fully healed";
pub const IS_PARTIALLY_HEALED: &str = "is partially healed";
/// Turn-undead's `MagicAttackDisplay` text (`ovr014.cs:655`).
pub const IS_TURNED: &str = "is turned";

// --- round boundaries and the manual surface ------------------------------

/// `BattleSetup`'s opening line (`ovr011.cs:1184`) — printed straight onto
/// the prompt row with no `GameDelay` of its own.
pub const A_BATTLE_BEGINS: &str = "A battle begins...";
/// The round-end display scan (`ovr009.cs:388`) — prompt line.
pub const TEAMMATE_IS_DYING: &str = "Your Teammate is Dying";
/// The round-end prompt (`ovr009.cs:407`) — a `yes_no`.
pub const CONTINUE_BATTLE: &str = "Continue Battle:";
/// The auto-magic toggle's two prompt-line messages (`ovr009.cs:259-263`,
/// `ovr010.cs:724`).
pub const MAGIC_ON: &str = "Magic On";
pub const MAGIC_OFF: &str = "Magic Off";

/// The Aim view's status line (`ovr014.cs:1760,1895`) — note the **two**
/// trailing spaces, which is how the original erases a wider previous value.
pub fn range_status(range: i32) -> String {
    format!("Range = {range}  ")
}

// --- ★ the M6c manual surface (doc §9) ------------------------------------

/// The movement loop's prompt (`sub_33B26`, `ovr009.cs:436`) — `N` is
/// `actions.move / 2`, the WHOLE moves left, because the budget is kept in
/// halves (§45). The trailing space is the original's.
pub fn move_left_prompt(whole_moves: i32) -> String {
    format!("Move/Attack, Move Left = {whole_moves} ")
}

/// The movement loop's unaffordable-step message (`ovr009.cs:533`).
pub const CANT_GO_THERE: &str = "can't go there";
/// `sub_33F03`'s refusal when a pure-ranged weapon is readied
/// (`ovr009.cs:596`).
pub const NOT_WITH_THAT_WEAPON: &str = "Not with that weapon";
/// The manual targeting loop's duplicate-pick message (`ovr014.cs:1344`) — the
/// player's arm only; QuickFight silently spends the pick instead.
pub const ALREADY_BEEN_TARGETED: &str = "Already been targeted";

/// Aim's prompt (`aim_sub_menu`'s `displayInput(…, "Aim:")`, `ovr014.cs:1803`).
pub const AIM_PROMPT: &str = "Aim:";
/// Aim's words either side of the conditional `Target` (`ovr014.cs:1793`).
pub const AIM_WORDS_HEAD: &str = "Next Prev Manual ";
pub const AIM_WORDS_TAIL: &str = "Center Exit";
/// The conditional word itself, with the original's trailing space.
pub const AIM_TARGET_WORD: &str = "Target ";
/// The free cursor's prompt (`Target`, `ovr014.cs:1985`).
pub const CURSOR_PROMPT: &str = "(Use Cursor keys) ";
/// The free cursor's unconditional words (`ovr014.cs:1976-1983`).
pub const CURSOR_WORDS: &str = "Center Exit";

/// `set_gamespeed`'s prompt (`ovr009.cs:678`) — the space before the colon is
/// the original's.
pub fn game_speed_prompt(speed: u8) -> String {
    format!("GameSpeed ({speed}) :")
}

/// `set_gamespeed`'s word list (`ovr009.cs:679-690`): Slower only below 9,
/// Faster only above 0, and the leading space is the original's. (Higher
/// `game_speed_var` = longer delays, so **Slower** is the one that increments.)
pub fn game_speed_words(speed: u8) -> String {
    let mut out = String::from(" ");
    if speed < 9 {
        out.push_str("Slower ");
    }
    if speed > 0 {
        out.push_str("Faster ");
    }
    out.push_str("Exit");
    out
}

/// `yes_no`'s words (`ovr027.cs:684`); the prompt beside them is the caller's
/// ([`FLEE_PROMPT`], [`CONTINUE_BATTLE`]).
pub const YES_NO: &str = "Yes No";

/// `ovr023.SpellNames` (`ovr023.cs:10-105`), the table `DisplayCaseSpellText`
/// prints from — 101 rows, indexed by spell id.
///
/// Transcribed whole rather than lazily (the three rows [`spell_entry`] models
/// today) because it is a name table with a real consumer *now* — §1.5's
/// "Spell:<name>" status line — and M6c's spell menu prints from the same
/// rows. Empty strings are the original's own `string.Empty` holes; duplicate
/// names ("Detect Magic" at 0x05/0x0B/0x4D, the cleric/MU protection pairs)
/// are the original's too, not transcription slips.
///
/// [`spell_entry`]: crate::combat::spells
pub const SPELL_NAMES: [&str; 101] = [
    "",                                 // 0x00
    "Bless",                            // 0x01
    "Curse",                            // 0x02
    "Cure Light Wounds",                // 0x03
    "Cause Light Wounds",               // 0x04
    "Detect Magic",                     // 0x05
    "Protection from Evil",             // 0x06
    "Protection from Good",             // 0x07
    "Resist Cold",                      // 0x08
    "Burning Hands",                    // 0x09
    "Charm Person",                     // 0x0A
    "Detect Magic",                     // 0x0B
    "Enlarge",                          // 0x0C
    "Reduce",                           // 0x0D
    "Friends",                          // 0x0E
    "Magic Missile",                    // 0x0F
    "Protection From Evil",             // 0x10
    "Protection From Good",             // 0x11
    "Read Magic",                       // 0x12
    "Shield",                           // 0x13
    "Shocking Grasp",                   // 0x14
    "Sleep",                            // 0x15
    "Find Traps",                       // 0x16
    "Hold Person",                      // 0x17
    "Resist Fire",                      // 0x18
    "Silence, 15' Radius",              // 0x19
    "Slow Poison",                      // 0x1A
    "Snake Charm",                      // 0x1B
    "Spiritual Hammer",                 // 0x1C
    "Detect Invisibility",              // 0x1D
    "Invisibility",                     // 0x1E
    "Knock",                            // 0x1F
    "Mirror Image",                     // 0x20
    "Ray of Enfeeblement",              // 0x21
    "Stinking Cloud",                   // 0x22
    "Strength",                         // 0x23
    "Animate Dead",                     // 0x24
    "Cure Blindness",                   // 0x25
    "Cause Blindness",                  // 0x26
    "Cure Disease",                     // 0x27
    "Cause Disease",                    // 0x28
    "Dispel Magic",                     // 0x29
    "Prayer",                           // 0x2A
    "Remove Curse",                     // 0x2B
    "Bestow Curse",                     // 0x2C
    "Blink",                            // 0x2D
    "Dispel Magic",                     // 0x2E
    "Fireball",                         // 0x2F
    "Haste",                            // 0x30
    "Hold Person",                      // 0x31
    "Invisibility, 10' Radius",         // 0x32
    "Lightning Bolt",                   // 0x33
    "Protection From Evil, 10' Radius", // 0x34
    "Protection From Good, 10' Radius", // 0x35
    "Protection From Normal Missiles",  // 0x36
    "Slow",                             // 0x37
    "Restoration",                      // 0x38
    "",                                 // 0x39
    "Cure Serious Wounds",              // 0x3A
    "",                                 // 0x3B
    "",                                 // 0x3C
    "",                                 // 0x3D
    "",                                 // 0x3E
    "",                                 // 0x3F
    "",                                 // 0x40
    "",                                 // 0x41
    "Cause Serious Wounds",             // 0x42
    "Neutralize Poison",                // 0x43
    "Poison",                           // 0x44
    "Protection Evil, 10' Radius",      // 0x45
    "Sticks to Snakes",                 // 0x46
    "Cure Critical Wounds",             // 0x47
    "Cause Critical Wounds",            // 0x48
    "Dispel Evil",                      // 0x49
    "Flame Strike",                     // 0x4A
    "Raise Dead",                       // 0x4B
    "Slay Living",                      // 0x4C
    "Detect Magic",                     // 0x4D
    "Entangle",                         // 0x4E
    "Faerie Fire",                      // 0x4F
    "Invisibility to Animals",          // 0x50
    "Charm Monsters",                   // 0x51
    "Confusion",                        // 0x52
    "Dimension Door",                   // 0x53
    "Fear",                             // 0x54
    "Fire Shield",                      // 0x55
    "Fumble",                           // 0x56
    "Ice Storm",                        // 0x57
    "Minor Globe Of Invulnerability",   // 0x58
    "Remove Curse",                     // 0x59
    "Animate Dead",                     // 0x5A
    "Cloud Kill",                       // 0x5B
    "Cone of Cold",                     // 0x5C
    "Feeblemind",                       // 0x5D
    "Hold Monsters",                    // 0x5E
    "",                                 // 0x5F
    "",                                 // 0x60
    "",                                 // 0x61
    "",                                 // 0x62
    "",                                 // 0x63
    "Bestow Curse",                     // 0x64
];

/// `SpellNames[id]`, or `""` past the table.
///
/// The original indexes it unguarded; a spell id that big cannot come out of
/// any modeled cast, and an empty name is exactly what its own holes print —
/// so the fallback prints "Spell:" with nothing after it, as the original does
/// for `SpellNames[0x39]`, rather than inventing a placeholder.
pub fn spell_name(id: u8) -> &'static str {
    SPELL_NAMES.get(id as usize).copied().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_damage_line_is_singular_at_exactly_one() {
        assert_eq!(hitting_for(1), "Hitting for 1 point of damage");
        assert_eq!(hitting_for(2), "Hitting for 2 points of damage");
        assert_eq!(hitting_for(0), "Hitting for 0 points of damage");
    }

    #[test]
    fn spell_names_index_the_ids_the_engine_models() {
        // The three `spell_entry` rows — the cross-check that the transcription
        // is aligned with the ids `combat::spells` already uses.
        assert_eq!(spell_name(0x03), "Cure Light Wounds");
        assert_eq!(spell_name(0x0F), "Magic Missile");
        assert_eq!(spell_name(0x17), "Hold Person");
        // And the two ids `sub_5D2E1`'s cast-sound fork singles out
        // (`ovr023.cs:749-758`), which is an independent check on the same
        // alignment: 0x2F takes `sound_b`, 0x33 `sound_8`.
        assert_eq!(spell_name(0x2F), "Fireball");
        assert_eq!(spell_name(0x33), "Lightning Bolt");
    }

    #[test]
    fn an_unnamed_or_out_of_range_id_prints_nothing() {
        assert_eq!(spell_name(0x39), "", "the original's own hole");
        assert_eq!(spell_name(0xFF), "", "past the table");
    }

    #[test]
    fn the_range_status_keeps_its_erasing_spaces() {
        assert_eq!(range_status(4), "Range = 4  ");
    }
}
