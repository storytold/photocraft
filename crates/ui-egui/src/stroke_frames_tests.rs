//! A brush stroke driven one event per `ui.pointer` call, as the stroke script drives it: every frame of it runs one
//! pass, and the status bar, a settling region beside the canvas, does not run while the stroke moves.

use serde_json::{Value, json};

use crate::Services;
use crate::lifecycle_tests::{answered, app_harness};

/// One `ui.pointer` event of a Brush stroke at document point `(x, 16)`.
fn event(kind: &str, x: f64) -> Value {
    json!({"tool": "Brush", "events": [{"kind": kind, "x": x, "y": 16.0}]})
}

/// Whether a component of the type named `short` ran in the last pass.
fn ran(h: &egui_kittest::Harness<'static, crate::PhotocraftApp>, short: &str) -> bool {
    craft::ui::testing::last_pass_runs(&h.ctx).iter().any(|(ty, _)| ty.split('<').next().is_some_and(|name| name.ends_with(short)))
}

/// A frame's passes, the reasons a pass asked to run again, and whether the status bar's readout ran in it.
fn frame(h: &egui_kittest::Harness<'static, crate::PhotocraftApp>) -> (usize, Vec<String>, bool) {
    let output = &h.output().platform_output;
    (output.num_completed_passes, output.request_discard_reasons.iter().map(|cause| cause.reason.to_string()).collect(), ran(h, "StatusReadout"))
}

#[test]
fn a_stroke_runs_one_pass_per_frame_and_leaves_the_status_bar_settled() {
    let (mut h, transport) = app_harness(Services::default(), 1);
    assert_eq!(answered(&mut h, &transport, "ui.pointer", event("down", 4.0))["ok"], true);
    h.run_steps(4);
    // Frames of the moves (`frame`).
    let mut frames = Vec::new();
    for step in 1..=12 {
        let (req, reply) = crate::ControlRequest::new("ui.pointer", event("move", 4.0 + 2.0 * f64::from(step)));
        transport.send(req).expect("the channel is open");
        for _ in 0..64 {
            h.step();
            frames.push(frame(&h));
            if reply.try_recv().is_ok() {
                break;
            }
        }
        for _ in 0..4 {
            h.step();
            frames.push(frame(&h));
        }
    }
    assert_eq!(answered(&mut h, &transport, "ui.pointer", event("up", 28.0))["ok"], true);
    assert!(frames.iter().all(|(passes, _, _)| *passes == 1), "every frame of the stroke runs one pass: {frames:?}");
    assert!(frames.iter().all(|(_, _, status)| !status), "the settled status bar does not run during the stroke: {frames:?}");
}
