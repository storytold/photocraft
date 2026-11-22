//! #14285: Edit › Define Custom Shape is enabled only for a path targeted in the Paths panel, or
//! for a shape layer or vector-masked layer with a path tool, as in Photoshop. It never falls back
//! to the last saved path.

use photocraft_doc::VectorMask;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const ID: &str = "edit.defineCustomShape";

/// The tools Photoshop keeps it enabled with for a shape layer.
const PATH_TOOLS: [Tool; 15] = [
    Tool::PathSelection,
    Tool::DirectSelection,
    Tool::Pen,
    Tool::FreeformPen,
    Tool::CurvaturePen,
    Tool::AddAnchorPoint,
    Tool::Rectangle,
    Tool::EllipseShape,
    Tool::Triangle,
    Tool::Polygon,
    Tool::Star,
    Tool::Line,
    Tool::CustomShape,
    Tool::Move,
    Tool::Type,
];

/// The tools Photoshop greys it with when only a shape layer would give it a path.
const OTHER_TOOLS: [Tool; 9] =
    [Tool::RectMarquee, Tool::EllipseMarquee, Tool::Lasso, Tool::Brush, Tool::Eraser, Tool::Crop, Tool::Eyedropper, Tool::Hand, Tool::Zoom];

fn square(x: f64, y: f64, w: f64) -> serde_json::Value {
    json!({"subpaths": [{"closed": true, "knots": [[x, y], [x + w, y], [x + w, y + w], [x, y + w]]}]})
}

/// A 240×160 document whose pixel layer is active, with three saved paths and nothing targeted.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 240, "height": 160})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    for (i, name) in ["Outline", "Badge", "Notch"].into_iter().enumerate() {
        app.run("path.set", json!({"name": name, "path": square(10.0 + 30.0 * i as f64, 20.0, 24.0)})).unwrap();
    }
    app.ui.selected_path = None;
    app
}

fn enabled_with(app: &mut PhotocraftApp, tool: Tool) -> bool {
    app.ui.tool = tool;
    crate::menus::is_enabled(app, ID)
}

fn add_shape_layer(app: &mut PhotocraftApp) {
    app.run("shape.create", json!({"kind": "rect", "rect": [120, 40, 60, 50], "fill": "#2f7a4c"})).unwrap();
}

#[test]
fn no_document_greys_it() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(!crate::menus::is_enabled(&app, ID));
}

#[test]
fn a_pixel_or_type_layer_with_nothing_targeted_greys_it_despite_saved_paths() {
    let mut app = app();
    for tool in [Tool::Move, Tool::Brush, Tool::PathSelection, Tool::Pen] {
        assert!(!enabled_with(&mut app, tool), "pixel layer, {tool:?}");
    }
    app.run("type.create", json!({"x": 20, "y": 90, "text": "Quill", "size": 18})).unwrap();
    app.ui.selected_path = None;
    for tool in [Tool::Move, Tool::Type, Tool::PathSelection] {
        assert!(!enabled_with(&mut app, tool), "type layer, {tool:?}");
    }
}

#[test]
fn a_work_path_that_is_not_targeted_does_not_count() {
    let mut app = app();
    app.run("path.set", json!({"name": "work", "path": square(60.0, 60.0, 30.0)})).unwrap();
    app.ui.selected_path = None;
    assert!(!enabled_with(&mut app, Tool::PathSelection));
    assert!(!enabled_with(&mut app, Tool::Brush));
}

#[test]
fn a_targeted_saved_path_or_work_path_enables_it_with_any_tool() {
    let mut app = app();
    app.ui.selected_path = Some("Badge".into());
    for tool in PATH_TOOLS.into_iter().chain(OTHER_TOOLS) {
        assert!(enabled_with(&mut app, tool), "saved path targeted, {tool:?}");
    }
    app.run("path.set", json!({"name": "work", "path": square(60.0, 60.0, 30.0)})).unwrap();
    app.ui.selected_path = Some("work".into());
    for tool in [Tool::Brush, Tool::Hand, Tool::RectMarquee, Tool::Move] {
        assert!(enabled_with(&mut app, tool), "work path targeted, {tool:?}");
    }
}

#[test]
fn a_shape_layer_enables_it_only_with_the_path_tools() {
    let mut app = app();
    add_shape_layer(&mut app);
    for target in [None, Some("layer")] {
        app.ui.selected_path = target.map(str::to_string);
        for tool in PATH_TOOLS {
            assert!(enabled_with(&mut app, tool), "shape layer, {target:?}, {tool:?}");
        }
        for tool in OTHER_TOOLS {
            assert!(!enabled_with(&mut app, tool), "shape layer, {target:?}, {tool:?}");
        }
    }
}

#[test]
fn a_vector_masked_layer_follows_the_shape_layer_rule() {
    let mut app = app();
    let mask = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polygon(&[(30.0, 30.0), (90.0, 30.0), (90.0, 90.0)])]);
    app.session
        .edit("vector mask", |doc, active| {
            let id = active.expect("an active layer");
            doc.layer_mut(id).unwrap().vector_mask = Some(VectorMask { path: mask, enabled: true, linked: true, density: 1.0, feather: 0.0 });
            Ok(())
        })
        .unwrap();
    assert!(enabled_with(&mut app, Tool::DirectSelection));
    assert!(enabled_with(&mut app, Tool::Move));
    assert!(!enabled_with(&mut app, Tool::Brush));
    assert!(!enabled_with(&mut app, Tool::Lasso));
}

#[test]
fn running_it_never_stores_the_last_saved_path() {
    let mut app = app();
    assert!(app.session.execute(ID, json!({"name": "Fallback"})).is_err());
}

/// A tool whose state was not measured keeps the engine's rule: a pixel layer with a nonempty untargeted Work Path
/// enables it (CodeRabbit on #16186).
#[test]
fn an_unmeasured_tool_keeps_the_engines_rule_with_a_work_path() {
    let mut app = app();
    app.run("path.set", json!({"name": "work", "path": square(10.0, 20.0, 24.0), "work": true})).ok();
    app.session
        .edit("work path", |doc, _| {
            doc.work_path = Some(photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(1.0, 1.0), (11.0, 1.0), (11.0, 11.0), (1.0, 11.0)])]));
            Ok(())
        })
        .unwrap();
    app.ui.selected_path = None;
    assert!(app.session.is_enabled(ID), "the engine enables it with a Work Path");
    assert!(enabled_with(&mut app, Tool::CloneStamp), "and the unmeasured Clone Stamp follows the engine");
}

/// A path targeted in the Paths panel enables it with any tool, an unmeasured one included (CodeRabbit on #16186).
#[test]
fn a_targeted_saved_path_enables_it_with_an_unmeasured_tool() {
    let mut app = app();
    app.ui.selected_path = Some("Badge".into());
    assert!(enabled_with(&mut app, Tool::CloneStamp), "the targeted saved path wins over the engine's rule");
}
