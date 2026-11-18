//! Non-blocking notices: toast cards for things the user should see but that must not interrupt
//! work (import/export warnings such as "adjustment flattened", files that couldn't be opened).
//! Notices are posted outside any pass (engine results, file loads), where no `egui::Context`
//! exists, so [`UiState::notices`](crate::UiState) is an outbox of the notices posted since the last
//! [`show`]; `show` hands them to the window's toast queue, which owns the cards, their timers and
//! the hover pause. [`Live`] remembers which queued toast belongs to which notice.

use craft::theme::variant::Tone;
use craft::ui::notices::{NoticeCenter, NoticeEnd, PostedNotice};
use craft::ui::runtime::toast::ToastId;
use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

const CLIPBOARD_UNREADABLE: &str = "Pasting from other apps is unavailable";
/// A full canvas refresh on the CPU compositor slower than this (ms) gets a notice.
const SLOW_CPU_REFRESH_MS: f64 = 1000.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// The tone the notice reads in: `Tone::Bad` for a failure, `Tone::Neutral` for information.
    /// Not serialized: the framework's `Tone` has no serde form.
    #[serde(skip)]
    pub tone: Tone,
    /// Preference key to set when this notice is dismissed.
    #[serde(default)]
    pub dismiss_pref: Option<String>,
    /// `{name}` values filled into the title and lines after they are translated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<(String, String)>,
    /// Stays until dismissed (or until whatever it warns of has passed), whatever the auto-hide
    /// setting: for a warning the user must not miss, such as nearly full memory.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sticky: bool,
}

impl Notice {
    /// `s` (the title or a line) in the current language, with [`Notice::args`] filled in.
    pub fn text(&self, s: &str) -> String {
        let args: Vec<(&str, &str)> = self.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        crate::i18n::fmt(crate::i18n::t(s), &args)
    }
}

/// Show a notice (newest last); returns its id.
pub fn post(app: &mut PhotocraftApp, title: impl Into<String>, lines: Vec<String>, tone: Tone, dismiss_pref: Option<&str>) -> u64 {
    let id = app.ui.alloc_id();
    app.ui.notices.push(Notice { id, title: title.into(), lines, tone, dismiss_pref: dismiss_pref.map(str::to_owned), args: Vec::new(), sticky: false });
    id
}

/// The notices handed to the window's toast queue: for each, the app id, its untranslated title
/// (the app's own once-while-shown checks compare titles) and the toast handle. Toasts the app
/// removed wait in `removals` for the next [`show`], which has the context to dismiss them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Live {
    posted: Vec<(u64, String, ToastId)>,
    removals: Vec<ToastId>,
}

/// Drop notice `id`, whether it still waits in the outbox or is already a toast.
pub fn remove(app: &mut PhotocraftApp, id: u64) {
    app.ui.notices.retain(|notice| notice.id != id);
    take_posted(app, |posted_id, _| posted_id == id);
}

/// Drop every notice with this untranslated `title`.
pub fn remove_titled(app: &mut PhotocraftApp, title: &str) {
    app.ui.notices.retain(|notice| notice.title != title);
    take_posted(app, |_, posted_title| posted_title == title);
}

/// Move the posted entries `matches` selects from the handle table to the removal list.
fn take_posted(app: &mut PhotocraftApp, matches: impl Fn(u64, &str) -> bool) {
    let live = &mut app.notices_live;
    let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut live.posted).into_iter().partition(|(id, title, _)| matches(*id, title));
    live.posted = kept;
    live.removals.extend(gone.into_iter().map(|(.., toast)| toast));
}

/// Whether a notice with this untranslated `title` is waiting or on screen.
pub fn shown_titled(app: &PhotocraftApp, title: &str) -> bool {
    app.ui.notices.iter().any(|notice| notice.title == title) || app.notices_live.posted.iter().any(|(_, posted, _)| posted == title)
}

/// What a test sees after full UI passes: the notices still in the outbox, then the window's toast
/// cards, each as (title, lines, tone).
#[cfg(test)]
pub(crate) fn shown(app: &PhotocraftApp, ctx: &egui::Context) -> Vec<(String, Vec<String>, Tone)> {
    use craft::ui::runtime::env::WindowKey;
    let waiting = app.ui.notices.iter().map(|n| (n.text(&n.title), n.lines.iter().map(|line| n.text(line)).collect(), n.tone));
    let cards = craft::ui::runtime::toast::cards(ctx, &WindowKey::main()).into_iter().map(|card| {
        let toast = card.toast;
        let title = toast.title.as_ref().unwrap_or(&toast.message).as_str().to_owned();
        (title, toast.lines.iter().map(|line| line.as_str().to_owned()).collect(), toast.tone)
    });
    waiting.chain(cards).collect()
}

/// A paste found no clipboard to read (#5031). In a Flatpak on Wayland the sandbox hides the
/// data-control protocol that other apps' images are read through, and there is no X server to
/// fall back to, so the clipboard may hold an image all the same. Say so, and give the command
/// that starts this install under XWayland, where the X11 clipboard works, when there is one.
pub fn clipboard_unreadable(app: &mut PhotocraftApp) {
    app.ui.status = tl!("Nothing to paste: PhotoCraft can't read the system clipboard here").into();
    app.ui.status_error = false;
    if shown_titled(app, CLIPBOARD_UNREADABLE) {
        return;
    }
    // English templates, translated when drawn, as in `wayland_file_drop_guidance`.
    let mut lines =
        vec!["PhotoCraft can't reach the system clipboard in this session. On Wayland, a Flatpak sandbox hides the protocol that images from other apps are read through.".to_owned()];
    let mut args = Vec::new();
    if let Some(command) = &app.services.xwayland_command {
        lines.push("To paste from other apps, start PhotoCraft under XWayland: `{command}`".to_owned());
        args.push(("command".to_owned(), command.clone()));
    }
    let id = post(app, CLIPBOARD_UNREADABLE, lines, Tone::Neutral, None);
    if let Some(notice) = app.ui.notices.iter_mut().find(|notice| notice.id == id) {
        notice.args = args;
    }
}

/// A full refresh of document `doc` fell back to the CPU compositor (`reason`, the GPU's) and took
/// `ms`. The canvas doesn't respond while it runs, so once per document say why it was slow and
/// which edits will do it again (#1015).
pub fn slow_cpu_refresh(app: &mut PhotocraftApp, doc: u64, ms: f64, reason: Option<&crate::render_path::CpuReason>) {
    if matches!(reason, Some(crate::render_path::CpuReason::CompositorBuilding))
        || ms.is_nan()
        || ms < SLOW_CPU_REFRESH_MS
        || app.ui.slow_refresh_noticed.contains(&doc)
    {
        return;
    }
    app.ui.slow_refresh_noticed.push(doc);
    let seconds = format!("{:.1}", ms / 1000.0);
    let mut lines = vec![
        crate::i18n::fmt(tl!("This redraw took {seconds} s on the CPU."), &[("seconds", &seconds)]),
        tl!("Edits that change the whole document, such as a layer's blend mode, opacity or visibility, redraw all of it.").into(),
    ];
    if let Some(&crate::render_path::CpuReason::OverBudget { needed, budget }) = reason {
        if needed > budget {
            lines.push(crate::i18n::fmt(
                tl!("Its layers need {needed} of GPU memory; the budget leaves them {budget}."),
                &[("needed", &crate::chrome_ui::fmt_bytes(needed)), ("budget", &crate::chrome_ui::fmt_bytes(budget))],
            ));
        }
        lines.push(
            tl!("Set GPU Memory in Preferences › Performance to what your graphics card can hold, at most 3/8 of its memory. Automatic follows Memory Usage, within a quarter of this computer's memory and 3/8 of the card's, and doesn't count the document against a card whose memory is known. The status bar shows GPU Memory Full while they don't fit.")
                .into(),
        );
    } else if let Some(r) = reason {
        lines.push(crate::i18n::fmt(tl!("The GPU compositor wasn't used: {reason}"), &[("reason", &r.to_string())]));
    }
    post(app, tl!("Large document: redrawing on the CPU"), lines, Tone::Neutral, None);
}

/// Report import/export `warnings` for the file operation `what` (e.g. "Opened a.psd"): the status
/// bar says how many there were, and a notice lists them. Nothing happens when there are none.
pub fn io_warnings(app: &mut PhotocraftApp, what: &str, warnings: &[String]) {
    let Some(first) = warnings.first() else { return };
    let n = warnings.len();
    let title = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n as u64, "{what} with {n} warning", "{what} with {n} warnings"), &[("what", what)]);
    app.ui.status = if n == 1 { format!("{what}: {first}") } else { format!("{title}: {first} …") };
    app.ui.status_error = true;
    post(app, title, warnings.to_vec(), Tone::Neutral, None);
}

/// Report the files a folder run (Batch, Image Processor, …) could not process, one line each:
/// the status bar shows the first as an error, and a notice titled with how many failed lists
/// them all until dismissed (#10682).
pub fn file_failures(app: &mut PhotocraftApp, what: &str, failures: &[String]) {
    let Some(first) = failures.first() else { return };
    let n = failures.len();
    let title = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n as u64, "{what}: {n} file failed", "{what}: {n} files failed"), &[("what", what)]);
    app.ui.status = if n == 1 { format!("{what}: {first}") } else { format!("{title}: {first} …") };
    app.ui.status_error = true;
    post(app, title, failures.to_vec(), Tone::Bad, None);
}

/// Report a failed file operation: the status bar shows it as an error and a notice keeps it on
/// screen until dismissed.
pub fn error(app: &mut PhotocraftApp, message: String) {
    app.ui.status = message.clone();
    app.ui.status_error = true;
    post(app, message, Vec::new(), Tone::Bad, None);
}

/// Disclose the salvage warnings a footage-loading command result carries (#11113): a GIF or
/// WebP damaged after some frames keeps its earlier frames and returns the decoder's warning in
/// `warnings`, which must reach the user like any other import warning. Nothing happens when
/// every frame came through.
pub fn footage_warnings(app: &mut PhotocraftApp, result: &serde_json::Value, what: &str) {
    io_result_warnings(app, result, what);
}

/// Surface import or update warnings returned by a successful command.
pub fn io_result_warnings(app: &mut PhotocraftApp, result: &serde_json::Value, what: &str) {
    let warnings: Vec<String> = result["warnings"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).map(str::to_owned).collect();
    io_warnings(app, what, &warnings);
}

/// Hand the outbox to the window's toast queue and collect how the queued notices ended. Auto-hide
/// (Interface › Notification) is the queue's timeout; the queue holds a card while the pointer rests
/// on it. A second pass in the same frame finds the outbox empty and the ends already reported, so
/// it posts and writes nothing. A measuring run of the shell's body does none of it: the queue skips
/// a post there, so the notice would leave the outbox without reaching the window; the real run posts it.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if craft::ui::runtime::measure::active(&ctx) {
        return;
    }
    let (auto_hide, seconds) = {
        let i = &app.session.prefs().interface;
        (i.notification_auto_hide, i.notification_duration_seconds.max(1) as f32)
    };
    // `NoticeCenter::timeout` takes seconds and cannot switch off; with auto-hide off every notice is posted sticky.
    let center = NoticeCenter::new().timeout(seconds);
    let silenced: Vec<String> = app.session.prefs().dialogs.iter().filter(|(_, value)| value.as_bool() == Some(true)).map(|(key, _)| key.clone()).collect();
    for notice in std::mem::take(&mut app.ui.notices) {
        let mut posted =
            PostedNotice::new(notice.text(&notice.title)).lines(notice.lines.iter().map(|line| notice.text(line)).collect::<Vec<_>>()).tone(notice.tone);
        if let Some(key) = &notice.dismiss_pref {
            posted = posted.dismiss_key(key.clone());
        }
        if notice.sticky || !auto_hide {
            posted = posted.sticky();
        }
        if let Some(toast) = center.post(&ctx, &posted, &silenced) {
            app.notices_live.posted.push((notice.id, notice.title, toast));
        }
    }
    for toast in std::mem::take(&mut app.notices_live.removals) {
        center.remove(&ctx, toast);
    }
    for end in center.show(ui).ends {
        match end {
            NoticeEnd::Closed(toast) | NoticeEnd::Expired(toast) | NoticeEnd::Dropped(toast) => forget(app, toast),
            NoticeEnd::Silenced { id, key, .. } => {
                forget(app, id);
                app.session.prefs.edit(|prefs| {
                    prefs.dialogs.insert(key, serde_json::Value::Bool(true));
                });
            }
            _ => {}
        }
    }
}

/// Drop the handle-table entry of a toast that left the screen.
fn forget(app: &mut PhotocraftApp, toast: ToastId) {
    app.notices_live.posted.retain(|(.., posted)| *posted != toast);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PhotocraftApp, Services};
    use photocraft_engine::Session;

    #[test]
    fn wayland_session_does_not_post_unavailable_notice() {
        let app = PhotocraftApp::new(Session::new(), Services { is_wayland: true, ..Default::default() });
        assert!(app.ui.notices.is_empty());
    }

    /// #5031: a paste that can't reach the clipboard (a Flatpak on Wayland) says why and how to
    /// paste anyway, once while shown; a clipboard that simply holds no image stays a quiet no-op.
    #[test]
    fn unreadable_clipboard_paste_explains_the_sandbox() {
        let command = "flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft";
        let services = |unreadable: bool| Services {
            clipboard_get_image: Some(Box::new(move || if unreadable { Err("no clipboard backend".into()) } else { Ok(None) })),
            clipboard_unreadable: Some(Box::new(move || unreadable)),
            xwayland_command: Some(command.to_string()),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services(true));
        let ctx = app.ctx().clone();
        for _ in 0..2 {
            let r = crate::menus::invoke(&mut app, &ctx, "edit.paste", serde_json::json!({})).unwrap();
            assert_eq!(r["pasted"], serde_json::json!(false));
        }
        assert_eq!(app.ui.status, "Nothing to paste: PhotoCraft can't read the system clipboard here");
        let shown: Vec<_> = app.ui.notices.iter().filter(|notice| notice.title == CLIPBOARD_UNREADABLE).collect();
        assert_eq!(shown.len(), 1, "a second paste doesn't stack another notice");
        assert!(shown_titled(&app, CLIPBOARD_UNREADABLE));
        let guidance = english(shown[0]);
        assert!(guidance.contains(&format!("under XWayland: `{command}`")), "{guidance}");

        let mut empty = PhotocraftApp::new(Session::new(), services(false));
        crate::menus::invoke(&mut empty, &ctx, "edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(empty.ui.status, "Nothing to paste: the clipboard holds no image");
        assert!(empty.ui.notices.iter().all(|notice| notice.title != CLIPBOARD_UNREADABLE));
    }

    /// A notice's lines in English with their arguments filled in.
    fn english(notice: &Notice) -> String {
        let args: Vec<(&str, &str)> = notice.args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        notice.lines.iter().map(|line| crate::i18n::fmt(line, &args)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn preparing_compositor_does_not_warn_about_large_documents() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        slow_cpu_refresh(&mut app, 1, 60_000.0, Some(&crate::render_path::CpuReason::CompositorBuilding));
        assert!(app.ui.notices.is_empty());
        assert!(app.ui.slow_refresh_noticed.is_empty());
    }

    #[test]
    fn a_slow_cpu_refresh_is_explained_once_per_document() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        let over = crate::render_path::CpuReason::OverBudget { needed: 3600 << 20, budget: 3400 << 20 };
        slow_cpu_refresh(&mut app, 1, 400.0, Some(&over));
        slow_cpu_refresh(&mut app, 1, f64::NAN, Some(&over));
        assert!(app.ui.notices.is_empty(), "a quick refresh needs no explanation");

        // The layers of document 1 need 3.6 GB and may hold 3.4 GB: the notice says so.
        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(&over));
        assert_eq!(app.ui.notices.len(), 1);
        let text = app.ui.notices[0].lines.join(" ");
        assert!(text.contains("55.0 s"), "{text}");
        assert!(text.contains("blend mode, opacity or visibility"), "{text}");
        assert!(text.contains("need 3.5G of GPU memory; the budget leaves them 3.3G"), "{text}");
        assert!(text.contains("GPU Memory in Preferences › Performance"), "{text}");
        assert_eq!(app.ui.notices[0].tone, Tone::Neutral);

        slow_cpu_refresh(&mut app, 1, 55_000.0, Some(&over));
        assert_eq!(app.ui.notices.len(), 1, "once per document");

        slow_cpu_refresh(&mut app, 2, 3_000.0, Some(&crate::render_path::CpuReason::Unsupported("Blend If on `Overlay` (composited on the CPU)".into())));
        assert_eq!(app.ui.notices.len(), 2);
        let text = app.ui.notices[1].lines.join(" ");
        assert!(text.contains("Blend If on `Overlay`"), "{text}");
        assert!(!text.contains("Memory usage"), "the budget hint is only for the budget fallback: {text}");
    }

    #[test]
    fn import_warning_punctuation_is_localized_without_changing_english() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        // English in every build; Czech too when the craft-i18n catalogs are embedded.
        let czech = crate::i18n::needs_catalogs().then(|| crate::i18n::Lang::from_code("cs").expect("cs registered"));
        for lang in std::iter::once(crate::i18n::Lang::EN).chain(czech) {
            crate::i18n::with_language(lang, || {
                let what = crate::i18n::fmt(tl!("Opened {name}"), &[("name", "x.psd")]);
                let warnings = vec!["first warning".into(), "second warning".into(), "third warning".into()];
                io_warnings(&mut app, &what, &warnings);
                if lang == crate::i18n::Lang::EN {
                    assert_eq!(app.ui.status, "Opened x.psd with 3 warnings: first warning …");
                }
                let title = crate::i18n::fmt(&crate::i18n::trn(lang, 3, "{what} with {n} warning", "{what} with {n} warnings"), &[("what", &what)]);
                assert_eq!(app.ui.status, format!("{title}: first warning …"));
                assert_eq!(app.ui.notices.last().unwrap().lines, warnings);
                io_warnings(&mut app, &what, &warnings[..1]);
                assert_eq!(app.ui.status, format!("{what}: first warning"));
            });
        }
    }
}
