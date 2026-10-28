//! Sensor imports for the 20 former integration gaps, opt-in without enlarging CI downloads.
#![cfg(all(feature = "performance-corpus", not(target_arch = "wasm32")))]
use photocraft_color::{ColorMode, SampleType};
use std::path::PathBuf;

#[test]
fn formerly_unsupported_formats_reach_full_sensor_document_import() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let history = std::fs::read_to_string(root.join("perf/raw-decode/m1-pro-2026-10-28/results.jsonl")).unwrap();
    let mut count = 0;
    for line in history.lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        let baseline = case["benchmark"]["results"].as_array().unwrap().iter().find(|r| r["backend"] == "auto").unwrap();
        if baseline["status"] == "ok" {
            continue;
        }
        count += 1;
        let path = case["sample"]["path"].as_str().unwrap();
        let bytes = std::fs::read(root.join(path)).unwrap();
        let sensor = photocraft_raw::decode(&bytes, &photocraft_raw::Limits::default()).unwrap();
        let mut expected = (sensor.crop.width as u32, sensor.crop.height as u32);
        if (5..=8).contains(&sensor.orientation) {
            expected = (expected.1, expected.0);
        }
        drop(sensor);
        let imported = photocraft_io::import(path, &bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!((imported.document.size.width, imported.document.size.height), expected, "{path}");
        assert_eq!(imported.document.depth, SampleType::U16, "{path}");
        assert_eq!(imported.document.mode, ColorMode::Rgb, "{path}");
        assert!(imported.warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "{path}");
        assert!(imported.warnings.iter().all(|w| !w.contains("preview instead")), "{path}");
        let profile = imported.document.icc_profile.clone();
        drop(imported);
        let (tuned, _) = photocraft_io::raw::import_raw_tuned(path, &bytes, &photocraft_io::raw::RawTuning { exposure: 0.5, ..Default::default() })
            .unwrap_or_else(|e| panic!("{path}, Camera Raw redevelop: {e}"));
        assert_eq!((tuned.document.size.width, tuned.document.size.height), expected, "{path}");
        assert_eq!(tuned.document.depth, SampleType::U16, "{path}");
        assert_eq!(tuned.document.icc_profile, profile, "{path}");
    }
    assert_eq!(count, 20);
}
