use super::*;

#[test]
fn applying_a_previewed_layout_reveals_hidden_guides() {
    for hidden in ["extras", "canvas", "guides"] {
        let mut app = app();
        match hidden {
            "extras" => app.ui.view.extras = false,
            "canvas" => app.ui.view.show.canvas_guides = false,
            _ => app.ui.extras.guides = false,
        }
        let id = open(&mut app);
        let doc = &app.session.active().unwrap().doc;
        assert!(preview(&app, doc).is_some(), "the dialog shows a preview despite hidden {hidden}");
        crate::dialogs::confirm(&mut app, id).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert!(!doc.guides.vertical.is_empty(), "the layout was actually created");
        assert!(preview(&app, doc).is_none(), "the temporary preview is gone");
        assert!(app.ui.view.shows(app.ui.view.show.canvas_guides) && app.ui.extras.guides, "applied guides must remain visible when {hidden} was hidden");
    }
}

#[test]
fn cancelling_invalid_and_empty_layouts_keep_guides_hidden() {
    for action in ["cancel", "invalid", "empty"] {
        let mut app = app();
        app.ui.view.extras = false;
        app.ui.view.show.canvas_guides = false;
        app.ui.extras.guides = false;
        let id = open(&mut app);
        match action {
            "cancel" => {
                crate::dialogs::cancel(&mut app, id).unwrap();
            }
            "invalid" => {
                app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("invalid"));
                assert!(crate::dialogs::confirm(&mut app, id).is_err());
            }
            _ => {
                let f = &mut app.ui.dialog_mut(id).unwrap().fields;
                for key in ["__columnsEnabled", "__rowsEnabled", "__marginEnabled"] {
                    f.insert(key.into(), json!(false));
                }
                f.insert("clearExisting".into(), json!(true));
                assert!(crate::dialogs::confirm(&mut app, id).is_err(), "an empty layout is rejected");
            }
        }
        let guides = &app.session.active().unwrap().doc.guides;
        assert!(guides.vertical.is_empty() && guides.horizontal.is_empty(), "{action}");
        assert!(!app.ui.view.extras && !app.ui.view.show.canvas_guides && !app.ui.extras.guides, "{action} must not change visibility");
    }
}

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

/// The disabled Number field must not clamp a zero count to 1 and so switch its axis on (#5625).
#[test]
fn disabled_number_fields_do_not_turn_their_axis_on() {
    let mut app = app();
    let id = open(&mut app);
    let mut h = harness(app);
    h.run_steps(3);
    let f = &h.state().ui.dialogs.first().unwrap().fields;
    assert_eq!(f["rows"], 0, "the default layout has no rows");
    assert!(!on(f, "__rowsEnabled", positive(f, "rows")), "Rows stays off");
    let mut app = h.into_state();
    crate::dialogs::confirm(&mut app, id).unwrap();
    let guides = &app.session.active().unwrap().doc.guides;
    assert!(!guides.vertical.is_empty(), "the columns were created");
    assert!(guides.horizontal.is_empty(), "no row guides at the top and bottom");

    let mut app = self::app();
    let id = open(&mut app);
    app.ui.dialog_mut(id).unwrap().fields.insert("columns".into(), json!(0));
    let mut h = harness(app);
    h.run_steps(3);
    let f = &h.state().ui.dialogs.first().unwrap().fields;
    assert_eq!(f["columns"], 0);
    assert!(!on(f, "__columnsEnabled", positive(f, "columns")), "Columns stays off");
}

/// New Guide Layout offers New Guide's colours, by value, and keeps another one as Custom.
#[test]
fn color_list_is_new_guides() {
    let hexes: Vec<String> = color_options("#4affff").into_iter().map(|(hex, _)| hex).collect();
    assert_eq!(hexes, ["#3366ff", "#ff3333", "#33ff33", "#3333ff", "#ffcc00", "#ff00ff", "#4affff", "#e7e7e7", "#000000"]);
    assert_eq!(color_options("#4affff")[6].1, "Cyan", "the default guide colour reads as Cyan");
    let custom = color_options("#123456");
    assert_eq!((custom.len(), custom[9].0.as_str(), custom[9].1), (10, "#123456", "Custom"));
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

#[test]
fn alt_reset_restores_factory_layout() {
    let mut app = app();
    let id = open(&mut app);
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("columns".into(), json!(3));
    fields.insert("gutter".into(), json!(99));
    fields.insert("__preset".into(), json!("twoColumn"));
    reset(&app, &mut fields);
    app.ui.dialog_mut(id).unwrap().fields = fields;
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["columns"], json!(8));
    assert_eq!(f["gutter"], json!(20));
    assert_eq!(f["__preset"], json!("custom"));
    assert!(app.ui.dialogs.iter().any(|d| d.id == id));
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

/// The Columns and Rows group boxes and the OK button, in one run of the dialog in `dir`, plus
/// the dialog frame's centre x — the axis a mirrored run must mirror about.
fn mirrored_rows(dir: crate::text::direction::Dir) -> (egui::Rect, egui::Rect, egui::Rect, f32) {
    use egui_kittest::kittest::Queryable;
    let _dir = crate::text::direction::direction_scope(dir);
    let mut app = app();
    let id = open(&mut app);
    let mut h = harness(app);
    h.run_steps(3);
    let frame = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog frame");
    (h.get_by_label("Columns").rect(), h.get_by_label("Rows").rect(), h.get_by_label("OK").rect(), frame.center().x)
}

/// In a right-to-left UI the body reads from the right — the Columns group leads, the Rows group
/// follows toward the reading end and the action rail moves to the left — while only the
/// direction is forced: the labels and data stay the ones the left-to-right run pins.
#[test]
fn the_body_rows_mirror_in_a_right_to_left_ui() {
    let (l_columns, l_rows, l_ok, l_axis) = mirrored_rows(crate::text::direction::Dir::Ltr);
    assert!(l_columns.right() < l_rows.left(), "LTR: the Columns group leads: {l_columns:?} / {l_rows:?}");
    assert!(l_ok.left() > l_rows.right(), "LTR: the rail follows the fields: {l_ok:?} / {l_rows:?}");

    let (r_columns, r_rows, r_ok, r_axis) = mirrored_rows(crate::text::direction::Dir::Rtl);
    assert!((l_axis - r_axis).abs() < 1.0, "the dialog sits at the same place in both directions: {l_axis} / {r_axis}");
    assert!(r_columns.left() > r_rows.right(), "RTL: the Columns group still leads, from the right: {r_columns:?} / {r_rows:?}");
    assert!(r_ok.right() < r_rows.left(), "RTL: the rail leads at the reading end: {r_ok:?} / {r_rows:?}");
    // Every row mirrors about one common axis (the dialog frame's centre need not be it: the
    // host's own chrome is not mirrored).
    let axis = (l_columns.center().x + r_columns.center().x) / 2.0;
    for (name, ltr, rtl) in [("Columns", l_columns, r_columns), ("Rows", l_rows, r_rows), ("OK", l_ok, r_ok)] {
        assert!((ltr.center().x + rtl.center().x - 2.0 * axis).abs() < 2.0, "{name} mirrors about the same axis {axis}: {ltr:?} / {rtl:?}");
    }
}

/// Each settings label the `Grid` replaced, its rect and the same-named editor beside it on the
/// same row, plus the Number fields' shown values, in one run of the dialog in `dir`.
fn label_editor_pairs(dir: crate::text::direction::Dir) -> (Vec<(&'static str, egui::Rect, egui::Rect)>, Vec<String>) {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let _dir = crate::text::direction::direction_scope(dir);
    let mut app = app();
    let _id = open(&mut app);
    let mut h = harness(app);
    h.run_steps(3);
    // The editors by their accessible label (a `:` caption names its dropdown without it).
    let mut editors: Vec<(&str, egui::Rect)> = Vec::new();
    for role in [Role::SpinButton, Role::TextInput, Role::ComboBox] {
        for text in ["Preset:", "Target", "Number", "Width", "Height", "Spacing"] {
            for n in h.query_all_by_role_and_label(role, text) {
                editors.push((text.trim_end_matches(':'), n.rect()));
            }
        }
    }
    let mut pairs = Vec::new();
    for text in ["Preset:", "Target:", "Number", "Width", "Height", "Spacing"] {
        let key = text.trim_end_matches(':');
        for label in h.query_all_by_role_and_label(Role::Label, text) {
            let label = label.rect();
            // Same row, same label, nearest horizontally: the two axes' rows share their y.
            let editor = editors
                .iter()
                .filter(|(l, r)| *l == key && (r.center().y - label.center().y).abs() < 5.0)
                .map(|(_, r)| *r)
                .min_by(|a, b| (a.center().x - label.center().x).abs().total_cmp(&(b.center().x - label.center().x).abs()))
                .unwrap_or_else(|| panic!("no editor beside the {text} label"));
            pairs.push((text, label, editor));
        }
    }
    let values = h.query_all_by_role_and_label(Role::SpinButton, "Number").filter_map(|n| n.value()).collect();
    (pairs, values)
}

/// In a right-to-left UI every settings label sits at the reading start of its editor — the label
/// on the right, the editor toward the reading end — while the values stay the LTR run's ones.
#[test]
fn the_settings_rows_mirror_their_label_editor_order_in_a_right_to_left_ui() {
    let (ltr_pairs, ltr_values) = label_editor_pairs(crate::text::direction::Dir::Ltr);
    assert!(!ltr_pairs.is_empty());
    for (text, label, editor) in &ltr_pairs {
        assert!(label.right() <= editor.left() + 1.0, "LTR: the {text} label leads its editor: {label:?} / {editor:?}");
    }

    let (rtl_pairs, rtl_values) = label_editor_pairs(crate::text::direction::Dir::Rtl);
    assert_eq!(rtl_pairs.len(), ltr_pairs.len(), "the same rows in both directions");
    for (text, label, editor) in &rtl_pairs {
        assert!(editor.right() <= label.left() + 1.0, "RTL: the {text} label sits at the reading start: {label:?} / {editor:?}");
    }
    assert_eq!(ltr_values, rtl_values, "only the direction changed, not the values: {ltr_values:?} / {rtl_values:?}");
}
