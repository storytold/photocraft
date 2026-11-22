//! #11975: with a selection, the Perspective Crop tool's first drag (or first corner click) drops
//! it as its own "Deselect" step, so the selection is gone after a commit and after Esc.

use egui::Modifiers;
use serde_json::json;

use super::*;
use crate::canvas::testing::{ToolEvent, frame, tool_event};

/// A 200 × 150 document with a selection and the Perspective Crop tool.
fn app(selection: bool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    if selection {
        app.run("select.rect", json!({"x": 30, "y": 30, "width": 60, "height": 40})).unwrap();
    }
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

fn selected(app: &PhotocraftApp) -> bool {
    app.session.active().unwrap().doc.selection.is_some()
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

#[test]
fn the_first_press_of_a_session_deselects_once_as_its_own_step() {
    // A drag with a selection.
    let mut app = app(true);
    assert!(selected(&app), "choosing the tool keeps the selection");
    let before = steps(&app);
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    assert!(!selected(&app), "the selection is gone");
    assert_eq!(steps(&app), before + 1, "one step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Deselect"));
    assert!(quad(&app).is_some(), "the quad appears");
    // Further passes of the frame run no second Deselect.
    frame(&mut app);
    frame(&mut app);
    assert_eq!(steps(&app), before + 1);
    // Esc afterwards leaves the document deselected.
    cancel(&mut app);
    assert!(!selected(&app));

    // The first corner click, then three more.
    let mut app = self::app(true);
    let before = steps(&app);
    for p in [[20.0, 20.0], [180.0, 25.0], [175.0, 130.0], [25.0, 125.0]] {
        drag(&mut app, p, p);
    }
    assert!(!selected(&app));
    assert_eq!(steps(&app), before + 1, "one Deselect for the four clicks");
    assert!(quad(&app).is_some(), "four clicks made the quad");

    // Without a selection no step is added, nor by editing the quad later.
    let mut app = self::app(false);
    let before = steps(&app);
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    assert_eq!(steps(&app), before, "no Deselect state");
    let q = quad(&app).unwrap();
    drag(&mut app, q[0], [30.0, 30.0]);
    assert_eq!(steps(&app), before);

    // The commit leaves the selection gone.
    let mut app = self::app(true);
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    apply(&mut app);
    assert!(!selected(&app));
    assert!(quad(&app).is_none(), "committed");
}

#[test]
fn a_selection_made_between_corner_clicks_is_kept() {
    let mut app = app(true);
    let p = [[20.0, 20.0], [180.0, 25.0], [175.0, 130.0], [25.0, 125.0]];
    drag(&mut app, p[0], p[0]);
    // Select All after the first corner: the later clicks belong to the same quad.
    app.run("select.all", json!({})).unwrap();
    let before = steps(&app);
    for c in &p[1..] {
        drag(&mut app, *c, *c);
    }
    assert!(selected(&app), "only the quad's first press deselects");
    assert_eq!(steps(&app), before, "no second Deselect step");
    assert!(quad(&app).is_some(), "four clicks made the quad");
}

#[test]
fn a_press_while_a_job_locks_the_document_starts_no_quad() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut app = app(true);
    let gate = Arc::new(AtomicBool::new(false));
    let open = gate.clone();
    let job = app
        .session
        .start_job(
            "test.slow",
            json!({}),
            "Slow Filter",
            true,
            move |ctx| {
                while !open.load(Ordering::Relaxed) {
                    ctx.check()?;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            },
            |_, ()| Ok(json!(null)),
        )
        .unwrap();
    let photocraft_engine::jobs::Started::Job(job) = job else { panic!("ran inline") };
    // Deselect is refused while the job runs: the press doesn't start a quad that would keep the
    // selection, so it can be tried again once the job is done.
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    assert!(selected(&app), "the job refused the Deselect");
    assert!(!pending(&app), "no quad without the Deselect: {:?}", quad(&app));
    // A click starts no clicked corner either.
    drag(&mut app, [30.0, 30.0], [30.0, 30.0]);
    assert!(!pending(&app), "no corner without the Deselect");
    gate.store(true, Ordering::Relaxed);
    app.session.wait_job(job).unwrap();
    drag(&mut app, [20.0, 20.0], [180.0, 130.0]);
    assert!(!selected(&app));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Deselect"));
    assert!(quad(&app).is_some());
}
