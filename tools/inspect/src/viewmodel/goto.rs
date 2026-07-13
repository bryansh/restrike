//! Goto-address support for the disassembly pane (task brief deliverable
//! 4): parses a user-typed address (`"0x8295"` or `"8295"`, both hex — every
//! address this app ever displays is hex, so a bare-digit paste is
//! interpreted the same way) and finds which line of a
//! [`gbx_vm::disasm::Listing::render`] blob that address lands on, so the
//! pane can scroll to it and highlight the line. Pure string/number parsing
//! over the rendered text — no dependency on `Listing` itself, so this is
//! unit-testable without constructing a real disassembly.

/// Parses a goto-box address: an optional `0x`/`0X` prefix, then hex digits
/// — matches this app's own `{addr:#06X}` rendering convention, so a value
/// copied from the listing (with or without the prefix) round-trips.
/// `None` for empty input or anything that isn't valid hex.
pub fn parse_address(input: &str) -> Option<u16> {
    let trimmed = input.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if hex.is_empty() {
        return None;
    }
    u16::from_str_radix(hex, 16).ok()
}

/// Parses the `0x`-prefixed hex address (or address range, for a data
/// region's `start..end`) at the very start of one rendered listing line.
/// Point addresses (instructions, errors, quarantine entries) are rendered
/// as `[addr, addr]`; data regions as `[start, end)` per
/// `Listing::render`'s own `region.start + region.len` exclusive-end math.
/// `None` for label lines and anything else with no leading `0x`.
fn leading_address_range(line: &str) -> Option<(u16, u16)> {
    let rest = line.strip_prefix("0x")?;
    let first_len = rest.find(|c: char| !c.is_ascii_hexdigit())?;
    if first_len == 0 {
        return None;
    }
    let start = u16::from_str_radix(&rest[..first_len], 16).ok()?;
    let after = &rest[first_len..];
    if let Some(range_rest) = after.strip_prefix("..0x") {
        let second_len = range_rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(range_rest.len());
        if second_len == 0 {
            return None;
        }
        let end = u16::from_str_radix(&range_rest[..second_len], 16).ok()?;
        Some((start, end))
    } else {
        Some((start, start.wrapping_add(1)))
    }
}

/// Finds the 0-based line index of `rendered` (a `Listing::render` blob)
/// whose address range contains `addr`. Prefers an exact/containing match;
/// falls back to the line with the closest preceding start address (e.g. an
/// address that falls inside an instruction's operand bytes, which has no
/// line of its own) — `None` only if no address-prefixed line starts at or
/// before `addr` at all.
pub fn find_line_for_address(rendered: &str, addr: u16) -> Option<usize> {
    let mut best: Option<(usize, u16)> = None;
    for (i, line) in rendered.lines().enumerate() {
        let Some((start, end)) = leading_address_range(line) else {
            continue;
        };
        if (start..end).contains(&addr) {
            return Some(i);
        }
        if start <= addr {
            let dist = addr - start;
            if best.is_none_or(|(_, best_dist)| dist < best_dist) {
                best = Some((i, dist));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// The char offset (not byte offset — matches `egui::text::CCursor`'s
/// counting unit) of the start of `text`'s line `line_idx`, assuming `\n`
/// line endings (true of every string this app renders internally).
pub fn char_offset_for_line(text: &str, line_idx: usize) -> usize {
    text.lines()
        .take(line_idx)
        .map(|line| line.chars().count() + 1)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_address_accepts_0x_prefix() {
        assert_eq!(parse_address("0x8295"), Some(0x8295));
    }

    #[test]
    fn parse_address_accepts_bare_hex_digits() {
        assert_eq!(parse_address("8295"), Some(0x8295));
    }

    #[test]
    fn parse_address_is_case_insensitive_on_prefix_and_digits() {
        assert_eq!(parse_address("0XABCD"), Some(0xABCD));
        assert_eq!(parse_address("abcd"), Some(0xABCD));
    }

    #[test]
    fn parse_address_trims_whitespace() {
        assert_eq!(parse_address("  0x10  "), Some(0x10));
    }

    #[test]
    fn parse_address_rejects_empty_and_garbage() {
        assert_eq!(parse_address(""), None);
        assert_eq!(parse_address("0x"), None);
        assert_eq!(parse_address("not hex"), None);
        assert_eq!(parse_address("0x1234G"), None);
    }

    #[test]
    fn find_line_for_address_matches_an_instruction_line_exactly() {
        let rendered = "0x8000: NOP\n0x8001: NOP\n0x8295: HALT\n";
        assert_eq!(find_line_for_address(rendered, 0x8295), Some(2));
    }

    #[test]
    fn find_line_for_address_matches_inside_a_data_region_range() {
        let rendered = "0x8000: NOP\n0x8010..0x8020: <data, 16 bytes>\n";
        assert_eq!(find_line_for_address(rendered, 0x8015), Some(1));
    }

    #[test]
    fn find_line_for_address_falls_back_to_nearest_preceding_line() {
        // 0x8003 has no line of its own (mid-instruction); nearest
        // preceding start is 0x8000.
        let rendered = "0x8000: LONGOP a, b, c\n0x8010: NOP\n";
        assert_eq!(find_line_for_address(rendered, 0x8003), Some(0));
    }

    #[test]
    fn find_line_for_address_skips_label_lines() {
        let rendered = "some_label:\n0x8000: NOP\n";
        assert_eq!(find_line_for_address(rendered, 0x8000), Some(1));
    }

    #[test]
    fn find_line_for_address_none_when_addr_precedes_everything() {
        let rendered = "0x8000: NOP\n";
        assert_eq!(find_line_for_address(rendered, 0x7FFF), None);
    }

    #[test]
    fn find_line_for_address_prefers_closest_preceding_over_file_order() {
        // Quarantine section (later in the file) starts lower than the
        // main listing's last line -- the nearest-by-address line should
        // win, not merely the last line seen.
        let rendered = "0x8000: NOP\n0x9000: NOP\n-- quarantine --\n0x8500: NOP [quarantined]\n";
        assert_eq!(find_line_for_address(rendered, 0x8600), Some(3));
    }

    #[test]
    fn char_offset_for_line_sums_preceding_lines_plus_newlines() {
        let text = "abc\nde\nfghij\n";
        assert_eq!(char_offset_for_line(text, 0), 0);
        assert_eq!(char_offset_for_line(text, 1), 4); // "abc\n"
        assert_eq!(char_offset_for_line(text, 2), 7); // "abc\n" + "de\n"
    }
}
