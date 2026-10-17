//! GPU health in the shell (#243, #4): when the wgpu device is lost or reports an error, the
//! GPU canvas is dropped and every document keeps drawing through the CPU compositor and egui
//! textures, with a recovery warning. After an error Retry GPU builds a new GPU canvas on the same
//! device (#6577); after a loss the CPU canvas stays for the rest of the session. Also runs the desktop app's
//! "started" hook once the first frames have rendered (the crash-safe startup marker), and
//! provides the Help › System Info text (including the monitor profile in use).

use serde_json::json;

use crate::PhotocraftApp;

/// Notice title when the device was lost.
pub const LOST_MESSAGE: &str = "GPU device was lost; using the CPU renderer.";
/// Notice title when the device reported an error (out of memory, validation, internal).
pub const ERROR_MESSAGE: &str = "GPU error; using the CPU renderer.";
/// Frames after which startup counts as done even if a document never drew.
const STARTED_MAX_FRAMES: u64 = 120;

/// Hook run once the app has rendered its first frames (see [`PhotocraftApp::on_started`]).
pub type StartedHook = Box<dyn FnOnce(&mut PhotocraftApp)>;

/// Per-frame check: switch to the CPU canvas when the GPU faulted, and run the started hook.
pub fn check(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(fault) = app.gpu.as_ref().and_then(|g| g.fault()) {
        fall_back(app, &fault);
        ctx.request_repaint();
    }
    if app.started.is_some() {
        // Frames 1–2 set up fonts; by frame 3 the window has presented, and a document open at
        // launch has drawn its canvas (or the GPU path was abandoned).
        let drawn = app.session.documents().is_empty() || !app.perf.last_refresh.is_empty() || app.gpu.is_none();
        if app.frame >= 3 && (drawn || app.frame >= STARTED_MAX_FRAMES) {
            if let Some(hook) = app.started.take() {
                hook(app);
            }
        } else {
            ctx.request_repaint();
        }
    }
}

/// Drop the GPU canvas after `fault`: free its resources, send every view through the CPU
/// path, record why, and tell the user. Documents are untouched.
pub fn fall_back(app: &mut PhotocraftApp, fault: &photocraft_gpu::Fault) {
    let Some(gpu) = app.gpu.take() else { return };
    log::error!("{fault}; using the CPU renderer");
    gpu.release();
    app.gpu_fault = Some(fault.clone());
    // Canvas caches pointed at GPU textures: rebuild them as egui textures.
    app.canvases.clear();
    app.proxy_uploaded = None;
    app.prefs_rt.gpu_style = None;
    app.perf.gpu = false;
    app.perf.gpu_fallback = Some(fault.to_string());
    // The CPU canvas has no GPU memory budget: GPU Memory Full no longer applies.
    app.perf.gpu_layer_fit = None;
    app.perf.gpu_info.canvas = "cpu".into();
    app.perf.gpu_info.lost = Some(fault.to_string());
    let title = if fault.is_lost() { LOST_MESSAGE } else { ERROR_MESSAGE };
    queue_fallback_notice(app, fault.to_string());
    app.ui.status = tl!(title).to_string();
    app.ui.status_error = true;
}

/// English diagnostics for issue reports (Copy and the control API), independent of UI language.
pub fn system_info(app: &PhotocraftApp) -> Vec<String> {
    crate::i18n::with_language(crate::i18n::Lang::EN, || system_info_ui(app))
}

/// Help › System Info's on-screen text: version, platform and the graphics state.
pub fn system_info_ui(app: &PhotocraftApp) -> Vec<String> {
    let mut v = vec![format!("PhotoCraft {}", photocraft_engine::build_info::long_version()), {
        let template = "Platform: {os} {arch}";
        crate::i18n::fmt(tl!(template), &[("os", std::env::consts::OS), ("arch", std::env::consts::ARCH)])
    }];
    v.extend(app.perf.gpu_info.lines());
    v.extend(crate::monitor_status::summary_lines(app));
    v
}

/// The `help.systemInfo` result: the same facts as JSON.
pub fn system_info_json(app: &PhotocraftApp) -> serde_json::Value {
    json!({
        "version": photocraft_engine::build_info::long_version(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "gpu": app.perf.gpu_info,
        "monitor": app.session.color.monitor_status(),
        "displays": app.session.color.display_statuses(),
        "lines": system_info(app),
    })
}

/// Queue the same recovery warning for startup and runtime failures.
pub fn queue_fallback_notice(app: &mut PhotocraftApp, reason: impl Into<String>) {
    app.ui.gpu_fallback_notice = Some(reason.into());
}

/// Whether Retry GPU can build a new GPU canvas now: the canvas was dropped after an error (out
/// of memory, validation) on a device that still works. After a loss, or a GPU start that
/// failed, only a restart can try the GPU again.
fn can_retry_now(app: &PhotocraftApp) -> bool {
    app.gpu_render_state.is_some() && app.gpu_fault.as_ref().is_some_and(|f| !f.is_lost())
}

/// Build a new GPU canvas on the device the dropped one used, with a fresh health flag, and send
/// the views back through it (#6577). Returns whether the canvas is active again.
fn retry_now(app: &mut PhotocraftApp) -> bool {
    let Some(rs) = app.gpu_render_state.clone() else { return false };
    app.set_wgpu(rs);
    if !app.gpu_active() {
        return false;
    }
    log::info!("GPU canvas rebuilt after {}", app.gpu_fault.as_ref().map_or_else(String::new, ToString::to_string));
    app.gpu_fault = None;
    // The CPU canvas's egui textures: rebuild every view on the GPU.
    app.canvases.clear();
    app.proxy_uploaded = None;
    app.perf.gpu_fallback = None;
    app.perf.gpu_info.lost = None;
    app.ui.status = tl!("GPU acceleration resumed.").to_string();
    app.ui.status_error = false;
    true
}

/// Store the next-launch choice without restarting or risking unsaved documents. Retry GPU after
/// an error on a working device also builds a new GPU canvas at once (see [`retry_now`]).
fn choose_recovery(app: &mut PhotocraftApp, retry: bool) -> Result<(), String> {
    app.run(
        "prefs.set",
        json!({"values": {
            "performance.renderingMode": if retry { "gpu" } else { "cpu" },
            "performance.useGpu": retry,
            "performance.gpuBackend": "auto"
        }}),
    )?;
    crate::prefs_ui::save_preferences(app)?;
    app.ui.gpu_fallback_notice = None;
    if retry && can_retry_now(app) && retry_now(app) {
        return Ok(());
    }
    if retry {
        crate::notices::post(
            app,
            "GPU retry scheduled",
            vec!["Save your work and restart PhotoCraft to retry GPU acceleration. CPU rendering remains active for this session.".into()],
            false,
            None,
        );
    }
    Ok(())
}

/// Recovery is an explicit choice; the CPU document renderer stays active while the user works.
pub fn show_fallback(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(reason) = app.ui.gpu_fallback_notice.clone() else { return };
    let retry_now = can_retry_now(app);
    let t = crate::theme::Tokens::get(ctx);
    let mut choice = None;
    egui::Window::new(tl!("Switched to CPU rendering"))
        .id(egui::Id::new("gpu-fallback-warning"))
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(tl!("GPU acceleration could not continue. Your documents are unchanged.")).color(t.warning));
            ui.label(tl!("PhotoCraft is using the CPU image compositor. The window may still use your graphics adapter."));
            ui.collapsing(tl!("Details"), |ui| {
                ui.label(&reason);
            });
            if retry_now {
                ui.label(tl!("Retry GPU tries the graphics processor again now."));
            } else {
                ui.label(tl!("Retry GPU requires a restart. Save your work first."));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                use crate::widgets::{ButtonRole, DialogButton};
                let buttons =
                    [DialogButton::new(ButtonRole::Default, tl!("Keep Using CPU"), 150.0), DialogButton::new(ButtonRole::Alternate, tl!("Retry GPU"), 120.0)];
                if let Some(role) = crate::widgets::dialog_buttons(ui, &buttons) {
                    choice = Some(role == ButtonRole::Alternate);
                }
            });
        });
    if let Some(retry) = choice
        && let Err(error) = choose_recovery(app, retry)
    {
        crate::notices::error(app, error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_info_copy_and_api_stay_english_in_czech_ui() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.perf.gpu_info.adapter = "Test GPU".into();
        app.perf.gpu_info.preference = "auto".into();
        let english = crate::i18n::with_language(crate::i18n::Lang::EN, || system_info(&app));
        crate::i18n::with_language(crate::i18n::Lang::from_code("cs").unwrap(), || {
            assert_eq!(system_info(&app), english, "clipboard diagnostics include English monitor profiles too");
            let result = crate::menus::invoke(&mut app, &egui::Context::default(), "help.systemInfo", json!({})).unwrap();
            assert_eq!(result["info"]["lines"], json!(english));
            assert_eq!(crate::i18n::current().code(), "cs", "diagnostics restore the drawing language");
            let screen = system_info_ui(&app);
            assert!(screen.iter().any(|line| line == "Grafický adaptér: Test GPU"), "{screen:?}");
            assert!(screen.iter().any(|line| line.starts_with("Profil monitoru:")), "{screen:?}");
            assert!(english.iter().any(|line| line == "GPU backend preference: auto"));
        });
    }

    #[test]
    fn system_info_translates_empty_and_stored_auto_only_for_display() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // Hardware names are opaque even when they happen to match a lookup key.
        app.perf.gpu_info.adapter = "auto".into();
        crate::i18n::with_language(crate::i18n::Lang::from_code("cs").unwrap(), || {
            let empty = system_info_ui(&app);
            app.perf.gpu_info.preference = "auto".into();
            app.perf.gpu_info.selected = "auto".into();
            assert_eq!(system_info_ui(&app), empty);
            assert!(empty.iter().any(|line| line == "Grafický adaptér: auto"));
            assert!(empty.iter().any(|line| line == "Preferovaný backend GPU: automaticky"), "{empty:?}");
            assert_eq!(app.perf.gpu_info.preference, "auto");
            assert_eq!(app.perf.gpu_info.selected, "auto");
        });
    }

    #[test]
    fn fallback_warning_keeps_reason_in_inspectable_state() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        queue_fallback_notice(&mut app, "device unavailable");
        let state = serde_json::to_value(&app.ui).unwrap();
        assert_eq!(state["gpu_fallback_notice"], "device unavailable");
        assert_eq!(crate::control::inspect(&app, &egui::Context::default())["gpuFallbackNotice"], "device unavailable");
    }

    #[test]
    fn failed_preference_save_keeps_recovery_warning_open() {
        let services = crate::Services { save_prefs: Some(Box::new(|_| Err("disk full".into()))), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        queue_fallback_notice(&mut app, "GPU fault");
        assert!(choose_recovery(&mut app, true).is_err());
        assert_eq!(app.ui.gpu_fallback_notice.as_deref(), Some("GPU fault"));
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn recovery_choices_save_next_launch_mode_without_restarting() {
        let saves = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = saves.clone();
        let services = crate::Services {
            save_prefs: Some(Box::new(move |_| {
                count.set(count.get() + 1);
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        queue_fallback_notice(&mut app, "device unavailable");
        choose_recovery(&mut app, false).unwrap();
        assert!(!app.session.prefs().performance.use_gpu);
        assert_eq!(app.session.prefs().get("performance.renderingMode"), Some(json!("cpu")));
        assert!(app.ui.gpu_fallback_notice.is_none());
        crate::prefs_ui::tick(&mut app, &egui::Context::default());
        assert_eq!(saves.get(), 1, "recovery choice writes once");
        choose_recovery(&mut app, true).unwrap();
        assert!(app.session.prefs().performance.use_gpu);
        assert_eq!(app.session.prefs().get("performance.renderingMode"), Some(json!("gpu")));
        assert!(app.gpu.is_none());
        crate::prefs_ui::tick(&mut app, &egui::Context::default());
        assert_eq!(saves.get(), 2, "retry choice writes once");
    }

    /// #6577: after a GPU error on a device that still works (out of memory, validation), Retry
    /// GPU builds a new GPU canvas at once, without a restart or closing documents. After a loss
    /// the device can't be used again, so the retry still waits for the next launch.
    #[test]
    fn retry_gpu_after_an_error_rebuilds_the_canvas_without_a_restart() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default()));
        let Ok(rs) = rs else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let services = crate::Services { save_prefs: Some(Box::new(|_| Ok(()))), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.set_wgpu(rs);
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let ctx = egui::Context::default();
        // An error on a live device: Retry GPU rebuilds the canvas now.
        app.gpu_health().unwrap().mark(photocraft_gpu::Fault::Error("Out of Memory".into()));
        check(&mut app, &ctx);
        assert!(!app.gpu_active() && app.ui.gpu_fallback_notice.is_some());
        assert!(can_retry_now(&app));
        choose_recovery(&mut app, true).unwrap();
        assert!(app.gpu_active(), "Retry GPU rebuilt the GPU canvas");
        assert!(app.gpu_health().unwrap().is_ok(), "with a fresh health flag");
        assert_eq!(app.perf.gpu_info.canvas, "gpu");
        assert_eq!((app.perf.gpu_info.lost.as_deref(), app.perf.gpu_fallback.as_deref()), (None, None));
        assert_eq!(app.ui.status, "GPU acceleration resumed.");
        assert!(app.ui.gpu_fallback_notice.is_none() && app.ui.notices.is_empty(), "no restart notice");
        assert_eq!(app.session.documents().len(), 1, "the document stayed open");
        // A loss: the device can't be reused, so the retry is scheduled for the next launch.
        app.gpu_health().unwrap().mark(photocraft_gpu::Fault::Lost("simulated".into()));
        check(&mut app, &ctx);
        assert!(!can_retry_now(&app));
        choose_recovery(&mut app, true).unwrap();
        assert!(!app.gpu_active());
        assert_eq!(app.ui.notices.len(), 1, "the restart notice");
    }
}
