//! Telling the user why a launch failed when there is no window to say it in (#7643).
//!
//! A desktop launch has no console — release builds are GUI-subsystem apps on Windows — so the
//! `Err` that ends `main` used to vanish and PhotoCraft "just closed". This shows one native
//! dialog with the error, the renderer that was tried and the log file to attach. The text is
//! English diagnostics, like Help › System Info, so a bug report reads the same in every language.
//!
//! Terminal and automation launches keep the old behaviour: they can read standard error or the
//! exit status, and a modal dialog would leave a script waiting.

use std::io::IsTerminal;
use std::path::Path;

/// Whether a failed launch needs the dialog: no terminal under standard error, no control client
/// (an agent launch reads the exit status instead of clicking OK), and a screen to put it on.
pub fn dialog_wanted(stderr_is_terminal: bool, automation: bool, display: bool) -> bool {
    !stderr_is_terminal && !automation && display
}

/// Whether this launch asks for the dialog (see [`dialog_wanted`]).
pub fn needs_dialog(automation: bool) -> bool {
    dialog_wanted(std::io::stderr().is_terminal(), automation, has_display())
}

/// Whether there is a screen for a dialog: always on Windows and macOS; on Linux a headless box
/// (a CI runner, an SSH session — `startup_without_display` removes both variables) has none.
fn has_display() -> bool {
    #[cfg(target_os = "linux")]
    {
        ["DISPLAY", "WAYLAND_DISPLAY"].iter().any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// The dialog's text: the error, the renderer that was tried and why it wasn't the preference,
/// the version and, when one was opened, the log file.
pub fn text(error: &str, renderer: &str, note: Option<&str>, version: &str, log: Option<&Path>) -> String {
    body("PhotoCraft could not start.", error, renderer, note, version, log)
}

/// [`text`] for a window that had been open: the app started fine and then lost its window (a
/// display-server disconnect, for example), so "could not start" would send a bug report down
/// the wrong path. Crash recovery can only offer what autosave had written before the window
/// was lost, so the wording stays conditional.
pub fn closed_text(error: &str, renderer: &str, note: Option<&str>, version: &str, log: Option<&Path>) -> String {
    body("PhotoCraft closed unexpectedly. Documents with unsaved changes may be offered for recovery at the next start.", error, renderer, note, version, log)
}

fn body(headline: &str, error: &str, renderer: &str, note: Option<&str>, version: &str, log: Option<&Path>) -> String {
    let mut text = format!("{headline}\n\n{error}\n\nRenderer: {renderer}");
    if let Some(note) = note {
        text.push_str(&format!("\nStartup: {note}"));
    }
    text.push_str(&format!("\nVersion: {version}"));
    if let Some(path) = log {
        text.push_str(&format!("\nLog file: {}", path.display()));
    }
    text
}

/// How a launch that ended with an error is reported: after the window had shown its first
/// frames it closed unexpectedly; before that it could not start.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    /// The dialog's title, translated (each wording is a literal at its call, for the catalogs).
    pub title: &'static str,
    /// The dialog's text: [`closed_text`] or [`text`].
    pub body: String,
    /// Which dialog, for the log.
    pub kind: &'static str,
}

/// The log line's headline for a launch that ended with an error, in English like the rest of
/// the log; `started`: the window had shown its first frames.
pub fn log_headline(started: bool) -> &'static str {
    if started { "PhotoCraft closed unexpectedly" } else { "PhotoCraft could not start" }
}

/// The [`Report`] for a launch that ended with `error`; `started`: the window had shown its first
/// frames.
pub fn report(started: bool, error: &str, renderer: &str, note: Option<&str>, version: &str, log: Option<&Path>) -> Report {
    if started {
        Report {
            title: photocraft_ui_egui::i18n::t("PhotoCraft closed unexpectedly"),
            body: closed_text(error, renderer, note, version, log),
            kind: "unexpected-exit",
        }
    } else {
        Report { title: photocraft_ui_egui::i18n::t("PhotoCraft could not start"), body: text(error, renderer, note, version, log), kind: "startup-failure" }
    }
}

/// Show the [`report`] for a launch that ended with `error` in a native error dialog: "could not
/// start" when the window never showed its first frames (`started` false), "closed unexpectedly"
/// when it did. Called after the event loop returned, so nothing is repainting any more.
pub fn show(error: &str, renderer: &str, note: Option<&str>, log: Option<&Path>, started: bool) {
    let version = photocraft_engine::build_info::long_version();
    let report = report(started, error, renderer, note, &version, log);
    log::info!("showing the {} dialog", report.kind);
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(report.title)
        .set_description(report.body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_launch_asks_the_user_and_a_script_does_not() {
        assert!(dialog_wanted(false, false, true), "no console: the error would vanish");
        assert!(!dialog_wanted(true, false, true), "a terminal shows the error itself");
        assert!(!dialog_wanted(false, true, true), "an automation client reads the exit status");
        assert!(!dialog_wanted(false, false, false), "a headless launch has no screen for a dialog");
    }

    #[test]
    fn the_text_names_the_error_renderer_version_and_log() {
        let log = Path::new("/home/me/.config/photocraft/logs/photocraft.log");
        let body = text("no adapter for Vulkan", "dx12", None, "0.15.1 (abc1234)", Some(log));
        for part in [
            "could not start",
            "no adapter for Vulkan",
            "Renderer: dx12",
            "Version: 0.15.1 (abc1234)",
            "Log file: /home/me/.config/photocraft/logs/photocraft.log",
        ] {
            assert!(body.contains(part), "{part:?} missing from {body:?}");
        }
        assert!(!body.contains("Startup:"), "no fallback to explain: {body:?}");
        // A start whose log file could not be created says so by leaving the line out.
        assert!(!text("boom", "cpu", None, "0.15.1", None).contains("Log file:"));
    }

    #[test]
    fn a_window_lost_after_starting_does_not_say_it_could_not_start() {
        let log = Path::new("/home/me/.config/photocraft/logs/photocraft.log");
        let note = "the previous start didn't finish on vulkan; using gl";
        let body = closed_text("winit EventLoopError: Exit Failure: 1", "gl", Some(note), "0.18.0", Some(log));
        let startup = format!("Startup: {note}");
        for part in [
            "closed unexpectedly",
            "may be offered for recovery at the next start",
            "winit EventLoopError: Exit Failure: 1",
            "Renderer: gl",
            startup.as_str(),
            "Version: 0.18.0",
            "Log file: /home/me/.config/photocraft/logs/photocraft.log",
        ] {
            assert!(body.contains(part), "{part:?} missing from {body:?}");
        }
        assert!(!body.contains("could not start"), "{body:?}");
    }

    #[test]
    fn the_report_follows_whether_the_window_had_shown() {
        let closed = report(true, "boom", "vulkan", None, "0.18.0", None);
        assert_eq!((closed.title, closed.kind), ("PhotoCraft closed unexpectedly", "unexpected-exit"));
        assert_eq!(closed.body, closed_text("boom", "vulkan", None, "0.18.0", None));
        assert_eq!(log_headline(true), "PhotoCraft closed unexpectedly");

        let failed = report(false, "boom", "vulkan", None, "0.18.0", None);
        assert_eq!((failed.title, failed.kind), ("PhotoCraft could not start", "startup-failure"));
        assert_eq!(failed.body, text("boom", "vulkan", None, "0.18.0", None));
        assert!(failed.body.contains("could not start"), "{:?}", failed.body);
        assert!(!failed.body.contains("recovery") && !failed.body.contains("closed unexpectedly"), "{:?}", failed.body);
        assert_eq!(log_headline(false), "PhotoCraft could not start");
    }

    /// A crash fallback is the confusing case: the app picks a renderer the user never chose, so
    /// the dialog has to say why as well as which.
    #[test]
    fn a_fallback_that_was_not_the_preference_is_explained() {
        let note = "the previous start didn't finish on vulkan; using dx12";
        let body = text("device lost", "dx12", Some(note), "0.15.1", None);
        assert!(body.contains(&format!("Renderer: dx12\nStartup: {note}\n")), "{body:?}");
    }
}
