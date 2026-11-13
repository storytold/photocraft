//! LightCraft's develop widgets (ported from its `widgets.rs`): the slider row (label left, value
//! right that is typed into on a click and scrubbed by a sideways drag, a thin track below with a
//! hollow ring thumb, coloured from the control's track kind), the chevron section header with
//! its eye, the flyout row, dividers, small bordered text buttons, segmented controls, icon
//! buttons and the borderless dropdown. They draw with [`Lc`] and announce themselves to screen
//! readers (and to the tests) as egui's own widgets do.

use craft_develop_model::controls::{ControlSpec, Track};
use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::icons::{Icon, paint};
use super::{Lc, SLIDER_ROW_H};

fn tr(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

/// `#rrggbb` as a colour (grey for anything else, never a panic).
pub(crate) fn hex(s: &str) -> Color32 {
    let v = s.strip_prefix('#').and_then(|h| h.get(..6)).and_then(|h| u32::from_str_radix(h, 16).ok()).unwrap_or(0x808080);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub(crate) fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// The eight Color Mixer band colours (swatches and tracks), LightCraft's.
pub(crate) const BAND_COLORS: [&str; 8] = ["#aa0000", "#bb6600", "#aaaa00", "#009900", "#00aaaa", "#0044cc", "#7722aa", "#aa0077"];

fn band(i: u8) -> Color32 {
    hex(BAND_COLORS.get(usize::from(i) % BAND_COLORS.len()).copied().unwrap_or("#808080"))
}

/// The colours a track of kind `track` runs through, left to right (none: the plain grey track).
pub(crate) fn track_stops(track: Track) -> Option<Vec<Color32>> {
    Some(match track {
        Track::Plain | Track::Centered => return None,
        Track::Temp => ["#193961", "#47566a", "#60666d", "#70705b", "#737339", "#75742a"].iter().map(|h| hex(h)).collect(),
        Track::Tint => ["#246123", "#2c552c", "#394739", "#4a3d4c", "#6b336b", "#872c87"].iter().map(|h| hex(h)).collect(),
        Track::Gradient { from, to } => vec![hex(from), hex(to)],
        Track::Hue { band: b } => {
            let b = b % 8;
            let (prev, me, next) = (band((b + 7) % 8), band(b), band((b + 1) % 8));
            vec![
                prev.gamma_multiply(0.7),
                lerp(prev, me, 0.5).gamma_multiply(0.7),
                me.gamma_multiply(0.7),
                lerp(me, next, 0.5).gamma_multiply(0.7),
                next.gamma_multiply(0.7),
            ]
        }
        Track::Sat { band: b } => vec![hex("#454343"), lerp(hex("#454343"), band(b), 0.6), band(b)],
        Track::Lum { band: b } => {
            let c = band(b);
            vec![c.gamma_multiply(0.2), c.gamma_multiply(0.55), lerp(c, Color32::WHITE, 0.35), lerp(c, Color32::WHITE, 0.6)]
        }
        Track::Rainbow => ["#aa0000", "#aaaa00", "#009900", "#00aaaa", "#0044cc", "#aa0077", "#aa0000"].iter().map(|h| hex(h)).collect(),
    })
}

fn paint_track(p: &egui::Painter, rect: Rect, track: Track, t: &Lc, thumb_x: f32, enabled: bool) {
    let y = rect.center().y;
    let h = 1.0;
    match track_stops(track) {
        Some(stops) if stops.len() >= 2 => {
            let n = stops.len() - 1;
            let steps = 48;
            for i in 0..steps {
                let (a, b) = (i as f32 / steps as f32, (i + 1) as f32 / steps as f32);
                let seg = ((a * n as f32).floor() as usize).min(n);
                let local = a * n as f32 - seg as f32;
                let (c0, c1) = (stops.get(seg).copied().unwrap_or(t.track), stops.get((seg + 1).min(n)).copied().unwrap_or(t.track));
                let c = lerp(c0, c1, local);
                let c = if enabled { c } else { c.gamma_multiply(0.45) };
                let (x0, x1) = (rect.left() + rect.width() * a, rect.left() + rect.width() * b);
                p.rect_filled(Rect::from_min_max(pos2(x0, y - h - 0.5), pos2(x1 + 0.5, y + h + 0.5)), 0.0, c);
            }
        }
        _ => {
            p.rect_filled(Rect::from_min_max(pos2(rect.left(), y - h), pos2(rect.right(), y + h)), 1.0, t.track);
        }
    }
    // a gap round the hollow thumb
    p.rect_filled(Rect::from_center_size(pos2(thumb_x, y), vec2(THUMB_RING * 2.0 + 4.0, 6.0)), 0.0, t.chrome);
}

/// What a slider did this frame.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub(crate) struct SliderOut {
    /// The new value (on the control's steps, in its range).
    pub value: Option<f64>,
    /// Double-clicked: `value` is the default.
    pub reset: bool,
    /// Alt (Option) is held while the thumb is dragged.
    pub alt_drag: bool,
}

/// What a value typed into a slider asks for: `0.5`, `+12`, `-1,5` or `5600 K`, on the control's
/// steps and in its range. `None` when it isn't a finite number.
pub(crate) fn typed_value(spec: &ControlSpec, text: &str) -> Option<f64> {
    let t = text.trim().replace(',', ".").replace('−', "-");
    let t = t.trim_end_matches(|c: char| c == '%' || c == 'K' || c == 'k' || c.is_whitespace());
    let v: f64 = t.parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(snap(spec, v))
}

/// `v` on `spec`'s steps and in its range.
pub(crate) fn snap(spec: &ControlSpec, v: f64) -> f64 {
    let v = if spec.step > 0.0 && spec.step.is_finite() { (v / spec.step).round() * spec.step } else { v };
    spec.clamp(v)
}

/// A slider's value as shown next to its label ("+0.50", "0", "5600").
pub(crate) fn shown_value(spec: &ControlSpec, v: f64) -> String {
    let shown = if spec.id == "wb.temp" { format!("{v:.0}") } else { spec.format(v) };
    // a value that rounds to zero reads "0", whatever the decimals
    if shown.bytes().any(|b| matches!(b, b'1'..=b'9')) { shown } else { "0".to_string() }
}

const THUMB_RING: f32 = 7.0;
/// Sideways travel that turns a press on the value into a scrub rather than a click to type.
const SCRUB_START: f32 = 3.0;

/// A LightCraft slider row: the label and value above a thin track with a hollow ring thumb.
/// Double-click the label or the track to reset; Shift-drag for fine steps; click the value to
/// type one (Return or clicking away applies it, Esc keeps the old one); drag the value sideways
/// to scrub it. `label` is shown (already translated).
pub(crate) fn slider(ui: &mut Ui, spec: &ControlSpec, value: f64, enabled: bool, label: &str) -> SliderOut {
    let t = Lc::get(ui.ctx());
    let w = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(w, SLIDER_ROW_H), Sense::hover());
    let (pad_l, pad_r) = (24.0, 22.0);
    let label_rect = Rect::from_min_size(pos2(row.left() + pad_l, row.top() + 4.0), vec2((w - pad_l - pad_r).max(1.0), 18.0));
    let track = Rect::from_min_max(pos2(row.left() + pad_l, row.top() + 22.0), pos2((row.right() - pad_r).max(row.left() + pad_l + 1.0), row.top() + 40.0));
    let value_rect = Rect::from_min_max(pos2(label_rect.right() - 64.0, label_rect.top()), label_rect.max);
    let id = ui.make_persistent_id(("lc-slider", spec.id));
    let g = Slider { spec, value, enabled, id, track };
    let sense = if enabled { Sense::click_and_drag() } else { Sense::hover() };
    let resp = ui.interact(track.expand2(vec2(8.0, 2.0)), id, sense);
    resp.widget_info(|| egui::WidgetInfo::slider(enabled, value, label));
    let label_resp = ui.interact(label_rect, id.with("label"), if enabled { Sense::click() } else { Sense::hover() });
    let value_resp = ui.interact(value_rect, id.with("value"), sense);
    let mut out = SliderOut::default();
    let typing = value_edit(ui, &g, &value_resp, value_rect, &mut out);
    if out.value.is_none() && !typing {
        if enabled && (resp.double_clicked() || label_resp.double_clicked()) {
            out.reset = true;
            out.value = Some(spec.default);
        } else {
            track_edit(ui, &g, &resp, &mut out);
        }
    }
    let v = out.value.unwrap_or(value);
    let hovered = resp.hovered() || resp.dragged();
    let text_c = if enabled { t.text_label } else { t.text_disabled };
    let p = ui.painter();
    p.text(label_rect.left_center(), Align2::LEFT_CENTER, label, t.font(12.5), text_c);
    if !typing {
        p.text(label_rect.right_center(), Align2::RIGHT_CENTER, shown_value(spec, v), t.font(12.5), text_c);
    }
    let tx = g.to_x(v);
    paint_track(p, track, spec.track, &t, tx, enabled);
    let ring_c = match (enabled, hovered) {
        (false, _) => t.text_disabled,
        (true, true) => t.thumb_hover,
        (true, false) => t.thumb,
    };
    let c = pos2(tx, track.center().y);
    p.circle_filled(c, THUMB_RING, t.chrome);
    p.circle_stroke(c, THUMB_RING, Stroke::new(2.0, ring_c));
    if enabled && (hovered || (!typing && (value_resp.hovered() || value_resp.dragged()))) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    out
}

/// What a slider's gestures need of it.
struct Slider<'a> {
    spec: &'a ControlSpec,
    value: f64,
    enabled: bool,
    id: egui::Id,
    track: Rect,
}

impl Slider<'_> {
    fn to_x(&self, v: f64) -> f32 {
        self.track.left() + self.spec.position(v) as f32 * self.track.width()
    }
    fn value_at(&self, x: f32) -> f64 {
        self.spec.value_at(f64::from((x - self.track.left()) / self.track.width().max(1.0)))
    }
    /// `nv` into `out`, when it is another value.
    fn set(&self, out: &mut SliderOut, nv: f64) {
        if (nv - self.value).abs() > 1e-12 {
            out.value = Some(nv);
        }
    }
}

/// The value's gestures: a sideways drag scrubs it, a click opens a field to type one. Returns
/// whether the field is open.
fn value_edit(ui: &mut Ui, g: &Slider, value_resp: &Response, value_rect: Rect, out: &mut SliderOut) -> bool {
    let (typing_id, field_id, scrub_id) = (g.id.with("typing"), g.id.with("field"), g.id.with("scrub"));
    let mut typing: Option<String> = ui.data(|d| d.get_temp(typing_id));
    if g.enabled && value_resp.dragged() {
        if let Some(p) = scrub_position(ui, g, scrub_id) {
            typing = None;
            ui.data_mut(|d| {
                d.insert_temp(scrub_id, p);
                d.remove::<String>(typing_id);
            });
            g.set(out, snap(g.spec, g.spec.value_at(p)));
        }
    } else {
        ui.data_mut(|d| d.remove::<f64>(scrub_id));
    }
    if g.enabled && typing.is_none() && value_resp.clicked() {
        typing = Some(shown_value(g.spec, g.value).trim_start_matches('+').to_string());
        ui.memory_mut(|m| m.request_focus(field_id));
    }
    let Some(mut text) = typing else { return false };
    let t = Lc::get(ui.ctx());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(value_rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    super::style(&mut child);
    let field = child.add(
        egui::TextEdit::singleline(&mut text)
            .id(field_id)
            .font(t.font(12.5))
            .align(egui::Align2::RIGHT_CENTER)
            .desired_width(value_rect.width())
            .margin(egui::Margin::ZERO),
    );
    let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if escape || !g.enabled {
        ui.data_mut(|d| d.remove::<String>(typing_id));
        return false;
    }
    if field.lost_focus() || enter {
        if let Some(nv) = typed_value(g.spec, &text) {
            g.set(out, nv);
        }
        ui.data_mut(|d| d.remove::<String>(typing_id));
        ui.memory_mut(|m| m.surrender_focus(field_id));
        return false;
    }
    if !field.has_focus() {
        field.request_focus();
    }
    ui.data_mut(|d| d.insert_temp(typing_id, text));
    true
}

/// Where a scrub of the value has taken the track position (0..1): from where it was, by the
/// pointer's sideways travel over the track's width (a tenth with Shift). `None` before the press
/// has moved far enough to be a scrub.
fn scrub_position(ui: &Ui, g: &Slider, scrub_id: egui::Id) -> Option<f64> {
    let pos: Option<f64> = ui.data(|d| d.get_temp(scrub_id));
    let (origin, at, dx, shift) = ui.input(|i| (i.pointer.press_origin(), i.pointer.interact_pos(), i.pointer.delta().x, i.modifiers.shift));
    let fine = if shift { 0.1 } else { 1.0 };
    let width = g.track.width().max(1.0);
    match (pos, origin, at) {
        (Some(p), _, _) => Some((p + f64::from(dx / width) * fine).clamp(0.0, 1.0)),
        (None, Some(o), Some(a)) if (a.x - o.x).abs() >= SCRUB_START => {
            Some((g.spec.position(g.value) + f64::from((a.x - o.x) / width) * fine).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

/// The track's gestures: drag the thumb (Shift: a tenth as far), click the track, the arrow keys.
fn track_edit(ui: &mut Ui, g: &Slider, resp: &Response, out: &mut SliderOut) {
    if (resp.dragged() || resp.drag_started())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let fine_id = g.id.with("fine");
        let raw = if ui.input(|i| i.modifiers.shift) {
            let kept: f64 = ui.data(|d| d.get_temp(fine_id)).unwrap_or_else(|| g.spec.position(g.value));
            let pos = (kept + f64::from(ui.input(|i| i.pointer.delta().x) / g.track.width().max(1.0)) * 0.1).clamp(0.0, 1.0);
            ui.data_mut(|d| d.insert_temp(fine_id, pos));
            g.spec.value_at(pos)
        } else {
            ui.data_mut(|d| d.remove::<f64>(fine_id));
            g.value_at(p.x)
        };
        g.set(out, snap(g.spec, raw));
        out.alt_drag = ui.input(|i| i.modifiers.alt);
        return;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && (p.x - g.to_x(g.value)).abs() > THUMB_RING + 1.0
    {
        out.value = Some(snap(g.spec, g.value_at(p.x)));
        return;
    }
    if !(g.enabled && (resp.hovered() || resp.has_focus())) {
        return;
    }
    // the arrow keys nudge it (Shift: five steps), as in LightCraft
    let (up, down, shift) = ui.input_mut(|i| {
        let shift = i.modifiers.shift;
        let m = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
        let up = i.consume_key(m, egui::Key::ArrowUp) || i.consume_key(m, egui::Key::ArrowRight);
        let down = i.consume_key(m, egui::Key::ArrowDown) || i.consume_key(m, egui::Key::ArrowLeft);
        (up, down, shift)
    });
    if up || down {
        let step = g.spec.step.max(1e-9);
        let unit = if g.spec.id == "wb.temp" { 50.0 } else { (((g.spec.max - g.spec.min) / 200.0 / step).round().max(1.0)) * step };
        let sign = if up { 1.0 } else { -1.0 };
        g.set(out, snap(g.spec, g.value + unit * sign * if shift { 5.0 } else { 1.0 }));
    }
}

/// Announces a disclosure row with its expanded state, as egui's collapsing header does.
fn announce_disclosure(resp: &Response, expanded: bool, label: &str) {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, label));
    resp.ctx.accesskit_node_builder(resp.id, |node| node.set_expanded(expanded));
}

/// A section header ("⌄ Light"): click to fold. With `enabled`, an eye (on hover, or always while
/// the section is off) switches the section; the second value is its new state when clicked.
pub(crate) fn section_header(ui: &mut Ui, id: &str, title: &str, open: bool, enabled: Option<bool>) -> (Response, Option<bool>) {
    let t = Lc::get(ui.ctx());
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 52.0), Sense::click());
    announce_disclosure(&resp, open, title);
    let p = ui.painter();
    if ui.rect_contains_pointer(r) {
        p.rect_filled(r, 0.0, t.chrome.gamma_multiply(1.06));
    }
    let chev = Rect::from_center_size(pos2(r.left() + 30.0, r.center().y), vec2(16.0, 16.0));
    paint(p, chev, if open { Icon::ChevronDown } else { Icon::ChevronRight }, t.text_label);
    p.text(pos2(r.left() + 44.0, r.center().y), Align2::LEFT_CENTER, title, t.semibold(14.0), t.text);
    let mut toggled = None;
    if let Some(on) = enabled
        && (ui.rect_contains_pointer(r) || !on)
    {
        let eye = Rect::from_center_size(pos2(r.right() - 34.0, r.center().y), vec2(18.0, 18.0));
        let er = ui.interact(eye, ui.make_persistent_id(("lc-section-eye", id)), Sense::click());
        er.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, tr("Show / hide")));
        paint(ui.painter(), eye, Icon::Eye, if on { t.icon } else { t.text_disabled });
        if !on {
            ui.painter().line_segment([eye.left_bottom(), eye.right_top()], Stroke::new(1.5, t.text_disabled));
        }
        if er.clicked() {
            toggled = Some(!on);
        }
    }
    (resp, toggled)
}

/// A panel title ("Edit", "Masking").
pub(crate) fn title(ui: &mut Ui, text: &str) {
    let t = Lc::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 24.0, r.bottom() - 10.0), Align2::LEFT_CENTER, text, t.semibold(15.0), t.text);
}

/// A sub-heading inside a section ("Vignette", "Grain").
pub(crate) fn sub_title(ui: &mut Ui, text: &str) {
    let t = Lc::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 24.0, r.center().y + 4.0), Align2::LEFT_CENTER, text, t.semibold(13.0), t.text_label);
}

/// The thin line between groups.
pub(crate) fn divider(ui: &mut Ui) {
    let t = Lc::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.divider);
}

/// `add` inside LightCraft's row margins (24 pt left, 22 right).
pub(crate) fn padded<R>(ui: &mut Ui, top: i8, bottom: i8, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top, bottom }).show(ui, add).inner
}

/// A flyout row inside a section (Curve, Color Mixer, Color Grading): an inset box with an icon
/// tile, the title and a chevron.
pub(crate) fn flyout_row(ui: &mut Ui, id: &str, title: &str, icon: Icon, open: bool) -> Response {
    let t = Lc::get(ui.ctx());
    let w = ui.available_width();
    let (outer, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let r = Rect::from_min_max(pos2(outer.left() + 16.0, outer.top() + 4.0), pos2(outer.right() - 16.0, outer.bottom() - 4.0));
    let resp = ui.interact(r, ui.make_persistent_id(("lc-flyout", id)), Sense::click());
    announce_disclosure(&resp, open, title);
    let p = ui.painter();
    p.rect_filled(r, 4.0, if resp.hovered() { t.hover.gamma_multiply(0.8) } else { t.inset });
    let tile = Rect::from_min_size(pos2(r.left() + 4.0, r.top() + 4.0), vec2(r.height() - 8.0, r.height() - 8.0));
    p.rect_filled(tile, 3.0, t.chrome.gamma_multiply(1.15));
    paint(p, tile.shrink(3.0), icon, t.text_label);
    p.text(pos2(tile.right() + 10.0, r.center().y), Align2::LEFT_CENTER, title, t.semibold(13.0), t.text_label);
    paint(
        p,
        Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(16.0, 16.0)),
        if open { Icon::ChevronDown } else { Icon::ChevronLeft },
        t.text_label,
    );
    resp
}

/// A small bordered text button (Auto, B&W, Reset, Add…); `active` draws it pressed.
pub(crate) fn text_button(ui: &mut Ui, label: &str, active: bool) -> Response {
    let t = Lc::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), t.semibold(11.5), t.text);
    let size = vec2((galley.size().x + 18.0).max(37.0), 24.0);
    let enabled = ui.is_enabled();
    let (r, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, active, label));
    let fill = if active {
        t.pressed
    } else if resp.hovered() && enabled {
        t.hover
    } else {
        t.button
    };
    let p = ui.painter();
    p.rect(r, CornerRadius::same(4), fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    p.galley(r.center() - galley.size() / 2.0, galley, if enabled { t.text } else { t.text_disabled });
    resp
}

/// A text button filled with the accent (the dialog's Open / OK).
pub(crate) fn accent_button(ui: &mut Ui, label: &str, width: f32) -> Response {
    let t = Lc::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = if resp.hovered() { t.accent.gamma_multiply(1.15) } else { t.accent };
    ui.painter().rect_filled(r, 14.0, fill);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, t.semibold(13.0), Color32::WHITE);
    resp
}

/// A rounded outline button for the footer (Cancel, Done, Load Settings…).
pub(crate) fn pill_button(ui: &mut Ui, label: &str, width: Option<f32>) -> Response {
    let t = Lc::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.to_string(), t.font(13.0), t.text);
    let w = width.unwrap_or(galley.size().x + 28.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let fill = if resp.hovered() { t.hover } else { t.chrome };
    ui.painter().rect(r, CornerRadius::same(14), fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    ui.painter().galley(r.center() - galley.size() / 2.0, galley, t.text);
    resp
}

/// A segmented control: the items share the width equally. Returns the clicked index.
pub(crate) fn segmented(ui: &mut Ui, items: &[&str], active: Option<usize>) -> Option<usize> {
    let t = Lc::get(ui.ctx());
    let n = items.len().max(1);
    let gap = 4.0;
    let seg_w = ((ui.available_width() - gap * (n as f32 - 1.0)) / n as f32).max(24.0);
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (i, label) in items.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(seg_w, 24.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active == Some(i), *label));
            let fill = if active == Some(i) {
                t.pressed
            } else if resp.hovered() {
                t.hover
            } else {
                t.button
            };
            let p = ui.painter();
            p.rect(r, CornerRadius::same(4), fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, *label, t.semibold(11.5), t.text);
            if resp.clicked() {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// An icon-only button; `active` draws the selected square (the rail's current tool).
pub(crate) fn icon_button(ui: &mut Ui, icon: Icon, size: egui::Vec2, active: bool, enabled: bool, tooltip: &str) -> Response {
    let t = Lc::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, active, tooltip));
    let p = ui.painter();
    let sq = Rect::from_center_size(r.center(), vec2(32.0f32.min(size.x), 32.0f32.min(size.y)));
    if active {
        p.rect_filled(sq, 4.0, t.tool_active);
    } else if resp.hovered() && enabled {
        p.rect_filled(sq, 4.0, t.hover.gamma_multiply(0.7));
    }
    let c = if !enabled {
        t.text_disabled
    } else if active {
        t.pick
    } else if resp.hovered() {
        t.text
    } else {
        t.icon
    };
    let glyph = (size.x.min(size.y) * 0.62).min(22.0);
    paint(p, Rect::from_center_size(r.center(), vec2(glyph, glyph)), icon, c);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

/// A borderless dropdown button: `text` and a chevron. Open a menu on its response.
pub(crate) fn dropdown(ui: &mut Ui, text: &str, font: egui::FontId, color: Color32) -> Response {
    let t = Lc::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = vec2(galley.size().x + 20.0, galley.size().y.max(20.0));
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, text));
    let c = if resp.hovered() { t.text } else { color };
    ui.painter().galley(pos2(r.left(), r.center().y - galley.size().y / 2.0), galley, c);
    paint(ui.painter(), Rect::from_center_size(pos2(r.right() - 10.0, r.center().y), vec2(16.0, 16.0)), Icon::ChevronDown, c);
    resp
}

/// A row of a dropdown's menu: `selected` marks the current choice. Says whether it was picked.
pub(crate) fn menu_item(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let r = ui.selectable_label(selected, label);
    if r.clicked() {
        ui.close();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_values_snap_and_bad_text_is_refused() {
        let spec = craft_develop_model::controls::find("light.exposure").unwrap();
        assert_eq!(typed_value(spec, " +0,456 "), Some(0.46));
        assert_eq!(typed_value(spec, "−1"), Some(-1.0));
        assert_eq!(typed_value(spec, "99"), Some(5.0));
        assert_eq!(typed_value(spec, "abc"), None);
        assert_eq!(typed_value(spec, "NaN"), None);
        let temp = craft_develop_model::controls::find("wb.temp").unwrap();
        assert_eq!(typed_value(temp, "5600 K"), Some(5600.0));
        assert_eq!(shown_value(spec, 0.0), "0");
        assert_eq!(shown_value(spec, 0.5), "+0.50");
        assert_eq!(hex("bad"), Color32::from_gray(128));
        assert!(track_stops(Track::Hue { band: 200 }).is_some(), "an out-of-range band wraps, never panics");
    }
}
