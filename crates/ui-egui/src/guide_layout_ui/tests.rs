use super::*;

#[test]
fn tab_includes_optional_sizes_and_shift_tab_selects_their_text() {
    let mut app = app();
    let id = open(&mut app);
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("columns".into(), json!(5));
    f.insert("rows".into(), json!(3));
    f.insert("width".into(), json!(100));
    f.insert("height".into(), json!(80));
    let mut h = harness(app);
    h.run_steps(3);
    let original = h.state().session.active().unwrap().doc.guides.clone();
    for text in ["4", "120", "12", "2", "90"] {
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
    }
    let f = &h.state().ui.dialogs.first().unwrap().fields;
    assert_eq!((f["columns"].as_u64(), f["rows"].as_u64()), (Some(4), Some(2)));
    assert_eq!(params(f)["width"], 120.0);
    assert_eq!(params(f)["height"], 90.0);
    assert_eq!(f["gutter"], 12.0);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
    h.run_steps(2);
    h.event(egui::Event::Text("3".into()));
    h.run_steps(2);
    assert_eq!(h.state().ui.dialogs.first().unwrap().fields["rows"], 3);
    assert_eq!(h.state().session.active().unwrap().doc.guides, original);
}

#[test]
fn tab_skips_disabled_rows_and_margins() {
    let mut app = app();
    let id = open(&mut app);
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    f.insert("columns".into(), json!(2));
    f.insert("__rowsEnabled".into(), json!(false));
    f.insert("__marginEnabled".into(), json!(false));
    let mut h = harness(app);
    h.run_steps(3);
    for text in ["5", "100", "4", "6"] {
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        h.event(egui::Event::Text(text.into()));
        h.run_steps(2);
    }
    let f = &h.state().ui.dialogs.first().unwrap().fields;
    assert_eq!(f["columns"], 6, "Tab wraps after column spacing, skipping disabled row and margin fields");
    assert_eq!(params(f)["width"], 100.0);
    assert_eq!(f["gutter"], 4.0);
}

fn harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut initialized = false;
    egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        move |ui, app| {
            if initialized {
                crate::dialogs::show(app, ui.ctx());
            } else {
                PhotocraftApp::setup_context(ui.ctx(), Default::default());
                initialized = true;
            }
        },
        app,
    )
}

fn app() -> PhotocraftApp {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":1200,"height":800})).unwrap();
    PhotocraftApp::new(session, crate::Services::default())
}

fn draw(app: &mut PhotocraftApp, id: u64) {
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() }, |ui| {
        body(app, ui, &mut f, false);
    })
    .textures_delta
    .clear();
    app.ui.dialog_mut(id).unwrap().fields = f;
}

#[test]
fn typing_previews_without_editing_and_cancel_restores_preferences_and_lines() {
    let mut app = app();
    app.run(COMMAND, json!({"margin":50})).unwrap();
    let original = app.session.active().unwrap().doc.guides.clone();
    let revision = app.session.active().unwrap().revision;
    let color = app.session.prefs().guides_grid_and_slices.guide_color.clone();
    let id = open(&mut app);
    for (k, v) in [
        ("columns", json!(5)),
        ("rows", json!(3)),
        ("gutter", json!(24)),
        ("margin", json!([20, 40, 60, 80])),
        ("color", json!("#ff00ff")),
        ("clearExisting", json!(true)),
    ] {
        app.ui.dialog_mut(id).unwrap().fields.insert(k.into(), v);
    }
    let (pending, shown_color) = preview(&app, &app.session.active().unwrap().doc).unwrap();
    assert_ne!(pending, original);
    assert_eq!(shown_color, Color32::from_rgb(255, 0, 255));
    assert_eq!(app.session.active().unwrap().revision, revision);
    assert_eq!(app.session.prefs().guides_grid_and_slices.guide_color, color);
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(preview(&app, &app.session.active().unwrap().doc).is_none());
    assert_eq!(app.session.active().unwrap().doc.guides, original);
    assert_eq!(app.session.prefs().guides_grid_and_slices.guide_color, color);
}

#[test]
fn ok_matches_preview_and_is_one_undo_step() {
    let mut app = app();
    app.run(COMMAND, json!({"columns":2})).unwrap();
    let original = app.session.active().unwrap().doc.guides.clone();
    let id = open(&mut app);
    for (k, v) in [
        ("columns", json!(3)),
        ("rows", json!(2)),
        ("width", json!("100*2")),
        ("height", json!(150)),
        ("centerColumns", json!(true)),
        ("clearExisting", json!(true)),
        ("color", json!("#ff00ff")),
    ] {
        app.ui.dialog_mut(id).unwrap().fields.insert(k.into(), v);
    }
    let expected = preview(&app, &app.session.active().unwrap().doc).unwrap().0;
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().doc.guides, expected);
    assert_eq!(app.session.prefs().guides_grid_and_slices.guide_color, "#ff00ff");
    assert!(preview(&app, &app.session.active().unwrap().doc).is_none());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.guides, original);
}

#[test]
fn preview_off_and_foreign_document_never_replace_guides() {
    let mut app = app();
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(false));
    assert!(preview(&app, &app.session.active().unwrap().doc).is_none());
    app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(true));
    app.run("file.new", json!({"width":50,"height":50})).unwrap();
    assert!(preview(&app, &app.session.active().unwrap().doc).is_none());
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
}

#[test]
fn invalid_fields_keep_last_valid_preview_and_dialog_open() {
    let mut app = app();
    let id = open(&mut app);
    draw(&mut app, id);
    let expected = preview(&app, &app.session.active().unwrap().doc).unwrap().0;
    app.ui.dialog_mut(id).unwrap().fields.insert("gutter".into(), json!(1000));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
    assert_eq!(preview(&app, &app.session.active().unwrap().doc).unwrap().0, expected);
    app.ui.dialog_mut(id).unwrap().fields.insert("gutter".into(), json!(20));
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("invalid"));
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    assert!(app.ui.dialog_mut(id).is_some());
}

#[test]
fn disabled_axes_preserve_values_and_do_not_validate_stale_inputs() {
    let mut app = app();
    let id = open(&mut app);
    let f = &mut app.ui.dialog_mut(id).unwrap().fields;
    for (k, v) in [("columns", json!(8)), ("width", json!("invalid")), ("gutter", json!(2000)), ("__columnsEnabled", json!(false)), ("rows", json!(3))] {
        f.insert(k.into(), v);
    }
    let p = params(f);
    assert_eq!(p["columns"], 0);
    assert_eq!(p["width"], Value::Null);
    assert_eq!(f["columns"], 8);
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.active().unwrap().doc.guides.vertical.is_empty());
    let reopened = open(&mut app);
    let f = &app.ui.dialog_mut(reopened).unwrap().fields;
    assert_eq!(f["columns"], 8);
    assert_eq!(f["__columnsEnabled"], false);
}

#[test]
fn parent_cancel_also_closes_guide_color_picker() {
    let mut app = app();
    let id = open(&mut app);
    let child = crate::color_picker_ui::open_for_field(&mut app, "Color Picker", [0.0, 1.0, 1.0], id, "color");
    assert!(crate::dialogs::confirm(&mut app, id).is_err());
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(app.ui.dialog_mut(child).is_none());
    assert!(app.ui.dialog_mut(id).is_none());
}

#[test]
fn automation_opens_the_same_guide_layout_dialog() {
    let mut app = app();
    app.ui.view.guide_layout["target"] = json!("selectedArtboards");
    let id = crate::dialogs::open_command_dialog(&mut app, COMMAND, "New Guide Layout");
    assert!(owns(&app.ui.dialog_mut(id).unwrap().fields));
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["target"], "document");
    assert!(preview(&app, &app.session.active().unwrap().doc).is_some());
}

#[test]
fn preview_cache_tracks_document_revision() {
    let mut app = app();
    let id = open(&mut app);
    draw(&mut app, id);
    assert!(!preview(&app, &app.session.active().unwrap().doc).unwrap().0.vertical.contains(&23.0));
    app.run(COMMAND, json!({"margin":23})).unwrap();
    assert!(preview(&app, &app.session.active().unwrap().doc).unwrap().0.vertical.contains(&23.0));
}
