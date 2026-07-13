//! `restrike compare <ours.ppm> <capture.png|capture.ppm> [--diff-out <path>]`
//! — the DOSBox side-by-side comparison helper (M2 step 8, D-UI7's capture
//! procedure). Decodes both images (`image` crate, CLI-only per D-UI5's
//! crate-purity rule — never a `gbx-*` dependency), reports per-pixel diff
//! stats, and writes a diff image highlighting mismatched pixels in red.
//!
//! Both sides go through the same decoder (`image::open`), so `ours.ppm`
//! (our own `restrike walk --dump-at`/`dump-image` output) and a DOSBox
//! screenshot (PNG) are handled uniformly; a raw `.ppm` capture works too.
//! Structural comparison only — no palette/rounding normalization; the M2
//! exit gate's own bar is "structural match... exact pixel equality is the
//! aspiration once palette/rounding details settle" (D-UI7).

use image::{GenericImageView, Rgba};
use std::path::PathBuf;
use std::process::ExitCode;

pub fn cmd_compare(args: Vec<String>) -> ExitCode {
    let opts = match Args::parse(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("restrike: {msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let ours = match image::open(&opts.ours) {
        Ok(img) => img,
        Err(err) => {
            eprintln!(
                "restrike: failed to decode '{}': {err}",
                opts.ours.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let theirs = match image::open(&opts.theirs) {
        Ok(img) => img,
        Err(err) => {
            eprintln!(
                "restrike: failed to decode '{}': {err}",
                opts.theirs.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let (ow, oh) = ours.dimensions();
    let (tw, th) = theirs.dimensions();
    if (ow, oh) != (tw, th) {
        eprintln!(
            "restrike: dimension mismatch — '{}' is {ow}x{oh}, '{}' is {tw}x{th}. \
             Capture at raw, unscaled 320x200 (see docs/dosbox-capture.md) before comparing.",
            opts.ours.display(),
            opts.theirs.display()
        );
        return ExitCode::FAILURE;
    }

    let stats = diff_stats(&ours, &theirs);
    println!(
        "-- compare: '{}' vs '{}' --",
        opts.ours.display(),
        opts.theirs.display()
    );
    println!("dimensions: {ow}x{oh} ({} pixels)", stats.total_pixels);
    println!(
        "differing pixels: {} ({:.2}%)",
        stats.differing_pixels,
        100.0 * stats.differing_pixels as f64 / stats.total_pixels as f64
    );
    println!("mean abs channel diff: {:.3}", stats.mean_abs_diff);
    println!("max channel diff: {}", stats.max_diff);

    if let Some(path) = &opts.diff_out {
        let diff_img = render_diff(&ours, &theirs);
        match diff_img.save(path) {
            Ok(()) => println!("diff image written to '{}'", path.display()),
            Err(err) => {
                eprintln!(
                    "restrike: failed to write diff image '{}': {err}",
                    path.display()
                );
                return ExitCode::FAILURE;
            }
        }
    }

    ExitCode::SUCCESS
}

struct DiffStats {
    total_pixels: u64,
    differing_pixels: u64,
    mean_abs_diff: f64,
    max_diff: u8,
}

/// Per-pixel diff over the RGB channels (alpha ignored — neither PPM dumps
/// nor DOSBox PNG captures carry meaningful transparency). A pixel "differs"
/// if any channel differs at all — the strictest useful bar; `mean_abs_diff`
/// and `max_diff` give the magnitude for pixels that do.
fn diff_stats(a: &image::DynamicImage, b: &image::DynamicImage) -> DiffStats {
    let (w, h) = a.dimensions();
    let mut differing_pixels = 0u64;
    let mut sum_abs_diff = 0u64;
    let mut max_diff = 0u8;
    for y in 0..h {
        for x in 0..w {
            let pa = a.get_pixel(x, y);
            let pb = b.get_pixel(x, y);
            let mut pixel_differs = false;
            for c in 0..3 {
                let d = pa.0[c].abs_diff(pb.0[c]);
                if d > 0 {
                    pixel_differs = true;
                }
                sum_abs_diff += d as u64;
                max_diff = max_diff.max(d);
            }
            if pixel_differs {
                differing_pixels += 1;
            }
        }
    }
    let total_pixels = w as u64 * h as u64;
    DiffStats {
        total_pixels,
        differing_pixels,
        mean_abs_diff: sum_abs_diff as f64 / (total_pixels * 3) as f64,
        max_diff,
    }
}

/// A diff image the same size as the inputs: matching pixels dim to
/// grayscale (so the underlying image is still legible), differing pixels
/// paint solid red — a quick visual spot-check, not a heatmap.
fn render_diff(a: &image::DynamicImage, b: &image::DynamicImage) -> image::RgbaImage {
    let (w, h) = a.dimensions();
    let mut out = image::RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let pa = a.get_pixel(x, y);
            let pb = b.get_pixel(x, y);
            let differs = (0..3).any(|c| pa.0[c] != pb.0[c]);
            let pixel = if differs {
                Rgba([255, 0, 0, 255])
            } else {
                let gray = ((pa.0[0] as u32 + pa.0[1] as u32 + pa.0[2] as u32) / 3 / 2) as u8;
                Rgba([gray, gray, gray, 255])
            };
            out.put_pixel(x, y, pixel);
        }
    }
    out
}

struct Args {
    ours: PathBuf,
    theirs: PathBuf,
    diff_out: Option<PathBuf>,
}

impl Args {
    fn parse(args: Vec<String>) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut diff_out = None;
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--diff-out" => {
                    diff_out = Some(PathBuf::from(
                        iter.next().ok_or("--diff-out requires a PATH argument")?,
                    ))
                }
                other => positional.push(other.to_string()),
            }
        }
        if positional.len() != 2 {
            return Err("compare requires exactly two image paths".to_string());
        }
        let mut it = positional.into_iter();
        Ok(Args {
            ours: PathBuf::from(it.next().unwrap()),
            theirs: PathBuf::from(it.next().unwrap()),
            diff_out,
        })
    }
}

fn print_usage() {
    eprintln!("usage: restrike compare <ours.ppm> <capture.png|capture.ppm> [--diff-out <path>]");
    eprintln!();
    eprintln!(
        "Decodes both images and reports per-pixel diff stats: differing pixel count/percent, \
         mean absolute channel diff, max channel diff. Both images must be the same dimensions \
         (capture DOSBox at raw, unscaled 320x200 — see docs/dosbox-capture.md). --diff-out \
         writes a same-size image (any format the `image` crate can save, by extension) with \
         differing pixels painted red and matching pixels dimmed to grayscale."
    );
}
