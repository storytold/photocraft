//! Writes a starting corpus for this crate's fuzz targets from the example Invert plug-in the
//! crate's own tests use (`tests/fixtures/invert.wasm`, built from `examples/plugins/invert-rs`), so
//! libFuzzer starts from a module and manifest that load. Each seed must load, or this fails.
//!
//! `cargo run --example seeds -- [target ...]` (no target: all) writes `corpus/<target>/seed-*`.

use std::path::Path;

use photocraft_plugins::{Limits, Manifest, Plugin};

const INVERT: &[u8] = include_bytes!("../../tests/fixtures/invert.wasm");

/// The seeds of one target.
fn seeds(target: &str) -> Vec<Vec<u8>> {
    match target {
        "module" => vec![INVERT.to_vec()],
        "manifest" => {
            let plugin = Plugin::load(INVERT, Limits::default()).unwrap_or_else(|e| panic!("the Invert fixture does not load: {e}"));
            let own = serde_json::to_vec(plugin.manifest()).expect("a manifest serialises");
            let params = br#"{"id":"seed.params","name":"Seed","version":"1.0","kind":"filter","params":{"amount":{"type":"number","min":0,"max":100,"default":12,"unit":"px"},"steps":{"type":"int","min":1,"max":16,"default":4},"mode":{"type":"choice","options":["soft","hard"],"default":"hard"},"mono":{"type":"bool"}}}"#;
            vec![own, params.to_vec()]
        }
        _ => Vec::new(),
    }
}

const TARGETS: &[&str] = &["module", "manifest"];

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
            let loaded = match t {
                "manifest" => Manifest::parse(seed).map(|_| ()),
                _ => Plugin::load(seed, Limits::default()).map(|_| ()),
            };
            if let Err(e) = loaded {
                panic!("{t}: seed {i} is refused: {e}");
            }
            std::fs::write(dir.join(format!("seed-{i}")), seed).expect("write a seed");
        }
        println!("{t}: {} seeds", seeds.len());
    }
}
