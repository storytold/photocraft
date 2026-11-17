//! The Layers panel's rows through the real panel: what a key on a row runs in PhotoCraft, and what an eye sweep
//! becomes in the history. How the rows look and hit-test is craft-layers'.

use std::sync::Arc;

use craft::behavior::{EditEvent, GestureId, Mixed};
use craft::layers::ui::panel::{LayerColumn, LayerToggle, LayersEvent};
use craft::layers::ui::tree::LayerRowKey;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::layers_panel_nodes::{click, row};

type H = Harness<'static, PhotocraftApp>;

fn harness(s: Session) -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h
}

fn st(h: &H) -> &photocraft_engine::DocState {
    h.state().session.active().unwrap()
}

fn names(h: &H) -> Vec<String> {
    st(h).doc.layers.iter().map(|l| l.name.clone()).collect()
}

fn steps(h: &H) -> usize {
    st(h).history.past_len()
}

/// "L0" … "L2" over the Background, "L2" active.
fn three() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for i in 0..3 {
        s.execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
    }
    s
}

/// One sweep down the eye column (the panel's events of one gesture) hides every layer swept over in one history step.
#[test]
fn sweeping_the_eyes_is_one_step() {
    let mut app = PhotocraftApp::new(three(), crate::Services::default());
    let ctx = app.ctx().clone();
    let doc = app.session.active().unwrap().doc.clone();
    let rows: Vec<LayerRowKey<_>> = ["L2", "L1", "L0"].iter().map(|n| LayerRowKey::Layer(doc.layers.iter().find(|l| l.name == *n).unwrap().id)).collect();
    let before = app.session.active().unwrap().history.past_len();
    let g = GestureId::new(1);
    let toggle = |rows: Vec<LayerRowKey<_>>, edit| LayersEvent::Toggle(LayerToggle::new(rows, LayerColumn::Visibility, edit));
    let began = EditEvent::begin(g, Mixed::Same(true), Arc::from("Layer Visibility"), None);
    super::panels::layers_events::apply_layers_events(&mut app, &ctx, vec![toggle(rows[..1].to_vec(), began)]);
    super::panels::layers_events::apply_layers_events(&mut app, &ctx, vec![toggle(rows[..2].to_vec(), EditEvent::update(g, false))]);
    super::panels::layers_events::apply_layers_events(&mut app, &ctx, vec![toggle(rows.clone(), EditEvent::update(g, false))]);
    super::panels::layers_events::apply_layers_events(&mut app, &ctx, vec![toggle(rows, EditEvent::commit(g, Mixed::Same(true), false))]);
    let st = app.session.active().unwrap();
    for name in ["L0", "L1", "L2"] {
        assert!(!st.doc.layers.iter().find(|l| l.name == name).unwrap().visible, "{name} swept hidden");
    }
    assert!(st.doc.layers.iter().find(|l| l.name == "Background").unwrap().visible, "rows past the sweep are untouched");
    assert_eq!(st.history.past_len(), before + 1, "one history step");
    assert_eq!(st.history.undo_label(), Some("Layer Visibility"));
}

/// Cmd+] with the rows focused moves the active layer up once per press: the list's stacking key
/// and PhotoCraft's Layer › Arrange › Bring Forward share the chord, and one press runs one of them.
#[test]
fn command_bracket_moves_the_layer_once() {
    let mut h = harness(three());
    let at = row(&h, "L0").center();
    click(&mut h, at, Modifiers::NONE);
    let n = steps(&h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::CloseBracket);
    h.run_steps(3);
    assert_eq!(names(&h), ["Background", "L1", "L0", "L2"], "one place up");
    assert_eq!(steps(&h), n + 1, "one history step");
}
