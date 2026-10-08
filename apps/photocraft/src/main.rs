//! Photocraft desktop app.
//!
//! Usage: `photocraft [--control <port>] [--control-token <64-hex> |
//! --control-token-file <path>] [--automation-read-root <dir>]
//! [--automation-write-root <dir>] [--safe-gpu] [files…]`
//!
//! `--safe-gpu` starts with the CPU renderer (no GPU canvas; a software adapter for the window
//! where the platform has one) for this launch, e.g. after a graphics driver crash. A start that
//! crashes inside the driver also falls back by itself next time (see `gpu_startup`).
//!
//! `--control <port>` (or `PHOTOCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server.
//! The first line must authenticate; subsequent request lines get reply lines.
//! `{"id":1,"ok":true,"result":…}`. See `photocraft_ui_egui::control` for the methods.

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--version` output then only shows when redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app_dirs;
mod app_icon;
#[cfg(target_os = "macos")]
mod apple_events;
mod control_server;
mod crash_guard;
mod cursor;
mod gpu_startup;
#[cfg(target_os = "macos")]
mod mac_window;
// Pure logic is tested on every platform; only Linux runs the check.
#[cfg(any(target_os = "linux", test))]
mod linux_libs;
mod monitor_profile;
mod services;
// Windows gets pen pressure from winit (WM_POINTER); the web runner has its own listener.
#[cfg(any(target_os = "macos", target_os = "linux", test))]
mod tablet;
mod ui_state;

use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;

/// Matches the `.desktop` file and hicolor icon name, so Wayland docks pick up the icon.
const APP_ID: &str = "ai.storyteller.photocraft";

/// The main window: 1440 × 900 (shrunk to fit the monitor, and maximized on the first frame
/// when it still doesn't fit, `work_area::fit_window`), centred on the main monitor. Without
/// `centered`, Windows cascades each new window from the top-left corner, so it opened at a
/// different offset every launch (#419). Wayland compositors place windows themselves.
fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(app_icon::window_icon())
            .with_app_id(APP_ID)
            .with_title("PhotoCraft")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        centered: true,
        // eframe saves native window geometry and egui panel/window sizes on exit.
        // Keep that state beside preferences, including config overrides and portable mode.
        persistence_path: services::config_dir().map(|dir| dir.join("ui.ron")),
        ..Default::default()
    }
}

/// Parse a `--control` / `PHOTOCRAFT_CONTROL_PORT` value. An unparseable port is an error that
/// names the value (issue #701): silently running with no control server leaves a launcher or
/// agent unable to tell a typo from a successful grant.
fn parse_control_port(value: &str, source: &str) -> Result<u16, String> {
    value.trim().parse().map_err(|_| format!("{source}: `{value}` is not a valid port (expected a number from 0 to 65535)"))
}

/// Process exit status for the collected `--control` / `PHOTOCRAFT_CONTROL_PORT` errors: `None`
/// when there are none, else 2 (a command-line usage error), never 0.
fn control_args_exit_code(errors: &[String]) -> Option<i32> {
    if errors.is_empty() { None } else { Some(2) }
}

#[cfg(test)]
mod control_port_tests {
    use super::{control_args_exit_code, parse_control_port};

    #[test]
    fn control_port_errors_exit_non_zero() {
        assert_eq!(control_args_exit_code(&[]), None);
        let errors = vec![parse_control_port("nope", "--control").unwrap_err()];
        assert_eq!(control_args_exit_code(&errors), Some(2));
    }

    #[test]
    fn control_port_accepts_valid_numbers() {
        assert_eq!(parse_control_port("50494", "--control"), Ok(50494));
        assert_eq!(parse_control_port(" 8080 ", "--control"), Ok(8080));
        assert_eq!(parse_control_port("0", "PHOTOCRAFT_CONTROL_PORT"), Ok(0));
    }

    #[test]
    fn control_port_rejects_unparseable_values_naming_them() {
        // Out of range, a following flag eaten as the value, and an empty value all fail,
        // naming the offending value in the message.
        let err = parse_control_port("78787", "--control").unwrap_err();
        assert!(err.contains("78787"), "{err}");
        assert!(parse_control_port("--safe-gpu", "--control").is_err());
        assert!(parse_control_port("", "--control").is_err());
    }
}

fn main() -> eframe::Result {
    crash_guard::install_hook();
    let mut control_port: Option<u16> = None;
    let mut control_arg_errors: Vec<String> = Vec::new();
    if let Some(value) = std::env::var("PHOTOCRAFT_CONTROL_PORT").ok().filter(|v| !v.trim().is_empty()) {
        match parse_control_port(&value, "PHOTOCRAFT_CONTROL_PORT") {
            Ok(port) => control_port = Some(port),
            Err(error) => control_arg_errors.push(error),
        }
    }
    let mut control_token = None;
    let mut control_token_file = None;
    let mut automation_read_root = std::env::var_os("PHOTOCRAFT_AUTOMATION_READ_ROOT").map(std::path::PathBuf::from);
    let mut automation_write_root = std::env::var_os("PHOTOCRAFT_AUTOMATION_WRITE_ROOT").map(std::path::PathBuf::from);
    let mut files = Vec::new();
    let mut safe_gpu = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--control" => match args.next() {
                Some(value) => match parse_control_port(&value, "--control") {
                    Ok(port) => control_port = Some(port),
                    Err(error) => control_arg_errors.push(error),
                },
                None => control_arg_errors.push("--control: missing port value (expected `--control <port>`)".to_string()),
            },
            "--control-token" => control_token = args.next(),
            "--control-token-file" => control_token_file = args.next().map(std::path::PathBuf::from),
            "--automation-read-root" => automation_read_root = args.next().map(std::path::PathBuf::from),
            "--automation-write-root" => automation_write_root = args.next().map(std::path::PathBuf::from),
            "--safe-gpu" => safe_gpu = true,
            "--version" => {
                println!("photocraft {}", photocraft_engine::build_info::long_version());
                return Ok(());
            }
            // Old macOS passes a process serial number when launched from Finder.
            _ if a.starts_with("-psn_") => {}
            _ => files.push(a),
        }
    }

    // A malformed control port must not silently drop the control server (issue #701): name the
    // bad value and fail the launch, like `photocraft-cli serve --port` does for the same typo.
    // Exit status 2 is the usual command-line usage error, so a launcher sees the failure.
    if let Some(code) = control_args_exit_code(&control_arg_errors) {
        for error in &control_arg_errors {
            eprintln!("photocraft: {error}");
        }
        std::process::exit(code);
    }

    // winit and wgpu dlopen the windowing and GPU libraries, and some of those crates panic when
    // one is missing (issue #201). Name the package to install and exit instead.
    #[cfg(target_os = "linux")]
    if let Err(message) = linux_libs::preflight() {
        eprint!("{message}");
        std::process::exit(1);
    }

    let control = if let Some(port) = control_port {
        let (supplied, token_file) = photocraft_automation::security::token_inputs(control_token, control_token_file);
        let token = match photocraft_automation::security::server_token(supplied.as_deref(), token_file.as_deref()) {
            Ok(token) => token,
            Err(e) => {
                eprintln!("photocraft: cannot configure control authentication: {e}");
                return Ok(());
            }
        };
        if let Some(path) = token_file {
            eprintln!("photocraft: control token file: {}", path.display());
        } else if supplied.is_none() {
            eprintln!("photocraft: control token: {token}");
        } else {
            eprintln!("photocraft: using supplied control token");
        }
        let workspace = match photocraft_automation::AuthorizedWorkspace::new(automation_read_root.as_deref(), automation_write_root.as_deref()) {
            Ok(workspace) => workspace,
            Err(error) => {
                eprintln!("photocraft: cannot configure automation workspace: {error}");
                return Ok(());
            }
        };
        Some((port, token, workspace))
    } else {
        None
    };
    // Finder / Dock / Open With deliver files as Apple events, not arguments; catch the one that
    // launched us as well as later ones. Lives until the event loop returns.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;

    // Pen tablet samples on macOS (AppKit event monitor, before winit sees each event) and X11
    // (started once eframe says which display server it is on). The monitor lives until the event
    // loop returns.
    let stylus_feed = photocraft_ui_egui::stylus::StylusFeed::default();
    #[cfg(target_os = "macos")]
    let _tablet = tablet::install_macos(&stylus_feed);

    // Read the displays' ICC profiles while the window opens (colour-managed canvas; `None`
    // where the platform has no reader).
    let monitor = monitor_profile::detect_async();
    // Brush presets load in the background; the app attaches them when they arrive.
    let presets = services::presets_dir().map(photocraft_engine::preset_store::open_dir_async);
    let mut options = native_options();
    // eframe restores the saved window layout before our code runs; drop values that would crash it.
    ui_state::sanitize(options.persistence_path.as_deref());
    // Crash-safe GPU startup (#4): pick the backend (a marker left by a start that died in the
    // driver moves to a safer one), and lock this start's marker until the first frames render.
    let t_sentinel = std::time::Instant::now();
    let os = gpu_startup::Os::current();
    let (pref, mode) = gpu_startup::read_rendering_prefs(services::prefs_file().as_deref());
    let (previous, sentinel) = match services::config_dir() {
        Some(dir) => gpu_startup::Sentinel::begin(&dir),
        None => (gpu_startup::Previous::Clean, None),
    };
    let env_backend = std::env::var("WGPU_BACKEND").ok();
    let plan = gpu_startup::plan_with_mode(pref, mode, previous.crashed(), env_backend.as_deref(), safe_gpu, os);
    if let Some(m) = previous.crashed() {
        log::warn!("the previous start didn't finish (GPU backend {}, adapter {:?}); {}", m.backend, m.adapter, plan.reason.as_deref().unwrap_or(""));
    }
    let sentinel: gpu_startup::SharedSentinel = std::sync::Arc::new(std::sync::Mutex::new(sentinel));
    if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
        let backend = match &plan.env {
            Some(v) => format!("env:{v}"),
            None => plan.backend.name().to_string(),
        };
        let marker = gpu_startup::Marker { backend, version: photocraft_engine::build_info::long_version().to_string(), ..Default::default() };
        if let Err(e) = s.write(marker) {
            log::warn!("GPU startup marker: {e}");
        }
    }
    let gpu_note: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    // The adapter's real texture limits (egui asks for 8192 px), so big documents stay on the GPU.
    gpu_startup::configure(&mut options.wgpu_options.wgpu_setup, &plan, os, sentinel.clone(), gpu_note.clone());
    let sentinel_ms = t_sentinel.elapsed().as_secs_f64() * 1000.0;
    log::info!("GPU startup: {:?} ({sentinel_ms:.2} ms)", plan);
    let retry_cpu = !safe_gpu && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu;
    let app_created = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let created_in_callback = app_created.clone();
    let started_sentinel = sentinel.clone();
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            created_in_callback.store(true, std::sync::atomic::Ordering::Relaxed);
            let automation = control.as_ref().map(|(_, _, workspace)| workspace.clone());
            let mut services = services::native(automation);
            services.preset_store = presets;
            #[cfg(target_os = "linux")]
            let display = tablet::DisplayKind::of(cc);
            #[cfg(target_os = "linux")]
            {
                services.is_wayland = display == Some(tablet::DisplayKind::Wayland);
            }
            let mut app = PhotocraftApp::new(Session::new(), services);
            app.integrated_titlebar = cfg!(target_os = "macos");
            // Only the title bar's free gap drags the window, never the menus (mac_window.rs).
            #[cfg(target_os = "macos")]
            mac_window::disable_native_title_drag();
            // Long commands and file opens run as background jobs with progress and Cancel (#210).
            app.background_jobs = std::env::var_os("PHOTOCRAFT_INLINE_JOBS").is_none();
            // Displays and their profiles (#569): wait briefly so the first frames already use
            // the right profile; a slower reading is applied when it arrives, and the shell reads
            // again when the displays may have changed.
            if let Some(rx) = monitor {
                app.services.read_displays = Some(std::sync::Arc::new(monitor_profile::detect_async));
                match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(r) => photocraft_ui_egui::monitor_status::apply(&mut app, r),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => photocraft_ui_egui::monitor_status::pending(&mut app, rx),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        photocraft_ui_egui::monitor_status::apply(&mut app, Err("the display profile reader stopped without an answer".into()));
                    }
                }
            }
            // Preferences › Performance › Use Graphics Processor (and the GPU backend: `cpu`
            // composites on the CPU).
            let info = &mut app.perf.gpu_info;
            info.preference = pref.name().to_string();
            info.selected = if plan.env.is_some() { "env".into() } else { plan.backend.name().to_string() };
            info.fallback = match (plan.reason.clone(), gpu_note.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()) {
                (Some(reason), Some(note)) => Some(format!("{reason}. {note}")),
                (reason, note) => reason.or(note),
            };
            info.canvas = "cpu".into();
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
                let software_window = rs.adapter.get_info().device_type == eframe::wgpu::DeviceType::Cpu;
                if software_window && mode != photocraft_engine::prefs::RenderingMode::Cpu {
                    app.perf.gpu_info.fallback = Some("No compatible hardware graphics adapter; using software graphics.".into());
                }
                if !software_window
                    && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu
                    && std::env::var_os("PHOTOCRAFT_CPU_CANVAS").is_none()
                    && app.session.prefs().performance.effective_rendering_mode() != photocraft_engine::prefs::RenderingMode::Cpu
                {
                    app.set_wgpu(rs);
                } else {
                    // The window still draws with wgpu: record its errors instead of panicking.
                    let _ = photocraft_ui_egui::gpu_canvas::DeviceHealth::watch(&rs.device);
                }
            }
            let fallback_reason = std::env::var("PHOTOCRAFT_GPU_STARTUP_FAILURE").ok().or_else(|| {
                (app.perf.gpu_info.canvas == "cpu" && mode != photocraft_engine::prefs::RenderingMode::Cpu)
                    .then(|| app.perf.gpu_info.fallback.clone())
                    .flatten()
            });
            if let Some(reason) = fallback_reason {
                photocraft_ui_egui::gpu_status::queue_fallback_notice(&mut app, &reason);
            }
            app.perf.span("gpuSentinel", sentinel_ms);
            // Once the first frames rendered: clear the marker, and keep a crash fallback.
            let remember_cpu =
                std::env::var_os("PHOTOCRAFT_GPU_STARTUP_FAILURE").is_some() || (plan.remember && plan.backend == photocraft_engine::prefs::GpuBackend::Cpu);
            let remember = plan.remember.then_some(plan.backend);
            app.on_started(move |app| {
                if let Some(s) = started_sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                    s.finish();
                }
                if remember_cpu
                    && let Err(error) = app.run(
                        "prefs.set",
                        serde_json::json!({"values": {
                            "performance.renderingMode": "cpu", "performance.useGpu": false
                        }}),
                    )
                {
                    log::warn!("couldn't remember CPU recovery: {error}");
                }
                if let Some(b) = remember
                    && app.session.prefs().performance.gpu_backend != b
                    && let Err(e) = app.run("prefs.set", serde_json::json!({"path": "performance.gpuBackend", "value": b.name()}))
                {
                    log::warn!("couldn't remember GPU backend {}: {e}", b.name());
                }
            });
            if let Some((port, token, _)) = control {
                let rx = control_server::start(port, token, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            #[cfg(target_os = "macos")]
            {
                app.services.os_events = Some(apple_events.connect(&cc.egui_ctx));
            }
            // Where file drags and drops are (winit 0.30 doesn't say).
            app.services.cursor_pos = cursor::service(cc);
            // Tablet pressure/tilt/eraser (winit drops them): the macOS monitor installed above
            // and the X11 reader write into this feed.
            app.stylus.feed = stylus_feed;
            #[cfg(target_os = "linux")]
            tablet::spawn_x11(&app.stylus.feed, display);
            // Paths on the command line (Linux/Windows file associations, `photocraft a.psd`).
            app.open_paths(&files);
            // Portable marker found but its data folder isn't writable (#228): say where settings went.
            if let Some(w) = &app_dirs::current().warning {
                photocraft_ui_egui::notices::post(&mut app, "Portable mode is off", vec![w.clone()], false, None);
            }
            Ok(Box::new(app))
        }),
    );
    // Closed before the first frames rendered: not a driver crash. (A start that failed to
    // create its device keeps the marker, so the next one tries a safer backend.)
    if result.is_ok()
        && let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    {
        s.finish();
    }
    // Retry in a fresh process: winit event loops cannot be recreated reliably in-process.
    // Only renderer initialization failures qualify; never restart after editing has begun.
    if retry_cpu && !app_created.load(std::sync::atomic::Ordering::Relaxed) && matches!(&result, Err(eframe::Error::Wgpu(_))) {
        let reason = result.as_ref().err().map(ToString::to_string).unwrap_or_default();
        if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            s.finish();
        }
        if let Ok(exe) = std::env::current_exe() {
            let launched = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .arg("--safe-gpu")
                .env_remove("WGPU_BACKEND")
                .env("PHOTOCRAFT_GPU_STARTUP_FAILURE", &reason)
                .spawn();
            match launched {
                Ok(_) => return Ok(()),
                Err(error) => log::error!("could not start CPU compatibility mode: {error}"),
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_and_panel_geometry_survive_a_restart() {
        let options = super::native_options();
        assert!(options.persist_window);
        assert_eq!(options.persistence_path, super::services::config_dir().map(|dir| dir.join("ui.ron")));

        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let ctx = egui::Context::default();
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input(), |ui| {
            egui::Panel::right("dock").exact_size(410.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let mut storage = Storage::default();
        ctx.memory(|memory| eframe::set_value(&mut storage, "egui", memory));
        let restored = egui::Context::default();
        restored.memory_mut(|memory| *memory = eframe::get_value(&storage, "egui").unwrap());
        let mut output = restored.run_ui(input(), |ui| {
            egui::Panel::right("dock").default_size(290.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let panel = egui::containers::panel::PanelState::load(&restored, egui::Id::new("dock")).unwrap();
        assert_eq!(panel.size().x, 410.0);
    }

    #[test]
    fn the_window_opens_centred_at_its_default_size() {
        let o = super::native_options();
        assert!(o.centered, "#419: centred, not cascaded from the top-left corner");
        assert_eq!(o.viewport.inner_size, Some(egui::vec2(1440.0, 900.0)));
        // eframe shrinks the start size to the monitor, so the centred position is on-screen.
        assert_ne!(o.viewport.clamp_size_to_monitor_size, Some(false));
    }
}
