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
    let template = "Opened {url}";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("url", url)]);
    json!({"url": url})
}

/// Help › Report an Issue: copy the diagnostics report (Help › System Info ▸ Copy) to the
/// clipboard, then open the issues page, and say so in the status bar.
///
/// `copy_text` only queues the copy: the platform delivers it with this call's output. The
/// `open_url` service opens at once, so it waits for a later call ([`open_pending`]) and the page
/// never comes up before the clipboard holds the report. Without the service, `ctx.open_url`
/// queues behind the copy in the same output (and keeps the click's gesture on the web).
pub fn report_issue(app: &mut PhotocraftApp, ctx: &egui::Context) -> Value {
    ctx.copy_text(crate::gpu_status::diagnostics_report(app).join("\n"));
    if app.services.open_url.is_none() {
        return open_issues(app, ctx);
    }
    // In a UI pass the copy goes out when this root frame ends. A hidden window runs `logic`
    // alone, outside any pass, and its output goes out as that call returns: `None`.
    let frame = crate::type_tool::in_ui_pass(ctx).then(|| ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT));
    ctx.data_mut(|d| d.insert_temp(pending_id(), frame));
    ctx.request_repaint();
    json!({"url": ISSUES})
}

/// Each `logic` call, visible or hidden, before control commands run: open the issues page
/// [`report_issue`] queued in an earlier call, once its output (the copy) has been delivered.
/// Not in a later pass of the frame that copied.
pub fn open_pending(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(queued) = ctx.data(|d| d.get_temp::<Option<u64>>(pending_id())) else { return };
    // A logic-only call runs after every earlier call's output was handed over.
    let delivered = !crate::type_tool::in_ui_pass(ctx) || queued.is_none_or(|frame| ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) > frame);
    if !delivered {
        ctx.request_repaint();
        return;
    }
    ctx.data_mut(|d| d.remove::<Option<u64>>(pending_id()));
    open_issues(app, ctx);
}

fn pending_id() -> egui::Id {
    egui::Id::unique("links::report_issue")
}

fn open_issues(app: &mut PhotocraftApp, ctx: &egui::Context) -> Value {
    let r = open(app, ctx, ISSUES);
    let template = "Opened {url}. System Info was copied to the clipboard: paste it into the issue.";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("url", ISSUES)]);
    r
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
            // Report an Issue opens once its copy is delivered (`report_issue_copies_the_diagnostics_first`).
            if *id == "help.reportIssue" {
                ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
                open_pending(&mut app, &ctx);
            }
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
        assert_eq!(
            help,
            [
                "help.discord",
                "help.website",
                "help.artcraftWebsite",
                "help.github",
                "help.reportIssue",
                "help.checkForUpdates",
                "---",
                "help.systemInfo",
                "help.about"
            ]
        );
        for (id, _) in COMMANDS {
            assert!(crate::menus::is_live(id) && crate::menus::is_enabled(&app, id), "{id}");
        }
    }

    #[test]
    fn report_issue_copies_the_diagnostics_first() {
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
        app.perf.gpu_info.adapter = "Test GPU".into();
        let ctx = egui::Context::default();
        // As the app's creator does: marks UI passes apart from a hidden window's logic-only calls.
        PhotocraftApp::install_type_reset_hooks(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| {
            crate::menus::invoke(&mut app, ui.ctx(), "help.reportIssue", json!({})).unwrap();
            // A later pass of the same frame: its output (the copy) isn't delivered yet.
            open_pending(&mut app, ui.ctx());
        });
        // No renderer here: drop the frame's font atlas upload.
        out.textures_delta.clear();
        let copied = out.platform_output.commands.iter().find_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None });
        assert_eq!(copied, Some(crate::gpu_status::diagnostics_report(&app).join("\n")));
        assert!(copied.is_some_and(|t| t.contains("Graphics adapter: Test GPU")));
        assert!(opened.lock().unwrap().is_empty(), "the page waits for the copy to be delivered");
        // The next frame, after the platform handled the copy, opens it.
        ctx.run_ui(Default::default(), |ui| open_pending(&mut app, ui.ctx())).textures_delta.clear();
        assert_eq!(opened.lock().unwrap().as_slice(), [ISSUES.to_string()], "the URL is unchanged, opened once");
        ctx.run_ui(Default::default(), |ui| open_pending(&mut app, ui.ctx())).textures_delta.clear();
        assert_eq!(opened.lock().unwrap().len(), 1);
        assert!(app.ui.status.contains("System Info was copied to the clipboard"), "{}", app.ui.status);

        // Hidden window: eframe runs `logic` alone (`run_logic`), no UI pass and no new frame. An
        // agent's `help.reportIssue` there copies in that call's output and opens on the next call.
        let logic = |app: &mut PhotocraftApp, f: &dyn Fn(&mut PhotocraftApp, &egui::Context)| ctx.run_logic(&Default::default(), |ctx| f(app, ctx));
        let out = logic(&mut app, &|app, ctx| {
            crate::menus::invoke(app, ctx, "help.reportIssue", json!({})).unwrap();
        });
        assert!(out.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))), "a hidden window still delivers the copy");
        assert_eq!(opened.lock().unwrap().len(), 1, "not before that call's output is delivered");
        logic(&mut app, &|app, ctx| open_pending(app, ctx));
        assert_eq!(opened.lock().unwrap().len(), 2, "opens while still hidden");
        logic(&mut app, &|app, ctx| open_pending(app, ctx));
        assert_eq!(opened.lock().unwrap().len(), 2, "once");

        // Copied in a UI pass, then hidden: that frame ended before the logic-only call, so it opens.
        ctx.run_ui(Default::default(), |ui| {
            crate::menus::invoke(&mut app, ui.ctx(), "help.reportIssue", json!({})).unwrap();
        })
        .textures_delta
        .clear();
        logic(&mut app, &|app, ctx| open_pending(app, ctx));
        assert_eq!(opened.lock().unwrap().len(), 3);
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
