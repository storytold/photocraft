//! Keep native pickers attached to the viewport that asked for them (#10838).
//!
//! eframe's Frame exposes the root window, even while an immediate viewport is drawing. Its
//! lifecycle hook supplies each actual window. Configure rfd there through its safe API, and
//! discard the template when that window goes away rather than retaining stale handles.

use std::collections::HashMap;
use std::sync::Mutex;

use eframe::wgpu::rwh::{HasDisplayHandle, HasWindowHandle};
use egui::ViewportId;
use photocraft_ui_egui::FileDialogParent;

#[derive(Clone, Default)]
pub(crate) struct WindowDialog {
    pub dialog: rfd::AsyncFileDialog,
    #[cfg(target_os = "linux")]
    pub portal_parent: String,
}

impl WindowDialog {
    fn for_window(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Self {
        Self {
            dialog: rfd::AsyncFileDialog::new().set_parent(window),
            #[cfg(target_os = "linux")]
            portal_parent: crate::portal_save::parent_id(window.window_handle().ok().map(|h| h.as_raw())),
        }
    }
}

#[derive(Default)]
pub(crate) struct FileDialogWindows {
    windows: Mutex<HashMap<ViewportId, WindowDialog>>,
}

impl FileDialogWindows {
    pub fn picker(&self, parent: FileDialogParent<'_>) -> Result<WindowDialog, String> {
        if let Some(window) = self.windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&parent.viewport).cloned() {
            return Ok(window);
        }
        if parent.viewport != ViewportId::ROOT {
            // A closed auxiliary window must never give its picker to the root window.
            return Err("The window requesting the file dialog is no longer available".into());
        }
        Ok(parent.frame.map(WindowDialog::for_window).unwrap_or_default())
    }
}

impl eframe::ViewportHooks for FileDialogWindows {
    fn window_created(&self, viewport: ViewportId, window: &winit::window::Window) {
        self.windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(viewport, WindowDialog::for_window(window));
    }

    fn window_destroyed(&self, viewport: ViewportId) {
        self.windows.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::ViewportHooks as _;

    #[test]
    fn missing_auxiliary_window_never_falls_back_to_root() {
        let windows = FileDialogWindows::default();
        windows.windows.lock().unwrap().insert(ViewportId::ROOT, WindowDialog::default());
        assert!(windows.picker(FileDialogParent { viewport: ViewportId::from_hash_of("closed"), frame: None }).is_err());
        assert!(windows.picker(FileDialogParent { viewport: ViewportId::ROOT, frame: None }).is_ok());
    }

    #[test]
    fn destroying_a_window_forgets_its_picker_without_affecting_root() {
        let windows = FileDialogWindows::default();
        let auxiliary = ViewportId::from_hash_of("camera-raw-window");
        for id in [ViewportId::ROOT, auxiliary] {
            windows.windows.lock().unwrap().insert(id, WindowDialog::default());
        }
        assert!(windows.picker(FileDialogParent { viewport: auxiliary, frame: None }).is_ok());
        windows.window_destroyed(auxiliary);
        assert!(windows.picker(FileDialogParent { viewport: auxiliary, frame: None }).is_err());
        assert!(windows.picker(FileDialogParent { viewport: ViewportId::ROOT, frame: None }).is_ok());
        windows.window_destroyed(auxiliary);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recreated_window_uses_its_new_portal_parent() {
        let windows = FileDialogWindows::default();
        let auxiliary = ViewportId::from_hash_of("camera-raw-window");
        for parent in ["x11:100", "x11:200"] {
            windows.windows.lock().unwrap().insert(auxiliary, WindowDialog { portal_parent: parent.into(), ..Default::default() });
            assert_eq!(windows.picker(FileDialogParent { viewport: auxiliary, frame: None }).unwrap().portal_parent, parent);
            windows.window_destroyed(auxiliary);
        }
    }
}
