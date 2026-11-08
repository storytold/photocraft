//! The Edit column, laid out as Camera Raw's: an "Edit" header (Auto and the settings menu), then
//! the panels in Camera Raw's order as collapsible sections, each with an eye button that hides
//! the panel's settings from the preview and from OK. Sliders put the label left and the value
//! right above the track, colour the tracks Camera Raw colours (tone, temperature, tint,
//! saturation, the mixer's bands) and reset to the setting's default on a double-click.

use super::panels::{CURVE_CHANNELS, GRADING_VIEWS, MIXER_ALL, MIXER_TABS, MixerAdjust, PanelState};
use super::{BANDS, CameraRaw, CameraRawDialog, Color32, ERect, Pos2, Sense, Stroke, Tokens, ToneZone, Wheel, curve_lut, pos2, vec2, widgets};

/// A Camera Raw label in the current language (the `cameraRaw` context first).
fn trc(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

// Geometry of a Camera Raw slider row, measured on Camera Raw 18.6 at 100 % display scale: rows
// 30 pt apart, the value in a 42 × 17 box at the right of the label, a 3 pt track 4 pt under the
// box running the row's full width, and a 11 × 9 pentagon knob pointing up at the track.
const ROW_H: f32 = 30.0;
const FIELD_W: f32 = 42.0;
const FIELD_H: f32 = 17.0;
const TRACK_TOP: f32 = 21.0;
const TRACK_H: f32 = 3.0;
const KNOB_W: f32 = 11.0;
const KNOB_H: f32 = 9.0;
/// Space under an open panel's last row, before the next panel's separator.
const PANEL_END: f32 = 18.0;

/// Camera Raw's track colours, sampled from its panels (content colours, not chrome): white
/// balance runs from blue to yellow and green to magenta, Vibrance / Saturation from a dull
/// lavender through green and yellow to red.
pub(super) fn temp_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(44, 44, 165), Color32::from_rgb(219, 219, 76)]
}

pub(super) fn tint_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(52, 159, 48), Color32::from_rgb(138, 59, 138)]
}

/// Dark to light: every slider without a colour of its own has this track in Camera Raw.
fn tone_stops() -> Vec<Color32> {
    vec![Color32::from_gray(90), Color32::from_gray(230)]
}

fn vivid_stops() -> Vec<Color32> {
    [
        (143, 138, 145),
        (138, 138, 148),
        (131, 146, 148),
        (126, 149, 140),
        (121, 147, 124),
        (156, 168, 118),
        (213, 201, 116),
        (219, 183, 109),
        (216, 156, 100),
        (204, 105, 82),
        (198, 79, 73),
    ]
    .into_iter()
    .map(|(r, g, b)| Color32::from_rgb(r, g, b))
    .collect()
}

/// A hue track centred on `deg`, ±`span` degrees.
fn hue_span(deg: f32, span: f32) -> Vec<Color32> {
    vec![widgets::hue_color((deg - span).rem_euclid(360.0)), widgets::hue_color(deg), widgets::hue_color((deg + span).rem_euclid(360.0))]
}

/// The track of Color Mixer band `k` on `tab` (0 Hue, 1 Saturation, 2 Luminance). A Hue slider
/// turns its band towards the neighbouring bands, so its track runs from the previous band's hue
/// through its own to the next one's (Reds: magenta, red, orange; #9550); Saturation runs from
/// grey to the band's colour, Luminance from dark to light through it.
pub(super) fn mixer_gradient(tab: usize, k: usize) -> Vec<Color32> {
    let bands = &photocraft_algo::camera_raw::HSL_BANDS;
    let hue = |i: usize| widgets::hue_color(bands.get(i % bands.len()).copied().unwrap_or(0.0));
    match tab {
        0 => vec![hue(k + bands.len() - 1), hue(k), hue(k + 1)],
        2 => vec![Color32::from_gray(24), hue(k), Color32::from_gray(232)],
        _ => vec![Color32::from_gray(128), hue(k)],
    }
}

/// Paints `stops` evenly across `track`.
fn paint_gradient(painter: &egui::Painter, track: ERect, stops: &[Color32]) {
    let n = stops.len().saturating_sub(1).max(1) as f32;
    for (i, w) in stops.windows(2).enumerate() {
        let x0 = track.left() + track.width() * i as f32 / n;
        let x1 = track.left() + track.width() * (i + 1) as f32 / n;
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(pos2(x0, track.top()), w[0]);
        mesh.colored_vertex(pos2(x1, track.top()), w[1]);
        mesh.colored_vertex(pos2(x1, track.bottom()), w[1]);
        mesh.colored_vertex(pos2(x0, track.bottom()), w[0]);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }
}

/// The decimal places Camera Raw shows for a slider: two for a small range (Exposure's "0.00"),
/// one for Sharpening's Radius, Geometry's Rotate and a range starting at a fraction (brush Size),
/// none for the -100…100 sliders.
pub(super) fn places(label: &str, lo: f32, hi: f32) -> u32 {
    if matches!(label, "Radius" | "Rotate") || lo.fract() != 0.0 && hi - lo > 10.0 {
        1
    } else if hi - lo <= 10.0 {
        2
    } else {
        0
    }
}

/// One Camera Raw slider row (the box shows two decimals for a small range such as Exposure's):
/// the label at the left and the value box at the right on one line,
/// the track under both. Dragging or clicking the track sets the value, a double-click on it sets
/// `reset`; the box takes typed values, arrow keys and the wheel. Marks `dirty` when it changes.
pub(super) fn slider(
    ui: &mut egui::Ui,
    dirty: &mut bool,
    label: &str,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    grad: Option<&[Color32]>,
    reset: f32,
) -> egui::Response {
    slider_more(ui, dirty, label, v, range, grad, reset, None)
}

/// Width Camera Raw leaves right of the value box for a slider's disclosure triangle.
const MORE_W: f32 = 20.0;

/// A [`slider`] with Camera Raw's disclosure triangle at the right of its value box (Vignette,
/// Grain): ◀ while its options are hidden, ▼ while `more` shows them. A click toggles `more`.
#[expect(clippy::too_many_arguments)]
fn slider_more(
    ui: &mut egui::Ui,
    dirty: &mut bool,
    label: &str,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    grad: Option<&[Color32]>,
    reset: f32,
    more: Option<&mut bool>,
) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let label_en = label;
    let label = trc(label);
    let (lo, hi) = (*range.start(), *range.end());
    let id = ui.next_auto_id().with("camera-raw-track");
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width().max(FIELD_W * 2.0), ROW_H), Sense::hover());
    let gap = if more.is_some() { MORE_W } else { 0.0 };
    let field_rect = ERect::from_min_size(pos2(row.right() - gap - FIELD_W, row.top()), vec2(FIELD_W, FIELD_H));
    if let Some(open) = more {
        let tri = ERect::from_min_max(pos2(field_rect.right(), row.top()), pos2(row.right(), field_rect.bottom()));
        let r = ui.interact(tri, id.with("more"), Sense::click());
        if r.clicked() {
            *open = !*open;
        }
        let c = tri.center() + vec2(3.0, 0.0);
        let col = if r.hovered() { t.text } else { t.text_dim };
        let pts = if *open {
            vec![pos2(c.x - 5.0, c.y - 3.0), pos2(c.x + 5.0, c.y - 3.0), pos2(c.x, c.y + 4.0)]
        } else {
            vec![pos2(c.x + 4.0, c.y - 5.0), pos2(c.x + 4.0, c.y + 5.0), pos2(c.x - 4.0, c.y)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        let name = format!("{} {}", trc(label), trc("Options"));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, &name));
    }
    let track = ERect::from_min_max(pos2(row.left(), row.top() + TRACK_TOP), pos2(row.right(), row.top() + TRACK_TOP + TRACK_H));
    // The label, cut short with its full text on hover when a translation is too long.
    let mut label_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(ERect::from_min_max(row.min, pos2(field_rect.left() - 6.0, field_rect.bottom())))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let label_resp = label_ui.add(egui::Label::new(egui::RichText::new(label).color(t.text)).truncate().selectable(false));
    // The value box.
    let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let before = *v;
    let field = widgets::camera_raw_value_field(&mut field_ui, v, range.clone(), places(label_en, lo, hi), vec2(FIELD_W, FIELD_H)).labelled_by(label_resp.id);
    // The track: the hit area is the row under the box.
    let hit = ERect::from_min_max(pos2(row.left(), field_rect.bottom()), row.max);
    let mut resp = ui.interact(hit, id, Sense::click_and_drag());
    let f_of = |x: f32| ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
    if let Some(p) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        let nv = lo + f_of(p.x) * (hi - lo);
        if (nv - *v).abs() > f32::EPSILON {
            *v = nv;
        }
    }
    // The double-click's first click has moved the knob to the pointer; the second resets.
    if widgets::second_click(ui, &resp) {
        *v = reset;
    }
    if *v != before || field.changed() {
        resp.mark_changed();
        *dirty = true;
    }
    // A scrub of the box is one drag, like the track's, for history.
    use egui::response::Flags;
    resp.flags |= field.flags & (Flags::DRAG_STARTED | Flags::DRAGGED | Flags::DRAG_STOPPED);
    let painter = ui.painter();
    match grad {
        Some(stops) if stops.len() >= 2 => paint_gradient(painter, track, stops),
        _ => paint_gradient(painter, track, &tone_stops()),
    }
    let fraction = if hi > lo && v.is_finite() { ((*v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let x = track.left() + KNOB_W / 2.0 + fraction * (track.width() - KNOB_W);
    let tip = track.top() - 3.0;
    let shoulder = tip + KNOB_W / 2.0 - 1.5;
    let base = tip + KNOB_H;
    let knob =
        vec![pos2(x, tip), pos2(x + KNOB_W / 2.0, shoulder), pos2(x + KNOB_W / 2.0, base), pos2(x - KNOB_W / 2.0, base), pos2(x - KNOB_W / 2.0, shoulder)];
    let outline = if resp.dragged() || resp.hovered() { t.accent } else { t.canvas };
    painter.add(egui::Shape::convex_polygon(knob, t.text, Stroke::new(1.0, outline)));
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*v), label));
    resp.labelled_by(label_resp.id)
}

pub(super) fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>, reset: f32) {
    let _ = slider(ui, dirty, label, v, range, grad, reset);
}

/// A Light tone slider; Alt-dragging it shows that zone's channel clipping on the preview.
fn tone_row(ui: &mut egui::Ui, dirty: &mut bool, alt: &mut Option<ToneZone>, zone: ToneZone, v: &mut f32) {
    let r = slider(ui, dirty, zone.name(), v, -zone.limit()..=zone.limit(), None, 0.0);
    if r.dragged() && ui.input(|i| i.modifiers.alt) {
        *alt = Some(zone);
    }
}

/// A sub-heading inside a panel (Vignette, Grain, Sharpening…).
fn sub(ui: &mut egui::Ui, title: &str) {
    ui.add_space(6.0);
    widgets::section_label(ui, trc(title));
    ui.add_space(6.0);
}

/// A panel's header row: the disclosure triangle and title (click to open or close), and the eye
/// button at the right (shown on hover, or while the panel is hidden). `true` when open.
fn section(ui: &mut egui::Ui, panels: &mut PanelState, id: &'static str, title: &str, dirty: &mut bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = trc(title);
    let hidden = panels.hidden.contains(id);
    // Camera Raw: a 2 pt dark rule between panels, then a 36 pt header row.
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    ui.painter().rect_filled(rule, 0.0, t.canvas);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let open = panels.open.contains(id);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, ui.is_enabled(), label));
    let over = ui.rect_contains_pointer(rect);
    if over {
        ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.45));
    }
    let cy = rect.center().y;
    // Camera Raw's bold disclosure chevron, 9 pt wide, aligned with the slider labels.
    let c = pos2(rect.left() + 4.5, cy);
    let chevron = if open {
        vec![c + vec2(-4.0, -2.0), c + vec2(0.0, 2.0), c + vec2(4.0, -2.0)]
    } else {
        vec![c + vec2(-2.0, -4.0), c + vec2(2.0, 0.0), c + vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::line(chevron, Stroke::new(2.0, t.text)));
    let title_color = if hidden { t.text_faint } else { t.text };
    ui.painter().text(pos2(rect.left() + 20.0, cy), egui::Align2::LEFT_CENTER, label, crate::theme::semibold(12.5), title_color);
    let eye_rect = ERect::from_center_size(pos2(rect.right() - 9.0, cy), vec2(20.0, 20.0));
    let eye = ui.interact(eye_rect, ui.make_persistent_id(("camera-raw-eye", id)), Sense::click());
    let tip = tl!("Show or hide this panel's settings");
    eye.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, !hidden, format!("{label}: {tip}")));
    // Always there, faint, as in Camera Raw; brighter under the pointer or while hidden.
    let tint = if eye.hovered() || hidden {
        t.text
    } else if over {
        t.text_dim
    } else {
        t.text_faint
    };
    crate::icons::paint(ui, eye_rect, if hidden { "eye-off" } else { "eye" }, 15.0, tint);
    let eye = eye.on_hover_text(tip);
    if eye.clicked() {
        if hidden {
            panels.hidden.remove(id);
        } else {
            panels.hidden.insert(id);
        }
        *dirty = true;
    } else if resp.clicked() {
        if open {
            panels.open.remove(id);
        } else {
            panels.open.insert(id);
        }
    }
    let open = panels.open.contains(id);
    if open {
        ui.add_space(4.0);
    }
    open
}

/// Color Grading as Camera Raw lays it out: an Adjust row of view buttons, then either the three
/// wheels (Midtones centred above Shadows and Highlights, each with its Luminance slider) or one
/// large wheel with its Hue, Saturation and Luminance sliders.
fn color_grading(ui: &mut egui::Ui, p: &mut CameraRaw, panels: &mut PanelState, dirty: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(trc("Adjust")).color(t.text));
        ui.add_space(4.0);
        for (i, (_, name)) in GRADING_VIEWS.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            let on = panels.grading_view == i;
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, trc(name)));
            let c = rect.center();
            let ink = if on || resp.hovered() { t.text } else { t.text_dim };
            let painter = ui.painter();
            match i {
                0 => {
                    for o in [vec2(0.0, -3.5), vec2(-4.0, 3.0), vec2(4.0, 3.0)] {
                        painter.circle_stroke(c + o, 3.0, Stroke::new(1.0, ink));
                    }
                }
                1 => {
                    painter.circle_filled(c, 6.5, Color32::from_gray(40));
                    painter.circle_stroke(c, 6.5, Stroke::new(1.0, ink));
                }
                2 => {
                    painter.circle_stroke(c, 6.5, Stroke::new(1.0, ink));
                    let half: Vec<Pos2> = (0..=16).map(|k| c + egui::Vec2::angled(std::f32::consts::PI * (0.5 + k as f32 / 16.0)) * 6.0).collect();
                    painter.add(egui::Shape::convex_polygon(half, ink, Stroke::NONE));
                }
                3 => {
                    painter.circle_filled(c, 6.5, ink);
                }
                _ => {
                    painter.circle_stroke(c, 6.5, Stroke::new(1.0, ink));
                    painter.circle_stroke(c, 3.0, Stroke::new(1.0, ink));
                }
            }
            if on {
                painter.rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
            }
            if resp.on_hover_text(trc(name)).clicked() {
                panels.grading_view = i;
            }
        }
    });
    ui.add_space(8.0);
    let full = ui.available_width();
    match panels.grading_view {
        0 => {
            let small = (full / 2.0 - 12.0).clamp(60.0, 110.0);
            ui.vertical_centered(|ui| {
                ui.set_max_width(full / 2.0);
                grade_block(ui, dirty, "Midtones", &mut p.grade_midtones, small, false);
            });
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                if let [left, right] = cols {
                    // A gutter between the two Luminance sliders, as Camera Raw leaves.
                    left.set_max_width(left.available_width() - 8.0);
                    grade_block(left, dirty, "Shadows", &mut p.grade_shadows, small, false);
                    let r = right.available_rect_before_wrap();
                    let mut inner = right.new_child(egui::UiBuilder::new().max_rect(ERect::from_min_max(pos2(r.left() + 8.0, r.top()), r.max)));
                    grade_block(&mut inner, dirty, "Highlights", &mut p.grade_highlights, small, false);
                    let used = inner.min_rect();
                    right.allocate_rect(ERect::from_min_max(r.min, pos2(r.right(), used.bottom())), Sense::hover());
                }
            });
        }
        k => {
            let (name, w) = match k {
                1 => ("Shadows", &mut p.grade_shadows),
                2 => ("Midtones", &mut p.grade_midtones),
                3 => ("Highlights", &mut p.grade_highlights),
                _ => ("Global", &mut p.grade_global),
            };
            grade_block(ui, dirty, name, w, (full - 40.0).clamp(80.0, 170.0), true);
        }
    }
}

/// One Color Grading zone: its title, its wheel `size` across and, under it, the Luminance slider
/// (and the Hue and Saturation sliders when `sliders`, the single-wheel views).
fn grade_block(ui: &mut egui::Ui, dirty: &mut bool, title: &str, w: &mut Wheel, size: f32, sliders: bool) {
    let t = Tokens::get(ui.ctx());
    ui.push_id(title, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(egui::RichText::new(trc(title)).color(t.text));
            grade_wheel(ui, dirty, title, w, size);
        });
        ui.add_space(4.0);
        if sliders {
            let hs = widgets::hue_stops();
            row(ui, dirty, "Hue", &mut w.hue, 0.0..=360.0, Some(&hs), 0.0);
            let sat = vec![Color32::from_gray(128), widgets::hue_color(w.hue.rem_euclid(360.0))];
            row(ui, dirty, "Saturation", &mut w.sat, 0.0..=100.0, Some(&sat), 0.0);
        }
        row(ui, dirty, "Luminance", &mut w.lum, -100.0..=100.0, Some(&tone_stops()), 0.0);
    });
}

/// Where a wheel's handle sits for hue `hue` (degrees, red at 3 o'clock, counter-clockwise) and
/// saturation `sat` (0 at the centre, 100 at the rim), on a wheel of `radius` around `c`.
pub(super) fn wheel_point(c: Pos2, radius: f32, hue: f32, sat: f32) -> Pos2 {
    let a = hue.to_radians();
    let r = radius * (sat / 100.0).clamp(0.0, 1.0);
    if !r.is_finite() || !a.is_finite() {
        return c;
    }
    pos2(c.x + r * a.cos(), c.y - r * a.sin())
}

/// The hue and saturation a pointer at `pos` picks on a wheel of `radius` around `c`.
pub(super) fn wheel_pick(c: Pos2, radius: f32, pos: Pos2) -> (f32, f32) {
    let d = pos - c;
    let hue = (-d.y).atan2(d.x).to_degrees().rem_euclid(360.0);
    let sat = (d.length() / radius.max(1.0) * 100.0).clamp(0.0, 100.0);
    (if hue.is_finite() { hue } else { 0.0 }, if sat.is_finite() { sat } else { 0.0 })
}

/// A Color Grading wheel: hue around the rim, saturation from the grey centre outwards, and a
/// handle a click or drag moves. A double-click sets the wheel back to neutral.
fn grade_wheel(ui: &mut egui::Ui, dirty: &mut bool, title: &str, w: &mut Wheel, size: f32) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click_and_drag());
    let c = rect.center();
    let radius = size / 2.0 - 2.0;
    let before = (w.hue, w.sat);
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked())
    {
        (w.hue, w.sat) = wheel_pick(c, radius, pos);
    }
    if widgets::second_click(ui, &resp) {
        (w.hue, w.sat) = (0.0, 0.0);
    }
    if (w.hue, w.sat) != before {
        *dirty = true;
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, Color32::from_gray(150));
    const N: u32 = 96;
    for k in 0..=N {
        let deg = 360.0 * k as f32 / N as f32;
        let hue = widgets::hue_color(deg);
        // Camera Raw's rim is a little muted, not full-strength primaries.
        let mute = |v: u8| ((u16::from(v) * 3 + 150) / 4) as u8;
        mesh.colored_vertex(wheel_point(c, radius, deg, 100.0), Color32::from_rgb(mute(hue.r()), mute(hue.g()), mute(hue.b())));
        if k > 0 {
            mesh.add_triangle(0, k, k + 1);
        }
    }
    let painter = ui.painter();
    painter.add(mesh);
    painter.circle_stroke(c, radius, Stroke::new(1.0, t.canvas));
    let cross = Stroke::new(1.0, Color32::from_gray(90));
    painter.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], cross);
    painter.line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], cross);
    let at = wheel_point(c, radius, w.hue, w.sat);
    let ring = if resp.hovered() || resp.dragged() { t.accent } else { Color32::WHITE };
    painter.circle_filled(at, 4.5, Color32::from_gray(30));
    painter.circle_stroke(at, 4.5, Stroke::new(1.5, ring));
    let name = trc(title).to_owned();
    resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(w.sat), &name));
}

/// The parametric curve's shape, 256 outputs for inputs 0…1: the Camera Raw pipeline run on a
/// grey ramp with only the curve's settings, minus the same ramp without them, so nothing else
/// the pipeline does shows in the graph. Cached per setting.
fn parametric_shape(ui: &egui::Ui, p: &CameraRaw) -> std::sync::Arc<Vec<f32>> {
    let key = [p.curve_highlights, p.curve_lights, p.curve_darks, p.curve_shadows, p.curve_splits[0], p.curve_splits[1], p.curve_splits[2]];
    let id = egui::Id::unique("camera-raw-parametric-shape");
    if let Some((k, shape)) = ui.data(|d| d.get_temp::<([f32; 7], std::sync::Arc<Vec<f32>>)>(id))
        && k == key
    {
        return shape;
    }
    let mut base = CameraRaw::default();
    (base.sharpen_amount, base.noise_luminance, base.noise_color) = (0.0, 0.0, 0.0);
    let mut curved = base.clone();
    (curved.curve_highlights, curved.curve_lights, curved.curve_darks, curved.curve_shadows, curved.curve_splits) =
        (p.curve_highlights, p.curve_lights, p.curve_darks, p.curve_shadows, p.curve_splits);
    let ramp: Vec<[f32; 4]> = (0..256).map(|i| i as f32 / 255.0).map(|v| [v, v, v, 1.0]).collect();
    let (mut a, mut b) = (ramp.clone(), ramp.clone());
    photocraft_algo::camera_raw::develop(&mut a, 256, 1, &curved, false);
    photocraft_algo::camera_raw::develop(&mut b, 256, 1, &base, false);
    let shape: Vec<f32> = ramp.iter().zip(a.iter().zip(&b)).map(|(r, (a, b))| (r[1] + a[1] - b[1]).clamp(0.0, 1.0)).collect();
    let shape = std::sync::Arc::new(shape);
    ui.data_mut(|d| d.insert_temp(id, (key, shape.clone())));
    shape
}

/// The image's tones (the mean of the R, G and B histograms), scaled to their tallest bin, for
/// the curve graphs' backdrop as Camera Raw draws it.
fn tone_backdrop(h: &photocraft_algo::histogram::RgbHistogram) -> Vec<f32> {
    let bins: Vec<f32> =
        (0..photocraft_algo::histogram::BINS).map(|i| h.channels.iter().map(|c| c.get(i).copied().unwrap_or(0) as f32).sum::<f32>() / 3.0).collect();
    // The edge bins hold everything clipped; they would flatten the rest.
    let peak = bins.iter().skip(1).take(bins.len().saturating_sub(2)).copied().fold(0.0, f32::max);
    if peak <= 0.0 { vec![0.0; bins.len()] } else { bins.iter().map(|v| (v / peak).min(1.0)).collect() }
}

/// Paints a curve graph's square: background, the tone backdrop, a 4 × 4 grid and the diagonal.
fn graph_backdrop(painter: &egui::Painter, rect: ERect, t: &Tokens, backdrop: &[f32]) {
    painter.rect_filled(rect, t.radius_sm, t.field);
    let n = backdrop.len().max(2) as f32 - 1.0;
    let fill = Color32::from_gray(78);
    for (i, h) in backdrop.iter().enumerate() {
        let x0 = rect.left() + rect.width() * i as f32 / n;
        let x1 = rect.left() + rect.width() * (i + 1) as f32 / n;
        let y = rect.bottom() - h * rect.height() * 0.9;
        if *h > 0.0 {
            painter.rect_filled(ERect::from_min_max(pos2(x0, y), pos2(x1.min(rect.right()), rect.bottom())), 0.0, fill);
        }
    }
    for i in 1..4 {
        let f = i as f32 / 4.0;
        painter.line_segment(
            [pos2(rect.left() + f * rect.width(), rect.top()), pos2(rect.left() + f * rect.width(), rect.bottom())],
            Stroke::new(1.0, t.separator),
        );
        painter.line_segment(
            [pos2(rect.left(), rect.top() + f * rect.height()), pos2(rect.right(), rect.top() + f * rect.height())],
            Stroke::new(1.0, t.separator),
        );
    }
    painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.0, t.separator));
}

/// Which of the parametric curve's regions (0 Shadows, 1 Darks, 2 Lights, 3 Highlights) the
/// input `x` (0…1) falls in, for `splits` in percent.
pub(super) fn parametric_region(x: f32, splits: [f32; 3]) -> usize {
    splits.iter().take_while(|s| x * 100.0 >= **s).count().min(3)
}

/// Camera Raw's parametric Curve: the graph (the image's tones behind the curve; dragging up or
/// down inside it raises or lowers the region under the pointer, which it shades), the region
/// splitter under it (three handles), then the four region sliders.
fn parametric_curve(ui: &mut egui::Ui, p: &mut CameraRaw, backdrop: &[f32], dirty: &mut bool) -> ERect {
    let t = Tokens::get(ui.ctx());
    let side = ui.available_width().min(260.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    graph_backdrop(&painter, rect, &t, backdrop);
    let splits = p.curve_splits;
    let region_at = |x: f32| parametric_region(((x - rect.left()) / side.max(1.0)).clamp(0.0, 1.0), splits);
    let drag_key = resp.id.with("region");
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(drag_key, region_at(pos.x)));
    }
    let active = if resp.dragged() { ui.data(|d| d.get_temp::<usize>(drag_key)) } else { resp.hover_pos().map(|pos| region_at(pos.x)) };
    if resp.dragged()
        && let Some(k) = active
    {
        let dy = -resp.drag_delta().y / side.max(1.0) * 200.0;
        let v = match k {
            0 => &mut p.curve_shadows,
            1 => &mut p.curve_darks,
            2 => &mut p.curve_lights,
            _ => &mut p.curve_highlights,
        };
        let nv = (*v + dy).clamp(-100.0, 100.0);
        if nv.is_finite() && nv != *v {
            *v = nv;
            *dirty = true;
        }
    }
    if let Some(k) = active {
        let edges = [0.0, splits[0], splits[1], splits[2], 100.0];
        let (a, b) = (edges.get(k).copied().unwrap_or(0.0), edges.get(k + 1).copied().unwrap_or(100.0));
        let band = ERect::from_min_max(pos2(rect.left() + a / 100.0 * side, rect.top()), pos2(rect.left() + b / 100.0 * side, rect.bottom()));
        painter.rect_filled(band, 0.0, Color32::from_white_alpha(14));
    }
    let shape = parametric_shape(ui, p);
    let pts: Vec<Pos2> = shape.iter().enumerate().map(|(i, y)| pos2(rect.left() + i as f32 / 255.0 * side, rect.bottom() - y * side)).collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, t.text)));
    let name = trc("Parametric Curve").to_owned();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &name));
    // The splitter: three handles under the graph, each kept 5 % from its neighbours.
    let (strip, _) = ui.allocate_exact_size(vec2(side, 14.0), Sense::hover());
    ui.painter().rect_filled(ERect::from_min_max(pos2(strip.left(), strip.top() + 2.0), pos2(strip.right(), strip.top() + 5.0)), 0.0, Color32::from_gray(90));
    for k in 0..3 {
        let x = strip.left() + p.curve_splits[k] / 100.0 * side;
        let hit = ERect::from_center_size(pos2(x, strip.center().y), vec2(12.0, 14.0));
        let r = ui.interact(hit, resp.id.with(("split", k)), Sense::click_and_drag());
        if r.dragged()
            && let Some(pos) = r.interact_pointer_pos()
        {
            let (lo, hi) = split_bounds(&p.curve_splits, k);
            let v = ((pos.x - strip.left()) / side.max(1.0) * 100.0).clamp(lo, hi).round();
            if v.is_finite() && v != p.curve_splits[k] {
                p.curve_splits[k] = v;
                *dirty = true;
            }
        }
        if widgets::second_click(ui, &r) {
            p.curve_splits[k] = reset_split(&p.curve_splits, k);
            *dirty = true;
        }
        let ink = if r.hovered() || r.dragged() { t.accent } else { t.text };
        let tip = strip.top() + 4.0;
        ui.painter().add(egui::Shape::convex_polygon(vec![pos2(x, tip), pos2(x + 5.0, tip + 8.0), pos2(x - 5.0, tip + 8.0)], ink, Stroke::new(1.0, t.canvas)));
        let split_name = format!("{} {}", trc("Split"), k + 1);
        r.widget_info(|| egui::WidgetInfo::slider(true, f64::from(p.curve_splits[k]), &split_name));
    }
    ui.add_space(6.0);
    rect
}

/// Where split `k` may go: 5 % from its neighbours (and from the ends), low end first.
pub(super) fn split_bounds(s: &[f32; 3], k: usize) -> (f32, f32) {
    let lo = if k == 0 { 5.0 } else { s.get(k - 1).map_or(5.0, |v| v + 5.0) };
    let hi = s.get(k + 1).map_or(95.0, |v| v - 5.0);
    (lo.min(hi), hi.max(lo))
}

/// Split `k`'s default (double-click), kept between its neighbours so the handles stay ordered.
pub(super) fn reset_split(s: &[f32; 3], k: usize) -> f32 {
    let (lo, hi) = split_bounds(s, k);
    CameraRaw::default().curve_splits.get(k).copied().unwrap_or(50.0).clamp(lo, hi)
}

/// Small point-curve editor: click to add, drag to move, right-click to delete. `color` draws the
/// curve (the channel's colour).
fn curve_editor(
    ui: &mut egui::Ui,
    points_of: &mut Vec<[f32; 2]>,
    color: Color32,
    backdrop: &[f32],
    dirty: &mut bool,
    state: &mut crate::state::PointCurveState,
) -> ERect {
    let t = Tokens::get(ui.ctx());
    let side = ui.available_width().min(260.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    graph_backdrop(&painter, rect, &t, backdrop);
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    // The identity handles are presentation until a gesture actually edits them.
    let mut points = if points_of.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { points_of.clone() };
    if crate::point_curve::interact(ui, &resp, rect, &mut points, state).changed {
        *points_of = points.clone();
        *dirty = true;
    }
    let lut = curve_lut(&points, 256);
    let pts: Vec<Pos2> = lut.iter().enumerate().map(|(i, value)| to_screen(i as f32, value * 255.0)).collect();
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
    for (i, c) in points.iter().enumerate() {
        let center = to_screen(c[0], c[1]);
        if state.selected == Some(i) {
            painter.circle_filled(center, 3.0, t.accent);
        }
        painter.circle_stroke(center, 4.0, Stroke::new(1.5, t.accent));
    }
    rect
}

/// Round chips choosing an item, as Camera Raw's Curve channels and Color Mixer colours: a filled
/// circle each (`None`: the parametric curve's outline), the chosen one ringed.
fn chips(ui: &mut egui::Ui, items: &[(Option<Color32>, &str)], selected: &mut usize) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    ui.horizontal(|ui| {
        // Every chip on one row, as Camera Raw's eight Color Mixer colours: narrower when the
        // column can't fit 24 pt each.
        let n = items.len().max(1) as f32;
        let gap = ui.spacing().item_spacing.x;
        let side = ((ui.available_width() - gap * (n - 1.0)) / n).clamp(1.0, 24.0);
        for (i, (fill, name)) in items.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(side, 24.0), Sense::click());
            let on = *selected == i;
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, *name));
            let c = rect.center();
            match fill {
                Some(color) => {
                    ui.painter().circle_filled(c, 7.5, *color);
                }
                None => {
                    ui.painter().circle_stroke(c, 7.0, Stroke::new(1.5, t.text_dim));
                    ui.painter().line_segment([c + vec2(-4.0, 3.0), c + vec2(4.0, -3.0)], Stroke::new(1.5, t.text_dim));
                }
            }
            let ring = (side / 2.0 - 1.5).min(10.5);
            if on {
                ui.painter().circle_stroke(c, ring, Stroke::new(1.5, t.text));
            } else if resp.hovered() {
                ui.painter().circle_stroke(c, ring, Stroke::new(1.0, t.text_faint));
            }
            if resp.on_hover_text(*name).clicked() && !on {
                *selected = i;
                changed = true;
            }
        }
    });
    changed
}

/// What the header's menu asks the dialog to do with files.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FileAction {
    Load,
    Save,
}

/// "Edit" with Auto and the settings menu (Load / Save Settings, Reset to Open / Default).
pub(super) fn header(ui: &mut egui::Ui, d: &mut CameraRawDialog, dirty: &mut bool) -> Option<FileAction> {
    let t = Tokens::get(ui.ctx());
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Edit")).font(crate::theme::semibold(12.5)).color(t.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = widgets::panel_menu_button(ui);
            egui::Popup::menu(&more).show(|ui| {
                widgets::style_spectrum_popup_menu(ui);
                ui.set_min_width(190.0);
                if ui.button(tl!("Load Settings…")).on_hover_text(tl!("Load settings from a Camera Raw .xmp file")).clicked() {
                    action = Some(FileAction::Load);
                }
                if ui.button(tl!("Save Settings…")).on_hover_text(tl!("Save these settings as a Camera Raw .xmp file")).clicked() {
                    action = Some(FileAction::Save);
                }
                ui.separator();
                if ui.button(tl!("Reset to Open")).clicked() {
                    let mut p = d.original.clone();
                    p.pixel_scale = d.params.pixel_scale;
                    d.params = p;
                    *dirty = true;
                }
                if ui.button(tl!("Reset to Default")).clicked() {
                    d.params = super::panels::defaults_for(&d.params);
                    *dirty = true;
                }
            });
            // Auto: Camera Raw's one-click tone from the histogram; the sliders show what it
            // chose, so each can be adjusted afterwards.
            if widgets::compact_button(ui, tl!("Auto"), 19.0).on_hover_text(tl!("Set the tone sliders from the image")).clicked() {
                photocraft_algo::camera_raw::auto_tone(&d.proxy, &mut d.params);
                *dirty = true;
            }
        });
    });
    ui.add_space(4.0);
    action
}

/// The panels, in Camera Raw's order, inside the column's scroll area.
pub(super) fn panels(ui: &mut egui::Ui, d: &mut CameraRawDialog, dirty: &mut bool) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let def = CameraRaw::default();
    let (ts, tn, vivid) = (temp_stops(), tint_stops(), vivid_stops());
    let backdrop = if d.panels.open.contains("curve") { tone_backdrop(d.histogram()) } else { Vec::new() };
    let d_ref = &mut *d;
    let p = &mut d_ref.params;
    let panels = &mut d_ref.panels;
    let alt = &mut d_ref.scope.alt_tone;
    if section(ui, panels, "light", "Light", dirty) {
        tone_row(ui, dirty, alt, ToneZone::Exposure, &mut p.exposure);
        row(ui, dirty, "Contrast", &mut p.contrast, -100.0..=100.0, Some(&tone_stops()), 0.0);
        tone_row(ui, dirty, alt, ToneZone::Highlights, &mut p.highlights);
        tone_row(ui, dirty, alt, ToneZone::Shadows, &mut p.shadows);
        tone_row(ui, dirty, alt, ToneZone::Whites, &mut p.whites);
        tone_row(ui, dirty, alt, ToneZone::Blacks, &mut p.blacks);
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "color", "Color", dirty) {
        let t = Tokens::get(ui.ctx());
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // As Shot is the image's own balance (no Temperature / Tint shift); moving either
                // slider makes it Custom, as in Camera Raw.
                let mut custom = p.temperature != 0.0 || p.tint != 0.0;
                if widgets::dropdown(ui, "cr-white-balance", trc("White Balance"), &mut custom, &[(false, tl!("As Shot")), (true, tl!("Custom"))], 112.0)
                    && !custom
                {
                    (p.temperature, p.tint) = (0.0, 0.0);
                    *dirty = true;
                }
                // The label takes what is left, cut short (full text on hover) when too long.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(trc("White Balance")).color(t.text)).truncate());
                });
            });
        });
        ui.add_space(10.0);
        row(ui, dirty, "Temperature", &mut p.temperature, -100.0..=100.0, Some(&ts), def.temperature);
        row(ui, dirty, "Tint", &mut p.tint, -100.0..=100.0, Some(&tn), def.tint);
        row(ui, dirty, "Vibrance", &mut p.vibrance, -100.0..=100.0, Some(&vivid), def.vibrance);
        row(ui, dirty, "Saturation", &mut p.saturation, -100.0..=100.0, Some(&vivid), def.saturation);
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "effects", "Effects", dirty) {
        row(ui, dirty, "Texture", &mut p.texture, -100.0..=100.0, None, def.texture);
        row(ui, dirty, "Clarity", &mut p.clarity, -100.0..=100.0, None, def.clarity);
        row(ui, dirty, "Dehaze", &mut p.dehaze, -100.0..=100.0, None, def.dehaze);
        ui.push_id("vignette", |ui| {
            let key = ui.make_persistent_id("more");
            let mut open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
            let _ = slider_more(ui, dirty, "Vignetting", &mut p.vignette_amount, -100.0..=100.0, None, def.vignette_amount, Some(&mut open));
            ui.data_mut(|d| d.insert_temp(key, open));
            if open {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(trc("Style")).color(Tokens::get(ui.ctx()).text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut style = p.vignette_style.clone();
                        if widgets::dropdown(
                            ui,
                            "cr-vig-style",
                            tl!("Vignette Style"),
                            &mut style,
                            &[
                                ("highlightPriority".to_string(), tl!("Highlight Priority")),
                                ("colorPriority".to_string(), tl!("Color Priority")),
                                ("paintOverlay".to_string(), tl!("Paint Overlay")),
                            ],
                            150.0,
                        ) {
                            p.vignette_style = style;
                            *dirty = true;
                        }
                    });
                });
                ui.add_space(6.0);
                row(ui, dirty, "Midpoint", &mut p.vignette_midpoint, 0.0..=100.0, None, def.vignette_midpoint);
                row(ui, dirty, "Roundness", &mut p.vignette_roundness, -100.0..=100.0, None, def.vignette_roundness);
                row(ui, dirty, "Feather", &mut p.vignette_feather, 0.0..=100.0, None, def.vignette_feather);
                row(ui, dirty, "Highlights", &mut p.vignette_highlights, 0.0..=100.0, None, def.vignette_highlights);
            }
        });
        ui.push_id("grain", |ui| {
            let key = ui.make_persistent_id("more");
            let mut open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
            let _ = slider_more(ui, dirty, "Grain", &mut p.grain_amount, 0.0..=100.0, None, def.grain_amount, Some(&mut open));
            ui.data_mut(|d| d.insert_temp(key, open));
            if open {
                row(ui, dirty, "Size", &mut p.grain_size, 0.0..=100.0, None, def.grain_size);
                row(ui, dirty, "Roughness", &mut p.grain_roughness, 0.0..=100.0, None, def.grain_roughness);
            }
        });
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "curve", "Curve", dirty) {
        let t = Tokens::get(ui.ctx());
        let items = [
            (None, tl!("Parametric Curve")),
            (Some(Color32::from_gray(235)), tl!("Point Curve")),
            (Some(Color32::from_rgb(230, 70, 70)), tl!("Red Channel")),
            (Some(Color32::from_rgb(80, 200, 90)), tl!("Green Channel")),
            (Some(Color32::from_rgb(80, 130, 240)), tl!("Blue Channel")),
        ];
        let mut channel = panels.curve_channel.min(CURVE_CHANNELS.len() - 1);
        if chips(ui, &items, &mut channel) {
            panels.curve_channel = channel;
            d_ref.curve_state = Default::default();
        }
        ui.add_space(2.0);
        let curve = match channel {
            1 => Some((&mut p.point_curve, t.text)),
            2 => Some((&mut p.point_curve_red, Color32::from_rgb(230, 70, 70))),
            3 => Some((&mut p.point_curve_green, Color32::from_rgb(80, 200, 90))),
            4 => Some((&mut p.point_curve_blue, Color32::from_rgb(80, 130, 240))),
            _ => None,
        };
        match curve {
            Some((points, color)) => d_ref.curve_rect = Some(curve_editor(ui, points, color, &backdrop, dirty, &mut d_ref.curve_state)),
            None => {
                d_ref.curve_rect = Some(parametric_curve(ui, p, &backdrop, dirty));
                row(ui, dirty, "Highlights", &mut p.curve_highlights, -100.0..=100.0, Some(&tone_stops()), def.curve_highlights);
                row(ui, dirty, "Lights", &mut p.curve_lights, -100.0..=100.0, Some(&tone_stops()), def.curve_lights);
                row(ui, dirty, "Darks", &mut p.curve_darks, -100.0..=100.0, Some(&tone_stops()), def.curve_darks);
                row(ui, dirty, "Shadows", &mut p.curve_shadows, -100.0..=100.0, Some(&tone_stops()), def.curve_shadows);
            }
        }
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "detail", "Detail", dirty) {
        if let Some(raw) = d_ref.raw_open.as_mut() {
            super::controls::demosaic_row(ui, &mut raw.demosaic);
        }
        widgets::section_label(ui, trc("Sharpening"));
        row(ui, dirty, "Amount", &mut p.sharpen_amount, 0.0..=150.0, None, def.sharpen_amount);
        row(ui, dirty, "Radius", &mut p.sharpen_radius, 0.5..=3.0, None, def.sharpen_radius);
        row(ui, dirty, "Detail", &mut p.sharpen_detail, 0.0..=100.0, None, def.sharpen_detail);
        row(ui, dirty, "Masking", &mut p.sharpen_masking, 0.0..=100.0, None, def.sharpen_masking);
        sub(ui, "Noise Reduction");
        row(ui, dirty, "Luminance", &mut p.noise_luminance, 0.0..=100.0, None, def.noise_luminance);
        row(ui, dirty, "Luminance Detail", &mut p.noise_luminance_detail, 0.0..=100.0, None, def.noise_luminance_detail);
        row(ui, dirty, "Color", &mut p.noise_color, 0.0..=100.0, None, def.noise_color);
        row(ui, dirty, "Color Detail", &mut p.noise_color_detail, 0.0..=100.0, None, def.noise_color_detail);
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "colorMixer", "Color Mixer", dirty) {
        color_mixer(ui, p, panels, dirty);
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "colorGrading", "Color Grading", dirty) {
        color_grading(ui, p, panels, dirty);
        ui.add_space(4.0);
        row(ui, dirty, "Blending", &mut p.grade_blending, 0.0..=100.0, None, def.grade_blending);
        row(ui, dirty, "Balance", &mut p.grade_balance, -100.0..=100.0, Some(&tone_stops()), def.grade_balance);
        ui.add_space(PANEL_END);
    }
    if section(ui, panels, "geometry", "Geometry", dirty) {
        super::geometry_panel::body(ui, p, dirty);
        ui.add_space(PANEL_END);
    }
    // Camera Raw's Calibration panel: a tint for the shadows and each primary's hue and
    // saturation (Process Version is implied: 2012 and later).
    if section(ui, panels, "calibration", "Calibration", dirty) {
        sub(ui, "Shadows");
        row(ui, dirty, "Tint", &mut p.shadow_tint, -100.0..=100.0, Some(&tn), def.shadow_tint);
        for (i, (name, base)) in [("Red Primary", 0.0), ("Green Primary", 120.0), ("Blue Primary", 240.0)].into_iter().enumerate() {
            sub(ui, name);
            ui.push_id(name, |ui| {
                if let Some(h) = p.calibration_hue.get_mut(i) {
                    row(ui, dirty, "Hue", h, -100.0..=100.0, Some(&hue_span(base, 40.0)), 0.0);
                }
                if let Some(s) = p.calibration_sat.get_mut(i) {
                    row(ui, dirty, "Saturation", s, -100.0..=100.0, Some(&[Color32::from_gray(128), widgets::hue_color(base)]), 0.0);
                }
            });
        }
        ui.add_space(PANEL_END);
    }
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    ui.painter().rect_filled(rule, 0.0, Tokens::get(ui.ctx()).canvas);
}

/// Color Mixer: Adjust HSL (Hue / Saturation / Luminance / All tabs over the eight bands) or
/// Color (the eight colours as chips, the chosen one's three sliders).
fn color_mixer(ui: &mut egui::Ui, p: &mut CameraRaw, panels: &mut PanelState, dirty: &mut bool) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Adjust")).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            widgets::dropdown(
                ui,
                "cr-mixer-adjust",
                tl!("Adjust"),
                &mut panels.mixer_adjust,
                &[(MixerAdjust::Hsl, tl!("HSL")), (MixerAdjust::Color, trc("Color"))],
                110.0,
            );
        });
    });
    match panels.mixer_adjust {
        MixerAdjust::Hsl => {
            // Text tabs as Camera Raw's: the chosen one bright and underlined, the rest dimmed.
            let t = Tokens::get(ui.ctx());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // Long translations shrink the tabs (text and padding) so all four fit the column.
                let natural: f32 = MIXER_TABS
                    .iter()
                    .map(|(_, source)| ui.painter().layout_no_wrap(trc(source).to_string(), egui::FontId::proportional(12.0), t.text).size().x + 14.0)
                    .sum();
                let fit = (ui.available_width() / natural.max(1.0)).clamp(0.6, 1.0);
                for (i, (_, source)) in MIXER_TABS.iter().enumerate() {
                    let name = trc(source);
                    let on = panels.mixer_tab == i;
                    let galley = ui.painter().layout_no_wrap(name.to_string(), egui::FontId::proportional(12.0 * fit), t.text);
                    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 14.0 * fit, 22.0), Sense::click());
                    let color = if on || response.hovered() { t.text } else { t.text_dim };
                    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
                    if on {
                        ui.painter().line_segment(
                            [pos2(rect.left() + 5.0, rect.bottom() - 1.0), pos2(rect.right() - 5.0, rect.bottom() - 1.0)],
                            Stroke::new(2.0, t.text),
                        );
                    }
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, name));
                    if response.clicked() {
                        panels.mixer_tab = i;
                    }
                }
            });
            ui.add_space(8.0);
            let tabs: &[usize] = if panels.mixer_tab == MIXER_ALL { &[0, 1, 2] } else { std::slice::from_ref(&panels.mixer_tab) };
            for &tab in tabs {
                if panels.mixer_tab == MIXER_ALL {
                    sub(ui, MIXER_TABS.get(tab).map_or("", |t| t.1));
                }
                let arr = match tab {
                    0 => &mut p.hsl_hue,
                    1 => &mut p.hsl_sat,
                    _ => &mut p.hsl_lum,
                };
                ui.push_id(("mixer", tab), |ui| {
                    for (k, (name, v)) in BANDS.iter().zip(arr.iter_mut()).enumerate() {
                        row(ui, dirty, name, v, -100.0..=100.0, Some(&mixer_gradient(tab, k)), 0.0);
                    }
                });
            }
        }
        MixerAdjust::Color => {
            let bands = &photocraft_algo::camera_raw::HSL_BANDS;
            let items: Vec<(Option<Color32>, &str)> = BANDS.iter().zip(bands).map(|(name, h)| (Some(widgets::hue_color(*h)), trc(name))).collect();
            let mut k = panels.mixer_color.min(BANDS.len() - 1);
            if chips(ui, &items, &mut k) {
                panels.mixer_color = k;
            }
            ui.add_space(2.0);
            ui.push_id(("mixer-color", k), |ui| {
                for (tab, (_, label), arr) in [(0, MIXER_TABS[0], &mut p.hsl_hue), (1, MIXER_TABS[1], &mut p.hsl_sat), (2, MIXER_TABS[2], &mut p.hsl_lum)] {
                    if let Some(v) = arr.get_mut(k) {
                        row(ui, dirty, label, v, -100.0..=100.0, Some(&mixer_gradient(tab, k)), 0.0);
                    }
                }
            });
        }
    }
}
