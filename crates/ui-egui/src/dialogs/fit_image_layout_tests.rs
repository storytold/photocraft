//! #13851: the Fit Image dialog as the reference application lays it out: Width, Height and
//! Don't Enlarge in a "Constrain Within" group with "pixels" after each size, OK and Cancel
//! stacked on the right, Tab through all five, and Alt+W / Alt+H to the sizes.

use egui::{Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::json;

use super::show;
use crate::{PhotocraftApp, Services};

const FIT_IMAGE: &str = "file.automate.fitImage";

fn open() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    let id = crate::menus::invoke(h.state_mut(), &ctx, FIT_IMAGE, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    (h, id)
}

/// Where each painted text of exactly `text` is (its galley's rect on screen).
fn texts(h: &Harness<'_, PhotocraftApp>, text: &str) -> Vec<Rect> {
    fn walk(shape: &egui::Shape, text: &str, out: &mut Vec<Rect>) {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => out.push(t.galley.rect.translate(t.pos.to_vec2())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, text, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|c| walk(&c.shape, text, &mut out));
    out
}

fn field(h: &Harness<'_, PhotocraftApp>, name: &str) -> Rect {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(name)).expect("the field").rect()
}

fn focused(h: &Harness<'_, PhotocraftApp>) -> Option<String> {
    h.query_all_by_label_contains("").find(|n| n.accesskit_node().is_focused()).and_then(|n| n.accesskit_node().label())
}

fn dims(h: &Harness<'_, PhotocraftApp>) -> (u32, u32) {
    let size = h.state().session.active().unwrap().doc.size;
    (size.width, size.height)
}

fn same_row(a: Rect, b: Rect) -> bool {
    (a.center().y - b.center().y).abs() < 4.0
}

#[test]
fn rows_labels_pixels_and_group_read_as_the_reference() {
    let (h, _) = open();
    let (width, height) = (field(&h, "Width"), field(&h, "Height"));
    let enlarge = h.get_by_label("Don't Enlarge").rect();
    assert!(width.bottom() <= height.top() && height.bottom() <= enlarge.top(), "Width, Height, Don't Enlarge");
    for (label, at) in [("Width:", width), ("Height:", height)] {
        let caption = texts(&h, label);
        assert!(caption.iter().any(|r| same_row(*r, at) && r.right() <= at.left()), "{label} before its field");
        assert!(texts(&h, "pixels").iter().any(|r| same_row(*r, at) && r.left() >= at.right()), "pixels after the {label} field");
    }
    assert!(texts(&h, "Dont enlarge").is_empty(), "the old wording is gone");
    let group = texts(&h, "Constrain Within");
    assert_eq!(group.len(), 1);
    assert!(group[0].bottom() <= width.top(), "the group title above its rows");
}

#[test]
fn ok_and_cancel_stand_right_of_the_group() {
    let (h, _) = open();
    let (ok, cancel) = (h.get_by_label("OK").rect(), h.get_by_label("Cancel").rect());
    let right_of_fields = texts(&h, "pixels").iter().map(|r| r.right()).fold(f32::MIN, f32::max).max(h.get_by_label("Don't Enlarge").rect().right());
    assert!(ok.left() > right_of_fields, "OK {ok:?} right of the group");
    assert!((ok.left() - cancel.left()).abs() < 1.0 && ok.bottom() <= cancel.top(), "OK above Cancel");
    assert!(ok.top() <= field(&h, "Width").bottom(), "beside the group, not under it");
}

#[test]
fn tab_goes_width_height_dont_enlarge_ok_cancel_and_round() {
    let (mut h, _) = open();
    assert_eq!(focused(&h), None, "nothing has the keyboard on open");
    let mut order = Vec::new();
    for _ in 0..6 {
        h.key_press(Key::Tab);
        h.run_steps(3);
        order.push(focused(&h).unwrap_or_default());
    }
    assert_eq!(order, ["Width", "Height", "Don't Enlarge", "OK", "Cancel", "Width"]);
}

#[test]
fn alt_w_and_alt_h_go_to_the_sizes_with_their_text_selected() {
    let (mut h, id) = open();
    h.key_press_modifiers(Modifiers::ALT, Key::H);
    h.run_steps(3);
    assert_eq!(focused(&h).as_deref(), Some("Height"));
    h.event(Event::Text("120".into()));
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::ALT, Key::W);
    h.run_steps(3);
    assert_eq!(focused(&h).as_deref(), Some("Width"));
    h.event(Event::Text("160".into()));
    h.run_steps(1);
    h.key_press(Key::Tab);
    h.run_steps(3);
    let f = &h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!((f["width"].clone(), f["height"].clone()), (json!(160), json!(120)), "typed over the selected text");
    // Alt+D isn't a mnemonic here.
    h.key_press_modifiers(Modifiers::ALT, Key::D);
    h.run_steps(3);
    assert_eq!(focused(&h).as_deref(), Some("Height"), "Alt+D moves nothing");
}

#[test]
fn enter_is_ok_and_escape_is_cancel_with_the_same_field_names() {
    let (mut h, id) = open();
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id), "Escape closes it");
    assert_eq!(dims(&h), (400, 300));

    let (mut h, id) = open();
    let keys: Vec<String> = crate::view_cmds::form_keys(&h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields);
    assert_eq!(keys.iter().filter(|k| ["width", "height", "dontEnlarge"].contains(&k.as_str())).count(), 3, "{keys:?}");
    let d = h.state_mut().ui.dialogs.iter_mut().find(|d| d.id == id).unwrap();
    d.fields.insert("width".into(), json!(200));
    d.fields.insert("height".into(), json!(200));
    h.run_steps(2);
    h.event(Event::PointerMoved(Pos2::ZERO));
    h.key_press(Key::Enter);
    h.run_steps(6);
    assert!(h.state().ui.dialogs.iter().all(|d| d.id != id), "Enter runs it");
    assert_eq!(dims(&h), (200, 150), "fitted within 200 × 200");
}

/// CodeRabbit on #15123: numbers typed in the same frame as the shortcut still reach the field that has the
/// focus; only the character the shortcut itself types (after it) is dropped.
#[test]
fn text_typed_in_the_frame_of_alt_w_still_reaches_the_active_field() {
    let (mut h, id) = open();
    h.key_press_modifiers(Modifiers::ALT, Key::H);
    h.run_steps(3);
    assert_eq!(focused(&h).as_deref(), Some("Height"));
    h.event(Event::Text("120".into()));
    h.key_press_modifiers(Modifiers::ALT, Key::W);
    h.run_steps(3);
    assert_eq!(focused(&h).as_deref(), Some("Width"), "the shortcut still moves the focus");
    h.key_press(Key::Tab);
    h.run_steps(3);
    let f = &h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert_eq!(f["height"], json!(120), "the number typed with the shortcut was entered");
}

/// The shortcut's own character goes; the numbers typed before it, and the text typed after it in the same
/// frame, stay (the frame's events in order).
#[test]
fn only_the_shortcuts_own_character_is_dropped() {
    use super::fit_image_range::drop_text_after;
    let text = |t: &str| Event::Text(t.into());
    // [Text 120, (the shortcut, already consumed, was here), Text ∑]: the shortcut sat at index 1.
    let mut events = vec![text("120"), Event::PointerGone, text("\u{2211}")];
    drop_text_after(&mut events, 1);
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], Event::Text(t) if t == "120"), "the number stays");
    assert!(matches!(events[1], Event::PointerGone), "other events stay");
    // A shortcut first: its character goes, the entry typed after it in the same frame stays.
    let mut events = vec![text("\u{2211}"), text("7"), text("42")];
    drop_text_after(&mut events, 0);
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(matches!(&events[0], Event::Text(t) if t == "7"), "{events:?}");
    assert!(matches!(&events[1], Event::Text(t) if t == "42"), "{events:?}");
    // Nothing the shortcut typed (Windows): nothing goes.
    let mut events = vec![text("7")];
    drop_text_after(&mut events, 0);
    assert_eq!(events.len(), 1);
}
