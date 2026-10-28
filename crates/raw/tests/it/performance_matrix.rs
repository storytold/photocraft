//! Opt-in coverage for the full SHA-pinned benchmark matrix, including all 20
//! formerly failing integration cases. Ordinary CI corpus downloads stay small.
#![cfg(all(feature = "performance-corpus", feature = "flashraw", not(target_arch = "wasm32")))]
use photocraft_raw::{DecodeBackend, Limits, decode_with_backend_traced, is_raw};
use serde_json::Value;
use std::path::PathBuf;

#[test]
fn every_camera_original_decodes_sensor_data_and_identifies_without_its_filename() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("perf/raw-decode/samples.json")).unwrap()).unwrap();
    let cases = manifest["samples"].as_array().unwrap();
    assert_eq!(cases.len(), 88);
    for case in cases {
        let path = case["path"].as_str().unwrap();
        let bytes = std::fs::read(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}; run python3 perf/raw-decode/run.py first"));
        assert_eq!(bytes.len() as u64, case["bytes"].as_u64().unwrap());
        assert!(is_raw(&bytes), "{path} must reach RAW import without relying on its extension");
        let (sensor, backend) = decode_with_backend_traced(&bytes, &Limits::default(), DecodeBackend::Auto).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(matches!(backend, DecodeBackend::Builtin | DecodeBackend::FlashRaw));
        assert_eq!(sensor.width.checked_mul(sensor.height).and_then(|n| n.checked_mul(sensor.samples)), Some(sensor.data.len()), "{path}");
        assert!(!sensor.crop.is_empty(), "{path}");
        assert_eq!(sensor.crop.intersect(&sensor.active), sensor.crop, "{path}");
        assert!(sensor.data.iter().any(|v| *v > 0), "{path}: sensor cannot be an empty placeholder");
        assert!(sensor.white.iter().all(|v| v.is_finite() && *v > 0.0), "{path}");
        assert!(sensor.camera_wb.is_none_or(|v| v.iter().all(|v| v.is_finite() && *v > 0.0)), "{path}");
    }
}
