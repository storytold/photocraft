//! The develop view's Masking, Heal and Red Eye panels (LightCraft's `masking.rs` and the Remove /
//! Red Eye panels of its right panel), in its look ([`super::lc`]). They edit the dialog's develop
//! settings; the preview side of each tool is [`super::local`].

use craft_develop_model::controls::{self, ControlSpec, Section, Track};
use craft_develop_model::{DevelopSettings, MaskOp, MaskShape, SpotMode};
use egui::{Rect, Sense, Stroke, pos2, vec2};

use super::develop_panel::tr;
use super::lc::icons::{Icon, paint};
use super::lc::widgets::{self as w, divider, padded, slider, sub_title, text_button, title};
use super::lc::{self, Lc};
use super::local::{self, Draw, LocalTools, kind_label};

/// A slider of the panels' own: `(min, max, default)`, its step and decimals.
const fn spec(id: &'static str, label: &'static str, (min, max, default): (f64, f64, f64), step: f64, decimals: u8, track: Track) -> ControlSpec {
    ControlSpec { id, label, section: Section::Light, min, max, default, step, decimals, track }
}

/// A mask's sliders (LightCraft's local adjustments; key = the `LocalAdjustments` field).
pub(crate) const LOCAL: &[ControlSpec] = &[
    spec("temp", "Temp", (-100.0, 100.0, 0.0), 1.0, 0, Track::Temp),
    spec("tint", "Tint", (-100.0, 100.0, 0.0), 1.0, 0, Track::Tint),
    spec("exposure", "Exposure", (-4.0, 4.0, 0.0), 0.01, 2, Track::Centered),
    spec("contrast", "Contrast", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("highlights", "Highlights", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("shadows", "Shadows", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("whites", "Whites", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("blacks", "Blacks", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("texture", "Texture", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("clarity", "Clarity", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("dehaze", "Dehaze", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("hue", "Hue", (-100.0, 100.0, 0.0), 1.0, 0, Track::Rainbow),
    spec("saturation", "Saturation", (-100.0, 100.0, 0.0), 1.0, 0, Track::Gradient { from: "#7a7a7a", to: "#e04a3a" }),
    spec("sharpness", "Sharpness", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("noise", "Noise", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("moire", "Moiré", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("defringe", "Defringe", (-100.0, 100.0, 0.0), 1.0, 0, Track::Centered),
    spec("amount", "Amount", (0.0, 200.0, 100.0), 1.0, 0, Track::Plain),
];

const REFINE: ControlSpec = spec("refine", "Refine Edges", (0.0, 100.0, 0.0), 1.0, 0, Track::Plain);

fn kind_icon(s: &MaskShape) -> Icon {
    match s {
        MaskShape::Brush { .. } => Icon::Brush,
        MaskShape::Linear { .. } => Icon::Linear,
        MaskShape::Radial { .. } => Icon::Radial,
        MaskShape::ColorRange { .. } => Icon::Picker,
        MaskShape::LuminanceRange { .. } => Icon::Sliders,
        _ => Icon::Mask,
    }
}

/// The Create New Mask tiles: (kind, label, icon). Only the kinds that need no AI model.
const TILES: [(&str, &str, Icon); 5] = [
    ("brush", "Brush", Icon::Brush),
    ("linear", "Linear", Icon::Linear),
    ("radial", "Radial", Icon::Radial),
    ("luminanceRange", "Luminance", Icon::Sliders),
    ("colorRange", "Color", Icon::Picker),
];

/// The menu of component kinds that Add or Subtract offers.
fn component_menu(ui: &mut egui::Ui, st: &mut LocalTools, s: &mut DevelopSettings, op: MaskOp) {
    lc::style(ui);
    for (kind, label) in
        [("brush", "Brush"), ("linear", "Linear Gradient"), ("radial", "Radial Gradient"), ("luminanceRange", "Luminance Range"), ("colorRange", "Color Range")]
    {
        if ui.button(tr(label)).clicked() {
            let _ = local::add_component(st, s, kind, op);
            ui.close();
        }
    }
}

/// The Masking panel.
pub(crate) fn masking(ui: &mut egui::Ui, st: &mut LocalTools, s: &mut DevelopSettings) {
    let t = Lc::get(ui.ctx());
    st.validate(s);
    title(ui, tr("Masking"));
    padded(ui, 0, 10, |ui| {
        ui.label(egui::RichText::new(tr("Create New Mask")).color(t.text_dim));
        ui.add_space(6.0);
        let cols = 4usize;
        let tile_w = ((ui.available_width() - 6.0 * (cols as f32 - 1.0)) / cols as f32).floor().clamp(44.0, 60.0);
        egui::Grid::new("lc-mask-tiles").spacing(vec2(6.0, 6.0)).show(ui, |ui| {
            for (i, (kind, label, icon)) in TILES.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(tile_w, 52.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr(label)));
                ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.hover } else { t.inset });
                paint(ui.painter(), Rect::from_center_size(r.center() - vec2(0.0, 7.0), vec2(20.0, 20.0)), *icon, t.text_label);
                ui.painter().text(pos2(r.center().x, r.bottom() - 9.0), egui::Align2::CENTER_CENTER, tr(label), t.font(10.5), t.text_dim);
                if resp.clicked() {
                    let _ = local::add_mask(st, s, kind);
                }
                if i % cols == cols - 1 {
                    ui.end_row();
                }
            }
        });
    });
    divider(ui);
    // the masks
    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 8, bottom: 8 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("Masks")).font(t.semibold(13.0)).color(t.text_label));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let on = st.overlay;
                if w::icon_button(ui, Icon::Eye, vec2(24.0, 24.0), on, true, tr("Show Overlay")).clicked() {
                    st.overlay = !on;
                }
            });
        });
        if s.masks.is_empty() {
            ui.label(egui::RichText::new(tr("No masks yet. Choose a mask type above.")).color(t.text_dim));
        }
        let mut delete = None;
        for i in 0..s.masks.len() {
            let Some(m) = s.masks.get_mut(i) else { break };
            let sel = st.selected == Some(m.id);
            if let Some((rid, name)) = st.renaming.as_mut().filter(|(rid, _)| *rid == m.id) {
                // rename in place: Return or leaving the field keeps it, Esc doesn't
                let rid = *rid;
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).id_salt(("lc-mask-rename", rid)));
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    st.renaming = None;
                } else if r.lost_focus() {
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        m.name = name;
                    }
                    st.renaming = None;
                }
                continue;
            }
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, sel, &m.name));
            ui.painter().rect_filled(
                r,
                4.0,
                if sel {
                    t.tool_active
                } else if resp.hovered() {
                    t.hover.gamma_multiply(0.7)
                } else {
                    t.chrome
                },
            );
            let icon = m.components.first().map_or(Icon::Mask, |c| kind_icon(&c.shape));
            paint(ui.painter(), Rect::from_min_size(r.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), icon, t.text_label);
            let color = if m.visible { t.text } else { t.text_disabled };
            ui.painter().text(pos2(r.left() + 32.0, r.center().y), egui::Align2::LEFT_CENTER, &m.name, t.font(13.0), color);
            // show / hide on hover, and always while hidden
            if ui.rect_contains_pointer(r) || !m.visible {
                let er = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(22.0, 22.0));
                let eye = ui.interact(er, ui.make_persistent_id(("lc-mask-visible", m.id)), Sense::click());
                eye.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, m.visible, tr("Show Mask")));
                paint(ui.painter(), er.shrink(3.0), if m.visible { Icon::Eye } else { Icon::EyeOff }, if eye.hovered() { t.text } else { t.text_dim });
                if eye.clicked() {
                    m.visible = !m.visible;
                }
            }
            if resp.clicked() {
                st.selected = Some(m.id);
                st.component = 0;
                st.draw = m.components.first().map_or(Draw::None, |c| local::draw_of(&c.shape));
            }
            if resp.double_clicked() {
                st.renaming = Some((m.id, m.name.clone()));
            }
            let (id, name, visible) = (m.id, m.name.clone(), m.visible);
            resp.context_menu(|ui| {
                lc::style(ui);
                if ui.button(tr("Invert Mask")).clicked() {
                    if let Some(m) = s.masks.get_mut(i) {
                        m.invert = !m.invert;
                    }
                    ui.close();
                }
                if ui.button(tr(if visible { "Hide Mask" } else { "Show Mask" })).clicked() {
                    if let Some(m) = s.masks.get_mut(i) {
                        m.visible = !m.visible;
                    }
                    ui.close();
                }
                if ui.button(tr("Rename…")).clicked() {
                    st.renaming = Some((id, name.clone()));
                    ui.close();
                }
                if ui.button(tr("Delete Mask")).clicked() {
                    delete = Some(id);
                    ui.close();
                }
            });
        }
        if let Some(id) = delete {
            s.masks.retain(|m| m.id != id);
            st.validate(s);
        }
    });
    let Some(id) = st.selected else {
        // the Brush armed: its settings, for the stroke that makes the mask
        if st.armed() {
            divider(ui);
            brush_settings(ui, st);
        }
        return;
    };
    if !s.masks.iter().any(|m| m.id == id) {
        return;
    }
    divider(ui);
    mask_components(ui, st, s);
    if st.draw == Draw::Brush {
        divider(ui);
        brush_settings(ui, st);
    }
    divider(ui);
    let Some(m) = local::selected_mask(st, s) else { return };
    for spec in LOCAL {
        let Some(v) = local::local_field(&mut m.adjust, spec.id) else { continue };
        let out = ui.push_id(("lc-local", spec.id), |ui| slider(ui, spec, *v, true, tr(spec.label))).inner;
        if let Some(nv) = out.value {
            *v = nv;
        }
    }
    let out = slider(ui, &REFINE, m.refine, true, tr(REFINE.label));
    if let Some(v) = out.value {
        m.refine = v;
    }
    ui.add_space(30.0);
}

/// The selected mask's components, the selected one's own controls, and Add / Subtract / Invert /
/// Delete.
fn mask_components(ui: &mut egui::Ui, st: &mut LocalTools, s: &mut DevelopSettings) {
    let t = Lc::get(ui.ctx());
    let comp = st.component;
    let mut delete_mask = false;
    let mut remove_comp = None;
    padded(ui, 8, 8, |ui| {
        let Some(m) = local::selected_mask(st, s) else { return };
        let n = m.components.len();
        for (i, c) in m.components.iter_mut().enumerate() {
            let sel = i == comp;
            let op = match c.op {
                MaskOp::Add => "",
                MaskOp::Subtract => "− ",
                MaskOp::Intersect => "∩ ",
            };
            let label = c.name.clone().unwrap_or_else(|| tr(kind_label(&c.shape)).to_string());
            let text = format!("{op}{label}{}", if c.invert { tr(" (inverted)") } else { "" });
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, sel, &text));
            if sel && n > 1 {
                ui.painter().rect_filled(r, 3.0, t.tool_active.gamma_multiply(0.7));
            }
            paint(ui.painter(), Rect::from_min_size(r.min + vec2(2.0, 5.0), vec2(16.0, 16.0)), kind_icon(&c.shape), t.text_label);
            ui.painter().text(pos2(r.left() + 26.0, r.center().y), egui::Align2::LEFT_CENTER, &text, t.font(13.5), t.text);
            if resp.clicked() {
                st.component = i;
                st.draw = local::draw_of(&c.shape);
            }
            resp.context_menu(|ui| {
                lc::style(ui);
                if ui.button(tr("Invert Component")).clicked() {
                    c.invert = !c.invert;
                    ui.close();
                }
                if n > 1 && ui.button(tr("Delete Component")).clicked() {
                    remove_comp = Some(i);
                    ui.close();
                }
            });
        }
    });
    let draw_now = st.draw;
    if let Some(c) = local::selected_mask(st, s).and_then(|m| m.components.get_mut(comp)) {
        component_controls(ui, draw_now, &mut c.shape, &mut st.draw);
    }
    if let Some(i) = remove_comp
        && let Some(m) = local::selected_mask(st, s)
        && m.components.len() > 1
        && i < m.components.len()
    {
        m.components.remove(i);
        st.component = 0;
    }
    padded(ui, 0, 10, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            let add = text_button(ui, tr("Add"), false).on_hover_text(tr("Add to this mask"));
            egui::Popup::menu(&add).show(|ui| component_menu(ui, st, s, MaskOp::Add));
            let sub = text_button(ui, tr("Subtract"), false).on_hover_text(tr("Subtract from this mask"));
            egui::Popup::menu(&sub).show(|ui| component_menu(ui, st, s, MaskOp::Subtract));
            let inverted = local::selected_mask(st, s).is_some_and(|m| m.invert);
            if text_button(ui, tr("Invert"), inverted).clicked()
                && let Some(m) = local::selected_mask(st, s)
            {
                m.invert = !m.invert;
            }
            if w::icon_button(ui, Icon::Trash, vec2(26.0, 24.0), false, true, tr("Delete Mask")).clicked() {
                delete_mask = true;
            }
        });
    });
    if delete_mask && let Some(id) = st.selected {
        s.masks.retain(|m| m.id != id);
        st.validate(s);
    }
}

/// The selected component's own controls: a radial's Feather, a luminance range's range and
/// Smoothness, a colour range's samples, Pick and Refine.
fn component_controls(ui: &mut egui::Ui, draw_now: Draw, shape: &mut MaskShape, draw: &mut Draw) {
    let t = Lc::get(ui.ctx());
    match shape {
        MaskShape::Radial { feather, .. } => {
            let spec = spec("radialFeather", "Feather", (0.0, 100.0, 50.0), 1.0, 0, Track::Plain);
            if let Some(v) = row(ui, &spec, *feather) {
                *feather = v;
            }
        }
        MaskShape::LuminanceRange { lo, hi, lo_feather, hi_feather } => {
            padded(ui, 0, 4, |ui| luminance_bar(ui, lo, hi));
            let smooth = spec("smoothness", "Smoothness", (0.0, 100.0, 20.0), 1.0, 0, Track::Plain);
            let cur = ((*lo_feather + *hi_feather) / 2.0 / 0.5 * 100.0).clamp(0.0, 100.0);
            if let Some(v) = row(ui, &smooth, cur) {
                let f = v / 100.0 * 0.5;
                (*lo_feather, *hi_feather) = (f, f);
            }
        }
        MaskShape::ColorRange { samples, refine } => {
            padded(ui, 0, 4, |ui| {
                ui.horizontal(|ui| {
                    let n = samples.len();
                    ui.label(egui::RichText::new(crate::i18n::fmt(tr("{n} samples"), &[("n", &n.to_string())])).color(t.text_dim).size(11.5));
                    let picking = draw_now == Draw::ColorPick;
                    if text_button(ui, tr("Pick"), picking).on_hover_text(tr("Click the photo to pick a color; Shift-click adds more (up to 5)")).clicked() {
                        *draw = if picking { Draw::None } else { Draw::ColorPick };
                    }
                });
            });
            let spec = spec("colorRefine", "Refine", (0.0, 100.0, 50.0), 1.0, 0, Track::Plain);
            if let Some(v) = row(ui, &spec, *refine) {
                *refine = v;
            }
        }
        MaskShape::Brush { strokes } => {
            padded(ui, 0, 4, |ui| {
                ui.label(egui::RichText::new(crate::i18n::fmt(tr("{n} strokes"), &[("n", &strokes.len().to_string())])).color(t.text_dim).size(11.5));
            });
        }
        _ => {}
    }
}

/// A luminance range's two handles over a dark → light bar.
fn luminance_bar(ui: &mut egui::Ui, lo: &mut f64, hi: &mut f64) {
    let t = Lc::get(ui.ctx());
    ui.label(egui::RichText::new(tr("Select Luminance Range")).color(t.text_dim).size(11.5));
    let width = ui.available_width().min(240.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::slider(true, *lo, tr("Select Luminance Range")));
    let p = ui.painter();
    for k in 0..24 {
        let a = k as f32 / 24.0;
        let r = Rect::from_min_max(pos2(rect.left() + a * width, rect.top() + 4.0), pos2(rect.left() + (a + 1.0 / 24.0) * width + 0.5, rect.bottom() - 4.0));
        p.rect_filled(r, 0.0, egui::Color32::from_gray((a * 255.0) as u8));
    }
    let x = |v: f64| rect.left() + v.clamp(0.0, 1.0) as f32 * width;
    p.rect_stroke(
        Rect::from_min_max(pos2(x(*lo), rect.top() + 2.0), pos2(x(*hi), rect.bottom() - 2.0)),
        2.0,
        Stroke::new(1.5, t.accent),
        egui::StrokeKind::Middle,
    );
    for v in [*lo, *hi] {
        p.circle_filled(pos2(x(v), rect.center().y), 5.0, egui::Color32::WHITE);
        p.circle_stroke(pos2(x(v), rect.center().y), 5.0, Stroke::new(1.0, egui::Color32::from_gray(40)));
    }
    // the handle nearer the press moves, for the whole drag
    let handle_id = resp.id.with("handle");
    if resp.drag_started()
        && let Some(o) = ui.input(|i| i.pointer.press_origin())
    {
        let low = (o.x - x(*lo)).abs() <= (o.x - x(*hi)).abs();
        ui.data_mut(|d| d.insert_temp(handle_id, low));
    }
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let v = f64::from((pos.x - rect.left()) / width.max(1.0)).clamp(0.0, 1.0);
        let nearer = (pos.x - x(*lo)).abs() <= (pos.x - x(*hi)).abs();
        let low = if resp.dragged() { ui.data(|d| d.get_temp::<bool>(handle_id)).unwrap_or(nearer) } else { nearer };
        if low {
            *lo = v.min((*hi - 0.01).max(0.0));
        } else {
            *hi = v.max((*lo + 0.01).min(1.0));
        }
    }
}

/// A slider row of the panel's own (not a registry control).
fn row(ui: &mut egui::Ui, spec: &ControlSpec, v: f64) -> Option<f64> {
    ui.push_id(spec.id, |ui| slider(ui, spec, v, true, tr(spec.label))).inner.value
}

/// The brush's Add / Erase, Size, Feather, Flow and Auto Mask.
fn brush_settings(ui: &mut egui::Ui, st: &mut LocalTools) {
    let t = Lc::get(ui.ctx());
    padded(ui, 6, 0, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("Brush")).font(t.semibold(13.0)));
            if text_button(ui, tr("Add"), !st.brush.erase).clicked() {
                st.brush.erase = false;
            }
            if text_button(ui, tr("Erase"), st.brush.erase).on_hover_text(tr("Erase (or hold Alt while painting)")).clicked() {
                st.brush.erase = true;
            }
        });
    });
    let b = &mut st.brush;
    let size = spec("brushSize", "Size", (1.0, 100.0, 12.0), 1.0, 0, Track::Plain);
    if let Some(v) = slider(ui, &size, b.size * 400.0, true, tr("Size")).value {
        b.size = (v / 400.0).clamp(0.0025, 0.25);
    }
    let feather = spec("brushFeather", "Feather", (0.0, 100.0, 50.0), 1.0, 0, Track::Plain);
    if let Some(v) = slider(ui, &feather, b.feather, true, tr("Feather")).value {
        b.feather = v;
    }
    let flow = spec("brushFlow", "Flow", (1.0, 100.0, 100.0), 1.0, 0, Track::Plain);
    if let Some(v) = slider(ui, &flow, b.flow, true, tr("Flow")).value {
        b.flow = v;
    }
    padded(ui, 2, 4, |ui| {
        ui.checkbox(&mut b.auto_mask, tr("Auto Mask")).on_hover_text(tr("Paint only areas like the one under the brush's centre, stopping at edges"));
    });
}

/// The Heal panel: the mode for new spots, and Size / Feather / Opacity of the selected spot (or
/// of the next one), Delete and Clear All.
pub(crate) fn heal(ui: &mut egui::Ui, st: &mut LocalTools, s: &mut DevelopSettings) {
    let t = Lc::get(ui.ctx());
    st.validate(s);
    title(ui, tr("Heal"));
    let modes = [(SpotMode::Heal, "Heal"), (SpotMode::Clone, "Clone"), (SpotMode::Remove, "Remove")];
    let labels: Vec<&str> = modes.iter().map(|(_, l)| tr(l)).collect();
    let current = st.spot.and_then(|i| s.spots.get(i)).map_or(st.heal.mode, |sp| sp.mode);
    padded(ui, 8, 8, |ui| {
        if let Some(i) = w::segmented(ui, &labels, modes.iter().position(|(m, _)| *m == current))
            && let Some((mode, _)) = modes.get(i)
        {
            st.heal.mode = *mode;
            if let Some(sp) = st.spot.and_then(|i| s.spots.get_mut(i)) {
                sp.mode = *mode;
                // Remove finds its own source; Heal and Clone keep (or get) one beside the spot
                sp.source_offset = match mode {
                    SpotMode::Remove | SpotMode::Fill => None,
                    _ => sp.source_offset.or(Some(craft_geom::Point::new(2.5 * sp.size, 0.0))),
                };
            }
        }
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(tr("Click the photo to fix a spot; drag to brush along a line. Drag a spot or its source to move it."))
                .color(t.text_dim)
                .size(11.5),
        );
    });
    if let Some(i) = st.spot {
        sub_title(ui, &crate::i18n::fmt(tr("Spot {index} of {n}"), &[("index", &(i + 1).to_string()), ("n", &s.spots.len().to_string())]));
    }
    let size = spec("spotSize", "Size", (1.0, 250.0, 20.0), 1.0, 0, Track::Plain);
    let feather = spec("spotFeather", "Feather", (0.0, 100.0, 50.0), 1.0, 0, Track::Plain);
    let opacity = spec("spotOpacity", "Opacity", (0.0, 100.0, 100.0), 1.0, 0, Track::Plain);
    let (sz, fe, op) = match st.spot.and_then(|i| s.spots.get(i)) {
        Some(sp) => (sp.size, sp.feather, sp.opacity),
        None => (st.heal.size, st.heal.feather, st.heal.opacity),
    };
    let new_size = slider(ui, &size, sz * 1000.0, true, tr("Size")).value.map(|v| (v / 1000.0).clamp(0.001, 0.25));
    let new_feather = slider(ui, &feather, fe, true, tr("Feather")).value;
    let new_opacity = slider(ui, &opacity, op, true, tr("Opacity")).value;
    let sp = st.spot.and_then(|i| s.spots.get_mut(i));
    for (new, heal_v) in [(new_size, &mut st.heal.size), (new_feather, &mut st.heal.feather), (new_opacity, &mut st.heal.opacity)] {
        if let Some(v) = new {
            *heal_v = v;
        }
    }
    if let Some(sp) = sp {
        if let Some(v) = new_size {
            sp.size = v;
        }
        if let Some(v) = new_feather {
            sp.feather = v;
        }
        if let Some(v) = new_opacity {
            sp.opacity = v;
        }
    }
    padded(ui, 8, 10, |ui| {
        ui.label(egui::RichText::new(crate::i18n::fmt(tr("{n} spots on this photo"), &[("n", &s.spots.len().to_string())])).color(t.text_dim));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_enabled_ui(st.spot.is_some(), |ui| {
                if text_button(ui, tr("Delete Spot"), false).clicked()
                    && let Some(i) = st.spot.take()
                    && i < s.spots.len()
                {
                    s.spots.remove(i);
                }
            });
            ui.add_enabled_ui(!s.spots.is_empty(), |ui| {
                if text_button(ui, tr("Clear All"), false).clicked() {
                    s.spots.clear();
                    st.spot = None;
                }
            });
        });
    });
}

/// The Red Eye panel: Red Eye / Pet Eye for new eyes, the selected eye's registry controls
/// (`redEye.<i>.*`), Delete and Clear All.
pub(crate) fn red_eye(ui: &mut egui::Ui, st: &mut LocalTools, s: &mut DevelopSettings) {
    let t = Lc::get(ui.ctx());
    st.validate(s);
    title(ui, tr("Red Eye"));
    padded(ui, 8, 8, |ui| {
        let labels = [tr("Red Eye"), tr("Pet Eye")];
        let pet = st.eye.and_then(|i| s.red_eye.get(i)).map_or(st.pet, |e| e.pet);
        if let Some(i) = w::segmented(ui, &labels, Some(usize::from(pet))) {
            st.pet = i == 1;
            if let Some(e) = st.eye.and_then(|i| s.red_eye.get_mut(i)) {
                e.pet = st.pet;
            }
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new(tr("Click an eye on the photo, or drag from its centre to size the ellipse round it.")).color(t.text_dim).size(11.5));
    });
    if let Some(i) = st.eye.filter(|i| *i < s.red_eye.len()) {
        sub_title(ui, &crate::i18n::fmt(tr("Eye {index} of {n}"), &[("index", &(i + 1).to_string()), ("n", &s.red_eye.len().to_string())]));
        for spec in controls::INDEXED.iter().filter(|c| c.section == Section::RedEye) {
            let Some(field) = spec.id.split_once('.').map(|(_, f)| f) else { continue };
            let id = format!("redEye.{i}.{field}");
            let Some(v) = controls::get(s, &id) else { continue };
            let out = ui.push_id(("lc-eye", spec.id), |ui| slider(ui, spec, v, true, tr(spec.label))).inner;
            if let Some(nv) = out.value {
                controls::set(s, &id, nv);
            }
        }
    }
    padded(ui, 8, 10, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_enabled_ui(st.eye.is_some(), |ui| {
                if text_button(ui, tr("Delete Eye"), false).clicked()
                    && let Some(i) = st.eye.take()
                    && i < s.red_eye.len()
                {
                    s.red_eye.remove(i);
                }
            });
            ui.add_enabled_ui(!s.red_eye.is_empty(), |ui| {
                if text_button(ui, tr("Clear All"), false).clicked() {
                    s.red_eye.clear();
                    st.eye = None;
                }
            });
        });
    });
}
