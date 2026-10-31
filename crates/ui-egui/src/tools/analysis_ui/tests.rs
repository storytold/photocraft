use super::*;
use crate::direction::{Dir, direction_scope};
use crate::i18n::{self, Lang};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    (app, egui::Context::default())
}

/// Draw `f` for a few frames (fonts load on the first).
pub(crate) fn render(app: &mut PhotocraftApp, ctx: &egui::Context, f: fn(&mut PhotocraftApp, &egui::Context)) {
    PhotocraftApp::setup_context(ctx, Default::default());
    for _ in 0..3 {
        let mut o = ctx.run_ui(Default::default(), |ui| f(app, ui.ctx()));
        o.textures_delta.clear();
    }
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

fn ruler(app: &PhotocraftApp) -> Option<Ruler> {
    app.session.active().unwrap().doc.measurement.ruler
}

#[test]
fn menu_selects_tools_and_toggles_panels() {
    let (mut app, ctx) = app();
    crate::menus::invoke(&mut app, &ctx, "image.analysis.rulerTool", json!({})).unwrap();
    assert_eq!(app.ui.tool, Tool::Ruler);
    assert_eq!(checked(&app, "image.analysis.rulerTool"), Some(true));
    crate::menus::invoke(&mut app, &ctx, "image.analysis.countTool", json!({})).unwrap();
    assert_eq!(app.ui.tool, Tool::Count);
    crate::menus::invoke(&mut app, &ctx, "window.panel.measurementLog", json!({})).unwrap();
    assert!(app.ui.analysis.measurement_log);
    crate::menus::invoke(&mut app, &ctx, "window.panel.notes", json!({"show": true})).unwrap();
    assert!(app.ui.analysis.notes);
    let items = crate::menus::menu_items(&app);
    for id in [
        "window.panel.measurementLog",
        "window.panel.notes",
        "image.analysis.setMeasurementScale",
        "image.analysis.placeScaleMarker",
        "file.import.notes",
        "view.proofSetup.workingCyanPlate",
    ] {
        assert!(items.iter().any(|i| i.id == id && i.enabled), "{id} live");
    }
}

#[test]
fn arbitrary_rotation_starts_at_the_ruler_angle() {
    let (mut app, ctx) = app();
    let open = |app: &mut PhotocraftApp| {
        let r = crate::menus::invoke(app, &ctx, "image.rotation.arbitrary", json!({})).unwrap();
        let f = &app.ui.dialog_mut(r["dialog"].as_u64().unwrap()).unwrap().fields;
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
fn ruler_drag_protractor_and_clear() {
    let (mut app, _) = app();
    app.ui.tool = Tool::Ruler;
    let none = egui::Modifiers::NONE;
    down(&mut app, 10.0, 10.0, none);
    mv(&mut app, 60.0, 10.0, none);
    up(&mut app, 60.0, 10.0);
    assert_eq!(ruler(&app).map(|r| r.end), Some([60.0, 10.0]));
    // Drag the end point with ⇧: constrained to 45°.
    down(&mut app, 60.0, 10.0, none);
    mv(&mut app, 12.0, 52.0, egui::Modifiers::SHIFT);
    up(&mut app, 12.0, 52.0);
    let r = ruler(&app).unwrap();
    assert!((r.end[0] - 10.0).abs() < 1e-6, "vertical: {r:?}");
    // ⌥-drag from the start point: protractor arm.
    down(&mut app, 10.0, 10.0, egui::Modifiers::ALT);
    mv(&mut app, 60.0, 10.0, none);
    up(&mut app, 60.0, 10.0);
    let info = photocraft_engine::analysis_cmds::ruler_info(&ruler(&app).unwrap(), &Default::default());
    assert!((info["angle"].as_f64().unwrap() - 90.0).abs() < 1e-6, "{info}");
    // A click without a drag far away starts a new line and clears it.
    down(&mut app, 150.0, 80.0, none);
    up(&mut app, 150.0, 80.0);
    assert!(ruler(&app).is_none());
    assert_eq!(app.session.active().unwrap().history.past_len(), 0, "the ruler adds no history");
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
    // Drag marker 1.
    down(&mut app, 20.5, 20.5, egui::Modifiers::NONE);
    up(&mut app, 30.0, 25.0);
    assert_eq!(pts(&app)[0], [30.0, 25.0]);
    // ⌥-click removes.
    down(&mut app, 50.0, 50.0, egui::Modifiers::ALT);
    up(&mut app, 50.0, 50.0);
    assert_eq!(pts(&app).len(), 1);
}

#[test]
fn note_tool_and_panel_state() {
    let (mut app, ctx) = app();
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_author = "QA".into();
    down(&mut app, 30.0, 40.0, egui::Modifiers::NONE);
    up(&mut app, 30.0, 40.0);
    let n = app.session.active().unwrap().doc.notes.clone();
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].author, "QA");
    assert!(app.ui.analysis.notes && app.ui.analysis.note_selected == Some(0));
    // Drag the icon.
    down(&mut app, 32.0, 42.0, egui::Modifiers::NONE);
    up(&mut app, 62.0, 52.0);
    assert_eq!(app.session.active().unwrap().doc.notes[0].position, [60.0, 50.0]);
    // The panel renders without panicking.
    render(&mut app, &ctx, windows);
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
fn dialogs_open_and_run_their_commands() {
    let (mut app, ctx) = app();
    let r = crate::menus::invoke(&mut app, &ctx, "image.analysis.setMeasurementScale", json!({})).unwrap();
    assert_eq!(r["dialog"], "scale");
    let (kind, mut f) = app.ui.analysis.dialog.clone().unwrap();
    f.insert("preset".into(), json!("custom"));
    f.insert("pixelLength".into(), json!(40));
    f.insert("logicalLength".into(), json!(1));
    f.insert("units".into(), json!("cm"));
    let (id, p) = dialog_command(&kind, &f).unwrap();
    app.run(id, p).unwrap();
    assert_eq!(app.session.active().unwrap().doc.measurement.scale.units, "cm");
    crate::menus::invoke(&mut app, &ctx, "image.analysis.placeScaleMarker", json!({})).unwrap();
    let (kind, f) = app.ui.analysis.dialog.clone().unwrap();
    assert_eq!(f["length"], 1.0);
    let (id, p) = dialog_command(&kind, &f).unwrap();
    app.run(id, p).unwrap();
    crate::menus::invoke(&mut app, &ctx, "image.analysis.selectDataPoints", json!({})).unwrap();
    let (kind, mut f) = app.ui.analysis.dialog.clone().unwrap();
    f.insert("ruler".into(), json!(["label", "length"]));
    let (id, p) = dialog_command(&kind, &f).unwrap();
    app.run(id, p).unwrap();
    assert_eq!(app.session.analysis.data_points.ruler, ["label", "length"]);
    // Record from the Count tool records counts and opens the log.
    app.ui.tool = Tool::Count;
    crate::menus::invoke(&mut app, &ctx, "image.analysis.recordMeasurements", json!({})).unwrap();
    assert!(app.ui.analysis.measurement_log);
    assert_eq!(app.session.analysis.log.last().unwrap().values["source"], "Count Tool");
    render(&mut app, &ctx, windows);
}

#[test]
fn color_sampler_tool_places_moves_and_removes_points() {
    let (mut app, ctx) = app();
    app.ui.tool = Tool::ColorSampler;
    let pts = |app: &PhotocraftApp| app.session.active().unwrap().color_samplers.clone();
    down(&mut app, 20.0, 20.0, egui::Modifiers::NONE);
    up(&mut app, 20.0, 20.0);
    down(&mut app, 60.0, 40.0, egui::Modifiers::NONE);
    up(&mut app, 60.0, 40.0);
    assert_eq!(pts(&app), vec![[20.0, 20.0], [60.0, 40.0]]);
    // Drag the first point to a new pixel (committed on release).
    down(&mut app, 20.0, 20.0, egui::Modifiers::NONE);
    mv(&mut app, 30.0, 25.0, egui::Modifiers::NONE);
    up(&mut app, 30.0, 25.0);
    assert_eq!(pts(&app)[0], [30.0, 25.0]);
    // A click on a point, with no drag, leaves it where it is.
    down(&mut app, 30.0, 25.0, egui::Modifiers::NONE);
    up(&mut app, 30.0, 25.0);
    assert_eq!(pts(&app)[0], [30.0, 25.0]);
    // ⌥-click removes that one point.
    down(&mut app, 60.0, 40.0, egui::Modifiers::ALT);
    up(&mut app, 60.0, 40.0);
    assert_eq!(pts(&app).len(), 1);
    // Dragging a point off the canvas removes it.
    down(&mut app, 30.0, 25.0, egui::Modifiers::NONE);
    mv(&mut app, -50.0, 25.0, egui::Modifiers::NONE);
    up(&mut app, -50.0, 25.0);
    assert!(pts(&app).is_empty());
    assert_eq!(app.session.active().unwrap().history.past_len(), 0, "sampling adds no history");
    // The overlay and the options bar render without panicking.
    down(&mut app, 10.0, 10.0, egui::Modifiers::NONE);
    up(&mut app, 10.0, 10.0);
    render(&mut app, &ctx, windows);
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0)),
        zoom: 1.0,
        center: [100.0, 50.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut o = ctx.run_ui(Default::default(), |ui| draw_overlay(&app, ui.painter(), &xf));
    o.textures_delta.clear();
}

#[test]
fn color_sampler_readout_and_crop_removal_report() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    down(&mut app, 5.0, 5.0, egui::Modifiers::NONE);
    up(&mut app, 5.0, 5.0);
    down(&mut app, 150.0, 90.0, egui::Modifiers::NONE);
    up(&mut app, 150.0, 90.0);
    // The list reports both, numbered, with their position and RGB/CMYK readout.
    let l = app.run("view.colorSamplers.list", json!({})).unwrap();
    assert_eq!(l["samplers"].as_array().unwrap().len(), 2);
    assert_eq!(l["samplers"][0]["number"], 1);
    assert_eq!(l["samplers"][1]["position"], json!([150.0, 90.0]));
    assert!(l["samplers"][0]["rgb"].as_array().is_some_and(|a| a.len() == 3));
    // Cropping to the top-left drops the outside point and reports how many went.
    app.run("image.crop", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(app.session.active().unwrap().color_samplers, vec![[5.0, 5.0]]);
    assert!(app.ui.status.contains("removed"), "{}", app.ui.status);
    assert!(!app.ui.status_error);
}

#[test]
fn color_sampler_is_per_document_and_inert_without_one() {
    let (mut app, _) = app();
    app.ui.tool = Tool::ColorSampler;
    down(&mut app, 10.0, 10.0, egui::Modifiers::NONE);
    up(&mut app, 10.0, 10.0);
    // A second document starts empty; the first keeps its own point.
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    assert!(app.session.active().unwrap().color_samplers.is_empty());
    down(&mut app, 20.0, 20.0, egui::Modifiers::NONE);
    up(&mut app, 20.0, 20.0);
    assert_eq!(app.session.active().unwrap().color_samplers, vec![[20.0, 20.0]]);
    app.run("document.activate", json!({"document": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().color_samplers, vec![[10.0, 10.0]]);
    // With no document the tool says so (no dialog, no panic) and the commands error.
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::ColorSampler;
    down(&mut empty, 5.0, 5.0, egui::Modifiers::NONE);
    assert_eq!(empty.ui.status, "No document open");
    assert!(!empty.ui.status_error);
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

/// The shell harness: the app in a window, so the analysis panels draw as they do live.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).expect("new document");
        app
    });
    h.run_steps(4);
    h
}

/// The first node labelled `label` (the icon buttons and the fields can share a name).
fn node(h: &Harness<'static, PhotocraftApp>, label: &str) -> egui::Rect {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("`{label}` drawn")).rect()
}

/// The node labelled `label` inside `window`: the label alone is not unique — the document
/// tab's close button is labelled `Close` too, and it is drawn first.
fn node_in(h: &Harness<'static, PhotocraftApp>, window: egui::Rect, label: &str) -> egui::Rect {
    h.query_all_by_label(label).find(|n| window.contains(n.rect().center())).unwrap_or_else(|| panic!("`{label}` drawn inside the window")).rect()
}

/// The Measurement Log opens 16 px inside the canvas's reading-end edge, 300 px away from
/// it. The window is 620 px wide: the left-to-right branch measures that width from the
/// reading-end edge, the right-to-left branch anchors the window's near edge there, so the
/// reading-end edge ends up the same distance away in either direction.
const LOG_EDGE_GAP: f32 = 16.0 + 300.0;
const LOG_WIDTH: f32 = 620.0;

/// The Measurement Log opens at the canvas's reading-end edge, and its close button and
/// buttons read from the reading start.
#[test]
fn the_measurement_log_window_mirrors() {
    let _lang = i18n::language_scope(Lang::EN);
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let mut h = harness();
        h.state_mut().ui.analysis.measurement_log = true;
        h.run_steps(4);
        let window = h.ctx.memory(|m| m.area_rect(egui::Id::unique("measurement-log"))).expect("the log window");
        let canvas = h.state().last_canvas_rect;
        let close = node_in(&h, window, "Close");
        let record = node_in(&h, window, "Record Measurements");
        let trash = node_in(&h, window, "Delete selected measurements");
        if dir == Dir::Rtl {
            assert!(close.center().x < window.center().x, "the close button sits at the reading end (left)");
            assert!(record.center().x > trash.center().x, "the buttons read right to left");
        } else {
            assert!(close.center().x > window.center().x, "the close button sits at the reading end (right)");
            assert!(record.center().x < trash.center().x, "the buttons read left to right");
        }
        // The gap between the canvas's reading-end edge and the window, with the part of the
        // window that extends from that edge taken off.
        let gap = if dir == Dir::Rtl { window.left() - canvas.left() } else { canvas.right() - window.left() - LOG_WIDTH };
        assert!((gap - LOG_EDGE_GAP).abs() < 2.0, "the window opens {LOG_EDGE_GAP} px inside the canvas's reading-end edge: {gap}");
    }
}

/// The Notes panel's previous/next chevrons and its delete button follow the reading direction.
#[test]
fn the_notes_panel_mirrors() {
    let _lang = i18n::language_scope(Lang::EN);
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let mut h = harness();
        h.state_mut().run("notes.add", json!({"x": 20, "y": 20, "author": "Ann", "color": [1.0, 0.5, 0.75]})).expect("note");
        h.state_mut().run("notes.add", json!({"x": 120, "y": 80, "author": "Bob", "color": [0.25, 0.5, 1.0]})).expect("note");
        h.state_mut().ui.analysis.notes = true;
        h.run_steps(4);
        let prev = node(&h, "Previous note");
        let next = node(&h, "Next note");
        let delete = node(&h, "Delete note");
        if dir == Dir::Rtl {
            assert!(prev.center().x > next.center().x, "previous sits at the reading start (right)");
            assert!(delete.center().x < next.center().x, "the delete button sits at the reading end (left)");
        } else {
            assert!(prev.center().x < next.center().x, "previous sits at the reading start (left)");
            assert!(delete.center().x > next.center().x, "the delete button sits at the reading end (right)");
        }
    }
}

/// The analysis dialog's field label leads at the reading start, and its buttons follow the
/// reading direction.
#[test]
fn the_analysis_dialog_mirrors() {
    let _lang = i18n::language_scope(Lang::EN);
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let mut h = harness();
        let mut f = serde_json::Map::new();
        f.insert("preset".into(), json!("custom"));
        f.insert("pixelLength".into(), json!(40));
        f.insert("logicalLength".into(), json!(1));
        f.insert("units".into(), json!("cm"));
        h.state_mut().ui.analysis.dialog = Some(("scale".into(), f));
        h.run_steps(4);
        let ok = node(&h, "OK");
        let cancel = node(&h, "Cancel");
        let cells: Vec<egui::Rect> = h.query_all_by_label("Pixel Length:").map(|n| n.rect()).collect();
        assert!(cells.len() >= 2, "the field and its label: {cells:?}");
        let label = cells.iter().copied().min_by(|a, b| (a.width() - 110.0).abs().total_cmp(&(b.width() - 110.0).abs())).unwrap_or_else(|| panic!("the label"));
        if dir == Dir::Rtl {
            assert!(ok.center().x > cancel.center().x, "OK comes first, at the reading start");
            assert!(
                label.center().x > cells.iter().filter(|c| **c != label).map(|c| c.center().x).fold(f32::MIN, f32::max),
                "the label leads at the reading start"
            );
        } else {
            assert!(ok.center().x < cancel.center().x, "OK comes first, at the reading start");
            assert!(
                label.center().x < cells.iter().filter(|c| **c != label).map(|c| c.center().x).fold(f32::MAX, f32::min),
                "the label leads at the reading start"
            );
        }
    }
}
