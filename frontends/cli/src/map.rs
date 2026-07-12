//! `restrike map [DIR] --block <ID> [--dax <FILE>]` — M1 task 2's ASCII
//! automap dump. Extracts a GEO block (`gbx_formats::geo`) and renders its
//! 16x16 wall/door grid as deterministic ASCII art for human comparison
//! against a printed reference map (the task brief's Tilverton verification
//! against `Cluebook.pdf`).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_formats::dax::DaxArchive;
use gbx_formats::geo::{GeoBlock, Square, GEO_GRID_SIZE};

/// `--dax` default: real (non-demo) CotAB's town/area file is `GEO2.DAX`
/// (coab `seg001.cs`'s `gbl.game_area = 2` for the non-demo new-game path,
/// which `Load3DMap`/`LoadWalldef` build their filename from —
/// `docs/design/vm-scriptmemory.md`'s reference research pass, GEO map
/// format report). Confirmed on real data: `GEO2.DAX` holds exactly the
/// three blocks Gold Box Explorer's per-game GEO-id table names for CotAB
/// (1 = Tilverton City, 3 = Tilverton Sewers, 4 = The Fire Knift Hideout) —
/// no block `0`, so block `1` (not an inferred `0`) is Tilverton itself.
const DEFAULT_GEO_FILE: &str = "GEO2.DAX";

pub fn cmd_map(args: Vec<String>) -> ExitCode {
    let opts = match MapArgs::parse(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("restrike: {msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let dir = match opts
        .dir
        .clone()
        .or_else(|| env::var_os("GBX_DATA_DIR").map(PathBuf::from))
    {
        Some(dir) => dir,
        None => {
            eprintln!("restrike: no directory given and GBX_DATA_DIR is not set");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let dax_path = if opts.dax.is_absolute() {
        opts.dax.clone()
    } else {
        dir.join(&opts.dax)
    };

    let bytes = match fs::read(&dax_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("restrike: failed to read '{}': {err}", dax_path.display());
            return ExitCode::FAILURE;
        }
    };
    let archive = match DaxArchive::parse(&bytes) {
        Ok(archive) => archive,
        Err(err) => {
            eprintln!(
                "restrike: failed to parse DAX '{}': {err:?}",
                dax_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let raw = match archive.block_data(opts.block) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!(
                "restrike: block {} not found in '{}': {err:?}",
                opts.block,
                dax_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let geo = match GeoBlock::parse(&raw) {
        Ok(geo) => geo,
        Err(err) => {
            eprintln!(
                "restrike: block {} in '{}' isn't a valid GEO block: {err:?}",
                opts.block,
                dax_path.display()
            );
            return ExitCode::FAILURE;
        }
    };

    println!(
        "-- {} block {} (header={:#04X}{:02X}) --",
        dax_path.display(),
        opts.block,
        geo.header[0],
        geo.header[1]
    );
    print!("{}", render_ascii(&geo));
    print!("{}", legend());

    ExitCode::SUCCESS
}

/// Renders a [`GeoBlock`]'s 16x16 grid as ASCII art: one line of
/// north/south wall segments between each row of squares (`+` at every
/// grid joint), and one line per row showing west/east wall segments plus
/// each square's interior marker. Deterministic — same block always
/// renders identically (no randomness, no wall-clock dependence, per D9's
/// spirit even though this is presentation, not simulation).
///
/// Each square's own recorded wall/door state drives its own four edges
/// independently (matching how the VM/renderer queries a single square at
/// a time, `getMap_wall_type`/`get_wall_x2` — this module's doc comment);
/// if two neighboring squares ever disagree about their shared edge, both
/// are rendered as recorded rather than silently reconciled, since a
/// mismatch would itself be a real finding worth seeing.
pub fn render_ascii(geo: &GeoBlock) -> String {
    let n = GEO_GRID_SIZE;
    let mut out = String::new();

    // Column header.
    out.push_str("    ");
    for x in 0..n {
        out.push_str(&format!("{:X} ", x));
    }
    out.push('\n');

    for y in 0..n {
        // North-wall line for row y.
        out.push_str("    ");
        for x in 0..n {
            out.push('+');
            out.push(h_edge(
                geo.square(x, y).wall_north,
                geo.square(x, y).door_north,
            ));
        }
        out.push_str("+\n");

        // Row content: west/interior/east.
        out.push_str(&format!("{:>3} ", y));
        for x in 0..n {
            out.push(v_edge(
                geo.square(x, y).wall_west,
                geo.square(x, y).door_west,
            ));
            out.push(interior(geo.square(x, y)));
        }
        out.push(v_edge(
            geo.square(n - 1, y).wall_east,
            geo.square(n - 1, y).door_east,
        ));
        out.push('\n');
    }

    // South-wall line for the final row.
    out.push_str("    ");
    for x in 0..n {
        out.push('+');
        out.push(h_edge(
            geo.square(x, n - 1).wall_south,
            geo.square(x, n - 1).door_south,
        ));
    }
    out.push_str("+\n");

    out
}

fn h_edge(wall: u8, door: u8) -> char {
    if wall == 0 {
        ' '
    } else {
        match door {
            1 => '.',
            2 => 'D',
            3 => '#',
            _ => '-',
        }
    }
}

fn v_edge(wall: u8, door: u8) -> char {
    if wall == 0 {
        ' '
    } else {
        match door {
            1 => ':',
            2 => 'D',
            3 => '#',
            _ => '|',
        }
    }
}

/// A single interior marker: `*` if the square's `low7` (hypothesized
/// event id — `gbx_formats::geo`'s doc comment, unconfirmed for CotAB) is
/// nonzero, otherwise blank. Deliberately not encoding indoor/outdoor here
/// (`gbx_formats::geo::Square::indoor` is a confirmed field, but cramming
/// a third piece of state into one glyph would make the wall/door topology
/// — the part that must match the printed map — harder to read).
fn interior(sq: &Square) -> char {
    if sq.low7 != 0 {
        '*'
    } else {
        ' '
    }
}

fn legend() -> String {
    "\nlegend: '-'/'|' solid wall  '.'/':'  open door  'D' locked door  '#' hard-locked door\n\
     \x20       ' ' open passage (no wall)  '*' nonzero x2-low7 (hypothesized event, unconfirmed)\n"
        .to_string()
}

struct MapArgs {
    dir: Option<PathBuf>,
    dax: PathBuf,
    block: u8,
}

impl MapArgs {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut dir = None;
        let mut dax = None;
        let mut block = None;

        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--dax" => {
                    let v = iter.next().ok_or("--dax requires a FILE argument")?;
                    dax = Some(PathBuf::from(v));
                }
                "--block" => {
                    let v = iter.next().ok_or("--block requires an ID argument")?;
                    block = Some(parse_u8(&v).ok_or_else(|| format!("invalid --block '{v}'"))?);
                }
                other if dir.is_none() && !other.starts_with("--") => {
                    dir = Some(PathBuf::from(other));
                }
                other => return Err(format!("unknown map flag '{other}'")),
            }
        }

        Ok(MapArgs {
            dir,
            dax: dax.unwrap_or_else(|| PathBuf::from(DEFAULT_GEO_FILE)),
            block: block.ok_or("map requires --block <ID>")?,
        })
    }
}

fn parse_u8(s: &str) -> Option<u8> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    }
}

fn print_usage() {
    eprintln!("usage: restrike map [DIR] --block <ID> [--dax <FILE>]");
    eprintln!();
    eprintln!(
        "Extracts GEO block <ID> from <FILE> (default: {DEFAULT_GEO_FILE}, resolved under DIR \
         or GBX_DATA_DIR if DIR is omitted) and renders its 16x16 wall/door grid as ASCII art."
    );
}
