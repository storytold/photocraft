//! 3D workspace chrome and view layout. Camera/panel changes do not edit documents.
use crate::{
    PhotocraftApp, icons,
    theme::Tokens,
    uv_ui::{self, OrbitPivot},
};
use egui::{Color32, Rect, Sense, Ui, vec2};
use serde_json::Value;

/// Only small view preferences are remembered: no file paths, status, strokes or open dialogs.
pub(crate) fn preferences(app: &PhotocraftApp) -> Value {
    let s = &app.ui.uv_workbench;
    serde_json::json!({"paint_wrap":s.paint_wrap,"show_overlay":s.show_overlay,"show_wireframe":s.show_wireframe,"wireframe_opacity":s.wireframe_opacity,"wireframe_color":s.wireframe_color,"split_view":s.split_view,"swap_views":s.swap_views,"split_ratio":s.split_ratio,"view_3d":s.view_3d,"perspective":s.perspective,"antialias":s.antialias,"fast_navigation":s.fast_navigation,"orbit_pivot":s.orbit_pivot,"shaded":s.shaded,"channel":s.channel,"channel_2d":s.channel_2d,"shell_enclosed":s.shell_enclosed,"cull_backfaces":s.cull_backfaces,"flip_faces":s.flip_faces,"lighting":s.lighting,"normal_strength":s.normal_strength,"flip_normal_y":s.flip_normal_y})
}
pub(crate) fn restore_preferences(app: &mut PhotocraftApp, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    let Ok(saved) = serde_json::from_value::<uv_ui::State>(value.clone()) else {
        return;
    };
    let s = &mut app.ui.uv_workbench;
    s.show_overlay = saved.show_overlay;
    s.show_wireframe = saved.show_wireframe;
    s.wireframe_opacity = if saved.wireframe_opacity.is_finite() { saved.wireframe_opacity.clamp(0.0, 1.0) } else { uv_ui::State::default().wireframe_opacity };
    s.wireframe_color = saved.wireframe_color;
    s.paint_wrap = saved.paint_wrap;
    s.split_view = saved.split_view;
    s.swap_views = saved.swap_views;
    s.split_ratio = saved.split_ratio;
    s.view_3d = saved.view_3d;
    s.perspective = saved.perspective;
    s.antialias = saved.antialias;
    s.fast_navigation = saved.fast_navigation;
    s.orbit_pivot = saved.orbit_pivot;
    s.shaded = saved.shaded;
    s.channel = saved.channel;
    s.channel_2d = if value.get("channel_2d").is_some() {
        saved.channel_2d
    } else if saved.channel != photocraft_uv::preview::Channel::Material {
        saved.channel
    } else {
        app.session.uv.current_channel()
    };
    s.shell_enclosed = saved.shell_enclosed;
    s.cull_backfaces = saved.cull_backfaces;
    s.flip_faces = saved.flip_faces;
    s.lighting = saved.lighting.sanitized();
    s.normal_strength = saved.normal_strength;
    s.flip_normal_y = saved.flip_normal_y;
    s.split_ratio = if s.split_ratio.is_finite() { s.split_ratio.clamp(0.2, 0.8) } else { 0.5 };
    s.normal_strength = s.normal_strength.clamp(0.0, 4.0);
}

pub(crate) fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if app.ui.workspace == "3D" && matches!(id, "file.close" | "file.closeAll") {
        return Some(app.session.uv.scene.is_some());
    }
    if app.ui.workspace == "3D" && matches!(id, "file.save" | "file.saveAs") {
        return Some(app.session.uv.scene.is_some());
    }
    if !id.starts_with("file.uv.") {
        return None;
    }
    Some(match id {
        "file.uv.import" | "file.uv.settings" | "file.uv.openProject" => true,
        "file.uv.saveAllTextures" => app.session.uv.scene.is_some(),
        "file.uv.closeProject" => app.session.uv.scene.is_some(),
        "file.uv.reimport" | "file.uv.saveProject" => app.session.uv.scene.is_some() && app.drag.is_none(),
        _ => false,
    })
}
pub(crate) fn menu(app: &mut PhotocraftApp, id: &str) -> Option<Result<Value, String>> {
    if app.ui.workspace == "3D" && app.session.uv.scene.is_some() && matches!(id, "file.save" | "file.saveAs") {
        return Some(if id == "file.saveAs" { uv_ui::save_project_as(app) } else { uv_ui::save_project(app) });
    }
    if !id.starts_with("file.uv.") {
        return None;
    }
    if is_enabled(app, id) != Some(true) {
        return Some(Err("3D command unavailable".into()));
    }
    Some(match id {
        "file.uv.import" => uv_ui::reveal(app).and_then(|()| uv_ui::pick_file(app, "model", 0, false)),
        "file.uv.openProject" => uv_ui::reveal(app).and_then(|()| uv_ui::pick_file(app, "project", 0, false)),
        "file.uv.saveProject" => uv_ui::save_project(app),
        "file.uv.closeProject" => {
            crate::uv_project::close(app);
            Ok(Value::Null)
        }
        "file.uv.saveAllTextures" => {
            if let Err(e) = uv_ui::reveal(app) {
                return Some(Err(e));
            }
            uv_ui::finish_paint(app);
            if let Some([x, y, _]) = app.drag.as_ref().filter(|d| crate::canvas::strokes_live(d.tool)).and_then(|d| d.points.last()).copied() {
                crate::canvas::tool_event(app, crate::canvas::ToolEvent::Up { x, y }, egui::Modifiers::NONE);
            }
            app.run(id, serde_json::json!({})).inspect(|v| {
                app.ui.status = crate::i18n::fmt(
                    tl!("Export Textures: exported {saved}, missing path {unsaved}, errors {errors}"),
                    &[
                        ("saved", &v["saved"].to_string()),
                        ("unsaved", &v["unsaved"].to_string()),
                        ("errors", &v["errors"].as_array().map_or(0, Vec::len).to_string()),
                    ],
                );
                app.ui.status_error = v["errors"].as_array().is_some_and(|e| !e.is_empty());
                app.ui.uv_workbench.status =
                    v["errors"].as_array().map(|e| e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")).unwrap_or_default();
            })
        }
        "file.uv.reimport" => {
            if let Err(e) = uv_ui::reveal(app) {
                return Some(Err(e));
            }
            uv_ui::reimport(app);
            Ok(Value::Null)
        }
        "file.uv.settings" => {
            if let Err(e) = uv_ui::reveal(app) {
                return Some(Err(e));
            }
            app.ui.uv_workbench.import_settings = true;
            Ok(Value::Null)
        }
        _ => Err("Unknown 3D command".into()),
    })
}
fn layout(app: &mut PhotocraftApp, mode: u8) {
    uv_ui::finish_paint(app);
    let s = &mut app.ui.uv_workbench;
    s.split_view = mode == 0;
    s.view_3d = mode != 2;
    s.floating_3d = false;
    s.floating_2d = false;
}
/// Egui's consume_key permits extra Shift: view keys must match all modifiers exactly.
pub(crate) fn plain_key(ctx: &egui::Context, key: egui::Key) -> bool {
    ctx.input_mut(|i| {
        let found = i.events.iter().any(|e| matches!(e,egui::Event::Key{key:k,pressed:true,modifiers,..} if *k==key && *modifiers==egui::Modifiers::NONE));
        if found {
            i.events.retain(|e| !matches!(e,egui::Event::Key{key:k,pressed:true,modifiers,..} if *k==key && *modifiers==egui::Modifiers::NONE));
        }
        found
    })
}
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let viewport = ctx.viewport_id();
    if app.ui.workspace != "3D"
        || app.session.uv.scene.is_none()
        || ctx.egui_wants_keyboard_input()
        || !ctx.input(|i| {
            i.pointer.hover_pos().is_some_and(|p| {
                let main = viewport == egui::ViewportId::ROOT && app.uv_cache.area.is_some_and(|r| r.contains(p));
                let model = app.uv_cache.viewport_id.is_none_or(|id| id == viewport) && app.uv_cache.viewport.is_some_and(|r| r.contains(p));
                main || model
            })
        })
    {
        return false;
    }
    for (key, mode) in [(egui::Key::F1, 0), (egui::Key::F2, 1), (egui::Key::F3, 2)] {
        if plain_key(ctx, key) {
            layout(app, mode);
            return true;
        }
    }
    if plain_key(ctx, egui::Key::F4) {
        app.ui.uv_workbench.swap_views = !app.ui.uv_workbench.swap_views;
        return true;
    }
    for (key, perspective) in [(egui::Key::F5, true), (egui::Key::F6, false)] {
        if plain_key(ctx, key) {
            uv_ui::finish_paint(app);
            app.ui.uv_workbench.perspective = perspective;
            return true;
        }
    }
    false
}
pub(crate) fn shell_tool(app: &mut PhotocraftApp, ui: &mut Ui, size: f32) {
    if app.ui.workspace != "3D" {
        return;
    }
    let response = icons::button(ui, "square-dashed-mouse-pointer", size, app.ui.uv_workbench.select_shells, tl!("UV shell selection"));
    if response.clicked() {
        uv_ui::finish_paint(app);
        app.ui.uv_workbench.select_shells = true;
        uv_ui::begin_polygon_selection_wireframe(&mut app.ui.uv_workbench);
        app.uv_cache.shell_tool_owner = Some(app.ui.tool);
    }
    if !app.ui.uv_workbench.select_shells {
        egui::Popup::close_id(ui.ctx(), egui::Popup::default_response_id(&response));
        return;
    }
    egui::Popup::from_toggle_button_response(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_max_width(360.0);
        ui.strong(tl!("UV shell selection"));
        uv_ui::shell_settings(app, ui);
    });
}
fn icon_menu<R>(ui: &mut Ui, icon: &str, tip: &str, body: impl FnOnce(&mut Ui) -> R) {
    icon_menu_with_close_behavior(ui, icon, tip, egui::PopupCloseBehavior::CloseOnClick, body);
}
fn icon_menu_with_close_behavior<R>(ui: &mut Ui, icon: &str, tip: &str, close_behavior: egui::PopupCloseBehavior, body: impl FnOnce(&mut Ui) -> R) {
    let image = icons::image(icon, 17.0, Tokens::get(ui.ctx()).icon);
    let config = egui::containers::menu::MenuConfig::new().close_behavior(close_behavior);
    let (response, _) = egui::containers::menu::MenuButton::from_button(egui::Button::image(image).min_size(vec2(32.0, 26.0))).config(config).ui(ui, body);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    response.on_hover_text(tip);
}

fn wireframe_menu_rect_id() -> egui::Id {
    egui::Id::unique("uv-wireframe-parent-menu-rect")
}

fn wireframe_picker_rect_id() -> egui::Id {
    egui::Id::unique("uv-wireframe-picker-rect")
}

fn bar(app: &mut PhotocraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let mut display_menu_rect = None;
    let mut display_menu_button = None;
    egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(5, 2)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            icon_menu(ui, "panels-top-left", tl!("3D / 2D views"), |ui| {
                for (label, mode) in
                    [(tl!("3D / 2D                       F1"), 0), (tl!("3D only                      F2"), 1), (tl!("2D only                      F3"), 2)]
                {
                    let s = &app.ui.uv_workbench;
                    if ui.selectable_label(if mode == 0 { s.split_view } else { !s.split_view && s.view_3d == (mode == 1) }, label).clicked() {
                        layout(app, mode);
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button(tl!("Swap 3D / 2D                 F4")).clicked() {
                    app.ui.uv_workbench.swap_views = !app.ui.uv_workbench.swap_views;
                    ui.close();
                }
                if ui.button(tl!("Open 3D in a window")).clicked() {
                    layout(app, 2);
                    app.ui.uv_workbench.floating_3d = true;
                    ui.close();
                }
                if ui.button(tl!("Open 2D in a window")).clicked() {
                    layout(app, 1);
                    app.ui.uv_workbench.floating_2d = true;
                    ui.close();
                }
                ui.separator();
                ui.checkbox(&mut app.ui.uv_workbench.show_overlay, tl!("Show UV shells"));
            });
            icon_menu(ui, if app.ui.uv_workbench.perspective { "uv-perspective" } else { "uv-orthographic" }, tl!("Camera projection"), |ui| {
                for (p, label) in [(true, tl!("Perspective view             F5")), (false, tl!("Orthographic view            F6"))] {
                    if ui
                        .add(
                            egui::Button::image_and_text(crate::icons::image(if p { "uv-perspective" } else { "uv-orthographic" }, 16.0, t.icon), label)
                                .selected(app.ui.uv_workbench.perspective == p),
                        )
                        .clicked()
                    {
                        uv_ui::finish_paint(app);
                        app.ui.uv_workbench.perspective = p;
                        ui.close();
                    }
                }
            });
            ui.separator();
            let image = icons::image("sun", 17.0, t.icon);
            let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            let (response, _) =
                egui::containers::menu::MenuButton::from_button(egui::Button::image(image).min_size(vec2(32.0, 26.0))).config(config).ui(ui, |ui| {
                    let s = &mut app.ui.uv_workbench;
                    ui.checkbox(&mut s.shaded, tl!("Lighting"));
                    ui.checkbox(&mut s.show_wireframe, tl!("Show polygon wireframe"));
                    if s.show_wireframe {
                        ui.add(egui::Slider::new(&mut s.wireframe_opacity, 0.0..=1.0).text(tl!("Wireframe opacity")));
                        ui.horizontal(|ui| {
                            ui.label(tl!("Wireframe color"));
                            let color = Color32::from_rgba_unmultiplied(s.wireframe_color[0], s.wireframe_color[1], s.wireframe_color[2], s.wireframe_color[3]);
                            if crate::widgets::color_swatch_button(ui, color, tl!("Wireframe color")).clicked() {
                                s.wireframe_picker_open = true;
                            }
                        });
                    }
                    ui.checkbox(&mut s.antialias, tl!("MSAA antialiasing"));
                    ui.checkbox(&mut s.fast_navigation, tl!("Smooth navigation")).on_hover_text(tl!(
                        "Up to 1600 px while navigating; source maps stay full size and full viewport resolution returns when you release the mouse."
                    ));
                    ui.checkbox(&mut s.cull_backfaces, tl!("Cull backfaces"));
                    ui.checkbox(&mut s.flip_faces, tl!("Flip faces"));
                    ui.checkbox(&mut s.lighting.pbr, tl!("PBR · Roughness / Metallic"));
                    ui.add(egui::Slider::new(&mut s.normal_strength, 0.0..=4.0).text(tl!("Normal strength")));
                    ui.checkbox(&mut s.flip_normal_y, tl!("Normal: DirectX Y−"));
                    if let Some(gpu) = &app.uv_cache.gpu
                        && let Some((size, samples)) = gpu.frame_info()
                    {
                        ui.separator();
                        ui.weak(format!("GPU · {} × {} · MSAA {}×", size[0], size[1], samples)).on_hover_text(gpu.adapter_label());
                    }
                    display_menu_rect = Some(ui.min_rect().expand(4.0));
                });
            let response = response.on_hover_text(tl!("3D display"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("3D display")));
            display_menu_button = Some(response);
            if app.ui.workspace == "3D" && app.ui.uv_workbench.view_3d && app.session.uv.scene.is_some() {
                icon_menu(ui, "painting-symmetry", tl!("3D brush symmetry"), |ui| crate::uv_symmetry::menu(ui));
            }
            icon_menu(ui, "sliders-horizontal", tl!("Camera navigation"), |ui| {
                ui.label(tl!("Orbit pivot"));
                for mode in [OrbitPivot::ViewCenter, OrbitPivot::Cursor, OrbitPivot::Fixed] {
                    ui.selectable_value(&mut app.ui.uv_workbench.orbit_pivot, mode, mode.label());
                }
                ui.separator();
                if ui.button(tl!("Reset camera")).clicked() {
                    let c = photocraft_uv::preview::Camera::default();
                    let s = &mut app.ui.uv_workbench;
                    s.yaw = c.yaw;
                    s.pitch = c.pitch;
                    s.zoom = c.zoom;
                    s.pan = [0.0; 2];
                    s.target = None;
                    ui.close();
                }
                ui.weak(tl!("Alt+LMB — orbit · Alt+MMB — pan"));
                ui.weak(tl!("Alt+RMB — dolly · Ctrl+LMB — pipette · F / A — frame"));
            });
            ui.separator();
            ui.weak(if app.ui.uv_workbench.split_view {
                "3D / 2D"
            } else if app.ui.uv_workbench.view_3d {
                "3D"
            } else {
                "2D"
            });
            ui.separator();
            use photocraft_uv::preview::Channel;
            let available: Vec<_> = Channel::MAPS.into_iter().filter(|kind| uv_ui::has_channel(app, *kind)).collect();
            if !available.contains(&app.ui.uv_workbench.channel_2d)
                && let Some(channel) = available.first().copied()
            {
                uv_ui::select_2d_channel(app, channel);
            }
            let previous = app.ui.uv_workbench.channel;
            let s = &mut app.ui.uv_workbench;
            if s.channel != Channel::Material && !available.contains(&s.channel) {
                s.channel = Channel::Material;
            }
            let response = craft_ui::ComboBox::from_id_salt("uv-display-channel")
                .width(170.0)
                .selected_text(s.channel.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.channel, Channel::Material, tl!("Material"));
                    for kind in available.iter().copied() {
                        ui.selectable_value(&mut s.channel, kind, kind.label());
                    }
                })
                .response;
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tl!("Display channel")));
            response.on_hover_text(tl!("Material shows the shaded material. Map channels show unlit texture colors."));
            let channel = app.ui.uv_workbench.channel;
            if channel != previous {
                uv_ui::select_view_channel(app, channel);
            }
            if app.ui.uv_workbench.split_view || !app.ui.uv_workbench.view_3d || app.ui.uv_workbench.floating_2d {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let before = app.ui.uv_workbench.channel_2d;
                    let mut selected = before;
                    let response = craft_ui::ComboBox::from_id_salt("uv-2d-channel")
                        .width(150.0)
                        .selected_text(selected.label())
                        .show_ui(ui, |ui| {
                            for kind in available.iter().copied() {
                                ui.selectable_value(&mut selected, kind, kind.label());
                            }
                        })
                        .response;
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tl!("2D map")));
                    response.on_hover_text(tl!("The 2D map can be selected independently of the 3D display"));
                    ui.weak("2D");
                    if selected != before {
                        uv_ui::select_2d_channel(app, selected);
                    }
                });
            }
        });
    });
    let ctx = ui.ctx().clone();
    let parent_rect = display_menu_rect.or_else(|| ctx.data(|d| d.get_temp::<Rect>(wireframe_menu_rect_id())));
    if let Some(rect) = display_menu_rect {
        ctx.data_mut(|d| d.insert_temp(wireframe_menu_rect_id(), rect));
    }
    let parent_id = display_menu_button.as_ref().map(egui::Popup::default_response_id);
    let parent_was_clicked = display_menu_button.as_ref().is_some_and(egui::Response::clicked);
    if parent_was_clicked && app.ui.uv_workbench.wireframe_picker_open {
        // Clicking the parent button dismisses only the palette and leaves its menu open.
        app.ui.uv_workbench.wireframe_picker_open = false;
        if let Some(id) = parent_id {
            egui::Popup::open_id(&ctx, id);
        }
    }
    if app.ui.uv_workbench.wireframe_picker_open {
        let mut open = true;
        let mut color = Color32::from_rgba_unmultiplied(
            app.ui.uv_workbench.wireframe_color[0],
            app.ui.uv_workbench.wireframe_color[1],
            app.ui.uv_workbench.wireframe_color[2],
            app.ui.uv_workbench.wireframe_color[3],
        );
        let mut changed = false;
        let position = parent_rect.map_or_else(|| ui.next_widget_position(), |r| egui::pos2(r.right() + 5.0, r.top()));
        let shown = egui::Window::new(tl!("Wireframe color"))
            .id(egui::Id::unique("uv-wireframe-color-picker"))
            .open(&mut open)
            .fixed_pos(position)
            .default_width(350.0)
            .resizable(false)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.set_min_width(330.0);
                changed = egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::BlendOrAdditive);
            });
        if let Some(shown) = shown {
            ctx.data_mut(|d| d.insert_temp(wireframe_picker_rect_id(), shown.response.rect));
        }

        // The picker is a sibling window, so preserve the 3D menu while it receives clicks.
        // A click outside both windows dismisses the picker; the menu's own close behavior then
        // dismisses its popup as usual.
        let picker_rect = ctx.data(|d| d.get_temp::<Rect>(wireframe_picker_rect_id()));
        let (click_pos, clicked) = ctx.input(|i| {
            let pressed = i.pointer.button_pressed(egui::PointerButton::Primary)
                || i.pointer.button_pressed(egui::PointerButton::Secondary)
                || i.pointer.button_pressed(egui::PointerButton::Middle);
            (i.pointer.interact_pos(), pressed)
        });
        if clicked && let Some(pos) = click_pos {
            let in_parent = parent_rect.is_some_and(|rect| rect.contains(pos));
            let in_picker = picker_rect.is_some_and(|rect| rect.contains(pos));
            if in_picker {
                if let Some(id) = parent_id {
                    egui::Popup::open_id(&ctx, id);
                }
            } else if !in_parent {
                open = false;
                if let Some(id) = parent_id {
                    egui::Popup::close_id(&ctx, id);
                }
            }
        }
        app.ui.uv_workbench.wireframe_picker_open = open;
        if changed {
            app.ui.uv_workbench.wireframe_color = color.to_srgba_unmultiplied();
        }
    }
}
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut Ui) -> bool {
    if app.ui.workspace != "3D" {
        return false;
    }
    if app.ui.uv_workbench.floating_3d {
        app.ui.uv_workbench.floating_2d = false;
        app.ui.uv_workbench.view_3d = false;
        app.ui.uv_workbench.split_view = false;
    } else if app.ui.uv_workbench.floating_2d {
        app.ui.uv_workbench.view_3d = true;
        app.ui.uv_workbench.split_view = false;
    }
    if app.ui.uv_workbench.select_shells {
        if app.uv_cache.shell_tool_owner.is_some_and(|tool| tool != app.ui.tool) {
            if crate::uv_ui::commit_current_polygon_selection(app) {
                app.ui.uv_workbench.select_shells = false;
            }
        } else {
            app.uv_cache.shell_tool_owner = Some(app.ui.tool);
        }
    }
    app.uv_cache.area = Some(ui.available_rect_before_wrap());
    bar(app, ui);
    if app.session.uv.scene.is_none() {
        return false;
    }
    if !app.ui.uv_workbench.split_view {
        if app.ui.uv_workbench.view_3d {
            uv_ui::main_view(app, ui);
            return true;
        }
        if !app.ui.uv_workbench.floating_3d {
            uv_ui::finish_paint(app);
        }
        crate::canvas::uv_documents(app, ui);
        return true;
    }
    let rect = ui.available_rect_before_wrap();
    let ratio = app.ui.uv_workbench.split_ratio.clamp(0.2, 0.8);
    let x = rect.left() + rect.width() * ratio;
    let divider = Rect::from_min_max(egui::pos2(x - 3.0, rect.top()), egui::pos2(x + 3.0, rect.bottom()));
    let response = ui.interact(divider, ui.scope_id().with("uv-divider"), Sense::drag());
    if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        app.ui.uv_workbench.split_ratio = ((p.x - rect.left()) / rect.width().max(1.0)).clamp(0.2, 0.8);
    }
    ui.painter().rect_filled(divider, 0, Tokens::get(ui.ctx()).chrome);
    response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let left = Rect::from_min_max(rect.min, egui::pos2(x - 3.0, rect.bottom()));
    let right = Rect::from_min_max(egui::pos2(x + 3.0, rect.top()), rect.max);
    let (three, two) = if app.ui.uv_workbench.swap_views { (right, left) } else { (left, right) };
    let mut model_ui = ui.new_child(egui::UiBuilder::new().id_salt("uv-3d-cell").max_rect(three));
    model_ui.set_clip_rect(three);
    uv_ui::main_view(app, &mut model_ui);
    let mut texture_ui = ui.new_child(egui::UiBuilder::new().id_salt("uv-2d-cell").max_rect(two));
    texture_ui.set_clip_rect(two);
    crate::canvas::uv_documents(app, &mut texture_ui);
    true
}
pub(crate) fn floating(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.workspace != "3D" {
        return;
    }
    let mut settings = app.ui.uv_workbench.import_settings;
    let mut import = false;
    egui::Window::new(tl!("3D import settings")).open(&mut settings).collapsible(false).resizable(false).show(ctx, |ui| {
        ui.label("FBX Binary 7.4+ / OBJ");
        ui.horizontal(|ui| {
            ui.label(tl!("UV set:"));
            ui.add(egui::DragValue::new(&mut app.ui.uv_workbench.uv_set).range(0..=31));
        });
        ui.weak(tl!("Materials, color textures and normal maps are loaded automatically."));
        if ui.button(tl!("Choose model…")).clicked() {
            import = true;
        }
    });
    app.ui.uv_workbench.import_settings = settings;
    if import && let Err(e) = uv_ui::pick_file(app, "model", 0, false) {
        app.ui.uv_workbench.status = e;
    }
    if app.session.uv.scene.is_none() {
        return;
    }
    if app.ui.uv_workbench.floating_3d {
        let mut open = true;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("uv-model-window"),
            egui::ViewportBuilder::default().with_title("PhotoCraft — 3D").with_inner_size([800.0, 600.0]),
            |ui, class| {
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                if class == egui::ViewportClass::EmbeddedWindow {
                    egui::Window::new(tl!("3D · Model")).open(&mut open).default_size(vec2(800.0, 600.0)).show(ui.ctx(), |ui| uv_ui::main_view(app, ui));
                } else {
                    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Tokens::get(ui.ctx()).canvas)).show(ui, |ui| {
                        bar(app, ui);
                        uv_ui::main_view(app, ui);
                        uv_ui::keys(app, ui.ctx());
                        crate::paint_mouse::show_picker(app, ui.ctx());
                        crate::brush_mode_menu::show(app, ui.ctx());
                    });
                }
            },
        );
        app.ui.uv_workbench.floating_3d &= open;
        if !open && app.ui.uv_workbench.picker_3d {
            crate::brush_picker::close(&mut app.ui);
        }
    }
    if app.ui.uv_workbench.floating_2d {
        let mut open = true;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("uv-texture-window"),
            egui::ViewportBuilder::default().with_title("PhotoCraft — 2D").with_inner_size([800.0, 600.0]),
            |ui, class| {
                if ui.ctx().input(|i| i.viewport().close_requested()) {
                    open = false;
                }
                if class == egui::ViewportClass::EmbeddedWindow {
                    egui::Window::new(tl!("2D · Texture"))
                        .open(&mut open)
                        .default_size(vec2(800.0, 600.0))
                        .show(ui.ctx(), |ui| crate::canvas::uv_documents(app, ui));
                } else {
                    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Tokens::get(ui.ctx()).canvas)).show(ui, |ui| {
                        crate::canvas::uv_documents(app, ui);
                        crate::paint_mouse::show_picker(app, ui.ctx());
                        crate::brush_mode_menu::show(app, ui.ctx());
                    });
                }
            },
        );
        app.ui.uv_workbench.floating_2d &= open;
        if !open && app.ui.uv_workbench.picker_viewport == Some(egui::ViewportId::from_hash_of("uv-texture-window")) {
            crate::brush_picker::close(&mut app.ui);
        }
    }
}

/// Switch document banks after ending native gestures, preserving each canvas transform.
pub(crate) fn sync_workspace(app: &mut PhotocraftApp) -> Result<(), String> {
    let active = app.ui.workspace == "3D";
    if app.session.uv_workspace.active == active {
        return Ok(());
    }
    // Commit a valid transient edit before changing banks. app.run handles stale revisions
    // without dropping the preview, and recognizes this exact command to avoid a second commit.
    if let Some(commit) = crate::uv_mask_paint_preview::pending(app) {
        app.run("uv.maskPaint.modify", commit.params()).inspect_err(|error| {
            app.ui.status = error.clone();
            app.ui.status_error = true;
        })?;
    }
    if !active {
        // A built-in preset is free to replace its own panels and dock. Remember the UV panel's
        // native layout first so returning to 3D restores its tabs, floating returns and visibility.
        app.uv_cache.parked_dock = Some(crate::uv_ui::ParkedDock {
            dock: app.ui.dock.clone(),
            docking: app.ui.docking.clone(),
            returns: app.ui.docking_returns.clone(),
            kept: app.ui.docking_kept.clone(),
            collapsed: app.ui.docking_collapsed.clone(),
            tabs: app.ui.dock_tabs,
            panels: app.ui.panels.clone(),
            dock_visible: app.ui.panels.dock,
            uv_open: app.ui.uv_workbench.open,
        });
    }
    uv_ui::finish_paint(app);
    app.ui.brush_picker = None;
    app.ui.uv_workbench.picker_3d = false;
    if let Some([x, y, _]) = app.drag.as_ref().and_then(|d| d.points.last()).copied() {
        crate::canvas::tool_event(app, crate::canvas::ToolEvent::Up { x, y }, egui::Modifiers::NONE);
    }
    if app.ui.transform.is_some() {
        crate::transform_tool::commit(app);
    }
    if !active {
        app.uv_cache.project.parked_view = Some(crate::uv_project::snapshot(app));
    }
    for (st, view) in app.session.documents().iter().zip(&app.ui.views) {
        app.uv_cache.workspace_views.insert(st.doc.id, view.clone());
    }
    app.run("uv.workspace.activate", serde_json::json!({"active":active})).inspect_err(|error| {
        app.ui.status = error.clone();
        app.ui.status_error = true;
    })?;
    app.sync_views();
    for (st, view) in app.session.documents().iter().zip(&mut app.ui.views) {
        if let Some(saved) = app.uv_cache.workspace_views.get(&st.doc.id) {
            *view = saved.clone();
        }
    }
    app.uv_cache.area = None;
    app.uv_cache.viewport = None;
    app.uv_cache.viewport_id = None;
    app.uv_cache.workspace_views.retain(|id, _| app.session.documents().iter().chain(app.session.uv_workspace.parked()).any(|st| st.doc.id == *id));
    Ok(())
}

/// Restore the UV workspace's native dock after a built-in preset has applied its own layout.
/// This is called only while re-entering 3D; an explicit Reset Workspace continues to use the
/// preset defaults.
pub(crate) fn restore_parked_dock(app: &mut PhotocraftApp) {
    let Some(parked) = app.uv_cache.parked_dock.take() else { return };
    app.ui.dock = parked.dock;
    app.ui.docking = parked.docking;
    app.ui.docking_returns = parked.returns;
    app.ui.docking_kept = parked.kept;
    app.ui.docking_collapsed = parked.collapsed;
    app.ui.dock_tabs = parked.tabs;
    app.ui.panels = parked.panels;
    app.ui.panels.dock = parked.dock_visible;
    app.ui.uv_workbench.open = parked.uv_open;
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable as _;
    use photocraft_color::PixelFormat;
    use photocraft_doc::{
        LayerMask,
        mask_paint::{EffectKind, Paint, Stack},
    };
    use photocraft_raster::Surface;
    use serde_json::json;
    #[test]
    fn uv_tabs_are_scoped_to_the_3d_workspace() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let tabs = |app: &PhotocraftApp| crate::dock::visible_tabs(app, crate::dock::Group::Properties, false);
        assert_eq!(tabs(&app), [(0, "Properties"), (1, "Adjustments")]);
        app.ui.workspace = "3D".into();
        assert_eq!(tabs(&app), [(0, "Properties"), (1, "Adjustments"), (2, "3D / UV"), (3, "Images")]);
        app.ui.workspace = "Painting".into();
        assert_eq!(tabs(&app), [(0, "Properties"), (1, "Adjustments")]);
    }
    #[test]
    fn uv_shell_options_are_localized_and_hidden_outside_3d() {
        let _gpu = photocraft_testkit::gpu_lock();
        for (language, label, hint) in [
            ("en", "UV shell selection", "Click to select · Shift to add · Ctrl to subtract"),
            ("ru", "Выделение UV-шеллов", "Щелчок — выбрать · Shift — добавить · Ctrl — вычесть"),
        ] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.session.uv.scene = Some(std::sync::Arc::new(photocraft_uv::Scene::default()));
            app.ui.workspace = "3D".into();
            app.ui.uv_workbench.select_shells = true;
            let app = std::rc::Rc::new(std::cell::RefCell::new(app));
            let state = app.clone();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 90.0)).build_ui(move |ui| {
                crate::i18n::with_language(crate::i18n::Lang::from_pref(language), || {
                    crate::panels::options_bar(&mut state.borrow_mut(), ui);
                });
            });
            PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run_steps(2);
            assert!(h.query_by_label(label).is_some(), "{language}: shell settings");
            assert!(h.query_by_label(hint).is_some(), "{language}: selection modifiers");
            app.borrow_mut().ui.workspace = "Essentials".into();
            h.run_steps(2);
            assert!(h.query_by_label(label).is_none(), "{language}: ordinary tool options return");
            assert!(h.query_by_label(hint).is_none(), "{language}: shell instructions stay in 3D");
        }
    }
    #[test]
    fn uv_controls_follow_english_and_russian_interface_languages() {
        let _gpu = photocraft_testkit::gpu_lock();
        for (language, views, projection, shell) in
            [("en", "3D / 2D views", "Camera projection", "UV shell selection"), ("ru", "Виды 3D / 2D", "Проекция камеры", "Выделение UV-шеллов")]
        {
            let mut session = photocraft_engine::Session::new();
            session.prefs.edit(|p| p.interface.language = language.into());
            let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut app = PhotocraftApp::new(session, Default::default());
                app.ui.workspace = "3D".into();
                crate::menus::apply_workspace(&mut app);
                app
            });
            h.step();
            h.step();
            assert!(h.query_by_label(views).is_some(), "{language}: view menu");
            assert!(h.query_by_label(projection).is_some(), "{language}: camera menu");
            assert!(h.query_by_label(shell).is_some(), "{language}: shell tool");
            h.get_by_label(views).click();
            h.step();
            if let Ok(folder) = std::env::var("PHOTOCRAFT_UV_I18N_SCREENSHOTS") {
                h.render().unwrap().save(std::path::Path::new(&folder).join(format!("uv-{language}.png"))).unwrap();
            }
        }
    }
    #[test]
    fn three_d_preset_and_file_commands_are_available_without_a_document() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.threeD", Value::Null).unwrap();
        assert_eq!(app.ui.workspace, "3D");
        assert!(crate::workspace_ui::switcher_options(&app).iter().any(|(key, _)| key == "3D"));
        assert!(app.ui.uv_workbench.open && app.ui.uv_workbench.view_3d);
        assert_eq!(app.ui.dock_tabs.properties, 2);
        let items = crate::menus::menu_items(&app);
        for id in ["file.uv.import", "file.uv.settings", "file.uv.openProject"] {
            let item = items.iter().find(|item| item.id == id).unwrap();
            assert!(item.enabled);
            assert_eq!(item.path, vec!["File", "3D Import"]);
        }
        assert_eq!(is_enabled(&app, "file.uv.reimport"), Some(false));
        crate::menus::invoke(&mut app, &ctx, "file.uv.settings", Value::Null).unwrap();
        assert!(app.ui.uv_workbench.import_settings);
        assert!(app.session.documents().is_empty());
        crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name":"3D"})).unwrap();
        app.ui.uv_workbench.perspective = true;
        app.ui.uv_workbench.split_view = true;
        crate::dock::persist(&mut app, &ctx);
        let mut session = photocraft_engine::Session::new();
        session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
        let mut restored = PhotocraftApp::new(session, Default::default());
        crate::dock::restore(&mut restored);
        assert_eq!(restored.ui.workspace, "3D");
        assert!(restored.ui.uv_workbench.open);
        assert!(restored.ui.uv_workbench.perspective && restored.ui.uv_workbench.split_view);
        assert!(!restored.ui.panels.color);
    }
    #[test]
    fn view_menu_shortcuts_shell_tool_and_floating_views_work_with_gpu() {
        let _gpu = photocraft_testkit::gpu_lock();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        let mut session = photocraft_engine::Session::new();
        session.execute("uv.model.import", json!({"path":source})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Default::default());
            app.set_uv_wgpu(cc.wgpu_render_state.clone().unwrap()).unwrap();
            app.ui.workspace = "3D".into();
            crate::menus::apply_workspace(&mut app);
            app
        });
        h.step();
        assert!(h.state().uv_gpu_active());
        h.get_by_label("3D / 2D views").click();
        h.run_steps(2);
        h.get_by_label("Show UV shells").click();
        h.step();
        assert!(!h.state().ui.uv_workbench.show_overlay);
        h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        h.step();
        let center = h.state().uv_cache.area.unwrap().center();
        let key = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>, key| {
            h.event(egui::Event::PointerMoved(center));
            h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
            h.step();
            h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE });
            h.step();
        };
        key(&mut h, egui::Key::F1);
        assert!(h.state().ui.uv_workbench.split_view);
        assert!(h.state().uv_cache.viewport.unwrap().width() < h.state().uv_cache.area.unwrap().width() * 0.7);
        let before = h.state().uv_cache.viewport.unwrap().center().x;
        key(&mut h, egui::Key::F4);
        assert!(h.state().uv_cache.viewport.unwrap().center().x > before);
        key(&mut h, egui::Key::F5);
        assert!(h.state().ui.uv_workbench.perspective);
        key(&mut h, egui::Key::F6);
        assert!(!h.state().ui.uv_workbench.perspective);
        h.get_by_label("UV shell selection").click();
        h.step();
        assert!(h.state().ui.uv_workbench.select_shells);
        key(&mut h, egui::Key::B);
        assert!(!h.state().ui.uv_workbench.select_shells);
        layout(h.state_mut(), 2);
        h.state_mut().ui.uv_workbench.floating_3d = true;
        h.step();
        h.step();
        assert!(h.state().uv_gpu_active());
        assert!(h.state().uv_cache.viewport.is_some());
        layout(h.state_mut(), 1);
        h.state_mut().ui.uv_workbench.floating_2d = true;
        h.step();
        h.step();
        assert!(h.state().uv_gpu_active());
        layout(h.state_mut(), 0);
        h.step();
        h.step();
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_WORKSPACE_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
    }

    #[test]
    fn stale_mask_paint_preview_rejects_workspace_switch_and_restores_ui() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        app.run("uv.model.import", json!({"path":model.to_string_lossy()})).unwrap();
        app.run("file.new", json!({"width":32,"height":24})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let state = app.session.active_mut().unwrap();
        let mut mask = LayerMask::hide_all();
        mask.paint = Some(Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.2]),
            layers: vec![Paint {
                name: "Opacity".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: Surface::with_default(PixelFormat::RGBA8, &[1.0, 0.0, 0.0, 0.5]),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            }],
            active: 0,
        });
        std::sync::Arc::make_mut(&mut state.doc).layer_mut(layer).unwrap().mask = Some(mask);

        crate::uv_mask_paint_preview::set(&mut app, layer, 0, 0.5);
        assert!(app.uv_mask_paint_preview.is_some());
        let document = app.session.active().unwrap().doc.id;
        let history = app.session.active().unwrap().history.past_len();
        crate::menus::invoke(&mut app, &egui::Context::default(), "window.workspace.threeD", Value::Null).unwrap();
        assert!(app.session.uv_workspace.active);
        assert!(app.uv_mask_paint_preview.is_none());
        let committed_history =
            app.session.documents().iter().chain(app.session.uv_workspace.parked()).find(|state| state.doc.id == document).unwrap().history.past_len();
        assert_eq!(committed_history, history + 1, "valid preview commits once before switching");
        crate::menus::invoke(&mut app, &egui::Context::default(), "window.workspace.essentials", Value::Null).unwrap();
        assert!(!app.session.uv_workspace.active);
        crate::menus::invoke(&mut app, &egui::Context::default(), "window.workspace.threeD", Value::Null).unwrap();
        assert!(app.uv_cache.parked_dock.is_none(), "the parked layout is restored on re-entry");

        // Returning to the UV bank makes a different document active. Build the second preview
        // against that bank's actual target instead of reusing the ordinary-document LayerId.
        app.run("uv.texture.create", json!({"width":32,"height":24,"depth":8})).unwrap();
        let uv_layer = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("uv.maskPaint.add", json!({})).unwrap();
        crate::uv_mask_paint_preview::set(&mut app, uv_layer, 0, 0.25);
        assert!(app.uv_mask_paint_preview.is_some());
        app.session.active_mut().unwrap().revision += 1;

        let result = crate::menus::invoke(&mut app, &egui::Context::default(), "window.workspace.essentials", Value::Null);
        assert!(result.is_err());
        assert_eq!(app.ui.workspace, "3D");
        assert!(app.session.uv_workspace.active);
        assert!(app.uv_cache.parked_dock.is_none(), "denied switch must not park a second layout");
        assert!(app.uv_mask_paint_preview.is_some(), "stale preview stays available for explicit cancellation");
        assert!(app.ui.status_error);
    }

    #[test]
    fn builtin_workspace_round_trip_preserves_uv_dock_but_reset_still_resets_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.threeD", Value::Null).unwrap();
        crate::menus::invoke(&mut app, &ctx, "window.panel.float", json!({"panel":"uvWorkbench","x":80,"y":90,"width":520,"height":360})).unwrap();
        assert!(app.ui.uv_workbench.open);
        let dock = app.ui.dock.clone();
        let docking = app.ui.docking.clone();
        let returns = app.ui.docking_returns.clone();
        let kept = app.ui.docking_kept.clone();
        let collapsed = app.ui.docking_collapsed.clone();
        let tabs = app.ui.dock_tabs;
        let dock_visible = app.ui.panels.dock;

        crate::menus::invoke(&mut app, &ctx, "window.workspace.painting", Value::Null).unwrap();
        assert!(!app.session.uv_workspace.active);
        assert_ne!(app.ui.docking, docking, "the Painting preset keeps its normal layout policy");
        crate::menus::invoke(&mut app, &ctx, "window.workspace.threeD", Value::Null).unwrap();

        assert!(app.session.uv_workspace.active);
        assert_eq!(app.ui.dock, dock);
        assert_eq!(app.ui.docking, docking);
        assert_eq!(app.ui.docking_returns, returns);
        assert_eq!(app.ui.docking_kept, kept);
        assert_eq!(app.ui.docking_collapsed, collapsed);
        assert_eq!(app.ui.dock_tabs, tabs);
        assert_eq!(app.ui.panels.dock, dock_visible);
        assert!(app.ui.uv_workbench.open);

        crate::menus::invoke(&mut app, &ctx, "window.uvWorkbench", Value::Null).unwrap();
        assert!(!app.ui.uv_workbench.open);
        let closed_dock = app.ui.docking.clone();
        let closed_returns = app.ui.docking_returns.clone();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.painting", Value::Null).unwrap();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.threeD", Value::Null).unwrap();
        assert_eq!(app.ui.docking, closed_dock);
        assert_eq!(app.ui.docking_returns, closed_returns);
        assert!(!app.ui.uv_workbench.open, "a closed UV panel stays closed when returning to 3D");

        crate::menus::invoke(&mut app, &ctx, "window.workspace.resetWorkspace", Value::Null).unwrap();
        assert!(app.ui.docking.is_none(), "resetting the active 3D workspace still applies the built-in default");
        assert!(app.ui.uv_workbench.open, "resetting 3D restores its default UV panel visibility");
    }
}
