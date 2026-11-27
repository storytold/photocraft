//! View › Guides › New Guide…: the framework's hosted New Guide over the active document: an orientation, a position
//! with a unit and a colour, as in Photoshop.
//!
//! The dialog reads the position in the ruler unit and takes any unit Photoshop's field takes. OK runs `view.newGuide`
//! with the position in pixels. A position the engine refuses reopens the dialog with what was typed and the engine's
//! message, as Photoshop keeps New Guide open on a bad position. The dialog remembers the last guide's values itself
//! (`remember`), shown in the current ruler unit at the next open.
//!
//! Color lists Photoshop's guide colours (Cyan by default) and Custom…, which opens the Color Picker. The guide stores
//! the chosen colour itself, so changing the preference guide colour later doesn't recolour it.
//!
//! Edit Selected Guides (the guide menu, `guide_select`) opens the same dialog on the selected guide. OK runs
//! `view.editGuide`, sending the colour only when it was changed, so a guide that uses the preference colour keeps
//! doing so.

use craft::behavior::Axis;
use craft::ui::canvas::{Guide, GuideColor};
use craft::ui::dialog::{DialogId, DialogResult, StackDialog};
use craft::ui::patterns::form_dialogs::{HostRequests, HostedNewGuideDialog, NewGuideDialog, NewGuideValues};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::memory::SaveKey;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::guide_edit::Target;
use crate::guide_layout_ui::{guide_units, preference_rgba};

const COMMAND: &str = "view.newGuide";
/// Where New Guide's values are remembered between opens.
const REMEMBER: SaveKey = SaveKey::new("craft_dialogs.photocraft.new_guide");

/// `"#rrggbb"` of a guide colour, as the engine takes it.
fn hex(color: GuideColor) -> String {
    let [r, g, b, _] = color.rgba().0;
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn orientation(vertical: bool) -> &'static str {
    if vertical { "vertical" } else { "horizontal" }
}

/// A New Guide dialog opening with `values`, remembering them for the next open.
fn new_dialog(app: &PhotocraftApp, values: NewGuideValues) -> HostedNewGuideDialog {
    let units = guide_units(app);
    HostedNewGuideDialog::new(values, move |bind| NewGuideDialog::new(units, bind).remember(REMEMBER))
}

/// An Edit Selected Guides dialog showing `guide`; its values start as the guide's.
fn edit_dialog(app: &PhotocraftApp, guide: Guide) -> HostedNewGuideDialog {
    let units = guide_units(app);
    let preference = preference_rgba(app);
    let values = NewGuideValues::of_guide(&guide, preference);
    HostedNewGuideDialog::new(values, move |bind| NewGuideDialog::edit(units, guide.clone(), preference, bind))
}

/// Opens New Guide: the last guide's orientation, position and colour (in the current ruler unit), else horizontal at
/// 0 in Cyan.
pub fn open(app: &mut PhotocraftApp) -> DialogId {
    let dialog = new_dialog(app, NewGuideValues::new(Axis::X, 0.0, GuideColor::default()));
    app.framework.dialogs.open(dialog)
}

/// Edit Selected Guides: the dialog on the selected guide, with its orientation, position and colour (its own, or the
/// preference colour it is drawn in).
pub fn open_edit(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let (vertical, index) = crate::guide_select::selected(app).ok_or_else(|| "No guide is selected".to_string())?;
    let guide = {
        let g = &app.session.active().ok_or_else(|| "No guide is selected".to_string())?.doc.guides;
        let position = *if vertical { &g.vertical } else { &g.horizontal }.get(index).ok_or_else(|| "No guide is selected".to_string())?;
        let mut guide = Guide::new(crate::guide_select::key(vertical, index), if vertical { Axis::Y } else { Axis::X }, position);
        guide.color = g.color(vertical, position).map(|[r, g, b]| craft::theme::Rgba::rgb(r, g, b));
        guide
    };
    let values = NewGuideValues::of_guide(&guide, preference_rgba(app));
    let dialog = edit_dialog(app, guide);
    let id = app.framework.dialogs.open(dialog);
    if let Err(error) = crate::guide_edit::begin(app, id, &values, (vertical, index)) {
        let ctx = app.ctx().clone();
        app.framework.dialogs.cancel_by_id(&ctx, id);
        return Err(error);
    }
    Ok(id)
}

/// The command's parameters for the dialog's values: positions go as pixels. An edit sends the colour only when it
/// differs from the one the dialog opened with.
pub(crate) fn params(values: &NewGuideValues, edit: Option<&Target>) -> Value {
    let along = orientation(values.along == Axis::Y);
    match edit {
        Some(target) => {
            let mut p = json!({"orientation": orientation(target.vertical), "index": target.index, "newOrientation": along, "position": values.position_px});
            if values.color != target.original.color {
                p["color"] = json!(hex(values.color));
            }
            p
        }
        None => json!({"orientation": along, "position": values.position_px, "color": hex(values.color)}),
    }
}

/// The values a finished dialog's guide stands for; a guide with no colour of its own keeps `fallback`.
fn values_of(guide: &Guide, fallback: GuideColor) -> NewGuideValues {
    NewGuideValues::new(guide.along, guide.position, guide.color.map_or(fallback, GuideColor::from_rgba))
}

fn new_guide_requests(app: &mut PhotocraftApp, id: DialogId) -> Option<&mut HostRequests> {
    app.framework.dialogs.get_mut_by_id::<HostedNewGuideDialog>(id).map(HostedNewGuideDialog::requests)
}

/// The dialog's colour requests and its answer, once per pass.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    if let Some(id) = app.framework.dialogs.get::<HostedNewGuideDialog>().map(StackDialog::id) {
        crate::guide_layout_ui::pump_colors(app, id, new_guide_requests);
    }
    let draft = app.guide_edit.as_ref().map(|d| d.dialog);
    let result = match draft {
        Some(id) => app.framework.dialogs.take_by_id::<HostedNewGuideDialog>(id),
        None => app.framework.dialogs.take::<HostedNewGuideDialog>(),
    };
    let outcome = match (result, draft) {
        (None, _) => return,
        (Some(DialogResult::Ok(guide)), Some(_)) => confirm_edit(app, &guide),
        (Some(DialogResult::Ok(guide)), None) => confirm_new(app, &guide),
        // Cancel and any other way of closing end the edit draft.
        (Some(_), draft) => {
            // A dialog replaced by another under the same id closes with a Cancel that is not the new draft's.
            if let Some(id) = draft.filter(|id| app.framework.dialogs.get_by_id::<HostedNewGuideDialog>(*id).is_none()) {
                crate::guide_edit::cancel(app, id);
            }
            return;
        }
    };
    if let Err(error) = outcome {
        app.ui.status = error;
    }
}

/// New Guide's OK: a refused position opens the dialog again with what was typed and the engine's message.
fn confirm_new(app: &mut PhotocraftApp, guide: &Guide) -> Result<(), String> {
    let values = values_of(guide, GuideColor::default());
    match app.run(COMMAND, params(&values, None)) {
        Ok(_) => Ok(()),
        Err(error) => {
            let mut again = new_dialog(app, values);
            again.set_error(Some(Label::from(error.clone())));
            app.framework.dialogs.open(again);
            Err(error)
        }
    }
}

/// Edit Selected Guides' OK: the edited guide stays selected (at the end of the other list if it turned). A refused
/// edit opens the dialog again on the typed values, as New Guide does.
fn confirm_edit(app: &mut PhotocraftApp, guide: &Guide) -> Result<(), String> {
    let (target, original) = match app.guide_edit.as_ref() {
        Some(draft) => ((draft.target().vertical, draft.target().index), draft.target().original.color),
        None => return Err("Reopen Edit Selected Guides to start an edit.".into()),
    };
    let values = values_of(guide, original);
    let params = match crate::guide_edit::finish(app, &values) {
        Ok(Some((params, _))) => params,
        Ok(None) => return Ok(()),
        Err(error) => {
            if crate::guide_edit::retryable(app) {
                // The typed proposal is shown as the guide the dialog edits, so it opens on the typed values.
                let mut typed = Guide::new(crate::guide_select::key(target.0, target.1), values.along, values.position_px);
                typed.color = Some(values.color.rgba());
                let mut again = edit_dialog(app, typed);
                again.set_error(Some(Label::from(error.clone())));
                let id = app.framework.dialogs.open(again);
                if let Some(draft) = app.guide_edit.as_mut() {
                    draft.dialog = id;
                }
            } else {
                app.guide_edit = None;
            }
            return Err(error);
        }
    };
    let (from, to) = (params["orientation"] == "vertical", params["newOrientation"] == "vertical");
    let index = if from == to {
        params["index"].as_u64().and_then(|i| usize::try_from(i).ok())
    } else {
        app.session.active().map(|st| if to { st.doc.guides.vertical.len() } else { st.doc.guides.horizontal.len() }.saturating_sub(1))
    };
    if let Some(i) = index {
        crate::guide_select::select(app, to, i);
    }
    Ok(())
}

/// The validated draft shown on the canvas, without changing the document or its history.
pub fn edit_preview<'a>(app: &'a PhotocraftApp, doc: &photocraft_doc::Document) -> Option<&'a photocraft_doc::Guides> {
    crate::guide_edit::preview(app, doc)
}

/// The open New Guide dialog's id (also while Edit Selected Guides is open), for drivers outside the crate.
pub fn dialog_id(app: &PhotocraftApp) -> Option<DialogId> {
    app.framework.dialogs.get::<HostedNewGuideDialog>().map(StackDialog::id)
}

/// Sets one field of the open New Guide dialog by its JSON name, for drivers outside the crate.
pub fn set_field(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) -> Result<(), String> {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, field, value)
}

/// The message Edit Selected Guides shows for its current values, if any.
pub fn edit_error(app: &PhotocraftApp) -> Option<&str> {
    crate::guide_edit::error(app)
}

#[cfg(test)]
mod tests;
