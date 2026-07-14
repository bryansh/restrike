# Design: Rules Packs & the Flavor-Trait Seam

> M3 architecture pass per PLAN.md §9 operating rule 3 (one design review before
> each one-way door). Status: **v3, draft for review.** v1 (2026-07-13) was
> built from a full coab table inventory plus a live anchoring experiment
> against the real CotAB v1.3 `START.EXE`; round 1 (two adversarial reviewers)
> corrected the experiment itself — the binary is **EXEPACK-compressed** (v1
> misread compression escapes as dead cells; confirmed by implementing the
> decoder and finding coab's tables verbatim in the decompressed image) — and
> found v1's schema unable to express four tables on its own M3 list. v2 folded
> those. A second bounded round scoped to v2's new surface produced v3: no
> architectural changes, but real contract holes — coab's turn-undead array is
> a **truncated prefix** the engine indexes past (byte-exact prefix
> verification would have passed the truncated pack — hence the new
> index-domain-coverage rule), the exepack contract omitted the raw-prefix /
> `src == dst` invariant (a demonstrably wrong decoder passed every v2
> acceptance check), and D-RP7 had three independent green-paths (vacuous
> `len = 0` anchors, silent local-tier skips, extract-table verifying its own
> output). All findings re-verified against coab or the binary before folding.
> Review stops here per the bounded-round doctrine; remaining risk is carried
> by the (now hole-patched) D-RP7 gates and the oracle rungs.
>
> Scope: `gbx-rules` — pack format, verification, typed access, flavor-trait
> seam — plus `gbx-formats::exepack`. Out of scope: the party/character model
> (M3 implementation), save formats (separate M3 design), combat mechanics
> content (M4 authors against this schema).

## 1. Evidence

### 1.1 What the original actually stores vs computes

A complete sweep of coab found **23 stored rules tables** and a large body of
**computed-in-code mechanics**. The split is the single most design-relevant
fact: SSI did *not* ship the AD&D rulebook as data. The famous PHB
ability-modifier tables (STR hit/damage including the 18/xx exceptional bands,
DEX AC/missile, CON bonuses, WIS bonus slots) exist only as if/else chains
(`ovr025.cs:498–760`, `ovr018.cs:894–929`, `ovr026.cs:291–423`), as do race
level limits (`Limits.cs:106–290` — even stat-dependent, untabularizable),
to-hit and saving-throw resolution (`ovr024.cs:487–583`), XP awards
(`ovr006.cs:8–145`), the door-bash chains (`ovr015.cs:49–224` — code, stays
code), and every spell effect magnitude (`ovr013.cs`/`ovr023.cs` handlers).

**Stored tables (Group A — pack material), by cluster.** Dimensions below are
coab's *transliterated* shapes; the decompressed binary image is authoritative
for actual layout (D-RP2), confirmed per table during authoring. Known
divergences to resolve at authoring (docket §5.2): `con_hp_adj` is 23 entries
indexed `[CON−3]` vs coab's zero-padded 26; the thief cluster's three tables
have provably overlapping coab extents (the `unk_1A1D0`/`unk_1A230`/`unk_1A243`
address spacing cannot fit all three declared shapes — image resolves);
`SaveThrowValues` appears without coab's synthesized level-0 column; the XP
table's leading-0 column and `-1` rows are coab normalization. Each table
records an **evidence tier**: `image` (located in the decompressed binary) or
`coab-only` (no image location found — e.g. `RaceClasses`, no original-address
comment, plausibly coab-reconstructed) — the tier must correspond to the
anchor kind (`image`/`raw` ⟺ `image` tier; `none` ⟺ `coab-only`), enforced by
CI.

| Cluster | Tables | coab anchors |
|---|---|---|
| Progression | XP thresholds (`exp_table`, ovr018.cs:2179, `unk_1A5A3` — **element proven i32le** in the image: 1501/3001/6001/13001 contiguous); to-hit progression (`thac0_table`, ovr018.cs:313, `unk_1A14A` — stored `0x27..0x33`; **display THAC0 = `0x3C − stored`**, `Player.cs:599`; consumed via max() ovr018.cs:578/ovr026.cs:192 and `PC_CanHitTarget`'s `roll + bonus >= target_ac`, ovr024.cs:515–545); saving throws (`SaveThrowValues`, ovr026.cs:323, order per `Spells.cs:7`); **turn undead — coab's 21-byte array is a truncated prefix**: the consumer indexes `[undead_type * 10 + cleric_bracket]` with undead type from monster data `field_76` accepted up to 12 (`ovr014.cs:642, 699–714`; `ovr017.cs:286`), so the real table is ~13 × 10 and its extent **must be established from the image, not coab** (docket §5.1; coab's two rows cover types 0–1 only — its type-1 row is exactly the DMG skeleton row) |
| HP / hit dice | max hit dice per class (`Gbl.cs:273`), HD count at level 1 (ovr018.cs:2122), hit-die size (ovr018.cs:2123), `hp_calc` 4-field records (ovr018.cs:2069–2087), CON HP adjustment (ovr018.cs:1980, image-indexed `[CON−3]`) |
| Spell slots | four **incremental** gain tables — cleric/paladin/ranger (ovr026.cs:8/22/37) and magic-user (ovr020.cs:627); accumulation quirks are code (cleric's `skillLevel−2` bound, paladin-from-9, ranger's druid/MU column split, ovr026.cs:57–140) |
| Spell metadata | `SpellEntry[0x66]` (`Gbl.cs:567`, `asc_19AEC`) — 16 heterogeneous byte-wide numeric fields per record (`Spells.cs:153–203`), **record 0 physically present but undefined** (coab transliterates it as `null` — the dead-record convention in D-RP3a applies); **no name strings** (display names are NOT pack material; M5 source is an open question, §5) |
| Creation legality | class alignments (`Gbl.cs:801`, `unk_1A4EA` — the §1.2 flagship); race→classes (`Gbl.cs:276`, jagged, `coab-only` tier, and coab carries a 9th "Cheaters" row beyond the 8 races that the pack must exclude — race axis is 8); class stat minimums (`Gbl.cs:701`, `unk_1A484`, field order STR/INT/WIS/DEX/CON/CHA); race/sex stat min-max, 7 × 3-axis `[8,2,2]` (`Limits.cs:28–96`, consumed `[race, min/max, sex]` at `Player.cs:47–51`; **no original-address comments — prospectively `coab-only`**, so D-RP7's independent-fixture rule applies); race age brackets + aging deltas (`Limits.cs:9–25`); starting-age dice by race×class (`Gbl.cs:724`, `unk_1A35E` — 8×7 records `{base:u16, dice_count:u8, dice_size:u8}`, mixed widths proven: 0xfa/0x28a/0xc08) |
| Thief skills | base by level (ovr026.cs:465, `unk_1A1D0`); race adj (ovr026.cs:426); DEX adj (ovr026.cs:441) — consumed by `reclac_thief_skills` ovr026.cs:482; all three shapes image-resolved at authoring (see divergence note above) |
| Constants | coin values (`MoneySet.cs:19` — no address comment, may prove `coab-only` or reclassify to code if the image shows immediates); time scales (ovr021.cs:8); class item/training bitmasks (ovr018.cs:308/2124 — `bitmask(8)` domain); paladin-cureable diseases (ovr020.cs:1497, 6 affect ids). **Ruling:** the gem = 250 / jewelry = 2200 XP constants are code inside `GetExpWorth` (`MoneySet.cs:59–66`) — Group-B trait constants, not pack rows |

**Computed in code (Group B — flavor-trait material, NOT packs):** everything
in the first paragraph, plus multiple-attack schedules (`ovr026.cs:196–257`),
backstab multiplier (`ovr014.cs:96`), level-up HP roll-twice-take-higher
(`ovr018.cs:2145–2151`), post-name-level fixed HP, creation rolls (best-of-6
of `3d6+1`, ovr018.cs:675–683), starting XP/money/spellbook, dual-class
eligibility (`ovr026.cs:558–599`), camp memorization timing (`ovr016.cs:8–64`),
CON≥20 regeneration (`ovr024.cs:1110–1118`), the door-bash chains, and the
gem/jewelry XP constants. These are **reimplemented as cited trait methods**;
re-tabularizing them from the PHB would invite rulebook-vs-engine drift — the
engine, not the book, is the fidelity target. (Banked for the docket:
`ovr024.cs:487–545` settles FD-1's coab side — natural 20 promotes the roll to
100, natural 1 misses, both attack paths — pending H4 confirmation.)

**Loaded from game data (Group C — never pack material):** the ITEMS
weapon/armor table (`seg001.cs:323`, 0x81 × 16-byte records, `ItemData.cs:43–56`
— SSI shipped it as a file, so it is game content read at runtime), monster
stat records (`MON*` files, including the XP-award fields and the turn-undead
type byte `field_76`), and all script/map/art content. The boundary rule:
**packs hold only what the original bakes into its executable as data**;
file-shipped content stays user-supplied, and code-shaped mechanics stay code.

### 1.2 The anchoring experiment, corrected: the binary is EXEPACK-compressed

The v1 experiment searched the raw `START.EXE` for coab's `class_alignments`
and found 15 of 17 rows with unexplained "junk" and a 2-byte drift — misread
as dead cells. Round 1 diagnosed compression; direct verification settled it:

- `START.EXE` is **EXEPACK-packed**: `RB` signature at `0xdcc0`, zero MZ
  relocations, 18-byte EXEPACK header at `0xdcb0`; `dest_len` is `0xf3e`
  **paragraphs** = `0xf3e0` bytes.
- The decode contract (normative, not an edge case): the packed stream ends
  in trailing `0xff` pad bytes (8 of them on v1.3) which are skipped before
  the first backwards read; opcodes decode back-to-front (`0xB0`-fill /
  `0xB2`-copy, low bit = final); the output image is **the raw prefix
  `packed[..src]` kept in place** (877 bytes on v1.3) **plus the
  backwards-decoded tail**, and the decoder must hard-error unless
  `src == dst` when the final-bit opcode lands — that invariant is the
  stream's only cheap integrity check, and a decoder that ignores the raw
  prefix passes naive length/table spot-checks while silently corrupting the
  low image (proven during review). `skip_len != 1` is a hard error until a
  binary that uses it appears.
- The decompressed image contains **coab's tables verbatim**: the full
  `class_alignments` matrix — clean paladin row — at image offset `0xedba`,
  and the cleric XP run (1501/3001/6001/13001 i32le) at `0xee7b`. coab
  transliterated the *runtime* image.

Conclusions baked in: verification operates on the decompressed image by
byte-exact comparison; the decompressor is a small cited `gbx-formats` module
**treated as an untrusted-input parser** (it runs on user-supplied bytes every
boot — it joins the fuzz roster per PLAN M1's convention); raw-file
comparison survives for tables in unpacked regions/files (`anchor.kind =
"raw"`); and the decompression strategy is a **per-detection-entry property**
— M7's Buck Rogers binaries may pack differently or not at all.

## 2. Decisions

**D-RP1 — Packs are TOML files, embedded, one file per cluster.**
`crates/gbx-rules/packs/<flavor>/<cluster>.toml` (`adnd1` now, `xxvc` at M7),
embedded via `include_str!`, parsed once into typed structs at
`RuleSet::load()`. TOML because evidence citations live as comments next to
the rows they justify. Packs and parser ship in the same binary — no
cross-version loading, no migration story; `schema_version` is reserved in
`[meta]` solely against future user-suppliable packs. `RuleSet::load()` is
**infallible-or-panic** (a malformed embedded pack is a shipped bug CI must
catch); diagnostics belong to verification only.

**D-RP2 — Packs store the runtime image's encoding verbatim; ergonomics live
in accessors.** Incremental deltas stay incremental; stored conventions stay
stored (with display relations documented in `[meta]`); `-1` sentinels stay;
image-true indexing (`[CON−3]`) is recorded in `axes`. Where coab normalizes
(padding, synthesized rows/columns, truncation), **the image wins for shape
and coab for semantics**, confirmed per table at authoring. `coab-only`
tables follow coab's shape until an image location is found. Typed accessors
own all conversions, with conformance tests reproducing coab's consumption
loops.

**D-RP3 — Every table carries a `[meta]` block:** `id`, `flavor`,
`schema_version`, `description`, named `axes` (sizes, index meanings, and any
index offset like `[CON−3]`; a physically-present-but-undefined record — the
SpellEntry record 0 — is declared here and the anchor skips it), element
typing (table-level `element` or per-column `columns`), `evidence_tier`,
`source` citations, **`consumed_by` (the coab call site(s) that index this
table — required, see D-RP7's domain rule)**, `anchor`, `notes`. A pack
without sources or consumption cites fails CI.

**D-RP3a — Three data shapes, declared explicitly:**
- `rows` — rectangular N-axis numeric data, one `element`. Flattening rule:
  the first axis indexes rows; remaining axes are row-major within each row.
  **Declared axis order is the image's storage order** — for anchored tables
  the byte comparison pins it; for `coab-only` multi-axis tables a
  transposition is invisible to CI, so D-RP7 requires independently
  transcribed value fixtures (e.g. "dwarf-female STR max = 17,
  Limits.cs:30"). Count-prefixed rows at **fixed stride** (class_alignments:
  17 × 10 bytes) are `rows`, not jagged — jagged is only for physically
  varying stride in the image.
- `records` — struct-shaped tables: `columns = [{ name, element, meaning,
  domain? }]` with per-field width and signedness. The `domain` grammar:
  `"min..=max"` (inclusive), `"set(0,1,2,4)"` (non-contiguous enums — e.g.
  `SpellTargets`, `Spells.cs:23–29`), `"bitmask(n)"` (the class-flag
  constants). D-RP7 validates values against domains in CI.
- `jagged` — variable-length rows (`RaceClasses`): the length rule
  (`count-prefix` or `explicit`) is declared; anchoring is per-row (subject
  to D-RP4's uniqueness threshold) or `none`.

**D-RP4 — Verify-on-load = decompress, then compare (the D6 protocol).**
`gbx-formats::exepack` implements §1.2's contract. At engine boot,
`RuleSet::verify(&GameData)` decompresses the relevant binaries once
(~61 KB, sub-millisecond — **deliberately every boot**) and checks each
anchored table byte-exact:
- `anchor = { kind = "image", file, offset, len }` — offset recorded at
  authoring; boot verifies by comparison at the offset, falling back to a
  full-image search. `Moved { found_at: Vec<offset> }` reports **all** hits
  of the full table-length pattern; per-row anchors (jagged) below a
  16-byte uniqueness threshold are forbidden (use `none`). A `Moved` finding
  is actionable, not noise: it means a new binary version — file a
  detection-table entry and re-anchor at the next authoring pass. **Anchors
  are keyed to the detection entry** (per-version offsets; per-title anchors
  ride the D-RP6 override mechanism); CI's v1.3 pin is a property of the
  reference detection entry.
- `kind = "raw"` — unpacked regions/files, same semantics.
- `kind = "none"` — `coab-only` tier; oracle rungs verify (H2/H4).
Statuses: `Verified`, `Moved`, `NotFound`, `BinaryAbsent { file }` (present
detection, missing binary — distinct from version skew),
`ImageUndecodable { file, reason }` (present but not decodable: wrong/corrupt
packing, `src != dst`, `skip_len != 1` — distinct from both), `Unanchored`.
On `Detection::Unknown` or a detection entry with no decompression strategy,
verification is skipped with an explicit `NotAttempted` report line — never
silence. The report is **advisory only** (the pack stays authoritative —
D6's "warn, never silently diverge"). Plumbing: `verify` runs immediately
after `Engine::new`'s asset loads, never blocks or fails boot; the
`VerifyReport` is retained on the `Engine` behind a getter for boot
diagnostics, the `restrike verify` CLI subcommand (PLAN §2 reserved it), and
the inspector; the web frontend logs it to the console until a real surface
exists. Reports are ephemeral — **never serialized into saves**.

**D-RP5 — The flavor-trait seam (D7).** `gbx-rules` defines the flavor
trait; `adnd1` implements it consuming the loaded `RuleSet` plus cited quirk
code for all Group-B mechanics. M3's slice: creation legality, stat rolling,
starting age/money/XP, HP determination (incl. roll-twice-take-higher),
XP-to-train and level-up, thief skill recalculation, spell-slot
accumulation, CON adjustment, door bashing (code). Combat-facing methods
(to-hit, saves, turn undead) get table plumbing now, roll semantics at M4
against oracle traces. `xxvc` (M7) gets its own pack directory and trait
impl; the trait speaks engine terms, never AD&D vocabulary, outside the
`adnd1` impl.

**D-RP6 — Per-title overlays (M9 allowance), whole-table only.** A detected
title may supply `packs/<flavor>/overrides/<title>/<cluster>.toml` replacing
whole tables by `id` (anchors included). No cell-level merging.

**D-RP7 — Validation is two-tier, with the holes patched.**
- **In-repo CI:** every pack parses (strict serde); dims/columns match
  declared axes; values within declared ranges/domains; cluster invariants
  (XP rows monotone where not `-1`, alignment ids < 9, plausible d20
  targets); **anchor well-formedness**: `image`/`raw` anchors require
  `len > 0` **and** `len == product(axes) × element width` (records: row
  count × summed column widths) — vacuous placeholder anchors and element
  width misdeclarations die in CI, before real data is ever involved;
  **tier ⟺ anchor-kind consistency**; **index-domain coverage**: every
  table's `consumed_by` cite is present, and declared axis sizes must cover
  the consuming code's index domain (the turn-undead lesson: a byte-exact
  prefix of a bigger table verifies perfectly and then panics in M4).
- **Local tier (GBX_DATA_DIR):** every `image`/`raw`-anchored table must
  report `Verified` against the reference detection entry's data; any other
  status fails unless a docket entry is cited in the pack's `notes`. The
  local test emits a **loud skip marker** when GBX_DATA_DIR is absent, and
  pack-authoring changes merge only with local-tier evidence recorded in
  the commit message (the repo's established evidence-annotation pattern);
  PLAN §9.4's overnight-grind gate explicitly includes this for pack work.
- **Extraction is confirmatory, never originating:** for `image`-tier
  tables, row values are transcribed from (or diffed against) coab's
  transliteration; `restrike extract-table` pre-fills offsets and
  cross-checks bytes but an `original-re` citation **alone** is
  insufficient sourcing — the coab leg is normative (D11 posture). On
  `NotFound` the tool re-sweeps under alternate element widths and reports
  likely misdeclarations.

**D-RP8 — Retrofits and non-retrofits.** The EGA palette stays a code
constant (presentation, not mechanics). Door-bash logic stays code entirely
(v1's TOML retrofit withdrawn — the original is if/else; tabularizing would
*invent* an encoding). The pack loader's first real test case is
`class_alignments` — experimentally anchored, clean shape, known offset.

**D-RP9 — M3 ships these packs** (authoring order): progression (XP, to-hit,
saves, turn undead — the latter with its image-established extent), HP/HD,
creation legality (records + jagged shapes exercised immediately), thief
skills (image-shape resolution first), spell slots, constants. Spell
metadata is expressible today via `records` + the dead-record convention,
authored at M5 with its consumers. Authoring flow per table: transcribe from
coab → extract-table locates + cross-checks against the image → divergences
resolved per D-RP2 (image wins for shape) with the resolution recorded in
`axes`/`notes` → local tier pins `Verified`.

**Dependency note:** `gbx-rules` gains `serde`/`toml` and (for `verify`) an
edge to `gbx-formats` — acyclic, wasm-clean.

## 3. The format, by example

```toml
# packs/adnd1/creation.toml

[[table]]                              # shape: rows (rectangular)
id = "class_alignments"
schema_version = 1
flavor = "adnd1"
description = "Allowed alignments per class; row = count-prefixed id list (fixed stride)"
element = "u8"
axes = [
  { name = "class", size = 17, index = "ClassId order, Enums.cs:69" },
  { name = "slot", size = 10, index = "[0]=count, [1..=count]=alignment ids per ovr020.cs:23" },
]
evidence_tier = "image"
source = [
  { kind = "coab", loc = "Classes/Gbl.cs:801" },
  { kind = "original-address", loc = "unk_1A4EA" },
  { kind = "original-re", loc = "START.EXE exepack image @0xedba (v1.3), byte-exact, 2026-07-13" },
]
consumed_by = ["ovr018.cs:590-618", "ovr026.cs:583"]
anchor = { kind = "image", file = "START.EXE", offset = 0xedba, len = 170 }
rows = [
  [9, 0,1,2,3,4,5,6,7,8],   # cleric: any alignment
  [5, 1,3,4,5,7, 0,0,0,0],  # druid: the five neutral-touching ids
  # ... one commented row per class ...
]

[[table]]                              # shape: records (mixed widths)
id = "starting_age"
description = "Starting-age dice by race x class"
axes = [
  { name = "race", size = 8, index = "RaceId order, Enums.cs:45" },
  { name = "class", size = 7, index = "base classes, ClassId 0..6" },
]
columns = [
  { name = "base_age",   element = "u16le", meaning = "years", domain = "0..=5000" },
  { name = "dice_count", element = "u8", domain = "0..=20" },
  { name = "dice_size",  element = "u8", domain = "0..=20" },
]
source = [{ kind = "coab", loc = "Classes/Gbl.cs:724 (unk_1A35E)" }]
consumed_by = ["ovr018.cs:624-650"]
evidence_tier = "image"                # set when extract-table confirms
anchor = { kind = "image", file = "START.EXE", offset = 0xBEEF, len = 224 }  # real values at authoring; len>0 enforced by CI
rows = [
  [6, 2, 6], [0x0c08, 0xe, 0], [0, 0, 0], # race 0, classes 0..6 ...
]
```

Strict serde into `RawTable { meta, rows }` → shape/domain/anchor/domain-
coverage validation → typed wrappers per cluster. `RuleSet::load()` panics on
malformed embedded packs (CI-caught); `RuleSet::verify(&GameData) ->
VerifyReport` implements D-RP4.

## 4. Testing

- **In-repo:** the D-RP7 CI suite (schema, invariants, anchor
  well-formedness, tier consistency, domain coverage); typed-accessor
  conformance (delta accumulation reproduces coab's loops, `0x3C −` display
  relation, `-1` handling, `[CON−3]` indexing); trait-method conformance for
  every Group-B quirk, each citing its coab source; independently
  transcribed value fixtures for `coab-only` multi-axis tables; an
  `exepack.rs` unit suite — synthetic streams covering fill/copy/final-bit,
  **nonzero raw prefix**, `src != dst` → error, trailing-pad skip,
  `skip_len != 1` → error — plus a cargo-fuzz target (untrusted-input
  parser, PLAN M1 convention).
- **Local (GBX_DATA_DIR, loud-skip when absent):** decompression produces
  `dest_len` paragraphs exactly with the `src == dst` invariant; the
  all-anchored-tables-`Verified` assertion; extract-table cross-check for
  each authored table.
- **Oracle rungs:** Group-B trait methods and `anchor = none` tables get H2
  micro-scenarios where cheap, H4 trace parity at M4 — tracked per item in
  the fidelity docket.

## 5. Open questions → fidelity docket

1. **RESOLVED (2026-07-14, M3 step 2).** Turn-undead extent: the image's
   real extent is 11 rows (undead types 0-10), **not** ~13 as this
   hypothesis assumed — type 11's would-be row is ASCII menu text in the
   image, not table data. `packs/adnd1/progression.toml`'s `turn_undead`
   table and its accessor encode exactly this (11×10, a 1-byte dead
   header excluded from the anchor). Whether any *shipped monster* actually
   carries an undead type the table can't represent is still open —
   tracked as `docs/fidelity-docket.md` FD-20, deferred to M4 (monster
   data loading).
2. **RESOLVED (2026-07-14, M3 step 2), per table:**
   - `con_hp_adj`: image confirms 23 entries (`[CON-3]`), no leading
     padding — as hypothesized.
   - **All three thief tables:** the address-spacing conflict was real.
     `thief_skill_base_chance` (base) is 12×8, image-confirmed, but its
     thief levels 6-11 diverge from a naive per-row reading of coab's
     declaration in a way this session didn't fully explain (docket
     FD-21). `thief_skill_dex_adj` is 22×5 (coab's declared column 5
     absent, not column 0 as guessed), byte-verified across all 22 rows.
     `thief_skill_race_adj` (race adj) does **not** fit anywhere in the
     confirmed gap between the other two — actively disproven at coab's
     declared shape, ships `coab-only` verbatim (docket FD-22).
   - `SaveThrowValues`: confirmed — the image has no level-0 column,
     stored as a clean contiguous 8×12×5 `u8` block.
   - XP table: confirmed — the leading-0 column is absent from the image
     and each class's real thresholds sit in a separately anchored
     location (not one contiguous 8×13 block); ships as six independent
     per-class tables (druid/monk have none). "Row interleaving with the
     slot tables" was directionally right but imprecise: the true image
     layout interleaves exp-threshold runs with *other* per-class data
     (confirmed adjacent: `save_throw_values`, `cleric_spell_levels`,
     `paladin_spell_levels`, `ranger_spell_levels`,
     `mu_spell_lvl_learn`), not literally the spell-slot tables sharing
     per-class records with exp thresholds.
3. **RESOLVED (2026-07-14, M3 step 2).** `RaceClasses`: searched the image
   (the human row's class-id sequence, with and without a count-prefix
   byte); no clean match found — the only hits were coincidental overlaps
   with the already-anchored `starting_age` table. Ships `coab-only`,
   race axis 8 (monster included, 9th "Cheaters" debug row excluded),
   `explicit` lengths, as this item anticipated.
4. Spell display names are not pack material; their M5 source (engine
   constants with citations? read from the user's binary at runtime?) is an
   M5 design question flagged now.
5. FD-1 (nat 20/nat 1): coab-side answer banked (§1.1); H4 confirms at M4.
6. Buck Rogers binaries' packing (M7): decompression strategy recorded per
   detection entry (D-RP4); expect a new experiment then.
7. Locate the in-binary EXEPACK stub's exact grammar edges if any real
   binary ever trips the `skip_len`/`src != dst` errors (none expected for
   v1.3 — the invariant holds empirically).

## 6. What this unblocks (M3 build order)

1. `gbx-formats::exepack` (contract per §1.2) + `gbx-rules` pack loader +
   schema/anchor validation + the verify engine (first test:
   `class_alignments` end-to-end — parse, validate, decompress, `Verified`);
2. `restrike extract-table` + authoring of the D-RP9 clusters
   (coab-transcribed, extraction-confirmed, local-tier-pinned);
3. the `adnd1` trait skeleton + creation-legality methods; then
4. `gbx-engine`'s party/character model consuming the trait (separate M3
   design), training-hall/level-up flows;
5. M4 inherits progression-table plumbing with roll semantics
   oracle-verified.
