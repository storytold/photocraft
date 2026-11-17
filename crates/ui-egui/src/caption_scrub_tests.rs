//! Shared steps for the options-bar tests that scrub a numeric field's caption past the end of
//! its range: a scrub stops at the end and never raises the range alert.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::PhotocraftApp;

/// Clicks the spin button labelled `label` (for example "Strength:") so it holds the keyboard,
/// and returns its rectangle.
pub(crate) fn click_field(h: &mut Harness<'static, PhotocraftApp>, label: &str) -> egui::Rect {
    let field =
        h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("the {label} field"));
    let rect = field.rect();
    field.click();
    h.run_steps(1);
    rect
}

/// Drags the caption that sits `inset` points left of the field at `field` by `dx` in six steps,
/// releases it, then leaves the field with Tab.
pub(crate) fn scrub_caption(h: &mut Harness<'static, PhotocraftApp>, field: egui::Rect, inset: f32, dx: f32) {
    let primary = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let from = egui::pos2(field.left() - inset, field.center().y);
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(primary(from, true));
    h.run_steps(1);
    let mut at = from;
    for _ in 0..6 {
        at.x += dx;
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
    }
    h.event(primary(at, false));
    h.run_steps(3);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}
