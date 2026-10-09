//! Community and project links: the Help menu, the About dialog, the start screen and the title
//! bar all open these. URLs follow the crafting-app pattern (`getartcraft.com/apps/<app>`,
//! `github.com/storytold/<app>`).

use serde_json::{Value, json};

use crate::PhotocraftApp;

pub const DISCORD: &str = "https://discord.gg/artcraft";
pub const ARTCRAFT_WEBSITE: &str = "https://getartcraft.com";
pub const APP_PAGE: &str = "https://getartcraft.com/apps/photocraft";
pub const GITHUB: &str = "https://github.com/storytold/photocraft";
pub const ISSUES: &str = "https://github.com/storytold/photocraft/issues";
const NEW_ISSUE: &str = "https://github.com/storytold/photocraft/issues/new";

/// Report choices in priority order: (localisable label, GitHub issue-form filename).
pub const ISSUE_TYPES: &[(&str, &str)] =
    &[("Bug report", "bug_report.yml"), ("Compatibility issue", "compatibility.yml"), ("Feature request", "feature_request.yml")];

/// Prefill only build/platform facts and structured graphics metadata. System Info's free-form
/// fallback/error text can contain paths or document details, so it must not enter the URL.
pub fn issue_report_url(app: &PhotocraftApp, ctx: &egui::Context, template: &str) -> String {
    report_url(
        template,
        &photocraft_engine::build_info::long_version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        &app.perf.gpu_info,
        app.ui.theme.label(),
        ctx.pixels_per_point(),
    )
}

fn report_url(template: &str, version: &str, os: &str, arch: &str, gpu: &crate::gpu_canvas::GpuInfo, theme: &str, pixels_per_point: f32) -> String {
    // Bound UTF-8 bytes (not characters) so even percent-encoded Unicode stays below 2 KiB.
    fn bounded(value: &str, limit: usize) -> String {
        value.char_indices().take_while(|(i, c)| i.saturating_add(c.len_utf8()) <= limit).map(|(_, c)| c).collect()
    }
    let version = bounded(version, 80);
    let platform = format!("{} {}", bounded(os, 32), bounded(arch, 32));
    let mut lines = vec![
        format!("PhotoCraft {version}"),
        format!("Platform: {platform}"),
        format!("Build: {}", if cfg!(debug_assertions) { "debug" } else { "release" }),
        format!("Canvas renderer: {}", if gpu.canvas == "gpu" { "GPU" } else { "CPU" }),
        format!("Theme: {}", bounded(theme, 32)),
        format!("UI pixels per point: {pixels_per_point:.2}"),
    ];
    if gpu.fallback.is_some() {
        lines.push("Startup GPU fallback: yes".into());
    }
    if gpu.lost.is_some() {
        lines.push("GPU lost or faulted this session: yes".into());
    }
    for (label, value) in [
        ("Graphics adapter", gpu.adapter.as_str()),
        ("Backend", gpu.backend.as_str()),
        ("Device type", gpu.device_type.as_str()),
        ("Driver", gpu.driver.as_str()),
    ] {
        if !value.is_empty() {
            lines.push(format!("{label}: {}", bounded(value, 80)));
        }
    }
    let info = bounded(&lines.join("\n"), 400);
    let params = [("template", template), ("version", version.as_str()), ("os", platform.as_str()), ("system-info", info.as_str())];
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{NEW_ISSUE}?{query}")
}

/// Open the editable GitHub form only in response to the user's report action.
pub fn report_issue(app: &mut PhotocraftApp, ctx: &egui::Context, template: &str) -> Value {
    let url = issue_report_url(app, ctx, template);
    open(app, ctx, &url)
}

/// Help-menu link commands: (id, url). Labels live in `menus::UI_COMMANDS`.
pub const COMMANDS: &[(&str, &str)] =
    &[("help.discord", DISCORD), ("help.website", APP_PAGE), ("help.artcraftWebsite", ARTCRAFT_WEBSITE), ("help.github", GITHUB), ("help.reportIssue", ISSUES)];

pub fn url_for(id: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)
}

/// Open `url` in the system browser (a new tab on the web) and note it in the status bar. Prefers
/// the platform `open_url` service (reliable on Windows/macOS/Linux); falls back to `ctx.open_url`.
pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context, url: &str) -> Value {
    let opened = app.services.open_url.as_ref().map(|f| f(url).is_ok()).unwrap_or(false);
    if !opened {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
    app.ui.status = format!("Opened {url}");
    json!({"url": url})
}

/// The prominent "Join us on Discord" button.
pub fn discord_button(app: &mut PhotocraftApp, ui: &mut egui::Ui, min_width: f32) -> egui::Response {
    let r = crate::widgets::primary_button(ui, tl!("Join us on Discord"), min_width).on_hover_text(DISCORD);
    if r.clicked() {
        open(app, ui.ctx(), DISCORD);
    }
    r
}

/// "PhotoCraft website · GitHub · ArtCraft" as links, centred. Clicks route through [`open`] (the
/// platform browser service) rather than `ui.hyperlink_to`, which uses the unreliable `ctx.open_url`.
pub fn link_row(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let links = [(tl!("PhotoCraft website"), APP_PAGE), (tl!("GitHub"), GITHUB), (tl!("ArtCraft"), ARTCRAFT_WEBSITE)];
    let font = egui::FontId::proportional(12.5);
    let sep = "  ·  ";
    let width: f32 = links.iter().map(|(l, _)| ui.painter().layout_no_wrap((*l).into(), font.clone(), t.text).size().x).sum::<f32>()
        + 2.0 * ui.painter().layout_no_wrap(sep.into(), font.clone(), t.text).size().x;
    let mut clicked: Option<&str> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        for (i, (label, url)) in links.iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new(sep).font(font.clone()).color(t.text_faint));
            }
            if ui.link(egui::RichText::new(*label).font(font.clone()).color(t.accent)).on_hover_text(*url).clicked() {
                clicked = Some(url);
            }
        }
    });
    if let Some(url) = clicked {
        open(app, ui.ctx(), url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_fields(url: &str) -> std::collections::BTreeMap<String, String> {
        let (base, query) = url.split_once('?').unwrap();
        assert_eq!(base, NEW_ISSUE);
        query
            .split('&')
            .map(|pair| {
                let (key, value) = pair.split_once('=').unwrap();
                (key.to_owned(), percent_encoding::percent_decode_str(value).decode_utf8().unwrap().into_owned())
            })
            .collect()
    }

    #[test]
    fn issue_choices_and_button_have_translations_in_every_shipped_catalog() {
        for lang in crate::i18n::Lang::all().filter(|lang| *lang != crate::i18n::Lang::EN) {
            for label in ISSUE_TYPES.iter().map(|(label, _)| *label).chain(["Report an Issue", "Report an issue on GitHub"]) {
                assert!(crate::i18n::has(lang, label), "{}: missing {label}", lang.code());
            }
        }
    }

    #[test]
    fn bug_report_encodes_injected_version_and_graphics_metadata() {
        let version = "2.3.4-rc+1 (abc&def, β)";
        let gpu = crate::gpu_canvas::GpuInfo {
            adapter: "GPU & 光影 #1?name=value".into(),
            backend: "vulkan".into(),
            device_type: "discreteGpu".into(),
            driver: "Mesa + LLVM\nversion=25.1%".into(),
            canvas: "gpu".into(),
            ..Default::default()
        };
        let url = report_url("bug_report.yml", version, "linux", "aarch64", &gpu, "Studio (Light)", 1.5);
        let fields = report_fields(&url);
        assert_eq!(fields.len(), 4, "Special characters must not become new parameters");
        assert_eq!(fields["template"], "bug_report.yml");
        assert_eq!(fields["version"], version);
        assert_eq!(fields["os"], "linux aarch64");
        let info = &fields["system-info"];
        assert!(info.contains(&format!("PhotoCraft {version}")));
        assert!(info.contains(&format!("Graphics adapter: {}", gpu.adapter)));
        assert!(info.contains(&format!("Driver: {}", gpu.driver)));
        assert!(info.contains("Backend: vulkan\nDevice type: discreteGpu"));
        assert!(info.contains("Canvas renderer: GPU"));
        assert!(info.contains("Theme: Studio (Light)"));
        assert!(info.contains("UI pixels per point: 1.50"));
        assert!(!url.contains('\n'));
        assert!(!url.contains('#'));
        assert!(url.contains("%0A"));
    }

    #[test]
    fn bug_report_uses_product_build_and_actual_runtime_info_not_private_diagnostics() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.perf.gpu_info.adapter = "Actual runtime GPU".into();
        app.perf.gpu_info.backend = "metal".into();
        app.perf.gpu_info.driver = "Actual driver 1.2".into();
        app.perf.gpu_info.canvas = "cpu".into();
        let private = "private-user@private-host /home/private-user/secret-document.psd TOKEN=secret";
        app.perf.gpu_info.fallback = Some(private.into());
        app.perf.gpu_info.lost = Some(private.into());
        app.perf.gpu_info.preference = private.into();
        app.perf.gpu_info.selected = private.into();
        app.ui.status = private.into();
        app.ui.theme = crate::theme::ThemeKind::Classic;
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(2.0);
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        let fields = report_fields(&issue_report_url(&app, &ctx, "bug_report.yml"));
        assert_eq!(fields["version"], photocraft_engine::build_info::long_version());
        assert_eq!(fields["os"], format!("{} {}", std::env::consts::OS, std::env::consts::ARCH));
        let info = &fields["system-info"];
        assert!(info.contains("Graphics adapter: Actual runtime GPU"));
        assert!(info.contains("Backend: metal"));
        assert!(info.contains("Driver: Actual driver 1.2"));
        assert!(info.contains("Canvas renderer: CPU"));
        assert!(info.contains("Theme: Classic"));
        assert!(info.contains("UI pixels per point: 2.00"));
        assert!(info.contains("Startup GPU fallback: yes"));
        assert!(info.contains("GPU lost or faulted this session: yes"));
        for secret in ["private-user", "private-host", "secret-document", "TOKEN", private] {
            assert!(!fields.values().any(|value| value.contains(secret)));
        }
    }

    #[test]
    fn bug_report_without_adapter_does_not_invent_hardware_and_bounds_unicode_url() {
        let gpu = crate::gpu_canvas::GpuInfo::default();
        let fields = report_fields(&report_url("bug_report.yml", "1.0", "unknown", "wasm32", &gpu, "Pro", 1.0));
        assert!(!fields["system-info"].contains("Graphics adapter:"));
        assert!(!fields["system-info"].contains("Driver:"));
        assert!(fields["system-info"].contains("Canvas renderer: CPU"));
        let long = "光".repeat(10_000);
        let gpu =
            crate::gpu_canvas::GpuInfo { adapter: long.clone(), backend: long.clone(), device_type: long.clone(), driver: long.clone(), ..Default::default() };
        let url = report_url("bug_report.yml", &long, &long, &long, &gpu, &long, 1.0);
        assert!(url.len() < 2048, "Encoded URL is {} bytes", url.len());
        let fields = report_fields(&url);
        assert!(fields["system-info"].contains("Canvas renderer: CPU"));
    }

    #[test]
    fn bug_report_failed_browser_service_falls_back_to_new_tab() {
        let services = crate::Services { open_url: Some(Box::new(|_| Err("browser unavailable".into()))), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let expected = issue_report_url(&app, &ctx, "bug_report.yml");
        let result = report_issue(&mut app, &ctx, "bug_report.yml");
        assert_eq!(result["url"], expected);
        ctx.output(|output| {
            let urls: Vec<_> =
                output.commands.iter().filter_map(|command| if let egui::OutputCommand::OpenUrl(url) = command { Some(url) } else { None }).collect();
            assert_eq!(urls.len(), 1);
            assert_eq!(urls[0].url, expected);
            assert!(urls[0].new_tab);
        });
    }

    #[test]
    fn links_open_through_the_platform_service() {
        // Issue #14: links (Help menu, Discord button, start-page links) must open in the browser.
        // They route through the `open_url` service rather than the unreliable `ctx.open_url`.
        use std::sync::{Arc, Mutex};
        let opened: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let rec = opened.clone();
        let services = crate::Services {
            open_url: Some(Box::new(move |u: &str| {
                rec.lock().unwrap().push(u.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        // Every Help-menu link id reaches the service with its URL.
        for (id, url) in COMMANDS {
            crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
            assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(*url), "{id}");
        }
        // The direct open() helper (Discord button / start-page links) also uses it.
        open(&mut app, &ctx, DISCORD);
        assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(DISCORD));
    }

    #[test]
    fn help_menu_lists_links_then_separator_then_system_info_and_about() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let help: Vec<String> = crate::menus::menu_items(&app).into_iter().filter(|i| i.path == ["Help"]).map(|i| i.id).collect();
        assert_eq!(help, ["help.discord", "help.website", "help.artcraftWebsite", "help.github", "help.reportIssue", "---", "help.systemInfo", "help.about"]);
        for (id, _) in COMMANDS {
            assert!(crate::menus::is_live(id) && crate::menus::is_enabled(&app, id), "{id}");
        }
    }

    #[test]
    fn link_commands_open_their_urls() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        for (id, url) in [
            ("help.discord", "https://discord.gg/artcraft"),
            ("help.website", "https://getartcraft.com/apps/photocraft"),
            ("help.github", "https://github.com/storytold/photocraft"),
        ] {
            let r = crate::menus::invoke(&mut app, &ctx, id, serde_json::json!({})).unwrap();
            assert_eq!(r["url"], url);
            assert_eq!(app.ui.status, format!("Opened {url}"));
        }
    }
}
