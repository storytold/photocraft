use super::PhotocraftApp;

pub(crate) fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    // Every byte counts: the old step_by(997) sampling could miss a change entirely in a small
    // image (a 3×2 clip samples one byte), and the freshness guard would then call a genuinely
    // newer OS clipboard image unchanged.
    fold_bytes((w as u64) << 32 | h as u64, px)
}

/// Folds `bytes` into `sig`, one rolling multiply per byte; streaming callers (the SVG mirror's
/// row-wise signature) feed chunk after chunk to stay within the same result.
pub(crate) fn fold_bytes(mut sig: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        sig = (sig ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    sig
}

/// The bitmap signature of the clip's raster mirror, byte-identical to what
/// [`PhotocraftApp::import_os_clipboard`] computes for the same pixels: the two import paths
/// must agree, or the freshness guard between them never fires. Row-wise, so a 24 MP mirror
/// does not allocate the whole image twice.
fn svg_clip_signature(clip: &photocraft_engine::edit_cmds::Clip) -> u64 {
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 {
        return (u64::from(w) << 32) | u64::from(h);
    }
    let mut sig = (u64::from(w) << 32) | u64::from(h);
    let mut row = vec![[0u8; 4]; w as usize];
    for y in 0..h {
        clip.surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(b.x0, b.y0 + y as i32, w, 1), &mut row);
        let flat: Vec<u8> = row.iter().flatten().copied().collect();
        sig = fold_bytes(sig, &flat);
    }
    sig
}

impl PhotocraftApp {
    /// Capture SVG and its raster mirror together; failures leave the previous copy intact.
    pub(super) fn import_os_svg_clipboard(&mut self) -> Result<bool, String> {
        let Some(get) = self.services.clipboard_get_svg.as_mut() else { return Ok(false) };
        let source = match get() {
            Ok(source) => source,
            Err(_) if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() => return Ok(false),
            Err(error) => {
                self.ui.status = error.clone();
                self.ui.status_error = true;
                return Err(error);
            }
        };
        let Some(source) = source else {
            self.os_clip_svg = None;
            // Bitmap equality cannot prove that an earlier external vector source is current.
            if self.clip_external && self.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some()) {
                self.session.clipboard = None;
                self.os_clip_sig = None;
                self.clip_external = false;
                self.clip_unmirrored = true;
            }
            return Ok(false);
        };
        // Already imported, or a newer internal copy (pixels or a path component): either
        // stays until the external bytes change. A raster mirror an ordinary paste took of
        // these bytes is not their vectors. `false` (nothing imported) lets Paste As pixels
        // fall through to the bitmap import: the OS clipboard can hold both, and the bitmap
        // may have changed even while these bytes did not.
        let holds_vectors_or_newer =
            self.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some() || !self.clip_external) || self.session.path_clipboard.is_some();
        if self.os_clip_svg.as_deref().is_some_and(|old| old == &source) && holds_vectors_or_newer {
            return Ok(false);
        }
        let clip = photocraft_engine::edit_cmds::Clip::from_svg(&source).map_err(|error| {
            let error = error.to_string();
            self.ui.status = error.clone();
            self.ui.status_error = true;
            error
        })?;
        self.os_clip_sig = Some(svg_clip_signature(&clip));
        self.os_clip_svg = clip.svg.clone();
        // One clipboard: an imported SVG replaces a path clip, like any installed clip.
        self.session.path_clipboard = None;
        self.session.clipboard = Some(clip);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }

    /// The SVG bytes on the OS clipboard right now, read on an explicit paste without importing
    /// them; `None` when there is no SVG service or the read fails (nothing is known then).
    fn os_svg_now(&mut self) -> Option<Option<std::sync::Arc<Vec<u8>>>> {
        let get = self.services.clipboard_get_svg.as_mut()?;
        get().ok().map(|source| source.map(std::sync::Arc::new))
    }

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
            self.os_clip_svg = None;
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
            if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() {
                self.os_clip_sig = None;
                // Keep remembering an SVG only while it is still there (an SVG-only clipboard),
                // so a later copy of the same bytes in another app is imported again.
                if let Some(now) = self.os_svg_now()
                    && now != self.os_clip_svg
                {
                    self.os_clip_svg = None;
                }
                return Ok(false);
            }
            self.os_clip_svg = None;
            if (self.os_clip_sig.take().is_some() || self.clip_external) && !self.clip_unmirrored {
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
            Some(photocraft_engine::edit_cmds::Clip { components: None, surface, bounds: r, layers: None, resolution: None, source: None, svg: None });
        // The SVG that came with this image (a vector app puts both): a later internal copy is
        // newer than it, so Paste As must not bring it back while it is unchanged.
        self.os_clip_svg = self.os_svg_now().flatten();
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }
}
