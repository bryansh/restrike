# Copy-Protection (the CotAB code wheel) — reference for M6

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

## Open items for M6 implementation

- The exact rune-index origin/direction on the wheel rim (where index 0 sits,
  CW vs CCW) and the path-symbol → `code_path` (0/1/2) ordering are **not**
  captured above — Simeon's interactive tool encoded them in clickable rune
  images we did not extract. Pin both against a live prompt (or the DOSBox
  oracle at M4) before shipping "answer shown", or the overlay will be
  confidently wrong.
- Row-0 length must be validated to 36 on transcription (the web fetch showed
  35; likely a copy artifact).
- Faithful-optional per D4: default the prompt to authentic (player answers);
  the shown-answer overlay is an opt-in QoL toggle.
