//! PhotoCraft's half of Open and Save against cloud storage.
//!
//! The contract types here ([`CloudBackend`], [`Job`], [`Post`], [`Event`], [`Fault`],
//! [`Account`]) are kept in the same shape as `crates/ui/src/cloud/backend.rs` on
//! wdunn001/craft-libs, which is where storytold/craft-libs#160 is adding the same contract to
//! `craft-ui` itself. They are not pulled from that crate directly. craft-ui's own `cloud` feature
//! exists today only on that fork, pinned to an egui release older than the one this workspace
//! builds against. Once #160 merges, craft-ui carries this contract at the egui release this
//! workspace uses, and this module is replaced by `pub use craft_ui::cloud::{...}` in one commit
//! alongside the `craft-ui` and `craft-cloud` pin bump in the workspace `Cargo.toml`.
//!
//! PhotoCraft holds no provider credential and speaks no provider protocol. A [`CloudBackend`] is
//! the one thing a host application supplies, and this crate never learns what it reaches.
//! Replacing one storage provider with another changes nothing here. Nothing in this module opens
//! a socket either, so the pages stay buildable for the web and testable with no service running.
//!
//! Every service call blocks, and a blocking read of a forty megabyte PSD inside a frame would
//! freeze the window for seconds. So the pages state a [`Job`], the host runs it on a worker
//! thread, and the answer arrives on a channel that [`pump`] drains on a later frame.
//!
//! With no backend connected, [`available`] is `false`. File › Open from the Cloud and File ›
//! Save to the Cloud are not in the menu at all, the account badge is not drawn, and every cloud
//! command is refused (see [`menu`] and `menus.rs`). A build with no cloud service configured is
//! the application as it was.
//!
//! What a document is, PhotoCraft answers alone. The extensions and content types below are that
//! answer, and the app shell's own File › Open list (`apps/photocraft/src/services.rs`, held to
//! this one by a test there) is where they come from. Bytes become a document through the same
//! `open_bytes` File › Open uses, and go back out through the same exporter File › Save As uses
//! (`Services::export`), so a cloud copy is exactly the file a save to disk would have written.
//!
//! Real-time shared editing, the presence row and the share-link dialog that the original
//! CollabCraft integration offered are not part of this contract. Each depended on
//! `wdunn001/collab-craft` and on `craft_ui::share` / `craft_ui::presence`, and none of those has
//! a storytold-hosted replacement or an open craft-libs contract today. See the pull request
//! description for where that stands.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

// Re-exported so a host application, and a test, can name these types without depending on the
// crate they come from. `craft-cloud` is types and contracts only: it opens no socket and names no
// provider, so it builds for the web too.
pub use craft_cloud::{Capability, Entry, EntryKind};
use serde_json::{Value as Json, json};

use crate::PhotocraftApp;

/// The extensions a cloud file list offers to open: the documents and images File › Open turns
/// into a document, lower case and without a dot.
///
/// Left out on purpose, each because opening it does not end in a document this call can name:
/// camera raws (the Camera Raw dialog asks first), HEIC (a build option), the preset files
/// (`.abr`, `.grd`, `.aco`, `.ase`, `.kys`), which go to their libraries, and `.psdt` templates,
/// which open as an untitled document with no file behind it. `apps/photocraft` holds this list to
/// a subset of File › Open's.
pub const OPEN_EXTS: &[&str] = &[
    "pcraft", "psd", "psb", "ora", "kra", "krz", "pdn", "af", "afdesign", "afphoto", "afpub", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "tga",
    "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pam", "pfm", "pcx", "dds", "xpm", "fits", "fit", "fts", "dpx", "svg", "svgz",
];

/// Content types PhotoCraft opens, matched against the start of what the provider said.
///
/// Read beside the extension because neither is reliable alone. A WebDAV server hands back
/// `application/octet-stream` for a perfectly good `.psd`, and a file exported by a web application
/// often has a correct content type and no extension at all. Every entry has a row in
/// [`extension_for_mime`], and a test holds the two together.
const MIMES: [&str; 15] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/bmp",
    "image/svg+xml",
    "image/vnd.adobe.photoshop",
    "image/x-psd",
    "image/x-tga",
    "image/x-icon",
    "image/vnd.microsoft.icon",
    "image/openraster",
    "image/x-exr",
    "image/vnd.radiance",
];

/// The file extension a content type stands for, for a destination whose name has none.
fn extension_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/tiff" => "tif",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        "image/vnd.adobe.photoshop" | "image/x-psd" => "psd",
        "image/x-tga" => "tga",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "image/openraster" => "ora",
        "image/x-exr" => "exr",
        "image/vnd.radiance" => "hdr",
        _ => return None,
    })
}

/// What a PhotoCraft document is, as a cloud file list needs to know. Data only, in the shape
/// `craft_ui::cloud::DocumentKind` has on the fork.
///
/// A name typed with no known extension becomes `.pcraft`, the native format: it is the one that
/// keeps every layer, mask, effect and smart object, and a save the user did not choose a format
/// for should lose nothing. A PSD, an OpenRaster file or a flat image is a name away.
pub struct DocumentKind {
    pub extensions: &'static [&'static str],
    pub mimes: &'static [&'static str],
    pub default_extension: &'static str,
}

pub const DOCUMENT: DocumentKind = DocumentKind { extensions: OPEN_EXTS, mimes: &MIMES, default_extension: photocraft_format::EXTENSION };

/// Where a save is going, as the provider described it: the file name, and the content type when
/// the provider issued one.
pub struct Destination<'a> {
    pub name: &'a str,
    pub mime: Option<&'a str>,
}

impl<'a> Destination<'a> {
    pub fn new(name: &'a str) -> Destination<'a> {
        Destination { name, mime: None }
    }

    /// The name's extension, lower case and without the dot. Empty when it carries none.
    pub fn extension(&self) -> String {
        match self.name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => extension.to_ascii_lowercase(),
            _ => String::new(),
        }
    }
}

/// The revision of each document a cloud save in flight encoded, by document id.
///
/// Only PhotoCraft knows what a version of a document is, so the pages hold none of these. A save
/// that lands marks exactly the revision that reached the server as saved, so an edit made while
/// the bytes were on the wire stays unsaved.
pub type Sent = HashMap<u64, u64>;

/// Whether PhotoCraft can open this cloud entry. A folder is always offered: it is browsed into.
pub fn opens(entry: &Entry) -> bool {
    if entry.kind == EntryKind::Folder {
        return true;
    }
    let extension = entry.name.rsplit('.').next().map(str::to_ascii_lowercase);
    if extension.as_deref().is_some_and(|e| DOCUMENT.extensions.contains(&e)) {
        return true;
    }
    entry.mime.as_deref().is_some_and(|mime| DOCUMENT.mimes.iter().any(|m| mime.starts_with(m)))
}

/// A typed name as a document file name. Folder separators are dropped, because a pasted path
/// cannot become a folder, and a name with no extension PhotoCraft writes becomes `.pcraft`.
pub fn document_name(typed: &str) -> String {
    let leaf = typed.rsplit(['/', '\\']).next().unwrap_or(typed).trim();
    if leaf.is_empty() {
        return String::new();
    }
    let extension = leaf.rsplit_once('.').map(|(stem, e)| (stem, e.to_ascii_lowercase()));
    match extension {
        Some((stem, e)) if !stem.is_empty() && writes(&e) => leaf.to_owned(),
        _ => format!("{leaf}.{}", DOCUMENT.default_extension),
    }
}

/// Whether PhotoCraft writes a file with this lower-case extension: the layered formats and every
/// flat format the codecs can encode. The same set File › Save As offers.
fn writes(extension: &str) -> bool {
    photocraft_engine::file_cmds::layered_extension(extension) || photocraft_codecs::from_extension(extension).is_some_and(|f| f.caps().write)
}

/// The file name a save should be exported under, for a destination the provider named.
///
/// The name is the first answer: its extension is what the file claims to be, and it is what the
/// user typed when they typed one. A provider that issued no extension at all still says what the
/// thing is, so the content type is the second. Both cases are common, and a file exported by a web
/// application usually has a correct content type and a bare name. The exporter picks the format
/// from the extension of what this returns.
///
/// A destination this application cannot write is refused by name, with the extensions it can write.
/// Encoding a document as `.pcraft` under the name `poster.kra` would hand somebody a file nothing
/// opens, and nothing would tell them. A refusal a person can act on is better.
fn export_name(into: Destination<'_>) -> Result<String, String> {
    let extension = into.extension();
    if !extension.is_empty() {
        return if writes(&extension) {
            Ok(into.name.to_owned())
        } else if OPEN_EXTS.contains(&extension.as_str()) {
            Err(cannot_write(&format!("PhotoCraft reads .{extension} files but cannot write one")))
        } else {
            Err(cannot_write(&format!("PhotoCraft does not write a .{extension}")))
        };
    }
    let mime = into.mime.ok_or_else(|| cannot_write("that name carries no extension"))?;
    let mime = mime.split(';').next().unwrap_or(mime).trim().to_ascii_lowercase();
    match extension_for_mime(&mime).filter(|e| writes(e)) {
        Some(extension) => Ok(format!("{}.{extension}", into.name)),
        None => Err(cannot_write(&format!("PhotoCraft does not write {mime}"))),
    }
}

/// A refusal that says what went wrong and which extensions are available instead.
fn cannot_write(why: &str) -> String {
    let writable: Vec<String> = OPEN_EXTS.iter().filter(|e| writes(e)).map(|e| format!(".{e}")).collect();
    format!("{why}. PhotoCraft writes {}", writable.join(", "))
}

// ---------------------------------------------------------------- the contract

/// Work for a host's worker. Each one answers with one [`Event`].
pub enum Job {
    /// Who is signed in.
    Account,
    /// One folder's contents. An empty path is the user's root.
    List(String),
    /// A document's bytes.
    Read { entry: Entry },
    /// Save under a new name in `folder`.
    Create { uid: u64, folder: String, name: String, bytes: Vec<u8> },
    /// Save back to an entry this document was already opened from.
    Write { uid: u64, entry: Entry, bytes: Vec<u8> },
}

/// What a worker reports back.
pub enum Event {
    Account(Result<Account, Fault>),
    Listed { path: String, entries: Result<Vec<Entry>, Fault> },
    Read { entry: Entry, bytes: Result<Vec<u8>, Fault> },
    Saved { uid: u64, entry: Result<Entry, Fault> },
}

/// Where a worker's answers go. Handed to [`CloudBackend::submit`] with the job it belongs to.
pub struct Post {
    tx: Sender<Event>,
}

impl Post {
    fn new(tx: Sender<Event>) -> Post {
        Post { tx }
    }

    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

/// Somewhere a [`Job`] can run without blocking a frame.
///
/// `submit` must return immediately. The answer arrives through `post` whenever it is ready, or
/// never, and the pages survive either.
pub trait CloudBackend: Send + Sync {
    fn submit(&self, job: Job, post: Post);
}

/// What the service said about the signed-in user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub display: String,
    /// What the provider behind the service can do. Carried through from `craft_cloud::Capability`
    /// so a host's backend can report it. Nothing in this crate reads it yet.
    pub capabilities: Vec<Capability>,
}

/// A call that did not succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    Unreachable(String),
    Refused(String),
}

impl Fault {
    /// One sentence for a person to read.
    pub fn text(&self) -> String {
        match self {
            Fault::Unreachable(detail) => format!("The cloud service could not be reached. {detail}"),
            Fault::Refused(detail) => detail.clone(),
        }
    }
}

/// One folder's contents, as last fetched.
#[derive(Clone, Default)]
pub struct Folder {
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<Fault>,
}

/// Which cloud page the modal is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Open,
    Save,
}

/// Open and Save against cloud storage. Inert until the host connects a [`CloudBackend`].
pub struct CloudFiles {
    backend: Option<Arc<dyn CloudBackend>>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    pub account: Option<Result<Account, Fault>>,
    pub mine: Folder,
    /// The name typed into the Save page.
    pub save_name: String,
    /// The page on screen, or `None` while neither is open.
    pub page: Option<Page>,
    /// A transfer in flight, for the page's status line.
    pub busy: Option<String>,
    /// Where each open document came from, by document id. Present only for a document opened
    /// from, or already saved to, the cloud. File › Save on one of these writes back here instead
    /// of asking for a path on this computer.
    origins: HashMap<u64, Entry>,
}

impl Default for CloudFiles {
    fn default() -> CloudFiles {
        let (tx, rx) = std::sync::mpsc::channel();
        CloudFiles { backend: None, tx, rx, account: None, mine: Folder::default(), save_name: String::new(), page: None, busy: None, origins: HashMap::new() }
    }
}

impl CloudFiles {
    pub fn new() -> CloudFiles {
        CloudFiles::default()
    }

    /// Connect the half that reaches the service. The host application calls this once, and only
    /// when a service is configured.
    pub fn connect(&mut self, backend: impl CloudBackend + 'static) {
        self.backend = Some(Arc::new(backend));
    }
}

pub const UNAVAILABLE: &str = "no cloud service is configured";

/// Whether a backend is connected at all. Every cloud surface in this crate is drawn only when
/// this is `true`.
pub fn available(app: &PhotocraftApp) -> bool {
    app.cloud.backend.is_some()
}

fn submit(app: &PhotocraftApp, job: Job) {
    if let Some(backend) = &app.cloud.backend {
        backend.submit(job, Post::new(app.cloud.tx.clone()));
    }
}

/// Ensures the account and the current folder are loaded. Call when a cloud page is about to be
/// drawn.
pub fn start(app: &mut PhotocraftApp) {
    if !available(app) {
        return;
    }
    if app.cloud.account.is_none() {
        submit(app, Job::Account);
    }
    if !app.cloud.mine.loaded && !app.cloud.mine.loading {
        let path = app.cloud.mine.path.clone();
        browse(app, &path);
    }
}

/// List a folder.
pub fn browse(app: &mut PhotocraftApp, path: &str) {
    if !available(app) {
        return;
    }
    app.cloud.mine.path = path.to_owned();
    app.cloud.mine.loading = true;
    app.cloud.mine.loaded = false;
    app.cloud.mine.error = None;
    submit(app, Job::List(path.to_owned()));
}

/// Open a cloud entry, or browse into it when it is a folder.
pub fn open_entry(app: &mut PhotocraftApp, entry: Entry) {
    if entry.kind == EntryKind::Folder {
        let path = entry.path.clone();
        browse(app, &path);
        return;
    }
    app.cloud.busy = Some(format!("Opening {} from the cloud", entry.name));
    submit(app, Job::Read { entry });
}

/// Whether the active document already has a cloud destination (opened from, or already saved to,
/// the cloud), so File › Save can write back there instead of asking for a path on this computer.
pub fn has_target(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| app.cloud.origins.contains_key(&st.doc.id.0))
}

/// The whole of the cloud state, for the control channel and for tests.
pub fn status(app: &PhotocraftApp) -> Json {
    json!({
        "available": available(app),
        "account": app.cloud.account.as_ref().map(|a| match a {
            Ok(account) => json!({"display": account.display}),
            Err(fault) => json!({"error": fault.text()}),
        }),
        "folder": app.cloud.mine.path,
        "page": app.cloud.page.map(|p| match p { Page::Open => "open", Page::Save => "save" }),
        "busy": app.cloud.busy,
        "hasTarget": has_target(app),
    })
}

/// Type being edited in place is not in the document yet. File › Save commits it first, so a cloud
/// save does too.
fn ready_to_save(app: &mut PhotocraftApp) {
    if app.ui.text_edit.is_some() {
        crate::type_tool::commit(app);
    }
}

/// Encode the active document for `into`, and record the revision that is going out.
///
/// A format that cannot hold the document (a flat file for a layered one) is a copy, as in File ›
/// Save As: the cloud file is written and the document stays unsaved, so the layers are not
/// forgotten.
fn encode(app: &mut PhotocraftApp, into: Destination<'_>) -> Result<(String, u64, Vec<u8>), String> {
    let name = export_name(into)?;
    let (uid, revision, bytes, warnings, copy) = {
        let st = app.session.active().ok_or("no document open")?;
        let settings = app.save_settings();
        let export = app.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&st.doc, &name, &settings)?;
        (st.doc.id.0, st.revision, bytes, warnings, app.saves_copy(&name, None)?)
    };
    if !copy {
        app.cloud_sent.insert(uid, revision);
    }
    crate::notices::io_warnings(app, &crate::i18n::fmt(crate::i18n::t("Saved {name}"), &[("name", &name)]), &warnings);
    Ok((name, uid, bytes))
}

/// Writes the active document back to the cloud entry it came from.
pub fn save(app: &mut PhotocraftApp) -> Result<Json, String> {
    if !available(app) {
        return Err(UNAVAILABLE.to_owned());
    }
    ready_to_save(app);
    let uid = app.session.active().map(|st| st.doc.id.0).ok_or("no document open")?;
    let entry = app.cloud.origins.get(&uid).cloned().ok_or("this document did not come from the cloud")?;
    let (name, uid, bytes) = encode(app, Destination { name: &entry.name, mime: entry.mime.as_deref() })?;
    app.cloud.busy = Some(format!("Saving {name} to the cloud"));
    submit(app, Job::Write { uid, entry, bytes });
    Ok(json!({"saving": name}))
}

/// Saves the active document under `name` in `folder`, as a new cloud file.
pub fn save_as(app: &mut PhotocraftApp, folder: &str, name: &str) -> Result<Json, String> {
    if !available(app) {
        return Err(UNAVAILABLE.to_owned());
    }
    let name = document_name(name);
    if name.is_empty() {
        return Err("type a file name".to_owned());
    }
    ready_to_save(app);
    let (name, uid, bytes) = encode(app, Destination::new(&name))?;
    app.cloud.busy = Some(format!("Saving {name} to the cloud"));
    submit(app, Job::Create { uid, folder: folder.to_owned(), name: name.clone(), bytes });
    Ok(json!({"saving": name, "folder": folder}))
}

/// Opens bytes fetched from the cloud, through the same path File › Open takes.
///
/// Read inline even where the app opens files as background jobs: the pages need the document this
/// opened to attach its origin to, and a background open has produced none when this returns. The
/// bytes are already in memory, so what is skipped is the decode running off the UI thread, not a
/// read.
fn open_bytes(app: &mut PhotocraftApp, entry: &Entry, bytes: Vec<u8>) {
    let before: Vec<u64> = app.session.documents().iter().map(|st| st.doc.id.0).collect();
    let background = std::mem::replace(&mut app.background_jobs, false);
    let opened = app.open_bytes(&entry.name, &bytes);
    app.background_jobs = background;
    if let Err(e) = opened {
        crate::notices::error(app, format!("could not open \"{}\" from the cloud: {e}", entry.name));
        return;
    }
    // A raw file or a colour-profile prompt answers with a dialog rather than a document, and a
    // brush file goes to its library. Nothing is said about the bytes in those cases, so there is
    // no document to attach an origin to.
    let opened = app.session.documents().iter().map(|st| st.doc.id.0).find(|id| !before.contains(id));
    if let Some(uid) = opened {
        app.cloud.origins.insert(uid, entry.clone());
    }
}

/// Marks `uid` saved at the revision the save that just landed had encoded.
fn mark_saved(app: &mut PhotocraftApp, uid: u64) {
    let Some(revision) = app.cloud_sent.remove(&uid) else { return };
    let _ = app.with_document(photocraft_doc::DocId(uid), |app| {
        if let Some(st) = app.session.active_mut() {
            st.saved_revision = revision;
        }
        Ok(())
    });
}

/// Drain the backend's answers and apply them. Called once a frame.
pub fn pump(app: &mut PhotocraftApp) {
    let events: Vec<Event> = app.cloud.rx.try_iter().collect();
    for event in events {
        match event {
            Event::Account(answer) => app.cloud.account = Some(answer),
            Event::Listed { path, entries } => {
                if path != app.cloud.mine.path {
                    continue;
                }
                app.cloud.mine.loading = false;
                match entries {
                    Ok(entries) => {
                        app.cloud.mine.entries = entries;
                        app.cloud.mine.loaded = true;
                    }
                    Err(fault) => app.cloud.mine.error = Some(fault),
                }
            }
            Event::Read { entry, bytes } => {
                app.cloud.busy = None;
                match bytes {
                    Ok(bytes) => {
                        app.cloud.page = None;
                        open_bytes(app, &entry, bytes);
                    }
                    Err(fault) => crate::notices::error(app, format!("could not read \"{}\" from the cloud: {}", entry.name, fault.text())),
                }
            }
            Event::Saved { uid, entry } => {
                app.cloud.busy = None;
                match entry {
                    Ok(entry) => {
                        app.cloud.origins.insert(uid, entry.clone());
                        mark_saved(app, uid);
                        app.cloud.page = None;
                        app.ui.status = format!("Saved \"{}\" to the cloud", entry.name);
                        app.ui.status_error = false;
                    }
                    Err(fault) => {
                        app.cloud_sent.remove(&uid);
                        crate::notices::error(app, format!("could not save to the cloud: {}", fault.text()));
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- the pages

/// The account badge: the display name once known, a spinner while the first account check is in
/// flight. Hidden entirely by the caller (`panels.rs`) when [`available`] is `false`.
pub fn who(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    start(app);
    match &app.cloud.account {
        None => {
            ui.add(egui::Spinner::new().size(13.0));
        }
        Some(Ok(account)) => {
            ui.label(egui::RichText::new(&account.display).size(12.5));
        }
        Some(Err(fault)) => {
            ui.label(egui::RichText::new(fault.text()).size(11.0));
        }
    }
}

/// Who is signed in, at the title bar's far end where every office suite puts it. Nothing is drawn
/// in a build with no cloud service.
pub fn title_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !available(app) {
        return;
    }
    who(app, ui);
}

/// The cloud folder listing shared by the Open and Save pages.
fn folder_list(app: &mut PhotocraftApp, ui: &mut egui::Ui, for_save: bool) -> Option<Entry> {
    start(app);
    let folder = app.cloud.mine.clone();
    let here = if folder.path.is_empty() { "Cloud storage".to_owned() } else { folder.path.clone() };
    ui.label(egui::RichText::new(here).size(12.0));
    ui.add_space(6.0);
    let mut chosen = None;
    egui::ScrollArea::vertical().max_height(260.0).id_salt(if for_save { "cloud_save_list" } else { "cloud_open_list" }).show(ui, |ui| {
        if folder.loading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label("Loading…");
            });
            return;
        }
        if let Some(fault) = &folder.error {
            ui.label(fault.text());
            return;
        }
        if folder.entries.is_empty() {
            ui.label("This folder is empty.");
        }
        for entry in &folder.entries {
            // A save lists everything, because a folder is browsed into and a file name can be
            // typed over. An open offers only what this application turns into a document.
            let openable = for_save || opens(entry);
            let label = if entry.kind == EntryKind::Folder { format!("\u{1f4c1} {}", entry.name) } else { entry.name.clone() };
            let clicked = ui.add_enabled_ui(openable, |ui| ui.selectable_label(false, label).clicked()).inner;
            if clicked {
                chosen = Some(entry.clone());
            }
        }
    });
    chosen
}

/// The Open from the Cloud page.
pub fn open_page(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Some(entry) = folder_list(app, ui, false) {
        open_entry(app, entry);
    }
}

/// The Save to the Cloud page. Returns whether the page should close, which is when a save was
/// accepted by the backend.
pub fn save_page(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    let chosen = folder_list(app, ui, true);
    if let Some(entry) = &chosen {
        if entry.kind == EntryKind::Folder {
            let path = entry.path.clone();
            browse(app, &path);
        } else {
            app.cloud.save_name = entry.name.clone();
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut app.cloud.save_name);
    });
    ui.label(egui::RichText::new("A name with no extension is saved as a PhotoCraft document (.pcraft).").size(11.0));
    ui.add_space(8.0);
    if ui.button("Save").clicked() {
        let folder = app.cloud.mine.path.clone();
        let name = app.cloud.save_name.clone();
        match save_as(app, &folder, &name) {
            Ok(_) => return true,
            Err(e) => crate::notices::error(app, e),
        }
    }
    false
}

/// The modal holding whichever cloud page is open. Drawn once a frame from the app's `update`.
///
/// A window rather than one of this crate's command dialogs: the page is a live listing that keeps
/// drawing while jobs land, and it owns its own actions. Nothing is drawn with no page open, which
/// is always the case in a build with no cloud service.
pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(page) = app.cloud.page else { return };
    if !available(app) {
        app.cloud.page = None;
        return;
    }
    let title = match page {
        Page::Open => "Open from the Cloud",
        Page::Save => "Save to the Cloud",
    };
    let mut open = true;
    let mut close = false;
    egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).min_width(420.0).show(ctx, |ui| {
        if let Some(busy) = app.cloud.busy.clone() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(busy);
            });
            ui.add_space(6.0);
        }
        match page {
            Page::Open => open_page(app, ui),
            Page::Save => close = save_page(app, ui),
        }
    });
    if !open || close {
        app.cloud.page = None;
    }
}

// ---------------------------------------------------------------- the shell's seams

/// The command ids this module answers, by the same names the control channel and the menus use.
const IDS: [&str; 6] = ["cloud.status", "cloud.open", "cloud.openFile", "cloud.browse", "cloud.save", "cloud.saveAs"];

/// Whether `id` is a cloud command ([`menu`] runs it).
/// File › Open from the Cloud… and Save to the Cloud…, each beside the local command it mirrors.
///
/// A build with no cloud service configured offers nothing cloud-shaped at all. An item greyed out
/// for ever would be a promise the application cannot keep.
///
/// The path comes from the row the new one sits after, never spelled here. `Row::path` is the
/// untranslated English path the menu bar groups by, so a path written out in this function would
/// match no node under any other language, exactly as the plug-in rows once did (#2540).
pub fn rows(app: &PhotocraftApp, rows: &mut Vec<craft::menu::Row>) {
    if !available(app) {
        return;
    }
    for (id, label, after) in [("cloud.open", "Open from the Cloud…", "file.open"), ("cloud.saveAs", "Save to the Cloud…", "file.saveAs")] {
        let Some(at) = rows.iter().position(|r| r.command.id == after) else { continue };
        let path: Vec<String> = rows[at].path.clone();
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        let enabled = is_enabled(app, id).unwrap_or(false);
        rows.insert(at + 1, craft::menu::Row::new(&path, id, label).enabled(enabled));
    }
}

pub fn handles(id: &str) -> bool {
    IDS.contains(&id)
}

/// Whether a cloud command is enabled: nothing without a service, and the ones that act on a
/// document need one open. `None` for an id that is not a cloud command.
pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let open = app.session.active().is_some() && app.services.export.is_some();
    Some(match id {
        "cloud.status" | "cloud.open" | "cloud.openFile" | "cloud.browse" => available(app),
        "cloud.save" | "cloud.saveAs" => available(app) && open,
        _ => return None,
    })
}

/// Run a cloud command, and File › Save for a document that came from the cloud, which writes back
/// to the cloud file it came from rather than asking for a file on this computer.
///
/// Every cloud command answers as soon as the job is stated, never when it is done: the transfer
/// runs on a worker thread, and `cloud.status` reports what is in flight. `None` for any other id.
pub(crate) fn menu(app: &mut PhotocraftApp, id: &str, params: &Json) -> Option<Result<Json, String>> {
    if id == "file.save" {
        return (params.get("path").is_none() && has_target(app)).then(|| save(app));
    }
    if !handles(id) {
        return None;
    }
    if id == "cloud.status" {
        return Some(Ok(status(app)));
    }
    if !available(app) {
        return Some(Err(UNAVAILABLE.to_owned()));
    }
    Some(match id {
        "cloud.open" => {
            app.cloud.page = Some(Page::Open);
            start(app);
            Ok(status(app))
        }
        "cloud.browse" => {
            let path = params.get("path").and_then(Json::as_str).unwrap_or("");
            browse(app, path);
            Ok(json!({"folder": path}))
        }
        // Opens something already listed, by handle or by name. The listing is what the user can
        // see, which keeps a scripted open and a clicked one on the same footing.
        "cloud.openFile" => {
            let handle = params.get("id").and_then(Json::as_str);
            let name = params.get("name").and_then(Json::as_str).unwrap_or("");
            let found = app.cloud.mine.entries.iter().find(|e| handle.is_some_and(|handle| e.id == handle) || (!name.is_empty() && e.name == name)).cloned();
            match found {
                Some(entry) if opens(&entry) => {
                    open_entry(app, entry);
                    Ok(status(app))
                }
                Some(entry) => Err(format!("PhotoCraft cannot open {}", entry.name)),
                None => Err("no such file in the cloud listing".to_owned()),
            }
        }
        "cloud.save" => save(app),
        "cloud.saveAs" => match (params.get("folder").and_then(Json::as_str), params.get("name").and_then(Json::as_str)) {
            (Some(folder), Some(name)) => save_as(app, folder, name),
            _ => {
                app.cloud.page = Some(Page::Save);
                app.cloud.save_name = app.session.active().map(|st| st.doc.name.clone()).unwrap_or_default();
                start(app);
                Ok(status(app))
            }
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    /// An in-process mock [`CloudBackend`]: answers every job at once, so the pages' listing,
    /// opening and saving logic is exercised with no cloud service running.
    struct MockBackend {
        entries: Vec<Entry>,
    }

    struct Noop;

    impl CloudBackend for Noop {
        fn submit(&self, _job: Job, _post: Post) {}
    }

    fn mock_entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            id: name.to_owned(),
            room: String::new(),
            name: name.to_owned(),
            path: name.to_owned(),
            kind,
            size: Some(0),
            modified: None,
            mime: None,
            etag: Some("1".into()),
            writable: true,
            shared_by: None,
        }
    }

    impl CloudBackend for MockBackend {
        fn submit(&self, job: Job, post: Post) {
            match job {
                Job::Account => post.send(Event::Account(Ok(Account { display: "mock@example.test".into(), capabilities: vec![] }))),
                Job::List(path) => post.send(Event::Listed { path, entries: Ok(self.entries.clone()) }),
                Job::Read { entry } => post.send(Event::Read { entry, bytes: Ok(Vec::new()) }),
                Job::Create { uid, name, .. } => post.send(Event::Saved { uid, entry: Ok(mock_entry(&name, EntryKind::File)) }),
                Job::Write { uid, entry, .. } => post.send(Event::Saved { uid, entry: Ok(entry) }),
            }
        }
    }

    #[test]
    fn every_content_type_is_one_the_extension_table_knows() {
        for mime in MIMES {
            let extension = extension_for_mime(mime);
            assert!(extension.is_some(), "{mime} has no extension");
            assert!(extension.is_some_and(|e| OPEN_EXTS.contains(&e)), "{mime} names an extension File › Open does not offer");
        }
        assert!(MIMES.contains(&"image/png"));
        assert!(!MIMES.contains(&"application/octet-stream"), "or every file in somebody's storage would be offered as a document");
        assert!(!MIMES.contains(&"application/json"), "or every JSON file would be offered as a document");
    }

    #[test]
    fn the_extensions_are_documents_and_images_not_presets_or_raws() {
        assert!(OPEN_EXTS.contains(&photocraft_format::EXTENSION), "the native format opens");
        assert!(OPEN_EXTS.contains(&"psd") && OPEN_EXTS.contains(&"png"));
        for left_out in ["abr", "grd", "aco", "ase", "kys", "psdt", "dng", "cr2", "nef", "heic"] {
            assert!(!OPEN_EXTS.contains(&left_out), ".{left_out} opens as something other than a document");
        }
        let mut seen = std::collections::HashSet::new();
        assert!(OPEN_EXTS.iter().all(|e| seen.insert(*e)), "one row per extension");
        assert!(OPEN_EXTS.iter().all(|e| *e == e.to_ascii_lowercase() && !e.starts_with('.')));
    }

    #[test]
    fn a_name_decides_the_format_and_a_bare_name_asks_the_content_type() {
        let name = |name: &str, mime: Option<&str>| export_name(Destination { name, mime });
        assert_eq!(name("Poster.pcraft", None), Ok("Poster.pcraft".to_owned()));
        assert_eq!(name("Poster.PSD", None), Ok("Poster.PSD".to_owned()), "an extension in any case");
        assert_eq!(name("Poster.png", None), Ok("Poster.png".to_owned()));
        // The name wins over the type, because the name is what somebody will double-click.
        assert_eq!(name("Poster.psd", Some("image/png")), Ok("Poster.psd".to_owned()));
        // No extension at all, and a type that says what the thing is.
        assert_eq!(name("exported", Some("image/png")), Ok("exported.png".to_owned()));
        assert_eq!(name("exported", Some("image/jpeg; charset=binary")), Ok("exported.jpg".to_owned()), "a type with a parameter on it");
        assert_eq!(name("exported", Some("image/vnd.adobe.photoshop")), Ok("exported.psd".to_owned()));
    }

    #[test]
    fn a_destination_this_app_cannot_write_is_refused_by_name() {
        let refused = |name: &str, mime: Option<&str>| export_name(Destination { name, mime }).err().unwrap_or_default();
        // Krita files are read and never written.
        let kra = refused("Art.kra", None);
        assert!(kra.contains("cannot write one"), "{kra}");
        assert!(kra.contains(".pcraft") && kra.contains(".psd"), "and it names what is available instead: {kra}");
        let unknown = refused("Art.wibble", None);
        assert!(unknown.contains(".wibble"), "{unknown}");
        // SVG is read, never written.
        assert!(refused("Art.svg", None).contains("cannot write one"));
        // A bare name with nothing to go on cannot be guessed at.
        assert!(refused("exported", None).contains("no extension"));
        let foreign = refused("exported", Some("application/octet-stream"));
        assert!(foreign.contains("application/octet-stream"), "{foreign}");
        // A readable type that is not writable is refused too.
        assert!(refused("exported", Some("image/svg+xml")).contains("image/svg+xml"));
    }

    #[test]
    fn a_typed_name_becomes_a_document_name() {
        assert_eq!(document_name(" Poster "), "Poster.pcraft");
        assert_eq!(document_name("Poster.psd"), "Poster.psd");
        assert_eq!(document_name("Poster.v2"), "Poster.v2.pcraft", "an unknown suffix is part of the name");
        assert_eq!(document_name("/a/b/Poster"), "Poster.pcraft", "a pasted path cannot become a folder");
        assert_eq!(document_name("   "), "");
    }

    #[test]
    fn opens_checks_the_extension_and_the_content_type() {
        assert!(opens(&mock_entry("Poster.psd", EntryKind::File)));
        assert!(opens(&mock_entry("Work", EntryKind::Folder)), "a folder is browsed into");
        assert!(!opens(&mock_entry("notes.txt", EntryKind::File)));
        let mut typed = mock_entry("exported", EntryKind::File);
        typed.mime = Some("image/png".to_owned());
        assert!(opens(&typed), "a bare name with a content type that says what it is");
    }

    #[test]
    fn a_build_with_no_service_offers_nothing_cloud_shaped() {
        let mut app = app();
        assert!(!available(&app));
        assert!(!has_target(&app));
        assert_eq!(is_enabled(&app, "cloud.open"), Some(false));
        assert_eq!(is_enabled(&app, "file.save"), None, "File › Save is the ordinary one");
        for id in IDS.iter().filter(|id| **id != "cloud.status") {
            assert!(menu(&mut app, id, &json!({})).is_some_and(|r| r == Err(UNAVAILABLE.to_owned())), "{id} is refused");
        }
        assert!(menu(&mut app, "cloud.status", &json!({})).is_some_and(|r| r.is_ok()), "the state is always readable");
        assert!(menu(&mut app, "file.save", &json!({})).is_none(), "File › Save is the ordinary one");
        assert!(menu(&mut app, "edit.undo", &json!({})).is_none());
        assert_eq!(save_as(&mut app, "", "Poster").unwrap_err(), UNAVAILABLE);
        assert_eq!(save(&mut app).unwrap_err(), UNAVAILABLE);
    }

    #[test]
    fn a_connected_backend_turns_the_surfaces_on() {
        let mut app = app();
        app.cloud.connect(Noop);
        assert!(available(&app));
        assert_eq!(is_enabled(&app, "cloud.open"), Some(true));
        // Nothing to save until a document is open.
        assert_eq!(is_enabled(&app, "cloud.save"), Some(false));
        assert!(menu(&mut app, "cloud.open", &json!({})).is_some_and(|r| r.is_ok()));
        assert_eq!(app.cloud.page, Some(Page::Open), "the Open page is on screen");
    }

    #[test]
    fn the_mock_backend_lists_a_folder_and_names_the_account() {
        let mut app = app();
        app.cloud.connect(MockBackend { entries: vec![mock_entry("Poster.psd", EntryKind::File)] });
        start(&mut app);
        pump(&mut app);
        assert!(app.cloud.mine.loaded, "the folder is loaded after one pump");
        assert_eq!(app.cloud.mine.entries.len(), 1);
        assert_eq!(app.cloud.mine.entries[0].name, "Poster.psd");
        assert!(matches!(&app.cloud.account, Some(Ok(a)) if a.display == "mock@example.test"));
    }

    #[test]
    fn save_needs_a_document_that_came_from_the_cloud() {
        let mut app = app();
        app.cloud.connect(MockBackend { entries: vec![] });
        let refusal = save(&mut app).unwrap_err();
        assert_eq!(refusal, "no document open");
    }

    #[test]
    fn browsing_into_a_folder_lists_it() {
        let mut app = app();
        app.cloud.connect(MockBackend { entries: vec![mock_entry("Work", EntryKind::Folder)] });
        browse(&mut app, "Work");
        pump(&mut app);
        assert_eq!(app.cloud.mine.path, "Work");
        assert!(app.cloud.mine.loaded);
    }

    #[test]
    fn open_file_refuses_a_name_the_listing_does_not_hold() {
        let mut app = app();
        app.cloud.connect(MockBackend { entries: vec![] });
        let answer = menu(&mut app, "cloud.openFile", &json!({"name": "Missing.psd"}));
        assert_eq!(answer, Some(Err("no such file in the cloud listing".to_owned())));
    }

    #[test]
    fn open_file_refuses_a_listed_file_this_app_does_not_open() {
        let mut app = app();
        app.cloud.connect(MockBackend { entries: vec![mock_entry("notes.txt", EntryKind::File)] });
        start(&mut app);
        pump(&mut app);
        let answer = menu(&mut app, "cloud.openFile", &json!({"name": "notes.txt"}));
        assert!(answer.is_some_and(|r| r.is_err_and(|e| e.contains("cannot open notes.txt"))));
    }

    #[test]
    fn the_status_report_says_whether_a_service_is_there() {
        let mut app = app();
        assert_eq!(status(&app)["available"], json!(false));
        app.cloud.connect(Noop);
        let report = status(&app);
        assert_eq!(report["available"], json!(true));
        assert_eq!(report["hasTarget"], json!(false));
        assert_eq!(report["page"], Json::Null);
    }
}
