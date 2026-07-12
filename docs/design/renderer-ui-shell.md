# Design: Renderer & UI Shell (the engine loop, the screen, and the tick core)

> M2 architecture pass per PLAN.md §9 operating rule 3 (one design review before
> each one-way door). Status: **v2, draft for review.** v1 was written
> 2026-07-12 from a read of coab's presentation and walk-loop internals
> (read-for-behavior per D11 — no code copied; see SOURCES.md), cross-checked
> against ssi-engine's Java renderer and Gold Box Explorer's walldef/image
> plugins, then subjected the same day to one bounded adversarial review round
> (two independent reviewers, fresh context: one attacking D8/state-machine
> soundness and the VM-contract inheritance, one attacking renderer/format
> feasibility against coab). Every folded finding was re-verified against coab
> directly before editing. v2's changes: chain checkpoints gained
> resume-after-chain semantics (the boot and Look sites resume their flow, they
> don't abandon it); the input model adopted the original's drain-to-last read
> semantics (docketed for DOSBox confirmation); `party_killed` and the
> persistent `chained` flag became explicit engine state with transcribed
> guard/commit sites; per-char pacing became a fractional accumulator; script
> menus gained their party-scroll-while-parked and valid-keys-re-prompt
> behaviors; the presentation queue gained explicit press-any-key gate items
> (DAMAGE's terminal pause); pixel-transform corrections (XOR-delta scope,
> mask-13 on all symbol sets, sprite recolor, mutable no-draw state, J-filler
> semantics, sun-window hours). Review stops here per the one-round bound;
> remaining risk is carried by M2's goldens and the DOSBox captures. The tick
> contract (D-UI1) is a named one-way door: this doc goes to Fable + human
> review before any implementation session is prompted (PLAN §9 rule 3; M2
> milestone note).
>
> Scope: the `gbx-engine` crate's UI shell — the `tick(input) → frame` core
> (D8), the engine-loop state machine that orchestrates the VM per
> [vm-scriptmemory.md](vm-scriptmemory.md) D-VM3's engine obligations, the
> faithful 320×200 renderer (D4/D5), the asset-format inventory `gbx-formats`
> needs for M2, and the frontend presentation contract (desktop + web). Out of
> scope: combat UI (M4), spell/character screens (M3+), audio synthesis (M8),
> QoL overlays (M8), save format (M3) — see §3 Non-goals.

## 1. The original presentation machine, as verified in coab

Everything below was read directly from coab (CotAB dialect). File references
are to `~/src/goldbox-refs/coab/`. Cross-checks against ssi-engine (Java,
GPL-3) and Gold Box Explorer (GBE) are called out inline; contradictions are
collected in §1.11 and flagged, not absorbed.

### 1.1 Display model

The screen is a **320×200 buffer of 4-bit palette indices** over a 16-entry
EGA palette (`Classes/Display.cs:25-26` — the canonical EGA RGB triples, e.g.
color 10 = `{82,255,82}`). Palette slots are remappable at runtime:
`SetEgaPalette(index, colour)` (`Display.cs:82-103`) repoints a slot at a
different EGA color and the whole screen re-presents — palette effects without
touching pixels. Composited assets use **16 as the transparency code**
(`DaxBlock.SetMaskedColor`, `Classes/DaxFiles/DaxBlock.cs:149-159`), enforced
by `Display.SetPixel3`'s `value < 16` guard (`Display.cs:163-174`). The
clipped blit additionally carries **mutable draw state**: a no-draw color
(default 17) and a recolor pair, set/restored around specific draws
(`draw_clipped_nodraw`/`draw_clipped_recolor`, `seg040.cs:58-71,93-104`) —
e.g. the area-map party arrow masks color 8 (§1.7). These are blit
*parameters* in our renderer, not global state. All UI
drawing is aligned to a **40×25 grid of 8×8 cells**; free-pixel addressing
exists only inside `DrawColorBlock`/picture blits. Batching: the original
brackets multi-part draws in `UpdateStop()`/`UpdateStart()` so partial
composites never present (`Display.cs:112-136`) — in a tick model this is free
(a frame presents only at tick end).

### 1.2 Screen geometry (the 3D-view layout)

The standard exploration screen (`seg037.draw8x8_03`, `seg037.cs:73-102`) is
composed from 8×8 border symbols:

```
cell cols → 0        16                 39
row 0      ┌─────────────────┬──────────┐
           │  (2-14: inner   │ party    │   party header row 2:
           │   frame)        │ panel    │     "Name" @ col 17, "AC  HP" @ col 33
           │   3D viewport   │ rows 2+  │   player rows from row 4
           │   cells 3-13    │          │
           │   (px 24-111²)  ├──────────┤
           │                 │ row 15: position/time line
row 16     ├─────────────────┴──────────┤
row 17-22  │  text window, cols 1-38    │
row 23     └────────────────────────────┘
row 24       prompt/menu line, cols 0-39
```

- Outer border: row 0, row 23 (`0x17`), cols 0 and 39 (`DrawFrame_Outer`,
  `seg037.cs:31-54`); border symbol sequences are fixed engine tables
  (`outer_frame_top/bottom/left/right`, `seg037.cs:7-27`), drawn from 8×8
  symbol set 4 (ids `0x11E`+).
- Horizontal divider at row 16, vertical divider at col 16 (rows 0–16), inner
  viewport frame at rows/cols 2–14 (ids `0x114`+) — `draw8x8_03`.
- **3D viewport: an 11×11-cell square at cells (3,3)–(13,13), pixels
  24–111.** Established three ways: `draw_3D_8x8_titles` guards `rowY/colX ∈
  0..=10` then draws at `+2` (`ovr031.cs:145-171`), and the overlay path adds
  one more cell (`Put8x8Symbol(overlay=true)` → `OverlayUnbounded` →
  `draw_combat_picture(rowY+1, colX+1)`, `ovr038.cs:58-60` +
  `seg040.cs:23-32`); the background fills start at pixel (24,24)
  (`Draw3dWorldBackground`, §1.7); and ssi-engine clips the same region
  (`DungeonRenderer.java:106`, `zoom8(3)..+zoom8(11)`). The blit clip for
  overlay draws is pixels 8–175 (`draw_combat_picture`, `seg040.cs:115-118`).
- Text regions (`seg041.bounds`, `seg041.cs:119-123`): `NormalBottom` = rows
  17–22 × cols 1–38 (the exploration text window); `Normal2` = rows 21–22
  (two-line variant); `CombatSummary` = rows 1–21 × cols 23–38 (M4).
- Prompt/menu line: row 24, cols 0–39 (`ClearPromptAreaNoUpdate`,
  `ovr027.cs:351-354`).
- Party panel (§1.9): cols 17–38; status line at row 15, cols 17–38.

Other frame layouts exist for other modes — `DrawFrame_WildernessMap`
(divider at row 16, no vertical divider — bigpic viewport), `draw8x8_02`
(dividers at rows 3 and 8), `DrawFrame_Combat` (`seg037.cs:105-193`) — M2
implements `draw8x8_03` + `DrawFrame_Outer`; the rest are stubs with named
seams.

### 1.3 8×8 symbol sets and boot assets

`Put8x8Symbol(set…, symbol_id, row, col)` routes a symbol id to one of five
resident symbol sets by id range (`ovr038.cs:25-72`): set 0 = ids
`0x01–0x2D`, set 1 = `0x2E–0x73`, set 2 = `0x74–0xB9`, set 3 = `0xBA–0xFF`,
set 4 = `0x100–0x127`; per-set base offsets `symbol_set_fix = {0x01, 0x2E,
0x74, 0xBA, 0x100}` (`Gbl.cs:425`). Loaded at boot (`seg001.cs:305-321`):
the **mono font** (`8X8D1.DAX` block 201 → 177 glyphs × 8 bytes, 1bpp,
`seg041.Load8x8Tiles`, indexed `toupper(ch) % 0x40` — `display_char01`,
`seg041.cs:44-61`), **set 4** from `8X8D1.DAX` block 0xCA (frame/area-map
symbols), **set 0** from block 0xCB (universal tiles — GBE independently
documents block 203 = 0xCB as "universal", `DaxWallDefFile.cs:198-202`), and
the three **SKY** blocks 250/251/252 (moon, sun, horizon backdrop; mask
color 13). Sets 1–3 are loaded per-area by LOAD PIECES from the walldef's
paired 8×8 blocks — block id `walldef_id` when the walldef holds one wallset,
`walldef_id*10 + n` (n = 1..3) when it holds several (`LoadWalldef`,
`ovr031.cs:642-687`; GBE agrees except for a `block_id == 0 → base 100`
special case coab lacks — §1.11 item 8; walldef block 0 is a live path via
LOAD FILES' `0x7F` argument, `ovr003.cs:539-541`).

Three contracts the v1 draft under-specified, all load-bearing for pixel
goldens:

- **Every 8×8 symbol set is loaded color-13-masked** — `Load8x8D` calls
  `LoadDax(13, 1, …)` (`ovr038.cs:13`), so color 13 in wall, frame, and
  area-map tiles decodes to transparency-16 and is skipped at blit time.
  Per-pixel transparency *inside* wall pieces comes from this, independent
  of the symbol-0 skip.
- **`Put8x8Symbol` treats id 0 and `0x128..0x7FFF` as a hard error**
  (`ovr038.cs:49-51`); the "symbol 0 = draw nothing" skip lives at the wall
  drawer's call site (`draw_3D_8x8_titles`'s `symbolId > 0` guard,
  `ovr031.cs:161`). Our primitive keeps the loud error so id-arithmetic bugs
  can't hide behind a silent skip.
- **The ≥0x2D id rebase is computed once per load call from the *base* set**
  (`var_A = symbol_set_fix[symbolSet] - symbol_set_fix[1]`, `ovr031.cs:658`)
  and applied uniformly to every wallset block in the load (`:671`) — a
  multi-set walldef loaded at base set 1 is rebased by zero everywhere, i.e.
  its later wallsets' tile ids are already absolute in the data.
  `setBlocks[0..2]` records `(blockId, setId)` per set — only the base entry
  on a multi-block load (`ovr031.cs:684-685`) — is reset by LOAD PIECES'
  `0xFF` arguments, and is persisted by the original's saves to re-run
  `LoadWalldef` on restore (`ovr017.cs:1078-1087`) — named engine state, M3
  will serialize it.

Boot also loads 13 `COMSPR` combat-icon blocks and the `ITEMS` table
(`seg001.cs:312-323`) — combat/M4 and inventory/M3 surfaces; M2's boot
declares them stubbed (not loaded), noted here so the boot transcription
isn't mistaken for complete.

### 1.4 Text system

- **Glyph draw**: mono 8×8 glyphs with explicit bg/fg palette colors
  (`display_char01`). PRINT text is color 10; the pagination prompt is 13;
  status text varies per call site.
- **Word wrap** (`press_any_key`, `seg041.cs:134-231`): tokens are maximal
  runs bounded by the punctuation set `!,-.:;?` and spaces
  (`seg041.cs:132`); a token that would overflow `xEnd` wraps (with a
  drop-one-trailing-space special case at exactly-fits, `:193-198`); leading
  spaces are skipped after a wrap (`text_skip_space`).
- **Pagination**: wrapping past `yEnd` with text remaining resets the cursor
  to the region start and issues a modal `DisplayAndPause("Press any key to
  continue", 13)` on the prompt line, drains any keys typed *behind* the
  gating keypress (`clear_keyboard` immediately after, `seg041.cs:210-211`),
  clears the region, and continues (`seg041.cs:204-216`). This is D-VM3's
  engine-side gate; the keypress is an input-trace event (H5).
- **Pacing**: `displayStringSlow` sleeps `game_speed_var * 3` ms per
  character when `DelayBetweenCharacters` is set (`seg041.cs:90-107`);
  PRINT/PRINTCLEAR set it for exactly their own duration
  (`CMD_Print`, `ovr003.cs:397/416`); ENCOUNTER MENU holds it across its loop
  (`ovr003.cs:1247/1535`). `game_speed_var` defaults to 4 (`seg001.cs:274`)
  and is a save-file setting (`ovr017.cs:1034`) — 12 ms/char at default.
  `GameDelay()` = `game_speed_var * 100` ms (`seg041.cs:335-339`) — the DELAY
  opcode and CALL-0xE804 pause.
- **Cursor persistence**: `textXCol/textYCol` are globals that persist across
  instructions and even across scripts (PRINT RETURN advances them,
  `ovr003.cs:1730-1738`; DAMAGE's pagination depends on the carried-over
  `textYCol` — opcode-classification.md item 11). PRINTCLEAR resets the
  cursor to (row 17, col 1) and clears the window first (`ovr003.cs:404-414`);
  plain PRINT appends at the cursor. `press_any_key` also snaps an
  out-of-region cursor to the region start (`seg041.cs:143-150`).
- **`bottomTextHasBeenCleared`**: PRINT marks the window dirty
  (`ovr003.cs:396`); the world menu clears the text window before its next
  prompt if the flag is unset (`main_3d_world_menu`, `ovr015.cs:457-462`) —
  event text stays visible until the player's next command.

### 1.5 Prompt-line input (`displayInput` and friends)

All original interaction funnels through a few prompt-line widgets, each a
blocking loop in the original and a parked state for us:

- **Hotbar** (`ovr027.displayInput`, `ovr027.cs:132-341`): draws an optional
  leading prompt (`colors.prompt`) then the menu text with per-word
  highlighting; words are maximal runs of `[0-9A-Z]` (`BuildInputKeys`,
  `:59-86`). Input semantics: a letter matching any word's first character
  selects that word and returns it (uppercased); `,`/`.` cycle the
  highlighted word; Enter returns the highlighted word's first char (or
  `\r` when nothing is highlightable); Esc returns `'\0'`; Space returns
  Space (menu exit in list contexts). When `accept_ctrlkeys` is set,
  extended keys (arrows/keypad — a 0-prefixed second byte from
  `GetInputKey`) and digits map through `keypad_ctrl_codes = {'O','P','Q',
  'K',' ','M','G','H','I'}` (keypad 1–9; `ovr027.cs:124,297-311`) and return
  with `specialKeyPressed` — movement keys. Two time-based behaviors run
  *inside* the wait loop: an optional **timeout** (`displayInputSecondsToWait`
  → returns `displayInputTimeoutValue`, `:201-206`), and the **running
  animation** — if a multi-frame picture is active, frames advance by their
  per-frame delay ×100 ms while waiting (`:185-199`); the wilderness map
  cursor blinks 300/500 ms (`:151-153,176-183`, M6+).
- **List menu** (`sl_select_item`, `ovr027.cs:532-673`): a vertical list
  (highlight, headings skipped, in-page and page scrolling) combined with a
  Hotbar whose text grows ` Next`/` Prev`/` Exit` as applicable. Scroll keys,
  per coab: Home/End ctrl-codes `'G'`/`'O'` move the highlight, PgUp/PgDn
  `'I'`/`'Q'` page, and the plain letters `'P'`/`'N'` also page
  (`ovr027.cs:617-653`) — **Up/Down arrows (`'H'`/`'P'` ctrl-codes) are
  ignored by the special-key switch entirely**, which contradicts common
  memory of the game; docketed (§4 item 9) for a DOSBox check before the key
  map is pinned. Esc/`'E'`/`'\0'` exit with no selection; anything else
  returns (selected item, key). VERTICAL MENU and every roster/shop screen
  build on it — and VERTICAL MENU's list geometry is *coupled to the text
  cursor*: the list starts at `textYCol + 1` after the header text has
  rendered (and possibly paginated) through `press_any_key`
  (`ovr003.cs:682-689`), so the widget's region is a function of text-system
  state at open time.
- **Text entry** (`getUserInputString`, `seg041.cs:234-273`): echo at the
  prompt line from the prompt's end, printable chars `0x20–0x7A` up to a max
  length, Backspace edits, CR or Esc ends, result uppercased. **Numeric
  entry** (`getUserInputShort`, `:276-294`) re-runs the string editor until
  the input parses as `0..=65535` — INPUT NUMBER's validation loop.
- **Press-any-key** (`DisplayAndPause`, `seg041.cs:297-303`): prompt text +
  one key.
- **Yes/No** (`ovr027.yes_no`, `:676-689`): a Hotbar restricted to
  `"Yes No"`.
- Keyboard: extended keys arrive as `0x00` + scancode-byte. **`GetInputKey`
  is not a plain queue pop**: after reading any nonzero key it drains the
  entire buffer, keeping the *newest* key (`seg043.cs:55-62`) — so mashing
  forward five times during a slow redraw yields **one** step, and type-ahead
  is largely discarded (the `0x00` extended prefix skips the drain; the
  scancode byte read then drains). `clear_keyboard` (`seg043.cs:88-94`) is
  an explicit full drain layered on top, called after asset loads and after
  the pagination keypress. Whether drain-to-last is the original binary's
  behavior or a coab transliteration artifact is docketed (§4 item 8) with a
  DOSBox type-ahead test; we ship coab's semantics until that settles.
- Script menus that route through `sub_317AA` (HORIZONTAL MENU,
  `ovr003.cs:748`; ENCOUNTER MENU, `:1362`; PARLAY, `:1550`) are **not inert
  while parked**: the loop consumes extended keys as
  `scroll_team_list` + party-panel redraws — *mutating the selected player
  while the VM Request is suspended*, which retargets Party-window reads
  after resume — and re-prompts on any key outside its valid set instead of
  returning (`ovr008.cs:1176-1190`); Esc does not exit these menus.

### 1.6 The walk loop, transcribed

The M2 engine loop is the original's exploration control flow made explicit.
Three routines own it (all `ovr003.cs`/`ovr015.cs`):

**The world menu** (`main_3d_world_menu`, `ovr015.cs:348-465`): one Hotbar —
`"Area Cast View Encamp Search Look"` with `accept_ctrlkeys=1`. Dispatch:

| key | action | stays in menu loop? |
|---|---|---|
| `A` | toggle area-map view (if `block_area_view == 0`), redraw viewport; else `DisplayStatusText("Not Here")` — a draw + `GameDelay()` (24 ticks) + clear, i.e. a **timed wait inside the menu** (`ovr015.cs:378`, `seg041.cs:323-332`) | yes |
| `C` | cast-spell UI (M3) | yes |
| `V` | view-character UI (M3) | yes |
| `E` | encamp | **exits** (handled by caller) |
| `S` | toggle `search_flags & 1` (search mode) | yes |
| `L` | `search_flags \|= 2`, advance clock, look | **exits** (handled by caller) |
| fwd (`H`) | `TryStepForward()` — only clamps + sets `tried_to_exit_map`; the *move itself commits later* (§ below) | **exits** |
| turn L/R/180 (`K`/`M`/`P`) | update `mapDirection`, wall-type cache, redraw viewport, sound (L/R) | yes |
| other extended | scroll selected player (`scroll_team_list`), redraw party panel | yes |

After every command the position/time line refreshes
(`display_map_position_time`, `ovr015.cs:452`); on menu exit, the text window
is cleared if a PRINT had dirtied it (`:457-462`). Turning fires **no**
scripts; only stepping/looking does. (The `L` handler's direct
`ecl_offset = SearchLocationAddr` write at `ovr015.cs:407` is dead code —
every `RunEclVm` entry reassigns the offset (`ovr003.cs:2149`) — noted so
nobody models it.)

**The per-step sequence** (`sub_29758`, `ovr003.cs:2230-2396`): the outer
do-loop around the world menu. On entry to a block: choose block id
(`LastEclBlockId`, else 1), reload from disk only when
`reload_ecl_and_pictures` (else mark resident), **`vm_init_ecl` always**,
run the entry vector, chain-run if it chained, `LoadPic`+`RedrawView` when
reloaded. Then loop:

1. `main_3d_world_menu()` → key.
2. While `search_flags > 1 || key == 'E'`: `'E'` → `TryEncamp()` (M3 —
   fires vectors 3/4 around the camp UI, `ovr003.cs:1913-1926`); Look →
   back up `search_flags & 1`, force `search_flags = 1`, redraw, **run
   vector 2** (SearchLocationAddr), chain-run if chained, restore flags.
   Re-prompt the world menu each iteration.
3. **Run vector 1** (`vm_run_addr_1`).
4. If it chained (`vmFlag01`) → **chain runner**, and the pending step is
   abandoned (movement never commits). Else:
   - save last position; `locked_door()` (`ovr015.cs:468-593`): the whole
     interaction is gated on `area2_ptr.field_592 < 0xFF` (script-writable
     state, zeroed at every world-menu entry — `ovr015.cs:352,477,581-584`);
     it reads the facing edge's door state — `WallDoorFlagsGet`
     (`ovr031.cs:181-219`) returns 1 when the edge has no wall, else the
     2-bit door field: 0 = solid (no move), 1 = open/passable → move, 2 =
     locked → Hotbar `"Bash Pick Knock Exit"` (options gated on
     `can_bash/pick/knock_door` step-flags, thief presence, knock spell),
     3 = unpickable → same menu, Pick always fails. A success calls
     `MovePartyForward` (`ovr015.cs:318-345`): sound, position += facing
     delta **wrapped to 0–15 by masking**, refresh wall caches, reset the
     three door flags, advance the clock — slot 2 in search mode, slot 1
     otherwise (time passes twice as fast searching).
   - `RedrawView()`; movement sound if the position actually changed.
   - **Run vector 2** (SearchLocationAddr) — the enter-square event; chain-run
     if chained.
5. Loop while `!party_killed`; on exit reset the flag and fall out to the
   outer game loop (game-over/menu surface).

**Bookkeeping sites (plot-affecting, transcribed exactly):**

- `LastSelectedPlayer = SelectedPlayer` is saved at walk-loop entry
  (`ovr003.cs:2232`), after **every** world-menu return (`:2319`), on every
  E/Look re-prompt (`:2353`), and per chain-runner round (`:2192`). The
  consumer is EXIT's `restore_player_ptr`-gated restore (`ovr003.cs:14-18`)
  — a misplaced save means EXIT restores a stale player and every later
  Party-window read targets the wrong character.
- `area_ptr.LastEclBlockId = EclBlockId` commits at three sites, each gated
  on `!vmFlag01`: post-entry-vector (`:2292-2294`), per world-menu return
  (`:2321-2324`), and per chain-runner round (`:2196-2199`). **NEWECL itself
  writes `LastEclBlockId = <old block id>`** before swapping
  (`ovr003.cs:488`), so a chained script observes its predecessor's id until
  the next commit.
- **`party_killed` is live in M2**: DAMAGE computes it (`ovr003.cs:1682-1690`,
  reachable from any lethal trap event) and `RunEclVm` aborts mid-script on
  it (`:2154-2155`); the walk loop guards it at four points (`:2326, 2350,
  2358, 2369`) plus the do-while (`:2392`) and resets it on exit (`:2394`).
- **`vmFlag01` is *not* consumed where it is set** when the chain happens
  inside a flow with no checkpoint: `TryEncamp`/`MakeCamp` never test it
  (`ovr003.cs:1913-1926`), so after a pre-camp NEWECL the walk loop
  **re-prompts the world menu with the flag still up** — the player can
  turn, scroll, and toggle search with the *new* block resident — and the
  flag is consumed at the next step's post-vector-1 checkpoint (`:2363`),
  while the per-menu `LastEclBlockId` commit stays suppressed meanwhile
  (`:2321`). The flag is cleared only at walk-loop entry (`:2241`) and per
  chain-runner round (`:2187`). It is genuine, persistent, serializable
  engine state, not a per-run result.

**The chain runner** (`sub_29677`, `ovr003.cs:2180-2227`): after a NEWECL
chained mid-flow, loop: free the running animation and invalidate the
last-picture cache (`:2184-2186`), clear `vmFlag01`, refresh the roof cache,
clear `tried_to_exit_map`, save `LastSelectedPlayer` (`:2188-2192`), run the
**entry vector** of the (already-resident new) block; if it didn't chain
again: commit `LastEclBlockId`, conditionally redraw (the condition reads
`last_game_state`/`game_state`/`byte_1AB0B`, `:2203-2207`), run **vector
1**; if still no chain, run **vector 2**; if still no chain, restore the
selected player and redraw the party panel. Repeat while chains keep
firing; on exit commit `last_game_state = game_state` (`:2226`).

This is the D-VM3 inheritance in engine terms: `vmFlag01` == "the last run
ended in `Exit::ChainTo`", checked at fixed checkpoints of a *fixed
per-plan sequence* — never inside the VM.

### 1.7 3D view composition

`RedrawView` (`ovr029.cs:10-49`): in a dungeon, pick sky color — indoor
palette-table entry if the current cell's overhead byte `x2 > 0x7F`, else
outdoor (`sky_colours` 16-entry table, `ovr029.cs:7-8`; `area_ptr` holds the
two indices) — then `Draw3dWorld`. Outside dungeons (wilderness), draw the
bigpic instead (M6).

`Draw3dWorld` (`ovr031.cs:321-370`): either the **area map** or the corridor:

- **Background** (`Draw3dWorldBackground`, `ovr031.cs:93-137`): sky fill
  (pixels 24–67 of the viewport), a 2-px black band (68–69), a gray-8 fill
  (70–111); **sun/moon overlays** picked by hour and facing — sun `SKY#251`:
  facing East hours 1–5, facing West hours 13–18, facing **South only 3–5
  and 16–18** (`hour > 2` / `hour >= 16` narrowing, `ovr031.cs:113,124`);
  moon `SKY#250` at a fixed cell whenever facing North; all only outdoors
  with daytime sky color 11 — then the **horizon backdrop** `SKY#252`
  overlaid at cell row 8, terrain art covering the horizon bands.
- **Corridor**: three depth slots scanned **far → mid → near** (steps 2, 1,
  0 cells ahead of the party; `drawStep` walks back toward the party,
  `ovr031.cs:333-365`). Each slot draws front-facing walls and side walls
  from per-slot scans across the party's left/right axis: far fronts in two
  center-outward sweeps of **4 iterations each starting at the axis cell**
  (offsets 0–3 left and 0–3 right — a 7-cell span with the center scanned
  twice; the outermost front's piece anchors at screen column −1/11 and is
  fully clipped by the `0..=10` guard, mattering only for filler tracking),
  far sides 3 cells per half, mid 3 per half, near 2 per half
  (`Draw3dWorldFar/Mid/Near`, `ovr031.cs:373-640`). Every drawn piece is one
  of **ten draw-cell classes** (A–J) with fixed anchor rows/columns
  (`Column_A..Row_J` consts, `ovr031.cs:8-27`) and fixed shapes:
  `idxOffset = {0,2,6,10,22,38,54,110,132,154,1}`, `colCount =
  {1,1,1,3,2,2,7,2,2,1}`, `rowCount = {2,4,4,4,8,8,8,11,11,2}`
  (`ovr031.cs:140-142`). Class semantics: A = far front (1×2), B/C = far
  side L/R (1×4), D = mid front (3×4), E/F = mid side L/R (2×8), G = near
  front (7×8), H/I = near side L/R (2×11), J = far filler (1×2).
- **J-filler semantics** (exact, they were the v1 gap): each far front sweep
  tracks the previous iteration's front wall type (`var_17`). J fires in two
  cases, and in both draws with **the previous front's type, not the current
  cell's**: (a) a new front found while `var_17 > 0` — a gap filler between
  two consecutive far fronts, at the intervening column
  (`ovr031.cs:391-400,436-444`); (b) the front run ends but the previous
  cell's sweep-side wall continues — the end-cap case (`:404-411,448-455`).
  An invalid-coordinate probe resets the run tracker (`:385-389,430-434`).
  ssi-engine models only case (a) and takes the filler texture from the
  scan-order-earlier wall of the pair — see §1.11 item 7.
- **Wall texture selection**: a cell edge's 4-bit wall type `t` (from GEO)
  picks `wallset = (t-1)/5`, `slice = (t-1)%5`; the resident walldef block
  supplies `symbol_id = wallDef.blocks[wallset].Id(slice, idx)` per 8×8
  position, `idx` running over the class's index window
  (`draw_3D_8x8_titles`, `ovr031.cs:145-171`). Symbol 0 = draw nothing
  (transparency within a wall piece). Wall type 0 = no wall.
- **Coordinate wrap**: map queries wrap out-of-range coordinates to the
  opposite edge (`getMap_XXX`/`get_wall_x2`, `ovr031.cs:254-318`), except
  for blocks 0/10 where invalid coordinates return "nothing" — and note
  `MapCoordIsValid` tests `mapX >= 0` twice, never `mapY >= 0`
  (`ovr031.cs:175-178`) — flagged in §1.11.
- **Area map** (`DrawAreaMap`, `ovr031.cs:29-90`): an 11×11 cell window over
  the 16×16 grid (offset clamped 0–5), each cell one symbol: `0x104` + N/E/S/W
  wall bits; the party as `0x100 + facing/2` — drawn with **no-draw color
  temporarily set to 8** so the arrow's color-8 pixels let the underlying
  cell symbol show through (`draw_clipped_nodraw(8)` … restore 17,
  `ovr031.cs:86-88`). FD-16's consequence applies:
  the window is over the *block*, so side-by-side packed logical maps
  (Tilverton City + Thieves' Guild) can both appear near the seam —
  faithful behavior is whatever the original shows; verify at the seam
  against DOSBox before assuming a divergence.

Cross-check: ssi-engine composes the same view from **pre-baked wall
images** per (distance, placement) with pixel anchor tables
(`DungeonRenderer.java:26-30,105-140`) — an equivalent presentation built
from the same 156-byte walldef rows, useful as a second opinion but not our
model; we follow coab's per-8×8-cell placement because it is byte-provable
against the original's tables (and GBE renders walldefs with exactly coab's
`idxOffset/colCount/rowCount`, credited as such — `DaxWallDefFile.cs:204-239`).

### 1.8 Pictures, portraits, sprites, animations

- **Image container** (all DAX image files): header `{height:u16, width:u16
  (in 8-px columns), x_pos:u16, y_pos:u16, item_count:u8, unknown[8]}` at
  offset 0, then packed 4bpp pixels, 2 per byte high-nibble-first,
  `width*4` bytes per row per item (`DaxBlock.cs:33-50,124-147`). Masked
  loads turn one palette code into transparency-16. The 8-byte header field
  (`field_9`) is stored but never read by the draw path — docket.
- **Animated pictures** (`PIC*/SPRIT/FINAL` via `load_pic_final`,
  `ovr030.cs:35-149`): `numFrames:u8`, then per frame `{delay:u32, height:
  u16, width:u16, x_pos:u16, y_pos:u16+pad, unknown[8], 4bpp data}`; for
  `PIC`/`FINAL`, frames ≥1 are XOR deltas against frame 0's encoded bytes —
  **but only bytes `0..(bpp/2 − 1)`; the final packed byte (the frame's last
  two pixels) is copied verbatim** (`ega_encoded_size = bpp/2 − 1`, XOR loop
  `i < ega_encoded_size`, consume `+1` — `ovr030.cs:107,119-134`; the delta
  also indexes frame 0's bytes by the current frame's size, so frame
  dimensions are effectively required equal). Decode uses **mask color 0**
  (`DaxToPicture(0, masked, …)`, `:127`), and masked loads (`masked & 1` —
  SPRIT is loaded masked, `ovr008.cs:235`) then recolor **13 → 0**
  (`transparentNew/OldColors`, `ovr030.cs:10-11,129-132`): an encounter
  sprite's color-0 pixels are transparent and its color-13 pixels are black.
  Animations advance while a prompt waits (§1.5), `delay × 100` ms per
  frame. `AnimationsOn == false` collapses PIC/FINAL to one frame.
- **Small pictures** draw at cell (3,3) in the viewport (`DrawMaybeOverlayed`
  call sites, e.g. `ovr027.cs:188`); `picture_fade` applies a
  fade-recolor with a **1-in-4 random dither per pixel**
  (`DaxBlock.Recolor(useRandom=true)`, `ovr030.cs:8-9,17-24`) — RNG in the
  presentation path, and the recolor **mutates the cached frame in place,
  cumulatively, on every draw of the wait loop** (`DaxBlock.cs:71-94`), so
  the image converges to fully-faded at a rate set by the loop frequency
  and the cache stays mutated until reload — see §1.11 and §4.
- **Portraits**: `HEAD`/`BODY` pairs cached by id, head at (3,3), body at
  (8,3) (`head_body`/`draw_head_and_body`, `ovr030.cs:168-212`).
- **Encounter sprites**: `SPRIT` blocks are 3-frame arrays indexed by
  approach distance; `Show3DSprite(frames[distance])` overlays the frame at
  its own `(x_pos, y_pos) + (3,3)` cells (`ovr030.cs:215-228`). The
  engine-side dispatch `sub_30580` (`ovr008.cs:220-276`) picks sprite vs
  pic vs head/body by distance and `HeadBlockId`, loading `SPRIT` at
  distance > 0 and `PIC`/portrait at distance 0 — this is what SETUP
  MONSTER/APPROACH/ENCOUNTER MENU call through `load_encounter_visual`.
- **Bigpics**: `BIGPIC*` single images drawn at cell (1,1) inside the
  wilderness frame (`draw_bigpic`, `ovr030.cs:243-248`) — M6 surface, format
  lands in M2's inventory anyway (same container).
- **`LoadPic`** (`ovr025.cs:1398-1456`): the per-`game_state` screen
  recomposition (frame + view + panel + status) — our render-all entry
  point's spiritual ancestor. M2 needs its `DungeonMap` arm only.

### 1.9 Party panel and status line

`PartySummary` (`ovr025.cs:216-261`): header `"Name"` at (2,17) and
`"AC  HP"` at (2,33), one row per party member from row 4: name (white when
selected, else status-colored via `displayPlayerName`), AC in color 10 at
col 31, HP right-aligned near col 36 — color 10 full, 14 wounded
(`display_hp`, `:270-289`). Skipped entirely in wilderness state.
`display_map_position_time` (`ovr025.cs:1476-1511`): at (15,17), `"X,Y DIR
HH:MM"` (coordinates hidden when `block_area_view` forbids the area map),
plus `" search"` when search mode is on (or `" camping"`). The world menu
repaints it after every command.

### 1.10 Sounds and timing touchpoints (M2 slice)

Walking plays `sound_a` on turn and on a committed step (`ovr015.cs:321,
431,439`, `ovr003.cs:2380`); CALL-0x3201 plays a state-selected effect. M2
does not synthesize audio (M8) but the **events** cross the tick boundary
now so traces are complete. Original real-time waits in M2-relevant code
paths: per-char pacing (3·speed ms), `GameDelay` (100·speed ms) — also the
body of `DisplayStatusText`'s timed status flash (§1.6's "Not Here") —
animation frame delays (100·frame ms), `MovePartyForward`'s 50 ms step
delay (`ovr015.cs:322`), DAMAGE's fixed 3000 ms party-wipe pause
(`ovr003.cs:1699`), and prompt blink timers — each becomes a tick count
(D-UI1's time model).

### 1.11 Cross-check contradictions and oddities (flagged, not absorbed)

1. **ssi-engine's flat-color backdrop bands disagree with coab's**: white/
   light-gray/brown at y 67–111 (`DungeonRenderer.java:76-86`, its `COLOR`
   mode) vs coab's black band + gray-8 fill (`ovr031.cs:96-97`). Its `SKY`
   mode (backdrop image at (3,8) + sky fill) matches coab's structure.
   Likely a different-title mode; coab + DOSBox screenshots are our oracle
   → docket, settle at first golden capture.
2. **`MapCoordIsValid` never validates `mapY >= 0`** (checks `mapX >= 0`
   twice, `ovr031.cs:175-178`). Transliteration typo or genuine original
   bug — behavioral difference only at north-edge cells of blocks 0/10.
   Docket; replicate whatever DOSBox shows.
3. **`Draw3dWorldBackground` queries `get_wall_x2(mapPosY, mapPosY)`** —
   Y passed twice (`ovr031.cs:99`) — gating only the sun/moon overlay.
   Same class as #2: docket, verify against the running game.
4. **`Recolor`'s fade dither uses a time-seeded RNG** (`DaxBlock.cs:7,84`)
   — in the original the dither pattern is timing-dependent, so it can
   never be oracle-trace-comparable. We draw it from the engine PRNG (D9)
   and exclude fade frames from cross-implementation pixel comparisons
   (hash-goldens of *our own* output remain deterministic).
5. **hackdocs' DRAW18/DRAW23/GEODATA describe the TLB/FRUA generation**
   (interlaced planar rows, 24×24 6-byte-record geo) — cross-checked and
   confirmed *not* applicable to CotAB's DAX-era formats (consistent with
   the geo.rs module's existing finding). Useful only at M9.
6. **vm-scriptmemory.md §1's vector table** says vectors 1/2 fire "after
   every world-menu command" — §1.6's transcription refines this: panel
   commands and turns are consumed *inside* the menu loop; the vectors fire
   per **step/look attempt** (and per chain-runner round). Flagged for a
   one-line correction in that doc rather than absorbed silently.
7. **ssi-engine's J-filler model disagrees with coab's**: it draws fillers
   only between two adjacent far fronts (`renderFiller` when the *next*
   index is a wall, `DungeonRenderer.java:112-118,135-139`) and textures
   them from the scan-order-earlier wall, while coab also draws end-caps
   where a side wall continues past the last front and always textures from
   the *previous* front (§1.7). Since coab's far sweeps run center-outward,
   the two pick different neighbors on the left half when adjacent fronts
   differ in type. coab + DOSBox is our oracle; settle at golden capture.
8. **GBE's walldef→8×8 pairing has a `block_id == 0 → base 100` special
   case** (`DaxWallDefFile.cs:153-157`) that coab lacks (`ovr031.cs:675`
   multiplies unconditionally), and walldef block 0 is a live CotAB path
   (LOAD FILES `0x7F` → `LoadWalldef(1, 0)`, `ovr003.cs:539-541`). Check
   whether WALLDEF block 0 is ever multi-wallset in real data; docket
   either way.
9. **coab's `GetInputKey` drains the keyboard buffer to the newest key**
   on every nonzero read (`seg043.cs:55-62`) — type-ahead is discarded.
   Original behavior or transliteration artifact? DOSBox test: mash forward
   during a slow redraw; count committed steps. The input-queue read
   contract (D-UI1) ships coab's semantics pending this.
10. **coab's list menus ignore Up/Down arrows** (`sl_select_item`'s
    special-key switch handles only `'G'/'O'/'I'/'Q'` — Home/End/PgUp/PgDn,
    `ovr027.cs:617-640`), contradicting common memory of the game. DOSBox
    check before D-UI6's key map is pinned.

## 2. Decisions

### D-UI1 — The tick contract (one-way door)

```rust
// gbx-engine (shapes only; names bikesheddable at implementation)
pub struct Engine { /* owns: EclMachine, game state, UI shell state,
                       framebuffer, palette, PRNG, GameData, input queue */ }

pub const TICK_HZ: u32 = 60;

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum InputEvent {
    Char(u8),          // printable 0x20..=0x7A, pre-uppercased by nobody —
                       // the engine uppercases exactly where the original does
    Enter, Escape, Backspace,
    Ext(ExtKey),       // the original's 0x00-prefixed extended scancodes
}
#[derive(Clone, Copy, PartialEq, Eq, ...)]
pub enum ExtKey { Up, Down, Left, Right, Home, End, PgUp, PgDn,
                  Kp1, Kp2, Kp3, Kp4, Kp5, Kp6, Kp7, Kp8, Kp9 }
// Kp5 included: the original maps it to ' ' via keypad_ctrl_codes[4]
// (ovr027.cs:124) — the mapping table must be total.

pub struct Frame<'a> {
    pub pixels:  &'a [u8; 320 * 200],   // palette indices 0..=15
    pub palette: &'a [[u8; 3]; 16],     // RGB; palette effects mutate this
    pub sounds:  &'a [SoundEvent],      // events fired this tick (M8 synthesizes)
    pub serial:  u64,                   // bumps on any visible change → skip redundant presents
}

impl Engine {
    pub fn new(data: GameData, seed: u64) -> Result<Self, BootError>;
    pub fn tick(&mut self, input: &[InputEvent]) -> Frame<'_>;
    pub fn title(&self) -> &str;
}
```

- **Input model.** Two-level, mirroring the original keyboard stream
  (printables vs 0-prefixed extended codes, §1.5). The frontend pushes the
  events it collected since the last tick, in order; the engine appends them
  to its own small queue. **Reads replicate the original's semantics, which
  are not plain pops**: a widget key-read consumes the whole queue and takes
  the newest event (`GetInputKey`'s drain-to-last, §1.5 — docketed for
  DOSBox confirmation, §4 item 8), and `clear_keyboard` call sites become
  queue-clears at the same points. Both are deterministic functions of
  engine state, so replays are unaffected. A session's input trace is
  `[(tick_index, InputEvent)]` (H5); replaying it against the same
  `(data fingerprint, seed)` reproduces every frame hash.
- **Time model.** One tick = 1/60 s of *game-presentation* time; the engine
  never reads a clock (D9). One-shot waits convert as
  `ticks = max(1, round(ms * 60 / 1000))` — at default speed 4: `GameDelay`
  400 ms → 24 ticks, animation frames `delay×100` ms → `delay×6` ticks,
  step delay 50 ms → 3 ticks, DAMAGE's party-wipe pause 3000 ms → 180 ticks
  (fixed, not speed-scaled — `ovr003.cs:1699`). **Per-character pacing uses
  a fractional accumulator, not per-char rounding**: each tick the text
  presenter emits `⌊acc⌋` characters where `acc += tick_ms / char_ms`
  (`char_ms = game_speed_var × 3`) — rounding per character would run 39%
  slow at default speed (12 ms/char vs a 16.7 ms tick) and 5.5× slow at
  speed 1, and the error accumulates per character; the accumulator makes
  average pacing exact at every speed. The original pacing is itself
  parameterized by the save-file speed setting, so fidelity is parametric.
  The game-world clock (ECL CLOCK / `step_game_time`) is unrelated to
  ticks — it advances only via steps/rest, as in the original.
- **Frame.** A borrowed view of the engine-owned indexed framebuffer +
  palette (D5). Frontends only present + scale; palette expansion to RGBA is
  a frontend loop (or a shared helper), never engine state. `serial` lets
  frontends skip presenting unchanged frames; hash-goldens hash
  `pixels ‖ palette`.
- **Data access.** `GameData` (in `gbx-formats`) is an in-memory archive
  set — the frontend reads `GBX_DATA_DIR` (or the browser fetches a
  user-supplied bundle) and hands the bytes over. CotAB's full data set is
  a few MB; the core does **zero I/O** (wasm-clean, deterministic, and the
  original's mid-game `load_ecl_dax`/`LoadDax` calls become in-memory
  lookups). The `"Loading...Please Wait"` prompt still paints (faithful),
  it just never lingers.
- **Nothing blocks.** `tick` runs at most: input dispatch → bounded state
  advance (incl. a VM step budget, D-UI2) → recomposition of dirty screen
  regions. No call path in `gbx-engine` waits on anything.

**Alternatives considered** (required for the door):

1. *Engine-calls-presenter (inversion of control):* the engine invokes a
   `Presenter` trait for modal interactions. Rejected: reintroduces
   blocking-shaped control flow, poisons WASM (no reentrant event loop),
   makes save-anywhere and replay retrofits (the exact D8 rationale).
2. *Coroutine/async core:* suspend points via `async` or generators.
   Rejected: the VM already models suspension explicitly (D-VM3
   `Request`/`resume`); a second suspension mechanism on top adds runtime
   machinery, hurts determinism auditing, and buys nothing the state enum
   doesn't.
3. *Draw-command stream out (frontend rasterizes):* `tick → Vec<DrawCmd>`.
   Rejected: frontends stop being thin (D5 says core owns the framebuffer),
   hash-goldens become format-goldens, and faithful EGA quirks (palette
   remaps, transparency codes) leak into every frontend.
4. *RGBA framebuffer out:* rejected — palette effects are pointer-cheap on
   indexed + palette, and indexed pixels are what DOSBox screenshots and
   golden hashes want to compare.
5. *Variable-timestep tick (`tick(dt)`):* rejected — D9 wants replays keyed
   by tick index; a fixed atom makes traces integers and frame hashes
   reproducible everywhere. 60 Hz chosen over 120 Hz because every M2
   pacing constant lands ≥ 1 tick anyway.

### D-UI2 — The shell state machine (the D8 door)

One serializable state enum, advanced only by `tick`. Two layers, because
the original has two: a **flow layer** (which script/engine sequence is in
progress — §1.6's transcriptions) and an **interaction layer** (which
prompt-line widget or presentation gate is consuming input *right now*).

```rust
enum Shell {
    Boot(BootFlow),                        // block entry (sub_29758 preamble) — a flow:
                                           //   its own chain checkpoint + post-chain stages
    WorldMenu { menu: Widget },            // §1.6 world menu; owns turn/panel commands;
                                           //   Widget (not bare Hotbar) so the timed
                                           //   "Not Here" status (Delay) can park here
    Step(StepFlow),                        // per-step sequence after fwd/Look/E
    GameOver,                              // party_killed unwound the loop (M2 stub screen;
                                           //   M3+ routes to the game menu)
}

// One vector-run in progress, inside any flow stage:
enum VmPhase {
    Pump,                     // stepping EclMachine within the tick budget
    Present,                  // draining the presentation queue (text pacing, gates)
    Gate(Widget),             // a Request's interaction is open (see below)
}

// The interaction layer — each maps 1:1 to an original blocking call site (§1.5):
enum Widget {
    Hotbar { text, highlights, selected, accept_ext,
             timeout: Option<(u32 /*ticks*/, u8)>,
             ext_scrolls_party: bool,      // sub_317AA menus: extended keys scroll the
                                           //   party panel while parked (§1.5)
             valid_keys: Option<KeySet> }, // sub_317AA menus re-prompt on anything else;
                                           //   Esc does NOT exit them
    ListMenu { items, index, screen_index, top_row, .. },  // sl_select_item; top_row is
                                           //   bound from the text cursor at open (§1.5,
                                           //   VERTICAL MENU's textYCol+1 coupling)
    TextEntry { prompt, buf, max, numeric },   // getUserInput{String,Short}
    PressAnyKey,                               // pagination / DisplayAndPause
    Delay { ticks_left: u32 },                 // GameDelay / DELAY / anim pauses
}
```

**The flow plans are fixed sequences with chain checkpoints — and a
checkpoint *suspends into* the chain runner, it does not discard its flow.**
`BootFlow`/`StepFlow` are cursored transcriptions of §1.6 — e.g. `StepFlow =
[SaveBookkeeping, RunVector(1), ChainCheckpoint, DoorInteraction(commit move),
Redraw, RunVector(2), ChainCheckpoint] → WorldMenu`. `RunVector(n)` pushes an
activation (`machine.enter`) and pumps `VmPhase` to completion;
`ChainCheckpoint` reads the persistent `chained` flag (the `vmFlag01`
equivalent — see below) and, if set, runs the **chain runner as a nested
sub-flow of the current stage** (`ChainFlow` rounds looping while chains
fire), then **resumes the suspended plan at the stage after the
checkpoint**. This resume-after-chain shape is mandatory, not stylistic: the
original's boot site runs `LoadPic`/`RedrawView` and clears
`reload_ecl_and_pictures` *after* its chain-runner call
(`ovr003.cs:2298-2313`), and the Look site restores `search_flags` after its
(`:2344-2347`) — abandoning the plan at those two checkpoints would leave
the screen uncomposed / the party stuck in permanent search mode. At the two
tail checkpoints (post-vector-1, post-vector-2) the remaining plan is empty,
so resume degenerates into the abandonment the v1 draft described. The chain
runner is itself a plan with the same checkpoints. **No sequencing decision
lives inside the VM**, and every "blocking" site in the original is one of
the five `Widget`s parked in `VmPhase::Gate` or `WorldMenu` — there is
nowhere left for hidden blocking to live.

**`chained` is persistent, serializable engine state — not a per-run
result.** Per §1.6: it is set by any run ending in `ChainTo`, cleared only
at walk-loop entry and per chain-runner round, and *survives across the
world menu* when a chain fires in a flow with no checkpoint (the M3 camp
case) — suppressing the per-menu `LastEclBlockId` commit meanwhile and
being consumed at the next step's post-vector-1 checkpoint. The state
machine must therefore tolerate `WorldMenu` running with `chained` set and
the *new* block resident — an invariant with its own conformance test.

**`party_killed` is engine state with an abort rule**: when a service or
script sets it, `Pump` aborts the current run mid-script (the activation
stack is abandoned; not presented as `Done(Ended)`), flow stages guard on
it at the transcribed §1.6 sites, and the walk loop unwinds to
`Shell::GameOver` (an M2 stub screen), resetting the flag — matching
`RunEclVm`'s abort condition and the loop's four guards
(`ovr003.cs:2154-2155, 2326-2392`).

**The presentation queue carries more than text**: its items are paced text
jobs, draw commands, and **explicit gates** — `Gate::PressAnyKey` for the
original's unconditional `DisplayAndPause` sites that are not
overflow-pagination (DAMAGE's terminal "press <enter>" at `ovr003.cs:1703`
and its party-wipe sequence: outer frame, text into the custom region
rows 1–22 × cols 1–38, a fixed 180-tick delay — `:1692-1700`). Region
bounds are queue-item parameters (the three §1.2 regions are presets;
`press_any_key` takes arbitrary bounds in the original, `seg041.cs:125-129`).
DAMAGE itself is an M4 opcode (census count 1), but the queue's gate/region
machinery is M2 design surface so its arrival changes nothing structural.

**How each D-VM3 engine obligation is discharged:**

| Obligation (vm-scriptmemory D-VM3) | Mechanism here |
|---|---|
| Effects presented **in yield order, none dropped**, before the next Request's UI | `VmPhase::Pump` appends every `Effect` to an ordered presentation queue. On `Request`, the phase moves to `Present` and the queue drains (text pacing/pagination may take many ticks) **before** the request's `Widget` opens (`Gate`). The machine is not stepped in `Present`/`Gate`. |
| Requests parked across ticks | `Gate(widget)` persists across ticks; `machine.resume(reply)` is called only when the widget completes. `pending()` re-presents after restore (M3). |
| ChainTo: swap immediately, finish the in-progress flow against the new block | On `Done(ChainTo(id))`: write `LastEclBlockId = <old id>` (NEWECL's own write, `ovr003.cs:488`), reload block bytes from `GameData`, apply the init resets, set the persistent `chained` flag — *during* whatever flow is running. The flow's remaining non-VM stages still run (the TryEncamp behavior: the camp UI completes, vector 4 fires from the new block — M3); the chain runner enters only at the next `ChainCheckpoint`, as a nested sub-flow that **resumes** the suspended plan afterwards — exactly where and how `sub_29758`/`sub_29677` test `vmFlag01` (§1.6). |
| `vm_init_ecl` on every walk-loop entry; reload rules | `Boot`/block-entry stages call `machine.reinit()` when re-entering the resident block and full reload+init on switch — the `reload_ecl_and_pictures` distinction (§1.6). The **engine-side half** of `vm_init_ecl` (redraw/sprite/encounter flags, `HeadBlockId = 0xFF`, rest-encounter params, `can_cast_spells`, `inDungeon = 1`, the conditional area-field resets — vm-scriptmemory §1, `ovr008.cs:89-133`) is a named flow stage owned here; `gbx-vm`'s `reinit()` cannot and does not perform it. |
| `party_killed` mid-script abort | `Pump` checks after every step; abandons the activation stack without a `Done`; flow guards + `GameOver` unwind per §1.6. |
| Fuel watchdog (engine policy) | `Pump` executes ≤ `STEP_BUDGET` (default 10 000) VM steps per tick, then yields to the next tick — a `GOTO`-self script keeps the app responsive at 60 fps forever. A cumulative per-flow counter (default 1 M steps) raises a loud diagnostic (log + inspector surface), does **not** kill the machine (the original wedges; we wedge *observably*). |
| GOSUB-depth watchdog | Checked after each step against a threshold (default 256); same loud-diagnostic policy (the stack itself stays faithfully unbounded). |
| `Done(Ended)` with a suspended outer frame | The flow consults `machine.pending()` before advancing its cursor, per the D-VM3 rule — it never blindly runs the next vector over a parked activation. |

**Request → Widget mapping** (the M2 slice of the Request taxonomy;
`gbx-vm::Request` grows variants as opcodes land, each declaring its widget):

| Request | Widget | Reply |
|---|---|---|
| `HorizontalMenu{options}` | `Hotbar` with `ext_scrolls_party` + `valid_keys` set (the `sub_317AA` behaviors: party scrolling while parked mutates the selected player before `resume`; invalid keys and Esc re-prompt, §1.5) | `Selection(i)` |
| `VerticalMenu{header, items, ..}` (M2, opcode 0x15) | header rendered (and possibly paginated) through the text system first, then `ListMenu` with `top_row` bound from the resulting cursor (+1) — the §1.5 coupling | `Selection(i)` |
| `InputNumber`/`InputString` (M2, 0x0F/0x10) | `TextEntry` (numeric re-prompts until parse — engine-side loop, §1.5) | value |
| `Delay` | `Delay{ticks}` (speed-scaled) | `Delay` |
| `Combat` | **M2 stub**: paint `"COMBAT (stub)"`, auto-reply a scripted outcome after a keypress; logged | `Combat` |
| `SelectPlayer{prompt}` (WHO — census count 0, ships as stub) | `Hotbar` over party | `PlayerId` |

The world-menu, door-menu, encounter-menu, and pagination interactions are
**engine-owned** `Widget`s that never touch the VM at all (they exist in
`WorldMenu`/flow stages) — same widget code, different owner, matching the
original's shared `displayInput`.

**Engine state carried (M2 slice, all serializable):** position/facing
(`mapPosX/Y` 0–15, `mapDirection` ∈ {0,2,4,6} of the 8-dir encoding),
`search_flags` (+ the Look backup), the `chained` flag, `party_killed`,
`restore_player_ptr`, per-step door flags, `field_592`, `tried_to_exit_map`,
game clock, selected/last-selected player index, `EclBlockId`/
`LastEclBlockId`, resident GEO + walldef + symbol sets (incl. the
`setBlocks` `(blockId, setId)` triple the original persists in saves,
§1.3) + picture caches (by block id, reloadable from `GameData`), text
cursor + `bottomTextHasBeenCleared`, redraw flags (a named struct replacing
coab's `byte_1AB0B`/`can_draw_bigpic`/`spriteChanged`/… — the exact set is
pinned during implementation against the §1.6/§1.8 call sites), the running
animation (frames + `ticks_until_next`), the active `Shell`. `game_state`
(Dungeon/Wilderness/Camping/…) exists as a `ScreenLayout` discriminant with
only `DungeonMap` live in M2, plus the `last_game_state` shadow the chain
runner's redraw condition reads (§1.6).

### D-UI3 — PRINT presentation

- The **text system** owns: region geometry (§1.2's three regions), the
  persistent cursor, the wrap algorithm (transcribed from `press_any_key` —
  punctuation set, exact-fit space case, post-wrap space skip), per-char
  pacing (1 tick/char at default speed while `DelayBetweenCharacters`),
  and the pagination gate (cursor reset → `PressAnyKey` widget with the
  color-13 prompt → keyboard-queue clear → region clear → continue).
  Pagination keypresses are ordinary input-trace events (D-VM3/H5).
- `Effect::Print{text, clear_first}` enqueues a paced text job against
  `NormalBottom` in color 10; `clear_first` (PRINTCLEAR) resets the cursor
  to (17,1) and clears the region before the job. `Effect::PrintReturn`
  advances the cursor (row+1, col=1). The queue is the D-VM3 presentation
  buffer: strictly FIFO, spans ticks, must be empty before any `Gate` opens.
- Cursor state persists across jobs, scripts, and flows (DAMAGE's
  data-dependent pagination emerges from this for free, as in the original).
- The world menu's clear-if-dirty behavior (§1.4 last bullet) is a
  `WorldMenu`-entry stage, not a text-system policy.

### D-UI4 — Renderer composition pipeline

**Immediate-mode, like the original.** The framebuffer is persistent;
draw routines mutate it at the same call sites the original mutates VRAM
(`RedrawView`, `PartySummary`, region clears, glyph/tile blits). There is no
scene graph, no retained display list, no per-frame full recomposition —
what didn't get drawn over stays. This is load-bearing for fidelity (the
original's screen is full of "stale until repainted" behavior — §1.4's text
window, §1.9's panel refresh discipline) and makes hash-goldens exact.

Draw routine inventory (all in `gbx-engine`, pure functions over
`&mut Framebuffer` + assets):

1. **Primitives**: cell-rect clear/fill, 8×8 symbol blit (5-set id routing +
   per-set base, §1.3; id 0 / out-of-range = loud error — the skip belongs
   to the wall drawer), mono glyph blit (bg/fg), 4bpp image blit with
   transparency-16, the pixel clip window, and **per-call no-draw color and
   recolor-pair parameters** (the original's mutable blit state, §1.1 —
   e.g. the area-map arrow's no-draw-8), recolor tables (fade/transparent —
   §1.8), `DrawColorBlock` pixel fills.
2. **Frames**: `draw8x8_03` + `DrawFrame_Outer` from the §1.2 symbol
   tables (engine-binary constants, shipped like rules-pack data with an
   evidence citation — they are coordinate/id tables, not game art; the art
   is the user's 8×8 DAX symbols).
3. **3D corridor**: §1.7 transcribed — background (sky/bands/sun/moon/
   horizon), then far→mid→near wall passes over the ten draw-cell classes,
   walldef-selected symbols, symbol-0 skip. Area-map alternative view.
4. **Pictures**: viewport picture at (3,3), head/body at (3,3)/(8,3),
   distance sprite at block-declared offset, bigpic (stub seam, M6),
   animation frame advance.
5. **Panels**: party summary, position/time line.
6. **Prompt line + text window**: from D-UI3 and the widgets.

**Palette**: 16-entry engine-owned table initialized to the EGA canon
(§1.1); `SetEgaPalette`-equivalent mutates it (frame.serial bumps). Pixels
are palette indices; transparency-16/no-draw-17 never reach the
framebuffer.

**Redraw coordination**: the original's redraw flags (§1.6/§1.8) become one
named `RedrawFlags` struct; each flow stage sets/clears exactly what its
coab counterpart does. We do not invent a dirty-rect system — regions are
repainted where the original repaints them.

### D-UI5 — Crate boundaries and the M2 format inventory

- **`gbx-formats`** (decoders only, no drawing): existing `dax`/`geo`/
  `ecl_text`/`detect`, plus M2 additions:

  | Format | Files | Shape (pinned in implementation against refs) | Reference |
  |---|---|---|---|
  | 4bpp image block | `8X8D*`, `BIGPIC*`, `HEAD*`, `BODY*`, `SKY` | §1.8 header + packed pixels, mask-color→16 | coab `DaxBlock.cs`; GBE `DaxImagePlugin` |
  | Animated picture | `PIC*`, `SPRIT*`, `FINAL*` | §1.8 frame container; XOR delta over bytes `0..bpp/2−1` only, last byte verbatim (PIC/FINAL); mask-0 decode + masked 13→0 recolor | coab `ovr030.load_pic_final` |
  | Mono font | `8X8D1` block 201 | 177 × 8-byte 1bpp glyphs | coab `seg041.Load8x8Tiles` |
  | Walldef | `WALLDEF*` | 780 B/wallset = 5 styles × 156 tile-ids laid out per the ten class windows; style→(set,slice) selection; ≥0x2D rebase computed once from the base set per load call (§1.3) | coab `WallDefs`/`LoadWalldef`; GBE `DaxWallDefFile` |
  | `GameData` | all of the above + `ECL*`/`GEO*` | in-memory archive set keyed (file, block); detection fingerprint | this doc, D-UI1 |

- **`gbx-engine`**: everything in D-UI1–D-UI4 (framebuffer, text, widgets,
  flows, walk loop, renderer, `ScriptMemory`/`EngineServices`/`VmHost`
  implementations, tick API). **Platform purity is enforced**: no `winit`/
  `softbuffer`/`egui`/`wgpu`/`std::fs`/`std::time` dependencies; the wasm32
  CI build of the core plus a `#[deny]`-style dependency check in CI (a
  script greping `cargo tree`) keep it honest.
- **`gbx-rules`**: the door-bash STR table (§1.6's `bash_door` matrix) and
  the EGA palette canon land here as evidence-tagged data (D6) — first real
  rules-pack entries.
- **`frontends/desktop`** (new): winit + softbuffer presenter (D-UI6).
- **`frontends/web`** (new): wasm-bindgen + canvas presenter (D-UI6).
- **`tools/inspect`** (new): egui, D-UI8.
- **`frontends/cli`**: gains `restrike walk` (headless tick-driver: feed a
  trace, dump frame hashes/PNGs) — the H5 seed and the golden-test debugger.

### D-UI6 — Frontend presentation contract

A frontend is ≤ ~300 lines that: loads `GBX_DATA_DIR` bytes into `GameData`,
constructs `Engine`, runs a 60 Hz loop (winit `ControlFlow::WaitUntil` /
`requestAnimationFrame` with accumulator), maps platform key events →
`InputEvent`, calls `tick`, expands indexed→RGBA via the palette when
`frame.serial` changed, presents scaled. No other knowledge.

- **Scaling (default): aspect-correct per-axis integer scaling at the 5:6
  pixel ratio** — 320×200 → ×5,×6 = 1600×1200 (or ×10,×12 = 3200×2400 on
  higher-DPI monitors; largest `(5k, 6k)` that fits, letterboxed on black).
  Rationale: the art targeted 4:3 CRTs with 1:1.2 non-square pixels; D4
  makes the faithful look the default. **First QoL toggle: square-pixel
  integer mode** (×k,×k — crisper but 17% squashed geometry), default-off
  per D4. Sharp-bilinear / CRT shaders: deferred to M8 (the stated wgpu
  trigger); softbuffer does nearest-neighbor integer copies only.
- **Keyboard**: letters/digits/punctuation → `Char` (layout-resolved text,
  not scancodes, so AZERTY users get the keys they type); Enter/Esc/
  Backspace → their variants; arrows, Home/End/PgUp/PgDn, and the numpad →
  `Ext(..)`. The engine maps `Ext` → the original's `keypad_ctrl_codes`
  semantics internally (Up/Kp8 = forward, Left/Kp4 = turn left, Right/Kp6 =
  turn right, Down/Kp2 = turn around, §1.5) — frontends never know what a
  key *means*. Note the map is context-dependent in the original (list
  menus scroll on Home/End, not Up/Down — §1.11 item 10, docket 9); the
  engine owns those decisions per widget.
- **Web**: same crate graph via wasm32; canvas 2D `putImageData` of the
  RGBA expansion + CSS `transform: scale(5,6)`-style sizing with
  `image-rendering: pixelated`; data supplied by the user via a
  directory-picker/zip (never bundled — D10). The M2 web build is the D8
  proof, not a product: one page, one canvas, keyboard only.
- Window title from `Engine::title()`. Sounds ignored by both frontends in
  M2 (events exist in `Frame` for traces).

### D-UI7 — Testing (M2 rungs of H1/H2/H5)

- **In-repo (CI, no game data):**
  - Format decoders: synthetic fixtures for walldef/image/font/PIC-delta
    (hand-authored bytes, shipped freely) + fuzz smoke (existing pattern).
  - Text system conformance: wrap/pagination/pacing unit tests transcribed
    from §1.4's semantics (exact-fit space case, punctuation runs,
    pagination cursor reset, queue-before-gate ordering).
  - Widget conformance: hotkey/highlight/cycle/timeout semantics per §1.5.
  - **Framebuffer-hash goldens**: a synthetic mini-game (fixture GEO block +
    fixture walldef/8×8/font assets + micro-ECL event scripts) driven
    headlessly through `tick` with pinned input traces; SHA-256 of
    `pixels ‖ palette` at checkpoints **defined as explicit `(trace,
    tick_index)` pairs**, never named moments — a running animation makes
    "menu open" hash-ambiguous by sample tick. Fixture coverage must
    include a delta-animation whose last packed byte differs between frames
    (the XOR-scope edge, §1.8). Regeneration via an env flag; PNG dumps on
    mismatch for eyeballing.
  - State-machine soundness: a property test that no reachable `Shell` state
    can call `machine.step` while a `Gate` is open or the presentation
    queue is non-empty (the D-VM3 MUST, mechanically enforced); serialize/
    restore round-trips of every `Shell`/`Widget` variant mid-flight;
    ordering tests for the two resume-after-chain sites (boot's
    post-chain `LoadPic`/`RedrawView` + flag clear, Look's `search_flags`
    restore — D-UI2); a `WorldMenu`-with-`chained`-set invariant test; a
    party-scroll-during-parked-menu test asserting post-`resume`
    Party-window reads target the scrolled-to player.
- **Local-only (GBX_DATA_DIR):**
  - Real-asset decode sweeps (every walldef/8×8/PIC block in the data set
    decodes without error; dimensions sane).
  - **Tilverton renders vs DOSBox**: a documented capture procedure
    (dosbox-staging screenshot at a pinned position/facing → crop to
    320×200 → compare). Spot squares chosen to cover: open street, wall
    left+right corridor, door ahead (each door state), area-map view,
    event text with pagination, a menu open. Initial bar: structural match
    (same walls in same cells, same text layout); exact pixel equality is
    the aspiration once palette/rounding details settle. Divergences →
    docket entries, not silent fixes.
  - **The M2 exit gate**, verified as: (1) a scripted walk trace through
    Tilverton's streets (enter from the city gate block, walk a fixed
    circuit past ≥ 3 event squares) runs headlessly with all event text
    matching a DOSBox transcript of the same walk; (2) the desktop build
    plays the same walk interactively with spot-check screenshot parity;
    (3) the web build loads the same data and walks the same circuit
    (manual smoke — core hashes are identical by construction since it is
    the same crate compiled to wasm32, and a wasm-run subset of the golden
    tests in CI proves the compilation isn't lying).

### D-UI8 — tools/inspect v0 (seams only)

An `eframe`/egui app, read-only in v0: opens `GBX_DATA_DIR`, embeds the
existing disassembler (block picker → `gbx_vm::disassemble` listing), a
resource browser (DAX block tree → decoded views: images with palette,
walldef composites per style, GEO automap render), and a live engine pane —
an embedded `Engine` instance driven by inspector-owned ticks with a
framebuffer view, `Shell`/`VmPhase` display, ScriptMemory watch (the
unknown-access log front and center), and a step/pause control. It consumes
only public-ish read surfaces (`#[doc(hidden)] pub` inspection getters or an
`inspect` feature on `gbx-engine`) — no `winit` leakage into the core, no
inspector types in engine signatures. Design deferred; only these seams are
commitments.

## 3. Non-goals (and what they must not be blocked by)

- **Combat UI (M4)**: `Request::Combat` parks and stubs; `DrawFrame_Combat`
  and `CombatSummary` region geometry are catalogued (§1.2) but dead.
  Nothing in the Shell enum prevents a `Combat(CombatFlow)` variant later.
- **Spell/character screens, camp, shops (M3+)**: `C`/`V`/`E` world-menu
  commands stub to status text ("Not yet" class); the camp flow's
  vector-3/4 protocol is already representable (flow plan + checkpoints).
- **Audio (M8)**: `SoundEvent`s cross the boundary now; synthesis later.
- **QoL overlays, mouse, gamepad (M8)**: input enum is `#[non_exhaustive]`;
  overlays would be a post-compose framebuffer pass — nothing here assumes
  pixels are script-authored only.
- **Save format (M3)**: every Shell/Widget/flow state is `serde`-able by
  construction (this doc's structs carry no borrows); M3 decides the
  envelope/versioning, not the shape.
- **Wilderness/overworld, Parlay, vault, temples (M6)**: `ScreenLayout`
  discriminants + bigpic/mapcursor seams exist; only `DungeonMap` is live.

## 4. Open questions → fidelity docket seeds

1. §1.11 items 1–3 and 7 (backdrop band colors; the two coab coordinate
   oddities; the J-filler cross-check disagreement) — settle against DOSBox
   screenshots at first golden capture.
2. `field_9` (image-header 8 bytes): meaning unknown; carried, unused.
3. Frame-symbol tables (§1.2) and the bash matrix (§1.6) ship as
   engine-constant data — confirm none encode copyrightable *art* (they are
   id/coordinate tables; the art lives in the user's DAX files). Legal
   posture per PLAN §6 rule 2.
4. Pagination prompt text/color and the `"Loading...Please Wait"` string are
   engine-generated (not from game data) in the original too — confirmed
   from coab string literals; keep as constants, cite.
5. Sun/moon hour windows and cell math (§1.7) are transcribed but
   unverified against the sky in motion — verify at DOSBox capture time
   (cheap: set clock via camp/rest, face each direction).
6. The 50 ms `MovePartyForward` delay (§1.10): confirm it is perceptible
   (3 ticks) and whether DOSBox shows it distinctly from redraw cost.
7. FD-16 seam behavior (area-map window spilling across packed logical
   maps): capture the Tilverton City/Guild seam in DOSBox and match.
8. **Input read semantics** (§1.11 item 9): is drain-to-last the original
   binary's behavior or a coab artifact? DOSBox type-ahead test (mash
   forward during a slow redraw, count steps). D-UI1 ships coab's
   semantics; the queue read is one function to swap if this falsifies.
9. **List-menu arrow keys** (§1.11 item 10): coab ignores Up/Down in
   `sl_select_item`; verify against DOSBox before pinning D-UI6's map.
10. **Fade-recolor dynamics** (§1.8): the original mutates the cached image
    in place per wait-loop iteration, so convergence rate is loop-frequency
    dependent; our mapping is one recolor pass per tick while a fade is
    active — confirm the visual against DOSBox and docket the rate if it
    reads differently.
11. **Walldef block 0 pairing** (§1.11 item 8): GBE's base-100 special case
    vs coab's unconditional `×10` — check whether WALLDEF block 0 is ever
    multi-wallset in real CotAB data. **Answered (M2 step 1):** across all
    six `WALLDEF{2..6}.DAX` files in the real CotAB data set, block id `0`
    never appears at all (observed ids: 1-4, 8-14, 16-17). The GBE/coab
    contradiction is moot for this data set — LOAD FILES' `0x7F` ->
    `LoadWalldef(1, 0)` is a live code path but no shipped CotAB block
    exercises it. Multi-wallset blocks do exist among the non-zero ids
    (block 14 in `WALLDEF5.DAX` and block 17 in `WALLDEF6.DAX`, both 2
    wallsets/1560 bytes) — the general multi-wallset path is real and
    covered, just not at id 0.

## 5. What this unblocks (M2 build order)

1. `gbx-formats`: image/font/walldef/animated-pic decoders + `GameData`
   (goldens vs GBE/daxdump on real data, H1).
2. `gbx-engine`: framebuffer + primitives + frames + text system (hash
   goldens over fixtures).
3. Widgets + input queue; walk-loop flows over a stub VM host (fixture GEO,
   no scripts) — walk a synthetic map.
4. `VmHost` implementation binding the real `EclMachine`; vector flows;
   Effects/Requests wired to the text system and widgets (micro-ECL
   conformance, H2).
5. 3D renderer (fixture walldefs → hash goldens; then real data vs DOSBox).
6. `frontends/desktop`, then `frontends/web`; `restrike walk` trace driver.
7. `tools/inspect` v0.
8. Exit-gate captures + docket updates.
