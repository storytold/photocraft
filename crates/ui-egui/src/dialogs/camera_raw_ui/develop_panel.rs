//! The Edit panel for a raw the shared develop engine (craft-develop, LightCraft's) develops: the
//! dialog's settings are its [`DevelopSettings`], edited here directly.
//!
//! The sliders come from the shared control registry ([`controls::CONTROLS`]): each section lists
//! every control of its [`Section`] in registry order, with the spec's label, range, default
//! (double-click resets to it), step, decimals and track. A control LightCraft adds to a section,
//! or a section it adds, shows here on a dependency bump without a change to this file. What the
//! registry does not describe is drawn by hand on the same settings: the profile menu, Auto, B&W,
//! the white-balance mode menu, the point curve, the Color Mixer's Hue / Saturation / Luminance
//! tabs, vignette style, the Optics and Lens Blur switches and Upright.
//!
//! Not drawn: Point Color and Red Eye (indexed controls, which need a sampling tool on the
//! preview), the HDR switch (the sliders of HDR editing show only once settings have it on: an
//! opened raw is a 16-bit document) and AI Denoise's amount (PhotoCraft has no denoise model).

use craft_develop_model::controls::{self, ControlSpec, Section, Track};
use craft_develop_model::{DevelopSettings, MIXER_HUES, Treatment, Upright, VignetteStyle, WbMode, WbScale, WhiteBalance};
use egui::{Color32, FontId};
use photocraft_io::develop_bridge::{DevelopEdit, WbBasis};
use photocraft_io::raw_develop::DevelopSource;

use crate::camera_raw_scope_ui::ToneZone;
use crate::widgets;

/// Controls the panel leaves out whatever the settings (module docs): AI Denoise's amount.
pub(crate) const NOT_SHOWN: &[&str] = &["enhance.denoise"];

/// The panel's own state: tabs, the curve channel, and what it drew last.
#[derive(Default)]
pub(crate) struct DevelopPanel {
    /// Color Mixer tab: 0 Hue, 1 Saturation, 2 Luminance.
    pub(crate) mixer_tab: usize,
    /// Point curve channel: 0 master, 1 red, 2 green, 3 blue.
    pub(crate) curve_channel: usize,
    pub(crate) curve_state: crate::state::PointCurveState,
    /// Every section open (the control channel's `expandAll`, and tests).
    pub(crate) expand_all: bool,
    /// The control ids drawn in the last frame, in order.
    pub(crate) drawn: Vec<&'static str>,
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

fn tr(s: &str) -> &str {
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

/// `v` on `spec`'s steps, in range.
fn snap(spec: &ControlSpec, v: f64) -> f64 {
    let v = if spec.step > 0.0 && spec.step.is_finite() { (v / spec.step).round() * spec.step } else { v };
    spec.clamp(v)
}

fn hex(s: &str) -> Option<Color32> {
    let h = s.strip_prefix('#')?;
    let n = u32::from_str_radix(h.get(..6)?, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn band_hue(band: u8) -> f32 {
    MIXER_HUES.get(usize::from(band) % MIXER_HUES.len()).copied().unwrap_or(0.0) as f32
}

/// The track colours of `track` (none: PhotoCraft's plain track).
pub(crate) fn gradient(track: Track) -> Option<Vec<Color32>> {
    let n = MIXER_HUES.len() as u8;
    Some(match track {
        Track::Plain | Track::Centered => return None,
        Track::Temp => super::controls::temp_stops(),
        Track::Tint => super::controls::tint_stops(),
        Track::Gradient { from, to } => vec![hex(from)?, hex(to)?],
        Track::Hue { band } => {
            let band = band % n;
            [(band + n - 1) % n, band, (band + 1) % n].map(|b| widgets::hue_color(band_hue(b))).to_vec()
        }
        Track::Sat { band } => vec![Color32::from_gray(128), widgets::hue_color(band_hue(band))],
        Track::Lum { band } => {
            let c = widgets::hue_color(band_hue(band));
            let scale = |k: f32| Color32::from_rgb((f32::from(c.r()) * k) as u8, (f32::from(c.g()) * k) as u8, (f32::from(c.b()) * k) as u8);
            vec![scale(0.25), c, Color32::from_rgb(c.r().saturating_add(90), c.g().saturating_add(90), c.b().saturating_add(90))]
        }
        Track::Rainbow => widgets::hue_stops(),
    })
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

/// One registry control as a slider row.
fn row(ui: &mut egui::Ui, drawn: &mut Vec<&'static str>, s: &mut DevelopSettings, spec: &'static ControlSpec, basis: WbBasis, alt: &mut Option<ToneZone>) {
    let Some(stored) = controls::get(s, spec.id) else { return };
    let (shown_spec, value, store) = view(spec, stored, basis);
    let mut x = value as f32;
    let grad = gradient(shown_spec.track);
    let on = enabled(spec.id, s);
    let r = ui
        .push_id(spec.id, |ui| {
            ui.add_enabled_ui(on, |ui| {
                widgets::slider_row_mapped(
                    ui,
                    tr(shown_spec.label),
                    &mut x,
                    shown_spec.min as f32..=shown_spec.max as f32,
                    u32::from(shown_spec.decimals),
                    grad.as_deref(),
                    shown_spec.default as f32,
                    widgets::TrackMap {
                        position: &|v| shown_spec.position(f64::from(v)) as f32,
                        value_at: &|p| snap(shown_spec, shown_spec.value_at(f64::from(p))) as f32,
                    },
                )
            })
            .inner
        })
        .inner;
    drawn.push(spec.id);
    if r.dragged()
        && ui.input(|i| i.modifiers.alt)
        && let Some(zone) = tone_zone(spec.id)
    {
        *alt = Some(zone);
    }
    if r.changed() && x.is_finite() {
        let v = store(snap(shown_spec, f64::from(x)));
        controls::set(s, spec.id, v);
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

fn collapse(ui: &mut egui::Ui, expand: bool, title: &str, open: bool, force: Option<bool>, body: impl FnOnce(&mut egui::Ui)) -> egui::CollapsingResponse<()> {
    egui::CollapsingHeader::new(egui::RichText::new(tr(title)).font(FontId::proportional(13.0)))
        .id_salt(("camera-raw-develop-section", title))
        .default_open(open)
        .open(if expand { Some(true) } else { force })
        .show(ui, body)
}

/// Auto, B&W and the profile, above the sections.
fn header(
    ui: &mut egui::Ui,
    st: &mut DevelopPanel,
    s: &mut DevelopSettings,
    source: Option<&DevelopSource>,
    basis: WbBasis,
    tools: &mut Tools<'_>,
    out: &mut PanelOut,
) {
    ui.horizontal(|ui| {
        if let Some(source) = source
            && ui.button(tl!("Auto")).on_hover_text(tl!("Set the tone sliders from the image")).clicked()
        {
            source.auto_tone(s).apply(s);
            out.auto = true;
        }
        let bw = s.treatment == Treatment::Bw;
        let r = widgets::pill_tab(ui, tr("B&W"), bw).on_hover_text(tl!("Black & White"));
        if r.clicked() {
            s.treatment = if bw { Treatment::Color } else { Treatment::Bw };
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("Profile"));
        let mut id = s.profile.id.clone();
        let mut options: Vec<(String, &str)> = photocraft_io::raw_develop::PROFILES.iter().map(|p| (p.id.to_string(), tr(p.name))).collect();
        if !options.iter().any(|(o, _)| *o == id) {
            // a profile this build doesn't know (an imported one in LightCraft): kept, and shown by id
            options.push((id.clone(), s.profile.id.as_str()));
        }
        if widgets::dropdown(ui, "camera-raw-profile", tl!("Profile"), &mut id, &options, 180.0) {
            s.profile.id = id;
        }
    });
    if s.profile.id != "lc.color" {
        rows(ui, st, s, Section::Profile, basis, tools.alt, |_| true);
    }
}

/// The white-balance mode menu: a preset or Auto sets its white, as LightCraft's does.
fn white_balance(ui: &mut egui::Ui, s: &mut DevelopSettings, source: Option<&DevelopSource>, basis: WbBasis) {
    ui.horizontal(|ui| {
        ui.label(tr("White Balance"));
        let mut mode = s.wb.mode;
        let options: Vec<(WbMode, &str)> = WbMode::ALL.iter().map(|m| (*m, tr(m.label()))).collect();
        if widgets::dropdown(ui, "camera-raw-wb-mode", tl!("White Balance"), &mut mode, &options, 150.0) && mode != s.wb.mode {
            match (mode, source) {
                (WbMode::Custom, _) | (_, None) => s.wb.mode = mode,
                (_, Some(source)) => {
                    let (temp, tint) = source.white_balance_for(mode);
                    s.wb = WhiteBalance { mode, temp: temp.clamp(2000.0, 50000.0), tint: tint.clamp(-150.0, 150.0), scale: scale(basis) };
                }
            }
        }
    });
}

/// The point curve of the chosen channel (0..1 in the model, 0..255 in the editor).
fn curve(ui: &mut egui::Ui, st: &mut DevelopPanel, s: &mut DevelopSettings) -> egui::Rect {
    ui.horizontal_wrapped(|ui| {
        for (i, name) in ["Master", "Red", "Green", "Blue"].into_iter().enumerate() {
            let on = st.curve_channel == i;
            let response = widgets::pill_tab(ui, tr(name), on);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, tr(name)));
            if response.clicked() && !on {
                st.curve_channel = i;
                st.curve_state = Default::default();
            }
        }
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
    let (rect, changed) = super::controls::curve_points_editor(ui, &mut points, &mut st.curve_state);
    if changed {
        *pts = points.iter().map(|[x, y]| craft_geom::Point::new(f64::from(*x) / 255.0, f64::from(*y) / 255.0)).collect();
    }
    rect
}

/// Upright's modes (Off … Guided) and Guided's guides, as the classic panel has them.
fn upright(ui: &mut egui::Ui, s: &mut DevelopSettings, tools: &mut Tools<'_>) {
    widgets::section_label(ui, tl!("Upright"));
    const MODES: [(Upright, &str, &str); 6] = [
        (Upright::Off, "Off", "Disable Upright"),
        (Upright::Auto, "Auto", "Auto: apply balanced perspective corrections"),
        (Upright::Level, "Level", "Level: apply only level correction"),
        (Upright::Vertical, "Vertical", "Vertical: apply level and vertical perspective correction"),
        (Upright::Full, "Full", "Full: apply level, horizontal, and vertical perspective corrections"),
        (Upright::Guided, "Guided", "Guided: draw two or more guides to customize perspective corrections"),
    ];
    ui.horizontal_wrapped(|ui| {
        for (mode, name, tip) in MODES {
            let on = s.geometry.upright == mode;
            let response = widgets::pill_tab(ui, tr(name), on).on_hover_text(tr(tip));
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, tr(name)));
            if response.clicked() {
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
        let n = s.geometry.guides.len();
        let text = if n < 2 {
            tl!("Draw two or more guides on the image along lines that should be vertical or horizontal (Shift+T).")
        } else {
            tl!("Drag a guide's end to move it; select a guide and press Delete to remove it. Up to four guides.")
        };
        ui.label(egui::RichText::new(text).small().weak());
        if n > 0 && ui.button(tl!("Clear Guides")).clicked() {
            s.geometry.guides.clear();
            tools.guides.selected = None;
        }
    }
}

/// The panel: every registry section in order, each with its controls. Edits `e.settings` in place.
pub(crate) fn panel(ui: &mut egui::Ui, e: &mut DevelopEdit, source: Option<&DevelopSource>, st: &mut DevelopPanel, mut tools: Tools<'_>) -> PanelOut {
    st.drawn.clear();
    let basis = e.basis();
    let s = &mut e.settings;
    let mut out = PanelOut::default();
    header(ui, st, s, source, basis, &mut tools, &mut out);
    let expand = st.expand_all;
    for section in sections() {
        match section {
            // drawn by the header, the Color Mixer and Effects
            Section::Profile | Section::BwMix | Section::Vignette | Section::Grain => {}
            Section::Light => {
                collapse(ui, expand, "Light", true, None, |ui| rows(ui, st, s, Section::Light, basis, tools.alt, |_| true));
            }
            Section::Curve => {
                collapse(ui, expand, "Curve", false, None, |ui| {
                    out.curve_rect = Some(curve(ui, st, s));
                    rows(ui, st, s, Section::Curve, basis, tools.alt, |_| true);
                });
            }
            Section::Color => {
                collapse(ui, expand, "Color", true, None, |ui| {
                    white_balance(ui, s, source, basis);
                    rows(ui, st, s, Section::Color, basis, tools.alt, |_| true);
                });
            }
            Section::Mixer => {
                let bw = s.treatment == Treatment::Bw;
                collapse(ui, expand, if bw { "B&W Mixer" } else { "Color Mixer" }, false, None, |ui| {
                    if bw {
                        rows(ui, st, s, Section::BwMix, basis, tools.alt, |_| true);
                        return;
                    }
                    ui.horizontal_wrapped(|ui| {
                        for (i, name) in ["Hue", "Saturation", "Luminance"].into_iter().enumerate() {
                            let on = st.mixer_tab == i;
                            let response = widgets::pill_tab(ui, tr(name), on);
                            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, tr(name)));
                            if response.clicked() {
                                st.mixer_tab = i;
                            }
                        }
                    });
                    let tab = st.mixer_tab;
                    rows(ui, st, s, Section::Mixer, basis, tools.alt, |c| on_tab(c.track, tab));
                });
            }
            Section::Effects => {
                collapse(ui, expand, "Effects", false, None, |ui| {
                    rows(ui, st, s, Section::Effects, basis, tools.alt, |_| true);
                    widgets::section_label(ui, tr(Section::Vignette.label()));
                    let mut style = s.vignette.style;
                    let styles = [
                        (VignetteStyle::HighlightPriority, tl!("Highlight Priority")),
                        (VignetteStyle::ColorPriority, tl!("Color Priority")),
                        (VignetteStyle::PaintOverlay, tl!("Paint Overlay")),
                    ];
                    if widgets::dropdown(ui, "camera-raw-develop-vignette-style", tl!("Vignetting"), &mut style, &styles, 200.0) {
                        s.vignette.style = style;
                    }
                    rows(ui, st, s, Section::Vignette, basis, tools.alt, |_| true);
                    widgets::section_label(ui, tr(Section::Grain.label()));
                    rows(ui, st, s, Section::Grain, basis, tools.alt, |_| true);
                });
            }
            Section::Optics => {
                collapse(ui, expand, "Optics", false, None, |ui| {
                    widgets::checkbox(ui, &mut s.optics.remove_ca, tl!("Remove Chromatic Aberration"));
                    widgets::checkbox(ui, &mut s.optics.lens_profile, tl!("Enable Lens Corrections"));
                    rows(ui, st, s, Section::Optics, basis, tools.alt, |_| true);
                });
            }
            Section::Geometry => {
                // Shift+T opens the section (the Guided Upright tool)
                let reveal = std::mem::take(&mut tools.guides.reveal);
                let r = collapse(ui, expand, "Geometry", false, reveal.then_some(true), |ui| {
                    upright(ui, s, &mut tools);
                    widgets::section_label(ui, tl!("Manual Transformations"));
                    rows(ui, st, s, Section::Geometry, basis, tools.alt, |_| true);
                });
                if reveal {
                    r.header_response.scroll_to_me(Some(egui::Align::TOP));
                }
            }
            Section::LensBlur => {
                collapse(ui, expand, "Lens Blur", false, None, |ui| {
                    widgets::checkbox(ui, &mut s.lens_blur.enabled, tl!("Apply"));
                    rows(ui, st, s, Section::LensBlur, basis, tools.alt, |_| true);
                });
            }
            // any other section, including one added to the registry later: its controls in order
            other => {
                collapse(ui, expand, other.label(), false, None, |ui| rows(ui, st, s, other, basis, tools.alt, |_| true));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::camera_raw::CameraRaw;

    /// One frame of the panel on `e`, every section open; the control ids it drew.
    fn draw(ctx: &egui::Context, e: &mut DevelopEdit, st: &mut DevelopPanel, frame: f64) -> Vec<&'static str> {
        let mut guides = crate::camera_raw_guides_ui::GuideTool::default();
        let (mut sampler, mut alt) = (false, None);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(330.0, 20000.0))),
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
        assert!(gradient(Track::Temp).is_some() && gradient(Track::Plain).is_none());
        assert_eq!(gradient(Track::Gradient { from: "#ff0000", to: "#0000ff" }), Some(vec![Color32::from_rgb(255, 0, 0), Color32::from_rgb(0, 0, 255)]));
        assert_eq!(gradient(Track::Gradient { from: "bad", to: "#0000ff" }), None, "a bad colour is a plain track, not a crash");
        assert!(sections().contains(&Section::Light) && !sections().contains(&Section::PointColor), "indexed-only sections have no rows");
    }
}
