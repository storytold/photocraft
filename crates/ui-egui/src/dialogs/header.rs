//! Shared dialog chrome: content icon, elided title and a separate close target.

use egui::{Align, Layout, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::state::{Dialog, DialogKind};
use crate::theme::{self, Tokens};

const MARGIN: i8 = 16;
const HEIGHT: f32 = 32.0;
const CLOSE_SIZE: f32 = 24.0;

pub(super) fn frame(ctx: &egui::Context) -> egui::Frame {
    let mut frame = egui::Frame::popup(&ctx.global_style()).corner_radius(ctx.global_style().visuals.window_corner_radius).inner_margin(egui::Margin {
        left: MARGIN,
        right: MARGIN,
        top: 0,
        bottom: MARGIN,
    });
    // Keep the frame geometry; the directional border is painted after the contents.
    frame.stroke.color = egui::Color32::TRANSPARENT;
    frame.shadow = Tokens::get(ctx).dialog_shadow();
    frame
}

pub(super) fn paint_border(ctx: &egui::Context, response: &egui::Response) {
    let rect = response.rect;
    if !rect.is_finite() || rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    let [top, bottom] = Tokens::get(ctx).dialog_border();
    let rounded = egui::epaint::RoundedRect::new(rect, ctx.global_style().visuals.window_corner_radius).shrink(0.5);
    let mut points = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(&mut points, rounded);
    let fade_height = 16.0_f32.min(rect.height() / 2.0);
    // Sample the eased fade along both sides. The GPU interpolates vertex colours,
    // so a single end vertex would turn the curve back into a visibly linear ramp.
    let mut rim = Vec::with_capacity(points.len() + 16);
    for (from, to) in points.iter().zip(points.iter().cycle().skip(1)) {
        rim.push(*from);
        for step in 1..=8 {
            let sample = if from.y < to.y { step } else { 9 - step };
            let y = rect.top() + fade_height * sample as f32 / 8.0;
            if (from.y < y && to.y > y) || (from.y > y && to.y < y) {
                rim.push(from.lerp(*to, (y - from.y) / (to.y - from.y)));
            }
        }
    }
    let stroke = egui::epaint::PathStroke::new_uv(1.0, move |_, position| {
        let fraction = ((position.y - rect.top()) / fade_height).clamp(0.0, 1.0);
        let eased = fraction * fraction * (3.0 - 2.0 * fraction);
        top.lerp_to_gamma(bottom, eased)
    });
    ctx.layer_painter(response.layer_id).add(egui::epaint::PathShape { points: rim, closed: true, fill: egui::Color32::TRANSPARENT, stroke });
}

/// Use stable dialog kinds and field markers, never translated titles, to choose the icon.
fn icon(d: &Dialog) -> &'static str {
    match d.kind {
        DialogKind::LayerStyle => "fx",
        DialogKind::NewDocument => "file-plus",
        DialogKind::About => "info",
        DialogKind::Error => "triangle-alert",
        DialogKind::Command if crate::color_picker_ui::owns(&d.fields) || crate::color_range_ui::owns(&d.fields) => "palette",
        DialogKind::Command if crate::prefs_ui::owns(&d.fields) => "settings",
        DialogKind::Command if d.fields.contains_key("__export") => "image",
        DialogKind::Command if d.fields.contains_key("__sizing") => "scan",
        DialogKind::Command => "sliders-horizontal",
    }
}

fn paint_icon(ui: &Ui, rect: Rect, name: &str, tint: egui::Color32) {
    if name == "fx" {
        // Match the existing italic effects mark in the Layers footer.
        let mut job = egui::text::LayoutJob::default();
        job.append("fx", 0.0, egui::TextFormat { font_id: egui::FontId::proportional(15.0), color: tint, italics: true, ..Default::default() });
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, tint);
    } else {
        crate::icons::paint(ui, rect, name, 14.0, tint);
    }
}

pub(super) fn show(ui: &mut Ui, id: egui::Id, dialog: &Dialog, title: &str) -> (Vec2, bool) {
    let t = Tokens::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    let band = row.expand2(vec2(f32::from(MARGIN), 0.0));
    // Resize clips to the body. Only the header may paint and receive drags in the side insets.
    let body_clip = ui.clip_rect();
    ui.set_clip_rect(body_clip.union(band).intersect(ui.ctx().content_rect()));
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(band, egui::CornerRadius { sw: 0, se: 0, ..radius }, t.dialog_header());
    ui.painter().hline(band.x_range(), band.bottom(), Stroke::new(1.0, t.separator));

    // Square end slots keep each glyph equally far from the outer and vertical edges.
    let icon_rect = Rect::from_center_size(pos2(band.left() + HEIGHT / 2.0, row.center().y), Vec2::splat(20.0));
    paint_icon(ui, icon_rect, icon(dialog), t.icon);
    let divider_x = icon_rect.right() + 8.0;
    ui.painter().vline(divider_x, row.center().y - 6.0..=row.center().y + 6.0, Stroke::new(1.0, t.field_border));

    let close = super::CLOSE_BOX.then(|| Rect::from_center_size(pos2(band.right() - HEIGHT / 2.0, row.center().y), Vec2::splat(CLOSE_SIZE)));
    let title_left = divider_x + 8.0;
    let title_right = close.map_or(row.right(), |r| r.left() - 8.0);
    let galley = crate::tab_strip::elided(ui, title, theme::semibold(12.0), t.text, (title_right - title_left).max(0.0));
    let elided = galley.elided;
    let title_rect = Rect::from_min_size(pos2(title_left, row.center().y - galley.size().y / 2.0), galley.size());
    ui.painter().galley(title_rect.min, galley, t.text);
    let label = ui.interact(title_rect, id.with("title-label"), Sense::hover());
    label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), title));
    if elided {
        label.on_hover_text(title);
    }

    // Leave the close target out of the drag band, including when a press becomes a drag.
    let drag_rect = Rect::from_min_max(band.min, pos2(close.map_or(band.right(), |r| r.left()), band.bottom()));
    let drag = ui.interact(drag_rect, id.with("title"), Sense::drag()).drag_delta();
    let clicked = close.is_some_and(|rect| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center)));
        crate::icons::button_with_icon_size(&mut child, "x", CLOSE_SIZE, 12.0, false, tl!("Close")).clicked()
    });
    ui.set_clip_rect(body_clip);
    // allocate_exact_size already advances by item_spacing; keep the body inset exactly 16 pt.
    ui.add_space((f32::from(MARGIN) - ui.spacing().item_spacing.y).max(0.0));
    (drag, clicked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn long_titles_leave_an_even_inset_and_a_separate_close_target_in_every_theme() {
        let title = "Sélecteur de couleurs (Couleur de premier plan) — a deliberately long translated title";
        for theme in theme::ThemeKind::ALL {
            for width in [260.0, 600.0] {
                let dialog = Dialog { id: 1, kind: DialogKind::LayerStyle, fields: Default::default() };
                let mut h = Harness::builder().with_size(vec2(800.0, 300.0)).build_ui_state(
                    move |ui, ready| {
                        if *ready {
                            frame(ui.ctx()).show(ui, |ui| {
                                ui.set_width(width);
                                show(ui, egui::Id::unique("header-test"), &dialog, title);
                            });
                        }
                    },
                    false,
                );
                crate::PhotocraftApp::setup_context(&h.ctx, theme);
                *h.state_mut() = true;
                h.run_steps(4);
                let label = h.get_by_label(title).rect();
                let band = h.ctx.read_response(egui::Id::unique("header-test").with("title")).unwrap().rect;
                // The drag band stops at the close target, so recover the full header width.
                let right = band.left() + width + 2.0 * f32::from(MARGIN);
                assert!(label.right() <= right - f32::from(MARGIN) + 1.0, "{theme:?}: title stays inside the header at {width} pt");
                if super::super::CLOSE_BOX {
                    let close = h.get_by_label("Close").rect();
                    assert!((close.width() - CLOSE_SIZE).abs() < 0.5, "{theme:?}: a usable close target");
                    assert!((label.center().y - close.center().y).abs() < 0.5, "{theme:?}: common center line");
                    assert!(label.right() <= close.left() - 7.0, "{theme:?}: title never overlaps close at {width} pt");
                    assert!((right - close.right() - (close.top() - band.top())).abs() < 1.0, "{theme:?}: equal close insets");
                    assert!((label.left() - band.left() - 42.0).abs() < 1.0, "{theme:?}: compact icon and title inset");
                }
            }
        }
    }
}
