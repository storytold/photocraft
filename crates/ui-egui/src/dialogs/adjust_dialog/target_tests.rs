//! #10091: the Levels and Curves eyedroppers' target colours in the modal dialogs.

use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn app_in(depth: u32, fill: [f32; 3]) -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
    s.execute("edit.fill", json!({"color": fill})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

fn arm(app: &mut PhotocraftApp, id: u64, picker: &str) {
    let dialog = app.ui.dialog_mut(id).unwrap();
    let base = crate::filter_dialog::params_of(&dialog.fields);
    dialog.fields.insert(CURVE_PICKER.into(), json!(picker));
    dialog.fields.insert(CURVE_PICKER_BASE.into(), base);
}

fn dialog_harness(app: PhotocraftApp, dt: f32) -> Harness<'static, PhotocraftApp> {
    let h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_step_dt(dt).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h
}

/// Two quick clicks on the centre of `rect`, after the last gesture's double-click time.
fn double_click(h: &mut Harness<'_, PhotocraftApp>, rect: egui::Rect) {
    h.run_steps(40);
    let at = rect.center();
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }
    h.run_steps(3);
}

/// Double-clicking a Set Black / Gray / White Point eyedropper in the Curves and Levels dialogs
/// opens the Color Picker over the dialog on that point's target colour, titled as in Photoshop,
/// and arms the eyedropper. A single click only arms it.
#[test]
fn double_clicking_an_eyedropper_opens_its_target_color_picker() {
    let cases = [
        ("Set Black Point", "black", "Select target shadow color:", "#000000"),
        ("Set Neutral Gray Point", "gray", "Select target midtone color:", "#808080"),
        ("Set White Point", "white", "Select target highlight color:", "#ffffff"),
    ];
    for command in ["image.adjustments.curves", "image.adjustments.levels"] {
        for (label, point, title, color) in cases {
            let mut h = dialog_harness(app_in(8, [0.5; 3]), 1.0 / 60.0);
            let id = open(h.state_mut(), command).unwrap();
            h.run_steps(3);
            let rect = h.get_by_label(label).rect();
            double_click(&mut h, rect);
            let app = h.state();
            let top = app.ui.dialogs.last().unwrap();
            assert_ne!(top.id, id, "{command} {point}: the picker opens over the dialog");
            let target = format!("eyedropperTarget.{point}");
            assert_eq!(top.fields.get("__colorPicker").and_then(Value::as_str), Some(target.as_str()));
            assert_eq!(top.fields.get("__label").and_then(Value::as_str), Some(title));
            assert_eq!(top.fields.get("color").and_then(Value::as_str), Some(color));
            let dialog = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
            assert_eq!(dialog.fields.get(CURVE_PICKER).and_then(Value::as_str), Some(point), "{command}: armed");
            assert!(!dialog.fields.contains_key("__pickTarget"), "the request is consumed");
        }
    }
    let mut h = dialog_harness(app_in(8, [0.5; 3]), 1.0 / 60.0);
    open(h.state_mut(), "image.adjustments.curves").unwrap();
    h.run_steps(3);
    h.get_by_label("Set White Point").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1, "no picker on a single click");
    assert!(picker_armed(h.state()));
}

/// OK in the target picker stores the colour in the `targetColors` preference (which the swatch
/// then shows); Cancel leaves it. Unknown points open nothing and fail without panicking.
#[test]
fn target_picker_ok_stores_the_target_and_cancel_keeps_it() {
    let mut app = app_in(8, [0.5; 3]);
    let id = crate::layer_picker::open_target_picker(&mut app, "black").unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#203040"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().target_colors.shadows, "#203040");
    assert_eq!(crate::layer_picker::target_rgb(&app, "black"), Some([32.0 / 255.0, 48.0 / 255.0, 64.0 / 255.0]));
    assert_eq!(crate::layer_picker::target_swatches(&app)[0], egui::Color32::from_rgb(32, 48, 64));

    let id = crate::layer_picker::open_target_picker(&mut app, "black").unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields.get("color").and_then(Value::as_str), Some("#203040"), "opens on the stored target");
    app.ui.dialog_mut(id).unwrap().fields.insert("color".into(), json!("#ff0000"));
    app.ui.close_dialog(id);
    assert_eq!(app.session.prefs().target_colors.shadows, "#203040", "Cancel keeps the target");
    assert_eq!(app.session.prefs().target_colors.highlights, "#ffffff", "other targets untouched");

    assert!(crate::layer_picker::open_target_picker(&mut app, "purple").is_none());
    assert!(crate::layer_picker::target_rgb(&app, "").is_none());
    assert!(crate::layer_picker::confirm_target(&mut app, "eyedropperTarget.purple", "#000000").unwrap().is_err());
    assert!(crate::layer_picker::confirm_target(&mut app, "eyedropperTarget.white", "not a colour").unwrap().is_err());
    assert!(crate::layer_picker::confirm_target(&mut app, "cropShield", "#000000").is_none());
    let mut fields = Map::new();
    crate::layer_picker::request_target(&mut fields, "nope");
    let d = app.ui.open_dialog(crate::state::DialogKind::Command, fields);
    crate::layer_picker::open_requested_target(&mut app, d);
    assert_eq!(app.ui.dialogs.last().map(|d| d.id), Some(d), "an unknown request opens nothing");
}

/// The dialog's eyedroppers map the sampled pixel to the stored target at 8, 16 and 32 bits.
#[test]
fn dialog_eyedroppers_map_the_sample_to_the_stored_target_at_every_depth() {
    for depth in [8, 16, 32] {
        for command in ["image.adjustments.curves", "image.adjustments.levels"] {
            let mut app = app_in(depth, [0.6, 0.5, 0.4]);
            app.run("prefs.set", json!({"path": "targetColors.highlights", "value": "#f0e0c8"})).unwrap();
            let id = open(&mut app, command).unwrap();
            arm(&mut app, id, "white");
            assert!(sample_at(&mut app, 8.0, 8.0));
            crate::dialogs::confirm(&mut app, id).unwrap();
            let px: Vec<f32> = serde_json::from_value(app.run("document.pixel", json!({"x": 8, "y": 8})).unwrap()).unwrap();
            let want = [240.0f32, 224.0, 200.0].map(|v| v / 255.0);
            assert!(px.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.5 / 255.0), "{command} @{depth}: {px:?}");
        }
    }
}
