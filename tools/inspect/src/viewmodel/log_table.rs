//! ScriptMemory unknown-access log table: formatting and filtering for
//! `gbx_engine::vmhost::UnknownAccessLog` entries — "the unknown-access log
//! as a live table (addr, kind, origin block+pc, first-seen tick)" (task
//! brief deliverable 4). The log itself carries no first-seen tick (D-VM5's
//! `UnknownAccess` is a first-seen dedup backlog keyed only by `(addr,
//! kind)`, with an `Origin` carrying just a `pc` — see `vmhost.rs`); this
//! module's `LogRow::first_seen_tick` is filled in by the engine pane, which
//! is the only place that knows which tick a *new* entry appeared at
//! (comparing log length across ticks). Pure formatting/filtering — no
//! egui.

use gbx_engine::vmhost::{AccessKind, UnknownAccess};

/// One formatted row for the log table widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRow {
    pub addr_hex: String,
    pub kind: &'static str,
    pub origin_pc_hex: String,
    pub first_seen_tick: Option<u64>,
}

pub fn kind_label(kind: AccessKind) -> &'static str {
    match kind {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
        AccessKind::ReadByte => "read_byte",
        AccessKind::WriteByte => "write_byte",
        AccessKind::ReadString => "read_string",
        AccessKind::WriteString => "write_string",
    }
}

/// Formats one log entry. `first_seen_tick` is the caller's own
/// bookkeeping (see this module's doc comment) — passed through verbatim.
pub fn format_row(entry: &UnknownAccess, first_seen_tick: Option<u64>) -> LogRow {
    LogRow {
        addr_hex: format!("{:#06X}", entry.addr),
        kind: kind_label(entry.kind),
        origin_pc_hex: format!("{:#06X}", entry.origin.pc),
        first_seen_tick,
    }
}

/// Filters `entries` by an optional `kind` and a case-insensitive substring
/// match against the entry's hex address (e.g. typing `"7c1"` matches
/// `0x7C10`) — the resource-light filtering a live table with a growing log
/// needs, kept as plain data transforms so it's testable without a live
/// engine.
pub fn filter_entries<'a>(
    entries: &'a [UnknownAccess],
    kind: Option<AccessKind>,
    addr_substr: &str,
) -> Vec<&'a UnknownAccess> {
    let needle = addr_substr.to_ascii_lowercase();
    entries
        .iter()
        .filter(|e| kind.is_none_or(|k| e.kind == k))
        .filter(|e| needle.is_empty() || format!("{:#06x}", e.addr).contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_vm::Origin;

    fn entry(addr: u16, kind: AccessKind, pc: u16) -> UnknownAccess {
        UnknownAccess {
            addr,
            kind,
            origin: Origin { pc },
        }
    }

    #[test]
    fn format_row_hex_encodes_addr_and_origin_pc() {
        let row = format_row(&entry(0x7C10, AccessKind::Write, 0x8123), Some(42));
        assert_eq!(row.addr_hex, "0x7C10");
        assert_eq!(row.origin_pc_hex, "0x8123");
        assert_eq!(row.kind, "write");
        assert_eq!(row.first_seen_tick, Some(42));
    }

    #[test]
    fn kind_label_covers_every_variant() {
        assert_eq!(kind_label(AccessKind::Read), "read");
        assert_eq!(kind_label(AccessKind::Write), "write");
        assert_eq!(kind_label(AccessKind::ReadByte), "read_byte");
        assert_eq!(kind_label(AccessKind::WriteByte), "write_byte");
        assert_eq!(kind_label(AccessKind::ReadString), "read_string");
        assert_eq!(kind_label(AccessKind::WriteString), "write_string");
    }

    #[test]
    fn filter_entries_with_no_filters_returns_everything() {
        let entries = vec![
            entry(0x100, AccessKind::Read, 0),
            entry(0x200, AccessKind::Write, 0),
        ];
        assert_eq!(filter_entries(&entries, None, "").len(), 2);
    }

    #[test]
    fn filter_entries_by_kind() {
        let entries = vec![
            entry(0x100, AccessKind::Read, 0),
            entry(0x200, AccessKind::Write, 0),
        ];
        let filtered = filter_entries(&entries, Some(AccessKind::Write), "");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].addr, 0x200);
    }

    #[test]
    fn filter_entries_by_addr_substring_is_case_insensitive() {
        let entries = vec![
            entry(0x7C10, AccessKind::Read, 0),
            entry(0x1234, AccessKind::Read, 0),
        ];
        let filtered = filter_entries(&entries, None, "7c1");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].addr, 0x7C10);
    }

    #[test]
    fn filter_entries_combines_kind_and_substring() {
        let entries = vec![
            entry(0x7C10, AccessKind::Read, 0),
            entry(0x7C10, AccessKind::Write, 0),
            entry(0x1234, AccessKind::Write, 0),
        ];
        let filtered = filter_entries(&entries, Some(AccessKind::Write), "7c1");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].kind, AccessKind::Write);
    }
}
