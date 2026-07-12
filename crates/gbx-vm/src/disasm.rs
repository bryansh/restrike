//! Flow-following ECL disassembler (D-VM8,
//! `docs/design/vm-scriptmemory.md` §2 and §6 build-order item 1).
//!
//! A linear byte sweep desyncs at the first embedded data byte (in-block
//! strings, GETTABLE/SAVETABLE tables, self-modified regions), so this
//! traverses from known entry points instead, following each opcode's
//! table-driven [`SuccessorKind`] (never a hard-coded match on opcode
//! number — that's the whole point of D-VM7's "dialects are data" rule).
//! Anything never reached by that traversal is reported as a data region,
//! not guessed at.
//!
//! **Skip is not decode** (`vm-scriptmemory.md` §1): an IF's false-flag path
//! advances by the *next* opcode's declared `skip_size`, not by how many
//! bytes that opcode's operands actually occupy. When those disagree (the
//! confirmed `0x15`/`0x25`/`0x26`/`0x2B`/`0x34`/`0x36` cases, and — because
//! `decode()` doesn't special-case opcodes — potentially any dialect entry
//! with the same shape of mismatch), the skip successor lands mid-operand:
//! a **hazard site**. This module decodes those into a tagged quarantine
//! bucket, kept out of the normal instruction listing and out of
//! [`Summary`]'s headline counts, per D-VM8.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::decode::{
    decode, skip_batches, Arg, BlockBytes, DecodeError, Instr, ECL_BLOCK_BASE, ECL_BLOCK_SIZE,
};
use crate::dialect::{Dialect, OperandShape, SuccessorKind};

/// One past the last valid ECL block address (`ECL_BLOCK_BASE + ECL_BLOCK_SIZE`).
const BLOCK_END: u16 = ECL_BLOCK_BASE + ECL_BLOCK_SIZE as u16;

fn in_block(addr: u16) -> bool {
    (ECL_BLOCK_BASE..BLOCK_END).contains(&addr)
}

/// A D-VM8 traversal diagnostic. Every variant is a *report*, not a halt —
/// unlike the interpreter (D-VM6), disassembly never stops at trouble; it
/// marks the site and keeps going from the next known target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hazard {
    /// `decode()` failed for an address reached through normal traversal
    /// (unknown opcode, or an unresolved variable-tail count). The address
    /// contributes no instruction to the listing; traversal simply doesn't
    /// continue past it on this path.
    DecodeError { addr: u16, error: DecodeError },
    /// The canonical D-VM8 hazard: an IF's skip successor (computed from the
    /// *following* opcode's `skip_size`) disagrees with that opcode's real
    /// decoded length. `skip_target` is decoded into the quarantine bucket.
    SkipDivergence {
        if_addr: u16,
        next_addr: u16,
        skip_target: u16,
    },
    /// An IF's skip successor couldn't be computed at all: the opcode byte
    /// at `next_addr` (where the maybe-skipped instruction starts) has no
    /// dialect entry, so its `skip_size` is unknown.
    UnknownSkipTargetOpcode {
        if_addr: u16,
        next_addr: u16,
        opcode: u8,
    },
    /// A jump/call/computed-table target operand didn't carry a resolvable
    /// raw word (`Arg::raw_word` returned `None`) — `decode()` tolerates the
    /// mode byte, but static traversal can't follow a target it can't read.
    UnresolvedJumpTarget { addr: u16, opcode: u8 },
}

/// A contiguous span of bytes no traversal path reached — rendered as data,
/// per D-VM8 ("marks unreached bytes as data, resynchronizes only at known
/// targets"). `referenced_by` lists the addresses of in-block `0x81`
/// (`MemStr`) operands whose target falls inside this span, so a reader can
/// tell embedded strings apart from truly-unaccounted-for bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRegion {
    pub start: u16,
    pub len: u16,
    pub referenced_by: Vec<u16>,
}

/// The machine-usable half of a disassembly (D-VM8: "must report, not
/// assume"): per-opcode reached-instruction counts, the hazard list, and
/// data-region spans. This is the surface a future census tool consumes —
/// this module does not build the census itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub opcode_reached_counts: BTreeMap<u8, usize>,
    pub hazards: Vec<Hazard>,
    pub data_region_spans: Vec<(u16, u16)>,
}

/// The result of a flow-following traversal over one ECL block.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Normally-reached instructions, keyed by their starting address.
    pub instructions: BTreeMap<u16, Instr>,
    /// Addresses reached by normal traversal where `decode()` failed.
    pub errors: BTreeMap<u16, DecodeError>,
    /// Skip-divergence hazard sites: decoded (or attempted), but excluded
    /// from `instructions`/`errors` and from every coverage count — never
    /// merged into normal listing/coverage, per D-VM8.
    pub quarantine: BTreeMap<u16, Result<Instr, DecodeError>>,
    /// Labels at every statically-known jump/call/table target, plus the
    /// caller-supplied entry points. Multiple names at one address are
    /// possible (e.g. an entry point that's also a GOTO target).
    pub labels: BTreeMap<u16, BTreeSet<String>>,
    pub data_regions: Vec<DataRegion>,
    pub hazards: Vec<Hazard>,
    /// In-block `0x81` operand target -> the addresses of the operands
    /// referencing it. Used to annotate data regions; not otherwise
    /// exposed (rebuild from `instructions` if a consumer needs the reverse
    /// mapping some other way).
    string_refs: BTreeMap<u16, Vec<u16>>,
}

impl Listing {
    /// Derives the machine-usable [`Summary`] from this listing.
    pub fn summary(&self) -> Summary {
        let mut opcode_reached_counts = BTreeMap::new();
        for instr in self.instructions.values() {
            *opcode_reached_counts.entry(instr.op.0).or_insert(0) += 1;
        }
        Summary {
            opcode_reached_counts,
            hazards: self.hazards.clone(),
            data_region_spans: self
                .data_regions
                .iter()
                .map(|r| (r.start, r.start + r.len))
                .collect(),
        }
    }

    /// Renders a deterministic textual listing, suitable for golden tests:
    /// addresses always in the same order (the underlying maps are all
    /// `BTreeMap`/sorted `Vec`s), one line per instruction/error/data
    /// region, quarantine sites in their own clearly separated section.
    pub fn render(&self, dialect: &Dialect) -> String {
        enum Item<'a> {
            Instr(u16, &'a Instr),
            Error(u16, &'a DecodeError),
            Data(&'a DataRegion),
        }

        let mut items: Vec<Item> = Vec::new();
        items.extend(self.instructions.iter().map(|(a, i)| Item::Instr(*a, i)));
        items.extend(self.errors.iter().map(|(a, e)| Item::Error(*a, e)));
        items.extend(self.data_regions.iter().map(Item::Data));
        items.sort_by_key(|item| match item {
            Item::Instr(a, _) => *a,
            Item::Error(a, _) => *a,
            Item::Data(r) => r.start,
        });

        let mut out = String::new();
        for item in items {
            match item {
                Item::Instr(addr, instr) => {
                    if let Some(names) = self.labels.get(&addr) {
                        for name in names {
                            out.push_str(&format!("{name}:\n"));
                        }
                    }
                    out.push_str(&format!("{addr:#06X}: {}\n", render_instr(instr, dialect)));
                }
                Item::Error(addr, err) => {
                    out.push_str(&format!("{addr:#06X}: <decode error: {err:?}>\n"));
                }
                Item::Data(region) => {
                    let refs = if region.referenced_by.is_empty() {
                        String::new()
                    } else {
                        format!(
                            ", referenced by [{}]",
                            region
                                .referenced_by
                                .iter()
                                .map(|a| format!("{a:#06X}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    out.push_str(&format!(
                        "{:#06X}..{:#06X}: <data, {} bytes{}>\n",
                        region.start,
                        region.start + region.len,
                        region.len,
                        refs
                    ));
                }
            }
        }

        if !self.quarantine.is_empty() {
            out.push_str("-- quarantine (skip-divergence hazard sites) --\n");
            for (addr, result) in &self.quarantine {
                match result {
                    Ok(instr) => out.push_str(&format!(
                        "{addr:#06X}: {} [quarantined]\n",
                        render_instr(instr, dialect)
                    )),
                    Err(err) => out.push_str(&format!(
                        "{addr:#06X}: <decode error: {err:?}> [quarantined]\n"
                    )),
                }
            }
        }

        out
    }
}

/// Renders one decoded instruction as `NAME arg, arg, ...` (plus a
/// `ConservativeFallthrough` note where relevant). Exposed beyond this
/// module's own [`Listing::render`] for `restrike run-script --trace`
/// (M1 task 3), which disassembles one live instruction at a time rather
/// than a full flow-following listing.
pub fn render_instr(instr: &Instr, dialect: &Dialect) -> String {
    let info = dialect.lookup(instr.op.0);
    let name = info.map(|i| i.name).unwrap_or("<unknown>");
    let args = instr
        .args
        .iter()
        .map(render_arg)
        .collect::<Vec<_>>()
        .join(", ");
    let note = match info.map(|i| i.successor) {
        Some(SuccessorKind::ConservativeFallthrough) => {
            " ; NOTE: conservative fall-through (case is operand-dependent; some cases terminate/chain)"
        }
        _ => "",
    };
    if args.is_empty() {
        format!("{name}{note}")
    } else {
        format!("{name} {args}{note}")
    }
}

fn render_arg(arg: &Arg) -> String {
    match arg {
        Arg::ImmByte(b) => format!("imm={b:#04X}"),
        Arg::Mem(w) => format!("mem={w:#06X}"),
        Arg::MemAlt(w) => format!("mem_alt={w:#06X}"),
        Arg::ImmWord(w) => format!("word={w:#06X}"),
        Arg::InlineStr(bytes) => format!("str_inline(len={})", bytes.len()),
        Arg::MemStr(w) => format!("mem_str={w:#06X}"),
        Arg::UnknownMode { mode, byte } => format!("unknown_mode({mode:#04X}, {byte:#04X})"),
    }
}

fn enqueue(worklist: &mut VecDeque<u16>, queued: &mut BTreeSet<u16>, addr: u16) {
    if queued.insert(addr) {
        worklist.push_back(addr);
    }
}

fn add_target(
    worklist: &mut VecDeque<u16>,
    queued: &mut BTreeSet<u16>,
    labels: &mut BTreeMap<u16, BTreeSet<String>>,
    target: u16,
) {
    labels
        .entry(target)
        .or_default()
        .insert(format!("L{target:04X}"));
    enqueue(worklist, queued, target);
}

/// Computes an IF instruction's skip successor and, if it diverges from the
/// following opcode's real decoded length, quarantines it. `next_addr` is
/// the IF's fall-through address — also where the maybe-skipped instruction
/// starts.
fn handle_branch_skip(
    bytes: &BlockBytes,
    dialect: &Dialect,
    if_addr: u16,
    next_addr: u16,
    listing: &mut Listing,
) {
    let skip_opcode = bytes.get(next_addr);
    let Some(skip_info) = dialect.lookup(skip_opcode) else {
        listing.hazards.push(Hazard::UnknownSkipTargetOpcode {
            if_addr,
            next_addr,
            opcode: skip_opcode,
        });
        return;
    };
    // skip_size is a *batch count* fed into the same mode-byte-driven batch
    // decoder normal decode uses (`vm_LoadCmdSets`), not a byte count — see
    // `skip_batches`'s doc comment and vm-scriptmemory.md §1.
    let skip_target = skip_batches(bytes, next_addr.wrapping_add(1), skip_info.skip_size);

    let normal_next = decode(bytes, next_addr, dialect).ok().map(|i| i.next);
    if normal_next == Some(skip_target) {
        // Skip size matches real operand consumption: the "skip successor"
        // is just the ordinary continuation, already reached by next_addr's
        // own normal decode. Not a hazard.
        return;
    }

    listing.hazards.push(Hazard::SkipDivergence {
        if_addr,
        next_addr,
        skip_target,
    });
    listing
        .quarantine
        .entry(skip_target)
        .or_insert_with(|| decode(bytes, skip_target, dialect));
}

/// Runs a flow-following traversal of `bytes` starting from `entry_points`
/// (the block's header vectors, in caller-supplied order — this module has
/// no opinion on where those come from) plus every statically-known target
/// discovered along the way.
pub fn disassemble(bytes: &BlockBytes, dialect: &Dialect, entry_points: &[u16]) -> Listing {
    let mut listing = Listing::default();
    let mut worklist: VecDeque<u16> = VecDeque::new();
    let mut queued: BTreeSet<u16> = BTreeSet::new();

    for (i, &entry) in entry_points.iter().enumerate() {
        listing
            .labels
            .entry(entry)
            .or_default()
            .insert(format!("entry_{i}"));
        enqueue(&mut worklist, &mut queued, entry);
    }

    while let Some(addr) = worklist.pop_front() {
        if listing.instructions.contains_key(&addr) || listing.errors.contains_key(&addr) {
            continue;
        }

        let instr = match decode(bytes, addr, dialect) {
            Ok(instr) => instr,
            Err(error) => {
                listing.errors.insert(addr, error);
                listing.hazards.push(Hazard::DecodeError { addr, error });
                continue;
            }
        };

        let info = dialect
            .lookup(instr.op.0)
            .expect("decode() only succeeds for an opcode the dialect knows");

        for arg in &instr.args {
            if let Arg::MemStr(target) = arg {
                if in_block(*target) {
                    listing.string_refs.entry(*target).or_default().push(addr);
                }
            }
        }

        let next = instr.next;
        match info.successor {
            SuccessorKind::Sequential | SuccessorKind::ConservativeFallthrough => {
                enqueue(&mut worklist, &mut queued, next);
            }
            SuccessorKind::Terminal => {}
            SuccessorKind::Jump { target_operand } => {
                match instr.args.get(target_operand).and_then(Arg::raw_word) {
                    Some(target) => {
                        add_target(&mut worklist, &mut queued, &mut listing.labels, target)
                    }
                    None => listing.hazards.push(Hazard::UnresolvedJumpTarget {
                        addr,
                        opcode: instr.op.0,
                    }),
                }
            }
            SuccessorKind::Call { target_operand } => {
                enqueue(&mut worklist, &mut queued, next);
                match instr.args.get(target_operand).and_then(Arg::raw_word) {
                    Some(target) => {
                        add_target(&mut worklist, &mut queued, &mut listing.labels, target)
                    }
                    None => listing.hazards.push(Hazard::UnresolvedJumpTarget {
                        addr,
                        opcode: instr.op.0,
                    }),
                }
            }
            SuccessorKind::ComputedTable => {
                enqueue(&mut worklist, &mut queued, next);
                let fixed_prefix = match info.shape {
                    OperandShape::VariableTail { fixed_prefix } => fixed_prefix as usize,
                    OperandShape::Fixed(_) => {
                        unreachable!("ComputedTable opcodes are always VariableTail-shaped")
                    }
                };
                for arg in instr.args.iter().skip(fixed_prefix) {
                    match arg.raw_word() {
                        Some(target) => {
                            add_target(&mut worklist, &mut queued, &mut listing.labels, target)
                        }
                        None => listing.hazards.push(Hazard::UnresolvedJumpTarget {
                            addr,
                            opcode: instr.op.0,
                        }),
                    }
                }
            }
            SuccessorKind::Branch => {
                enqueue(&mut worklist, &mut queued, next);
                handle_branch_skip(bytes, dialect, addr, next, &mut listing);
            }
        }

        listing.instructions.insert(addr, instr);
    }

    listing.data_regions = compute_data_regions(&listing);
    listing
}

fn compute_data_regions(listing: &Listing) -> Vec<DataRegion> {
    let mut covered: BTreeMap<u16, u16> = BTreeMap::new();
    for (&addr, instr) in &listing.instructions {
        covered.insert(addr, instr.next);
    }
    for &addr in listing.errors.keys() {
        // The original wedges on an unknown opcode (D-VM6) — no further
        // bytes are consumed. For disassembly, only the opcode byte itself
        // is "reached"; the rest is data unless some other path covers it.
        covered.entry(addr).or_insert(addr.wrapping_add(1));
    }

    let mut regions = Vec::new();
    let mut cursor = ECL_BLOCK_BASE;
    for (start, end) in covered {
        if start > cursor {
            regions.push(DataRegion {
                start: cursor,
                len: start - cursor,
                referenced_by: Vec::new(),
            });
            cursor = start;
        }
        if end > cursor {
            cursor = end;
        }
    }
    if cursor < BLOCK_END {
        regions.push(DataRegion {
            start: cursor,
            len: BLOCK_END - cursor,
            referenced_by: Vec::new(),
        });
    }

    for region in &mut regions {
        let span = region.start..(region.start + region.len);
        for (&target, refs) in &listing.string_refs {
            if span.contains(&target) {
                region.referenced_by.extend(refs.iter().copied());
            }
        }
        region.referenced_by.sort_unstable();
    }

    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialect::COTAB;
    use crate::test_support::EclBuilder;

    /// All fixtures here are hand-authored via [`EclBuilder`] (D10) — nothing
    /// derived from real game data.

    #[test]
    fn linear_code_all_reached_and_remainder_is_data() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x04).imm_byte(5).mem(0x4B00).imm_word(0x7C10); // ADD, Fixed(3)
        b.label("exit");
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let data_start = b.addr_of("exit") + 1;
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert_eq!(listing.instructions.len(), 2);
        assert!(listing.errors.is_empty());
        assert!(listing.hazards.is_empty());
        assert!(listing.quarantine.is_empty());
        assert_eq!(
            listing.data_regions,
            vec![DataRegion {
                start: data_start,
                len: BLOCK_END - data_start,
                referenced_by: vec![],
            }]
        );
    }

    #[test]
    fn goto_gosub_graph_forward_and_backward_jumps() {
        // entry: GOSUB sub            (Call: targets `sub` AND falls through to ret_site)
        // ret_site: GOTO end          (forward jump)
        // sub: GOTO ret_site          (backward jump — also exercises worklist cycle safety)
        // end: EXIT
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x02).imm_word_label("sub"); // GOSUB sub
        b.label("ret_site");
        b.op(0x01).imm_word_label("end"); // GOTO end
        b.label("sub");
        b.op(0x01).imm_word_label("ret_site"); // GOTO ret_site (backward)
        b.label("end");
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let ret_site = b.addr_of("ret_site");
        let sub = b.addr_of("sub");
        let end = b.addr_of("end");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.is_empty());
        assert!(listing.errors.is_empty());
        assert_eq!(
            listing.instructions.keys().copied().collect::<Vec<_>>(),
            vec![entry, ret_site, sub, end]
        );
        // The GOSUB's fall-through (the return site) must be reachable even
        // though RETURN itself has no static successor.
        assert!(listing.labels.contains_key(&ret_site));
        assert!(listing.labels.contains_key(&sub));
        assert!(listing.labels.contains_key(&end));
    }

    #[test]
    fn if_dual_successor_non_divergent_no_hazard() {
        // IF over a fixed-arity opcode whose skip_size (batch count) matches
        // its real batch count exactly — skip and normal decode must land
        // on the same address, so no hazard/quarantine.
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x16); // IF =
        b.label("add");
        b.op(0x04).imm_byte(1).imm_byte(2).imm_byte(3); // ADD, Fixed(3), skip_size 3
        b.label("exit");
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.is_empty());
        assert!(listing.quarantine.is_empty());
        assert_eq!(listing.instructions.len(), 3);
    }

    #[test]
    fn goto_then_unreachable_code_becomes_data_region() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x01).imm_word_label("target"); // GOTO target (forward, no fall-through)
        b.label("dead");
        b.op(0x00); // never reached: GOTO has no fall-through successor
        b.raw(&[0xAA, 0xBB]);
        b.label("target");
        b.op(0x00); // EXIT, reached only via the GOTO

        let entry = b.addr_of("entry");
        let dead_start = b.addr_of("dead");
        let target = b.addr_of("target");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.is_empty());
        assert_eq!(listing.instructions.len(), 2); // GOTO + target's EXIT
        assert!(listing.data_regions.iter().any(|r| r.start == dead_start
            && r.start + r.len == target
            && r.referenced_by.is_empty()));
    }

    #[test]
    fn variable_tail_menu_with_immediate_count_decodes_normally() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x15) // VERTICAL MENU
            .mem(0x4B00)
            .inline_str(&[])
            .imm_byte(2)
            .inline_str_packed(&[0xAA])
            .inline_str_packed(&[0xBB, 0xCC]);
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.is_empty());
        assert!(listing.quarantine.is_empty());
        assert_eq!(listing.instructions.len(), 2);
        assert_eq!(listing.instructions[&entry].args.len(), 5);
    }

    #[test]
    fn if_before_vertical_menu_quarantines_skip_divergence() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x16); // IF =
        b.label("menu");
        b.op(0x15).mem(0x4B00).inline_str(&[]).imm_byte(0); // VERTICAL MENU, skip_size 0
        b.op(0x00); // EXIT (reached via the menu's normal fall-through)

        let entry = b.addr_of("entry");
        let menu = b.addr_of("menu");
        let skip_target = menu + 1; // 0 batches skipped -> lands right past the opcode byte
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.contains(&Hazard::SkipDivergence {
            if_addr: entry,
            next_addr: menu,
            skip_target,
        }));
        assert!(listing.quarantine.contains_key(&skip_target));
        assert!(!listing.instructions.contains_key(&skip_target));
    }

    #[test]
    fn if_before_ecl_clock_quarantines_fixed_arity_mismatch() {
        // ECL CLOCK (0x34): skip_size 1 batch, but the handler consumes 2 —
        // the confirmed fixed-arity divergence (docs/design/opcode-classification.md).
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x16); // IF =
        b.label("clock");
        b.op(0x34).imm_byte(1).imm_byte(5); // ECL CLOCK, Fixed(2)
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let clock = b.addr_of("clock");
        // skip_batches(1 ImmByte batch) from clock+1 consumes 2 bytes (mode + value).
        let skip_target = clock + 1 + 2;
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.contains(&Hazard::SkipDivergence {
            if_addr: entry,
            next_addr: clock,
            skip_target,
        }));
        assert!(listing.quarantine.contains_key(&skip_target));
    }

    #[test]
    fn unknown_mode_operand_on_sequential_opcode_is_tolerated_without_hazard() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x04).unknown_mode(0x99, 0x42).imm_byte(1).imm_byte(2); // ADD, one unknown-mode operand
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.hazards.is_empty());
        assert_eq!(
            listing.instructions[&entry].args[0],
            Arg::UnknownMode {
                mode: 0x99,
                byte: 0x42
            }
        );
    }

    #[test]
    fn unknown_mode_on_jump_target_is_flagged_unresolved() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x01).unknown_mode(0x99, 0x42); // GOTO with an unresolvable target operand
        b.op(0x00);

        let entry = b.addr_of("entry");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        // decode() tolerates the mode; only the disassembler's static
        // traversal can't follow a target it can't read as a raw word.
        assert_eq!(listing.instructions.len(), 1);
        assert!(listing.hazards.contains(&Hazard::UnresolvedJumpTarget {
            addr: entry,
            opcode: 0x01
        }));
    }

    #[test]
    fn mem_str_operand_references_in_block_data_region() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x11).mem_str_label("greeting"); // PRINT, Fixed(1), mode 0x81
        b.op(0x00); // EXIT
        b.label("greeting");
        b.raw(b"HELLO");

        let entry = b.addr_of("entry");
        let greeting = b.addr_of("greeting");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        let region = listing
            .data_regions
            .iter()
            .find(|r| r.start <= greeting && greeting < r.start + r.len)
            .expect("the string bytes must fall in some data region");
        assert!(region.referenced_by.contains(&entry));
    }

    #[test]
    fn unresolved_variable_tail_count_is_a_decode_error_hazard() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x2B).mem(0x4B00).mem(0x4C00); // HORIZONTAL MENU, count operand memory-moded
        b.op(0x00);

        let entry = b.addr_of("entry");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        assert!(listing.instructions.is_empty());
        assert!(matches!(
            listing.errors.get(&entry),
            Some(DecodeError::UnresolvedVariableTail {
                addr,
                opcode: 0x2B
            }) if *addr == entry
        ));
        assert!(listing
            .hazards
            .iter()
            .any(|h| matches!(h, Hazard::DecodeError { addr, .. } if *addr == entry)));
    }

    #[test]
    fn unknown_opcode_is_a_decode_error_hazard_and_other_entries_still_traversed() {
        let mut b = EclBuilder::new();
        b.label("bad_entry");
        b.op(0x41); // no dialect entry for 0x41
        b.label("good_entry");
        b.op(0x00); // EXIT

        let bad_entry = b.addr_of("bad_entry");
        let good_entry = b.addr_of("good_entry");
        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[bad_entry, good_entry]);

        assert!(matches!(
            listing.errors.get(&bad_entry),
            Some(DecodeError::UnknownOpcode { opcode: 0x41, .. })
        ));
        assert!(listing.instructions.contains_key(&good_entry));
    }

    #[test]
    fn golden_snapshot_full_listing() {
        let mut b = EclBuilder::new();
        b.label("entry");
        b.op(0x16); // IF =
        b.label("menu");
        b.op(0x15).mem(0x4B00).inline_str(&[]).imm_byte(0); // VERTICAL MENU, count 0
        b.label("exit");
        b.op(0x00); // EXIT

        let entry = b.addr_of("entry");
        let menu = b.addr_of("menu");
        let exit = b.addr_of("exit");
        let skip_target = menu + 1;
        let data_start = exit + 1;

        let block = b.build();
        let listing = disassemble(&block, &COTAB, &[entry]);

        let expected = format!(
            "entry_0:\n\
             {entry:#06X}: IF =\n\
             {menu:#06X}: VERTICAL MENU mem=0x4B00, str_inline(len=0), imm=0x00\n\
             {exit:#06X}: EXIT\n\
             {data_start:#06X}..{BLOCK_END:#06X}: <data, {data_len} bytes>\n\
             -- quarantine (skip-divergence hazard sites) --\n\
             {skip_target:#06X}: GOTO imm=0x4B [quarantined]\n",
            data_len = BLOCK_END - data_start,
        );

        assert_eq!(listing.render(&COTAB), expected);
    }
}
