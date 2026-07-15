//! D-OR4 **part A** — the purpose-built 8086 stepper and the H3 acceptance test.
//!
//! This integration test executes the **real binary's own RNG-cluster bytes**
//! (the CotAB v1.3 GOG `START.EXE`, EXEPACK-decompressed at runtime from the
//! user's `$GBX_DATA_DIR`) under a minimal real-mode 8086, and compares the
//! result against `gbx_prng::Prng`. The point is *independent execution*: if
//! the recovery in `docs/design/oracle-rig.md` §1 (the multiplier, the
//! mod-vs-scale semantics, the draw-before-`N==0`-test ordering) is wrong,
//! running the actual bytes catches it — because the stepper is written to
//! **generic x86 semantics**, never "shaped" to the RNG (D-OR4A).
//!
//! ## Provenance / D11 / D10
//!
//! The 8086 is **original work** written to the Intel 8086/80186 ISA (see
//! `SOURCES.md`). It is *not* derived from coab (coab's `seg051.cs` swaps in
//! C# `System.Random` — a refuted hypothesis) and *not* derived from our own
//! `gbx-prng`: the stepper never imports a constant or a line of logic from the
//! crate under test. It reads the multiplier `0x8405` out of *emulated memory*
//! because the `mul` instruction says to, exactly as the real CPU does.
//!
//! **No game bytes are committed** (D10). The opcode bytes are read at runtime
//! from the user's binary; the synthetic opcode tests below are hand-authored
//! programs the ISA defines, containing no game data. Nothing here pastes an
//! image byte, an opcode array, or a fixture.
//!
//! ## Independence rules honored (D-OR4A)
//!
//! - Every opcode is implemented from its ISA definition, in isolation, as a
//!   generic decoder+executor. `div` is "DX:AX ÷ rm16 → AX quotient, DX
//!   remainder, `#DE` on overflow or divide-by-zero" — not "the thing that
//!   makes Random work".
//! - No address special-casing: there is no `if ip == 0x…`, no fast path keyed
//!   to this routine. The stepper decodes and executes; it does not know what
//!   it is running.
//! - The synthetic opcode tests (deliverable 2) validate each opcode against
//!   **hand-computed ISA expectations**, not against the RNG's answer — so a
//!   stepper bug surfaces as a *stepper test failure*, not a fake H3
//!   disagreement.
//!
//! ## What is executed (oracle-rig §1)
//!
//! Two routines in the decompressed image (cs base paragraph `0x8F7`, so
//! `image_offset = 0x8F70 + in_segment_offset`, and loading the image at linear
//! 0 makes `cs:ip` resolve straight to image offsets):
//! - the integer `Random(N)` wrapper, `cs:0x15EA` = image `0xa55a` (far entry,
//!   `retf 2`);
//! - `RandNext`, `cs:0x1639` = image `0xa5a9` (near, `ret`), which reads the
//!   `0x8405` multiplier word via a `CS`-override `mul` at `cs:[0x166F]`.
//!
//! The float path (`0xa570`) and `Randomize` (`0xa5e1`) are never entered.

use sha2::{Digest, Sha256};

// ─────────────────────────────────────────────────────────────────────────────
// A minimal real-mode 8086.
//
// Flat 1 MiB address space with 20-bit wrap; the 8 general registers; the four
// segment registers; IP; and only the flags the executed instructions actually
// define (ZF, CF — see the flag note at `step`). Decode → execute → step.
// Unknown opcode / unknown group extension / `#DE` are all HARD typed errors
// naming the opcode and IP — never a silent no-op or skip.
// ─────────────────────────────────────────────────────────────────────────────

const MEM_MASK: usize = 0xF_FFFF; // 20-bit address wrap (1 MiB real mode)

// General-register indices, in 8086 encoding order.
const AX: usize = 0;
const CX: usize = 1;
const DX: usize = 2;
const BX: usize = 3;
const SP: usize = 4;
const BP: usize = 5;
const SI: usize = 6;
const DI: usize = 7;

/// Every way execution can stop that is not "reached the sentinel". Each names
/// the offending opcode/IP so a decode gap surfaces loudly, never as a wrong
/// answer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CpuError {
    /// A byte the decoder does not implement. Names the opcode and the IP it
    /// was fetched from.
    UnknownOpcode { op: u8, ip: u16 },
    /// A group opcode (F7/D1/D3/83) whose ModR/M `reg` extension is not
    /// implemented. Names the primary opcode, the extension, and the IP.
    UnknownGroup { op: u8, ext: u8, ip: u16 },
    /// `#DE`: divide-by-zero or quotient overflow (quotient > 0xFFFF). The real
    /// 8086 raises INT 0 here; we surface it as a typed error.
    DivideError { ip: u16 },
    /// The step budget was exhausted — a runaway guard, never an expected
    /// outcome. Hard-errors rather than looping forever.
    Runaway { steps: u64 },
}

impl std::fmt::Display for CpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CpuError::UnknownOpcode { op, ip } => {
                write!(f, "unimplemented opcode {op:#04x} at ip={ip:#06x}")
            }
            CpuError::UnknownGroup { op, ext, ip } => {
                write!(
                    f,
                    "unimplemented group opcode {op:#04x} /{ext} at ip={ip:#06x}"
                )
            }
            CpuError::DivideError { ip } => write!(f, "#DE (divide error) at ip={ip:#06x}"),
            CpuError::Runaway { steps } => write!(f, "runaway: exceeded {steps} steps"),
        }
    }
}

/// A resolved ModR/M operand: either a register (by encoding index) or a fully
/// computed linear memory address. Width (8 vs 16) is applied by the caller.
#[derive(Debug, Clone, Copy)]
enum Ea {
    Reg(u8),
    Mem(usize),
}

struct Cpu {
    mem: Vec<u8>,
    r: [u16; 8],
    cs: u16,
    ds: u16,
    ss: u16,
    es: u16,
    ip: u16,
    zf: bool,
    cf: bool,
    /// Segment value chosen by an override prefix for the *current* instruction,
    /// or `None` to use the addressing mode's default segment.
    seg_override: Option<u16>,
    steps: u64,
}

impl Cpu {
    fn new() -> Self {
        Cpu {
            mem: vec![0u8; MEM_MASK + 1],
            r: [0; 8],
            cs: 0,
            ds: 0,
            ss: 0,
            es: 0,
            ip: 0,
            zf: false,
            cf: false,
            seg_override: None,
            steps: 0,
        }
    }

    // ── memory ────────────────────────────────────────────────────────────

    fn lin(seg: u16, off: u16) -> usize {
        (((seg as usize) << 4).wrapping_add(off as usize)) & MEM_MASK
    }

    fn read8(&self, lin: usize) -> u8 {
        self.mem[lin & MEM_MASK]
    }

    fn read16(&self, lin: usize) -> u16 {
        let lo = self.mem[lin & MEM_MASK] as u16;
        let hi = self.mem[(lin + 1) & MEM_MASK] as u16;
        lo | (hi << 8)
    }

    fn write16(&mut self, lin: usize, v: u16) {
        self.mem[lin & MEM_MASK] = (v & 0xff) as u8;
        self.mem[(lin + 1) & MEM_MASK] = (v >> 8) as u8;
    }

    fn read32(&self, seg: u16, off: u16) -> u32 {
        let lo = self.read16(Self::lin(seg, off)) as u32;
        let hi = self.read16(Self::lin(seg, off.wrapping_add(2))) as u32;
        lo | (hi << 16)
    }

    fn write32(&mut self, seg: u16, off: u16, v: u32) {
        self.write16(Self::lin(seg, off), (v & 0xffff) as u16);
        self.write16(Self::lin(seg, off.wrapping_add(2)), (v >> 16) as u16);
    }

    // ── instruction-stream fetch ──────────────────────────────────────────

    fn fetch8(&mut self) -> u8 {
        let b = self.read8(Self::lin(self.cs, self.ip));
        self.ip = self.ip.wrapping_add(1);
        b
    }

    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch8() as u16;
        let hi = self.fetch8() as u16;
        lo | (hi << 8)
    }

    // ── 8-bit register file (AL/CL/DL/BL/AH/CH/DH/BH by encoding index) ────

    fn get_r8(&self, i: u8) -> u8 {
        let reg = self.r[(i & 3) as usize];
        if i < 4 {
            (reg & 0xff) as u8
        } else {
            (reg >> 8) as u8
        }
    }

    fn set_r8(&mut self, i: u8, v: u8) {
        let idx = (i & 3) as usize;
        if i < 4 {
            self.r[idx] = (self.r[idx] & 0xff00) | v as u16;
        } else {
            self.r[idx] = (self.r[idx] & 0x00ff) | ((v as u16) << 8);
        }
    }

    // ── ModR/M decode (a real decoder, not a byte pattern-matcher) ─────────

    /// Fetches the ModR/M byte and returns `(mod, reg, rm)`.
    fn modrm(&mut self) -> (u8, u8, u8) {
        let b = self.fetch8();
        (b >> 6, (b >> 3) & 7, b & 7)
    }

    /// Resolves the `rm` operand to a register or a linear address, consuming
    /// any displacement bytes. Implements the full 16-bit effective-address
    /// table, including the `mod=00 rm=110` disp16-direct special case and the
    /// BP-implies-SS default-segment rule. The active segment-override prefix,
    /// if any, replaces the default segment.
    fn decode_rm(&mut self, md: u8, rm: u8) -> Ea {
        if md == 3 {
            return Ea::Reg(rm);
        }
        // The disp16-direct form: no base/index, absolute offset, default DS.
        if md == 0 && rm == 6 {
            let off = self.fetch16();
            let seg = self.seg_override.unwrap_or(self.ds);
            return Ea::Mem(Self::lin(seg, off));
        }
        let (base, default_ss) = match rm {
            0 => (self.r[BX].wrapping_add(self.r[SI]), false),
            1 => (self.r[BX].wrapping_add(self.r[DI]), false),
            2 => (self.r[BP].wrapping_add(self.r[SI]), true),
            3 => (self.r[BP].wrapping_add(self.r[DI]), true),
            4 => (self.r[SI], false),
            5 => (self.r[DI], false),
            6 => (self.r[BP], true), // md != 0 here (md==0 handled above)
            7 => (self.r[BX], false),
            _ => unreachable!("rm is 3 bits"),
        };
        let disp = match md {
            0 => 0u16,
            1 => self.fetch8() as i8 as i16 as u16, // sign-extended disp8
            2 => self.fetch16(),
            _ => unreachable!("md handled above"),
        };
        let off = base.wrapping_add(disp);
        let default_seg = if default_ss { self.ss } else { self.ds };
        let seg = self.seg_override.unwrap_or(default_seg);
        Ea::Mem(Self::lin(seg, off))
    }

    fn read_rm16(&self, ea: Ea) -> u16 {
        match ea {
            Ea::Reg(i) => self.r[i as usize],
            Ea::Mem(l) => self.read16(l),
        }
    }

    fn write_rm16(&mut self, ea: Ea, v: u16) {
        match ea {
            Ea::Reg(i) => self.r[i as usize] = v,
            Ea::Mem(l) => self.write16(l, v),
        }
    }

    fn read_rm8(&self, ea: Ea) -> u8 {
        match ea {
            Ea::Reg(i) => self.get_r8(i),
            Ea::Mem(l) => self.read8(l),
        }
    }

    // ── stack ─────────────────────────────────────────────────────────────

    fn push16(&mut self, v: u16) {
        self.r[SP] = self.r[SP].wrapping_sub(2);
        self.write16(Self::lin(self.ss, self.r[SP]), v);
    }

    fn pop16(&mut self) -> u16 {
        let v = self.read16(Self::lin(self.ss, self.r[SP]));
        self.r[SP] = self.r[SP].wrapping_add(2);
        v
    }

    // ── one instruction ───────────────────────────────────────────────────
    //
    // Flags note: only ZF and CF are modeled, because only they are *read* by
    // the executed routines — `jz` reads ZF (set by the `or bx,bx` guard and
    // the `xor ax,ax`), and `adc dx,0` reads CF (set by the immediately
    // preceding `add ax,1`). SF/OF/AF/PF are deliberately omitted: nothing in
    // the two routines reads them, and implementing flags that are never read
    // would be inventing untested behavior. Each opcode below sets ZF/CF
    // exactly where the ISA defines them for that opcode, generically — not
    // tuned to this routine.

    fn step(&mut self) -> Result<(), CpuError> {
        self.steps += 1;
        if self.steps > BUDGET {
            return Err(CpuError::Runaway { steps: BUDGET });
        }
        self.seg_override = None;

        // Consume segment-override prefixes (generic: all four segments).
        let mut opcode = self.fetch8();
        loop {
            let seg = match opcode {
                0x26 => self.es,
                0x2e => self.cs,
                0x36 => self.ss,
                0x3e => self.ds,
                _ => break,
            };
            self.seg_override = Some(seg);
            opcode = self.fetch8();
        }

        // IP of this opcode's first byte (post-prefix) for error reporting.
        let op_ip = self.ip.wrapping_sub(1);

        match opcode {
            // CALL rel16 (near): push IP-of-next, then IP += rel16.
            0xe8 => {
                let rel = self.fetch16();
                let ret = self.ip;
                self.push16(ret);
                self.ip = self.ip.wrapping_add(rel);
            }

            // RET (near): pop IP.
            0xc3 => {
                self.ip = self.pop16();
            }

            // RETF imm16 (far): pop IP, pop CS, then SP += imm16 (caller-arg
            // cleanup — here the `2` that removes the pushed N).
            0xca => {
                let imm = self.fetch16();
                self.ip = self.pop16();
                self.cs = self.pop16();
                self.r[SP] = self.r[SP].wrapping_add(imm);
            }

            // XOR r16, rm16  (33 /r): reg is destination. Clears CF, sets ZF.
            0x33 => {
                let (md, reg, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                let res = self.r[reg as usize] ^ self.read_rm16(ea);
                self.r[reg as usize] = res;
                self.cf = false;
                self.zf = res == 0;
            }

            // OR r16, rm16  (0B /r): reg is destination. Clears CF, sets ZF.
            0x0b => {
                let (md, reg, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                let res = self.r[reg as usize] | self.read_rm16(ea);
                self.r[reg as usize] = res;
                self.cf = false;
                self.zf = res == 0;
            }

            // JZ rel8 (74 cb): branch if ZF.
            0x74 => {
                let rel = self.fetch8() as i8 as i16 as u16;
                if self.zf {
                    self.ip = self.ip.wrapping_add(rel);
                }
            }

            // XCHG AX, r16 (90+r): swap AX with the encoded register.
            0x90..=0x97 => {
                let r = (opcode & 7) as usize;
                self.r.swap(AX, r);
            }

            // MOV r16, rm16 (8B /r): reg is destination.
            0x8b => {
                let (md, reg, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                self.r[reg as usize] = self.read_rm16(ea);
            }

            // MOV rm16, r16 (89 /r): rm is destination.
            0x89 => {
                let (md, reg, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                self.write_rm16(ea, self.r[reg as usize]);
            }

            // MOV AX, moffs16 (A1): load AX from [seg:disp16] (default DS).
            0xa1 => {
                let off = self.fetch16();
                let seg = self.seg_override.unwrap_or(self.ds);
                self.r[AX] = self.read16(Self::lin(seg, off));
            }

            // MOV moffs16, AX (A3): store AX to [seg:disp16] (default DS).
            0xa3 => {
                let off = self.fetch16();
                let seg = self.seg_override.unwrap_or(self.ds);
                let ax = self.r[AX];
                self.write16(Self::lin(seg, off), ax);
            }

            // MOV r8, imm8 (B0+r).
            0xb0..=0xb7 => {
                let r = opcode & 7;
                let imm = self.fetch8();
                self.set_r8(r, imm);
            }

            // Group F7 /ext, rm16: /4 MUL, /6 DIV (others unimplemented).
            0xf7 => {
                let (md, ext, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                let src = self.read_rm16(ea);
                match ext {
                    // MUL rm16: DX:AX = AX * rm16 (unsigned). CF=OF=(DX!=0).
                    4 => {
                        let prod = (self.r[AX] as u32) * (src as u32);
                        self.r[AX] = (prod & 0xffff) as u16;
                        self.r[DX] = (prod >> 16) as u16;
                        self.cf = self.r[DX] != 0;
                        // ZF is undefined after MUL on the 8086; left unchanged.
                    }
                    // DIV rm16: AX = (DX:AX)/rm16, DX = (DX:AX)%rm16.
                    // #DE on divide-by-zero or if the quotient exceeds 16 bits.
                    6 => {
                        if src == 0 {
                            return Err(CpuError::DivideError { ip: op_ip });
                        }
                        let dividend = ((self.r[DX] as u32) << 16) | (self.r[AX] as u32);
                        let q = dividend / (src as u32);
                        if q > 0xffff {
                            return Err(CpuError::DivideError { ip: op_ip });
                        }
                        self.r[AX] = q as u16;
                        self.r[DX] = (dividend % (src as u32)) as u16;
                        // Flags undefined after DIV; left unchanged.
                    }
                    _ => {
                        return Err(CpuError::UnknownGroup {
                            op: opcode,
                            ext,
                            ip: op_ip,
                        })
                    }
                }
            }

            // Group D1 /ext, rm16, 1: /4 SHL by 1 (others unimplemented).
            0xd1 => {
                let (md, ext, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                match ext {
                    4 => {
                        let v = self.read_rm16(ea);
                        self.cf = (v & 0x8000) != 0;
                        let res = v << 1;
                        self.write_rm16(ea, res);
                        self.zf = res == 0;
                    }
                    _ => {
                        return Err(CpuError::UnknownGroup {
                            op: opcode,
                            ext,
                            ip: op_ip,
                        })
                    }
                }
            }

            // Group D3 /ext, rm16, CL: /4 SHL by CL (others unimplemented).
            0xd3 => {
                let (md, ext, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                match ext {
                    4 => {
                        let v = self.read_rm16(ea);
                        let count = self.get_r8(CX as u8) as u32; // CL
                        let (res, cf) = shl16(v, count);
                        // A shift count of 0 leaves flags unchanged (8086).
                        if count != 0 {
                            self.cf = cf;
                            self.zf = res == 0;
                        }
                        self.write_rm16(ea, res);
                    }
                    _ => {
                        return Err(CpuError::UnknownGroup {
                            op: opcode,
                            ext,
                            ip: op_ip,
                        })
                    }
                }
            }

            // ADD r8, rm8 (02 /r): reg is destination (byte width).
            0x02 => {
                let (md, reg, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                let a = self.get_r8(reg);
                let b = self.read_rm8(ea);
                let sum = a as u16 + b as u16;
                let res = sum as u8;
                self.set_r8(reg, res);
                self.cf = sum > 0xff;
                self.zf = res == 0;
            }

            // ADD r16, rm16 (03 /r): reg is destination.
            0x03 => {
                let (md, reg, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                let a = self.r[reg as usize];
                let b = self.read_rm16(ea);
                let sum = a as u32 + b as u32;
                let res = sum as u16;
                self.r[reg as usize] = res;
                self.cf = sum > 0xffff;
                self.zf = res == 0;
            }

            // ADD AX, imm16 (05 iw).
            0x05 => {
                let imm = self.fetch16();
                let sum = self.r[AX] as u32 + imm as u32;
                let res = sum as u16;
                self.r[AX] = res;
                self.cf = sum > 0xffff;
                self.zf = res == 0;
            }

            // Group 83 /ext, rm16, imm8 (sign-extended): /2 ADC (others
            // unimplemented — this group also holds ADD/SUB/AND/... but only
            // ADC is executed, and inventing the rest would be untested).
            0x83 => {
                let (md, ext, rm) = self.modrm();
                let ea = self.decode_rm(md, rm);
                let imm = self.fetch8() as i8 as i16 as u16; // sign-extended
                match ext {
                    // ADC rm16, imm8-sx: rm = rm + imm + CF.
                    2 => {
                        let a = self.read_rm16(ea);
                        let sum = a as u32 + imm as u32 + (self.cf as u32);
                        let res = sum as u16;
                        self.write_rm16(ea, res);
                        self.cf = sum > 0xffff;
                        self.zf = res == 0;
                    }
                    _ => {
                        return Err(CpuError::UnknownGroup {
                            op: opcode,
                            ext,
                            ip: op_ip,
                        })
                    }
                }
            }

            _ => {
                return Err(CpuError::UnknownOpcode {
                    op: opcode,
                    ip: op_ip,
                })
            }
        }
        Ok(())
    }

    /// Runs until `cs:ip` reaches the sentinel, or errors (unknown opcode,
    /// `#DE`, or runaway). Never loops forever — the step budget is the guard.
    fn run_until(&mut self, sentinel_cs: u16, sentinel_ip: u16) -> Result<(), CpuError> {
        while !(self.cs == sentinel_cs && self.ip == sentinel_ip) {
            self.step()?;
        }
        Ok(())
    }
}

/// Generic 16-bit logical left shift returning `(result, carry_out)`. `carry`
/// is the last bit shifted out (bit `16-count` of the original); a count that
/// shifts every bit out yields 0 with carry 0.
fn shl16(v: u16, count: u32) -> (u16, bool) {
    if count == 0 {
        return (v, false);
    }
    if count > 16 {
        return (0, false);
    }
    let wide = (v as u32) << count;
    let res = (wide & 0xffff) as u16;
    let cf = (v >> (16 - count)) & 1 == 1;
    (res, cf)
}

/// A generous per-call step budget. The two routines are ~30 instructions; this
/// is the runaway backstop, never reached in a correct run.
const BUDGET: u64 = 100_000;

// The far-call return sentinel. `cs` is not `0x8F7` (the image's code segment),
// so `cs:ip` can never collide with any instruction the routines execute — the
// run loop halts here the instant the wrapper's `retf 2` pops it.
const SENTINEL_CS: u16 = 0xF000;
const SENTINEL_IP: u16 = 0xFFF0;

// Non-overlapping segment bases for the hermetic run:
//   image  : linear 0 .. 0xf3e0
//   DS 0x2000 → DS:0x47F0 = linear 0x247F0  (well clear of image and stack)
//   SS 0x3000 → stack around linear 0x30F00 (clear of image and DS)
const CS_BASE: u16 = 0x08F7; // cs<<4 = 0x8F70, so cs:ip = image offset
const DS_BASE: u16 = 0x2000;
const SS_BASE: u16 = 0x3000;
const STATE_OFF: u16 = 0x47F0; // DS:0x47F0 — the live LCG state dword
const WRAPPER_IP: u16 = 0x15EA; // cs:0x15EA = image 0xa55a

// ─────────────────────────────────────────────────────────────────────────────
// Running the real wrapper.
// ─────────────────────────────────────────────────────────────────────────────

/// A stepper wired to run the real RNG cluster: the image is preloaded once,
/// and each call resets the machine and executes the integer wrapper.
struct Rig {
    cpu: Cpu,
    image_len: usize,
}

impl Rig {
    fn new(image: &[u8]) -> Self {
        let mut cpu = Cpu::new();
        cpu.mem[..image.len()].copy_from_slice(image);
        Rig {
            cpu,
            image_len: image.len(),
        }
    }

    /// Seeds `DS:0x47F0 = k`, builds the far-call frame for `Random(n)`, runs
    /// the wrapper, and returns `(ax, state')` — the returned value and the new
    /// state dword read back out of emulated memory. Also asserts frame
    /// integrity (SP restored). Errors propagate as typed `CpuError`s.
    ///
    /// Calling convention (Turbo Pascal, far): the caller pushes the argument,
    /// then the far-call pushes return CS:IP; the callee cleans the argument via
    /// `retf 2`. We build that frame by hand:
    /// ```text
    /// push N            ; the word argument
    /// push sentinel_cs  ; far-call return segment
    /// push sentinel_ip  ; far-call return offset
    /// ; then cs:ip = 0x8F7:0x15EA and run to the sentinel
    /// ```
    /// At wrapper entry that gives `ss:[sp+0]=ret_ip`, `ss:[sp+2]=ret_cs`,
    /// `ss:[sp+4]=N`, which is exactly what the wrapper's `ss:[bx+4]` reads.
    fn call_wrapper(&mut self, k: u32, n: u16) -> Result<(u16, u32), CpuError> {
        let cpu = &mut self.cpu;

        // Reset registers/flags/IP (memory image persists; the state dword and
        // stack words are overwritten below, so no cross-call contamination).
        cpu.r = [0; 8];
        cpu.zf = false;
        cpu.cf = false;
        cpu.seg_override = None;
        cpu.steps = 0;
        cpu.cs = CS_BASE;
        cpu.ds = DS_BASE;
        cpu.ss = SS_BASE;
        cpu.es = 0;

        // Seed the live LCG state (little-endian dword) at DS:0x47F0.
        cpu.write32(DS_BASE, STATE_OFF, k);

        // Build the far-call frame.
        cpu.r[SP] = 0x1000;
        let sp_start = cpu.r[SP]; // SP before any push
        cpu.push16(n);
        cpu.push16(SENTINEL_CS);
        cpu.push16(SENTINEL_IP);

        // Enter the wrapper and run to the sentinel.
        cpu.ip = WRAPPER_IP;
        cpu.run_until(SENTINEL_CS, SENTINEL_IP)?;

        // Frame integrity: three pushes (-6) then `retf 2` (pop ip+cs = +4,
        // then +2 arg cleanup = +6) nets zero, so SP must be back to sp_start.
        // Equivalently, SP is the post-`push N` SP plus 2 — the retf-2 cleanup.
        assert_eq!(
            cpu.r[SP], sp_start,
            "frame integrity: SP not restored after retf 2 (k={k:#010x}, n={n})"
        );

        let ax = cpu.r[AX];
        let state2 = cpu.read32(DS_BASE, STATE_OFF);
        Ok((ax, state2))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Loading the user's binary + the pin assertion.
// ─────────────────────────────────────────────────────────────────────────────

/// The `[0xa55a, 0xa5ee)` verification pin (oracle-rig §1); identical to the
/// hash the step-1 `gbx_prng::pin` test asserts. Re-checked here **before**
/// stepping so a changed binary fails on the pin, not on a confusing execution
/// diff.
const PIN_SHA256: &str = "0f770ce01cc999eb8ca75406d57de94ffd7c01e7438c0647395b26a668bea68b";

/// Loads + EXEPACK-decodes the user's `START.EXE`, or `None` (with a loud
/// `SKIPPED:` line) when `GBX_DATA_DIR` is absent. Local-tier gate — mirrors
/// `gbx_prng::pin`.
fn load_image(test_name: &str) -> Option<Vec<u8>> {
    let dir = std::env::var_os("GBX_DATA_DIR")?;
    let path = std::path::Path::new(&dir).join("START.EXE");
    let packed = std::fs::read(&path).expect("GBX_DATA_DIR/START.EXE must be readable");
    let image =
        gbx_formats::exepack::decode(&packed).expect("real START.EXE must EXEPACK-decode cleanly");

    // Assert the pin before anyone steps a byte.
    let cluster = image
        .get(0xa55a..0xa5ee)
        .expect("decompressed image must contain the RNG cluster range [0xa55a,0xa5ee)");
    let hash: String = Sha256::digest(cluster)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hash, PIN_SHA256,
        "RNG-cluster pin MISMATCH before stepping ({test_name}): the binary at GBX_DATA_DIR is \
         not the image the stepper and gbx-prng were derived from. Re-derive and re-pin per \
         oracle-rig §1; do NOT update this hash blindly."
    );
    Some(image)
}

// ═════════════════════════════════════════════════════════════════════════════
// DELIVERABLE 2 — synthetic opcode tests: the stepper's own correctness, proven
// WITHOUT game data. Each opcode is checked against hand-computed ISA
// expectations, so a stepper bug surfaces here, not as a fake H3 disagreement.
//
// These build tiny hand-authored programs (D10-clean — no game bytes) at a
// chosen cs:ip, run a fixed number of instructions, and assert register/flag
// state the ISA defines.
// ═════════════════════════════════════════════════════════════════════════════

/// Loads a program at `cs:ip = 0x100:0x0000`, primed for `count`-instruction
/// runs. Returns the Cpu ready to `step()`.
fn synth(prog: &[u8]) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.cs = 0x100;
    cpu.ip = 0x0000;
    cpu.ds = 0x200; // for data-touching tests
    cpu.ss = 0x300;
    let base = Cpu::lin(cpu.cs, 0);
    cpu.mem[base..base + prog.len()].copy_from_slice(prog);
    cpu
}

fn run_n(cpu: &mut Cpu, count: usize) {
    for _ in 0..count {
        cpu.step().expect("synthetic program must not error");
    }
}

#[test]
fn mul_low_and_high_word() {
    // MUL BX (F7 E3): DX:AX = AX * BX (unsigned).
    // Case 1: 0x1234 * 0x8405 = 0x0963_2B04 → DX=0x0963, AX=0x2B04, CF=1.
    let mut cpu = synth(&[0xf7, 0xe3]);
    cpu.r[AX] = 0x1234;
    cpu.r[BX] = 0x8405;
    run_n(&mut cpu, 1);
    let expected = 0x1234u32 * 0x8405u32;
    assert_eq!(cpu.r[AX], (expected & 0xffff) as u16);
    assert_eq!(cpu.r[DX], (expected >> 16) as u16);
    assert_eq!(cpu.r[DX], 0x0963);
    assert_eq!(cpu.r[AX], 0x2b04);
    assert!(cpu.cf, "CF set because the high word (DX) is nonzero");

    // Case 2: high word nonzero at the extreme. 0xFFFF*0xFFFF = 0xFFFE_0001.
    let mut cpu = synth(&[0xf7, 0xe3]);
    cpu.r[AX] = 0xffff;
    cpu.r[BX] = 0xffff;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[DX], 0xfffe);
    assert_eq!(cpu.r[AX], 0x0001);
    assert!(cpu.cf);

    // Case 3: product fits in AX → DX=0, CF=0.
    let mut cpu = synth(&[0xf7, 0xe3]);
    cpu.r[AX] = 0x0010;
    cpu.r[BX] = 0x0010;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[DX], 0x0000);
    assert_eq!(cpu.r[AX], 0x0100);
    assert!(!cpu.cf);
}

#[test]
fn div_quotient_remainder_split() {
    // DIV BX (F7 F3): AX = (DX:AX)/BX, DX = (DX:AX)%BX.
    // DX:AX = 0x0001_0007 = 65543, / 3 → q=21847 (0x5557), rem=2.
    let mut cpu = synth(&[0xf7, 0xf3]);
    cpu.r[DX] = 0x0001;
    cpu.r[AX] = 0x0007;
    cpu.r[BX] = 3;
    run_n(&mut cpu, 1);
    let dividend = 0x0001_0007u32;
    assert_eq!(cpu.r[AX], (dividend / 3) as u16);
    assert_eq!(cpu.r[DX], (dividend % 3) as u16);
    assert_eq!(cpu.r[AX], 0x5557);
    assert_eq!(cpu.r[DX], 2);

    // A clean split with DX=0 (the wrapper's actual shape): 0x0000_8405 / 6.
    let mut cpu = synth(&[0xf7, 0xf3]);
    cpu.r[DX] = 0x0000;
    cpu.r[AX] = 0x8405;
    cpu.r[BX] = 6;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[AX], 0x8405 / 6);
    assert_eq!(cpu.r[DX], 0x8405 % 6);
}

#[test]
fn div_overflow_is_a_typed_error() {
    // DX:AX = 0xFFFF_0000, / 1 → quotient 0xFFFF0000 > 0xFFFF → #DE.
    let mut cpu = synth(&[0xf7, 0xf3]);
    cpu.r[DX] = 0xffff;
    cpu.r[AX] = 0x0000;
    cpu.r[BX] = 1;
    let err = cpu.step().unwrap_err();
    assert!(matches!(err, CpuError::DivideError { .. }), "got {err:?}");
}

#[test]
fn div_by_zero_is_a_typed_error() {
    let mut cpu = synth(&[0xf7, 0xf3]); // DIV BX with BX=0
    cpu.r[DX] = 0x0000;
    cpu.r[AX] = 0x1234;
    cpu.r[BX] = 0;
    let err = cpu.step().unwrap_err();
    assert!(matches!(err, CpuError::DivideError { .. }), "got {err:?}");
}

#[test]
fn shl_by_one() {
    // SHL BX,1 (D1 E3). 0x4001 << 1 = 0x8002, CF = old bit15 = 0.
    let mut cpu = synth(&[0xd1, 0xe3]);
    cpu.r[BX] = 0x4001;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[BX], 0x8002);
    assert!(!cpu.cf);
    assert!(!cpu.zf);

    // 0x8000 << 1 = 0x0000, CF = old bit15 = 1, ZF = 1.
    let mut cpu = synth(&[0xd1, 0xe3]);
    cpu.r[BX] = 0x8000;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[BX], 0x0000);
    assert!(cpu.cf);
    assert!(cpu.zf);
}

#[test]
fn shl_by_cl() {
    // SHL BX,CL (D3 E3). 0x0001 << 5 = 0x0020.
    let mut cpu = synth(&[0xd3, 0xe3]);
    cpu.r[BX] = 0x0001;
    cpu.set_r8(CX as u8, 5); // CL = 5
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[BX], 0x0020);

    // 0x0FF0 << 5 = 0xFE00 (bits above 16 dropped); CF = bit (16-5)=bit11 = 1.
    let mut cpu = synth(&[0xd3, 0xe3]);
    cpu.r[BX] = 0x0ff0;
    cpu.set_r8(CX as u8, 5);
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[BX], ((0x0ff0u32 << 5) & 0xffff) as u16);
    assert_eq!(cpu.r[BX], 0xfe00);
    assert!(cpu.cf, "bit 11 of 0x0ff0 is 1, shifted out last");
}

#[test]
fn adc_reads_carry_in_both_states() {
    // ADC DX, 0 (83 D2 00) with CF clear: DX unchanged.
    let mut cpu = synth(&[0x83, 0xd2, 0x00]);
    cpu.r[DX] = 0x1234;
    cpu.cf = false;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[DX], 0x1234);

    // ADC DX, 0 with CF set: DX += 1.
    let mut cpu = synth(&[0x83, 0xd2, 0x00]);
    cpu.r[DX] = 0x1234;
    cpu.cf = true;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[DX], 0x1235);

    // ADC with sign-extended negative imm8 (0xFF = -1) and CF set: net +0.
    let mut cpu = synth(&[0x83, 0xd2, 0xff]);
    cpu.r[DX] = 0x1000;
    cpu.cf = true;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[DX], 0x1000, "0x1000 + (-1) + 1 = 0x1000");
    assert!(cpu.cf, "the add wrapped past 16 bits → CF set");
}

#[test]
fn add_byte_touches_only_the_target_byte() {
    // ADD CH, CL (02 E9) — the RandNext idiom. Must NOT disturb CL.
    // CX = 0x12FF (CH=0x12, CL=0xFF). CH += CL = 0x11 (0x111 & 0xFF), CF=1.
    let mut cpu = synth(&[0x02, 0xe9]);
    cpu.r[CX] = 0x12ff;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[CX], 0x11ff, "only CH changed; CL stayed 0xFF");
    assert!(cpu.cf, "0x12 + 0xFF = 0x111 → byte carry");

    // Contrast: word ADD CX, ... changes the whole register.
    // ADD DX, CX (03 D1): DX = 0x00FF + 0x0001 = 0x0100 (word add).
    let mut cpu = synth(&[0x03, 0xd1]);
    cpu.r[DX] = 0x00ff;
    cpu.r[CX] = 0x0001;
    run_n(&mut cpu, 1);
    assert_eq!(
        cpu.r[DX], 0x0100,
        "word add carries across the byte boundary"
    );
    assert!(!cpu.cf);
}

#[test]
fn add_dh_bl_byte_add_into_high_byte() {
    // ADD DH, BL (02 F3): high byte of DX gets BL added; DL untouched.
    let mut cpu = synth(&[0x02, 0xf3]);
    cpu.r[DX] = 0x00aa; // DH=0x00, DL=0xAA
    cpu.r[BX] = 0x0080; // BL=0x80
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[DX], 0x80aa, "DH=0x80, DL preserved");
    assert!(!cpu.cf);
}

#[test]
fn xchg_ax_dx() {
    // XCHG AX, DX (92).
    let mut cpu = synth(&[0x92]);
    cpu.r[AX] = 0x1111;
    cpu.r[DX] = 0x2222;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[AX], 0x2222);
    assert_eq!(cpu.r[DX], 0x1111);
}

#[test]
fn xor_ax_ax_zeroes_and_sets_zf() {
    // XOR AX, AX (33 C0).
    let mut cpu = synth(&[0x33, 0xc0]);
    cpu.r[AX] = 0xdead;
    cpu.cf = true;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[AX], 0);
    assert!(cpu.zf, "result zero → ZF");
    assert!(!cpu.cf, "logical op clears CF");
}

#[test]
fn or_sets_zf_read_by_jz() {
    // OR BX, BX (0B DB) then JZ +4 (74 04). BX=0 → ZF → branch taken.
    let prog = &[0x0b, 0xdb, 0x74, 0x04];
    let mut cpu = synth(prog);
    cpu.r[BX] = 0;
    run_n(&mut cpu, 1); // OR
    assert!(cpu.zf);
    let ip_before = cpu.ip;
    run_n(&mut cpu, 1); // JZ +4
    assert_eq!(cpu.ip, ip_before.wrapping_add(2 + 4), "JZ taken");

    // BX nonzero → no branch.
    let mut cpu = synth(prog);
    cpu.r[BX] = 0x0001;
    run_n(&mut cpu, 1);
    assert!(!cpu.zf);
    let ip_before = cpu.ip;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.ip, ip_before.wrapping_add(2), "JZ not taken");
}

#[test]
fn segment_override_selects_the_right_segment() {
    // Same offset 0x0050 holds different bytes in DS vs CS. A1 (MOV AX,moffs)
    // defaults DS; 2E-prefixed reads CS.
    // Program at CS:0 = [2E A1 50 00]  (MOV AX, CS:[0x0050]).
    let mut cpu = synth(&[0x2e, 0xa1, 0x50, 0x00]);
    // Put 0xBEEF at DS:0x0050 and 0xCAFE at CS:0x0050.
    cpu.write16(Cpu::lin(cpu.ds, 0x0050), 0xbeef);
    cpu.write16(Cpu::lin(cpu.cs, 0x0050), 0xcafe);
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[AX], 0xcafe, "CS override read CS, not DS");

    // Without the prefix, the same instruction reads DS.
    let mut cpu = synth(&[0xa1, 0x50, 0x00]);
    cpu.write16(Cpu::lin(cpu.ds, 0x0050), 0xbeef);
    cpu.write16(Cpu::lin(cpu.cs, 0x0050), 0xcafe);
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[AX], 0xbeef, "no prefix → default DS");
}

#[test]
fn ss_override_on_modrm_memory() {
    // 36 8B 5F 04 = MOV BX, SS:[BX+4] — the wrapper's argument fetch.
    let mut cpu = synth(&[0x36, 0x8b, 0x5f, 0x04]);
    cpu.r[BX] = 0x0100;
    // SS:[0x0104] = 0x0123.
    cpu.write16(Cpu::lin(cpu.ss, 0x0104), 0x0123);
    // A decoy at DS:[0x0104] to prove the override matters.
    cpu.write16(Cpu::lin(cpu.ds, 0x0104), 0xffff);
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[BX], 0x0123, "read SS:[BX+4], not DS");
}

#[test]
fn modrm_disp16_direct_form() {
    // 8B 1E F2 47 = MOV BX, [0x47F2] (mod=00 rm=110 → disp16-direct, default DS).
    let mut cpu = synth(&[0x8b, 0x1e, 0xf2, 0x47]);
    cpu.write16(Cpu::lin(cpu.ds, 0x47f2), 0xabcd);
    run_n(&mut cpu, 1);
    assert_eq!(cpu.r[BX], 0xabcd);

    // And the store form 89 16 F2 47 = MOV [0x47F2], DX.
    let mut cpu = synth(&[0x89, 0x16, 0xf2, 0x47]);
    cpu.r[DX] = 0x9876;
    run_n(&mut cpu, 1);
    assert_eq!(cpu.read16(Cpu::lin(cpu.ds, 0x47f2)), 0x9876);
}

#[test]
fn near_call_and_ret_stack_effects() {
    // At CS:0 : E8 03 00  (CALL +3 → target CS:0x0006), then filler.
    // At CS:6 : C3        (RET).
    // Program bytes: [E8 03 00, 90, 90, 90, C3]  (three NOPs as filler at 3..6)
    let mut cpu = synth(&[0xe8, 0x03, 0x00, 0x90, 0x90, 0x90, 0xc3]);
    cpu.r[SP] = 0x0800;
    let sp0 = cpu.r[SP];
    run_n(&mut cpu, 1); // CALL
    assert_eq!(cpu.ip, 0x0006, "CALL jumped to +3 past the 3-byte insn");
    assert_eq!(cpu.r[SP], sp0.wrapping_sub(2), "CALL pushed return IP");
    assert_eq!(
        cpu.read16(Cpu::lin(cpu.ss, cpu.r[SP])),
        0x0003,
        "return IP is the insn after CALL"
    );
    run_n(&mut cpu, 1); // RET
    assert_eq!(cpu.ip, 0x0003, "RET returned to after the CALL");
    assert_eq!(cpu.r[SP], sp0, "RET restored SP");
}

#[test]
fn far_retf_imm_stack_effects() {
    // RETF 2 (CA 02 00): pop IP, pop CS, then SP += 2.
    let mut cpu = synth(&[0xca, 0x02, 0x00]);
    cpu.r[SP] = 0x0800;
    // Stack layout (top → up): IP=0x1111, CS=0x2222, ARG=0x3333.
    cpu.write16(Cpu::lin(cpu.ss, 0x0800), 0x1111); // [sp+0]
    cpu.write16(Cpu::lin(cpu.ss, 0x0802), 0x2222); // [sp+2]
    cpu.write16(Cpu::lin(cpu.ss, 0x0804), 0x3333); // [sp+4] (the arg, cleaned)
    run_n(&mut cpu, 1);
    assert_eq!(cpu.ip, 0x1111, "IP popped");
    assert_eq!(cpu.cs, 0x2222, "CS popped");
    assert_eq!(cpu.r[SP], 0x0806, "SP += 4 (pops) + 2 (imm) = +6");
}

#[test]
fn unknown_opcode_is_a_hard_typed_error() {
    // 0xF4 (HLT) is deliberately unimplemented → must error naming op and IP.
    let mut cpu = synth(&[0xf4]);
    let err = cpu.step().unwrap_err();
    match err {
        CpuError::UnknownOpcode { op, ip } => {
            assert_eq!(op, 0xf4);
            assert_eq!(ip, 0x0000);
        }
        other => panic!("expected UnknownOpcode, got {other:?}"),
    }
}

#[test]
fn runaway_budget_hard_errors() {
    // JMP $ would need an opcode we don't have; instead prove the guard fires
    // by driving steps past BUDGET with a trivial 1-byte NOP loop via IP reset.
    let mut cpu = synth(&[0x90]); // NOP (xchg ax,ax)
                                  // Manually spin: each step re-reads the same NOP if we keep IP at 0.
    let mut last = Ok(());
    for _ in 0..(BUDGET + 5) {
        cpu.ip = 0; // pin IP so it never advances out of the 1-byte program
        last = cpu.step();
        if last.is_err() {
            break;
        }
    }
    assert!(
        matches!(last, Err(CpuError::Runaway { .. })),
        "budget guard must fire, got {last:?}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// DELIVERABLE 3 — the acceptance test (D-OR4 part A): 10,000 (K, N) pairs.
//
// Local-tier, GBX_DATA_DIR-gated, loud-skip when absent. For each pair: run the
// real wrapper bytes; compare BOTH (state', AX) against gbx_prng::Prng.
// ═════════════════════════════════════════════════════════════════════════════

/// Deterministic (K, N) pair generation — **independent of gbx-prng** (D9). The
/// explicit edge cases are enumerated first (so they are covered by intent, not
/// by luck), then the set is filled to 10,000 from a splitmix64 stream. The
/// stream constants are the published splitmix64 mixer, categorically not the TP
/// LCG under test — the thing under test does not choose its own inputs.
fn pair_set() -> Vec<(u32, u16)> {
    // The required N edge values, incl. N == 0 (the draw-always edge).
    let edge_ns: [u16; 9] = [0, 1, 2, 3, 6, 100, 255, 256, 0xFFFF];
    // The required K edge values, incl. asymmetric hi/lo pairs.
    let edge_ks: [u32; 8] = [
        0,
        1,
        0xFFFF_FFFF,
        0x8000_0000,
        0x0000_FFFF, // low word only
        0xFFFF_0000, // high word only
        0x0001_FFFF, // carry-boundary asymmetry
        0xABCD_0001, // arbitrary asymmetric
    ];

    let mut pairs = Vec::with_capacity(10_000);
    // Full cross product of the edges — 72 pairs, N==0 among them for every K.
    for &k in &edge_ks {
        for &n in &edge_ns {
            pairs.push((k, n));
        }
    }

    // Fill the rest from splitmix64. Fixed seed → deterministic, no wall clock.
    let mut s: u64 = 0x1234_5678_9ABC_DEF0;
    let mut next = || {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    while pairs.len() < 10_000 {
        let a = next();
        let k = a as u32;
        // Bias N toward small, game-realistic moduli half the time, full range
        // the other half — but never let it be gbx-prng deciding.
        let n = if (a >> 63) & 1 == 0 {
            ((a >> 32) as u16) % 300
        } else {
            (a >> 32) as u16
        };
        pairs.push((k, n));
    }
    pairs
}

#[test]
fn acceptance_ten_thousand_pairs_match_gbx_prng() {
    let Some(image) = load_image("acceptance_ten_thousand_pairs_match_gbx_prng") else {
        eprintln!(
            "SKIPPED: local tier needs GBX_DATA_DIR \
             (stepper::acceptance_ten_thousand_pairs_match_gbx_prng)"
        );
        return;
    };
    let mut rig = Rig::new(&image);
    assert_eq!(rig.image_len, 0xf3e0, "decompressed image is 62,432 bytes");

    let pairs = pair_set();
    assert_eq!(pairs.len(), 10_000);
    assert!(
        pairs.iter().any(|&(_, n)| n == 0),
        "the N==0 draw-always edge must be present"
    );

    let mut n_zero_checked = 0usize;
    for (i, &(k, n)) in pairs.iter().enumerate() {
        let (ax, state2) = rig
            .call_wrapper(k, n)
            .unwrap_or_else(|e| panic!("stepper error at pair {i} (k={k:#010x}, n={n}): {e}"));

        let mut prng = gbx_prng::Prng::new(0);
        prng.set_state(k);
        let want_value = prng.random(n);
        let want_state = prng.state();

        assert!(
            ax == want_value && state2 == want_state,
            "H3 DISAGREEMENT at pair {i}: k={k:#010x} n={n}\n  \
             real bytes : (state'={state2:#010x}, AX={ax})\n  \
             gbx_prng   : (state'={want_state:#010x}, value={want_value})\n  \
             instruction budget was {BUDGET}. If this implicates the recovery \
             (multiplier / mod-vs-scale / draw-before-N==0), STOP and file a \
             docket candidate per D-OR4A — do not 'fix' either side."
        );

        if n == 0 {
            // The single most important assertion in this test: the binary
            // draws BEFORE the N==0 test and returns 0, and the state advanced.
            assert_eq!(ax, 0, "random(0) returns 0 (k={k:#010x})");
            let mut drawn = gbx_prng::Prng::new(0);
            drawn.set_state(k);
            drawn.next();
            assert_eq!(
                state2,
                drawn.state(),
                "random(0) must have DRAWN: state advanced by exactly one LCG step (k={k:#010x})"
            );
            assert_ne!(
                state2, k,
                "random(0) must not be a no-op on state (k={k:#010x})"
            );
            n_zero_checked += 1;
        }
    }
    assert!(n_zero_checked >= 8, "N==0 covered across all edge Ks");
    eprintln!(
        "acceptance: 10,000 (K,N) pairs matched gbx_prng bit-for-bit \
         ({n_zero_checked} N==0 draw-always cases confirmed by execution)"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// DELIVERABLE 4 — the teeth test: prove the acceptance test could actually FAIL.
//
// The mutants below are wrong implementations, defined LOCALLY (never shipped).
// We run the REAL wrapper bytes and assert they DISAGREE with each mutant on at
// least one (K, N) in the set. If a mutant agreed, the test set would be too
// weak — we say so loudly. This is where the v1 "scaled high word" claim is
// refuted *empirically*, by execution, for the first time in the project.
// ═════════════════════════════════════════════════════════════════════════════

/// A local reference LCG step, used only to derive the post-draw state the
/// mutants need. This duplicates the recovered algebra deliberately — it is the
/// mutants' scaffold, not the thing under test, and the teeth test's job is to
/// show the *real bytes* diverge from the *mutant results*, not from this.
fn ref_step(state: u32) -> u32 {
    state.wrapping_mul(0x0808_8405).wrapping_add(1)
}

#[test]
fn teeth_real_bytes_refute_the_v1_scaled_semantics() {
    let Some(image) = load_image("teeth_real_bytes_refute_the_v1_scaled_semantics") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (stepper::teeth_...scaled_semantics)");
        return;
    };
    let mut rig = Rig::new(&image);

    // The v1-spec mutant (door v1's refuted claim): TP6+ scaled high word,
    // `((hi16 * n) >> 16)`, instead of the TP5.x modulo the binary uses.
    let scaled = |new_state: u32, n: u16| -> u16 {
        let hi16 = new_state >> 16;
        ((hi16 * n as u32) >> 16) as u16
    };

    let mut disagreements = 0usize;
    let mut first: Option<(u32, u16, u16, u16)> = None;
    for &(k, n) in &pair_set() {
        if n < 2 {
            continue; // scaled and modulo agree trivially at n<2 (both 0)
        }
        let (ax_real, _) = rig.call_wrapper(k, n).expect("stepper");
        let new_state = ref_step(k);
        let mutant = scaled(new_state, n);
        if ax_real != mutant {
            disagreements += 1;
            first.get_or_insert((k, n, ax_real, mutant));
        }
    }
    assert!(
        disagreements > 0,
        "TEETH FAILURE: the real bytes NEVER disagreed with the v1 'scaled high word' mutant — \
         the acceptance set is too weak to refute v1, or something far more interesting is true. \
         Investigate before trusting the acceptance result."
    );
    let (k, n, real, mutant) = first.unwrap();
    eprintln!(
        "teeth: v1 scaled-semantics REFUTED empirically by execution — {disagreements} \
         disagreements; first at k={k:#010x} n={n}: real bytes = {real}, v1 scaled = {mutant}"
    );
}

#[test]
fn teeth_real_bytes_refute_the_no_draw_short_circuit() {
    let Some(image) = load_image("teeth_real_bytes_refute_the_no_draw_short_circuit") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (stepper::teeth_...short_circuit)");
        return;
    };
    let mut rig = Rig::new(&image);

    // The short-circuit mutant (coab's exact bug, seg051.cs:33-40): return 0 for
    // n==0 WITHOUT drawing, so state' would equal K. Assert the real bytes
    // disagree on state' for N==0 — i.e. the draw provably happened.
    let mut disagreements = 0usize;
    for &(k, n) in &pair_set() {
        if n != 0 {
            continue;
        }
        let (_ax, state2) = rig.call_wrapper(k, n).expect("stepper");
        let mutant_state = k; // no-draw short-circuit leaves state at K
        if state2 != mutant_state {
            disagreements += 1;
        }
    }
    assert!(
        disagreements > 0,
        "TEETH FAILURE: the real bytes never advanced state on N==0 — the draw-always contract \
         (D-OR1(b)) is NOT confirmed by execution. This implicates the recovery; STOP per D-OR4A."
    );
    eprintln!(
        "teeth: no-draw short-circuit REFUTED — {disagreements} N==0 cases where the real bytes \
         advanced state (coab's seg051.cs:33-40 bug would not have)"
    );
}

#[test]
fn teeth_real_bytes_refute_the_wrong_multiplier() {
    let Some(image) = load_image("teeth_real_bytes_refute_the_wrong_multiplier") else {
        eprintln!("SKIPPED: local tier needs GBX_DATA_DIR (stepper::teeth_...wrong_multiplier)");
        return;
    };
    let mut rig = Rig::new(&image);

    // The wrong-multiplier mutant: 0x08088406 instead of 0x08088405.
    let wrong_step = |state: u32| state.wrapping_mul(0x0808_8406).wrapping_add(1);

    let mut disagreements = 0usize;
    for &(k, n) in &pair_set() {
        let (_ax, state2) = rig.call_wrapper(k, n).expect("stepper");
        if state2 != wrong_step(k) {
            disagreements += 1;
        }
    }
    assert!(
        disagreements > 0,
        "TEETH FAILURE: the real bytes agreed with a WRONG multiplier (0x08088406) on every pair \
         — the stepper is not actually reading the multiplier from memory, or the set is empty."
    );
    eprintln!(
        "teeth: wrong multiplier 0x08088406 REFUTED — {disagreements} disagreements on state'"
    );
}
