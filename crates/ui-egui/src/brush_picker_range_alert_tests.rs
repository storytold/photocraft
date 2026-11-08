//! #11817: the Brush Preset picker's Size and Hardness take whole numbers; a value typed past the
//! range becomes the closest one and, once the field is left, an alert with OK says so. The
//! picker stays open after OK.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::brush_picker::{HARDNESS_RANGE_ALERT, SIZE_RANGE_ALERT};
use crate::state::DialogKind;
use crate::{PhotocraftApp, Services, Tool};

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 0.5}})).unwrap();
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(5);
    let ctx = h.ctx.clone();
    let c = h.state().last_canvas_rect.center();
    crate::brush_picker::open(&mut h.state_mut().ui, &ctx, [c.x, c.y]);
    h.run_steps(3);
    h
}

/// Type `text` into the picker field named `name`, then Tab out of it.
fn type_into(h: &mut Harness<'static, PhotocraftApp>, name: &str, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some(name)).expect("the field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alert(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).and_then(|d| d.fields.get("message")).and_then(|m| m.as_str()).map(str::to_owned)
}

fn size(h: &Harness<'static, PhotocraftApp>) -> f32 {
    h.state().session.tools.brush.size
}

#[test]
fn size_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("9999", 5000.0), ("0", 1.0), ("-5", 1.0)] {
        let mut h = harness();
        type_into(&mut h, "Size", typed);
        assert_eq!(size(&h), want, "{typed}");
        assert_eq!(alert(&h).as_deref(), Some(SIZE_RANGE_ALERT), "{typed}");
        assert!(h.state().ui.brush_picker.is_some(), "{typed}: the picker stays open under the alert");
        h.get_by_label("OK").click();
        h.run_steps(4);
        assert_eq!(alert(&h), None, "{typed}: OK closes the alert");
        assert!(h.state().ui.brush_picker.is_some(), "{typed}: the picker stays open after OK");
    }
}

#[test]
fn size_in_range_cuts_fractions_without_an_alert() {
    let mut h = harness();
    type_into(&mut h, "Size", "2.5");
    assert_eq!(size(&h), 2.0);
    assert_eq!(alert(&h), None);
    type_into(&mut h, "Size", "abc");
    assert_eq!(size(&h), 2.0, "not a number: the value stays");
    assert_eq!(alert(&h), None);
}

#[test]
fn hardness_past_the_range_takes_100_and_alerts() {
    let mut h = harness();
    type_into(&mut h, "Hardness", "150");
    assert_eq!(h.state().session.tools.brush.hardness, 1.0);
    assert_eq!(alert(&h).as_deref(), Some(HARDNESS_RANGE_ALERT));
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(h.state().ui.brush_picker.is_some());
    type_into(&mut h, "Hardness", "33.7");
    assert_eq!(h.state().session.tools.brush.hardness, 0.33, "a whole percent");
    assert_eq!(alert(&h), None);
}
