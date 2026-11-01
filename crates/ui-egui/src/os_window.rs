//! Extra OS windows (document windows, Camera Raw, the screen color picker) share the main
//! window's app id, so docks show the PhotoCraft icon on them and Wayland compositors' window
//! rules can match them (Hyprland's `class:`, #6697). Without one their app id is empty.

/// Matches the `.desktop` file and hicolor icon name, so Wayland docks pick up the icon.
pub const APP_ID: &str = "ai.storyteller.photocraft";

/// A builder for an extra OS window, carrying [`APP_ID`].
pub fn builder() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default().with_app_id(APP_ID)
}

/// Whether PhotoCraft is the app in front: one of its windows (the main window, a document
/// window, Camera Raw) has the keyboard focus. Coming to the front and going to the back each
/// send a window a focus event, which draws a frame.
pub(crate) fn in_front(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.focused || i.raw.viewports.values().any(|v| v.focused == Some(true)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn extra_windows_carry_the_app_id() {
        assert_eq!(super::builder().app_id.as_deref(), Some("ai.storyteller.photocraft"));
    }
}
