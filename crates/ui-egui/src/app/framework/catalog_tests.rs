//! Count trials: the command catalog and the keymap overrides are built only when what they read changes.

use craft::automation::actions::ActionStep;
use photocraft_engine::actions_cmds::ActionsExt;
use serde_json::json;

use super::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    testing::frame(&mut app);
    app
}

/// How many times the rows and the overrides were built.
fn builds(app: &PhotocraftApp) -> (usize, usize) {
    (app.framework.catalog_builds, app.framework.overrides_builds)
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

    app.session.actions_mut().list.new_action("Trial", None).unwrap();
    frames(&mut app, 2);
    assert_eq!(builds(&app), (2, 3), "a new action builds the rows and the overrides once");
    assert_eq!(app.framework.keymap_inputs().1, generation + 1, "the catalog takes a new generation");
    assert!(app.framework.catalog_rows.iter().any(|row| row.label == "Trial"), "the action has its row");

    // Edits that keep every action's name leave the rows.
    let list = &mut app.session.actions_mut().list;
    let at = list.actions().iter().position(|action| action.name == "Trial").unwrap();
    list.insert_step(at, 0, ActionStep::new("layer.new", json!({}))).unwrap();
    list.toggle_step(at, 0).unwrap();
    frames(&mut app, 2);
    assert_eq!(builds(&app), (2, 3), "a step edit builds nothing");
    assert_eq!(app.framework.keymap_inputs().1, generation + 1, "the catalog keeps its generation");
}
