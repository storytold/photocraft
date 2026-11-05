use super::*;

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

/// Exercise the production floating panel, not `export_log(Some(vec![]))`: that helper
/// already distinguished empty from omitted rows before #923 was fixed.
#[cfg(not(target_arch = "wasm32"))]
mod selected_export_923 {
    use super::*;
    use crate::{FileDialogAnswer, FileDialogReply, FileDialogRequest, Services};
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;
    use std::sync::Arc;

    const EXPORT: &str = "Export selected measurements (CSV)";
    const ONE: &[u8] = b"Label\nMeasurement 1\n";
    const BOTH: &[u8] = b"Label\nMeasurement 1\nMeasurement 2\n";
    type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;
    type Attempts = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

    #[derive(Clone, Copy)]
    enum Writer {
        Save,
        Absent,
        Refuse,
    }

    struct Fixture {
        h: Harness<'static, PhotocraftApp>,
        open: Open,
        attempts: Attempts,
        output: tempfile::TempDir,
        original_doc: Arc<Document>,
        original_state: String,
        original_log: Vec<photocraft_engine::analysis_cmds::LogRow>,
        original_points: photocraft_engine::analysis_cmds::DataPoints,
    }

    // Keep this snapshot after fixture construction: setting up a real document/measurement
    // is not part of exporting. Save-folder memory is intentionally outside the snapshot.
    fn document_state(app: &PhotocraftApp) -> String {
        let st = app.session.active().unwrap();
        format!(
            "{:?}",
            (&st.history, st.revision, st.saved_revision, st.view_revision, &st.path, st.active_layer, &st.selected_layers, &st.coalesce)
        )
    }

    impl Fixture {
        fn new(rows: usize, writer: Writer) -> Self {
            // The admitting native invocation pins TEMP/TMP/TMPDIR to its owned scratch.
            // Each test has a fresh directory; the injected service refuses every other path.
            let output = tempfile::Builder::new().prefix("photocraft-923-").tempdir_in(std::env::temp_dir()).unwrap();
            let allowed = output.path().join("selected.csv").to_str().unwrap().to_string();
            let open: Open = Default::default();
            let attempts: Attempts = Default::default();
            let (held, calls) = (open.clone(), attempts.clone());
            let mut services = Services {
                file_dialog: Some(Box::new(move |request, parent, reply| {
                    assert!(parent.is_none(), "panel-only harness, no native dialog");
                    held.borrow_mut().push((request, reply));
                })),
                ..Default::default()
            };
            if !matches!(writer, Writer::Absent) {
                services.write = Some(Box::new(move |path: &str, bytes: &[u8]| {
                    assert_eq!(path, allowed, "only this fixture's exact scratch file may be written");
                    calls.borrow_mut().push((path.to_string(), bytes.to_vec()));
                    if matches!(writer, Writer::Refuse) {
                        return Err("#923 injected writer refusal".into());
                    }
                    std::fs::write(path, bytes).map_err(|e| e.to_string())
                }));
            }
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
            // A label-only column makes the expected CSV independent of clock, filename,
            // raster statistics and the export implementation's own formatting helper.
            app.run("image.analysis.selectDataPoints", json!({"count": ["label"]})).unwrap();
            for _ in 0..rows {
                app.run("image.analysis.recordMeasurements", json!({"source": "count", "dateTime": "2026-01-01T00:00:00Z"})).unwrap();
            }
            assert_eq!(app.session.analysis.log.len(), rows);
            if rows == 2 {
                let log = &app.session.analysis.log;
                assert_ne!(log[0].id, log[1].id);
                assert_eq!(json!(log[0].values), json!({"label": "Measurement 1"}));
                assert_eq!(json!(log[1].values), json!({"label": "Measurement 2"}));
            }
            app.sync_views();
            app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1100.0));
            let ctx = egui::Context::default();
            crate::menus::invoke(&mut app, &ctx, "window.panel.measurementLog", json!({"show": true})).unwrap();
            assert!(app.ui.analysis.measurement_log);
            let original_doc = app.session.active().unwrap().doc.clone();
            let original_state = document_state(&app);
            let original_log = app.session.analysis.log.clone();
            let original_points = app.session.analysis.data_points.clone();
            let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1100.0)).build_ui_state(
                |ui, app| {
                    windows(app, ui.ctx());
                    app.poll_file_dialog(ui.ctx(), None);
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(4);
            Self { h, open, attempts, output, original_doc, original_state, original_log, original_points }
        }

        fn ids(&self) -> Vec<u64> {
            self.original_log.iter().map(|r| r.id).collect()
        }

        fn click(&mut self, role: Role, label: &str) {
            self.h.run_steps(2);
            let node = self.h.get_by_role_and_label(role, crate::i18n::t(label));
            let rect = node.rect();
            assert!(rect.is_finite() && rect.is_positive(), "{label}: fresh nonempty bounds");
            assert!(self.h.ctx.content_rect().contains_rect(rect), "{label}: fully inside viewport: {rect:?}");
            // Accessibility bounds alone are not proof of a pointer target. The pinned egui
            // API returns enabled hit rectangles after the parent UI's actual clipping.
            assert!(
                self.h.ctx.interactive_rects_last_pass().iter().any(|hit| {
                    hit.contains_rect(rect.shrink(0.1)) && (hit.width() - rect.width()).abs() < 1.0 && (hit.height() - rect.height()).abs() < 1.0
                }),
                "{label}: full target must survive production clipping: {rect:?}"
            );
            // Node::click queues PointerMoved + primary down/up, NOT an AccessKit Click.
            node.click();
            self.h.run_steps(3);
        }

        fn select_all(&mut self) {
            self.click(Role::Button, "Select All");
            assert_eq!(self.h.state().ui.analysis.log_selected, self.ids());
        }

        fn deselect_all(&mut self) {
            self.click(Role::Button, "Deselect All");
            assert!(self.h.state().ui.analysis.log_selected.is_empty());
        }

        fn select_one(&mut self) {
            self.click(Role::Button, "Measurement 1");
            assert_eq!(self.h.state().ui.analysis.log_selected, [self.ids()[0]]);
        }

        fn export(&mut self) {
            assert!(!self.h.state().file_dialog_open());
            assert!(self.open.borrow().is_empty());
            self.click(Role::Button, EXPORT);
            self.assert_pending();
        }

        fn assert_pending(&self) {
            assert!(self.h.state().file_dialog_open(), "real export queues its async picker");
            let open = self.open.borrow();
            assert_eq!(open.len(), 1, "one actual service request");
            assert!(
                matches!(&open[0].0, FileDialogRequest::Save { suggested } if Path::new(suggested).file_name().unwrap() == "Measurements.csv"),
                "{:?}", open[0].0
            );
        }

        fn path(&self) -> String {
            self.output.path().join("selected.csv").to_str().unwrap().to_string()
        }

        fn answer(&mut self, accepted: bool) {
            let (_, reply) = self.open.borrow_mut().pop().expect("actual retained reply");
            reply.send(accepted.then(|| FileDialogAnswer::SaveTo(self.path())));
            self.h.run_steps(3);
            assert!(!self.h.state().file_dialog_open(), "poll retires the continuation");
            assert!(self.open.borrow().is_empty());
        }

        fn assert_saved(&self, expected: &[u8]) {
            let calls = self.attempts.borrow();
            assert_eq!(calls.len(), 1, "must reach the real writer exactly once, not cancel or silently refuse");
            assert_eq!(calls[0].0, self.path());
            let saved = std::fs::read(self.path()).expect("the service actually saved its exact scratch file");
            assert_eq!(saved, calls[0].1, "read back actual writer bytes before the regression oracle");
            assert_eq!(saved, expected, "#923 production panel exported unselected measurements");
            assert!(!self.h.state().ui.status_error, "{}", self.h.state().ui.status);
            assert_eq!(std::fs::read_dir(self.output.path()).unwrap().count(), 1, "no other output");
        }

        fn assert_no_write(&self) {
            assert!(self.attempts.borrow().is_empty());
            assert!(!Path::new(&self.path()).exists());
            assert_eq!(std::fs::read_dir(self.output.path()).unwrap().count(), 0);
        }

        fn assert_preserved(&self, selection: &[u64]) {
            let app = self.h.state();
            assert!(Arc::ptr_eq(&self.original_doc, &app.session.active().unwrap().doc), "document snapshot unchanged");
            assert_eq!(document_state(app), self.original_state, "history, revision, path and layer selection unchanged");
            assert_eq!(app.session.analysis.log, self.original_log);
            assert_eq!(app.session.analysis.data_points, self.original_points);
            assert_eq!(app.ui.analysis.log_selected, selection);
            assert!(app.ui.analysis.measurement_log, "export does not close the panel");
        }
    }

    #[test]
    fn a1_deselect_all_exports_zero_rows_through_panel_picker_writer() {
        let mut f = Fixture::new(2, Writer::Save);
        f.select_all();
        f.deselect_all();
        f.export();
        f.assert_no_write();
        f.h.run_steps(3);
        f.assert_pending();
        f.assert_no_write();
        f.answer(true);
        f.assert_preserved(&[]);
        // Intentionally after successful request/click/answer/write/readback: baseline and
        // route-sabotage must fail the CSV assertion, not a query or missing service.
        f.assert_saved(b"\n");
    }

    #[test]
    fn a2_one_real_row_click_exports_only_that_row() {
        let mut f = Fixture::new(2, Writer::Save);
        f.select_one();
        f.export();
        f.assert_no_write();
        f.answer(true);
        f.assert_preserved(&[f.ids()[0]]);
        f.assert_saved(ONE);
    }

    #[test]
    fn a3_select_all_exports_both_rows_in_log_order() {
        let mut f = Fixture::new(2, Writer::Save);
        f.select_all();
        f.export();
        f.assert_no_write();
        f.answer(true);
        f.assert_preserved(&f.ids());
        f.assert_saved(BOTH);
    }

    #[test]
    fn a4_cancel_and_dropped_reply_do_not_write_or_mutate() {
        let mut f = Fixture::new(2, Writer::Save);
        f.select_one();
        f.export();
        f.assert_no_write();
        f.answer(false);
        f.assert_no_write();
        f.assert_preserved(&[f.ids()[0]]);
        assert!(!f.h.state().ui.status_error);
        // Reopening also proves Cancel released the pending-dialog slot.
        f.export();
        drop(f.open.borrow_mut().pop().expect("unanswered real reply"));
        f.h.run_steps(3);
        assert!(!f.h.state().file_dialog_open());
        f.assert_no_write();
        f.assert_preserved(&[f.ids()[0]]);
        assert!(!f.h.state().ui.status_error);
    }

    #[test]
    fn a5_whole_log_menu_ignores_empty_and_single_row_selection() {
        // Fresh files avoid overwrite behavior; both controls use the real menu route.
        for one in [false, true] {
            let mut f = Fixture::new(2, Writer::Save);
            if one {
                f.select_one();
            } else {
                f.select_all();
                f.deselect_all();
            }
            let selection = f.h.state().ui.analysis.log_selected.clone();
            let ctx = f.h.ctx.clone();
            crate::menus::invoke(f.h.state_mut(), &ctx, "measurementLog.export", json!({})).unwrap();
            f.h.run_steps(3);
            f.assert_pending();
            f.assert_no_write();
            f.answer(true);
            f.assert_preserved(&selection);
            f.assert_saved(BOTH);
        }
    }

    #[test]
    fn a6_session_omitted_rows_exports_all_but_explicit_empty_exports_none() {
        let mut f = Fixture::new(2, Writer::Save);
        let all = f.h.state_mut().session.execute("measurementLog.export", json!({})).unwrap();
        assert_eq!(all, json!({"rows": 2, "csv": std::str::from_utf8(BOTH).unwrap()}));
        let empty = f.h.state_mut().session.execute("measurementLog.export", json!({"rows": []})).unwrap();
        assert_eq!(empty, json!({"rows": 0, "csv": "\n"}));
        assert!(!f.h.state().file_dialog_open() && f.open.borrow().is_empty());
        f.assert_no_write();
        f.assert_preserved(&[]);
    }

    #[test]
    fn a7_empty_log_and_writer_failures_preserve_document_and_log() {
        let mut empty = Fixture::new(0, Writer::Save);
        empty.export();
        empty.assert_no_write();
        empty.answer(true);
        empty.assert_preserved(&[]);
        empty.assert_saved(b"\n");
        for writer in [Writer::Absent, Writer::Refuse] {
            let mut f = Fixture::new(2, writer);
            f.select_one();
            f.export();
            f.assert_no_write();
            f.answer(true);
            f.assert_preserved(&[f.ids()[0]]);
            assert!(f.h.state().ui.status_error, "continuation failures must be actionable");
            assert!(!Path::new(&f.path()).exists(), "failure must not create a phantom output");
            assert_eq!(std::fs::read_dir(f.output.path()).unwrap().count(), 0);
            match writer {
                Writer::Absent => {
                    assert_eq!(f.h.state().ui.status, "no writer configured");
                    assert!(f.attempts.borrow().is_empty());
                }
                Writer::Refuse => {
                    assert_eq!(f.h.state().ui.status, "#923 injected writer refusal");
                    assert_eq!(*f.attempts.borrow(), [(f.path(), ONE.to_vec())]);
                }
                Writer::Save => panic!("only failure controls here"),
            }
        }
    }
}
