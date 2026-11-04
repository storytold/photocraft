use egui::{Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{
    PhotocraftApp,
    smart_filter_dialog::tests::{app, smart},
};

/// The app with a recorded Smart Object, in a `width`-point dock and the given theme, ready to click.
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

/// Click `at` with the primary button, hovering first so the row reacts.
fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// A filter row's name and blend icon are separate double-click targets, side by side without overlap, in every theme and dock width.
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

/// The effects disclosure hides the effects and Smart Filters stacks together, as a view change only.
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

/// An adjustment on a Smart Object reopens its editor with the stored values and OK updates it in one step; Invert has no editor.
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

/// The Smart Filters mask and the layer mask of one layer are cached as separate thumbnails.
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

/// ⌘I on a Smart Object adds the ordinary Invert as a smart filter, and undo/redo leave the source contents alone.
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

/// A recorded filter that lacks a parameter reopens with the schema default for it, not with the
/// value the user last used in that filter's dialog; the parameters it does hold still win.
#[test]
fn filter_editor_starts_missing_parameters_from_defaults_not_remembered_values() {
    let (mut app, layer) = app(8, "rgb");
    let command = "filter.sharpen.smartSharpen";
    app.session.prefs.edit(|p| {
        p.dialogs.insert(command.into(), json!({"reduceNoise": 60, "radius": 9}));
    });
    let ctx = egui::Context::default();
    let mut sm = smart(&app, layer).clone();
    let index = sm.smart_filters.iter().position(|f| f.command == command).unwrap();
    sm.smart_filters[index].params = json!({"amount": 125, "radius": 2});

    super::open_editor(&mut app, &ctx, layer.0, &sm, index);
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert!(fields.contains_key("__smartFilter"), "it edits the recorded filter");
    assert_eq!(fields["reduceNoise"].as_f64(), Some(10.0), "the spec default, not the remembered 60");
    assert_eq!(fields["amount"].as_f64(), Some(125.0));
    assert_eq!(fields["radius"].as_f64(), Some(2.0), "recorded values beat the remembered 9");

    // A plain Filter menu dialog still starts from the remembered values.
    let id = crate::filter_dialog::open(&mut app, command).unwrap();
    assert_eq!(app.ui.dialog_mut(id).unwrap().fields["reduceNoise"].as_f64(), Some(60.0));
}

/// A captured tone filter must visibly affect its CPU preview, not merely retain parameters.
fn assert_tone_preview_is_non_neutral(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let idx = app.session.active_index().unwrap();
    let id = app.ui.dialogs.last().unwrap().id;
    let saved = app.ui.dialogs.last().unwrap().fields.clone();
    let kind = saved["__adjust"].as_str().unwrap();
    let mode = app.session.active().unwrap().doc.mode;
    crate::canvas::ensure_filter_preview(app, idx, ctx).unwrap();
    let rendered = photocraft_compose::flatten(app.filter_preview.as_ref().unwrap().result.as_ref().unwrap()).px.clone();
    let neutral = photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind, mode).unwrap());
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    fields.retain(|key, _| key.starts_with("__"));
    fields.extend(neutral.as_object().unwrap().clone());
    crate::canvas::ensure_filter_preview(app, idx, ctx).unwrap();
    let identity = photocraft_compose::flatten(app.filter_preview.as_ref().unwrap().result.as_ref().unwrap()).px.clone();
    assert_ne!(rendered, identity, "{mode:?} {kind}: the recorded filter must affect the preview");
    app.ui.dialog_mut(id).unwrap().fields = saved;
    crate::canvas::ensure_filter_preview(app, idx, ctx).unwrap();
}

/// Re-editing a whole-image tone Smart Filter keeps its colour controls and stored records,
/// even when a colour component, alpha, Quick Mask or either layer-mask UI target is selected.
#[test]
fn tone_filter_editor_ignores_current_channel_target() {
    use photocraft_engine::{adjust_params, channel_cmds::ChannelTarget};
    for mode in ["rgb", "lab", "cmyk"] {
        for kind in ["levels", "curves"] {
            for view in 0..6 {
                let (mut app, layer) = app(8, mode);
                let command = format!("image.adjustments.{kind}");
                let color_mode = app.session.active().unwrap().doc.mode;
                let mut recorded = adjust_params::to_params(&adjust_params::default_for(kind, color_mode).unwrap());
                let key = if mode == "lab" {
                    "lightness"
                } else if kind == "curves" {
                    "points"
                } else {
                    ""
                };
                if kind == "levels" {
                    if key.is_empty() {
                        recorded["gamma"] = json!(1.4);
                    } else {
                        recorded[key]["gamma"] = json!(1.4);
                    }
                } else {
                    recorded[key] = json!([[0, 0], [128, 180], [255, 255]]);
                }
                app.run(&command, recorded.clone()).unwrap();
                app.run("layer.layerMask.revealAll", json!({})).unwrap();
                app.run("channel.new", json!({"name":"Saved sky", "fill":"white"})).unwrap();
                if view == 4 {
                    app.run("select.editInQuickMaskMode", json!({})).unwrap();
                }
                app.session.active_mut().unwrap().channel_view.target = match view {
                    0 => ChannelTarget::Alpha(0),
                    1 => ChannelTarget::LayerMask,
                    2 => ChannelTarget::Color(1),
                    _ => ChannelTarget::Composite,
                };
                app.ui.mask_target = view == 3;
                app.ui.filter_mask_target = (view == 5).then_some(layer.0);
                let before = app.session.active().unwrap().doc.clone();
                let filters = smart(&app, layer).clone();
                let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
                PhotocraftApp::setup_context(&h.ctx, Default::default());
                let ctx = h.ctx.clone();
                super::open_editor(h.state_mut(), &ctx, layer.0, &filters, 2);
                let id = h.state().ui.dialogs.last().unwrap().id;
                let values = crate::filter_dialog::params_of(&h.state().ui.dialogs.last().unwrap().fields);
                assert_eq!(values.get("target"), None, "{mode} {kind} view={view}: no channel target is added to a Smart Filter");
                assert_eq!(values, recorded, "{mode} {kind} view={view}: recorded colour parameters survive reopening");
                h.run_steps(3);
                let names: Vec<_> = h.query_all_by_role(egui::accesskit::Role::ComboBox).filter_map(|n| n.value()).collect();
                let expected = match mode {
                    "lab" => "Lightness",
                    "cmyk" => "CMYK",
                    _ => "RGB",
                };
                assert!(names.iter().any(|n| n == expected), "{mode} {kind} view={view}: {names:?}");
                assert!(
                    !names.iter().any(|n| n == "Saved sky" || n == "Quick Mask" || n == "Smart Object Mask" || n == "Green" || n == "Magenta" || n == "a"),
                    "{names:?}"
                );
                let ctx = h.ctx.clone();
                let idx = h.state().session.active_index().unwrap();
                assert!(crate::canvas::ensure_filter_preview(h.state_mut(), idx, &ctx).is_some());
                assert_tone_preview_is_non_neutral(h.state_mut(), &ctx);
                assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &before), "preview never writes to the selected plane");
                let shown = photocraft_compose::flatten(h.state().filter_preview.as_ref().unwrap().result.as_ref().unwrap()).px.clone();
                crate::dialogs::confirm(h.state_mut(), id).unwrap();
                assert_eq!(
                    shown,
                    photocraft_compose::flatten(&h.state().session.active().unwrap().doc).px,
                    "{mode} {kind} view={view}: preview matches commit"
                );
                assert_eq!(smart(h.state(), layer).smart_filters.len(), 3);
                assert_eq!(smart(h.state(), layer).smart_filters[2].params, recorded);
                let after = &h.state().session.active().unwrap().doc;
                assert_eq!(after.channels, before.channels);
                assert_eq!(after.quick_mask, before.quick_mask);
                assert_eq!(after.layer(layer).unwrap().mask, before.layer(layer).unwrap().mask);
            }
        }
    }
}

/// Whole-image Curves stored through the gray alias reopen without changing their rendered effect.
#[test]
fn rabbit_whole_image_curves_alias_survives_reopening() {
    for mode in ["rgb", "lab", "cmyk"] {
        for depth in [8, 16, 32] {
            for recorded in [
                json!({"gray":[[0,255],[255,0]]}),
                json!({"gray":(0..256).map(|i|255-i).collect::<Vec<_>>(),"pencil":true,"smooth":2}),
                json!({"gray":"ignored","points":[[0,255],[255,0]]}),
            ] {
                let (mut app, layer) = app(depth, mode);
                app.run("image.adjustments.curves", recorded.clone()).unwrap();
                let before = app.session.active().unwrap().doc.clone();
                let original = photocraft_compose::flatten(&before).px.clone();
                let filters = smart(&app, layer).clone();
                let ctx = egui::Context::default();
                super::open_editor(&mut app, &ctx, layer.0, &filters, 2);
                let id = app.ui.dialogs.last().unwrap().id;
                let fields = &app.ui.dialogs.last().unwrap().fields;
                assert!(fields.get("gray").is_none(), "the alias must be consumed before defaults");
                let idx = app.session.active_index().unwrap();
                crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).unwrap();
                let shown = photocraft_compose::flatten(app.filter_preview.as_ref().unwrap().result.as_ref().unwrap()).px.clone();
                assert_eq!(shown, original, "{mode} {depth}: reopening preserves the stored effect");
                assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
                crate::dialogs::confirm(&mut app, id).unwrap();
                assert_eq!(shown, photocraft_compose::flatten(&app.session.active().unwrap().doc).px, "{mode} {depth}: confirm matches preview");
                assert_eq!(smart(&app, layer).smart_filters.len(), filters.smart_filters.len());
            }
        }
    }
}

/// A document switch through the control channel leaves the modal open but hides stale controls.
#[test]
fn rabbit_stale_tone_filter_dialog_hides_controls_until_the_original_document_returns() {
    for kind in ["levels", "curves"] {
        let (mut app, layer) = app(8, "rgb");
        let command = format!("image.adjustments.{kind}");
        app.run(&command, if kind == "levels" { json!({"gamma":2}) } else { json!({"points":[[0,255],[255,0]]}) }).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let filters = smart(&app, layer).clone();
        app.run("file.new", json!({"width":32,"height":24,"mode":"lab"})).unwrap();
        let other = app.session.active().unwrap().doc.clone();
        assert!(app.session.set_active(0));
        let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        let ctx = h.ctx.clone();
        super::open_editor(h.state_mut(), &ctx, layer.0, &filters, 2);
        let id = h.state().ui.dialogs.last().unwrap().id;
        h.run_steps(3);
        assert!(h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| n.value().as_deref() == Some("RGB")));
        let (req, _rx) = crate::control::ControlRequest::new("ui.window.open", json!({"document":1}));
        crate::control::handle(h.state_mut(), &ctx, &req);
        h.run_steps(3);
        assert_eq!(h.state().ui.dialogs.len(), 1);
        assert!(h.query_all_by_role(egui::accesskit::Role::ComboBox).next().is_none(), "stale targets cannot expose the other document's editor");
        assert!(h.query_by_label("The adjustment target is missing or unavailable.").is_some());
        assert!(!crate::adjust_dialog::picker_armed(h.state()));
        assert!(std::sync::Arc::ptr_eq(&other, &h.state().session.active().unwrap().doc));
        let (req, _rx) = crate::control::ControlRequest::new("ui.window.open", json!({"document":0}));
        crate::control::handle(h.state_mut(), &ctx, &req);
        h.run_steps(3);
        assert!(std::sync::Arc::ptr_eq(&original, &h.state().session.active().unwrap().doc));
        assert!(h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| n.value().as_deref() == Some("RGB")));
        let (req, _rx) = crate::control::ControlRequest::new("ui.window.open", json!({"document":1}));
        crate::control::handle(h.state_mut(), &ctx, &req);
        assert!(crate::dialogs::confirm(h.state_mut(), id).is_err());
        assert!(std::sync::Arc::ptr_eq(&other, &h.state().session.active().unwrap().doc));
    }
}

/// Malformed whole-image Curves aliases remain preview/commit errors instead of identity curves.
#[test]
fn rabbit_invalid_whole_curves_alias_does_not_turn_neutral() {
    for mode in ["rgb", "lab", "cmyk"] {
        for bad in [json!({"gray":"bad"}), json!({"gray":[[0,0]]}), json!({"gray":[0,1,2],"pencil":true})] {
            let (mut app, layer) = app(8, mode);
            app.run("image.adjustments.curves", json!({"points":[[0,255],[255,0]]})).unwrap();
            let before = app.session.active().unwrap().doc.clone();
            let target = crate::smart_filter_dialog::target(&mut app, layer, 2).unwrap();
            let id = crate::adjust_dialog::open_smart_filter(&mut app, "image.adjustments.curves", &bad, target).unwrap();
            assert!(!app.ui.dialog_mut(id).unwrap().fields.contains_key("points"), "invalid aliases cannot be hidden by defaults");
            let idx = app.session.active_index().unwrap();
            assert!(crate::canvas::ensure_filter_preview(&mut app, idx, &egui::Context::default()).is_none());
            assert!(app.filter_preview.as_ref().is_none_or(|preview| preview.result.is_none()));
            assert!(crate::dialogs::confirm(&mut app, id).is_err());
            assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        }
    }
}

/// Whole-image Levels aliases show the recorded gamma and cannot override a subsequent edit.
#[test]
fn rabbit_whole_image_levels_alias_is_visible_and_editable() {
    for mode in ["rgb", "cmyk"] {
        for depth in [8, 16, 32] {
            let (mut app, layer) = app(depth, mode);
            // A neutral CMYK adjustment still takes the existing colour-conversion path.
            // Compare with the same public neutral command, rather than omitting that stage.
            let mut oracle = photocraft_engine::Session::new();
            oracle.add_document(app.session.active().unwrap().doc.as_ref().clone(), None);
            oracle.select_layer(layer).unwrap();
            oracle.execute("image.adjustments.levels", json!({"gamma":1})).unwrap();
            let neutral = photocraft_compose::flatten(&oracle.active().unwrap().doc).px.clone();
            app.run("image.adjustments.levels", json!({"gray":{"gamma":2}})).unwrap();
            let filters = smart(&app, layer).clone();
            let ctx = egui::Context::default();
            super::open_editor(&mut app, &ctx, layer.0, &filters, 2);
            let id = app.ui.dialogs.last().unwrap().id;
            let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
            assert_eq!(fields["gamma"], json!(2.0));
            assert!(!fields.contains_key("gray"));
            fields.insert("gamma".into(), json!(1.0));
            let idx = app.session.active_index().unwrap();
            crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).unwrap();
            let shown = photocraft_compose::flatten(app.filter_preview.as_ref().unwrap().result.as_ref().unwrap()).px.clone();
            assert!(shown == neutral, "{mode} {depth}: the visible edit matches a neutral Levels command");
            crate::dialogs::confirm(&mut app, id).unwrap();
            assert_eq!(shown, photocraft_compose::flatten(&app.session.active().unwrap().doc).px);
            assert_eq!(smart(&app, layer).smart_filters[2].params["gray"], serde_json::Value::Null);
        }
    }
}
