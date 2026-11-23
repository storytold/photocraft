//! PhotoCraft desktop app.
//!
//! Usage: `photocraft [options] [--] [files…]`; `photocraft --help` lists the options, which are
//! declared, with what makes a usage error (exit status 2), in `launch_args`.
//!
//! On Windows and Linux a launch hands its files to the PhotoCraft that is already running (new
//! tabs in its window) and exits; `--new-instance` starts a separate copy instead, as does any
//! `--control` launch (see `single_instance`).
//!
//! `--present-mode` (`fifo`, `autovsync`, `novsync`) and `--frame-latency` (1–3) override how
//! frames reach the display for this launch (`PHOTOCRAFT_PRESENT_MODE` / `…_FRAME_LATENCY`;
//! `presentation`); `--stroke-trace` times each brush stroke's frames (`PHOTOCRAFT_STROKE_TRACE`;
//! `photocraft_ui_egui::stroke_trace`).
//!
//! `--safe-gpu` starts with the CPU renderer (no GPU canvas; a software adapter for the window
//! where the platform has one) for this launch, e.g. after a graphics driver crash. A start that
//! crashes inside the driver also falls back by itself next time (see `gpu_startup`).
//!
//! `--in-window-menus` (or `PHOTOCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar (`photocraft_ui_egui::native_menu`).
//!
//! `--control <port>` (or `PHOTOCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server.
//! The first line must authenticate; subsequent request lines get reply lines.
//! `{"id":1,"ok":true,"result":…}`. See `photocraft_ui_egui::control` for the methods.

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--help`, `--version` and usage errors then only show when
// redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app_dirs;
mod app_icon;
#[cfg(target_os = "macos")]
mod apple_events;
mod caps_lock;
mod crash_guard;
mod cursor;
mod gpu_startup;
mod launch_args;
#[cfg(target_os = "macos")]
mod mac_window;
// Pure logic is tested on every platform; only Linux runs the check.
#[cfg(not(target_arch = "wasm32"))]
mod external_svg;
#[cfg(any(target_os = "linux", test))]
mod linux_libs;
mod logging;
mod monitor_profile;
mod photoshop_settings;
mod presentation;
// The print portal needs the session bus (zbus, Linux only); its pure helpers are tested everywhere.
#[cfg(any(target_os = "linux", test))]
mod print_portal;
mod services;
// macOS gets one instance from LaunchServices (open-documents Apple events).
#[cfg(any(not(target_os = "macos"), test))]
mod single_instance;
mod startup_report;
// Tablet support (macOS AppKit monitor, Linux X11/Wayland, Windows barrel button).
mod tablet;
mod ui_state;
mod window_frame;

use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::Prefix;
use photocraft_ui_egui::window_policy::{self, Backend, Frame, TitleBarPref};

use photocraft_ui_egui::os_window::APP_ID;

/// The smallest the main window may be, in points.
const MIN_WINDOW_SIZE: [f32; 2] = [760.0, 480.0];

/// Which title bar Preferences › Interface › System Title Bar asks for (#1271, #1316), from the
/// preferences read before the window opens (`services::saved_prefs`). The frame decision
/// (`window_policy::decide_for`) turns it into the frame the platform allows.
fn title_bar_pref(prefs: &photocraft_engine::prefs::Preferences) -> TitleBarPref {
    if prefs.interface.system_title_bar { TitleBarPref::System } else { TitleBarPref::Integrated }
}

/// The automation roots: `--automation-read-root` / `--automation-write-root` (or their
/// variables) win; a direction without one takes Preferences › AI Integrations' folder, and with
/// that empty too the default agent folder `Documents/ArtCraft/Agents/PhotoCraft`, created when
/// first used (`AgentSettings::folders`). A flag's
/// root that can't be opened stops the launch (exit status 2) as before. A preference's root that
/// can't be opened, or isn't an absolute path (it would depend on the folder the app was started
/// from), is left out, so that direction fails closed, and the warning says why: a folder moved
/// since it was chosen must not stop the app from starting. Each folder is opened once, so one
/// that goes away during the launch can't turn into an error, and the same folder for reading
/// and writing shares one capability, as it does when given by flags.
fn automation_workspace(
    flags: &photocraft_automation::args::AutomationArgs,
    prefs: &craft::automation::agents::AgentSettings,
) -> Result<(photocraft_automation::workspace::AuthorizedWorkspace, Option<String>), photocraft_automation::AutomationError> {
    automation_workspace_in(flags, prefs, craft::control::agents::default_folder(AGENT_APP_NAME).as_deref())
}

/// The app's name in its default agent folder, `Documents/ArtCraft/Agents/PhotoCraft`.
const AGENT_APP_NAME: &str = "PhotoCraft";

/// [`automation_workspace`] with `default` as the default agent folder (tests use a temporary one).
fn automation_workspace_in(
    flags: &photocraft_automation::args::AutomationArgs,
    prefs: &craft::automation::agents::AgentSettings,
    default: Option<&std::path::Path>,
) -> Result<(photocraft_automation::workspace::AuthorizedWorkspace, Option<String>), photocraft_automation::AutomationError> {
    let mut workspace = flags.workspace()?;
    let mut warnings = Vec::new();
    let (read_given, write_given) = (!flags.automation_read_root.is_empty(), flags.automation_write_root.is_some());
    // The default folder is created only when a direction no flag names has no folder of the
    // person's own; otherwise it would be made and never used.
    let needs_default = (!read_given && prefs.read_folder().is_none()) || (!write_given && prefs.write_folder().is_none());
    let folders = prefs.folders_with(default.filter(|_| needs_default));
    for read in [true, false] {
        let given = if read { read_given } else { write_given };
        let Some(root) = (if read { folders.read.clone() } else { folders.write.clone() }).filter(|_| !given) else {
            continue;
        };
        let own = if read { prefs.read_folder() } else { prefs.write_folder() }.is_some();
        let opened = if !root.is_absolute() {
            Err(photocraft_automation::AutomationError::BadRequest(format!("`{}` is not an absolute path", root.display())))
        } else if read {
            workspace.with_read_root(&root)
        } else {
            workspace.with_write_root(&root)
        };
        match opened {
            Ok(w) => workspace = w,
            Err(e) if own => {
                let which = if read { "Automation Read Folder" } else { "Automation Write Folder" };
                warnings.push(format!("Preferences › AI Integrations › {which} is not used: {e}"));
            }
            Err(e) => warnings.push(format!("The default agent folder {} is not used: {e}", root.display())),
        }
    }
    Ok((workspace, (!warnings.is_empty()).then(|| warnings.join("\n"))))
}

/// The control port this launch listens on, and the warning to print when a request was refused.
/// Only `--control` (or `PHOTOCRAFT_CONTROL_PORT`) opens one at launch: the agent link starts and
/// stops its own while the app runs (`craft::automation::agents::AgentLink`). Allow Agent Control
/// off refuses the flag too, and says so rather than ignoring it silently.
fn control_port_for(cli: Option<u16>, prefs: &photocraft_engine::prefs::Integrations) -> (Option<u16>, Option<String>) {
    if !prefs.agents.allow_agent_control {
        let warning = cli.map(|p| {
            format!(
                "photocraft: Preferences › AI Integrations › Allow Agent Control is off: not listening on port {p}
"
            )
        });
        return (None, warning);
    }
    (cli, None)
}

/// The main window: 1440 × 900 (shrunk to fit the monitor, and maximized on the first frame
/// when it still doesn't fit, `embed::before_ui`), centred on the main monitor. Without
/// `centered`, Windows cascades each new window from the top-left corner, so it opened at a
/// different offset every launch (#419). A later launch restores the saved position instead
/// (`main` turns `centered` off, #4826). Wayland compositors place windows themselves.
fn native_options(frame: &Frame) -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default().with_icon(app_icon::viewport_icon(std::env::current_exe().ok().as_deref()));
    let viewport =
        viewport.with_app_id(APP_ID).with_title("PhotoCraft").with_inner_size([1440.0, 900.0]).with_min_inner_size(MIN_WINDOW_SIZE).with_drag_and_drop(true);
    eframe::NativeOptions {
        viewport: window_policy::apply(viewport, frame),
        centered: true,
        // PhotoCraft owns its fallback fonts (`cjk_fonts`, craft-fonts): eframe's system-font
        // provider would compete with it and make glyph choice machine-dependent (#8512).
        system_font_fallback: false,
        // eframe saves native window geometry and egui panel/window sizes on exit.
        // Keep that state beside preferences, including config overrides and portable mode.
        persistence_path: services::config_dir().map(|dir| dir.join("ui.ron")),
        window_builder: Some(Box::new(restored_window)),
        ..Default::default()
    }
}

/// The window eframe restores from `ui.ron`, corrected before it is created: at least the
/// minimum size, and never full screen. PhotoCraft always starts in Standard Screen Mode, but
/// eframe also saves and restores full screen, and a window closed in a full-screen mode is
/// saved full screen and maximized at once: the overlap the window's screen-mode sequence
/// (`AppShell`'s `ScreenModeFollow`) keeps apart at run time. [OBSERVED 2026-10-12, Windows 11] Restored that way, View › Screen Mode ›
/// Full Screen did nothing until `F` had cycled through Standard.
fn restored_window(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    at_least_min_size(builder).with_fullscreen(false)
}

/// Grow a restored window that is smaller than the minimum size, keeping its centre. On
/// Windows, eframe saves a minimized window's size (160 pt wide, 64 pt tall once eframe has
/// clamped it), and the minimum size isn't applied to a restored one: PhotoCraft then reopened
/// as a 160 × 64 strip on every launch.
fn at_least_min_size(mut builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let (Some(size), Some(min)) = (builder.inner_size, builder.min_inner_size) else { return builder };
    let grown = size.max(min);
    if grown != size {
        builder.position = builder.position.map(|pos| pos - (grown - size) / 2.0);
        builder.inner_size = Some(grown);
    }
    builder
}

/// Attach the log file under the settings directory. Called after the arguments are parsed, so
/// `--version` and usage errors leave no file behind. Records logged until now are written first.
fn attach_log_file(logger: Option<&'static logging::AppLogger>) {
    if let (Some(logger), Some(dir)) = (logger, services::config_dir()) {
        match logger.attach_dir(&dir.join("logs")) {
            Ok(path) => log::info!("PhotoCraft {}, log file {}", photocraft_engine::build_info::long_version(), path.display()),
            // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
            Err(e) => log::warn!("no log file: {e}"),
        }
    }
}

/// One PhotoCraft per user (#3708). Only with the per-user settings folder (a portable copy's may
/// be shared between users), and not for automation, `--new-instance` or per-process options.
/// `Break` when the files were handed to the running PhotoCraft and this launch is done.
#[cfg(not(target_os = "macos"))]
fn claim_single_instance(
    files: &[String],
    new_instance: bool,
    control: bool,
    unreadable_paths: bool,
    mut launch_options: Vec<launch_args::PerProcess>,
) -> std::ops::ControlFlow<(), Option<single_instance::Primary>> {
    use std::ops::ControlFlow::{Break, Continue};
    let new_instance_env = std::env::var("PHOTOCRAFT_NEW_INSTANCE").ok();
    // A variable documented as the equivalent of a per-launch flag counts as that flag: a
    // launch that sets only `PHOTOCRAFT_PRESENT_MODE` (say) has per-process options too, so it
    // keeps its own process instead of handing its files over and being ignored.
    launch_options.extend_from_slice(&single_instance::env_launch_options());
    let context = single_instance::LaunchContext {
        per_user_settings: app_dirs::current().mode == app_dirs::Mode::Platform,
        new_instance_flag: new_instance,
        new_instance_env: new_instance_env.as_deref(),
        control,
        unreadable_paths,
        launch_options: &launch_options,
    };
    match (single_instance::skip_reason(&context), services::config_dir()) {
        (Some(reason), _) => {
            log::info!("running as a separate PhotoCraft: {reason}");
            Continue(None)
        }
        (None, None) => Continue(None),
        (None, Some(dir)) => {
            let paths: Vec<String> = files.iter().map(|f| single_instance::absolute(f)).collect();
            match single_instance::claim(&dir, &paths) {
                single_instance::Launch::Forwarded => {
                    log::info!("handed {} file(s) to the running PhotoCraft", paths.len());
                    Break(())
                }
                single_instance::Launch::Primary(primary) => Continue(Some(primary)),
                single_instance::Launch::Standalone(reason) => {
                    log::warn!("starting a separate PhotoCraft: {reason}");
                    Continue(None)
                }
            }
        }
    }
}

/// The display server to open the window on: Xwayland for a pen on Wayland, which gives this app
/// no pen input (#639). Exits with the package to install when its libraries are missing.
#[cfg(target_os = "linux")]
fn linux_display_session(display_server: photocraft_engine::prefs::LinuxDisplayServer) -> linux_libs::DisplaySession {
    let session = tablet::display_session();
    // Preferences › Performance › Linux display server = X11: Xwayland on a Wayland session too,
    // where native file drops work (winit 0.30 has none on Wayland, #386). Only when Xwayland can
    // run the window ($DISPLAY set, X11 libraries installed); otherwise the session's own.
    let session = if session == linux_libs::DisplaySession::Wayland
        && display_server == photocraft_engine::prefs::LinuxDisplayServer::X11
        && std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty())
        && linux_libs::available(linux_libs::DisplaySession::X11)
    {
        eprintln!("photocraft: opening the window through Xwayland (Preferences › Performance › Linux display server)");
        linux_libs::DisplaySession::X11
    } else {
        session
    };
    // winit and wgpu dlopen the windowing and GPU libraries, and some of those crates panic when
    // one is missing (issue #201). Name the package to install and exit instead.
    if let Err(message) = linux_libs::preflight(session) {
        eprint!("{message}");
        std::process::exit(1);
    }
    session
}

/// The control server: its listener, token and workspace.
type Control = (std::net::TcpListener, String, photocraft_automation::workspace::AuthorizedWorkspace);

/// What [`control_setup`] set up: the control server when it is on, and a notice for the window
/// (a preference's automation folder that wasn't used).
struct ControlSetup {
    control: Option<Control>,
    warning: Option<ControlNotice>,
}

/// A notice for the window about the control server: a Dock launch has no console.
struct ControlNotice {
    title: &'static str,
    lines: Vec<String>,
}

/// The `--control` server's port, token and workspace, when it is on (exits 2 on a bad
/// configuration). Preferences › AI Integrations (`prefs`, read before the window opens) decide
/// whether agents may drive this launch, and its folders stand in for automation roots no flag
/// names. The server the agents switch starts is the agent link's, not this.
fn control_setup(
    prefs: &photocraft_engine::prefs::Integrations,
    cli_port: Option<u16>,
    auth: photocraft_automation::args::ControlArgs,
    automation: &photocraft_automation::args::AutomationArgs,
) -> ControlSetup {
    control_setup_in(services::config_dir(), prefs, cli_port, auth, automation)
}

/// [`control_setup`] with the settings folder and Preferences › AI Integrations given.
fn control_setup_in(
    settings: Option<std::path::PathBuf>,
    prefs: &photocraft_engine::prefs::Integrations,
    cli_port: Option<u16>,
    auth: photocraft_automation::args::ControlArgs,
    automation: &photocraft_automation::args::AutomationArgs,
) -> ControlSetup {
    let (port, warning) = control_port_for(cli_port, prefs);
    if let Some(warning) = &warning {
        eprint!("{warning}");
    }
    // A file a crashed (or, on macOS, quit) launch left behind names a port nothing serves. Before
    // binding, so this launch's own listener doesn't keep it.
    if let Some(dir) = &settings {
        craft::automation::agents::remove_stale_connection(dir);
    }
    let Some(port) = port else {
        return ControlSetup { control: None, warning: None };
    };
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("photocraft: control server failed to bind 127.0.0.1:{port}: {error}");
            std::process::exit(2);
        }
    };
    let (supplied, token_file) = (auth.control_token, auth.control_token_file);
    let token = match photocraft_automation::security::server_token(supplied.as_deref(), token_file.as_deref()) {
        Ok(token) => token,
        Err(e) => {
            eprintln!("photocraft: cannot configure control authentication: {e}");
            std::process::exit(2);
        }
    };
    if let Some(path) = &token_file {
        eprintln!("photocraft: control token file: {}", path.display());
    } else if supplied.is_none() {
        eprintln!("photocraft: control token: {token}");
    } else {
        eprintln!("photocraft: using supplied control token");
    }
    let (workspace, warning) = match automation_workspace(automation, &prefs.agents) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("photocraft: cannot configure automation workspace: {error}");
            std::process::exit(2);
        }
    };
    if let Some(warning) = &warning {
        eprintln!("photocraft: {warning}");
        log::warn!("{warning}");
    }
    let warning = warning.map(|w| ControlNotice { title: "Automation folder not used", lines: w.lines().map(str::to_owned).collect() });
    ControlSetup { control: Some((listener, token, workspace)), warning }
}

/// Record this start's backend in the GPU startup marker, if the marker is locked.
fn write_start_marker(sentinel: &gpu_startup::SharedSentinel, plan: &gpu_startup::Plan) {
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
}

fn main() -> eframe::Result {
    // Before anything starts a thread: may re-execute this process with glibc tuned to return
    // the memory of closed documents (#8743).
    photocraft_engine::malloc_tuning::apply();
    // Then, so the panic hook and every start-up warning are recorded (`logging`).
    let logger = logging::install();
    crash_guard::install_hook();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(done) = craft_update::self_update::finish(&photocraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        if let Err(e) = done {
            eprintln!("PhotoCraft update helper failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // The system font scan can take seconds (minutes on some Macs): run it off the UI thread.
    photocraft_ui_egui::type_tool::warm_fonts();
    // `--help` and `--version` print to standard output and exit 0. A usage error (an unknown
    // option, a missing or bad value, a bad PHOTOCRAFT_CONTROL_PORT: issues #701, #6429, #6431)
    // names the argument on standard error and exits 2, so a launcher sees the failure. Neither
    // panics when the stream is closed or a pipe (`photocraft --help | head -1`).
    let launch = launch_args::parse(launch_args::command(), std::env::args_os().skip(1)).unwrap_or_else(|e| e.exit());
    let launch_args::FileNames { openable: files, unreadable: unreadable_paths } = launch.file_names();
    let launch_args::LaunchArgs {
        files: _,
        safe_gpu,
        in_window_menus,
        new_instance,
        present_mode,
        frame_latency,
        stroke_trace,
        control: control_port,
        auth,
        automation,
        mut launch_options,
    } = launch;

    attach_log_file(logger);

    // The saved preferences the launch needs before the window opens, read once without opening
    // the settings store (the app opens it later).
    let saved = services::saved_prefs();
    let prefs = &saved.prefs;
    // An adapter named in the environment is chosen at startup, which a forwarded launch never reaches.
    let rendering_mode = gpu_startup::rendering_prefs(&prefs.performance).1;
    if gpu_startup::adapter_override_is_process_specific(std::env::var("PHOTOCRAFT_GPU_ADAPTER").ok().as_deref(), rendering_mode, gpu_startup::ADAPTER_CHOICE) {
        launch_options.push(launch_args::PerProcess::GpuAdapterEnv);
    }
    // One PhotoCraft per user (#3708): files opened from Explorer or a file manager while it runs
    // become tabs in its window. Lives until the event loop returns, then `shutdown` lets the next
    // launch become the primary.
    #[cfg(not(target_os = "macos"))]
    let single_instance = match claim_single_instance(&files, new_instance, control_port.is_some(), !unreadable_paths.is_empty(), launch_options) {
        std::ops::ControlFlow::Break(()) => return Ok(()),
        std::ops::ControlFlow::Continue(primary) => primary,
    };
    #[cfg(not(target_os = "macos"))]
    let single_instance = &single_instance;
    #[cfg(target_os = "macos")]
    let _ = (new_instance, &launch_options);

    // Bind and authenticate the authorized control listener before display/GPU initialization.
    let ControlSetup { control, warning: control_warning } = control_setup(&prefs.integrations, control_port, auth, &automation);

    #[cfg(target_os = "linux")]
    let native_session = linux_libs::session_from_env(|name| std::env::var(name).ok());
    #[cfg(target_os = "linux")]
    let session = linux_display_session(prefs.performance.linux_display_server);

    // Finder / Dock / Open With deliver files as Apple events, not arguments; catch the one that
    // launched us as well as later ones. Lives until the event loop returns.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;

    // The macOS pen tablet monitor (installed once eframe created the app, see below); it lives
    // until the event loop returns.
    #[cfg(target_os = "macos")]
    let tablet_monitor = std::cell::OnceCell::new();
    #[cfg(target_os = "macos")]
    let tablet_monitor = &tablet_monitor;
    // The Windows pointer message hook; dropping it unhooks when the event loop returns.
    #[cfg(target_os = "windows")]
    let tablet_monitor_win = std::cell::OnceCell::new();
    #[cfg(target_os = "windows")]
    let tablet_monitor_win = &tablet_monitor_win;

    // Read the displays' ICC profiles while the window opens (colour-managed canvas; `None`
    // where the platform has no reader).
    #[cfg(target_os = "linux")]
    let monitor = monitor_profile::detect_async_for_session(native_session);
    #[cfg(not(target_os = "linux"))]
    let monitor = monitor_profile::detect_async();
    // Brush presets load in the background; the app attaches them when they arrive.
    let presets = services::presets_dir().map(photocraft_engine::preset_store::open_dir_async);
    // Who draws the window frame, decided once: the viewport below and the runtime's root frame both follow it.
    // `PHOTOCRAFT_TITLEBAR`, `PHOTOCRAFT_CAPTION_STYLE` and `PHOTOCRAFT_CAPTION_LAYOUT` are read by the decision.
    let decision = window_policy::decide_for(Prefix::new("PHOTOCRAFT"), Backend::from_env(), title_bar_pref(prefs));
    let integrated_frame = matches!(decision.frame, Frame::Integrated(_));
    let mut options = native_options(&decision.frame);
    // winit picks Wayland whenever `WAYLAND_DISPLAY` is set; open on X11 when that was chosen.
    #[cfg(target_os = "linux")]
    if session == linux_libs::DisplaySession::X11 {
        use winit::platform::x11::EventLoopBuilderExtX11 as _;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_x11();
        }));
    }
    // eframe restores the saved window layout before our code runs; drop values that would crash it.
    ui_state::sanitize(options.persistence_path.as_deref());
    // Centre only the first window: eframe centres after restoring the saved position, so a
    // window closed on another monitor reopened in the middle of the main one (#4826).
    options.centered = !ui_state::has_saved_position(options.persistence_path.as_deref());
    // Crash-safe GPU startup (#4): pick the backend (a marker left by a start that died in the
    // driver moves to a safer one), and lock this start's marker until the first frames render.
    let t_sentinel = std::time::Instant::now();
    let os = gpu_startup::Os::current();
    let (pref, mode) = gpu_startup::rendering_prefs(&prefs.performance);
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
    write_start_marker(&sentinel, &plan);
    let gpu_note: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    // The adapter's real texture limits (egui asks for 8192 px), so big documents stay on the GPU.
    let wanted_adapter = gpu_startup::wanted_adapter(std::env::var("PHOTOCRAFT_GPU_ADAPTER").ok().as_deref(), &gpu_startup::gpu_adapter(&prefs.performance));
    gpu_startup::configure(&mut options.wgpu_options.wgpu_setup, &plan, os, wanted_adapter, sentinel.clone(), gpu_note.clone());
    // Preferences › Performance › Low-Latency Drawing (#1343): without vsync a stroke shows about
    // 50 ms sooner; wgpu picks Immediate, else Mailbox, else Fifo by what the driver offers.
    if prefs.performance.low_latency_drawing {
        options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    }
    // Testers' present mode and frame latency overrides (#1343), after the preference.
    let present = presentation::Overrides::from_args_or_env(present_mode, frame_latency);
    options.wgpu_options.surface = present.apply(options.wgpu_options.surface);
    let present_desc = presentation::describe(&options.wgpu_options.surface, &present);
    // Without vsync, frames are capped at 240 fps (#4472) so repaint loops don't spin flat out.
    let uncapped = !matches!(
        options.wgpu_options.surface.present_mode,
        eframe::wgpu::PresentMode::Fifo | eframe::wgpu::PresentMode::FifoRelaxed | eframe::wgpu::PresentMode::AutoVsync
    );
    log::info!("presentation: {present_desc}");
    let sentinel_ms = t_sentinel.elapsed().as_secs_f64() * 1000.0;
    log::info!("GPU startup: {:?} ({sentinel_ms:.2} ms)", plan);
    let retry_cpu = !safe_gpu && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu;
    let keep_marker = gpu_startup::keep_marker_after_error(&plan, os);
    // For the failure dialog below: the plan itself moves into the window closure. With
    // `WGPU_BACKEND` set (`plan.env`) wgpu picks the backend, so name that rather than `auto`.
    let started_renderer = plan.env.clone().unwrap_or_else(|| plan.backend.name().to_string());
    let started_note = plan.reason.clone();
    let app_created = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let created_in_callback = app_created.clone();
    let started_sentinel = sentinel.clone();
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            created_in_callback.store(true, std::sync::atomic::Ordering::Relaxed);
            let automation = control.as_ref().map(|(_, _, workspace)| workspace.clone());
            let wake = {
                let ctx = cc.egui_ctx.clone();
                craft::host::Wake::new(move || ctx.request_repaint())
            };
            let mut services = services::native(automation, wake);
            services.preset_store = presets;
            #[cfg(target_os = "linux")]
            let display = tablet::DisplayKind::of(cc);
            #[cfg(target_os = "linux")]
            {
                services.is_wayland = display == Some(tablet::DisplayKind::Wayland);
                if services.is_wayland {
                    let var = |name| std::env::var(name).ok();
                    services.xwayland_command =
                        services::xwayland_command(var("FLATPAK_ID").as_deref(), var("APPIMAGE").as_deref(), std::env::var_os("DISPLAY").is_some());
                }
            }
            #[cfg(target_os = "linux")]
            let session = {
                let mut session = Session::new();
                session.print_spooler = print_portal::spooler();
                session
            };
            #[cfg(not(target_os = "linux"))]
            let session = Session::new();
            // Before the first pass: the one install of the craft runtime must see every pass from
            // the first (R71); the app then runs on that context.
            PhotocraftApp::setup_window(&cc.egui_ctx, Default::default(), decision);
            let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, services);
            app.open_suite(&cc.egui_ctx);
            app.min_frame_ms = uncapped.then_some(1000.0 / 240.0);
            // Only the title bar's free gap drags the window, never the menus (mac_window.rs).
            #[cfg(target_os = "macos")]
            mac_window::disable_native_title_drag();
            // Long commands and file opens run as background jobs with progress and Cancel (#210).
            app.background_jobs = std::env::var_os("PHOTOCRAFT_INLINE_JOBS").is_none();
            // Displays and their profiles (#569): wait briefly so the first frames already use
            // the right profile; a slower reading is applied when it arrives, and the shell reads
            // again when the displays may have changed.
            if let Some(rx) = monitor {
                #[cfg(target_os = "linux")]
                {
                    app.services.read_displays = Some(std::sync::Arc::new(move || monitor_profile::detect_async_for_session(native_session)));
                }
                #[cfg(not(target_os = "linux"))]
                {
                    app.services.read_displays = Some(std::sync::Arc::new(monitor_profile::detect_async));
                }
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
            info.present = present_desc.clone();
            if let Some(v) = &stroke_trace {
                app.stroke_trace.set(v);
            }
            info.selected = if plan.env.is_some() { "env".into() } else { plan.backend.name().to_string() };
            info.fallback = match (plan.reason.clone(), gpu_note.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()) {
                (Some(reason), Some(note)) => Some(format!("{reason}. {note}")),
                (reason, note) => reason.or(note),
            };
            info.canvas = "cpu".into();
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
                app.perf.gpu_info.set_available(&rs.available_adapters);
                // Every egui texture uploads through this adapter, whichever canvas draws:
                // Iris Xe on DX12 shows stale memory in all-zero texels (texel_guard.rs).
                photocraft_ui_egui::texel_guard::install(&cc.egui_ctx, &rs.adapter.get_info());
                let software_window = rs.adapter.get_info().device_type == eframe::wgpu::DeviceType::Cpu;
                if software_window && mode != photocraft_engine::prefs::RenderingMode::Cpu {
                    // Keep an earlier note, such as a requested adapter that couldn't be used.
                    let software = "No compatible hardware graphics adapter; using software graphics.";
                    let fallback = &mut app.perf.gpu_info.fallback;
                    *fallback = Some(match fallback.take() {
                        Some(earlier) => format!("{earlier}. {software}"),
                        None => software.into(),
                    });
                }
                if !software_window
                    && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu
                    && std::env::var_os("PHOTOCRAFT_CPU_CANVAS").is_none()
                    && app.session.prefs().performance.effective_rendering_mode() != photocraft_engine::prefs::RenderingMode::Cpu
                {
                    app.set_wgpu(rs);
                    app.start_compositor(&cc.egui_ctx);
                } else {
                    // The window still draws with wgpu: record its errors instead of panicking,
                    // and notice when the device is lost (#3438).
                    app.watch_window_device(&rs.device);
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
            // CPU rendering after a failed GPU start is temporary. Only the
            // recovery-dialog choice may persist it (#3483).
            let remember = gpu_startup::backend_to_remember(&plan);
            app.on_started(move |app| {
                if let Some(s) = started_sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                    s.finish();
                }
                if let Some(b) = remember
                    && app.session.prefs().performance.gpu_backend != b
                    && let Err(e) = app.run("prefs.set", serde_json::json!({"path": "performance.gpuBackend", "value": b.name()}))
                {
                    log::warn!("couldn't remember GPU backend {}: {e}", b.name());
                }
            });
            // Agents in the open window. Behind the suite's AI-MCP flag the ArtCraft Launcher's
            // switch (Settings › AI agents) is the one consent; without it, as before, a Control
            // Port saved in the preferences. The launch flags (`--control`) manage the connection
            // for the whole run instead.
            app.services.agent_switch = Some(craft::automation::agents::AgentSwitch::watch(photocraft_automation::agent_link::LAUNCHER_APP_ID, {
                let ctx = cc.egui_ctx.clone();
                move || ctx.request_repaint()
            }));
            let link = if let Some((listener, token, _)) = control {
                let host = craft::ui::runtime::windows::WindowHost::of(&cc.egui_ctx);
                let (requests, server) = craft::automation::server::serve(listener, token, host)?;
                eprintln!("photocraft: control server listening on {}", server.addr);
                app = app.with_control(requests);
                craft::automation::agents::AgentLink::from_launch(server)
            } else {
                app.services.agent_automation = Some(Box::new(services::agent_automation));
                craft::automation::agents::AgentLink::new(services::config_dir(), photocraft_engine::build_info::long_version())
            };
            app.services.agent_link = Some(link);
            #[cfg(target_os = "macos")]
            {
                app.services.os_events = Some(apple_events.connect(&cc.egui_ctx));
                // The macOS menu bar, installed now so winit's default menu doesn't stay up.
                if !in_window_menus {
                    app.services.native_menu = photocraft_ui_egui::native_menu::install(&cc.egui_ctx, &mut app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = in_window_menus;
            // Files from later launches (Explorer, file managers) open here as new tabs.
            #[cfg(not(target_os = "macos"))]
            if let Some(primary) = single_instance {
                app.services.os_events = Some(primary.connect(&cc.egui_ctx));
            }
            // Where file drags and drops are (winit 0.30 doesn't say).
            app.services.cursor_pos = cursor::service(cc);
            // The physical primary button: it ends a floating document window's OS move by its title bar (#16415).
            app.services.primary_down = cursor::primary_down_service();
            // The custom title bar's undecorated window must stay borderless, or its caption and
            // border offset the content and the pointer on some Windows systems (#2246).
            app.services.window_frame = window_frame::service(cc, integrated_frame);
            // Caps Lock state (X11/Windows/macOS; `None` on native Wayland): the canvas shows the
            // precise crosshair for painting tools while it is toggled on (#1758).
            app.services.caps_lock = caps_lock::service(cc);
            // Tablet pressure/tilt/eraser (winit drops them): the macOS monitor and the X11 reader
            // write into the stylus feed. The monitor goes in here, not before the event loop:
            // AppKit's shared application only exists once winit created it (#759).
            #[cfg(target_os = "macos")]
            if let Some(monitor) = tablet::install_macos(&app.stylus.feed) {
                let _ = tablet_monitor.set(monitor);
            }
            #[cfg(target_os = "windows")]
            if let Some(monitor) = tablet::install_windows() {
                let _ = tablet_monitor_win.set(monitor);
            }
            // With Windows Ink disabled for the tablet, WinTab supplies the pen pressure, tilt,
            // rotation and eraser end; the vendor driver still moves the cursor.
            #[cfg(target_os = "windows")]
            if let Some(poll) =
                tablet::install_wintab(cc, &app.stylus.feed, app.session.prefs().tools.tablet_input == photocraft_engine::prefs::TabletInput::WinTab)
            {
                app.services.tablet_poll = Some(poll);
            }
            #[cfg(target_os = "linux")]
            tablet::spawn_x11(&app.stylus.feed, display);
            app.services.right_button_down = tablet::right_button_service();
            app.services.pen_end = tablet::pen_end_service();
            // Paths on the command line (Linux/Windows file associations, `photocraft a.psd`).
            app.open_paths(&files);
            if !unreadable_paths.is_empty() {
                use photocraft_ui_egui::i18n;
                let line = i18n::t("{path}: the path is not valid Unicode; rename the file and open it again.");
                let lines = unreadable_paths.iter().map(|p| i18n::fmt(line, &[("path", p.as_str())])).collect();
                photocraft_ui_egui::notices::post(&mut app, i18n::t("Could not open"), lines, craft::theme::variant::Tone::Bad, None);
            }
            // Portable marker found but its data folder isn't writable (#228): say where settings went.
            if let Some(w) = &app_dirs::current().warning {
                photocraft_ui_egui::notices::post(&mut app, "Portable mode is off", vec![w.clone()], craft::theme::variant::Tone::Neutral, None);
            }
            // A preference's automation folder that couldn't be opened, or a token file it couldn't
            // write: a Dock launch has no console.
            if let Some(ControlNotice { title, lines }) = control_warning {
                photocraft_ui_egui::notices::post(&mut app, title, lines, craft::theme::variant::Tone::Neutral, None);
            }
            Ok(app)
        }),
    );
    // The window is gone: refuse further files and free the lock, so a launch that is waiting (or
    // the CPU-compatibility relaunch below) becomes the primary instead of handing its files to a
    // process that is exiting.
    #[cfg(not(target_os = "macos"))]
    if let Some(primary) = single_instance {
        primary.shutdown();
    }
    // Closed or failed outside graphics initialization: not a driver crash. A renderer error
    // keeps the marker, so the next start tries a safer backend, unless there is no safer one:
    // a failed CPU start would otherwise pin every later start to CPU.
    let renderer_error = gpu_startup::keep_marker_after_run(&result);
    if !(renderer_error && keep_marker)
        && let Some(mut s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    {
        if result.is_err()
            && !renderer_error
            && let Some(marker) = previous.crashed()
        {
            // This failure supplies no new graphics-crash evidence: retain the previous
            // marker, rather than recording this attempt's fallback backend.
            if let Err(error) = s.write(marker.clone()) {
                log::warn!("couldn't restore previous GPU startup marker: {error}");
            }
        } else {
            s.finish();
        }
    }
    // Retry in a fresh process: winit event loops cannot be recreated reliably in-process.
    // Only renderer initialization failures qualify; never restart after editing has begun.
    if retry_cpu && !app_created.load(std::sync::atomic::Ordering::Relaxed) && matches!(&result, Err(eframe::Error::Wgpu(_))) {
        let reason = result.as_ref().err().map(ToString::to_string).unwrap_or_default();
        if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            s.finish();
        }
        if let Ok(exe) = std::env::current_exe() {
            // `--safe-gpu` first: after a `--`, it would be a file name.
            let launched = std::process::Command::new(exe)
                .arg("--safe-gpu")
                .args(std::env::args_os().skip(1))
                .env_remove("WGPU_BACKEND")
                .env("PHOTOCRAFT_GPU_STARTUP_FAILURE", &reason)
                .spawn();
            match launched {
                Ok(_) => return Ok(()),
                Err(error) => log::error!("could not start CPU compatibility mode: {error}"),
            }
        }
    }
    // A launch that fails before a window exists leaves nothing on screen, and a desktop launch
    // has no console for the `Err` above: log it, then say why in a native dialog with the log
    // file to attach (#7643). A terminal or automation launch can read standard error or the
    // exit status instead, so it keeps the silent exit.
    if let Err(error) = &result {
        log::error!("PhotoCraft could not start: {error}");
        if startup_report::needs_dialog(control_port.is_some()) {
            startup_report::show(&error.to_string(), &started_renderer, started_note.as_deref(), logger.and_then(logging::AppLogger::path).as_deref());
        }
    }
    result
}

/// Tests that spawn child processes hold this for reading, tests that check who holds a file lock
/// for writing ([`no_child_spawns`]): until its `exec`, a child holds a copy of every open file,
/// `flock`s included, so a spawn in another test could keep a lock this process just dropped held
/// for a moment.
#[cfg(test)]
pub(crate) static CHILD_SPAWNS: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Held by a test that checks who holds a file lock: no other test spawns a child meanwhile.
#[cfg(test)]
pub(crate) fn no_child_spawns() -> std::sync::RwLockWriteGuard<'static, ()> {
    CHILD_SPAWNS.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    /// Schema-probe `ui.ron` shapes *reconstructed* from the documented 0.14/0.15/0.37 layouts,
    /// not recorded machine saves: 0.14 and 0.15 both store the flat window entry, which the
    /// first 0.37 launch rewrites; a 0.37 file is left alone. Keeping all three probes makes a
    /// future change to any migration rule fail here first. They do not prove eframe's real
    /// `Memory`/`PanelState` deserialization.
    #[test]
    fn the_ui_ron_schema_probe_fixtures_migrate() {
        for (name, fixture) in [("0.14", include_str!("../tests/fixtures/ui-0.14.ron")), ("0.15", include_str!("../tests/fixtures/ui-0.15.ron"))] {
            let text = super::ui_state::migrate_for_test(fixture).unwrap_or_else(|e| panic!("{name}: {e}")).unwrap_or_else(|| panic!("{name}: not rewritten"));
            assert!(text.contains("normal_geometry"), "{name}: the flat window entry is gone: {text}");
            assert!(!super::ui_state::saved_position_for_test(fixture), "{name}: a flat entry is not what 0.37 restores");
        }
        let old = include_str!("../tests/fixtures/ui-0.14.ron");
        assert!(super::ui_state::migrate_for_test(old).expect("migratable").expect("rewritten").contains("x:1100.0"), "the 0.14 size is preserved");
        let current = include_str!("../tests/fixtures/ui-0.15.ron");
        assert!(
            super::ui_state::migrate_for_test(current).expect("migratable").expect("rewritten").contains("maximized:true"),
            "0.15 maximized state survives"
        );
        // Already 0.37-shaped: no rewrite, and (with no saved position) the window centres.
        let current = include_str!("../tests/fixtures/ui-0.37.ron");
        assert_eq!(super::ui_state::migrate_for_test(current), Ok(None));
        assert!(!super::ui_state::saved_position_for_test(current));
        // And a 0.37 file with a restorable outer position is a saved position.
        let with_position = current.replace("outer_position_pixels:None", "outer_position_pixels:Some((x:2048.0,y:1228.0))");
        assert!(super::ui_state::saved_position_for_test(&with_position));
    }

    #[test]
    fn the_window_opens_centred_at_its_default_size() {
        let o = super::native_options(&super::Frame::Native);
        assert!(o.centered, "#419: centred, not cascaded from the top-left corner");
        assert_eq!(o.viewport.inner_size, Some(egui::vec2(1440.0, 900.0)));
        // eframe shrinks the start size to the monitor, so the centred position is on-screen.
        assert_ne!(o.viewport.clamp_size_to_monitor_size, Some(false));
    }

    #[test]
    fn a_window_saved_while_minimized_reopens_at_the_minimum_size() {
        let hook = super::native_options(&super::Frame::Native).window_builder.expect("a window builder hook");
        // Saved minimized on Windows, then centred on a 1920 × 1080 monitor by eframe.
        let saved = super::native_options(&super::Frame::Native).viewport.with_inner_size([160.0, 64.0]).with_position([880.0, 508.0]);
        let opened = hook(saved);
        assert_eq!(opened.inner_size, Some(egui::vec2(760.0, 480.0)));
        assert_eq!(opened.position, Some(egui::pos2(580.0, 300.0)), "still centred");
        // Too narrow only: only the width grows.
        let opened = super::at_least_min_size(super::native_options(&super::Frame::Native).viewport.with_inner_size([500.0, 900.0]));
        assert_eq!(opened.inner_size, Some(egui::vec2(760.0, 900.0)));
        // A window at or above the minimum, or with no size, is left alone.
        for size in [[760.0, 480.0], [1100.0, 760.0]] {
            let builder = super::native_options(&super::Frame::Native).viewport.with_inner_size(size).with_position([10.0, 20.0]);
            let opened = super::at_least_min_size(builder);
            assert_eq!((opened.inner_size, opened.position), (Some(egui::Vec2::from(size)), Some(egui::pos2(10.0, 20.0))));
        }
        let mut builder = super::native_options(&super::Frame::Native).viewport;
        builder.inner_size = None;
        assert_eq!(super::at_least_min_size(builder).inner_size, None);
    }

    #[test]
    fn a_window_saved_in_full_screen_reopens_maximized_not_full_screen() {
        let hook = super::native_options(&super::Frame::Native).window_builder.expect("a window builder hook");
        // What eframe restores from a ui.ron saved in a full-screen mode on Windows.
        let saved = super::native_options(&super::Frame::Native)
            .viewport
            .with_inner_size([2560.0, 1392.0])
            .with_position([0.0, 0.0])
            .with_fullscreen(true)
            .with_maximized(true);
        let opened = hook(saved);
        assert_eq!(opened.fullscreen, Some(false), "Standard Screen Mode is where PhotoCraft starts");
        assert_eq!(opened.maximized, Some(true), "the rest of the saved geometry is kept");
        assert_eq!(opened.inner_size, Some(egui::vec2(2560.0, 1392.0)));
    }

    /// The preferences the launch reads from the settings folder `dir` (`services::saved_prefs`).
    fn launch_prefs(dir: Option<&std::path::Path>) -> photocraft_engine::prefs::Preferences {
        photocraft_engine::prefs::read_saved(dir).prefs
    }

    #[test]
    fn the_system_title_bar_preference_is_read_from_the_saved_preferences() {
        // #1271, #1316: Preferences › Interface › System Title Bar asks for the system's bar; the frame
        // decision (craft-window) turns the preference into the frame the platform allows.
        use super::TitleBarPref;
        let dir = std::env::temp_dir().join(format!("pc-titlebar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(photocraft_engine::prefs::LEGACY_PREFS_FILE);
        let read = || super::title_bar_pref(&launch_prefs(Some(&dir)));
        assert_eq!(super::title_bar_pref(&launch_prefs(None)), TitleBarPref::default(), "no settings folder: the default");
        assert_eq!(read(), TitleBarPref::default(), "nothing saved: the default");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(read(), TitleBarPref::default(), "an unreadable file: the default");
        std::fs::write(&file, r#"{"interface":{}}"#).unwrap();
        assert_eq!(read(), TitleBarPref::default(), "an older preference file: the default");
        std::fs::write(&file, r#"{"interface":{"systemTitleBar":true}}"#).unwrap();
        assert_eq!(read(), TitleBarPref::System);
        std::fs::write(&file, r#"{"interface":{"systemTitleBar":false}}"#).unwrap();
        assert_eq!(read(), TitleBarPref::Integrated);

        // Once the app's settings store has saved config.toml, the launch reads that, not the older file.
        let (mut store, _) = craft::settings::SettingsStore::open_with(photocraft_engine::prefs::store_options(Some(&dir)));
        store.update(|saved| saved.prefs.interface.system_title_bar = true);
        store.flush().unwrap();
        assert_eq!(read(), TitleBarPref::System, "config.toml wins over preferences.json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn integrations_preferences_decide_the_control_port() {
        let (off, on, preferred) = (agent_prefs(false, 0), agent_prefs(true, 0), agent_prefs(true, 7878));

        // Nothing asked for, nothing listening.
        assert_eq!(super::control_port_for(None, &on), (None, None));
        // --control wins over the preference.
        assert_eq!(super::control_port_for(Some(9000), &preferred), (Some(9000), None));
        // The preference alone opens nothing at launch: the agent link starts it while the app
        // runs.
        assert_eq!(super::control_port_for(None, &preferred), (None, None));

        // Allow Agent Control off refuses both, and says so when --control asked for one
        // rather than ignoring the flag.
        assert_eq!(super::control_port_for(None, &off), (None, None));
        let (port, warning) = super::control_port_for(Some(9000), &off);
        assert_eq!(port, None);
        assert!(warning.is_some_and(|w| w.contains("Allow Agent Control") && w.contains("9000")), "the refusal names the port");
        assert_eq!(super::control_port_for(None, &agent_prefs(false, 7878)), (None, None), "off beats a preferred port");
    }

    fn agent_prefs(allow: bool, port: u32) -> photocraft_engine::prefs::Integrations {
        let mut prefs = photocraft_engine::prefs::Integrations::default();
        prefs.agents.allow_agent_control = allow;
        prefs.agents.control_port = port;
        prefs
    }

    #[test]
    fn integrations_fall_back_to_the_defaults_without_a_readable_file() {
        let dir = std::env::temp_dir().join(format!("pc-integrations-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(photocraft_engine::prefs::LEGACY_PREFS_FILE);
        let default = photocraft_engine::prefs::Integrations::default();

        assert_eq!(launch_prefs(None).integrations, default, "no settings folder");
        assert_eq!(launch_prefs(Some(&dir)).integrations, default, "nothing saved");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(launch_prefs(Some(&dir)).integrations, default, "an unreadable file");
        std::fs::write(&file, r#"{"integrations":{"allowAgentControl":false,"controlPort":7878}}"#).unwrap();
        let read = launch_prefs(Some(&dir)).integrations;
        assert!(!read.agents.allow_agent_control);
        assert_eq!(read.agents.control_port, 7878);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The app loads preferences leniently (#6611): one wrong-typed value resets only itself. So
    /// a bad value elsewhere, or in the port, must not turn Allow Agent Control back on.
    #[test]
    fn a_bad_value_elsewhere_keeps_agent_control_off() {
        let dir = std::env::temp_dir().join(format!("pc-integrations-lenient-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(photocraft_engine::prefs::LEGACY_PREFS_FILE);
        // What the Preferences dialog shows for the same file.
        let shown = |text: &str| {
            let mut session = photocraft_engine::Session::new();
            session.load_prefs_json(text).unwrap();
            session.prefs().integrations.clone()
        };
        let read = |text: &str| {
            std::fs::write(&file, text).unwrap();
            let read = launch_prefs(Some(&dir)).integrations;
            assert_eq!(read, shown(text), "{text}");
            read
        };

        let r = read(r#"{"interface":{"showTooltips":"nope"},"integrations":{"allowAgentControl":false,"controlPort":7878}}"#);
        assert!(!r.agents.allow_agent_control, "a bad value in another section");
        assert_eq!(r.agents.control_port, 7878);
        let r = read(r#"{"integrations":{"allowAgentControl":false,"controlPort":"x"}}"#);
        assert!(!r.agents.allow_agent_control, "a bad port");
        assert_eq!(r.agents.control_port, 0, "the bad port alone takes its default");
        // A bad Allow Agent Control alone takes its default.
        let r = read(r#"{"integrations":{"allowAgentControl":"no","controlPort":7878}}"#);
        // (A saved port from before the agents switch turns it on, in both reads.)
        assert_eq!(r, photocraft_engine::prefs::Integrations { connect_agents: true, ..agent_prefs(true, 7878) });
        read(r#"{"integrations":{"allowAgentControl":false,"controlPort":-1}}"#);
        read(r#"{"integrations":{"allowAgentControl":false,"controlPort":4294967296}}"#);
        read(r#"{"integrations":"off"}"#);
        // A section that isn't an object keeps the defaults, in both reads.
        read(r#"{"integrations":[false,7878]}"#);
        // The automation folders: a bad one takes its default alone, too.
        let r = read(r#"{"integrations":{"allowAgentControl":false,"readRoot":5,"writeRoot":"/w"}}"#);
        assert!(!r.agents.allow_agent_control, "a bad folder");
        assert_eq!((r.agents.read_root.as_str(), r.agents.write_root.as_str()), ("", "/w"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn flags(read: Option<&std::path::Path>, write: Option<&std::path::Path>) -> photocraft_automation::args::AutomationArgs {
        photocraft_automation::args::AutomationArgs {
            automation_read_root: read.map(Into::into).into_iter().collect(),
            automation_write_root: write.map(Into::into),
        }
    }

    fn roots(read: &std::path::Path, write: &std::path::Path) -> craft::automation::agents::AgentSettings {
        let mut prefs = craft::automation::agents::AgentSettings::default();
        prefs.read_root = read.to_string_lossy().into_owned();
        prefs.write_root = write.to_string_lossy().into_owned();
        prefs
    }

    /// Preferences › AI Integrations' folders are the automation roots when no flag names one, and
    /// a flag wins over its preference, direction by direction.
    #[test]
    fn automation_folders_come_from_the_preferences_unless_a_flag_names_one() {
        let dir = temp_dir("automation-roots");
        let (pref, flag) = (dir.join("pref"), dir.join("flag"));
        std::fs::create_dir_all(&pref).unwrap();
        std::fs::create_dir_all(&flag).unwrap();

        // No flags: both directions from the preferences.
        let (ws, warning) = super::automation_workspace(&flags(None, None), &roots(&pref, &pref)).unwrap();
        assert_eq!(warning, None);
        ws.write("a.txt", b"pref").unwrap();
        assert_eq!(std::fs::read(pref.join("a.txt")).unwrap(), b"pref");
        assert_eq!(ws.read("a.txt").unwrap(), b"pref");
        // One folder for both shares one capability, as the flags do: an implicit save is allowed.
        assert_eq!(ws.read_with_save_path("a.txt").unwrap().1.as_deref(), Some("a.txt"));

        // A write flag wins over the write folder; reads still come from the preference.
        let (ws, _) = super::automation_workspace(&flags(None, Some(&flag)), &roots(&pref, &pref)).unwrap();
        ws.write("b.txt", b"flag").unwrap();
        assert!(flag.join("b.txt").is_file() && !pref.join("b.txt").exists(), "the flag's folder takes the write");
        assert_eq!(ws.read("a.txt").unwrap(), b"pref");
        assert_eq!(ws.read_with_save_path("a.txt").unwrap().1, None, "read and write folders differ: no implicit save");

        // A read folder spelled like the write flag shares the flag's capability.
        let (ws, _) = super::automation_workspace(&flags(None, Some(&flag)), &roots(&flag, &pref)).unwrap();
        assert_eq!(ws.read_with_save_path("b.txt").unwrap().1.as_deref(), Some("b.txt"));

        // Empty preferences and no default agent folder grant nothing, as no flags do.
        let (ws, warning) = super::automation_workspace_in(&flags(None, None), &craft::automation::agents::AgentSettings::default(), None).unwrap();
        assert_eq!(warning, None);
        assert!(ws.read("a.txt").is_err() && ws.write("c.txt", b"x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Empty preferences fall back to the default agent folder, created when first used, for
    /// both directions (one shared capability); a flag or the person's own folder still wins.
    #[test]
    fn empty_folder_preferences_use_the_default_agent_folder() {
        let dir = temp_dir("automation-roots-default");
        let default = dir.join("ArtCraft").join("Agents").join("PhotoCraft");
        let empty = craft::automation::agents::AgentSettings::default();

        let (ws, warning) = super::automation_workspace_in(&flags(None, None), &empty, Some(&default)).unwrap();
        assert_eq!(warning, None);
        assert!(default.is_dir(), "the default agent folder is created");
        ws.write("out.png", b"x").unwrap();
        assert_eq!(std::fs::read(default.join("out.png")).unwrap(), b"x");
        assert_eq!(ws.read_with_save_path("out.png").unwrap().1.as_deref(), Some("out.png"), "one folder for both: an implicit save is allowed");

        let flag = dir.join("flag");
        std::fs::create_dir_all(&flag).unwrap();
        let (ws, _) = super::automation_workspace_in(&flags(None, Some(&flag)), &empty, Some(&default)).unwrap();
        assert_eq!((ws.read_folders(), ws.write_folder()), (vec![default.clone()], Some(flag.clone())));

        // A write flag and the person's own read folder leave nothing for the default: not created.
        let unused = dir.join("unused");
        let mut own = empty.clone();
        own.read_root = flag.to_string_lossy().into_owned();
        let (ws, _) = super::automation_workspace_in(&flags(None, Some(&flag)), &own, Some(&unused)).unwrap();
        assert_eq!((ws.read_folders(), ws.write_folder()), (vec![flag.clone()], Some(flag.clone())));
        assert!(!unused.exists(), "an unused default folder is not created");

        let (ws, _) = super::automation_workspace_in(&flags(None, None), &own, Some(&default)).unwrap();
        assert_eq!((ws.read_folders(), ws.write_folder()), (vec![flag.clone()], Some(default.clone())));

        // A default that can't be made is left out with a warning; the app still starts.
        let file = dir.join("file");
        std::fs::write(&file, b"x").unwrap();
        let (ws, warning) = super::automation_workspace_in(&flags(None, None), &empty, Some(&file.join("PhotoCraft"))).unwrap();
        assert_eq!(warning, None, "a folder that couldn't be created is no folder at all");
        assert!(ws.read_folders().is_empty() && ws.write_folder().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A preference's folder that can't be opened is left out (that direction fails closed) and
    /// named in the warning; the app still starts. A flag's still stops the launch.
    #[test]
    fn a_missing_preference_folder_is_left_out_and_a_missing_flag_root_is_an_error() {
        let dir = temp_dir("automation-roots-missing");
        let missing = dir.join("gone");
        let (ws, warning) = super::automation_workspace(&flags(None, None), &roots(&dir, &missing)).unwrap();
        let warning = warning.expect("the missing write folder is named");
        assert!(warning.contains("Automation Write Folder") && !warning.contains("Read Folder"), "{warning}");
        std::fs::write(dir.join("in.txt"), b"x").unwrap();
        assert_eq!(ws.read("in.txt").unwrap(), b"x", "the read folder still applies");
        assert!(ws.write("out.txt", b"x").is_err(), "no write access without its folder");

        assert!(super::automation_workspace(&flags(Some(&missing), None), &roots(&dir, &dir)).is_err(), "a flag's root must open");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A relative folder preference would depend on the folder the app was started from: left
    /// out with a warning, like a missing one.
    #[test]
    fn a_relative_preference_folder_is_left_out() {
        let dir = temp_dir("automation-roots-relative");
        let mut relative = roots(&dir, &dir);
        relative.read_root = ".".into();
        let (ws, warning) = super::automation_workspace(&flags(None, None), &relative).unwrap();
        let warning = warning.expect("the relative read folder is named");
        assert!(warning.contains("Automation Read Folder") && warning.contains("not an absolute path"), "{warning}");
        assert!(ws.read("anything").is_err(), "no read access from a relative folder");
        ws.write("out.txt", b"x").unwrap();
        assert!(dir.join("out.txt").is_file(), "the absolute write folder still applies");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// X11 startup notification and docks match a window to its launcher by `StartupWMClass`,
    /// so it has to be the class the window carries: the app id, on Wayland and X11 alike.
    #[test]
    fn the_desktop_entry_names_the_window_class() {
        let entry = include_str!("../../../packaging/linux/ai.storyteller.photocraft.desktop");
        let class = entry.lines().find_map(|line| line.strip_prefix("StartupWMClass="));
        assert_eq!(class, Some(super::APP_ID));
    }

    /// Both flatpak manifests build with the craft-i18n commit `craft-i18n.rev` pins: CI and
    /// releases read the file, and the manifests have to name the commit themselves. The pin may
    /// have a trailing newline or CRLF (a Windows checkout), so it is trimmed, as every reader does.
    #[test]
    fn the_flatpak_manifests_pin_the_craft_i18n_rev() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let rev = std::fs::read_to_string(root.join("craft-i18n.rev")).unwrap();
        let rev = rev.trim();
        assert!(rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()), "craft-i18n.rev holds one full SHA: {rev:?}");
        for manifest in ["ai.storyteller.photocraft.yml", "ai.storyteller.photocraft.bundle.yml"] {
            let text = std::fs::read_to_string(root.join("packaging/linux/flatpak").join(manifest)).unwrap();
            let source = text.split("url: https://github.com/storytold/craft-i18n").nth(1).unwrap_or_else(|| panic!("{manifest}: a craft-i18n git source"));
            let source = source.split("- type:").next().unwrap_or(source);
            let commit =
                source.lines().find_map(|l| l.trim().strip_prefix("commit:")).unwrap_or_else(|| panic!("{manifest}: the craft-i18n source pins a commit"));
            assert_eq!(commit.trim().trim_matches(['\'', '"']), rev, "{manifest}");
            assert!(text.contains("CRAFT_I18N_DIR: /run/build/photocraft/craft-i18n"), "{manifest}: the build names the catalogs");
        }
    }
}
