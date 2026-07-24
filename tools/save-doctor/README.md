# save-doctor — clone-and-teleport staging tool (operator-side, Python)

Staging aids for placing the party anywhere in CotAB without playing there:
clone a save slot and patch its location fields so the game loads the party
into a chosen ECL/GEO block. Built 2026-07-24 to open the Tilverton sewers
(new monster rosters for captures) from the pre-carriage sandbox save.

All offsets are from `docs/design/save-formats.md` §1.1/§1.4 and were
validated against a live slot-B save before first use (`savedump.py`). The
scripts read `~/goldbox-data` at runtime and contain no game bytes (D10).

- `daxdump.py` — DAX index + RLE decode (mirrors `gbx_formats::dax`).
- `ecl_strings.py` — 6-bit ECL text scan per block (area identification;
  found: ECL2 #1=city #2=guild #3=sewers #4=hideout).
- `georender.py` — ASCII wall/passability render of a GEO block (position picking).
- `savedump.py` — field dump of a `savgam?.dat` (offset validation).
- `doctor.py` — the doctor: clone slot SRC→DST, set
  `current_3DMap_block_id`/`LastEclBlockId`/position (Area1 + section-6, kept
  consistent), zero the `field_200` per-block quest flags (fresh-entry
  semantics — the load path fires the entry vector), rewrite the `CHRDAT`
  slot letters, copy sibling character files. Edit the constants at the top.

Known-unvalidated (iterate live): wallset `setBlocks` are carried from the
source save — if the target block's entry vector doesn't reload wall art,
walls render with the source area's set until we patch them; the drop square
comes from `georender` + the cluebook and may need a nudge.
