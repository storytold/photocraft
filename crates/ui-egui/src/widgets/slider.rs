use super::fields::{FieldOpts, VALUE_FIELD_H, value_field_in};
use super::panel::surface;
use super::scrub::scrub_label_within;
use super::{Color32, Pos2, Rect, Response, Sense, Stroke, Tokens, Ui, Vec2, pos2, vec2};

/// Thin-track slider with a round knob. `gradient` paints the track (e.g. hue spectrum); a
/// gradient slider is a colour legend, so its spectrum, value, drag and knob stay physical in
/// every direction and the knob keeps pointing at its own colour. A plain slider in a
/// right-to-left UI mirrors its knob, fill and drag map from the track's right edge; the wheel
/// stays physical (up = higher).
pub fn slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, gradient: Option<&[Color32]>) -> Response {
    slider_with(ui, value, range, gradient, RowGestures::default())
}

/// Photoshop's gestures on a slider row beyond click and drag, as measured on Color Balance in
/// Photoshop 25.4 (Image › Adjustments dialog and the adjustment layer's Properties). Shift, Ctrl
/// and Alt change nothing about clicks, double-clicks or drags; the wheel takes ten steps with
/// Shift. (Up/Down in the field are every value field's, see [`value_field`].)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowGestures {
    /// A double-click on the slider (knob or track; not the label or field) sets this value.
    pub reset: Option<f32>,
    /// Each wheel notch over the slider moves the value by this step (up = higher; ×10 with
    /// Shift). Photoshop does this in dialogs, not in the Properties panel.
    pub wheel_step: Option<f32>,
    /// A curved track: the value at fraction f of the track is `lo + (hi - lo) * f^k`, so small
    /// values get fine steps (Photoshop's mask Feather, #6545). Typed, scrubbed and stepped
    /// values stay linear; only the track maps through the curve. The fine end is the track's
    /// start edge, which is its right edge in a right-to-left UI.
    pub curve: Option<f32>,
}

/// The value at fraction `f` of a track over `lo..=hi`, through `curve` (see [`RowGestures`]).
pub(crate) fn curve_value(f: f32, lo: f32, hi: f32, curve: Option<f32>) -> f32 {
    match curve {
        Some(k) if k.is_finite() && k > 0.0 => lo + f.clamp(0.0, 1.0).powf(k) * (hi - lo),
        _ => lo + f * (hi - lo),
    }
}

/// Where `v` sits on a track over `lo..=hi`, as a fraction: the inverse of [`curve_value`].
pub(crate) fn curve_fraction(v: f32, lo: f32, hi: f32, curve: Option<f32>) -> f32 {
    let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
    match curve {
        // An unset (NaN) value draws the knob at the start rather than nowhere.
        Some(k) if k.is_finite() && k > 0.0 => {
            if f.is_finite() {
                f.powf(1.0 / k)
            } else {
                0.0
            }
        }
        _ => f,
    }
}

/// The x of fraction `f` along `track`: from its left edge, or from its right edge when `rtl`,
/// where the knob and the fill lead from the reading start.
fn track_x(track: Rect, f: f32, rtl: bool) -> f32 {
    if rtl { track.right() - f * track.width() } else { track.left() + f * track.width() }
}

/// The fraction of `track` at `x`: the inverse of [`track_x`], mapping a dragged pointer.
fn track_fraction(track: Rect, x: f32, rtl: bool) -> f32 {
    if rtl { (track.right() - x) / track.width() } else { (x - track.left()) / track.width() }
}

/// Wheel notches this frame while the pointer is over `resp` (Shift ×10). Wheel lines add up in
/// egui memory until they make whole notches (high-resolution wheels report fractions of one);
/// smooth (point) deltas count `line_scroll_speed` points per notch, like the canvas wheel.
fn wheel_notches(ui: &Ui, resp: &Response) -> f32 {
    wheel_notches_of(ui, resp.id, resp.hovered(), 10.0)
}

/// [`wheel_notches`] of widget `id` while `hovered`, a notch with Shift counting `shift_notches`.
pub(crate) fn wheel_notches_of(ui: &Ui, id: egui::Id, hovered: bool, shift_notches: f32) -> f32 {
    if !hovered {
        return 0.0;
    }
    let wheel: Vec<(egui::MouseWheelUnit, Vec2, bool)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, modifiers.shift)),
                _ => None,
            })
            .collect()
    });
    count_notches(ui, id.with("wheel-acc"), &wheel, shift_notches)
}

/// Whole notches in `wheel` (unit, delta, Shift held), a notch with Shift counting
/// `shift_notches`; fractions carry over in egui memory under `acc_id`.
pub(crate) fn count_notches(ui: &Ui, acc_id: egui::Id, wheel: &[(egui::MouseWheelUnit, Vec2, bool)], shift_notches: f32) -> f32 {
    let mut acc: f32 = ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0);
    let per_line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let per_line = if per_line.is_finite() && per_line > 0.0 { per_line } else { 40.0 };
    let mut notches = 0.0;
    for &(unit, delta, shift) in wheel {
        // Some systems turn Shift+wheel into a horizontal scroll.
        let d = if delta.y != 0.0 { delta.y } else { delta.x };
        acc += match unit {
            egui::MouseWheelUnit::Point => d / per_line,
            _ => d,
        };
        let whole = acc.trunc();
        acc -= whole;
        notches += if shift { whole * shift_notches } else { whole };
    }
    if !acc.is_finite() {
        acc = 0.0;
    }
    ui.data_mut(|d| d.insert_temp(acc_id, acc));
    if notches.is_finite() { notches } else { 0.0 }
}

/// Windows' default double-click time (Photoshop's; egui's own default is 0.3 s).
pub(super) const DOUBLE_CLICK_S: f64 = 0.5;

/// Finger jitter between taps easily exceeds egui's 6 px `max_click_dist`; 64 px is about two-thirds of an inch at 96 DPI (roughly a fingertip pad). 32 px still rejected casual Framework double-taps; Windows' mouse SM_CXDOUBLECLK is only 4 px and is not used for touch.
const TOUCH_MAX_CLICK_DIST: f32 = 64.0;

/// True when this frame's pointer work came from a touch contact (still down, or a Touch event including End on the click frame).
fn pointer_from_touch(ui: &Ui) -> bool {
    ui.input(|i| i.any_touches() || i.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })))
}

/// Whether this click on `resp` completes a double-click / double-tap, counted the Windows way (Photoshop): a click soon after a lone click near the same spot; the click after a double starts over.
///
/// Time window is 0.5 s (Windows / Photoshop; egui's default is 0.3 s). Mouse uses egui's `max_click_dist` (6 px); touch uses [`TOUCH_MAX_CLICK_DIST`] so finger jitter between taps still counts. Call on every click for that widget so the first tap is remembered.
pub fn second_click(ui: &Ui, resp: &Response) -> bool {
    if !resp.clicked() {
        return false;
    }
    let key = resp.id.with("pc-last-click");
    // Prefer the response's hit pos: raw `pointer.interact_pos` can be unset on a release-only frame.
    let pos = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.interact_pos()));
    let time = ui.input(|i| i.time);
    let touch = pointer_from_touch(ui);
    let mouse_dist = ui.ctx().options(|o| o.input_options.max_click_dist);
    let last: Option<(f64, Pos2, bool)> = ui.data(|d| d.get_temp(key));
    // Looser distance if either tap in the pair was touch (End clears `any_touches` but leaves Touch in `events`).
    let dist = if touch || last.is_some_and(|(_, _, t)| t) { TOUCH_MAX_CLICK_DIST.max(mouse_dist) } else { mouse_dist };
    let double = match (last, pos) {
        (Some((t, p, _)), Some(q)) => time - t < DOUBLE_CLICK_S && p.distance(q) < dist,
        _ => false,
    };
    ui.data_mut(|d| {
        if let (false, Some(q)) = (double, pos) {
            d.insert_temp(key, (time, q, touch));
        } else {
            d.remove::<(f64, Pos2, bool)>(key);
        }
    });
    double
}

fn slider_with(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, gradient: Option<&[Color32]>, g: RowGestures) -> Response {
    let t = Tokens::get(ui.ctx());
    let width = ui.available_width().max(60.0);
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 14.0, if gradient.is_some() { 5.0 } else { 3.0 }));
    // A gradient strip is a colour legend: its colours keep their physical order, so the
    // value↔x mapping stays physical with them and the knob keeps pointing at its own colour.
    let strip = matches!(gradient, Some(colors) if colors.len() >= 2);
    let rtl = crate::text::direction::is_rtl() && !strip;
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let f = track_fraction(track, p.x, rtl).clamp(0.0, 1.0);
        let nv = curve_value(f, lo, hi, g.curve);
        if (nv - *value).abs() > f32::EPSILON {
            *value = nv;
            resp.mark_changed();
        }
    }
    // The double-click's first click has moved the knob to the pointer; the second resets.
    if let Some(reset) = g.reset
        && second_click(ui, &resp)
    {
        *value = reset;
        resp.mark_changed();
    }
    if let Some(step) = g.wheel_step {
        let n = wheel_notches(ui, &resp);
        if n != 0.0 {
            let nv = (*value + n * step).clamp(lo, hi);
            if nv != *value {
                *value = nv;
                resp.mark_changed();
            }
        }
    }
    let f = curve_fraction(*value, lo, hi, g.curve);
    let knob_x = track_x(track, f, rtl);
    let painter = ui.painter();
    match gradient {
        Some(colors) if colors.len() >= 2 => {
            let n = colors.len() - 1;
            for (i, w) in colors.windows(2).enumerate() {
                let x0 = track.left() + track.width() * i as f32 / n as f32;
                let x1 = track.left() + track.width() * (i + 1) as f32 / n as f32;
                let mut mesh = egui::Mesh::default();
                let r = Rect::from_min_max(pos2(x0, track.top()), pos2(x1, track.bottom()));
                mesh.colored_vertex(r.left_top(), w[0]);
                mesh.colored_vertex(r.right_top(), w[1]);
                mesh.colored_vertex(r.right_bottom(), w[1]);
                mesh.colored_vertex(r.left_bottom(), w[0]);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(mesh);
            }
        }
        _ => {
            painter.rect_filled(track, 2.0, t.field_border);
            // The fill grows from the leading edge: the left in LTR, the right in RTL.
            let filled = if rtl { Rect::from_min_max(pos2(knob_x, track.min.y), track.max) } else { Rect::from_min_max(track.min, pos2(knob_x, track.max.y)) };
            painter.rect_filled(filled, 2.0, if t.bevel { t.accent } else { t.text_dim });
        }
    }
    let knob = pos2(knob_x, rect.center().y);
    if t.bevel {
        let kr = Rect::from_center_size(knob, vec2(10.0, 16.0));
        surface(ui, kr, t.card, true);
    } else {
        let kr = if t.pro { 6.0 } else { 7.0 };
        painter.circle_filled(knob + vec2(0.0, 1.0), kr + 0.5, Color32::from_black_alpha(90));
        painter.circle_filled(knob, kr, if t.pro { Color32::from_gray(236) } else { Color32::WHITE });
        if let Some(outline) = t.knob_outline() {
            painter.circle_stroke(knob, kr, Stroke::new(1.0, outline));
        }
        if resp.hovered() || resp.dragged() {
            painter.circle_stroke(knob, 9.5, Stroke::new(2.0, t.accent_soft));
        }
    }
    resp
}

/// Labelled slider row: `Label ........ [value field]` above a full-width thin slider.
pub fn slider_row(ui: &mut Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, gradient: Option<&[Color32]>) -> Response {
    slider_row_with(ui, label, value, range, suffix, gradient, RowGestures::default())
}

/// [`slider_row`] with Photoshop's extra gestures ([`RowGestures`]).
pub fn slider_row_with(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    gradient: Option<&[Color32]>,
    g: RowGestures,
) -> Response {
    slider_row_prec(ui, label, value, range, suffix, gradient, g, None)
}

/// [`slider_row_with`] whose field shows `decimals` places instead of the one (or two, for a
/// small range) it picks itself: Exposure's ±20 stops show two, its Offset four (#4164).
#[allow(clippy::too_many_arguments)]
pub fn slider_row_prec(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    gradient: Option<&[Color32]>,
    g: RowGestures,
    decimals: Option<u32>,
) -> Response {
    let t = Tokens::get(ui.ctx());
    let mut changed_resp = None;
    // The row's caption names both its number (through the scrub link) and its slider.
    let mut label_id = None;
    ui.horizontal(|ui| {
        // The label scrubs the row's field, across the row (Photoshop's scrubby labels).
        let l = ui.label(egui::RichText::new(tl!(label)).color(t.text_dim));
        label_id = Some(l.id);
        scrub_label_within(ui, &l, f32::INFINITY);
        // The field belongs at the row's reading end: follow the row's own direction, so a
        // row that did not mirror keeps the field at its far end.
        let pin = if ui.layout().main_dir() == egui::Direction::RightToLeft {
            egui::Layout::left_to_right(egui::Align::Center)
        } else {
            egui::Layout::right_to_left(egui::Align::Center)
        };
        ui.with_layout(pin, |ui| {
            changed_resp = Some(
                value_field_in(
                    ui,
                    value,
                    range.clone(),
                    suffix,
                    74.0,
                    0.0,
                    FieldOpts { decimals, step_from_decimals: decimals.is_some(), ..Default::default() },
                    None,
                )
                .0,
            );
        });
    });
    let s = slider_with(ui, value, range, gradient, g);
    s.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*value), tl!(label)));
    let s = if let Some(id) = label_id { s.labelled_by(id) } else { s };
    let mut r = s.clone();
    if let Some(v) = changed_resp {
        if v.changed() {
            r.mark_changed();
        }
        // A scrub of the field or its label is one drag, like the slider's, for history.
        use egui::response::Flags;
        r.flags |= v.flags & (Flags::DRAG_STARTED | Flags::DRAGGED | Flags::DRAG_STOPPED);
    }
    ui.add_space(4.0);
    r
}

/// A dialog row's label, right-aligned in a column `width` wide, so the controls after it line up
/// (Photoshop's label column in Layer Style). A label too long for it, as some translations are,
/// is cut short and shows in full on hover.
pub fn row_label(ui: &mut Ui, label: &str, width: f32) -> Response {
    let t = Tokens::get(ui.ctx());
    ui.allocate_ui_with_layout(vec2(width, VALUE_FIELD_H), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        ui.add(egui::Label::new(egui::RichText::new(tl!(label)).color(t.text_dim)).truncate())
    })
    .inner
}

/// [`slider_row`] on one line, as Photoshop lays out Layer Style: the label in a `label_w` column
/// ([`row_label`]), the slider, then the value field. The label still scrubs the field.
pub fn slider_line(ui: &mut Ui, label: &str, label_w: f32, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str) -> Response {
    const FIELD_W: f32 = 64.0;
    let mut field = None;
    let s = ui
        .horizontal(|ui| {
            let l = row_label(ui, label, label_w);
            scrub_label_within(ui, &l, f32::INFINITY);
            let w = (ui.available_width() - FIELD_W - ui.spacing().item_spacing.x).max(60.0);
            let s = ui
                .allocate_ui_with_layout(vec2(w, VALUE_FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    slider_with(ui, value, range.clone(), None, RowGestures::default())
                })
                .inner;
            field = Some(value_field_in(ui, value, range.clone(), suffix, FIELD_W, 0.0, FieldOpts::default(), Some(label)).0);
            s
        })
        .inner;
    let mut r = s.clone();
    if let Some(v) = field {
        if v.changed() {
            r.mark_changed();
        }
        // A scrub of the field or its label is one drag, like the slider's, for history.
        use egui::response::Flags;
        r.flags |= v.flags & (Flags::DRAG_STARTED | Flags::DRAGGED | Flags::DRAG_STOPPED);
    }
    r
}

#[cfg(test)]
mod tests {
    use egui::Rect;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::{RowGestures, slider, slider_row, slider_row_with};
    use crate::text::direction::{Dir, direction_scope};

    /// Painted geometry of one bare `slider` frame: the thin track rect, the value fill rect
    /// (None over a gradient strip), the knob circle's centre x, and the gradient mesh vertex
    /// x's and colours in paint order.
    struct Painted {
        track: Option<Rect>,
        fill: Option<Rect>,
        knob_x: Option<f32>,
        verts: Vec<f32>,
        colors: Vec<egui::Color32>,
    }

    fn collect(shape: &egui::Shape, p: &mut Painted) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, p)),
            egui::Shape::Rect(r) => {
                if r.rect.height() <= 6.0 {
                    match &p.track {
                        Some(t) if t.width() >= r.rect.width() => p.fill = Some(r.rect),
                        _ => p.track = Some(r.rect),
                    }
                }
            }
            egui::Shape::Circle(c) => {
                if c.fill.a() == 255 && c.radius >= 5.0 {
                    p.knob_x = Some(c.center.x);
                }
            }
            egui::Shape::Mesh(m) => {
                p.verts.extend(m.vertices.iter().map(|v| v.pos.x));
                p.colors.extend(m.vertices.iter().map(|v| v.color));
            }
            _ => {}
        }
    }

    /// Renders one bare slider at `value` under direction `dir` and reads its painted geometry.
    fn probe(dir: Dir, value: f32, gradient: bool) -> Painted {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(egui::vec2(300.0, 60.0)).build_ui_state(
            move |ui, v: &mut f32| {
                let stops = gradient.then(crate::widgets::hue_stops);
                slider(ui, v, 0.0..=100.0, stops.as_deref());
            },
            value,
        );
        crate::theme::apply(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run();
        let mut p = Painted { track: None, fill: None, knob_x: None, verts: Vec::new(), colors: Vec::new() };
        for clipped in &h.output().shapes {
            collect(&clipped.shape, &mut p);
        }
        p
    }

    /// Renders a hue slider at `hue` degrees under direction `dir` and reads its geometry.
    fn probe_hue(dir: Dir, hue: f32) -> Painted {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(egui::vec2(300.0, 60.0)).build_ui_state(
            move |ui, v: &mut f32| {
                let stops = crate::widgets::hue_stops();
                slider(ui, v, 0.0..=360.0, Some(&stops));
            },
            hue,
        );
        crate::theme::apply(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run();
        let mut p = Painted { track: None, fill: None, knob_x: None, verts: Vec::new(), colors: Vec::new() };
        for clipped in &h.output().shapes {
            collect(&clipped.shape, &mut p);
        }
        p
    }
    /// RTL: the fill grows from the track's right edge and the knob rides right of centre for a
    /// small value; LTR is unchanged (fill from the left, knob left of centre).
    #[test]
    fn slider_fill_and_knob_mirror_in_rtl() {
        for (dir, rtl) in [(Dir::Ltr, false), (Dir::Rtl, true)] {
            let p = probe(dir, 25.0, false);
            let track = p.track.expect("the track is painted");
            let fill = p.fill.expect("the value fill is painted");
            let knob_x = p.knob_x.expect("the knob is painted");
            let mid = track.center().x;
            if rtl {
                assert!((fill.right() - track.right()).abs() < 0.51, "{dir:?}: fill ends at the right edge ({fill:?} of {track:?})");
                assert!((fill.left() - (track.right() - 0.25 * track.width())).abs() < 0.51, "{dir:?}: 25% spans 75%..100% ({fill:?})");
                assert!(knob_x > mid, "{dir:?}: knob right of centre at {knob_x} (mid {mid})");
            } else {
                assert!((fill.left() - track.left()).abs() < 0.51, "{dir:?}: fill starts at the left edge ({fill:?} of {track:?})");
                assert!((fill.right() - (track.left() + 0.25 * track.width())).abs() < 0.51, "{dir:?}: 25% spans 0%..25% ({fill:?})");
                assert!(knob_x < mid, "{dir:?}: knob left of centre at {knob_x} (mid {mid})");
            }
        }
    }
    /// RTL: a drag maps the pointer mirrored — pressing at 75% of the widget width picks ~25% of
    /// the range; the same press in LTR picks ~75%.
    #[test]
    fn slider_drag_follows_the_pointer_mirrored_in_rtl() {
        let press_at = |dir: Dir| -> f32 {
            let _dir = direction_scope(dir);
            let mut h = Harness::builder().with_size(egui::vec2(300.0, 60.0)).build_ui_state(
                |ui, s: &mut (f32, Rect)| {
                    s.1 = slider(ui, &mut s.0, 0.0..=100.0, None).rect;
                },
                (50.0, Rect::NOTHING),
            );
            h.run();
            let r = h.state().1;
            let at = egui::pos2(r.left() + 0.75 * r.width(), r.center().y);
            h.hover_at(at);
            h.run();
            h.drag_at(at);
            h.run();
            // A real movement, so the drag is under way and the slider reads the pointer.
            let moved = at + egui::vec2(4.0, 0.0);
            h.event(egui::Event::PointerMoved(moved));
            h.run();
            h.drop_at(moved);
            h.run();
            h.state().0
        };
        let ltr = press_at(Dir::Ltr);
        let rtl = press_at(Dir::Rtl);
        assert!(ltr > 70.0 && ltr < 85.0, "LTR: a press at 75% of the width picks ~75% ({ltr})");
        assert!(rtl > 15.0 && rtl < 30.0, "RTL: the same press picks ~25% ({rtl})");
    }
    /// RTL: in a mirrored body the row's value field sits at the reading end (the row's left),
    /// not pinned to the right; LTR keeps the field at the right end.
    #[test]
    fn slider_row_field_sits_at_the_reading_end_in_rtl() {
        use egui::accesskit::Role;
        let probe = |dir: Dir| -> (Rect, Rect) {
            let _dir = direction_scope(dir);
            let mut h = Harness::builder().with_size(egui::vec2(360.0, 120.0)).build_ui_state(
                |ui, v: &mut f32| {
                    ui.with_layout(crate::text::direction::body_layout(), |ui| {
                        slider_row(ui, "Opacity", v, 0.0..=100.0, "%", None);
                    });
                },
                50.0,
            );
            crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
            h.run();
            let field = h.get_by_role_and_label(Role::SpinButton, "Opacity").rect();
            let label = h
                .query_all(egui_kittest::kittest::by().label("Opacity").include_labels())
                .find(|n| n.accesskit_node().role() == Role::Label)
                .expect("the row's caption")
                .rect();
            (label, field)
        };
        let (label, field) = probe(Dir::Ltr);
        assert!(field.right() > label.right(), "LTR: the field is right of the label ({field:?} vs {label:?})");
        let (label, field) = probe(Dir::Rtl);
        assert!(field.left() < label.left(), "RTL: the field is left of the label, at the reading end ({field:?} vs {label:?})");
    }
    /// RTL in a body that did not mirror: the row keeps its left-to-right shape, so the field
    /// stays at the row's far end instead of sitting glued to the label.
    #[test]
    fn slider_row_field_stays_at_the_far_end_in_an_unmirrored_rtl_body() {
        use egui::accesskit::Role;
        let _rtl = direction_scope(Dir::Rtl);
        let mut h = Harness::builder().with_size(egui::vec2(360.0, 120.0)).build_ui_state(
            |ui, v: &mut f32| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    slider_row(ui, "Opacity", v, 0.0..=100.0, "%", None);
                });
            },
            50.0,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run();
        let field = h.get_by_role_and_label(Role::SpinButton, "Opacity").rect();
        let label = h
            .query_all(egui_kittest::kittest::by().label("Opacity").include_labels())
            .find(|n| n.accesskit_node().role() == Role::Label)
            .expect("the row's caption")
            .rect();
        assert!(field.left() > label.right() + 10.0, "the field is not glued to the label ({field:?} vs {label:?})");
        assert!(field.right() > 300.0, "the field sits at the row's far end ({field:?})");
    }
    /// The wheel stays physical in RTL: up steps the value up, Shift ×10 — input is not layout.
    #[test]
    fn slider_wheel_still_steps_upward_in_rtl() {
        use egui::accesskit::Role;
        let _rtl = direction_scope(Dir::Rtl);
        let mut h = Harness::builder().with_size(egui::vec2(300.0, 80.0)).build_ui_state(
            |ui, v: &mut f32| {
                slider_row_with(ui, "Opacity", v, 0.0..=100.0, "%", None, RowGestures { wheel_step: Some(1.0), ..Default::default() });
            },
            50.0,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run();
        let wheel = |h: &mut Harness<'_, f32>, delta: egui::Vec2, mods: egui::Modifiers| {
            let at = h.get_by_role_and_label(Role::Slider, "Opacity").rect().center();
            h.event(egui::Event::ModifiersChanged(mods));
            h.hover_at(at);
            h.run();
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers: mods,
            });
            h.run();
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.run();
        };
        wheel(&mut h, egui::vec2(0.0, 1.0), egui::Modifiers::NONE);
        assert_eq!(*h.state(), 51.0, "wheel up steps up in RTL");
        wheel(&mut h, egui::vec2(0.0, 1.0), egui::Modifiers::SHIFT);
        assert_eq!(*h.state(), 61.0, "Shift+wheel is ten steps");
    }
    /// A gradient strip is a colour legend: it keeps its physical colour order in RTL, and the
    /// knob stays with it, so the knob keeps pointing at the colour of its own value.
    #[test]
    fn slider_gradient_strip_stays_physical_in_rtl() {
        let ltr = probe_hue(Dir::Ltr, 100.0);
        let rtl = probe_hue(Dir::Rtl, 100.0);
        assert!(ltr.fill.is_none() && rtl.fill.is_none(), "a gradient track paints no value fill");
        assert_eq!(ltr.verts, rtl.verts, "the spectrum's vertex x's are identical in both directions");
        assert_eq!(ltr.colors, rtl.colors, "the spectrum's colours are identical in both directions");
        let lo = ltr.verts.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = ltr.verts.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(ltr.verts.first(), Some(&lo), "the strip is painted left to right");
        let knob_ltr = ltr.knob_x.expect("the knob is painted");
        let knob_rtl = rtl.knob_x.expect("the knob is painted");
        assert!((knob_ltr - knob_rtl).abs() < 0.51, "a gradient slider's knob stays over its colours (LTR {knob_ltr}, RTL {knob_rtl})");
        // Hue 100 lies between the 90° and 120° stops: the knob must sit over that segment,
        // whose colours bracket the value.
        let seg = (((knob_rtl - lo) / (hi - lo)) * 12.0).floor() as usize;
        assert_eq!(seg, 3, "the knob at hue 100 sits over the 90°..120° segment");
        assert_eq!(rtl.colors[12], crate::widgets::hue_color(90.0), "the segment starts at the 90° stop");
        assert_eq!(rtl.colors[13], crate::widgets::hue_color(120.0), "the segment ends at the 120° stop");
    }
}
