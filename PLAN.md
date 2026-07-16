# Implementation Plan — Restrike

> Written 2026-07-10, following the project brief (BRIEF.md), a prior-art sweep, and a design
> review. This is the working plan: decisions are locked unless a milestone gate forces a revisit.
> **Restrike** (n., numismatics): a coin struck at a later date from the original dies, faithful
> to the first issue — which is precisely what this engine does with the original game data.
> User-facing binary: `restrike`. Internal crates keep the descriptive `gbx-` prefix.

---

## 0. Decisions locked

| # | Decision | Rationale (short) |
|---|----------|-------------------|
| D1 | **Rust**, stable toolchain, cargo workspace | Safe binary parsing + enums/match for the VM + first-class wasm32. |
| D2 | **GPL-3.0-or-later** | ScummVM/GemRB precedent; compatible with farmboy0's ssi-engine (GPL-3) if we ever port logic from it. |
| D3 | **Curse of the Azure Bonds first**, then **Buck Rogers: Countdown to Doomsday**, then **Matrix Cubed** | coab gives CotAB a near-complete behavioral spec; CTD is the earlier, better-documented BR title; MC is mostly a delta on CTD. |
| D4 | **Faithful-first UI** at authentic 320×200, QoL as opt-in toggles | Every screen has a reference screenshot → thousands of small decisions become lookups. Divergences must be deliberate, documented, and default-off. |
| D5 | **Core-owned software framebuffer** (320×200 indexed, palettized in core) | Authentic rendering, deterministic, testable by hash. Frontends only present + scale. No wgpu until a real need appears. |
| D6 | **Rules packs ship in-repo and are verified against the user's data at first run** | Mechanics/tables are uncopyrightable facts; runtime extraction is fragile across binary versions. Verify-and-warn gets fidelity *and* robustness. |
| D7 | **Quirks live in code behind per-flavor traits; no rules meta-DSL** | Tables are data; behavior is code. A generic rules interpreter is the inner-platform trap. |
| D8 | **`tick(input) → frame` core; no blocking loops anywhere in core** | Buys WASM, save-anywhere, headless testing, and replay for free. Retrofitting is brutal. |
| D9 | **Deterministic core**: single seedable PRNG, no wall clock, replayable input traces | Converts oracle validation from statistics to exact trace equality. |
| D10 | **No game data in the repo or CI, ever** | Synthetic fixtures + hash-based goldens in CI; content-level tests run locally against user-supplied data (`GBX_DATA_DIR`). Enforced by .gitignore + a CI guard. |
| D11 | **Reference code is read-for-behavior, not copied** | coab's license is unclear and it transliterates SSI's binary → treat as documentation/oracle only. ssi-engine is GPL-3 (compatible) but prefer reimplementation; any ported logic gets a provenance note in SOURCES.md. |
| D12 | **Public repo from day 1; build in public, no proactive outreach** | GPL + provenance visible from the first commit. No marketing pings to forums/authors — let discovery happen organically. Asking for help when stuck (goldbox.games) is fine; that's help-seeking, not promotion. M6 announcement optional, decided then. |
| D13 | **Claude drives, Bryan reviews** | Claude implements per §9's model mix; Bryan owns one-way-door design approvals, code review, and plays every milestone demo. |
| D14 | **No game data on hand yet; Bryan sources it legally** | Fantasy titles via GOG (FR Archives Collection Two). Buck Rogers is not sold digitally → second-hand originals. Until data lands, work proceeds on synthetic fixtures; real-data gates activate on arrival. |
| D15 | **Name: Restrike** | Distinctive + trademark-neutral; "Gold Box" stays in the tagline for search (GemRB/xoreos pattern) but not in the brand (SNEG uses "Gold Box Classics" commercially; the goldbox-* repo namespace is a graveyard of lookalikes). Free on crates.io; no starred GitHub collisions. Binary `restrike`; crates keep `gbx-`; `GBX_DATA_DIR` unchanged. |

---

## 1. Context: what exists and what role it plays

- **coab** (C#, GitHub) — function-by-function transliteration of the CotAB binary. Role: *the
  behavioral spec* for the fantasy spine, and (instrumented) the primary combat oracle. Likely
  contains the game's exact PRNG. Never copy code; never touch its bundled `Data/*.DAX`.
- **farmboy0/ssi-engine** (Java, GitLab, GPL-3, dormant) — generic loaders + dungeon walking for
  the whole catalog, no combat/party. Role: format cross-check, second reference renderer, day-0
  sanity check that our data files are good.
- **Jzatopa/SSI-Engine-Full-Play-ability** (GitHub) — active agent-driven Matrix Cubed workspace.
  Role: *documentation trove for the Buck Rogers flavor* (DAX block catalogs, ECL disasm, item/
  monster/save decoding, skill tables). Everything marked "candidate" there gets re-verified here.
- **DAXDump/ECLDump, Gold Box Explorer** — reference decoders for golden comparisons.
- **GBC + DOSBox** — dynamic oracle; GBC's ECL monitor covers both Buck Rogers titles (no coab
  equivalent exists for BR). Windows-only → runs in the oracle VM (see M0).
- **Fidelity docket** (`docs/fidelity-docket.md`, created in M1) — running list of behavioral
  hypotheses to settle against the oracle. Seed entries: does a natural 20 auto-hit / natural 1
  auto-miss (brief and Jzatopa's notes disagree — at least one is wrong); exact initiative formula;
  attacks-per-round schedule; sleep/held auto-kill rules; treasure-table behavior; which titles
  actually use TLB.

---

## 2. Architecture

```
restrike/                       # cargo workspace
├── crates/
│   ├── gbx-formats/            # DAX container, ECL blocks, GEO maps, images/walldefs/fonts,
│   │                           #   original save files, game-detection fingerprints
│   ├── gbx-vm/                 # ECL decoder/disassembler + interpreter + ScriptMemory
│   ├── gbx-rules/              # rules packs (data) + flavor traits (adnd1, xxvc) + verify-on-load
│   ├── gbx-engine/             # game state, world sim, combat, magic, UI shell, framebuffer,
│   │                           #   tick API, save/load (ours + original import)
│   └── gbx-oracle/             # trace format, comparators, replay driver for differential tests
├── frontends/
│   ├── cli/                    # headless: dump | disasm | census | map | run-script | replay | verify
│   ├── desktop/                # winit + softbuffer (or pixels) + cpal; presents core framebuffer
│   └── web/                    # wasm-bindgen + canvas; same core
├── tools/
│   └── inspect/                # egui: resource browser, ECL disasm view, VM stepper,
│                               #   ScriptMemory watch, party/state view  ← the GBC-replacement seed
├── fixtures/                   # synthetic, hand-authored DAX/ECL/GEO test data (ships freely)
├── docs/                       # fidelity-docket.md, format notes, SOURCES.md (provenance ledger)
└── PLAN.md / BRIEF.md
```

Core principles (the load-bearing ones):

1. **Pure core.** `gbx-*` crates have zero platform dependencies. Frontends are thin presenters:
   input events in, framebuffer + audio + window title out.
2. **ScriptMemory facade.** ECL operands address game state by raw 16-bit offsets into the
   original data segment (verified in coab: `VmGetMemoryValue(ushort loc)`; concrete example —
   ECL clock ids `0x4BC6..0x4BCC` backing onto words at `0x6A00 + 2n`). The VM therefore reads/
   writes through a facade that maps known addresses ↔ named engine state, **logs every unknown
   access**, and is populated per game generation (seed the map from coab's `Gbl.cs` naming). The
   unknown-access log *is* the discovery backlog.
3. **Detection tables.** Data-driven per-version fingerprints (file names + hashes), because patch
   levels change behavior. Pick one canonical version per game (the GOG build for CotAB); the
   harness may still record traces from others.
4. **Rules packs.** TOML/JSON tables (THAC0 progressions, saves, XP, ability modifiers, weapons,
   spell/skill parameters) checked into `gbx-rules`, cross-referenced to their evidence source, and
   verified against the user's files at first run (warn on mismatch, never silently diverge).
5. **Determinism.** One PRNG owned by the engine, seedable; game time advances only via ticks.
   A session is fully described by (data fingerprint, seed, input trace) → replays byte-identically.
6. **Progress is measured, not felt.** The opcode census (M1) gives % of opcode *uses* handled,
   weighted by actual frequency in shipped scripts, per game. That number is the project dashboard.

---

## 3. The differential harness ladder (cross-cutting, built up over milestones)

- **H1 — Parser goldens (M1):** our DAX/ECL/GEO output vs DAXDump/ECLDump/Gold Box Explorer output
  on the same real files. In-repo: synthetic fixtures + SHA-256s of real-data results. Local-only:
  full content comparisons.
- **H2 — VM conformance (M1–M2):** hand-authored micro-ECL programs with pinned expected behavior;
  plus real scripts replayed with expected text/branch outcomes captured from DOSBox/coab.
- **H3 — Bit-exact PRNG (M4, first task):** recover the RNG from coab (fastest) or the binary
  (Ghidra fallback), reimplement exactly, verify by predicting a seeded DOSBox session.
- **H4 — Combat trace equality (M4–M5):** instrument the oracle (see M0) to emit per-action JSON
  traces (initiative order, to-hit rolls, damage, AI moves); our engine replays the same seed +
  inputs and must match exactly. Divergences either become fixes or documented docket entries.
  *(Superseded by `oracle-rig.md` D-OR5: no permitted oracle tier emits an action trace, so H4 =
  rng-stream equality on the D-OR3 `prng` projection with chain continuity (gate-carrying) +
  RE-gated endstate checkpoints, in the QuickFight-first bootstrap order. The `prng`-profile
  format/comparator/chain-continuity plumbing landed as M4 step 4 — see the M4 section; the
  `action` profile's mechanism exists but its vocabulary waits for combat, and stays advisory.)*
- **H5 — Full-session replay (M6):** recorded input traces for long play segments with state-hash
  checkpoints; run in CI against local data on the dev machine, hashes-only in public CI.

---

## 4. Milestones

Effort unit: one **focused weekend** (~2 days). Estimates are honest ranges, not commitments.

### M0 — Basecamp (1 weekend)
Environment, data, and oracles; no engine code. Per D14, nothing here blocks on data arriving:
scaffold/CI/fixtures proceed immediately; the data-dependent items activate when Bryan lands them.
- Acquire data (Bryan): GOG *Forgotten Realms: The Archives – Collection Two* → extract CotAB
  files (`innoextract` on macOS, or pull from the GOG mac app bundle). Source Buck Rogers
  originals second-hand (not sold anywhere digitally). Fingerprint everything (hashes) on arrival.
- Clone reference repos *outside* this repo (`~/src/goldbox-refs/`): coab, goldboxexplorer,
  ssi-engine (GitLab), Jzatopa workspace; download daxdump/ECL tools from gbc.zorbus.net.
- **Day-0 sanity check:** run farmboy0's ssi-engine (needs JDK 17+; `brew install temurin@21`)
  against our CotAB files — proves the data set is complete and shows correct title/dungeon output.
- Oracle rig: DOSBox Staging natively on macOS for play/screenshots. For GBC + instrumented coab:
  a small Windows VM (UTM), or CrossOver as an experiment. Timeboxed spike: port coab's core
  (non-WinForms classes) to a modern .NET console app for headless traces — if it works, the
  combat oracle later runs natively on the Mac; if not, VM fallback.
- Repo: `git init`, workspace scaffold, LICENSE (GPL-3.0-or-later), SOURCES.md, .gitignore +
  CI guard against game data, GitHub Actions skeleton (mac/linux/windows check + clippy + fmt +
  wasm32 build of an empty core).
- **Exit gate (scaffold half):** public repo live, `cargo test` green, CI green incl. wasm32 and
  the no-game-data guard; `GBX_DATA_DIR` convention documented. **(Data half, on arrival):**
  ssi-engine renders CotAB from our data; files fingerprinted.

### M1 — "It's alive" (1–2 weekends) — the brief's Weekend MVP
- DAX container reader (index + RLE) in `gbx-formats`, developed against synthetic fixtures,
  validated vs DAXDump on real files (H1).
- ECL disassembler for the CotAB dialect (ECLDump + coab as references).
- **Opcode census tool** (`restrike census`): disassemble every script in every owned game → opcode ×
  game frequency matrix (CSV + report). This sizes the VM work, quantifies the Buck Rogers dialect
  delta, and becomes the progress dashboard.
- GEO map parsing + `gbx map` ASCII automap dump.
- ECL VM skeleton in `gbx-vm`: instruction decode, operand types (immediate/address/string),
  ScriptMemory facade with unknown-access logging; implement the ~15–25 most frequent opcodes.
- Start `docs/fidelity-docket.md` with the seed entries from §1.
- 10 minutes of cargo-fuzz on every parser (keep fuzz targets around thereafter).
- **Exit gate (the brief's DoD):** headless CLI opens real CotAB data, lists/decompresses DAX,
  disassembles ECL, executes a real event script (text + a branch), dumps a correct town map as
  ASCII (verified against an in-game map / walkthrough).

### M2 — First steps (2–4 weekends)
Walk around Tilverton, looking right.
- [x] Graphics decode: 8×8 tile compositing, walldefs, EGA palettes, bigpics, fonts (Gold Box
  Explorer + ssi-engine as references). *(step 1: image/anim/font/walldef decoders + `GameData`,
  Fable-verified via a real decoded portrait.)*
- [x] Core framebuffer + faithful renderer: 3D corridor view composition, viewport layout, text
  window, menu bar — the Gold Box UI shell as a state machine (D8: no blocking menus).
  *(steps 2–5: framebuffer/text/widgets/flows, then the real 3D corridor + area map against real
  Tilverton wallsets, Fable-verified visually at all four spawn facings.)*
- [x] Movement/facing/turning; ECL event triggers (enter square, search, look); Parlay-free NPC
  text. *(steps 3–4 wired the walk loop + door interaction against the real `EclMachine`; step 8
  closed the gap that blocked most event text — see below.)*
- [x] `frontends/desktop`: winit + softbuffer window presenting the framebuffer, keyboard input,
  integer scaling. *(step 6.)*
- [x] `tools/inspect` v0 (egui): resource browser, ECL disassembly viewer, VM stepper,
  ScriptMemory watch. *(step 7 + v0.1 polish pass — selection/copy/paste, goto-address, image
  export; Fable-audited, verified end-to-end against real data.)*
- [x] **WASM proof:** the same core in a canvas via `frontends/web`. *(step 6; wasm32 core +
  web-frontend checks green in CI since.)*
- **Exit gate:** walk Tilverton streets/buildings with correct walls, art, and firing events;
  side-by-side spot-check vs DOSBox screenshots; web build walks the same map. Status as of step 8
  (2026-07-13), evidence-annotated per the M0/M1 pattern:
  - [x] **Headless circuit, proven by tests.** `fixtures/tilverton-circuit.jsonl` (inputs + tick
    indices only, no game content, D10) walks a real loop through Tilverton — past the tavern
    district and back to spawn — via `restrike walk`. Checkpoint hashes are stable across
    independent runs (`frontends/cli/tests/walk.rs`'s real-data determinism test, plus this
    session's own two-run diff); **halt records are empty across the whole circuit**; the local
    (uncommitted) transcript shows real event text at multiple distinct squares — an inn sign, a
    tavern scene with a menu/combat-stub/journal-entry chain, and street-tone description text —
    unblocked by this session's arithmetic-family fix (see below). One door was found to trigger a
    real cross-area transition this milestone doesn't implement; routed around rather than forced
    through (`docs/fidelity-docket.md` FD-19).
  - [x] **The blocking gap, closed.** The M2 step 7 field find — Tilverton's per-step script
    halting on every single step at a then-unimplemented DIVIDE (0x06) — is fixed: SUBTRACT/
    DIVIDE/MULTIPLY (0x05–0x07) share coab's `CMD_AddSubDivMulti` shape with the existing ADD;
    DIVIDE's remainder writes through the `ScriptMemory` facade to the confirmed `0x7F3F`
    Party-window alias (FD-9, resolved). The circuit surfaced three more real gaps beyond the
    planned arithmetic family — OR (0x30), ON GOSUB (0x26), and a `SAVE`-with-`mem_str`-
    destination decode bug (`Arg::raw_word()` didn't resolve mode `0x81`) — all fixed with
    citations + conformance tests, not guessed.
  - [ ] **DOSBox spot-check screenshots — awaits the human checklist.** `docs/dosbox-capture.md`
    documents the capture procedure (raw/unscaled dosbox-staging format, `restrike compare`
    tooling), pins eight spot squares across D-UI7's six categories with exact position/facing and
    steps from spawn, and gives the human-executable key sequence matching the committed circuit.
    This session's own automation attempt (`nohup dosbox-staging ...` launched cleanly; keyboard
    input and screenshots blocked — no display/Screen-Recording access in this environment,
    consistent with a prior session's note) confirms this genuinely needs a human at a real
    display; nothing here is a stand-in for that.
  - [ ] **Expected-transcript comparison — awaits the human checklist.** A local-only test
    (`frontends/cli/src/walk.rs`'s `expected_transcript` module) compares the circuit's live
    transcript against a human-maintained `~/goldbox-data/expected/tilverton-circuit.transcript`
    (a DOSBox-side capture of the same walk); it documents the convention and skips gracefully
    until that file exists. Not yet run for real.
  - [ ] **FD-17/FD-18 (type-ahead, list-menu arrows) — open**, each a 30–90 second DOSBox check
    per `docs/dosbox-capture.md` §5; unresolved since M2's design pass.
  - *Deferral note (2026-07-13, decided with Bryan):* the human-checklist items above are
    verification of existing behavior, each isolated behind a single function/constant by design —
    they do not gate M3, which starts now. Deadline: fold the DOSBox hour into M4's oracle-rig
    setup at the latest; do FD-18 earlier if M3's shop/training list menus land first (it is a
    per-widget key-map entry either way).
  - [x] **Web build:** loads the same data and walks the same circuit — manual smoke only (core
    hashes are identical by construction: the same crate compiled to wasm32), documented as a
    manual step since a scripted browser walk is out of this session's scope:
    `cargo build --target wasm32-unknown-unknown -p restrike-web`, serve `frontends/web/`, point it
    at a copy of `GBX_DATA_DIR` via the runtime directory picker, and drive the same key sequence
    as `docs/dosbox-capture.md` §4 by hand.

### M3 — The party assembles (2–3 weekends)
- [x] Character/party model for AD&D flavor; ability/derived-stat tables land in `gbx-rules`
  (evidence-tagged, verify-on-load per D6). *(rules-pack steps 1–3: exepack + pack loader/verify
  engine, D-RP9 clusters, the `adnd1` flavor-trait impl. Step 4: the field-complete party/character
  model itself, `gbx-engine::party`, D-SAVE11.)*
- [x] **Original save import**: read a real CotAB save (party, flags, position) — headless
  pipeline done (`gbx_engine::import::import_original`, D-SAVE5/7/8); synthetic-fixture-proven
  (D-SAVE10 tier 1: section offsets, record decode, window placement, byte-identical round-trip,
  a committed golden hash). *(Not yet done: proving against a real DOSBox save — D-SAVE10 tiers
  2–4 — since no real save exists under `GBX_DATA_DIR` yet; see the exit-gate note below. GBC
  oracle-VM comparison is deferred to M4's oracle rig per the design doc.)*
- [x] Our save format: full engine-state snapshot (versioned; save-anywhere falls out of D8).
  *(`docs/design/save-formats.md` D-SAVE1–4: hand-encoded `ContainerHeader` + `postcard(SaveState)`,
  reject-not-migrate, `Engine::save`/`Engine::restore`, CI-enforced determinism + a committed
  golden `.rsav` hash. Save/load **menu UI** is separate, still open below.)*
- [x] Camp/rest (minus time effects), training hall/leveling, shops/money. *(step 6: camp menu
  `Save View Magic Rest Alter Fix Exit`; training hall — pack-correct `train_player` level-up
  (fee/eligibility/HP/THAC0/spell-caps); shops — `CityShop` Buy with `ItemsValue` price
  arithmetic against the 7-coin money model. Rest's spell-memorize commit and Magic's memorize/
  scribe are M5 (Vancian) per FD-25. Journal entries: DECIDED 2026-07-15 (Bryan + Fable) —
  the engine shows the entry-number pointer exactly as the original did and never displays
  entry text. The text was deliberately print-only (booklet-as-copy-protection, like the code
  wheel); it exists in no data file, so embedding it would mean transcribing and
  redistributing the booklet — a D10 violation. The faithful behavior and the clean behavior
  coincide. QoL later per D4: a journal-log screen tracking *encountered entry numbers*
  (facts, mirroring the booklet's own checkboxes), and optionally letting a user point the
  engine at their own transcription file the same way they supply game data.
  **CLOSED 2026-07-15 (M4 orchestrator audit) — no implementation was required.** The
  entry pointer is not a coded feature at all: it arrives as ordinary script text through
  PRINT (0x11), and the entry *number* is a literal inside the script's own strings.
  Verified against real data on the committed circuit (`fixtures/tilverton-circuit.jsonl`,
  `restrike walk --transcript`): the tavern chain prints the lead-in at tick 316 and the
  entry number at tick 342, correctly, today — a behavior live since M2 step 8 unblocked
  event text. Corroboration: coab (a full transliteration of the binary) contains **zero**
  journal references, so there is no journal subsystem, screen, or opcode to reimplement.
  "Pointer only, never text" therefore holds *by construction* — the booklet text is in no
  data file, so the engine cannot show it. Only the QoL half (encountered-number log,
  user-supplied transcription file) remains, deferred by design per D4.)*
- [x] Character sheet + party screens in the faithful UI; same data visible in the inspector;
  save/load menu. *(step 6: `charsheet` (`playerDisplayFull`) verified against MATHEW's real
  reference capture; `screens` module Shell states for party-view/camp/magic/save-load/training/
  shop; inspector live-engine pane shows each member's `SheetView`; save/load screen emits
  host-fulfilled `SaveLoadRequest`s, slots ↔ `.rsav` via `saveload_fs`.)*
- **Exit gate: PASSED** (2026-07-15, step 6 deliverable 6). The local-only, `GBX_DATA_DIR`-gated
  `gbx_engine::demo::m3_exit_gate` imports GOG's bundled slot-A save → walks Tilverton
  (7,13)→(5,13) → enters a shop and buys an item (MATHEW, inventory 0→1, weight 300→310) →
  trains an eligible character with pack-correct numbers (MATHEW paladin L5→L6, HP 49→62; no
  bundled member has natural XP, so a clearly-marked dev-only hook grants exactly the L5→L6
  threshold 45001 — the *training numbers* stay pack-correct) → `Engine::save` →
  `Engine::restore` → `Engine::save` is **byte-identical (state-hash equality holds)** and the
  trained level survives the round trip. One reproducible command:
  `GBX_DATA_DIR=~/goldbox-data/cotab cargo test -p gbx-engine -- --nocapture m3_exit_gate`.

### M4 — First blood (4–8 weekends) — the bulk
- **H3 first:** bit-exact PRNG + seed control on both sides. *(Recovered 2026-07-15 and
  adversarially re-derived: the binary's `RandNext` is the Borland TP LCG `state*0x08088405+1`,
  state dword `DS:0x47F0`, integer `Random(N)` = `hi16 mod N` (TP 5.x — the review refuted a
  scaled-high-word v1 claim), `Randomize` = DOS clock, called once at boot; no overlay RNG
  copies (29+5 call sites, 1:1 with coab's). Door: `docs/design/oracle-rig.md` **v3** (two adversarial rounds) — D-OR1
  `gbx-prng` + draw-parity contract + `.rsav` v2 bump, D-OR2 pinned dosbox-staging trace-hook
  branch as tier-1 oracle (neither stock debugger is scriptable), D-OR3 canonical `.gbxtrace`
  in `gbx-oracle`, D-OR4 purpose-built-8086-stepper acceptance + one live session, D-OR5 H4 = rng-stream (gate-carrying;
  QuickFight-first bootstrap) + RE-gated endstate checkpoints (supersedes §3 H4's wording). Docket FD-26/27 updated,
  FD-28 filed.)* **Step 1 landed 2026-07-15** (`crates/gbx-prng`: binary-exact
  `next`/`random`, draw-always, no short-circuit; `[0xa55a,0xa5ee)` hash-pin
  local-tier test re-derives from the user's binary; audited per-site migration
  → `oracle-rig.md` §6 ledger, killing the splitmix64 placeholder and the CLI's
  second RNG; the `roll`/`op_random` off-by-one **confirmed and fixed**; fade
  dither moved off the PRNG to a position hash; `.rsav` → v2 + golden recomputed
  atomically; engine seed narrowed to `u32`. Docket FD-29/FD-30 filed.
  **Step 2 landed 2026-07-15** (`crates/gbx-prng/tests/stepper.rs`: a
  purpose-built ~20-opcode real-mode 8086, written to generic ISA semantics
  with no address special-casing and no `gbx-prng` import per D-OR4A, executed
  the real wrapper+`RandNext` bytes and matched `gbx-prng` bit-for-bit over
  10,000 (K,N) pairs — 0 mismatches; the `N==0` draw-always contract confirmed
  *by execution*; teeth tests empirically refuted the v1 scaled-high-word claim
  (first at k=1 n=3: real=1, scaled=0 — a project first), coab's no-draw
  short-circuit, and a wrong multiplier; zero new deps, wasm-clean, pin asserted
  before stepping). FD-26 records the executable confirmation.
  **Step 4 landed 2026-07-15** (`crates/gbx-oracle` + `restrike trace-compare`):
  the D-OR3 trace plumbing — the `.gbxtrace` format with a byte-deterministic
  canonical JSONL writer (integers only, fixed field order → hashable) and a
  liberal parser (ignores the staging hook's extra diagnostic fields); the
  projection comparator (validity gate + `(before, after)`/`(n, result)`
  equality with `caller`/diagnostics excluded, located divergences); the
  **chain-continuity** check that makes the D-OR4-part-B live capture
  self-validating (`after_i == Prng::next(before_i)`, `before_{i+1} == after_i`,
  a post-boot `randomize` = loud finding); the engine `RngSink` seam (trait on
  the engine side, hooked at `EngineRng::random`, zero-cost/inert and
  `#[serde(skip)]` so the `.rsav` golden is provably unchanged) with its
  `gbx-oracle` `TraceCollector`; and a synthetic restrike-vs-restrike SHA-256
  CI golden (D10-clean, generated by running, independently re-derived). The
  `action` profile's mechanism exists; its combat event vocabulary is left for
  step 5. Remaining: D-OR4 part B (one live staging-hook session — Bryan +
  Fable, step 3) and the combat systems (step 5) before H4's rng-stream half
  closes.
- Combat map generation from encounter data; combatant placement.
- Initiative, action economy, movement costs, facing/rear attacks; THAC0 melee + ranged with
  range brackets; damage; attacks-per-round.
- Monster AI reproducing original behavior (coab's QuickFight logic as the spec); morale.
- Status effects tier 1 (sleep, held, poison, unconscious/dying/dead); XP + treasure award.
- **H4:** instrumented-oracle combat traces vs ours, exact match for N seeds × M scripted
  encounters; docket resolves the nat-1/nat-20 question and initiative details with evidence.
- **Exit gate:** a real fixed encounter (e.g., the Tilverton sewers fight) playable interactively
  and headlessly; headless traces match the oracle exactly for at least 10 seeds.

### M5 — Fireball (3–5 weekends)
- Vancian memorization/slots/scribing; casting in and out of combat.
- Per-spell effects for the full CotAB spell list (enumerated from data + manual during this
  milestone); durations, areas, saving throws.
- Status effects tier 2 (level drain, paralysis cures, etc.); scrolls/wands items-in-combat.
- **Exit gate:** every CotAB spell implemented or explicitly stubbed-with-issue; oracle trace
  parity for a curated spell-heavy encounter set.

### M6 — Roll credits (3–6 weekends) — CotAB completable
- Opcode census → 100% of opcodes *used by CotAB* implemented (or consciously no-op'd with
  rationale); overworld travel, random encounters, Parlay dialogue system, traps/locks/secret
  doors, temples, vault, copy-protection prompt neutralized (answer shown, faithful-optional; algorithm+table captured in docs/copy-protection.md).
- **H5:** full-playthrough input trace with checkpoint hashes; runs locally in CI wrapper.
- **Exit gate: finish Curse of the Azure Bonds start-to-end in our engine**, importing a fresh
  party, with the fidelity docket either resolved or documented per item. The "it's real" moment —
  the repo has been public all along (D12); whether to announce anywhere is decided here, not
  presumed.

### M7 — To the stars (6–10 weekends) — Buck Rogers flavor
- Census diff CotAB↔CTD drives the ECL dialect work; BR DAX variants.
- `xxvc` flavor in `gbx-rules`: classes (Rocketjock/Warrior/Rogue/Medic/Engineer), races,
  percentage skill system (×2/÷2/÷4 difficulty scaling), one-time career change; skill-check
  opcodes (Jzatopa's corpus as leads, re-verified against our data + GBC ECL monitor as oracle).
- Combat additions: gun-heavy ranged model (brackets, bursts, explosives, grenades), tech items,
  medic/bandage rules; BR UI skin.
- **Ship-to-ship combat** subsystem (module per brief §7's optional-subsystem design), including
  boarding → grid combat handoff.
- Then **M7b — Matrix Cubed** as a delta: newer dialect differences, added systems, its ECL quirks.
- **Exit gate:** Countdown to Doomsday completable; Matrix Cubed reaches the same bar afterwards.
  *The original itch — Buck Rogers, native, on the Mac — is scratched here.*

### M8 — The companion (ongoing from M6, headline after M7)
- Automap overlay (we own the map + visited state), HUD, in-engine character editor,
  speed/turbo controls, mouse + gamepad, optional smoothing/hi-res modes — all default-off QoL
  per D4.
- Audio (PC speaker sfx first; later-title sound card support as needed).
- Distribution: signed/notarized mac .app, Linux/Windows builds, hosted web build; releases,
  user docs ("point it at your GOG folder").
- **Exit gate:** a Mac user with the GOG collection gets automap + HUD + editing in a native app —
  the GBC experience without Windows, which was the original motivation.

### M9 — The rest of the catalog (later, demand-driven)
- Remaining DAX-era fantasy titles as rules packs + detection entries (PoR, Silver Blades, Pools
  of Darkness, Krynn series, Savage Frontier pair), each needing only its quirk delta + validation
  pass. TLB/MicroMagic era (DQK, FRUA) as a separate compatibility effort — study FRUA's spirit,
  new container parser, likely new dialect.

---

## 5. Testing strategy summary

| Layer | In public CI (no game data) | Local-only (real data via GBX_DATA_DIR) |
|---|---|---|
| Parsers | Synthetic fixtures; fuzz smoke; unit tests | Golden comparisons vs reference tools; hash manifests |
| VM | Micro-ECL conformance programs | Real-script replays with expected outcomes |
| Rules | Table sanity + pack-schema checks | Verify-on-load vs user's binary; docket experiments |
| Combat/systems | Deterministic sim tests w/ fixed seeds | H4 oracle trace equality |
| Whole game | wasm + 3-OS builds, clippy, fmt, no-game-data guard | H5 playthrough replays w/ checkpoint hashes |

---

## 6. Legal & hygiene working rules

1. Engine code only in the repo; users supply data. CI job greps for known game-file signatures
   and fails the build if any sneak in (including in fixtures).
2. Rules packs contain uncopyrightable mechanics/numbers with evidence citations; no game text,
   art, maps, or scripts ship with the engine, ever.
3. SOURCES.md ledger: which reference (coab file, forum thread, blog post, manual page, own RE)
   informed each subsystem. Cheap to maintain, huge for credibility and for future contributors.
4. coab: read, cite, never copy (unclear license + transliterated-binary provenance).
   ssi-engine: GPL-3-compatible, but prefer reimplementation; ported logic gets provenance notes.
   Jzatopa corpus: treat all values as *candidate* until re-verified against our own data.
5. Trademark-neutral naming for any public release; game titles used descriptively only
   ("plays Curse of the Azure Bonds", not "Azure Bonds Engine").

---

## 7. Risks & tripwires

| Risk | Mitigation | Tripwire → action |
|---|---|---|
| Format friction (docs ≠ bytes) | Three independent reference decoders for cross-check | Any parser mismatch unresolved after a day → diff against all three refs, post to goldbox.games |
| ScriptMemory unknown-address explosion | Seed map from coab's `Gbl.cs`; unknown-access logging | Log keeps growing through M3 → dedicate a session to bulk-mapping from coab before M4 |
| Combat fidelity rabbit holes | H3/H4 exact traces; docket with per-item timebox | An item exceeds its timebox twice → document divergence, move on, revisit post-M6 |
| Buck Rogers RE cost (no coab) | Census delta first (measures it); Jzatopa corpus as leads; GBC ECL monitor oracle | Delta >> expected → consider CTD-only for M7, defer MC |
| WASM drift | wasm32 build in CI from M0; web frontend live from M2 | wasm build red > a week → stop feature work, fix |
| Solo-maintainer stall | Every milestone ends in a runnable demo; announce at M6; PLAN/docket public | Two months without a green gate → cut scope of current milestone, ship the demo |
| Legal | Rules 1–5 above; established ScummVM/GemRB precedent | Any takedown/complaint → engine-only posture already defensible; consult before responding |
| Someone else ships first | Different niche (native Mac + web + companion-first + clean provenance); watch competitors | Jzatopa/ssi-engine ships playable combat → evaluate collaboration on formats, stay differentiated on product |

---

## 8. Estimate summary

| Milestone | Focused weekends |
|---|---|
| M0 Basecamp | 1 |
| M1 It's alive | 1–2 |
| M2 First steps | 2–4 |
| M3 Party assembles | 2–3 |
| M4 First blood | 4–8 |
| M5 Fireball | 3–5 |
| M6 CotAB completable | 3–6 |
| **Subtotal to "finish CotAB natively"** | **16–29** |
| M7 Buck Rogers (CTD, then MC) | 6–10 |
| M8 Companion & distribution | 3+ then ongoing |

Working style assumption: Bryan + Claude sessions. Grindable, verifiable work (opcode
implementations against conformance tests, table extraction + verification, per-spell effects)
is well-suited to delegated/overnight agent runs *gated by the harness* — nothing merges without
its tests and, where applicable, oracle parity. Exploratory RE and design stay interactive.
Model and effort selection per task type is specified in §9 — most sessions should not be
running the top model.

---

## 9. Model & effort strategy (Claude Code)

Selection principle: **cost of being wrong × availability of a verification gate.** Where the
harness gates the output (parser goldens, VM conformance, oracle trace parity — the work doesn't
merge unless it passes), cheaper models are safe and fast. Where an error is silent and
architectural (a one-way-door design, an RE conclusion everything downstream trusts), spend for
reasoning depth. Feature "importance" is not a factor; verifiability is.

### Roster

| Model | Effort sweet spot | Right for | Wrong for |
|---|---|---|---|
| **Haiku 4.5** | low–medium | Dump/log triage, SOURCES.md upkeep, doc formatting, one-off shell/python helpers, commit hygiene | Anything touching VM/rules/format semantics |
| **Sonnet 5** | medium–high; xhigh when stuck | **The workhorse (~70% of build work):** parsers written against reference decoders, egui inspector, CLI plumbing, CI/packaging, and any well-specified mechanic/opcode/spell implemented against existing tests | Greenfield architecture; ambiguous RE with no reference |
| **Opus 4.8** | high–xhigh; fast mode for interactive grind sessions | Subsystem design-and-build (renderer/UI-shell state machine, save format, effect system), reading coab to spec a mechanic, ordinary trace-divergence debugging | Pure boilerplate (waste) |
| **Fable 5** | xhigh; **max sparingly** | One-way-door architecture (ScriptMemory, tick core, rules-pack schema), PRNG recovery, combat-trace forensics after Opus stalls, Buck Rogers binary RE, fidelity-docket adjudication, plan revisions | Routine implementation (waste) |

### Operating rules

1. **Default session = Sonnet 5 @ high.** Switch up only on a named trigger from the tables here,
   switch down (Haiku) for micro-tasks. `/model` + `/effort` per session; subagents can run a
   different model than the driving session, so a Fable/Opus driver can fan mechanical work out
   to Sonnet subagents rather than doing it itself.
2. **Two-strike escalation.** If a model fails the same task twice, don't let it thrash — bump
   model or effort one notch with the failure context attached. (Corollary: don't *start* high
   "just in case.")
3. **One Fable design pass before each one-way door.** ScriptMemory shape, tick/save-state
   schema, rules-pack format, combat-trace format: have Fable @ xhigh review the design doc
   *before* implementation starts. One session of insurance against weeks of rework — then
   implementation proceeds on cheaper models.
4. **Delegated/overnight grind runs on cheap models only when harness-gated.** Opcode long-tail,
   per-spell effects, table extraction: Sonnet (Haiku only for rigidly templated cases), and the
   run's definition of done is "conformance/oracle tests pass," never "looks right."
   Unverifiable work never goes to a cheap model unattended.
5. **Escalations are a design signal.** Keep a stuck-ledger; if a subsystem repeatedly needs
   Fable to make progress, the design is fighting you — fix the design, don't budget more tokens.

### Milestone mix

| Milestone | Dominant mix | Planned escalations |
|---|---|---|
| M0 Basecamp | Sonnet @ medium | — |
| M1 It's alive | Sonnet @ high | **Fable @ xhigh once:** VM + ScriptMemory architecture session. Opus: ECL operand-encoding puzzles |
| M2 First steps | Sonnet @ high | Opus @ high: UI-shell/renderer state-machine design (D8 compliance) |
| M3 Party assembles | Sonnet @ high | Opus: save-format design; original-save RE mismatches |
| M4 First blood | **Opus @ high–xhigh** | **Fable @ xhigh/max:** PRNG recovery (H3), trace-divergence forensics. Sonnet: each mechanic once trace tests exist |
| M5 Fireball | Sonnet @ high (the spell grind) | Opus: effect-system design. Fable: contested docket items |
| M6 Roll credits | Sonnet @ high (opcode grind) | Opus→Fable: playthrough-blocking divergences |
| M7 To the stars | **Opus @ xhigh** | **Fable @ xhigh/max:** BR binary RE, ECL dialect decoding, ship-combat design — the largest planned Fable concentration |
| M8 The companion | Sonnet @ medium–high | — |
| M9 Catalog | Sonnet @ high | Opus: per-title quirk hunting |

Net effect: Fable appears at a handful of named moments (architecture doors, PRNG, forensic
debugging, BR reverse engineering); Opus owns the genuinely hard middle of M4/M7; everything
else — most of the project by volume — runs on Sonnet with the harness as the safety net.

---

## 10. First session checklist (M0 kickoff)

- [x] `git init`; commit BRIEF.md + PLAN.md
- [x] Cargo workspace scaffold (crates + frontends/cli), LICENSE, SOURCES.md, .gitignore (game-data patterns), rust-toolchain.toml
- [x] CI skeleton: build + clippy + fmt + wasm32 check + no-game-data guard
- [x] Buy/locate GOG FR Archives Collection Two; extract CotAB → `~/goldbox-data/cotab` (outside repo); record file hashes
  *(2026-07-12: Mac offline installer unpacked via `pkgutil --expand-full`; 99 files + SHA-256
  manifest at `~/goldbox-data/cotab.sha256`; engine v1.3 per GAME.OVR; data files byte-identical
  to coab's bundled set and TITLE.DAX matches ssi-engine's detection MD5 — zero version skew,
  design-doc docket item 7 closed. Manual/Cluebook/Journal PDFs included for rules-pack evidence.)*
- [ ] Locate Buck Rogers CTD + MC originals → `~/goldbox-data/{ctd,mc}`
  *(research 2026-07-12: no legal digital source exists — rights litigation frozen by the 2017
  Dille trust bankruptcy; second-hand boxed DOS originals only, CIB ~$38 on eBay, prefer 3.5"
  media for USB-floppy reading; Matrix Cubed is the rarer find — buy on sight. Needed by M7.)*
- [x] Clone refs to `~/src/goldbox-refs/`: coab, goldboxexplorer, ssi-engine, Jzatopa workspace; fetch daxdump.zip
  *(2026-07-11; also vafada/daxviewer, plus formats/hackdocs/GBC 2.01/tlbutil2/goldboxfont archives
  from gbc.zorbus.net — daxdump.zip includes EclDump.exe, so both reference dumpers are on hand)*
- [x] `brew install temurin@21 innoextract`; build & run ssi-engine against the CotAB dir (day-0 sanity)
  *(tooling 2026-07-11: innoextract, maven, OpenJDK 21 via `openjdk@21` formula — the temurin
  cask needs an interactive sudo; `mvn package` clean. Day-0 run 2026-07-12: detects and renders
  CotAB. Note: must run with `-cp "target/*:src/main/resources"` — plain `java -jar` NPEs in
  `GameResourceConfiguration.findConfig`, which needs a directory on the classpath.)*
- [x] DOSBox Staging installed; CotAB boots in it *(0.82.2; booted 2026-07-12 via
  `dosbox-staging -c "mount c ~/goldbox-data/cotab" -c "c:" -c "start"`)*
- [ ] Decide oracle rig: UTM Windows VM (GBC + DOSBox + coab) vs CrossOver experiment; timebox the coab-core-on-.NET-8 spike to one evening
- [x] `GBX_DATA_DIR` convention wired into a hello-world `restrike detect` that fingerprints a game dir
