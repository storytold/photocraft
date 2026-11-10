//! #12651: the Blur tool starts, and comes back after Reset Tool, with the reference application's
//! tip: a 13 px soft round (hardness 0 %, angle 0°, roundness 100 %) at 25 % spacing. Other tools'
//! starting tips are unchanged.

use serde_json::json;

use crate::PhotocraftApp;
use crate::burn_default_tip_tests::{app, invoke, pick};
use crate::state::Tool;

/// Size, hardness, spacing, angle and roundness of the live brush, as `brush.get` reports them.
fn tip(app: &mut PhotocraftApp) -> [f64; 5] {
    let b = app.run("brush.get", json!({})).unwrap();
    ["size", "hardness", "spacing", "angle", "roundness"].map(|k| (b[k].as_f64().unwrap() * 1e4).round() / 1e4)
}

/// A tip that is no tool's starting tip, so a reset has something to undo.
fn scramble(app: &mut PhotocraftApp) {
    let b = &mut app.session.tools.brush;
    (b.size, b.hardness, b.spacing, b.angle, b.roundness) = (57.0, 0.4, 0.6, 30.0, 0.5);
}

const BLUR_TIP: [f64; 5] = [13.0, 0.0, 0.25, 0.0, 1.0];

#[test]
fn reset_tool_gives_the_blur_tool_a_13_px_soft_tip() {
    let mut app = app();
    pick(&mut app, Tool::Blur);
    scramble(&mut app);
    invoke(&mut app, "tool.reset", json!({}));
    assert_eq!(tip(&mut app), BLUR_TIP);
}

#[test]
fn a_first_use_starts_with_the_tip_and_then_keeps_its_own() {
    // No stored tool brushes, as on a fresh install: the Blur tool's first pick, after another
    // tool's tip.
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Blur);
    assert_eq!(tip(&mut app), BLUR_TIP);
    app.session.tools.brush.size = 42.0;
    pick(&mut app, Tool::Pencil);
    pick(&mut app, Tool::Blur);
    assert_eq!(tip(&mut app)[0], 42.0);
}

#[test]
fn the_sharpen_tool_still_takes_the_current_tip_on_first_use() {
    let mut app = app();
    pick(&mut app, Tool::Pencil);
    scramble(&mut app);
    pick(&mut app, Tool::Sharpen);
    assert_eq!(tip(&mut app)[0], 57.0);
}
