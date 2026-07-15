//! `restrike run-script [DIR] --dax <FILE> --block <ID> [--vector N]
//! [--trace] [--reply k=v ...]` — the M1 "it's alive" demo: load a real
//! CotAB ECL block into an [`EclMachine`] and run it headlessly, against a
//! CLI-only [`VmHost`] (raw-word-store `ScriptMemory` that logs every
//! access, `Effect`s printed to stdout, `Request`s answered from `--reply`
//! or a documented default policy, a deterministic fixed-seed RNG, and
//! `EngineServices` calls printed as a trace line with neutral scripted
//! results).
//!
//! This is intentionally *not* `gbx-engine`'s eventual `ScriptMemory`
//! implementation (that owns the real window map, per D-VM5) — it's a
//! throwaway diagnostic host scoped to this CLI command, matching the task
//! brief's explicit description ("a CLI host").

use std::collections::{HashMap, VecDeque};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_formats::dax::{self, DaxArchive};
use gbx_prng::Prng;
use gbx_vm::{
    decode, render_instr, BlockBytes, EclMachine, Effect, EngineServices, ItemHandle, MissingData,
    MonsterHandle, NotFound, Origin, PlayerId, Reply, Request, ScriptMemory, VmError, VmHost,
    VmRng, VmStep, VmString, COTAB, ECL_BLOCK_SIZE,
};

pub fn cmd_run_script(args: Vec<String>) -> ExitCode {
    let opts = match RunScriptArgs::parse(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("restrike: {msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let dir = match opts
        .dir
        .clone()
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
    {
        Some(dir) => dir,
        None => {
            eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let dax_path = if opts.dax.is_absolute() || dir.as_os_str().is_empty() {
        opts.dax.clone()
    } else {
        dir.join(&opts.dax)
    };

    let bytes = match fs::read(&dax_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("restrike: failed to read '{}': {err}", dax_path.display());
            return ExitCode::FAILURE;
        }
    };
    let archive = match DaxArchive::parse(&bytes) {
        Ok(archive) => archive,
        Err(err) => {
            eprintln!(
                "restrike: failed to parse DAX '{}': {err:?}",
                dax_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let raw = match archive.block_data(opts.block) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!(
                "restrike: block {} not found in '{}': {err:?}",
                opts.block,
                dax_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let payload = dax::ecl_block_payload(&raw);
    if payload.len() > ECL_BLOCK_SIZE {
        eprintln!(
            "restrike: block {} ECL payload is {} bytes, exceeding the 0x1E00-byte resident \
             block size",
            opts.block,
            payload.len()
        );
        return ExitCode::FAILURE;
    }
    let block = BlockBytes::from_bytes(payload);
    // A second, un-consumed copy purely for --trace's live disassembly
    // (`EclMachine` owns the block it's given; self-modification isn't
    // implemented this session — machine.rs's `mem_write` never touches the
    // Ecl window — so this copy never drifts from the machine's own).
    let trace_block = block.clone();

    let mut machine = match EclMachine::load_block(block, &COTAB) {
        Ok(m) => m,
        Err(err) => match err {},
    };

    let vector_index = opts.vector.unwrap_or(4); // default: ecl_initial_entryPoint (§1's vector 5, 0-indexed here)
    let Some(entry) = machine.vector(vector_index) else {
        eprintln!(
            "restrike: block {} has no resolvable vector at index {vector_index}",
            opts.block
        );
        return ExitCode::FAILURE;
    };
    println!(
        "-- run-script: {} block {} vector[{vector_index}]={entry:#06X} --",
        dax_path.display(),
        opts.block
    );
    machine.enter(entry);

    let mut host = CliHost::new(opts.replies);

    loop {
        if opts.trace {
            if let Some(pc) = machine.current_pc() {
                match decode::decode(&trace_block, pc, &COTAB) {
                    Ok(instr) => eprintln!("{pc:#06X}: {}", render_instr(&instr, &COTAB)),
                    Err(err) => eprintln!("{pc:#06X}: <decode error: {err:?}>"),
                }
            }
        }

        let step = machine.step(&mut host);
        match step {
            Ok(VmStep::Continue) => continue,
            Ok(VmStep::Effect(effect)) => print_effect(&effect),
            Ok(VmStep::Request(request)) => {
                let reply = host.answer(&request);
                println!(
                    "-- REQUEST: {} -> REPLY: {} --",
                    describe_request(&request),
                    describe_reply(&reply)
                );
                if let Err(err) = machine.resume(reply, &mut host) {
                    eprintln!("restrike: resume() rejected the chosen reply: {err:?}");
                    return ExitCode::FAILURE;
                }
            }
            Ok(VmStep::Done(exit)) => {
                println!("-- DONE: {exit:?} --");
                return ExitCode::SUCCESS;
            }
            Err(err) => {
                report_error(&err);
                return ExitCode::FAILURE;
            }
        }
    }
}

fn report_error(err: &VmError) {
    match err {
        VmError::UnknownOpcode { pc, opcode } => eprintln!(
            "restrike: halted at {pc:#06X}: opcode {opcode:#04X} has no dialect entry \
             (the original engine would wedge here too)"
        ),
        VmError::Unimplemented { pc, opcode } => {
            let name = COTAB.lookup(*opcode).map(|i| i.name).unwrap_or("?");
            eprintln!(
                "restrike: halted at {pc:#06X}: opcode {opcode:#04X} ({name}) is known to the \
                 CotAB dialect but not yet implemented by this interpreter"
            );
        }
        other => eprintln!("restrike: halted: {other:?}"),
    }
}

fn print_effect(effect: &Effect) {
    match effect {
        Effect::Print { text, clear_first } => {
            if *clear_first {
                println!("-- [clear] --");
            }
            println!("{}", vm_string_to_display(text));
        }
        Effect::PrintReturn => println!(),
        Effect::Picture(id) => println!("-- [picture {id:#04X}] --"),
        Effect::ClearPicture => println!("-- [clear picture] --"),
        Effect::Sound(id) => println!("-- [sound {id:#04X}] --"),
        Effect::AnimationFrame => println!("-- [animation frame] --"),
    }
}

/// Best-effort text rendering of a script string. Task 1 (ECL inline-string
/// decompression) makes this contain real decoded CotAB text; until then
/// (or for any byte sequence that isn't printable ASCII, e.g. a still-raw
/// packed string), non-printable bytes render as `\xNN` escapes rather than
/// producing garbled terminal output.
fn vm_string_to_display(s: &VmString) -> String {
    let mut out = String::with_capacity(s.0.len());
    for &b in &s.0 {
        if b.is_ascii_graphic() || b == b' ' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02X}"));
        }
    }
    out
}

fn describe_request(request: &Request) -> String {
    match request {
        Request::HorizontalMenu { options } => {
            let opts: Vec<String> = options.iter().map(vm_string_to_display).collect();
            format!("HorizontalMenu[{}]", opts.join(", "))
        }
        Request::Delay => "Delay".to_string(),
        Request::Combat => "Combat".to_string(),
    }
}

fn describe_reply(reply: &Reply) -> String {
    match reply {
        Reply::Selection(i) => format!("Selection({i})"),
        Reply::Delay => "Delay".to_string(),
        Reply::Combat => "Combat".to_string(),
    }
}

struct RunScriptArgs {
    dir: Option<PathBuf>,
    dax: PathBuf,
    block: u8,
    vector: Option<usize>,
    trace: bool,
    replies: ReplyPolicy,
}

impl RunScriptArgs {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut dir = None;
        let mut dax = None;
        let mut block = None;
        let mut vector = None;
        let mut trace = false;
        let mut replies = ReplyPolicy::default();

        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--dax" => {
                    let v = iter.next().ok_or("--dax requires a FILE argument")?;
                    dax = Some(PathBuf::from(v));
                }
                "--block" => {
                    let v = iter.next().ok_or("--block requires an ID argument")?;
                    block = Some(parse_u8(&v).ok_or_else(|| format!("invalid --block '{v}'"))?);
                }
                "--vector" => {
                    let v = iter.next().ok_or("--vector requires an N argument")?;
                    vector = Some(
                        v.parse::<usize>()
                            .map_err(|_| format!("invalid --vector '{v}'"))?,
                    );
                }
                "--trace" => trace = true,
                "--reply" => {
                    let v = iter.next().ok_or("--reply requires a k=v argument")?;
                    replies.push(&v)?;
                }
                other if dir.is_none() && !other.starts_with("--") => {
                    dir = Some(PathBuf::from(other));
                }
                other => return Err(format!("unknown run-script flag '{other}'")),
            }
        }

        Ok(RunScriptArgs {
            dir,
            dax: dax.ok_or("run-script requires --dax <FILE>")?,
            block: block.ok_or("run-script requires --block <ID>")?,
            vector,
            trace,
            replies,
        })
    }
}

fn parse_u8(s: &str) -> Option<u8> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

/// `--reply k=v` overrides, queued per request kind (`menu`, `delay`,
/// `combat`) and consumed in the order each kind's `Request` is next
/// encountered. Exhausted kinds fall back to the documented default policy
/// (`ReplyPolicy::answer`).
#[derive(Default)]
struct ReplyPolicy {
    menu: VecDeque<u8>,
}

impl ReplyPolicy {
    fn push(&mut self, spec: &str) -> Result<(), String> {
        let (key, value) = spec
            .split_once('=')
            .ok_or_else(|| format!("--reply '{spec}' must be key=value (e.g. menu=1)"))?;
        match key {
            "menu" => {
                let index = parse_u8(value)
                    .ok_or_else(|| format!("--reply menu value '{value}' must be a number"))?;
                self.menu.push_back(index);
            }
            "delay" | "combat" => {} // no data carried; presence alone documents intent
            other => {
                return Err(format!(
                    "unknown --reply key '{other}' (want menu/delay/combat)"
                ))
            }
        }
        Ok(())
    }

    /// The documented default policy (task brief): menus pick the first
    /// option, delay/combat requests are answered immediately with no
    /// scripted duration/outcome (the engine, not this diagnostic host,
    /// eventually owns pacing and combat resolution).
    fn answer(&mut self, request: &Request) -> Reply {
        match request {
            Request::HorizontalMenu { .. } => Reply::Selection(self.menu.pop_front().unwrap_or(0)),
            Request::Delay => Reply::Delay,
            Request::Combat => Reply::Combat,
        }
    }
}

/// The CLI's throwaway `VmHost`: a raw word/byte/string store (every access
/// logged to stderr), `EngineServices` calls trace-logged with neutral
/// scripted results, and the engine's one PRNG (D-OR1: no second RNG may
/// exist — this diagnostic host draws from `gbx-prng` like everything else).
struct CliHost {
    words: HashMap<u16, u16>,
    bytes: HashMap<u16, u8>,
    strings: HashMap<u16, VmString>,
    replies: ReplyPolicy,
    rng: Prng,
}

/// Fixed, arbitrary seed — deterministic across runs, not derived from
/// wall-clock/process state (D9's spirit: no hidden nondeterminism). This host
/// is a diagnostic throwaway, but it still uses the real engine PRNG so its
/// rolls are the same generator the game uses.
const RNG_SEED: u32 = 0xC0FF_EE15;

impl CliHost {
    fn new(replies: ReplyPolicy) -> Self {
        CliHost {
            words: HashMap::new(),
            bytes: HashMap::new(),
            strings: HashMap::new(),
            replies,
            rng: Prng::new(RNG_SEED),
        }
    }

    fn answer(&mut self, request: &Request) -> Reply {
        self.replies.answer(request)
    }
}

impl ScriptMemory for CliHost {
    fn read(&mut self, addr: u16, origin: Origin) -> u16 {
        let value = self.words.get(&addr).copied().unwrap_or(0);
        eprintln!(
            "mem: read  word {addr:#06X} = {value:#06X} (pc={:#06X})",
            origin.pc
        );
        value
    }

    fn write(&mut self, addr: u16, value: u16, origin: Origin) {
        eprintln!(
            "mem: write word {addr:#06X} = {value:#06X} (pc={:#06X})",
            origin.pc
        );
        self.words.insert(addr, value);
    }

    fn read_byte(&mut self, addr: u16, origin: Origin) -> u8 {
        let value = self.bytes.get(&addr).copied().unwrap_or(0);
        eprintln!(
            "mem: read  byte {addr:#06X} = {value:#04X} (pc={:#06X})",
            origin.pc
        );
        value
    }

    fn write_byte(&mut self, addr: u16, value: u8, origin: Origin) {
        eprintln!(
            "mem: write byte {addr:#06X} = {value:#04X} (pc={:#06X})",
            origin.pc
        );
        self.bytes.insert(addr, value);
    }

    fn read_string(&mut self, addr: u16, origin: Origin) -> VmString {
        let value = self.strings.get(&addr).cloned().unwrap_or_default();
        eprintln!(
            "mem: read  string {addr:#06X} = {} byte(s) (pc={:#06X})",
            value.0.len(),
            origin.pc
        );
        value
    }

    fn write_string(&mut self, addr: u16, s: &VmString, origin: Origin) {
        eprintln!(
            "mem: write string {addr:#06X} = {} byte(s) (pc={:#06X})",
            s.0.len(),
            origin.pc
        );
        self.strings.insert(addr, s.clone());
    }
}

/// Every method logs a trace line (task brief: "service calls printed as a
/// trace line") and returns a neutral, deterministic result — real dice
/// rolls go through the seeded RNG; everything else is a fixed, documented
/// default (`false`/`0`/`Ok`) since this host has no game-entity state to
/// consult.
impl EngineServices for CliHost {
    fn retarget_selected_player(&mut self, index: u8) -> Result<(), NotFound> {
        eprintln!("svc: retarget_selected_player(index={index})");
        Ok(())
    }

    fn free_current_player(&mut self, free_icon: bool, leave_party_size: bool) -> PlayerId {
        eprintln!(
            "svc: free_current_player(free_icon={free_icon}, leave_party_size={leave_party_size})"
        );
        PlayerId(0)
    }

    fn party_strength(&mut self) -> u8 {
        eprintln!("svc: party_strength()");
        0
    }

    fn check_party(&mut self, query: u16) -> u16 {
        eprintln!("svc: check_party(query={query:#06X})");
        0
    }

    fn party_has_item(&mut self, item_type: u8) -> bool {
        eprintln!("svc: party_has_item(item_type={item_type:#04X})");
        false
    }

    fn find_special(&mut self, affect_type: u8) -> bool {
        eprintln!("svc: find_special(affect_type={affect_type:#04X})");
        false
    }

    fn destroy_items(&mut self, item_type: u8) {
        eprintln!("svc: destroy_items(item_type={item_type:#04X})");
    }

    fn rob_money(&mut self, pct: u8) {
        eprintln!("svc: rob_money(pct={pct})");
    }

    fn rob_items(&mut self, chance: u8) {
        eprintln!("svc: rob_items(chance={chance})");
    }

    fn party_surprise_check(&mut self) -> (u8, u8) {
        eprintln!("svc: party_surprise_check()");
        (0, 0)
    }

    fn load_monster(
        &mut self,
        monster_id: u8,
        num_copies: u8,
        icon_block_id: u8,
    ) -> Result<MonsterHandle, MissingData> {
        eprintln!(
            "svc: load_monster(monster_id={monster_id}, num_copies={num_copies}, \
             icon_block_id={icon_block_id})"
        );
        Ok(MonsterHandle(monster_id as u16))
    }

    fn setup_monster(&mut self, sprite_id: u8, max_distance: u8, pic_id: u8) {
        eprintln!(
            "svc: setup_monster(sprite_id={sprite_id}, max_distance={max_distance}, \
             pic_id={pic_id})"
        );
    }

    fn clear_monsters(&mut self) {
        eprintln!("svc: clear_monsters()");
    }

    fn add_npc(&mut self, monster_id: u8, morale: u8) {
        eprintln!("svc: add_npc(monster_id={monster_id}, morale={morale})");
    }

    fn setup_duel(&mut self, is_duel: bool) {
        eprintln!("svc: setup_duel(is_duel={is_duel})");
    }

    fn calc_group_movement(&mut self) -> (u8, u8) {
        eprintln!("svc: calc_group_movement()");
        (0, 0)
    }

    fn approach_distance(&mut self) -> u8 {
        eprintln!("svc: approach_distance()");
        0
    }

    fn load_encounter_visual(&mut self, flags: u8, distance: u8, pic_id: u8, sprite_id: u8) {
        eprintln!(
            "svc: load_encounter_visual(flags={flags}, distance={distance}, pic_id={pic_id}, \
             sprite_id={sprite_id})"
        );
    }

    fn create_item(&mut self, item_type: u8) -> ItemHandle {
        eprintln!("svc: create_item(item_type={item_type:#04X})");
        ItemHandle(0)
    }

    fn load_item_from_table(&mut self, block_id: u8) -> ItemHandle {
        eprintln!("svc: load_item_from_table(block_id={block_id:#04X})");
        ItemHandle(0)
    }

    fn find_spell_in_party(&mut self, spell_id: u8) -> (u8, u8) {
        eprintln!("svc: find_spell_in_party(spell_id={spell_id})");
        (0xFF, 0xFF)
    }

    fn roll(&mut self, max: u8) -> u8 {
        // Corrected off-by-one, same as the engine host (oracle-rig §6 ledger):
        // exclusive `random(max)`, not the old inclusive `roll_uniform(max)`.
        let value = self.rng.random(max as u16) as u8;
        eprintln!("svc: roll(max={max}) = {value}");
        value
    }

    fn roll_dice(&mut self, size: u8, count: u8) -> u16 {
        // `1 + random(size)` per die; `size == 0` draws (ovr024.cs:586-598).
        let mut total = 0u16;
        for _ in 0..count {
            total += 1 + self.rng.random(size as u16);
        }
        eprintln!("svc: roll_dice(size={size}, count={count}) = {total}");
        total
    }

    fn roll_saving_throw(&mut self, bonus: u8, save_type: u8) -> bool {
        eprintln!("svc: roll_saving_throw(bonus={bonus}, save_type={save_type})");
        false
    }

    fn can_hit_target(&mut self, bonus: u8) -> bool {
        eprintln!("svc: can_hit_target(bonus={bonus})");
        false
    }

    fn apply_damage(&mut self, player: PlayerId, damage: u16) {
        eprintln!("svc: apply_damage(player={}, damage={damage})", player.0);
    }

    fn load_3d_map(&mut self, block_id: u8) {
        eprintln!("svc: load_3d_map(block_id={block_id})");
    }

    fn load_walldef(&mut self, set: u8, id: u8) {
        eprintln!("svc: load_walldef(set={set}, id={id})");
    }

    fn load_bigpic(&mut self, id: u8) {
        eprintln!("svc: load_bigpic(id={id:#04X})");
    }

    fn reset_wall_set(&mut self, index: u8) {
        eprintln!("svc: reset_wall_set(index={index})");
    }

    fn step_game_time(&mut self, time_slot: u8, amount: u8) {
        eprintln!("svc: step_game_time(time_slot={time_slot}, amount={amount})");
    }

    fn move_position_forward(&mut self) {
        eprintln!("svc: move_position_forward()");
    }

    fn wall_roof(&mut self) -> u8 {
        eprintln!("svc: wall_roof()");
        0
    }

    fn wall_type(&mut self) -> u8 {
        eprintln!("svc: wall_type()");
        0
    }

    fn call_sound_variant(&mut self) -> u8 {
        eprintln!("svc: call_sound_variant()");
        0
    }
}

impl VmRng for CliHost {
    fn random(&mut self, n: u16) -> u16 {
        self.rng.random(n)
    }
}

impl VmHost for CliHost {
    fn rng(&mut self) -> &mut dyn VmRng {
        self
    }
}

fn print_usage() {
    eprintln!(
        "usage: restrike run-script [DIR] --dax <FILE> --block <ID> [--vector N] [--trace] \
         [--reply k=v ...]"
    );
    eprintln!();
    eprintln!(
        "Loads ECL block <ID> from <FILE> (resolved under DIR, or GBX_DATA_DIR if DIR is \
         omitted), enters vector <N> (default 4, the ecl_initial_entryPoint), and runs it \
         headlessly against a diagnostic CLI host. Effects and request/reply pairs print to \
         stdout; memory access and service-call traces print to stderr. --trace additionally \
         disassembles each instruction to stderr as it executes."
    );
}
