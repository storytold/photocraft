//! #13316: the Properties panel of a live polygon lists Sides first, as a slider bound to its
//! number field (whole numbers, 3-100), then Corner radius, then Star ratio, as the reference
//! does. The shape follows the slider while it is dragged, and one drag is one History state.

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::{Value, json};

use super::shape_properties;
use crate::PhotocraftApp;

type H = Harness<'static, PhotocraftApp>;

/// The Properties panel, 320 px wide like the dock, for a new shape `create` makes in an
/// 800 x 600 document.
fn properties(create: Value) -> (H, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let made = app.run("shape.create", create).unwrap();
    let id = LayerId(made["layer"].as_u64().unwrap());
    let mut h = Harness::builder().with_size(vec2(320.0, 900.0)).build_ui_state(
        move |ui, app| {
            // The first frame runs before `setup_context` installs the app's fonts.
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                shape_properties(app, ui, id);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    (h, id)
}

/// The measured polygon: 5 sides, star ratio 100 %, corner radius 0 px.
fn pentagon() -> (H, LayerId) {
    properties(json!({"kind": "polygon", "rect": [300, 200, 200, 200], "sides": 5, "starRatio": 1.0}))
}

/// The live polygon's side count and the anchors its path has.
fn sides(h: &H, id: LayerId) -> (u32, usize) {
    let LayerContent::Shape(sh) = &h.state().session.active().unwrap().doc.layer(id).unwrap().content else { panic!("a shape layer") };
    let Some(LiveShape::Polygon { sides, .. }) = &sh.live else { panic!("a live polygon, got {:?}", sh.live) };
    (*sides, sh.path.subpaths[0].knots.len())
}

fn history(h: &H) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn slider(h: &H) -> Rect {
    h.get_by_role_and_label(Role::Slider, "Sides").rect()
}

fn field_named(h: &H, label: &str) -> Option<Rect> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).map(|n| n.rect())
}

fn field(h: &H, label: &str) -> Rect {
    field_named(h, label).unwrap_or_else(|| panic!("the {label} field"))
}

/// Where the slider's knob sits for `n` sides: its track is the row inset 7 px at each end.
fn at(h: &H, n: f32) -> Pos2 {
    let r = slider(h);
    pos2(r.left() + 7.0 + (n - 3.0) / 97.0 * (r.width() - 14.0), r.center().y)
}

fn button(h: &mut H, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn move_to(h: &mut H, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
}

/// Drags the knob from `from` sides through each of `through`, checking after each step that the
/// shape already has that many sides and anchors, then releases. The first step must be past
/// egui's 6 px click distance, so the press becomes a drag.
fn drag(h: &mut H, id: LayerId, from: f32, through: &[f32]) {
    let start = at(h, from);
    move_to(h, start);
    button(h, start, true);
    let mut last = start;
    for &n in through {
        last = at(h, n);
        move_to(h, last);
        let want = n.round() as u32;
        assert_eq!(sides(h, id), (want, want as usize), "the shape follows the slider to {n}");
    }
    button(h, last, false);
}

#[test]
fn the_sides_slider_and_field_come_before_corner_radius_then_star_ratio() {
    let (h, _) = pentagon();
    let s = h.get_by_role_and_label(Role::Slider, "Sides");
    assert_eq!(s.accesskit_node().numeric_value(), Some(5.0));
    let (s, f, radius, star) = (s.rect(), field(&h, "Sides"), field(&h, "Corner radius"), field(&h, "Star ratio"));
    assert!((s.center().y - f.center().y).abs() < 1.0, "the slider and its field share a row: {s:?} {f:?}");
    assert!(s.right() <= f.left() && s.width() >= 60.0, "the slider comes before its field: {s:?} {f:?}");
    assert!(f.bottom() <= radius.top(), "Sides comes before Corner radius: {f:?} {radius:?}");
    assert!(radius.bottom() <= star.top(), "Corner radius comes before Star ratio: {radius:?} {star:?}");
}

#[test]
fn one_drag_up_and_one_drag_down_are_one_history_state_each() {
    let (mut h, id) = pentagon();
    let before = history(&h);
    drag(&mut h, id, 5.0, &[20.0, 8.0]);
    assert_eq!(sides(&h, id), (8, 8), "5 -> 8 sides gives 8 anchors");
    assert_eq!(history(&h), before + 1, "the drag is one History state");
    drag(&mut h, id, 8.0, &[30.0, 4.0]);
    assert_eq!(sides(&h, id), (4, 4));
    assert_eq!(history(&h), before + 2, "a second drag is a state of its own");

    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(sides(&h, id), (8, 8));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(sides(&h, id), (5, 5));
    h.state_mut().run("edit.redo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(sides(&h, id), (8, 8));
    assert_eq!(h.get_by_role_and_label(Role::Slider, "Sides").accesskit_node().numeric_value(), Some(8.0), "the slider follows undo and redo");
}

#[test]
fn the_slider_takes_whole_sides_from_3_to_100() {
    let (mut h, id) = pentagon();
    // Past either end of the track stops at the range's end.
    let (r, start) = (slider(&h), at(&h, 5.0));
    let y = r.center().y;
    move_to(&mut h, start);
    button(&mut h, start, true);
    move_to(&mut h, pos2(r.right() + 40.0, y));
    assert_eq!(sides(&h, id), (100, 100));
    move_to(&mut h, pos2(r.left() - 40.0, y));
    assert_eq!(sides(&h, id), (3, 3));
    button(&mut h, pos2(r.left() - 40.0, y), false);
    // Between two whole values the nearest one wins.
    drag(&mut h, id, 3.0, &[10.4, 10.6]);
    assert_eq!(sides(&h, id), (11, 11));
}

#[test]
fn a_press_that_leaves_the_side_count_alone_is_no_history_state() {
    let (mut h, id) = pentagon();
    let before = history(&h);
    let knob = at(&h, 5.2);
    move_to(&mut h, knob);
    button(&mut h, knob, true);
    button(&mut h, knob, false);
    assert_eq!(sides(&h, id), (5, 5));
    assert_eq!(history(&h), before);
}

#[test]
fn the_sides_field_moves_the_slider() {
    let (mut h, id) = pentagon();
    let f = field(&h, "Sides");
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sides")).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text("8".into()));
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(sides(&h, id), (8, 8));
    assert_eq!(h.get_by_role_and_label(Role::Slider, "Sides").accesskit_node().numeric_value(), Some(8.0));
    assert_eq!(field(&h, "Sides"), f, "the field stays in place");
}

#[test]
fn a_triangle_has_no_sides_slider() {
    let (h, _) = properties(json!({"kind": "triangle", "rect": [300, 200, 200, 200]}));
    assert!(h.query_all_by_role(Role::Slider).all(|n| n.accesskit_node().label().as_deref() != Some("Sides")));
    assert!(field_named(&h, "Corner radius").is_some() && field_named(&h, "Sides").is_none());
}
