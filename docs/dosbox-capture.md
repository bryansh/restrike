# DOSBox Capture Procedure (M2 exit gate, D-UI7)

> Companion to `docs/design/renderer-ui-shell.md` D-UI7 ("Tilverton renders
> vs DOSBox") and the fidelity docket. This doc is the human-executable
> half of the M2 step 8 exit gate: everything here is a DOSBox side-by-side
> capture or a short keyboard-only behavioral check, each settling one
> docket item. Captures and any transcript of what the game prints are
> **local-only artifacts** (PLAN.md D10: no game text/art ever committed) —
> save them anywhere outside this repo (e.g. `~/goldbox-data/captures/`,
> `~/goldbox-data/expected/`).
>
> **Automation attempt (this session):** `dosbox-staging` launches cleanly
> headlessly (`nohup dosbox-staging -conf ... -c "mount c ..." -c "c:" -c
> "start" &`; confirmed running via `osascript`'s System Events query), but
> `screencapture` fails with "could not create image from display" in this
> environment — no attached/authorized display session for Screen
> Recording, so neither a screenshot nor (by extension) verified keystroke
> automation was possible. Consistent with a prior session's note (the M2
> step 7 audit needed the release build launched on Bryan's own display for
> visual verification). Process was cleanly terminated after the check.
> Everything below is therefore queued for a human at a real display.

## 1. DOSBox Staging setup for raw, unscaled captures

This machine has dosbox-staging 0.82.2 installed via Homebrew
(`~/Library/Preferences/DOSBox/dosbox-staging.conf`). Two settings matter
for pixel-comparable captures:

```ini
[capture]
capture_dir                   = <somewhere under ~/goldbox-data/captures>
default_image_capture_formats = raw
```

`raw` is the format that matters: dosbox-staging's `[capture]` section
documents three capture formats — `upscaled` (bilinear-sharp + aspect
correction, the default), `rendered` (post-shader, whatever's on screen),
and **`raw`** ("the contents of the raw framebuffer is captured — this
always results in square pixels", filenames end `-raw.png`). Only `raw`
gives a 320×200 image directly comparable to our own `restrike walk
--dump-at`/`dump-image` PPM output without a rescale step. Set
`default_image_capture_formats = raw` (or pass multiple formats
space-separated to also keep `upscaled` for eyeballing) before capturing.

**Screenshot hotkey**: dosbox-staging's default is **Ctrl+F5** (verify
against the in-app "Special Keys" list, accessible from the DOSBox Staging
menu, if it doesn't fire — key bindings are user-remappable via
`--startmapper`). Each press writes a numbered `imageNNNN-raw.png` into
`capture_dir`.

**Boot command** (per PLAN.md's M0 checklist, unchanged):

```
dosbox-staging -c "mount c ~/goldbox-data/cotab" -c "c:" -c "start"
```

**Ready-made combo-session config** (written 2026-07-14; lives with the
user's data, never in the repo): `~/goldbox-data/restrike-capture.conf`
pre-sets `capture_dir = ~/goldbox-data/captures`, raw+upscaled capture
formats, and an autoexec that mounts and boots the game. One command runs
the whole §7 session:

```
dosbox-staging -conf ~/goldbox-data/restrike-capture.conf
```

## 2. Comparing a capture against our own output

```
restrike walk ~/goldbox-data/cotab --trace <TRACE> --dump-at <TICK> --out-dir /tmp
restrike compare /tmp/restrike-walk-dump-<TICK>.ppm ~/goldbox-data/captures/imageNNNN-raw.png --diff-out /tmp/diff.png
```

`compare` requires identical dimensions (hence `raw` capture format above)
and reports differing-pixel count/percent, mean absolute channel diff, and
max channel diff; `--diff-out` writes a same-size image with differing
pixels painted red over a dimmed grayscale copy of the original — a quick
visual triage before deciding a divergence is real. Initial bar per D-UI7:
**structural match** (same walls in same cells, same text layout), not
pixel-exact — palette/rounding divergences are expected and get their own
docket entries, not silent "fixes" on either side.

## 3. Pinned spot squares (D-UI7's six categories)

All coordinates are `(x, y)` in `restrike map`'s grid (matches
`GEO2.DAX` block 1, Tilverton City half, columns 0–7); facings are
N/E/S/W. Party spawns at **(7,13) facing East**. Steps use the movement
keys below (§4). Real wall/door data for the spawn neighborhood (from this
session's own GEO read, `gbx_formats::geo::GeoBlock`) backs every pick;
if a square's actual on-screen framing doesn't clearly show its category
once you arrive, step to an adjacent square and use `restrike map` to find
a better one nearby — hitting the category matters more than the exact
coordinate.

| # | Category | Square | Facing | Steps from spawn |
|---|---|---|---|---|
| 1 | Open street (walls open on all four sides) | (4,11) | any | W, N, W, W, N (see §4 key sequence — this is the circuit's own leftward corridor, three squares past (6,12)) |
| 2 | Wall left+right corridor | (5,13) | East | W, S, W (three solid edges — N/S/W — with only the East edge open; steps into a hallway-like view) |
| 3 | Door ahead, **open** (state 1) | (6,13) | North | W (one step from spawn) |
| 4 | Door ahead, **locked** (state 2) | (7,12) | North | W, N, E (the circuit's own route to the tavern district; this is also docket FD-19's "side door" — approaching it opens the Bash/Pick/Knock/Exit menu, which doubles as spot #6) |
| 5 | Door ahead, **solid/no door** (state 0) | (7,13) [spawn] | North | none — the spawn square's own North edge |
| 6 | Area-map view | any (e.g. spawn) | — | press `A` at the world menu |
| 7 | Event text mid-pagination | (6,12) or (5,10) | — | walk the circuit (§4); screenshot *before* pressing the key that clears a "press any key"/"press button or return" prompt |
| 8 | Menu open | (5,10) tavern, or (7,12) door menu | — | walk the circuit to the tavern's option menu, or approach the locked door at (7,12) |

Door state 3 (unpickable) was not found among the squares surveyed this
session near spawn — skip it, or widen the search with `restrike map` if
desired; not required for the exit gate's own bar.

## 4. Walk-circuit key sequence (matches `fixtures/tilverton-circuit.jsonl`)

Reproduces the exact route `restrike walk` replays headlessly, so a human
DOSBox session and our own transcript describe the same walk. Movement
keys: **arrow keys** (Up = step forward, Left/Right = turn 90°, Down =
turn 180° in place — matches this engine's `ExtKey` mapping, D-UI6; if
arrows don't register in DOSBox, try the numpad with Num Lock off: 8/4/6/2
in the same roles). Press **Enter** (or Space) whenever text pauses for a
keypress — the amnesia-scene boot text pages twice before the world menu
appears.

1. Boot; press Enter through the two opening text pages.
2. At the world menu (spawn, facing East): **Down** (turn to face West),
   **Up** (step to (6,13) — text may appear; Enter through it).
3. **Right** (face North), **Up** (step to (6,12) — text may appear;
   Enter through it).
4. **Left** (face West), **Up** (step to (5,12)).
5. **Right** (face North), **Up** (step to (5,11)).
6. **Up** again, no turn (step to (5,10), the tavern — a menu and
   possibly a combat-stub prompt follow; pick the highlighted/first option
   each time by pressing Enter, matching this session's fixed "mash
   Enter" replay policy).
7. Return leg: **Down** (face South), **Up** (step to (5,11) or wherever
   the tavern scene's own scripted reposition left the party — see
   `docs/fidelity-docket.md` FD-19's sibling note in the M2 step 8 commit;
   this leg is deterministic in our engine but hasn't been cross-checked
   against DOSBox yet, which is exactly what this capture settles).
8. **Up** (no turn, step further south).
9. **Left** (face East), **Up**.
10. **Right** (face South), **Up**.
11. **Left** (face East), **Up** — back at spawn (7,13).

Record what actually happens at each step (position, facing, any text) —
if the DOSBox route diverges from the above (e.g. the tavern scene
repositions the party somewhere our engine didn't predict), that's a real
finding, not a mistake in these instructions; note it for FD-19 or a new
docket entry.

## 5. Human checklist — DOSBox-only fidelity checks

Each item is a 30–90 second keyboard check that settles a named docket
entry. No screenshots needed unless noted.

- [ ] **FD-17 (type-ahead / drain-to-last)**: during a moment of slow
  redraw or while text is actively printing, mash the forward arrow 5
  times rapidly. Count how many squares the party actually moves once the
  dust settles. **1 step** confirms coab's drain-to-last semantics
  (`GetInputKey` keeps only the newest key, docketed in D-UI1); **5 steps**
  falsifies it (type-ahead is real in the original binary) and the input
  queue's read semantics need to swap.
- [ ] **FD-18 (list-menu arrow keys)**: open any vertical/list menu (a
  shop or training-hall list once reachable, or any `VERTICAL MENU`
  opcode's output) and press Up/Down. coab's source says these are
  **ignored** (only Home/End/PgUp/PgDn move the highlight) — confirm
  whether Up/Down do anything in the real game.
- [ ] **Docket item 12 (exact-fit line wrap)**: find or engineer a line of
  event text that wraps exactly at the text window's right edge, ending a
  word in a trailing space right at the boundary. Note whether the
  trailing space is trimmed (our engine's `>=`-bound interpretation) or
  left in (the literal decompiled `>`-bound reading) — screenshot if
  either way is ambiguous by eye.
- [ ] **§1.11 item 1 (backdrop band colors)**: face an open outdoor
  direction and screenshot the sky/horizon band composition (raw capture).
  Compare against `corridor.rs`'s black-band + gray-8 fill — coab's own
  structure, not ssi-engine's flat-color alternative (already ruled
  unlikely, but this is the actual pixel confirmation).
- [ ] **§1.11 item 7 (J-filler texture at adjacent differing far fronts)**:
  find two adjacent far-front wall cells of *different* wall types (a
  street corner where materials change, e.g. near (6,10)/(6,11)'s
  transition) and screenshot facing down that sightline. Compare the
  filler texture between them against `corridor.rs`'s "previous front's
  type" rule (coab's own documented behavior) vs. ssi-engine's
  scan-order-earlier alternative.
- [ ] **Sun/moon hour windows** (design doc §1.7, item 5 in the open-items
  list): deferred unless trivially observable on this circuit — Tilverton
  City's spawn area is mostly indoor/urban-canyon per the walls surveyed;
  only attempt this if a clearly outdoor, sky-visible square turns up
  along the way. Not required for the exit gate.

## 6. What settles the exit gate vs. what stays queued

Per PLAN.md's M2 exit gate (updated in the same session as this doc):
the headless circuit (fixtures/tilverton-circuit.jsonl), its stable
checkpoint hashes, empty halt records, and the local engine-generated
transcript are **proven by this session's own tests** — no DOSBox
involvement needed for those. What *does* need a human at DOSBox:

1. The spot-square screenshots (§3) compared via `restrike compare`.
2. An `~/goldbox-data/expected/tilverton-circuit.transcript` file (one
   line per event, same format `restrike walk --transcript` writes —
   see `frontends/cli/src/walk.rs`'s `expected_transcript` test for the
   exact convention) so the automated comparison test in this repo can
   run for real instead of skipping.
3. The checklist in §5.

None of these block M2's other deliverables; they're the honestly-labeled
"awaits the human checklist" portion of the exit gate.

## 7. The combo session (added at M3 step 4): one sitting clears everything

Doing §3–§5 in the same DOSBox session that creates the M3 import save makes
the save itself evidence. Ordered for a single boot (~45–60 min):

1. **Boot capture-ready** (§1's launch line + capture dir).
2. **Party prep** (character creation / the modify option): include an
   18/xx exceptional-strength fighter (pins the Str00 range cell,
   save-formats.md §1.7 item 5) and at least one cleric and one magic-user.
3. **FD-17 type-ahead** (§5): on a long street, mash forward 5–6× during
   the redraw; count committed steps.
4. **The walk circuit** (§4): screenshots at the §3 spot squares; note the
   event text for the expected transcript
   (`~/goldbox-data/expected/tilverton-circuit.transcript`).
5. **A shop**: FD-18 (Up/Down arrows in the list menu — do they move the
   highlight?); BUY something and note the price paid (known money delta =
   import evidence).
6. **Training hall**: second FD-18 data point.
7. **Camp**: memorize a KNOWN spell set (write the exact counts, e.g.
   "cleric: 2×Bless 1×CLW; MU: 1×Sleep 1×MM") and rest — pins the
   spellCastCount stride cell (§1.7 item 2). Screenshot each character
   sheet (permanent DOSBox-side record for the D-SAVE10 tier-3
   field-by-field comparison, incl. current/max stat pairs for the
   byte-order cell — item 1 pins fully whenever a stat is ever drained).
8. **One combat**: try to open the game menu mid-fight (save-formats.md
   §5.1 — expected: unreachable). Flee or win, either way.
9. **Backdrop bands / J-filler / exact-fit wrap** (§5 items): opportunistic
   observations while walking, per their §5 entries.
10. **SAVE to slot A, quit.** The `savgam?.dat`/`CHRDAT*` files land in the
    mounted `~/goldbox-data/cotab` automatically. Then:
    `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine -- import`
    should light up the local tier, and `restrike compare` takes the
    screenshots.

Deliverables back to the repo: capture PNGs + the transcript under
`~/goldbox-data/` (never committed, D10); the noted answers (FD-17/18,
mid-combat menu, §5 items) reported for docket updates; the save files in
place.
