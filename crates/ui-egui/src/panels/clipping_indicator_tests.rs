//! The Properties panel's Clip to Layer Below control on an adjustment layer.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerContent;
use serde_json::json;

use crate::PhotocraftApp;

/// Open one real adjustment editor in the Properties panel: the Adjustments panel shows until its
/// pick makes an adjustment layer active, then Properties.
fn adjustment_harness(kind: &str, depth: u32) -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 16, "height": 8, "depth": depth})).unwrap();
    session.execute("edit.fill", json!({"color": "#b04020"})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session.execute("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 8})).unwrap();
    session.execute("edit.fill", json!({"color": "#208040"})).unwrap();
    session.execute("select.deselect", json!({})).unwrap();
    let app = PhotocraftApp::new(session, Default::default());
    let mut properties = false;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(320.0, 640.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            properties |= app
                .session
                .active()
                .and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id)))
                .is_some_and(|l| matches!(l.content, LayerContent::Adjustment(_)));
            crate::i18n::with_language(crate::i18n::Lang::EN, || {
                if properties {
                    super::properties::panel(app, ui);
                } else {
                    super::properties::adjustments::panel(app, ui);
                }
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h.get_by_label(kind).click();
    h.run_steps(4);
    h
}

/// Read a document pixel on the shared scanline used by the clipping-output assertions.
fn sample(h: &mut Harness<'_, PhotocraftApp>, x: i32) -> Vec<f32> {
    serde_json::from_value(h.state_mut().run("document.pixel", json!({"x": x, "y": 4})).unwrap()).unwrap()
}

#[test]
fn panel_adjustment_clips_to_selected_layer_and_scope_undoes_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut h = adjustment_harness("Hue/Saturation", depth);
        let id = h.state().session.active().unwrap().active_layer.unwrap();
        h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "saturation": -100})).unwrap();
        h.run_steps(4);
        let all_layers = sample(&mut h, 2);
        assert!((all_layers[0] - all_layers[1]).abs() < 0.01, "unclipped adjustment desaturates the lower layer");
        let adjusted_target = sample(&mut h, 12);
        let steps = h.state().session.active().unwrap().history.past_len();
        h.get_by_label("Clip to layer below").click();
        h.run_steps(4);
        assert!(h.state().session.active().unwrap().doc.layer(id).unwrap().clipped);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
        let lower_layer = sample(&mut h, 2);
        assert!(lower_layer[0] - lower_layer[1] > 0.2, "clipping restores the lower layer's colour");
        assert!(sample(&mut h, 12).iter().zip(&adjusted_target).all(|(a, b)| (a - b).abs() <= 1.0 / 255.0), "the selected layer stays adjusted");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(sample(&mut h, 2), all_layers);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(sample(&mut h, 2), lower_layer);
        h.run_steps(4);
        h.get_by_label("Clip to layer below").click();
        h.run_steps(4);
        assert!(!h.state().session.active().unwrap().doc.layer(id).unwrap().clipped);
        assert_eq!(sample(&mut h, 2), all_layers, "scope can be released through the same control");
    }
}
