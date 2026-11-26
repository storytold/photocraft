//! #13230: a Width or Height typed past 1..300000 px in Create Ellipse keeps the closest end and, once the field is
//! left, one "Closest value inserted" alert with OK says so, titled "Create Ellipse" as the reference's is. The other
//! Create Shape dialogs weren't measured and keep the app's name on their alert.

use craft::draw::ui::shape_drag::CreateShapeDialog;
use craft::ui::dialog::StackDialog as _;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::state::Tool;
use crate::{PhotocraftApp, Services};

type H = Harness<'static, Box<PhotocraftApp>>;

/// A new document, a click with `tool` and the Create dialog it opens, shown through `dialogs::run`.
fn dialog(tool: Tool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.ui.tool = tool;
    crate::canvas::testing::click(&mut app, [50.0, 50.0], egui::Modifiers::NONE);
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(egui::vec2(1200.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    assert!(h.state().framework.dialogs.get::<CreateShapeDialog>().is_some(), "{tool:?}: the click opens a Create dialog");
    h
}

/// Types `text` into the number field named `name` and leaves it with Tab.
fn type_into(h: &mut H, name: &str, text: &str) {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().as_deref().is_some_and(|l| l.trim_end_matches(':') == name))
        .unwrap_or_else(|| panic!("the {name} field"))
        .click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
}

/// The Create dialog's `field` ("width" or "height") in px.
fn size(h: &H, field: &str) -> f64 {
    let fields = h.state().framework.dialogs.get::<CreateShapeDialog>().and_then(|d| d.json_fields()).expect("the Create dialog is open");
    fields[field].as_f64().unwrap_or_else(|| panic!("{field}: {fields}"))
}

/// The titles of the alerts on screen.
fn alert_titles(h: &H) -> Vec<String> {
    h.query_all_by_role(Role::AlertDialog).filter_map(|n| n.accesskit_node().label()).collect()
}

/// The messages of the open dialogs that carry one: the alerts.
fn messages(h: &H) -> Vec<String> {
    h.state().framework.dialogs.open_dialogs().iter().filter_map(|d| d.fields.as_ref()?.get("message")?.as_str().map(str::to_owned)).collect()
}

/// Clicks OK on the alert, not on the Create dialog under it.
fn ok_on_the_alert(h: &mut H) {
    h.get_by_role(Role::AlertDialog).get_by_label("OK").click();
    h.run_steps(4);
}

#[test]
fn a_size_typed_past_the_range_alerts_once_under_the_dialogs_own_title() {
    for field in ["Width", "Height"] {
        let key = field.to_lowercase();
        for (typed, end) in [("-5", 1.0), ("0", 1.0), ("400000", 300_000.0)] {
            let what = format!("{field} {typed}");
            let mut h = dialog(Tool::EllipseShape);
            type_into(&mut h, field, typed);
            assert_eq!(size(&h, &key), end, "{what}: the closest end is kept");
            assert_eq!(alert_titles(&h), ["Create Ellipse"], "{what}: one alert, titled as the dialog");
            assert!(messages(&h).iter().any(|m| m.contains("Closest value inserted")), "{what}: {:?}", messages(&h));
            h.run_steps(4);
            assert_eq!(alert_titles(&h).len(), 1, "{what}: one alert, not one per frame");
            ok_on_the_alert(&mut h);
            assert!(alert_titles(&h).is_empty(), "{what}: OK closes the alert");
            assert_eq!(size(&h, &key), end, "{what}: the closest end stays, with Create Ellipse still open");
            type_into(&mut h, field, "250");
            assert!(alert_titles(&h).is_empty(), "{what}, then 250: a size in range raises no alert");
        }
    }
}

#[test]
fn the_other_create_dialogs_keep_the_apps_name_on_their_alert() {
    for tool in [Tool::Rectangle, Tool::Triangle, Tool::Polygon] {
        let mut h = dialog(tool);
        type_into(&mut h, "Width", "400000");
        assert_eq!(alert_titles(&h), ["PhotoCraft"], "{tool:?}: not measured, so unchanged");
    }
}
