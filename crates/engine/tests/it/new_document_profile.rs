//! #8832: File > New captures the working profile shown by the New Document dialog.

use photocraft_cms::Builtin;
use photocraft_color::ColorMode;
use photocraft_engine::Session;
use serde_json::{Value, json};

/// Use non-default working spaces, including a synthetic CMYK profile with no external asset.
fn configured_session(directory: &std::path::Path) -> Session {
    let mut cmyk = Builtin::CoatedCmyk.profile().clone();
    cmyk.description = "Synthetic working CMYK (#8832)".into();
    let profile = directory.join("Working CMYK 印刷.icc");
    std::fs::write(&profile, &*cmyk.with_encoded_bytes().to_bytes()).unwrap();
    let mut session = Session::new();
    session.execute("edit.colorSettings", json!({"workingRgb": "display-p3", "workingCmyk": profile, "workingGray": "gray-gamma-2.2"})).unwrap();
    session
}

/// Read the actual ICC bytes stored in the newly created document, rather than a display fallback.
fn embedded(session: &Session) -> Option<&[u8]> {
    session.active().unwrap().doc.icc_profile.as_deref().map(Vec::as_slice)
}

/// Missing and explicit `working` profiles follow each mode's current settings at every depth.
#[test]
fn new_documents_capture_the_working_profile_for_every_mode_and_depth() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = configured_session(directory.path());
    for (name, mode) in [("rgb", ColorMode::Rgb), ("cmyk", ColorMode::Cmyk), ("gray", ColorMode::Grayscale), ("lab", ColorMode::Lab)] {
        let expected = session.color.working(mode).to_bytes();
        for depth in [8, 16, 32] {
            for background in ["white", "transparent"] {
                for profile in [None, Some("working")] {
                    let mut params = json!({"width": 3, "height": 2, "mode": name, "depth": depth, "background": background});
                    if let Some(profile) = profile {
                        params["profile"] = json!(profile);
                    }
                    session.execute("file.new", params.clone()).unwrap();
                    assert_eq!(embedded(&session), Some(expected.as_slice()), "{params}");
                    assert_eq!(photocraft_engine::color_cmds::document_profile(&session.active().unwrap().doc).to_bytes(), expected);
                }
            }
        }
    }
}

/// The artboard creation path from main retains the same profile defaults and explicit opt-out.
#[test]
fn new_artboards_capture_the_working_profile_and_allow_untagged_creation() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = configured_session(directory.path());
    for (name, mode) in [("rgb", ColorMode::Rgb), ("cmyk", ColorMode::Cmyk), ("gray", ColorMode::Grayscale), ("lab", ColorMode::Lab)] {
        let expected = session.color.working(mode).to_bytes();
        for depth in [8, 16, 32] {
            for profile in [None, Some("working"), Some("none")] {
                let mut params = json!({"width": 3, "height": 2, "mode": name, "depth": depth, "artboard": true});
                if let Some(profile) = profile {
                    params["profile"] = json!(profile);
                }
                session.execute("file.new", params).unwrap();
                assert!(session.active().unwrap().doc.has_artboards());
                assert_eq!(embedded(&session), if profile == Some("none") { None } else { Some(expected.as_slice()) });
            }
        }
    }
}

/// An explicit profile still overrides the working space, and `none` still requests no ICC tag.
#[test]
fn explicit_profiles_and_untagged_creation_keep_their_meaning() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = configured_session(directory.path());
    for (mode, chosen) in [("rgb", Builtin::AdobeRgbCompat), ("cmyk", Builtin::CoatedCmyk), ("gray", Builtin::SGray), ("lab", Builtin::LabD50)] {
        for depth in [8, 16, 32] {
            session.execute("file.new", json!({"width": 3, "height": 2, "mode": mode, "depth": depth, "profile": chosen.id()})).unwrap();
            assert_eq!(embedded(&session), Some(chosen.profile().to_bytes().as_slice()));
            session.execute("file.new", json!({"width": 3, "height": 2, "mode": mode, "depth": depth, "profile": "none"})).unwrap();
            assert_eq!(embedded(&session), None);
        }
    }
}

/// Changing Color Settings affects future documents without reinterpreting existing pixels.
#[test]
fn a_created_document_keeps_its_profile_after_the_working_space_changes() {
    let directory = tempfile::tempdir().unwrap();
    for (name, mode) in [("rgb", ColorMode::Rgb), ("cmyk", ColorMode::Cmyk), ("gray", ColorMode::Grayscale)] {
        let mut session = configured_session(directory.path());
        let expected = session.color.working(mode).to_bytes();
        session.execute("file.new", json!({"width": 3, "height": 2, "mode": name, "depth": 16})).unwrap();
        let original = session.active().unwrap().doc.clone();
        session.execute("edit.colorSettings", json!({"reset": true})).unwrap();
        assert_ne!(session.color.working(mode).to_bytes(), expected);
        assert_eq!(embedded(&session), Some(expected.as_slice()), "{name}");
        assert_eq!(photocraft_engine::color_cmds::document_profile(&original).to_bytes(), expected);
        session.execute("file.new", json!({"width": 3, "height": 2, "mode": name})).unwrap();
        assert_eq!(embedded(&session), Some(session.color.working(mode).to_bytes().as_slice()));
        assert_eq!(original.icc_profile.as_deref().map(Vec::as_slice), Some(expected.as_slice()));
    }
}

/// Malformed or wrong-space profile requests leave the existing document and settings intact.
#[test]
fn invalid_profile_requests_do_not_open_a_document() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = configured_session(directory.path());
    session.execute("file.new", json!({"width": 3, "height": 2})).unwrap();
    let original = session.active().unwrap().doc.clone();
    let settings = session.color.settings.clone();
    for (mode, profiles) in [
        ("rgb", vec![json!("coated-cmyk"), json!("no-such-profile"), json!(""), json!(null), json!(12)]),
        ("cmyk", vec![json!("working-rgb"), json!("srgb")]),
        ("gray", vec![json!("working-cmyk"), Value::Bool(false)]),
    ] {
        for profile in profiles {
            assert!(session.execute("file.new", json!({"width": 3, "height": 2, "mode": mode, "profile": profile})).is_err());
            assert_eq!(session.documents().len(), 1);
            assert!(std::sync::Arc::ptr_eq(&session.active().unwrap().doc, &original));
            assert_eq!(session.color.settings, settings);
        }
    }
}
