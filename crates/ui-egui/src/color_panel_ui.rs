//! Photoshop's Color panel (#294). Its panel menu picks how the foreground (or background)
//! colour is shown and edited:
//!
//! - **Hue Cube** (the default): a saturation × brightness field beside a vertical hue strip.
//! - **Brightness Cube**: a hue × saturation field beside a vertical brightness strip.
//! - **Color Wheel**: a hue ring around a saturation × brightness square.
//! - **Grayscale, RGB, HSB, CMYK, Lab and Web Color Sliders**: one ramp per component, painted
//!   with the colours that component would give, plus a numeric field. An RGB spectrum bar below
//!   picks any colour, and a warning triangle flags a colour outside the CMYK gamut.
//!
//! Hue, HSB and Web values are arithmetic on the RGB values. Grayscale, CMYK and Lab go through
//! `photocraft-cms` ([`Spaces`]): the RGB side is the active RGB document's profile (else the
//! working RGB space), CMYK and Gray are the Color Settings working spaces, Lab is CIE L*a*b* D50,
//! with the Color Settings intent and black point compensation. Values typed in a mode stay as
//! typed while the colour is unchanged (CMYK has more than one way to make a colour, and grey has
//! no hue), instead of being recomputed from RGB every frame.
//!
//! Every mode shows the editable hex and R, G, B readout below its controls when the group has
//! room for it. The foreground and background chips pick the colour the panel edits; a
//! double-click opens the Color Picker on it.
//!
//! The mode and the edited chip are UI state (`UiState::color_panel`, drivable with `ui.set`).
//! The mode is remembered across launches in the preferences (`dialogs["panel.color"]`).

use std::sync::Arc;

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_cms::{Builtin, Intent, Profile, Transform, TransformOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::{self, Tokens};
use crate::widgets;

/// What the Color panel shows (its panel menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorPanelMode {
    #[default]
    HueCube,
    BrightnessCube,
    ColorWheel,
    Grayscale,
    Rgb,
    Hsb,
    Cmyk,
    Lab,
    Web,
}

impl ColorPanelMode {
    /// Panel menu order.
    pub const ALL: [ColorPanelMode; 9] = [
        ColorPanelMode::HueCube,
        ColorPanelMode::BrightnessCube,
        ColorPanelMode::ColorWheel,
        ColorPanelMode::Grayscale,
        ColorPanelMode::Rgb,
        ColorPanelMode::Hsb,
        ColorPanelMode::Cmyk,
        ColorPanelMode::Lab,
        ColorPanelMode::Web,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ColorPanelMode::HueCube => "Hue Cube",
            ColorPanelMode::BrightnessCube => "Brightness Cube",
            ColorPanelMode::ColorWheel => "Color Wheel",
            ColorPanelMode::Grayscale => "Grayscale Sliders",
            ColorPanelMode::Rgb => "RGB Sliders",
            ColorPanelMode::Hsb => "HSB Sliders",
            ColorPanelMode::Cmyk => "CMYK Sliders",
            ColorPanelMode::Lab => "Lab Sliders",
            ColorPanelMode::Web => "Web Color Sliders",
        }
    }

    /// The serialised name (`ui.set {colorPanel: {mode}}`, the preferences).
    pub fn key(self) -> &'static str {
        match self {
            ColorPanelMode::HueCube => "hueCube",
            ColorPanelMode::BrightnessCube => "brightnessCube",
            ColorPanelMode::ColorWheel => "colorWheel",
            ColorPanelMode::Grayscale => "grayscale",
            ColorPanelMode::Rgb => "rgb",
            ColorPanelMode::Hsb => "hsb",
            ColorPanelMode::Cmyk => "cmyk",
            ColorPanelMode::Lab => "lab",
            ColorPanelMode::Web => "web",
        }
    }

    pub fn from_key(key: &str) -> Option<ColorPanelMode> {
        ColorPanelMode::ALL.into_iter().find(|m| m.key() == key)
    }

    /// The cube and wheel modes edit HSB through a field; the others are slider sets.
    pub fn is_field(self) -> bool {
        matches!(self, ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::ColorWheel)
    }

    /// The components a mode edits (the field modes edit HSB).
    pub fn components(self) -> &'static [Component] {
        const K: [Component; 1] = [Component { label: "K", min: 0.0, max: 100.0, unit: "%" }];
        const RGB: [Component; 3] = [
            Component { label: "R", min: 0.0, max: 255.0, unit: "" },
            Component { label: "G", min: 0.0, max: 255.0, unit: "" },
            Component { label: "B", min: 0.0, max: 255.0, unit: "" },
        ];
        const HSB: [Component; 3] = [
            Component { label: "H", min: 0.0, max: 360.0, unit: "°" },
            Component { label: "S", min: 0.0, max: 100.0, unit: "%" },
            Component { label: "B", min: 0.0, max: 100.0, unit: "%" },
        ];
        const CMYK: [Component; 4] = [
            Component { label: "C", min: 0.0, max: 100.0, unit: "%" },
            Component { label: "M", min: 0.0, max: 100.0, unit: "%" },
            Component { label: "Y", min: 0.0, max: 100.0, unit: "%" },
            Component { label: "K", min: 0.0, max: 100.0, unit: "%" },
        ];
        const LAB: [Component; 3] = [
            Component { label: "L", min: 0.0, max: 100.0, unit: "" },
            Component { label: "a", min: -128.0, max: 127.0, unit: "" },
            Component { label: "b", min: -128.0, max: 127.0, unit: "" },
        ];
        match self {
            ColorPanelMode::Grayscale => &K,
            ColorPanelMode::Rgb | ColorPanelMode::Web => &RGB,
            ColorPanelMode::Cmyk => &CMYK,
            ColorPanelMode::Lab => &LAB,
            ColorPanelMode::Hsb | ColorPanelMode::HueCube | ColorPanelMode::BrightnessCube | ColorPanelMode::ColorWheel => &HSB,
        }
    }
}

/// One slider of a mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Component {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub unit: &'static str,
}

pub use crate::state::ColorPanelState;

// ------------------------------------------------------------------ colour spaces

/// The colour transforms the panel converts through.
pub struct Spaces {
    rgb_lab: Arc<Transform>,
    lab_rgb: Arc<Transform>,
    rgb_cmyk: Arc<Transform>,
    cmyk_rgb: Arc<Transform>,
    rgb_gray: Arc<Transform>,
    gray_rgb: Arc<Transform>,
}

impl Spaces {
    pub fn new(rgb: &Profile, cmyk: &Profile, gray: &Profile, opts: TransformOptions) -> Result<Spaces, photocraft_cms::CmsError> {
        let lab = Builtin::LabD50.profile();
        let t = |a: &Profile, b: &Profile| photocraft_cms::cached(a, b, opts);
        Ok(Spaces {
            rgb_lab: t(rgb, lab)?,
            lab_rgb: t(lab, rgb)?,
            rgb_cmyk: t(rgb, cmyk)?,
            cmyk_rgb: t(cmyk, rgb)?,
            rgb_gray: t(rgb, gray)?,
            gray_rgb: t(gray, rgb)?,
        })
    }

    /// sRGB, the built-in coated CMYK and sGray (the default working spaces).
    pub fn defaults() -> Result<Spaces, photocraft_cms::CmsError> {
        Spaces::new(Builtin::Srgb.profile(), Builtin::CoatedCmyk.profile(), Builtin::SGray.profile(), TransformOptions::default())
    }

    fn eval<const N: usize>(t: &Transform, input: &[f32]) -> [f32; N] {
        let mut out = [0.0f32; 16];
        if t.inputs() == input.len() && t.outputs() <= out.len() {
            t.eval(input, &mut out[..t.outputs()]);
        }
        let mut r = [0.0f32; N];
        for (d, s) in r.iter_mut().zip(out) {
            *d = if s.is_finite() { s } else { 0.0 };
        }
        r
    }

    /// CIE L*a*b* (L 0–100, a and b −128–127).
    pub fn rgb_to_lab(&self, rgb: [f32; 3]) -> [f32; 3] {
        let [l, a, b] = Self::eval::<3>(&self.rgb_lab, &rgb);
        [l * 100.0, a * 255.0 - 128.0, b * 255.0 - 128.0]
    }

    pub fn lab_to_rgb(&self, lab: [f32; 3]) -> [f32; 3] {
        let enc = [lab[0] / 100.0, (lab[1] + 128.0) / 255.0, (lab[2] + 128.0) / 255.0].map(|v| v.clamp(0.0, 1.0));
        clamp3(Self::eval::<3>(&self.lab_rgb, &enc))
    }

    /// Ink percentages 0–100.
    pub fn rgb_to_cmyk(&self, rgb: [f32; 3]) -> [f32; 4] {
        Self::eval::<4>(&self.rgb_cmyk, &rgb).map(|v| (v * 100.0).clamp(0.0, 100.0))
    }

    pub fn cmyk_to_rgb(&self, cmyk: [f32; 4]) -> [f32; 3] {
        clamp3(Self::eval::<3>(&self.cmyk_rgb, &cmyk.map(|v| (v / 100.0).clamp(0.0, 1.0))))
    }

    /// Grayscale ink (K %, 0 = white).
    pub fn rgb_to_gray_k(&self, rgb: [f32; 3]) -> f32 {
        let [g] = Self::eval::<1>(&self.rgb_gray, &rgb);
        ((1.0 - g) * 100.0).clamp(0.0, 100.0)
    }

    pub fn gray_k_to_rgb(&self, k: f32) -> [f32; 3] {
        clamp3(Self::eval::<3>(&self.gray_rgb, &[(1.0 - k / 100.0).clamp(0.0, 1.0)]))
    }

    /// The colour as printed in the working CMYK space and back (out-of-gamut check).
    pub fn printable(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.cmyk_to_rgb(self.rgb_to_cmyk(rgb))
    }

    /// The colour can't be printed in the working CMYK space: its print round trip moves it by
    /// more than View › Gamut Warning's ΔE76 threshold. Judged in Lab, not RGB, so the small
    /// shift of a colour the press prints well (black through rich black) isn't flagged.
    pub fn out_of_gamut(&self, rgb: [f32; 3]) -> bool {
        let [a, b] = [rgb, self.printable(rgb)].map(|c| self.rgb_to_lab(c));
        let de = a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>().sqrt();
        de > photocraft_cms::gamut::DEFAULT_THRESHOLD
    }
}

fn clamp3(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })
}

/// HSB (H 0–360, S and B 0–100) from RGB 0–1. Grey has hue 0.
pub fn rgb_to_hsb(c: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = clamp3(c);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    [h.rem_euclid(360.0), s * 100.0, max * 100.0]
}

pub fn hsb_to_rgb(hsb: [f32; 3]) -> [f32; 3] {
    let h = if hsb[0].is_finite() { hsb[0].rem_euclid(360.0) } else { 0.0 } / 60.0;
    let s = (hsb[1] / 100.0).clamp(0.0, 1.0);
    let v = (hsb[2] / 100.0).clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    clamp3([r + m, g + m, b + m])
}

/// The nearest web-safe level (0, 51, …, 255) of a 0–255 value.
pub fn web_safe(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    ((v.clamp(0.0, 255.0) / 51.0).round() * 51.0).clamp(0.0, 255.0)
}

/// A colour's components in `mode` (see [`ColorPanelMode::components`]).
pub fn to_components(mode: ColorPanelMode, rgb: [f32; 3], sp: &Spaces) -> Vec<f32> {
    let rgb = clamp3(rgb);
    match mode {
        ColorPanelMode::Grayscale => vec![sp.rgb_to_gray_k(rgb)],
        ColorPanelMode::Rgb | ColorPanelMode::Web => rgb.iter().map(|v| v * 255.0).collect(),
        ColorPanelMode::Cmyk => sp.rgb_to_cmyk(rgb).to_vec(),
        ColorPanelMode::Lab => sp.rgb_to_lab(rgb).to_vec(),
        _ => rgb_to_hsb(rgb).to_vec(),
    }
}

/// The RGB colour (0–1) that `mode` components describe. Missing components count as their minimum.
pub fn from_components(mode: ColorPanelMode, c: &[f32], sp: &Spaces) -> [f32; 3] {
    let comps = mode.components();
    let v = |i: usize| {
        let lim = comps.get(i).map_or((0.0, 0.0), |k| (k.min, k.max));
        c.get(i).copied().filter(|x| x.is_finite()).unwrap_or(lim.0).clamp(lim.0, lim.1)
    };
    match mode {
        ColorPanelMode::Grayscale => sp.gray_k_to_rgb(v(0)),
        ColorPanelMode::Rgb => [v(0) / 255.0, v(1) / 255.0, v(2) / 255.0],
        ColorPanelMode::Web => [web_safe(v(0)) / 255.0, web_safe(v(1)) / 255.0, web_safe(v(2)) / 255.0],
        ColorPanelMode::Cmyk => sp.cmyk_to_rgb([v(0), v(1), v(2), v(3)]),
        ColorPanelMode::Lab => sp.lab_to_rgb([v(0), v(1), v(2)]),
        _ => hsb_to_rgb([v(0), v(1), v(2)]),
    }
}

/// The transforms for the current document and Color Settings, built once per change of either.
fn spaces(app: &PhotocraftApp, ctx: &egui::Context) -> Option<Arc<Spaces>> {
    use photocraft_engine::color_cmds::document_profile;
    use photocraft_engine::doc::ColorMode;
    use std::hash::{Hash, Hasher};
    let settings = &app.session.color.settings;
    let doc = app.session.active().map(|st| st.doc.clone()).filter(|d| d.mode == ColorMode::Rgb);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&settings.working_rgb, &settings.working_cmyk, &settings.working_gray, &settings.intent, settings.bpc).hash(&mut h);
    doc.as_ref().and_then(|d| d.icc_profile.as_ref()).map(|b| Arc::as_ptr(b) as usize).hash(&mut h);
    let key = egui::Id::new(("color-panel-spaces", h.finish()));
    if let Some(sp) = ctx.data(|d| d.get_temp::<Arc<Spaces>>(key)) {
        return Some(sp);
    }
    let rgb = match &doc {
        Some(d) => document_profile(d),
        None => app.session.color.working(ColorMode::Rgb),
    };
    let cmyk = app.session.color.working(ColorMode::Cmyk);
    let gray = app.session.color.working(ColorMode::Grayscale);
    let opts = TransformOptions { intent: Intent::parse(&settings.intent).unwrap_or_default(), bpc: settings.bpc, precise_float: true };
    let sp = Arc::new(Spaces::new(&rgb, &cmyk, &gray, opts).or_else(|_| Spaces::defaults()).ok()?);
    ctx.data_mut(|d| d.insert_temp(key, sp.clone()));
    Some(sp)
}

// ------------------------------------------------------------------ state

/// Components as last typed or dragged, kept while the colour they made is unchanged. Each chip
/// has its own, so each keeps the hue last picked for it while it is grey.
#[derive(Clone, Debug, Default)]
struct Sticky {
    mode: Option<ColorPanelMode>,
    rgb: [f32; 3],
    comps: Vec<f32>,
}

fn sticky_id(background: bool) -> egui::Id {
    egui::Id::new(("color-panel-sticky", background))
}

fn current(app: &PhotocraftApp) -> [f32; 3] {
    let c = if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground };
    [c[0], c[1], c[2]]
}

/// Set the edited colour. A new foreground also recolours selected type, as in Photoshop.
fn set_current(app: &mut PhotocraftApp, rgb: [f32; 3]) {
    let [r, g, b] = clamp3(rgb);
    if app.ui.color_panel.background {
        app.session.tools.background = [r, g, b, 1.0];
    } else {
        app.session.tools.foreground = [r, g, b, 1.0];
        crate::type_tool::foreground_changed(app);
    }
}

/// The components shown for the current colour: the remembered ones while it hasn't changed.
fn components(app: &PhotocraftApp, ctx: &egui::Context, sp: &Spaces) -> Vec<f32> {
    let mode = app.ui.color_panel.mode;
    let rgb = current(app);
    let s = ctx.data(|d| d.get_temp::<Sticky>(sticky_id(app.ui.color_panel.background))).unwrap_or_default();
    let close = s.rgb.iter().zip(rgb).all(|(a, b)| (a - b).abs() < 1e-5);
    let same_kind = s.mode.is_some_and(|m| m.components() == mode.components());
    if same_kind && close && s.comps.len() == mode.components().len() {
        return s.comps;
    }
    let mut c = to_components(mode, rgb, sp);
    // Grey has no hue: keep the last one so the field doesn't jump to red.
    if mode.components() == ColorPanelMode::Hsb.components()
        && let (Some(h), true) = (s.comps.first().copied().filter(|_| same_kind), c.get(1).is_some_and(|s| *s < 0.5) || c.get(2).is_some_and(|b| *b < 0.5))
        && let Some(slot) = c.get_mut(0)
    {
        *slot = h;
    }
    c
}

/// Apply edited components: the colour they make becomes the current colour and they're
/// remembered as typed.
fn commit(app: &mut PhotocraftApp, ctx: &egui::Context, sp: &Spaces, comps: Vec<f32>) {
    let mode = app.ui.color_panel.mode;
    let rgb = from_components(mode, &comps, sp);
    set_current(app, rgb);
    remember(app, ctx, comps);
}

fn remember(app: &PhotocraftApp, ctx: &egui::Context, comps: Vec<f32>) {
    let sticky = Sticky { mode: Some(app.ui.color_panel.mode), rgb: current(app), comps };
    ctx.data_mut(|d| d.insert_temp(sticky_id(app.ui.color_panel.background), sticky));
}

/// Apply a colour picked as RGB (the spectrum, the readout): the mode's components are worked
/// out from it, keeping the hue of a grey.
fn pick(app: &mut PhotocraftApp, ctx: &egui::Context, sp: &Spaces, rgb: [f32; 3]) {
    set_current(app, rgb);
    let comps = components(app, ctx, sp);
    remember(app, ctx, comps);
}

/// Switch mode (panel menu) and remember it in the preferences.
pub fn set_mode(app: &mut PhotocraftApp, mode: ColorPanelMode) {
    app.ui.color_panel.mode = mode;
    save_mode(app);
}

const PREF_KEY: &str = "panel.color";

/// Remember the mode in the preferences once the pointer is up. Run every frame, so a mode set
/// by the control channel while the panel is hidden is remembered too. Cheap: a string compare.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !ctx.input(|i| i.pointer.any_down()) {
        save_mode(app);
    }
}

fn save_mode(app: &mut PhotocraftApp) {
    let key = app.ui.color_panel.mode.key();
    let saved = app.session.prefs().dialogs.get(PREF_KEY).and_then(|v| v.get("mode")).and_then(|v| v.as_str());
    // Nothing saved means the default: a first launch doesn't write the preferences.
    if saved.unwrap_or(ColorPanelMode::default().key()) != key {
        app.session.prefs.edit(|p| {
            p.dialogs.insert(PREF_KEY.into(), json!({"mode": key}));
        });
    }
}

/// Restore the remembered mode at launch (an unknown or missing one keeps the default).
pub fn restore(app: &mut PhotocraftApp) {
    if let Some(m) = app.session.prefs().dialogs.get(PREF_KEY).and_then(|v| v.get("mode")).and_then(|v| v.as_str()).and_then(ColorPanelMode::from_key) {
        app.ui.color_panel.mode = m;
    }
}

// ------------------------------------------------------------------ panel menu

/// The Color panel's menu items (the dock group's ≡ menu while the Color tab is showing).
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let modes = ColorPanelMode::ALL;
    for (i, m) in modes.into_iter().enumerate() {
        // Photoshop separates the field modes from the slider sets.
        if i == 3 {
            ui.separator();
        }
        let on = app.ui.color_panel.mode == m;
        if ui.add(egui::Button::new(tl!(m.label())).selected(on)).clicked() {
            set_mode(app, m);
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("Copy Color's Hex Code")).clicked() {
        ui.ctx().copy_text(hex(current(app)));
        ui.close();
    }
}

fn hex(c: [f32; 3]) -> String {
    let [r, g, b] = clamp3(c).map(|v| (v * 255.0).round() as u8);
    format!("{r:02X}{g:02X}{b:02X}")
}

fn c32(c: [f32; 3]) -> Color32 {
    let [r, g, b] = clamp3(c).map(|v| (v * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

// ------------------------------------------------------------------ drawing

/// Rects of the last frame's controls (screen points), for tests and automation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelRects {
    pub mode: Option<ColorPanelMode>,
    pub field: Option<Rect>,
    pub strip: Option<Rect>,
    pub ramps: Vec<Rect>,
    pub spectrum: Option<Rect>,
    pub foreground: Option<Rect>,
    pub background: Option<Rect>,
}

fn rects_id() -> egui::Id {
    egui::Id::new("color-panel-rects")
}

pub fn last_rects(ctx: &egui::Context) -> PanelRects {
    ctx.data(|d| d.get_temp::<PanelRects>(rects_id())).unwrap_or_default()
}

/// Draw the Color panel.
pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let Some(sp) = spaces(app, &ctx) else {
        ui.label(tl!("Color management is unavailable."));
        return;
    };
    let mut rects = PanelRects { mode: Some(app.ui.color_panel.mode), ..Default::default() };
    let mode = app.ui.color_panel.mode;
    // The controls shrink with the group (down to `min_room`); below that the panel scrolls. The
    // readout goes below them once they have the room they look right in.
    let avail = ui.available_height();
    let need = min_room(mode);
    let mut draw = |app: &mut PhotocraftApp, ui: &mut egui::Ui, room: f32| {
        if mode.is_field() {
            field_modes(app, ui, &sp, &mut rects, room);
        } else {
            slider_modes(app, ui, &sp, &mut rects, room);
        }
    };
    // The readout folds onto a second line in a narrow dock: reserve what it took last frame.
    let readout_h = ctx.data(|d| d.get_temp::<f32>(readout_id())).unwrap_or(READOUT) + 6.0;
    if avail.is_finite() && avail >= comfortable_room(mode) + readout_h {
        draw(app, ui, (avail - readout_h).min(MAX_ROOM));
        ui.add_space(6.0);
        let h = readout(app, ui, &sp);
        ctx.data_mut(|d| d.insert_temp(readout_id(), h));
    } else if avail.is_finite() && avail >= need {
        draw(app, ui, avail.min(MAX_ROOM));
    } else {
        egui::ScrollArea::vertical()
            .id_salt("color-panel-scroll")
            .max_height(if avail.is_finite() { avail.max(0.0) } else { need })
            .auto_shrink([false, false])
            .show(ui, |ui| draw(app, ui, need));
    }
    ctx.data_mut(|d| d.insert_temp(rects_id(), rects));
}

/// Height the panel content may grow to; taller groups leave the rest empty, as in Photoshop.
const MAX_ROOM: f32 = 220.0;
const FIELD_MIN: f32 = 56.0;
const ROW_MIN: f32 = 18.0;
const ROW_MAX: f32 = 26.0;
const SPECTRUM: f32 = 24.0;
/// The hex and R, G, B readout on one line.
const READOUT: f32 = 24.0;
const CHIPS_W: f32 = 38.0;

/// The smallest height a mode's controls fit in without scrolling.
fn min_room(mode: ColorPanelMode) -> f32 {
    if mode.is_field() { FIELD_MIN } else { ROW_MIN * mode.components().len() as f32 }
}

/// The height the controls need before the readout gets room below them: a field that reads as
/// a field, or every slider plus the spectrum bar.
fn comfortable_room(mode: ColorPanelMode) -> f32 {
    if mode.is_field() { FIELD_MIN + 40.0 } else { ROW_MIN * mode.components().len() as f32 + SPECTRUM + 6.0 }
}

fn readout_id() -> egui::Id {
    egui::Id::new("color-panel-readout-height")
}

/// The editable hex and R, G, B fields of the edited colour. Returns the height they took.
fn readout(app: &mut PhotocraftApp, ui: &mut egui::Ui, sp: &Spaces) -> f32 {
    let bg = app.ui.color_panel.background;
    let mut edited = if bg { app.session.tools.background } else { app.session.tools.foreground };
    let resp = ui.horizontal(|ui| crate::panels::color_readout(ui, ui.id().with(("color-panel-readout", bg)), &mut edited));
    if resp.inner {
        pick(app, &ui.ctx().clone(), sp, [edited[0], edited[1], edited[2]]);
    }
    resp.response.rect.height()
}

/// The foreground and background chips. A click picks the colour the panel edits, framed; a
/// double-click opens the Color Picker on it.
fn chips(app: &mut PhotocraftApp, ui: &mut egui::Ui, height: f32, rects: &mut PanelRects) {
    let t = Tokens::get(ui.ctx());
    let (area, _) = ui.allocate_exact_size(vec2(CHIPS_W, height), Sense::hover());
    let bgr = Rect::from_min_size(area.min + vec2(13.0, 13.0), vec2(22.0, 22.0));
    let fgr = Rect::from_min_size(area.min + vec2(3.0, 3.0), vec2(22.0, 22.0));
    let frame = Stroke::new(1.0, t.text_dim);
    let bg_active = app.ui.color_panel.background;
    let [fg, bg] = [app.session.tools.foreground, app.session.tools.background].map(|c| c32([c[0], c[1], c[2]]));
    let p = ui.painter();
    p.rect_filled(bgr, 2.0, bg);
    p.rect_stroke(bgr, 2.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    if bg_active {
        p.rect_stroke(bgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    p.rect_filled(fgr, 2.0, fg);
    p.rect_stroke(fgr, 2.0, Stroke::new(1.0, Color32::from_gray(210)), StrokeKind::Outside);
    if !bg_active {
        p.rect_stroke(fgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    rects.foreground = Some(fgr);
    rects.background = Some(bgr);
    // The foreground is on top, so it takes the clicks where the two overlap.
    let bg_resp = ui.interact(bgr, ui.id().with("field-bg"), Sense::click());
    let fg_resp = ui.interact(fgr, ui.id().with("field-fg"), Sense::click());
    let picked = if fg_resp.clicked() { false } else { bg_resp.clicked() || bg_active };
    app.ui.color_panel.background = picked;
    if fg_resp.double_clicked() || bg_resp.double_clicked() {
        crate::color_picker_ui::open(app, if picked { "background" } else { "foreground" });
    }
}

/// Fill `rect` with a `n`×`n` grid of colours from `f(fx, fy)` (0–1 across and down).
fn grid_mesh(p: &egui::Painter, rect: Rect, n: usize, f: impl Fn(f32, f32) -> Color32) {
    let mut mesh = egui::Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (fx, fy) = (i as f32 / n as f32, j as f32 / n as f32);
            mesh.colored_vertex(pos2(rect.left() + fx * rect.width(), rect.top() + fy * rect.height()), f(fx, fy));
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * w + i;
            mesh.add_triangle(a, a + 1, a + w + 1);
            mesh.add_triangle(a, a + w + 1, a + w);
        }
    }
    p.add(mesh);
}

/// A horizontal ramp of colours from `f(fx)`.
fn ramp_mesh(p: &egui::Painter, rect: Rect, stops: &[Color32]) {
    if stops.len() < 2 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    let n = stops.len() - 1;
    for (i, c) in stops.iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / n as f32;
        mesh.colored_vertex(pos2(x, rect.top()), *c);
        mesh.colored_vertex(pos2(x, rect.bottom()), *c);
    }
    for k in 0..n as u32 {
        let a = k * 2;
        mesh.add_triangle(a, a + 1, a + 3);
        mesh.add_triangle(a, a + 3, a + 2);
    }
    p.add(mesh);
}

fn knob_ring(p: &egui::Painter, at: Pos2, color: [f32; 3]) {
    // Dark ring on light colours, light ring on dark ones (readable on any colour).
    let light = 0.299 * color[0] + 0.587 * color[1] + 0.114 * color[2] > 0.55;
    let (inner, outer) = if light { (Color32::from_gray(20), Color32::from_white_alpha(140)) } else { (Color32::WHITE, Color32::from_black_alpha(140)) };
    p.circle_stroke(at, 4.5, Stroke::new(1.5, inner));
    p.circle_stroke(at, 6.0, Stroke::new(1.0, outer));
}

/// Which part of the wheel a drag started on.
#[derive(Clone, Copy, PartialEq)]
enum WheelPart {
    Ring,
    Square,
}

fn field_modes(app: &mut PhotocraftApp, ui: &mut egui::Ui, sp: &Spaces, rects: &mut PanelRects, room: f32) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let mode = app.ui.color_panel.mode;
    let mut hsb = components(app, &ctx, sp);
    hsb.resize(3, 0.0);
    let (h, s, b) = (hsb[0], hsb[1], hsb[2]);
    let height = (room - 2.0).clamp(FIELD_MIN, 170.0);
    let mut edited: Option<[f32; 3]> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        chips(app, ui, height, rects);
        let avail = ui.available_width();
        if mode == ColorPanelMode::ColorWheel {
            let side = (avail - 4.0).min(height).max(40.0);
            let (area, _) = ui.allocate_exact_size(vec2(avail.max(side), height), Sense::hover());
            let wheel = Rect::from_center_size(area.center(), vec2(side, side));
            let resp = ui.interact(wheel, ui.id().with("color-wheel"), Sense::click_and_drag());
            let p = ui.painter();
            let c = wheel.center();
            let r_out = side / 2.0;
            let ring = (side * 0.12).clamp(9.0, 16.0);
            let r_in = r_out - ring;
            // Hue ring: red at the right, hues counter-clockwise (screen y points down).
            let segs = 96;
            let mut mesh = egui::Mesh::default();
            for k in 0..=segs {
                let f = k as f32 / segs as f32;
                let a = f * std::f32::consts::TAU;
                let col = c32(hsb_to_rgb([f * 360.0, 100.0, 100.0]));
                let d = vec2(a.cos(), -a.sin());
                mesh.colored_vertex(c + d * r_in, col);
                mesh.colored_vertex(c + d * r_out, col);
            }
            for k in 0..segs as u32 {
                let a = k * 2;
                mesh.add_triangle(a, a + 1, a + 3);
                mesh.add_triangle(a, a + 3, a + 2);
            }
            p.add(mesh);
            let half = (r_in - 5.0) / std::f32::consts::SQRT_2;
            let sq = Rect::from_center_size(c, vec2(half * 2.0, half * 2.0));
            grid_mesh(p, sq, 12, |fx, fy| c32(hsb_to_rgb([h, fx * 100.0, (1.0 - fy) * 100.0])));
            p.rect_stroke(sq, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
            let a = h.to_radians();
            let hk = c + vec2(a.cos(), -a.sin()) * (r_in + ring / 2.0);
            p.circle_stroke(hk, ring / 2.0 - 1.0, Stroke::new(1.5, Color32::WHITE));
            p.circle_stroke(hk, ring / 2.0, Stroke::new(1.0, Color32::from_black_alpha(160)));
            let sk = pos2(sq.left() + s / 100.0 * sq.width(), sq.top() + (1.0 - b / 100.0) * sq.height());
            knob_ring(p, sk, hsb_to_rgb([h, s, b]));
            rects.field = Some(sq);
            rects.strip = Some(wheel);
            let part_id = ui.id().with("color-wheel-part");
            if resp.drag_started() || resp.clicked() {
                let part = resp.interact_pointer_pos().map(|q| if (q - c).length() >= r_in - 2.0 { WheelPart::Ring } else { WheelPart::Square });
                ui.data_mut(|d| d.insert_temp(part_id, part));
            }
            if (resp.dragged() || resp.clicked())
                && let Some(q) = resp.interact_pointer_pos()
            {
                match ui.data(|d| d.get_temp::<Option<WheelPart>>(part_id)).flatten() {
                    Some(WheelPart::Ring) => {
                        let d = q - c;
                        let hue = (-d.y).atan2(d.x).to_degrees().rem_euclid(360.0);
                        edited = Some([hue, s, b]);
                    }
                    Some(WheelPart::Square) => {
                        let fx = ((q.x - sq.left()) / sq.width()).clamp(0.0, 1.0);
                        let fy = ((q.y - sq.top()) / sq.height()).clamp(0.0, 1.0);
                        edited = Some([h, fx * 100.0, (1.0 - fy) * 100.0]);
                    }
                    None => {}
                }
            }
        } else {
            let strip_w = 14.0;
            let field_w = (avail - strip_w - 18.0).max(40.0);
            let (field, fresp) = ui.allocate_exact_size(vec2(field_w, height), Sense::click_and_drag());
            let (strip, sresp) = ui.allocate_exact_size(vec2(strip_w, height), Sense::click_and_drag());
            let p = ui.painter();
            let hue_cube = mode == ColorPanelMode::HueCube;
            if hue_cube {
                grid_mesh(p, field, 16, |fx, fy| c32(hsb_to_rgb([h, fx * 100.0, (1.0 - fy) * 100.0])));
            } else {
                // Brightness Cube: hue across, saturation down (full at the top).
                grid_mesh(p, field, 24, |fx, fy| c32(hsb_to_rgb([fx * 360.0, (1.0 - fy) * 100.0, b])));
            }
            p.rect_stroke(field, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
            let stops: Vec<Color32> = if hue_cube {
                (0..=24).map(|k| c32(hsb_to_rgb([360.0 * (1.0 - k as f32 / 24.0), 100.0, 100.0]))).collect()
            } else {
                (0..=8).map(|k| c32(hsb_to_rgb([h, s, 100.0 * (1.0 - k as f32 / 8.0)]))).collect()
            };
            // The strip is a vertical ramp: build it rotated.
            let mut mesh = egui::Mesh::default();
            let n = stops.len().saturating_sub(1).max(1);
            for (k, col) in stops.iter().enumerate() {
                let y = strip.top() + strip.height() * k as f32 / n as f32;
                mesh.colored_vertex(pos2(strip.left(), y), *col);
                mesh.colored_vertex(pos2(strip.right(), y), *col);
            }
            for k in 0..n as u32 {
                let a = k * 2;
                mesh.add_triangle(a, a + 1, a + 3);
                mesh.add_triangle(a, a + 3, a + 2);
            }
            p.add(mesh);
            p.rect_stroke(strip, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
            let (fx, fy, sf) = if hue_cube { (s / 100.0, 1.0 - b / 100.0, 1.0 - h / 360.0) } else { (h / 360.0, 1.0 - s / 100.0, 1.0 - b / 100.0) };
            knob_ring(p, pos2(field.left() + fx * field.width(), field.top() + fy * field.height()), hsb_to_rgb([h, s, b]));
            let y = strip.top() + sf.clamp(0.0, 1.0) * strip.height();
            let tri = vec![pos2(strip.right() + 1.0, y), pos2(strip.right() + 6.0, y - 4.0), pos2(strip.right() + 6.0, y + 4.0)];
            p.add(egui::Shape::convex_polygon(tri, t.text, Stroke::NONE));
            let tri = vec![pos2(strip.left() - 1.0, y), pos2(strip.left() - 6.0, y + 4.0), pos2(strip.left() - 6.0, y - 4.0)];
            p.add(egui::Shape::convex_polygon(tri, t.text, Stroke::NONE));
            rects.field = Some(field);
            rects.strip = Some(strip);
            if (fresp.dragged() || fresp.clicked())
                && let Some(q) = fresp.interact_pointer_pos()
            {
                let fx = ((q.x - field.left()) / field.width()).clamp(0.0, 1.0);
                let fy = ((q.y - field.top()) / field.height()).clamp(0.0, 1.0);
                edited = Some(if hue_cube { [h, fx * 100.0, (1.0 - fy) * 100.0] } else { [(fx * 360.0).min(359.99), (1.0 - fy) * 100.0, b] });
            }
            if (sresp.dragged() || sresp.clicked())
                && let Some(q) = sresp.interact_pointer_pos()
            {
                let f = ((q.y - strip.top()) / strip.height()).clamp(0.0, 1.0);
                edited = Some(if hue_cube { [((1.0 - f) * 360.0).min(359.99), s, b] } else { [h, s, (1.0 - f) * 100.0] });
            }
        }
    });
    if let Some(e) = edited {
        commit(app, &ctx, sp, e.to_vec());
    }
}

fn slider_modes(app: &mut PhotocraftApp, ui: &mut egui::Ui, sp: &Spaces, rects: &mut PanelRects, room: f32) {
    let ctx = ui.ctx().clone();
    let mode = app.ui.color_panel.mode;
    let comps_spec = mode.components();
    let mut comps = components(app, &ctx, sp);
    comps.resize(comps_spec.len(), 0.0);
    let n = comps_spec.len().max(1) as f32;
    let show_spectrum = room >= ROW_MIN * n + SPECTRUM + 6.0;
    let row_h = ((room - if show_spectrum { SPECTRUM + 6.0 } else { 0.0 }) / n).floor().clamp(ROW_MIN, ROW_MAX);
    let rows_h = (row_h * n).max(36.0);
    let mut edited: Option<Vec<f32>> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        chips(app, ui, rows_h, rects);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // Rows are exactly `row_h` (a horizontal layout is otherwise at least interact_size tall).
            ui.spacing_mut().interact_size.y = row_h;
            for (i, comp) in comps_spec.iter().enumerate() {
                let mut v = comps.get(i).copied().unwrap_or(comp.min);
                let stops: Vec<Color32> = (0..=16)
                    .map(|k| {
                        let mut c = comps.clone();
                        if let Some(slot) = c.get_mut(i) {
                            *slot = comp.min + (comp.max - comp.min) * k as f32 / 16.0;
                        }
                        // The hue ramp shows full colours, not the current saturation and brightness.
                        if mode == ColorPanelMode::Hsb && i == 0 {
                            c = vec![comp.min + (comp.max - comp.min) * k as f32 / 16.0, 100.0, 100.0];
                        }
                        c32(from_components(if mode == ColorPanelMode::Web { ColorPanelMode::Rgb } else { mode }, &c, sp))
                    })
                    .collect();
                let changed = ramp_row(ui, mode, comp, &mut v, &stops, row_h, rects);
                if changed {
                    let mut c = comps.clone();
                    if let Some(slot) = c.get_mut(i) {
                        *slot = v;
                    }
                    edited = Some(c);
                }
            }
        });
    });
    if let Some(c) = edited {
        commit(app, &ctx, sp, c);
    }
    if show_spectrum {
        ui.add_space(6.0);
        spectrum(app, ui, sp, rects);
    }
}

/// One slider: `label [ramp ▲] [value]`.
fn ramp_row(ui: &mut egui::Ui, mode: ColorPanelMode, comp: &Component, v: &mut f32, stops: &[Color32], row_h: f32, rects: &mut PanelRects) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (lr, _) = ui.allocate_exact_size(vec2(12.0, row_h), Sense::hover());
        ui.painter().text(lr.left_center(), egui::Align2::LEFT_CENTER, comp.label, theme::mono(12.0), t.text_dim);
        let field_w = if mode == ColorPanelMode::Web { 34.0 } else { 52.0 };
        let ramp_w = (ui.available_width() - field_w - 6.0).max(30.0);
        let (area, resp) = ui.allocate_exact_size(vec2(ramp_w, row_h), Sense::click_and_drag());
        // Ramp above centre, its ▲ knob just below it.
        let top = area.center().y - 7.0;
        let track = Rect::from_min_max(pos2(area.left() + 4.0, top), pos2(area.right() - 4.0, top + 8.0));
        let p = ui.painter();
        ramp_mesh(p, track, stops);
        p.rect_stroke(track, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        let span = (comp.max - comp.min).max(f32::EPSILON);
        let f = ((*v - comp.min) / span).clamp(0.0, 1.0);
        let x = track.left() + f * track.width();
        let tri = vec![pos2(x, track.bottom() + 1.0), pos2(x + 4.5, track.bottom() + 7.0), pos2(x - 4.5, track.bottom() + 7.0)];
        p.add(egui::Shape::convex_polygon(tri, t.text, Stroke::new(1.0, t.card)));
        rects.ramps.push(track);
        if (resp.dragged() || resp.clicked())
            && let Some(q) = resp.interact_pointer_pos()
        {
            let f = ((q.x - track.left()) / track.width()).clamp(0.0, 1.0);
            let mut nv = comp.min + f * span;
            if mode == ColorPanelMode::Web {
                nv = web_safe(nv);
            }
            if (nv - *v).abs() > 1e-4 {
                *v = nv;
                changed = true;
            }
        }
        if mode == ColorPanelMode::Web {
            // Two hex digits per channel.
            let id = ui.id().with(("web-hex", comp.label));
            // The colour's own value: only edits snap to the web-safe cube.
            let shown = format!("{:02X}", v.clamp(0.0, 255.0).round() as u8);
            let mut text = ui.data(|d| d.get_temp::<String>(id)).filter(|_| ui.memory(|m| m.has_focus(id))).unwrap_or(shown);
            let fh = (row_h - 2.0).min(22.0);
            let r = ui.add_sized(
                vec2(field_w, fh),
                egui::TextEdit::singleline(&mut text).id(id).font(theme::mono(12.0)).char_limit(2).margin(vec2(4.0, 0.0)).vertical_align(egui::Align::Center),
            );
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
            if (r.lost_focus() || r.changed())
                && let Ok(n) = u8::from_str_radix(text.trim(), 16)
                && text.trim().len() == 2
            {
                let nv = web_safe(f32::from(n));
                if (nv - *v).abs() > 1e-4 {
                    *v = nv;
                    changed = true;
                }
            }
        } else {
            let mut shown = (*v).round();
            if number_field(ui, &mut shown, comp, field_w, (row_h - 2.0).min(22.0)).changed() {
                *v = shown.clamp(comp.min, comp.max);
                changed = true;
            }
        }
    });
    changed
}

/// A compact numeric field (type or drag to scrub) with a dim unit, sized to the slider row.
fn number_field(ui: &mut egui::Ui, value: &mut f32, comp: &Component, width: f32, height: f32) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    widgets::surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let unit_w = if comp.unit.is_empty() { 0.0 } else { 12.0 };
    let inner = Rect::from_min_max(rect.min + vec2(3.0, 1.0), rect.max - vec2(3.0 + unit_w, 1.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::right_to_left(egui::Align::Center)));
    {
        let st = child.style_mut();
        for w in [&mut st.visuals.widgets.inactive, &mut st.visuals.widgets.hovered] {
            w.bg_fill = Color32::TRANSPARENT;
            w.weak_bg_fill = Color32::TRANSPARENT;
            w.bg_stroke = Stroke::NONE;
        }
        st.override_font_id = Some(theme::mono(12.0));
    }
    let resp = child.add_sized(inner.size(), egui::DragValue::new(value).range(comp.min..=comp.max).speed(0.5).custom_formatter(|v, _| format!("{v:.0}")));
    if !comp.unit.is_empty() {
        ui.painter().text(pos2(rect.right() - 4.0, rect.center().y), egui::Align2::RIGHT_CENTER, comp.unit, theme::mono(10.0), t.text_faint);
    }
    resp
}

/// Photoshop's RGB spectrum bar, with white and black at the right end, and the out-of-gamut
/// warning for the working CMYK space at the left.
fn spectrum(app: &mut PhotocraftApp, ui: &mut egui::Ui, sp: &Spaces, rects: &mut PanelRects) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let rgb = current(app);
    let printable = sp.printable(rgb);
    let out_of_gamut = sp.out_of_gamut(rgb);
    let mut picked: Option<[f32; 3]> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.spacing_mut().interact_size.y = SPECTRUM;
        let (warn, wresp) = ui.allocate_exact_size(vec2(36.0, SPECTRUM), Sense::click());
        if out_of_gamut {
            let p = ui.painter();
            let c = pos2(warn.left() + 9.0, warn.center().y);
            let tri = vec![pos2(c.x, c.y - 7.0), pos2(c.x + 8.0, c.y + 6.0), pos2(c.x - 8.0, c.y + 6.0)];
            p.add(egui::Shape::convex_polygon(tri, t.warning, Stroke::NONE));
            p.text(pos2(c.x, c.y + 1.5), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(10.0), Color32::BLACK);
            let sw = Rect::from_min_size(pos2(warn.left() + 20.0, c.y - 6.0), vec2(12.0, 12.0));
            p.rect_filled(sw, 0.0, c32(printable));
            p.rect_stroke(sw, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
            if wresp.clicked() {
                picked = Some(printable);
            }
            wresp.on_hover_text(tl!("Out of gamut for printing: click to use the closest printable color"));
        }
        let w = ui.available_width();
        let (bar, resp) = ui.allocate_exact_size(vec2(w, SPECTRUM), Sense::click_and_drag());
        let ends = 12.0;
        let spec = Rect::from_min_max(bar.min, pos2(bar.right() - ends, bar.bottom()));
        let p = ui.painter();
        // Hue across; white at the top, full colour in the middle, black at the bottom.
        let mut mesh = egui::Mesh::default();
        let n = 24;
        for k in 0..=n {
            let f = k as f32 / n as f32;
            let x = spec.left() + f * spec.width();
            let full = c32(hsb_to_rgb([f * 360.0, 100.0, 100.0]));
            mesh.colored_vertex(pos2(x, spec.top()), Color32::WHITE);
            mesh.colored_vertex(pos2(x, spec.center().y), full);
            mesh.colored_vertex(pos2(x, spec.bottom()), Color32::BLACK);
        }
        for k in 0..n as u32 {
            let a = k * 3;
            for (u, d) in [(a, a + 1), (a + 1, a + 2)] {
                mesh.add_triangle(u, u + 3, d + 3);
                mesh.add_triangle(u, d + 3, d);
            }
        }
        p.add(mesh);
        let white = Rect::from_min_max(pos2(spec.right(), bar.top()), pos2(bar.right(), bar.center().y));
        let black = Rect::from_min_max(pos2(spec.right(), bar.center().y), bar.max);
        p.rect_filled(white, 0.0, Color32::WHITE);
        p.rect_filled(black, 0.0, Color32::BLACK);
        p.rect_stroke(bar, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        rects.spectrum = Some(bar);
        if (resp.dragged() || resp.clicked())
            && let Some(q) = resp.interact_pointer_pos()
        {
            picked = Some(if q.x >= spec.right() {
                if q.y < bar.center().y { [1.0; 3] } else { [0.0; 3] }
            } else {
                let fx = ((q.x - spec.left()) / spec.width()).clamp(0.0, 1.0);
                let fy = ((q.y - spec.top()) / spec.height()).clamp(0.0, 1.0);
                let (s, b) = if fy < 0.5 { (fy * 2.0 * 100.0, 100.0) } else { (100.0, (1.0 - (fy - 0.5) * 2.0) * 100.0) };
                hsb_to_rgb([(fx * 360.0).min(359.99), s, b])
            });
        }
    });
    if let Some(c) = picked {
        pick(app, &ctx, sp, c);
    }
}

#[cfg(test)]
#[path = "color_panel_tests.rs"]
mod tests;
