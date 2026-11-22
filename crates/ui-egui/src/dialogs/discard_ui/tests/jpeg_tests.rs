//! JPEG Options on Save As and Save a Copy (FILE-215-19): the quality and format it asks for, the
//! scripted `quality` / `jpegFormat` that skip it, and recording and playback.
use super::*;
use crate::jpeg_options_ui::{self, REMEMBER_KEY, Remembered};
use craft::automation::actions::ActionStep;
use craft::host::{Picked, ScriptedFiles};
use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedFormatOptions;
use craft::ui::runtime::memory::{saved_value, set_saved_value};
use egui_kittest::kittest::Queryable;
use photocraft_codecs::JpegFormat;
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Each write: the path, and the JPEG quality (1–100) and format the export was asked for.
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The JSON a write records.
fn asked(quality: Option<u8>, format: JpegFormat) -> Vec<u8> {
    serde_json::to_vec(&(quality, format.name())).unwrap()
}

/// The encoder quality for Photoshop quality `q`.
fn pct(q: u8) -> Option<u8> {
    Some(photocraft_engine::file_cmds::jpeg_quality(f64::from(q)))
}

/// One 8×8 RGB document whose exports record their JPEG settings, in a harness running the
/// dialogs and answering a save dialog with `to`.
fn jpeg_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs_over(1, Some(Arc::new(ScriptedFiles::answering([Picked::Paths(vec![to.into()])]))));
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.export = Some(Box::new(|_, _, settings| Ok((asked(settings.jpeg_quality, settings.jpeg_format), Vec::new()))));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app| {
            crate::dialogs::run(app, ui);
            app.poll_file_dialog();
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    craft::ui::testing::windows::set_os(&h.ctx, craft::host::target::Os::Windows);
    (h, writes)
}

/// The path and copy flag of the parked save.
pub(super) fn parked(h: &Prompted) -> Option<(String, bool)> {
    h.state().parked_save.as_ref().map(|s| (s.path.clone(), s.copy))
}

/// The values of the open format options dialog by field.
pub(super) fn fields(h: &Prompted) -> Value {
    h.state().framework.dialogs.get::<HostedFormatOptions>().and_then(|d| d.json_fields()).expect("a format options dialog is open")
}

/// The quality (0–12) and the format id the open dialog shows.
fn shown(h: &Prompted) -> (u64, String) {
    let f = fields(h);
    (f["quality"].as_u64().expect("a quality"), f["format"].as_str().expect("a format").to_owned())
}

/// Edits `field` of the open format options dialog as the user would.
pub(super) fn set_field(h: &mut Prompted, field: &str, value: Value) {
    let ctx = h.ctx.clone();
    let id = h.state().framework.dialogs.get::<HostedFormatOptions>().expect("a format options dialog is open").id();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
    h.run_steps(2);
}

/// The steps of the first recorded action.
pub(super) fn steps(h: &Prompted) -> &Vec<ActionStep> {
    &h.state().session.actions.list.action(0).unwrap().steps
}

/// Save As to a JPEG asks first, starting at the Export preferences' quality (85 → 10, Maximum)
/// and Baseline, and writes only once answered.
#[test]
fn save_as_to_a_jpeg_asks_for_jpeg_options() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    assert_eq!(parked(&h), Some(("p.jpg".to_string(), false)));
    assert_eq!(shown(&h), (10, "baseline".to_string()));
    assert!(h.query_all_by_label("JPEG Options").next().is_some());
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    assert!(h.state_mut().save_as(Some("other.jpg".into())).is_err(), "one save at a time");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.jpg".to_string(), asked(pct(10), JpegFormat::Baseline))]);
    assert!(h.state().parked_save.is_none());
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.jpg"));
}

/// Re-saving an opened JPEG starts at the file's own quality (`file_encoding`, as File › Save
/// keeps it since #4032), not the Export preference's; left there, OK writes that quality exactly
/// rather than its nearest step on the 0–12 scale (95 would come back as 92), and the recorded step
/// leaves the quality out, so playback keeps it too. Save a Copy does the same; a quality changed
/// in the prompt is written and recorded on the scale.
#[test]
fn resaving_an_opened_jpeg_starts_at_its_own_quality() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    std::sync::Arc::make_mut(h.state_mut().session.active_mut().unwrap().doc_mut()).file_encoding.jpeg_quality = Some(95);
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    assert_eq!(shown(&h).0, 11, "95 is 11 on Photoshop's scale");
    h.key_press(Key::Enter);
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!((shown(&h).0, parked(&h).map(|p| p.1)), (11, Some(true)));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().save_as(Some("q.jpg".into())).unwrap();
    h.run_steps(2);
    set_field(&mut h, "quality", json!(12));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let expected = [
        ("p.jpg".to_string(), asked(Some(95), JpegFormat::Baseline)),
        ("copy.jpg".to_string(), asked(Some(95), JpegFormat::Baseline)),
        ("q.jpg".to_string(), asked(pct(12), JpegFormat::Baseline)),
    ];
    assert_eq!(writes.borrow().as_slice(), &expected);
    assert_eq!(
        steps(&h),
        &vec![
            ActionStep::new("file.saveAs", json!({"path": "p.jpg", "jpegFormat": "baseline"})),
            ActionStep::new("file.saveACopy", json!({"path": "copy.jpg", "jpegFormat": "baseline"})),
            ActionStep::new("file.saveAs", json!({"path": "q.jpg", "quality": 12, "jpegFormat": "baseline"})),
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(writes.borrow().as_slice(), &expected, "playback writes the file's own quality again");
}

/// A `file.saveACopy` step recorded from the engine command keeps its params as given, and the
/// engine clamps a quality past 12; playing it back writes the copy the same way instead of
/// failing on the quality.
#[test]
fn a_copy_recorded_from_the_engine_command_replays() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let path = std::env::temp_dir().join(format!("photocraft-jpeg-copy-{}.jpg", std::process::id())).to_string_lossy().into_owned();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().run("file.saveACopy", json!({"path": path, "quality": 90})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(steps(&h), &vec![ActionStep::new("file.saveACopy", json!({"path": path, "quality": 90}))]);
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(writes.borrow().as_slice(), &[(path, asked(pct(12), JpegFormat::Baseline))]);
}

/// The quality and format chosen in the dialog are the ones written; Cancel writes nothing.
#[test]
fn jpeg_options_writes_the_choice_or_cancels() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Progressive").click();
    h.run_steps(2);
    set_field(&mut h, "quality", json!(3));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.jpg".to_string(), asked(pct(3), JpegFormat::Progressive))]);

    h.state_mut().save_as(Some("c.jpg".into())).unwrap();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().parked_save.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: nothing more written");
    assert_eq!(h.state().ui.status, "Save canceled");
}

/// Scripts choose with `quality` or `jpegFormat` and are never asked; a bad value writes nothing;
/// other formats aren't asked.
#[test]
fn scripted_jpeg_options_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.jpg", "quality": 12, "jpegFormat": "optimized"})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "f.jpeg", "jpegFormat": "progressive"})).unwrap();
    assert!(h.state().parked_save.is_none());
    for bad in [json!({"quality": 13}), json!({"quality": "8"}), json!({"jpegFormat": "fast"})] {
        let mut p = bad.clone();
        p["path"] = json!("s.jpg");
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", p).is_err(), "{bad}");
    }
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    assert!(h.state().parked_save.is_none());
    assert_eq!(
        writes.borrow().as_slice(),
        &[
            ("s.jpg".to_string(), asked(Some(100), JpegFormat::Optimized)),
            ("f.jpeg".to_string(), asked(None, JpegFormat::Progressive)),
            ("p.png".to_string(), asked(None, JpegFormat::Baseline)),
        ]
    );
}

/// Save a Copy to a JPEG asks too, and the copy leaves the document's path and unsaved state.
#[test]
fn save_a_copy_to_a_jpeg_asks_and_keeps_the_document_unsaved() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(parked(&h).map(|p| p.1), Some(true));
    assert!(jpeg_options_ui::park(h.state_mut(), "other.jpg".into(), false, "file.saveAs").is_err(), "the waiting prompt is kept");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.jpg".to_string(), asked(pct(10), JpegFormat::Baseline))]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// An action being recorded keeps a JPEG Save As or Save a Copy with its answer (#2032, #4373)
/// and plays them back without asking.
#[test]
fn recorded_jpeg_saves_keep_their_options_and_replay_without_asking() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Baseline Optimized").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    set_field(&mut h, "quality", json!(6));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(
        steps(&h),
        &vec![
            ActionStep::new("file.saveAs", json!({"path": "p.jpg", "quality": 10, "jpegFormat": "optimized"})),
            // The Save a Copy prompt starts at the format just chosen (it is remembered).
            ActionStep::new("file.saveACopy", json!({"path": "copy.jpg", "quality": 6, "jpegFormat": "optimized"})),
        ]
    );
    let expected = [("p.jpg".to_string(), asked(pct(10), JpegFormat::Optimized)), ("copy.jpg".to_string(), asked(pct(6), JpegFormat::Optimized))];
    assert_eq!(writes.borrow().as_slice(), &expected);
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().parked_save.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &expected);
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.jpg"), "the replayed copy leaves the document's file");
}

/// The JPEG Options dialog starts at what it was last answered with (#9631): the quality and the
/// format, for a document that isn't an opened JPEG. Cancel remembers nothing.
#[test]
fn the_next_dialog_starts_at_the_last_answer() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let ctx = h.ctx.clone();
    h.state_mut().save_as(Some("a.jpg".into())).unwrap();
    h.run_steps(2);
    set_field(&mut h, "quality", json!(6));
    h.get_by_label("Progressive").click();
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(writes.borrow().is_empty());
    assert_eq!(saved_value::<Remembered>(&ctx, &REMEMBER_KEY), Remembered::default(), "a cancelled dialog is not an answer");
    h.state_mut().save_as(Some("b.jpg".into())).unwrap();
    h.run_steps(2);
    assert_eq!(shown(&h), (10, "baseline".to_string()));
    set_field(&mut h, "quality", json!(6));
    h.get_by_label("Progressive").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(saved_value::<Remembered>(&ctx, &REMEMBER_KEY), Remembered { quality: Some(6), jpeg_format: Some("progressive".into()) });
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(shown(&h), (6, "progressive".to_string()));
    h.key_press(Key::Enter);
    h.run_steps(3);
    let expected = [("b.jpg".to_string(), asked(pct(6), JpegFormat::Progressive)), ("unused.jpg".to_string(), asked(pct(6), JpegFormat::Progressive))];
    assert_eq!(writes.borrow().as_slice(), &expected);
}

/// An opened JPEG still starts at its own quality (File › Save keeps it, #4032) but at the last
/// format; left on its own quality, the answer doesn't replace the quality remembered for other
/// documents.
#[test]
fn an_opened_jpeg_starts_at_its_own_quality_and_the_last_format() {
    let (mut h, _writes) = jpeg_app("unused.jpg");
    let ctx = h.ctx.clone();
    let before = Remembered { quality: Some(3), jpeg_format: Some("progressive".into()) };
    set_saved_value(&ctx, &REMEMBER_KEY, before.clone());
    std::sync::Arc::make_mut(h.state_mut().session.active_mut().unwrap().doc_mut()).file_encoding.jpeg_quality = Some(95);
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    assert_eq!(shown(&h), (11, "progressive".to_string()));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(saved_value::<Remembered>(&ctx, &REMEMBER_KEY).quality, before.quality, "the own quality is not an answer");
}
