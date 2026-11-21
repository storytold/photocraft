//! The History panel's own items at the top of its panel menu (☰, #2966), in Photoshop's order:
//! Step Forward, Step Backward, New Snapshot…, Delete, Clear History, New Document, History
//! Options…. Every item is an engine command; the dock adds its own rows (Close, Close Tab Group …)
//! after them.

use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandState};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The menu's items: a command id, or `None` for a separator.
pub const ITEMS: &[Option<&str>] = &[
    Some("history.stepForward"),
    Some("history.stepBackward"),
    None,
    // Takes a Full Document snapshot at once; switches to the New Snapshot dialog when that
    // lands (#3567).
    Some("history.newSnapshot"),
    Some("history.deleteState"),
    Some("history.clear"),
    Some("history.newDocument"),
    None,
    // Without params `history.options` opens the History Options dialog (#3535).
    Some("history.options"),
];

/// The command and params an item runs: with a snapshot selected in the panel, Delete removes
/// that snapshot, as the panel's trash does (#5324); otherwise it removes the current state.
fn resolve(cmd: &str, snapshot: Option<u64>) -> (&str, Value) {
    match snapshot {
        Some(id) if cmd == "history.deleteState" => ("history.deleteSnapshot", json!({"snapshot": id})),
        _ => (cmd, json!({})),
    }
}

fn command_state(app: &PhotocraftApp, cmd: &str) -> bool {
    // An open Free Transform owns the history (see `panels::history`), and a drawn crop frame
    // holds its jumps back (`crop_ui::holds_history`). A Pen path being drawn steps back anchor
    // by anchor (`vector_ui::pen_history_enabled`).
    if let Some(on) = crate::vector_ui::pen_history_enabled(app, cmd) {
        return on;
    }
    app.ui.transform.is_none() && !crate::crop_ui::holds_history(app, cmd) && app.session.is_enabled(cmd)
}

/// The History items of the panel menu, as command rows: their labels and chords come from the menu
/// catalogue, their state from [`command_states`].
pub(crate) fn panel_rows(_app: &PhotocraftApp) -> Vec<PanelRow> {
    ITEMS.iter().map(|item| item.map_or(PanelRow::Separator, |id| PanelRow::Command(CommandRef::new(id)))).collect()
}

/// Whether each item can run now.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let snapshot = crate::panels::selected_snapshot(app);
    ITEMS.iter().flatten().map(|&id| (CommandRef::new(id), CommandState::new().enabled(command_state(app, resolve(id, snapshot).0)))).collect()
}

/// What [`command_states`] reads: the rules (the Pen and Crop histories and an open Free Transform among them) and the
/// snapshot selected in the panel.
#[derive(craft::ui::contract::MemoProps)]
struct StatesFacts<'a> {
    rules: crate::menus::facts::RuleFacts<'a>,
    #[memo_props(hash = "a plain model id, a snapshot id and the history spot: plain values")]
    snapshot: &'a Option<(photocraft_doc::DocId, u64, crate::panels::history::HistorySpot)>,
}

/// [`command_states`], derived from [`StatesFacts`].
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    let facts = StatesFacts { rules: crate::menus::facts::RuleFacts::of(app), snapshot: &app.ui.history_snapshot };
    crate::app::framework::derived_states(app, name, &facts, || command_states(app))
}

/// Runs the item `command` and reports whether it is one of this menu's. History Options… opens its
/// dialog, as the menu bar's command does. Step Backward/Forward over a Pen path step it as Edit ›
/// Undo/Redo do (`vector_ui::pen_history_step`).
// PLAN: the sheet's `run_fired(app, cmd)` has no context, but the registry's `PanelEntry::run` and
// `CommandSource::run` (pn.PC1, cx.P1) take one, so this follows pn.PC12 and ignores it.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    if !ITEMS.iter().flatten().any(|&id| id == command.id) {
        return false;
    }
    let snapshot = crate::panels::selected_snapshot(app);
    let (cmd, params) = resolve(&command.id, snapshot);
    let result = crate::history_options_ui::menu(app, cmd, &params)
        .or_else(|| crate::vector_ui::pen_history_step(app, cmd).map(|r| r.map(|_| Value::Null)))
        .unwrap_or_else(|| app.run(cmd, params));
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
        app
    }

    fn on(app: &PhotocraftApp, id: &str) -> bool {
        let states = command_states(app);
        let (_, state) = states.iter().find(|(command, _)| command.id == id).unwrap_or_else(|| panic!("{id} has a state"));
        *state == CommandState::new().enabled(true)
    }

    fn fire(app: &mut PhotocraftApp, id: &str) {
        let app_ctx = app.ctx().clone();
        assert!(run_fired(app, &app_ctx, &CommandRef::new(id)), "{id} is an item of the menu");
    }

    fn states(app: &PhotocraftApp) -> (usize, usize) {
        let st = app.session.active().unwrap();
        (st.history.entries().len(), st.history.redo_labels().count())
    }

    #[test]
    fn items_wait_for_an_open_transform() {
        let mut app = app();
        app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
        for id in ["history.stepBackward", "history.newSnapshot", "history.deleteState", "history.clear", "history.newDocument"] {
            assert!(on(&app, id), "{id} before the transform");
        }
        let app_ctx = app.ctx().clone();
        crate::transform_tool::begin(&mut app, &app_ctx).unwrap();
        for id in ["history.stepBackward", "history.newSnapshot", "history.deleteState", "history.clear", "history.newDocument"] {
            assert!(!on(&app, id), "{id}");
        }
    }

    /// Step Backward after a Pen path was closed gives it back to the Pen, open, rather than
    /// deleting it; Step Forward closes it again (#6619), as Edit › Undo and Redo do.
    #[test]
    fn steps_over_a_finished_pen_path_give_it_back_to_the_pen() {
        use crate::canvas::testing::click;
        let mut app = app();
        app.ui.tool = crate::state::Tool::Pen;
        for at in [[10.0, 10.0], [50.0, 10.0], [50.0, 40.0], [10.0, 10.0]] {
            click(&mut app, at, egui::Modifiers::NONE);
        }
        assert!(app.ui.pen.is_none(), "the path is closed");
        fire(&mut app, "history.stepBackward");
        assert_eq!(app.ui.pen.as_ref().map(|p| p.draft.knots.len()), Some(3), "the open path is back on the Pen");
        assert!(app.session.active().unwrap().doc.work_path.is_none());
        fire(&mut app, "history.stepForward");
        assert!(app.ui.pen.is_none());
        assert!(app.session.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].closed);
    }

    /// A drawn crop frame greys the items that move the document under it (#9285).
    #[test]
    fn jumps_wait_for_a_drawn_crop_frame() {
        let mut app = app();
        app.ui.tool = crate::state::Tool::Crop;
        app.session.execute("history.stepBackward", json!({})).unwrap();
        assert!(on(&app, "history.stepBackward"), "no frame is drawn");
        // A pass first: the tool makes its default frame for the document, then the frame is set.
        crate::canvas::testing::frame(&mut app);
        let frame = craft::behavior::geometry::CropBox::new(craft::behavior::kurbo::Rect::new(4.0, 4.0, 40.0, 30.0), 0.0);
        app.canvas_tools.crop.tool.crop_to(frame);
        assert!(crate::crop_ui::pending(&app), "the Crop tool holds a drawn frame");
        for id in ["history.stepBackward", "history.stepForward", "history.deleteState"] {
            assert!(!on(&app, id), "{id}");
        }
        assert!(on(&app, "history.newSnapshot"), "a snapshot leaves the document as it is");
    }

    /// With a snapshot selected in the panel, Delete removes the snapshot and leaves the states,
    /// as the panel's trash does (#5324); without one it removes the current state.
    #[test]
    fn delete_removes_the_selected_snapshot() {
        let mut app = app();
        app.run("history.newSnapshot", json!({})).unwrap();
        let snaps = |app: &PhotocraftApp| app.session.active().unwrap().snapshots().len();
        let before = snaps(&app);
        let id = app.session.active().unwrap().snapshots().last().unwrap().id;
        crate::panels::select_snapshot(&mut app, id);
        let steps = states(&app);
        fire(&mut app, "history.deleteState");
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(snaps(&app), before - 1, "Delete removed the selected snapshot");
        assert_eq!(states(&app), steps, "and left the states alone");
        fire(&mut app, "history.deleteState");
        assert_eq!(snaps(&app), before - 1, "no snapshot selected now: the snapshots are kept");
        assert_eq!(states(&app).0, steps.0 - 1, "the current state is deleted");
    }

    #[test]
    fn without_a_document_only_history_options_are_enabled() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for id in ITEMS.iter().flatten() {
            // History Options are preferences, not part of a document.
            assert_eq!(on(&app, id), *id == "history.options", "{id}");
        }
    }
}
