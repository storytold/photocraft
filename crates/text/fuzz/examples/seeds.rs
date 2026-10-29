//! Writes a starting corpus for this crate's fuzz targets: the bundled Inter and JetBrains Mono
//! (OFL, `assets/fonts`) for the font target, and type layers built by our own `TySh` writer for
//! the type target. Each seed must load, or this fails: a refused seed only teaches the fuzzer the
//! error path.
//!
//! `cargo run --example seeds -- [target ...]` (no target: all) writes `corpus/<target>/seed-*`.

use std::path::Path;

use photocraft_doc::TextLayer;
use photocraft_text::{fonts, psd};

/// The seeds of one target.
fn seeds(target: &str) -> Vec<Vec<u8>> {
    match target {
        "font_bytes" => fonts::BUNDLED.iter().map(|(_, bytes)| bytes.to_vec()).collect(),
        "type_layer" => {
            let point = TextLayer { text: "Seed".into(), font_family: "Inter".into(), size_pt: 12.0, ..Default::default() };
            let lines = TextLayer { text: "Two\rlines \u{e9}".into(), font_family: "JetBrains Mono".into(), size_pt: 30.0, ..Default::default() };
            let mut v: Vec<Vec<u8>> = [point, lines].iter().map(|l| psd::build_tysh(l, 72.0, Some([1.0, 0.0, 0.0, 1.0]))).collect();
            // The EngineData inside, on its own, for the parser path that doesn't start at `TySh`.
            v.push(b"\n\n<<\n\t/EngineDict\n\t<<\n\t\t/Editor\n\t\t<<\n\t\t\t/Text (\xfe\xff\x00S\x00e\x00e\x00d\x00\r)\n\t\t>>\n\t\t/FontSet [ << /Name (\xfe\xff\x00I\x00n\x00t\x00e\x00r) /Type 1 >> ]\n\t\t/Sizes [ 12.0 .5 -3 ]\n\t\t/On true\n\t>>\n>>\n".to_vec());
            v
        }
        _ => Vec::new(),
    }
}

const TARGETS: &[&str] = &["font_bytes", "type_layer"];

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
            let loads = match t {
                "font_bytes" => fonts::face_count(seed) > 0,
                _ => psd::text_layer_from_tysh(seed, 72.0).is_some() || photocraft_text::engine_data::parse(seed).is_ok(),
            };
            assert!(loads, "{t}: seed {i} is refused");
            std::fs::write(dir.join(format!("seed-{i}")), seed).expect("write a seed");
        }
        println!("{t}: {} seeds", seeds.len());
    }
}
