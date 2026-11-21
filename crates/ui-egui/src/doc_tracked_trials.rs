//! Count trials for the documents the session holds in a `Tracked` (#15709): a job that applies to a document in the
//! background moves only that document's revision and builds nothing keyed on the active one, an idle frame and a
//! preference write that keeps the history limits move no document's revision, and a view-only write that leaves the
//! edit revision alone still moves the tracked one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use craft::ui::runtime::measure::last_frame_derived_builds;
use egui_kittest::Harness;
use photocraft_engine::channel_cmds::ChannelTarget;
use photocraft_engine::jobs::Started;
use serde_json::json;

use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app with two new documents, A (index 0) and B (index 1), each with a pixel layer, B active, settled.
fn harness() -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        for _ in 0..2 {
            s.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
            s.execute("layer.new.layer", json!({})).unwrap();
        }
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    h
}

/// How many times the last frame built the value derived under `id`.
fn builds(h: &H, id: egui::Id) -> u32 {
    last_frame_derived_builds(&h.ctx).iter().filter(|(built, _)| *built == id).map(|(_, n)| n).sum()
}

/// Every open document's `Tracked` revision.
fn revisions(h: &H) -> Vec<craft::ui::contract::Revision> {
    h.state().session.documents().iter().map(|st| st.revision()).collect()
}

#[test]
fn a_job_on_another_document_builds_nothing_keyed_on_the_active_one() {
    let mut h = harness();
    // A filter job on A, held until B is the active document.
    h.state_mut().session.set_active(0);
    let gate = Arc::new(AtomicBool::new(false));
    let open = Arc::clone(&gate);
    let started = h
        .state_mut()
        .session
        .start_job(
            "filter.blur.gaussianBlur",
            json!({"radius": 2}),
            "Deferred Blur",
            true,
            move |job| {
                while !open.load(Ordering::Relaxed) {
                    job.check()?;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            },
            |s, ()| {
                s.edit("Deferred Blur", |doc, active| {
                    let layer = active.ok_or_else(|| photocraft_engine::EngineError::Other("no active layer".into()))?;
                    let surface = doc.layer_mut(layer).and_then(|l| l.surface_mut()).ok_or(photocraft_engine::EngineError::NoLayer(layer))?;
                    surface.fill_rect(photocraft_geom::Rect::new(0, 0, 1, 1), &[0.4, 0.3, 0.2, 1.0]);
                    Ok(())
                })?;
                Ok(json!({}))
            },
        )
        .unwrap();
    let Started::Job(job) = started else { panic!("the job ran inline") };
    h.state_mut().session.set_active(1);
    h.run_steps(4);

    let before = revisions(&h);
    gate.store(true, Ordering::Relaxed);
    h.state_mut().session.wait_job(job).unwrap();
    h.run_steps(1);
    let after = revisions(&h);
    assert_ne!(after[0], before[0], "the job's apply moves A's revision");
    assert_eq!(after[1], before[1], "and leaves B's");
    assert_eq!(builds(&h, crate::panels::layers_tree::model_id()), 0, "B's Layers model builds nothing");

    h.state_mut().session.set_active(0);
    h.run_steps(1);
    assert_eq!(builds(&h, crate::panels::layers_tree::model_id()), 1, "switching to A builds its Layers model once");
}

#[test]
fn an_idle_frame_moves_no_documents_revision() {
    let mut h = harness();
    h.state_mut().run("window.toggle.history", json!({"show": true})).unwrap();
    h.state_mut().run("window.toggle.navigator", json!({"show": true})).unwrap();
    h.run_steps(8);
    let settled = revisions(&h);
    for frame in 0..10 {
        h.run_steps(1);
        assert_eq!(revisions(&h), settled, "idle frame {frame} moves no document's revision");
    }
}

#[test]
fn a_preference_write_that_keeps_the_history_limits_moves_no_documents_revision() {
    let mut h = harness();
    let settled = revisions(&h);
    let on = h.state().session.prefs().script_events.enabled;
    h.state_mut().session.edit_prefs(|prefs| prefs.script_events.enabled = !on);
    assert_eq!(revisions(&h), settled, "a preference write that keeps the history limits moves no document's revision");
    let states = h.state().session.prefs().performance.history_states;
    h.state_mut().session.edit_prefs(|prefs| prefs.performance.history_states = states + 1);
    assert!(revisions(&h).iter().zip(&settled).all(|(now, was)| now != was), "a new history limit moves every document's revision");
}

#[test]
fn targeting_a_mask_over_colour_rows_moves_only_the_tracked_revision() {
    let mut h = harness();
    h.state_mut().run("layer.layerMask.revealAll", json!({})).unwrap();
    h.state_mut().session.active_mut().unwrap().channel_view.target = ChannelTarget::Color(0);
    h.run_steps(4);
    let st = h.state().session.active_tracked().unwrap();
    let (edits, tracked) = (st.revision, st.revision());
    h.state_mut().target_mask_over_colour_rows();
    let st = h.state().session.active_tracked().unwrap();
    assert_eq!(st.channel_view.target, ChannelTarget::LayerMask, "the mask is the target");
    assert_eq!(st.revision, edits, "a view change is no edit step");
    assert_ne!(st.revision(), tracked, "but it moves the tracked revision views key on");
}
