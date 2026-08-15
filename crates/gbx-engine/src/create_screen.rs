//! ★ **`startGameMenu`'s four character verbs** (roll-credits slice 9c), as
//! parked screens: Create (`createPlayer`, `ovr018.cs:325-891`), Modify
//! (`modifyPlayer`, `:999-1428`), Human Change Classes (`DuelClass`,
//! `ovr026.cs:603-699`) and Add Character to Party (`AddPlayer`,
//! `ovr018.cs:1431-1577`).
//!
//! Each is a stage machine over the widgets the original's own menu helpers
//! imply: `sl_select_item`'s boxed vertical list ([`ListMenu`]) for every
//! picker, `yes_no` for every confirmation, `getUserInputString` for the name.
//! The record work itself is [`crate::creation`]; this module is presentation
//! and control flow only.

use crate::chr_file::{CharFileDirectory, CharFileRequest};
use crate::creation::{self, Picks};
use crate::draw::{blit_image, Clip};
use crate::party::Character;
use crate::screens::{ReturnTo, Screen, ScreenTransition};
use crate::shell::FlowCtx;
use crate::widgets::{Hotbar, ListItem, ListLayout, ListMenu, TextEntry, Widget, WidgetOutcome};

/// `sl_select_item(..., 22, 38, 2, 1, ...)` (`ovr018.cs:368-369`) — endY,
/// endX, startY, startX, in that argument order.
const PICKER_BOX: ListLayout = ListLayout {
    start_row: 2,
    start_col: 1,
    end_row: 22,
    end_col: 38,
};

/// The word `sl_select_item` is handed as its `inputString` at every one of
/// `createPlayer`'s pickers (`ovr018.cs:369`) — and the only key that commits.
const PICKER_PROMPT: &str = "Select";
const PICKER_COMMIT: u8 = b'S';

fn picker(heading: &str, rows: impl IntoIterator<Item = String>) -> Widget {
    let mut items = vec![ListItem::Heading(heading.to_string())];
    // `var_C.Add(new MenuItem("  " + ...))` — the two-space indent is the
    // original's, and `draw_list_menu` preserves it.
    items.extend(rows.into_iter().map(|r| ListItem::Entry(format!("  {r}"))));
    Widget::ListMenu(ListMenu::boxed(items, PICKER_BOX))
}

/// `sl_select_item`'s own prompt line: the caller's word, then `" Next"`/
/// `" Prev"` as the window allows, then `" Exit"` (`ovr027.cs:585-604`).
fn draw_picker(ctx: &mut FlowCtx, menu: &Widget) {
    let Widget::ListMenu(list) = menu else {
        return;
    };
    crate::shell::draw_list_menu(ctx.fb, ctx.font, list);
    let line = format!("{PICKER_PROMPT}{} Exit", list.prompt_words());
    let span = crate::widgets::build_words(&line).first().copied();
    crate::combat::scene::render::draw_menu_line(ctx.fb, ctx.font, &line, span);
}

/// `yes_no(colors, prompt)` (`ovr027.cs:676`) — two words, `No` highlighted
/// (`gbl.menuSelectedWord = 2`).
fn confirm(prompt: &str) -> Confirm {
    let mut hotbar = Hotbar::new("Yes No");
    hotbar.seed_selected_word(2);
    Confirm {
        prompt: prompt.to_string(),
        menu: Widget::Hotbar(hotbar),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Confirm {
    prompt: String,
    menu: Widget,
}

impl Confirm {
    fn draw(&self, ctx: &mut FlowCtx) {
        crate::combat::scene::render::clear_prompt_line(ctx.fb);
        crate::text::draw_string(ctx.fb, ctx.font, &self.prompt, 0x18, 0, 0, 13);
        if let Widget::Hotbar(h) = &self.menu {
            crate::combat::scene::render::draw_menu_line_at(
                ctx.fb,
                ctx.font,
                &h.text,
                h.selected_span(),
                self.prompt.len(),
            );
        }
    }

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
}

// ---------------------------------------------------------------------------
// Create New Character (createPlayer)
// ---------------------------------------------------------------------------

/// Which of `createPlayer`'s blocking loops the screen is parked on.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum Stage {
    /// `:352-383` — "Pick Race".
    Race(Widget),
    /// `:425-450` — "Pick Gender".
    Sex(Widget),
    /// `:453-484` — "Pick Class", filtered by `RaceClasses[race]`.
    Class(Widget),
    /// `:588-618` — "Pick Alignment", filtered by `class_alignments`.
    Alignment(Widget),
    /// `:657-865` — the rolled sheet plus `yes_no("Reroll stats? ")`.
    Review(Confirm),
    /// `:869-872` — `getUserInputString(15, ...)`, re-prompting on empty.
    Name(TextEntry),
    /// `:874` — `icon_builder()`.
    Icon(Box<IconEditor>),
    /// `:883-888` — `yes_no("Save <name>? ")`, then `SavePlayer`.
    Save(Confirm),
    /// The flow is over (or was abandoned) — a parking slot the screen never
    /// renders, so a stage can be taken out by value and put back.
    Done,
}

/// ★ `createPlayer` (`ovr018.cs:325-891`) as a parked screen.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateCharacter {
    stage: Stage,
    race: u8,
    sex: u8,
    class_id: u8,
    /// The record, once the class pick has let [`creation::begin`] build it.
    draft: Option<Box<Character>>,
    status: Option<String>,
    return_to: ReturnTo,
}

impl CreateCharacter {
    pub fn new(return_to: ReturnTo) -> Self {
        CreateCharacter {
            stage: Stage::Race(picker(
                "Pick Race",
                creation::CREATABLE_RACES
                    .iter()
                    .map(|&r| creation::RACE_NAMES[r as usize].to_string()),
            )),
            race: 0,
            sex: 0,
            class_id: 0,
            draft: None,
            status: None,
            return_to,
        }
    }

    fn exit(&self) -> ScreenTransition {
        match self.return_to {
            ReturnTo::StartMenu => ScreenTransition::ToStartMenu,
            ReturnTo::GameOver => ScreenTransition::ToGameOver,
            _ => ScreenTransition::Exit,
        }
    }

    /// The full sheet the reroll loop shows (`playerDisplayFull` +
    /// `display_player_stats01` + `displayMoney`, `:655,860-861`).
    fn draw_sheet(&self, ctx: &mut FlowCtx) {
        let Some(draft) = &self.draft else {
            return;
        };
        ctx.fb.clear(0);
        let view = crate::charsheet::sheet_view(draft);
        crate::charsheet::render_sheet(ctx.fb, ctx.font, ctx.symbols, &view);
    }

    /// One tick. The stage is taken out by value so a stage that owns a
    /// widget and the draft that widget edits can both be borrowed at once —
    /// the returned stage is what the screen parks on next.
    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        let stage = std::mem::replace(&mut self.stage, Stage::Done);
        let (next, transition) = self.advance(stage, ctx);
        self.stage = next;
        transition
    }

    fn advance(&mut self, stage: Stage, ctx: &mut FlowCtx) -> (Stage, ScreenTransition) {
        match stage {
            Stage::Done => (Stage::Done, self.exit()),
            Stage::Race(mut menu) => {
                self.paint_picker(ctx, &menu);
                match menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::ListSelected { index, key } if key == PICKER_COMMIT => {
                        // `if (index == 6) index++;` (`:378-381`) — the sixth
                        // row is race 7, because Half-Orc is not offered.
                        self.race = creation::CREATABLE_RACES[index - 1];
                        (
                            Stage::Sex(picker(
                                "Pick Gender",
                                creation::SEX_NAMES.iter().map(|s| s.to_string()),
                            )),
                            ScreenTransition::Stay,
                        )
                    }
                    // `if (input_key == 0) { return; }` — Exit/Esc abandons
                    // the whole flow, at every picker (`:371-375,440-445,...`).
                    WidgetOutcome::ListCancelled => (Stage::Done, self.exit()),
                    _ => (Stage::Race(menu), ScreenTransition::Stay),
                }
            }
            Stage::Sex(mut menu) => {
                self.paint_picker(ctx, &menu);
                match menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::ListSelected { index, key } if key == PICKER_COMMIT => {
                        self.sex = (index - 1) as u8;
                        let classes = creation::class_choices(ctx.rules, self.race);
                        (
                            Stage::Class(picker(
                                "Pick Class",
                                classes
                                    .iter()
                                    .map(|&c| creation::CLASS_NAMES[c as usize].to_string()),
                            )),
                            ScreenTransition::Stay,
                        )
                    }
                    WidgetOutcome::ListCancelled => (Stage::Done, self.exit()),
                    _ => (Stage::Sex(menu), ScreenTransition::Stay),
                }
            }
            Stage::Class(mut menu) => {
                self.paint_picker(ctx, &menu);
                match menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::ListSelected { index, key } if key == PICKER_COMMIT => {
                        self.class_id = creation::class_choices(ctx.rules, self.race)[index - 1];
                        let alignments = creation::alignment_choices(ctx.rules, self.class_id);
                        (
                            Stage::Alignment(picker(
                                "Pick Alignment",
                                alignments
                                    .iter()
                                    .map(|&a| creation::ALIGNMENT_NAMES[a as usize].to_string()),
                            )),
                            ScreenTransition::Stay,
                        )
                    }
                    WidgetOutcome::ListCancelled => (Stage::Done, self.exit()),
                    _ => (Stage::Class(menu), ScreenTransition::Stay),
                }
            }
            Stage::Alignment(mut menu) => {
                self.paint_picker(ctx, &menu);
                match menu.tick(ctx.input, ctx.dt_ticks) {
                    WidgetOutcome::ListSelected { index, key } if key == PICKER_COMMIT => {
                        let alignment =
                            creation::alignment_choices(ctx.rules, self.class_id)[index - 1];
                        let picks = Picks {
                            race: self.race,
                            sex: self.sex,
                            class_id: self.class_id,
                            alignment,
                        };
                        let mut ch = creation::begin(ctx.rules, ctx.rng, picks);
                        creation::reroll(&mut ch, ctx.rules, ctx.rng);
                        self.draft = Some(Box::new(ch));
                        (
                            Stage::Review(confirm(REROLL_PROMPT)),
                            ScreenTransition::Stay,
                        )
                    }
                    WidgetOutcome::ListCancelled => (Stage::Done, self.exit()),
                    _ => (Stage::Alignment(menu), ScreenTransition::Stay),
                }
            }
            Stage::Review(mut c) => {
                self.draw_sheet(ctx);
                c.draw(ctx);
                match c.tick(ctx) {
                    // `while (input_key != 'N')` (`:865`) — Yes rerolls.
                    Some(true) => {
                        if let Some(draft) = &mut self.draft {
                            creation::reroll(draft, ctx.rules, ctx.rng);
                        }
                        (
                            Stage::Review(confirm(REROLL_PROMPT)),
                            ScreenTransition::Stay,
                        )
                    }
                    // `do { name = getUserInputString(15, 0, 13, "Character
                    // name: "); } while (name.Length == 0);` (`:869-872`).
                    Some(false) => (
                        Stage::Name(TextEntry::new(NAME_PROMPT, 15, false)),
                        ScreenTransition::Stay,
                    ),
                    None => (Stage::Review(c), ScreenTransition::Stay),
                }
            }
            Stage::Name(mut entry) => {
                self.draw_sheet(ctx);
                crate::combat::scene::render::clear_prompt_line(ctx.fb);
                crate::text::draw_string(ctx.fb, ctx.font, &entry.prompt, 0x18, 0, 0, 13);
                crate::text::draw_string(
                    ctx.fb,
                    ctx.font,
                    &String::from_utf8_lossy(&entry.buf),
                    0x18,
                    entry.prompt.len(),
                    0,
                    15,
                );
                match entry.tick(ctx.input) {
                    WidgetOutcome::TextSubmitted(text) => {
                        let name = text.trim().to_string();
                        if name.is_empty() {
                            // The `do`/`while` re-prompts rather than accepting.
                            return (
                                Stage::Name(TextEntry::new(NAME_PROMPT, 15, false)),
                                ScreenTransition::Stay,
                            );
                        }
                        match &mut self.draft {
                            Some(draft) => {
                                creation::finish(draft, &name);
                                (
                                    Stage::Icon(Box::new(IconEditor::new(draft))),
                                    ScreenTransition::Stay,
                                )
                            }
                            None => (Stage::Done, self.exit()),
                        }
                    }
                    _ => (Stage::Name(entry), ScreenTransition::Stay),
                }
            }
            Stage::Icon(mut editor) => {
                let Some(draft) = self.draft.as_mut() else {
                    return (Stage::Done, ScreenTransition::Stay);
                };
                if editor.tick(ctx, draft) {
                    let prompt = format!("Save {}? ", draft.name);
                    (Stage::Save(confirm(&prompt)), ScreenTransition::Stay)
                } else {
                    (Stage::Icon(editor), ScreenTransition::Stay)
                }
            }
            Stage::Save(mut c) => {
                self.draw_sheet(ctx);
                c.draw(ctx);
                match c.tick(ctx) {
                    Some(save) => {
                        if save {
                            if let Some(draft) = self.draft.take() {
                                // `SavePlayer(string.Empty, player)` (`:887`).
                                *ctx.char_io_request = Some(CharFileRequest::Save(draft));
                            }
                        }
                        (Stage::Done, self.exit())
                    }
                    None => (Stage::Save(c), ScreenTransition::Stay),
                }
            }
        }
    }

    fn paint_picker(&self, ctx: &mut FlowCtx, menu: &Widget) {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        draw_picker(ctx, menu);
    }
}

/// `getUserInputString(15, 0, 13, "Character name: ")` (`ovr018.cs:871`).
const NAME_PROMPT: &str = "Character name: ";
/// `yes_no(defaultMenuColors, "Reroll stats? ")` (`ovr018.cs:863`).
const REROLL_PROMPT: &str = "Reroll stats? ";

// ---------------------------------------------------------------------------
// icon_builder (ovr018.cs:1632-1977)
// ---------------------------------------------------------------------------

/// `iconStrings` (`ovr018.cs:1646-1651`) — the editor's five menu bars.
/// Index 3's third word is rewritten per colour nibble ("Hair" for the 1st,
/// "Face" for the 2nd, `:1724,1731`), and index 4's first word is the size
/// the toggle would switch *to*.
const ICON_MENU_PARTS: &str = "Parts 1st-color 2nd-color Size Exit";
const ICON_MENU_HEAD_WEAPON: &str = "Head Weapon Exit";
const ICON_MENU_COLOR_BASE: &str = "Weapon Body {} Shield Arm Leg Exit";
const ICON_MENU_SIZE_TAIL: &str = " Keep Exit";
const ICON_MENU_CYCLE: &str = "Next Prev Keep Exit";

/// Which of `icon_builder`'s five nested menus is on the prompt row
/// (`var_8`, `ovr018.cs:1662,1687-1701`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum IconMenu {
    /// `var_8 == 1`: the top bar.
    Parts,
    /// `var_8 == 2`: Head / Weapon.
    HeadWeapon,
    /// `var_8 == 3`: which body part's colour.
    ColorPart,
    /// `var_8 == 4`: Small/Large.
    Size,
    /// `var_8 == 5`: Next/Prev/Keep/Exit over whatever menu 2 or 3 selected.
    Cycle,
}

/// ★ `icon_builder` (`ovr018.cs:1632-1977`) — the shipped icon editor, as a
/// parked stage machine.
///
/// The commit rule is the original's and is worth stating once: every
/// submenu's **`Keep`** copies the live value into the backup, and its
/// **`Exit`** copies the backup back over the live value. The editor's own
/// tail then restores the backups unconditionally (`:1960-1964`), so a change
/// survives only if it was Kept — and `Is this icon ok? N` starts the whole
/// thing again from the (kept) state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IconEditor {
    menu: IconMenu,
    /// `var_1A` — the menu `Cycle` returns to; `0` means "the editor is done".
    return_menu: Option<IconMenu>,
    /// `var_1B` — `'H'` or `'W'`, which part `Cycle` is stepping.
    cycling: u8,
    /// `color_index` + `second_color` — which of the six colour pairs, and
    /// which nibble of it.
    color_index: usize,
    second_color: bool,
    /// The backups every `Keep`/`Exit` plays against.
    bkup_head: u8,
    bkup_weapon: u8,
    bkup_size: u8,
    bkup_colours: [u8; 6],
    /// The `Is this icon ok?` prompt, once the inner loop has ended.
    ok: Option<Confirm>,
    /// The icon as it was when this pass started — the "old" row.
    #[serde(skip)]
    old_icon: Option<crate::party::IconInfo>,
    hotbar: Widget,
}

impl IconEditor {
    fn new(ch: &Character) -> Self {
        IconEditor {
            menu: IconMenu::Parts,
            return_menu: Some(IconMenu::Parts),
            cycling: 0,
            color_index: 0,
            second_color: false,
            bkup_head: ch.icon.head_icon,
            bkup_weapon: ch.icon.weapon_icon,
            bkup_size: ch.icon.icon_size,
            bkup_colours: ch.icon.colours,
            ok: None,
            old_icon: Some(ch.icon),
            hotbar: Widget::Hotbar(Hotbar::new(ICON_MENU_PARTS)),
        }
    }

    /// The bar text for the current menu (`:1686-1701`).
    fn bar(&self, ch: &Character) -> String {
        match self.menu {
            IconMenu::Parts => ICON_MENU_PARTS.to_string(),
            IconMenu::HeadWeapon => ICON_MENU_HEAD_WEAPON.to_string(),
            IconMenu::ColorPart => {
                ICON_MENU_COLOR_BASE.replace("{}", if self.second_color { "Face" } else { "Hair" })
            }
            // "the OTHER size, then Keep Exit" — the word offers the switch.
            IconMenu::Size => format!(
                "{}{ICON_MENU_SIZE_TAIL}",
                if ch.icon.icon_size == 2 {
                    "Small"
                } else {
                    "Large"
                }
            ),
            IconMenu::Cycle => ICON_MENU_CYCLE.to_string(),
        }
    }

    /// `drawIconEditorIcons(titleY, titleX)` (`ovr018.cs:1611-1622`): the
    /// ready and action poses side by side, three cells apart, at a 24-pixel
    /// cell origin.
    fn draw_icon_row(ctx: &mut FlowCtx, icon: &crate::party::IconInfo, title_y: usize) {
        let Ok(art) = crate::combat_art::load_party_icon(ctx.data, icon, true) else {
            return;
        };
        for (i, pose) in [
            crate::combat_art::IconPose::Normal,
            crate::combat_art::IconPose::Attack,
        ]
        .into_iter()
        .enumerate()
        {
            let sprite = art.frame(pose, 0);
            // `(tileX*3 + 1) * 8` / `(tileY*3 + 1) * 8` — `draw_combat_picture`'s
            // own cell origin (`seg040.cs:22-25`), in 8-pixel cells.
            blit_image(
                ctx.fb,
                &sprite.pixels,
                sprite.width,
                sprite.height,
                title_y * 3 + 1,
                (1 + i * 3) * 3 + 1,
                Clip::FULL,
                None,
                None,
            );
        }
    }

    fn draw(&self, ctx: &mut FlowCtx, ch: &Character) {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        // `:1677-1680` — the four labels.
        for (text, row, col) in [
            ("old", 6, 8),
            ("ready   action", 10, 3),
            ("new", 12, 8),
            ("ready   action", 16, 3),
        ] {
            crate::text::draw_string(ctx.fb, ctx.font, text, row, col, 0, 15);
        }
        if let Some(old) = &self.old_icon {
            Self::draw_icon_row(ctx, old, 2);
        }
        Self::draw_icon_row(ctx, &ch.icon, 4);
        if let Some(ok) = &self.ok {
            ok.draw(ctx);
        } else if let Widget::Hotbar(h) = &self.hotbar {
            crate::combat::scene::render::draw_menu_line(
                ctx.fb,
                ctx.font,
                &h.text,
                h.selected_span(),
            );
        }
    }

    /// `true` once the editor has been left with `Is this icon ok? Y`.
    fn tick(&mut self, ctx: &mut FlowCtx, ch: &mut Character) -> bool {
        self.draw(ctx, ch);

        if let Some(ok) = &mut self.ok {
            return match ok.tick(ctx) {
                Some(true) => true,
                Some(false) => {
                    // `while (inputKey != 'Y')` — another pass, backups
                    // re-taken from the state that was kept.
                    *self = IconEditor::new(ch);
                    false
                }
                None => false,
            };
        }

        // The bar text is recomputed every tick (size, and the colour word).
        let bar = self.bar(ch);
        if let Widget::Hotbar(h) = &mut self.hotbar {
            if h.text != bar {
                let mut fresh = Hotbar::new(bar);
                fresh.accept_ext = true;
                *h = fresh;
            }
        }
        let WidgetOutcome::Hotbar(key) = self.hotbar.tick(ctx.input, ctx.dt_ticks) else {
            return false;
        };
        let key = key.to_ascii_uppercase();
        self.dispatch(key, ch);

        // `while (var_1A != 0 || unk_4FE94.MemberOf(inputKey) == false)`
        // (`:1958`): the inner loop ends only when the TOP menu was exited.
        if self.return_menu.is_none() {
            // `:1960-1967` — restore every backup, then ask.
            ch.icon.head_icon = self.bkup_head;
            ch.icon.weapon_icon = self.bkup_weapon;
            ch.icon.icon_size = self.bkup_size;
            ch.icon.colours = self.bkup_colours;
            self.ok = Some(confirm("Is this icon ok? "));
        }
        false
    }

    fn dispatch(&mut self, key: u8, ch: &mut Character) {
        match self.menu {
            IconMenu::Parts => {
                self.return_menu = Some(IconMenu::Parts);
                match key {
                    b'P' => self.menu = IconMenu::HeadWeapon,
                    b'1' => {
                        self.menu = IconMenu::ColorPart;
                        self.second_color = false;
                    }
                    b'2' => {
                        self.menu = IconMenu::ColorPart;
                        self.second_color = true;
                    }
                    b'S' => self.menu = IconMenu::Size,
                    b'E' | 0 => self.return_menu = None,
                    _ => {}
                }
            }
            IconMenu::HeadWeapon => {
                self.return_menu = Some(IconMenu::HeadWeapon);
                if key == b'E' || key == 0 {
                    self.menu = IconMenu::Parts;
                } else {
                    self.cycling = key;
                    self.menu = IconMenu::Cycle;
                }
            }
            IconMenu::ColorPart => {
                self.return_menu = Some(IconMenu::ColorPart);
                // `:1761-1794` — the part letter picks a colour slot.
                self.color_index = match key {
                    b'W' => 5,
                    b'B' => 0,
                    b'H' | b'F' => 3,
                    b'S' => 4,
                    b'A' => 1,
                    b'L' => 2,
                    _ => 0,
                };
                self.menu = if key == b'E' || key == 0 {
                    IconMenu::Parts
                } else {
                    IconMenu::Cycle
                };
            }
            IconMenu::Size => {
                // `:1806-1836` — L/S set it live, K banks it, E/Esc reverts.
                match key {
                    b'L' => ch.icon.icon_size = 2,
                    b'S' => ch.icon.icon_size = 1,
                    b'K' => {
                        self.bkup_size = ch.icon.icon_size;
                        self.menu = IconMenu::Parts;
                    }
                    b'E' | 0 => {
                        ch.icon.icon_size = self.bkup_size;
                        self.menu = IconMenu::Parts;
                    }
                    _ => {}
                }
            }
            IconMenu::Cycle => match self.return_menu {
                Some(IconMenu::HeadWeapon) => self.cycle_part(key, ch),
                Some(IconMenu::ColorPart) => self.cycle_colour(key, ch),
                _ => {}
            },
        }
    }

    /// `:1841-1906` — Head wraps 0..=13 (`WrapMinMax`), Weapon 0..=0x1F.
    fn cycle_part(&mut self, key: u8, ch: &mut Character) {
        const HEAD_MAX: u8 = 13;
        const WEAPON_MAX: u8 = 0x1F;
        let (value, max, backup): (&mut u8, u8, &mut u8) = if self.cycling == b'H' {
            (&mut ch.icon.head_icon, HEAD_MAX, &mut self.bkup_head)
        } else if self.cycling == b'W' {
            (&mut ch.icon.weapon_icon, WEAPON_MAX, &mut self.bkup_weapon)
        } else {
            return;
        };
        match key {
            b'N' => *value = if *value < max { *value + 1 } else { 0 },
            b'P' => *value = if *value > 0 { *value - 1 } else { max },
            b'K' => {
                *backup = *value;
                self.menu = IconMenu::HeadWeapon;
            }
            b'E' | 0 => {
                *value = *backup;
                self.menu = IconMenu::HeadWeapon;
            }
            _ => {}
        }
    }

    /// `:1908-1951` — the chosen nibble of `icon_colours[color_index]` steps
    /// mod 16; the whole six-entry array is what Keep/Exit swaps.
    fn cycle_colour(&mut self, key: u8, ch: &mut Character) {
        let cell = &mut ch.icon.colours[self.color_index];
        let (mut low, mut high) = (*cell & 0x0F, (*cell & 0xF0) >> 4);
        match key {
            b'N' => {
                if self.second_color {
                    high = (high + 1) % 16;
                } else {
                    low = (low + 1) % 16;
                }
                *cell = low + (high << 4);
            }
            b'P' => {
                if self.second_color {
                    high = high.wrapping_sub(1) & 0x0F;
                } else {
                    low = low.wrapping_sub(1) & 0x0F;
                }
                *cell = low + (high << 4);
            }
            b'K' => {
                self.bkup_colours = ch.icon.colours;
                self.menu = IconMenu::ColorPart;
            }
            b'E' | 0 => {
                ch.icon.colours = self.bkup_colours;
                self.menu = IconMenu::ColorPart;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Add Character to Party (AddPlayer)
// ---------------------------------------------------------------------------

/// ★ `AddPlayer` (`ovr018.cs:1431-1577`).
///
/// The original opens with `displayInput("Curse Pool Hillsfar Exit", "Add from
/// where? ")` — three import sources. Only **Curse** (`*.guy`) is wired here;
/// Pool of Radiance (`*.cha`/`*.sav`) and Hillsfar (`*.hil`) need their own
/// foreign record decoders (`ConvertPoolRadPlayer`/`ConvertHillsFarPlayer`,
/// `ovr017.cs:234-459`), which is a slice of its own. Both are on the bar and
/// both say so when picked, rather than silently missing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddCharacter {
    source: Option<Widget>,
    list: Option<Widget>,
    status: Option<String>,
    return_to: ReturnTo,
}

impl AddCharacter {
    pub fn new(return_to: ReturnTo) -> Self {
        AddCharacter {
            source: Some(Widget::Hotbar(Hotbar::new("Curse Pool Hillsfar Exit"))),
            list: None,
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

    fn build_list(files: &CharFileDirectory) -> Widget {
        let rows = files.entries.iter().map(|e| {
            // `select_sl.Text = "* " + select_sl.Text;` marks a taken row.
            if e.taken {
                format!("* {}", e.name)
            } else {
                format!("  {}", e.name)
            }
        });
        let mut items = vec![ListItem::Heading("Add a character".to_string())];
        items.extend(rows.map(ListItem::Entry));
        Widget::ListMenu(ListMenu::boxed(items, PICKER_BOX))
    }

    pub fn tick(&mut self, ctx: &mut FlowCtx) -> ScreenTransition {
        ctx.fb.clear(0);
        let _ = crate::frames::draw_frame_outer(ctx.fb, ctx.symbols);
        if let Some(s) = &self.status {
            crate::text::draw_string(ctx.fb, ctx.font, s, 0x16, 2, 0, 14);
        }

        if let Some(source) = &mut self.source {
            crate::combat::scene::render::clear_prompt_line(ctx.fb);
            crate::text::draw_string(ctx.fb, ctx.font, "Add from where? ", 0x18, 0, 0, 13);
            if let Widget::Hotbar(h) = source {
                crate::combat::scene::render::draw_menu_line_at(
                    ctx.fb,
                    ctx.font,
                    &h.text,
                    h.selected_span(),
                    "Add from where? ".len(),
                );
            }
            return match source.tick(ctx.input, ctx.dt_ticks) {
                WidgetOutcome::Hotbar(key) => match key.to_ascii_uppercase() {
                    b'C' => {
                        if ctx.char_files.entries.is_empty() {
                            self.status = Some("No characters saved.".to_string());
                            return ScreenTransition::Stay;
                        }
                        self.list = Some(Self::build_list(ctx.char_files));
                        self.source = None;
                        ScreenTransition::Stay
                    }
                    b'P' => {
                        self.status =
                            Some("Pool of Radiance import: not in this build.".to_string());
                        ScreenTransition::Stay
                    }
                    b'H' => {
                        self.status = Some("Hillsfar import: not in this build.".to_string());
                        ScreenTransition::Stay
                    }
                    _ => self.exit(),
                },
                _ => ScreenTransition::Stay,
            };
        }

        let Some(list) = &mut self.list else {
            return self.exit();
        };
        draw_picker(ctx, list);
        match list.tick(ctx.input, ctx.dt_ticks) {
            // `if ((input_key == 13 || input_key == 'A') && text[0] != '*')`
            // (`ovr018.cs:1474-1475`).
            WidgetOutcome::ListSelected { index, key } if key == b'A' || key == b'\r' => {
                let entry = ctx.char_files.entries.get(index - 1);
                match entry {
                    Some(e) if !e.taken => {
                        *ctx.char_io_request = Some(CharFileRequest::Load(e.stem.clone()));
                        // The host answers with a notice; the row is re-marked
                        // when it re-injects the directory.
                        ScreenTransition::Stay
                    }
                    _ => ScreenTransition::Stay,
                }
            }
            WidgetOutcome::ListCancelled => self.exit(),
            _ => ScreenTransition::Stay,
        }
    }
}

/// Wraps a screen for the shell's `Screen` enum.
pub fn create_screen(return_to: ReturnTo) -> Screen {
    Screen::CreateCharacter(Box::new(CreateCharacter::new(return_to)))
}
