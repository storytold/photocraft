//! One editor per adjustment kind, shared by the adjustment-layer Properties panel and the Image › Adjustments
//! dialogs (`adjust_dialog`).
//!
//! Every kind edits an [`AdjustValue`] with its framework editor (`craft::imaging::ui::adjust`, Gradient Map in
//! `craft::draw`). [`editor`] returns the one function a host calls each pass: it shows the editor, writes the edit
//! into the value, and returns the edit's events (Properties reads the phase and gesture of one drag from them) with
//! what the editor asks of the app ([`AdjustAsk`]). The engine's params convert to and from the value in [`values`].
//! Color Lookup keeps its editor in `adjust_ui`.
//!
//! The function reads no input and drains no queue: a second pass in one frame shows the value as the first left it and
//! reports only its own events and asks.

use photocraft_engine::presets::PresetsExt;
use std::sync::Arc;

use craft::behavior::{EditEvent, EditEvents, Mixed};
use craft::color_ui::ui::gradient::{ColorStop, GradientStops};
use craft::draw::ui::gradient::{GradientGroup, GradientItem, GradientMap, GradientMapEditor, GradientMapOutput};
use craft::imaging::ui::adjust::{
    AdjustDialogCx, AdjustField, AdjustSliders, AdjustWords, BlackWhite, BlackWhiteEditor, BrightnessContrast, BrightnessContrastEditor, ChannelMixer,
    ChannelMixerEditor, ColorBalance, ColorBalanceEditor, ColorBalanceTone, CurvesPanel, HueSaturation, HueSaturationEditor, Levels, LevelsPanel, PhotoFilter,
    PhotoFilterEditor, SelectiveColor, SelectiveColorEditor, ThresholdEditor, ToneChannels, ToneCurves, ToneSpace,
};
use craft::imaging::ui::histogram::{ChannelColor, HistogramData};
use craft::theme::Srgba;
use craft::ui::contract::Bind;
use craft::ui::runtime::env::Env;
use craft::ui::runtime::requests::ColorRequest;
use egui::Ui;
use photocraft_doc::LayerId;
use photocraft_engine::adjust_params::HUE_RANGES;
use photocraft_engine::presets::gradients::StopColor;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

mod session;
pub mod values;

pub(crate) use session::PropertiesAdjust;
pub use session::{layer_editor, layer_reset};

/// Kinds edited here (Color Lookup has its own editor in `adjust_ui`; Invert has none).
pub const KINDS: [&str; 14] = [
    "brightnessContrast",
    "levels",
    "curves",
    "exposure",
    "vibrance",
    "hueSaturation",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "posterize",
    "threshold",
    "gradientMap",
    "selectiveColor",
];

/// Whether `kind` has an editor here.
pub fn has_editor(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// The request key of Black & White's tint colour well (the framework editor's).
const BLACK_WHITE_TINT_KEY: &str = "blackWhite.tint";
/// The request key of Photo Filter's colour swatch (the framework editor's).
const PHOTO_FILTER_COLOR_KEY: &str = "photoFilter.color";
/// The request key prefix of a Gradient Map stop's colour; the stop's index follows. The stop bar's own key names no
/// stop, so the editor function rewrites it to this.
const GRADIENT_STOP_KEY: &str = "gradientMap.stop.";

/// The value an adjustment editor edits: one struct variant per kind, tagged with the kind's name, so the dialog's
/// automation fields replace one top-level key (`kind` and the variant's fields).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AdjustValue {
    /// Brightness/Contrast.
    BrightnessContrast {
        /// The value.
        value: BrightnessContrast,
    },
    /// Levels: one per channel of the editor's [`ToneChannels`].
    Levels {
        /// The levels, by channel.
        levels: Vec<Levels>,
    },
    /// Curves: one per channel of the editor's [`ToneChannels`].
    Curves {
        /// The curves, by channel.
        curves: ToneCurves,
    },
    /// Exposure: Exposure, Offset, Gamma Correction.
    Exposure {
        /// The three rows.
        values: Vec<f64>,
    },
    /// Vibrance: Vibrance, Saturation.
    Vibrance {
        /// The two rows.
        values: Vec<f64>,
    },
    /// Posterize: Levels.
    Posterize {
        /// The one row.
        values: Vec<f64>,
    },
    /// Hue/Saturation.
    HueSaturation {
        /// The value.
        value: HueSaturation,
    },
    /// Color Balance.
    ColorBalance {
        /// The value.
        value: ColorBalance,
    },
    /// Black & White.
    BlackWhite {
        /// The value.
        value: BlackWhite,
    },
    /// Photo Filter.
    PhotoFilter {
        /// The value.
        value: PhotoFilter,
    },
    /// Channel Mixer.
    ChannelMixer {
        /// The value.
        value: ChannelMixer,
    },
    /// Threshold.
    Threshold {
        /// The level, 1..=255.
        level: f64,
    },
    /// Gradient Map.
    GradientMap {
        /// The value.
        value: GradientMap,
    },
    /// Selective Color.
    SelectiveColor {
        /// The value.
        value: SelectiveColor,
    },
}

impl Default for AdjustValue {
    fn default() -> Self {
        AdjustValue::BrightnessContrast { value: BrightnessContrast::default() }
    }
}

/// What an editor asks of the app in one pass; the host answers it.
#[derive(Clone, Debug, PartialEq)]
pub enum AdjustAsk {
    /// Black & White's Auto: the host finds the weights ([`black_white_auto`]) and writes them with [`answer_auto`].
    Auto,
    /// Gradient Map's Import .GRD: the host opens its file dialog; the value is unchanged.
    ImportGradients,
    /// A colour well was clicked: the host opens the colour picker with the request and writes the answer with
    /// [`answer_color`].
    PickColor(ColorRequest),
}

/// What one pass of an editor reports.
#[derive(Debug, Default)]
pub struct AdjustOut {
    /// What the editor asks of the app this pass.
    pub ask: Option<AdjustAsk>,
    /// The pass's edit events of the value, in order, with their gesture ids and phases.
    pub edit: EditEvents<AdjustValue>,
}

/// What the editor function reads, built once when the adjustment opens.
#[derive(Clone, Debug)]
pub struct EditorSetup {
    /// The channels Levels and Curves list ([`values::tone_channels`]).
    pub channels: ToneChannels,
    /// The histograms in channel order ([`values::panel_histograms`]); Threshold draws the first.
    pub histograms: Vec<[u32; 256]>,
    /// The document has no point pickers (Lab, CMYK): the Levels and Curves picker rows are off.
    pub pickers_unavailable: bool,
    /// Gradient Map's presets ([`gradient_presets`]).
    pub gradients: Vec<GradientGroup>,
    /// The current foreground and background colours ([`swatches`]), for the host's colour requests.
    pub swatches: [[f32; 3]; 2],
    /// The tone Color Balance shows first; `None` keeps the editor's default (Midtones).
    pub tone: Option<ColorBalanceTone>,
    /// The Smart Filter the dialog edits is not reachable from the active document: the editor shows no controls.
    pub stale: bool,
    /// The clipping display the Levels editor reported on the pass before: the dialog's display changes when the editor's
    /// report does (an Alt-held thumb), so a display set from outside (automation) stays until the editor next reports.
    pub reported_clipping: std::cell::Cell<Option<craft::imaging::ui::adjust::LevelsClip>>,
}

impl Default for EditorSetup {
    fn default() -> Self {
        Self {
            channels: ToneChannels::of(ToneSpace::Rgb, false),
            histograms: Vec::new(),
            pickers_unavailable: false,
            gradients: Vec::new(),
            swatches: [[0.0; 3], [1.0; 3]],
            tone: None,
            stale: false,
            reported_clipping: Default::default(),
        }
    }
}

/// The edit events of one editor lifted onto the value.
fn lifted<T>(events: EditEvents<T>, wrap: impl Fn(T) -> AdjustValue) -> AdjustOut {
    AdjustOut { ask: None, edit: events.map(wrap) }
}

/// Shows the editor of `kind` over `value` for one pass.
fn show(kind: &str, setup: &EditorSetup, ui: &mut Ui, value: &mut AdjustValue, cx: &mut AdjustDialogCx<'_>) -> AdjustOut {
    let context = cx.context;
    match (kind, value) {
        ("brightnessContrast", AdjustValue::BrightnessContrast { value }) => {
            let out = BrightnessContrastEditor::new("Brightness/Contrast", Bind::bound(value)).context(context).show(ui);
            lifted(out.events, |value| AdjustValue::BrightnessContrast { value })
        }
        ("levels", AdjustValue::Levels { levels }) => {
            let mut panel = LevelsPanel::new("Levels", Bind::bound(levels), setup.channels.clone())
                .histograms(&setup.histograms)
                .pickers(Bind::bound(&mut *cx.picker))
                .channel(Bind::bound(&mut *cx.channel))
                .context(context);
            if let Some(canvas) = cx.canvas {
                panel = panel.canvas(canvas);
            }
            if setup.pickers_unavailable {
                panel = panel.pickers_unavailable();
            }
            let out = panel.show(ui);
            if setup.reported_clipping.replace(out.clipping) != out.clipping {
                *cx.clipping = out.clipping;
            }
            lifted(out.edit, |levels| AdjustValue::Levels { levels })
        }
        ("curves", AdjustValue::Curves { curves }) => {
            let mut panel = CurvesPanel::new("Curves", Bind::bound(&mut *curves), setup.channels.clone())
                .histograms(&setup.histograms)
                .sampler(values::curve_sampler)
                .pickers(Bind::bound(&mut *cx.picker))
                .channel(Bind::bound(&mut *cx.channel))
                .context(context);
            if let Some(canvas) = cx.canvas {
                panel = panel.canvas(canvas);
            }
            if let Some(point) = cx.select_point {
                panel = panel.select(point);
            }
            if setup.pickers_unavailable {
                panel = panel.pickers_unavailable();
            }
            let out = panel.show(ui);
            let mut edit = out.edit.map(|curves| AdjustValue::Curves { curves });
            // The panel reports the other mode's icon and leaves the conversion to the engine.
            if let Some(mode) = out.mode {
                let from = AdjustValue::Curves { curves: curves.clone() };
                let mut converted = curves.clone();
                values::set_curves_mode(&mut converted, mode);
                *curves = converted.clone();
                edit.push(EditEvent::set(Mixed::Same(from), AdjustValue::Curves { curves: converted }, Arc::from("Curves Mode"), None));
            }
            AdjustOut { ask: None, edit }
        }
        ("exposure", AdjustValue::Exposure { values }) => {
            let fields = AdjustField::exposure(&Env::of(ui).words::<AdjustWords>());
            let out = AdjustSliders::new("Exposure", &fields, Bind::bound(values)).context(context).show(ui);
            lifted(out.events, |values| AdjustValue::Exposure { values })
        }
        ("vibrance", AdjustValue::Vibrance { values }) => {
            let fields = AdjustField::vibrance(&Env::of(ui).words::<AdjustWords>());
            let out = AdjustSliders::new("Vibrance", &fields, Bind::bound(values)).context(context).show(ui);
            lifted(out.events, |values| AdjustValue::Vibrance { values })
        }
        ("posterize", AdjustValue::Posterize { values }) => {
            let fields = AdjustField::posterize(&Env::of(ui).words::<AdjustWords>());
            let out = AdjustSliders::new("Posterize", &fields, Bind::bound(values)).context(context).show(ui);
            lifted(out.events, |values| AdjustValue::Posterize { values })
        }
        ("hueSaturation", AdjustValue::HueSaturation { value }) => {
            let out = HueSaturationEditor::new("Hue/Saturation", Bind::bound(value)).adjusted(&values::hue_saturation_adjusted).context(context).show(ui);
            lifted(out.edit.events, |value| AdjustValue::HueSaturation { value })
        }
        ("colorBalance", AdjustValue::ColorBalance { value }) => {
            let mut editor = ColorBalanceEditor::new("Color Balance", Bind::bound(value)).context(context);
            if let Some(tone) = setup.tone {
                editor = editor.tone(tone);
            }
            let out = editor.show(ui);
            lifted(out.edit.events, |value| AdjustValue::ColorBalance { value })
        }
        ("blackWhite", AdjustValue::BlackWhite { value }) => {
            let out = BlackWhiteEditor::new("Black & White", Bind::bound(value)).context(context).show(ui);
            let ask = if out.auto { Some(AdjustAsk::Auto) } else { out.pick_color.map(AdjustAsk::PickColor) };
            AdjustOut { ask, edit: out.edit.events.map(|value| AdjustValue::BlackWhite { value }) }
        }
        ("photoFilter", AdjustValue::PhotoFilter { value }) => {
            let out = PhotoFilterEditor::new("Photo Filter", Bind::bound(value)).context(context).show(ui);
            AdjustOut { ask: out.pick_color.map(AdjustAsk::PickColor), edit: out.edit.events.map(|value| AdjustValue::PhotoFilter { value }) }
        }
        ("channelMixer", AdjustValue::ChannelMixer { value }) => {
            let out = ChannelMixerEditor::new("Channel Mixer", Bind::bound(value)).context(context).show(ui);
            lifted(out.events, |value| AdjustValue::ChannelMixer { value })
        }
        ("threshold", AdjustValue::Threshold { level }) => {
            let data = setup.histograms.first().map(|counts| HistogramData::tone(&counts[..], ChannelColor::Neutral));
            let mut editor = ThresholdEditor::new("Threshold", Bind::bound(level)).context(context);
            if let Some(data) = &data {
                editor = editor.histogram(data);
            }
            let out = editor.show(ui);
            lifted(out.events, |level| AdjustValue::Threshold { level })
        }
        ("gradientMap", AdjustValue::GradientMap { value }) => {
            let GradientMapOutput { event, color_requested, import, .. } = GradientMapEditor::new("Gradient Map", &*value).presets(&setup.gradients).show(ui);
            let mut edit = EditEvents::new();
            if let Some(event) = event {
                // The editor reports the edit; it does not write the value.
                event.apply(value);
                edit.push(event.map(|value| AdjustValue::GradientMap { value }));
            }
            let ask = if import {
                Some(AdjustAsk::ImportGradients)
            } else {
                color_requested.map(|stop| {
                    let mut request = ColorRequest::new(format!("{GRADIENT_STOP_KEY}{}", stop.index), stop.request.color);
                    if let Some(title) = stop.request.title.clone() {
                        request = request.title(title);
                    }
                    AdjustAsk::PickColor(request)
                })
            };
            AdjustOut { ask, edit }
        }
        ("selectiveColor", AdjustValue::SelectiveColor { value }) => {
            let out = SelectiveColorEditor::new("Selective Color", Bind::bound(value)).context(context).show(ui);
            lifted(out.edit.events, |value| AdjustValue::SelectiveColor { value })
        }
        // A value of another kind than the editor's: nothing to draw.
        _ => AdjustOut::default(),
    }
}

/// The editor of `kind` as the function a host calls each pass: it shows the editor over the value and returns the
/// pass's edit events and asks. `setup` is built once when the adjustment opens.
pub fn editor(kind: &str, setup: EditorSetup) -> impl Fn(&mut Ui, &mut AdjustValue, &mut AdjustDialogCx<'_>) -> AdjustOut + Send + 'static {
    let kind = kind.to_owned();
    move |ui: &mut Ui, value: &mut AdjustValue, cx: &mut AdjustDialogCx<'_>| -> AdjustOut { show(&kind, &setup, ui, value, cx) }
}

/// [`editor`] reduced to its asks: the function `HostedAdjustDialog::new` takes.
pub fn dialog_editor(kind: &str, setup: EditorSetup) -> impl Fn(&mut Ui, &mut AdjustValue, &mut AdjustDialogCx<'_>) -> Option<AdjustAsk> + Send + 'static {
    let shown = editor(kind, setup);
    move |ui: &mut Ui, value: &mut AdjustValue, cx: &mut AdjustDialogCx<'_>| -> Option<AdjustAsk> { shown(ui, value, cx).ask }
}

/// Writes the colour a picker answered for the request `key` into `value`; `false` when no field of this value has
/// that key. The key table lives here, beside the editors that issue the requests.
pub fn answer_color(value: &mut AdjustValue, key: &str, color: Srgba) -> bool {
    match value {
        AdjustValue::BlackWhite { value } if key == BLACK_WHITE_TINT_KEY => {
            value.tint_color = color;
            true
        }
        AdjustValue::PhotoFilter { value } if key == PHOTO_FILTER_COLOR_KEY => {
            value.color = color;
            true
        }
        AdjustValue::GradientMap { value } => {
            let stop = key.strip_prefix(GRADIENT_STOP_KEY).and_then(|index| index.parse::<usize>().ok()).and_then(|index| value.stops.color.get_mut(index));
            match stop {
                Some(stop) => {
                    // The map's colours are opaque: the picker's alpha is not the stop's.
                    stop.color = color.with_alpha(stop.color.a);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// Writes the weights `image.adjustments.blackWhite.auto` returned ([`black_white_auto`]) into a Black & White value;
/// `false` when `value` is another kind.
pub fn answer_auto(value: &mut AdjustValue, weights: &Value) -> bool {
    let AdjustValue::BlackWhite { value } = value else { return false };
    for (slot, key) in value.weights.iter_mut().zip(HUE_RANGES) {
        if let Some(weight) = weights.get(key).and_then(Value::as_f64) {
            *slot = weight;
        }
    }
    true
}

/// Black & White's Auto (#9269): the weights `image.adjustments.blackWhite.auto` finds for `layer` (an adjustment
/// layer: the image under it; a pixel layer: its pixels; with `filter`, the smart object's pixels below that smart
/// filter). The caller writes them into its value with [`answer_auto`].
pub fn black_white_auto(app: &mut PhotocraftApp, layer: Option<LayerId>, filter: Option<u64>) -> Result<Value, String> {
    let mut params = layer.map_or(json!({}), |id| json!({"layer": id.0}));
    if let Some(index) = filter {
        params["filter"] = json!(index);
    }
    app.run("image.adjustments.blackWhite.auto", params)
}

/// Whether a document of this mode edits Levels/Curves through a single Gray channel.
pub fn is_gray(mode: photocraft_doc::ColorMode) -> bool {
    matches!(mode, photocraft_doc::ColorMode::Grayscale | photocraft_doc::ColorMode::Duotone | photocraft_doc::ColorMode::Bitmap)
}

/// The current foreground and background colours.
pub fn swatches(app: &PhotocraftApp) -> [[f32; 3]; 2] {
    let c = |x: [f32; 4]| [x[0], x[1], x[2]];
    [c(app.session.tools.foreground), c(app.session.tools.background)]
}

/// The session's gradient presets as the Gradient Map picker takes them: colour stops only, the Foreground and
/// Background stops resolved to the current swatches. Converted once when the dialog opens.
pub fn gradient_presets(app: &PhotocraftApp) -> Vec<GradientGroup> {
    let [foreground, background] = swatches(app);
    let resolved = |color: &StopColor| match color {
        StopColor::Foreground => foreground,
        StopColor::Background => background,
        StopColor::Rgb(rgb) => *rgb,
    };
    app.session
        .presets()
        .gradients
        .iter()
        .map(|group| {
            let items = group
                .items
                .iter()
                .map(|preset| {
                    let stops = preset.stops.iter().map(|(at, color)| {
                        let [r, g, b] = resolved(color);
                        ColorStop::new(*at, Srgba::rgb(r, g, b))
                    });
                    GradientItem::new(preset.name.clone(), GradientStops::new(stops.collect()))
                })
                .collect();
            GradientGroup::new(group.name.clone(), items)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::values::{ValueCx, from_params};
    use super::*;
    use photocraft_doc::ColorMode;

    /// The `kind` tag is the key `set_json_field` replaces and the name `KINDS` lists: every kind's value carries its own.
    #[test]
    fn every_kind_converts_to_a_value_tagged_with_its_name() {
        let cx = ValueCx::new(ColorMode::Rgb, json!({}));
        for kind in KINDS {
            let value = from_params(kind, &json!({}), &cx).unwrap_or_else(|e| panic!("{kind}: {e}"));
            assert_eq!(serde_json::to_value(&value).unwrap()["kind"], kind);
            assert!(has_editor(kind));
        }
    }

    /// Answers are written by the key the editor asked with, and refused for a value without the field.
    #[test]
    fn a_colour_answer_lands_in_the_field_its_key_names() {
        let mut tint = AdjustValue::BlackWhite { value: BlackWhite::default() };
        assert!(answer_color(&mut tint, BLACK_WHITE_TINT_KEY, Srgba::rgb(1.0, 0.0, 0.0)));
        assert!(matches!(&tint, AdjustValue::BlackWhite { value } if value.tint_color == Srgba::rgb(1.0, 0.0, 0.0)));
        assert!(!answer_color(&mut tint, PHOTO_FILTER_COLOR_KEY, Srgba::BLACK), "another kind's key");
        let mut map = from_params("gradientMap", &json!({}), &ValueCx::new(ColorMode::Rgb, json!({}))).unwrap();
        assert!(answer_color(&mut map, &format!("{GRADIENT_STOP_KEY}1"), Srgba::rgb(0.0, 1.0, 0.0)));
        assert!(!answer_color(&mut map, &format!("{GRADIENT_STOP_KEY}9"), Srgba::BLACK), "no such stop");
    }
}

#[cfg(test)]
mod ui_tests;
