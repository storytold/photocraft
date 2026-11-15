//! #15122: one press on a layer the Brush can't paint (hidden, Lock All) opens its alert once,
//! through the real canvas: a click, a drag and a long press each open one alert, not one for the
//! press and another when egui recognises the drag or the click.

use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{DialogKind, Tool};

/// A document whose new layer is hidden or has Lock All (`setup`), with the Brush.
fn app(setup: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    match setup {
        "hidden" => app.run("layer.setProps", json!({"layer": id, "visible": false})).unwrap(),
        _ => app.run("layer.lockLayers", json!({"all": true})).unwrap(),
    };
    app.sync_views();
    app.ui.tool = Tool::Brush;
    app
}

/// The whole window, so the alert's modal layer is drawn over the canvas as in the app.
fn window(setup: &str) -> Harness<'static, PhotocraftApp> {
    let app = app(setup);
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(8);
    h
}

/// The canvas alone.
fn harness(setup: &str) -> Harness<'static, PhotocraftApp> {
    let app = app(setup);
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [100.0, 50.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn screen(h: &Harness<'static, PhotocraftApp>, x: f32, y: f32) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform {
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: v.zoom,
        center: v.center,
        flip: app.ui.view.flip_horizontal,
        rotation: v.rotation,
        aspect: app.ui.view.display_aspect(),
    };
    xf.to_screen(x, y)
}

fn button(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32, down: bool) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn move_to(h: &mut Harness<'static, PhotocraftApp>, x: f32, y: f32) {
    let p = screen(h, x, y);
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state()
        .ui
        .dialogs
        .iter()
        .filter(|d| d.kind == DialogKind::Error)
        .filter_map(|d| d.fields.get("message").and_then(Value::as_str).map(str::to_string))
        .collect()
}

fn steps(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

const HIDDEN: &str = "Could not use the brush tool because the target layer is hidden.";
const LOCKED: &str = "Could not use the brush tool because the layer is locked.";

/// Runs `gesture` on a fresh canvas set up as `setup` and checks one alert, nothing painted.
fn once(setup: &str, want: &str, gesture: fn(&mut Harness<'static, PhotocraftApp>)) {
    check(harness(setup), &format!("{setup}, canvas"), want, gesture);
    check(window(setup), &format!("{setup}, window"), want, gesture);
}

fn check(mut h: Harness<'static, PhotocraftApp>, setup: &str, want: &str, gesture: fn(&mut Harness<'static, PhotocraftApp>)) {
    let before = steps(&h);
    move_to(&mut h, 20.0, 50.0);
    gesture(&mut h);
    h.run_steps(3);
    assert_eq!(alerts(&h), [want], "{setup}: the alert, once");
    assert_eq!(steps(&h), before, "{setup}: nothing painted");
}

fn click(h: &mut Harness<'static, PhotocraftApp>) {
    button(h, 20.0, 50.0, true);
    button(h, 20.0, 50.0, false);
}

fn drag(h: &mut Harness<'static, PhotocraftApp>) {
    button(h, 20.0, 50.0, true);
    for x in [22.0, 30.0, 60.0, 100.0] {
        move_to(h, x, 50.0);
    }
    button(h, 100.0, 50.0, false);
}

fn long_press(h: &mut Harness<'static, PhotocraftApp>) {
    button(h, 20.0, 50.0, true);
    h.run_steps(10);
    button(h, 20.0, 50.0, false);
}

#[test]
fn a_click_alerts_once() {
    once("hidden", HIDDEN, click);
    once("locked", LOCKED, click);
}

#[test]
fn a_drag_alerts_once() {
    once("hidden", HIDDEN, drag);
    once("locked", LOCKED, drag);
}

#[test]
fn a_long_press_alerts_once() {
    once("hidden", HIDDEN, long_press);
    once("locked", LOCKED, long_press);
}

#[test]
fn the_next_attempt_alerts_again_and_a_paintable_layer_still_paints() {
    let mut h = window("hidden");
    move_to(&mut h, 20.0, 50.0);
    click(&mut h);
    h.run_steps(3);
    let ok = |h: &mut Harness<'static, PhotocraftApp>| {
        let id = h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::Error).map(|d| d.id).unwrap();
        crate::dialogs::confirm(h.state_mut(), id).unwrap();
        h.run_steps(3);
    };
    ok(&mut h);
    assert!(alerts(&h).is_empty());
    drag(&mut h);
    h.run_steps(3);
    assert_eq!(alerts(&h), [HIDDEN], "a second attempt is a second alert");
    ok(&mut h);
    // Shown again, the same gesture paints one stroke.
    let id = h.state().session.active().unwrap().active_layer.unwrap().0;
    h.state_mut().run("layer.setProps", json!({"layer": id, "visible": true})).unwrap();
    h.run_steps(2);
    let before = steps(&h);
    drag(&mut h);
    h.run_steps(3);
    assert!(alerts(&h).is_empty());
    assert_eq!(steps(&h), before + 1, "one stroke");
}
