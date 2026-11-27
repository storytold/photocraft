//! Channel-edit regressions retained across the shared UI migration.
use super::*;
use egui::vec2;

#[test]
fn live_transform_eyes_accept_clicks_while_channel_rows_stay_locked() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":32,"height":24})).unwrap();
    app.run("channel.target", json!({"channel":"red"})).unwrap();
    app.sync_views();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                show(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
    h.run_steps(2);
    let quad = h.state().ui.transform.as_ref().unwrap().quad_array();
    let history = h.state().session.active().unwrap().history.past_len();
    h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Green").click();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().channel_view.target, ChannelTarget::Color(0));
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Visibility Green").click();
    h.run_steps(2);
    assert!(h.state().session.active().unwrap().channel_view.color_visible(1));
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Visibility Red").click();
    h.run_steps(2);
    assert!(!h.state().session.active().unwrap().channel_view.color_visible(0));
    assert_eq!(h.state().ui.transform.as_ref().unwrap().quad_array(), quad);
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Visibility RGB").click();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().channel_view.visible_colors(3), 3);
    assert_eq!(h.state().session.active().unwrap().channel_view.target, ChannelTarget::Color(0));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    crate::transform_tool::cancel(h.state_mut());
}

#[test]
fn colour_eye_exits_grayscale_mask_view_without_unlocking_the_transform_target() {
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":32,"height":24})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("channel.target", json!({"channel":"mask"})).unwrap();
    app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                show(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Visibility Green").click();
    h.run_steps(2);
    let st = h.state().session.active().unwrap();
    assert_eq!(photocraft_engine::mask_view_cmds::current(st).unwrap().mode, photocraft_engine::mask_view_cmds::MaskViewMode::Overlay);
    assert_eq!(st.channel_view.visible_colors(3), 1);
    assert!(st.channel_view.color_visible(1));
    h.get_by_role_and_label(egui::accesskit::Role::ListItem, "Green").click();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().channel_view.target, ChannelTarget::LayerMask);
    assert_eq!(h.state().ui.transform.as_ref().unwrap().target, Some(json!("mask")));
    crate::transform_tool::cancel(h.state_mut());
}
