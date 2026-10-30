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

/// One labelled fact in Help › System Info. `mono` is for build ids and driver versions.
pub struct SystemInfoRow {
    pub label: String,
    pub value: String,
    pub mono: bool,
}

/// A grouped block of facts, plus warning notes that are not key/value pairs.
pub struct SystemInfoSection {
    pub title: String,
    pub rows: Vec<SystemInfoRow>,
    pub notes: Vec<String>,
}

/// On-screen Help › System Info. Clipboard text stays [`system_info`].
pub struct SystemInfoModel {
    /// One sentence: where images are composited, and which adapter draws the window.
    pub headline: String,
    /// The headline is a warning: a fallback, a lost device, or CPU compositing that was not chosen.
    pub attention: bool,
    pub sections: Vec<SystemInfoSection>,
}

/// The grouped view of [`system_info_ui`].
pub fn system_info_model(app: &PhotocraftApp) -> SystemInfoModel {
    let info = &app.perf.gpu_info;
    let images = if info.canvas == "gpu" { "GPU" } else { "CPU" };
    let template = "Images: {images}. Window: {window}.";
    let headline = crate::i18n::fmt(tl!(template), &[("images", images), ("window", &window_phrase(info))]);
    let attention = info.lost.is_some() || info.fallback.is_some() || (info.canvas == "cpu" && !info.selected.is_empty() && info.selected != "cpu");
    let mut graphics_notes = Vec::new();
    if let Some(reason) = &info.fallback {
        graphics_notes.push(localize_diagnostic(reason));
    }
    if let Some(reason) = &info.lost {
        graphics_notes.push(localize_diagnostic(reason));
    }
    let profiles = crate::monitor_status::profile_rows(app);
    let mut display_notes = Vec::new();
    if let Some(note) = crate::monitor_status::retained_note(app) {
        display_notes.push(note);
    }
    let mut graphics_rows = vec![
        SystemInfoRow { label: tl!("Adapter").to_string(), value: shown_token(&info.adapter, "unknown"), mono: false },
        SystemInfoRow { label: tl!("Backend").to_string(), value: backend_value(info), mono: false },
        SystemInfoRow { label: tl!("Driver").to_string(), value: shown_token(&info.driver, "unknown"), mono: true },
        SystemInfoRow { label: tl!("GPU preference").to_string(), value: shown_choice(&info.preference), mono: false },
        SystemInfoRow { label: tl!("At launch").to_string(), value: shown_choice(&info.selected), mono: false },
        SystemInfoRow { label: tl!("Compositor").to_string(), value: images.to_string(), mono: false },
    ];
    if let Some(mb) = info.video_memory_mb {
        graphics_rows.push(SystemInfoRow {
            label: tl!("Graphics memory").to_string(),
            value: crate::chrome_ui::fmt_bytes(mb.saturating_mul(1 << 20)),
            mono: false,
        });
    }
    SystemInfoModel {
        headline,
        attention,
        sections: vec![
            SystemInfoSection {
                title: tl!("Application").to_string(),
                rows: vec![
                    SystemInfoRow { label: tl!("Version").to_string(), value: photocraft_engine::build_info::long_version(), mono: true },
                    SystemInfoRow { label: tl!("Platform").to_string(), value: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH), mono: false },
                ],
                notes: Vec::new(),
            },
            SystemInfoSection { title: tl!("Graphics").to_string(), rows: graphics_rows, notes: graphics_notes },
            SystemInfoSection {
                title: tl!("Display").to_string(),
                rows: profiles.into_iter().map(|(label, value)| SystemInfoRow { label, value, mono: false }).collect(),
                notes: display_notes,
            },
        ],
    }
}

/// Draw Help › System Info: a status line, then grouped facts. Copy keeps the English diagnostic.
pub fn show_system_info(app: &PhotocraftApp, ui: &mut egui::Ui) {
    let model = system_info_model(app);
    let max_h = (ui.ctx().content_rect().height() - 148.0).max(180.0);
    egui::ScrollArea::vertical().id_salt("system-info").max_height(max_h).show(ui, |ui| {
        let t = crate::theme::Tokens::get(ui.ctx());
        let headline_color = if model.attention { t.warning } else { t.text };
        ui.label(egui::RichText::new(&model.headline).font(crate::theme::semibold(13.0)).color(headline_color));
        for (i, section) in model.sections.iter().enumerate() {
            ui.add_space(if i == 0 { 12.0 } else { 10.0 });
            ui.label(egui::RichText::new(&section.title).font(crate::theme::medium(11.5)).color(t.text_faint));
            ui.add_space(4.0);
            info_rows(ui, &section.rows);
            for note in &section.notes {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(note).color(t.warning).size(12.0)).selectable(true));
            }
        }
    });
    ui.add_space(8.0);
    if crate::widgets::secondary_button(ui, tl!("Copy"), 84.0).clicked() {
        ui.ctx().copy_text(system_info(app).join("\n"));
    }
}

fn info_rows(ui: &mut egui::Ui, rows: &[SystemInfoRow]) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let label_w = 156.0;
    for row in rows {
        ui.horizontal_top(|ui| {
            let rest = (ui.available_width() - label_w - 12.0).max(80.0);
            ui.allocate_ui_with_layout(egui::vec2(label_w, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.set_max_width(label_w);
                ui.label(egui::RichText::new(&row.label).color(t.text_dim).size(12.0));
            });
            let text = if row.mono {
                egui::RichText::new(&row.value).font(crate::theme::mono(12.0)).color(t.text)
            } else {
                egui::RichText::new(&row.value).size(12.5).color(t.text)
            };
            ui.allocate_ui_with_layout(egui::vec2(rest, 0.0), egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.set_max_width(rest);
                ui.add(egui::Label::new(text).selectable(true));
            });
        });
        ui.add_space(2.0);
    }
}

fn window_phrase(info: &crate::gpu_canvas::GpuInfo) -> String {
    if info.device_type == "cpu" {
        return tl!("Software graphics").to_string();
    }
    if info.adapter.is_empty() {
        return tl!("Unknown adapter").to_string();
    }
    match pretty_backend(&info.backend) {
        Some(backend) => format!("{} ({backend})", info.adapter),
        None => info.adapter.clone(),
    }
}

fn backend_value(info: &crate::gpu_canvas::GpuInfo) -> String {
    let kind = device_kind(&info.device_type);
    match (pretty_backend(&info.backend), kind) {
        (Some(backend), Some(kind)) => format!("{backend} · {kind}"),
        (Some(backend), None) => backend.to_string(),
        (None, Some(kind)) => kind,
        (None, None) => shown_token(&info.backend, "unknown"),
    }
}

fn device_kind(kind: &str) -> Option<String> {
    Some(match kind {
        "discreteGpu" => tl!("Discrete GPU").to_string(),
        "integratedGpu" => tl!("Integrated GPU").to_string(),
        "virtualGpu" => tl!("Virtual GPU").to_string(),
        "cpu" => "CPU".to_string(),
        "other" => tl!("Other GPU").to_string(),
        _ => return None,
    })
}

/// A backend id as shown on screen. Unknown ids stay as reported.
fn pretty_backend(backend: &str) -> Option<&'static str> {
    Some(match backend {
        "vulkan" => "Vulkan",
        "dx12" => "Direct3D 12",
        "metal" => "Metal",
        "gl" => "OpenGL",
        "webgpu" => "WebGPU",
        "cpu" => "CPU",
        "noop" => "No-op",
        _ => return None,
    })
}

/// Empty and the stored default `auto` use the translated label; hardware names stay as reported.
fn shown_token(value: &str, when_empty: &str) -> String {
    if value.is_empty() || (value == "auto" && when_empty == "auto") {
        match when_empty {
            "unknown" => tl!("Unknown").to_string(),
            "auto" => tl!("Automatic").to_string(),
            other => other.to_string(),
        }
    } else {
        value.to_string()
    }
}

/// English diagnostic sentences, translated for the open dialog. Driver text after a colon stays
/// as the driver wrote it. Unknown sentences stay English, which is also what Copy reports.
pub(crate) fn localize_diagnostic(text: &str) -> String {
    if text == "CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics" {
        return tl!("CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics").to_string();
    }
    if text == "Intel graphics on Windows: using DirectX 12 (the Intel Vulkan driver is known to crash)" {
        return tl!("Intel graphics on Windows: using DirectX 12 (the Intel Vulkan driver is known to crash)").to_string();
    }
    if text == "--safe-gpu: CPU renderer for this launch" {
        return tl!("--safe-gpu: CPU renderer for this launch").to_string();
    }
    if text == "this platform doesn't report display profiles" {
        return tl!("this platform doesn't report display profiles").to_string();
    }
    if text == "the display profile hasn't been read yet" {
        return tl!("the display profile hasn't been read yet").to_string();
    }
    if text == "GPU device lost" {
        return tl!("GPU device lost").to_string();
    }
    if let Some(detail) = text.strip_prefix("GPU device lost: ") {
        return crate::i18n::fmt(tl!("GPU device lost: {detail}"), &[("detail", detail)]);
    }
    if let Some(detail) = text.strip_prefix("GPU error: ") {
        return crate::i18n::fmt(tl!("GPU error: {detail}"), &[("detail", detail)]);
    }
    if let Some(rest) = text.strip_prefix("WGPU_BACKEND=")
        && let Some(value) = rest.strip_suffix(" overrides the backend")
    {
        return crate::i18n::fmt(tl!("WGPU_BACKEND={value} overrides the backend"), &[("value", value)]);
    }
    if let Some(rest) = text.strip_prefix("the previous start didn't finish on ")
        && let Some((backend, next)) = rest.split_once("; using ")
    {
        return crate::i18n::fmt(tl!("the previous start didn't finish on {backend}; using {next}"), &[("backend", backend), ("next", next)]);
    }
    if let Some(display) = text.strip_suffix(" has no ICC profile") {
        return crate::i18n::fmt(tl!("{display} has no ICC profile"), &[("display", display)]);
    }
    if let Some(error) = text.strip_prefix("the display profile can't be read: ") {
        return crate::i18n::fmt(tl!("the display profile can't be read: {error}"), &[("error", error)]);
    }
    if let Some(rest) = text.strip_prefix('`')
        && let Some((profile, tail)) = rest.split_once("` is a ")
        && let Some(space) = tail.strip_suffix(" profile, not RGB")
    {
        return crate::i18n::fmt(tl!("`{profile}` is a {space} profile, not RGB"), &[("profile", profile), ("space", space)]);
    }
    if let Some(rest) = text.strip_prefix('`')
        && let Some((profile, error)) = rest.split_once("` can't be used as a display profile: ")
    {
        return crate::i18n::fmt(tl!("`{profile}` can't be used as a display profile: {error}"), &[("profile", profile), ("error", error)]);
    }
    text.to_string()
}

/// A launch choice: `auto` reads as Automatic, known backends use their technology name.
fn shown_choice(value: &str) -> String {
    if value.is_empty() || value == "auto" {
        tl!("Automatic").to_string()
    } else if let Some(name) = pretty_backend(value) {
        name.to_string()
    } else {
        value.to_string()
    }
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

    #[test]
    fn system_info_model_leads_with_the_window_and_keeps_the_english_copy() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.perf.gpu_info.adapter = "NVIDIA GeForce RTX 2060".into();
        app.perf.gpu_info.backend = "vulkan".into();
        app.perf.gpu_info.device_type = "discreteGpu".into();
        app.perf.gpu_info.driver = "NVIDIA 615.78.08".into();
        app.perf.gpu_info.preference = "auto".into();
        app.perf.gpu_info.selected = "cpu".into();
        app.perf.gpu_info.canvas = "cpu".into();
        app.perf.gpu_info.fallback = Some("CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics".into());
        let copied = system_info(&app);
        crate::i18n::with_language(crate::i18n::Lang::from_code("de").unwrap(), || {
            let model = system_info_model(&app);
            assert_eq!(model.headline, "Bilder: CPU. Fenster: NVIDIA GeForce RTX 2060 (Vulkan).");
            assert!(model.attention);
            assert_eq!(model.sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Anwendung", "Grafik", "Anzeige"]);
            let graphics = &model.sections[1];
            assert_eq!(graphics.rows[0].value, "NVIDIA GeForce RTX 2060");
            assert_eq!(graphics.rows[1].value, "Vulkan · Dedizierte GPU");
            assert!(graphics.rows[2].mono);
            assert_eq!(graphics.rows[2].value, "NVIDIA 615.78.08");
            assert_eq!(graphics.rows[3].value, "Automatisch");
            assert_eq!(graphics.rows[4].value, "CPU");
            assert_eq!(graphics.rows[5].value, "CPU");
            assert_eq!(graphics.notes, vec!["CPU-Bildrendering; kein Software-Grafikadapter verfügbar, daher nutzt das Fenster Hardwaregrafik".to_string()]);
            assert_eq!(model.sections[2].rows[0].label, "Monitor");
            assert!(model.sections[2].rows[0].value.contains("Ersatz"));
            assert!(model.sections[2].rows[0].value.contains("Diese Plattform meldet keine Monitorprofile"));
            assert_eq!(system_info(&app), copied);
            assert!(copied.iter().any(|line| line == "Graphics adapter: NVIDIA GeForce RTX 2060"));
            assert!(copied.iter().any(|line| line.contains("this platform doesn't report display profiles")));
        });
    }

    #[test]
    fn grouped_graphics_memory_is_localized_and_optional() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::i18n::with_language(crate::i18n::Lang::from_code("de").unwrap(), || {
            assert!(!system_info_model(&app).sections[1].rows.iter().any(|row| row.label == "Grafikspeicher"));
            app.perf.gpu_info.video_memory_mb = Some(8192);
            let model = system_info_model(&app);
            let memory = model.sections[1].rows.iter().find(|row| row.label == "Grafikspeicher").unwrap();
            let value = crate::chrome_ui::fmt_bytes(8192_u64 << 20);
            assert_eq!(memory.value, value);
            assert!(system_info(&app).iter().any(|line| line == &format!("Graphics memory: {value}")));
            app.perf.gpu_info.video_memory_mb = None;
            assert!(!system_info_model(&app).sections[1].rows.iter().any(|row| row.label == "Grafikspeicher"));
        });
    }

    #[test]
    fn chosen_cpu_compositor_is_not_flagged() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.perf.gpu_info.adapter = "Intel".into();
        app.perf.gpu_info.backend = "gl".into();
        app.perf.gpu_info.device_type = "integratedGpu".into();
        app.perf.gpu_info.preference = "cpu".into();
        app.perf.gpu_info.selected = "cpu".into();
        app.perf.gpu_info.canvas = "cpu".into();
        let model = system_info_model(&app);
        assert!(!model.attention);
        assert_eq!(model.headline, "Images: CPU. Window: Intel (OpenGL).");
        assert_eq!(model.sections[1].rows[1].value, "OpenGL · Integrated GPU");
        assert_eq!(model.sections[1].rows[3].value, "CPU");
        assert!(model.sections[1].notes.is_empty());
    }

    #[test]
    fn diagnostics_follow_the_ui_language_and_keep_driver_text() {
        crate::i18n::with_language(crate::i18n::Lang::EN, || {
            assert_eq!(localize_diagnostic("GPU device lost: DXGI_ERROR_DEVICE_REMOVED"), "GPU device lost: DXGI_ERROR_DEVICE_REMOVED");
            assert_eq!(localize_diagnostic("a note the catalog does not know"), "a note the catalog does not know");
        });
        crate::i18n::with_language(crate::i18n::Lang::from_code("fr").unwrap(), || {
            let text = localize_diagnostic("CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics");
            assert!(text.contains("CPU"), "{text}");
            assert_ne!(text, "CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics");
            let lost = localize_diagnostic("GPU device lost: DXGI_ERROR_DEVICE_REMOVED");
            assert!(lost.contains("DXGI_ERROR_DEVICE_REMOVED"), "{lost}");
            assert!(lost.starts_with("Périphérique GPU perdu"), "{lost}");
        });
        crate::i18n::with_language(crate::i18n::Lang::from_code("ja").unwrap(), || {
            let text = localize_diagnostic("this platform doesn't report display profiles");
            assert_ne!(text, "this platform doesn't report display profiles");
            assert!(text.contains('プ'), "{text}");
        });
    }

    #[test]
    fn system_info_dialog_paints() {
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 700.0)).with_max_steps(8).build_eframe(|cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.perf.gpu_info.adapter = "NVIDIA GeForce RTX 2060".into();
            app.perf.gpu_info.backend = "vulkan".into();
            app.perf.gpu_info.device_type = "discreteGpu".into();
            app.perf.gpu_info.driver = "NVIDIA 615.78.08".into();
            app.perf.gpu_info.preference = "auto".into();
            app.perf.gpu_info.selected = "cpu".into();
            app.perf.gpu_info.canvas = "cpu".into();
            app.perf.gpu_info.fallback = Some("CPU image rendering; no software graphics adapter is available, so the window uses hardware graphics".into());
            let _ = crate::menus::invoke(&mut app, &cc.egui_ctx, "help.systemInfo", json!({}));
            app
        });
        h.run_steps(2);
        let img = h.render().expect("render system info");
        assert!(img.width() > 0 && img.height() > 0);
        assert!(h.state().ui.dialogs.iter().any(|d| d.fields.get("systemInfo").and_then(serde_json::Value::as_bool) == Some(true)));
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
