# ECL Opcode Channel Classification (M1 step 0)

> Produced per `docs/design/vm-scriptmemory.md` v3, §6 build-order item 0. Every
> row was built by reading the coab (CotAB dialect) handler in full — never
> copied, read-for-behavior per D11 — and cross-referencing the operand decoder
> (`ovr008.cs vm_LoadCmdSets`), the memory-window dispatcher
> (`vm_GetMemoryValueType`/`vm_GetMemoryValue`/`vm_SetMemoryValue`), and whatever
> subsystem files each handler calls into. All citations are to
> `~/src/goldbox-refs/coab/`. Four independent research passes covered
> `0x00–0x10`, `0x11–0x20`, `0x21–0x30`, `0x31–0x40`; each is later
> cross-checked against the others where opcodes share a handler or a pattern
> (both MENUs, the six IFs, AND/OR, ON GOTO/ON GOSUB, LOAD FILES/LOAD PIECES).
>
> **Channel tags**: **MACHINE** (pc/flags/stack/string-registers — pure VM
> state), **MEM** (ScriptMemory read/write through the address windows), **SVC**
> (a synchronous EngineServices call touching game entities that aren't raw
> memory cells), **EFF** (buffered presentation output, non-blocking from the
> VM's perspective), **REQ** (a Request that suspends the activation awaiting a
> reply). An opcode commonly carries more than one tag.
>
> **Run batches** = the total `vm_LoadCmdSets` consumption during normal
> execution. **Skip size** = transcribed verbatim from `SetupCommandTable`
> (`ovr003.cs:2063–2110`) — never derived. **skip≠run** = does the declared
> skip size actually reproduce what the run path consumes.

## 1. The 65-opcode table

| op | name | handler (file:line) | run batches | skip size | skip≠run | channels | services called | engine state touched | modal points | notes |
|---|---|---|---|---|---|---|---|---|---|---|
| 0x00 | EXIT | `CMD_Exit` ovr003.cs:9-42 | 0 (direct pc++) | 0 | NO | MACHINE, SVC | inline `SelectedPlayer=LastSelectedPlayer` restore (not a named call, but the taxonomy's own "SelectedPlayer retargeting" example) | `restore_player_ptr`, `SelectedPlayer`, `LastSelectedPlayer`, `encounter_flags[0..1]`, `spriteChanged`, `stopVM`, `ecl_offset`, `vmCallStack` (cleared), text cursor reset | none | Resets text cursor to (1, 0x11) — cosmetic. |
| 0x01 | GOTO | `CMD_Goto` ovr003.cs:45-53 | 1×1 (fixed) | 1 | NO | MACHINE | none | `ecl_offset` | none | Uses `cmd_opps[1].Word` directly, never `.GetCmdValue()` — a target encoded mode 0x01/0x03 ("address") never actually triggers a ScriptMemory read. Destination/target operands are uniformly raw-`.Word` across the whole table (see docket note). |
| 0x02 | GOSUB | `CMD_Gosub` ovr003.cs:56-65 | 1×1 (fixed) | 1 | NO | MACHINE | none | `ecl_offset`, `vmCallStack` (push) | none | Same raw-`.Word` behavior as GOTO. |
| 0x03 | COMPARE | `CMD_Compare` ovr003.cs:68-87 | 1×2 (fixed) | 2 | NO | MACHINE, MEM | Party-window read-through (`get_player_values`) if a numeric operand mode is 0x01/0x03 and targets `0x7C00-0x7FFF` | `compare_flags[0..5]`, `unk_1D972[1..2]`, `cmd_opps[1..2]` | none | String path compares `unk_1D972[2]` vs `unk_1D972[1]` whenever either operand's `Code>=0x80` (ovr003.cs:72-77) — confirms v3's staleness claim verbatim. |
| 0x04 | ADD | `CMD_AddSubDivMulti` ovr003.cs:90-130, case 4 (103-105) | 1×3 (fixed) | 3 | NO | MACHINE, MEM, SVC | Party-window read/write-through (`get_player_values`/`alter_character`) if operands/destination fall in `0x7C00-0x7FFF` | `cmd_opps[1..3]`; destination cell via `vm_SetMemoryValue` | none | Destination write uses raw `.Word`, never `.GetCmdValue()`. |
| 0x05 | SUBTRACT | `CMD_AddSubDivMulti` ovr003.cs:90-130, case 5 (107-109) | 1×3 (fixed) | 3 | NO | MACHINE, MEM, SVC | same as ADD | same as ADD | none | Result is `operand2 - operand1` (B−A), not A−B. |
| 0x06 | DIVIDE | `CMD_AddSubDivMulti` ovr003.cs:90-130, case 6 (111-114) | 1×3 (fixed) | 3 | NO | MACHINE, MEM, SVC | same as ADD | quotient via `vm_SetMemoryValue`; remainder via **direct field write** `gbl.area2_ptr.field_67E` (ovr003.cs:113) | none | **Finding**: `field_67E` bypasses `vm_SetMemoryValue` (no window dispatch, no write hooks). But `Area2.field_800_Get` maps struct offset `0x67E` back into the Party window's read path (`(loc*2+0x800) mod 0x10000`), so VM address **`0x7F3F`** reads the DIVIDE remainder even though the write that populates it is out-of-band. See docket. |
| 0x07 | MULTIPLY | `CMD_AddSubDivMulti` ovr003.cs:90-130, case 7 (116-118) | 1×3 (fixed) | 3 | NO | MACHINE, MEM, SVC | same as ADD | same as ADD | none | |
| 0x08 | RANDOM | `CMD_Random` ovr003.cs:132-151 | 1×2 (fixed) | 2 | NO | MACHINE, MEM, SVC | `seg051.Random(max: u8) -> u8` (ovr003.cs:145; seg051.cs:33) | `cmd_opps[1..2]`; destination via `vm_SetMemoryValue` | none | Upper bound is inclusive-adjusted: operand `<0xFF` incremented before rolling (138-141), so the script's range is inclusive. |
| 0x09 | SAVE | `CMD_Save` ovr003.cs:153-172 | 1×2 (fixed) | 2 | NO | MACHINE, MEM, SVC | `vm_SetMemoryValue`/`vm_WriteStringToMemory`, routing through `alter_character` (numeric, Party window) or a direct `SelectedPlayer.name` write (string, `loc==0x7C00`, ovr008.cs:943) | destination cell; possibly `SelectedPlayer.name` | none | Branches on `cmd_opps[1].Code<0x80` — numeric vs string write. Confirms v3's "SAVE is a plain memory/string write, no game-save opcode" claim. |
| 0x0A | LOAD CHARACTER | `CMD_LoadCharacter` ovr003.cs:174-213 | 1×1 (fixed) | 1 | NO | MACHINE, MEM (operand read), SVC, EFF | `FreeCurrentPlayer(player, free_icon, leave_party_size) -> Player` (ovr018.cs:1580); `PartySummary(player)` (ovr025.cs:216, EFF); `TeamList` indexed lookup | `SelectedPlayer`, `LastSelectedPlayer`, `restore_player_ptr`, `player_not_found`, `redrawPartySummary1/2` | none | High bit (0x80) of the operand triggers a "free current player + redraw" side path gated on both redraw flags. Index masked `&0x7F`, validated against `TeamList.Count`; out-of-range sets `player_not_found` (no error). |
| 0x0B | LOAD MONSTER | `CMD_LoadMonster` ovr003.cs:238-298 | 1×3 (fixed) | 3 | NO | MACHINE, MEM (operand reads), SVC | `load_mob(monster_id: u8) -> Player` (ovr017.cs:819/824, loads `.dax` files); `chead_cbody_comspr_icon(icon_idx, block_id, kind)` (ovr034.cs:52, asset/icon load) | `TeamList` (append), `numLoadedMonsters`, `monster_icon_id`, `monstersLoaded`, `SelectedPlayer` (saved/restored around the call) | **edge case**: `load_mob` → `DisplayAndPause` (seg041.cs:297) + `seg043.print_and_exit()` (ovr017.cs:836-838) if `.dax` data is missing — a fatal, non-recoverable modal, not a normal-flow Request | Caps at 63 loaded monsters; `num_copies<=0` clamped to 1. |
| 0x0C | SETUP MONSTER | `CMD_SetupMonster` ovr003.cs:215-236 | 1×3 (fixed) | 3 | NO | MACHINE, MEM (operand reads), SVC, EFF | `sub_304B4(dir,y,x) -> u8` (ovr008.cs:157, approach-distance calc from wall type); `sub_30580(...)` (ovr008.cs:220, sprite/picture presentation dispatch, EFF, non-blocking) | `sprite_block_id`, `area2_ptr.max_encounter_distance`, `area2_ptr.encounter_distance`, `pic_block_id`, `encounter_flags[0..1]` (via `sub_30580`) | none | |
| 0x0D | APPROACH | `CMD_Approach` ovr003.cs:300-310 | 0 (direct pc++) | 0 | NO | MACHINE, EFF | `sub_30580(...)` (same as 0x0C) | `area2_ptr.encounter_distance` (decremented), `encounter_flags[0..1]` (via `sub_30580`) | none | |
| 0x0E | PICTURE | `CMD_Picture` ovr003.cs:312-358 | 1×1 (fixed) | 1 | NO | MACHINE, MEM (operand read), EFF | none named — pure presentation dispatch (`load_bigpic`/`draw_bigpic`, `load_pic_final`/`DrawMaybeOverlayed`, `set_and_draw_head_body`, `RedrawView`), all confirmed non-blocking | `encounter_flags[0..1]`, `spriteChanged`, `byte_1EE8D`, `can_draw_bigpic`, `displayPlayerSprite`, `head_block_id`/`body_block_id` | none | `blockId==0xFF` is the "clear picture" sentinel (343-356). |
| 0x0F | INPUT NUMBER | `CMD_InputNumber` ovr003.cs:360-370 | 1×2 (fixed) | 2 | NO | MACHINE, MEM (write), REQ | `getUserInputShort(bg,fg,prompt) -> u16` (ovr003.cs:366; seg041.cs:276) | destination cell via `vm_SetMemoryValue` | `seg043.GetInputKey()` inside the validation loop (seg041.cs:281-291), retried until a parseable value is entered | Destination uses raw `.Word` — no MEM read on the location operand, only the write. |
| 0x10 | INPUT STRING | `CMD_InputString` ovr003.cs:372-387 | 1×2 (fixed) | 2 | NO | MACHINE, MEM (write), REQ | `getUserInputString(len,bg,fg,prompt) -> string` (ovr003.cs:378; seg041.cs:234) | destination cell via `vm_WriteStringToMemory` (may hit `SelectedPlayer.name` if `loc==0x7C00`) | `seg043.GetInputKey()` inside the input loop (seg041.cs:247) | Empty input coerced to a single space before the write (380-383) — INPUT STRING can never write a zero-length string. |
| 0x11 | PRINT | `CMD_Print` ovr003.cs:389-417 | 1×1 (fixed) | 1 | NO | MACHINE, MEM, EFF | `press_any_key(text, clearArea, fg, region)` (ovr003.cs:406) | `bottomTextHasBeenCleared`, `DelayBetweenCharacters`, `unk_1D972[1]` | `DisplayAndPause` (seg041.cs:210), **conditional** on text overflowing the display box (seg041.cs:204-216) | Operand 1: `Code<0x80` resolves via `GetCmdValue` (MEM) and is stringified into `unk_1D972[1]`; `Code>=0x80` was already placed there by `vm_LoadCmdSets` (MACHINE only). Confirms v3's pagination claim precisely. |
| 0x12 | PRINTCLEAR | `CMD_Print` ovr003.cs:389-417 (shared, branch 404-414 keyed on `gbl.command`) | 1×1 (fixed) | 1 | NO | MACHINE, MEM, EFF | `press_any_key(..., clearArea=true)` (ovr003.cs:413) | same as 0x11 plus explicit `textYCol=0x11; textXCol=1` (410-411) | same conditional pagination gate; `clearArea=true` also forces an unconditional (non-modal) redraw | Identical operand handling to 0x11; only region-clear + cursor reset differ. |
| 0x13 | RETURN | `CMD_Return` ovr003.cs:420-435 | 0 (direct pc++) | 0 | NO | MACHINE | falls through to `CMD_Exit()` if the call stack is empty | `ecl_offset` (pop from `vmCallStack`, or via `CMD_Exit`'s own advance), `vmCallStack` (pop) | none | Empty-stack RETURN silently becomes EXIT — worth flagging in the machine model. |
| 0x14 | COMPARE AND | `CMD_CompareAnd` ovr003.cs:438-461 | 1×4 (fixed) | 4 | NO | MACHINE, MEM | none | `compare_flags[0..5]` — only **flags[0]/[1]** (`==`/`!=`) are ever set; the other four are always cleared to `false`, never derived relationally | none | **Hazard**: calls `vm_GetCmdValue` unconditionally on all 4 operands with no `Code<0x80` guard; per `Opperation.GetCmdValue()` an operand with `code==0x80` never has `highSet`, so a script feeding COMPARE AND a string-mode operand would hit the C# `InvalidOperationException` path. UNSURE whether shipped scripts do this — census question. See docket. |
| 0x15 | VERTICAL MENU | `CMD_VertMenu` ovr003.cs:664-695 | 1×3 fixed + count-dependent tail: `vm_LoadCmdSets(3)` (668) → operand 3 decoded as `menuCount` (673) → `ecl_offset--` (674) → `vm_LoadCmdSets(menuCount)` (675) | 0 | **YES** | MACHINE, MEM, EFF, REQ | `press_any_key(...)` (header text, 682); `VertMenuSelect(...)` (689) → `ovr027.sl_select_item(...)` (ovr027.cs:532+) | `bottomTextHasBeenCleared`, text cursor, `unk_1D972[1..menuCount]`, `cmd_opps[1..3+menuCount]`; result written via `vm_SetMemoryValue` | `DisplayAndPause` (header pagination, conditional, seg041.cs:210); `ovr027.cs:611` `displayInput` inside `sl_select_item`'s selection loop (585-631+) — unconditional | Canonical variable-tail hazard: declared skip 0 means an IF-false landing here only advances 1 byte, stranding the pc inside the fixed 3-operand block. Result MEM write's window depends on the runtime `mem_loc` operand. |
| 0x16 | `IF =` | `CMD_If` ovr003.cs:464-477 | 0 (direct pc++, 466) | 0 | NO (the opcode's own decode is fixed 0/0 — the hazard belongs to whatever follows) | MACHINE | `SkipNextCommand()` (ovr003.cs:2130-2144) when the flag is false — dispatches through `CommandTable` to call the *next* opcode's `.Skip()` | tests `compare_flags[gbl.command-0x16]` = index 0 | none directly | `SkipNextCommand` re-reads the opcode byte at the (already-advanced) pc and looks it up — purely table-driven per whatever opcode follows; not statically analyzable. |
| 0x17 | `IF <>` | `CMD_If` ovr003.cs:464-477 | 0 | 0 | NO | MACHINE | `SkipNextCommand()` | tests `compare_flags[1]` | none directly | Same mechanism, index 1. |
| 0x18 | `IF <` | `CMD_If` ovr003.cs:464-477 | 0 | 0 | NO | MACHINE | `SkipNextCommand()` | tests `compare_flags[2]` | none directly | Same mechanism, index 2. Note COMPARE AND (0x14) never sets this flag — only COMPARE/AND/OR do. |
| 0x19 | `IF >` | `CMD_If` ovr003.cs:464-477 | 0 | 0 | NO | MACHINE | `SkipNextCommand()` | tests `compare_flags[3]` | none directly | Same mechanism, index 3. |
| 0x1A | `IF <=` | `CMD_If` ovr003.cs:464-477 | 0 | 0 | NO | MACHINE | `SkipNextCommand()` | tests `compare_flags[4]` | none directly | Same mechanism, index 4. |
| 0x1B | `IF >=` | `CMD_If` ovr003.cs:464-477 | 0 | 0 | NO | MACHINE | `SkipNextCommand()` | tests `compare_flags[5]` | none directly | Same mechanism, index 5. |
| 0x1C | CLEARMONSTERS | `CMD_ClearMonsters` ovr003.cs:758-769 | 0 (direct pc++, 760) | 0 | NO | MACHINE, SVC | `MoneySet.ClearAll()` (Classes/MoneySet.cs:41); `List<Item>.Clear()` on `items_pointer` | `numLoadedMonsters=0`, `monstersLoaded=false`, `monster_icon_id=8`, `pooled_money`, `items_pointer` | none | Pure state-clear, no operands. |
| 0x1D | PARTYSTRENGTH | `CMD_PartyStrength` ovr003.cs:772-809 | 1×1 (fixed) | 1 | NO | MACHINE, MEM, SVC | `Player.SkillLevel(SkillType) -> int`, iterates `TeamList` reading `hit_point_current`/`ac`/`hitBonus` | writes computed `power_value` via `vm_SetMemoryValue` | none | Pure computation over party stats, single MEM write. |
| 0x1E | CHECKPARTY | `CMD_CheckParty` ovr003.cs:822-907 | 1×6 (fixed — scrutinized for a hidden tail, none found; the four `setMemoryFour` writes reuse operands 3-6 already decoded) | 6 | NO | MACHINE, MEM, SVC | `TeamList.Exists(p=>p.HasAffect(affect_id))` (Player.cs:835); `player.thief_skills[]`/`player.movement` field reads | writes up to 4 result words via `setMemoryFour` (ovr003.cs:812-819, itself 4×`vm_SetMemoryValue`) | none | **Hazard, same class as 0x14**: operand 1 falls through to `vm_GetCmdValue(1)` with no `Code<0x80` guard when `Code!=1`. **Also**: `var_2` dispatch (`8001` exact / `0xA5-0xAC` thief skills / `0x9F` movement) has no `else` — unrecognized query codes are silently a no-op. See docket. |
| 0x1F | notsure 0x1f | none — null delegate, `CommandTable.Add(0x1F, new CmdItem(2, "notsure 0x1f", null))` (ovr003.cs:2094) | UNKNOWN — coab never implemented a handler | 2 | UNKNOWN | UNKNOWN | none | none | none | Genuine unknown, not resolvable by more reading (per assignment instructions, timeboxed at zero further attempts — there is no handler to read). If dispatch ever reached this opcode, `CmdItem.Run()`'s `cmd()` (ovr003.cs:2414-2417) would NPE on the null delegate. The skip path (size 2, real) is only exercised if 0x1F is skipped over by a preceding false IF — never if actually executed — implying shipped scripts likely never emit it directly. Census question. |
| 0x20 | NEWECL | `CMD_NewECL` ovr003.cs:480-498 | 1×1 (fixed) | 1 | NO | MACHINE, MEM, SVC | `load_ecl_dax(block_id)` (ovr008.cs:136-154, blocking disk load with a "Loading...Please Wait" prompt, ovr008.cs:146); `vm_init_ecl()` (ovr008.cs:89-133) | `area_ptr.LastEclBlockId`, `EclBlockId`; via `vm_init_ecl`: `spriteChanged`, redraw flags, `encounter_flags[0..1]`, `monster_icon_id=8`, **`ecl_offset=0x8000`**, **`vmCallStack.Clear()`**, **`compare_flags[0..5]=false`**, `area2_ptr.HeadBlockId=0xFF`, rest-encounter params, `can_cast_spells=false`, the 5 header vectors re-parsed via 5×`vm_LoadCmdSets(1)`, `inDungeon=1`; back in `CMD_NewECL`: `stopVM=true`, `vmFlag01=true`, `encounter_flags[0..1]=false` (redundant) | none directly (disk load is a blocking loop but not a user-input modal) | **Precisely confirms v3 §1** (`ovr008.cs vm_init_ecl:102-107`, `ovr003.cs:480-498`) line-for-line, and confirms "block bytes reloaded fresh from disk on every switch" (`load_ecl_dax` unconditionally clears+rereads `ecl_ptr`, ovr008.cs:141-151). |
| 0x21 | LOAD FILES | `CMD_LoadFiles` ovr003.cs:501-604 (shared with 0x37) | 1×3 (fixed) | 3 | NO | SVC, EFF (non-blocking redraw) | `Load3DMap(id)` (ovr031.cs:690); `LoadWalldef(set,id)` (ovr031.cs:642, up to 3×); `load_bigpic(0x79)` (ovr030.cs:231); non-blocking redraw refresh (`draw8x8_03`, `PartySummary`, `display_map_position_time`) | `filesLoaded`, `byte_1AB0B/1AB0C`, `area_ptr.current_3DMap_block_id`, `setBlocks[0..2]` | none in handler; callees not fully traced (UNSURE, timeboxed) | Branch on `gbl.command==0x21` (515). No MEM write — all 3 operands are reads. |
| 0x22 | PARTY SURPRISE | `CMD_PartySurprise` ovr003.cs:910-931 | 1×2 (fixed) | 2 | NO | MEM, SVC | iterates `TeamList` checking `player._class` (character-record read) | writes 2 operand-addressed cells via `vm_SetMemoryValue` (929-930) | none | Trivial ranger-class detector. |
| 0x23 | SURPRISE | `CMD_Surprise` ovr003.cs:934-968 | 1×4 (fixed) | 4 | NO | MEM, SVC (RNG) | `roll_dice(6,1)` ×2 (947-948) | writes a **hard-coded literal address `0x2CB`** (not operand-addressed!) via `vm_SetMemoryValue(val_a, 0x2cb)` (967) | none | Destination is a fixed global cell, not an operand — candidate for the named-global address map. See docket. |
| 0x24 | COMBAT | `CMD_Combat` ovr003.cs:971-1029 | 0 batches — no operand decode at all, direct `ecl_offset++` (973) | 0 | NO (nothing to diverge from) | REQ (Combat subsystem entry; conditionally CityShop/temple_shop), SVC, EFF, MACHINE | `sub_304B4` (approach-distance calc); `MainCombatLoop()` (ovr009.cs:22, full turn-based combat — internals UNSURE, timeboxed); `AfterCombatExpAndTreasure()` (ovr006.cs:763); conditionally `CityShop()`/`temple_shop()`; `load_bigpic(0x79)`; `LoadPic()` | `game_state`, `area2_ptr.encounter_distance/search_flags`, `encounter_flags[0..1]`, `spriteChanged` | Deep inside `MainCombatLoop`/`CityShop`/`temple_shop` (UNSURE, not traced — out of M1 step-0 scope); one confirmed modal in the party-wipe branch of `AfterCombatExpAndTreasure`: `press_any_key`+`DisplayAndPause` (ovr006.cs:807-808) | Matches the API sketch's coarse `Request::Combat{..}` — the engine owns the loop, consistent with D-VM3's stated preference for coarse requests. |
| 0x25 | ON GOTO | `CMD_OnGotoGoSub` ovr003.cs:1032-1064 (branch `gbl.command==0x25`) | `vm_LoadCmdSets(2)` (1034) → `ecl_offset--` (1037) → `vm_LoadCmdSets(var_2)` (1038), `var_2` = decoded byte, **data-dependent 0-255** | 0 | **YES** | MACHINE | none | `ecl_offset` (jump target) | none | Declared size 0 but runtime consumes `2+var_2` operand slots — an IF-false landing here desyncs the pc by the whole real operand span. **Out-of-range selector, pinned**: `var_1` (the selector, `vm_GetCmdValue(1)` — itself possibly memory-resolved, not necessarily static) is compared `< var_2` (the tail-entry count) at `ovr003.cs:1038`; when `var_1 >= var_2` there is **no `else` jump** — the `if`'s only body is the jump, so an out-of-range selector is a **fall-through to the next instruction** (past the whole decoded tail), not a wedge or a jump to entry 0. Confirmed by direct read of `ovr003.cs:1038-1059`: the `else` branch (1055-1059) only logs, it does not touch `ecl_offset`. |
| 0x26 | ON GOSUB | `CMD_OnGotoGoSub` ovr003.cs:1032-1064 (branch `gbl.command==0x26`) | same as 0x25 | 0 | **YES** | MACHINE | none | `vmCallStack.Push(ecl_offset)` (1055) in addition to the jump | none | Same mismatch shape as 0x25, plus the unbounded call-stack push. **Same out-of-range fall-through as 0x25** — the push onto `vmCallStack` (1055) is inside the *in-range* branch only (pushed value is `gbl.ecl_offset` as it stands after the full `2+var_2`-operand decode, i.e. the fall-through address — the correct "return site" for a later RETURN), so an out-of-range GOSUB neither jumps nor pushes; it is indistinguishable from ON GOTO's out-of-range case. |
| 0x27 | TREASURE | `CMD_Treasure` (`load_item`) ovr003.cs:1068-1199 | 1×8 (fixed) | 8 | NO | SVC, MEM (reads only) | `load_decode_dax(...)` (seg042.cs:115, file-table path, `block_id<0x80`); `create_item(ItemType)` (ovr022.cs:443, random-roll path, `block_id 0x80-0xFE`); `roll_dice` (item-type table rolls); `ItemDisplayNameBuild(false,false,0,0,item)` (ovr025.cs:170 — `display_new_name=false`, builds the name string only, **no display call**) | `pooled_money` (7 coin slots), `items_pointer` (treasure-pool list) | **none found** | **Zero presentation points.** See §3 contradiction. |
| 0x28 | ROB | `CMD_Rob` ovr003.cs:1202-1224 | 1×3 (fixed) | 3 | NO | SVC, MEM (reads only) | `RobMoney(player, pct)` (ovr008.cs:1346, `player.Money.ScaleAll`); `RobItems(player, chance)` (ovr008.cs:1352, RNG-backed `roll_dice(100,1)` per item, weight-tiered) | `SelectedPlayer` or all of `TeamList` — money/items mutated in place | none | No memory writes at all; purely reads 3 operands then mutates character records. |
| 0x29 | ENCOUNTER MENU | `CMD_EncounterMenu` ovr003.cs:1227-1537 | 1×14, all decoded up front (`vm_LoadCmdSets(0x0e)`, 1251) — the `do…while` loop (1281-1532) consumes no further operand batches | 14 | NO | MEM (one write among many candidate sites, branch-dependent), SVC, EFF (**multiple**), REQ (**multiple**) | `calc_group_movement` (ovr008.cs:1370); `sub_304B4` (approach distance); `sub_30580` (sprite/pic per distance step); `sub_317AA` (button menu) | `area2_ptr.encounter_distance/max_encounter_distance` (loop-owned), `sprite_block_id`, `pic_block_id` | `displayInput` busy-wait (ovr027.cs:132-188+), invoked once **per loop iteration** via `sub_317AA` (1362) | **Confirms v3's "interactive loop" claim precisely**: 3 distinct EFF text classes (encounter description, "Both sides wait.", "The monsters flee.") + 1 REQ menu per iteration; exits via exactly one `vm_SetMemoryValue` write among ~14 candidate sites — matches "one memory write exits it" verbatim. |
| 0x2A | GETTABLE | `CMD_GetTable` ovr003.cs:635-648 | 1×3 (fixed) | 3 | NO | MEM | none | reads `mem[cmd_opps[1].Word + vm_GetCmdValue(2)]`, writes to `cmd_opps[3].Word` | none | Operand 1 is a **raw base address** (`.Word`, unresolved) added to a resolved index — a computed-address read that can hit any window despite the "table" name. |
| 0x2B | HORIZONTAL MENU | `CMD_HorizontalMenu` ovr003.cs:698-753 | `vm_LoadCmdSets(2)` (703) → `ecl_offset--` (708) → `vm_LoadCmdSets(string_count)` (710), **data-dependent** | 0 | **YES** | MACHINE (string regs), MEM (1 write), SVC (minor), REQ (1) | `sub_317AA` (748, single button-menu call); `ClearPromptAreaNoUpdate` (752) | `unk_1D972[1]` occasionally canonicalized to `"PRESS <ENTER>..."` (720, self-modifying string register); selection written to `cmd_opps[1].Word` via `vm_SetMemoryValue` (750) | `displayInput` inside `sub_317AA`, one call | Same variable-tail mismatch shape as ON GOTO/GOSUB (declared 0, consumes `2+string_count`); unlike ON GOTO/GOSUB, exactly one REQ point despite the tail (contrast ENCOUNTER MENU's loop). Second confirmed instance of the "both MENUs" hazard the design doc names explicitly. |
| 0x2C | PARLAY | `CMD_Parlay` (`talk_style`) ovr003.cs:1540-1557 | 1×6 (fixed) | 6 | NO | MEM (1 write), REQ (1) | `sub_317AA` (1550, fixed 5-option HAUGHTY/SLY/NICE/MEEK/ABUSIVE menu) | selected byte written to `cmd_opps[6].Word` via `vm_SetMemoryValue` (1556) | `displayInput` inside `sub_317AA`, one call | **Exactly one** interaction point, **zero** EFF/text output. See §3 contradiction. |
| 0x2D | CALL | `CMD_Call` ovr003.cs:1832-1910 | 1×1 (fixed) | 1 | NO | MACHINE (dispatch), SVC, EFF, REQ | Dispatches on `var_4 = cmd_opps[1].Word - 0x7fff` (raw Word) — a **hidden second syscall table, fully enumerated: exactly 7 cases**, no `default` (unrecognized keys are a silent no-op, ovr003.cs:1853-1909). See §3 for the full per-case breakdown with proposed `EngineServices` signatures. | `mapWallRoof/mapWallType`, `combat_type`, `TeamList`, `mapPosX/Y`, `positionChanged`, `byte_1D556` (sprite/frame state) | `GameDelay` (case `0xE804`) confirmed a **timed sleep, not input-wait** — `seg041.cs:335-339` `GameDelay()` → `seg049.SysDelay(gbl.game_speed_var*100)` (`Thread.Sleep`, seg049.cs:11-17), identical to DELAY (0x3A)'s own call. Resolves the prior UNSURE. | **Fully enumerated below (§3)** — no longer a docket item (item 9 resolved; one sub-case name remains UNSURE). |
| 0x2E | DAMAGE | `CMD_Damage` (`sub_28958`) ovr003.cs:1595-1704 | 1×5 (fixed) | 5 | NO | SVC (RNG, saves, char records), EFF (**multiple, data-dependent**), MACHINE | `roll_dice(size,count)` (1606, +re-rolled per target in the loop branch, 1678); `RollSavingThrow` (ovr024.cs:554); `CanHitTarget` (ovr024.cs:487); `sub_32200(player, damage)` (ovr008.cs:1401 → `damage_player`, HP/death mutation), called once per affected player, up to `var_1` times in the "roll per target" branch (1668-1679) | `party_killed`, `SelectedPlayer` (temporarily), player HP/health_status | **Multiple, data-dependent**: `sub_32200` fires a conditional `DisplayAndPause` whenever `textYCol>0x16` (ovr008.cs:1421) — can occur once per player hit; an unconditional `DisplayAndPause` always fires at the end (1703); a conditional party-wipe branch adds `press_any_key`+a timed `SysDelay(3000)` (1698-1699) | **Surprising finding**: DAMAGE is not named by v3 among "multiple presentation points" opcodes, but its per-target loop structurally matches that pattern (several Effects before completing) — candidate to add alongside ENCOUNTER MENU. See docket. |
| 0x2F | AND | `CMD_AndOr` ovr003.cs:607-632 (branch `gbl.command==0x2F`) | 1×3 (fixed) | 3 | NO | MEM, MACHINE | none | sets all 6 `compare_flags` via `compare_variables(resultant, 0)` (630, bitwise-AND result compared against literal 0); writes result via `vm_SetMemoryValue` (631) | none | Side-effects the full 6-flag compare-flags file (unlike COMPARE AND, which only ever sets flags[0]/[1]). |
| 0x30 | OR | `CMD_AndOr` ovr003.cs:607-632 (branch `gbl.command==0x30`) | same as 0x2F | 3 | NO | MEM, MACHINE | none | same as 0x2F, bitwise OR | none | Identical structure to AND. |
| 0x31 | SPRITE OFF | `CMD_SpriteOff` ovr003.cs:1707-1717 | 0 (direct pc++, 1709) | 0 | NO | MACHINE, EFF | `RedrawView()` (ovr029.cs:10-49, pure draw) | `displayPlayerSprite`, `spriteChanged`, `can_draw_bigpic` | none | Conditional on `displayPlayerSprite`. |
| 0x32 | FIND ITEM | `CMD_FindItem` ovr003.cs:1560-1585 | 1×1 (fixed) | 1 | NO | MACHINE, SVC | candidate `party_has_item(item_type) -> bool` (inline `TeamList`/`player.items` scan, no named coab fn) | `compare_flags[0..5]` | none | Early-return on first match — later players/items never scanned. |
| 0x33 | PRINT RETURN | `CMD_PrintReturn` ovr003.cs:1730-1738 | 0 (direct pc++, 1732) | 0 | NO | EFF (text-cursor bookkeeping, not itself a draw call) | none | `textXCol=1`, `textYCol++` | none | Consumed by the *next* PRINT, not itself an Effect. |
| 0x34 | ECL CLOCK | `CMD_EclClock` ovr003.cs:1720-1727 | **1 call** `vm_LoadCmdSets(2)` (1722) → 2 operands | **1** (`CommandTable` ovr003.cs:2115) | **YES — self-check confirmed** | SVC | `step_game_time(timeSlot, timeStep)` (ovr021.cs:150-172) | rest-time clock in `area_ptr` (`0x6A00+...` words), timed-affect expiry via `CheckAffectsTimingOut` | none | Declared skip=1 but the handler decodes 2 operands via one `vm_LoadCmdSets(2)` call — one batch, two operands (not two call sites). `Skip()` (size 1) would decode only 1, landing the pc mid-operand relative to actual execution. Matches v3 §1's headline claim exactly. |
| 0x35 | SAVE TABLE | `CMD_SaveTable` ovr003.cs:651-661 | 1×3 (fixed) | 3 | NO | MEM (operand-addressed word write) | `vm_SetMemoryValue(value,location)` | ScriptMemory cell at `cmd_opps[2].Word + vm_GetCmdValue(3)` | none | Table write, index computed at runtime. |
| 0x36 | ADD NPC | `CMD_AddNPC` ovr003.cs:1769-1782 | **1 call** `vm_LoadCmdSets(2)` (1771) → 2 operands | **1** (`CommandTable` ovr003.cs:2117) | **YES — self-check confirmed** | SVC, EFF | `load_npc(monster_id)` (ovr017.cs:878-890 → `load_mob`, `AssignPlayerIconId`, `chead_cbody_comspr_icon`); `reclac_player_values`; `PartySummary` (EFF) | `SelectedPlayer.control_morale`, party roster | none | Same shape as 0x34: one `vm_LoadCmdSets(2)` call, declared skip 1. Matches v3 §1's headline claim exactly. |
| 0x37 | LOAD PIECES | `CMD_LoadFiles` (shared with 0x21) ovr003.cs:501-604 | 1×3 (fixed) | 3 | NO | SVC, EFF, MACHINE | `Load3DMap`, `LoadWalldef`, `load_bigpic` | `byte_1AB0B/1AB0C`, `area_ptr.current_3DMap_block_id`, `setBlocks[]` | none | Branches on `gbl.command` (0x21 vs 0x37) inside the shared body. |
| 0x38 | PROGRAM | `CMD_Program` ovr003.cs:1929-1987 | 1×1 (fixed) — single `vm_LoadCmdSets(1)` (1931); no case re-decodes PROGRAM's own operands | 1 | **NO** — own accounting is clean; nested `RunEclVm` calls (case 9) decode the *nested script's* operands, not PROGRAM's | SVC, REQ, EFF, MACHINE | case 0: `startGameMenu()` (ovr018.cs:69-306, blocking menu loop); case 8: `end_game_text()` (ovr019.cs:474-538, blocking cutscene), `yes_no(prompt)` (ovr027.cs:676-689), `SaveGame()` (ovr017.cs:1109-1205, blocking + disk I/O), `print_and_exit()` (seg043.cs:9-23, terminal); case 9: `TryEncamp()` (ovr003.cs:1913-1926) → `MakeCamp()` (ovr016.cs:1080-1162, nested `RunEclVm`); cases 3/9 fall through to `CMD_Exit()` | `gameWon`, `party_killed`, `stopVM`, `vmCallStack`, `encounter_flags`, `SelectedPlayer` | ovr018.cs:134 (game-menu loop), ovr027.cs:676-689 (yes/no), ovr017.cs:1117 (save-slot pick), ovr016.cs:1103 (camp menu), ovr019.cs:479+ (end-game press-any-key) | **PROGRAM-9 termination confirmed exactly**: `TryEncamp()` runs unconditionally, then an **unconditional** `CMD_Exit()` follows (ovr003.cs:1975-1981) — no branch skips it; since `RunEclVm` reassigns `ecl_offset` on every entry, the pre-`TryEncamp` offset backup is dead code, exactly as v3 claims. |
| 0x39 | WHO | `CMD_Who` ovr003.cs:1757-1766 | 1×1 (fixed) | 1 | NO | MACHINE, REQ | `selectAPlayer(ref SelectedPlayer, false, prompt)` (ovr025.cs:1527-1567, blocks on `displayInput`) | `SelectedPlayer` | ovr025.cs:1539 | Reads `unk_1D972[1]` unconditionally as its prompt — a second, independent confirmation of the string-register staleness pattern (beyond VERTICAL MENU). |
| 0x3A | DELAY | `CMD_Delay` ovr003.cs:1588-1592 | 0 (direct pc++, 1590) | 0 | NO | MACHINE, REQ | `GameDelay()` (seg041.cs:335-339) → `SysDelay(game_speed_var*100)` | none | none (time-scale pause, not keyboard-blocking) | Matches the API sketch's `Request::Delay{ticks}`. |
| 0x3B | SPELL | `CMD_Spell` ovr003.cs:1785-1829 | 1×3 (fixed) | 3 | NO | SVC, MEM (2 writes) | candidate `find_spell_in_party(spell_id) -> (spell_index, player_index)` (inline, no named coab fn) | ScriptMemory cells `loc_a`,`loc_b` via `vm_SetMemoryValue` | none | Not-found path: `player_index--` from 0 wraps a `byte` to `0xFF`, deliberately paired with sentinel `spell_index=0x0FF` (1818-1822) — an intentional wraparound to replicate exactly, not a bug to "fix". |
| 0x3C | PROTECTION | `CMD_Protection` ovr003.cs:1990-2004 | 1×1 (fixed) — operand decoded but **never read** (1997) | 1 | NO | MACHINE, REQ, EFF | `copy_protection()` (ovr004.cs:16-111, ≤3 keyboard attempts, can call `print_and_exit()` on failure); `LoadPic()` (EFF) | `encounter_flags[0..1]`, `spriteChanged` | ovr004.cs input loop (~16-111) | Guarded by `Cheats.skip_copy_protection`. The decoded operand is dead code — docket candidate: vestigial parameter, or does a dialect variant read it? |
| 0x3D | CLEAR BOX | `CMD_ClearBox` ovr003.cs:1741-1754 | 0 (direct pc++, 1743) | 0 | NO | EFF | `draw8x8_03`, `PartySummary`, `display_map_position_time`, `DrawMaybeOverlayed` | `byte_1EE98=false` | none | Pure presentation bundle. |
| 0x3E | DUMP | `CMD_Dump` ovr003.cs:2007-2018 | 0 (direct pc++, 2009) | 0 | NO | SVC, MACHINE, EFF | `FreeCurrentPlayer(player,true,false)` (ovr018.cs:1580-1608) | `SelectedPlayer`, `LastSelectedPlayer`, `TeamList` | none | Removes the selected (usually NPC) player from the party. |
| 0x3F | FIND SPECIAL | `CMD_FindSpecial` ovr003.cs:2021-2039 | 1×1 (fixed) | 1 | NO | MACHINE, SVC | `Player.HasAffect(affect_type)` (Classes/Player.cs:835-838) | `compare_flags[0..5]` | none | Mirrors FIND ITEM's pattern exactly. |
| 0x40 | DESTROY ITEMS | `CMD_DestroyItems` ovr003.cs:2042-2055 | 1×1 (fixed) | 1 | NO | SVC | candidate `destroy_items_of_type(item_type)` (inline `TeamList`/`items.RemoveAll` loop + `reclac_player_values` per player) | `player.items` per party member | none | Pairs with FIND ITEM (0x32). |

## 2. Self-check

Per the task's required self-check, the skip≠run column must independently
rediscover:

- **The size-0 opcodes that still consume operands at runtime**: **VERTICAL
  MENU (0x15)** and **HORIZONTAL MENU (0x2B)** — both MENUs — plus **ON GOTO
  (0x25)** and **ON GOSUB (0x26)**. All four are marked **YES** above, each
  independently derived by the batch covering that opcode, before cross-check.
- **The fixed-arity mismatches**: **ECL CLOCK (0x34)** and **ADD NPC (0x36)**
  — both declare skip size 1 but consume 2 operands via a single
  `vm_LoadCmdSets(2)` call each. Both marked **YES** above, with call-site
  citations. This also sharpens v3's own wording: the mismatch is "one call,
  two operands," not two separate `vm_LoadCmdSets` call sites — worth a small
  wording tightening in the design doc's §1 (not a substantive change).

Self-check passes: all six known divergent opcodes were independently
rediscovered. No additional fixed-arity mismatches were found across the other
59 opcodes — every other handler's cumulative `vm_LoadCmdSets` consumption
matches its declared `CommandTable` size exactly.

## 3. Draft EngineServices surface

Deduped across all four opcodes ranges, applying D-VM4's placement rule
strictly: a call that returns a value or mutates game entities *synchronously*
is a service; anything that blocks on user input or paces itself against real
time belongs to the Request taxonomy instead (noted separately below where a
batch's raw classification blurred that line — PROGRAM's menu/save/camp calls,
WHO's player picker, and COPY PROTECTION's keyboard loop are all blocking and
therefore Requests, not services, even though the coab call sites look like
plain function calls).

### Character / party
- `retarget_selected_player(index: u8) -> Result<(), NotFound>` — LOAD CHARACTER (0x0A)
- `free_current_player(player, free_icon: bool, leave_party_size: bool) -> Player` — LOAD CHARACTER (0x0A, high-bit path), DUMP (0x3E)
- `party_strength() -> u8` — PARTYSTRENGTH (0x1D)
- `check_party(query: u16, affect: Affects) -> CheckPartyResult` — CHECKPARTY (0x1E) (query codes: affect-present, 8 thief-skill bands, movement; unrecognized codes are a no-op per the original — see docket)
- `party_has_item(item_type: u8) -> bool` — FIND ITEM (0x32)
- `find_special(affect_type: u8) -> bool` — FIND SPECIAL (0x3F) (thin wrapper over `Player::HasAffect`)
- `destroy_items(item_type: u8)` — DESTROY ITEMS (0x40)
- `rob_money(player, pct: u8)` / `rob_items(player, chance: u8)` — ROB (0x28)
- `party_surprise_check() -> (u8, u8)` — PARTY SURPRISE (0x22) (ranger-class detector)
- `who_prompt_target() -> PlayerId` — **REQ, not SVC**: WHO (0x39) blocks on `selectAPlayer`/`displayInput`; belongs to a `Request::SelectPlayer{prompt}` kind, not this trait.

### Monsters / NPCs / combat setup
- `load_monster(monster_id: u8) -> Result<MonsterHandle, MissingData>` — LOAD MONSTER (0x0B) (the missing-`.dax` path is a fatal `print_and_exit()` in the original with no graceful-degradation branch — design question for docket: model as `VmError` or treat as unreachable given shipped assets are always present)
- `setup_monster(dir, y, x) -> ApproachDistance` — SETUP MONSTER (0x0C) (wraps `sub_304B4`)
- `clear_monsters()` — CLEARMONSTERS (0x1C)
- `add_npc(monster_id: u8)` — ADD NPC (0x36) (wraps `load_npc`/`load_mob`)
- `setup_duel(is_duel: bool)` — CALL (0x2D) case 1/2 (clones selected player into a hostile NPC)
- `calc_group_movement() -> (min: u8, max: u8)` — ENCOUNTER MENU (0x29)
- `approach_distance(dir, y, x) -> u8` — APPROACH (0x0D), SETUP MONSTER (0x0C), ENCOUNTER MENU (0x29) (all wrap `sub_304B4`)
- `load_encounter_visual(flags, distance, pic_id, sprite_id)` — ENCOUNTER MENU (0x29), SETUP MONSTER (0x0C) (wraps `sub_30580`, non-blocking presentation dispatch — tag as EFF-adjacent, called synchronously)

### Items / treasure
- `create_item(item_type: ItemType) -> Item` — TREASURE (0x27) (random-roll path)
- `load_item_from_table(block_id: u8) -> Item` — TREASURE (0x27) (file-table path, wraps `load_decode_dax`)
- `find_spell_in_party(spell_id: u8) -> (spell_index: u8, player_index: u8)` — SPELL (0x3B) (not-found sentinel: both outputs `0xFF` via intentional byte-underflow — replicate exactly)

### Combat math (wraps `VmRng` per D9 — not raw RNG calls)
- `roll_dice(size: u8, count: u8) -> u16` — RANDOM (0x08, thin `Random(max)` variant), SURPRISE (0x23), TREASURE (0x27), ROB (0x28), DAMAGE (0x2E)
- `roll_saving_throw(bonus, save_type, player) -> bool` — DAMAGE (0x2E)
- `can_hit_target(bonus, target) -> bool` — DAMAGE (0x2E)
- `apply_damage(player, damage: u16)` — DAMAGE (0x2E) (wraps `sub_32200`/`damage_player`; internally triggers a conditional pagination Effect per hit — see docket)

### World / map / files
- `load_3d_map(block_id: u8)`, `load_walldef(set: u8, id: u8)`, `load_bigpic(id: u8)` — LOAD FILES/LOAD PIECES (0x21/0x37), COMBAT (0x24, post-fight)
- `step_game_time(time_slot: u8, amount: u8)` — ECL CLOCK (0x34)
- `move_position_forward()` — CALL (0x2D) case 0x401F (writes `mapPosX/Y`/`positionChanged` directly)
- map-geometry query (`wall_type(dir, y, x) -> WallType`) — CALL (0x2D) case 0xAE11 (name UNSURE — coab's `get_wall_x2`/`getMap_wall_type` split not fully resolved)

### PROGRAM / meta (0x38) — mixed bag, several belong to Requests not services
- `try_encamp() -> bool` — wraps `TryEncamp`/`MakeCamp`; itself launches a nested `RunEclVm` (machine-level, not a simple service — see v3 §1's nested-run semantics)
- **REQ, not SVC**: `start_game_menu()`, `end_game_text()`, `yes_no(prompt) -> bool`, `save_game()`, `print_and_exit()` (terminal) — all block on keyboard input inside coab; PROGRAM needs `Request` kinds for the game menu, end-game sequence, yes/no confirm, and save-slot picker rather than folding these into EngineServices.
- **REQ, not SVC**: `copy_protection()` — PROTECTION (0x3C) blocks on up to 3 keyboard attempts.

### CALL (0x2D) — full case enumeration (ride-along, resolves docket item 9)

`CMD_Call` (`ovr003.cs:1832-1910`) decodes one fixed operand, computes
`var_4 = cmd_opps[1].Word - 0x7fff` (unsigned 16-bit wraparound — the raw
`.Word`, so the case values below are the *post-subtraction* keys the switch
actually matches, not the literal operand bytes a script encodes), and
switches on it. The switch has **no `default` arm** — an operand value that
maps to none of the 7 keys below is a silent no-op (matches the table's
"unrecognized dispatch code = drop" pattern seen elsewhere, e.g. CHECKPARTY's
query-code dispatch, docket item 7). All 7 cases were traced to their full
call depth within this pass; one leaf name (the wall-query split) remains
genuinely ambiguous and is called out below.

| key | behavior | coab evidence | proposed channel | proposed `EngineServices` signature |
|---|---|---|---|---|
| `0xAE11` | Reads the roof/wall-x2 value at the party's current cell; if a bundle of redraw-dirty flags is set (`byte_1AB0B` AND any of `spriteChanged`/`displayPlayerSprite`/`byte_1EE91`/`positionChanged`/`byte_1EE94`), triggers a full view redraw + map-position-time display, clears those flags, then re-reads the wall type for the facing direction. | `ovr003.cs:1853-1875`; `get_wall_x2` (`ovr031.cs:289-315`, `mi.x2` off `gbl.geo_ptr.maps[y,x]`, coordinate-clamping — not a `MapCoordIsValid` bounds error); `getMap_wall_type` (`ovr031.cs:222-249`, direction-keyed field off the same `MapInfo`); `RedrawView`/`display_map_position_time` (non-blocking, already-classified EFF calls) | SVC (both wall reads) + EFF (conditional redraw) | `wall_roof(map_y: u8, map_x: u8) -> u8` (wraps `get_wall_x2`); `wall_type(direction: u8, map_y: u8, map_x: u8) -> u8` (wraps `getMap_wall_type`) — **both names now resolved**: `get_wall_x2` returns `MapInfo.x2` (a distinct roof/overhead field), `getMap_wall_type` returns one of 4 direction-keyed wall-type fields; they are not aliases of one query, so two methods, not one. |
| `1` | Enters duel combat mode against the currently-selected player, cloned into a hostile NPC. | `SetupDuel(true)` (`ovr008.cs:1305-1338`): sets `combat_type=duel`, `area2_ptr.isDuel=true`, clones `SelectedPlayer` into `DuelMaster` (name "ROLF", `combat_team=Enemy`, `control_morale=NPC_Berzerk`, cloned items), appends to `TeamList`, loads a duel portrait (`chead_cbody_comspr_icon`) | SVC | `setup_duel(is_duel: bool)` — confirmed, same method as case `2` below. |
| `2` | Same as case `1` but `isDuel=false` — sets duel combat mode without a cloned NPC opponent (also flags all non-dueler party members `in_combat=false`). | `SetupDuel(false)` (`ovr008.cs:1305-1338`, the `if (isDuel)` NPC-clone block is skipped) | SVC | `setup_duel(is_duel: bool)` — same method, `false` arg. |
| `0x3201` | Plays one of two sound effects chosen by an engine-internal state word (not a script operand): `word_1EE76==8` → `sound_a`, `==10` → `sound_b`, else `sound_a`. | `ovr003.cs:1877-1889`; `seg044.PlaySound(Sound)` (`seg044.cs:22`) | SVC (variant selection reads engine state not exposed via ScriptMemory) + EFF (the sound itself is buffered presentation, like Picture/Sprite) | `call_sound_variant() -> SoundId` (SVC, reads `word_1EE76`) feeding an `Effect::Sound(SoundId)` — kept as two steps because the *selection* depends on engine state the VM doesn't own, but the *playback* is ordinary buffered presentation. |
| `0x401F` | Advances the party one cell in the current facing direction (wrapping map coords), re-reads wall-roof/wall-type for the new cell, flags position changed. | `MovePositionForward` (`ovr008.cs:1256-1277`) — writes `mapPosX/Y`/`mapWallRoof`/`mapWallType`/`positionChanged` directly, **bypassing ScriptMemory addressing entirely** (docket item 3's write-destination pattern, but for engine globals rather than an operand-addressed cell) | SVC | `move_position_forward()` — confirmed, no return value (all effects are engine-state writes). |
| `0x4019` | If the party is *not* currently in a dungeon (`area_ptr.inDungeon==0`), re-reads the wall type for the current facing/cell; otherwise a no-op. | `ovr003.cs:1891-1897` | SVC | Reuses `wall_type(direction, map_y, map_x) -> u8` from case `0xAE11` — same underlying query, gated by an `inDungeon` check the caller (not the service) should perform, since `inDungeon` is ordinary Area-window state the VM/engine already exposes. |
| `0xE804` | Draws the current frame of a running sprite/picture animation (`byte_1D556`), advances to the next frame, then pauses for the game's time-scaled delay. | `ovr003.cs:1899-1906`; `DrawMaybeOverlayed` (`ovr030.cs:13+`, non-blocking draw); `DaxArray.NextFrame()` (frame-advance, machine-internal to the animation object); `GameDelay` (`seg041.cs:335-339` → `seg049.SysDelay`, confirmed timed sleep — see the 0x2D table row) | EFF (frame draw + advance) + REQ (the delay — same `Request::Delay{ticks}` kind as DELAY 0x3A, not a keyboard-blocking modal) | No new service: `Effect::Picture`/`Effect::Sprite`-shaped output for the frame draw, then the same `Request::Delay` DELAY (0x3A) already needs. |
| *(any other key)* | Silent no-op — no `default`/`else` arm in the switch. | `ovr003.cs:1852-1909` (switch has no trailing `default:`) | — | Not modeled as an error; matches the original's silent-drop behavior. Census question: do shipped scripts ever emit an unrecognized key (deliberately or via a corrupted/self-modified operand)? |

Remaining UNSURE (not resolvable by further reading, timeboxed): whether
`get_wall_x2`/`getMap_wall_type` correspond to named fields the original
disassembly calls something more specific than "x2" — `MapInfo.x2`'s exact
semantic (vs. its four `wall_type_dir_N` siblings) isn't documented anywhere
in the read files; `wall_roof` above is our own descriptive name, not a
coab-derived one.

## 4. Contradictions with v3

**PARLAY and TREASURE do not have "multiple presentation points."** v3 §1
(D-VM3) claims: *"Instructions may legitimately yield several Effects/Requests
before completing (ENCOUNTER MENU is an interactive loop; PARLAY/TREASURE have
multiple presentation points)."*

- **ENCOUNTER MENU (0x29): confirmed precisely** — see the table row above.
- **PARLAY (0x2C): contradicted.** `CMD_Parlay` (ovr003.cs:1540-1557) contains
  exactly **one** `sub_317AA` menu call and **zero** `press_any_key`/
  `DisplayAndPause` calls anywhere in the handler body. One interaction point,
  not multiple.
- **TREASURE (0x27): contradicted.** `CMD_Treasure` (ovr003.cs:1068-1199)
  contains **zero** presentation calls of any kind. Its one call into
  presentation-adjacent code, `ItemDisplayNameBuild(false, false, 0, 0, item)`
  (ovr025.cs:170), is invoked with `display_new_name=false`, which explicitly
  skips the internal `displayString` call — it only builds a name string in
  memory. TREASURE is pure SVC+MEM(reads); no EFF/REQ at all.

This doesn't change the `Pending`-carries-per-opcode-phase design (D-VM3) —
that's still correct and necessary for ENCOUNTER MENU and DAMAGE (see below).
It does mean PARLAY and TREASURE were miscited as examples. Suggested doc
fix (human review, not applied here): replace the parenthetical with
*"ENCOUNTER MENU is an interactive loop (3 Effect classes + 1 Request per
iteration); DAMAGE's per-target loop shows the same shape; PARLAY and
TREASURE, despite `>1`-word fixed operand sizes, are single-shot (PARLAY: one
Request, no Effects; TREASURE: no Request or Effect at all)."*

No other contradictions were found across the other 63 opcodes; several v3
claims were independently re-confirmed with exact line citations (PRINT/
PRINTCLEAR pagination, NEWECL's block-switch reset, the VERTICAL MENU
variable-tail shape, PROGRAM-9's termination-not-resumption, the 0x34/0x36
fixed-arity mismatch).

## 5. New docket candidates

1. **0x1F is a confirmed dead opcode** in coab (null delegate, `ovr003.cs:2094`)
   — not resolvable by further reading. Census question: do shipped scripts
   ever emit it (even skipped-over)?
2. **DIVIDE's remainder may be Party-window-addressable at `0x7F3F`.** The
   remainder write bypasses `vm_SetMemoryValue` (direct field write to
   `area2_ptr.field_67E`, ovr003.cs:113), but `Area2.field_800_Get`'s offset
   arithmetic implies VM address `0x7F3F` reads it back through the ordinary
   Party window. Derived from arithmetic, not a traced live example — confirm
   with a census/golden test before relying on it as a named global.
   **CONFIRMED by a live example (2026-07-12, found by the human playtest via
   the inspector's halt records + disassembly pane):** Tilverton's per-step
   script (`ECL2.DAX` block 1) executes
   `0x8295: DIVIDE mem=0x7F7B, imm=0x08 -> mem=0x7F80` immediately followed by
   `0x829E: GETTABLE base=0x9DB8 index=mem[0x7F3F] -> 0x7E7A` — a modulo-8
   table lookup whose index is the out-of-band remainder. Shipped content
   depends on the alias; DIVIDE's implementation must write the remainder
   through the facade at `0x7F3F` or this GETTABLE silently reads garbage.
3. **Destination/target operands never trigger a ScriptMemory read regardless
   of encoded mode.** GOTO, GOSUB, and every `location`/`loc` write-destination
   operand across the table (ADD/SUB/DIV/MUL, RANDOM, SAVE, INPUT NUMBER/
   STRING, etc.) use `cmd_opps[n].Word` directly, never `.GetCmdValue()`. This
   sharpens existing docket item 3 (0x01 vs 0x03 cosmetic-only) with concrete
   evidence: for *write-destination* operands, the mode's general "address;
   value read through ScriptMemory" meaning doesn't apply at all.
4. **`load_mob`'s missing-`.dax` path is a hard `print_and_exit()`**
   (ovr017.cs:836-838, reachable from LOAD MONSTER 0x0B) — an original-engine
   fatal/debug-exit, not a normal `Done`/`Exit`. Design question: model as
   `VmError`, or treat as unreachable given shipped assets are always present?
5. **Type-mismatch hazard on COMPARE AND (0x14) and CHECKPARTY (0x1E)**: both
   call `vm_GetCmdValue` on an operand with no `Code<0x80` guard; per
   `Opperation.GetCmdValue()`, a `code==0x80` (inline string) operand never has
   `highSet`, so feeding either opcode a string-mode operand throws in the
   original. Different in kind from the documented 0x34/0x36 *count* mismatch
   — this is a *type* mismatch. Census question: do shipped scripts ever do
   this?
6. **CMD_CompareAnd only ever sets `compare_flags[0]`/`[1]`** (`==`/`!=`),
   never `[2..5]` (`<,>,<=,>=`) — a cross-opcode contract invisible from any
   single opcode's row: IF opcodes 0x18-0x1B are only meaningfully paired with
   COMPARE/AND/OR, not COMPARE AND.
7. **CHECKPARTY's query-code dispatch is a partial function** — `var_2` values
   outside `{8001, 0xA5-0xAC, 0x9F}` silently no-op (no `else`, no error).
   Intentional "unhandled query = leave memory untouched," or an original bug?
   Census/docket question.
8. **SURPRISE (0x23) writes a hard-coded literal address `0x2CB`**, not an
   operand-supplied one — a concrete named-global candidate for the address
   map (currently the Global window table only lists `mapPosX/Y`, facing,
   wall info).
9. **RESOLVED.** CALL (0x2D)'s hidden second dispatch table (keyed on
   `operand.Word - 0x7FFF`) is fully enumerated: exactly 7 cases, no
   `default` arm (unrecognized keys silently no-op) — see §3's per-case
   table. One naming detail remains genuinely UNSURE: whether
   `get_wall_x2`/`getMap_wall_type` have more specific original names than
   the descriptive ones used here (`wall_roof`/`wall_type`) — not resolvable
   without more of the original disassembly's symbol table than we have.
10. **COMBAT (0x24)'s deep call chain was not traced** (`MainCombatLoop`,
    `CityShop`, `temple_shop` are each large subsystems in their own files) —
    correctly out of scope for M1 step 0, but flagged so M4's combat work
    knows this opcode's classification is a coarse `Request::Combat{..}` stub,
    not a full behavioral spec.
11. **DAMAGE's internal pagination trigger is data-dependent and stateful**
    (depends on `gbl.textYCol` carried over from whatever was drawn
    immediately before DAMAGE ran) — the number of Effect-equivalent pauses
    per execution isn't statically determinable from the instruction alone. A
    genuine census/conformance-test hazard, structurally the same
    "several Effects before completing" pattern v3 calls out for ENCOUNTER
    MENU (see §4).
12. **GETTABLE (0x2A)'s effective address is computed** (`base+index`), so
    despite its name it isn't confined to the Table window — can address any
    window depending on script-supplied values.
13. **PROTECTION (0x3C) decodes an operand it never reads** (ovr003.cs:1997) —
    vestigial, or read by a dialect variant we haven't seen?
14. **SPELL (0x3B)'s not-found sentinel is a deliberate byte underflow**
    (`player_index=0; player_index--` → `0xFF`, paired with `spell_index=
    0x0FF`) — must be replicated exactly, not "fixed."
15. **String-register staleness, second confirmed instance**: WHO (0x39)
    reads `unk_1D972[1]` unconditionally as its prompt, exactly like VERTICAL
    MENU's documented header-text staleness (v3 §1) — independent evidence
    for the same behavior.

## 6. Provenance

New coab files read for this classification, beyond the two rows already in
`SOURCES.md` (`engine/ovr003.cs`, `engine/ovr008.cs`, `engine/seg041.cs`,
`Classes/Opperation.cs`, `Classes/EclBlock.cs`, `Classes/Gbl.cs`): see the
`SOURCES.md` row added alongside this document.
