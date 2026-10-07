//! macOS title bar dragging. The window draws under a transparent title bar (`main.rs`), so AppKit
//! would otherwise drag the window from anywhere in that strip, menus included: pressing a menu
//! title and dragging down to an item moved the window instead. Turning AppKit's own dragging off
//! leaves only the app's drag region (the free gap between the menus and the controls,
//! `panels::title_bar`), which starts a drag explicitly (`ViewportCommand::StartDrag`, AppKit's
//! `performWindowDragWithEvent:`, which works on an unmovable window). All safe objc2 calls.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

/// Stop AppKit dragging the app's windows by their title bar strip.
pub fn disable_native_title_drag() {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("title bar: not on the main thread; AppKit window dragging left on");
        return;
    };
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        window.setMovable(false);
    }
}
