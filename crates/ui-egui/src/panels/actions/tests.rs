use super::*;
use craft::automation::actions::ActionStep;
use craft::automation::ui::actions::ActionsHost as _;
use egui_kittest::{Harness, kittest::Queryable};

use crate::panels::panel_docking::{apply, is_front, is_shown, show_panel};
use craft::behavior::dock::Action as DockAction;

fn new_app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn step(id: &str, params: Value) -> ActionStep {
    ActionStep::new(id, params)
}

fn push(app: &mut PhotocraftApp, name: &str, set: Option<&str>, steps: Vec<ActionStep>) {
    let mut action = Action::new(name, steps);
    action.set = set.map(str::to_owned);
    app.session.actions.list.push(action).unwrap();
}

fn layer_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.active().unwrap().doc.walk().iter().map(|(_, _, l)| l.name.clone()).filter(|n| n != "Background").collect()
}

fn commands(app: &PhotocraftApp, action: usize) -> Vec<String> {
    app.session.actions.list.action(action).unwrap().steps.iter().map(|s| s.command.clone()).collect()
}

/// Two sets, Web listed first: "Resize" (Web) adds a layer "w", "Sharpen" and "Soften" (Retouch)
/// add "r1" and "r2".
fn two_set_app() -> Box<PhotocraftApp> {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, set, layer) in [("Resize", "Web", "w"), ("Sharpen", "Retouch", "r1"), ("Soften", "Retouch", "r2")] {
        push(&mut app, name, Some(set), vec![step("layer.new.layer", json!({"name": layer}))]);
    }
    app
}

#[test]
fn selected_layer_name_is_visible_during_recording_and_after_stop() {
    let mut app = new_app();
    app.run("file.new", json!({"width":4,"height":4})).unwrap();
    app.run("layer.setProps", json!({"name":"Background"})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("actions.record", json!({"name":"Select photo"})).unwrap();
    app.ui.actions.panel.expanded.insert("Select photo".into());
    app.run("layer.select", json!({"layer":layer.0})).unwrap();
    {
        let host = PhotoActions::new(&app.session, false);
        let pending = host.pending_steps(0);
        assert_eq!(pending.len(), 1);
        assert_eq!(host.step_label(&pending[0]), "Select Layer (Background)");
        assert_eq!(host.step_label(&step("layer.select", json!({"layer":26}))), "Select Layer (ID 26)");
    }
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(400.0, 260.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(4);
    h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Select Layer (Background)");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    h.run_steps(4);
    h.get_by_role_and_label(egui::accesskit::Role::TreeItem, "Select Layer (Background)");
}

/// #7646: an action's function key can take Shift and Command, as in Photoshop's Action
/// Options, and only that combination plays it.
#[test]
fn a_function_key_with_shift_and_command_plays_its_action() {
    use egui::{Key, Modifiers};
    let mut app = new_app();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.session
        .actions
        .list
        .push(Action { name: "Action 1".into(), steps: vec![step("layer.new.layer", json!({}))], disabled: Default::default(), ..Default::default() })
        .unwrap();
    let key = photocraft_engine::atn_cmds::function_key_name(2, true, true);
    assert_eq!(key, "Cmd+Shift+F2");
    assign_shortcut(&mut app, "Action 1", &key).unwrap();
    let press = |app: &mut PhotocraftApp, modifiers| crate::canvas::testing::key(app, Key::F2, modifiers);
    let layers = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.len();
    let before = layers(&app);
    press(&mut app, Modifiers::NONE);
    press(&mut app, Modifiers::SHIFT);
    assert_eq!(layers(&app), before, "F2 and Shift+F2 alone don't play it");
    press(&mut app, Modifiers::SHIFT | Modifiers::COMMAND);
    assert_eq!(layers(&app), before + 1, "Cmd+Shift+F2 plays it");
}

/// A function key belongs to one action, as its main key or an alternate, however the key was
/// spelled; other commands keep it.
#[test]
fn assigning_an_action_key_clears_it_only_from_other_actions() {
    let mut app = new_app();
    push(&mut app, "First", None, vec![]);
    push(&mut app, "Second", None, vec![]);
    app.session.edit_prefs(|prefs| {
        prefs.shortcuts.insert(shortcut_id("First"), "f6".into());
        prefs.shortcuts.insert("file.save".into(), "f6".into());
        prefs.alternate_shortcuts.insert(shortcut_id("First"), vec!["F6".into(), "F7".into()]);
        prefs.alternate_shortcuts.insert("file.save".into(), vec!["F6".into(), "F8".into()]);
    });
    assign_shortcut(&mut app, "Second", "F6").unwrap();
    let prefs = app.session.prefs();
    assert_eq!(prefs.shortcuts[&shortcut_id("Second")], "F6");
    assert_eq!(prefs.shortcuts[&shortcut_id("First")], "");
    assert_eq!(prefs.alternate_shortcuts[&shortcut_id("First")], ["F7"]);
    assert_eq!(prefs.shortcuts["file.save"], "f6");
    assert_eq!(prefs.alternate_shortcuts["file.save"], ["F6", "F8"]);
}

/// The panel shows a saved shortcut through the host: the text the engine writes reads back.
#[test]
fn the_host_reads_the_shortcut_text_the_engine_writes() {
    let mut app = new_app();
    push(&mut app, "Keyed", None, vec![]);
    let key = photocraft_engine::atn_cmds::function_key_name(2, true, true);
    assign_shortcut(&mut app, "Keyed", &key).unwrap();
    let action = app.session.actions.list.action(0).unwrap();
    let choice = PhotoActions::new(&app.session, false).shortcut(action);
    assert_eq!(choice, Some(craft::automation::ui::actions::ShortcutChoice::new(2, true, true)));
    assign_shortcut(&mut app, "Keyed", "Alt+F2").unwrap();
    let action = app.session.actions.list.action(0).unwrap();
    assert_eq!(PhotoActions::new(&app.session, false).shortcut(action), None);
}

#[test]
fn step_check_box_excludes_the_step_from_playback_and_batch() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let steps = ["A", "B", "C"].map(|n| step("layer.new.layer", json!({"name": n})));
    push(&mut app, "Three", None, steps.to_vec());
    // PLAN: `ToggleEdit` is non-exhaustive with no constructor outside the framework; the include cell's rule is `set_step_cell`.
    set_step_cell(&mut app, 0, 1, true, false).unwrap();
    let action = app.session.actions.list.action(0).unwrap();
    assert!(action.disabled.contains(&1));
    assert_eq!(action_steps(action).len(), 2, "Batch and droplets get only the checked steps");
    // A cell already at the wanted value runs nothing: the engine command would flip it back.
    set_step_cell(&mut app, 0, 1, true, false).unwrap();
    assert!(app.session.actions.list.action(0).unwrap().disabled.contains(&1));
    apply_actions_events(&mut app, vec![ActionsEvent::play(0, 0)]);
    assert_eq!(layer_names(&app), ["A", "C"]);
}

#[test]
fn reordering_moves_a_step_within_its_action_and_an_action_into_a_set() {
    let mut app = new_app();
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    let steps = ["view.zoomOut", "view.fitOnScreen", "view.actualPixels"].map(|id| step(id, json!({})));
    push(&mut app, "First", None, steps.to_vec());
    push(&mut app, "Second", None, vec![]);
    // The place's index counts the steps left after the removal: after zoomOut left, 2 is the end.
    // PLAN: `ActionMove` is non-exhaustive with no constructor outside the framework; `reorder` takes its two fields.
    reorder(&mut app, ActionSelection::Step(0, 0), ActionPlace::step(0, 2)).unwrap();
    assert_eq!(commands(&app, 0), ["view.fitOnScreen", "view.actualPixels", "view.zoomOut"]);
    // A step has no place in another action.
    reorder(&mut app, ActionSelection::Step(0, 0), ActionPlace::step(1, 0)).unwrap();
    assert_eq!(commands(&app, 0).len(), 3);
    assert!(commands(&app, 1).is_empty());
    reorder(&mut app, ActionSelection::Action(1), ActionPlace::action(0, Some("Web".into()))).unwrap();
    let list = &app.session.actions.list;
    assert_eq!((list.action(0).unwrap().name.as_str(), list.set_of(0)), ("Second", Some("Web")));
    assert_eq!((list.action(1).unwrap().name.as_str(), list.set_of(1)), ("First", None));
}

#[test]
fn set_folders_group_actions_and_collapse() {
    let mut app = new_app();
    push(&mut app, "Loose", None, vec![]);
    push(&mut app, "Sharpen", Some("Retouch"), vec![step("filter.sharpen.sharpen", json!({}))]);
    push(&mut app, "Also loose", None, vec![]);
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    let first = |h: &Harness<'_, Box<PhotocraftApp>>, label: &str| h.get_all_by_label(label).next().expect(label).rect();
    let folder = first(&h, "Retouch");
    assert!(first(&h, "Loose").top() < folder.top());
    assert!(first(&h, "Sharpen").top() > folder.top());
    // A click selects the set; Left closes it, as the framework list's keys do.
    h.get_all_by_label("Retouch").next().expect("the set folder").click();
    h.run_steps(2);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(3);
    assert_eq!(h.state().ui.actions.panel.settings.collapsed_sets, ["Retouch"]);
    assert!(h.query_by_label("Sharpen").is_none());
    // Button Mode and the collapsed sets survive a save and load; selection does not.
    let saved = serde_json::to_value(&h.state().ui.actions).unwrap();
    assert_eq!(saved, json!({"button_mode": false, "collapsed_sets": ["Retouch"]}));
    let restored: ActionsUi = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.panel.settings.collapsed_sets, ["Retouch"]);
    assert!(restored.panel.selected.is_none());
}

/// #10459 fix-up: Play with a set folder selected plays every action of that set in order, not
/// the first action of the whole list.
#[test]
fn play_with_a_set_folder_selected_plays_that_sets_actions() {
    let mut app = two_set_app();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Retouch".into()));
    apply_actions_events(&mut app, vec![ActionsEvent::PlaySet("Retouch".into())]);
    assert_eq!(layer_names(&app), ["r1", "r2"], "the Retouch set played, in order, not Resize");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    // Batch and Create Droplet start on the selected set too, and Play plays the set.
    assert_eq!(selected_action(&app).map(|a| a.name.as_str()), Some("Sharpen"));
    assert_eq!(playback_targets(&app), [1, 2]);
}

/// #10459 fix-up: Play with an empty set selected plays nothing.
#[test]
fn play_with_an_empty_set_selected_plays_nothing() {
    let mut app = two_set_app();
    app.run("actions.newSet", json!({"name": "Print"})).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Print".into()));
    apply_actions_events(&mut app, vec![ActionsEvent::PlaySet("Print".into())]);
    assert!(layer_names(&app).is_empty(), "{:?}", layer_names(&app));
}

/// With an empty set selected, Batch and Create Droplet start on the first action, as
/// `selected_action` documents.
#[test]
fn an_empty_selected_set_falls_back_to_the_first_action() {
    let mut app = two_set_app();
    app.run("actions.newSet", json!({"name": "Print"})).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Print".into()));
    assert_eq!(selected_action(&app).map(|a| a.name.as_str()), Some("Resize"));
}

#[test]
fn new_action_with_no_set_selected_is_in_no_set() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    push(&mut app, "Sharpen", Some("Retouch"), vec![]);
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    assert!(app.ui.actions.panel.selected.is_none());
    apply_actions_events(&mut app, vec![ActionsEvent::begin_recording(None)]);
    app.run("actions.stop", json!({})).unwrap();
    let list = &app.session.actions.list;
    assert_eq!((list.actions().len(), list.action(1).unwrap().name.as_str(), list.set_of(1)), (2, "Action 2", None));
}

/// Closing the Actions panel and opening it again keeps the selected set and the empty sets New
/// Set made, so New Action still records into the selected one.
#[test]
fn reopening_the_actions_panel_keeps_the_selected_set_and_empty_sets() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    push(&mut app, "Sharpen", Some("Retouch"), vec![]);
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    app.run("actions.newSet", json!({"name": "Print"})).unwrap();
    show_panel(&mut app, "actions", Some(true)).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Web".into()));
    // Essentials keeps Actions with History as an icon, whose Window menu row only closes its flyout: close the panel.
    apply(&mut app, vec![DockAction::close("actions".to_owned())]).unwrap();
    assert!(!is_shown(&app, "actions"), "closed");
    show_panel(&mut app, "actions", Some(true)).unwrap();
    assert!(is_shown(&app, "actions") && is_front(&app, "actions"), "open again");
    assert_eq!(app.session.actions.list.sets(), ["Retouch", "Web", "Print"]);
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Set("Web".into())));
    apply_actions_events(&mut app, vec![ActionsEvent::begin_recording(None)]);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list.set_of(1), Some("Web"));
}

/// #7535: with a set's folder row selected and no action, recording makes a new action in that set.
#[test]
fn start_recording_with_a_set_selected_records_a_new_action_into_it() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    push(&mut app, "Sharpen", Some("Retouch"), vec![]);
    app.run("actions.newSet", json!({"name": "Web"})).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Web".into()));
    assert_eq!(new_action_set(&app).as_deref(), Some("Web"));
    apply_actions_events(&mut app, vec![ActionsEvent::begin_recording(None)]);
    app.run("actions.stop", json!({})).unwrap();
    let list = &app.session.actions.list;
    assert_eq!((list.action(1).unwrap().name.as_str(), list.set_of(1)), ("Action 2", Some("Web")));
}

#[test]
fn recording_into_a_collapsed_set_expands_it() {
    let mut app = new_app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    push(&mut app, "Sharpen", Some("Retouch"), vec![]);
    app.ui.actions.panel.settings.collapsed_sets = vec!["Retouch".into()];
    apply_actions_events(&mut app, vec![ActionsEvent::begin_recording(Some(0))]);
    assert!(app.ui.actions.panel.settings.collapsed_sets.is_empty());
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Action(0)));
    assert!(app.ui.actions.panel.expanded.contains("Sharpen"));
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(360.0, 400.0)).build_ui_state(|ui, app| panel(app, ui), app);
    h.run_steps(3);
    assert!(h.query_all_by_label("Sharpen").next().is_some(), "the set shows its action");
    h.state_mut().run("actions.stop", json!({})).unwrap();
}

/// Save Set… on an empty set folder asks for a file named after the set.
#[test]
fn save_set_on_an_empty_set_folder_opens_the_save_dialog() {
    let (mut app, files) = super::files_tests::empty_set_app("Later");
    apply_actions_events(&mut app, vec![ActionsEvent::save(Some("Later".into()))]);
    let requests = files.requests();
    // PLAN: assumed `ScriptedFiles::requests()` items expose `kind` and `file_name: Option<String>`
    assert!(matches!(requests.as_slice(), [r] if format!("{:?}", r.kind).contains("Save") && r.file_name.as_deref() == Some("Later.atn")), "{requests:?}");
}
