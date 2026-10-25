//! Waiting for files to be closed (`photocraft --wait a.tif`, and forwarded opens that wait).
//!
//! An external tool that launches an editor and blocks until the user is done (git's
//! `core.editor`, `code --wait`, a photo manager's "Edit in", a build script) needs the editor to
//! say when every file it handed over is closed, and whether it was saved. A [`Wait`] tracks the
//! documents opened from its paths: a path is done once its documents were all closed, or when
//! it never opened (the open failed). When every path is done the wait ends with exit code 0 if
//! any of its files was saved at least once, else 1.

use photocraft_doc::DocId;
use serde_json::{Value, json};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};

/// The outcome a waiting process reads when the app exits (see [`Wait::outcome`]).
#[derive(Debug, Default)]
pub struct WaitOutcome {
    /// Set when every waited file was closed: 0 saved, 1 closed unsaved.
    pub code: Option<i32>,
    /// Any waited file was saved (for an app quit before the wait ended).
    pub saved: bool,
}

impl WaitOutcome {
    /// The process exit status: the finished wait's code, or 0/1 by `saved` when the app quit
    /// first.
    pub fn exit_code(&self) -> i32 {
        self.code.unwrap_or(if self.saved { 0 } else { 1 })
    }
}

/// Who hears that a wait ended.
pub enum WaitNotify {
    /// This process waits (`--wait`): record the outcome and quit once no document is left.
    Process(Arc<Mutex<WaitOutcome>>),
    /// A forwarding client waits: send `{"event": "wait.done", "params": {…}}`.
    Channel(Sender<Value>),
}

struct Tracked {
    path: String,
    docs: Vec<DocId>,
    seen: bool,
}

/// One set of files someone waits on.
pub struct Wait {
    files: Vec<Tracked>,
    saved: bool,
    notify: WaitNotify,
}

impl Wait {
    pub fn new(paths: Vec<String>, notify: WaitNotify) -> Self {
        Wait { files: paths.into_iter().map(|path| Tracked { path, docs: Vec::new(), seen: false }).collect(), saved: false, notify }
    }
}

/// Paths compare as written by `open_paths` (absolute); on Windows and macOS case-insensitively.
fn same_path(a: &str, b: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) { a.eq_ignore_ascii_case(b) } else { a == b }
}

impl crate::PhotocraftApp {
    /// Start waiting for `paths` (absolute, as opened) to be closed.
    pub fn wait_for(&mut self, paths: Vec<String>, notify: WaitNotify) {
        self.waits.push(Wait::new(paths, notify));
    }

    /// Marks waits whose document `doc` was saved to `path`.
    pub(crate) fn wait_saved(&mut self, doc: DocId, path: &str) {
        for w in &mut self.waits {
            if w.files.iter().any(|f| f.docs.contains(&doc) || same_path(&f.path, path)) {
                w.saved = true;
                if let WaitNotify::Process(o) = &w.notify {
                    o.lock().unwrap_or_else(PoisonError::into_inner).saved = true;
                }
            }
        }
    }

    /// Advances every wait against the open documents and pending opens; ends the finished ones.
    /// Returns true when a `--wait` process should quit now (its wait ended and no document is
    /// open).
    pub(crate) fn tick_waits(&mut self) -> bool {
        if self.waits.is_empty() {
            return false;
        }
        let docs: Vec<(DocId, Option<String>)> = self.session.documents().iter().map(|d| (d.doc.id, d.path.clone())).collect();
        let pending: Vec<String> = self.jobs.opens.iter().filter_map(|o| o.path.clone()).collect();
        let mut quit = false;
        let mut i = 0;
        while i < self.waits.len() {
            let w = &mut self.waits[i];
            for f in &mut w.files {
                for (id, path) in &docs {
                    if path.as_deref().is_some_and(|p| same_path(p, &f.path)) && !f.docs.contains(id) {
                        f.docs.push(*id);
                        f.seen = true;
                    }
                }
                f.docs.retain(|id| docs.iter().any(|(d, _)| d == id));
            }
            let done = w.files.iter().all(|f| f.docs.is_empty() && (f.seen || !pending.iter().any(|p| same_path(p, &f.path))));
            if !done {
                i += 1;
                continue;
            }
            let w = self.waits.remove(i);
            let code = if w.saved { 0 } else { 1 };
            match w.notify {
                WaitNotify::Process(o) => {
                    let mut o = o.lock().unwrap_or_else(PoisonError::into_inner);
                    o.code = Some(code);
                    o.saved = w.saved;
                    quit |= docs.is_empty();
                }
                WaitNotify::Channel(tx) => {
                    let paths: Vec<_> = w.files.iter().map(|f| f.path.clone()).collect();
                    let _ = tx.send(json!({"event": "wait.done", "params": {"paths": paths, "saved": w.saved, "exitCode": code}}));
                }
            }
        }
        quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> crate::PhotocraftApp {
        let services =
            crate::Services { export: Some(Box::new(|_, _, _| Ok((b"x".to_vec(), Vec::new())))), write: Some(Box::new(|_, _| Ok(()))), ..Default::default() };
        crate::PhotocraftApp::new(photocraft_engine::Session::new(), services)
    }

    fn open(app: &mut crate::PhotocraftApp, path: &str) {
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.session.active_mut().unwrap().path = Some(path.into());
    }

    #[test]
    fn saved_then_closed_exits_zero_and_quits_when_nothing_is_left() {
        let mut app = app();
        let outcome = Arc::new(Mutex::new(WaitOutcome::default()));
        open(&mut app, "/w/a.png");
        app.wait_for(vec!["/w/a.png".into()], WaitNotify::Process(outcome.clone()));
        assert!(!app.tick_waits());
        app.run("layer.new.layer", json!({})).unwrap();
        app.save(true, crate::FormatChoices::default()).unwrap();
        assert!(!app.tick_waits(), "still open");
        app.run("file.close", json!({})).unwrap();
        assert!(app.tick_waits(), "done and no document left: quit");
        assert_eq!(outcome.lock().unwrap().exit_code(), 0);
        assert!(app.waits.is_empty());
    }

    #[test]
    fn closed_unsaved_exits_one_and_other_documents_keep_the_app_open() {
        let mut app = app();
        let outcome = Arc::new(Mutex::new(WaitOutcome::default()));
        open(&mut app, "/w/other.psd");
        open(&mut app, "/w/a.tif");
        app.wait_for(vec!["/w/a.tif".into()], WaitNotify::Process(outcome.clone()));
        app.tick_waits();
        app.run("file.close", json!({})).unwrap();
        assert!(!app.tick_waits(), "another document is open: the app stays");
        assert_eq!(outcome.lock().unwrap().code, Some(1));
    }

    #[test]
    fn a_file_that_never_opens_ends_the_wait_and_channels_get_wait_done() {
        let mut app = app();
        let (tx, rx) = std::sync::mpsc::channel();
        app.wait_for(vec!["/w/missing.png".into()], WaitNotify::Channel(tx));
        app.tick_waits();
        let done = rx.try_recv().unwrap();
        assert_eq!(done, json!({"event": "wait.done", "params": {"paths": ["/w/missing.png"], "saved": false, "exitCode": 1}}));
        // An app quit before the wait ended still answers by `saved`.
        assert_eq!(WaitOutcome { code: None, saved: true }.exit_code(), 0);
        assert_eq!(WaitOutcome::default().exit_code(), 1);
    }
}
