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

/// Out-of-memory errors after which the GPU canvas is rebuilt by itself; past them the CPU
/// renderer stays until the user chooses Retry GPU, so a document that never fits doesn't rebuild
/// the canvas on every frame.
pub const OOM_AUTO_RETRIES: u32 = 3;

/// Per-frame check: switch to the CPU canvas when the GPU faulted, and run the started hook. An
/// out-of-memory error on a device that still works is not the end of the GPU for the session
/// (#6564): the dropped canvas freed its memory, so a new one is built at once and the views
/// redraw through it, up to [`OOM_AUTO_RETRIES`] times.
pub fn check(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(fault) = app.gpu.as_ref().and_then(|g| g.fault()) {
        fall_back(app, &fault);
        if fault.is_out_of_memory() && app.gpu_oom_retries < OOM_AUTO_RETRIES && can_retry_now(app) {
            app.gpu_oom_retries += 1;
            if retry_now(app) {
                log::warn!("{fault}; rebuilt the GPU canvas (automatic retry {} of {OOM_AUTO_RETRIES})", app.gpu_oom_retries);
                app.ui.gpu_fallback_notice = None;
            }
        }
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

/// End-of-frame check of the device the window itself draws with (#3438). Once wgpu reports it
/// lost (a driver reset, for example after the PC slept), egui-wgpu can't draw another frame: one
/// with shapes panics creating its staging buffers (#1215), and one without leaves a frozen window
/// whose close button opens an unsaved-changes prompt nobody can see. So this frame and every later
/// one draw nothing; the first writes every unsaved document to crash recovery, waits for the
/// writes, and closes the app without asking once all of them are saved. The next launch reopens
/// the documents. If a write fails (a full disk, say) the app stays open with the documents in
/// memory and tries again every [`RECOVERY_RETRY_SECS`]: closing would lose them.
pub fn check_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((device, health)) = app.window_device.as_ref() else { return };
    // A loss while the app sat idle or the PC slept is noticed here, before the next frame.
    health.probe(device);
    if !health.device_lost() {
        return;
    }
    ctx.graphics_mut(|g| *g = Default::default());
    let now = ctx.input(|i| i.time);
    if !app.window_lost {
        app.window_lost = true;
        let reason = health.fault().map_or_else(|| "GPU device lost".to_string(), |f| f.to_string());
        log::error!("{reason}: the window can't be drawn any more; saving unsaved documents for recovery and closing");
    } else {
        match app.window_recovery_retry {
            // Saved, and the close is on its way.
            None => return,
            Some(at) if now < at => {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0)));
                return;
            }
            Some(_) => {}
        }
    }
    match save_for_recovery(app) {
        Ok(()) => {
            app.window_recovery_retry = None;
            app.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        Err(e) => {
            log::error!("{e}; keeping the documents open and trying again in {RECOVERY_RETRY_SECS} s");
            app.window_recovery_retry = Some(now + RECOVERY_RETRY_SECS);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(RECOVERY_RETRY_SECS));
        }
    }
}

/// Seconds between attempts to save unsaved work for recovery after the window's device was lost.
pub const RECOVERY_RETRY_SECS: f64 = 2.0;

/// Write every document with unsaved changes to crash recovery, whatever the autosave timer and
/// preference, and wait until the writes finished (see `Services::flush_autosaves`). `Ok` only when
/// each of them is known to be on disk.
fn save_for_recovery(app: &mut PhotocraftApp) -> Result<(), String> {
    let jobs: Vec<_> = app.session.documents().iter().filter(|d| d.is_dirty()).map(|d| (d.doc.clone(), d.revision, d.path.clone())).collect();
    if jobs.is_empty() {
        return Ok(());
    }
    // (document id, why) for each document not known to be saved.
    let mut failed: Vec<(u64, String)> = Vec::new();
    let Some(save) = app.services.autosave.as_mut() else { return Err("no crash-recovery autosave".into()) };
    for (doc, revision, path) in &jobs {
        if let Err(e) = save(doc, *revision, path.as_deref()) {
            failed.push((doc.id.0, format!("{}: {e}", doc.name)));
        }
    }
    // Without a flush, only a service whose `autosave` reports the finished write can confirm it.
    let outcomes = match app.services.flush_autosaves.as_mut() {
        Some(flush) => flush(),
        None if app.services.autosave_results.is_none() => Vec::new(),
        None => return Err("crash-recovery autosaves can't be waited for".into()),
    };
    let confirm = app.services.flush_autosaves.is_some();
    for (doc, revision, _) in &jobs {
        if failed.iter().any(|(id, _)| *id == doc.id.0) {
            continue;
        }
        // The writer skips to the newest queued snapshot, so a later revision confirms this one.
        let mine: Vec<_> = outcomes.iter().filter(|(id, written, _)| *id == doc.id.0 && written >= revision).collect();
        if let Some((.., Err(e))) = mine.iter().find(|(.., outcome)| outcome.is_err()) {
            failed.push((doc.id.0, format!("{}: {e}", doc.name)));
        } else if confirm && mine.is_empty() {
            failed.push((doc.id.0, format!("{}: the write wasn't confirmed", doc.name)));
        } else {
            log::info!("{} (revision {revision}) saved for recovery", doc.name);
        }
    }
    let failed: Vec<_> = failed.into_iter().map(|(_, why)| why).collect();
    if failed.is_empty() { Ok(()) } else { Err(format!("couldn't save for recovery: {}", failed.join("; "))) }
}

/// Drop the GPU canvas after `fault`: free its resources, send every view through the CPU
/// path, record why, and tell the user. Documents are untouched.
pub fn fall_back(app: &mut PhotocraftApp, fault: &photocraft_gpu::Fault) {
    let Some(gpu) = app.gpu.take() else { return };
    // The GPU canvas is gone (that's this error); the switch to the CPU is `render_path`'s line.
    log::error!("{fault}; GPU canvas dropped");
    gpu.release();
    app.gpu_fault = Some(fault.clone());
    // Canvas caches pointed at GPU textures: rebuild them as egui textures.
    app.canvases.clear();
    app.proxy_uploaded = None;
    app.prefs_rt.gpu_style = None;
    app.perf.gpu = false;
    crate::render_path::note_app(app, Some(crate::render_path::CpuReason::from_fault(fault)));
    // The CPU canvas has no GPU memory budget: GPU Memory Full no longer applies.
    app.perf.gpu_layer_fit = None;
    app.perf.gpu_info.canvas = "cpu".into();
    app.perf.gpu_info.lost = Some(fault.to_string());
    let title = if fault.is_lost() { LOST_MESSAGE } else { ERROR_MESSAGE };
    queue_fallback_notice(app, fault.to_string());
    app.ui.status = tl!(title).to_string();
    app.ui.status_error = true;
}

/// What the GPU memory budgets were computed from (#9010, #9331): physical memory, the card's own
/// memory, Preferences › Performance › Memory Usage and GPU Memory, and the results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BudgetInputs {
    /// Physical memory in bytes (`None`: unknown, 16 GB assumed).
    pub ram: Option<u64>,
    /// The card's own memory in bytes (`None`: not reported, as for integrated GPUs, and discrete
    /// cards the platform doesn't report, such as NVIDIA on Linux; the budgets then come from
    /// system memory).
    pub vram: Option<u64>,
    /// Whether the adapter is a discrete card (`GpuInfo::device_type`), so an unreported `vram`
    /// isn't described as shared memory.
    pub discrete: bool,
    /// Memory Usage, bytes.
    pub memory_usage: u64,
    /// GPU Memory, bytes (0: follow Memory Usage).
    pub gpu_memory: u64,
    /// The compositor's layer budget in bytes (0 before the first GPU refresh).
    pub gpu_budget: u64,
    /// One document's display texture budget in bytes (`None` without a GPU canvas).
    pub canvas_budget: Option<u64>,
}

impl BudgetInputs {
    pub fn of(app: &PhotocraftApp) -> Self {
        let performance = &app.session.prefs().performance;
        Self {
            ram: crate::gpu_canvas::physical_memory(),
            vram: app.perf.gpu_info.video_memory_mb.map(|mb| mb.saturating_mul(1 << 20)),
            discrete: app.perf.gpu_info.device_type == "discreteGpu",
            memory_usage: u64::from(performance.memory_usage_mb).saturating_mul(1 << 20),
            gpu_memory: u64::from(performance.gpu_memory_mb).saturating_mul(1 << 20),
            gpu_budget: app.perf.gpu_budget_mb.saturating_mul(1 << 20),
            canvas_budget: app.gpu.as_ref().map(crate::gpu_canvas::GpuCanvas::canvas_budget_bytes),
        }
    }
}

/// The budget inputs and results, one per line (System Info, the log and Preferences ›
/// Performance), in the current UI language.
pub fn budget_lines(b: &BudgetInputs) -> Vec<String> {
    let size = crate::chrome_ui::fmt_bytes;
    let mut v = vec![match b.ram {
        Some(ram) => crate::i18n::fmt(tl!("System memory: {size}"), &[("size", &size(ram))]),
        None => tl!("System memory: unknown (16 GB assumed)").to_string(),
    }];
    // A card that reports its memory has a "Graphics memory" line already ([`GpuInfo::lines`]).
    if b.vram.is_none() {
        v.push(
            if b.discrete {
                tl!("Graphics memory: not reported (budgeted from system memory)")
            } else {
                tl!("Graphics memory: not reported (shares system memory)")
            }
            .to_string(),
        );
    }
    v.push(crate::i18n::fmt(tl!("Memory Usage: {size}"), &[("size", &size(b.memory_usage))]));
    v.push(if b.gpu_memory > 0 {
        crate::i18n::fmt(tl!("GPU Memory: {size}"), &[("size", &size(b.gpu_memory))])
    } else {
        tl!("GPU Memory: automatic (from Memory Usage)").to_string()
    });
    if b.gpu_budget > 0 {
        v.push(crate::i18n::fmt(tl!("GPU memory budget: {budget}"), &[("budget", &size(b.gpu_budget))]));
    }
    if let Some(c) = b.canvas_budget {
        v.push(crate::i18n::fmt(tl!("Canvas texture budget: {size}"), &[("size", &size(c))]));
    }
    v
}

/// The adapter report in the current UI language: version, platform, the adapter and why, the
/// other adapters, the budget inputs and the monitor profiles.
fn adapter_report_ui(app: &PhotocraftApp) -> Vec<String> {
    let mut v = vec![format!("PhotoCraft {}", photocraft_engine::build_info::long_version()), {
        let template = "Platform: {os} {arch}";
        crate::i18n::fmt(tl!(template), &[("os", std::env::consts::OS), ("arch", std::env::consts::ARCH)])
    }];
    v.extend(app.perf.gpu_info.lines());
    v.extend(budget_lines(&BudgetInputs::of(app)));
    v.extend(crate::monitor_status::summary_lines(app));
    v
}

/// Help › System Info's on-screen report, in the current UI language: the adapter report, the
/// render path and its history, and the slow frames.
pub fn diagnostics_report_ui(app: &PhotocraftApp) -> Vec<String> {
    let mut v = adapter_report_ui(app);
    v.extend(crate::render_path::report_lines(app));
    v.extend(crate::frame_stages::report_lines(&app.slow_frames));
    v
}

/// The diagnostics report in English, whatever the UI language: the one source for Help › System
/// Info ▸ Copy, `help.systemInfo`, Help › Report an Issue and the log.
pub fn diagnostics_report(app: &PhotocraftApp) -> Vec<String> {
    crate::i18n::with_language(crate::i18n::Lang::EN, || diagnostics_report_ui(app))
}

/// The adapter part of [`diagnostics_report`] as one English log line.
pub fn adapter_report_line(app: &PhotocraftApp) -> String {
    let lines = crate::i18n::with_language(crate::i18n::Lang::EN, || adapter_report_ui(app));
    format!("adapter report: {}", lines.join("; ").replace(['\n', '\r'], " "))
}

/// Log the adapter report once at `info`: at startup, once the window's adapter is known, and
/// when Retry GPU rebuilds the canvas.
pub fn log_adapter_report(app: &PhotocraftApp) {
    log::info!("{}", adapter_report_line(app));
}

/// English diagnostics for issue reports (Copy and the control API): [`diagnostics_report`].
pub fn system_info(app: &PhotocraftApp) -> Vec<String> {
    diagnostics_report(app)
}

/// Help › System Info's on-screen text: [`diagnostics_report_ui`].
pub fn system_info_ui(app: &PhotocraftApp) -> Vec<String> {
    diagnostics_report_ui(app)
}

/// The `help.systemInfo` result: the same facts as JSON, plus the report's parts as data.
pub fn system_info_json(app: &PhotocraftApp) -> serde_json::Value {
    let b = BudgetInputs::of(app);
    json!({
        "version": photocraft_engine::build_info::long_version(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "gpu": app.perf.gpu_info,
        "monitor": app.session.color.monitor_status(),
        "displays": app.session.color.display_statuses(),
        "lines": system_info(app),
        "diagnostics": {
            "budget": {
                "ramBytes": b.ram,
                "vramBytes": b.vram,
                "memoryUsageBytes": b.memory_usage,
                "gpuMemoryBytes": b.gpu_memory,
                "gpuBudgetBytes": b.gpu_budget,
                "canvasBudgetBytes": b.canvas_budget,
            },
            "renderPath": crate::render_path::inspect(app),
            "slowFrames": app.slow_frames,
        },
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
    crate::render_path::note_app(app, None);
    app.perf.gpu_info.lost = None;
    log_adapter_report(app);
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
        .id(egui::Id::unique("gpu-fallback-warning"))
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
            let cs = crate::i18n::current();
            let adapter = crate::i18n::fmt(crate::i18n::tr(cs, "Graphics adapter: {adapter}"), &[("adapter", "Test GPU")]);
            assert!(screen.contains(&adapter), "{screen:?}");
            let profile = crate::i18n::tr(cs, "Monitor profile: {profile}").split('{').next().unwrap_or_default();
            assert!(screen.iter().any(|line| line.starts_with(profile)), "{screen:?}");
            assert!(english.iter().any(|line| line == "GPU backend preference: auto"));
            // The report's newer parts are English in the copy and the API too.
            for part in ["System memory: ", "Memory Usage: ", "Render path: GPU", "Slow frames (over 50 ms): 0"] {
                assert!(english.iter().any(|line| line.starts_with(part)), "{part}: {english:?}");
            }
            assert!(result["info"]["diagnostics"]["budget"].is_object(), "{result}");
        });
    }

    fn en<T>(f: impl FnOnce() -> T) -> T {
        crate::i18n::with_language(crate::i18n::Lang::EN, f)
    }

    #[test]
    fn diagnostics_report_names_the_budget_inputs() {
        const G: u64 = 1 << 30;
        // An integrated GPU (#9010): no memory of its own, GPU Memory following Memory Usage.
        let shared =
            BudgetInputs { ram: Some(16 * G), vram: None, discrete: false, memory_usage: 8 * G, gpu_memory: 0, gpu_budget: 2 * G, canvas_budget: Some(4 * G) };
        assert_eq!(
            en(|| budget_lines(&shared)),
            [
                "System memory: 16.0G",
                "Graphics memory: not reported (shares system memory)",
                "Memory Usage: 8.0G",
                "GPU Memory: automatic (from Memory Usage)",
                "GPU memory budget: 2.0G",
                "Canvas texture budget: 4.0G",
            ]
        );
        // A discrete card names its memory in the adapter lines; a fixed GPU Memory; nothing
        // computed yet, and no GPU canvas.
        let card = BudgetInputs { ram: None, vram: Some(8 * G), discrete: true, memory_usage: 4 * G, gpu_memory: G, gpu_budget: 0, canvas_budget: None };
        assert_eq!(en(|| budget_lines(&card)), ["System memory: unknown (16 GB assumed)", "Memory Usage: 4.0G", "GPU Memory: 1.0G"]);
        // A discrete card the platform doesn't report (NVIDIA on Linux) doesn't share system
        // memory: its budgets are only taken from it.
        let unreported = BudgetInputs { vram: None, ..card };
        assert_eq!(en(|| budget_lines(&unreported))[1], "Graphics memory: not reported (budgeted from system memory)");
        // The app's report carries them, from the preferences and the computed budget.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.perf.gpu_budget_mb = 512;
        let report = diagnostics_report(&app);
        let b = BudgetInputs::of(&app);
        assert_eq!(b.gpu_budget, 512 << 20);
        for line in en(|| budget_lines(&b)) {
            assert!(report.contains(&line), "{line}: {report:?}");
        }
        assert!(report.iter().any(|l| l == "GPU memory budget: 512.0M"), "{report:?}");
        let json = system_info_json(&app);
        assert_eq!(json["diagnostics"]["budget"]["gpuBudgetBytes"], 512u64 << 20);
        assert_eq!(json["diagnostics"]["budget"]["memoryUsageBytes"], b.memory_usage);
    }

    #[test]
    fn diagnostics_report_lists_other_adapters_and_why_picked() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let info = &mut app.perf.gpu_info;
        info.adapter = "Intel UHD".into();
        info.selected = "auto".into();
        info.adapters = vec!["NVIDIA RTX".into(), "Intel UHD".into(), "Microsoft Basic Render Driver".into()];
        let report = diagnostics_report(&app);
        assert!(report.iter().any(|l| l == "Why this adapter: the automatic choice"), "{report:?}");
        assert!(report.iter().any(|l| l == "Other graphics adapters: NVIDIA RTX, Microsoft Basic Render Driver"), "{report:?}");
        // Picked by Preferences › Performance › Graphics Adapter: the request names it.
        app.perf.gpu_info.requested_adapter = Some("intel uhd".into());
        assert!(diagnostics_report(&app).iter().any(|l| l == "Why this adapter: the Graphics Adapter preference (\"intel uhd\")"));
        // Picked by PHOTOCRAFT_GPU_ADAPTER for this launch: the override, not the preference.
        app.perf.gpu_info.requested_adapter_env = true;
        let report = diagnostics_report(&app);
        assert!(report.iter().any(|l| l == "Why this adapter: the PHOTOCRAFT_GPU_ADAPTER environment override (\"intel uhd\")"), "{report:?}");
        app.perf.gpu_info.requested_adapter_env = false;
        // A request this adapter doesn't match isn't the reason (a missed one is a fallback note);
        // the automatic choice within a chosen backend says so.
        app.perf.gpu_info.requested_adapter = Some("nvidia".into());
        app.perf.gpu_info.selected = "vulkan".into();
        assert!(diagnostics_report(&app).iter().any(|l| l == "Why this adapter: the automatic choice, vulkan only"));
        app.perf.gpu_info.requested_adapter = None;
        assert_eq!(serde_json::to_value(&app.perf.gpu_info).unwrap()["requestedAdapter"], serde_json::Value::Null);
        // A startup note explains the pick instead (gpu_startup's note lands in `fallback`).
        let note = "Intel graphics on Windows: using DirectX 12 (the Intel Vulkan driver is known to crash)";
        app.perf.gpu_info.fallback = Some(note.into());
        app.perf.gpu_info.selected = "dx12".into();
        let report = diagnostics_report(&app);
        assert!(report.contains(&format!("Fallback: {note}")), "{report:?}");
        assert!(report.iter().all(|l| !l.starts_with("Why this adapter")), "{report:?}");
        assert!(report.iter().any(|l| l == "Selected at launch: dx12"), "{report:?}");
        // The only adapter: nothing else to pick.
        app.perf.gpu_info.adapters = vec!["Intel UHD".into()];
        assert!(diagnostics_report(&app).iter().any(|l| l == "Other graphics adapters: none"));
    }

    #[test]
    fn diagnostics_report_includes_fallback_history_and_slow_frames() {
        use crate::render_path::{CpuReason, Path, note};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        note(&mut app, 7, Path::Cpu(CpuReason::Unsupported("Advanced Blending on Layer 1".into())));
        note(&mut app, 7, Path::Gpu { full: true });
        let mut slow = crate::frame_stages::FrameStages::default();
        slow.refresh("full", 24_000_000, 80.0, 2.0);
        app.slow_frames.finish(slow, crate::gpu_canvas::now_ms(), false, None);
        let report = diagnostics_report(&app);
        let at = |p: &str| report.iter().position(|l| l.contains(p)).unwrap_or_else(|| panic!("{p}: {report:?}"));
        assert!(report.iter().any(|l| l == "Render path: GPU"), "{report:?}");
        assert!(report.iter().any(|l| l == "Render path changes: 2"), "{report:?}");
        // Newest first: back on the GPU, then why it left.
        assert!(at("document 7: back on the GPU") < at("document 7: CPU (unsupported: Advanced Blending on Layer 1)"));
        assert!(report.iter().any(|l| l == "Slow frames (over 50 ms): 1"), "{report:?}");
        assert!(at("slow frame: 82.0 ms on the cpu path") > at("Slow frames (over 50 ms): 1"));
        // A current reason names its kind and detail.
        note(&mut app, 0, Path::Cpu(CpuReason::ViewTransform));
        assert!(diagnostics_report(&app).iter().any(|l| l.starts_with("Render path: CPU (viewTransform: ")));
        let json = system_info_json(&app);
        assert_eq!(json["diagnostics"]["slowFrames"].as_array().map(Vec::len), Some(1));
        assert_eq!(json["diagnostics"]["renderPath"]["history"].as_array().map(Vec::len), Some(3));
    }

    #[test]
    fn adapter_report_log_line_is_one_line() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.perf.gpu_info.adapter = "Test GPU".into();
        app.perf.gpu_info.driver = "Driver 1\nbuild 2".into();
        crate::i18n::with_language(crate::i18n::Lang::from_code("cs").unwrap(), || {
            let line = adapter_report_line(&app);
            assert!(!line.contains('\n') && !line.contains('\r'), "{line}");
            assert!(line.starts_with("adapter report: PhotoCraft "), "{line}");
            for part in ["Graphics adapter: Test GPU", "Driver: Driver 1 build 2", "System memory: ", "Memory Usage: "] {
                assert!(line.contains(part), "{part}: {line}");
            }
            // The adapter part of the report, not the render path or the slow frames.
            assert!(!line.contains("Render path"), "{line}");
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
            let cs = crate::i18n::current();
            let adapter = crate::i18n::fmt(crate::i18n::tr(cs, "Graphics adapter: {adapter}"), &[("adapter", "auto")]);
            assert!(empty.contains(&adapter), "{empty:?}");
            let preference = crate::i18n::fmt(crate::i18n::tr(cs, "GPU backend preference: {backend}"), &[("backend", crate::i18n::tr(cs, "auto"))]);
            assert!(empty.contains(&preference), "{empty:?}");
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

    /// #6564: an out-of-memory error on a device that still works rebuilds the GPU canvas by itself,
    /// with no notice and the documents untouched, a few times a session; then the CPU renderer
    /// stays until Retry GPU. Other errors and a loss still fall back at once.
    #[test]
    fn out_of_memory_rebuilds_the_gpu_canvas_by_itself() {
        let _gpu = photocraft_testkit::gpu_lock();
        let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default()));
        let Ok(rs) = rs else {
            eprintln!("skipping: no GPU adapter");
            return;
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.set_wgpu(rs);
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let ctx = egui::Context::default();
        let oom = || photocraft_gpu::Fault::Error("Out of Memory".into());
        for n in 1..=OOM_AUTO_RETRIES {
            app.gpu_health().unwrap().mark(oom());
            check(&mut app, &ctx);
            assert!(app.gpu_active(), "retry {n}: the GPU canvas is back");
            assert!(app.gpu_health().unwrap().is_ok(), "retry {n}: with a fresh health flag");
            assert_eq!(app.perf.gpu_info.canvas, "gpu");
            assert!(app.ui.gpu_fallback_notice.is_none(), "retry {n}: no recovery dialog");
            assert_eq!(app.session.documents().len(), 1);
        }
        // Past the limit the CPU renderer stays and the dialog offers Retry GPU.
        app.gpu_health().unwrap().mark(oom());
        check(&mut app, &ctx);
        assert!(!app.gpu_active() && app.ui.gpu_fallback_notice.is_some());
        assert_eq!(app.perf.gpu_info.canvas, "cpu");

        // A validation error is not retried by itself.
        let rs = egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.set_wgpu(rs);
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.gpu_health().unwrap().mark(photocraft_gpu::Fault::Error("Validation Error".into()));
        check(&mut app, &ctx);
        assert!(!app.gpu_active() && app.ui.gpu_fallback_notice.is_some());
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
        // An error on a live device (one not retried by itself, see #6564): Retry GPU rebuilds
        // the canvas now.
        app.gpu_health().unwrap().mark(photocraft_gpu::Fault::Error("Validation Error".into()));
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
