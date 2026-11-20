//! A guide dialog's draft. Rendering and command validation stay in PhotoCraft; the
//! [`edit_session`](crate::edit_session) module owns proposal validity, stale-context rejection
//! and the single-use/no-op commit decision.
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedNewGuideDialog, NewGuideValues};
use craft::ui::runtime::env::Label;
use serde_json::Value;

use crate::PhotocraftApp;
use crate::edit_session::{EditSession, FinishError};
use photocraft_doc::{DocId, Document, Guides};
use photocraft_engine::prefs::{PointSize, Unit};

#[derive(Clone, Debug, PartialEq)]
struct Guide {
    vertical: bool,
    position: f64,
    color: Option<[u8; 3]>,
}

#[derive(Debug, PartialEq)]
struct Context {
    doc: DocId,
    revision: u64,
    rulers: Unit,
    points: PointSize,
    locked: bool,
}

/// The guide an Edit Selected Guides dialog edits and the values it opened with.
pub(crate) struct Target {
    pub(crate) vertical: bool,
    pub(crate) index: usize,
    /// The opening values: an unchanged colour is not sent, and an unchanged position keeps its exact pixels.
    pub(crate) original: NewGuideValues,
}

pub(crate) struct Draft {
    pub(crate) dialog: DialogId,
    target: Target,
    edit: EditSession<Guide, Context>,
    params: Option<Value>,
    guides: Option<Guides>,
    error: Option<String>,
}

impl Draft {
    /// The guide being edited.
    pub(crate) fn target(&self) -> &Target {
        &self.target
    }
}

/// Identity, revision, units and lock state that must still match at confirmation.
fn context(app: &PhotocraftApp) -> Option<Context> {
    let st = app.session.active()?;
    let units = &app.session.prefs().units_and_rulers;
    Some(Context { doc: st.doc.id, revision: st.revision, rulers: units.rulers, points: units.point_size, locked: app.ui.extras.lock_guides })
}

/// Read a guide and its explicit colour without trusting a saved index.
fn guide(g: &Guides, vertical: bool, index: usize) -> Option<Guide> {
    let position = *if vertical { &g.vertical } else { &g.horizontal }.get(index)?;
    Some(Guide { vertical, position, color: g.color(vertical, position) })
}

/// Capture the original target and context for the dialog `dialog`, retiring any previous draft.
pub(crate) fn begin(app: &mut PhotocraftApp, dialog: DialogId, values: &NewGuideValues, (vertical, index): (bool, usize)) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let value = guide(&st.doc.guides, vertical, index).ok_or("no such guide")?;
    let context = context(app).ok_or("no document")?;
    // The previous dialog is replaced by this one on the host; only its draft needs retiring.
    if let Some(previous) = app.guide_edit.take() {
        previous.edit.cancel();
    }
    app.guide_edit = Some(Draft {
        dialog,
        target: Target { vertical, index, original: *values },
        edit: EditSession::new(value, context),
        params: None,
        guides: None,
        error: None,
    });
    Ok(())
}

/// Read the live target for comparison with the draft's original context.
fn current(app: &PhotocraftApp, draft: &Draft) -> Option<(Guide, Context)> {
    Some((guide(&app.session.active()?.doc.guides, draft.target.vertical, draft.target.index)?, context(app)?))
}

/// Invalid text remains editable; a changed target or context requires a new draft.
fn stale(app: &PhotocraftApp, draft: &Draft) -> bool {
    current(app, draft).is_none_or(|(live, ctx)| !matches!(draft.edit.check(&live, &ctx), Ok(()) | Err(FinishError::InvalidProposal)))
}

/// Allow retry only while the retained draft still belongs to the same live guide.
pub(crate) fn retryable(app: &PhotocraftApp) -> bool {
    app.guide_edit.as_ref().is_some_and(|d| !stale(app, d))
}

/// Validate changed parameters on a COW document, retaining the last valid preview on error.
fn prepare(app: &PhotocraftApp, draft: &mut Draft, values: &NewGuideValues) -> Result<(), String> {
    if stale(app, draft) {
        return Err("The document or guide changed. Reopen Edit Selected Guides.".into());
    }
    let params = crate::new_guide_ui::params(values, Some(&draft.target));
    if draft.params.as_ref() == Some(&params) {
        return draft.error.clone().map_or(Ok(()), Err);
    }
    let st = app.session.active().ok_or("no document")?;
    // A cheap COW document on parameter changes only. The existing engine owns units,
    // validation, colour inheritance and turning a guide into the other orientation.
    let mut preview = photocraft_engine::Session::new();
    preview.prefs = app.session.prefs.clone();
    preview.add_document((*st.doc).clone(), None);
    let result = preview.execute("view.editGuide", params.clone()).map_err(|e| e.to_string()).and_then(|_| {
        let g = &preview.active().ok_or("no preview document")?.doc.guides;
        let (from, old_index) = (draft.target.vertical, draft.target.index);
        let to = params.get("newOrientation").and_then(Value::as_str) == Some("vertical");
        let index = if from == to { old_index } else { if to { g.vertical.len() } else { g.horizontal.len() }.checked_sub(1).ok_or("no preview guide")? };
        let value = guide(g, to, index).ok_or("no preview guide")?;
        draft.guides = Some(g.clone());
        Ok(value)
    });
    draft.params = Some(params);
    draft.edit.propose(result)
}

/// Keep the draft and its validation message available after either valid or invalid input.
pub(crate) fn update(app: &mut PhotocraftApp, values: &NewGuideValues) -> Result<(), String> {
    let mut draft = app.guide_edit.take().ok_or("Reopen Edit Selected Guides to start an edit.")?;
    let result = prepare(app, &mut draft, values);
    draft.error = result.as_ref().err().cloned();
    app.guide_edit = Some(draft);
    result
}

/// Commit once through the ordinary command path; a no-op preserves Redo.
/// A command refusal restores the proposal against its original context for a guarded retry.
pub(crate) fn finish(app: &mut PhotocraftApp, values: &NewGuideValues) -> Result<Option<(Value, Value)>, String> {
    update(app, values)?;
    let mut draft = app.guide_edit.take().ok_or("no guide draft")?;
    let (live, ctx) = current(app, &draft).ok_or("the guide is gone")?;
    let change = draft.edit.finish(&live, &ctx).map_err(|_| "The document or guide changed. Reopen Edit Selected Guides.")?;
    let Some(change) = change else { return Ok(None) };
    let params = draft.params.clone().ok_or("no guide parameters")?;
    match app.run("view.editGuide", params.clone()) {
        Ok(result) => Ok(Some((params, result))),
        Err(error) => {
            // Reuse the pre-command context, never a fresh document/revision. The caller's
            // retryable check rejects this draft if the failed command changed that context.
            draft.edit = EditSession::new(live, ctx);
            draft.edit.propose::<String>(Ok(change.after))?;
            app.guide_edit = Some(draft);
            Err(error)
        }
    }
}

/// Discard only the draft owned by the closing dialog, without touching history.
pub(crate) fn cancel(app: &mut PhotocraftApp, id: DialogId) {
    if app.guide_edit.as_ref().is_some_and(|d| d.dialog == id)
        && let Some(draft) = app.guide_edit.take()
    {
        draft.edit.cancel();
    }
}

/// Refresh after the dialog's values change; the message goes to the dialog. Returns whether the parameters changed.
pub(crate) fn sync(app: &mut PhotocraftApp) -> bool {
    let Some(id) = app.guide_edit.as_ref().map(|d| d.dialog) else { return false };
    // A dialog that finished this pass is read by `new_guide_ui::take`, which ends the draft.
    let Some(values) = app.framework.dialogs.get_by_id::<HostedNewGuideDialog>(id).map(|d| *d.values()) else { return false };
    let old = app.guide_edit.as_ref().and_then(|d| d.params.clone());
    let result = update(app, &values);
    let changed = old != app.guide_edit.as_ref().and_then(|d| d.params.clone());
    if let Some(dialog) = app.framework.dialogs.get_mut_by_id::<HostedNewGuideDialog>(id) {
        match result {
            Ok(()) if changed => dialog.set_error(None),
            // Unchanged, valid parameters may still have a command-policy refusal to show.
            Ok(()) => {}

            Err(error) => dialog.set_error(Some(Label::from(error))),
        }
    }
    changed
}

/// The validation message of the draft's current values, if any.
pub(crate) fn error(app: &PhotocraftApp) -> Option<&str> {
    app.guide_edit.as_ref()?.error.as_deref()
}

/// Expose the last valid preview only for its unchanged document and still-open dialog.
pub(crate) fn preview<'a>(app: &'a PhotocraftApp, doc: &Document) -> Option<&'a Guides> {
    let draft = app.guide_edit.as_ref()?;
    if draft.edit.context().doc != doc.id || stale(app, draft) || app.framework.dialogs.get_by_id::<HostedNewGuideDialog>(draft.dialog).is_none() {
        return None;
    }
    draft.guides.as_ref()
}
