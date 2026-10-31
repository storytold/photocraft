use super::PhotocraftApp;
use std::hash::{Hash, Hasher};

fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (w, h, px).hash(&mut hasher);
    hasher.finish()
}

/// Build the OS bitmap only when its full storage and one backend copy fit the export budget.
/// The clipboard backend is outside our control and may allocate more than this allowance.
fn os_clipboard_pixels(clip: &photocraft_engine::edit_cmds::Clip, budget: u64) -> Result<Vec<[u8; 4]>, &'static str> {
    let b = clip.bounds;
    let count = u64::from(b.width()).checked_mul(u64::from(b.height())).ok_or("clipboard image size overflowed")?;
    let estimated = count.checked_mul(8).ok_or("clipboard image size overflowed")?;
    if estimated > budget {
        return Err("the clipboard image exceeds the available export memory");
    }
    let count = usize::try_from(count).map_err(|_| "clipboard image size overflowed")?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).map_err(|_| "not enough memory to export the clipboard image")?;
    pixels.resize(count, [0u8; 4]);
    clip.surface.read_rgba8_into(b, &mut pixels);
    Ok(pixels)
}

impl PhotocraftApp {
    /// Mirror the session clipboard onto the OS clipboard (RGBA8).
    pub(super) fn export_os_clipboard(&mut self) {
        // Allow at most half of configured Memory Usage for the estimated export allocation;
        // cap one export at high settings. This does not measure actual available memory.
        let budget = (u64::from(self.session.prefs().performance.effective_memory_usage_mb()) * (1024 * 1024) / 2).min(1 << 30);
        self.export_os_clipboard_with_budget(budget);
    }

    fn export_os_clipboard_with_budget(&mut self, budget: u64) {
        self.clip_unmirrored = true;
        if !self.session.prefs().general.export_clipboard {
            return;
        }
        let (Some(set), Some(clip)) = (self.services.clipboard_set_image.as_mut(), self.session.clipboard.as_ref()) else { return };
        let b = clip.bounds;
        if b.is_empty() {
            return;
        }
        let pixels = match os_clipboard_pixels(clip, budget) {
            Ok(pixels) => pixels,
            Err(reason) => {
                self.ui.status = format!(
                    "Copied in PhotoCraft, but {reason}; the system clipboard was not updated and a different system image can replace this copy on paste."
                );
                self.ui.status_error = false;
                return;
            }
        };
        // [[u8; 4]] is contiguous: borrowing its bytes avoids a second full-size flatten buffer.
        let bytes = pixels.as_flattened();
        match set(b.width(), b.height(), bytes) {
            Ok(()) => {
                self.os_clip_sig = Some(clip_signature(b.width(), b.height(), bytes));
                self.clip_unmirrored = false;
            }
            Err(reason) => {
                self.ui.status = format!(
                    "Copied in PhotoCraft, but the system clipboard could not be updated: {reason}. A different system image can replace this copy on paste."
                );
                self.ui.status_error = false;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn export_os_clipboard_with_test_budget(&mut self, budget: u64) {
        self.export_os_clipboard_with_budget(budget);
    }

    /// If the OS clipboard holds an image that isn't the one we put there, make it the session
    /// clipboard (so ⌘V pastes screenshots and images copied in other apps, like Photoshop).
    /// Returns `Ok(true)` when a new external image was imported. Once the OS clipboard no longer holds
    /// the image we mirrored or imported (text or a file was copied since), that image is stale:
    /// it is dropped rather than pasted. Unexported internal copies survive unavailable or empty
    /// system clipboards.
    pub(crate) fn import_os_clipboard(&mut self) -> Result<bool, String> {
        let Some(get) = self.services.clipboard_get_image.as_mut() else { return Ok(false) };
        // A read failure says nothing about the clipboard contents. Keep our editable copy and
        // its signature rather than treating the failure as an empty image.
        let image = match get() {
            Ok(image) => image,
            Err(error) => {
                // An unexported copy remains usable without an OS clipboard (for example, in
                // a Wayland sandbox). Mirrored/imported copies still require a successful read
                // before pasting, because their system contents may have changed.
                if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() {
                    return Ok(false);
                }
                if self.session.clipboard.is_none() && self.services.clipboard_unreadable.as_ref().is_some_and(|unreadable| unreadable()) {
                    // Let paste explain the unavailable backend without treating a saved copy as empty.
                    return Ok(false);
                }
                self.ui.status = error.clone();
                self.ui.status_error = true;
                return Err(error);
            }
        };
        let image = image.filter(|(w, h, bytes)| *w > 0 && *h > 0 && bytes.len() == *w as usize * *h as usize * 4);
        let Some((w, h, bytes)) = image else {
            if self.os_clip_sig.take().is_some() && !self.clip_unmirrored {
                self.session.clipboard = None;
                self.clip_external = false;
                self.clip_unmirrored = true;
            }
            return Ok(false);
        };
        let sig = clip_signature(w, h, &bytes);
        if self.os_clip_sig == Some(sig) && self.session.clipboard.is_some() {
            return Ok(false);
        }
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes);
        surface.prune();
        self.session.clipboard = Some(photocraft_engine::edit_cmds::Clip { surface, bounds: r, layers: None, resolution: None, source: None });
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }
}
