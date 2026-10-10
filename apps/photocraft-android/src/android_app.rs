//! Android application runtime and eframe runner setup.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use photocraft_doc::Document;
use photocraft_engine::Session;
use photocraft_ui_egui::file_dialog::FileDialogRequest;
use photocraft_ui_egui::{PhotocraftApp, Services};

use crate::mobile;

/// Starts the PhotoCraft Android runner.
pub fn start(android_app: android_activity::AndroidApp) {
    let internal_path = android_app.internal_data_path().map(PathBuf::from);

    let mut options = eframe::NativeOptions::default();
    options.android_app = Some(android_app);

    // On Android, use GLES backend by default to avoid ANativeWindow double-binding
    // crashes when probing Vulkan on emulators, Waydroid, and older GPUs.
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(ref mut create) = options.wgpu_options.wgpu_setup {
        let use_vulkan = std::env::var("PHOTOCRAFT_VULKAN").map(|v| v == "1" || v == "true").unwrap_or(false);
        if use_vulkan {
            create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
        } else {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
    }

    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);

    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            photocraft_ui_egui::PhotocraftApp::setup_context(&cc.egui_ctx, photocraft_ui_egui::theme::ThemeKind::Pro);
            cc.egui_ctx.global_style_mut(mobile::configure_mobile_style);

            let mobile_state: mobile::SharedMobileState = Arc::new(Mutex::new(mobile::MobileState::default()));

            let services = create_mobile_services(internal_path.clone(), mobile_state.clone());
            let mut app = PhotocraftApp::new(Session::new(), services);

            // Apply phone layout defaults (canvas-first, collapsed dock, mobile action bar).
            mobile::apply_mobile_defaults(&mut app);

            if let Some(rs) = cc.wgpu_render_state.clone() {
                log::info!("photocraft-android: wgpu backend {:?}", rs.adapter.get_info().backend);
                app.set_wgpu(rs);
            }

            Ok(Box::new(MobileShell { app, state: mobile_state }))
        }),
    );

    if let Err(e) = result {
        log::error!("PhotoCraft failed to run on Android: {e}");
    }
}

/// The Android application wrapper.
struct MobileShell {
    app: PhotocraftApp,
    state: mobile::SharedMobileState,
}

impl eframe::App for MobileShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = ui.ctx().clone();

        // 1. Mobile top action bar (hidden during fullscreen)
        if !state.is_fullscreen {
            mobile::render_mobile_action_bar(&mut self.app, &mut state, ui);
        }

        // 2. Main PhotoCraft canvas & panels
        self.app.ui(ui, frame);

        // 3. Floating overlays
        mobile::render_fullscreen_exit_button(&ctx, &mut state, &mut self.app);
        mobile::render_file_picker_dialog(&ctx, &mut state);
        mobile::render_save_dialog(&ctx, &mut state);
    }
}

/// Creates platform services adapted for the Android app sandbox.
fn create_mobile_services(storage_dir: Option<PathBuf>, mobile_state: mobile::SharedMobileState) -> Services {
    let mut services = Services::default();

    if let Some(dir) = storage_dir {
        let prefs_dir = dir.join("prefs");
        let _ = std::fs::create_dir_all(&prefs_dir);
        let prefs_file = prefs_dir.join("photocraft_prefs.json");

        let read_path = prefs_file.clone();
        services.load_prefs = Some(Box::new(move || std::fs::read_to_string(&read_path).ok()));

        let write_path = prefs_file;
        services.save_prefs = Some(Box::new(move |content: &str| {
            std::fs::write(&write_path, content).map_err(|e| e.to_string())
        }));
    }

    // Import service for decoding image formats.
    services.import = Some(Box::new(|name: &str, bytes: &[u8]| {
        photocraft_io::import(name, bytes)
            .map(|r| (r.document, r.warnings))
            .map_err(|e| e.to_string())
    }));

    // Export service for saving images.
    services.export = Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
        let mut opts = photocraft_io::ExportOptions::default();
        if let Some(q) = settings.jpeg_quality {
            opts.encode.jpeg_quality = q;
        }
        opts.encode.webp_lossless = settings.webp_lossless;
        if let Some(q) = settings.webp_quality {
            opts.encode.webp_quality = q;
        }
        opts.tiff_layers = settings.tiff_layers;
        opts.xmp = if settings.xmp_all {
            photocraft_io::XmpEmbed::All
        } else {
            photocraft_io::XmpEmbed::None
        };
        photocraft_io::export(doc, path, &opts)
            .map(|r| (r.bytes, r.warnings))
            .map_err(|e| e.to_string())
    }));

    // File dialog service wired to mobile in-app picker.
    let state_dialog = mobile_state;
    services.file_dialog = Some(Box::new(move |request, _parent, reply| {
        if let Ok(mut state) = state_dialog.lock() {
            match request {
                FileDialogRequest::Open { .. } => {
                    state.file_picker_open = true;
                    state.selected_file = None;
                    state.pending_open = Some(reply);
                }
                FileDialogRequest::Save { suggested } => {
                    state.save_filename = suggested;
                    state.save_dialog_open = true;
                    state.pending_save = Some(reply);
                }
            }
        }
    }));

    // Write service for persisting files to disk.
    services.write = Some(Box::new(|path: &str, bytes: &[u8]| {
        let p = std::path::Path::new(path);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(p, bytes).map_err(|e| e.to_string())
    }));

    services
}
