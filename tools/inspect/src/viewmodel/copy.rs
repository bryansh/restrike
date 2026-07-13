//! Paste-friendly export formatters (task brief deliverable 2): TSV (header
//! row included) for tabular panes, `key=value` lines for the engine-state
//! summary. Pure string building, no egui — the shape every clipboard-copy
//! button in the app funnels through before handing text to
//! `egui::Context::copy_text`.

/// Joins `headers` and each row of `rows` into one TSV blob: the header
/// line first, then one `\t`-joined, `\n`-terminated line per row — the
/// shape a spreadsheet or plain-text paste target expects. Every row must
/// have the same length as `headers`; a mismatch isn't validated here (the
/// caller controls both, per this module's doc comment).
pub fn to_tsv(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(&headers.join("\t"));
    out.push('\n');
    for row in rows {
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
    out
}

/// Formats `pairs` as `key=value` lines, one per pair — the engine-state
/// summary's copy shape (task brief deliverable 2).
pub fn to_key_value(pairs: &[(&str, String)]) -> String {
    let mut out = String::new();
    for (key, value) in pairs {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_tsv_emits_header_then_tab_joined_rows() {
        let rows = vec![
            vec!["0x100".to_string(), "read".to_string()],
            vec!["0x200".to_string(), "write".to_string()],
        ];
        let out = to_tsv(&["addr", "kind"], &rows);
        assert_eq!(out, "addr\tkind\n0x100\tread\n0x200\twrite\n");
    }

    #[test]
    fn to_tsv_with_no_rows_is_just_the_header_line() {
        let out = to_tsv(&["a", "b"], &[]);
        assert_eq!(out, "a\tb\n");
    }

    #[test]
    fn to_key_value_joins_with_equals_and_newlines() {
        let pairs = vec![
            ("pos", "(3, 4)".to_string()),
            ("facing", "North".to_string()),
        ];
        let out = to_key_value(&pairs);
        assert_eq!(out, "pos=(3, 4)\nfacing=North\n");
    }

    #[test]
    fn to_key_value_empty_is_empty_string() {
        assert_eq!(to_key_value(&[]), "");
    }
}
