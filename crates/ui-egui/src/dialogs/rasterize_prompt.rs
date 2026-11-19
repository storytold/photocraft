//! "Rasterize?" prompt: painting the pixels of a type, shape, Smart Object or fill layer with a
//! pixel tool asks first, as Photoshop does ("This type layer must be rasterized before
//! proceeding. Its text will no longer be editable. Rasterize the type?"), instead of failing
//! with a status-bar message.
//!
//! The prompt is a `StandardPrompt` on the dialog host (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). OK runs the matching `layer.rasterize.*` command and nothing
//! else: the click that opened the prompt is not replayed, as in Photoshop (#8806), so the next
//! click paints. Cancel does nothing.
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
/// filter and its params on it.
// Whether Convert is offered is fixed when the prompt opens (the button exists or not), so the tag does not carry it.
pub(crate) struct Rasterize {
    kind: Kind,
    layer: u64,
    filter: Option<(String, Value)>,
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
/// see the press).
pub fn intercept(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64, pressure: f32) -> bool {
    if !paints_pixels(app, tool) {
        return false;
    }
    let Some((kind, layer)) = needed(app) else { return false };
    // The reference's Remove tool ignores a type layer: no prompt and no stroke (#12553).
    if tool == Tool::Remove && kind == Kind::Type {
        return true;
    }
    open(app, kind, layer, tool, [x, y, f64::from(pressure)]);
    true
}

/// Opens the prompt for `layer`; OK rasterizes it, and the id of the prompt already open comes back
/// when there is one. `tool` and `at` (x, y, pressure) name the press that opened it; the press is
/// never replayed, so they change nothing.
pub fn open(app: &mut PhotocraftApp, kind: Kind, layer: photocraft_doc::LayerId, _tool: Tool, _at: [f64; 3]) -> DialogId {
    let prompt = StandardPrompt::custom(tl!(kind.message()).to_owned())
        .title(tl!("PhotoCraft").to_owned())
        .button(PromptChoice::Proceed, tl!("OK").to_owned())
        .button(PromptChoice::Cancel, tl!("Cancel").to_owned())
        .tag(Rasterize { kind, layer: layer.0, filter: None });
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
        layer: layer.0,
        filter: Some((id.to_owned(), params.clone())),
    });
    let dialog = alerts::open_prompt(app, prompt).unwrap_or_else(|open| open);
    Some(Ok(json!({"dialog": dialog.value()})))
}

/// The `PROMPTS` entry: Proceed rasterizes, Alternate converts to a Smart Object (the filter prompt
/// only), Cancel does nothing. The filter of a filter prompt runs at once, as the menu would, now
/// that the layer can take it. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(Rasterize { kind, layer, filter }) = answer.take_tag::<Rasterize>() else { return false };
    let result = match (answer.choice, filter) {
        (PromptChoice::Proceed, None) => rasterize(app, kind, layer),
        (PromptChoice::Proceed | PromptChoice::Alternate, Some((filter, params))) => {
            convert_for_filter(app, ctx, answer.choice == PromptChoice::Alternate, &filter, params, layer)
        }
        _ => return true,
    };
    alerts::report(app, result);
    true
}

/// OK: rasterize the layer. The click that opened the prompt does not paint.
fn rasterize(app: &mut PhotocraftApp, kind: Kind, layer: u64) -> Result<Value, String> {
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

/// The filter prompt's answer: convert (`smart`) or rasterize the layer, then run the filter.
fn convert_for_filter(app: &mut PhotocraftApp, ctx: &egui::Context, smart: bool, filter: &str, params: Value, layer: u64) -> Result<Value, String> {
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
