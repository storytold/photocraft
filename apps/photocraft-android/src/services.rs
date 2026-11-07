#![cfg(target_os = "android")]

//! The host seams PhotoCraft asks its app shell for: the [`Services`] struct the desktop app fills
//! with `rfd`, `arboard`, `open` and its config directory, filled here with Android equivalents.
//!
//! | Desktop (`apps/photocraft/src/services.rs`) | Android (this module) |
//! |---|---|
//! | `rfd` file dialogs | the SAF picker behind [`Services::file_dialog`] (the async request/reply seam) plus [`spawn_publish`]; answers are paths the app reads or writes itself |
//! | `photocraft_io::import` / `export` | the same calls (the engine's own decode/encode path) |
//! | config dir from `app_dirs` | `AndroidApp::internal_data_path()` |
//! | `write_atomic` | `photocraft_format::atomic_write`, plus a write-back to the SAF source when the document has one |
//! | `arboard` clipboard | not wired (no Android backend in the desktop crate; the session clipboard still works) |
//! | `open::that` | not wired (`ctx.open_url` covers the About links) |
//!
//! Nothing else changes: the codecs, the document model and the recovery store are the desktop
//! ones, so an Android build imports and exports exactly what the desktop build does.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use photocraft_ui_egui::{FileDialogAnswer, FileDialogFn, FileDialogReply, FileDialogRequest, OsEvent, Recoverable, Services};

use crate::saf::{self, Sources};

/// Where everything the app persists lives, all under `AndroidApp::internal_data_path()` (the
/// activity's `filesDir`): app-private, backed up like any other app data, never on shared storage.
#[derive(Clone)]
pub struct Paths {
    /// The activity's `filesDir`; the Java bridge writes `import-manifest.txt` here.
    pub data: PathBuf,
    /// Files the SAF picker copied out of the user's storage; the documents the app opens.
    pub imports: PathBuf,
    /// Scratch space for saves and exports: a save writes here first, and the file is published to
    /// the destination the user chose (`ACTION_CREATE_DOCUMENT`) once it is complete.
    pub exports: PathBuf,
}

impl Paths {
    /// `<data>/{import,export}` plus the app's config directory, created if missing.
    pub fn resolve(data: PathBuf) -> Paths {
        let paths = Paths { imports: data.join("import"), exports: data.join("export"), data };
        for dir in [&paths.imports, &paths.exports] {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("photocraft-android: cannot create {}: {e}", dir.display());
            }
        }
        paths
    }

    /// Preferences, saved layouts and the preset store (the desktop app's config directory).
    pub fn config(&self) -> PathBuf {
        self.data.join("config")
    }

    /// Crash-recovery autosaves (Preferences ▸ File Handling).
    pub fn recovery(&self) -> PathBuf {
        self.data.join("Recovery")
    }
}

/// The Java activity object and the SAF state a picker needs.
#[derive(Clone)]
pub struct Saf {
    /// `AndroidApp::activity_as_ptr()` — the Java activity (not the Application: only the activity
    /// can start a document picker for a result).
    pub activity: usize,
    /// Import path → the `content://` URI it came from (File ▸ Save writes back there).
    pub sources: Sources,
    /// A clone of the egui context: a background picker wakes the UI so its result is drained.
    pub ctx: egui::Context,
}

/// Files a background picker delivered, waiting for the app's next frame.
///
/// PhotoCraft's file services are synchronous (`pick_open_paths` answers "which files?" inside the
/// frame), and a picker cannot be answered synchronously without freezing the UI thread, so the
/// hook starts a background task and answers "nothing". The picked paths arrive here a moment
/// later and are handed to the app through its own `os_events` service — `OsEvent::Open` is exactly
/// what the desktop shell delivers for a Finder double-click or an "Open With", so the app runs its
/// normal `open_paths` path (name, remembered path, Open Recent, warnings).
#[derive(Clone, Default)]
pub struct Incoming(Arc<Mutex<Vec<String>>>);

impl Incoming {
    /// Queue picked paths for the app's next frame.
    pub fn push(&self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        self.0.lock().unwrap_or_else(PoisonError::into_inner).extend(paths);
    }

    /// Everything queued since the last frame.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The `os_events` service: the picked files as an open request.
    pub fn events(&self) -> Vec<OsEvent> {
        let paths = self.take();
        if paths.is_empty() { Vec::new() } else { vec![OsEvent::Open(paths)] }
    }
}

/// Run `f`; a panic inside it becomes an `Err` naming `what`.
///
/// PhotoCraft's `AGENTS.md` puts the last-resort guard in the app shell: a panic that escapes the
/// codecs must not take the process (and the user's open documents) with it. This is the same
/// guard the desktop shell wraps Open/Export in.
fn guard<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(format!("{what} failed with an internal error (logged); your open documents are unchanged")))
}

/// Ask where a save/export should land, wait for the app to finish writing the scratch file, then
/// publish it there. Nothing is lost when the user cancels: the file stays in the export directory.
fn spawn_publish(saf: Saf, scratch: String) {
    std::thread::spawn(move || {
        let title = scratch.rsplit('/').next().unwrap_or("PhotoCraft").to_owned();
        if !saf::pick_save(saf.activity, &title, std::time::Duration::from_secs(300)) {
            log::info!("SAF: no save destination chosen; the file stays at {scratch}");
            return;
        }
        // The app writes the file straight after the dialog: wait until it exists and its size has
        // stopped changing for a moment (the encoder may stream into it).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
        let (mut size, mut stable_since) = (0u64, std::time::Instant::now());
        loop {
            let now = std::fs::metadata(&scratch).map(|m| m.len()).unwrap_or(0);
            if now != size {
                size = now;
                stable_since = std::time::Instant::now();
            } else if size > 0 && stable_since.elapsed() > std::time::Duration::from_millis(1500) {
                break;
            }
            if std::time::Instant::now() > deadline {
                log::warn!("SAF: giving up on publishing {scratch} (nothing was written)");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        if saf::publish(saf.activity, &scratch) {
            log::info!("SAF: published {scratch} ({size} bytes) to the chosen destination");
        } else {
            log::warn!("SAF: publishing {scratch} failed; the file stays in the export directory");
        }
    });
}

/// Crash recovery: background incremental `.pcraft` autosaves into `dir` (`None`: no data
/// directory, so autosaves fail and nothing is recovered). Recovered documents keep their entries
/// until a newer autosave replaces them or they are saved or closed — the desktop behaviour.
fn recovery_services(dir: Option<PathBuf>) -> Services {
    type Shared = Rc<RefCell<Option<photocraft_format::RecoveryStore>>>;
    let store: Shared = Rc::new(RefCell::new(dir.map(photocraft_format::RecoveryStore::new)));

    fn with_store<R>(store: &Shared, f: impl FnOnce(&mut photocraft_format::RecoveryStore) -> R) -> Result<R, String> {
        let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
        Ok(f(slot.as_mut().ok_or("no data directory")?))
    }

    let (s1, s2, s3, s4) = (store.clone(), store.clone(), store.clone(), store.clone());
    Services {
        autosave: Some(Box::new(move |doc: &Arc<photocraft_doc::Document>, revision: u64, path: Option<&str>| {
            with_store(&s1, |s| s.autosave(doc, revision, path.map(str::to_string)))
        })),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
            // Upstream now lists entries and lets the shell decode each one on a background
            // worker; the store already decoded them here, so the loader just hands the document
            // over.
            let found = with_store(&s3, |s| s.recover()).unwrap_or_default();
            found
                .into_iter()
                .map(|(e, doc)| Recoverable { key: e.info.key, name: e.info.document_name, path: e.info.original_path, load: Box::new(move || Ok(doc)) })
                .collect()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&s4, |s| s.adopt(id, key));
        })),
        ..Default::default()
    }
}

/// A scratch path for a save/export: the suggested name, sanitized, inside the app's export
/// directory (the engine writes with plain file APIs, so a real path is what it needs).
fn export_path(dir: &Path, name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let leaf = if leaf.is_empty() { "PhotoCraft" } else { leaf };
    let path = dir.join(leaf);
    log::info!("photocraft-android: save dialog → {}", path.display());
    path.to_string_lossy().into_owned()
}

/// Every seam the desktop shell fills, filled for Android.
pub fn services(paths: &Paths, saf: &Saf, incoming: &Incoming) -> Services {
    let config = paths.config();
    if let Err(e) = std::fs::create_dir_all(&config) {
        log::warn!("photocraft-android: cannot create {}: {e}", config.display());
    }
    let prefs_file = config.join("preferences.json");

    // File dialogs: upstream now asks through `Services::file_dialog` (a request, a reply sent from
    // any thread) instead of the old synchronous `pick_open`/`pick_save` hooks. The SAF pickers run
    // on their own thread — exactly what this API wants — and answer through the app's own reply
    // channel. The app then reads the copied files / writes the scratch path itself.
    let file_dialog = {
        let (saf, paths) = (saf.clone(), paths.clone());
        let show: FileDialogFn = Box::new(move |request: FileDialogRequest, _parent: Option<&eframe::Frame>, reply: FileDialogReply| {
            let (saf, paths) = (saf.clone(), paths.clone());
            std::thread::spawn(move || {
                let answer = match request {
                    FileDialogRequest::Open { multiple, .. } => {
                        let picked = saf::pick_media(saf.activity, &paths.data, std::time::Duration::from_secs(300));
                        if picked.is_empty() {
                            None
                        } else {
                            let mut files = Vec::with_capacity(picked.len());
                            for (local, uri) in picked {
                                saf.sources.remember(&local, &uri);
                                files.push(local);
                            }
                            // A command that reads one file takes the first (matching the desktop's
                            // single-selection dialog); File ▸ Open keeps every picked file.
                            if !multiple {
                                files.truncate(1);
                            }
                            log::info!("SAF: {} file(s) picked", files.len());
                            saf.ctx.request_repaint();
                            Some(FileDialogAnswer::Paths(files))
                        }
                    }
                    // Settings paths, "save into a folder" commands: the app's own export
                    // directory, the one writable folder it has (SAF trees are `content://` and
                    // the engine writes plain paths).
                    FileDialogRequest::Folder { .. } => Some(FileDialogAnswer::FolderTo(paths.exports.to_string_lossy().into_owned())),
                    // Save/Save As/Export: answer a scratch path in the export directory and ask
                    // where it should land; the watcher publishes the finished file there.
                    FileDialogRequest::Save { suggested } => {
                        let scratch = export_path(&paths.exports, &suggested);
                        spawn_publish(saf.clone(), scratch.clone());
                        Some(FileDialogAnswer::SaveTo(scratch))
                    }
                };
                reply.send(answer);
            });
        });
        show
    };

    // Every document write goes through here: crash-safe (temp file + fsync + rename), and a
    // document the user opened from their own storage is written back to it as well.
    let write = {
        let saf = saf.clone();
        Box::new(move |path: &str, bytes: &[u8]| -> Result<(), String> {
            photocraft_format::atomic_write(Path::new(path), bytes).map_err(|e| format!("{path}: {e}"))?;
            if let Some(uri) = saf.sources.source_of(path) {
                let (activity, local) = (saf.activity, path.to_owned());
                std::thread::spawn(move || {
                    if saf::publish_to(activity, &uri, &local) {
                        log::info!("SAF: saved back to {uri}");
                    } else {
                        log::warn!("SAF: cannot write back to {uri}; the app's copy at {local} holds the work");
                    }
                });
            }
            Ok(())
        }) as photocraft_ui_egui::WriteFn
    };

    let os_events = {
        let incoming = incoming.clone();
        Box::new(move || incoming.events()) as photocraft_ui_egui::OsEventsFn
    };

    let append_text = {
        Box::new(move |path: &str, text: &str| -> Result<(), String> {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{path}: {e}"))?;
            file.write_all(text.as_bytes()).map_err(|e| format!("{path}: {e}"))
        }) as photocraft_ui_egui::AppendTextFn
    };

    let read_prefs = prefs_file.clone();
    Services {
        // The engine's own importer and exporter: the desktop, the CLI, the web build and Android
        // all decode and encode through `photocraft_io`.
        import: Some(Box::new(|name: &str, bytes: &[u8], options: &photocraft_io::ImportOptions| {
            // Upstream added per-call import options (SVG group depth, …); use them so Android
            // imports exactly what the desktop does.
            guard("Open", || photocraft_io::import_with_options(name, bytes, options).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))
        })),
        export: Some(Box::new(|doc: &photocraft_doc::Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let mut opts = photocraft_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            opts.encode.webp_lossless = settings.webp_lossless;
            if let Some(q) = settings.webp_quality {
                opts.encode.webp_quality = q;
            }
            opts.tiff_layers = settings.tiff_layers;
            opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
            guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        file_dialog: Some(file_dialog),
        write: Some(write),
        // Screenshots (`ui.screenshot`) and `ui.render` encode here.
        encode_png: Some(Box::new(|w: u32, h: u32, rgba: &[u8]| {
            let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        // Preferences live in the app's private config directory, written crash-safely.
        load_prefs: Some(Box::new(move || std::fs::read_to_string(&read_prefs).ok())),
        save_prefs: Some(Box::new(move |text: &str| photocraft_format::atomic_write(&prefs_file, text.as_bytes()).map_err(|e| e.to_string()))),
        append_text: Some(append_text),
        // The picker's results (File ▸ Open and every one-file command).
        os_events: Some(os_events),
        ..recovery_services(Some(paths.recovery()))
    }
}
