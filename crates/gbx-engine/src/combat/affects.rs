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
        // Found on the actor → run the effect handler (§47.7 — the first REAL
        // `CallAffectTable` handlers; unknown kinds still trip, §39.4).
        if self.fighters[ci].find_affect(kind).is_some() {
            self.run_affect_handler(ci, kind);
            return;
        }
        // Radius-cast affects can be sourced from a team-mate carrier (the
        // 10-/15-foot-radius blessings). Scan first (immutable), then trip
        // (none of the four radius kinds has a landed handler yet).
        if RADIUS_CARRIER_KINDS.contains(&kind)
            && self.fighters.iter().any(|f| f.find_affect(kind).is_some())
        {
            self.trip_affect_effect(ci);
        }
    }

    /// The per-kind effect handlers (`CallAffectTable` check-time dispatch,
    /// coab `ovr013.cs:1780+` `affect_table`; §47.7) — the kinds the sewer
    /// captures actually dispatch, each binary-verified. Everything else keeps
    /// the `affect-effect` tripwire (§39.4). All draw-free — the one
    /// draw-bearing handler (`troll_fire_or_acid` 0x64, 3d6) lives on the
    /// Death dispatch path ([`CombatState::affect_death_check`]), the only
    /// site that carries an RNG.
    fn run_affect_handler(&mut self, ci: usize, kind: u8) {
        match kind {
            // `bless` 0x01 (`sub_3A096` @`ovr013:0096-00A3`, thunk `sub_BDA4`;
            // coab ovr013.cs:45 `Bless`): UNCONDITIONAL `monster_morale += 5`
            // (`add byte_1D2CC,5` @`0099`) + `attack_roll += 1` (`inc
            // byte_1D2C9` @`009E`). Liveness is per dispatch SITE: at Type_10
            // (attacker-side, inside `PC_CanHitTarget`) the +1 lands LIVE
            // between the d20 seed and the compare while the +5 is stale
            // (`FleeCheck_001` re-seeds `monster_morale` before every read);
            // at Morale (the FleeCheck pair) the +5 is live and the +1 stale
            // (the next swing re-seeds). No pinned capture blesses an NPC, so
            // only the Type_10 side is capture-exercised: buffed-otyugh (doc
            // §50) blesses all six PCs — 19 attacker dispatches, no boundary
            // roll (the capture pins the dispatch path, the listing the +1).
            0x01 => {
                self.monster_morale += 5;
                self.attack_roll += 1;
            }
            // `protection_from_evil` 0x08 (`sub_3A224` @`ovr013:0224-0256`,
            // thunk `sub_BDC2`; coab ovr013.cs:151): gate on the ACTING
            // combatant's `alignment@0x11B` ∈ {2, 5, 8} — the EVIL column
            // (LE/NE/CE), instruction-verified @`022B-0249` — then
            // `saving_throw += 2` + `attack_roll -= 2` (@`024B-0250`). The
            // binary reads the `player_ptr` GLOBAL (not the dispatched
            // player): the turn actor (`sub_33281` @`ovr009:02EA`),
            // re-pointed at the swing's attacker around `sub_3F4EB`
            // (`sub_3F9DB` @`1B6F-1B85`) — mirrored by `selected_attacker`.
            // Liveness by site (§47.7): at Type_11 (once per attack, BEFORE
            // the swing d20 seeds `attack_roll`) BOTH writes are stale —
            // capture-proven at a troll boundary hit (§47.7) and again in
            // buffed-otyugh with a byte-pinned evil attacker (otyugh 0x08,
            // doc §50); at SavingThrow the `+= 2` lands LIVE on the
            // accumulator (unexercised: no pinned save has a protected
            // saver). MARK's duplicate node rides inert via find-FIRST.
            0x08 => {
                let al = self.fighters[self.selected_attacker].alignment;
                if al == 2 || al == 5 || al == 8 {
                    self.saving_throw += 2;
                    self.attack_roll -= 2;
                }
            }
            // `dwarf_vs_orc` 0x1A (`AffectDwarfVsOrc` sub_3A7E8, ovr013.cs:357;
            // Type_10, attacker-side, LIVE inside `PC_CanHitTarget`): the
            // attacker's CURRENT target (actions.target — written by
            // AttackTarget before the swings) is orc-class (`field_14B & 4`;
            // the sewer TROLL carries 0x0E) → `attack_roll += 1`.
            0x1A => {
                if let Some(t) = self.fighters[ci].target {
                    if self.fighters[t].field_14b & 4 != 0 {
                        self.attack_roll += 1;
                    }
                }
            }
            // `dwarf_and_gnome_vs_giants` 0x2F (`AffectDwarfGnomeVsGiants`,
            // ovr013.cs:687; Type_16, TARGET-side, LIVE): the ATTACKER
            // (`gbl.SelectedPlayer`, mirrored in `selected_attacker`) is
            // monsterType giant(2)/troll(10) AND size-class 2
            // (`field_DE & 0x7F == 2`) → `attack_roll -= 4`. The sewer-fight-3
            // @978 boundary: troll roll 6 vs TRAVIS-the-dwarf misses.
            0x2F => {
                let a = &self.fighters[self.selected_attacker];
                if (a.monster_type == 2 || a.monster_type == 10) && (a.field_de & 0x7F) == 2 {
                    self.attack_roll -= 4;
                }
            }
            // `troll_regen` 0x65 (`sp_regenerate` @`ovr013:1FCC`,
            // AffectTrollRegenerate; Type_5 — the on-hit target check): unless
            // already regenerating (0x62 then 0x3B probed, `:1FD8-1FFE`), add
            // `regenerate` 0x3B with `call_spell_jump_list = TRUE`
            // (`add_affect(1, 0xFF, 3, regenerate)` @`:2000-2013`) — and the
            // ADD fires the kind's handler through the SAME jump table
            // (`sub_630C7` → `spell_jump_list[kind]`, no flag gate):
            // `AffectRegenration` (ovr013.cs:774) adds `regen_3_hp` 0x62
            // (call FALSE — the cascade stops). §47.7: the wounded troll then
            // heals +3 at EVERY round end (Type_19) — capture-proven ([9]
            // survives MATHEW's @1982 punch at hp 6 = two banked ticks ours
            // lacked). Draw-free.
            0x65 => {
                if !self.fighters[ci].has_affect(0x62) && !self.fighters[ci].has_affect(0x3B) {
                    self.fighters[ci].add_affect(0x3B, 3, 0xFF, true);
                    // The 0x3B ADD-handler (AffectRegenration).
                    self.fighters[ci].add_affect(0x62, 0, 0xFF, false);
                }
            }
            // `regen_3_hp` 0x62 (`AffectRegen3Hp` sub_3BEB8, ovr013.cs:1240;
            // Type_19 — BattleRoundChecks' per-combatant round-end sweep,
            // ovr009.cs:371): `hp += 3`, capped at max. No status gate — the
            // binary would tick a corpse too (unexercised: no troll dies in
            // any capture; the death strip leaves at most one 0x62 behind).
            0x62 => {
                let f = &mut self.fighters[ci];
                f.hp_current = (f.hp_current + 3).min(f.hp_max);
            }
            // `sub_3A071` = `clear_actions(player)` — the shared handler for
            // the RESTRAINED family (coab `affect_table`, ovr013.cs:1816/1820/
            // 1840-1842: fumbling 0x1B, helpless 0x1F, snake_charm 0x33,
            // paralyze 0x34, sleep 0x35). Fired by the turn-head
            // `CheckAffectsEffect(PlayerRestrained)` (`sub_33281`, coab
            // ovr009.cs:108) it zeroes `actions` — including `delay` — so the
            // held combatant's whole turn body is SKIPPED, draw-free (§49;
            // capture-proven: cleric-guildwar's held thieves stand mute).
            // sticks_to_snakes 0x03 / entangle 0x88 have DIFFERENT handlers
            // (attack-count decrement / save-to-break) — still tripwired.
            0x1B | 0x1F | 0x33 | 0x34 | 0x35 => self.clear_actions(ci),
            // `con_saving_bonus` 0x61 / `elf_resist_sleep` 0x6B: dispatched
            // only under SavingThrow / MagicResistance — neither fires on a
            // modeled path, so reaching here is a real surprise → trip.
            _ => self.trip_affect_effect(ci),
        }
    }

    /// The DEATH dispatch (`CheckAffectsEffect(target, Death)` at the weapon
    /// death tail, `ovr014:0630`; list = `{affect_63, troll_fire_or_acid 0x64,
    /// weap_dragon_slayer}` @coab `ovr024.cs:300-304`) — the ONE dispatch that
    /// can DRAW today: `troll_fire_or_acid`'s handler
    /// (`AffectTrollFireOrAcid`, ovr013.cs:1278) rolls **3d6** for the rise
    /// timer when the kill was not fire/acid: `add_affect(true, data 0xFF,
    /// minutes roll_dice(6,3), TrollRegen 0x66)`. Weapon damage carries no
    /// fire/acid `damage_flags` in any modeled path (`sub_3E192` zeroes them,
    /// doc §40), so the gate is always true here. The added `TrollRegen` is
    /// combat-inert beyond the list (0x66 is in no combat CheckType list —
    /// the corpse-rise machinery is camp/tick territory, cited §47.7; no
    /// capture shows a rise). The other two list ids fall through to the
    /// normal draw-free dispatch (trip on an unknown find).
    pub(super) fn affect_death_check(&mut self, rng: &mut EngineRng, ci: usize) {
        for &kind in CheckType::Death.affect_ids() {
            if kind == 0x64 && self.fighters[ci].find_affect(kind).is_some() {
                let minutes = roll_dice(rng, 6, 3); // ovr013.cs:1283 — 3d6
                self.fighters[ci].add_affect(0x66, minutes, 0xFF, true);
                // The 0x66 ADD-handler (`AffectTrollRegen` sub_3C01E): the
                // RISE attempt — `combat_heal(hp_max)` stands the troll back
                // up if placeable ("stands up and grins"), else re-adds the
                // timer. Unexercised (no capture kills a troll — regen keeps
                // them up); tripwire the territory rather than model it.
                let id = self.fighters[ci].id;
                self.emit(ActionEvent::StubTripped {
                    combatant_id: id,
                    stub: "troll-rise",
                });
                continue;
            }
            self.calc_affect_effect(ci, kind);
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
            // §47.7 — the first REAL `CallAffectTable(Remove)` handler:
            // `regenerate` 0x3B (added call_table=true by the troll on-hit
            // handler) fires `AffectRegenration` (ovr013.cs:774) on removal
            // too (the handler ignores the Effect arg): it adds `regen_3_hp`
            // 0x62 (coab `add_affect(false, data 0xFF, minutes 0, ...)` —
            // call_table FALSE). At the death strip this NETS TO ZERO by
            // TABLE ORDER: `STRIP_COMBAT_KINDS` lists 0x3B before 0x62, so
            // the re-added 0x62 is stripped two entries later (its own
            // removal fires nothing) — why no capture ever shows a corpse
            // regenerating. Draw-free. Everything else keeps the wire.
            if kind == 0x3B {
                self.fighters[ci].add_affect(0x62, 0, 0xFF, false);
            } else {
                let id = self.fighters[ci].id;
                self.emit(ActionEvent::StubTripped {
                    combatant_id: id,
                    stub: "affect-remove-side",
                });
            }
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
