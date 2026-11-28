//! Opening files from disk: File › Open, Open Recent, drag-and-drop onto the window (except onto
//! a document's canvas, which places the file as a layer), command-line paths at launch
//! and the OS's "open these documents" requests (macOS Finder double-click, Open With, drops on the
//! Dock icon) all go through [`PhotocraftApp::open_file`], so each one names the document after the
//! file, remembers its path (File › Save writes back to it) and adds it to Open Recent. Failures
//! are shown as modal alerts and in the status bar; import warnings as a notice.

use crate::{PhotocraftApp, notices};
use craft::ui::dialog::{Alert, DialogId, DialogResult, DialogStack, HostedAlert, StackDialog};
use craft::ui::runtime::drag::OsFile;
use craft::ui::runtime::windows::{WindowAction, WindowHost};
use photocraft_doc::DocId;
use photocraft_engine::file_cmds::{is_template, untitled_name};

/// A request from the operating system, delivered by the platform shell through
/// [`Services::os_events`](crate::Services::os_events).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OsEvent {
    /// Open these files (absolute paths), e.g. a Finder double-click.
    Open(Vec<String>),
    /// Quit (e.g. the Dock menu's Quit, log out): goes through the normal close path, which asks
    /// about unsaved changes.
    Quit,
}

/// Where files dropped on the window land, as in the reference app. The framework's drop targets
/// say it: the tab strip (`Slot`), the document canvas (`Canvas`) and the window (`Window`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DropTo {
    /// The active document's canvas: placed as layers (Place Embedded), each in Free Transform.
    /// `at` is the document point under the drop, which picks a Frame layer to fill; `None` when
    /// it is not known.
    Canvas {
        /// The document point under the drop.
        at: Option<[f64; 2]>,
    },
    /// The tab strip: opened with their tabs from this document position.
    Slot(usize),
    /// Anywhere else (panels, a dialog, no document): opened like File › Open.
    Window,
}

impl DropTo {
    /// What a dropped file called `name` does here. Preset files (brushes, gradients) go to their
    /// libraries wherever they land, so only a canvas drop of anything else is placed.
    pub(crate) fn route(self, name: &str) -> DropRoute {
        if matches!(self, DropTo::Canvas { .. }) && !crate::preset_files_ui::is_preset_file(name) { DropRoute::Place } else { DropRoute::Open }
    }
}

/// What happens to one dropped file, by [`DropTo::route`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropRoute {
    /// Placed on the active document's canvas (File › Place Embedded), in Free Transform.
    Place,
    /// Opened like File › Open (at the drop's tab position when it landed on the tab strip).
    Open,
}

/// Where a web-build drop at `at` lands, from what the last frame drew: the canvas (a drop is
/// placed only while a document is open) and whether `at` is free of windows and dialogs above
/// the workspace (`uncovered`). No pointer, a covered one, or no canvas means [`DropTo::Window`].
/// The web build's own drop path until FW-264, and native drops the tab strip does not take (`canvas::window_drop`).
pub(crate) fn target_at(at: Option<egui::Pos2>, uncovered: bool, canvas: Option<egui::Rect>, has_document: bool) -> DropTo {
    let Some(p) = at.filter(|_| uncovered) else {
        return DropTo::Window;
    };
    // The canvas shown last frame may belong to a document closed since.
    if has_document && canvas.is_some_and(|r| r.contains(p)) {
        return DropTo::Canvas { at: None };
    }
    DropTo::Window
}

/// Why a dropped `path` can't be opened or placed when it is a folder, for a clear message in
/// place of the operating system's (Windows says "Access is denied" for reading a directory).
/// `None` for anything else, including a path that can't be inspected: the open reports that.
pub(crate) fn folder_error(path: &std::path::Path) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
        return Some(tl!("it is a folder, not a file; drop the files inside it instead").to_string());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = path;
    None
}

/// Log, once per drag, that files are being dragged over the window (`hovering` of them). Together
/// with the line `open_dropped` logs for a drop, it shows how far a drag got: nothing logged
/// means the operating system never offered the files to PhotoCraft (a window running as
/// administrator is not offered files from a File Explorer that isn't, #10492).
pub(crate) fn log_drag_over(ctx: &egui::Context, hovering: usize) {
    let id = egui::Id::unique("log-drag-over");
    let was = ctx.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false));
    if (hovering > 0) != was {
        ctx.data_mut(|d| d.insert_temp(id, hovering > 0));
        if hovering > 0 {
            log::info!("{hovering} file(s) dragged over the window");
        }
    }
}

/// A file dropped on the canvas, waiting its turn to be placed in the document it was dropped on
/// (queued with its [`DocId`] in `drop_places`), or one of several files chosen at once in File ›
/// Place Embedded's dialog, waiting its turn in the document the command was given in.
pub(crate) enum DropPlace {
    /// Destination captured when the file is dropped, before a later pointer move.
    At { source: Box<DropPlace>, frame: Option<photocraft_doc::LayerId> },
    /// Dropped onto the window (desktop): read from its path, or from the bytes the OS gave.
    File(OsFile),
    /// Read by the shell itself (the browser reads dropped files asynchronously): its name and
    /// contents, delivered through [`Services::place_inbox`](crate::Services::place_inbox).
    Bytes(String, Vec<u8>),
    /// Chosen in File › Place Embedded's dialog (desktop), by its full path: read when its turn
    /// comes and placed as that dialog places a single file. The web's picks carry their contents
    /// and wait as [`Self::Bytes`].
    Path(String),
}

impl DropPlace {
    /// The file contents held while it waits for its document (a path drop holds none yet).
    fn queued_bytes(&self) -> usize {
        match self {
            Self::At { source, .. } => source.queued_bytes(),
            Self::File(f) => f.bytes.as_ref().map_or(0, |b| b.len()),
            Self::Path(_) => 0,
            Self::Bytes(_, bytes) => bytes.len(),
        }
    }

    /// Whether it holds its file's contents while it waits (a browser drop); a path drop is read
    /// only when its turn comes.
    fn holds_bytes(&self) -> bool {
        match self {
            Self::At { source, .. } => source.holds_bytes(),
            Self::File(f) => f.bytes.is_some(),
            Self::Path(_) => false,
            Self::Bytes(..) => true,
        }
    }
}

/// The most browser drops, each holding its whole file, that can wait for their document to come
/// back in view. Path drops hold only their path and aren't counted: dropping a folder's worth of
/// files on the canvas places them all.
const MAX_QUEUED_DROPS: usize = 32;
/// The most dropped file contents kept in memory while they wait: a browser drop carries its whole
/// file, and anything queued on a document in another tab stays until it is back in view.
const MAX_QUEUED_DROP_BYTES: usize = 256 * 1024 * 1024;

/// Whether the drop queue takes a browser drop of `incoming` bytes when it holds `queued` of them
/// and `bytes` in all.
fn queue_takes_drop(queued: usize, bytes: usize, incoming: usize) -> bool {
    queued < MAX_QUEUED_DROPS && bytes.saturating_add(incoming) <= MAX_QUEUED_DROP_BYTES
}

/// Whether the shell's place inbox, holding `files` files of `bytes` bytes together, takes one more
/// of `incoming` bytes: the limits of the drop queue it is drained into, so browser reads that
/// finish between two frames can't hold more than the queue itself would.
pub fn place_inbox_takes(files: usize, bytes: usize, incoming: usize) -> bool {
    queue_takes_drop(files, bytes, incoming)
}

/// `warnings` from an import, plus one for each font family the active document's type uses that
/// isn't installed (#4559): Photoshop warns about missing fonts as a file opens, and the CLI and
/// automation already report them; the type would otherwise silently render in a fallback face.
pub(crate) fn with_missing_fonts(app: &PhotocraftApp, warnings: Vec<String>) -> Vec<String> {
    match app.session.active() {
        Some(st) => with_missing_fonts_of(&st.doc, warnings),
        None => warnings,
    }
}

/// [`with_missing_fonts`] for `doc` itself: a file held back for its profile prompt
/// (`open_prompt`) isn't the active document yet.
pub(crate) fn with_missing_fonts_of(doc: &photocraft_doc::Document, mut warnings: Vec<String>) -> Vec<String> {
    for w in photocraft_engine::type_extra_cmds::missing_font_warnings(doc) {
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    warnings
}

/// A dropped file's name ("dropped" when it has none).
fn dropped_name(f: &OsFile) -> String {
    if f.name.is_empty() { "dropped".into() } else { f.name.clone() }
}

/// `path` as the `String` file paths are carried in, or what to tell the user when its name isn't
/// valid Unicode (#6601): non-UTF-8 bytes on Linux, an unpaired surrogate on Windows. A lossy copy
/// would name another file, so Open would fail with "No such file" and Save would write a second,
/// renamed file beside the original. `saving`: the path is to be written to.
pub fn path_text(path: &std::path::Path, saving: bool) -> Result<String, String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        let line = if saving {
            tl!("{path}: the path is not valid Unicode; save under another name.")
        } else {
            tl!("{path}: the path is not valid Unicode; rename the file and open it again.")
        };
        crate::i18n::fmt(line, &[("path", &path.to_string_lossy())])
    })
}

/// The file name of `path` (the document's display name), or `path` itself when it has none.
pub fn display_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| path.to_string())
}

// Named PSD/PSB/templates deliberately keep the PSD parser's errors for direct callers. Only
// its missing file-header signature means unrecognized content; damaged internal signatures
// (8BIM, etc.) and a recognized 8BPS file's other parse errors must retain their reasons.
fn unrecognized_open_reason(reason: &str) -> bool {
    if reason == "codec: unrecognized image format" {
        return true;
    }
    let Some(signature) = reason.strip_prefix("PSD: invalid signature [").and_then(|s| s.strip_suffix("], expected 8BPS")) else {
        return false;
    };
    let mut values = signature.split(", ");
    let mut found = [0u8; 4];
    for byte in &mut found {
        let Some(text) = values.next() else { return false };
        let Ok(value) = text.parse::<u8>() else { return false };
        if text != value.to_string() {
            return false;
        }
        *byte = value;
    }
    values.next().is_none() && &found != b"8BPS"
}

/// One failed file, rendered by the shared alert. Distinct instances keep a multi-file open's
/// errors separate; a plain HostedAlert has one fixed identity and would replace the earlier file.
struct OpenFailureAlert {
    id: DialogId,
    alert: HostedAlert,
}

impl StackDialog for OpenFailureAlert {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }
    fn instance(&self) -> Option<DialogId> {
        Some(self.id)
    }
    fn key(&self) -> Option<&str> {
        self.alert.key()
    }
    fn json_fields(&self) -> Option<serde_json::Value> {
        self.alert.json_fields()
    }
    fn confirm(&mut self, ctx: &egui::Context) -> Option<()> {
        self.alert.confirm(ctx)
    }
    fn show(&mut self, ui: &mut egui::Ui, stack: &DialogStack) -> Option<DialogResult<()>> {
        ui.push_id(self.id, |ui| self.alert.show(ui, stack)).inner
    }
}

/// Acknowledgements contain no command; drain every completed occurrence without reopening it.
pub(crate) fn take_open_alerts(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    while app.framework.dialogs.take_keyed::<OpenFailureAlert>().is_some() {}
}

impl PhotocraftApp {
    /// Open `bytes` read from the file at `path`: the document is named after the file, keeps
    /// `path` for File › Save unless it is a derivative preview, and `path` goes to the top of Open Recent. Returns the import
    /// warnings (also shown to the user).
    pub fn open_file(&mut self, path: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        self.open_file_placed(path, bytes, None, None)
    }

    /// [`Self::open_file`] for a file dropped on a tab slot or opened as one of a set.
    pub(crate) fn open_file_placed(
        &mut self,
        path: &str,
        bytes: &[u8],
        slot: Option<usize>,
        order: Option<crate::jobs_ui::OpenOrder>,
    ) -> Result<Vec<String>, String> {
        // Brushes/gradients go to the preset libraries (no document, no Open Recent entry).
        if let Some(r) = crate::preset_files_ui::open(self, path, bytes) {
            return r.map(|()| Vec::new());
        }
        if self.background_jobs {
            // The path and Open Recent are recorded when the background open finishes.
            crate::jobs_ui::start_open(self, &display_name(path), Some(path.to_string()), crate::jobs_ui::bytes(bytes), slot, order)?;
            return Ok(Vec::new());
        }
        self.open_bytes_placed(&display_name(path), bytes, Some(path), slot, order)
    }

    /// Record that the active document was just opened from `path`: File › Save writes back to it
    /// (unless it's a template or derivative preview) and it goes to the top of Open Recent.
    pub(crate) fn opened_from(&mut self, path: &str) {
        self.opened_from_listed(path, Some(path));
    }

    /// [`Self::opened_from`], with `recent` the path Open Recent lists for it (`None`: not listed).
    /// An automation open is requested by a root-relative path, which names nothing outside
    /// automation, so it lists where the file is on disk instead.
    pub(crate) fn opened_from_listed(&mut self, path: &str, recent: Option<&str>) {
        if !is_template(path)
            && let Some(st) = self.session.active_mut()
            && !st.source_read_only
        {
            st.path = Some(path.to_string());
            let doc = st.doc.id;
            self.remember_stamp(doc, path);
        }
        if let Some(recent) = recent {
            self.push_recent(recent);
        }
    }

    /// The name a file called `name` opens under: its own, or the first free "Untitled-N" for a
    /// template.
    pub(crate) fn open_name(&self, name: &str) -> String {
        if !is_template(name) {
            return name.to_string();
        }
        untitled_name(tl!("Untitled"), |n| self.session.documents().iter().any(|d| d.doc.name == n) || self.jobs.opens.iter().any(|o| o.name == n))
    }

    /// Read and open the file at `path` (see [`open_file`](Self::open_file)).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_path(&mut self, path: &str) -> Result<Vec<String>, String> {
        self.open_path_placed(path, None, None)
    }

    /// [`Self::open_path`] for a file dropped on a tab slot or opened as one of a set: the slot
    /// and order survive an inline memory/profile prompt and the document's final placement.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_path_placed(&mut self, path: &str, slot: Option<usize>, order: Option<crate::jobs_ui::OpenOrder>) -> Result<Vec<String>, String> {
        if self.show_if_open(path) {
            return Ok(Vec::new());
        }
        // Documents opening in the background are read on the worker too (a 2 GB PSB read
        // would block the window). Preset files (brushes, gradients) go the usual way.
        let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if self.background_jobs && !crate::preset_files_ui::PRESET_EXTS.contains(&ext.as_str()) {
            crate::jobs_ui::start_open(
                self,
                &display_name(path),
                Some(path.to_string()),
                photocraft_engine::jobs::OpenSource::Path(path.to_string()),
                slot,
                order,
            )?;
            return Ok(Vec::new());
        }
        let bytes = photocraft_format::read_file(std::path::Path::new(path)).map_err(|e| format!("{path}: {e}"))?;
        self.open_file_placed(path, &bytes, slot, order)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open_path(&mut self, path: &str) -> Result<Vec<String>, String> {
        Err(format!("cannot open paths on the web: {path}"))
    }

    /// The web has no filesystem paths: a drop read by the shell arrives as bytes.
    #[cfg(target_arch = "wasm32")]
    fn open_path_placed(&mut self, path: &str, _slot: Option<usize>, _order: Option<crate::jobs_ui::OpenOrder>) -> Result<Vec<String>, String> {
        Err(format!("cannot open paths on the web: {path}"))
    }

    /// Open several files, reporting each failure to the user instead of stopping. Relative paths
    /// are resolved against the working directory so the remembered path stays valid. The files
    /// open in name order ([`natural_cmp`]: `File_2` before `File_10`), and their tabs keep that
    /// order however long each takes to open in the background (#3248). Returns how many opened.
    pub fn open_paths(&mut self, paths: &[String]) -> usize {
        let mut paths: Vec<String> = paths.iter().map(|p| absolute(p)).collect();
        paths.sort_by(|a, b| natural_cmp(a, b));
        self.open_paths_in_order(&paths)
    }

    /// [`Self::open_paths`] without the name sort: the files open in the order given and the last
    /// one is the active document. File › Open uses it, so files picked together open as picked.
    pub fn open_paths_in_order(&mut self, paths: &[String]) -> usize {
        // The command line's files open before the first frame, which otherwise applies the
        // interface language: their status lines and notices would come out in English.
        crate::i18n::set_current(crate::i18n::Lang::from_pref(&self.session.prefs().interface.language));
        let paths: Vec<String> = paths.iter().map(|p| absolute(p)).collect();
        let batch = crate::jobs_ui::begin_batch(self);
        let mut opened = 0;
        for (seq, abs) in paths.iter().enumerate() {
            // Not a member of the set: its tab is already in place.
            if self.show_if_open(abs) {
                continue;
            }
            let order = crate::jobs_ui::OpenOrder { batch, seq };
            // The order travels with the open: a background tab, an inline document, or a memory
            // prompt that is answered later all place the file at its place in the set.
            match self.open_path_placed(abs, None, Some(order)) {
                Ok(_) => opened += 1,
                Err(e) => self.open_failed(abs, &e),
            }
        }
        crate::jobs_ui::end_batch(self);
        opened
    }

    /// Show the document already open from the file at `path`, or its tab while it is still
    /// opening in the background, as Photoshop does, rather than open a second copy bound to the
    /// same file (#6598). True when there was one.
    pub(crate) fn show_if_open(&mut self, path: &str) -> bool {
        let canon = canonical(path);
        let same = |p: &str| same_file(p, path, canon.as_deref());
        if let Some(i) = self.session.documents().iter().position(|d| d.path.as_deref().is_some_and(same)) {
            self.session.set_active(i);
            self.jobs.focus = None;
        } else if let Some(o) = self.jobs.opens.iter().find(|o| o.path.as_deref().is_some_and(|p| !is_template(p) && same(p))) {
            // A template still loading opens as another Untitled, as it does once loaded.
            self.jobs.focus = Some(o.job);
        } else if self.held_memory_prompts.iter().any(|m| m.path.as_deref().is_some_and(|p| !is_template(p) && same(p))) {
            // The file's high-memory prompt is already waiting: show it, don't queue a second
            // prompt and a second open for the same file. A template still opens again as
            // another Untitled, like one that is still loading.
            crate::open_prompt::advance_memory(self);
            self.ui.chrome.home = None;
            return true;
        } else {
            return false;
        }
        // The document count didn't change, so the Home screen wouldn't give way by itself.
        self.ui.chrome.home = None;
        true
    }

    /// Report one failed open in an OK alert, naming its source file once. Pass the source path
    /// when available so its exact prefix can be removed from filesystem read errors too.
    pub fn open_failed(&mut self, source: &str, err: &str) {
        let name = display_name(source);
        let source_prefix = format!("{source}: ");
        let name_prefix = format!("{name}: ");
        // The native background importer wraps its path-prefixed read error in the file name.
        let wrapped_source_prefix = format!("{name}: {source}: ");
        let reason = (source != name)
            .then(|| err.strip_prefix(&wrapped_source_prefix))
            .flatten()
            .or_else(|| err.strip_prefix(&source_prefix))
            .or_else(|| err.strip_prefix(&name_prefix))
            .unwrap_or(err);
        let msg = if unrecognized_open_reason(reason) {
            crate::i18n::fmt(tl!("Could not open \"{name}\" because PhotoCraft does not recognize this type of file."), &[("name", &name)])
        } else {
            let template = "Couldn't open {name}: {error}";
            crate::i18n::fmt(tl!(template), &[("name", &name), ("error", reason)])
        };
        log::warn!("{msg}");
        self.ui.status = msg.clone();
        self.ui.status_error = true;
        let id = crate::jobs_ui::open_failure_id(self);
        self.framework.dialogs.open(OpenFailureAlert { id, alert: HostedAlert::new(Alert::new("PhotoCraft", msg)) });
    }

    /// Show "Couldn't place <name>: <error>" as an error (status bar + notice).
    pub fn place_failed(&mut self, name: &str, err: &str) {
        self.failed(crate::i18n::fmt(tl!("Couldn't place {name}: {error}"), &[("name", name), ("error", err)]));
    }

    /// Log `msg` and show it as an error (status bar + notice).
    fn failed(&mut self, msg: String) {
        log::warn!("{msg}");
        notices::error(self, msg);
    }

    /// Files dropped onto the window, routed by `to` ([`DropTo`]). Preset files (brushes,
    /// gradients) always go to their libraries. A native drop carries an absolute path, read in
    /// bounded reads (#375); a drop without a path carries its bytes.
    pub(crate) fn open_dropped(&mut self, ctx: &egui::Context, files: Vec<OsFile>, to: DropTo) {
        // Files dragged in from another app's window bring PhotoCraft to the front and give it the keyboard, as
        // Photoshop does, instead of leaving it behind the source (#5687). A drop on a floating panel's window
        // brings the main window.
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        // The log tells a drop that never reached the app (the operating system, or an elevated
        // process, refused it) from one the app routed (#10492). File names stay out of it.
        log::info!("{} file(s) dropped on the window: {to:?}", files.len());
        let frame = match to {
            DropTo::Canvas { at } => at.and_then(|p| crate::frame_tool::frame_at(self, p)),
            DropTo::Slot(_) | DropTo::Window => None,
        };
        let mut slot = match to {
            DropTo::Slot(slot) => Some(slot),
            DropTo::Canvas { .. } | DropTo::Window => None,
        };
        // A box still open from an earlier drop (or any Free Transform) is applied first (#4833):
        // the queue below places the next file only once no box is open, so a separate drop
        // would otherwise wait unseen for ↩. Files dropped together still go one by one.
        if matches!(to, DropTo::Canvas { .. }) && self.ui.transform.is_some() && files.iter().any(|f| to.route(&dropped_name(f)) == DropRoute::Place) {
            crate::transform_tool::commit(self);
        }
        for f in files {
            let name = dropped_name(&f);
            let source = f.path.as_deref().filter(|path| path.is_absolute()).map(|path| path.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
            let route = to.route(&name);

            if let Some(why) = f.path.as_deref().and_then(folder_error) {
                match route {
                    DropRoute::Place => self.failed(crate::i18n::fmt(tl!("Couldn't place {name}: {error}"), &[("name", &name), ("error", &why)])),
                    DropRoute::Open => self.open_failed(&source, &why),
                }
                continue;
            }
            if route == DropRoute::Place {
                if let Some(doc) = self.session.active().map(|st| st.doc.id) {
                    self.queue_drop(doc, DropPlace::At { source: Box::new(DropPlace::File(f)), frame });
                }
                continue;
            }
            match self.open_dropped_file(&f, &name, slot) {
                Ok(true) => slot = slot.map(|s| s + 1),
                Ok(false) => {}
                Err(e) => self.open_failed(&source, &e),
            }
        }
    }

    /// The pointer position (where a web or window-wide drop lands), from the OS cursor service when available, falling back to egui's
    /// last-known pointer position (e.g. on Wayland).
    pub(crate) fn cursor_pos(&mut self, ctx: &egui::Context) -> Option<egui::Pos2> {
        self.services.cursor_pos.as_mut().and_then(|f| f(ctx)).or_else(|| ctx.input(|i| i.pointer.latest_pos()))
    }

    /// Which of the files called `names`, dropped onto the window just now, are placed on the
    /// active document's canvas rather than opened, for a shell that reads dropped files itself
    /// (the browser reads them asynchronously, so the web shell can't hand them to
    /// [`open_dropped`](Self::open_dropped)). It delivers those through
    /// [`Services::place_inbox`](crate::Services::place_inbox) and the rest through
    /// [`Services::inbox`](crate::Services::inbox). The pointer comes from
    /// [`cursor_pos`](Self::cursor_pos). As with a desktop canvas drop, a Free
    /// Transform still open is applied first (#4833).
    pub fn dropped_on_canvas(&mut self, ctx: &egui::Context, names: &[String]) -> Vec<bool> {
        let at = self.cursor_pos(ctx);
        let to = self.drop_to(ctx, at);
        let canvas = matches!(to, DropTo::Canvas { .. });
        let places: Vec<bool> = names.iter().map(|n| canvas && to.route(n) == DropRoute::Place).collect();
        if places.contains(&true) && self.ui.transform.is_some() {
            crate::transform_tool::commit(self);
        }
        places
    }

    /// Where a web-build or window-wide drop at `at` lands (`canvas::window_drop`). Over a
    /// window or dialog it's not the canvas. On the canvas it carries the document point under `at`.
    pub(crate) fn drop_to(&self, ctx: &egui::Context, at: Option<egui::Pos2>) -> DropTo {
        let uncovered = at.is_some_and(|p| {
            ctx.layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background)
                && (self.ui.document_windows.arrangement != Some(craft::behavior::dock::Arrangement::Tile)
                    || crate::canvas::arranged_cells(self, self.last_canvas_rect).is_none_or(|cells| cells.iter().any(|(_, rect)| rect.contains(p))))
        });
        match target_at(at, uncovered, self.drop_canvas_rect, self.session.active().is_some()) {
            DropTo::Canvas { .. } => DropTo::Canvas {
                at: at.and_then(|p| {
                    crate::canvas::document_display::active_view(self).map(|view| {
                        let d = view.to_doc(p);
                        [d.x, d.y]
                    })
                }),
            },
            to => to,
        }
    }

    /// Open one dropped file; with `slot`, its tab goes there (at once, or when a held prompt is
    /// answered). True when it took the slot.
    fn open_dropped_file(&mut self, f: &OsFile, name: &str, slot: Option<usize>) -> Result<bool, String> {
        let (docs, opens) = (self.session.documents().len(), self.jobs.opens.len());
        let held = self.held_opens.len() + self.held_memory_prompts.len();
        // The slot travels with the open through the inline document, a background tab and a
        // memory/profile prompt, so no separate move here can land on another document.
        if let Some(path) = f.path.as_deref().filter(|p| p.is_absolute()) {
            self.open_path_placed(&path_text(path, false)?, slot, None)?;
        } else if let Some(bytes) = &f.bytes {
            self.open_bytes_placed(name, bytes, None, slot, None)?;
        } else {
            return Err(tl!("couldn't read the file").to_string());
        }
        if slot.is_none() {
            return Ok(false);
        }
        let took = self.jobs.opens.len() > opens || self.session.documents().len() > docs || self.held_opens.len() + self.held_memory_prompts.len() > held;
        Ok(took)
    }

    /// Place the next file dropped on the canvas and start Free Transform on it, once no other
    /// transform is in progress (so several files are placed and transformed one after another).
    /// With background jobs on (or a session that steps its jobs, as on the web), a file is placed
    /// by a job: the next one waits until that job has landed and its Free Transform has been
    /// applied (see `jobs_ui::finish_place`).
    ///
    /// Each file goes into the document it was dropped on, never into whichever one is active now:
    /// files dropped on a document in another tab wait until it is back in view, and are discarded
    /// (with a message) when it is closed.
    pub(crate) fn place_next_dropped(&mut self, ctx: &egui::Context) {
        // Leave the closed-document notice up for this frame: starting the next queued placement
        // would clear the status bar. The next frame retries the queue, so ask for one at once:
        // another document's files would otherwise wait for the next input event.
        if self.discard_drops_of_closed_documents() {
            if !self.drop_places.is_empty() {
                ctx.request_repaint();
            }
            return;
        }
        // A box still preparing (or its canceled job not yet handled) holds the queue too: the
        // next place would lock the document before a canceled place could be taken back.
        while self.ui.transform.is_none() && self.transform_preparing.is_none() && self.jobs.place.is_none() && self.session.active_job().is_none() {
            let Some(active) = self.session.active().map(|st| st.doc.id) else { return };
            let Some((_, next)) = self.drop_places.iter().position(|(doc, _)| *doc == active).and_then(|i| self.drop_places.remove(i)) else {
                return;
            };
            let (next, frame) = match next {
                DropPlace::At { source, frame } => (*source, Some(frame)),
                next => (next, None),
            };
            let params = frame.map_or_else(
                || serde_json::json!({}),
                |frame| serde_json::json!({"frame": frame.map_or(serde_json::Value::Bool(false), |id| serde_json::json!(id.0))}),
            );
            let (name, placed) = match next {
                DropPlace::At { .. } => {
                    self.failed(tl!("Invalid nested file drop").into());
                    continue;
                }
                DropPlace::File(f) => {
                    let name = dropped_name(&f);
                    let placed = if let Some(path) = f.path.as_deref().filter(|p| p.is_absolute()) {
                        path_text(path, false).and_then(|path| {
                            let mut p = params.clone();
                            p["path"] = serde_json::json!(path);
                            self.run("file.placeEmbedded", p)
                        })
                    } else if let Some(bytes) = &f.bytes {
                        self.place_bytes_with(&name, bytes.to_vec(), None, &params)
                    } else {
                        Err(tl!("couldn't read the file").to_string())
                    };
                    (name, placed)
                }
                DropPlace::Bytes(name, bytes) => {
                    let placed = self.place_bytes_with(&name, bytes, None, &params);
                    (name, placed)
                }
                DropPlace::Path(path) => {
                    let placed = photocraft_format::read_file(std::path::Path::new(&path))
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| self.place_bytes_with(&path, bytes, None, &params));
                    (display_name(&path), placed)
                }
            };
            match placed {
                // Running in the background: Free Transform starts when the layer has landed.
                Ok(started) if crate::jobs_ui::track_place(self, &name, &started) => return,
                Ok(result) => {
                    crate::notices::io_result_warnings(self, &result, &name);
                    if result.get("frame").and_then(serde_json::Value::as_u64).is_some() {
                        continue;
                    }
                    if let Err(e) = crate::transform_tool::begin_placed(self, ctx) {
                        self.ui.status = e;
                    }
                }
                Err(e) => self.place_failed(&name, &e),
            }
        }
    }

    /// File › Place Embedded with several files chosen (#14107): as in the reference application,
    /// each is placed in `doc` in turn, in the order the dialog returned them, with its own Free
    /// Transform and its own history state. They wait in the queue of files dropped on the canvas,
    /// so the next one is placed once the previous one's transform is committed or cancelled (Esc
    /// takes back only that file), and with Skip Transform when Placing all of them are placed at
    /// once. A file that can't be read or placed is reported and the next one placed.
    ///
    /// Every chosen file is queued: the drop queue's admission limits ([`queue_takes_drop`]) are
    /// not applied to them. A desktop pick waits as its path, and a web pick's contents were
    /// already read by the browser when the dialog answered, so refusing it would free nothing
    /// and only lose a file the user chose. Each is released as soon as it is placed, and while
    /// they wait they count against the limit for later drops.
    ///
    /// None of them fills a selected Frame layer (`"frame": false`), as a single file would: each
    /// would replace the previous one's image there, without a box or a layer of its own.
    pub(crate) fn place_picked(&mut self, ctx: &egui::Context, doc: DocId, files: Vec<crate::file_dialog::PickedFile>) {
        for file in files {
            let place = match file {
                crate::file_dialog::PickedFile::Path(path) => DropPlace::Path(path),
                crate::file_dialog::PickedFile::Contents(name, bytes) => DropPlace::Bytes(name, bytes),
            };
            self.drop_places.push_back((doc, DropPlace::At { source: Box::new(place), frame: None }));
        }
        self.place_next_dropped(ctx);
    }

    /// Queue the web drops onto the canvas that the shell has read since the last frame
    /// ([`Services::place_inbox`](crate::Services::place_inbox)), for the document they were
    /// dropped on.
    pub(crate) fn queue_web_places(&mut self) {
        let arrived: Vec<(DocId, String, Vec<u8>)> =
            self.services.place_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (doc, name, bytes) in arrived {
            self.queue_drop(doc, DropPlace::Bytes(name, bytes));
        }
    }

    /// Queue a canvas drop for the document it was dropped on, keeping the file contents it holds
    /// bounded: files dropped on a document in another tab wait here, and a browser drop carries
    /// its whole file. A path drop holds no contents, so any number of them queue.
    fn queue_drop(&mut self, doc: DocId, place: DropPlace) {
        let held = self.drop_places.iter().filter(|(_, p)| p.holds_bytes()).count();
        let bytes = self.drop_places.iter().fold(0usize, |sum, (_, p)| sum.saturating_add(p.queued_bytes()));
        if place.holds_bytes() && !queue_takes_drop(held, bytes, place.queued_bytes()) {
            self.ui.status = tl!("Dropped files weren't placed: too many are waiting for their document.").into();
            self.ui.status_error = true;
            return;
        }
        self.drop_places.push_back((doc, place));
    }

    /// Drop the queued files whose document has been closed, saying so. True when one was dropped
    /// (the caller lets the notice stand).
    fn discard_drops_of_closed_documents(&mut self) -> bool {
        if self.drop_places.is_empty() {
            return false;
        }
        let before = self.drop_places.len();
        let open: Vec<DocId> = self.session.documents().iter().map(|st| st.doc.id).collect();
        self.drop_places.retain(|(doc, _)| open.contains(doc));
        let discarded = self.drop_places.len() < before;
        if discarded {
            self.ui.status = tl!("Dropped files weren't placed: their document was closed.").into();
            self.ui.status_error = false;
        }
        discarded
    }

    /// Place a file's bytes as a smart object layer (File › Place; `linked`: Place Linked, to that
    /// path), for files that weren't read from a path the command can read. With background jobs
    /// on it starts a job (`{"job", "pending": true}`; see [`crate::jobs_ui::track_place`]).
    ///
    /// As in the reference application, Place Embedded centres the file on a selection, removing
    /// it (#14102); otherwise a place centres it on the part of the canvas in view (#14103).
    pub(crate) fn place_bytes(&mut self, name: &str, bytes: Vec<u8>, linked: Option<String>) -> Result<serde_json::Value, String> {
        let mut params = serde_json::json!({"centerOnSelection": linked.is_none()});
        if let Some(centre) = visible_canvas_centre(self) {
            params["center"] = serde_json::json!(centre);
        }
        self.place_bytes_with(name, bytes, linked, &params)
    }

    /// [`PhotocraftApp::place_bytes`] with the command's `params` (a frame to fill): a job where
    /// places run in the background, else synchronously.
    fn place_bytes_with(&mut self, name: &str, bytes: Vec<u8>, linked: Option<String>, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let r = if crate::jobs_ui::place_in_background(self) {
            crate::jobs_ui::start_place_bytes(self, name, bytes, linked, params)
        } else {
            photocraft_engine::file_cmds::place_bytes(&mut self.session, name, bytes, linked, params).map_err(|e| e.to_string())
        };
        self.sync_views();
        r
    }

    /// Handle the OS requests queued since the last frame.
    pub(crate) fn drain_os_events(&mut self, ctx: &egui::Context) {
        let events = self.services.os_events.as_mut().map(|f| f()).unwrap_or_default();
        for e in events {
            match e {
                OsEvent::Open(paths) => {
                    self.open_paths(&paths);
                    // Bring the window forward when another app (Finder, the Dock, Explorer via a
                    // second launch) asked us to open. The host restores it from minimized and, on
                    // native Wayland, asks for attention instead of focus.
                    WindowHost::of(ctx).act(egui::ViewportId::ROOT, WindowAction::Focus);
                    ctx.request_repaint();
                }
                OsEvent::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }
}

/// Where a place lands when only part of the canvas is in view (#14103): the centre of the part
/// of the canvas the active view shows, in document pixels. `None` (the canvas centre) when the
/// whole canvas is in view, and in the cases not measured in the reference application: a
/// rotated view, or one that shows the canvas whole on one axis. `None` too when the place fills
/// the active layer's frame, whose `center` is in the frame's own space.
fn visible_canvas_centre(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let view = crate::canvas::document_display::active_view(app)?;
    if view.rotation.abs() > 1e-6 {
        return None;
    }
    let st = app.session.active()?;
    if st.active_layer.is_some_and(|id| photocraft_engine::frame_cmds::frame_of(&st.doc, id).is_some()) {
        return None;
    }
    let size = st.doc.size;
    let shown = view.visible();
    // The shown span of the canvas on one axis, if the view cuts it there.
    let cut = |(lo, hi): (f64, f64), len: u32| {
        let len = f64::from(len);
        let (lo, hi) = (lo.max(0.0), hi.min(len));
        (lo < hi && (lo > 0.0 || hi < len)).then_some((lo + hi) / 2.0)
    };
    Some([cut((shown.x0, shown.x1), size.width)?, cut((shown.y0, shown.y1), size.height)?])
}

/// Order names like a file browser does: runs of digits by their value (`File_2` before
/// `File_10`), everything else case-insensitively; names equal that way fall back to the exact
/// text so the order is total.
pub(crate) fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let (m, n) = (digit_run(&mut x), digit_run(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                match m.len().cmp(&n.len()).then_with(|| m.cmp(n)) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            (Some(c), Some(d)) => {
                x.next();
                y.next();
                match c.to_lowercase().cmp(d.to_lowercase()) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
        }
    }
}

/// Take the run of ASCII digits at the front of `it`.
fn digit_run(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut run = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        run.push(c);
        it.next();
    }
    run
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn absolute(p: &str) -> String {
    std::path::absolute(p).map(|a| a.to_string_lossy().to_string()).unwrap_or_else(|_| p.to_string())
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn absolute(p: &str) -> String {
    p.to_string()
}

/// `path` with symlinks, `.` and `..` and (on case-insensitive volumes) letter case resolved;
/// `None` when it can't be (a missing file).
#[cfg(not(target_arch = "wasm32"))]
fn canonical(path: &str) -> Option<std::path::PathBuf> {
    std::fs::canonicalize(path).ok()
}

#[cfg(target_arch = "wasm32")]
fn canonical(_path: &str) -> Option<std::path::PathBuf> {
    None
}

/// Whether an open document's path `p` names the file at `path` (`canon`: its [`canonical`]
/// form). The cheap comparisons come first: `p` is resolved on disk only when they fail, so an
/// open of a file that isn't open costs one resolution per document. A file moved or deleted
/// since it was opened only matches its old path as written.
fn same_file(p: &str, path: &str, canon: Option<&std::path::Path>) -> bool {
    p == path || canon.is_some_and(|c| std::path::Path::new(p) == c || canonical(p).is_some_and(|q| q == c))
}

#[cfg(test)]
mod dialog_order_tests;
#[cfg(test)]
#[path = "place_multiple_files_tests.rs"]
mod place_multiple_files_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod unreadable_alert_tests;
