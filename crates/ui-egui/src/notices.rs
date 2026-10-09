//! Non-blocking notices: a small stack of cards in the lower-right corner of the window for things
//! the user should see but that must not interrupt work (import/export warnings such as "adjustment
//! flattened", files that couldn't be opened). Notices live in [`UiState`](crate::UiState) so the
//! control channel can read them (`ui.inspect`) and clear them (`ui.set`).

use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

const WAYLAND_FILE_DROP_DISMISSED: &str = "ui.waylandFileDropGuidanceDismissed";
/// At most this many notices are kept; older ones drop off.
pub const MAX_NOTICES: usize = 3;
/// Lines shown per notice before "…and N more".
const MAX_LINES: usize = 8;
/// A full canvas refresh on the CPU compositor slower than this (ms) gets a notice.
const SLOW_CPU_REFRESH_MS: f64 = 1000.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// An error (warning colour) rather than an informational notice.
    #[serde(default)]
    pub error: bool,
    /// Preference key to set when this notice is dismissed.
    #[serde(default)]
    pub dismiss_pref: Option<String>,
    /// `{name}` values filled into the title and lines after they are translated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<(String, String)>,
}

impl Notice {
    /// `s` (the title or a line) in the current language, with [`Notice::args`] filled in.
    pub fn text(&self, s: &str) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::i18n::fmt(crate::i18n::t(s), &args)
    }
}

/// Show a notice (newest last); returns its id.
pub fn post(app: &mut PhotocraftApp, title: impl Into<String>, lines: Vec<String>, error: bool, dismiss_pref: Option<&str>) -> u64 {
    let id = app.ui.alloc_id();
    app.ui.notices.push(Notice { id, title: title.into(), lines, error, dismiss_pref: dismiss_pref.map(str::to_owned), args: Vec::new() });
    cap_notices(app);
    id
}

fn cap_notices(app: &mut PhotocraftApp) {
    while app.ui.notices.len() > MAX_NOTICES {
        let oldest_temporary = app.ui.notices.iter().position(|notice| notice.dismiss_pref.is_none()).unwrap_or(0);
        app.ui.notices.remove(oldest_temporary);
    }
}

/// How to paste in hints: Edit › Paste's effective shortcut (`Ctrl+V`), else the menu path.
pub(crate) fn paste_hint(app: &PhotocraftApp) -> String {
    crate::shortcuts::shortcut_label(app, "edit.paste").unwrap_or_else(|| tl!("Edit › Paste").to_string())
}

/// Show the Wayland-specific fallback guidance unless the user dismissed it in preferences: winit
/// 0.30 delivers no file drops on Wayland, so point at File › Open, pasting a copied file, and the
/// command that starts this install under XWayland (where drops work), when there is one.
pub fn wayland_file_drop_guidance(app: &mut PhotocraftApp) {
    if !app.services.is_wayland
        || app.session.prefs().dialogs.get(WAYLAND_FILE_DROP_DISMISSED).and_then(serde_json::Value::as_bool) == Some(true)
        || app.ui.notices.iter().any(|notice| notice.dismiss_pref.as_deref() == Some(WAYLAND_FILE_DROP_DISMISSED))
    {
        return;
    }
    // English templates, translated and filled in when drawn, so the notice follows a language change.
    let mut lines = vec![
        "Native file drag-and-drop is not supported on Wayland yet. Use File › Open, or copy the image in your file manager and paste it with {paste}."
            .to_owned(),
    ];
    let mut args = vec![("paste".to_owned(), paste_hint(app))];
    if let Some(command) = &app.services.xwayland_command {
        lines.push("To drop files, start PhotoCraft under XWayland: `{command}`".to_owned());
        args.push(("command".to_owned(), command.clone()));
        lines.push("Or set Preferences › Performance › Linux display server to X11: PhotoCraft then always starts under XWayland.".to_owned());
    }
    let id = post(app, "Native file drag-and-drop is unavailable", lines, false, Some(WAYLAND_FILE_DROP_DISMISSED));
    if let Some(notice) = app.ui.notices.iter_mut().find(|notice| notice.id == id) {
        notice.args = args;
    }
}

/// A full refresh of document `doc` fell back to the CPU compositor (`reason`, the GPU's) and took
/// `ms`. The canvas doesn't respond while it runs, so once per document say why it was slow and
/// which edits will do it again (#1015).
pub fn slow_cpu_refresh(app: &mut PhotocraftApp, doc: u64, ms: f64, reason: Option<&str>) {
    if ms.is_nan() || ms < SLOW_CPU_REFRESH_MS || app.ui.slow_refresh_noticed.contains(&doc) {
        return;
    }
    app.ui.slow_refresh_noticed.push(doc);
    let seconds = format!("{:.1}", ms / 1000.0);
    let mut lines = vec![
        crate::i18n::fmt(tl!("This redraw took {seconds} s on the CPU."), &[("seconds", &seconds)]),
        tl!("Edits that change the whole document, such as a layer's blend mode, opacity or visibility, redraw all of it.").into(),
    ];
    if reason == Some(crate::gpu_canvas::OVER_BUDGET) {
        lines.push(
            tl!("Its layers don't fit the GPU memory budget. Raising Memory usage in Preferences › Performance raises the budget, up to a quarter of this computer's memory.")
                .into(),
        );
    } else if let Some(r) = reason {
        lines.push(crate::i18n::fmt(tl!("The GPU compositor wasn't used: {reason}"), &[("reason", r)]));
    }
    post(app, tl!("Large document: redrawing on the CPU"), lines, false, None);
}

fn dismiss(app: &mut PhotocraftApp, id: u64) {
    let dismiss_pref = app.ui.notices.iter().find(|notice| notice.id == id).and_then(|notice| notice.dismiss_pref.clone());
    app.ui.notices.retain(|notice| notice.id != id);
    if let Some(key) = dismiss_pref {
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(key, serde_json::Value::Bool(true));
        });
    }
}

/// Report import/export `warnings` for the file operation `what` (e.g. "Opened a.psd"): the status
/// bar says how many there were, and a notice lists them. Nothing happens when there are none.
pub fn io_warnings(app: &mut PhotocraftApp, what: &str, warnings: &[String]) {
    let Some(first) = warnings.first() else { return };
    let n = warnings.len();
    app.ui.status = if n == 1 { format!("{what}: {first}") } else { format!("{what} with {n} warnings: {first} …") };
    app.ui.status_error = true;
    post(app, format!("{what} with {n} warning{}", if n == 1 { "" } else { "s" }), warnings.to_vec(), false, None);
}

/// Report a failed file operation: the status bar shows it as an error and a notice keeps it on
/// screen until dismissed.
pub fn error(app: &mut PhotocraftApp, message: String) {
    app.ui.status = message.clone();
    app.ui.status_error = true;
    post(app, message, Vec::new(), true, None);
}

/// Draw the notices; each has a close button.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.notices.is_empty() {
        return;
    }
    let t = crate::theme::Tokens::get(ctx);
    let mut dismiss_id = None;
    // Clear the status bar (~24 px) and leave the dock's edge some air.
    egui::Area::new(egui::Id::new("photocraft-notices"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0))
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_max_width(360.0);
            for n in &app.ui.notices {
                egui::Frame::NONE
                    .fill(t.card)
                    .stroke(egui::Stroke::new(1.0, if n.error { t.warning } else { t.card_border }))
                    .corner_radius(t.radius)
                    .inner_margin(egui::Margin::same(10))
                    .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: t.shadow })
                    .show(ui, |ui| {
                        ui.set_width(340.0);
                        ui.horizontal(|ui| {
                            let title = egui::RichText::new(n.text(&n.title)).strong().color(if n.error { t.warning } else { t.text });
                            ui.add(egui::Label::new(title).wrap());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                if ui.add(egui::Button::new(egui::RichText::new("×").color(t.text_dim)).frame(false)).on_hover_text(tl!("Dismiss")).clicked() {
                                    dismiss_id = Some(n.id);
                                }
                            });
                        });
                        for line in n.lines.iter().take(MAX_LINES) {
                            let line = n.text(line);
                            ui.add(egui::Label::new(egui::RichText::new(format!("• {line}")).color(t.text_dim)).wrap());
                        }
                        if n.lines.len() > MAX_LINES {
                            let text = crate::i18n::fmt(tl!("…and {n} more"), &[("n", &(n.lines.len() - MAX_LINES).to_string())]);
                            ui.label(egui::RichText::new(text).color(t.text_faint));
                        }
                    });
                ui.add_space(6.0);
            }
        });
    if let Some(id) = dismiss_id {
        dismiss(app, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PhotocraftApp, Services};
    use photocraft_engine::Session;
    use std::sync::{Arc, Mutex};

    #[test]
    fn wayland_guidance_follows_language_changes_after_startup() {
        let command = Some("WAYLAND_DISPLAY= photocraft".to_string());
        let app = PhotocraftApp::new(Session::new(), Services { is_wayland: true, xwayland_command: command, ..Default::default() });
        let notice = &app.ui.notices[0];
        assert!(notice.lines.iter().all(|line| crate::i18n::tr(crate::i18n::Lang::from_code("es").unwrap(), line) != line), "every line is a catalog template");
        // the notice keeps its English source text and is translated when drawn (`i18n::t` with
        // the current language); `tr` with an explicit language leaves the process-wide language,
        // which other tests running in parallel draw with, untouched
        let es = crate::i18n::Lang::from_code("es").unwrap();
        assert_eq!(crate::i18n::tr(es, &notice.title), "El arrastrar y soltar archivos de forma nativa no está disponible en Wayland");
        assert_ne!(crate::i18n::tr(es, &notice.lines[0]), notice.lines[0]);
        assert_eq!(crate::i18n::tr(crate::i18n::Lang::EN, &notice.title), "Native file drag-and-drop is unavailable");
    }

    /// A notice's lines in English with their arguments filled in.
    fn english(notice: &Notice) -> String {
        let args: Vec<(&str, &str)> = notice.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        notice.lines.iter().map(|line| crate::i18n::fmt(line, &args)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn wayland_guidance_is_only_shown_in_wayland_sessions() {
        let command = Some("WAYLAND_DISPLAY= '/apps/Photo Craft.AppImage'".to_string());
        let mut app = PhotocraftApp::new(Session::new(), Services { is_wayland: true, xwayland_command: command, ..Default::default() });
        assert_eq!(app.ui.notices.len(), 1);
        assert_eq!(app.ui.notices[0].title, "Native file drag-and-drop is unavailable");
        let guidance = english(&app.ui.notices[0]);
        assert!(guidance.contains("not supported on Wayland yet"));
        assert!(guidance.contains("File › Open"));
        // Pasting a copied file works on Wayland (#338), so the notice offers it.
        let paste = paste_hint(&app);
        assert!(guidance.contains(&format!("paste it with {paste}")), "{guidance}");
        // The relaunch command is the one for this install (an AppImage here), not a guess.
        assert!(guidance.contains("under XWayland: `WAYLAND_DISPLAY= '/apps/Photo Craft.AppImage'`"), "{guidance}");
        assert!(guidance.contains("Linux display server to X11"), "{guidance}");
        assert_eq!(app.ui.notices[0].dismiss_pref.as_deref(), Some(WAYLAND_FILE_DROP_DISMISSED));
        for i in 0..MAX_NOTICES {
            post(&mut app, format!("Transient {i}"), Vec::new(), false, None);
        }
        assert_eq!(app.ui.notices.len(), MAX_NOTICES);
        assert!(app.ui.notices.iter().any(|notice| notice.dismiss_pref.is_some()));

        let app = PhotocraftApp::new(Session::new(), Services::default());
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn wayland_guidance_without_an_x_server_does_not_suggest_xwayland() {
        let app = PhotocraftApp::new(Session::new(), Services { is_wayland: true, ..Default::default() });
        let guidance = english(&app.ui.notices[0]);
        assert!(guidance.contains("File › Open"));
        assert!(!guidance.contains("XWayland"), "{guidance}");
    }

    #[test]
    fn paste_hint_follows_the_shortcut_and_falls_back_to_the_menu() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        assert_eq!(paste_hint(&app), crate::shortcuts::shortcut_label(&app, "edit.paste").unwrap());
        app.session.prefs.edit(|prefs| {
            prefs.shortcuts.insert("edit.paste".into(), String::new());
        });
        assert_eq!(paste_hint(&app), "Edit › Paste");
    }

    #[test]
    fn dismissing_wayland_guidance_persists_across_launches() {
        let saved = Arc::new(Mutex::new(None::<String>));
        let load_store = saved.clone();
        let save_store = saved.clone();
        let services = Services {
            is_wayland: true,
            load_prefs: Some(Box::new(move || load_store.lock().unwrap_or_else(|e| e.into_inner()).clone())),
            save_prefs: Some(Box::new(move |text| {
                *save_store.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        let id = app.ui.notices[0].id;
        dismiss(&mut app, id);
        crate::prefs_ui::tick(&mut app, &egui::Context::default());

        let services =
            Services { is_wayland: true, load_prefs: Some(Box::new(move || saved.lock().unwrap_or_else(|e| e.into_inner()).clone())), ..Default::default() };
        let app = PhotocraftApp::new(Session::new(), services);
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn a_slow_cpu_refresh_is_explained_once_per_document() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        slow_cpu_refresh(&mut app, 1, 400.0, Some(crate::gpu_canvas::OVER_BUDGET));
        slow_cpu_refresh(&mut app, 1, f64::NAN, Some(crate::gpu_canvas::OVER_BUDGET));
        assert!(app.ui.notices.is_empty(), "a quick refresh needs no explanation");

        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(crate::gpu_canvas::OVER_BUDGET));
        assert_eq!(app.ui.notices.len(), 1);
        let text = app.ui.notices[0].lines.join(" ");
        assert!(text.contains("55.0 s"), "{text}");
        assert!(text.contains("blend mode, opacity or visibility"), "{text}");
        assert!(text.contains("Memory usage in Preferences › Performance"), "{text}");
        assert!(!app.ui.notices[0].error);

        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(crate::gpu_canvas::OVER_BUDGET));
        assert_eq!(app.ui.notices.len(), 1, "once per document");

        slow_cpu_refresh(&mut app, 2, 3_000.0, Some("Blend If on `Overlay` (composited on the CPU)"));
        assert_eq!(app.ui.notices.len(), 2);
        let text = app.ui.notices[1].lines.join(" ");
        assert!(text.contains("Blend If on `Overlay`"), "{text}");
        assert!(!text.contains("Memory usage"), "the budget hint is only for the budget fallback: {text}");
    }

    #[test]
    fn dismissing_a_notice_writes_its_own_preference_key() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Dismissible", Vec::new(), false, Some("ui.testNoticeDismissed"));
        let id = app.ui.notices[0].id;
        dismiss(&mut app, id);
        assert_eq!(app.session.prefs().dialogs.get("ui.testNoticeDismissed").and_then(serde_json::Value::as_bool), Some(true));
    }
}
