//! PhotoCraft on iPadOS and Android tablets, hosted by `craft-mobile`.
//!
//! The same [`PhotocraftApp`] as the desktop and web builds. The native shells (SwiftUI with
//! CraftKit, Compose with craftkit) own the app and its panels; this crate only says how
//! PhotoCraft meets them:
//! - **Files:** File › Open asks the shell's document picker. A file the shell can write back
//!   (picked, or opened in place from Files) opens with its document path, as a file from disk
//!   does, so File › Save writes it back without a sheet; the shell confirms the write, and only
//!   then is the document saved. A flat file (PNG, JPEG…) is written back too while it keeps the
//!   whole document (one plain layer). Everything else (Save As, a copy from Share or Photos, a
//!   format the file can't keep) goes to the shell's exporter, as on the web.
//!   Saves, Save a Copy and Export As encode on a worker (PhotoCraft's background save job), so a
//!   large document never holds up the main thread; the bytes go to the shell on the next frame,
//!   and the shells keep the app running in the background until they have them
//!   ([`CraftApp::busy`]).
//! - **Crash recovery:** leaving the screen starts writing recovery copies of unsaved documents
//!   on PhotoCraft's autosave workers and returns at once (the OS ends apps whose main thread
//!   hangs); the app stays [busy](CraftApp::busy) until they are on disk, so the shells keep it
//!   running meanwhile. A copy is removed once its document is saved or closed.
//! - **Plug-ins** are bundled-only on iOS (App Store guideline 2.5.2): no Install Plug-in… and no
//!   plug-ins folder. Android keeps both.
//! - **Menus:** PhotoCraft's own, plus what the tablet has: File › Import from Photos… (the
//!   system photo picker; photos open as copies) and File › Share… (the active document as Save a
//!   Copy writes it, in its own format, encoded on the save worker, then the share sheet).
//! - **Clipboard:** Edit › Paste, Paste in Place and New from Clipboard ask the shell to read the
//!   clipboard (only now: iPadOS may ask the user to allow it) and run when it answers, as ⌘V does.
//!   While the clipboard still holds the image PhotoCraft copied, the shell doesn't read it back
//!   and PhotoCraft pastes its own copy, layers and all. Copies go out as images.
//! - **Native controls** map to PhotoCraft's own commands, exactly as its menu items run them.
//! - **Panels** see the active document and its layers, top first.
//! - **Power:** while the device is hot or saving power, live previews run on a reduced copy
//!   (Preferences › Performance › Low Resolution Previews), whatever the preference says.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use craft_mobile::{Action, CraftApp, DocumentInfo, Item, Pasted, PickedFile, Platform, Power, SaveRoute, Snapshot, Stylus, WriteOutcome};
use egui::TouchPhase;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::{DocId, Document, Layer};
use photocraft_engine::Session;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::stylus::PenSample;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, HostMenu, HostMenuItem, Inbox, PhotocraftApp, Recoverable, Services};
use serde_json::{Value, json};

/// PhotoCraft as `craft-mobile` hosts it.
pub struct PhotoCraftMobile {
    app: PhotocraftApp,
    /// Copies the system handed over, opened by the app on its next frame.
    inbox: Inbox,
    /// Files that can be saved back where they came from, as (document path, contents), opened
    /// on the next frame.
    located: Located,
    /// Document paths the app wrote in place during this frame.
    issued: Issued,
    /// Writes in place the shell hasn't confirmed yet, oldest first.
    writing: Vec<PendingWrite>,
    /// Saves a worker encoded, waiting for the next frame to go to the shell.
    encoded: Encoded,
    /// The user's Low Resolution Previews while the power state overrides it.
    previews: PreviewOverride,
    /// Recovery copies started as the app left the screen.
    suspended: Shared<Suspended>,
    /// The shell.
    platform: Platform,
    /// The paste waiting for the shell to read the clipboard.
    pastes: Shared<Pastes>,
    /// The paste the clipboard this frame brought answers, run before the app's own logic.
    answered: Option<(String, Value)>,
    /// PhotoCraft's own copy as it put it on the clipboard, while the clipboard still holds it
    /// and a paste is reading it (this frame only).
    own_copy: OwnCopy,
}

/// PhotoCraft's own copy's pixels for the clipboard service: see [`PhotoCraftMobile::own_copy`].
type OwnCopy = Rc<RefCell<Option<(u32, u32, Vec<u8>)>>>;

/// File › Import from Photos….
const IMPORT_PHOTOS: &str = "mobile.importFromPhotos";
/// File › Share….
const SHARE: &str = "mobile.share";
/// What a Save a Copy for File › Share… writes to: the bytes go to the share sheet.
const SHARE_ROOT: &str = "share://";

/// The clipboard service's answer outside a paste: the clipboard is read only when the user pastes.
/// PhotoCraft keeps its own copy then, and the message is never shown ([`quiet_clipboard`]).
const NOT_PASTING: &str = "The clipboard is read when you paste";

/// The status never reports [`NOT_PASTING`]: it isn't a failure the user could act on.
fn quiet_clipboard(app: &mut PhotocraftApp) {
    if app.ui.status_error && app.ui.status == NOT_PASTING {
        app.ui.status.clear();
        app.ui.status_error = false;
    }
}

/// The commands that read the system clipboard: they wait for the shell to read it.
fn reads_clipboard(id: &str) -> bool {
    matches!(id, "edit.paste" | "edit.pasteSpecial.pasteInPlace" | "file.newFromClipboard")
}

/// A paste the user asked for (Edit › Paste, Paste in Place, New from Clipboard), waiting for the
/// shell to read the clipboard: the command and its parameters.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pastes {
    asked: Option<(String, Value)>,
}

impl Pastes {
    /// Run `id`, a command that reads the clipboard: `true` when the clipboard is here (it runs
    /// now), `false` when it must wait for the shell to read it.
    fn run(&mut self, id: &str, params: &Value, pasted: bool) -> bool {
        if !pasted {
            self.asked = Some((id.to_string(), params.clone()));
        }
        pasted
    }

    /// A frame begins: the paste the clipboard it brought answers.
    fn answered(&mut self, pasted: bool) -> Option<(String, Value)> {
        if pasted { self.asked.take() } else { None }
    }
}

/// State the services and the frame share, one at a time: see [`update`].
type Shared<T> = Rc<Cell<T>>;

/// Recovery copies written as the app leaves the screen. They write on PhotoCraft's autosave
/// workers; the app stays busy until each is on disk, and each is removed once its document has
/// no unsaved changes left (saved, or closed).
#[derive(Debug, Default, PartialEq, Eq)]
struct Suspended {
    /// Document id → the revision being written, until a write of it (or a later one) ends.
    writing: HashMap<u64, u64>,
    /// Documents with a copy from a suspend.
    kept: HashSet<u64>,
}

impl Suspended {
    /// A copy of document `id` at `revision` is queued.
    fn queued(&mut self, id: u64, revision: u64) {
        self.writing.insert(id, revision);
        self.kept.insert(id);
    }

    /// Autosave writes ended, as (document id, revision, outcome). The writer skips to the newest
    /// queued revision, so a later revision ends the wait for an earlier one.
    fn ended(&mut self, done: &[(u64, u64, Result<(), String>)]) {
        for (id, revision, outcome) in done {
            if self.writing.get(id).is_some_and(|waiting| waiting <= revision) {
                self.writing.remove(id);
                match outcome {
                    Ok(()) => log::info!("photocraft-mobile: recovery copy of document {id} (revision {revision}) written"),
                    Err(e) => log::warn!("photocraft-mobile: no recovery copy of document {id}: {e}"),
                }
            }
        }
    }

    /// Whether a copy is still being written.
    fn writing(&self) -> bool {
        !self.writing.is_empty()
    }

    /// Forget the documents `unsaved` no longer says have unsaved changes (saved, or closed),
    /// and return them: their copies are stale.
    fn stale(&mut self, unsaved: impl Fn(u64) -> bool) -> Vec<u64> {
        let stale: Vec<u64> = self.kept.iter().copied().filter(|&id| !unsaved(id)).collect();
        for id in &stale {
            self.kept.remove(id);
            self.writing.remove(id);
        }
        stale
    }
}

/// Run `f` on the value in `cell`. The services and the frame never call each other while one
/// runs, so nothing else sees the value taken meanwhile.
fn update<T: Default, R>(cell: &Cell<T>, f: impl FnOnce(&mut T) -> R) -> R {
    let mut value = cell.take();
    let result = f(&mut value);
    cell.set(value);
    result
}

/// See [`PhotoCraftMobile::encoded`]: (path, file) pairs. A mutex, not a channel, so the app can
/// see it's still busy while saves wait here.
type Encoded = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Live previews follow the power state: on a reduced copy while it's constrained.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PreviewOverride {
    /// The user's setting, kept while the override holds.
    user: Option<bool>,
}

impl PreviewOverride {
    /// The Low Resolution Previews setting to apply for `power`, given the current one; `None`
    /// leaves it alone. The user's own choice comes back once the device is fine again, unless
    /// they changed it meanwhile.
    fn apply(&mut self, power: Power, current: bool) -> Option<bool> {
        match (power.constrained(), self.user) {
            (true, None) => {
                self.user = Some(current);
                (!current).then_some(true)
            }
            (false, Some(user)) => {
                self.user = None;
                // Still the override's value: give the user's back.
                (current && !user).then_some(user)
            }
            _ => None,
        }
    }
}

/// See [`PhotoCraftMobile::located`]. A `Cell`: the services and the frame take turns, and
/// neither can find it borrowed.
type Located = Rc<Cell<Vec<(String, Vec<u8>)>>>;
/// See [`PhotoCraftMobile::issued`].
type Issued = Rc<Cell<Vec<String>>>;

/// A document written back to its own file at `revision`: saved once the shell confirms.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingWrite {
    path: String,
    doc: DocId,
    revision: u64,
}

impl CraftApp for PhotoCraftMobile {
    fn create(ctx: &egui::Context, wgpu: Option<eframe::egui_wgpu::RenderState>, platform: Platform) -> Self {
        let (inbox, located, issued, encoded) = (Inbox::default(), Located::default(), Issued::default(), Encoded::default());
        let (suspended, pastes, own_copy) = (Shared::<Suspended>::default(), Shared::<Pastes>::default(), OwnCopy::default());
        let mut services = services(&platform, inbox.clone(), located.clone(), issued.clone(), encoded.clone(), suspended.clone());
        tablet_menus(&mut services, &platform, pastes.clone(), own_copy.clone());
        let app = create_app(ctx, wgpu, services);
        Self {
            app,
            inbox,
            located,
            issued,
            writing: Vec::new(),
            encoded,
            previews: PreviewOverride::default(),
            suspended,
            platform,
            pastes,
            answered: None,
            own_copy,
        }
    }

    fn wgpu_configuration() -> eframe::egui_wgpu::WgpuConfiguration {
        let mut config = eframe::egui_wgpu::WgpuConfiguration::default();
        // Large canvases need the adapter's own texture limits, as on the desktop.
        photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut config.wgpu_setup);
        config
    }

    fn open_file(&mut self, ctx: &egui::Context, file: PickedFile) {
        match file.document_path() {
            Some(path) => push(&self.located, (path, file.bytes)),
            None => self.inbox.lock().unwrap_or_else(PoisonError::into_inner).push((file.name, file.bytes)),
        }
        ctx.request_repaint();
    }

    /// The document is saved now, at the revision that was written, if it still lives in that
    /// file; a failure is reported (the shell offers the exporter for the same bytes).
    fn written(&mut self, ctx: &egui::Context, outcome: &WriteOutcome) {
        let pending = self.writing.iter().position(|w| w.path == outcome.path).map(|i| self.writing.remove(i));
        let name = photocraft_ui_egui::file_open::display_name(&outcome.path);
        match (&outcome.result, pending) {
            (Ok(()), Some(write)) => {
                mark_saved(&mut self.app.session, &write);
                log::info!("photocraft-mobile: wrote {name} in place");
            }
            (Ok(()), None) => log::info!("photocraft-mobile: wrote {name} in place (a copy)"),
            (Err(e), _) => photocraft_ui_egui::notices::error(&mut self.app, format!("Couldn't save {name}: {e}")),
        }
        ctx.request_repaint();
    }

    fn action(&mut self, ctx: &egui::Context, action: &Action) -> Result<(), String> {
        let menu = |app: &mut PhotocraftApp, id: &str| {
            // As its menu item runs it: enable rules, dialogs and status messages included.
            photocraft_ui_egui::shortcut_dispatch::dispatch(app, ctx, id);
            Ok(())
        };
        let invoke = |app: &mut PhotocraftApp, id: &str, params| photocraft_ui_egui::menus::invoke(app, ctx, id, params).map(drop);
        match action {
            Action::Open => menu(&mut self.app, "file.open"),
            Action::Save => menu(&mut self.app, "file.save"),
            Action::Undo => menu(&mut self.app, "edit.undo"),
            Action::Redo => menu(&mut self.app, "edit.redo"),
            Action::Select { id } => invoke(&mut self.app, "layer.select", json!({ "layer": id })),
            Action::SetVisible { id, visible } => invoke(&mut self.app, "layer.setProps", json!({ "layer": id, "visible": visible })),
            Action::Command { id, params } if params.is_null() => menu(&mut self.app, id),
            Action::Command { id, params } => invoke(&mut self.app, id, params.clone()),
        }
    }

    fn snapshot(&self) -> Snapshot {
        let Some(st) = self.app.session.active() else {
            return Snapshot { items_title: "Layers".into(), ..Snapshot::default() };
        };
        let doc = &st.doc;
        let mut items = Vec::new();
        let selected = |id| st.active_layer == Some(id) || st.selected_layers.contains(&id);
        layer_rows(&doc.layers, 0, &selected, &mut items);
        Snapshot {
            document: Some(DocumentInfo {
                name: doc.name.clone(),
                detail: format!("{} × {} px", doc.size.width, doc.size.height),
                dirty: st.revision != st.saved_revision,
            }),
            can_undo: st.history.can_undo(),
            can_redo: st.history.can_redo(),
            items_title: "Layers".into(),
            items,
        }
    }

    /// Start writing crash-recovery copies of every unsaved document, and return: they write on
    /// the autosave workers, and the app is busy until they are on disk (the OS may end it
    /// without another word once it's off screen, and ends it if this thread hangs).
    fn suspend(&mut self, _ctx: &egui::Context) {
        let unsaved: Vec<(Arc<Document>, u64, Option<String>)> =
            self.app.session.documents().iter().filter(|st| st.is_dirty()).map(|st| (st.doc.clone(), st.revision, st.path.clone())).collect();
        let Some(autosave) = self.app.services.autosave.as_mut() else { return };
        let mut queued = 0;
        for (doc, revision, path) in &unsaved {
            match autosave(doc, *revision, path.as_deref()) {
                Ok(()) => {
                    update(&self.suspended, |s| s.queued(doc.id.0, *revision));
                    queued += 1;
                }
                Err(e) => log::warn!("photocraft-mobile: no recovery copy of {}: {e}", doc.name),
            }
        }
        log::info!("photocraft-mobile: suspended; writing {queued} of {} unsaved documents for recovery", unsaved.len());
    }

    /// A save or another job the user started is running, its bytes haven't reached the shell, or
    /// a recovery copy started on leaving the screen isn't on disk yet.
    fn busy(&self) -> bool {
        self.app.session.has_jobs()
            || !self.app.jobs.saves.is_empty()
            || !self.encoded.lock().unwrap_or_else(PoisonError::into_inner).is_empty()
            || update(&self.suspended, |s| s.writing())
    }

    /// Hot or saving power: live previews (adjustments, filters) run on a reduced copy.
    fn power(&mut self, ctx: &egui::Context, power: Power) {
        let current = self.app.session.prefs().performance.low_resolution_previews;
        let next = self.previews.apply(power, current);
        if let Some(reduced) = next {
            self.app.session.edit_prefs(|p| p.performance.low_resolution_previews = reduced);
        }
        log::info!(
            "photocraft-mobile: power {power:?}: low resolution previews {}",
            match next {
                Some(true) => "on while the device is constrained",
                Some(false) => "back to the user's setting (off)",
                None => {
                    if current { "on" } else { "off" }
                }
            }
        );
        ctx.request_repaint();
    }

    /// Rendered smart-object stages and sources rebuild on demand; undo history is the user's work.
    fn trim_memory(&mut self, ctx: &egui::Context) {
        let freed = photocraft_engine::cmd::smart_cmds::purge_stack_cache() + photocraft_engine::cmd::smart_cmds::purge_source_cache();
        log::info!("photocraft-mobile: memory warning; released {} MB of caches", freed / (1024 * 1024));
        ctx.request_repaint();
    }

    /// Pencil samples feed the brush engine's pressure and tilt, as a desktop tablet's do.
    fn stylus(&mut self, sample: &Stylus) {
        self.app.stylus.feed.set(pen_sample(sample));
    }
}

/// A stylus reading in PhotoCraft's terms, or `None` once it lifts.
fn pen_sample(sample: &Stylus) -> Option<PenSample> {
    if matches!(sample.phase, TouchPhase::End | TouchPhase::Cancel) {
        return None;
    }
    let (tilt_x, tilt_y) = sample.tilt_degrees();
    Some(PenSample { pressure: sample.pressure, tilt_x, tilt_y, rotation: sample.azimuth.to_degrees().rem_euclid(360.0), eraser: sample.eraser })
}

/// Layers top first, as the Layers panel lists them (documents store them bottom first), with
/// each group's children under it.
fn layer_rows(layers: &[Layer], depth: u32, selected: &dyn Fn(photocraft_doc::LayerId) -> bool, out: &mut Vec<Item>) {
    for layer in layers.iter().rev() {
        out.push(Item {
            id: layer.id.0,
            name: layer.name.clone(),
            kind: layer.content.kind_name().into(),
            depth,
            visible: Some(layer.visible),
            selected: selected(layer.id),
        });
        if let photocraft_doc::LayerContent::Group(group) = &layer.content {
            layer_rows(&group.children, depth + 1, selected, out);
        }
    }
}

impl eframe::App for PhotoCraftMobile {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let pasted = self.platform.pasted();
        self.answered = update(&self.pastes, |p| p.answered(pasted.is_some()));
        if self.answered.is_some() {
            // The paste asked for runs itself (`logic`), not as the ⌘V these events stand for:
            // the menu it was chosen from still holds the keyboard for a frame or two.
            raw_input.events.retain(|e| !matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)));
        }
        // The clipboard still holds PhotoCraft's own copy: what it put there, for the paste to
        // recognise as its own, never read back.
        *self.own_copy.borrow_mut() = match pasted.as_deref() {
            Some(Pasted::OwnImage) => {
                log::info!("photocraft-mobile: the clipboard still holds PhotoCraft's own copy: pasting that");
                self.app.session.clipboard.as_ref().and_then(clipboard_pixels)
            }
            _ => None,
        };
        self.app.raw_input_hook(ctx, raw_input);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // As files from disk: decoded on a worker, then File › Save writes back to them.
        for (path, bytes) in self.located.take() {
            if let Err(e) = self.app.open_file(&path, &bytes) {
                self.app.open_failed(&photocraft_ui_egui::file_open::display_name(&path), &e);
            }
        }
        if let Some((id, params)) = self.answered.take()
            && let Err(e) = photocraft_ui_egui::menus::invoke(&mut self.app, ctx, &id, params)
        {
            self.app.ui.status = e;
            self.app.ui.status_error = true;
        }
        self.app.logic(ctx, frame);
        self.forget_stale_recovery();
        // After the jobs moved on: a save whose job just ended has handed its bytes over.
        self.hand_over_saves();
        self.note_writes();
        show_file_names(&mut self.app.ui.status);
        quiet_clipboard(&mut self.app);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        self.note_writes();
        show_file_names(&mut self.app.ui.status);
        quiet_clipboard(&mut self.app);
    }
}

/// The status bar names a document by its file, never by the path the shell writes it through.
fn show_file_names(status: &mut String) {
    if let std::borrow::Cow::Owned(shown) = craft_mobile::with_file_names(status) {
        *status = shown;
    }
}

impl PhotoCraftMobile {
    /// Remove the recovery copies written on leaving the screen whose documents have no unsaved
    /// changes left: saved since, or closed. (PhotoCraft removes its own timed autosaves.)
    fn forget_stale_recovery(&mut self) {
        let session = &self.app.session;
        let stale = update(&self.suspended, |s| s.stale(|id| session.documents().iter().any(|st| st.doc.id.0 == id && st.is_dirty())));
        if let Some(discard) = self.app.services.discard_autosave.as_mut() {
            for id in stale {
                discard(id);
            }
        }
    }

    /// Send the saves workers encoded to the shell, as an inline save writes them: in place, or
    /// to the exporter.
    fn hand_over_saves(&mut self) {
        let saves = std::mem::take(&mut *self.encoded.lock().unwrap_or_else(PoisonError::into_inner));
        let (shares, saves): (Vec<_>, Vec<_>) = saves.into_iter().partition(|(path, _)| path.starts_with(SHARE_ROOT));
        for (path, bytes) in shares {
            let name = file_name(&path);
            // Saved as a copy: the status says where it's going instead.
            self.app.ui.status = format!("Sharing {name}");
            self.platform.share(name, bytes);
        }
        let Some(write) = self.app.services.write.as_mut() else { return };
        for (path, bytes) in saves {
            if let Err(e) = write(&path, &bytes) {
                log::warn!("photocraft-mobile: couldn't hand {path} over: {e}");
            }
        }
    }

    /// Tie each write in place issued since the last call to the document and revision it
    /// wrote. Saves write the document they're for and record it in the same frame, so the
    /// document now bound to the path is that one, at the revision written.
    fn note_writes(&mut self) {
        for path in self.issued.take() {
            // None: a copy (the document lives elsewhere), which never marks it saved.
            if let Some(st) = self.app.session.documents().iter().find(|st| st.path.as_deref() == Some(path.as_str())) {
                self.writing.push(PendingWrite { path, doc: st.doc.id, revision: st.revision });
            }
        }
    }
}

/// PhotoCraft's copy as it mirrors it on the clipboard (`export_os_clipboard`): its bounds as
/// RGBA8, so a paste knows it for its own.
fn clipboard_pixels(clip: &photocraft_engine::edit_cmds::Clip) -> Option<(u32, u32, Vec<u8>)> {
    let b = clip.bounds;
    if b.is_empty() {
        return None;
    }
    let mut px = vec![[0_u8; 4]; usize::try_from(u64::from(b.width()) * u64::from(b.height())).ok()?];
    clip.surface.read_rgba8_into(b, &mut px);
    Some((b.width(), b.height(), px.into_iter().flatten().collect()))
}

/// The file File › Share… sends: the document's own file when it holds the whole document (its
/// PSD or .pcraft, or the flat file it came from while that keeps it; a copy from Photos or a
/// share goes by its name), else a PSD named for it, as File › Save would suggest.
fn share_name(doc: &Document, path: Option<&str>) -> String {
    use photocraft_engine::file_cmds::{extension, holds_at};
    let source = file_name(path.unwrap_or(&doc.name));
    if holds_at(doc, &source) || photocraft_ui_egui::flat_file_keeps(doc, &source) {
        return source;
    }
    let ext = match extension(&source).as_deref() {
        Some("pdn") => "pcraft",
        Some("kra" | "krz") => "ora",
        _ => "psd",
    };
    std::path::Path::new(&source).with_extension(ext).to_string_lossy().into_owned()
}

/// File › Share…: Save a Copy of the active document, to the share sheet instead of a file.
fn share(app: &mut PhotocraftApp, ctx: &egui::Context, next: &Shared<Option<String>>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document is open")?;
    next.set(Some(share_name(&st.doc, st.path.as_deref())));
    let shared = photocraft_ui_egui::menus::invoke(app, ctx, "file.saveACopy", json!({}));
    if shared.is_err() {
        // It stopped before asking where: the next save mustn't go to the share sheet.
        next.set(None);
    }
    shared
}

/// The tablet's File › Import from Photos… and File › Share…, and the clipboard read on paste.
fn tablet_menus(services: &mut Services, platform: &Platform, pastes: Shared<Pastes>, own_copy: OwnCopy) {
    let reader = platform.clone();
    services.clipboard_get_image = Some(Box::new(move || match reader.pasted().as_deref() {
        Some(Pasted::Image { width, height, rgba }) => Ok(Some((*width, *height, rgba.clone()))),
        Some(Pasted::OwnImage) => Ok(own_copy.borrow().clone()),
        Some(Pasted::Text(_)) => Ok(None),
        // Every paste waits for the shell's read (`tablet_menus`): this is no paste (the New
        // Document dialog sizing itself, say). Nothing is known, so nothing changes.
        None => Err(NOT_PASTING.into()),
    }));
    let mut items = Vec::new();
    if platform.has_photo_picker() {
        items.push(HostMenuItem { id: IMPORT_PHOTOS.into(), label: "Import from Photos…".into(), after: "file.open".into() });
    }
    items.push(HostMenuItem { id: SHARE.into(), label: "Share…".into(), after: "file.saveACopy".into() });
    let shell = platform.clone();
    let share_next = Shared::<Option<String>>::default();
    let next = share_next.clone();
    services.host_menu = Some(HostMenu {
        items,
        enabled: Box::new(|app, id| id != SHARE || app.session.active().is_some()),
        run: Box::new(move |app, ctx, id, params| match id {
            IMPORT_PHOTOS => {
                shell.pick_photos();
                Some(Ok(Value::Null))
            }
            SHARE => Some(share(app, ctx, &next)),
            // Typing in a type layer: PhotoCraft asks for the clipboard's text itself.
            id if reads_clipboard(id) && app.ui.text_edit.is_none() && !update(&pastes, |p| p.run(id, params, shell.pasted().is_some())) => {
                shell.request_paste();
                Some(Ok(json!({"pending": true})))
            }
            _ => None,
        }),
    });
    // Save a Copy for File › Share… asks where; the answer sends it to the share sheet.
    if let Some(mut dialog) = services.file_dialog.take() {
        services.file_dialog = Some(Box::new(move |request, frame, reply| {
            if matches!(request, FileDialogRequest::Save { .. })
                && let Some(name) = share_next.take()
            {
                reply.send(Some(FileDialogAnswer::SaveTo(format!("{SHARE_ROOT}{name}"))));
                return;
            }
            dialog(request, frame, reply);
        }));
    }
}

/// Mark `write`'s document saved at the revision written, if it still lives in that file (a
/// Save As since moved it elsewhere, or it was closed).
fn mark_saved(session: &mut Session, write: &PendingWrite) {
    let Some(index) = session.documents().iter().position(|st| st.doc.id == write.doc && st.path.as_deref() == Some(write.path.as_str())) else {
        return;
    };
    // As PhotoCraft finishes a background save: the document is made active for the change,
    // then the active one is restored.
    let active = session.active_index();
    session.set_active(index);
    if let Some(st) = session.active_mut() {
        st.saved_revision = write.revision;
    }
    if let Some(active) = active {
        session.set_active(active);
    }
}

fn push<T>(queue: &Cell<Vec<T>>, item: T) {
    let mut items = queue.take();
    items.push(item);
    queue.set(items);
}

craft_mobile::export!(PhotoCraftMobile);

/// Build the app on `ctx`, drawing its canvas with wgpu when a render state is given.
fn create_app(ctx: &egui::Context, wgpu: Option<eframe::egui_wgpu::RenderState>, services: Services) -> PhotocraftApp {
    PhotocraftApp::install_type_reset_hooks(ctx);
    PhotocraftApp::setup_context(ctx, ThemeKind::Pro);
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.set_theme(ctx, ThemeKind::Pro);
    // Long commands run as background jobs with progress, as on the desktop.
    app.background_jobs = true;
    if let Some(rs) = wgpu {
        log::info!("photocraft-mobile: wgpu {:?} on {}", rs.adapter.get_info().backend, rs.adapter.get_info().name);
        app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
        app.set_wgpu(rs);
        app.start_compositor(ctx);
    }
    app
}

/// The in-process codecs (as on the web), and files through the shell's picker, its writes in
/// place and its exporter.
fn services(platform: &Platform, inbox: Inbox, located: Located, issued: Issued, encoded: Encoded, suspended: Shared<Suspended>) -> Services {
    let picker = platform.clone();
    let exporter = platform.clone();
    let clipboard = platform.clone();
    let browser = platform.clone();
    let mut services = Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        export: Some(Box::new(|doc: &photocraft_doc::Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        })),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        file_dialog: Some(Box::new(move |request, _frame, reply| match request {
            // The picker hands over contents, not paths, as a browser does.
            FileDialogRequest::Open { multiple, extensions, .. } => {
                let located = located.clone();
                picker.pick_files(
                    multiple,
                    extensions.unwrap_or_default(),
                    Box::new(move |files| {
                        let (answer, in_place) = pick_answer(multiple, files);
                        for file in in_place {
                            push(&located, file);
                        }
                        reply.send(answer);
                    }),
                );
            }
            // Engine commands that read a folder need paths; a picker's folder isn't one.
            FileDialogRequest::Folder { .. } => reply.send(None),
            // The exporter asks where; here only the name is settled. Where the desktop asks (Save
            // As, a format the file can't keep), so does the exporter: only a save PhotoCraft
            // writes without asking goes back in place (`save_flat_in_place` below).
            FileDialogRequest::Save { suggested } => reply.send(Some(FileDialogAnswer::SaveTo(file_name(&suggested)))),
        })),
        write: Some(Box::new(move |path: &str, bytes: &[u8]| {
            if path.starts_with(SHARE_ROOT) {
                exporter.share(file_name(path), bytes.to_vec());
                return Ok(());
            }
            if exporter.save(path, bytes.to_vec()) == SaveRoute::InPlace {
                push(&issued, path.to_string());
            }
            Ok(())
        })),
        // Saves encode on a worker (a background job with progress and Cancel); the bytes reach
        // `write` on a later frame (`PhotoCraftMobile::hand_over_saves`).
        save_file: Some(Arc::new(
            move |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings, ctl: &photocraft_ui_egui::jobs_ui::SaveCtl| {
                let (file, warnings) = encode_save(doc, path, settings, ctl)?;
                encoded.lock().unwrap_or_else(PoisonError::into_inner).push((path.to_string(), file));
                Ok(warnings)
            },
        )),
        // The user may cancel the exporter, and nothing reports back: never mark the document
        // saved here. A write in place is confirmed later (`PhotoCraftMobile::written`).
        write_unconfirmed: true,
        // File › Save writes a flat file (a PNG, a JPEG) back while it keeps the document: the
        // desktop's save dialog would suggest that very file, but the exporter can only make a
        // new one.
        save_flat_in_place: true,
        // App Store guideline 2.5.2: no installing code. Plug-ins are only those the app brings;
        // Android keeps Install Plug-in… and the plug-ins folder (sandboxed WebAssembly).
        bundled_plugins_only: cfg!(target_os = "ios"),
        inbox: Some(inbox),
        clipboard_set_image: Some(Box::new(move |w, h, rgba: &[u8]| {
            clipboard.copy_image(w, h, rgba.to_vec());
            Ok(())
        })),
        open_url: Some(Box::new(move |url: &str| {
            browser.open_url(url.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    recovery(&mut services, platform.data_dir().map(|dir| dir.join("Recovery")), suspended);
    services
}

/// Encode `doc` for `path` on a save job's worker, with its progress and Cancel: the file and
/// the export warnings. Nothing is handed over once the job is cancelled.
fn encode_save(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
) -> Result<(Vec<u8>, Vec<String>), String> {
    ctl.progress(0.0, "Encoding");
    let cancelled = || ctl.check().is_err();
    let progress = |f: f32| ctl.progress(f, "");
    let interrupt = photocraft_raster::Interrupt::new(&cancelled, &progress);
    let result = photocraft_io::export_with(doc, path, &settings.io_options(), &interrupt).map_err(|e| e.to_string())?;
    ctl.commit()?;
    ctl.progress(1.0, "");
    Ok((result.bytes, result.warnings))
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

/// Run `f` on the recovery store (`Err` without a data folder). The service closures never call
/// each other, so the store is never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no data folder for crash recovery")?))
}

/// Crash recovery in the app's data folder, as the desktop app keeps it: background .pcraft
/// autosaves, offered again after the OS ended the app. The writes PhotoCraft polls for each
/// frame also end the wait for the copies `suspended` started.
fn recovery(services: &mut Services, dir: Option<PathBuf>, suspended: Shared<Suspended>) {
    let recover_dir = dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
    let (s1, s2, s3, s4) = (store.clone(), store.clone(), store.clone(), store.clone());
    services.autosave = Some(Box::new(move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
        with_store(&s1, |s| s.autosave_checked(doc, revision, path.map(str::to_string)))?
    }));
    services.autosave_results = Some(Box::new(move || {
        let done = with_store(&s2, |s| s.take_completed()).unwrap_or_default();
        update(&suspended, |s| s.ended(&done));
        done
    }));
    services.flush_autosaves = Some(Box::new(move || with_store(&s3, RecoveryStore::flush).unwrap_or_default()));
    services.discard_autosave = Some(Box::new(move |id: u64| {
        let _ = with_store(&s4, |s| s.discard(id));
    }));
    services.recover = Some(Box::new(move || {
        let Some(dir) = recover_dir.as_ref() else { return Vec::new() };
        photocraft_format::list_recovery(dir)
            .into_iter()
            .map(|e| Recoverable {
                key: e.info.key.clone(),
                name: e.info.document_name.clone(),
                path: e.info.original_path.clone(),
                load: Box::new(move || photocraft_format::recover(&e).map_err(|e| e.to_string())),
            })
            .collect()
    }));
    services.adopt_autosave = Some(Box::new(move |id: u64, key: &str| {
        let _ = with_store(&store, |s| s.adopt(id, key));
    }));
}

/// The open picker's files as the dialog's answer, and the ones to open as files from disk (by
/// document path) instead. Only File › Open (several files) opens documents; a command reading
/// one file wants its contents, wherever it came from.
fn pick_answer(multiple: bool, files: Vec<PickedFile>) -> (Option<FileDialogAnswer>, Vec<(String, Vec<u8>)>) {
    if !multiple {
        return (files.into_iter().next().map(|f| FileDialogAnswer::Contents(f.name, f.bytes)), Vec::new());
    }
    if files.is_empty() {
        return (None, Vec::new());
    }
    let mut copies = Vec::new();
    let mut in_place = Vec::new();
    for file in files {
        match file.document_path() {
            Some(path) => in_place.push((path, file.bytes)),
            None => copies.push((file.name, file.bytes)),
        }
    }
    (Some(FileDialogAnswer::ContentsMany(copies)), in_place)
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> PickedFile {
        PickedFile { name: name.into(), bytes: vec![1], location: None }
    }

    fn located(name: &str) -> PickedFile {
        PickedFile { location: craft_mobile::Location::new(format!("file:///docs/{name}")), ..file(name) }
    }

    /// File › Open (several files) and a command reading one file get the shapes they expect;
    /// a cancelled picker answers nothing, never an empty list.
    #[test]
    fn picked_files_answer_the_dialog_that_asked() {
        assert!(matches!(pick_answer(true, vec![file("a.png"), file("b.psd")]), (Some(FileDialogAnswer::ContentsMany(f)), l) if f.len() == 2 && l.is_empty()));
        assert!(matches!(pick_answer(false, vec![file("a.png")]), (Some(FileDialogAnswer::Contents(n, _)), _) if n == "a.png"));
        assert_eq!(pick_answer(true, vec![]), (None, vec![]));
        assert_eq!(pick_answer(false, vec![]), (None, vec![]));
    }

    /// Files File › Open may write back open by their document path; copies by their contents.
    /// A command reading one file gets contents either way.
    #[test]
    fn picked_files_that_can_be_written_back_open_by_path() {
        let (answer, in_place) = pick_answer(true, vec![located("a.psd"), file("b.png")]);
        assert!(matches!(answer, Some(FileDialogAnswer::ContentsMany(f)) if f.len() == 1 && f.first().is_some_and(|(n, _)| n == "b.png")));
        assert_eq!(in_place.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), [located("a.psd").document_path().unwrap_or_default()]);
        let (answer, in_place) = pick_answer(true, vec![located("a.psd")]);
        assert_eq!(answer, Some(FileDialogAnswer::ContentsMany(vec![])), "answered, not cancelled");
        assert_eq!(in_place.len(), 1);
        assert!(matches!(pick_answer(false, vec![located("a.psd")]), (Some(FileDialogAnswer::Contents(n, _)), l) if n == "a.psd" && l.is_empty()));
    }

    /// A confirmed write marks the document saved at the revision written (later edits stay
    /// unsaved), only while it still lives in that file, and leaves the active tab alone.
    #[test]
    fn a_confirmed_write_saves_the_revision_written() {
        let mut session = Session::new();
        for name in ["a", "b"] {
            let doc = Document::new(name, photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
            session.add_document(doc, None);
        }
        let path = "inplace://1/a.psd".to_string();
        let doc = session.documents().first().map(|st| st.doc.id).unwrap_or(DocId(0));
        session.set_active(0);
        if let Some(st) = session.active_mut() {
            st.path = Some(path.clone());
            st.revision = 7;
            st.saved_revision = 3;
        }
        session.set_active(1);
        mark_saved(&mut session, &PendingWrite { path: path.clone(), doc, revision: 5 });
        assert_eq!(session.documents().first().map(|st| st.saved_revision), Some(5), "saved as written, still dirty after");
        assert_eq!(session.active_index(), Some(1));
        mark_saved(&mut session, &PendingWrite { path: "inplace://2/a.psd".into(), doc, revision: 7 });
        assert_eq!(session.documents().first().map(|st| st.saved_revision), Some(5), "moved elsewhere since: not this file's save");
    }

    /// Hot or saving power turns reduced previews on for as long as it lasts, then gives the
    /// user's setting back, unless it already is.
    #[test]
    fn previews_are_reduced_while_the_device_is_constrained() {
        let hot = Power { thermal: craft_mobile::Thermal::Serious, low_power: false };
        let fine = Power::default();
        let mut previews = PreviewOverride::default();
        assert_eq!(previews.apply(fine, false), None);
        assert_eq!(previews.apply(hot, false), Some(true), "reduced while hot");
        assert_eq!(previews.apply(hot, true), None, "already overridden");
        assert_eq!(previews.apply(fine, true), Some(false), "the user's full-resolution previews again");
        assert_eq!(previews.apply(hot, true), None, "already reduced: nothing to change");
        assert_eq!(previews.apply(fine, true), None, "and nothing to give back");
        assert_eq!(previews.apply(hot, false), Some(true));
        assert_eq!(previews.apply(fine, false), None, "turned off meanwhile (in Preferences): nothing to give back");
    }

    /// Leaving the screen keeps the app busy until each copy is written (or a later revision of
    /// it is), whatever the outcome; a copy is stale once its document has no unsaved changes.
    #[test]
    fn suspend_copies_keep_the_app_busy_until_written_and_go_once_saved() {
        let mut s = Suspended::default();
        assert!(!s.writing());
        s.queued(1, 5);
        s.queued(2, 3);
        s.ended(&[(1, 4, Ok(())), (3, 9, Ok(()))]);
        assert!(s.writing(), "an older revision's write doesn't end the wait");
        s.ended(&[(1, 6, Ok(()))]);
        assert_eq!(s.writing.keys().copied().collect::<Vec<_>>(), [2], "a later revision does");
        s.ended(&[(2, 3, Err("disk full".into()))]);
        assert!(!s.writing(), "a failed write ends the wait too");
        assert_eq!(s.stale(|_| true), Vec::<u64>::new(), "both still unsaved");
        assert_eq!(s.stale(|id| id == 2), [1], "document 1 was saved or closed");
        assert_eq!(s.stale(|_| false), [2]);
        assert_eq!(s, Suspended::default());
        s.queued(4, 1);
        assert_eq!(s.stale(|_| false), [4]);
        assert!(!s.writing(), "closed while writing: nothing left to wait for");
    }

    #[test]
    fn the_status_names_a_document_by_its_file() {
        let path = located("a.psd").document_path().unwrap_or_default();
        let mut status = format!("Saved {path}");
        show_file_names(&mut status);
        assert_eq!(status, "Saved a.psd");
    }

    #[test]
    fn a_save_path_becomes_the_suggested_file_name() {
        assert_eq!(file_name("/var/mobile/Untitled-1.psd"), "Untitled-1.psd");
        assert_eq!(file_name("Untitled-1.pcraft"), "Untitled-1.pcraft");
    }

    /// A Pencil sample reaches the brush engine with its pressure and tilt while it touches, and
    /// lifting it ends the stroke's pen data, as a desktop tablet's does.
    #[test]
    fn pencil_samples_feed_pressure_and_tilt_until_it_lifts() {
        let sample = Stylus { id: 1, phase: TouchPhase::Move, pos: egui::Pos2::ZERO, pressure: 0.4, altitude: 0.0, azimuth: 0.0, eraser: false };
        let pen = pen_sample(&sample).expect("a pen in contact");
        assert!((pen.pressure - 0.4).abs() < 1e-6);
        assert!((pen.tilt_x - 90.0).abs() < 1e-3, "lying flat, leaning right: {pen:?}");
        assert!(pen_sample(&Stylus { phase: TouchPhase::End, ..sample }).is_none());
        assert!(pen_sample(&Stylus { phase: TouchPhase::Cancel, ..sample }).is_none());
        assert!(pen_sample(&Stylus { eraser: true, ..sample }).is_some_and(|p| p.eraser));
    }

    /// A paste waits for the clipboard and runs, once, in the frame that brings it; a paste the
    /// clipboard is already here for (⌘V on a hardware keyboard) runs at once.
    #[test]
    fn a_paste_waits_for_the_clipboard_and_runs_when_it_comes() {
        let mut p = Pastes::default();
        assert!(!p.run("edit.pasteSpecial.pasteInPlace", &json!({}), false), "ask the shell");
        assert_eq!(p.answered(false), None, "still waiting");
        assert_eq!(p.answered(true), Some(("edit.pasteSpecial.pasteInPlace".into(), json!({}))));
        assert_eq!(p.answered(true), None, "answered once");
        assert!(p.run("edit.paste", &json!({}), true), "the clipboard is here");
        assert_eq!(p, Pastes::default());
    }

    /// A paste of PhotoCraft's own copy hands back exactly what Copy mirrored to the clipboard,
    /// soft edges included, so PhotoCraft knows it for its own and pastes the copy itself.
    #[test]
    fn the_own_copy_is_what_copy_put_on_the_clipboard() {
        let mirrored = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&mirrored);
        let services = Services {
            clipboard_set_image: Some(Box::new(move |w, h, rgba: &[u8]| {
                *sink.borrow_mut() = Some((w, h, rgba.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 9, "height": 7, "background": "transparent"})).expect("a document");
        app.run("select.rect", json!({"x": 1, "y": 1, "width": 5, "height": 4})).expect("a selection");
        app.run("edit.fill", json!({"color": "#ff8000", "opacity": 37})).expect("a soft fill");
        photocraft_ui_egui::menus::invoke(&mut app, &ctx, "edit.copy", json!({})).expect("copied");
        let own = app.session.clipboard.as_ref().and_then(clipboard_pixels);
        assert!(own.as_ref().is_some_and(|(_, _, px)| px.chunks(4).any(|p| p.get(3).is_some_and(|a| *a > 0 && *a < 255))), "soft pixels");
        assert_eq!(own, *mirrored.borrow());
    }

    /// Outside a paste the clipboard isn't read: the New Document dialog, which looks at it, keeps
    /// PhotoCraft's own copy and says nothing about it.
    #[test]
    fn a_read_outside_a_paste_keeps_the_copy_and_says_nothing() {
        let services = Services {
            clipboard_set_image: Some(Box::new(|_, _, _: &[u8]| Ok(()))),
            clipboard_get_image: Some(Box::new(|| Err(NOT_PASTING.into()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 4, "height": 4})).expect("a document");
        photocraft_ui_egui::menus::invoke(&mut app, &ctx, "edit.copy", json!({})).expect("copied");
        let _ = photocraft_ui_egui::menus::invoke(&mut app, &ctx, "file.new", json!({}));
        assert_eq!(app.ui.status, NOT_PASTING, "the dialog looked at the clipboard");
        quiet_clipboard(&mut app);
        assert!(app.session.clipboard.is_some(), "the copy stays");
        assert!(!app.ui.status_error && app.ui.status.is_empty(), "{}", app.ui.status);
    }

    /// File › Share… sends the document's own file when it keeps everything, else a PSD.
    #[test]
    fn a_share_is_the_documents_own_file_or_a_psd() {
        use photocraft_doc::{Color, ColorMode, SampleType, Size};
        let flat = Document::with_background("a.png", Size::new(4, 4), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let path = located("a.png").document_path().unwrap_or_default();
        assert_eq!(share_name(&flat, Some(&path)), "a.png", "a flat file that keeps it");
        assert_eq!(share_name(&flat, Some("inplace://1/b.psd")), "b.psd");
        assert_eq!(share_name(&flat, None), "a.png", "a copy (from Photos, say): by its name");
        let mut layered = flat.clone();
        layered.layers.push(Layer::group("G", vec![]));
        assert_eq!(share_name(&layered, Some(&path)), "a.psd", "a PNG can't keep its layers");
        assert_eq!(share_name(&layered, Some("/x/c.pdn")), "c.pcraft");
        assert_eq!(share_name(&Document::new("Untitled-1", Size::new(1, 1), ColorMode::Rgb, SampleType::U8), None), "Untitled-1.psd", "never saved");
    }

    /// The panel lists layers top first, children under their group, as the desktop panel does.
    #[test]
    fn layers_are_listed_top_first_with_groups_nested() {
        let with_id = |mut layer: Layer, id| {
            layer.id = photocraft_doc::LayerId(id);
            layer
        };
        let bottom = with_id(Layer::group("Bottom", vec![]), 1);
        let top = with_id(Layer::group("Top", vec![with_id(Layer::group("Child", vec![]), 3)]), 2);
        let mut rows = Vec::new();
        layer_rows(&[bottom, top], 0, &|id| id.0 == 3, &mut rows);
        let shown: Vec<(&str, u32, bool)> = rows.iter().map(|r| (r.name.as_str(), r.depth, r.selected)).collect();
        assert_eq!(shown, [("Top", 0, false), ("Child", 1, true), ("Bottom", 0, false)]);
    }
}
