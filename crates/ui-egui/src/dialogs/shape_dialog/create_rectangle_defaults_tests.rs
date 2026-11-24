//! #13339: Create Rectangle opens with Width 100 px, Height 100 px, the radii at the options bar's Radius (0) and From
//! Center off every time, whatever the last OK was. Its Width also takes 0, and OK then closes the dialog and creates
//! nothing. Create Ellipse, Triangle and Polygon still open with their last OK.

use craft::ui::dialog::StackDialog as _;
use serde_json::{Value, json};

use super::tests::{app, click, confirm_by_id, layers, live, only_dialog, set};
use super::*;
use crate::state::Tool;
use photocraft_doc::vector::LiveShape;

/// The fields the open Create Shape dialog lists, as `ui.dialog.get` reads them.
fn fields(app: &PhotocraftApp) -> Value {
    app.framework.dialogs.get::<CreateShapeDialog>().and_then(CreateShapeDialog::json_fields).expect("a Create Shape dialog")
}

/// The active document's History length.
fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn the_next_click_opens_the_defaults_whatever_the_last_ok_was() {
    let mut app = app();
    // The gear's Fixed Size isn't the dialog's size.
    (app.ui.tool_options.rect_constraint, app.ui.tool_options.rect_fixed_width, app.ui.tool_options.rect_fixed_height) = ("fixedSize".into(), 120.0, 60.0);
    let options = serde_json::to_value(&app.ui.tool_options).unwrap();
    click(&mut app, Tool::Rectangle, [200.0, 150.0]);
    let id = only_dialog(&app);
    set(&mut app, id, json!({"width": 150, "height": 80, "radii": [10, 10, 10, 10], "fromCenter": true}));
    confirm_by_id(&mut app, id);
    assert_eq!(live(&app), LiveShape::Rect { rect: [125.0, 110.0, 150.0, 80.0], radii: [10.0; 4] });
    assert_eq!(serde_json::to_value(&app.ui.tool_options).unwrap(), options, "OK changes no tool option");
    for at in [[50.0, 60.0], [300.0, 200.0]] {
        click(&mut app, Tool::Rectangle, at);
        only_dialog(&app);
        let f = fields(&app);
        assert_eq!(
            (f["width"].clone(), f["height"].clone(), f["radii"].clone(), f["linkRadii"].clone(), f["fromCenter"].clone()),
            (json!(100.0), json!(100.0), json!([0.0, 0.0, 0.0, 0.0]), json!(true), json!(false)),
            "the click at {at:?} opens the defaults"
        );
        app.framework.dialogs = craft::ui::dialog::DialogHost::new();
    }
}

#[test]
fn ellipse_triangle_and_polygon_still_open_with_their_last_ok() {
    for tool in [Tool::EllipseShape, Tool::Triangle, Tool::Polygon] {
        let mut app = app();
        click(&mut app, tool, [20.0, 20.0]);
        let id = only_dialog(&app);
        set(&mut app, id, json!({"width": 150, "height": 80, "fromCenter": true}));
        confirm_by_id(&mut app, id);
        click(&mut app, tool, [300.0, 200.0]);
        only_dialog(&app);
        let f = fields(&app);
        assert_eq!((f["width"].clone(), f["height"].clone(), f["fromCenter"].clone()), (json!(150.0), json!(80.0), json!(true)), "{tool:?}");
        // Their Width keeps its 1 px minimum: OK with 0 is refused and the dialog stays.
        let id = only_dialog(&app);
        set(&mut app, id, json!({"width": 0}));
        let _ = crate::dialogs::testing::confirm(&mut app, id);
        assert!(!app.framework.dialogs.is_empty(), "{tool:?}: Width 0 is refused and the dialog stays open");
    }
}

#[test]
fn ok_with_width_zero_closes_and_creates_nothing() {
    for extra in [json!({}), json!({"radii": [10, 10, 10, 10], "fromCenter": true})] {
        let mut app = app();
        let (layers_before, history_before) = (layers(&app), history(&app));
        click(&mut app, Tool::Rectangle, [200.0, 150.0]);
        let id = only_dialog(&app);
        set(&mut app, id, json!({"width": 0}));
        set(&mut app, id, extra.clone());
        confirm_by_id(&mut app, id);
        assert!(app.framework.dialogs.is_empty(), "{extra}: OK closes the dialog");
        assert_eq!((layers(&app), history(&app)), (layers_before, history_before), "{extra}: nothing is created");
        assert!(!app.ui.status_error, "{extra}: {}", app.ui.status);
    }
}

#[test]
fn height_zero_and_a_width_under_a_pixel_are_still_refused() {
    for fields in [json!({"height": 0}), json!({"width": 0.5}), json!({"width": 0, "height": 0})] {
        let mut app = app();
        click(&mut app, Tool::Rectangle, [200.0, 150.0]);
        let id = only_dialog(&app);
        let before = layers(&app);
        set(&mut app, id, fields.clone());
        let _ = crate::dialogs::testing::confirm(&mut app, id);
        assert!(!app.framework.dialogs.is_empty(), "{fields}: the dialog stays open");
        assert_eq!(layers(&app), before, "{fields}: nothing is created");
    }
}

#[test]
fn ok_with_width_zero_over_the_control_channel_creates_nothing() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let call = |app: &mut PhotocraftApp, method: &str, params: Value| {
        let (req, _rx) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Reply::Done(v) => v,
            _ => panic!("deferred"),
        }
    };
    let (layers_before, history_before) = (layers(&app), history(&app));
    crate::canvas::testing::control(
        &mut app,
        "ui.pointer",
        json!({"tool": "rectangle", "events": [{"kind": "down", "x": 40, "y": 50}, {"kind": "up", "x": 40, "y": 50}]}),
    );
    let id = only_dialog(&app);
    call(&mut app, "ui.dialog.set", json!({"dialog": id.value(), "field": "width", "value": 0}));
    let r = call(&mut app, "ui.dialog.confirm", json!({"dialog": id.value()}));
    assert_eq!(r["ok"], json!(true), "{r}");
    assert!(app.framework.dialogs.is_empty());
    assert_eq!((layers(&app), history(&app)), (layers_before, history_before));
    assert!(!app.ui.status_error, "{}", app.ui.status);
}
