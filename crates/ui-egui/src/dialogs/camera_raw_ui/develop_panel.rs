//! The Edit panel for a raw the shared develop engine (craft-develop, LightCraft's) develops: the
//! dialog's settings are its [`DevelopSettings`], edited here directly, in LightCraft's develop
//! look ([`super::lc`]): an "Edit" title with Auto, B&W and Reset, the Profile row, then the
//! sections under chevron headers (with their eye where the engine has a switch for them), the
//! sliders in LightCraft's style, and the flyouts (Curve, Color Mixer, Color Grading) inside them.
//!
//! The sliders come from the shared control registry ([`controls::CONTROLS`]): each section lists
//! every control of its [`Section`] in registry order, with the spec's label, range, default
//! (double-click resets to it), step, decimals and track. A control LightCraft adds to a section,
//! or a section it adds, shows here on a dependency bump without a change to this file. What the
//! registry does not describe is drawn by hand on the same settings: the profile menu, Auto, B&W,
//! Reset, the white-balance mode menu, the point curve, the Color Mixer's Hue / Saturation /
//! Luminance tabs, vignette style, the Optics and Lens Blur switches and Upright.
//!
//! Not drawn: Point Color (indexed controls, which need a sampling tool on the preview), the HDR
//! switch (the sliders of HDR editing show only once settings have it on: an opened raw is a
//! 16-bit document) and AI Denoise's amount (PhotoCraft has no denoise model).

use craft_develop_model::controls::{self, ControlSpec, Section, Track};
use craft_develop_model::{DevelopSettings, Treatment, Upright, VignetteStyle, WbMode, WbScale, WhiteBalance};
use egui::{Rect, Sense, Stroke, pos2, vec2};
use photocraft_io::develop_bridge::{DevelopEdit, WbBasis};
use photocraft_io::raw_develop::DevelopSource;

use super::lc::icons::Icon;
use super::lc::widgets::{self as w, snap};
use super::lc::{self, Lc};
use crate::camera_raw_scope_ui::ToneZone;

/// Controls the panel leaves out whatever the settings (module docs): AI Denoise's amount.
pub(crate) const NOT_SHOWN: &[&str] = &["enhance.denoise"];

/// The panel's own state: open sections and flyouts, tabs, the curve channel, and what it drew last.
pub(crate) struct DevelopPanel {
    /// Color Mixer tab: 0 Hue, 1 Saturation, 2 Luminance.
    pub(crate) mixer_tab: usize,
    /// Point curve channel: 0 master, 1 red, 2 green, 3 blue.
    pub(crate) curve_channel: usize,
    pub(crate) curve_state: crate::state::PointCurveState,
    /// Every section and flyout open (the control channel's `expandAll`, and tests).
    pub(crate) expand_all: bool,
    /// The sections and flyouts open (by id: "light", "curve"…).
    pub(crate) open: Vec<String>,
    /// The control ids drawn in the last frame, in order.
    pub(crate) drawn: Vec<&'static str>,
}

impl Default for DevelopPanel {
    fn default() -> Self {
        // Light open, as LightCraft opens a photo
        DevelopPanel { mixer_tab: 0, curve_channel: 0, curve_state: Default::default(), expand_all: false, open: vec!["light".into()], drawn: Vec::new() }
    }
}

impl DevelopPanel {
    fn is_open(&self, id: &str) -> bool {
        self.expand_all || self.open.iter().any(|o| o == id)
    }

    fn toggle(&mut self, id: &str) {
        if let Some(i) = self.open.iter().position(|o| o == id) {
            self.open.remove(i);
        } else {
            self.open.push(id.to_string());
        }
    }
}

/// What a frame of the panel did beyond editing the settings.
#[derive(Default)]
pub(crate) struct PanelOut {
    /// Auto was pressed (the other selected raws get their own Auto).
    pub(crate) auto: bool,
    pub(crate) curve_rect: Option<egui::Rect>,
}

/// The tools the panel's controls take up or put down.
pub(crate) struct Tools<'a> {
    pub(crate) guides: &'a mut crate::camera_raw_guides_ui::GuideTool,
    pub(crate) sampler: &'a mut bool,
    /// The tone zone an Alt-drag of a Light slider shows the clipping of.
    pub(crate) alt: &'a mut Option<ToneZone>,
}

pub(crate) fn tr(s: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", s)
}

/// The panel's order of the sections it lays out itself (LightCraft's Edit panel order, with
/// Geometry, a tool of its own there, before Lens Blur).
const ORDER: [Section; 15] = [
    Section::Profile,
    Section::Light,
    Section::Curve,
    Section::Color,
    Section::Mixer,
    Section::BwMix,
    Section::Grading,
    Section::Effects,
    Section::Vignette,
    Section::Grain,
    Section::Detail,
    Section::Optics,
    Section::Geometry,
    Section::LensBlur,
    Section::Calibration,
];

/// The sections the registry has controls in: those in [`ORDER`] first, then any other in
/// registry order (a section is where its first control is).
pub(crate) fn sections() -> Vec<Section> {
    let mut out: Vec<Section> = ORDER.iter().copied().filter(|s| controls::in_section(*s).next().is_some()).collect();
    for c in controls::CONTROLS {
        if !out.contains(&c.section) {
            out.push(c.section);
        }
    }
    out
}

/// Whether control `id` is drawn with settings `s`: the sliders of a switch that is off are not.
pub(crate) fn shown(id: &str, s: &DevelopSettings) -> bool {
    if NOT_SHOWN.contains(&id) {
        return false;
    }
    if id.starts_with("hdr.") {
        return s.hdr.enabled;
    }
    if id.starts_with("optics.profile") {
        return s.optics.lens_profile;
    }
    true
}

/// Whether control `id` takes edits: a setting that does nothing until its amount is set is
/// greyed out, as in LightCraft.
fn enabled(id: &str, s: &DevelopSettings) -> bool {
    match id {
        "vignette.amount" | "grain.amount" | "detail.sharpenAmount" => true,
        "detail.sharpenRadius" | "detail.sharpenDetail" | "detail.sharpenMasking" => s.detail.sharpen_amount > 0.0,
        "optics.vignettingMidpoint" => s.optics.vignetting != 0.0,
        "optics.defringePurpleHueLo" | "optics.defringePurpleHueHi" => s.optics.defringe_purple_amount > 0.0,
        "optics.defringeGreenHueLo" | "optics.defringeGreenHueHi" => s.optics.defringe_green_amount > 0.0,
        _ if id.starts_with("vignette.") => s.vignette.amount != 0.0,
        _ if id.starts_with("grain.") => s.grain.amount != 0.0,
        _ if id.starts_with("lensBlur.") => s.lens_blur.enabled,
        _ => true,
    }
}

/// A Color Mixer control on tab `tab`: hue, saturation and luminance tracks each have their own,
/// anything else shows on every tab.
fn on_tab(track: Track, tab: usize) -> bool {
    match track {
        Track::Hue { .. } => tab == 0,
        Track::Sat { .. } => tab == 1,
        Track::Lum { .. } => tab == 2,
        _ => true,
    }
}

/// The relative white-balance scale of a raw without a Kelvin scale (a fitted look): −100..100
/// about the as-shot white, as LightCraft shows it.
const REL_TEMP: ControlSpec =
    ControlSpec { id: "wb.temp", label: "Temp", section: Section::Color, min: -100.0, max: 100.0, default: 0.0, step: 1.0, decimals: 0, track: Track::Temp };
const REL_TINT: ControlSpec =
    ControlSpec { id: "wb.tint", label: "Tint", section: Section::Color, min: -100.0, max: 100.0, default: 0.0, step: 1.0, decimals: 0, track: Track::Tint };

fn k_to_rel(k: f64) -> f64 {
    if k.is_finite() && k > 0.0 { ((1e6 / 6500.0 - 1e6 / k) / 0.8).clamp(-100.0, 100.0) } else { 0.0 }
}

/// The spec a control is shown with, its shown value and the stored value of a shown one.
fn view(spec: &'static ControlSpec, v: f64, basis: WbBasis) -> (&'static ControlSpec, f64, fn(f64) -> f64) {
    match (spec.id, basis) {
        ("wb.temp", WbBasis::Relative) => (&REL_TEMP, k_to_rel(v), craft_develop_model::crs::rel_to_kelvin),
        ("wb.tint", WbBasis::Relative) => (&REL_TINT, v.clamp(-100.0, 100.0), |v| v),
        _ => (spec, v, |v| v),
    }
}

fn tone_zone(id: &str) -> Option<ToneZone> {
    Some(match id {
        "light.exposure" => ToneZone::Exposure,
        "light.highlights" => ToneZone::Highlights,
        "light.shadows" => ToneZone::Shadows,
        "light.whites" => ToneZone::Whites,
        "light.blacks" => ToneZone::Blacks,
        _ => return None,
    })
}

/// One registry control as a LightCraft slider row.
fn row(ui: &mut egui::Ui, drawn: &mut Vec<&'static str>, s: &mut DevelopSettings, spec: &'static ControlSpec, basis: WbBasis, alt: &mut Option<ToneZone>) {
    let Some(stored) = controls::get(s, spec.id) else { return };
    let (shown_spec, value, store) = view(spec, stored, basis);
    let on = enabled(spec.id, s);
    let out = ui.push_id(spec.id, |ui| w::slider(ui, shown_spec, value, on, tr(shown_spec.label))).inner;
    drawn.push(spec.id);
    if out.alt_drag
        && let Some(zone) = tone_zone(spec.id)
    {
        *alt = Some(zone);
    }
    if let Some(v) = out.value.filter(|v| v.is_finite()) {
        controls::set(s, spec.id, store(snap(shown_spec, v)));
        if spec.id.starts_with("wb.") {
            s.wb.mode = WbMode::Custom;
            s.wb.scale = scale(basis);
        }
    }
}

fn scale(basis: WbBasis) -> WbScale {
    match basis {
        WbBasis::Kelvin { .. } => WbScale::Kelvin,
        WbBasis::Relative => WbScale::Legacy,
    }
}

/// Every control of `section` drawn with `s`, in registry order; `keep` filters them further.
fn rows(
    ui: &mut egui::Ui,
    st: &mut DevelopPanel,
    s: &mut DevelopSettings,
    section: Section,
    basis: WbBasis,
    alt: &mut Option<ToneZone>,
    keep: impl Fn(&ControlSpec) -> bool,
) {
    for spec in controls::in_section(section) {
        if shown(spec.id, s) && keep(spec) {
            row(ui, &mut st.drawn, s, spec, basis, alt);
        }
    }
}

/// The engine's switch for a section (its eye), where it has one.
fn eye_id(section: Section) -> Option<&'static str> {
    Some(match section {
        Section::Light => "light",
        Section::Color => "color",
        Section::Effects => "effects",
        Section::Detail => "detail",
        Section::Optics => "optics",
        Section::Geometry => "geometry",
        Section::LensBlur => "lensBlur",
        Section::Calibration => "calibration",
        _ => return None,
    })
}

/// A section under a LightCraft header, its body when open, and the divider below.
fn section(
    ui: &mut egui::Ui,
    st: &mut DevelopPanel,
    s: &mut DevelopSettings,
    id: &str,
    title: &str,
    eye: Option<&str>,
    body: impl FnOnce(&mut egui::Ui, &mut DevelopPanel, &mut DevelopSettings),
) {
    let open = st.is_open(id);
    let (resp, toggled) = w::section_header(ui, id, tr(title), open, eye.map(|e| s.section_enabled(e)));
    if let (Some(on), Some(e)) = (toggled, eye) {
        s.set_section_enabled(e, on);
    } else if resp.clicked() {
        st.toggle(id);
    }
    if open {
        body(ui, st, s);
        ui.add_space(8.0);
    }
    w::divider(ui);
}

/// A flyout inside a section; `body` when open.
fn flyout(ui: &mut egui::Ui, st: &mut DevelopPanel, id: &str, title: &str, icon: Icon, body: impl FnOnce(&mut egui::Ui, &mut DevelopPanel)) {
    let open = st.is_open(id);
    if w::flyout_row(ui, id, title, icon, open).clicked() {
        st.toggle(id);
    }
    if open {
        body(ui, st);
        ui.add_space(4.0);
    }
}

/// "Edit", then Auto, B&W and Reset, as LightCraft's header has them.
fn header(ui: &mut egui::Ui, unedited: &DevelopSettings, s: &mut DevelopSettings, source: Option<&DevelopSource>, out: &mut PanelOut) {
    w::title(ui, tr("Edit"));
    w::padded(ui, 8, 14, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if let Some(source) = source
                && w::text_button(ui, tr("Auto"), false).on_hover_text(tr("Set the tone sliders from the image")).clicked()
            {
                source.auto_tone(s).apply(s);
                out.auto = true;
            }
            let bw = s.treatment == Treatment::Bw;
            if w::text_button(ui, tr("B&W"), bw).on_hover_text(tr("Black & White")).clicked() {
                s.treatment = if bw { Treatment::Color } else { Treatment::Bw };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if w::text_button(ui, tr("Reset"), false).on_hover_text(tr("Reset all settings")).clicked() {
                    *s = unedited.clone();
                }
            });
        });
    });
}

/// The Profile row (a borderless dropdown of craft-develop's profiles) and its Amount.
fn profile(ui: &mut egui::Ui, st: &mut DevelopPanel, s: &mut DevelopSettings, basis: WbBasis, alt: &mut Option<ToneZone>) {
    let t = Lc::get(ui.ctx());
    w::padded(ui, 14, 14, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("Profile")).font(t.font(13.0)).color(t.text_dim));
            let known = photocraft_io::raw_develop::PROFILES.iter().find(|p| p.id == s.profile.id);
            // a profile this build doesn't know (an imported one in LightCraft): kept, and shown by id
            let name = known.map_or(s.profile.id.clone(), |p| tr(p.name).to_string());
            let r = w::dropdown(ui, &name, t.font(15.0), t.text_label);
            egui::Popup::menu(&r).show(|ui| {
                lc::style(ui);
                for p in photocraft_io::raw_develop::PROFILES {
                    if w::menu_item(ui, tr(p.name), p.id == s.profile.id) {
                        s.profile.id = p.id.to_string();
                    }
                }
            });
        });
    });
    if s.profile.id != "lc.color" {
        rows(ui, st, s, Section::Profile, basis, alt, |_| true);
        ui.add_space(6.0);
    }
}

/// The white-balance row: the mode menu (a preset or Auto sets its white, as LightCraft's does).
fn white_balance(ui: &mut egui::Ui, s: &mut DevelopSettings, source: Option<&DevelopSource>, basis: WbBasis) {
    let t = Lc::get(ui.ctx());
    w::padded(ui, 2, 2, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tr("White Balance")).font(t.font(13.0)).color(t.text_dim));
            let r = w::dropdown(ui, tr(s.wb.mode.label()), t.font(14.0), t.text_label);
            egui::Popup::menu(&r).show(|ui| {
                lc::style(ui);
                for mode in WbMode::ALL {
                    if mode == WbMode::Custom || !w::menu_item(ui, tr(mode.label()), s.wb.mode == mode) || mode == s.wb.mode {
                        continue;
                    }
                    match source {
                        None => s.wb.mode = mode,
                        Some(source) => {
                            let (temp, tint) = source.white_balance_for(mode);
                            s.wb = WhiteBalance { mode, temp: temp.clamp(2000.0, 50000.0), tint: tint.clamp(-150.0, 150.0), scale: scale(basis) };
                        }
                    }
                }
            });
        });
    });
}

/// The point curve of the chosen channel (0..1 in the model, 0..255 in the editor), drawn as
/// LightCraft's: channel dots above a dark square with a grid and a white curve.
fn curve(ui: &mut egui::Ui, st: &mut DevelopPanel, s: &mut DevelopSettings) -> egui::Rect {
    let t = Lc::get(ui.ctx());
    let channels = [
        ("Master", t.text),
        ("Red", egui::Color32::from_rgb(220, 60, 60)),
        ("Green", egui::Color32::from_rgb(60, 190, 80)),
        ("Blue", egui::Color32::from_rgb(60, 120, 230)),
    ];
    w::padded(ui, 4, 4, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (i, (name, color)) in channels.into_iter().enumerate() {
                let on = st.curve_channel == i;
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 26.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tr(name)));
                let c = pos2(r.center().x, r.top() + 11.0);
                ui.painter().circle_stroke(c, 8.0, Stroke::new(1.5, color));
                if on {
                    ui.painter().circle_filled(c, 4.0, color);
                    ui.painter().line_segment([pos2(r.left() + 3.0, r.bottom() - 1.0), pos2(r.right() - 3.0, r.bottom() - 1.0)], Stroke::new(2.0, t.text));
                }
                if resp.on_hover_text(tr(name)).clicked() && !on {
                    st.curve_channel = i;
                    st.curve_state = Default::default();
                }
            }
        });
    });
    let c = &mut s.curve;
    let pts = match st.curve_channel {
        1 => &mut c.red,
        2 => &mut c.green,
        3 => &mut c.blue,
        _ => &mut c.master,
    };
    let mut points: Vec<[f32; 2]> = pts.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| [(p.x * 255.0) as f32, (p.y * 255.0) as f32]).collect();
    if points.len() < 2 {
        points = vec![[0.0, 0.0], [255.0, 255.0]];
    }
    let (rect, changed) = w::padded(ui, 4, 8, |ui| curve_editor(ui, &mut points, &mut st.curve_state));
    if changed {
        *pts = points.iter().map(|[x, y]| craft_geom::Point::new(f64::from(*x) / 255.0, f64::from(*y) / 255.0)).collect();
    }
    rect
}

/// The point-curve editor in LightCraft's look, with PhotoCraft's curve gestures (click to add,
/// drag, right-click to delete).
fn curve_editor(ui: &mut egui::Ui, points: &mut Vec<[f32; 2]>, state: &mut crate::state::PointCurveState) -> (Rect, bool) {
    let t = Lc::get(ui.ctx());
    let side = ui.available_width().min(270.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let painter = ui.painter_at(rect.expand(6.0));
    painter.rect_filled(rect, 2.0, t.canvas);
    for i in 1..4 {
        let f = i as f32 / 4.0;
        let grid = Stroke::new(1.0, t.chrome);
        painter.line_segment([pos2(rect.left() + f * side, rect.top()), pos2(rect.left() + f * side, rect.bottom())], grid);
        painter.line_segment([pos2(rect.left(), rect.top() + f * side), pos2(rect.right(), rect.top() + f * side)], grid);
    }
    painter.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.0, t.track.gamma_multiply(0.6)));
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    let changed = crate::point_curve::interact(ui, &resp, rect, points, state).changed;
    let lut = photocraft_algo::camera_raw::curve_lut(points, 256);
    let line: Vec<egui::Pos2> = lut.iter().enumerate().map(|(i, v)| to_screen(i as f32, v * 255.0)).collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, t.text)));
    for (i, c) in points.iter().enumerate() {
        let at = to_screen(c[0], c[1]);
        painter.circle_filled(at, 4.0, if state.selected == Some(i) { t.accent } else { t.text });
    }
    (rect, changed)
}

/// The controls the colour wheels set (hue and saturation of the three tonal ranges).
const WHEEL_IDS: [&str; 6] =
    ["grading.shadows.hue", "grading.shadows.sat", "grading.midtones.hue", "grading.midtones.sat", "grading.highlights.hue", "grading.highlights.sat"];

/// An HSV colour (hue in degrees, saturation and value 0..1) as sRGB.
fn hsv(h: f32, s: f32, v: f32) -> egui::Color32 {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let u = |x: f32| ((x + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgb(u(r), u(g), u(b))
}

/// A grading wheel's picture (LightCraft's): a hue disc fully saturated at the rim, grey at the
/// centre, built from rings so saturation ramps evenly.
fn wheel_mesh(c: egui::Pos2, rad: f32) -> egui::epaint::Mesh {
    const SEGMENTS: u32 = 72;
    const RINGS: u32 = 12;
    let colour = |hue: f32, t: f32| hsv(hue, t, 0.6 + 0.4 * t);
    let mut mesh = egui::epaint::Mesh::default();
    let vertex = |pos, color| egui::epaint::Vertex { pos, uv: egui::Pos2::ZERO, color };
    mesh.vertices.push(vertex(c, colour(0.0, 0.0)));
    let row = SEGMENTS + 1;
    for k in 1..=RINGS {
        let t = k as f32 / RINGS as f32;
        for i in 0..=SEGMENTS {
            let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            mesh.vertices.push(vertex(c + vec2(a.cos(), -a.sin()) * rad * t, colour(a.to_degrees(), t)));
        }
    }
    for i in 0..SEGMENTS {
        mesh.indices.extend([0, 1 + i, 2 + i]);
    }
    for k in 1..RINGS {
        let inner = 1 + (k - 1) * row;
        let outer = inner + row;
        for i in 0..SEGMENTS {
            mesh.indices.extend([inner + i, outer + i, outer + i + 1, inner + i, outer + i + 1, inner + i + 1]);
        }
    }
    mesh
}

/// The hue (degrees) and saturation (0..100) at offset `v` from a wheel's centre of radius `rad`.
fn wheel_pick(v: egui::Vec2, rad: f32) -> (f64, f64) {
    let v = if v.x.is_finite() && v.y.is_finite() { v } else { egui::Vec2::ZERO };
    let hue = ((-v.y).atan2(v.x).to_degrees() + 360.0) % 360.0;
    (f64::from(hue).round(), f64::from(v.length() / rad.max(1.0) * 100.0).min(100.0).round())
}

/// LightCraft's 3-way colour grading: the midtones wheel on top, shadows and highlights below. A
/// drag (or click) sets the range's hue and saturation; a double-click resets them.
fn grading_wheels(ui: &mut egui::Ui, st: &mut DevelopPanel, s: &mut DevelopSettings) {
    let t = Lc::get(ui.ctx());
    let w = ui.available_width();
    let rad = 40.0;
    let (area, _) = ui.allocate_exact_size(vec2(w, 276.0), Sense::hover());
    let wheels = [
        ("shadows", "Shadows", pos2(area.left() + w * 0.28, area.top() + 196.0)),
        ("midtones", "Midtones", pos2(area.center().x, area.top() + 70.0)),
        ("highlights", "Highlights", pos2(area.left() + w * 0.72, area.top() + 196.0)),
    ];
    for (key, label, c) in wheels {
        let g = &mut s.grading;
        let wheel = match key {
            "shadows" => &mut g.shadows,
            "midtones" => &mut g.midtones,
            _ => &mut g.highlights,
        };
        let r = Rect::from_center_size(c, vec2(rad * 2.0, rad * 2.0));
        let resp = ui.interact(r, ui.make_persistent_id(("lc-wheel", key)), Sense::click_and_drag());
        let name = tr(label);
        resp.widget_info(|| egui::WidgetInfo::slider(true, wheel.hue, name));
        ui.painter().add(wheel_mesh(c, rad));
        ui.painter().circle_stroke(c, rad, Stroke::new(1.0, egui::Color32::from_gray(40)));
        if resp.double_clicked() {
            (wheel.hue, wheel.sat) = (0.0, 0.0);
        } else if (resp.dragged() || resp.clicked())
            && let Some(q) = resp.interact_pointer_pos()
        {
            (wheel.hue, wheel.sat) = wheel_pick(q - c, rad);
        }
        let a = (wheel.hue as f32).to_radians();
        let dot = c + vec2(a.cos(), -a.sin()) * rad * (wheel.sat as f32 / 100.0);
        ui.painter().circle_filled(c, 2.0, egui::Color32::WHITE);
        ui.painter().circle_filled(dot, 5.0, egui::Color32::WHITE);
        ui.painter().circle_stroke(dot, 5.0, Stroke::new(1.0, egui::Color32::BLACK));
        ui.painter().text(pos2(c.x, c.y + rad + 12.0), egui::Align2::CENTER_CENTER, name, t.font(12.0), t.text_label);
        let shown = format!("{:.0}° {:.0}", wheel.hue, wheel.sat);
        ui.painter().text(pos2(c.x, c.y - rad - 10.0), egui::Align2::CENTER_CENTER, shown, t.font(11.0), t.text_dim);
    }
    // the wheels are these controls
    st.drawn.extend(WHEEL_IDS);
}

/// Upright's modes (Off … Guided) and Guided's guides.
fn upright(ui: &mut egui::Ui, s: &mut DevelopSettings, tools: &mut Tools<'_>) {
    w::sub_title(ui, tr("Upright"));
    const MODES: [(Upright, &str, &str); 6] = [
        (Upright::Off, "Off", "Disable Upright"),
        (Upright::Auto, "Auto", "Auto: apply balanced perspective corrections"),
        (Upright::Level, "Level", "Level: apply only level correction"),
        (Upright::Vertical, "Vertical", "Vertical: apply level and vertical perspective correction"),
        (Upright::Full, "Full", "Full: apply level, horizontal, and vertical perspective corrections"),
        (Upright::Guided, "Guided", "Guided: draw two or more guides to customize perspective corrections"),
    ];
    w::padded(ui, 2, 4, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (mode, name, tip) in MODES {
                let on = s.geometry.upright == mode;
                if w::text_button(ui, tr(name), on).on_hover_text(tr(tip)).clicked() {
                    if !on {
                        let g = &mut s.geometry;
                        g.upright = mode;
                        // guides belong to Guided, and the solved transform to the mode it was solved for
                        g.guides.clear();
                        g.upright_transform = None;
                    }
                    tools.guides.active = mode == Upright::Guided;
                    if tools.guides.active {
                        *tools.sampler = false;
                    }
                }
            }
        });
        if s.geometry.upright == Upright::Guided {
            let t = Lc::get(ui.ctx());
            let n = s.geometry.guides.len();
            let text = if n < 2 {
                tl!("Draw two or more guides on the image along lines that should be vertical or horizontal (Shift+T).")
            } else {
                tl!("Drag a guide's end to move it; select a guide and press Delete to remove it. Up to four guides.")
            };
            ui.label(egui::RichText::new(text).size(11.5).color(t.text_dim));
            if n > 0 && w::text_button(ui, tl!("Clear Guides"), false).clicked() {
                s.geometry.guides.clear();
                tools.guides.selected = None;
            }
        }
    });
}

/// A LightCraft check box row.
fn check(ui: &mut egui::Ui, v: &mut bool, label: &str) {
    w::padded(ui, 2, 2, |ui| {
        ui.checkbox(v, label);
    });
}

/// The panel: every registry section in order, each with its controls. Edits `e.settings` in place.
pub(crate) fn panel(ui: &mut egui::Ui, e: &mut DevelopEdit, source: Option<&DevelopSource>, st: &mut DevelopPanel, mut tools: Tools<'_>) -> PanelOut {
    lc::style(ui);
    ui.spacing_mut().item_spacing.y = 0.0;
    st.drawn.clear();
    let basis = e.basis();
    let unedited = e.unedited().clone();
    let s = &mut e.settings;
    let mut out = PanelOut::default();
    header(ui, &unedited, s, source, &mut out);
    w::divider(ui);
    profile(ui, st, s, basis, tools.alt);
    w::divider(ui);
    let all = sections();
    for sec in all.iter().copied() {
        let eye = eye_id(sec);
        match sec {
            // drawn by the Profile row, in Light (Curve), Color (the mixers, Color Grading) and Effects
            Section::Profile | Section::Curve | Section::Mixer | Section::BwMix | Section::Grading | Section::Vignette | Section::Grain => {}
            Section::Light => section(ui, st, s, "light", "Light", eye, |ui, st, s| {
                rows(ui, st, s, Section::Light, basis, tools.alt, |_| true);
                ui.add_space(6.0);
                if all.contains(&Section::Curve) {
                    flyout(ui, st, "curve", tr("Curve"), Icon::Curve, |ui, st| {
                        out.curve_rect = Some(curve(ui, st, s));
                        rows(ui, st, s, Section::Curve, basis, tools.alt, |_| true);
                    });
                }
            }),
            Section::Color => section(ui, st, s, "color", "Color", eye, |ui, st, s| {
                white_balance(ui, s, source, basis);
                rows(ui, st, s, Section::Color, basis, tools.alt, |_| true);
                ui.add_space(6.0);
                let bw = s.treatment == Treatment::Bw;
                flyout(ui, st, "mixer", if bw { tr("B&W Mixer") } else { tr("Color Mixer") }, Icon::Radial, |ui, st| {
                    if bw {
                        rows(ui, st, s, Section::BwMix, basis, tools.alt, |_| true);
                        return;
                    }
                    let tabs = [tr("Hue"), tr("Saturation"), tr("Luminance")];
                    if let Some(i) = w::padded(ui, 4, 4, |ui| w::segmented(ui, &tabs, Some(st.mixer_tab))) {
                        st.mixer_tab = i;
                    }
                    let tab = st.mixer_tab;
                    rows(ui, st, s, Section::Mixer, basis, tools.alt, |c| on_tab(c.track, tab));
                });
                if all.contains(&Section::Grading) {
                    flyout(ui, st, "grading", tr("Color Grading"), Icon::GradeAll, |ui, st| {
                        grading_wheels(ui, st, s);
                        rows(ui, st, s, Section::Grading, basis, tools.alt, |c| !WHEEL_IDS.contains(&c.id));
                    });
                }
            }),
            Section::Effects => section(ui, st, s, "effects", "Effects", eye, |ui, st, s| {
                rows(ui, st, s, Section::Effects, basis, tools.alt, |_| true);
                w::sub_title(ui, tr(Section::Vignette.label()));
                let styles = [VignetteStyle::HighlightPriority, VignetteStyle::ColorPriority, VignetteStyle::PaintOverlay];
                let labels = [tr("Highlight"), tr("Color"), tr("Paint")];
                let active = styles.iter().position(|v| *v == s.vignette.style);
                if let Some(i) = w::padded(ui, 2, 4, |ui| w::segmented(ui, &labels, active))
                    && let Some(v) = styles.get(i)
                {
                    s.vignette.style = *v;
                }
                rows(ui, st, s, Section::Vignette, basis, tools.alt, |_| true);
                w::sub_title(ui, tr(Section::Grain.label()));
                rows(ui, st, s, Section::Grain, basis, tools.alt, |_| true);
            }),
            Section::Optics => section(ui, st, s, "optics", "Optics", eye, |ui, st, s| {
                check(ui, &mut s.optics.remove_ca, tl!("Remove Chromatic Aberration"));
                check(ui, &mut s.optics.lens_profile, tl!("Enable Lens Corrections"));
                rows(ui, st, s, Section::Optics, basis, tools.alt, |_| true);
            }),
            Section::Geometry => {
                // Shift+T opens the section (the Guided Upright tool)
                if std::mem::take(&mut tools.guides.reveal) && !st.is_open("geometry") {
                    st.toggle("geometry");
                }
                section(ui, st, s, "geometry", "Geometry", eye, |ui, st, s| {
                    upright(ui, s, &mut tools);
                    w::sub_title(ui, tl!("Manual Transformations"));
                    rows(ui, st, s, Section::Geometry, basis, tools.alt, |_| true);
                });
            }
            Section::LensBlur => section(ui, st, s, "lensBlur", "Lens Blur", eye, |ui, st, s| {
                check(ui, &mut s.lens_blur.enabled, tl!("Apply"));
                rows(ui, st, s, Section::LensBlur, basis, tools.alt, |_| true);
            }),
            // any other section, including one added to the registry later: its controls in order
            other => {
                let id = format!("{other:?}");
                section(ui, st, s, &id, other.label(), eye, |ui, st, s| rows(ui, st, s, other, basis, tools.alt, |_| true));
            }
        }
    }
    ui.add_space(40.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::camera_raw::CameraRaw;

    /// One frame of the panel on `e`; the control ids it drew.
    fn draw(ctx: &egui::Context, e: &mut DevelopEdit, st: &mut DevelopPanel, frame: f64) -> Vec<&'static str> {
        let mut guides = crate::camera_raw_guides_ui::GuideTool::default();
        let (mut sampler, mut alt) = (false, None);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(330.0, 30000.0))),
            time: Some(frame),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            let tools = Tools { guides: &mut guides, sampler: &mut sampler, alt: &mut alt };
            panel(ui, e, None, st, tools);
        });
        out.textures_delta.clear();
        st.drawn.clone()
    }

    /// Every control the shared registry has is a slider in the panel, in a section of its own
    /// kind, but for the ones it leaves out on purpose ([`NOT_SHOWN`]): a control or section
    /// LightCraft adds shows here with no change to this file. Settings that hide sliders (HDR
    /// off, lens corrections off, the default profile, a mixer tab, B&W) are each tried.
    #[test]
    fn every_registry_control_is_in_the_panel() {
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        ctx.all_styles_mut(|s| s.animation_time = 0.0);
        let basis = WbBasis::Kelvin { as_shot_temp: 5000.0, as_shot_tint: 2.0 };
        let mut drawn = std::collections::BTreeSet::new();
        let mut frame = 0.0;
        for bw in [false, true] {
            for tab in 0..3 {
                let mut e = DevelopEdit::new(&CameraRaw::default(), basis, &DevelopSettings::for_raw(5000.0, 2.0));
                e.settings.hdr.enabled = true;
                e.settings.optics.lens_profile = true;
                e.settings.profile.id = "lc.vivid".into();
                e.settings.treatment = if bw { Treatment::Bw } else { Treatment::Color };
                let mut st = DevelopPanel { mixer_tab: tab, expand_all: true, ..Default::default() };
                for _ in 0..3 {
                    frame += 1.0;
                    drawn.extend(draw(&ctx, &mut e, &mut st, frame));
                }
            }
        }
        let expected: std::collections::BTreeSet<&str> = controls::CONTROLS.iter().map(|c| c.id).filter(|id| !NOT_SHOWN.contains(id)).collect();
        let missing: Vec<_> = expected.difference(&drawn).collect();
        assert!(missing.is_empty(), "registry controls not in the panel: {missing:?}");
        assert!(drawn.iter().all(|id| controls::find(id).is_some()), "{drawn:?}");
        // and each section lists its own controls, in registry order
        let mut e = DevelopEdit::new(&CameraRaw::default(), basis, &DevelopSettings::for_raw(5000.0, 2.0));
        let mut st = DevelopPanel { expand_all: true, ..Default::default() };
        draw(&ctx, &mut e, &mut st, frame + 1.0);
        let light: Vec<&str> =
            draw(&ctx, &mut e, &mut st, frame + 2.0).into_iter().filter(|id| controls::find(id).is_some_and(|c| c.section == Section::Light)).collect();
        let registry: Vec<&str> = controls::in_section(Section::Light).map(|c| c.id).filter(|id| shown(id, &e.settings)).collect();
        assert_eq!(light, registry);
        // folded, a section draws no sliders; Light is open by default
        let mut st = DevelopPanel::default();
        let shown_now = draw(&ctx, &mut e, &mut st, frame + 3.0);
        assert!(shown_now.contains(&"light.exposure") && !shown_now.contains(&"color.vibrance"), "{shown_now:?}");
    }

    /// A slider writes its control by id, on the spec's steps and in its range, and a white
    /// balance moved by hand is Custom on the source's scale.
    #[test]
    fn rows_write_by_id_and_white_balance_becomes_custom() {
        let spec = controls::find("light.exposure").unwrap();
        assert_eq!(snap(spec, 0.123), 0.12);
        assert_eq!(snap(spec, 9.0), 5.0);
        let temp = controls::find("wb.temp").unwrap();
        assert_eq!(snap(temp, 5523.0), 5500.0);
        // the relative scale of a raw without Kelvin: 0 is as shot, and round-trips
        let (shown_spec, v, store) = view(temp, craft_develop_model::crs::rel_to_kelvin(30.0), WbBasis::Relative);
        assert_eq!(shown_spec.min, -100.0);
        assert!((v - 30.0).abs() < 1e-6, "{v}");
        assert!((store(v) - craft_develop_model::crs::rel_to_kelvin(30.0)).abs() < 1e-6);
        assert_eq!(view(temp, 5000.0, WbBasis::Kelvin { as_shot_temp: 5000.0, as_shot_tint: 0.0 }).0.id, "wb.temp");
        assert!(w::track_stops(Track::Temp).is_some() && w::track_stops(Track::Plain).is_none());
        assert!(sections().contains(&Section::Light) && !sections().contains(&Section::PointColor), "indexed-only sections have no rows");
        assert_eq!(eye_id(Section::Light), Some("light"));
        // a wheel's right edge is hue 0 at full saturation, its top 90°, its centre neutral
        assert_eq!(wheel_pick(vec2(40.0, 0.0), 40.0), (0.0, 100.0));
        assert_eq!(wheel_pick(vec2(0.0, -20.0), 40.0), (90.0, 50.0));
        assert_eq!(wheel_pick(vec2(f32::NAN, 1.0), 40.0), (0.0, 0.0));
        assert!(WHEEL_IDS.iter().all(|id| controls::find(id).is_some()), "the wheels' controls are the registry's");
    }
}
