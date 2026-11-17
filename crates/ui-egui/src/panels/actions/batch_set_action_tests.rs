//! #13672: the Batch and Create Droplet dialogs' Set dropdown is always shown - it lists the
//! actions outside any set first (shown as "Ungrouped Actions"), then every set in panel order,
//! empty sets included - and the Action dropdown lists only the chosen set's actions, in list
//! order, actions without steps included (the list keeps action names unique across its sets).
//! Changing the set selects that set's first action; OK runs the action chosen there.

use super::*;
use craft::automation::actions::ActionStep;
use craft::ui::dialog::DialogId;

use crate::menus::invoke as menu;

fn app_with(n: usize) -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for i in 0..n {
        app.run("file.new", json!({"width": 100 + i as u32 * 10, "height": 80})).unwrap();
    }
    app.sync_views();
    let ctx = app.ctx().clone();
    (app, ctx)
}

fn push(app: &mut PhotocraftApp, name: &str, set: Option<&str>, steps: Vec<(&'static str, Value)>) {
    let mut action = Action::new(name, steps.into_iter().map(|(id, params)| ActionStep::new(id, params)).collect());
    action.set = set.map(str::to_owned);
    app.session.actions.list.push(action).unwrap();
}

/// One frame of the app on `ctx`.
fn frame(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let mut f = eframe::Frame::_new_kittest();
        app.logic_frame(ui.ctx(), &mut f);
        app.ui_frame(ui, &mut f);
    });
    output.textures_delta.clear();
}

/// Runs a menu command that opens a form and returns the form's dialog, after the frame that
/// shows it: a hosted form loads its remembered values and defaults when it first shows.
fn open_form(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) -> DialogId {
    let reply = menu(app, ctx, id, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog");
    let dialog = app.framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|id| id.value().to_string() == want).expect("the named dialog is open");
    frame(app, ctx);
    dialog
}

/// The values the open dialog `id` lists.
fn fields(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).expect("an open dialog that lists its fields")
}

/// Sets a form value as automation would; `Err` says why the row refused it.
fn try_set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, key: &str, value: Value) -> Result<(), String> {
    app.framework.dialogs.set_json_field(ctx, id, key, value)
}

fn set(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId, key: &str, value: Value) {
    try_set(app, ctx, id, key, value).unwrap_or_else(|e| panic!("{key}: {e}"));
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// The Set dropdown is shown even when the only action is outside any set: it opens on the
/// "Ungrouped Actions" entry, and OK runs that action through it. (#13672 Done-when 1)
#[test]
fn batch_shows_the_set_dropdown_for_an_action_outside_any_set_and_runs_it() {
    let (mut app, ctx) = app_with(1);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png").to_string_lossy().into_owned();
    let output = dir.path().join("out");
    app.run("file.saveACopy", json!({"path": input})).unwrap();
    push(&mut app, "Solo", None, vec![("image.adjustments.invert", json!({}))]);
    let id = open_form(&mut app, &ctx, "file.automate.batch");
    let shown = fields(&app, id);
    assert_eq!((shown["set"].as_str(), shown["action"].as_str()), (Some(""), Some("Solo")), "the ungrouped entry is shown and chosen");
    set(&mut app, &ctx, id, "input", json!(dir.path().to_string_lossy()));
    set(&mut app, &ctx, id, "output", json!(output.to_string_lossy()));
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    assert_eq!(names_in(&output).len(), 1, "the loose action ran through the ungrouped entry");
}

/// The Set dropdown lists every set in panel order: the ungrouped actions first, then the sets
/// of actions, then an empty set; the Action row lists the chosen set's actions, and an empty
/// set has none. (#13672 Done-when 1 and 2)
#[test]
fn batch_set_dropdown_lists_every_set_and_the_action_list_follows_it() {
    let (mut app, _ctx) = app_with(0);
    let ctx = app.ctx().clone();
    push(&mut app, "Solo", None, vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Alpha", Some("A"), vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Beta", Some("A"), vec![("layer.new.layer", json!({}))]);
    push(&mut app, "Gamma", Some("B"), vec![("image.imageRotation.90cw", json!({}))]);
    app.session.actions.list.new_set("Zed").unwrap();
    let id = open_form(&mut app, &ctx, "file.automate.batch");
    assert_eq!(fields(&app, id)["set"], json!(""), "the ungrouped actions are listed first");
    set(&mut app, &ctx, id, "set", json!("A"));
    frame(&mut app, &ctx);
    assert_eq!(fields(&app, id)["action"], "Alpha", "set A's own list starts on its first action");
    set(&mut app, &ctx, id, "set", json!("Zed"));
    frame(&mut app, &ctx);
    let shown = fields(&app, id);
    assert_eq!(&shown["set"], &json!("Zed"), "the empty set is listed like the reference");
    assert!(shown["action"].is_null(), "the empty set offers no action: {}", shown);
    set(&mut app, &ctx, id, "set", json!(""));
    frame(&mut app, &ctx);
    assert_eq!(fields(&app, id)["action"], "Solo", "the ungrouped entry keeps the loose action");
}

/// An action name of another set is no choice of this set: a value that is not in the chosen
/// set's list falls back to that set's first action. The list keeps names unique across its
/// sets, so a repeated name per set is the reference application's case, not ours.
/// (#13672 Done-when 2)
#[test]
fn batch_action_choices_follow_the_set_and_a_name_of_another_set_falls_back() {
    let (mut app, _ctx) = app_with(0);
    let ctx = app.ctx().clone();
    push(&mut app, "Invert", Some("A"), vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Alpha", Some("A"), vec![("layer.new.layer", json!({}))]);
    push(&mut app, "Empty", Some("B"), vec![]);
    push(&mut app, "Turn", Some("B"), vec![("image.imageRotation.90cw", json!({}))]);
    app.ui.actions.panel.selected = Some(craft::automation::ui::actions::ActionSelection::Action(0));
    let id = open_form(&mut app, &ctx, "file.automate.batch");
    assert_eq!((fields(&app, id)["set"].as_str(), fields(&app, id)["action"].as_str()), (Some("A"), Some("Invert")), "A's Invert opens chosen");
    set(&mut app, &ctx, id, "set", json!("B"));
    frame(&mut app, &ctx);
    assert_eq!(fields(&app, id)["action"], "Empty", "B starts on its own first action, an action with no steps");
    set(&mut app, &ctx, id, "action", json!("Alpha"));
    frame(&mut app, &ctx);
    assert_eq!(fields(&app, id)["action"], "Empty", "A's Alpha is no choice while set B is picked");
}

/// After changing the set, OK runs the new set's chosen action, not the old set's: the command
/// takes the set and the name together and runs that pair. (#13672 Done-when 2, by position)
#[test]
fn batch_runs_the_first_action_of_the_set_switched_to_in_the_form() {
    let (mut app, ctx) = app_with(1);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.png").to_string_lossy().into_owned();
    let output = dir.path().join("out");
    app.run("file.saveACopy", json!({"path": input})).unwrap();
    push(&mut app, "Alpha", Some("A"), vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Gamma", Some("B"), vec![("image.imageRotation.90cw", json!({}))]);
    app.ui.actions.panel.selected = Some(craft::automation::ui::actions::ActionSelection::Action(0));
    let id = open_form(&mut app, &ctx, "file.automate.batch");
    set(&mut app, &ctx, id, "input", json!(dir.path().to_string_lossy()));
    set(&mut app, &ctx, id, "output", json!(output.to_string_lossy()));
    set(&mut app, &ctx, id, "set", json!("B"));
    frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app, &ctx, id).unwrap();
    let written = names_in(&output);
    assert_eq!(written.len(), 1, "the batch ran: {}", app.ui.status);
    let image = photocraft_io::import("out.png", &std::fs::read(output.join(&written[0])).unwrap()).unwrap().document;
    assert_eq!((image.size.width, image.size.height), (80, 100), "set B's rotation ran, not set A's Invert");
}

/// Not measured (#13672): the first-open state stays the Actions panel's selection. With sets
/// loaded, the dialog opens on the selected action's set and on that action's name.
#[test]
fn batch_starts_on_the_panel_selection_and_offers_no_ungrouped_entry_without_loose_actions() {
    let (mut app, _ctx) = app_with(0);
    let ctx = app.ctx().clone();
    push(&mut app, "Alpha", Some("A"), vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Gamma", Some("B"), vec![("image.imageRotation.90cw", json!({}))]);
    app.ui.actions.panel.selected = Some(craft::automation::ui::actions::ActionSelection::Action(1));
    let id = open_form(&mut app, &ctx, "file.automate.batch");
    let shown = fields(&app, id);
    assert_eq!((&shown["set"], &shown["action"]), (&json!("B"), &json!("Gamma")), "the panel's selection opens");
    assert!(try_set(&mut app, &ctx, id, "set", json!("")).is_err(), "no ungrouped entry while every action has a set");
}

/// The dropdown's data, in panel order: ungrouped first, then the sets of actions in order of
/// first appearance, then an empty set; each entry names its own actions in list order, an
/// action with no steps included. (#13672 Done-when 1 and 2)
#[test]
fn action_form_choices_lists_the_sets_in_panel_order_with_their_own_actions() {
    let (mut app, _ctx) = app_with(0);
    push(&mut app, "Solo", None, vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Invert", Some("A"), vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Empty", Some("A"), vec![]);
    push(&mut app, "Turn", Some("B"), vec![("image.imageRotation.90cw", json!({}))]);
    app.session.actions.list.new_set("Zed").unwrap();
    let choices = action_form_choices(&app);
    let listed: Vec<(&str, Vec<&str>)> = choices.iter().map(|entry| (entry.id.as_str(), entry.actions.iter().map(String::as_str).collect())).collect();
    assert_eq!(listed, [("", vec!["Solo"]), ("A", vec!["Invert", "Empty"]), ("B", vec!["Turn"]), ("Zed", vec![])]);
}

/// Create Droplet shares the dropdowns (#13672 Done-when 4): the Set row is there for a loose
/// action and the Action row follows the set.
#[test]
fn droplet_offers_the_same_set_and_action_dropdowns() {
    let (mut app, _ctx) = app_with(0);
    let ctx = app.ctx().clone();
    push(&mut app, "Solo", None, vec![("image.adjustments.invert", json!({}))]);
    push(&mut app, "Alpha", Some("A"), vec![("layer.new.layer", json!({}))]);
    let id = crate::dialogs::file_forms::open_create_droplet(&mut app).unwrap();
    frame(&mut app, &ctx);
    let shown = fields(&app, id);
    assert_eq!((shown["set"].as_str(), shown["action"].as_str()), (Some(""), Some("Solo")), "the droplet shows the ungrouped entry too");
    set(&mut app, &ctx, id, "set", json!("A"));
    frame(&mut app, &ctx);
    assert_eq!(fields(&app, id)["action"], "Alpha", "the droplet's Action row follows its Set row");
}
