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

    /// If the OS clipboard holds an image that isn't the one we put there, make it the session
    /// clipboard (so ⌘V pastes screenshots and images copied in other apps, like Photoshop).
    /// Returns `Ok(true)` when a new external image was imported. Once the OS clipboard no longer holds
    /// the image we mirrored or imported (text or a file was copied since), that image is stale:
    /// it is dropped rather than pasted. Unexported internal copies survive unavailable or empty
    /// system clipboards.
    ///
    /// Freshness rule (one clipboard): an OS image displaces a path clip only when the app
    /// observed the OS clipboard before (a signature baseline exists) and the image changed
    /// since. On the first observation with a path clip, the baseline is learned but the image
    /// is not installed — known-fresh path content never loses to an image of unknown age (the
    /// OS clipboard may have held it since before the app started). The accepted tradeoff: an
    /// image copied in another app before that first observation waits for a later change;
    /// ordering the two without an OS change token is impossible, and known-fresh beats
    /// unknown. Pixel clips keep the older rule: a different OS image supersedes them.
    pub(crate) fn import_os_clipboard(&mut self) -> Result<bool, String> {
        let Some(get) = self.services.clipboard_get_image.as_mut() else { return Ok(false) };
        let baseline_existed = self.os_clip_sig.is_some();
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
        // multiply on input-derived numbers; beyond-i32 sizes would wrap the cast to Rect.
        let image = image.filter(|(w, h, bytes)| {
            let rgba = u64::from(*w).checked_mul(u64::from(*h)).and_then(|n| n.checked_mul(4));
            *w > 0 && *h > 0 && *w <= i32::MAX as u32 && *h <= i32::MAX as u32 && rgba == Some(bytes.len() as u64)
        });
        let Some((w, h, bytes)) = image else {
            if self.os_clip_sig.take().is_some() && !self.clip_unmirrored {
                self.session.clipboard = None;
                self.clip_external = false;
                self.clip_unmirrored = true;
            }
            return Ok(false);
        };
        let sig = clip_signature(w, h, &bytes);
        let internal = self.session.clipboard.is_some() || self.session.path_clipboard.is_some();
        // The unchanged OS image is stale when an internal clip has replaced it since (our
        // unexported pixel copy or a path component): importing would paste superseded content.
        if self.os_clip_sig == Some(sig) && internal {
            return Ok(false);
        }
        // First observation with a path clip: learn the baseline, but never displace known-fresh
        // path content with an image of unknown age. (Pixel clips keep the long-standing rule:
        // a different OS image supersedes them.)
        if !baseline_existed && self.session.path_clipboard.is_some() {
            self.os_clip_sig = Some(sig);
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
