use craft::draw::ui::shape::VectorMode;
use craft::ui::dialog::{DialogId, StackDialog as _};
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::{Value, json};

use super::*;
use crate::canvas::testing;
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app
}

/// A click with `tool` at `at`, through the canvas pass.
fn click(app: &mut PhotocraftApp, tool: Tool, at: [f64; 2]) {
    app.ui.tool = tool;
    testing::click(app, at, egui::Modifiers::NONE);
}

/// A drag with `tool` from `start` to `end`, through the canvas pass.
fn drag(app: &mut PhotocraftApp, tool: Tool, start: [f64; 2], end: [f64; 2]) {
    app.ui.tool = tool;
    testing::press(app, start, egui::Modifiers::NONE);
    testing::move_to(app, end, egui::Modifiers::NONE);
    testing::release(app, end, egui::Modifiers::NONE);
}

fn layers(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn live(app: &PhotocraftApp) -> LiveShape {
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let LayerContent::Shape(sh) = &l.content else { panic!("not a shape layer") };
    sh.live.clone().unwrap()
}

/// Writes fields of the hosted dialog `id`, as `ui.dialog.set` does.
fn set(app: &mut PhotocraftApp, id: DialogId, fields: Value) {
    let ctx = app.ctx().clone();
    for (k, v) in fields.as_object().unwrap() {
        app.framework.dialogs.set_json_field(&ctx, id, k, v.clone()).unwrap_or_else(|e| panic!("{k}: {e}"));
    }
}

fn confirm_by_id(app: &mut PhotocraftApp, id: DialogId) {
    crate::dialogs::testing::confirm(app, id).unwrap();
}

fn only_dialog(app: &PhotocraftApp) -> DialogId {
    assert_eq!(app.framework.dialogs.stack().ids().count(), 1, "a click opens one dialog");
    app.framework.dialogs.get::<CreateShapeDialog>().expect("it is a Create Shape dialog").id()
}

#[test]
fn a_rectangle_click_opens_create_rectangle_and_ok_draws_it_at_the_click() {
    let mut app = app();
    let before = layers(&app);
    click(&mut app, Tool::Rectangle, [20.4, 30.2]);
    let id = only_dialog(&app);
    assert_eq!(layers(&app), before, "the click itself draws nothing");
    set(&mut app, id, json!({"width": 120, "height": 45.5, "radii": [4, 4, 4, 4]}));
    confirm_by_id(&mut app, id);
    assert!(app.framework.dialogs.is_empty());
    assert_eq!(layers(&app), before + 1);
    assert_eq!(live(&app), LiveShape::Rect { rect: [20.0, 30.0, 120.0, 45.5], radii: [4.0; 4] });
    // The options bar's fill (foreground colour) is used, as for a drag.
    let st = app.session.active().unwrap();
    let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!() };
    assert!(sh.fill.is_some());
}

#[test]
fn from_center_and_the_other_tools() {
    let mut app = app();
    click(&mut app, Tool::EllipseShape, [200.0, 150.0]);
    let id = only_dialog(&app);
    set(&mut app, id, json!({"width": 80, "height": 40, "fromCenter": true}));
    confirm_by_id(&mut app, id);
    assert_eq!(live(&app), LiveShape::Ellipse { rect: [160.0, 130.0, 80.0, 40.0] });

    click(&mut app, Tool::Polygon, [10.0, 10.0]);
    let id = only_dialog(&app);
    set(&mut app, id, json!({"width": 50, "height": 60, "sides": 7, "starRatio": 50}));
    confirm_by_id(&mut app, id);
    assert_eq!(live(&app), LiveShape::Polygon { rect: [10.0, 10.0, 50.0, 60.0], sides: 7, star_ratio: 0.5, corner_radius: 0.0 });

    // Away from the polygon just drawn: its scale box, still selected, takes a press at its handles.
    click(&mut app, Tool::Triangle, [305.0, 206.0]);
    let id = only_dialog(&app);
    confirm_by_id(&mut app, id);
    assert_eq!(live(&app), LiveShape::Triangle { rect: [305.0, 206.0, 100.0, 100.0], corner_radius: 0.0 });
}

#[test]
fn click_dialogs_and_drags_keep_complete_stroke_defaults() {
    let style = crate::shape_stroke_ui::StrokeOptions {
        align: photocraft_doc::StrokeAlign::Outside,
        cap: photocraft_doc::LineCap::Square,
        join: photocraft_doc::LineJoin::Bevel,
        miter_limit: 7.0,
        dashes: vec![3.0, 1.0, 0.0, 2.0],
        dash_offset: -0.75,
        opacity: 0.6,
    };
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon] {
        for is_click in [false, true] {
            let mut app = app();
            app.ui.tool_options.stroke_width = 7.0;
            app.ui.tool_options.shape_stroke = style.clone();
            let history = app.session.active().unwrap().history.past_len();
            if is_click {
                click(&mut app, tool, [20.0, 30.0]);
                let id = only_dialog(&app);
                confirm_by_id(&mut app, id);
            } else {
                drag(&mut app, tool, [20.0, 30.0], [120.0, 130.0]);
                assert!(app.framework.dialogs.is_empty(), "a drag draws without a dialog");
            }
            let st = app.session.active().unwrap();
            let LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected shape") };
            let stroke = sh.stroke.as_ref().unwrap();
            assert_eq!(crate::shape_stroke_ui::StrokeOptions::from(stroke), style, "{tool:?}, click={is_click}");
            assert_eq!(stroke.width, 7.0);
            assert_eq!(st.history.past_len(), history + 1);
        }
    }
}

#[test]
fn agents_click_set_and_confirm_over_the_control_channel() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let call = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Reply::Done(v) => v,
            _ => panic!("deferred"),
        }
    };
    // The canvas shows first, so the request maps the points through its view, and the frames after it deliver the events.
    testing::control(&mut app, "ui.pointer", json!({"tool": "rectangle", "events": [{"kind": "down", "x": 40, "y": 50}, {"kind": "up", "x": 40, "y": 50}]}));
    let id = only_dialog(&app);
    for (field, value) in [("width", json!(64)), ("height", json!(32))] {
        call(&mut app, "ui.dialog.set", json!({"dialog": id.value(), "field": field, "value": value}));
    }
    let r = call(&mut app, "ui.dialog.confirm", json!({"dialog": id.value()}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert_eq!(live(&app), LiveShape::Rect { rect: [40.0, 50.0, 64.0, 32.0], radii: [0.0; 4] });
}

/// CodeRabbit on #9456: in Path mode a click opens no Create dialog, whose OK would make a shape
/// layer; Path mode never draws one (a drag adds its outline to the work path).
#[test]
fn a_path_mode_click_opens_no_create_dialog() {
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon] {
        let mut app = app();
        app.ui.tool_options.shape_mode = VectorMode::Path;
        let before = layers(&app);
        click(&mut app, tool, [20.4, 30.2]);
        assert!(app.framework.dialogs.is_empty(), "{tool:?}: no Create dialog in Path mode");
        assert_eq!(layers(&app), before, "{tool:?}: and no layer");
    }
}

/// CodeRabbit on #9456: a Path-mode click that moved a little, still within the click slop, adds
/// nothing to the work path either. Its box would round to a 1 × 1 px outline.
#[test]
fn a_path_mode_click_that_moved_a_little_adds_no_subpath() {
    let work = |app: &PhotocraftApp| app.session.active().unwrap().doc.work_path.as_ref().map_or(0, |p| p.subpaths.len());
    for tool in [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line] {
        for end in [[20.8, 30.8], [21.0, 31.0]] {
            let mut app = app();
            app.ui.tool_options.shape_mode = VectorMode::Path;
            let before = layers(&app);
            drag(&mut app, tool, [20.2, 30.2], end);
            assert_eq!(work(&app), 0, "{tool:?} to {end:?}: no subpath");
            assert!(app.framework.dialogs.is_empty(), "{tool:?} to {end:?}: no Create dialog");
            assert_eq!(layers(&app), before, "{tool:?} to {end:?}: and no layer");
        }
    }
}
