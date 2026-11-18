//! #13235: with a path operation other than New Layer, the shape tools' next drag becomes a
//! component of the active shape layer instead of a new layer.

use photocraft_doc::vector::PathOp;
use photocraft_doc::{LayerContent, ShapeLayer};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::EllipseShape;
    app
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let mods = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layer_count()
}

fn active_shape(app: &PhotocraftApp) -> ShapeLayer {
    let st = app.session.active().unwrap();
    match &st.doc.layer(st.active_layer.unwrap()).unwrap().content {
        LayerContent::Shape(s) => s.clone(),
        _ => panic!("the active layer is not a shape layer"),
    }
}

/// What is active before the drag.
#[derive(Clone, Copy)]
enum Active {
    Shape,
    Pixel,
}

/// The custom shape the drag draws.
#[derive(Clone, Copy)]
enum Preset {
    Ring,
    /// Two parts that exclude each other: not one component.
    Split,
}

/// What the drag does.
enum Want {
    /// No new layer; the active shape's subpath operations are these.
    Joined(&'static [PathOp]),
    /// One more layer, a shape layer, active.
    NewShapeLayer,
    /// No new layer, the shape unchanged, a status error.
    Refused,
}

struct Case {
    op: &'static str,
    active: Active,
    tool: Tool,
    preset: Option<Preset>,
    want: Want,
}

#[test]
fn path_operations_place_the_next_shape() {
    use PathOp::{Combine, Exclude, Intersect, Join, Subtract};
    let ellipse = |op, want| Case { op, active: Active::Shape, tool: Tool::EllipseShape, preset: None, want };
    let cases = [
        ellipse("combine", Want::Joined(&[Combine, Combine])),
        ellipse("subtract", Want::Joined(&[Combine, Subtract])),
        ellipse("intersect", Want::Joined(&[Combine, Intersect])),
        ellipse("exclude", Want::Joined(&[Combine, Exclude])),
        ellipse("newLayer", Want::NewShapeLayer),
        Case { op: "combine", active: Active::Pixel, tool: Tool::EllipseShape, preset: None, want: Want::NewShapeLayer },
        Case { op: "subtract", active: Active::Shape, tool: Tool::CustomShape, preset: Some(Preset::Ring), want: Want::Joined(&[Combine, Subtract, Join]) },
        Case { op: "combine", active: Active::Shape, tool: Tool::CustomShape, preset: Some(Preset::Split), want: Want::Refused },
    ];
    for (i, case) in cases.iter().enumerate() {
        let mut app = app();
        match case.active {
            Active::Shape => drag(&mut app, [50.0, 50.0], [200.0, 200.0]),
            Active::Pixel => {
                app.run("layer.new.layer", json!({})).unwrap();
            }
        }
        let before = matches!(case.active, Active::Shape).then(|| active_shape(&app).path);
        let preset = match case.preset {
            None => None,
            Some(Preset::Ring) => Some("Ring".to_string()),
            Some(Preset::Split) => {
                use photocraft_doc::vector::{Path, Subpath};
                let xor = Path::new(vec![
                    Subpath::polygon(&[(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (0.0, 60.0)]),
                    Subpath::polygon(&[(40.0, 40.0), (100.0, 40.0), (100.0, 100.0), (40.0, 100.0)]).with_op(PathOp::Exclude),
                ]);
                let r = app.run("shape.presets.new", json!({"name": "Xor", "path": photocraft_engine::vector_cmds::path_json(&xor)})).unwrap();
                Some(r["name"].as_str().unwrap().to_string())
            }
        };
        app.ui.tool = case.tool;
        if let Some(name) = preset {
            app.ui.presets_ui.custom_shape = name;
        }
        app.ui.tool_options.shape_path_op = case.op.into();
        let layers = layer_count(&app);
        drag(&mut app, [100.0, 100.0], [250.0, 250.0]);
        match &case.want {
            Want::Joined(ops) => {
                assert_eq!(layer_count(&app), layers, "case {i} ({}): no new layer", case.op);
                let got: Vec<PathOp> = active_shape(&app).path.subpaths.iter().map(|s| s.op).collect();
                assert_eq!(got, *ops, "case {i} ({})", case.op);
            }
            Want::NewShapeLayer => {
                assert_eq!(layer_count(&app), layers + 1, "case {i} ({}): a new layer", case.op);
                let st = app.session.active().unwrap();
                assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, LayerContent::Shape(_)), "case {i}: a shape layer is active");
            }
            Want::Refused => {
                assert_eq!(layer_count(&app), layers, "case {i}: no layer");
                assert_eq!(Some(active_shape(&app).path), before, "case {i}: the layer is unchanged");
                assert!(app.ui.status_error, "case {i}: {}", app.ui.status);
            }
        }
    }
}
