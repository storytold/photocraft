//! PhotoCraft's half of Open and Save against cloud storage.
//!
//! The contract types here ([`CloudBackend`], [`Job`], [`Post`], [`Event`], [`Fault`],
//! [`Account`]) are the shapes `crates/ui/src/cloud/backend.rs` states on
//! `storytold/craft-libs` at `fd7232d7`, the commit that merged storytold/craft-libs#160. Field
//! for field, including [`Fault::Conflict`] and `Post`'s generation guard. The vocabulary they
//! pass around is the real thing, taken straight from `craft_cloud` at that revision.
//!
//! They are restated here rather than imported for one reason, and it is a pin, not a design
//! choice. The shared pages and this contract live in craft-libs' `crates/ui`, which this
//! workspace reaches as the `craft` framework crate. That crate is pinned at `a6b9d9c9`, which is
//! not an ancestor of `fd7232d7`: the two diverge from `83cb1133`, and `a6b9d9c9` carries no cloud
//! module at all while `fd7232d7` carries no framework crate. They also pin different
//! `storytold/egui` revisions, so taking `crates/ui` from one and `crates/craft` from the other
//! would put two egui revisions in one graph. Until a craft-libs revision carries both, the
//! contract cannot be imported at a revision this workspace can build.
//!
//! When one does, this module becomes `pub use craft_ui::cloud::{...}` and the two page calls in
//! [`window`] become the shared `open_page` and `save_page`. Nothing else here has to move: those
//! are `(app, ui)` functions on the merged revision too.
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
//! Real-time shared editing and the presence row are not part of this contract. RFC-0020 states
//! that surface separately from the documents contract here. The contracts crate it calls for is
//! not open yet. Sharing does belong to the merged contract, through `craft_ui::share`. It
//! arrives with the pages once the pin above allows them. This module states the Files half
//! alone. That is what a framework crate pinned without a cloud module can support.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Extensions this application opens, lower case and without a dot. The same set its desktop
    /// file picker offers.
    pub extensions: &'static [&'static str],
    /// Content types it opens, matched against the start of what the provider said. The match is a
    /// prefix, because a provider commonly appends a profile to a type it is otherwise right about.
    pub mimes: &'static [&'static str],
    /// What one is called in a sentence a user reads.
    pub noun: &'static str,
    /// The extension a typed name without a known one gets.
    pub default_extension: &'static str,
}

pub const DOCUMENT: DocumentKind = DocumentKind { extensions: OPEN_EXTS, mimes: &MIMES, noun: "document", default_extension: photocraft_format::EXTENSION };

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

/// Work for a host's worker. Each one answers with one or more [`Event`].
///
/// The variants this application states are a subset of the contract's. The field shapes are the
/// contract's own, so the swap described in the module header stays a re-export.
pub enum Job {
    /// Who is signed in, and what the provider behind the service can do.
    Account,
    /// One folder's contents. An empty path is the user's root.
    List(String),
    /// A document's bytes. `into` names the open document a reload replaces.
    Read { entry: Entry, into: Option<u64> },
    /// Save under a new name in `folder`.
    Create { uid: u64, folder: String, name: String, bytes: Vec<u8> },
    /// Save back over `entry`, refusing to overwrite a newer version. The entry carries the
    /// [`Entry::etag`] the write is conditional on.
    Write { uid: u64, entry: Entry, bytes: Vec<u8> },
}

/// What a worker reports back.
pub enum Event {
    Account(Result<Account, Fault>),
    Listed { path: String, entries: Result<Vec<Entry>, Fault> },
    Read { entry: Entry, into: Option<u64>, bytes: Result<Vec<u8>, Fault> },
    Saved { uid: u64, entry: Result<Entry, Fault> },
}

/// Where a worker's answers go. Handed to [`CloudBackend::submit`] with the job it belongs to.
///
/// A worker is free to keep it and answer later, or to answer several times. A send after the
/// window moved on is not a failure.
pub struct Post {
    tx: Sender<Event>,
    /// The session generation as of the moment the job this answers was issued, beside a handle to
    /// what it is now. Connecting a backend bumps the live value. A reply that arrives after is for
    /// a session that is gone, and must not land on the one that replaced it.
    generation: Arc<AtomicU64>,
    issued: u64,
}

impl Post {
    fn new(tx: Sender<Event>, generation: Arc<AtomicU64>) -> Post {
        let issued = generation.load(Ordering::Relaxed);
        Post { tx, generation, issued }
    }

    /// Another handle to the same place, for a worker that answers more than once. Carries the same
    /// generation: every answer to one job was issued at the same moment.
    pub fn dup(&self) -> Post {
        Post { tx: self.tx.clone(), generation: self.generation.clone(), issued: self.issued }
    }

    /// Delivers the event, unless the session moved on since the job this answers was issued.
    pub fn send(&self, event: Event) {
        if self.generation.load(Ordering::Relaxed) != self.issued {
            return;
        }
        let _ = self.tx.send(event);
    }
}

/// Somewhere a [`Job`] can run without blocking a frame.
///
/// `submit` must return immediately. Answers arrive through `post` whenever they are ready, or
/// never, and the pages survive either.
pub trait CloudBackend {
    fn submit(&self, job: Job, post: Post);
}

/// What the service said about the signed-in user.
///
/// Capabilities are reported by the service and never assumed here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Account {
    pub display: String,
    /// What the provider behind the service can do, carried through from `craft_cloud::Capability`.
    pub capabilities: Vec<Capability>,
    /// Until this is true, Open and Save have nowhere to go.
    pub provider_linked: bool,
}

impl Account {
    pub fn can(&self, capability: Capability) -> bool {
        self.capabilities.contains(&capability)
    }
}

/// A call that did not succeed, in the shapes the pages act on differently.
///
/// One case is worth telling apart from every other refusal. A conditional write the server refused
/// means somebody else's work is in the stored copy. That one must never be retried and must never
/// be forced, which is why [`Fault::Conflict`] has a shape of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// No cloud account is linked yet. This is a state with an obvious next step. It is never drawn
    /// as an error.
    NotLinked,
    /// Somebody else saved while this user was editing. The save was refused and their work is
    /// intact.
    Conflict,
    /// The session expired. The user signs in again.
    Unauthenticated,
    Unreachable(String),
    Refused(String),
}

impl Fault {
    /// One sentence for a person to read. `noun` is what the application calls one of its
    /// documents.
    pub fn text(&self, noun: &str) -> String {
        match self {
            Fault::NotLinked => tl!("No cloud account is linked yet.").to_owned(),
            Fault::Conflict => crate::i18n::fmt(tl!("Somebody else saved this {noun} while you were editing it."), &[("noun", noun)]),
            Fault::Unauthenticated => tl!("Your cloud session has expired. Sign in again.").to_owned(),
            Fault::Unreachable(detail) => crate::i18n::fmt(tl!("The cloud service could not be reached. {detail}"), &[("detail", detail)]),
            Fault::Refused(detail) => detail.clone(),
        }
    }

    /// A short machine-readable name, for the control channel.
    pub fn code(&self) -> &'static str {
        match self {
            Fault::NotLinked => "not_linked",
            Fault::Conflict => "conflict",
            Fault::Unauthenticated => "unauthenticated",
            Fault::Unreachable(_) => "unreachable",
            Fault::Refused(_) => "refused",
        }
    }
}

/// One folder's contents, as last fetched.
///
/// A listing that failed leaves [`Folder::error`] set and nothing in flight. That is a settled
/// state, not a reason to ask again: [`start`] leaves it alone, so the page can draw the refusal
/// and the user decides whether to retry. Only [`browse`] clears it.
#[derive(Clone, Default)]
pub struct Folder {
    pub path: String,
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub loaded: bool,
    pub error: Option<Fault>,
}

impl Folder {
    /// Whether a listing of this folder is worth asking for. False while one is in flight, false
    /// once one answered, and false once one was refused.
    fn wants_listing(&self) -> bool {
        !self.loading && !self.loaded && self.error.is_none()
    }
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
    /// Where each open document's cloud copy is, by document id. Present for a document opened
    /// from the cloud, and for one that was saved to it.
    ///
    /// A cloud destination is not by itself a reason for File › Save to go there. A document that
    /// also has a file of its own on disk keeps that file as what Save writes (see
    /// [`prefers_cloud`]). Each entry carries the [`Entry::etag`] the next write is conditional on.
    origins: HashMap<u64, Entry>,
    /// Whether an account check is in flight. The title bar asks on every frame, and without this
    /// the answer being outstanding would be a reason to ask again.
    account_asked: bool,
    /// Bumped whenever a backend is connected, so answers from the previous one are dropped rather
    /// than landing on the session that replaced it (see [`Post::send`]).
    generation: Arc<AtomicU64>,
}

impl Default for CloudFiles {
    fn default() -> CloudFiles {
        let (tx, rx) = std::sync::mpsc::channel();
        CloudFiles {
            backend: None,
            tx,
            rx,
            account: None,
            mine: Folder::default(),
            save_name: String::new(),
            page: None,
            busy: None,
            origins: HashMap::new(),
            account_asked: false,
            generation: Arc::new(AtomicU64::new(0)),
        }
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
        // Anything the previous backend still owes is for a session that is gone.
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.account = None;
        self.account_asked = false;
        self.mine = Folder::default();
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
        backend.submit(job, Post::new(app.cloud.tx.clone(), app.cloud.generation.clone()));
    }
}

/// Ensures the account and the current folder are on their way. Call when a cloud page, or the
/// account badge, is about to be drawn.
///
/// Every caller runs once a frame, so each request here is guarded by what is already in flight or
/// already settled. A service that is slow, or down, would otherwise be asked again sixty times a
/// second for as long as the window is open.
pub fn start(app: &mut PhotocraftApp) {
    if !available(app) {
        return;
    }
    if app.cloud.account.is_none() && !app.cloud.account_asked {
        app.cloud.account_asked = true;
        submit(app, Job::Account);
    }
    if app.cloud.mine.wants_listing() {
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
    app.cloud.busy = Some(crate::i18n::fmt(tl!("Opening {name} from the cloud"), &[("name", &entry.name)]));
    submit(app, Job::Read { entry, into: None });
}

/// Whether the active document has a cloud copy at all, from being opened out of the cloud or from
/// having been saved to it. This says where a cloud save would go. It does not say that File › Save
/// should go there (see [`prefers_cloud`]).
pub fn has_target(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| app.cloud.origins.contains_key(&st.doc.id.0))
}

/// Whether File › Save on the active document should write to the cloud.
///
/// Only when the cloud is the document's only home. A document that also has a file of its own on
/// this computer keeps that file as what Save writes, for ever: the user opened a file, and Save
/// means that file. Saving a copy of it to the cloud does not change where it lives, and taking
/// Save over would leave every later edit unwritten on disk while the window reported it saved.
pub fn prefers_cloud(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|st| st.path.is_none() && app.cloud.origins.contains_key(&st.doc.id.0))
}

/// Whether a cloud save that counts as some document's own save is still on the wire.
///
/// Only the writes that will mark a document saved are here. A copy is not one of them, and leaves
/// its document dirty whatever happens. The unsaved-changes prompt reads this the way it reads
/// [`PhotocraftApp::saving`] for a background save: it waits rather than asking the user again
/// while the answer they already gave is in flight.
pub fn saving(app: &PhotocraftApp) -> bool {
    !app.cloud_sent.is_empty()
}

/// Forget the cloud copy of `uid`, because the document's own file is somewhere else now.
///
/// A local Save As gives the document a file on this computer. From then on that file is what Save
/// writes, and the cloud copy it came from is a different file that this document is no longer the
/// editor of.
pub fn forget_origin(app: &mut PhotocraftApp, uid: u64) {
    app.cloud.origins.remove(&uid);
    app.cloud_sent.remove(&uid);
}

/// The whole of the cloud state, for the control channel and for tests.
pub fn status(app: &PhotocraftApp) -> Json {
    json!({
        "available": available(app),
        "account": app.cloud.account.as_ref().map(|a| match a {
            Ok(account) => json!({"display": account.display, "providerLinked": account.provider_linked}),
            Err(fault) => json!({"error": fault.text(DOCUMENT.noun), "code": fault.code()}),
        }),
        "folder": app.cloud.mine.path,
        "folderError": app.cloud.mine.error.as_ref().map(|fault| json!({"error": fault.text(DOCUMENT.noun), "code": fault.code()})),
        "page": app.cloud.page.map(|p| match p { Page::Open => "open", Page::Save => "save" }),
        "busy": app.cloud.busy,
        "hasTarget": has_target(app),
        "prefersCloud": prefers_cloud(app),
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
/// Two cases make the write a copy rather than the document's save. A format that cannot hold the
/// document (a flat file for a layered one) is a copy, as in File › Save As. A document that lives
/// in a file on this computer is a copy too: nothing reached that file, so the edits in it are
/// still unsaved. In both cases the cloud file is written and the document stays dirty, which is
/// what keeps Quit from discarding work it never wrote anywhere.
///
/// Only a document whose sole home is the cloud has its revision recorded here, and only that
/// revision is ever marked saved when the write lands (see [`mark_saved`]).
fn encode(app: &mut PhotocraftApp, into: Destination<'_>) -> Result<(String, u64, Vec<u8>), String> {
    let name = export_name(into)?;
    let (uid, revision, bytes, warnings, copy) = {
        let st = app.session.active().ok_or("no document open")?;
        let settings = app.save_settings();
        let export = app.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&st.doc, &name, &settings)?;
        let copy = app.saves_copy(&name, None)? || st.path.is_some();
        (st.doc.id.0, st.revision, bytes, warnings, copy)
    };
    if !copy {
        app.cloud_sent.insert(uid, revision);
    }
    crate::notices::io_warnings(app, &crate::i18n::fmt(tl!("Saved {name}"), &[("name", &name)]), &warnings);
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
pub fn pump(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let events: Vec<Event> = app.cloud.rx.try_iter().collect();
    for event in events {
        match event {
            Event::Account(answer) => {
                app.cloud.account = Some(answer);
                app.cloud.account_asked = false;
            }
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
                    // Latched on purpose. `start` leaves a refused listing alone, so this survives
                    // to be drawn and the user decides whether to ask again.
                    Err(fault) => app.cloud.mine.error = Some(fault),
                }
            }
            Event::Read { entry, into, bytes } => {
                app.cloud.busy = None;
                // One open document this read replaces, for a reload. Nothing states one yet.
                let _ = into;
                match bytes {
                    Ok(bytes) => {
                        app.cloud.page = None;
                        open_bytes(app, &entry, bytes);
                    }
                    Err(fault) => {
                        let detail = fault.text(DOCUMENT.noun);
                        crate::notices::error(
                            app,
                            crate::i18n::fmt(tl!("{name} could not be read from the cloud. {detail}"), &[("name", &entry.name), ("detail", &detail)]),
                        );
                    }
                }
            }
            Event::Saved { uid, entry } => {
                app.cloud.busy = None;
                match entry {
                    Ok(entry) => {
                        let name = entry.name.clone();
                        // The entry the service answered with carries the new etag. Keeping it is
                        // what makes the next write conditional on this version.
                        app.cloud.origins.insert(uid, entry);
                        mark_saved(app, uid);
                        app.cloud.page = None;
                        app.ui.status = crate::i18n::fmt(tl!("Saved {name} to the cloud"), &[("name", &name)]);
                        app.ui.status_error = false;
                        // The close prompt may have been waiting on exactly this save. Only a write
                        // that was the document's own save can release it, which is what
                        // `mark_saved` above decided by leaving a copy dirty.
                        crate::discard_ui::saved_document(app, ctx, photocraft_doc::DocId(uid));
                    }
                    Err(fault) => {
                        // Nothing reached the service, so the revision that was going out is not
                        // saved anywhere.
                        app.cloud_sent.remove(&uid);
                        if fault == Fault::Conflict {
                            // Never folded into a generic refusal, and never retried. The stored
                            // copy holds somebody else's work, and this user's is still here.
                            conflict(app, uid);
                        } else {
                            let detail = fault.text(DOCUMENT.noun);
                            crate::notices::error(app, crate::i18n::fmt(tl!("The cloud save did not happen. {detail}"), &[("detail", &detail)]));
                        }
                    }
                }
            }
        }
    }
}

/// A conditional write the service refused, reported as its own outcome.
///
/// The stored copy moved on while this user was editing. Their edits are intact and still unsaved,
/// which is the one thing they have to be told. Saving under a new name is the way out, so the
/// refusal says that and the document stays dirty.
fn conflict(app: &mut PhotocraftApp, uid: u64) {
    let name = app.cloud.origins.get(&uid).map(|entry| entry.name.clone()).unwrap_or_default();
    let detail = Fault::Conflict.text(DOCUMENT.noun);
    crate::notices::error(
        app,
        crate::i18n::fmt(
            tl!("{detail} Your changes to {name} are still here and still unsaved. Save to the Cloud under another name to keep them."),
            &[("detail", &detail), ("name", &name)],
        ),
    );
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
            ui.label(egui::RichText::new(fault.text(DOCUMENT.noun)).size(11.0));
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
    let here = if folder.path.is_empty() { tl!("Cloud storage").to_owned() } else { folder.path.clone() };
    ui.label(egui::RichText::new(here).size(12.0));
    ui.add_space(6.0);
    let mut chosen = None;
    let mut retry = false;
    egui::ScrollArea::vertical().max_height(260.0).id_salt(if for_save { "cloud_save_list" } else { "cloud_open_list" }).show(ui, |ui| {
        if folder.loading {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0));
                ui.label(tl!("Loading…"));
            });
            return;
        }
        // A refused listing is latched, so it is still here to be read. Asking again is the user's
        // decision, which is the only thing that clears it.
        if let Some(fault) = &folder.error {
            ui.label(fault.text(DOCUMENT.noun));
            ui.add_space(6.0);
            retry = ui.button(tl!("Try again")).clicked();
            return;
        }
        if folder.entries.is_empty() {
            ui.label(tl!("This folder is empty."));
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
    if retry {
        let path = app.cloud.mine.path.clone();
        browse(app, &path);
    }
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
        ui.label(tl!("Name:"));
        ui.text_edit_singleline(&mut app.cloud.save_name);
    });
    ui.label(egui::RichText::new(tl!("A name with no extension is saved as a PhotoCraft document (.pcraft).")).size(11.0));
    ui.add_space(8.0);
    if ui.button(tl!("Save")).clicked() {
        let folder = app.cloud.mine.path.clone();
        let name = app.cloud.save_name.clone();
        match save_as(app, &folder, &name) {
            Ok(_) => return true,
            Err(e) => crate::notices::error(app, e),
        }
    }
    false
}

/// Whether a cloud page is on screen. The menu gate reads it, so the menus behave while it is up
/// exactly as they do for a framework dialog ([`crate::menus::menu_gate`]).
pub fn page_open(app: &PhotocraftApp) -> bool {
    app.cloud.page.is_some()
}

/// The modal holding whichever cloud page is open. Drawn once a frame from the app's `update`.
///
/// A real modal, so the page takes the window: a backdrop, Escape to close, and the menu gate told
/// about it through [`page_open`]. Nothing is drawn with no page open, which is always the case in
/// a build with no cloud service.
///
/// The pages themselves are plain `(app, ui)` functions on purpose. That is the shape
/// `craft_ui::cloud::{open_page, save_page}` has on the revision that merged
/// storytold/craft-libs#160, so adopting the shared pages is a swap of these two calls rather than
/// a rewrite of the surface around them. It is also why they are not a `craft::ui::dialog`
/// `StackDialog`: that trait's `show` is handed no application, and the shared pages are given one.
pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(page) = app.cloud.page else { return };
    if !available(app) {
        app.cloud.page = None;
        return;
    }
    let title = match page {
        Page::Open => tl!("Open from the Cloud"),
        Page::Save => tl!("Save to the Cloud"),
    };
    let mut close = false;
    let answer = egui::Modal::new(egui::Id::unique("pc-cloud-page")).show(ctx, |ui| {
        ui.set_min_width(420.0);
        ui.heading(title);
        ui.add_space(6.0);
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
        ui.add_space(8.0);
        if ui.button(tl!("Cancel")).clicked() {
            close = true;
        }
    });
    // Escape, or a click on the backdrop, is a cancel.
    if close || answer.should_close() {
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
    // File › Save goes to the cloud only for a document whose sole home is the cloud. One that also
    // has a file on this computer is saved to that file by the ordinary path.
    if id == "file.save" {
        return (params.get("path").is_none() && prefers_cloud(app)).then(|| save(app));
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
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    fn app() -> Box<PhotocraftApp> {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
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

    /// A backend that accepts every job and answers nothing.
    struct Noop;

    impl CloudBackend for Noop {
        fn submit(&self, _job: Job, _post: Post) {}
    }

    /// An in-process mock backend: answers every job at once, so the pages' listing, opening and
    /// saving logic is exercised with no cloud service running.
    struct MockBackend {
        entries: Vec<Entry>,
    }

    impl CloudBackend for MockBackend {
        fn submit(&self, job: Job, post: Post) {
            match job {
                Job::Account => post.send(Event::Account(Ok(Account { display: "mock@example.test".into(), capabilities: vec![], provider_linked: true }))),
                Job::List(path) => post.send(Event::Listed { path, entries: Ok(self.entries.clone()) }),
                Job::Read { entry, into } => post.send(Event::Read { entry, into, bytes: Ok(Vec::new()) }),
                Job::Create { uid, name, .. } => post.send(Event::Saved { uid, entry: Ok(mock_entry(&name, EntryKind::File)) }),
                Job::Write { uid, entry, .. } => post.send(Event::Saved { uid, entry: Ok(entry) }),
            }
        }
    }

    /// A backend that refuses everything with `fault`.
    struct Refusing(Fault);

    impl CloudBackend for Refusing {
        fn submit(&self, job: Job, post: Post) {
            match job {
                Job::Account => post.send(Event::Account(Ok(Account::default()))),
                Job::List(path) => post.send(Event::Listed { path, entries: Ok(Vec::new()) }),
                Job::Read { entry, into } => post.send(Event::Read { entry, into, bytes: Err(self.0.clone()) }),
                Job::Create { uid, .. } | Job::Write { uid, .. } => post.send(Event::Saved { uid, entry: Err(self.0.clone()) }),
            }
        }
    }

    #[derive(Default)]
    struct Counts {
        accounts: AtomicUsize,
        lists: AtomicUsize,
    }

    /// Accepts every job and answers nothing, counting what it was asked.
    struct Silent(Arc<Counts>);

    impl CloudBackend for Silent {
        fn submit(&self, job: Job, _post: Post) {
            match job {
                Job::Account => self.0.accounts.fetch_add(1, Ordering::Relaxed),
                Job::List(_) => self.0.lists.fetch_add(1, Ordering::Relaxed),
                _ => 0,
            };
        }
    }

    /// Refuses every listing, counting what it was asked.
    struct Failing(Arc<Counts>);

    impl CloudBackend for Failing {
        fn submit(&self, job: Job, post: Post) {
            match job {
                Job::Account => {
                    self.0.accounts.fetch_add(1, Ordering::Relaxed);
                    post.send(Event::Account(Ok(Account::default())));
                }
                Job::List(path) => {
                    self.0.lists.fetch_add(1, Ordering::Relaxed);
                    post.send(Event::Listed { path, entries: Err(Fault::Unreachable("the host is down".to_owned())) });
                }
                _ => {}
            }
        }
    }

    /// An app with one small document, an exporter, and a writer that records the paths it got.
    fn app_with_document() -> (Box<PhotocraftApp>, Arc<Mutex<Vec<String>>>) {
        let written: Arc<Mutex<Vec<String>>> = Arc::default();
        let w = written.clone();
        let mut app = app();
        app.run("file.new", json!({"width": 8, "height": 8})).expect("a new document");
        app.sync_views();
        app.services.export = Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new()))));
        app.services.write = Some(Box::new(move |p: &str, _: &[u8]| {
            w.lock().expect("the writer's record").push(p.to_owned());
            Ok(())
        }));
        (app, written)
    }

    /// Gives the active document a file of its own on this computer, and marks it saved there.
    fn lives_on_disk(app: &mut PhotocraftApp, path: &str) {
        let st = app.session.active_mut().expect("a document");
        st.path = Some(path.to_owned());
        st.saved_revision = st.revision;
    }

    fn dirty(app: &PhotocraftApp) -> bool {
        app.session.documents()[0].is_dirty()
    }

    #[test]
    fn every_content_type_is_one_the_extension_table_knows() {
        for mime in MIMES {
            let extension = extension_for_mime(mime);
            assert!(extension.is_some(), "{mime} has no extension");
            assert!(extension.is_some_and(|e| OPEN_EXTS.contains(&e)), "{mime} names an extension File  Open does not offer");
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
        assert_eq!(name("exported", Some("image/png")), Ok("exported.png".to_owned()));
        assert_eq!(name("exported", Some("image/jpeg; charset=binary")), Ok("exported.jpg".to_owned()), "a type with a parameter on it");
        assert_eq!(name("exported", Some("image/vnd.adobe.photoshop")), Ok("exported.psd".to_owned()));
    }

    #[test]
    fn a_destination_this_app_cannot_write_is_refused_by_name() {
        let refused = |name: &str, mime: Option<&str>| export_name(Destination { name, mime }).err().unwrap_or_default();
        let kra = refused("Art.kra", None);
        assert!(kra.contains("cannot write one"), "{kra}");
        assert!(kra.contains(".pcraft") && kra.contains(".psd"), "and it names what is available instead: {kra}");
        assert!(refused("Art.wibble", None).contains(".wibble"));
        assert!(refused("Art.svg", None).contains("cannot write one"));
        assert!(refused("exported", None).contains("no extension"));
        assert!(refused("exported", Some("application/octet-stream")).contains("application/octet-stream"));
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
    fn every_fault_reads_as_a_sentence_and_has_a_code() {
        let faults = [
            Fault::NotLinked,
            Fault::Conflict,
            Fault::Unauthenticated,
            Fault::Unreachable("the host is down".to_owned()),
            Fault::Refused("the server said no".to_owned()),
        ];
        let mut codes = std::collections::HashSet::new();
        for fault in &faults {
            assert!(!fault.text(DOCUMENT.noun).is_empty(), "{fault:?} says something");
            assert!(codes.insert(fault.code()), "{fault:?} has a code of its own");
        }
        assert!(Fault::Conflict.text("document").contains("document"));
        assert_eq!(Fault::Conflict.code(), "conflict");
    }

    #[test]
    fn a_build_with_no_service_offers_nothing_cloud_shaped() {
        let mut app = app();
        assert!(!available(&app));
        assert!(!has_target(&app));
        assert!(!prefers_cloud(&app));
        assert_eq!(is_enabled(&app, "cloud.open"), Some(false));
        assert_eq!(is_enabled(&app, "file.save"), None, "File  Save is the ordinary one");
        for id in IDS.iter().filter(|id| **id != "cloud.status") {
            assert!(menu(&mut app, id, &json!({})).is_some_and(|r| r == Err(UNAVAILABLE.to_owned())), "{id} is refused");
        }
        assert!(menu(&mut app, "cloud.status", &json!({})).is_some_and(|r| r.is_ok()), "the state is always readable");
        assert!(menu(&mut app, "file.save", &json!({})).is_none(), "File  Save is the ordinary one");
        assert!(menu(&mut app, "edit.undo", &json!({})).is_none());
        assert_eq!(save_as(&mut app, "", "Poster").unwrap_err(), UNAVAILABLE);
        assert_eq!(save(&mut app).unwrap_err(), UNAVAILABLE);
    }

    #[test]
    fn a_connected_backend_turns_the_surfaces_on() {
        let mut app = app();
        app.cloud.connect(Noop);
        assert!(available(&app));
        assert!(!page_open(&app));
        assert_eq!(is_enabled(&app, "cloud.open"), Some(true));
        assert_eq!(is_enabled(&app, "cloud.save"), Some(false), "nothing to save until a document is open");
        assert!(menu(&mut app, "cloud.open", &json!({})).is_some_and(|r| r.is_ok()));
        assert_eq!(app.cloud.page, Some(Page::Open));
        assert!(page_open(&app), "the menu gate can see it");
    }

    #[test]
    fn the_mock_backend_lists_a_folder_and_names_the_account() {
        let mut app = app();
        let ctx = app.ctx().clone();
        app.cloud.connect(MockBackend { entries: vec![mock_entry("Poster.psd", EntryKind::File)] });
        start(&mut app);
        pump(&mut app, &ctx);
        assert!(app.cloud.mine.loaded, "the folder is loaded after one pump");
        assert_eq!(app.cloud.mine.entries.len(), 1);
        assert!(matches!(&app.cloud.account, Some(Ok(a)) if a.display == "mock@example.test" && a.provider_linked));
    }

    /// The title bar calls `start` through `who` on every frame. Five frames with the answer still
    /// outstanding is one account check, not five.
    #[test]
    fn the_account_check_is_issued_once_not_once_per_frame() {
        let counts = Arc::new(Counts::default());
        let mut app = app();
        app.cloud.connect(Silent(counts.clone()));
        for _ in 0..5 {
            start(&mut app);
        }
        assert_eq!(counts.lists.load(Ordering::Relaxed), 1, "the folder listing is asked once");
        assert_eq!(counts.accounts.load(Ordering::Relaxed), 1, "and so is the account check");
    }

    /// A listing that was refused is a settled state. Asking again every frame would hammer a
    /// service that is down for as long as the window is open.
    #[test]
    fn a_refused_listing_is_not_retried_every_frame() {
        let counts = Arc::new(Counts::default());
        let mut app = app();
        let ctx = app.ctx().clone();
        app.cloud.connect(Failing(counts.clone()));
        for _ in 0..5 {
            start(&mut app);
            pump(&mut app, &ctx);
        }
        assert_eq!(counts.lists.load(Ordering::Relaxed), 1, "one refused listing, not one request per frame");
    }

    /// The page calls `start` and then reads the folder it is about to draw. A fault cleared by a
    /// retry in that same pass would never be seen by anybody.
    #[test]
    fn the_listing_fault_survives_long_enough_to_be_drawn() {
        let counts = Arc::new(Counts::default());
        let mut app = app();
        let ctx = app.ctx().clone();
        app.cloud.connect(Failing(counts.clone()));
        start(&mut app);
        pump(&mut app, &ctx);
        assert!(app.cloud.mine.error.is_some(), "the fault is recorded");
        start(&mut app);
        assert!(app.cloud.mine.error.is_some(), "and it is still there for the page to show");
        // Asking again is the user's decision, and only that clears it.
        browse(&mut app, "");
        assert!(app.cloud.mine.error.is_none(), "a retry clears it");
        assert_eq!(counts.lists.load(Ordering::Relaxed), 2, "and asks exactly once more");
    }

    /// A cloud destination is not a reason for File  Save to stop writing the document's own file.
    /// Taking Save over would leave every later edit unwritten on disk while the window reported
    /// the document saved.
    #[test]
    fn save_to_the_cloud_does_not_take_over_the_documents_own_file() {
        let (mut app, written) = app_with_document();
        lives_on_disk(&mut app, "/pics/poster.pcraft");
        let ctx = app.ctx().clone();

        // An ordinary File  Save writes the document's own file.
        app.run("layer.new.layer", json!({})).expect("a new layer");
        crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).expect("the baseline save");
        assert_eq!(&*written.lock().expect("the record"), &["/pics/poster.pcraft".to_owned()], "the baseline save writes to disk");

        // File  Save to the Cloud, answered by the service.
        app.cloud.connect(MockBackend { entries: vec![] });
        crate::menus::invoke(&mut app, &ctx, "cloud.saveAs", json!({"folder": "", "name": "poster.pcraft"})).expect("the cloud save");
        pump(&mut app, &ctx);
        assert!(has_target(&app), "the document now has a cloud copy");
        assert!(!prefers_cloud(&app), "but its own file is still what Save means");

        // Edit again and press the Save key.
        app.run("layer.new.layer", json!({})).expect("another layer");
        written.lock().expect("the record").clear();
        crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).expect("the second save");
        assert_eq!(&*written.lock().expect("the record"), &["/pics/poster.pcraft".to_owned()], "File  Save still writes the document's own file");
    }

    /// Nothing reached the document's own file, so the edits in it are still unsaved. Quitting must
    /// ask about them rather than discarding them.
    #[test]
    fn a_cloud_save_does_not_mark_a_disk_document_clean() {
        let (mut app, written) = app_with_document();
        lives_on_disk(&mut app, "/pics/poster.pcraft");
        let ctx = app.ctx().clone();
        app.cloud.connect(MockBackend { entries: vec![] });
        app.run("layer.new.layer", json!({})).expect("a new layer");
        assert!(dirty(&app));
        crate::menus::invoke(&mut app, &ctx, "cloud.saveAs", json!({"folder": "", "name": "poster.pcraft"})).expect("the cloud save");
        pump(&mut app, &ctx);
        assert!(written.lock().expect("the record").is_empty(), "nothing reached the document's own file");
        assert!(dirty(&app), "so the edit to the disk document is still unsaved");
    }

    /// A document whose only home is the cloud is a different case: the cloud write is its save.
    #[test]
    fn a_cloud_only_document_is_saved_back_to_the_cloud() {
        let (mut app, written) = app_with_document();
        let ctx = app.ctx().clone();
        app.cloud.connect(MockBackend { entries: vec![] });
        app.run("layer.new.layer", json!({})).expect("a new layer");
        crate::menus::invoke(&mut app, &ctx, "cloud.saveAs", json!({"folder": "", "name": "poster.pcraft"})).expect("the cloud save");
        assert!(saving(&app), "the bytes are on the wire");
        pump(&mut app, &ctx);
        assert!(!saving(&app), "and the answer landed");
        assert!(prefers_cloud(&app), "the cloud is this document's only home");
        assert!(!dirty(&app), "so the cloud write is its save");
        assert!(written.lock().expect("the record").is_empty(), "and no file on this computer was touched");

        // The Save key now goes back to the same entry, through the Write arm.
        app.run("layer.new.layer", json!({})).expect("another layer");
        assert!(dirty(&app));
        crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).expect("the write back");
        pump(&mut app, &ctx);
        assert!(!dirty(&app), "writing back to the cloud entry saved it");
        assert!(written.lock().expect("the record").is_empty(), "still nothing on this computer");
    }

    /// A refused save leaves the document unsaved, whatever the reason.
    #[test]
    fn a_refused_save_leaves_the_document_unsaved() {
        let (mut app, _written) = app_with_document();
        let ctx = app.ctx().clone();
        app.cloud.connect(Refusing(Fault::Unreachable("the host is down".to_owned())));
        app.run("layer.new.layer", json!({})).expect("a new layer");
        crate::menus::invoke(&mut app, &ctx, "cloud.saveAs", json!({"folder": "", "name": "poster.pcraft"})).expect("the attempt");
        pump(&mut app, &ctx);
        assert!(dirty(&app), "nothing was stored, so nothing is saved");
        assert!(!saving(&app), "and the attempt is over");
        assert!(!has_target(&app), "no cloud copy was made");
        assert!(app.ui.status_error, "the refusal is reported");
    }

    /// A refused conditional write is the one refusal with a shape of its own. Somebody else's work
    /// is in the stored copy, and this user's is still here.
    #[test]
    fn a_refused_conditional_write_is_reported_as_a_conflict() {
        let (mut app, _written) = app_with_document();
        let ctx = app.ctx().clone();
        app.cloud.connect(Refusing(Fault::Conflict));
        app.run("layer.new.layer", json!({})).expect("a new layer");
        crate::menus::invoke(&mut app, &ctx, "cloud.saveAs", json!({"folder": "", "name": "poster.pcraft"})).expect("the attempt");
        pump(&mut app, &ctx);
        assert!(dirty(&app), "their work is intact and still unsaved");
        let said = app.ui.status.clone();
        assert!(said.contains("Somebody else saved"), "the conflict is named as itself: {said}");
        assert!(said.contains("still unsaved"), "and it says the edits are still here: {said}");
        assert!(!said.contains("could not be reached"), "never folded into a generic refusal: {said}");
    }

    /// The entry the service answered with is kept, so the next write is conditional on the version
    /// this user actually has.
    #[test]
    fn the_answered_entry_is_kept_so_the_next_write_is_conditional() {
        let (mut app, _written) = app_with_document();
        let ctx = app.ctx().clone();
        app.cloud.connect(MockBackend { entries: vec![] });
        crate::menus::invoke(&mut app, &ctx, "cloud.saveAs", json!({"folder": "", "name": "poster.pcraft"})).expect("the cloud save");
        pump(&mut app, &ctx);
        let uid = app.session.active().expect("a document").doc.id.0;
        let kept = app.cloud.origins.get(&uid).expect("the cloud copy");
        assert_eq!(kept.etag.as_deref(), Some("1"), "the etag the service answered with is the one kept");
        assert_eq!(kept.name, "poster.pcraft");
    }

    /// A read the service refused opens nothing and attaches no cloud copy to anything.
    #[test]
    fn a_refused_read_opens_nothing() {
        let mut app = app();
        let ctx = app.ctx().clone();
        app.cloud.connect(Refusing(Fault::Unauthenticated));
        open_entry(&mut app, mock_entry("Poster.psd", EntryKind::File));
        assert!(app.cloud.busy.is_some(), "the read is in flight");
        pump(&mut app, &ctx);
        assert!(app.cloud.busy.is_none(), "and over");
        assert!(app.session.documents().is_empty(), "no document was opened");
        assert!(app.ui.status_error, "the refusal is reported");
    }

    /// Bytes no decoder accepts are reported, and no cloud copy is attached to a document that was
    /// never opened.
    #[test]
    fn a_read_whose_bytes_do_not_decode_attaches_no_origin() {
        let mut app = app();
        let ctx = app.ctx().clone();
        // The mock answers a read with no bytes at all.
        app.cloud.connect(MockBackend { entries: vec![] });
        open_entry(&mut app, mock_entry("Poster.psd", EntryKind::File));
        pump(&mut app, &ctx);
        assert!(app.cloud.origins.is_empty(), "nothing opened, so nothing has a cloud copy");
        assert!(app.session.documents().is_empty());
    }

    #[test]
    fn the_status_report_says_whether_a_service_is_there() {
        let mut app = app();
        assert_eq!(status(&app)["available"], json!(false));
        app.cloud.connect(Noop);
        let report = status(&app);
        assert_eq!(report["available"], json!(true));
        assert_eq!(report["hasTarget"], json!(false));
        assert_eq!(report["prefersCloud"], json!(false));
        assert_eq!(report["page"], Json::Null);
        assert_eq!(report["folderError"], Json::Null);
    }

    /// A refused listing is readable by a script, with the code that says which refusal it was.
    #[test]
    fn the_status_report_carries_a_listing_refusal() {
        let counts = Arc::new(Counts::default());
        let mut app = app();
        let ctx = app.ctx().clone();
        app.cloud.connect(Failing(counts));
        start(&mut app);
        pump(&mut app, &ctx);
        assert_eq!(status(&app)["folderError"]["code"], json!("unreachable"));
    }

    /// Connecting a backend drops what the previous one still owed, so a late answer cannot land on
    /// the session that replaced it.
    #[test]
    fn a_late_answer_from_a_replaced_backend_is_dropped() {
        struct Keeper(Arc<Mutex<Option<Post>>>);
        impl CloudBackend for Keeper {
            fn submit(&self, job: Job, post: Post) {
                if matches!(job, Job::Account) {
                    *self.0.lock().expect("the kept post") = Some(post);
                }
            }
        }

        let mut app = app();
        let ctx = app.ctx().clone();
        let kept: Arc<Mutex<Option<Post>>> = Arc::default();
        app.cloud.connect(Keeper(kept.clone()));
        start(&mut app);
        // A second service replaces the first.
        app.cloud.connect(Noop);
        // The first one answers at last.
        if let Some(post) = kept.lock().expect("the kept post").take() {
            post.send(Event::Account(Ok(Account { display: "gone@example.test".into(), capabilities: vec![], provider_linked: true })));
        }
        pump(&mut app, &ctx);
        assert!(app.cloud.account.is_none(), "the previous session's answer is not this session's account");
    }
}
