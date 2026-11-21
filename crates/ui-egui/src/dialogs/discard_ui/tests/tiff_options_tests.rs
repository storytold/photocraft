//! TIFF Options on Save As and Save a Copy (#8827): Image Compression, Save Image Pyramid, Save
//! Transparency and the layers choice; the scripted params that skip it; recording and playback.
use super::jpeg_tests::{fields, parked, set_field, steps};
use super::*;
use crate::tiff_options_ui;
use craft::automation::actions::ActionStep;
use craft::host::{Picked, ScriptedFiles};
use egui_kittest::kittest::Queryable;
use std::{cell::RefCell, rc::Rc, sync::Arc};

type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// What a write was asked for: compression, Save Transparency, pyramid, layers.
fn asked(compression: &str, transparency: Option<bool>, pyramid: bool, layers: bool) -> Vec<u8> {
    serde_json::to_vec(&(compression, transparency, pyramid, layers)).unwrap()
}

/// A harness with one 8×8 document (`background`: "white" or "transparent") whose exports record
/// their TIFF settings, showing TIFF Options and answering a save dialog with `to`.
fn tiff_app(background: &str, to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs_over(0, Some(Arc::new(ScriptedFiles::answering([Picked::Paths(vec![to.into()])]))));
    app.run("file.new", json!({"width": 8, "height": 8, "background": background})).unwrap();
    app.sync_views();
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.export = Some(Box::new(|_, _, s| {
        Ok((asked(tiff_options_ui::compression_name(s.tiff_compression), s.tiff_transparency, s.tiff_pyramid, s.tiff_layers), Vec::new()))
    }));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_context(app.ctx().clone());
    let builder = if std::env::var_os("PHOTOCRAFT_TIFF_SNAPSHOTS").is_some() { builder.wgpu() } else { builder };
    let h = builder.build_ui_state(
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

/// `PHOTOCRAFT_TIFF_SNAPSHOTS=<dir>` saves the dialog as a PNG to look at.
fn capture(h: &mut Prompted, name: &str) {
    if let Some(dir) = std::env::var_os("PHOTOCRAFT_TIFF_SNAPSHOTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

/// A flat TIFF (one Background) asks too: compression and pyramid, but no transparency or layers
/// to choose. The answer is written, recorded and remembered for the next TIFF.
#[test]
fn save_as_to_a_flat_tiff_asks_for_compression_and_pyramid() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, writes) = tiff_app("white", "unused.tif");
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("p.tif".into())).unwrap();
    h.run_steps(2);
    capture(&mut h, "tiff-options-flat");
    let f = fields(&h);
    assert_eq!((f["compression"].as_str(), f["pyramid"].as_bool(), parked(&h).map(|p| p.1)), (Some("none"), Some(false), Some(false)));
    assert!(f.get("transparency").is_none() && f.get("discardLayers").is_none(), "no transparency or layers rows: {f}");
    assert!(h.query_by_label("Save Transparency").is_none(), "nothing transparent to save");
    assert!(h.query_by_label("Discard Layers and Save a Copy").is_none());
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    set_field(&mut h, "compression", json!("lzw"));
    set_field(&mut h, "pyramid", json!(true));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.tif".to_string(), asked("lzw", None, true, true))]);
    assert!(h.state().parked_save.is_none());
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.tif"));
    // The next TIFF starts where this one ended.
    h.state_mut().save_as(Some("q.tif".into())).unwrap();
    h.run_steps(2);
    let f = fields(&h);
    assert_eq!((f["compression"].as_str(), f["pyramid"].as_bool()), (Some("lzw"), Some(true)));
    set_field(&mut h, "compression", json!("zip"));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(
        steps(&h),
        &vec![
            ActionStep::new("file.saveAs", json!({"path": "p.tif", "tiffCompression": "lzw", "tiffPyramid": true})),
            ActionStep::new("file.saveAs", json!({"path": "q.tif", "tiffCompression": "zip", "tiffPyramid": true})),
        ]
    );
    // Playback writes the same files without asking.
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    h.run_steps(2);
    assert!(h.state().parked_save.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("p.tif".to_string(), asked("lzw", None, true, true)), ("q.tif".to_string(), asked("zip", None, true, true))]);
}

/// A document without a Background offers Save Transparency, on to start; with an alpha or spot
/// channel it starts off, as an unset Save Transparency writes. Its layers can be discarded,
/// which saves a copy.
#[test]
fn layered_transparent_documents_choose_transparency_and_layers() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, writes) = tiff_app("transparent", "unused.tif");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().save_as(Some("t.tif".into())).unwrap();
    h.run_steps(4);
    capture(&mut h, "tiff-options-layered");
    let f = fields(&h);
    assert_eq!((f["transparency"].as_bool(), f["discardLayers"].as_bool()), (Some(true), Some(false)), "{f}");
    assert!(h.query_by_label("Including layers will increase file size.").is_some(), "the preference is on by default");
    set_field(&mut h, "transparency", json!(false));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("t.tif".to_string(), asked("none", Some(false), false, true))]);
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("t.tif"));

    // An alpha channel: Save Transparency starts off. Discarding the layers writes a copy.
    h.state_mut().run("select.all", json!({})).unwrap();
    h.state_mut().run("select.saveSelection", json!({})).unwrap();
    assert!(!h.state().session.active().unwrap().doc.channels.is_empty(), "an alpha channel");
    h.state_mut().save_as(Some("u.tif".into())).unwrap();
    h.run_steps(4);
    assert_eq!(fields(&h)["transparency"].as_bool(), Some(false));
    set_field(&mut h, "discardLayers", json!(true));
    assert!(h.query_by_label("Including layers will increase file size.").is_none(), "no layers, no note");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().last().unwrap(), &("u.tif".to_string(), asked("none", Some(false), false, false)));
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("t.tif"), "a copy: the document keeps its file");
}

/// "Don't show again" turns off the layered-file note for the next save.
#[test]
fn dont_show_again_clears_the_layered_tiff_preference() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, _) = tiff_app("transparent", "unused.tif");
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().save_as(Some("t.tif".into())).unwrap();
    h.run_steps(4);
    // With Discard Layers on the note and its checkbox are hidden: the preference stays.
    set_field(&mut h, "dontShowAgain", json!(true));
    set_field(&mut h, "discardLayers", json!(true));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.prefs().file_handling.ask_before_saving_layered_tiff, "the note was not shown, so it is not cleared");
    h.state_mut().save_as(Some("u.tif".into())).unwrap();
    h.run_steps(4);
    set_field(&mut h, "dontShowAgain", json!(true));
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(!h.state().session.prefs().file_handling.ask_before_saving_layered_tiff);
    h.state_mut().save_as(Some("v.tif".into())).unwrap();
    h.run_steps(4);
    assert!(h.state().parked_save.is_some(), "TIFF Options still asks");
    assert!(h.query_by_label("Including layers will increase file size.").is_none());
}

/// File › Save confirming the document's own TIFF asks TIFF Options, and an action being
/// recorded keeps it as a plain Save (no path) with the answer: played back, it writes the TIFF
/// the action is played on, not the one it was recorded on (#3531).
#[test]
fn recorded_plain_save_of_a_tiff_writes_the_file_it_is_played_on() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, writes) = tiff_app("white", "own.tif");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "own.tif", "tiffCompression": "none"})).unwrap();
    h.state_mut().run("actions.record", json!({})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.save", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.state().parked_save.is_some(), "confirming its own TIFF asks TIFF Options");
    set_field(&mut h, "compression", json!("lzw"));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(steps(&h), &vec![ActionStep::new("file.save", json!({"tiffCompression": "lzw"}))]);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "other.tif", "tiffCompression": "none"})).unwrap();
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().parked_save.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &[("other.tif".to_string(), asked("lzw", None, false, true))]);
}

/// A scripted plain Save that passes the TIFF answers skips the prompt and is recorded the same
/// way: `file.save` with the answers and no path, so playback writes the file it is played on.
#[test]
fn recorded_scripted_plain_save_of_a_tiff_has_no_path() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, writes) = tiff_app("white", "own.tif");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "own.tif", "tiffCompression": "none"})).unwrap();
    h.state_mut().run("actions.record", json!({})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.save", json!({"tiffCompression": "lzw", "tiffTransparency": false, "tiffPyramid": true})).unwrap();
    h.run_steps(3);
    assert!(h.state().parked_save.is_none(), "the script answered");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(steps(&h), &vec![ActionStep::new("file.save", json!({"tiffCompression": "lzw", "tiffTransparency": false, "tiffPyramid": true}))]);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "other.tif", "tiffCompression": "none"})).unwrap();
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(writes.borrow().as_slice(), &[("other.tif".to_string(), asked("lzw", Some(false), true, true))]);
}

/// OK writes the waiting document with its own save settings, even after another tab became
/// active: its path is what linked smart objects resolve against.
#[test]
fn confirm_uses_the_waiting_documents_settings_not_the_active_tabs() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, _) = tiff_app("white", "unused.tif");
    let seen: Rc<RefCell<Vec<Option<std::path::PathBuf>>>> = Rc::default();
    let saw = seen.clone();
    h.state_mut().services.export = Some(Box::new(move |_, _, s| {
        saw.borrow_mut().push(s.document_path.clone());
        Ok((Vec::new(), Vec::new()))
    }));
    h.state_mut().save_as(Some("a.tif".into())).unwrap();
    h.run_steps(4);
    h.key_press(Key::Enter);
    h.run_steps(3);
    let a = h.state().active_doc_id().unwrap();
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("a.tif"));
    h.state_mut().run("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    h.state_mut().sync_views();
    let b = h.state().active_doc_id().unwrap();
    h.state_mut().refocus(a).unwrap();
    h.state_mut().save_as(Some("b.tif".into())).unwrap();
    h.run_steps(4);
    assert!(h.state().parked_save.is_some());
    h.state_mut().refocus(b).unwrap();
    seen.borrow_mut().clear();
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(seen.borrow().as_slice(), &[Some(std::path::PathBuf::from("a.tif"))]);
}

/// Scripted saves that pass TIFF answers skip the prompt; a bad one is an error, not a save.
#[test]
fn scripted_tiff_answers_skip_the_prompt() {
    let _gpu = photocraft_testkit::gpu_lock();
    let (mut h, writes) = tiff_app("transparent", "unused.tif");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.tif", "tiffCompression": "zip", "tiffTransparency": true, "tiffPyramid": true}))
        .unwrap();
    h.run_steps(2);
    assert!(h.state().parked_save.is_none());
    assert_eq!(writes.borrow().as_slice(), &[("s.tif".to_string(), asked("zip", Some(true), true, true))]);
    for bad in [json!({"path": "b.tif", "tiffCompression": "jpeg"}), json!({"path": "b.tif", "tiffPyramid": "yes"})] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(writes.borrow().len(), 1);
    assert!(h.state().parked_save.is_none());
    // Automation saves never prompt, and take the answers too.
    let choices = crate::FormatChoices::from_params(&json!({"tiffCompression": "lzw"}), "app.save").unwrap();
    h.state_mut().services.automation_write = Some(Box::new({
        let w = writes.clone();
        move |path, bytes| {
            w.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        }
    }));
    h.state_mut().save_automation_with(Some("a.tif".into()), choices).unwrap();
    assert_eq!(writes.borrow().last().unwrap(), &("a.tif".to_string(), asked("lzw", None, false, true)));
}
