use super::*;

mod seq_check;
use craft::behavior::geometry::quad::BoxState;
use craft::behavior::kurbo::{Point, Rect, Vec2};
use craft::behavior::{EditEvent, EditEvents, GestureId, Mixed};
use craft::draw::ui::transform::{TransformChoice, TransformEdit, TransformMenuWords};
use craft::i18n::WordSet;
use craft::imaging::trace::{MaskSource, outline};
use craft::imaging::ui::warp::{CylinderValues, WarpMesh, WarpSplit};

use super::preview::AbovePlan;
use super::warp::{cylinder_edit, cylinder_values, grid_edit};
use crate::state::{FreeTransform, SessionWarp};

// ---- Helpers ----

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// An app inside a window, run frame by frame as the desktop app is.
fn harness(app: Box<PhotocraftApp>, size: egui::Vec2) -> Harness {
    // The harness runs on the app's own context, so what the app set up on it (the runtime, the command catalog) is there.
    egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(size).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    })
}

/// The corners of `[x0, y0, x1, y1]`, clockwise from the top left.
fn corners(r: [f64; 4]) -> [[f64; 2]; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

/// The open Free Transform.
fn open(app: &PhotocraftApp) -> &FreeTransform {
    app.ui.transform.as_ref().expect("a Free Transform is open")
}

/// The box's corners as arrays.
fn quad_of(t: &FreeTransform) -> [[f64; 2]; 4] {
    t.session.quad().map(|p| [p.x, p.y])
}

/// The box's frame as `[x0, y0, x1, y1]`.
fn rect_of(t: &FreeTransform) -> [f64; 4] {
    let r = t.session.rect();
    [r.x0, r.y0, r.x1, r.y1]
}

/// Puts the box on `quad`, keeping its reference point, as one `Set` edit of the session.
fn set_quad(app: &mut PhotocraftApp, quad: [[f64; 2]; 4]) {
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = BoxState::new(quad.map(|[x, y]| Point::new(x, y)), b.pivot));
}

/// Moves the box and its reference point by (`dx`, `dy`).
fn move_box(app: &mut PhotocraftApp, dx: f64, dy: f64) {
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = b.moved(Vec2::new(dx, dy)));
}

/// Scales the box along its own axes about its reference point.
fn scale_box(app: &mut PhotocraftApp, kx: f64, ky: f64) {
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = b.scaled_about_pivot(kx, ky));
}

/// Turns the box by `radians` about its reference point.
fn turn_box(app: &mut PhotocraftApp, radians: f64) {
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = b.turned_about_pivot(radians));
}

/// Puts `warp` on the session, as Warp mode holds it (on type when the layer is type).
fn set_warp(app: &mut PhotocraftApp, warp: Warp) {
    let (on_type, now) = (warps_type(app), app.nudge_bundle.now);
    let _ = app.ui.transform.as_mut().unwrap().session.apply(TransformEdit::Warp(SessionWarp::new(warp, on_type)), now);
}

/// The warp of the open Warp mode.
fn session_warp(app: &PhotocraftApp) -> &SessionWarp {
    open(app).session.warp().expect("Warp mode")
}

/// The Warp mode's mesh.
fn warp_mesh(app: &PhotocraftApp) -> WarpMesh {
    session_warp(app).mesh.clone()
}

/// Ends a run of edits as its own undo step of the session.
fn settle(app: &mut PhotocraftApp) {
    let now = app.nudge_bundle.now;
    app.ui.transform.as_mut().unwrap().session.settle(now, false);
}

/// One box gesture, `Begin`, an `Update` per state and `Commit` of the last, applied the way the box
/// reports a drag (`id` names the gesture).
fn box_events(app: &mut PhotocraftApp, id: u64, states: &[BoxState]) {
    let now = 0.0;
    let session = &mut app.ui.transform.as_mut().unwrap().session;
    let (gesture, before) = (GestureId::new(id), *session.state());
    let mut events = vec![EditEvent::begin(gesture, Mixed::Same(before), "Transform".into(), None)];
    events.extend(states.iter().map(|s| EditEvent::update(gesture, *s)));
    events.push(EditEvent::commit(gesture, Mixed::Same(before), *states.last().expect("a state to end on")));
    for event in events {
        let _ = session.apply(TransformEdit::Box(event), now);
    }
}

/// One warp-grid gesture that ends on `to`, handed to the warp driver.
fn pull_mesh(app: &mut PhotocraftApp, id: u64, to: WarpMesh) {
    let (gesture, before) = (GestureId::new(id), warp_mesh(app));
    let mut events = EditEvents::new();
    events.push(EditEvent::begin(gesture, Mixed::Same(before.clone()), "Warp".into(), None));
    events.push(EditEvent::update(gesture, to.clone()));
    events.push(EditEvent::commit(gesture, Mixed::Same(before), to));
    grid_edit(app, &events, false);
}

/// The cylinder's values as the warp holds them.
fn cylinder_of(app: &PhotocraftApp) -> [f64; 7] {
    cylinder_values(&session_warp(app).warp).expect("a cylinder warp").to_array()
}

/// One cylinder gesture that ends on `to`, handed to the cylinder driver.
fn drag_cylinder(app: &mut PhotocraftApp, id: u64, to: [f64; 7]) {
    let (gesture, before, after) = (GestureId::new(id), CylinderValues::from_array(cylinder_of(app)), CylinderValues::from_array(to));
    let mut events = EditEvents::new();
    events.push(EditEvent::begin(gesture, Mixed::Same(before), "Cylinder".into(), None));
    events.push(EditEvent::update(gesture, after));
    events.push(EditEvent::commit(gesture, Mixed::Same(before), after));
    cylinder_edit(app, &events);
}

// ---- Dropped files and placing ----

#[test]
fn editing_the_contents_of_a_dropped_file_keeps_the_layer() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));

    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.sync_views();

    app.session.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    app.sync_views();

    let parent_id = app.session.active().unwrap().doc.id;
    let placed_layer = app.session.active().unwrap().active_layer.unwrap();

    let app_ctx = app.ctx().clone();
    begin_placed(&mut app, &app_ctx).unwrap();

    app.session.execute("layer.smartObjects.editContents", json!({})).unwrap();

    assert_ne!(app.session.active().unwrap().doc.id, parent_id, "Edit Contents should activate the child document");

    end_if_left(&mut app);

    let parent = app.session.documents().iter().find(|st| st.doc.id == parent_id);

    assert!(parent.is_some_and(|st| st.doc.layer(placed_layer).is_some()), "the placed Smart Object must remain in the parent document");
    assert!(app.ui.transform.is_none(), "the transform session should end");
    assert!(app.transform_preview.is_none(), "the transform preview should end");
}

/// #8492: Preferences ▸ General ▸ Skip Transform when Placing (off by default) places a file
/// without opening Free Transform on it, as in the reference app.
#[test]
fn skip_transform_when_placing_leaves_the_place_without_a_box() {
    for skip in [false, true] {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
        app.session.execute("prefs.set", json!({"path": "general.skipTransformWhenPlacing", "value": skip})).unwrap();
        app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.session.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
        app.sync_views();
        let placed = app.session.active().unwrap().active_layer.unwrap();
        let app_ctx = app.ctx().clone();
        begin_placed(&mut app, &app_ctx).unwrap();
        assert_eq!(app.ui.transform.is_some(), !skip, "skip {skip}: Free Transform opens only with the preference off");
        assert!(app.session.active().unwrap().doc.layer(placed).is_some(), "skip {skip}: the placed layer stays");
    }
}

/// Polls the app's background jobs until they have all landed.
fn land_jobs(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    for _ in 0..2000 {
        crate::jobs_ui::tick(app, &ctx);
        if !app.session.has_jobs() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the background jobs never landed");
}

/// A smart object placed the way a dropped file is (the last step made it), in Free Transform
/// and moved 8 px right, with commits running in the background as in the desktop app:
/// (app, the placed layer, its frame before the move, history steps before the place).
fn moved_place() -> (Box<PhotocraftApp>, LayerId, photocraft_geom::Rect, usize) {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    app.session.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    app.sync_views();
    let st = app.session.active().unwrap();
    let placed = st.active_layer.unwrap();
    let frame = photocraft_engine::transform_cmds::transform_frame(&st.doc, st.doc.layer(placed).unwrap());
    let app_ctx = app.ctx().clone();
    begin_placed(&mut app, &app_ctx).unwrap();
    move_box(&mut app, 8.0, 0.0);
    app.background_jobs = true;
    (app, placed, frame, steps)
}

fn frame_of(app: &PhotocraftApp, layer: LayerId) -> Option<photocraft_geom::Rect> {
    let st = app.session.active()?;
    Some(photocraft_engine::transform_cmds::transform_frame(&st.doc, st.doc.layer(layer)?))
}

/// #5665: picking another tool or selecting another layer while a dropped file is in Free
/// Transform applies the transform when the commit runs in the background too. Ending the
/// box used to cancel that commit, which took the dropped file back (or left a transformed
/// layer where it was).
#[test]
fn another_tool_or_layer_applies_a_transform_committed_in_the_background() {
    for by_layer in [false, true] {
        let (mut app, placed, frame, steps) = moved_place();
        if by_layer {
            let below = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).find(|id| *id != placed).unwrap();
            app.run("layer.select", json!({"layer": below.0})).unwrap();
        } else {
            app.ui.tool = if app.ui.tool == Tool::Brush { Tool::Move } else { Tool::Brush };
        }
        end_if_left(&mut app);
        assert!(committing(&app).is_some(), "the commit runs in the background");
        land_jobs(&mut app);
        assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
        assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "the dropped file stays, moved (by layer: {by_layer})");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1, "the place and its move are one step");
        assert_eq!(st.history.undo_label(), Some(photocraft_engine::file_cmds::PLACE_EMBEDDED));
    }
}

/// #5667: New Layer while a dropped file is in Free Transform applies the transform first. When
/// the commit runs in the background a click waits for it to land, then runs (#9974): the place
/// (which takes the move into its step) and the new layer each stay their own step, in that
/// order. A script is refused instead, as it needs an answer now.
#[test]
fn new_layer_applies_the_open_transform() {
    let (mut script, ..) = moved_place();
    script.automation_input = true;
    let err = script.run("layer.new.layer", json!({})).unwrap_err();
    assert!(err.contains("Free Transform is still being applied"), "a script is refused while the commit runs: {err}");

    let (mut app, placed, frame, steps) = moved_place();
    let layers = app.session.active().unwrap().doc.layers.len();
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "no layer yet");
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
    assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "applied as it showed, not the original");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers + 1);
    assert_eq!(st.history.past_len(), steps + 2, "the place with its move, then New Layer");
    assert_eq!(st.history.undo_label(), Some("New Layer"));
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "one undo takes back the new layer");
    assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "the transform stays");
}

/// #9974: only a click waits for the commit. A control request (authorized, not automation
/// input) needs its answer now, and a second click while one waits is refused, not queued.
#[test]
fn only_one_click_waits_for_the_commit() {
    let (mut control, ..) = moved_place();
    control.session.authorize = Some(|_, _| Ok(()));
    let layers = control.session.active().unwrap().doc.layers.len();
    let err = control.run("layer.new.layer", json!({})).unwrap_err();
    assert!(err.contains("Free Transform is still being applied"), "a control request is refused: {err}");
    land_jobs(&mut control);
    assert_eq!(control.session.active().unwrap().doc.layers.len(), layers, "and nothing runs later");

    let (mut app, ..) = moved_place();
    let layers = app.session.active().unwrap().doc.layers.len();
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(app.run("layer.new.layer", json!({})).is_err(), "a second click is refused");
    land_jobs(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the waiting click ran once");
}

/// #5665, the report's second case through the UI with background jobs on, as in the desktop
/// app: Free Transform on a layer, move it, then click another layer's row in the Layers panel.
#[test]
fn clicking_another_layer_row_applies_the_transform() {
    let app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = harness(app, egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    let before = frame_of(h.state(), layer).unwrap();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    assert!(h.state().ui.transform.is_some(), "in Free Transform");
    move_box(h.state_mut(), 8.0, 0.0);
    h.state_mut().background_jobs = true;
    h.run_steps(2);
    let row = crate::layers_panel_nodes::row(&h, "Background").center();
    h.hover_at(row);
    h.run_steps(1);
    h.drag_at(row);
    h.run_steps(1);
    h.drop_at(row);
    for _ in 0..400 {
        h.run_steps(1);
        if h.state().ui.transform.is_none() && !h.state().session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_ne!(h.state().session.active().unwrap().active_layer, Some(layer), "the click selected the Background");
    assert!(h.state().ui.transform.is_none(), "the box ended");
    assert_eq!(frame_of(h.state(), layer), Some(before.translate(8, 0)), "the move was applied");
}

// ---- Commit and cancel ----

#[test]
fn begin_and_commit_through_the_engine() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    assert_eq!(rect_of(open(&app)), [8.0, 8.0, 24.0, 24.0]);
    scale_box(&mut app, 2.0, 2.0);
    commit(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.width().abs_diff(32) <= 2 && b.x0.abs_diff(0) <= 1, "{b:?}");
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

/// A commit the engine refuses (here a collapsed box; a result over the pixel budget, #3773, is
/// refused the same way) shows the reason and keeps the box open, unchanged; picking another
/// tool then ends it.
#[test]
fn a_refused_commit_keeps_the_box_open() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    let (revision, steps) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
    begin(&mut app, &ctx).unwrap();
    set_quad(&mut app, [[16.0, 16.0]; 4]);
    commit(&mut app);
    assert_eq!(quad_of(open(&app)), [[16.0, 16.0]; 4], "the box stays open, as the user left it");
    assert!(app.transform_preview.is_some(), "with its preview");
    assert!(app.ui.status.contains("collapses"), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.history.past_len()), (revision, steps), "the document was not changed");
    app.ui.tool = if app.ui.tool == Tool::Brush { Tool::Move } else { Tool::Brush };
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "another tool ends it");
}

/// Image Processor must not silently export the pixels from before the visible transform.
/// Both direct commands and dialog OK preserve the box and write nothing until it is settled.
#[test]
fn image_processor_document_sources_wait_for_the_transform() {
    for source in ["current", "open"] {
        for through_dialog in [false, true] {
            for apply in [false, true] {
                let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
                let ctx = app.ctx().clone();
                let output = tempfile::tempdir().unwrap();
                begin(&mut app, &ctx).unwrap();
                move_box(&mut app, 6.0, 0.0);
                let quad = quad_of(open(&app));
                let before = app.session.active().unwrap().doc.clone();
                let revision = app.session.active().unwrap().revision;
                let history = app.session.active().unwrap().history.past_len();
                let params = json!({"source": source, "output": output.path(), "format": "png"});
                let result: Result<(), String> = if through_dialog {
                    // The menu's own dialog holds the Source row; the bare command dialog has only the engine's fields.
                    let reply = crate::menus::invoke(&mut app, &ctx, "file.scripts.imageProcessor", json!({})).unwrap();
                    let id =
                        craft::automation::dialogs::dialog_id(&app.framework.dialogs, &json!({"dialog": reply["dialog"]})).expect("the menu opened the dialog");
                    app.framework.dialogs.set_json_field(&ctx, id, "source", json!(source)).unwrap();
                    app.framework.dialogs.set_json_field(&ctx, id, "output", json!(output.path())).unwrap();
                    // The dialog picks its file types by ticking them; OK is refused while none is ticked.
                    app.framework.dialogs.set_json_field(&ctx, id, "jpeg/enabled", json!(true)).unwrap();
                    crate::dialogs::confirm(&mut app, &ctx, id)
                } else {
                    app.run("file.scripts.imageProcessor", params.clone()).map(|_| ())
                };
                assert_eq!(result.unwrap_err(), "Commit or cancel the transform first");
                assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
                assert_eq!(quad_of(open(&app)), quad);
                let st = app.session.active().unwrap();
                assert!(Arc::ptr_eq(&st.doc, &before));
                assert_eq!((st.revision, st.history.past_len()), (revision, history));
                if apply {
                    commit(&mut app);
                } else {
                    cancel(&mut app);
                }
                assert!(app.ui.transform.is_none());
                let result = app.run("file.scripts.imageProcessor", params).unwrap();
                assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
                assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
            }
        }
    }
}

/// Folder input, including the legacy omitted source, does not read the open transform.
#[test]
fn image_processor_folder_source_keeps_working_during_a_transform() {
    for source in [None, Some("folder")] {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
        let ctx = app.ctx().clone();
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        app.run("file.saveACopy", json!({"path": input.path().join("source.png")})).unwrap();
        begin(&mut app, &ctx).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let mut params = json!({"input": input.path(), "output": output.path(), "format": "png"});
        if let Some(source) = source {
            params["source"] = json!(source);
        }
        let result = app.run("file.scripts.imageProcessor", params).unwrap();
        assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
        assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
        assert!(app.ui.transform.is_some());
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before));
    }
}

/// All open documents includes a transform in another tab; Current only reads its own tab.
#[test]
fn image_processor_checks_the_transform_document_for_each_source() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"name": "Other", "width": 16, "height": 16})).unwrap();
    assert!(app.session.set_active(0));
    begin(&mut app, &ctx).unwrap();
    assert!(app.session.set_active(1));
    let output = tempfile::tempdir().unwrap();
    let result = app.run("file.scripts.imageProcessor", json!({"source": "open", "output": output.path(), "format": "png"}));
    assert_eq!(result.unwrap_err(), "Commit or cancel the transform first");
    assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
    let result = app.run("file.scripts.imageProcessor", json!({"source": "current", "output": output.path(), "format": "png"})).unwrap();
    assert_eq!(result["files"].as_array().unwrap().len(), 1, "{result}");
    assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
}

/// #6737: Save for Web exports the committed document, so it is off while a box is pending.
#[test]
fn save_for_web_is_off_under_a_pending_transform() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    assert!(crate::menus::is_enabled(&app, SAVE_FOR_WEB), "enabled with no box");
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    assert!(!crate::menus::is_enabled(&app, SAVE_FOR_WEB), "off under the box");
    assert!(crate::menus::invoke(&mut app, &ctx, SAVE_FOR_WEB, json!({})).is_err(), "the dialog doesn't open");
    let path = std::env::temp_dir().join(format!("pc-web6737-{}.png", std::process::id())).to_string_lossy().into_owned();
    let r = crate::menus::invoke(&mut app, &ctx, SAVE_FOR_WEB, json!({"path": path, "format": "png24"}));
    assert!(r.is_err() && !std::path::Path::new(&path).exists(), "nor does the export run");
    assert!(app.ui.transform.is_some(), "the box stays open");
    cancel(&mut app);
    assert!(crate::menus::is_enabled(&app, SAVE_FOR_WEB), "back after the box closes");
}

/// Layer › Copy CSS reads the committed document, so, as Save for Web (#6737), it is off while a
/// box is pending: the clipboard never gets the bounds from before the move. After Cancel it
/// copies the old place, after a commit the new one.
#[test]
fn copy_css_is_off_under_a_pending_transform() {
    let id = photocraft_engine::layer_css_cmds::ID;
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    let copy_css = |app: &mut PhotocraftApp| {
        let mut result = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| result = Some(crate::menus::invoke(app, ui.ctx(), id, json!({}))));
        out.textures_delta.clear();
        (result.unwrap(), out.platform_output.commands)
    };
    let moved = |app: &mut PhotocraftApp| {
        let app_ctx = app.ctx().clone();
        crate::menus::invoke(app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
        move_box(app, 20.0, 0.0);
    };
    assert!(crate::menus::is_enabled(&app, id), "enabled with no box");
    moved(&mut app);
    assert!(!crate::menus::is_enabled(&app, id), "off under the box");
    let (result, clipboard) = copy_css(&mut app);
    assert!(result.is_err(), "refused under the box: {result:?}");
    assert!(clipboard.is_empty(), "nothing reaches the clipboard: {clipboard:?}");
    assert!(app.ui.transform.is_some(), "the box stays open");
    cancel(&mut app);
    assert!(crate::menus::is_enabled(&app, id), "back after Cancel");
    let (result, clipboard) = copy_css(&mut app);
    let css = result.unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("left: 8px;"), "{css}");
    assert_eq!(clipboard, [egui::OutputCommand::CopyText(css)]);
    moved(&mut app);
    commit(&mut app);
    assert!(app.ui.transform.is_none() && crate::menus::is_enabled(&app, id), "back after the commit");
    let (result, clipboard) = copy_css(&mut app);
    let css = result.unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("left: 28px;"), "the moved place: {css}");
    assert_eq!(clipboard, [egui::OutputCommand::CopyText(css)]);
}

// ---- Transform a copy ----

/// #352: ⌥⌘T transforms a copy. OK leaves the original and a moved copy as one history step;
/// Cancel leaves no copy and nothing to redo; with a selection the copy holds the selected
/// pixels only.
#[test]
fn free_transform_a_copy() {
    let bounds = |app: &PhotocraftApp, i: usize| app.session.active().unwrap().doc.layers[i].surface().unwrap().content_bounds();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    let (layers, steps) = (app.session.active().unwrap().doc.layers.len(), app.session.active().unwrap().history.past_len());
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(open(&app).made, Some(MadeLayer::Copy));
    assert!(!crate::menus::is_enabled(&app, "edit.freeTransformCopy"), "one transform at a time");
    // Esc: the copy goes too.
    cancel(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers, steps));
    assert!(!st.history.can_redo(), "nothing to redo");
    // OK: original in place, a moved copy, one step.
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    move_box(&mut app, 20.0, 0.0);
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers + 1, steps + 1));
    assert_eq!(bounds(&app, layers - 1), photocraft_geom::Rect::new(8, 8, 24, 24), "the original stays");
    assert_eq!(bounds(&app, layers), photocraft_geom::Rect::new(28, 8, 44, 24), "the copy moved");
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "one undo removes copy and move");
    // With a selection: Layer via Copy, so only the selected pixels are copied.
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 16})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(rect_of(open(&app)), [8.0, 8.0, 16.0, 24.0]);
    cancel(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
}

/// #4112: ⌥⌘T on a cylinder smart object, Warp mode, Enter with nothing touched: the copy
/// keeps the cylinder and stays its own step (no warp step to fold into), so one undo takes
/// back the copy and nothing before it.
#[test]
fn untouched_warp_of_a_cylinder_copy_keeps_its_own_step() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", json!({"style": "cylinder", "values": [8, 24, 24, 8, 0.3, 0.3, 0.5], "rect": [8, 8, 24, 24]})).unwrap();
    let warp = |app: &PhotocraftApp, i: usize| match &app.session.active().unwrap().doc.layers[i].content {
        photocraft_doc::LayerContent::Smart(sm) => sm.warp.as_ref().map(|w| w.style),
        _ => None,
    };
    let (layers, steps) = (app.session.active().unwrap().doc.layers.len(), app.session.active().unwrap().history.past_len());
    assert_eq!(warp(&app, layers - 1), Some(WarpStyle::Cylinder));
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    enter_warp(&mut app, &ctx);
    assert!(open(&app).session.warp().is_some(), "Warp mode shows the cylinder");
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers + 1, steps + 1));
    assert_eq!(warp(&app, layers), Some(WarpStyle::Cylinder), "the copy is still a cylinder");
    app.session.undo();
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers, steps), "one undo takes back the copy only");
    assert_eq!(warp(&app, layers - 1), Some(WarpStyle::Cylinder));
}

/// #1099: cancel, frame cleanup and a commit before the next frame all belong to the
/// original document, including after tab reordering and with shared layer IDs.
#[test]
fn cancelling_a_copy_after_switching_documents_preserves_the_active_document() {
    for reorder in [false, true] {
        for finish in [cancel, end_if_left, commit] {
            let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
            let original = app.session.active().unwrap().doc.clone();
            let steps = app.session.active().unwrap().history.past_len();
            let app_ctx = app.ctx().clone();
            begin_copy(&mut app, &app_ctx).unwrap();
            set_quad(&mut app, corners([12.0, 8.0, 28.0, 24.0]));
            // Cloning a document deliberately shares layer IDs, but gets a fresh document ID.
            let other = app.session.active().unwrap().doc.as_ref().clone();
            app.session.add_document(other, None);
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            assert!(app.session.undo());
            let st = app.session.active().unwrap();
            let (doc, history, redo, revision, layer) =
                (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), st.revision, st.active_layer);
            if reorder {
                app.session.move_document(0, 1);
            }
            finish(&mut app);
            assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
            let st = app.session.active().unwrap();
            assert!(Arc::ptr_eq(&st.doc, &doc), "the selected document's pixels are unchanged");
            assert_eq!((st.history.entries(), st.revision, st.active_layer), (history, revision, layer));
            assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
            let origin = app.session.documents().iter().find(|st| st.doc.id == original.id).unwrap();
            assert!(Arc::ptr_eq(&origin.doc, &original), "only the originating copy is undone");
            assert_eq!(origin.history.past_len(), steps);
            assert!(!origin.history.can_redo());
        }
    }
}

#[test]
fn cancelling_a_copy_after_its_document_closes_does_not_undo_another_document() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let app_ctx = app.ctx().clone();
    begin_copy(&mut app, &app_ctx).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    app.session.close(0);
    end_if_left(&mut app);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.past_len(), steps);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

#[test]
fn cancelling_an_already_undone_copy_preserves_history_and_redo() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let app_ctx = app.ctx().clone();
    begin_copy(&mut app, &app_ctx).unwrap();
    assert!(app.session.undo());
    let st = app.session.active().unwrap();
    let (doc, history, redo) = (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>());
    end_if_left(&mut app);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.entries(), history);
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

#[test]
fn cancelling_a_copy_does_not_undo_a_later_edit() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let app_ctx = app.ctx().clone();
    begin_copy(&mut app, &app_ctx).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let (doc, history) = (st.doc.clone(), st.history.entries());
    cancel(&mut app);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.entries(), history);
}

// ---- The preview follows the document ----

/// #1384: the Layers panel's opacity, fill, visibility and blend edits to the moving layer
/// and to the others show while the box is open, and survive the commit.
#[test]
fn edits_during_a_transform_show_in_the_preview() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool = Tool::Move;
    let bg = app.session.active().unwrap().doc.layers[0].id;
    let mut h = harness(app, egui::vec2(1200.0, 800.0));
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    move_box(h.state_mut(), 20.0, 0.0);
    h.run_steps(2);
    let layer = open(h.state()).layer;
    assert_eq!(h.state().transform_preview.as_ref().unwrap().opacity, 1.0);

    h.state_mut().run("layer.setProps", json!({"layer": layer, "opacity": 0.5, "fill": 0.6, "blend": "Multiply"})).unwrap();
    h.state_mut().run("layer.setProps", json!({"layer": bg.0, "visible": false})).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert!(app.ui.transform.is_some(), "the edits leave the box open");
    let pv = app.transform_preview.as_ref().unwrap();
    assert!((pv.opacity - 0.3).abs() < 1e-6, "the moving pixels take the new opacity × fill: {}", pv.opacity);
    assert!(!pv.doc.layer(bg).unwrap().visible, "the document under the box follows");
    let moving = pv.doc.layer(LayerId(layer)).unwrap();
    assert!(!moving.visible, "the moving layer stays out of the document under the box");
    assert_eq!(moving.blend, photocraft_color::BlendMode::Multiply);

    h.state_mut().run("layer.setProps", json!({"layer": layer, "visible": false})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().transform_preview.as_ref().unwrap().opacity, 0.0, "hiding the layer hides the moving pixels");
    h.state_mut().run("layer.setProps", json!({"layer": layer, "visible": true})).unwrap();

    commit(h.state_mut());
    let st = h.state().session.active().unwrap();
    let l = st.doc.layer(LayerId(layer)).unwrap();
    assert_eq!((l.opacity, l.fill_opacity, l.blend), (0.5, 0.6, photocraft_color::BlendMode::Multiply));
    assert_eq!(l.surface().unwrap().content_bounds(), photocraft_geom::Rect::new(28, 8, 44, 24));
}

/// #1384 with a selection: the pixels it leaves behind stay in the rebuilt preview without
/// the lifted ones, and take the layer's new properties.
#[test]
fn edits_during_a_selection_transform_keep_the_lifted_pixels_out() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool = Tool::Move;
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 16})).unwrap();
    let mut h = harness(app, egui::vec2(1200.0, 800.0));
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    let layer = LayerId(open(h.state()).layer);
    // Alpha in the preview's copy of the layer: in the lifted half (x 8..16) and the rest (16..24).
    let left = |app: &PhotocraftApp| {
        let s = app.transform_preview.as_ref().unwrap().doc.layer(layer).unwrap().surface().unwrap().clone();
        (s.sample_channel(10, 12, 3), s.sample_channel(20, 12, 3))
    };
    assert_eq!(left(h.state()), (0.0, 1.0), "the selected half is lifted out");

    h.state_mut().run("layer.setProps", json!({"layer": layer.0, "opacity": 0.4})).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(left(app), (0.0, 1.0), "still without the lifted pixels");
    let moving = app.transform_preview.as_ref().unwrap().doc.layer(layer).unwrap();
    assert!(moving.visible && (moving.opacity - 0.4).abs() < 1e-6, "what stays behind is drawn with the new opacity");
    assert!((app.transform_preview.as_ref().unwrap().opacity - 0.4).abs() < 1e-6);
}

/// #1400: selecting another layer while transforming applies the box to the layer it was on,
/// and the next transform starts on the newly selected layer.
#[test]
fn selecting_another_layer_applies_the_transform_and_the_next_one_follows() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(40, 40, 56, 60), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let second = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": first.0})).unwrap();
    app.ui.tool = Tool::Move;
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    move_box(&mut app, 20.0, 0.0);
    let steps = app.session.active().unwrap().history.past_len();
    app.run("layer.select", json!({"layer": second.0})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box left the old layer");
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(second));
    assert_eq!(st.history.past_len(), steps + 1, "applied as one step");
    let bounds = |id: LayerId| st.doc.layer(id).unwrap().surface().unwrap().content_bounds();
    assert_eq!(bounds(first), photocraft_geom::Rect::new(28, 8, 44, 24), "applied to the layer it was on");
    assert_eq!(bounds(second), photocraft_geom::Rect::new(40, 40, 56, 60), "the new layer is untouched");
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let t = open(&app);
    assert_eq!((t.layer, rect_of(t)), (second.0, [40.0, 40.0, 56.0, 60.0]), "the next box is on the selected layer");
}

/// The bounds, in document units, of the selection's outline mapped through the open box, as the
/// ants are drawn over it; `None` while the canvas draws the selection where it is.
fn ants_bounds(app: &PhotocraftApp) -> Option<Rect> {
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.as_ref().unwrap();
    // `PointMap` has no public way to apply it, so the outline goes through the box's own homography, which
    // `selection_map` is built from (tools-w28 note 5); `selection_map` itself is asserted to be present.
    let _map = super::draw::selection_map(app, st.doc.id)?;
    let t = app.ui.transform.as_ref()?;
    let h = t.session.homography()?;
    let mask = crate::canvas::outline::SurfaceMask::new(sel, st.revision, [st.doc.size.width, st.doc.size.height]);
    let mut mapped = outline(&mask, mask.content_bounds(), 1)
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .filter_map(|p| h.apply_in_front(Point::new(f64::from(p[0]), f64::from(p[1])), t.session.rect()));
    let first = mapped.next()?;
    Some(mapped.fold(Rect::from_points(first, first), |r, p| r.union_pt(p)))
}

/// The bounds of the box's corners.
fn box_bounds(quad: &[[f64; 2]; 4]) -> Rect {
    let first = Point::new(quad[0][0], quad[0][1]);
    quad.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(Point::new(p[0], p[1])))
}

fn rects_match(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() <= 1e-6)
}

/// #5297: with a selection, Free Transform's box is the selection's own rectangle (wider here
/// than the pixels it holds), and the marching ants stay on the box through a move, a scale, a
/// rotation and a distort, before anything is applied. Applying leaves them there.
#[test]
fn the_box_is_the_selection_and_the_ants_follow_it() {
    let ctx = gpu_ctx();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 30, "height": 26})).unwrap();
    assert_eq!(ants_bounds(&app), None, "no box open: the canvas draws them where they are");
    begin(&mut app, &ctx).unwrap();
    let rest = corners([4.0, 4.0, 34.0, 30.0]);
    assert_eq!(quad_of(open(&app)), rest, "the box is the selection, not the 16 px square in it");
    assert!(rects_match(ants_bounds(&app).unwrap(), box_bounds(&rest)), "untouched, the ants are where they were");

    let on_the_box = |app: &PhotocraftApp, what: &str| {
        let (ants, boxed) = (ants_bounds(app).unwrap(), box_bounds(&quad_of(open(app))));
        assert!(rects_match(ants, boxed), "{what}: ants {ants:?}, box {boxed:?}");
    };
    move_box(&mut app, 20.0, 10.0);
    on_the_box(&app, "moved");
    let moved = quad_of(open(&app));
    scale_box(&mut app, 0.5, 1.5);
    on_the_box(&app, "scaled");
    turn_box(&mut app, 0.6);
    on_the_box(&app, "rotated");
    app.ui.transform.as_mut().unwrap().edit_box(|b| b.quad[2] += Vec2::new(9.0, 13.0));
    on_the_box(&app, "distorted");

    // Applied as moved only: the outline is where the ants showed it.
    set_quad(&mut app, moved);
    commit(&mut app);
    assert!(app.ui.transform.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(24, 14, 54, 40));
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds(), photocraft_geom::Rect::new(28, 18, 44, 34));
}

/// Transform Selection moves the selection outline, not a layer: selecting a layer keeps it open.
#[test]
fn selecting_another_layer_keeps_a_transform_selection_open() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    begin_selection(&mut app, &ctx).unwrap();
    app.run("layer.select", json!({"layer": first.0})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.transform.as_ref().is_some_and(|t| t.selection));
}

/// A Distort edge drag commits what it shows, in one undo step: the box events of the drag end on
/// the parallelogram, and the commit lands there.
#[test]
fn distort_mode_edge_drag_commits_what_it_shows_in_one_step() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    let q0 = *open(&app).session.state();
    // The right edge's handle, 8 px right and 8 px down.
    let by = |k: f64| Vec2::new(8.0 * k, 8.0 * k);
    let at = |k: f64| BoxState::new([q0.quad[0], q0.quad[1] + by(k), q0.quad[2] + by(k), q0.quad[3]], q0.pivot);
    box_events(&mut app, 1, &[at(0.5), at(1.0)]);
    let shown = quad_of(open(&app));
    let q = q0.quad.map(|p| [p.x, p.y]);
    assert_eq!(shown, [q[0], [q[1][0] + 8.0, q[1][1] + 8.0], [q[2][0] + 8.0, q[2][1] + 8.0], q[3]]);
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one undo step");
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let alpha = |x: i32, y: i32| {
        let mut p = [0.0f32; 4];
        s.read_pixel(x, y, &mut p);
        p[3]
    };
    // The parallelogram (8, 8) (32, 16) (32, 32) (8, 24): its slanted top and bottom edges.
    assert!(alpha(28, 22) > 0.99 && alpha(10, 20) > 0.99, "inside {} {}", alpha(28, 22), alpha(10, 20));
    assert!(alpha(28, 11) < 0.01 && alpha(10, 28) < 0.01, "outside {} {}", alpha(28, 11), alpha(10, 28));
    let b = s.content_bounds();
    // Within the resampling filter's pixel of the quad's bounds (8, 8)..(32, 32).
    assert!((7..=8).contains(&b.x0) && (7..=8).contains(&b.y0) && (32..=33).contains(&b.x1) && (32..=33).contains(&b.y1), "{b:?}");
}

/// A path box offers Warp (Photoshop's Transform Path menu has it too, #3027); a lone mask refuses
/// it.
#[test]
fn right_click_menu_offers_warp_for_a_path() {
    let words = TransformMenuWords::english();
    let warp_row = |t: &FreeTransform| {
        t.session.menu(&words).iter().find(|e| matches!(e.value(), Some(TransformChoice::Warp))).map(|e| e.is_enabled()).expect("the menu has a Warp row")
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[10, 10], [30, 10], [30, 20], [10, 20]]}]}})).unwrap();
    app.ui.tool = Tool::PathSelection;
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let t = open(&app);
    assert!(t.path.is_some(), "the box is on the path");
    assert!(t.session.allows().warp && warp_row(t), "a path can be warped");
    cancel(&mut app);

    let mut app = masked_app(false);
    app.ui.mask_target = true;
    begin(&mut app, &ctx).unwrap();
    let t = open(&app);
    assert_eq!(t.target, Some(json!("mask")));
    assert!(!t.session.allows().warp && !warp_row(t), "a lone mask is not warped");
}

// ---- Linked layers ----

fn app_with_square(size: u32, fill: photocraft_geom::Rect) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": size, "height": size})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(fill, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app
}

/// `app_with_square` (A, red over 8..24) plus B (x 40..56, y 8..24) and C (x 8..24, y 40..56)
/// over it. A and B are linked; C isn't. Only A is selected, and active: (app, A, B, C).
fn linked_layers_app() -> (Box<PhotocraftApp>, LayerId, LayerId, LayerId) {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let a = app.session.active().unwrap().active_layer.unwrap();
    let add = |app: &mut PhotocraftApp, fill: photocraft_geom::Rect| {
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, l| {
                doc.layer_mut(l.unwrap()).unwrap().surface_mut().unwrap().fill_rect(fill, &[0.0, 0.0, 1.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.session.active().unwrap().active_layer.unwrap()
    };
    let b = add(&mut app, photocraft_geom::Rect::new(40, 8, 56, 24));
    let c = add(&mut app, photocraft_geom::Rect::new(8, 40, 24, 56));
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    app.session.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    (app, a, b, c)
}

/// #11217: ⌥⌘T on a linked layer without a selection makes no copy in the reference application
/// (measured): it frames and moves the whole link group, as Free Transform does. An unlinked
/// layer still gets its copy.
#[test]
fn free_transform_a_copy_of_a_linked_layer_boxes_the_link_group_without_a_copy() {
    let ctx = egui::Context::default();
    let (mut app, a, b, c) = linked_layers_app();
    let layers = app.session.active().unwrap().doc.layers.len();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "no copy of a linked layer");
    let t = open(&app);
    assert_eq!(t.made, None, "nothing made to take back");
    assert_eq!(t.layers, [a.0, b.0], "the box frames the link group");
    cancel(&mut app);
    app.session.execute("layer.select", json!({"layer": c.0})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "an unlinked layer is copied");
    assert_eq!(open(&app).made, Some(MadeLayer::Copy));
    cancel(&mut app);
}

/// #11217: the box over one selected linked layer frames, previews and commits its whole link
/// group, as in Photoshop. It used to frame A alone; B only moved at the commit.
#[test]
fn free_transform_of_a_linked_layer_boxes_its_link_group() {
    let (mut app, a, b, c) = linked_layers_app();
    begin(&mut app, &egui::Context::default()).unwrap();
    let t = open(&app);
    assert_eq!(t.layer, a.0, "the active layer stays the session's");
    assert_eq!(t.layers, [a.0, b.0], "a box over the link group, in stack order");
    assert_eq!(rect_of(t), [8.0, 8.0, 56.0, 24.0], "framing A and B together");
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.several.as_ref().is_some_and(|sv| sv.ids == [a, b]), "the preview moves both");
    // The moving pixels hold B's blue block where B is, beside A's red one: B moves with the box
    // on the canvas, not only at the commit.
    assert_eq!(moving_texel(&app, [48, 16]).0, Some(Color32::BLUE), "the linked layer's pixels are in the preview");
    assert_eq!(moving_texel(&app, [16, 16]).0, Some(Color32::RED), "and the active layer's");
    assert_eq!(moving_texel(&app, [16, 48]).0, None, "the unlinked layer is outside the moving pixels");
    move_box(&mut app, 4.0, 0.0);
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let bounds = |id: LayerId| st.doc.layer(id).unwrap().surface().unwrap().content_bounds();
    assert_eq!(bounds(a), photocraft_geom::Rect::new(12, 8, 28, 24));
    assert_eq!(bounds(b), photocraft_geom::Rect::new(44, 8, 60, 24));
    assert_eq!(bounds(c), photocraft_geom::Rect::new(8, 40, 24, 56), "an unlinked layer stays put");
}

/// #11217: a layer without linked layers still opens the one-layer box.
#[test]
fn free_transform_of_an_unlinked_layer_boxes_that_layer_alone() {
    let (mut app, _, _, c) = linked_layers_app();
    app.session.execute("layer.select", json!({"layer": c.0})).unwrap();
    begin(&mut app, &egui::Context::default()).unwrap();
    let t = open(&app);
    assert_eq!((t.layer, rect_of(t)), (c.0, [8.0, 40.0, 24.0, 56.0]));
    assert!(t.layers.is_empty(), "a one-layer session");
    assert!(app.transform_preview.as_ref().unwrap().several.is_none());
}

/// #11217 review: a layer linked to the Background opens the one-layer box: the link doesn't pull
/// the Background into the box, as the selection doesn't either (#6501).
#[test]
fn free_transform_of_a_layer_linked_to_the_background_boxes_that_layer_alone() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let a = app.session.active().unwrap().active_layer.unwrap();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.session.execute("layer.select", json!({"layer": bg.0})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    let app_ctx = app.ctx().clone();
    begin(&mut app, &app_ctx).unwrap();
    let t = open(&app);
    assert_eq!((t.layer, rect_of(t)), (a.0, [8.0, 8.0, 24.0, 24.0]), "the box frames A, not the canvas-sized Background");
    assert!(t.layers.is_empty(), "a one-layer session");
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.several.is_none());
    assert!(pv.doc.layer(bg).unwrap().visible, "the Background stays under the box");
}

/// #11217: a locked layer in the link group refuses the box before it opens, as a locked layer
/// among several selected ones does.
#[test]
fn a_locked_linked_layer_refuses_the_box_over_its_link_group() {
    let (mut app, a, b, _) = linked_layers_app();
    app.session.execute("layer.setProps", json!({"layer": b.0, "locks": {"position": true}})).unwrap();
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    assert_eq!(begin(&mut app, &egui::Context::default()), Err(LOCKED.to_string()));
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

// ---- Layers over the moving one ----

/// #2439: a layer under others keeps its place in the stack while the box is open. The
/// canvas under the box shows only the layers below it; the ones over it are composited on
/// their own and drawn over the moving pixels.
#[test]
fn free_transform_keeps_the_layer_under_the_layers_over_it() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let red = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(16, 16, 40, 40), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let blue = app.session.active().unwrap().active_layer.unwrap();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.session.execute("layer.select", json!({"layer": red.0})).unwrap();
    let ctx = gpu_ctx();
    begin(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.doc.layer(bg).unwrap().visible, "the layers below show under the box");
    assert!(!pv.doc.layer(blue).unwrap().visible, "the layer over it is drawn over the box instead");
    let plan = AbovePlan::new(&app.session.active().unwrap().doc, red);
    assert_eq!(plan.ids, [blue], "something covers the layer");
    assert_eq!(plan.area(), 24 * 24, "over the covering layer's frame");
    assert_eq!(plan.image(4096).expect("the composite").0.size, [24, 24]);
    cancel(&mut app);

    // The top layer: nothing to draw over it.
    app.session.execute("layer.select", json!({"layer": blue.0})).unwrap();
    begin(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(AbovePlan::new(&app.session.active().unwrap().doc, blue).image(4096).is_none());
    assert!(pv.doc.layer(red).unwrap().visible && pv.doc.layer(bg).unwrap().visible);
    cancel(&mut app);
}

/// #9387: a layer under others keeps its place in the stack when something over it blends or
/// adjusts (a PSD usually has one): the plain layers over the moving one still cover it.
#[test]
fn plain_layers_still_cover_the_moving_one_under_a_blend_or_adjustment_layer() {
    use photocraft_color::BlendMode;
    for adjustment in [false, true] {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
        let red = app.session.active().unwrap().active_layer.unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(16, 16, 40, 40), &[0.0, 0.0, 1.0, 1.0]);
                Ok(())
            })
            .unwrap();
        let blue = app.session.active().unwrap().active_layer.unwrap();
        let bg = app.session.active().unwrap().doc.layers[0].id;
        let top = if adjustment {
            app.session.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
            app.session.active().unwrap().active_layer.unwrap()
        } else {
            app.session.execute("layer.new.layer", json!({})).unwrap();
            let top = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("blend", |doc, _| {
                    doc.layer_mut(top).unwrap().blend = BlendMode::Multiply;
                    Ok(())
                })
                .unwrap();
            top
        };
        app.session.execute("layer.select", json!({"layer": red.0})).unwrap();
        begin(&mut app, &gpu_ctx()).unwrap();
        let pv = app.transform_preview.as_ref().unwrap();
        assert!(!pv.doc.layer(blue).unwrap().visible, "adjustment={adjustment}: the plain layer is drawn over the box, not under it");
        assert!(pv.doc.layer(top).unwrap().visible, "adjustment={adjustment}: the layer that blends stays in the document under the box");
        assert!(pv.doc.layer(bg).unwrap().visible);
        let plan = AbovePlan::new(&app.session.active().unwrap().doc, red);
        assert_eq!(plan.ids, [blue], "adjustment={adjustment}: the plain layer covers the moving one");
        let (image, _) = plan.image(4096).expect("the composite of the covering layer");
        assert_eq!(image.size, [24, 24], "adjustment={adjustment}");
        let centre = image.pixels[12 * image.size[0] + 12];
        // The adjustment is composited with the layer it covers: inverted blue is yellow.
        assert_eq!(centre, if adjustment { Color32::from_rgb(255, 255, 0) } else { Color32::BLUE }, "adjustment={adjustment}");
        cancel(&mut app);
    }
}

#[test]
fn plain_layers_from_the_top_cover_the_moving_one() {
    use photocraft_color::BlendMode;
    use photocraft_doc::Layer;
    let px = |name: &str| Layer::raster(name, photocraft_color::PixelFormat::RGBA8);
    let covers = |layers: &[Layer], id: LayerId| {
        let mut out = Vec::new();
        covering(layers, id, &mut out, &mut Vec::new());
        out
    };
    // In a group: the group's layers over it, then the ones over the group; never the layers
    // clipped to it.
    let (below, target, mut clip, inner_top, top) = (px("below"), px("target"), px("clip"), px("inner top"), px("top"));
    clip.clipped = true;
    let (t, c, inner, tp) = (target.id, clip.id, inner_top.id, top.id);
    let mut layers = vec![below, Layer::group("group", vec![target, clip, inner_top]), top];
    assert_eq!(covers(&layers, t), [tp, inner]);
    assert_eq!(covers(&layers, c), [tp, inner], "a clipped layer: covered by what is over its clipping group");

    // `Above::hide_in` (the document under the box) needs a built `Above`, which is a `RasterSource` after
    // tools-w28 with no public fields; only the over-the-box half (`show_only`) is asserted here.
    // Over the box, the covering layers alone show.
    let mut over = layers.clone();
    show_only(&mut over, &[tp, inner]);
    let vis = |ls: &[Layer]| [ls[0].visible, ls[1].visible, ls[2].visible];
    assert_eq!(vis(&over), [false, true, true]);
    let inside: Vec<bool> = match &over[1].content {
        LayerContent::Group(g) => g.children.iter().map(|l| l.visible).collect(),
        _ => Vec::new(),
    };
    assert_eq!(inside, [false, false, true]);

    // A layer that blends with what is under it can't be drawn on its own: it stays in the
    // document under the box, and the plain layers around it still cover (#9387).
    layers[2].blend = BlendMode::Multiply;
    assert_eq!(covers(&layers, t), [inner]);
    layers[2].blend = BlendMode::Normal;
    let mut multiply = px("multiply");
    multiply.blend = BlendMode::Multiply;
    let m = multiply.id;
    layers.insert(2, multiply);
    assert_eq!(covers(&layers, t), [tp, inner]);
    assert!(!covers(&layers, t).contains(&m));
    // A group that isn't Normal or Pass Through keeps what is inside it.
    layers[1].blend = BlendMode::Screen;
    layers.remove(2);
    assert_eq!(covers(&layers, t), [tp]);
    assert_eq!(covers(&layers, LayerId::fresh()), [tp, layers[0].id], "not in the stack: every plain layer, the Screen group passed over");
}

/// #9387: an adjustment layer over covering layers is composited with them as well as staying
/// under the box; one with nothing covering under it has nothing to adjust.
#[test]
fn adjustments_over_covering_layers_are_applied_to_them() {
    use photocraft_doc::{Adjustment, Layer};
    let px = |name: &str| Layer::raster(name, photocraft_color::PixelFormat::RGBA8);
    let adjustment = |name: &str| Layer::new(name, LayerContent::Adjustment(Adjustment::Invert));
    let find = |layers: &[Layer], id: LayerId| {
        let (mut out, mut applied) = (Vec::new(), Vec::new());
        covering(layers, id, &mut out, &mut applied);
        (out, applied)
    };
    let (target, under, over, adj_top, adj_between, adj_under) =
        (px("target"), px("under"), px("over"), adjustment("top"), adjustment("between"), adjustment("under"));
    let (t, u, o, top, between, below) = (target.id, under.id, over.id, adj_top.id, adj_between.id, adj_under.id);
    // Bottom to top: target, an adjustment with nothing to adjust over it, two plain layers
    // around another adjustment, and an adjustment on top.
    let layers = vec![target, adj_under, under, adj_between, over, adj_top];
    let (covers, applied) = find(&layers, t);
    assert_eq!(covers, [o, u]);
    assert_eq!(applied, [top, between], "adjustments over a covering layer, top down");
    assert!(!applied.contains(&below), "nothing covers under it");

    // Nothing covers: no adjustment applies.
    let alone = vec![px("a"), adjustment("b")];
    assert_eq!(find(&alone, alone[0].id), (Vec::new(), Vec::new()));
}

/// A context that reports a real GPU's texture limit (the default reports 2048).
fn gpu_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    // Clear the font atlas delta: epaint panics on dropping unapplied deltas in debug builds.
    let mut out = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |_| {});
    out.textures_delta.clear();
    ctx
}

#[test]
fn nested_group_preview_excludes_its_siblings() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::Layer;

    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(st.doc_mut());
    let target_pixels = doc.layers.pop().unwrap();
    let mut sibling = Layer::raster("sibling", PixelFormat::RGBA8);
    sibling.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(12, 12, 20, 20), &[0.0, 0.0, 1.0, 1.0]);
    let target = Layer::group("target", vec![target_pixels]);
    let target_id = target.id;
    doc.layers.push(Layer::group("parent", vec![target, sibling]));

    let (image, _) = preview_image(&st.doc, target_id, None, photocraft_geom::Rect::new(8, 8, 24, 24), 4096, false);
    assert_eq!(image.pixels[8 * image.size[0] + 8], Color32::from_rgb(255, 0, 0), "a sibling in the parent group must not be baked into the moving texture");
}

fn artboard_preview_app(background: photocraft_doc::ArtboardBackground) -> (Box<PhotocraftApp>, LayerId, LayerId) {
    use photocraft_doc::{Artboard, Layer};
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(st.doc_mut());
    let red = doc.layers.pop().unwrap();
    let red_id = red.id;
    let mut blue = Layer::raster("covering blue", photocraft_color::PixelFormat::RGBA8);
    blue.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(16, 16, 40, 40), &[0.0, 0.0, 1.0, 1.0]);
    let mut board = Layer::group("board", vec![red, blue]);
    let board_id = board.id;
    if let LayerContent::Group(g) = &mut board.content {
        let mut ab = Artboard::new(photocraft_geom::Rect::new(0, 0, 64, 64));
        ab.background = background;
        g.artboard = Some(ab);
    }
    doc.layers = vec![board].into();
    (app, board_id, red_id)
}

#[test]
fn artboard_transform_overlay_does_not_cover_moving_pixels_with_the_board_background() {
    use photocraft_doc::ArtboardBackground;
    for background in [ArtboardBackground::White, ArtboardBackground::Black] {
        let (mut app, board_id, red_id) = artboard_preview_app(background);
        let original = app.session.active().unwrap().doc.clone();
        begin(&mut app, &gpu_ctx()).unwrap();
        let pv = app.transform_preview.as_ref().unwrap();
        let (image, _) = AbovePlan::new(&original, red_id).image(4096).expect("the covering layers' composite");
        // The board's composite spans the whole 64 px board,
        // so the texel of document point (x, y) is at (x, y).
        assert_eq!(image.size, [64, 64], "the composite covers the board");
        let at = |x: usize, y: usize| image.pixels[y * image.size[0] + x];
        assert_eq!(at(12, 12), Color32::TRANSPARENT, "the board background belongs under the moving layer");
        assert_eq!(at(20, 20), Color32::BLUE, "covering siblings must still cover the moving layer");
        let under = photocraft_compose::render(&pv.doc, photocraft_geom::Rect::new(0, 0, 64, 64));
        assert_eq!(under.px[12 * 64 + 12], background.rgba().unwrap(), "the board background remains on the base canvas");
        assert!(!pv.doc.layer(red_id).unwrap().visible);
        cancel(&mut app);
        let LayerContent::Group(g) = &app.session.active().unwrap().doc.layer(board_id).unwrap().content else { panic!("board") };
        assert_eq!(g.artboard.as_ref().unwrap().background, background);
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &original), "preview and cancellation do not edit the document");
    }
}

#[test]
fn artboard_solo_preview_keeps_holes_transparent_and_the_board_clip() {
    use photocraft_doc::ArtboardBackground;
    let (app, board_id, red_id) = artboard_preview_app(ArtboardBackground::White);
    let original = &app.session.active().unwrap().doc;
    let mut solo = (**original).clone();
    solo.layer_mut(red_id).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(-2, -2, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
    solo_layer(&mut solo.layers, red_id);
    let b = photocraft_geom::Rect::new(-2, -2, 66, 66);
    let rendered = photocraft_compose::render(&solo, b);
    let at = |x: i32, y: i32| rendered.px[(y + 2) as usize * 68 + (x + 2) as usize];
    assert_eq!(at(12, 12), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(30, 30), [0.0; 4], "neither the board's white nor another child enters the moving texture");
    assert_eq!(at(-1, -1), [0.0; 4], "the artboard still clips its children");
    let LayerContent::Group(g) = &solo.layer(board_id).unwrap().content else { panic!("board") };
    assert!(g.artboard.is_some(), "keep the board boundary while omitting its background");
}

#[test]
fn artboard_moved_as_a_whole_keeps_its_background() {
    use photocraft_doc::ArtboardBackground;
    let (app, board_id, _) = artboard_preview_app(ArtboardBackground::Black);
    let mut doc = (*app.session.active().unwrap().doc).clone();
    solo_layer(&mut doc.layers, board_id);
    let rendered = photocraft_compose::render(&doc, photocraft_geom::Rect::new(0, 0, 64, 64));
    assert_eq!(rendered.px[50 * 64 + 50], [0.0, 0.0, 0.0, 1.0]);
    show_only(&mut doc.layers, &[board_id]);
    let rendered = photocraft_compose::render(&doc, photocraft_geom::Rect::new(0, 0, 64, 64));
    assert_eq!(rendered.px[50 * 64 + 50], [0.0, 0.0, 0.0, 1.0], "a whole covering artboard keeps its own background");
}

#[test]
fn nested_group_preview_detaches_every_clipped_ancestor() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::Layer;

    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(st.doc_mut());
    let target_pixels = doc.layers.pop().unwrap();
    let target = Layer::group("target", vec![target_pixels]);
    let target_id = target.id;
    let mut middle = Layer::group("middle clipped ancestor", vec![target]);
    middle.clipped = true;
    let mut inner_base = Layer::raster("hidden inner clipping base", PixelFormat::RGBA8);
    inner_base.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0, 0.0, 1.0, 1.0]);
    let mut outer = Layer::group("outer clipped ancestor", vec![inner_base, middle]);
    outer.clipped = true;
    doc.layers.push(outer);

    let (image, _) = preview_image(&st.doc, target_id, None, photocraft_geom::Rect::new(8, 8, 24, 24), 4096, false);
    assert_eq!(image.pixels[8 * image.size[0] + 8], Color32::from_rgb(255, 0, 0), "hidden clipping bases must not make a nested target disappear");
}

#[test]
fn several_preview_detaches_clipped_ancestors() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::Layer;
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(st.doc_mut());
    let first = doc.layers.pop().unwrap();
    let first_id = first.id;
    let mut second = Layer::raster("second", PixelFormat::RGBA8);
    second.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(32, 8, 48, 24), &[0., 1., 0., 1.]);
    let second_id = second.id;
    let mut group = Layer::group("clipped", vec![first, second]);
    group.clipped = true;
    doc.layers.push(Layer::raster("hidden base", PixelFormat::RGBA8));
    doc.layers.push(group);
    let shown = Several::shown(doc, &[first_id, second_id], &[]);
    let (image, _) = composite_image(&shown, photocraft_geom::Rect::new(0, 0, 64, 64), 4096);
    assert_eq!(image.pixels[16 * image.size[0] + 16], Color32::from_rgb(255, 0, 0));
    assert_eq!(image.pixels[16 * image.size[0] + 40], Color32::from_rgb(0, 255, 0));
}

#[test]
fn unknown_preview_target_keeps_the_previous_document_fallback() {
    let app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active().unwrap();
    // The canvas's preview document (`pv.doc` after `show_in_canvas`) cannot be reached from a
    // trial without a canvas; the fallback is `preview_image`'s, which `show_in_canvas` reads.
    let (image, _) = preview_image(&st.doc, LayerId::fresh(), None, photocraft_geom::Rect::new(8, 8, 24, 24), 4096, false);
    assert_eq!(image.pixels[8 * image.size[0] + 8], Color32::from_rgb(255, 0, 0));
}

/// The preview reads the layer at full size under the GPU's texture limit and box-filters it by
/// the smallest whole factor above it.
#[test]
fn full_resolution_unless_over_the_texture_limit() {
    let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    // A one-pixel checkerboard: any downsampling would turn it grey.
    for y in 0..300 {
        for x in 0..500 {
            if (x + y) % 2 == 0 {
                s.fill_rect(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &[1.0, 1.0, 1.0, 1.0]);
            }
        }
    }
    let r = photocraft_geom::Rect::new(0, 0, 500, 300);
    let full = super::preview::read_surface(&s, r, 8192);
    assert_eq!(full.image.size, [500, 300]);
    assert_eq!(full.base_uv, [1.0, 1.0]);
    assert_eq!(full.image.pixels[0], Color32::WHITE);
    assert_eq!(full.image.pixels[1].a(), 0);
    // Over the limit: box-filtered (grey-ish average), uv covers the padded edge.
    let small = super::preview::read_surface(&s, r, 256);
    assert_eq!(small.image.size, [250, 150]);
    assert!((100..=155).contains(&small.image.pixels[0].a()), "{:?}", small.image.pixels[0]);
    assert_eq!(small.base_uv, [1.0, 1.0]);
    let odd = super::preview::read_surface(&s, photocraft_geom::Rect::new(0, 0, 301, 3), 256);
    assert_eq!(odd.image.size, [151, 2]);
    assert!(odd.base_uv[0] < 1.0 && odd.base_uv[1] < 1.0);
    // Empty rect: a placeholder, no panic.
    assert_eq!(super::preview::read_surface(&s, photocraft_geom::Rect::new(5, 5, 5, 9), 256).image.size, [1, 1]);
}

#[test]
fn transform_selection_moves_only_the_outline() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let px0 = app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 64, 64));
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "select.transformSelection", json!({})).unwrap();
    let t = open(&app);
    assert!(t.selection && rect_of(t) == [8.0, 8.0, 24.0, 24.0]);
    assert!(app.transform_preview.as_ref().unwrap().doc.selection.is_none(), "ants hidden while transforming");
    scale_box(&mut app, 2.0, 2.0);
    commit(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.selection.as_ref().unwrap().content_bounds();
    assert!(b.width().abs_diff(32) <= 2 && b.x0.abs_diff(0) <= 1, "{b:?}");
    assert_eq!(st.doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 64, 64)), px0);
}

// ---- Warp ----

/// A 64 px document with a red square over 8..32, in Warp mode on its default grid.
fn warping_square() -> Box<PhotocraftApp> {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    let app_ctx = app.ctx().clone();
    begin_warp(&mut app, &app_ctx).unwrap();
    app
}

/// A warp anchor dragged with its handles, a split cut through the grid, and the commit.
#[test]
fn warp_mode_drags_anchor_with_handles_splits_and_commits() {
    let mut app = warping_square();
    let ctx = app.ctx().clone();
    assert!(session_warp(&app).warp.is_identity());
    // Drag the bottom-right anchor (point 15) by (+10, +6): its two handles follow.
    let before = warp_mesh(&app);
    let mut dragged = before.clone();
    for k in [15, 14, 11] {
        dragged.points[k] += Vec2::new(10.0, 6.0);
    }
    pull_mesh(&mut app, 1, dragged);
    let m = warp_mesh(&app);
    assert_eq!(m.points[15], Point::new(42.0, 38.0));
    let near = |a: Point, b: Point| (a - b).hypot() < 1e-9;
    assert!(near(m.points[14], Point::new(24.0 + 10.0, 38.0)), "{:?}", m.points[14]);
    assert!(near(m.points[11], Point::new(42.0, 24.0 + 6.0)), "{:?}", m.points[11]);
    split(&mut app, "edit.transform.splitWarpCrosswise", None).unwrap();
    let split_m = warp_mesh(&app);
    assert_eq!(split_m.points.last().copied(), Some(Point::new(42.0, 38.0)), "the corner survives the cut");
    // The split adds only its own lines, as the reference's does: the Default grid's thirds stay drawn handles.
    assert!(split_m.us.iter().any(|u| (u - 0.5).abs() < 1e-6) && !split_m.us.iter().any(|u| (u - 1.0 / 3.0).abs() < 1e-6), "{:?}", split_m.us);
    commit(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.x1 >= 41 && b.y1 >= 37, "{b:?}");
    assert!(app.ui.transform.is_none());
    // Toggling back to the box drops the warp.
    begin_warp(&mut app, &ctx).unwrap();
    leave_warp(&mut app);
    assert!(open(&app).session.warp().is_none());
}

#[test]
fn a_thin_layer_scaled_then_warped_commits_the_curve() {
    // #5793: a 3 px column stretched wide, switched to Warp and pulled down in the middle.
    // The commit came out straight: the warp's grid was one cell across the 3 source pixels.
    let mut app = app_with_square(128, photocraft_geom::Rect::new(40, 20, 43, 60));
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    scale_box(&mut app, 30.0, 1.0);
    enter_warp(&mut app, &ctx);
    // The bottom edge's two inner points pulled 40 px down: the middle of the edge sags 30 px.
    let mut pulled = warp_mesh(&app);
    for k in [13, 14] {
        pulled.points[k].y += 40.0;
    }
    pull_mesh(&mut app, 1, pulled);
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let alpha = |x: i32, y: i32| {
        let mut p = [0.0f32; 4];
        s.read_pixel(x, y, &mut p);
        p[3]
    };
    // The middle sags below the band's old bottom edge (y 60). The pulled edge is a cubic with its inner points 40 px
    // down: across the band (x -3.5 to 86.5) it hangs 120 t (1 - t) px, 30 in the middle and about 20 at x 15, so y 82
    // is inside the band in the middle and below it at x 15. The band is opaque only between its outer source pixels'
    // centres (x 11.5 to 71.5): the 30× stretch spreads each edge pixel over 30 px.
    assert!(alpha(41, 70) > 0.99 && alpha(41, 82) > 0.99, "middle {} {}", alpha(41, 70), alpha(41, 82));
    assert!(alpha(15, 82) < 0.01 && alpha(15, 40) > 0.99, "side {} {}", alpha(15, 82), alpha(15, 40));
}

/// #7885: a surface drag (a press inside a cell, on no anchor or handle) commits exactly as
/// the preview draws it. The layer is a gradient whose colour encodes where each pixel came
/// from, so every committed pixel says which source point landed there; the preview puts
/// source point `p` at `warp.map(p)`, and the commit must agree.
#[test]
fn warp_surface_drag_commits_as_previewed() {
    const N: i32 = 96;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": N, "height": N})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let code = |x: f64, y: f64| [x / (N - 1) as f64, y / (N - 1) as f64];
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..N {
                for x in 0..N {
                    let [r, g] = code(x as f64, y as f64);
                    s.write_pixel(x, y, &[r as f32, g as f32, 0.5, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    let app_ctx = app.ctx().clone();
    begin_warp(&mut app, &app_ctx).unwrap();
    // A pull of the surface inside the top-left cell: the grid's four inner points move.
    let (start, by) = ([30.0, 30.0], Vec2::new(9.0, -5.0));
    let mut pulled = warp_mesh(&app);
    for k in [5, 6, 9, 10] {
        pulled.points[k] += by;
    }
    pull_mesh(&mut app, 1, pulled);
    // What the preview draws: the mapping of the warp the session holds.
    let previewed = session_warp(&app).warp.clone();
    let landed = previewed.map(start[0], start[1]);
    assert!((landed.0 - start[0]).abs() > 1.0 || (landed.1 - start[1]).abs() > 1.0, "the pulled surface moved: {landed:?}");
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let from = |x: f64, y: f64| {
        let mut p = [0.0f32; 4];
        s.read_pixel(x.round() as i32, y.round() as i32, &mut p);
        [p[0] as f64 * (N - 1) as f64, p[1] as f64 * (N - 1) as f64]
    };
    // The grabbed point's pixel moved with the pull: the edit did not get lost.
    let got = from(landed.0, landed.1);
    assert!((got[0] - start[0]).abs() < 2.0 && (got[1] - start[1]).abs() < 2.0, "pixel at the landing point came from {got:?}, not {start:?}");
    // And everywhere else the committed pixels sit where the preview put them.
    for j in 1..12 {
        for i in 1..12 {
            let src = [i as f64 * 8.0, j as f64 * 8.0];
            let (dx, dy) = previewed.map(src[0], src[1]);
            if !(1.0..(N - 2) as f64).contains(&dx) || !(1.0..(N - 2) as f64).contains(&dy) {
                continue;
            }
            let got = from(dx, dy);
            assert!((got[0] - src[0]).abs() < 2.0 && (got[1] - src[1]).abs() < 2.0, "{src:?} previewed at {:?} but committed {got:?} there", (dx, dy));
        }
    }
}

/// An armed Split button places one split line, then the mode ends (#12498). The grid reports
/// the split as a `Set` of the cut mesh.
#[test]
fn an_armed_split_places_one_line_and_disarms() {
    let mut app = app_with_square(512, photocraft_geom::Rect::new(56, 56, 456, 356));
    let app_ctx = app.ctx().clone();
    begin_warp(&mut app, &app_ctx).unwrap();
    app.transform_preview.as_mut().unwrap().split = Some(WarpSplit::Vertical);
    let before = warp_mesh(&app);
    let cut = before.split(WarpSplit::Vertical, 0, 0, 0.5, 0.5);
    let mut events = EditEvents::new();
    events.push(EditEvent::set(Mixed::Same(before.clone()), cut, "Split warp".into(), None));
    grid_edit(&mut app, &events, true);
    assert_eq!(warp_mesh(&app).us.len(), before.us.len() + 1, "one column of anchors at the cut");
    assert_eq!(warp_mesh(&app).vs, before.vs, "no row");
    assert!(app.transform_preview.as_ref().unwrap().split.is_none(), "the mode ends after one split");
}

/// A preset warp becomes a custom one only when an edit reaches the grid; a pass with no edit
/// leaves it alone (no invisible undo step).
#[test]
fn a_press_on_a_preset_warp_converts_it_only_when_it_grabs_the_grid() {
    let mut app = warping_square();
    let preset = Warp::preset(WarpStyle::Arc, 50.0, rect_of(open(&app)));
    set_warp(&mut app, preset.clone());
    // A press that misses the grid reports no edit.
    grid_edit(&mut app, &EditEvents::new(), false);
    assert_eq!(session_warp(&app).warp, preset, "no edit changes nothing");
    // An edit of the grid: the preset becomes a custom mesh.
    let mut tugged = warp_mesh(&app);
    tugged.points[5] += Vec2::new(3.0, 2.0);
    pull_mesh(&mut app, 1, tugged);
    assert_eq!(session_warp(&app).warp.style, WarpStyle::Custom);
}

// ---- Layer masks (#205) ----

/// `app_with_square` (red over 8..24) plus a reveal-all mask hiding x < 16.
fn masked_app(linked: bool) -> Box<PhotocraftApp> {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    app.session
        .edit("mask", |doc, a| {
            let m = doc.layer_mut(a.unwrap()).unwrap().mask.as_mut().unwrap();
            m.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 16, 64), &[0.0]);
            m.linked = linked;
            Ok(())
        })
        .unwrap();
    app
}

fn texel_alpha(img: &egui::ColorImage, x: usize, y: usize) -> u8 {
    img.pixels[y * img.size[0] + x].a()
}

/// The moving pixels, converted for the monitor the way the canvas display converts them.
fn shown_texels(app: &PhotocraftApp) -> egui::ColorImage {
    let raw = app.transform_preview.as_ref().unwrap().textures.level_image(0).expect("the moving pixels").0;
    match super::draw::preview_display(app, app.ctx()) {
        Some(display) => (display.convert)(&raw),
        None => (*raw).clone(),
    }
}

#[test]
fn transform_preview_preserves_displayed_profile_colors() {
    let bounds = photocraft_geom::Rect::new(0, 0, 8, 8);
    for (wide_gamut, monitor, warp) in [(false, "srgb", false), (true, "srgb", false), (false, "display-p3", false), (true, "srgb", true)] {
        let mut app = app_with_square(8, bounds);
        app.session
            .edit("original color sample", |doc, active| {
                if wide_gamut {
                    doc.icc_profile = Some(craft::cms::Builtin::AdobeRgbCompat.profile().to_bytes());
                }
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.55, 0.35, 0.2, 1.0]);
                Ok(())
            })
            .unwrap();
        app.session.color.settings.monitor_profile = monitor.into();
        let st = app.session.active().unwrap();
        let display = app.session.color.canvas_display(&st.doc).unwrap();
        assert_eq!(!display.is_identity(), wide_gamut || monitor != "srgb");
        let composite = photocraft_compose::render(&st.doc, bounds);
        let expected = crate::canvas::display_image(Some(&display), &composite);
        let ctx = gpu_ctx();
        if warp {
            begin_warp(&mut app, &ctx).unwrap();
        } else {
            begin(&mut app, &ctx).unwrap();
        }
        let shown = shown_texels(&app);
        assert_eq!(shown.pixels[0], expected.pixels[0], "transform changed displayed color; wide_gamut={wide_gamut}, monitor={monitor}, warp={warp}");
    }
}

#[test]
fn preview_applies_the_linked_mask() {
    for linked in [true, false] {
        let app = masked_app(linked);
        let st = app.session.active().unwrap();
        let id = st.active_layer.unwrap();
        let b = photocraft_geom::Rect::new(8, 8, 24, 24);
        let (img, _) = preview_image(&st.doc, id, None, b, 4096, false);
        // Texel x = doc x − 8: x 10 is hidden by the mask, x 20 shows.
        assert_eq!(texel_alpha(&img, 12, 4), 255);
        assert_eq!(texel_alpha(&img, 2, 4), if linked { 0 } else { 255 }, "linked={linked}");
    }
    // With a selection the lifted pixels carry the mask too.
    let mut app = masked_app(true);
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 12, "height": 16})).unwrap();
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.clone().unwrap();
    let id = st.active_layer.unwrap();
    let (lifted, _) = photocraft_engine::transform_cmds::split_selected(st.doc.layer(id).unwrap().surface().unwrap(), &sel).unwrap();
    let b = photocraft_geom::Rect::new(8, 8, 20, 24);
    let (img, _) = preview_image(&st.doc, id, Some(&lifted), b, 4096, false);
    assert_eq!(texel_alpha(&img, 2, 4), 0, "masked");
    assert_eq!(texel_alpha(&img, 10, 4), 255, "revealed");
}

/// The Transform Selection preview is the blue tint over the selection, not the document's own
/// (red) pixels under a monitor profile.
#[test]
fn transform_selection_tint_is_not_document_color() {
    let mut app = app_with_square(8, photocraft_geom::Rect::new(0, 0, 8, 8));
    app.session.color.settings.monitor_profile = "display-p3".into();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    let ctx = gpu_ctx();
    begin_selection(&mut app, &ctx).unwrap();
    let (raw, _) = app.transform_preview.as_ref().unwrap().textures.level_image(0).unwrap();
    assert!(raw.pixels.iter().any(|p| p.a() > 0), "the selection has a tint");
    for p in raw.pixels.iter().filter(|p| p.a() > 0) {
        assert!(p.b() > p.r() && p.b() > p.g(), "the tint is blue, not the document's red: {p:?}");
    }
}

#[test]
fn transform_covering_layer_uses_the_same_monitor_profile() {
    let bounds = photocraft_geom::Rect::new(0, 0, 8, 8);
    let mut app = app_with_square(8, bounds);
    let lower = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let upper = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("original color samples", |doc, _| {
            doc.layer_mut(lower).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.55, 0.35, 0.2, 1.0]);
            doc.layer_mut(upper).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.2, 0.5, 0.7, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.color.settings.monitor_profile = "display-p3".into();
    let doc = app.session.active().unwrap().doc.clone();
    let display = app.session.color.canvas_display(&doc).unwrap();
    let expected = [lower, upper].map(|id| {
        let mut solo = (*doc).clone();
        for layer in &mut solo.layers {
            layer.visible = layer.id == id;
        }
        crate::canvas::display_image(Some(&display), &photocraft_compose::render(&solo, bounds)).pixels[0]
    });
    app.session.execute("layer.select", json!({"layer": lower.0})).unwrap();
    let ctx = gpu_ctx();
    begin(&mut app, &ctx).unwrap();
    // The moving pixels and the covering layers' composite go through one display conversion.
    let moving = shown_texels(&app).pixels[0];
    let (covering_raw, _) = AbovePlan::new(&doc, lower).image(4096).expect("the upper layer covers");
    let covering = match super::draw::preview_display(&app, app.ctx()) {
        Some(display) => (display.convert)(&covering_raw),
        None => covering_raw,
    }
    .pixels[0];
    assert_eq!([moving, covering], expected, "moving pixels and covering pixels use the same display conversion");
}

#[test]
fn preview_feathers_the_linked_mask() {
    // A feathered pixel mask without a vector mask: the compositor softens the edge at x 16,
    // so the moving texels must be soft there too, not hard until the commit (#4560).
    let mut app = masked_app(true);
    app.session
        .edit("feather", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().mask.as_mut().unwrap().feather = 4.0;
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let b = photocraft_geom::Rect::new(8, 8, 24, 24);
    let (img, _) = preview_image(&st.doc, id, None, b, 4096, false);
    // Texel x 7 = doc x 15, just inside the hidden half; texel x 8 = doc x 16, just revealed.
    assert!(texel_alpha(&img, 7, 4) > 0, "the feather reaches into the hidden side");
    assert!(texel_alpha(&img, 8, 4) < 255, "the feather reaches into the revealed side");
    // The same values the compositor masks the committed layer with.
    let m = photocraft_compose::masks::combined_mask(st.doc.layer(id).unwrap(), b).unwrap();
    for x in [6usize, 7, 8, 9, 15] {
        let want = (m.sample_channel(8 + x as i32, 12, 0) * 255.0).round() as i32;
        assert!((i32::from(texel_alpha(&img, x, 4)) - want).abs() <= 1, "texel {x}");
    }
}

/// #5239: a smart object clipped to the layer under it previewed as nothing while the box
/// was open (its flattened render hid the clipping base too).
#[test]
fn preview_of_a_clipped_smart_object_is_not_empty() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.session.execute("layer.createClippingMask", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    assert!(st.doc.layer(id).unwrap().clipped);
    let (img, _) = preview_image(&st.doc, id, None, photocraft_geom::Rect::new(0, 0, 64, 64), 4096, false);
    assert_eq!(texel_alpha(&img, 32, 32), 255, "the moving object shows");
}

#[test]
fn targeted_unlinked_mask_transforms_alone_through_the_box() {
    let mut app = masked_app(false);
    app.ui.mask_target = true;
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    let t = open(&app);
    assert_eq!(t.target, Some(json!("mask")));
    assert_eq!(rect_of(t), [0.0, 0.0, 16.0, 64.0], "the box frames the mask's content");
    // The preview document shows the mask vacated (revealed).
    let pd = &app.transform_preview.as_ref().unwrap().doc;
    assert!(pd.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(4, 4, 0) > 0.99);
    // Move the box 20 px right and commit.
    move_box(&mut app, 20.0, 0.0);
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let m = &l.mask.as_ref().unwrap().surface;
    assert!(m.sample_channel(4, 30, 0) > 0.99 && m.sample_channel(30, 30, 0) < 0.01, "mask moved 20 px right");
    assert_eq!(l.surface().unwrap().content_bounds(), photocraft_geom::Rect::new(8, 8, 24, 24), "pixels untouched");
    // Warp mode doesn't apply to a lone mask.
    app.ui.mask_target = true;
    begin(&mut app, &ctx).unwrap();
    let app_ctx = app.ctx().clone();
    enter_warp(&mut app, &app_ctx);
    assert!(open(&app).session.warp().is_none());
}

#[test]
fn mask_view_keeps_move_and_transform_available_behind_a_colour_set() {
    for view in ["gray", "overlay"] {
        let mut app = masked_app(false);
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.run("view.layerMask", json!({"mode":view})).unwrap();
        assert!(crate::menus::is_enabled(&app, "edit.freeTransform"));
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let original = st.doc.layer(layer).unwrap().surface().unwrap().clone();
        let app_ctx = app.ctx().clone();
        begin(&mut app, &app_ctx).unwrap();
        assert_eq!(open(&app).target, Some(json!("mask")));
        move_box(&mut app, 20.0, 0.0);
        commit(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        app.ui.tool = crate::state::Tool::Move;
        app.ui.tool_options.move_auto_select = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let mods = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 28.0, y: 30.0, pressure: 1.0 }, mods);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 38.0, y: 30.0, pressure: 1.0 }, mods);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 38.0, y: 30.0 }, mods);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let actual = app.session.active().unwrap().doc.layer(layer).unwrap();
        assert_eq!(actual.surface().unwrap(), &original);
        let mask = &actual.mask.as_ref().unwrap().surface;
        assert!(mask.sample_channel(24, 30, 0) > 0.99 && mask.sample_channel(40, 30, 0) < 0.01, "only the mask moved");
    }
}

/// Batch 16 review: ⌥⌘T with the unlinked mask of a linked layer targeted still copies; only the
/// layer's own pixels take the link group's box without a copy.
#[test]
fn free_transform_a_copy_of_a_targeted_mask_on_a_linked_layer_still_copies() {
    let mut app = masked_app(false);
    let a = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let b = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    app.session.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    assert_eq!(app.session.execute("layer.linkLayers", json!({})).unwrap()["linked"], true);
    app.session.execute("layer.select", json!({"layer": a.0})).unwrap();
    app.ui.mask_target = true;
    let layers = app.session.active().unwrap().doc.layers.len();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the layer is copied");
    assert_eq!(open(&app).made, Some(MadeLayer::Copy));
    cancel(&mut app);
}

#[test]
fn linked_mask_target_transforms_the_layer() {
    let mut app = masked_app(true);
    app.ui.mask_target = true;
    let ctx = app.ctx().clone();
    begin(&mut app, &ctx).unwrap();
    assert_eq!(open(&app).target, None);
    assert_eq!(rect_of(open(&app)), [8.0, 8.0, 24.0, 24.0]);
}

/// ⌘T with Path Selection and a path transforms the path, not the layer's pixels, as one
/// undoable step; it works on the Background too.
#[test]
fn free_transform_moves_the_selected_path_not_pixels() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[10, 10], [30, 10], [30, 20], [10, 20]]}]}})).unwrap();
    app.ui.tool = Tool::PathSelection;
    assert!(crate::menus::is_enabled(&app, "edit.freeTransform"), "a path is transformable on the Background");
    let pixels = app.session.active().unwrap().doc.layers[0].surface().cloned();
    let steps = app.session.active().unwrap().history.past_len();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let t = open(&app);
    assert_eq!(rect_of(t), [10.0, 10.0, 30.0, 20.0], "the box frames the path");
    assert_eq!(t.path, Some(json!({"name": "work"})));
    // Twice as wide and 5 px down.
    set_quad(&mut app, [[10.0, 15.0], [50.0, 15.0], [50.0, 25.0], [10.0, 25.0]]);
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one step");
    let wp = st.doc.work_path.as_ref().unwrap();
    let anchors: Vec<[f64; 2]> = wp.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert_eq!(anchors, vec![[10.0, 15.0], [50.0, 15.0], [50.0, 25.0], [10.0, 25.0]]);
    assert_eq!(st.doc.layers[0].surface().cloned(), pixels, "the pixels stay put");

    // ⌘-drag of a corner (Distort) maps the path onto the box (#3027): one step.
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    app.ui.transform.as_mut().unwrap().edit_box(|b| b.quad[2] = Point::new(70.0, 40.0));
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 2, "one step: {}", app.ui.status);
    let anchors: Vec<[f64; 2]> = st.doc.work_path.as_ref().unwrap().subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    let want = [[10.0, 15.0], [50.0, 15.0], [70.0, 40.0], [10.0, 25.0]];
    assert!(anchors.iter().zip(want).all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9), "{anchors:?}");

    // Edit › Transform Path › Warp bends it: one more step.
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    assert!(open(&app).session.warp().is_some(), "Warp mode on a path");
    let bounds = rect_of(open(&app));
    set_warp(&mut app, Warp { style: WarpStyle::Arc, bend: 50.0, ..Warp::none(bounds) });
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 3, "one step: {}", app.ui.status);
    assert!(st.doc.work_path.as_ref().unwrap().subpaths[0].knots.len() > 4, "the warp bends the edges");

    // Without a path tool, ⌘T is the pixel transform again.
    app.ui.tool = Tool::Move;
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(crate::vector_ui::free_transform_path(&app).is_none());
}

/// Edit › Transform › Warp on editable type (#3004), as in Photoshop: no rasterize prompt, no
/// mesh to drag, only the warp styles; the commit is the type's own Warp Text, one step, and
/// the layer stays type.
#[test]
fn warp_on_type_takes_a_style_and_keeps_the_type_editable() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.run("type.create", json!({"x": 10, "y": 60, "text": "Hello"})).unwrap();
    let ctx = app.ctx().clone();
    let past = |app: &PhotocraftApp| app.session.active().unwrap().history.past_len();
    let before = past(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    assert!(!app.dialog_open(), "no prompt");
    let w0 = session_warp(&app).warp.clone();
    assert!(session_warp(&app).on_type, "type takes only the styles");
    assert_eq!(w0.style, WarpStyle::None);
    // No grid to cut on type.
    assert!(split(&mut app, "edit.transform.splitWarpCrosswise", None).is_err());
    assert_eq!(session_warp(&app).warp, w0, "the refused split changed nothing");
    // Pick Flag in the options bar, then Enter.
    set_warp(&mut app, Warp { style: WarpStyle::Flag, bend: 40.0, ..Warp::none(w0.bounds) });
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("still type") };
    let tw = t.warp.as_ref().expect("Warp Text set");
    assert_eq!((tw.style.as_str(), tw.value, tw.horizontal), ("warpFlag", 40.0, true));
    assert_eq!(past(&app), before + 1, "one history step");
    // Warp again shows Flag; None takes it off in one more step.
    begin_warp(&mut app, &ctx).unwrap();
    assert_eq!(session_warp(&app).warp.style, WarpStyle::Flag);
    let bounds = rect_of(open(&app));
    set_warp(&mut app, Warp::none(bounds));
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert!(matches!(&st.doc.layer(st.active_layer.unwrap()).unwrap().content, LayerContent::Text(t) if t.warp.is_none()));
    assert_eq!(past(&app), before + 2);
}

// ---- Cylinder (#4112) ----

/// The cylinder a Warp starts with, as the framework's `CylinderValues::for_box` gives it.
fn cylinder_start(bounds: [f64; 4]) -> Warp {
    Warp::cylinder(&CylinderValues::for_box(bounds).to_array(), bounds).expect("a cylinder over the box")
}

/// In Warp mode a cylinder's handle drags set its values and the preview follows; Undo steps back
/// through the drags; Enter commits the cylinder, which a smart object keeps as one with the
/// dragged values, and Warp mode shows it again as a cylinder, whose untouched commit is no step.
#[test]
fn warp_mode_drags_a_cylinders_handles_and_commits_it_as_a_cylinder() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    let b = rect_of(open(&app));
    set_warp(&mut app, cylinder_start(b));
    settle(&mut app);
    let v0 = cylinder_of(&app);
    // The bend handle, dragged 3 px down (a quarter of the 12 px half axis): both arcs bend more.
    let mut bent = v0;
    (bent[4], bent[5]) = (v0[4] + 0.25, v0[5] + 0.25);
    drag_cylinder(&mut app, 1, bent);
    let v = cylinder_of(&app);
    assert!((v[4] - (v0[4] + 0.25)).abs() < 1e-9 && (v[5] - (v0[5] + 0.25)).abs() < 1e-9, "{v:?}");
    // The preview is the mesh of the new values, and the warp is still a cylinder.
    let w = session_warp(&app).warp.clone();
    assert_eq!(w.style, WarpStyle::Cylinder);
    assert_eq!(w.mesh, Warp::cylinder(&v, b).unwrap().mesh);
    assert!(w.mesh != Warp::cylinder(&v0, b).unwrap().mesh);
    settle(&mut app);
    // The wrap handle, dragged 3 px right: a quarter of the 12 px half axis further along.
    let mut wrapped = cylinder_of(&app);
    wrapped[6] = 0.6;
    drag_cylinder(&mut app, 2, wrapped);
    assert!((cylinder_of(&app)[6] - 0.6).abs() < 1e-9, "{:?}", cylinder_of(&app));
    settle(&mut app);
    // A drag on the surface moves the whole cylinder.
    let before = cylinder_of(&app);
    let mut moved = before;
    (moved[0], moved[2], moved[1], moved[3]) = (before[0] + 4.0, before[2] + 4.0, before[1] + 2.0, before[3] + 2.0);
    drag_cylinder(&mut app, 3, moved);
    let moved = cylinder_of(&app);
    assert_eq!([moved[0] - before[0], moved[2] - before[2], moved[1] - before[1], moved[3] - before[3]], [4.0, 4.0, 2.0, 2.0], "{moved:?}");
    assert_eq!(&moved[4..], &before[4..], "the shape is unchanged");
    settle(&mut app);
    // Undo and Redo step through the drags inside the session.
    let _ = intercept(&mut app, "edit.undo");
    assert_eq!(cylinder_of(&app), before, "undo takes back the move");
    let _ = intercept(&mut app, "edit.undo");
    assert!((cylinder_of(&app)[6] - 0.85).abs() < 1e-9, "undo takes back the wrap: {:?}", cylinder_of(&app));
    let _ = intercept(&mut app, "edit.redo");
    assert!((cylinder_of(&app)[6] - 0.6).abs() < 1e-9, "redo: {:?}", cylinder_of(&app));
    let v = cylinder_of(&app);
    let steps = app.session.active().unwrap().history.past_len();
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one step");
    let photocraft_doc::LayerContent::Smart(sm) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("smart") };
    assert_eq!(sm.warp.as_ref().map(|w| w.style), Some(WarpStyle::Cylinder), "the smart object keeps a cylinder");
    // Warp mode shows the same cylinder again, with its values, and Enter changes nothing.
    begin_warp(&mut app, &ctx).unwrap();
    assert_eq!(session_warp(&app).warp.style, WarpStyle::Cylinder);
    for (a, b) in cylinder_of(&app).iter().zip(&v) {
        assert!((a - b).abs() < 1e-6, "{:?} vs {v:?}", cylinder_of(&app));
    }
    let steps = app.session.active().unwrap().history.past_len();
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "untouched: no step");
}

/// Warp mode on a smart object that already has a warp previews the source placed without it,
/// over the warp box, so the mesh warps the picture once and it sits on the grid (#9798). The
/// texels read when the box opened were the warped render, taller than the box by the lower
/// arc's bulge; warping them again put the picture well inside the lines.
#[test]
fn warp_mode_previews_a_stored_smart_warp_from_the_unwarped_source() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", json!({"style": "cylinder", "values": [8, 32, 32, 8, 0.5, 0.5, 0.85], "rect": [8, 8, 32, 32]})).unwrap();
    // The warped render reaches below the box.
    let st = app.session.active().unwrap();
    let warped = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(warped.y1 > 36, "{warped:?}");
    begin_warp(&mut app, &ctx).unwrap();
    let t = open(&app);
    assert_eq!(rect_of(t), [8.0, 8.0, 32.0, 32.0]);
    assert_eq!(t.session.warp().map(|w| w.warp.style), Some(WarpStyle::Cylinder));
    let (img, _) = app.transform_preview.as_ref().unwrap().textures.level_image(0).unwrap();
    assert_eq!(img.size, [24, 24], "the texels cover the box, not the warped render");
    // Opaque to its corners: the flat square, not the cylinder's shape (whose corners are clear).
    for (x, y) in [(0, 0), (23, 0), (0, 23), (23, 23), (12, 12)] {
        assert_eq!(img.pixels[y * 24 + x].a(), 255, "texel {x},{y}");
    }
}

/// Leaving Warp mode, and Undo back into Free Transform, show the render with the stored warp
/// again (the texels the box opened with), and re-entering shows the source without it (#9798).
#[test]
fn leaving_warp_mode_restores_the_stored_warps_render() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", json!({"style": "cylinder", "values": [8, 32, 32, 8, 0.5, 0.5, 0.85], "rect": [8, 8, 32, 32]})).unwrap();
    begin(&mut app, &ctx).unwrap();
    let texels = |app: &PhotocraftApp| app.transform_preview.as_ref().unwrap().textures.size();
    let rect = |app: &PhotocraftApp| rect_of(open(app));
    let (warped, box0) = (texels(&app), rect(&app));
    assert!(warped[1] > 24 && box0[3] > 32.0, "the render with the warp hangs below the box: {warped:?} {box0:?}");
    enter_warp(&mut app, &ctx);
    settle(&mut app);
    assert_eq!((texels(&app), rect(&app)), ([24, 24], [8.0, 8.0, 32.0, 32.0]), "the source over the warp box");
    leave_warp(&mut app);
    settle(&mut app);
    assert!(open(&app).session.warp().is_none());
    assert_eq!((texels(&app), rect(&app)), (warped, box0), "back to the render with the warp");
    enter_warp(&mut app, &ctx);
    settle(&mut app);
    assert_eq!((texels(&app), rect(&app)), ([24, 24], [8.0, 8.0, 32.0, 32.0]), "and back again, without a re-render");
    let _ = intercept(&mut app, "edit.undo");
    assert!(open(&app).session.warp().is_none());
    assert_eq!((texels(&app), rect(&app)), (warped, box0), "undo into Free Transform");
    let _ = intercept(&mut app, "edit.redo");
    assert_eq!((texels(&app), rect(&app)), ([24, 24], [8.0, 8.0, 32.0, 32.0]), "redo into Warp");
    leave_warp(&mut app);
    assert_eq!((texels(&app), rect(&app)), (warped, box0), "leaving after a redo");
}

/// A cylinder on a pixel layer commits as previewed: the pixels wrap round between the
/// silhouettes and the ones on the back are hidden.
#[test]
fn a_cylinder_commits_the_wrapped_pixels() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    let app_ctx = app.ctx().clone();
    begin_warp(&mut app, &app_ctx).unwrap();
    let b = rect_of(open(&app));
    // Silhouettes pulled in to x 12..28: the square's outer columns go round the back.
    let w = Warp::cylinder(&[12.0, 29.0, 28.0, 8.0, 0.25, 0.25, 0.5], b).unwrap();
    set_warp(&mut app, w);
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let alpha = |x: i32, y: i32| {
        let mut p = [0.0f32; 4];
        s.read_pixel(x, y, &mut p);
        p[3]
    };
    assert!(alpha(20, 20) > 0.99, "the front is covered: {}", alpha(20, 20));
    assert!(alpha(9, 20) < 0.01 && alpha(31, 20) < 0.01, "nothing outside the silhouettes: {} {}", alpha(9, 20), alpha(31, 20));
    // The upper arc's front dips below the top edge at the centre, so the top row there is clear.
    assert!(alpha(20, 9) < 0.2, "the top bows down in the middle: {}", alpha(20, 9));
    assert!(alpha(13, 9) > 0.5, "and stays up at the silhouettes: {}", alpha(13, 9));
}

/// #3592: with several layers in the box, an opacity or visibility edit made while it is open shows in the moving
/// pixels at once, and the layers stay hidden under the box.
#[test]
fn a_multi_layer_box_follows_opacity_and_visibility_edits() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let red = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(40, 40, 56, 56), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let blue = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.select", json!({"layer": red.0, "mode": "add"})).unwrap();
    app.sync_views();
    begin(&mut app, &egui::Context::default()).unwrap();
    let alpha = |app: &PhotocraftApp, x: usize, y: usize| {
        let img = app.transform_preview.as_ref().unwrap().textures.level_image(0).expect("the moving pixels").0;
        texel_alpha(&img, x, y)
    };
    // The box is (8, 8)-(56, 56): red at its top-left texel, blue at (32, 32) onwards.
    assert_eq!(alpha(&app, 0, 0), 255);
    assert_eq!(alpha(&app, 40, 40), 255);
    app.session
        .edit("opacity", |doc, _| {
            doc.layer_mut(red).unwrap().opacity = 0.5;
            doc.layer_mut(blue).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
    follow_edits(&mut app);
    let a = alpha(&app, 0, 0);
    assert!(a > 0 && a < 255, "half-opaque red: {a}");
    assert_eq!(alpha(&app, 40, 40), 0, "hidden blue drops out of the preview");
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(!pv.doc.layer(red).unwrap().visible && !pv.doc.layer(blue).unwrap().visible, "both still hidden under the box");
    assert_eq!(app.ui.transform.as_ref().unwrap().layers, vec![red.0, blue.0]);
}

/// #9974: the click waiting for the commit acts on the layer selected when it was made: no other
/// layer can be picked while the commit runs (the engine refuses document commands meanwhile).
#[test]
fn a_waiting_new_layer_click_keeps_the_layer_it_was_made_on() {
    let (mut app, placed, ..) = moved_place();
    let below = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).find(|id| *id != placed).unwrap();
    let before: Vec<_> = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect();
    app.run("layer.new.layer", json!({})).unwrap();
    let err = app.run("layer.select", json!({"layer": below.0})).unwrap_err();
    assert!(err.contains("still running"), "{err}");
    land_jobs(&mut app);
    let st = app.session.active().unwrap();
    let order: Vec<_> = st.doc.layers.iter().map(|l| l.id).collect();
    let new = *order.iter().find(|id| !before.contains(id)).expect("the new layer");
    let at = |id| order.iter().position(|l| *l == id).unwrap();
    assert_eq!(at(new), at(placed) + 1, "the new layer sits right above the layer selected at the click: {order:?}");
    assert_eq!(st.active_layer, Some(new));
}

/// #15547: once Escape or Enter ends Free Transform, the options bar shows the active tool's members again in the
/// next frame, not the transform fields.
#[test]
fn ending_the_transform_brings_back_the_tools_options_bar() {
    use egui_kittest::kittest::Queryable as _;
    for key in [egui::Key::Escape, egui::Key::Enter] {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
        app.ui.tool = Tool::Gradient;
        let mut h = harness(app, egui::vec2(1400.0, 900.0));
        h.run_steps(4);
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
        h.run_steps(2);
        assert!(h.query_by_label("Reverse").is_none(), "the transform fields replace the Gradient members");
        h.key_press(key);
        h.run_steps(2);
        assert!(h.state().ui.transform.is_none(), "{key:?} ended the transform");
        assert!(h.query_by_label("Reverse").is_some(), "after {key:?} the Gradient members are back");
    }
}

// ---- Smart objects on the craft session: the box on the placement, the warp in the contents (#3649, #8927) ----

/// A 64 × 64 document whose smart object (a 32 × 24 square) has been turned by `radians` and committed.
fn turned_smart_object(radians: f64) -> (Box<PhotocraftApp>, egui::Context, LayerId) {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    begin(&mut app, &ctx).unwrap();
    turn_box(&mut app, radians);
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let id = app.session.active().unwrap().active_layer.unwrap();
    (app, ctx, id)
}

/// The smart object `id`'s placement, `[a, b, c, d, e, f]`.
fn placement_of(app: &PhotocraftApp, id: LayerId) -> [f64; 6] {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Smart(sm) => sm.transform.m,
        _ => panic!("a smart object"),
    }
}

#[test]
fn the_box_opens_on_a_turned_smart_objects_placement_and_scales_from_there() {
    let (mut app, ctx, id) = turned_smart_object(0.5);
    let turned = placement_of(&app, id);
    assert!((turned[1].atan2(turned[0]) - 0.5).abs() < 1e-6, "{turned:?}");
    begin(&mut app, &ctx).unwrap();
    let q = quad_of(open(&app));
    assert!(((q[1][1] - q[0][1]).atan2(q[1][0] - q[0][0]) - 0.5).abs() < 1e-6, "the box runs at the object's angle: {q:?}");
    let r = rect_of(open(&app));
    assert!(((r[2] - r[0]) - 32.0).abs() < 1e-6 && ((r[3] - r[1]) - 24.0).abs() < 1e-6, "the frame is the contents' size: {r:?}");
    // Enter on the box as it opened changes nothing and adds no step.
    let steps = app.session.active().unwrap().history.past_len();
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "an untouched box adds no step");
    assert_eq!(placement_of(&app, id), turned);
    // Scaled from there, the object keeps its angle.
    begin(&mut app, &ctx).unwrap();
    scale_box(&mut app, 1.5, 1.5);
    commit(&mut app);
    land_jobs(&mut app);
    let scaled = placement_of(&app, id);
    assert!((scaled[1].atan2(scaled[0]) - 0.5).abs() < 1e-6, "still at its angle: {scaled:?}");
    assert!((scaled[0].hypot(scaled[1]) - 1.5).abs() < 1e-6, "scaled by half again: {scaled:?}");
}

#[test]
fn a_warp_on_a_turned_smart_object_is_stored_in_its_contents_and_keeps_the_placement() {
    let (mut app, ctx, id) = turned_smart_object(0.5);
    let turned = placement_of(&app, id);
    let (doc, source) = {
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(id).unwrap();
        (st.doc.clone(), photocraft_engine::warp_cmds::smart_warp_source(&st.doc, layer).unwrap().0)
    };
    let placed = photocraft_engine::warp_cmds::smart_box(&doc, doc.layer(id).unwrap()).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    // The grid lies on the placement: its corners are the placed corners.
    let mesh = warp_mesh(&app);
    let w = mesh.width();
    let last = mesh.points.len() - 1;
    for (at, want) in [(0, placed[0]), (w - 1, placed[1]), (last, placed[2]), (last + 1 - w, placed[3])] {
        let p = mesh.points[at];
        assert!((p.x - want[0]).abs() < 1e-6 && (p.y - want[1]).abs() < 1e-6, "corner {at}: {p:?} vs {want:?}");
    }
    let mut to = mesh.clone();
    to.points[last] += Vec2::new(6.0, 4.0);
    pull_mesh(&mut app, 1, to);
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    assert_eq!(placement_of(&app, id), turned, "the placement stays: the warp is the contents'");
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
    let warp = sm.warp.as_ref().expect("a stored warp");
    assert_eq!(warp.bounds, source.bounds, "over the contents' own box");
    assert!(!warp.is_identity());
}

#[test]
fn a_box_mode_chosen_in_warp_holds_the_warp_and_the_commit_applies_both() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
    let ctx = app.ctx().clone();
    begin_warp(&mut app, &ctx).unwrap();
    let mesh = warp_mesh(&app);
    let last = mesh.points.len() - 1;
    let mut to = mesh.clone();
    to.points[last] += Vec2::new(4.0, 4.0);
    pull_mesh(&mut app, 1, to.clone());
    hold_warp(&mut app);
    assert!(open(&app).session.warp().is_none(), "the box is back");
    assert!(app.transform_preview.as_ref().unwrap().held.is_some(), "with the warp held");
    move_box(&mut app, 8.0, 0.0);
    // Warp again carries on with the warp, moved with the box.
    enter_warp(&mut app, &ctx);
    for (a, b) in warp_mesh(&app).points.iter().zip(&to.points) {
        assert!((a.x - (b.x + 8.0)).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9, "{a:?} vs {b:?}");
    }
    // Held again and applied: the square is warped and moved.
    hold_warp(&mut app);
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert_eq!((b.x0, b.y0), (24, 16), "moved 8 px: {b:?}");
    assert!(b.x1 >= 59 && b.y1 >= 43, "the dragged corner reaches past the square: {b:?}");
}

#[test]
fn a_split_of_the_default_grid_adds_only_its_own_lines() {
    let mesh = WarpMesh::identity(Rect::new(0.0, 0.0, 90.0, 60.0), 1, 1);
    let cut = super::warp::rule_mesh(&mesh, super::warp::RuleEdit::SplitOnePatch { split: WarpSplit::Cross, s: 0.5, t: 0.5 }, 1.0).unwrap();
    assert_eq!((cut.columns(), cut.rows()), (2, 2), "the cross alone, not the thirds and the cross");
}

#[test]
fn a_pull_moves_the_points_the_reference_moves() {
    let mesh = WarpMesh::identity(Rect::new(0.0, 0.0, 90.0, 60.0), 1, 1);
    let got = super::warp::rule_mesh(&mesh, super::warp::RuleEdit::Surface { s: 0.5, t: 0.5, by: [0.0, -10.0] }, 2.0).unwrap();
    let base = BezierMesh::identity([0.0, 0.0, 90.0, 60.0], 1, 1);
    // The reference presses half a pixel up and left of the pointer, at (44.5, 29.5) here.
    let (s, t) = (44.5 / 90.0, 29.5 / 60.0);
    let mut want = base.clone();
    want.pull_at(&base.points, s, t, base.eval(s, t), [0.0, -10.0], 46.0);
    for (g, w) in got.points.iter().zip(&want.points) {
        assert!((g.x - w[0]).abs() < 1e-9 && (g.y - w[1]).abs() < 1e-9, "{g:?} vs {w:?}");
    }
}

#[test]
fn a_pull_pressed_at_the_grids_corner_keeps_the_pointers_point() {
    // Half a pixel up and left of the top-left corner is off the surface: the pull presses the corner itself.
    let base = BezierMesh::identity([0.0, 0.0, 90.0, 60.0], 1, 1);
    assert_eq!(super::warp::pressed(&base, 0.0, 0.0), (0.0, 0.0));
    let (s, t) = super::warp::pressed(&base, 1.0, 1.0);
    assert!((s - 89.5 / 90.0).abs() < 1e-9 && (t - 59.5 / 60.0).abs() < 1e-9, "inside from the far corner: {s} {t}");
}

#[test]
fn a_warp_built_over_the_contents_box_lies_on_a_turned_smart_objects_placement() {
    let (mut app, ctx, id) = turned_smart_object(0.5);
    let doc = app.session.active().unwrap().doc.clone();
    let placed = photocraft_engine::warp_cmds::smart_box(&doc, doc.layer(id).unwrap()).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    // Reset (or a style) builds the warp over the warp's box, the contents' own: it goes to the document through
    // the placement, where the grid and the preview read it.
    let bounds = session_warp(&app).warp.bounds;
    let built = super::warp::placed_doc_warp(&app, Warp::custom(BezierMesh::identity(bounds, 1, 1), bounds));
    let mesh = built.to_mesh(1, 1);
    let last = mesh.points.len() - 1;
    for (at, want) in [(0, placed[0]), (3, placed[1]), (last, placed[2]), (last - 3, placed[3])] {
        let p = mesh.points[at];
        assert!((p[0] - want[0]).abs() < 1e-6 && (p[1] - want[1]).abs() < 1e-6, "corner {at}: {p:?} vs {want:?}");
    }
}

#[test]
fn a_smart_objects_held_warp_goes_on_in_its_contents_after_the_box_moves() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let source = {
        let st = app.session.active().unwrap();
        photocraft_engine::warp_cmds::smart_warp_source(&st.doc, st.doc.layer(id).unwrap()).unwrap().0
    };
    begin_warp(&mut app, &ctx).unwrap();
    let mesh = warp_mesh(&app);
    let last = mesh.points.len() - 1;
    let mut to = mesh.clone();
    to.points[last] += Vec2::new(3.0, 3.0);
    pull_mesh(&mut app, 1, to);
    hold_warp(&mut app);
    // A Perspective-like move of the box: its top-left corner pulled in.
    let mut q = quad_of(open(&app));
    q[0][0] += 6.0;
    set_quad(&mut app, q);
    enter_warp(&mut app, &ctx);
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.placed_warp.is_some_and(|(_, moved)| moved), "the warp goes on in the contents, through the moved box");
    assert_eq!(session_warp(&app).warp.bounds, source.bounds, "over the contents' own box");
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
    assert_eq!(sm.warp.as_ref().map(|w| w.bounds), Some(source.bounds), "stored over the contents");
    assert!(sm.perspective.is_some(), "the box's move is the new placement");
}

#[test]
fn a_pull_under_a_perspective_starts_from_the_pressed_point_of_the_contents() {
    // The g-vp-lt200 probe: a perspective placement (contents 302 × 202), the surface pressed at document (301, 226)
    // and pulled 200 px to the left.
    let to_doc = photocraft_algo::transform::Homography([0.2053, -1.18418, 270.0, 0.0, -0.28647, 125.0, 0.0, -0.00393, 1.0]);
    let back = to_doc.inverse().unwrap();
    let contents = BezierMesh::identity([0.0, 0.0, 302.0, 202.0], 1, 1);
    let doc = contents.map_points(|p| {
        let (x, y) = to_doc.apply(p[0], p[1]);
        [x, y]
    });
    let mesh = WarpMesh::new(doc.us.clone(), doc.vs.clone(), doc.points.iter().map(|p| Point::new(p[0], p[1])).collect()).unwrap();
    let (px, py) = back.apply(301.0, 226.0);
    let (s, t) = contents.param_at([px, py]);
    let got = super::warp::in_contents(&mesh, super::warp::RuleEdit::Surface { s, t, by: [-200.0, 0.0] }, 1.0, &to_doc, &back).unwrap();
    // Point 9 in the contents, as pull_at moves it from the pressed point: the contents' point half a document pixel up
    // and left of the pointer, (300.5, 225.5), found again on the contents. The carried mesh's point at (s, t) lies
    // 36 px below the press under this perspective, and a pull from there moved it to about -478 instead; a press at
    // the pointer itself moved it to -614.
    let (x, y) = back.apply(got.points[9].x, got.points[9].y);
    assert!((x - -633.48).abs() < 0.5 && (y - 134.67).abs() < 0.01, "{x}, {y}");
}

#[test]
fn a_warped_smart_objects_box_keeps_its_warp_for_a_projective_preview() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    begin(&mut app, &ctx).unwrap();
    assert!(app.transform_preview.as_ref().unwrap().warped.is_none(), "no stored warp, nothing kept");
    cancel(&mut app);
    app.run("layer.smartObjects.warp", json!({"style": "arc", "bend": 30, "rect": [16, 16, 48, 40]})).unwrap();
    begin(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().unwrap();
    let warped = pv.warped.as_ref().expect("the stored warp is kept for the preview");
    // Its points are the stored warp's in the contents, its texels the contents unplaced over its box.
    let id = app.session.active().unwrap().active_layer.unwrap();
    let st = app.session.active().unwrap();
    let (source, place) = photocraft_engine::warp_cmds::smart_warp_source(&st.doc, st.doc.layer(id).unwrap()).unwrap();
    assert_eq!(warped.bounds, source.bounds);
    assert_eq!(warped.mesh.points, source.to_mesh(1, 1).points);
    assert_eq!(warped.place, place.0);
    assert!(warped.texture.size()[0] > 0);
}

#[test]
fn a_warp_pulled_past_the_horizon_squeezes_the_placement_and_one_inside_does_not() {
    use super::warp::{HORIZON_SQUEEZE, horizon_squeeze};
    // A perspective whose weight w = 1 - y / 200 falls to nothing at y = 200, over a box 0..100 × 0..100.
    let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0 / 200.0, 1.0];
    let bounds = [0.0, 0.0, 100.0, 100.0];
    let inside = [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]];
    assert_eq!(horizon_squeeze(m, &inside, bounds), None, "a warp inside its box stays as placed");
    // A point pulled to y = 199.9, a hair from the horizon.
    let pulled = [[0.0, 0.0], [100.0, 0.0], [100.0, 199.9], [0.0, 100.0]];
    let sq = horizon_squeeze(m, &pulled, bounds).expect("the placement gives way");
    let placed = photocraft_algo::transform::Homography(m).mul(&photocraft_algo::transform::Homography(sq));
    let w = |p: [f64; 2]| placed.0[6] * p[0] + placed.0[7] * p[1] + placed.0[8];
    // The nearest point comes back to HORIZON_SQUEEZE of the far side's weight.
    let (far, near) = (w([0.0, 0.0]), w([100.0, 199.9]));
    assert!((near / far - HORIZON_SQUEEZE).abs() < 1e-9, "{near} / {far}");
}

#[test]
fn a_split_on_a_default_grid_third_keeps_both_lines() {
    use super::warp::one_patch_split;
    // On the one patch a third is no line: the armed cross keeps both lines there, as the reference's does.
    assert_eq!(one_patch_split(WarpSplit::Cross, 1.0 / 3.0 - 0.002, 0.5), Some(WarpSplit::Cross));
    // Only the patch's own edges drop a line.
    assert_eq!(one_patch_split(WarpSplit::Cross, 0.005, 0.5), Some(WarpSplit::Horizontal));
    assert_eq!(one_patch_split(WarpSplit::Vertical, 0.5, 0.005), Some(WarpSplit::Vertical));
    assert_eq!(one_patch_split(WarpSplit::Horizontal, 0.5, 0.995), None);
}

#[test]
fn a_split_alone_is_kept_when_the_warp_is_applied() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    split(&mut app, "edit.transform.splitWarpCrosswise", Some([30.0, 26.0])).unwrap();
    assert!(!open(&app).session.is_untouched(), "a flat grid with split lines is a change");
    commit(&mut app);
    land_jobs(&mut app);
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
    let mesh = sm.warp.as_ref().and_then(|w| w.mesh.as_ref()).expect("the split grid is stored");
    assert_eq!((mesh.us.len(), mesh.vs.len()), (3, 3), "its lines come back in Warp mode");
}

#[test]
fn undo_back_into_warp_takes_the_held_warp_back_into_its_contents() {
    let (mut app, ctx, id) = turned_smart_object(0.5);
    let source = {
        let st = app.session.active().unwrap();
        photocraft_engine::warp_cmds::smart_warp_source(&st.doc, st.doc.layer(id).unwrap()).unwrap().0
    };
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    let mesh = warp_mesh(&app);
    let last = mesh.points.len() - 1;
    let mut to = mesh.clone();
    to.points[last] += Vec2::new(5.0, 3.0);
    pull_mesh(&mut app, 1, to);
    settle(&mut app);
    // Scale chosen in Warp mode holds the warp; Undo takes the session back into Warp mode.
    hold_warp(&mut app);
    settle(&mut app);
    assert!(app.transform_preview.as_ref().unwrap().held.is_some());
    let _ = intercept(&mut app, "edit.undo");
    assert!(open(&app).session.warp().is_some(), "back in Warp mode");
    super::begin::reconcile_held(&mut app);
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.held.is_none() && pv.placed_warp.is_some(), "the warp is the contents' again");
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
    assert_eq!(sm.warp.as_ref().map(|w| w.bounds), Some(source.bounds), "stored over the contents' own box");
}

/// A Perspective move, then Warp and a pull on the smart object, all undone in the open session: Warp afresh is the
/// plain Warp, with nothing of the undone move's map from the contents left in it (the user's report: the next warp
/// behaved as if the perspective were still applied).
#[test]
fn undoing_a_warp_on_a_perspective_box_leaves_no_trace_in_the_next_warp() {
    fn smart() -> (Box<PhotocraftApp>, egui::Context, LayerId) {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
        let ctx = app.ctx().clone();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        begin(&mut app, &ctx).unwrap();
        (app, ctx, id)
    }
    fn pull_and_apply(app: &mut PhotocraftApp, ctx: &egui::Context, id: LayerId) -> Warp {
        begin_warp(app, ctx).unwrap();
        settle(app);
        let mut to = warp_mesh(app);
        to.points[5] += Vec2::new(-4.0, 2.0);
        pull_mesh(app, 9, to);
        settle(app);
        commit(app);
        land_jobs(app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
        sm.warp.clone().expect("a stored warp")
    }
    let (mut plain, ctx, id) = smart();
    let expected = pull_and_apply(&mut plain, &ctx, id);

    let (mut app, ctx, id) = smart();
    let start = quad_of(open(&app));
    let pivot = open(&app).session.state().pivot;
    let persp = [[start[0][0] + 6.0, start[0][1]], [start[1][0] - 6.0, start[1][1]], start[2], start[3]];
    box_events(&mut app, 1, &[BoxState::new(persp.map(|[x, y]| Point::new(x, y)), pivot)]);
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    assert!(app.transform_preview.as_ref().unwrap().placed_warp.is_some(), "Warp on the moved box edits the contents");
    let mut to = warp_mesh(&app);
    to.points[6] += Vec2::new(3.0, -2.0);
    pull_mesh(&mut app, 2, to);
    settle(&mut app);
    for _ in 0..8 {
        if open(&app).session.is_untouched() {
            break;
        }
        let _ = intercept(&mut app, "edit.undo");
        super::begin::reconcile_held(&mut app);
    }
    assert!(open(&app).session.is_untouched(), "every step undone");
    assert_eq!(quad_of(open(&app)), start, "the box is back where it opened");
    assert!(app.transform_preview.as_ref().unwrap().placed_warp.is_none(), "no map from the contents is left");
    let got = pull_and_apply(&mut app, &ctx, id);
    assert_eq!(got.bounds, expected.bounds, "stored over the same box");
    let (a, b) = (got.mesh.as_ref().expect("a mesh"), expected.mesh.as_ref().expect("a mesh"));
    for (p, q) in a.points.iter().zip(&b.points) {
        assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6, "{p:?} vs {q:?}");
    }
}

/// Undo out of Warp on a Perspective box's smart object and Redo back in: the warp comes back with its map from the
/// contents and its texels, and applies as it would have without the round trip.
#[test]
fn undo_and_redo_across_warp_on_a_perspective_box_applies_the_same_warp() {
    fn run(round_trip: bool) -> (Warp, [f64; 6]) {
        let mut app = app_with_square(64, photocraft_geom::Rect::new(16, 16, 48, 40));
        let ctx = app.ctx().clone();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        begin(&mut app, &ctx).unwrap();
        let start = quad_of(open(&app));
        let pivot = open(&app).session.state().pivot;
        let persp = [[start[0][0] + 6.0, start[0][1]], [start[1][0] - 6.0, start[1][1]], start[2], start[3]];
        box_events(&mut app, 1, &[BoxState::new(persp.map(|[x, y]| Point::new(x, y)), pivot)]);
        begin_warp(&mut app, &ctx).unwrap();
        settle(&mut app);
        super::begin::reconcile_held(&mut app);
        if round_trip {
            let _ = intercept(&mut app, "edit.undo");
            super::begin::reconcile_held(&mut app);
            assert!(open(&app).session.warp().is_none(), "out of Warp");
            assert!(app.transform_preview.as_ref().unwrap().placed_warp.is_none(), "the box mode has no contents map");
            let _ = intercept(&mut app, "edit.redo");
            super::begin::reconcile_held(&mut app);
            assert!(open(&app).session.warp().is_some(), "back in Warp");
            assert!(app.transform_preview.as_ref().unwrap().placed_warp.is_some(), "with its map from the contents");
        }
        let mut to = warp_mesh(&app);
        to.points[6] += Vec2::new(3.0, -2.0);
        pull_mesh(&mut app, 2, to);
        settle(&mut app);
        commit(&mut app);
        land_jobs(&mut app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
        (sm.warp.clone().expect("a stored warp"), placement_of(&app, id))
    }
    let (plain, plain_at) = run(false);
    let (tripped, tripped_at) = run(true);
    assert_eq!(tripped.bounds, plain.bounds, "stored over the same box");
    let (a, b) = (tripped.mesh.as_ref().expect("a mesh"), plain.mesh.as_ref().expect("a mesh"));
    for (p, q) in a.points.iter().zip(&b.points) {
        assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6, "{p:?} vs {q:?}");
    }
    assert_eq!(tripped_at, plain_at, "the same placement");
}

/// A box mode chosen in Warp mode holds the warp; Undo takes the session back into Warp and Redo must hold it again,
/// not drop it: Enter then applies what it applies without the round trip (found by the sequence checker).
#[test]
fn redo_of_a_box_mode_chosen_in_warp_holds_the_warp_again() {
    fn run(round_trip: bool) -> (Warp, [f64; 6]) {
        let (mut app, ctx, id) = turned_smart_object(0.5);
        begin(&mut app, &ctx).unwrap();
        begin_warp(&mut app, &ctx).unwrap();
        settle(&mut app);
        super::begin::reconcile_held(&mut app);
        let mut to = warp_mesh(&app);
        to.points[5] += Vec2::new(-4.0, 3.0);
        pull_mesh(&mut app, 2, to);
        settle(&mut app);
        hold_warp(&mut app);
        settle(&mut app);
        super::begin::reconcile_held(&mut app);
        assert!(app.transform_preview.as_ref().unwrap().held.is_some(), "Scale chosen in Warp mode holds the warp");
        if round_trip {
            let _ = intercept(&mut app, "edit.undo");
            super::begin::reconcile_held(&mut app);
            assert!(open(&app).session.warp().is_some(), "Undo is back in Warp mode");
            let _ = intercept(&mut app, "edit.redo");
            super::begin::reconcile_held(&mut app);
            assert!(open(&app).session.warp().is_none(), "Redo is back in the box mode");
            assert!(app.transform_preview.as_ref().unwrap().held.is_some(), "with the warp held again");
        }
        commit(&mut app);
        land_jobs(&mut app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
        (sm.warp.clone().expect("the held warp is applied"), placement_of(&app, id))
    }
    let (plain, plain_at) = run(false);
    let (tripped, tripped_at) = run(true);
    assert_eq!(tripped.bounds, plain.bounds);
    let (a, b) = (tripped.mesh.as_ref().expect("a mesh"), plain.mesh.as_ref().expect("a mesh"));
    for (p, q) in a.points.iter().zip(&b.points) {
        assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6, "{p:?} vs {q:?}");
    }
    for (a, b) in tripped_at.iter().zip(plain_at) {
        assert!((a - b).abs() < 1e-9, "{tripped_at:?} vs {plain_at:?}");
    }
}

/// A smart object with a stored warp: a turn of its box, Warp, and both undone. Warp afresh edits the stored warp as
/// a fresh session does, not through the undone turn's map kept for a Redo (found by the sequence checker).
#[test]
fn warp_after_undoing_a_turned_warp_on_a_warped_smart_object_is_a_fresh_warp() {
    fn warped() -> (Box<PhotocraftApp>, egui::Context, LayerId) {
        let mut app = app_with_square(96, photocraft_geom::Rect::new(16, 16, 48, 40));
        let ctx = app.ctx().clone();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        begin(&mut app, &ctx).unwrap();
        begin_warp(&mut app, &ctx).unwrap();
        settle(&mut app);
        let mut to = warp_mesh(&app);
        to.points[6] += Vec2::new(4.0, -3.0);
        pull_mesh(&mut app, 1, to);
        settle(&mut app);
        commit(&mut app);
        land_jobs(&mut app);
        begin(&mut app, &ctx).unwrap();
        (app, ctx, id)
    }
    fn ending(app: &mut PhotocraftApp, ctx: &egui::Context, id: LayerId) -> (Warp, [f64; 6]) {
        begin_warp(app, ctx).unwrap();
        settle(app);
        super::begin::reconcile_held(app);
        let mut to = warp_mesh(app);
        to.points[5] += Vec2::new(-3.0, 2.0);
        pull_mesh(app, 9, to);
        settle(app);
        commit(app);
        land_jobs(app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
        (sm.warp.clone().expect("a stored warp"), placement_of(app, id))
    }
    let (mut fresh, ctx, id) = warped();
    let want = ending(&mut fresh, &ctx, id);

    let (mut app, ctx, id) = warped();
    turn_box(&mut app, -0.07);
    settle(&mut app);
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    super::begin::reconcile_held(&mut app);
    for _ in 0..8 {
        if open(&app).session.is_untouched() {
            break;
        }
        let _ = intercept(&mut app, "edit.undo");
        super::begin::reconcile_held(&mut app);
    }
    assert!(open(&app).session.is_untouched(), "every step undone");
    let got = ending(&mut app, &ctx, id);
    assert_eq!(got.0.bounds, want.0.bounds, "stored over the contents' box");
    for (a, b) in got.1.iter().zip(want.1) {
        assert!((a - b).abs() < 1e-9, "placement {:?} vs {:?}", got.1, want.1);
    }
    let (a, b) = (got.0.mesh.as_ref().expect("a mesh"), want.0.mesh.as_ref().expect("a mesh"));
    for (p, q) in a.points.iter().zip(&b.points) {
        assert!((p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6, "{p:?} vs {q:?}");
    }
}

/// Perspective on a smart object, then Warp without a pull: the reference shows the picture exactly as the Perspective
/// left it and Enter applies the Perspective alone; the picture changes only once the warp is pulled. A flat grid's
/// control points carried through the move would spread the rows evenly (the picture popped toward the middle).
#[test]
fn warp_after_an_unapplied_perspective_keeps_it_until_the_first_pull() {
    fn perspective_box() -> (Box<PhotocraftApp>, egui::Context, LayerId) {
        let mut app = app_with_square(100, photocraft_geom::Rect::new(20, 30, 80, 70));
        let ctx = app.ctx().clone();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        begin(&mut app, &ctx).unwrap();
        let pivot = open(&app).session.state().pivot;
        let quad = [[42.0, 30.0], [58.0, 30.0], [80.0, 70.0], [20.0, 70.0]];
        box_events(&mut app, 1, &[BoxState::new(quad.map(|[x, y]| Point::new(x, y)), pivot)]);
        settle(&mut app);
        (app, ctx, id)
    }
    let flat = |app: &PhotocraftApp| {
        let t = open(app);
        let w = t.session.warp().expect("in Warp mode");
        super::warp::flat_placed_warp(&app.session.active().unwrap().doc, t.layer, &w.warp, app.transform_preview.as_ref().unwrap().placed_warp)
    };
    // The Perspective applied on its own.
    let (mut want, _, wid) = perspective_box();
    commit(&mut want);
    land_jobs(&mut want);
    let want_at = placement_of(&want, wid);

    // Warp, untouched: shown and applied as the Perspective.
    let (mut app, ctx, id) = perspective_box();
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    let corners = flat(&app).expect("an untouched grid shows the box's exact map");
    for (c, q) in corners.iter().zip([[42.0, 30.0], [58.0, 30.0], [80.0, 70.0], [20.0, 70.0]]) {
        assert!((c[0] - q[0]).abs() < 1e-6 && (c[1] - q[1]).abs() < 1e-6, "{corners:?}");
    }
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
    assert!(sm.warp.as_ref().is_none_or(|w| w.is_identity()), "no warp stored");
    for (a, b) in placement_of(&app, id).iter().zip(want_at) {
        assert!((a - b).abs() < 1e-9, "{:?} vs {want_at:?}", placement_of(&app, id));
    }

    // A pull ends it: from then on the carried points.
    let (mut app, ctx, _) = perspective_box();
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    let mut to = warp_mesh(&app);
    to.points[5] += Vec2::new(2.0, 1.0);
    pull_mesh(&mut app, 2, to);
    settle(&mut app);
    assert!(flat(&app).is_none(), "a pulled warp is drawn from its carried points");
}

/// A Perspective box with its top shortened, Warp, and a long pull from near the short edge diagonally toward the long
/// one, frame by frame as the warp grid reports it. Past the placement's horizon the carried points went through
/// infinity, and the grid and the picture flipped from one side to the other with every few pixels of the pull. The
/// reference's picture passes the horizon once and settles: the placement gives way through the drag, not only when it
/// lands. No point of the shown grid may jump from far on one side of the box to far on the other between two frames.
#[test]
fn a_long_pull_under_a_perspective_does_not_flip() {
    let mut app = app_with_square(400, photocraft_geom::Rect::new(60, 90, 240, 210));
    let ctx = app.ctx().clone();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    begin(&mut app, &ctx).unwrap();
    let pivot = open(&app).session.state().pivot;
    let quad = [[126.0, 90.0], [174.0, 90.0], [240.0, 210.0], [60.0, 210.0]];
    box_events(&mut app, 1, &[BoxState::new(quad.map(|[x, y]| Point::new(x, y)), pivot)]);
    settle(&mut app);
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    let start = warp_mesh(&app);
    let to_doc = photocraft_algo::transform::Homography(app.transform_preview.as_ref().unwrap().placed_warp.expect("Warp edits the contents").0);
    let back = to_doc.inverse().unwrap();
    let gesture = GestureId::new(7);
    let centre = Point::new(150.0, 150.0);
    let mut shown = start.points.clone();
    let last = 400;
    for d in (5..=last).step_by(5) {
        let by = [d as f64 / 2f64.sqrt(), d as f64 / 2f64.sqrt()];
        let pulled = super::warp::in_contents(&start, super::warp::RuleEdit::Surface { s: 0.5, t: 0.1, by }, 1.0, &to_doc, &back).expect("a pulled mesh");
        let mut events = EditEvents::new();
        if d == 5 {
            events.push(EditEvent::begin(gesture, Mixed::Same(start.clone()), "Warp".into(), None));
        }
        events.push(if d == last { EditEvent::commit(gesture, Mixed::Same(start.clone()), pulled) } else { EditEvent::update(gesture, pulled) });
        grid_edit(&mut app, &events, false);
        settle(&mut app);
        let now = warp_mesh(&app).points;
        for (i, (p, q)) in now.iter().zip(&shown).enumerate() {
            assert!(p.x.is_finite() && p.y.is_finite(), "pull {d}: point {i} is not finite");
            // Through infinity: from far on one side of the box to far on the other in one frame.
            let crossed = |a: f64, b: f64| a * b < 0.0 && a.abs() > 500.0 && b.abs() > 500.0;
            assert!(
                !crossed(p.x - centre.x, q.x - centre.x) && !crossed(p.y - centre.y, q.y - centre.y),
                "pull {d}: point {i} flipped through infinity: {q:?} -> {p:?}"
            );
        }
        shown = now;
    }
    assert!(app.transform_preview.as_ref().unwrap().drag_placed.is_none(), "the drag's start map goes with the release");
}

/// A warped smart object: Warp and a long pull, a box mode chosen (the warp held), a corner of the box moved, and Warp
/// again. The move took the held warp's points past the box's horizon, where carried they had gone through infinity:
/// every point must stay on the picture's side, the map giving way as under a pull (found by the sequence checker).
#[test]
fn a_held_warp_moved_by_the_box_stays_on_the_pictures_side_of_its_horizon() {
    let (mut app, ctx, _) = {
        let mut app = app_with_square(96, photocraft_geom::Rect::new(16, 16, 48, 40));
        let ctx = app.ctx().clone();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        begin(&mut app, &ctx).unwrap();
        begin_warp(&mut app, &ctx).unwrap();
        settle(&mut app);
        let mut to = warp_mesh(&app);
        to.points[6] += Vec2::new(4.0, -3.0);
        pull_mesh(&mut app, 1, to);
        settle(&mut app);
        commit(&mut app);
        land_jobs(&mut app);
        (app, ctx, id)
    };
    begin(&mut app, &ctx).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    super::begin::reconcile_held(&mut app);
    let start = warp_mesh(&app);
    let (a, len) = (4.802292692204863f64, 269.5955745602801f64);
    let pulled = super::warp::rule_mesh(
        &start,
        super::warp::RuleEdit::Surface { s: 0.6616269562899482, t: 0.4973305551415017, by: [len * a.cos(), len * a.sin()] },
        1.0,
    )
    .expect("a pulled mesh");
    let gesture = GestureId::new(3);
    let mut events = EditEvents::new();
    events.push(EditEvent::begin(gesture, Mixed::Same(start.clone()), "Warp".into(), None));
    events.push(EditEvent::commit(gesture, Mixed::Same(start), pulled));
    grid_edit(&mut app, &events, false);
    settle(&mut app);
    hold_warp(&mut app);
    settle(&mut app);
    super::begin::reconcile_held(&mut app);
    let q = quad_of(open(&app));
    let pivot = open(&app).session.state().pivot;
    let moved = [[q[0][0] - 1.182275882846704, q[0][1] + 4.621861299596933], q[1], q[2], q[3]];
    box_events(&mut app, 2, &[BoxState::new(moved.map(|[x, y]| Point::new(x, y)), pivot)]);
    settle(&mut app);
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    super::begin::reconcile_held(&mut app);
    let (m, _) = app.transform_preview.as_ref().unwrap().placed_warp.expect("Warp edits the contents through the moved box");
    let back = photocraft_algo::transform::Homography(m).inverse().unwrap();
    let w = |x: f64, y: f64| m[6] * x + m[7] * y + m[8];
    let mesh = warp_mesh(&app);
    let contents: Vec<[f64; 2]> = mesh
        .points
        .iter()
        .map(|p| {
            let (x, y) = back.apply(p.x, p.y);
            [x, y]
        })
        .collect();
    let (cx, cy) = (contents.iter().map(|p| p[0]).sum::<f64>() / 16.0, contents.iter().map(|p| p[1]).sum::<f64>() / 16.0);
    let side = w(cx, cy).signum();
    for (i, p) in contents.iter().enumerate() {
        assert!(w(p[0], p[1]) * side > 0.0, "point {i} lies past the horizon (weight {} against {})", w(p[0], p[1]), w(cx, cy));
    }
}

/// A surface drag through the pull rule, frame by frame as the canvas reports it (busy until the release).
fn drag_surface(app: &mut PhotocraftApp, id: u64, s: f64, t: f64, angle: f64, len: f64) {
    let start = warp_mesh(app);
    let maps = app.transform_preview.as_ref().and_then(|pv| pv.placed_warp).and_then(|(m, _)| {
        let h = photocraft_algo::transform::Homography(m);
        Some((h, h.inverse()?))
    });
    let gesture = GestureId::new(id);
    let frames = (len / 10.0).ceil().max(1.0) as usize;
    for f in 1..=frames {
        let d = len * f as f64 / frames as f64;
        let by = [d * angle.cos(), d * angle.sin()];
        let edit = super::warp::RuleEdit::Surface { s, t, by };
        let pulled = match &maps {
            Some((to_doc, back)) => super::warp::in_contents(&start, edit, 1.0, to_doc, back),
            None => super::warp::rule_mesh(&start, edit, 1.0),
        }
        .expect("a pulled mesh");
        let mut events = EditEvents::new();
        if f == 1 {
            events.push(EditEvent::begin(gesture, Mixed::Same(start.clone()), "Warp".into(), None));
        }
        events.push(if f == frames { EditEvent::commit(gesture, Mixed::Same(start.clone()), pulled) } else { EditEvent::update(gesture, pulled) });
        grid_edit(app, &events, false);
        let now = app.nudge_bundle.now;
        app.ui.transform.as_mut().unwrap().session.settle(now, f < frames);
    }
}

/// Warp on a smart object under a Distort box: a long pull makes the placement give way, a small pull follows, and
/// Undo takes the small pull back. The map from the contents comes back with the warp it was made with; it stayed as
/// the undone pull left it, so the next drag carried the warp through the wrong map and jumped (found by the sequence
/// checker).
#[test]
fn undo_of_a_pull_brings_back_the_map_its_warp_was_made_with() {
    let (mut app, ctx, _) = turned_smart_object(0.5);
    begin(&mut app, &ctx).unwrap();
    let pivot = open(&app).session.state().pivot;
    let q = quad_of(open(&app));
    let (sin, cos) = 0.1597282707915736f64.sin_cos();
    let c = [(q[0][0] + q[1][0] + q[2][0] + q[3][0]) / 4.0, (q[0][1] + q[1][1] + q[2][1] + q[3][1]) / 4.0];
    let turned = q.map(|p| [c[0] + (p[0] - c[0]) * cos - (p[1] - c[1]) * sin, c[1] + (p[0] - c[0]) * sin + (p[1] - c[1]) * cos]);
    box_events(&mut app, 1, &[BoxState::new(turned.map(|[x, y]| Point::new(x, y)), pivot)]);
    settle(&mut app);
    let mut distorted = turned;
    distorted[2] = [turned[2][0] - 3.5494413803294567, turned[2][1] - 3.1396337075350877];
    box_events(&mut app, 2, &[BoxState::new(distorted.map(|[x, y]| Point::new(x, y)), pivot)]);
    settle(&mut app);
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    drag_surface(&mut app, 3, 0.4339069484806843, 0.31281510135156776, 1.2864430371535345, 233.2823628870126);
    settle(&mut app);
    let (after_long, mesh_long) = (app.transform_preview.as_ref().unwrap().placed_warp, warp_mesh(&app));
    let mut to = warp_mesh(&app);
    to.points[5] += Vec2::new(1.8128934714029672, 3.0611538757835515);
    pull_mesh(&mut app, 4, to);
    settle(&mut app);
    let _ = intercept(&mut app, "edit.undo");
    super::begin::reconcile_held(&mut app);
    assert_eq!(warp_mesh(&app), mesh_long, "Undo takes the small pull back");
    assert_eq!(app.transform_preview.as_ref().unwrap().placed_warp, after_long, "with the map that warp was made with");
}

/// The reference's own state after a long pull of the middle under a Perspective with the top edge at half (its file:
/// the placement from the warp's box to its stored corners, and the warp's points), and where its open session drew
/// that grid (read off its screenshot at 50 %, about a pixel each way): the left side's points on the picture's left
/// side, carried on past the object, the bottom corners far out below it. The grid is drawn through the render's
/// capped map; through the plain one the corners sat inside the picture (doc y 197) and the grid looked unrelated to
/// it.
#[test]
fn a_long_pulled_warp_under_a_perspective_shows_its_grid_as_the_reference_draws_it() {
    let placed: [f64; 9] = [
        0.42195220320486854,
        -0.3472955765368178,
        173.41558569614162,
        -6.391204905766668e-13,
        0.08072772162426656,
        96.82079868541324,
        -7.752258179945735e-16,
        -0.0011576471111578094,
        1.0,
    ];
    let contents: Vec<[f64; 2]> = vec![
        [150.0, 125.0],
        [250.0, 125.0],
        [350.0, 125.0],
        [450.0, 125.0],
        [150.0, 191.66666666666663],
        [250.1087491285402, 206.59954955455055],
        [350.1017110638, 205.63311793407522],
        [450.0000000000002, 191.66666666666663],
        [150.0, 258.33333333333326],
        [254.3499651416068, 855.6486488486745],
        [354.0684425520055, 816.9913840296676],
        [449.9999999999999, 258.33333333333326],
        [150.0, 325.0],
        [250.0310711795829, 329.26653796796677],
        [350.0290603039429, 328.99041464783096],
        [450.0, 325.0],
    ];
    let h = photocraft_algo::transform::Homography(placed);
    let stored = WarpMesh::new(
        vec![0.0, 1.0],
        vec![0.0, 1.0],
        contents
            .iter()
            .map(|p| {
                let (x, y) = h.apply(p[0], p[1]);
                Point::new(x, y)
            })
            .collect(),
    )
    .unwrap();
    let shown = super::warp::shown_mesh(&stored, Some(placed), &super::warp::ShownGrids::default());
    for (i, want) in [(0usize, [226.0, 125.0]), (4, [134.0, 370.0]), (8, [22.0, 664.0]), (12, [-110.0, 1014.0]), (15, [714.0, 1014.0]), (11, [578.0, 664.0])] {
        let p = shown.points[i];
        assert!((p.x - want[0]).abs() <= 4.0 && (p.y - want[1]).abs() <= 4.0, "point {i}: {p:?}, the reference drew it at {want:?}");
    }
}

/// A preset style picked in the Warp options bar is the preset the engine and a PSD make (`Warp::preset`): Arc is an
/// envelope. The bar built it without, so the same Arc rendered differently from the command's and was saved so.
#[test]
fn a_style_picked_in_the_options_bar_is_the_presets_own() {
    let mut app = warping_square();
    super::options_bar::warp_edit(&mut app, craft::imaging::ui::warp::WarpFieldsEdit::Style("warpArc".into()));
    let w = &session_warp(&app).warp;
    assert_eq!(w.style, photocraft_geom::warp::WarpStyle::Arc, "{w:?}");
    let want = photocraft_geom::warp::Warp::preset(w.style, w.bend, w.bounds);
    assert_eq!(w.envelope, want.envelope, "the bar's Arc is the engine's Arc");
    assert!(w.envelope);
}

/// Free Transform on a turned smart object, Warp, and Enter without a pull: nothing changes and no step is added (the
/// flat grid was dropped after the "box unmoved" check, and an identity transform ran as an edit).
#[test]
fn enter_on_an_untouched_warp_of_a_turned_smart_object_adds_no_step() {
    let (mut app, ctx, id) = turned_smart_object(0.5);
    let before = placement_of(&app, id);
    begin(&mut app, &ctx).unwrap();
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    let steps = app.session.active().unwrap().history.past_len();
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "no step");
    assert_eq!(placement_of(&app, id), before);
}

/// Transform Selection, Warp, a pull, then a box mode (the warp held, the preview showing it): Enter applies the held
/// warp to the selection. It was taken and dropped, and the selection stayed as it was.
#[test]
fn a_warp_held_on_a_selection_is_applied() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 32, "height": 32})).unwrap();
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "select.transformSelection", json!({})).unwrap();
    assert!(open(&app).selection);
    begin_warp(&mut app, &ctx).unwrap();
    settle(&mut app);
    assert!(open(&app).selection, "still the selection's session");
    let mut to = warp_mesh(&app);
    // The bottom edge down by 12 px.
    for i in [12usize, 13, 14, 15] {
        to.points[i] += Vec2::new(0.0, 12.0);
    }
    pull_mesh(&mut app, 2, to);
    settle(&mut app);
    hold_warp(&mut app);
    settle(&mut app);
    let mask = |app: &PhotocraftApp| app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    let before = mask(&app);
    commit(&mut app);
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let after = mask(&app).expect("a selection");
    assert!(after.y1 > before.expect("a selection").y1, "the pull moved the outline: {before:?} -> {after:?}");
}

/// Warp, a pull, a box mode (the warp held), Undo back into Warp, and Enter before any pass draws: the hold is settled
/// first, so the warp is stored as it is after an ordinary pass (the stale hold was committed as a document warp).
#[test]
fn enter_right_after_an_undo_back_into_warp_commits_what_a_pass_would() {
    fn run(redraw: bool) -> (Option<photocraft_geom::warp::Warp>, [f64; 6]) {
        let (mut app, ctx, id) = turned_smart_object(0.5);
        begin(&mut app, &ctx).unwrap();
        begin_warp(&mut app, &ctx).unwrap();
        settle(&mut app);
        let mut to = warp_mesh(&app);
        to.points[5] += Vec2::new(4.0, 3.0);
        pull_mesh(&mut app, 2, to);
        settle(&mut app);
        hold_warp(&mut app);
        settle(&mut app);
        let _ = intercept(&mut app, "edit.undo");
        if redraw {
            super::begin::reconcile_held(&mut app);
        }
        commit(&mut app);
        land_jobs(&mut app);
        assert!(app.ui.transform.is_none(), "{}", app.ui.status);
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a smart object") };
        (sm.warp.clone(), placement_of(&app, id))
    }
    let (now, then) = (run(false), run(true));
    assert_eq!(now.0, then.0, "the warp");
    for (a, b) in now.1.iter().zip(then.1) {
        assert!((a - b).abs() < 1e-9, "{:?} vs {:?}", now.1, then.1);
    }
}

/// #15548: Free Transform on an unturned, unskewed box shows the horizontal skew as "0", not "-0".
#[test]
fn an_unskewed_box_shows_its_horizontal_skew_as_zero() {
    use egui_kittest::kittest::Queryable as _;
    let mut h = harness(app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24)), egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.get_by_label("Horizontal skew").value().as_deref(), Some("0"));
}
