//! Per-filter blending and shared target validation for Smart Filter editors.
//! Preview and commit dispatch the existing engine commands against the captured filter.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::value_drag::SliderScale;
use photocraft_color::BlendMode;
use photocraft_doc::{DocId, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::blend_options::{blend_mode, blend_param};
use crate::dialogs::params::{captured, open_param};

/// The command of the Blending Options dialog, which is also its dialog key.
pub const COMMAND: &str = "layer.smartFilter.blendingOptions";

/// Identify the Smart Filter at `layer`/`index` (document, revision, filter) so a stale dialog can be told apart from a current one.
fn snapshot(app: &PhotocraftApp, layer: LayerId, index: usize) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    if app.session.job_on(st.doc.id).is_some() {
        return Err(tl!("Wait for the current job to finish.").into());
    }
    let sm = match &st.doc.layer(layer).ok_or("Smart Object no longer exists")?.content {
        LayerContent::Smart(sm) => sm,
        _ => return Err("the layer is not a Smart Object".into()),
    };
    let filter = sm.smart_filters.get(index).ok_or("Smart Filter no longer exists")?;
    Ok(json!({"doc": st.doc.id.0, "revision": st.revision, "layer": layer.0, "index": index, "filter": filter}))
}

/// The Smart Filter target an editor captures at open: the filter's snapshot. A closed document cancels the
/// dialogs that hold its target, so the snapshot alone tells a stale target from a current one.
pub(crate) fn target(app: &PhotocraftApp, layer: LayerId, index: usize) -> Result<Value, String> {
    snapshot(app, layer, index)
}

/// Whether `captured` is a Smart Filter target made by [`target`].
pub(crate) fn is_target(captured: &Value) -> bool {
    ["doc", "revision", "layer", "index", "filter"].iter().all(|k| captured.get(k).is_some())
}

/// The document a captured Smart Filter target names: the family's dialogs close with it.
pub(crate) fn document(captured: &Value) -> Option<DocId> {
    if !is_target(captured) {
        return None;
    }
    captured.get("doc").and_then(Value::as_u64).map(DocId)
}

/// Revision and filter identity catch every edit made since the target was captured.
pub(crate) fn validate_target(app: &PhotocraftApp, target: &Value) -> Result<(LayerId, usize), String> {
    let layer = LayerId(target.get("layer").and_then(Value::as_u64).ok_or("invalid Smart Filter target")?);
    let index = target.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("invalid Smart Filter index")?;
    if snapshot(app, layer, index)? != *target {
        return Err(tl!("The document or Smart Filter changed. Reopen its dialog.").into());
    }
    Ok((layer, index))
}

/// Open the Blending Options dialog for one Smart Filter, starting from its recorded mode and opacity (shown as a percentage).
pub fn open(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<DialogId, String> {
    let target = self::target(app, layer, index)?;
    let st = app.session.active().ok_or("no document open")?;
    let Some(LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else { return Err("the layer is not a Smart Object".into()) };
    let filter = sm.smart_filters.get(index).ok_or("Smart Filter no longer exists")?;
    let name = crate::smart_ui::filter_label(filter).to_string();
    let title = Label::from(crate::i18n::fmt(tl!("Blending Options ({filter})"), &[("filter", name.as_str())]));
    let params = vec![
        blend_param("blend", tl!("Mode"), false, filter.blend),
        Param::read_only_text("blendNote", "", tl!("Dissolve is approximated: it blends without Photoshop's dither."))
            .visible_when("blend", BlendMode::Dissolve.key().to_string()),
        Param::new("opacity", tl!("Opacity"), ParamKind::range(0.0, 100.0, 100.0, SliderScale::Linear)),
    ];
    let mut fields = DialogFields::new();
    fields.set("blend", filter.blend.key().to_string());
    fields.set("opacity", f64::from(filter.opacity) * 100.0);
    app.filter_preview = None;
    Ok(open_param(app, COMMAND, fields, target, move |f| ParamDialog::new(COMMAND, title.clone(), params.clone()).values(Bind::bound(f)).previews(true)))
}

/// Whether the dialog `key` is the Blending Options dialog rather than another Smart Filter editor.
pub(crate) fn owns(key: &str) -> bool {
    key == COMMAND
}

/// UI opacity is always percent; the engine gets a fraction, including 1% = 0.01.
fn blend_params(fields: &DialogFields) -> Result<Value, String> {
    let opacity = fields.number("opacity").ok_or(tl!("Opacity must be between 0% and 100%."))?;
    Ok(json!({"blend": blend_mode(fields, "blend").label(), "opacity": opacity / 100.0}))
}

/// The command a confirmed Blending Options dialog runs on the validated `target`.
pub(crate) fn blend_command(app: &PhotocraftApp, target: &Value, fields: &DialogFields) -> Result<(String, Value), String> {
    let (layer, index) = validate_target(app, target)?;
    let mut params = blend_params(fields)?;
    if let Some(p) = params.as_object_mut() {
        p.insert("layer".into(), json!(layer.0));
        p.insert("index".into(), json!(index));
    }
    Ok((COMMAND.into(), params))
}

/// The command a confirmed parameter editor of a Smart Filter runs on the validated `target`: `layer.smartFilter.setParams`.
pub(crate) fn params_command(app: &PhotocraftApp, target: &Value, command: &str, params: Value) -> Result<(String, Value), String> {
    let (layer, index) = validate_target(app, target)?;
    // An adjustment's dialog opens on all the recorded settings and may remove keys, so OK
    // replaces them: merged, a removed one stays stored and can override the edit (a stale
    // Channel Mixer `gray` is read over `red`, a stale Color Lookup `data` over `lut`).
    let replace = command.starts_with("image.adjustments.");
    Ok(("layer.smartFilter.setParams".into(), json!({"layer": layer.0, "index": index, "params": params, "replace": replace})))
}

/// OK is held until the captured target still names the filter.
pub(crate) fn check(app: &mut PhotocraftApp, result: &ParamResult) -> Result<(), Label> {
    let captured = captured(app, &result.key);
    validate_target(app, &captured).map(|_| ()).map_err(Label::from)
}

/// Run the confirmed blending on the captured filter.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let captured = captured(app, &result.key);
    let done = blend_command(app, &captured, &result.fields).and_then(|(command, params)| app.run(&command, params));
    if let Err(e) = done {
        crate::notices::error(app, e);
    }
}

#[cfg(test)]
pub(crate) mod tests;
