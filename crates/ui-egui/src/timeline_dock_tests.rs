use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

/// An app with a three-layer document that has a frame animation (one frame per layer).
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute("layer.new.layer", json!({"name": "A"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "B"})).unwrap();
        s.execute("animation.makeFramesFromLayers", json!({"seconds": 0.2})).unwrap();
        PhotocraftApp::new(s, Default::default())
    });
    h.state_mut().ui.timeline.open = true;
    h.run_steps(6);
    h
}

#[test]
fn the_timeline_docks_along_the_bottom_by_default() {
    let h = harness();
    assert!(!h.state().ui.timeline.floating);
    // Docked, the header offers to float it; the strip shows each frame's delay.
    assert!(h.query_by_label("Float Panel").is_some());
    assert!(h.query_by_label("Dock Panel").is_none());
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 3);
}

#[test]
fn the_dock_button_floats_the_panel_and_back() {
    let mut h = harness();
    h.get_by_label("Float Panel").click();
    h.run_steps(6);
    assert!(h.state().ui.timeline.floating);
    assert!(h.query_by_label("Dock Panel").is_some(), "a floating panel offers to dock");
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 3, "the strip is the same floating");
    h.get_by_label("Dock Panel").click();
    h.run_steps(6);
    assert!(!h.state().ui.timeline.floating);
}

#[test]
fn clicking_a_frame_selects_it_when_docked() {
    let mut h = harness();
    h.state_mut().run("animation.selectFrame", json!({"frame": 0})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.frame_animation.as_ref().unwrap().current, 0);
    h.get_by_label("Next frame").click();
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.frame_animation.as_ref().unwrap().current, 1);
}

#[test]
fn the_window_menu_hides_and_shows_the_docked_timeline() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.timeline", json!({})).unwrap();
    h.run_steps(4);
    assert!(!h.state().ui.timeline.open);
    assert!(h.query_by_label("Float Panel").is_none());
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.timeline", json!({})).unwrap();
    h.run_steps(6);
    assert!(h.query_by_label("Float Panel").is_some(), "it comes back docked");
}

fn current_frame(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().doc.frame_animation.as_ref().unwrap().current
}

#[test]
fn play_at_the_last_frame_of_a_finite_animation_starts_over() {
    let mut h = harness();
    h.state_mut().run("animation.setLoop", json!({"loops": "once"})).unwrap();
    h.state_mut().run("animation.selectFrame", json!({"frame": 2})).unwrap();
    h.run_steps(3);
    h.get_by_label("Play").click();
    h.run_steps(1);
    assert!(h.state().ui.timeline.playing);
    assert_eq!(current_frame(&h), 0, "pressing Play on the last frame goes back to the first");
}

#[test]
fn play_on_a_looping_animation_carries_on_from_where_it_is() {
    let mut h = harness();
    h.state_mut().run("animation.selectFrame", json!({"frame": 2})).unwrap();
    h.run_steps(3);
    h.get_by_label("Play").click();
    h.run_steps(1);
    assert!(h.state().ui.timeline.playing);
    assert_eq!(current_frame(&h), 2, "a looping animation doesn't need to rewind");
}

#[test]
fn playback_does_not_count_a_pass_while_a_background_job_blocks_the_frame_change() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut h = harness();
    h.state_mut().run("animation.setLoop", json!({"loops": 3})).unwrap();
    h.state_mut().run("animation.selectFrame", json!({"frame": 2})).unwrap();
    h.run_steps(2);
    // A job that holds the document until the test opens its gate, so it can't finish (and be polled away
    // by the UI's own per-frame poll) before the first playback tick.
    let gate = Arc::new(AtomicBool::new(false));
    let hold = gate.clone();
    let started = h.state_mut().session.start_job(
        "test.gated",
        json!({}),
        "Gated Job",
        true,
        move |ctx| {
            while !hold.load(Ordering::Relaxed) {
                ctx.check()?;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Ok(())
        },
        |_, ()| Ok(json!({})),
    );
    if !matches!(started.unwrap(), photocraft_engine::jobs::Started::Job(_)) {
        return; // ran inline: nothing to block
    }
    h.state_mut().ui.timeline.playing = true;
    h.state_mut().ui.timeline.last_time = -100.0;
    h.state_mut().ui.timeline.plays_done = 0;
    h.run_steps(1);
    assert_eq!(h.state().ui.timeline.plays_done, 0, "the refused frame change isn't a pass");
    assert_eq!(current_frame(&h), 2);
    // Open the gate and let the UI's own poll finish the job; the waiting tick then goes through and counts.
    gate.store(true, Ordering::Relaxed);
    let t = std::time::Instant::now();
    while h.state().session.active_job().is_some() {
        assert!(t.elapsed() < std::time::Duration::from_secs(60), "the job did not finish");
        h.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(current_frame(&h), 0, "wrapped round to the first frame");
    assert_eq!(h.state().ui.timeline.plays_done, 1);
}
