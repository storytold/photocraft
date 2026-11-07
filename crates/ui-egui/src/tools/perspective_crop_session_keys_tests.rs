//! #11807: pending Perspective Crop quads own Undo and default tool letters.
//! These use the full eframe application, native-shaped pointer/key events and the production
//! shortcut ordering. No shortcut dry-run, replacement handler or product-only test seam is used.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_doc::{Document, LayerId, Size};
use serde_json::json;

use crate::{PhotocraftApp, Services, state::Tool};

type App = Harness<'static, PhotocraftApp>;

fn app() -> App {
    let mut h = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(64)
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, Default::default()))
        .build_eframe(|_| {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
            app.run("file.new", json!({"width": 96, "height": 72, "background": "white"})).unwrap();
            app.run("layer.new.layer", json!({"name": "session-key sentinel"})).unwrap();
            app.run("select.rect", json!({"x": 20, "y": 16, "width": 36, "height": 28})).unwrap();
            app.run("edit.fill", json!({"color": "#d03070"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            app.ui.tool = Tool::PerspectiveCrop;
            app.ui.extras.snap = false;
            app.ui.extras.rulers = false;
            app
        });
    h.run_steps(6);
    let view = &mut h.state_mut().ui.views[0];
    view.zoom = 3.0;
    view.center = [48.0, 36.0];
    view.fit_pending = false;
    view.fill_pending = false;
    h.run_steps(3);
    assert!(h.state().fonts_ready, "production shortcut handling is ready");
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Deselect"));
    crate::shortcut_dispatch::take_log(&h.ctx);
    h
}

fn command(shift: bool) -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers { command: true, mac_cmd: true, shift, ..Modifiers::NONE }
    } else {
        Modifiers { command: true, ctrl: true, shift, ..Modifiers::NONE }
    }
}

fn key_frame(h: &mut App, key: Key, pressed: bool, repeat: bool, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::Key { key, physical_key: Some(key), pressed, repeat, modifiers });
    h.step();
}

fn tap(h: &mut App, key: Key, modifiers: Modifiers) {
    key_frame(h, key, true, false, modifiers);
    key_frame(h, key, false, false, Modifiers::NONE);
    h.run_steps(2);
}

fn screen(h: &App, point: [f64; 2]) -> Pos2 {
    crate::canvas::ViewXform::active(h.state()).expect("the production canvas mapping").to_screen(point[0] as f32, point[1] as f32)
}

fn pointer_button(h: &mut App, at: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.step();
}

fn drag(h: &mut App, from: [f64; 2], to: [f64; 2]) {
    let (from, to) = (screen(h, from), screen(h, to));
    h.hover_at(from);
    h.step();
    pointer_button(h, from, true);
    for fraction in [0.25, 0.5, 0.75, 1.0] {
        h.event(Event::PointerMoved(from + (to - from) * fraction));
        h.step();
    }
    pointer_button(h, to, false);
    h.run_steps(2);
}

fn quad(h: &mut App) {
    let before = document(h);
    drag(h, [8.0, 8.0], [80.0, 56.0]);
    let box_quad = [[8.0, 8.0], [80.0, 8.0], [80.0, 56.0], [8.0, 56.0]];
    assert_quad_near(h, box_quad);
    // Keep a genuine last handle move: Undo must not undo even this transient edit.
    drag(h, [80.0, 56.0], [76.0, 60.0]);
    assert_quad_near(h, [[8.0, 8.0], [80.0, 8.0], [76.0, 60.0], [8.0, 56.0]]);
    assert_eq!(document(h), before, "drawing and moving a quad do not edit the document");
    assert_eq!(crate::shortcut_dispatch::Focus::of(&h.ctx), crate::shortcut_dispatch::Focus::None);
}

fn assert_quad_near(h: &App, expected: [[f64; 2]; 4]) {
    let actual = h.state().ui.perspective_crop.expect("native canvas gesture made a quad");
    assert!(actual.iter().flatten().zip(expected.iter().flatten()).all(|(a, e)| (a - e).abs() < 1e-4), "pointer mapping: {actual:?} != {expected:?}");
    assert!(h.state().pcrop.drag.is_none());
}

fn document(h: &App) -> Arc<Document> {
    h.state().session.active().unwrap().doc.clone()
}

/// Compare the complete document (including native layer/selection samples), full-canvas CPU
/// pixels, every undo/current/redo row's document, exact labels, stable IDs and current position.
#[derive(Debug, PartialEq)]
struct DocumentHistory {
    doc: Arc<Document>,
    pixels: Vec<[f32; 4]>,
    labels: Vec<String>,
    redo: Vec<String>,
    rows: Vec<(u64, Arc<Document>)>,
    position: usize,
    active_layer: Option<LayerId>,
    selected_layers: Vec<LayerId>,
}

fn document_history(h: &App) -> DocumentHistory {
    let st = h.state().session.active().unwrap();
    let pixels = photocraft_compose::render(&st.doc, photocraft_geom::Rect::new(0, 0, st.doc.size.width as i32, st.doc.size.height as i32));
    let rows = st.history.row_ids().enumerate().map(|(i, id)| (id, st.history.row_doc(i, &st.doc).unwrap())).collect();
    DocumentHistory {
        doc: st.doc.clone(),
        pixels: pixels.px.clone(),
        labels: st.history.entries(),
        redo: st.history.redo_labels().map(str::to_owned).collect(),
        rows,
        position: st.history.past_len(),
        active_layer: st.active_layer,
        selected_layers: st.selected_layers.clone(),
    }
}

#[derive(Debug, PartialEq)]
struct Pending {
    document: DocumentHistory,
    quad: Option<[[f64; 2]; 4]>,
    clicks: Vec<[f64; 2]>,
    drag: String,
    tool: Tool,
    spring: Option<crate::spring_tools::Spring>,
    revision: u64,
}

fn pending(h: &App) -> Pending {
    Pending {
        document: document_history(h),
        quad: h.state().ui.perspective_crop,
        clicks: h.state().pcrop.clicks.clone(),
        drag: format!("{:?}", h.state().pcrop.drag),
        tool: h.state().ui.tool,
        spring: h.state().ui.spring.clone(),
        revision: h.state().session.active().unwrap().revision,
    }
}

fn dispatched(h: &App, id: &str) {
    assert_eq!(crate::shortcut_dispatch::take_log(&h.ctx), [(id.to_owned(), crate::shortcut_dispatch::Outcome::Ran)]);
}

#[test]
fn issue_11807_pending_quad_ctrl_z_preserves_document_and_all_history() {
    let mut h = app();
    quad(&mut h);
    let before = pending(&h);
    key_frame(&mut h, Key::Z, true, false, command(false));
    assert_eq!(pending(&h), before, "Ctrl+Z must not undo Deselect or the last handle move");
    key_frame(&mut h, Key::Z, true, true, command(false));
    assert_eq!(pending(&h), before, "repeat must not leak Undo");
    key_frame(&mut h, Key::Z, false, false, Modifiers::NONE);
    h.run_steps(4);
    assert_eq!(pending(&h), before, "release and later frames cannot undo either");
    assert!(crate::shortcut_dispatch::take_log(&h.ctx).is_empty());
    undo_with_released_frame_modifiers(&mut h);
}

#[test]
fn issue_11807_invalid_pending_quad_also_owns_ctrl_z() {
    let mut h = app();
    quad(&mut h);
    // Move the bottom-right corner across the opposite edge through the real canvas.
    drag(&mut h, [76.0, 60.0], [4.0, 4.0]);
    assert!(!photocraft_algo::transform::convex_quad(&h.state().ui.perspective_crop.unwrap()));
    let before = pending(&h);
    tap(&mut h, Key::Z, command(false));
    assert_eq!(pending(&h), before, "validity does not release a pending session's keyboard");
}

#[test]
fn issue_11807_v_hold_repeat_and_consecutive_letters_never_arm_a_spring() {
    let mut h = app();
    quad(&mut h);
    let before = pending(&h);
    key_frame(&mut h, Key::V, true, false, Modifiers::NONE);
    assert_eq!(pending(&h), before, "the initial V press cannot select Move or arm spring state");
    h.run_steps(24);
    key_frame(&mut h, Key::V, true, true, Modifiers::NONE);
    assert_eq!(pending(&h), before, "held and repeated V cannot switch tools");
    key_frame(&mut h, Key::B, true, false, Modifiers::NONE);
    assert_eq!(pending(&h), before, "another held letter cannot strand the quad");
    key_frame(&mut h, Key::B, false, false, Modifiers::NONE);
    key_frame(&mut h, Key::V, false, false, Modifiers::NONE);
    tap(&mut h, Key::V, Modifiers::NONE);
    assert_eq!(pending(&h), before);
}

#[test]
fn issue_11807_every_other_default_tool_letter_preserves_the_session() {
    let letters: std::collections::BTreeSet<char> = Tool::ALL.into_iter().map(Tool::key).filter(|c| *c != '\0' && *c != 'V').collect();
    for letter in letters {
        // Each letter has a new document/session, so a preceding choice cannot mask its route.
        let mut h = app();
        quad(&mut h);
        let before = pending(&h);
        let key = crate::spring_tools::key_of(letter).unwrap();
        tap(&mut h, key, Modifiers::NONE);
        assert_eq!(pending(&h), before, "default tool letter {letter}");
    }
}

#[test]
fn issue_11807_shift_c_does_not_cycle_the_crop_group_with_either_preference() {
    for shift_switch in [false, true] {
        let mut h = app();
        h.state_mut().run("prefs.set", json!({"path": "tools.useShiftKeyForToolSwitch", "value": shift_switch})).unwrap();
        quad(&mut h);
        let before = pending(&h);
        tap(&mut h, Key::C, Modifiers::SHIFT);
        assert_eq!(pending(&h), before, "Shift+C, shift-switch preference {shift_switch}");
        tap(&mut h, Key::C, Modifiers::NONE);
        assert_eq!(pending(&h), before, "plain C must not select Slice either");
    }
}

fn ordinary_undo_and_tools(h: &mut App) {
    let before = document_history(h);
    let st = h.state().session.active().unwrap();
    let expected = st.history.state(st.history.past_len() - 1).unwrap();
    assert_ne!(expected, before.doc, "the Undo control has an observable selection change");
    tap(h, Key::Z, command(false));
    dispatched(h, "edit.undo");
    assert_eq!(document(h), expected);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before.position - 1);
    tap(h, Key::V, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Move, "ordinary tool keys resume");
    assert!(h.state().ui.spring.is_none(), "a released tap leaves no spring");
}

#[test]
fn issue_11807_no_quad_keeps_ordinary_undo_and_tools() {
    let mut h = app();
    assert!(!super::pending(h.state()));
    ordinary_undo_and_tools(&mut h);
}

#[test]
fn issue_11807_one_two_or_three_clicked_corners_keep_existing_key_behavior() {
    for count in 1..=3 {
        let mut h = app();
        for point in [[8.0, 8.0], [80.0, 8.0], [80.0, 56.0]].into_iter().take(count) {
            let at = screen(&h, point);
            pointer_button(&mut h, at, true);
            pointer_button(&mut h, at, false);
        }
        assert_eq!(h.state().pcrop.clicks.len(), count);
        assert!(h.state().ui.perspective_crop.is_none());
        let corners = h.state().pcrop.clicks.clone();
        ordinary_undo_and_tools(&mut h);
        assert_eq!(h.state().pcrop.clicks, corners, "existing partial-corner behavior, count {count}");
    }
}

#[test]
fn issue_11807_ctrl_shift_z_keeps_real_redo_during_a_quad() {
    let mut h = app();
    let redone = document(&h);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let position = h.state().session.active().unwrap().history.past_len();
    quad(&mut h);
    let quad = h.state().ui.perspective_crop;
    tap(&mut h, Key::Z, command(true));
    dispatched(&h, "edit.redo");
    assert_eq!(document(&h), redone);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), position + 1);
    assert_eq!(h.state().ui.perspective_crop, quad);
    assert_eq!(h.state().ui.tool, Tool::PerspectiveCrop);
}

#[test]
fn issue_11807_ctrl_y_keeps_its_actual_proof_colors_binding_not_an_invented_redo() {
    let mut h = app();
    quad(&mut h);
    let before = document_history(&h);
    let quad = h.state().ui.perspective_crop;
    let checked = |h: &App| crate::menus::menu_items(h.state()).into_iter().find(|item| item.id == "view.proofColors").unwrap().checked;
    assert_eq!(checked(&h), Some(false));
    tap(&mut h, Key::Y, command(false));
    dispatched(&h, "view.proofColors");
    assert_eq!(checked(&h), Some(true));
    assert_eq!(document_history(&h), before);
    assert_eq!(h.state().ui.perspective_crop, quad);
    assert_eq!(h.state().ui.tool, Tool::PerspectiveCrop);
}

/// Width is hint text, not an accessible label. Bind the crop row by its swap button,
/// then take its sole TextInput immediately to the left, never Properties' Width SpinButton.
fn crop_width(h: &App) -> egui_kittest::Node<'_> {
    use egui_kittest::kittest::NodeT;

    let swap = h.get_by_role_and_label(egui::accesskit::Role::Button, "Swaps height and width");
    let row = swap.parent().expect("the production crop options row");
    let swap_rect = swap.rect();
    let mut fields = row.query_all_by_role(egui::accesskit::Role::TextInput).filter(|field| {
        let rect = field.rect();
        rect.right() <= swap_rect.left() && rect.top() <= swap_rect.center().y && rect.bottom() >= swap_rect.center().y
    });
    let width = fields.next().expect("the crop-width TextInput before its swap button");
    assert!(fields.next().is_none(), "only crop width occupies this part of the options row");
    assert!(width.rect().is_positive() && swap_rect.is_positive(), "the real controls have clickable geometry");
    assert!(!width.accesskit_node().is_hidden() && !width.accesskit_node().is_disabled());
    width
}

#[test]
fn issue_11807_focused_width_field_keeps_typing_and_text_undo() {
    let mut h = app();
    quad(&mut h);
    let before = pending(&h);
    assert_eq!(crop_width(&h).value().as_deref(), Some(""), "the actual crop-width editor starts empty");
    crop_width(&h).click();
    h.run_steps(2);
    assert!(crop_width(&h).is_focused(), "the clicked crop-width editor itself owns focus");
    assert_eq!(crate::shortcut_dispatch::Focus::of(&h.ctx), crate::shortcut_dispatch::Focus::Text);
    for (key, text) in [(Key::V, "v"), (Key::B, "b")] {
        h.event(Event::Text(text.into()));
        tap(&mut h, key, Modifiers::NONE);
        assert_eq!(crate::shortcut_dispatch::Focus::of(&h.ctx), crate::shortcut_dispatch::Focus::Text);
        assert_eq!(pending(&h), before, "the field owns the tool letters");
    }
    let value = || crop_width(&h).value();
    assert_eq!(value().as_deref(), Some("vb"), "letters were delivered to the actual production field");
    h.run_steps(40);
    tap(&mut h, Key::Z, command(false));
    assert_ne!(crop_width(&h).value().as_deref(), Some("vb"), "the focused text editor, not the quad, takes text Undo");
    assert_eq!(pending(&h), before, "Undo belongs to the focused text editor, not the document");
    assert!(crate::shortcut_dispatch::take_log(&h.ctx).is_empty());
}

fn assert_committed_then_undo_and_tools(h: &mut App, before: &DocumentHistory) {
    assert!(!super::pending(h.state()), "successful commit closes the session");
    let st = h.state().session.active().unwrap();
    assert_ne!(st.doc.size, Size::new(96, 72), "a real crop was applied");
    assert_eq!(st.history.past_len(), before.position + 1, "exactly one crop step");
    assert_eq!(st.history.entries().last().map(String::as_str), Some("Perspective Crop"));
    tap(h, Key::Z, command(false));
    dispatched(h, "edit.undo");
    let after = document_history(h);
    assert_eq!(after.doc, before.doc, "Undo restores the complete uncropped document and native samples");
    assert_eq!(after.pixels, before.pixels, "every uncropped canvas pixel returns");
    assert_eq!(after.labels, before.labels);
    assert_eq!(after.position, before.position);
    assert_eq!(after.active_layer, before.active_layer);
    assert_eq!(after.selected_layers, before.selected_layers);
    assert_eq!(after.redo, ["Perspective Crop"]);
    tap(h, Key::V, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Move);
    assert!(h.state().ui.spring.is_none());
}

#[test]
fn issue_11807_enter_commits_one_step_then_undo_and_tool_letters_resume() {
    let mut h = app();
    quad(&mut h);
    let before = document_history(&h);
    tap(&mut h, Key::Enter, Modifiers::NONE);
    assert_committed_then_undo_and_tools(&mut h, &before);
}

#[test]
fn issue_11807_checkmark_commits_one_step_then_undo_and_tool_letters_resume() {
    let mut h = app();
    quad(&mut h);
    let before = document_history(&h);
    h.get_by_label_contains("Commit current crop operation").click();
    h.run_steps(3);
    assert_committed_then_undo_and_tools(&mut h, &before);
}

fn assert_cancelled_then_undo_and_tools(h: &mut App, before: &DocumentHistory) {
    assert!(!super::pending(h.state()), "cancel clears quad, clicks and drag");
    assert_eq!(&document_history(h), before, "cancel records no history and edits no pixels");
    ordinary_undo_and_tools(h);
}

#[test]
fn issue_11807_escape_cancels_without_history_then_undo_and_tools_resume() {
    let mut h = app();
    quad(&mut h);
    let before = document_history(&h);
    tap(&mut h, Key::Escape, Modifiers::NONE);
    assert_cancelled_then_undo_and_tools(&mut h, &before);
}

#[test]
fn issue_11807_cancel_button_records_no_step_then_undo_and_tools_resume() {
    let mut h = app();
    quad(&mut h);
    let before = document_history(&h);
    h.get_by_label("Cancel current crop operation  (Esc)").click();
    h.run_steps(3);
    assert_cancelled_then_undo_and_tools(&mut h, &before);
}

fn undo_with_released_frame_modifiers(h: &mut App) {
    let before = pending(h);
    // Bypass Harness::event's per-event queue: these events must share one native input batch.
    let input = h.input_mut();
    input.events.extend([
        Event::ModifiersChanged(command(false)),
        Event::Key { key: Key::Z, physical_key: Some(Key::Z), pressed: true, repeat: false, modifiers: command(false) },
        Event::ModifiersChanged(Modifiers::NONE),
    ]);
    h.step();
    assert_eq!(pending(h), before, "the actual Ctrl+Z event still belongs to the quad");
    key_frame(h, Key::Z, false, false, Modifiers::NONE);
}

#[test]
fn issue_11807_other_keyboard_owners_and_custom_chords_keep_current_behavior() {
    let mut h = app();
    quad(&mut h);
    let before = pending(&h);
    // The production menu, not the canvas tool, takes Escape first.
    h.get_by_label("Edit").click();
    h.run_steps(3);
    assert!(crate::menu_nav::is_open(&h.ctx));
    tap(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!crate::menu_nav::is_open(&h.ctx));
    assert_eq!(pending(&h), before);
    // Likewise the actual options-bar unit popup.
    h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Resolution").click();
    h.run_steps(3);
    assert!(crate::widgets::dropdown_list_open(&h.ctx));
    tap(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!crate::widgets::dropdown_list_open(&h.ctx));
    assert_eq!(pending(&h), before);
    // Space is temporary navigation, not a tool-letter switch.
    key_frame(&mut h, Key::Space, true, false, Modifiers::NONE);
    assert_eq!(crate::hold_keys::held_tool(h.state(), &h.ctx), Some(crate::hold_keys::Temporary::Hand));
    h.run_steps(4);
    key_frame(&mut h, Key::Space, false, false, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(pending(&h), before);
    // The report deliberately does not define remapped/non-letter tool policy: retain it.
    h.state_mut().run("prefs.set", json!({"path": "shortcuts.tools.select.move", "value": "F6"})).unwrap();
    tap(&mut h, Key::F6, Modifiers::NONE);
    dispatched(&h, "tools.select.move");
    assert_eq!(h.state().ui.tool, Tool::Move);
    assert_eq!(h.state().ui.perspective_crop, before.quad);
    assert_eq!(document_history(&h), before.document);
}
