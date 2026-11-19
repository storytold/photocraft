//! Count trials: the command catalog, the keymap overrides and the keyed command states are built only when what
//! they read changes.

use serde_json::json;

use super::{COMMAND_SOURCES, PhotocraftApp};
use crate::canvas::testing;
use crate::state::Tool;

/// The place of the tool commands' source in [`COMMAND_SOURCES`].
const TOOL_SOURCE: usize = 1;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    testing::frame(&mut app);
    app
}

/// How many times the rows and the overrides were built.
fn builds(app: &PhotocraftApp) -> (usize, usize) {
    (app.framework.catalog_builds, app.framework.overrides_builds)
}

/// How many times each keyed command source computed its states, by its place in [`COMMAND_SOURCES`]; `None` for a
/// source that computes them every frame.
fn keyed_runs(app: &PhotocraftApp) -> Vec<Option<usize>> {
    COMMAND_SOURCES.iter().zip(&app.framework.held_states.runs).map(|(source, &runs)| source.key.map(|_| runs)).collect()
}

fn frames(app: &mut PhotocraftApp, n: usize) {
    for _ in 0..n {
        testing::frame(app);
    }
}

#[test]
fn the_catalog_is_built_only_when_its_inputs_change() {
    let mut app = app();
    assert_eq!(builds(&app), (1, 1), "the first frame builds the catalog and the overrides");

    frames(&mut app, 100);
    assert_eq!(builds(&app), (1, 1), "idle frames build nothing");

    let prefs = app.session.prefs.rev();
    crate::tool_memory::select(&mut app, Tool::Brush);
    app.run("tools.swapColors", json!({})).unwrap();
    frames(&mut app, 2);
    assert_ne!(app.session.prefs.rev(), prefs, "the tool and the colours are preferences");
    assert_eq!(builds(&app), (1, 1), "a tool or colour change builds nothing");

    let generation = app.framework.keymap_inputs().1;
    app.session.edit_prefs(|p| {
        p.shortcuts.insert("edit.copy".into(), "Ctrl+Shift+K".into());
    });
    frames(&mut app, 2);
    assert_eq!(builds(&app), (1, 2), "a shortcut edit builds the overrides once and leaves the rows");
    assert_eq!(app.framework.keymap_inputs().1, generation, "the catalog keeps its generation");

    app.session.actions.list.new_action("Trial", None).unwrap();
    frames(&mut app, 2);
    assert_eq!(builds(&app), (2, 3), "a new action builds the rows and the overrides once");
    assert_eq!(app.framework.keymap_inputs().1, generation + 1, "the catalog takes a new generation");
    assert!(app.framework.catalog_rows.iter().any(|row| row.label == "Trial"), "the action has its row");
}

#[test]
fn keyed_command_states_run_only_when_their_key_changes() {
    let mut app = app();
    assert_eq!((COMMAND_SOURCES[TOOL_SOURCE].states)(&app).len(), Tool::ALL.len(), "TOOL_SOURCE names the tool commands");
    let before = keyed_runs(&app);
    assert!(before.iter().flatten().all(|&runs| runs == 1), "the first frame computes every keyed source once");

    frames(&mut app, 100);
    assert_eq!(keyed_runs(&app), before, "idle frames compute no keyed source");

    let other = if app.ui.tool == Tool::Brush { Tool::Move } else { Tool::Brush };
    crate::tool_memory::select(&mut app, other);
    frames(&mut app, 2);
    let after = keyed_runs(&app);
    for (at, (was, now)) in before.iter().zip(&after).enumerate() {
        let moved = usize::from(at == TOOL_SOURCE);
        assert_eq!(now.map(|n| n - was.unwrap_or_default()), was.map(|_| moved), "source {at} after a tool change");
    }
}
