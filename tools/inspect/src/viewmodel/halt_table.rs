//! Halt-records TSV export (task brief deliverable 2): the engine pane's
//! "Copy halts" button shape. Pure formatting over
//! `gbx_engine::vmhost::HaltRecord` — no egui.

use gbx_engine::vmhost::HaltRecord;

pub const TSV_HEADERS: [&str; 3] = ["pc", "opcode", "description"];

/// One halt record as a TSV row, matching [`TSV_HEADERS`]'s column order.
pub fn to_tsv_row(halt: &HaltRecord) -> Vec<String> {
    vec![
        format!("{:#06X}", halt.pc),
        format!("{:#04X}", halt.opcode),
        halt.description.clone(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_tsv_row_hex_encodes_pc_and_opcode() {
        let halt = HaltRecord {
            pc: 0x8295,
            opcode: 0x2B,
            description: "opcode 0x2B has no dialect entry".to_string(),
        };
        assert_eq!(
            to_tsv_row(&halt),
            vec![
                "0x8295".to_string(),
                "0x2B".to_string(),
                "opcode 0x2B has no dialect entry".to_string(),
            ]
        );
    }

    #[test]
    fn tsv_headers_match_row_column_count() {
        let halt = HaltRecord {
            pc: 0,
            opcode: 0,
            description: String::new(),
        };
        assert_eq!(TSV_HEADERS.len(), to_tsv_row(&halt).len());
    }
}
