//! Blending controls shared by Edit › Fill and Edit › Stroke.
//!
//! Both commands support the same layer blend modes, opacity and transparency lock.
//! Keeping one widget and validation implementation prevents their options diverging.

use egui::{Sense, vec2};
use serde_json::{Map, Value, json};

use crate::theme::Tokens;

/// Validate a persisted blending choice before restoring it from Preferences.
pub fn valid(key: &str, v: &Value) -> bool {
    match key {
        "mode" => v.as_str().and_then(photocraft_engine::commands::blend_from_str).is_some_and(|m| m != photocraft_color::BlendMode::PassThrough),
        "opacity" => v.as_f64().is_some_and(|o| o.is_finite() && (0.0..=100.0).contains(&o)),
        "preserveTransparency" => v.is_boolean(),
        _ => false,
    }
}

/// UI for the identical Blending section of Fill and Stroke.
pub fn body(ui: &mut egui::Ui, fields: &mut Map<String, Value>, prefix: &str) {
    ui.add_space(4.0);
    crate::widgets::section_label(ui, tl!("Blending"));
    crate::widgets::hairline(ui);
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("Mode:"));
        let mut mode = fields.get("mode").and_then(Value::as_str).unwrap_or("normal").to_string();
        let opts: Vec<(String, &str)> = photocraft_color::BlendMode::LAYER_MODES.iter().map(|m| (mode_key(*m), m.label())).collect();
        if crate::widgets::dropdown(ui, &format!("{prefix}-mode"), tl!("Blend Mode"), &mut mode, &opts, 170.0) {
            fields.insert("mode".into(), json!(mode));
        }
    });
    crate::direction::dialog_row(ui, |ui| {
        label(ui, tl!("Opacity:"));
        let mut opacity = fields.get("opacity").and_then(Value::as_f64).unwrap_or(100.0) as f32;
        if crate::widgets::value_field(ui, &mut opacity, 0.0..=100.0, "%", 70.0).accessible_name(tl!("Opacity:")).changed() {
            fields.insert("opacity".into(), json!(opacity.round()));
        }
    });
    crate::direction::dialog_row(ui, |ui| {
        label(ui, "");
        let mut preserve = fields.get("preserveTransparency").and_then(Value::as_bool).unwrap_or(false);
        if crate::widgets::checkbox(ui, &mut preserve, tl!("Preserve Transparency")).changed() {
            fields.insert("preserveTransparency".into(), json!(preserve));
        }
    });
}

/// A caption in a fixed column that hugs the field it names: the box's right edge in a
/// left-to-right UI, its left edge in a right-to-left one (the row mirrors under the host's
/// `body_layout`, so the caption box sits at the reading start and the field beside it toward
/// the reading end).
fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(112.0, 22.0), Sense::hover());
    let (pos, align) = crate::direction::end_text_anchor(r, 6.0);
    ui.painter().text(pos, align, text, egui::FontId::proportional(12.0), t.text_dim);
}

/// The `mode` param naming a blend mode ("normal", "colorBurn"…), parsed by the engine.
pub(crate) fn mode_key(m: photocraft_color::BlendMode) -> String {
    let s = format!("{m:?}");
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Align2;

    /// The caption hugs the control beside it: the box's right edge in a left-to-right row,
    /// its left edge in a mirrored one — the old fixed right-edge paint left the caption
    /// detached from its field, and the gap read as dead void, in a right-to-left dialog.
    #[test]
    fn the_caption_anchor_follows_the_direction() {
        let r = egui::Rect::from_min_size(egui::pos2(10.0, 0.0), vec2(112.0, 22.0));
        let _ltr = crate::direction::direction_scope(crate::direction::Dir::Ltr);
        let (ltr_pos, ltr_align) = crate::direction::end_text_anchor(r, 6.0);
        assert!(matches!(ltr_align, Align2::RIGHT_CENTER));
        assert!((ltr_pos.x - (r.right() - 6.0)).abs() < 0.01);
        drop(_ltr);
        let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
        let (rtl_pos, rtl_align) = crate::direction::end_text_anchor(r, 6.0);
        assert!(matches!(rtl_align, Align2::LEFT_CENTER));
        assert!((rtl_pos.x - (r.left() + 6.0)).abs() < 0.01);
    }

    /// Rendered twin: in a real `body_layout` row the caption box sits flush against the field
    /// it names — left of the field left-to-right, right of it right-to-left.
    #[test]
    fn the_rendered_caption_hugs_the_field_in_both_directions() {
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1280.0, 800.0));
        let row = |dir: crate::text::direction::Dir| {
            let _dir = crate::text::direction::direction_scope(dir);
            let ctx = egui::Context::default();
            let mut rects = (egui::Rect::NOTHING, egui::Rect::NOTHING);
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(area), ..Default::default() }, |ui| {
                ui.with_layout(crate::direction::body_layout(), |ui| {
                    ui.horizontal(|ui| {
                        let (cap, _) = ui.allocate_exact_size(vec2(112.0, 22.0), Sense::hover());
                        let (pos, align) = crate::direction::end_text_anchor(cap, 6.0);
                        ui.painter().text(pos, align, "Mode:", egui::FontId::proportional(12.0), egui::Color32::WHITE);
                        let (field, _) = ui.allocate_exact_size(vec2(170.0, 22.0), Sense::hover());
                        rects = (cap, field);
                    });
                });
            });
            out.textures_delta.clear();
            rects
        };
        let (ltr_cap, ltr_field) = row(crate::text::direction::Dir::Ltr);
        let (rtl_cap, rtl_field) = row(crate::text::direction::Dir::Rtl);
        assert!(ltr_cap.center().x < ltr_field.center().x, "LTR: the caption leads: {ltr_cap:?} / {ltr_field:?}");
        assert!((ltr_field.left() - ltr_cap.right()).abs() <= 16.0, "LTR: flush: {ltr_cap:?} / {ltr_field:?}");
        assert!(rtl_cap.center().x > rtl_field.center().x, "RTL: the caption leads from the right: {rtl_cap:?} / {rtl_field:?}");
        assert!((rtl_cap.left() - rtl_field.right()).abs() <= 16.0, "RTL: flush: {rtl_cap:?} / {rtl_field:?}");
    }
}

#[cfg(test)]
mod rtl_row_twin {
    use super::*;
    use crate::direction::Dir;
    use egui_kittest::kittest::Queryable;

    /// The Blending section drawn in `dir`, with the column it draws in.
    fn harness(dir: Dir) -> egui_kittest::Harness<'static, (Map<String, Value>, egui::Rect)> {
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(440.0, 360.0))
            .with_context_setup(|ctx| crate::PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ALL[0]))
            .build_ui_state(
                move |ui, s: &mut (Map<String, Value>, egui::Rect)| {
                    let _dir = crate::direction::direction_scope(dir);
                    s.1 = ui.max_rect();
                    ui.with_layout(crate::direction::body_layout(), |ui| body(ui, &mut s.0, "t"));
                },
                (Map::new(), egui::Rect::ZERO),
            );
        h.run_steps(3);
        h
    }

    /// The migrated rows mirror with the body: each field's gap from the reading-start edge of
    /// the column is the same in LTR and RTL (the caption box leads the field in both).
    #[test]
    fn the_blending_rows_mirror_to_the_reading_start() {
        let ltr = harness(Dir::Ltr);
        let rtl = harness(Dir::Rtl);
        let ltr_col = ltr.state().1;
        let rtl_col = rtl.state().1;
        let ltr_combo = ltr.get_by_label("Blend Mode").rect();
        let rtl_combo = rtl.get_by_label("Blend Mode").rect();
        let ltr_field = ltr.get_by_label("Opacity:").rect();
        let rtl_field = rtl.get_by_label("Opacity:").rect();
        assert!((ltr_combo.left() - ltr_col.left() - (rtl_col.right() - rtl_combo.right())).abs() < 1.0, "the mode row mirrors: {ltr_combo:?} {rtl_combo:?} in {ltr_col:?} {rtl_col:?}");
        assert!((ltr_field.left() - ltr_col.left() - (rtl_col.right() - rtl_field.right())).abs() < 1.0, "the opacity row mirrors: {ltr_field:?} {rtl_field:?}");
        assert!(ltr_combo.left() - ltr_col.left() > 100.0, "the caption leads the field in LTR: {ltr_combo:?} {ltr_col:?}");
        assert!(rtl_col.right() - rtl_combo.right() > 100.0, "the caption leads the field in RTL: {rtl_combo:?} {rtl_col:?}");
    }
}
