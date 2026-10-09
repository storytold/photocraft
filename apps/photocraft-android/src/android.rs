//! The Android shell: logcat logging, app folders, Android `Services`, and the eframe runner.

use std::path::{Path, PathBuf};

use crate::storage;
use android_activity::AndroidApp;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services};

const LOG_TAG: &str = "photocraft";

/// The entry point. android-activity's NativeActivity glue starts a thread and calls this symbol
/// by name; when it returns, the activity finishes.
// SAFETY: android-activity 0.6.1 calls this one exported symbol with the same AndroidApp
// type and Rust ABI. No other crate in this cdylib defines android_main. This proposed
// exception is limited to exporting the symbol; unsafe operations remain denied.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag(LOG_TAG));
    log_panics();
    log::info!("PhotoCraft {} starting", photocraft_engine::build_info::long_version());
    if let Err(e) = run(app) {
        log::error!("PhotoCraft stopped: {e}");
    }
}

/// A panic message would go to stderr, which Android discards: send it to logcat as well.
fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        previous(info);
    }));
}

fn run(android: AndroidApp) -> eframe::Result {
    let folders = Folders::of(&android);
    let mut options = eframe::NativeOptions { android_app: Some(android), ..Default::default() };
    // The adapter's real texture limits (egui asks for 8192 px), so big documents stay on the GPU.
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
            let mut app = PhotocraftApp::new(Session::new(), services(&folders));
            app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
            if let Some(rs) = cc.wgpu_render_state.clone() {
                let info = rs.adapter.get_info();
                log::info!("wgpu adapter {:?} ({:?}, {:?})", info.name, info.backend, info.device_type);
                app.perf.gpu_info.set_adapter(&info);
                if info.device_type != eframe::wgpu::DeviceType::Cpu {
                    app.set_wgpu(rs);
                }
            }
            Ok(Box::new(app))
        }),
    )
}

/// Where the app keeps its files. Both come from the activity; either may be missing.
struct Folders {
    /// Private to the app: preferences.
    config: Option<PathBuf>,
    /// Saved and exported documents. Prefer the app-specific external folder, then private
    /// storage. Access from other apps or USB depends on Android's storage restrictions.
    documents: Option<PathBuf>,
}

impl Folders {
    fn of(android: &AndroidApp) -> Self {
        let config = android.internal_data_path();
        let documents = android.external_data_path().or_else(|| config.clone());
        for dir in [&config, &documents].into_iter().flatten() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("couldn't create {}: {e}", dir.display());
            }
        }
        log::info!("documents folder: {documents:?}");
        Self { config, documents }
    }
}

fn services(folders: &Folders) -> Services {
    let documents = folders.documents.clone();
    let prefs_file = folders.config.as_ref().map(|dir| dir.join("preferences.json"));
    let prefs_save = prefs_file.clone();
    Services {
        import: Some(Box::new(|name: &str, bytes: &[u8]| {
            storage::guard("Open", || photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let mut opts = photocraft_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            storage::guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| match request {
            // No Android file picker yet: Open is cancelled.
            FileDialogRequest::Open { .. } => {
                log::warn!("File Open is not supported by this Android prototype yet");
                reply.send(None);
            }
            // No save dialog yet: the suggested name goes into the documents folder.
            FileDialogRequest::Save { suggested } => {
                let path = documents.as_deref().map(|dir| storage::save_path(dir, &suggested).to_string_lossy().into_owned());
                reply.send(path.map(FileDialogAnswer::SaveTo));
            }
        })),
        write: Some(Box::new(|path: &str, bytes: &[u8]| storage::save_document(Path::new(path), bytes))),
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        load_prefs: Some(Box::new(move || std::fs::read_to_string(prefs_file.as_ref()?).ok())),
        save_prefs: Some(Box::new(move |text: &str| {
            let path = prefs_save.as_ref().ok_or("no app data folder")?;
            storage::save_preferences(path, text)
        })),
        ..Default::default()
    }
}
