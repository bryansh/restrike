# Design: module restructure — splitting the oversized files (2026-07-23)

The question this doc answers: `combat.rs` has grown past the point of comfortable
navigation — what do we split, how do we split it without risking the capture pins,
and which other files earn the same treatment?

## 0. The survey (2026-07-23, workspace = 55,979 lines of Rust)

| file | total | code | tests | verdict |
|---|---|---|---|---|
| `gbx-engine/src/combat.rs` | 8,848 | 5,425 | 3,423 | **SPLIT** — 4× the next file; every active M5 slice (affects, spells) lands here |
| `gbx-engine/src/shell.rs` | 2,038 | 1,266 | 772 | WATCH — M3's party/camp/shop screens; quiet since M3; split only if Phase-3 breadth work regrows it |
| `gbx-vm/src/conformance.rs` | 1,742 | — | — | leave — one purpose (ECL conformance harness) |
| `gbx-vm/src/machine.rs` | 1,437 | — | — | leave — the VM core, stable since M1/M2 |
| `gbx-engine/src/demo.rs` | 1,357 | — | — | leave — demos, low churn |
| `gbx-engine/src/corridor.rs` | 1,257 | 794 | 463 | leave — renderer, cohesive |
| `gbx-engine/src/vmhost.rs` | 1,234 | 1,048 | 186 | leave — one seam (EngineVmHost) |
| `gbx-formats/src/save_orig.rs` | 1,127 | 840 | 287 | leave — one format family; new formats (e.g. the §39 affects decode) start as sibling modules |
| `gbx-rules/src/adnd1/flavor_impl.rs` | 1,100 | 677 | 423 | leave — one flavor impl |

Size alone is not the trigger. The split criteria, in order of weight:

1. **Multiple binary overlays / subsystems share one file** (combat.rs spans
   ovr009/010/014/024/025/033 material) — the strongest signal.
2. **Active growth**: the file is where current and next slices land.
3. **Inline tests past ~1k lines** — move to child test modules regardless of the
   rest.

Only `combat.rs` meets all three today.

## 1. Why the split is safe here (the guard is the referee)

The frontier guard pins every capture's exact outcome (six CLOSED operand-exact, two
frontiers at exact draw indices). A **move-only** refactor that leaves the draw stream
untouched is therefore *provable*, not just plausible: guard 8/8 + the full workspace
suite before and after each commit is a mechanical no-change proof. This is precisely
the situation the guard was built for — use it.

## 2. The `combat/` split map (along the binary's own seams)

The file's section comments already mirror the original's overlay structure; the
module tree makes that structure physical, which also makes citations navigable
(`ovr014:xxxx` → you know the file before you grep):

```
crates/gbx-engine/src/combat/
  mod.rs        CombatState, Combatant, Phase/TurnDriver, the round/turn loop,
                battle_round_checks, initiative     (ovr009 + ovr011 entry)
  ai.rs         the QuickFight turn body: sub_3504B order, spell selection
                (sub_3560B/ShouldCastSpellX), flee/morale ladder, guarding
                (ovr010)
  attack.rs     find_target, to-hit/AC selection, backstab, departure loop,
                sweep, ranged/ammo/items_selection (§34)     (ovr014)
  facing.rs     direction bookkeeping + the combat camera (§36)     (ovr033)
  affects.rs    the §39 substrate: storage, find/add/remove, the 24-case
                dispatch, strip tables     (ovr024)
  records.rs    combatant_from_record + entry decode
  tests/        the ~3,400 test lines, split by the same areas
                (child modules keep private-method access)
```

Boundaries follow the overlays, not abstract layering — a function moves to the file
matching its binary citation. `CombatState` stays in `mod.rs`; cross-module access
via `pub(super)`/`pub(crate)` exactly as needed, no API redesign, no visibility
broadening beyond what compiles.

## 3. Protocol

1. **Timing: at the next quiet boundary** — after the affects substrate and the
   caster peel land (or at the M5 exit gate). Never mid-slice: an in-flight
   implementer branch on combat.rs vetoes the split until it merges.
2. **One module per commit**, each a pure move (plus the minimal `use`/visibility
   edits to compile). Review question per commit: "is this a pure move?"
3. **Full gates per commit** including guard 8/8 — the no-change proof.
4. **Blame preservation**: add the move commits to `.git-blame-ignore-revs`
   (create the file with the first move commit); `git log --follow` covers the rest.
5. Estimated size: a half-day implementer slice; spec = this doc + the commit list.

## 4. The standing rule (in force immediately)

**New subsystems start as their own modules** — nothing new is appended to an
oversized file. First beneficiaries: the spell subsystem (SpellEntry table + cast
handlers — sizable and naturally separate) and the §39 `gbx-formats` affects decode
(already specced as its own module). This rule costs nothing now and caps the
problem while the split waits for its quiet window.

## 5. Non-goals

- No crate splits: `gbx-engine` remains one crate; this is a module tree, not a
  workspace reshuffle.
- No behavior or API change of any kind rides a move commit.
- No renames of types/functions during the move (rename later if wanted, as its own
  reviewable change).
