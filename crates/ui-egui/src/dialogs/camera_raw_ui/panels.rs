//! The Edit column's presentation state, as Camera Raw lays it out: which panels are open, which
//! are hidden by their eye button (their settings stay in the dialog but neither the preview nor
//! OK applies them, as in Camera Raw), the Color Mixer's Adjust mode and tab, and the Curve
//! panel's channel. Driven by the dialog and by `{"ui": {"panels" | "mixer" | "curve": …}}`.

use std::collections::BTreeSet;

use super::{CameraRaw, Value, json};

/// The Edit panels in Camera Raw's order, with their control-channel ids and English titles.
pub(super) const PANELS: [(&str, &str); 9] = [
    ("light", "Light"),
    ("color", "Color"),
    ("effects", "Effects"),
    ("curve", "Curve"),
    ("detail", "Detail"),
    ("colorMixer", "Color Mixer"),
    ("colorGrading", "Color Grading"),
    ("geometry", "Geometry"),
    ("calibration", "Calibration"),
];

/// Color Mixer › Adjust: the HSL sliders by property, or by colour (one band's three sliders).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum MixerAdjust {
    #[default]
    Hsl,
    Color,
}

impl MixerAdjust {
    pub(super) fn id(self) -> &'static str {
        match self {
            MixerAdjust::Hsl => "hsl",
            MixerAdjust::Color => "color",
        }
    }
}

/// Color Mixer HSL tabs: Hue, Saturation, Luminance, All.
pub(super) const MIXER_TABS: [(&str, &str); 4] = [("hue", "Hue"), ("saturation", "Saturation"), ("luminance", "Luminance"), ("all", "All")];
pub(super) const MIXER_ALL: usize = 3;

/// Curve channels: the parametric curve, then the point curve's master, red, green and blue.
pub(super) const CURVE_CHANNELS: [&str; 5] = ["parametric", "point", "red", "green", "blue"];

/// Color Grading views: the three wheels together (Camera Raw's default), then one large wheel
/// for Shadows, Midtones, Highlights or Global, with control-channel ids and English names.
pub(super) const GRADING_VIEWS: [(&str, &str); 5] =
    [("threeWay", "3-Way"), ("shadows", "Shadows"), ("midtones", "Midtones"), ("highlights", "Highlights"), ("global", "Global")];

/// How the preview compares Before with After, as Camera Raw's Before/After views: the After
/// image alone, Before and After side by side, or one image split down the middle (Before left).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Compare {
    #[default]
    Single,
    SideBySide,
    Split,
}

impl Compare {
    pub(super) const ALL: [Compare; 3] = [Compare::Single, Compare::SideBySide, Compare::Split];

    pub(super) fn id(self) -> &'static str {
        match self {
            Compare::Single => "single",
            Compare::SideBySide => "sideBySide",
            Compare::Split => "split",
        }
    }

    /// The next view, as Camera Raw's Before/After button cycles them.
    pub(super) fn next(self) -> Compare {
        match self {
            Compare::Single => Compare::SideBySide,
            Compare::SideBySide => Compare::Split,
            Compare::Split => Compare::Single,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PanelState {
    pub(super) open: BTreeSet<&'static str>,
    pub(super) hidden: BTreeSet<&'static str>,
    pub(super) mixer_adjust: MixerAdjust,
    /// Index into [`MIXER_TABS`].
    pub(super) mixer_tab: usize,
    /// The band Adjust › Color edits (index into the eight HSL bands).
    pub(super) mixer_color: usize,
    /// Index into [`CURVE_CHANNELS`].
    pub(super) curve_channel: usize,
    /// Index into [`GRADING_VIEWS`].
    pub(super) grading_view: usize,
    /// The preview's Before/After view.
    pub(super) compare: Compare,
}

impl Default for PanelState {
    fn default() -> Self {
        PanelState {
            open: BTreeSet::from(["light"]),
            hidden: BTreeSet::new(),
            mixer_adjust: MixerAdjust::Hsl,
            mixer_tab: 0,
            mixer_color: 0,
            curve_channel: 0,
            grading_view: 0,
            compare: Compare::Single,
        }
    }
}

fn panel_id(name: &str) -> Option<&'static str> {
    PANELS.iter().map(|(id, _)| *id).find(|id| *id == name)
}

fn panel_set(value: &Value, key: &str) -> Result<BTreeSet<&'static str>, String> {
    let list = value.as_array().ok_or_else(|| format!("panels.{key} must be a list of panel ids"))?;
    list.iter().map(|v| v.as_str().and_then(panel_id).ok_or_else(|| format!("panels.{key}: unknown panel {v}; expected one of {}", ids()))).collect()
}

fn ids() -> String {
    PANELS.iter().map(|(id, _)| *id).collect::<Vec<_>>().join(", ")
}

impl PanelState {
    /// The settings the preview and OK apply: hidden panels' settings back at their defaults.
    pub(super) fn effective(&self, p: &CameraRaw) -> CameraRaw {
        let mut out = p.clone();
        if self.hidden.is_empty() {
            return out;
        }
        let d = CameraRaw::default();
        for id in &self.hidden {
            match *id {
                "light" => {
                    (out.exposure, out.contrast, out.highlights, out.shadows, out.whites, out.blacks) =
                        (d.exposure, d.contrast, d.highlights, d.shadows, d.whites, d.blacks)
                }
                "color" => (out.temperature, out.tint, out.vibrance, out.saturation) = (d.temperature, d.tint, d.vibrance, d.saturation),
                "effects" => {
                    (out.texture, out.clarity, out.dehaze) = (d.texture, d.clarity, d.dehaze);
                    (out.vignette_amount, out.vignette_midpoint, out.vignette_roundness, out.vignette_feather, out.vignette_highlights) =
                        (d.vignette_amount, d.vignette_midpoint, d.vignette_roundness, d.vignette_feather, d.vignette_highlights);
                    out.vignette_style = d.vignette_style.clone();
                    (out.grain_amount, out.grain_size, out.grain_roughness) = (d.grain_amount, d.grain_size, d.grain_roughness);
                }
                "curve" => {
                    (out.curve_highlights, out.curve_lights, out.curve_darks, out.curve_shadows, out.curve_splits) =
                        (d.curve_highlights, d.curve_lights, d.curve_darks, d.curve_shadows, d.curve_splits);
                    out.point_curve = Vec::new();
                    out.point_curve_red = Vec::new();
                    out.point_curve_green = Vec::new();
                    out.point_curve_blue = Vec::new();
                }
                "detail" => {
                    (out.sharpen_amount, out.sharpen_radius, out.sharpen_detail, out.sharpen_masking) =
                        (d.sharpen_amount, d.sharpen_radius, d.sharpen_detail, d.sharpen_masking);
                    (out.noise_luminance, out.noise_luminance_detail, out.noise_color, out.noise_color_detail) =
                        (d.noise_luminance, d.noise_luminance_detail, d.noise_color, d.noise_color_detail);
                }
                "colorMixer" => (out.hsl_hue, out.hsl_sat, out.hsl_lum) = (d.hsl_hue, d.hsl_sat, d.hsl_lum),
                "colorGrading" => {
                    (out.grade_shadows, out.grade_midtones, out.grade_highlights, out.grade_global) =
                        (d.grade_shadows, d.grade_midtones, d.grade_highlights, d.grade_global);
                    (out.grade_blending, out.grade_balance) = (d.grade_blending, d.grade_balance);
                }
                "geometry" => super::geometry_panel::reset(&mut out, &d),
                "calibration" => (out.shadow_tint, out.calibration_hue, out.calibration_sat) = (d.shadow_tint, d.calibration_hue, d.calibration_sat),
                _ => {}
            }
        }
        out
    }

    pub(super) fn describe(&self) -> Value {
        json!({
            "open": self.open.iter().collect::<Vec<_>>(),
            "hidden": self.hidden.iter().collect::<Vec<_>>(),
            "mixer": {
                "adjust": self.mixer_adjust.id(),
                "tab": MIXER_TABS.get(self.mixer_tab).map(|t| t.0),
                "color": super::BANDS.get(self.mixer_color).map(|b| b.to_ascii_lowercase()),
            },
            "curve": CURVE_CHANNELS.get(self.curve_channel),
            "grading": GRADING_VIEWS.get(self.grading_view).map(|v| v.0),
            "compare": self.compare.id(),
        })
    }

    /// The state after a control request's `panels`, `mixer` and `curve` parts; `self` is
    /// untouched when any part is invalid.
    pub(super) fn prepare(&self, ui: &Value) -> Result<PanelState, String> {
        let mut next = self.clone();
        if let Some(panels) = ui.get("panels") {
            let fields = panels.as_object().ok_or("panels must be an object {open?, hidden?}")?;
            for (key, value) in fields {
                match key.as_str() {
                    "open" => next.open = panel_set(value, "open")?,
                    "hidden" => next.hidden = panel_set(value, "hidden")?,
                    _ => return Err(format!("unknown panels property {key}")),
                }
            }
        }
        if let Some(mixer) = ui.get("mixer") {
            let fields = mixer.as_object().ok_or("mixer must be an object {adjust?, tab?, color?}")?;
            for (key, value) in fields {
                let name = value.as_str().map(str::to_ascii_lowercase);
                match key.as_str() {
                    "adjust" => {
                        next.mixer_adjust = match name.as_deref() {
                            Some("hsl") => MixerAdjust::Hsl,
                            Some("color") => MixerAdjust::Color,
                            _ => return Err("mixer.adjust must be hsl or color".into()),
                        }
                    }
                    "tab" => {
                        next.mixer_tab =
                            MIXER_TABS.iter().position(|t| Some(t.0) == name.as_deref()).ok_or("mixer.tab must be hue, saturation, luminance or all")?
                    }
                    "color" => {
                        next.mixer_color = super::BANDS
                            .iter()
                            .position(|b| name.as_deref() == Some(b.to_ascii_lowercase().as_str()))
                            .ok_or("mixer.color must be red, orange, yellow, green, aqua, blue, purple or magenta")?
                    }
                    _ => return Err(format!("unknown mixer property {key}")),
                }
            }
        }
        if let Some(curve) = ui.get("curve") {
            next.curve_channel =
                curve.as_str().and_then(|c| CURVE_CHANNELS.iter().position(|x| *x == c)).ok_or("curve must be parametric, point, red, green or blue")?;
        }
        if let Some(view) = ui.get("grading") {
            next.grading_view = view
                .as_str()
                .and_then(|v| GRADING_VIEWS.iter().position(|x| x.0 == v))
                .ok_or("grading must be threeWay, shadows, midtones, highlights or global")?;
        }
        if let Some(view) = ui.get("compare") {
            next.compare = view.as_str().and_then(|v| Compare::ALL.into_iter().find(|c| c.id() == v)).ok_or("compare must be single, sideBySide or split")?;
        }
        Ok(next)
    }
}

/// `{"ui": {"reset": "default" | "open"}}`: the settings Camera Raw's Reset to Default / Reset to
/// Open give. The proxy's pixel scale stays.
pub(super) fn reset_params(kind: &Value, current: &CameraRaw, original: &CameraRaw) -> Result<CameraRaw, String> {
    match kind.as_str() {
        Some("default") => Ok(defaults_for(current)),
        Some("open") => {
            let mut p = original.clone();
            p.pixel_scale = current.pixel_scale;
            Ok(p)
        }
        _ => Err("reset must be default or open".into()),
    }
}

/// Every setting at its default, keeping what describes the image rather than the edit.
pub(super) fn defaults_for(current: &CameraRaw) -> CameraRaw {
    CameraRaw { pixel_scale: current.pixel_scale, seed: current.seed, encoding_gamma: current.encoding_gamma, ..CameraRaw::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_panels_apply_their_defaults_and_keep_the_rest() {
        let mut p = CameraRaw { exposure: 1.0, contrast: 20.0, vibrance: 30.0, grade_blending: 70.0, ..CameraRaw::default() };
        p.point_curve_red = vec![[0.0, 10.0], [255.0, 255.0]];
        let mut s = PanelState::default();
        assert_eq!(s.effective(&p), p, "nothing hidden: unchanged");
        s.hidden = BTreeSet::from(["light", "curve", "colorGrading"]);
        let e = s.effective(&p);
        assert_eq!((e.exposure, e.contrast, e.vibrance, e.grade_blending), (0.0, 0.0, 30.0, 50.0));
        assert!(e.point_curve_red.is_empty());
    }

    #[test]
    fn bad_requests_leave_the_state_alone() {
        let s = PanelState::default();
        for bad in [
            json!({"panels": {"open": ["nope"]}}),
            json!({"panels": {"open": "light"}}),
            json!({"panels": {"shut": []}}),
            json!({"panels": 3}),
            json!({"mixer": {"adjust": "lab"}}),
            json!({"mixer": {"tab": 2}}),
            json!({"mixer": {"color": "teal"}}),
            json!({"mixer": {"speed": 1}}),
            json!({"curve": "alpha"}),
            json!({"curve": null}),
            json!({"grading": "fourWay"}),
            json!({"grading": 1}),
            json!({"compare": "overlay"}),
            json!({"compare": true}),
        ] {
            assert!(s.prepare(&bad).is_err(), "{bad}");
        }
        let next = s.prepare(&json!({"panels": {"open": ["colorMixer"], "hidden": ["light"]}, "mixer": {"adjust": "color", "tab": "all", "color": "Aqua"}, "curve": "green"})).unwrap();
        assert_eq!(next.open, BTreeSet::from(["colorMixer"]));
        assert_eq!(
            (next.mixer_adjust, next.mixer_tab, next.mixer_color, next.curve_channel),
            (MixerAdjust::Color, MIXER_ALL, 4, 3),
            "aqua is band 4, green channel 3"
        );
        assert!(reset_params(&json!("everything"), &CameraRaw::default(), &CameraRaw::default()).is_err());
    }

    /// Color Grading opens on the three wheels and Before/After on the After image alone, as in
    /// Camera Raw; both are set by id and reported back.
    #[test]
    fn grading_and_compare_views_round_trip() {
        let s = PanelState::default();
        assert_eq!((s.grading_view, s.compare), (0, Compare::Single));
        let next = s.prepare(&json!({"grading": "highlights", "compare": "split"})).unwrap();
        assert_eq!((next.grading_view, next.compare), (3, Compare::Split));
        let d = next.describe();
        assert_eq!((d["grading"].as_str(), d["compare"].as_str()), (Some("highlights"), Some("split")));
        let cycle: Vec<Compare> = std::iter::successors(Some(Compare::Single), |c| Some(c.next())).take(4).collect();
        assert_eq!(cycle, [Compare::Single, Compare::SideBySide, Compare::Split, Compare::Single]);
    }
}
