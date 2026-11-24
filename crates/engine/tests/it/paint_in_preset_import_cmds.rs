//! Tests that run paint commands, moved from the engine-core `preset_import_cmds` tests: 3 test(s).

use craft_psd::abr::{AbrSample, LegacyBrush, LegacyTip, write_v6, write_v12};
use photocraft_engine::Session;
use serde_json::json;

use craft_psd::descriptor::{Descriptor, UnicodeString, Value as DV};

fn abr_v2() -> Vec<u8> {
    let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: vec![255; 24] };
    write_v12(
        2,
        &[
            LegacyBrush { name: "Hard Round".into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip) },
            LegacyBrush {
                name: String::new(),
                spacing: 10,
                anti_alias: true,
                tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
            },
        ],
        true,
    )
    .unwrap()
}

#[test]
fn import_adds_a_group_and_paints() {
    let mut s = Session::new();
    let before = s.tools.presets.len();
    let data = photocraft_paint::tile::b64_encode(&abr_v2());
    let r = s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy", "select": true})).unwrap();
    assert_eq!(r["count"], 2);
    // The name clashing with the built-in Hard Round is made unique.
    assert_eq!(r["imported"], json!(["Hard Round 2", "9 px Round"]));
    assert_eq!(s.tools.presets.len(), before + 2);
    assert!(s.tools.presets.iter().filter(|p| p.group == "Legacy").all(|p| !p.builtin));
    assert!(matches!(*s.tools.brush.tip, photocraft_paint::TipShape::Sampled(_)));
    // Re-import with `replace` replaces the group.
    s.execute("brush.presets.importAbr", json!({"data": data, "group": "Legacy", "replace": true})).unwrap();
    assert_eq!(s.tools.presets.len(), before + 2);
    // Imported presets paint.
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("paint.stroke", json!({"points": [[5, 30], [60, 30]], "preset": "9 px Round"})).unwrap();
    // From a file on disk, the group defaults to the file name.
    let dir = std::env::temp_dir().join(format!("pc-abr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("My Set.abr");
    let preset = Descriptor::new("brushPreset")
        .with("Nm  ", DV::Text(UnicodeString::new_nul("Speckle")))
        .with("Brsh", DV::Descriptor(Descriptor::new("sampledBrush").with("sampledData", DV::Text(UnicodeString::new_nul("$s")))));
    let tip = AbrSample { id: "$s".into(), width: 5, height: 5, depth: 16, data: vec![0x80; 50] };
    std::fs::write(&path, write_v6(2, &[tip], &[], &[preset], true).unwrap()).unwrap();
    let r = s.execute("brush.presets.importAbr", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!((r["group"].as_str(), r["imported"][0].as_str(), r["version"].as_u64()), (Some("My Set"), Some("Speckle"), Some(6)));
    std::fs::remove_dir_all(&dir).ok();
}

/// A version 1 brush with a computed tip and no name is named after its tip ("9 px Round"). The
/// import used to number each repeated name by searching the preset names once for every number
/// it tried, so importing 10,000 such brushes took minutes. The test fails when the import takes
/// more than 20 seconds.
#[test]
fn repeated_brush_names_are_numbered_in_linear_time() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = Session::new();
        s.execute("brush.presets.save", json!({"name": "9 PX ROUND 3"})).unwrap();
        let round = LegacyBrush {
            name: String::new(),
            spacing: 25,
            anti_alias: true,
            tip: LegacyTip::Computed { diameter: 9, hardness: 100, angle: 0, roundness: 100 },
        };
        let data = photocraft_paint::tile::b64_encode(&write_v12(1, &vec![round; 10_000], false).unwrap());
        let r = s.execute("brush.presets.importAbr", json!({"data": data})).unwrap();
        let names: Vec<String> = r["imported"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        let _ = tx.send(names);
    });
    let names = match rx.recv_timeout(std::time::Duration::from_secs(20)) {
        Ok(names) => names,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!("importing the brushes did not finish"),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("the import thread panicked"),
    };
    // Numbering starts at 2 and skips the number a preset already uses, in any letter case.
    assert_eq!(names[..4], ["9 px Round", "9 px Round 2", "9 px Round 4", "9 px Round 5"]);
    assert_eq!(names.last(), Some(&format!("9 px Round {}", names.len() + 1)));
    assert_eq!(names.iter().collect::<std::collections::HashSet<_>>().len(), names.len());
}

#[test]
fn repeated_brush_names_take_the_first_free_number() {
    let mut s = Session::new();
    s.execute("brush.presets.save", json!({"name": "speckle 3"})).unwrap();
    let tip = AbrSample { id: String::new(), width: 6, height: 4, depth: 8, data: vec![255; 24] };
    let brush = |name: &str| LegacyBrush { name: name.into(), spacing: 25, anti_alias: true, tip: LegacyTip::Sampled(tip.clone()) };
    let file = write_v12(2, &["Speckle", "SPECKLE", "speckle", "Speckle 2", "Speckle", "hard round"].map(brush), true).unwrap();
    let r = s.execute("brush.presets.importAbr", json!({"data": photocraft_paint::tile::b64_encode(&file)})).unwrap();
    // Names that differ only in letter case count as the same name. The file's "Speckle 2" comes
    // after "SPECKLE" took that name, and "hard round" clashes with the built-in Hard Round.
    assert_eq!(r["imported"], json!(["Speckle", "SPECKLE 2", "speckle 4", "Speckle 2 2", "Speckle 5", "hard round 2"]));
}
