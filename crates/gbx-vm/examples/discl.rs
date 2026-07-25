//! Dev aid: print the flow-following disassembly of one real ECL block.
//! Usage: cargo run -p gbx-vm --example discl -- <ECLn.DAX path> <block id> [entry-hex]
//! Local-only (reads operator-given game data paths; prints listing text).
use gbx_formats::dax::{self, DaxArchive};
use gbx_vm::decode::{read_header_vectors, BlockBytes, ECL_BLOCK_SIZE};
use gbx_vm::dialect::{COTAB, COTAB_VECTOR_COUNT};
use gbx_vm::disassemble;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("dax path");
    let id: u8 = args.next().expect("block id").parse().unwrap();
    let only_entry: Option<u16> = args
        .next()
        .map(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).unwrap());

    let bytes = std::fs::read(&path).unwrap();
    let archive = DaxArchive::parse(&bytes).unwrap();
    let raw = archive.block_data(id).unwrap();
    let payload = dax::ecl_block_payload(&raw);
    assert!(!payload.is_empty() && payload.len() <= ECL_BLOCK_SIZE);
    let block = BlockBytes::from_bytes(payload);
    let (vectors, _) = read_header_vectors(&block, COTAB_VECTOR_COUNT);
    println!("vectors: {vectors:04X?}");
    let entries: Vec<u16> = match only_entry {
        Some(e) => vec![e],
        None => vectors.into_iter().flatten().collect(),
    };
    let listing = disassemble(&block, &COTAB, &entries);
    print!("{}", listing.render(&COTAB));
}
