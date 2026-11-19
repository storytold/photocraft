//! Count trials: the command catalog, the keymap overrides and the keyed command states are built only when what
//! they read changes.

use craft::automation::actions::ActionStep;
use serde_json::json;

use super::{COMMAND_SOURCES, PhotocraftApp};
use crate::canvas::testing;
use crate::state::Tool;

/// The places of the keyed sources in [`COMMAND_SOURCES`].
const TOOL_SOURCE: usize = 1;
const SLICE_SOURCE: usize = 3;
const ARTBOARD_SOURCE: usize = 5;

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

/// The keyed sources that computed their states between `before` and `after`.
fn ran(before: &[Option<usize>], after: &[Option<usize>]) -> Vec<usize> {
    before.iter().zip(after).enumerate().filter(|(_, (was, now))| was.is_some() && was != now).map(|(at, _)| at).collect()
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

    // The tool and the colours as the app saves them: preference edits outside the catalog's inputs.
    let prefs = app.session.prefs.rev();
    crate::tool_memory::select(&mut app, Tool::Brush);
    app.run("tools.swapColors", json!({})).unwrap();
    app.session.edit_prefs(|p| {
        p.selected_tool = json!("Brush");
        p.tool_colors = Default::default();
    });
    frames(&mut app, 2);
    assert_ne!(app.session.prefs.rev(), prefs);
    assert_eq!(builds(&app), (1, 1), "a tool or colour change builds nothing");

    let generation = app.framework.keymap_inputs().1;
    assert_eq!(app.framework.overrides.get("filter.blur.gaussian"), None);
    app.session.edit_prefs(|p| {
        p.shortcuts.insert("filter.blur.gaussian".into(), "Cmd+Alt+G".into());
    });
    frames(&mut app, 2);
    assert_eq!(builds(&app), (1, 2), "a shortcut edit builds the overrides once and leaves the rows");
    assert!(app.framework.overrides.get("filter.blur.gaussian").is_some_and(|keys| !keys.is_empty()), "the overrides carry the new key");
    assert_eq!(app.framework.keymap_inputs().1, generation, "the catalog keeps its generation");

    app.session.actions.list.new_action("Trial", None).unwrap();
    frames(&mut app, 2);
    assert_eq!(builds(&app), (2, 3), "a new action builds the rows and the overrides once");
    assert_eq!(app.framework.keymap_inputs().1, generation + 1, "the catalog takes a new generation");
    assert!(app.framework.catalog_rows.iter().any(|row| row.label == "Trial"), "the action has its row");

    // Edits that keep every action's name leave the rows.
    let list = &mut app.session.actions.list;
    let at = list.actions().iter().position(|action| action.name == "Trial").unwrap();
    list.insert_step(at, 0, ActionStep::new("layer.new", json!({}))).unwrap();
    list.toggle_step(at, 0).unwrap();
    frames(&mut app, 2);
    assert_eq!(builds(&app), (2, 3), "a step edit builds nothing");
    assert_eq!(app.framework.keymap_inputs().1, generation + 1, "the catalog keeps its generation");
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

    let before = keyed_runs(&app);
    app.ui.artboard_tool.adding = true;
    frames(&mut app, 2);
    assert_eq!(ran(&before, &keyed_runs(&app)), [ARTBOARD_SOURCE], "the placing mode");

    let before = keyed_runs(&app);
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    frames(&mut app, 2);
    let after = keyed_runs(&app);
    assert!(ran(&before, &after).contains(&SLICE_SOURCE) && ran(&before, &after).contains(&ARTBOARD_SOURCE), "a document opens");
    let has_doc = |app: &PhotocraftApp| (COMMAND_SOURCES[ARTBOARD_SOURCE].states)(app)[0].1.is_enabled();
    assert!(has_doc(&app));

    let before = keyed_runs(&app);
    app.ui.slices.selected = Some(1);
    frames(&mut app, 2);
    let selected = keyed_runs(&app);
    assert_eq!(ran(&before, &selected), [SLICE_SOURCE], "the selected slice");
    app.session.file_menu.slices_locked = true;
    frames(&mut app, 2);
    assert_eq!(ran(&selected, &keyed_runs(&app)), [SLICE_SOURCE], "the slice lock");
}
