//! #14577: Artboard from Layers' Width and Height take 4 to 32000 px. A value typed outside is
//! clamped when the field is left (Tab): 99999 becomes 32000, 1 becomes 4.

use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::{PhotocraftApp, Services};

const CMD: &str = "layer.new.artboardFromLayers";

/// A 100 x 80 document whose new layer holds 20 x 30 px of red, with the dialog open over it.
fn dialog() -> (Harness<'static, Box<PhotocraftApp>>, craft::ui::dialog::DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 10, "y": 5, "width": 20, "height": 30})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let id = super::open(&mut app, CMD).unwrap();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    (h, id)
}

fn int(h: &Harness<'static, Box<PhotocraftApp>>, id: craft::ui::dialog::DialogId, key: &str) -> Option<i64> {
    h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().int(key))
}

/// The open alerts' messages.
fn alerts(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_string)).collect()
}

/// Types `text` into the number field labelled `name` and leaves it with Tab.
fn type_and_tab(h: &mut Harness<'static, Box<PhotocraftApp>>, name: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == name))
        .unwrap_or_else(|| panic!("the {name} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

#[test]
fn width_and_height_clamp_to_4_to_32000_when_the_field_is_left() {
    // (field, key, typed, kept)
    for (name, key, typed, kept) in [("Width", "width", "99999", 32_000), ("Height", "height", "1", 4), ("Width", "width", "640", 640)] {
        let (mut h, id) = dialog();
        assert_eq!((int(&h, id, "width"), int(&h, id, "height")), (Some(20), Some(30)), "opens at the layers' size");
        type_and_tab(&mut h, name, typed);
        assert_eq!(int(&h, id, key), Some(kept), "{name} {typed}");
        // The framework's range alert says the closest value was kept; OK on it keeps the value.
        if typed.parse::<i64>().ok() != Some(kept) {
            let shown = alerts(&h);
            assert!(shown.iter().any(|m| m.starts_with("An integer between 4 and ") && m.contains("Closest value inserted")), "{name} {typed}: {shown:?}");
        }
    }
}

/// A union of 30000 to 32000 px fits the field now, so the dialog opens at it.
#[test]
fn a_union_up_to_32000_px_opens_at_its_size() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 31_000, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = super::open(&mut app, CMD).unwrap();
    let f = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
    assert_eq!((f.int("width"), f.int("height")), (Some(31_000), Some(8)));
}

#[test]
fn a_union_wider_than_32000_px_still_opens_at_0_to_keep_the_bounds() {
    // Not measured in the reference: today's 0 x 0 ("keep the layers' bounds") stays, now past 32000 px.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 33_000, "height": 8})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let id = super::open(&mut app, CMD).unwrap();
    let f = app.framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
    assert_eq!((f.int("width"), f.int("height")), (Some(0), Some(0)));
}
