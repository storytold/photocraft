//! Exercise the actual Clear All panel entry and its confirmation, including stale state.
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_engine::{EngineError, Session, actions_cmds::Action};
use photocraft_ui_egui::{PhotocraftApp, actions, control, dialogs};
use serde_json::{Value, json};

/// Show the real panel menu and modal with a selected action from a two-set library.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 8, "height": 6})).unwrap();
    for (name, set) in [("A", "One"), ("B", "Two")] {
        let mut a = Action::new(name, vec![("select.all".into(), json!({}))]);
        a.set = Some(set.into());
        app.session.actions.list.push(a);
    }
    app.session.prefs.edit(|p| {
        p.shortcuts.insert("actions.play:A".into(), "F5".into());
    });
    app.ui.actions.selected = Some(0);
    app.ui.actions.selected_step = Some(0);
    app.ui.actions.expanded = vec![true, true];
    app.ui.actions.collapsed_sets = vec!["Remember".into()];
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 760.0)).build_ui_state(
        |ui, app| {
            actions::panel_menu(app, ui);
            dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h
}

/// Click the menu entry, rather than bypass its authorization and opening logic.
fn open(h: &mut Harness<'static, PhotocraftApp>) -> u64 {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Clear All Actions").click();
    h.run_steps(3);
    h.state().ui.dialogs.last().unwrap().id
}

/// Cancel preserves the library and selection; OK clears only the intended state.
#[test]
fn cancel_preserves_the_library_and_ok_clears_it_and_panel_selection() {
    let mut h = harness();
    let definitions = h.state().session.actions.list.clone();
    let shortcuts = h.state().session.prefs().shortcuts.clone();
    let revision = h.state().session.active().unwrap().revision;
    open(&mut h);
    h.get_by_label("Clear all actions? This cannot be undone.");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.actions.list, definitions);
    assert_eq!(h.state().session.prefs().shortcuts, shortcuts);
    assert_eq!(h.state().ui.actions.selected, Some(0));
    assert_eq!(h.state().ui.actions.selected_step, Some(0));
    assert_eq!(h.state().ui.actions.expanded, [true, true]);
    open(&mut h);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.is_empty());
    assert!(h.state().session.actions.list.is_empty());
    assert_eq!(h.state().ui.actions.selected, None);
    assert_eq!(h.state().ui.actions.selected_step, None);
    assert!(h.state().ui.actions.expanded.is_empty());
    assert_eq!(h.state().ui.actions.collapsed_sets, ["Remember"]);
    assert!(!h.state().session.prefs().shortcuts.contains_key("actions.play:A"));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
}

/// Deny the concrete destructive command at either authorization boundary.
fn deny_clear(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id == "actions.clearAll" { Err(EngineError::Other("clear denied".into())) } else { Ok(()) }
}

/// Stale state, denied authority and a changed command cannot redirect confirmation.
#[test]
fn stale_denied_or_retargeted_confirmations_never_clear_the_library() {
    for mode in ["stale", "denied", "retargeted"] {
        let mut h = harness();
        if mode == "denied" {
            h.state_mut().session.authorize = Some(deny_clear);
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Clear All Actions").click();
            h.run_steps(3);
            assert!(h.state().ui.dialogs.is_empty());
            assert_eq!(h.state().session.actions.list.len(), 2);
            h.state_mut().session.authorize = None;
        }
        let dialog = open(&mut h);
        let context = h.ctx.clone();
        let original = h.state().ui.dialogs.last().unwrap().fields["__clearActionsRevision"].clone();
        let (request, _receiver) = control::ControlRequest::new("ui.dialog.set", json!({"dialog": dialog, "field": "__clearActionsRevision", "value": 12345}));
        let control::Outcome::Done(result) = control::handle(h.state_mut(), &context, &request) else { panic!("expected an immediate field refusal") };
        assert_eq!(result["ok"], false);
        assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__clearActionsRevision"], original);
        match mode {
            "stale" => {
                h.state_mut().run("actions.rename", json!({"action": "A", "name": "Changed"})).unwrap();
            }
            "denied" => h.state_mut().session.authorize = Some(deny_clear),
            _ => {
                h.state_mut().ui.dialog_mut(dialog).unwrap().fields.insert("__command".into(), json!("select.all"));
            }
        }
        let definitions = h.state().session.actions.list.clone();
        let shortcuts = h.state().session.prefs().shortcuts.clone();
        let revision = h.state().session.active().unwrap().revision;
        assert!(dialogs::confirm(h.state_mut(), dialog).is_err(), "{mode}");
        assert_eq!(h.state().session.actions.list, definitions, "{mode}");
        assert_eq!(h.state().session.prefs().shortcuts, shortcuts, "{mode}");
        assert_eq!(h.state().session.active().unwrap().revision, revision, "{mode}");
    }
}
