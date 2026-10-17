//! Shape fill and stroke use the same independent paint picker and preset libraries.

use egui::{Color32, Sense, Stroke, vec2};
use photocraft_doc::{Color, Fill, GradientStyle};
use serde_json::{Map, Value};

use crate::{PhotocraftApp, theme::Tokens};

fn gradient(app: &PhotocraftApp, preset: &photocraft_engine::presets::gradients::GradientPreset) -> Fill {
    let (stops, opacity_stops) = preset.fill_stops(app.session.tools.foreground, app.session.tools.background);
    let mut fill = Fill::gradient(stops, 90.0, 1.0, GradientStyle::Linear, false);
    if let Fill::Gradient { opacity_stops: opacity, .. } = &mut fill {
        *opacity = opacity_stops;
    }
    fill
}

fn pattern(app: &PhotocraftApp, id: &str) -> Option<Fill> {
    let p = app.session.patterns.items.iter().find(|p| p.id == id)?;
    Some(Fill::Pattern { name: p.name.clone(), id: p.id.clone(), scale: 1.0, angle: 0.0, link: true, phase: (0.0, 0.0) })
}

fn paint(app: &PhotocraftApp, ui: &egui::Ui, rect: egui::Rect, fill: Option<&Fill>) {
    let t = Tokens::get(ui.ctx());
    match fill {
        Some(Fill::Solid(color)) => {
            if color.alpha < 1.0 {
                crate::widgets::checker(ui.painter(), rect, 4.0);
            }
            let [r, g, b, a] = color.to_rgba8();
            ui.painter().rect_filled(rect, t.radius_sm, Color32::from_rgba_unmultiplied(r, g, b, a));
        }
        Some(fill @ Fill::Gradient { reverse, .. }) => {
            if let Some(ramp) = photocraft_compose::gradient_fill::Ramp::new(fill) {
                let stops: Vec<_> = (0..=32)
                    .map(|i| {
                        let t = i as f32 / 32.0;
                        (t, ramp.sample(if *reverse { 1.0 - t } else { t }))
                    })
                    .collect();
                crate::preset_panels::paint_gradient(ui, rect, &stops);
            }
        }
        Some(Fill::Pattern { id, name, .. }) => {
            crate::widgets::checker(ui.painter(), rect, 4.0);
            let pattern = app
                .session
                .active()
                .and_then(|st| photocraft_doc::pattern::find(&st.doc.patterns, id, name))
                .or_else(|| photocraft_doc::pattern::find(&app.session.patterns.items, id, name));
            if let Some(pattern) = pattern {
                let texture = crate::preset_panels::pattern_texture(ui.ctx(), pattern);
                ui.painter().image(texture.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        None => {
            ui.painter().rect_filled(rect, t.radius_sm, t.field);
            ui.painter().line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.5, t.danger));
        }
    }
}

fn context(app: &PhotocraftApp) -> egui::Id {
    egui::Id::new((app.session.active().map(|st| (st.doc.id, st.revision, st.active_layer)), app.ui.tool, app.ui.mask_target, app.ui.vector_mask_target))
}

fn paint_target(app: &PhotocraftApp) -> egui::Id {
    egui::Id::new((app.session.active().map(|st| (st.doc.id, st.active_layer)), app.ui.tool, app.ui.mask_target, app.ui.vector_mask_target))
}

#[derive(Clone)]
struct PaintPopup {
    owner: egui::Id,
    target: egui::Id,
    last_seen: u64,
    left: f32,
    child: Option<egui::Id>,
}

fn popup_id() -> egui::Id {
    egui::Id::new("shape-paint-popup")
}

pub(crate) fn paint_popup_open(ctx: &egui::Context) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<PaintPopup>(popup_id()).is_some() || d.get_temp::<u64>(popup_id().with("frame")) == Some(frame))
}

pub(crate) fn close_popup(ctx: &egui::Context) {
    let popup = ctx.data_mut(|d| {
        let popup = d.get_temp::<PaintPopup>(popup_id());
        d.remove::<PaintPopup>(popup_id());
        popup
    });
    if let Some(popup) = popup
        && let Some(child) = popup.child
    {
        egui::Popup::close_id(ctx, child);
    }
}

fn recent_id() -> egui::Id {
    egui::Id::new("shape-paint-recent-colors")
}

fn remember(ctx: &egui::Context, color: Color) {
    ctx.data_mut(|d| {
        let mut colors = d.get_temp::<Vec<Color>>(recent_id()).unwrap_or_default();
        colors.retain(|c| *c != color);
        colors.insert(0, color);
        colors.truncate(12);
        d.insert_temp(recent_id(), colors);
    });
}

fn color_button(ui: &mut egui::Ui, color: Color, label: &str, selected: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    let inner = rect.shrink(2.0);
    if color.alpha < 1.0 {
        crate::widgets::checker(ui.painter(), inner, 4.0);
    }
    let [r, g, b, a] = color.to_rgba8();
    ui.painter().rect_filled(inner, t.radius_sm, Color32::from_rgba_unmultiplied(r, g, b, a));
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, if selected { t.accent } else { t.field_border }), egui::StrokeKind::Inside);
    response.on_hover_text(label)
}

fn palette(app: &PhotocraftApp, ui: &mut egui::Ui, current: Color) -> Option<Fill> {
    let mut chosen = None;
    let recent = ui.data(|d| d.get_temp::<Vec<Color>>(recent_id()).unwrap_or_default());
    ui.label(tl!("Recent Colors"));
    if recent.is_empty() {
        ui.label(egui::RichText::new(tl!("No recent colors")).color(Tokens::get(ui.ctx()).text_dim));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for color in recent {
                let label = crate::color_picker_ui::hex(color.to_rgb());
                if color_button(ui, color, &label, color == current).clicked() {
                    chosen = Some(Fill::Solid(color));
                }
            }
        });
    }
    ui.separator();
    let conv = photocraft_engine::presets::swatches::SwatchConv::new(&app.session);
    egui::ScrollArea::vertical().id_salt("shape-color-library").max_height(220.0).show(ui, |ui| {
        for (i, group) in app.session.presets.swatches.iter().enumerate() {
            egui::CollapsingHeader::new(&group.name).id_salt(("shape-colors", i)).default_open(i == 0).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for swatch in &group.items {
                        let [r, g, b] = conv.rgb(&swatch.color);
                        let color = Color::rgb(r, g, b);
                        if color_button(ui, color, &swatch.name, color == current).clicked() {
                            chosen = Some(Fill::Solid(color));
                        }
                    }
                });
            });
        }
    });
    chosen
}

#[derive(Clone)]
struct ColorDialog {
    owner: egui::Id,
    context: egui::Id,
    last_seen: u64,
    original: Option<Fill>,
    color: Color,
    stop: Option<usize>,
    fields: Map<String, Value>,
    alpha: f32,
}

fn dialog_id() -> egui::Id {
    egui::Id::new("shape-paint-color-dialog")
}

pub(crate) fn color_dialog_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<ColorDialog>(dialog_id())).is_some()
}

/// Runs even when another tool or the screen sampler temporarily hides the shape controls.
pub(crate) fn prune(app: &PhotocraftApp, ctx: &egui::Context) {
    if ctx
        .data(|d| d.get_temp::<PaintPopup>(popup_id()))
        .is_some_and(|p| p.target != paint_target(app) || ctx.cumulative_frame_nr() > p.last_seen.saturating_add(2))
    {
        close_popup(ctx);
    }
    let Some(mut dialog) = ctx.data(|d| d.get_temp::<ColorDialog>(dialog_id())) else { return };
    let frame = ctx.cumulative_frame_nr();
    // The screen sampler temporarily replaces the entire app UI, including the owner swatch.
    if crate::screen_picker::busy(ctx) {
        dialog.last_seen = frame;
        ctx.data_mut(|d| d.insert_temp(dialog_id(), dialog.clone()));
    }
    if dialog.context != context(app) || frame > dialog.last_seen.saturating_add(2) {
        ctx.data_mut(|d| d.remove::<ColorDialog>(dialog_id()));
    }
}

fn open_color(app: &PhotocraftApp, ui: &egui::Ui, owner: egui::Id, fill: Option<&Fill>, fallback: [f32; 4], stop: Option<usize>) {
    let color = match (fill, stop) {
        (Some(Fill::Solid(c)), _) => *c,
        (Some(Fill::Gradient { stops, .. }), Some(index)) => {
            let mut stops = stops.clone();
            stops.sort_by(|a, b| a.0.total_cmp(&b.0));
            let Some((_, color)) = stops.get(index) else { return };
            *color
        }
        _ => Color::rgba(fallback[0], fallback[1], fallback[2], fallback[3]),
    };
    let dialog = ColorDialog {
        owner,
        context: context(app),
        last_seen: ui.ctx().cumulative_frame_nr(),
        original: fill.cloned(),
        color,
        stop,
        fields: crate::color_picker_ui::initial_fields("shape", "Color Picker", color.to_rgb()),
        alpha: color.alpha * 100.0,
    };
    ui.data_mut(|d| d.insert_temp(dialog_id(), dialog));
    close_popup(ui.ctx());
    egui::Popup::close_all(ui.ctx());
}

fn color_window(app: &PhotocraftApp, ui: &egui::Ui, owner: egui::Id, fill: Option<&Fill>) -> Option<Fill> {
    let mut dialog = ui.data(|d| d.get_temp::<ColorDialog>(dialog_id()))?;
    if dialog.context != context(app) || (dialog.owner == owner && dialog.original.as_ref() != fill) {
        ui.data_mut(|d| d.remove::<ColorDialog>(dialog_id()));
        return None;
    }
    if dialog.owner != owner {
        return None;
    }
    dialog.last_seen = ui.ctx().cumulative_frame_nr();
    let bounds = ui.ctx().content_rect();
    let mut result = None;
    let modal = egui::Modal::new(owner.with("full-color-picker")).show(ui.ctx(), |ui| {
        ui.set_width((bounds.width() - 48.0).clamp(100.0, 650.0));
        // Area remembers its last size; allow the scroll viewport to grow to fit the new content.
        ui.set_max_height((bounds.height() - 48.0).max(100.0));
        ui.heading(tl!("Color Picker"));
        egui::ScrollArea::both().max_width((bounds.width() - 48.0).max(100.0)).max_height((bounds.height() - 90.0).max(100.0)).show(ui, |ui| {
            result = crate::color_picker_ui::body_with_swatches(ui, &mut dialog.fields, false);
            ui.horizontal(|ui| {
                ui.label(tl!("Opacity"));
                let response = crate::widgets::value_field(ui, &mut dialog.alpha, 0.0..=100.0, "%", 80.0);
                response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(dialog.alpha), tl!("Color opacity")));
            });
        });
    });
    if modal.should_close() || result == Some(false) {
        ui.data_mut(|d| d.remove::<ColorDialog>(dialog_id()));
        return None;
    }
    if result == Some(true) {
        ui.data_mut(|d| d.remove::<ColorDialog>(dialog_id()));
        let mut color = dialog.color;
        if dialog.fields.get("color") != dialog.fields.get("__orig") {
            let [r, g, b] = crate::color_picker_ui::current(&dialog.fields).rgb;
            color = Color::rgba(r, g, b, color.alpha);
        }
        if dialog.alpha != dialog.color.alpha * 100.0 {
            color.alpha = dialog.alpha / 100.0;
        }
        // A no-op must preserve the full original colour model and float precision.
        if color == dialog.color && (dialog.stop.is_some() || matches!(dialog.original, Some(Fill::Solid(_)))) {
            return None;
        }
        let next = if let Some(index) = dialog.stop {
            let mut f = dialog.original.clone()?;
            let Fill::Gradient { stops, .. } = &mut f else { return None };
            let mut order: Vec<_> = stops.iter().enumerate().map(|(i, (at, _))| (i, *at)).collect();
            order.sort_by(|a, b| a.1.total_cmp(&b.1));
            let (slot, _) = order.get(index)?;
            stops.get_mut(*slot)?.1 = color;
            f
        } else {
            Fill::Solid(color)
        };
        remember(ui.ctx(), color);
        return Some(next);
    }
    ui.data_mut(|d| d.insert_temp(dialog_id(), dialog));
    None
}

fn mode_button(ui: &mut egui::Ui, icon: &str, selected: bool, label: &str) -> bool {
    let response = crate::icons::button(ui, icon, 28.0, selected, label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    response.clicked()
}

/// One picker action, with an explicit No Color value.
pub(crate) struct PaintEdit {
    pub paint: Option<Fill>,
    /// Type and preset buttons are actions, rather than a continuous colour/field drag.
    pub discrete: bool,
}

fn paint_swatch(app: &PhotocraftApp, ui: &egui::Ui, rect: egui::Rect, fill: Option<&Fill>) {
    let t = Tokens::get(ui.ctx());
    // Keep the checkerboard and gradient inside the allocated frame. An outside stroke can
    // lose its bottom edge to the toolbar clip, especially beside a bright transparent stop.
    paint(app, ui, rect.shrink(1.0), fill);
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
}

pub(crate) fn swatch(app: &PhotocraftApp, ui: &mut egui::Ui, fill: Option<&Fill>, fallback: [f32; 4], tip: &str) -> Option<PaintEdit> {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 18.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
    paint_swatch(app, ui, rect, fill);
    let response = response.on_hover_text(tip);
    if response.clicked() {
        let same = ui.data(|d| d.get_temp::<PaintPopup>(popup_id())).is_some_and(|p| p.owner == response.id);
        close_popup(ui.ctx());
        if !same {
            let bounds = ui.ctx().content_rect();
            let width = (bounds.width() - 32.0).clamp(160.0, 280.0);
            let frame = ui.ctx().cumulative_frame_nr();
            ui.data_mut(|d| {
                d.insert_temp(
                    popup_id(),
                    PaintPopup {
                        owner: response.id,
                        target: paint_target(app),
                        last_seen: frame,
                        left: response.rect.left().min(bounds.right() - width - 32.0).max(bounds.left()),
                        child: None,
                    },
                )
            });
        }
    }
    let owned = ui.data(|d| d.get_temp::<PaintPopup>(popup_id())).filter(|p| p.owner == response.id);
    if owned.is_some() {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.data_mut(|d| {
            if let Some(mut popup) = d.get_temp::<PaintPopup>(popup_id()) {
                popup.last_seen = frame;
                d.insert_temp(popup_id(), popup);
            }
            d.insert_temp(popup_id().with("frame"), frame);
        });
    }
    let screen_id = response.id.with("screen-color");
    let solid = |c: [f32; 4]| Fill::Solid(Color::rgba(c[0], c[1], c[2], c[3]));
    let alpha = if let Some(Fill::Solid(color)) = fill { color.alpha } else { fallback[3] };
    let mut out = crate::screen_picker::take(ui.ctx(), screen_id).map(|c| Some(solid([c[0], c[1], c[2], alpha])));
    let mut discrete = out.is_some();
    // The child ComboBox owns egui's memory popup slot. Keep this parent explicitly owned,
    // like Stroke Options, and consider both hit regions when dismissing it.
    let mut child = None;
    let mut child_shown = false;
    let mut child_clicked = false;
    let parent_popup = egui::Popup::from_response(&response)
        .open(owned.is_some())
        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
        .anchor(egui::Rect::from_x_y_ranges(
            owned.as_ref().map_or(response.rect.left(), |p| p.left)..=owned.as_ref().map_or(response.rect.left(), |p| p.left),
            response.rect.y_range(),
        ))
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[egui::RectAlign::TOP_START]);
    let popup = parent_popup.show(|ui| {
        ui.set_width((ui.ctx().content_rect().width() - 32.0).clamp(160.0, 280.0));
        // Changing paint mode must not retain the previous Area height as a scroll limit.
        ui.set_max_height((ui.ctx().content_rect().height() - 80.0).max(100.0));
        ui.horizontal(|ui| {
            if mode_button(ui, "ban", fill.is_none(), tl!("No Color")) {
                out = Some(None);
                discrete = true;
            }
            if mode_button(ui, "square", matches!(fill, Some(Fill::Solid(_))), tl!("Solid Color")) && !matches!(fill, Some(Fill::Solid(_))) {
                out = Some(Some(solid(fallback)));
                discrete = true;
            }
            if mode_button(ui, "blend", matches!(fill, Some(Fill::Gradient { .. })), tl!("Gradient")) && !matches!(fill, Some(Fill::Gradient { .. })) {
                out = Some(Some(gradient(app, &app.session.presets.gradient)));
                discrete = true;
            }
            ui.add_enabled_ui(!app.session.patterns.items.is_empty(), |ui| {
                if mode_button(ui, "grid-2x2", matches!(fill, Some(Fill::Pattern { .. })), tl!("Pattern")) && !matches!(fill, Some(Fill::Pattern { .. })) {
                    out = app.session.patterns.items.first().and_then(|p| pattern(app, &p.id)).map(Some);
                    discrete = true;
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if mode_button(ui, "palette", false, tl!("Color Picker")) {
                    let stop = matches!(fill, Some(Fill::Gradient { .. })).then_some(0);
                    open_color(app, ui, response.id, fill, fallback, stop);
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("paint-body").max_height((ui.ctx().content_rect().height() - 140.0).max(100.0)).show(ui, |ui| match fill {
            Some(f @ Fill::Gradient { .. }) => {
                ui.label(tl!("Gradient"));
                egui::ScrollArea::vertical().id_salt("shape-gradient-library").max_height(132.0).show(ui, |ui| {
                    for (i, group) in app.session.presets.gradients.iter().enumerate() {
                        egui::CollapsingHeader::new(&group.name).id_salt(("shape-gradients", i)).default_open(i == 0).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for preset in &group.items {
                                    let (r, response) = ui.allocate_exact_size(vec2(28.0, 22.0), Sense::click());
                                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &preset.name));
                                    paint(app, ui, r.shrink(2.0), Some(&gradient(app, preset)));
                                    ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
                                    if response.on_hover_text(&preset.name).clicked() {
                                        out = Some(Some(gradient(app, preset)));
                                        discrete = true;
                                    }
                                }
                            });
                        });
                    }
                });
                ui.add_space(4.0);
                let key = response.id.with(("gradient-stop-edit", context(app)));
                // Selection belongs to the swatch's document/layer, rather than its revision.
                // Numeric opacity edits must keep the selected handle across their own commits.
                let strip_key = response.id.with(("gradient-stop-strip", paint_target(app)));
                if let Some(p) = crate::gradient_ui::paint_stop_opacity(ui, strip_key, f)
                    && let Ok(next) = photocraft_engine::gradient_fill_cmds::apply_stop(f, &p, app.session.tools.foreground, app.session.tools.background)
                {
                    out = Some(Some(next));
                }
                let pending = ui.data(|d| d.get_temp::<Value>(key));
                match crate::gradient_ui::stop_strip(ui, strip_key, f, true, pending.as_ref(), (app.session.tools.foreground, app.session.tools.background)) {
                    Some(crate::gradient_ui::StopAction::Preview(p)) => {
                        ui.data_mut(|d| d.insert_temp(key, p));
                    }
                    Some(crate::gradient_ui::StopAction::Commit(p)) => {
                        ui.data_mut(|d| d.remove::<Value>(key));
                        if let Some(f) = fill
                            && let Ok(next) =
                                photocraft_engine::gradient_fill_cmds::apply_stop(f, &p, app.session.tools.foreground, app.session.tools.background)
                        {
                            crate::gradient_ui::retain_stop_selection(ui, strip_key, f, &p);
                            out = Some(Some(next));
                            discrete = true;
                        }
                    }
                    Some(crate::gradient_ui::StopAction::Color(index)) => open_color(app, ui, response.id, fill, fallback, Some(index)),
                    None => {}
                }
                let mut next = fill.cloned();
                if let Some(Fill::Gradient { angle, scale, style, reverse, dither, align, .. }) = &mut next {
                    let opts =
                        [GradientStyle::Linear, GradientStyle::Radial, GradientStyle::Angle, GradientStyle::Reflected, GradientStyle::Diamond].map(|s| {
                            (
                                s,
                                tl!(match s {
                                    GradientStyle::Linear => "Linear",
                                    GradientStyle::Radial => "Radial",
                                    GradientStyle::Angle => "Angle",
                                    GradientStyle::Reflected => "Reflected",
                                    GradientStyle::Diamond => "Diamond",
                                }),
                            )
                        });
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        let parent_layer = ui.layer_id();
                        let child_id = ui.make_persistent_id(egui::IdSalt::new("shape-gradient-style")).with("popup");
                        child = Some(child_id);
                        child_shown |= egui::Popup::is_id_open(ui.ctx(), child_id);
                        if crate::widgets::dropdown(ui, "shape-gradient-style", style, &opts, 130.0) {
                            changed = true;
                            discrete = true;
                        }
                        child_shown |= egui::Popup::is_id_open(ui.ctx(), child_id);
                        if let Some(response) = ui.ctx().read_response(child_id) {
                            ui.ctx().set_sublayer(parent_layer, response.layer_id);
                            child_clicked |= !response.clicked_elsewhere();
                        }
                        ui.label(tl!("Angle"));
                        changed |= crate::widgets::value_field(ui, angle, (-180.0_f32).min(*angle)..=180.0_f32.max(*angle), "°", 62.0).changed();
                    });
                    let mut percent = *scale * 100.0;
                    ui.horizontal(|ui| {
                        ui.label(tl!("Scale"));
                        let range = 1.0_f32.min(percent)..=10000.0_f32.max(percent);
                        if crate::widgets::value_field(ui, &mut percent, range, "%", 80.0).changed() {
                            *scale = percent / 100.0;
                            changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(reverse, tl!("Reverse")).changed();
                        changed |= ui.checkbox(dither, tl!("Dither")).changed();
                    });
                    changed |= ui.checkbox(align, tl!("Align with layer")).changed();
                    if changed {
                        out = Some(next);
                    }
                }
            }
            Some(Fill::Pattern { id, scale, angle, link, .. }) => {
                if let Some(id) = crate::preset_panels::pattern_grid(app, ui, id) {
                    out = pattern(app, &id).map(Some);
                    discrete = true;
                }
                let mut next = fill.cloned();
                let (mut percent, mut rotation, mut linked) = (*scale * 100.0, *angle, *link);
                let mut changed = false;
                let mut scale_changed = false;
                let mut angle_changed = false;
                ui.horizontal(|ui| {
                    ui.label(tl!("Scale"));
                    let range = 1.0_f32.min(percent)..=10000.0_f32.max(percent);
                    scale_changed = crate::widgets::value_field(ui, &mut percent, range, "%", 80.0).changed();
                    changed |= scale_changed;
                });
                ui.horizontal(|ui| {
                    ui.label(tl!("Angle"));
                    let range = (-180.0_f32).min(rotation)..=180.0_f32.max(rotation);
                    angle_changed = crate::widgets::value_field(ui, &mut rotation, range, "°", 80.0).changed();
                    changed |= angle_changed;
                });
                changed |= ui.checkbox(&mut linked, tl!("Link with Layer")).changed();
                if changed && let Some(Fill::Pattern { scale, angle, link, .. }) = &mut next {
                    if scale_changed {
                        *scale = percent / 100.0;
                    }
                    if angle_changed {
                        *angle = rotation;
                    }
                    *link = linked;
                    out = Some(next);
                }
            }
            Some(Fill::Solid(color)) => {
                if let Some(next) = palette(app, ui, *color) {
                    out = Some(Some(next));
                    discrete = true;
                }
                if let Some(rgb) = crate::screen_picker::button(ui, screen_id) {
                    out = Some(Some(solid([rgb[0], rgb[1], rgb[2], color.alpha])));
                    discrete = true;
                }
            }
            None => {}
        });
    });
    if owned.is_some() {
        ui.data_mut(|d| {
            if let Some(mut popup) = d.get_temp::<PaintPopup>(popup_id()) {
                popup.child = child;
                d.insert_temp(popup_id(), popup);
            }
        });
        let outside = popup.as_ref().is_some_and(|p| p.response.clicked_elsewhere()) && !child_clicked && !response.clicked();
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if outside || escape && !child_shown {
            close_popup(ui.ctx());
        }
    }
    if popup.is_none() {
        ui.data_mut(|d| d.remove::<Value>(response.id.with(("gradient-stop-edit", context(app)))));
    }
    if let Some(next) = color_window(app, ui, response.id, fill) {
        out = Some(Some(next));
        discrete = true;
    }
    let edit = out.filter(|paint| paint.as_ref() != fill).map(|paint| PaintEdit { paint, discrete });
    if edit.is_some() {
        ui.data_mut(|d| d.remove::<Value>(response.id.with(("gradient-stop-edit", context(app)))));
    }
    if let Some(PaintEdit { paint: Some(Fill::Solid(color)), .. }) = &edit {
        remember(ui.ctx(), *color);
    }
    edit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Tool;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use photocraft_doc::{LayerContent, LayerId, ShapeLayer};
    use serde_json::json;

    fn app(tool: Tool, depth: u32) -> (PhotocraftApp, LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100, "depth": depth})).unwrap();
        let id = LayerId(
            app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 60, 40], "fill": "#bb2288", "stroke": {"color": "#224488", "width": 4}})).unwrap()
                ["layer"]
                .as_u64()
                .unwrap(),
        );
        app.ui.tool = tool;
        (app, id)
    }

    fn shape(app: &PhotocraftApp, id: LayerId) -> &ShapeLayer {
        match &app.session.active().unwrap().doc.layer(id).unwrap().content {
            LayerContent::Shape(shape) => shape,
            _ => panic!("shape layer"),
        }
    }

    fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(vec2(1300.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                prune(app, ui.ctx());
                if ui.data(|d| d.get_temp::<bool>(egui::Id::new("test-hide-shape-options"))).unwrap_or(false) {
                    return;
                }
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    crate::i18n::with_language(crate::i18n::lang_from_tag("en").unwrap(), || {
                        ui.horizontal(|ui| crate::vector_ui::options_bar(app, ui, app.ui.tool));
                    });
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(4);
        h
    }

    fn open_picker(h: &mut Harness<'static, PhotocraftApp>) {
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Color Picker").click();
        h.run_steps(3);
        assert!(color_dialog_open(&h.ctx));
    }

    #[test]
    fn transparent_swatch_frame_fits_even_when_clipped_to_its_toolbar_slot() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let fill = Fill::gradient(vec![(0.0, Color::BLACK), (1.0, Color::rgba(0.0, 0.0, 0.0, 0.0))], 90.0, 1.0, GradientStyle::Linear, false);
        for theme in crate::theme::ThemeKind::ALL {
            for scale in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, theme);
                ctx.set_pixels_per_point(scale);
                let rect = egui::Rect::from_min_size(egui::pos2(20.25, 20.5), vec2(26.0, 18.0));
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_clip_rect(rect);
                    paint_swatch(&app, ui, rect, Some(&fill));
                });
                output.textures_delta.clear();
                let frame = output.shapes.iter().rev().find(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.width > 0.0)).unwrap();
                assert!(
                    frame.clip_rect.contains_rect(frame.shape.visual_bounding_rect()),
                    "{theme:?} at {scale}: the border must survive an exact toolbar clip"
                );
            }
        }
    }

    #[test]
    fn stroke_stop_opacity_keeps_selection_and_edits_only_that_paint_with_undo() {
        let (mut app, id) = app(Tool::Rectangle, 32);
        app.run(
            "shape.edit",
            json!({"layer": id.0, "stroke": {"fill": {"gradient": {"stops": [[0, "#000000"], [1, "#000000"]], "opacityStops": [[0, 1], [1, 0]]}}}}),
        )
        .unwrap();
        let before = shape(&app, id).clone();
        let globals = (app.session.tools.foreground, app.session.tools.background);
        let history = app.session.active().unwrap().history.past_len();
        let mut h = harness(app);
        h.get_by_label("Stroke color").click();
        h.run_steps(3);
        assert!(h.get_by_label("Stop opacity").accesskit_node().is_disabled());
        let rect = h.get_by_label("Gradient stops").rect();
        let pos = egui::pos2(rect.right() - 8.0, rect.top() + 6.0);
        h.event(egui::Event::PointerMoved(pos));
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        let opacity = h.get_by_label("Stop opacity");
        assert!(!opacity.accesskit_node().is_disabled());
        opacity.click();
        h.run_steps(2);
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(3);
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(3);
        assert!(!h.get_by_label("Stop opacity").accesskit_node().is_disabled(), "own commits must keep the selected opacity handle");
        let mut expected = before.clone();
        if let Fill::Gradient { opacity_stops, .. } = &mut expected.stroke.as_mut().unwrap().paint {
            opacity_stops[1].1 = 0.02;
        }
        assert_eq!(shape(h.state(), id).stroke, expected.stroke);
        assert_eq!(shape(h.state(), id).fill, before.fill);
        assert_eq!(shape(h.state(), id).path, before.path);
        assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), globals);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id), &before);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id).stroke, expected.stroke);
    }

    #[test]
    fn palette_picks_are_independent_actions_and_recent_colors_stay_local() {
        let (app, id) = app(Tool::Rectangle, 16);
        let before = shape(&app, id).clone();
        let globals = (app.session.tools.foreground, app.session.tools.background);
        let history = app.session.active().unwrap().history.past_len();
        let library = &app.session.presets.swatches[0].items;
        let first = library[0].clone();
        let second = library[1].clone();
        let conv = photocraft_engine::presets::swatches::SwatchConv::new(&app.session);
        let [r, g, b] = conv.rgb(&first.color);
        let fill = Fill::Solid(Color::rgb(r, g, b));
        let [r, g, b] = conv.rgb(&second.color);
        let stroke = Fill::Solid(Color::rgb(r, g, b));
        let mut h = harness(app);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label(&first.name).click();
        h.run_steps(3);
        assert_eq!(shape(h.state(), id).fill, Some(fill.clone()));
        assert_eq!(shape(h.state(), id).stroke, before.stroke);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Stroke color").click();
        h.run_steps(3);
        h.get_by_label(&second.name).click();
        h.run_steps(3);
        assert_eq!(shape(h.state(), id).stroke.as_ref().unwrap().paint, stroke);
        assert_eq!(shape(h.state(), id).fill, Some(fill));
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2);
        assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), globals);
        assert_eq!(h.ctx.data(|d| d.get_temp::<Vec<Color>>(recent_id()).unwrap()).len(), 2);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id), &before);
    }

    #[test]
    fn full_picker_cancel_noop_and_alpha_only_preserve_float_model_and_history() {
        let (mut app, id) = app(Tool::Rectangle, 32);
        let mut original = Color::gray(0.1234567);
        original.alpha = 0.73123455;
        app.run("shape.edit", json!({"layer": id.0, "fill": {"color": original}})).unwrap();
        let before = shape(&app, id).clone();
        let history = app.session.active().unwrap().history.past_len();
        let globals = (app.session.tools.foreground, app.session.tools.background);
        let mut h = harness(app);
        open_picker(&mut h);
        h.ctx.data_mut(|d| {
            let mut dialog = d.get_temp::<ColorDialog>(dialog_id()).unwrap();
            dialog.fields.insert("color".into(), json!("#119944"));
            dialog.alpha = 25.0;
            d.insert_temp(dialog_id(), dialog);
        });
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert_eq!(shape(h.state(), id), &before);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        open_picker(&mut h);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(shape(h.state(), id), &before);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        open_picker(&mut h);
        h.ctx.data_mut(|d| {
            let mut dialog = d.get_temp::<ColorDialog>(dialog_id()).unwrap();
            dialog.alpha = 25.0;
            d.insert_temp(dialog_id(), dialog);
        });
        h.get_by_label("OK").click();
        h.run_steps(3);
        original.alpha = 0.25;
        assert_eq!(shape(h.state(), id).fill, Some(Fill::Solid(original)));
        assert_eq!(shape(h.state(), id).stroke, before.stroke);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), globals);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id), &before);
    }

    #[test]
    fn full_picker_can_enable_solid_paint_from_no_color() {
        let (mut app, id) = app(Tool::Rectangle, 32);
        app.run("shape.edit", json!({"fill": null})).unwrap();
        let stroke = shape(&app, id).stroke.clone();
        let history = app.session.active().unwrap().history.past_len();
        let mut h = harness(app);
        open_picker(&mut h);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert!(matches!(shape(h.state(), id).fill, Some(Fill::Solid(_))));
        assert_eq!(shape(h.state(), id).stroke, stroke);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert!(shape(h.state(), id).fill.is_none());
    }

    #[test]
    fn deferred_screen_colors_keep_alpha_and_full_dialog_survives_hidden_sampler_frames() {
        for full in [false, true] {
            let (mut app, id) = app(Tool::Rectangle, 32);
            app.run("shape.edit", json!({"layer": id.0, "fill": [0.1234567, 0.2345678, 0.3456789, 0.25]})).unwrap();
            let before = shape(&app, id).clone();
            let history = app.session.active().unwrap().history.past_len();
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut receiver = Some(receiver);
            app.services.screen_pick =
                Some(Box::new(move |_| crate::screen_picker::Pending { receiver: receiver.take().unwrap(), cancelled: Default::default() }));
            let mut h = harness(app);
            let ctx = h.ctx.clone();
            crate::screen_picker::tick(h.state_mut(), &ctx);
            if full {
                open_picker(&mut h);
            } else {
                h.get_by_label("Fill color").click();
                h.run_steps(3);
            }
            h.get_by_label("Pick screen color").click();
            h.run_steps(3);
            assert!(crate::screen_picker::tick(h.state_mut(), &ctx));
            egui::Popup::close_all(&ctx);
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-hide-shape-options"), true));
            h.run_steps(5);
            sender.send(Ok(crate::screen_picker::Capture::Color(Some([0.4, 0.5, 0.6])))).unwrap();
            assert!(!crate::screen_picker::tick(h.state_mut(), &ctx));
            ctx.data_mut(|d| d.remove::<bool>(egui::Id::new("test-hide-shape-options")));
            h.run_steps(3);
            if full {
                assert!(color_dialog_open(&ctx));
                assert_eq!(shape(h.state(), id), &before);
                h.get_by_label("OK").click();
                h.run_steps(3);
            }
            let green = if full { 128.0 / 255.0 } else { 0.5 };
            assert_eq!(shape(h.state(), id).fill, Some(Fill::Solid(Color::rgba(0.4, green, 0.6, 0.25))));
            assert_eq!(shape(h.state(), id).stroke, before.stroke);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        }
    }

    #[test]
    fn full_picker_owns_application_shortcuts() {
        let (app, _) = app(Tool::Rectangle, 32);
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let colors = (app.session.tools.foreground, app.session.tools.background);
        let mut h = harness(app);
        open_picker(&mut h);
        let ctx = h.ctx.clone();
        for (key, modifiers) in [
            (egui::Key::Delete, egui::Modifiers::NONE),
            (egui::Key::X, egui::Modifiers::NONE),
            (egui::Key::J, egui::Modifiers { ctrl: true, command: true, ..Default::default() }),
        ] {
            ctx.begin_pass(egui::RawInput {
                events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
                ..Default::default()
            });
            crate::shortcuts::handle(h.state_mut(), &ctx);
            ctx.end_pass().textures_delta.clear();
            h.run_steps(1);
            assert_eq!(h.state().session.active().unwrap().doc, before);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
            assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), colors);
            assert!(color_dialog_open(&ctx));
        }
    }

    #[test]
    fn full_picker_target_and_tool_changes_discard_the_local_draft() {
        let (app, id) = app(Tool::Rectangle, 8);
        let mut h = harness(app);
        open_picker(&mut h);
        h.state_mut().ui.tool = Tool::Move;
        h.run_steps(3);
        assert!(!color_dialog_open(&h.ctx));
        h.state_mut().ui.tool = Tool::Rectangle;
        h.run_steps(3);
        open_picker(&mut h);
        let result = h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [60, 60, 20, 20], "fill": "#bb2288"})).unwrap();
        let next = LayerId(result["layer"].as_u64().unwrap());
        let original = shape(h.state(), id).clone();
        let other = shape(h.state(), next).clone();
        let history = h.state().session.active().unwrap().history.past_len();
        h.run_steps(3);
        assert!(!color_dialog_open(&h.ctx));
        assert_eq!(shape(h.state(), id), &original);
        assert_eq!(shape(h.state(), next), &other);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        open_picker(&mut h);
        h.ctx.data_mut(|d| d.insert_temp(egui::Id::new("test-hide-shape-options"), true));
        h.run_steps(4);
        assert!(!color_dialog_open(&h.ctx), "hiding the owner controls must leave no invisible modal blocking shortcuts");
    }

    #[test]
    fn all_shape_tools_pick_independent_full_paints_with_one_undo_per_action() {
        for (tool, depth) in
            [Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape].into_iter().zip([8, 16, 32].into_iter().cycle())
        {
            let (app, id) = app(tool, depth);
            let colors = (app.session.tools.foreground, app.session.tools.background);
            let before = shape(&app, id).clone();
            let history = app.session.active().unwrap().history.past_len();
            let mut h = harness(app);
            h.get_by_label("Fill color").click();
            h.run_steps(3);
            h.get_by_label("Gradient").click();
            h.run_steps(3);
            let fill = shape(h.state(), id).fill.clone();
            assert!(matches!(fill, Some(Fill::Gradient { .. })));
            assert_eq!(shape(h.state(), id).stroke, before.stroke);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
            h.get_by_label("Fill color").click();
            h.run_steps(3);
            h.get_by_label("Stroke color").click();
            h.run_steps(3);
            h.get_by_label("Pattern").click();
            h.run_steps(3);
            let stroke = shape(h.state(), id).stroke.clone();
            assert!(matches!(stroke.as_ref().unwrap().paint, Fill::Pattern { .. }));
            assert_eq!(shape(h.state(), id).fill, fill);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2);
            assert_eq!((h.state().session.tools.foreground, h.state().session.tools.background), colors);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(h.state(), id).stroke, before.stroke);
            assert_eq!(shape(h.state(), id).fill, fill);
            h.state_mut().run("edit.undo", json!({})).unwrap();
            assert_eq!(shape(h.state(), id), &before);
            h.state_mut().run("edit.redo", json!({})).unwrap();
            h.state_mut().run("edit.redo", json!({})).unwrap();
            let result = crate::vector_ui::create_shape(h.state_mut(), json!({"kind": "ellipse", "rect": [10, 60, 50, 20]})).unwrap();
            let next = shape(h.state(), LayerId(result["layer"].as_u64().unwrap()));
            assert_eq!(next.fill, fill);
            assert_eq!(next.stroke, stroke);
        }
    }

    #[test]
    fn gradient_presets_and_pattern_grid_apply_to_the_explicit_paint_only() {
        let (app, id) = app(Tool::Rectangle, 32);
        let preset = app.session.presets.gradients.iter().flat_map(|g| &g.items).find(|p| p.name != app.session.presets.gradient.name).unwrap().clone();
        let expected = gradient(&app, &preset);
        let global = app.session.presets.gradient.clone();
        let pat = app.session.patterns.items.get(1).unwrap().clone();
        let mut h = harness(app);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Gradient").click();
        h.run_steps(3);
        h.get_by_label(&preset.name).click();
        h.run_steps(3);
        assert_eq!(shape(h.state(), id).fill, Some(expected));
        assert_eq!(h.state().session.presets.gradient, global);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Stroke color").click();
        h.run_steps(3);
        h.get_by_label("Pattern").click();
        h.run_steps(3);
        h.get_by_label(pat.display_name()).click();
        h.run_steps(3);
        assert!(matches!(&shape(h.state(), id).stroke.as_ref().unwrap().paint, Fill::Pattern { id, .. } if id == &pat.id));
    }

    #[test]
    fn gradient_style_choice_in_the_paint_popup_edits_in_one_undo_step() {
        let (app, id) = app(Tool::Rectangle, 32);
        let before = shape(&app, id).clone();
        let mut h = harness(app);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Gradient").click();
        h.run_steps(3);
        let linear = shape(h.state(), id).fill.clone();
        let history = h.state().session.active().unwrap().history.past_len();
        h.get_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Linear")).unwrap().click();
        h.run_steps(3);
        h.get_by_label("Radial").click();
        h.run_steps(3);
        assert!(h.query_by_label("Gradient stops").is_some(), "the child style dropdown must keep its paint popup open");
        let mut expected = linear.clone();
        if let Some(Fill::Gradient { style, .. }) = &mut expected {
            *style = GradientStyle::Radial;
        }
        assert_eq!(shape(h.state(), id).fill, expected);
        assert_eq!(shape(h.state(), id).stroke, before.stroke);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id).fill, linear);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(shape(h.state(), id).fill, expected);
        h.get_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Radial")).unwrap().click();
        h.run_steps(3);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.query_by_label("Gradient stops").is_some(), "first Escape closes the child list");
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.query_by_label("Gradient stops").is_none(), "second Escape closes the paint popup");
        assert_eq!(shape(h.state(), id).fill, expected);
    }

    #[test]
    fn empty_pattern_library_disables_pattern_choice_and_old_settings_follow_colors() {
        let (mut app, id) = app(Tool::Rectangle, 8);
        app.session.patterns.items.clear();
        let before = shape(&app, id).clone();
        let mut h = harness(app);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Pattern").click();
        h.run_steps(3);
        assert_eq!(shape(h.state(), id), &before);
        let mut saved = serde_json::to_value(crate::state::ToolOptions::default()).unwrap();
        for key in ["shape_fill_paint", "shape_stroke_paint"] {
            saved.as_object_mut().unwrap().remove(key);
        }
        let options: crate::state::ToolOptions = serde_json::from_value(saved).unwrap();
        assert!(options.shape_fill_paint.is_none() && options.shape_stroke_paint.is_none());
    }

    #[test]
    fn paint_defaults_without_a_shape_target_persist_and_seed_the_next_shape() {
        let (mut app, _) = app(Tool::EllipseShape, 16);
        app.run("layer.new.layer", json!({})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        let expected = gradient(&app, &app.session.presets.gradient);
        let mut h = harness(app);
        h.get_by_label("Fill color").click();
        h.run_steps(3);
        h.get_by_label("Gradient").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc, before);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        let saved = serde_json::to_value(&h.state().ui.tool_options).unwrap();
        let restored: crate::state::ToolOptions = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.shape_fill_paint, Some(expected.clone()));
        h.state_mut().ui.tool_options = restored;
        let result = crate::vector_ui::create_shape(h.state_mut(), json!({"kind": "ellipse", "rect": [10, 60, 50, 20]})).unwrap();
        let next = shape(h.state(), LayerId(result["layer"].as_u64().unwrap()));
        assert_eq!(next.fill, Some(expected));
    }
}
