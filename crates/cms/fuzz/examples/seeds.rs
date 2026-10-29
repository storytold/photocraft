//! Writes a starting corpus for this crate's fuzz targets: our built-in ICC profiles (matrix and
//! curve, gray, and the CLUT-based coated CMYK) and Color Lookup files in each format the target
//! reads, so libFuzzer starts from real tag tables and LUT layouts. Each seed must parse, or this
//! fails: a refused seed only teaches the fuzzer the error path.
//!
//! `cargo run --example seeds -- [target ...]` (no target: all) writes `corpus/<target>/seed-*`.

use std::path::Path;

use photocraft_cms::{Profile, builtin, lutfile};

/// A .3dl with a linear 17-point input shaper line and 12-bit output (scaled down on read).
fn lut_3dl() -> String {
    let shaper: Vec<String> = (0..17).map(|i| (i * 64).min(1023).to_string()).collect();
    let mut s = format!("# seed\n{}\n", shaper.join(" "));
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                s.push_str(&format!("{} {} {}\n", r * 4095, g * 3000, b * 4095));
            }
        }
    }
    s
}

/// A SpeedGrade .look: size 2, little-endian float32 RGB triplets as hex.
fn lut_look() -> String {
    let hex: String = (0..8u32)
        .flat_map(|i| [(i & 1) as f32, ((i >> 1) & 1) as f32 * 0.75, (i >> 2) as f32])
        .flat_map(f32::to_le_bytes)
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("<?xml version=\"1.0\"?>\n<look><title>\"Seed\"</title><LUT><size>\"2\"</size><data>\"{hex}\"</data></LUT></look>\n")
}

/// The seeds of one target.
fn seeds(target: &str) -> Vec<Vec<u8>> {
    match target {
        "icc_profile" => builtin::Builtin::ALL.iter().map(|b| b.profile().to_bytes().to_vec()).chain([builtin::COATED_CMYK_ICC.to_vec()]).collect(),
        // The target's first byte picks the extension: 0 .cube, 1 .3dl, 2 .look.
        "lut_file" => {
            let mut v: Vec<Vec<u8>> = lutfile::BUILTIN
                .iter()
                .take(3)
                .filter_map(|(id, _)| lutfile::builtin(id))
                .map(|l| [&[0u8][..], lutfile::write_cube(&l).as_bytes()].concat())
                .collect();
            v.push(b"\0TITLE \"1D\"\nLUT_1D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n1 1 1\n".to_vec());
            v.push([&[1u8][..], lut_3dl().as_bytes()].concat());
            v.push([&[2u8][..], lut_look().as_bytes()].concat());
            v
        }
        _ => Vec::new(),
    }
}

const TARGETS: &[&str] = &["icc_profile", "lut_file"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let targets: Vec<&str> = if args.is_empty() { TARGETS.to_vec() } else { args.iter().map(String::as_str).collect() };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    for t in targets {
        let seeds = seeds(t);
        if seeds.is_empty() {
            println!("{t}: no generated seeds");
            continue;
        }
        let dir = root.join(t);
        std::fs::create_dir_all(&dir).expect("create the corpus folder");
        for (i, seed) in seeds.iter().enumerate() {
            let parsed = match t {
                "icc_profile" => Profile::parse(seed).map(|_| ()).map_err(|e| e.to_string()),
                _ => {
                    let (pick, bytes) = seed.split_first().expect("a LUT seed has its pick byte");
                    let name = ["seed.cube", "seed.3dl", "seed.look"][usize::from(*pick % 3)];
                    lutfile::parse(name, bytes).map(|_| ()).map_err(|e| e.0)
                }
            };
            if let Err(e) = parsed {
                panic!("{t}: seed {i} is refused: {e}");
            }
            std::fs::write(dir.join(format!("seed-{i}")), seed).expect("write a seed");
        }
        println!("{t}: {} seeds", seeds.len());
    }
}
