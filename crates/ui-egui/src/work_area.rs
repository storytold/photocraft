//! The part of the window the user can actually see and reach (#315).
//!
//! A window can be larger than its display: PhotoCraft opens at 1440 × 900 pt, which runs past
//! the bottom of a 1366 × 768 screen and under the Windows taskbar. The visible-part rules live in
//! craft-window; [`fit_window`] fixes the cause at startup: a window that doesn't fit its monitor is
//! maximized, which puts it in the work area (Photoshop opens maximized on small displays).

use craft::window::desktop::fits;

fn fitted_id() -> egui::Id {
    egui::Id::unique("pc-window-fitted")
}

/// Once, as soon as the monitor is known: maximize a window that is bigger than its monitor
/// (it ran under the taskbar and off-screen). Returns true when it asked to.
pub fn fit_window(ctx: &egui::Context) -> bool {
    if ctx.data(|d| d.get_temp::<bool>(fitted_id())).is_some() {
        return false;
    }
    let content = ctx.content_rect();
    let (monitor, outer, maximized, fullscreen) = ctx.input(|i| {
        let v = i.viewport();
        (v.monitor_size, v.outer_rect, v.maximized, v.fullscreen)
    });
    // On Wayland the window's position, and so its outer rect, is never known (#3737): its content
    // size is the least the window takes up.
    let outer = outer.unwrap_or(content);
    let Some(m) = monitor else { return false };
    // macOS keeps new windows inside the screen's visible frame itself.
    if cfg!(target_os = "macos") {
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(fitted_id(), true));
    if maximized == Some(true) || fullscreen == Some(true) || !m.is_finite() || !outer.is_finite() || fits(outer.size(), m) {
        return false;
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, Vec2, pos2, vec2};

    const MONITOR: Vec2 = vec2(1366.0, 768.0);

    fn frame(ctx: &egui::Context, monitor: Option<Vec2>, outer: Option<Rect>, maximized: Option<bool>) -> Vec<egui::ViewportCommand> {
        let mut input = egui::RawInput::default();
        if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.monitor_size = monitor;
            v.outer_rect = outer;
            v.maximized = maximized;
        }
        let out = crate::testing::run_ui(ctx, input, |ui| {
            fit_window(ui.ctx());
        });
        out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
    }

    #[test]
    fn a_window_too_big_for_its_display_is_maximized_once() {
        let big = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1456.0, 939.0)));
        let ctx = egui::Context::default();
        // The monitor isn't known on the first frames: wait.
        assert!(!frame(&ctx, None, big, Some(false)).contains(&egui::ViewportCommand::Maximized(true)));
        let maximize = frame(&ctx, Some(MONITOR), big, Some(false)).contains(&egui::ViewportCommand::Maximized(true));
        assert_eq!(maximize, !cfg!(target_os = "macos"));
        // Once only: un-maximizing it later is the user's choice.
        assert!(!frame(&ctx, Some(MONITOR), big, Some(false)).contains(&egui::ViewportCommand::Maximized(true)));
        // A window that fits, or one already maximized, is left alone.
        let ctx = egui::Context::default();
        assert!(
            !frame(&ctx, Some(MONITOR), Some(Rect::from_min_size(pos2(40.0, 40.0), vec2(1200.0, 700.0))), Some(false))
                .contains(&egui::ViewportCommand::Maximized(true))
        );
        let ctx = egui::Context::default();
        assert!(!frame(&ctx, Some(MONITOR), big, Some(true)).contains(&egui::ViewportCommand::Maximized(true)));
        // Wayland: no outer rect, but a content area bigger than the monitor is maximized too (#3737).
        let ctx = egui::Context::default();
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 900.0))), ..Default::default() };
        if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.monitor_size = Some(MONITOR);
            v.maximized = Some(false);
        }
        let out = crate::testing::run_ui(&ctx, input, |ui| {
            fit_window(ui.ctx());
        });
        let cmds = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
        assert_eq!(cmds.contains(&egui::ViewportCommand::Maximized(true)), !cfg!(target_os = "macos"));
    }
}
