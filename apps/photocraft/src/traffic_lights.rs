//! Where AppKit puts the traffic lights, so the integrated title strip fits around them (#2554).
//!
//! The lights' size and position change between macOS versions (they grew in macOS 26), so the
//! strip measures the real buttons instead of assuming one layout, as WordCraft does. These calls
//! are safe in the `objc2-app-kit` bindings (the workspace forbids unsafe).

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindowButton};
use photocraft_ui_egui::MacTitleBar;

/// The strip around the app window's traffic lights, or `None` off the main thread, before the
/// window exists, or when AppKit has no zoom button to measure (the strip keeps its standard size).
pub fn title_bar() -> Option<MacTitleBar> {
    let mtm = MainThreadMarker::new()?;
    let window = NSApplication::sharedApplication(mtm).windows().firstObject()?;
    let zoom = window.standardWindowButton(NSWindowButton::ZoomButton)?;
    // In window coordinates, which count up from the bottom.
    let frame = zoom.convertRect_toView(zoom.bounds(), None);
    let window_height = window.frame().size.height;
    let right = frame.origin.x + frame.size.width;
    let centre_from_top = window_height - (frame.origin.y + frame.size.height / 2.0);
    if !(right.is_finite() && centre_from_top.is_finite() && right > 0.0 && centre_from_top > 0.0) {
        return None;
    }
    Some(MacTitleBar::around_lights(right as f32, centre_from_top as f32, frame.size.height as f32))
}
