//! Raw Options (#14051): File › Open As of a file as Photoshop Raw asks how to read it, with the
//! measured defaults; OK opens the bytes, Cancel opens nothing, and automation is never asked.

use super::*;
use egui::Key;
use egui_kittest::kittest::Queryable;

/// The measured file: 16 bytes, read as 4 rows of 4 gray pixels.
const NOTES: &[u8] = b"hello world 1234";

/// A fresh folder holding `bytes` as `notes.txt`; returns the file's path.
fn notes_file(case: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("photocraft-raw-options-ui-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notes.txt");
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

/// An app with no document in a harness showing Raw Options.
fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 700.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h
}

/// Open As `path` as the raw type with no Raw Options field, as a recorded step or the menu
/// with the type chosen sends it.
fn open_as_raw(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, path: &str) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.openAs", json!({"path": path, "as": "raw"}))
}

/// The active document's pixels as 8-bit gray values, row by row.
fn gray_bytes(app: &PhotocraftApp) -> Vec<u8> {
    let doc = &app.session.active().unwrap().doc;
    let surface = doc.layers[0].surface().unwrap();
    let (w, h) = (doc.size.width as i32, doc.size.height as i32);
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| (surface.pixel(x, y)[0] * 255.0).round() as u8).collect()
}

#[test]
fn open_as_raw_asks_with_the_measured_defaults_and_ok_opens_the_bytes() {
    let path = notes_file("ok", NOTES);
    let mut h = harness();
    assert_eq!(open_as_raw(&mut h, &path), Ok(json!({"rawOptions": true})));
    assert_eq!(h.state().session.documents().len(), 0, "nothing opens before OK");
    let p = h.state().raw_options.as_ref().unwrap();
    assert_eq!(p.layout, RawLayout { width: 4, height: 4, channels: 1, depth: 8, byte_order: ByteOrder::Mac, interleaved: true, header: 0 });
    h.run_steps(2);
    h.get_by_label("Describe \"notes.txt\": (16 bytes)");
    // One channel: Interleaved and Last Channel is Transparency greyed out; 8 bits: Byte Order
    // too; Retain When Saving with no header. Depth is free.
    for (label, greyed) in [
        ("Interleaved", true),
        ("Last Channel is Transparency", true),
        ("Mac", true),
        ("IBM PC", true),
        ("Retain When Saving", true),
        ("8 Bits", false),
        ("16 Bits", false),
        ("32 Bits", false),
        ("OK", false),
        ("Cancel", false),
    ] {
        assert_eq!(h.get_by_label(label).accesskit_node().is_disabled(), greyed, "{label}");
    }
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().raw_options.is_none());
    let app = h.state();
    assert_eq!(app.session.documents().len(), 1);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!((doc.name.as_str(), doc.size.width, doc.size.height), ("notes.txt", 4, 4));
    assert_eq!(doc.mode, photocraft_color::ColorMode::Grayscale);
    assert_eq!(gray_bytes(app), NOTES);
}

#[test]
fn byte_order_and_interleaved_open_up_with_their_depth_and_channels() {
    let path = notes_file("enable", NOTES);
    let mut h = harness();
    open_as_raw(&mut h, &path).unwrap();
    h.run_steps(2);
    h.get_by_label("16 Bits").click();
    h.run_steps(2);
    assert_eq!(h.state().raw_options.as_ref().map(|p| p.layout.depth), Some(16));
    assert!(!h.get_by_label("Mac").accesskit_node().is_disabled());
    h.get_by_label("IBM PC").click();
    h.run_steps(2);
    assert_eq!(h.state().raw_options.as_ref().map(|p| p.layout.byte_order), Some(ByteOrder::IbmPc));
    h.state_mut().raw_options.as_mut().unwrap().layout.channels = 2;
    h.run_steps(2);
    assert!(!h.get_by_label("Interleaved").accesskit_node().is_disabled());
}

#[test]
fn cancel_opens_nothing() {
    let path = notes_file("cancel", NOTES);
    let mut h = harness();
    open_as_raw(&mut h, &path).unwrap();
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().raw_options.is_none());
    assert_eq!(h.state().session.documents().len(), 0);
    // Escape cancels too.
    open_as_raw(&mut h, &path).unwrap();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().raw_options.is_none());
    assert_eq!(h.state().session.documents().len(), 0);
}

#[test]
fn ok_with_a_size_the_file_cannot_fill_reports_it_and_opens_nothing() {
    let path = notes_file("misfit", NOTES);
    let mut h = harness();
    open_as_raw(&mut h, &path).unwrap();
    h.state_mut().raw_options.as_mut().unwrap().layout.width = 5;
    answer(h.state_mut());
    assert!(h.state().raw_options.is_none());
    assert_eq!(h.state().session.documents().len(), 0);
    let status = &h.state().ui.status;
    assert!(status.contains("take 20 bytes, but the file has 16"), "{status}");
}

#[test]
fn the_answer_carries_every_field_and_the_params_that_asked() {
    let mut p = Prompt::new(json!({"path": "/x/notes.txt", "as": "raw", "approveHighMemory": true}), "notes.txt".into(), 16);
    p.layout.byte_order = ByteOrder::IbmPc;
    p.layout.header = 2;
    assert_eq!(
        p.answer(),
        json!({"path": "/x/notes.txt", "as": "raw", "approveHighMemory": true, "width": 4, "height": 4, "channels": 1, "depth": 8,
            "byteOrder": "ibm", "interleaved": true, "header": 2})
    );
    assert_eq!(p.describe(), "Describe \"notes.txt\": (16 bytes)");
    // The answer is a complete request: it is never asked again.
    assert!(!asks(&p.answer()));
}

#[test]
fn only_the_raw_type_without_fields_asks() {
    assert!(asks(&json!({"path": "a", "as": "raw"})));
    assert!(asks(&json!({"path": "a", "as": ".RAW"})));
    assert!(asks(&json!({"path": "a", "format": "raw", "width": null})));
    assert!(!asks(&json!({"path": "a", "as": "raw", "width": 4})));
    assert!(!asks(&json!({"path": "a", "as": "raw", "header": 0})));
    assert!(!asks(&json!({"path": "a", "as": "png"})));
    assert!(!asks(&json!({"path": "a"})));
}

#[test]
fn automation_and_unreadable_files_are_never_asked() {
    let path = notes_file("automation", NOTES);
    let mut h = harness();
    h.state_mut().automation_input = true;
    // Automation gets the engine's answer: a text file isn't a camera raw.
    assert!(open_as_raw(&mut h, &path).is_err());
    assert!(h.state().raw_options.is_none());
    h.state_mut().automation_input = false;
    // A file that can't be read is the engine's to report.
    assert!(open_as_raw(&mut h, &format!("{path}.missing")).is_err());
    assert!(h.state().raw_options.is_none());
}
