//! Channel-edit regressions retained across the shared UI migration.
use super::*;

#[test]
fn filter_mask_target_switches_respect_the_channel_transform_lock() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.background_jobs = false;
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#336699"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius":1})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("view.layerMask", json!({"mode":"overlay"})).unwrap();
    app.ui.mask_target = true;
    app.sync_mask_targets();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    let st = app.session.active().unwrap();
    let layer = st.active_layer.unwrap();
    let (doc, view, transform, views) = (st.doc.clone(), st.channel_view.clone(), app.ui.transform.clone(), app.ui.views.clone());
    app.select_filter_mask_target(Some(layer.0));
    assert_eq!(app.ui.filter_mask_target, None);
    assert!(app.ui.mask_target);
    let error = call(&mut app, &ctx, "ui.set", json!({"filterMaskTarget":true,"zoom":2.0}));
    assert_eq!(error["ok"], false);
    assert!(error["error"].as_str().unwrap().contains(photocraft_engine::channel_transform::LOCKED));
    assert_eq!(app.ui.views, views, "a refused request applies nothing");
    assert_eq!(app.ui.transform, transform);
    assert_eq!(app.session.active().unwrap().doc, doc);
    assert_eq!(app.session.active().unwrap().channel_view, view);
    assert!(photocraft_engine::channel_transform::locked(&app.session));
    crate::transform_tool::cancel(&mut app);
    app.select_filter_mask_target(Some(layer.0));
    assert!(app.filter_mask_targeted(), "the target can change again after Cancel");
}

#[test]
fn layer_style_automation_reports_the_channel_transform_lock() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"layerStyle"}))["error"], "no active layer");
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    app.run("channel.target", json!({"channel":"green"})).unwrap();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let captured = app.ui.transform.clone();
    let error = call(&mut app, &ctx, "ui.dialog.open", json!({"kind":"layerStyle"}));
    assert_eq!(error["ok"], false);
    assert!(error["error"].as_str().is_some_and(|message| message.contains(photocraft_engine::channel_transform::LOCKED)));
    assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
    assert_eq!(app.ui.transform, captured);
    crate::transform_tool::cancel(&mut app);
}
