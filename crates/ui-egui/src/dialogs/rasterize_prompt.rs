//! "Rasterize?" prompt: painting the pixels of a type, shape, Smart Object or fill layer with a
//! pixel tool asks first, as Photoshop does ("This type layer must be rasterized before
//! proceeding. Its text will no longer be editable. Rasterize the type?"), instead of failing
//! with a status-bar message.
//!
//! The prompt is a `StandardPrompt` on the dialog host (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). OK runs the matching `layer.rasterize.*` command and nothing
//! else: the click that opened the prompt is not replayed, as in Photoshop (#8806), so the next
//! click paints. Cancel does nothing. The Patch tool's prompt is the exception (#12121): its drag is
//! over when that prompt opens, so OK also applies the `paint.patch` the drag worked out.
//!
//! A filter on a type layer asks the same question (#4303), with Photoshop's Convert to Smart
//! Object / Rasterize / Cancel: the layer is converted or rasterized, then the filter is chosen
//! again from the menu (its dialog, window or mode opens, or it runs), a smart filter on the Smart
//! Object. `ui.dialog.set` with `"choice": "alternate"` picks the conversion.

use craft::ui::dialog::{DialogId, PromptAnswer, PromptChoice, StandardPrompt};
use serde_json::{Value, json};

use super::alerts;
use crate::PhotocraftApp;
use crate::state::Tool;

/// A layer kind that must be rasterized before its pixels can be painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Type,
    Shape,
    SmartObject,
    Fill,
}

/// What the prompt asked about: rasterize `layer` (a `kind` layer); with `filter`, then run that
/// filter and its params on it; with `patch`, then run `paint.patch` with those params (#12121).
// Whether Convert is offered is fixed when the prompt opens (the button exists or not), so the tag does not carry it.
pub(crate) struct Rasterize {
    kind: Kind,
    /// The document `layer` was taken from. A document read back from a file keeps the layer ids
    /// stored in it, so one number can name a layer of another open document; see
    /// [`active_doc`].
    doc: u64,
    layer: u64,
    filter: Option<(String, Value)>,
    patch: Option<Value>,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Type, Kind::Shape, Kind::SmartObject, Kind::Fill];

    /// The command that rasterizes it.
    pub fn command(self) -> &'static str {
        match self {
            Kind::Type => "layer.rasterize.type",
            Kind::Shape => "layer.rasterize.shape",
            Kind::SmartObject => "layer.rasterize.smartObject",
            Kind::Fill => "layer.rasterize.fillContent",
        }
    }

    /// Photoshop's question.
    pub fn message(self) -> &'static str {
        match self {
            Kind::Type => "This type layer must be rasterized before proceeding. Its text will no longer be editable. Rasterize the type?",
            Kind::Shape => "This shape layer must be rasterized before proceeding. It will no longer be editable as a shape. Rasterize the shape?",
            // As measured in the reference application (#11899): lower-case "smart object".
            Kind::SmartObject => {
                "This smart object must be rasterized before proceeding. Edit Contents will no longer be available. Rasterize the smart object?"
            }
            Kind::Fill => "The fill for this layer must be rasterized before proceeding. It will no longer be editable as a fill. Rasterize the fill?",
        }
    }
}

/// Does `tool` paint the layer's pixels (so a vector or Smart Object layer must be rasterized)?
pub fn paints_pixels(app: &PhotocraftApp, tool: Tool) -> bool {
    tool.is_brushlike()
        || matches!(tool, Tool::PaintBucket | Tool::MagicEraser | Tool::RedEye)
        || (tool == Tool::Gradient && app.ui.tool_options.gradient_classic)
}

/// The active layer's kind when painting its pixels needs rasterizing first: the tool targets
/// the layer's pixels (not its mask, an alpha channel or the Quick Mask) and the layer is a
/// type, shape, Smart Object or fill layer that isn't locked.
pub fn needed(app: &PhotocraftApp) -> Option<(Kind, photocraft_doc::LayerId)> {
    if crate::canvas::paint_target(app) != json!("pixels") {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    let l = st.doc.layer(id)?;
    let locks = st.doc.effective_locks(id);
    if locks.all || locks.pixels {
        return None;
    }
    use photocraft_doc::LayerContent as C;
    let kind = match &l.content {
        C::Text(_) => Kind::Type,
        C::Shape(_) => Kind::Shape,
        C::Smart(_) => Kind::SmartObject,
        C::Fill(_) => Kind::Fill,
        _ => return None,
    };
    Some((kind, id))
}

/// A tool's pointer press: when it would paint a layer that must be rasterized first, open the
/// prompt, or drop the press (the Remove tool on a type layer), and return true (the tool must not
/// see the press). The Remove tool on a Smart Object layer is the one press that goes on without an
/// answer: its fill lands on a new layer above the smart object, which keeps its content (#12555).
pub fn intercept(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64, pressure: f32) -> bool {
    if !paints_pixels(app, tool) {
        return false;
    }
    let Some((kind, layer)) = needed(app) else { return false };
    match (tool, kind) {
        // The reference's Remove tool ignores a type layer: no prompt and no stroke (#12553).
        (Tool::Remove, Kind::Type) => return true,
        (Tool::Remove, Kind::SmartObject) => return false,
        _ => {}
    }
    open(app, kind, layer, tool, [x, y, f64::from(pressure)]);
    true
}

/// The document a prompt is opened for, as the id of the active document. The tag carries numbers,
/// and `Session::add_document` only re-picks a document id, so two documents read from files can hold
/// the same layer id (`photocraft_format` keeps the ids stored in a native file). `u64::MAX` means no
/// document: nothing matches it, so the answer refuses. See [`stale_document`].
fn active_doc(app: &PhotocraftApp) -> u64 {
    app.session.active().map_or(u64::MAX, |st| st.doc.id.0)
}

/// The refusal the writing answers share. The tag names its target by number, and a document read
/// back from a file keeps the layer ids stored in it, so that number can name a layer of another
/// open document: when the active document is not the one the question was opened for, the answer
/// writes in neither of them (#12121).
fn stale_document(app: &PhotocraftApp, doc: u64) -> Option<String> {
    (active_doc(app) != doc).then(|| "the document this question was about is no longer the active document, so nothing was rasterized".to_owned())
}

/// Opens the same question for a Patch drag on a `layer` that must be rasterized first: OK rasterizes
/// it and then runs `paint.patch` with `params`, the params the finished drag had worked out, so the
/// patch lands on the rasterized layer (#12121). Cancel does nothing.
pub fn open_patch(app: &mut PhotocraftApp, kind: Kind, layer: photocraft_doc::LayerId, params: Value) -> DialogId {
    let prompt = StandardPrompt::custom(tl!(kind.message()).to_owned())
        .title(tl!("PhotoCraft").to_owned())
        .button(PromptChoice::Proceed, tl!("OK").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .tag(Rasterize { kind, doc: active_doc(app), layer: layer.0, filter: None, patch: Some(params) });
    alerts::open_prompt(app, prompt).unwrap_or_else(|open| open)
}

/// Opens the prompt for `layer`; OK rasterizes it, and the id of the prompt already open comes back
/// when there is one. `tool` and `at` (x, y, pressure) name the press that opened it; the press is
/// never replayed, so they change nothing.
pub fn open(app: &mut PhotocraftApp, kind: Kind, layer: photocraft_doc::LayerId, _tool: Tool, _at: [f64; 3]) -> DialogId {
    let prompt = StandardPrompt::custom(tl!(kind.message()).to_owned())
        .title(tl!("PhotoCraft").to_owned())
        .button(PromptChoice::Proceed, tl!("OK").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .tag(Rasterize { kind, doc: active_doc(app), layer: layer.0, filter: None, patch: None });
    alerts::open_prompt(app, prompt).unwrap_or_else(|open| open)
}

/// The active type layer when `id` is a filter that only needs it rasterized: the engine greys it
/// for not being a pixel layer, and the layer is not locked. The flag says whether the filter also
/// runs on a Smart Object, as a smart filter (Vanishing Point doesn't).
pub fn filter_needs(app: &PhotocraftApp, id: &str) -> Option<(photocraft_doc::LayerId, bool)> {
    if !id.starts_with("filter.") {
        return None;
    }
    let st = app.session.active()?;
    let layer = st.active_layer?;
    if !matches!(st.doc.layer(layer)?.content, photocraft_doc::LayerContent::Text(_)) {
        return None;
    }
    let locks = st.doc.effective_locks(layer);
    if locks.all || locks.pixels {
        return None;
    }
    // Last Filter checks the layer before the journal: with no filter to repeat it stays grey.
    if id == "filter.lastFilter" && photocraft_engine::filters::last_filter(&app.session).is_none() {
        return None;
    }
    // The engine's reason for a type layer: "filters need a pixel layer (active layer is a Type
    // layer)" from filters::has_filterable_layer, which also takes Smart Objects, or a filter's own
    // wording ("Vanishing Point needs a pixel layer (…)", "active layer is a Type layer, not a pixel
    // layer"). Any other reason (a job running) keeps the item grey.
    let reason = app.session.disabled_reason(id)?;
    (reason.contains("active layer is a Type layer") && reason.contains("pixel layer")).then(|| (layer, reason.starts_with("filters need a pixel layer")))
}

/// A filter chosen on a type layer: open the prompt instead and return its answer to the caller.
pub fn intercept_filter(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let (layer, smart) = filter_needs(app, id)?;
    let mut prompt = StandardPrompt::custom(tl!(Kind::Type.message()).to_owned())
        .title(tl!("PhotoCraft").to_owned())
        .button(PromptChoice::Proceed, tl!("Rasterize").to_owned());
    if smart {
        prompt = prompt.button(PromptChoice::Alternate, tl!("Convert to Smart Object").to_owned());
    }
    let prompt = prompt.button(PromptChoice::Cancel, tl!("Cancel").to_owned()).tag(Rasterize {
        kind: Kind::Type,
        doc: active_doc(app),
        layer: layer.0,
        filter: Some((id.to_owned(), params.clone())),
        patch: None,
    });
    let dialog = alerts::open_prompt(app, prompt).unwrap_or_else(|open| open);
    Some(Ok(json!({"dialog": dialog.value()})))
}

/// The `PROMPTS` entry: Proceed rasterizes, Alternate converts to a Smart Object (the filter prompt
/// only), Cancel does nothing. The filter of a filter prompt runs at once, as the menu would, now
/// that the layer can take it. A patch prompt's Proceed runs `paint.patch` on the layer it just
/// rasterized (#12121). Every answer that writes first refuses when the active document is not the
/// one the question was opened for ([`stale_document`]). `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(Rasterize { kind, doc, layer, filter, patch }) = answer.take_tag::<Rasterize>() else { return false };
    let result = match (answer.choice, filter, patch) {
        (PromptChoice::Proceed, None, None) => rasterize(app, kind, doc, layer),
        // The Patch drag had already ended when this prompt opened, so its params re-run here.
        (PromptChoice::Proceed, None, Some(params)) => rasterize_then_patch(app, kind, doc, layer, params),
        (PromptChoice::Proceed | PromptChoice::Alternate, Some((filter, params)), None) => {
            convert_for_filter(app, ctx, answer.choice == PromptChoice::Alternate, &filter, params, doc, layer)
        }
        _ => return true,
    };
    alerts::report(app, result);
    true
}

/// OK: rasterize the layer. The click that opened the prompt does not paint.
fn rasterize(app: &mut PhotocraftApp, kind: Kind, doc: u64, layer: u64) -> Result<Value, String> {
    if let Some(why) = stale_document(app, doc) {
        return Err(why);
    }
    let revision = app.session.active().map(|st| st.revision);
    let r = app.run(kind.command(), json!({ "layer": layer }))?;
    // A painting tool's type prompt names its step as the reference does (#12888); Layer ›
    // Rasterize › Type keeps "Rasterize Type". Only a step this run added is renamed.
    if kind == Kind::Type
        && let Some(st) = app.session.active_mut().filter(|st| Some(st.revision) != revision)
    {
        st.history.set_current_label("Rasterize Layer");
    }
    // The press that opened the prompt is not replayed: Photoshop only converts the layer, the
    // next click paints (#8806).
    Ok(json!({ "rasterized": r }))
}

/// OK on the Patch tool's prompt: rasterize the layer, then apply the patch the drag had worked out.
/// A patch that fails afterwards leaves the layer rasterized and reports the error in the status bar.
fn rasterize_then_patch(app: &mut PhotocraftApp, kind: Kind, doc: u64, layer: u64, params: Value) -> Result<Value, String> {
    // `rasterize` answers for the document the question was opened for, so a patch can never follow
    // it into another document whose layers happen to carry the same ids (#12121).
    let rasterized = rasterize(app, kind, doc, layer)?;
    // The patch params name no layer: run as they stand, its pixels go to whatever is active when OK
    // comes. The rasterize above settled the layer the question was about, so a different active
    // layer now refuses the patch instead of painting somewhere else (#12121).
    if app.session.active().and_then(|st| st.active_layer).map(|id| id.0) != Some(layer) {
        return Err("the layer this question was about is no longer the active layer, so the patch was not applied".into());
    }
    let patch = app.run("paint.patch", params)?;
    Ok(json!({ "rasterized": rasterized, "patched": patch }))
}

/// The filter prompt's answer: convert (`smart`) or rasterize the layer, then run the filter.
fn convert_for_filter(app: &mut PhotocraftApp, ctx: &egui::Context, smart: bool, filter: &str, params: Value, doc: u64, layer: u64) -> Result<Value, String> {
    if let Some(why) = stale_document(app, doc) {
        return Err(why);
    }
    let command = if smart { "layer.smartObjects.convertToSmartObject" } else { Kind::Type.command() };
    let converted = app.run(command, json!({ "layer": layer }))?;
    crate::menus::invoke(app, ctx, filter, params)?;
    Ok(json!({ "converted": converted, "filter": filter }))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../rasterize_prompt_text_tests.rs"]
mod rasterize_prompt_text_tests;

#[cfg(test)]
#[path = "rasterize_prompt_fill_text_tests.rs"]
mod rasterize_prompt_fill_text_tests;

#[cfg(test)]
#[path = "../rasterize_prompt_history_name_tests.rs"]
mod rasterize_prompt_history_name_tests;

#[cfg(test)]
#[path = "../tools/remove_type_layer_tests.rs"]
mod remove_type_layer_tests;

#[cfg(test)]
#[path = "../tools/remove_smart_object_tests.rs"]
mod remove_smart_object_tests;
