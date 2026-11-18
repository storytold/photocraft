//! A type edit ends when its layer stops being the one worked on (#6483): another layer or
//! another document became active. Photoshop commits the text then; left open, the session kept
//! taking single-key shortcuts as typing and could type into a layer of the wrong document.
//!
//! The helpers marked `pub(super)` are the session harness the other type test files share: a full
//! app frame (the canvas editor holds the keys and the input method), and the session calls
//! with the canvas pass's context.

use crate::app::framework::BuildApp;

use craft::ui::runtime::windows::WindowHost;
use egui_kittest::Harness;
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use super::end_if_left;
use super::mask_replace_selection_tests::click;
use crate::PhotocraftApp;
use crate::canvas::testing;

fn new_app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

fn text_of(app: &PhotocraftApp, doc: usize, id: LayerId) -> Option<String> {
    match &app.session.documents().get(doc)?.doc.layer(id)?.content {
        LayerContent::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

/// Starts a session on the active layer, as the Type tool's click on its text does.
pub(super) fn edit_active(app: &mut PhotocraftApp) {
    let ctx = testing::ctx(app);
    super::edit_active(app, &ctx).unwrap();
    // The editor shows and takes the keyboard in the next pass.
    testing::frame(app);
}

/// A key press event.
pub(super) fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

/// Runs the whole app (logic and UI) on the app's own context: a warm-up so the fonts and the
/// editor's focus are in place, then two frames after each of `events`.
pub(super) fn frames(app: PhotocraftApp, events: Vec<egui::Event>) -> PhotocraftApp {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_context(ctx).build_app(
        |ui, app: &mut PhotocraftApp| {
            let mut frame = eframe::Frame::_new_kittest();
            app.logic_frame(ui.ctx(), &mut frame);
            app.ui_frame(ui, &mut frame);
        },
        app,
    );
    // Between passes, as eframe's input hook does: the runtime's output of the pass is taken (the
    // commands it fired run in the next frame's logic) and its input for the next is set.
    let step = |h: &mut Harness<'_, PhotocraftApp>, n: usize| {
        for _ in 0..n {
            h.step();
            let ctx = h.ctx.clone();
            h.state_mut().framework_before_input(&ctx);
        }
    };
    step(&mut h, 8);
    for event in events {
        h.event(event);
        step(&mut h, 2);
    }
    h.into_state()
}

/// Queues input with `inject` and runs frames until the window host has delivered it, so it reaches the focused
/// editor as the platform's would.
pub(super) fn deliver(app: &mut PhotocraftApp, inject: impl FnOnce(&WindowHost)) {
    let host = WindowHost::of(&testing::ctx(app));
    inject(&host);
    for _ in 0..64 {
        testing::frame(app);
        if host.injected_idle() {
            return;
        }
    }
    panic!("the injected input was not taken");
}

/// Types `s` through the focused editor.
pub(super) fn type_keys(app: &mut PhotocraftApp, s: &str) {
    deliver(app, |host| host.inject_text(egui::ViewportId::ROOT, s));
}

/// An input-method event, delivered to the focused editor.
pub(super) fn ime(app: &mut PhotocraftApp, event: egui::ImeEvent) {
    deliver(app, |host| host.inject_ime(egui::ViewportId::ROOT, event));
}

/// Start a new type layer with a click and type `s` into it, as the canvas does.
pub(super) fn type_new(app: &mut PhotocraftApp, s: &str) -> LayerId {
    click(app, 50.0, 100.0, egui::Modifiers::NONE);
    // The editor shows and takes the keyboard in the next pass.
    testing::frame(app);
    if s.is_empty() {
        // The placeholder is selected: deleting it leaves the text empty.
        testing::key(app, egui::Key::Backspace, egui::Modifiers::NONE);
    } else {
        type_keys(app, s);
    }
    LayerId(app.ui.text_edit.as_ref().unwrap().layer)
}

#[test]
fn typing_after_a_document_switch_never_edits_the_other_document() {
    let mut app = new_app();
    let id = app.run("type.create", json!({"text": "Bee", "size": 24, "x": 20, "y": 80})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    // A second document with the same layer ids, as when one .pcraft is opened twice (its saved
    // ids are kept; only the document id is made unique).
    let copy = (*app.session.active().unwrap().doc).clone();
    app.session.add_document(copy, None);
    app.sync_views();
    // Edit the copy's type layer (index 1), then switch to the first document's tab and type.
    edit_active(&mut app);
    super::host::type_text(&mut app, "Ay");
    assert!(app.session.set_active(0));
    end_if_left(&mut app);
    let app = frames(app, vec![egui::Event::Text("Z".into())]);
    assert_eq!(text_of(&app, 0, id).as_deref(), Some("Bee"), "the other document is untouched");
    assert_eq!(text_of(&app, 1, id).as_deref(), Some("Ay"), "the edited copy keeps its text");
    assert!(app.ui.text_edit.is_none(), "the copy's edit was committed");
    assert_eq!(app.session.active_index(), Some(0), "and the document switched to stays active");
}

#[test]
fn selecting_another_layer_commits_the_edit() {
    let mut app = new_app();
    let other = LayerId(app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap());
    let text = type_new(&mut app, "Hello");
    end_if_left(&mut app);
    assert!(app.ui.text_edit.is_some(), "a session's own layer is active: it goes on");
    // The Layers panel menus, the canvas layer picker, ⌥[ / ⌥] and automation all select this way.
    app.run("layer.select", json!({"layer": other.0})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(other), "the selection stands");
    assert_eq!(st.doc.layer(text).unwrap().name, "Hello", "committed: the new layer is named after its text");
    assert_eq!(text_of(&app, 0, text).as_deref(), Some("Hello"));
}

#[test]
fn an_empty_new_layer_goes_when_another_layer_is_selected() {
    let mut app = new_app();
    let other = LayerId(app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap());
    let text = type_new(&mut app, "");
    app.run("layer.select", json!({"layer": other.0})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
    assert!(app.session.active().unwrap().doc.layer(text).is_none(), "an empty new type layer is deleted on commit");
}

#[test]
fn a_type_mask_session_goes_on_while_its_layer_is_active() {
    let mut app = new_app();
    app.ui.tool = crate::state::Tool::HorizontalTypeMask;
    type_new(&mut app, "Mask");
    end_if_left(&mut app);
    let ed = app.ui.text_edit.as_ref().expect("the Type Mask session is still open");
    assert!(ed.mask);
}

#[test]
fn closing_the_edited_document_drops_the_edit_without_touching_another() {
    let mut app = new_app();
    let id = app.run("type.create", json!({"text": "Bee", "size": 24, "x": 20, "y": 80})).unwrap()["layer"].as_u64().map(LayerId).unwrap();
    let copy = (*app.session.active().unwrap().doc).clone();
    app.session.add_document(copy, None);
    app.sync_views();
    edit_active(&mut app);
    super::host::type_text(&mut app, "");
    assert!(app.session.close(1).is_some());
    let before = app.session.active().unwrap().revision;
    end_if_left(&mut app);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(app.session.active().unwrap().revision, before, "nothing ran in the remaining document");
    assert_eq!(text_of(&app, 0, id).as_deref(), Some("Bee"), "its same-id type layer keeps its text");
}

/// The issue's second symptom: after a layer is selected, single-key tool shortcuts work again
/// instead of typing into the type layer.
#[test]
fn tool_shortcuts_work_after_another_layer_is_selected() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    let app = h.state_mut();
    app.ui.tool = crate::state::Tool::Type;
    let other = app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap();
    let text = type_new(app, "Hello");
    app.run("layer.select", json!({"layer": other})).unwrap();
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_none(), "the frame commits the edit");
    testing::key(h.state_mut(), egui::Key::V, egui::Modifiers::NONE);
    type_keys(h.state_mut(), "v");
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move, "V picks the Move tool");
    assert_eq!(text_of(h.state(), 0, text).as_deref(), Some("Hello"), "and types nothing");
}

/// Ending an empty Type Mask undoes its session, and undo restores the layer selection from
/// before it: the layer just selected (or Deselect Layers) must stand. So must it when a Type
/// Mask with text becomes the selection and its working layer goes.
#[test]
fn ending_a_type_mask_keeps_the_new_layer_selection() {
    for (deselect, text) in [(false, ""), (true, ""), (false, "Mask"), (true, "Mask")] {
        let mut app = new_app();
        let first = LayerId(app.run("layer.new.layer", json!({"name": "First"})).unwrap()["layer"].as_u64().unwrap());
        let second = LayerId(app.run("layer.new.layer", json!({"name": "Second"})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.select", json!({"layer": first.0})).unwrap();
        app.ui.tool = crate::state::Tool::HorizontalTypeMask;
        let mask = type_new(&mut app, text);
        if deselect {
            app.run("select.deselectLayers", json!({})).unwrap();
        } else {
            app.run("layer.select", json!({"layer": second.0})).unwrap();
        }
        end_if_left(&mut app);
        assert!(app.ui.text_edit.is_none());
        let st = app.session.active().unwrap();
        assert!(st.doc.layer(mask).is_none(), "{text:?}: the mask's working layer is gone");
        assert_eq!(st.doc.selection.is_some(), !text.is_empty(), "{text:?}: text becomes the selection");
        let want = if deselect { None } else { Some(second) };
        assert_eq!(st.active_layer, want, "{text:?}, deselect={deselect}: the new selection stands");
        assert_eq!(st.selected_layers(), want.into_iter().collect::<Vec<_>>(), "{text:?}, deselect={deselect}");
    }
}
