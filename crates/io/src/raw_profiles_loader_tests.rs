//! Synthetic installed-profile tests use explicit sources instead of mutating process environment.
#![allow(clippy::unwrap_used)] // Fixture setup failures should fail this test-only module.

use super::*;
use photocraft_raw::{Limits, RawFormat, decode, testgen::DngSpec};
use std::path::PathBuf;

/// Isolated profile fixtures that clean themselves up without changing process environment.
struct ProfilesDir(PathBuf);

impl ProfilesDir {
    /// Create a unique directory so parallel loader tests cannot share fixtures or cache keys.
    fn new() -> Self {
        static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("photocraft-profile-test-{}-{time}-{serial}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    /// Build catalog metadata, optionally writing bytes or leaving the path missing for failure tests.
    fn entry(&self, file: &str, name: &str, bytes: Option<&[u8]>) -> catalog::Entry {
        let path = self.0.join(file);
        if let Some(bytes) = bytes {
            std::fs::write(&path, bytes).unwrap();
        }
        catalog::Entry { path, model: "Canon EOS RP".into(), name: name.into() }
    }
}

impl Drop for ProfilesDir {
    /// Remove temporary fixtures even when an assertion unwinds the test.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Decode an original CFA fixture and mark it as NEF to exercise DNG-only profile restrictions.
fn sensor(model: &str) -> Sensor {
    let mut spec = DngSpec::cfa(8, 8, vec![512; 64]);
    spec.model = model.into();
    let mut sensor = decode(&spec.build(), &Limits::default()).unwrap();
    sensor.format = RawFormat::Nef;
    sensor
}

/// Select requested and base Picture Controls while leaving manual offsets unspecified.
fn settings(name: &str, base: &str) -> CameraSettings {
    CameraSettings {
        make: "Canon".into(),
        model: "Canon EOS RP".into(),
        color_space: Some(1),
        picture_control: Some(photocraft_raw::PictureControl {
            version: "0310".into(),
            name: name.into(),
            base: base.into(),
            adjustment: 1,
            contrast_code: 255,
            brightness_code: 255,
            saturation_code: 255,
            hue_code: 255,
            filter_effect_code: 255,
            toning_effect_code: 255,
        }),
    }
}

/// Minimal original DCP fixture: identity calibration, explicit usage policy, no vendor tables.
fn dcp(model: &str, name: &str, policy: u32) -> Vec<u8> {
    let matrix: Vec<_> = [1i32, 0, 0, 0, 1, 0, 0, 0, 1].into_iter().flat_map(|v| [v.to_le_bytes(), 1i32.to_le_bytes()].concat()).collect();
    let tags = vec![
        (50708u16, 2u16, model.len() as u32 + 1, [model.as_bytes(), &[0]].concat()),
        (50721, 10, 9, matrix),
        (50778, 3, 1, 21u16.to_le_bytes().to_vec()),
        (50936, 2, name.len() as u32 + 1, [name.as_bytes(), &[0]].concat()),
        (50941, 4, 1, policy.to_le_bytes().to_vec()),
        (51110, 4, 1, 1u32.to_le_bytes().to_vec()),
    ];
    let mut out = b"IIRC".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend((tags.len() as u16).to_le_bytes());
    let directory = out.len();
    out.resize(directory + tags.len() * 12 + 4, 0);
    for (i, (tag, typ, count, bytes)) in tags.into_iter().enumerate() {
        let entry = directory + i * 12;
        out[entry..entry + 2].copy_from_slice(&tag.to_le_bytes());
        out[entry + 2..entry + 4].copy_from_slice(&typ.to_le_bytes());
        out[entry + 4..entry + 8].copy_from_slice(&count.to_le_bytes());
        if bytes.len() <= 4 {
            out[entry + 8..entry + 8 + bytes.len()].copy_from_slice(&bytes);
        } else {
            let offset = out.len() as u32;
            out[entry + 8..entry + 12].copy_from_slice(&offset.to_le_bytes());
            out.extend(bytes);
        }
    }
    out
}

/// Exhaust same-name candidates before falling back, retaining diagnostics without absolute paths.
#[test]
fn automatic_selection_tries_every_requested_profile_before_lower_priorities() {
    let dir = ProfilesDir::new();
    let base = dir.entry("base.dcp", "Camera Neutral", Some(&dcp("Canon EOS RP", "Camera Neutral", 3)));
    let missing = dir.entry("missing.dcp", "Camera Vivid", None);
    let broken = dir.entry("broken.dcp", "Camera Vivid", Some(b"broken"));
    let good = dir.entry("good.dcp", "Camera Vivid", Some(&dcp("Canon EOS RP", "Camera Vivid", 3)));
    let loaded = load_installed(&sensor("Canon EOS RP"), Some(&settings("Vivid", "Neutral")), None, || vec![base, missing, broken, good]).unwrap().unwrap();
    assert_eq!(loaded.report["file"], "good.dcp");
    assert_eq!(loaded.report["source"], "external-dcp");
    let report = loaded.report.to_string();
    assert!(report.contains("missing.dcp") && report.contains("broken.dcp"));
    assert!(!report.contains(dir.0.to_str().unwrap()));
}

/// Preserve requested/base/standard ordering when corrupt or incompatible candidates are rejected.
#[test]
fn automatic_selection_preserves_picture_control_priority_after_rejections() {
    let dir = ProfilesDir::new();
    let others = dir.entry("other.dcp", "Other Look", Some(&dcp("Canon EOS RP", "Other Look", 3)));
    let adobe = dir.entry("adobe.dcp", "Adobe Standard", Some(&dcp("Canon EOS RP", "Adobe Standard", 3)));
    let standard = dir.entry("wrong-model.dcp", "Camera Standard", Some(&dcp("Other Camera", "Camera Standard", 3)));
    let base = dir.entry("dng-only.dcp", "Camera Neutral", Some(&dcp("Canon EOS RP", "Camera Neutral", 0)));
    let requested = dir.entry("corrupt.dcp", "Camera Vivid", Some(b"corrupt"));
    let loaded =
        load_installed(&sensor("Canon EOS RP"), Some(&settings("Vivid", "Neutral")), None, || vec![others, adobe, standard, base, requested]).unwrap().unwrap();
    assert_eq!(loaded.report["file"], "adobe.dcp");
    let report = loaded.report.to_string();
    assert!(report.contains("does not match") && report.contains("DNG input only"));
    assert!(report.contains("corrupt.dcp"));
    assert!(!report.contains(dir.0.to_str().unwrap()));
}

/// Fall back to the bundle after missing, malformed, incompatible, non-file and oversized candidates.
#[test]
fn automatic_selection_falls_back_to_bundle_after_all_file_failure_kinds() {
    let dir = ProfilesDir::new();
    let mut entries = vec![
        dir.entry("missing.dcp", "Camera Standard", None),
        dir.entry("broken.dcp", "Camera Standard", Some(b"broken")),
        dir.entry("wrong-model.dcp", "Camera Standard", Some(&dcp("Other Camera", "Camera Standard", 3))),
        dir.entry("dng-only.dcp", "Camera Standard", Some(&dcp("Canon EOS RP", "Camera Standard", 0))),
    ];
    let directory = dir.entry("directory.dcp", "Camera Standard", None);
    std::fs::create_dir(&directory.path).unwrap();
    entries.push(directory);
    let oversized = dir.entry("oversized.dcp", "Camera Standard", Some(b""));
    std::fs::File::options().write(true).open(&oversized.path).unwrap().set_len(32 * 1024 * 1024 + 1).unwrap();
    entries.push(oversized);
    let loaded = load_installed(&sensor("Canon EOS RP"), None, None, || entries).unwrap().unwrap();
    assert_eq!(loaded.report["source"], "bundled-dcp");
    let report = loaded.report.to_string();
    for name in ["missing.dcp", "broken.dcp", "wrong-model.dcp", "dng-only.dcp", "directory.dcp", "oversized.dcp"] {
        assert!(report.contains(name), "{report}");
    }
    assert!(!report.contains(dir.0.to_str().unwrap()));
}

/// Honor explicit selection failures without scanning the catalog or silently substituting a profile.
#[test]
fn explicit_override_errors_are_not_hidden_by_installed_or_bundled_fallback() {
    let dir = ProfilesDir::new();
    for entry in [
        dir.entry("missing.dcp", "Camera Standard", None),
        dir.entry("broken.dcp", "Camera Standard", Some(b"broken")),
        dir.entry("wrong-model.dcp", "Camera Standard", Some(&dcp("Other Camera", "Camera Standard", 3))),
        dir.entry("dng-only.dcp", "Camera Standard", Some(&dcp("Canon EOS RP", "Camera Standard", 0))),
    ] {
        let result = load_installed(&sensor("Canon EOS RP"), None, Some(&entry.path), || panic!("explicit override must not scan the catalog"));
        assert!(result.is_err(), "{}", entry.path.display());
    }
    let valid = dir.entry("valid.dcp", "Camera Standard", Some(&dcp("Canon EOS RP", "Camera Standard", 3)));
    let loaded = load_installed(&sensor("Canon EOS RP"), None, Some(&valid.path), || panic!("explicit override must not scan the catalog")).unwrap().unwrap();
    assert_eq!(loaded.report["source"], "external-dcp");
}

/// Preserve explicit override errors even when model metadata would disable automatic discovery.
#[test]
fn explicit_override_errors_are_not_hidden_when_sensor_model_is_missing_or_unsafe() {
    let dir = ProfilesDir::new();
    let missing = dir.entry("missing.dcp", "Camera Standard", None);
    let valid = dir.entry("valid.dcp", "Camera Standard", Some(&dcp("Canon EOS RP", "Camera Standard", 3)));
    let mut sensor = sensor("Canon EOS RP");
    for model in [None, Some("../unsafe-model".into())] {
        sensor.model = model;
        for entry in [&missing, &valid] {
            let result = load_installed(&sensor, None, Some(&entry.path), || panic!("explicit override must not scan the catalog"));
            assert!(result.is_err(), "model {:?}, profile {}", sensor.model, entry.path.display());
        }
    }
}

/// Prevent a DNG-compatible cache entry from bypassing validation on a later NEF import.
#[test]
fn cached_profiles_are_revalidated_for_each_sensor_format() {
    let dir = ProfilesDir::new();
    let profile = dir.entry("dng-only.dcp", "Camera Standard", Some(&dcp("Canon EOS RP", "Camera Standard", 0)));
    let mut dng = sensor("Canon EOS RP");
    dng.format = RawFormat::Dng;
    let loaded = load_installed(&dng, None, None, || vec![profile.clone()]).unwrap().unwrap();
    assert_eq!(loaded.report["source"], "external-dcp");
    let loaded = load_installed(&sensor("Canon EOS RP"), None, None, || vec![profile]).unwrap().unwrap();
    assert_eq!(loaded.report["source"], "bundled-dcp");
    assert!(loaded.report.to_string().contains("DNG input only"));
}

/// Preserve the sensor-only path when both installed selection and bundled coverage are absent.
#[test]
fn automatic_failure_without_a_bundle_preserves_the_existing_sensor_calibration_path() {
    let dir = ProfilesDir::new();
    let mut entry = dir.entry("broken.dcp", "Camera Standard", Some(b"broken"));
    entry.model = "Unknown Camera".into();
    assert!(load_installed(&sensor("Unknown Camera"), None, None, || vec![entry]).unwrap().is_none());
}

/// Summarize excess candidate failures while keeping provenance small enough to save and recover.
#[test]
fn many_bad_candidates_keep_provenance_bounded_and_report_the_skipped_count() {
    let dir = ProfilesDir::new();
    let entries = (0..20).map(|i| dir.entry(&format!("missing-{i}.dcp"), "Camera Standard", None)).collect();
    let loaded = load_installed(&sensor("Canon EOS RP"), None, None, || entries).unwrap().unwrap();
    assert!(loaded.report.to_string().contains("12 additional unusable installed camera profiles"));
    let mut doc = photocraft_doc::Document::new(
        "bounded provenance",
        photocraft_doc::Size::new(2, 2),
        photocraft_color::ColorMode::Rgb,
        photocraft_color::SampleType::U16,
    );
    record(&mut doc, &loaded);
    assert_eq!(report(&doc), Some(loaded.report));
}
