//! #14862: Layer Style opens on Blending Options with Opacity focused and its text selected, as
//! Photoshop does; opening it on an effect page keeps today's behaviour (no field focused).

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

fn harness() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Alpha"})).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap().0;
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    (h, layer)
}

/// Layer > Layer Style > Blending Options…, as the menu runs it.
fn open_blending(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "layer.layerStyle.blendingOptions", Value::Null).unwrap();
}

fn style(h: &Harness<'_, PhotocraftApp>) -> serde_json::Map<String, Value> {
    h.state().ui.dialogs.iter().find(|d| d.kind == DialogKind::LayerStyle).expect("Layer Style open").fields.clone()
}

fn blending(h: &Harness<'_, PhotocraftApp>, key: &str) -> f64 {
    style(h)["p:blendingOptions"][key].as_f64().unwrap()
}

fn focused(h: &Harness<'_, PhotocraftApp>) -> Option<egui::Id> {
    h.ctx.memory(|m| m.focused())
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(2);
}

fn type_text(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

/// Typing 50 then Tab sets Opacity without a click; Tab lands on Fill Opacity, its text selected.
fn typing_sets_opacity_then_fill(h: &mut Harness<'_, PhotocraftApp>) {
    assert!(focused(h).is_some(), "a field has the keyboard focus");
    type_text(h, "50");
    key(h, Key::Tab);
    assert_eq!(blending(h, "opacity"), 50.0, "typing replaced Opacity's 100");
    assert_eq!(blending(h, "fillOpacity"), 100.0);
    type_text(h, "30");
    key(h, Key::Tab);
    assert_eq!(blending(h, "fillOpacity"), 30.0, "Tab went on to Fill Opacity");
    assert_eq!(blending(h, "opacity"), 50.0);
}

#[test]
fn blending_options_opens_with_opacity_focused_and_selected() {
    let (mut h, _) = harness();
    open_blending(&mut h);
    h.run_steps(4);
    typing_sets_opacity_then_fill(&mut h);
}

/// A key typed on the very frame the focus arrives replaces the value rather than appending to it.
#[test]
fn typing_on_the_first_focused_frame_replaces_the_value() {
    // The frame the focus arrives on.
    let (mut h, _) = harness();
    open_blending(&mut h);
    let mut frames = 0;
    while focused(&h).is_none() && frames < 8 {
        h.step();
        frames += 1;
    }
    assert!(focused(&h).is_some(), "the focus arrives");
    // A fresh dialog, the digits queued for that frame.
    let (mut h, _) = harness();
    open_blending(&mut h);
    for _ in 1..frames {
        h.step();
    }
    assert!(focused(&h).is_none());
    h.event(Event::Text("5".into()));
    h.step();
    assert!(focused(&h).is_some());
    key(&mut h, Key::Tab);
    assert_eq!(blending(&h, "opacity"), 5.0, "5 replaced 100, not 1005");
}

/// Opening on an effect page keeps today's behaviour: no field takes the focus, and choosing
/// Blending Options afterwards doesn't either (only an opening on the page does).
#[test]
fn opening_on_an_effect_page_focuses_nothing() {
    let (mut h, _) = harness();
    crate::layer_style::open(h.state_mut(), Some("dropShadow")).unwrap();
    h.run_steps(4);
    assert_ne!(style(&h)["selected"], json!(crate::layer_style::BLENDING));
    assert_eq!(focused(&h), None);
    h.state_mut()
        .ui
        .dialogs
        .iter_mut()
        .find(|d| d.kind == DialogKind::LayerStyle)
        .unwrap()
        .fields
        .insert("selected".into(), json!(crate::layer_style::BLENDING));
    h.run_steps(4);
    assert_eq!(focused(&h), None);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

/// The same after a double-click on a Layers panel row beside the name.
#[test]
fn double_click_on_a_layers_row_focuses_opacity() {
    let (mut h, layer) = harness();
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == layer).expect("row drawn");
    let name = row.name.expect("name drawn");
    let beside = pos2(name.right() + 40.0, row.row.center().y);
    assert!(row.indicators.iter().all(|(_, r)| !r.expand(2.0).contains(beside)) && row.row.contains(beside));
    h.run_steps(40);
    click(&mut h, beside);
    click(&mut h, beside);
    assert_eq!(style(&h)["selected"], json!(crate::layer_style::BLENDING));
    h.run_steps(2);
    typing_sets_opacity_then_fill(&mut h);
}
