//! Camera Raw's own look, measured on Photoshop 25.4's Camera Raw 16.1.1 (Filter › Camera Raw
//! Filter at 100 % on the Medium Gray interface): 38 pt panel rows split by 2 pt rules, 30 pt
//! slider rows with a 3 pt track and a house-shaped knob 12 × 11 pt, value boxes 42 × 17 pt
//! with a 1 pt border, 21 pt / 19 pt side insets. Colours derive from the theme (the measured
//! dialog follows Photoshop's interface brightness the same way), so the dialog matches every
//! PhotoCraft theme. Only behaviour and look are copied; drawing is our own.

use egui::{Align2, Color32, CursorIcon, FontId, Key, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::Tokens;

/// Height of a panel header row, without its rule.
pub(crate) const ROW: f32 = 38.0;
/// The rule between rows.
pub(crate) const RULE: f32 = 2.0;
/// One slider row: label and value box, then the track.
pub(crate) const SLIDER_ROW: f32 = 30.0;
/// Left inset of labels and tracks, right inset of value boxes.
pub(crate) const INSET_L: f32 = 21.0;
pub(crate) const INSET_R: f32 = 19.0;
pub(crate) const FIELD_W: f32 = 42.0;
pub(crate) const FIELD_H: f32 = 17.0;

/// The dialog's colours for the current theme.
#[derive(Clone, Copy)]
pub(crate) struct Acr {
    /// Panels, title bar, footer (Camera Raw's 83 on Medium Gray).
    pub base: Color32,
    /// Rules between rows and the histogram ground (52).
    pub rule: Color32,
    /// The preview's surround (40).
    pub well: Color32,
    pub field: Color32,
    pub field_border: Color32,
    /// Sub-group boxes (Vignette, Grain, Sharpening, …): a step darker than the panel.
    pub group: Color32,
    pub text: Color32,
    pub label: Color32,
    pub faint: Color32,
    pub knob: Color32,
    pub knob_edge: Color32,
    pub accent: Color32,
    pub hover: Color32,
}

impl Acr {
    pub(crate) fn get(ctx: &egui::Context) -> Self {
        let t = Tokens::get(ctx);
        let base = t.card;
        let dark = t.dark();
        let mix = |a: Color32, b: Color32, f: f32| {
            let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * f).round().clamp(0.0, 255.0) as u8;
            Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
        };
        Acr {
            base,
            rule: mix(base, Color32::BLACK, 0.37),
            well: t.canvas,
            field: t.field,
            field_border: t.field_border,
            group: mix(base, Color32::BLACK, if dark { 0.14 } else { 0.08 }),
            text: t.text,
            label: t.text_dim,
            faint: t.text_faint,
            knob: if dark { Color32::from_gray(210) } else { Color32::from_gray(250) },
            knob_edge: Color32::from_gray(33),
            accent: t.accent,
            hover: t.hover,
        }
    }
}

pub(crate) fn title_font() -> FontId {
    crate::theme::semibold(13.0)
}
pub(crate) fn label_font() -> FontId {
    // Camera Raw's labels are 12 px Adobe Clean; Inter at 11.5 has the same width.
    FontId::proportional(11.5)
}
pub(crate) fn small_font() -> FontId {
    FontId::proportional(11.0)
}

/// A full-width rule (2 pt) at the cursor.
pub(crate) fn rule(ui: &mut Ui) {
    let c = Acr::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), RULE), Sense::hover());
    ui.painter().rect_filled(r, 0.0, c.rule);
}

/// Panel visibility, as the eye at the right of a panel header: bright while the panel has
/// edits, dim without; a click hides (or shows again) the panel's edits in the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Eye {
    None,
    Shown { edited: bool },
    Hidden,
}

/// What a panel header did this frame.
#[derive(Default)]
pub(crate) struct HeaderResponse {
    pub toggled: bool,
    pub eye_clicked: bool,
    /// Alt-click on the title: Camera Raw's "solo" (open this panel, close the others).
    pub solo: bool,
}

/// A panel header row: chevron, title, optional badge ("Early Access") and eye. Clicking the
/// row opens or closes the panel.
pub(crate) fn panel_header(ui: &mut Ui, id: &str, title: &str, open: bool, badge: Option<&str>, eye: Eye) -> HeaderResponse {
    let c = Acr::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, ui.is_enabled(), open, title));
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, 0.0, c.hover.gamma_multiply(0.25));
    }
    let chevron = pos2(rect.left() + 27.0, rect.center().y);
    chevron_glyph(p, chevron, open, c.text);
    p.text(pos2(rect.left() + 42.0, rect.center().y), Align2::LEFT_CENTER, title, title_font(), c.text);
    let mut out = HeaderResponse::default();
    let eye_rect = Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(20.0, 20.0));
    if let Some(badge) = badge {
        let g = p.layout_no_wrap(badge.to_owned(), small_font(), if open { Color32::WHITE } else { c.faint });
        let b = Rect::from_min_size(pos2(eye_rect.left() - 10.0 - g.size().x - 10.0, rect.center().y - 10.0), vec2(g.size().x + 10.0, 20.0));
        p.rect_filled(b, 2.0, if open { c.accent } else { c.rule.gamma_multiply(0.6) });
        p.galley(b.center() - g.size() * 0.5, g, c.text);
    }
    if eye != Eye::None {
        let er = ui.interact(eye_rect, ui.id().with((id, "eye")), Sense::click());
        let tip = crate::i18n::fmt(tl!("Toggle visibility of {panel} settings"), &[("panel", title)]);
        er.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, eye != Eye::Hidden, &tip));
        let er = er.on_hover_text(tip);
        let (name, tint) = match eye {
            Eye::Hidden => ("eye-off", c.text),
            Eye::Shown { edited: true } => ("eye", c.text),
            _ => ("eye", c.faint.gamma_multiply(0.7)),
        };
        crate::icons::paint(ui, eye_rect, name, 15.0, tint);
        if er.clicked() {
            out.eye_clicked = true;
        }
        if er.clicked() || er.hovered() {
            // The eye takes its own clicks.
            if resp.clicked() && er.clicked() {
                return out;
            }
        }
    }
    if resp.clicked() {
        if ui.input(|i| i.modifiers.alt) {
            out.solo = true;
        } else {
            out.toggled = true;
        }
    }
    out
}

/// The panel chevron: › closed, ⌄ open.
pub(crate) fn chevron_glyph(p: &egui::Painter, at: Pos2, open: bool, color: Color32) {
    let s = Stroke::new(1.8, color);
    if open {
        p.line_segment([at + vec2(-4.5, -2.0), at + vec2(0.0, 2.5)], s);
        p.line_segment([at + vec2(0.0, 2.5), at + vec2(4.5, -2.0)], s);
    } else {
        p.line_segment([at + vec2(-2.0, -4.5), at + vec2(2.5, 0.0)], s);
        p.line_segment([at + vec2(2.5, 0.0), at + vec2(-2.0, 4.5)], s);
    }
}

/// The small filled triangle that opens a sub-group: ◀ closed, ▼ open.
pub(crate) fn group_triangle(p: &egui::Painter, center: Pos2, open: bool, color: Color32) {
    let pts = if open {
        vec![center + vec2(-5.0, -3.0), center + vec2(5.0, -3.0), center + vec2(0.0, 4.0)]
    } else {
        vec![center + vec2(4.0, -5.0), center + vec2(4.0, 5.0), center + vec2(-3.0, 0.0)]
    };
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Track painting of a slider.
#[derive(Clone)]
pub(crate) enum Track {
    /// Camera Raw's gray ramp (Light and Effects sliders): dark left, light right.
    Tone,
    /// A colour ramp (Temperature, Tint, hue bands, …), evenly spaced stops.
    Colors(Vec<Color32>),
    /// Starts at the left end (amounts with a 0 minimum), same ramp as Tone.
    Amount,
}

/// How a slider shows and edits its number.
#[derive(Clone)]
pub(crate) struct Spec {
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub decimals: u32,
    /// Show "+" for positive values (centred sliders), as Camera Raw does.
    pub signed: bool,
    pub track: Track,
    pub enabled: bool,
}

impl Spec {
    pub(crate) fn centred(limit: f32) -> Self {
        Spec { min: -limit, max: limit, default: 0.0, decimals: 0, signed: true, track: Track::Tone, enabled: true }
    }
    pub(crate) fn amount(max: f32, default: f32) -> Self {
        Spec { min: 0.0, max, default, decimals: 0, signed: false, track: Track::Amount, enabled: true }
    }
    pub(crate) fn range(min: f32, max: f32, default: f32) -> Self {
        Spec { min, max, default, decimals: 0, signed: min < 0.0, track: if min < 0.0 { Track::Tone } else { Track::Amount }, enabled: true }
    }
    pub(crate) fn decimals(mut self, d: u32) -> Self {
        self.decimals = d;
        self
    }
    pub(crate) fn colors(mut self, stops: Vec<Color32>) -> Self {
        self.track = Track::Colors(stops);
        self
    }
    pub(crate) fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    fn text(&self, v: f32) -> String {
        let v = if v.is_finite() { v } else { self.default };
        let s = format!("{:.*}", self.decimals as usize, v);
        let zero = s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.');
        if zero {
            return format!("{:.*}", self.decimals as usize, 0.0);
        }
        if self.signed && v > 0.0 { format!("+{s}") } else { s }
    }
}

/// What a slider row did.
pub(crate) struct SliderResponse {
    pub changed: bool,
    /// The knob is being dragged with Alt held (clipping diagnostics for tone sliders).
    pub alt_drag: bool,
}

/// One Camera Raw slider row: `Label ……… [value]` with the track and knob below it. Drag or
/// click the track to set, double-click the knob or the label to reset, type in the box (Enter /
/// Tab commit, Esc reverts, ↑/↓ step by one, Shift ×10).
pub(crate) fn slider(ui: &mut Ui, id: &str, label: &str, value: &mut f32, spec: &Spec) -> SliderResponse {
    let c = Acr::get(ui.ctx());
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, SLIDER_ROW), Sense::hover());
    let enabled = spec.enabled && ui.is_enabled();
    let text_color = if enabled { c.label } else { c.faint.gamma_multiply(0.6) };
    let label_pos = pos2(row.left() + INSET_L + 2.0, row.top() + 9.0);
    let label_rect = Rect::from_min_size(row.min + vec2(INSET_L, 0.0), vec2((width - INSET_L - INSET_R - FIELD_W - 6.0).max(10.0), 18.0));
    let label_resp = ui.interact(label_rect, ui.id().with((id, "label")), if enabled { Sense::click() } else { Sense::hover() });
    ui.painter().text(label_pos, Align2::LEFT_CENTER, label, label_font(), text_color);
    let field = Rect::from_min_size(pos2(row.right() - INSET_R - FIELD_W, row.top()), vec2(FIELD_W, FIELD_H));
    let mut changed = value_box(ui, ui.id().with((id, "field")), field, value, spec, enabled);
    let track = Rect::from_min_max(pos2(row.left() + INSET_L, row.top() + 21.0), pos2(row.right() - INSET_R, row.top() + 24.0));
    let hit = Rect::from_min_max(pos2(track.left() - 6.0, track.top() - 7.0), pos2(track.right() + 6.0, track.bottom() + 6.0));
    let resp = ui.interact(hit, ui.id().with((id, "track")), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::slider(enabled, *value as f64, label));
    let span = (spec.max - spec.min).max(f32::EPSILON);
    if enabled
        && (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let f = ((p.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
        let step = 10f32.powi(spec.decimals as i32);
        let nv = ((spec.min + f * span) * step).round() / step;
        if nv != *value {
            *value = nv;
            changed = true;
        }
    }
    if enabled && (resp.double_clicked() || label_resp.double_clicked()) && *value != spec.default {
        *value = spec.default;
        changed = true;
    }
    let p = ui.painter();
    paint_track(p, track, &spec.track, enabled, &c);
    let f = ((*value - spec.min) / span).clamp(0.0, 1.0);
    let x = track.left() + f * track.width();
    knob(p, pos2(x, track.top() - 3.0), if enabled { c.knob } else { c.knob.gamma_multiply(0.55) }, c.knob_edge);
    SliderResponse { changed, alt_drag: resp.dragged() && ui.input(|i| i.modifiers.alt) }
}

/// A track and knob without label or value box (luminance under the 3-way colour wheels).
/// `label` names it for accessibility; double-click resets.
pub(crate) fn bare_slider(ui: &mut Ui, id: &str, label: &str, value: &mut f32, spec: &Spec) -> SliderResponse {
    let c = Acr::get(ui.ctx());
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, 16.0), Sense::hover());
    let track = Rect::from_min_max(pos2(row.left() + width * 0.2, row.top() + 4.0), pos2(row.right() - width * 0.2, row.top() + 7.0));
    let resp = ui.interact(track.expand2(vec2(6.0, 7.0)), ui.id().with((id, "bare")), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::slider(true, *value as f64, label));
    let span = (spec.max - spec.min).max(f32::EPSILON);
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let f = ((p.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
        let nv = (spec.min + f * span).round();
        if nv != *value {
            *value = nv;
            changed = true;
        }
    }
    if resp.double_clicked() && *value != spec.default {
        *value = spec.default;
        changed = true;
    }
    paint_track(ui.painter(), track, &spec.track, true, &c);
    let f = ((*value - spec.min) / span).clamp(0.0, 1.0);
    knob(ui.painter(), pos2(track.left() + f * track.width(), track.top() - 3.0), c.knob, c.knob_edge);
    SliderResponse { changed, alt_drag: false }
}

fn paint_track(p: &egui::Painter, r: Rect, track: &Track, enabled: bool, c: &Acr) {
    let dim = |col: Color32| if enabled { col } else { col.gamma_multiply(0.45) };
    let stops: Vec<Color32> = match track {
        Track::Tone | Track::Amount => {
            let lo = c.base.r().saturating_add(7);
            vec![Color32::from_gray(lo), Color32::from_gray(132.max(lo)), Color32::from_gray(232)]
        }
        Track::Colors(v) if v.len() >= 2 => v.clone(),
        Track::Colors(_) => vec![c.label, c.label],
    };
    let n = (stops.len() - 1) as f32;
    let mut mesh = egui::Mesh::default();
    for (i, s) in stops.iter().enumerate() {
        let x = r.left() + r.width() * i as f32 / n;
        mesh.colored_vertex(pos2(x, r.top()), dim(*s));
        mesh.colored_vertex(pos2(x, r.bottom()), dim(*s));
        if i > 0 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    p.add(mesh);
}

/// Camera Raw's knob: a house shape (apex up) 12 × 11 pt with a dark 1 pt edge; `top` is the apex.
pub(crate) fn knob(p: &egui::Painter, top: Pos2, fill: Color32, edge: Color32) {
    let pts = vec![top, top + vec2(6.0, 6.0), top + vec2(6.0, 10.0), top + vec2(-6.0, 10.0), top + vec2(-6.0, 6.0)];
    p.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(1.0, edge)));
}

/// The value box of a slider row. Returns whether the value changed.
fn value_box(ui: &mut Ui, id: egui::Id, rect: Rect, value: &mut f32, spec: &Spec, enabled: bool) -> bool {
    let c = Acr::get(ui.ctx());
    let editing_id = id.with("text");
    let mut text: Option<String> = ui.data(|d| d.get_temp(editing_id));
    let mut changed = false;
    let p = ui.painter();
    p.rect_filled(rect, 0.0, if enabled { c.field } else { c.base });
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, if enabled { c.field_border } else { c.field_border.gamma_multiply(0.5) }), StrokeKind::Inside);
    if !enabled {
        return false;
    }
    match text.as_mut() {
        Some(buffer) => {
            let edit = egui::TextEdit::singleline(buffer)
                .id(editing_id)
                .frame(egui::Frame::NONE)
                .font(label_font())
                .margin(vec2(3.0, 1.0))
                .desired_width(rect.width());
            let r = ui.put(rect, edit);
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            let (enter, tab, esc, up, down, shift) = ui.input(|i| {
                (
                    i.key_pressed(Key::Enter),
                    i.key_pressed(Key::Tab),
                    i.key_pressed(Key::Escape),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.modifiers.shift,
                )
            });
            if up || down {
                let step = if shift { 10.0 } else { 1.0 } * if spec.decimals > 0 { 0.1f32.powi(spec.decimals as i32 - 1).min(1.0) * 0.1 } else { 1.0 };
                let nv = (*value + if up { step } else { -step }).clamp(spec.min, spec.max);
                if nv != *value {
                    *value = nv;
                    changed = true;
                }
                *buffer = spec.text(*value);
            }
            if esc {
                ui.data_mut(|d| d.remove::<String>(editing_id));
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
                return changed;
            }
            if enter || tab {
                // The box takes its own Enter / Tab: they must not reach the dialog (OK).
                ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, Key::Enter);
                    i.consume_key(egui::Modifiers::NONE, Key::Tab);
                });
            }
            if enter || tab || r.lost_focus() {
                if let Some(v) = crate::widgets::parse_num(buffer.trim_start_matches('+')).map(|v| v as f32).filter(|v| v.is_finite()) {
                    let nv = v.clamp(spec.min, spec.max);
                    if nv != *value {
                        *value = nv;
                        changed = true;
                    }
                }
                ui.data_mut(|d| d.remove::<String>(editing_id));
            } else {
                ui.data_mut(|d| d.insert_temp(editing_id, buffer.clone()));
            }
        }
        None => {
            let r = ui.interact(rect, id, Sense::click()).on_hover_cursor(CursorIcon::Text);
            p.text(rect.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, spec.text(*value), label_font(), c.text);
            if r.clicked() {
                ui.data_mut(|d| d.insert_temp(editing_id, spec.text(*value)));
                ui.ctx().request_repaint();
            }
        }
    }
    changed
}

/// A sub-group (Vignette, Grain, Sharpening, Noise Reduction, Defringe, Manual
/// Transformations): its first row carries a triangle that opens a darker box with the
/// remaining controls. `head` draws the first row (usually the amount slider) and returns
/// whether its settings changed.
pub(crate) fn group(ui: &mut Ui, id: &str, open: &mut bool, head: impl FnOnce(&mut Ui) -> bool, body: impl FnOnce(&mut Ui) -> bool) -> bool {
    let c = Acr::get(ui.ctx());
    let top = ui.cursor().top();
    let full = ui.available_width();
    // While open the box spans the head too; paint it behind both afterwards.
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let mut changed = false;
    // The head row keeps room on the right for the triangle.
    let head_rect = Rect::from_min_size(pos2(ui.cursor().left(), top), vec2(full, SLIDER_ROW));
    let mut head_ui = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(head_rect.min, vec2(full - 18.0, SLIDER_ROW))));
    changed |= head(&mut head_ui);
    ui.allocate_exact_size(vec2(full, SLIDER_ROW), Sense::hover());
    let tri = Rect::from_center_size(pos2(head_rect.right() - 13.0, head_rect.top() + 9.0), vec2(16.0, 16.0));
    let tr = ui.interact(tri, ui.id().with((id, "group")), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    tr.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, ui.is_enabled(), *open, id));
    if tr.clicked() {
        *open = !*open;
    }
    group_triangle(ui.painter(), tri.center(), *open, c.text);
    if *open {
        changed |= body(ui);
        ui.add_space(6.0);
        let bottom = ui.cursor().top();
        let rect = Rect::from_min_max(pos2(ui.max_rect().left() + 12.0, top - 4.0), pos2(ui.max_rect().right() - 10.0, bottom));
        ui.painter().set(bg_idx, egui::Shape::rect_filled(rect, 4.0, c.group));
        ui.add_space(4.0);
    }
    changed
}

/// A boxed group header with an icon (Defringe, Manual Transformations): `icon  Title  ◀`.
pub(crate) fn boxed_group(ui: &mut Ui, _id: &str, icon: &str, title: &str, open: &mut bool, body: impl FnOnce(&mut Ui) -> bool) -> bool {
    let c = Acr::get(ui.ctx());
    ui.add_space(4.0);
    let top = ui.cursor().top();
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, ui.is_enabled(), *open, title));
    if resp.clicked() {
        *open = !*open;
    }
    let p = ui.painter();
    let icon_rect = Rect::from_center_size(pos2(row.left() + 30.0, row.center().y), vec2(18.0, 18.0));
    crate::icons::paint(ui, icon_rect, icon, 15.0, c.label);
    p.text(pos2(row.left() + 46.0, row.center().y), Align2::LEFT_CENTER, title, label_font(), c.text);
    group_triangle(p, pos2(row.right() - 22.0, row.center().y), *open, c.text);
    let mut changed = false;
    if *open {
        changed = body(ui);
        ui.add_space(6.0);
    }
    let bottom = ui.cursor().top();
    let rect = Rect::from_min_max(pos2(ui.max_rect().left() + 12.0, top), pos2(ui.max_rect().right() - 10.0, bottom));
    ui.painter().set(bg_idx, egui::Shape::rect_filled(rect, 4.0, c.group));
    ui.add_space(4.0);
    changed
}

/// A small boxed text button (Auto, B&W, HDR, Refresh).
pub(crate) fn small_button(ui: &mut Ui, label: &str, enabled: bool, selected: bool, width: f32) -> Response {
    let c = Acr::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 22.0), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label));
    let p = ui.painter();
    let fill = if selected {
        c.accent.gamma_multiply(0.55)
    } else if enabled && resp.hovered() {
        c.hover
    } else {
        c.field
    };
    p.rect_filled(rect, 2.0, fill);
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, if enabled { c.field_border } else { c.field_border.gamma_multiply(0.5) }), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, label, small_font(), if enabled { c.text } else { c.faint.gamma_multiply(0.7) });
    resp
}

/// Text tabs with an underline under the selected one (Mixer / Point Color; Hue / Saturation /
/// Luminance / All). Returns the clicked index.
pub(crate) fn text_tabs(ui: &mut Ui, id: &str, labels: &[&str], selected: usize, enabled: bool) -> Option<usize> {
    let c = Acr::get(ui.ctx());
    let mut clicked = None;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let mut x = row.left() + INSET_L - 6.0;
    let p = ui.painter().clone();
    for (i, label) in labels.iter().enumerate() {
        let g = p.layout_no_wrap((*label).to_owned(), label_font(), c.text);
        let w = g.size().x + 12.0;
        let r = Rect::from_min_size(pos2(x, row.top()), vec2(w, row.height()));
        let resp = ui.interact(r, ui.id().with((id, i)), if enabled { Sense::click() } else { Sense::hover() });
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, i == selected, *label));
        let color = if !enabled {
            c.faint.gamma_multiply(0.6)
        } else if i == selected || resp.hovered() {
            c.text
        } else {
            c.faint
        };
        p.text(r.center(), Align2::CENTER_CENTER, *label, label_font(), color);
        if i == selected {
            p.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - 2.0), pos2(r.right(), r.bottom())), 0.0, c.text);
        }
        if resp.clicked() {
            clicked = Some(i);
        }
        x += w + 4.0;
    }
    p.rect_filled(Rect::from_min_max(pos2(row.left() + INSET_L - 6.0, row.bottom() - 1.0), pos2(row.right() - INSET_R, row.bottom())), 0.0, c.rule);
    clicked
}

/// A centred caption (Midtones, Shadows, Upright, Red Primary, …).
pub(crate) fn caption(ui: &mut Ui, text: &str) {
    let c = Acr::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, small_font(), c.label);
}

/// An information note with an (i) badge, wrapped to the panel width.
pub(crate) fn note(ui: &mut Ui, text: &str, info_icon: bool) {
    let c = Acr::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(INSET_L);
        if info_icon {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
            crate::icons::paint(ui, r, "info", 13.0, c.label);
        }
        ui.add(egui::Label::new(egui::RichText::new(text).font(FontId::proportional(10.5)).color(c.label)).wrap());
    });
    ui.add_space(4.0);
}

/// A checkbox row in the panel's inset.
pub(crate) fn check_row(ui: &mut Ui, label: &str, on: &mut bool, enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(INSET_L);
        ui.add_enabled_ui(enabled, |ui| {
            changed = crate::widgets::checkbox(ui, on, label).changed();
        });
    });
    changed
}

/// A row of square icon buttons (Upright modes, curve channels) drawn by `paint`, one selected.
pub(crate) fn icon_strip(
    ui: &mut Ui,
    id: &str,
    count: usize,
    selected: Option<usize>,
    size: f32,
    tips: &[&str],
    mut paint: impl FnMut(&egui::Painter, Rect, usize, Color32),
) -> Option<usize> {
    let c = Acr::get(ui.ctx());
    let mut clicked = None;
    let total = size * count as f32 + 4.0;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size + 8.0), Sense::hover());
    let frame = Rect::from_center_size(row.center(), vec2(total, size + 4.0));
    ui.painter().rect_filled(frame, 3.0, c.rule.gamma_multiply(0.8));
    for i in 0..count {
        let r = Rect::from_min_size(pos2(frame.left() + 2.0 + size * i as f32, frame.top() + 2.0), vec2(size, size));
        let resp = ui.interact(r, ui.id().with((id, i)), Sense::click());
        let resp = match tips.get(i) {
            Some(t) => resp.on_hover_text(*t),
            None => resp,
        };
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected == Some(i), tips.get(i).copied().unwrap_or("")));
        if selected == Some(i) {
            ui.painter().rect_filled(r.shrink(1.0), 2.0, c.base);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(1.0), 2.0, c.hover.gamma_multiply(0.6));
        }
        paint(ui.painter(), r, i, if selected == Some(i) { c.text } else { c.label });
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Track stops for hue band `k` of the Color Mixer: from the previous band's hue to the next
/// band's, at Camera Raw's muted saturation.
pub(crate) fn band_stops(k: usize) -> Vec<Color32> {
    let centres = photocraft_algo::camera_raw::HSL_BANDS;
    let c = centres.get(k).copied().unwrap_or(0.0);
    let prev = centres.get((k + 7) % 8).copied().unwrap_or(0.0);
    let next = centres.get((k + 1) % 8).copied().unwrap_or(0.0);
    let lo = c - (c - prev).rem_euclid(360.0) * 0.6;
    let hi = c + (next - c).rem_euclid(360.0) * 0.6;
    (0..5).map(|i| hsv(lo + (hi - lo) * i as f32 / 4.0, 0.55, 0.76)).collect()
}

/// A colour from hue (degrees), saturation and value in 0..=1.
pub(crate) fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(r), q(g), q(b))
}

pub(crate) fn temperature_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(56, 56, 158), Color32::from_rgb(138, 138, 117), Color32::from_rgb(221, 221, 75)]
}
pub(crate) fn tint_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(62, 149, 58), Color32::from_rgb(98, 106, 97), Color32::from_rgb(144, 52, 144)]
}
/// Vibrance and Saturation: gray-blue through cyan and yellow to red.
pub(crate) fn saturation_stops() -> Vec<Color32> {
    vec![
        Color32::from_rgb(140, 132, 150),
        Color32::from_rgb(137, 138, 149),
        Color32::from_rgb(128, 149, 147),
        Color32::from_rgb(170, 175, 130),
        Color32::from_rgb(219, 204, 116),
        Color32::from_rgb(217, 160, 102),
        Color32::from_rgb(196, 69, 69),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_as_camera_raw_shows_them() {
        let s = Spec::centred(100.0);
        assert_eq!(s.text(40.0), "+40");
        assert_eq!(s.text(-12.0), "-12");
        assert_eq!(s.text(0.0), "0");
        assert_eq!(s.text(-0.2), "0");
        assert_eq!(Spec::centred(5.0).decimals(2).text(1.0), "+1.00");
        assert_eq!(Spec::amount(100.0, 25.0).text(25.0), "25");
        assert_eq!(s.text(f32::NAN), "0");
    }

    #[test]
    fn band_tracks_run_through_the_neighbouring_hues() {
        for k in 0..8 {
            assert_eq!(band_stops(k).len(), 5);
        }
        assert_eq!(band_stops(99).len(), 5);
    }
}
