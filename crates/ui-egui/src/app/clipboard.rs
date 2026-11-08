use super::PhotocraftApp;

/// `surface`'s pixels in `b` as straight RGBA8 bytes, row by row (read in parallel bands).
fn rgba8(surface: &photocraft_raster::Surface, b: photocraft_geom::Rect) -> Vec<u8> {
    photocraft_algo::selection::rgba8_image(surface, b).into_flattened()
}

/// The largest copy mirrored to the OS clipboard, in pixels (64 MP: 256 MiB of RGBA, an
/// 8192 × 8192 image). Larger clips stay PhotoCraft's own: Paste inside PhotoCraft is
/// unaffected. Beyond this the mirror costs gigabytes of RGBA, and the clipboard library
/// encodes it as PNG on the UI thread when the OS takes it, which froze the window for as long
/// as that took; few other apps could open such an image anyway.
pub(crate) const MAX_OS_CLIPBOARD_PIXELS: u64 = 64 << 20;

/// The title of the notice for a copy too large for the system clipboard.
const KEPT_IN_PHOTOCRAFT: &str = "Copy kept in PhotoCraft";

/// Whether a copy of `b` is mirrored to the OS clipboard (see [`MAX_OS_CLIPBOARD_PIXELS`]).
pub(super) fn mirrors_to_os(b: photocraft_geom::Rect) -> bool {
    b.size().area() <= MAX_OS_CLIPBOARD_PIXELS
}

fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut sig = (w as u64) << 32 | h as u64;
    for b in px.iter().step_by(997) {
        sig = sig.rotate_left(5) ^ *b as u64;
    }
    sig
}

/// Match the bitmap signature by sampling the bounded SVG mirror, without another full pixel buffer.
fn svg_clip_signature(clip: &photocraft_engine::edit_cmds::Clip) -> u64 {
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    let mut sig = (u64::from(w) << 32) | u64::from(h);
    if w == 0 || h == 0 {
        return sig;
    }
    for i in (0..b.size().area().saturating_mul(4)).step_by(997) {
        let x = b.x0.saturating_add((i / 4 % u64::from(w)) as i32);
        let y = b.y0.saturating_add((i / 4 / u64::from(w)) as i32);
        let mut pixel = [[0; 4]];
        clip.surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(x, y, 1, 1), &mut pixel);
        let value = pixel[0].get((i % 4) as usize).copied().unwrap_or_default();
        sig = sig.rotate_left(5) ^ u64::from(value);
    }
    sig
}

impl PhotocraftApp {
    /// Capture SVG and its raster mirror together; failures leave the previous copy intact.
    pub(super) fn import_os_svg_clipboard(&mut self) -> Result<bool, String> {
        // Our own copy is still on its way to the OS clipboard: it is the newest, keep it.
        if self.os_clip_export.is_some() {
            return Ok(false);
        }
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
        self.os_clip_sig = Some(svg_clip_signature(&clip));
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
    ///
    /// Converting the clip to RGBA8 reads every pixel: hundreds of megabytes and seconds of work
    /// for a large canvas, which on the UI thread froze the window until the Wayland compositor
    /// dropped it. With background jobs on (the desktop app) the conversion runs on a worker and
    /// [`Self::poll_os_clipboard_export`] hands the pixels to the OS clipboard when they are ready.
    pub(crate) fn export_os_clipboard(&mut self) {
        self.clip_unmirrored = true;
        // A new copy: a notice about an older one no longer applies, whatever happens to this one
        // (export off, a failed write); it is posted again below if this copy is too large too.
        self.ui.notices.retain(|n| n.title != KEPT_IN_PHOTOCRAFT);
        // One conversion at a time: a newer copy waits for the running one, which is then
        // dropped (see `poll_os_clipboard_export`), so superseded copies can't pile up workers.
        let exporting = self.session.prefs().general.export_clipboard && self.services.clipboard_set_image.is_some();
        if self.os_clip_export.is_some() {
            self.os_clip_export_again = true;
            // Say at once that this copy won't reach other apps, not when the running one lands.
            if exporting && self.session.clipboard.as_ref().is_some_and(|c| !c.bounds.is_empty() && !mirrors_to_os(c.bounds)) {
                self.note_kept_in_photocraft();
            }
            return;
        }
        if !exporting {
            return;
        }
        let Some(clip) = self.session.clipboard.as_ref() else { return };
        let b = clip.bounds;
        if b.is_empty() {
            return;
        }
        if !mirrors_to_os(b) {
            log::info!("copy of {}×{} px kept in PhotoCraft: too large for the system clipboard", b.width(), b.height());
            self.note_kept_in_photocraft();
            return;
        }
        if self.background_jobs {
            let surface = clip.surface.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new().name("photocraft-clipboard-export".into()).spawn(move || {
                let _ = tx.send((b.width(), b.height(), rgba8(&surface, b)));
            });
            if spawned.is_ok() {
                self.os_clip_export = Some(rx);
                return;
            }
        }
        let bytes = rgba8(&clip.surface, b);
        self.set_os_clipboard_image(b.width(), b.height(), &bytes);
    }

    /// Tell the user a copy is too large for the system clipboard. One notice however many such
    /// copies follow: a newer one replaces it rather than stacking up and pushing others out.
    fn note_kept_in_photocraft(&mut self) {
        self.ui.notices.retain(|n| n.title != KEPT_IN_PHOTOCRAFT);
        crate::notices::post(
            self,
            KEPT_IN_PHOTOCRAFT,
            vec![
                "PhotoCraft doesn't send copies over 64 megapixels to the system clipboard, so other apps can't paste this one.".into(),
                "Paste in PhotoCraft may use an image from the system clipboard instead.".into(),
            ],
            false,
            None,
        );
    }

    /// Hand a finished background conversion (see [`Self::export_os_clipboard`]) to the OS
    /// clipboard. Called every frame; keeps frames coming while a conversion runs.
    pub(crate) fn poll_os_clipboard_export(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.os_clip_export else { return };
        match rx.try_recv() {
            Ok((w, h, bytes)) => {
                self.os_clip_export = None;
                if std::mem::take(&mut self.os_clip_export_again) {
                    self.export_os_clipboard();
                } else {
                    self.set_os_clipboard_image(w, h, &bytes);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.os_clip_export = None;
                if std::mem::take(&mut self.os_clip_export_again) {
                    self.export_os_clipboard();
                }
            }
        }
    }

    fn set_os_clipboard_image(&mut self, w: u32, h: u32, bytes: &[u8]) {
        let Some(set) = self.services.clipboard_set_image.as_mut() else { return };
        if set(w, h, bytes).is_ok() {
            // A newer copy reached the system clipboard: the notice about an older one is stale.
            self.ui.notices.retain(|n| n.title != KEPT_IN_PHOTOCRAFT);
            self.os_clip_svg = None;
            self.os_clip_sig = Some(clip_signature(w, h, bytes));
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
        // Our own copy is still on its way to the OS clipboard: it is the newest, keep it.
        if self.os_clip_export.is_some() {
            return Ok(false);
        }
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
        // The copy the notice described has been replaced by the system clipboard's image.
        self.ui.notices.retain(|n| n.title != KEPT_IN_PHOTOCRAFT);
        Ok(true)
    }
}
