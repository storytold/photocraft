//! Modal Image › Adjustments dialogs (Levels, Curves, Hue/Saturation, …): a framework `HostedAdjustDialog` over an
//! [`AdjustValue`], with the kind's editor from `adjust_editors` (the editors the adjustment-layer Properties panel
//! uses too).
//!
//! The dialog host keeps the value and the parameters it converts over ([`DialogValue`]), so the dialog lists the
//! command's parameters itself. What the app keeps beside it is [`OpenAdjust`] (`app.adjust_open`): the command and
//! target the dialog opened with. While it is open the canvas previews the
//! settings as a temporary adjustment layer clipped to the target (`adjust_preview`, composited on the GPU); where
//! that can't match the command, the *real command* runs on the proxy document through the filter-preview machinery.
//! OK runs the command once (one history step) and Cancel drops the preview, leaving the document untouched.
//!
//! [`take`] is the one entry the result pump calls each pass: it answers the editor's asks, follows a retarget, and
//! turns the host's OK or Cancel into the command, the Actions step hooks and the remembered settings.

use std::sync::{Arc, Mutex};

use craft::imaging::ui::adjust::{AdjustDialogCx, CurvesPanel, HostedAdjustDialog as FrameworkDialog, ToneChannels, TonePicker};
use craft::theme::Srgba;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use photocraft_doc::ColorMode;
use photocraft_engine::channel_cmds::ToneTarget;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::adjust_editors::values::{self, ValueCx};
use crate::adjust_editors::{self, AdjustAsk, AdjustValue, EditorSetup};
use crate::tone::{self, HistSource};

const PREFIX: &str = "image.adjustments.";

/// The hosted adjustment dialog the app opens.
pub(crate) type HostedAdjustDialog = FrameworkDialog<DialogValue, AdjustAsk>;

/// What the hosted dialog holds: the editor's value and the context it converts to the command's parameters in, so
/// the dialog lists its own parameters. The context is not the user's to edit: an opening or a retarget seeds it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct DialogValue {
    #[serde(flatten)]
    pub(crate) value: AdjustValue,
    /// The parameters the dialog was opened with, addressing included, and the value's conversion context.
    #[serde(skip)]
    pub(crate) cx: ValueCx,
}

impl DialogValue {
    /// The command's parameters: the value over the parameters the dialog opened with ([`values::to_params`]).
    pub(crate) fn params(&self, edited: bool) -> Value {
        values::to_params(&self.value, &self.cx, edited)
    }
}

/// The dialog's automation fields: the command's parameters.
fn json_form(value: &DialogValue, edited: bool) -> Map<String, Value> {
    match value.params(edited) {
        Value::Object(params) => params,
        _ => Map::new(),
    }
}

/// The open adjustment dialog's context in the app: what it was opened for, not what it shows (the value is the host's).
pub(crate) struct OpenAdjust {
    /// The dialog's id on the host.
    pub(crate) id: DialogId,
    /// The adjustment kind (`levels`, `curves`, …).
    pub(crate) kind: String,
    /// The command OK runs (`image.adjustments.<kind>`).
    pub(crate) command: String,
    /// The colour mode the dialog seeds and parses in: a mask, saved channel or Quick Mask holds gray levels whatever
    /// the document's mode, so its curve is a neutral one and carries no Lab or CMYK records (#5335).
    pub(crate) mode: ColorMode,
    /// The layer and isolated plane the dialog edits.
    pub(crate) target: ToneTarget,
    /// A captured Smart Filter's index and target: the dialog edits that filter's settings, in document colour.
    pub(crate) smart_filter: Option<(usize, Value)>,
    /// The target did not resolve at open: the dialog shows defaults and seeds again once it does.
    pub(crate) unresolved: bool,
    /// The parameters before the armed point picker's last sample, which a second sample of the same picker replaces.
    pub(crate) picker_base: Option<Value>,
    /// The picker and parameters its last sample produced: the sample is the latest edit while the parameters still match.
    picked: Option<(TonePicker, Value)>,
    /// What the editor reads: histograms, channels, swatches. Rebuilt only when the target changes.
    pub(crate) setup: Arc<Mutex<EditorSetup>>,
    /// The recorded parameters the value was seeded from, for seeding again when an unresolved target resolves.
    recorded: Value,
    /// The captured parameters a Smart Filter edit lays over the recorded ones through the engine's normal form.
    normalized: Option<Value>,
}

/// The conversion context of a value in `mode` with the parameters `base` and the isolated plane `isolated`.
fn value_cx(mode: ColorMode, isolated: Option<photocraft_engine::channel_cmds::IsolatedCurveChannel>, base: Value) -> ValueCx {
    let mut cx = ValueCx::new(mode, base);
    cx.isolated = isolated;
    cx
}

/// The command parameters of the open adjustment dialog `id`: its value over the parameters it opened with, or `null`
/// when `id` is not the open adjustment dialog.
pub fn command_params(app: &PhotocraftApp, id: DialogId) -> Value {
    open_value(app).filter(|open| open.id == id).and_then(|_| open_params(app)).unwrap_or(Value::Null)
}

/// The open dialog's context, while the host still holds that dialog.
pub(crate) fn open_value(app: &PhotocraftApp) -> Option<&OpenAdjust> {
    let open = app.adjust_open.as_ref()?;
    app.framework.dialogs.get_by_id::<HostedAdjustDialog>(open.id).map(|_| open)
}

/// The hosted dialog `id` names.
pub(crate) fn hosted(app: &PhotocraftApp, id: DialogId) -> Option<&HostedAdjustDialog> {
    app.framework.dialogs.get_by_id::<HostedAdjustDialog>(id)
}

#[cfg(test)]
/// The hosted dialog `id` names, for tests and automation that arm its picker or edit its value.
///
/// # Panics
/// When no adjustment dialog with this id is open.
pub(crate) fn hosted_mut(app: &mut PhotocraftApp, id: DialogId) -> &mut HostedAdjustDialog {
    app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id).expect("the adjustment dialog is open")
}

/// The open dialog's engine parameters: its value over the parameters it opened with.
pub(crate) fn open_params(app: &PhotocraftApp) -> Option<Value> {
    let open = open_value(app)?;
    let dialog = hosted(app, open.id)?;
    Some(dialog.value().params(dialog.edited()))
}

/// Whether `command` opens an adjustment dialog (when run without params).
pub fn has_dialog(command: &str) -> bool {
    command.strip_prefix(PREFIX).is_some_and(adjust_editors::has_editor)
}

/// The Levels/Curves dialog's target from panel state or explicit parameters, including filter masks.
/// Re-editing a captured Smart Filter always exposes its document-colour controls.
fn target_context(app: &PhotocraftApp, smart: Option<&(usize, Value)>, params: &Value) -> photocraft_engine::Result<ToneTarget> {
    if let Some((_, filter)) = smart {
        let (layer, _) = crate::smart_filter_dialog::validate_target(app, filter).map_err(photocraft_engine::EngineError::Other)?;
        return Ok(ToneTarget { layer: Some(layer), channel: None });
    }
    let mut params = params.clone();
    if params.get("target").is_none()
        && app.filter_mask_targeted()
        && let Some(object) = params.as_object_mut()
    {
        object.insert("target".into(), json!("filterMask"));
    }
    photocraft_engine::channel_cmds::resolve_tone_target(&app.session, app.layer_mask_targeted(), &params)
}

/// The colour mode the Levels/Curves dialog seeds and parses in (see [`OpenAdjust::mode`]).
fn curve_mode(app: &PhotocraftApp, kind: &str, target: &ToneTarget) -> ColorMode {
    let mode = app.session.active().map_or(ColorMode::Rgb, |s| s.doc.mode);
    if matches!(kind, "levels" | "curves") && target.channel.as_ref().is_some_and(|c| c.gray.is_some()) { ColorMode::Grayscale } else { mode }
}

/// Whether the armed `picker` can sample for the open dialog now: a resolved target in a mode the picker works in.
pub(crate) fn picker_applies(app: &PhotocraftApp, open: &OpenAdjust, picker: TonePicker) -> bool {
    matches!(open.kind.as_str(), "levels" | "curves")
        && app.session.active().is_some()
        && hosted(app, open.id).is_some_and(|dialog| target_context(app, open.smart_filter.as_ref(), &dialog.value().cx.base).is_ok())
        && picker.supported(open.mode == ColorMode::Rgb, adjust_editors::is_gray(open.mode))
}

/// The Curves eyedropper of the engine for a tone picker.
fn eyedropper_of(picker: TonePicker) -> Option<(photocraft_engine::adjust_params::CurvesEyedropper, &'static str)> {
    use photocraft_engine::adjust_params::CurvesEyedropper;
    match picker {
        TonePicker::Black => Some((CurvesEyedropper::Black, "black")),
        TonePicker::Gray => Some((CurvesEyedropper::NeutralGray, "gray")),
        TonePicker::White => Some((CurvesEyedropper::White, "white")),
        _ => None,
    }
}

/// Edits the open dialog from engine parameters (automation's sets, a picker's sample, Auto): the addressing keys
/// (`target`, `layer`, `document`) retarget it, the rest become its value. An unresolved dialog that is retargeted seeds
/// again from the new address when it resolves. Refused parameters leave the value as it is.
pub fn apply_params(app: &mut PhotocraftApp, id: DialogId, params: &Value) -> bool {
    set_params(app, id, params, Change::Edit)
}

/// Seeds the open dialog from engine parameters as [`apply_params`] sets them, without counting as an edit: the
/// settings an opening restores.
fn seed_params(app: &mut PhotocraftApp, id: DialogId, params: &Value) -> bool {
    set_params(app, id, params, Change::Seed)
}

/// Whether a change of the dialog's value is the user's edit or the value the dialog opens at.
#[derive(Clone, Copy, PartialEq)]
enum Change {
    Edit,
    Seed,
}

fn set_params(app: &mut PhotocraftApp, id: DialogId, params: &Value, change: Change) -> bool {
    let Some(open) = app.adjust_open.as_mut().filter(|open| open.id == id) else { return false };
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) else { return false };
    let mut shown = dialog.value().clone();
    let mut retargeted = false;
    for (key, address) in addressing(params) {
        if shown.cx.base.get(&key) != Some(&address) {
            retargeted = true;
            for held in [&mut shown.cx.base, &mut open.recorded] {
                if let Some(object) = held.as_object_mut() {
                    object.insert(key.clone(), address.clone());
                }
            }
        }
    }
    let read = if retargeted && open.unresolved { None } else { values::from_params(&open.kind, params, &shown.cx).ok() };
    let taken = read.is_some();
    match read {
        Some(value) if change == Change::Edit => *dialog.value_mut() = DialogValue { value, ..shown },
        Some(value) => dialog.seed(DialogValue { value, ..shown }),
        // The address is the dialog's context, not a setting: a refused value keeps the retarget.
        None if retargeted => dialog.seed(shown),
        None => {}
    }
    taken || (retargeted && open.unresolved)
}

/// The parameter keys that address the command (not settings): they stay when parameters are replaced.
fn addressing(params: &Value) -> Map<String, Value> {
    params
        .as_object()
        .map(|p| p.iter().filter(|(key, _)| matches!(key.as_str(), "target" | "layer" | "document")).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// Sample the committed colour composite or targeted grayscale plane and set the armed Levels or Curves point on the
/// open dialog's value. Returns true when an armed picker consumed the pointer event, even if the point was outside
/// the image, transparent or in a mode the picker does not work in: nothing changes then and the picker stays armed.
///
/// The sample replaces the picker's own previous one: it is laid over the parameters from before that sample while
/// they are still the dialog's latest edit, so a second click with one picker moves its point instead of adding one.
pub(crate) fn pick(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    let Some(open) = open_value(app) else { return false };
    let id = open.id;
    let Some(picker) = hosted(app, id).and_then(|dialog| dialog.picker()) else { return false };
    if !matches!(open.kind.as_str(), "levels" | "curves") {
        return false;
    }
    let (kind, mode) = (open.kind.clone(), open.mode);
    let gray = open.target.channel.as_ref().is_some_and(|c| c.gray.is_some());
    let base = hosted(app, id).map_or(Value::Null, |dialog| dialog.value().cx.base.clone());
    let resolved = target_context(app, open.smart_filter.as_ref(), &base).err().map(|error| error.to_string());
    let current = open_params(app).unwrap_or(Value::Null);
    let base = match open.picked.as_ref() {
        Some((last, result)) if *last == picker && *result == current => open.picker_base.clone().unwrap_or_else(|| current.clone()),
        _ => current,
    };
    if let Some(error) = resolved {
        app.ui.status = error;
        return true;
    }
    let Some((eyedropper, point)) = eyedropper_of(picker) else { return true };
    if !picker.supported(mode == ColorMode::Rgb, adjust_editors::is_gray(mode)) {
        return true;
    }
    let adjustment = if gray {
        let mut params = app.with_mask_target(&format!("{PREFIX}{kind}"), base.clone());
        let Some(object) = params.as_object_mut() else {
            app.ui.status = "Invalid eyedropper parameters".into();
            return true;
        };
        object.insert("eyedropper".into(), json!({"point": point, "at": at, "size": app.ui.tool_options.eyedropper.size.pixels()}));
        photocraft_engine::adjust_params::destructive_eyedropper(&app.session, &kind, &params)
            .and_then(|a| a.ok_or_else(|| photocraft_engine::EngineError::Other("missing eyedropper adjustment".into())))
    } else {
        let Some(sample) = crate::canvas::composite_color(app, at[0], at[1]) else { return true };
        photocraft_engine::adjust_params::adjustment_eyedropper(&kind, &base, eyedropper, sample, mode)
    };
    let adjustment = match adjustment {
        Ok(adjustment) => adjustment,
        Err(error) => {
            app.ui.status = error.to_string();
            return true;
        }
    };
    // Scalar samples retain their native precision. The colour picker keeps its existing hundredth-level
    // curve-point convention, independently of source sample depth.
    let mut params =
        if gray { photocraft_engine::adjust_params::to_dialog_params(&adjustment) } else { photocraft_engine::adjust_params::to_params(&adjustment) };
    if let (Value::Object(object), Some(dialog)) = (&mut params, hosted(app, id)) {
        object.extend(addressing(&dialog.value().cx.base));
    }
    if !apply_params(app, id, &params) {
        return true;
    }
    let result = open_params(app).unwrap_or(Value::Null);
    if let Some(open) = app.adjust_open.as_mut() {
        open.picker_base = Some(base);
        open.picked = Some((picker, result));
    }
    true
}

/// The curve's output at each whole input level 0..=255, in levels, through the engine's interpolation of `points`
/// (levels): the sampler `CurvesPanel` draws and inserts points with.
fn curve_levels(points: &[[f32; 2]]) -> Vec<f32> {
    let curve: Vec<_> = points.iter().map(|q| photocraft_doc::adjust::CurvePoint { input: q[0] / 255.0, output: q[1] / 255.0 }).collect();
    let lut = photocraft_compose::adjust::curve_lut(&curve);
    let last = lut.len().saturating_sub(1) as f32;
    (0..256).map(|level| lut.get((level as f32 / 255.0 * last).round() as usize).map_or(level as f32, |output| (output * 255.0).round())).collect()
}

/// ⌘/Ctrl-click on the image with Curves open (#7325): a point on the curve shown in the dialog at the clicked pixel's
/// tone, at the output the curve gives there rounded to a whole level, so the image barely changes (where the curve is
/// clipped at 0 or 255 the point re-shapes it, as a click on the graph there does); with `every_channel` (Shift) one on
/// each colour channel instead, at that channel's value. The pixel comes from the same merged composite as the
/// eyedroppers. Returns true when Curves is open: the click is the dialog's even when nothing is added (pencil curve,
/// Lab or CMYK, a mask or saved channel target, outside the image, a full curve).
pub(crate) fn add_point(app: &mut PhotocraftApp, at: [f64; 2], every_channel: bool) -> bool {
    let Some(open) = open_value(app).filter(|open| open.kind == "curves") else { return false };
    let id = open.id;
    let Some(base) = hosted(app, id).map(|dialog| &dialog.value().cx.base) else { return false };
    let names_plane =
        base.get("target").is_some_and(|target| matches!(target.as_str(), Some("mask" | "filterMask" | "quickMask")) || target.get("channel").is_some());
    // A mask, saved channel or Quick Mask holds its own gray levels, not the composite's.
    let gray_plane = open.target.channel.as_ref().is_some_and(|c| c.gray.is_some());
    let isolated = open.target.channel.clone();
    if names_plane {
        return true;
    }
    let Some(mode) = app.session.active().map(|s| s.doc.mode) else { return true };
    if !matches!(mode, ColorMode::Rgb | ColorMode::Grayscale) {
        app.ui.status = tl!("Unavailable in this color mode").into();
        return true;
    }
    if gray_plane {
        return true;
    }
    let Some(sample) = crate::canvas::composite_color(app, at[0], at[1]) else { return true };
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) else { return true };
    let channels = values::tone_channels(photocraft_engine::adjust_params::default_space(mode), adjust_editors::is_gray(mode), isolated.as_ref());
    let shown = dialog.channel();
    let mut value = dialog.value().clone();
    let AdjustValue::Curves { curves, .. } = &mut value.value else { return true };
    let added = CurvesPanel::add_point_from_sample(curves, &channels, shown, Srgba::from_rgb_array(sample), every_channel, &curve_levels);
    if let Some(index) = added {
        *dialog.value_mut() = value;
        dialog.select_point(index);
        // The next sample of a picker starts from the curve with this point.
        if let Some(open) = app.adjust_open.as_mut() {
            open.picker_base = None;
            open.picked = None;
        }
    }
    true
}

/// Settings an adjustment dialog keeps from its last OK although it opens at its neutral ones:
/// the reference application's Photo Filter reopens at Warming Filter (85) and 25 % every time,
/// but with Preserve Luminosity as last confirmed (measured 2026-11-08, #9879). ⌥ still reopens
/// at all the last settings ([`open_last`]).
const KEEPS_LAST: &[(&str, &str)] = &[("photoFilter", "preserveLuminosity")];

/// The saved-tier key of the settings last confirmed in the dialog of `kind`.
pub fn last_key(kind: &str) -> SaveKey {
    SaveKey::named(format!("photocraft.adjust.last.{kind}"))
}

/// The settings last confirmed in the dialog of `kind`, if any.
fn last_confirmed(app: &PhotocraftApp, kind: &str) -> Option<Value> {
    saved_value::<Option<Value>>(app.ctx(), &last_key(kind))
}

/// Opens the dialog for `command` at the kind's neutral settings, except the [`KEEPS_LAST`] ones,
/// which keep their last confirmed value.
pub fn open(app: &mut PhotocraftApp, command: &str) -> Result<DialogId, String> {
    let id = open_with_params_checked(app, command, &json!({})).map_err(|reason| reason.unwrap_or_else(|| refusal(app, command)))?;
    let kind = command.strip_prefix(PREFIX).unwrap_or_default();
    let last = last_confirmed(app, kind);
    let kept: Map<String, Value> = KEEPS_LAST
        .iter()
        .filter(|(k, _)| *k == kind)
        .filter_map(|(_, key)| last.as_ref()?.get(*key).filter(|v| v.is_boolean()).map(|v| ((*key).to_owned(), v.clone())))
        .collect();
    if !kept.is_empty()
        && let Some(Value::Object(mut params)) = open_params(app)
    {
        params.extend(kept);
        seed_params(app, id, &Value::Object(params));
    }
    Ok(id)
}

/// Why `command` has no dialog to open when the editor gave no reason of its own: no document, or no adjustment of
/// that name.
fn refusal(app: &PhotocraftApp, command: &str) -> String {
    if app.session.active().is_none() { "no document".to_string() } else { format!("{command} has no dialog") }
}

/// Why a dialog did not open: `Some` is the editor's refusal of these parameters, to show the user (a CMYK Channel
/// Mixer, #8047); `None` means there is no dialog for them.
pub(crate) type Refused = Option<String>;

/// Opens a neutral adjustment dialog using the current edit target.
pub fn open_neutral(app: &mut PhotocraftApp, command: &str) -> Option<DialogId> {
    open_with_params(app, command, &json!({}))
}

/// Seed an Action's dialog only after resolving its recorded target, so document-space
/// defaults cannot overwrite the target's neutral grayscale values.
pub(crate) fn open_with_params(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<DialogId> {
    open_with_params_checked(app, command, params).ok()
}

/// [`open_with_params`], with the reason it did not open.
pub(crate) fn open_with_params_checked(app: &mut PhotocraftApp, command: &str, params: &Value) -> Result<DialogId, Refused> {
    let params = if params.is_null() { json!({}) } else { Value::Object(params.as_object().ok_or(None)?.clone()) };
    let params = complete_params(app, command, &params).unwrap_or(params);
    let mut params = app.with_mask_target(command, params);
    if params.get("target").is_none()
        && let Some(colors) = photocraft_engine::color_channels::selected(&app.session)
        && let Some(object) = params.as_object_mut()
    {
        object.insert("target".into(), colors.params());
    }
    open_hosted(app, command, params, None, None)
}

/// [`open_smart_filter_checked`] without the reason (the tests' form).
#[cfg(test)]
pub(crate) fn open_smart_filter(app: &mut PhotocraftApp, command: &str, params: &Value, target: Value) -> Option<DialogId> {
    open_smart_filter_checked(app, command, params, target).ok()
}

/// Reopen a captured Smart Filter with document-colour defaults, independent of channel targeting.
/// Only the caller's validated snapshot may supply the Smart Filter target. When it does not open, the reason: the
/// Layers panel shows an editor's refusal (#8047).
pub(crate) fn open_smart_filter_checked(app: &mut PhotocraftApp, command: &str, params: &Value, target: Value) -> Result<DialogId, Refused> {
    let completed = complete_params(app, command, params);
    let mut recorded = completed.as_ref().unwrap_or(params).as_object().cloned().unwrap_or_default();
    recorded.retain(|key, _| !key.starts_with("__"));
    let index = target.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or(None)?;
    open_hosted(app, command, Value::Object(recorded), Some((index, target)), Some(params.clone()))
}

/// What an opening seeds: the value, its conversion context and the target it resolved.
struct Seeded {
    value: AdjustValue,
    cx: ValueCx,
    target: ToneTarget,
    mode: ColorMode,
    unresolved: bool,
}

/// Seed the dialog from `recorded`: defaults for the kind and mode, the recorded parameters over them, the gray alias
/// resolved before the defaults (Levels' alias would otherwise override visible control edits, while Curves' default
/// points would hide the alias entirely), an invalid gray alias kept for the engine to refuse at OK, Selective Color's
/// `colors` row kept. With `normalized` (a Smart Filter edit: the captured parameters) those then replace the parameters
/// through the engine's own reading of them, so a Lab `lightness` default cannot be read over recorded `points`. An
/// unresolved Levels/Curves target opens with defaults and seeds again in [`take`] once it resolves.
fn seed(app: &PhotocraftApp, kind: &str, recorded: &Value, smart: Option<&(usize, Value)>, normalized: Option<&Value>) -> Result<Seeded, Refused> {
    let resolved = target_context(app, smart, recorded);
    let valid_target = !matches!(kind, "levels" | "curves") || resolved.is_ok();
    let target = resolved.unwrap_or_else(|_| ToneTarget { layer: app.session.active().and_then(|s| s.active_layer), channel: None });
    let mode = curve_mode(app, kind, &target);
    let defaults = if valid_target { Some(photocraft_engine::adjust_params::default_for(kind, mode).map_err(|_| None)?) } else { None };
    let tone_kind = matches!(kind, "levels" | "curves");
    let gray_target = tone_kind && target.channel.as_ref().is_some_and(|c| c.gray.is_some());
    let color_set = if tone_kind { target.channel.as_ref().and_then(|c| c.components) } else { None };
    let supplied = recorded.as_object().cloned().unwrap_or_default();
    let mut supplied = if gray_target {
        photocraft_engine::adjust_params::gray_tone_params(&Value::Object(supplied)).as_object().cloned().unwrap_or_default()
    } else {
        supplied
    };
    if valid_target && tone_kind && supplied.contains_key("gray") {
        let params = Value::Object(supplied.clone());
        let canonical = if let Some(colors) = color_set {
            photocraft_engine::color_channels::tone_dialog_params(kind, &params, mode, colors)
        } else {
            photocraft_engine::adjust_params::from_params(kind, &params, None, mode).map(|a| photocraft_engine::adjust_params::to_dialog_params(&a))
        };
        if let Ok(Value::Object(canonical)) = canonical {
            supplied = photocraft_engine::adjust_params::gray_tone_params(&params).as_object().cloned().unwrap_or_default();
            supplied.remove("gray");
            supplied.remove("smooth"); // Already applied by parsing; never smooth a second time.
            supplied.extend(canonical);
            // A Lab set's canonical parameters carry the true combined record, which the editor reads only when flagged.
            if color_set.is_some() && mode == ColorMode::Lab {
                supplied.insert("__combinedLab".into(), json!(true));
            }
        }
    }
    let mut params = Map::new();
    let mut defaults_only = Map::new();
    if let Some(defaults) = defaults
        && let Value::Object(mut p) = photocraft_engine::adjust_params::to_params(&defaults)
    {
        defaults_only = p.clone();
        // An invalid gray alias must reach engine validation: seeded identity points would
        // otherwise override it. Valid aliases were already canonicalized above.
        if kind == "curves" && supplied.get("gray").is_some_and(|v| !v.is_null()) && supplied.get("points").is_none_or(Value::is_null) {
            p.remove("points");
        }
        params.extend(p);
    }
    params.extend(supplied);
    if let Some(captured) = normalized
        && let Ok(adjustment) = photocraft_engine::adjust_params::from_params(kind, captured, None, mode)
        && let Value::Object(mut settings) = photocraft_engine::adjust_params::to_dialog_params(&adjustment)
    {
        // Selective Color's chosen row is editor state, absent from the normalized adjustment.
        if kind == "selectiveColor"
            && let Some(colors) = captured.get("colors")
        {
            settings.insert("colors".into(), colors.clone());
        }
        // The normalized pencil maps already include any recorded smoothing: replacing the parameters drops the old count.
        params.retain(|key, _| matches!(key.as_str(), "target" | "layer" | "document"));
        params.extend(settings);
    }
    let base = Value::Object(params);
    let cx = value_cx(mode, target.channel.clone(), base.clone());
    // A CMYK Channel Mixer has no framework editor yet (#8047): no dialog, rather than RGB sliders over its inks. Checked
    // before the defaults fallback too, which in an RGB or Lab document reads (RGB defaults) and would open the RGB
    // editor over a Smart Filter's ink keys.
    let value = match values::from_params(kind, &base, &cx) {
        Err(err) if err == values::CMYK_MIXER_UNAVAILABLE => return Err(Some(err)),
        read => read.or_else(|_| values::from_params(kind, &Value::Object(defaults_only), &cx)),
    };
    let value = match value {
        Ok(value) => value,
        Err(err) if err == values::CMYK_MIXER_UNAVAILABLE => return Err(Some(err)),
        Err(_) => AdjustValue::default(),
    };
    Ok(Seeded { value, cx, target, mode, unresolved: !valid_target })
}

/// Whether the kind's editor shows a histogram.
fn needs_histogram(kind: &str) -> bool {
    matches!(kind, "levels" | "curves" | "threshold")
}

/// Whether the dialog shows a gray document or plane.
fn gray_context(app: &PhotocraftApp, kind: &str, target: &ToneTarget) -> bool {
    let plane = matches!(kind, "levels" | "curves") && target.channel.as_ref().is_some_and(|c| c.gray.is_some());
    plane || app.session.active().is_some_and(|s| adjust_editors::is_gray(s.doc.mode))
}

/// What the kind's editor reads, built from the app once per opening and again when the target changes: the
/// histograms of the target's pixels (a mask or saved channel's curve edits that plane's own levels, so its histogram
/// describes it, not the colour layers beneath, #5335), the tone channels (none for an `unresolved` target), swatches and gradient presets.
fn build_setup(app: &mut PhotocraftApp, kind: &str, target: &ToneTarget, mode: ColorMode, unresolved: bool) -> EditorSetup {
    let space = photocraft_engine::adjust_params::default_space(mode);
    let gray_plane = if matches!(kind, "levels" | "curves") { target.channel.as_ref().and_then(|c| c.gray) } else { None };
    let gray = gray_context(app, kind, target);
    let histograms = match target.layer {
        Some(layer) if needs_histogram(kind) => {
            let source = match gray_plane {
                Some(surface) => HistSource::Channel { layer, surface },
                None => match target.channel.as_ref().and_then(|c| c.components) {
                    Some(colors) => HistSource::Colors { layer, colors },
                    None => HistSource::Layer(layer),
                },
            };
            let counted = tone::histograms(app, source, space);
            values::panel_histograms(&counted, space, gray, target.channel.as_ref())
        }
        _ => Vec::new(),
    };
    let rgb = mode == ColorMode::Rgb;
    let pickers_unavailable =
        ![TonePicker::Black, TonePicker::Gray, TonePicker::White].iter().any(|picker| picker.supported(rgb, adjust_editors::is_gray(mode)));
    EditorSetup {
        // An unresolved Action target has no channels to show: the panel draws no Channel select.
        channels: if unresolved && matches!(kind, "levels" | "curves") {
            ToneChannels::listed(Vec::new())
        } else {
            values::tone_channels(space, gray, target.channel.as_ref())
        },
        histograms,
        pickers_unavailable,
        gradients: if kind == "gradientMap" { adjust_editors::gradient_presets(app) } else { Vec::new() },
        swatches: adjust_editors::swatches(app),
        tone: None,
        stale: false,
        reported_clipping: Arc::new(Mutex::new(None)),
    }
}

/// The kind's editor over a setup the app can replace when the target changes.
fn shared_editor(
    kind: String,
    setup: Arc<Mutex<EditorSetup>>,
) -> impl for<'b> Fn(&mut egui::Ui, &mut DialogValue, &mut AdjustDialogCx<'b>) -> Option<AdjustAsk> + Send + 'static {
    move |ui, value, cx| {
        let setup = setup.lock().ok()?.clone();
        if setup.stale {
            ui.label(tl!("The adjustment target is missing or unavailable.").to_string());
            return None;
        }
        adjust_editors::dialog_editor(&kind, setup)(ui, &mut value.value, cx)
    }
}

/// Alt+Cancel (Reset): engine defaults for the kind and the dialog's mode; Color Balance keeps the user's Preserve
/// Luminosity (Photoshop 25.4). The dialog stays open, in its context.
fn reset_target(kind: String) -> impl Fn(&DialogValue, &DialogValue) -> DialogValue + Send + 'static {
    move |_opening, current| {
        let (mode, isolated) = (current.cx.mode, current.cx.isolated.clone());
        let Ok(defaults) = photocraft_engine::adjust_params::default_for(&kind, mode) else { return current.clone() };
        let mut params = photocraft_engine::adjust_params::to_params(&defaults);
        let cx = value_cx(mode, isolated, params.clone());
        if kind == "colorBalance"
            && let Some(preserve) = values::to_params(&current.value, &cx, true).get("preserveLuminosity").cloned()
            && let Some(object) = params.as_object_mut()
        {
            object.insert("preserveLuminosity".into(), preserve);
        }
        let value = values::from_params(&kind, &params, &cx).unwrap_or_else(|_| current.value.clone());
        DialogValue { value, cx: current.cx.clone() }
    }
}

/// Opens the dialog for `command` over `recorded` parameters (and a captured Smart Filter's `smart` index and target).
fn open_hosted(app: &mut PhotocraftApp, command: &str, recorded: Value, smart: Option<(usize, Value)>, normalized: Option<Value>) -> Result<DialogId, Refused> {
    let kind = command.strip_prefix(PREFIX).filter(|k| adjust_editors::has_editor(k)).ok_or(None)?.to_owned();
    let spec = photocraft_engine::commands::find(command).ok_or(None)?;
    let seeded = seed(app, &kind, &recorded, smart.as_ref(), normalized.as_ref())?;
    // A dialog this one replaces on the host ends its Actions step with it.
    if let Some(previous) = app.adjust_open.take()
        && hosted(app, previous.id).is_some()
        && let Some(token) = crate::actions::token_of(app, previous.id)
    {
        crate::actions::abandoned(app, token);
    }
    let setup = Arc::new(Mutex::new(build_setup(app, &kind, &seeded.target, seeded.mode, seeded.unresolved)));
    let gray = gray_context(app, &kind, &seeded.target);
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let value = DialogValue { value: seeded.value, cx: seeded.cx };
    let mut dialog = HostedAdjustDialog::new(
        format!("photocraft.adjust.{kind}"),
        super::filter_dialog::title_of(spec.label),
        value,
        shared_editor(kind.clone(), setup.clone()),
    )
    .context(dynamic, gray)
    .reset_with(reset_target(kind.clone()))
    .json_form(json_form)
    .held_ok();
    if let Some(idx) = app.session.active_index() {
        dialog = dialog.canvas(crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    }
    if kind == "curves" {
        dialog = dialog.resizable(true).remember(SaveKey::new("photocraft.adjust.curves"));
    }
    let id = app.framework.dialogs.open(dialog);
    app.adjust_open = Some(OpenAdjust {
        id,
        kind,
        command: command.to_owned(),
        mode: seeded.mode,
        target: seeded.target,
        smart_filter: smart,
        unresolved: seeded.unresolved,
        picker_base: None,
        picked: None,
        setup,
        recorded,
        normalized,
    });
    Ok(id)
}

/// Opens the dialog for `command` at the settings last confirmed in it (Photoshop's ⌥ plus the
/// adjustment's shortcut, ⌥⌘L / ⌥⌘M, #8900), or at the neutral ones when there are none or they
/// don't fit the current colour mode.
pub fn open_last(app: &mut PhotocraftApp, command: &str) -> Result<DialogId, String> {
    let id = open_with_params_checked(app, command, &json!({})).map_err(|reason| reason.unwrap_or_else(|| refusal(app, command)))?;
    let kind = command.strip_prefix(PREFIX).unwrap_or_default();
    let Some(open) = app.adjust_open.as_ref().filter(|open| open.id == id) else { return Ok(id) };
    if matches!(kind, "levels" | "curves") && open.unresolved {
        return Ok(id);
    }
    let mode = open.mode;
    let Some(mut last) = last_confirmed(app, kind) else { return Ok(id) };
    // Recall settings in the current plane's space, independently of the target they came from.
    if matches!(kind, "levels" | "curves") && mode == ColorMode::Grayscale {
        last = photocraft_engine::adjust_params::gray_tone_params(&last);
    }
    let Ok(adjustment) = photocraft_engine::adjust_params::from_params(kind, &last, None, mode) else { return Ok(id) };
    if matches!(
        &adjustment,
        photocraft_doc::Adjustment::Levels { space, .. } | photocraft_doc::Adjustment::Curves { space, .. }
            if *space != photocraft_engine::adjust_params::default_space(mode)
    ) {
        return Ok(id);
    }
    let mut restored = photocraft_engine::adjust_params::to_dialog_params(&adjustment);
    // The target `open` captured, as `remember` reads it: a viewed layer mask wins over the
    // Channels panel's colour set, so its settings are restored as saved.
    let Some(base) = hosted(app, id).map(|dialog| dialog.value().cx.base.clone()) else { return Ok(id) };
    let captured_set = combined_lab_target(&base);
    if mode == ColorMode::Lab && matches!(kind, "levels" | "curves") && captured_set {
        // The parser validates all records, but ordinary Lab serialization aliases the master
        // to Lightness. A partial set keeps the separately saved combined record.
        let neutral = photocraft_engine::adjust_params::default_for(kind, ColorMode::Grayscale)
            .ok()
            .map(|a| photocraft_engine::adjust_params::to_params(&a))
            .unwrap_or(Value::Null);
        let flagged = last.get("__combinedLab").and_then(Value::as_bool) == Some(true);
        let combined = if flagged { &last } else { &neutral };
        // The editor reads the combined record only when flagged, as the colour-set seed flags it.
        restored["__combinedLab"] = json!(flagged);
        for key in ["points", "inBlack", "inWhite", "gamma", "outBlack", "outWhite"] {
            if let Some(value) = combined.get(key) {
                restored[key] = value.clone();
            }
        }
    }
    if let Value::Object(object) = &mut restored {
        object.extend(addressing(&base));
    }
    seed_params(app, id, &restored);
    Ok(id)
}

/// `params` of the adjustment `command` with the keys the dialog would otherwise fill in wrongly:
/// a recorded Photo Filter that names only its `filter` gets that preset's `color`, not the
/// dialog's neutral one. Every recorded value is kept as it was (a round trip through the engine
/// would rewrite them: float colours as hex, a pencil curve smoothed again). `None` when nothing
/// needs adding, or for params the engine rejects.
pub(crate) fn complete_params(app: &PhotocraftApp, command: &str, params: &Value) -> Option<Value> {
    let kind = command.strip_prefix(PREFIX).filter(|k| *k == "photoFilter")?;
    // A `null` colour reads as none, as the engine reads it.
    let recorded = params.as_object().filter(|p| p.contains_key("filter") && p.get("color").is_none_or(Value::is_null))?;
    let adjustment = photocraft_engine::adjust_params::from_params(kind, params, None, app.session.active().map_or(ColorMode::Rgb, |s| s.doc.mode)).ok()?;
    let color = photocraft_engine::adjust_params::to_params(&adjustment).get("color").cloned()?;
    let mut done = recorded.clone();
    done.insert("color".into(), color);
    Some(Value::Object(done))
}

/// Whether the dialog's captured target is a set of several Lab components, whose Levels and
/// Curves settings are kept as a separate combined record.
fn combined_lab_target(params: &Value) -> bool {
    photocraft_engine::color_channels::explicit(params, 3).ok().flatten().is_some_and(|c| c.len() > 1)
}

/// Remember an adjustment dialog's settings after a successful OK, for [`open_last`]: the engine parameters without
/// the addressing keys, in the saved tier. Editing a smart filter's settings is not a use of the dialog's own.
fn remember(app: &PhotocraftApp, ctx: &egui::Context, open: &OpenAdjust, cx: &ValueCx, params: &Value) {
    if open.smart_filter.is_some() {
        return;
    }
    let mut settings = params.clone();
    if let Some(object) = settings.as_object_mut() {
        for key in ["target", "layer", "document"] {
            object.remove(key);
        }
    }
    if app.session.active().is_some_and(|st| st.doc.mode == ColorMode::Lab)
        && combined_lab_target(&cx.base)
        && matches!(open.kind.as_str(), "levels" | "curves")
    {
        settings["__combinedLab"] = json!(true);
    }
    set_saved_value(ctx, &last_key(&open.kind), Some(settings));
}

/// Validate tone edits before the Smart Filter renderer uses them for preview or commit: the parameters the edit
/// sends, or the engine's refusal of them. `edited` is the dialog's [`edited`](FrameworkDialog::edited).
pub(crate) fn smart_filter_params(app: &PhotocraftApp, open: &OpenAdjust, value: &DialogValue, edited: bool) -> Result<Value, String> {
    let params = value.params(edited);
    if matches!(open.kind.as_str(), "levels" | "curves") {
        photocraft_engine::adjust_params::from_params(&open.kind, &params, None, curve_mode(app, &open.kind, &open.target))
            .map_err(|error| error.to_string())?;
    }
    Ok(params)
}

/// The editor's asks since the last pass, answered in order. Auto reads the layer (or the smart filter's input) the
/// dialog edits and writes the weights into the value; a colour well opens the Color Picker as the dialog's child; the
/// colour it answers with goes into the value.
fn answer_asks(app: &mut PhotocraftApp, id: DialogId) {
    let Some(asks) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id).map(HostedAdjustDialog::take_asks) else { return };
    for ask in asks {
        match ask {
            AdjustAsk::Auto => {
                let Some(open) = app.adjust_open.as_ref().filter(|open| open.id == id) else { continue };
                // A Smart Filter's Auto reads what that filter edits, not the filtered result.
                let (layer, filter) = (open.target.layer, open.smart_filter.as_ref().and_then(|(index, _)| u64::try_from(*index).ok()));
                match adjust_editors::black_white_auto(app, layer, filter) {
                    Ok(Value::Object(weights)) => {
                        let mut params = open_params(app).unwrap_or(Value::Null);
                        if let Some(object) = params.as_object_mut() {
                            object.extend(weights);
                        }
                        apply_params(app, id, &params);
                    }
                    Ok(_) => {}
                    Err(error) => app.ui.status = error,
                }
            }
            AdjustAsk::ImportGradients => crate::preset_files_ui::import_gradients_dialog(app),
            AdjustAsk::PickColor(request) => {
                crate::color_picker_ui::open_for_request(app, id, request);
            }
        }
    }
    while let Some((key, color)) = crate::color_picker_ui::answer_for(app, id) {
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) {
            adjust_editors::answer_color(&mut dialog.value_mut().value, &key, color);
        }
    }
}

/// Follows the target while the dialog is open: seeds the value again once an unresolved target resolves, and rebuilds
/// what the editor reads (histograms, channels) when the layer or isolated plane changed (automation retargets it).
fn follow_target(app: &mut PhotocraftApp, id: DialogId) {
    let Some(open) = app.adjust_open.as_ref().filter(|open| open.id == id) else { return };
    let resolved = target_context(app, open.smart_filter.as_ref(), &open.recorded);
    // A Smart Filter dialog whose target another document took the front from, or an Action's dialog whose recorded
    // target does not resolve, shows no controls until the target returns.
    let stale = (open.smart_filter.is_some() || open.unresolved) && resolved.is_err();
    if let Ok(mut shared) = open.setup.lock()
        && shared.stale != stale
    {
        shared.stale = stale;
    }
    if open.unresolved {
        if resolved.is_err() {
            return;
        }
        let Ok(seeded) = seed(app, &open.kind, &open.recorded, open.smart_filter.as_ref(), open.normalized.as_ref()) else { return };
        let kind = open.kind.clone();
        let setup = build_setup(app, &kind, &seeded.target, seeded.mode, false);
        let Some(open) = app.adjust_open.as_mut() else { return };
        if let Ok(mut shared) = open.setup.lock() {
            *shared = setup;
        }
        (open.mode, open.target, open.unresolved, open.picker_base, open.picked) = (seeded.mode, seeded.target, false, None, None);
        if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) {
            dialog.seed(DialogValue { value: seeded.value, cx: seeded.cx });
        }
        return;
    }
    let Ok(target) = resolved else { return };
    if target.layer == open.target.layer && target.channel == open.target.channel {
        return;
    }
    let (kind, smart_mode) = (open.kind.clone(), curve_mode(app, &open.kind, &target));
    let setup = build_setup(app, &kind, &target, smart_mode, false);
    let Some(open) = app.adjust_open.as_mut() else { return };
    if let Ok(mut shared) = open.setup.lock() {
        *shared = setup;
    }
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) {
        let mut shown = dialog.value().clone();
        shown.cx = value_cx(smart_mode, target.channel.clone(), shown.cx.base.clone());
        dialog.seed(shown);
    }
    (open.mode, open.target, open.picker_base, open.picked) = (smart_mode, target, None, None);
}

/// What OK on the open dialog runs, `(command, its params, the dialog's params)`: the adjustment command on the dialog's
/// value, or for a Smart Filter edit the command that replaces the captured filter. OK and the CPU proxy preview
/// (`canvas::ensure_filter_preview`) both take it from here, so the preview is what OK commits.
pub(crate) fn confirmed_command(app: &PhotocraftApp, open: &OpenAdjust, value: &DialogValue, edited: bool) -> Result<(String, Value, Value), String> {
    let params = match open.smart_filter {
        Some(_) => smart_filter_params(app, open, value, edited)?,
        None => value.params(edited),
    };
    let captured = open.smart_filter.as_ref().map_or(Value::Null, |(_, target)| target.clone());
    let (command, run_params) = crate::smart_ui::confirm_command(app, &captured, open.command.clone(), params.clone())?;
    Ok((command, run_params, params))
}

/// Authorizes and runs the confirmed command (one history step), then remembers the settings.
fn run_confirmed(app: &mut PhotocraftApp, ctx: &egui::Context, open: &OpenAdjust, value: &DialogValue, edited: bool) -> Result<(), String> {
    let (command, run_params, params) = confirmed_command(app, open, value, edited)?;
    crate::actions::authorize_panel_preview(app, open.id, &open.command, &params)?;
    // The editor Record Again opened answers the step it was opened for, not a new run of the command.
    if crate::actions::record_again::owns_dialog(app, open.id) {
        crate::actions::record_again::confirm(app, open.id, &command, run_params)?;
    } else {
        crate::actions::run_step_dialog(app, open.id, &command, run_params)?;
    }
    remember(app, ctx, open, &value.cx, &params);
    Ok(())
}

/// The hooks of a playing Action around a confirmed step dialog: the action continues after the command. A refused
/// command keeps the dialog open and the step paused: the user can change the values and press OK again, or cancel.
fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context, open: &OpenAdjust, value: &DialogValue, edited: bool) -> Result<(), String> {
    // Taken before the command runs: the step dialog's playback is looked up by the dialog's id.
    let token = crate::actions::token_of(app, open.id);
    run_confirmed(app, ctx, open, value, edited)?;
    match token {
        Some(token) => crate::actions::resume(app, token),
        None => Ok(()),
    }
}

/// The held OK of the dialog `id`: runs its command and answers the dialog. A refusal keeps the dialog open, showing
/// why, with the adjustment still open; an accepted OK ends it, and the dialog closes at its next pass.
fn answer_held(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) {
    let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) else { return };
    let edited = dialog.edited();
    let Some(value) = dialog.take_ok() else { return };
    let Some(open) = app.adjust_open.take() else { return };
    let answer = confirm(app, ctx, &open, &value, edited);
    if answer.is_err() {
        app.adjust_open = Some(open);
    }
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id) {
        dialog.answer_ok(answer.map_err(Label::from));
    }
}

/// The result pump's entry for the adjustment dialog (`dialogs::run`, once per pass): answers the editor's asks,
/// follows the target, runs the command of a held OK (one history step) and answers the dialog with it, and turns the
/// host's Cancel into the Actions step abandoned. A Cancel that arrives while the dialog is still open is the one an
/// opening of the same kind replaced: that step ended when the new dialog opened.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(id) = app.adjust_open.as_ref().map(|open| open.id)
        && hosted(app, id).is_some()
    {
        answer_asks(app, id);
        follow_target(app, id);
        answer_held(app, ctx, id);
    }
    let Some((result, _)) = app.framework.dialogs.take_keyed::<HostedAdjustDialog>() else { return };
    // The accepted OK's command ran when it was held (`answer_held`), which ended the open adjustment.
    let Some(id) = app.adjust_open.as_ref().map(|open| open.id) else { return };
    let still_open = hosted(app, id).is_some();
    match result {
        DialogResult::Ok(_) => {}
        _ if still_open => {}
        _ => {
            if let Some(token) = crate::actions::token_of(app, id) {
                crate::actions::stopped(app, token);
            }
            app.adjust_open = None;
        }
    }
}

#[cfg(test)]
mod tests;
