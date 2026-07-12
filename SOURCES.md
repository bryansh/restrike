# SOURCES.md — Provenance Ledger

Purpose: track which reference informed each part of this engine — a coab
file, a forum thread, a blog post, a manual page, or our own reverse
engineering — so behavior in this codebase is always traceable to evidence.
Per PLAN.md D11, reference implementations (notably coab) are read for
behavior and cited here, never copied; any logic ported from a
GPL-compatible source (e.g. ssi-engine) gets its own row noting the license
and exact provenance.

Add a row whenever a subsystem, table, or algorithm is implemented from an
external reference. Keep entries small and specific — link to the exact
file/section/thread/post, not just a project name.

## Ledger

| Subsystem | Source | Type | License / terms | Notes |
|---|---|---|---|---|
| `gbx-vm` ECL execution model: fetch/dispatch loop, vector-table block header, operand modes, persistent string registers, compare/IF/skip semantics, nested runs (camp), block-chaining (NEWECL/PROGRAM), text pagination, CotAB opcode table (`0x00`–`0x40`) | coab `engine/ovr003.cs` (`RunEclVm`/`sub_29607`, `SetupCommandTable`, `CmdItem.Skip`, `TryEncamp`, `CMD_*` handlers), `engine/ovr008.cs` (`vm_LoadCmdSets`, `vm_init_ecl`, `load_ecl_dax`), `engine/seg041.cs` (`press_any_key` pagination), `Classes/Opperation.cs`, `Classes/EclBlock.cs` | reference | unclear (transliterated from the disassembled binary) | Read for behavior, no code copied (D11). Findings recorded in [docs/design/vm-scriptmemory.md](docs/design/vm-scriptmemory.md), incl. corrections from the 2026-07-11 adversarial review. |
| ScriptMemory address windows (`0x4B00`/`0x7A00`/`0x7C00`/`0x8000` ranges), write-side-effect cells, named globals | coab `engine/ovr008.cs` (`vm_GetMemoryValueType`/`sub_30723`, `vm_GetMemoryValue`/`sub_30F16`, `vm_SetMemoryValue`, `alter_character`), `Classes/Gbl.cs` field naming | reference | unclear (as above) | Seed for the per-game address map; unknown cells discovered via access log at runtime. |
| M1 step-0 opcode channel classification (65-opcode table, EngineServices candidate surface): character/party records, monster/NPC load, combat entry, treasure/item instantiation, saving throws, map/file loads, save/menu/camp flow, RNG-backed rolls | coab `engine/ovr004.cs` (`copy_protection`), `engine/ovr006.cs` (`AfterCombatExpAndTreasure`), `engine/ovr009.cs` (`MainCombatLoop`), `engine/ovr016.cs` (`MakeCamp`), `engine/ovr017.cs` (`load_mob`/`load_npc`, `SaveGame`), `engine/ovr018.cs` (`startGameMenu`, `FreeCurrentPlayer`), `engine/ovr019.cs` (`end_game_text`), `engine/ovr021.cs` (`step_game_time`), `engine/ovr022.cs` (`create_item`), `engine/ovr024.cs` (`roll_dice`, `RollSavingThrow`, `CanHitTarget`), `engine/ovr025.cs` (`PartySummary`, `selectAPlayer`, `reclac_player_values`, `ItemDisplayNameBuild`, `LoadPic`, `display_map_position_time`), `engine/ovr027.cs` (`yes_no`, `sl_select_item`, `displayInput`, `ClearPromptAreaNoUpdate`), `engine/ovr029.cs` (`RedrawView`), `engine/ovr030.cs` (`load_bigpic`, `load_pic_final`, `draw_bigpic`, `DrawMaybeOverlayed`), `engine/ovr031.cs` (`Load3DMap`, `LoadWalldef`, `get_wall_x2`, `getMap_wall_type`), `engine/ovr034.cs` (`chead_cbody_comspr_icon`), `engine/seg042.cs` (`load_decode_dax`), `engine/seg043.cs` (`print_and_exit`, `GetInputKey`, `clear_keyboard`), `engine/seg049.cs` (`SysDelay`/`GameDelay`), `engine/seg051.cs` (`Random`), `Classes/Area2.cs` (`field_800_Get`/`field_67E` mapping), `Classes/MoneySet.cs` (`ClearAll`), `Classes/Player.cs` (`HasAffect`, `SkillLevel`, `thief_skills`, `movement`), `Classes/Item.cs` (`StructSize`, signature only) | reference | unclear (as above) | Read for behavior, no code copied (D11). Full per-opcode findings, including two confirmed contradictions of vm-scriptmemory.md v3 (PARLAY/TREASURE presentation-point claims) and 15 new fidelity-docket candidates, recorded in [docs/design/opcode-classification.md](docs/design/opcode-classification.md). `engine/seg037.cs`, `engine/seg044.cs`, `engine/seg001.cs`, `engine/ovr020.cs`, `engine/ovr026.cs`, `engine/ovr033.cs` were named by call sites but not independently read. |

### Column guide

- **Subsystem** — the engine area or file this entry covers (e.g. `gbx-vm`
  opcode dispatch, THAC0 table, DAX container format).
- **Source** — the specific reference: repo + file/commit, forum thread +
  post, blog post URL, manual page, or "original RE" for work done directly
  against the binary/data with no secondary source.
- **Type** — one of: `reference` (read for behavior, not copied), `ported`
  (logic adapted from a compatible-license source), `data` (uncopyrightable
  facts/tables extracted from the binary or docs), `original-re` (derived
  from our own disassembly/black-box testing).
- **License / terms** — the source's license, or "N/A" for docs/forums used
  purely as documentation.
- **Notes** — anything a future contributor needs to judge provenance at a
  glance (e.g. "transliterated logic, not copied — see D11").
