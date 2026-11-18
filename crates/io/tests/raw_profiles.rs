//! Native DCP selection, rendering and provenance through the public import API.
#![cfg(not(target_arch = "wasm32"))]
use photocraft_io::raw::{RawTuning, import_raw, import_raw_tuned, profile_info};
use photocraft_raw::testgen::{DngSpec, mosaic, scene};

/// Build a small synthetic CFA DNG with enough scene variation to compare developed pixels.
fn fixture(model: &str) -> Vec<u8> {
    let mut spec = DngSpec::cfa(32, 24, mosaic(&scene(32, 24), 32, [0, 1, 1, 2], 0, 4095));
    spec.bits = 12;
    spec.white = 4095;
    spec.model = model.into();
    spec.build()
}

/// Keep default and tuned imports pixel-identical and preserve CC0 provenance through native saves.
#[test]
fn cc0_profile_is_shared_by_default_open_and_raw_tuning_and_survives_save() {
    let bytes = fixture("Canon EOS RP");
    let opened = import_raw("synthetic.dng", &bytes).unwrap();
    let info = profile_info(&opened.document).unwrap();
    assert_eq!(info["source"], "bundled-dcp");
    assert_eq!(info["license"], "CC0-1.0");
    assert_eq!(info["copyright"], "RawTherapee CC0");
    let (tuned, _) = import_raw_tuned("synthetic.dng", &bytes, &RawTuning::default()).unwrap();
    assert_eq!(profile_info(&tuned.document), Some(info.clone()));
    assert_eq!(photocraft_compose::flatten(&opened.document).px, photocraft_compose::flatten(&tuned.document).px);
    let encoded = photocraft_io::export(&opened.document, "profile.pcraft", &Default::default()).unwrap();
    let restored = photocraft_io::import("profile.pcraft", &encoded.bytes).unwrap();
    assert_eq!(profile_info(&restored.document), Some(info));
}

/// Leave sensor calibration intact for excluded, fictional and unknown camera models.
#[test]
fn excluded_and_unknown_models_keep_existing_calibration_without_substitution() {
    for model in ["NIKON Z f", "Canon EOS RP fictional successor", "Unknown synthetic body"] {
        let opened = import_raw("synthetic.dng", &fixture(model)).unwrap();
        assert!(profile_info(&opened.document).is_none(), "{model}");
    }
}
