//! Channel-edit regressions retained across the shared UI migration.
use super::*;

#[test]
fn frame_thumbnail_refuses_warp_without_discarding_the_affine_transform() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let st = app.session.active().unwrap();
    let (doc, steps) = (st.doc.clone(), st.history.past_len());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.begin_pass(Default::default());

    let error = "A frame supports affine transforms; select its image to warp it";
    assert_eq!(crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap_err(), error);
    assert!(app.ui.transform.is_none());
    assert!(app.transform_preview.is_none());

    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    assert!(frame_transform(app.ui.transform.as_ref().unwrap()));
    crate::channel_transform_preview::edit(&mut app, |t| t.quad = t.quad.map(|[x, y]| [x + 10.0, y + 5.0]));
    let before = app.ui.transform.clone();
    assert!(!crate::menus::is_enabled(&app, "edit.transform.warp"));
    assert!(crate::canvas_tool_menu::open_at(&mut app, Tool::Move, craft::keymap::Mods::NONE, egui::pos2(10.0, 10.0), [40.0, 50.0], None));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!crate::canvas_tool_menu::available(&app, &menu, "edit.transform.warp"));
    assert_eq!(crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap_err(), error);
    crate::transform_tool::enter_warp(&mut app, &ctx);
    assert_eq!(app.ui.transform, before);
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.past_len(), steps);

    crate::transform_tool::commit(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(30, 35, 130, 115));
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn frame_transform_bar_keeps_commit_and_cancel_without_a_warp_toggle() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let ctx = app.ctx().clone();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    let mut h = egui_kittest::Harness::builder().with_context(ctx).with_size(egui::vec2(1200.0, 100.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            ui.horizontal(|ui| crate::transform_tool::options_bar(app, ui));
        },
        app,
    );
    h.run_steps(2);
    assert!(h.query_by_label("Switch between free transform and warp modes").is_none());
    assert!(h.query_by_label("Commit transform").is_some());
    assert!(h.query_by_label("Cancel transform").is_some());
}
