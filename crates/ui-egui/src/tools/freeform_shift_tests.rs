//! #12832: Shift held while drawing with the Freeform Pen leaves the freehand stroke as drawn: a vee gives the same
//! path with Shift as without, its corner kept. Shift keeps constraining the painting tools (their own tests).

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;
use craft::draw::ui::VectorMode;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::FreeformPen;
    app.ui.extras.snap = false;
    app.ui.tool_options.vector_mode = VectorMode::Path;
    app
}

/// The anchors of the Work Path a vee from (100,450) down to (250,550) and up to (400,450) leaves, drawn with `mods`.
fn vee(mods: Modifiers) -> Vec<[f64; 2]> {
    let mut app = app();
    testing::press(&mut app, [100.0, 450.0], mods);
    for i in 1..=10 {
        let t = f64::from(i) / 10.0;
        testing::move_to(&mut app, [100.0 + 150.0 * t, 450.0 + 100.0 * t], mods);
    }
    for i in 1..=10 {
        let t = f64::from(i) / 10.0;
        testing::move_to(&mut app, [250.0 + 150.0 * t, 550.0 - 100.0 * t], mods);
    }
    testing::release(&mut app, [400.0, 450.0], mods);
    let work = app.session.active().unwrap().doc.work_path.clone().expect("the stroke committed a work path");
    work.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

#[test]
fn shift_leaves_the_freehand_vee_as_drawn() {
    let plain = vee(Modifiers::NONE);
    let shifted = vee(Modifiers::SHIFT);
    assert_eq!(shifted, plain, "the same path with and without Shift");
    assert!(shifted.iter().any(|a| a[1] > 520.0), "the corner is kept: {shifted:?}");
}
