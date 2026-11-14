//! #13569: making another document active while Free Transform is open applies the box in its own
//! document, as clicking outside the canvas does in Photoshop, instead of throwing it away. The
//! document switched to keeps its pixels, history and active state.

use super::super::tests::{app_with_square, frame_of, land_jobs, moved_place};
use super::*;
use crate::transform_tool::begin;
use photocraft_engine::DocState;
use photocraft_geom::Rect;

/// Where the red square of [`app_with_square`] starts.
const SQUARE: Rect = Rect::new(8, 8, 24, 24);

/// A (`app_with_square`, the square's layer active) and B (32 × 32 with a new layer: one step of
/// its own), with A active and Free Transform open on the square, moved 8 px right:
/// (app, A, B, the square's layer, A's history steps before the box).
fn moved_in_a() -> (PhotocraftApp, DocId, DocId, LayerId, usize) {
    let mut app = app_with_square(64, SQUARE);
    let a = app.session.active().unwrap().doc.id;
    app.session.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let b = app.session.active().unwrap().doc.id;
    assert!(app.session.set_active(0));
    app.sync_views();
    let st = app.session.active().unwrap();
    let (layer, steps) = (st.active_layer.unwrap(), st.history.past_len());
    begin(&mut app, &egui::Context::default()).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 8.0, y]);
    (app, a, b, layer, steps)
}

/// The open document `id`.
fn doc(app: &PhotocraftApp, id: DocId) -> &DocState {
    app.session.documents().iter().find(|st| st.doc.id == id).expect("the document is open")
}

/// The opaque bounds of `layer`'s pixels in document `id`.
fn square(app: &PhotocraftApp, id: DocId, layer: LayerId) -> Rect {
    doc(app, id).doc.layer(layer).unwrap().surface().unwrap().content_bounds()
}

/// What a switch must leave alone in the document switched to: its pixels, history, redo list,
/// revision and active layer.
fn snapshot(st: &DocState) -> (Arc<photocraft_doc::Document>, Vec<String>, Vec<String>, u64, Option<LayerId>) {
    (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect(), st.revision, st.active_layer)
}

/// Asserts that document `id` is as `before` ([`snapshot`]) left it.
fn assert_untouched(app: &PhotocraftApp, id: DocId, before: &(Arc<photocraft_doc::Document>, Vec<String>, Vec<String>, u64, Option<LayerId>)) {
    let now = snapshot(doc(app, id));
    assert!(Arc::ptr_eq(&now.0, &before.0), "the other document's pixels are unchanged");
    assert_eq!((&now.1, &now.2, now.3, now.4), (&before.1, &before.2, before.3, before.4), "and its history, redo, revision and active layer");
}

/// Picking B's tab applies A's box in A as one Free Transform step, and B stays active and
/// untouched. Undo and Redo then work per document: B's Undo takes back B's own step, A's takes
/// back the transform and Redo puts it back.
#[test]
fn another_document_applies_the_box_in_its_own_document_with_undo_and_redo() {
    let (mut app, a, b, layer, steps) = moved_in_a();
    let b_before = snapshot(doc(&app, b));
    assert!(app.session.set_active(1));
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
    assert_eq!(app.session.active().unwrap().doc.id, b, "the document picked stays active");
    assert_untouched(&app, b, &b_before);
    assert_eq!(square(&app, a, layer), SQUARE.translate(8, 0), "A has the move");
    assert_eq!(doc(&app, a).history.past_len(), steps + 1);
    assert_eq!(doc(&app, a).history.undo_label(), Some("Free Transform"));

    // Undo in B is B's own: its new layer goes, A keeps the move.
    let b_steps = doc(&app, b).history.past_len();
    assert!(app.session.undo());
    assert_eq!(doc(&app, b).history.past_len(), b_steps - 1);
    assert_eq!(square(&app, a, layer), SQUARE.translate(8, 0));
    assert!(app.session.redo());

    // Back in A, Undo takes the move back and Redo applies it again.
    assert!(app.session.set_active(0));
    app.sync_views();
    end_if_left(&mut app);
    assert!(app.session.undo());
    assert_eq!(square(&app, a, layer), SQUARE, "Undo takes the move back");
    assert_eq!(doc(&app, a).history.past_len(), steps);
    assert!(app.session.redo());
    assert_eq!(square(&app, a, layer), SQUARE.translate(8, 0), "Redo applies it again");
    assert_eq!(doc(&app, a).history.past_len(), steps + 1);
    assert_eq!(doc(&app, b).history.past_len(), b_steps, "B's history is its own");
}

/// Window › Next Document (⌃Tab) is a switch like a tab click: the box applies in A.
#[test]
fn next_document_applies_the_box() {
    let (mut app, a, b, layer, steps) = moved_in_a();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.nextDocument", json!({})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(app.session.active().unwrap().doc.id, b);
    assert_eq!(square(&app, a, layer), SQUARE.translate(8, 0));
    assert_eq!(doc(&app, a).history.past_len(), steps + 1);
}

/// Closing the document switched to returns to A with the box applied, and Undo there takes it
/// back. Closing another document while A stays active leaves the box open, as nothing left it.
#[test]
fn closing_the_other_document_keeps_the_applied_box() {
    let (mut app, a, _, layer, steps) = moved_in_a();
    assert!(app.session.set_active(1));
    end_if_left(&mut app);
    app.session.close(1);
    app.sync_views();
    end_if_left(&mut app);
    assert_eq!(app.session.active().unwrap().doc.id, a, "back in A");
    assert!(app.ui.transform.is_none());
    assert_eq!(square(&app, a, layer), SQUARE.translate(8, 0));
    assert!(app.session.undo());
    assert_eq!(square(&app, a, layer), SQUARE);
    assert_eq!(doc(&app, a).history.past_len(), steps);

    let (mut app, a, _, layer, steps) = moved_in_a();
    let revision = doc(&app, a).revision;
    app.session.close(1);
    end_if_left(&mut app);
    assert!(app.ui.transform.is_some(), "the box stays open over A");
    assert_eq!((square(&app, a, layer), doc(&app, a).revision), (SQUARE, revision), "nothing applied yet");
    commit(&mut app);
    assert_eq!(square(&app, a, layer), SQUARE.translate(8, 0), "↩ still applies it");
    assert_eq!(doc(&app, a).history.past_len(), steps + 1);
}

/// A box the engine refuses (here a collapsed one) can't stay open on a document that is no
/// longer active: it ends as Esc would, A unchanged, with the reason in the status bar.
#[test]
fn a_refused_box_ends_without_changing_either_document() {
    let (mut app, a, b, layer, steps) = moved_in_a();
    app.ui.transform.as_mut().unwrap().quad = [[16.0, 16.0]; 4];
    let (a_before, b_before) = (snapshot(doc(&app, a)), snapshot(doc(&app, b)));
    assert!(app.session.set_active(1));
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
    assert!(app.ui.status.contains("collapses"), "{}", app.ui.status);
    assert_untouched(&app, a, &a_before);
    assert_untouched(&app, b, &b_before);
    assert_eq!((square(&app, a, layer), doc(&app, a).history.past_len()), (SQUARE, steps));
    assert_eq!(app.session.active().unwrap().doc.id, b);
}

/// With commits in the background, as in the desktop app, a dropped file moved in its box and
/// left for another tab is applied once the job lands: the place and its move are one step in A,
/// and the other document stays active and untouched.
#[test]
fn a_background_commit_lands_in_its_own_document() {
    let (mut app, placed, frame, steps) = moved_place();
    let a = app.session.active().unwrap().doc.id;
    app.session.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    let b = app.session.active().unwrap().doc.id;
    let b_before = snapshot(doc(&app, b));
    end_if_left(&mut app);
    assert!(committing(&app).is_some(), "the commit runs in the background");
    assert_eq!(app.session.active().unwrap().doc.id, b, "while B stays active");
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
    assert_eq!(app.session.active().unwrap().doc.id, b);
    assert_untouched(&app, b, &b_before);
    let st = doc(&app, a);
    assert_eq!(st.history.past_len(), steps + 1, "the place and its move are one step");
    assert_eq!(st.history.undo_label(), Some(photocraft_engine::file_cmds::PLACE_EMBEDDED));
    assert!(app.session.set_active(0));
    assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "the dropped file stays, moved");
}
