//! What the chrome reads from a document at the cost of a walk, a parse or a list rebuild, computed once per change
//! of what it reads: the status bar's sizes and profile, the File Info copyright mark of the titles, the empty frames
//! the canvas marks, whether linked files need watching, the artboard pages, the layers a Move drag takes along and
//! the pasteboard's uncovered work (once per document revision, the last also per revision of the document the
//! canvas shows), and the History panel's rows (once per change of the history or its snapshots).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use craft::behavior::kurbo::Rect as DocRect;
use craft::ui::bars::StatusValues;
use craft::ui::canvas::PageFrame;
use craft::ui::patterns::panels::HistoryRow;
use photocraft_doc::{DocId, LayerId};
use photocraft_engine::{DocState, Session};

use crate::i18n::Lang;

/// The derived values of every open document. Held behind a lock because the chrome reads them through
/// `&PhotocraftApp`.
#[derive(Default)]
pub(crate) struct DocQueries(Mutex<Entries>);

#[derive(Default)]
struct Entries {
    /// `Session::documents_generation` the entries were made under. A document closed and another opened with the
    /// same id starts at a revision the old entry may also have had, so the entries go whenever the set changes.
    documents: u64,
    by_doc: HashMap<DocId, Entry>,
    /// How many values were computed: the trials' count of rebuilds.
    builds: u64,
}

/// The history's generation and the snapshots' generation.
type HistoryKey = (u64, u64);

/// One document's values. Those keyed on the document revision are valid for `revision`; the History rows carry
/// their own key.
#[derive(Default)]
struct Entry {
    revision: u64,
    /// The status values about the document, in the language they were written in.
    status: Option<(Lang, StatusValues)>,
    copyrighted: Option<bool>,
    empty_frames: Option<Arc<[LayerId]>>,
    has_links: Option<bool>,
    pages: Option<Arc<[PageFrame]>>,
    moving: Option<MovingLayers>,
    /// The uncovered rects and the display revision of the document they were measured on.
    uncovered: Option<(u64, Arc<[DocRect]>)>,
    /// The History rows and the history and snapshot generations they were built from.
    history: Option<(HistoryKey, Arc<[HistoryRow]>)>,
}

/// The layers of a document a Move tool drag moves, before the live text edit and Free Transform are added.
#[derive(Clone)]
pub(crate) struct MovingLayers {
    /// The selected layers.
    pub(crate) selected: Arc<[LayerId]>,
    /// What moves whole with them: the layers linked to them and everything inside a moved group.
    pub(crate) whole: Arc<[LayerId]>,
}

impl DocQueries {
    /// Runs `f` on the entry of `st` and the build count, the revision-keyed values emptied first when the document
    /// moved past the entry's revision.
    fn with<R>(&self, session: &Session, st: &DocState, f: impl FnOnce(&mut Entry, &mut u64) -> R) -> R {
        let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Entries { documents, by_doc, builds } = &mut *entries;
        let open = session.documents_generation();
        if *documents != open {
            by_doc.clear();
            *documents = open;
        }
        let entry = by_doc.entry(st.doc.id).or_default();
        if entry.revision != st.revision {
            *entry = Entry { revision: st.revision, history: entry.history.take(), ..Entry::default() };
        }
        f(entry, builds)
    }

    /// How many values have been computed since the app started; equal counts mean nothing was rebuilt.
    #[cfg(test)]
    pub(crate) fn builds(&self) -> u64 {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).builds
    }

    /// The status bar's values about `st` in the current language, `compute`d on the first read after a change.
    pub(crate) fn status(&self, session: &Session, st: &DocState, compute: impl FnOnce() -> StatusValues) -> StatusValues {
        let lang = crate::i18n::current();
        self.with(session, st, |entry, builds| match &entry.status {
            Some((made_in, values)) if *made_in == lang => values.clone(),
            _ => built(builds, &mut entry.status, || (lang, compute())).1.clone(),
        })
    }

    /// Whether File Info marks `st` Copyrighted, read from its XMP on the first read after a change.
    pub(crate) fn copyrighted(&self, session: &Session, st: &DocState) -> bool {
        self.with(session, st, |entry, builds| {
            *cached(builds, &mut entry.copyrighted, || photocraft_engine::file_cmds::copyright_marked(st.doc.metadata.xmp.as_deref()))
        })
    }

    /// The visible frames of `st` with nothing placed in them, which the canvas marks with a placeholder.
    pub(crate) fn empty_frames(&self, session: &Session, st: &DocState) -> Arc<[LayerId]> {
        self.with(session, st, |entry, builds| {
            cached(builds, &mut entry.empty_frames, || {
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
        self.with(session, st, |entry, builds| *cached(builds, &mut entry.has_links, || !photocraft_engine::link_watch::linked_files(&st.doc).is_empty()))
    }

    /// The canvas's page frames of `st`, `compute`d on the first read after a change (the active layer, and so the
    /// active frame, moves the revision).
    pub(crate) fn pages(&self, session: &Session, st: &DocState, compute: impl FnOnce() -> Vec<PageFrame>) -> Arc<[PageFrame]> {
        self.with(session, st, |entry, builds| cached(builds, &mut entry.pages, || compute().into()).clone())
    }

    /// The layers of `st` a Move drag moves, `compute`d on the first read after a change (a layer selection moves the
    /// revision).
    pub(crate) fn moving(&self, session: &Session, st: &DocState, compute: impl FnOnce() -> MovingLayers) -> MovingLayers {
        self.with(session, st, |entry, builds| cached(builds, &mut entry.moving, compute).clone())
    }

    /// Where the shown layers draw outside the artboards of `st`, for the document the canvas shows at display
    /// revision `shown`, `compute`d when either changes.
    pub(crate) fn uncovered(&self, session: &Session, st: &DocState, shown: u64, compute: impl FnOnce() -> Vec<DocRect>) -> Arc<[DocRect]> {
        self.with(session, st, |entry, builds| match &entry.uncovered {
            Some((at, rects)) if *at == shown => rects.clone(),
            _ => built(builds, &mut entry.uncovered, || (shown, compute().into())).1.clone(),
        })
    }

    /// The History panel's rows of `st`, `compute`d when its history or its snapshots change.
    pub(crate) fn history_rows(&self, session: &Session, st: &DocState, compute: impl FnOnce() -> Vec<HistoryRow>) -> Arc<[HistoryRow]> {
        let key = (st.history.generation(), st.snapshots_generation());
        self.with(session, st, |entry, builds| match &entry.history {
            Some((at, rows)) if *at == key => rows.clone(),
            _ => built(builds, &mut entry.history, || (key, compute().into())).1.clone(),
        })
    }
}

/// `slot`'s value, computed (and counted in `builds`) when it has none.
fn cached<'a, T>(builds: &mut u64, slot: &'a mut Option<T>, compute: impl FnOnce() -> T) -> &'a T {
    if slot.is_none() {
        *builds += 1;
    }
    slot.get_or_insert_with(compute)
}

/// Puts a freshly computed value in `slot`, counted in `builds`.
fn built<'a, T>(builds: &mut u64, slot: &'a mut Option<T>, compute: impl FnOnce() -> T) -> &'a T {
    *builds += 1;
    slot.insert(compute())
}
