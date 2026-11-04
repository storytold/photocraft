//! #10879: applying the Layers panel filter, or changing its criteria while it filters, deselects
//! the selected layers it no longer lists, as in Photoshop. The listed ones stay selected (one of
//! them becomes the active layer when the active one is hidden), and turning the filter off changes
//! nothing. Like any layer selection this is no history step. A layer selected later, while the
//! filter hides it, stays selected: the filter only acts when it changes.

use photocraft_doc::LayerId;
use serde_json::json;

use super::Filter;
use crate::PhotocraftApp;

/// Deselect hidden selected layers only when this open document's criteria change.
/// Returning to a tab or changing document-derived context does not apply the same filter again.
pub fn follow(app: &mut PhotocraftApp) {
    let now = Filter::of(app);
    if super::switch::changed_since_draw(app, &now) && now.active() {
        deselect_hidden(app);
    }
}

/// Deselect the active document's selected layers that [`super::FilterContext`] doesn't list: through
/// `select.deselectLayers` when none stays listed, else by keeping only the listed ones.
pub fn deselect_hidden(app: &mut PhotocraftApp) {
    let f = Filter::of(app);
    let context = super::FilterContext::of_app(app, &f);
    if !f.active_with(&context) {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let selected = st.selected_layers();
    let listed: Vec<LayerId> = selected.iter().copied().filter(|id| st.doc.layer(*id).is_some_and(|l| f.shows_with(l, &context))).collect();
    if listed.len() == selected.len() && st.active_layer.is_none_or(|a| listed.contains(&a)) {
        return;
    }
    if listed.is_empty() {
        let _ = app.run("select.deselectLayers", json!({}));
        return;
    }
    let active = st.active_layer.filter(|a| listed.contains(a)).or(listed.last().copied());
    let anchor = st.layer_anchor.filter(|a| listed.contains(a)).or(active);
    let _ = photocraft_engine::layer_multi_cmds::set_selection(&mut app.session, listed, active, anchor);
}
