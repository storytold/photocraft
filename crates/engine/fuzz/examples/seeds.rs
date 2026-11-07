//! Writes a starting corpus for this crate's fuzz targets: files PhotoCraft writes itself plus a
//! few handwritten ones in Photoshop's spelling, so libFuzzer starts from real structure. Each
//! seed must load, or this fails: a seed that is refused only teaches the fuzzer the error path.
//!
//! `cargo run --example seeds -- [target ...]` (no target: all) writes `corpus/<target>/seed-*`.

use std::path::Path;

use photocraft_engine::Session;
use serde_json::json;

/// A Windows Photoshop set: Ctrl is the primary modifier, a tool row and an alternate key.
const PHOTOSHOP_KYS: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/Seed=Seed\">
\t<command kind=\"static\" name=\"Levels...\"><shortcut>Ctrl+Shift+L</shortcut></command>
\t<command kind=\"static\" name=\"Zoom In\"><shortcut>Ctrl++</shortcut><shortcut>Ctrl+=</shortcut></command>
\t<command kind=\"static\" name=\"Gaussian Blur...\"><shortcut>Ctrl+Alt+F9</shortcut></command>
\t<tool name=\"Move Tool\">V</tool>
\t<tool name=\"Brush Tool\">B</tool>
</photoshop-keyboard-shortcuts>
";

const XMP: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="3"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Seed</rdf:li></rdf:Alt></dc:title><dc:subject><rdf:Bag><rdf:li>seed</rdf:li><rdf:li>layer</rdf:li></rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#;

fn kys_seeds() -> Vec<String> {
    let mut s = Session::new();
    let exported = |s: &mut Session, params: serde_json::Value| -> String {
        let reply = s.execute("edit.keyboardShortcuts", params).unwrap_or_else(|e| panic!("Keyboard Shortcuts failed: {e}"));
        reply.get("kys").and_then(|v| v.as_str()).expect("the reply holds the exported set").to_string()
    };
    let defaults = exported(&mut s, json!({ "exportKys": true }));
    let edited = exported(&mut s, json!({ "set": { "image.adjustments.levels": "Cmd+Shift+K" }, "exportKys": true }));
    vec![defaults, edited, PHOTOSHOP_KYS.to_string()]
}

fn prefs_seeds() -> Vec<String> {
    let mut s = Session::new();
    let defaults = s.prefs_to_json();
    // A hand-edited file of an older version: wrong-typed and unknown keys load around.
    let older = r#"{"version":1,"fileHandling":{"recentFileCount":"ten"},"tools":{"useShiftKeyForToolSwitch":false},"unknown":{"x":[1,2,3]}}"#;
    s.load_prefs_json(older).expect("the older preferences seed loads");
    vec![defaults, older.to_string(), s.prefs_to_json()]
}

/// The seeds of one target.
fn seeds(target: &str) -> Vec<String> {
    match target {
        "kys" => kys_seeds(),
        "prefs" => prefs_seeds(),
        "file_info" => vec![
            XMP.to_string(),
            photocraft_engine::file_cmds::write_file_info(
                None,
                &json!({ "title": "Seed", "author": "A; B", "keywords": ["x", "y"], "copyrightStatus": "copyrighted", "copyrightUrl": "https://example.com/" }),
            ),
        ],
        "layer_xmp" => vec![XMP.to_string(), r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"/>"#.to_string()],
        _ => Vec::new(),
    }
}

/// Whether `target` takes `seed`, the way the fuzz target first calls it.
fn loads(target: &str, seed: &str) -> Result<(), String> {
    let mut s = Session::new();
    match target {
        "kys" => s.execute("edit.keyboardShortcuts", json!({ "importKys": seed })).map(|_| ()).map_err(|e| e.to_string()),
        "prefs" => s.load_prefs_json(seed).map(|_| ()),
        "layer_xmp" => {
            s.execute("file.new", json!({ "width": 4, "height": 4 })).map_err(|e| e.to_string())?;
            s.execute("layer.xmpMetadata", json!({ "rawData": seed })).map(|_| ()).map_err(|e| e.to_string())
        }
        // File Info reads any text; a seed only has to hold File Info fields.
        "file_info" => match photocraft_engine::file_cmds::read_file_info(Some(seed)).get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => Ok(()),
            _ => Err("no File Info title in the seed".into()),
        },
        _ => Ok(()),
    }
}

const TARGETS: &[&str] = &["kys", "prefs", "file_info", "layer_xmp"];

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
            if let Err(e) = loads(t, seed) {
                panic!("{t}: seed {i} is refused: {e}");
            }
            std::fs::write(dir.join(format!("seed-{i}")), seed).expect("write a seed");
        }
        println!("{t}: {} seeds", seeds.len());
    }
}
