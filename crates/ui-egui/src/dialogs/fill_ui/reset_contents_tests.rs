//! #14642: Fill opens (and Alt+Reset returns) on the fill-from-surroundings entry, as the
//! reference application does. Remembered choices after OK still win.

use super::*;
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000", "background": "#0000ff"})).unwrap();
    app
}

#[test]
fn the_defaults_open_on_content_aware_with_the_measured_options() {
    let app = app();
    let f = default_fields(&app);
    assert_eq!(f["contents"], json!("contentAware"));
    assert_eq!(f["colorAdaptation"], json!(true));
    assert_eq!(f["mode"], json!("normal"));
    assert_eq!(f["opacity"].as_f64(), Some(100.0));
    assert_eq!(f["preserveTransparency"], json!(false));
}

#[test]
fn reset_returns_to_content_aware_and_keeps_the_dialog_open() {
    let mut app = app();
    let id = open(&mut app);
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("contents".into(), json!("white"));
    d.fields.insert("colorAdaptation".into(), json!(false));
    d.fields.insert("mode".into(), json!("multiply"));
    d.fields.insert("opacity".into(), json!(40.0));
    d.fields.insert("preserveTransparency".into(), json!(true));
    let mut fields = d.fields.clone();
    reset(&app, &mut fields);
    app.ui.dialog_mut(id).unwrap().fields = fields;
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["contents"], json!("contentAware"), "Alt+Reset sets the fill-from-surroundings entry");
    assert_eq!((f["colorAdaptation"].clone(), f["mode"].clone()), (json!(true), json!("normal")));
    assert_eq!(f["opacity"].as_f64(), Some(100.0));
    assert_eq!(f["preserveTransparency"], json!(false));
    assert!(app.ui.dialogs.iter().any(|d| d.id == id), "Reset keeps the dialog open");
}

#[test]
fn remembered_choices_still_win_over_the_default() {
    let mut app = app();
    app.session.prefs.edit(|p| p.dialogs.insert(COMMAND.into(), json!({"contents": "white"})));
    let f = fields(&app);
    assert_eq!(f["contents"], json!("white"));
}
