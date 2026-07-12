//! `restrike dump-image [DIR] --dax <FILE> --block <ID> [--frame N] [--mask N] --out <path.ppm>`
//! — M2 step 1's optional deliverable 7. Decodes one block from a DAX file
//! as either a static 4bpp image ([`gbx_formats::image`]) or an animated
//! picture ([`gbx_formats::anim`]), depending on the file name's prefix, and
//! writes the selected sub-image/frame as a binary PPM using
//! [`gbx_rules::palette::EGA_PALETTE`] — a cheap way to see decoded art
//! before the renderer exists. The PPM goes wherever `--out` points, never
//! into this repo (D10: no game data or derived art ships here).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use gbx_formats::dax::DaxArchive;
use gbx_formats::{anim, image};
use gbx_rules::palette::EGA_PALETTE;

/// Debug color for transparency-16 pixels in the PPM dump (there's no alpha
/// channel in plain PPM) — a color that never appears in the 16-entry EGA
/// palette itself, so it reads unambiguously as "transparent" by eye.
const TRANSPARENT_DEBUG_RGB: [u8; 3] = [255, 0, 255];

struct Args {
    dir: Option<PathBuf>,
    dax: PathBuf,
    block: u8,
    frame: usize,
    mask: Option<u8>,
    out: PathBuf,
}

impl Args {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut dir = None;
        let mut dax = None;
        let mut block = None;
        let mut frame = 0usize;
        let mut mask = None;
        let mut out = None;

        let mut iter = args.into_iter().peekable();
        // An optional leading positional DIR (no leading `--`).
        if let Some(first) = iter.peek() {
            if !first.starts_with("--") {
                dir = Some(PathBuf::from(iter.next().unwrap()));
            }
        }
        while let Some(flag) = iter.next() {
            match flag.as_str() {
                "--dax" => dax = Some(PathBuf::from(next_val(&mut iter, "--dax")?)),
                "--block" => {
                    block = Some(
                        next_val(&mut iter, "--block")?
                            .parse::<u8>()
                            .map_err(|_| "--block must be a number 0-255".to_string())?,
                    )
                }
                "--frame" => {
                    frame = next_val(&mut iter, "--frame")?
                        .parse::<usize>()
                        .map_err(|_| "--frame must be a non-negative integer".to_string())?
                }
                "--mask" => {
                    mask = Some(
                        next_val(&mut iter, "--mask")?
                            .parse::<u8>()
                            .map_err(|_| "--mask must be a number 0-15".to_string())?,
                    )
                }
                "--out" => out = Some(PathBuf::from(next_val(&mut iter, "--out")?)),
                other => return Err(format!("unknown dump-image flag '{other}'")),
            }
        }

        Ok(Args {
            dir,
            dax: dax.ok_or("dump-image requires --dax <FILE>")?,
            block: block.ok_or("dump-image requires --block <ID>")?,
            frame,
            mask,
            out: out.ok_or("dump-image requires --out <path.ppm>")?,
        })
    }
}

fn next_val(
    iter: &mut std::iter::Peekable<std::vec::IntoIter<String>>,
    flag: &str,
) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("{flag} requires a value"))
}

pub fn cmd_dump_image(args: Vec<String>) -> ExitCode {
    let opts = match Args::parse(args) {
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

    let file_name = dax_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_uppercase();
    let is_pic_or_final = file_name.starts_with("PIC") || file_name.starts_with("FINAL");
    let is_animated = is_pic_or_final || file_name.starts_with("SPRIT");

    let (width, height, pixels) = if is_animated {
        let decoded = match anim::decode(&raw, opts.mask.is_some(), is_pic_or_final) {
            Ok(decoded) => decoded,
            Err(err) => {
                eprintln!(
                    "restrike: failed to decode block {} as an animation: {err:?}",
                    opts.block
                );
                return ExitCode::FAILURE;
            }
        };
        let Some(frame) = decoded.frames.get(opts.frame) else {
            eprintln!(
                "restrike: --frame {} out of range ({} frame(s) decoded)",
                opts.frame,
                decoded.frames.len()
            );
            return ExitCode::FAILURE;
        };
        (
            frame.width_px(),
            frame.height as usize,
            frame.pixels.clone(),
        )
    } else {
        let decoded = match image::decode(&raw, opts.mask) {
            Ok(decoded) => decoded,
            Err(err) => {
                eprintln!(
                    "restrike: failed to decode block {} as an image: {err:?}",
                    opts.block
                );
                return ExitCode::FAILURE;
            }
        };
        let Some(item) = decoded.items.get(opts.frame) else {
            eprintln!(
                "restrike: --frame {} out of range ({} item(s) decoded)",
                opts.frame,
                decoded.items.len()
            );
            return ExitCode::FAILURE;
        };
        (
            decoded.width_px(),
            decoded.height as usize,
            item.pixels.clone(),
        )
    };

    if let Err(err) = write_ppm(&opts.out, width, height, &pixels) {
        eprintln!("restrike: failed to write '{}': {err}", opts.out.display());
        return ExitCode::FAILURE;
    }

    println!(
        "restrike: wrote {}x{} PPM to '{}' ({} bytes source)",
        width,
        height,
        opts.out.display(),
        raw.len()
    );
    ExitCode::SUCCESS
}

fn write_ppm(
    path: &std::path::Path,
    width: usize,
    height: usize,
    pixels: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = Vec::with_capacity(32 + width * height * 3);
    out.extend_from_slice(format!("P6\n{width} {height}\n255\n").as_bytes());
    for &p in pixels {
        let rgb = if p == 16 {
            TRANSPARENT_DEBUG_RGB
        } else {
            EGA_PALETTE[p as usize & 0x0F]
        };
        out.extend_from_slice(&rgb);
    }
    fs::File::create(path)?.write_all(&out)
}

fn print_usage() {
    eprintln!(
        "usage: restrike dump-image [DIR] --dax <FILE> --block <ID> [--frame N] [--mask N] \
         --out <path.ppm>"
    );
    eprintln!();
    eprintln!(
        "Decodes block <ID> from <FILE> as a static 4bpp image (8X8D*/BIGPIC*/HEAD*/BODY*/SKY) \
         or, when <FILE> starts with PIC/SPRIT/FINAL, as an animated picture; --frame selects \
         which item (static) or frame (animated), default 0. --mask <N> masks palette code N to \
         transparency, rendered as a bright magenta debug color in the PPM (no alpha channel in \
         plain PPM)."
    );
}
