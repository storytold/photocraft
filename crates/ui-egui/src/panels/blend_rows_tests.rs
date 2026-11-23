//! The Layers panel's blend and opacity rows (craft-layers' `BlendRows`) as PhotoCraft applies
//! them: what the rows report becomes a canvas preview, one history step or nothing (#970,
//! #1481, #9065). How the rows look and read their input is craft-layers'.

use std::sync::Arc;

use craft::behavior::{CancelReason, EditEvent, GestureId, Mixed};
use craft::layers::ui::panel::LayersEvent;
use craft::layers::ui::rows::LayerPropertyEdit;
use egui::Modifiers;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_color::BlendMode;
use photocraft_doc::{Layer, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn blend(e: EditEvent<String>) -> LayersEvent<LayerId> {
    LayersEvent::Properties(LayerPropertyEdit::Blend(e))
}

fn opacity(e: EditEvent<f64>) -> LayersEvent<LayerId> {
    LayersEvent::Properties(LayerPropertyEdit::Opacity(e))
}

fn key(m: BlendMode) -> String {
    m.key().to_string()
}

fn label() -> Arc<str> {
    Arc::from("Blend Mode")
}

/// The active layer after the events, each in its own pass: (blend, opacity, history steps added,
/// the blend mode the canvas previews).
#[derive(Debug, PartialEq)]
struct After {
    blend: BlendMode,
    opacity: f32,
    steps: usize,
    preview: Option<BlendMode>,
}

/// What the rows report, pass by pass, and what PhotoCraft makes of it.
#[test]
fn blend_rows_edits_become_previews_and_history_steps() {
    let g = GestureId::new(1);
    let normal = || Mixed::Same(key(BlendMode::Normal));
    type Case = (&'static str, Vec<LayersEvent<LayerId>>, After);
    let after = |blend, opacity, steps, preview| After { blend, opacity, steps, preview };
    let cases: Vec<Case> = vec![
        (
            "a mode hovered in the open list previews on the canvas, no step",
            vec![blend(EditEvent::update(g, key(BlendMode::Multiply)))],
            after(BlendMode::Normal, 1.0, 0, Some(BlendMode::Multiply)),
        ),
        (
            "the hovered mode picked: applied in one step, the preview ends",
            vec![blend(EditEvent::update(g, key(BlendMode::Multiply))), blend(EditEvent::commit(g, normal(), key(BlendMode::Multiply)))],
            after(BlendMode::Multiply, 1.0, 1, None),
        ),
        (
            "the list closed without a pick: no step, the preview ends",
            vec![blend(EditEvent::update(g, key(BlendMode::Multiply))), blend(EditEvent::cancel(g, CancelReason::Dismissed, Some(normal())))],
            after(BlendMode::Normal, 1.0, 0, None),
        ),
        (
            "each wheel notch over the closed list is one step",
            vec![
                blend(EditEvent::set(normal(), key(BlendMode::Dissolve), label(), None)),
                blend(EditEvent::set(Mixed::Same(key(BlendMode::Dissolve)), key(BlendMode::Darken), label(), None)),
            ],
            after(BlendMode::Darken, 1.0, 2, None),
        ),
        (
            "an opacity scrub is one step",
            vec![
                opacity(EditEvent::begin(g, Mixed::Same(100.0), Arc::from("Opacity"), None)),
                opacity(EditEvent::update(g, 80.0)),
                opacity(EditEvent::update(g, 55.0)),
                opacity(EditEvent::commit(g, Mixed::Same(100.0), 50.0)),
            ],
            after(BlendMode::Normal, 0.5, 1, None),
        ),
    ];
    for (case, events, want) in cases {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        let top = LayerId(s.execute("layer.new.layer", json!({"name": "top"})).unwrap()["layer"].as_u64().unwrap());
        s.execute("edit.fill", json!({"color": "#cc3366"})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let ctx = app.ctx().clone();
        let steps = app.session.active().unwrap().history.past_len();
        for event in events {
            super::layers_events::apply_layers_events(&mut app, &ctx, vec![event]);
        }
        let l: Layer = app.session.active().unwrap().doc.layer(top).unwrap().clone();
        let preview = super::blend_preview::display_doc(&mut app, 0).map(|(d, _)| d.layer(top).unwrap().blend);
        let got = After { blend: l.blend, opacity: l.opacity, steps: app.session.active().unwrap().history.past_len() - steps, preview };
        assert_eq!(got, want, "{case}");
    }
}

/// A document of `width` × `height` with one more layer, the Move tool active, in the real window.
fn harness(width: u32, height: u32) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": width, "height": height})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app
    });
    h.run_steps(8);
    h
}

fn active_blend(h: &Harness<'_, Box<PhotocraftApp>>) -> BlendMode {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().blend
}

fn list_open(h: &Harness<'_, Box<PhotocraftApp>>) -> bool {
    h.query_all_by_label("Multiply").next().is_some()
}

/// Opens the Layers panel's Blend Mode list with a click.
fn open_list(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    let select = h
        .query_all_by_role(Role::ComboBox)
        .filter(|n| n.accesskit_node().label().as_deref() == Some("Blend Mode"))
        .map(|n| n.rect())
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .expect("the Layers panel's Blend Mode select");
    crate::layers_panel_nodes::click(h, select.center(), Modifiers::NONE);
    assert!(list_open(h), "the list is open");
}

/// With the Blend Mode list open, a letter is the list's and does not switch tools (#1481).
#[test]
#[ignore = "craft: a letter typed into an open Select list still reaches the app's egui input (consumed-chord class)"]
fn letters_in_the_open_blend_list_do_not_switch_tools() {
    let mut h = harness(64, 64);
    open_list(&mut h);
    h.key_press(egui::Key::M);
    h.event(egui::Event::Text("m".into()));
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::Move, "M doesn't pick the Marquee tool");
}

/// With the Blend Mode list open, a command shortcut closes the list and runs (#9065: ⌘1, 100%, as
/// Photoshop's menus do).
#[test]
fn a_command_shortcut_closes_the_open_blend_list_and_runs() {
    let mut h = harness(3000, 2000);
    assert_ne!(h.state().ui.views[0].zoom, 1.0, "the document starts fitted, not at 100%");
    open_list(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Num1);
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].zoom, 1.0, "⌘1 (100%) runs with the list open");
    assert!(!list_open(&h), "and closes the list");
    assert_eq!(active_blend(&h), BlendMode::Normal, "the mode is unchanged");
}
