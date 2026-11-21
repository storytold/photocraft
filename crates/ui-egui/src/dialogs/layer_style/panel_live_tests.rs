//! #14861: while the Layer Style dialog is open with Preview on, the Layers panel's Opacity, Fill and blending badge
//! follow the dialog; with Preview off, and after Cancel, the panel shows the values at opening, and nothing reaches
//! the history before OK.

use craft::imaging::layer_style::{BlendIf, BlendIfRange};
use craft::layers::ui::filter::LayerFilter;
use craft::layers::ui::tree::LayerTree;

use super::tests::{app_with_layer, cancel, edit};
use super::{DialogId, open};
use crate::PhotocraftApp;
use crate::panels::layers_tree::PhotoLayers;

/// The Layers panel's model of the active document, after a canvas pass built the dialog's preview.
fn panel(app: &mut PhotocraftApp) -> PhotoLayers {
    let idx = app.session.active_index().unwrap();
    let _ = super::display_doc(app, idx);
    let st = app.session.active().unwrap();
    PhotoLayers::new(app, st, &LayerFilter::default(), false, None)
}

/// The panel's (Opacity, Fill) of the active layer, in whole percent as the fields show them.
fn opacity_fill(app: &mut PhotocraftApp) -> (f64, f64) {
    let p = panel(app).properties().unwrap();
    (p.opacity.round(), p.fill.round())
}

fn badge(app: &mut PhotocraftApp) -> bool {
    let id = app.session.active().unwrap().active_layer.unwrap();
    panel(app).info(&id).advanced_blending
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn dialog(app: &mut PhotocraftApp) -> DialogId {
    open(app, None).unwrap()
}

#[test]
fn opacity_and_fill_follow_the_dialog_while_preview_is_on() {
    let mut app = app_with_layer(32);
    let before = steps(&app);
    let id = dialog(&mut app);
    assert_eq!(opacity_fill(&mut app), (100.0, 100.0));
    edit(&mut app, id, |d| {
        d.style.blending.fill_opacity = 30.0;
        d.style.blending.opacity = 60.0;
    });
    assert_eq!(opacity_fill(&mut app), (60.0, 30.0), "the panel shows the dialog's values before OK");
    assert_eq!(steps(&app), before, "nothing in the history yet");
    cancel(&mut app, id);
    assert_eq!(opacity_fill(&mut app), (100.0, 100.0), "Cancel puts the panel back");
    assert_eq!(steps(&app), before);
}

#[test]
fn the_blending_badge_shows_while_the_dialog_is_open() {
    let mut app = app_with_layer(32);
    let id = dialog(&mut app);
    assert!(!badge(&mut app));
    edit(&mut app, id, |d| {
        d.style.blending.blend_if = vec![BlendIf { channel: 0, this_layer: BlendIfRange([40, 40, 255, 255]), underlying: BlendIfRange([0, 0, 255, 255]) }];
    });
    assert!(badge(&mut app), "a changed Blend If shows the badge before OK");
    cancel(&mut app, id);
    assert!(!badge(&mut app));
}

#[test]
fn with_preview_off_the_panel_keeps_the_values_at_opening() {
    let mut app = app_with_layer(32);
    let id = dialog(&mut app);
    edit(&mut app, id, |d| {
        d.preview = false;
        d.style.blending.fill_opacity = 30.0;
    });
    assert_eq!(opacity_fill(&mut app), (100.0, 100.0));
}
