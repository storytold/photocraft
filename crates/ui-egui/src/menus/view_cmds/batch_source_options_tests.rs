//! #13668: the Batch dialog's Source group. Source lists Folder, Import and Opened Files in the
//! reference application's order, with Import greyed (no import plug-in exists) and Opened Files
//! only pickable while a document is open. Folder shows a Choose… button whose folder reads back as
//! a label with its trailing separator, and the four check boxes under it in the measured order, all
//! off by default, sent as `overrideOpen`, `includeSubfolders`, `suppressOpenOptions` and
//! `suppressWarnings`. Opened Files draws none of them and sends none of their values, so the
//! engine's rule that those options need a folder source never fires from the dialog. Ticking
//! Override Action "Open" Commands shows the reference application's message with OK and Don't show
//! again; unticking it shows nothing.

use std::path::MAIN_SEPARATOR;
use std::sync::Arc;

use crate::dialogs::alerts::AlertOnce;
use craft::automation::actions::{Action, ActionStep};
use craft::automation::ui::actions::ActionSelection;
use craft::host::{PickKind, Picked, ScriptedFiles};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{Param, ParamKind};
use egui::accesskit::{Role, Toggled};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const BATCH: &str = "file.automate.batch";
const CHOOSE: &str = "Choose…";
const OVERRIDE: &str = "Override Action \"Open\" Commands";
const SUBFOLDERS: &str = "Include All Subfolders";
const SUPPRESS_OPTIONS: &str = "Suppress File Open Options Dialogs";
const SUPPRESS_PROFILES: &str = "Suppress Color Profile Warnings";
const CAPTIONS: [&str; 4] = [OVERRIDE, SUBFOLDERS, SUPPRESS_OPTIONS, SUPPRESS_PROFILES];
/// The Source options' keys: the folder the run reads, then the four check boxes.
const KEYS: [&str; 5] = ["input", "overrideOpen", "includeSubfolders", "suppressOpenOptions", "suppressWarnings"];
const MESSAGE: &str = "When this option is on, source files will be opened from the source folder only by open steps in the action. If there are no open steps, no files will be opened.";

/// An app with `docs` documents and one selected Invert action; `files` answers the host's pickers.
fn app_with(docs: usize, files: Arc<ScriptedFiles>) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { files: Some(files), ..Default::default() });
    for _ in 0..docs {
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    }
    app.session.actions_mut().list.push(Action::new("Invert", vec![ActionStep::new("image.adjustments.invert", json!({}))])).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
    app.sync_views();
    app
}

/// The same app in a harness that draws the hosted dialogs, so the Source controls can be clicked and
/// read. The window is tall enough to hold the whole Batch form at once.
fn harness(docs: usize, files: Arc<ScriptedFiles>) -> Harness<'static, Box<PhotocraftApp>> {
    let app = app_with(docs, files);
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 1400.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

/// Opens the Batch form through the menu and finds its id, without drawing a frame: a control client's
/// open and its first writes land in one turn, before the form has shown.
fn open_undrawn(h: &mut Harness<'_, Box<PhotocraftApp>>) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, BATCH, json!({})).expect("the Batch form opens");
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    h.state().framework.dialogs.open_dialogs().into_iter().find(|open| open.id.value().to_string() == want).expect("the form is open").id
}

/// Opens the Batch form through the menu and runs the frames that show it.
fn open(h: &mut Harness<'_, Box<PhotocraftApp>>) -> DialogId {
    let id = open_undrawn(h);
    h.run_steps(4);
    id
}

/// The open form's value under `key`, in the shape the command gets it.
fn field(h: &Harness<'_, Box<PhotocraftApp>>, id: DialogId, key: &str) -> Option<Value> {
    h.state().framework.dialogs.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields)?.get(key).cloned()
}

/// Sets a form value the way `ui.set` on the control channel does.
fn set(h: &mut Harness<'_, Box<PhotocraftApp>>, id: DialogId, key: &str, value: Value) -> Result<(), String> {
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, key, value).map_err(|error| error.to_string())
}

/// The Source row as the form builds its rows.
fn source_row(app: &mut PhotocraftApp) -> Param {
    let (params, _) = crate::view_cmds::dialogs::params_for(app, BATCH).expect("the command has a form");
    params.into_iter().find(|param| param.key == "source").expect("a Source row")
}

/// The open Override message, by its `alert_once` tag.
fn override_prompt(app: &PhotocraftApp) -> Option<DialogId> {
    crate::dialogs::alerts::prompt_tagged::<AlertOnce>(app)
}

/// Clicks a Source check box: the pointer moves onto the box first, and the press and the release
/// reach it in the frame after that.
fn click_check(h: &mut Harness<'_, Box<PhotocraftApp>>, caption: &str) {
    h.get_by_role_and_label(Role::CheckBox, caption).hover();
    h.run_steps(1);
    h.get_by_role_and_label(Role::CheckBox, caption).click();
    h.run_steps(2);
}

/// The entries of the Source dropdown, their captions, and which of them take a pick.
#[test]
fn source_lists_folder_import_and_opened_files_with_import_and_opened_files_greyed() {
    for docs in [0usize, 1] {
        let mut h = harness(docs, Arc::new(ScriptedFiles::new()));
        let id = open(&mut h);
        assert_eq!(field(&h, id, "source"), Some(json!("folder")), "{docs} documents: Source starts on Folder");
        let row = source_row(h.state_mut());
        let options: Vec<(&str, String)> = match &row.kind {
            ParamKind::Choice { options, .. } => options.iter().map(|(option, label)| (option.as_str(), label.as_str().to_owned())).collect(),
            other => panic!("Source is a choice row, not {other:?}"),
        };
        assert_eq!(options.iter().map(|(option, _)| *option).collect::<Vec<_>>(), ["folder", "import", "open"], "{docs}");
        assert_eq!(options.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>(), ["Folder", "Import", "Opened Files"], "{docs}");
        assert!(!row.option_enabled("import"), "{docs}: Import is greyed while no import plug-in exists");
        assert_eq!(row.option_enabled("open"), docs > 0, "{docs}: Opened Files takes a pick only while a document is open");
        assert!(set(&mut h, id, "source", json!("import")).is_err(), "{docs}: a greyed entry takes no ui.set either");
        assert_eq!(set(&mut h, id, "source", json!("open")).is_ok(), docs > 0, "{docs}: ui.set follows the same rule");
    }
}

/// Source Folder: the Choose… button heads the group, the four boxes follow it in the measured order,
/// all of them off, and no folder is in the form until one is chosen.
#[test]
fn folder_source_draws_choose_and_the_four_check_boxes_in_the_measured_order() {
    let mut h = harness(0, Arc::new(ScriptedFiles::new()));
    let id = open(&mut h);
    for key in KEYS {
        assert!(field(&h, id, key).is_none(), "{key}: nothing is chosen or ticked when the form opens");
    }
    assert!(h.query_by_label(CHOOSE).is_some(), "Choose… heads the Source options");
    let tops: Vec<f32> = CAPTIONS.iter().map(|caption| h.get_by_label(caption).rect().top()).collect();
    let mut ordered = tops.clone();
    ordered.sort_by(f32::total_cmp);
    assert_eq!(tops, ordered, "the check boxes follow the reference application's order: {CAPTIONS:?}");
    assert!(h.get_by_label(CHOOSE).rect().top() < tops[0], "Choose… sits above the check boxes");
    for caption in CAPTIONS {
        assert_eq!(h.get_by_role_and_label(Role::CheckBox, caption).accesskit_node().toggled(), Some(Toggled::False), "{caption}: off by default");
    }
}

/// Choose… asks the host for a folder titled as the reference application's, and the answer becomes
/// the form's `input` and the label under the button, with the trailing separator.
#[test]
fn choose_asks_for_a_folder_and_its_path_shows_under_it_with_a_trailing_separator() {
    let dir = std::env::temp_dir().join(format!("photocraft-batch-source-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.to_string_lossy().into_owned();
    let shown = format!("{path}{MAIN_SEPARATOR}");
    let files = Arc::new(ScriptedFiles::held());
    let mut h = harness(0, files.clone());
    let id = open(&mut h);
    assert!(h.query_by_label(&shown).is_none(), "before a folder is chosen there is no path label");
    h.get_by_label(CHOOSE).click();
    h.run_steps(2);
    let asked = files.requests();
    assert_eq!(asked.len(), 1, "Choose… asks the host's file service for a folder");
    assert_eq!(asked[0].kind, PickKind::Folder);
    assert_eq!(asked[0].title.as_deref(), Some("Choose a batch folder…"), "the picker carries the reference application's title");
    files.answer(Picked::Paths(vec![dir.clone()]));
    h.run_steps(3);
    assert_eq!(field(&h, id, "input"), Some(json!(path)), "the chosen folder is the form's input");
    assert!(h.query_by_label(&shown).is_some(), "the label under the button shows the path with its trailing separator");
    // Each control has its own row: the label that appears has room of its own and covers nothing.
    let label = h.get_by_label(&shown).rect();
    let first = h.get_by_role_and_label(Role::CheckBox, OVERRIDE).rect();
    assert!(
        label.top() > h.get_by_label(CHOOSE).rect().bottom() && label.bottom() <= first.top() + 1.0,
        "the path label sits between Choose… and the first check box, not over it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The three check boxes the dialog was missing, and Include All Subfolders: a tick lands under the
/// key the engine names, and a box nobody touched sends nothing at all. Override Action "Open"
/// Commands answers a tick with its message, so it is clicked last and its message answered first.
#[test]
fn ticking_the_source_check_boxes_stores_their_ticks_under_the_engine_keys() {
    let mut h = harness(0, Arc::new(ScriptedFiles::new()));
    let id = open(&mut h);
    click_check(&mut h, SUPPRESS_OPTIONS);
    click_check(&mut h, SUPPRESS_PROFILES);
    for key in ["suppressOpenOptions", "suppressWarnings"] {
        assert_eq!(field(&h, id, key), Some(json!(true)), "{key}: the tick goes under the engine's key");
    }
    assert!(field(&h, id, "includeSubfolders").is_none(), "{SUBFOLDERS} sits between them and stays off");
    click_check(&mut h, SUBFOLDERS);
    assert_eq!(field(&h, id, "includeSubfolders"), Some(json!(true)));
    click_check(&mut h, OVERRIDE);
    let shown = override_prompt(h.state()).expect("the Override box answers with its message");
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, shown).unwrap();
    h.run_steps(2);
    assert_eq!(field(&h, id, "overrideOpen"), Some(json!(true)));
    click_check(&mut h, OVERRIDE);
    assert_eq!(field(&h, id, "overrideOpen"), Some(json!(false)), "unticking a box that was ticked sends false");
}

/// `ui.set` on the form fields gives the same values as the same choices made with the mouse.
#[test]
fn driving_the_source_options_over_the_control_channel_gives_the_values_the_mouse_gives() {
    let mut mouse = harness(0, Arc::new(ScriptedFiles::new()));
    let mouse_id = open(&mut mouse);
    set(&mut mouse, mouse_id, "input", json!("/tmp/batch/in")).unwrap();
    click_check(&mut mouse, SUPPRESS_OPTIONS);
    click_check(&mut mouse, SUBFOLDERS);
    mouse.run_steps(2);

    let mut typed = harness(0, Arc::new(ScriptedFiles::new()));
    let typed_id = open(&mut typed);
    set(&mut typed, typed_id, "input", json!("/tmp/batch/in")).unwrap();
    for key in ["suppressOpenOptions", "includeSubfolders"] {
        set(&mut typed, typed_id, key, json!(true)).unwrap();
    }
    typed.run_steps(2);

    for key in KEYS {
        assert_eq!(field(&typed, typed_id, key), field(&mouse, mouse_id, key), "{key}");
    }
    assert_eq!(field(&typed, typed_id, "suppressOpenOptions"), Some(json!(true)));
}

/// Source Opened Files: no Choose…, no path label, no check boxes, and none of their values in what
/// the command gets.
#[test]
fn opened_files_draws_no_source_option_and_sends_none_of_their_values() {
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    let id = open(&mut h);
    set(&mut h, id, "input", json!("/tmp/batch/in")).unwrap();
    for key in &KEYS[1..] {
        set(&mut h, id, key, json!(true)).unwrap();
    }
    h.run_steps(2);
    assert_eq!(field(&h, id, "input"), Some(json!("/tmp/batch/in")), "the values exist with Source Folder");
    for key in &KEYS[1..] {
        assert_eq!(field(&h, id, key), Some(json!(true)), "{key}");
    }
    set(&mut h, id, "source", json!("open")).unwrap();
    h.run_steps(2);
    assert_eq!(field(&h, id, "source"), Some(json!("open")), "Opened Files sends source: open");
    for key in KEYS {
        assert!(field(&h, id, key).is_none(), "{key}: none of the Source options' values goes with Opened Files");
    }
    for caption in std::iter::once(CHOOSE).chain(CAPTIONS) {
        assert!(h.query_by_label(caption).is_none(), "{caption}: hidden with Opened Files");
    }
}

/// The message the reference application shows when Override Action "Open" Commands is ticked, and
/// that unticking shows nothing. Its Don't show again box is kept as a dialog preference.
#[test]
fn ticking_override_open_shows_the_message_and_unticking_shows_nothing() {
    let mut h = harness(0, Arc::new(ScriptedFiles::new()));
    open(&mut h);
    assert!(crate::dialogs::alerts::alerts(h.state()).is_empty(), "an untouched form shows nothing");
    click_check(&mut h, OVERRIDE);
    h.run_steps(2);
    let shown = override_prompt(h.state()).expect("ticking the box shows the message");
    assert!(h.query_by_label(MESSAGE).is_some(), "the reference application's sentence, with OK and Don't show again");
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, shown).unwrap();
    h.run_steps(2);
    assert!(override_prompt(h.state()).is_none(), "OK closes the message");
    click_check(&mut h, OVERRIDE);
    h.run_steps(2);
    assert!(override_prompt(h.state()).is_none(), "unticking shows nothing");
    click_check(&mut h, OVERRIDE);
    h.run_steps(2);
    let again = override_prompt(h.state()).expect("ticking it again shows the message");
    h.state_mut().framework.dialogs.set_json_field(&ctx, again, "skip", json!(true)).unwrap();
    crate::dialogs::confirm(h.state_mut(), &ctx, again).unwrap();
    click_check(&mut h, OVERRIDE);
    click_check(&mut h, OVERRIDE);
    h.run_steps(2);
    assert!(override_prompt(h.state()).is_none(), "Don't show again keeps the message away");
    assert!(crate::dialogs::alerts::dismissed(h.state(), "batch.overrideOpenMessage"), "the answer is kept as a dialog preference");
}

/// The tick reaches the engine under `overrideOpen`: with it on, an action with no Open step fails
/// each input instead of being fed the file; with it off the same run writes its output.
#[test]
fn the_override_open_tick_is_the_command_s_override_open() {
    let dir = std::env::temp_dir().join(format!("photocraft-batch-override-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("in")).unwrap();
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    h.state_mut().run("file.saveACopy", json!({"path": dir.join("in").join("a.png").to_string_lossy()})).unwrap();
    for tick in [false, true] {
        let id = open(&mut h);
        set(&mut h, id, "input", json!(dir.join("in").to_string_lossy())).unwrap();
        set(&mut h, id, "output", json!(dir.join(if tick { "out-on" } else { "out-off" }).to_string_lossy())).unwrap();
        set(&mut h, id, "overrideOpen", json!(tick)).unwrap();
        h.run_steps(2);
        let ctx = h.ctx.clone();
        // A ticked box answers with its message, and the message is a child of the form: an OK aimed at
        // the form waits for that answer, so the run settles the message first.
        if let Some(message) = override_prompt(h.state()) {
            crate::dialogs::confirm(h.state_mut(), &ctx, message).unwrap();
            h.run_steps(2);
        }
        crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
        h.run_steps(4);
        let written = std::fs::read_dir(dir.join(if tick { "out-on" } else { "out-off" })).map(|entries| entries.count()).unwrap_or_default();
        assert_eq!(written, usize::from(!tick), "overrideOpen {tick}: the input the action cannot open is refused, not silently skipped");
    }
    assert!(h.state().ui.status_error, "the refused input is reported in the status bar");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Choosing Opened Files over the control channel takes the Source options out at once: an OK in the
/// same frame, before the form drew again, must not carry them to the engine's rule that they need a
/// folder source.
#[test]
fn switching_the_source_over_the_control_channel_drops_the_options_before_ok_reads_them() {
    let dir = std::env::temp_dir().join(format!("photocraft-batch-open-source-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("in")).unwrap();
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    h.state_mut().run("file.saveACopy", json!({"path": dir.join("in").join("a.png").to_string_lossy()})).unwrap();
    let id = open(&mut h);
    set(&mut h, id, "input", json!(dir.join("in").to_string_lossy())).unwrap();
    set(&mut h, id, "output", json!(dir.join("out").to_string_lossy())).unwrap();
    // Nothing runs a frame between these writes and the OK: the fields are exactly as the last write
    // left them, with no draw to sweep them.
    for key in &KEYS[1..] {
        set(&mut h, id, key, json!(true)).unwrap();
    }
    set(&mut h, id, "source", json!("open")).unwrap();
    let ctx = h.ctx.clone();
    crate::dialogs::confirm(h.state_mut(), &ctx, id).unwrap();
    h.run_steps(4);
    assert!(!h.state().ui.status_error, "the options went with an open source: {:?}", h.state().ui.status.clone());
    assert_eq!(std::fs::read_dir(dir.join("out")).map(|entries| entries.count()).unwrap_or_default(), 1, "the opened document was still written");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder pick Choose… filed for a form that closed is forgotten: the host is not left holding it,
/// and its late answer does not walk into the next form.
#[test]
fn a_pick_filed_for_a_form_that_closed_is_forgotten() {
    let dir = std::env::temp_dir().join(format!("photocraft-batch-late-pick-{}", std::process::id()));
    let files = Arc::new(ScriptedFiles::held());
    let mut h = harness(0, files.clone());
    let id = open(&mut h);
    h.get_by_label(CHOOSE).click();
    h.run_steps(2);
    assert_eq!(files.open(), 1, "Choose… leaves one pick open with the host");
    let ctx = h.ctx.clone();
    assert!(h.state_mut().framework.dialogs.cancel_by_id(&ctx, id), "the form closes");
    h.run_steps(3);
    assert_eq!(files.open(), 0, "the pick of a form that closed is forgotten");
    files.answer(Picked::Paths(vec![dir.clone()]));
    let next = open(&mut h);
    assert!(field(&h, next, "input").is_none(), "its late answer does not reach the next form");
    assert!(h.query_by_label(&format!("{}{MAIN_SEPARATOR}", dir.to_string_lossy())).is_none(), "and no path label appears");
}

/// The Choose… row stays drawn under another Source only to take the Source options' keys out; it takes
/// no value of its own there, so a write between the switch and OK cannot put `input` back.
#[test]
fn the_folder_row_takes_no_write_while_another_source_is_chosen() {
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    let id = open(&mut h);
    set(&mut h, id, "input", json!("/tmp/batch/in")).unwrap();
    set(&mut h, id, "source", json!("open")).unwrap();
    // No frame runs between the switch and this write, so the row's own sweep has had no turn: the
    // write itself is what has to be refused.
    let late = set(&mut h, id, "input", json!("/tmp/batch/other"));
    assert!(late.is_err(), "the Choose… row takes no input while Opened Files is chosen: {late:?}");
    assert!(field(&h, id, "input").is_none(), "and `input` stays out of the form for OK to read");
}

/// A pick Choose… filed is forgotten when Source moves away from Folder, not only when the form closes:
/// its late answer must not walk into the folder the form was given after the way back.
#[test]
fn a_pending_pick_is_forgotten_when_the_source_leaves_folder() {
    let temp = std::env::temp_dir();
    let old = temp.join(format!("photocraft-batch-left-folder-{}-old", std::process::id()));
    let fresh = temp.join(format!("photocraft-batch-left-folder-{}-fresh", std::process::id()));
    let files = Arc::new(ScriptedFiles::held());
    let mut h = harness(1, files.clone());
    let id = open(&mut h);
    h.get_by_label(CHOOSE).click();
    h.run_steps(2);
    assert_eq!(files.open(), 1, "Choose… leaves one pick with the host");
    set(&mut h, id, "source", json!("open")).unwrap();
    h.run_steps(2);
    assert_eq!(files.open(), 0, "the pick of a Source that left Folder is forgotten");
    set(&mut h, id, "source", json!("folder")).unwrap();
    h.run_steps(2);
    set(&mut h, id, "input", json!(fresh.to_string_lossy())).unwrap();
    h.run_steps(2);
    assert!(files.answer(Picked::Paths(vec![old])).is_none(), "the forgotten pick has no answer left to carry");
    h.run_steps(3);
    assert_eq!(field(&h, id, "input"), Some(json!(fresh.to_string_lossy())), "the folder the form holds stays its own");
}

/// The tick a control client writes in the same turn that opened the form is still a tick: the message
/// answers the change from what the form opened with, not the first value the pass happens to read.
#[test]
fn a_tick_written_before_the_form_drew_shows_the_message() {
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    let ctx = h.ctx.clone();
    let id = open_undrawn(&mut h);
    set(&mut h, id, "overrideOpen", json!(true)).unwrap();
    // What a `ui.dialog.set` does after its write, before the form has ever drawn.
    crate::dialogs::take_results(h.state_mut(), &ctx);
    assert!(override_prompt(h.state()).is_some(), "the first write that ticks Override shows the message");
    assert_eq!(field(&h, id, "overrideOpen"), Some(json!(true)), "and the tick stands");
}

/// A closed form's observation is not carried into the next open: the reopened form starts from what it
/// opened with, so its first tick shows the message even though the last form left the same box ticked.
#[test]
fn a_reopened_form_starts_a_new_observation() {
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    let ctx = h.ctx.clone();
    let first = open(&mut h);
    click_check(&mut h, OVERRIDE);
    let shown = override_prompt(h.state()).expect("ticking the box shows the message");
    crate::dialogs::confirm(h.state_mut(), &ctx, shown).unwrap();
    h.run_steps(2);
    assert!(h.state_mut().framework.dialogs.cancel_by_id(&ctx, first), "the form closes with the box ticked");
    h.run_steps(3);
    let again = open_undrawn(&mut h);
    set(&mut h, again, "overrideOpen", json!(true)).unwrap();
    crate::dialogs::take_results(h.state_mut(), &ctx);
    assert!(override_prompt(h.state()).is_some(), "the reopened form's first tick shows the message again");
}

/// The message belongs to the form that ticked its box: an explicit OK by id waits for the message's
/// answer, and a Cancel of the form takes the message with it.
#[test]
fn the_override_message_is_a_child_of_the_form_that_ticked_it() {
    let mut h = harness(1, Arc::new(ScriptedFiles::new()));
    let id = open(&mut h);
    click_check(&mut h, OVERRIDE);
    h.run_steps(2);
    assert!(override_prompt(h.state()).is_some(), "the tick opens the message");
    let ctx = h.ctx.clone();
    assert!(!h.state_mut().framework.dialogs.confirm_by_id(&ctx, id), "the form waits for its message's answer");
    assert!(override_prompt(h.state()).is_some(), "and the message is still there to answer");
    assert!(h.state_mut().framework.dialogs.cancel_by_id(&ctx, id), "Cancel closes the form");
    h.run_steps(2);
    assert!(override_prompt(h.state()).is_none(), "and takes its message with it");
}
