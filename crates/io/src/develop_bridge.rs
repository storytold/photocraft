//! Camera Raw settings ([`CameraRaw`], Camera Raw's slider units) to and from the shared develop
//! model ([`DevelopSettings`], `craft-develop-model`), so a raw opened through Camera Raw can be
//! rendered by the shared develop engine.
//!
//! Both sides use Camera Raw's slider units and read the same `crs:` keys (compare
//! [`crate::camera_raw_xmp`] with `craft_develop_model::crs`), so most controls map one to one,
//! with the same ranges and signs. The exceptions:
//!
//! * **White balance.** PhotoCraft's Temperature / Tint are relative, −100..=100 about the as-shot
//!   white (`crs:IncrementalTemperature` / `IncrementalTint`). The model stores a white in Kelvin
//!   and tint, on one of two scales ([`WbScale`]). [`WbBasis::Relative`] is the model's relative
//!   scale (6500 K / 0 means *as shot*; temperature a mired shift of 0.8 per step, as LightCraft's
//!   panel and `craft_develop_model::crs::rel_to_kelvin` define it), for files the Kelvin scale does
//!   not reach. [`WbBasis::Kelvin`] is for raws with colour matrices: the same mired shift and tint
//!   offset, about the file's real as-shot white. Zero / zero is written as `As Shot` with the
//!   exact as-shot values. The model reads its relative scale colorimetrically (a Bradford
//!   adaptation), PhotoCraft's own develop as channel gains, so equal slider values do not give
//!   identical colours; the mapping keeps the slider positions.
//! * **Point curves.** PhotoCraft: `[input, output]` in 0..=255; the model: points in 0..=1.
//! * **Color Mixer.** The eight bands map by index (red, orange, yellow, green, aqua, blue, purple,
//!   magenta), which is also how both read `crs:HueAdjustment<Band>` and friends. Their centre hues
//!   differ for two bands: PhotoCraft's `HSL_BANDS` puts blue at 240° and magenta at 300°, the
//!   model's `MIXER_HUES` at 225° and 315° (the others agree: 0, 30, 60, 120, 180, 270). A slider
//!   value moves the band of the same name, so blues and magentas near the band edges respond a
//!   little differently between the two renderers.
//! * **Text enums.** Vignette style and Upright mode are strings in PhotoCraft
//!   (`highlightPriority|colorPriority|paintOverlay`, `off|auto|level|vertical|full|guided`) and
//!   enums in the model; an unknown string reads as the default (Highlight Priority, Off), as
//!   PhotoCraft's own pipeline treats it.
//!
//! Mapped, one to one: Exposure, Contrast, Highlights, Shadows, Whites, Blacks; Texture, Clarity,
//! Dehaze; Vibrance, Saturation; the parametric curve (four regions, three splits); the master,
//! red, green and blue point curves; Color Mixer hue / saturation / luminance; Color Grading
//! (four wheels, Blending, Balance); Sharpening Amount / Radius / Detail / Masking; Luminance and
//! Color noise reduction with their Detail; Grain Amount / Size / Roughness and its seed; post-crop
//! Vignette (amount, midpoint, roundness, feather, highlights, style); Calibration (shadows tint,
//! red / green / blue primary hue and saturation); Upright mode and Guided guides (fractions of the
//! frame before geometry on both sides: PhotoCraft applies no lens correction first); Manual
//! Transformations Vertical, Horizontal, Rotate, Aspect, Scale, Offset X / Y.
//!
//! **PhotoCraft fields with no counterpart** (kept from the base [`CameraRaw`] on the way back):
//! `pixel_scale` (a preview's proxy scale: a render parameter, the shared renderer takes a size
//! instead) and `encoding_gamma` (the document's transfer curve: a render parameter, the shared
//! renderer is told the output space). The raw-only options of [`crate::raw::RawTuning`]
//! (demosaic method, sidecar crop) are not in [`CameraRaw`] and are not mapped here.
//!
//! **Model fields with no counterpart** (kept from the base [`DevelopSettings`] on the way in):
//! `process`, `version`, `profile`, `treatment` (B&W) and `bw_mix`, `hdr`, `point_colors`,
//! `curve.refine_saturation`, `detail.nr_contrast`, `detail.nr_color_smoothness`,
//! `detail.nr_outliers`, all of `optics` (lens profile, chromatic aberration, defringe, manual
//! distortion and lens vignetting), `geometry.upright_transform` (the model's cached Upright
//! solution: the renderer must solve it, see the phase-2 notes), `geometry.constrain_crop`,
//! `crop`, `orientation`, `masks`, `spots`, `red_eye`, `lens_blur`, `enhance`,
//! `disabled_sections`, and the white-balance presets of `wb.mode` (Daylight, Tungsten, …: read
//! back through their temperature and tint).

use craft_develop_model::{DevelopSettings, Hsl, Upright, VignetteStyle, WbMode, WbScale, Wheel as ModelWheel, crs::rel_to_kelvin};
use craft_geom::Point;
use photocraft_algo::camera_raw::{CameraRaw, Wheel};

/// The relative scale's as-shot reference temperature (Kelvin).
const REL_REFERENCE_K: f64 = 6500.0;
/// Mireds per step of the relative temperature slider (LightCraft's relative scale).
const MIRED_PER_STEP: f64 = 0.8;

/// Which white-balance scale a file is developed on (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WbBasis {
    /// The model's relative scale: 6500 K / 0 is as shot ([`WbScale::Legacy`]). For files without
    /// colour matrices (developed from a look fitted to the camera's preview).
    Relative,
    /// Kelvin about the file's own as-shot white ([`WbScale::Kelvin`]), for raws with colour
    /// matrices.
    Kelvin { as_shot_temp: f64, as_shot_tint: f64 },
}

/// A finite `f32` as `f64`, else `fallback` (hostile settings never reach the renderer as NaN).
fn f(v: f32, fallback: f64) -> f64 {
    if v.is_finite() { f64::from(v) } else { fallback }
}

/// A model value as PhotoCraft's `f32`, else `fallback`.
fn g(v: f64, fallback: f32) -> f32 {
    if v.is_finite() { v as f32 } else { fallback }
}

/// PhotoCraft's relative Temperature / Tint as the model's white balance on `basis`.
fn wb_to_model(temperature: f32, tint: f32, basis: WbBasis) -> craft_develop_model::WhiteBalance {
    let r = f(temperature, 0.0).clamp(-100.0, 100.0);
    let t = f(tint, 0.0).clamp(-100.0, 100.0);
    let (ref_k, ref_tint, scale) = match basis {
        WbBasis::Relative => (REL_REFERENCE_K, 0.0, WbScale::Legacy),
        WbBasis::Kelvin { as_shot_temp, as_shot_tint } if as_shot_temp.is_finite() && as_shot_temp > 0.0 && as_shot_tint.is_finite() => {
            (as_shot_temp, as_shot_tint, WbScale::Kelvin)
        }
        // an unusable as-shot white: the relative scale, which needs none
        WbBasis::Kelvin { .. } => (REL_REFERENCE_K, 0.0, WbScale::Legacy),
    };
    if r == 0.0 && t == 0.0 {
        return craft_develop_model::WhiteBalance { mode: WbMode::AsShot, temp: ref_k, tint: ref_tint, scale };
    }
    let temp = if scale == WbScale::Legacy { rel_to_kelvin(r) } else { 1e6 / (1e6 / ref_k - r * MIRED_PER_STEP) };
    craft_develop_model::WhiteBalance { mode: WbMode::Custom, temp, tint: ref_tint + t, scale }
}

/// The model's white balance as PhotoCraft's relative Temperature / Tint on `basis`.
fn wb_from_model(wb: &craft_develop_model::WhiteBalance, basis: WbBasis) -> (f32, f32) {
    if wb.mode == WbMode::AsShot {
        return (0.0, 0.0);
    }
    let (ref_k, ref_tint) = match basis {
        WbBasis::Kelvin { as_shot_temp, as_shot_tint } if as_shot_temp.is_finite() && as_shot_temp > 0.0 && as_shot_tint.is_finite() => {
            (as_shot_temp, as_shot_tint)
        }
        _ => (REL_REFERENCE_K, 0.0),
    };
    let r = if wb.temp.is_finite() && wb.temp > 0.0 { (1e6 / ref_k - 1e6 / wb.temp) / MIRED_PER_STEP } else { 0.0 };
    let t = if wb.tint.is_finite() { wb.tint - ref_tint } else { 0.0 };
    (g(r.clamp(-100.0, 100.0), 0.0), g(t.clamp(-100.0, 100.0), 0.0))
}

fn curve_to_model(pts: &[[f32; 2]]) -> Vec<Point> {
    pts.iter().filter(|p| p.iter().all(|v| v.is_finite())).map(|[x, y]| Point::new(f64::from(*x) / 255.0, f64::from(*y) / 255.0)).collect()
}

fn curve_from_model(pts: &[Point]) -> Vec<[f32; 2]> {
    pts.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| [(p.x * 255.0) as f32, (p.y * 255.0) as f32]).collect()
}

fn wheel_to_model(w: &Wheel) -> ModelWheel {
    ModelWheel { hue: f(w.hue, 0.0), sat: f(w.sat, 0.0), lum: f(w.lum, 0.0) }
}

fn wheel_from_model(w: &ModelWheel) -> Wheel {
    Wheel { hue: g(w.hue, 0.0), sat: g(w.sat, 0.0), lum: g(w.lum, 0.0) }
}

fn vignette_style_to_model(s: &str) -> VignetteStyle {
    match s {
        "colorPriority" => VignetteStyle::ColorPriority,
        "paintOverlay" => VignetteStyle::PaintOverlay,
        _ => VignetteStyle::HighlightPriority,
    }
}

fn vignette_style_from_model(s: VignetteStyle) -> &'static str {
    match s {
        VignetteStyle::HighlightPriority => "highlightPriority",
        VignetteStyle::ColorPriority => "colorPriority",
        VignetteStyle::PaintOverlay => "paintOverlay",
    }
}

fn upright_to_model(s: &str) -> Upright {
    match s {
        "auto" => Upright::Auto,
        "level" => Upright::Level,
        "vertical" => Upright::Vertical,
        "full" => Upright::Full,
        "guided" => Upright::Guided,
        _ => Upright::Off,
    }
}

fn upright_from_model(u: Upright) -> &'static str {
    match u {
        Upright::Off => "off",
        Upright::Auto => "auto",
        Upright::Level => "level",
        Upright::Vertical => "vertical",
        Upright::Full => "full",
        Upright::Guided => "guided",
    }
}

/// The Color Mixer bands as the model's, by index (see the module docs on the centre hues).
fn mixer_bands(m: &mut craft_develop_model::Mixer) -> [&mut Hsl; 8] {
    [&mut m.red, &mut m.orange, &mut m.yellow, &mut m.green, &mut m.aqua, &mut m.blue, &mut m.purple, &mut m.magenta]
}

/// `cr` as develop settings, starting from `base` for the fields Camera Raw has no control for
/// (module docs). `basis` is the file's white-balance scale.
pub fn to_develop(cr: &CameraRaw, basis: WbBasis, base: &DevelopSettings) -> DevelopSettings {
    let mut d = base.clone();
    d.wb = wb_to_model(cr.temperature, cr.tint, basis);

    let l = &mut d.light;
    l.exposure = f(cr.exposure, 0.0);
    l.contrast = f(cr.contrast, 0.0);
    l.highlights = f(cr.highlights, 0.0);
    l.shadows = f(cr.shadows, 0.0);
    l.whites = f(cr.whites, 0.0);
    l.blacks = f(cr.blacks, 0.0);
    d.effects.texture = f(cr.texture, 0.0);
    d.effects.clarity = f(cr.clarity, 0.0);
    d.effects.dehaze = f(cr.dehaze, 0.0);
    d.color.vibrance = f(cr.vibrance, 0.0);
    d.color.saturation = f(cr.saturation, 0.0);

    let c = &mut d.curve;
    c.highlights = f(cr.curve_highlights, 0.0);
    c.lights = f(cr.curve_lights, 0.0);
    c.darks = f(cr.curve_darks, 0.0);
    c.shadows = f(cr.curve_shadows, 0.0);
    c.split_shadows = f(cr.curve_splits[0], 25.0);
    c.split_mid = f(cr.curve_splits[1], 50.0);
    c.split_highlights = f(cr.curve_splits[2], 75.0);
    c.master = curve_to_model(&cr.point_curve);
    c.red = curve_to_model(&cr.point_curve_red);
    c.green = curve_to_model(&cr.point_curve_green);
    c.blue = curve_to_model(&cr.point_curve_blue);

    for (i, band) in mixer_bands(&mut d.mixer).into_iter().enumerate() {
        band.hue = f(cr.hsl_hue[i], 0.0);
        band.sat = f(cr.hsl_sat[i], 0.0);
        band.lum = f(cr.hsl_lum[i], 0.0);
    }

    let gr = &mut d.grading;
    gr.shadows = wheel_to_model(&cr.grade_shadows);
    gr.midtones = wheel_to_model(&cr.grade_midtones);
    gr.highlights = wheel_to_model(&cr.grade_highlights);
    gr.global = wheel_to_model(&cr.grade_global);
    gr.blending = f(cr.grade_blending, 50.0);
    gr.balance = f(cr.grade_balance, 0.0);

    let de = &mut d.detail;
    de.sharpen_amount = f(cr.sharpen_amount, 0.0);
    de.sharpen_radius = f(cr.sharpen_radius, 1.0);
    de.sharpen_detail = f(cr.sharpen_detail, 25.0);
    de.sharpen_masking = f(cr.sharpen_masking, 0.0);
    de.nr_luminance = f(cr.noise_luminance, 0.0);
    de.nr_detail = f(cr.noise_luminance_detail, 50.0);
    de.nr_color = f(cr.noise_color, 0.0);
    de.nr_color_detail = f(cr.noise_color_detail, 50.0);

    d.grain.amount = f(cr.grain_amount, 0.0);
    d.grain.size = f(cr.grain_size, 25.0);
    d.grain.roughness = f(cr.grain_roughness, 50.0);
    d.grain.seed = cr.seed;
    let v = &mut d.vignette;
    v.amount = f(cr.vignette_amount, 0.0);
    v.midpoint = f(cr.vignette_midpoint, 50.0);
    v.roundness = f(cr.vignette_roundness, 0.0);
    v.feather = f(cr.vignette_feather, 50.0);
    v.highlights = f(cr.vignette_highlights, 0.0);
    v.style = vignette_style_to_model(&cr.vignette_style);

    let ca = &mut d.calibration;
    ca.shadows_tint = f(cr.shadow_tint, 0.0);
    ca.red_hue = f(cr.calibration_hue[0], 0.0);
    ca.green_hue = f(cr.calibration_hue[1], 0.0);
    ca.blue_hue = f(cr.calibration_hue[2], 0.0);
    ca.red_sat = f(cr.calibration_sat[0], 0.0);
    ca.green_sat = f(cr.calibration_sat[1], 0.0);
    ca.blue_sat = f(cr.calibration_sat[2], 0.0);

    let ge = &mut d.geometry;
    let upright = upright_to_model(&cr.upright);
    if upright != ge.upright {
        // the cached solution was for another mode
        ge.upright_transform = None;
    }
    ge.upright = upright;
    ge.guides = cr
        .upright_guides
        .iter()
        .filter(|s| s.iter().flatten().all(|v| v.is_finite()))
        .map(|[a, b]| (Point::new(f64::from(a[0]), f64::from(a[1])), Point::new(f64::from(b[0]), f64::from(b[1]))))
        .collect();
    ge.vertical = f(cr.transform_vertical, 0.0);
    ge.horizontal = f(cr.transform_horizontal, 0.0);
    ge.rotate = f(cr.transform_rotate, 0.0);
    ge.aspect = f(cr.transform_aspect, 0.0);
    ge.scale = f(cr.transform_scale, 100.0);
    ge.offset_x = f(cr.transform_offset_x, 0.0);
    ge.offset_y = f(cr.transform_offset_y, 0.0);
    d
}

/// `d` as Camera Raw settings, starting from `base` for the fields the model has no counterpart
/// for (`pixel_scale`, `encoding_gamma`). `basis` is the file's white-balance scale, as given to
/// [`to_develop`].
pub fn to_camera_raw(d: &DevelopSettings, basis: WbBasis, base: &CameraRaw) -> CameraRaw {
    let mut cr = base.clone();
    (cr.temperature, cr.tint) = wb_from_model(&d.wb, basis);

    cr.exposure = g(d.light.exposure, 0.0);
    cr.contrast = g(d.light.contrast, 0.0);
    cr.highlights = g(d.light.highlights, 0.0);
    cr.shadows = g(d.light.shadows, 0.0);
    cr.whites = g(d.light.whites, 0.0);
    cr.blacks = g(d.light.blacks, 0.0);
    cr.texture = g(d.effects.texture, 0.0);
    cr.clarity = g(d.effects.clarity, 0.0);
    cr.dehaze = g(d.effects.dehaze, 0.0);
    cr.vibrance = g(d.color.vibrance, 0.0);
    cr.saturation = g(d.color.saturation, 0.0);

    let c = &d.curve;
    cr.curve_highlights = g(c.highlights, 0.0);
    cr.curve_lights = g(c.lights, 0.0);
    cr.curve_darks = g(c.darks, 0.0);
    cr.curve_shadows = g(c.shadows, 0.0);
    cr.curve_splits = [g(c.split_shadows, 25.0), g(c.split_mid, 50.0), g(c.split_highlights, 75.0)];
    cr.point_curve = curve_from_model(&c.master);
    cr.point_curve_red = curve_from_model(&c.red);
    cr.point_curve_green = curve_from_model(&c.green);
    cr.point_curve_blue = curve_from_model(&c.blue);

    for (i, band) in d.mixer.bands().iter().enumerate() {
        cr.hsl_hue[i] = g(band.hue, 0.0);
        cr.hsl_sat[i] = g(band.sat, 0.0);
        cr.hsl_lum[i] = g(band.lum, 0.0);
    }

    let gr = &d.grading;
    cr.grade_shadows = wheel_from_model(&gr.shadows);
    cr.grade_midtones = wheel_from_model(&gr.midtones);
    cr.grade_highlights = wheel_from_model(&gr.highlights);
    cr.grade_global = wheel_from_model(&gr.global);
    cr.grade_blending = g(gr.blending, 50.0);
    cr.grade_balance = g(gr.balance, 0.0);

    let de = &d.detail;
    cr.sharpen_amount = g(de.sharpen_amount, 0.0);
    cr.sharpen_radius = g(de.sharpen_radius, 1.0);
    cr.sharpen_detail = g(de.sharpen_detail, 25.0);
    cr.sharpen_masking = g(de.sharpen_masking, 0.0);
    cr.noise_luminance = g(de.nr_luminance, 0.0);
    cr.noise_luminance_detail = g(de.nr_detail, 50.0);
    cr.noise_color = g(de.nr_color, 0.0);
    cr.noise_color_detail = g(de.nr_color_detail, 50.0);

    cr.grain_amount = g(d.grain.amount, 0.0);
    cr.grain_size = g(d.grain.size, 25.0);
    cr.grain_roughness = g(d.grain.roughness, 50.0);
    cr.seed = d.grain.seed;
    let v = &d.vignette;
    cr.vignette_amount = g(v.amount, 0.0);
    cr.vignette_midpoint = g(v.midpoint, 50.0);
    cr.vignette_roundness = g(v.roundness, 0.0);
    cr.vignette_feather = g(v.feather, 50.0);
    cr.vignette_highlights = g(v.highlights, 0.0);
    cr.vignette_style = vignette_style_from_model(v.style).into();

    let ca = &d.calibration;
    cr.shadow_tint = g(ca.shadows_tint, 0.0);
    cr.calibration_hue = [g(ca.red_hue, 0.0), g(ca.green_hue, 0.0), g(ca.blue_hue, 0.0)];
    cr.calibration_sat = [g(ca.red_sat, 0.0), g(ca.green_sat, 0.0), g(ca.blue_sat, 0.0)];

    let ge = &d.geometry;
    cr.upright = upright_from_model(ge.upright).into();
    cr.upright_guides = ge
        .guides
        .iter()
        .filter(|(a, b)| [a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite()))
        .map(|(a, b)| [[a.x as f32, a.y as f32], [b.x as f32, b.y as f32]])
        .collect();
    cr.transform_vertical = g(ge.vertical, 0.0);
    cr.transform_horizontal = g(ge.horizontal, 0.0);
    cr.transform_rotate = g(ge.rotate, 0.0);
    cr.transform_aspect = g(ge.aspect, 0.0);
    cr.transform_scale = g(ge.scale, 100.0);
    cr.transform_offset_x = g(ge.offset_x, 0.0);
    cr.transform_offset_y = g(ge.offset_y, 0.0);
    cr
}

#[cfg(test)]
mod tests {
    use super::*;

    const KELVIN: WbBasis = WbBasis::Kelvin { as_shot_temp: 5150.0, as_shot_tint: 7.0 };

    /// Every mapped control off its default, so a field mapped to the wrong place shows.
    fn busy() -> CameraRaw {
        let mut n = 0.0f32;
        let mut next = || {
            n += 1.0;
            n
        };
        let mut w = || Wheel { hue: 10.0 * next(), sat: next(), lum: -next() };
        let (gs, gm, gh, gg) = (w(), w(), w(), w());
        let mut next = || {
            n += 1.0;
            n
        };
        CameraRaw {
            temperature: 23.0,
            tint: -11.0,
            exposure: 0.75,
            contrast: next(),
            highlights: -next(),
            shadows: next(),
            whites: -next(),
            blacks: next(),
            texture: next(),
            clarity: -next(),
            dehaze: next(),
            vibrance: next(),
            saturation: -next(),
            curve_highlights: next(),
            curve_lights: -next(),
            curve_darks: next(),
            curve_shadows: -next(),
            curve_splits: [20.0, 45.0, 80.0],
            point_curve: vec![[0.0, 10.0], [128.0, 140.0], [255.0, 250.0]],
            point_curve_red: vec![[0.0, 0.0], [64.0, 70.0], [255.0, 255.0]],
            point_curve_green: vec![[0.0, 5.0], [255.0, 255.0]],
            point_curve_blue: vec![[0.0, 0.0], [200.0, 180.0], [255.0, 255.0]],
            hsl_hue: [1.0, -2.0, 3.0, -4.0, 5.0, -6.0, 7.0, -8.0],
            hsl_sat: [11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0],
            hsl_lum: [-21.0, -22.0, -23.0, -24.0, -25.0, -26.0, -27.0, -28.0],
            grade_shadows: gs,
            grade_midtones: gm,
            grade_highlights: gh,
            grade_global: gg,
            grade_blending: 63.0,
            grade_balance: -17.0,
            sharpen_amount: 55.0,
            sharpen_radius: 1.4,
            sharpen_detail: 33.0,
            sharpen_masking: 12.0,
            noise_luminance: 21.0,
            noise_luminance_detail: 44.0,
            noise_color: 30.0,
            noise_color_detail: 61.0,
            grain_amount: 25.0,
            grain_size: 31.0,
            grain_roughness: 66.0,
            vignette_amount: -40.0,
            vignette_midpoint: 35.0,
            vignette_roundness: 12.0,
            vignette_feather: 70.0,
            vignette_highlights: 15.0,
            vignette_style: "paintOverlay".into(),
            seed: 4242,
            pixel_scale: 0.25,
            encoding_gamma: 1.8,
            shadow_tint: -9.0,
            calibration_hue: [4.0, -5.0, 6.0],
            calibration_sat: [-7.0, 8.0, -9.0],
            upright: "guided".into(),
            upright_guides: vec![[[0.1, 0.2], [0.15, 0.8]], [[0.9, 0.1], [0.85, 0.9]]],
            transform_vertical: -30.0,
            transform_horizontal: 12.0,
            transform_rotate: 2.5,
            transform_aspect: -8.0,
            transform_scale: 110.0,
            transform_offset_x: 3.5,
            transform_offset_y: -4.5,
        }
    }

    #[test]
    fn defaults_map_to_the_models_defaults() {
        let d = to_develop(&CameraRaw::default(), WbBasis::Relative, &DevelopSettings::default());
        assert_eq!(d, DevelopSettings::default());
    }

    #[test]
    fn round_trip_is_identity_on_both_bases() {
        for basis in [WbBasis::Relative, KELVIN] {
            for cr in [CameraRaw::default(), busy()] {
                let d = to_develop(&cr, basis, &DevelopSettings::default());
                let back = to_camera_raw(&d, basis, &cr);
                assert_eq!(back, cr, "{basis:?}");
                // and with a default base: only the render parameters come from the base
                let fresh = to_camera_raw(&d, basis, &CameraRaw::default());
                assert_eq!(CameraRaw { pixel_scale: cr.pixel_scale, encoding_gamma: cr.encoding_gamma, ..fresh }, cr, "{basis:?}");
            }
        }
    }

    #[test]
    fn model_round_trip_keeps_unmapped_fields_from_the_base() {
        let mut base = DevelopSettings::for_raw(5150.0, 7.0);
        base.treatment = craft_develop_model::Treatment::Bw;
        base.optics.lens_profile = true;
        base.detail.nr_color_smoothness = 70.0;
        let d = to_develop(&busy(), KELVIN, &base);
        assert_eq!(d.treatment, base.treatment);
        assert_eq!(d.optics, base.optics);
        assert_eq!(d.detail.nr_color_smoothness, 70.0);
        assert!(d.detail.nr_outliers);
        let again = to_develop(&to_camera_raw(&d, KELVIN, &busy()), KELVIN, &base);
        assert_eq!(again, d);
    }

    #[test]
    fn relative_white_balance_follows_lightcrafts_scale() {
        let d = to_develop(&CameraRaw { temperature: 25.0, tint: -10.0, ..Default::default() }, WbBasis::Relative, &DevelopSettings::default());
        assert_eq!(d.wb.scale, WbScale::Legacy);
        assert_eq!(d.wb.mode, WbMode::Custom);
        assert!((d.wb.temp - rel_to_kelvin(25.0)).abs() < 1e-9 && d.wb.temp > 6500.0, "warmer is above 6500 K: {}", d.wb.temp);
        assert_eq!(d.wb.tint, -10.0);
        // LightCraft's panel reads it back with `k_to_rel`: (1e6/6500 − 1e6/K) / 0.8
        assert!(((1e6 / 6500.0 - 1e6 / d.wb.temp) / 0.8 - 25.0).abs() < 1e-9);
    }

    #[test]
    fn as_shot_is_exact_on_both_bases() {
        let d = to_develop(&CameraRaw::default(), WbBasis::Relative, &DevelopSettings::default());
        assert_eq!((d.wb.mode, d.wb.temp, d.wb.tint), (WbMode::AsShot, 6500.0, 0.0));
        let d = to_develop(&CameraRaw::default(), KELVIN, &DevelopSettings::default());
        assert_eq!((d.wb.mode, d.wb.temp, d.wb.tint, d.wb.scale), (WbMode::AsShot, 5150.0, 7.0, WbScale::Kelvin));
    }

    #[test]
    fn kelvin_white_balance_shifts_about_the_as_shot_white() {
        let d = to_develop(&CameraRaw { temperature: -40.0, tint: 12.0, ..Default::default() }, KELVIN, &DevelopSettings::default());
        assert_eq!(d.wb.scale, WbScale::Kelvin);
        assert!(d.wb.temp < 5150.0, "cooler is below the as-shot white: {}", d.wb.temp);
        assert!(((1e6 / 5150.0 - 1e6 / d.wb.temp) / 0.8 + 40.0).abs() < 1e-9);
        assert_eq!(d.wb.tint, 19.0);
    }

    #[test]
    fn an_unusable_as_shot_white_falls_back_to_the_relative_scale() {
        let basis = WbBasis::Kelvin { as_shot_temp: f64::NAN, as_shot_tint: 0.0 };
        let d = to_develop(&CameraRaw { temperature: 10.0, ..Default::default() }, basis, &DevelopSettings::default());
        assert_eq!(d.wb.scale, WbScale::Legacy);
        assert!((d.wb.temp - rel_to_kelvin(10.0)).abs() < 1e-9);
    }

    #[test]
    fn white_balance_from_a_preset_mode_reads_through_its_temperature() {
        let wb = craft_develop_model::WhiteBalance { mode: WbMode::Daylight, temp: rel_to_kelvin(-30.0), tint: 10.0, scale: WbScale::Legacy };
        let mut d = DevelopSettings { wb, ..Default::default() };
        let cr = to_camera_raw(&d, WbBasis::Relative, &CameraRaw::default());
        assert!((cr.temperature + 30.0).abs() < 1e-4 && cr.tint == 10.0, "{} {}", cr.temperature, cr.tint);
        // beyond the relative range: clamped to the slider
        d.wb.temp = 50000.0;
        assert_eq!(to_camera_raw(&d, WbBasis::Relative, &CameraRaw::default()).temperature, 100.0);
    }

    #[test]
    fn mixer_bands_map_by_name() {
        let mut cr = CameraRaw::default();
        cr.hsl_hue[5] = 40.0;
        cr.hsl_sat[7] = -60.0;
        cr.hsl_lum[0] = 15.0;
        let d = to_develop(&cr, WbBasis::Relative, &DevelopSettings::default());
        assert_eq!(d.mixer.blue.hue, 40.0);
        assert_eq!(d.mixer.magenta.sat, -60.0);
        assert_eq!(d.mixer.red.lum, 15.0);
        assert_eq!(craft_develop_model::MIXER_BANDS[5], "blue");
        assert_eq!(craft_develop_model::MIXER_BANDS[7], "magenta");
    }

    #[test]
    fn point_curves_scale_to_unit_range() {
        let cr = CameraRaw { point_curve: vec![[0.0, 0.0], [128.0, 140.0], [255.0, 255.0]], ..Default::default() };
        let d = to_develop(&cr, WbBasis::Relative, &DevelopSettings::default());
        assert_eq!(d.curve.master, vec![Point::new(0.0, 0.0), Point::new(128.0 / 255.0, 140.0 / 255.0), Point::new(1.0, 1.0)]);
        assert!(d.curve.red.is_empty());
    }

    #[test]
    fn text_enums_map_and_unknown_text_is_the_default() {
        for (s, v) in [
            ("highlightPriority", VignetteStyle::HighlightPriority),
            ("colorPriority", VignetteStyle::ColorPriority),
            ("paintOverlay", VignetteStyle::PaintOverlay),
        ] {
            assert_eq!(vignette_style_to_model(s), v);
            assert_eq!(vignette_style_from_model(v), s);
        }
        assert_eq!(vignette_style_to_model("bogus"), VignetteStyle::HighlightPriority);
        for s in photocraft_algo::camera_raw::UPRIGHT_MODES {
            assert_eq!(upright_from_model(upright_to_model(s)), s);
        }
        assert_eq!(upright_to_model("sideways"), Upright::Off);
    }

    #[test]
    fn a_new_upright_mode_drops_the_cached_solution() {
        let mut base = DevelopSettings::default();
        base.geometry.upright = Upright::Auto;
        base.geometry.upright_transform = Some(craft_geom::Homography([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]));
        let kept = to_develop(&CameraRaw { upright: "auto".into(), ..Default::default() }, WbBasis::Relative, &base);
        assert!(kept.geometry.upright_transform.is_some());
        let changed = to_develop(&CameraRaw { upright: "level".into(), ..Default::default() }, WbBasis::Relative, &base);
        assert!(changed.geometry.upright_transform.is_none());
    }

    #[test]
    fn non_finite_values_become_defaults() {
        let cr = CameraRaw {
            exposure: f32::NAN,
            transform_scale: f32::INFINITY,
            temperature: f32::NAN,
            point_curve: vec![[0.0, 0.0], [f32::NAN, 1.0], [255.0, 255.0]],
            upright_guides: vec![[[f32::NAN, 0.0], [1.0, 1.0]]],
            ..Default::default()
        };
        let d = to_develop(&cr, WbBasis::Relative, &DevelopSettings::default());
        assert_eq!(d.light.exposure, 0.0);
        assert_eq!(d.geometry.scale, 100.0);
        assert_eq!(d.wb.mode, WbMode::AsShot);
        assert_eq!(d.curve.master.len(), 2);
        assert!(d.geometry.guides.is_empty());
    }
}
