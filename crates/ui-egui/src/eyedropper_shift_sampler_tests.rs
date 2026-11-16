//! #11748: with the Eyedropper, ⇧-click adds a colour sampler on the clicked pixel (the foreground
//! stays), ⇧⌥-click on a sampler deletes it and off one adds one, ⌥-click without ⇧ still sets
//! the background, and at the 10-sampler limit the reference's alert opens and nothing is added.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::{DialogKind, Tool};

const SHIFT: Modifiers = Modifiers::SHIFT;
const SHIFT_ALT: Modifiers = Modifiers { alt: true, shift: true, ctrl: false, mac_cmd: false, command: false };

/// 800 × 600, white, with a blue (30,30,220) rectangle at 600,350–700,450; the Eyedropper.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 600, "y": 350, "width": 100, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#1e1edc"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Eyedropper;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x, y }, m);
}

fn samplers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().color_samplers.clone()
}

fn foreground(app: &PhotocraftApp) -> [u8; 3] {
    let c = app.session.tools.foreground;
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
fn shift_click_adds_a_sampler_and_keeps_the_foreground() {
    let mut app = app();
    let fg = foreground(&app);
    click(&mut app, 650.0, 400.0, SHIFT);
    assert_eq!(samplers(&app), vec![[650.5, 400.5]], "on the clicked pixel's centre");
    assert_eq!(foreground(&app), fg, "the foreground doesn't change");
}

#[test]
fn shift_alt_click_deletes_a_sampler_and_adds_one_off_a_sampler() {
    let mut app = app();
    click(&mut app, 650.0, 400.0, SHIFT);
    click(&mut app, 650.0, 400.0, SHIFT_ALT);
    assert!(samplers(&app).is_empty(), "⇧⌥ on the sampler deletes it");
    click(&mut app, 200.0, 100.0, SHIFT_ALT);
    assert_eq!(samplers(&app), vec![[200.5, 100.5]], "⇧⌥ away from any sampler adds one");
}

#[test]
fn alt_click_on_a_sampler_still_sets_the_background() {
    let mut app = app();
    click(&mut app, 650.0, 400.0, SHIFT);
    click(&mut app, 650.0, 400.0, Modifiers::ALT);
    assert_eq!(samplers(&app).len(), 1, "the sampler stays");
    let bg = app.session.tools.background;
    assert_eq!([0, 1, 2].map(|i| (bg[i] * 255.0).round() as u8), [30, 30, 220]);
}

#[test]
fn the_eleventh_shift_click_shows_the_alert_and_adds_nothing() {
    let mut app = app();
    for k in 0..10 {
        click(&mut app, 20.0 + 30.0 * f64::from(k), 50.0, SHIFT);
    }
    assert_eq!(samplers(&app).len(), 10);
    click(&mut app, 500.0, 500.0, SHIFT);
    assert_eq!(samplers(&app).len(), 10, "no 11th sampler");
    let alert = app.ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).and_then(|d| d.fields.get("message")).and_then(|m| m.as_str()).map(str::to_owned);
    assert_eq!(alert.as_deref(), Some("Could not create a new sampler because the maximum number of samplers is 10."));
}

#[test]
fn plain_click_and_drag_still_sample_the_foreground() {
    let mut app = app();
    click(&mut app, 650.0, 400.0, Modifiers::NONE);
    assert_eq!(foreground(&app), [30, 30, 220]);
    assert!(samplers(&app).is_empty());
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 650.0, y: 400.0, pressure: 1.0 }, Modifiers::NONE);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 100.0 }, Modifiers::NONE);
    assert_eq!(foreground(&app), [255, 255, 255], "a drag samples where it goes");
}

#[test]
fn a_shift_click_off_the_canvas_adds_nothing() {
    let mut app = app();
    click(&mut app, -20.0, 50.0, SHIFT);
    assert!(samplers(&app).is_empty());
    assert!(app.ui.dialogs.is_empty());
}

/// CodeRabbit on #14693: pressing ⇧ in the middle of a sampling drag doesn't stop the sampling
/// (drag sampling is unchanged), and a drag that started with ⇧ (it placed a sampler) samples
/// nothing, even if ⇧ is let go meanwhile.
#[test]
fn shift_pressed_mid_drag_keeps_sampling_and_a_shift_press_never_samples() {
    let mut app = app();
    let blue = [30, 30, 220];
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 650.0, y: 400.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(foreground(&app), blue);
    // ⇧ goes down and the pointer moves onto the white: it samples the white.
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 200.0, y: 100.0, pressure: 1.0 }, SHIFT);
    assert_eq!(foreground(&app), [255, 255, 255], "the drag keeps sampling with ⇧ held");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 200.0, y: 100.0 }, SHIFT);
    assert!(samplers(&app).is_empty(), "no sampler from a drag that began without ⇧");

    // A press with ⇧ places a sampler; the moves that follow sample nothing, ⇧ held or not.
    let fg = foreground(&app);
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 650.0, y: 400.0, pressure: 1.0 }, SHIFT);
    for m in [SHIFT, Modifiers::NONE] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 660.0, y: 410.0, pressure: 1.0 }, m);
    }
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 660.0, y: 410.0 }, Modifiers::NONE);
    assert_eq!(samplers(&app).len(), 1, "one sampler");
    assert_eq!(foreground(&app), fg, "the drag after a ⇧ press samples nothing");
}

/// CodeRabbit on #14693: a release that another tool receives still ends a ⇧ press, so the next
/// Eyedropper press-less move samples as before.
#[test]
fn a_release_taken_by_another_tool_ends_the_shift_press() {
    let mut app = app();
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 650.0, y: 400.0, pressure: 1.0 }, SHIFT);
    app.ui.tool = Tool::Hand;
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 650.0, y: 400.0 }, Modifiers::NONE);
    app.ui.tool = Tool::Eyedropper;
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 200.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(foreground(&app), [255, 255, 255], "the move samples: the ⇧ press is over");
}
