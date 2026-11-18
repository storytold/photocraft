//! DDS Options on Save As and Save a Copy: the compression and mipmaps it asks for, the scripted
//! `ddsCompression` and `ddsMipmaps` that skip it, and action recording and playback (#1327).
use super::jpeg_tests::{fields, parked, set_field, steps};
use super::*;
use craft::automation::actions::ActionStep;
use craft::host::{Picked, ScriptedFiles};
use egui_kittest::kittest::Queryable;
use photocraft_io::{DdsCompression, DdsOptions};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Each write: the path, and the DDS options the export was asked for.
type Writes = Rc<RefCell<Vec<(String, DdsOptions)>>>;

const PLAIN: DdsOptions = DdsOptions { compression: DdsCompression::None, mipmaps: false };

/// One 8×8 RGB document whose exports record their DDS options, in a harness running the dialogs
/// and answering a save dialog with `to`.
fn dds_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs_over(1, Some(Arc::new(ScriptedFiles::answering([Picked::Paths(vec![to.into()])]))));
    let writes: Writes = Rc::default();
    let (asked, written) = (Rc::new(RefCell::new(PLAIN)), writes.clone());
    let seen = asked.clone();
    app.services.export = Some(Box::new(move |_, _, settings| {
        *seen.borrow_mut() = settings.dds;
        Ok((Vec::new(), Vec::new()))
    }));
    app.services.write = Some(Box::new(move |path, _| {
        written.borrow_mut().push((path.to_string(), *asked.borrow()));
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

/// Save As to a DDS texture asks first, starting uncompressed without mipmaps, and writes only
/// what is chosen once answered.
#[test]
fn save_as_to_a_dds_texture_asks_for_its_compression() {
    let (mut h, writes) = dds_app("unused.dds");
    h.state_mut().save_as(Some("t.dds".into())).unwrap();
    h.run_steps(2);
    let f = fields(&h);
    assert_eq!((f["compression"].as_str(), f["mipmaps"].as_bool(), parked(&h).map(|p| p.1)), (Some("none"), Some(false), Some(false)));
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    assert!(h.state_mut().save_as(Some("other.dds".into())).is_err(), "one save at a time");
    set_field(&mut h, "compression", json!("bc3"));
    set_field(&mut h, "mipmaps", json!(true));
    h.get_by_label("OK").click();
    h.run_steps(3);
    let chosen = DdsOptions { compression: DdsCompression::Bc3, mipmaps: true };
    assert_eq!(writes.borrow().as_slice(), &[("t.dds".to_string(), chosen)]);
    assert!(h.state().parked_save.is_none());
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("t.dds"));
}

/// Scripts choose with `ddsCompression` or `ddsMipmaps` and are never asked; bad values write
/// nothing, and other formats aren't asked.
#[test]
fn scripted_dds_options_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = dds_app("unused.dds");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.dds", "ddsCompression": "bc1"})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "m.dds", "ddsMipmaps": true})).unwrap();
    assert!(h.state().parked_save.is_none());
    for bad in [json!({"ddsCompression": "bc7"}), json!({"ddsMipmaps": 1})] {
        let mut p = bad.clone();
        p["path"] = json!("bad.dds");
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", p).is_err(), "{bad}");
    }
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    assert!(h.state().parked_save.is_none());
    assert_eq!(
        writes.borrow().as_slice(),
        &[
            ("s.dds".to_string(), DdsOptions { compression: DdsCompression::Bc1, mipmaps: false }),
            ("m.dds".to_string(), DdsOptions { compression: DdsCompression::None, mipmaps: true }),
            ("p.png".to_string(), PLAIN),
        ]
    );
}

/// An action being recorded keeps a DDS Save As or Save a Copy with its options, answered in DDS
/// Options or scripted, and plays them back without asking.
#[test]
fn recorded_dds_saves_keep_their_options_and_replay_without_asking() {
    let (mut h, writes) = dds_app("copy.dds");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("t.dds".into())).unwrap();
    h.run_steps(2);
    set_field(&mut h, "compression", json!("bc1"));
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    set_field(&mut h, "mipmaps", json!(true));
    h.key_press(Key::Enter);
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.dds", "ddsCompression": "bc2"})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let want = [
        ("t.dds".to_string(), DdsOptions { compression: DdsCompression::Bc1, mipmaps: false }),
        ("copy.dds".to_string(), DdsOptions { compression: DdsCompression::None, mipmaps: true }),
        ("s.dds".to_string(), DdsOptions { compression: DdsCompression::Bc2, mipmaps: false }),
    ];
    assert_eq!(writes.borrow().as_slice(), &want);
    assert_eq!(
        steps(&h),
        &vec![
            ActionStep::new("file.saveAs", json!({"path": "t.dds", "ddsCompression": "bc1", "ddsMipmaps": false})),
            ActionStep::new("file.saveACopy", json!({"path": "copy.dds", "ddsCompression": "none", "ddsMipmaps": true})),
            ActionStep::new("file.saveAs", json!({"path": "s.dds", "ddsCompression": "bc2", "ddsMipmaps": false})),
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().parked_save.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &want);
}
