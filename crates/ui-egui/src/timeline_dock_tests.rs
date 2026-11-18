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
    crate::panel_docking::show_panel(h.state_mut(), "timeline", Some(true)).unwrap();
    h.run_steps(6);
    h
}

#[test]
fn the_timeline_panel_shows_the_frame_strip() {
    let h = harness();
    assert!(crate::panel_docking::is_shown(h.state(), "timeline"));
    // Each frame's delay is on the strip.
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 3);
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
fn window_timeline_is_available_in_the_menu_with_or_without_a_document() {
    // Without a document the panel can still be opened (it offers to make an animation once there is one), and
    // the item says whether the panel is showing.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(crate::menus::is_enabled(&app, "window.panel.timeline"), "greyed out in Window ▸ Timeline");
    let shown = crate::view_cmds::checked(&app, "window.panel.timeline");
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.panel.timeline", json!({})).unwrap();
    assert_ne!(crate::view_cmds::checked(&app, "window.panel.timeline"), shown, "the checkmark follows the panel");
}

#[test]
fn the_window_menu_hides_and_shows_the_timeline() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.timeline", json!({})).unwrap();
    h.run_steps(4);
    assert!(!crate::panel_docking::is_shown(h.state(), "timeline"));
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 0, "a closed panel draws no strip");
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.timeline", json!({})).unwrap();
    h.run_steps(6);
    assert!(crate::panel_docking::is_shown(h.state(), "timeline"));
    assert_eq!(h.query_all_by_label_contains("0.2 sec.").count(), 3, "it comes back with its strip");
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
    assert!(h.state().ui.timeline.playing());
    assert_eq!(current_frame(&h), 0, "pressing Play on the last frame goes back to the first");
}

#[test]
fn play_on_a_looping_animation_carries_on_from_where_it_is() {
    let mut h = harness();
    h.state_mut().run("animation.selectFrame", json!({"frame": 2})).unwrap();
    h.run_steps(3);
    h.get_by_label("Play").click();
    h.run_steps(1);
    assert!(h.state().ui.timeline.playing());
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
    h.state_mut().ui.timeline.frame_playing = true;
    h.state_mut().ui.timeline.last_time = -100.0;
    h.state_mut().ui.timeline.plays_done = 0;
    // The shell runs a retained panel again when the app's data changes (as pressing Play does); from then on
    // playback asks for its own repaints.
    h.state_mut().commands_run = h.state().commands_run.wrapping_add(1);
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
    // The panel isn't drawn while the job runs, so playback waits; the first passes after it ends tick again.
    for _ in 0..30 {
        if current_frame(&h) != 2 {
            break;
        }
        h.run_steps(1);
    }
    // A step can run more than one playback pass, so the animation may have moved on from the first frame by
    // now: what matters is that it left the last frame, wrapping round once.
    assert!(current_frame(&h) <= 1, "wrapped round to the start");
    assert_eq!(h.state().ui.timeline.plays_done, 1);
}
