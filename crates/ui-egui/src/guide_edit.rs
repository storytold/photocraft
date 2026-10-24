//! A guide dialog's draft. Rendering and command validation stay in PhotoCraft; edit-session
//! owns proposal validity, stale-context rejection and the single-use/no-op commit decision.
use edit_session::{EditSession, FinishError};
use photocraft_doc::{DocId, Document, Guides};
use photocraft_engine::prefs::{PointSize, Unit};
use serde_json::{Map, Value};

use crate::PhotocraftApp;

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

pub(crate) struct Draft {
    pub(crate) dialog: u64,
    target: Value,
    initial_position: String,
    edit: EditSession<Guide, Context>,
    params: Option<Value>,
    guides: Option<Guides>,
    error: Option<String>,
}

fn context(app: &PhotocraftApp) -> Option<Context> {
    let st = app.session.active()?;
    let units = &app.session.prefs().units_and_rulers;
    Some(Context { doc: st.doc.id, revision: st.revision, rulers: units.rulers, points: units.point_size, locked: app.ui.extras.lock_guides })
}

fn guide(g: &Guides, vertical: bool, index: usize) -> Option<Guide> {
    let position = *if vertical { &g.vertical } else { &g.horizontal }.get(index)?;
    Some(Guide { vertical, position, color: g.color(vertical, position) })
}

fn target(f: &Value) -> Option<(bool, usize)> {
    let vertical = match f.get("orientation")?.as_str()? {
        "vertical" => true,
        "horizontal" => false,
        _ => return None,
    };
    Some((vertical, usize::try_from(f.get("index")?.as_u64()?).ok()?))
}

pub(crate) fn begin(app: &mut PhotocraftApp, dialog: u64, fields: &Map<String, Value>) -> Result<(), String> {
    let target_value = fields.get("__edit").ok_or("missing guide target")?;
    let (vertical, index) = target(target_value).ok_or("invalid guide target")?;
    let st = app.session.active().ok_or("no document")?;
    let value = guide(&st.doc.guides, vertical, index).ok_or("no such guide")?;
    let context = context(app).ok_or("no document")?;
    if let Some(previous) = app.guide_edit.take() {
        app.ui.close_dialog(previous.dialog);
        previous.edit.cancel();
    }
    app.guide_edit = Some(Draft {
        dialog,
        target: target_value.clone(),
        initial_position: fields.get("position").and_then(Value::as_str).unwrap_or_default().into(),
        edit: EditSession::new(value, context),
        params: None,
        guides: None,
        error: None,
    });
    Ok(())
}

fn current(app: &PhotocraftApp, draft: &Draft) -> Option<(Guide, Context)> {
    let (vertical, index) = target(&draft.target)?;
    Some((guide(&app.session.active()?.doc.guides, vertical, index)?, context(app)?))
}

fn stale(app: &PhotocraftApp, draft: &Draft) -> bool {
    current(app, draft).is_none_or(|(live, ctx)| !matches!(draft.edit.check(&live, &ctx), Ok(()) | Err(FinishError::InvalidProposal)))
}

pub(crate) fn retryable(app: &PhotocraftApp) -> bool {
    app.guide_edit.as_ref().is_some_and(|d| !stale(app, d))
}

fn prepare(app: &PhotocraftApp, draft: &mut Draft, fields: &Map<String, Value>) -> Result<(), String> {
    if stale(app, draft) || fields.get("__edit") != Some(&draft.target) {
        return Err("The document or guide changed. Reopen Edit Selected Guides.".into());
    }
    let mut params = crate::new_guide_ui::params(app, fields);
    // Opening a fractional position in cm/percent rounds its display. Leaving the text and
    // orientation alone must not move the guide or consume Redo through that rounding.
    if fields.get("position").and_then(Value::as_str) == Some(&draft.initial_position)
        && (params.get("newOrientation").and_then(Value::as_str) == Some("vertical")) == draft.edit.original().vertical
    {
        params["position"] = serde_json::json!(draft.edit.original().position);
    }
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
        let (from, old_index) = target(&draft.target).ok_or("invalid guide target")?;
        let to = params.get("newOrientation").and_then(Value::as_str) == Some("vertical");
        let index = if from == to { old_index } else { if to { g.vertical.len() } else { g.horizontal.len() }.checked_sub(1).ok_or("no preview guide")? };
        let value = guide(g, to, index).ok_or("no preview guide")?;
        draft.guides = Some(g.clone());
        Ok(value)
    });
    draft.params = Some(params);
    draft.edit.propose(result)
}

pub(crate) fn update(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<(), String> {
    let mut draft = app.guide_edit.take().ok_or("Reopen Edit Selected Guides to start an edit.")?;
    let result = prepare(app, &mut draft, fields);
    draft.error = result.as_ref().err().cloned();
    app.guide_edit = Some(draft);
    result
}

/// The GUI and automation share this path; a no-op emits no command, preserving Redo.
pub(crate) fn finish(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Option<Value>, String> {
    update(app, fields)?;
    let draft = app.guide_edit.take().ok_or("no guide draft")?;
    let (live, ctx) = current(app, &draft).ok_or("the guide is gone")?;
    let change = draft.edit.finish(&live, &ctx).map_err(|_| "The document or guide changed. Reopen Edit Selected Guides.")?;
    Ok(change.and(draft.params))
}

pub(crate) fn cancel(app: &mut PhotocraftApp, id: u64) {
    if app.guide_edit.as_ref().is_some_and(|d| d.dialog == id)
        && let Some(draft) = app.guide_edit.take()
    {
        draft.edit.cancel();
    }
}

/// Refresh after dialog fields change, and drop drafts whose dialog has gone away.
pub(crate) fn sync(app: &mut PhotocraftApp) -> bool {
    let Some(id) = app.guide_edit.as_ref().map(|d| d.dialog) else { return false };
    let Some(fields) = app.ui.dialogs.iter().find(|d| d.id == id).map(|d| d.fields.clone()) else {
        cancel(app, id);
        return false;
    };
    let old = app.guide_edit.as_ref().and_then(|d| d.params.clone());
    let result = update(app, &fields);
    if let Some(d) = app.ui.dialog_mut(id) {
        match result {
            Ok(()) => {
                d.fields.remove("__error");
            }
            Err(error) => {
                d.fields.insert("__error".into(), Value::String(error));
            }
        }
    }
    old != app.guide_edit.as_ref().and_then(|d| d.params.clone())
}

pub(crate) fn preview<'a>(app: &'a PhotocraftApp, doc: &Document) -> Option<&'a Guides> {
    let draft = app.guide_edit.as_ref()?;
    if draft.edit.context().doc != doc.id || stale(app, draft) || !app.ui.dialogs.iter().any(|d| d.id == draft.dialog) {
        return None;
    }
    draft.guides.as_ref()
}
