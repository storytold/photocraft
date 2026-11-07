//! Documents floating in their own OS windows stay in front of the main window, as in Photoshop:
//! a click on the toolbar, the options bar or a panel must not send them behind it. They follow
//! the policy craft-ui gives floating panels' windows (`docking::render::native`):
//!
//! - While the application is active they are `AlwaysOnTop`, so the main window can't cover them.
//! - While a modal dialog or workspace is up in the main window they drop to the normal level and
//!   the main window comes forward once, so they can't cover the dialog.
//! - While the application is inactive they drop behind other applications' windows, so they
//!   don't cover them.

use egui::{Context, Id, ViewportCommand, ViewportId, WindowLevel};

use crate::PhotocraftApp;

/// How long none of the application's windows may have focus before it counts as inactive, so
/// focus moving between its own windows doesn't (craft-ui's `AWAY_DELAY`).
const AWAY_DELAY: f64 = 0.25;

/// The level of a document window while the application is inactive, behind other applications'
/// windows. On Windows `WindowLevel::Normal` is `HWND_NOTOPMOST`, which puts the window in front
/// of every window that is not topmost, the activated application's too; `HWND_BOTTOM` sends it
/// behind them without activating it (craft-ui's `INACTIVE_PALETTE`).
const INACTIVE: WindowLevel = if cfg!(target_os = "windows") { WindowLevel::AlwaysOnBottom } else { WindowLevel::Normal };

/// The level of the document windows for whether the application is `active` and whether a modal
/// in the main window has the input (`blocked`).
pub(crate) fn level(active: bool, blocked: bool) -> WindowLevel {
    match (blocked, active) {
        (true, _) => WindowLevel::Normal,
        (false, true) => WindowLevel::AlwaysOnTop,
        (false, false) => INACTIVE,
    }
}

/// Whether the application is active this frame: one of its windows has focus, or had it less
/// than [`AWAY_DELAY`] ago. A backend that reports no focus at all counts as active.
pub(crate) fn app_active(ctx: &Context) -> bool {
    let (focused, now) = ctx.input(|i| {
        let infos = || i.raw.viewports.values();
        (i.focused || infos().any(|v| v.focused == Some(true)) || infos().all(|v| v.focused.is_none()), i.time)
    });
    let key = Id::unique("pc-docwin-last-focused");
    let last = if focused { now } else { ctx.data(|d| d.get_temp::<f64>(key)).unwrap_or(now) };
    ctx.data_mut(|d| d.insert_temp(key, last));
    let active = focused || now - last < AWAY_DELAY;
    if active && !focused {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((AWAY_DELAY - (now - last)).max(0.0)));
    }
    active
}

/// The level for this frame's document windows (`any`: there are some). When a modal opens in the
/// main window the windows drop to the normal level, which on Windows puts them in front of every
/// window that isn't topmost, the main window too: the main window, with the modal, is raised the
/// pass after. Toggling its level raises it without taking focus (craft-ui's `raise_next`).
pub(crate) fn frame_level(app: &PhotocraftApp, ctx: &Context, any: bool) -> WindowLevel {
    let blocked = crate::panel_docking::modal_open(app, ctx);
    let active = app_active(ctx);
    if raise_main(ctx, any, blocked, active) {
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::WindowLevel(WindowLevel::Normal));
    }
    level(active, blocked)
}

/// Whether to raise the main window this pass: the pass after a modal opened in it while there
/// were document windows (`any`), and only once the application is `active`. While it is
/// inactive (a file handed over by another launch can open a dialog in the background) the raise
/// waits, so PhotoCraft never comes in front of the application the user is in; it is dropped
/// once the modal closes.
fn raise_main(ctx: &Context, any: bool, blocked: bool, active: bool) -> bool {
    let (blocked_key, raise_key) = (Id::unique("pc-docwin-blocked"), Id::unique("pc-docwin-raise-main"));
    let was_blocked = ctx.data(|d| d.get_temp::<bool>(blocked_key)).unwrap_or(false);
    let pending = ctx.data(|d| d.get_temp::<bool>(raise_key)).unwrap_or(false);
    let raise = pending && blocked && active;
    let queued = any && blocked && !was_blocked;
    let wait = (pending && blocked && !active) || queued;
    ctx.data_mut(|d| {
        d.insert_temp(blocked_key, blocked);
        if wait {
            d.insert_temp(raise_key, true);
        } else {
            d.remove_temp::<bool>(raise_key);
        }
    });
    if queued {
        ctx.request_repaint();
    }
    raise
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_windows_stay_above_the_main_window_while_the_app_is_in_use() {
        assert_eq!(level(true, false), WindowLevel::AlwaysOnTop);
        // A dialog in the main window: they must not cover it.
        assert_eq!(level(true, true), WindowLevel::Normal);
        // Another application in front: they must not cover it.
        assert_eq!(level(false, false), INACTIVE);
        assert_ne!(INACTIVE, WindowLevel::AlwaysOnTop);
    }

    /// One frame of the main window at `time`, with the given focus of the main window and of
    /// one document window.
    fn frame(ctx: &Context, time: f64, main: bool, document: bool) -> bool {
        let info = |focused| egui::ViewportInfo { focused: Some(focused), ..Default::default() };
        let raw = egui::RawInput {
            time: Some(time),
            focused: main,
            viewports: [(ViewportId::ROOT, info(main)), (ViewportId::from_hash_of(("docwin", 1_u64)), info(document))].into_iter().collect(),
            ..Default::default()
        };
        let mut active = false;
        ctx.run_ui(raw, |ui| active = app_active(ui.ctx())).textures_delta.clear();
        active
    }

    /// A modal opening over document windows raises the main window once, the pass after, while
    /// the application is active (CodeRabbit on #11666).
    #[test]
    fn a_modal_raises_the_main_window_once_the_pass_after_it_opens() {
        let ctx = Context::default();
        assert!(!raise_main(&ctx, true, false, true), "no modal: nothing to raise");
        assert!(!raise_main(&ctx, true, true, true), "the modal opens: the raise waits for the windows to drop");
        assert!(raise_main(&ctx, true, true, true), "the pass after: the main window is raised");
        assert!(!raise_main(&ctx, true, true, true), "once");
        assert!(!raise_main(&ctx, true, false, true));
        // Without document windows there is nothing for the main window to come in front of.
        assert!(!raise_main(&ctx, false, true, true));
        assert!(!raise_main(&ctx, false, true, true));
    }

    /// A modal that opens while the application is inactive does not raise it over the
    /// application the user is in; the raise waits until it is active, and is dropped once the
    /// modal closes (CodeRabbit on #11666).
    #[test]
    fn the_raise_waits_while_the_application_is_inactive() {
        let ctx = Context::default();
        assert!(!raise_main(&ctx, true, false, false));
        assert!(!raise_main(&ctx, true, true, false), "the modal opens in the background");
        assert!(!raise_main(&ctx, true, true, false), "still in the background: no raise");
        assert!(!raise_main(&ctx, true, true, false));
        assert!(raise_main(&ctx, true, true, true), "the user comes back: now it is raised");
        assert!(!raise_main(&ctx, true, true, true), "once");

        let ctx = Context::default();
        assert!(!raise_main(&ctx, true, false, false));
        assert!(!raise_main(&ctx, true, true, false), "the modal opens in the background");
        assert!(!raise_main(&ctx, true, false, false), "and closes before the user comes back");
        assert!(!raise_main(&ctx, true, false, true), "nothing is raised later");
    }

    #[test]
    fn focus_moving_between_the_apps_windows_keeps_it_active() {
        let ctx = Context::default();
        assert!(frame(&ctx, 1.0, true, false), "the main window has focus");
        assert!(frame(&ctx, 1.1, false, true), "a document window has focus");
        // A click from one window to another: for a moment neither has focus.
        assert!(frame(&ctx, 1.2, false, false), "still active within the delay");
        assert!(!frame(&ctx, 1.6, false, false), "inactive once no window has had focus for a while");
        assert!(frame(&ctx, 2.0, false, true), "active again as soon as one of its windows has focus");
    }
}
