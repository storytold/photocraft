//! Profile discovery must not turn scoped RAW bytes into ambient filesystem authority.
#![cfg(not(target_arch = "wasm32"))]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use photocraft_automation::{AuthorizedWorkspace, Headless};
use photocraft_io::raw::{RawTuning, import_raw_tuned, profile_info};
use photocraft_raw::testgen::DngSpec;
use serde_json::json;

struct Fixture(PathBuf);

impl Fixture {
    /// Create separate allowed and outside directories containing a synthetic RAW and matching CC0 DCP.
    fn new(case: &str) -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("pc-raw-authority-{}-{stamp}-{case}", std::process::id()));
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        std::fs::create_dir_all(root.join("outside")).unwrap();
        let mut spec = DngSpec::cfa(16, 12, vec![512; 16 * 12]);
        spec.model = "Canon EOS RP".into();
        std::fs::write(root.join("workspace/synthetic.dng"), spec.build()).unwrap();
        let mut profile = Vec::new();
        flate2::read::GzDecoder::new(include_bytes!("../../../assets/camera-profiles/profiles/Canon EOS RP.dcp.gz").as_slice())
            .read_to_end(&mut profile)
            .unwrap();
        std::fs::write(root.join("outside/private-profile.dcp"), profile).unwrap();
        Self(root)
    }
}

impl Drop for Fixture {
    /// Remove only this test fixture after success or unwinding.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Use child-process configuration so parallel tests never mutate process-global environment.
#[test]
fn scoped_open_place_and_redevelopment_ignore_installed_profiles() {
    for case in ["explicit", "catalog"] {
        let fixture = Fixture::new(case);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--ignored", "--exact", "isolated_profile_authority_child", "--nocapture"]);
        command.env("PHOTOCRAFT_TEST_PROFILE_CASE", case).env("PHOTOCRAFT_TEST_PROFILE_ROOT", &fixture.0);
        command.env_remove("PHOTOCRAFT_CAMERA_PROFILE").env_remove("PHOTOCRAFT_CAMERA_PROFILES_DIR");
        if case == "explicit" {
            command.env("PHOTOCRAFT_CAMERA_PROFILE", fixture.0.join("outside/private-profile.dcp"));
        } else {
            command.env("PHOTOCRAFT_CAMERA_PROFILES_DIR", fixture.0.join("outside"));
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{case}:\n{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
}

/// Read the active document's recorded profile source for the authority assertions.
fn source(headless: &Headless) -> serde_json::Value {
    profile_info(&headless.session.active().unwrap().doc).unwrap()["source"].clone()
}

/// Invoked only by the parent test, with isolated fixture paths and profile configuration.
#[test]
#[ignore = "child process of scoped_open_place_and_redevelopment_ignore_installed_profiles"]
fn isolated_profile_authority_child() {
    let Ok(case) = std::env::var("PHOTOCRAFT_TEST_PROFILE_CASE") else { return };
    let root = PathBuf::from(std::env::var_os("PHOTOCRAFT_TEST_PROFILE_ROOT").unwrap());
    let workspace = root.join("workspace");
    let input = workspace.join("synthetic.dng");
    let bytes = std::fs::read(&input).unwrap();

    // Warm the shared external-profile cache in a trusted session first.
    let mut local = Headless::trusted_local();
    local.open(&input).unwrap();
    assert_eq!(source(&local), "external-dcp");
    local.command_run("file.revert", json!({})).unwrap();
    assert_eq!(source(&local), "external-dcp", "trusted Revert keeps the installed calibration");

    // Engine byte entry points do not grant installed access from optional save metadata.
    for save_path in [None, Some("metadata-only.dng".to_string())] {
        let mut engine = photocraft_engine::Session::new();
        photocraft_engine::file_cmds::open_bytes_as(&mut engine, "memory.dng", &bytes, None, save_path).unwrap();
        assert_eq!(profile_info(&engine.active().unwrap().doc).unwrap()["source"], "bundled-dcp");
    }
    for requested in [None, Some(photocraft_io::CameraProfilePolicy::Installed)] {
        for restricted in [false, true] {
            let mut engine = photocraft_engine::Session::new();
            if restricted {
                engine.authorize = Some(|_, _| Ok(()));
            }
            let source = photocraft_engine::jobs::OpenSource::Bytes(std::sync::Arc::new(bytes.clone()));
            let started = match requested {
                Some(policy) => engine.start_open_with_profile_policy("memory.dng", source, policy),
                None => engine.start_open("memory.dng", source),
            }
            .unwrap();
            engine.authorize = None;
            if let photocraft_engine::jobs::Started::Job(job) = started {
                engine.wait_job(job).unwrap();
            }
            let expected = if requested.is_some() && !restricted { "external-dcp" } else { "bundled-dcp" };
            assert_eq!(profile_info(&engine.active().unwrap().doc).unwrap()["source"], expected);
        }
    }
    local.command_run("file.openAs", json!({"path": input.to_str().unwrap()})).unwrap();
    assert_eq!(source(&local), "external-dcp", "native open-as retains installed-profile authority");

    let mut scoped = Headless::with_workspace(AuthorizedWorkspace::new(Some(&workspace), None).unwrap());
    scoped.open(Path::new("synthetic.dng")).unwrap();
    assert_eq!(source(&scoped), "bundled-dcp");
    let report = profile_info(&scoped.session.active().unwrap().doc).unwrap();
    assert!(!report.to_string().contains("private-profile"));
    assert!(scoped.command_run("raw.profiles", json!({})).is_err());

    // Placement stores original RAW bytes; editing them must keep the same bounded authority.
    let placed = scoped.place(None, "synthetic.dng", &json!({})).unwrap();
    scoped.command_run("layer.smartObjects.editContents", json!({"layer": placed["layer"]})).unwrap();
    assert_eq!(source(&scoped), "bundled-dcp");

    // Byte-only and raw-stage re-development APIs are safe even outside an automation wrapper.
    let imported = photocraft_io::import("synthetic.dng", &bytes).unwrap();
    assert_eq!(profile_info(&imported.document).unwrap()["source"], "bundled-dcp");
    let (tuned, _) = import_raw_tuned("synthetic.dng", &bytes, &RawTuning { exposure: 0.5, ..Default::default() }).unwrap();
    assert_eq!(profile_info(&tuned.document).unwrap()["source"], "bundled-dcp");

    // Retained sensor data must not carry installed authority into a byte-only re-development.
    let decoded = photocraft_io::raw::RawDecode::with_policy(&bytes, photocraft_io::CameraProfilePolicy::Installed).unwrap();
    let (cached, _) = photocraft_io::raw::import_raw_tuned_from("synthetic.dng", &bytes, &RawTuning::default(), &decoded).unwrap();
    assert_eq!(profile_info(&cached.document).unwrap()["source"], "bundled-dcp");
    let opts = photocraft_io::ImportOptions { keep_raw: true, camera_profiles: photocraft_io::CameraProfilePolicy::Installed, ..Default::default() };
    photocraft_io::import_with_options("synthetic.dng", &bytes, &opts).unwrap();
    assert!(photocraft_io::raw::take_kept_decode(&bytes).is_none(), "bundled readers cannot take an installed decode");
    assert!(photocraft_io::raw::take_kept_decode_with_policy(&bytes, photocraft_io::CameraProfilePolicy::Installed).is_some());

    // An unreadable DCP must still be irrelevant to scoped opens; explicit trusted overrides
    // remain hard errors, while automatic discovery falls back to the bundled calibration.
    std::fs::write(root.join("outside/private-profile.dcp"), b"invalid profile").unwrap();
    scoped.open(Path::new("synthetic.dng")).unwrap();
    assert_eq!(source(&scoped), "bundled-dcp");
    if case == "explicit" {
        let count = local.session.documents().len();
        assert!(local.open(&input).is_err());
        assert_eq!(local.session.documents().len(), count);
        assert_eq!(source(&local), "external-dcp");
    } else {
        local.open(&input).unwrap();
        assert_eq!(source(&local), "bundled-dcp");
    }
}
