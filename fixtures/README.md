# fixtures/

Synthetic, hand-authored test data only. Everything in this directory ships
with the repo and public CI, so **nothing here may resemble or contain real
Gold Box game data** (see PLAN.md D10) — no real DAX/ECL/GEO bytes, no game
text, art, or tables copied from an original title.

Each subdirectory is a self-contained fixture with its own short README
explaining what it's for and how it was constructed. The CI no-game-data
guard scans this directory (along with the rest of the tree) for known game
file signatures/names and fails the build if any are found.

Real-data testing (golden comparisons, full parser conformance, oracle trace
equality) runs locally against a user-supplied data directory via
`GBX_DATA_DIR` — never from files checked into this repo.
