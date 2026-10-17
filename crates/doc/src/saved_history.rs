//! Undo history carried with a document to and from a `.pcraft` file (Preferences › Performance ›
//! Save History in Document, #6053).
//!
//! This is transport, not editing state: the native loader fills [`crate::Document::saved_history`]
//! when the file holds a readable history, the session turns it back into its undo history when
//! the document opens (and clears the field), and a save puts the history it wants written there
//! just for that save. A document being edited never holds one.

use std::sync::Arc;

use crate::{Document, LayerId};

/// One past state: the document as it was before a step, its History panel label and the layers
/// it targeted.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedState {
    pub label: String,
    pub doc: Arc<Document>,
    pub active: Option<LayerId>,
    pub selected: Vec<LayerId>,
}

/// A document's linear undo history, oldest state first. Redo states are not kept.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SavedHistory {
    /// Past states, oldest first; the last one is the state just before the document itself.
    pub states: Vec<SavedState>,
    /// Label of the step that produced the document itself (the History panel's current row).
    pub current_label: String,
    /// Layers targeted by the document itself.
    pub current_active: Option<LayerId>,
    pub current_selected: Vec<LayerId>,
    /// Saving: the most bytes of pixels (tiles and blobs, uncompressed) that only the history may
    /// add to the file. The newest states that fit are written; older ones are left out. 0 writes
    /// no states. Ignored when loading.
    pub max_bytes: u64,
}
