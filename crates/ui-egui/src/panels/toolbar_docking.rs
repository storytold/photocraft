//! Whole-palette Tools gestures; individual tool buttons remain ordinary controls.
use crate::PhotocraftApp;
use egui::{Pos2, Rect, Sense, vec2};
use serde_json::{Value, json};

#[derive(Clone)]
struct Drag {
    origin: Pos2,
    press: Pos2,
    generation: u64,
}
fn key(part: &str) -> egui::Id {
    egui::Id::unique("tools-palette").with(part)
}
pub(crate) fn command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Result<Value, String> {
    if app.session.prefs().workspace_locked {
        return Err("workspace is locked".into());
    }
    let position = match id {
        "window.tools.float" => {
            let coordinate = |key, default| match params.get(key) {
                None => Ok(default),
                Some(value) => {
                    value.as_f64().filter(|v| v.is_finite() && v.abs() <= 1_000_000.0).map(|v| v as f32).ok_or_else(|| format!("{key} must be finite"))
                }
            };
            Some([coordinate("x", 80.0)?, coordinate("y", 120.0)?])
        }
        "window.tools.dock" => None,
        _ => return Err("unknown Tools operation".into()),
    };
    let mut size = app.ui.panel_presentation.tools_size;
    if id == "window.tools.float" && (params.get("width").is_some() || params.get("height").is_some()) {
        let fallback = size.unwrap_or([crate::panel_docking::COMPACT_COLUMN_WIDTH + if app.ui.panels.toolbar_double { 32.0 } else { 0.0 }, 600.0]);
        let number = |key, default| match params.get(key) {
            None => Ok(default),
            Some(v) => v.as_f64().filter(|v| v.is_finite()).map(|v| v as f32).ok_or_else(|| format!("{key} must be finite")),
        };
        let width = number("width", fallback[0])?;
        let height = number("height", fallback[1])?;
        if !(crate::panel_docking::COMPACT_COLUMN_WIDTH..=240.0).contains(&width) || !(160.0..=100_000.0).contains(&height) {
            return Err("Tools size must be 42..240 by 160..100000".into());
        }
        size = Some([width, height]);
    }
    app.ui.toolbar_floating = position;
    app.ui.panel_presentation.tools_size = size;
    app.ui.panels.toolbar = true;
    Ok(json!({"toolbarFloating":position,"toolbarSize":size}))
}
pub(crate) fn header_drag(app: &PhotocraftApp, ui: &egui::Ui, response: &egui::Response) {
    if !response.dragged() || app.session.prefs().workspace_locked || ui.data(|d| d.get_temp::<bool>(key("suppressed"))).unwrap_or(false) {
        return;
    }
    let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(response.rect.center());
    ui.data_mut(|d| {
        if d.get_temp::<Drag>(key("drag")).is_none() {
            let origin = d.get_temp::<Pos2>(key("origin")).unwrap_or(response.rect.min);
            d.insert_temp(key("drag"), Drag { origin, press, generation: app.ui.docking_generation });
        }
    });
    ui.ctx().request_repaint();
}
pub(crate) fn grip(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
    let response = ui.interact(rect, key("grip"), Sense::drag());
    let response = response.on_hover_cursor(if app.session.prefs().workspace_locked { egui::CursorIcon::Default } else { egui::CursorIcon::Grab });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Move Tools")));
    let tint = if response.hovered() || response.dragged() { t.text_dim.gamma_multiply(0.5) } else { t.text_faint.gamma_multiply(0.3) };
    let half = crate::panel_docking::compact_grip_width(rect.width()) * 0.5;
    for y in [-1.5, 1.5] {
        ui.painter().line_segment([rect.center() + vec2(-half, y), rect.center() + vec2(half, y)], egui::Stroke::new(1.0, tint));
    }
    header_drag(app, ui, &response);
    response.context_menu(|ui| {
        let (id, label) = if app.ui.toolbar_floating.is_some() { ("window.tools.dock", tl!("Dock Tools")) } else { ("window.tools.float", tl!("Float Tools")) };
        if ui.add_enabled(!app.session.prefs().workspace_locked, egui::Button::new(label)).clicked() {
            if let Err(error) = app.run(id, json!({})) {
                app.ui.status = error;
            }
            ui.close();
        }
    });
}
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, width: f32, margin: i8, contents: impl FnOnce(&mut PhotocraftApp, &mut egui::Ui)) {
    let ctx = ui.ctx().clone();
    let t = crate::theme::Tokens::get(&ctx);
    let bounds = ui.available_rect_before_wrap();
    let mut drag = ctx.data_mut(|d| d.get_temp::<Drag>(key("drag")));
    let cancelled = drag.as_ref().is_some_and(|d| d.generation != app.ui.docking_generation)
        || app.session.prefs().workspace_locked
        || crate::panel_docking::modal_open(app, &ctx)
        || ctx.input(|i| !i.focused || i.key_pressed(egui::Key::Escape) || (!i.pointer.primary_down() && !i.pointer.primary_released()));
    if cancelled {
        let down = ctx.input(|i| i.pointer.primary_down());
        ctx.data_mut(|d| {
            d.remove::<Drag>(key("drag"));
            if drag.is_some() && down {
                d.insert_temp(key("suppressed"), true);
            }
        });
        drag = None;
    }
    if !ctx.input(|i| i.pointer.primary_down()) {
        ctx.data_mut(|d| d.remove::<bool>(key("suppressed")));
    }
    let dock_x = if crate::direction::is_rtl() { bounds.right() } else { bounds.left() };
    let pointer = ctx.pointer_interact_pos().unwrap_or(bounds.min);
    let docking =
        !ctx.input(|i| i.modifiers.ctrl || i.modifiers.command) && drag.is_some() && (pointer.x - dock_x).abs() <= 18.0 && bounds.y_range().contains(pointer.y);
    let position = drag.as_ref().map(|d| d.origin + (pointer - d.press)).or_else(|| app.ui.toolbar_floating.map(|p| egui::pos2(p[0], p[1])));
    let frame = egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(margin, 3));
    if let Some(position) = position {
        if app.ui.toolbar_floating.is_none() {
            crate::direction::start_panel("toolbar")
                .exact_size(width)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::NONE.fill(t.chrome))
                .show(ui, |_| {});
        }
        let size = app.ui.panel_presentation.tools_size.unwrap_or([width, (bounds.bottom() - position.y).clamp(160.0, bounds.height().max(160.0))]);
        let width = size[0].max(width).min(240.0).min(bounds.width().max(width));
        let height = size[1].clamp(160.0, bounds.height().max(160.0));
        let position = egui::pos2(
            position.x.clamp(ctx.content_rect().left(), (ctx.content_rect().right() - width).max(ctx.content_rect().left())),
            position.y.clamp(ctx.content_rect().top(), (ctx.content_rect().bottom() - height).max(ctx.content_rect().top())),
        );
        ctx.data_mut(|d| d.insert_temp(key("origin"), position));
        let mut chrome = craft_ui::docking::chrome::ChromeStyle::from_ui(ui);
        chrome.background = t.chrome;
        chrome.stroke = egui::Stroke::new(ctx.pixels_per_point().recip(), t.card_border);
        let rect = Rect::from_min_size(position, vec2(width, height));
        let blocked = crate::panel_docking::modal_open(app, &ctx);
        let response = craft_ui::docking::chrome::FloatingFrame::new(key("floating"), rect)
            .bounds(bounds)
            .limits(craft_ui::docking::PanelLimits {
                min: vec2(crate::panel_docking::COMPACT_COLUMN_WIDTH + if app.ui.panels.toolbar_double { 32.0 } else { 0.0 }, 160.0),
                max: vec2(240.0, bounds.height().max(160.0)),
            })
            .reserve_top_resize_band(false)
            .resizable(!app.session.prefs().workspace_locked && !blocked)
            .opacity(if drag.is_some() { 0.85 } else { 1.0 })
            .show(&ctx, &chrome, |ui, body| {
                if blocked {
                    ui.disable();
                }
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
                child.set_clip_rect(body.intersect(ui.clip_rect()));
                frame.show(&mut child, |ui| {
                    ui.set_width((width - 2.0 * f32::from(margin)).max(20.0));
                    ui.set_height((height - 6.0).max(100.0));
                    contents(app, ui);
                });
                crate::panel_visibility::chrome(ui.ctx(), "tools", rect);
            });
        if let Some(resized) = response.resized
            && let Err(error) = app.run("window.tools.float", json!({"x":resized.left(),"y":resized.top(),"width":resized.width(),"height":resized.height()}))
        {
            app.ui.status = error;
        }
        if let Some(drag) = drag {
            if docking {
                let rect = Rect::from_min_max(egui::pos2(dock_x - 2.0, bounds.top()), egui::pos2(dock_x + 2.0, bounds.bottom()));
                ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, key("preview"))).rect_filled(rect, 0.0, t.accent);
            }
            if ctx.input(|i| i.pointer.primary_released()) {
                if let Err(error) = app.run(if docking { "window.tools.dock" } else { "window.tools.float" }, json!({"x":position.x,"y":position.y})) {
                    app.ui.status = error;
                }
                ctx.data_mut(|d| d.remove::<Drag>(key("drag")));
                ctx.request_repaint();
            } else if drag.generation == app.ui.docking_generation {
                ctx.request_repaint();
            }
        }
    } else {
        let frame_nr = ctx.cumulative_frame_nr();
        ctx.data_mut(|d| d.insert_temp(key("docked-frame"), frame_nr));
        let result = crate::direction::start_panel("toolbar").resizable(false).show_separator_line(false).exact_size(width).frame(frame).show(ui, |ui| {
            ctx.data_mut(|d| d.insert_temp(key("origin"), egui::pos2(ui.max_rect().left() - f32::from(margin), bounds.top())));
            contents(app, ui);
        });
        crate::panel_visibility::chrome(&ctx, "tools", result.response.rect);
    }
    crate::dock::persist(app, &ctx);
}

pub(crate) fn docked_this_frame(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<u64>(key("docked-frame"))) == Some(ctx.cumulative_frame_nr())
}
