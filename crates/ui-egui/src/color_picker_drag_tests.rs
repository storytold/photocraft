//! The Color Picker moves by its whole top strip, as the reference application's title bar does: a drag in the
//! frame's padding above the title or beside it moves the dialog, not only a drag on the title's line (#17112).

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::color_picker_ui::title_for;

/// How far each drag moves the pointer.
const DRAG: Vec2 = vec2(30.0, 20.0);

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        crate::color_picker_ui::open(&mut app, "foreground");
        app
    });
    h.run_steps(8);
    h
}

fn picker(h: &Harness<'static, Box<PhotocraftApp>>) -> egui::Rect {
    h.get_by_role_and_label(Role::Dialog, &title_for("Foreground Color")).rect()
}

/// Presses at `at`, drags by [`DRAG`] and releases.
fn drag(h: &mut Harness<'static, Box<PhotocraftApp>>, at: Pos2) {
    let press = |pressed, pos| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let events = [Event::PointerMoved(at), press(true, at), Event::PointerMoved(at + DRAG * 0.5), Event::PointerMoved(at + DRAG), press(false, at + DRAG)];
    for event in events {
        h.event(event);
        h.run_steps(2);
    }
}

#[test]
fn the_color_picker_moves_by_the_padding_above_and_beside_its_title() {
    let mut h = harness();
    // The frame's padding, from the picker's top-left corner: 6 points inside the top edge (at the middle), and 6
    // points inside the left edge at the title's height.
    let width = picker(&h).width();
    let spots = [("above the title", vec2(width / 2.0, 6.0)), ("before the title", vec2(6.0, 30.0))];
    for (what, spot) in spots {
        let before = picker(&h);
        drag(&mut h, before.min + spot);
        assert_eq!(picker(&h).min - before.min, DRAG, "a drag in the padding {what} moves the Color Picker");
    }
    let before = picker(&h);
    drag(&mut h, before.center());
    assert_eq!(picker(&h).min, before.min, "a drag in the picker's body does not move it");
}
