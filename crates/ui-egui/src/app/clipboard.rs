use super::PhotocraftApp;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Hash dimensions and the complete RGBA payload for a process-local freshness check.
/// Sampling can miss changed channels even in a 1×1 image. This finite signature is a
/// comparison hint, not a persistent identifier or a guarantee against hash collisions.
fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut sig = DefaultHasher::new();
    w.hash(&mut sig);
    h.hash(&mut sig);
    sig.write(px);
    sig.finish()
}

/// Hash the SVG raster mirror in row order with the bitmap's signature algorithm.
/// A fixed scratch buffer avoids duplicating the full image. This must stay consistent
/// with `clip_signature`, or an unchanged SVG mirror can replace a newer internal copy.
fn svg_clip_signature(clip: &photocraft_engine::edit_cmds::Clip) -> Result<u64, String> {
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    let mut sig = DefaultHasher::new();
    w.hash(&mut sig);
    h.hash(&mut sig);
    let mut pixels = [[0u8; 4]; 256];
    for y in b.y0..b.y1 {
        let mut x = b.x0;
        while x < b.x1 {
            let count = (i64::from(b.x1) - i64::from(x)).min(256) as usize;
            let chunk = pixels.get_mut(..count).ok_or("invalid SVG clipboard signature chunk")?;
            clip.surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(x, y, count as u32, 1), chunk);
            sig.write(chunk.as_flattened());
            x = x.saturating_add(count as i32);
        }
    }
    Ok(sig.finish())
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
        // Already imported, or a newer internal copy: either stays until the external bytes
        // change. A raster mirror an ordinary paste took of these bytes is not their vectors.
        let holds_vectors_or_newer = self.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some() || !self.clip_external);
        if self.os_clip_svg.as_deref().is_some_and(|old| old == &source) && holds_vectors_or_newer {
            return Ok(true);
        }
        let clip = photocraft_engine::edit_cmds::Clip::from_svg(&source).map_err(|error| {
            let error = error.to_string();
            self.ui.status = error.clone();
            self.ui.status_error = true;
            error
        })?;
        self.os_clip_sig = Some(svg_clip_signature(&clip)?);
        self.os_clip_svg = clip.svg.clone();
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
        if self.os_clip_sig == Some(sig) && self.session.clipboard.is_some() {
            return Ok(false);
        }
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes);
        surface.prune();
        self.session.clipboard = Some(photocraft_engine::edit_cmds::Clip {
            components: None,
            surface,
            bounds: r,
            layers: None,
            resolution: None,
            source: None,
            svg: None,
            board: None,
        });
        // The SVG that came with this image (a vector app puts both): a later internal copy is
        // newer than it, so Paste As must not bring it back while it is unchanged.
        self.os_clip_svg = self.os_svg_now().flatten();
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }
}

#[cfg(test)]
mod signature_tests {
    use super::*;

    /// The streamed SVG mirror must match the whole bitmap at row and scratch-buffer edges,
    /// including translated bounds, transparent pixels and an empty mirror.
    #[test]
    fn streamed_svg_signature_matches_bitmap() {
        use photocraft_color::PixelFormat;
        use photocraft_geom::Rect;
        use photocraft_raster::Surface;
        for (width, height) in [(0, 0), (1, 1), (255, 2), (256, 3), (257, 2), (513, 3)] {
            let bounds = Rect::from_xywh(-3, 5, width, height);
            let mut surface = Surface::new(PixelFormat::RGBA8);
            for y in bounds.y0..bounds.y1 {
                for x in bounds.x0..bounds.x1 {
                    surface.write_pixel(x, y, &[0.2, 0.4, 0.8, if x % 2 == 0 { 0.5 } else { 1.0 }]);
                }
            }
            let mut pixels = vec![[0; 4]; (width * height) as usize];
            surface.read_rgba8_into(bounds, &mut pixels);
            let clip =
                photocraft_engine::edit_cmds::Clip { surface, bounds, layers: None, resolution: None, source: None, board: None, svg: None, components: None };
            assert_eq!(svg_clip_signature(&clip).unwrap(), clip_signature(width, height, pixels.as_flattened()));
        }
    }
}
