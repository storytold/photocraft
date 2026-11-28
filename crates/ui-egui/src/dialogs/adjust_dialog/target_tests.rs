//! #10091: the Levels and Curves eyedroppers' target colours in the modal dialogs.

use super::*;
use crate::color_picker_ui::{self, PickTarget};
use egui::{Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn app_in(depth: u32, fill: [f32; 3]) -> Box<PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
    s.execute("edit.fill", json!({"color": fill})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

fn dialog_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .with_step_dt(1.0 / 60.0)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(2);
    h
}

/// Two quick clicks at `at`, one pass per press and release, as the window delivers them.
fn double_click(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
}

/// Double-clicking a Set Black / Gray / White Point eyedropper in the Curves and Levels dialogs opens the Color
/// Picker over the dialog on that point's target colour, and leaves the eyedropper armed. A single click only arms it.
#[test]
fn double_clicking_an_eyedropper_opens_its_target_color_picker() {
    let cases = [
        ("Set Black Point", TonePicker::Black, "#000000"),
        ("Set Neutral Gray Point", TonePicker::Gray, "#808080"),
        ("Set White Point", TonePicker::White, "#ffffff"),
    ];
    for command in ["image.adjustments.curves", "image.adjustments.levels"] {
        for (label, picker, color) in cases {
            let mut h = dialog_harness(app_in(8, [0.5; 3]));
            let id = open(h.state_mut(), command).unwrap();
            h.run_steps(3);
            let at = h.get_by_label(label).rect().center();
            double_click(&mut h, at);
            let app = h.state();
            let top = color_picker_ui::top(app).unwrap_or_else(|| panic!("{command} {label}: the picker opens"));
            assert!(matches!(color_picker_ui::target(app, top), Some(PickTarget::ToneTarget(p)) if *p == picker), "{command} {label}");
            let shown = color_picker_ui::testing::current(app, top).map(|c| color_picker_ui::rgb_hex(c.color));
            assert_eq!(shown.as_deref(), Some(color), "{command} {label}: opens on the target");
            assert_eq!(hosted(app, id).and_then(HostedAdjustDialog::picker), Some(picker), "{command} {label}: armed");
        }
    }
    let mut h = dialog_harness(app_in(8, [0.5; 3]));
    let id = open(h.state_mut(), "image.adjustments.curves").unwrap();
    h.run_steps(3);
    h.get_by_label("Set White Point").click();
    h.run_steps(3);
    assert!(color_picker_ui::top(h.state()).is_none(), "no picker on a single click");
    assert_eq!(hosted(h.state(), id).and_then(HostedAdjustDialog::picker), Some(TonePicker::White));
}

/// OK in the target picker stores the colour in the `targetColors` preference; Cancel leaves it.
#[test]
fn target_picker_ok_stores_the_target_and_cancel_keeps_it() {
    let mut app = app_in(8, [0.5; 3]);
    let id = crate::layer_picker::open_target_picker(&mut app, TonePicker::Black, None).unwrap();
    color_picker_ui::testing::set_color(&mut app, id, "#203040");
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().target_colors.shadows, "#203040");
    assert_eq!(crate::layer_picker::target_rgb(&app, TonePicker::Black), Some([32.0 / 255.0, 48.0 / 255.0, 64.0 / 255.0]));

    let id = crate::layer_picker::open_target_picker(&mut app, TonePicker::Black, None).unwrap();
    let shown = color_picker_ui::testing::current(&app, id).map(|c| color_picker_ui::rgb_hex(c.color));
    assert_eq!(shown.as_deref(), Some("#203040"), "opens on the stored target");
    color_picker_ui::testing::set_color(&mut app, id, "#ff0000");
    crate::dialogs::testing::cancel(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().target_colors.shadows, "#203040", "Cancel keeps the target");
    assert_eq!(app.session.prefs().target_colors.highlights, "#ffffff", "other targets untouched");
    assert!(crate::layer_picker::confirm_target(&mut app, TonePicker::White, "not a colour").is_err(), "a bad colour is an error, not a panic");
    assert_eq!(app.session.prefs().target_colors.highlights, "#ffffff");
}

/// The dialog's eyedroppers map the sampled pixel to the stored target at 8, 16 and 32 bits.
#[test]
fn dialog_eyedroppers_map_the_sample_to_the_stored_target_at_every_depth() {
    for depth in [8, 16, 32] {
        for command in ["image.adjustments.curves", "image.adjustments.levels"] {
            let mut app = app_in(depth, [0.6, 0.5, 0.4]);
            app.run("prefs.set", json!({"path": "targetColors.highlights", "value": "#f0e0c8"})).unwrap();
            let id = open(&mut app, command).unwrap();
            hosted_mut(&mut app, id).set_picker(Some(TonePicker::White));
            assert!(pick(&mut app, [8.0, 8.0]));
            crate::dialogs::testing::confirm(&mut app, id).unwrap();
            let px: Vec<f32> = serde_json::from_value(app.run("document.pixel", json!({"x": 8, "y": 8})).unwrap()).unwrap();
            let want = [240.0f32, 224.0, 200.0].map(|v| v / 255.0);
            assert!(px.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.5 / 255.0), "{command} @{depth}: {px:?}");
        }
    }
}
