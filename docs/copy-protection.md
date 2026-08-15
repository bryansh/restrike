# Copy-Protection (the CotAB code wheel)

> ★ **IMPLEMENTED, roll-credits slice 9b (D-RC4).** The algorithm and table
> live in `crates/gbx-engine/src/copy_wheel.rs`, transcribed from **coab**
> (`ovr004.cs:7-111`) rather than from the web page below, and the prompt is on
> screen in `crates/gbx-engine/src/front_door.rs` with the answer shown. Both
> open items at the foot of this file are **resolved** — see §"Resolved" — and
> row 0 is confirmed 36 characters. The rest of the page is kept as the
> provenance record it was written to be.


PLAN.md M6 lists "copy-protection prompt neutralized (answer shown,
faithful-optional)" as a task. The original `START.EXE` (`ovr004.cs
copy_protection()` in coab) shows two runes — an Espruar (elvish) and a Dethek
(dwarvish) — plus a box number and a path symbol, and demands the letter the
physical *translation wheel* reveals at that box/path once the two runes are
aligned. coab gates it behind `Cheats.skip_copy_protection`; our engine reads
the data files directly and never runs this prompt at all, but M6's
"answer shown" QoL wants us to be able to *compute* the answer (e.g. an
optional overlay that displays it) faithfully.

Simeon Pilgrim (author of coab, our primary reference) reverse-engineered the
wheel and published the algorithm + lookup table
(<https://simeonpilgrim.com/blog/2007/11/01/curse-of-the-azure-bonds-code-wheel-copy-protection/>).
Recorded here so M6 is a transcription, not a rediscovery — read-for-behavior
per D11, cited.

## The algorithm

```js
// espruar, dethek : rune index 0..35 (position on the wheel rim; the key row
//                   below maps index -> its A..Z,1..9,0 label)
// code_path       : 0,1,2  (the three spiral paths: dotted / dash-dot / dashed)
// code_row        : 0..5   (the box number, 1..6, minus 1)
function calc(espruar, dethek, code_path, code_row) {
  let code_index = espruar + 0x22 - dethek + (code_path * 12) + ((5 - code_row) << 1);
  while (code_index > 35) code_index -= 36;
  while (code_index < 0)  code_index += 36;
  const index = code_row * 36 + code_index;
  return CODE_WHEEL[index];               // one character
}
```

## The lookup table (6 rows × 36 chars, row-major)

```
row 0: CWLNRTESSCEDCSHSISERRRNSHSSTSSNNHSHN   (35 shown here; verify length 36 at impl)
row 1: LAASRDAIILIDSUGADAEEOEGRLSELIITESOIO
row 2: LRUNIMMORIIGRRIUPTIIUELIMLHMIXACGRIL
row 3: Z0LIOHEUVNODSGEOGXYWISIOCRARLRARRHOI
row 4: AMTELRLUIYNAEOOITOUELRREREUIMADPPFAB
row 5: ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890   (the key: index -> rune label)
```

Row 5 labels the 36 rune positions (`A`..`Z`,`1`..`9`,`0`), so a rune's index
is the position of its label in that string.

## Resolved (slice 9b, 2026-08-15)

- **Row 0 is 36 characters.** The web fetch showed 35 — a copy artifact.
  coab's `ovr004.codeWheel` row 0 is
  `CWLNRTESSCEDCSHSISERRRNSHSSTSSNNHSHN`, 36 long, as are all six rows;
  `copy_wheel::tests::every_wheel_row_is_thirty_six_characters` pins it.
- **There is no wheel geometry to pin.** The old worry — "where does rune
  index 0 sit on the rim, CW or CCW, and which path symbol is `code_path`
  0/1/2" — does not exist inside the program. The runes are **tile indices**:
  `Load24x24Set(0x1A, 0, 1, "tiles")` loads the 26 Espruar runes from
  `TILES.DAX` block 1 into 24×24 cells `0..26` and
  `Load24x24Set(0x16, 0x1A, 2, "tiles")` the 22 Dethek runes from block 2 into
  cells `0x1A..0x30` (`ovr004.cs:22-23`); the prompt then draws
  `DrawIsoTile(var_6, 3, 0x11)` and `DrawIsoTile(var_7 + 0x1A, 7, 0x11)`
  (`:38-39`) — the very indices the arithmetic consumes. The real data agrees:
  `TILES.DAX` block 1 decodes to 26 items, block 2 to 22, matching
  `Random(26)`/`Random(22)` exactly. `code_path` is likewise just `Random(3)`,
  and the same value picks the path string in the switch at `:44-61`. So an
  engine that draws the same tiles computes the same answer *by construction*
  — there is nothing to calibrate against a live prompt.
- **Row 5 is both the key row and an answer row.** `code_row` is `Random(6)`,
  so box number 1 (`6 - 5`) reads its answer out of `ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890`.
- **The neutralization** (D-RC4, decided): the prompt is faithful — same
  frame, same three instruction lines, same two runes, same box/path sentence,
  same `type character and press return: ` editor — and the answer is
  **pre-filled into the input line**, so `<Enter>` accepts it and a player who
  owns the wheel can backspace and answer for themselves. Nothing about the
  challenge is softened: a wrong answer still says "Sorry, that's incorrect.",
  re-rolls all four values, and the third failure still ends the session
  (`ovr004.cs:102-110`). Chosen over printing the answer beside the prompt
  because it leaves the screen byte-faithful and puts the QoL entirely in the
  one place the original leaves empty.
- **Faithful-optional per D4**: `RESTRIKE_COPY_PROTECTION=faithful` leaves the
  input line empty (the player answers). The default is the shown answer.

## The one shipped `PROTECTION` opcode site

`ECL1` block `0x50` (the overland loop), subroutine `L9A92` — the sixth
journey's bridge keeper, `GOSUB`'d from the journey encounter at `@0x9A84`:

```
0x9A92: COMPARE [0x4CA3], #0x06     ; the every-6th-journey counter
0x9A98: IF <>  -> ADD #1,[0x4CA3] -> [0x4CA3]
0x9AA2: IF <>  -> RETURN
0x9AA4: SAVE #0x00 -> [0x4CA3]      ; sixth journey: reset and run
0x9AAA: PRINTCLEAR "YOUR WAY IS BLOCKED BY AN IMPASSABLE CHASM. A "
0x9AD0: PRINT      "NARROW BRIDGE IS GUARDED BY AN OLD MAN. HE CACKLES, '"
0x9AFB: PRINT      "YOU MUST ANSWER ME BEFORE THE OTHER SIDE YE SEE.'"
0x9B23: GOSUB 0x9B85                ; "PRESS BUTTON OR RETURN TO CONTINUE."
0x9B27: PRINTCLEAR "WHAT IS YOUR QUEST?"
0x9B39: INPUT STRING #0x2D -> str[0x7B00]
0x9B3F: PRINTCLEAR "WHAT IS YOUR FAVORITE FRUIT?"
0x9B57: INPUT STRING #0x2D -> str[0x7B00]
0x9B5D: PRINTCLEAR "WHAT DOES THIS MEAN?"
0x9B6F: PROTECTION [0x7F79]         ; <- the copy wheel
0x9B73: PRINTCLEAR "YOU MAY PASS."
0x9B80: GOSUB 0x9B85
0x9B84: RETURN
```

The two `INPUT STRING`s are answered but never read — Monty Python, not
mechanics. `PROTECTION`'s operand (`[0x7F79]`, this block's scratch cell) is
decoded by `vm_LoadCmdSets(1)` and never looked at (`ovr003.cs:1990-2004`),
which settles **FD-12**: the operand is vestigial, at the only site that
exists.
