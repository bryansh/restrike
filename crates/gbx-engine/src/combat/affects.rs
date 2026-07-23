use super::*;
use gbx_formats::affects::AffectRecord;

impl Combatant {
    // --- the affect substrate API (doc §39.2, all PRNG-free) ---------------

    /// `FindAffect(out affect, kind, player)` (`ovr025.cs:1175-1180`, binary
    /// `find_affect` @`ovr025:2345`): the **first** affect of `kind` in list
    /// order, or `None`. `player.affects.Find(aff => aff.type == kind)`.
    pub fn find_affect(&self, kind: u8) -> Option<&AffectRecord> {
        self.affects.iter().find(|a| a.kind == kind)
    }

    /// `player.HasAffect(kind)` — whether any affect of `kind` is present.
    pub fn has_affect(&self, kind: u8) -> bool {
        self.affects.iter().any(|a| a.kind == kind)
    }

    /// `add_affect(call_table, data, minutes, type, player)` (`ovr024:13F0-14A4`,
    /// coab `ovr024.cs:609`): construct the affect and **append it at the TAIL**
    /// (`player.affects.Add`; the binary walks the `next` chain to the end). The
    /// `call_table=true` add-side handler (`CallAffectTable(Add)`, `ovr013`) is
    /// NOT modeled — no current caller adds affects; the spell slice will.
    pub fn add_affect(&mut self, kind: u8, minutes: u16, data: u8, call_affect_table: bool) {
        self.affects.push(AffectRecord {
            kind,
            minutes,
            data,
            call_affect_table,
        });
    }
}

impl CombatState {
    // === the affect substrate (doc §39, all PRNG-free) =====================
    //
    // Every method below makes ZERO `roll_dice` calls (the only `@Random`
    // consumer in ovr024 is `roll_dice` itself, `ovr024:13AC`), and with the
    // empty affect lists every capture carries, every FIND misses — so no
    // tripwire fires and no draw moves. That PRNG-free dispatch over empty
    // state is the whole draw-neutrality argument (doc §39.2/§39.4); the guard
    // 8/8 run per commit is its check.

    /// `CheckAffectsEffect(player, type)` (`work_on_00` @`ovr024:0414-0D02`) —
    /// the 24-case dispatch: for each affect id in the case's ORDERED list, run
    /// [`calc_affect_effect`](Self::calc_affect_effect) on `ci`. The id lists are
    /// transcribed verbatim from coab `ovr024.cs:140-375` (verified id-for-id and
    /// order-for-order against the binary); find-first semantics make list order
    /// observable once effect handlers land, so it is preserved. Draw-free.
    pub(super) fn check_affects_effect(&mut self, ci: usize, ty: CheckType) {
        for &kind in ty.affect_ids() {
            self.calc_affect_effect(ci, kind);
        }
    }

    /// `calc_affect_effect(kind, player)` (`ovr024:027A-0411`, coab `:99-136`):
    /// find `kind` on the actor `ci`; if absent AND `kind` is one of the
    /// radius-cast affects [`RADIUS_CARRIER_KINDS`] {silence_15_radius 0x15,
    /// prot_from_evil_10_radius 0x2D, prot_from_good_10_radius 0x2E, prayer 0x31}
    /// (`unk_6325A` bitmask @`ovr024:025A`, decoded), scan the team lists for a
    /// **carrier** holding `kind`. A carrier found in combat gates on range in the
    /// binary (≤6 for prayer, else ≤1, via the near-list builder @`ovr024:031C-0388`)
    /// — the range gate + the effect handler (`CallAffectTable(Add)`) are the
    /// spell slice's; here we model the scan and **TRIP** on any found affect (on
    /// the actor, or a carrier for a radius kind). Draw-free.
    pub(super) fn calc_affect_effect(&mut self, ci: usize, kind: u8) {
        // Found on the actor → the point where the binary runs a `CallAffectTable`
        // handler we don't model yet (doc §39.4).
        if self.fighters[ci].find_affect(kind).is_some() {
            self.trip_affect_effect(ci);
            return;
        }
        // Radius-cast affects can be sourced from a team-mate carrier (the
        // 10-/15-foot-radius blessings). Scan first (immutable), then trip.
        if RADIUS_CARRIER_KINDS.contains(&kind)
            && self.fighters.iter().any(|f| f.find_affect(kind).is_some())
        {
            self.trip_affect_effect(ci);
        }
    }

    fn trip_affect_effect(&mut self, ci: usize) {
        let id = self.fighters[ci].id;
        self.emit(ActionEvent::StubTripped {
            combatant_id: id,
            stub: "affect-effect",
        });
    }

    /// `remove_affect(null, kind, player)` (`ovr024:010A-0257`, an UNHEADERED
    /// label reached via the `stub024` thunk; coab `:67-95`) — remove the FIRST
    /// matching instance (not all). Side effects cited, tripwired via
    /// `"affect-remove-side"`, not modeled: the `CallAffectTable(Remove)` when the
    /// removed record carries `call_affect_table` (`ovr024:016B-0186`), and the
    /// `CalcStatBonuses` recompute — **CHA for `friends` 0x0E** (`ovr024:0222`;
    /// coab says `resist_fire`, a coab≠binary bug — the binary compares `0x0E`,
    /// and Friends buffs Charisma) and **STR for enlarge 0x0C / strength 0x26 /
    /// strength_spell 0x92** (`ovr024:0235-0245`). Draw-free.
    pub(super) fn remove_affect(&mut self, ci: usize, kind: u8) {
        let Some(idx) = self.fighters[ci]
            .affects
            .iter()
            .position(|a| a.kind == kind)
        else {
            return;
        };
        let removed = self.fighters[ci].affects.remove(idx);
        if removed.call_affect_table || STAT_RECOMPUTE_KINDS.contains(&kind) {
            let id = self.fighters[ci].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "affect-remove-side",
            });
        }
    }

    /// `RemoveCombatAffects(player)` (`sub_645AB` @`ovr024:15AB`, coab `:661-691`):
    /// strip the fixed table [`STRIP_COMBAT_KINDS`] (each id via
    /// [`remove_affect`](Self::remove_affect)), then the berserk quirk
    /// (`ovr024:15DC-1601`): if the combatant `HasAffect(berserk 0x4D)` and
    /// `control_morale == PC_Berzerk 0xB3` (`field_F7`), the binary flips
    /// `combat_team = Ours` — **tripwired** (`"affect-berserk"`), not modeled (a
    /// runtime team flip we don't carry; it never fires on an empty list). Table
    /// ids transcribed from the LISTING data `unk_16D41[1..19]` @`seg600:0A32-0A44`
    /// (`07 0B 0D 15 17 1E 1F 20 33 34 35 3A 3B 5F 62 88 89 8B 90` — 19 entries,
    /// matching coab). Draw-free.
    pub(super) fn remove_combat_affects(&mut self, ci: usize) {
        for &kind in STRIP_COMBAT_KINDS {
            self.remove_affect(ci, kind);
        }
        if self.fighters[ci].has_affect(AFF_BERSERK)
            && self.fighters[ci].control_morale == PC_BERZERK
        {
            let id = self.fighters[ci].id;
            self.emit(ActionEvent::StubTripped {
                combatant_id: id,
                stub: "affect-berserk",
            });
        }
    }

    /// `RemoveAttackersAffects(player)` (`sub_6460D` @`ovr024:160D`, coab
    /// `:694-702`): strip [`STRIP_ATTACKERS_KINDS`]. Ids transcribed from the
    /// LISTING data `[0xA46..0xA49]` @`seg600` (`0D 3A 8B 90` = reduce,
    /// clear_movement, affect_8b, owlbear_hug_round_attack — 4 entries, matching
    /// coab). Draw-free.
    pub(super) fn remove_attackers_affects(&mut self, ci: usize) {
        for &kind in STRIP_ATTACKERS_KINDS {
            self.remove_affect(ci, kind);
        }
    }

    /// `remove_invisibility(player)` (coab `ovr024.cs:650-658`): while an
    /// `invisibility` (0x19) affect remains, remove it — clears every instance.
    /// Draw-free (a list walk).
    pub(super) fn remove_invisibility(&mut self, ci: usize) {
        while self.fighters[ci].find_affect(AFF_INVISIBILITY).is_some() {
            self.remove_affect(ci, AFF_INVISIBILITY);
        }
    }
}

// --- affect ids + fixed tables (doc §39, binary/coab-cited) ----------------
//
// With the §39.5 census fully wired, every id/table below is live. The one
// `#[allow(dead_code)]` that remains sits on `CheckType` (below): its full
// 24-value set is transcribed for dispatch fidelity, but only the subset
// constructed at census sites is built. (`add_affect` needs no allow — a `pub`
// method on a `pub` struct is never dead-code-flagged; it stays uncalled until
// the spell slice supplies the first affect-adding caller.)

/// `Affects.invisibility` (`Classes/Affect.cs:32`).
const AFF_INVISIBILITY: u8 = 0x19;
/// `Affects.berserk` (`Affect.cs:84`) — the [`RemoveCombatAffects`] quirk gate.
const AFF_BERSERK: u8 = 0x4D;
/// `Control.PC_Berzerk` (`Player.cs:324`) — `control_morale@0xF7`; the listing
/// compares `es:[di+field_F7], 0B3h` (`ovr024:15F6`) after finding berserk.
const PC_BERZERK: u8 = 0xB3;

/// The radius-cast affects a team-mate can source (`unk_6325A` bitmask
/// @`ovr024:025A`, decoded to a set): silence_15_radius 0x15,
/// prot_from_evil_10_radius 0x2D, prot_from_good_10_radius 0x2E, prayer 0x31.
const RADIUS_CARRIER_KINDS: [u8; 4] = [0x15, 0x2D, 0x2E, 0x31];

/// The affect kinds whose `remove_affect` triggers a `CalcStatBonuses` recompute
/// (`ovr024:0222-0245`) — the `"affect-remove-side"` tripwire set alongside
/// `call_affect_table`. From the LISTING: **CHA on friends 0x0E** (`@0222`,
/// coab≠binary — coab wrote `resist_fire`; the binary compares `0x0E`), **STR on
/// enlarge 0x0C / strength 0x26 / strength_spell 0x92** (`@0235-0245`).
const STAT_RECOMPUTE_KINDS: [u8; 4] = [0x0E, 0x0C, 0x26, 0x92];

/// `RemoveCombatAffects`'s strip table (`unk_16D41[1..19]` @`seg600:0A32-0A44`,
/// transcribed from the LISTING; == coab `ovr024.cs:661-691`): faerie_fire,
/// charm_person, reduce, silence_15_radius, spiritual_hammer, stinking_cloud,
/// helpless, animate_dead, snake_charm, paralyze, sleep, clear_movement,
/// regenerate, affect_5F, regen_3_hp, entangle, affect_89, affect_8b,
/// owlbear_hug_round_attack.
const STRIP_COMBAT_KINDS: &[u8] = &[
    0x07, 0x0B, 0x0D, 0x15, 0x17, 0x1E, 0x1F, 0x20, 0x33, 0x34, 0x35, 0x3A, 0x3B, 0x5F, 0x62, 0x88,
    0x89, 0x8B, 0x90,
];

/// `RemoveAttackersAffects`'s strip table (`[0xA46..0xA49]` @`seg600`,
/// transcribed from the LISTING; == coab `ovr024.cs:694-702`): reduce 0x0D,
/// clear_movement 0x3A, affect_8b 0x8B, owlbear_hug_round_attack 0x90.
const STRIP_ATTACKERS_KINDS: &[u8] = &[0x0D, 0x3A, 0x8B, 0x90];

/// `CheckType` (`ovr024.cs:6-32`) — the argument to `CheckAffectsEffect`
/// (`work_on_00`). The full 24-value set is transcribed for fidelity; only the
/// subset wired at census sites (doc §39.5) is ever constructed, so the rest are
/// `dead_code` by construction — allowed, not removed, because the dispatch
/// [`affect_ids`](CheckType::affect_ids) is only faithful with every case
/// present and ordered.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CheckType {
    None = 0,
    Visibility = 1,
    Type2 = 2,
    Type3 = 3,
    SpecialAttacks = 4,
    Type5 = 5,
    PreDamage = 6,
    PlayerRestrained = 7,
    Type8 = 8,
    MagicResistance = 9,
    Type10 = 10,
    Type11 = 11,
    SavingThrow = 12,
    Death = 13,
    Type14 = 14,
    Type15 = 15,
    Type16 = 16,
    Morale = 17,
    Movement = 18,
    Type19 = 19,
    FireShield = 20,
    Confusion = 21,
    Type22 = 22,
    Type23 = 23,
}

impl CheckType {
    /// The ORDERED affect-id list this check runs `calc_affect_effect` over,
    /// transcribed verbatim from coab `ovr024.cs:140-375` (ids from
    /// `Classes/Affect.cs`, verified id-for-id and order-for-order against the
    /// binary dispatch `work_on_00` @`ovr024:0414-0D02`).
    pub(super) fn affect_ids(self) -> &'static [u8] {
        match self {
            CheckType::None => &[],
            CheckType::Visibility => &[0x25, 0x19, 0x47, 0x45],
            CheckType::Type2 => &[0x4F, 0x50, 0x91, 0x39, 0x60, 0x7A, 0x7B],
            CheckType::Type3 => &[0x40, 0x41, 0x42, 0x43, 0x46, 0x4F, 0x57],
            CheckType::SpecialAttacks => &[0x1D, 0x06, 0x67, 0x4B, 0x4C, 0x86],
            CheckType::Type5 => &[
                0x1C, 0x29, 0x68, 0x78, 0x65, 0x73, 0x74, 0x77, 0x5E, 0x75, 0x3C, 0x51, 0x52, 0x55,
                0x82, 0x8F,
            ],
            CheckType::PreDamage => &[
                0x71, 0x3D, 0x0A, 0x14, 0x69, 0x6A, 0x70, 0x72, 0x76, 0x11, 0x5D, 0x65, 0x1C, 0x6E,
                0x49, 0x52, 0x54, 0x81, 0x85, 0x87, 0x3F,
            ],
            CheckType::PlayerRestrained => &[0x33, 0x34, 0x35, 0x1F, 0x03, 0x1B, 0x88],
            CheckType::Type8 => &[0x63, 0x52, 0x59, 0x48, 0x38],
            CheckType::MagicResistance => &[
                0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70, 0x7C, 0x7D, 0x3F, 0x81,
            ],
            CheckType::Type10 => &[0x01, 0x02, 0x21, 0x24, 0x31, 0x06, 0x12, 0x1A, 0x4B, 0x4C],
            CheckType::Type11 => &[0x21, 0x11, 0x08, 0x09, 0x2D, 0x2E, 0x1E, 0x07],
            CheckType::SavingThrow => &[
                0x08, 0x09, 0x0A, 0x11, 0x14, 0x21, 0x24, 0x2D, 0x2E, 0x31, 0x3D, 0x6F, 0x7D, 0x61,
                0x32, 0x36,
            ],
            CheckType::Death => &[0x63, 0x64, 0x4B],
            CheckType::Type14 => &[
                0x53, 0x58, 0x79, 0x56, 0x57, 0x5A, 0x7E, 0x80, 0x83, 0x84, 0x8B,
            ],
            CheckType::Type15 => &[0x15, 0x1E, 0x0B, 0x0D, 0x4D],
            CheckType::Type16 => &[0x19, 0x47, 0x25, 0x2F, 0x30, 0x59, 0x04],
            CheckType::Morale => &[0x01, 0x02, 0x0B],
            CheckType::Movement => &[0x27, 0x2A, 0x3A],
            CheckType::Type19 => &[0x62, 0x17, 0x48, 0x38, 0x0B],
            CheckType::FireShield => &[0x32, 0x36],
            CheckType::Confusion => &[0x23],
            CheckType::Type22 => &[0x8A],
            CheckType::Type23 => &[0x4A],
        }
    }
}
