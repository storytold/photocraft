use super::*;
use photocraft_doc::{
    LayerContent,
    import_conversion::{ConversionDisposition, ImportConversion},
};
use serde_json::json;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

// A one-pixel linear-F32 XCF26, independently framed here. Its top-first
// contrast/exposure list must execute Exposure before native contrast.
fn fixture() -> Vec<u8> {
    struct W(Vec<u8>);
    impl W {
        fn word(&mut self, v: u32) {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        fn pointer(&mut self, v: usize) {
            self.0.extend_from_slice(&(v as u64).to_be_bytes());
        }
        fn slot(&mut self) -> usize {
            let at = self.0.len();
            self.pointer(0);
            at
        }
        fn patch(&mut self, at: usize) {
            let value = self.0.len() as u64;
            self.0[at..at + 8].copy_from_slice(&value.to_be_bytes());
        }
        fn text(&mut self, v: &str) {
            self.word(v.len() as u32 + 1);
            self.0.extend_from_slice(v.as_bytes());
            self.0.push(0);
        }
        fn property(&mut self, kind: u32, data: &[u8]) {
            self.word(kind);
            self.word(data.len() as u32);
            self.0.extend_from_slice(data);
        }
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
    }
    let mut w = W(b"gimp xcf v026\0".to_vec());
    for value in [1, 1, 0, 600] {
        w.word(value);
    }
    w.property(17, &[0]);
    w.end();
    let owner = w.slot();
    for _ in 0..3 {
        w.pointer(0);
    }
    w.patch(owner);
    for value in [1, 1, 1] {
        w.word(value);
    }
    w.text("Tone owner");
    w.end();
    let hierarchy = w.slot();
    w.pointer(0);
    let contrast = w.slot();
    let exposure = w.slot();
    w.pointer(0);
    w.patch(hierarchy);
    for value in [1, 1, 16] {
        w.word(value);
    }
    let level = w.slot();
    w.pointer(0);
    w.patch(level);
    w.word(1);
    w.word(1);
    let tile = w.slot();
    w.pointer(0);
    w.patch(tile);
    for value in [0.25f32, 0.5, 2., 1.] {
        w.0.extend_from_slice(&value.to_be_bytes());
    }
    fn effect(w: &mut W, operation: &str, version: &str, name: &str, value: f32) {
        for value in ["Tone", "", operation, version] {
            w.text(value);
        }
        let mut argument = W(Vec::new());
        argument.text(name);
        argument.word(3);
        argument.0.extend_from_slice(&value.to_be_bytes());
        w.property(45, &argument.0);
        w.property(7, &62u32.to_be_bytes());
        w.property(8, &1u32.to_be_bytes());
        w.property(33, &1f32.to_be_bytes());
        w.end();
        w.pointer(0);
    }
    w.patch(contrast);
    effect(&mut w, "gegl:brightness-contrast", "0:0", "contrast", 0.5);
    w.patch(exposure);
    effect(&mut w, "gegl:exposure", "1:0", "exposure", 1.);
    w.0
}
fn session() -> crate::Session {
    let mut session = crate::session();
    session.execute("edit.colorSettings", json!({"askOnMismatch":false,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
    session
}

#[test]
fn authorized_binary_graph_renders_from_native_source_after_reopen() {
    let mut session = session();
    crate::file_cmds::open_bytes_as(&mut session, "binary-over.xcf", &photocraft_io::native_xcf_tones::fixtures::binary_over(), Some("xcf"), None).unwrap();
    let doc = &session.active().unwrap().doc;
    let saved = photocraft_format::save_to_bytes(doc, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
    let LayerContent::Smart(smart) = &reopened.layers[0].content else { panic!("native binary graph owner") };
    assert!(smart.smart_filters.is_empty());
    let expected = smart.cache.as_ref().unwrap().rgba(0, 0);
    let mut source_only = smart.clone();
    source_only.cache = None;
    let result = crate::smart_cmds::render(&reopened, &source_only).unwrap().unwrap().rgba(0, 0);
    for (actual, expected) in result.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.000002, "{actual} versus {expected}");
    }
    assert!(result[0] > 0.7 && result[2] < 0.1, "preserve signed/HDR compositing and the selected branch order: {result:?}");
    assert!((result[3] - 0.75).abs() < 0.000001);
}
fn deny_contrast(command: &str, _: &Value) -> Result<()> {
    if command == "image.adjustments.brightnessContrast" { Err(EngineError::Other("contrast request denied".into())) } else { Ok(()) }
}
fn cancel_contrast(command: &str, _: &Value) -> Result<()> {
    if command == "image.adjustments.brightnessContrast" { Err(EngineError::Cancelled) } else { Ok(()) }
}
fn assert_native(session: &crate::Session) {
    let document = &session.active().unwrap().doc;
    let LayerContent::Smart(smart) = &document.layers[0].content else { panic!("authorized source must install native controls") };
    assert_eq!(smart.smart_filters.len(), 2);
    assert_eq!(smart.smart_filters[0].command, "image.adjustments.exposure");
    assert_eq!(smart.smart_filters[1].command, "image.adjustments.brightnessContrast");
    let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("native mapping report missing") };
    assert!(report.entries.iter().all(|entry| entry.disposition == ConversionDisposition::NativeApproximation));
    // The generated native source and filter stack must be independently
    // re-renderable by the existing engine, not merely display a supplied cache.
    let rendered = crate::smart_cmds::render(document, smart).unwrap().unwrap();
    let expected = smart.cache.as_ref().unwrap().rgba(0, 0);
    let actual = rendered.rgba(0, 0);
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.000001, "{actual} vs {expected}");
    }
    assert!(session.active().unwrap().source_read_only);
}

#[test]
fn trusted_sync_open_and_open_as_enter_only_the_explicit_import_scope() {
    let bytes = fixture();
    for forced in [false, true] {
        let mut session = session();
        let before = photocraft_io::native_xcf_tones::preparation_count();
        super::super::open_bytes_as(&mut session, if forced { "source.bin" } else { "source.xcf" }, &bytes, forced.then_some("xcf"), None).unwrap();
        assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before + 1);
        assert_native(&session);
        let direct = photocraft_io::import("direct.xcf", &bytes).unwrap();
        assert!(matches!(direct.document.layers[0].content, LayerContent::Raster(_)), "the completed engine request must not leave IO authority installed");
    }
}

#[test]
fn denied_and_cancelled_native_commands_never_materialize_or_publish() {
    let bytes = fixture();
    for (gate, cancelled) in [(deny_contrast as Gate, false), (cancel_contrast as Gate, true)] {
        let mut session = session();
        session.authorize = Some(gate);
        let before = photocraft_io::native_xcf_tones::preparation_count();
        let error = super::super::open_bytes_as(&mut session, "source.xcf", &bytes, None, None).unwrap_err();
        if cancelled {
            assert_eq!(error, EngineError::Cancelled);
        } else {
            assert!(error.to_string().contains("not authorized"), "{error}");
        }
        assert!(session.documents().is_empty());
        assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before, "authorize every command before the first stage is evaluated");
    }
}

#[test]
fn trusted_nested_open_inherits_an_existing_request_restriction() {
    let bytes = fixture();
    let mut session = session();
    {
        let _outer = crate::jobs::render_authorization_scope(Some(deny_contrast));
        let direct = photocraft_io::import("direct.xcf", &bytes).unwrap();
        assert!(matches!(direct.document.layers[0].content, LayerContent::Raster(_)), "generic render/edit authority alone must not activate IO");
        assert!(super::super::open_bytes_as(&mut session, "source.xcf", &bytes, None, None).is_err());
        assert!(session.documents().is_empty());
    }
    super::super::open_bytes_as(&mut session, "source.xcf", &bytes, None, None).unwrap();
    assert_native(&session);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn open_worker_reinstalls_captured_policy_for_bytes_and_admitted_paths() {
    use crate::jobs::{JobOutcome, OpenSource, Started};
    use std::time::{Duration, Instant};
    let bytes = fixture();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("source.xcf");
    std::fs::write(&path, &bytes).unwrap();
    for use_path in [false, true] {
        for denied in [false, true] {
            let mut session = session();
            if denied {
                session.authorize = Some(deny_contrast);
            }
            let source = if use_path { OpenSource::Path(path.to_string_lossy().into_owned()) } else { OpenSource::Bytes(Arc::new(bytes.clone())) };
            let Started::Job(job) = session.start_open("source.xcf", source).unwrap() else { panic!("native worker expected") };
            if denied {
                session.authorize = None;
            } // a later caller cannot weaken captured work
            let deadline = Instant::now() + Duration::from_secs(60);
            let outcome = loop {
                if let Some(event) = session.poll_jobs().into_iter().find(|event| event.id == job) {
                    break event.outcome;
                }
                assert!(Instant::now() < deadline, "native open worker did not finish");
                std::thread::sleep(Duration::from_millis(2));
            };
            if denied {
                assert!(matches!(outcome, JobOutcome::Failed(_)));
                assert!(session.documents().is_empty());
            } else {
                assert!(matches!(outcome, JobOutcome::Done(_)), "{outcome:?}");
                assert_native(&session);
                if use_path {
                    assert_eq!(session.active().unwrap().protected_import_source.as_deref(), path.to_str());
                }
            }
        }
    }
}

#[test]
fn ordinary_svg_import_remains_unchanged_inside_the_explicit_open_scope() {
    let bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#00ff00"/></svg>"##;
    let direct = photocraft_io::import("ordinary.svg", bytes).unwrap();
    let expected = photocraft_compose::flatten(&direct.document).into_px();
    let mut session = session();
    session.authorize = Some(deny_contrast);
    let before = photocraft_io::native_xcf_tones::preparation_count();
    super::super::open_bytes_as(&mut session, "ordinary.svg", bytes, None, None).unwrap();
    assert_eq!(photocraft_compose::flatten(&session.active().unwrap().doc).into_px(), expected);
    assert_eq!(photocraft_io::native_xcf_tones::preparation_count(), before);
}

mod raster_place_tests;
