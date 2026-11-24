//! #13316: the Properties panel of a live polygon lists Sides first, as a slider bound to its
//! number field (whole numbers, 3-100), then Corner radius, then Star ratio, as the reference
//! does. The shape follows the slider while it is dragged, and one drag is one History state.
//! The row is the framework's `ShapePanel` (storytold/craft-libs); these tests drive it through
//! `shape_properties`, which turns its edits into `shape.edit` commands.

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::{Value, json};

use super::shape_properties;
use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The Properties panel, 320 px wide like the dock, for a new shape `create` makes in an
/// 800 x 600 document.
fn properties(create: Value) -> (H, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let made = app.run("shape.create", create).unwrap();
    let id = LayerId(made["layer"].as_u64().unwrap());
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(320.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
            // The first frame runs before `setup_context` installs the app's fonts.
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                shape_properties(app, ui, id);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro_medium());
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

/// The count the Sides slider reports.
fn slider_value(h: &H) -> Option<f64> {
    h.get_by_role_and_label(Role::Slider, "Sides").accesskit_node().numeric_value()
}

fn field_named(h: &H, label: &str) -> Option<Rect> {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).map(|n| n.rect())
}

fn field(h: &H, label: &str) -> Rect {
    field_named(h, label).unwrap_or_else(|| panic!("the {label} field"))
}

/// The point `fraction` of the way along the Sides slider, on its middle line.
fn along(h: &H, fraction: f32) -> Pos2 {
    let r = slider(h);
    pos2(r.left() + fraction * r.width(), r.center().y)
}

fn button(h: &mut H, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn move_to(h: &mut H, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
}

/// Drags the slider from `from` through each of `through` (fractions of its width), checking
/// after each step that the shape already has the count the slider shows, as sides and as
/// anchors, then releases. The count after each step.
fn drag(h: &mut H, id: LayerId, from: f32, through: &[f32]) -> Vec<u32> {
    let start = along(h, from);
    move_to(h, start);
    button(h, start, true);
    let mut last = start;
    let mut counts = Vec::new();
    for &f in through {
        last = along(h, f);
        move_to(h, last);
        let shown = slider_value(h).expect("the slider's count");
        assert_eq!(shown.fract(), 0.0, "the slider takes whole sides: {shown}");
        let want = shown as u32;
        assert_eq!(sides(h, id), (want, want as usize), "the shape follows the slider at {f}");
        counts.push(want);
    }
    button(h, last, false);
    counts
}

#[test]
fn the_sides_slider_and_field_come_before_corner_radius_then_star_ratio() {
    let (h, _) = pentagon();
    assert_eq!(slider_value(&h), Some(5.0));
    let (s, f, radius, star) = (slider(&h), field(&h, "Sides"), field(&h, "Corner radius"), field(&h, "Star ratio"));
    assert!((s.center().y - f.center().y).abs() < 1.0, "the slider and its field share a row: {s:?} {f:?}");
    assert!(s.right() <= f.left() && s.width() >= 60.0, "the slider comes before its field: {s:?} {f:?}");
    assert!(f.bottom() <= radius.top(), "Sides comes before Corner radius: {f:?} {radius:?}");
    assert!(radius.bottom() <= star.top(), "Corner radius comes before Star ratio: {radius:?} {star:?}");
}

#[test]
fn one_drag_up_and_one_drag_down_are_one_history_state_each() {
    let (mut h, id) = pentagon();
    let before = history(&h);
    let first = drag(&mut h, id, 0.03, &[0.5, 0.1]);
    let up = *first.last().unwrap();
    assert!(first[0] > up && up > 5, "the drag went up, then back down: {first:?}");
    assert_eq!(sides(&h, id), (up, up as usize), "the polygon has the count's anchors");
    assert_eq!(history(&h), before + 1, "the drag is one History state");
    let second = drag(&mut h, id, 0.1, &[0.8, 0.02]);
    let down = *second.last().unwrap();
    assert!(down < up, "the second drag went down: {second:?}");
    assert_eq!(history(&h), before + 2, "a second drag is a state of its own");

    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(sides(&h, id), (up, up as usize));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(sides(&h, id), (5, 5));
    h.state_mut().run("edit.redo", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(sides(&h, id), (up, up as usize));
    assert_eq!(slider_value(&h), Some(f64::from(up)), "the slider follows undo and redo");
}

#[test]
fn the_slider_takes_whole_sides_from_3_to_100() {
    let (mut h, id) = pentagon();
    // Past either end of the track stops at the range's end, in one drag.
    let (r, start) = (slider(&h), along(&h, 0.5));
    let y = r.center().y;
    move_to(&mut h, start);
    button(&mut h, start, true);
    move_to(&mut h, pos2(r.right() + 40.0, y));
    assert_eq!(sides(&h, id), (100, 100));
    move_to(&mut h, pos2(r.left() - 40.0, y));
    assert_eq!(sides(&h, id), (3, 3));
    button(&mut h, pos2(r.left() - 40.0, y), false);
    // Anywhere along the track the count is whole (checked in `drag`).
    drag(&mut h, id, 0.0, &[0.137, 0.291, 0.443, 0.617, 0.759]);
}

#[test]
fn a_press_that_leaves_the_side_count_alone_is_no_history_state() {
    let (mut h, id) = pentagon();
    let before = history(&h);
    // A press on the track jumps the thumb there: one state.
    let at = along(&h, 0.4);
    move_to(&mut h, at);
    button(&mut h, at, true);
    button(&mut h, at, false);
    let jumped = sides(&h, id).0;
    assert_ne!(jumped, 5);
    assert_eq!(history(&h), before + 1);
    // A second press at the same place lands on the thumb and changes nothing.
    button(&mut h, at, true);
    button(&mut h, at, false);
    assert_eq!(sides(&h, id).0, jumped);
    assert_eq!(history(&h), before + 1, "no state for a press that keeps the count");
}

#[test]
fn the_sides_field_moves_the_slider() {
    let (mut h, id) = pentagon();
    let f = field(&h, "Sides");
    let before = history(&h);
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Sides")).unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text("8".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(sides(&h, id), (8, 8));
    assert_eq!(history(&h), before + 1, "a typed count is one History state");
    assert_eq!(slider_value(&h), Some(8.0));
    assert_eq!(field(&h, "Sides"), f, "the field stays in place");
}

#[test]
fn a_triangle_has_no_sides_slider() {
    let (h, _) = properties(json!({"kind": "triangle", "rect": [300, 200, 200, 200]}));
    assert!(h.query_all_by_role(Role::Slider).all(|n| n.accesskit_node().label().as_deref() != Some("Sides")));
    assert!(field_named(&h, "Corner radius").is_some() && field_named(&h, "Sides").is_none());
}
