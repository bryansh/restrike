# The M4 oracle rig: bit-exact PRNG (H3) and combat-trace equality (H4)

Status: **v2** (Fable-authored 2026-07-15; v1 → three-lens adversarial round →
v2 same day). The round refuted two v1 claims outright (integer `Random(N)`
semantics; the cs-base constant), demoted both of v1's oracle-host picks, and
restructured the H3 acceptance test and the H4 definition. Findings and their
dispositions are inline. A bounded round 2 on the *new* surface (D-OR2's
staging hook, D-OR4's emulated tier, D-OR5) is warranted before build.

## 0. What M4 needs from this door

M4's exit gate: a real fixed encounter playable interactively and headlessly,
with headless traces matching the oracle exactly for ≥10 seeds. Dependency
order: (1) our RNG reproduces the original's exact stream **and per-call
results** (H3), (2) a defined trace artifact (D-OR3), (3) an oracle that can
emit or validate it (D-OR2), (4) a comparator (`gbx-oracle` + CLI wrapper),
and (5) a non-circular definition of what "traces match" means (D-OR5).

## 1. H3 — the PRNG, recovered and adversarially re-derived

**coab is not the spec here.** `seg051.cs` swaps the original RNG for C#
`System.Random`. Recovery is from the binary (CotAB v1.3 GOG START.EXE,
EXEPACK-decompressed via `gbx_formats::exepack::decode`; image 62,432 bytes,
whole-image SHA-256 `f0ce4b2036e48151077bdc34f7afba48bd9d2032139871924b6854c4b5784ec1`).
All claims below survived an independent adversarial re-derivation
(capstone-decoded, instruction-semantics simulated over 200k states, 0
mismatches) except where marked *corrected*.

The RNG cluster lives in one code segment, **cs base paragraph `0x8F7`**
(image-relative; *corrected* — v1 said `0x997`, a runtime segment with a
`0xA0` load base baked in; the v1 arithmetic didn't even self-check).
Image offset = `0x8F70 + in-segment offset`:

| Routine | cs:offset | image offset | behavior |
|---|---|---|---|
| integer `Random(N)` wrapper | `0x8F7:0x15EA` | `0xa55a` | `call RandNext`; if N==0 → 0 (draw **already consumed**); else `hi16(new_state) DIV N`, remainder returned (AX). Far entry, `retf 2`. |
| float `Random` entry | `0x8F7:0x1600` | `0xa570` | TP 6-byte real = `new_state / 2^32`. |
| `RandNext` (internal, near) | `0x8F7:0x1639` | `0xa5a9` | `state = state × 0x08088405 + 1` (mod 2^32); state dword at `DS:0x47F0`; new state in DX:AX. TP 16-bit idiom: `mul` by the code-segment data word `0x8405` at `cs:[0x166F]` (image `0xa5df`) + shift-add sequences for the `0x0808` and `state_hi × 0x8405` contributions — why naive 32-bit-constant scans find nothing. |
| `Randomize` | `0x8F7:0x1671` | `0xa5e1` | `int 21h/AH=2Ch` (DOS wall clock); low word ← CX (hour:min), high word ← DX (sec:centisec) — the seed dword is **DX:CX** (*corrected*; v1 wrote the composition inverted). |

**Integer semantics (v1 REFUTED, corrected):** `Random(N) = hi16(new_state) mod N`
— the TP **5.x** idiom (`div bx`, verified in the wrapper bytes), *not* TP6+'s
scaled high word as v1 claimed. coab's `% N` reduction shape was faithful all
along; its generator and its `Random(0)` short-circuit (returns before
drawing; the binary draws first) are not. A v1-spec `gbx-prng` would have
matched the state stream while getting **every game roll wrong** — caught by
review before any code existed.

**Caller census (renders FD-27 largely answered):** GAME.OVR (272,137-byte
overlay file) contains **no local RNG copy** — all overlay randomness
far-calls the resident cluster. Integer `Random`: 29 call sites in GAME.OVR +
5 in START.EXE (matching coab's 29 `seg051.Random(` sites 1:1). Float
`Random`: 4 sites (= coab `ovr019.cs`'s 4 `Random__Real` calls — so D-OR1
needs the real path eventually, M5-ish). `Randomize`: **exactly one call
site, at boot** (GAME.OVR `0xf5f6` = coab `seg001.InitFirst`) — never
re-seeded per battle. Seed-poke-once is therefore sound *statically*; D-OR4
part B still verifies it dynamically (single-writer on `DS:0x47F0`).

**Verification pin (D10-clean, hash only; *corrected* — v1's two ranges left
a hole exactly over the multiplier word):** one contiguous range covering
wrapper + float entry + RandNext + multiplier + Randomize:
`[0xa55a, 0xa5ee)` SHA-256 =
`0f770ce01cc999eb8ca75406d57de94ffd7c01e7438c0647395b26a668bea68b`.
The M4 step-1 local-tier test re-derives this from the user's binary and
loud-fails on mismatch (another game version needs re-pinning, not trust).

## 2. Decisions

### D-OR1 — `gbx-prng`: one crate, binary-verified semantics, draw-parity contract (one-way door)

- API: `next() -> u32` (full state step); `random(n: u16) -> u16` implementing
  the binary's wrapper **exactly**: always advance state (including n==0,
  which returns 0 *after* drawing), else `(next() >> 16) as u16 % n`;
  `state()/set_state(u32)`; `random_real()` (TP 6-byte-real semantics) added
  when M5 reaches ovr019's 4 call sites. No other RNG may exist in the engine.
- **Draw-parity contract** (new in v2): (a) parity is *scoped* — from a
  synchronization point (poked/set state) through a faithful-input window,
  every `gbx-prng` draw maps 1:1, in order, same operand, to an original call
  site; (b) `random(n)` never short-circuits (the current engine
  `roll_uniform(0)` early-return dies in the migration); (c) draws with **no
  original counterpart must not touch gbx-prng** — concretely, the fade
  dither (`draw.rs` `apply_recolor_dithered`, per-changed-pixel draws)
  becomes a deterministic position-hash pattern (the renderer doc already
  declares dither pixels non-comparable; whether the binary's dither touches
  `DS:0x47F0` at all is docketed as FD-28); (d) the migration is an audited
  per-site checklist (each site → original citation: RANDOM opcode,
  `roll_dice` service, door bash/pick, training HP, creation rolls when they
  land), not a mechanical rename.
- **`.rsav` consequence** (new in v2; unstated in v1): `SaveState.prng` is
  today a `u64`-state `EngineRng`, postcard-serialized as the **last** field —
  a v1 save with state < 2^32 would *silently misload* into a u32 field.
  The gbx-prng migration commit therefore bumps `SAVE_FORMAT_VERSION` 1→2
  (reject-not-migrate per D-SAVE2; acceptable pre-1.0), recomputes the
  committed golden hash in the same commit, and narrows the engine seed
  parameter to u32 at the API surface; `ContainerHeader.seed` stays u64
  (provenance, zero-extended) so the container layout doesn't churn.

### D-OR2 — Oracle hosts: a pinned dosbox-staging trace hook is tier 1 (v1's picks demoted)

- **Tier 1 (ground truth): a pinned local branch of dosbox-staging 0.82.2**
  (the emulator this project already uses) carrying a ~30-line trace hook in
  the normal-core dispatch: when CS:IP matches the pinned RandNext location
  **and the prologue bytes match** (self-locating; robust to load base and,
  if it ever mattered, overlay swaps), append `{before, after, ss_sp_words}`
  for `DS:0x47F0` as JSONL to an env-var path; a second env var performs a
  one-shot seed poke at first hit; a third hooks `Randomize` and marks the
  trace if it ever fires post-boot. `AUTOTYPE` scripts the fixed input;
  `core=normal` + fixed cycles pinned in the rig conf. The branch lives in
  `~/src/goldbox-refs` beside the coab fork (never in our repo; the hook
  references only our own pinned offsets — D10-clean). *Why the change:*
  review established that **neither** DOSBox-X's nor staging's debugger is
  scriptable — both inherit the same interactive TUI (`debug.cpp`); every
  breakpoint hit is a manual curses stop (plus an open macOS break-in bug in
  DOSBox-X). v1's "DOSBox-X ships machinery we can drive" was false; the
  tiny owned patch is less total work than puppeting a TUI and is
  deterministic and headless. DOSBox-X (Homebrew, debugger enabled) is kept
  only as a *manual inspection* aide (`MEMFIND`, one-off memory reads).
- **Tier 2 (contingent, was "cheap"):** an instrumented coab fork emits
  action-level structural traces — but coab is a WinForms .NET 4.5.2 app;
  on this machine (darwin/arm64, no mono/dotnet, Mono WinForms effectively
  dead on macOS) it does **not** run. Reworded: tier 2 is spun up only if H4
  bisection stalls on structure, and costs a Windows VM or a bespoke
  headless engine harness. Nothing on the critical path may assume it.
- **Tier 3 (aide):** GBC's published memory maps corroborate combatant-struct
  offsets for tier-1 endstate watches (D-OR5b). Documentation only.

### D-OR3 — The trace format: canonical JSONL owned by `gbx-oracle`, projection-based comparison

- **Ownership** (new in v2): the `.gbxtrace` types, writer, comparator, and
  replay driver live in `crates/gbx-oracle` (per PLAN §2's reservation);
  `restrike trace-compare` is a thin CLI wrapper; `gbx-engine` emits through
  a sink trait so the core stays pure.
- **Canonical encoding** (new in v2): fixed field order per event type, no
  insignificant whitespace, integers only (fixed-point for fractional
  quantities) — trace files are byte-hashable (the H1 hashes-only pattern).
- Header: `{"gbxtrace": 1, "profile": "prng"|"action", "game": "cotab-v1.3",
  "seed": u32, "encounter": ..., "source": "restrike"|"staging-hook"|"coab-fork",
  "notes": ...}`. For a comparison to be valid, `gbxtrace`/`profile`/`game`/
  `seed`/`encounter` must match; `source`/`notes` ignored.
- **`prng` profile:** `{"e":"rng","before":u32,"after":u32}` plus optional
  `"n"`/`"result"` (when the emitter knows the wrapper operand) and optional
  `"caller"` — **diagnostic only, excluded from equality**, normalized to a
  decompressed-image offset when the load base resolves it (raw + flagged
  otherwise). Equality surface: `(before, after)`, extended to
  `(n, result)` when both sides carry them. Rationale: a runtime seg:ofs can
  never equal restrike's synthetic tags, and overlay-resident return
  addresses aren't even stable run-to-run.
- **`action` profile:** semantic events (`init`/`attack`/`dmg`/`move`/`ai`/
  `status`/`award`, vocabularies pinned as combat systems land). Emission
  order for same-tick events is part of the format contract.
- **Versioning** (corrected — v1's "additive growth allowed" contradicted an
  exact comparator): comparisons run over a declared event-type/field
  projection (default: the intersection of what both sources' declared
  versions emit), exact equality within it. Additive changes bump a minor
  version and regenerate in-repo goldens in the same commit; renames or
  semantic changes bump major and reject on mismatch (mirroring D-SAVE2).
- **D10 / CI posture** (stated explicitly now): real-encounter traces live
  under `GBX_DATA_DIR`/local only. CI trace goldens are synthetic,
  restrike-vs-restrike regression locks; **all differential H4 value is
  local** (H1/H5 precedent). CI may additionally pin SHA-256s of locally
  verified oracle traces.

### D-OR4 — H3 acceptance, restructured: hermetic emulation closes the math; one live session closes the environment

*(v1's "ten manual DOSBox boots through copy protection" was the wrong place
to spend the seeds — each boot burns wheel-prompt draws (`ovr004`'s
Random(26)/(22)/(3)/(6)) and an answer per attempt, and v1's roll-heavy
target (training hall) isn't even reachable from the bundled save, which has
no naturally trainable member.)*

- **Part A (hermetic, closes stream + return-path equality):** a local-tier
  Rust test maps the decompressed image under a CPU emulator library
  (`unicorn-engine` crate, works on darwin/arm64), seeds `DS:0x47F0` = K,
  calls the integer wrapper at `0x8F7:0x15EA` with chosen N, and compares
  `(state', AX)` against `gbx-prng` — for ten thousand (K, N) pairs, no
  boot, no copy protection, no wall clock. Closes FD-26's return path and
  the stream math. Gated on `GBX_DATA_DIR` like every real-data test.
- **Part B (live, closes the environment):** **one** session under the
  D-OR2 staging hook: boot → past copy protection → poke seed → a scripted
  roll-heavy window (character-creation stat **rerolls**: fixed keystrokes,
  reachable from a fresh boot, zero prerequisites) → captured prng trace.
  The comparator verifies **chain continuity** — `after_i == step(before_i)`
  and `before_{i+1} == after_i` — so a mid-session reseed, a missed hook
  hit, or a foreign write to `DS:0x47F0` is *detected by the test*, not
  assumed away (this also makes part B independent of FD-27's answer while
  confirming it). Our side replays the same window via `gbx-prng` and must
  predict the full stream and results.
- H3 closes on A + B together.

### D-OR5 — What H4 "traces match" means (new in v2; v1's definition was circular)

v1 defined H4 as action-profile equality — an artifact no permitted tier
produces (tier 1 emits rng events; tier 2 was barred from closing dockets).
Redefined as a three-part composite:

- **(a) Ground truth, gate-closing:** rng-profile equality vs the tier-1
  hook — from a poked seed through a fixed input script, over N seeds × M
  encounters, on the D-OR3 equality surface with chain continuity.
- **(b) Endstate equality, gate-closing:** the tier-1 hook additionally
  dumps pinned combatant-struct memory (tier-3 GBC offsets, verified against
  coab's `[DataOffset]`s) at defined checkpoints — round boundaries and
  fight end. Our engine's state at the same checkpoints must match
  field-for-field. This is what makes action *semantics* falsifiable against
  ground truth without oracle action events. Promoted from v1's §5
  "only if needed" to half of the H4 definition.
- **(c) Structural, advisory:** our action profile vs a tier-2 coab-fork
  trace, if tier 2 is ever stood up. Bisection aid; never closes a docket.

PLAN §3's H4 wording ("instrument the oracle to emit per-action JSON
traces") predates this door and is superseded by (a)+(b); the M4 exit-gate
"traces match the oracle exactly" = (a)+(b) for ≥10 seeds. PLAN annotated.

## 3. What this absorbs from the deferred human-validation list

Unchanged from v1 — the tier-1 rig retires the parked items (drained-stat
save for FD-23 item 1, mid-combat menu check, capture screenshots via the
existing staging rig, code-wheel `input_expected` read-out for the M6
prerequisite — the last via a one-off memory read at the prompt, which the
hook branch or DOSBox-X manual mode both support).

## 4. M4 build order (session-sized steps)

1. **`gbx-prng` + pins + save bump** (session): the crate per D-OR1
   (binary-exact `random`, no short-circuits); the `[0xa55a,0xa5ee)` hash
   re-derivation local-tier test; the audited per-site migration checklist
   (incl. dither → position-hash, FD-28 filed); `SAVE_FORMAT_VERSION` 2 +
   golden recompute; seed narrows to u32.
2. **D-OR4 part A** (same or next session): the unicorn-engine local-tier
   acceptance test, 10k (K, N) pairs.
3. **The staging hook branch + D-OR4 part B** (Bryan + Fable): ~30-line
   patch on a pinned 0.82.2 branch in `~/src/goldbox-refs`; one live
   session; absorbs §3's human items opportunistically.
4. **Trace plumbing** (session): `gbx-oracle` (format types, canonical
   writer, projection comparator, replay driver), engine sink, CLI wrapper,
   synthetic goldens in CI.
5. **Combat systems** (sessions): map gen, initiative, action economy, … —
   each landing with its action-profile events and, where GBC/coab offsets
   allow, its D-OR5b endstate checkpoint fields, keeping H4 continuously
   checkable.

## 5. Open questions → docket

- **FD-26** (updated): integer semantics now settled statically (modulo,
  draw-always); D-OR4 A/B provide the executable confirmation.
- **FD-27** (updated): statically answered — one boot-time `Randomize`, no
  overlay RNG copies; D-OR4 part B's chain-continuity check is the dynamic
  confirmation.
- **FD-28** (new): does the original's fade dither draw from `DS:0x47F0` at
  all (coab uses a separate time-seeded RNG for it — unfaithful either way)?
  Settled by a tier-1 trace across a fade; until then our dither is a
  deterministic position hash and dither pixels stay non-comparable.
- Combat-overlay memory map for D-OR5b checkpoints: built incrementally per
  combat system from coab `[DataOffset]`s + GBC maps, verified by the first
  endstate comparisons.
