//! Conversions between the engine's adjustment params (JSON, `photocraft_engine::adjust_params`) and the framework
//! editors' typed [`AdjustValue`]s.
//!
//! The engine's params stay the one stored form; this module is the only place that knows both key sets. The values
//! an editor does not show (Gradient Map's smoothness and method, the other records of a partial Lab set) travel in
//! [`ValueCx::base`], the params the dialog opened with, and [`to_params`] writes the edited fields over them.

use craft::color_ui::ui::gradient::{ColorStop, GradientStops};
use craft::draw::ui::gradient::GradientMap;
use craft::imaging::ui::adjust::{
    BlackWhite, BrightnessContrast, ChannelMixer, ColorBalance, CurvesMode, HueRangeValue, HueSaturation, Levels, PhotoFilter, SelectiveColor, SelectiveMethod,
    ToneChannel, ToneChannels, ToneCurves, ToneSpace as FwSpace,
};
use craft::imaging::ui::curve::Curve;
use craft::imaging::ui::histogram::ChannelColor;
use craft::theme::Srgba;
use photocraft_doc::ColorMode;
use photocraft_doc::adjust::{CURVE_MAP_LEN, CurvePoint, ToneSpace, curve_map, curve_map_outputs};
use photocraft_engine::adjust_params::{self, HUE_RANGES};
use photocraft_engine::channel_cmds::IsolatedCurveChannel;
use serde_json::{Map, Value, json};

use super::{AdjustValue, is_gray};
use crate::tone::Histograms;

/// What a dialog or a layer was opened with, for the conversions of one adjustment.
///
/// `base` is the open operation's input (the recorded, remembered or neutral params), kept by the host for the life of
/// the dialog; it is not state carried over from a pass.
// The decision lists a `target` field; nothing reads it (`isolated` carries the resolved channel), so it is left out.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueCx {
    /// The document's colour mode (picks the Levels/Curves space when the params name none).
    pub mode: ColorMode,
    /// The params the adjustment opened with.
    pub base: Value,
    /// Image › Levels/Curves with a Channels-panel target: the one channel or the colour set it isolates.
    pub isolated: Option<IsolatedCurveChannel>,
}

/// An RGB context over no parameters: what a value nothing opened yet converts in.
impl Default for ValueCx {
    fn default() -> Self {
        Self::new(ColorMode::Rgb, Value::Object(serde_json::Map::new()))
    }
}

impl ValueCx {
    /// A context over `base` with no isolated channel.
    pub fn new(mode: ColorMode, base: Value) -> Self {
        Self { mode, base, isolated: None }
    }

    /// Whether Levels and Curves edit one Gray channel: a grey document, or an isolated gray plane (mask, saved channel).
    pub fn gray(&self) -> bool {
        is_gray(self.mode) || self.isolated.as_ref().is_some_and(|c| c.gray.is_some())
    }

    /// The colour mode the engine parses tone params in: a gray plane parses as Grayscale whatever the document is.
    fn tone_mode(&self) -> ColorMode {
        if self.isolated.as_ref().is_some_and(|c| c.gray.is_some()) { ColorMode::Grayscale } else { self.mode }
    }

    /// Whether the isolated target is a set of colour components: its Levels/Curves keep a combined record.
    fn colour_set(&self) -> bool {
        self.isolated.as_ref().is_some_and(|c| c.components.is_some())
    }
}

// ---------------------------------------------------------------------------------------------
// Tone channels

/// The framework's name of the engine's tone space.
fn framework_space(space: ToneSpace) -> FwSpace {
    match space {
        ToneSpace::Rgb => FwSpace::Rgb,
        ToneSpace::Cmyk => FwSpace::Cmyk,
        ToneSpace::Lab => FwSpace::Lab,
    }
}

/// The Levels/Curves space `v`'s keys address, else the one a document of `mode` gets.
fn space_of(v: &Value, mode: ColorMode) -> ToneSpace {
    let has = |key: &str| v.get(key).is_some_and(|x| !x.is_null());
    if has("cyan") || has("black") {
        ToneSpace::Cmyk
    } else if has("a") || (has("lightness") && !has("red")) {
        ToneSpace::Lab
    } else if has("red") || has("green") || has("blue") {
        ToneSpace::Rgb
    } else {
        adjust_params::default_space(mode)
    }
}

/// The channel of the params key `key` in `space` (`""` is the composite).
fn channel_of_key(key: &str, space: ToneSpace) -> ToneChannel {
    match key {
        "red" => ToneChannel::Red,
        "green" => ToneChannel::Green,
        "blue" => ToneChannel::Blue,
        "cyan" => ToneChannel::Cyan,
        "magenta" => ToneChannel::Magenta,
        "yellow" => ToneChannel::Yellow,
        "black" => ToneChannel::Black,
        "lightness" => ToneChannel::Lightness,
        "a" => ToneChannel::A,
        "b" => ToneChannel::B,
        _ => ToneChannel::Composite(framework_space(space)),
    }
}

/// The params key of the record `channel` edits; `""` is the top-level (composite) record.
fn record_key(channel: &ToneChannel) -> &'static str {
    match channel {
        ToneChannel::Red => "red",
        ToneChannel::Green => "green",
        ToneChannel::Blue => "blue",
        ToneChannel::Cyan => "cyan",
        ToneChannel::Magenta => "magenta",
        ToneChannel::Yellow => "yellow",
        ToneChannel::Black => "black",
        ToneChannel::Lightness => "lightness",
        ToneChannel::A => "a",
        ToneChannel::B => "b",
        _ => "",
    }
}

/// The channels a tone editor lists with the slot of each one's histogram: the one place that rule lives.
///
/// - No isolation: the space's channels, slot = position.
/// - One isolated channel: it alone, at the slot the engine names.
/// - A colour set: a neutral combined entry named after the set (slot 0), then the selected native components, whose
///   slots follow `HistSource::Colors` (component `k` at slot `k + 1`, also in Lab).
fn tone_entries(space: ToneSpace, gray: bool, isolated: Option<&IsolatedCurveChannel>) -> Vec<(ToneChannel, usize)> {
    let Some(iso) = isolated else {
        return ToneChannels::of(framework_space(space), gray).as_slice().iter().cloned().enumerate().map(|(slot, c)| (c, slot)).collect();
    };
    if let Some(colors) = iso.components {
        let keys = adjust_params::channel_keys(space);
        let mut entries = vec![(ToneChannel::named(iso.name.clone(), ChannelColor::Neutral), 0)];
        entries.extend(colors.indices().filter_map(|k| keys.get(k).map(|key| (channel_of_key(key, space), k + 1))));
        return entries;
    }
    let channel =
        if iso.gray.is_none() && !iso.key.is_empty() { channel_of_key(iso.key, space) } else { ToneChannel::named(iso.name.clone(), ChannelColor::Neutral) };
    vec![(channel, iso.hist)]
}

/// The channels a Levels or Curves panel lists for `space` ([`tone_entries`]).
pub fn tone_channels(space: ToneSpace, gray: bool, isolated: Option<&IsolatedCurveChannel>) -> ToneChannels {
    let mut channels: Vec<ToneChannel> = tone_entries(space, gray, isolated).into_iter().map(|(channel, _)| channel).collect();
    match isolated {
        None => ToneChannels::of(framework_space(space), gray),
        Some(iso) if iso.components.is_none() && channels.len() == 1 => ToneChannels::isolated(channels.remove(0)),
        Some(_) => ToneChannels::listed(channels),
    }
}

/// The slot of `histograms` each channel of [`tone_channels`] shows, in channel order.
pub fn histogram_slots(space: ToneSpace, gray: bool, isolated: Option<&IsolatedCurveChannel>) -> Vec<usize> {
    tone_entries(space, gray, isolated).into_iter().map(|(_, slot)| slot).collect()
}

/// The histograms in channel order, the form the panels take (one per channel of [`tone_channels`]).
pub fn panel_histograms(histograms: &Histograms, space: ToneSpace, gray: bool, isolated: Option<&IsolatedCurveChannel>) -> Vec<[u32; 256]> {
    histogram_slots(space, gray, isolated).into_iter().map(|slot| histograms[slot]).collect()
}

/// The host's interpolation of a point curve: the output level (0..=255) of every input level, as the engine draws it.
pub fn curve_sampler(points: &[[f32; 2]]) -> Vec<f32> {
    let curve: Vec<CurvePoint> = points.iter().map(|q| CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
    photocraft_compose::adjust::curve_samples::<256>(&curve).into_iter().map(|y| y * 255.0).collect()
}

/// Converts every curve of `curves` to `mode` with the engine: point curves are drawn into maps, maps are fitted with
/// points (Photoshop's two curve buttons). Curves already in `mode` are kept.
pub fn set_curves_mode(curves: &mut ToneCurves, mode: CurvesMode) {
    for curve in &mut curves.curves {
        *curve = match (&*curve, mode) {
            (Curve::Points { points, .. }, CurvesMode::Pencil) => {
                let pts: Vec<CurvePoint> = points.iter().map(|q| CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
                let outputs = curve_map_outputs(&adjust_params::points_to_map(&pts));
                Curve::Map(outputs.into_iter().map(|o| o * 255.0).collect())
            }
            (Curve::Map(map), CurvesMode::Points) => {
                let outputs: Vec<f32> = map.iter().map(|o| o / 255.0).collect();
                let fitted = adjust_params::map_to_points(&curve_map(&outputs));
                Curve::smooth(fitted.into_iter().map(|p| [(p.input * 255.0).round(), (p.output * 255.0).round()]).collect())
            }
            _ => continue,
        };
    }
    curves.mode = mode;
}

// ---------------------------------------------------------------------------------------------
// JSON readers

fn number(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn numbers<const N: usize>(v: &Value, key: &str, default: [f64; N]) -> [f64; N] {
    let a = v.get(key).and_then(Value::as_array);
    std::array::from_fn(|i| a.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(default[i]))
}

/// A colour as the engine writes it (`"#rrggbb"`) or as it reads it (`[r, g, b]` in 0..1).
fn rgb_of(v: &Value) -> Option<Srgba> {
    match v {
        Value::String(s) => {
            let h = s.trim_start_matches('#');
            let channel = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|x| f32::from(x) / 255.0);
            Some(Srgba::rgb(channel(0)?, channel(2)?, channel(4)?))
        }
        Value::Array(a) => {
            let c = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
            Some(Srgba::rgb(c(0)?, c(1)?, c(2)?))
        }
        _ => None,
    }
}

fn color(v: &Value, key: &str, default: Srgba) -> Srgba {
    v.get(key).and_then(rgb_of).unwrap_or(default)
}

fn hex(c: Srgba) -> String {
    adjust_params::hex([c.r, c.g, c.b])
}

fn object(base: &Value) -> Map<String, Value> {
    base.as_object().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Params -> value

/// Why [`from_params`] refuses a CMYK document's Channel Mixer (cyan, magenta, yellow and black outputs, #8047).
pub const CMYK_MIXER_UNAVAILABLE: &str =
    "Editing a CMYK Channel Mixer (cyan, magenta, yellow and black outputs) is not available in this editor yet; the mixer is kept as it is";

/// The typed value of the adjustment `kind` the engine reads from `params`.
///
/// The engine parses and clamps first (`adjust_params::from_params`), so the value holds what the engine would apply.
/// For a Levels or Curves of a Lab colour set the combined record comes from `params` when they carry `__combinedLab`,
/// else it is neutral: the engine's own Lab serializer aliases the composite to Lightness and would drop it.
pub fn from_params(kind: &str, params: &Value, cx: &ValueCx) -> Result<AdjustValue, String> {
    let tone = matches!(kind, "levels" | "curves");
    let given = if tone && cx.gray() { adjust_params::gray_tone_params(params) } else { params.clone() };
    let mode = if tone { cx.tone_mode() } else { cx.mode };
    // The framework Channel Mixer editor has red, green and blue outputs only. Opened on a CMYK mixer (#8047) it
    // would show RGB defaults, and its edits would be dropped: the engine keeps the ink outputs while the params carry
    // ink keys. Refuse it, so the mixer is kept as it is, until the framework has an ink-output editor. Checked on the
    // given params (an imported monochrome one fails the engine's own parse) and on the parsed ones (a CMYK
    // document's defaults).
    let cmyk_mixer = |p: &Value| kind == "channelMixer" && adjust_params::MIXER_INKS.iter().any(|k| p.get(*k).is_some());
    if cmyk_mixer(params) {
        return Err(CMYK_MIXER_UNAVAILABLE.to_owned());
    }
    let adjustment = adjust_params::from_params(kind, &given, None, mode).map_err(|e| e.to_string())?;
    let mut v = adjust_params::to_dialog_params(&adjustment);
    if cmyk_mixer(&v) {
        return Err(CMYK_MIXER_UNAVAILABLE.to_owned());
    }
    let space = space_of(&v, cx.mode);
    if tone && space == ToneSpace::Lab && cx.colour_set() {
        let pencil = flag(&v, "pencil", false);
        let neutral = adjust_params::from_params(kind, &json!({"pencil": pencil}), None, ColorMode::Grayscale)
            .map(|a| adjust_params::to_params(&a))
            .unwrap_or(Value::Null);
        let combined = if flag(params, "__combinedLab", false) { params } else { &neutral };
        for key in ["points", "inBlack", "inWhite", "gamma", "outBlack", "outWhite"] {
            if let Some(x) = combined.get(key) {
                v[key] = x.clone();
            }
        }
    }
    Ok(match kind {
        "brightnessContrast" => AdjustValue::BrightnessContrast {
            value: BrightnessContrast::new(number(&v, "brightness", 0.0), number(&v, "contrast", 0.0), flag(&v, "legacy", false)),
        },
        "levels" => AdjustValue::Levels { levels: read_levels(&v, &tone_entries(space, cx.gray(), cx.isolated.as_ref())) },
        "curves" => AdjustValue::Curves { curves: read_curves(&v, &tone_entries(space, cx.gray(), cx.isolated.as_ref())) },
        "exposure" => AdjustValue::Exposure { values: vec![number(&v, "exposure", 0.0), number(&v, "offset", 0.0), number(&v, "gamma", 1.0)] },
        "vibrance" => AdjustValue::Vibrance { values: vec![number(&v, "vibrance", 0.0), number(&v, "saturation", 0.0)] },
        "posterize" => AdjustValue::Posterize { values: vec![number(&v, "levels", 4.0)] },
        "hueSaturation" => AdjustValue::HueSaturation { value: read_hue_saturation(&v) },
        "colorBalance" => AdjustValue::ColorBalance {
            value: ColorBalance::new(
                numbers(&v, "shadows", [0.0; 3]),
                numbers(&v, "midtones", [0.0; 3]),
                numbers(&v, "highlights", [0.0; 3]),
                flag(&v, "preserveLuminosity", true),
            ),
        },
        "blackWhite" => {
            let weights = std::array::from_fn(|i| number(&v, HUE_RANGES[i], f64::from(adjust_params::BW_DEFAULTS[i])));
            AdjustValue::BlackWhite { value: BlackWhite::new(weights, flag(&v, "tint", false), color(&v, "tintColor", Srgba::rgb(0.882, 0.827, 0.702))) }
        }
        "photoFilter" => AdjustValue::PhotoFilter {
            value: PhotoFilter::new(color(&v, "color", PhotoFilter::default().color), number(&v, "density", 25.0), flag(&v, "preserveLuminosity", true)),
        },
        "channelMixer" => {
            let gray = flag(&v, "monochrome", false).then(|| numbers(&v, "gray", numbers(&v, "red", [0.0; 4])));
            AdjustValue::ChannelMixer {
                value: ChannelMixer::new(
                    numbers(&v, "red", [100.0, 0.0, 0.0, 0.0]),
                    numbers(&v, "green", [0.0, 100.0, 0.0, 0.0]),
                    numbers(&v, "blue", [0.0, 0.0, 100.0, 0.0]),
                    gray,
                ),
            }
        }
        "threshold" => AdjustValue::Threshold { level: number(&v, "level", 128.0) },
        "gradientMap" => AdjustValue::GradientMap { value: read_gradient_map(&v) },
        "selectiveColor" => {
            let ranges = std::array::from_fn(|i| numbers(&v, photocraft_engine::adjust_cmds::RANGES[i], [0.0; 4]));
            let method = if v.get("method").and_then(Value::as_str) == Some("absolute") { SelectiveMethod::Absolute } else { SelectiveMethod::Relative };
            AdjustValue::SelectiveColor { value: SelectiveColor::new(ranges, method) }
        }
        other => return Err(format!("no editor for adjustment `{other}`")),
    })
}

fn read_levels(v: &Value, entries: &[(ToneChannel, usize)]) -> Vec<Levels> {
    entries
        .iter()
        .map(|(channel, _)| {
            let key = record_key(channel);
            let record = if key.is_empty() { v } else { v.get(key).unwrap_or(&Value::Null) };
            Levels::new(
                number(record, "inBlack", 0.0),
                number(record, "gamma", 1.0),
                number(record, "inWhite", 255.0),
                number(record, "outBlack", 0.0),
                number(record, "outWhite", 255.0),
            )
        })
        .collect()
}

fn read_curves(v: &Value, entries: &[(ToneChannel, usize)]) -> ToneCurves {
    let pencil = flag(v, "pencil", false);
    let curves = entries
        .iter()
        .map(|(channel, _)| {
            let key = record_key(channel);
            let record = v.get(if key.is_empty() { "points" } else { key }).and_then(Value::as_array);
            if pencil {
                let map: Option<Vec<f32>> = record.filter(|a| a.len() == CURVE_MAP_LEN).and_then(|a| a.iter().map(|o| o.as_f64().map(|o| o as f32)).collect());
                Curve::Map(map.unwrap_or_else(|| (0..CURVE_MAP_LEN).map(|k| k as f32).collect()))
            } else {
                let points: Vec<[f32; 2]> = record
                    .map(|a| {
                        a.iter()
                            .filter_map(|q| {
                                let q = q.as_array()?;
                                Some([q.first()?.as_f64()? as f32, q.get(1)?.as_f64()? as f32])
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Curve::smooth(if points.len() < 2 { vec![[0.0, 0.0], [255.0, 255.0]] } else { points })
            }
        })
        .collect();
    ToneCurves::new(curves, if pencil { CurvesMode::Pencil } else { CurvesMode::Points })
}

fn read_hue_saturation(v: &Value) -> HueSaturation {
    let ranges = std::array::from_fn(|i| {
        let r = v.get(HUE_RANGES[i]).unwrap_or(&Value::Null);
        HueRangeValue::new(
            number(r, "hue", 0.0),
            number(r, "saturation", 0.0),
            number(r, "lightness", 0.0),
            numbers(r, "range", HueRangeValue::neutral(i).bounds),
        )
    });
    HueSaturation::new(number(v, "hue", 0.0), number(v, "saturation", 0.0), number(v, "lightness", 0.0), flag(v, "colorize", false), ranges)
}

/// The stops the editor shows: Photoshop's own when the map keeps them (`source`, #8351), else the map's.
fn read_gradient_map(v: &Value) -> GradientMap {
    let source = v.get("source").filter(|s| s.is_object());
    let held = source.unwrap_or(v);
    let mids: Vec<f32> = source
        .and_then(|s| s.get("midpoints"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|m| m.as_f64().map(|m| m as f32)).collect())
        .unwrap_or_default();
    let mut stops: Vec<ColorStop> = held
        .get("stops")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    let (at, c) = match s {
                        Value::Array(p) => (p.first()?.as_f64()?, rgb_of(p.get(1)?)?),
                        Value::Object(o) => (o.get("location")?.as_f64()?, rgb_of(o.get("color")?)?),
                        _ => return None,
                    };
                    Some(ColorStop::new((at as f32).clamp(0.0, 1.0), c).with_midpoint(mids.get(i).copied().unwrap_or(0.5)))
                })
                .collect()
        })
        .unwrap_or_default();
    if stops.len() < 2 {
        stops = vec![ColorStop::new(0.0, Srgba::BLACK), ColorStop::new(1.0, Srgba::WHITE)];
    }
    stops.sort_by(|a, b| a.location.total_cmp(&b.location));
    GradientMap::new(GradientStops::new(stops), flag(v, "reverse", false), flag(v, "dither", false))
}

// ---------------------------------------------------------------------------------------------
// Value -> params

/// The engine's name of the kind `value` edits.
fn kind_name(value: &AdjustValue) -> &'static str {
    match value {
        AdjustValue::BrightnessContrast { .. } => "brightnessContrast",
        AdjustValue::Levels { .. } => "levels",
        AdjustValue::Curves { .. } => "curves",
        AdjustValue::Exposure { .. } => "exposure",
        AdjustValue::Vibrance { .. } => "vibrance",
        AdjustValue::Posterize { .. } => "posterize",
        AdjustValue::HueSaturation { .. } => "hueSaturation",
        AdjustValue::ColorBalance { .. } => "colorBalance",
        AdjustValue::BlackWhite { .. } => "blackWhite",
        AdjustValue::PhotoFilter { .. } => "photoFilter",
        AdjustValue::ChannelMixer { .. } => "channelMixer",
        AdjustValue::Threshold { .. } => "threshold",
        AdjustValue::GradientMap { .. } => "gradientMap",
        AdjustValue::SelectiveColor { .. } => "selectiveColor",
    }
}

/// The complete params of `value`: its fields written over [`ValueCx::base`], so what no editor shows is kept.
///
/// A base the engine cannot read opens the editor on the neutral value; while the value is not `edited`, the base
/// itself is the params, so the engine refuses it at OK instead of the dialog turning it into a neutral setting.
pub fn to_params(value: &AdjustValue, cx: &ValueCx, edited: bool) -> Value {
    if !edited && from_params(kind_name(value), &cx.base, cx).is_err() {
        return cx.base.clone();
    }
    write_params(value, cx)
}

/// What a colour of the image becomes under a Hue/Saturation value: the engine's adjustment run over that one colour
/// (the editor's range swatches and spectrum bar).
pub fn hue_saturation_adjusted(value: &HueSaturation, color: Srgba) -> Srgba {
    let cx = ValueCx::default();
    let params = write_params(&AdjustValue::HueSaturation { value: *value }, &cx);
    let Ok(adjustment) = adjust_params::from_params("hueSaturation", &params, None, cx.mode) else { return color };
    Srgba::from_array(photocraft_compose::adjust::adjust_color(&adjustment, color.to_array()))
}

fn write_params(value: &AdjustValue, cx: &ValueCx) -> Value {
    let mut out = object(&cx.base);
    match value {
        AdjustValue::BrightnessContrast { value } => {
            out.insert("brightness".into(), json!(value.brightness));
            out.insert("contrast".into(), json!(value.contrast));
            out.insert("legacy".into(), json!(value.legacy));
        }
        AdjustValue::Levels { levels } => {
            let space = space_of(&cx.base, cx.mode);
            let entries = tone_entries(space, cx.gray(), cx.isolated.as_ref());
            write_levels(&mut out, &entries, levels, space, cx);
        }
        AdjustValue::Curves { curves } => {
            let space = space_of(&cx.base, cx.mode);
            let entries = tone_entries(space, cx.gray(), cx.isolated.as_ref());
            write_curves(&mut out, &entries, curves, space, cx);
        }
        AdjustValue::Exposure { values } => {
            for (key, x) in ["exposure", "offset", "gamma"].into_iter().zip(values) {
                out.insert(key.into(), json!(x));
            }
        }
        AdjustValue::Vibrance { values } => {
            for (key, x) in ["vibrance", "saturation"].into_iter().zip(values) {
                out.insert(key.into(), json!(x));
            }
        }
        AdjustValue::Posterize { values } => {
            if let Some(levels) = values.first() {
                out.insert("levels".into(), json!(levels.round()));
            }
        }
        AdjustValue::HueSaturation { value } => {
            out.insert("hue".into(), json!(value.hue));
            out.insert("saturation".into(), json!(value.saturation));
            out.insert("lightness".into(), json!(value.lightness));
            out.insert("colorize".into(), json!(value.colorize));
            for (key, r) in HUE_RANGES.iter().zip(&value.ranges) {
                out.insert((*key).into(), json!({"hue": r.hue, "saturation": r.saturation, "lightness": r.lightness, "range": r.bounds}));
            }
        }
        AdjustValue::ColorBalance { value } => {
            out.insert("shadows".into(), json!(value.shadows));
            out.insert("midtones".into(), json!(value.midtones));
            out.insert("highlights".into(), json!(value.highlights));
            out.insert("preserveLuminosity".into(), json!(value.preserve_luminosity));
        }
        AdjustValue::BlackWhite { value } => {
            for (key, w) in HUE_RANGES.iter().zip(value.weights) {
                out.insert((*key).into(), json!(w));
            }
            out.insert("tint".into(), json!(value.tint));
            out.insert("tintColor".into(), json!(hex(value.tint_color)));
        }
        AdjustValue::PhotoFilter { value } => {
            out.remove("filter");
            out.insert("color".into(), json!(hex(value.color)));
            out.insert("density".into(), json!(value.density));
            out.insert("preserveLuminosity".into(), json!(value.preserve_luminosity));
        }
        AdjustValue::ChannelMixer { value } => {
            out.insert("red".into(), json!(value.red));
            out.insert("green".into(), json!(value.green));
            out.insert("blue".into(), json!(value.blue));
            out.insert("monochrome".into(), json!(value.gray.is_some()));
            match value.gray {
                Some(row) => out.insert("gray".into(), json!(row)),
                None => out.remove("gray"),
            };
        }
        AdjustValue::Threshold { level } => {
            out.insert("level".into(), json!(level.round()));
        }
        AdjustValue::GradientMap { value } => write_gradient_map(&mut out, value),
        AdjustValue::SelectiveColor { value } => {
            // The rows carry every range: the single-colour values and `relative` the dialog was opened with would be
            // read after them (and over them) by the engine. `colors`, the chosen row, stays: without those values it
            // changes nothing.
            for key in ["cyan", "magenta", "yellow", "black", "relative"] {
                out.remove(key);
            }
            out.insert("method".into(), json!(if value.method == SelectiveMethod::Absolute { "absolute" } else { "relative" }));
            for (key, row) in photocraft_engine::adjust_cmds::RANGES.iter().zip(value.ranges) {
                out.insert((*key).into(), json!(row));
            }
        }
    }
    let out = Value::Object(out);
    if matches!(value, AdjustValue::Levels { .. } | AdjustValue::Curves { .. }) && cx.gray() { neutral_colour_records(kind_name(value), &out) } else { out }
}

/// A gray plane's parameters: the colour records are the neutral ones of its gray adjustment, whatever `params` carried
/// (#5335: hidden colour curves stay neutral).
fn neutral_colour_records(kind: &str, params: &Value) -> Value {
    let mut out = adjust_params::gray_tone_params(params);
    if let Ok(neutral) = adjust_params::default_for(kind, ColorMode::Grayscale)
        && let (Value::Object(out), Value::Object(neutral)) = (&mut out, adjust_params::to_params(&neutral))
    {
        for key in ["red", "green", "blue", "cyan", "magenta", "yellow", "black", "lightness", "a", "b"] {
            if let Some(record) = neutral.get(key) {
                out.insert(key.into(), record.clone());
            }
        }
    }
    out
}

/// Whether the Lab entries are Lightness, a, b with no combined record: the top-level record then aliases Lightness.
fn lab_alias(space: ToneSpace, cx: &ValueCx) -> bool {
    space == ToneSpace::Lab && !cx.colour_set() && !cx.gray()
}

fn write_levels(out: &mut Map<String, Value>, entries: &[(ToneChannel, usize)], levels: &[Levels], space: ToneSpace, cx: &ValueCx) {
    let record = |l: &Levels| json!({"inBlack": l.in_black, "gamma": l.gamma, "inWhite": l.in_white, "outBlack": l.out_black, "outWhite": l.out_white});
    for ((channel, _), l) in entries.iter().zip(levels) {
        let key = record_key(channel);
        if let (true, Value::Object(fields)) = (key.is_empty(), record(l)) {
            out.extend(fields);
        } else {
            out.insert(key.into(), record(l));
        }
        if lab_alias(space, cx)
            && key == "lightness"
            && let Value::Object(fields) = record(l)
        {
            out.extend(fields);
        }
    }
    if cx.colour_set() && space == ToneSpace::Lab {
        out.insert("__combinedLab".into(), json!(true));
    }
}

fn write_curves(out: &mut Map<String, Value>, entries: &[(ToneChannel, usize)], curves: &ToneCurves, space: ToneSpace, cx: &ValueCx) {
    let pencil = curves.mode == CurvesMode::Pencil;
    let record = |c: &Curve| match c {
        Curve::Points { points, .. } => Some(json!(points.iter().map(|q| [q[0], q[1]]).collect::<Vec<_>>())),
        Curve::Map(map) => Some(json!(map.iter().map(|o| (o * 100.0).round() / 100.0).collect::<Vec<_>>())),
        // A curve kind the engine has no record for is left as `base` has it.
        _ => None,
    };
    for ((channel, _), curve) in entries.iter().zip(&curves.curves) {
        let Some(written) = record(curve) else { continue };
        let key = record_key(channel);
        if lab_alias(space, cx) && key == "lightness" {
            out.insert("points".into(), written.clone());
        }
        out.insert(if key.is_empty() { "points".into() } else { key.to_owned() }, written);
    }
    // Like the engine's own record, a freehand curve says so and a point curve says nothing.
    if pencil {
        out.insert("pencil".into(), json!(true));
    } else {
        out.remove("pencil");
    }
    if cx.colour_set() && space == ToneSpace::Lab {
        out.insert("__combinedLab".into(), json!(true));
    }
}

/// Writes the stops: into Photoshop's gradient when `base` keeps one (its midpoints follow the stops, a new segment
/// halfway; smoothness and method stay as `base` has them), else as the map's own stops.
fn write_gradient_map(out: &mut Map<String, Value>, map: &GradientMap) {
    let mut stops: Vec<&ColorStop> = map.stops.color.iter().collect();
    stops.sort_by(|a, b| a.location.total_cmp(&b.location));
    let written: Vec<Value> = stops.iter().map(|s| json!([(s.location * 1000.0).round() / 1000.0, hex(s.color)])).collect();
    match out.get_mut("source").and_then(Value::as_object_mut) {
        Some(source) => {
            // The engine accepts midpoints in 0.05..=0.95, one per segment.
            let mids: Vec<f32> = stops.iter().take(stops.len().saturating_sub(1)).map(|s| s.midpoint.clamp(0.05, 0.95)).collect();
            source.insert("midpoints".into(), json!(mids));
            source.insert("stops".into(), Value::Array(written));
        }
        None => {
            out.insert("stops".into(), Value::Array(written));
        }
    }
    out.insert("reverse".into(), json!(map.reverse));
    out.insert("dither".into(), json!(map.dither));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tone::{self, HistSource};
    use photocraft_engine::{Session, channel_cmds};

    fn session(mode: &str, depth: u32) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24, "mode": mode, "depth": depth})).unwrap();
        s.execute("channel.target", json!({"colors": [0, 2]})).unwrap();
        s
    }

    #[test]
    fn hue_saturation_swatches_run_the_engines_adjustment() {
        let red = Srgba::from_array([1.0, 0.0, 0.0, 1.0]);
        let blue = Srgba::from_array([0.0, 0.0, 1.0, 1.0]);
        let same = |a: Srgba, b: Srgba| a.to_array().iter().zip(b.to_array()).all(|(x, y)| (x - y).abs() < 1e-3);
        let neutral = HueSaturation::default();
        assert!(same(hue_saturation_adjusted(&neutral, red), red));
        let mut shifted = neutral;
        shifted.hue = 60.0;
        let moved = hue_saturation_adjusted(&shifted, red).to_array();
        assert!(moved[1] > 0.5 && moved[0] > 0.5 && moved[2] < 0.2, "red turns yellow: {moved:?}");
        let mut reds = neutral;
        reds.ranges[0].hue = 60.0;
        assert!(same(hue_saturation_adjusted(&reds, blue), blue), "a Reds-only edit leaves blue");
        assert!(!same(hue_saturation_adjusted(&reds, red), red));
    }

    #[test]
    fn combined_tone_entry_is_neutral_and_only_selected_components_are_listed() {
        for (mode, space, parts, label) in [
            ("rgb", ToneSpace::Rgb, [ToneChannel::Red, ToneChannel::Blue], "RB"),
            ("cmyk", ToneSpace::Cmyk, [ToneChannel::Cyan, ToneChannel::Yellow], "CY"),
            ("lab", ToneSpace::Lab, [ToneChannel::Lightness, ToneChannel::B], "Lb"),
        ] {
            let s = session(mode, 8);
            let isolated = channel_cmds::isolated_curve_channel(s.active().unwrap()).unwrap();
            assert!(!isolated.tint);
            let channels = tone_channels(space, false, Some(&isolated));
            let [first, second] = parts;
            assert_eq!(channels.as_slice(), &[ToneChannel::named(label, ChannelColor::Neutral), first, second], "{mode}");
        }
    }

    #[test]
    fn subset_histograms_match_both_editors_in_all_native_spaces_and_depths() {
        use photocraft_geom::Rect as PixelRect;
        for mode in ["rgb", "cmyk", "lab"] {
            for depth in [8, 16, 32] {
                let mut s = session(mode, depth);
                s.edit("Distinct native histogram bins", |doc, id| {
                    let surface = doc.layer_mut(id.unwrap()).unwrap().surface_mut().unwrap();
                    let mut pixel = vec![0.25, 0.5, 0.75];
                    pixel.resize(surface.format().mode.color_channels(), 0.0);
                    pixel.push(1.0);
                    surface.fill_rect(PixelRect::new(0, 0, 32, 24), &pixel);
                    pixel[0] = 0.375;
                    pixel[2] = 0.875;
                    surface.fill_rect(PixelRect::new(16, 0, 32, 24), &pixel);
                    Ok(())
                })
                .unwrap();
                let st = s.active().unwrap();
                let isolated = channel_cmds::isolated_curve_channel(st).unwrap();
                let colors = isolated.components.unwrap();
                let space = adjust_params::default_space(st.doc.mode);
                let hist =
                    tone::compute_histograms(&st.doc, HistSource::Colors { layer: st.active_layer.unwrap(), colors }, space, tone::HistScope::WholeInput);
                assert_eq!(hist.ch.len(), 1 + st.doc.pixel_format().mode.color_channels());
                let slots = histogram_slots(space, false, Some(&isolated));
                assert_eq!(slots.len(), tone_channels(space, false, Some(&isolated)).len());
                assert_eq!(slots[0], 0, "the combined set is slot 0, also in Lab");
                for slot in &slots {
                    assert!(hist[*slot].iter().any(|n| *n > 0), "{mode}@{depth} slot {slot}");
                }
                assert_eq!(panel_histograms(&hist, space, false, Some(&isolated)).len(), slots.len());
            }
        }
    }

    #[test]
    fn lab_subset_keeps_combined_curve_in_point_pencil_and_component_edits() {
        for pencil in [false, true] {
            for channel in [0usize, 2] {
                let s = session("lab", 8);
                let isolated = channel_cmds::isolated_curve_channel(s.active().unwrap());
                let mut params = adjust_params::to_params(&adjust_params::default_for("curves", ColorMode::Lab).unwrap());
                params["points"] = json!([[0, 0], [100, 150], [255, 255]]);
                params["__combinedLab"] = json!(true);
                let cx = ValueCx { mode: ColorMode::Lab, base: params.clone(), isolated };
                let AdjustValue::Curves { mut curves } = from_params("curves", &params, &cx).unwrap() else { panic!("not curves") };
                assert_eq!(curves.curves.len(), 3, "combined, Lightness, b");
                if pencil {
                    set_curves_mode(&mut curves, CurvesMode::Pencil);
                }
                let value = AdjustValue::Curves { curves: curves.clone() };
                let before = to_params(&value, &cx, true);
                assert!(before.get("points").is_some() && before.get("b").is_some(), "combined and component records both written");
                match &mut curves.curves[channel] {
                    Curve::Points { points, corners, .. } => {
                        points.insert(1, [64.0, 200.0]);
                        corners.insert(1, false);
                    }
                    Curve::Map(map) => map[100] = 240.0,
                    _ => {}
                }
                let after = to_params(&AdjustValue::Curves { curves: curves.clone() }, &cx, true);
                assert_eq!(after.get("pencil").and_then(Value::as_bool).unwrap_or(false), pencil, "only a freehand curve says so");
                if channel == 0 {
                    assert_ne!(after["points"], before["points"]);
                    assert_eq!(after["b"], before["b"]);
                } else {
                    assert_eq!(after["points"], before["points"], "the combined record is not the component's");
                    assert_ne!(after["b"], before["b"]);
                }
                let mut back = curves;
                set_curves_mode(&mut back, if pencil { CurvesMode::Points } else { CurvesMode::Pencil });
                let flipped = to_params(&AdjustValue::Curves { curves: back }, &cx, true);
                assert!(flipped.get("points").is_some() && flipped.get("b").is_some());
            }
        }
    }
}
