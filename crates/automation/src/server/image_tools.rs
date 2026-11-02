//! Task-shaped image tools: `resize_image`, `crop_image`, `apply_blur`, `batch_process` and the
//! rest. Each one is a thin, typed front for engine commands that `command_run` already
//! dispatches (`image.imageSize`, `filter.blur.gaussianBlur`, …): the tool validates its
//! arguments, derives the command parameters (aspect ratio, fit, rotation direction) and runs
//! the commands through [`Headless::command_run`], so the command authorization, the history
//! and the filesystem capabilities are exactly those of the generic tools.
//!
//! Results are structured JSON (`structuredContent` plus the same JSON as text). The tools run
//! in headless mode only; in bridge mode they say so and point at `command_run`.
//!
//! The pure planning functions ([`plan`]) hold the parameter logic and are unit tested without
//! an engine.

use std::path::Path;
use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ErrorData as McpError, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::PhotocraftMcp;
use crate::AutomationError;
use crate::budgets::json_bytes;
use crate::headless::Headless;

/// Most files one `batch_process` call looks at.
pub const MAX_BATCH_FILES: usize = 500;
/// Default `max_files` of `batch_process`.
const DEFAULT_BATCH_FILES: usize = 100;
/// Most operations one `batch_process` call applies to each file.
pub const MAX_BATCH_OPERATIONS: usize = 32;
/// Larger input files are skipped by `batch_process` (decoders also enforce their own limits).
pub const MAX_BATCH_FILE_BYTES: u64 = 512 << 20;
/// Largest canvas a task tool will resize to (256 megapixels): a typo such as `percent: 10000`
/// fails with a clear message instead of trying to allocate tens of gigabytes.
pub const MAX_TASK_PIXELS: u64 = 1 << 28;
/// Longest text `add_text` accepts.
const MAX_TEXT_CHARS: usize = 10_000;

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Resample {
    Automatic,
    Bicubic,
    BicubicSmoother,
    BicubicSharper,
    Bilinear,
    Nearest,
    Lanczos,
    PreserveDetails,
}

impl Resample {
    fn engine(self) -> &'static str {
        match self {
            Resample::Automatic => "automatic",
            Resample::Bicubic => "bicubic",
            Resample::BicubicSmoother => "bicubicSmoother",
            Resample::BicubicSharper => "bicubicSharper",
            Resample::Bilinear => "bilinear",
            Resample::Nearest => "nearest",
            Resample::Lanczos => "lanczos",
            Resample::PreserveDetails => "preserveDetails",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResizeParams {
    /// Target width in pixels. With only `width`, the height follows the aspect ratio.
    #[serde(default)]
    pub width: Option<u32>,
    /// Target height in pixels. With only `height`, the width follows the aspect ratio.
    #[serde(default)]
    pub height: Option<u32>,
    /// Scale both sides by this percentage (for example 50). Excludes `width`/`height`.
    #[serde(default)]
    pub percent: Option<f64>,
    /// Fit inside `width` x `height`, keeping the aspect ratio. Never enlarges unless `enlarge`.
    #[serde(default)]
    pub fit: bool,
    /// With `fit`: also scale up an image smaller than the box.
    #[serde(default)]
    pub enlarge: bool,
    /// Resampling method (default bicubic).
    #[serde(default)]
    pub resample: Option<Resample>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CropParams {
    /// Left edge of the kept area, in pixels from the left.
    pub x: i32,
    /// Top edge of the kept area, in pixels from the top.
    pub y: i32,
    /// Width of the kept area in pixels.
    pub width: u32,
    /// Height of the kept area in pixels.
    pub height: u32,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Bicubic,
    Bilinear,
    Nearest,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RotateParams {
    /// Degrees, positive = clockwise. Multiples of 90 turn the canvas losslessly; other angles
    /// rotate it with interpolation and grow the canvas to hold the result.
    pub angle: f64,
    /// Interpolation for angles that are not a multiple of 90 (default bicubic).
    #[serde(default)]
    pub interpolation: Option<Interpolation>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlipDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlipParams {
    /// `horizontal` mirrors left-right, `vertical` mirrors top-bottom (the whole canvas).
    pub direction: FlipDirection,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmountParams {
    /// Adjustment amount. Brightness -150..150, contrast -50..100, saturation -100..100; 0 changes nothing.
    pub amount: f64,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlurKind {
    #[default]
    Gaussian,
    Box,
    Motion,
    Surface,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlurParams {
    /// gaussian (radius 0.1..1000), box (radius 1..2000), motion (angle, distance 1..2000) or
    /// surface (radius 1..100, threshold 2..255). Default gaussian.
    #[serde(default)]
    pub kind: BlurKind,
    /// Radius in pixels (gaussian, box, surface).
    #[serde(default)]
    pub radius: Option<f64>,
    /// Motion blur direction in degrees (-360..360).
    #[serde(default)]
    pub angle: Option<f64>,
    /// Motion blur length in pixels (1..2000).
    #[serde(default)]
    pub distance: Option<f64>,
    /// Surface blur threshold in levels (2..255).
    #[serde(default)]
    pub threshold: Option<f64>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SharpenKind {
    #[default]
    UnsharpMask,
    Smart,
    Basic,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SharpenParams {
    /// unsharp_mask (amount 1..500, radius 0.1..1000, threshold 0..255), smart (amount 1..500,
    /// radius 0.1..64, reduce_noise 0..100) or basic (no settings). Default unsharp_mask.
    #[serde(default)]
    pub kind: SharpenKind,
    /// Strength in percent.
    #[serde(default)]
    pub amount: Option<f64>,
    /// Radius in pixels.
    #[serde(default)]
    pub radius: Option<f64>,
    /// unsharp_mask: minimum level difference to sharpen (0..255).
    #[serde(default)]
    pub threshold: Option<f64>,
    /// smart: noise reduction in percent (0..100).
    #[serde(default)]
    pub reduce_noise: Option<f64>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveBackgroundParams {
    /// Refine the edge (hair, soft edges). Default true.
    #[serde(default = "default_true")]
    pub refine: bool,
    /// Apply the mask so the background pixels are deleted (default true). False keeps an
    /// editable layer mask instead.
    #[serde(default = "default_true")]
    pub apply_mask: bool,
    /// Crop the canvas to the subject afterwards.
    #[serde(default)]
    pub trim: bool,
    /// Put a solid colour (#rrggbb) behind the subject instead of transparency.
    #[serde(default)]
    pub background_color: Option<String>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

impl Default for RemoveBackgroundParams {
    fn default() -> Self {
        Self { refine: true, apply_mask: true, trim: false, background_color: None, index: None }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextParams {
    /// The text. `\n` starts a new line.
    pub text: String,
    /// Anchor x in pixels (left end of the baseline; its centre with align=center).
    pub x: f64,
    /// Baseline y in pixels from the top.
    pub y: f64,
    /// Font size in points (= pixels at 72 ppi). Default 12.
    #[serde(default)]
    pub size: Option<f64>,
    /// Font family, for example "Inter" or "Arial". Missing fonts are reported as warnings.
    #[serde(default)]
    pub font: Option<String>,
    /// Text colour as #rrggbb (default: the foreground colour, black).
    #[serde(default)]
    pub color: Option<String>,
    /// Horizontal alignment around `x`.
    #[serde(default)]
    pub align: Option<TextAlign>,
    /// Layer name (default: the text).
    #[serde(default)]
    pub name: Option<String>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayerAction {
    List,
    New,
    Duplicate,
    Delete,
    Rename,
    SetProps,
    Select,
    BringForward,
    SendBackward,
    BringToFront,
    SendToBack,
    MergeDown,
    Flatten,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayersParams {
    /// What to do. list, new (name, opacity, blend), duplicate (layer, name), delete (layer),
    /// rename (layer, name), set_props (layer, visible, opacity, blend, name), select (layer),
    /// bring_forward / send_backward / bring_to_front / send_to_back (layer), merge_down (layer),
    /// flatten.
    pub action: LayerAction,
    /// Target layer: its numeric id (from `list`) or its exact name. Default: the active layer.
    #[serde(default)]
    pub layer: Option<Value>,
    /// Layer name (new, duplicate, rename, set_props).
    #[serde(default)]
    pub name: Option<String>,
    /// Visibility (set_props).
    #[serde(default)]
    pub visible: Option<bool>,
    /// Opacity in percent 0..100 (new, set_props).
    #[serde(default)]
    pub opacity: Option<f64>,
    /// Blend mode, for example Normal, Multiply, Screen, Overlay (new, set_props).
    #[serde(default)]
    pub blend: Option<String>,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenImageParams {
    /// Path beneath the configured read root: relative (forward slashes) or absolute.
    pub path: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectImageParams {
    /// A file beneath the read root to inspect without changing the session (it is opened,
    /// summarised and closed). Omit to inspect an open document.
    #[serde(default)]
    pub path: Option<String>,
    /// Document index from `session_list` (default: the active document). Not with `path`.
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveImageParams {
    /// Target beneath the write root. The extension picks the format: .pcraft (native,
    /// layered), .psd/.psb (layered), .png, .jpg, .webp, .tif, …
    pub path: String,
    /// JPEG/WebP quality 1..100.
    #[serde(default)]
    pub quality: Option<u8>,
    /// Replace an existing image file at `path`. Default false: an existing file is refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportImageParams {
    /// Target beneath the write root.
    pub path: String,
    /// Format as an extension (png, jpg, webp, tif, gif, bmp, tga, exr, psd, pcraft, …).
    /// Default: from the extension of `path`.
    #[serde(default)]
    pub format: Option<String>,
    /// JPEG/WebP quality 1..100.
    #[serde(default)]
    pub quality: Option<u8>,
    /// Replace an existing image file at `path`. Default false: an existing file is refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Document index from `session_list` (default: the active document).
    #[serde(default)]
    pub index: Option<usize>,
}

/// One step of `batch_process`: the name of a task tool plus that tool's own arguments
/// (without `index`).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchOp {
    ResizeImage(ResizeParams),
    CropImage(CropParams),
    RotateImage(RotateParams),
    FlipImage(FlipParams),
    AdjustBrightness(AmountParams),
    AdjustContrast(AmountParams),
    AdjustSaturation(AmountParams),
    ApplyBlur(BlurParams),
    ApplySharpen(SharpenParams),
    RemoveBackground(RemoveBackgroundParams),
    AddText(TextParams),
}

impl BatchOp {
    fn index(&self) -> Option<usize> {
        match self {
            BatchOp::ResizeImage(p) => p.index,
            BatchOp::CropImage(p) => p.index,
            BatchOp::RotateImage(p) => p.index,
            BatchOp::FlipImage(p) => p.index,
            BatchOp::AdjustBrightness(p) | BatchOp::AdjustContrast(p) | BatchOp::AdjustSaturation(p) => p.index,
            BatchOp::ApplyBlur(p) => p.index,
            BatchOp::ApplySharpen(p) => p.index,
            BatchOp::RemoveBackground(p) => p.index,
            BatchOp::AddText(p) => p.index,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            BatchOp::ResizeImage(_) => "resize_image",
            BatchOp::CropImage(_) => "crop_image",
            BatchOp::RotateImage(_) => "rotate_image",
            BatchOp::FlipImage(_) => "flip_image",
            BatchOp::AdjustBrightness(_) => "adjust_brightness",
            BatchOp::AdjustContrast(_) => "adjust_contrast",
            BatchOp::AdjustSaturation(_) => "adjust_saturation",
            BatchOp::ApplyBlur(_) => "apply_blur",
            BatchOp::ApplySharpen(_) => "apply_sharpen",
            BatchOp::RemoveBackground(_) => "remove_background",
            BatchOp::AddText(_) => "add_text",
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchProcessParams {
    /// Folder beneath the read root whose files are processed (not recursive).
    pub input_dir: String,
    /// Folder beneath the write root that receives `<name><suffix>.<ext>` (created if missing).
    pub output_dir: String,
    /// Operations applied to every file in order, each `{"tool": "<task tool>", …its arguments}`,
    /// for example `{"tool":"resize_image","width":1080}`. Tools: resize_image, crop_image,
    /// rotate_image, flip_image, adjust_brightness, adjust_contrast, adjust_saturation,
    /// apply_blur, apply_sharpen, remove_background, add_text. May be empty (pure conversion).
    #[serde(default)]
    pub operations: Vec<BatchOp>,
    /// Output format as an extension (png, jpg, webp, …). Default: each input's own format.
    #[serde(default)]
    pub format: Option<String>,
    /// JPEG/WebP quality 1..100.
    #[serde(default)]
    pub quality: Option<u8>,
    /// Text appended to each output name before the extension, for example "_small".
    #[serde(default)]
    pub suffix: Option<String>,
    /// Only files with these extensions (default: every format PhotoCraft opens).
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    /// Most files to process (default 100, at most 500).
    #[serde(default)]
    pub max_files: Option<usize>,
    /// Replace existing output files. Default false: such files are skipped.
    #[serde(default)]
    pub overwrite: bool,
    /// Stop at the first failing file. Default false: report it and go on.
    #[serde(default)]
    pub stop_on_error: bool,
}

// ---------------------------------------------------------------------------
// Planning: arguments -> engine commands (pure, unit tested)
// ---------------------------------------------------------------------------

pub mod plan {
    use super::{
        AmountParams, BlurKind, BlurParams, CropParams, FlipDirection, FlipParams, Interpolation, MAX_TASK_PIXELS, MAX_TEXT_CHARS, ResizeParams, RotateParams,
        SharpenKind, SharpenParams, TextAlign, TextParams, Value, json,
    };

    /// One engine command.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Step {
        pub id: &'static str,
        pub params: Value,
    }

    fn step(id: &'static str, params: Value) -> Step {
        Step { id, params }
    }

    fn finite(name: &str, v: f64) -> Result<f64, String> {
        if v.is_finite() { Ok(v) } else { Err(format!("`{name}` must be a finite number")) }
    }

    fn in_range(name: &str, v: f64, lo: f64, hi: f64) -> Result<f64, String> {
        let v = finite(name, v)?;
        if (lo..=hi).contains(&v) { Ok(v) } else { Err(format!("`{name}` must be between {lo} and {hi}, got {v}")) }
    }

    fn unused(kind: &str, fields: &[(&str, bool)]) -> Result<(), String> {
        let given: Vec<&str> = fields.iter().filter(|(_, set)| *set).map(|(n, _)| *n).collect();
        if given.is_empty() { Ok(()) } else { Err(format!("`{}` does not apply to {kind}", given.join("`, `"))) }
    }

    fn side(name: &str, v: f64) -> Result<u32, String> {
        let v = finite(name, v)?.round();
        if v < 1.0 {
            return Err(format!("the resulting {name} would be less than 1 pixel"));
        }
        if v > f64::from(u32::MAX) {
            return Err(format!("the resulting {name} is too large"));
        }
        // In range and integral after the checks above.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Ok(v as u32)
    }

    /// `image.imageSize` for the requested size, given the current canvas `cur` (w, h).
    pub fn resize(p: &ResizeParams, cur: (u32, u32)) -> Result<Step, String> {
        let (cw, ch) = (f64::from(cur.0.max(1)), f64::from(cur.1.max(1)));
        let (w, h) = match (p.percent, p.width, p.height) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => return Err("give either `percent` or `width`/`height`, not both".into()),
            (Some(pc), None, None) => {
                if p.fit || p.enlarge {
                    return Err("`fit`/`enlarge` go with `width` and `height`, not `percent`".into());
                }
                let pc = in_range("percent", pc, 0.01, 10_000.0)?;
                (cw * pc / 100.0, ch * pc / 100.0)
            }
            (None, Some(w), Some(h)) if p.fit => {
                if w == 0 || h == 0 {
                    return Err("`width` and `height` must be at least 1".into());
                }
                let mut k = (f64::from(w) / cw).min(f64::from(h) / ch);
                if !p.enlarge {
                    k = k.min(1.0);
                }
                (cw * k, ch * k)
            }
            (None, _, _) if p.fit => return Err("`fit` needs both `width` and `height`".into()),
            (None, Some(w), Some(h)) => (f64::from(w), f64::from(h)),
            (None, Some(w), None) => (f64::from(w), ch * f64::from(w) / cw),
            (None, None, Some(h)) => (cw * f64::from(h) / ch, f64::from(h)),
            (None, None, None) => return Err("resize_image needs `width`, `height` or `percent`".into()),
        };
        if p.enlarge && !p.fit {
            return Err("`enlarge` only applies with `fit`".into());
        }
        let (w, h) = (side("width", w)?, side("height", h)?);
        if u64::from(w) * u64::from(h) > MAX_TASK_PIXELS {
            return Err(format!("{w} x {h} exceeds the {} megapixel limit of resize_image", MAX_TASK_PIXELS >> 20));
        }
        let mut params = json!({"width": w, "height": h});
        if let Some(r) = p.resample {
            params["resample"] = json!(r.engine());
        }
        Ok(step("image.imageSize", params))
    }

    pub fn crop(p: &CropParams, cur: (u32, u32)) -> Result<Step, String> {
        if p.width == 0 || p.height == 0 {
            return Err("`width` and `height` must be at least 1".into());
        }
        let (x0, y0) = (i64::from(p.x), i64::from(p.y));
        let (x1, y1) = (x0 + i64::from(p.width), y0 + i64::from(p.height));
        if x1 <= 0 || y1 <= 0 || x0 >= i64::from(cur.0) || y0 >= i64::from(cur.1) {
            return Err(format!("the crop rectangle ({}, {}, {} x {}) lies outside the {} x {} canvas", p.x, p.y, p.width, p.height, cur.0, cur.1));
        }
        if u64::from(p.width) * u64::from(p.height) > MAX_TASK_PIXELS {
            return Err(format!("{} x {} exceeds the {} megapixel limit of crop_image", p.width, p.height, MAX_TASK_PIXELS >> 20));
        }
        Ok(step("image.crop", json!({"x": p.x, "y": p.y, "width": p.width, "height": p.height})))
    }

    pub fn rotate(p: &RotateParams) -> Result<Step, String> {
        let a = finite("angle", p.angle)?;
        if a.abs() >= 3600.0 {
            return Err("`angle` must be within ±3600 degrees".into());
        }
        // Normalise to (-180, 180].
        let mut n = a % 360.0;
        if n > 180.0 {
            n -= 360.0;
        } else if n <= -180.0 {
            n += 360.0;
        }
        let quarter = |v: f64| (n - v).abs() < 1e-9;
        let lossless = if quarter(90.0) {
            Some("image.imageRotation.90cw")
        } else if quarter(-90.0) {
            Some("image.imageRotation.90ccw")
        } else if quarter(180.0) {
            Some("image.imageRotation.180")
        } else {
            None
        };
        if let Some(id) = lossless {
            if p.interpolation.is_some() {
                return Err("`interpolation` does not apply to multiples of 90 degrees (they are lossless)".into());
            }
            return Ok(step(id, json!({})));
        }
        if n.abs() < 0.01 {
            return Err("an angle of 0 (or a multiple of 360) does not rotate the image".into());
        }
        let mut params = json!({"angle": n.abs(), "direction": if n > 0.0 { "cw" } else { "ccw" }});
        if let Some(i) = p.interpolation {
            params["interpolation"] = json!(match i {
                Interpolation::Bicubic => "bicubic",
                Interpolation::Bilinear => "bilinear",
                Interpolation::Nearest => "nearest",
            });
        }
        Ok(step("image.rotation.arbitrary", params))
    }

    pub fn flip(p: &FlipParams) -> Step {
        match p.direction {
            FlipDirection::Horizontal => step("image.imageRotation.flipCanvasHorizontal", json!({})),
            FlipDirection::Vertical => step("image.imageRotation.flipCanvasVertical", json!({})),
        }
    }

    pub fn brightness(p: &AmountParams) -> Result<Step, String> {
        let v = in_range("amount", p.amount, -150.0, 150.0)?;
        Ok(step("image.adjustments.brightnessContrast", json!({"brightness": v, "contrast": 0})))
    }

    pub fn contrast(p: &AmountParams) -> Result<Step, String> {
        let v = in_range("amount", p.amount, -50.0, 100.0)?;
        Ok(step("image.adjustments.brightnessContrast", json!({"brightness": 0, "contrast": v})))
    }

    pub fn saturation(p: &AmountParams) -> Result<Step, String> {
        let v = in_range("amount", p.amount, -100.0, 100.0)?;
        Ok(step("image.adjustments.hueSaturation", json!({"saturation": v})))
    }

    pub fn blur(p: &BlurParams) -> Result<Step, String> {
        match p.kind {
            BlurKind::Gaussian => {
                unused("a gaussian blur", &[("angle", p.angle.is_some()), ("distance", p.distance.is_some()), ("threshold", p.threshold.is_some())])?;
                let r = in_range("radius", p.radius.unwrap_or(2.0), 0.1, 1000.0)?;
                Ok(step("filter.blur.gaussianBlur", json!({"radius": r})))
            }
            BlurKind::Box => {
                unused("a box blur", &[("angle", p.angle.is_some()), ("distance", p.distance.is_some()), ("threshold", p.threshold.is_some())])?;
                let r = in_range("radius", p.radius.unwrap_or(2.0), 1.0, 2000.0)?;
                Ok(step("filter.blur.boxBlur", json!({"radius": r})))
            }
            BlurKind::Motion => {
                unused("a motion blur", &[("radius", p.radius.is_some()), ("threshold", p.threshold.is_some())])?;
                let a = in_range("angle", p.angle.unwrap_or(0.0), -360.0, 360.0)?;
                let d = in_range("distance", p.distance.unwrap_or(10.0), 1.0, 2000.0)?;
                Ok(step("filter.blur.motionBlur", json!({"angle": a, "distance": d})))
            }
            BlurKind::Surface => {
                unused("a surface blur", &[("angle", p.angle.is_some()), ("distance", p.distance.is_some())])?;
                let r = in_range("radius", p.radius.unwrap_or(5.0), 1.0, 100.0)?;
                let t = in_range("threshold", p.threshold.unwrap_or(15.0), 2.0, 255.0)?;
                Ok(step("filter.blur.surfaceBlur", json!({"radius": r, "threshold": t})))
            }
        }
    }

    pub fn sharpen(p: &SharpenParams) -> Result<Step, String> {
        match p.kind {
            SharpenKind::UnsharpMask => {
                unused("unsharp_mask", &[("reduce_noise", p.reduce_noise.is_some())])?;
                let a = in_range("amount", p.amount.unwrap_or(50.0), 1.0, 500.0)?;
                let r = in_range("radius", p.radius.unwrap_or(1.0), 0.1, 1000.0)?;
                let t = in_range("threshold", p.threshold.unwrap_or(0.0), 0.0, 255.0)?;
                Ok(step("filter.sharpen.unsharpMask", json!({"amount": a, "radius": r, "threshold": t})))
            }
            SharpenKind::Smart => {
                unused("smart sharpen", &[("threshold", p.threshold.is_some())])?;
                let a = in_range("amount", p.amount.unwrap_or(100.0), 1.0, 500.0)?;
                let r = in_range("radius", p.radius.unwrap_or(1.0), 0.1, 64.0)?;
                let n = in_range("reduce_noise", p.reduce_noise.unwrap_or(10.0), 0.0, 100.0)?;
                Ok(step("filter.sharpen.smartSharpen", json!({"amount": a, "radius": r, "reduceNoise": n})))
            }
            SharpenKind::Basic => {
                unused(
                    "basic sharpen",
                    &[
                        ("amount", p.amount.is_some()),
                        ("radius", p.radius.is_some()),
                        ("threshold", p.threshold.is_some()),
                        ("reduce_noise", p.reduce_noise.is_some()),
                    ],
                )?;
                Ok(step("filter.sharpen.sharpen", json!({})))
            }
        }
    }

    /// Whether `c` is a `#rrggbb` colour.
    pub fn is_hex_color(c: &str) -> bool {
        c.len() == 7 && c.starts_with('#') && c.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
    }

    pub fn text(p: &TextParams) -> Result<Step, String> {
        if p.text.trim().is_empty() {
            return Err("`text` is empty".into());
        }
        if p.text.chars().count() > MAX_TEXT_CHARS {
            return Err(format!("`text` is longer than {MAX_TEXT_CHARS} characters"));
        }
        let x = in_range("x", p.x, -1e6, 1e6)?;
        let y = in_range("y", p.y, -1e6, 1e6)?;
        let mut params = json!({"x": x, "y": y, "text": p.text});
        if let Some(s) = p.size {
            params["size"] = json!(in_range("size", s, 0.1, 5000.0)?);
        }
        if let Some(f) = p.font.as_deref().filter(|f| !f.trim().is_empty()) {
            params["font"] = json!(f);
        }
        if let Some(c) = &p.color {
            if !is_hex_color(c) {
                return Err(format!("`color` must be #rrggbb, got {c:?}"));
            }
            params["color"] = json!(c);
        }
        if let Some(a) = p.align {
            params["align"] = json!(match a {
                TextAlign::Left => "left",
                TextAlign::Center => "center",
                TextAlign::Right => "right",
            });
        }
        if let Some(n) = &p.name {
            params["name"] = json!(n);
        }
        Ok(step("type.create", params))
    }

    /// The output file name of `batch_process` for `input`.
    pub fn batch_output_name(input: &str, suffix: &str, format: Option<&str>) -> Result<String, String> {
        let (stem, ext) = match input.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem, ext),
            _ => (input, ""),
        };
        let ext = format.map(|f| f.trim_start_matches('.')).unwrap_or(ext);
        if ext.is_empty() {
            return Err(format!("`{input}` has no extension; pass `format`"));
        }
        if suffix.contains(['/', '\\']) || suffix.contains("..") {
            return Err("`suffix` must not contain path separators or `..`".into());
        }
        Ok(format!("{stem}{suffix}.{}", ext.to_ascii_lowercase()))
    }

    /// Whether a file name has an extension PhotoCraft opens (or one of `only`).
    pub fn batch_wants(name: &str, only: Option<&[String]>) -> bool {
        if name.starts_with('.') {
            return false;
        }
        let Some((_, ext)) = name.rsplit_once('.') else { return false };
        let ext = ext.to_ascii_lowercase();
        match only {
            Some(list) => list.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext)),
            None => {
                ["psd", "psb", "pcraft", "ora", "kra", "xcf", "pdn", "afphoto", "afdesign", "svg"].contains(&ext.as_str())
                    || photocraft_codecs::from_extension(&ext).is_some()
            }
        }
    }

    fn be16(b: &[u8], at: usize) -> Option<u32> {
        let s = b.get(at..at.checked_add(2)?)?;
        Some(u32::from(u16::from_be_bytes([s[0], s[1]])))
    }

    fn be32(b: &[u8], at: usize) -> Option<u32> {
        let s = b.get(at..at.checked_add(4)?)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    /// The pixel size an image file declares in its header, read from its first bytes without
    /// decoding: PNG, JPEG and PSD/PSB here, other formats through
    /// [`photocraft_codecs::probe_size`]. `None` when the header does not say (or the format has
    /// no cheap probe); the decoder's own limits still apply to those.
    pub fn header_size(head: &[u8]) -> Option<(u32, u32)> {
        if head.starts_with(b"\x89PNG\r\n\x1a\n") && head.get(12..16) == Some(b"IHDR".as_slice()) {
            return Some((be32(head, 16)?, be32(head, 20)?));
        }
        if head.starts_with(b"8BPS") {
            // Version 1 (PSD) or 2 (PSB): height then width, 32-bit big-endian.
            return Some((be32(head, 18)?, be32(head, 14)?));
        }
        if head.starts_with(&[0xFF, 0xD8]) {
            return jpeg_size(head);
        }
        photocraft_codecs::probe_size(head).ok()
    }

    /// Walk JPEG markers to the first start-of-frame segment.
    fn jpeg_size(b: &[u8]) -> Option<(u32, u32)> {
        let mut at = 2usize;
        // A JPEG has a few dozen segments before its frame header; bound the walk anyway.
        for _ in 0..4096 {
            if *b.get(at)? != 0xFF {
                return None;
            }
            let marker = *b.get(at + 1)?;
            match marker {
                0xFF => at += 1, // fill byte
                0x01 | 0xD0..=0xD7 => at += 2,
                0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                    let (h, w) = (be16(b, at + 5)?, be16(b, at + 7)?);
                    return (h > 0 && w > 0).then_some((w, h));
                }
                0xD9 | 0xDA => return None, // end of image / start of scan before any frame
                _ => at = at.checked_add(2)?.checked_add(usize::try_from(be16(b, at + 2)?).ok()?)?,
            }
        }
        None
    }

    /// Refuse a declared size over the task tools' pixel budget.
    pub fn check_declared_size(name: &str, size: Option<(u32, u32)>) -> Result<(), String> {
        match size {
            Some((w, h)) if u64::from(w) * u64::from(h) > MAX_TASK_PIXELS => {
                Err(format!("`{name}` declares {w} x {h} pixels, over the {} megapixel limit of the task tools; it was not decoded", MAX_TASK_PIXELS >> 20))
            }
            _ => Ok(()),
        }
    }

    /// Join a request directory and a file name with a forward slash.
    pub fn join(dir: &str, name: &str) -> String {
        let dir = dir.trim_end_matches(['/', '\\']);
        if dir.is_empty() || dir == "." { name.to_owned() } else { format!("{dir}/{name}") }
    }
}

// ---------------------------------------------------------------------------
// Execution on the headless session
// ---------------------------------------------------------------------------

fn bad(msg: impl Into<String>) -> AutomationError {
    AutomationError::BadRequest(msg.into())
}

fn active_size(h: &Headless) -> Result<(u32, u32), AutomationError> {
    let d = h.session.active().ok_or_else(|| bad("no document open: open one with open_image or doc_new"))?;
    Ok((d.doc.size.width, d.doc.size.height))
}

/// How much of a file is read to find its declared pixel size.
const HEADER_BYTES: usize = 1 << 20;

/// Refuse a file whose header declares more pixels than the task tools allow, before the file is
/// read in full or decoded. Files whose header gives no size are left to the decoder's limits.
fn check_image_header(h: &Headless, path: &str) -> Result<(), AutomationError> {
    let Some(ws) = h.workspace() else { return Ok(()) };
    let head = ws.read_head(path, HEADER_BYTES)?;
    plan::check_declared_size(path, plan::header_size(&head)).map_err(bad)
}

fn run(h: &mut Headless, s: plan::Step) -> Result<Value, AutomationError> {
    tracing::debug!(command = s.id, params = %s.params, "task tool step");
    let id = s.id;
    h.command_run(id, s.params).map_err(|e| {
        // Adjustments and filters edit the active layer's pixels: say how to get there.
        let msg = e.to_string();
        if (id.starts_with("filter.") || id.starts_with("image.adjustments.")) && msg.contains("pixel layer") {
            bad(format!(
                "{msg}. Select a pixel layer first (manage_layers action=select, layer=<id or name>), or flatten the image (manage_layers action=flatten)"
            ))
        } else {
            e
        }
    })
}

/// The compact layer tree of an inspect reply, bounded in depth.
fn layer_rows(layers: Option<&Value>, depth: usize) -> Vec<Value> {
    let Some(rows) = layers.and_then(Value::as_array) else { return Vec::new() };
    rows.iter()
        .map(|l| {
            let mut row = json!({
                "id": l.get("id"), "name": l.get("name"), "kind": l.get("kind"), "visible": l.get("visible"),
                "opacity": l.get("opacity"), "blend": l.get("blend"), "bounds": l.get("bounds"),
            });
            if depth < 16 && l.get("children").is_some() {
                row["children"] = Value::Array(layer_rows(l.get("children"), depth + 1));
            }
            row
        })
        .collect()
}

/// Document summary for a task tool reply (the active document unless `index`).
fn summary(h: &Headless, index: Option<usize>, with_layers: bool) -> Result<Value, AutomationError> {
    let full = h.inspect(index)?;
    let i = match index {
        Some(i) => Some(i),
        None => h.session.active_index(),
    };
    let mut s = json!({"index": i});
    for k in ["name", "width", "height", "mode", "depth", "resolution", "activeLayer", "hasSelection"] {
        if let Some(v) = full.get(k) {
            s[k] = v.clone();
        }
    }
    let layers = layer_rows(full.get("layers"), 0);
    s["layerCount"] = json!(layers.len());
    if with_layers {
        s["layers"] = Value::Array(layers);
    }
    Ok(s)
}

/// Find a layer id by numeric id or exact name, walking the inspect tree.
fn resolve_layer(h: &Headless, layer: &Value) -> Result<u64, AutomationError> {
    if let Some(id) = layer.as_u64() {
        return Ok(id);
    }
    let Some(name) = layer.as_str() else { return Err(bad("`layer` must be a layer id (number) or a layer name (string)")) };
    let full = h.inspect(None)?;
    let mut found = Vec::new();
    fn walk(rows: Option<&Value>, name: &str, found: &mut Vec<u64>, depth: usize) {
        let Some(rows) = rows.and_then(Value::as_array) else { return };
        for l in rows {
            if l.get("name").and_then(Value::as_str) == Some(name)
                && let Some(id) = l.get("id").and_then(Value::as_u64)
            {
                found.push(id);
            }
            if depth < 64 {
                walk(l.get("children"), name, found, depth + 1);
            }
        }
    }
    walk(full.get("layers"), name, &mut found, 0);
    match found.as_slice() {
        [id] => Ok(*id),
        [] => Err(bad(format!("no layer named {name:?}; use manage_layers action=list"))),
        ids => Err(bad(format!("{} layers are named {name:?}; pass the id instead ({ids:?})", ids.len()))),
    }
}

/// Apply one task operation to the active document.
fn apply(h: &mut Headless, op: &BatchOp) -> Result<Value, AutomationError> {
    match op {
        BatchOp::ResizeImage(p) => {
            let s = plan::resize(p, active_size(h)?).map_err(bad)?;
            run(h, s)
        }
        BatchOp::CropImage(p) => {
            let s = plan::crop(p, active_size(h)?).map_err(bad)?;
            run(h, s)
        }
        BatchOp::RotateImage(p) => run(h, plan::rotate(p).map_err(bad)?),
        BatchOp::FlipImage(p) => run(h, plan::flip(p)),
        BatchOp::AdjustBrightness(p) => run(h, plan::brightness(p).map_err(bad)?),
        BatchOp::AdjustContrast(p) => run(h, plan::contrast(p).map_err(bad)?),
        BatchOp::AdjustSaturation(p) => run(h, plan::saturation(p).map_err(bad)?),
        BatchOp::ApplyBlur(p) => run(h, plan::blur(p).map_err(bad)?),
        BatchOp::ApplySharpen(p) => run(h, plan::sharpen(p).map_err(bad)?),
        BatchOp::RemoveBackground(p) => remove_background(h, p),
        BatchOp::AddText(p) => run(h, plan::text(p).map_err(bad)?),
    }
}

fn remove_background(h: &mut Headless, p: &RemoveBackgroundParams) -> Result<Value, AutomationError> {
    if let Some(c) = &p.background_color
        && !plan::is_hex_color(c)
    {
        return Err(bad(format!("`background_color` must be #rrggbb, got {c:?}")));
    }
    let removed = h.command_run("layer.removeBackground", json!({"refine": p.refine}))?;
    let layer = removed.get("layer").cloned().unwrap_or(Value::Null);
    let mut steps = vec![json!({"command": "layer.removeBackground", "result": removed})];
    if p.apply_mask {
        let r = h.command_run("layer.layerMask.apply", json!({"layer": layer}))?;
        steps.push(json!({"command": "layer.layerMask.apply", "result": r}));
    }
    // A selection left behind would become the fill layer's mask.
    if h.session.active().is_some_and(|d| d.doc.selection.is_some()) {
        h.command_run("select.deselect", json!({}))?;
    }
    if p.trim {
        let r = h.command_run("image.trim", json!({"basedOn": "transparent"}))?;
        steps.push(json!({"command": "image.trim", "result": r}));
    }
    if let Some(c) = &p.background_color {
        let fill = h.command_run("layer.newFillLayer.solidColor", json!({"color": c}))?;
        let fill_layer = match fill.get("layer").and_then(Value::as_u64) {
            Some(id) => json!(id),
            None => h.inspect(None)?.get("activeLayer").cloned().unwrap_or(Value::Null),
        };
        let r = h.command_run("layer.arrange.sendToBack", json!({"layer": fill_layer}))?;
        steps.push(json!({"command": "layer.newFillLayer.solidColor", "result": fill}));
        steps.push(json!({"command": "layer.arrange.sendToBack", "result": r}));
    }
    Ok(json!({"subjectLayer": layer, "bounds": removed.get("bounds"), "steps": steps}))
}

fn manage_layers(h: &mut Headless, p: &LayersParams) -> Result<Value, AutomationError> {
    let opacity = match p.opacity {
        Some(o) => Some(plan_opacity(o)?),
        None => None,
    };
    let target = match &p.layer {
        Some(l) => Some(resolve_layer(h, l)?),
        None => None,
    };
    let with_layer = |mut v: Value| {
        if let Some(id) = target {
            v["layer"] = json!(id);
        }
        v
    };
    let need_target = |what: &str| target.ok_or_else(|| bad(format!("`{what}` needs `layer` (an id or a name)")));
    let result = match p.action {
        LayerAction::List => Value::Null,
        LayerAction::New => {
            let mut v = json!({});
            if let Some(n) = &p.name {
                v["name"] = json!(n);
            }
            if let Some(o) = opacity {
                v["opacity"] = json!(o);
            }
            if let Some(b) = &p.blend {
                v["mode"] = json!(b);
            }
            h.command_run("layer.new.layer", v)?
        }
        LayerAction::Duplicate => {
            let mut v = with_layer(json!({}));
            if let Some(n) = &p.name {
                v["name"] = json!(n);
            }
            h.command_run("layer.duplicate", v)?
        }
        LayerAction::Delete => h.command_run("layer.delete", with_layer(json!({})))?,
        LayerAction::Rename => {
            let name = p.name.as_deref().filter(|n| !n.is_empty()).ok_or_else(|| bad("`rename` needs `name`"))?;
            h.command_run("layer.setProps", with_layer(json!({"name": name})))?
        }
        LayerAction::SetProps => {
            let mut v = with_layer(json!({}));
            if let Some(n) = &p.name {
                v["name"] = json!(n);
            }
            if let Some(vis) = p.visible {
                v["visible"] = json!(vis);
            }
            if let Some(o) = opacity {
                v["opacity"] = json!(o);
            }
            if let Some(b) = &p.blend {
                v["blend"] = json!(b);
            }
            if v.as_object().is_some_and(|m| m.keys().all(|k| k == "layer")) {
                return Err(bad("`set_props` needs at least one of `name`, `visible`, `opacity`, `blend`"));
            }
            h.command_run("layer.setProps", v)?
        }
        LayerAction::Select => h.command_run("layer.select", json!({"layer": need_target("select")?}))?,
        LayerAction::BringForward => h.command_run("layer.arrange.bringForward", with_layer(json!({})))?,
        LayerAction::SendBackward => h.command_run("layer.arrange.sendBackward", with_layer(json!({})))?,
        LayerAction::BringToFront => h.command_run("layer.arrange.bringToFront", with_layer(json!({})))?,
        LayerAction::SendToBack => h.command_run("layer.arrange.sendToBack", with_layer(json!({})))?,
        LayerAction::MergeDown => h.command_run("layer.mergeDown", with_layer(json!({})))?,
        LayerAction::Flatten => {
            if target.is_some() {
                return Err(bad("`flatten` merges every layer and takes no `layer`"));
            }
            h.command_run("layer.flattenImage", json!({}))?
        }
    };
    Ok(json!({"result": result, "document": summary(h, None, true)?}))
}

fn plan_opacity(o: f64) -> Result<f64, AutomationError> {
    if !o.is_finite() || !(0.0..=100.0).contains(&o) {
        return Err(bad(format!("`opacity` must be a percentage from 0 to 100, got {o}")));
    }
    Ok(o / 100.0)
}

fn export_options(quality: Option<u8>) -> Result<photocraft_io::ExportOptions, AutomationError> {
    let mut opts = photocraft_io::ExportOptions::default();
    if let Some(q) = quality {
        if !(1..=100).contains(&q) {
            return Err(bad(format!("`quality` must be a whole number from 1 to 100, got {q}")));
        }
        opts.encode.jpeg_quality = q;
        opts.encode.webp_quality = q;
        opts.encode.webp_lossless = false;
    }
    Ok(opts)
}

/// Refuse to replace an existing file unless `overwrite`.
fn check_target(h: &Headless, path: &str, overwrite: bool) -> Result<(), AutomationError> {
    if path.trim().is_empty() {
        return Err(bad("`path` is empty"));
    }
    let Some(ws) = h.workspace() else {
        return Err(bad("automation filesystem access is not granted: start the server with --automation-write-root"));
    };
    if !overwrite && ws.write_target_exists(path)? {
        return Err(bad(format!("`{path}` already exists; pass `overwrite: true` to replace it, or choose another name")));
    }
    // `out/photo.png` into a folder that does not exist yet: create it below the write root
    // (validated like the file path itself, so it cannot leave the root).
    if let Some(parent) = Path::new(path).parent().and_then(Path::to_str).filter(|p| !p.is_empty()) {
        ws.create_write_dir(parent)?;
    }
    Ok(())
}

fn save(h: &mut Headless, index: Option<usize>, path: &str, format: Option<&str>, quality: Option<u8>, overwrite: bool) -> Result<Value, AutomationError> {
    check_target(h, path, overwrite)?;
    let opts = export_options(quality)?;
    let saved = h.save(index, Some(Path::new(path)), format, &opts)?;
    Ok(json!({"path": saved.get("path"), "warnings": saved.get("warnings"), "document": summary(h, index, false)?}))
}

fn inspect_file(h: &mut Headless, path: &str) -> Result<Value, AutomationError> {
    let previous = h.session.active_index();
    check_image_header(h, path)?;
    let opened = h.open(Path::new(path))?;
    let index = opened.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("open returned no index"))?;
    let result = summary(h, Some(index), true);
    // Leave the session as it was: close the copy, then restore the active document.
    let _ = h.close(Some(index));
    if let Some(p) = previous {
        h.session.set_active(p);
    }
    let mut s = result?;
    s["index"] = Value::Null;
    s["file"] = json!(path);
    s["warnings"] = opened.get("warnings").cloned().unwrap_or(json!([]));
    Ok(s)
}

fn batch(h: &mut Headless, p: &BatchProcessParams) -> Result<Value, AutomationError> {
    if p.operations.len() > MAX_BATCH_OPERATIONS {
        return Err(bad(format!("at most {MAX_BATCH_OPERATIONS} operations per batch, got {}", p.operations.len())));
    }
    if let Some(op) = p.operations.iter().find(|op| op.index().is_some()) {
        return Err(bad(format!("`index` is not used inside batch_process operations ({})", op.name())));
    }
    let max_files = p.max_files.unwrap_or(DEFAULT_BATCH_FILES);
    if max_files == 0 || max_files > MAX_BATCH_FILES {
        return Err(bad(format!("`max_files` must be from 1 to {MAX_BATCH_FILES}")));
    }
    if let Some(f) = &p.format
        && (f.trim_start_matches('.').is_empty() || f.contains(['/', '\\']))
    {
        return Err(bad(format!("`format` must be an extension such as png or jpg, got {f:?}")));
    }
    let opts = export_options(p.quality)?;
    let suffix = p.suffix.as_deref().unwrap_or("");
    let ws = h
        .workspace()
        .ok_or_else(|| bad("automation filesystem access is not granted: start the server with --automation-read-root and --automation-write-root"))?;
    // Listing more than `max_files` tells us whether some were left out.
    let (entries, listing_truncated) = ws.list_read_dir(&p.input_dir, MAX_BATCH_FILES.saturating_mul(4))?;
    ws.create_write_dir(&p.output_dir)?;
    let wanted: Vec<(String, u64)> = entries.iter().filter(|(name, _)| plan::batch_wants(name, p.extensions.as_deref())).cloned().collect();
    let truncated = listing_truncated || wanted.len() > max_files;

    // Plan every file first: nothing is written until all outputs are known, so an output can be
    // checked against every input before the first save.
    let mut items: Vec<BatchItem> = Vec::new();
    for (name, bytes) in wanted.into_iter().take(max_files) {
        let input = plan::join(&p.input_dir, &name);
        let item = plan_one(h, &input, &name, bytes, suffix, p);
        let stop = p.stop_on_error && matches!(item, BatchItem::Failed(_));
        items.push(item);
        if stop {
            break;
        }
    }
    check_batch_collisions(h, &entries, p, &items)?;

    let previous = h.session.active_index();
    let mut files = Vec::new();
    let (mut processed, mut failed, mut skipped) = (0usize, 0usize, 0usize);
    for item in items {
        let (input, output) = match item {
            BatchItem::Skipped(row) => {
                skipped += 1;
                files.push(row);
                continue;
            }
            BatchItem::Failed(row) => {
                failed += 1;
                files.push(row);
                continue;
            }
            BatchItem::Run { input, output } => (input, output),
        };
        let started = Instant::now();
        let outcome = process_one(h, &input, &output, p, &opts);
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match outcome {
            Ok(warnings) => {
                processed += 1;
                files.push(json!({"input": input, "output": output, "status": "ok", "ms": ms, "warnings": warnings}));
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(input = %input, error = %e, "batch_process file failed");
                files.push(json!({"input": input, "output": output, "status": "failed", "error": e.to_string()}));
                if p.stop_on_error {
                    break;
                }
            }
        }
    }
    if let Some(prev) = previous {
        h.session.set_active(prev);
    }
    Ok(json!({"processed": processed, "failed": failed, "skipped": skipped, "truncated": truncated, "files": files}))
}

/// One file of a batch after planning.
enum BatchItem {
    /// Process `input` into `output`.
    Run { input: String, output: String },
    /// Left alone; the reply row says why.
    Skipped(Value),
    /// Cannot be processed; the reply row says why.
    Failed(Value),
}

fn batch_failed(input: &str, output: Option<&str>, error: &str) -> BatchItem {
    // Every failed file is logged, whatever stage refused it.
    tracing::warn!(input = %input, error = %error, "batch_process file failed");
    let mut row = json!({"input": input, "status": "failed", "error": error});
    if let Some(o) = output {
        row["output"] = json!(o);
    }
    BatchItem::Failed(row)
}

/// Decide what happens to one batch file, without writing anything.
fn plan_one(h: &Headless, input: &str, name: &str, bytes: u64, suffix: &str, p: &BatchProcessParams) -> BatchItem {
    if bytes > MAX_BATCH_FILE_BYTES {
        return BatchItem::Skipped(json!({"input": input, "status": "skipped", "reason": format!("larger than {} MiB", MAX_BATCH_FILE_BYTES >> 20)}));
    }
    let out_name = match plan::batch_output_name(name, suffix, p.format.as_deref()) {
        Ok(n) => n,
        Err(e) => return batch_failed(input, None, &e),
    };
    let output = plan::join(&p.output_dir, &out_name);
    // A name the path rules refuse (`12:30.png`, `con.png`) fails this file, not the batch.
    let exists = match h.workspace().map(|ws| ws.write_target_exists(&output)).transpose() {
        Ok(exists) => exists.unwrap_or(false),
        Err(e) => return batch_failed(input, Some(&output), &e.to_string()),
    };
    if exists && !p.overwrite {
        return BatchItem::Skipped(
            json!({"input": input, "output": output, "status": "skipped", "reason": "output exists (pass overwrite: true to replace it)"}),
        );
    }
    BatchItem::Run { input: input.to_owned(), output }
}

/// Refuse the whole batch, before anything is written, when an output would replace a file of
/// the input folder other than its own source (which may not have been read yet), or when two
/// inputs would write the same output (`photo.jpg` and `photo.png` with `format: "webp"`).
/// Writing a file back over its own source stays possible (with `overwrite`).
fn check_batch_collisions(h: &Headless, entries: &[(String, u64)], p: &BatchProcessParams, items: &[BatchItem]) -> Result<(), AutomationError> {
    let Some(ws) = h.workspace() else { return Ok(()) };
    let input_dir = p.input_dir.as_str();
    // The listing is bounded, so in a very large folder not every input is in `entries`: when the
    // output folder is the input folder, any existing output other than its own source counts too.
    let same_dir = ws.comparable_path(input_dir, false)? == ws.comparable_path(&p.output_dir, true)?;
    let mut inputs = std::collections::HashMap::new();
    for (name, _) in entries {
        let input = plan::join(input_dir, name);
        // A name the read root refuses cannot be an input of this batch either.
        if let Ok(key) = ws.comparable_path(&input, false) {
            inputs.insert(key, input);
        }
    }
    let mut outputs: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    let mut problems = Vec::new();
    for item in items {
        let BatchItem::Run { input, output } = item else { continue };
        let own = ws.comparable_path(input, false)?;
        let key = ws.comparable_path(output, true)?;
        if key != own {
            if let Some(other) = inputs.get(&key) {
                problems.push(format!("`{input}` would write `{output}`, replacing the input `{other}`"));
            } else if same_dir && ws.write_target_exists(output)? {
                problems.push(format!("`{input}` would write `{output}`, replacing a file of the input folder"));
            }
        }
        if let Some(first) = outputs.insert(key, input.as_str()) {
            problems.push(format!("`{first}` and `{input}` would both write `{output}`"));
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    let shown: Vec<String> = problems.iter().take(5).cloned().collect();
    let more = if problems.len() > 5 { format!(" (and {} more)", problems.len() - 5) } else { String::new() };
    Err(bad(format!(
        "batch_process refused before writing anything: {}{more}. Use another output_dir, a suffix or a format that keeps the names apart",
        shown.join("; ")
    )))
}

/// Open, edit, save and close one batch file. The document is always closed, so memory holds at
/// most one batch document at a time.
fn process_one(h: &mut Headless, input: &str, output: &str, p: &BatchProcessParams, opts: &photocraft_io::ExportOptions) -> Result<Value, AutomationError> {
    check_image_header(h, input)?;
    let opened = h.open(Path::new(input))?;
    let index = opened.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("open returned no index"))?;
    let result = edit_and_save(h, index, output, p, opts);
    let _ = h.close(Some(index));
    let saved = result?;
    let mut warnings: Vec<Value> = opened.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default();
    warnings.extend(saved.get("warnings").and_then(Value::as_array).cloned().unwrap_or_default());
    Ok(Value::Array(warnings))
}

fn edit_and_save(h: &mut Headless, index: usize, output: &str, p: &BatchProcessParams, opts: &photocraft_io::ExportOptions) -> Result<Value, AutomationError> {
    for (n, op) in p.operations.iter().enumerate() {
        apply(h, op).map_err(|e| bad(format!("step {n} ({}): {e}", op.name())))?;
    }
    h.save(Some(index), Some(Path::new(output)), p.format.as_deref(), opts)
}

// ---------------------------------------------------------------------------
// Replies
// ---------------------------------------------------------------------------

fn structured_ok(tool: &str, mut v: Value) -> CallToolResult {
    if let Value::Object(m) = &mut v {
        m.insert("ok".into(), json!(true));
        m.insert("tool".into(), json!(tool));
    }
    // Measure the finished result: it carries the JSON twice (structuredContent and the text
    // block), so checking `v` alone would let a reply over half the budget through (#504).
    let result = CallToolResult::structured(v);
    match json_bytes(&result) {
        Ok(_) => result,
        Err(e) => structured_err(tool, &format!("{e}; the operation may have completed")),
    }
}

/// Longest error text a task tool sends back (characters).
const MAX_ERROR_CHARS: usize = 4096;

fn structured_err(tool: &str, msg: &str) -> CallToolResult {
    let result = CallToolResult::structured_error(json!({"ok": false, "tool": tool, "error": msg}));
    if json_bytes(&result).is_ok() {
        return result;
    }
    let short: String = msg.chars().take(MAX_ERROR_CHARS).collect();
    CallToolResult::structured_error(json!({"ok": false, "tool": tool, "error": format!("{short}… (error text truncated)")}))
}

impl PhotocraftMcp {
    /// Run a task tool on the headless session, with timing in the log and a structured reply.
    async fn task<F>(&self, tool: &'static str, f: F) -> Result<CallToolResult, McpError>
    where
        F: FnOnce(&mut Headless) -> Result<Value, AutomationError> + Send + 'static,
    {
        let started = Instant::now();
        let Some(r) = self.headless_op(f).await else {
            return Ok(structured_err(
                tool,
                &format!("`{tool}` runs in headless mode; against the live app use command_run with the engine command id (see command_list)"),
            ));
        };
        let ms = started.elapsed().as_millis();
        Ok(match r {
            Ok(v) => {
                tracing::info!(tool, ms, "task tool ok");
                structured_ok(tool, v)
            }
            Err(e) => {
                tracing::info!(tool, ms, error = %e, "task tool failed");
                structured_err(tool, &e.to_string())
            }
        })
    }

    /// One editing operation on document `index`, replying with its result and the document.
    async fn edit(&self, tool: &'static str, index: Option<usize>, op: BatchOp) -> Result<CallToolResult, McpError> {
        self.task(tool, move |h| {
            h.on_doc(index, |h| {
                let result = apply(h, &op)?;
                Ok(json!({"result": result, "document": summary(h, None, false)?}))
            })
        })
        .await
    }
}

#[tool_router(router = image_tool_router, vis = "pub(crate)")]
impl PhotocraftMcp {
    #[tool(description = "Open an image file (PNG, JPEG, WebP, TIFF, PSD, .pcraft, EXR, …) beneath the read root and make it the active document. \
                          Returns {document: {index, name, width, height, layerCount, …}, warnings}.")]
    async fn open_image(&self, Parameters(p): Parameters<OpenImageParams>) -> Result<CallToolResult, McpError> {
        self.task("open_image", move |h| {
            check_image_header(h, &p.path)?;
            let opened = h.open(Path::new(&p.path))?;
            let index = opened.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok());
            Ok(json!({"document": summary(h, index, false)?, "warnings": opened.get("warnings")}))
        })
        .await
    }

    #[tool(description = "Describe an image: size, colour mode, bit depth, resolution and layer tree. With `path`, the file is read without \
                          changing the session; otherwise an open document (`index`, default active) is described.")]
    async fn inspect_image(&self, Parameters(p): Parameters<InspectImageParams>) -> Result<CallToolResult, McpError> {
        self.task("inspect_image", move |h| match (&p.path, p.index) {
            (Some(_), Some(_)) => Err(bad("pass either `path` or `index`, not both")),
            (Some(path), None) => inspect_file(h, path),
            (None, index) => summary(h, index, true),
        })
        .await
    }

    #[tool(description = "Save the document beneath the write root; the extension picks the format (.pcraft native, .psd/.psb layered, \
                          .png, .jpg, .webp, .tif, …). Refuses to replace an existing file unless `overwrite` is true.")]
    async fn save_image(&self, Parameters(p): Parameters<SaveImageParams>) -> Result<CallToolResult, McpError> {
        self.task("save_image", move |h| save(h, p.index, &p.path, None, p.quality, p.overwrite)).await
    }

    #[tool(description = "Export the document as `format` (png, jpg, webp, tif, gif, bmp, tga, exr, psd, …) beneath the write root. Flat \
                          formats write a flattened copy and leave the document unchanged. Refuses to replace an existing file unless `overwrite`.")]
    async fn export_image(&self, Parameters(p): Parameters<ExportImageParams>) -> Result<CallToolResult, McpError> {
        self.task("export_image", move |h| save(h, p.index, &p.path, p.format.as_deref(), p.quality, p.overwrite)).await
    }

    #[tool(description = "Resize the image (Image Size): `width` and/or `height` in pixels (one side keeps the aspect ratio), or `percent`, \
                          or `fit` inside width x height. Returns the new size.")]
    async fn resize_image(&self, Parameters(p): Parameters<ResizeParams>) -> Result<CallToolResult, McpError> {
        self.edit("resize_image", p.index, BatchOp::ResizeImage(p)).await
    }

    #[tool(description = "Crop the canvas to the rectangle x, y, width, height (pixels from the top-left).")]
    async fn crop_image(&self, Parameters(p): Parameters<CropParams>) -> Result<CallToolResult, McpError> {
        self.edit("crop_image", p.index, BatchOp::CropImage(p)).await
    }

    #[tool(description = "Rotate the whole canvas by `angle` degrees clockwise (negative: counter-clockwise). 90/180/270 are lossless.")]
    async fn rotate_image(&self, Parameters(p): Parameters<RotateParams>) -> Result<CallToolResult, McpError> {
        self.edit("rotate_image", p.index, BatchOp::RotateImage(p)).await
    }

    #[tool(description = "Mirror the whole canvas horizontally or vertically.")]
    async fn flip_image(&self, Parameters(p): Parameters<FlipParams>) -> Result<CallToolResult, McpError> {
        self.edit("flip_image", p.index, BatchOp::FlipImage(p)).await
    }

    #[tool(description = "Change the brightness of the active layer: `amount` -150..150 (Brightness/Contrast).")]
    async fn adjust_brightness(&self, Parameters(p): Parameters<AmountParams>) -> Result<CallToolResult, McpError> {
        self.edit("adjust_brightness", p.index, BatchOp::AdjustBrightness(p)).await
    }

    #[tool(description = "Change the contrast of the active layer: `amount` -50..100 (Brightness/Contrast).")]
    async fn adjust_contrast(&self, Parameters(p): Parameters<AmountParams>) -> Result<CallToolResult, McpError> {
        self.edit("adjust_contrast", p.index, BatchOp::AdjustContrast(p)).await
    }

    #[tool(description = "Change the colour saturation of the active layer: `amount` -100 (greyscale) .. 100 (Hue/Saturation).")]
    async fn adjust_saturation(&self, Parameters(p): Parameters<AmountParams>) -> Result<CallToolResult, McpError> {
        self.edit("adjust_saturation", p.index, BatchOp::AdjustSaturation(p)).await
    }

    #[tool(description = "Blur the active layer: kind gaussian (radius), box (radius), motion (angle, distance) or surface (radius, threshold).")]
    async fn apply_blur(&self, Parameters(p): Parameters<BlurParams>) -> Result<CallToolResult, McpError> {
        self.edit("apply_blur", p.index, BatchOp::ApplyBlur(p)).await
    }

    #[tool(description = "Sharpen the active layer: kind unsharp_mask (amount, radius, threshold), smart (amount, radius, reduce_noise) or basic.")]
    async fn apply_sharpen(&self, Parameters(p): Parameters<SharpenParams>) -> Result<CallToolResult, McpError> {
        self.edit("apply_sharpen", p.index, BatchOp::ApplySharpen(p)).await
    }

    #[tool(description = "Remove the background of the active pixel layer (Select Subject + edge refinement, on-device, no model download). \
                          Options: keep an editable mask (apply_mask=false), trim to the subject, or put a solid background_color behind it.")]
    async fn remove_background(&self, Parameters(p): Parameters<RemoveBackgroundParams>) -> Result<CallToolResult, McpError> {
        self.edit("remove_background", p.index, BatchOp::RemoveBackground(p)).await
    }

    #[tool(description = "Add a text layer at x, y (baseline) with optional size (pt), font, color (#rrggbb), align and layer name.")]
    async fn add_text(&self, Parameters(p): Parameters<TextParams>) -> Result<CallToolResult, McpError> {
        self.edit("add_text", p.index, BatchOp::AddText(p)).await
    }

    #[tool(description = "Work with layers: list, new, duplicate, delete, rename, set_props (visible, opacity %, blend), select, \
                          bring_forward, send_backward, bring_to_front, send_to_back, merge_down, flatten. `layer` is an id or an exact name. \
                          Replies with the updated layer tree.")]
    async fn manage_layers(&self, Parameters(p): Parameters<LayersParams>) -> Result<CallToolResult, McpError> {
        let index = p.index;
        self.task("manage_layers", move |h| h.on_doc(index, |h| manage_layers(h, &p))).await
    }

    #[tool(description = "Apply the same operations to every image in a folder beneath the read root and write the results to a folder \
                          beneath the write root, one file at a time. Each operation is {\"tool\": <task tool>, …its arguments}. Existing \
                          outputs are skipped unless `overwrite`. Returns per-file status.")]
    async fn batch_process(&self, Parameters(p): Parameters<BatchProcessParams>) -> Result<CallToolResult, McpError> {
        self.task("batch_process", move |h| batch(h, &p)).await
    }
}

#[cfg(test)]
mod tests;
