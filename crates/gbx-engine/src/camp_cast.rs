//! ★ The camp Magic ▸ **Cast** leaf, and the out-of-combat spell effects
//! (roll-credits §9, G7's Task 1).
//!
//! Derived by reading coab for behavior (D11, never copied).
//!
//! ## One cast function, two worlds
//!
//! `cast_spell` (`ovr016.cs:158-197`) is thirty lines: gate the caster, open
//! `spell_menu2(SpellSource.Cast, SpellLoc.memory)`, and hand whatever it
//! returns to **`ovr023.sub_5D2E1`** — the same function combat calls. The only
//! difference between the two worlds is `gbl.SpellCastFunction`, which is
//! `ovr014.target` in combat (`ovr009.cs:25`) and
//! `ovr023.NonCombatSpellCast` outside it (`ovr023.cs:3144`). Everything else —
//! the miscast gate, the casting text, `remove_invisibility`, `ClearSpell`, the
//! `gbl.spellTable` dispatch — is shared.
//!
//! So this module is `sub_5D2E1`'s **non-combat arm**, over
//! [`crate::party::Party`] instead of a `CombatState`:
//!
//! - [`NonCombatTargets`] is `NonCombatSpellCast`'s three-way switch on the
//!   row's `targetType` — `Self` is the caster, `PartyMember` opens
//!   `selectAPlayer("Cast Spell on whom")`, `WholeParty` is everybody, and the
//!   fourth case (`Combat`) cannot be cast here at all;
//! - [`cast`] is the effect dispatch, `gbl.spellTable`'s camp-reachable half.
//!
//! ## The gate that surprises people
//!
//! `sub_5D2E1`'s **first** branch (`ovr023.cs:668-690`) fires when
//! `game_state != Combat && targetType == SpellTargets.Combat`: it prints the
//! spell's name, `"can't be cast here..."`, and offers **`"Lose it? "`** — say
//! yes and the memorized slot is spent for nothing. That is how the original
//! lets you clear a useless Magic Missile out of memory before resting, and it
//! is why [`CastScreen`] has a confirm stage.
//!
//! ## Draws
//!
//! Six of the twelve camp-castable rows draw (the four cures' d8s, Dispel
//! Magic's d100 ladder). All of it is camp-only and therefore unreachable from
//! any captured fight — a capture replays a `combat_entry` snapshot straight
//! into `CombatState` and never camps, the same argument `crate::rest`'s own
//! draws already ride.

use crate::affects;
use crate::camp_magic::{
    camp_scrolls, member_name, paint_confirm, paint_list, player_status, selected_member,
    ConfirmWidget, SpellListUi,
};
use crate::magic::{self, SpellLoc, SpellSource};
use crate::party::{Character, Party};
use crate::rng::EngineRng;
use crate::screens::{Screen, ScreenTransition};
use crate::shell::FlowCtx;
use crate::spells::{self, SpellEntry, SpellTargets};
use crate::widgets::WidgetOutcome;

// ===========================================================================
// `NonCombatSpellCast` — the targeting (`ovr023.cs:622-660`)
// ===========================================================================

/// What `NonCombatSpellCast` leaves in `gbl.spellTargets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonCombatTargets {
    /// `SpellTargets.Self` (`:628`) — the list opens as `[SelectedPlayer]` and
    /// the `Self` case simply does not touch it.
    Caster,
    /// `SpellTargets.PartyMember` (`:631-645`) — `selectAPlayer(…, "Cast Spell
    /// on whom")`, whose Exit answer aborts the cast entirely.
    ChooseOne,
    /// `SpellTargets.WholeParty` (`:647-650`) — `AddRange(gbl.TeamList)` on top
    /// of the caster, so the caster appears **twice**. Faithfully reproduced:
    /// see [`whole_party_targets`].
    WholeParty,
    /// The `default` arm (`:652-655`): `castSpell = false`. Unreachable in
    /// practice, because `sub_5D2E1`'s own gate has already refused a
    /// `SpellTargets::Combat` row before the cast function runs.
    None,
}

impl NonCombatTargets {
    /// The row's `targetType`, as the switch reads it.
    pub fn of(entry: &SpellEntry) -> Self {
        match entry.target_type {
            SpellTargets::Self_ => NonCombatTargets::Caster,
            SpellTargets::PartyMember => NonCombatTargets::ChooseOne,
            SpellTargets::WholeParty => NonCombatTargets::WholeParty,
            SpellTargets::Combat => NonCombatTargets::None,
        }
    }
}

/// `SpellTargets.WholeParty`'s list (`ovr023.cs:647-650`) — **the caster, then
/// the whole roster**, so the caster is in it twice.
///
/// That duplicate is not a transcription slip: `spellTargets` opens as
/// `[SelectedPlayer]` at `:625` and the `WholeParty` arm calls `AddRange`
/// without clearing first, where every other arm clears. For an affect-planting
/// row (Bless, Prayer) it means the caster's `DoSpellCastingWork` pass runs
/// twice — and the second pass finds the affect it just added with
/// `minutes > 0`, removes it, and re-adds it, so the *observable* result is
/// unchanged. Kept as written rather than tidied, with the reason recorded.
pub fn whole_party_targets(caster: usize, party: &Party) -> Vec<usize> {
    let mut out = vec![caster];
    out.extend(0..party.members.len());
    out
}

/// `sub_5D2E1`'s opening refusal (`ovr023.cs:668-690`): out of combat, a row
/// whose `targetType` is `Combat` cannot be cast, and the player is offered the
/// chance to burn the slot anyway.
pub fn cant_be_cast_here(entry: &SpellEntry) -> bool {
    entry.target_type == SpellTargets::Combat
}

// ===========================================================================
// The effects — `gbl.spellTable`'s camp-reachable half
// ===========================================================================

/// One line of feedback from a camp cast, the way
/// `DisplayPlayerStatusString`/`MagicAttackDisplay` would have shown it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastReport {
    /// The party index the line is about.
    pub member: usize,
    /// The text after the name (`"is Blessed"`, `"is unpoisoned"`, …).
    pub text: String,
}

/// `gbl.spellTable[spell_id]()` for a cast made outside combat.
///
/// `targets` is what [`NonCombatTargets`] resolved to. Returns the lines the
/// original would have printed; an empty result means the spell did nothing
/// visible, which several of them legitimately do.
///
/// Combat-only rows are not reachable here — [`cant_be_cast_here`] refuses them
/// before this is called — so their arms are absent rather than stubbed.
pub fn cast(
    party: &mut Party,
    rng: &mut EngineRng,
    caster: usize,
    spell_id: u8,
    targets: &[usize],
) -> Vec<CastReport> {
    let Some(entry) = spells::spell_entry(spell_id) else {
        return Vec::new();
    };
    let casting_lvl = casting_level(&party.members[caster], &entry);
    let minutes = spells::spell_affect_timeout(&entry, casting_lvl);
    let mut out = Vec::new();
    match spell_id {
        // --- `CastTeamSpell` (`ovr023.cs:989-1011`) ------------------------
        // Out of combat the `BuildNearTargets(1, …)` melee filter is gated on
        // `game_state == Combat` (`:994`), so it never fires: everybody in the
        // party gets the blessing. `combat_team` is uniformly Ours in camp, so
        // the team filter keeps everyone too.
        0x01 => {
            for &t in targets {
                plant(party, t, &entry, minutes, casting_lvl as u8, false);
                out.push(report(party, t, "is Blessed"));
            }
        }
        // --- the affect-only rows (`is_affected`, `SpellProtectionFromX`) ---
        0x06 | 0x07 | 0x45 => {
            for &t in targets {
                plant(party, t, &entry, minutes, casting_lvl as u8, false);
                out.push(report(party, t, "is protected"));
            }
        }
        0x12 | 0x16 => {
            for &t in targets {
                plant(party, t, &entry, minutes, casting_lvl as u8, false);
                out.push(report(party, t, "is affected"));
            }
        }
        0x2A => {
            // `SpellPrayer` (`:1823`): `data` packs the caster's team (always
            // Ours out of combat, so bit 4 is clear) and the casting level.
            for &t in targets {
                plant(party, t, &entry, minutes, casting_lvl as u8, false);
                out.push(report(party, t, "is praying"));
            }
        }
        // --- the cures (`heal_player` with the row's own dice) --------------
        0x03 | 0x3A | 0x47 => {
            let heal = match spell_id {
                0x03 => i32::from(crate::rest::roll_dice(rng, 8, 1)),
                0x3A => i32::from(crate::rest::roll_dice(rng, 8, 2)) + 1,
                _ => i32::from(crate::rest::roll_dice(rng, 8, 3)) + 3,
            };
            if let Some(&t) = targets.first() {
                if crate::rest::heal_player(0, heal, &mut party.members[t]) {
                    out.push(report(party, t, describe_healing(&party.members[t])));
                }
            }
        }
        // --- `is_affected2` — Slow Poison (`:1291-1311`) --------------------
        0x1A => {
            if let Some(&t) = targets.first() {
                let ch = &mut party.members[t];
                if ch.status.health_status != crate::rest::status::ANIMATED
                    && ch.has_affect(spells::AFF_POISONED)
                {
                    if ch.hit_point_current == 0 {
                        ch.hit_point_current = 1;
                    }
                    affects::add_affect(ch, entry.affect_id, minutes, 0xFF, true);
                    affects::remove_affect(ch, spells::AFF_4E);
                    affects::add_affect(ch, spells::AFF_POISON_DAMAGE, 10, 0xFF, true);
                    out.push(report(party, t, "is affected"));
                }
            }
        }
        // --- `SpellCureBlindness` (`:1587`) ---------------------------------
        0x25 => {
            if let Some(&t) = targets.first() {
                if affects::cure_affect(&mut party.members[t], spells::AFF_BLINDED) {
                    out.push(report(party, t, "can see"));
                }
            }
        }
        // --- `SpellCureDisease` → `sub_5F037` (`:1602-1645`) ----------------
        0x27 => {
            if let Some(&t) = targets.first() {
                if cure_disease(&mut party.members[t]) {
                    out.push(report(party, t, "is Cured"));
                }
            }
        }
        // --- `SpellDispelMagic` (`:1667-1716`) ------------------------------
        0x29 | 0x2E => {
            if let Some(&t) = targets.first() {
                if dispel_magic(&mut party.members[t], rng, casting_lvl) {
                    out.push(report(party, t, "is affected"));
                }
            }
        }
        // --- `SpellRemoveCurse` (`:1831-1864`) ------------------------------
        0x2B => {
            if let Some(&t) = targets.first() {
                out.extend(remove_curse(party, t));
            }
        }
        // --- `SpellNeutralizePoison` (`cure_poison`, `:2242-2275`) ----------
        0x43 => {
            if let Some(&t) = targets.first() {
                let text = neutralize_poison(&mut party.members[t]);
                out.push(report(party, t, text));
            }
        }
        // --- `SpellRaiseDead` (`cast_raise`, `:2341-2365`) ------------------
        0x4B => {
            if let Some(&t) = targets.first() {
                if raise_dead(&mut party.members[t]) {
                    out.push(report(party, t, "is raised"));
                }
            }
        }
        _ => {}
    }
    out
}

/// `spellMaxTargetCount`'s camp twin (`ovr025.cs:1342-1385`): the caster's
/// level in the row's own class, with the `Paladin − 8` / `Ranger − 8`
/// fallbacks the combat side already transcribes.
fn casting_level(ch: &Character, entry: &SpellEntry) -> i32 {
    use crate::party::{SKILL_CLERIC, SKILL_MAGIC_USER, SKILL_PALADIN, SKILL_RANGER};
    match entry.spell_class {
        spells::SpellClass::Cleric => ch
            .skill_level(SKILL_CLERIC)
            .max(ch.skill_level(SKILL_PALADIN) - 8),
        spells::SpellClass::MagicUser => ch
            .skill_level(SKILL_MAGIC_USER)
            .max(ch.skill_level(SKILL_RANGER) - 8),
        // No druid or monster row is camp-castable in §9.1's set.
        _ => 0,
    }
}

/// `ApplyAttackSpellAffect`'s else arm (`ovr024.cs:1316-1331`) with no save to
/// consider (every camp-castable row is `DamageOnSave::Normal`): drop an
/// existing instance that still has time on it, then add the fresh one.
fn plant(
    party: &mut Party,
    member: usize,
    entry: &SpellEntry,
    minutes: u16,
    data: u8,
    call_affect_table: bool,
) {
    let ch = &mut party.members[member];
    if affects::find_affect(ch, entry.affect_id).is_some_and(|a| a.minutes > 0) {
        affects::remove_affect(ch, entry.affect_id);
    }
    affects::add_affect(ch, entry.affect_id, minutes, data, call_affect_table);
}

fn report(party: &Party, member: usize, text: impl Into<String>) -> CastReport {
    let _ = party;
    CastReport {
        member,
        text: text.into(),
    }
}

/// `DescribeHealing` (`ovr025.cs:1246-1259`) — chosen from the hit points
/// **after** the heal.
fn describe_healing(ch: &Character) -> &'static str {
    if ch.hit_point_current >= ch.hit_point_max {
        "is fully healed"
    } else {
        "is partially healed"
    }
}

/// `sub_5F037` (`ovr023.cs:1602-1645`) — Cure Disease's three cures and their
/// cascades: `cause_disease_1`; `weaken`, which also strips `cause_disease_2`
/// and `helpless`; and `hot_fire_shield`, which also strips `affect_39`.
fn cure_disease(ch: &mut Character) -> bool {
    let mut any = false;
    if affects::cure_affect(ch, spells::AFF_CAUSE_DISEASE_1) {
        any = true;
    }
    if affects::cure_affect(ch, spells::AFF_WEAKEN) {
        any = true;
        affects::remove_affect(ch, spells::AFF_CAUSE_DISEASE_2);
        affects::remove_affect(ch, spells::AFF_HELPLESS);
    }
    if affects::cure_affect(ch, spells::AFF_HOT_FIRE_SHIELD) {
        any = true;
        affects::remove_affect(ch, spells::AFF_39);
    }
    any
}

/// `SpellDispelMagic`'s affect half (`ovr023.cs:1673-1716`) — one **d100** per
/// affect whose `affect_data < 0xFF`, against the level ladder. Draw-bearing.
///
/// The `targetPos` ground sweep (`:1719-1822`) that follows it in combat has no
/// camp counterpart: there is no combat map and no gas cloud to dispel.
fn dispel_magic(ch: &mut Character, rng: &mut EngineRng, casting_lvl: i32) -> bool {
    let candidates: Vec<(u8, i32)> = affects::decoded(ch)
        .filter(|a| a.data < 0xFF)
        .map(|a| (a.kind, i32::from(a.data & 0x0F)))
        .collect();
    let mut any = false;
    for (kind, level) in candidates {
        let needed = if casting_lvl > level {
            50 + (casting_lvl - level) * 5
        } else if casting_lvl < level {
            50 - (level - casting_lvl) * 2
        } else {
            50
        };
        if i32::from(crate::rest::roll_dice(rng, 100, 1)) <= needed {
            affects::remove_affect(ch, kind);
            any = true;
        }
    }
    any
}

/// `SpellRemoveCurse` (`uncurse`, `ovr023.cs:1831-1864`): cure `bestow_curse`
/// if it is there, **otherwise** un-ready the first cursed item.
///
/// ★ This is the arm the combat side cannot have — a combatant carries no
/// inventory, only a `has_items` bool. Out of camp the items are right there,
/// so the real behavior lands here: the item's `readied` flag is cleared and
/// the `CalcStatBonuses` recompute for an item affect above `0x7F` is cited
/// (no §9.1 row plants one, and the stat machinery is `crate::training`'s).
fn remove_curse(party: &mut Party, member: usize) -> Vec<CastReport> {
    if affects::cure_affect(&mut party.members[member], spells::AFF_BESTOW_CURSE) {
        return vec![report(party, member, "is un-cursed")];
    }
    let ch = &mut party.members[member];
    let cursed = ch
        .items
        .iter()
        .position(|it| gbx_formats::save_orig::item_is_cursed(it));
    let Some(idx) = cursed else {
        return Vec::new();
    };
    gbx_formats::save_orig::set_item_readied(&mut ch.items[idx], false);
    ch.readied_items.remove(&idx);
    vec![report(party, member, "has an item un-cursed")]
}

/// `SpellNeutralizePoison` (`cure_poison`, `ovr023.cs:2242-2275`) — the real
/// answer to a spider bite: strip `poisoned`, `slow_poison` and
/// `poison_damage`, lift a member at 0 hit points to 1, and put them back on
/// their feet (`in_combat = true`, `health_status = okey`).
fn neutralize_poison(ch: &mut Character) -> &'static str {
    if ch.status.health_status == crate::rest::status::ANIMATED {
        return "is unaffected";
    }
    if !ch.has_affect(spells::AFF_POISONED) {
        return "is unaffected";
    }
    if ch.hit_point_current == 0 {
        ch.hit_point_current = 1;
    }
    affects::remove_affect(ch, spells::AFF_POISONED);
    affects::remove_affect(ch, spells::AFF_SLOW_POISON);
    affects::remove_affect(ch, spells::AFF_POISON_DAMAGE);
    ch.status.in_combat = true;
    ch.status.health_status = crate::rest::status::OKEY;
    "is unpoisoned"
}

/// `SpellRaiseDead` (`cast_raise`, `ovr023.cs:2341-2365`) — the effect G8's
/// temple service will also want.
///
/// Three gates, all the original's: the target is `dead` or `animated`, its
/// **current** Constitution is above zero, and it is **not an elf**. Then
/// `animate_dead` and `poisoned` come off, the status goes to `okey`,
/// **Constitution drops by one**, and the character stands up at exactly one
/// hit point. The CON cost is what makes raising a habit expensive, and the elf
/// clause is why an elf who dies stays dead.
fn raise_dead(ch: &mut Character) -> bool {
    const RACE_ELF: u8 = 2; // `Race.elf` (`Classes/Enums.cs`)
    let raisable = matches!(
        ch.status.health_status,
        crate::rest::status::DEAD | crate::rest::status::ANIMATED
    );
    if !raisable || ch.stats.con.current == 0 || ch.race == RACE_ELF {
        return false;
    }
    affects::remove_affect(ch, spells::AFF_ANIMATE_DEAD);
    affects::remove_affect(ch, spells::AFF_POISONED);
    ch.status.health_status = crate::rest::status::OKEY;
    ch.status.in_combat = true;
    ch.stats.con.current -= 1;
    ch.hit_point_current = 1;
    true
}

// ===========================================================================
// The screen — `cast_spell` (`ovr016.cs:158-197`) as a parked state machine
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum CastStage {
    /// `spell_menu2(Cast, memory)` (`:174`).
    Picker,
    /// `"can't be cast here..."` + `yes_no("Lose it? ")` (`ovr023.cs:672-682`).
    LoseIt,
    /// `selectAPlayer(…, "Cast Spell on whom")` (`ovr023.cs:635`).
    ChooseTarget,
}

/// The Magic ▸ Cast screen.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CastScreen {
    member: usize,
    stage: CastStage,
    list: Option<SpellListUi>,
    confirm: Option<ConfirmWidget>,
    /// The spell chosen, waiting on a target or on the "Lose it?" answer.
    pending: Option<u8>,
    /// `gbl.lastSelectetSpellTarget` (`ovr023.cs:624`) — the cursor
    /// `selectAPlayer` opens on, remembered across casts within one visit.
    target: usize,
    status: Option<String>,
}

impl CastScreen {
    /// `cast_spell`'s head (`ovr016.cs:160-172`): clear the remembered target,
    /// run `sub_443A0(1)` — the "cannot cast spells in this area" / "is in no
    /// condition to cast any spells" gate — and open the list.
    pub fn open(ctx: &mut FlowCtx) -> ScreenTransition {
        let member = selected_member(ctx);
        let Some(ch) = ctx.roster.members.get(member) else {
            return back_to_magic(None);
        };
        if let Err(text) = magic::learn_gate(ch, 1, ctx.state.can_cast_spells) {
            return back_to_magic(Some(player_status(&ch.name, text)));
        }
        let scrolls = camp_scrolls(ctx);
        let list = SpellListUi::build(ch, &scrolls, SpellLoc::Memory, SpellSource::Cast);
        if list.is_none() {
            // `var_3 == false` — nothing memorized at all (`ovr016.cs:189`).
            return back_to_magic(Some(player_status(&ch.name, "has no spells memorized")));
        }
        ScreenTransition::To(Screen::Cast(Box::new(CastScreen {
            member,
            stage: CastStage::Picker,
            list,
            confirm: None,
            pending: None,
            target: member,
            status: None,
        })))
    }

    /// Rebuild the picker after a cast — the `do … while (spell_id != 0)` loop
    /// re-enters `spell_menu2` every time (`ovr016.cs:172-190`), so a consumed
    /// slot disappears from the list immediately.
    fn reopen(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[self.member];
        self.list = SpellListUi::build(ch, &scrolls, SpellLoc::Memory, SpellSource::Cast);
        self.pending = None;
        self.confirm = None;
        self.stage = CastStage::Picker;
        if self.list.is_none() {
            return back_to_magic(self.status.clone());
        }
        ScreenTransition::Stay
    }

    /// `sub_5D2E1`'s body for one chosen spell.
    fn begin(&mut self, ctx: &mut FlowCtx, spell_id: u8) -> ScreenTransition {
        let Some(entry) = spells::spell_entry(spell_id) else {
            // The lazy-transcription rule (§9.1): a pruned row is a loud
            // refusal, not a guess. The slot is NOT consumed.
            self.status = Some(format!(
                "{} is not implemented yet",
                magic::spell_name(spell_id)
            ));
            return self.reopen(ctx);
        };
        self.pending = Some(spell_id);
        if cant_be_cast_here(&entry) {
            self.stage = CastStage::LoseIt;
            self.confirm = Some(crate::camp_magic::yes_no("Lose it? "));
            self.status = Some(format!(
                "{} can't be cast here...",
                magic::spell_name(spell_id)
            ));
            return ScreenTransition::Stay;
        }
        match NonCombatTargets::of(&entry) {
            NonCombatTargets::ChooseOne => {
                self.stage = CastStage::ChooseTarget;
                self.list = None;
                ScreenTransition::Stay
            }
            NonCombatTargets::Caster => {
                let targets = vec![self.member];
                self.resolve(ctx, spell_id, &targets)
            }
            NonCombatTargets::WholeParty => {
                let targets = whole_party_targets(self.member, ctx.roster);
                self.resolve(ctx, spell_id, &targets)
            }
            // Unreachable: `cant_be_cast_here` already took this row.
            NonCombatTargets::None => self.reopen(ctx),
        }
    }

    /// The tail of `sub_5D2E1` (`ovr023.cs:769-782`): `remove_invisibility`,
    /// `ClearSpell`, then the effect.
    fn resolve(&mut self, ctx: &mut FlowCtx, spell_id: u8, targets: &[usize]) -> ScreenTransition {
        affects::remove_all(&mut ctx.roster.members[self.member], AFF_INVISIBILITY);
        magic::clear_spell(
            &mut ctx.roster.members[self.member].magic.spell_list,
            spell_id,
        );
        let reports = cast(ctx.roster, ctx.rng, self.member, spell_id, targets);
        self.status = reports.first().map(|r| {
            player_status(
                &ctx.roster
                    .members
                    .get(r.member)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
                &r.text,
            )
        });
        self.reopen(ctx)
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if self.member >= ctx.roster.members.len() {
            return back_to_magic(None);
        }
        let name = member_name(ctx, self.member);
        match &self.list {
            Some(list) => {
                let list = list.clone();
                paint_list(ctx, &list, &name, self.status.as_deref());
            }
            None => {
                ctx.fb.clear(0);
                let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
                let rows: Vec<_> = ctx
                    .roster
                    .members
                    .iter()
                    .map(crate::charsheet::sheet_view)
                    .collect();
                crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, Some(self.target));
                if let Some(s) = &self.status {
                    crate::text::draw_string(ctx.fb, ctx.font, s, 0x12, 1, 0, 0x0E);
                }
                if matches!(self.stage, CastStage::ChooseTarget) {
                    crate::text::draw_string(
                        ctx.fb,
                        ctx.font,
                        "Cast Spell on whom  Select Exit",
                        0x17,
                        1,
                        0,
                        13,
                    );
                }
            }
        }
        if let Some(confirm) = &self.confirm {
            let confirm = confirm.clone();
            paint_confirm(ctx, &confirm);
        }

        match self.stage.clone() {
            CastStage::Picker => {
                let Some(ui) = &mut self.list else {
                    return back_to_magic(self.status.clone());
                };
                match ui.tick(ctx) {
                    WidgetOutcome::ListSelected { index, .. } => match ui.id_at_row(index) {
                        Some(id) => self.begin(ctx, id),
                        None => ScreenTransition::Stay,
                    },
                    // `spell_menu2` returning 0 with `var_3 == true` ends the
                    // loop (`ovr016.cs:186`).
                    WidgetOutcome::ListCancelled => back_to_magic(self.status.clone()),
                    _ => ScreenTransition::Stay,
                }
            }
            CastStage::LoseIt => {
                let Some(confirm) = &mut self.confirm else {
                    return self.reopen(ctx);
                };
                match confirm.tick(ctx) {
                    None => ScreenTransition::Stay,
                    Some(true) => {
                        // `ClearSpell` and nothing else (`ovr023.cs:680`).
                        if let Some(id) = self.pending {
                            magic::clear_spell(
                                &mut ctx.roster.members[self.member].magic.spell_list,
                                id,
                            );
                        }
                        self.status = None;
                        self.reopen(ctx)
                    }
                    Some(false) => {
                        self.status = None;
                        self.reopen(ctx)
                    }
                }
            }
            CastStage::ChooseTarget => {
                // `selectAPlayer` (`ovr025.cs:1527-1567`): Up/Down cycle the
                // roster, 'S'/Enter select, 'E'/Escape abort the cast.
                let count = ctx.roster.members.len();
                match ctx.input.read_key().and_then(select_key) {
                    Some(SelectKey::Next) => {
                        self.target = (self.target + 1) % count;
                        ScreenTransition::Stay
                    }
                    Some(SelectKey::Prev) => {
                        self.target = (self.target + count - 1) % count;
                        ScreenTransition::Stay
                    }
                    Some(SelectKey::Select) => {
                        let id = self.pending.unwrap_or(0);
                        let targets = vec![self.target];
                        self.resolve(ctx, id, &targets)
                    }
                    Some(SelectKey::Exit) => {
                        // `lastSelectetSpellTarget == null` → `castSpell =
                        // false`, and `sub_5D2E1`'s non-combat arm then sets
                        // `stillCast = false` **without** ClearSpell
                        // (`ovr023.cs:786-789`) — the slot survives an abort.
                        self.status = None;
                        self.reopen(ctx)
                    }
                    None => ScreenTransition::Stay,
                }
            }
        }
    }
}

/// `Affects.invisibility` (0x19) — `remove_invisibility(caster)`
/// (`ovr024.cs:650-658`) runs on every successful cast, in or out of combat.
const AFF_INVISIBILITY: u8 = 0x19;

fn back_to_magic(status: Option<String>) -> ScreenTransition {
    ScreenTransition::To(Screen::Magic(match status {
        Some(s) => crate::screens::MagicMenu::with_status(s),
        None => crate::screens::MagicMenu::new(),
    }))
}

enum SelectKey {
    Next,
    Prev,
    Select,
    Exit,
}

/// `selectAPlayer`'s two key sets (`ovr025.cs:1527-1566`):
///
/// - the **terminators** `unk_68DFA = {13, 27, 'E', 'S'}` (`:1522`) — Enter,
///   Escape, `'E'` and `'S'`. With `showExit == true` (which "Cast Spell on
///   whom" passes) `'E'` and Escape null the target and abort the cast; the
///   other two commit whoever the cursor is on;
/// - the **cursor** keys, which arrive as `special_key` ctrl codes rather than
///   characters: `'O'` steps forward and `'G'` steps back (`:1545-1556`).
///   Through [`crate::input::ExtKey::ctrl_code`] those are End/Kp1 and
///   Home/Kp7 — so the comparison is written against the ctrl code, not
///   against a guessed arrow key.
fn select_key(ev: crate::input::InputEvent) -> Option<SelectKey> {
    use crate::input::InputEvent;
    match ev {
        InputEvent::Enter => Some(SelectKey::Select),
        InputEvent::Escape => Some(SelectKey::Exit),
        InputEvent::Char(c) => match c.to_ascii_uppercase() {
            b'S' => Some(SelectKey::Select),
            b'E' => Some(SelectKey::Exit),
            _ => None,
        },
        InputEvent::Ext(k) => match k.ctrl_code() {
            b'O' => Some(SelectKey::Next),
            b'G' => Some(SelectKey::Prev),
            // The original's own arrow pair, which the party-summary cursor
            // also honours in every other roster picker in this shell.
            b'P' | b'M' => Some(SelectKey::Next),
            b'H' | b'K' => Some(SelectKey::Prev),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests;
