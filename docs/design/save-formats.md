# Design: Save Formats (our envelope + original-save import)

> M3 architecture pass per PLAN.md §9 operating rule 3 (one design review before
> each one-way door). Status: **v2, draft for review.** v1 was written 2026-07-14
> from a read of coab's save/load internals (read-for-behavior per D11 — no code
> copied; see SOURCES.md), the GBC-project's independent CotAB character-record
> documentation, and the FRUA-era hackdocs save note (cross-checks only,
> contradictions flagged in §1.7, never absorbed), then subjected the same day to
> one bounded adversarial review round (two independent reviewers, fresh context:
> one attacking import fidelity against coab/GBC bytes, one attacking the envelope
> as systems design). Every folded finding was re-verified against coab directly
> before editing. v2's changes — no architectural reversals, but real corrections:
> **(import)** the saved ECL block (§1.1 section 5) is *discarded* on load, not
> restored — the original reloads the pristine block by `LastEclBlockId` from data
> (`ovr003.cs:2268-2272`), so import ignores section 5 (§1.5, D-SAVE5); the
> `field_200`/`field_6F2` flag-clear was described backwards — it is *suppressed*
> on save-load by the `reload_ecl_and_pictures==false` guard (`ovr008.cs:128-132`,
> §1.4); a **third** pinned record cell surfaced — coab's `StatValue.Read` wrongly
> clamps exceptional-strength Str00 to 25 (§1.7 item 5); `age` and two named
> fields added to the D-SAVE11 completeness enumeration. **(envelope)** determinism
> is now a CI-enforced collection-ordering invariant (no `HashMap` in `SaveState`),
> not an assumption (D-SAVE1); the pacing accumulator + animation phase are
> *included* (they change pixels — excluding them broke the identical-frame-hash
> claim, D-SAVE3/D-SAVE4); the container header is explicit little-endian, not
> `repr(C)` (D-SAVE1/§3); `seed`/`tick_index` demoted to provenance (PRNG state
> drives resume, D-SAVE4); the three version scalars reconciled to one authority
> (D-SAVE2); the H5 hash is pinned to the payload only (D-SAVE4); a cross-platform
> golden-hash CI gate added (D-SAVE10). Review stops here per the bounded-round
> doctrine; remaining risk is carried by the M3 fixtures + the human/oracle tiers.
> This doc then goes to Fable + human review before any implementation session
> (PLAN §9 rule 3; M3 milestone).
>
> Scope: two formats. **(a) Our save envelope** — the versioned container around
> full engine state (D8's tick core makes save-anywhere fall out; D9's determinism
> makes it a replay checkpoint). **(b) Original-save import** — reading a real
> CotAB `savgam?.dat` set into engine state, M3's exit gate. Out of scope: the
> party/character *model* shape (M3 implementation, this doc only constrains its
> field completeness — §2 D-SAVE11); mid-combat saves (M4); combat/spell runtime
> state serialization beyond what D-VM3/D-UI2 already commit; cloud/slot UX.

## 1. Evidence

### 1.1 What constitutes an original save (coab, verified)

Everything below was read directly from coab (CotAB dialect). File references are
to `~/src/goldbox-refs/coab/`. A CotAB save is **a set of files**, not one file —
a master container plus one record file per party member (plus optional side
files):

**The master container `savgam<X>.dat`** (`X` = `A`..`J`, the ten save slots;
`ovr017.cs:1129, 937`). Written flat (uncompressed) by `SaveGame`
(`ovr017.cs:1109-1205`) as a fixed sequence of `seg051.BlockWrite` calls; read
back by `loadSaveGame` (`ovr017.cs:976-1103`) with matching fixed-size
`BlockRead`s. The write order **is** the format:

| # | Section | Size (bytes) | coab source | Contents |
|---|---------|--------------|-------------|----------|
| 1 | `game_area` | 1 | `:1150-1151` | current game area/module id (2 for Tilverton content) |
| 2 | `area_ptr` (`Area1`) | 0x800 (2048) | `:1153` | the **Area ScriptMemory window** backing — time, position, quest flags, sky, speed… (§1.4) |
| 3 | `area2_ptr` (`Area2`) | 0x800 (2048) | `:1154` | the **Party ScriptMemory window** backing — search flags, party size, encounter, temple/shop… (§1.4) |
| 4 | `stru_1B2CA` (`Struct_1B2CA`) | 0x400 (1024) | `:1155` | the **Table ScriptMemory window** backing (opaque word store, `Struct_1B2CA.cs:10`) |
| 5 | `ecl_ptr` (`EclBlock`) | 0x1E00 (7680) | `:1156` | resident ECL block bytes — **written on save but discarded on load** (§1.5): `loadSaveGame` reloads the pristine block from `ECL{area}.dax` by `LastEclBlockId`, so import ignores these bytes (`EclBlock.cs`) |
| 6 | position block | 5 | `:1158-1163` | `mapPosX, mapPosY, mapDirection, mapWallType, mapWallRoof` |
| 7 | `last_game_state` | 1 | `:1165-1166` | prior `GameState` enum |
| 8 | `game_state` | 1 | `:1167-1168` | current `GameState` enum |
| 9 | `setBlocks[0..2]` | 12 (0x0C) | `:1170-1175` | three `{blockId:i16, setId:i16}` pairs — the loaded wallset descriptors (§1.5) |
| 10 | `party_count` | 1 | `:1184-1185` | number of party members |
| 11 | character-file names | 0x148 (328) | `:1187-1191` | up to 8 × 0x29 (41)-byte names, `"CHRDAT<X><n>"` |

Total = `1 + 2048 + 2048 + 1024 + 7680 + 5 + 1 + 1 + 12 + 1 + 328` = **13149
bytes**, fixed. (`SaveGame` allocates a `0x1E00` scratch buffer and reuses it;
sizes above are the `BlockWrite` lengths, not the buffer.) Note sections 2–4 are
exactly the three ScriptMemory window backings the VM already models (D-VM5);
section 5 is the resident block bytes, but on load the original reloads a pristine
block from data (§1.5) and the saved bytes are dead — see §1.4/§1.5.

**The per-character record `CHRDAT<X><n>.sav`** (`n` = 1..party_count). Each is
one `Player.StructSize` = **0x1A6 (422)-byte** record, written by
`SavePlayer("CHRDAT"+X+n, player)` (`ovr017.cs:1198`, which routes to
`ovr017.cs:134-209`; a non-empty name arg selects the `.sav` extension,
`:151-152`). Layout in §1.3.

**Optional side files per character** (`SavePlayer`, `:183-208`):
- `CHRDAT<X><n>.swg` — the character's carried **items**, written only when
  `items.Count > 0`, each `Item.StructSize` bytes (`:185-193`).
- `CHRDAT<X><n>.fx` — active **affects/effects**, written only when
  `affects.Count > 0`, each `Affect.StructSize` bytes (`:197-208`).

On load, `loadSaveGame` reads sections 1–11, then for each of `number_of_players`
names it calls `import_char01(name + ".sav")` (`:1037-1048`), which reads the
0x1A6 record and then the `.swg`/`.fx` side files if present
(`ovr017.cs:486-613`). So **items and affects are NOT in the 422-byte record** —
they are separate files; the record's item/affect/equipment fields are runtime
pointers (§1.3, §1.7 item 3).

**Roster / transfer files.** Individually-saved characters (party roster kept
between games, and characters exported for transfer) use the same 0x1A6 record
under a `.guy` extension (`SavePlayer` with empty name arg, `:146-147`), with
`.swg`/`.fx` siblings. `.cha` (Pool of Radiance) and `.hil` (Hillsfar) are
*foreign* import formats converted on load (§1.6). The master-container save
deletes the loose `.guy`/`.swg`/`.fx` after packing them into `CHRDAT` files
(`:1199` → `remove_player_file`, `:125-132`).

### 1.2 When a save can be taken (the mid-combat question)

Saving is reachable from exactly two sites, both VM-quiescent:
- **the in-game menu** — `PROGRAM` opcode case 0 (the full game menu:
  save/load/training/party, then the script *continues*), per vm-scriptmemory.md
  §1 ("saving enters via PROGRAM(0)'s menu and the camp menu; **no game-save
  opcode exists**", D-VM3 evidence);
- **the camp menu** — reached through `PROGRAM` case 9 / `TryEncamp`
  (`ovr003.cs:1913-1926`); a save taken here sits *above a parked PROGRAM-9
  frame* (vm-scriptmemory.md §1).

Combat (`ovr003.cs` combat path) **never re-enters `RunEclVm`** and exposes no
save site (vm-scriptmemory.md §1: "combat/rest/training never re-enter the VM").
Therefore **no original save is ever taken mid-combat** — the exit-gate "import a
real mid-game save" always lands in an exploration/camp state with the VM idle
(between vector runs) or parked in the camp flow. Crucially, `savgam?.dat` stores
**no VM activation registers** — not the string registers, call stack, compare
flags, or a pending Request. It writes the resident block bytes (§1.1 section 5)
but **discards them on load**: `loadSaveGame` sets `reload_ecl_and_pictures = true`
(`:983`), so the walk-loop entry reloads a pristine block from `ECL{area}.dax` by
`area_ptr.LastEclBlockId` (`ovr003.cs:2260, 2268-2272`; `load_ecl_dax` `Clear()`s
then `SetData`s the freshly-decoded block, `ovr008.cs:141-151`) before any script
runs, then re-enters cleanly (`vm_init_ecl` re-parses vectors and fires the entry
vector — renderer-ui-shell.md §1.6). The original's save is thus **coarser than
ours will be**: it snapshots persistent state and re-derives both VM control flow
*and the resident block*, where our save-anywhere (D8/D9) must snapshot the live
(possibly self-modified) EclMachine too (§2 D-SAVE3). *(Confirm against DOSBox that the game menu is unreachable during a
combat round — docket §5.1.)*

### 1.3 The character record (0x1A6 bytes), unified

The 422-byte record, from coab's `Player.cs` `[DataOffset]` attributes
(authoritative for our reimplementation — coab transliterates the binary's field
I/O; one caveat: the `CustSaveLoad`/`IDataIO` field `stats2` declares a *nominal*
attribute length 0x12 that `DataIO` ignores — `PlayerStats.Write` writes exactly
14 bytes at +0x00..+0x0d, `Player.cs:107-116`, the true extent in the table
below) cross-checked against the GBC project's independent CotAB character-format
doc (`~/src/goldbox-refs/tools/formats/Character file formats/02. Curse of the
Azure Bonds.txt`, "GBC-doc" below). **They agree on every offset**; two
value-level disagreements are flagged in §1.7 (stat byte order; the
`spellCastCount` stride). `DataIO.ReadObject`/`WriteObject` reads/writes only
attributed fields, so the pointer fields (below) are disk garbage on read.

| Offset | Len | coab field (`Player.cs`) | GBC-doc label | Notes |
|--------|-----|--------------------------|---------------|-------|
| 0x00 | 15 | `name` (PString) | name length @0, name @1–15 | length-prefixed, 15 max |
| 0x10 | 14 | `stats2` (`PlayerStats`, 7 × StatValue) | str/int/wis/dex/con/cha/str-exc, orig+cur each | order STR,INT,WIS,DEX,CON,CHA,STR% (`Player.cs:107-116`); **byte order flagged §1.7** |
| 0x1e | 84 | `spellList` (`SpellList.Load @+0x1e`) | memorized spells 0x1e–0x71 | per-slot memorized-spell list |
| 0x72 | 1 | `spell_to_learn_count` | ??? | |
| 0x73 | 1 | `thac0` (sbyte, base) | thac0 base | |
| 0x74 | 1 | `race` | race | 0=monster,1=dwarf…7=human (GBC-doc) |
| 0x75 | 1 | `_class` (ClassId) | class | 0..0x10, multiclass combos (GBC-doc) |
| 0x76 | 2 | `age` (i16) | age | |
| 0x78 | 1 | `hit_point_max` | hp max | |
| 0x79 | 100 | `spellBook[100]` | per-spell known flags (0x79–0xDC) | `KnowsSpell`/`LearnSpell` index `spell-1` |
| 0xdd | 1 | `attackLevel` | attack level | |
| 0xde | 1 | `field_DE` | icon dimensions | |
| 0xdf | 5 | `saveVerse[5]` | save 1–5 | paralyze/petrify/rod/breath/spell |
| 0xe4 | 1 | `base_movement` | movement base | |
| 0xe5 | 1 | `HitDice` | level highest 1 | |
| 0xe6 | 1 | `multiclassLevel` | level highest 2 | |
| 0xe7 | 1 | `lost_lvls` | drained levels | |
| 0xe8 | 1 | `lost_hp` | drained hps | |
| 0xe9 | 1 | `field_E9` | level undead | turn-undead type index |
| 0xea | 8 | `thief_skills[8]` | thief 1–8 | pick/locks/traps/silent/hide/hear/climb/read |
| 0xf2 | 4 | *(affects list ptr)* | effects address | **runtime pointer** — affects in `.fx` file |
| 0xf6 | 1 | `field_F6` | ??? | |
| 0xf7 | 1 | `control_morale` | npc | ≥0x80 = NPC (`Control.cs:322`) |
| 0xf8 | 1 | `npcTreasureShareCount` | modified | |
| 0xf9 | 2 | `field_F9`,`field_FA` | ??? ×2 | |
| 0xfb | 14 | `Money` (7 × i16) | copper/silver/electrum/gold/plat/gems/jewelry | `MoneySet`, 0xfb–0x108 |
| 0x109 | 8 | `ClassLevel[8]` | level cleric…monk | per-class current levels |
| 0x111 | 8 | `ClassLevelsOld[8]` | former level cleric…monk | dual-class prior levels |
| 0x119 | 1 | `sex` | gender | |
| 0x11a | 1 | `monsterType` | type | |
| 0x11b | 1 | `alignment` | alignment | 0..8 (GBC-doc) |
| 0x11c | 8 | `attacksCount`,`baseHalfMoves`,attack1/2 dice-base ×6 | attacks…unarmed modifier 2 | 0x11c–0x123 |
| 0x124 | 1 | `base_ac` | ac base | |
| 0x125 | 1 | `field_125` | ??? | |
| 0x126 | 1 | `mod_id` | monster index | |
| 0x127 | 4 | `exp` (i32) | experience | |
| 0x12b | 1 | `classFlags` | item limits | |
| 0x12c | 1 | `hit_point_rolled` | hp rolled | |
| 0x12d | 15 | `spellCastCount[3,5]` | cleric/druid/mage spells 1–5 | **stride flagged §1.7** (coab `i*i` bug) |
| 0x13c | 2 | `field_13C` (i16) | xp award | |
| 0x13e | 3 | `field_13E/13F/140` | xp bonus/hp, ??? ×2 | |
| 0x141 | 1 | `head_icon` | icon head | |
| 0x142 | 1 | `weapon_icon` | icon body | |
| 0x143 | 1 | `icon_id` | order number | party display order |
| 0x144 | 1 | `icon_size` | icon size | 1=small,2=normal |
| 0x145 | 6 | `icon_colours[6]` | icon colors (nibble-packed pairs) | `LoadPlayerCombatIcon` `&0x0F`/`>>4` (`:112-113`) |
| 0x14b | 1 | `field_14B` | flags 1 | |
| 0x14c | 1 | *(item count)* | number of items | commented out in coab; items in `.swg` |
| 0x14d | 4 | *(items list ptr)* | items address | **runtime pointer** |
| 0x151 | 52 | *(activeItems: 13 ptrs)* | equipped weapon…bolt addresses | **runtime pointers** — reconstructed from item readied flags (§1.7 item 3) |
| 0x185 | 1 | `weaponsHandsUsed` | hands equipped | |
| 0x186 | 1 | `field_186` (sbyte) | save bonus | |
| 0x187 | 2 | `weight` (i16) | encumbrance | |
| 0x189 | 4 | *(next-char ptr)* | next character address | **runtime pointer** |
| 0x18d | 4 | *(actions ptr)* | combat address | **runtime pointer** |
| 0x191 | 1 | `paladinCuresLeft` | ??? | |
| 0x192 | 3 | `field_192/193/194` | ??? ×3 | |
| 0x195 | 1 | `health_status` (Status) | status | 0=okay…8=gone (GBC-doc) |
| 0x196 | 1 | `in_combat` (bool) | enabled | |
| 0x197 | 1 | `combat_team` | hostile | 0=ours,1=enemy |
| 0x198 | 1 | `quick_fight` | quickfight | |
| 0x199 | 1 | `hitBonus` | thac0 current | |
| 0x19a | 1 | `ac` | ac current | display AC = `0x3C - ac` (`Player.cs:598`) |
| 0x19b | 1 | `ac_behind` | ac behind | |
| 0x19c | 8 | attack1/2 left/dice-count/dice-size/dmg-bonus | current attacks…modifier 2 | 0x19c–0x1a3 |
| 0x1a4 | 1 | `hit_point_current` | hp current | |
| 0x1a5 | 1 | `movement` | movement current | initiative |

`StructSize = 0x1A6` (`Player.cs:708`). The GBC-doc's last field is 0x1A5, same
extent. Enums (race/class/alignment/status values) are transcribed in GBC-doc
§tables and coab's `Enums.cs` — these feed the party model + rules packs (D-RP5),
not this doc's format concern.

### 1.4 The area/flag blobs → our ScriptMemory windows

Sections 2–4 of the container are exactly the backing stores of three of the
D-VM5 ScriptMemory windows — the save persists them verbatim and the VM addresses
into them at runtime:

- **`area_ptr` (`Area1`, 0x800)** ⟷ the **Area window** (`0x4B00–0x4EFF`, word).
  `Area1.field_6A00_Get/Set` (`Area1.cs:203-649`) is the exact address→field
  dispatch; `ToByteArray` dumps the whole 0x800 `origData` backing
  (`Area1.cs:651-656`), and unmapped addresses fall through to
  `DataIO.Get/SetObjectUShort(origData, loc)` — i.e. **named cells over a raw
  store**, precisely D-VM5's model. Named cells that matter to import:
  - **game clock**: `time_minutes_ones/tens` (0x18E/0x190), `time_hour` (0x192),
    `time_day` (0x194), `time_year` (0x196) — matches the ECL-clock window
    (vm-scriptmemory.md §1, `0x4BC6..`).
  - **`inDungeon`** (0x1CC) — write side-effects `game_state` in the original
    (vm-scriptmemory.md §1 D-VM5 "writing inDungeon flips game_state").
  - **position**: `lastXPos`/`lastYPos` (0x1E0/0x1E2), `current_3DMap_block_id`
    (0x18A), `current_city` (0x342).
  - **`LastEclBlockId`** (0x1E4) — which script block is resident (renderer §1.6
    bookkeeping).
  - **`block_area_view`** (0x1F6), sky colours (0x1FA/0x1FC), `game_speed`
    (0x1F8 — the save-file speed setting, renderer §1.4), `pics_on` (0x1FE),
    `can_cast_spells` (0x1FF).
  - **`field_200[33]`** (0x200–0x240) — the per-area **script/quest flag words**;
    this is the mechanism by which plot state survives a save. `vm_init_ecl`
    clears them on a *fresh* block-entry but **suppresses the clear on save-load**
    — the guard is `if (reload_ecl_and_pictures == false) RestField200Values()`
    (`ovr008.cs:128-132`; the method body `Area1.cs:658-664`), and `loadSaveGame`
    sets the flag true (`:983`), so the flags survive a load. A restore must
    likewise **not** clear them; the save carries them. (Described backwards in
    v1 — the clear is on-reload-*skip*, not on-reload.)
  - a large sparse set of further script-writable words (0x244–0x596) — carried
    as named-or-raw, no interpretation needed for import.
- **`area2_ptr` (`Area2`, 0x800)** ⟷ the **Party window** (`0x7C00–0x7FFF`,
  word). `Area2.field_800_Get/Set` (`Area2.cs:158-318`). Named cells:
  `search_flags` (0x594, bit 1 searching / bit 2 looking), `party_size` (0x67C),
  `game_area` (0x624), `training_class_mask` (0x550), encounter distances
  (0x580/0x582), `HeadBlockId` (0x5C2), `EnterTemple` (0x5C4)/`EnterShop`
  (0x6D8), rest-encounter params (0x5A4/0x5A6), `tried_to_exit_map` (0x5AA),
  `field_6F2..704` (rest state, cleared on fresh block-entry via
  `RestField6F2Values` under the same `reload_ecl_and_pictures == false` guard,
  `Area2.cs:320-332` — preserved on save-load), and a run of individually-named
  byte cells `field_799..7AB` (`Area2.cs:110-147`). Note the Party window *also*
  read/write-throughs to the
  selected character's fields at runtime (`get_player_values`/`alter_character`,
  vm-scriptmemory.md §1) — but the *stored* 0x800 blob is only the `area2_ptr`
  words; character data lives in the `CHRDAT` records.
- **`stru_1B2CA` (0x400)** ⟷ the **Table window** (`0x7A00–0x7BFF`, word). coab
  models it as an opaque `byte[0x400]` (`Struct_1B2CA.cs:10`) with no named
  fields — our import carries it as a raw word store (D-VM5 raw fallback), no
  interpretation.

The container's ECL block (section 5, 0x1E00) is the resident block the
EclMachine owns (D-VM3). Position (section 6) duplicates `lastXPos/Y` semantics
at the engine level (`mapPosX/Y` globals, restored to `gbl` at `:1003-1007`).

### 1.5 Wallset descriptors and asset reload (`setBlocks`)

Section 9 (`setBlocks[0..2]`, three `{blockId, setId}` i16 pairs) is **named
engine state that drives asset reload on restore, not pixels**. On load, when
`inDungeon != 0` and not the start menu, `loadSaveGame` re-runs
`ovr031.Load3DMap(current_3DMap_block_id)` then `LoadWalldef(setId, blockId)` for
each non-zero `setBlocks[i]` (`ovr017.cs:1074-1091`). This is the "resident-asset
IDs not bytes" pattern (renderer-ui-shell.md §1.3: "`setBlocks[0..2]` … persisted
by the original's saves to re-run `LoadWalldef` on restore,
`ovr017.cs:1078-1087`"). Outside a dungeon it loads a bigpic instead
(`load_bigpic(0x79)`, `:1094`). Both our envelope and our import store these
**ids** and reload bytes from `GameData` (D-UI1: core does zero I/O; assets are
in-memory lookups).

**The resident ECL block reloads the same way.** `loadSaveGame` reads section 5
into `ecl_ptr` (`:999-1000`), but the walk-loop entry immediately overwrites it:
with `reload_ecl_and_pictures` set (always true after a load, `:983`) it runs
`load_ecl_dax(area_ptr.LastEclBlockId)`, which `Clear()`s `ecl_ptr` and `SetData`s
the freshly-decoded block from `ECL{game_area}.dax` (`ovr003.cs:2260, 2268-2272`;
`ovr008.cs:141-151`). So the block, like the wallsets, is a **reload-by-id** —
import ignores section 5 and reloads by `LastEclBlockId`. (Our own `.rsav`
save-anywhere is different: it *does* store the live block, because D-VM3's
EclMachine may be parked mid-instruction on a self-modified block — §2 D-SAVE3.)

### 1.6 Foreign character import (Pool / Hillsfar)

CotAB imports characters from two predecessors, via explicit field-by-field
conversion maps (leads for our M6 transfer path, §2 D-SAVE9):
- **Pool of Radiance `.cha`** — `PoolRadPlayer.StructSize` record →
  `ConvertPoolRadPlayer` (`ovr017.cs:234-381`): maps `bp_var_1C0.field_XX` →
  `player.*`, enforces race/sex stat limits (`EnforceRaceSexLimits`), copies the
  0x38-byte spellbook prefix, zeroes `animate_dead`, grants 300 platinum
  (`:296`), copies class levels, icon data, combat fields. `.spc` side file
  carries a filtered affect set (`asc_49280` membership, `:593`).
- **Hillsfar `.hil`** — `HillsFarPlayer.StructSize` → `ConvertHillsFarPlayer`
  (`ovr017.cs:616-816`) + `TransferHillsFarCharacter` (`:384-459`):
  take-the-higher stat merge onto an existing `.guy`/`.cha` character, or a
  from-scratch build (`HillsFarClassMap`, `:479-483`; race/affect grants
  `:748-783`; `SilentTrainPlayer` `:461-473`). Hillsfar has no `npc` byte
  (`:31-34`).

These are import-*into*-CotAB conversions; they establish that record-to-record
field mapping with limit-enforcement is coab's own idiom. Our restrike equivalent
(importing a Pool/Hillsfar character *into restrike's CotAB flavor*) reuses these
maps — deferred to M6 (§2 D-SAVE9).

### 1.7 Cross-check contradictions and oddities (flagged, not absorbed)

1. **Stat byte order — coab vs GBC-doc disagree.** coab's `StatValue.Write`
   stores `data[+0]=cur` (current), `data[+1]=full` (original/max)
   (`Player.cs:77-88`). GBC-doc labels 0x10 "str original", 0x11 "str current"
   — i.e. **the opposite order**. One is wrong. coab is a transliteration of the
   binary's own I/O so it is the stronger authority, but this is exactly a
   flag-don't-absorb case (D11). Resolution: import a real save of a
   *stat-drained* character (e.g. STR reduced below max by a monster or
   ability-damage), read both ways, and match the pair to what DOSBox / GBC shows
   as "current (max)". Docket §5.2; the import implementation must pin this
   against real data before the M3 gate, not guess.
2. **`spellCastCount` stride — coab has an arithmetic bug.** coab reads/writes
   the 3×5 memorized-count grid with `data[0x12d + j + (i*i)]`
   (`Player.cs:727, 769`) — `i*i` gives row offsets 0,1,4, which *overlap*
   (i=1→0x12e..0x132 collides with i=0's cleric row). GBC-doc's independent
   layout is clean and contiguous: cleric 0x12d–0x131, druid 0x132–0x136, mage
   0x137–0x13b — i.e. stride **5** (`i*5`), the natural 3×5. Our import uses
   **`i*5`** (GBC-doc's self-consistent layout), a documented, deliberate
   divergence from coab's transliterated bug. Docket §5.2 (confirm against a real
   save with known memorized counts). Cite both.
3. **Equipment pointers are not data.** Record offsets 0xf2, 0x14d, 0x151–0x184,
   0x189, 0x18d are 4-byte far pointers (GBC-doc "…address"). They are
   meaningless across a save — coab does not attribute them for `DataIO` and
   reconstructs equipment from the loaded items' `readied` flags
   (`ActiveItems.UndreadyAll`, `Player.cs:303-314`; `reclac_player_values` +
   `ReclacClassBonuses` after item load, `ovr017.cs:611-612`). Our import loads
   items from `.swg`, then reconstructs the ready/equipped set — it must **not**
   trust the on-disk pointer bytes.
4. **SAVGAM.TXT is a different generation.** The FRUA-era hackdoc
   (`~/src/goldbox-refs/tools/hackdocs/SAVGAM.TXT`) describes a *single-file*
   save with a 3900-bit event-flag region and CCH records appended — **not**
   CotAB's multi-file `savgam?.dat` + `CHRDAT` structure. It corroborates the
   *family shape* (a game-speed byte, module/area id, party dungeon column/row,
   quest/key/item flag arrays, per-step event counts, party size, appended
   character records — cf. `field_200[]` and the position block) but **its
   offsets do not apply to CotAB**. Same status as renderer-ui-shell.md §1.11
   item 5's DRAW18/GEODATA: useful only for the TLB/FRUA era (M9). Not absorbed.
5. **Exceptional-strength (Str00) is clamped to 25 by coab — a third pinned
   cell.** `StatValue.Read` does `full = Math.Min(data[+1], 25)`
   (`Player.cs:86-87`), applied uniformly to all seven stats including Str00 via
   `PlayerStats.Read` (`Player.cs:126`). But Str00 (the exceptional-strength
   percentile at 0x1c/0x1d) legitimately ranges **0–100** — it is
   `Load(Random(100)+1)` in play (`ovr018.cs:703`) and the `18/xx` display value
   (`seg043.cs:132`). coab's own read corrupts 18/91 → "18/25". Our import must
   **not** clamp Str00 to 25 (use `0..=100`); the six main ability scores keep
   the `0..=25` bound. Docket §5.2 — a third cell to pin against real data,
   alongside items 1–2.
6. **The three flagged record cells (items 1, 2, 5) are the only import-fidelity
   hazards found.** coab's *save-file section* I/O (§1.1) is a plain fixed-size
   block sequence with no comparable arithmetic, so the container layout is
   low-risk to transcribe; and every §1.3 byte offset verified clean against both
   coab and the GBC doc — the hazards are all value-level, in three cells.

## 2. Decisions

### D-SAVE1 — Our envelope: one file, a versioned binary container

A restrike save is **a single file** (`<name>.rsav`), not a file-set — the
original's multi-file split was an MS-DOS memory-management artifact, not a
fidelity requirement. Inside: a small **container header** (magic `b"RSAV"`,
versions, fingerprint, provenance — §3) wrapping one `postcard`-serialized
`SaveState` payload.

**Serialization: `postcard` (serde) for the payload.** Rationale against the
D1/D8/D9 constraints:
- **Deterministic — but only under a mandated collection discipline.** postcard
  has a canonical, non-self-describing wire form, so a fixed `SaveState` value
  serializes to identical bytes every time — which makes a save hashable as an H5
  checkpoint (D-SAVE4). That is **not automatic**: postcard serializes maps/sets
  in iteration order, and `HashMap`/`HashSet` (std `RandomState`) randomize it per
  process → machine-dependent bytes, silently breaking D9/H5 hash-equality.
  **Invariant (CI-enforced, D-SAVE10): every collection in `SaveState` and its
  nested snapshots serializes deterministically** — `BTreeMap`/`BTreeSet`, a
  sorted `Vec<(k,v)>`, or an insertion-ordered map — and `#[serde(flatten)]` is
  forbidden (postcard cannot buffer it canonically). The ScriptMemory raw-cell
  store (D-VM5) is the prime case: a sorted/ordered map, never a `HashMap`. The
  one float in the engine (the pacing accumulator, renderer §D-UI1) is stored as
  **fixed-point, not `f32`** (D-SAVE3), so the payload carries no floats and no
  format-nondeterminism.
- **wasm-clean** — `#![no_std]`-friendly, no I/O or platform deps; the core
  serializes to a `Vec<u8>` the frontend persists (D-UI1: core does zero I/O).
- **Compact** — CotAB state is a few tens of KB (four window blobs ≈ 5 KB, the
  resident block 7.5 KB, N × ~0.5 KB characters); postcard adds negligible
  framing.
- **serde-derived** — D-UI2/§3 already require "every Shell/Widget/flow state is
  serde-able by construction"; postcard reuses those derives with zero new
  per-type work.

*Alternatives considered:* **JSON/TOML** — human-readable but bulky, and float/map
nondeterminism risks break hash-equality (rejected for the payload; the header
stays trivially inspectable). **bincode** — equivalent in kind, but its wire
format has historically shifted across major versions and it is less explicitly
"canonical"; postcard's stability contract is stronger for a reject-not-migrate
format. **CBOR/MessagePack** — self-describing (wasteful here: we control both
ends via versioning) and larger. **A hand-rolled format** — rejected: the D-VM3
snapshot + party model are non-trivial nested structures; serde derive is the
low-defect path. The container header is hand-written (fixed, tiny) so the file
is greppable/diagnosable without deserializing the payload.

### D-SAVE2 — Version tag + reject-not-migrate (pre-1.0 posture)

Restating D-VM3's snapshot commitment at the envelope level: `save_format_version`
is a single monotonically-increasing `u32`. On load, an unrecognized version is
**rejected with a clear diagnostic, never migrated** (pre-1.0; revisit if saves
need longevity — same trigger D-VM3 named).

**Three version scalars, one authority.** `container_version` (header layout,
§3) is checked **first** — the header must parse before the payload; an
unrecognized `container_version` is rejected with a header-level diagnostic.
`save_format_version` is then **the single authority for the payload**: it
subsumes the nested snapshots' own version tags (EclMachine's D-VM3 tag, and any
party/shell tag), so any serialization-incompatible change to *any* of them bumps
`save_format_version`. The nested tags are retained (they serve gbx-vm's non-save
conformance uses, D-VM3) and re-checked belt-and-suspenders at restore, but they
never *substitute* for the envelope gate — reject-not-migrate checks the envelope
version first, the inner tags second. On any reject, load fails cleanly with an
actionable message (which version the save is, which the binary expects); boot is
unaffected — a save load is user-initiated, never on the boot path.

The container also stores:
- a **data fingerprint** (the detection-table hash of the `GameData` the save was
  made against — PLAN §2.3) so loading a save against the wrong/updated data set
  is caught, not silently corrupt — a **load-bearing** guard;
- the **PRNG seed** and **tick index** at save time — **provenance only**, not a
  load-bearing binding (D-SAVE4: resume uses the captured PRNG *state*, not the
  seed);
- the **flavor id** (`adnd1`) — a *selector* that re-binds the dialect via
  `restore(snapshot, &Flavor)`, never an embedding (D-VM3: "the dialect is
  re-bound at restore, never embedded in the snapshot"); a save naming an
  unavailable flavor is rejected like a version mismatch.

Reject-not-migrate is also the *inter-format* rule: an original `savgam?.dat` is
**never** loaded by the `.rsav` path — import is a separate, explicit entry point
(D-SAVE5) that produces a fresh engine state, which is then saved as `.rsav`.

### D-SAVE3 — Content inventory (what the envelope carries)

`SaveState` is the full, restorable engine state. Carried:

- **EclMachine snapshot** (D-VM3, verbatim): resident block bytes, parsed vectors,
  compare flags, the 15 persistent string registers, the GOSUB call stack, the
  activation stack with per-activation `Pending` (phase + decoded operands + the
  outstanding Request stored **verbatim** for `pending()` re-presentation), and
  the snapshot version tag. This is what makes save-anywhere strictly more
  general than the original (§1.2): our save can sit mid-instruction (camp save
  above a parked PROGRAM-9 frame), the original's cannot.
- **Shell / flow / widget state** (D-UI2): the `Shell` enum, `VmPhase`, the active
  `Widget`, flow-plan cursors (`BootFlow`/`StepFlow`/`ChainFlow` position), the
  persistent `chained` flag, `party_killed`, and the **presentation queue** (paced
  text jobs, draw commands, gates) — all serde-able by construction (D-UI2 §3).
- **Engine state** (the D-UI1/D-UI2 M3 slice): `game_state`/`last_game_state`,
  position (`mapPosX/Y/dir/wallType/wallRoof`), `search_flags`, `game_speed`, the
  text cursor (`textXCol/YCol`) + `bottomTextHasBeenCleared`, `LastSelectedPlayer`
  / `LastEclBlockId`, the mutable **palette** (SetEgaPalette remaps, renderer §1.1),
  and the **per-tick presentation phase that changes pixels** — the pacing
  accumulator (as **fixed-point**, not `f32`, D-SAVE1), the active animation's
  frame index + countdown, and each queued paced-text job's emit-progress (how
  many characters already shown). These are stored because a render-all recompose
  on restore cannot re-derive them, and omitting them makes a save taken
  mid-paced-text / mid-animation diverge from the continuous run on the very next
  frame — breaking D-SAVE4's identical-frame-hash guarantee. (Stored because not
  reconstructable from ids alone.)
- **ScriptMemory window state** (D-VM5): the Area / Party / Table window backings
  as **named cells + the raw fallback store** — i.e. the same content the original
  keeps in `area_ptr`/`area2_ptr`/`stru_1B2CA` (§1.4). The **unknown-access log is
  excluded** (diagnostic, not state).
- **Party model state** — the party roster (each member's full character record +
  items + affects + memorized-spell state + reconstructed-equipment set). This
  doc **constrains** the field set (D-SAVE11) without designing the model; step 4
  owns the struct shape, and its serde derive is the storage mechanism.
- **PRNG state** (D9) — the engine's single PRNG internal state, so post-load rolls
  continue the exact sequence (a save-anywhere taken between two combat rolls
  resumes bit-identically — the D9 replay guarantee).
- **Resident-asset ids, not bytes** — `setBlocks[0..2]`,
  `current_3DMap_block_id`, `HeadBlockId`, loaded bigpic/SPRIT/PIC/portrait ids
  (§1.5). On restore, bytes are re-fetched from `GameData` and `LoadWalldef` /
  `Load3DMap` re-run (mirroring `loadSaveGame:1074-1095`). The **data
  fingerprint** (D-SAVE2) binds the save to the detected data *version* (PLAN
  §2.3), so those ids resolve against the same asset set. (Note the resident ECL
  block itself is stored as *bytes* in the EclMachine snapshot — D-VM3, it may be
  self-modified — not as a reload id; only the *import* path reloads the block by
  id, §1.5.)

**Excluded** (reconstructed or ephemeral): the framebuffer (recomposed by a
render-all on restore — renderer's `LoadPic`/`RedrawView` ancestor); the
re-derivable caches (last-picture, roof — renderer §1.8; the fade-recolor cache,
whose dither is already non-comparable, renderer §1.11 item 4); the `VerifyReport`
(D-RP4: "never serialized into saves"); the unknown-access log and any
diagnostics; and genuinely tick-transient scratch with **no** pixel effect. The
pacing accumulator and animation phase are **not** excluded (see Engine state
above — they change pixels and must round-trip). The rule: **store what cannot be
re-derived *or affects pixels*; recompute the rest.**

### D-SAVE4 — A save is a replay checkpoint (H5 tie-in)

D9 makes a session fully described by `(data fingerprint, seed, input trace)`; a
save materializes the *resulting state* at a tick.
- **A `.rsav` is self-contained: PRNG state (D-SAVE3), not the seed, drives
  resume.** Loading a save and continuing produces the same stream as the
  un-interrupted run because the PRNG internal state is restored verbatim.
  `seed` and `tick_index` in the header are therefore **provenance, not a
  load-bearing binding** — they record which session/tick this state came from
  (useful when pairing a save with an input trace during H5 debugging), but resume
  needs neither. The header does **not** claim to bind a specific trace (unlike
  `data_fingerprint`, which does guard the asset set); "replay the trace from
  `tick_index`" is a *debugging affordance* over a separately-held trace, not a
  property the save enforces. If a future feature needs verifiable
  replay-from-checkpoint, it must reference-and-fingerprint the trace — out of M3
  scope.
- **The H5 checkpoint hash and a save share one canonical serialization, over the
  payload only.** An H5 checkpoint is `hash(postcard(SaveState))` at a tick; a
  save is the same `postcard(SaveState)` bytes un-hashed, wrapped in the header.
  **The header (magic, versions, `seed`, `tick_index`, `data_fingerprint`,
  `flavor`) is metadata and is excluded from the state hash** — two implementers
  must not disagree here, or every H5 comparison mismatches. So "does our replay
  still match?" (the H5 hash) and "does load/save round-trip?" (`.rsav`) exercise
  the *same* serialization path (§4).
- **The bit-identical guarantee is scoped to what `SaveState` captures.** With the
  pacing accumulator and animation phase now included (D-SAVE3), a save taken
  mid-paced-text or mid-animation resumes to the same next-frame hashes; the v1
  exclusion of those would have broken this.

### D-SAVE5 — Original import: a separate read-only entry point

Import is `import_original(save_set, &GameData, &Flavor) -> Result<EngineState>`
— distinct from `.rsav` load (D-SAVE2). It reads the §1.1 file-set:
- locate the master `savgam<X>.dat` (slot `X`), parse the 11 fixed sections
  (§1.1 table) — a flat 13149-byte read, no decompression;
- for each of `party_count` names, read `CHRDAT<X><n>.sav` (0x1A6 record, §1.3)
  and its optional `.swg` (items) / `.fx` (affects) siblings;
- populate: the three ScriptMemory windows from sections 2–4 (§1.4: named cells
  **and** raw store — do **not** clear `field_200[]`/`field_6F2..704`, §1.4);
  the resident block **reloaded by `area_ptr.LastEclBlockId` from `GameData`**
  (§1.5 — section 5's saved bytes are discarded, exactly as `loadSaveGame`
  discards them), the EclMachine starting **idle** (empty activation stack, per
  §1.2, because the original stored no VM registers);
  position/game_state from sections 1/6/7/8; wallset reload from `setBlocks`
  (§1.5, re-run `LoadWalldef`/`Load3DMap` against `GameData`); the party roster
  from the `CHRDAT` records (reconstruct equipment from item readied flags,
  §1.7 item 3; use `i*5` spell stride, the pinned stat byte order, and Str00
  unclamped to `0..=100`, §1.7 items 1/2/5).

The import path lives in `gbx-formats` (PLAN §2: gbx-formats owns "original save
files"); it produces plain data that `gbx-engine` assembles into an `Engine`.
After import, the engine is in the exact state a fresh walk-loop entry expects
(§1.2) — the exit-gate "walk around, transact in a shop, level up" then exercises
the *same* code paths as a native `.rsav` load. Direction is **import-only**:
exporting restrike state back to `savgam?.dat` is a non-goal (D-SAVE12).

### D-SAVE6 — The character record is transcribed from §1.3, two cells pinned

The 0x1A6 layout (§1.3) is the import spec. The three flagged cells (§1.7 items
1, 2, 5) are resolved **against real data before the M3 gate**, not guessed: stat
byte order via a drained-stat character; `spellCastCount` via known memorized
counts; the Str00 range (`0..=100`, not coab's buggy 25 clamp) via an 18/xx
fighter. Until pinned, the importer carries each as a named constant with a single
flip point and a docket cite (§5.2). Pointer fields (§1.7 item 3) are read as
padding and discarded; equipment is reconstructed post-item-load.

### D-SAVE7 — Import populates named cells **and** the raw window store

Per D-VM5, each window is named-cells-over-raw. Import writes the raw 0x800/
0x800/0x400 blobs into the window backing **first** (so every script-stashed word
round-trips, including cells restrike hasn't named yet), **then** the named cells
are read *through the same facade* — no separate parallel decode. This guarantees
an imported save that touches an unnamed Area/Table cell behaves exactly as the
original (the discovery-backlog property, D-VM5), and it means import fidelity
does not depend on restrike having named every cell by M3.

### D-SAVE8 — What cannot be imported, and why

- **Mid-combat saves** — do not exist in the original (§1.2, verify docket §5.1);
  nothing to import. M4 decides whether *restrike's own* `.rsav` may save
  mid-combat (D-SAVE12 non-goal here).
- **Live VM control flow** — the original stores no activation stack / string
  registers / pending Request (§1.2). Import therefore starts the EclMachine
  **idle**; the walk loop re-enters via the entry vector (renderer §1.6), exactly
  as `loadSaveGame` does (`reload_ecl_and_pictures=true`). A camp-menu original
  save (parked PROGRAM-9) imports as a plain exploration state at the saved
  position — faithful, because the original itself re-enters the walk loop on
  load, it does not resume the parked camp frame (vm-scriptmemory.md §1: PROGRAM-9
  ends in `CMD_Exit`).
- **Framebuffer / caches** — not in the original save; recomposed on load
  (D-SAVE3 exclusions).

### D-SAVE9 — Pool/Hillsfar transfer characters: spec now, implement at M6

The conversion path (§1.6) is specified — `.cha`/`.hil` record → CotAB 0x1A6
record via coab's `ConvertPoolRadPlayer` / `ConvertHillsFarPlayer` field maps,
with stat-limit enforcement and the class/affect grants. **Implementation may
defer to M6** (PLAN M3 scopes only the CotAB `savgam?.dat` exit gate; the
transfer-character UI belongs with new-party creation / the wider roster flow).
Recorded here so the M6 work is a transcription, not a rediscovery. Direction is
import-only (a Pool/Hillsfar character *into* restrike-CotAB), never export.

### D-SAVE10 — Validation is four tiers

1. **In-repo synthetic fixtures (D10-clean, CI).** Hand-authored bytes — a
   minimal `savgam?.dat` (1 party member, position (0,0), zeroed quest flags, a
   trivial resident block that halts cleanly) + one `CHRDAT` record with known
   field values. These bytes are *structural and self-authored*, carrying no
   extracted game content (same posture as `fixtures/tilverton-circuit.jsonl`
   carrying only inputs), so they ship freely. Assert: import parses every
   section at the right offset/size; the character record decodes to the authored
   values (guards the §1.3 offset table and the three pinned cells); the window
   backings land in the right ScriptMemory addresses; a `.rsav` round-trip
   (import → save → load → save) is byte-identical. **The round-trip alone proves
   only an in-process fixed point** — it can pass while cross-machine bytes diverge
   (`HashMap` order, header endianness). So CI also pins a **committed golden
   SHA-256 of a synthetic `.rsav`**, asserted on all three OSes + wasm32 (the
   fixture is D10-clean self-authored bytes, so its hash ships freely) — that is
   what actually guards the D-SAVE1 collection-ordering and the §3 header-encoding
   invariants. *(Implementation note, Fable review: CI currently only
   `cargo check`s wasm32 — asserting the golden on wasm needs a wasm test
   runner added to CI (e.g. wasmtime/wasm-pack for the one test), or the wasm
   leg is deferred with a comment until one exists; the three-OS legs run
   today.)*
2. **Local real-save import (GBX_DATA_DIR, loud-skip when absent).** The user
   makes a real CotAB save in DOSBox under their own save dir (never committed,
   D10); a local-only test imports it and asserts structural sanity (party_count
   matches file count, positions in range, the six ability scores in `0..=25` and
   exceptional-strength Str00 in `0..=100` — **not** coab's buggy 25 clamp, §1.7
   item 5 — and other field bounds) and a full `.rsav` round-trip.
   Pack-authoring-style evidence recorded in the commit message (repo convention).
3. **Human tier: character screen vs DOSBox.** Import a real mid-game save, open
   restrike's character screen, and compare field-by-field against DOSBox showing
   the *same* save (the D-SAVE6 procedure resolves the three flagged cells here).
   Spec'd like `docs/dosbox-capture.md`: exact save slot, the fields to compare
   (name, race/class, the six stats current+max, AC, HP cur/max, XP, per-class
   levels, memorized spells, money, equipped items). This is the M3 exit-gate
   evidence for import correctness.
4. **GBC oracle tier (deferred to M4's oracle rig).** GBC opens a CotAB save and
   renders the character; our imported values compared against GBC's view
   (PLAN M3: "prove against GBC's view of the same save"). Deferred with the rest
   of the oracle-VM setup (M4), consistent with the M2 DOSBox-checklist deferral.

### D-SAVE11 — Constraints this doc imposes on the party model (step 4)

The import record (§1.3) is the field-completeness driver: **if the original
save stores it, our party model must hold it** (else import is lossy and the exit
gate's "level up with correct numbers" can't reproduce). The model must hold, per
member:
- **identity**: name, race, class (incl. multiclass combo id), sex, alignment,
  `age` (i16, drives `AgeEffects`), monster type/index, icon
  id/size/head/weapon/colours, party order, control (PC/NPC + morale);
- **the seven ability scores** as current+max pairs (STR incl. exceptional STR%);
- **level/XP**: `exp` (i32), the 8-entry per-class current levels + 8-entry
  former (dual-class) levels, HitDice/multiclassLevel, drained levels/hp
  (`lost_lvls`/`lost_hp`);
- **HP**: max, current, rolled (the `roll` vs `+CON` split the level-up flow needs
  — D-RP5);
- **combat**: base + current THAC0, base + current AC (+ac_behind), attacks
  count/half-moves, the two attack profiles (dice count/size/bonus, base +
  current), `hitBonus`, movement/initiative, weapons-hands-used, weight;
- **magic**: the 100-byte known-spellbook, the 84-byte memorized `spellList`, the
  3×5 memorized-cast counts, `spell_to_learn_count`;
- **skills/saves**: the 8 thief skills, the 5 saving throws, `attackLevel`,
  base_movement, `classFlags` (item limits), turn-undead type (`field_E9`);
- **money**: the 7-coin `MoneySet`;
- **status**: health status, in-combat, combat team, quick-fight, paladin cures
  left, `npcTreasureShareCount`, the save-bonus `field_186`, and **every
  remaining record byte** (the `field_XX` cells) held as opaque named fields
  until a consumer needs them — the D-VM5 raw-store discipline applied to the
  character record, so completeness (and the round-trip test) is by construction,
  not by enumeration;
- **inventory**: the item list (from `.swg`) and active/readied equipment set
  (reconstructed, §1.7 item 3); the affect list (from `.fx`).

This is an enumeration, not a struct — step 4 designs the shape (grouping,
newtypes, which `field_XX` stay opaque). The test is round-trip: import → model →
`.rsav` → reload must preserve every listed datum.

### D-SAVE12 — Non-goals

- **Mid-combat saves** — M4 decides whether restrike's `.rsav` may save inside a
  combat round; out of scope here (the original never does, §1.2).
- **Original-format export** — restrike never writes `savgam?.dat`/`CHRDAT`;
  import is one-way. (GBC interop / round-tripping reconsidered at M8 with the
  companion tooling, per PLAN M3's framing.)
- **Save migration tooling** — reject-not-migrate pre-1.0 (D-SAVE2); no
  cross-version `.rsav` upgraders.
- **Cloud/slot UX** — beyond minimal file naming (`<name>.rsav`); the save/load
  *menu* is engine-UI work (M3 shell), not a format concern.

## 3. The formats, by example

```rust
// gbx-formats + gbx-engine — shapes only; names bikesheddable at implementation.

// ---- Our envelope (D-SAVE1) --------------------------------------------------
// File = [ContainerHeader][postcard(SaveState)]. Header is fixed & greppable.
// Hand-encoded LITTLE-ENDIAN, field-by-field, NO padding — NOT repr(C)/transmute
// (repr(C) pins neither endianness nor alignment padding). container_version is
// parsed & checked before the payload (D-SAVE2). The H5/state hash covers
// postcard(SaveState) ONLY — this header is excluded (D-SAVE4).
struct ContainerHeader {
    magic: [u8; 4],            // b"RSAV"
    container_version: u16,    // header layout version — checked FIRST
    save_format_version: u32,  // D-SAVE2 — single authority for the payload; reject-not-migrate
    flavor: FlavorId,          // selector: re-binds via restore(.., &Flavor); reject if unavailable
    data_fingerprint: [u8; 32],// detection-table hash of GameData (PLAN §2.3) — load-bearing guard
    seed: u64,                 // provenance only (D-SAVE4) — resume uses SaveState.prng, not this
    tick_index: u64,           // provenance only (D-SAVE4) — session coordinate, not a trace binding
    payload_len: u64,
}

#[derive(Serialize, Deserialize)]   // postcard — deterministic, wasm-clean
struct SaveState {
    ecl: EclSnapshot,          // D-VM3 verbatim (block bytes, regs, call stack, activations+pendings)
    shell: ShellSnapshot,      // D-UI2 (Shell/VmPhase/Widget, flow cursors, chained, party_killed, queue)
    engine: EngineSnapshot,    // game_state, position, search_flags, game_speed, palette, cursor,
                               //   + pixel-affecting tick phase: pacing accumulator (FIXED-POINT,
                               //   not f32), animation frame index+countdown, paced-text emit-progress
    windows: WindowsSnapshot,  // Area/Party/Table: named cells + DETERMINISTIC raw store (BTreeMap/
                               //   sorted, never HashMap — D-SAVE1); NO unknown-access log
    party: PartySnapshot,      // step-4 model, serde-derived (D-SAVE11 field set)
    prng: PrngState,           // D9 — opaque to this doc; shape follows H3's recovered generator
                               //   (M4), may not be a bare u64 seed
    assets: ResidentAssetIds,  // setBlocks, 3DMap id, HeadBlockId, bigpic/sprit/pic ids (D-SAVE3);
                               //   the resident ECL block itself rides in `ecl` (D-VM3), not here —
                               //   only IMPORT reloads it by id (§1.5)
    // excluded (re-derivable / no pixel effect): framebuffer, last-picture+roof+fade caches,
    //   VerifyReport, unknown-access log. NOTE pacing accumulator + animation phase are NOT
    //   excluded — they change pixels (D-SAVE3/D-SAVE4).
}

// ---- Original import (D-SAVE5) ----------------------------------------------
struct OriginalSaveSet {         // located under the user's save dir, slot X
    master: [u8; 13149],         // savgam<X>.dat, §1.1 sections 1..11 (flat)
    chars: Vec<OriginalChar>,    // one per party_count
}
struct OriginalChar {
    record: [u8; 0x1A6],         // CHRDAT<X><n>.sav, §1.3
    items:   Vec<[u8; ITEM_SIZE]>,   // CHRDAT<X><n>.swg (optional)
    affects: Vec<[u8; AFFECT_SIZE]>, // CHRDAT<X><n>.fx  (optional)
}

fn import_original(set: &OriginalSaveSet, data: &GameData, flavor: &Flavor)
    -> Result<EngineState, ImportError>;
// Sections 2-4 -> WindowsSnapshot (raw blob first, then named cells read through
// the facade, D-SAVE7). Section 5 bytes IGNORED -> reload pristine ECL block by
// area_ptr.LastEclBlockId from `data` (§1.5); EclMachine starts idle (empty
// activation stack, §1.2). Section 9 -> LoadWalldef/Load3DMap against `data`
// (§1.5). CHRDAT records -> PartySnapshot (i*5 spell stride, pinned stat order,
// Str00 unclamped 0..=100 — §1.7 items 1/2/5).
```

The `savgam<X>.dat` section table (§1.1) and the 0x1A6 record table (§1.3) are the
byte-level spec; the importer is a straight transcription of them with the three
§1.7 cells pinned against real data.

## 4. Testing

- **In-repo (CI, D10-clean):** the D-SAVE10 tier-1 synthetic fixtures — section
  offset/size parse, character-record decode to authored values (guards §1.3 +
  the three pinned cells), window-address placement, `.rsav` round-trip
  byte-identity, and the **committed cross-platform golden SHA-256** on all 3 OSes
  + wasm32 (catches `HashMap`-order / header-endianness nondeterminism the
  in-process round-trip cannot — D-SAVE1/D-SAVE10). Plus a `SaveState` serde
  round-trip property test (`postcard(deserialize(x)) == x`) and a version-mismatch
  rejection test (D-SAVE2). The import parser joins the fuzz roster (untrusted user
  bytes, PLAN M1 convention) — malformed `savgam?.dat`/`CHRDAT` must error, never
  panic.
- **Local (GBX_DATA_DIR, loud-skip when absent):** import a real save →
  structural sanity + field-bound checks + `.rsav` round-trip (D-SAVE10 tier 2);
  evidence recorded in the commit message.
- **Human tier:** the character-screen-vs-DOSBox comparison (D-SAVE10 tier 3) —
  the M3 exit-gate import evidence; also pins §1.7 items 1, 2, 5.
- **Oracle tier (M4):** GBC's view of the same save (D-SAVE10 tier 4), deferred
  with the oracle rig.
- **Replay coherence (H5):** a save taken at tick T, then the input trace replayed
  from T, produces the same frame hashes as the un-saved run (D-SAVE4) — shares
  the canonical serialization with H5 checkpoints.

## 5. Open questions → fidelity docket

1. **Mid-combat save unreachability** — confirmed by code path (§1.2: combat
   never re-enters the VM; save sites are PROGRAM-0 menu + camp only). Verify
   against DOSBox that the game menu is genuinely unreachable during a combat
   round (no keybind opens it mid-fight). Low risk; a `docs/fidelity-docket.md`
   entry, checkable in the M4 oracle hour.
2. **The three record-cell hazards (§1.7 items 1, 2, 5)** — stat byte order (coab
   `cur,full` vs GBC `original,current`), `spellCastCount` stride (coab `i*i` bug
   vs GBC `i*5`), and the Str00 25-clamp (coab clamps exceptional strength to 25;
   use `0..=100`). All resolved against real data before the M3 gate (drained-stat
   character; known memorized counts; an 18/xx fighter). Docket entries with the
   exact disambiguating procedure; the importer carries one flip point per cell
   until pinned.
3. **Table window (`stru_1B2CA`) semantics** — imported as a raw 0x400 word store
   (§1.4); if the M3/M4 work names cells in it, they become named-over-raw like
   the other windows. No import blocker (raw round-trips).
4. **`game_area` boot default** — vm-scriptmemory.md §5 item 8 flagged coab's
   `game_area` init as UNSURE (engine pins 2). Import reads it from section 1 /
   `area2_ptr.game_area` (0x624) directly, sidestepping the boot-default question
   for imported saves; native new-game keeps the engine's fixed 2. Cross-ref only.
5. **Item / Affect record layouts** — this doc treats `.swg`/`.fx` records as
   opaque fixed-size blobs (`Item.StructSize`/`Affect.StructSize`); their
   field-level layout is a party-model/inventory concern (step 4 + M4), spec'd
   from coab's `Item.cs`/`Affect.cs` when the model lands. Named here so the
   import file-set is complete even though the record interiors are deferred.
6. **Pool/Hillsfar record layouts** (`PoolRadPlayer`/`HillsFarPlayer` +
   `.spc`) — transcribed from coab (§1.6) at M6, not M3.
7. **Spellbook slot→spell interpretation** — the 100-byte `spellBook` (0x79–0xDC)
   is imported as **raw bytes** and round-trips regardless of enum order; but any
   code that *interprets* a slot as a named spell depends on restrike's `Spells`
   enum matching the on-disk slot layout (the GBC doc's 0x79-based table with its
   gaps). That mapping is a rules/party-model concern (D-RP5), pinned when the
   model interprets the bytes, not at import.

## 6. What this unblocks (M3 build order)

1. `gbx-formats`: the `savgam?.dat` section parser + `CHRDAT` record decoder
   (§1.1/§1.3), against tier-1 synthetic fixtures — first test: parse a
   hand-authored 1-character save end to end.
2. `gbx-formats`/`gbx-engine`: the `.rsav` envelope (D-SAVE1) — `ContainerHeader`
   + `postcard(SaveState)`, version-reject, round-trip determinism tests. This
   depends on step 4's serde-able party model existing, so it lands *with* the
   model, not before.
3. the party/character **model** (separate step, constrained by D-SAVE11) — its
   field set is fixed by §1.3; its serde derive is the `.rsav` storage mechanism.
4. `import_original` (D-SAVE5) wiring the two above into a real engine state,
   pinning the §1.7 cells against real data (tier-3 human check) — the M3 exit
   gate: import → walk → shop → level-up → `.rsav` round-trip.
5. M6 inherits the Pool/Hillsfar transfer path (D-SAVE9), a transcription of §1.6.
```
