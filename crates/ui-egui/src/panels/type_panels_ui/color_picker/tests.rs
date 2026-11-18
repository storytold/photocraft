use super::*;
use crate::canvas;
use craft::color_ui::ui::PickedColor;

fn fixture(kind: Kind, depth: u32) -> (PhotocraftApp, u32) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("prefs.set", json!({"path":"interface.language","value":"en"})).unwrap();
    app.run("file.new", json!({"width":320,"height":160,"depth":depth,"background":"transparent"})).unwrap();
    let layer = app.run("type.create", json!({"x":10,"y":55,"text":"Shared color","size":30})).unwrap()["layer"].as_u64().unwrap();
    let prefix = if kind.paragraph() { "type.paragraphStyle" } else { "type.characterStyle" };
    let id = app.run(&format!("{prefix}.new"), json!({"fromSelection":false,"attrs":{"color":"#ff0000"}})).unwrap()["id"].as_u64().unwrap() as u32;
    app.run(&format!("{prefix}.apply"), json!({"id":id,"layer":layer})).unwrap();
    (app, id)
}

/// The colour the picker opened with, as hex.
fn fields(app: &PhotocraftApp, id: DialogId) -> String {
    let opening = app.framework.dialogs.get_by_id::<ColorPickerDialog>(id).unwrap().opening().unwrap();
    color_picker_ui::hex(opening.color.to_rgb_array())
}

fn set(app: &mut PhotocraftApp, id: DialogId, color: &str) {
    let color = Srgba::from_hex(color).unwrap();
    app.framework.dialogs.get_mut_by_id::<ColorPickerDialog>(id).unwrap().set_color(PickedColor::rgb(color));
}

/// OK, as the dialog's button gives it: the app's taker files the answer with [`answered`].
fn confirm_picker(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::confirm(app, id).expect("OK is enabled");
}

fn cancel_picker(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::cancel(app, id).expect("Cancel is enabled");
}

#[test]
fn preview_is_cached_cancel_is_clean_and_ok_is_one_undo_step_at_all_depths() {
    for kind in [Kind::Character, Kind::Paragraph] {
        for depth in [8, 16, 32] {
            let (mut app, id) = fixture(kind, depth);
            let st = app.session.active().unwrap();
            let (doc, rev, dirty, history) = (st.doc.clone(), st.revision, st.is_dirty(), st.history.past_len());
            let original_pixels = photocraft_compose::flatten(&doc);
            let (x, y) =
                (0..160).flat_map(|y| (0..320).map(move |x| (x, y))).find(|&(x, y)| original_pixels.get(x, y)[3] > 0.1).expect("fixture renders visible text");
            let picker = open(&mut app, kind, id).unwrap();
            set(&mut app, picker, "#00ff00");
            let (shown, key) = canvas::display_doc(&mut app, 0);
            let again = canvas::display_doc(&mut app, 0);
            assert!(Arc::ptr_eq(&shown, &again.0));
            assert_eq!(key, again.1);
            assert_ne!(photocraft_compose::flatten(&shown).get(x, y), photocraft_compose::flatten(&doc).get(x, y));
            let st = app.session.active().unwrap();
            assert!(Arc::ptr_eq(&st.doc, &doc));
            assert_eq!((st.revision, st.is_dirty(), st.history.past_len()), (rev, dirty, history));
            set(&mut app, picker, "#0000ff");
            assert_ne!(canvas::display_doc(&mut app, 0).1, key);
            cancel_picker(&mut app, picker);
            assert!(app.text_style_preview.is_none());
            assert!(Arc::ptr_eq(&canvas::display_doc(&mut app, 0).0, &doc));
            let picker = open(&mut app, kind, id).unwrap();
            set(&mut app, picker, "#00ff00");
            let shown = canvas::display_doc(&mut app, 0).0;
            confirm_picker(&mut app, picker);
            let st = app.session.active().unwrap();
            assert_eq!(st.history.past_len(), history + 1);
            assert_eq!(serde_json::to_value(&st.doc.text_styles).unwrap(), serde_json::to_value(&shown.text_styles).unwrap());
            assert_eq!(photocraft_compose::flatten(&st.doc).get(x, y), photocraft_compose::flatten(&shown).get(x, y));
            assert!(app.text_style_preview.is_none());
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(serde_json::to_value(&app.session.active().unwrap().doc.text_styles).unwrap(), serde_json::to_value(&doc.text_styles).unwrap());
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(color(&app.session.active().unwrap().doc, kind, id).unwrap().to_rgb(), [0.0, 1.0, 0.0]);
        }
    }
}

#[test]
fn no_op_keeps_precise_imported_color_and_returning_to_original_removes_preview() {
    let (mut app, id) = fixture(Kind::Character, 32);
    app.run("type.characterStyle.set", json!({"id":id,"attrs":{"color":photocraft_doc::Color::rgba(0.12345,0.67891,0.33333,0.45678)}})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let picker = open(&mut app, Kind::Character, id).unwrap();
    let original = fields(&app, picker);
    set(&mut app, picker, "#ffffff");
    canvas::display_doc(&mut app, 0);
    set(&mut app, picker, &original);
    assert!(Arc::ptr_eq(&canvas::display_doc(&mut app, 0).0, &before));
    assert!(app.text_style_preview.is_none());
    confirm_picker(&mut app, picker);
    assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn invalidated_targets_never_redirect() {
    let ctx = egui::Context::default();
    for change in 0..6 {
        let (mut app, id) = fixture(Kind::Character, 8);
        let picker = open(&mut app, Kind::Character, id).unwrap();
        set(&mut app, picker, "#00ff00");
        canvas::display_doc(&mut app, 0);
        match change {
            0 => {
                app.run("file.new", json!({"width":10,"height":10})).unwrap();
            }
            1 => {
                app.run("type.characterStyle.set", json!({"id":id,"attrs":{"size":42}})).unwrap();
            }
            2 => {
                app.run("type.characterStyle.delete", json!({"id":id})).unwrap();
            }
            3 => app.ui.type_panels.options = None,
            4 => {
                panel_docking::show_panel(&mut app, "characterStyles", Some(false)).unwrap();
            }
            // Another style open in Style Options: each styles panel is its own window, so showing the other one
            // leaves this one in front.
            _ => app.ui.type_panels.options = Some((false, id + 1)),
        }
        let doc = app.session.active().unwrap().doc.clone();
        check(&mut app, &ctx);
        assert!(app.framework.dialogs.stack().is_empty(), "change {change}");
        assert!(app.ui.type_panels.color_target.is_none());
        assert!(app.text_style_preview.is_none());
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
    }
}

#[test]
fn inherited_colors_and_invalid_open_requests_are_atomic() {
    let (mut app, _) = fixture(Kind::Character, 8);
    let inherited = app.run("type.characterStyle.new", json!({"fromSelection":false})).unwrap()["id"].as_u64().unwrap() as u32;
    let panels = app.ui.type_panels.clone();
    for (kind, id) in [(Kind::Character, 0), (Kind::Character, u32::MAX), (Kind::Character, inherited), (Kind::Paragraph, 0)] {
        assert!(open(&mut app, kind, id).is_err());
        assert_eq!(app.ui.type_panels, panels);
        assert!(app.framework.dialogs.stack().is_empty());
    }
    app.run("type.paragraphStyle.set", json!({"id":0,"attrs":{"color":"#112233"}})).unwrap();
    let picker = open(&mut app, Kind::Paragraph, 0).unwrap();
    set(&mut app, picker, "#abcdef");
    confirm_picker(&mut app, picker);
    assert_eq!(color_picker_ui::hex(color(&app.session.active().unwrap().doc, Kind::Paragraph, 0).unwrap().to_rgb()), "#abcdef");
}
