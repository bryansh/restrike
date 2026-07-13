//! M2 step 4 deliverable 5: engine-level H2 conformance — real
//! `EclBuilder`-authored fixture blocks driven tick-by-tick through
//! `Engine`/`Shell`, proving PRINT reaches the framebuffer and paginates,
//! a real `Request::HorizontalMenu` parks, resolves via input, and writes
//! memory, `DELAY` counts real ticks, and an unimplemented opcode invokes
//! the M2 halt policy rather than crashing. Everything here is D10
//! synthetic (hand-authored ECL bytecode), never derived from real game
//! data. Hash goldens follow `hash_goldens.rs`'s pinning pattern
//! (`RESTRIKE_REGEN_GOLDENS=1` to regenerate).

#![cfg(test)]

use crate::engine::{Engine, GAME_AREA, INITIAL_ECL_BLOCK};
use crate::input::InputEvent;
use crate::shell::Shell;
use crate::test_support::{ecl_game_data, exit_only_block, simple_block};
use gbx_formats::font::{self, Font};
use gbx_formats::geo::{GeoBlock, GEO_BLOCK_SIZE};
use gbx_formats::image::{DecodedItem, ImageBlock};
use gbx_vm::test_support::EclBuilder;

fn synthetic_set4() -> ImageBlock {
    ImageBlock {
        height: 8,
        width_cols: 1,
        x_pos: 0,
        y_pos: 0,
        field_9: [0; 8],
        items: (0..40)
            .map(|i| DecodedItem {
                pixels: vec![(i % 16) as u8; 64],
            })
            .collect(),
    }
}

fn synthetic_font() -> Font {
    let mut data = Vec::with_capacity(font::GLYPH_COUNT * font::GLYPH_BYTES);
    for j in 0..font::GLYPH_COUNT {
        data.extend_from_slice(&[j as u8; font::GLYPH_BYTES]);
    }
    font::decode(&data)
}

fn open_geo() -> GeoBlock {
    GeoBlock::parse(&vec![0u8; GEO_BLOCK_SIZE]).unwrap()
}

/// Builds an `Engine` whose resident block (`ECL{GAME_AREA}.DAX` block
/// [`INITIAL_ECL_BLOCK`]) is `entry_block`, plus any `extra_blocks` (e.g. a
/// NEWECL target).
fn engine_with(entry_block: EclBuilder, extra_blocks: Vec<(u8, EclBuilder)>) -> Engine {
    let mut blocks = vec![(INITIAL_ECL_BLOCK, entry_block)];
    blocks.extend(extra_blocks);
    let mut sets = crate::symbols::SymbolSets::new();
    sets.load(4, synthetic_set4());
    let data = ecl_game_data(GAME_AREA, blocks);
    Engine::new_fixture(synthetic_font(), sets, open_geo(), data, 1)
}

fn write_ppm(name: &str, frame: &crate::engine::Frame) {
    let path = std::env::temp_dir().join(format!("restrike-h2-golden-{name}.ppm"));
    let mut out = format!(
        "P6\n{} {}\n255\n",
        crate::framebuffer::WIDTH,
        crate::framebuffer::HEIGHT
    )
    .into_bytes();
    for y in 0..crate::framebuffer::HEIGHT {
        for x in 0..crate::framebuffer::WIDTH {
            let idx = frame.pixels[y * crate::framebuffer::WIDTH + x];
            out.extend_from_slice(&frame.palette[idx as usize]);
        }
    }
    if std::fs::write(&path, &out).is_ok() {
        eprintln!("H2 golden '{name}': dumped {}", path.display());
    }
}

fn check_golden(name: &str, frame: &crate::engine::Frame, expected_hex: &str) {
    let actual = frame.hash_hex();
    let regen = std::env::var_os("RESTRIKE_REGEN_GOLDENS").is_some();
    if regen {
        eprintln!("H2 golden '{name}': {actual}");
        write_ppm(name, frame);
        return;
    }
    if actual != expected_hex {
        write_ppm(name, frame);
    }
    assert_eq!(
        actual, expected_hex,
        "H2 golden '{name}' mismatched — see dumped .ppm (or rerun with RESTRIKE_REGEN_GOLDENS=1)"
    );
}

fn tick_n(e: &mut Engine, n: u32) {
    for _ in 0..n {
        e.tick(&[]);
    }
}

/// PRINT (0x11) flowing all the way into the framebuffer, then pagination
/// (a text long enough to overflow `NormalBottom`'s 6-row window) gating
/// the run — proving the D-VM3 "queue drains before any Gate opens"
/// obligation for the *real* interpreter's own `Effect::Print`, not just
/// `text.rs`'s unit-level wrap tests. Proven by comparing the settled
/// frame against a no-op (EXIT-only) boot's settled frame rather than
/// asserting exact pixel values, which would depend on font-glyph bit
/// layout this test has no business knowing.
#[test]
fn print_reaches_the_framebuffer_and_paginates_on_long_text() {
    // NORMAL_BOTTOM is 6 rows x 37 cols (~222 chars) before it must
    // paginate; two repetitions of this phrase (264 chars) overflow that
    // while staying under the inline packed-string operand's 255-packed-byte
    // ceiling (`EclBuilder::inline_str_packed`'s length-prefix byte).
    let long_text = "ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT NINE TEN ELEVEN TWELVE \
                      THIRTEEN FOURTEEN FIFTEEN SIXTEEN SEVENTEEN EIGHTEEN NINETEEN TWENTY "
        .repeat(2);
    let block = simple_block(|b| {
        b.op(0x11).inline_str(long_text.as_bytes()); // PRINT
        b.op(0x00); // EXIT
    });
    let mut e = engine_with(block, vec![]);

    // Drive well past pagination — the text is long enough to paginate at
    // least once inside the 6-row/38-col NormalBottom window.
    let mut saw_gate = false;
    for _ in 0..2000 {
        e.tick(&[]);
        if matches!(&e.shell, Shell::Boot(_)) && e.shell.gate_open() {
            saw_gate = true;
            e.tick(&[InputEvent::Enter]); // release the pagination gate
        }
        if matches!(e.shell, Shell::WorldMenu { .. }) {
            break;
        }
    }
    assert!(
        saw_gate,
        "a long PRINT must trigger at least one pagination gate"
    );
    assert!(matches!(e.shell, Shell::WorldMenu { .. }));
    let printed_hash = e.tick(&[]).hash_hex();

    let mut baseline = engine_with(exit_only_block(), vec![]);
    tick_n(&mut baseline, 5);
    let baseline_hash = baseline.tick(&[]).hash_hex();

    assert_ne!(
        printed_hash, baseline_hash,
        "PRINT's real text must have drawn into the framebuffer relative to a no-op boot"
    );
}

/// The pinned `(trace, tick_index)` golden for the PRINT/pagination
/// scenario above — a short, single-page PRINT so the checkpoint is a
/// stable, non-paginating frame.
#[test]
fn golden_print_frame() {
    let block = simple_block(|b| {
        b.op(0x11).inline_str(b"HELLO"); // PRINT, short enough to fit on one line
        b.op(0x00); // EXIT
    });
    let mut e = engine_with(block, vec![]);
    tick_n(&mut e, 10);
    assert!(matches!(e.shell, Shell::WorldMenu { .. }));
    let f = e.tick(&[]);
    check_golden(
        "print-hello",
        &f,
        "eead7cb28a5c78d2b87d599dbe70aa7a29e5b8c911f2bd3d590add7c0a1d7bd2",
    );
}

/// A real `Request::HorizontalMenu` parks, resolves via an ordinary input
/// event, and the interpreter's own `WriteWordThenAdvance` completion
/// writes the selection back to real `ScriptMemory` — the M2 halt-free
/// happy path for `Reply` construction (`shell.rs`'s
/// `resolve_horizontal_menu_reply`).
#[test]
fn horizontal_menu_reply_writes_memory() {
    const DEST: u16 = 0x5000; // an unclaimed Global-window cell (raw store)
    let block = simple_block(|b| {
        b.op(0x2B) // HORIZONTAL MENU
            .mem(DEST)
            .imm_byte(2)
            .inline_str(b"YES")
            .inline_str(b"NO");
        b.op(0x00); // EXIT
    });
    let mut e = engine_with(block, vec![]);

    // Reach the parked menu.
    let mut parked = false;
    for _ in 0..20 {
        e.tick(&[]);
        if matches!(&e.shell, Shell::Boot(_)) && e.shell.gate_open() {
            parked = true;
            break;
        }
    }
    assert!(parked, "the HorizontalMenu Request must park a Gate widget");

    // Select "NO".
    e.tick(&[InputEvent::Char(b'n')]);
    for _ in 0..10 {
        if matches!(e.shell, Shell::WorldMenu { .. }) {
            break;
        }
        e.tick(&[]);
    }
    assert!(matches!(e.shell, Shell::WorldMenu { .. }));
    assert_eq!(
        e.vm_memory().raw_word(DEST),
        Some(1),
        "the real interpreter must have written the selected index (1 = NO) back to DEST"
    );
}

/// `DELAY` (0x3A) parks a `Request::Delay` -> `Widget::Delay`, which counts
/// down real ticks (the placeholder 24-tick duration, `shell.rs`'s
/// `widget_for_request`) rather than resolving instantly.
#[test]
fn delay_counts_real_ticks() {
    let block = simple_block(|b| {
        b.op(0x3A); // DELAY
        b.op(0x00); // EXIT
    });
    let mut e = engine_with(block, vec![]);

    let mut ticks_gated = 0u32;
    for _ in 0..40 {
        e.tick(&[]);
        if matches!(&e.shell, Shell::Boot(_)) && e.shell.gate_open() {
            ticks_gated += 1;
        } else if ticks_gated > 0 {
            break;
        }
    }
    assert!(
        ticks_gated >= 20,
        "DELAY must gate for multiple real ticks, not resolve instantly (gated for {ticks_gated})"
    );
    for _ in 0..10 {
        if matches!(e.shell, Shell::WorldMenu { .. }) {
            break;
        }
        e.tick(&[]);
    }
    assert!(matches!(e.shell, Shell::WorldMenu { .. }));
}

/// An opcode the interpreter has no handler for (`0x05` SUBTRACT — outside
/// the census top-25 + ride-alongs this session's `EclMachine` implements,
/// per the M1 handoff note) invokes the M2 halt policy: logged, the run
/// treated as `Done(Ended)`, never a panic or a stuck flow.
#[test]
fn unimplemented_opcode_invokes_the_halt_policy_not_a_panic() {
    let block = simple_block(|b| {
        b.op(0x05); // SUBTRACT — dialect-known, not implemented by this interpreter
    });
    let mut e = engine_with(block, vec![]);
    for _ in 0..10 {
        e.tick(&[]);
        if matches!(e.shell, Shell::WorldMenu { .. }) {
            break;
        }
    }
    assert!(
        matches!(e.shell, Shell::WorldMenu { .. }),
        "a halted vector run must still unwind to WorldMenu, not stall"
    );
    assert_eq!(
        e.vm_memory().halts.len(),
        1,
        "the halt must be logged exactly once"
    );
    assert_eq!(e.vm_memory().halts[0].opcode, 0x05);
}

/// Serde round-trip of the whole `Shell` while a real `VectorRun` is parked
/// on a genuine `gbx_vm::Request` (not a synthetic one) — the D-UI7
/// "serialize/restore every Shell/Widget variant mid-flight" requirement,
/// now exercised with a real machine-sourced Request.
#[test]
fn shell_round_trips_while_parked_on_a_real_request() {
    let block = simple_block(|b| {
        b.op(0x24); // COMBAT — a Request the interpreter really emits
        b.op(0x00); // EXIT
    });
    let mut e = engine_with(block, vec![]);
    let mut parked = false;
    for _ in 0..10 {
        e.tick(&[]);
        if e.shell.gate_open() {
            parked = true;
            break;
        }
    }
    assert!(parked);

    let json = serde_json::to_string(&e.shell).expect("Shell must serialize while gated");
    let restored: Shell = serde_json::from_str(&json).expect("Shell must deserialize while gated");
    assert!(matches!(restored, Shell::Boot(_)));
}
