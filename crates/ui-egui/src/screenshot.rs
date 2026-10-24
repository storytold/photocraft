//! Screenshots and the screenshot library, Snagit-style (File › New Screenshot, File › Screenshot
//! Library…).
//!
//! New Screenshot hides PhotoCraft, runs the system's interactive capture (macOS `screencapture
//! -i`: drag a region, Space for a window, Esc to cancel), then brings PhotoCraft back with the
//! capture open. Every capture goes to the library folder (Pictures › PhotoCraft Screenshots by
//! default) named by its date and time, `2026-10-15 18-42-07.png`, plus an editable PhotoCraft
//! document of the same name (`.pcraft`), which is what opens. Arrows, labels and markers stay
//! editable layers in the `.pcraft`; every save of it also rewrites the `.png` beside it, so the
//! library always holds an up-to-date PNG to insert anywhere.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The library folder's name under the user's Pictures folder.
const FOLDER: &str = "PhotoCraft Screenshots";
/// Where the library folder setting is kept (`prefs.dialogs`).
const PREF: &str = "screenshot.library";
/// Time for the window to leave the screen before the capture starts.
#[cfg(target_os = "macos")]
const HIDE_DELAY: std::time::Duration = std::time::Duration::from_millis(350);

/// A capture in progress: the PNG path it writes and the capture thread's answer.
pub struct Pending {
    png: PathBuf,
    done: Receiver<Result<bool, String>>,
}

/// The library folder: the one chosen in the library window, else Pictures › PhotoCraft Screenshots.
pub fn library_dir(app: &PhotocraftApp) -> PathBuf {
    if let Some(dir) = app.session.prefs().dialogs.get(PREF).and_then(|v| v.get("folder")).and_then(Value::as_str).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Pictures").join(FOLDER)
}

/// A file stem for a capture taken at `stamp` (`YYYY-MM-DD HH-MM-SS`, the local time), made unique
/// in `dir` with " 2", " 3"… when two captures land in the same second.
pub fn unique_stem(dir: &Path, stamp: &str) -> String {
    let taken = |s: &str| dir.join(format!("{s}.png")).exists() || dir.join(format!("{s}.pcraft")).exists();
    if !taken(stamp) {
        return stamp.to_string();
    }
    (2..1000).map(|n| format!("{stamp} {n}")).find(|s| !taken(s)).unwrap_or_else(|| format!("{stamp} {}", std::process::id()))
}

/// The local date and time as `YYYY-MM-DD HH-MM-SS` (colons are not allowed in macOS file names).
fn local_stamp() -> Result<String, String> {
    let out = std::process::Command::new("/bin/date").arg("+%Y-%m-%d %H-%M-%S").output().map_err(|e| format!("Could not read the time: {e}"))?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.len() == 19 { Ok(s) } else { Err("Could not read the time".into()) }
}

/// File › New Screenshot: hides PhotoCraft and starts the system capture on a worker thread;
/// [`tick`] finishes it.
pub fn start(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        return Err(tl!("Screenshots are available on macOS for now").into());
    }
    if app.screenshot.is_some() {
        return Err(tl!("A screenshot is already being taken").into());
    }
    let dir = library_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let png = dir.join(format!("{}.png", unique_stem(&dir, &local_stamp()?)));
    let (tx, rx) = std::sync::mpsc::channel();
    let target = png.clone();
    let wake = ctx.clone();
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    std::thread::Builder::new()
        .name("screenshot".into())
        .spawn(move || {
            let _ = tx.send(capture(&target));
            wake.request_repaint();
        })
        .map_err(|e| format!("Could not start the screenshot: {e}"))?;
    app.screenshot = Some(Pending { png: png.clone(), done: rx });
    Ok(json!({"path": png.to_string_lossy()}))
}

/// Runs the interactive capture into `png`. Ok(false): cancelled (Esc).
#[cfg(target_os = "macos")]
fn capture(png: &Path) -> Result<bool, String> {
    std::thread::sleep(HIDE_DELAY);
    let status = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-i", "-x", "-t", "png"])
        .arg(png)
        .status()
        .map_err(|e| format!("Could not start the screen capture: {e}"))?;
    if !status.success() {
        let app = running_app_name();
        return Err(crate::i18n::fmt(
            tl!("The screen capture failed. Allow {app} in System Settings › Privacy & Security › Screen Recording"),
            &[("app", app.as_str())],
        ));
    }
    Ok(png.exists())
}

/// The name of the app bundle this runs from (the one macOS asks permission for), e.g.
/// "PhotoCraft" or "PhotoCraft Dev"; "PhotoCraft" when not run from a bundle.
#[cfg(target_os = "macos")]
fn running_app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().find_map(|a| a.file_name()?.to_str()?.strip_suffix(".app").map(str::to_string)))
        .unwrap_or_else(|| "PhotoCraft".to_string())
}

#[cfg(not(target_os = "macos"))]
fn capture(_: &Path) -> Result<bool, String> {
    Ok(false)
}

/// Each frame: when a capture finished, bring PhotoCraft back and open it for editing.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    handle_request(app, ctx);
    let Some(p) = app.screenshot.as_ref() else { return };
    let result = match p.done.try_recv() {
        Err(TryRecvError::Empty) => return,
        Ok(r) => r,
        Err(TryRecvError::Disconnected) => Err("The screen capture stopped".into()),
    };
    let png = p.png.clone();
    app.screenshot = None;
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    let opened = match result {
        Ok(true) => store(app, &png).and_then(|pcraft| app.open_path(&pcraft.to_string_lossy()).map(|_| pcraft)),
        Ok(false) => return,
        Err(e) => Err(e),
    };
    match opened {
        Ok(pcraft) => {
            let name = pcraft.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            app.ui.status = crate::i18n::fmt(tl!("Screenshot {name} saved to the library"), &[("name", name.as_str())]);
            app.ui.status_error = false;
        }
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Writes the editable `.pcraft` beside a captured `png` and returns its path.
pub fn store(app: &mut PhotocraftApp, png: &Path) -> Result<PathBuf, String> {
    let pcraft = png.with_extension("pcraft");
    let bytes = std::fs::read(png).map_err(|e| format!("{}: {e}", png.display()))?;
    let mut doc = photocraft_io::import(&png.to_string_lossy(), &bytes).map_err(|e| e.to_string())?.document;
    doc.name = pcraft.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let export = app.services.export.as_ref().ok_or("no exporter configured")?;
    let (out, _) = export(&doc, &pcraft.to_string_lossy(), &app.save_settings())?;
    let write = app.services.write.as_mut().ok_or("no writer configured")?;
    write(&pcraft.to_string_lossy(), &out)?;
    Ok(pcraft)
}

/// The file to open for `path`: a PNG in the library opens as its editable `.pcraft` (made the
/// first time), however it arrives (PhotoCraft Capture, Finder, Open With, File › Open).
pub fn editable(app: &mut PhotocraftApp, path: &str) -> String {
    let p = Path::new(path);
    let in_library = p.parent().is_some_and(|d| same_dir(d, &library_dir(app)));
    if !in_library || !p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        return path.to_string();
    }
    let pcraft = p.with_extension("pcraft");
    if pcraft.exists() {
        return pcraft.to_string_lossy().into_owned();
    }
    store(app, p).map_or_else(|_| path.to_string(), |pc| pc.to_string_lossy().into_owned())
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// After a save of the active document to `path`: a library `.pcraft` also rewrites its `.png`.
/// Returns the message to show when the PNG could not be written (the caller shows it after its
/// own "Saved" status, so neither hides the other).
pub fn after_save(app: &mut PhotocraftApp, path: &str) -> Option<String> {
    let p = Path::new(path);
    if p.extension().and_then(|e| e.to_str()) != Some("pcraft") || !p.parent().is_some_and(|d| same_dir(d, &library_dir(app))) {
        return None;
    }
    let png = p.with_extension("png").to_string_lossy().into_owned();
    let r = (|| -> Result<(), String> {
        let st = app.session.active().ok_or("no document")?;
        let export = app.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, _) = export(&st.doc, &png, &crate::ExportSettings::default())?;
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&png, &bytes)
    })();
    r.err().map(|e| crate::i18n::fmt(tl!("Saved, but the library PNG could not be updated: {error}"), &[("error", e.as_str())]))
}

/// The library's screenshots, newest first: (stem, the file to open, the PNG for its thumbnail).
pub fn entries(dir: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut stems: Vec<String> = read
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            (ext == "png" || ext == "pcraft").then(|| p.file_stem()?.to_str().map(str::to_string)).flatten()
        })
        .collect();
    stems.sort();
    stems.dedup();
    // Newest capture first, by when the file was made: names differ between PhotoCraft
    // ("2026-10-15 18-42-07") and the system's ⌘⇧4 ("Screenshot 2026-10-15 at 18.42.07"),
    // so they don't sort by time on their own.
    let made = |p: &Path| std::fs::metadata(p).and_then(|m| m.created().or_else(|_| m.modified())).ok();
    let mut list: Vec<(Option<std::time::SystemTime>, String, PathBuf, PathBuf)> = stems
        .into_iter()
        .map(|s| {
            let (pcraft, png) = (dir.join(format!("{s}.pcraft")), dir.join(format!("{s}.png")));
            let when = made(&png).or_else(|| made(&pcraft));
            let open = if pcraft.exists() { pcraft } else { png.clone() };
            (when, s, open, png)
        })
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    list.into_iter().map(|(_, s, open, png)| (s, open, png)).collect()
}

// ---------------------------------------------------------------------------------------------
// Screenshot Library window

const MARK: &str = "__screenshots";
/// Thumbnails shown at most (newest first).
const SHOWN: usize = 60;
/// Thumbnail box in points.
const THUMB: f32 = 150.0;

pub fn owns(f: &serde_json::Map<String, Value>) -> bool {
    f.contains_key(MARK)
}

/// File › Screenshot Library…: the library's screenshots as thumbnails, newest first.
pub fn open_library(app: &mut PhotocraftApp) -> Value {
    let mut f = serde_json::Map::new();
    f.insert(MARK.into(), json!(true));
    f.insert("__label".into(), json!(tl!("Screenshot Library")));
    json!({"dialog": app.ui.open_dialog(crate::state::DialogKind::Command, f)})
}

/// A click in the library window, carried to [`tick`] (the window only reads the app).
#[derive(Clone, Debug)]
enum Request {
    Open(PathBuf),
    New,
    Reveal(PathBuf),
    ChooseFolder,
}

fn request_id() -> egui::Id {
    egui::Id::new("screenshot-library-request")
}

pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui) {
    let dir = library_dir(app);
    let list = cached_entries(ui.ctx(), &dir);
    let request = |ui: &egui::Ui, r: Request| ui.ctx().data_mut(|d| d.insert_temp(request_id(), r));
    ui.horizontal(|ui| {
        if crate::widgets::primary_button(ui, tl!("New Screenshot"), 150.0).clicked() {
            request(ui, Request::New);
        }
        if ui.button(tl!("Show in Finder")).clicked() {
            request(ui, Request::Reveal(dir.clone()));
        }
        if ui.button(tl!("Change Folder…")).clicked() {
            request(ui, Request::ChooseFolder);
        }
    });
    ui.label(egui::RichText::new(dir.to_string_lossy()).small());
    ui.separator();
    if list.is_empty() {
        ui.label(tl!("No screenshots yet. Use New Screenshot to take one."));
        return;
    }
    let columns = ((ui.available_width() / (THUMB + 12.0)).floor() as usize).clamp(1, 6);
    egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
        egui::Grid::new("screenshot-library").spacing([12.0, 12.0]).show(ui, |ui| {
            for (i, (stem, open, png)) in list.iter().take(SHOWN).enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(THUMB);
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(THUMB, THUMB * 0.66), egui::Sense::click());
                    let t = crate::theme::Tokens::get(ui.ctx());
                    ui.painter().rect_filled(rect, t.radius_sm, t.field);
                    if let Some(tex) = thumbnail(ui.ctx(), png) {
                        let size = tex.size_vec2();
                        let k = (rect.width() / size.x).min(rect.height() / size.y);
                        let r = egui::Rect::from_center_size(rect.center(), size * k);
                        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    let border = if resp.hovered() { t.accent } else { t.field_border };
                    ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
                    let resp = resp.on_hover_text(tl!("Open for editing"));
                    if resp.clicked() {
                        request(ui, Request::Open(open.clone()));
                    }
                    ui.label(egui::RichText::new(stem).small());
                });
                if (i + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
    });
}

/// [`entries`] for the library window, read again only when the folder changes (a file added,
/// removed or renamed moves its modification time), not on every frame. Thumbnails are decoded
/// once per file version ([`thumbnail`]).
/// A library listing with the folder modification time it was read at.
type CachedEntries = (Option<std::time::SystemTime>, Vec<(String, PathBuf, PathBuf)>);

fn cached_entries(ctx: &egui::Context, dir: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    let modified = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    // One entry per folder, replaced when the folder changes, so stale lists don't pile up.
    let key = egui::Id::new(("screenshot-entries", dir.to_path_buf()));
    if let Some((at, list)) = ctx.data(|d| d.get_temp::<CachedEntries>(key))
        && at == modified
    {
        return list;
    }
    let list = entries(dir);
    ctx.data_mut(|d| d.insert_temp::<CachedEntries>(key, (modified, list.clone())));
    list
}

/// A thumbnail of `png`, decoded once per file version and kept for the session.
fn thumbnail(ctx: &egui::Context, png: &Path) -> Option<egui::TextureHandle> {
    let modified = std::fs::metadata(png).and_then(|m| m.modified()).ok()?;
    let key = egui::Id::new(("screenshot-thumb", png.to_path_buf(), modified));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(key)) {
        return t;
    }
    let made = (|| {
        let bytes = std::fs::read(png).ok()?;
        let doc = photocraft_io::import(&png.to_string_lossy(), &bytes).ok()?.document;
        let buf = photocraft_compose::flatten(&doc);
        let (w, h) = (buf.rect.width() as usize, buf.rect.height() as usize);
        if w == 0 || h == 0 || buf.px.len() < w * h {
            return None;
        }
        // Nearest sampling to at most 300 px (thumbnails are 150 pt on a 2× display).
        let k = (300.0 / w.max(h) as f32).min(1.0);
        let (tw, th) = (((w as f32 * k).round() as usize).max(1), ((h as f32 * k).round() as usize).max(1));
        let mut rgba = Vec::with_capacity(tw * th * 4);
        for y in 0..th {
            for x in 0..tw {
                let p = buf.px.get((y * h / th) * w + x * w / tw).copied().unwrap_or([0.0; 4]);
                rgba.extend(p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
        }
        Some(ctx.load_texture(format!("screenshot-{}", png.display()), egui::ColorImage::from_rgba_unmultiplied([tw, th], &rgba), egui::TextureOptions::LINEAR))
    })();
    ctx.data_mut(|d| d.insert_temp(key, made.clone()));
    made
}

/// Carries out a click in the library window (called every frame from [`tick`]).
fn handle_request(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(r) = ctx.data(|d| d.get_temp::<Request>(request_id())) else { return };
    ctx.data_mut(|d| d.remove::<Request>(request_id()));
    let close = |app: &mut PhotocraftApp| app.ui.dialogs.retain(|d| !owns(&d.fields));
    let result = match r {
        Request::Open(path) => {
            close(app);
            // open_paths turns a library PNG into its editable copy (`editable`).
            let opened = app.open_paths(&[path.to_string_lossy().into_owned()]);
            if opened == 1 { Ok(()) } else { Err(tl!("Could not open the screenshot").to_string()) }
        }
        Request::New => {
            close(app);
            start(app, ctx).map(|_| ())
        }
        Request::Reveal(dir) => {
            let _ = std::fs::create_dir_all(&dir);
            std::process::Command::new("/usr/bin/open").arg(&dir).spawn().map(|_| ()).map_err(|e| e.to_string())
        }
        Request::ChooseFolder => app
            .ask_folder(|app, path| {
                app.session.edit_prefs(|p| {
                    p.dialogs.insert(PREF.into(), json!({"folder": path}));
                });
                Ok(Value::Null)
            })
            .map(|_| ()),
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("photocraft-shots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_menu_items_are_enabled_without_a_document() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        assert!(crate::menus::is_enabled(&app, "file.screenshot.library"));
        assert_eq!(crate::menus::is_enabled(&app, "file.screenshot.new"), cfg!(target_os = "macos"));
    }

    #[test]
    fn the_library_listing_is_cached_once_per_folder() {
        let d = dir("cache");
        let ctx = egui::Context::default();
        assert!(cached_entries(&ctx, &d).is_empty());
        std::fs::write(d.join("2026-10-15 18-42-07.png"), b"x").unwrap();
        // The folder changed: the listing is read again and replaces the old one.
        let set_time = |secs: u64| {
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            std::fs::File::open(&d).and_then(|f| f.set_modified(t)).unwrap();
        };
        set_time(2_000_000_000);
        assert_eq!(cached_entries(&ctx, &d).len(), 1);
        std::fs::write(d.join("2026-10-15 18-43-00.png"), b"x").unwrap();
        set_time(2_000_000_100);
        assert_eq!(cached_entries(&ctx, &d).len(), 2);
        let key = egui::Id::new(("screenshot-entries", d.clone()));
        let held: Option<CachedEntries> = ctx.data(|m| m.get_temp(key));
        assert_eq!(held.map(|h| h.1.len()), Some(2), "one entry per folder, the latest listing");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_png_that_cannot_follow_the_save_is_reported() {
        let d = dir("png-error");
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                export: Some(Box::new(|doc, path, _| {
                    photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
                })),
                write: Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
                ..Default::default()
            },
        );
        app.session.edit_prefs(|p| {
            p.dialogs.insert(PREF.into(), json!({"folder": d.to_string_lossy()}));
        });
        app.run("file.new", json!({"width": 20, "height": 20, "background": "white"})).unwrap();
        // A folder where the PNG should go: writing it fails.
        std::fs::create_dir_all(d.join("2026-10-19 10-00-00.png")).unwrap();
        let pcraft = d.join("2026-10-19 10-00-00.pcraft");
        app.save_in_place(pcraft.to_string_lossy().into_owned(), crate::FormatChoices::default()).unwrap();
        assert!(app.ui.status_error, "the failed PNG is reported: {}", app.ui.status);
        assert!(app.ui.status.contains("PNG"), "{}", app.ui.status);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn names_are_the_date_and_time_and_stay_unique() {
        let d = dir("names");
        assert_eq!(unique_stem(&d, "2026-10-15 18-42-07"), "2026-10-15 18-42-07");
        std::fs::write(d.join("2026-10-15 18-42-07.png"), b"x").unwrap();
        assert_eq!(unique_stem(&d, "2026-10-15 18-42-07"), "2026-10-15 18-42-07 2");
        #[cfg(target_os = "macos")]
        {
            let s = local_stamp().unwrap();
            assert!(s.len() == 19 && s.as_bytes()[4] == b'-' && s.as_bytes()[10] == b' ' && !s.contains(':'), "{s}");
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_capture_becomes_an_editable_pair_and_saves_refresh_the_png() {
        let d = dir("pair");
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                import: Some(Box::new(|name, bytes, depth| {
                    photocraft_io::import_with_options(name, bytes, depth).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
                })),
                export: Some(Box::new(|doc, path, _| {
                    photocraft_io::export(doc, path, &Default::default()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
                })),
                write: Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
                ..Default::default()
            },
        );
        app.session.edit_prefs(|p| {
            p.dialogs.insert(PREF.into(), json!({"folder": d.to_string_lossy()}));
        });
        // A "captured" PNG.
        app.run("file.new", json!({"width": 64, "height": 40, "background": "white"})).unwrap();
        let png = d.join("2026-10-15 18-42-07.png");
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, &png.to_string_lossy(), &Default::default()).unwrap().bytes;
        std::fs::write(&png, bytes).unwrap();
        let pcraft = store(&mut app, &png).unwrap();
        assert!(pcraft.exists() && pcraft.extension().unwrap() == "pcraft");
        app.open_path(&pcraft.to_string_lossy()).unwrap();
        // An arrow, then Save: the .pcraft keeps it as a layer and the PNG is rewritten.
        let before = std::fs::read(&png).unwrap();
        let g = crate::arrow_ui::geometry([5.0, 5.0], [50.0, 30.0], egui::Modifiers::NONE).unwrap();
        crate::arrow_ui::create(&mut app, &g);
        app.save_in_place(pcraft.to_string_lossy().into_owned(), crate::FormatChoices::default()).unwrap();
        assert_ne!(std::fs::read(&png).unwrap(), before, "the PNG follows the save");
        // An automation save (control channel, MCP) refreshes the PNG as well.
        let before_auto = std::fs::read(&png).unwrap();
        let g = crate::arrow_ui::geometry([5.0, 35.0], [60.0, 5.0], egui::Modifiers::NONE).unwrap();
        crate::arrow_ui::create(&mut app, &g);
        let st = app.session.active().unwrap();
        let (doc, revision) = (st.doc.id, st.revision);
        let bytes = photocraft_io::export(&st.doc, &pcraft.to_string_lossy(), &Default::default()).unwrap().bytes;
        std::fs::write(&pcraft, bytes).unwrap();
        app.saved_automation(doc, revision, &pcraft.to_string_lossy(), &[], false);
        assert_ne!(std::fs::read(&png).unwrap(), before_auto, "an automation save refreshes the PNG too");
        let reopened = photocraft_format::read_file(&pcraft).unwrap();
        let doc = photocraft_io::import(&pcraft.to_string_lossy(), &reopened).unwrap().document;
        assert!(doc.layers.iter().any(|l| l.name.starts_with("Arrow")), "the arrow is still a layer");
        // The library lists it once, opening the editable file.
        let list = entries(&d);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1, pcraft);
        // A screenshot the system saved here (⌘⇧4) lists first, being newer, and opening it from
        // the library makes its editable copy.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let system = d.join("Screenshot 2026-10-15 at 19.01.02.png");
        std::fs::copy(&png, &system).unwrap();
        let list = entries(&d);
        assert_eq!(list[0].1, system, "newest first");
        app.ui.dialogs.clear();
        app.ui.open_dialog(crate::state::DialogKind::Command, serde_json::Map::from_iter([(MARK.to_string(), json!(true))]));
        let ctx = egui::Context::default();
        // Opening a PNG from outside (Finder, PhotoCraft Capture) also gives the editable copy.
        let other = d.join("Screenshot 2026-10-15 at 19.05.00.png");
        std::fs::copy(&png, &other).unwrap();
        assert_eq!(app.open_paths(&[other.to_string_lossy().into_owned()]), 1);
        assert!(other.with_extension("pcraft").exists());
        assert_eq!(app.session.active().and_then(|st| st.path.clone()), Some(other.with_extension("pcraft").to_string_lossy().into_owned()));
        // So does a single open (Open Recent, a drop, File › Open with a path).
        let recent = d.join("Screenshot 2026-10-15 at 19.06.00.png");
        std::fs::copy(&png, &recent).unwrap();
        app.open_path(&recent.to_string_lossy()).unwrap();
        assert_eq!(app.session.active().and_then(|st| st.path.clone()), Some(recent.with_extension("pcraft").to_string_lossy().into_owned()));
        ctx.data_mut(|data| data.insert_temp(request_id(), Request::Open(system.clone())));
        handle_request(&mut app, &ctx);
        assert!(system.with_extension("pcraft").exists(), "editable copy made");
        assert!(app.ui.dialogs.is_empty(), "the library closes");
        assert_eq!(app.session.active().and_then(|st| st.path.clone()), Some(system.with_extension("pcraft").to_string_lossy().into_owned()));
        let _ = std::fs::remove_dir_all(d);
    }
}
