//! #10888: the Crop options bar's Fill help. The help is a hosted alert; Escape closes it and must
//! not reach the pending crop underneath.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::crop_ui;
use serde_json::json;

type App = Harness<'static, Box<PhotocraftApp>>;

/// The label of the Fill help button in the bar.
const HELP_BUTTON: &str = "Displays pop-up help for the fill methods";

/// The app with the Crop tool in Classic Mode (a drag draws a frame) and English labels.
fn app() -> App {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 320.0)).with_max_steps(64).with_accessibility_check(false).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"name": "crop bar", "width": 800, "height": 600})).expect("new document");
        app.ui.tool_options.crop.shield.classic_mode = true;
        app
    });
    h.run_steps(4);
    request(&mut h, "ui.set", json!({"tool": "crop"}));
    h
}

/// Runs one control-channel request, which must succeed, then lets the app draw.
fn request(h: &mut App, method: &str, params: serde_json::Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => assert_eq!(v["ok"], true, "{method} {params}: {v}"),
        // Input the canvas takes on its next pass: the frames below deliver it.
        Reply::AfterInput => {}
        _ => panic!("{method} {params}: not done"),
    }
    h.run_steps(6);
}

/// Every text shape the harness drew this frame, tooltips and modals included.
fn drawn<State>(h: &Harness<'_, State>) -> Vec<String> {
    fn collect(shape: &egui::Shape, text: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(shape) => text.push(shape.galley.job.text.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, text)),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

fn help_open(h: &App) -> bool {
    drawn(h).iter().any(|t| t == "Fill options")
}

#[test]
fn the_fill_help_alert_takes_escape_and_the_crop_stays_pending() {
    let mut h = app();
    let events = json!([
        {"kind": "down", "x": 100.0, "y": 80.0},
        {"kind": "move", "x": 400.0, "y": 300.0},
        {"kind": "up", "x": 400.0, "y": 300.0},
    ]);
    request(&mut h, "ui.pointer", json!({"events": events}));
    assert!(crop_ui::pending(h.state()), "the drawn frame is pending");
    let before = crop_ui::frame(h.state()).expect("a frame");

    h.get_by_label(HELP_BUTTON).click();
    h.run_steps(2);
    let text = drawn(&h);
    assert!(help_open(&h), "the alert opened: {text:?}");
    assert!(text.iter().any(|t| t.contains("Background (default):")));
    assert!(text.iter().any(|t| t.contains("Content-aware fill")));

    // Escape closes the alert only.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!help_open(&h), "Escape closes the alert");
    assert!(crop_ui::pending(h.state()), "Escape left the crop pending");
    assert_eq!(crop_ui::frame(h.state()), Some(before), "Escape left the frame alone");

    // A quick user: Escape the first frame the alert shows, right after the click that opened it, must not cancel
    // the crop underneath either.
    h.get_by_label(HELP_BUTTON).click();
    let mut frames = 0;
    while !help_open(&h) && frames < 8 {
        h.step();
        frames += 1;
    }
    assert!(help_open(&h), "the click opened the alert again");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(!help_open(&h), "Escape in the first frame of the alert closes it");
    assert!(crop_ui::pending(h.state()), "Escape in the first frame of the alert leaves the crop pending");
    assert_eq!(crop_ui::frame(h.state()), Some(before), "the frame survives");
}
