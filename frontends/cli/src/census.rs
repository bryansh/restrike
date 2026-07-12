//! `restrike census [DIR]` — the project dashboard (PLAN.md §2.6, D-VM8's
//! census hazard reports). Pipeline: detect the game, find every `ECL*.DAX`
//! in `DIR`, extract every block, decode each block's header vectors, run
//! the flow-following disassembler (`gbx_vm::disassemble`) from those
//! vectors, and aggregate opcode frequency + hazard statistics across every
//! block. This module is CLI-only (filesystem access) — the pure decode/
//! disassemble logic it drives lives in `gbx-vm`/`gbx-formats`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gbx_formats::dax::{self, DaxArchive};
use gbx_formats::detect;
use gbx_vm::dialect::COTAB_VECTOR_COUNT;
use gbx_vm::{disassemble, Arg, BlockBytes, DecodeError, Hazard, Listing, COTAB, ECL_BLOCK_SIZE};

/// One disassembled block plus the metadata needed to cite it in a report.
struct BlockCensus {
    file: String,
    block_id: u8,
    vectors: Vec<Option<u16>>,
    listing: Listing,
}

pub fn cmd_census(args: Vec<String>) -> ExitCode {
    let mut dir_arg: Option<String> = None;
    let mut out_path: Option<PathBuf> = None;

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out" => {
                let Some(path) = iter.next() else {
                    eprintln!("restrike: --out requires a PATH argument");
                    return ExitCode::FAILURE;
                };
                out_path = Some(PathBuf::from(path));
            }
            other if dir_arg.is_none() && !other.starts_with("--") => {
                dir_arg = Some(other.to_string());
            }
            other => {
                eprintln!("restrike: unknown census flag '{other}'");
                return ExitCode::FAILURE;
            }
        }
    }

    let dir = match dir_arg
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("GBX_DATA_DIR").map(PathBuf::from))
    {
        Some(dir) => dir,
        None => {
            eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
            return ExitCode::FAILURE;
        }
    };
    if !dir.is_dir() {
        eprintln!("restrike: '{}' is not a directory", dir.display());
        return ExitCode::FAILURE;
    }

    let game = match detect::detect_dir(&dir) {
        Ok(detect::Detection::Known { game, .. }) => game.to_string(),
        Ok(detect::Detection::Unknown { .. }) => "unknown game".to_string(),
        Err(err) => {
            eprintln!("restrike: failed to scan '{}': {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };

    let ecl_files = match find_ecl_dax_files(&dir) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("restrike: failed to read '{}': {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };
    if ecl_files.is_empty() {
        eprintln!("restrike: no ECL*.DAX files found in '{}'", dir.display());
        return ExitCode::FAILURE;
    }

    let mut blocks = Vec::new();
    for path in &ecl_files {
        match census_one_file(path) {
            Ok(mut file_blocks) => blocks.append(&mut file_blocks),
            Err(err) => {
                eprintln!("restrike: {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        }
    }

    let report = Report::build(&game, blocks);

    match out_path {
        Some(path) => {
            if let Err(err) = fs::write(&path, report.to_csv()) {
                eprintln!("restrike: failed to write '{}': {err}", path.display());
                return ExitCode::FAILURE;
            }
            eprint!("{}", report.human_report());
        }
        None => {
            print!("{}", report.to_csv());
            eprint!("{}", report.human_report());
        }
    }

    ExitCode::SUCCESS
}

fn find_ecl_dax_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_uppercase();
        if name.starts_with("ECL") && name.ends_with(".DAX") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn census_one_file(path: &Path) -> Result<Vec<BlockCensus>, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read: {e}"))?;
    let archive = DaxArchive::parse(&bytes).map_err(|e| format!("failed to parse DAX: {e:?}"))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>")
        .to_string();

    let mut out = Vec::new();
    for entry in archive.entries() {
        let raw = archive
            .block_data(entry.id)
            .map_err(|e| format!("block {}: failed to extract: {e:?}", entry.id))?;
        let payload = dax::ecl_block_payload(&raw);
        if payload.len() > ECL_BLOCK_SIZE {
            return Err(format!(
                "block {}: ECL payload is {} bytes, exceeding the 0x1E00-byte resident block \
                 size",
                entry.id,
                payload.len()
            ));
        }
        let block = BlockBytes::from_bytes(payload);
        let (vectors, _code_start) = gbx_vm::read_header_vectors(&block, COTAB_VECTOR_COUNT);
        let entry_points: Vec<u16> = vectors.iter().filter_map(|v| *v).collect();
        let listing = disassemble(&block, &COTAB, &entry_points);

        out.push(BlockCensus {
            file: file_name.clone(),
            block_id: entry.id,
            vectors,
            listing,
        });
    }
    Ok(out)
}

/// A memory address, classified into a ScriptMemory window
/// (`docs/design/vm-scriptmemory.md` §1's window table) — used to answer
/// docket question 6 (writes/reads addressing the Global window).
fn window_name(addr: u16) -> &'static str {
    match addr {
        0x4B00..=0x4EFF => "Area",
        0x7A00..=0x7BFF => "Table",
        0x7C00..=0x7FFF => "Party",
        0x8000..=0x9DFF => "Ecl",
        _ => "Global",
    }
}

/// The seven known CALL (0x2D) dispatch keys, per
/// `docs/design/opcode-classification.md` §3's full enumeration — used to
/// flag any key seen on real data that isn't one of these.
const KNOWN_CALL_KEYS: &[u16] = &[0xAE11, 1, 2, 0x3201, 0x401F, 0x4019, 0xE804];

/// The six opcodes with a confirmed skip-size-vs-run-consumption divergence
/// (`docs/design/vm-scriptmemory.md` §1) — used to distinguish "the known
/// hazard fired" from "a *new* skip-divergent opcode showed up."
const KNOWN_SKIP_DIVERGENT_OPCODES: &[u8] = &[0x15, 0x25, 0x26, 0x2B, 0x34, 0x36];

struct Citation {
    file: String,
    block_id: u8,
    addr: u16,
}

impl std::fmt::Display for Citation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}#{} @ {:#06X}", self.file, self.block_id, self.addr)
    }
}

struct Report {
    game: String,
    block_count: usize,
    file_count: usize,
    opcode_counts: BTreeMap<u8, usize>,
    total_instructions: usize,

    op_1f_reached: Vec<Citation>,
    op_1f_quarantined: Vec<Citation>,

    skip_divergences: Vec<(Citation, u16, u8)>, // (if-site, skip_target, skip-target opcode)
    unknown_skip_targets: Vec<(Citation, u8)>,  // (if-site, unknown opcode byte)
    new_skip_divergent_opcodes: BTreeMap<u8, usize>,

    unresolved_variable_tails: Vec<Citation>,
    unknown_modes: Vec<(Citation, u8, u8)>, // (site, mode, byte)

    call_keys: BTreeMap<u16, Vec<Citation>>,

    mem_mode_01_count: usize,
    mem_mode_03_count: usize,
    global_window_hits: Vec<(Citation, u16)>, // (site, address)
    surprise_cell_hits: Vec<Citation>,

    #[allow(clippy::type_complexity)]
    per_block_coverage: Vec<(String, u8, usize, usize, usize, Vec<Option<u16>>)>, // file, id, code, data, error, vectors
    unknown_opcode_decode_errors: Vec<(Citation, u8)>, // reached via NORMAL traversal

    /// Traversal reached an instruction or decode-error site whose address
    /// falls *outside* the block's own `0x8000..=0x9DFF` window — a jump/
    /// call/computed-table target resolved to a raw word pointing elsewhere
    /// in the 16-bit address space. `BlockBytes::get` still services the
    /// read (wraps mod `0x1E00`, matching the original's own `& 0xFFFF`
    /// indexer — D-VM2), so this is faithfully-followed behavior, not a
    /// traversal bug; it's flagged separately because it's surprising
    /// enough to warrant human review (§4 Contradictions in the report).
    out_of_block_targets: Vec<(Citation, &'static str)>, // (site, "instruction" | "error")
}

impl Report {
    fn build(game: &str, blocks: Vec<BlockCensus>) -> Self {
        let mut opcode_counts: BTreeMap<u8, usize> = BTreeMap::new();
        let mut op_1f_reached = Vec::new();
        let mut op_1f_quarantined = Vec::new();
        let mut skip_divergences = Vec::new();
        let mut unknown_skip_targets = Vec::new();
        let mut new_skip_divergent_opcodes: BTreeMap<u8, usize> = BTreeMap::new();
        let mut unresolved_variable_tails = Vec::new();
        let mut unknown_modes = Vec::new();
        let mut call_keys: BTreeMap<u16, Vec<Citation>> = BTreeMap::new();
        let mut mem_mode_01_count = 0usize;
        let mut mem_mode_03_count = 0usize;
        let mut global_window_hits = Vec::new();
        let mut surprise_cell_hits = Vec::new();
        let mut per_block_coverage = Vec::new();
        let mut unknown_opcode_decode_errors = Vec::new();
        let mut out_of_block_targets = Vec::new();

        let block_count = blocks.len();
        let mut files = std::collections::BTreeSet::new();

        for b in &blocks {
            files.insert(b.file.clone());
            let cite = |addr: u16| Citation {
                file: b.file.clone(),
                block_id: b.block_id,
                addr,
            };

            for (&addr, instr) in &b.listing.instructions {
                *opcode_counts.entry(instr.op.0).or_insert(0) += 1;
                if instr.op.0 == 0x1F {
                    op_1f_reached.push(cite(addr));
                }
                if instr.op.0 == 0x2D {
                    if let Some(word) = instr.args.first().and_then(Arg::raw_word) {
                        let key = word.wrapping_sub(0x7FFF);
                        call_keys.entry(key).or_default().push(cite(addr));
                    }
                }
                for arg in &instr.args {
                    match arg {
                        Arg::UnknownMode { mode, byte } => {
                            unknown_modes.push((cite(addr), *mode, *byte));
                        }
                        Arg::Mem(w) => {
                            mem_mode_01_count += 1;
                            if window_name(*w) == "Global" {
                                global_window_hits.push((cite(addr), *w));
                            }
                            if *w == 0x02CB {
                                surprise_cell_hits.push(cite(addr));
                            }
                        }
                        Arg::MemAlt(w) => {
                            mem_mode_03_count += 1;
                            if window_name(*w) == "Global" {
                                global_window_hits.push((cite(addr), *w));
                            }
                            if *w == 0x02CB {
                                surprise_cell_hits.push(cite(addr));
                            }
                        }
                        _ => {}
                    }
                }
            }

            for (&addr, err) in &b.listing.errors {
                if let DecodeError::UnknownOpcode { opcode, .. } = err {
                    unknown_opcode_decode_errors.push((cite(addr), *opcode));
                }
                if let DecodeError::UnresolvedVariableTail { .. } = err {
                    unresolved_variable_tails.push(cite(addr));
                }
            }

            for hazard in &b.listing.hazards {
                match hazard {
                    Hazard::SkipDivergence {
                        if_addr,
                        next_addr,
                        skip_target,
                    } => {
                        // The divergent opcode is the one at `next_addr` (the
                        // IF's fall-through, always normally decoded too —
                        // `disasm.rs` enqueues it before computing the skip
                        // successor) whose declared `skip_size` disagreed
                        // with its own real consumption. `skip_target` is
                        // where the *skip* lands (mid-operand garbage), not
                        // where that opcode is.
                        let divergent_opcode = b
                            .listing
                            .instructions
                            .get(next_addr)
                            .map(|i| i.op.0)
                            .unwrap_or(0);
                        skip_divergences.push((cite(*if_addr), *skip_target, divergent_opcode));
                        let quarantined_opcode = b
                            .listing
                            .quarantine
                            .get(skip_target)
                            .and_then(|r| r.as_ref().ok())
                            .map(|i| i.op.0);
                        if quarantined_opcode == Some(0x1F) {
                            op_1f_quarantined.push(cite(*skip_target));
                        }
                    }
                    Hazard::UnknownSkipTargetOpcode {
                        if_addr, opcode, ..
                    } => {
                        unknown_skip_targets.push((cite(*if_addr), *opcode));
                    }
                    _ => {}
                }
            }

            // Coverage is computed independently here rather than trusting
            // `Listing::data_regions` directly: real CotAB scripts contain
            // jump/call/computed-table targets whose raw word resolves
            // *outside* the block's own 0x8000..=0x9DFF window (see the
            // `out_of_block_targets` doc comment) — `compute_data_regions`
            // assumes every traversed key falls inside that window, so an
            // out-of-range key corrupts its region-merge arithmetic. Rather
            // than patch that assumption into the shared disassembler (the
            // out-of-range target is a genuine, faithfully-followed finding,
            // not a bug to paper over — see §4 of the census report), this
            // census-only bitmap reduction sidesteps it: only in-block bytes
            // are tallied, and every out-of-block site is reported on its
            // own via `out_of_block_targets`.
            let mut reached = [false; ECL_BLOCK_SIZE];
            let mut error_bytes = 0usize;
            for (&addr, instr) in &b.listing.instructions {
                if !(0x8000..=0x9DFF).contains(&addr) {
                    out_of_block_targets.push((cite(addr), "instruction"));
                    continue;
                }
                let start = (addr - 0x8000) as usize;
                let len = instr.next.wrapping_sub(addr) as usize;
                let end = (start + len).min(ECL_BLOCK_SIZE);
                reached[start..end].fill(true);
            }
            for &addr in b.listing.errors.keys() {
                if !(0x8000..=0x9DFF).contains(&addr) {
                    out_of_block_targets.push((cite(addr), "error"));
                    continue;
                }
                reached[(addr - 0x8000) as usize] = true; // D-VM6: the original wedges after 1 byte
                error_bytes += 1;
            }
            let code_bytes = reached.iter().filter(|&&r| r).count() - error_bytes;
            let data_bytes = ECL_BLOCK_SIZE - code_bytes - error_bytes;
            per_block_coverage.push((
                b.file.clone(),
                b.block_id,
                code_bytes,
                data_bytes,
                error_bytes,
                b.vectors.clone(),
            ));
        }

        // Cross-reference: which opcode(s) triggered a skip divergence that
        // AREN'T one of the six already-documented ones?
        for (_, _, opcode) in &skip_divergences {
            if !KNOWN_SKIP_DIVERGENT_OPCODES.contains(opcode) {
                *new_skip_divergent_opcodes.entry(*opcode).or_insert(0) += 1;
            }
        }

        let total_instructions = opcode_counts.values().sum();

        Self {
            game: game.to_string(),
            block_count,
            file_count: files.len(),
            opcode_counts,
            total_instructions,
            op_1f_reached,
            op_1f_quarantined,
            skip_divergences,
            unknown_skip_targets,
            new_skip_divergent_opcodes,
            unresolved_variable_tails,
            unknown_modes,
            call_keys,
            mem_mode_01_count,
            mem_mode_03_count,
            global_window_hits,
            surprise_cell_hits,
            per_block_coverage,
            unknown_opcode_decode_errors,
            out_of_block_targets,
        }
    }

    fn to_csv(&self) -> String {
        let mut out = String::from("opcode,name,count,pct_of_total\n");
        let mut ranked: Vec<(u8, usize)> =
            self.opcode_counts.iter().map(|(&k, &v)| (k, v)).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (op, count) in ranked {
            let name = COTAB.lookup(op).map(|i| i.name).unwrap_or("<unknown>");
            let pct = if self.total_instructions == 0 {
                0.0
            } else {
                100.0 * count as f64 / self.total_instructions as f64
            };
            let _ = writeln!(out, "{op:#04X},{name},{count},{pct:.3}");
        }
        out
    }

    fn human_report(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "== restrike census: {} ==\n{} file(s), {} block(s), {} reached instruction(s)\n\n",
            self.game, self.file_count, self.block_count, self.total_instructions
        );

        let _ = write!(
            out,
            "-- docket 1: opcode 0x1F --\nreached (normal traversal): {}\nreached (skip-quarantine only): {}\n",
            self.op_1f_reached.len(),
            self.op_1f_quarantined.len()
        );
        for c in self.op_1f_reached.iter().chain(&self.op_1f_quarantined) {
            let _ = writeln!(out, "  {c}");
        }

        let _ = write!(
            out,
            "\n-- docket 2: IF before a skip-divergent opcode --\n{} skip-divergence hazard(s), {} unknown-skip-target hazard(s)\n",
            self.skip_divergences.len(),
            self.unknown_skip_targets.len()
        );
        for (cite, target, opcode) in &self.skip_divergences {
            let name = COTAB.lookup(*opcode).map(|i| i.name).unwrap_or("?");
            let _ = writeln!(
                out,
                "  {cite} -> skip target {target:#06X} ({name}, {opcode:#04X})"
            );
        }
        if !self.new_skip_divergent_opcodes.is_empty() {
            let _ = writeln!(out, "  NEW divergent opcodes not in the known six:");
            for (op, n) in &self.new_skip_divergent_opcodes {
                let _ = writeln!(out, "    {op:#04X}: {n} occurrence(s)");
            }
        }
        for (cite, opcode) in &self.unknown_skip_targets {
            let _ = writeln!(out, "  {cite} -> unknown skip-target opcode {opcode:#04X}");
        }

        let _ = write!(
            out,
            "\n-- docket 3: unresolved (memory-mode) variable-tail counts --\n{} occurrence(s)\n",
            self.unresolved_variable_tails.len()
        );
        for c in &self.unresolved_variable_tails {
            let _ = writeln!(out, "  {c}");
        }

        let _ = write!(
            out,
            "\n-- docket 4: unknown mode bytes in reached code --\n{} occurrence(s)\n",
            self.unknown_modes.len()
        );
        for (cite, mode, byte) in &self.unknown_modes {
            let _ = writeln!(out, "  {cite} mode={mode:#04X} byte={byte:#04X}");
        }

        let _ = write!(
            out,
            "\n-- docket 5: CALL (0x2D) operand keys --\n{} distinct key(s)\n",
            self.call_keys.len()
        );
        for (key, cites) in &self.call_keys {
            let known = if KNOWN_CALL_KEYS.contains(key) {
                "known"
            } else {
                "UNKNOWN"
            };
            let _ = writeln!(out, "  key {key:#06X} [{known}]: {} use(s)", cites.len());
            for c in cites {
                let _ = writeln!(out, "    {c}");
            }
        }

        let total_mem = self.mem_mode_01_count + self.mem_mode_03_count;
        let _ = write!(
            out,
            "\n-- docket 6: operand mode 0x01 vs 0x03, Global-window writes --\n\
             mode 0x01 (Mem): {} ({:.1}%)\nmode 0x03 (MemAlt): {} ({:.1}%)\n\
             NOTE: this is the overall distribution across every reached memory-mode \
             operand; distinguishing *write-destination* operands specifically would \
             require per-opcode operand-role semantics (which operand index each \
             opcode treats as a destination) — that's interpreter-level knowledge, out \
             of the census tool's scope (task brief \"out of scope\": EclMachine). \
             Reported instead: every memory-mode operand address whose window \
             classifies as Global ({} of {} total memory-mode operands), and the \
             SURPRISE (0x23) 0x2CB pattern specifically ({} hit(s)).\n",
            self.mem_mode_01_count,
            pct(self.mem_mode_01_count, total_mem),
            self.mem_mode_03_count,
            pct(self.mem_mode_03_count, total_mem),
            self.global_window_hits.len(),
            total_mem,
            self.surprise_cell_hits.len(),
        );
        for (cite, addr) in &self.global_window_hits {
            let _ = writeln!(out, "  {cite} -> {addr:#06X} (Global)");
        }
        for c in &self.surprise_cell_hits {
            let _ = writeln!(out, "  {c} -> 0x02CB (SURPRISE result cell)");
        }

        let _ = writeln!(out, "\n-- docket 7: per-block coverage --");
        for (file, id, code, data, err, vectors) in &self.per_block_coverage {
            let vecs = vectors
                .iter()
                .map(|v| match v {
                    Some(w) => format!("{w:#06X}"),
                    None => "?".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "  {file}#{id}: code={code} ({:.1}%) data={data} ({:.1}%) error={err} bytes \
                 vectors=[{vecs}]",
                pct(*code, ECL_BLOCK_SIZE),
                pct(*data, ECL_BLOCK_SIZE),
            );
        }
        let _ = write!(
            out,
            "\n-- docket 7: decode desyncs (UnknownOpcode via NORMAL traversal, not skip-quarantine) --\n{} occurrence(s)\n",
            self.unknown_opcode_decode_errors.len()
        );
        for (cite, opcode) in &self.unknown_opcode_decode_errors {
            let _ = writeln!(out, "  {cite} opcode={opcode:#04X}");
        }
        let _ = write!(
            out,
            "\n-- docket 7: out-of-block traversal targets (raw address outside 0x8000..=0x9DFF) --\n{} occurrence(s)\n",
            self.out_of_block_targets.len()
        );
        for (cite, kind) in &self.out_of_block_targets {
            let _ = writeln!(out, "  {cite} ({kind})");
        }

        let _ = writeln!(out, "\n-- docket 8: frequency-ordered opcode list --");
        let mut ranked: Vec<(u8, usize)> =
            self.opcode_counts.iter().map(|(&k, &v)| (k, v)).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (rank, (op, count)) in ranked.iter().enumerate() {
            let name = COTAB.lookup(*op).map(|i| i.name).unwrap_or("<unknown>");
            let marker = if rank < 25 { "*" } else { " " };
            let _ = writeln!(
                out,
                " {marker} {:>2}. {op:#04X} {name:<18} {count:>5} ({:.2}%)",
                rank + 1,
                pct(*count, self.total_instructions)
            );
        }

        out
    }
}

fn pct(n: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * n as f64 / total as f64
    }
}
