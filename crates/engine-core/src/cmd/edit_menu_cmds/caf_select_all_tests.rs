//! #14253: with the whole canvas selected, Content-Aware Fill (Auto sampling) has nothing to
//! sample from and does nothing, without an error or a history state.

use super::*;

/// A 64 x 48 document with a striped Background and a red blob.
fn caf_session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 8, "background": "white"})).unwrap();
    s.edit("stripes", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        for x in (0..64).step_by(8) {
            surf.fill_rect(Rect::new(x, 0, x + 3, 48), &[0.0, 0.0, 0.0, 1.0]);
        }
        surf.fill_rect(Rect::new(28, 18, 36, 28), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn with_select_all_the_fill_does_nothing_without_an_error_or_a_step() {
    for output in ["current", "new", "duplicate"] {
        let mut s = caf_session();
        s.execute("select.all", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let (past, revision) = (s.active().unwrap().history.past_len(), s.active().unwrap().revision);
        let p = if output == "current" { json!({}) } else { json!({"output": output}) };
        let r = s.execute("edit.contentAwareFill", p).unwrap_or_else(|e| panic!("{output}: {e}"));
        assert_eq!(r["filled"], 0, "{output}: {r}");
        let st = s.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &st.doc), "{output}: the document changed");
        assert_eq!(st.history.past_len(), past, "{output}: a history step was recorded");
        assert_eq!(st.revision, revision, "{output}: the revision moved");
        // There is no fill step for Edit › Fade to fade, and the Select All before it isn't one.
        assert!(!s.is_enabled("edit.fade"), "{output}: Fade is offered after a fill that did nothing");
    }
}

/// The silence is for the whole canvas only: one unselected strip along the edge is still
/// sampled and filled from, as one step named after the command.
#[test]
fn a_selection_short_of_the_whole_canvas_still_fills() {
    let mut s = caf_session();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 60, "height": 48})).unwrap();
    let past = s.active().unwrap().history.past_len();
    let r = s.execute("edit.contentAwareFill", json!({})).unwrap();
    assert!(r["filled"].as_u64().unwrap() > 0, "{r}");
    let st = s.active().unwrap();
    assert_eq!(st.history.past_len(), past + 1);
    assert_eq!(st.history.undo_label(), Some("Content-Aware Fill"));
}

/// Rectangular or Custom sampling with the whole canvas selected was not measured, so it keeps
/// the "nothing to sample from" refusal of #6532.
#[test]
fn rectangular_and_custom_sampling_with_select_all_are_still_refused() {
    let mut s = caf_session();
    s.execute("select.all", json!({})).unwrap();
    let revision = s.active().unwrap().revision;
    for p in [json!({"sampling": "rectangular"}), json!({"sampling": "custom", "area": [0, 0, 64, 48]})] {
        let e = s.execute("edit.contentAwareFill", p.clone()).unwrap_err().to_string();
        assert!(e.contains("nothing to sample from"), "{p}: {e}");
        assert_eq!(s.active().unwrap().revision, revision, "{p}: no history step");
    }
}
