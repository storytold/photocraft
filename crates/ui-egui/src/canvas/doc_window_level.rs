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
    let (blocked_key, raise_key) = (Id::unique("pc-docwin-blocked"), Id::unique("pc-docwin-raise-main"));
    let was_blocked = ctx.data(|d| d.get_temp::<bool>(blocked_key)).unwrap_or(false);
    ctx.data_mut(|d| d.insert_temp(blocked_key, blocked));
    if ctx.data_mut(|d| d.remove_temp::<bool>(raise_key)).unwrap_or(false) && blocked {
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::WindowLevel(WindowLevel::AlwaysOnTop));
        ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::WindowLevel(WindowLevel::Normal));
    }
    if any && blocked && !was_blocked {
        ctx.data_mut(|d| d.insert_temp(raise_key, true));
        ctx.request_repaint();
    }
    level(app_active(ctx), blocked)
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
