//! Native PhotoCraft entry point for Android (no WebView, no JS).
//!
//! Winit/eframe owns Activity lifecycle, rotation, touch and the Vulkan surface.
//! Platform-only services are separate from the desktop app to avoid X11,
//! Wayland, desktop file pickers and subprocess dependencies.

#![cfg(target_os = "android")]
// Exporting android_main and the JNI callback requires unsafe symbol attributes.
// The entrypoints and their bodies do not dereference raw pointers.

mod picker;
mod services;
mod spen;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Once;

use android_activity::{AndroidApp, input::Axis};
use photocraft_engine::Session;
use photocraft_ui_egui::theme::ThemeKind;
use photocraft_ui_egui::{PhotocraftApp, FileDialogReply, FileDialogRequest};

type PendingDialogs = Rc<RefCell<VecDeque<(FileDialogRequest, FileDialogReply)>>>;

/// Called by android-activity, not by the desktop main().
/// The AndroidApp is supplied by the OS for every Activity creation.
#[unsafe(no_mangle)]
fn android_main(android_app: AndroidApp) {
    static INIT_LOGGER: Once = Once::new();
    INIT_LOGGER.call_once(|| {
        android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info));
    });

    // Non-position axes are opt-in in android-activity. Winit still owns the
    // input queue; we must never drain the same queue ourselves.
    android_app.enable_motion_axis(Axis::Pressure);
    android_app.enable_motion_axis(Axis::Tilt);
    android_app.enable_motion_axis(Axis::Orientation);

    let data_dir = android_app.internal_data_path();
    let documents_dir = data_dir.as_ref().map(|path| path.join("Documents"));
    let dialogs: PendingDialogs = Rc::default();

    let mut options = eframe::NativeOptions {
        android_app: Some(android_app),
        viewport: egui::ViewportBuilder::default()
            .with_title("PhotoCraft")
            .with_fullscreen(true)
            .with_decorations(false),
        ..Default::default()
    };

    // Keep the engine's GPU canvas limited to the real hardware adapter.
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
    }

    let result = eframe::run_native("PhotoCraft", options, Box::new(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(Session::new(), services::android_services(data_dir, dialogs.clone()));
        app.set_theme(&cc.egui_ctx, ThemeKind::Pro);
        spen::install(Some(app.stylus.feed.clone()));
        photocraft_ui_egui::android_touch::configure_touch_ui(&cc.egui_ctx);
        if let Some(state) = cc.wgpu_render_state.clone() {
            app.perf.gpu_info.set_adapter(&state.adapter.get_info());
            // Diagnostic workaround: the Android Vulkan compositor may terminate
            // the Activity when a new document exceeds 128 pixels on either axis.
            // Keep Vulkan for egui, but render document pixels on the tested CPU
            // path until we can identify the offending GPU operation.
            //
            // To compare the original behavior, build with
            // PHOTOCRAFT_ANDROID_GPU_CANVAS=1 in the Rust build environment.
            if option_env!("PHOTOCRAFT_ANDROID_GPU_CANVAS") == Some("1") {
                log::info!("Android document canvas: GPU (diagnostic override)");
                app.set_wgpu(state);
            } else {
                log::warn!("Android document canvas: CPU diagnostic fallback; egui still uses Vulkan");
            }
        } else {
            log::warn!("No wgpu render state: PhotoCraft uses the CPU canvas");
        }
        Ok(Box::new(AndroidShell {
            app,
            documents_dir,
            dialogs,
            picker: None,
        }))
    }));

    spen::install(None);
    if let Err(err) = result {
        log::error!("PhotoCraft Android Activity terminated: {err}");
    }
}

/// The editor is exactly the same PhotoCraftApp used by desktop and web.
/// Only the file picker UI is supplied by the Android shell.
struct AndroidShell {
    app: PhotocraftApp,
    documents_dir: Option<PathBuf>,
    dialogs: PendingDialogs,
    picker: Option<picker::Picker>,
}

impl eframe::App for AndroidShell {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);

        if self.picker.is_none() {
            if let Some((request, reply)) = self.dialogs.borrow_mut().pop_front() {
                self.picker = Some(picker::Picker::new(request, reply));
            }
        }

        if let Some(picker) = self.picker.as_mut() {
            if picker.show(ui.ctx(), self.documents_dir.as_deref()) {
                self.picker = None;
            }
        }
    }
}

impl Drop for AndroidShell {
    fn drop(&mut self) {
        // A re-created Activity must not write samples into the old session.
        spen::install(None);
    }
}
