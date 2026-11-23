use super::bars::count_size_params;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

fn app() -> (Box<PhotocraftApp>, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = app.ctx().clone();
    (app, ctx)
}

fn down(app: &mut PhotocraftApp, x: f64, y: f64, m: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
}
fn mv(app: &mut PhotocraftApp, x: f64, y: f64, m: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, m);
}
fn up(app: &mut PhotocraftApp, x: f64, y: f64) {
    crate::canvas::tool_event(app, ToolEvent::Up { x, y }, egui::Modifiers::NONE);
}

fn ruler(app: &PhotocraftApp) -> Option<photocraft_doc::Ruler> {
    app.session.active().unwrap().doc.measurement.ruler
}

#[test]
fn arbitrary_rotation_starts_at_the_ruler_angle() {
    let (mut app, ctx) = app();
    // The dialog opens on the host; its values are what the prefill rule produced. Closing it keeps
    // the next open from replacing a dialog that is still up.
    let open = |app: &mut PhotocraftApp| {
        crate::menus::invoke(app, &ctx, "image.rotation.arbitrary", json!({})).unwrap();
        let shown = app.framework.dialogs.open_dialogs();
        let dialog = shown.last().expect("the Arbitrary rotation dialog is open on the host");
        let f = dialog.fields.clone().expect("the dialog lists its values");
        app.framework.dialogs.cancel_by_id(&ctx, dialog.id);
        (f["angle"].as_f64().unwrap(), f["direction"].clone())
    };
    assert_eq!(open(&mut app), (0.0, json!("cw")));
    // A line falling to the right straightens counter-clockwise, a near-vertical one to the y axis.
    for (end, dir) in [([150, 60], "ccw"), ([150, 40], "cw"), ([60, -50], "ccw")] {
        app.run("image.analysis.rulerTool", json!({"start": [50, 50], "end": end})).unwrap();
        let (a, d) = open(&mut app);
        assert!((a - 5.7106).abs() < 1e-3 && d == json!(dir), "{end:?}: {a} {d}");
    }
}

#[test]
fn count_tool_add_move_remove() {
    let (mut app, _) = app();
    app.ui.tool = Tool::Count;
    down(&mut app, 20.0, 20.0, egui::Modifiers::NONE);
    up(&mut app, 20.0, 20.0);
    down(&mut app, 50.0, 50.0, egui::Modifiers::NONE);
    up(&mut app, 50.0, 50.0);
    let pts = |app: &PhotocraftApp| app.session.active().unwrap().doc.measurement.count_groups[0].points.clone();
    assert_eq!(pts(&app).len(), 2);
    let past = |app: &PhotocraftApp| app.session.active().unwrap().history.past_len();
    // A move under half a pixel sends no `count.move`: the marker and the history stay.
    let before = past(&app);
    down(&mut app, 20.0, 20.0, egui::Modifiers::NONE);
    mv(&mut app, 20.3, 20.0, egui::Modifiers::NONE);
    up(&mut app, 20.3, 20.0);
    assert_eq!(pts(&app)[0], [20.0, 20.0]);
    assert_eq!(past(&app), before, "a sub-pixel move is no step");
    // A longer drag moves marker 1 in one step.
    down(&mut app, 20.5, 20.5, egui::Modifiers::NONE);
    up(&mut app, 30.0, 25.0);
    assert_eq!(pts(&app)[0], [30.0, 25.0]);
    assert_eq!(past(&app), before + 1, "the move is one step");
    // ⌥-click removes.
    down(&mut app, 50.0, 50.0, egui::Modifiers::ALT);
    up(&mut app, 50.0, 50.0);
    assert_eq!(pts(&app).len(), 1);
}

/// The Count tool's cursor mark and its press share one hit test (#11773): a press at each distance
/// from a marker moves the marker exactly where `count_marker_at` finds one, and adds a marker
/// elsewhere.
#[test]
fn over_a_marker_is_the_clicks_own_hit_test() {
    for dx in [0.0, 2.0, 4.0, 6.0, 6.9, 7.1, 8.0, 15.0] {
        let (mut app, _) = app();
        app.ui.tool = Tool::Count;
        app.run("count.add", json!({"x": 50, "y": 50})).unwrap();
        let at = [50.0 + dx, 50.0];
        // The tool's reach, as the press and the cursor pass it to `count_marker_at`.
        let hit = super::canvas::count_marker_at(&app, at, app.tool_reach());
        down(&mut app, at[0], at[1], egui::Modifiers::NONE);
        up(&mut app, at[0], at[1] + 10.0);
        let pts = app.session.active().unwrap().doc.measurement.count_groups[0].points.clone();
        match hit {
            Some(_) => {
                // The pointer position passes through f32 on its way to the marker.
                assert!(
                    pts.len() == 1 && (pts[0][0] - at[0]).abs() < 1e-4 && (pts[0][1] - (at[1] + 10.0)).abs() < 1e-4,
                    "{dx} px: the press moved the marker the cursor marked: {pts:?}"
                );
            }
            None => assert_eq!(pts.len(), 2, "{dx} px: the press added a marker where the cursor marked none"),
        }
        if dx == 0.0 {
            assert!(hit.is_some(), "a press on the marker's centre hits it");
        }
        if dx == 15.0 {
            assert!(hit.is_none(), "15 px away is no hit");
        }
    }
}

#[test]
fn note_tool_and_panel_state() {
    let (mut app, _) = app();
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_author = "QA".into();
    down(&mut app, 30.0, 40.0, egui::Modifiers::NONE);
    up(&mut app, 30.0, 40.0);
    let n = app.session.active().unwrap().doc.notes.clone();
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].author, "QA");
    assert!(crate::panel_docking::is_shown(&app, "notes") && app.ui.analysis.note_selected == Some(0));
    // Drag the icon.
    down(&mut app, 32.0, 42.0, egui::Modifiers::NONE);
    up(&mut app, 62.0, 52.0);
    assert_eq!(app.session.active().unwrap().doc.notes[0].position, [60.0, 50.0]);
}

/// Clicking a note with the Note tool shows its author and colour in the options bar (#7940),
/// so a note from a PSD reads as itself, not as the next note's defaults.
#[test]
fn clicking_a_note_shows_its_author_and_color() {
    let (mut app, _ctx) = app();
    app.run("notes.add", json!({"x": 20, "y": 20, "author": "Ann", "color": [1.0, 0.5, 0.75]})).unwrap();
    app.run("notes.add", json!({"x": 120, "y": 80, "author": "Bob", "color": [0.25, 0.5, 1.0]})).unwrap();
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_author = "Me".into();
    app.ui.analysis.note_color = [1.0, 1.0, 0.51];
    down(&mut app, 22.0, 22.0, egui::Modifiers::NONE);
    up(&mut app, 22.0, 22.0);
    assert_eq!(app.ui.analysis.note_selected, Some(0));
    assert_eq!((app.ui.analysis.note_author.as_str(), app.ui.analysis.note_color), ("Ann", [1.0, 0.5, 0.75]));
    down(&mut app, 122.0, 82.0, egui::Modifiers::NONE);
    up(&mut app, 122.0, 82.0);
    assert_eq!((app.ui.analysis.note_author.as_str(), app.ui.analysis.note_color), ("Bob", [0.25, 0.5, 1.0]));
    // A click off the notes adds one with what the bar now shows, and leaves the notes as they were.
    down(&mut app, 170.0, 40.0, egui::Modifiers::NONE);
    up(&mut app, 170.0, 40.0);
    let notes = app.session.active().unwrap().doc.notes.clone();
    assert_eq!(notes.iter().map(|n| n.author.as_str()).collect::<Vec<_>>(), ["Ann", "Bob", "Bob"]);
}

#[test]
fn color_sampler_readout_and_crop_removal_report() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    for (x, y) in [(5, 5), (150, 90)] {
        app.run("view.colorSamplers.add", json!({"x": x, "y": y})).unwrap();
    }
    // The list reports both, numbered, with their position and RGB/CMYK readout.
    let l = app.run("view.colorSamplers.list", json!({})).unwrap();
    assert_eq!(l["samplers"].as_array().unwrap().len(), 2);
    assert_eq!(l["samplers"][0]["number"], 1);
    assert_eq!(l["samplers"][1]["position"], json!([150.5, 90.5]));
    assert!(l["samplers"][0]["rgb"].as_array().is_some_and(|a| a.len() == 3));
    // Cropping to the top-left drops the outside point and reports how many went.
    app.run("image.crop", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(app.session.active().unwrap().color_samplers, vec![[5.5, 5.5]]);
    assert!(app.ui.status.contains("removed"), "{}", app.ui.status);
    assert!(!app.ui.status_error);
}

#[test]
fn color_sampler_is_per_document_and_inert_without_one() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    app.run("view.colorSamplers.add", json!({"x": 10, "y": 10})).unwrap();
    // A second document starts empty; the first keeps its own point.
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    assert!(app.session.active().unwrap().color_samplers.is_empty());
    app.run("view.colorSamplers.add", json!({"x": 20, "y": 20})).unwrap();
    assert_eq!(app.session.active().unwrap().color_samplers, vec![[20.5, 20.5]]);
    app.run("document.activate", json!({"document": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().color_samplers, vec![[10.5, 10.5]]);
    // With no document no sampler is found and the commands error.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::ColorSampler;
    assert_eq!(super::sampler_at(&empty, [5.0, 5.0]), None);
    assert!(empty.run("view.colorSamplers.add", json!({"x": 1, "y": 1})).is_err());
    assert!(empty.run("view.colorSamplers.list", json!({})).is_err());
}

#[test]
fn count_size_fields_send_whole_numbers_the_engine_applies() {
    let p = count_size_params(7.6, 20.0);
    assert_eq!(p, json!({"markerSize": 8, "labelSize": 20}));
    assert!(p["markerSize"].is_u64() && p["labelSize"].is_u64());
    assert_eq!(count_size_params(f32::NAN, 500.0), json!({"markerSize": 1, "labelSize": 72}));
    let (mut app, _ctx) = app();
    app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    app.run("count.setGroup", count_size_params(8.0, 20.0)).unwrap();
    let g = &app.session.active().unwrap().doc.measurement.count_groups[0];
    assert_eq!((g.marker_size, g.label_size), (8, 20));
}

// ---------------------------------------------------------------- Straighten Layer button (#12033)

/// The Ruler options bar, drawn wide enough for its buttons to show.
fn ruler_bar(mut app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    use egui::vec2;
    app.ui.tool = Tool::Ruler;
    let ctx = app.ctx().clone();
    let mut h = egui_kittest::Harness::builder().with_context(ctx).with_size(vec2(1400.0, 60.0)).build_ui_state(
        |ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

/// A ruler line to straighten by, without one the button is off for every target.
fn rule(app: &mut PhotocraftApp) {
    app.run("image.analysis.rulerTool", json!({"start": [10, 30], "end": [150, 25]})).unwrap();
}

/// How many dialogs are open on the host.
fn open_dialogs(app: &PhotocraftApp) -> usize {
    app.framework.dialogs.open_dialogs().len()
}

/// A new layer on top of the plain document `app()` makes.
fn add_layer(app: &mut PhotocraftApp, name: &str) -> u64 {
    app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap()
}

fn on_a_layer(app: &mut PhotocraftApp) {
    add_layer(app, "Paint");
}

/// A refused click says so. `PhotocraftApp::run` reports what the *command* answered in the status
/// line, but a refusal raised before it runs - the automation gate a control/MCP-driven click goes
/// through - returns early, so the button has to report it itself. As the toolbar flyout does
/// (`a_failed_edit_toolbar_from_the_flyout_reports_its_error_in_the_status_bar`).
#[test]
fn a_denied_synthetic_click_reports_the_refusal_instead_of_saying_nothing() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let (mut app, _) = app();
    on_a_layer(&mut app);
    rule(&mut app);
    app.automation_input = true;
    app.services.automation_command =
        Some(Box::new(|id, _| if id == "image.analysis.straightenLayer" { Err("straightening is denied".into()) } else { Ok(()) }));
    let mut h = ruler_bar(app);
    assert!(!h.get_by_label("Straighten Layer").accesskit_node().is_disabled(), "the layer is a valid target");
    let steps = h.state().session.active().unwrap().history.past_len();
    h.get_by_label("Straighten Layer").click();
    h.run_steps(2);
    let st = h.state();
    assert_eq!(st.ui.status, "straightening is denied", "the refusal reaches the status line");
    assert!(st.ui.status_error, "as an error");
    assert_eq!(open_dialogs(st), 0, "a refusal opens no alert");
    let now = st.session.active().unwrap();
    assert!(now.doc.measurement.ruler.is_some(), "nothing ran: the ruler line is still up");
    assert_eq!(now.history.past_len(), steps, "and nothing was recorded");
}

/// #12033: a hidden target layer is the one state that keeps its button. The click answers with the
/// reference's alert - its measured message and an OK button - instead of the status line, and the
/// document, its History and the ruler line stay as they were.
#[test]
fn a_hidden_layer_keeps_the_button_and_the_click_shows_the_alert() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let (mut app, _) = app();
    let l = app.run("layer.new.layer", json!({"name": "Gone"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.setProps", json!({"layer": l, "visible": false})).unwrap();
    rule(&mut app);
    let mut h = ruler_bar(app);
    assert!(!h.get_by_label("Straighten Layer").accesskit_node().is_disabled(), "a hidden layer keeps the button on");
    let (before, steps) = (h.state().session.active().unwrap().doc.clone(), h.state().session.active().unwrap().history.past_len());
    h.get_by_label("Straighten Layer").click();
    h.run_steps(2);
    let st = h.state();
    assert_eq!(open_dialogs(st), 1, "one alert on the host");
    let now = st.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &now.doc), "the document was not touched");
    assert_eq!(now.history.past_len(), steps, "no History step");
    assert!(ruler(st).is_some(), "the ruler line is still there");
}

/// The messages of the alerts open on the host.
fn alerts(app: &PhotocraftApp) -> Vec<String> {
    app.framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter(|d| d.key.as_deref() == Some("alert"))
        .filter_map(|d| Some(d.fields.as_ref()?["message"].as_str()?.to_string()))
        .collect()
}

/// Closes every alert the host has open.
fn close_alerts(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let ids: Vec<_> = app.framework.dialogs.open_dialogs().into_iter().filter(|d| d.key.as_deref() == Some("alert")).map(|d| d.id).collect();
    for id in ids {
        app.framework.dialogs.cancel_by_id(ctx, id);
    }
}

#[test]
fn placing_color_sampler_reveals_closed_info_panel() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    // Close the Info panel.
    let _ = crate::panel_docking::show_panel(&mut app, "info", Some(false));
    assert!(!crate::panel_docking::is_checked(&app, "info"));

    // Placing a sampler inside the canvas opens the Info panel (#11745).
    down(&mut app, 25.0, 30.0, egui::Modifiers::NONE);
    up(&mut app, 25.0, 30.0);
    assert!(crate::panel_docking::is_checked(&app, "info"));

    // Placing a second sampler while the panel is already open leaves it open without error.
    down(&mut app, 45.0, 50.0, egui::Modifiers::NONE);
    up(&mut app, 45.0, 50.0);
    assert!(crate::panel_docking::is_checked(&app, "info"));

    // A click on an existing sampler (starting a move or selection) does not reopen a closed panel.
    let _ = crate::panel_docking::show_panel(&mut app, "info", Some(false));
    assert!(!crate::panel_docking::is_checked(&app, "info"));
    down(&mut app, 25.0, 30.0, egui::Modifiers::NONE);
    up(&mut app, 25.0, 30.0);
    assert!(!crate::panel_docking::is_checked(&app, "info"));
}

#[test]
fn placing_color_sampler_in_docked_workspace_leaves_info_panel_open() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    // Dock the Info panel in an expanded column and close it.
    app.run("window.panel.move", json!({"panel": "info", "anchor": "layers", "zone": "top"})).unwrap();
    let _ = crate::panel_docking::show_panel(&mut app, "info", Some(false));
    assert!(!crate::panel_docking::is_checked(&app, "info"));

    // Placing a sampler inside the canvas reveals the Info panel (#11745).
    down(&mut app, 25.0, 30.0, egui::Modifiers::NONE);
    up(&mut app, 25.0, 30.0);
    assert!(crate::panel_docking::is_checked(&app, "info"));
}

#[test]
fn color_sampler_limit_opens_alert_dialog_instead_of_status_note() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    for i in 0..10 {
        let x = 10.0 + (i as f64) * 10.0;
        down(&mut app, x, 20.0, egui::Modifiers::NONE);
        up(&mut app, x, 20.0);
    }
    assert_eq!(app.session.active().unwrap().color_samplers.len(), 10);
    assert!(alerts(&app).is_empty());

    // Prepare non-empty status and closed Info panel to verify state is cleared and panel is untouched.
    app.ui.status = "Previous status".into();
    app.ui.status_error = true;
    let _ = crate::panel_docking::show_panel(&mut app, "info", Some(false));
    assert!(!crate::panel_docking::is_checked(&app, "info"));

    // The 11th click opens the limit alert dialog instead of a status-bar note (#11743).
    down(&mut app, 150.0, 20.0, egui::Modifiers::NONE);
    up(&mut app, 150.0, 20.0);
    assert_eq!(app.session.active().unwrap().color_samplers.len(), 10);
    assert_eq!(app.ui.status, "");
    assert!(!app.ui.status_error);
    // Limit rejection does not reveal or change the Info panel.
    assert!(!crate::panel_docking::is_checked(&app, "info"));

    let dialogs = alerts(&app);
    assert_eq!(dialogs, ["Could not create a new sampler because the maximum number of samplers is 10."]);
    assert_eq!(dialogs, [super::canvas::MAX_SAMPLERS_ALERT]);
}

#[test]
fn color_sampler_limit_alert_also_for_alt_click_off_a_point() {
    // Measured in the reference application: Alt+click off a point adds a point like a plain
    // click, so at 10 points it opens the same alert; Alt+click on a point deletes it, no alert.
    let (mut app, ctx) = app();
    app.ui.tool = Tool::ColorSampler;
    for i in 0..10 {
        let x = 10.0 + (i as f64) * 10.0;
        down(&mut app, x, 20.0, egui::Modifiers::NONE);
        up(&mut app, x, 20.0);
    }
    down(&mut app, 150.0, 20.0, egui::Modifiers::ALT);
    up(&mut app, 150.0, 20.0);
    assert_eq!(app.session.active().unwrap().color_samplers.len(), 10);
    let dialogs = alerts(&app);
    assert_eq!(dialogs, ["Could not create a new sampler because the maximum number of samplers is 10."]);
    assert_eq!(dialogs, [super::canvas::MAX_SAMPLERS_ALERT]);

    close_alerts(&mut app, &ctx);
    down(&mut app, 10.0, 20.0, egui::Modifiers::ALT);
    up(&mut app, 10.0, 20.0);
    assert_eq!(app.session.active().unwrap().color_samplers.len(), 9);
    assert!(alerts(&app).is_empty());
}

#[test]
fn color_sampler_drag_error_reports_to_status_bar_without_dialog() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    // Place a sampler
    down(&mut app, 25.0, 30.0, egui::Modifiers::NONE);
    up(&mut app, 25.0, 30.0);
    assert_eq!(app.session.active().unwrap().color_samplers.len(), 1);

    // Start dragging the sampler point
    down(&mut app, 25.0, 30.0, egui::Modifiers::NONE);
    // Clear samplers while drag is in flight
    app.run("view.colorSamplers.clear", json!({})).unwrap();

    // Release the drag inside canvas: move fails because point no longer exists
    up(&mut app, 50.0, 50.0);
    assert!(app.ui.status.contains("no sample point"), "status was: {}", app.ui.status);
    assert!(alerts(&app).is_empty());
}

/// The height of the number painted beside the first Count marker of a document whose group has `label_size`, read
/// from the frame's text shapes.
fn count_number_height(label_size: u32) -> f32 {
    use crate::app::framework::BuildApp as _;
    use egui::epaint::Shape;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("count.add", json!({"x": 100, "y": 50})).unwrap();
    app.run("count.setGroup", json!({"labelSize": label_size})).unwrap();
    app.sync_views();
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 600.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    h.run_steps(6);
    let mut heights = Vec::new();
    let mut stack: Vec<&Shape> = h.output().shapes.iter().map(|c| &c.shape).collect();
    while let Some(shape) = stack.pop() {
        match shape {
            Shape::Vec(v) => stack.extend(v.iter()),
            Shape::Text(t) if t.galley.text() == "1" => heights.push(t.galley.size().y),
            _ => {}
        }
    }
    heights.into_iter().fold(0.0, f32::max)
}

/// Label Size sets the size of the numbers beside the Count markers.
#[test]
fn count_label_size_sizes_the_marker_numbers() {
    let (small, large) = (count_number_height(8), count_number_height(40));
    assert!(small > 0.0, "the marker number is painted");
    assert!(large > small * 2.0, "a larger Label Size paints larger numbers: {small} then {large}");
}

/// A 300 x 200 document with two notes (the second without an author) and the Notes panel body in a harness,
/// nothing selected.
fn notes_harness() -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let (mut app, _) = app();
    app.run("notes.add", json!({"x": 20, "y": 20, "author": "Ann", "text": "first"})).unwrap();
    app.run("notes.add", json!({"x": 120, "y": 80, "author": "", "text": "second"})).unwrap();
    app.ui.analysis.note_selected = None;
    let mut h = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(400.0, 400.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            super::notes_panel::body(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

fn disabled(h: &egui_kittest::Harness<'static, Box<PhotocraftApp>>, label: &str) -> bool {
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    h.get_by_label(label).accesskit_node().is_disabled()
}

/// With no note selected the panel selects none by itself, shows an inactive empty text field, a blank counter and
/// greyed buttons; a selected note shows "N / M" and its stored author, and nothing for a missing one.
#[test]
fn notes_panel_empty_state_and_counter() {
    use egui_kittest::kittest::Queryable as _;
    let mut h = notes_harness();
    assert_eq!(h.state().ui.analysis.note_selected, None, "note 1 isn't selected by itself");
    assert!(h.query_by_label("1 / 2").is_none() && h.query_by_label("2 / 2").is_none(), "blank counter");
    assert!(disabled(&h, "Notes"), "the text field is inactive");
    for label in ["Previous note", "Next note", "Delete note"] {
        assert!(disabled(&h, label), "{label} is greyed out");
    }
    assert!(h.query_by_label("first").is_none(), "no note's text is shown");

    h.state_mut().ui.analysis.note_selected = Some(1);
    h.run_steps(3);
    assert!(h.query_by_label("2 / 2").is_some());
    assert!(h.query_by_label("No author").is_none(), "an empty author stays empty");
    assert!(!disabled(&h, "Next note") && !disabled(&h, "Delete note"));
    h.state_mut().ui.analysis.note_selected = Some(0);
    h.run_steps(3);
    assert!(h.query_by_label("1 / 2").is_some());
    assert!(h.query_by_label("Ann").is_some(), "the stored author");
}

/// A note made with a click takes the keyboard: typing goes into its text.
#[test]
fn a_note_made_with_a_click_takes_the_keyboard() {
    let mut h = notes_harness();
    h.state_mut().ui.tool = Tool::Note;
    down(h.state_mut(), 200.0, 150.0, egui::Modifiers::NONE);
    up(h.state_mut(), 200.0, 150.0);
    h.run_steps(3);
    assert_eq!(h.state().ui.analysis.note_selected, Some(2));
    h.event(egui::Event::Text("hello".into()));
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.notes[2].text, "hello", "typing goes into the new note");
}
