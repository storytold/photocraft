//! PhotoCraft on iPadOS and Android tablets.
//!
//! The same [`PhotocraftApp`] as the desktop and web builds.
//! - **iPadOS:** a SwiftUI app owns the process, the scene and the frame clock, and hosts the
//!   canvas as a Metal-backed view. [`host`] draws the app into that view; the C functions in
//!   [`ios`] are the whole interface (`ios/PhotoCraft.h`). Native chrome (toolbars, the layers
//!   panel, document pickers) grows around the view in SwiftUI.
//! - **Android:** for now eframe's runner owns a `NativeActivity` (the Kotlin shell subclasses
//!   it). It moves to the same host under Compose next.
//!
//! Prototype scope: documents are made with File › New. No file pickers yet (the platform
//! document pickers replace `rfd`), no preference persistence, no control server.

#[cfg(any(target_os = "ios", target_os = "android", test))]
mod host;
#[cfg(target_os = "ios")]
mod ios;

#[cfg(any(target_os = "ios", target_os = "android", test))]
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
#[cfg(any(target_os = "ios", target_os = "android", test))]
use photocraft_engine::Session;
#[cfg(any(target_os = "ios", target_os = "android", test))]
use photocraft_ui_egui::theme::ThemeKind;
#[cfg(any(target_os = "ios", target_os = "android", test))]
use photocraft_ui_egui::{PhotocraftApp, Services};

/// Build the app on `ctx`, drawing its canvas with wgpu when a render state is given.
#[cfg(any(target_os = "ios", target_os = "android", test))]
#[cfg_attr(test, expect(dead_code, reason = "only the native shells start the app; tests build the host alone"))]
fn create_app(ctx: &egui::Context, wgpu: Option<eframe::egui_wgpu::RenderState>) -> PhotocraftApp {
    PhotocraftApp::install_type_reset_hooks(ctx);
    PhotocraftApp::setup_context(ctx, ThemeKind::Pro);
    let mut app = PhotocraftApp::new(Session::new(), services());
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

/// Import, export and PNG encoding: the in-process codecs, as on the web.
#[cfg(any(target_os = "ios", target_os = "android", test))]
#[cfg_attr(test, expect(dead_code, reason = "only the native shells start the app; tests build the host alone"))]
fn services() -> Services {
    Services {
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
        ..Default::default()
    }
}

#[cfg(any(target_os = "ios", target_os = "android", test))]
#[cfg_attr(test, expect(dead_code, reason = "only the native shells start the app; tests build the host alone"))]
fn options() -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions {
        // The app owns its fallback fonts (`cjk_fonts`), as on the desktop.
        system_font_fallback: false,
        ..Default::default()
    };
    photocraft_ui_egui::gpu_canvas::use_adapter_limits(&mut options.wgpu_options.wgpu_setup);
    options
}

/// The Android entry point, called by `NativeActivity` (through android-activity's glue) on its
/// own thread once the Kotlin shell has loaded this library.
#[cfg(target_os = "android")]
#[expect(unsafe_code, reason = "the symbol android-activity's glue looks up; Rust owns the AndroidApp it is handed")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("photocraft"));
    let mut options = options();
    options.android_app = Some(app);
    let result = eframe::run_native("PhotoCraft", options, Box::new(|cc| Ok(Box::new(create_app(&cc.egui_ctx, cc.wgpu_render_state.clone())))));
    if let Err(e) = result {
        log::error!("photocraft-mobile: {e}");
    }
}
