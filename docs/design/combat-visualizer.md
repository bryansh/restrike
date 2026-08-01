# Design: The Combat Visualizer (M6) — watching fights, then fighting them

**Status: v2 — drafted and adversarially reviewed 2026-08-01 (the day the M5
§46 exit gate passed, guard 15/15). Two independent review lenses ran
(architecture attack + line-level evidence audit): no model-level blocker;
eight design gaps and three evidence errors found, all folded in below (the
per-finding trail is in the session journal). Exit gate is a PROPOSAL until
Bryan ratifies it.**

Working-ledger milestone **M6 = the visualizer + the manual-combat UI** (per
the §46 ruling in `h4-entry-state-snapshot.md`). Note on numbering: PLAN.md's
original table calls M6 "Roll credits — CotAB completable"; the working ledger
inserted combat milestones (M4/M5) and this one after M3, so PLAN's M6/M7/M8
shift right. A one-line annotation rides this doc's commit; PLAN's tables are
otherwise left as history.

**The goal (from §46):** stop reading draw streams and start watching fights.
The visualizer renders combat from the engine's event stream; the
manual-combat UI rides on the same stream and the same screen, with
player-driven turns instead of QuickFight AI. It exists to *accelerate* the
circle-back ledger (doc §50.5/§50.6): staging, triaging, and debugging future
captures by watching them.

**What this doc is:** the M6 door — placement, the render-feed contract, the
interactivity suspensions, the fidelity boundary for live fights, milestones,
and the exit-gate proposal. Presentation-layer *transcription detail* (exact
per-message coordinates, per-weapon missile rows) is inventoried in §1 with
citations and transcribed at implementation, not re-derived here.

---

## 1. The original combat presentation, as read in coab

All citations are coab (`~/src/goldbox-refs/coab/`), read-for-behavior
2026-08-01. Text units = 8 px cells (40×25 on the 320×200 screen); hex as in
source. Listing verification (`coab_new.lst`) is deferred to implementation
for any site that becomes load-bearing to *logic* (the M5 rule); pure
presentation sites may trust coab with a docket note (§6).

### 1.1 Layout and palette

- Combat **fully replaces** the exploration screen: `RedrawCombatScreen`
  (`ovr025.cs:1514`) = `Color_0_8_inverse()` → `seg037.DrawFrame_Combat()` →
  full `redrawCombatArea(8, 0xff, center)`. Exit restores via
  `free_combat_stuff` (`ovr009.cs:9`) + `CMD_Combat`'s `LoadPic` rebuild
  (`ovr003.cs:971`).
- **Palette swap**: `Color_0_8_inverse` (`ovr033.cs:44`) swaps EGA registers
  0↔8 for the whole fight — the classic grey combat backdrop. Toggled back
  around the mid-combat View character sheet (`ovr020.cs:240,334`).
- **`DrawFrame_Combat`** (`seg037.cs:151`): clear rows 0–0x17 / cols 0–0x27;
  borders at row 0, row 0x16, and vertical cols 0/0x16/0x27, from 8×8 symbol
  ids `0x11E+n` (tables at `seg037.cs:7-27`), set 4 = `8X8D{area}.DAX` block
  0xCA (`ovr038.cs:8-52`, `seg001.cs:309-310`).
- Regions: **map viewport** cols 1–21 / rows 1–21 = pixels (8,8)–(175,175),
  hard-clipped in `seg040.draw_combat_picture` (`seg040.cs:115-118`);
  **right panel** rows 1–0x15 / cols 0x17–0x26 (`seg041.cs:119-123` —
  our `COMBAT_SUMMARY` region, `text.rs:58-63`, matches); **status line**
  row 0x17 ("Range = N", "Spell:<name>"); **prompt/menu line** row 0x18
  (`ClearPromptArea`, `ovr027.cs:344-354`; menu rendering `displayInput`,
  `ovr027.cs:126-341`).
- **Right panel** (`CombatDisplayPlayerSummary`, `ovr025.cs:292-335`, drawn at
  every turn start, `ovr009.cs:124`): name (color 0x0B party / 0x0E enemy /
  0x0C removed, `ovr025.cs:827-847`), Hitpoints (green full / yellow damaged,
  `ovr025.cs:270-289`), AC, readied weapon, health status. **Messages** print
  in the same panel from row 10 down (`DisplayPlayerStatusString`,
  `ovr025.cs:787-811`).

### 1.2 The map view

- **7×7 cells at 24×24 px each**; screen cell (cx,cy) → pixels
  (8+24·cx, 8+24·cy) (`ovr033.cs:316-334` with `IconColumnSize=3`;
  `seg040.cs:23-26`; `ovr034.cs:90-98`). Matches our modeled camera
  (`SCREEN_MAX=6`, `combat/mod.rs:1858-1861`).
- **Camera** = `ScreenMapCheck(radius,pos)` (`ovr033.cs:265-341`): recenter
  only when `pos` leaves the radius box; centre clamped x∈[3,46], y∈[3,21];
  recenters are a single full redraw, **no scroll animation**. Radii: 2 at
  turn start, 3 for missile/spell focus, 0xff = forced full redraw
  (`ovr009.cs:121,393`). All sites already modeled draw-exactly in
  `combat/facing.rs:48-153`.
- **Tiles**: map cell value → `BackGroundTiles[value]`
  `{move_cost,f1,f2,tile_index}` (74 entries, `Gbl.cs:194-267` — the same
  table our `unk_189B4` cost column came from, "74 entries transliterated"
  per `combat/mod.rs:1921`) → `DrawIsoTile`
  (`ovr034.cs:30-40`) blits a **24×24 atlas frame** (`dax24x24Set`, 48 slots,
  `seg001.cs:214`). Tile art (`SetupGroundTiles`, `ovr011.cs:757-784`):
  dungeon = **DUNGCOM.DAX** block 1 (0x19 tiles), wilderness = **WILDCOM.DAX**
  block 1 (0x21 tiles), always + **RANDCOM.DAX** block 1 (6 tiles: table,
  chair, cloudkill, [unnamed], stinking cloud, **dead-body tile** — our
  `Tile_DownPlayer` 0x1F).
- **Dungeon floor generation** (`SetupDungeonFloor`, `ovr011.cs:500-522`):
  projects GEO wall data (dx −6..6, dy −2..2 around the party) into combat
  cells via `combatX = dx*6 + dy*5 + 21 + x`, `combatY = dy*5 + 10 + y`
  (`ovr011.cs:11-23`) — an oblique shear where E–W walls run horizontal
  (tiles 5/10) and N–S walls run as diagonal tile runs (4/3/13 stepping
  (+1,+1) per row; `ovr011.cs:149-173`); doors/corners from dedicated tables.
  **Draw-bearing**: tables/chairs inside "building" cells roll 50%/90% dice
  (`sub_370D3`, `ovr011.cs:28-93`). Wilderness floors
  (`SetupWildernessFloor`, `ovr011.cs:551-754`) are heavily random
  (roads/rivers/scatter) — deferred with the wilderness circle-back item.

### 1.3 Combatant icons

- **Store**: `combat_icons[26]` (`seg001.cs:227`); each icon = **2 frames
  (Normal, Attack)** + cached horizontal mirrors; 8-way facing renders as
  base art for directions 0–3 and the mirror for 4–7
  (`Classes/Combat/CombatIcon.cs:56-66`).
- **Slots**: 0–7 party; 8+ per-monster-type (assigned per LOADMONSTER,
  `ovr003.cs:238-297,763`); 13–24 = **COMSPR.DAX** blocks 0–11
  (missiles/effects); 25 = COMSPR block 0x19, the **grey focus-box cursor**
  (`seg001.cs:312-317` — the "13 COMSPR blocks" our boot stubs,
  `boot.rs:9-11`).
- **Party icons** = **CHEAD.DAX**[head_icon] + **CBODY.DAX**[weapon_icon]
  merged (+0x40 blocks for tall size-2 icons, +0x80 for Attack frames), then
  recolored: template colors {1,2,3,4,6,7} → `icon_colours[6]`, low nibble =
  base color, high nibble = +8 (`ovr034.cs:52-87`,
  `ovr017.LoadPlayerCombatIcon:86-122`). **Monster icons** =
  **CPIC{area}.DAX**, block id from LOADMONSTER's 3rd operand (recorded but
  unconsumed by us today — `combat-study.md:789-796`), Attack = +0x80; the
  CPIC path's `Recolor` runs with identity tables (`ovr034.cs:80-81`) — a
  functional no-op, so monsters render unrecolored.
- 4bpp, mask color 0 → transparent (`DaxBlock.cs:124-159`); size-1 icons are
  24×24 (height 24 × width 3 cols); multi-cell sizes are data-driven from the
  DAX headers (footprints per the `Steps` table, `ovr033.cs:10-16` — matches
  our §47.6 size decode).
- **Downed party members** leave the body *background tile* (RANDCOM), with
  original-tile save/restore (`CombatantKilled`, `ovr033.cs:578-592`;
  `sub_7515A:614-668`); monsters just vanish. The **turn/aim indicator** is
  the grey focus box drawn under the icon on the acting/targeted cell(s)
  (`sub_7416E`, `ovr033.cs:58-83`).

### 1.4 Animation and pacing

Everything is wall-clock sleeps in coab; our conversion is D-UI1's
`ticks = max(1, round(ms·60/1000))`. `game_speed_var` default 4
(`seg001.cs:274`), player-settable 0–9 mid-combat (Speed menu,
`ovr009.cs:672-704`).

| Beat | Quantity | Site |
|---|---|---|
| `GameDelay` message beat | `speed×100` ms (400 → 24 ticks) | `seg041.cs:335-339` |
| Attack pose hold | 100 ms → 6 ticks | `ovr014.cs:904-1008` |
| Missile step (arrow/dart/quarrel/spear) | 10 ms/8-px step, 1 static directional frame | `ovr014.cs:1590-1671` |
| Missile step (axe/club/oil, 4-frame spin) | 50 ms | same |
| Missile step (sling, 2-frame) | 10 ms | same |
| Spell projectile (icon slot 0x12 = COMSPR block 5) | 30 ms/step | `ovr023.cs:741-762` |
| Lightning bolt (icon slot 0x13 = COMSPR block 6) | **50 ms/step** | load `ovr023.cs:1958-1996`, draw `ovr023.cs:2052` |
| On-target burst (stars 0x16 / 0x17) | 70 ms/frame ×4, ×(speed+1) reps for stars | `ovr025.cs:1118-1172` |
| Death flash | 9 alternations of two COMSPR frames, 10 ms each, then `GameDelay` | `ovr033.cs:534-611` |
| Movement step | no timer — redraw radius 1 (3 in QuickFight) + step sound | `ovr014.cs:251-321` |
| Quick-fight engage | 200 ms | `ovr009.cs:181,274` |

- Missile flight runs in **8-px sub-cell units** over a Bresenham path
  (`draw_missile_attack`, `ovr025.cs:882-1115`) with save/blit/restore per
  step and mid-flight re-scroll (pan to attacker, replay tail
  target-centred, `ovr025.cs:1003-1087`). Our `facing.rs` models only the
  **persistent end-state** of that camera (`draw_missile_camera`,
  `facing.rs:103-155` — one `screen_map_check` to midpoint or
  target-anchored `center2`); the transient phase-1 pan never exists in
  engine state, so the scene recomputes it **presentation-locally** from the
  `Missile` event + presented endpoints (a coab transcription in the scene —
  legal, it draws nothing from engine state and rolls no dice; review
  finding).
  Missile sprite = a 4-frame buffer built from one COMSPR icon
  {Normal, Normal-flipped, Attack-flipped, Attack} (`ovr025.cs:873-879`).
- **Per-character slow text is OFF during combat** (`BattleSetup` clears
  `DelayBetweenCharacters`, `ovr011.cs:1171`; restored at `ovr009.cs:55`) —
  combat messages print whole, paced by `GameDelay` beats. The M2 `TextPacer`
  is not part of the combat scene.
- **No hit flicker / damage flash**: feedback = pose + sound + text only.
- Sounds at faithful sites (ids per `Gbl.cs:45-64`): missile launch 0x0C
  (arrows/darts/quarrels/spears, `ovr014.cs:1592,1630`; 9 for
  axe/club/glaive + default, 6 for oil/sling), magic-hit 3 (stars variant
  plays 4, `ovr025.cs:1133`), death 5, hit 7, miss 9, step 0x0A, spell-cast
  2/8/0x0B by spell. We emit `SoundEvent`s (D-UI1 `Frame.sounds`); synthesis
  stays M8.
- coab sleeps are **not keyboard-interruptible**; whether the original
  allowed keypress-skip is OPEN (§6) — we transcribe coab's behavior.

### 1.5 Messages and prompts

- Two channels: **panel messages** (right panel rows 10+, `GameDelay`-paced,
  never key-blocking) and **prompt-line messages** (row 0x18, print →
  `GameDelay` → erase, `string_print01`, `ovr025.cs:775-784`). Menus and
  `yes_no` block on keys; the wrap-printer key-blocks only on region
  overflow ("Press any key to continue").
- The attack sequence (`DisplayAttackMessage`, `ovr014.cs:113-223`):
  attacker name + "Attacks"/"-Backstabs-"/"slays helpless" (row 10), target
  name (row 12), then "Hitting for N point(s) of damage" / "and Misses" /
  "with one cruel blow" (+ "(from behind) "), then "goes down" +
  "and is Dying"/"is killed".
- String inventory (all cited in the research pass, transcribed at
  implementation): "A battle begins...", "Begins Casting", "Casts a Spell",
  "Spell:<name>", "Surrenders", "flees in panic", "is forced to flee",
  "Got Away", "Escape is blocked", "Flee:", "Guarding", "is bandaged",
  "Your Teammate is Dying", "Continue Battle:", "Magic On"/"Magic Off",
  aim/menu prompts, turn-undead and affect/heal messages
  (`ovr014.cs:609-687`, `ovr025.cs:1246-1259`, ovr013/ovr023/ovr024).

### 1.6 The entry/exit boundary

Combat entry art (encounter SPRIT sprite scaled by distance into the 3D
viewport, `ovr008.cs:220-276`; "You encounter…" script text) happens in the
**exploration** screen before `CMD_Combat` — already M2/M3 territory, not the
scene's. `BattleSetup` (`ovr011.cs:1169-1220`) prints "A battle begins...",
builds floor + placement, loads missile art, centres the camera, and draws
the combat screen. Exit = `AfterCombatExpAndTreasure` (`ovr006.cs:763`) →
`displayCombatResults` result screens (`ovr006.cs:381-440`) — **out of M6
scope** (XP/treasure is a circle-back
item); M6 ends a fight by restoring the exploration screen exactly as today's
headless path does, with the transcript line kept.

### 1.7 The manual combat UI (M6c surface)

- `combat_menu` (`ovr009.cs:313-360`): **"Move View Aim Use Cast Turn Quick
  Done"**, words conditional (Move iff moves left, Use iff items, Cast iff
  spells & allowed, Turn iff cleric). `Done` → "Guard Delay Quit Bandage
  Speed Exit" (`ovr009.cs:616-669`); Speed → "GameSpeed (N): Slower Faster".
- **Movement**: direction keys G/H/I/K/M/O/P/Q → dirs 7/0/1/6/2/5/4/3 with
  "Move/Attack, Move Left = N" (`ovr009.cs:416-588`); walking into an enemy
  attacks; stepping off-map prompts "Flee:". **Aim** (`ovr014.cs:1752-2060`):
  Next/Prev/Manual/Target/Center/Exit cycling targets with the grey focus
  box + live right-panel summary + "Range = N"; Manual = free cursor moved
  cell-by-cell. **Quick** hands the side to `PlayerQuickFight`; during AI
  turns SPACE revokes quick-fight, '2' toggles auto-magic (the §38 toggle —
  in live play it is just a keypress again).

---

## 2. What already exists (and the gaps)

**In hand** (survey 2026-08-01, citations in-repo):

- `CombatState::step` — turn-granular tick core (`RoundStarted`/`Turn`/
  `RoundEnded`/`Ended`), pure, capture-proven 15/15. Fine-grained visibility
  already exists as **`ActionEvent` via `ActionSink`** (`combat/mod.rs:
  737-848`): `Init/Pick/Attack/Dmg/Save/Ai/Morale/Move/StubTripped`, with
  `Move` already **per-cell** — the event stream §46 scoped the visualizer
  around.
- The **combat camera is already modeled draw-exactly** (`map_screen_top_left`
  + `focus`, `combat/mod.rs:1017-1029`; every scroll site in
  `facing.rs:48-153`) — it needs a public getter and presentation events,
  nothing more.
- Replay reconstruction (capture → roster/terrain/knobs → `CombatState`) is
  proven in `h4_replay.rs`; the record-decode half
  (`combat/records.rs::combat_state_from_records`) is **already engine
  library code** — only the loadout/toggle/continue-battle tables and the
  capture-to-inputs assembly live in **test-only** code
  (`gbx-oracle/tests/common/mod.rs`, the guard `PINS` manifest).
- Rendering substrate: 320×200 indexed framebuffer + `Clip`/`blit_image`/
  `draw_glyph`/recolor primitives, the 8×8 symbol router, text regions
  (incl. `COMBAT_SUMMARY`), `Delay` widgets, D-UI1 `Frame` with sounds +
  serial, three thin frontends needing zero changes, and the egui inspector's
  live engine pane as a debug host.
- Formats: `image.rs` (4bpp multi-item blocks — 8X8D/BIGPIC/HEAD/BODY),
  `anim.rs` (PIC/SPRIT/FINAL), `monster.rs`, `items.rs`, `dax.rs`.

**Gaps M6 must fill:**

| Gap | Where |
|---|---|
| No public camera accessor | `combat/mod.rs:1017-1029` |
| `Combatant` has no name (harnesses decode from the record) | `records.rs` path |
| `DrawFrame_Combat` is a named stub seam | `renderer-ui-shell.md:108-111` |
| COMSPR/CPIC/CHEAD/CBODY/DUNGCOM/WILDCOM/RANDCOM: not loaded, not decoded, formats unverified (likely `image.rs`-shaped — OPEN §6) | `boot.rs:9-11` |
| No 24×24 atlas store (`dax24x24Set` analog) | new |
| Capture→state reconstruction + knob schedules are test-only | `gbx-oracle/tests/common` |
| Live fights use `provisional_combat_map` + `DEFAULT_PARTY_WEAPON_DIE` 1d8 | `shell.rs:225`, `combat/mod.rs:2173-2200` |
| Presentation events beyond the current nine variants | §D-CV2 |
| No suspension points for player turns / Continue-Battle | §D-CV5 |

---

## 3. Decisions

### D-CV1 — Placement: an engine-side CombatScene with two hosts

The visualizer is a **`CombatScene` component inside `gbx-engine`** (new
`combat/scene/` module family): it owns presentation state only (presented
board, event timeline, message/panel state, loaded art handles) and draws
into the engine framebuffer through the existing primitives. It is driven by
whoever owns the fight:

1. **The Shell** (live fights, M6b/M6c): `Request::Combat` with monsters
   loaded stops calling `run_encounter` synchronously (`shell.rs:472-496`)
   and instead parks the VM and enters the combat scene. **The parking is
   interaction-level (the `VmPhase::Gate` shape), not a new top-level
   `Shell` variant** — the request surfaces inside a `VectorRun` that can
   live under Boot, Step, Look, or a chain round, and the suspended
   flow/stage cursor must survive the fight to resume it (review finding).
   Each `Engine::tick` advances the scene; when the fight ends,
   `Reply::Combat` resumes the VM with the same outcome enum as today. Two
   sequencing obligations (both review findings, owed to the slice-6
   state chart): the transcript line and `party_killed` are set only when
   the scene's **exit stage completes** — `Shell::tick` unconditionally
   replaces the shell with `GameOver` at top-of-tick when `party_killed` is
   set (`shell.rs:1146-1150`), so setting it at outcome-known time would
   annihilate the fight's final beats mid-playback.
2. **A reel host** (capture replay, M6a): `.gbxtrace` parsing stays in
   `gbx-oracle` (the dependency direction is a settled door: oracle → engine
   only), but the reel itself is **engine-side** — an `Engine` watch-mode
   constructor (`Engine::new_reel(data, ReelInput)` or equivalent) that owns
   `CombatState` + `CombatScene` + the framebuffer and exports the same
   `tick(&[InputEvent]) -> Frame` surface as normal play. It needs real
   `GameData` for art anyway, and D-UI6's thin-frontend contract survives
   intact: the desktop/CLI `--watch` flag only parses the capture via
   `gbx-oracle`, builds the engine-side `ReelInput` (roster records,
   terrain, knob + icon sidecar data), and presents Frames as always. (v1
   had the frontend pumping `CombatState` directly — rejected in review: a
   frontend owning a framebuffer, atlas, and timeline is no longer a thin
   presenter.) **The reel is h4_replay with pixels**: it attaches the same
   draw tap and asserts capture equality live while rendering.

The **headless path stays untouched**: `run_combat`/`run_combat_observed`,
h4_replay, h4_turndiff, and the frontier guard never construct a scene.

Alternatives considered:
- *A separate combat frontend/window* — rejected: D5 thin-frontend rule, one
  framebuffer, and the manual UI must interleave with the walk loop anyway.
- *Build it in the egui inspector only* — rejected as the product path (fine
  as a debug pane): the inspector is dev tooling, not the game screen, and
  M6c's manual UI must ship in the real shell.
- *Post-hoc viewer over recorded event logs* (render without running the
  engine) — rejected: captures don't contain presentation events; replaying
  a capture already *is* running the proven engine, and M6b/M6c need live
  interleave. A recorded-events format would be a second contract to keep
  honest for zero consumers.

### D-CV2 — The render feed: entry snapshot + buffered per-step event playback

`CombatState::step` stays **turn-granular** (the proven control flow is not
restructured for presentation). The scene's input contract is:

- an **entry snapshot** (roster: ids, names, teams, icon assignments, sizes,
  positions, hp; the map; the camera as read after the first `step()` — the
  camera initializes lazily inside `combat_setup` on that first call,
  `mod.rs:1207-1253`, so the getter is read at that boundary), and
- the **`step()`'s `ActionEvent` batch** — per *step*, not per turn (review
  finding): `RoundStarted` steps carry the `Init` batch, `Turn` steps the
  turn's events, and `RoundEnded` steps the round-end beats (bleed
  deaths, the Continue-Battle prompt) — captured by an attached sink during
  the one `step()` call, then **played back over subsequent ticks** as an
  animation timeline (walk steps from `Move`, swings from `Attack`/`Dmg`,
  missiles, messages, `GameDelay` beats — §1.4's table).

The scene maintains a **presented board** (positions/poses/hp/status as of
the playback cursor) that starts from the entry snapshot and is advanced by
events; it must **reconcile exactly** with `CombatState`'s true roster at
each step boundary (debug-asserted, scoped to the event-carried fields:
position, hp, health status, in-combat). Mid-playback state reads of
`CombatState` are forbidden in the scene — `step()` has already run the
whole turn, so live reads would show end-of-turn state (the §49
"end-of-STEP state" trap, now a design rule). **Boundary reads are legal
and sufficient for panel text**: the right-panel summary draws at turn
start (§1.1), a step boundary, so name/hp/AC/readied-weapon/status come
from a boundary read — no `Readied` event is needed (the original's
ready/unready is message-silent), and the reconciliation assert deliberately
excludes panel-only fields like `readied_weapon` (review finding).

**Lockstep invariant (review finding, load-bearing for live fidelity):** the
host **presents step N's playback to completion before calling `step()` for
N+1**, and queued live input (SPACE quick-fight revoke, '2') is applied to
engine flags at the step head — mirroring the original's `sub_36269` head
polls. Draws depend on those flags, and there is no rollback: a host that
pipelines "compute next turn while presenting this one" silently breaks live
fidelity in a way no replay guard can catch.

**Vocabulary policy:** the `.gbxtrace` `action` profile is **FROZEN** at its
current translated set — the equality surface never grows for presentation's
sake. Every new variant below is **engine-local presentation vocabulary**,
dropped by the oracle collector exactly as `StubTripped` is today. Additions
(payloads finalized at implementation; emitting sites are the original's own
call sites, already modeled):

| New event | Drives | Original site |
|---|---|---|
| `Camera { top_left }` | viewport scroll during playback | every modeled scroll site in `facing.rs` (§1.2) |
| `Removed { id, reason: Killed\|Downed(dying\|dead)\|Fled\|Surrendered }` | death flash, body tile, vanish, message choice | `CombatantKilled`/`RemoveFromCombat`/`KillPlayer` |
| `Bled { id, died }` | round-end bleed beat + "is killed" on bleed-out | the `battle_round_checks` bleed tick (`mod.rs:1437-1443` — mutates with **zero emissions** today; review finding) |
| `ContinueBattlePrompt { answered_yes }` | the round-end prompt is *displayed* even in replay (schedule-answered); live mode = the D-CV5 suspension | `mod.rs:1456-1462` / `ovr009.cs:404-410` |
| `Missile { attacker, target, weapon_class }` | flight animation row (COMSPR slot, frames, step delay) | `DrawRangedAttack` (`ovr014.cs:1590-1671`) |
| `BeginsCasting { caster }` / `Cast { caster, spell_id }` + one `SpellTarget { target }` per pick (keeps `ActionEvent: Copy` — no list payloads) | messages, projectile + per-target burst | `ovr014.cs:1416`, `ovr023` cast path |
| `Healed { by, target, amount, kind: cure\|bandage }` | heal/bandage messages | cure/bandage sites |
| `SlayHelpless { attacker, target }` | the draw-free held-slay beat | `sub_3F4EB` head (§49) |
| `Sound { id }` | `Frame.sounds` passthrough | §1.4 sound sites |

Vocabulary mechanics (review findings): the oracle collector matches
`ActionEvent` **exhaustively** with an explicit `StubTripped => return` arm
(`gbx-oracle/src/sink.rs:121-217`) — every new variant gets its own explicit
drop arm there, and a `_ =>` catch-all is **forbidden** (it would silently
eat a future trace-worthy event). Emission points are made
presentation-true where today's order isn't: `Move` currently emits *after*
both of `sub_3e748`'s scroll sites (`attack.rs:765-780`), which would play
back as scroll-then-move; the emit moves to just after the position write —
draw-neutral (no draws intervene), guard-refereed. The gated-off
round-end `bandage(false)` "Your Teammate is Dying" display scan
(`mod.rs:1414-1417`) is a presentation-only beat that lands with its own
event when modeled.

`Pick` (turn start), `Ai` (mode/guard), `Morale` (flee messages), `Attack`/
`Dmg`/`Save` and per-cell `Move` are already sufficient for their beats.
Where a message is fully determined by an existing event + presented-state
(e.g. "Guarding" from `Ai{target_id: -1}`), no new event is added; message
*text* is composed in the scene from event data + roster names — the combat
core stays presentation-free.

**The hard invariant (the whole design hangs on it):** presentation never
touches the PRNG and never perturbs the draw stream. Sinks are observation-
only by construction (`mod.rs:1178-1183`); the scene adds no draw sites.
Proof obligations: (a) the frontier guard stays 15/15 untouched all through
M6; (b) a new test drives the same synthetic fight headless and through the
scene and asserts **identical `RngDraw` streams**; (c) the M6a reel asserts
capture draw-equality *while rendering* (§D-CV8).

Alternative considered — *sub-turn stepping* (make `step()` yield per
move/swing so the scene reads live state): rejected. It restructures the
capture-proven core's control flow for presentation's benefit, multiplies
suspension states, and buys nothing the event buffer doesn't — the original
itself renders at exactly these event sites.

### D-CV3 — Time: original beats through the D-UI1 tick clock

All §1.4 quantities convert via the D-UI1 rule (`ms → max(1, round(ms·60/1000))`
ticks) against `game_speed_var` (default 4, live-settable 0–9 via the Speed
menu in M6c; the reel host may also set it). Delays are **not**
keyboard-interruptible (coab behavior; original-skip question docketed §6).
Per-character text pacing is disabled inside combat (§1.4) — combat text
prints whole. The scene consumes whole ticks; it never sleeps, never reads a
clock (D9), and a reel host may run it at N× by ticking faster — determinism
is per-tick, so speed never changes frames, only wall time.

### D-CV4 — Faithful-first rendering; synthetic-fixture goldens

The scene draws the original screen (§1.1–§1.3): palette 0↔8 swap for the
fight's duration, `DrawFrame_Combat` borders from the real 8×8 set, the 7×7
viewport of 24×24 tiles, icons with Normal/Attack poses + mirror facings +
party recolor, the grey focus box, the right-panel summary + messages, the
prompt line. New format work, all `gbx-formats`-side and pure:

- decode COMSPR / CPIC / CHEAD / CBODY (expected `image.rs`-shaped 4bpp
  blocks with mask-0 transparency — verify, OPEN §6) and DUNGCOM / WILDCOM /
  RANDCOM tile blocks;
- a 24×24 **atlas store** in the engine (the `dax24x24Set` analog) beside the
  existing 8×8 symbol router; icon merge (head+body) + the nibble recolor
  rule (`ovr017.cs:86-122`);
- boot/entry loading: the 13 COMSPR slots at boot (`seg001.cs:312-317`
  discharges the `boot.rs:9-11` stub), CPIC per LOADMONSTER (the operand we
  already record), CHEAD/CBODY per party member, ground tiles at
  `BattleSetup`.

Pixel goldens follow the house pattern: **synthetic hand-authored fixture
art** in CI (`hash_goldens`/`walk_goldens` precedent, D10 — no real art ever
committed), real-art rendering exercised by `GBX_DATA_DIR`-gated loud-skip
demos that dump PPMs outside the repo. Fidelity of real-art rendering is
Bryan's eyeball + DOSBox screenshot comparison at the exit gate (the M2
procedure, `dosbox-capture.md`).

QoL is deferred whole: no zoom, no free camera, no combat log pane, no
health bars (M8 companion-layer territory). The one knob that ships is the
original's own Speed menu.

### D-CV5 — Interactivity: two suspension points in the tick core

M6c makes fights playable. The core gains **suspensions, mirroring the VM's
Request/resume shape (D-VM3)** — not callbacks, not blocking:

1. **`CombatStep::AwaitPlayerTurn { combatant_id }`** — returned instead of
   auto-running a turn when the picked combatant is a party PC whose
   quick-fight flag is off (the original's per-player `quick_fight` at record
   +0x198, already noted in oracle-rig D-OR5a). The driver then issues
   **semantic turn commands** — `TurnCmd::{MoveStep(dir), AttackAdjacent
   (dir), Aim(target)/AimCursor(cell), UseRanged, Guard, DelayTurn, Bandage,
   Quit, SetSpeed(n), EngageQuickFight, EndTurn, Flee, …}` — which execute
   through the SAME proven primitives the AI uses (`sub_3E748` movement,
   `attack_target`, `TryGuarding`, the §28 flee ladder). Menu presentation
   (which words show, key handling, the aim cursor) lives in the scene;
   command legality (moves left, range, ammo) lives in the core, transcribed
   from `ovr009.cs:416-588`/`ovr014.cs:1752-2060`.
2. **`CombatStep::AwaitContinueBattle`** — the round-end "Continue Battle:"
   prompt becomes a real suspension; the driver answers y/n. The suspension
   is **conditional on an interactive-driver mode flag**: harnesses and the
   reel keep setting `continue_battle_yes` and never suspend (an
   unconditional suspension would hang `run_combat`, `mod.rs:1456-1462`;
   review finding) — schedule-driven replays stay byte-identical.

Invariants: with every PC's quick-fight flag on and schedules supplied, the
step sequence and draw stream are **bit-identical to today's** — the guard
15/15 referees this (the §36.5 canary discipline: suspensions land dark,
guard-exact, before any UI drives them). Manual turns draw through the same
attack/move/spell primitives, so a **capture with manual turns closes the
manual path the same way QuickFight captures did** (precedent: sewer-fight-4
take 1's manual first turn was plain d20/d2 draws) — that capture campaign is
part of M6c (§4).

SPACE-revokes-quickfight and '2' (auto-magic) during AI turns become real
keypresses routed by the shell host; in replay they remain schedule knobs
(the §38 machinery is unchanged — it was always the *recording* of a
keypress).

### D-CV6 — The live-fight fidelity boundary (M6b's real work)

Rendered replays (M6a) are exact by construction — captures carry terrain and
the proven engine generates the events. **Live** fights from the walk loop
currently ride two placeholders, and M6b's substance is retiring them:

1. **`provisional_combat_map` → faithful `SetupDungeonFloor`** (§1.2: the
   oblique GEO projection + wall/door tile runs + the 50%/90% furniture dice
   — **draw-bearing**, so it lands with tests against the real tile tables
   and gets pinned by the first live-fight capture, §D-CV8). Wilderness
   floors stay deferred (circle-back, with wilderness tiles).
2. **`DEFAULT_PARTY_WEAPON_DIE` → real party combat stats**: derive the
   party's combat records from actual M3 party state (equipment kits, readied
   weapons, ammo-readied — the §49 gate lives here) through the same decode
   path the capture rosters use. This is the piece that makes a live bar
   brawl *be* the capture-proven fight rather than resemble it.

M6b's honesty rule: a live fight is **exact where capture-proven, cited
elsewhere** — the first live-fight capture (stage it like any campaign)
converts the remainder. Monster records/CPIC/placement already flow from the
real LOADMONSTER path.

### D-CV7 — Saves and serialization (review finding — the type system forces it)

Saving mid-combat is **faithfully impossible**: the combat menu and its Done
submenu build no Save word (`ovr009.cs:313-360,616-631`) — saves are
camp-only, and camp is unreachable from combat. But reachability doesn't
discharge the type-level obligation: `Shell` derives serde
(`shell.rs:1080`) and D-UI2 promises every parked state is serializable *by
construction*, so whatever type parks the fight must compile under serde.
Today nothing in `combat/` derives it, `CombatState` holds
`sink: Option<Box<dyn ActionSink>>` (`mod.rs:1035`) and
`item_data: Option<ItemDataTable>` where `gbx-formats` has no serde
dependency. Decision: serde derives land across `combat/` types with
`#[serde(skip)]` on the sink (it is `Option` + default-inert by design) and
`skip` on transient scene/timeline state; `item_data` gets either a serde
feature in `gbx-formats` or an engine-side serializable mirror — chosen at
implementation, budgeted in slice 6. Snapshotting a *suspended* fight thus
works by construction (and is what the inspector's debug pane wants anyway),
while the player-facing save UI stays faithfully absent in combat.

### D-CV8 — Testing

- **Draw-parity invariant** (CI): one synthetic fight run headless and
  through `CombatScene` (scene ticked to completion) → identical `RngDraw`
  streams and identical final rosters. The single most load-bearing test in
  M6.
- **Scene timeline units** (CI): event batch in → deterministic tick
  schedule out (beats per §1.4's table); presented-board reconciliation
  asserts at turn boundaries.
- **Pixel goldens** (CI): synthetic fixture art, `(scenario, tick)`-keyed
  hashes — the walk_goldens pattern for a scripted fight.
- **Reel smokes** (local tier, `GBX_TRACES_DIR`): every closed capture plays
  end-to-end through the reel with **live draw-equality against the capture**
  (the h4_replay assert, now with pixels) and completion; no pixel pins over
  real art (art-rendering fidelity is eyeball + DOSBox comparison, D-CV4).
- **The frontier guard stays the referee**: 15/15 exact at every M6 commit,
  same rule as M5.

---

## 4. Milestones and the exit gate (proposal)

- **M6a — the reel**: formats (COMSPR/CPIC/CHEAD/CBODY/tiles) + atlas +
  scene (layout, tiles, icons, playback timeline, panel/messages, camera) +
  the reel host + reconstruction library-ification (the knob/loadout tables
  and capture-to-`ReelInput` assembly move from test-only `common/mod.rs`
  into `gbx-oracle` proper; the record-decode half is already engine lib).
  Knob schedules AND **icon assignments** become a **versioned per-capture
  sidecar** mirroring the PINS rows — a review finding: captures carry no
  monster CPIC ids (`format.rs:430-437`; LOADMONSTER's icon operand is
  recorded only on the live path), so the 15 existing captures get
  hand-pinned CPIC ids exactly as loadouts are pinned today, party icons
  derive from the records, and a hook TODO adds icon ids to future
  `combat_entry` emissions. (The sidecar is input data with a real consumer
  — not the recorded-events second contract D-CV1 rejects.) **Done =**
  `restrike-desktop --watch <capture>` plays any closed capture; draw
  equality asserted live; D-CV8's CI tests green.
- **M6b — live QuickFight**: the Shell Combat flow stage; faithful dungeon
  floor-gen + real party combat stats (D-CV6); "A battle begins..." through
  outcome-and-restore. **Done =** boot → walk to the Tilverton bar → the
  brawl happens **on screen** with QuickFight, VM resumes correctly after.
- **M6c — manual combat**: the D-CV5 suspensions + TurnCmd core; combat
  menu / movement keys / Aim + cursor / Done submenu / Speed / Flee /
  Continue-Battle prompt / SPACE + '2' as real keys; View can open the M3
  character sheet (palette toggle). **Done =** a full manual fight won on
  screen; a staged **manual-turn capture** closes draw-exactly through the
  same replay harness.

**Exit gate (proposed — Bryan ratifies):**
1. All 15 closed captures play in the reel with live draw-equality (M6a).
2. Boot→bar-brawl watched live via QuickFight (M6b), and won manually (M6c).
3. One new manual-turn capture staged and CLOSED in the guard.
4. Guard 15/15 + all six CI gates green throughout; draw-parity invariant in
   CI.
5. Bryan's playtest (D13) — the fight *looks* like the original against a
   DOSBox side-by-side.

## 5. Build order and session guidance

Slices sized for the established loop (Fable specs already in this doc; Opus
implements; Fable audits; guard referees every commit):

1. Formats + atlas + boot/entry art loading (pure `gbx-formats` + loaders;
   verify COMSPR/CPIC shapes against real data first — the §6 OPEN).
2. Camera getter + presentation events (engine; draw-neutral by
   construction, guard-proven).
3. `CombatScene` core: layout + tiles + icons + presented board (pixel
   goldens on fixtures).
4. Playback timeline: beats, messages, missiles, death flash, sounds
   (timeline units + the draw-parity invariant test).
5. Reel host + reconstruction library-ification (M6a closes).
6. Shell Combat flow + D-CV6 fidelity work + the D-CV7 serde budget and the
   D-CV1 sequencing obligations (parking level, `party_killed`/transcript at
   exit-stage completion) (M6b closes).
7. Suspensions + TurnCmd + menus/aim (dark-landing first, guard-exact, then
   UI; M6c closes with the manual capture campaign).

Model/effort per PLAN §9: slices 1/3/4/5 are Opus 4.8 implementation
sessions off this door; slice 2 is small enough to ride with 3; slices 6/7
get a short Fable spec-refresh each (the shell-flow state chart and the
TurnCmd legality table deserve their own §-sections in this doc before
implementation, same as §26/§28/§34/§36 did in M5). Staging the manual-turn
capture is Bryan+Fable, one launch (the §25 runbook applies unchanged).

## 6. Open questions → docket seeds

1. COMSPR/CPIC/CHEAD/CBODY block shapes: assumed `image.rs` 4bpp multi-item;
   verify against real data before slice 3 (a `dump-image` session settles
   it in minutes).
2. `DrawIsoTile`'s `tileIndex > 0x7f` overlay path (`dword_1C8FC`) is
   stubbed in coab — original behavior unknown; loud-fail if reached.
3. RANDCOM atlas slot 0x25 (background 0x1D) is unnamed in coab — identify
   from pixels.
4. Were original delays keypress-skippable (coab: no)? A 2-minute DOSBox
   check at the next staging session.
5. PC-speaker timing for the sound ids (M8 concern; we only emit events).
6. Multi-cell icon pixel dims — read from real CPIC headers at slice-1 time.
7. The mid-combat View sheet's palette-swap toggle interaction with M3's
   character screens — pin at M6c implementation.
8. `boot.rs`'s stub comment cites the COMSPR loads as `seg001.cs:308-311,321`;
   the loop is actually at `:312-317` (audit finding) — fix the comment when
   slice 1 discharges the stub.

## 7. Non-goals

- No XP/treasure screens (circle-back; the fight still ends cleanly).
- No wilderness floor generation (deferred with wilderness tiles).
- No spell-menu UI beyond what M6c's Cast needs from the already-modeled
  memorized lists; exotic spell visuals land with their circle-back rows
  (unknown spell id still trips `spell-entry`).
- No QoL rendering (zoom/log/health bars) — M8.
- No sound synthesis — M8; M6 emits `SoundEvent`s only.
- The egui inspector combat pane is welcome as a byproduct, never the
  deliverable.
