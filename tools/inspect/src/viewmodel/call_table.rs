//! Service-call log TSV export and substring filter (task brief deliverables
//! 2 and 4): the engine pane's "Copy calls" button shape and its filter
//! box. `gbx_vm::RecordedCall` is a method-tagged enum with no fields common
//! across variants, so each row is just its index plus a `Debug`-rendered
//! call — still one TSV column per [`TSV_HEADERS`], still paste-friendly.
//! Pure formatting/filtering — no egui.

use gbx_vm::RecordedCall;

pub const TSV_HEADERS: [&str; 2] = ["index", "call"];

/// One recorded call as a TSV row (`index` is the call's position within
/// the log, not a stored field — `RecordedCall` carries none).
pub fn to_tsv_row(index: usize, call: &RecordedCall) -> Vec<String> {
    vec![index.to_string(), format!("{call:?}")]
}

/// Filters `calls` by a case-insensitive substring match against the call's
/// `Debug` rendering (e.g. typing `"memread"` matches every `MemRead {
/// .. }`) — the same light filtering `log_table::filter_entries` does for
/// the unknown-access log, extended to the call log's less structured shape.
pub fn filter_calls<'a>(calls: &'a [RecordedCall], needle: &str) -> Vec<&'a RecordedCall> {
    if needle.is_empty() {
        return calls.iter().collect();
    }
    let needle = needle.to_ascii_lowercase();
    calls
        .iter()
        .filter(|c| format!("{c:?}").to_ascii_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbx_vm::Origin;

    fn mem_read(addr: u16) -> RecordedCall {
        RecordedCall::MemRead {
            addr,
            origin: Origin { pc: 0x8000 },
        }
    }

    #[test]
    fn to_tsv_row_carries_the_index_and_debug_string() {
        let call = mem_read(0x100);
        let row = to_tsv_row(3, &call);
        assert_eq!(row[0], "3");
        assert!(row[1].contains("MemRead"));
        assert!(row[1].contains("256")); // 0x100 in decimal, Debug's own format
    }

    #[test]
    fn filter_calls_empty_needle_returns_everything() {
        let calls = vec![mem_read(1), mem_read(2)];
        assert_eq!(filter_calls(&calls, "").len(), 2);
    }

    #[test]
    fn filter_calls_matches_case_insensitively() {
        let calls = vec![mem_read(1)];
        assert_eq!(filter_calls(&calls, "memread").len(), 1);
        assert_eq!(filter_calls(&calls, "MEMREAD").len(), 1);
    }

    #[test]
    fn filter_calls_excludes_non_matches() {
        let calls = vec![mem_read(1)];
        assert_eq!(filter_calls(&calls, "partystrength").len(), 0);
    }
}
