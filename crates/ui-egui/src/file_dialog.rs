//! File dialogs that never block the event loop (#673, #574).
//!
//! A command that needs a file asks with [`PhotocraftApp::ask_file`] (or one of its wrappers) and
//! hands over what to do with the answer. The request goes to the host's file service
//! ([`craft::host::Files`]); [`PhotocraftApp::poll_file_dialog`], every frame, continues the action
//! once the service has the user's answer. On native targets the service shows the platform
//! pickers with the window as their parent when [`PhotocraftApp::pump_files`] runs at the end of
//! the frame, so the window keeps drawing and answering the compositor meanwhile. A dialog run to
//! completion inside the frame froze the window (Linux compositors offered to kill it, #574) and,
//! on macOS, ran AppKit's modal loop inside winit's event handler, which aborts the app when
//! another event arrives (#673).
//!
//! One dialog at a time: asking while one is open is refused.

use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use craft::host::PickId;
use craft::host::{PickKind, PickRequest, Picked};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::file_open::{display_name, path_text};

mod filters;
use filters::{SaveCopyFormats, open_filters, save_request};

/// What a file or folder dialog asks the user for, on the web build, which keeps its own dialog
/// path until the framework's web `Files` replaces it (FW-264).
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogRequest {
    /// Files to read: several for File › Open, one for a command that reads a file (Place,
    /// scripts, notes, presets). Starts in `initial_dir` (the last-used folder, UI-217-3).
    Open { multiple: bool, initial_dir: Option<String>, extensions: Option<Vec<String>> },
    /// Which folder to read from. Starts in `initial_dir` when available.
    Folder { initial_dir: Option<String> },
    /// Where to write, starting from `suggested` (a file name, or the document's own path).
    Save { suggested: String },
}

/// The user's choice in a web dialog (a cancelled dialog answers `None`).
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogAnswer {
    /// The chosen files on disk; the app reads them.
    Paths(Vec<String>),
    /// A chosen file's name and contents (the web, where a page gets contents rather than paths).
    Contents(String, Vec<u8>),
    /// Several chosen files' names and contents, in order (the web's multi-select File › Open).
    ContentsMany(Vec<(String, Vec<u8>)>),
    /// The download's name.
    SaveTo(String),
    /// The selected folder's path.
    FolderTo(String),
    /// No dialog could be shown, or the file chosen can't be used. Reported as an error rather
    /// than taken for Cancel.
    Unavailable(String),
}

/// Hands a web dialog's answer to the app and wakes it, from any thread. Dropping it unanswered
/// counts as Cancel.
#[cfg(target_arch = "wasm32")]
pub struct FileDialogReply {
    tx: std::sync::mpsc::Sender<Option<FileDialogAnswer>>,
    ctx: egui::Context,
}

#[cfg(target_arch = "wasm32")]
impl FileDialogReply {
    /// Answers the dialog.
    pub fn send(self, answer: Option<FileDialogAnswer>) {
        // Nothing is waiting any more when the app has quit meanwhile.
        let _ = self.tx.send(answer);
        self.ctx.request_repaint();
    }
}

/// Show a web file dialog and return at once, answering through the reply when the user is done.
#[cfg(target_arch = "wasm32")]
pub type FileDialogFn = Box<dyn FnMut(FileDialogRequest, Option<&eframe::Frame>, FileDialogReply)>;

/// What to do with the answer. It runs with the user's files only: Cancel and an unavailable
/// dialog end in `Err` before it.
type Then = Box<dyn FnOnce(&mut PhotocraftApp, Picked) -> Result<Value, String>>;

/// The file dialog in progress.
pub(crate) struct Pending {
    /// The service's ticket for the pick.
    #[cfg(not(target_arch = "wasm32"))]
    id: PickId,
    /// The web dialog's answer.
    #[cfg(target_arch = "wasm32")]
    answer: std::sync::mpsc::Receiver<Option<FileDialogAnswer>>,
    then: Then,
}

/// The error of a cancelled dialog: not reported, since backing out is the user's choice.
pub(crate) const CANCELLED: &str = "cancelled";
pub(crate) const UNEXPECTED: &str = "the file dialog gave an unexpected answer";

/// The text of a chosen path. [`PhotocraftApp::ask_file`] has refused a path that is not valid
/// Unicode by the time an action sees it.
fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The chosen paths of an answer as text; empty for an answer that has none. A browser's file
/// names ([`Picked::Contents`]) are not paths, so a command that writes or reads a path refuses
/// them.
fn path_strings(picked: &Picked) -> Vec<String> {
    match picked {
        Picked::Paths(paths) => paths.iter().map(|p| path_string(p)).collect(),
        Picked::Saved { path, .. } => vec![path_string(path)],
        Picked::Contents(_) | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => Vec::new(),
    }
}

impl PhotocraftApp {
    /// Whether a file dialog is open (or about to be shown).
    pub fn file_dialog_open(&self) -> bool {
        self.file_dialog.is_some()
    }

    /// Show a file dialog; `then` continues with the user's choice on a later frame (see the
    /// module docs). Returns at once with `{"fileDialog": "open" | "folder" | "save"}`; `Err` while
    /// one is open. `then` gets [`Picked::Paths`] or [`Picked::Saved`] (every path valid Unicode) or
    /// [`Picked::Contents`]; Cancel ends the action with [`CANCELLED`] and an unavailable dialog
    /// with its reason, neither reaching `then`.
    pub(crate) fn ask_file(&mut self, request: PickRequest, then: impl FnOnce(&mut Self, Picked) -> Result<Value, String> + 'static) -> Result<Value, String> {
        if self.file_dialog.is_some() {
            return Err("a file dialog is already open".into());
        }
        let kind = match request.kind {
            PickKind::Open | PickKind::OpenMany => "open",
            PickKind::Folder => "folder",
            PickKind::Save => "save",
        };
        let saving = request.kind == PickKind::Save;
        let then: Then = Box::new(move |app, picked| match picked {
            Picked::Cancelled | Picked::Downloaded(_) => Err(CANCELLED.into()),
            Picked::Unavailable(why) | Picked::Failed(why) => Err(why),
            Picked::Paths(paths) => {
                // A lossy copy of a name that isn't valid Unicode would name another file (#6601).
                for path in &paths {
                    path_text(path, saving)?;
                }
                then(app, Picked::Paths(paths))
            }
            Picked::Saved { path, file_type } => {
                path_text(&path, saving)?;
                then(app, Picked::Saved { path, file_type })
            }
            picked @ Picked::Contents(_) => then(app, picked),
        });
        self.file_dialog = Some(self.start_pick(request, then)?);
        Ok(json!({ "fileDialog": kind }))
    }

    /// Hands `request` to the host's file service.
    #[cfg(not(target_arch = "wasm32"))]
    fn start_pick(&mut self, request: PickRequest, then: Then) -> Result<Pending, String> {
        // PLAN: hub-w1's `Framework` has no accessor for the host's `Files`; `Framework::files(&self) -> &dyn Files` (the host's) is assumed.
        let id = self.framework.files().pick(request);
        Ok(Pending { id, then })
    }

    /// Shows the web dialog of [`Services::file_dialog`](crate::Services::file_dialog) for
    /// `request`; the web has no host `Files` until FW-264.
    #[cfg(target_arch = "wasm32")]
    fn start_pick(&mut self, request: PickRequest, then: Then) -> Result<Pending, String> {
        let ctx = self.ctx().clone();
        let show = self.services.file_dialog.as_mut().ok_or(CANCELLED)?;
        let (tx, answer) = std::sync::mpsc::channel();
        show(web_request(&request), None, FileDialogReply { tx, ctx });
        Ok(Pending { answer, then })
    }

    /// The answer of the pick in progress, once it has come.
    #[cfg(not(target_arch = "wasm32"))]
    fn take_picked(&mut self) -> Option<Picked> {
        let id = self.file_dialog.as_ref()?.id;
        self.framework.files().take_answer(id)
    }

    /// The answer of the web dialog in progress, once it has come.
    #[cfg(target_arch = "wasm32")]
    fn take_picked(&mut self) -> Option<Picked> {
        use std::sync::mpsc::TryRecvError;
        match self.file_dialog.as_ref()?.answer.try_recv() {
            Err(TryRecvError::Empty) => None,
            Ok(answer) => Some(web_picked(answer)),
            // The reply was dropped unanswered: the dialog couldn't be shown.
            Err(TryRecvError::Disconnected) => Some(Picked::Cancelled),
        }
    }

    /// Ask the user to choose a folder, starting in the folder of the most recent file when known.
    /// `then` receives the selected path when the dialog is answered.
    pub(crate) fn ask_folder(&mut self, then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static) -> Result<Value, String> {
        let mut request = PickRequest::folder("folder");
        if let Some(dir) = last_used_dir(&self.ui.recent_files) {
            request = request.directory(dir);
        }
        self.ask_file(request, move |app, picked| match path_strings(&picked).into_iter().next() {
            Some(path) => then(app, path),
            None => Err(UNEXPECTED.into()),
        })
    }

    /// Ask where to save: `then` gets the chosen path as the file service settled it: with a
    /// lowercase extension when Preferences ▸ File Handling › Lowercase Extension is on (its
    /// default), and the suggested extension added to a bare name. The service asks whether to
    /// replace a file the settling made the target before `then` runs.
    ///
    /// A bare file name (export dialogs, an untitled document) starts in the folder the active
    /// document was last saved or exported to through a dialog, else beside the document
    /// (#1826, #1866). A caller's explicit directory is kept. The chosen folder is remembered for
    /// the document the dialog was asked for.
    pub(crate) fn pick_save(&mut self, suggested: &str, then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static) -> Result<Value, String> {
        self.pick_save_in(suggested, false, None, then)
    }

    /// Save a Copy filters the picker by the copied document's mode and depth.
    pub(crate) fn pick_save_copy(
        &mut self,
        suggested: &str,
        mode: ColorMode,
        depth: SampleType,
        then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static,
    ) -> Result<Value, String> {
        self.pick_save_in(suggested, false, SaveCopyFormats::for_document(mode, depth), then)
    }

    /// [`Self::pick_save`] for the export flows (Export As, Quick Export, Save for Web, ...): starts in
    /// the folder [`export_start_dir`] picks (#10192), and once `then` has exported, remembers the
    /// chosen file's folder in `export.lastFolder`; a failed or cancelled export leaves it alone.
    pub(crate) fn pick_export(&mut self, suggested: &str, then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static) -> Result<Value, String> {
        self.pick_save_in(suggested, true, None, then)
    }

    fn pick_save_in(
        &mut self,
        suggested: &str,
        export: bool,
        copy: Option<SaveCopyFormats>,
        then: impl FnOnce(&mut Self, String) -> Result<Value, String> + 'static,
    ) -> Result<Value, String> {
        let mut suggested = PathBuf::from(suggested);
        let doc = self.session.active().map(|d| d.doc.id);
        // Closed documents' folders are no longer needed.
        let open: Vec<_> = self.session.documents().iter().map(|d| d.doc.id).collect();
        self.save_dirs.retain(|id, _| open.contains(id));
        if suggested.parent().is_some_and(|p| p.as_os_str().is_empty()) {
            let document_dir = doc
                .and_then(|d| self.save_dirs.get(&d).cloned())
                // An export doesn't start in a folder that is gone.
                .filter(|dir| !export || folder_exists(dir))
                .or_else(|| self.session.active().and_then(|d| d.path.as_deref()).and_then(|p| Path::new(p).parent()).map(Path::to_path_buf));
            let remembered = export.then(|| self.session.prefs().export.last_folder.clone());
            if let Some(dir) = export_start_dir(remembered.as_deref(), document_dir, folder_exists) {
                suggested = dir.join(suggested);
            }
        }
        let request = save_request(if export { "export" } else { "document" }, &suggested.to_string_lossy(), copy);
        self.ask_file(request, move |app, picked| {
            let Some(path) = path_strings(&picked).into_iter().next() else { return Err(UNEXPECTED.into()) };
            // Some pickers accept a typed extension outside their filter list.
            if copy.is_some_and(|formats| !formats.allows_path(Path::new(&path))) {
                return Err(tl!("Choose a file type that supports this document's colour mode and bit depth.").to_string());
            }
            // The web answers a download name, which has no folder to remember.
            let dir = doc.zip(Path::new(&path).parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf));
            // An export's folder only once it has exported: a failed one doesn't move where
            // the next export starts.
            if !export && let Some((doc, dir)) = dir.clone() {
                app.save_dirs.insert(doc, dir);
            }
            let exported = then(app, path.clone())?;
            if export {
                if let Some((doc, dir)) = dir {
                    app.save_dirs.insert(doc, dir);
                }
                app.remember_export_folder(&path);
            }
            Ok(exported)
        })
    }

    /// Remember the folder of an export that succeeded as where the next export dialog starts.
    /// The web answers a download name, which has no folder, and leaves it alone.
    fn remember_export_folder(&mut self, path: &str) {
        let Some(dir) = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()) else { return };
        let dir = dir.to_string_lossy().into_owned();
        if self.session.prefs().export.last_folder != dir {
            self.session.prefs.edit(|p| p.export.last_folder = dir);
        }
    }

    /// Ask for a file a command reads (a script, notes, a placed image, presets): `then` gets its
    /// name (the full path on the desktop) and bytes. A file that can't be read fails with
    /// "<file name>: <why>".
    pub(crate) fn pick_file_bytes(&mut self, then: impl FnOnce(&mut Self, String, Vec<u8>) -> Result<Value, String> + 'static) -> Result<Value, String> {
        self.pick_file_bytes_filtered(&[], then)
    }

    /// Same asynchronous picker, with a platform-native extension filter (desktop and web).
    pub(crate) fn pick_file_bytes_filtered(
        &mut self,
        extensions: &[&str],
        then: impl FnOnce(&mut Self, String, Vec<u8>) -> Result<Value, String> + 'static,
    ) -> Result<Value, String> {
        let request = self.open_file_request(PickKind::Open, None, extensions);
        self.ask_file(request, move |app, picked| {
            let (name, bytes) = read_picked(picked)?;
            then(app, name, bytes)
        })
    }

    /// [`Self::pick_file_bytes`] letting the user choose several files (File › Place Embedded,
    /// optionally titled as in #14110 instead of the system's default title): `then` gets every
    /// chosen file, in the order the dialog returns them. A desktop file is handed over by path and
    /// read only when [`PickedFile::read`] is called, so files waiting their turn hold no contents;
    /// the web hands over each file's contents.
    pub(crate) fn pick_files_bytes(
        &mut self,
        title: Option<String>,
        then: impl FnOnce(&mut Self, Vec<PickedFile>) -> Result<Value, String> + 'static,
    ) -> Result<Value, String> {
        let request = self.open_file_request(PickKind::OpenMany, title, &[]);
        self.ask_file(request, move |app, picked| {
            let files = picked_files(picked)?;
            then(app, files)
        })
    }

    fn open_file_request(&self, kind: PickKind, title: Option<String>, extensions: &[&str]) -> PickRequest {
        let mut request = open_filters(PickRequest::new(kind, "file"), extensions);
        if let Some(title) = title {
            request = request.title(title);
        }
        if let Some(dir) = last_used_dir(&self.ui.recent_files) {
            request = request.directory(dir);
        }
        request
    }

    /// File › Open: opens every chosen file in the order the dialog returns them, reporting each
    /// failure (see [`Self::open_paths_in_order`]).
    pub fn open_dialog_file(&mut self) -> Result<Value, String> {
        let request = self.open_many_request();
        self.ask_file(request, |app, picked| {
            match picked {
                Picked::Paths(_) => {
                    // The order the dialog returns, not name order; the last is active.
                    app.open_paths_in_order(&path_strings(&picked));
                }
                Picked::Contents(files) => app.open_contents(files),
                Picked::Saved { .. } | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => return Err(UNEXPECTED.into()),
            }
            Ok(Value::Null)
        })
    }

    /// File › Open as Smart Object…: the File › Open dialog (same filters, several files); each
    /// chosen file opens as its own document holding one embedded smart object, in name order,
    /// reporting each failure as File › Open does.
    pub fn open_dialog_as_smart_object(&mut self) -> Result<Value, String> {
        let request = self.open_many_request();
        self.ask_file(request, |app, picked| {
            let Picked::Paths(_) = picked else { return Err(UNEXPECTED.into()) };
            // Each file is a tracked open job like File › Open's, in name order. No `show_if_open`:
            // the file opens as another document even when it is already open.
            let mut paths: Vec<String> = path_strings(&picked).iter().map(|p| crate::file_open::absolute(p)).collect();
            paths.sort_by(|a, b| crate::file_open::natural_cmp(a, b));
            let batch = crate::jobs_ui::begin_batch(app);
            for (seq, path) in paths.iter().enumerate() {
                let order = crate::jobs_ui::OpenOrder { batch, seq };
                if let Err(e) = crate::jobs_ui::start_open_as_smart_object(app, path, Some(order)) {
                    app.open_failed(&crate::file_open::display_name(path), &e);
                }
            }
            crate::jobs_ui::end_batch(app);
            app.sync_views();
            Ok(Value::Null)
        })
    }

    /// The File › Open picker: every supported type, several files, starting in the folder of the
    /// most recent file.
    fn open_many_request(&self) -> PickRequest {
        let request = open_filters(PickRequest::open_many("document"), &[]);
        match last_used_dir(&self.ui.recent_files) {
            Some(dir) => request.directory(dir),
            None => request,
        }
    }

    /// Opens each picked file's contents as its own document, in name order (like
    /// [`Self::open_paths`]), reporting each failure.
    fn open_contents(&mut self, mut files: Vec<(String, Vec<u8>)>) {
        files.sort_by(|a, b| crate::file_open::natural_cmp(&a.0, &b.0));
        for (name, bytes) in files {
            if let Err(e) = self.open_bytes(&name, &bytes) {
                self.open_failed(&name, &e);
            }
        }
    }

    /// Run `next` on the result of the open dialog's action once it is answered; its result
    /// replaces the action's. False when no dialog is open.
    pub(crate) fn after_file_dialog(&mut self, next: impl FnOnce(&mut Self, Result<Value, String>) -> Result<Value, String> + 'static) -> bool {
        let Some(p) = self.file_dialog.as_mut() else { return false };
        let then = std::mem::replace(&mut p.then, Box::new(|_, _| Ok(Value::Null)));
        p.then = Box::new(move |app, picked| {
            let r = then(app, picked);
            next(app, r)
        });
        true
    }

    /// The document `id` becomes active again: an answer applies to the document it was asked
    /// for, even when another one was activated while the dialog was open. `Err` once it's closed.
    pub(crate) fn refocus(&mut self, id: DocId) -> Result<(), String> {
        let i = self.session.documents().iter().position(|d| d.doc.id == id).ok_or("no document")?;
        self.session.set_active(i);
        Ok(())
    }

    /// Complete a deferred operation on its requesting document without stealing the selected
    /// tab. Restore by ID as script events can close or reorder documents during a save.
    pub(crate) fn with_document<T>(&mut self, id: DocId, run: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let active = self.session.active().map(|d| d.doc.id);
        self.refocus(id)?;
        let result = run(self);
        if let Some(active) = active {
            let _ = self.refocus(active);
        }
        result
    }

    /// The active document's id, for [`Self::refocus`].
    pub(crate) fn active_doc_id(&self) -> Result<DocId, String> {
        Ok(self.session.active().ok_or("no document")?.doc.id)
    }

    /// Shows the queued native pickers with `parent` (the window) as their parent and settles the
    /// answers, once at the end of a frame. A second pass in the same frame pumps again with no
    /// input of its own to apply: the panels hold the only state. The Save extension policy comes
    /// from Preferences ▸ File Handling: lowercase an upper-case extension when that is on, and
    /// always add the suggested extension to a bare name (the format follows the extension).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn pump_files(&mut self, parent: &eframe::Frame) {
        let policy = craft::dialogs::service::ExtensionPolicy {
            lowercase: self.session.prefs().file_handling.lowercase_extension,
            add_suggested: true,
            picked_type: true,
        };
        // PLAN: `Services::file_panels: Option<craft::dialogs::FilePanels>` is dialogs-w59's field.
        let Some(panels) = self.services.file_panels.as_mut() else { return };
        panels.set_extension_policy(policy);
        panels.pump_with(&mut self.framework.dialogs, Some(parent));
    }

    /// Called every frame: continues the action that asked once the file service has the answer.
    /// Its errors go to the status bar.
    pub fn poll_file_dialog(&mut self) {
        let Some(picked) = self.take_picked() else { return };
        let _ = self.finish_file_dialog(picked);
    }

    /// Ends the file dialog in progress: its action continues with `picked`, and its errors go to
    /// the status bar. Returns the action's result.
    fn finish_file_dialog(&mut self, picked: Picked) -> Result<Value, String> {
        let Some(Pending { then, .. }) = self.file_dialog.take() else { return Err(CANCELLED.into()) };
        let r = then(self, picked);
        if let Err(e) = &r
            && e != CANCELLED
        {
            self.ui.status = e.clone();
            self.ui.status_error = true;
        }
        r
    }
}

/// The web dialog request for `request`.
#[cfg(target_arch = "wasm32")]
fn web_request(request: &PickRequest) -> FileDialogRequest {
    let initial_dir = request.directory.as_ref().map(|d| path_string(d));
    match request.kind {
        PickKind::Open | PickKind::OpenMany => {
            // A request with several types is File › Open's; one type is a command's own list.
            let extensions = (request.filters.len() == 1).then(|| request.filters[0].extensions.clone());
            FileDialogRequest::Open { multiple: request.kind == PickKind::OpenMany, initial_dir, extensions }
        }
        PickKind::Folder => FileDialogRequest::Folder { initial_dir },
        PickKind::Save => {
            let name = request.file_name.clone().unwrap_or_default();
            let suggested = request.directory.as_ref().map_or(name.clone(), |dir| path_string(&dir.join(&name)));
            FileDialogRequest::Save { suggested }
        }
    }
}

/// The answer of a web dialog as the host's [`Picked`].
#[cfg(target_arch = "wasm32")]
fn web_picked(answer: Option<FileDialogAnswer>) -> Picked {
    match answer {
        None => Picked::Cancelled,
        Some(FileDialogAnswer::Paths(paths)) => Picked::Paths(paths.into_iter().map(PathBuf::from).collect()),
        Some(FileDialogAnswer::SaveTo(path)) => Picked::Saved { path: PathBuf::from(path), file_type: None },
        Some(FileDialogAnswer::FolderTo(path)) => Picked::Paths(vec![PathBuf::from(path)]),
        Some(FileDialogAnswer::Contents(name, bytes)) => Picked::Contents(vec![(name, bytes)]),
        Some(FileDialogAnswer::ContentsMany(files)) => Picked::Contents(files),
        Some(FileDialogAnswer::Unavailable(why)) => Picked::Unavailable(why),
    }
}

/// Whether `dir` is a folder on disk. The web has no file system, so nothing is remembered there.
#[cfg(not(target_arch = "wasm32"))]
fn folder_exists(dir: &Path) -> bool {
    dir.is_dir()
}

#[cfg(target_arch = "wasm32")]
fn folder_exists(_dir: &Path) -> bool {
    false
}

/// The folder an export dialog starts in (#10192): the `remembered` folder of the last export
/// while `exists` finds it, else the document's own folder (`document_dir`, whatever it is),
/// else none, which leaves the choice to the system dialog.
pub(crate) fn export_start_dir(remembered: Option<&str>, document_dir: Option<PathBuf>, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    remembered.filter(|d| !d.is_empty()).map(PathBuf::from).filter(|d| exists(d)).or(document_dir)
}

/// The folder an open dialog starts in: the directory of the most recent file (UI-217-3).
pub(crate) fn last_used_dir(recent: &[String]) -> Option<String> {
    let dir = Path::new(recent.first()?).parent()?;
    (!dir.as_os_str().is_empty()).then(|| dir.to_string_lossy().into_owned())
}

/// The name and bytes of the single file an open dialog answered with.
fn read_picked(picked: Picked) -> Result<(String, Vec<u8>), String> {
    match picked {
        Picked::Contents(files) => files.into_iter().next().ok_or_else(|| CANCELLED.into()),
        Picked::Paths(paths) => read_path(path_string(paths.first().ok_or(CANCELLED)?)),
        Picked::Saved { .. } | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => Err(UNEXPECTED.into()),
    }
}

/// The file at `path` with its contents; one that can't be read fails with "<file name>: <why>".
fn read_path(path: String) -> Result<(String, Vec<u8>), String> {
    let bytes = photocraft_format::read_file(Path::new(&path)).map_err(|e| format!("{}: {e}", display_name(&path)))?;
    Ok((path, bytes))
}

/// One file chosen in the several-file picker ([`PhotocraftApp::pick_files_bytes`]).
#[derive(Debug)]
pub(crate) enum PickedFile {
    /// A file on disk (desktop), by its full path: read when its turn comes.
    Path(String),
    /// A file's name and contents (the web, where the page gets contents rather than paths).
    Contents(String, Vec<u8>),
}

impl PickedFile {
    /// Its name (the full path on the desktop) and contents, reading a desktop file now; one that
    /// can't be read fails with "<file name>: <why>", as in [`PhotocraftApp::pick_file_bytes`].
    pub(crate) fn read(self) -> Result<(String, Vec<u8>), String> {
        match self {
            Self::Path(path) => read_path(path),
            Self::Contents(name, bytes) => Ok((name, bytes)),
        }
    }
}

/// The files chosen in a several-file picker, in the order the dialog returns them (none: Cancel).
fn picked_files(picked: Picked) -> Result<Vec<PickedFile>, String> {
    let files: Vec<PickedFile> = match picked {
        Picked::Paths(paths) => paths.iter().map(|p| PickedFile::Path(path_string(p))).collect(),
        Picked::Contents(files) => files.into_iter().map(|(name, bytes)| PickedFile::Contents(name, bytes)).collect(),
        Picked::Saved { .. } | Picked::Cancelled | Picked::Unavailable(_) | Picked::Failed(_) | Picked::Downloaded(_) => return Err(UNEXPECTED.into()),
    };
    if files.is_empty() {
        return Err(CANCELLED.into());
    }
    Ok(files)
}

#[cfg(test)]
#[path = "file_dialog/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "file_dialog/place_title_tests.rs"]
mod place_title_tests;
