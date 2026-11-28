//! #16783: a second click on the targeted vector mask thumbnail un-targets it, while the layer mask
//! stays targeted. Each click is a `LayersEvent::Select` sent through `apply_layers_events`, as the
//! panel reports it.

use craft::behavior::SelectionChange;
use craft::layers::ui::panel::{ActivateOn, LayerSelect, LayersEvent};
use craft::layers::ui::tree::{LayerPart, LayerRowKey};
use photocraft_doc::LayerId;
use serde_json::json;

use super::apply_layers_events;
use super::tests::session;
use crate::PhotocraftApp;

fn app_of(s: photocraft_engine::Session) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.background_jobs = false;
    app.sync_views();
    app
}

fn click(app: &mut PhotocraftApp, layer: u64, part: LayerPart) {
    let ctx = app.ctx().clone();
    let event = LayersEvent::Select(LayerSelect::new(SelectionChange::Set(vec![LayerId(layer)]), Some(part)));
    apply_layers_events(app, &ctx, vec![event]);
}

/// A double click as the panel reports it: the first click selects, the second only activates.
fn double_click(app: &mut PhotocraftApp, layer: u64, part: LayerPart) {
    click(app, layer, part);
    let ctx = app.ctx().clone();
    apply_layers_events(app, &ctx, vec![LayersEvent::activate(LayerRowKey::Layer(LayerId(layer)), ActivateOn::Thumbnail(part))]);
}

fn vector_mask_targeted(app: &PhotocraftApp) -> bool {
    app.ui.vector_mask_target && !app.ui.mask_target && app.ui.selected_path.as_deref() == Some("layer")
}

#[test]
fn a_second_click_on_the_targeted_vector_mask_untargets_it() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    click(&mut app, masked, LayerPart::VectorMask);
    assert!(vector_mask_targeted(&app));
    click(&mut app, masked, LayerPart::VectorMask);
    assert!(!app.ui.vector_mask_target && !app.ui.mask_target, "the layer's pixels are the target");
    assert_eq!(app.ui.selected_path, None, "Paths no longer selects the mask's path");
    assert_eq!(crate::canvas::paint_target(&app), json!("pixels"));
    assert_eq!(app.session.active().unwrap().active_layer, Some(LayerId(masked)), "the layer stays active");
    click(&mut app, masked, LayerPart::VectorMask);
    assert!(vector_mask_targeted(&app), "a third click targets it again");
}

#[test]
fn a_double_click_toggles_the_vector_mask_target_with_its_first_click() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    double_click(&mut app, masked, LayerPart::VectorMask);
    assert!(vector_mask_targeted(&app), "on an untargeted vector mask, the double click targets it");
    double_click(&mut app, masked, LayerPart::VectorMask);
    assert!(!app.ui.vector_mask_target && !app.ui.mask_target, "on the targeted one, its first click un-targets it");
    assert_eq!(app.ui.selected_path, None);
}

#[test]
fn a_second_click_on_the_targeted_layer_mask_keeps_it_targeted() {
    let (s, masked, _) = session();
    let mut app = app_of(s);
    click(&mut app, masked, LayerPart::LayerMask);
    click(&mut app, masked, LayerPart::LayerMask);
    assert!(app.ui.mask_target && !app.ui.vector_mask_target);
    assert_eq!(crate::canvas::paint_target(&app), json!("mask"));
}

#[test]
fn the_vector_mask_of_another_layer_is_targeted_while_one_is_targeted() {
    let (mut s, masked, _) = session();
    let other = s.execute("layer.new.layer", json!({"name": "Other"})).unwrap()["layer"].as_u64().unwrap();
    let path = json!({"subpaths": [{"knots": [[10, 10], [60, 10], [60, 60]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": other, "path": path})).unwrap();
    s.execute("layer.select", json!({"layer": masked})).unwrap();
    let mut app = app_of(s);
    click(&mut app, masked, LayerPart::VectorMask);
    click(&mut app, other, LayerPart::VectorMask);
    assert_eq!(app.session.active().unwrap().active_layer, Some(LayerId(other)));
    assert!(vector_mask_targeted(&app), "the first click on another layer's vector mask targets it");
}
