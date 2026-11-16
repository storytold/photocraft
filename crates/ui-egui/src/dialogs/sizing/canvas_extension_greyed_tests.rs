//! #14546: Canvas Size greys the "Canvas extension color" label and dropdown in a document
//! without a Background layer; the params the dialog sends stay as they were.

use super::*;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const LABEL: &str = "Canvas extension color:";

fn app_with(background: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300, "background": background})).unwrap();
    app
}

fn dropdown_disabled(app: PhotocraftApp) -> bool {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "image.canvasSize").unwrap();
    h.run_steps(3);
    let combos: Vec<_> = h.query_all_by_role(Role::ComboBox).filter(|n| n.accesskit_node().label().as_deref() == Some(LABEL)).collect();
    assert_eq!(combos.len(), 1, "one extension colour dropdown");
    combos.iter().all(|n| n.accesskit_node().is_disabled())
}

#[test]
fn extension_colour_is_greyed_without_a_background() {
    let app = app_with("transparent");
    assert!(app.session.active().unwrap().doc.background().is_none(), "the transparent document has no Background");
    assert!(dropdown_disabled(app), "the extension colour dropdown is disabled without a Background");
}

#[test]
fn extension_colour_is_enabled_with_a_background() {
    let app = app_with("white");
    assert!(app.session.active().unwrap().doc.background().is_some(), "the white document has a Background");
    assert!(!dropdown_disabled(app), "the extension colour dropdown is enabled with a Background");
}

/// Grow by 100 x 100 (top left anchor) and return the params the dialog sends (its private `__`
/// keys stripped) and the document after OK.
fn grow(mut app: PhotocraftApp) -> (Value, PhotocraftApp) {
    let id = open(&mut app, "image.canvasSize").unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("width".into(), json!(500));
    d.fields.insert("height".into(), json!(400));
    d.fields.insert("anchor".into(), json!("topLeft"));
    let params = crate::filter_dialog::params_of(&d.fields);
    crate::dialogs::confirm(&mut app, id).unwrap();
    (params, app)
}

#[test]
fn ok_sends_the_same_params_with_and_without_a_background() {
    let (with, with_app) = grow(app_with("white"));
    let (without, without_app) = grow(app_with("transparent"));
    assert_eq!(with, without, "the dialog's params do not depend on the Background");
    assert!(with.as_object().unwrap().keys().all(|k| !k.starts_with("__")), "{with}");
    assert_eq!(with.get("extensionColor"), Some(&json!("background")));
    for app in [&with_app, &without_app] {
        assert_eq!(app.session.active().unwrap().doc.size, photocraft_doc::Size::new(500, 400));
    }
}

#[test]
fn growing_without_a_background_leaves_the_new_area_transparent() {
    let (_, app) = grow(app_with("transparent"));
    let doc = &app.session.active().unwrap().doc;
    assert!(doc.background().is_none());
    let mut seen = 0;
    for layer in &doc.layers {
        if let Some(surface) = layer.surface() {
            for (x, y) in [(0, 0), (499, 0), (0, 399), (499, 399), (450, 350)] {
                assert_eq!(surface.pixel(x, y).last().copied(), Some(0.0), "alpha at ({x}, {y})");
                seen += 1;
            }
        }
    }
    assert!(seen > 0, "the document has a pixel layer to look at");
}
