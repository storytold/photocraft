//! Escape in the Brush Preset picker goes back to the opening brush with one `tools.setBrush` and no history step; one
//! drag on the tip widget is one `tools.setBrush`.

use crate::app::framework::BuildApp as _;
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.0, "angle": 0.0, "roundness": 1.0}})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).with_context(app.ctx().clone()).build_app(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            // The canvas shows the picker itself (`canvas_tool_menu::run`); a second `show_at` would show it twice.
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    let c = h.state().last_canvas_rect.center();
    crate::brush_picker::open(h.state_mut(), [c.x, c.y]);
    h.run_steps(3);
    h
}

/// Type `text` into the picker's `n`th spin field (0 Size, 1 Hardness), then Tab.
fn type_into(h: &mut Harness<'static, Box<PhotocraftApp>>, n: usize, text: &str) {
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(n).expect("the field").click();
    h.run_steps(1);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.event(Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
}

fn brush(h: &Harness<'static, Box<PhotocraftApp>>) -> (f32, f32, f32) {
    let b = &h.state().session.tools.brush;
    (b.size, b.hardness, b.angle)
}

fn journal_ids(h: &Harness<'static, Box<PhotocraftApp>>, from: usize) -> Vec<String> {
    h.state().session.journal[from..].iter().map(|(id, _)| id.clone()).collect()
}

#[test]
fn escape_reverts_typed_size_and_hardness() {
    let mut h = harness();
    let steps = h.state().session.active().unwrap().history.past_len();
    type_into(&mut h, 0, "60");
    type_into(&mut h, 1, "100");
    assert_eq!(brush(&h).0, 60.0, "the typed size reached the brush");
    let journal = h.state().session.journal.len();
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker, None);
    let b = brush(&h);
    assert_eq!((b.0, b.1), (20.0, 0.0), "back to the brush the picker opened with");
    assert_eq!(journal_ids(&h, journal), ["tools.setBrush"], "the revert is one command");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "no history step");
}

#[test]
fn dragging_the_arrow_sets_the_angle_in_one_command() {
    let mut h = harness();
    let c = h.get_by_label("Tip angle and roundness").rect().center();
    // On the arrow (angle 0 points right), away from the roundness dots.
    let from = c + vec2(28.0, 0.0);
    let path = [c + vec2(10.0, -25.0), c + vec2(-20.0, -20.0)];
    let journal = h.state().session.journal.len();
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for p in path {
        h.event(Event::PointerMoved(p));
        h.run_steps(1);
    }
    let end: Pos2 = path[1];
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(brush(&h).2, 135.0, "the angle of the pointer's last position");
    assert_eq!(journal_ids(&h, journal), ["tools.setBrush"], "one journaled change for the drag");
}
