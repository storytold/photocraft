//! #11970: committing a Perspective Crop quad the crop can't use (crossing sides, no area) opens
//! the reference application's alert with Cancel (the default, also Enter and Esc) and Don't
//! Crop, instead of the engine's reason in the status bar. Cancel keeps the quad for fixing, Don't
//! Crop ends the session; neither touches the document. A convex quad still commits as before.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;

/// Top-right and bottom-right swapped: the sides cross.
const CROSSED: [[f64; 2]; 4] = [[20.0, 15.0], [140.0, 110.0], [150.0, 20.0], [25.0, 100.0]];
const CONVEX: [[f64; 2]; 4] = [[20.0, 15.0], [150.0, 20.0], [140.0, 110.0], [25.0, 100.0]];

/// A 160 × 120 document with the Perspective Crop tool and `quad` pending.
fn setup(quad: [[f64; 2]; 4]) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 160, "height": 120})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app.ui.perspective_crop = Some(quad);
    app
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn size(app: &PhotocraftApp) -> (u32, u32) {
    let d = &app.session.active().unwrap().doc;
    (d.size.width, d.size.height)
}

fn alerts(app: &PhotocraftApp) -> Vec<u64> {
    app.ui.dialogs.iter().filter(|d| owns_invalid(&d.fields)).map(|d| d.id).collect()
}

/// ↵ on the canvas, through the tool's key handling.
fn enter(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    ctx.begin_pass(egui::RawInput { events: vec![key], ..Default::default() });
    assert!(keys(app, &ctx), "the tool takes ↵ while a quad is pending");
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn enter_on_a_crossed_quad_opens_the_alert() {
    let mut app = setup(CROSSED);
    let before = steps(&app);
    enter(&mut app);
    let open = alerts(&app);
    assert_eq!(open.len(), 1, "the alert opens");
    let d = app.ui.dialogs.iter().find(|d| d.id == open[0]).unwrap();
    assert_eq!(d.fields["message"], INVALID_MESSAGE);
    assert_eq!(
        INVALID_MESSAGE,
        "Could not use the perspective crop tool because the center point is not correctly placed or the corners are not properly selected."
    );
    assert!(!app.ui.status_error, "no status-bar error as well: {}", app.ui.status);
    assert_eq!((app.ui.perspective_crop, steps(&app), size(&app)), (Some(CROSSED), before, (160, 120)));
    // The options bar's commit button and the other ways in go through `commit`: one alert.
    commit(&mut app);
    assert_eq!(alerts(&app).len(), 1);
}

#[test]
fn a_quad_without_area_gets_the_alert_too() {
    let mut app = setup([[50.0, 50.0]; 4]);
    commit(&mut app);
    assert_eq!(alerts(&app).len(), 1);
}

#[test]
fn cancel_keeps_the_quad_for_fixing() {
    // Cancel is the default button: OK on the dialog answers Cancel.
    let mut app = setup(CROSSED);
    let before = steps(&app);
    commit(&mut app);
    let id = alerts(&app)[0];
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(alerts(&app).is_empty());
    assert_eq!((app.ui.tool, app.ui.perspective_crop, steps(&app)), (Tool::PerspectiveCrop, Some(CROSSED), before));
    // Esc closes it the same way.
    commit(&mut app);
    let id = alerts(&app)[0];
    crate::dialogs::cancel(&mut app, id).unwrap();
    assert_eq!((app.ui.perspective_crop, steps(&app)), (Some(CROSSED), before));
}

#[test]
fn dont_crop_ends_the_session_without_cropping() {
    let mut app = setup(CROSSED);
    let before = steps(&app);
    commit(&mut app);
    let id = alerts(&app)[0];
    app.ui.dialog_mut(id).unwrap().fields.insert("choice".into(), json!("dontCrop"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(!pending(&app), "the session ended");
    assert_eq!((app.ui.perspective_crop, steps(&app), size(&app)), (None, before, (160, 120)));
}

#[test]
fn a_convex_quad_still_commits_without_an_alert() {
    let mut app = setup(CONVEX);
    let before = steps(&app);
    enter(&mut app);
    assert!(alerts(&app).is_empty());
    assert_eq!(steps(&app), before + 1);
    assert!(app.ui.perspective_crop.is_none());
}

#[test]
fn the_buttons_are_cancel_then_dont_crop_and_enter_picks_cancel() {
    let mut app = setup(CROSSED);
    commit(&mut app);
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    if !h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        return;
    }
    let (cancel, dont) = (h.get_by_label("Cancel").rect().center().x, h.get_by_label("Don't Crop").rect().center().x);
    // Cancel is the default button: first elsewhere, last (in the corner) on macOS.
    let mac = h.ctx.os() == egui::os::OperatingSystem::Mac;
    assert_eq!(cancel < dont, !mac, "Cancel at {cancel}, Don't Crop at {dont}");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(alerts(h.state()).is_empty(), "Enter answers");
    assert_eq!(h.state().ui.perspective_crop, Some(CROSSED), "with Cancel");
}
