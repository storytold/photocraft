//! Replace Actions exercises the real asynchronous picker and its owned confirmation.
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{EngineError, Session, actions_cmds::Action, atn_cmds};
use photocraft_ui_egui::{FileDialogAnswer, FileDialogReply, FileDialogRequest, PhotocraftApp, Services, actions, dialogs, state::DialogKind};
use serde_json::{Value, json};

type Picker = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// Retain actual picker replies while drawing the menu, picker polling and confirmation.
fn harness() -> (Harness<'static, PhotocraftApp>, Picker) {
    let picker: Picker = Default::default();
    let held = picker.clone();
    let services = Services { file_dialog: Some(Box::new(move |request, _, reply| held.borrow_mut().push((request, reply)))), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    for (name, set) in [("A", "One"), ("B", "Two")] {
        let mut action = Action::new(name, vec![("select.deselect".into(), json!({}))]);
        action.set = Some(set.into());
        app.session.actions.list.push(action);
    }
    app.session.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:A".into(), "F5".into());
    });
    app.ui.actions.selected = Some(1);
    app.ui.actions.selected_set = Some("Two".into());
    app.session.actions.sets.push("Old Empty".into());
    app.ui.actions.selected_step = Some(0);
    app.ui.actions.expanded = vec![true, true];
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 760.0)).build_ui_state(
        |ui, app| {
            actions::panel_menu(app, ui);
            app.poll_file_dialog(ui.ctx(), None);
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    (h, picker)
}

/// Encode a small valid file with visibly different action and set names.
fn input() -> Vec<u8> {
    let action = Action::new("New", vec![("select.all".into(), json!({}))]);
    atn_cmds::export("Incoming", &[&action], &[], false).unwrap().0
}

/// Open through the menu and verify that Replace does not use Load's multi-file picker.
fn pick(h: &mut Harness<'static, PhotocraftApp>, picker: &Picker) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Replace Actions…").click();
    h.run_steps(3);
    assert!(matches!(picker.borrow().as_slice(), [(FileDialogRequest::Open { multiple: false, extensions: Some(ext), .. }, _)] if ext == &["atn"]));
}

/// Deliver a platform answer through the real continuation, without bypassing staging.
fn answer(h: &mut Harness<'static, PhotocraftApp>, picker: &Picker, answer: Option<FileDialogAnswer>) {
    picker.borrow_mut().pop().unwrap().1.send(answer);
    h.run_steps(3);
}

/// Stage a valid inline file and return its actual warning identity.
fn stage(h: &mut Harness<'static, PhotocraftApp>, picker: &Picker) -> u64 {
    pick(h, picker);
    answer(h, picker, Some(FileDialogAnswer::Contents("Chosen.atn".into(), input())));
    let dialog = h.state().ui.dialogs.last().unwrap();
    assert_eq!(dialog.fields["__replaceActions"], true);
    assert!(!dialog.fields.contains_key("data"));
    dialog.id
}

/// Both cancellations and malformed input preserve state; OK uses the bytes already picked.
#[test]
fn cancelling_preserves_actions_and_confirmation_never_rereads_the_source_path() {
    let (mut h, picker) = harness();
    let actions = h.state().session.actions.list.clone();
    let shortcuts = h.state().session.prefs().shortcuts.clone();
    let revision = h.state().session.active().unwrap().revision;
    pick(&mut h, &picker);
    answer(&mut h, &picker, None);
    assert!(h.state().ui.dialogs.is_empty());
    pick(&mut h, &picker);
    answer(&mut h, &picker, Some(FileDialogAnswer::Contents("Broken.atn".into(), b"bad".to_vec())));
    assert!(h.state().ui.dialogs.is_empty());
    stage(&mut h, &picker);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list, actions);
    assert_eq!(h.state().session.prefs().shortcuts, shortcuts);
    assert_eq!(h.state().ui.actions.selected, Some(1));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Picked.atn");
    std::fs::write(&path, input()).unwrap();
    pick(&mut h, &picker);
    answer(&mut h, &picker, Some(FileDialogAnswer::Paths(vec![path.to_str().unwrap().into()])));
    assert!(h.state().ui.dialogs.last().unwrap().fields.contains_key("__replaceActions"));
    std::fs::write(&path, b"changed after the warning").unwrap();
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert_eq!(h.state().session.actions.list.len(), 1);
    assert_eq!(h.state().session.actions.list[0].name, "New");
    assert_eq!(h.state().ui.actions.selected, Some(0));
    assert_eq!(h.state().ui.actions.selected_set, None);
    assert!(h.state().session.actions.sets.is_empty());
    assert_eq!(h.state().ui.actions.selected_step, None);
    assert_eq!(h.state().ui.actions.expanded, [true]);
    assert!(!h.state().session.prefs().shortcuts.contains_key("actions.play:A"));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
}

static DENIED: AtomicBool = AtomicBool::new(false);

/// Model a retained caller whose authorization can be revoked while its warning is open.
fn retained_gate(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "actions.replace" && DENIED.load(Ordering::SeqCst) { Err(EngineError::Other("replace denied".into())) } else { Ok(()) }
}

/// Stale input, revoked retained authority and retargeting all refuse without consuming other dialogs.
#[test]
fn stale_denied_and_retargeted_confirmations_preserve_the_library() {
    for mode in ["picker-stale", "warning-stale", "denied", "retargeted"] {
        DENIED.store(false, Ordering::SeqCst);
        let (mut h, picker) = harness();
        if mode == "denied" {
            h.state_mut().session.authorize = Some(retained_gate);
        }
        if mode == "picker-stale" {
            pick(&mut h, &picker);
            h.state_mut().run("actions.rename", json!({"action": "A", "name": "Changed"})).unwrap();
            answer(&mut h, &picker, Some(FileDialogAnswer::Contents("Chosen.atn".into(), input())));
            assert!(h.state().ui.dialogs.is_empty());
        } else {
            let dialog = stage(&mut h, &picker);
            match mode {
                "warning-stale" => {
                    h.state_mut().run("actions.rename", json!({"action": "A", "name": "Changed"})).unwrap();
                }
                "denied" => {
                    h.state_mut().session.authorize = None;
                    DENIED.store(true, Ordering::SeqCst);
                }
                _ => {
                    h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__command".into(), json!("select.all"));
                }
            }
            let before = h.state().session.actions.list.clone();
            assert!(dialogs::confirm(h.state_mut(), dialog).is_err(), "{mode}");
            assert_eq!(h.state().session.actions.list, before, "{mode}");
        }
        assert_eq!(h.state().session.actions.list.len(), 2, "{mode}");
        assert_eq!(h.state().session.prefs().shortcuts.get("actions.play:New"), None);
        DENIED.store(false, Ordering::SeqCst);
        let retry = stage(&mut h, &picker);
        dialogs::cancel(h.state_mut(), retry).unwrap();
        h.run_steps(3);
    }
}

/// A forged warning cannot confirm or cancel the real warning's privately held bytes.
#[test]
fn another_dialog_cannot_consume_the_pending_replacement() {
    let (mut h, picker) = harness();
    let original = stage(&mut h, &picker);
    let fields = h.state().ui.dialogs.last().unwrap().fields.clone();
    let impostor = h.state_mut().ui.open_dialog(DialogKind::Command, fields.clone());
    assert!(dialogs::confirm(h.state_mut(), impostor).is_err());
    let impostor = h.state_mut().ui.open_dialog(DialogKind::Command, fields);
    dialogs::cancel(h.state_mut(), impostor).unwrap();
    assert_eq!(h.state().session.actions.list.len(), 2);
    dialogs::confirm(h.state_mut(), original).unwrap();
    assert_eq!(h.state().session.actions.list[0].name, "New");
    assert_eq!(h.state().session.actions.list.len(), 1);
}

/// Replacing a library removes its collapsed set names even when the replacement reuses one.
#[test]
fn replacing_library_reveals_reused_set_names() {
    let (mut h, picker) = harness();
    h.state_mut().ui.actions.collapsed_sets = vec!["One".into(), "Two".into(), "Old Empty".into(), "Remember".into()];
    pick(&mut h, &picker);
    let action = Action::new("New", vec![("select.all".into(), json!({}))]);
    let bytes = atn_cmds::export("One", &[&action], &[], false).unwrap().0;
    answer(&mut h, &picker, Some(FileDialogAnswer::Contents("Replacement.atn".into(), bytes)));
    let dialog = h.state().ui.dialogs.last().unwrap().id;
    dialogs::confirm(h.state_mut(), dialog).unwrap();
    assert_eq!(h.state().session.actions.list[0].set.as_deref(), Some("One"));
    assert_eq!(h.state().ui.actions.collapsed_sets, ["Remember"]);
}
