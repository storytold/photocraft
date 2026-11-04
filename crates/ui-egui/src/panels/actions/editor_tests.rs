//! Owned page/command behavior. These tests do not qualify the final page visually.
use std::{cell::RefCell, rc::Rc};

use photocraft_engine::{
    Session,
    preset_store::{self, MemBackend},
};
use serde_json::json;

use super::*;

fn app(services: crate::Services) -> (PhotocraftApp, MemBackend) {
    let mut session = Session::new();
    session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    session.execute("actions.record", json!({"name": "Original"})).unwrap();
    session.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
    session.execute("actions.stop", json!({})).unwrap();
    let mem = MemBackend::default();
    assert!(session.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    (PhotocraftApp::new(session, services), mem)
}

fn existing(app: &mut PhotocraftApp) -> Page {
    Page::open(app, json!({"action": "Original"})).unwrap()
}

fn store(mem: &MemBackend) -> std::collections::BTreeMap<String, Vec<u8>> {
    mem.files.lock().unwrap().clone()
}

#[test]
fn picker_choices_reach_the_owned_draft_and_only_save_publishes_them() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    struct State {
        app: PhotocraftApp,
        page: Page,
    }
    let (mut app, mem) = app(Default::default());
    let page = existing(&mut app);
    let before = app.session.actions.list.clone();
    let persisted = store(&mem);
    let mut harness = Harness::builder().with_size(egui::vec2(650.0, 750.0)).build_ui_state(
        |ui, state: &mut State| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let view = state.page.get(&mut state.app).unwrap();
            let swatches = crate::theme::Tokens::get(ui.ctx()).action_swatches();
            let _ = state.page.metadata(&mut state.app, ui, &view, &swatches);
        },
        State { app, page },
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::Studio);
    harness.run_steps(3);
    for label in ["Text colour", "Yellow"] {
        harness.get_by_role_and_label(Role::Button, "Choose icon and color").click();
        harness.run_steps(3);
        harness.get_by_role_and_label(Role::Button, label).click();
        harness.run_steps(3);
    }
    let state = harness.state_mut();
    let view = state.page.get(&mut state.app).unwrap();
    assert_eq!(action_text(&view, "icon"), "TextColor");
    assert_eq!(action_text(&view, "color"), "yellow");
    assert_eq!(state.app.session.actions.list, before);
    assert_eq!(store(&mem), persisted);
    state.page.ask_save(&mut state.app, Activity::default()).unwrap();
    assert_eq!(state.page.answer(&mut state.app, Answer::Save, Activity::default()).unwrap(), Some(Event::Saved { source: Some(0) }));
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(preset_store::open(Box::new(mem))).is_empty());
    assert_eq!(reopened.actions.list[0].icon, "TextColor");
    assert_eq!(reopened.actions.list[0].color, "yellow");
}

#[test]
fn untouched_new_page_keeps_one_default_and_leaves_without_publishing() {
    let (mut app, mem) = app(Default::default());
    let before = app.session.actions.list.clone();
    let persisted = store(&mem);
    let mut page = Page::open(&mut app, json!({})).unwrap();
    let name = page.name.text.clone();
    assert_eq!(name, "Action 2");
    assert_eq!(action_text(&page.get(&mut app).unwrap(), "name"), name);
    let view = page.get(&mut app).unwrap();
    assert!(!page.dirty(&view));
    assert_eq!(page.request_leave(&mut app, Activity::default()).unwrap(), Some(Event::Closed { saved: false, source: None }));
    assert_eq!(app.session.actions.list, before);
    assert_eq!(store(&mem), persisted);
}

#[test]
fn cancel_keeps_local_buffers_and_capture_discard_keeps_real_document_edits() {
    let (mut app, mem) = app(Default::default());
    let before = app.session.actions.list.clone();
    let persisted = store(&mem);
    let mut page = existing(&mut app);
    page.name.text = "Buffered name".into();
    page.ask_save(&mut app, Activity::default()).unwrap();
    page.dont_show_again = true;
    assert_eq!(page.answer(&mut app, Answer::Cancel, Activity::default()).unwrap(), None);
    assert_eq!(page.name.text, "Buffered name");
    assert!(!Page::suppressed(&app));
    page.call(&mut app, "record", json!({})).unwrap();
    app.run("layer.new.layer", json!({"name": "Recorded edit"})).unwrap();
    let document = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    assert_eq!(page.request_leave(&mut app, Activity::default()).unwrap(), None);
    assert_eq!(page.answer(&mut app, Answer::Cancel, Activity::default()).unwrap(), Some(Event::CancelledLeave));
    assert_eq!(page.get(&mut app).unwrap()["recording"], true);
    page.request_leave(&mut app, Activity::default()).unwrap();
    page.dont_show_again = true;
    assert!(matches!(page.answer(&mut app, Answer::Discard, Activity::default()).unwrap(), Some(Event::Closed { saved: false, .. })));
    assert_eq!(app.session.actions.list, before);
    assert_eq!(store(&mem), persisted);
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert!(!Page::suppressed(&app));
}

#[test]
fn explicit_save_stays_open_and_dirty_navigation_saves_then_closes_owned_handle() {
    let (mut app, mem) = app(Default::default());
    let mut page = existing(&mut app);
    page.name.text = "Saved once".into();
    page.description.text = "  Description\nwith spacing  ".into();
    let before = app.session.actions.list.clone();
    page.ask_save(&mut app, Activity::default()).unwrap();
    assert_eq!(app.session.actions.list, before);
    assert_eq!(page.answer(&mut app, Answer::Save, Activity::default()).unwrap(), Some(Event::Saved { source: Some(0) }));
    assert!(!page.closed);
    assert_eq!(app.session.actions.list[0].name, "Saved once");
    assert_eq!(app.session.actions.list[0].description, "  Description\nwith spacing  ");
    page.name.text = "Saved on leaving".into();
    page.request_leave(&mut app, Activity::default()).unwrap();
    assert_eq!(page.answer(&mut app, Answer::Save, Activity::default()).unwrap(), Some(Event::Closed { saved: true, source: Some(0) }));
    assert!(app.run("actions.editor.get", json!({"draft": page.draft()})).is_err());
    let persisted = store(&mem);
    let mut reopened = Session::new();
    assert!(reopened.attach_preset_store(preset_store::open(Box::new(mem.clone()))).is_empty());
    // Reload attaches ATN wire provenance; every public definition field remains identical.
    assert_eq!(serde_json::to_value(&reopened.actions.list).unwrap(), serde_json::to_value(&app.session.actions.list).unwrap());
    assert_eq!(store(&mem), persisted, "reopening must not rewrite the stored library");
}

#[test]
fn invalid_unsent_name_keeps_prompt_and_buffers_without_persisting_suppression() {
    let writes = Rc::new(RefCell::new(0));
    let calls = writes.clone();
    let (mut app, mem) = app(crate::Services {
        save_prefs: Some(Box::new(move |_| {
            *calls.borrow_mut() += 1;
            Ok(())
        })),
        ..Default::default()
    });
    let mut page = existing(&mut app);
    let before = app.session.actions.list.clone();
    let persisted = store(&mem);
    page.name.text = "   ".into();
    page.description.text = "Retain this buffer".into();
    page.ask_save(&mut app, Activity::default()).unwrap();
    page.dont_show_again = true;
    assert!(page.answer(&mut app, Answer::Save, Activity::default()).is_err());
    assert_eq!(page.prompt, Some(Intent::Save));
    assert_eq!(page.name.text, "   ");
    assert_eq!(page.description.text, "Retain this buffer");
    assert!(page.error.is_some());
    assert_eq!(app.session.actions.list, before);
    assert_eq!(store(&mem), persisted);
    assert_eq!(*writes.borrow(), 0);
    assert!(!Page::suppressed(&app));
    page.answer(&mut app, Answer::Discard, Activity::default()).unwrap();
}

#[test]
fn stale_save_keeps_the_draft_and_discard_does_not_restore_the_old_library() {
    let (mut app, mem) = app(Default::default());
    let mut page = existing(&mut app);
    page.description.text = "Private edit".into();
    page.ask_save(&mut app, Activity::default()).unwrap();
    app.run("actions.rename", json!({"action": "Original", "name": "External rename"})).unwrap();
    let external = app.session.actions.list.clone();
    let persisted = store(&mem);
    assert!(page.answer(&mut app, Answer::Save, Activity::default()).is_err());
    assert_eq!(page.prompt, Some(Intent::Save));
    assert_eq!(page.description.text, "Private edit");
    assert_eq!(app.session.actions.list, external);
    page.answer(&mut app, Answer::Discard, Activity::default()).unwrap();
    assert_eq!(app.session.actions.list, external);
    assert_eq!(store(&mem), persisted);
}

#[test]
fn definition_edit_cancel_insert_and_step_flags_never_execute_document_commands() {
    let (mut app, mem) = app(Default::default());
    let mut page = existing(&mut app);
    let library = app.session.actions.list.clone();
    let persisted = store(&mem);
    let document = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let view = page.get(&mut app).unwrap();
    page.edit_settings(&view, 0);
    page.editing.as_mut().unwrap().params.insert("name".into(), json!("Definition only"));
    page.accept_step(&mut app).unwrap();
    let view = page.get(&mut app).unwrap();
    assert_eq!(steps(&view)[0][1]["name"], "Definition only");
    page.edit_settings(&view, 0);
    page.editing.as_mut().unwrap().params.insert("name".into(), json!("Cancelled"));
    page.editing = None;
    assert_eq!(page.get(&mut app).unwrap(), view);
    page.selected = Some(0);
    page.insert_stop(&view);
    page.editing.as_mut().unwrap().params.insert("message".into(), json!("Read this stop"));
    page.accept_step(&mut app).unwrap();
    page.expanded.insert(1);
    page.step(&mut app, json!({"op": "enabled", "step": 1, "enabled": false})).unwrap();
    page.step(&mut app, json!({"op": "dialog", "step": 1, "enabled": true})).unwrap();
    page.step(&mut app, json!({"op": "delete", "step": 0})).unwrap();
    assert_eq!(page.selected, Some(0));
    assert!(page.expanded.contains(&0));
    page.step(&mut app, json!({"op": "duplicate", "step": 0})).unwrap();
    assert_eq!(page.selected, Some(1));
    page.step(&mut app, json!({"op": "move", "step": 1, "to": 0})).unwrap();
    assert_eq!(page.selected, Some(0));
    let view = page.get(&mut app).unwrap();
    assert_eq!(steps(&view).len(), 2);
    for at in [0, 1] {
        assert!(flagged(&view, "disabled", at));
        assert!(flagged(&view, "dialogs", at));
        assert_eq!(steps(&view)[at][1]["message"], "Read this stop");
    }
    assert_eq!(app.session.actions.list, library);
    assert_eq!(store(&mem), persisted);
    assert_eq!(app.session.active().unwrap().doc, document);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn stale_step_dialog_and_old_page_identity_cannot_retarget_new_content() {
    let (mut app, _) = app(Default::default());
    let mut page = existing(&mut app);
    let view = page.get(&mut app).unwrap();
    page.edit_settings(&view, 0);
    page.editing.as_mut().unwrap().params.insert("name".into(), json!("Old dialog"));
    let command = step_parts(&steps(&view)[0]).unwrap().0;
    app.run("actions.editor.step", json!({"draft": page.draft(), "op": "replace", "step": 0, "id": command, "params": {"name": "External"}})).unwrap();
    let external = app.run("actions.editor.get", json!({"draft": page.draft()})).unwrap();
    assert!(page.accept_step(&mut app).is_err());
    assert!(page.editing.is_some());
    assert_eq!(app.run("actions.editor.get", json!({"draft": page.draft()})).unwrap(), external);
    app.run("actions.editor.discard", json!({"draft": page.draft()})).unwrap();
    let fresh = app.run("actions.editor.open", json!({"action": "Original"})).unwrap();
    page.name.text = "Must not retarget".into();
    assert!(page.stage(&mut app).is_err());
    assert_eq!(app.run("actions.editor.get", json!({"draft": fresh["draft"]})).unwrap(), fresh);
    assert_eq!(page.name.text, "Must not retarget");
}

#[test]
fn preview_is_an_owned_request_and_preserves_active_draft_capture() {
    let (mut app, mem) = app(Default::default());
    let mut page = existing(&mut app);
    let library = app.session.actions.list.clone();
    let persisted = store(&mem);
    let document = app.session.active().unwrap().doc.clone();
    page.name.text = "Private preview name".into();
    page.call(&mut app, "record", json!({})).unwrap();
    assert_eq!(page.preview(&mut app, 0).unwrap(), Event::Preview { draft: page.draft(), from: 0 });
    assert_eq!(page.get(&mut app).unwrap()["recording"], true);
    assert_eq!(app.session.actions.list, library);
    assert_eq!(store(&mem), persisted);
    assert_eq!(app.session.active().unwrap().doc, document);
    assert!(page.request_leave(&mut app, Activity { playback_busy: true, finishing_step: false }).is_err());
    assert_eq!(page.get(&mut app).unwrap()["recording"], true);
}

#[test]
fn suppression_persists_only_after_save_and_reopen_bypasses_both_prompts() {
    let saved = Rc::new(RefCell::new(String::new()));
    let sink = saved.clone();
    let (mut app, mem) = app(crate::Services {
        save_prefs: Some(Box::new(move |text| {
            *sink.borrow_mut() = text.to_string();
            Ok(())
        })),
        ..Default::default()
    });
    let mut page = existing(&mut app);
    page.description.text = "Remembered Save".into();
    page.ask_save(&mut app, Activity::default()).unwrap();
    page.dont_show_again = true;
    page.answer(&mut app, Answer::Save, Activity::default()).unwrap();
    assert!(Page::suppressed(&app));
    let text = saved.borrow().clone();
    assert!(text.contains(CONFIRM_PREF));
    let mut session = Session::new();
    assert!(session.attach_preset_store(preset_store::open(Box::new(mem))).is_empty());
    let mut fresh = PhotocraftApp::new(session, crate::Services { load_prefs: Some(Box::new(move || Some(text.clone()))), ..Default::default() });
    assert!(Page::suppressed(&fresh));
    let mut page = existing(&mut fresh);
    page.description.text = "No explicit Save prompt".into();
    assert!(matches!(page.ask_save(&mut fresh, Activity::default()).unwrap(), Some(Event::Saved { .. })));
    assert!(page.prompt.is_none());
    page.description.text = "No dirty navigation prompt".into();
    assert!(matches!(page.request_leave(&mut fresh, Activity::default()).unwrap(), Some(Event::Closed { saved: true, .. })));
    assert_eq!(fresh.session.actions.list[0].description, "No dirty navigation prompt");
}

#[test]
fn preference_write_failure_keeps_prompt_and_page_and_can_retry_without_resaving_definition() {
    let (mut app, _) = app(crate::Services { save_prefs: Some(Box::new(|_| Err("preference write failed".into()))), ..Default::default() });
    let mut page = existing(&mut app);
    page.description.text = "Saved definition".into();
    page.request_leave(&mut app, Activity::default()).unwrap();
    page.dont_show_again = true;
    assert!(page.answer(&mut app, Answer::Save, Activity::default()).is_err());
    assert_eq!(page.prompt, Some(Intent::Leave));
    assert!(!page.closed);
    assert_eq!(page.description.text, "Saved definition");
    assert_eq!(app.session.actions.list[0].description, "Saved definition");
    assert!(!Page::suppressed(&app));
    assert!(page.error.as_deref().unwrap().contains("preference write failed"));
    let revision = app.session.actions.rev;
    app.services.save_prefs = Some(Box::new(|_| Ok(())));
    assert!(matches!(page.answer(&mut app, Answer::Save, Activity::default()).unwrap(), Some(Event::Closed { saved: true, .. })));
    assert_eq!(app.session.actions.rev, revision);
    assert!(Page::suppressed(&app));
}

#[test]
fn unfinished_capture_requires_an_explicit_leave_decision_and_cannot_save_early() {
    let (mut app, _) = app(Default::default());
    let mut page = Page::open(&mut app, json!({})).unwrap();
    page.call(&mut app, "record", json!({})).unwrap();
    let activity = Activity { playback_busy: false, finishing_step: true };
    assert_eq!(page.request_leave(&mut app, activity).unwrap(), None);
    assert_eq!(page.prompt, Some(Intent::Leave));
    assert!(page.answer(&mut app, Answer::Save, activity).is_err());
    assert_eq!(page.get(&mut app).unwrap()["recording"], true);
    page.answer(&mut app, Answer::Cancel, activity).unwrap();
    assert_eq!(page.get(&mut app).unwrap()["recording"], true);
}

#[test]
fn back_button_and_escape_route_through_the_same_owned_cancel_decision() {
    use egui_kittest::{Harness, kittest::Queryable};
    struct State {
        app: PhotocraftApp,
        page: Option<Page>,
        events: Vec<Event>,
    }
    let (mut app, _) = app(Default::default());
    let mut page = existing(&mut app);
    page.description.text = "Keep unsent text".into();
    let library = app.session.actions.list.clone();
    let mut h = Harness::builder().with_size(egui::vec2(500.0, 800.0)).build_ui_state(
        |ui, state: &mut State| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let mut page = state.page.take().unwrap();
            let swatches = [Tokens::get(ui.ctx()).card; 12];
            if let Some(event) = page.show(&mut state.app, ui, &swatches, Activity::default()) {
                state.events.push(event);
            }
            state.page = Some(page);
        },
        State { app, page: Some(page), events: Vec::new() },
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    h.get_by_label("Back").click();
    h.run_steps(3);
    assert_eq!(h.state().page.as_ref().unwrap().prompt, Some(Intent::Leave));
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().events, vec![Event::CancelledLeave]);
    assert_eq!(h.state().page.as_ref().unwrap().description.text, "Keep unsent text");
    assert_eq!(h.state().app.session.actions.list, library);
}
