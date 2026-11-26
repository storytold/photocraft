use craft::ui::dialog::StackDialog;
use egui_kittest::Harness;

use super::*;

fn app() -> Box<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":1200,"height":800})).unwrap();
    PhotocraftApp::new(session, crate::Services::default())
}

/// A harness whose every pass runs the dialog host, as the shell does.
fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    Harness::<'static, Box<PhotocraftApp>>::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app)
}

fn open_dialog(h: &mut Harness<'static, Box<PhotocraftApp>>) -> DialogId {
    let id = open(h.state_mut()).unwrap();
    h.run_steps(2);
    id
}

fn set(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, field: &str, value: Value) -> Result<(), String> {
    let ctx = h.ctx.clone();
    let result = h.state_mut().framework.dialogs.set_json_field(&ctx, id, field, value);
    h.run_steps(2);
    result
}

/// Sets the group field after `edit` changed what the dialog reports for it.
fn set_group(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId, field: &str, edit: impl FnOnce(&mut Value)) {
    let mut group = fields(h, id)[field].clone();
    edit(&mut group);
    set(h, id, field, group).unwrap();
}

fn confirm(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) -> bool {
    let accepted = crate::dialogs::testing::confirm(h.state_mut(), id).is_ok();
    h.run_steps(3);
    accepted
}

fn cancel(h: &mut Harness<'static, Box<PhotocraftApp>>, id: DialogId) {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.cancel_by_id(&ctx, id);
    h.run_steps(3);
}

fn fields(h: &Harness<'static, Box<PhotocraftApp>>, id: DialogId) -> Value {
    h.state().framework.dialogs.get_by_id::<HostedGuideLayoutDialog>(id).unwrap().json_fields().unwrap()
}

fn shown(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<(Guides, Color32)> {
    preview(h.state(), &h.state().session.active().unwrap().doc)
}

fn guides(h: &Harness<'static, Box<PhotocraftApp>>) -> Guides {
    h.state().session.active().unwrap().doc.guides.clone()
}

#[test]
fn applying_a_previewed_layout_reveals_hidden_guides() {
    for hidden in ["extras", "canvas", "guides"] {
        let mut app = app();
        match hidden {
            "extras" => app.ui.view.extras = false,
            "canvas" => app.ui.view.show.canvas_guides = false,
            _ => app.ui.extras.guides = false,
        }
        let mut h = harness(app);
        let id = open_dialog(&mut h);
        assert!(shown(&h).is_some(), "the dialog shows a preview despite hidden {hidden}");
        assert!(confirm(&mut h, id));
        assert!(!guides(&h).vertical.is_empty(), "the layout was actually created");
        assert!(shown(&h).is_none(), "the temporary preview is gone");
        let app = h.state();
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
        let mut h = harness(app);
        let id = open_dialog(&mut h);
        match action {
            "cancel" => cancel(&mut h, id),
            "invalid" => {
                assert!(set(&mut h, id, "target", json!("selectedArtboards")).is_err(), "no artboard is selected");
                cancel(&mut h, id);
            }
            _ => {
                set_group(&mut h, id, "columns", |c| c["enabled"] = json!(false));
                set(&mut h, id, "marginEnabled", json!(false)).unwrap();
                assert!(!confirm(&mut h, id), "an empty layout is rejected");
                assert!(h.state().framework.dialogs.is_open::<HostedGuideLayoutDialog>());
            }
        }
        let g = guides(&h);
        assert!(g.vertical.is_empty() && g.horizontal.is_empty(), "{action}");
        let app = h.state();
        assert!(!app.ui.view.extras && !app.ui.view.show.canvas_guides && !app.ui.extras.guides, "{action} must not change visibility");
    }
}

#[test]
fn typing_previews_without_editing_and_cancel_restores_preferences_and_lines() {
    let mut app = app();
    app.run(COMMAND, json!({"margin":50})).unwrap();
    let original = app.session.active().unwrap().doc.guides.clone();
    let revision = app.session.active().unwrap().revision;
    let color = app.session.prefs().guides_grid_and_slices.guide_color.clone();
    let mut h = harness(app);
    let id = open_dialog(&mut h);
    set_group(&mut h, id, "columns", |c| c["number"] = json!(5));
    set_group(&mut h, id, "rows", |r| {
        r["enabled"] = json!(true);
        r["number"] = json!(3);
    });
    set(&mut h, id, "color", json!({"r": 1.0, "g": 0.0, "b": 1.0, "a": 1.0})).unwrap();
    set(&mut h, id, "clearExisting", json!(true)).unwrap();
    let (pending, shown_color) = shown(&h).unwrap();
    assert_ne!(pending, original);
    assert_eq!(shown_color, Color32::from_rgb(255, 0, 255));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.prefs().guides_grid_and_slices.guide_color, color);
    cancel(&mut h, id);
    assert!(shown(&h).is_none());
    assert_eq!(guides(&h), original);
    assert_eq!(h.state().session.prefs().guides_grid_and_slices.guide_color, color);
}

#[test]
fn ok_matches_preview_and_is_one_undo_step() {
    let mut app = app();
    app.run(COMMAND, json!({"columns":2})).unwrap();
    let original = app.session.active().unwrap().doc.guides.clone();
    let mut h = harness(app);
    let id = open_dialog(&mut h);
    set_group(&mut h, id, "columns", |c| c["number"] = json!(3));
    set_group(&mut h, id, "rows", |r| {
        r["enabled"] = json!(true);
        r["number"] = json!(2);
    });
    set(&mut h, id, "centerColumns", json!(true)).unwrap();
    set(&mut h, id, "clearExisting", json!(true)).unwrap();
    set(&mut h, id, "color", json!({"r": 1.0, "g": 0.0, "b": 1.0, "a": 1.0})).unwrap();
    let expected = shown(&h).unwrap().0;
    assert!(confirm(&mut h, id));
    assert_eq!(guides(&h), expected);
    assert_eq!(h.state().session.prefs().guides_grid_and_slices.guide_color, "#ff00ff");
    assert!(shown(&h).is_none());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(guides(&h), original);
}

#[test]
fn preview_off_and_foreign_document_never_replace_guides() {
    let mut h = harness(app());
    let id = open_dialog(&mut h);
    set(&mut h, id, "preview", json!(false)).unwrap();
    assert!(shown(&h).is_none());
    set(&mut h, id, "preview", json!(true)).unwrap();
    assert!(shown(&h).is_some());
    h.state_mut().run("file.new", json!({"width":50,"height":50})).unwrap();
    assert!(shown(&h).is_none(), "the layout belongs to the document it opened on");
    confirm(&mut h, id);
    let g = guides(&h);
    assert!(g.vertical.is_empty() && g.horizontal.is_empty(), "the other document is untouched");
}

#[test]
fn invalid_fields_keep_last_valid_preview_and_dialog_open() {
    let mut h = harness(app());
    let id = open_dialog(&mut h);
    let expected = shown(&h).unwrap().0;
    // Seven gutters of 1000 px cannot fit 1200 px.
    set_group(&mut h, id, "columns", |c| c["gutter"]["value"] = json!(1000));
    assert!(!confirm(&mut h, id));
    assert!(h.state().framework.dialogs.is_open::<HostedGuideLayoutDialog>());
    assert_eq!(shown(&h).unwrap().0, expected);
}

/// A unitless length is remembered as the length it applied, so a later ruler unit change does not reinterpret it.
#[test]
fn remembered_lengths_keep_their_size_when_the_ruler_unit_changes() {
    let mut app = app();
    app.ui.view.guide_layout = json!({"gutter": 5, "margin": [5, 5, 5, 5]});
    let mut h = harness(app);
    let id = open_dialog(&mut h);
    assert!(confirm(&mut h, id));
    h.state_mut().session.prefs.edit(|p| p.units_and_rulers.rulers = Unit::Millimeters);
    let id = open_dialog(&mut h);
    let f = fields(&h, id);
    let mm = 5.0 / 72.0 * 25.4;
    assert!((f["columns"]["gutter"]["value"].as_f64().unwrap() - mm).abs() < 1e-3, "{f}");
    assert!((f["margin"][0]["value"].as_f64().unwrap() - mm).abs() < 1e-3, "{f}");
}

/// Percent is of each selected artboard, not of the document: 50% of a 140 x 40 board is a 70 px column, centred in
/// it; the field reopens still in percent.
#[test]
fn percent_is_of_the_selected_artboard_and_stays_percent() {
    let mut app = app();
    app.run("layer.new.artboard", json!({"rect": [200, 100, 140, 40]})).unwrap();
    app.ui.view.guide_layout = json!({
        "target": "selectedArtboards", "columns": 1, "width": "50%", "centerColumns": true, "clearExisting": true,
        "rows": 0, "marginEnabled": false,
    });
    let mut h = harness(app);
    let id = open_dialog(&mut h);
    assert_eq!(fields(&h, id)["target"], "selectedArtboards");
    assert!(confirm(&mut h, id));
    let g = guides(&h);
    assert!(g.vertical.contains(&235.0) && g.vertical.contains(&305.0), "{:?}", g.vertical);
    let id = open_dialog(&mut h);
    assert_eq!(fields(&h, id)["columns"]["size"]["value"], 50.0);
}
