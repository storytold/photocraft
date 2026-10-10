//! Session-wide, demand-grown history cache. Current documents and adjacent history stay hot.
//! Native scratch writes run one at a time; wasm uses the same RAM accounting and eviction.
use std::collections::HashSet;
use std::sync::Arc;

use photocraft_doc::Document;
use photocraft_ops::document_bytes;

use crate::Session;

#[cfg(not(target_arch = "wasm32"))]
use photocraft_format::{HistoryArchive, HistorySnapshot};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct ColdDocument(HistorySnapshot);
#[cfg(not(target_arch = "wasm32"))]
impl photocraft_ops::ArchivedDocument for ColdDocument {
    fn load(&self) -> std::result::Result<Arc<Document>, String> {
        self.0.load().map(Arc::new).map_err(|e| format!("Could not restore undo history: {e}"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
enum Completed {
    Stored(HistorySnapshot),
    Collected,
}

#[cfg(not(target_arch = "wasm32"))]
struct Pending {
    source: Option<Arc<Document>>,
    rx: std::sync::mpsc::Receiver<std::result::Result<Completed, String>>,
    worker: std::thread::JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct Cache {
    pub(super) dirty: bool,
    draining: bool,
    failed: bool,
    #[cfg(not(target_arch = "wasm32"))]
    quota_blocked: bool,
    pub(super) notice: Option<String>,
    next_document: usize,
    #[cfg(not(target_arch = "wasm32"))]
    archive: Option<HistoryArchive>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<Pending>,
}

impl Session {
    /// Reapply shared limits. Paths are selected lazily on the first spill.
    pub(crate) fn configure_history_cache(&mut self) {
        self.history_cache.dirty = true;
        self.history_cache.failed = false;
        self.poll_history_cache();
    }

    /// Unique resident managed payload bytes across open documents and history, without reading disk.
    pub fn history_resident_bytes(&self) -> usize {
        let mut seen = HashSet::new();
        let mut total = 0usize;
        for st in &self.docs {
            total = total.saturating_add(document_bytes(&st.doc, &mut seen));
            for doc in st.history.resident_states() {
                total = total.saturating_add(document_bytes(&doc, &mut seen));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(pending) = &self.history_cache.pending
            && let Some(source) = &pending.source
        {
            total = total.saturating_add(document_bytes(source, &mut seen));
        }
        total
    }

    /// Descriptor bytes for retained history states and compact replay records.
    pub fn history_metadata_bytes(&self) -> usize {
        self.docs.iter().fold(0usize, |n, st| n.saturating_add(st.history.metadata_bytes()))
    }

    /// Actual compressed scratch bytes (never the configured capacity).
    pub fn history_disk_bytes(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.history_cache.archive.as_ref().map_or(0, HistoryArchive::bytes_used)
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
    }

    pub fn history_cache_busy(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.history_cache.pending.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// One-shot diagnostic for the UI/agent. Cache failures never roll back a successful edit.
    pub fn take_history_cache_notice(&mut self) -> Option<String> {
        self.history_cache.notice.take()
    }

    /// Poll once per frame or command. No disk I/O or full-document traversal on idle frames.
    pub fn poll_history_cache(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(pending) = &self.history_cache.pending {
            let result = match pending.rx.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // New edits must not create an unbounded RAM backlog behind slow I/O.
                    if self.history_cache.dirty {
                        self.history_cache.dirty = false;
                        let budget = self.prefs().performance.history_budget_bytes();
                        let pinned = self.history_pinned_bytes();
                        if budget > 0 && self.trim_history_memory(budget.max(pinned)) {
                            self.history_cache.notice = Some("Older undo history was shortened because scratch writes could not keep up with editing.".into());
                        }
                    }
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("Undo cache worker stopped unexpectedly".into())),
            };
            if let (Some(result), Some(pending)) = (result, self.history_cache.pending.take()) {
                // A received result means work is complete. Join only a finished worker.
                // Dropping a not-yet-finished handle detaches its final return, never blocks UI.
                if pending.worker.is_finished() && pending.worker.join().is_err() {
                    self.history_cache.notice = Some("Undo cache worker failed; your document is unchanged".into());
                }
                match result {
                    Ok(Completed::Stored(snapshot)) => {
                        let Some(source) = pending.source else { return };
                        let protected = self.docs.iter().any(|st| Arc::ptr_eq(&st.doc, &source) || st.history.is_hot(&source));
                        let cold: Arc<dyn photocraft_ops::ArchivedDocument> = Arc::new(ColdDocument(snapshot));
                        if !protected {
                            for st in &mut self.docs {
                                st.history.replace_resident(&source, cold.clone());
                            }
                        }
                    }
                    Ok(Completed::Collected) => {
                        self.history_cache.quota_blocked = false;
                    }
                    Err(error) => {
                        self.history_cache.quota_blocked = error.contains("scratch history exceeds");
                        self.history_cache.failed = !self.history_cache.quota_blocked;
                        self.history_cache.notice =
                            Some(format!("{error}. Older undo history may be shortened to limit memory; your current document is kept."));
                    }
                }
                self.history_cache.dirty = true;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.history_cache.archive.as_ref().is_some_and(HistoryArchive::has_pending_releases) {
            self.history_cache.dirty = true;
        }
        if !self.history_cache.dirty {
            return;
        }
        self.history_cache.dirty = false;
        let budget = self.prefs().performance.history_budget_bytes();
        let used = self.history_resident_bytes();

        if used > budget {
            self.history_cache.draining = true;
        }
        let target = if self.history_cache.draining { budget.saturating_sub(budget / 10) } else { budget };

        #[cfg(not(target_arch = "wasm32"))]
        {
            let disk_budget = u64::from(self.prefs().scratch_disks.budget_mb).saturating_mul(1 << 20);
            // Deferred lease destruction keeps file deletion and compression off the UI.
            if self.history_cache.archive.as_ref().is_some_and(HistoryArchive::has_pending_releases) {
                self.start_history_gc();
                return;
            }
            if self.history_cache.quota_blocked {
                if self.docs.iter().any(|st| st.history.archived_states() > 0) && self.docs.iter_mut().any(|st| st.history.drop_oldest()) {
                    self.history_cache.dirty = true;
                    self.start_history_gc();
                    return;
                }
                self.history_cache.quota_blocked = false;
                self.history_cache.failed = true;
            }
            // Explicit reductions retire one oldest state then collect off-thread before
            // considering another. Actual disk bytes can lag queued lease releases.
            if self.history_disk_bytes() > disk_budget && self.docs.iter_mut().any(|st| st.history.discard_oldest()) {
                self.history_cache.notice = Some("Older undo history was shortened to apply the new scratch disk budget.".into());
                self.history_cache.dirty = true;
                self.start_history_gc();
                return;
            }
            if let Some(archive) = &self.history_cache.archive
                && let Err(error) = archive.set_budget(disk_budget)
            {
                self.history_cache.notice = Some(format!("Could not apply undo disk budget: {error}"));
                self.history_cache.failed = true;
            }
            let disk_enabled = disk_budget > 0 && self.prefs().scratch_disks.disks.iter().any(|disk| disk.enabled);
            if budget > 0 && used > target && disk_enabled && !self.history_cache.failed {
                let n = self.docs.len();
                for offset in 0..n {
                    let index = self.history_cache.next_document.wrapping_add(offset) % n;
                    let Some(source) = self.history_spill_candidate(index) else { continue };
                    if self.history_cache.archive.is_none() {
                        let root = self
                            .prefs()
                            .scratch_disks
                            .disks
                            .iter()
                            .find(|disk| disk.enabled)
                            .filter(|disk| disk.path != "(system temp)")
                            .map(|disk| std::path::PathBuf::from(&disk.path));
                        self.history_cache.archive = Some(HistoryArchive::new(root, disk_budget));
                    }
                    let Some(archive) = self.history_cache.archive.clone() else { break };
                    let (tx, rx) = std::sync::mpsc::sync_channel(1);
                    let doc = source.clone();
                    match std::thread::Builder::new().name("photocraft-undo-cache".into()).spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| archive.store(&doc)))
                            .map_err(|_| "Undo cache worker panicked".to_string())
                            .and_then(|result| result.map(Completed::Stored).map_err(|error| format!("Could not store undo history: {error}")));
                        let _ = tx.send(result);
                    }) {
                        Ok(worker) => {
                            self.history_cache.pending = Some(Pending { source: Some(source), rx, worker });
                            self.history_cache.next_document = index.wrapping_add(1);
                            return;
                        }
                        Err(error) => {
                            self.history_cache.failed = true;
                            self.history_cache.notice = Some(format!("Could not start undo cache worker: {error}"));
                            break;
                        }
                    }
                }
            }
        }
        // No disk, unavailable disk, or only pinned states left: discard oldest history only.
        if budget > 0 {
            self.trim_history_memory(target);
        }
        self.history_cache.draining = budget > 0 && self.history_resident_bytes() > target;
        #[cfg(not(target_arch = "wasm32"))]
        if self.history_cache.archive.as_ref().is_some_and(HistoryArchive::has_pending_releases) {
            self.history_cache.dirty = true;
            self.start_history_gc();
        }
    }
    fn trim_history_memory(&mut self, target: usize) -> bool {
        let mut shortened = false;
        while self.history_resident_bytes() > target {
            let Some(index) = (0..self.docs.len()).find(|index| self.history_memory_candidate(*index).is_some()) else { break };
            if !self.docs.get_mut(index).is_some_and(|st| st.history.drop_oldest()) {
                break;
            }
            shortened = true;
        }
        shortened
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn history_pinned_bytes(&self) -> usize {
        let mut seen = HashSet::new();
        let mut total = 0usize;
        for st in &self.docs {
            total = total.saturating_add(document_bytes(&st.doc, &mut seen));
            for doc in st.history.hot_states() {
                total = total.saturating_add(document_bytes(&doc, &mut seen));
            }
        }
        if let Some(pending) = &self.history_cache.pending
            && let Some(source) = &pending.source
        {
            total = total.saturating_add(document_bytes(source, &mut seen));
        }
        total
    }

    fn history_spill_candidate(&self, index: usize) -> Option<Arc<Document>> {
        self.history_candidate(index, true)
    }

    fn history_memory_candidate(&self, index: usize) -> Option<Arc<Document>> {
        self.history_candidate(index, false)
    }

    fn history_candidate(&self, index: usize, disk: bool) -> Option<Arc<Document>> {
        let mut pinned = HashSet::new();
        for st in &self.docs {
            document_bytes(&st.doc, &mut pinned);
            for doc in st.history.hot_states() {
                document_bytes(&doc, &mut pinned);
            }
        }
        let st = self.docs.get(index)?;
        let candidates = if disk { st.history.spillable_states() } else { st.history.resident_states() };
        candidates.into_iter().find(|doc| {
            !self.docs.iter().any(|state| Arc::ptr_eq(&state.doc, doc) || state.history.is_hot(doc)) && document_bytes(doc, &mut pinned.clone()) > 0
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_history_gc(&mut self) {
        let Some(archive) = self.history_cache.archive.clone() else { return };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        match std::thread::Builder::new().name("photocraft-undo-gc".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| archive.collect_garbage()))
                .map_err(|_| "Undo cache cleanup worker panicked".to_string())
                .and_then(|result| result.map(|()| Completed::Collected).map_err(|error| error.to_string()));
            let _ = tx.send(result);
        }) {
            Ok(worker) => self.history_cache.pending = Some(Pending { source: None, rx, worker }),
            Err(error) => {
                self.history_cache.notice = Some(format!("Could not start undo cache cleanup: {error}"));
                self.history_cache.failed = true;
            }
        }
    }
}

#[cfg(test)]
mod tests;
