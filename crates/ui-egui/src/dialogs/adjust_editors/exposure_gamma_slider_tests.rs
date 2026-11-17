//! #14637: the Exposure dialog's Gamma Correction slider runs from 9.99 at the left to 0.01 at
//! the right with 1 in the middle (Photoshop); the Properties panel keeps its plain track.

use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::adjust_editors::{EditorCx, editor};

/// The Exposure editor as the dialog (`dialog`) or the Properties panel hosts it.
fn exposure_harness(dialog: bool) -> Harness<'static, serde_json::Value> {
    // Clicks a second apart, so two near each other never make a double-click (reset).
    let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).with_step_dt(0.25).build_ui_state(
        move |ui, values: &mut serde_json::Value| {
            let cx = EditorCx {
                mem: egui::Id::unique("exposure-gamma"),
                hist: None,
                gray: false,
                swatches: [[0.0; 3], [1.0; 3]],
                gradients: Vec::new(),
                dialog,
                dynamic_color_sliders: true,
                isolated_curve: None,
            };
            editor(ui, "exposure", values, &cx);
        },
        json!({}),
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// The Gamma Correction slider's track (the slider's rect less its knob margins).
fn gamma_track(h: &Harness<'static, serde_json::Value>) -> Rect {
    let sliders: Vec<Rect> = h.query_all_by_role(egui::accesskit::Role::Slider).map(|n| n.rect()).collect();
    assert_eq!(sliders.len(), 3, "Exposure, Offset and Gamma Correction");
    sliders[2].shrink2(vec2(7.0, 0.0))
}

/// Clicks the Gamma track at `x` and returns the value it stores.
fn gamma_at(h: &mut Harness<'static, serde_json::Value>, x: f32) -> f32 {
    let at = Pos2 { x, y: gamma_track(h).center().y };
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(4);
    h.state()["gamma"].as_f64().expect("gamma set") as f32
}

#[test]
fn dialog_gamma_track_runs_high_to_low_around_one() {
    let mut h = exposure_harness(true);
    let track = gamma_track(&h);
    assert_eq!(gamma_at(&mut h, track.left()), 9.99, "left end");
    assert_eq!(gamma_at(&mut h, track.center().x), 1.0, "centre");
    assert_eq!(gamma_at(&mut h, track.right()), 0.01, "right end");
    assert!(gamma_at(&mut h, track.center().x - 30.0) > gamma_at(&mut h, track.center().x - 10.0), "moving left raises the value");
}

/// Photoshop's measured knob positions from the centre of a ~240 px track (9.99 at 113 px left,
/// 0.01 at 112 px right), scaled to ours: each value lies within 3 px of where we draw it.
#[test]
fn dialog_gamma_track_matches_measured_points() {
    let mut h = exposure_harness(true);
    let track = gamma_track(&h);
    let scale = track.width() / (113.0 + 112.0);
    for (gamma, offset) in [(1.2, -4.0), (2.35, -19.0), (9.99, -113.0), (0.01, 112.0)] {
        let x = track.center().x + offset * scale;
        let (left, right) = (gamma_at(&mut h, (x - 3.0).max(track.left())), gamma_at(&mut h, (x + 3.0).min(track.right())));
        assert!(right <= gamma && gamma <= left, "{gamma} lies between {right} and {left} (3 px either side of {x})");
    }
}

#[test]
fn panel_gamma_track_stays_plain() {
    let mut h = exposure_harness(false);
    let track = gamma_track(&h);
    assert_eq!(gamma_at(&mut h, track.left()), 0.01, "left end");
    assert_eq!(gamma_at(&mut h, track.right()), 9.99, "right end");
    assert_eq!(gamma_at(&mut h, track.center().x), 5.0, "centre of a linear track");
}
