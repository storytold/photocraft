//! #12839: in Shape mode a new shape layer from an open path takes the options bar's fill, as in
//! Photoshop. The fill covers the curve and the chord back to its start; the path stays open and
//! its stroke stays as before.

use craft::draw::ui::VectorMode;
use photocraft_doc::ShapeLayer;

use super::*;

/// The measured document: 800 × 600 px RGB/8 with the Freeform Pen in Shape mode.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::FreeformPen;
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    app
}

/// Finishes a Freeform trace as a stroke that began and ended in this state does.
fn finish(app: &mut PhotocraftApp, points: &[[f64; 3]]) {
    let owner = capture_pen_owner(app).unwrap();
    freeform_finish_owned(app, points, &[], owner);
}

/// The measured open stroke: x 100 to 500 in 4 px steps, y = 400 + 60 sin(2π (x − 100) / 200).
fn sine() -> Vec<[f64; 3]> {
    (0..=100)
        .map(|i| {
            let x = 100.0 + 4.0 * f64::from(i);
            [x, 400.0 + 60.0 * (std::f64::consts::TAU * (x - 100.0) / 200.0).sin(), 1.0]
        })
        .collect()
}

/// The measured closed stroke: a circle around (600, 300) of radius 100, back to its start.
fn circle() -> Vec<[f64; 3]> {
    (0..=60)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / 60.0;
            [600.0 + 100.0 * a.cos(), 300.0 + 100.0 * a.sin(), 1.0]
        })
        .collect()
}

/// The active layer's shape.
fn shape(app: &PhotocraftApp) -> ShapeLayer {
    let st = app.session.active().unwrap();
    let LayerContent::Shape(shape) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("expected a shape layer") };
    shape.clone()
}

/// The flattened document's pixel at (x, y).
fn flat_px(app: &PhotocraftApp, x: usize, y: usize) -> [f32; 4] {
    let doc = &app.session.active().unwrap().doc;
    let buf = photocraft_compose::flatten(doc);
    buf.px[y * buf.rect.width() as usize + x]
}

fn is_black(p: [f32; 4]) -> bool {
    p[..3].iter().all(|c| *c < 0.05) && p[3] > 0.95
}

/// The options bar's fill as a stored paint.
fn options_fill(app: &PhotocraftApp) -> photocraft_doc::Fill {
    assert!(app.ui.tool_options.shape_fill, "the reset options have a fill");
    super::super::shapes::tool_paint(app, false)
}

#[test]
fn open_freeform_sine_takes_the_options_fill_and_stays_open() {
    let mut app = app();
    let n = app.session.active().unwrap().history.past_len();
    finish(&mut app, &sine());
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), n + 1, "one step");
    assert_eq!(st.history.undo_label(), Some("Freeform Pen"));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let shape = shape(&app);
    assert_eq!(shape.fill, Some(options_fill(&app)), "the open shape takes the options bar's fill");
    assert_eq!(shape.path.subpaths.len(), 1);
    assert!(!shape.path.subpaths[0].closed, "the fill does not close the path");
    // Between the curve and the chord y = 400: below it in the first period, above in the second.
    for (x, y) in [(150, 430), (250, 370)] {
        let p = flat_px(&app, x, y);
        assert!(is_black(p), "({x},{y}) is filled, got {p:?}");
    }
}

#[test]
fn closed_freeform_circle_still_closes_and_fills() {
    let mut app = app();
    finish(&mut app, &circle());
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Freeform Pen"));
    let shape = shape(&app);
    assert!(shape.path.subpaths[0].closed);
    assert_eq!(shape.fill, Some(options_fill(&app)));
    assert!(is_black(flat_px(&app, 600, 300)), "the circle's centre is filled");
}

#[test]
fn open_freeform_shape_keeps_its_stroke() {
    let mut app = app();
    let foreground = [0.836521, 0.125468, 0.745632, 1.0];
    app.run("tools.setColors", json!({"foreground": foreground, "background": [0.0, 1.0, 0.0, 1.0]})).unwrap();
    assert!(stroke_param(&app).is_null(), "the reset options have no stroke");
    finish(&mut app, &sine());
    let stroke = shape(&app).stroke.expect("an open shape keeps its 1 px foreground stroke");
    assert_eq!(stroke.width, 1.0);
    assert_eq!(stroke.paint, super::super::options::solid_paint(foreground));
}

#[test]
fn open_pen_draft_committed_in_shape_mode_takes_the_fill() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.pen = Some(PenPath::corners(&[[100.0, 100.0], [300.0, 100.0], [300.0, 300.0]], capture_pen_owner(&app)));
    assert!(pen_commit(&mut app, false));
    let shape = shape(&app);
    assert_eq!(shape.fill, Some(options_fill(&app)));
    assert!(!shape.path.subpaths[0].closed);
    // Inside the triangle the chord (300,300)–(100,100) closes.
    assert!(is_black(flat_px(&app, 250, 150)), "the open Pen path is filled to its chord");
}
