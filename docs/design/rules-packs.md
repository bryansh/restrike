# Design: Rules Packs & the Flavor-Trait Seam

> M3 architecture pass per PLAN.md §9 operating rule 3 (one design review before
> each one-way door). Status: **v2, draft for review.** v1 (2026-07-13) was
> built from a full inventory of every rules table transliterated in coab plus
> a live anchoring experiment against the real CotAB v1.3 `START.EXE`, then
> subjected the same day to one bounded adversarial round (two independent
> reviewers, fresh context). Both returned NEEDS-V2 with verified findings, the
> largest of which **corrected the experiment's own interpretation**: v1 read
> compression artifacts as "dead table cells" — the binary's data image is in
> fact **EXEPACK-compressed**, proven by implementing the decoder and finding
> coab's tables verbatim in the decompressed image (see §1.2). v2 therefore
> replaces v1's masked-row-search verification with decompress-then-compare,
> extends the schema to record/jagged/N-axis tables (v1 could not express four
> tables on its own M3 shipping list), reclassifies the door-bash retrofit
> (code, not data — v1 violated its own boundary rule), and folds ~a dozen
> smaller corrections. Every folded finding was re-verified against coab or the
> real binary before editing. Review stops here per the one-round bound;
> remaining risk is carried by authoring-time cross-checks (D-RP7) and the
> oracle rungs.
>
> Scope: `gbx-rules` — the pack format, verification, typed access, and the
> flavor-trait seam — plus a small `gbx-formats` addition (the EXEPACK
> decoder). Out of scope: the party/character model (M3 implementation), save
> formats (separate M3 design), combat mechanics content (M4 authors against
> this schema).

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
(`ovr006.cs:8–145`), the door-bash chains (`ovr015.cs:49–224` — v1 misfiled
these as a stored table; they are if/else code and stay code), and every spell
effect magnitude (`ovr013.cs`/`ovr023.cs` handlers).

**Stored tables (Group A — pack material), by cluster.** Dimensions below are
coab's *transliterated* shapes; the decompressed binary image is authoritative
for actual layout and is confirmed per table during authoring (§2 D-RP7 —
known divergences already found: `con_hp_adj` is 23 entries indexed `[CON−3]`
in the image vs coab's zero-padded `sbyte[26]`; the thief base table appears
depadded; `SaveThrowValues` appears without coab's synthesized level-0
column). An **evidence tier** accompanies each table at authoring: `image`
(located in the decompressed binary), `coab-only` (no binary location found —
e.g. `RaceClasses`, which has no original-address comment and may be
coab-reconstructed), recorded in the pack's `[meta]`.

| Cluster | Tables | coab anchors |
|---|---|---|
| Progression | XP thresholds (`exp_table`, ovr018.cs:2179, `unk_1A5A3` — **element proven i32le** in the image: 1501/3001/6001/13001 found contiguously; druid/monk `-1` rows are coab normalization, confirm during authoring); to-hit progression (`thac0_table`, ovr018.cs:313, `unk_1A14A` — stored values `0x27..0x33`; **display THAC0 = `0x3C − stored`**, `Player.cs:599`, consumed via max() at ovr018.cs:578/ovr026.cs:192 and in `PC_CanHitTarget`'s `roll + bonus >= target_ac`, ovr024.cs:515–545; v1's "base 50" claim was unsupported and is retracted); saving throws (`SaveThrowValues`, ovr026.cs:323, order Poison/Petrification/Wand/Breath/Spell per `Spells.cs:7`); turn undead (ovr014.cs:603, cleric-level brackets per :626–637) |
| HP / hit dice | max hit dice per class (`Gbl.cs:273`), HD count at level 1 (ovr018.cs:2122), hit-die size (ovr018.cs:2123), `hp_calc` params (ovr018.cs:2078), CON HP adjustment (ovr018.cs:1980, image-indexed `[CON−3]`) |
| Spell slots | four **incremental** gain tables — cleric/paladin/ranger (ovr026.cs:8/22/37) and magic-user (ovr020.cs:627); accumulated in code (cleric's `skillLevel−2` loop bound, paladin-from-9, ranger's druid/MU column split, ovr026.cs:57–140) |
| Spell metadata | `SpellEntry[0x66]` (`Gbl.cs:567`, `asc_19AEC`; ~0xce5d region of the raw file) — 16 heterogeneous byte-wide numeric fields per record (`Spells.cs:153–203`); **contains no name strings** (display names are NOT pack material; their M5 source is an open question, §5) |
| Creation legality | class alignments (`Gbl.cs:801`, `unk_1A4EA` — the §1.2 flagship); race→classes (`Gbl.cs:276`, jagged, coab-only tier); class stat minimums (`Gbl.cs:701`, `unk_1A484`); race/sex stat min-max, 7 × 3-axis `[8,2,2]` (`Limits.cs:28–96`); race age brackets + aging deltas (`Limits.cs:9–25`); starting-age dice by race×class (`Gbl.cs:724`, `unk_1A35E` — 8×7 **records** `{base:u16, dice_count:u8, dice_size:u8}`, mixed widths proven: bases 0xfa/0x28a/0xc08) |
| Thief skills | base by level (ovr026.cs:465, `unk_1A1D0`); race adj (ovr026.cs:426); DEX adj (ovr026.cs:441) — consumed by `reclac_thief_skills` ovr026.cs:482 |
| Constants | coin values (`MoneySet.cs:19`), time scales (ovr021.cs:8), class item/training bitmasks (ovr018.cs:308/2124), paladin-cureable diseases (ovr020.cs:1497) |

**Computed in code (Group B — flavor-trait material, NOT packs):** everything
in the first paragraph, plus multiple-attack schedules (`ovr026.cs:196–257`),
backstab multiplier (`ovr014.cs:96`), level-up HP roll-twice-take-higher
(`ovr018.cs:2145–2151`), post-name-level fixed HP, creation rolls (best-of-6
of `3d6+1`, ovr018.cs:675–683), starting XP/money/spellbook, dual-class
eligibility (`ovr026.cs:558–599`), camp memorization timing (`ovr016.cs:8–64`),
CON≥20 regeneration (`ovr024.cs:1110–1118`), and the door-bash chains. These
are **reimplemented as cited trait methods**; re-tabularizing them from the
PHB would invite rulebook-vs-engine drift — the engine, not the book, is the
fidelity target. (Bonus evidence, banked for the docket: `ovr024.cs:487–545`
settles FD-1's coab side — natural 20 promotes the roll to 100, natural 1
misses, on both attack paths — pending H4 oracle confirmation.)

**Loaded from game data (Group C — never pack material):** the ITEMS
weapon/armor table (`seg001.cs:323`, 0x81 × 16-byte records, `ItemData.cs:43–56`
— SSI shipped it as a file, so it is game content read at runtime like any
DAX), monster stat records (`MON*` files, including the XP-award fields), and
all script/map/art content. The boundary rule: **packs hold only what the
original bakes into its executable as data**; file-shipped content stays
user-supplied, and code-shaped mechanics stay code.

### 1.2 The anchoring experiment, corrected: the binary is EXEPACK-compressed

D6 requires verifying pack tables against the user's binary. The v1 experiment
searched the raw `START.EXE` for coab's `class_alignments` and found 15 of 17
rows with unexplained "junk" and a 2-byte drift — which v1 misread as dead
cells needing masked comparison. The adversarial round diagnosed the truth
(escape sequences, not junk), and direct verification settled it:

- `START.EXE` is **EXEPACK-packed** (`RB` signature at `0xdcc0`, zero MZ
  relocations, 18-byte EXEPACK header; the data image decompresses backwards
  via the documented `0xB0`-fill / `0xB2`-copy opcode stream).
- A 20-line decoder produces exactly `dest_len` (`0xf3e0`) bytes, and the
  decompressed image contains **coab's tables verbatim**: the full
  `class_alignments` matrix — clean paladin row, no junk, no drift — at image
  offset `0xedba`, and the cleric XP run (1501/3001/6001/13001 as i32le) at
  `0xee7b`. coab transliterated the *runtime* image; of course the packed
  file differs.

Conclusions baked into the design: verification operates on the
**decompressed image**, where byte-exact comparison is possible and masks are
unnecessary; the decompressor is a small, cited `gbx-formats` module (the
format is public knowledge, and our own experiment is the provenance);
raw-file search survives only as a fallback tier; and the decompression
strategy is a **per-binary property recorded with the detection entry** —
M7's Buck Rogers binaries may pack differently or not at all.

## 2. Decisions

**D-RP1 — Packs are TOML files, embedded, one file per cluster.**
`crates/gbx-rules/packs/<flavor>/<cluster>.toml` (`adnd1` now, `xxvc` at M7),
embedded via `include_str!` (no filesystem, wasm-clean, versioned with the
code), parsed once into typed structs at `RuleSet::load()`. TOML because
evidence citations live as comments next to the rows they justify; JSON has
no comments, and Rust constants can't be uniformly schema-validated or
authored by non-programmers (M9's catalog work is table authoring). Because
packs and parser always ship in the same binary, there is **no cross-version
pack-loading problem and no migration story needed** — a `schema_version`
field is reserved in `[meta]` solely against a future where packs become
user-suppliable. `RuleSet::load()` is **infallible-or-panic** (a malformed
embedded pack is a shipped bug that D-RP7's CI must have caught);
diagnostics belong to verification only.

**D-RP2 — Packs store the runtime image's encoding verbatim; ergonomics live
in accessors.** Incremental spell-slot deltas stay incremental; the to-hit
table stays in its stored convention (with the `0x3C −` display relation
documented in `[meta]`); `-1` sentinels stay `-1`; image-true indexing (e.g.
`[CON−3]`) is recorded in `axes` rather than re-padded. Where coab's
transliteration normalizes (padding rows/columns that don't exist in the
image), **the image wins for shape and coab for semantics** — confirmed per
table during authoring via the extract cross-check. One deliberate carve-out:
tables at `coab-only` evidence tier obviously follow coab's shape until an
image location is found. Typed accessors (`xp_to_train(class, level)`,
`slots_gained(...)`) own all conversions, with conformance tests reproducing
coab's consumption loops.

**D-RP3 — Every table carries a `[meta]` block:** `id`, `flavor`,
`schema_version`, `description`, named `axes` (sizes and index meanings;
`size = "var"` marks a jagged axis, with its length rule stated), element
typing (table-level `element`, or per-column via `columns` — D-RP3a),
`evidence_tier`, `source` citations (coab file:line, original-address,
original-re with experiment date, manual pages), `anchor` (D-RP4), `notes`.
A pack without sources fails CI.

**D-RP3a — Three data shapes, declared explicitly** (v1 supported only the
first and could not express four tables on its own M3 list):
- `rows` — rectangular N-axis numeric data, one `element`; the flattening
  rule is fixed: **the first axis indexes rows; remaining axes are row-major
  within each row** (a `[8,2,2]` table is 8 rows of 4 values, order
  `[min/max][sex]` as declared). CI's dims check applies per-axis.
- `records` — struct-shaped tables: `columns = [{ name, element, meaning,
  range? }]` declares per-field width, signedness, and validation domain;
  rows are tuples in column order (this is `race_ages` now and the M5
  `SpellEntry[0x66]` later — the schema expresses it today, no M5 revision).
- `jagged` — variable-length rows (`RaceClasses`): the axis's length rule
  (`count-prefix` or `explicit`) is declared; anchoring of jagged tables is
  per-row or `none`.

**D-RP4 — Verify-on-load = decompress, then compare (the D6 protocol).**
`gbx-formats` gains `exepack.rs` (decode `START.EXE`-style packed images;
cited to the public format + our §1.2 experiment). At engine boot,
`RuleSet::verify(&GameData)` decompresses the relevant binaries once
(~60 KB, single-digit ms — **deliberately every boot**, cheaper than any
cache and immune to staleness) and checks each anchored table byte-exact:
- `anchor = { kind = "image", file = "START.EXE", offset = 0x…, len = … }` —
  offset recorded at authoring time by the extract tool; boot verifies by
  comparison at the offset, falling back to a full-image search (status
  `Moved { found_at }`) so patch-level layout shifts are visible, not fatal.
- `kind = "raw"` — for any table found in an unpacked region or file.
- `kind = "none"` — `coab-only` tier; verification deferred to the oracle
  rungs (H2/H4), reason stated in `notes`.
Statuses: `Verified`, `Moved`, `NotFound`, `BinaryAbsent { file }` (a data
dir can pass detection without `START.EXE` — this must be distinguishable
from version skew), `Unanchored`. The report is **advisory only** (the pack
stays authoritative — D6's "warn, never silently diverge"), surfaced at boot
diagnostics, the `restrike verify` CLI subcommand (PLAN §2 reserved it), and
an inspector pane later; it is ephemeral and **never serialized into saves**.
Duplicate-content ambiguity (e.g. thac0's identical cleric/druid/monk rows)
is a non-issue under offset-anchored comparison.

**D-RP5 — The flavor-trait seam (D7).** `gbx-rules` defines the flavor
trait; `adnd1` implements it consuming the loaded `RuleSet` plus cited quirk
code for all Group-B mechanics. M3's slice: creation legality (stat
minima/maxima, race/class/alignment), stat rolling, starting age/money/XP,
HP determination (incl. roll-twice-take-higher), XP-to-train and level-up,
thief skill recalculation, spell-slot accumulation, CON adjustment, door
bashing (staying code, per §1.1). Combat-facing methods (to-hit, saves, turn
undead) get their table plumbing now and their roll semantics at M4 against
oracle traces. `xxvc` (M7) gets its own pack directory and trait impl; the
trait speaks in engine terms ("may this character enter this class", "hp
gained on level-up"), never AD&D vocabulary, outside the `adnd1` impl.

**D-RP6 — Per-title overlays (M9 allowance), whole-table only.** A detected
title may supply `packs/<flavor>/overrides/<title>/<cluster>.toml` replacing
whole tables by `id`. No cell-level merging — complexity with no customer.

**D-RP7 — Validation is two-tier, and anchored means Verified.** In-repo CI:
every pack parses (strict serde, unknown fields error), dims/columns match
declared axes, values within declared ranges/domains, cluster invariants
(XP rows monotone where not `-1`, alignment ids < 9, save values plausible
d20 targets). Local tier (GBX_DATA_DIR): **every `image`/`raw`-anchored
table must report `Verified` against real v1.3 data; any other status fails
the test unless a docket entry is cited in the pack's `notes`** — pinning
"whatever was measured" would normalize authoring errors (a wrong element
width reads as NotFound and must not become an expected status). The
extract tool assists diagnosis: on NotFound it re-sweeps under alternate
element widths and reports likely misdeclarations.

**D-RP8 — Retrofits and non-retrofits.** The EGA palette stays a code
constant (presentation data, not mechanics — no anchor machinery). The
door-bash logic stays code entirely (`bash_door.rs` as today, v1's TOML
retrofit withdrawn — it is if/else in the original and tabularizing it
would *invent* an encoding, the exact D-RP2 risk). The pack loader's first
real test case is `class_alignments` — experimentally anchored, clean shape,
known offset.

**D-RP9 — M3 ships these packs** (authoring order): progression (XP,
to-hit, saves, turn undead), HP/HD cluster, creation-legality cluster
(records + jagged shapes exercise D-RP3a immediately), thief skills, spell
slots, constants. Spell metadata is *expressible today* via `records`
(D-RP3a) but authored at M5 with its consumers. Authoring tactic:
`restrike extract-table` (CLI, local-only) decompresses the image, locates
candidate tables, and pre-fills/cross-checks hand-transcription from coab —
extracted mechanics numbers are uncopyrightable facts (PLAN §6.2), the
double-sourcing (coab read + image bytes agreeing) is recorded in `source`,
and the tool never emits strings (the schema cannot hold them by
construction).

**Dependency note:** `gbx-rules` gains `serde`/`toml` and (for `verify`) an
edge to `gbx-formats` — acyclic (formats knows nothing of rules), and both
wasm-clean.

## 3. The format, by example

```toml
# packs/adnd1/creation.toml

[[table]]                              # shape: rows (rectangular)
id = "class_alignments"
schema_version = 1
flavor = "adnd1"
description = "Allowed alignments per class; row = count-prefixed id list"
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
  { name = "base_age",   element = "u16le", meaning = "years" },
  { name = "dice_count", element = "u8" },
  { name = "dice_size",  element = "u8" },
]
source = [{ kind = "coab", loc = "Classes/Gbl.cs:724 (unk_1A35E)" }]
evidence_tier = "image"                # confirmed at authoring
anchor = { kind = "image", file = "START.EXE", offset = 0x0, len = 0 }  # filled by extract-table
rows = [
  # race 0 (monster), classes 0..6:
  [6, 2, 6], [0x0c08, 0xe, 0], [0, 0, 0], # ...
]
```

Strict serde into `RawTable { meta, rows }` → shape/domain validation →
typed wrappers per cluster. `RuleSet::load()` panics on malformed embedded
packs (CI-caught); `RuleSet::verify(&GameData) -> VerifyReport` implements
D-RP4.

## 4. Testing

- **In-repo:** D-RP7 schema/invariant CI over all packs; typed-accessor
  conformance (delta accumulation reproduces coab's loops incl. cleric's
  `skillLevel−2` bound and ranger's column split; `0x3C −` display relation;
  `-1` handling; `[CON−3]` indexing); trait-method conformance for every
  Group-B quirk, each citing its coab source; an `exepack.rs` unit suite
  (synthetic packed streams: fill/copy/final-bit/pad edge cases).
- **Local (GBX_DATA_DIR):** decompression produces `dest_len` exactly;
  the D-RP7 all-anchored-tables-Verified assertion; extract-table
  cross-check for each authored table.
- **Oracle rungs:** Group-B trait methods and `anchor = none` tables get H2
  micro-scenarios where cheap, H4 trace parity at M4 — tracked per item in
  the fidelity docket.

## 5. Open questions → fidelity docket

1. EXEPACK decoder edge cases: `skip_len` semantics, pad handling, and
   locating the in-binary decompressor stub to pin grammar edges — settle
   during `exepack.rs` implementation against the real file (the
   whole-image `dest_len` check already passes).
2. Per-table image-shape confirmations during authoring (con_hp_adj's
   23-vs-26, thief base depadding, SaveThrowValues' level-0 column, XP's
   `-1` rows and row interleaving with slot tables) — each recorded in the
   pack's `axes`/`notes` as authored.
3. `RaceClasses` evidence tier: search the image during authoring; if
   absent, it ships `coab-only`/`anchor = none` with an H2 scenario.
4. Spell display names are not pack material; their M5 source (engine
   constants with citations, like frame tables? read from the user's binary
   at runtime?) is an M5 design question flagged now.
5. FD-1 (nat 20/nat 1): coab-side answer banked (§1.1); H4 confirms at M4.
6. Buck Rogers binaries' packing (M7): decompression strategy recorded per
   detection entry (D-RP4); expect a new experiment then.

## 6. What this unblocks (M3 build order)

1. `gbx-formats::exepack` + `gbx-rules` pack loader + schema validation +
   the verify engine (first test: `class_alignments` end-to-end — parse,
   validate, decompress, Verified);
2. `restrike extract-table` + authoring of the D-RP9 clusters
   (extract-assisted, double-sourced);
3. the `adnd1` trait skeleton + creation-legality methods; then
4. `gbx-engine`'s party/character model consuming the trait (separate M3
   design), training-hall/level-up flows;
5. M4 inherits progression-table plumbing with roll semantics
   oracle-verified.
