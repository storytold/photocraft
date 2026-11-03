//! File › Scripts › Load Files into Stack… and Statistics: shared Source Files controls
//! with Use (Files or Folder), the list of what will be loaded, Browse…, Remove, Add Open Files and
//! Sort By Name, then Attempt to Automatically Align Source Images and Create Smart Object after
//! Loading Layers. Statistics adds Stack Mode and always creates a smart object. File › Automate ›
//! Photomerge… shares the Source Files controls with Layout above them and Blend Images Together,
//! Vignette Removal, Geometric Distortion Correction and Content Aware Fill Transparent Areas
//! below, and Use can also be Open Files (every open document, saved or not). OK runs the
//! corresponding engine command with the list or the folder.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use egui::{RichText, vec2};
use serde_json::{Map, Value, json};

use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
use crate::{PhotocraftApp, state::DialogKind, theme::Tokens, widgets};

const MARK: &str = "__loadStack";
pub const COMMAND: &str = "file.scripts.loadFilesIntoStack";
const STATISTICS: &str = "file.scripts.statistics";
const PHOTOMERGE: &str = "file.automate.photomerge";

/// Photomerge's layouts (engine ids), in Photoshop's order.
const LAYOUTS: [(&str, &str); 6] = [
    ("auto", "Auto"),
    ("perspective", "Perspective"),
    ("cylindrical", "Cylindrical"),
    ("spherical", "Spherical"),
    ("collage", "Collage"),
    ("reposition", "Reposition"),
];

/// Photomerge's options: field (the engine's parameter), label, default.
const PHOTOMERGE_OPTIONS: [(&str, &str, bool); 4] = [
    ("blend", "Blend Images Together", true),
    ("vignetteRemoval", "Vignette Removal", false),
    ("geometricCorrection", "Geometric Distortion Correction", false),
    ("contentAwareFill", "Content Aware Fill Transparent Areas", false),
];

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// Open the dialog with an empty Files list. Returns its id.
pub fn open(app: &mut PhotocraftApp) -> u64 {
    open_for(app, COMMAND, "Load Files into Stack…")
}

pub fn open_statistics(app: &mut PhotocraftApp) -> u64 {
    open_for(app, STATISTICS, "Image Statistics")
}

pub fn open_photomerge(app: &mut PhotocraftApp) -> u64 {
    open_for(app, PHOTOMERGE, "Photomerge")
}

fn statistics(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(STATISTICS)
}

fn photomerge(f: &Map<String, Value>) -> bool {
    f.get("__command").and_then(Value::as_str) == Some(PHOTOMERGE)
}

fn flag(f: &Map<String, Value>, key: &str, default: bool) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn open_for(app: &mut PhotocraftApp, command: &str, label: &str) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(command));
    f.insert("__label".into(), json!(label));
    f.insert("use".into(), json!("files"));
    f.insert("files".into(), json!([]));
    f.insert("folder".into(), json!(""));
    reset(&mut f);
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Alt+Cancel → Reset: the options back to their defaults; the source files stay.
pub fn reset(f: &mut Map<String, Value>) {
    if photomerge(f) {
        f.insert("layout".into(), json!("auto"));
        for (key, _, on) in PHOTOMERGE_OPTIONS {
            f.insert(key.into(), json!(on));
        }
    } else {
        f.insert("align".into(), json!(false));
        f.insert("smartObject".into(), json!(false));
    }
    if statistics(f) {
        f.insert("mode".into(), json!("median"));
    }
}

fn folder_mode(f: &Map<String, Value>) -> bool {
    f.get("use").and_then(Value::as_str) == Some("folder")
}

/// Use: Open Files (Photomerge only): the open documents, unsaved ones included.
fn open_mode(f: &Map<String, Value>) -> bool {
    photomerge(f) && f.get("use").and_then(Value::as_str) == Some("open")
}

fn files(f: &Map<String, Value>) -> Vec<String> {
    f.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn folder(f: &Map<String, Value>) -> &str {
    f.get("folder").and_then(Value::as_str).unwrap_or("")
}

/// How often (seconds) the chosen folder is listed again, for files added or removed meanwhile.
const RELIST_SECS: f64 = 1.0;

/// A folder's images, or why it can't be read.
type FolderListing = Result<Vec<String>, String>;

/// Lists a folder.
type Lister = fn(&str) -> FolderListing;

fn list_folder(dir: &str) -> FolderListing {
    photocraft_engine::file_cmds::list_images(dir).map_err(|e| e.to_string())
}

/// The folder listing of one dialog kept between frames (as the Color Lookup panel's LUT folder
/// scan): the folder and Browse… generation it is for, when the last listing started, and the
/// listing in flight, if any.
#[derive(Clone, Default)]
struct FolderScan {
    folder: String,
    generation: u64,
    started: Option<f64>,
    pending: Option<Pending>,
}

/// Most folder listings running at once, over all dialogs. A listing stalled on a dead share
/// can't be cancelled, so without a cap each folder chosen after it would add a thread.
const MAX_LISTINGS: usize = 4;
static LISTINGS_RUNNING: AtomicUsize = AtomicUsize::new(0);

/// A result slot a worker fills in.
type Pending = Arc<Mutex<Option<FolderListing>>>;

/// How starting a background listing went.
enum Started {
    Running(Pending),
    /// `MAX_LISTINGS` are running: try again shortly.
    Busy,
    /// No worker thread (the web, or the system refused one): list inline.
    NoThread,
}

fn scan_key(id: u64) -> egui::Id {
    egui::Id::unique(("stack-folder-listing", id))
}

/// The images in the chosen folder of dialog `id`, kept in the fields (`__images`, or
/// `__listError` when the folder can't be read) for the list and `can_confirm`. The folder is
/// listed when it changes, after Browse…, and again `RELIST_SECS` after a listing started, but
/// never while one is running: on a worker when `background` (the desktop app: a slow share must
/// not stall the window; `__listing` is set meanwhile), inline otherwise (tests, and the web,
/// which has no folders). No repaint is scheduled for it: an idle dialog doesn't poll a share.
fn folder_images(ui: &egui::Ui, f: &mut Map<String, Value>, id: u64, background: bool, list: Lister) -> Vec<String> {
    let ctx = ui.ctx();
    let key = scan_key(id);
    let now = ui.input(|i| i.time);
    let dir = folder(f).to_string();
    let generation = f.get("__listGen").and_then(Value::as_u64).unwrap_or(0);
    let mut scan = ctx.data(|m| m.get_temp::<FolderScan>(key)).unwrap_or_default();
    if scan.folder != dir || scan.generation != generation {
        // Another folder, Browse… again, or back to Use: Folder: drop the old listing, running or
        // not, and what it found (OK must wait for the new one).
        f.remove("__listed");
        scan = FolderScan { folder: dir.clone(), generation, ..FolderScan::default() };
    } else if let Some(done) = scan.pending.as_ref().and_then(|p| p.lock().unwrap_or_else(PoisonError::into_inner).take()) {
        store(f, &dir, done);
        scan.pending = None;
    }
    let mut busy = false;
    if !dir.is_empty() && scan.pending.is_none() && scan.started.is_none_or(|at| (now - at).abs() >= RELIST_SECS) {
        scan.started = Some(now);
        match if background { start(ctx, &dir, list) } else { Started::NoThread } {
            Started::Running(pending) => scan.pending = Some(pending),
            Started::Busy => {
                scan.started = None;
                busy = true;
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            Started::NoThread => {
                let took = web_time::Instant::now();
                store(f, &dir, list(&dir));
                // From when it finished, so a listing slower than RELIST_SECS isn't repeated on
                // every frame.
                scan.started = Some(now + took.elapsed().as_secs_f64());
            }
        }
    }
    if scan.pending.is_some() || busy {
        f.insert("__listing".into(), json!(true));
    } else {
        f.remove("__listing");
    }
    ctx.data_mut(|m| m.insert_temp(key, scan));
    if folder_listed(f) { images(f) } else { Vec::new() }
}

/// Drops dialog `id`'s folder listing (Use is no longer Folder): a result coming back later is
/// not taken for a fresh one.
fn drop_folder_scan(ctx: &egui::Context, id: u64) {
    ctx.data_mut(|m| m.remove::<FolderScan>(scan_key(id)));
}

/// One of the `LISTINGS_RUNNING`, given back when dropped (the worker ends, or never started).
struct RunningListing;

impl Drop for RunningListing {
    fn drop(&mut self) {
        LISTINGS_RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Starts listing `dir` on a worker thread, unless `MAX_LISTINGS` are running.
#[cfg(not(target_arch = "wasm32"))]
fn start(ctx: &egui::Context, dir: &str, list: Lister) -> Started {
    let mut n = LISTINGS_RUNNING.load(Ordering::SeqCst);
    loop {
        if n >= MAX_LISTINGS {
            return Started::Busy;
        }
        match LISTINGS_RUNNING.compare_exchange(n, n + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break,
            Err(now) => n = now,
        }
    }
    let running = RunningListing;
    let result: Pending = Arc::new(Mutex::new(None));
    let (slot, ctx, dir) = (Arc::clone(&result), ctx.clone(), dir.to_string());
    let spawned = std::thread::Builder::new().name("photocraft-folder-listing".into()).spawn(move || {
        let _running = running;
        let r = std::panic::catch_unwind(|| list(&dir)).unwrap_or_else(|_| Err(format!("{dir}: listing the folder failed")));
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
        ctx.request_repaint();
    });
    // A thread that didn't start dropped its `RunningListing` with the closure.
    if spawned.is_ok() { Started::Running(result) } else { Started::NoThread }
}

#[cfg(target_arch = "wasm32")]
fn start(_: &egui::Context, _: &str, _: Lister) -> Started {
    Started::NoThread
}

/// Keeps the listing of `dir` in the fields.
fn store(f: &mut Map<String, Value>, dir: &str, result: FolderListing) {
    match result {
        Ok(images) => {
            f.insert("__images".into(), json!(images));
            f.remove("__listError");
        }
        Err(e) => {
            f.insert("__images".into(), json!([]));
            f.insert("__listError".into(), json!(e));
        }
    }
    f.insert("__listed".into(), json!(dir));
}

/// Whether the fields hold a listing of the chosen folder (not of one chosen before it).
fn folder_listed(f: &Map<String, Value>) -> bool {
    f.get("__listed").and_then(Value::as_str) == Some(folder(f))
}

/// The images of the last listing.
fn images(f: &Map<String, Value>) -> Vec<String> {
    f.get("__images").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// How many images `folder_images` found in the chosen folder (0 until it has listed it).
fn folder_image_count(f: &Map<String, Value>) -> usize {
    if folder_listed(f) { f.get("__images").and_then(Value::as_array).map_or(0, Vec::len) } else { 0 }
}

fn selected(f: &Map<String, Value>) -> Option<usize> {
    f.get("selected").and_then(Value::as_u64).map(|i| i as usize)
}

/// Add `paths` to the Files list, skipping those already in it, in their order.
fn add(f: &mut Map<String, Value>, paths: impl IntoIterator<Item = String>) {
    let mut list = files(f);
    for p in paths {
        if !list.contains(&p) {
            list.push(p);
        }
    }
    f.insert("files".into(), json!(list));
}

fn file_name(p: &str) -> &str {
    std::path::Path::new(p).file_name().and_then(|n| n.to_str()).unwrap_or(p)
}

/// The open documents that have a file, in tab order (what Add Open Files adds).
fn open_files(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().filter_map(|d| d.path.clone()).collect()
}

/// OK needs files to load: some in the list or a chosen folder (two images for Photomerge, also
/// in the folder), or for Use: Open Files two open documents.
pub fn can_confirm(app: &PhotocraftApp, f: &Map<String, Value>) -> bool {
    let least = if photomerge(f) { 2 } else { 1 };
    !owns(f)
        || if open_mode(f) {
            app.session.documents().len() >= least
        } else if folder_mode(f) {
            !folder(f).is_empty() && (!photomerge(f) || folder_image_count(f) >= least)
        } else {
            files(f).len() >= least
        }
}

/// Browse…: files to add (Files) or the folder to load (Folder), into dialog `id` once the file
/// dialog answers.
fn browse(app: &mut PhotocraftApp, id: u64, folder: bool) {
    let r = if folder {
        app.ask_folder(move |app, path| {
            let d = app.ui.dialog_mut(id).ok_or("the dialog was closed")?;
            d.fields.insert("folder".into(), json!(path));
            // Choosing a folder again lists it again, dropping a listing still running.
            d.fields.remove("__listed");
            let generation = d.fields.get("__listGen").and_then(Value::as_u64).unwrap_or(0);
            d.fields.insert("__listGen".into(), json!(generation.wrapping_add(1)));
            Ok(Value::Null)
        })
    } else {
        let initial_dir = crate::file_dialog::last_used_dir(&app.ui.recent_files);
        app.ask_file(FileDialogRequest::Open { multiple: true, initial_dir, extensions: None }, move |app, answer| {
            let FileDialogAnswer::Paths(paths) = answer else { return Err("source files are read from disk".into()) };
            let d = app.ui.dialog_mut(id).ok_or("the dialog was closed")?;
            add(&mut d.fields, paths);
            Ok(Value::Null)
        })
    };
    if let Err(e) = r.map(|_| ()).or_else(|e| if e == crate::file_dialog::CANCELLED { Ok(()) } else { Err(e) }) {
        app.ui.status = e;
    }
}

/// The dialog's body (dialog `id`); OK and Cancel are the standard ones below it.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64) {
    let t = Tokens::get(ui.ctx());
    if statistics(f) {
        ui.horizontal(|ui| {
            ui.label(tl!("Choose Stack Mode:"));
            let mut mode = f.get("mode").and_then(Value::as_str).unwrap_or("median").to_string();
            let options: Vec<_> = photocraft_doc::StackMode::ALL.iter().map(|m| (m.id().to_string(), m.label())).collect();
            if widgets::dropdown(ui, "statistics-stack-mode", tl!("Mode"), &mut mode, &options, 180.0) {
                f.insert("mode".into(), json!(mode));
            }
        });
        ui.add_space(8.0);
    }
    if photomerge(f) {
        ui.horizontal(|ui| {
            ui.label(tl!("Layout:"));
            let mut layout = f.get("layout").and_then(Value::as_str).unwrap_or("auto").to_string();
            let options: Vec<_> = LAYOUTS.iter().map(|(id, label)| (id.to_string(), tl!(*label))).collect();
            if widgets::dropdown(ui, "photomerge-layout", tl!("Layout"), &mut layout, &options, 180.0) {
                f.insert("layout".into(), json!(layout));
            }
        });
        ui.add_space(8.0);
    }
    ui.label(RichText::new(tl!("Source Files")).font(crate::theme::semibold(13.0)));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(tl!("Use:"));
        let mut mode = if open_mode(f) { 2 } else { usize::from(folder_mode(f)) };
        let mut options = vec![(0, tl!("Files")), (1, tl!("Folder"))];
        if photomerge(f) {
            options.push((2, tl!("Open Files")));
        }
        if widgets::dropdown(ui, "load-stack-use", tl!("Use"), &mut mode, &options, 120.0) {
            f.insert("use".into(), json!(["files", "folder", "open"].get(mode).copied().unwrap_or("files")));
            f.remove("selected");
        }
    });
    ui.add_space(6.0);
    let open_mode = open_mode(f);
    let folder_mode = folder_mode(f);
    // Folder and Open Files list what will be merged; only Files is edited by hand.
    let fixed = folder_mode || open_mode;
    let mut browse_now = false;
    ui.horizontal_top(|ui| {
        // The list: the chosen files, the images in the chosen folder, or the open documents.
        if !folder_mode {
            drop_folder_scan(ui.ctx(), id);
        }
        let shown: Vec<String> = if open_mode {
            app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
        } else if folder_mode {
            folder_images(ui, f, id, app.background_jobs, list_folder)
        } else {
            files(f)
        };
        let sel = selected(f);
        egui::Frame::NONE.fill(t.field).stroke(egui::Stroke::new(1.0, t.field_border)).corner_radius(t.radius_sm).inner_margin(4.0).show(ui, |ui| {
            ui.set_min_size(vec2(300.0, 160.0));
            ui.set_max_width(300.0);
            egui::ScrollArea::vertical().id_salt("load-stack-files").max_height(160.0).auto_shrink([false, false]).show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    if folder_mode && !folder(f).is_empty() {
                        ui.label(RichText::new(folder(f)).color(t.text_dim));
                        let listed = folder_listed(f);
                        if let Some(e) = f.get("__listError").and_then(Value::as_str).filter(|_| listed) {
                            ui.label(RichText::new(e).color(t.danger));
                        } else if f.contains_key("__listing") && !listed {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new().size(12.0));
                                ui.label(RichText::new(tl!("Reading…")).color(t.text_dim));
                            });
                        }
                    }
                    for (i, p) in shown.iter().enumerate() {
                        // Document names are shown as they are (one may contain a slash).
                        let name = if open_mode { p.as_str() } else { file_name(p) };
                        let row = ui.add(egui::Button::selectable(!fixed && sel == Some(i), name)).on_hover_text(p.as_str());
                        if row.clicked() && !fixed {
                            f.insert("selected".into(), json!(i));
                        }
                    }
                });
            });
        });
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            if ui.add_enabled_ui(!open_mode, |ui| widgets::secondary_button(ui, tl!("Browse…"), 120.0)).inner.clicked() {
                browse_now = true;
            }
            let can_remove = !fixed && sel.is_some_and(|i| i < files(f).len());
            if ui.add_enabled_ui(can_remove, |ui| widgets::secondary_button(ui, tl!("Remove"), 120.0)).inner.clicked()
                && let Some(i) = sel
            {
                let mut list = files(f);
                list.remove(i);
                f.insert("files".into(), json!(list));
                f.remove("selected");
            }
            let open = open_files(app);
            if ui.add_enabled_ui(!fixed && !open.is_empty(), |ui| widgets::secondary_button(ui, tl!("Add Open Files"), 120.0)).inner.clicked() {
                add(f, open);
            }
            if ui.add_enabled_ui(!fixed && files(f).len() > 1, |ui| widgets::secondary_button(ui, tl!("Sort By Name"), 120.0)).inner.clicked() {
                let mut list = files(f);
                list.sort_by_key(|p| file_name(p).to_lowercase());
                f.insert("files".into(), json!(list));
                f.remove("selected");
            }
        });
    });
    ui.add_space(8.0);
    if photomerge(f) {
        for (key, label, default) in PHOTOMERGE_OPTIONS {
            // The engine fills transparent areas from the blended composite only.
            let enabled = key != "contentAwareFill" || flag(f, "blend", true);
            let mut on = flag(f, key, default);
            if ui.add_enabled_ui(enabled, |ui| widgets::checkbox(ui, &mut on, label)).inner.changed() {
                f.insert(key.into(), json!(on));
            }
        }
    } else {
        for (key, label) in [("align", tl!("Attempt to Automatically Align Source Images")), ("smartObject", tl!("Create Smart Object after Loading Layers"))] {
            if statistics(f) && key == "smartObject" {
                continue;
            }
            let mut on = flag(f, key, false);
            if widgets::checkbox(ui, &mut on, label).changed() {
                f.insert(key.into(), json!(on));
            }
        }
    }
    if browse_now {
        // The file dialog's answer lands in the dialog's own fields: save this frame's edits first.
        if let Some(d) = app.ui.dialog_mut(id) {
            d.fields = f.clone();
        }
        browse(app, id, folder_mode);
    }
}

/// OK: load the list (or the folder, or the open documents) into a new document.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let paths = if open_mode(f) {
        Value::Null
    } else if folder_mode(f) {
        let dir = folder(f);
        if dir.is_empty() {
            return Err("Choose a folder to load".into());
        }
        json!(dir)
    } else {
        let list = files(f);
        if list.is_empty() {
            return Err("Add the files to load".into());
        }
        json!(list)
    };
    let align = flag(f, "align", false);
    let smart = flag(f, "smartObject", false);
    let r = if photomerge(f) {
        let mut p = json!({"layout": f.get("layout").and_then(Value::as_str).unwrap_or("auto")});
        if paths.is_null() {
            p["useOpenDocuments"] = json!(true);
        } else {
            p["paths"] = paths;
        }
        for (key, _, default) in PHOTOMERGE_OPTIONS {
            p[key] = json!(flag(f, key, default));
        }
        // Content Aware Fill is greyed out without Blend Images Together: don't send it then.
        p["contentAwareFill"] = json!(flag(f, "contentAwareFill", false) && flag(f, "blend", true));
        app.run(PHOTOMERGE, p)
    } else if statistics(f) {
        let mode = f.get("mode").and_then(Value::as_str).unwrap_or("median");
        app.run(STATISTICS, json!({"paths": paths, "align": align, "mode": mode}))
    } else {
        app.run(COMMAND, json!({"paths": paths, "align": align, "createSmartObject": smart}))
    };
    if r.is_ok() {
        app.sync_views();
    }
    r
}

#[cfg(test)]
mod tests;
