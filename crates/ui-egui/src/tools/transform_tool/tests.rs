use super::*;
use craft::behavior::geometry::quad::BoxState;
use craft::behavior::kurbo::{Point, Rect, Vec2};
use craft::behavior::{EditEvent, EditEvents, GestureId, Mixed};
use craft::draw::ui::transform::{TransformChoice, TransformEdit, TransformMenuWords};
use craft::i18n::WordSet;
use craft::imaging::trace::{MaskSource, outline};
use craft::imaging::ui::warp::{CylinderValues, WarpMesh, WarpSplit};
use photocraft_engine::color_cmds::ColorExt;

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
    assert!(split_m.us.iter().any(|u| (u - 1.0 / 3.0).abs() < 1e-6) && split_m.us.iter().any(|u| (u - 0.5).abs() < 1e-6));
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
        app.session.color_mut().settings.monitor_profile = monitor.into();
        let st = app.session.active().unwrap();
        let display = app.session.color().canvas_display(&st.doc).unwrap();
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
    app.session.color_mut().settings.monitor_profile = "display-p3".into();
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
    app.session.color_mut().settings.monitor_profile = "display-p3".into();
    let doc = app.session.active().unwrap().doc.clone();
    let display = app.session.color().canvas_display(&doc).unwrap();
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

/// #15548: Free Transform on an unturned, unskewed box shows the horizontal skew as "0", not "-0". The field's value
/// reads the unit it draws after the number.
#[test]
fn an_unskewed_box_shows_its_horizontal_skew_as_zero() {
    use egui_kittest::kittest::Queryable as _;
    let mut h = harness(app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24)), egui::vec2(1400.0, 900.0));
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.get_by_label("Horizontal skew").value().as_deref(), Some("0 °"));
}

#[path = "warp_remove_split_tests.rs"]
mod warp_remove_split_tests;
