//! The History panel: the framework's `HistoryPanel` over the active document's snapshots and
//! states. This module builds its rows and maps its events to engine commands.

use craft::behavior::IconId;
use craft::ui::patterns::panels::{HistoryEvent, HistoryFooter, HistoryKey, HistoryPanel, HistoryRow};
use craft::ui::primitives::textures::{ThumbSource, ThumbnailOptions, Thumbnails};

use super::{HistorySource, PhotocraftApp, json};
use crate::app::thumbs::SnapshotThumbs;

pub(super) fn history(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    forget_stale_selection(app);
    let reveal = take(app);
    let thumbnails = Thumbnails::<u64>::in_context(&ctx, "photocraft.history", ThumbnailOptions::new());
    let events = {
        let shown: &PhotocraftApp = app;
        let st = shown.session.active();
        let rows = st.map(rows).unwrap_or_default();
        let thumbs = st.map(|st| SnapshotThumbs::new(shown, st.doc.id));
        let mut panel =
            HistoryPanel::new(&rows, current_key(shown)).source(st.and_then(source_key)).can_delete(|key| can_delete(shown, key)).footer(footer(shown));
        if let Some(key) = reveal {
            panel = panel.reveal(key);
        }
        if let Some(thumbs) = &thumbs {
            panel = panel.thumbnails(&thumbnails, thumbs as &dyn ThumbSource<u64>);
        }
        // The panel reports what this pass's input did; a second pass of the frame gets none.
        panel.show(ui).events
    };
    for event in events {
        apply(app, event);
    }
}

/// The panel's rows: the snapshots first (the first is the document as opened), then the states
/// oldest to newest ending with the current one, then the redo states, next redo first. A state's key is
/// its row id, which stays when the states before it go.
pub(crate) fn rows(st: &photocraft_engine::DocState) -> Vec<HistoryRow> {
    let mut rows: Vec<HistoryRow> = st.snapshots.iter().map(|snap| HistoryRow::snapshot(snap.id, snap.name.clone())).collect();
    let past = st.history.past_len();
    let labels = st.history.entries().into_iter().chain(st.history.redo_labels().map(str::to_string));
    for (i, label) in labels.enumerate() {
        let Some(id) = st.history.row_id(i) else { continue };
        let icon = step_icon(&label);
        rows.push(HistoryRow::state(id, label).icon(icon).redo(i > past));
    }
    rows
}

/// The row shown selected: the selected snapshot, else the current state (Photoshop highlights one row).
fn current_key(app: &PhotocraftApp) -> Option<HistoryKey> {
    if let Some(id) = selected_snapshot(app) {
        return Some(HistoryKey::Snapshot(id));
    }
    let st = app.session.active()?;
    st.history.row_id(st.history.past_len()).map(HistoryKey::State)
}

/// The row that is the History Brush's source.
fn source_key(st: &photocraft_engine::DocState) -> Option<HistoryKey> {
    match photocraft_engine::history_source_cmds::source(st)? {
        HistorySource::State(row) => st.history.row_id(row).map(HistoryKey::State),
        HistorySource::Snapshot(id) => Some(HistoryKey::Snapshot(id)),
    }
}

/// Which rows the trash takes (#5324): a snapshot always; a state never the oldest; a later (redo)
/// state only with Allow Non-Linear History, when the engine offers it.
pub(crate) fn can_delete(app: &PhotocraftApp, key: &HistoryKey) -> bool {
    let Some(st) = app.session.active() else { return false };
    match *key {
        HistoryKey::Snapshot(id) => st.snapshots.iter().any(|snap| snap.id == id),
        HistoryKey::State(id) => match st.history.row_of(id) {
            None | Some(0) => false,
            Some(row) if row > st.history.past_len() => st.history.non_linear && app.session.is_enabled("history.deleteState"),
            Some(_) => true,
        },
        _ => false,
    }
}

/// The footer buttons: with a snapshot selected the trash deletes it; otherwise it needs a past state.
fn footer(app: &PhotocraftApp) -> HistoryFooter {
    let Some(st) = app.session.active() else { return HistoryFooter::default() };
    HistoryFooter::default()
        .delete(selected_snapshot(app).is_some() || st.history.past_len() > 0)
        .new_snapshot(app.session.is_enabled("history.newSnapshot"))
        .new_document(app.session.is_enabled("history.newDocument"))
}

/// The active document and where its history stands at one moment: what [`file`] compares across a
/// command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryMark {
    document: Option<photocraft_doc::DocId>,
    spot: Option<HistorySpot>,
}

/// The active document and its history spot now.
pub(crate) fn mark(app: &PhotocraftApp) -> HistoryMark {
    let st = app.session.active();
    HistoryMark { document: st.map(|st| st.doc.id), spot: st.map(history_spot) }
}

/// Called after a command ran, with the [`mark`] taken before it. When the history moved (a new step,
/// Undo, a state row) or another document came up, files the current state's row for the panel's next
/// draw, which scrolls it into view only if it is hidden (#5909). Other changes (a layer click, a
/// channel eye) and a selected snapshot leave the list where the user put it. The command dispatch
/// calls this beside `layer_reveal::file`.
pub(crate) fn file(app: &mut PhotocraftApp, before: HistoryMark) {
    if before != mark(app) && selected_snapshot(app).is_none() {
        app.ui.history_reveal = current_key(app);
    }
}

/// The pending reveal, for the panel's draw. The first pass of a frame takes it, so a second pass in
/// the same frame reveals nothing again, and a hidden panel reveals when it is shown again.
pub(crate) fn take(app: &mut PhotocraftApp) -> Option<HistoryKey> {
    app.ui.history_reveal.take()
}

/// Runs what the user did in the panel as engine commands. Rows that move the document and the footer's
/// buttons wait while Free Transform is open: it owns Undo (`transform_tool::intercept`), and stepping
/// the history under its box would leave it transforming pixels that changed.
pub(crate) fn apply(app: &mut PhotocraftApp, event: HistoryEvent) {
    let Some(st) = app.session.active() else { return };
    let waits = app.ui.transform.is_some();
    let selected = selected_snapshot(app);
    let past = st.history.past_len();
    let mut snapshot_picked = None;
    let (cmd, params) = match event {
        HistoryEvent::Select(HistoryKey::State(id)) => {
            let Some(row) = st.history.row_of(id) else { return };
            if row == past {
                // The current state is already the document: clicking it only selects it.
                if selected.is_some() {
                    app.ui.history_snapshot = None;
                }
                return;
            }
            if waits {
                return;
            }
            ("history.selectState", json!({"state": row}))
        }
        HistoryEvent::Select(HistoryKey::Snapshot(id)) => {
            if waits {
                return;
            }
            snapshot_picked = Some(id);
            ("history.selectSnapshot", json!({"snapshot": id}))
        }
        HistoryEvent::SetSource(HistoryKey::State(id)) => {
            let Some(row) = st.history.row_of(id) else { return };
            ("history.setSource", json!({"state": row}))
        }
        HistoryEvent::SetSource(HistoryKey::Snapshot(id)) => ("history.setSource", json!({"snapshot": id})),
        HistoryEvent::Rename { snapshot, text, .. } => ("history.renameSnapshot", json!({"snapshot": snapshot, "name": text})),
        // With a snapshot selected the trash deletes it, as in Photoshop (#5324); otherwise it deletes
        // the current state, never the oldest.
        HistoryEvent::Delete(None) => {
            if waits {
                return;
            }
            match selected {
                Some(id) => ("history.deleteSnapshot", json!({"snapshot": id})),
                None if past > 0 => ("history.deleteState", json!({})),
                None => return,
            }
        }
        HistoryEvent::Delete(Some(key)) => {
            if waits || !can_delete(app, &key) {
                return;
            }
            match key {
                HistoryKey::Snapshot(id) => ("history.deleteSnapshot", json!({"snapshot": id})),
                HistoryKey::State(id) => {
                    let Some(row) = st.history.row_of(id) else { return };
                    ("history.deleteState", json!({"state": row}))
                }
                _ => return,
            }
        }
        // ⌥-click opens New Snapshot (name, From) instead, as in Photoshop; with History Options ›
        // Show New Snapshot Dialog by Default a plain click does too.
        HistoryEvent::NewSnapshot { mods, .. } => {
            if waits {
                return;
            }
            if mods.alt || photocraft_engine::history_options_cmds::show_new_snapshot_dialog(&app.session) {
                crate::snapshot_dialog::open(app);
                return;
            }
            ("history.newSnapshot", json!({}))
        }
        HistoryEvent::NewDocument => {
            if waits {
                return;
            }
            ("history.newDocument", json!({}))
        }
        _ => return,
    };
    match app.run(cmd, params) {
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        // The snapshot stays selected while the document is the state the click made.
        Ok(_) => {
            if let Some(id) = snapshot_picked {
                select_snapshot(app, id);
            }
        }
    }
}

/// The icon of a History step, from its name.
pub(crate) fn step_icon(step: &str) -> IconId {
    IconId::new(match step {
        s if s.contains("Brush") || s.contains("Eraser") => "brush",
        // Applying a patch edits pixels; only the Patch tool's lasso is a selection step.
        "Patch Selection" => "adjustments",
        // The Layers panel's controls (layer.setProps).
        "Opacity Change" | "Fill Opacity Change" | "Blending Change" => "layers",
        s if s.contains("Layer") => "layers",
        s if s.contains("Select") || s.contains("Marquee") || s.contains("Deselect") || s.contains("Lasso") => "rect_dashed",
        // The selections the Patch tool draws (select.lasso's historyName).
        "Patch Tool" => "rect_dashed",
        s if s.contains("Fill") => "paint_bucket",
        // The opening state: "Open" for a file, "New" for a document from File › New (#14029).
        "Open" | "New" => "image",
        _ => "adjustments",
    })
}

/// Where the history stood after a snapshot click: the number of past states, the current state's
/// name, the number of redo states and the newest past state. That last one is a new document on
/// every step, also at the state limit, where the counts and a repeated name stay the same.
pub(crate) type HistorySpot = (usize, Option<String>, usize, Option<usize>);

fn history_spot(st: &photocraft_engine::DocState) -> HistorySpot {
    let newest = st.history.past_len().checked_sub(1).and_then(|i| st.history.state(i)).map(|d| std::sync::Arc::as_ptr(&d) as usize);
    (st.history.past_len(), st.history.entries().pop(), st.history.redo_labels().count(), newest)
}

/// Select snapshot `id` at the history's current state, as a click on its row does (#5324).
pub(crate) fn select_snapshot(app: &mut PhotocraftApp, id: u64) {
    let Some(st) = app.session.active() else { return };
    app.ui.history_snapshot = Some((st.doc.id, id, history_spot(st)));
    // The command that made the state filed a reveal before the snapshot was selected; the list stays on the snapshot row.
    app.ui.history_reveal = None;
}

/// The selected snapshot: the one last clicked, while it still exists and the history is still at
/// the state that click made. Any other step (an edit, Undo, a state row) deselects it, as in
/// Photoshop.
pub(crate) fn selected_snapshot(app: &PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let (doc, id, spot) = app.ui.history_snapshot.as_ref()?;
    (*doc == st.doc.id && *spot == history_spot(st) && st.snapshots.iter().any(|snap| snap.id == *id)).then_some(*id)
}

/// Drops a selection that no longer holds, so the history returning to the same spot later does not
/// bring it back.
fn forget_stale_selection(app: &mut PhotocraftApp) {
    if app.ui.history_snapshot.is_some() && selected_snapshot(app).is_none() {
        app.ui.history_snapshot = None;
    }
}
