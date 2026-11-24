//! Community and project links: the Help menu, the About dialog, the start screen and the title
//! bar all open these. URLs follow the crafting-app pattern (`getartcraft.com/apps/<app>`,
//! `github.com/storytold/<app>`).

use serde_json::{Value, json};

use crate::PhotocraftApp;

pub const DISCORD: &str = "https://discord.gg/artcraft";
pub const ARTCRAFT_WEBSITE: &str = "https://getartcraft.com";
pub const APP_PAGE: &str = "https://getartcraft.com/apps/photocraft";
pub const TUTORIALS: &str = "https://getartcraft.com/tutorials";
pub const GITHUB: &str = "https://github.com/storytold/photocraft";
pub const ISSUES: &str = "https://github.com/storytold/photocraft/issues";

/// Help-menu link commands: (id, url). Labels live in `menus::UI_COMMANDS`.
pub const COMMANDS: &[(&str, &str)] =
    &[("help.discord", DISCORD), ("help.website", APP_PAGE), ("help.artcraftWebsite", ARTCRAFT_WEBSITE), ("help.github", GITHUB), ("help.reportIssue", ISSUES)];

pub fn url_for(id: &str) -> Option<&'static str> {
    static URLS: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static str>> = std::sync::OnceLock::new();
    URLS.get_or_init(|| {
        let mut urls = std::collections::HashMap::new();
        for &(id, url) in COMMANDS {
            urls.entry(id).or_insert(url);
        }
        urls
    })
    .get(id)
    .copied()
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
    install_pass_hooks(ctx);
    ctx.copy_text(crate::gpu_status::diagnostics_report(app).join("\n"));
    if app.services.open_url.is_none() {
        return open_issues(app, ctx);
    }
    // In a UI pass the copy goes out when this root frame ends; a hidden window's logic-only
    // call hands its output over as it returns. The frame is kept either way, so a pass not yet
    // marked as one (hooks installed during this pass) still waits for the next frame.
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| d.insert_temp(pending_id(), frame));
    ctx.request_repaint();
    json!({"url": ISSUES})
}

/// Each `logic` call, visible or hidden, before control commands run: open the issues page
/// [`report_issue`] queued in an earlier call, once its output (the copy) has been delivered.
/// Not in a later pass of the frame that copied.
pub fn open_pending(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(queued) = ctx.data(|d| d.get_temp::<u64>(pending_id())) else { return };
    // A logic-only call runs after every earlier call's output was handed over.
    let delivered = !in_ui_pass(ctx) || ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) > queued;
    if !delivered {
        ctx.request_repaint();
        return;
    }
    ctx.data_mut(|d| d.remove::<u64>(pending_id()));
    open_issues(app, ctx);
}

fn pending_id() -> egui::Id {
    egui::Id::unique("links::report_issue")
}

fn ui_pass_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("links::ui_pass").with(viewport)
}

/// Mark UI passes apart from a hidden window's logic-only calls (`Context::run_logic`), which
/// run `logic` with no pass around it. Once per Context; installed from inside a running pass,
/// it marks the passes after it (egui is already iterating that pass's begin hooks). The hooks
/// retain neither the app nor the Context.
pub(crate) fn install_pass_hooks(ctx: &egui::Context) {
    let installed = egui::Id::unique("links::pass_hooks");
    if ctx.data(|d| d.get_temp::<bool>(installed)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(installed, true));
    ctx.on_begin_pass(
        "photocraft report-issue UI pass",
        std::sync::Arc::new(|ui| {
            let key = ui_pass_id(ui.ctx().viewport_id());
            ui.ctx().data_mut(|d| d.insert_temp(key, true));
        }),
    );
    ctx.on_end_pass(
        "photocraft report-issue UI pass end",
        std::sync::Arc::new(|ui| {
            let key = ui_pass_id(ui.ctx().viewport_id());
            ui.ctx().data_mut(|d| d.insert_temp(key, false));
        }),
    );
}

fn in_ui_pass(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(ui_pass_id(ctx.viewport_id()))).unwrap_or(false)
}

fn open_issues(app: &mut PhotocraftApp, ctx: &egui::Context) -> Value {
    let r = open(app, ctx, ISSUES);
    let template = "Opened {url}. System Info (graphics adapter, render path, slow frames) was copied to the clipboard: paste it into the issue.";
    app.ui.status = crate::i18n::fmt(tl!(template), &[("url", ISSUES)]);
    r
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
        let ctx = app.ctx().clone();
        // Every Help-menu link id reaches the service with its URL.
        for (id, url) in COMMANDS {
            crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
            // Report an Issue opens on the next call, once its copy is delivered
            // (`report_issue_copies_the_diagnostics_first`); outside a pass that is this one.
            if *id == "help.reportIssue" {
                open_pending(&mut app, &ctx);
            }
            assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(*url), "{id}");
        }
        // The direct open() helper (Discord button / start-page links) also uses it.
        open(&mut app, &ctx, DISCORD);
        assert_eq!(opened.lock().unwrap().last().map(String::as_str), Some(DISCORD));
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
        // Hooks from an earlier report: they mark UI passes apart from a hidden window's
        // logic-only calls from the first pass on.
        install_pass_hooks(&ctx);
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
        assert!(app.ui.status.contains("System Info (graphics adapter, render path, slow frames) was copied to the clipboard"), "{}", app.ui.status);

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

    /// The first report installs the pass hooks during its pass: that pass isn't marked, the
    /// next pass of the same frame is, and the page still waits for the frame's copy.
    #[test]
    fn report_issue_waits_for_its_frame_when_hooks_arrive_late() {
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
        ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(2).expect("two passes"));
        let mut passes = 0;
        ctx.run_ui(Default::default(), |ui| {
            open_pending(&mut app, ui.ctx());
            if passes == 0 {
                crate::menus::invoke(&mut app, ui.ctx(), "help.reportIssue", json!({})).unwrap();
                ui.ctx().request_discard("a second pass of the frame that copied");
            }
            passes += 1;
        })
        .textures_delta
        .clear();
        assert_eq!(passes, 2);
        assert!(opened.lock().unwrap().is_empty(), "not in a later pass of the frame that copied");
        ctx.run_ui(Default::default(), |ui| open_pending(&mut app, ui.ctx())).textures_delta.clear();
        assert_eq!(opened.lock().unwrap().as_slice(), [ISSUES.to_string()]);
    }
}
