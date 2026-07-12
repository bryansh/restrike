//! Per-opcode static tables ("dialects") for the ECL bytecode.
//!
//! Per D-VM7 (`docs/design/vm-scriptmemory.md`), dialects are data plus small
//! code: the opcode table (names, skip sizes, operand shapes) is registered
//! per game flavor. This module ships the CotAB dialect only — its table is
//! transcribed directly from `docs/design/opcode-classification.md` (M1 step
//! 0), which was built by reading every coab handler for `0x00`-`0x40`.
//!
//! `skip_size` and `shape` are deliberately two different fields (D-VM1):
//! `skip_size` is the original's `CmdItem` size column, used only by the
//! (not-yet-implemented) skip path. `shape` is what [`decode`](crate::decode)
//! actually consumes at run time, derived from each handler's real
//! `vm_LoadCmdSets` call sequence. For most opcodes these agree; ECL CLOCK
//! (`0x34`) and ADD NPC (`0x36`) are the two confirmed exceptions — both
//! declare `skip_size` 1 but their handlers consume 2 operands via a single
//! `vm_LoadCmdSets(2)` call (`ovr003.cs:1720-1727`/`1769-1782`, cross-checked
//! against `ovr003.cs:2115`/`2117`'s `CommandTable` entries).

/// A single channel an opcode's handler touches, per D-VM4's placement rule.
/// An opcode commonly carries more than one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Pure VM-internal state: pc, compare flags, call stack, string registers.
    Machine,
    /// ScriptMemory reads/writes through the address windows.
    Mem,
    /// A synchronous EngineServices call touching game entities that aren't
    /// raw memory cells.
    Svc,
    /// Buffered presentation output; does not suspend the activation.
    Eff,
    /// Suspends the activation awaiting a reply.
    Req,
}

/// How an opcode's operands are laid out, independent of the declared skip
/// size (D-VM1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandShape {
    /// Exactly `n` operand batches, unconditionally.
    Fixed(u8),
    /// Decode `fixed_prefix` operand batches, then reinterpret the *last* of
    /// those as a count and decode that many more. The four opcodes with
    /// this shape in CotAB (VERTICAL MENU, HORIZONTAL MENU, ON GOTO, ON
    /// GOSUB) all follow the identical `vm_LoadCmdSets(fixed_prefix)` →
    /// rewind → `vm_LoadCmdSets(count)` pattern (see
    /// `docs/design/opcode-classification.md` rows 0x15/0x25/0x26/0x2B).
    VariableTail { fixed_prefix: u8 },
}

/// A dialect's static entry for one opcode.
#[derive(Debug, Clone, Copy)]
pub struct OpcodeInfo {
    pub op: u8,
    pub name: &'static str,
    /// The original `CmdItem` table's size column, transcribed verbatim —
    /// never derived. Not consulted by [`decode`](crate::decode); reserved
    /// for the future skip-path implementation.
    pub skip_size: u8,
    pub shape: OperandShape,
    pub channels: &'static [Channel],
}

/// A named table of [`OpcodeInfo`] entries for one game flavor.
#[derive(Debug, Clone, Copy)]
pub struct Dialect {
    pub name: &'static str,
    pub opcodes: &'static [OpcodeInfo],
}

impl Dialect {
    pub fn lookup(&self, op: u8) -> Option<&OpcodeInfo> {
        self.opcodes.iter().find(|info| info.op == op)
    }
}

use Channel::{Eff, Machine, Mem, Req, Svc};
use OperandShape::{Fixed, VariableTail};

/// The CotAB (Curse of the Azure Bonds) opcode table, `0x00`-`0x40`.
///
/// `0x1F` ("notsure 0x1f") has no handler in coab — a null delegate in its
/// `CommandTable` entry (`ovr003.cs:2094`). Its `shape` here falls back to
/// `Fixed(skip_size)` since there is no run-time behavior to derive a shape
/// from; this is the best-known datum, not a verified run-time fact. See
/// `docs/design/opcode-classification.md` §5 item 1.
pub static COTAB: Dialect = Dialect {
    name: "CotAB",
    opcodes: &COTAB_OPCODES,
};

static COTAB_OPCODES: [OpcodeInfo; 65] = [
    OpcodeInfo {
        op: 0x00,
        name: "EXIT",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine, Svc],
    },
    OpcodeInfo {
        op: 0x01,
        name: "GOTO",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x02,
        name: "GOSUB",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x03,
        name: "COMPARE",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[Machine, Mem],
    },
    OpcodeInfo {
        op: 0x04,
        name: "ADD",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x05,
        name: "SUBTRACT",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x06,
        name: "DIVIDE",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x07,
        name: "MULTIPLY",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x08,
        name: "RANDOM",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x09,
        name: "SAVE",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x0A,
        name: "LOAD CHARACTER",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Mem, Svc, Eff],
    },
    OpcodeInfo {
        op: 0x0B,
        name: "LOAD MONSTER",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x0C,
        name: "SETUP MONSTER",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Machine, Mem, Svc, Eff],
    },
    OpcodeInfo {
        op: 0x0D,
        name: "APPROACH",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine, Eff],
    },
    OpcodeInfo {
        op: 0x0E,
        name: "PICTURE",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Mem, Eff],
    },
    OpcodeInfo {
        op: 0x0F,
        name: "INPUT NUMBER",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[Machine, Mem, Req],
    },
    OpcodeInfo {
        op: 0x10,
        name: "INPUT STRING",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[Machine, Mem, Req],
    },
    OpcodeInfo {
        op: 0x11,
        name: "PRINT",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Mem, Eff],
    },
    OpcodeInfo {
        op: 0x12,
        name: "PRINTCLEAR",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Mem, Eff],
    },
    OpcodeInfo {
        op: 0x13,
        name: "RETURN",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x14,
        name: "COMPARE AND",
        skip_size: 4,
        shape: Fixed(4),
        channels: &[Machine, Mem],
    },
    OpcodeInfo {
        op: 0x15,
        name: "VERTICAL MENU",
        skip_size: 0,
        shape: VariableTail { fixed_prefix: 3 },
        channels: &[Machine, Mem, Eff, Req],
    },
    OpcodeInfo {
        op: 0x16,
        name: "IF =",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x17,
        name: "IF <>",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x18,
        name: "IF <",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x19,
        name: "IF >",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x1A,
        name: "IF <=",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x1B,
        name: "IF >=",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x1C,
        name: "CLEARMONSTERS",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine, Svc],
    },
    OpcodeInfo {
        op: 0x1D,
        name: "PARTYSTRENGTH",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x1E,
        name: "CHECKPARTY",
        skip_size: 6,
        shape: Fixed(6),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x1F,
        name: "notsure 0x1f",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[],
    },
    OpcodeInfo {
        op: 0x20,
        name: "NEWECL",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Mem, Svc],
    },
    OpcodeInfo {
        op: 0x21,
        name: "LOAD FILES",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Svc, Eff],
    },
    OpcodeInfo {
        op: 0x22,
        name: "PARTY SURPRISE",
        skip_size: 2,
        shape: Fixed(2),
        channels: &[Mem, Svc],
    },
    OpcodeInfo {
        op: 0x23,
        name: "SURPRISE",
        skip_size: 4,
        shape: Fixed(4),
        channels: &[Mem, Svc],
    },
    OpcodeInfo {
        op: 0x24,
        name: "COMBAT",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Req, Svc, Eff, Machine],
    },
    OpcodeInfo {
        op: 0x25,
        name: "ON GOTO",
        skip_size: 0,
        shape: VariableTail { fixed_prefix: 2 },
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x26,
        name: "ON GOSUB",
        skip_size: 0,
        shape: VariableTail { fixed_prefix: 2 },
        channels: &[Machine],
    },
    OpcodeInfo {
        op: 0x27,
        name: "TREASURE",
        skip_size: 8,
        shape: Fixed(8),
        channels: &[Svc, Mem],
    },
    OpcodeInfo {
        op: 0x28,
        name: "ROB",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Svc, Mem],
    },
    OpcodeInfo {
        op: 0x29,
        name: "ENCOUNTER MENU",
        skip_size: 14,
        shape: Fixed(14),
        channels: &[Mem, Svc, Eff, Req],
    },
    OpcodeInfo {
        op: 0x2A,
        name: "GETTABLE",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Mem],
    },
    OpcodeInfo {
        op: 0x2B,
        name: "HORIZONTAL MENU",
        skip_size: 0,
        shape: VariableTail { fixed_prefix: 2 },
        channels: &[Machine, Mem, Svc, Req],
    },
    OpcodeInfo {
        op: 0x2C,
        name: "PARLAY",
        skip_size: 6,
        shape: Fixed(6),
        channels: &[Mem, Req],
    },
    OpcodeInfo {
        op: 0x2D,
        name: "CALL",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Svc, Eff],
    },
    OpcodeInfo {
        op: 0x2E,
        name: "DAMAGE",
        skip_size: 5,
        shape: Fixed(5),
        channels: &[Svc, Eff, Machine],
    },
    OpcodeInfo {
        op: 0x2F,
        name: "AND",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Mem, Machine],
    },
    OpcodeInfo {
        op: 0x30,
        name: "OR",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Mem, Machine],
    },
    OpcodeInfo {
        op: 0x31,
        name: "SPRITE OFF",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine, Eff],
    },
    OpcodeInfo {
        op: 0x32,
        name: "FIND ITEM",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Svc],
    },
    OpcodeInfo {
        op: 0x33,
        name: "PRINT RETURN",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Eff],
    },
    // ECL CLOCK: skip_size=1 but the handler decodes 2 operands via one
    // vm_LoadCmdSets(2) call — the confirmed skip≠run divergence.
    OpcodeInfo {
        op: 0x34,
        name: "ECL CLOCK",
        skip_size: 1,
        shape: Fixed(2),
        channels: &[Svc],
    },
    OpcodeInfo {
        op: 0x35,
        name: "SAVE TABLE",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Mem],
    },
    // ADD NPC: same divergence shape as ECL CLOCK.
    OpcodeInfo {
        op: 0x36,
        name: "ADD NPC",
        skip_size: 1,
        shape: Fixed(2),
        channels: &[Svc, Eff],
    },
    OpcodeInfo {
        op: 0x37,
        name: "LOAD PIECES",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Svc, Eff, Machine],
    },
    OpcodeInfo {
        op: 0x38,
        name: "PROGRAM",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Svc, Req, Eff, Machine],
    },
    OpcodeInfo {
        op: 0x39,
        name: "WHO",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Req],
    },
    OpcodeInfo {
        op: 0x3A,
        name: "DELAY",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Machine, Req],
    },
    OpcodeInfo {
        op: 0x3B,
        name: "SPELL",
        skip_size: 3,
        shape: Fixed(3),
        channels: &[Svc, Mem],
    },
    OpcodeInfo {
        op: 0x3C,
        name: "PROTECTION",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Req, Eff],
    },
    OpcodeInfo {
        op: 0x3D,
        name: "CLEAR BOX",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Eff],
    },
    OpcodeInfo {
        op: 0x3E,
        name: "DUMP",
        skip_size: 0,
        shape: Fixed(0),
        channels: &[Svc, Machine, Eff],
    },
    OpcodeInfo {
        op: 0x3F,
        name: "FIND SPECIAL",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Machine, Svc],
    },
    OpcodeInfo {
        op: 0x40,
        name: "DESTROY ITEMS",
        skip_size: 1,
        shape: Fixed(1),
        channels: &[Svc],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_exactly_65_entries_0x00_to_0x40() {
        assert_eq!(COTAB.opcodes.len(), 65);
        for (i, info) in COTAB.opcodes.iter().enumerate() {
            assert_eq!(
                info.op, i as u8,
                "opcode table must be dense and ordered 0x00..=0x40"
            );
        }
    }

    #[test]
    fn no_duplicate_opcodes() {
        let mut seen = std::collections::HashSet::new();
        for info in COTAB.opcodes {
            assert!(seen.insert(info.op), "duplicate opcode {:#04x}", info.op);
        }
    }

    #[test]
    fn lookup_finds_known_and_rejects_unknown() {
        assert_eq!(COTAB.lookup(0x00).unwrap().name, "EXIT");
        assert_eq!(COTAB.lookup(0x29).unwrap().name, "ENCOUNTER MENU");
        assert!(COTAB.lookup(0x41).is_none());
        assert!(COTAB.lookup(0xFF).is_none());
    }

    #[test]
    fn known_skip_run_divergences_are_encoded() {
        let clock = COTAB.lookup(0x34).unwrap();
        assert_eq!(clock.skip_size, 1);
        assert_eq!(clock.shape, Fixed(2));

        let add_npc = COTAB.lookup(0x36).unwrap();
        assert_eq!(add_npc.skip_size, 1);
        assert_eq!(add_npc.shape, Fixed(2));

        for op in [0x15, 0x25, 0x26, 0x2B] {
            let info = COTAB.lookup(op).unwrap();
            assert_eq!(
                info.skip_size, 0,
                "opcode {op:#04x} should declare skip size 0"
            );
            assert!(
                matches!(info.shape, VariableTail { .. }),
                "opcode {op:#04x} should have a variable-tail shape"
            );
        }
    }

    #[test]
    fn unknown_opcode_0x1f_has_no_channels() {
        let unk = COTAB.lookup(0x1F).unwrap();
        assert_eq!(unk.name, "notsure 0x1f");
        assert!(unk.channels.is_empty());
    }
}
