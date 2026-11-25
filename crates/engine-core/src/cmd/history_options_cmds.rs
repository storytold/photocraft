//! History panel › History Options… (#2966), Photoshop's five checkboxes: Automatically Create
//! First Snapshot, Automatically Create New Snapshot When Saving, Allow Non-Linear History, Show
//! New Snapshot Dialog by Default and Make Layer Visibility Changes Undoable. The values are
//! preferences (`historyOptions.*`, also `prefs.set`), applied by [`Session::add_document`],
//! [`snapshot_on_save`], [`Session::apply_prefs`], [`Session::edit_visibility`] and the GUI's
//! camera button. Without params the command reports them; the GUI opens its dialog instead and
//! runs it with the dialog's values on OK.

use serde_json::{Map, Value};

use crate::commands::CommandSpec;
use crate::history_cmds::Snapshot;
use crate::prefs::HistoryOptions;
use crate::{EngineError, Result, Session};

const CMD: &str = "history.options";

/// The option keys, as in `prefs.get historyOptions` and the command's params, in the order of
/// Photoshop's dialog.
pub const OPTIONS: [&str; 5] = [
    "automaticallyCreateFirstSnapshot",
    "automaticallyCreateNewSnapshotWhenSaving",
    "allowNonLinearHistory",
    "showNewSnapshotDialogByDefault",
    "makeLayerVisibilityChangesUndoable",
];

/// The option named `key` (one of [`OPTIONS`]).
fn option_mut<'a>(o: &'a mut HistoryOptions, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "automaticallyCreateFirstSnapshot" => &mut o.automatically_create_first_snapshot,
        "automaticallyCreateNewSnapshotWhenSaving" => &mut o.automatically_create_new_snapshot_when_saving,
        "allowNonLinearHistory" => &mut o.allow_non_linear_history,
        "showNewSnapshotDialogByDefault" => &mut o.show_new_snapshot_dialog_by_default,
        "makeLayerVisibilityChangesUndoable" => &mut o.make_layer_visibility_changes_undoable,
        _ => return None,
    })
}

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// The current History Options, keyed by [`OPTIONS`].
pub fn current(s: &Session) -> Value {
    let o = &s.prefs().history_options;
    serde_json::json!({
        "automaticallyCreateFirstSnapshot": o.automatically_create_first_snapshot,
        "automaticallyCreateNewSnapshotWhenSaving": o.automatically_create_new_snapshot_when_saving,
        "allowNonLinearHistory": o.allow_non_linear_history,
        "showNewSnapshotDialogByDefault": o.show_new_snapshot_dialog_by_default,
        "makeLayerVisibilityChangesUndoable": o.make_layer_visibility_changes_undoable,
    })
}

/// History Options › Show New Snapshot Dialog by Default: whether a plain click on the History
/// panel's camera button opens New Snapshot.
pub fn show_new_snapshot_dialog(s: &Session) -> bool {
    s.prefs().history_options.show_new_snapshot_dialog_by_default
}

/// History Options › Automatically Create New Snapshot When Saving: after the active document
/// was saved to its own file (not a copy or an export), add a snapshot of it named with the local
/// time of the save, as Photoshop does. Not a history state. Returns the new snapshot's id, or
/// `None` when the option is off, no document is active, or the document is unchanged since the
/// snapshot the previous save added (and that snapshot is still there), so saving a clean
/// document again doesn't add a duplicate row. Shells call it where they record a document save.
///
/// Memory: like every History panel snapshot, each one pins a copy of the document for the
/// session (only the tiles that differ from the document and the other snapshots cost memory).
/// [`crate::DocState::trim_history`] counts those tiles against the History & Cache budget by
/// dropping the oldest history states, but never drops a snapshot itself: many saves of a
/// changing document can hold more than the budget until snapshots are deleted or the document
/// is closed.
pub fn snapshot_on_save(s: &mut Session) -> Option<u64> {
    if !s.prefs().history_options.automatically_create_new_snapshot_when_saving {
        return None;
    }
    let st = s.active_mut()?;
    // Saved again without a change: the previous save's snapshot already shows this document.
    if let Some(prev) = st.save_snapshot
        && st.snapshots().iter().any(|snap| snap.id == prev && std::sync::Arc::ptr_eq(&snap.doc, &st.doc))
    {
        return None;
    }
    let name = save_snapshot_name().unwrap_or_else(|| crate::history_cmds::next_snapshot_name(st));
    let id = st.next_snapshot;
    st.next_snapshot = id.saturating_add(1);
    let (doc, layers) = (st.doc.clone(), st.layer_target());
    // Like the camera button: the document and its saved state are unchanged.
    st.snapshots_mut().push(Snapshot { id, name, doc, layers });
    st.save_snapshot = Some(id);
    // The snapshot's tiles count against the history budget.
    st.trim_history();
    Some(id)
}

/// The local time as `HH:MM:SS`, the name of a snapshot made on save; `None` without a clock
/// (the web build), where the snapshot gets the camera button's "Snapshot N" instead.
fn save_snapshot_name() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(chrono::Local::now().format("%H:%M:%S").to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let given: Map<String, Value> = match p {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(bad("params must be an object")),
    };
    // Validate everything before changing anything.
    let mut set = Vec::new();
    for (k, v) in &given {
        if option_mut(&mut HistoryOptions::default(), k).is_none() {
            return Err(bad(format!("unknown option `{k}` (expected {})", OPTIONS.join(", "))));
        }
        let b = v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false")))?;
        set.push((k.clone(), b));
    }
    if !set.is_empty() {
        s.edit_prefs(|prefs| {
            for (k, b) in &set {
                if let Some(o) = option_mut(&mut prefs.history_options, k) {
                    *o = *b;
                }
            }
        });
    }
    Ok(current(s))
}

/// History panel › Delete in non-linear history: delete state `i` (numbered as the History panel
/// lists them, `current` = the current state's index, the later dimmed states after it) and
/// nothing else. Deleting the current state steps back to the one before it; the later states
/// stay.
pub(crate) fn delete_state(s: &mut Session, i: usize, current: usize) -> Result<()> {
    if i == 0 {
        return Err(EngineError::BadParams { cmd: "history.deleteState".into(), msg: "the oldest state can't be deleted".into() });
    }
    // A background job is computing from the current state: it must not move under it.
    if s.active_job().is_some() {
        return Err(EngineError::Other("the history can't step back right now".into()));
    }
    let keep_visibility = !s.visibility_undoable();
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    st.coalesce = None;
    if i != current {
        let removed = match i.checked_sub(current.saturating_add(1)) {
            Some(j) => st.history.remove_later(j),
            None => st.history.remove(i),
        };
        if !removed {
            return Err(EngineError::Other(format!("state {i} is not held")));
        }
        // The document is unchanged: a view-only change that keeps a clean document clean.
        st.bump_view_revision(!st.is_dirty());
        return Ok(());
    }
    // The step that made the current state: the layers whose visibility it changed get it back.
    let changed = st.history.current_visibility().to_vec();
    let (mut d, layers) = st.history.drop_current().ok_or_else(|| EngineError::Other("the history can't step back right now".into()))?;
    // As Undo does: Save As changes file identity outside history.
    if d.name != st.doc.name {
        std::sync::Arc::make_mut(&mut d).name.clone_from(&st.doc.name);
    }
    if keep_visibility {
        crate::carry_visibility(&st.doc, &mut d, &changed);
    }
    let damage = crate::layer_multi_cmds::step_damage(&d, &st.doc);
    *st.doc_mut() = d;
    crate::restore_target(st, layers);
    st.revision += 1;
    st.record_unknown_damage();
    st.last_damage = damage;
    st.record_damage();
    Ok(())
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "History Options…",
        menu: &[],
        shortcut: None,
        params: r##"{"automaticallyCreateFirstSnapshot":bool?,"automaticallyCreateNewSnapshotWhenSaving":bool?,"allowNonLinearHistory":bool?,"showNewSnapshotDialogByDefault":bool?,"makeLayerVisibilityChangesUndoable":bool?} → all five values (no params: report them). Automatically Create First Snapshot: a new or opened document starts with a snapshot of itself, named after it (default on). Automatically Create New Snapshot When Saving: each save of the document (not a copy) adds a snapshot named with the time of the save (default off). Allow Non-Linear History: an edit after stepping back keeps the later states and is appended at the end; deleting a state deletes only it (default off). Show New Snapshot Dialog by Default: a click on the History panel's camera button opens New Snapshot, not only ⌥-click (default off). Make Layer Visibility Changes Undoable: showing or hiding a layer is a history state (default on). Also `prefs.set historyOptions.*`"##,
        enabled: always,
        run: options,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::SavedPrefsExt;
    use serde_json::json;

    fn entries(s: &Session) -> Vec<String> {
        s.active().unwrap().history.entries()
    }

    /// A document with a layer above the background: entries Open, New Layer.
    fn doc() -> (Session, u64) {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap().0;
        (s, id)
    }

    #[test]
    fn defaults_match_photoshop_and_round_trip_through_prefs() {
        let mut s = crate::session();
        let defaults = json!({
            "automaticallyCreateFirstSnapshot": true,
            "automaticallyCreateNewSnapshotWhenSaving": false,
            "allowNonLinearHistory": false,
            "showNewSnapshotDialogByDefault": false,
            "makeLayerVisibilityChangesUndoable": true,
        });
        assert_eq!(s.execute(CMD, json!({})).unwrap(), defaults);
        assert_eq!(s.execute("prefs.get", json!({"path": "historyOptions"})).unwrap(), current(&s));
        let flipped: Map<String, Value> = defaults.as_object().unwrap().iter().map(|(k, v)| (k.clone(), json!(!v.as_bool().unwrap()))).collect();
        let r = s.execute(CMD, Value::Object(flipped.clone())).unwrap();
        assert_eq!(r, Value::Object(flipped));
        for key in OPTIONS {
            assert_eq!(s.execute("prefs.get", json!({"path": format!("historyOptions.{key}")})).unwrap(), r[key], "{key}");
        }
        assert_eq!(s.execute("prefs.get", json!({"path": "historyOptions.allowNonLinearHistory"})).unwrap(), json!(true));
        // Saved and restored with the other preferences.
        let saved = s.prefs_to_json();
        let mut t = crate::session();
        t.load_prefs_json(&saved).unwrap();
        assert_eq!(current(&t), r);
        // prefs.set and prefs.reset reach them too.
        t.execute("prefs.set", json!({"path": "historyOptions.allowNonLinearHistory", "value": false})).unwrap();
        assert_eq!(t.execute(CMD, Value::Null).unwrap()["allowNonLinearHistory"], json!(false));
        t.execute("prefs.reset", json!({"path": "historyOptions"})).unwrap();
        assert_eq!(current(&t), current(&crate::session()));
        // Older preference files without the section load with the defaults.
        t.load_prefs_json("{}").unwrap();
        assert_eq!(current(&t), current(&crate::session()));
    }

    #[test]
    fn bad_params_are_refused_and_change_nothing() {
        let mut s = crate::session();
        let before = current(&s);
        for p in [
            json!({"allowNonLinearHistory": "yes"}),
            json!({"allowNonLinearHistory": 1}),
            json!({"makeLayerVisibilityChangesUndoable": null}),
            json!({"allowNonLinearHistory": true, "snapshots": true}),
            json!({"automaticallyCreateFirstSnapshot": "true"}),
            json!({"automaticallyCreateNewSnapshotWhenSaving": 0}),
            json!({"showNewSnapshotDialogByDefault": [true]}),
            json!({"showNewSnapshotDialogByDefault": true, "showNewSnapshotDialog": true}),
            json!({"AutomaticallyCreateFirstSnapshot": false}),
            json!([true]),
            json!("x"),
            json!(3),
        ] {
            assert!(s.execute(CMD, p.clone()).is_err(), "{p}");
            assert_eq!(current(&s), before, "{p} changed nothing");
        }
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.allowNonLinearHistory", "value": "on"})).is_err());
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.automaticallyCreateFirstSnapshot", "value": 1})).is_err());
        assert!(s.execute("prefs.set", json!({"path": "historyOptions.showNewSnapshotDialogByDefault", "value": null})).is_err());
        assert_eq!(current(&s), before);
    }

    #[test]
    fn layer_visibility_is_a_history_state_only_when_undoable() {
        // On (the default): the eye, Hide/Show Layers and ⌥-click each add a state.
        let (mut s, id) = doc();
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        assert_eq!(entries(&s).last().map(String::as_str), Some("Layer Visibility"));
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().visible);

        // Off: the document changes (and is modified) but History doesn't.
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        let before = entries(&s);
        let rev = s.active().unwrap().revision;
        s.execute("layer.setProps", json!({"layer": id, "visible": false})).unwrap();
        s.execute("layer.showLayers", json!({"layer": id})).unwrap();
        s.execute("layer.hideLayers", json!({"layer": id})).unwrap();
        s.execute("layer.showOnly", json!({"layer": id})).unwrap();
        s.execute("layer.showOnly", json!({"layer": id})).unwrap();
        // Hide Layers toggles (⌘,), so show the layer first for it to hide.
        s.execute("layer.showLayers", json!({"layer": id})).unwrap();
        s.execute("layer.hideLayers", json!({"layer": id})).unwrap();
        assert_eq!(entries(&s), before, "no visibility state was recorded");
        let st = s.active().unwrap();
        assert!(!st.doc.layer(photocraft_doc::LayerId(id)).unwrap().visible, "the change still applies");
        assert!(st.revision > rev && st.is_dirty());
        // Other properties still record a state.
        s.execute("layer.setProps", json!({"layer": id, "opacity": 0.5})).unwrap();
        assert_eq!(entries(&s).len(), before.len() + 1);
        // An unrecorded change leaves the redo states alone.
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
        assert!(s.active().unwrap().history.can_redo());
    }

    fn visible(s: &Session, id: u64) -> bool {
        s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().visible
    }

    fn snapshot_names(s: &Session) -> Vec<String> {
        s.active().unwrap().snapshots().iter().map(|snap| snap.name.clone()).collect()
    }

    /// PNG bytes of a small image, to open.
    fn png() -> Vec<u8> {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        crate::file_cmds::encode(&s.active().unwrap().doc, "a.png", None::<f64>).unwrap().0
    }

    /// Automatically Create First Snapshot (checked against Photoshop): on (the default), a new or
    /// opened document starts with a snapshot named after it; off, it starts with none and the
    /// camera's first snapshot is still "Snapshot 1".
    #[test]
    fn first_snapshot_follows_the_option_on_new_and_open() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let name = s.active().unwrap().doc.name.clone();
        assert_eq!(snapshot_names(&s), [name]);
        crate::file_cmds::open_bytes_as(&mut s, "photo.png", &png(), None, None).unwrap();
        assert_eq!(snapshot_names(&s), ["photo.png"], "named after the file, as Photoshop does");

        s.execute(CMD, json!({"automaticallyCreateFirstSnapshot": false})).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert!(snapshot_names(&s).is_empty());
        assert_eq!(entries(&s).len(), 1, "the opening state is still there");
        crate::file_cmds::open_bytes_as(&mut s, "photo.png", &png(), None, None).unwrap();
        assert!(snapshot_names(&s).is_empty());
        assert_eq!(s.execute("history.newSnapshot", json!({})).unwrap(), json!({"snapshot": 1, "name": "Snapshot 1"}));
        assert_eq!(crate::inspect::document(s.active().unwrap())["snapshots"], json!([{"id": 1, "name": "Snapshot 1"}]));
        // Documents opened earlier keep theirs: the option applies when a document opens.
        s.set_active(0);
        assert_eq!(snapshot_names(&s).len(), 1);
    }

    #[test]
    fn show_new_snapshot_dialog_reads_the_option() {
        let mut s = crate::session();
        assert!(!show_new_snapshot_dialog(&s));
        s.execute("prefs.set", json!({"path": "historyOptions.showNewSnapshotDialogByDefault", "value": true})).unwrap();
        assert!(show_new_snapshot_dialog(&s));
        assert_eq!(crate::inspect::session(&s)["historyOptions"]["showNewSnapshotDialogByDefault"], json!(true));
    }

    /// With the option off, a recorded step that changes visibility along with other properties
    /// (layer.setProps visible + opacity is one Layer Properties state) is undone and redone as a
    /// whole: the carried-over visibility skips the layers the step itself changed.
    #[test]
    fn undoing_a_mixed_visibility_and_opacity_change_undoes_both() {
        let (mut s, id) = doc();
        s.execute(CMD, json!({"makeLayerVisibilityChangesUndoable": false})).unwrap();
        let n = entries(&s).len();
        s.execute("layer.setProps", json!({"layer": id, "visible": false, "opacity": 0.5})).unwrap();
        assert_eq!(entries(&s).len(), n + 1, "the mixed change is a state");
        let opacity = |s: &Session| s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().opacity;
        s.execute("edit.undo", json!({})).unwrap();
        assert!(visible(&s, id) && (opacity(&s) - 1.0).abs() < 1e-6, "undo shows the layer again and restores its opacity");
        s.execute("edit.redo", json!({})).unwrap();
        assert!(!visible(&s, id) && (opacity(&s) - 0.5).abs() < 1e-6, "redo hides it again");
    }
}
