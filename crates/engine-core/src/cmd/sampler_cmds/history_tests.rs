use super::*;

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    s
}

fn samplers(s: &mut Session) -> Value {
    s.execute("view.colorSamplers.list", json!({})).unwrap()["samplers"].clone()
}

fn check_step(s: &mut Session, cmd: &str, params: Value, label: &str) {
    let before = samplers(s);
    let steps = s.active().unwrap().history.past_len();
    s.execute(cmd, params).unwrap();
    let after = samplers(s);
    assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some(label));

    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(s), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(samplers(s), after);
}

#[test]
fn sampler_commands_record_undoable_history() {
    let mut s = session();
    for (x, y) in [(3, 4), (8, 9), (12, 15)] {
        check_step(&mut s, "view.colorSamplers.add", json!({"x": x, "y": y}), "New Color Sampler");
    }
    check_step(&mut s, "view.colorSamplers.move", json!({"index": 1, "x": 6, "y": 7}), "Drag Color Sampler");
    check_step(&mut s, "view.colorSamplers.delete", json!({"index": 1}), "Delete Color Sampler");
    assert_eq!(samplers(&mut s)[1]["position"], json!([12.5, 15.5]));
    assert_eq!(samplers(&mut s)[1]["number"], 2);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s).as_array().unwrap().len(), 3);
    check_step(&mut s, "view.colorSamplers.clear", json!({}), "Delete Color Sampler");
    assert_eq!(samplers(&mut s), json!([]));
    assert!(!s.active().unwrap().is_dirty());
}

#[test]
fn refused_sampler_edits_leave_undo_and_redo_intact() {
    let mut s = session();
    for x in 0..10 {
        s.execute("view.colorSamplers.add", json!({"x": x, "y": 1})).unwrap();
    }
    s.execute("view.colorSamplers.delete", json!({"index": 0})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let before = samplers(&mut s);
    let st = s.active().unwrap();
    let history = st.history.entries();
    let redo: Vec<_> = st.history.redo_labels().map(str::to_owned).collect();
    let revision = st.revision;
    for (cmd, params) in [
        ("view.colorSamplers.add", json!({"x": 20, "y": 1})),
        ("view.colorSamplers.add", json!({"x": -1, "y": 1})),
        ("view.colorSamplers.move", json!({"index": 0, "x": 1, "y": f64::NAN})),
        ("view.colorSamplers.move", json!({"index": 100, "x": 1, "y": 1})),
        ("view.colorSamplers.move", json!({"x": 1, "y": 1})),
        ("view.colorSamplers.delete", json!({"index": 100})),
        ("view.colorSamplers.delete", json!({})),
    ] {
        assert!(s.execute(cmd, params).is_err(), "{cmd}");
    }
    let st = s.active().unwrap();
    assert_eq!(st.history.entries(), history);
    assert!(st.history.redo_labels().eq(redo.iter().map(String::as_str)));
    assert_eq!(st.revision, revision);
    assert_eq!(samplers(&mut s), before);
}

#[test]
fn sampler_edits_stay_in_history_after_clear_history_is_undone() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 3, "y": 4})).unwrap();
    let before = samplers(&mut s);
    s.execute("history.clear", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), before);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), json!([]));
}

#[test]
fn undoing_canvas_size_keeps_existing_sampler_geometry_behavior() {
    let mut s = session();
    s.execute("view.colorSamplers.add", json!({"x": 3, "y": 4})).unwrap();
    s.execute("image.canvasSize", json!({"width": 50, "height": 40, "anchor": "bottomRight"})).unwrap();
    let moved = samplers(&mut s);
    assert_eq!(moved[0]["position"], json!([13.5, 14.5]));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), moved);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), json!([]));
}

#[test]
fn non_linear_sampler_steps_cross_retained_rows_in_order() {
    let mut s = session();
    s.execute("history.options", json!({"allowNonLinearHistory": true})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 3, "y": 4})).unwrap();
    let first = samplers(&mut s);
    s.execute("view.colorSamplers.add", json!({"x": 8, "y": 9})).unwrap();
    let second = samplers(&mut s);
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("view.colorSamplers.clear", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), second);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), first);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), second);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), json!([]));
}

#[test]
fn deleting_a_sampler_history_row_preserves_the_surrounding_states() {
    let mut s = session();
    s.execute("history.options", json!({"allowNonLinearHistory": true})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 3, "y": 4})).unwrap();
    let first = samplers(&mut s);
    s.execute("view.colorSamplers.add", json!({"x": 8, "y": 9})).unwrap();
    s.execute("view.colorSamplers.move", json!({"index": 1, "x": 12, "y": 14})).unwrap();
    let moved = samplers(&mut s);
    let current = s.active().unwrap().history.past_len();
    s.execute("history.deleteState", json!({"state": current - 1})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), first);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), moved);
}

#[test]
fn grouped_action_keeps_undo_for_sampler_only_edits() {
    let mut s = session();
    s.edit_prefs(|prefs| prefs.actions.create_single_history_state = true);
    crate::action_history::begin(&mut s, "Pin colors");
    s.execute("view.colorSamplers.add", json!({"x": 3, "y": 4})).unwrap();
    s.execute("view.colorSamplers.add", json!({"x": 8, "y": 9})).unwrap();
    crate::action_history::finish(&mut s);

    assert_eq!(s.active().unwrap().history.undo_label(), Some("Action: Pin colors"));
    let pinned = samplers(&mut s);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), json!([]));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(samplers(&mut s), pinned);
}

#[test]
fn clearing_an_empty_sampler_list_leaves_history_alone() {
    let mut s = session();
    let history = s.active().unwrap().history.entries();
    assert_eq!(s.execute("view.colorSamplers.clear", json!({})).unwrap(), json!({"deleted": 0, "remaining": 0}));
    assert_eq!(s.active().unwrap().history.entries(), history);
}
