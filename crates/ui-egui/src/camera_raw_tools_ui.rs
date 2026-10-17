//! Camera Raw's tool strip beyond Edit (Remove, Masking, Red Eye, Presets), its More (…) menu
//! and its Preferences, laid out as Photoshop 25.4's Camera Raw 16.1.1 shows them. Presets and
//! the More menu work; Remove, Masking and Red Eye are laid out with their controls disabled
//! until PhotoCraft has those local tools (Masking's Subject / Sky / People need models).

use egui::{Align2, Color32, Rect, Sense, Ui, pos2, vec2};
use photocraft_algo::camera_raw::CameraRaw;
use serde_json::Value;

use crate::camera_raw_style::{self as st, Acr, Spec};
use crate::camera_raw_ui::CameraRawDialog;
use crate::state::CameraRawLayoutState;

/// The strip's tools: id, icon, tooltip (with Camera Raw's shortcut).
pub(crate) const TOOLS: [(&str, &str, &str); 5] = [
    ("edit", "sliders-horizontal", "Edit (E)"),
    ("remove", "bandage", "Remove (B)"),
    ("masking", "circle-dashed", "Masking (M)"),
    ("redEye", "scan-eye", "Red Eye (Shift+E)"),
    ("presets", "blend", "Presets (Shift+P)"),
];

fn tr(s: &'static str) -> &'static str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

/// Dialog-local state of the placeholder tools (not filter settings).
#[derive(Clone, Debug)]
pub(crate) struct ToolState {
    pub remove_mode: usize,
    pub remove_size: f32,
    pub remove_opacity: f32,
    pub show_overlay: bool,
    pub visualize_spots: bool,
    pub red_eye_pet: bool,
    pub pupil_size: f32,
    pub darken: f32,
    pub red_eye_overlay: bool,
    /// Presets: the settings before the last preset, the preset's settings, and Amount (0–200).
    pub preset: Option<(CameraRaw, CameraRaw, String)>,
    pub preset_amount: f32,
    pub open_groups: Vec<&'static str>,
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            remove_mode: 0,
            remove_size: 25.0,
            remove_opacity: 100.0,
            show_overlay: false,
            visualize_spots: false,
            red_eye_pet: false,
            pupil_size: 50.0,
            darken: 50.0,
            red_eye_overlay: true,
            preset: None,
            preset_amount: 100.0,
            open_groups: vec!["Black & White", "Color"],
        }
    }
}

fn panel_title(ui: &mut Ui, title: &str) {
    let c = Acr::get(ui.ctx());
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), st::ROW), Sense::hover());
    ui.painter().text(pos2(row.left() + st::INSET_L, row.center().y), Align2::LEFT_CENTER, title, st::title_font(), c.text);
    crate::icons::paint(ui, Rect::from_center_size(pos2(row.right() - 46.0, row.center().y), vec2(18.0, 18.0)), "undo-2", 14.0, c.faint);
    crate::icons::paint(ui, Rect::from_center_size(pos2(row.right() - 18.0, row.center().y), vec2(18.0, 18.0)), "eye", 14.0, c.faint);
}

fn not_yet(ui: &mut Ui) {
    st::note(ui, tl!("This tool is not available in PhotoCraft yet."), true);
}

/// Remove (healing): Remove / Heal / Clone, Size, Opacity, Refresh, overlays.
pub(crate) fn remove(ui: &mut Ui, s: &mut ToolState) {
    panel_title(ui, tr("Healing"));
    let tips = [tr("Remove"), tr("Heal"), tr("Clone")];
    let icons = ["eraser", "bandage", "stamp"];
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L - 2.0);
        for (i, tip) in tips.iter().enumerate() {
            if crate::icons::button(ui, icons[i], 26.0, s.remove_mode == i, tip).clicked() {
                s.remove_mode = i;
            }
        }
    });
    ui.add_enabled_ui(false, |ui| {
        st::slider(ui, "removeSize", tr("Size"), &mut s.remove_size, &Spec::amount(100.0, 25.0).enabled(false));
        st::slider(ui, "removeOpacity", tr("Opacity"), &mut s.remove_opacity, &Spec::amount(100.0, 100.0).enabled(false));
        ui.horizontal(|ui| {
            ui.add_space(st::INSET_L);
            st::small_button(ui, tr("Refresh"), false, false, 70.0);
        });
    });
    st::rule(ui);
    st::check_row(ui, tr("Show Overlay"), &mut s.show_overlay, false);
    st::check_row(ui, tr("Visualize Spots"), &mut s.visualize_spots, false);
    not_yet(ui);
}

/// Masking: Create New Mask (Subject, Sky, Background need segmentation models; Objects,
/// Brush, gradients and Range are local tools to come).
pub(crate) fn masking(ui: &mut Ui) {
    let c = Acr::get(ui.ctx());
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L);
        ui.label(egui::RichText::new(tr("Create New Mask")).font(st::label_font()).color(c.text));
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L - 4.0);
        for (icon, label) in [("user-round", tr("Subject")), ("mountain", tr("Sky")), ("image", tr("Background"))] {
            ui.vertical(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
                ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, c.field_border.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                crate::icons::paint(ui, r, icon, 26.0, c.faint);
                ui.label(egui::RichText::new(label).font(st::small_font()).color(c.faint));
            });
            ui.add_space(14.0);
        }
    });
    st::rule(ui);
    for (icon, label, key) in [
        ("square-dashed-mouse-pointer", tr("Objects"), ""),
        ("brush", tr("Brush"), "K"),
        ("rectangle-horizontal", tr("Linear Gradient"), "G"),
        ("circle-dot", tr("Radial Gradient"), "J"),
        ("pipette", tr("Range"), ""),
    ] {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        crate::icons::paint(ui, Rect::from_center_size(pos2(r.left() + st::INSET_L + 8.0, r.center().y), vec2(18.0, 18.0)), icon, 15.0, c.faint);
        ui.painter().text(pos2(r.left() + st::INSET_L + 26.0, r.center().y), Align2::LEFT_CENTER, label, st::label_font(), c.faint);
        if !key.is_empty() {
            let k = Rect::from_center_size(pos2(r.right() - st::INSET_R - 8.0, r.center().y), vec2(16.0, 16.0));
            ui.painter().rect_stroke(k, 1.0, egui::Stroke::new(1.0, c.field_border.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            ui.painter().text(k.center(), Align2::CENTER_CENTER, key, st::small_font(), c.faint);
        }
    }
    st::rule(ui);
    st::note(ui, tl!("Masks need local adjustments and segmentation models; PhotoCraft does not have them in Camera Raw yet."), true);
}

pub(crate) fn red_eye(ui: &mut Ui, s: &mut ToolState) {
    panel_title(ui, tr("Red Eye"));
    let c = Acr::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(st::INSET_L);
        ui.set_min_height(30.0);
        ui.label(egui::RichText::new(tr("Type")).font(st::label_font()).color(c.label));
        ui.add_space(8.0);
        ui.add_enabled_ui(false, |ui| {
            crate::widgets::dropdown(ui, "cr-redeye-type", &mut s.red_eye_pet, &[(false, tr("Red Eye")), (true, tr("Pet Eye"))], 100.0);
            st::small_button(ui, tr("Auto"), false, false, 40.0);
        });
    });
    st::slider(ui, "pupilSize", tr("Pupil Size"), &mut s.pupil_size, &Spec::amount(100.0, 50.0).enabled(false));
    st::slider(ui, "darken", tr("Darken"), &mut s.darken, &Spec::amount(100.0, 50.0).enabled(false));
    st::rule(ui);
    st::check_row(ui, tr("Overlay"), &mut s.red_eye_overlay, false);
    not_yet(ui);
}

/// PhotoCraft's own starting points (not Adobe's presets): setting changes on top of the
/// defaults. Amount scales the change, 0–200 %.
pub(crate) fn builtin_presets() -> Vec<(&'static str, Vec<(&'static str, CameraRaw)>)> {
    let d = CameraRaw::default;
    vec![
        (
            "Black & White",
            vec![
                ("B&W Neutral", CameraRaw { monochrome: true, ..d() }),
                ("B&W High Contrast", CameraRaw { monochrome: true, contrast: 40.0, clarity: 15.0, blacks: -15.0, whites: 10.0, ..d() }),
                ("B&W Soft", CameraRaw { monochrome: true, contrast: -20.0, highlights: -25.0, shadows: 20.0, ..d() }),
            ],
        ),
        (
            "Color",
            vec![
                ("Vivid", CameraRaw { vibrance: 35.0, saturation: 8.0, contrast: 15.0, ..d() }),
                ("Muted", CameraRaw { vibrance: -25.0, saturation: -10.0, contrast: -5.0, ..d() }),
                ("Warm", CameraRaw { temperature: 15.0, tint: 5.0, white_balance: "custom".into(), ..d() }),
                ("Cool", CameraRaw { temperature: -15.0, white_balance: "custom".into(), ..d() }),
            ],
        ),
        (
            "Creative",
            vec![
                ("Faded", CameraRaw { point_curve: vec![[0.0, 32.0], [128.0, 130.0], [255.0, 238.0]], contrast: -10.0, saturation: -10.0, ..d() }),
                (
                    "Split Tone",
                    CameraRaw {
                        grade_shadows: photocraft_algo::camera_raw::Wheel { hue: 210.0, sat: 25.0, lum: 0.0 },
                        grade_highlights: photocraft_algo::camera_raw::Wheel { hue: 45.0, sat: 20.0, lum: 0.0 },
                        ..d()
                    },
                ),
                ("Clarity Pop", CameraRaw { clarity: 30.0, texture: 15.0, vibrance: 10.0, ..d() }),
                ("Haze Cut", CameraRaw { dehaze: 25.0, contrast: 5.0, ..d() }),
            ],
        ),
    ]
}

/// `base` moved toward `preset`'s changes by `amount` (0..=2): numbers and number arrays
/// interpolate, other settings switch at half.
pub(crate) fn blend(base: &CameraRaw, preset: &CameraRaw, amount: f32) -> CameraRaw {
    let d = serde_json::to_value(CameraRaw::default()).unwrap_or(Value::Null);
    let (Ok(mut b), Ok(p)) = (serde_json::to_value(base), serde_json::to_value(preset)) else { return base.clone() };
    fn lerp(out: &mut Value, def: &Value, target: &Value, t: f64) {
        match (out, def, target) {
            (Value::Object(o), Value::Object(dm), Value::Object(tm)) => {
                for (k, tv) in tm {
                    if let (Some(ov), Some(dv)) = (o.get_mut(k), dm.get(k)) {
                        lerp(ov, dv, tv, t);
                    }
                }
            }
            (o @ Value::Number(_), Value::Number(dv), Value::Number(tv)) => {
                let (dv, tv) = (dv.as_f64().unwrap_or(0.0), tv.as_f64().unwrap_or(0.0));
                if (dv - tv).abs() > 1e-9 {
                    let ov = o.as_f64().unwrap_or(0.0);
                    if let Some(n) = serde_json::Number::from_f64(ov + (tv - dv) * t) {
                        *o = Value::Number(n);
                    }
                }
            }
            (o @ Value::Array(_), Value::Array(da), Value::Array(ta)) if da.len() == ta.len() && da.iter().all(Value::is_number) => {
                if let Value::Array(oa) = o
                    && oa.len() == ta.len()
                {
                    for ((ov, dv), tv) in oa.iter_mut().zip(da).zip(ta) {
                        lerp(ov, dv, tv, t);
                    }
                }
            }
            (o, def, target) => {
                if def != target && t >= 0.5 {
                    *o = target.clone();
                }
            }
        }
    }
    lerp(&mut b, &d, &p, amount as f64);
    let mut out: CameraRaw = serde_json::from_value(b).unwrap_or_else(|_| base.clone());
    (out.pixel_scale, out.encoding_gamma, out.seed) = (base.pixel_scale, base.encoding_gamma, base.seed);
    out
}

/// Presets: Amount, then the groups. Returns whether settings changed.
pub(crate) fn presets(ui: &mut Ui, d: &mut CameraRawDialog) -> bool {
    let c = Acr::get(ui.ctx());
    let mut changed = false;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), st::ROW), Sense::hover());
    ui.painter().text(pos2(row.left() + st::INSET_L, row.center().y), Align2::LEFT_CENTER, tr("Presets"), st::title_font(), c.text);
    st::rule(ui);
    let name = d.tools.preset.as_ref().map(|(_, _, n)| n.clone());
    let active = name.is_some();
    let mut amount = d.tools.preset_amount;
    let label = name.unwrap_or_else(|| tr("None").to_owned());
    if st::slider(ui, "presetAmount", &label, &mut amount, &Spec::amount(200.0, 100.0).enabled(active)).changed {
        d.tools.preset_amount = amount;
        if let Some((base, preset, _)) = &d.tools.preset {
            d.params = blend(base, preset, amount / 100.0);
            changed = true;
        }
    }
    st::rule(ui);
    for (group, items) in builtin_presets() {
        let mut open = d.tools.open_groups.contains(&group);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        if resp.clicked() {
            open = !open;
        }
        let tri = pos2(r.left() + st::INSET_L - 4.0, r.center().y);
        let pts = if open {
            vec![tri + vec2(-4.0, -3.0), tri + vec2(4.0, -3.0), tri + vec2(0.0, 3.0)]
        } else {
            vec![tri + vec2(-3.0, -4.0), tri + vec2(-3.0, 4.0), tri + vec2(3.0, 0.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, c.text, egui::Stroke::NONE));
        ui.painter().text(pos2(r.left() + st::INSET_L + 8.0, r.center().y), Align2::LEFT_CENTER, tr(group), st::label_font(), c.label);
        d.tools.open_groups.retain(|g| *g != group);
        if open {
            d.tools.open_groups.push(group);
            for (item, preset) in items {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                let selected = d.tools.preset.as_ref().is_some_and(|(_, _, n)| n == tr(item));
                if selected {
                    ui.painter().rect_filled(r, 0.0, c.accent.gamma_multiply(0.45));
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, c.hover.gamma_multiply(0.4));
                }
                ui.painter().text(pos2(r.left() + st::INSET_L + 22.0, r.center().y), Align2::LEFT_CENTER, tr(item), st::label_font(), c.text);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, tr(item)));
                if resp.clicked() {
                    let base = d.tools.preset.as_ref().map(|(b, _, _)| b.clone()).unwrap_or_else(|| d.params.clone());
                    d.tools.preset_amount = 100.0;
                    d.params = blend(&base, &preset, 1.0);
                    d.tools.preset = Some((base, preset, tr(item).to_owned()));
                    changed = true;
                }
            }
        }
    }
    st::rule(ui);
    st::note(ui, tl!("PhotoCraft's own presets. Saving and importing presets is not available yet."), true);
    changed
}

/// The More (…) menu. Returns whether settings changed; Load / Save Settings set `action`
/// ("load" / "save", run by the dialog after the frame).
pub(crate) fn more_menu(ui: &mut Ui, d: &mut CameraRawDialog, previous: Option<CameraRaw>, action: &mut Option<&'static str>) -> bool {
    ui.set_min_width(240.0);
    let mut changed = false;
    let item = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool| -> bool {
        let b = egui::Button::new(label);
        let b = if shortcut.is_empty() { b } else { b.shortcut_text(shortcut) };
        ui.add_enabled(enabled, b).clicked()
    };
    let at_open = d.params == d.original;
    if item(ui, tr("Reset to Open"), "", !at_open) {
        d.params = d.original.clone();
        changed = true;
    }
    let defaults = CameraRaw { pixel_scale: d.params.pixel_scale, encoding_gamma: d.params.encoding_gamma, ..Default::default() };
    if item(ui, tr("Reset to Default"), "", d.params != defaults) {
        d.params = defaults;
        changed = true;
    }
    ui.separator();
    if item(ui, tr("Apply Previous Settings"), "", previous.is_some())
        && let Some(prev) = previous
    {
        d.params = CameraRaw { pixel_scale: d.params.pixel_scale, encoding_gamma: d.params.encoding_gamma, ..prev };
        changed = true;
    }
    item(ui, tr("Apply Favorite Preset"), "", false);
    ui.separator();
    item(ui, tr("Create Preset..."), "Ctrl+Shift+P", false);
    ui.separator();
    if item(ui, tr("Copy Edit Settings"), "Ctrl+C", true) {
        let copy = serde_json::to_value(&d.params).unwrap_or(Value::Null);
        ui.ctx().data_mut(|m| m.insert_temp(egui::Id::new("camera-raw-clipboard"), copy));
    }
    item(ui, tr("Copy Selected Edit Settings..."), "Ctrl+Alt+C", false);
    let clip: Option<Value> = ui.ctx().data(|m| m.get_temp(egui::Id::new("camera-raw-clipboard")));
    if item(ui, tr("Paste Edit Settings"), "Ctrl+V", clip.is_some())
        && let Some(v) = clip
        && let Ok(p) = serde_json::from_value::<CameraRaw>(v)
        && p.validate().is_ok()
    {
        d.params = CameraRaw { pixel_scale: d.params.pixel_scale, encoding_gamma: d.params.encoding_gamma, ..p };
        changed = true;
    }
    ui.separator();
    if item(ui, tr("Load Settings..."), "", true) {
        *action = Some("load");
        ui.close();
    }
    if item(ui, tr("Save Settings..."), "", true) {
        *action = Some("save");
        ui.close();
    }
    if changed {
        ui.close();
    }
    changed
}

/// Camera Raw Preferences (General, Performance). Only Edit panel behavior acts in PhotoCraft;
/// the other options are shown disabled.
pub(crate) fn preferences(ctx: &egui::Context, open: &mut bool, tab: &mut usize, layout: &mut CameraRawLayoutState) {
    if !*open {
        return;
    }
    let c = Acr::get(ctx);
    let mut keep = true;
    egui::Window::new(tl!("Camera Raw Preferences"))
        .id(egui::Id::new("camera-raw-prefs"))
        .collapsible(false)
        .resizable(false)
        .fixed_size(vec2(760.0, 470.0))
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .open(&mut keep)
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(110.0);
                    for (i, name) in [tr("General"), tr("Performance")].iter().enumerate() {
                        if ui.selectable_label(*tab == i, *name).clicked() {
                            *tab = i;
                        }
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(520.0);
                    let off = |ui: &mut Ui, label: &str, mut v: bool| {
                        ui.add_enabled(false, egui::Checkbox::new(&mut v, label)).on_disabled_hover_text(tl!("Not available yet"));
                    };
                    if *tab == 0 {
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(tr("Appearance")).strong());
                            ui.horizontal(|ui| {
                                ui.label(tr("Color Theme:"));
                                ui.add_enabled(false, egui::Button::new(tr("Default")))
                                    .on_disabled_hover_text(tl!("Camera Raw follows PhotoCraft's theme (Window › Theme)"));
                            });
                        });
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(tr("Panels")).strong());
                            ui.horizontal(|ui| {
                                ui.label(tr("Edit panel behavior:"));
                                let mut single = layout.single_panel;
                                if crate::widgets::dropdown(ui, "cr-pref-panels", &mut single, &[(false, tr("Multiple")), (true, tr("Single"))], 160.0) {
                                    layout.single_panel = single;
                                    if single && layout.open.len() > 1 {
                                        layout.open.truncate(1);
                                    }
                                }
                            });
                            let note = if layout.single_panel {
                                tl!("Single: opening an edit panel closes the others.")
                            } else {
                                tl!("Multiple: Edit panels will stay open until you close them.")
                            };
                            ui.label(egui::RichText::new(note).color(c.label));
                            off(ui, tr("Use compact layout"), false);
                            off(ui, tr("Show rich tooltips"), true);
                            off(ui, tr("Use legacy Target Adjustment Tool"), false);
                        });
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(tr("Zoom and Pan")).strong());
                            ui.horizontal(|ui| {
                                ui.label(tr("Default image click zooms to:"));
                                ui.add_enabled(false, egui::Button::new("100%"));
                            });
                            off(ui, tr("Use Lightroom style zoom and pan"), false);
                        });
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(tr("Keyboard Shortcuts")).strong());
                            off(ui, tr("Use legacy undo shortcuts"), false);
                        });
                    } else {
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(tr("Performance")).strong());
                            ui.label(tl!("PhotoCraft develops Camera Raw previews on the CPU; the result uses the full-resolution pipeline."));
                        });
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(tr("Camera Raw Cache")).strong());
                            ui.label(tl!("PhotoCraft keeps no Camera Raw cache."));
                        });
                    }
                });
            });
        });
    if !keep {
        *open = false;
    }
    let _ = Color32::WHITE;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_amount_scales_the_change_from_the_base() {
        let base = CameraRaw { exposure: 0.5, ..Default::default() };
        let preset = CameraRaw { contrast: 40.0, monochrome: true, ..Default::default() };
        let full = blend(&base, &preset, 1.0);
        assert_eq!((full.exposure, full.contrast, full.monochrome), (0.5, 40.0, true));
        let half = blend(&base, &preset, 0.5);
        assert_eq!(half.contrast, 20.0);
        let none = blend(&base, &preset, 0.0);
        assert_eq!((none.contrast, none.monochrome), (0.0, false));
        assert_eq!(blend(&base, &preset, 2.0).contrast, 80.0);
    }

    #[test]
    fn builtin_presets_are_valid_settings() {
        for (_, items) in builtin_presets() {
            for (name, p) in items {
                assert!(p.validate().is_ok(), "{name}");
                assert!(!p.is_identity() || p.monochrome, "{name} changes something");
            }
        }
    }
}
