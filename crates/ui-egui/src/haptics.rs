//! Haptic feedback on a haptic mouse, at the moments Photoshop pulses one: something snaps to an
//! alignment guide, a slider reaches its end, a lasso closes, Object Selection selects, and a
//! Content-Aware Move drops.
//!
//! The desktop app plays them through [`photocraft_haptics`] ([`Services::haptics`]); the web
//! build and tests have none, and every call here does nothing then. App code reaches the player
//! through the services; widgets, which see only the egui context, through [`attach`].
//!
//! [`Services::haptics`]: crate::Services::haptics

use photocraft_engine::jobs::{JobEvent, JobOutcome};
use photocraft_engine::snap::{SnapKind, SnapLine};
pub use photocraft_haptics::{Haptics, Waveform};
use serde_json::Value;

use crate::PhotocraftApp;

/// Where [`attach`] keeps the player in the egui context.
fn key() -> egui::Id {
    egui::Id::unique("photocraft-haptics")
}

/// Plays `waveform` on the haptic mouse, if the app has one.
pub(crate) fn play(app: &PhotocraftApp, waveform: Waveform) {
    if let Some(h) = &app.services.haptics {
        h.play(waveform);
    }
}

/// [`play`] for code that has only the egui context (widgets).
pub(crate) fn play_in(ctx: &egui::Context, waveform: Waveform) {
    if let Some(h) = ctx.data(|d| d.get_temp::<Haptics>(key())) {
        h.play(waveform);
    }
}

/// Makes the player reachable from [`play_in`] and applies Preferences › Enhanced Controls ›
/// Haptic feedback. Called every frame; cheap once it is there.
pub(crate) fn attach(ctx: &egui::Context, haptics: Option<&Haptics>, enabled: bool) {
    let Some(h) = haptics else { return };
    h.set_enabled(enabled);
    if ctx.data(|d| d.get_temp::<Haptics>(key()).is_none()) {
        ctx.data_mut(|d| d.insert_temp(key(), h.clone()));
    }
}

/// Whether `lines` shows an alignment `before` didn't: the moment a guide appears is the moment
/// to pulse, once, however many lines it brings (Microsoft's haptics guidance). Grid lines don't
/// count: a drag crosses one every few pixels, and Photoshop doesn't pulse for them.
pub(crate) fn new_alignment(before: &[SnapLine], lines: &[SnapLine]) -> bool {
    let same = |a: &SnapLine, b: &SnapLine| a.vertical == b.vertical && a.kind == b.kind && (a.pos - b.pos).abs() < 1e-6;
    lines.iter().filter(|l| l.kind != SnapKind::Grid).any(|l| !before.iter().any(|b| same(b, l)))
}

/// Whether a slider moved by the user from `before` to `after` has just reached an end of
/// `lo..=hi`.
pub(crate) fn reached_end(before: f32, after: f32, lo: f32, hi: f32) -> bool {
    // An empty or NaN range has no ends to reach.
    if lo.partial_cmp(&hi) != Some(std::cmp::Ordering::Less) {
        return false;
    }
    let at_end = |v: f32| v <= lo || v >= hi;
    after != before && at_end(after) && !(at_end(before) && (before <= lo) == (after <= lo))
}

/// Whether a selection command's result selected something: it ran and didn't report
/// `changed: false` (nothing found). A command that went to the background hasn't selected
/// anything yet: its pulse comes when the job lands ([`on_job_event`]).
pub(crate) fn selected(result: &Result<Value, String>) -> bool {
    result.as_ref().is_ok_and(selected_now)
}

fn selected_now(v: &Value) -> bool {
    !pending(v) && v.get("changed").and_then(Value::as_bool) != Some(false)
}

/// Plays the change-of-state pulse for a command that ran, unless it went to the background:
/// then [`on_job_event`] plays it when the result lands.
pub(crate) fn landed(app: &PhotocraftApp, result: &Value) {
    if !pending(result) {
        play(app, Waveform::Success);
    }
}

/// A background job ended: Content-Aware Move and the selection commands pulse when their result
/// lands, which is when Photoshop pulses for them.
pub(crate) fn on_job_event(app: &PhotocraftApp, e: &JobEvent) {
    if job_pulses(&e.command, &e.outcome) {
        play(app, Waveform::Success);
    }
}

fn job_pulses(command: &str, outcome: &JobOutcome) -> bool {
    let JobOutcome::Done(result) = outcome else { return false };
    match command {
        "paint.contentAwareMove" => true,
        "select.object" | "select.lasso" | "select.magneticLasso" => selected_now(result),
        _ => false,
    }
}

/// A command's reply when it was started as a background job (`jobs_ui::run`).
fn pending(result: &Value) -> bool {
    result.get("pending").and_then(Value::as_bool) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(vertical: bool, pos: f64, kind: SnapKind) -> SnapLine {
        SnapLine { vertical, pos, from: 0.0, to: 10.0, kind }
    }

    #[test]
    fn a_new_guide_pulses_once() {
        let guide = line(true, 40.0, SnapKind::Guide);
        assert!(new_alignment(&[], &[guide]));
        assert!(!new_alignment(&[guide], &[guide]));
        assert!(!new_alignment(&[guide], &[]));
    }

    #[test]
    fn moving_to_another_alignment_pulses_again() {
        let edge = line(false, 12.0, SnapKind::LayerEdge);
        let center = line(false, 50.0, SnapKind::LayerCenter);
        assert!(new_alignment(&[edge], &[center]));
        assert!(new_alignment(&[edge], &[edge, line(true, 3.0, SnapKind::DocumentEdge)]));
    }

    #[test]
    fn grid_lines_never_pulse() {
        assert!(!new_alignment(&[], &[line(true, 8.0, SnapKind::Grid)]));
        assert!(new_alignment(&[], &[line(true, 8.0, SnapKind::Grid), line(false, 0.0, SnapKind::Guide)]));
    }

    #[test]
    fn a_slider_pulses_when_it_reaches_an_end() {
        assert!(reached_end(50.0, 100.0, 0.0, 100.0));
        assert!(reached_end(3.0, 0.0, 0.0, 100.0));
        // Dragged past the end, the value is clamped there.
        assert!(reached_end(99.0, 100.0, 0.0, 100.0));
    }

    #[test]
    fn a_slider_stays_quiet_inside_or_at_the_same_end() {
        assert!(!reached_end(10.0, 20.0, 0.0, 100.0));
        assert!(!reached_end(100.0, 100.0, 0.0, 100.0));
        assert!(!reached_end(100.0, 60.0, 0.0, 100.0));
        assert!(!reached_end(f32::NAN, 100.0, 100.0, 0.0));
        assert!(!reached_end(1.0, 0.0, f32::NAN, 1.0));
    }

    #[test]
    fn a_slider_dragged_from_one_end_to_the_other_pulses() {
        assert!(reached_end(0.0, 100.0, 0.0, 100.0));
        assert!(reached_end(100.0, 0.0, 0.0, 100.0));
    }

    #[test]
    fn only_a_selection_that_found_something_pulses() {
        assert!(selected(&Ok(serde_json::json!({"changed": true}))));
        assert!(selected(&Ok(serde_json::json!({}))));
        assert!(!selected(&Ok(serde_json::json!({"changed": false}))));
        assert!(!selected(&Err("no".into())));
        assert!(!selected(&Ok(serde_json::json!({"job": 4, "pending": true}))), "a background job hasn't selected yet");
    }

    #[test]
    fn background_moves_and_selections_pulse_when_they_land() {
        let done = |v: Value| JobOutcome::Done(v);
        assert!(job_pulses("paint.contentAwareMove", &done(serde_json::json!({}))));
        assert!(job_pulses("select.object", &done(serde_json::json!({"changed": true}))));
        assert!(!job_pulses("select.object", &done(serde_json::json!({"changed": false}))));
        assert!(!job_pulses("paint.contentAwareMove", &JobOutcome::Cancelled));
        assert!(!job_pulses("paint.contentAwareMove", &JobOutcome::Failed("no".into())));
        assert!(!job_pulses("filter.gaussianBlur", &done(serde_json::json!({}))));
    }

    #[test]
    fn without_a_player_nothing_happens() {
        let ctx = egui::Context::default();
        play_in(&ctx, Waveform::Hover);
        attach(&ctx, None, true);
        play_in(&ctx, Waveform::Hover);
    }
}
