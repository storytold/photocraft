//! A modal color transaction for a named text style; the canvas preview never edits the session.
use std::sync::Arc;

use craft::color_ui::ui::{ColorPickerDialog, ColorPickerResult};
use craft::theme::Srgba;
use craft::ui::dialog::{DialogId, DialogResult};
use photocraft_doc::{Color, DocId, Document};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{PhotocraftApp, color_picker_ui, panel_docking};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Character,
    Paragraph,
}

impl Kind {
    fn paragraph(self) -> bool {
        self == Self::Paragraph
    }

    fn name(self) -> &'static str {
        if self.paragraph() { "paragraph" } else { "character" }
    }

    fn command(self) -> &'static str {
        if self.paragraph() { "type.paragraphStyle.set" } else { "type.characterStyle.set" }
    }

    /// The panel that shows the style's list and Style Options.
    fn panel(self) -> &'static str {
        if self.paragraph() { "paragraphStyles" } else { "characterStyles" }
    }
}

/// What the open picker is bound to: the document and revision it opened on, the style, the style's colour and the
/// picker's dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Target {
    document: DocId,
    revision: u64,
    kind: Kind,
    id: u32,
    color: Srgba,
    dialog: DialogId,
}

pub(crate) struct Preview {
    key: u64,
    shown: Result<Arc<Document>, String>,
}

fn color(doc: &Document, kind: Kind, id: u32) -> Result<Color, String> {
    let attrs = if kind.paragraph() {
        &doc.text_styles.para_style(id).ok_or("no such paragraph style")?.char_attrs
    } else {
        &doc.text_styles.char_style(id).ok_or("no such character style")?.attrs
    };
    serde_json::from_value(attrs.get("color").cloned().ok_or("enable the style's Color attribute first")?).map_err(|_| "invalid text style color".into())
}

/// Whether the target still names what it was opened on: the same document and revision, the same style open in
/// Style Options on the front tab of its panel, and the style still has a colour.
fn valid(app: &PhotocraftApp, t: &Target) -> bool {
    let Some(st) = app.session.active() else { return false };
    st.doc.id == t.document
        && st.revision == t.revision
        && app.ui.type_panels.options == Some((t.kind.paragraph(), t.id))
        && panel_docking::is_front(app, t.kind.panel())
        && color(&st.doc, t.kind, t.id).is_ok()
}

/// Validate before changing either the panel state or the dialog stack. Also used by automation.
pub(crate) fn open(app: &mut PhotocraftApp, kind: Kind, id: u32) -> Result<DialogId, String> {
    if app.dialog_open() {
        return Err("close the current dialog before opening a text style color picker".into());
    }
    let st = app.session.active().ok_or("no active document")?;
    let rgb = color(&st.doc, kind, id)?.to_rgb();
    if !rgb.iter().all(|v| v.is_finite()) {
        return Err("invalid text style color".into());
    }
    let (document, revision) = (st.doc.id, st.revision);
    // Style colors use encoded RGB, just like the existing compact style picker.
    let color = Srgba::from_rgb_array(rgb);
    panel_docking::show_panel(app, kind.panel(), Some(true))?;
    app.ui.type_panels.options = Some((kind.paragraph(), id));
    // The instance keeps two style pickers apart: the host replaces a dialog only by one of the same instance. No
    // screen eyedropper button, as in the reference application (#17017).
    let picker = ColorPickerDialog::new(format!("textStyle.{}.{id}", kind.name()), tl!("Color Picker (Text Color)"), color)
        .eyedropper(false)
        .instance((kind.name(), id));
    let dialog = app.framework.dialogs.open(picker);
    app.ui.type_panels.color_target = Some(Target { document, revision, kind, id, color, dialog });
    Ok(dialog)
}

/// Once per pass before the host runs: cancels the picker when its target no longer holds.
pub(crate) fn check(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(t) = app.ui.type_panels.color_target.clone() else { return };
    if !valid(app, &t) {
        app.framework.dialogs.cancel_by_id(ctx, t.dialog);
        app.ui.type_panels.color_target = None;
        app.text_style_preview = None;
    }
}

/// The picker finished: OK with a colour other than the style's runs one undo step; every result ends the target.
pub(crate) fn answered(app: &mut PhotocraftApp, result: DialogResult<ColorPickerResult>) {
    let target = app.ui.type_panels.color_target.take();
    app.text_style_preview = None;
    let (DialogResult::Ok(answer), Some(t)) = (result, target) else { return };
    // A no-op OK runs no command, so a precise imported colour is not quantised.
    if !valid(app, &t) || answer.picked.color.same_rgb8(t.color) {
        return;
    }
    let hex = color_picker_ui::hex(answer.picked.color.to_rgb_array());
    if let Err(error) = app.run(t.kind.command(), json!({"id": t.id, "attrs": {"color": hex}})) {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Takes the open picker's result and files it with [`answered`]; a picker that is gone without one ends the target.
/// A taker of `dialogs::take_results`, so a confirm or cancel from the dialog's buttons or from automation both end here.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(dialog) = app.ui.type_panels.color_target.as_ref().map(|t| t.dialog) else { return };
    if let Some(result) = app.framework.dialogs.take_by_id::<ColorPickerDialog>(dialog) {
        answered(app, result);
    } else if app.framework.dialogs.get_by_id::<ColorPickerDialog>(dialog).is_none() {
        app.ui.type_panels.color_target = None;
        app.text_style_preview = None;
    }
}

/// CPU and GPU canvases use the same isolated document; unchanged frames reuse its Arc.
pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let t = app.ui.type_panels.color_target.clone()?;
    let picked = app.framework.dialogs.get_by_id::<ColorPickerDialog>(t.dialog)?.current()?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id != t.document {
        return None;
    }
    if picked.color.same_rgb8(t.color) {
        app.text_style_preview = None;
        return None;
    }
    let hex = color_picker_ui::hex(picked.color.to_rgb_array());
    let key = (egui::Id::unique((t.document.0, t.revision, t.dialog.value(), t.kind.paragraph(), t.id, &hex)).value() & ((1 << 60) - 1)) | (1 << 60);
    if app.text_style_preview.as_ref().map(|p| p.key) != Some(key) {
        let shown = photocraft_engine::type_styles_cmds::preview_options(&st.doc, &json!({"id": t.id, "attrs": {"color": hex}}), t.kind.paragraph())
            .map(Arc::new)
            .map_err(|e| e.to_string());
        if let Err(error) = &shown {
            app.ui.status = error.clone();
            app.ui.status_error = true;
        }
        app.text_style_preview = Some(Preview { key, shown });
    }
    app.text_style_preview.as_ref()?.shown.as_ref().ok().map(|doc| (doc.clone(), key))
}

#[cfg(test)]
mod tests;
