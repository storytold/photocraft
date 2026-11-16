//! Single-slice view state shared by commands and the Slice Select tool.

use super::{Document, EngineError, Rect, ResolvedSlice, Result, Session, Target, Value, bad, json, slices};
use crate::DocState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SliceSelection {
    id: Option<u32>,
    rect: Rect,
}

/// Hit-test resolved pieces, with the latest stored slice above auto slices.
pub fn slice_at(doc: &Document, x: f64, y: f64) -> Option<ResolvedSlice> {
    let bounds = doc.bounds();
    if !x.is_finite() || !y.is_finite() || x < f64::from(bounds.x0) || y < f64::from(bounds.y0) || x >= f64::from(bounds.x1) || y >= f64::from(bounds.y1) {
        return None;
    }
    let all = slices::resolve(doc);
    let order = |r: &ResolvedSlice| r.id.and_then(|id| doc.slices.list.iter().position(|s| s.id == id));
    all.into_iter().filter(|r| r.rect.contains(x.floor() as i32, y.floor() as i32)).max_by_key(|r| order(r).map_or(0, |i| i + 1))
}

/// Resolve identity against an already computed list; display numbers are not stable targets.
pub fn selected_number_in(st: &DocState, resolved: &[ResolvedSlice]) -> Option<usize> {
    let selected = st.slice_selection?;
    resolved
        .iter()
        .find(|r| r.id == selected.id && r.rect == selected.rect)
        .or_else(|| selected.id.and_then(|id| resolved.iter().find(|r| r.id == Some(id))))
        .map(|r| r.number)
}

/// Current display number, or None when the selected stored slice/auto region no longer exists.
pub fn selected_number(st: &DocState) -> Option<usize> {
    st.slice_selection?;
    selected_number_in(st, &slices::resolve(&st.doc))
}

pub(super) fn selected<'a>(st: &DocState, resolved: &'a [ResolvedSlice]) -> Option<&'a ResolvedSlice> {
    let number = selected_number_in(st, resolved)?;
    resolved.iter().find(|r| r.number == number)
}

/// Implicit selection after a slice edit: the edit already updates the document revision.
pub(super) fn remember(s: &mut Session, selected: Option<&ResolvedSlice>) {
    if let Some(st) = s.active_mut() {
        st.slice_selection = selected.map(|r| SliceSelection { id: r.id, rect: r.rect });
    }
}

pub(super) fn promoted(s: &mut Session, target: Target, id: u32) {
    let Some(st) = s.active_mut() else { return };
    let Some(selected) = st.slice_selection else { return };
    let matches = match target {
        Target::Stored(old) => selected.id == Some(old),
        Target::Auto(rect) => selected.id.is_none() && selected.rect == rect,
    };
    if matches && let Some(slice) = st.doc.slices.get(id) {
        st.slice_selection = Some(SliceSelection { id: Some(id), rect: slice.rect });
    }
}

pub(super) fn prune(st: &mut DocState) {
    if st.slice_selection.is_some_and(|selected| selected.id.is_some_and(|id| st.doc.slices.get(id).is_none())) {
        st.slice_selection = None;
    }
}

pub(super) fn select(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "slice.select";
    let st = s.active().ok_or(EngineError::NoDocument)?;
    // `coalesce` belongs to the dispatcher and does not add another slice selector.
    if p.as_object().is_none_or(|p| p.keys().filter(|key| key.as_str() != "coalesce").count() != 1) {
        return Err(bad(CMD, "give `point`: [x,y] or `clear`: true"));
    }
    let selected = if p.get("clear") == Some(&Value::Bool(true)) {
        None
    } else {
        if s.file_menu.slices_locked {
            return Err(bad(CMD, "Slices are locked (View › Lock Slices)"));
        }
        let point = p.get("point").and_then(Value::as_array).filter(|p| p.len() == 2).ok_or_else(|| bad(CMD, "`point` must be [x,y] in document pixels"))?;
        let coordinate =
            |v: Option<&Value>| v.and_then(Value::as_f64).filter(|n| n.is_finite()).ok_or_else(|| bad(CMD, "point coordinates must be finite numbers"));
        let (x, y) = (coordinate(point.first())?, coordinate(point.get(1))?);
        Some(slice_at(&st.doc, x, y).ok_or_else(|| bad(CMD, "no slice at that point"))?)
    };
    let next = selected.as_ref().map(|r| SliceSelection { id: r.id, rect: r.rect });
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if st.slice_selection != next {
        let clean = !st.is_dirty();
        st.slice_selection = next;
        let floating = st.floating.as_mut().filter(|f| f.revision == st.revision);
        st.revision = st.revision.saturating_add(1);
        // Only the slice target changed; floating pixels still belong to this document.
        if let Some(floating) = floating {
            floating.revision = st.revision;
        }
        st.view_revision = st.view_revision.saturating_add(1);
        if clean {
            st.saved_revision = st.revision;
        }
        st.last_damage = Some(Rect::EMPTY);
    }
    Ok(match selected {
        Some(r) => json!({"slice":r.id,"number":r.number}),
        None => json!({"slice":null,"number":null}),
    })
}
