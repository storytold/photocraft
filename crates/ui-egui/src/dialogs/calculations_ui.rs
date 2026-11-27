//! The Calculations dialog's live canvas preview (#14530).
//!
//! The dialog is the hosted parameter form `view_cmds::dialogs` opens for the command
//! [`COMMAND`]; its Preview check box and the value's automation half come from the framework's
//! form dialog (`ParamDialog::previews`): a captioned box, on from the first pass, reported and
//! written as the `preview` field like every other value. The command itself only adds a channel,
//! so the post-OK document composites exactly as before: what the measured reference shows on the
//! canvas is not the edited document but the *result channel itself*, as a grey image over the
//! whole document, redrawn with every field. [`preview_request`] gets that picture out of the same
//! engine: the form's values run on a scratch session as the New Document result, whose one-layer
//! grayscale document is the result channel's grey rendering, and the existing filter-preview
//! pipeline draws it over the canvas (proxy factor, GPU texture and CPU path included). While the
//! dialog stands, the document, its channels and its history stay untouched; OK runs the real
//! command with the chosen Result and the preview drops.
//!
//! Alt+P toggles the box while the dialog is open, as in the reference application (its mnemonic P).
//! The pinned framework draws the box but keys no shortcut to it, so the app binds the key here:
//! [`preview_shortcut`] consumes it in the canvas pass that feeds the preview, before the pass
//! reads the box, and writes the new value through the dialog host's own field channel.

use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// The command whose dialog carries the Preview box and the grey channel preview.
pub(crate) const COMMAND: &str = "image.calculations";

/// The command and parameters the open Calculations form's canvas preview runs: the form's own
/// values (the same map OK sends), with the Result routed to New Document. Every Result option
/// computes the same result channel, and that run's one-layer grayscale document is exactly the
/// grey picture the reference shows - nothing else of the dialog's answer is previewed, which keeps
/// the New Document and Selection results at their measured behaviour.
pub(crate) fn preview_request(app: &PhotocraftApp, fields: &Map<String, Value>) -> (String, Value) {
    let mut params = crate::filter_dialog::params_of(fields);
    if let Some(object) = params.as_object_mut() {
        object.insert("result".to_owned(), json!("newDocument"));
    }
    (COMMAND.to_owned(), app.with_mask_target(COMMAND, params))
}

/// Alt+P toggles the open Calculations form's Preview check box; whether it did. The pinned
/// framework has no key bound to the box, so this is the app's half; consuming the event keeps the
/// `P` away from any row. `canvas::ensure_filter_preview` calls it on every canvas pass, before it
/// reads the box, so it works from either state and the very pass that toggled shows the result.
pub(crate) fn preview_shortcut(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let Some((id, on)) =
        app.framework.dialogs.get::<HostedParamDialog>().and_then(|dialog| (dialog.key() == Some(COMMAND)).then(|| (dialog.id(), !dialog.is_previewing())))
    else {
        return false;
    };
    if !ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::P)) {
        return false;
    }
    let _ = app.framework.dialogs.set_json_field(ctx, id, "preview", Value::Bool(on));
    ctx.request_repaint();
    true
}
