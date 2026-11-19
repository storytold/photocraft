//! What the chrome reads from a document at the cost of a walk or a parse, computed once per document revision:
//! the status bar's sizes and profile, the File Info copyright mark of the titles, the empty frames the canvas marks,
//! whether linked files need watching, and the pasteboard's uncovered work (once per revision of the document the
//! canvas shows).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use craft::behavior::kurbo::Rect as DocRect;
use craft::ui::bars::StatusValues;
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::{DocState, Session};

use crate::i18n::Lang;

/// The derived values of every open document, each entry valid for one `DocState::revision`. Held behind a lock
/// because the chrome reads them through `&PhotocraftApp`.
#[derive(Default)]
pub(crate) struct DocQueries(Mutex<Entries>);

#[derive(Default)]
struct Entries {
    /// `Session::documents_generation` when closed documents were last dropped.
    documents: u64,
    by_doc: HashMap<DocId, Entry>,
}

#[derive(Default)]
struct Entry {
    revision: u64,
    /// The status values about the document, in the language they were written in.
    status: Option<(Lang, StatusValues)>,
    copyrighted: Option<bool>,
    empty_frames: Option<Arc<[LayerId]>>,
    has_links: Option<bool>,
    /// The uncovered rects and the display revision of the document they were measured on.
    uncovered: Option<(u64, Arc<[DocRect]>)>,
}

impl DocQueries {
    /// Runs `f` on the entry of `st`, emptied first when the document moved past the entry's revision.
    fn with<R>(&self, session: &Session, st: &DocState, f: impl FnOnce(&mut Entry) -> R) -> R {
        let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let documents = session.documents_generation();
        if entries.documents != documents {
            entries.by_doc.retain(|id, _| session.documents().iter().any(|open| open.doc.id == *id));
            entries.documents = documents;
        }
        let entry = entries.by_doc.entry(st.doc.id).or_default();
        if entry.revision != st.revision {
            *entry = Entry { revision: st.revision, ..Entry::default() };
        }
        f(entry)
    }

    /// The status bar's values about `st` in the current language, `compute`d on the first read after a change.
    pub(crate) fn status(&self, session: &Session, st: &DocState, compute: impl FnOnce() -> StatusValues) -> StatusValues {
        let lang = crate::i18n::current();
        self.with(session, st, |entry| match &entry.status {
            Some((made_in, values)) if *made_in == lang => values.clone(),
            _ => entry.status.insert((lang, compute())).1.clone(),
        })
    }

    /// Whether File Info marks `st` Copyrighted, read from its XMP on the first read after a change.
    pub(crate) fn copyrighted(&self, session: &Session, st: &DocState) -> bool {
        self.with(session, st, |entry| *entry.copyrighted.get_or_insert_with(|| photocraft_engine::file_cmds::copyright_marked(st.doc.metadata.xmp.as_deref())))
    }

    /// The visible frames of `st` with nothing placed in them, which the canvas marks with a placeholder.
    pub(crate) fn empty_frames(&self, session: &Session, st: &DocState) -> Arc<[LayerId]> {
        self.with(session, st, |entry| {
            entry
                .empty_frames
                .get_or_insert_with(|| {
                    let doc = &st.doc;
                    doc.walk()
                        .into_iter()
                        .filter(|(path, _, l)| l.frame.is_some() && l.children().is_some_and(<[_]>::is_empty) && doc.shown_at(path))
                        .map(|(_, _, l)| l.id)
                        .collect()
                })
                .clone()
        })
    }

    /// Whether `st` has linked smart objects, whose files the link watcher scans.
    pub(crate) fn has_linked_files(&self, session: &Session, st: &DocState) -> bool {
        self.with(session, st, |entry| *entry.has_links.get_or_insert_with(|| !photocraft_engine::link_watch::linked_files(&st.doc).is_empty()))
    }

    /// Where the shown layers draw outside the artboards of `st`, for the document the canvas shows at display
    /// revision `shown`, `compute`d when either changes.
    pub(crate) fn uncovered(&self, session: &Session, st: &DocState, shown: u64, compute: impl FnOnce() -> Vec<DocRect>) -> Arc<[DocRect]> {
        self.with(session, st, |entry| match &entry.uncovered {
            Some((at, rects)) if *at == shown => rects.clone(),
            _ => entry.uncovered.insert((shown, compute().into())).1.clone(),
        })
    }
}
