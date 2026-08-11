//! ★ The camp Magic submenu's four leaves (roll-credits §8, D-S4c/D-S4d):
//! Memorize, Scribe, Display and Rest.
//!
//! Derived by reading coab for behavior (D11, never copied). The flows are
//! `memorize_spell` (`ovr016.cs:301-374`), `scribe_spell` (`:377-499`),
//! `DisplayMagicEffects` (`:556-597`) and `rest_menu` → `resting`
//! (`:274-298` / `ovr021.cs:516-612`).
//!
//! ★ **A door correction.** D-S4d says "Display renders the grimoire +
//! memorized sets". `magic_menu`'s `'D'` case is `DisplayMagicEffects`
//! (`ovr016.cs:632`), which lists every party member's **affects** — the
//! spell *effects* currently running — not their spells. Implemented as the
//! original has it; the grimoire and memorized sets are what Memorize's own
//! two lists already show.
//!
//! Each screen is a parked-widget state machine (D-UI1: no blocking loops),
//! which is the one structural change from the original's nested `while`s.
//! The states are the original's own control flow, named.

use crate::magic::{self, SpellListing, SpellLoc, SpellSource};
use crate::rest::{self, RestSession, RestTime};
use crate::screens::{ReturnTo, Screen, ScreenTransition};
use crate::shell::FlowCtx;
use crate::widgets::{Hotbar, ListMenu, Widget, WidgetOutcome};

/// `yes_no`'s two words (`ovr027.cs`), the established idiom in this codebase.
pub(crate) fn yes_no(prompt: &str) -> ConfirmWidget {
    ConfirmWidget {
        prompt: prompt.to_string(),
        menu: Widget::Hotbar(Hotbar::new("Yes No")),
    }
}

/// A parked `yes_no` — the prompt plus its two-word bar.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConfirmWidget {
    pub prompt: String,
    menu: Widget,
}

impl ConfirmWidget {
    /// `'Y'`, `'N'`, or still waiting. Escape reads as `'N'`, matching every
    /// other decline in this shell.
    fn tick(&mut self, ctx: &mut FlowCtx) -> Option<bool> {
        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::Hotbar(key) => match key.to_ascii_uppercase() {
                b'Y' => Some(true),
                b'N' | 0 => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn span(&self) -> Option<crate::widgets::WordRange> {
        match &self.menu {
            Widget::Hotbar(h) => h.selected_span(),
            _ => None,
        }
    }
}

/// One `spell_menu2` list on screen: the rows, the parallel id/scroll arrays,
/// and the heading `spell_menu2` writes beside the character's name
/// (`ovr020.cs:1436-1440`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpellListUi {
    menu: Widget,
    listing: SpellListing,
    heading: String,
    prompt: String,
}

impl SpellListUi {
    fn build(
        ch: &crate::party::Character,
        scrolls: &magic::ScrollLookup,
        loc: SpellLoc,
        source: SpellSource,
    ) -> Option<Self> {
        let listing = magic::build_spell_list(loc, ch, scrolls, None);
        if listing.is_empty() {
            // `BuildSpellList` returned false, so `spell_menu2` returns 0
            // without drawing anything at all (`ovr020.cs:1442-1445`).
            return None;
        }
        let menu = Widget::ListMenu(ListMenu::boxed(
            listing.items.clone(),
            magic::spell_list_layout(source),
        ));
        Some(SpellListUi {
            menu,
            listing,
            heading: format!("Spells {}", loc.heading()),
            prompt: source.prompt().to_string(),
        })
    }
}

/// The shared paint: the outer frame, the character's name and the list's
/// heading on row 1, then the boxed list itself.
///
/// `spell_menu2` picks its backdrop by source — `draw8x8_05` for Memorize,
/// `draw8x8_07` otherwise, `DrawFrame_Outer` in combat (`ovr020.cs:1419-1434`).
/// Those two `draw8x8_*` variants are frame layouts this engine does not
/// separate yet, so every camp list gets the outer frame; noted rather than
/// guessed.
pub(crate) fn paint_list(ctx: &mut FlowCtx, ui: &SpellListUi, name: &str, status: Option<&str>) {
    ctx.fb.clear(0);
    let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
    crate::text::draw_string(ctx.fb, ctx.font, name, 1, 1, 0, 0x0F);
    crate::text::draw_string(ctx.fb, ctx.font, &ui.heading, 1, name.len() + 4, 0, 10);
    if let Widget::ListMenu(list) = &ui.menu {
        crate::shell::draw_list_menu(ctx.fb, ctx.font, list);
    }
    if !ui.prompt.is_empty() {
        crate::text::draw_string(ctx.fb, ctx.font, &ui.prompt, 0x17, 1, 0, 13);
    }
    if let Some(s) = status {
        crate::text::draw_string(ctx.fb, ctx.font, s, 0x12, 1, 0, 0x0E);
    }
}

pub(crate) fn paint_confirm(ctx: &mut FlowCtx, confirm: &ConfirmWidget) {
    crate::text::draw_string(ctx.fb, ctx.font, &confirm.prompt, 0x18, 0, 0, 13);
    crate::combat::scene::render::draw_menu_line_at(
        ctx.fb,
        ctx.font,
        "Yes No",
        confirm.span(),
        confirm.prompt.len(),
    );
}

/// The refusal line every gate shows: `DisplayPlayerStatusString(true, 10,
/// text, player)` renders as `"<name> <text>"`.
pub(crate) fn player_status(name: &str, text: &str) -> String {
    format!("{name} {text}")
}

pub(crate) fn member_name(ctx: &FlowCtx, member: usize) -> String {
    ctx.roster
        .members
        .get(member)
        .map(|m| m.name.clone())
        .unwrap_or_default()
}

/// The resident `ITEMS` table, for every flow that needs to know what a scroll
/// is.
pub fn camp_scrolls(ctx: &FlowCtx) -> magic::ScrollLookup {
    magic::ScrollLookup::load(ctx.data)
}

// ===========================================================================
// Memorize (`memorize_spell`, `ovr016.cs:301-374`)
// ===========================================================================

/// `memorize_spell`'s control flow, as states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum MemorizeStage {
    /// The opening review of what is already staged (`:311`), then its
    /// `"Memorize These Spells? "` confirm (`:316`).
    OpeningReview,
    OpeningConfirm,
    /// The grimoire picker loop (`:332-354`), with the capacity table under it.
    Picker,
    /// The closing review + `"Memorize these spells? "` (`:356-367`) — reached
    /// only when the picker was actually opened (`index != -1`).
    ClosingReview,
    ClosingConfirm,
}

/// The Magic ▸ Memorize screen.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemorizeScreen {
    member: usize,
    stage: MemorizeStage,
    list: Option<SpellListUi>,
    confirm: Option<ConfirmWidget>,
    /// `index != -1` — whether the grimoire picker was ever opened, which is
    /// what decides if the closing review happens at all (`:356`).
    picker_opened: bool,
    status: Option<String>,
}

impl MemorizeScreen {
    /// Enters the flow, or bounces straight back to the magic menu with the
    /// gate's refusal on screen (`sub_443A0(2)`, `:305`).
    pub fn open(ctx: &mut FlowCtx) -> ScreenTransition {
        let member = selected_member(ctx);
        let Some(ch) = ctx.roster.members.get(member) else {
            return ScreenTransition::To(Screen::Magic(crate::screens::MagicMenu::new()));
        };
        if let Err(text) = magic::learn_gate(ch, 2, ctx.state.can_cast_spells) {
            return ScreenTransition::To(Screen::Magic(crate::screens::MagicMenu::with_status(
                player_status(&ch.name, text),
            )));
        }
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[member];
        let list = SpellListUi::build(ch, &scrolls, SpellLoc::Memorize, SpellSource::None);
        let mut screen = MemorizeScreen {
            member,
            stage: MemorizeStage::OpeningReview,
            list,
            confirm: None,
            picker_opened: false,
            status: None,
        };
        if screen.list.is_some() {
            // `var_2 == true`: something is already staged, so confirm it.
            screen.stage = MemorizeStage::OpeningConfirm;
            screen.confirm = Some(yes_no("Memorize These Spells? "));
        } else {
            screen.enter_picker(ctx);
        }
        ScreenTransition::To(Screen::Memorize(Box::new(screen)))
    }

    /// One turn of the picker loop's head (`:334-339`): rebuild the capacity
    /// table; if the character has no slots at all the loop ends with
    /// `"cannot memorize any spells"`, otherwise open the grimoire list.
    fn enter_picker(&mut self, ctx: &mut FlowCtx) {
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[self.member];
        if magic::memorize_capacity_table(ch).is_empty() {
            self.status = Some(player_status(&ch.name, "cannot memorize any spells"));
            self.stage = MemorizeStage::ClosingReview;
            self.list = None;
            return;
        }
        self.stage = MemorizeStage::Picker;
        self.list = SpellListUi::build(ch, &scrolls, SpellLoc::Grimoire, SpellSource::Memorize);
        if self.list.is_some() {
            self.picker_opened = true;
        } else {
            // An empty grimoire returns 0 from `spell_menu2`, which the loop
            // reads as "done" (`:345-347`).
            self.stage = MemorizeStage::ClosingReview;
        }
    }

    /// The closing review (`:356-367`), or straight back to the magic menu.
    fn close(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if !self.picker_opened {
            return self.back_to_magic();
        }
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[self.member];
        match SpellListUi::build(ch, &scrolls, SpellLoc::Memorize, SpellSource::None) {
            Some(list) => {
                self.list = Some(list);
                self.stage = MemorizeStage::ClosingConfirm;
                self.confirm = Some(yes_no("Memorize these spells? "));
                ScreenTransition::Stay
            }
            // `var_2 == false` — nothing staged, so no confirm at all.
            None => self.back_to_magic(),
        }
    }

    fn back_to_magic(&self) -> ScreenTransition {
        ScreenTransition::To(Screen::Magic(match &self.status {
            Some(s) => crate::screens::MagicMenu::with_status(s.clone()),
            None => crate::screens::MagicMenu::new(),
        }))
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if self.member >= ctx.roster.members.len() {
            return self.back_to_magic();
        }
        let name = member_name(ctx, self.member);

        // Paint first (immediate mode, D-UI4).
        match (&self.list, &self.confirm) {
            (Some(list), _) => {
                let list = list.clone();
                paint_list(ctx, &list, &name, self.status.as_deref());
                if matches!(self.stage, MemorizeStage::Picker) {
                    paint_capacity_table(ctx, self.member);
                }
            }
            (None, _) => {
                ctx.fb.clear(0);
                let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
                if let Some(s) = &self.status {
                    crate::text::draw_string(ctx.fb, ctx.font, s, 0x12, 1, 0, 0x0E);
                }
            }
        }
        if let Some(confirm) = &self.confirm {
            let confirm = confirm.clone();
            paint_confirm(ctx, &confirm);
        }

        match self.stage.clone() {
            MemorizeStage::OpeningConfirm | MemorizeStage::ClosingConfirm => {
                let closing = matches!(self.stage, MemorizeStage::ClosingConfirm);
                let Some(confirm) = &mut self.confirm else {
                    return self.back_to_magic();
                };
                match confirm.tick(ctx) {
                    None => ScreenTransition::Stay,
                    Some(true) => {
                        self.confirm = None;
                        if closing {
                            self.back_to_magic()
                        } else {
                            // `var_1 = true` — the staging stands, the picker
                            // is skipped entirely (`:322`).
                            self.back_to_magic()
                        }
                    }
                    Some(false) => {
                        magic::cancel_memorize(&mut ctx.roster.members[self.member].magic);
                        self.confirm = None;
                        if closing {
                            self.back_to_magic()
                        } else {
                            self.enter_picker(ctx);
                            ScreenTransition::Stay
                        }
                    }
                }
            }
            MemorizeStage::Picker => {
                let Some(ui) = &mut self.list else {
                    return self.close(ctx);
                };
                match ui.menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::ListSelected { index, .. } => {
                        let id = ui.listing.id_at_row(index);
                        if let Some(id) = id {
                            self.stage_one(ctx, id);
                        }
                        // The loop re-enters at its head: capacity table, then
                        // a freshly built grimoire list (`:334-342`).
                        self.enter_picker(ctx);
                        ScreenTransition::Stay
                    }
                    // `spellId == 0` — Exit/Escape ends the picker (`:345`).
                    WidgetOutcome::ListCancelled => self.close(ctx),
                    _ => ScreenTransition::Stay,
                }
            }
            MemorizeStage::OpeningReview | MemorizeStage::ClosingReview => self.close(ctx),
        }
    }

    /// The picker's one mutation (`:349-352`): stage the pick **iff** the
    /// character still has room at that spell's own class and level. A pick
    /// with no room is silently ignored, exactly as written — the capacity
    /// table above the list is the feedback.
    fn stage_one(&mut self, ctx: &mut FlowCtx, id: u8) {
        let class_ = magic::spell_class(id);
        let level = magic::spell_level(id);
        let ch = &mut ctx.roster.members[self.member];
        if magic::how_many_spells_player_can_learn(&ch.magic, class_, level) > 0 {
            magic::add_learn(&mut ch.magic.spell_list, id);
        }
    }
}

/// `BuildMemorizeSpellText`'s on-screen table (`ovr016.cs:233-268`): the
/// `"can memorize:"` line, then one row per class with five level columns
/// three cells apart starting at column `0x14`.
fn paint_capacity_table(ctx: &mut FlowCtx, member: usize) {
    let Some(ch) = ctx.roster.members.get(member) else {
        return;
    };
    let rows = magic::memorize_capacity_table(ch);
    if rows.is_empty() {
        return;
    }
    let line = player_status(&ch.name, "can memorize:");
    crate::text::draw_string(ctx.fb, ctx.font, &line, 0x12, 1, 0, 10);
    let rows: Vec<_> = rows
        .into_iter()
        .map(|r| (r.class_.memorize_heading(), r.cells))
        .collect();
    // `y_col` starts at 3 and the rows print at `y_col + 0x11`; the five level
    // columns start at `0x13` and step by 3 (`ovr016.cs:236-266`).
    for (y_col, (heading, cells)) in (3usize..).zip(rows) {
        crate::text::draw_string(ctx.fb, ctx.font, heading, y_col + 0x11, 1, 0, 10);
        for (i, cell) in cells.iter().enumerate() {
            let x_col = 0x13 + i * 3;
            crate::text::draw_string(ctx.fb, ctx.font, cell, y_col + 0x11, x_col + 1, 0, 10);
        }
    }
}

// ===========================================================================
// Scribe (`scribe_spell`, `ovr016.cs:377-499`)
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ScribeStage {
    OpeningConfirm,
    Picker,
    ClosingConfirm,
    Closing,
}

/// The Magic ▸ Scribe screen — the same shape as Memorize, with the scroll
/// lists and three extra per-pick gates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScribeScreen {
    member: usize,
    stage: ScribeStage,
    list: Option<SpellListUi>,
    confirm: Option<ConfirmWidget>,
    picker_opened: bool,
    status: Option<String>,
}

impl ScribeScreen {
    pub fn open(ctx: &mut FlowCtx) -> ScreenTransition {
        let member = selected_member(ctx);
        let Some(ch) = ctx.roster.members.get(member) else {
            return ScreenTransition::To(Screen::Magic(crate::screens::MagicMenu::new()));
        };
        if let Err(text) = magic::learn_gate(ch, 3, ctx.state.can_cast_spells) {
            return ScreenTransition::To(Screen::Magic(crate::screens::MagicMenu::with_status(
                player_status(&ch.name, text),
            )));
        }
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[member];
        let list = SpellListUi::build(ch, &scrolls, SpellLoc::Scribe, SpellSource::None);
        let mut screen = ScribeScreen {
            member,
            stage: ScribeStage::OpeningConfirm,
            list,
            confirm: None,
            picker_opened: false,
            status: None,
        };
        if screen.list.is_some() {
            screen.confirm = Some(yes_no("Scribe These Spells? "));
        } else {
            screen.enter_picker(ctx);
        }
        ScreenTransition::To(Screen::Scribe(Box::new(screen)))
    }

    fn enter_picker(&mut self, ctx: &mut FlowCtx) {
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[self.member];
        self.stage = ScribeStage::Picker;
        self.list = SpellListUi::build(ch, &scrolls, SpellLoc::Scrolls, SpellSource::Scribe);
        match &self.list {
            Some(_) => self.picker_opened = true,
            None => {
                // `var_4 == 0` with `var_2 == false` (`:417-420`).
                self.status = Some(player_status(&ch.name, "has no copyable scrolls"));
                self.stage = ScribeStage::Closing;
            }
        }
    }

    fn close(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if !self.picker_opened {
            return self.back_to_magic();
        }
        let scrolls = camp_scrolls(ctx);
        let ch = &ctx.roster.members[self.member];
        match SpellListUi::build(ch, &scrolls, SpellLoc::Scribe, SpellSource::None) {
            Some(list) => {
                self.list = Some(list);
                self.stage = ScribeStage::ClosingConfirm;
                self.confirm = Some(yes_no("Scribe these spells? "));
                ScreenTransition::Stay
            }
            None => self.back_to_magic(),
        }
    }

    fn back_to_magic(&self) -> ScreenTransition {
        ScreenTransition::To(Screen::Magic(match &self.status {
            Some(s) => crate::screens::MagicMenu::with_status(s.clone()),
            None => crate::screens::MagicMenu::new(),
        }))
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if self.member >= ctx.roster.members.len() {
            return self.back_to_magic();
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
                if let Some(s) = &self.status {
                    crate::text::draw_string(ctx.fb, ctx.font, s, 0x12, 1, 0, 0x0E);
                }
            }
        }
        if let Some(confirm) = &self.confirm {
            let confirm = confirm.clone();
            paint_confirm(ctx, &confirm);
        }

        match self.stage.clone() {
            ScribeStage::OpeningConfirm | ScribeStage::ClosingConfirm => {
                let closing = matches!(self.stage, ScribeStage::ClosingConfirm);
                let Some(confirm) = &mut self.confirm else {
                    return self.back_to_magic();
                };
                match confirm.tick(ctx) {
                    None => ScreenTransition::Stay,
                    Some(true) => {
                        self.confirm = None;
                        self.back_to_magic()
                    }
                    Some(false) => {
                        let scrolls = camp_scrolls(ctx);
                        magic::cancel_scribes(&mut ctx.roster.members[self.member].items, &scrolls);
                        self.confirm = None;
                        if closing {
                            self.back_to_magic()
                        } else {
                            self.enter_picker(ctx);
                            ScreenTransition::Stay
                        }
                    }
                }
            }
            ScribeStage::Picker => {
                let Some(ui) = &mut self.list else {
                    return self.close(ctx);
                };
                match ui.menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::ListSelected { index, .. } => {
                        if let Some(id) = ui.listing.id_at_row(index) {
                            self.stage_one(ctx, id);
                        }
                        self.enter_picker(ctx);
                        ScreenTransition::Stay
                    }
                    WidgetOutcome::ListCancelled => self.close(ctx),
                    _ => ScreenTransition::Stay,
                }
            }
            ScribeStage::Closing => self.close(ctx),
        }
    }

    /// ★ The three gates on one pick (`ovr016.cs:429-476`), in order:
    ///
    /// 1. `KnowsSpell` → `"You already know that spell"`.
    /// 2. any carried scroll already `ScrollLearning` this id → `"You are
    ///    already scibing that spell"` (the misspelling is the original's).
    /// 3. `spellCastCount[class, level-1] == 0` → `"You can not scribe that
    ///    spell."`
    ///
    /// Note the third gate reads **capacity**, not free capacity: a caster
    /// whose slots are all spoken for may still scribe, because scribing writes
    /// the grimoire, not the memorized list.
    ///
    /// The staging write itself (`:455-470`) walks **every** item, not only
    /// scrolls, and sets the high bit on the first affect byte equal to the
    /// chosen id.
    fn stage_one(&mut self, ctx: &mut FlowCtx, id: u8) {
        let scrolls = camp_scrolls(ctx);
        let ch = &mut ctx.roster.members[self.member];
        if magic::knows_spell(ch, id) {
            self.status = Some("You already know that spell".into());
            return;
        }
        let already = ch.items.iter().any(|item| {
            scrolls.is_scroll(item)
                && (1..=3).any(|i| {
                    let raw = gbx_formats::save_orig::item_affect(item, i);
                    raw > 0x7F && (raw & 0x7F) == id
                })
        });
        if already {
            self.status = Some("You are already scibing that spell".into());
            return;
        }
        let class_ = magic::spell_class(id);
        let level = magic::spell_level(id);
        if magic::cast_count_at(&ch.magic, class_, level) == 0 {
            self.status = Some("You can not scribe that spell.".into());
            return;
        }
        self.status = None;
        for item in ch.items.iter_mut() {
            let mut staged = false;
            for i in 1..=3 {
                if gbx_formats::save_orig::item_affect(item, i) == id {
                    gbx_formats::save_orig::set_item_affect(item, i, id | 0x80);
                    staged = true;
                    break;
                }
            }
            if staged {
                break;
            }
        }
    }
}

// ===========================================================================
// Display (`DisplayMagicEffects`, `ovr016.cs:556-597`)
// ===========================================================================

/// The Magic ▸ Display screen: a heading-per-member list of running affects.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpellEffectsScreen {
    menu: Widget,
}

impl SpellEffectsScreen {
    /// `DisplayMagicEffects`'s list build (`:558-585`): a leading blank row,
    /// then per member a name **heading**, one row per named affect (or
    /// `" <No Spell Effects>"`), then a blank spacer.
    pub fn open(ctx: &FlowCtx) -> Screen {
        use crate::widgets::ListItem;
        let mut items = vec![ListItem::Entry(String::new())];
        for member in &ctx.roster.members {
            items.push(ListItem::Heading(member.name.clone()));
            let mut any = false;
            for raw in &member.affects {
                let Some(rec) = gbx_formats::affects::AffectRecord::decode(raw) else {
                    continue;
                };
                if let Some(name) = magic::effect_name(rec.kind) {
                    any = true;
                    items.push(ListItem::Entry(format!(" {name}")));
                }
            }
            if !any {
                items.push(ListItem::Entry(" <No Spell Effects>".into()));
            }
            items.push(ListItem::Entry(" ".into()));
        }
        // `sl_select_item(…, 0x16, 0x26, 4, 1, MenuColorSet(15,10,11), "", "")`
        // (`ovr016.cs:592-593`).
        let layout = crate::widgets::ListLayout {
            start_row: 4,
            start_col: 1,
            end_row: 0x16,
            end_col: 0x26,
        };
        Screen::SpellEffects(Box::new(SpellEffectsScreen {
            menu: Widget::ListMenu(ListMenu::boxed(items, layout)),
        }))
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        if let Widget::ListMenu(list) = &self.menu {
            crate::shell::draw_list_menu(ctx.fb, ctx.font, list);
        }
        match self.menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::ListCancelled | WidgetOutcome::ListSelected { .. } => {
                ScreenTransition::To(Screen::Magic(crate::screens::MagicMenu::new()))
            }
            _ => ScreenTransition::Stay,
        }
    }
}

// ===========================================================================
// Rest (`rest_menu` → `resting`, `ovr016.cs:274-298` / `ovr021.cs:516-612`)
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum RestStage {
    /// `resting_time_menu` (`ovr021.cs:252-355`) — adjust the countdown before
    /// committing.
    TimeMenu,
    /// The loop is running (`:548-606`).
    Running,
    /// `yes_no("Stop Resting? ")` (`:559`).
    StopConfirm,
}

/// The camp Rest screen — `rest_menu`'s computed countdown, the Days/Hours/Mins
/// adjustment bar, and the loop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RestScreen {
    session: RestSession,
    stage: RestStage,
    menu: Widget,
    confirm: Option<ConfirmWidget>,
    /// `time_index` (`ovr021.cs:257`) — which field Add/Subtract move. Starts
    /// at 2 (minutes).
    time_index: usize,
    /// Where the camp screen goes back to (`rest_menu` is reachable from both
    /// the camp bar and the magic bar, `ovr016.cs:1134`/`:636`).
    from_magic: bool,
    /// The camp Fix plan this rest is serving, if any (`FixTeam`, `:1055-1067`).
    fix: Option<rest::FixPlan>,
    messages: Vec<String>,
    /// The campfire `MakeCamp` left in the viewport — `resting` never repaints
    /// it, so the rest screen inherits it. Transient, never serialized.
    #[serde(skip)]
    picture: Option<crate::screens::CampPicture>,
}

/// `resting_time_menu`'s command bar (`ovr021.cs:264`).
const REST_TIME_MENU: &str = "Rest Days Hours Mins Add Subtract Exit";

impl RestScreen {
    /// `rest_menu` (`ovr016.cs:274-298`): compute the party's required time,
    /// then hand it to `resting(true)`.
    pub fn open(ctx: &mut FlowCtx, from_magic: bool) -> Screen {
        let scrolls = camp_scrolls(ctx);
        let time = rest::rest_menu_time(ctx.roster, &scrolls);
        Self::with_time(ctx, time, from_magic, None)
    }

    /// `FixTeam`'s call, `resting(false)` (`ovr016.cs:1057`): the countdown is
    /// `CalculateTimeAndSpellNumbers`', and there is no time menu — the rest
    /// starts immediately.
    pub fn open_for_fix(ctx: &mut FlowCtx, plan: rest::FixPlan) -> Screen {
        let time = plan.time_to_rest;
        let mut screen = Self::with_time(ctx, time, false, Some(plan));
        if let Screen::Rest(r) = &mut screen {
            r.stage = RestStage::Running;
        }
        screen
    }

    fn with_time(
        ctx: &mut FlowCtx,
        time: RestTime,
        from_magic: bool,
        fix: Option<rest::FixPlan>,
    ) -> Screen {
        let session = RestSession::start(time, ctx.roster.members.len());
        let mut hotbar = Hotbar::new(REST_TIME_MENU);
        hotbar.accept_ext = true;
        Screen::Rest(Box::new(RestScreen {
            session,
            stage: RestStage::TimeMenu,
            menu: Widget::Hotbar(hotbar),
            confirm: None,
            time_index: 2,
            from_magic,
            fix,
            messages: Vec::new(),
            picture: None,
        }))
    }

    fn back(&self, ctx: &mut FlowCtx) -> ScreenTransition {
        if self.from_magic {
            ScreenTransition::To(Screen::Magic(crate::screens::MagicMenu::new()))
        } else {
            ScreenTransition::To(Screen::Camp(crate::screens::Camp::new(ctx)))
        }
    }

    /// `display_resting_time` (`ovr021.cs:220-247`): `"Rest Time:"` then
    /// `DD:HH:MM`, the highlighted field in colour 15 and the rest in 10.
    fn paint(&mut self, ctx: &mut FlowCtx) {
        ctx.fb.clear(0);
        let _ = crate::frames::draw8x8_03(ctx.fb, ctx.symbols);
        // `resting` inherits `MakeCamp`'s composition; the campfire stays up.
        let mut picture = self.picture.take();
        crate::screens::draw_camp_picture(ctx, &mut picture);
        self.picture = picture;
        let rows: Vec<_> = ctx
            .roster
            .members
            .iter()
            .map(crate::charsheet::sheet_view)
            .collect();
        let selected =
            (!rows.is_empty()).then(|| (ctx.state.selected_player as usize) % rows.len());
        crate::charsheet::render_party_summary(ctx.fb, ctx.font, &rows, selected);
        crate::screens::draw_position_time(ctx, true);

        let highlight = if matches!(self.stage, RestStage::TimeMenu) {
            self.time_index
        } else {
            0
        };
        let (days, hours, minutes) = self.session.time_to_rest.display_parts();
        crate::text::draw_string(ctx.fb, ctx.font, "Rest Time:", 17, 1, 0, 10);
        let mut col = 11usize;
        for (slot, value) in [(4usize, days), (3, hours), (2, minutes)] {
            let colour = if slot == highlight { 15 } else { 10 };
            let text = format!("{value:02}");
            crate::text::draw_string(ctx.fb, ctx.font, &text, 17, col + 1, 0, colour);
            if slot != 2 {
                crate::text::draw_string(ctx.fb, ctx.font, ":", 17, col + 3, 0, 10);
            }
            col += 3;
        }
        for (i, msg) in self.messages.iter().rev().take(2).enumerate() {
            crate::text::draw_string(ctx.fb, ctx.font, msg, 19 + i, 1, 0, 10);
        }
        if matches!(self.stage, RestStage::TimeMenu) {
            let span = match &self.menu {
                Widget::Hotbar(h) => h.selected_span(),
                _ => None,
            };
            crate::combat::scene::render::draw_menu_line(ctx.fb, ctx.font, REST_TIME_MENU, span);
        }
        if let Some(confirm) = &self.confirm {
            let confirm = confirm.clone();
            paint_confirm(ctx, &confirm);
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        self.paint(ctx);
        match self.stage.clone() {
            RestStage::TimeMenu => self.tick_time_menu(ctx),
            RestStage::StopConfirm => {
                let Some(confirm) = &mut self.confirm else {
                    self.stage = RestStage::Running;
                    return ScreenTransition::Stay;
                };
                match confirm.tick(ctx) {
                    None => ScreenTransition::Stay,
                    Some(true) => {
                        self.session.stop();
                        self.confirm = None;
                        self.finish(ctx)
                    }
                    Some(false) => {
                        self.confirm = None;
                        self.stage = RestStage::Running;
                        ScreenTransition::Stay
                    }
                }
            }
            RestStage::Running => {
                // `KEYPRESSED()` (`ovr021.cs:555`): any key mid-rest opens the
                // "Stop Resting?" prompt.
                if ctx.input.read_key().is_some() {
                    self.confirm = Some(yes_no("Stop Resting? "));
                    self.stage = RestStage::StopConfirm;
                    return ScreenTransition::Stay;
                }
                if !self.session.resting() {
                    return self.finish(ctx);
                }
                let scrolls = camp_scrolls(ctx);
                let (period, percentage) = rest::schedule_cells(ctx.vm_memory);
                // A handful of iterations per tick: the original's loop is not
                // frame-paced at all, and a full night at one iteration a tick
                // would take minutes of wall clock.
                for _ in 0..REST_ITERATIONS_PER_TICK {
                    if !self.session.resting() {
                        break;
                    }
                    let events = self.session.step(
                        ctx.roster,
                        &mut ctx.state.clock,
                        &scrolls,
                        ctx.rng,
                        period,
                        percentage,
                    );
                    for event in events {
                        if let Some(line) = self.describe(ctx, &event) {
                            self.messages.push(line);
                        }
                    }
                }
                ScreenTransition::Stay
            }
        }
    }

    /// `DisplayCaseSpellText` (`ovr023.cs:3114-3134`)'s out-of-combat arm plus
    /// `rest_heal`'s and the interruption's own lines.
    fn describe(&self, ctx: &FlowCtx, event: &rest::RestEvent) -> Option<String> {
        use rest::RestEvent::*;
        Some(match event {
            Memorized { member, spell_id } => format!(
                "{} has memorized {}",
                member_name(ctx, *member),
                magic::spell_name(*spell_id)
            ),
            Scribed { member, spell_id } => format!(
                "{} has scribed {}",
                member_name(ctx, *member),
                magic::spell_name(*spell_id)
            ),
            ScrollConsumed { .. } => return None,
            // An expiring buff is silent in the original (`CheckAffectsTimingOut`
            // prints nothing at all) — the Display list is where it shows.
            AffectsExpired(_) => return None,
            PartyHealed => rest::PARTY_HEALED_TEXT.to_string(),
            Interrupted => rest::INTERRUPTED_TEXT.to_string(),
        })
    }

    /// `resting_time_menu` (`ovr021.cs:252-355`).
    fn tick_time_menu(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let outcome = self.menu.tick(ctx.input, ctx.dt_ticks);
        let key = match outcome {
            WidgetOutcome::Hotbar(k) => k.to_ascii_uppercase(),
            // The extended keys the menu remaps (`:266-300`): up/down are Add/
            // Subtract, left/right cycle the field.
            WidgetOutcome::PartyScroll(code) => match code {
                b'H' => b'A',
                b'P' => b'S',
                b'K' => {
                    self.time_index = if self.time_index >= 4 {
                        2
                    } else {
                        self.time_index + 1
                    };
                    return ScreenTransition::Stay;
                }
                b'M' => {
                    self.time_index = if self.time_index <= 2 {
                        4
                    } else {
                        self.time_index - 1
                    };
                    return ScreenTransition::Stay;
                }
                _ => return ScreenTransition::Stay,
            },
            _ => return ScreenTransition::Stay,
        };
        match key {
            // Enter is Rest (`:303-306`).
            b'R' | b'\r' => {
                self.stage = RestStage::Running;
                ScreenTransition::Stay
            }
            b'D' => {
                self.time_index = 4;
                ScreenTransition::Stay
            }
            b'H' => {
                self.time_index = 3;
                ScreenTransition::Stay
            }
            b'M' => {
                self.time_index = 2;
                ScreenTransition::Stay
            }
            b'A' => {
                // Minutes step in FIVES; days and hours by one (`:327-334`).
                if self.time_index == 2 {
                    self.session.time_to_rest.slots[1] += 5;
                } else {
                    self.session.time_to_rest.slots[self.time_index] += 1;
                }
                rest::clock_583c8(&mut self.session.time_to_rest);
                ScreenTransition::Stay
            }
            b'S' => {
                if self.time_index == 2 {
                    rest::rest_time_subtract(&mut self.session.time_to_rest, 1, 5);
                } else {
                    rest::rest_time_subtract(&mut self.session.time_to_rest, self.time_index, 1);
                }
                rest::clock_583c8(&mut self.session.time_to_rest);
                ScreenTransition::Stay
            }
            // Exit/Escape leaves without resting (`unk_58731`, `:250`).
            b'E' | 0 => self.finish(ctx),
            _ => ScreenTransition::Stay,
        }
    }

    /// `resting` returns, then `rest_menu` clears the countdown
    /// (`ovr016.cs:293`) and — when this rest was serving a Fix — the healing
    /// is applied only if nothing interrupted it (`:1059-1068`).
    ///
    /// ★ An interruption is `MakeCamp`'s `actionInterrupted`, which `TryEncamp`
    /// turns into a run of the ECL header's **vector 3**
    /// ([`ScreenTransition::CampInterrupted`]).
    fn finish(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if let Some(plan) = self.fix.take() {
            if !self.session.interrupted {
                rest::apply_fix(ctx.roster, &plan, ctx.rng);
            }
        }
        if self.session.interrupted {
            return ScreenTransition::CampInterrupted;
        }
        self.back(ctx)
    }
}

/// How many rest iterations one tick advances. Purely a pacing choice: the
/// original's loop is a tight `while` with no frame budget at all, and a
/// twelve-hour rest is 144 iterations.
const REST_ITERATIONS_PER_TICK: usize = 8;

/// `gbl.SelectedPlayer`'s roster index, clamped.
pub(crate) fn selected_member(ctx: &FlowCtx) -> usize {
    let count = ctx.roster.members.len();
    if count == 0 {
        0
    } else {
        (ctx.state.selected_player as usize) % count
    }
}

/// `ReturnTo` is unused by these screens (they all return to the magic menu or
/// camp), but the enum is part of the screens API they live beside.
const _: Option<ReturnTo> = None;
