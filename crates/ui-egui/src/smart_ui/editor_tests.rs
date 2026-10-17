use egui::{Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{
    PhotocraftApp,
    smart_filter_dialog::tests::{app, smart},
};

fn harness(width: f32, scale: f32, theme: crate::theme::ThemeKind) -> Harness<'static, PhotocraftApp> {
    let (app, _) = app(8, "rgb");
    let mut h =
        Harness::builder().with_size(vec2(1200.0, 850.0)).with_pixels_per_point(scale).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            app
        });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":theme.id(),"dockWidth":width,"dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(5);
    h
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn names_and_blend_icons_have_separate_double_click_targets_and_no_overlap() {
    for theme in crate::theme::ThemeKind::ALL {
        for width in [250.0, 420.0] {
            let mut h = harness(width, 1.0, theme);
            let layer = h.state().session.active().unwrap().active_layer.unwrap();
            let rows = crate::layer_row_ui::recorded(&h.ctx);
            let row = rows.iter().find(|r| r.layer == layer.0).unwrap();
            assert!(row.indicators.iter().any(|(k, _)| *k == crate::layer_row_ui::Indicator::SmartFilters));
            let blend = h.get_by_label("Blending Options (Box Blur)").rect();
            let edit = h.get_by_label("Box Blur").rect();
            assert!(edit.right() <= blend.left(), "{width} {theme:?}: {edit:?} {blend:?}");
            click(&mut h, blend.center());
            assert!(h.state().ui.dialogs.is_empty(), "single click does not open blending");
            click(&mut h, blend.center());
            assert!(crate::smart_filter_dialog::owns(&h.state().ui.dialogs.last().unwrap().fields));
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__smartFilter"]["index"], json!(0));
            let id = h.state().ui.dialogs.last().unwrap().id;
            crate::dialogs::cancel(h.state_mut(), id).unwrap();
            // Start a new gesture after egui's triple-click interval, as native users do.
            for _ in 0..40 {
                h.step();
            }
            click(&mut h, edit.center());
            click(&mut h, edit.center());
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["__command"], json!("filter.blur.boxBlur"));
            assert_eq!(h.state().ui.dialogs.last().unwrap().fields["radius"].as_f64(), Some(4.0));
        }
    }
}

#[test]
fn disclosure_hides_both_stacks_without_changing_pixels_history_or_selection() {
    let mut h = harness(290.0, 1.5, Default::default());
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().run("layer.layerStyle.dropShadow", json!({"layer":layer.0})).unwrap();
    h.state_mut().run("layer.new.layer", json!({"name":"Selected elsewhere"})).unwrap();
    h.run_steps(5);
    let before = h.state().session.active().unwrap().doc.clone();
    let revision = h.state().session.active().unwrap().revision;
    let steps = h.state().session.active().unwrap().history.past_len();
    let active = h.state().session.active().unwrap().active_layer;
    let at = h.get_by_label("Collapse effects Smart Object").rect().center();
    click(&mut h, at);
    assert!(h.state().session.active().unwrap().fx_collapsed.contains(&layer));
    assert!(h.query_by_label("Blending Options (Box Blur)").is_none());
    assert!(h.query_by_label("Drop Shadow").is_none());
    assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
    assert_eq!(h.state().session.active().unwrap().active_layer, active);
    let at = h.get_by_label("Expand effects Smart Object").rect().center();
    click(&mut h, at);
    assert!(h.query_by_label("Blending Options (Box Blur)").is_some());
}

#[test]
fn adjustment_editor_reopens_stored_parameters_and_updates_instead_of_appending() {
    let (mut app, layer) = app(8, "rgb");
    app.run("image.adjustments.hueSaturation", json!({"hue":35,"saturation":12,"lightness":-7})).unwrap();
    let ctx = egui::Context::default();
    let sm = smart(&app, layer).clone();
    super::open_editor(&mut app, &ctx, layer.0, &sm, 2);
    let id = app.ui.dialogs.last().unwrap().id;
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(fields["hue"], json!(35));
    assert!(crate::adjust_dialog::owns(fields));
    fields.insert("hue".into(), json!(75));
    let idx = app.session.active_index().unwrap();
    assert!(!crate::adjust_preview::on_layer(&mut app, idx));
    assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).is_some());
    let steps = app.session.active().unwrap().history.past_len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(smart(&app, layer).smart_filters.len(), 3);
    assert_eq!(smart(&app, layer).smart_filters[2].params["hue"], json!(75));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(&app, layer).smart_filters[2].params["hue"], json!(35));
    app.run("image.adjustments.invert", json!({})).unwrap();
    let sm = smart(&app, layer).clone();
    super::open_editor(&mut app, &ctx, layer.0, &sm, 3);
    assert!(app.ui.dialogs.is_empty(), "Invert has no parameter editor");
}

#[test]
fn smart_and_layer_masks_have_separate_thumbnail_cache_entries() {
    let (mut app, layer) = app(8, "rgb");
    app.run("layer.layerMask.revealAll", json!({"layer":layer.0})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let mask = doc.layer(layer).unwrap().mask.as_ref().unwrap();
    let ctx = egui::Context::default();
    let first = app.mask_thumb(&ctx, &doc, layer, mask);
    let mut other = mask.clone();
    other.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 64, 48), &[0.0]);
    let second = app.mask_thumb_kind(&ctx, &doc, layer, &other, crate::mask_thumbs_ui::THUMB_SMART_MASK);
    assert_ne!(first, second);
    assert_eq!(first, app.mask_thumb(&ctx, &doc, layer, mask));
}

#[test]
fn ctrl_i_uses_existing_invert_command_and_undo_redo_keep_the_source() {
    let mut h = harness(290.0, 1.0, Default::default());
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    let before = smart(h.state(), layer).clone();
    let source = photocraft_engine::smart_cmds::contents_of(&h.state().session.active().unwrap().doc, layer).unwrap();
    let steps = h.state().session.active().unwrap().history.past_len();
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::I);
    h.run_steps(4);
    let sm = smart(h.state(), layer);
    assert_eq!(sm.smart_filters.len(), 3);
    assert_eq!(sm.smart_filters[2].command, "image.adjustments.invert");
    assert_eq!(photocraft_engine::smart_cmds::contents_of(&h.state().session.active().unwrap().doc, layer).unwrap(), source);
    assert_ne!(sm.cache.as_ref().unwrap().pixel(8, 8), before.cache.as_ref().unwrap().pixel(8, 8));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(smart(h.state(), layer).smart_filters.len(), 2);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(smart(h.state(), layer).smart_filters.len(), 3);
}
