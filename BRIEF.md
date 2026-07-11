# Project Brief: A Native, Cross-Platform Gold Box Engine ("ScummVM for Gold Box")

> Handoff document. This captures the design thinking for a from-scratch reimplementation
> of SSI's "Gold Box" RPG engine so the classic games run natively on modern systems
> (macOS first-class). Written to be picked up by another agent/collaborator cold.
>
> *Captured 2026-07-10. See PLAN.md for the implementation plan, including amendments
> from the design review and prior-art sweep of the same date.*

---

## 1. Goal & Motivation

Build a **new, portable game engine** that reads the *original* Gold Box game data files and
runs them natively — on macOS, other desktops, and ideally in a browser via WebAssembly.

- **Immediate itch:** Gold Box Companion (GBC) — the beloved fan tool that adds automap, HUD,
  and character editing — is **Windows-only**. There is no good way to get that experience on macOS.
- **Realization:** rather than fight to port a tool that *hooks into* the original game running
  under DOSBox, it's cleaner (and more capable) to reimplement the engine itself. Once we own the
  engine, the "companion" features are free.
- **Ultimate target:** play **Buck Rogers: Countdown to Doomsday** and **Matrix Cubed** natively
  on macOS — and, by extension, the whole Gold Box family.

**The niche appears genuinely open.** Extensive catalogs of open-source engine reimplementations
exist, and the adjacent D&D engines are well covered — **GemRB** (BioWare Infinity Engine:
Baldur's Gate/Icewind Dale) and **xoreos** (BioWare Aurora: Neverwinter Nights), both
cross-platform incl. macOS. But there is **no known, maintained, cross-platform reimplementation
of the SSI Gold Box engine.** (Caveat: can't prove a negative; a small/unindexed repo could exist.
Recommend a focused GitHub sweep before committing serious time.)

---

## 2. Core Concept (the one idea that makes everything click)

**Every Gold Box game = one shared engine + that game's own data files.**

- SSI wrote the engine once and reused it across ~11 titles, feeding each different data.
- The *data* (maps, monsters, items, text, art, scripts) is fine forever. What's stranded is the
  original **1990 DOS executable**, because modern CPUs/OSes don't run 16-bit DOS binaries. DOSBox
  works by *faking the old machine*; we instead **write a fresh engine for the still-good data.**

This is the **ScummVM / OpenMW model** (engine reimplementation), **NOT emulation** (DOSBox).

### Legal model (important)
Clean reimplementation is the well-worn, defensible path (ScummVM precedent):
- Distribute **only our engine code** (e.g. GPL).
- **Users supply their own legally-obtained data files.**
- **Never bundle copyrighted game assets.**

---

## 3. Terminology (so the reader isn't lost)

- **Engine** — the reusable program SSI wrote once and used for all the games.
- **Data files** — per-game content the engine loads.
- **DAX** — the compressed archive/container format the data ships in ("a zip with RLE compression").
  *Note:* the latest fantasy titles switched to a different container, **TLB** (hence the community's
  separate `TLButil2` tool).
- **ECL** — a small scripting language, stored as compiled **bytecode**, that drives events:
  dialogue, "open chest → fight starts," branching. The brain of each adventure.
- **Opcode** — one instruction in ECL bytecode (e.g. "print text," "check flag," "begin combat").
- **VM / interpreter** — the part of our engine that reads ECL and executes opcodes. A small, tame
  VM — nothing like the whole-PC emulation DOSBox does.

---

## 4. Architecture / How It Works

Runtime flow when a user opens a game folder:

1. **Detect the game** — fingerprint the files to identify which title it is (Matrix Cubed vs Pool
   of Radiance, etc.), because rules/quirks differ. (ScummVM does exactly this.)
2. **Parse/unpack the data** — DAX (or TLB) archives → ECL scripts, maps, monsters, items, text, art.
3. **Run the ECL VM** — *load once, interpret forever.* ECL executes continuously the whole play
   session, reacting to player actions. Not a one-shot transform at load time.
4. **Game systems / rules engine** — combat, movement, magic/tech, world simulation (see §5). This is
   where ~90% of the real work lives ("what happens next").
5. **Backend abstraction** — rendering, input, and audio sit behind a thin platform-agnostic
   interface. **Key lesson stolen from ScummVM (its "OSystem" layer):** keep the core (parsing +
   ECL VM + rules) pure and platform-neutral; put I/O behind an interface. Then the *same core*
   compiles to a native Mac app **and** to WASM in a browser — only the backend swaps.
6. **Save/load** — our own save format, **plus** read the *original* save format so players can
   import an existing party.

**Payoff:** because we own the game state, the companion features that motivated this whole project
— automap, HUD, character editing — become **first-class and basically free.** No memory-scanning,
no DOSBox.

---

## 5. The Rules Problem

### Which rulesets
- **Fantasy Gold Box** (Pool of Radiance et al.): **AD&D 1st edition** rules, **plus SSI's own
  modifications**.
- **Buck Rogers** (Countdown to Doomsday, Matrix Cubed): TSR's **Buck Rogers XXVc** ruleset
  (AD&D 2e-era, sci-fi), adding a **percentage-based skill system** and its own classes/races, plus
  a separate **ship-to-ship combat** subsystem.
- **Shared combat spine (both):** turn-based **grid** combat, front/rear ranks, **dexterity-based
  initiative**, and **THAC0** attack resolution (d20 ≥ THAC0 − target AC), with **descending AC**.
  So the engine's combat *core* is common; mostly tables and the skill layer differ.

### CRITICAL fidelity principle
**The rules to reproduce are what the *binary actually does*, NOT the rulebook.** SSI modified and
sometimes mis-implemented AD&D. Classic example: the games follow the treasure tables literally
while ignoring the balancing DMG rules, producing a famously "broken" economy — which is
nonetheless the *authentic* behavior to reproduce. Rulebook = intent; binary = truth; ship the truth.

### Rule categories the engine needs
- **Character/party model:** six ability scores + all derived-modifier lookup tables (these tables
  *are* rules), races/classes with level limits, HP by hit die, XP/leveling (incl. training halls),
  saving-throw tables. Buck Rogers adds its distinct classes (Rocketjock, Warrior, Rogue, Medic,
  Engineer), races, the % skill system (difficulty scales the rating ×2/÷2/÷4), and one-time
  career change.
- **Combat (the bulk):** initiative, THAC0 progressions by class/hit-die, auto-miss on natural 1
  (no auto-hit on 20, no crits), AC modifiers (cover/concealment/dex), damage per weapon,
  attacks-per-round, ranged/range brackets (big for the gun-heavy Buck Rogers games), monster
  stats + morale/AI, status effects (poison/paralysis/level drain/comatose).
- **Magic / tech:** fantasy = full Vancian spell system (memorization, slots, per-spell
  effect/duration/area/save — each spell is its own rule). Buck Rogers = reskinned tech, rocket
  weapons, psi-powers.
- **World simulation:** first-person maze + overworld movement, random-encounter generation keyed
  to party strength, economy (shops/prices/treasure tables), Parlay/NPC dialogue system, traps/
  locks/secret doors, and (Buck Rogers only) ship-to-ship combat.

### How to LEARN the rules (source hierarchy, cheapest → most authoritative)
1. **Published rulebooks** — AD&D 1e PHB/DMG (fantasy) or the Buck Rogers XXVc rulebook. Cheap,
   legal, gives the intended shape of every system. This is *intent*, not implementation.
2. **The game's boxed docs** — manuals, adventurer's journal, reference cards; often print the exact
   tables the game uses.
3. **The binary itself (ground truth):**
   - *Static:* disassemble the DOS executable (Ghidra/IDA) and read routines. Shortcut: many rule
     tables (THAC0, saving throws, XP) live as **data arrays** in the executable — extract the
     tables directly rather than decoding logic.
   - *Dynamic / black-box:* treat the running game (DOSBox) as an **oracle** — feed known inputs,
     observe outputs; use GBC to inspect memory and the ECL-monitor to watch scripts; roll the same
     attack 1000× and histogram to infer formulas.
4. **Prior reverse-engineering** — coab already encodes CotAB's rules in readable C#; goldbox.games
   forums, Hacking UA docs, Simeon Pilgrim's blog document formats/tables; CRPG Addict analyses
   catalog exactly where SSI diverged from AD&D-as-written.
5. **Fan tabletop write-ups** — for XXVc etc., as sanity checks against the digital behavior.

**Recommended method:** triangulate — rulebook as the map, **extract real tables from the binary**
for exact values, **validate against the running game** to catch SSI's mods/bugs. And **encode
rules as data tables, not hardcoded logic**, mirroring how the originals stored them.

---

## 6. Per-Game vs Shared Work (do we RE every game?)

Mostly **no** — the split is clean:

- **Engine LOGIC → reverse-engineered ~once per "flavor," not per game.** The combat/initiative/
  movement/casting logic is largely the same program across the fantasy family. coab effectively
  proved this by fully working out one game (CotAB); most generalizes. Buck Rogers is a *second*
  flavor (sci-fi + skills + ship combat), but the two Buck Rogers games share their engine with
  each other. So: a handful of engine flavors, not 11 disassembly projects.
- **Per-game rule DATA → read generically at runtime.** Shared formats mean one set of loaders
  pulls each game's rule tables from its files at load time. Point the engine at Matrix Cubed's
  data → it loads Matrix Cubed's tables.

**On "do it dynamically at runtime":**
- Dynamically **loading data tables** → YES, that's the plan.
- Dynamically **deriving executable logic** so you never understand it → NOT a real thing. Logic is
  behavior, not data. Your only options are (a) implement it yourself from RE, or (b) run the
  original code — and (b) is just DOSBox again, defeating the purpose.
- **Clever hybrid:** use the running original as a **development-time oracle**, not a runtime
  dependency. Capture (input → output) pairs via DOSBox + ECL-monitor + GBC and build a
  **differential test harness** that checks your engine reproduces them. The original teaches and
  validates during dev; the shipped engine has zero dependence on it. This catches SSI's quirks
  semi-automatically.
- **Concrete artifact:** a small **rules-extraction tool** that reads a game's binary + data once
  and emits clean **"rules packs"** (JSON/TOML) the shared engine loads. Automated, per-game, cheap.

**Honest caveats:** the engine *evolved* (later games added moon-phase casting, weather, party
romance) — so it's a few engine generations with small quirks, not one frozen codebase. And some
rules genuinely live in code (exact combat sequence, AI, Parlay resolution) — implement those once
from engine RE rather than extract.

---

## 7. Sequencing — Which Game to Target First

**Do NOT start with the "most mature" engine assuming it's a superset of the earlier games.** It
isn't:
- No version was ever built for backward compatibility — the newest engine has no code to read
  older games' files.
- **Data format changed:** early/mid games use DAX; the latest fantasy titles (Dark Queen of Krynn,
  Unlimited Adventures) use **TLB**.
- **The newest engine is partly a different codebase** — Dark Queen of Krynn and Unlimited
  Adventures were built by MicroMagic, and the latest releases are C/C++ rather than the original
  Pascal. "Maturity" here is closer to a *fork* than linear accumulation.
- Growth wasn't purely additive (new subsystems, not a strict superset).
- **Buck Rogers is a separate branch entirely** — no fantasy engine "handles" it.

**Reframe — separate two axes:**
- **Design-for-the-superset (capabilities):** YES. Architect the engine + rules model to accommodate
  the most demanding cases (very high level caps à la Pools of Darkness, pluggable spell/effect
  defs, optional subsystems like weather/romance/ship-combat as modules). Cheap early, painful to
  retrofit.
- **First reverse-engineering target (which game to prove it on):** the OPPOSITE of "newest." Pick
  the **best-understood, best-documented, best-oracle'd** game.

**Recommended plan (given the Buck Rogers goal): a two-target path**
1. **Curse of the Azure Bonds first** — leverage **coab as a working oracle** to nail the shared
   engine spine cheaply (combat, movement, ECL, rules-as-data loading), on the DAX format.
   *(Pool of Radiance is the alternative — original, most-documented.)*
2. **Then jump straight to a Buck Rogers game** (the actual destination) as its own ruleset variant.
   Doing one Buck Rogers game gets most of the second for free.
3. **Handle TLB / MicroMagic-era games last**, as a separate compatibility effort.

**North star:** FRUA (Unlimited Adventures) is worth studying as *proof-of-concept for the
data-driven architecture* (the one time SSI made the engine a general "load any module" system) —
emulate its spirit, not its TLB format.

---

## 8. Technology Recommendation

Target scope is macOS + desktop + browser (NOT ancient consoles), which frees us from the extreme-
portability constraints that shaped ScummVM's deliberately conservative, exception-free, custom-
container C++.

**Lean: Rust core.** Fits this work almost suspiciously well:
- Excellent, safe **binary-parsing** story (`binrw`/`nom`/`byteorder`) — and this project is ~80%
  parsing weird 1989 binary formats + running an interpreter, exactly where C's footguns bite.
- **Enums + pattern matching** map perfectly onto an ECL opcode set and game state.
- First-class **`wasm32`** target → browser play; `wgpu` gives one renderer across Metal/Vulkan/
  D3D/WebGPU.
- **Cargo** makes cross-platform builds painless.
- Cost: learning curve (borrow checker).

Front end: `wgpu`/`winit`, or a lightweight framework like `macroquad`, **behind a backend trait**
so the same core targets native + WASM.

**Alternatives:**
- **Modern C++ (C++20/23)** — proven, huge ecosystem (SDL3, raylib), can read ScummVM idioms
  directly; weaker safety on the risky parsing/VM code + rougher tooling.
- **Zig** — fun, great low-level + cross-compilation; pre-1.0, smaller ecosystem.
- **TypeScript** — worth it *if browser-first distribution* is the priority; clunkier binary parsing.
- (coab used **C#**; today .NET is cross-platform, but native/WASM distribution is heavier than Rust.)

**Most important, language-independent:** keep a pure, platform-agnostic core + thin backend
interface. That architecture matters more than the language.

---

## 9. Prior Art & Resources (the "four buckets")

### RUN the original
- **DOSBox** (DOSBox Staging / DOSBox-X are the maintained forks; both native macOS) — emulates a PC.
- **Gold Box Companion (GBC)** — Windows-only; adds automap/HUD/editing by reading DOSBox memory.
  Includes an **ECL Tool** (browse/edit bytecode) and **ECL-monitor** (watch the running script live
  — invaluable as a test oracle). Home: `gbc.zorbus.net`.

### READ / unpack data
- **Gold Box Explorer** — browse the data files.
  `github.com/simeonpilgrim/goldboxexplorer` ; 1.x at `github.com/bsimser/Gold-Box-Explorer`.
- **DAXDump** / **ECLDump** (Simeon Pilgrim) — decode DAX RLE + dump ECL scripts. Reference decoders
  / ground-truth to check our parser against (bundled as `daxdump.zip` on the GBC site).

### REBUILD (prior reimplementations to study)
- **coab** — `github.com/simeonpilgrim/coab`. From-scratch reimplementation of **Curse of the Azure
  Bonds** in **C#** (with a lot of C/IDA reference material), ~feature-complete, **Windows-targeted**.
  THE reference implementation — read how it dispatches ECL opcodes and models combat. Also our
  best behavioral **oracle** for CotAB.
- **Dungeon Craft** — open-source rebuild of the **FRUA** editor/engine (FRUA-lineage; historically
  Windows).

### DOCS + validation
- **goldbox.games forums** — active hub. "Hacking original Gold Box games" section; "ECL file
  contents" thread (topic 1241).
- **"Hacking UA" / `hackdocs.zip`** — in-depth file-format docs (FRUA-oriented; extremely useful but
  FRUA's event model is a slightly higher-level layer than the originals' raw ECL).
- **Simeon Pilgrim's blog** — `simeonpilgrim.com/blog` — multi-year RE write-ups + Gold Box cheat
  codes. He also contributed format notes to the **REWiki**.
- **CRPG Addict** — deep analyses (e.g. the economy teardown) cataloging where SSI diverged from
  AD&D-as-written.
- FRUA forums (`ua.reonis.com`) — ECL-monitor discussion (topic 4110).

> Note on ECL documentation: the *most thorough* format material is FRUA-oriented (it was a
> construction set, so most dissected). For the original-game ECL specifically, prefer
> **coab + ECLDump + the goldbox.games thread**. Expect small per-game differences in opcode tables
> (Buck Rogers ≠ CotAB); cross-check against the actual Buck Rogers data.

---

## 10. Weekend MVP (realistic scope)

**Reality:** a *playable* game is not a weekend (coab took years; combat + the full opcode set is
weeks+). But the **foundation** — the genuinely hard, interesting core — is a weekend, and can end
with something real running.

- **Fri (setup + first contact):** scaffold the project; write the **DAX reader** (parse the index,
  decompress a block). *Win:* point it at real game files and print the resource list + one
  decompressed block.
- **Sat (the meat):** **ECL disassembler** (decode bytecode → readable opcodes; ECLDump as
  reference); start the **ECL VM** — implement enough opcodes to execute a simple script (print text,
  check a flag, take a branch). Parse a couple of static structures (string/text table, map header).
  *Win:* a real encounter script runs headless and prints its own dialogue ("it's alive").
- **Sun (make it tangible + validate):** parse a map layout and **dump it as an ASCII grid** (crude
  automap from real data). Validate ECL execution + map against the original (DOSBox side-by-side /
  coab / walkthroughs). *Stretch:* a bare `wgpu`/SDL window drawing the map grid with arrow-key
  movement.

**Definition of done (Sun night):** a headless tool that opens a real data set, lists/decompresses
DAX resources, disassembles ECL into readable opcodes, executes a simple ECL script to produce its
text/branching, and dumps a map layout. (Stretch: a movement window.) Not a game — the entire
**load-and-interpret spine** of one, which is the part that's never existed cross-platform.

**De-risk:** even though Buck Rogers is the destination, do the weekend against **Curse of the Azure
Bonds** or **Pool of Radiance** — best docs + coab as an oracle. Re-point loaders at Buck Rogers
data afterward.

**Honesty flags:**
- Biggest unknown = **format friction** (how cleanly published docs match actual bytes).
- **Graphics decoding** (walls/sprites + palette handling) is the classic time sink — kept as a
  stretch, not a baseline.
- Working loop: I write first-pass parsers from documented formats → you run them against your real
  files → we fix mismatches together. (I don't have — and shouldn't bundle — the copyrighted game
  data.)

---

## 11. Roadmap Beyond the Weekend

1. **Foundation** — data parsing + ECL VM (the weekend).
2. **Non-combat engine** — map loading, first-person navigation, party state, save/load, + importer
   for original saves.
3. **Combat** — turn-based grid, THAC0/XXVc rules math, monster AI; + Buck Rogers ship combat.
4. **QoL / companion layer** — automap, HUD, character editing, save-anywhere, mouse/gamepad,
   hi-res, speed controls (all "free" once we own the state).
5. **More games** — via per-game **rules packs** + generic data loaders; TLB-era titles as a later
   compatibility pass.

---

## 12. One-Paragraph Pitch

A native, cross-platform reimplementation of SSI's Gold Box engine — the ScummVM model applied to a
D&D CRPG engine that, unlike the Infinity/Aurora engines, nobody has ported yet. It detects a game
from its data files, unpacks the DAX archives, runs the ECL scripts on a small bytecode VM, and
implements the shared AD&D-derived rules as swappable per-game data — validated bug-for-bug against
the original running in DOSBox as a development oracle. Built as a pure, platform-neutral core
(lean: Rust) behind a thin backend, so it runs natively on macOS and in the browser via WASM, with
automap/HUD/editing as first-class features. First target: Curse of the Azure Bonds (to borrow
coab as an oracle and nail the engine spine); destination: Buck Rogers on macOS.
