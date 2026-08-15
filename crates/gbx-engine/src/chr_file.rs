//! ★ **Character files** (roll-credits slice 9c) — the `.guy`/`.swg`/`.fx`
//! trio `SavePlayer`/`import_char01` write and read, plus the party-join
//! legality `AddPlayer` applies.
//!
//! ★ **The extension is `.guy`, not `.CHR`.** `SavePlayer` (`ovr017.cs:134-209`)
//! writes `<clean_name>.guy` for the character record, `<clean_name>.swg` for
//! its items and `<clean_name>.fx` for its affects, all in the save directory
//! beside `savgam<X>.dat`. `.CHR` is another Gold Box title's convention;
//! CotAB never uses it. `BuildLoadablePlayersLists` (`ovr017.cs:63-81`) confirms
//! it from the other side: `"*.guy"` for a Curse character, `"*.cha"`/`"*.sav"`
//! for a Pool of Radiance import and `"*.hil"` for Hillsfar.
//!
//! The `.guy` payload is **exactly** the 0x1A6-byte `charStruct` — the same
//! record a `CHRDAT<X><n>.SAV` holds — so
//! [`gbx_formats::save_orig::decode_char_record`] and its new inverse are the
//! whole format. Verified against the three `.GUY` files the original left in
//! the bundle's own `SAVE/` directory.
//!
//! **D8 (no I/O in the tick core).** Nothing here touches the filesystem: the
//! screens render from a host-injected [`CharFileDirectory`] and emit a
//! [`CharFileRequest`] the host fulfills after the tick
//! ([`crate::saveload_fs::fulfill_char_file`]).

use crate::party::{Character, Party};

/// The character-file extensions (`ovr017.cs:129-131,146,183,195`).
pub const CHAR_EXT: &str = "guy";
pub const ITEMS_EXT: &str = "swg";
pub const AFFECTS_EXT: &str = "fx";

/// ★ `seg042.clean_string(player.name)` (`seg042.cs:68-78`, called at
/// `ovr017.cs:147`) — a character's name turned into a DOS filename stem.
///
/// Exactly three steps, and each one is worth naming because none of them is
/// what a modern reader would guess: **trim** the ten-character set
/// `[space . * , ? / \ : ; |]` from *both ends* (`seg042.cs:66`), **lowercase**
/// the rest, and truncate to **8** characters. Interior characters are left
/// alone — a space inside a name survives into the filename, and so does
/// anything else the player typed.
///
/// The one addition of ours: a name that trims away to nothing would give the
/// original a bare `.guy`, so it becomes `char` here instead. Named, not
/// silent.
pub fn clean_stem(name: &str) -> String {
    const TRIM: [char; 10] = [' ', '.', '*', ',', '?', '/', '\\', ':', ';', '|'];
    let stem: String = name
        .trim_matches(|c| TRIM.contains(&c))
        .to_lowercase()
        .chars()
        .take(8)
        .collect();
    if stem.is_empty() {
        "char".to_string()
    } else {
        stem
    }
}

/// One loadable character file, as the host found it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CharFileEntry {
    /// The filename stem (no extension) — what a [`CharFileRequest::Load`]
    /// names.
    pub stem: String,
    /// The name stored inside the record, which is what the picker lists
    /// (`BuildLoadablePlayersLists` reads it out of the file, `ovr017.cs:24-27`).
    pub name: String,
    /// Set once this entry has been added this session — the original marks a
    /// taken row with a leading `"* "` and refuses to take it twice
    /// (`ovr018.cs:1474-1485`).
    pub taken: bool,
}

/// Host-injected view of the save directory's character files — the `Add
/// Character to Party` picker renders from this, never from the filesystem.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CharFileDirectory {
    pub entries: Vec<CharFileEntry>,
}

impl CharFileDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    /// `BuildLoadablePlayersLists`' own filter (`ovr017.cs:47-53`): a
    /// character already in the party by name is not offered.
    pub fn without_party_members(mut self, party: &Party) -> Self {
        self.entries
            .retain(|e| !party.members.iter().any(|m| m.name.trim() == e.name.trim()));
        self
    }
}

/// What a character-file screen asks the host to do. Taken after the tick
/// with [`crate::engine::Engine::take_char_file_request`].
#[derive(Debug, Clone, PartialEq)]
pub enum CharFileRequest {
    /// `SavePlayer("", player)` (`ovr017.cs:134-209`) — write the record plus
    /// its `.swg`/`.fx` siblings under `clean_stem(name)`.
    Save(Box<Character>),
    /// `import_char01` (`ovr017.cs:486-534`) — read this stem's `.guy` (and
    /// its siblings) and offer it to the party.
    Load(String),
}

/// ★ `AddPlayer`'s party-join gate (`ovr018.cs:1497-1567`) — every reason the
/// original refuses a character, in its own order, with its own words.
///
/// The size rules are two different limits on two different counters: a
/// **PC** (`control_morale < NPC_Base`) may join while fewer than **6** PCs
/// are present, an **NPC** while `area2_ptr.party_size` is under **8**. The
/// three social rules are: a paladin will not travel with anyone evil, no
/// more than **three** rangers, and an evil character cannot join a party
/// that has a paladin in it.
pub fn join_refusal(party: &Party, candidate: &Character) -> Option<&'static str> {
    // `if (tmp_player.name == new_player.name && tmp_player.mod_id ==
    // new_player.mod_id) { found = true; break; }` — the duplicate check
    // stops the scan, so a duplicate is refused before anything else is even
    // counted.
    if party
        .members
        .iter()
        .any(|m| m.name == candidate.name && m.monster_index == candidate.monster_index)
    {
        return Some("already in the party");
    }
    if party.members.is_empty() {
        return None; // `if (gbl.TeamList.Count == 0)` joins unconditionally
    }

    let pc_count = party.members.iter().filter(|m| !m.is_npc()).count();
    let ranger_count = party
        .members
        .iter()
        .filter(|m| m.class_level[4] > 0)
        .count();
    let evil_present = party.members.iter().any(is_evil);
    let paladin = party.members.iter().find(|m| m.class_level[3] > 0);

    let room = if candidate.is_npc() {
        party.members.len() < 8
    } else {
        pc_count < 6
    };
    if !room {
        return Some("the party is full");
    }
    if candidate.class_level[3] > 0 && evil_present {
        return Some("paladins do not join with evil scum");
    }
    if candidate.class_level[4] > 0 && ranger_count > 2 {
        return Some("too many rangers in party");
    }
    if is_evil(candidate) && paladin.is_some() {
        return Some("will tolerate no evil!");
    }
    None
}

/// `(alignment + 1) % 3 == 0` (`ovr018.cs:1522`) — the evil column of the
/// 3×3 alignment grid (ids 2, 5, 8: Lawful/Neutral/Chaotic Evil).
fn is_evil(ch: &Character) -> bool {
    (ch.alignment as usize + 1).is_multiple_of(3)
}

/// `AssignPlayerIconId` (`ovr017.cs:892`) — the joining character takes the
/// lowest free party icon slot, which is also their display order.
pub fn assign_icon_id(party: &Party, candidate: &mut Character) {
    let used: Vec<u8> = party.members.iter().map(|m| m.icon.icon_id).collect();
    candidate.icon.icon_id = (0..crate::combat_art::PARTY_ICON_SLOTS as u8)
        .find(|id| !used.contains(id))
        .unwrap_or(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, class: usize, alignment: u8) -> Character {
        let bytes = [0u8; gbx_formats::save_orig::CHAR_RECORD_SIZE];
        let rec = gbx_formats::save_orig::decode_char_record(&bytes).unwrap();
        let mut ch = crate::party::character_from_record(&rec, vec![], vec![]);
        ch.name = name.to_string();
        ch.class_level[class] = 1;
        ch.alignment = alignment;
        ch
    }

    /// `clean_string` trims only the ends, lowercases, and cuts at 8 — it
    /// does NOT strip interior characters, which is the easy thing to assume
    /// and the wrong thing to implement.
    #[test]
    fn clean_stem_trims_the_ends_lowercases_and_cuts_at_eight() {
        assert_eq!(clean_stem("Sir Robin"), "sir robi", "the space survives");
        assert_eq!(clean_stem("  Alias.  "), "alias", "both ends trimmed");
        assert_eq!(clean_stem("a-very-long-name"), "a-very-l");
        assert_eq!(clean_stem("JOE"), "joe");
        // Ours, not the original's: a name that trims to nothing would give
        // the original a bare `.guy`.
        assert_eq!(clean_stem("..."), "char");
    }

    #[test]
    fn an_empty_party_takes_anyone() {
        let party = Party::default();
        assert_eq!(join_refusal(&party, &member("A", 3, 0)), None);
    }

    #[test]
    fn a_paladin_will_not_join_an_evil_party() {
        let mut party = Party::default();
        party.members.push(member("Villain", 2, 2)); // Lawful Evil
        assert_eq!(
            join_refusal(&party, &member("Saint", 3, 0)),
            Some("paladins do not join with evil scum")
        );
    }

    #[test]
    fn an_evil_character_will_not_join_a_paladins_party() {
        let mut party = Party::default();
        party.members.push(member("Saint", 3, 0));
        assert_eq!(
            join_refusal(&party, &member("Villain", 2, 8)),
            Some("will tolerate no evil!")
        );
        // ...but a good one is welcome.
        assert_eq!(join_refusal(&party, &member("Friend", 2, 0)), None);
    }

    #[test]
    fn a_fourth_ranger_is_refused() {
        let mut party = Party::default();
        for i in 0..3 {
            party.members.push(member(&format!("R{i}"), 4, 4));
        }
        assert_eq!(
            join_refusal(&party, &member("R4", 4, 4)),
            Some("too many rangers in party")
        );
        // A non-ranger still fits.
        assert_eq!(join_refusal(&party, &member("F", 2, 4)), None);
    }

    #[test]
    fn a_seventh_pc_is_refused_but_an_npc_still_fits() {
        let mut party = Party::default();
        for i in 0..6 {
            party.members.push(member(&format!("P{i}"), 2, 4));
        }
        assert_eq!(
            join_refusal(&party, &member("P7", 2, 4)),
            Some("the party is full")
        );
        let mut npc = member("Helper", 2, 4);
        npc.control_morale = 0x80;
        assert_eq!(join_refusal(&party, &npc), None);
    }

    #[test]
    fn the_same_character_cannot_join_twice() {
        let mut party = Party::default();
        let a = member("Twin", 2, 4);
        party.members.push(a.clone());
        assert_eq!(join_refusal(&party, &a), Some("already in the party"));
    }

    #[test]
    fn assign_icon_id_takes_the_lowest_free_slot() {
        let mut party = Party::default();
        let mut first = member("A", 2, 4);
        first.icon.icon_id = 0;
        let mut second = member("B", 2, 4);
        second.icon.icon_id = 2;
        party.members.push(first);
        party.members.push(second);
        let mut c = member("C", 2, 4);
        assign_icon_id(&party, &mut c);
        assert_eq!(c.icon.icon_id, 1);
    }

    #[test]
    fn the_directory_hides_characters_already_in_the_party() {
        let mut party = Party::default();
        party.members.push(member("JOE", 6, 4));
        let dir = CharFileDirectory {
            entries: vec![
                CharFileEntry {
                    stem: "JOE".into(),
                    name: "JOE".into(),
                    taken: false,
                },
                CharFileEntry {
                    stem: "PHIL".into(),
                    name: "PHIL".into(),
                    taken: false,
                },
            ],
        }
        .without_party_members(&party);
        assert_eq!(dir.entries.len(), 1);
        assert_eq!(dir.entries[0].name, "PHIL");
    }
}
