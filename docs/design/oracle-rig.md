# The M4 oracle rig: bit-exact PRNG (H3) and combat-trace equality (H4)

Status: **v3** (Fable-authored 2026-07-15; v1 → three-lens round → v2 →
bounded round 2 on the new surface → v3 same day). Round 1 refuted two v1
claims (integer `Random(N)` semantics; the cs-base constant) and demoted both
oracle-host picks. Round 2 killed v2's `unicorn-engine` tier (does not build
on this machine's toolchain — verified empirically — and its dev-dep would
redden the tri-OS CI) and v2's D-OR5(b) as written (combatants are
heap-pointer-chased, not pinned addresses — the watch was circular with the
RE it validates). v3's replacements are reviewer-designed and
source-verified: a purpose-built 8086 stepper (D-OR4A), a two-trigger
offset-keyed hook (D-OR2), and a projection-based, RE-gated endstate
checkpoint with a verified `combat_round` trigger and a QuickFight-first
bootstrap (D-OR5). Review closes here; the step-1/step-2 builds validate
empirically from this point.

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
  (the emulator this project already uses) carrying a small trace hook in
  the normal-core per-instruction dispatch (`core_normal.cpp`'s
  fetch/decode/execute loop — verified a true per-instruction interpreter at
  0.82.2, and `core=auto` already picks it for real-mode games, so forcing
  `core=normal` costs nothing). **Two trigger points, keyed on the
  load-base-invariant in-segment offset + a byte signature at CS:IP** (round
  2 corrected v2 on both counts — one point cannot observe both `before` and
  `after`, and an absolute CS:IP compare breaks under DOS relocation):
  `reg_ip == 0x1639` + RandNext prologue bytes → log `before = [DS:0x47F0]`
  and `ss_sp_words`; `reg_ip == 0x166E` + the `ret` signature → log
  `after = [DS:0x47F0]`. (Capturing `after` independently is what makes the
  chain-continuity check non-trivial; computing it hook-side would
  re-implement the LCG and forfeit independence.) JSONL to an env-var path;
  a second env var performs a one-shot seed poke at first entry-hit; a third
  trigger on `Randomize` (`reg_ip == 0x1671`) marks the trace if it fires
  post-boot. DS is trustworthy at these offsets: a TP program's DS is the
  fixed global data segment and RandNext itself depends on that invariant.
  `AUTOTYPE` (verified present in 0.82: `-w` initial wait, `-p` pace)
  scripts the fixed input — it is open-loop/timing-based, so a dropped
  keystroke is possible; chain-continuity + the Randomize flag turn a bad
  run into a *detected re-run*, never silent corruption. Fixed cycles pinned
  in the rig conf. The branch lives in `~/src/goldbox-refs` beside the coab
  fork (never in our repo; the hook references only our own pinned offsets —
  D10-clean). Bring-up note: a local source build needs `meson`, `ninja`,
  `pkg-config` (+ the formula's dep set) installed first — a few hours, once.
  *Why not a stock debugger:* round 1 established **neither** DOSBox-X's nor
  staging's debugger is scriptable — both inherit the same interactive TUI
  (`debug.cpp`); every breakpoint hit is a manual curses stop (plus an open
  macOS break-in bug in DOSBox-X). DOSBox-X (Homebrew, debugger enabled) is
  kept only as a *manual inspection* aide (`MEMFIND`, one-off memory reads).
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
  Rust test executes the **actual RNG-cluster bytes from the user's
  decompressed image** under a purpose-built minimal real-mode 8086 stepper
  (the cluster is ~150 bytes over ~20 distinct opcodes: `mov`/`xchg`/`mul`/
  `div`/`shl`/`add`/`adc`/`jz`/`call`/`ret(f)`), seeds the emulated
  `DS:0x47F0` = K, calls the integer wrapper at `0x8F7:0x15EA` with chosen
  N, and compares `(state', AX)` against `gbx-prng` — for ten thousand
  (K, N) pairs, no boot, no copy protection, no wall clock. **Independence
  rule:** the stepper is written to *generic x86 semantics* (`div` =
  DX:AX ÷ BX → AX quotient / DX remainder, `#DE` on overflow; never
  "shaped" to the RNG), so it still catches a wrong multiplier, mod-vs-scale
  errors, or a bad `Random(0)` short-circuit. Pure Rust, zero deps,
  compiles on every CI leg and wasm-clean; runtime-gated on `GBX_DATA_DIR`
  like every real-data test. *(Round 2 killed v2's `unicorn-engine` pick
  empirically: the crate's vendored QEMU does not compile under this
  machine's toolchain — Apple clang 21 / SDK 26, latest crate version — and
  as a dev-dependency it would have been compiled by the tri-OS CI `test`
  job regardless of the runtime gate, reddening at least the arm64 macOS
  leg. The reviewer's recommended stepper is strictly cheaper and keeps the
  independent-execution value.)*
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

### D-OR5 — What H4 "traces match" means (v3: reweighted after round 2 broke (b)'s premise)

v1 defined H4 as action-profile equality — an artifact no permitted tier
produces. v2 split it into rng-stream + endstate halves, but specified (b)
as "pinned combatant-struct memory dumps," which round 2 refuted: in the
original, combatants are a **heap linked list** (coab `Gbl.cs` `TeamList`
with its `player_next_ptr` annotation; monsters `ShallowClone`d per
encounter, up to 63), initiative/turn state hangs off a *second*
pointer-chased `Action` struct with **no** `[DataOffset]`s at all, and the
only fixed-address combat array (`CombatMap`, `seg600:66BD`) holds grid
geometry only. There is no address to pin; a watch requires the very combat
RE it was meant to validate. GBC doesn't rescue it: GBC *signature-scans*
DOSBox memory for the character block each session — it corroborates
**intra-record field layout** (as coab's `[DataOffset]`s already do), never
struct addresses. v3:

- **(a) Ground truth, gate-closing, carries H4's weight:** rng-profile
  equality vs the tier-1 hook on the D-OR3 equality surface with chain
  continuity, over ≥10 seeds × M encounters. Credited properly now:
  **draw-order parity settles FD-2 by itself** — the original *consumes*
  initiative ordering (`CalculateInitiative`'s d6+DexReact rolls, then
  `FindNextCombatant`'s per-pick d100 re-rolls against `delay`,
  `ovr009.cs:59-99`) and never persists it, so two orderings can share an
  endstate; only the draw stream distinguishes them.
  **Bootstrap order (mandatory — a fixed input script through combat
  otherwise presupposes the parity it validates):**
  1. *Phase 0 — observe-only, all-AI:* both sides on QuickFight (coab
     `quick_fight` @ Player+0x198, `SetPlayerQuickFight`/`PlayerQuickFight`,
     `ovr009.cs:707`/`ovr010.cs:8`) — the only scripted input is the
     keystroke that triggers the fight, so `AUTOTYPE` cannot desync.
     Capture oracle rng-stream (+ checkpoints when (b) exists).
  2. *Phase 1 — replay to parity:* implement combat until our headless
     replay matches Phase-0 draw order for ≥10 seeds. AI-decision parity
     closes here.
  3. *Phase 2 — scripted player turns:* only after Phase 1 holds; parity
     guarantees AI-turn shape, so fixed menu scripts stay synced.
- **(b) Endstate checkpoints — RE-gated, promoted to gate-closing only when
  its walk is pinned and validated:** a **structure-walk snapshot**, not an
  address dump: locate the `TeamList` head global, walk the
  `player_next_ptr` chain, decode each node per the (known) 0x1A6 record
  layout plus the (unpinned, needs live RE) `Action` struct via the pointer
  at +0x18d. The walk's prerequisites — list-head address, in-memory node
  layout vs the on-disk record, `Action` layout — are their own RE
  deliverables, sequenced *before* (b) can close anything. **Checkpoint
  trigger (verified fixed address):** a hook watch on the round counter
  `combat_round` = `byte_1D8B7` (data-segment global, coab `Gbl.cs:382`;
  incremented in `BattleRoundChecks`, `ovr009.cs:366`) → snapshot per round;
  fallback until the walk is pinned: fight-end only. **Equality is over a
  declared, versioned checkpoint projection** (the D-OR3 discipline; v2's
  "field-for-field" was undefined across the representation gap — our party
  model holds opaque cells and reconstructed sets, and has no runtime combat
  struct yet): `{combatant_id, hp_current, hp_max, status, grid_pos,
  attacks_left, ac}` + a per-round **turn-order list** (acted combatant ids
  with rolled delays) so ordering is checkpoint-visible too.
- **(c) Structural, advisory:** unchanged — our action profile vs a tier-2
  coab-fork trace, if ever stood up. Never closes a docket.

**Settling FD-1..FD-4 concretely:** FD-2 → (a) draw order (above). FD-3 →
(b) round-start `attacks_left` + (a) draw count. FD-1 and FD-4 → need
**round-granular** checkpoints *plus curated minimal encounters* (one
attacker, one target) so an HP delta or status transition is attributable
to a specific draw — fight-end endstate cannot attribute either. The M4
exit-gate encounter set must include these curated fights.

PLAN §3's H4 wording ("instrument the oracle to emit per-action JSON
traces") predates this door and is superseded; the M4 exit-gate "traces
match the oracle exactly" = (a) for ≥10 seeds, plus (b) at whatever
checkpoint granularity is pinned by then (fight-end minimum). PLAN
annotated.

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
2. **D-OR4 part A** (same or next session): the purpose-built 8086 stepper
   + acceptance test, 10k (K, N) pairs. Pure Rust, generic-semantics rule
   per D-OR4A.
3. **The staging hook branch + D-OR4 part B** (Bryan + Fable): the
   two-trigger patch on a pinned 0.82.2 branch in `~/src/goldbox-refs`
   (bring-up: install meson/ninja/pkg-config + formula deps first); one
   live session; absorbs §3's human items opportunistically.
4. **Trace plumbing** (session): `gbx-oracle` (format types, canonical
   writer, projection comparator, replay driver), engine sink, CLI wrapper,
   synthetic goldens in CI.
5. **Combat systems** (sessions), in D-OR5(a)'s bootstrap order: Phase-0
   observe-only QuickFight captures first, then implement-to-parity, then
   scripted player turns — each system landing with its action-profile
   events. The (b) structure-walk RE (TeamList head, in-memory node layout,
   `Action` struct) proceeds alongside; the `combat_round` watch +
   fight-end checkpoints come first, round-granular + curated 1v1
   encounters (FD-1/FD-4) once the walk is pinned.

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
- **The D-OR5(b) structure-walk prerequisites** (each its own RE
  deliverable, tracked in the docket when M4's combat work opens them): the
  `TeamList` head global's data-segment address; the in-memory combatant
  node layout (the 0x1A6 record fields are known — the runtime pointer
  fields and any in-combat-only cells are not); the `Action` struct layout
  (coab reconstructs it without `[DataOffset]`s — needs live RE against the
  hook). GBC corroborates intra-record field layout only — never addresses.

## 6. The migration ledger (M4 step 1 — additive, not a door change)

D-OR1(d): the migration from the splitmix64 placeholder (`EngineRng`, killed
this session) to `gbx-prng` is an **audited per-site checklist**, each site
mapped to its original citation, **not a mechanical rename**. This section is
the record a future session audits the migration against.

**The trap.** The old `VmRng::roll_uniform(inclusive_max)` took an *inclusive*
bound (`0..=max`); the binary's `random(n)` (oracle-rig §1, image `0xa55a`)
takes an *exclusive* one (`0..n`) and **always draws** (including `n == 0`,
returning 0 after drawing — no short-circuit, D-OR1(b)). The naive translation
`roll_uniform(k) → random(k + 1)` is correct at some sites and **wrong at
others**; each row below states whether the translation was *mechanical* (the
old and new expressions denote the same range) or *corrected* (the old
expression was itself a bug the migration fixes).

The `VmRng` trait method was renamed `roll_uniform(inclusive_max) → random(n)`
(exclusive, draw-always) so every call site expresses the binary operand
directly rather than juggling `±1`. `EngineRng` is now a thin wrapper over
`gbx_prng::Prng`; the state (`u32`, the `DS:0x47F0` dword) is what `.rsav`
serializes and the oracle rig pokes.

| # | Site | Original citation | Old expression | New expression | Kind | `n == 0` reachable? |
|---|---|---|---|---|---|---|
| 1 | `gbx-engine/src/vmhost.rs` `EngineServices::roll` | `seg051.Random(max)` = `Next() % max`, exclusive (`seg051.cs:33-40`); `CMD_Random` pre-increments (`ovr003.cs:132-151`, mirrored `machine.rs` `op_random`) | `roll_uniform(max)` = `0..=max` | `random(max)` = `0..max` | **corrected** (off-by-one; old could return `operand+1`) | **No** — `op_random` does `rand_max.saturating_add(1)` before calling, so `max ≥ 1` always. |
| 2 | `frontends/cli/src/run_script.rs` `CliHost::roll` (+ the whole `CliHost` RNG) | same as #1 | `roll_uniform(max)` (an inherent xorshift64\*, `RNG_SEED` — a **second RNG**, D-OR1-forbidden) | `self.rng.random(max)` over `gbx_prng::Prng` | **corrected** (same off-by-one; also removed the second generator) | **No** — same `saturating_add(1)` path (this host shares the VM's `op_random`). |
| 3 | `gbx-engine/src/vmhost.rs` `EngineServices::roll_dice` | `roll_total += Random(dice_size) + 1` per die (`ovr024.cs:586-598`) | `1 + roll_uniform(size - 1)`, with `size == 0` short-circuiting to `+1` **without drawing** (matched coab, not the binary) | `1 + random(size)` | **mechanical** for the range; **corrected** for the `size == 0` draw (now draws, per binary) | **Yes** — `size == 0` → `random(0)` draws then returns 0 → die value 1. Correct and intended. |
| 4 | `frontends/cli/src/run_script.rs` `CliHost::roll_dice` | same as #3 | `1 + roll_uniform(size - 1)` (over the second RNG) | `1 + self.rng.random(size)` | same as #3 | **Yes** — same as #3. |
| 5 | `gbx-engine/src/movement.rs` `roll_die` (door bash) | `roll_dice(size, 1)` = `Random(die_size) + 1` (`ovr024.cs:586`; bash `ovr015.cs:180-215`) | `roll_uniform(die_size - 1) + 1`, **underflow-panics** at `die_size == 0` | `random(die_size) + 1` | **mechanical** for the range; **corrected** for the panic (subtraction removed) | **Yes** — `die_size` from `bash_outcome`'s table; `random(0)` now safely draws → 1 instead of panicking. |
| 6 | `gbx-engine/src/training.rs` `EngineRoller::roll` (the `gbx-rules` `Roller` adapter) | `roll_dice(size, count)` shape (`ovr024.cs:586-598`) | `size.max(1)`, then `roll_uniform(size - 1) + 1` per die | `random(size) + 1` per die (`.max(1)` guard dropped) | **mechanical** for the range; **corrected** for the `size == 0` draw | **Yes** — `size == 0` → `random(0)` draws → 1. Training dice are never 0 in practice; the guard is gone because draw-always is the faithful behavior. `gbx-rules` gains **no** `gbx-prng` dep — the `Roller` trait stays, only `EngineRoller`'s body re-points. |
| 7 | `gbx-engine/src/draw.rs` `apply_recolor_dithered` (fade dither) | `DaxBlock.Recolor(useRandom=true)`, `(random_number.Next() % 4) == 0` (`DaxBlock.cs:84`) — coab's **separate** time-seeded RNG | `roll_uniform(3) == 0` per changed pixel, from the one engine PRNG | deterministic position hash `dither_hit(index)`, **no PRNG at all** (signature loses `&mut dyn VmRng`) | **corrected** (D-OR1(c)/FD-28: a framebuffer-content-dependent draw count would desync any traced window) | **N/A** — no PRNG draw. See FD-28. |

**Sites deliberately left fixed/fake (judged on their own terms, D-OR1(d)
item 7).** `gbx-vm/src/test_support.rs` `FixedRng` and any other test doubles
are scripted sequences, not the generator — they were renamed
`roll_uniform → random` and given exclusive-clamp semantics but stay
deterministic fakes. `draw.rs`'s old `AlwaysZeroRng`/`NeverZeroRng` doubles were
**deleted** (the dither no longer takes an RNG). No production code holds a
generator other than `gbx-prng` after this session.

**The `roll` off-by-one — CONFIRMED, fixed (rows 1, 2).** The reasoning was
re-verified against both citations: `op_random` pre-increments the operand
(`rand_max.saturating_add(1)`) so the *script's* intended range for operand `v`
is inclusive `0..=v`; that requires `roll(v+1)` to yield `0..=v`, i.e. `roll`
must be *exclusive*. `roll_uniform(max)` was inclusive `0..=max`, so
`RANDOM x, v` could write `v+1` — one above the script's maximum. `random(max)`
restores the exclusive bound. The mechanical rename `roll_uniform(max) →
random(max+1)` would have **frozen the bug** — the whole reason D-OR1(d) forbids
a mechanical rename.

**The `roll_dice` byte-truncation — docketed, not fixed (FD-29).** coab returns
`(byte)roll_total` (`ovr024.cs:595`, the original's `AL` return); our `roll_dice`
returns `u16` and never truncates. No shipped call site sums `> 255` in one
`roll_dice`, so the two are bit-identical for every reachable input; narrowing
the return to `u8` would churn the `VmHost` trait for no behavioral gain yet.
Filed as **FD-29**.

**Creation-roll draw ORDER — docketed, not touched (FD-30).** The original
interleaves the six stats within each reroll iteration (`ovr018.cs:675-683`);
our per-stat `Flavor::roll_ability_score` cannot express that ordering. No
production caller exists yet, so nothing is broken — but D-OR4 part B's live
window *is* creation rerolls, so this must be resolved when creation lands.
Filed as **FD-30**. No flavor-API redesign this session (out of scope).
