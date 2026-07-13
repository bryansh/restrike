// The generator for fixtures/tilverton-circuit.jsonl (M2 step 8, D-UI7's
// exit-gate circuit) — not part of the shipped CLI surface, but kept
// (rather than discarded after one run) as the trace's authoritative,
// re-runnable definition: walks the fixed circuit adaptively (feeding input
// only once the screen goes quiet, same discipline as gbx-engine's
// demo.rs walk demos), recording every tick's input as a JSONL walk trace
// plus a checkpoint after each square. Regenerate with:
//   cargo run -p restrike-cli --example record_circuit -- \
//     $GBX_DATA_DIR fixtures/tilverton-circuit.jsonl
// Re-running is only needed if the circuit itself changes (new squares,
// different route) or an engine change shifts the deterministic tick
// counts (RNG-consuming opcodes gap-filled, pacing changes, etc.) — the
// committed trace file is what CI and `restrike walk` actually consume.
use gbx_engine::engine::Engine;
use gbx_engine::input::{ExtKey, InputEvent};
use gbx_engine::movement::Facing;
use gbx_engine::shell::Shell;
use gbx_engine::vmhost::TranscriptEntry;
use gbx_formats::game_data::load_dir;
use std::io::Write;

struct Recorder {
    engine: Engine,
    tick: u64,
    lines: Vec<String>,
}

impl Recorder {
    fn log_transcript(&mut self) {
        for entry in self.engine.take_transcript() {
            match entry {
                TranscriptEntry::Print { text, clear_first } => {
                    eprintln!(
                        "  [tick {}] {}: {}",
                        self.tick,
                        if clear_first { "PRINTCLEAR" } else { "PRINT" },
                        text
                    );
                }
                TranscriptEntry::Request(label) => {
                    eprintln!("  [tick {}] REQUEST: {}", self.tick, label);
                }
            }
        }
    }

    /// Feeds `input` once, then ticks empty until the screen goes quiet
    /// (bounded), feeding Enter through any pagination/press-any-key gate
    /// along the way, and asserting `done` holds at the end.
    fn drive(&mut self, input: &[InputEvent], max_ticks: u32, done: impl Fn(&Engine) -> bool) {
        self.tick += 1;
        self.engine.tick(input);
        for ev in input {
            self.lines
                .push(serde_json::json!({"tick": self.tick, "event": {"input": ev}}).to_string());
        }
        self.log_transcript();

        let mut last_serial = u64::MAX;
        let mut quiet = 0u32;
        for _ in 0..max_ticks {
            if done(&self.engine) {
                return;
            }
            let feed: &[InputEvent] = if quiet >= 2 {
                quiet = 0;
                &[InputEvent::Enter]
            } else {
                &[]
            };
            self.tick += 1;
            let serial = self.engine.tick(feed).serial;
            for ev in feed {
                self.lines.push(
                    serde_json::json!({"tick": self.tick, "event": {"input": ev}}).to_string(),
                );
            }
            self.log_transcript();
            if serial == last_serial {
                quiet += 1;
            } else {
                quiet = 0;
                last_serial = serial;
            }
        }
        assert!(
            done(&self.engine),
            "step did not converge within {max_ticks} ticks (at tick {})",
            self.tick
        );
    }

    fn checkpoint(&mut self, label: &str) {
        self.lines
            .push(serde_json::json!({"tick": self.tick, "event": "checkpoint"}).to_string());
        eprintln!(
            "-- checkpoint '{label}' at tick {} pos={:?} facing={:?} --",
            self.tick,
            self.engine.state().pos,
            self.engine.state().facing
        );
    }

    fn world_menu(e: &Engine) -> bool {
        matches!(e.shell(), Shell::WorldMenu { .. })
    }

    /// Turns to face `dir` (minimal turn), then steps forward once, then
    /// drops a checkpoint labeled by the destination square.
    fn turn_and_step(&mut self, dir: Facing) {
        let cur = self.engine.state().facing;
        if cur != dir {
            let turn = if opposite(cur) == dir {
                ExtKey::Down
            } else if turn_right(cur) == dir {
                ExtKey::Right
            } else {
                ExtKey::Left
            };
            self.drive(&[InputEvent::Ext(turn)], 200, Self::world_menu);
        }
        self.drive(&[InputEvent::Ext(ExtKey::Up)], 600, Self::world_menu);
        let pos = self.engine.state().pos;
        self.checkpoint(&format!("square-{}-{}", pos.0, pos.1));
    }
}

fn opposite(f: Facing) -> Facing {
    match f {
        Facing::North => Facing::South,
        Facing::South => Facing::North,
        Facing::East => Facing::West,
        Facing::West => Facing::East,
    }
}

fn turn_right(f: Facing) -> Facing {
    match f {
        Facing::North => Facing::East,
        Facing::East => Facing::South,
        Facing::South => Facing::West,
        Facing::West => Facing::North,
    }
}

fn dir_between(a: (u8, u8), b: (u8, u8)) -> Facing {
    let (ax, ay) = (a.0 as i32, a.1 as i32);
    let (bx, by) = (b.0 as i32, b.1 as i32);
    match (bx - ax, by - ay) {
        (0, -1) => Facing::North,
        (0, 1) => Facing::South,
        (1, 0) => Facing::East,
        (-1, 0) => Facing::West,
        other => panic!("non-adjacent path step {a:?} -> {b:?} ({other:?})"),
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: record_circuit <DATA_DIR> <OUT_TRACE>");
    let out = std::env::args()
        .nth(2)
        .expect("usage: record_circuit <DATA_DIR> <OUT_TRACE>");

    let data = load_dir(std::path::Path::new(&dir)).expect("GBX_DATA_DIR must be readable");
    let engine = Engine::new(data, 1).expect("Engine::new must boot against real CotAB data");
    let mut rec = Recorder {
        engine,
        tick: 0,
        lines: Vec::new(),
    };

    rec.drive(&[], 600, Recorder::world_menu);
    assert_eq!(rec.engine.state().pos, (7, 13));
    rec.checkpoint("spawn");

    // (7,14) [event 0x14, the "nightmare man"] is deliberately NOT on this
    // circuit: its script calls Load3dMap to relocate the party to a
    // different resident area (a real cross-area transition — coab's
    // "found near the sewer outfall" text lines up), which M2's Engine
    // doesn't support (`engine.rs`'s doc comment: GEO/ECL block *selection*
    // is step 5+ scope, this session always keeps the original resident
    // block). Confirmed by running it in isolation: position lands at
    // (0,0) (an out-of-bounds corner of the still-resident Tilverton grid)
    // and a real gap surfaced along the way (OR/0x30, now implemented) —
    // both are docketed rather than routed through by force.

    // Leg 2: (6,13) -> (6,12)[0x05] -> (5,12) -> (5,11) -> (5,10)[0x07] — the
    // tavern. Real content here can script its own party reposition (D10:
    // not quoted, but the gist is a brawl that ends with the party ejected)
    // once the auto-driver's "feed Enter on quiet" policy picks the
    // brawl/investigate branches (both menus' highlighted-first-word
    // default, per this session's fixed input policy) — deterministic
    // given (seed, policy), but NOT always the square this fixed direction
    // list would predict, so the walk stops driving by planned waypoint
    // here and instead asserts the real landing square.
    let leg2: &[(u8, u8)] = &[(7, 13), (6, 13), (6, 12), (5, 12), (5, 11), (5, 10)];
    for w in leg2.windows(2) {
        rec.turn_and_step(dir_between(w[0], w[1]));
    }
    let landing = rec.engine.state().pos;

    // Leg 3: return from wherever the tavern scene left the party, back to
    // spawn via the same corridor, passing (7,12)[0x10, the narrow-street
    // text] on the way if the reposition (still observed, RNG-dependent —
    // see this fn's leg-2 doc comment) put us at (5,10) itself.
    let leg3: &[(u8, u8)] = &[(5, 10), (5, 11), (5, 12), (6, 12), (6, 13), (7, 13)];
    assert_eq!(landing, leg3[0], "unexpected tavern landing square");
    for w in leg3.windows(2) {
        rec.turn_and_step(dir_between(w[0], w[1]));
    }
    assert_eq!(rec.engine.state().pos, (7, 13));
    rec.checkpoint("back-at-spawn");

    let halts = &rec.engine.vm_memory().halts;
    eprintln!("-- halts: {} --", halts.len());
    for h in halts {
        eprintln!(
            "  pc={:#06X} opcode={:#04X}: {}",
            h.pc, h.opcode, h.description
        );
    }

    let mut f = std::fs::File::create(&out).unwrap();
    for line in &rec.lines {
        writeln!(f, "{line}").unwrap();
    }
    eprintln!(
        "wrote {} trace line(s) to {out}, {} total ticks",
        rec.lines.len(),
        rec.tick
    );
}
