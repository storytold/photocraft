//! Help › System Info: the facts a bug report needs, so whoever picks it up (often an agent) can
//! tell which build, machine, graphics path, fonts and session it came from without a round of
//! questions. The report is taken when the dialog opens; the dialog shows it in sections, Copy
//! puts it on the clipboard as aligned plain text, and `help.systemInfo` returns it as JSON.
//!
//! The report is in English whatever the UI language: it is read by maintainers. It holds no
//! document names, document paths or pixels, and paths under the home folder are written `~/…`,
//! so it can be pasted into a public issue as is.

use serde::Serialize;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// What kind of install this is and where it keeps its files, filled in by the desktop app.
#[derive(Clone, Debug, Default)]
pub struct InstallInfo {
    /// "Installed", "Portable", "AppImage", "Flatpak", "Built from source", …
    pub kind: String,
    pub settings_dir: Option<std::path::PathBuf>,
    pub log_file: Option<std::path::PathBuf>,
    pub recovery_dir: Option<std::path::PathBuf>,
}

/// One titled group of label/value rows.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

impl Section {
    fn new(title: &str) -> Self {
        Self { title: title.to_string(), rows: Vec::new() }
    }

    fn row(&mut self, label: impl Into<String>, value: impl Into<String>) {
        self.rows.push((label.into(), value.into()));
    }
}

/// The whole report. `ctx` adds what only the window knows (display size and scale, uptime).
/// Built in English: the monitor lines it borrows are translated for the UI language.
pub fn report(app: &PhotocraftApp, ctx: Option<&egui::Context>) -> Vec<Section> {
    let ui_language = crate::i18n::current();
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        vec![application(app, ctx, ui_language), system(), graphics(app), displays(app, ctx), input(app), fonts(), session(app), files(app)]
    })
}

/// The report as plain text: a title, then each section with its values in one column.
pub fn text(sections: &[Section]) -> String {
    let width = sections.iter().flat_map(|s| &s.rows).map(|(l, _)| l.chars().count()).max().unwrap_or(0);
    let mut out = String::from("PhotoCraft System Info\n");
    for s in sections {
        out.push('\n');
        out.push_str(&s.title);
        out.push('\n');
        for (label, value) in &s.rows {
            let pad = width.saturating_sub(label.chars().count()) + 2;
            out.push_str(&format!("  {label}{}{value}\n", " ".repeat(pad)));
        }
    }
    out
}

/// The `help.systemInfo` result: the sections, the same as text lines, and the raw graphics and
/// monitor state.
pub fn json(app: &PhotocraftApp, sections: &[Section]) -> Value {
    json!({
        "version": photocraft_engine::build_info::long_version(),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "gpu": app.perf.gpu_info,
        "monitor": app.session.color.monitor_status(),
        "displays": app.session.color.display_statuses(),
        "sections": sections,
        "lines": text(sections).lines().filter(|l| !l.trim().is_empty()).map(str::trim_end).collect::<Vec<_>>(),
    })
}

fn application(app: &PhotocraftApp, ctx: Option<&egui::Context>, ui_language: crate::i18n::Lang) -> Section {
    let mut s = Section::new("Application");
    s.row("Version", photocraft_engine::build_info::long_version());
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    s.row("Build", format!("{profile}, {}-{}", std::env::consts::ARCH, std::env::consts::OS));
    let kind = app.services.install.as_ref().map_or(if cfg!(target_arch = "wasm32") { "Web browser" } else { "Unknown" }, |i| i.kind.as_str());
    s.row("Install", kind);
    let pref = &app.session.prefs().interface.language;
    s.row("Language", format!("{} (preference: {})", ui_language.name(), if pref.is_empty() { "auto" } else { pref }));
    if let Some(ctx) = ctx {
        s.row("Running for", duration(ctx.input(|i| i.time)));
    }
    s
}

fn system() -> Section {
    let mut s = Section::new("System");
    let p = platform();
    s.row("OS", p.os.clone());
    if let Some(k) = &p.kernel {
        s.row("Kernel", k.clone());
    }
    if let Some(d) = &p.desktop {
        s.row("Desktop", d.clone());
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let cpu = p.cpu.clone().unwrap_or_else(|| std::env::consts::ARCH.to_string());
    s.row("CPU", format!("{cpu}, {threads} threads{}", if p.translated { " (x86_64 build under Rosetta 2)" } else { "" }));
    #[cfg(not(target_arch = "wasm32"))]
    s.row("Worker threads", rayon::current_num_threads().to_string());
    let ram = match (crate::gpu_canvas::physical_memory(), available_memory()) {
        (Some(t), Some(a)) => format!("{} installed, {} available", bytes(t), bytes(a)),
        (Some(t), None) => format!("{} installed", bytes(t)),
        _ => "unknown".into(),
    };
    s.row("Memory", ram);
    s
}

fn graphics(app: &PhotocraftApp) -> Section {
    let mut s = Section::new("Graphics");
    let g = &app.perf.gpu_info;
    let or = |v: &str| if v.is_empty() { "unknown".to_string() } else { v.to_string() };
    let ids = match (g.vendor_id, g.device_id) {
        (0, 0) => String::new(),
        (v, d) => format!(" ({}, {v:04x}:{d:04x})", vendor_name(v)),
    };
    s.row("Adapter", format!("{}{ids}", or(&g.adapter)));
    s.row("Type", device_type_label(&g.device_type));
    s.row("Graphics API", backend_label(&g.backend));
    s.row("Driver", or(&g.driver));
    let preference = if g.preference.is_empty() { "auto" } else { g.preference.as_str() };
    let selected = if g.selected.is_empty() { "auto" } else { g.selected.as_str() };
    if selected == preference {
        s.row("Backend preference", backend_label(preference));
    } else {
        s.row("Backend preference", format!("{}, chosen at launch: {}", backend_label(preference), backend_label(selected)));
    }
    if let Some(f) = &g.fallback {
        s.row("Startup fallback", f.clone());
    }
    s.row("Image compositor", if g.canvas == "gpu" { "GPU" } else { "CPU" });
    if let Some(l) = &g.lost {
        s.row("This session", l.clone());
    }
    if app.perf.gpu_budget_mb > 0 {
        s.row("GPU memory budget", bytes(app.perf.gpu_budget_mb << 20));
    }
    s
}

fn displays(app: &PhotocraftApp, ctx: Option<&egui::Context>) -> Section {
    let mut s = Section::new("Displays");
    if let Some(ctx) = ctx {
        let ppp = ctx.pixels_per_point();
        let zoom = ctx.zoom_factor();
        let (monitor, native) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().native_pixels_per_point));
        if let Some(m) = monitor.filter(|m| m.x > 0.0 && m.y > 0.0) {
            s.row("Window's display", format!("{:.0}×{:.0} px", m.x * ppp, m.y * ppp));
        }
        let os = native.filter(|n| *n > 0.0).map_or(String::new(), |n| format!("OS scale {:.0}%, ", n * 100.0));
        s.row("UI scale", format!("{:.0}% ({os}UI zoom {:.0}%)", ppp * 100.0, zoom * 100.0));
        let window = ctx.content_rect().size() * ppp;
        s.row("Window", format!("{:.0}×{:.0} px", window.x, window.y));
    }
    for line in crate::monitor_status::summary_lines(app) {
        match line.split_once(": ") {
            Some((label, value)) => s.row(label, value),
            None => s.row("Monitor profile", line),
        }
    }
    s
}

fn input(app: &PhotocraftApp) -> Section {
    let mut s = Section::new("Pen input");
    s.row("Source", pen_source(app.services.is_wayland));
    let seen = app.stylus.seen();
    let parts: Vec<&str> = [(seen.pressure, "pressure"), (seen.tilt, "tilt"), (seen.rotation, "rotation"), (seen.eraser, "eraser end")]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect();
    s.row("Seen this session", if parts.is_empty() { "nothing (no pen or touch pressure yet)".to_string() } else { parts.join(", ") });
    s.row("Use Tablet Pressure", if app.stylus.use_pressure { "on" } else { "off" });
    s
}

fn fonts() -> Section {
    let mut s = Section::new("Fonts");
    let bundled = photocraft_text::CRAFT_FONTS.len();
    s.row("craft-fonts bundle", if bundled == 0 { "not in this build".to_string() } else { format!("{bundled} fonts") });
    s.row("Families", crate::type_tool::families().len().to_string());
    let fallbacks = photocraft_text::shared().lock().map(|e| e.fonts.fallback_families()).unwrap_or_default();
    s.row("Fallbacks", if fallbacks.is_empty() { "none found".to_string() } else { fallbacks.join(", ") });
    s
}

fn session(app: &PhotocraftApp) -> Section {
    let mut s = Section::new("Session");
    let docs = app.session.documents();
    s.row("Open documents", docs.len().to_string());
    for (i, st) in docs.iter().enumerate() {
        let d = &st.doc;
        s.row(
            format!("Document {}", i + 1),
            format!(
                "{}×{} px, {} {}-bit, {} layers, {} undo states, {} of pixels",
                d.size.width,
                d.size.height,
                crate::canvas::mode_label(d),
                d.depth.bits(),
                d.layer_count(),
                st.history.past_len(),
                bytes(st.history.pixel_bytes(d) as u64)
            ),
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(m) = memory_stats::memory_stats() {
        s.row("PhotoCraft memory", format!("{} in use", bytes(m.physical_mem as u64)));
    }
    s.row("Memory Usage limit", bytes(u64::from(app.session.prefs().performance.memory_usage_mb) << 20));
    let plugins = photocraft_plugins::registry::list();
    let names: Vec<String> = plugins.iter().map(|p| format!("{} {}", p.manifest().name, p.manifest().version)).collect();
    s.row("Plug-ins", if names.is_empty() { "none".to_string() } else { names.join(", ") });
    s
}

fn files(app: &PhotocraftApp) -> Section {
    let mut s = Section::new("Files");
    let home = home_dir();
    let path = |p: &Option<std::path::PathBuf>| p.as_deref().map_or("none".to_string(), |p| tidy_path(p, home.as_deref()));
    match &app.services.install {
        Some(i) => {
            s.row("Settings", path(&i.settings_dir));
            s.row("Log file", path(&i.log_file));
            s.row("Crash recovery", path(&i.recovery_dir));
        }
        None => s.row("Settings", if cfg!(target_arch = "wasm32") { "browser storage" } else { "unknown" }),
    }
    s
}

/// `path` with `home` written as `~`, so a pasted report doesn't carry the user's account name.
pub fn tidy_path(path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    match home.filter(|h| !h.as_os_str().is_empty()).and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

fn home_dir() -> Option<std::path::PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
}

/// `1.5 GB`, `812 MB`, `64 KB`.
pub fn bytes(b: u64) -> String {
    const K: f64 = 1024.0;
    let f = b as f64;
    if f >= K * K * K {
        format!("{:.1} GB", f / (K * K * K))
    } else if f >= K * K {
        format!("{:.0} MB", f / (K * K))
    } else if f >= K {
        format!("{:.0} KB", f / K)
    } else {
        format!("{b} bytes")
    }
}

/// `45 s`, `12 min`, `3 h 05 min`.
pub fn duration(secs: f64) -> String {
    let s = if secs.is_finite() { secs.max(0.0) as u64 } else { 0 };
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ => format!("{} h {:02} min", s / 3600, s % 3600 / 60),
    }
}

/// The graphics API in plain words (`dx12` → `DirectX 12`).
pub fn backend_label(backend: &str) -> String {
    match backend {
        "" | "auto" => "Auto",
        "dx12" => "DirectX 12",
        "vulkan" => "Vulkan",
        "metal" => "Metal",
        "gl" => "OpenGL",
        "webgpu" => "WebGPU",
        "cpu" => "CPU (software)",
        "env" => "WGPU_BACKEND environment variable",
        other => other,
    }
    .to_string()
}

/// The adapter kind in plain words (`discreteGpu` → `Dedicated GPU`, the word Windows uses).
pub fn device_type_label(kind: &str) -> String {
    match kind {
        "discreteGpu" => "Dedicated GPU",
        "integratedGpu" => "Integrated GPU",
        "virtualGpu" => "Virtual GPU",
        "cpu" => "Software renderer (no GPU)",
        "" => "unknown",
        other => other,
    }
    .to_string()
}

/// The GPU maker from its PCI vendor id.
pub fn vendor_name(id: u32) -> String {
    match id {
        0x10de => "NVIDIA".into(),
        0x1002 | 0x1022 => "AMD".into(),
        0x8086 => "Intel".into(),
        0x106b => "Apple".into(),
        0x5143 => "Qualcomm".into(),
        0x13b5 => "Arm".into(),
        0x1414 => "Microsoft".into(),
        0x1af4 => "virtio".into(),
        0x15ad => "VMware".into(),
        other => format!("vendor {other:04x}"),
    }
}

/// Where pen data comes from on this platform (see `stylus`).
fn pen_source(native_wayland: bool) -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "browser Pointer Events"
    } else if cfg!(windows) {
        "Windows Ink (WM_POINTER pressure; no tilt)"
    } else if cfg!(target_os = "macos") {
        "AppKit tablet events"
    } else if native_wayland {
        "none: native Wayland gives this app no pen input"
    } else {
        "X11 XInput2"
    }
}

/// What the OS reports about itself, read once (registry, `/proc`, `sw_vers`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Platform {
    pub os: String,
    pub kernel: Option<String>,
    /// Linux: the session type and desktop (`Wayland, GNOME`).
    pub desktop: Option<String>,
    pub cpu: Option<String>,
    /// macOS: an x86_64 build running under Rosetta 2.
    pub translated: bool,
}

fn platform() -> &'static Platform {
    static P: std::sync::OnceLock<Platform> = std::sync::OnceLock::new();
    P.get_or_init(detect_platform)
}

#[cfg(windows)]
fn detect_platform() -> Platform {
    use winsafe::{HKEY, RegistryValue, co};
    let get = |key: &str, name: &str| HKEY::LOCAL_MACHINE.RegGetValue(Some(key), Some(name), co::RRF::RT_ANY).ok();
    let text = |v: Option<RegistryValue>| match v {
        Some(RegistryValue::Sz(s) | RegistryValue::ExpandSz(s)) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        _ => None,
    };
    const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let ubr = match get(NT, "UBR") {
        Some(RegistryValue::Dword(n)) => Some(n),
        _ => None,
    };
    let os = windows_name(text(get(NT, "ProductName")).as_deref(), text(get(NT, "DisplayVersion")).as_deref(), text(get(NT, "CurrentBuild")).as_deref(), ubr);
    let cpu = text(get(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString"));
    Platform { os, cpu, ..Default::default() }
}

#[cfg(target_os = "macos")]
fn detect_platform() -> Platform {
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let version = run("/usr/bin/sw_vers", &["-productVersion"]).unwrap_or_else(|| "unknown".into());
    let os = match run("/usr/bin/sw_vers", &["-buildVersion"]) {
        Some(b) => format!("macOS {version} ({b})"),
        None => format!("macOS {version}"),
    };
    let cpu = run("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"]);
    let translated = run("/usr/sbin/sysctl", &["-n", "sysctl.proc_translated"]).as_deref() == Some("1");
    Platform { os, cpu, translated, ..Default::default() }
}

#[cfg(target_os = "linux")]
fn detect_platform() -> Platform {
    let read = |p: &str| std::fs::read_to_string(p).ok();
    let os = read("/etc/os-release").or_else(|| read("/usr/lib/os-release")).and_then(|t| os_release_name(&t)).unwrap_or_else(|| "Linux".into());
    let kernel = read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let cpu = read("/proc/cpuinfo").and_then(|t| cpuinfo_model(&t));
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let desktop = match (env("XDG_SESSION_TYPE"), env("XDG_CURRENT_DESKTOP")) {
        (Some(t), Some(d)) => Some(format!("{t}, {d}")),
        (t, d) => t.or(d),
    };
    Platform { os, kernel, desktop, cpu, translated: false }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn detect_platform() -> Platform {
    #[cfg(not(target_arch = "wasm32"))]
    let os = std::process::Command::new("uname")
        .arg("-sr")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    #[cfg(target_arch = "wasm32")]
    let os: Option<String> = None;
    Platform { os: os.unwrap_or_else(|| std::env::consts::OS.to_string()), ..Default::default() }
}

/// `Windows 11 Pro 25H2 (build 26200.6899)`. Windows 11 still calls itself "Windows 10" in
/// `ProductName`; its builds start at 22000.
pub fn windows_name(product: Option<&str>, display: Option<&str>, build: Option<&str>, ubr: Option<u32>) -> String {
    let build_no = build.and_then(|b| b.parse::<u32>().ok());
    let mut name = product.unwrap_or("Windows").to_string();
    if build_no.is_some_and(|b| b >= 22000)
        && let Some(rest) = name.strip_prefix("Windows 10")
    {
        name = format!("Windows 11{rest}");
    }
    if let Some(d) = display {
        name = format!("{name} {d}");
    }
    match (build, ubr) {
        (Some(b), Some(u)) => format!("{name} (build {b}.{u})"),
        (Some(b), None) => format!("{name} (build {b})"),
        _ => name,
    }
}

/// `PRETTY_NAME` from os-release (`Ubuntu 24.04.1 LTS`), else `NAME VERSION_ID`.
pub fn os_release_name(text: &str) -> Option<String> {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|v| !v.is_empty())
    };
    field("PRETTY_NAME").or_else(|| match (field("NAME"), field("VERSION_ID")) {
        (Some(n), Some(v)) => Some(format!("{n} {v}")),
        (n, _) => n,
    })
}

/// The CPU model from `/proc/cpuinfo` (x86 `model name`, Arm `Model` or `Hardware`).
pub fn cpuinfo_model(text: &str) -> Option<String> {
    ["model name", "Model", "Hardware"].iter().find_map(|key| {
        text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == *key).map(|(_, v)| v.trim().to_string())).filter(|v| !v.is_empty())
    })
}

/// Memory the OS could give out now without swapping, in bytes.
fn available_memory() -> Option<u64> {
    #[cfg(windows)]
    {
        winsafe::GlobalMemoryStatusEx().ok().map(|m| m.ullAvailPhys)
    }
    #[cfg(target_os = "linux")]
    {
        meminfo_available(&std::fs::read_to_string("/proc/meminfo").ok()?)
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// `MemAvailable` from `/proc/meminfo`, in bytes.
pub fn meminfo_available(text: &str) -> Option<u64> {
    let kb: u64 = text.lines().find_map(|l| l.strip_prefix("MemAvailable:"))?.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}

/// The System Info dialog's body: the report in sections, then Copy.
pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>) {
    let sections = match fields.get("report").cloned().and_then(|v| serde_json::from_value::<Vec<Section>>(v).ok()) {
        Some(s) => s,
        None => {
            let s = report(app, Some(ui.ctx()));
            fields.insert("report".into(), serde_json::to_value(&s).unwrap_or(Value::Null));
            s
        }
    };
    let t = crate::theme::Tokens::get(ui.ctx());
    let max_h = (ui.ctx().content_rect().height() * 0.7).clamp(240.0, 640.0);
    egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        for (i, s) in sections.iter().enumerate() {
            if i > 0 {
                ui.add_space(10.0);
            }
            ui.add(egui::Label::new(egui::RichText::new(&s.title).font(crate::theme::semibold(13.0)).color(t.text)).selectable(false));
            ui.add_space(2.0);
            egui::Grid::new(("system-info", i)).num_columns(2).spacing([16.0, 3.0]).min_col_width(140.0).show(ui, |ui| {
                for (label, value) in &s.rows {
                    ui.add(egui::Label::new(egui::RichText::new(label).color(t.text_dim)).selectable(false));
                    ui.add(egui::Label::new(egui::RichText::new(value).font(crate::theme::mono(12.0)).color(t.text)).wrap().selectable(true));
                    ui.end_row();
                }
            });
        }
    });
    ui.add_space(10.0);
    if crate::widgets::secondary_button(ui, tl!("Copy"), 84.0).clicked() {
        ui.ctx().copy_text(text(&sections));
    }
}

#[cfg(test)]
mod tests;
