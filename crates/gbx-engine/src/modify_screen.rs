//! ★ **Modify Character** (`modifyPlayer`, `ovr018.cs:999-1428`) and **Human
//! Change Classes** (`DuelClass`, `ovr026.cs:603-699`) — roll-credits slice
//! 9c's other two `startGameMenu` verbs.

use crate::creation;
use crate::party::Character;
use crate::screens::{ReturnTo, Screen, ScreenTransition};
use crate::shell::FlowCtx;
use crate::widgets::{ListItem, ListLayout, ListMenu, Widget, WidgetOutcome};
use gbx_rules::adnd1::flavor_impl::Adnd1;
use gbx_rules::flavor::Flavor;

/// ★ `modifyPlayer`'s own gate (`ovr018.cs:1004-1013`), which is the whole
/// answer to "how permissive is the Gold Box modify?": **completely**
/// permissive, but only on a character who has not adventured.
///
/// The condition is `exp ∉ {0, 8333, 12500, 25000} || multiclassLevel != 0`.
/// Those four values are exactly the XP totals `createPlayer` hands out
/// (`ovr018.cs:483,510,517,524,...` — 25,000 single-class, 12,500 two-class,
/// 8,333 three-class, and 0 for a freshly dual-classed character), so a
/// character who has earned a single experience point is refused, as is any
/// dual-class character. Inside that window every stat, the hit points and
/// the name are freely editable within the race/sex/class limits.
pub fn can_modify(ch: &Character) -> bool {
    matches!(ch.exp, 0 | 8333 | 12500 | 25000) && ch.multiclass_level == 0
}

/// `Player.CanDuelClass()` (`Classes/Player.cs:782-798`): human, and no class
/// already banked in `ClassLevelsOld`.
pub fn can_dual_class(ch: &Character) -> bool {
    const HUMAN: u8 = 7;
    ch.race == HUMAN && ch.class_levels_old.iter().all(|&l| l == 0)
}

/// Which of the eight fields `modifyPlayer` is editing (`edited_stat`,
/// `ovr018.cs:1025`): 0..=5 the ability scores, 6 the hit points, 7 the name.
const FIELD_COUNT: u8 = 8;
const FIELD_HP: u8 = 6;
const FIELD_NAME: u8 = 7;

/// `displayInput(..., "Keep Exit", "Modify: ")` (`ovr018.cs:1059`).
const MODIFY_BAR: &str = "Keep Exit";
const MODIFY_PROMPT: &str = "Modify: ";

/// ★ `modifyPlayer` (`ovr018.cs:999-1428`) as a parked screen.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModifyCharacter {
    member: usize,
    field: u8,
    /// `name_cursor_pos`, 1-based into the name (`:1024`).
    cursor: usize,
    /// The undo state every `Exit`/Esc restores (`:1017-1022`).
    backup: Box<Character>,
    status: Option<String>,
    return_to: ReturnTo,
}

impl ModifyCharacter {
    pub fn new(member: usize, ch: &Character, return_to: ReturnTo) -> Self {
        ModifyCharacter {
            member,
            field: 0, // `edited_stat = 0; draw_highlight_stat(true, ...)` (`:1027`)
            cursor: 1,
            backup: Box::new(ch.clone()),
            status: None,
            return_to,
        }
    }

    fn exit(&self) -> ScreenTransition {
        match self.return_to {
            ReturnTo::StartMenu => ScreenTransition::ToStartMenu,
            _ => ScreenTransition::Exit,
        }
    }

    fn field_label(&self) -> &'static str {
        match self.field {
            0 => "STR",
            1 => "INT",
            2 => "WIS",
            3 => "DEX",
            4 => "CON",
            5 => "CHA",
            FIELD_HP => "HP",
            _ => "NAME",
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if self.member >= ctx.roster.members.len() {
            return self.exit();
        }
        // The sheet, with the edited field named on the status row — the
        // original inverts the field in place (`draw_highlight_stat`,
        // `:965-996`); naming it is the same information on our surface.
        ctx.fb.clear(0);
        let view = crate::charsheet::sheet_view(&ctx.roster.members[self.member]);
        crate::charsheet::render_sheet(ctx.fb, ctx.font, ctx.symbols, &view);
        let editing = format!("Editing: {}", self.field_label());
        crate::text::draw_string(ctx.fb, ctx.font, &editing, 0x16, 2, 0, 15);
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 0x17, 2, 0, 14);
        }
        crate::combat::scene::render::clear_prompt_line(ctx.fb);
        crate::text::draw_string(ctx.fb, ctx.font, MODIFY_PROMPT, 0x18, 0, 0, 13);
        crate::combat::scene::render::draw_menu_line_at(
            ctx.fb,
            ctx.font,
            MODIFY_BAR,
            None,
            MODIFY_PROMPT.len(),
        );

        // `modifyPlayer` reads raw keys, not a menu widget: the same keystroke
        // means different things in the name field and out of it (`:1033-1060`).
        let Some(key) = ctx.input.read_key() else {
            return ScreenTransition::Stay;
        };
        use crate::input::InputEvent;
        match key {
            InputEvent::Ext(ext) => {
                self.control_key(ext.ctrl_code(), ctx);
                ScreenTransition::Stay
            }
            // `if (inputkey == 0x0d) { edited_stat++; ... }` (`:1284-1292`).
            InputEvent::Enter => {
                self.field = (self.field + 1) % FIELD_COUNT;
                ScreenTransition::Stay
            }
            // `else if (inputkey == 0x08)` — backspace edits the name only.
            InputEvent::Backspace => {
                if self.field == FIELD_NAME && self.cursor > 1 {
                    let ch = &mut ctx.roster.members[self.member];
                    let del = self.cursor - 1;
                    if del <= ch.name.len() {
                        ch.name.remove(del - 1);
                    }
                    self.cursor = self.cursor.min(ch.name.len().max(1));
                }
                ScreenTransition::Stay
            }
            // `else if (inputkey == 0)` — Esc restores everything and returns.
            InputEvent::Escape => {
                ctx.roster.members[self.member] = (*self.backup).clone();
                self.exit()
            }
            InputEvent::Char(c) => {
                let upper = c.to_ascii_uppercase();
                if self.field == FIELD_NAME {
                    // `if (name_cursor_pos <= 15)` — insert and advance.
                    let ch = &mut ctx.roster.members[self.member];
                    if self.cursor <= 15 {
                        let at = (self.cursor - 1).min(ch.name.len());
                        ch.name.insert(at, c as char);
                        ch.name.truncate(15);
                        self.cursor = (self.cursor + 1).min(15);
                    }
                    return ScreenTransition::Stay;
                }
                match upper {
                    // `while (controlkey == true || inputkey != 0x4B)` (`:1385`)
                    // — 'K' (Keep) is the only ordinary exit.
                    b'K' => {
                        self.finish(ctx);
                        self.exit()
                    }
                    // `else if (inputkey == 0x45)` (`:1355`) — 'E' undoes.
                    b'E' => {
                        ctx.roster.members[self.member] = (*self.backup).clone();
                        self.exit()
                    }
                    _ => ScreenTransition::Stay,
                }
            }
        }
    }

    /// The extended-key arm (`:1064-1281`): O/G move between fields, K/M step
    /// the current one down/up.
    ///
    /// The original's `'S'` case (delete a character from the name, `:1068`)
    /// is a Ctrl+Left scancode our [`crate::input::ExtKey`] set does not carry
    /// — Backspace does the same job here and is named where it lands.
    fn control_key(&mut self, code: u8, ctx: &mut FlowCtx) {
        match code {
            b'O' => self.field = (self.field + 1) % FIELD_COUNT,
            b'G' => self.field = (self.field + FIELD_COUNT - 1) % FIELD_COUNT,
            b'K' => self.step(-1, ctx),
            b'M' => self.step(1, ctx),
            _ => {}
        }
    }

    /// One `Inc`/`Dec` on the current field, re-clamped exactly as
    /// `modifyPlayer` re-clamps (`:1103-1279`).
    fn step(&mut self, delta: i32, ctx: &mut FlowCtx) {
        let rules = ctx.rules;
        let flavor = Adnd1::new(rules);
        let ch = &mut ctx.roster.members[self.member];
        match self.field {
            0..=5 => {
                let stat = self.field as usize;
                let current = read_stat(ch, stat) as i32;
                // ★ STR down with a percentile in hand spends the percentile
                // first (`:1114-1124`): `Str00.Dec(); Str.Inc();` — the net
                // effect is 18/98 → 18/97, not 18/00 → 17.
                if stat == 0 && delta < 0 && ch.stats.str_exceptional.current > 0 {
                    ch.stats.str_exceptional.current -= 1;
                    ch.stats.str_exceptional.original = ch.stats.str_exceptional.current;
                    return;
                }
                let raw = (current + delta).clamp(0, 255) as u8;
                let clamped = flavor.clamp_stat_for_creation(
                    stat_tag(stat),
                    ch.race as usize,
                    ch.sex as usize,
                    ch.class_id as usize,
                    raw,
                );
                write_stat(ch, stat, clamped);
                // ★ STR up past 18 for a warrior starts the percentile
                // (`:1203-1215`); anything else zeroes it (`Str00.Load(0)`).
                if stat == 0 {
                    let warrior =
                        ch.class_level[2] > 0 || ch.class_level[3] > 0 || ch.class_level[4] > 0;
                    if clamped == 18 && warrior && delta > 0 {
                        let next = ch.stats.str_exceptional.current.saturating_add(1);
                        ch.stats.str_exceptional.current = next;
                        ch.stats.str_exceptional.original = next;
                    } else if delta > 0 {
                        ch.stats.str_exceptional = Default::default();
                    }
                }
                if stat == 4 {
                    // CON moved: the HP window moves with it (`:1147-1161`,
                    // `:1235-1247`), and the original jumps the highlight to
                    // the HP field and back so the new value is repainted.
                    creation::reclac_class_bonuses(ch, rules);
                    clamp_hp(ch, rules);
                }
                if stat == 2 {
                    // WIS moved: a cleric's level-1 slot count is re-seeded
                    // (`:1136-1139`, `:1225-1228`) before the full recompute.
                    creation::reclac_class_bonuses(ch, rules);
                }
            }
            FIELD_HP => {
                let next = (ch.hit_point_max as i32 + delta).clamp(1, 255) as u8;
                ch.hit_point_max = next;
                clamp_hp(ch, rules);
            }
            _ => {
                // The name field's K/M move the cursor (`:1180-1190`,
                // `:1267-1277`).
                let len = ctx.roster.members[self.member].name.len().max(1);
                if delta < 0 {
                    self.cursor = if self.cursor == 1 {
                        len
                    } else {
                        self.cursor - 1
                    };
                } else {
                    self.cursor = if self.cursor == len + 1 {
                        1
                    } else {
                        self.cursor + 1
                    };
                }
            }
        }
    }

    /// `modifyPlayer`'s tail (`:1387-1420`): the cleric slots are recomputed,
    /// `npcTreasureShareCount` is set to 1, and `hit_point_rolled` is
    /// back-solved from the (possibly hand-edited) maximum.
    fn finish(&mut self, ctx: &mut FlowCtx) {
        let rules = ctx.rules;
        let ch = &mut ctx.roster.members[self.member];
        creation::reclac_class_bonuses(ch, rules);
        ch.status.npc_treasure_share_count = 1;
        let flavor = Adnd1::new(rules);
        let classes = ch.class_levels();
        let con_total: i32 = flavor.con_hp_adjustment(&classes, ch.stats.con.current);
        let count = classes.len().max(1) as i32;
        let rolled = ch.hit_point_max as i32 - (con_total / count);
        ch.hit_point_rolled = rolled.clamp(0, 255) as u8;
        ch.hit_point_current = ch.hit_point_max;
    }
}

/// `calc_max_hp`'s ceiling and `sub_506BA`'s floor, applied together
/// (`ovr018.cs:1151-1178`, `:1238-1265`).
fn clamp_hp(ch: &mut Character, rules: &gbx_rules::pack::RuleSet) {
    let flavor = Adnd1::new(rules);
    let classes = ch.class_levels();
    let ceiling = flavor.max_hp_ceiling(&classes, ch.stats.con.current) as i32;
    if ceiling > 0 && (ch.hit_point_max as i32) > ceiling {
        ch.hit_point_max = ceiling.clamp(1, 255) as u8;
    }
    if ch.hit_point_max == 0 {
        ch.hit_point_max = 1;
    }
    ch.hit_point_current = ch.hit_point_max;
}

fn stat_tag(stat: usize) -> gbx_rules::flavor::AbilityStat {
    use gbx_rules::flavor::AbilityStat::*;
    [Str, Int, Wis, Dex, Con, Cha][stat]
}

fn read_stat(ch: &Character, stat: usize) -> u8 {
    match stat {
        0 => ch.stats.str_score.current,
        1 => ch.stats.int.current,
        2 => ch.stats.wis.current,
        3 => ch.stats.dex.current,
        4 => ch.stats.con.current,
        _ => ch.stats.cha.current,
    }
}

fn write_stat(ch: &mut Character, stat: usize, value: u8) {
    let pair = crate::party::AbilityScorePair {
        current: value,
        original: value,
    };
    match stat {
        0 => ch.stats.str_score = pair,
        1 => ch.stats.int = pair,
        2 => ch.stats.wis = pair,
        3 => ch.stats.dex = pair,
        4 => ch.stats.con = pair,
        _ => ch.stats.cha = pair,
    }
}

// ---------------------------------------------------------------------------
// Human Change Classes (DuelClass)
// ---------------------------------------------------------------------------

/// The classes `ch` may dual-class INTO (`SecondClassAllowed`,
/// `ovr026.cs:558-599`, via `Flavor::dual_class_eligible`): the old class's
/// prime requisites must all exceed 14, the new class's must all exceed 16,
/// and the new class must accept the character's alignment.
pub fn dual_class_choices(ch: &Character, rules: &gbx_rules::pack::RuleSet) -> Vec<u8> {
    let flavor = Adnd1::new(rules);
    let Some(current) = first_class(ch) else {
        return Vec::new();
    };
    creation::class_choices(rules, ch.race)
        .into_iter()
        // Only the single base classes are dual-class targets: `DuelClass`
        // resolves its pick with `while (newClass <= 7 && ...)`
        // (`ovr026.cs:646`), so a combo id could never be found.
        .filter(|&c| c <= 7)
        .filter(|&c| {
            flavor.dual_class_eligible(
                current,
                c as usize,
                ch.stats.to_stat_block(),
                ch.alignment as usize,
            )
        })
        .collect()
}

/// `HumanCurrentClass_Unknown` (`ovr026.cs:702-718`): the first base class
/// with a level, for a human; nothing for anyone else.
fn first_class(ch: &Character) -> Option<usize> {
    const HUMAN: u8 = 7;
    if ch.race != HUMAN {
        return None;
    }
    (0..8).find(|&c| ch.class_level[c] > 0)
}

/// ★ `DuelClass` (`ovr026.cs:603-699`) as a parked screen.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DualClass {
    member: usize,
    menu: Option<Widget>,
    choices: Vec<u8>,
    status: Option<String>,
    return_to: ReturnTo,
}

impl DualClass {
    pub fn new(
        member: usize,
        ch: &Character,
        rules: &gbx_rules::pack::RuleSet,
        return_to: ReturnTo,
    ) -> Self {
        let choices = dual_class_choices(ch, rules);
        let menu = if choices.is_empty() {
            None
        } else {
            let mut items = vec![ListItem::Heading("Pick New Class".to_string())];
            items.extend(
                choices
                    .iter()
                    .map(|&c| ListItem::Entry(creation::CLASS_NAMES[c as usize].to_string())),
            );
            Some(Widget::ListMenu(ListMenu::boxed(
                items,
                ListLayout {
                    start_row: 2,
                    start_col: 1,
                    end_row: 0x16,
                    end_col: 0x26,
                },
            )))
        };
        // `if (list.Count == 1) DisplayStatusText(... " doesn't qualify.")`
        let status = menu
            .is_none()
            .then(|| format!("{} doesn't qualify.", ch.name));
        DualClass {
            member,
            menu,
            choices,
            status,
            return_to,
        }
    }

    fn exit(&self) -> ScreenTransition {
        match self.return_to {
            ReturnTo::StartMenu => ScreenTransition::ToStartMenu,
            _ => ScreenTransition::Exit,
        }
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        if self.member >= ctx.roster.members.len() {
            return self.exit();
        }
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 0x14, 2, 0, 14);
        }
        let Some(menu) = &mut self.menu else {
            // The refusal needs acknowledging before the menu comes back.
            crate::combat::scene::render::draw_prompt(ctx.fb, ctx.font, "Press any key");
            return if ctx.input.read_key().is_some() {
                self.exit()
            } else {
                ScreenTransition::Stay
            };
        };
        if let Widget::ListMenu(list) = menu {
            crate::shell::draw_list_menu(ctx.fb, ctx.font, list);
            let line = format!("Select{} Exit", list.prompt_words());
            let span = crate::widgets::build_words(&line).first().copied();
            crate::combat::scene::render::draw_menu_line(ctx.fb, ctx.font, &line, span);
        }
        match menu.tick(ctx.input, ctx.dt_ticks) {
            WidgetOutcome::ListSelected { index, key: b'S' } => {
                let new_class = self.choices[index - 1] as usize;
                let rules = ctx.rules;
                let items = crate::items::load_table(ctx.data);
                let ch = &mut ctx.roster.members[self.member];
                apply_dual_class(ch, new_class, rules, &items);
                self.status = Some(format!(
                    "{} is now a 1st level {}.",
                    ch.name,
                    creation::CLASS_NAMES[new_class]
                ));
                self.menu = None;
                ScreenTransition::Stay
            }
            WidgetOutcome::ListCancelled => self.exit(),
            _ => ScreenTransition::Stay,
        }
    }
}

/// ★ `DuelClass`'s record surgery (`ovr026.cs:642-698`): the old class is
/// banked in `ClassLevelsOld`, `multiclassLevel` remembers the level reached,
/// the new class starts at 1 with **zero** XP, the spell state is wiped and
/// re-seeded, and every item the new class may not use is un-readied.
pub fn apply_dual_class(
    ch: &mut Character,
    new_class: usize,
    rules: &gbx_rules::pack::RuleSet,
    items: &gbx_formats::items::ItemDataTable,
) {
    let Some(old) = first_class(ch) else {
        return;
    };
    ch.exp = 0;
    ch.combat.attacks.base[0] = 2; // `attacksCount = 2`
    ch.class_levels_old[old] = ch.class_level[old];
    ch.multiclass_level = ch.hit_dice;
    ch.hit_dice = 1;
    ch.class_level[old] = 0;
    ch.class_level[new_class] = 1;
    ch.class_id = new_class as u8;

    ch.magic.cast_count = [[0; 5]; 3];
    ch.magic.spell_list = vec![0u8; crate::magic::SPELL_LIST_SIZE];
    if new_class == 0 {
        ch.magic.cast_count[0][0] = 1;
    } else if new_class == 5 {
        ch.magic.cast_count[2][0] = 1;
        // ★ THREE spells here, not creation's four: `DuelClass` grants
        // detect magic, read magic and sleep (`ovr026.cs:675-677`) and leaves
        // `enlarge` out, which `createPlayer` does hand a new magic-user.
        for id in [0x0Bu8, 0x12, 0x15] {
            crate::magic::learn_spell(ch, id);
        }
    }

    creation::reclac_class_bonuses(ch, rules);

    // `foreach (item ...) if ((ItemDataTable[type].classFlags & classFlags)
    // == 0 && !cursed) item.readied = false;` (`ovr026.cs:691-698`).
    let flags = ch.skills.class_flags;
    let mut unready = Vec::new();
    for (i, item) in ch.items.iter().enumerate() {
        let type_id = gbx_formats::save_orig::item_type(item);
        let cursed = gbx_formats::save_orig::item_is_cursed(item);
        if items.get(type_id).class_flags & flags == 0 && !cursed {
            unready.push(i);
        }
    }
    for i in unready {
        gbx_formats::save_orig::set_item_readied(&mut ch.items[i], false);
        ch.readied_items.remove(&i);
    }
}

/// Wraps the modify screen for the shell's `Screen` enum.
pub fn modify_screen(member: usize, ch: &Character, return_to: ReturnTo) -> Screen {
    Screen::ModifyCharacter(Box::new(ModifyCharacter::new(member, ch, return_to)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::EngineRng;
    use gbx_rules::pack::RuleSet;

    fn rules() -> RuleSet {
        RuleSet::load()
    }

    fn fresh(class_id: u8) -> Character {
        let r = rules();
        let mut rng = EngineRng::new(9);
        creation::create(
            &r,
            &mut rng,
            creation::Picks {
                race: 7,
                sex: 0,
                class_id,
                alignment: creation::alignment_choices(&r, class_id)[0],
            },
        )
    }

    /// ★ The modify window: open on a brand-new character, shut the moment
    /// they earn anything at all.
    #[test]
    fn modify_is_allowed_only_at_a_creation_experience_total() {
        let mut ch = fresh(2);
        assert_eq!(ch.exp, 25_000);
        assert!(can_modify(&ch));
        ch.exp = 25_001;
        assert!(!can_modify(&ch), "one earned XP closes the window");
        ch.exp = 12_500;
        assert!(can_modify(&ch), "a two-class creation total");
        ch.exp = 8_333;
        assert!(can_modify(&ch), "a three-class creation total");
        ch.exp = 0;
        assert!(can_modify(&ch), "a freshly dual-classed character");
        ch.multiclass_level = 4;
        assert!(!can_modify(&ch), "...until they have banked a class");
    }

    #[test]
    fn can_dual_class_is_human_and_unbanked() {
        let mut ch = fresh(2);
        assert!(can_dual_class(&ch));
        ch.class_levels_old[6] = 3;
        assert!(!can_dual_class(&ch));
        ch.class_levels_old = [0; 8];
        ch.race = 1; // dwarf
        assert!(!can_dual_class(&ch));
    }

    /// A fighter with mediocre stats qualifies for nothing; one with 17s
    /// everywhere qualifies for the classes their alignment allows.
    #[test]
    fn dual_class_choices_need_both_sets_of_prime_requisites() {
        let r = rules();
        let mut ch = fresh(2);
        for stat in 0..6 {
            write_stat(&mut ch, stat, 12);
        }
        assert!(
            dual_class_choices(&ch, &r).is_empty(),
            "12s qualify for nothing"
        );
        for stat in 0..6 {
            write_stat(&mut ch, stat, 17);
        }
        ch.alignment = 0; // Lawful Good
        let choices = dual_class_choices(&ch, &r);
        assert!(!choices.is_empty(), "17s qualify for something");
        assert!(!choices.contains(&2), "never the class you already are");
        for c in &choices {
            assert!(*c <= 7, "only base classes are dual-class targets");
        }
    }

    /// The record surgery: the old class is banked, the new one starts at 1
    /// with no experience, and the magic-user grant is the DuelClass three
    /// (no `enlarge`).
    #[test]
    fn apply_dual_class_banks_the_old_class_and_reseeds_the_new() {
        let r = rules();
        let mut ch = fresh(2); // fighter
        let old_level = ch.class_level[2];
        assert!(old_level > 1, "creation trained the fighter up");
        let items = gbx_formats::items::ItemDataTable::parse(&[0, 0]).unwrap();
        apply_dual_class(&mut ch, 5, &r, &items); // -> magic-user

        assert_eq!(ch.class_levels_old[2], old_level, "the fighter is banked");
        assert_eq!(ch.class_level[2], 0);
        assert_eq!(ch.class_level[5], 1);
        assert_eq!(ch.multiclass_level, old_level);
        assert_eq!(ch.hit_dice, 1);
        assert_eq!(ch.exp, 0);
        assert_eq!(ch.class_id, 5);
        let known: Vec<usize> = ch
            .magic
            .spell_book
            .iter()
            .enumerate()
            .filter(|(_, &b)| b != 0)
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(
            known,
            vec![0x0B, 0x12, 0x15],
            "detect magic, read magic, sleep"
        );
        assert_eq!(ch.magic.cast_count[2][0], 1);
    }
}
