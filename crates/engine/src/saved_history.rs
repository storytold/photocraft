//! Preferences › Performance › Save History in Document (#6053): the undo history goes into the
//! copy of a document a save writes (only `.pcraft` stores it), and comes back when a document
//! opens with one.

use std::sync::Arc;

use photocraft_doc::{Document, SavedHistory, SavedState};
use photocraft_ops::{HistoryState, LayerTarget};

use crate::{DocState, Session};

impl Session {
    /// Document `index` as a save writes it. With Save History in Document on and steps to undo,
    /// a copy carrying the undo history up to the preferences' size limit; otherwise the document
    /// itself. Formats other than `.pcraft` ignore the history.
    pub fn document_to_save(&self, index: usize) -> Option<Arc<Document>> {
        let st = self.documents().get(index)?;
        let perf = &self.prefs().performance;
        if !perf.save_history_in_document || !st.history.can_undo() {
            return Some(st.doc.clone());
        }
        let states = st
            .history
            .past_states()
            .map(|s| SavedState { label: s.label.clone(), doc: s.doc.clone(), active: s.layers.active, selected: s.layers.selected.clone() })
            .collect();
        let (label, layers) = st.history.current();
        let history = SavedHistory {
            states,
            current_label: label.to_owned(),
            current_active: layers.active,
            current_selected: layers.selected.clone(),
            max_bytes: perf.history_in_document_bytes(),
        };
        let mut doc = (*st.doc).clone();
        doc.saved_history = Some(Arc::new(history));
        Some(Arc::new(doc))
    }
}

/// Makes the history a document was opened with `st`'s undo history (its history limits already
/// set): the newest states within them, every state with the document's identity, and the layers
/// the document targeted when it was saved.
pub(crate) fn restore(st: &mut DocState, saved: &SavedHistory) {
    let id = st.doc.id;
    let past = saved
        .states
        .iter()
        .map(|s| {
            let mut doc = s.doc.clone();
            if doc.id != id || doc.saved_history.is_some() {
                let d = Arc::make_mut(&mut doc);
                d.id = id;
                d.saved_history = None;
            }
            HistoryState { label: s.label.clone(), doc, layers: LayerTarget { active: s.active, selected: s.selected.clone() } }
        })
        .collect();
    let current = LayerTarget { active: saved.current_active, selected: saved.current_selected.clone() };
    st.history.restore(past, saved.current_label.clone(), current.clone());
    crate::restore_target(st, current);
    st.trim_history();
}
