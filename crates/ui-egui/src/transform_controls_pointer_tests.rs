//! Move tool transform controls through the control channel (#11105), and Free Transform's snapping.

use craft::ui::dialog::DialogId;
use egui::Modifiers;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::testing::{key, move_to, press, release};
use crate::state::Tool;

/// Sends a control request and returns its reply, through the canvas driver's control channel (`testing::control`).
pub(crate) fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    crate::canvas::testing::control(app, method, params)
}

/// `call` that asserts the reply is ok.
pub(crate) fn accepted(app: &mut PhotocraftApp, method: &str, params: Value) {
    let reply = call(app, method, params);
    assert_eq!(reply["ok"], true, "{method}: {reply}");
}

/// Two selected shapes share the measured controls frame (100,100)..(800,500).
fn fixture() -> (PhotocraftApp, egui::Context, [LayerId; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 1000, "height": 700, "background": "white"})).unwrap();
    let mut layers = Vec::new();
    for rect in [[100, 100, 200, 100], [600, 400, 200, 100]] {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "fill": "#ff0000"})).unwrap();
        layers.push(app.session.active().unwrap().active_layer.unwrap());
    }
    app.run("layer.select", json!({"layer": layers[0].0, "mode": "add"})).unwrap();
    app.sync_views();
    crate::canvas::testing::window(&mut app, egui::vec2(1400.0, 900.0));
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let ctx = crate::canvas::testing::ctx(&app);
    accepted(&mut app, "ui.set", json!({"tool": "Move", "moveShowTransform": true, "zoom": 1.0, "center": [500, 350]}));
    (app, ctx, [layers[0], layers[1]])
}

/// Every layer's bounds are its `old` bounds moved by the drag of 20 and 10 pixels.
fn assert_translated(app: &PhotocraftApp, layers: [LayerId; 2], old: [Rect; 2], what: &str) {
    for (id, previous) in layers.into_iter().zip(old) {
        assert_eq!(bounds(app, id), previous.translate(20, 10), "{what}");
    }
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let state = app.session.active().unwrap();
    photocraft_engine::transform_cmds::transform_bounds(&state.doc, state.doc.layer(id).unwrap())
}

fn pointer(app: &mut PhotocraftApp, _ctx: &egui::Context, kind: &str, at: [f64; 2]) {
    accepted(app, "ui.pointer", json!({"events": [{"kind": kind, "x": at[0], "y": at[1]}]}));
}

fn enter(app: &mut PhotocraftApp) {
    key(app, egui::Key::Enter, Modifiers::NONE);
}

/// The open session's box corners.
fn quad(app: &PhotocraftApp) -> [[f64; 2]; 4] {
    app.ui.transform.as_ref().expect("a Free Transform session is open").session.quad().map(|p| [p.x, p.y])
}

/// The open session's reference point.
fn pivot(app: &PhotocraftApp) -> [f64; 2] {
    let p = app.ui.transform.as_ref().expect("a Free Transform session is open").session.pivot();
    [p.x, p.y]
}

fn dialog_count(app: &PhotocraftApp) -> usize {
    app.framework.dialogs.stack().ids().count()
}

/// The Color Picker's colour field, from the hosted dialog's `json_fields` as the host lists it.
fn color_field(app: &PhotocraftApp, id: DialogId) -> Value {
    let open = app.framework.dialogs.open_dialogs().into_iter().find(|dialog| dialog.id == id).expect("the Color Picker is open");
    open.fields.expect("the Color Picker lists its fields")["color"].clone()
}

fn close_dialog(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) {
    assert!(app.framework.dialogs.cancel_by_id(ctx, id), "the dialog was open");
}

fn assert_one_transform(app: &PhotocraftApp, steps: usize) {
    assert!(app.ui.transform.is_none(), "Enter closes Free Transform");
    let state = app.session.active().unwrap();
    assert_eq!(state.history.past_len(), steps + 1);
    assert_eq!(state.history.entries().last().map(String::as_str), Some("Free Transform"));
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Opens {
    /// A Free Transform session that scales both layers.
    Scale,
    /// A Free Transform session that rotates both layers.
    Rotate,
    /// An ordinary Move drag.
    Move,
    /// Nothing.
    Nothing,
}

/// The bounds each of `old` takes under the homography that maps the open session's rectangle onto `preview`; both
/// committed layer bounds follow it, so a translation or a one-layer rotation fails.
fn bounds_under_preview(app: &PhotocraftApp, old: [Rect; 2], preview: [[f64; 2]; 4]) -> [[f64; 4]; 2] {
    let r = app.ui.transform.as_ref().unwrap().session.rect();
    let homography = photocraft_algo::transform::Homography::rect_to_quad([r.x0, r.y0, r.x1, r.y1], preview).unwrap();
    old.map(|r| {
        [[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]]
            .map(|[x, y]| homography.apply(f64::from(x), f64::from(y)))
            .into_iter()
            .fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, (x, y)| [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)])
    })
}

/// Where a press lands decides the gesture: a corner, the rotation band at 100 %, the visible corner at 200 %, a handle
/// off the canvas and the interior.
#[test]
fn move_controls_press_zones_start_the_matching_gesture() {
    for (zoom, center, down, to, expect) in [
        (1.0, [500, 350], [800.0, 500.0], [700.0, 440.0], Opens::Scale),
        // Fourteen points beyond the corner: outside its 12pt handle, inside the rotation band.
        (1.0, [500, 350], [814.0, 500.0], [800.0, 514.0], Opens::Rotate),
        (2.0, [500, 350], [800.0, 500.0], [700.0, 440.0], Opens::Scale),
        (3.0, [500, 350], [800.0, 500.0], [700.0, 440.0], Opens::Nothing),
        (1.0, [0, 0], [800.0, 500.0], [700.0, 440.0], Opens::Nothing),
        (1.0, [500, 350], [200.0, 150.0], [220.0, 160.0], Opens::Move),
    ] {
        let (mut app, ctx, layers) = fixture();
        accepted(&mut app, "ui.set", json!({"zoom": zoom, "center": center}));
        let before = app.session.active().unwrap().doc.clone();
        let old = layers.map(|id| bounds(&app, id));
        let steps = app.session.active().unwrap().history.past_len();
        let what = format!("zoom {zoom}, center {center:?}, press {down:?}");
        pointer(&mut app, &ctx, "down", down);
        match expect {
            Opens::Nothing => {
                assert!(app.ui.transform.is_none(), "{what}: a hidden handle cannot be pressed");
                pointer(&mut app, &ctx, "move", to);
                pointer(&mut app, &ctx, "up", to);
                assert!(app.ui.transform.is_none(), "{what}");
                assert_eq!(app.session.active().unwrap().doc, before, "{what}");
                assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{what}");
                continue;
            }
            Opens::Move => {
                assert!(app.ui.transform.is_none(), "{what}");
                pointer(&mut app, &ctx, "move", to);
                pointer(&mut app, &ctx, "up", to);
                assert_translated(&app, layers, old, &what);
                assert!(app.ui.transform.is_none(), "{what}");
                assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "{what}");
                continue;
            }
            Opens::Scale | Opens::Rotate => {}
        }
        // The framework's transform box begins its edit when the drag starts, not at the press.
        let halfway = [(down[0] + to[0]) / 2.0, (down[1] + to[1]) / 2.0];
        pointer(&mut app, &ctx, "move", halfway);
        let transform = app.ui.transform.as_ref().unwrap_or_else(|| panic!("{what}: Move controls open Free Transform"));
        let r = transform.session.rect();
        assert_eq!([r.x0, r.y0, r.x1, r.y1], [100.0, 100.0, 800.0, 500.0], "{what}");
        assert_eq!(transform.layers, layers.map(|id| id.0), "{what}");
        let initial = quad(&app);
        pointer(&mut app, &ctx, "move", to);
        pointer(&mut app, &ctx, "up", to);
        assert!(app.ui.transform.is_some(), "{what}: Up ends the gesture, not the transform session");
        assert_eq!(app.session.active().unwrap().doc, before, "{what}: the preview has not changed the document");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{what}");
        let preview = quad(&app);
        assert_ne!(preview, initial, "{what}");
        let expected = if expect == Opens::Rotate {
            assert!((preview[1][1] - preview[0][1]).abs() > 1.0, "{what}: the upper edge rotates");
            Some(bounds_under_preview(&app, old, preview))
        } else {
            None
        };
        enter(&mut app);
        assert_one_transform(&app, steps);
        let after = app.session.active().unwrap().doc.clone();
        assert_ne!(after, before, "{what}: Enter commits the transform");
        for (index, (id, previous)) in layers.into_iter().zip(old).enumerate() {
            let now = bounds(&app, id);
            match &expected {
                None => assert!(now.width() < previous.width() && now.height() < previous.height(), "{what}: both layers scale: {previous:?} -> {now:?}"),
                Some(expected) => {
                    let expected = expected[index];
                    assert!(
                        (expected[2] - expected[0] - f64::from(previous.width())).abs() > 2.0,
                        "{what}: this fixture distinguishes rotation from translation"
                    );
                    // The preview frame is continuous; committed raster bounds round to whole pixels.
                    for (actual, expected) in [now.x0, now.y0, now.x1, now.y1].into_iter().zip(expected) {
                        assert!(
                            (f64::from(actual) - expected).abs() <= 1.0,
                            "{what}: layer {id:?}: committed bound {actual} does not match preview {expected}"
                        );
                    }
                }
            }
        }
        assert!(app.session.undo());
        assert_eq!(app.session.active().unwrap().doc, before, "{what}: one Undo restores both layers");
        assert!(app.session.redo());
        assert_eq!(app.session.active().unwrap().doc, after, "{what}: one Redo restores the transform");
    }
}

/// Visibility and setting gates must reject entry even when the point geometrically hits a handle.
#[test]
fn disabled_hidden_and_missed_controls_do_not_open_a_transform() {
    for (enabled, extras, at) in [(false, true, [800.0, 500.0]), (true, false, [800.0, 500.0]), (true, true, [900.0, 600.0])] {
        let (mut app, ctx, _) = fixture();
        accepted(&mut app, "ui.set", json!({"moveShowTransform": enabled}));
        app.ui.view.extras = extras;
        pointer(&mut app, &ctx, "down", at);
        assert!(app.ui.transform.is_none(), "enabled={enabled}, Extras={extras}, at={at:?}");
        pointer(&mut app, &ctx, "up", at);
        assert!(app.ui.transform.is_none());
    }
}

/// Secondary buttons and Space-pan retain their established input ownership.
#[test]
fn nonprimary_and_space_presses_do_not_take_over_move_controls() {
    for params in [json!({"button": "right"}), json!({"button": "middle"}), json!({"space": true})] {
        let (mut app, _, _) = fixture();
        let mut params = params;
        params["events"] = json!([{"kind": "down", "x": 800, "y": 500}]);
        accepted(&mut app, "ui.pointer", params);
        assert!(app.ui.transform.is_none());
    }
}

/// Disabling Move controls must not disable the explicit Free Transform command.
#[test]
fn explicit_free_transform_still_works_with_move_controls_disabled() {
    let (mut app, ctx, _) = fixture();
    accepted(&mut app, "ui.set", json!({"moveShowTransform": false}));
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [800.0, 500.0]);
    pointer(&mut app, &ctx, "move", [700.0, 440.0]);
    pointer(&mut app, &ctx, "up", [700.0, 440.0]);
    enter(&mut app);
    assert_one_transform(&app, steps);
}

/// A malformed boolean must reject the complete patch before a valid sibling field changes.
#[test]
fn invalid_move_show_transform_is_atomic_with_other_ui_fields() {
    for invalid in [json!("true"), json!(1), json!(null), json!([]), json!({})] {
        let (mut app, _, _) = fixture();
        accepted(&mut app, "ui.set", json!({"moveShowTransform": false}));
        let before = serde_json::to_value(&app.ui).unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        let reply = call(&mut app, "ui.set", json!({"tool": "Brush", "zoom": 2.0, "moveShowTransform": invalid}));
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap().contains("moveShowTransform"), "{reply}");
        assert_eq!(serde_json::to_value(&app.ui).unwrap(), before, "no UI field changed");
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }
}

/// Omission retains the setting, and later validation failure cannot partially apply a new value.
#[test]
fn move_show_transform_is_retained_when_omitted_and_waits_for_all_validation() {
    let (mut app, _, _) = fixture();
    accepted(&mut app, "ui.set", json!({"moveShowTransform": false}));
    let reply = call(&mut app, "ui.set", json!({"moveShowTransform": true, "zoom": "bad"}));
    assert_eq!(reply["ok"], false);
    assert!(!app.ui.tool_options.move_show_transform);
    accepted(&mut app, "ui.set", json!({"moveShowTransform": true}));
    accepted(&mut app, "ui.set", json!({"tool": "Brush"}));
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(app.ui.tool_options.move_show_transform);
}

/// A malformed later sample must be rejected before the first valid Down can open a session.
#[test]
fn invalid_later_pointer_event_does_not_begin_a_transform() {
    let (mut app, _, _) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let reply = call(
        &mut app,
        "ui.pointer",
        json!({"events": [
            {"kind": "down", "x": 800, "y": 500},
            {"kind": "unknown", "x": 700, "y": 440}
        ]}),
    );
    assert_eq!(reply["ok"], false);
    assert!(app.ui.transform.is_none());
    assert!(!crate::canvas::testing::in_gesture(&app));
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// Full-window dialogs with embedded previews must not forward ordinary Move gestures.
#[test]
fn modal_workspaces_without_tool_event_controllers_do_not_move_layers() {
    for command in ["filter.filterGallery", "filter.adaptiveWideAngle"] {
        let (mut app, ctx, layers) = fixture();
        app.run("layer.rasterize.layer", json!({"layer": layers[0].0})).unwrap();
        app.run("layer.select", json!({"layer": layers[0].0})).unwrap();
        crate::menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        assert_eq!(crate::distort_ui::modal_workspace(&app), Some(command));
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        for controls in [true, false] {
            app.ui.tool_options.move_show_transform = controls;
            for (kind, at) in [("down", [200.0, 150.0]), ("move", [220.0, 160.0]), ("up", [220.0, 160.0])] {
                pointer(&mut app, &ctx, kind, at);
                assert!(app.ui.transform.is_none());
                assert!(!crate::canvas::testing::in_gesture(&app));
            }
            assert_eq!(app.session.active().unwrap().doc, before);
            assert_eq!(app.session.active().unwrap().history.past_len(), steps);
            assert_eq!(app.ui.tool, Tool::Move);
            assert_eq!(crate::distort_ui::modal_workspace(&app), Some(command));
        }
    }
}

/// Modal canvas samplers retain ownership even when Move and its controls are selected.
#[test]
fn move_pointer_still_samples_into_the_color_picker() {
    let (mut app, ctx, _) = fixture();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let _ = crate::color_picker_ui::open(&mut app, "foreground");
    let id = crate::color_picker_ui::top(&app).expect("the Color Picker is open");
    for (kind, at) in [("down", [200.0, 150.0]), ("move", [220.0, 160.0]), ("up", [220.0, 160.0])] {
        pointer(&mut app, &ctx, kind, at);
    }
    assert_eq!(color_field(&app, id), "#ff0000");
    assert!(app.ui.transform.is_none());
    assert!(!crate::canvas::testing::in_gesture(&app));
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// A full-window editor receives its own stroke instead of a Move or Free Transform gesture.
#[test]
fn move_pointer_still_reaches_the_liquify_workspace() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 120, "height": 80, "depth": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.sync_views();
    let ctx = crate::canvas::testing::ctx(&app);
    accepted(&mut app, "ui.set", json!({"tool": "Move", "moveShowTransform": true}));
    crate::menus::invoke(&mut app, &ctx, "filter.liquify", json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    for (kind, at) in [("down", [50.0, 40.0]), ("move", [60.0, 40.0]), ("up", [60.0, 40.0])] {
        pointer(&mut app, &ctx, kind, at);
    }
    let fields = crate::liquify_ui::describe(&app).expect("the Liquify workspace is open");
    assert_eq!(fields["strokes"].as_u64(), Some(1));
    assert!(app.ui.transform.is_none());
    assert!(!crate::canvas::testing::in_gesture(&app));
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
}

/// ⌘ makes another tool the Move tool for the press: a modal blocks that layer drag too.
#[test]
fn modal_dialogs_block_a_command_held_move_from_another_tool() {
    let (mut app, ctx, _) = fixture();
    accepted(&mut app, "ui.set", json!({"tool": "Brush"}));
    crate::menus::invoke(&mut app, &ctx, "file.new", json!({})).unwrap();
    assert_eq!(dialog_count(&app), 1);
    let before = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    for (kind, at) in [("down", [200.0, 150.0]), ("move", [220.0, 160.0]), ("up", [220.0, 160.0])] {
        accepted(&mut app, "ui.pointer", json!({"command": true, "events": [{"kind": kind, "x": at[0], "y": at[1]}]}));
        assert!(!crate::canvas::testing::in_gesture(&app), "{kind}: the modal owns the ⌘ Move press");
    }
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert_eq!(app.ui.tool, Tool::Brush);
}

/// The number of Smart Guide lines the tool host reported in its last pass.
fn guide_lines(app: &PhotocraftApp) -> usize {
    app.canvas_tools.last_response.as_ref().map_or(0, |response| response.snapped.len())
}

/// A Move drag that runs when the Color Picker opens keeps the Move tool to its release, which ends the drag where the
/// pointer is and commits it once; the picker's pick takes the canvas only after that, and moving after the picker
/// closes needs a new press.
#[test]
fn color_picker_release_ends_an_ordinary_move_drag() {
    let (mut app, ctx, layers) = fixture();
    let old = layers.map(|id| bounds(&app, id));
    let steps = app.session.active().unwrap().history.past_len();
    pointer(&mut app, &ctx, "down", [200.0, 150.0]);
    pointer(&mut app, &ctx, "move", [220.0, 160.0]);
    assert!(crate::canvas::testing::in_gesture(&app));
    accepted(&mut app, "ui.dialog.open", json!({"kind": "colorPicker"}));
    let dialog = crate::color_picker_ui::top(&app).expect("the Color Picker is open");
    let color = color_field(&app, dialog);
    pointer(&mut app, &ctx, "up", [240.0, 170.0]);
    assert!(!crate::canvas::testing::in_gesture(&app), "the release ends the drag");
    assert_eq!(color_field(&app, dialog), color, "the drag's release samples nothing");
    close_dialog(&mut app, &ctx, dialog);
    pointer(&mut app, &ctx, "move", [270.0, 190.0]);
    pointer(&mut app, &ctx, "up", [270.0, 190.0]);
    for (id, previous) in layers.into_iter().zip(old) {
        assert_eq!(bounds(&app, id), previous.translate(40, 20), "moved to the release, and no further after the picker closed");
    }
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

// Snapping of the open box (View › Snap To, Smart Guides), driven through the canvas as window input.

/// A 400 × 300 document with a target block at (300..350, 200..260) and the moving block at (40..90, 40..80), at
/// 100 %, with Snap and Smart Guides on.
fn snap_box() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = 1.0;
    app.ui.views[0].clear_pending();
    app.run("layer.new.layer", json!({"name": "target"})).unwrap();
    app.run("select.rect", json!({"x": 300, "y": 200, "width": 50, "height": 60})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("layer.new.layer", json!({"name": "mover"})).unwrap();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.extras.snap = true;
    app.ui.view.show.smart_guides = true;
    app
}

fn mover_bounds(app: &PhotocraftApp) -> Rect {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds()
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

fn assert_quad(app: &PhotocraftApp, expected: [[f64; 2]; 4], what: &str) {
    let q = quad(app);
    assert!(q.iter().zip(&expected).all(|(a, b)| near(*a, *b)), "{what}: {q:?} != {expected:?}");
}

/// The app of `snap_box` with guides at x 110 and y 30 and the box of `menu` open.
fn corner_guides(menu: &str) -> PhotocraftApp {
    let mut app = snap_box();
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 110})).unwrap();
    app.run("view.newGuide", json!({"orientation": "horizontal", "position": 30})).unwrap();
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &app_ctx, menu, json!({})).unwrap();
    app
}

/// Presses corner `index` off by `offset` with `down` held, then drags so the proposed corner is `end` with `moving`
/// held; the pointer keeps its grab offset.
fn move_corner(app: &mut PhotocraftApp, index: usize, offset: [f64; 2], end: [f64; 2], down: Modifiers, moving: Modifiers) {
    let corner = quad(app)[index];
    press(app, [corner[0] + offset[0], corner[1] + offset[1]], down);
    move_to(app, [end[0] + offset[0], end[1] + offset[1]], moving);
}

fn free_transform(app: &mut PhotocraftApp, reference_point: bool) {
    app.ui.tool_options.transform.show_reference_point = reference_point;
    let app_ctx = app.ctx().clone();
    crate::menus::invoke(app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
}

/// Press at `from`, drag to `to`, release; returns the reference point.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) -> [f64; 2] {
    press(app, from, mods);
    move_to(app, to, mods);
    release(app, to, mods);
    pivot(app)
}

#[test]
fn distort_corner_snaps_from_the_grab_offset_either_way() {
    for offset in [[4.0, 3.0], [-4.0, -3.0]] {
        let mut app = corner_guides("edit.transform.distort");
        let before = quad(&app);
        // The proposed corner is (108, 32); the pointer keeps its original grab offset.
        move_corner(&mut app, 1, offset, [108.0, 32.0], Modifiers::NONE, Modifiers::NONE);
        let moved = quad(&app);
        assert!(near(moved[1], [110.0, 30.0]), "grab offset {offset:?}: {moved:?}");
        assert_quad(&app, [before[0], moved[1], before[2], before[3]], "the other corners stay");
        assert_eq!(guide_lines(&app), 2, "a line per guide, grab offset {offset:?}");
        release(&mut app, [108.0 + offset[0], 32.0 + offset[1]], Modifiers::NONE);
        assert_quad(&app, moved, "the release keeps the snapped corner");
        assert_eq!(guide_lines(&app), 0, "the lines end with the gesture");
    }
}

#[test]
fn free_transform_command_corner_snaps_with_the_real_grab_offset() {
    let windows = Modifiers { command: true, ctrl: true, ..Modifiers::NONE };
    let mac = Modifiers { command: true, mac_cmd: true, ..Modifiers::NONE };
    for m in [windows, mac] {
        for offset in [[4.0, 3.0], [-4.0, -3.0], [0.0, 0.0]] {
            for down in [m, Modifiers::NONE] {
                let mut app = corner_guides("edit.freeTransform");
                let before = quad(&app);
                move_corner(&mut app, 1, offset, [108.0, 32.0], down, m);
                let what = format!("{m:?}, press {down:?}, offset {offset:?}");
                assert_quad(&app, [before[0], [110.0, 30.0], before[2], before[3]], &what);
                assert_eq!(guide_lines(&app), 2, "{what}");
            }
        }
    }
}

#[test]
fn skew_control_corner_on_editable_type_slides_the_whole_edge() {
    let ctrl = Modifiers { command: true, ctrl: true, ..Modifiers::NONE };
    for legacy in [false, true] {
        let mut app = snap_box();
        app.run("type.create", json!({"x": 40, "y": 60, "text": "Hello"})).unwrap();
        app.run("prefs.set", json!({"path": "general.useLegacyFreeTransform", "value": legacy})).unwrap();
        let app_ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &app_ctx, "edit.transform.skew", json!({})).unwrap();
        let before = quad(&app);
        let end = [before[1][0] + 18.0, before[1][1] - 8.0];
        let doc = app.session.active().unwrap().doc.clone();
        let past = app.session.active().unwrap().history.past_len();
        move_corner(&mut app, 1, [2.0, 1.0], end, Modifiers::NONE, ctrl);
        let mut expected = before;
        expected[0][0] += 18.0;
        expected[1][0] += 18.0;
        assert_quad(&app, expected, &format!("type remains affine, legacy {legacy}"));
        assert_eq!(guide_lines(&app), 0);
        release(&mut app, [end[0] + 2.0, end[1] + 1.0], ctrl);
        assert_quad(&app, expected, "release preserves the type's edge");
        crate::transform_tool::cancel(&mut app);
        assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
        assert_eq!(app.session.active().unwrap().history.past_len(), past);
    }
}

#[test]
fn a_snapped_distort_commits_one_undoable_transform() {
    let mut app = corner_guides("edit.transform.distort");
    let before_doc = app.session.active().unwrap().doc.clone();
    let before_bounds = mover_bounds(&app);
    let past = app.session.active().unwrap().history.past_len();
    move_corner(&mut app, 1, [4.0, 3.0], [108.0, 32.0], Modifiers::NONE, Modifiers::NONE);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &before_doc), "preview does not edit the document");
    release(&mut app, [112.0, 35.0], Modifiers::NONE);
    crate::transform_tool::commit(&mut app);
    assert_eq!(app.ui.status, "");
    let after_bounds = mover_bounds(&app);
    assert_ne!(after_bounds, before_bounds);
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(mover_bounds(&app), before_bounds);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(mover_bounds(&app), after_bounds);
}

/// #5751: a Rectangle drawn on a 1080 px canvas, then resized with Free Transform under the
/// Move tool. With Snap Vector Tools and Transforms to Pixel Grid on (the default) the box's
/// edges stay on whole pixels: the grab offset and a proportional scale used to leave them at
/// 20.6 and 1040.4. Off, the box follows the pointer exactly.
#[test]
fn free_transform_of_a_shape_keeps_whole_pixels() {
    for grid in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 1080, "height": 1080, "resolution": 72})).unwrap();
        app.run("prefs.set", json!({"path": "tools.snapVectorToolsAndTransformsToPixelGrid", "value": grid})).unwrap();
        app.sync_views();
        // The whole document shows at 60 %, so no press or drag below reaches past the canvas edge and autoscrolls.
        crate::canvas::testing::window(&mut app, egui::vec2(1400.0, 900.0));
        app.ui.views[0].zoom = 0.6;
        app.ui.views[0].center = [540.0, 540.0];
        app.ui.views[0].clear_pending();
        app.ui.extras.snap = true;
        app.ui.view.show.smart_guides = true;
        app.ui.tool = Tool::Rectangle;
        let m = Modifiers::NONE;
        press(&mut app, [40.0, 31.0], m);
        move_to(&mut app, [300.0, 700.0], m);
        release(&mut app, [481.0, 1000.0], m);
        app.ui.tool = Tool::Move;
        let app_ctx = app.ctx().clone();
        crate::menus::invoke(&mut app, &app_ctx, "edit.freeTransform", json!({})).unwrap();
        let whole = |q: [[f64; 2]; 4]| q.iter().flatten().all(|v| v.fract() == 0.0);
        // The left edge handle, grabbed 1.3 px inside, dropped away from any snap target.
        let q = quad(&app);
        let mid_y = (q[0][1] + q[3][1]) / 2.0;
        press(&mut app, [q[0][0] + 1.3, mid_y], m);
        move_to(&mut app, [60.0, mid_y], m);
        release(&mut app, [80.6, mid_y], m);
        let q = quad(&app);
        assert_eq!(whole(q), grid, "grid {grid}: left edge {q:?}");
        // The bottom-right corner scaled proportionally (the default), off any snap target.
        press(&mut app, q[2], m);
        move_to(&mut app, [450.4, 900.1], m);
        release(&mut app, [450.4, 900.1], m);
        let q = quad(&app);
        assert_eq!(whole(q), grid, "grid {grid}: corner {q:?}");
        crate::transform_tool::commit(&mut app);
        let st = app.session.active().unwrap();
        let photocraft_doc::LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("a shape layer") };
        let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
        if grid {
            for v in [x0, y0, x1, y1] {
                assert!((v - v.round()).abs() < 1e-6, "committed shape {:?}", (x0, y0, x1, y1));
            }
        }
    }
}

/// #5153: a reference point drag (or an Alt-drag inside the box) moves the point alone. It used to
/// snap as a box move: the point stuck and snap lines showed at edges the box never reached.
#[test]
fn free_transform_reference_point_drag_snaps_the_point_not_the_box() {
    let mut app = snap_box();
    free_transform(&mut app, true);
    let quad0 = quad(&app);
    assert_eq!(pivot(&app), [65.0, 60.0]);
    let alt = Modifiers::ALT;
    // Moving the box's right edge by the same amount would land it 2 px from the target's left
    // edge (x = 300), but the point alone moves, so nothing pulls it there.
    press(&mut app, [65.0, 60.0], alt);
    move_to(&mut app, [273.0, 60.0], alt);
    assert_eq!(guide_lines(&app), 0);
    release(&mut app, [273.0, 60.0], alt);
    assert!(near(pivot(&app), [273.0, 60.0]), "{:?}", pivot(&app));
    assert_quad(&app, quad0, "the box stays put");
    // Alt-dragging from inside the box places the reference point the same way.
    assert!(near(drag(&mut app, [50.0, 50.0], [258.0, 50.0], alt), [258.0, 50.0]));
    // A plain drag of the reference point moves it too.
    assert!(near(drag(&mut app, [258.0, 50.0], [80.0, 63.0], Modifiers::NONE), [80.0, 63.0]));
    assert_quad(&app, quad0, "the box stays put");
}

/// A box dragged from inside doesn't snap back to the selection it transforms (measured in the
/// reference application: a 6 px drag moves it 6 px).
#[test]
fn free_transform_of_a_selection_never_snaps_back_to_it() {
    let mut app = snap_box();
    app.run("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
    free_transform(&mut app, false);
    let quad0 = quad(&app);
    let m = Modifiers::NONE;
    press(&mut app, [50.0, 50.0], m);
    move_to(&mut app, [53.0, 50.0], m);
    assert_eq!(guide_lines(&app), 0);
    release(&mut app, [53.0, 50.0], m);
    let q = quad(&app);
    assert!((q[0][0] - (quad0[0][0] + 3.0)).abs() < 1e-3, "{q:?}");
}
