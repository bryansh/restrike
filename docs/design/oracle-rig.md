# The M4 oracle rig: bit-exact PRNG (H3) and combat-trace equality (H4)

Status: **v1 draft** (Fable-authored, 2026-07-15). One-way-door review pending.
Companion to PLAN.md §3's harness ladder; this doc pins how H3/H4 actually get
built, what a combat trace *is*, and which oracle produces it.

## 0. What M4 needs from this door

M4's exit gate: a real fixed encounter playable interactively and headlessly,
with headless traces matching the oracle exactly for ≥10 seeds. That requires,
in dependency order: (1) our RNG produces the original's exact stream (H3),
(2) a defined trace artifact both sides can emit (this doc's D-OR3), (3) an
oracle that emits it (D-OR2), and (4) a comparator (`restrike trace-compare`).

## 1. H3 — the PRNG, recovered (2026-07-15, Fable session)

**coab is not the spec here.** `seg051.cs:8,61` swaps the original RNG for C#
`System.Random` wholesale — algorithm, seeding, and draw semantics all
unfaithful. H3 was therefore recovered from the binary directly, using our own
`gbx_formats::exepack` decoder on START.EXE (62,432-byte decompressed image,
CotAB v1.3 GOG):

- **Algorithm: the Borland Turbo Pascal LCG.** `RandNext` at image offset
  `0xa5a9`: loads a 32-bit state dword from `DS:0x47F0`, computes
  `state = state * 0x08088405 + 1` (mod 2^32), stores it back, returns the new
  state in DX:AX. The multiply is the TP runtime's 16-bit idiom: a `mul` by a
  code-segment *data word* holding `0x8405` plus shift-add sequences supplying
  the `0x0808` high-word and `0x8405 × state_hi` contributions. (The
  multiplier living in a data word — image offset `0xa5df`, `cs:[0x166F]`
  with cs base paragraph `0x997` — is why a naive scan for the 32-bit
  constant finds nothing.)
- **Seeding: `Randomize` = DOS wall clock.** Image `0xa5e1`: `int 21h/AH=2Ch`
  (get system time) stores CX:DX — hour:minute:second:centisecond packed — to
  the same `DS:0x47F0`. One dword is the *entire* RNG state.
- **Float path:** image `0xa570` builds TP's 6-byte real from the state (used
  by `Random`-real callers, if any; census TBD).
- **Verification pins (D10-clean, hashes only):** decompressed-image SHA-256
  ranges — `RandNext` `[0xa5a9,0xa5df)` =
  `d6afac0a1f8a6ec4458d6c06f6a33488d604f3d6e75afaeebc83ee3379cee54e`,
  `Randomize` `[0xa5e1,0xa5ee)` =
  `d3a4eb04534508db7a20d063cbeed76d2de68d65f49afc79ccd6cf9e4962ab09`.
  An M4 step-1 local-tier test re-derives these from the user's binary and
  loud-fails if the image doesn't match (a different game version needs
  re-pinning, not silent trust).

**Two consequences, docketed:**

- **FD-26:** Turbo Pascal's *integer* `Random(N)` is
  `(hi16(new_state) * N) >> 16` — a scaled high word. coab's is
  `Next() % N` — modulo. Different distribution, different bit consumption.
  Every coab call site (`roll_dice`, initiative, AI) is therefore only
  *structurally* trustworthy: which rolls happen in what order, but not the
  arithmetic on each draw. Our `gbx-prng` must implement the TP semantics;
  the exact `Random(N)` return path (scale idiom, register convention) gets
  confirmed against a live DOSBox session in the H3 acceptance test below.
- **FD-27:** seed lifecycle. Where does the game call `Randomize` — once at
  boot, per battle, never (fixed seed + wall-clock drift)? Callers of
  `RandNext`/`Randomize` need enumeration (cheap: scan for `call` rel16 to
  the two entry points across the image + GAME.OVR overlays). Also: GAME.OVR
  (overlay file) may hold its own copies — the RNG the *combat* overlay calls
  must be pinned, not assumed identical.

## 2. Decisions

### D-OR1 — `gbx-prng`: one crate, TP semantics, oracle-mode parity (one-way door)

A tiny `gbx-prng` crate owns the LCG: `next() -> u32` (full state),
`random(n: u16) -> u16` (TP scaled-high-word), `state()/set_state(u32)` for
checkpointing (already required by `.rsav`'s PRNG capture). The engine's
existing `roll_uniform` call sites migrate to it. Draw *shape* (how
`roll_dice(count, size)` maps to `random()` calls) is transcribed from coab's
`ovr024.roll_dice` — structure from coab, arithmetic from the binary (the
FD-26 split). No other RNG exists anywhere in the engine (D9 already
guarantees this; the door is that oracle-parity mode and normal play use the
*same* generator — no "fast path" divergence, ever).

### D-OR2 — Oracle tiers: DOSBox-X debugger is tier 1; instrumented coab is tier 2

- **Tier 1 (ground truth): the real binary under DOSBox-X** (debugger build).
  dosbox-staging stays the *play/capture* rig (`docs/dosbox-capture.md`), but
  its debugger is not scriptable enough for tracing; DOSBox-X ships
  breakpoint/logging machinery (`bp`, `log`, memory watch) we can drive.
  Rig mechanics: locate `RandNext` at runtime by its pinned byte pattern
  (or the known image offset + load base), set a breakpoint, log
  `DS:0x47F0` before/after plus the caller's return address on each hit.
  Seed control: poke the dword at `DS:0x47F0` after boot — no need to fight
  the wall-clock `Randomize`. Every draw is thereby attributable and the
  whole session reproducible.
- **Tier 2 (cheap, structural): an instrumented coab build.** coab runs; a
  local fork with trace `printf`s at `roll_dice`/attack/damage/initiative/AI
  call sites emits *action-level* traces cheaply. Caveat, now with seven
  confirmed divergences' worth of evidence: tier 2 validates *structure*
  (event order, call sites), never arithmetic. Tier-2 traces are advisory;
  only tier-1 equality closes a docket item. The fork stays in
  `~/src/goldbox-refs` (never in our repo).
- **Tier 3 (bootstrap aide): GBC's published memory maps** corroborate
  combatant-struct offsets for tier-1 memory watches. Documentation only.

### D-OR3 — The trace format (one-way door): versioned JSONL, two profiles

One artifact, `.gbxtrace`, JSON Lines; first line a header object, then one
object per event. Header: `{"gbxtrace": 1, "profile": "prng"|"action",
"game": "cotab-v1.3", "seed": u32, "encounter": <id or free-form>,
"source": "restrike"|"dosbox-x"|"coab-fork", "notes": ...}`.

- **`prng` profile** (tier 1's native output): one event per `RandNext` hit —
  `{"e":"rng","before":u32,"after":u32,"caller":u32}` (+ optional
  `"n"`/`"result"` when the integer-Random wrapper is the caller and its
  operand is recoverable). This profile is what H3's acceptance test compares
  and what makes any divergence *bisectable*: the first mismatching draw
  localizes the bug to one call site.
- **`action` profile** (our engine's native output; tier 2's too): semantic
  events — `{"e":"init", "order":[...]}`, `{"e":"attack","actor":..,
  "target":..,"roll":..,"needed":..,"hit":bool}`, `{"e":"dmg",...}`,
  `{"e":"move",...}`, `{"e":"ai","actor":..,"choice":..}`,
  `{"e":"status",...}`, `{"e":"award",...}`. Field vocabularies get pinned
  during M4 implementation sessions (additive growth allowed; renames are
  format-version bumps).
- Our engine emits **both** profiles simultaneously in trace mode (every
  `gbx-prng` draw is also an `rng` event with a synthetic caller tag). H4
  equality = action-profile equality with rng-profile as the bisection
  tool. D10: traces derived from real encounters live under
  `GBX_DATA_DIR`/local only; repo/CI traces come from synthetic encounters.
- `restrike trace-compare <a> <b>`: first-divergence report (event index,
  both events, rng-context window), exit code for CI.

### D-OR4 — H3 acceptance test (the "predicting a seeded DOSBox session" rung)

Scripted: boot CotAB under DOSBox-X → poke `DS:0x47F0` = K → play a fixed
short input script through something roll-heavy (the training-hall HP roll or
a scripted fight) → capture the prng-profile trace. Our side: `gbx-prng` from
seed K must predict the *entire* logged stream (states before/after, in
order). Ten different K values. This single test closes H3, confirms the
`Random(N)` return semantics (FD-26's open half), and proves the rig
end-to-end before any combat work leans on it.

## 3. What this absorbs from the deferred human-validation list

The tier-1 debugger rig, once standing, cheaply retires every parked item:
- **FD-23 item 1 (stat byte order):** drain a stat in play (or find a drained
  NPC), save, compare bytes — same session that first exercises combat.
- **Mid-combat menu check** (save-formats §1.2 assumption): try it live.
- **M2 capture checklist / screenshot comparisons:** dosbox-staging rig,
  `restrike compare`, unchanged.
- **Code-wheel rune-order pin** (`docs/copy-protection.md` open item): read
  `input_expected` from memory at the copy-protection prompt once, note the
  two runes shown, and the rune-index origin/direction falls out — closes the
  M6 "answer shown" prerequisite from the debugger for free.

## 4. M4 build order (session-sized steps)

1. **`gbx-prng` + binary pinning** (session): the crate per D-OR1; a
   local-tier test re-deriving §1's hashes from `$GBX_DATA_DIR/START.EXE`;
   a `RandNext`/`Randomize` caller census across START.EXE + GAME.OVR
   (settles FD-27; loud-documents any overlay-local RNG copies); migrate
   engine `roll_uniform` call sites.
2. **DOSBox-X rig + H3 acceptance** (Bryan + Fable session): install
   DOSBox-X, script the breakpoint/poke/log flow, run D-OR4's ten seeds.
   Absorbs the §3 human items opportunistically.
3. **Trace plumbing** (session): `.gbxtrace` emit in the engine (both
   profiles), `restrike trace-compare`, synthetic-encounter goldens in CI.
4. Combat systems proper (map gen, initiative, action economy, …) — each new
   system lands with its action-profile events, keeping H4 continuously
   checkable rather than big-bang at milestone end.

## 5. Open questions → docket

- **FD-26** (filed): TP scaled `Random(N)` vs coab modulo; return-path
  confirmation via D-OR4.
- **FD-27** (filed): `Randomize`/seed lifecycle; caller census; GAME.OVR
  overlay RNG copies.
- Combat-overlay memory map for action-level tier-1 watches (needed only if
  rng-profile bisection proves insufficient — deliberately deferred; start
  with prng-profile + our action events, add oracle action watches when a
  concrete divergence demands them).
