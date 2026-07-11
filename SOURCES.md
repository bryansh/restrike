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
| _(none yet — M0 scaffold only)_ | | | | |

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
