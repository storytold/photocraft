use photocraft_engine::actions_cmds::ActionsExt;
use std::sync::Arc;

use craft::host::{PickKind, Picked, ScriptedFiles};
use serde_json::json;

use super::ActionSelection;
use super::files;
use crate::PhotocraftApp;

/// A small valid Photoshop set, generated without external action files.
pub(super) fn action_set_bytes(set: &str, action: &str) -> Vec<u8> {
    use craft_psd::atn::{AtnAction, AtnEvent, AtnFile, AtnItem};
    use craft_psd::descriptor::UnicodeString;
    craft_psd::atn::write(&AtnFile {
        name: UnicodeString::new_nul(set),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul(action),
            items: vec![AtnItem { event: AtnEvent::Code(*b"Invr"), dictionary: "Invert".into(), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// Delivers the answer of the pick the last call asked for.
fn settle(app: &mut PhotocraftApp) {
    app.poll_file_dialog();
}

fn names(app: &PhotocraftApp) -> Vec<&str> {
    app.session.actions().list.actions().iter().map(|a| a.name.as_str()).collect()
}

/// The file names the Save requests of `files` suggested.
fn suggested(files: &ScriptedFiles) -> Vec<String> {
    files.requests().into_iter().filter(|r| r.kind == PickKind::Save).filter_map(|r| r.file_name).collect()
}

/// An app with no actions and one empty set New Set made, whose Save dialog the fake cancels.
pub(super) fn empty_set_app(set: &str) -> (Box<PhotocraftApp>, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::answering([Picked::Cancelled]));
    let mut app = super::runner_tests::disk_app(files.clone());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("actions.newSet", json!({"name": set})).unwrap();
    (app, files)
}

/// Save Actions… for the empty set `set`, selected in the panel; the suggested names asked.
fn save_suggestions(set: &str) -> Vec<String> {
    let (mut app, files) = empty_set_app(set);
    app.ui.actions.panel.selected = Some(ActionSelection::Set(set.into()));
    files::save_dialog(&mut app, None).unwrap();
    settle(&mut app);
    suggested(&files)
}

/// Load `files` (name, bytes) in one Load Actions… selection; the app and the status bar text.
fn bulk_load_status(files: Vec<(String, Vec<u8>)>) -> (Box<PhotocraftApp>, String) {
    let fake = Arc::new(ScriptedFiles::answering([Picked::Contents(files)]));
    let mut app = super::runner_tests::disk_app(fake);
    files::load_dialog(&mut app).unwrap();
    settle(&mut app);
    let status = app.ui.status.clone();
    (app, status)
}

#[test]
/// Browser multi-selection must not be truncated to the first file.
fn bulk_load_imports_every_chosen_action_file() {
    let chosen = vec![("first.atn".into(), action_set_bytes("First set", "First")), ("second.atn".into(), action_set_bytes("Second set", "Second"))];
    let fake = Arc::new(ScriptedFiles::answering([Picked::Contents(chosen)]));
    let mut app = super::runner_tests::disk_app(fake.clone());
    files::load_dialog(&mut app).unwrap();
    settle(&mut app);
    assert_eq!(names(&app), ["First", "Second"]);
    let asked = fake.requests();
    assert!(matches!(asked.as_slice(), [r] if r.kind == PickKind::OpenMany && r.filters.len() == 1 && r.filters[0].extensions == ["atn"]), "{asked:?}");
    assert!(!app.file_dialog_open());
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

#[test]
/// A corrupt middle file cannot hide warnings or prevent a later set replacement.
fn bulk_load_keeps_selection_order_and_earlier_warnings_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("z-first.atn");
    let broken = dir.path().join("broken.atn");
    let last = dir.path().join("a-last.atn");
    let mut unsupported = craft_psd::atn::parse(&action_set_bytes("Same set", "Old")).unwrap();
    unsupported.actions[0].items[0].event = craft_psd::atn::AtnEvent::Code(*b"Xxxx");
    std::fs::write(&first, craft_psd::atn::write(&unsupported).unwrap()).unwrap();
    std::fs::write(&broken, b"not an action file").unwrap();
    std::fs::write(&last, action_set_bytes("Same set", "Replacement")).unwrap();
    let paths = [first, broken, last].to_vec();
    let fake = Arc::new(ScriptedFiles::answering([Picked::Paths(paths)]));
    let mut app = super::runner_tests::disk_app(fake);
    files::load_dialog(&mut app).unwrap();
    settle(&mut app);
    assert_eq!(names(&app), ["Replacement"]);
    assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Action(0)));
    assert!(app.ui.status_error, "a later success must not hide an earlier failure");
    // The replaced set's unsupported step is gone, so it is not counted.
    for part in ["2 action files loaded", "1 action;", "Same set from a-last.atn replaced the one from z-first.atn", "1 file could not be loaded", "broken.atn"]
    {
        assert!(app.ui.status.contains(part), "{part}: {}", app.ui.status);
    }
    assert!(!app.ui.status.contains("PhotoCraft equivalent"), "{}", app.ui.status);
}

#[test]
/// Two files whose sets share a name: the later replaces the earlier, and the summary counts only
/// what the panel holds afterwards and says which set was replaced.
fn bulk_load_summary_counts_a_replaced_set_once() {
    let mut a = craft_psd::atn::parse(&action_set_bytes("Set 1", "A")).unwrap();
    a.actions[0].items[0].event = craft_psd::atn::AtnEvent::Code(*b"Xxxx");
    let chosen = vec![("a.atn".into(), craft_psd::atn::write(&a).unwrap()), ("b.atn".into(), action_set_bytes("Set 1", "B"))];
    let (app, status) = bulk_load_status(chosen);
    assert_eq!(names(&app), ["B"]);
    assert!(!app.ui.status_error, "{status}");
    assert_eq!(status, "2 action files loaded (1 action; Set 1 from b.atn replaced the one from a.atn)");
}

#[test]
/// Distinct set names: every set counts, nothing is reported as replaced, and the file count reads
/// right for one loaded file as well as several.
fn bulk_load_summary_counts_distinct_sets_and_pluralises_files() {
    let chosen = vec![("a.atn".into(), action_set_bytes("Set A", "A")), ("b.atn".into(), action_set_bytes("Set B", "B"))];
    assert_eq!(bulk_load_status(chosen).1, "2 action files loaded (2 actions)");
    let chosen = vec![("c.atn".into(), action_set_bytes("Set C", "C")), ("bad.atn".into(), b"not an action file".to_vec())];
    let (app, status) = bulk_load_status(chosen);
    assert!(app.ui.status_error);
    assert!(status.starts_with("1 action file loaded (1 action; 1 file could not be loaded: bad.atn: "), "{status}");
}

#[test]
/// Dismissing the picker is inert; one selected file keeps its existing status message.
fn bulk_load_keeps_cancel_empty_and_single_file_behavior() {
    for answer in [Picked::Cancelled, Picked::Paths(Vec::new()), Picked::Contents(Vec::new())] {
        let fake = Arc::new(ScriptedFiles::answering([answer]));
        let mut app = super::runner_tests::disk_app(fake);
        app.session.actions_mut().list.new_action("Existing", None).unwrap();
        app.ui.actions.panel.selected = Some(ActionSelection::Action(0));
        app.ui.status = "Unchanged".into();
        files::load_dialog(&mut app).unwrap();
        settle(&mut app);
        assert_eq!(names(&app), ["Existing"]);
        assert_eq!(app.ui.actions.panel.selected, Some(ActionSelection::Action(0)));
        assert_eq!(app.ui.status, "Unchanged");
    }
    let fake = Arc::new(ScriptedFiles::answering([Picked::Contents(vec![("single.atn".into(), action_set_bytes("Single", "One"))])]));
    let mut app = super::runner_tests::disk_app(fake);
    files::load_dialog(&mut app).unwrap();
    settle(&mut app);
    assert_eq!(app.session.actions().list.actions()[0].name, "One");
    assert!(app.ui.status.contains("single.atn"));
    assert!(!app.ui.status_error);
}

#[test]
/// The row is grey while a recording runs, and a keyboard or control-channel route must not open
/// the picker anyway: `actions.load` refuses a recording, so the panel says so before asking for a
/// file instead of after the person has chosen one.
fn load_dialog_refuses_a_running_recording_before_asking_for_a_file() {
    let fake = Arc::new(ScriptedFiles::answering([Picked::Contents(vec![("set.atn".into(), action_set_bytes("Loaded", "Incoming"))])]));
    let mut app = super::runner_tests::disk_app(fake.clone());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.run("actions.record", json!({"name": "Rec"})).unwrap();
    let error = files::load_dialog(&mut app).unwrap_err();
    settle(&mut app);
    assert!(fake.requests().is_empty(), "no file was asked for: {error}");
    assert!(!app.file_dialog_open(), "the picker stays closed: {error}");
    assert_eq!(names(&app), ["Rec"], "the recording keeps its action: {error}");
    app.run("actions.stop", json!({})).unwrap();
    files::load_dialog(&mut app).unwrap();
    settle(&mut app);
    assert_eq!(names(&app), ["Rec", "Incoming"], "idle: the same path loads");
}

/// Save Actions… after a click on set B's folder row saves set B, not the first action's set.
#[test]
fn save_actions_with_a_set_selected_saves_that_set() {
    let fake = Arc::new(ScriptedFiles::answering([Picked::Cancelled]));
    let mut app = super::runner_tests::disk_app(fake.clone());
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    for (name, set) in [("Sharpen", "Retouch"), ("Resize", "Web")] {
        app.session.actions_mut().list.new_action(name, Some(set.into())).unwrap();
    }
    app.ui.actions.panel.selected = Some(ActionSelection::Set("Web".into()));
    files::save_dialog(&mut app, None).unwrap();
    settle(&mut app);
    assert_eq!(suggested(&fake), ["Web.atn"], "{:?}", fake.requests());
}

/// FredApps review on #10459: a set name can hold characters a file name can't. Save Actions…
/// suggests the file with them replaced, and the set keeps its name in the file. The set here
/// is an empty one New Set made, which Save Actions… writes as Photoshop does.
#[test]
fn save_actions_suggests_a_file_name_without_path_characters() {
    let set = "Web/Print\\Mail: v2";
    let (mut app, files) = empty_set_app(set);
    app.session.actions_mut().list.new_action("Loose", None).unwrap();
    app.ui.actions.panel.selected = Some(ActionSelection::Set(set.into()));
    files::save_dialog(&mut app, None).unwrap();
    settle(&mut app);
    assert_eq!(suggested(&files), ["Web_Print_Mail_ v2.atn"], "{:?}", files.requests());
    let saved = app.run("actions.save", json!({"set": set})).unwrap();
    assert_eq!(saved["set"], set, "the file keeps the set's name");
}

/// A selected empty set with no action at all still saves (the engine saves it).
#[test]
fn save_actions_saves_a_selected_empty_set_with_no_actions() {
    assert_eq!(save_suggestions("Later"), ["Later.atn"]);
}

/// CodeRabbit review on #10459: a set named after a Windows device (`CON`, `com1.v2`) or holding a
/// control character gets a suggested file name Windows can create.
#[test]
fn save_actions_suggests_a_windows_safe_name_for_device_and_control_character_sets() {
    for (set, expected) in [("CON", "CON_.atn"), ("com1.v2", "com1_.v2.atn"), ("Web\u{1}Print\tMail", "Web_Print_Mail.atn"), ("Print", "Print.atn")] {
        assert_eq!(save_suggestions(set), [expected], "{set:?}");
    }
}

/// CodeRabbit on batch 11: a set name longer than a file name may be is cut to 255 bytes with its
/// suffix, at a character boundary.
#[test]
fn save_actions_suggests_a_file_name_of_at_most_255_bytes() {
    let set = "é".repeat(200);
    assert_eq!(save_suggestions(&set), [format!("{}.atn", "é".repeat(125))]);
}

#[test]
/// An unsupported step in a set that stays loaded is counted in the multi-file summary, unlike
/// one in a set a later file replaced (CodeRabbit on #10776).
fn bulk_load_summary_counts_unsupported_steps_of_surviving_sets() {
    let mut a = craft_psd::atn::parse(&action_set_bytes("Set A", "A")).unwrap();
    a.actions[0].items[0].event = craft_psd::atn::AtnEvent::Code(*b"Xxxx");
    let chosen = vec![("a.atn".into(), craft_psd::atn::write(&a).unwrap()), ("b.atn".into(), action_set_bytes("Set B", "B"))];
    let (app, status) = bulk_load_status(chosen);
    assert!(status.contains("1 step has no PhotoCraft equivalent yet"), "{status}");
    assert!(!app.ui.status_error, "{status}");
    // Both sets stay loaded: the unsupported step is counted for one that is in the panel.
    assert_eq!(names(&app), ["A", "B"]);
    assert!(app.session.actions().list.actions()[0].steps.iter().any(|step| step.command == "actions.photoshopStep"));
}
