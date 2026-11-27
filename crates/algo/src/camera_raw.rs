//! Filter › Camera Raw Filter: thin delegation to [`craft_develop::camera_raw`].
//!
//! PhotoCraft keeps its [`craft::imaging::raw::RawDevelopSettings`] (`CameraRaw`) type at the
//! public boundary for UI/command/GPU callers and converts to [`craft_develop::camera_raw::CameraRaw`]
//! before delegating all CPU math, tone mapping, spatial filters, and per-pixel parameter packing
//! to `craft_develop::camera_raw`.

pub use craft::imaging::raw::{GradeWheel as Wheel, MAX_CURVE_POINTS, RawDevelopSettings as CameraRaw, VignetteStyle as StoredVignetteStyle, validate_curve};
pub use craft_develop::camera_raw::{
    CURVE_LUT, Clarity, Dehaze, DevelopInputs, Grade, Grain, HSL_BANDS, LocalTone, PerPixel, Texture, ToneKind, TonePass, Vignette, VignetteStyle, WB_STEPS,
    WhitesBlacks, WhitesBlacksModel, band_weights, calibration_weights, curve_lut, local_tone, migrate_to_develop_settings, quantile_rank, white_balance_gains,
    whites_blacks_model,
};

pub mod spatial {
    pub use craft_develop::camera_raw::spatial::*;
}

fn to_shared_wheel(w: Wheel) -> craft_develop::camera_raw::Wheel {
    craft_develop::camera_raw::Wheel { hue: w.hue, sat: w.sat, lum: w.lum }
}

fn from_shared_wheel(w: craft_develop::camera_raw::Wheel) -> Wheel {
    Wheel { hue: w.hue, sat: w.sat, lum: w.lum }
}

fn to_shared_style(s: StoredVignetteStyle) -> craft_develop::camera_raw::StoredVignetteStyle {
    match s {
        StoredVignetteStyle::ColorPriority => craft_develop::camera_raw::StoredVignetteStyle::ColorPriority,
        StoredVignetteStyle::PaintOverlay => craft_develop::camera_raw::StoredVignetteStyle::PaintOverlay,
        StoredVignetteStyle::HighlightPriority => craft_develop::camera_raw::StoredVignetteStyle::HighlightPriority,
    }
}

fn from_shared_style(s: craft_develop::camera_raw::StoredVignetteStyle) -> StoredVignetteStyle {
    match s {
        craft_develop::camera_raw::StoredVignetteStyle::ColorPriority => StoredVignetteStyle::ColorPriority,
        craft_develop::camera_raw::StoredVignetteStyle::PaintOverlay => StoredVignetteStyle::PaintOverlay,
        craft_develop::camera_raw::StoredVignetteStyle::HighlightPriority => StoredVignetteStyle::HighlightPriority,
    }
}

fn to_shared(p: &CameraRaw) -> craft_develop::camera_raw::CameraRaw {
    let mut s = craft_develop::camera_raw::CameraRaw::default();
    s.temperature = p.temperature;
    s.tint = p.tint;
    s.exposure = p.exposure;
    s.contrast = p.contrast;
    s.highlights = p.highlights;
    s.shadows = p.shadows;
    s.whites = p.whites;
    s.blacks = p.blacks;
    s.texture = p.texture;
    s.clarity = p.clarity;
    s.dehaze = p.dehaze;
    s.vibrance = p.vibrance;
    s.saturation = p.saturation;
    s.curve_highlights = p.curve_highlights;
    s.curve_lights = p.curve_lights;
    s.curve_darks = p.curve_darks;
    s.curve_shadows = p.curve_shadows;
    s.curve_splits = p.curve_splits;
    s.point_curve.clone_from(&p.point_curve);
    s.point_curve_red.clone_from(&p.point_curve_red);
    s.point_curve_green.clone_from(&p.point_curve_green);
    s.point_curve_blue.clone_from(&p.point_curve_blue);
    s.hsl_hue = p.hsl_hue;
    s.hsl_sat = p.hsl_sat;
    s.hsl_lum = p.hsl_lum;
    s.grade_shadows = to_shared_wheel(p.grade_shadows);
    s.grade_midtones = to_shared_wheel(p.grade_midtones);
    s.grade_highlights = to_shared_wheel(p.grade_highlights);
    s.grade_global = to_shared_wheel(p.grade_global);
    s.grade_blending = p.grade_blending;
    s.grade_balance = p.grade_balance;
    s.sharpen_amount = p.sharpen_amount;
    s.sharpen_radius = p.sharpen_radius;
    s.sharpen_detail = p.sharpen_detail;
    s.sharpen_masking = p.sharpen_masking;
    s.noise_luminance = p.noise_luminance;
    s.noise_luminance_detail = p.noise_luminance_detail;
    s.noise_color = p.noise_color;
    s.noise_color_detail = p.noise_color_detail;
    s.grain_amount = p.grain_amount;
    s.grain_size = p.grain_size;
    s.grain_roughness = p.grain_roughness;
    s.vignette_amount = p.vignette_amount;
    s.vignette_midpoint = p.vignette_midpoint;
    s.vignette_roundness = p.vignette_roundness;
    s.vignette_feather = p.vignette_feather;
    s.vignette_highlights = p.vignette_highlights;
    s.vignette_style = to_shared_style(p.vignette_style);
    s.shadow_tint = p.shadow_tint;
    s.calibration_hue = p.calibration_hue;
    s.calibration_sat = p.calibration_sat;
    s
}

fn copy_from_shared(s: &craft_develop::camera_raw::CameraRaw, p: &mut CameraRaw) {
    p.temperature = s.temperature;
    p.tint = s.tint;
    p.exposure = s.exposure;
    p.contrast = s.contrast;
    p.highlights = s.highlights;
    p.shadows = s.shadows;
    p.whites = s.whites;
    p.blacks = s.blacks;
    p.texture = s.texture;
    p.clarity = s.clarity;
    p.dehaze = s.dehaze;
    p.vibrance = s.vibrance;
    p.saturation = s.saturation;
    p.curve_highlights = s.curve_highlights;
    p.curve_lights = s.curve_lights;
    p.curve_darks = s.curve_darks;
    p.curve_shadows = s.curve_shadows;
    p.curve_splits = s.curve_splits;
    p.point_curve.clone_from(&s.point_curve);
    p.point_curve_red.clone_from(&s.point_curve_red);
    p.point_curve_green.clone_from(&s.point_curve_green);
    p.point_curve_blue.clone_from(&s.point_curve_blue);
    p.hsl_hue = s.hsl_hue;
    p.hsl_sat = s.hsl_sat;
    p.hsl_lum = s.hsl_lum;
    p.grade_shadows = from_shared_wheel(s.grade_shadows);
    p.grade_midtones = from_shared_wheel(s.grade_midtones);
    p.grade_highlights = from_shared_wheel(s.grade_highlights);
    p.grade_global = from_shared_wheel(s.grade_global);
    p.grade_blending = s.grade_blending;
    p.grade_balance = s.grade_balance;
    p.sharpen_amount = s.sharpen_amount;
    p.sharpen_radius = s.sharpen_radius;
    p.sharpen_detail = s.sharpen_detail;
    p.sharpen_masking = s.sharpen_masking;
    p.noise_luminance = s.noise_luminance;
    p.noise_luminance_detail = s.noise_luminance_detail;
    p.noise_color = s.noise_color;
    p.noise_color_detail = s.noise_color_detail;
    p.grain_amount = s.grain_amount;
    p.grain_size = s.grain_size;
    p.grain_roughness = s.grain_roughness;
    p.vignette_amount = s.vignette_amount;
    p.vignette_midpoint = s.vignette_midpoint;
    p.vignette_roundness = s.vignette_roundness;
    p.vignette_feather = s.vignette_feather;
    p.vignette_highlights = s.vignette_highlights;
    p.vignette_style = from_shared_style(s.vignette_style);
    p.shadow_tint = s.shadow_tint;
    p.calibration_hue = s.calibration_hue;
    p.calibration_sat = s.calibration_sat;
}

/// Fills in an automatic tone adjustment from the image's percentiles (`px` in sRGB).
pub fn auto_tone(px: &[[f32; 4]], p: &mut CameraRaw) {
    let mut s = to_shared(p);
    craft_develop::camera_raw::auto_tone(px, &mut s);
    copy_from_shared(&s, p);
}

/// Runs the Camera Raw pipeline on straight RGBA pixels (`w × h`, display-encoded RGB).
pub fn develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, inputs: &DevelopInputs, float: bool) {
    craft_develop::camera_raw::develop(px, w, h, &to_shared(p), inputs, float);
}

/// Per-pixel stages packed for the GPU preview pass.
pub fn per_pixel(p: &CameraRaw, inputs: &DevelopInputs, w: usize, h: usize, float: bool, points: Option<(f32, f32)>) -> Option<PerPixel> {
    craft_develop::camera_raw::per_pixel(&to_shared(p), inputs, w, h, float, points)
}

/// Whether `p` needs a spatial pass on the GPU (`Dehaze`, `Shadows`/`Highlights`, `Clarity`, `Texture`, `Detail`).
pub fn spatial(p: &CameraRaw) -> bool {
    craft_develop::camera_raw::spatial(&to_shared(p))
}

/// Whether a stage before Whites and Blacks reads neighbouring pixels (`Dehaze`, `Shadows`/`Highlights`, `Texture`, `Clarity`).
pub fn spatial_tone(p: &CameraRaw) -> bool {
    craft_develop::camera_raw::spatial_tone(&to_shared(p))
}

/// Image black and white points for `Whites` and `Blacks`, measured on the CPU.
pub fn white_black_points(px: &[[f32; 4]], w: usize, h: usize, p: &CameraRaw, inputs: &DevelopInputs) -> Option<(f32, f32)> {
    craft_develop::camera_raw::white_black_points(px, w, h, &to_shared(p), inputs)
}
