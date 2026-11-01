use super::PhotocraftApp;

pub(crate) fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    // Every byte counts: the old step_by(997) sampling could miss a change entirely in a small
    // image (a 3×2 clip samples one byte), and the freshness guard would then call a genuinely
    // newer OS clipboard image unchanged.
    let mut sig = (w as u64) << 32 | h as u64;
    for &b in px {
        sig = (sig ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    sig
}

/// Freshness-baseline sentinel for an observed-empty OS clipboard: remembering it lets a later
/// Paste tell an image copied *after* that observation from the empty state. A real image
/// signature collides with it with probability 2^-64.
const OS_EMPTY: u64 = u64::MAX;

impl PhotocraftApp {
    /// Mirror the session clipboard onto the OS clipboard (RGBA8).
    pub(super) fn export_os_clipboard(&mut self) {
        self.clip_unmirrored = true;
        if !self.session.prefs().general.export_clipboard {
            return;
        }
        let (Some(set), Some(clip)) = (self.services.clipboard_set_image.as_mut(), self.session.clipboard.as_ref()) else { return };
        let b = clip.bounds;
        if b.is_empty() {
            return;
        }
        let mut px = vec![[0u8; 4]; b.width() as usize * b.height() as usize];
        clip.surface.read_rgba8_into(b, &mut px);
        let bytes: Vec<u8> = px.into_iter().flatten().collect();
        if set(b.width(), b.height(), &bytes).is_ok() {
            self.os_clip_sig = Some(clip_signature(b.width(), b.height(), &bytes));
            self.clip_unmirrored = false;
        }
    }

    /// Record the OS clipboard's current signature without importing it: the freshness baseline
    /// a later Paste compares against, so an unchanged or older OS image never displaces a
    /// known-fresh internal clip while a genuinely newer one still wins. An empty OS clipboard
    /// is remembered as a sentinel too. Runs on explicit clipboard actions only (Copy, Cut,
    /// Paste), never in the background.
    pub(crate) fn calibrate_os_clipboard(&mut self) {
        let Some(get) = self.services.clipboard_get_image.as_mut() else { return };
        // The RGBA length from the OS-provided dimensions is computed with checked arithmetic:
        // malformed metadata must be rejected, never overflow.
        match get() {
            Ok(Some((w, h, bytes))) => {
                let rgba = u64::from(w).checked_mul(u64::from(h)).and_then(|n| n.checked_mul(4));
                self.os_clip_sig = Some(if rgba == Some(bytes.len() as u64) { clip_signature(w, h, &bytes) } else { OS_EMPTY });
            }
            Ok(None) => self.os_clip_sig = Some(OS_EMPTY),
            Err(_) => {}
        }
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
        // Malformed dimensions are rejected with checked arithmetic, never an overflowing
        // multiply on input-derived numbers.
        let image = image.filter(|(w, h, bytes)| {
            let rgba = u64::from(*w).checked_mul(u64::from(*h)).and_then(|n| n.checked_mul(4));
            *w > 0 && *h > 0 && rgba == Some(bytes.len() as u64)
        });
        let Some((w, h, bytes)) = image else {
            if self.os_clip_sig.take().is_some() && !self.clip_unmirrored {
                self.session.clipboard = None;
                self.clip_external = false;
                self.clip_unmirrored = true;
            }
            // While a path clip holds, keep the freshness baseline: an empty OS clipboard
            // observed once is the baseline, so an image copied later still reads as newer.
            if self.session.path_clipboard.is_some() {
                self.os_clip_sig = Some(OS_EMPTY);
            }
            return Ok(false);
        };
        let sig = clip_signature(w, h, &bytes);
        // The unchanged OS image is stale when an internal clip has replaced it since (our
        // unexported pixel copy or a path component): importing would paste superseded content.
        if self.os_clip_sig == Some(sig) && (self.session.clipboard.is_some() || self.session.path_clipboard.is_some()) {
            return Ok(false);
        }
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes);
        surface.prune();
        // One clipboard: imported pixels replace a path clip, like a path clip replaces them.
        self.session.path_clipboard = None;
        self.session.clipboard =
            Some(photocraft_engine::edit_cmds::Clip { components: None, surface, bounds: r, layers: None, resolution: None, source: None });
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }
}
