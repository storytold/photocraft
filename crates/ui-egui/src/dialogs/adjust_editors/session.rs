//! The Properties panel's editor of an adjustment layer: the layer's value over the kind's editor
//! closure, its live preview, its history steps and Reset.

use std::hash::{Hash as _, Hasher as _};

use craft::behavior::EditEvent;
use craft::color_ui::ui::ColorPickerDialog;
use craft::imaging::ui::adjust::{AdjustContext, AdjustDialogCx, AdjustSession, ColorBalanceTone, LevelsClip, TonePicker};
use craft::theme::Srgba;
use craft::ui::dialog::DialogId;
use photocraft_doc::{Adjustment, ColorMode, LayerId};
use photocraft_engine::adjust_params;
use serde_json::{Value, json};

use super::values::{self, ValueCx};
use super::{AdjustAsk, AdjustValue, EditorSetup, answer_color, black_white_auto, editor, gradient_presets, is_gray, swatches};
use crate::PhotocraftApp;
use crate::color_picker_ui::PickWindow;
use crate::theme::Tokens;
use crate::tone::{self, HistSource};

/// Prefix of the `coalesce` keys of gesture edits (`history_keys::coalesce` appends the gesture).
const GESTURE_KEY: &str = "properties-adjustment";

/// What the Properties editor keeps for the layer it shows; built anew when another layer shows.
pub(crate) struct PropertiesAdjust {
    /// The layer the record belongs to.
    pub layer: LayerId,
    /// The settings the panel found, for Color Balance's Photoshop-style Reset.
    pub session: AdjustSession<AdjustValue>,
    /// The shown entry of the Channel select (an index into `values::tone_channels`).
    pub channel: usize,
    /// The Levels clipping display.
    pub clipping: Option<LevelsClip>,
    /// A tone for Color Balance to show, given once: the Reset to defaults shows Midtones again.
    pub tone: Option<ColorBalanceTone>,
    /// The Color Picker a colour well opened, while it is open.
    pub pick: Option<ColorPick>,
}

/// A colour well's open Color Picker.
pub(crate) struct ColorPick {
    dialog: DialogId,
    /// The key the editor asked with (`answer_color` finds the field by it).
    key: String,
    /// The colour the picker opened with: an OK that answers it is no edit (a colour the value holds in
    /// finer steps than the picker's would otherwise be rewritten).
    opened: Option<Srgba>,
    /// The value the colour is written into.
    before: AdjustValue,
}

impl PropertiesAdjust {
    fn new(layer: LayerId) -> Self {
        Self { layer, session: AdjustSession::new(), channel: 0, clipping: None, tone: None, pick: None }
    }
}

/// Kinds whose Properties edits follow Photoshop's history and Reset (measured for Color Balance,
/// Photoshop 25.4): consecutive edits of the layer make one history step, and Reset first returns
/// to the settings the layer had when the panel started showing it.
fn photoshop_session(kind: &str) -> bool {
    kind == "colorBalance"
}

/// Kinds that draw a histogram behind their controls.
fn has_histogram(kind: &str) -> bool {
    matches!(kind, "levels" | "curves" | "threshold")
}

/// The `coalesce` key of a session kind's edits: one history step while they follow each other.
/// It differs in form from the gesture keys, which end in a gesture number.
fn layer_key(id: LayerId) -> String {
    format!("{GESTURE_KEY}:layer-{}", id.0)
}

fn value_cx(mode: ColorMode, committed: &Value) -> ValueCx {
    ValueCx::new(mode, committed.clone())
}

/// The identity a session tracks: the document and the layer.
fn session_target(doc: u64, id: LayerId) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (doc, id.0).hash(&mut hasher);
    hasher.finish()
}

/// The Color Balance value with every tone back to 0 (Preserve Luminosity kept).
fn zeroed(mut value: AdjustValue) -> AdjustValue {
    if let AdjustValue::ColorBalance { value, .. } = &mut value {
        value.reset();
    }
    value
}

/// The Properties panel's Reset for an adjustment layer: the `layer.setAdjustment` params. A kind
/// that follows Photoshop's session ([`photoshop_session`]) goes back to the settings the panel
/// found if it edited them since (or since the last Reset), else to the defaults (Tone choice kept
/// in the first case, Midtones shown in the second); either way it is part of the layer's edit
/// step. Other kinds go to the defaults for the document's mode.
pub fn layer_reset(app: &mut PhotocraftApp, id: LayerId, adj: &Adjustment) -> Value {
    let kind = photocraft_engine::commands::adjustment_kind(adj);
    let mode = app.session.active().map_or(ColorMode::Rgb, |s| s.doc.mode);
    let committed = adjust_params::to_params(adj);
    let mut params = if photoshop_session(kind) {
        let vcx = value_cx(mode, &committed);
        match values::from_params(kind, &committed, &vcx) {
            Ok(found) => {
                let record = app.adjust_session.as_mut().filter(|r| r.layer == id);
                let value = match record {
                    Some(record) => {
                        let was_edited = record.session.is_edited();
                        let value = record.session.reset(|| zeroed(found));
                        if !was_edited {
                            record.tone = Some(ColorBalanceTone::Midtones);
                        }
                        value
                    }
                    None => zeroed(found),
                };
                values::to_params(&value, &vcx, true)
            }
            Err(_) => committed,
        }
    } else {
        adjust_params::default_for(kind, mode).map_or_else(|_| json!({}), |defaults| adjust_params::to_params(&defaults))
    };
    params["layer"] = json!(id.0);
    if photoshop_session(kind) {
        params["coalesce"] = json!(layer_key(id));
    }
    params
}

/// The Properties-panel editor of an adjustment layer: previews live (`app.live_adjust`) while a
/// control moves and commits one `layer.setAdjustment` per gesture.
///
/// Each pass applies only the edit events its own editor run reports, so a second pass in the same
/// frame repeats nothing.
pub fn layer_editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &Adjustment) {
    let t = Tokens::get(ui.ctx());
    match adj {
        Adjustment::ColorLookup { .. } => return crate::adjust_ui::color_lookup_editor(app, ui, id, adj),
        Adjustment::Invert => {
            ui.label(egui::RichText::new(tl!("Invert has no settings.")).color(t.text_faint));
            return;
        }
        Adjustment::Unsupported { psd_key, .. } => {
            let template = "This adjustment ({key}) is kept as imported and can't be edited yet.";
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!(template), &[("key", psd_key)])).color(t.text_faint));
            return;
        }
        _ => {}
    }
    let kind = photocraft_engine::commands::adjustment_kind(adj);
    let Some((doc, mode)) = app.session.active().map(|s| (s.doc.id.0, s.doc.mode)) else { return };
    let committed_params = adjust_params::to_params(adj);
    let vcx = value_cx(mode, &committed_params);
    let committed = match values::from_params(kind, &committed_params, &vcx) {
        Ok(value) => value,
        Err(err) => {
            app.ui.status = err;
            return;
        }
    };

    let mut record = match app.adjust_session.take() {
        Some(record) if record.layer == id => record,
        other => {
            // A colour well's picker of the layer shown before has nothing to answer to.
            if let Some(ColorPick { dialog, .. }) = other.and_then(|r| r.pick) {
                app.framework.dialogs.cancel_by_id(ui.ctx(), dialog);
                app.live_adjust = None;
            }
            PropertiesAdjust::new(id)
        }
    };
    let session = photoshop_session(kind);
    if session {
        record.session.track(session_target(doc, id), ui.ctx().cumulative_pass_nr(), &committed);
    }
    follow_color_pick(app, id, &mut record, &committed, &vcx);
    let mut value = match &app.live_adjust {
        Some((layer, params)) if *layer == id => values::from_params(kind, params, &vcx).unwrap_or_else(|_| committed.clone()),
        _ => committed.clone(),
    };

    let gray = is_gray(mode);
    let space = adjust_params::default_space(mode);
    let dynamic = app.session.prefs().interface.dynamic_color_sliders;
    let setup = EditorSetup {
        channels: values::tone_channels(space, gray, None),
        histograms: if has_histogram(kind) {
            values::panel_histograms(&tone::histograms(app, HistSource::BelowLayer(id), space), space, gray, None)
        } else {
            Vec::new()
        },
        gradients: if kind == "gradientMap" { gradient_presets(app) } else { Vec::new() },
        swatches: swatches(app),
        tone: record.tone.take(),
        ..Default::default()
    };
    let armed = crate::layer_picker::armed(app).filter(|(layer, _)| *layer == id).map(|(_, picker)| picker);
    let mut picker: Option<TonePicker> = armed;
    let out = {
        let run = editor(kind, setup);
        let mut cx = AdjustDialogCx::new(AdjustContext::properties(dynamic).gray(gray), &mut picker, &mut record.channel, &mut record.clipping);
        run(ui, &mut value, &mut cx)
    };
    if picker != armed {
        crate::layer_picker::arm(app, id, picker);
    }

    for event in out.edit {
        match &event {
            EditEvent::Update { value: proposed, .. } => app.live_adjust = Some((id, values::to_params(proposed, &vcx, true))),
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                let key = if session { Some(layer_key(id)) } else { crate::history_keys::coalesce(GESTURE_KEY, &event) };
                commit(app, id, &mut record, &committed, after, &vcx, key);
            }
            EditEvent::Cancel { .. } => app.live_adjust = None,
            _ => {}
        }
    }
    match out.ask {
        Some(AdjustAsk::Auto) => match black_white_auto(app, Some(id), None) {
            Ok(Value::Object(weights)) => {
                let mut params = values::to_params(&value, &vcx, true);
                if let Some(target) = params.as_object_mut() {
                    target.extend(weights);
                }
                match values::from_params(kind, &params, &vcx) {
                    Ok(auto) => commit(app, id, &mut record, &committed, &auto, &vcx, None),
                    Err(err) => app.ui.status = err,
                }
            }
            Ok(_) => {}
            Err(err) => app.ui.status = err,
        },
        Some(AdjustAsk::ImportGradients) => crate::preset_files_ui::import_gradients_dialog(app),
        Some(AdjustAsk::PickColor(request)) if record.pick.is_none() => {
            let (key, opened) = (request.key.clone(), request.color);
            // Properties has no parent dialog for `open_for_request`: the panel holds its picker itself.
            let dialog = crate::color_picker_ui::open_for_window(app, PickWindow::Properties, request);
            record.pick = Some(ColorPick { dialog, key, opened, before: value });
        }
        _ => {}
    }
    app.adjust_session = Some(record);
}

/// Runs one `layer.setAdjustment` for `value` unless it is the layer's committed value, and ends
/// the preview. `coalesce` joins the edit to the step before it.
fn commit(
    app: &mut PhotocraftApp,
    id: LayerId,
    record: &mut PropertiesAdjust,
    committed: &AdjustValue,
    value: &AdjustValue,
    vcx: &ValueCx,
    coalesce: Option<String>,
) {
    if value != committed {
        let rev = app.session.active().map(|s| s.revision);
        let mut params = values::to_params(value, vcx, true);
        params["layer"] = json!(id.0);
        if let Some(key) = coalesce {
            params["coalesce"] = json!(key);
        }
        record.session.edited();
        if let Err(err) = app.run("layer.setAdjustment", params) {
            app.ui.status = err;
        }
        tone::keep_after_commit(app, id, rev);
    }
    app.live_adjust = None;
}

/// A colour well's Color Picker while it is open: its colour previews on the layer. Once it
/// closed, OK's colour is one edit and Cancel (or OK without a colour) leaves the layer as it was.
fn follow_color_pick(app: &mut PhotocraftApp, id: LayerId, record: &mut PropertiesAdjust, committed: &AdjustValue, vcx: &ValueCx) {
    let Some(pick) = record.pick.take() else { return };
    match app.framework.dialogs.get_by_id::<ColorPickerDialog>(pick.dialog).map(ColorPickerDialog::current) {
        Some(current) => {
            if let Some(picked) = current {
                let mut preview = pick.before.clone();
                if answer_color(&mut preview, &pick.key, picked.color) {
                    app.live_adjust = Some((id, values::to_params(&preview, vcx, true)));
                }
            }
            record.pick = Some(pick);
        }
        None => match crate::color_picker_ui::answer_for_window(app, PickWindow::Properties) {
            Some((key, color)) if key == pick.key && Some(color) != pick.opened => {
                let mut answered = pick.before;
                if answer_color(&mut answered, &key, color) {
                    commit(app, id, record, committed, &answered, vcx, None);
                } else {
                    app.live_adjust = None;
                }
            }
            _ => app.live_adjust = None,
        },
    }
}
