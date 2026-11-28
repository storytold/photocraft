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
//!
//! The same file holds the dialog's row table (#14526): [`rows`] builds it in the reference
//! application's order, from the framework's own row kinds. The `image.calculations` arm of
//! `menus::view_cmds::dialogs` hands it the Channel options it lists for the source and the
//! Blending options it shares with the other channel forms; the arm keeps opening the hosted
//! parameter form it opened before.

use craft::ui::dialog::StackDialog as _;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamKind};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::value_drag::SliderScale;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// The command whose dialog carries the Preview box and the grey channel preview.
pub(crate) const COMMAND: &str = "image.calculations";

/// The rows of the Calculations form, in the reference application's order (#14526): a Source 1 group
/// (Channel, Invert), a Source 2 group, then Blending, Opacity, Scale and Offset, and Result last.
/// The Source and Layer dropdowns of the two groups come with #14520 and the Mask group with its own
/// item; the section heads hold their places meanwhile, and no row here gains a control those items
/// own. `channels` and `channel_default` are the Channel options of the source the form reads
/// (#14521); `blends` are the form's Blending options, whose list is #14523's.
///
/// Every row is one of the pinned framework's own row kinds, no drawn layout of the app's: Opacity is
/// a `Range` row over 0..100 at whole numbers with a percent unit and a
/// [`popup_slider`](Param::popup_slider), which is the reference's number field with the arrow that
/// opens the slider; Scale and Offset are `Range` rows over the reference's bounds that show only
/// while Blending names Add or Subtract. A change of Blending [`resets`](Param::resets) those two
/// keys, so a value typed for Add never reaches OK under another mode; what every other Blending
/// sends is the command's own defaults, Scale 1 and Offset 0.
pub(crate) fn rows(channels: Vec<(String, Label)>, channel_default: &str, blends: Vec<(String, Label)>) -> Vec<Param> {
    let channel = |key: &str| Param::new(key, tl!("Channel"), ParamKind::choice(channels.clone(), Some(channel_default.to_owned())));
    let percent =
        Param::new("opacity", tl!("Opacity"), ParamKind::range(0.0, 100.0, 100.0, SliderScale::Linear)).decimals(0).unit(UnitKind::Percent).popup_slider();
    let for_add_and_subtract = |row: Param| row.visible_when_in("blending", ["add", "subtract"]);
    vec![
        Param::section(tl!("Source 1")),
        channel("source1Channel"),
        Param::new("source1Invert", tl!("Invert"), ParamKind::bool(false)),
        Param::section(tl!("Source 2")),
        channel("source2Channel"),
        Param::new("source2Invert", tl!("Invert"), ParamKind::bool(false)),
        Param::new("blending", tl!("Blending"), ParamKind::choice(blends, Some("multiply".to_owned()))).resets(["scale", "offset"]),
        percent,
        for_add_and_subtract(Param::new("scale", tl!("Scale"), ParamKind::range(1.0, 2.0, 1.0, SliderScale::Linear)).decimals(3)),
        for_add_and_subtract(Param::new("offset", tl!("Offset"), ParamKind::range(-255.0, 255.0, 0.0, SliderScale::Linear)).decimals(0)),
        Param::new(
            "result",
            tl!("Result"),
            ParamKind::choice(
                vec![("newDocument", tl!("New Document")), ("newChannel", tl!("New Channel")), ("selection", tl!("Selection"))]
                    .into_iter()
                    .map(|(id, name)| (id.to_owned(), Label::from(name)))
                    .collect(),
                Some("newChannel".to_owned()),
            ),
        ),
    ]
}

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
