//! Raw Options (#14051): File › Open As of a file as Photoshop Raw asks how to read it, with the
//! measured defaults; OK opens the bytes, Cancel opens nothing, and automation is never asked.

use super::*;
use egui::Key;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

type H = Harness<'static, Box<PhotocraftApp>>;

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

/// An app with no document under a harness whose passes run the dialog host.
fn harness() -> H {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    Harness::<'static, Box<PhotocraftApp>>::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(900.0, 800.0))
        .build_ui_state(|ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app)
}

/// Open As `path` as the raw type with no Raw Options field, as a recorded step or the menu
/// with the type chosen sends it.
fn open_as_raw(h: &mut H, path: &str) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    let result = crate::menus::invoke(h.state_mut(), &ctx, "file.openAs", json!({"path": path, "as": "raw"}));
    h.run_steps(3);
    result
}

/// The open Raw Options dialog, if any.
fn dialog(h: &H) -> Option<DialogId> {
    h.state().framework.dialogs.open_dialogs().iter().find(|d| d.key.as_deref() == Some(KIND)).map(|d| d.id)
}

/// The open dialog's values.
fn fields(h: &H) -> Value {
    let open = h.state().framework.dialogs.open_dialogs();
    open.iter().find(|d| d.key.as_deref() == Some(KIND)).and_then(|d| d.fields.clone()).expect("Raw Options is open")
}

/// Sets the open dialog's field `key` to `value`.
fn set(h: &mut H, key: &str, value: Value) {
    let (id, ctx) = (dialog(h).expect("Raw Options is open"), h.ctx.clone());
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, key, value).unwrap();
    h.run_steps(3);
}

/// Answers the open dialog with OK.
fn confirm(h: &mut H) {
    let id = dialog(h).expect("Raw Options is open");
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
}

/// Whether the control labelled `label` is greyed out.
fn greyed(h: &H, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
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
    let f = fields(&h);
    assert_eq!(
        [&f["width"], &f["height"], &f["channels"], &f["depth"], &f["byteOrder"], &f["interleaved"], &f["header"]],
        [&json!(4), &json!(4), &json!(1), &json!("8"), &json!("mac"), &json!(true), &json!(0)]
    );
    assert!(h.query_by_label_contains("Describe \"notes.txt\": (16 bytes)").is_some(), "the title line names the file and its size");
    // One channel: Interleaved and Last Channel is Transparency greyed out; 8 bits: Byte Order
    // too; Retain When Saving always. Depth is free.
    for (label, off) in [
        ("Interleaved", true),
        ("Last Channel is Transparency", true),
        ("Mac", true),
        ("IBM PC", true),
        ("Retain When Saving", true),
        ("8 Bits", false),
        ("16 Bits", false),
        ("32 Bits", false),
    ] {
        assert_eq!(greyed(&h, label), off, "{label}");
    }
    confirm(&mut h);
    assert!(dialog(&h).is_none());
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
    set(&mut h, "depth", json!("16"));
    assert!(!greyed(&h, "Mac"));
    assert!(!greyed(&h, "IBM PC"));
    set(&mut h, "byteOrder", json!("ibm"));
    assert_eq!(fields(&h)["byteOrder"], json!("ibm"));
    assert!(greyed(&h, "Interleaved"));
    set(&mut h, "channels", json!(2));
    assert!(!greyed(&h, "Interleaved"));
}

#[test]
fn cancel_opens_nothing() {
    let path = notes_file("cancel", NOTES);
    let mut h = harness();
    open_as_raw(&mut h, &path).unwrap();
    let id = dialog(&h).unwrap();
    crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.documents().len(), 0);
    // Escape cancels too.
    open_as_raw(&mut h, &path).unwrap();
    assert!(dialog(&h).is_some());
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.documents().len(), 0);
}

#[test]
fn a_second_open_as_raw_is_refused_while_the_dialog_is_up() {
    let path = notes_file("twice", NOTES);
    let mut h = harness();
    open_as_raw(&mut h, &path).unwrap();
    assert!(open_as_raw(&mut h, &path).is_err());
    assert_eq!(h.state().framework.dialogs.open_dialogs().len(), 1);
}

#[test]
fn ok_with_a_size_the_file_cannot_fill_reports_it_and_opens_nothing() {
    let path = notes_file("misfit", NOTES);
    let mut h = harness();
    open_as_raw(&mut h, &path).unwrap();
    set(&mut h, "width", json!(5));
    confirm(&mut h);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.documents().len(), 0);
    let status = &h.state().ui.status;
    assert!(status.contains("take 20 bytes, but the file has 16"), "{status}");
}

#[test]
fn the_answer_carries_every_field_and_the_params_that_asked() {
    let mut layout = RawLayout::defaults_for(16);
    layout.byte_order = ByteOrder::IbmPc;
    layout.header = 2;
    let asked = json!({"path": "/x/notes.txt", "as": "raw", "approveHighMemory": true});
    let p = answer(&asked, &fields_of(&layout));
    assert_eq!(
        p,
        json!({"path": "/x/notes.txt", "as": "raw", "approveHighMemory": true, "width": 4, "height": 4, "channels": 1, "depth": 8,
            "byteOrder": "ibm", "interleaved": true, "header": 2})
    );
    assert_eq!(describe("notes.txt", 16), "Describe \"notes.txt\": (16 bytes)");
    // The answer is a complete request: it is never asked again.
    assert!(!asks(&p));
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
    assert!(dialog(&h).is_none());
    h.state_mut().automation_input = false;
    // A file that can't be read is the engine's to report.
    assert!(open_as_raw(&mut h, &format!("{path}.missing")).is_err());
    assert!(dialog(&h).is_none());
}

/// The dialog reads the file as the engine does, so a path that isn't a regular file is refused
/// at once: a FIFO with no writer would block a plain whole-file read on the UI thread for good.
#[cfg(unix)]
#[test]
fn a_path_that_is_not_a_regular_file_is_refused_without_blocking() {
    let dir = std::path::PathBuf::from(notes_file("fifo", NOTES)).parent().unwrap().to_path_buf();
    let fifo = dir.join("pipe.raw");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success(), "mkfifo");
    let mut h = harness();
    assert!(open_as_raw(&mut h, &fifo.to_string_lossy()).is_err());
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.documents().len(), 0);
}
