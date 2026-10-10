//! Viewport contextual actions. Drag the grip to move the bar; background removal uses the
//! existing engine command, background job, editable mask and undo.

use egui::{Area, Id, Order, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{PhotocraftApp, icons, menus, theme::Tokens, widgets};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskbarState {
    pub visible: bool,
    /// Normalized within the viewport's movement range, independent of zoom and document
    /// pixels. Restored bars stay reachable after a window or dock resize.
    pub position: Option<[f32; 2]>,
}

impl Default for TaskbarState {
    fn default() -> Self {
        Self { visible: true, position: None }
    }
}

/// Restore the shell-owned preference at startup. Old/malformed settings use defaults; a
/// hand-edited finite position is clamped so the bar cannot become unreachable.
pub(crate) fn restore(app: &mut PhotocraftApp) {
    let mut state: TaskbarState = serde_json::from_value(app.session.prefs().contextual_taskbar.clone()).unwrap_or_default();
    state.position = state.position.filter(|p| p.iter().all(|v| v.is_finite())).map(|p| p.map(|v| v.clamp(0.0, 1.0)));
    app.ui.contextual_taskbar = state;
}

/// Persist through the normal preference revision/save path after a drag finishes. Run even
/// while hidden so Window/menu and automation changes are remembered too.
pub(crate) fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let value = json!(app.ui.contextual_taskbar);
    if app.session.prefs().contextual_taskbar != value {
        app.session.prefs.edit(|prefs| prefs.contextual_taskbar = value);
    }
}

fn position(viewport: Rect, size: Vec2, saved: Option<[f32; 2]>) -> Pos2 {
    let room = (viewport.size() - size).max(Vec2::ZERO);
    let [x, y] = saved.filter(|p| p.iter().all(|v| v.is_finite())).unwrap_or([0.5, 0.94]);
    viewport.min + vec2(x.clamp(0.0, 1.0) * room.x, y.clamp(0.0, 1.0) * room.y)
}

fn normalized(viewport: Rect, size: Vec2, at: Pos2) -> [f32; 2] {
    let room = (viewport.size() - size).max(Vec2::ZERO);
    let relative = at - viewport.min;
    [if room.x > 0.0 { (relative.x / room.x).clamp(0.0, 1.0) } else { 0.0 }, if room.y > 0.0 { (relative.y / room.y).clamp(0.0, 1.0) } else { 0.0 }]
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.contextual_taskbar.visible
        || app.ui.tool == crate::Tool::Crop
        || app.ui.view.hides_chrome()
        || app.jobs.focus.is_some()
        || !menus::modal_allows(app, "layer.removeBackground")
        || app.ui.transform.is_some()
    {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return };
    if !matches!(layer.content, photocraft_doc::LayerContent::Raster(_) | photocraft_doc::LayerContent::Smart(_)) {
        return;
    }
    let enabled = menus::is_enabled(app, "layer.removeBackground");
    let viewport = app.last_canvas_rect.intersect(ctx.content_rect()).shrink(10.0);
    if viewport.width() < 220.0 || viewport.height() < 60.0 {
        return;
    }
    let remembered = ctx.data(|d| d.get_temp::<Vec2>(Id::new("contextual-taskbar-size")));
    let size = remembered.unwrap_or(vec2(220.0, 44.0));
    if viewport.width() < size.x || viewport.height() < size.y {
        return;
    }
    let at = position(viewport, size, app.ui.contextual_taskbar.position);
    let t = Tokens::get(ctx);
    let mut remove = false;
    let mut movement = Vec2::ZERO;
    let response = Area::new(Id::new("contextual-taskbar")).order(Order::Foreground).fixed_pos(at).movable(false).constrain_to(viewport).show(ctx, |ui| {
        egui::Frame::new().fill(t.card).stroke(Stroke::new(1.0, t.card_border)).corner_radius(t.radius).inner_margin(8).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (grip, drag) = ui.allocate_exact_size(vec2(18.0, 26.0), Sense::drag());
                icons::paint(ui, grip, "move", 14.0, t.text_dim);
                drag.clone().on_hover_text(tl!("Drag contextual task bar"));
                drag.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, tl!("Drag contextual task bar")));
                if drag.dragged() {
                    movement = drag.drag_delta();
                }
                remove = ui
                    .add_enabled_ui(enabled, |ui| widgets::secondary_button(ui, tl!("Remove Background"), 140.0))
                    .inner
                    .on_hover_text(tl!("Hide the background with an editable layer mask."))
                    .clicked();
                let options = ui
                    .menu_button("…", |ui| {
                        if ui.button(tl!("Reset position")).clicked() {
                            app.ui.contextual_taskbar.position = None;
                            ui.close();
                        }
                        if ui.button(tl!("Hide contextual task bar")).clicked() {
                            app.ui.contextual_taskbar.visible = false;
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text(tl!("Contextual task bar options"));
                options.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, tl!("Contextual task bar options")));
            });
        });
    });
    let rect = response.response.rect;
    ctx.data_mut(|d| d.insert_temp(Id::new("contextual-taskbar-size"), rect.size()));
    if movement != Vec2::ZERO {
        app.ui.contextual_taskbar.position = Some(normalized(viewport, rect.size(), rect.min + movement));
        ctx.request_repaint();
    }
    if remove && let Err(error) = menus::invoke(app, ctx, "layer.removeBackground", json!({})) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;
    use egui_kittest::{Harness, kittest::Queryable};

    fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: serde_json::Value) -> serde_json::Value {
        let (request, _) = crate::control::ControlRequest::new(method, params);
        match crate::control::handle(app, ctx, &request) {
            crate::control::Outcome::Done(result) => result,
            _ => panic!("expected a synchronous UI response"),
        }
    }

    #[test]
    fn old_settings_get_defaults_and_taskbar_settings_round_trip() {
        let state = crate::UiState { contextual_taskbar: TaskbarState { visible: false, position: Some([0.25, 0.75]) }, ..Default::default() };
        let mut value = serde_json::to_value(&state).unwrap();
        let restored: crate::UiState = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.contextual_taskbar, state.contextual_taskbar);
        value.as_object_mut().unwrap().remove("contextual_taskbar");
        let old: crate::UiState = serde_json::from_value(value).unwrap();
        assert_eq!(old.contextual_taskbar, TaskbarState::default());
    }

    #[test]
    fn visibility_and_position_survive_preference_save_and_restart() {
        use std::sync::{Arc, Mutex};
        let store = Arc::new(Mutex::new(None::<String>));
        let (read, write) = (store.clone(), store.clone());
        let services = crate::Services {
            load_prefs: Some(Box::new(move || read.lock().unwrap().clone())),
            save_prefs: Some(Box::new(move |value| {
                *write.lock().unwrap() = Some(value.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        app.ui.contextual_taskbar = TaskbarState { visible: false, position: Some([0.25, 0.75]) };
        persist(&mut app, &ctx);
        crate::prefs_ui::tick(&mut app, &ctx);
        let saved = store.lock().unwrap().clone().unwrap();
        let values: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(values["contextualTaskbar"], json!({"visible":false,"position":[0.25,0.75]}));
        let mut session = photocraft_engine::Session::new();
        session.load_prefs_json(&saved).unwrap();
        let restarted = PhotocraftApp::new(session, Default::default());
        assert_eq!(restarted.ui.contextual_taskbar, app.ui.contextual_taskbar);
        app.session.prefs.edit(|p| p.contextual_taskbar = json!({"visible":false,"position":[50,-4]}));
        restore(&mut app);
        assert_eq!(app.ui.contextual_taskbar, TaskbarState { visible: false, position: Some([1.0, 0.0]) });
        app.session.prefs.edit(|p| p.contextual_taskbar = json!({"visible":"bad","position":[1e300,0.5]}));
        restore(&mut app);
        assert_eq!(app.ui.contextual_taskbar, TaskbarState::default());
    }

    #[test]
    fn automation_patches_read_back_and_window_menu_restores_hidden_bar() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let set = call(&mut app, &ctx, "ui.set", json!({"contextualTaskbar":{"visible":false,"position":[0.2,0.8]}}));
        assert_eq!(set["ok"], true, "{set}");
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(inspected["result"]["contextualTaskbar"]["visible"], false);
        assert_eq!(app.ui.contextual_taskbar.position, Some([0.2, 0.8]));
        let item = menus::menu_items(&app).into_iter().find(|i| i.id == "window.contextualTaskbar").unwrap();
        assert!(item.enabled);
        assert_eq!(item.checked, Some(false));
        assert_eq!(call(&mut app, &ctx, "ui.menu.invoke", json!({"id":"window.contextualTaskbar"}))["ok"], true);
        assert!(app.ui.contextual_taskbar.visible);
        assert_eq!(app.ui.contextual_taskbar.position, Some([0.2, 0.8]));
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"contextualTaskbar":{"position":null}}))["ok"], true);
        assert!(app.ui.contextual_taskbar.visible, "a position patch preserves visibility");
        assert_eq!(app.ui.contextual_taskbar.position, None);
    }

    #[test]
    fn invalid_taskbar_and_mixed_ui_updates_are_atomic() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let before = app.ui.clone();
        for params in [
            json!({"tool":"move","contextualTaskbar":{"position":[-0.1,0.5]}}),
            json!({"tool":"move","contextualTaskbar":{"position":[0.5,1.1]}}),
            json!({"tool":"move","contextualTaskbar":{"position":[1e300,0.5]}}),
            json!({"tool":"move","contextualTaskbar":{"position":[0.5]}}),
            json!({"tool":"move","contextualTaskbar":{"position":[0.2,0.5,0.8]}}),
            json!({"tool":"move","contextualTaskbar":{"position":"here"}}),
            json!({"tool":"move","contextualTaskbar":{"visible":"yes"}}),
            json!({"tool":"move","contextualTaskbar":{"unknown":true}}),
            json!({"tool":"move","contextualTaskbar":null}),
            json!({"contextualTaskbar":{"visible":false},"theme":"invalid"}),
            json!({"contextualTaskbar":{"visible":false},"brushSize":1e300}),
        ] {
            let result = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(result["ok"], false, "{params}: {result}");
            assert_eq!(app.ui, before, "{params}: no field from a rejected call is applied");
        }
    }

    #[test]
    fn toolbar_labels_cover_every_complete_language() {
        let labels = [
            "Contextual Task Bar",
            "Drag contextual task bar",
            "Reset position",
            "Hide contextual task bar",
            "Contextual task bar options",
            "Hide the background with an editable layer mask.",
            "Remove Background",
        ];
        for info in &crate::i18n::LANGUAGES {
            if !info.complete_menus {
                continue;
            }
            let lang = crate::i18n::Lang::from_pref(info.code);
            let missing: Vec<_> = labels.iter().filter(|label| !crate::i18n::has(lang, label)).collect();
            assert!(missing.is_empty(), "{}: missing taskbar labels: {missing:?}", info.code);
        }
    }

    #[test]
    fn restored_positions_and_small_viewports_keep_the_bar_reachable() {
        let viewport = Rect::from_min_size(pos2(40.0, 60.0), vec2(300.0, 200.0));
        let size = vec2(160.0, 60.0);
        let p = position(viewport, size, Some([f32::NAN, -100.0]));
        assert!(viewport.contains_rect(Rect::from_min_size(p, size)));
        assert_eq!(position(viewport, size, Some([20.0, -2.0])), pos2(180.0, 60.0));
        assert_eq!(normalized(viewport, size, pos2(-400.0, 600.0)), [0.0, 1.0]);
        let smaller = Rect::from_min_size(pos2(20.0, 30.0), vec2(180.0, 70.0));
        assert!(smaller.contains_rect(Rect::from_min_size(position(smaller, size, Some([1.0, 1.0])), size)));
    }

    #[test]
    fn modal_rotation_hides_actions_and_cancel_restores_them_without_an_edit() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":400,"height":300})).unwrap();
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(session, crate::Services::default())
        });
        h.run_steps(8);
        assert!(h.query_by_label("Drag contextual task bar").is_some());
        let revision = h.state().session.active().unwrap().revision;
        let ctx = h.ctx.clone();
        let opened = menus::invoke(h.state_mut(), &ctx, "image.rotation.arbitrary", json!({})).unwrap();
        let id = opened["dialog"].as_u64().unwrap();
        h.run_steps(8);
        assert!(h.query_by_label("Drag contextual task bar").is_none(), "modal previews must have an unobstructed canvas");
        assert!(h.query_by_label("Remove Background").is_none());
        h.state_mut().ui.close_dialog(id).unwrap();
        h.run_steps(8);
        assert!(h.query_by_label("Drag contextual task bar").is_some());
        assert_eq!(h.state().session.active().unwrap().revision, revision);
    }

    #[test]
    fn pixel_and_placed_smart_layers_show_actions_and_text_layers_do_not() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":400,"height":300})).unwrap();
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(session, crate::Services::default())
        });
        h.run_steps(8);
        assert!(h.query_by_label("Remove Background").is_some());
        h.state_mut().session.execute("select.rect", json!({"x":10,"y":20,"width":80,"height":60})).unwrap();
        h.run_steps(8);
        assert!(h.query_by_label("Remove Background").is_some());
        h.state_mut().session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        h.run_steps(8);
        assert!(h.query_by_label("Remove Background").is_some());
        h.state_mut().ui.tool = crate::Tool::Crop;
        h.run_steps(8);
        assert!(h.query_by_label("Drag contextual task bar").is_none(), "Crop owns the viewport, even before a pending frame");
        h.state_mut().ui.tool = crate::Tool::Brush;
        h.run_steps(8);
        assert!(h.query_by_label("Drag contextual task bar").is_some());
        h.state_mut().session.execute("type.create", json!({"text":"Title","size":24,"x":20,"y":40})).unwrap();
        h.run_steps(8);
        assert!(h.query_by_label("Remove Background").is_none());
    }
    fn photo() -> Vec<u8> {
        let mut bytes = b"P6\n200 160\n255\n".to_vec();
        for y in 0_i32..160 {
            for x in 0_i32..200 {
                let inside = (x - 100).pow(2) + (y - 80).pow(2) < 45 * 45;
                let noise = ((x * 13 + y * 7) % 17) as u8;
                bytes.extend_from_slice(&if inside { [210 + noise, 50 + noise, 30 + noise] } else { [60 + noise, 100 + noise, 170 + noise] });
            }
        }
        bytes
    }

    #[test]
    fn placed_photo_button_click_creates_one_editable_mask_and_drag_does_not_paint() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":200,"height":160})).unwrap();
        photocraft_engine::file_cmds::place_bytes(&mut session, "dropped-photo.ppm", photo(), None, &json!({})).unwrap();
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(session, crate::Services::default())
        });
        h.run_steps(8);
        let layer = h.state().session.active().unwrap().active_layer.unwrap();
        let before = h.state().session.active().unwrap().doc.layer(layer).unwrap().content.clone();
        let steps = h.state().session.active().unwrap().history.past_len();
        let start = h.get_by_label("Drag contextual task bar").rect().center();
        let end = start + vec2(-120.0, -100.0);
        h.event(egui::Event::PointerMoved(start));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (k as f32 / 6.0)));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(4);
        assert!(h.state().ui.contextual_taskbar.position.is_some());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "dragging the bar must not paint");
        assert_eq!(h.state().session.active().unwrap().doc.layer(layer).unwrap().content, before);
        h.get_by_label("Remove Background").click();
        h.run_steps(8);
        let state = h.state().session.active().unwrap();
        assert!(state.doc.layer(layer).unwrap().mask.is_some(), "floating action should dispatch the existing command: {}", h.state().ui.status);
        assert_eq!(state.history.past_len(), steps + 1);
        assert_eq!(state.doc.layer(layer).unwrap().content, before);
        h.state_mut().session.execute("edit.undo", json!({})).unwrap();
        assert!(h.state().session.active().unwrap().doc.layer(layer).unwrap().mask.is_none());
        h.state_mut().session.execute("edit.redo", json!({})).unwrap();
        assert!(h.state().session.active().unwrap().doc.layer(layer).unwrap().mask.is_some());
        h.run_steps(8);
        h.get_by_label("Contextual task bar options").click();
        h.run_steps(8);
        h.get_by_label("Reset position").click();
        h.run_steps(8);
        assert_eq!(h.state().ui.contextual_taskbar.position, None);
        h.get_by_label("Contextual task bar options").click();
        h.run_steps(8);
        h.get_by_label("Hide contextual task bar").click();
        h.run_steps(8);
        assert!(!h.state().ui.contextual_taskbar.visible);
        assert!(h.query_by_label("Drag contextual task bar").is_none());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "view controls do not edit the document");
    }
}
