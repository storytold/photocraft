//! Native platform services: filesystem, codecs, clipboard, crash recovery, the settings store and
//! the file panels.

use photocraft_doc::Document;
use photocraft_format::RecoveryStore;
use photocraft_ui_egui::{Recoverable, Services};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[cfg(test)]
mod recovery_tests;

/// Runs `future` to completion on this thread, sleeping until it is woken.
pub(crate) fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        // Spurious wake-ups just poll again.
        std::thread::park();
    }
}

/// Per-user settings directory: `PHOTOCRAFT_CONFIG_DIR`, else `<exe dir>/PhotoCraftData` in
/// portable mode, else the platform convention. Everything the app persists lives under it
/// except, on Windows and Linux, crash-recovery autosaves; see [`crate::app_dirs`].
pub fn config_dir() -> Option<PathBuf> {
    crate::app_dirs::config_dir()
}

/// The preferences store in [`config_dir`] (in memory when there is none), importing the older
/// `preferences.json` on the first run (`photocraft_engine::prefs::store_options`).
fn settings_store() -> (craft::settings::SettingsStore<photocraft_engine::prefs::SavedPrefs>, craft::settings::LoadReport) {
    craft::settings::SettingsStore::open_with(photocraft_engine::prefs::store_options(config_dir().as_deref()))
}

/// The preferences [`settings_store`] would load, read without opening it: the launch reads
/// before the window opens.
pub fn saved_prefs() -> photocraft_engine::prefs::SavedPrefs {
    photocraft_engine::prefs::read_saved(config_dir().as_deref())
}

fn gpu_memo_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("gpu-compositor.json"))
}

fn load_gpu_memo_text() -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(gpu_memo_file()?).ok()?;
    let cap = photocraft_ui_egui::gpu_memo::MAX_GPU_MEMO_BYTES;
    let mut bytes = Vec::new();
    file.take((cap + 1) as u64).read_to_end(&mut bytes).ok()?;
    if bytes.len() > cap {
        log::info!("ignoring oversized GPU compositor memo");
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn shader_compiler_identity() -> String {
    #[cfg(target_os = "windows")]
    {
        let exe = std::env::current_exe().ok();
        let exe_dir = exe.as_deref().and_then(Path::parent);
        match crate::gpu_startup::dx12_compiler(std::env::var("WGPU_DX12_COMPILER").ok().as_deref(), exe_dir) {
            eframe::wgpu::Dx12Compiler::Fxc => "fxc".into(),
            eframe::wgpu::Dx12Compiler::DynamicDxc { dxc_path } => format!("dxc:{dxc_path}"),
            eframe::wgpu::Dx12Compiler::StaticDxc => "static-dxc".into(),
            eframe::wgpu::Dx12Compiler::Auto => "auto".into(),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        String::new()
    }
}

/// The brush preset store (one file per preset group plus tip bitmaps; see
/// `photocraft_engine::preset_store`).
pub fn presets_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("Presets"))
}

/// Crash-recovery autosaves: under the settings directory, except where that is synced with the
/// settings: the Windows roaming profile and the Linux config folder (see
/// [`crate::app_dirs::resolve_recovery`]).
fn recovery_dir() -> Option<PathBuf> {
    crate::app_dirs::recovery_dir()
}

/// Write `bytes` crash-safely (temp file beside the target, fsync, rename, directory fsync; see
/// [`photocraft_format::atomic`]). Every document write (Save, Save As, Export, Save for Web) and
/// the preferences go through here, so a failed or interrupted save never destroys the old file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
}

/// [`write_atomic`] for an encoded save, reporting the fraction written (a background save's
/// progress, #3389). A PSD/PSB is serialized straight into the temporary file, so the whole file
/// is never held in memory next to its model (#6607).
fn write_encoded_atomic(path: &Path, encoded: &photocraft_io::Encoded, progress: &dyn Fn(f32)) -> Result<(), String> {
    match encoded {
        photocraft_io::Encoded::Bytes(bytes) => photocraft_format::atomic_write_progress(path, bytes, progress),
        photocraft_io::Encoded::Psd(_) => {
            // Only the progress bar's scale: a model whose length can't be worked out fails the
            // write the same way, through the atomic write's "not changed" error.
            let len = encoded.encoded_len().unwrap_or(0);
            photocraft_format::atomic_write_stream_progress(path, len, progress, &mut |w| encoded.write_to(w))
        }
    }
    .map_err(|e| e.to_string())
}

/// Whether `a` and `b` reach the same file: the same device and inode on Unix, the same final path
/// on Windows (where it holds the name's letter case as stored). False when either can't be read.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

/// Whether `a` and `b` name one entry in one folder: `same_file` finds they reach the same file
/// in the same folder, and the folder doesn't list both names. Another spelling of a name on a
/// file system that ignores letter case is the same entry; a hard link is another entry, which a
/// save replaces while the first name keeps the old file. False when the folder can't be read.
fn same_entry(a: &Path, b: &Path, same_file: impl Fn(&Path, &Path) -> bool) -> bool {
    let folder = |p: &Path| p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
    if !same_file(a, b) || !same_file(&folder(a), &folder(b)) {
        return false;
    }
    let (Some(x), Some(y)) = (a.file_name(), b.file_name()) else { return false };
    if x == y {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(folder(a)) else { return false };
    let listed: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.file_name())).collect();
    !(listed.iter().any(|n| n == x) && listed.iter().any(|n| n == y))
}

fn export_options(settings: &photocraft_ui_egui::ExportSettings) -> photocraft_io::ExportOptions {
    settings.io_options()
}

/// The share of a save's progress bar that encoding fills; writing the file fills the rest. A
/// 24 MP, 5-layer PSD (572 MB) takes about 3 s to encode and 0.5 s or more to write and flush.
const ENCODE_SHARE: f32 = 0.85;

/// A save on a worker thread (`Services::save_file` and `Services::automation_save`, #2017):
/// encode, then write through `write` (a PSD/PSB is serialized as it is written). Cancelling
/// while encoding leaves the target untouched; once writing starts the save can't be cancelled. The encoder's progress fills the first
/// [`ENCODE_SHARE`] of the job's progress, so the status bar and tab read "Saving NN%" (#3389).
fn save_through(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
    write: impl Fn(&str, &photocraft_io::Encoded, &dyn Fn(f32)) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    ctl.progress(0.0, "Encoding");
    let opts = export_options(settings);
    let cancelled = || ctl.check().is_err();
    let progress = |f: f32| ctl.progress(f * ENCODE_SHARE, "");
    let interrupt = photocraft_raster::Interrupt::new(&cancelled, &progress);
    let r = crate::crash_guard::guard("Save", || photocraft_io::encode_with(doc, path, &opts, &interrupt).map_err(|e| e.to_string()))?;
    ctl.commit()?;
    ctl.progress(ENCODE_SHARE, "Writing");
    // The last bit is the flush and rename after the last chunk: 100% only once they're done.
    write(path, &r.encoded, &|f| ctl.progress(ENCODE_SHARE + (0.99 - ENCODE_SHARE) * f, ""))?;
    ctl.progress(1.0, "");
    Ok(r.warnings)
}

fn save_file(
    doc: &Document,
    path: &str,
    settings: &photocraft_ui_egui::ExportSettings,
    ctl: &photocraft_ui_egui::jobs_ui::SaveCtl,
) -> Result<Vec<String>, String> {
    save_through(doc, path, settings, ctl, |p, encoded, progress| write_encoded_atomic(Path::new(p), encoded, progress))
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

/// Run `f` on the recovery store (`Err` without a config directory). The service closures never
/// call each other, so the store is never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no config directory")?))
}

/// Crash recovery: background incremental .pcraft autosaves into `dir` (`None`: no config
/// directory, so autosaves fail and nothing is recovered). Recovered documents keep their entries
/// until a newer autosave replaces them or they're saved or closed (see [`RecoveryStore`]).
fn recovery_services(dir: Option<PathBuf>) -> Services {
    let recover_dir = dir.clone();
    let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
    let (s1, s2, s4, s5) = (store.clone(), store.clone(), store.clone(), store.clone());
    Services {
        autosave: Some(Box::new(move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
            with_store(&s1, |s| s.autosave_checked(doc, revision, path.map(str::to_string)))?
        })),
        autosave_results: Some(Box::new(move || with_store(&s4, |s| s.take_completed()).unwrap_or_default())),
        flush_autosaves: Some(Box::new(move || with_store(&s5, RecoveryStore::flush).unwrap_or_default())),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
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
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&store, |s| s.adopt(id, key));
        })),
        ..Default::default()
    }
}

/// The OS clipboard read and write run under the crash guard, like Open and Export (#7582): a
/// panic in arboard, or in decoding a copied file, becomes the paste's (or copy's) error and the
/// app keeps its documents.
fn guarded_clipboard_get(mut read: impl FnMut() -> Result<Option<(u32, u32, Vec<u8>)>, String> + 'static) -> photocraft_ui_egui::ClipboardGetFn {
    Box::new(move || crate::crash_guard::guard("Paste", &mut read))
}

/// SVG reads use the same recovery boundary as raster clipboard reads.
fn guarded_clipboard_svg(mut read: impl FnMut() -> Result<Option<Vec<u8>>, String> + 'static) -> photocraft_ui_egui::ClipboardGetSvgFn {
    Box::new(move || crate::crash_guard::guard("Paste", &mut read))
}

fn guarded_clipboard_set(mut write: impl FnMut(u32, u32, &[u8]) -> Result<(), String> + 'static) -> photocraft_ui_egui::ClipboardSetFn {
    Box::new(move |w: u32, h: u32, px: &[u8]| crate::crash_guard::guard("Copy", || write(w, h, px)))
}

/// The pixels to paste when the clipboard holds copied files rather than an image (Copy in
/// Files, Finder or Explorer puts paths on the clipboard, #338): the first file that decodes,
/// upright, as RGBA8. Files that aren't images are skipped after reading only their header.
fn image_from_files(paths: &[PathBuf]) -> Option<(u32, u32, Vec<u8>)> {
    use std::io::Read;
    paths.iter().find_map(|path| {
        // text/uri-list lines end in CRLF (RFC 2483, and GTK writes them so), but arboard splits
        // on LF only, so a path copied in GNOME Files arrives with a trailing '\r'.
        let path = path.to_str().map_or_else(|| path.clone(), |s| PathBuf::from(s.trim_end_matches('\r')));
        let mut head = Vec::with_capacity(256);
        std::fs::File::open(&path).ok()?.take(256).read_to_end(&mut head).ok()?;
        // TGA has no magic number and is only guessed from the header, so trust the extension
        // too before reading what may be a large non-image file.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match photocraft_codecs::detect(&head)? {
            photocraft_codecs::Format::Tga if photocraft_codecs::from_extension(ext) != Some(photocraft_codecs::Format::Tga) => return None,
            _ => {}
        }
        let img = photocraft_codecs::decode(&std::fs::read(&path).ok()?).ok()?;
        Some((img.width(), img.height(), img.to_rgba8()))
    })
}

/// Largest clipboard image payload read on Wayland (1 GiB, a 16K × 16K RGBA BMP), so no real
/// screenshot is turned away, while a broken or hostile source can't make us allocate without end.
#[cfg(target_os = "linux")]
const MAX_CLIPBOARD_IMAGE_BYTES: u64 = 4 * 16384 * 16384;

/// The clipboard image arboard can't read on Wayland (#2546): arboard 3.6 asks for `image/png`
/// only, so a screenshot tool or clipboard manager that offers just `image/jpeg`, `image/bmp` or
/// `image/webp` pasted nothing. Reads the other `image/*` types through the same data-control
/// protocol arboard uses, and decodes them with our own codecs. `None` off Wayland.
#[cfg(target_os = "linux")]
fn wayland_clipboard_image() -> Option<(u32, u32, Vec<u8>)> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents, get_mime_types_ordered};
    let offers = get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified).ok()?;
    image_from_offers(&offers, |mime| {
        let (pipe, _) = get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Specific(mime)).ok()?;
        let mut bytes = Vec::new();
        pipe.take(MAX_CLIPBOARD_IMAGE_BYTES + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_CLIPBOARD_IMAGE_BYTES).then_some(bytes)
    })
}

/// The first clipboard offer, in the source's order, that `fetch` returns as a decodable image,
/// upright as RGBA8. Only `image/*` types are fetched, so text offers are never transferred.
#[cfg(any(target_os = "linux", test))]
fn image_from_offers(offers: &[String], mut fetch: impl FnMut(&str) -> Option<Vec<u8>>) -> Option<(u32, u32, Vec<u8>)> {
    offers.iter().filter(|mime| mime.starts_with("image/")).find_map(|mime| {
        let bytes = fetch(mime)?;
        // TGA has no magic number (see `image_from_files`), so trust it only when the type says so.
        if photocraft_codecs::detect(&bytes)? == photocraft_codecs::Format::Tga && !mime.contains("tga") {
            return None;
        }
        let img = photocraft_codecs::decode(&bytes).ok()?;
        Some((img.width(), img.height(), img.to_rgba8()))
    })
}

/// Largest SVG markup a paste reads from clipboard text (128 MiB, the size an `.svgz` may inflate
/// to).
const MAX_CLIPBOARD_SVG_BYTES: u64 = photocraft_io::svg::MAX_INFLATED_BYTES;

/// The pixels of a drawing copied as SVG markup (on macOS and Linux, VectorCraft's Copy leaves
/// only the markup, as text): one pixel per SVG unit, scaled down to the SVG import's pixel limit,
/// as RGBA8. `None` for text that isn't SVG, markup longer than [`MAX_CLIPBOARD_SVG_BYTES`], and
/// markup that doesn't parse.
fn image_from_svg_text(text: &str) -> Option<(u32, u32, Vec<u8>)> {
    if u64::try_from(text.len()).ok()? > MAX_CLIPBOARD_SVG_BYTES || !text.trim_start_matches('\u{feff}').trim_start().starts_with('<') {
        return None;
    }
    crate::crash_guard::guard("Paste", || photocraft_io::svg::rasterize_rgba8(text.as_bytes()).map_err(|e| e.to_string())).ok()
}

/// What a paste reads from the OS clipboard, in this order: the first copied file that is an
/// image (Finder also puts the file's icon on the clipboard as an image, which would otherwise
/// paste instead of the file), the clipboard's image, then a drawing copied as SVG markup. Each
/// read runs only when the ones before it found nothing. A read that fails, such as one of a busy
/// clipboard, returns its error, and the reads after it don't run.
fn clipboard_image<C>(
    clipboard: &mut C,
    files: impl FnOnce(&mut C) -> Result<Option<Vec<PathBuf>>, String>,
    image: impl FnOnce(&mut C) -> Result<Option<(u32, u32, Vec<u8>)>, String>,
    text: impl FnOnce(&mut C) -> Result<Option<String>, String>,
) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
    if let Some(img) = files(clipboard)?.and_then(|paths| image_from_files(&paths)) {
        return Ok(Some(img));
    }
    if let Some(img) = image(clipboard)? {
        return Ok(Some(img));
    }
    Ok(text(clipboard)?.and_then(|text| image_from_svg_text(&text)))
}

/// One OS clipboard read as [`clipboard_image`] takes it: no content in the format asked for is
/// `None`, and any other failure, such as a busy clipboard or a broken pipe on Wayland, is the
/// read's error.
fn clipboard_read<T>(read: Result<T, arboard::Error>) -> Result<Option<T>, String> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Clipboard text as [`clipboard_image`] takes it: like [`clipboard_read`], except that text that
/// isn't UTF-8 is `None`, like other text that isn't SVG markup.
fn clipboard_text(read: Result<String, arboard::Error>) -> Result<Option<String>, String> {
    match read {
        Err(arboard::Error::ConversionFailure) => Ok(None),
        read => clipboard_read(read),
    }
}

/// Keep SVG source for Paste As without accepting an unbounded clipboard string as a new copy.
fn svg_clipboard_source(text: Option<String>) -> Result<Option<Vec<u8>>, String> {
    let Some(text) = text else { return Ok(None) };
    if !photocraft_io::svg::is_svg(text.as_bytes()) {
        return Ok(None);
    }
    if text.len() > 16 << 20 {
        return Err("SVG clipboard data exceeds the 16 MiB limit".into());
    }
    Ok(Some(text.into_bytes()))
}

/// The shell command that starts this install under XWayland, where winit delivers file drops
/// (winit 0.30 has none on Wayland, #386). A Flatpak gets the X11 socket only without the Wayland
/// one, an AppImage relaunches by its own path, and anything else is `photocraft` on the PATH.
/// `None` outside Flatpak when there is no X server (`DISPLAY` unset) to run on.
#[cfg(any(target_os = "linux", test))]
pub fn xwayland_command(flatpak_id: Option<&str>, appimage: Option<&str>, x_display: bool) -> Option<String> {
    if let Some(id) = flatpak_id.filter(|id| !id.is_empty()) {
        return Some(format!("flatpak run --nosocket=wayland --socket=x11 {}", shell_word(id)));
    }
    let exe = appimage.filter(|path| !path.is_empty()).map_or_else(|| "photocraft".to_string(), shell_word);
    x_display.then(|| format!("WAYLAND_DISPLAY= {exe}"))
}

/// `s` as one POSIX shell word: bare when every character is safe there, else single-quoted.
#[cfg(any(target_os = "linux", test))]
fn shell_word(s: &str) -> String {
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+:,@".contains(&b)) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// What control and MCP requests may read, write and run, through `workspace` (`None`: no
/// automation, as without `--control` or an agent connection).
pub fn automation(workspace: Option<photocraft_automation::AuthorizedWorkspace>) -> photocraft_ui_egui::AutomationServices {
    let read = workspace.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let (bytes, save_path) = workspace.read_with_save_path(path).map_err(|error| error.to_string())?;
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            Ok(photocraft_ui_egui::AutomationReadResult { name, bytes, save_path })
        }) as photocraft_ui_egui::AutomationReadFn
    });
    let write = workspace.clone().map(|workspace| {
        Box::new(move |path: &str, bytes: &[u8]| workspace.write(path, bytes).map_err(|error| error.to_string())) as photocraft_ui_egui::AutomationWriteFn
    });
    let open_pcraft = workspace.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            let opened = crate::crash_guard::guard("Open", || workspace.open_pcraft(path).map_err(|e| e.to_string()))?;
            Ok(opened.map(|doc| (name, doc)))
        }) as photocraft_ui_egui::AutomationOpenPcraftFn
    });
    let locate = workspace.clone().map(|workspace| {
        Box::new(move |path: &str| workspace.location(path).map(|location| location.to_string_lossy().into_owned())) as photocraft_ui_egui::AutomationLocateFn
    });
    let save = workspace.clone().map(|workspace| {
        std::sync::Arc::new(move |doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings, ctl: &photocraft_ui_egui::jobs_ui::SaveCtl| {
            save_through(doc, path, settings, ctl, |p, encoded, _| workspace.write_stream(p, &mut |w| encoded.write_to(w)).map_err(|e| e.to_string()))
        }) as photocraft_ui_egui::SaveFileFn
    });
    let step: photocraft_ui_egui::AutomationAuthorizeFn = photocraft_automation::workspace::authorize_desktop_engine_step;
    let authorize = workspace.is_some().then_some(step);
    let command = workspace.map(|_| {
        Box::new(|id: &str, params: &serde_json::Value| {
            photocraft_automation::workspace::authorize_desktop_engine_command(id, params).map_err(|error| error.to_string())
        }) as photocraft_ui_egui::AutomationCommandFn
    });
    photocraft_ui_egui::AutomationServices { read, write, open_pcraft, locate, save, command, authorize }
}

/// What agent requests may read, write and run with the agent settings' folders
/// (`Services::agent_automation`): the automation policy always, and without folders no file
/// access. A folder that can't be opened, or isn't an absolute path, is left out (that direction
/// fails closed) and named, as at launch.
pub fn agent_automation(settings: &craft::automation::agents::AgentSettings) -> (photocraft_ui_egui::AutomationServices, Option<String>) {
    let no_flags = photocraft_automation::args::AutomationArgs { automation_read_root: Vec::new(), automation_write_root: None };
    match crate::automation_workspace(&no_flags, settings) {
        Ok((workspace, warning)) => (automation(Some(workspace)), warning.map(|w| format!("Agents can connect, but without file access there: {w}"))),
        Err(e) => (automation(Some(photocraft_automation::AuthorizedWorkspace::default())), Some(format!("Agents can connect, but without file access: {e}"))),
    }
}

/// Every native service. `wake` requests a repaint from any thread; the file panels answer through it.
pub fn native(automation: Option<photocraft_automation::AuthorizedWorkspace>, wake: craft::dialogs::Wake) -> Services {
    let (files, file_panels) = craft::dialogs::service::FileService::new(wake);
    let clip: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
    let clip_unreadable: Rc<Cell<bool>> = Rc::default();
    let automation = self::automation(automation);
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
            crate::crash_guard::guard("Open", || {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let opts = export_options(settings);
            crate::crash_guard::guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        save_file: Some(Arc::new(save_file)),
        files: Some(Arc::new(files)),
        file_panels: Some(file_panels),
        settings: Some(settings_store()),
        write: Some(Box::new(|path: &str, bytes: &[u8]| write_atomic(Path::new(path), bytes))),
        file_stamp: Some(Box::new(|path: &str| {
            // Follows a symlink, as Save writes through it to the file it points at.
            let meta = std::fs::metadata(path).ok()?;
            Some(photocraft_ui_egui::FileStamp { len: meta.len(), modified: meta.modified().ok() })
        })),
        same_file: Some(Box::new(|a: &str, b: &str| same_entry(Path::new(a), Path::new(b), same_file))),
        automation_read: automation.read,
        automation_write: automation.write,
        automation_open_pcraft: automation.open_pcraft,
        automation_locate: automation.locate,
        automation_save: automation.save,
        automation_command: automation.command,
        automation_authorize: automation.authorize,
        inbox: None,
        open_url: Some(Box::new(|url: &str| open::that(url).map_err(|e| e.to_string()))),
        beep: Some(platform_beep),
        clipboard_set_image: Some({
            let clip = clip.clone();
            guarded_clipboard_set(move |w: u32, h: u32, px: &[u8]| {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                };
                cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(px) }).map_err(|e| e.to_string())
            })
        }),
        open_external_svg: Some(Box::new(crate::external_svg::open)),
        clipboard_get_image: Some({
            let clip = clip.clone();
            let unreadable = clip_unreadable.clone();
            guarded_clipboard_get(move || {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => {
                        // arboard opens only when some backend works. In a Flatpak on Wayland none
                        // does (#5031): the sandbox hides the data-control protocol from it, and
                        // `--socket=fallback-x11` gives no X server while a compositor runs.
                        let opened = arboard::Clipboard::new();
                        unreadable.set(opened.is_err());
                        slot.insert(opened.map_err(|e| e.to_string())?)
                    }
                };
                clipboard_image(
                    cb,
                    |cb| clipboard_read(cb.get().file_list()),
                    |cb| match cb.get_image() {
                        Ok(img) => Ok(Some((img.width as u32, img.height as u32, img.bytes.into_owned()))),
                        Err(error) => {
                            #[cfg(target_os = "linux")]
                            if let Some(image) = wayland_clipboard_image() {
                                return Ok(Some(image));
                            }
                            match error {
                                arboard::Error::ContentNotAvailable => Ok(None),
                                _ => Err(error.to_string()),
                            }
                        }
                    },
                    |cb| clipboard_text(cb.get_text()),
                )
            })
        }),
        clipboard_get_svg: Some({
            let clip = clip.clone();
            guarded_clipboard_svg(move || {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                };
                svg_clipboard_source(clipboard_text(cb.get_text())?)
            })
        }),
        clipboard_unreadable: Some(Box::new(move || clip_unreadable.get())),
        photoshop_shortcuts: Some(Box::new(|| crate::photoshop_settings::live_keyboard_shortcuts(&|k| std::env::var_os(k)))),
        load_gpu_memo: Some(Box::new(load_gpu_memo_text)),
        save_gpu_memo: Some(Box::new(|text: &str| {
            if text.len() > photocraft_ui_egui::gpu_memo::MAX_GPU_MEMO_BYTES {
                return Err("GPU compositor memo exceeds 64 KiB".into());
            }
            write_atomic(&gpu_memo_file().ok_or("no config directory")?, text.as_bytes())
        })),
        shader_compiler: Some(shader_compiler_identity()),
        append_text: Some(Box::new(|path: &str, text: &str| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())
        })),
        // Set by main once the Apple-event handlers are connected (macOS).
        os_events: None,
        // Set by main, which starts loading the store before the window opens.
        preset_store: None,
        is_wayland: false,
        ..recovery_services(recovery_dir())
    }
}

/// Preferences ▸ General ▸ Beep When Done: the platform's default information beep.
fn platform_beep() {
    #[cfg(target_os = "windows")]
    {
        let _ = winsafe::MessageBeep(winsafe::co::MBP::OK);
    }
    // The user's alert sound, at the alert volume set in System Settings ▸ Sound. The terminal
    // bell below is silent for an app started from the Finder or the Dock.
    #[cfg(target_os = "macos")]
    objc2_app_kit::NSBeep();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        // No beep API among the other platforms' dependencies: write the terminal bell (audible
        // when launched from a terminal).
        use std::io::Write as _;
        let _ = std::io::stdout().write_all(b"\x07").and_then(|()| std::io::stdout().flush());
    }
}

#[cfg(test)]
mod tests;
