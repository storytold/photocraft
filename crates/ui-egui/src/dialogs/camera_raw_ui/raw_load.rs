//! The open-time dialog's raw source, built on a worker in the desktop app: reading, decoding and
//! binning a large raw takes long enough to freeze the dialog if it ran in the UI frame. The dialog
//! opens on the document's own pixels and takes the sensor data when it lands.

use super::{CameraRawDialog, PROXY_SIDE, convert};
use crate::camera_raw_detail_ui::RawDetail;

/// What the worker built: the raw source, the binned frame developed as the document was (`None`
/// when its size isn't the proxy's), and the default develop box-averaged for Auto.
pub(super) struct RawLoaded {
    detail: RawDetail,
    proxy: Option<Vec<[f32; 4]>>,
    auto: Option<Vec<[f32; 4]>>,
}

/// The raw file to decode again: the bytes it was opened from, else its path.
pub(super) enum RawFile {
    Bytes(std::sync::Arc<Vec<u8>>),
    #[cfg(not(target_arch = "wasm32"))]
    Path(String),
}

impl RawFile {
    pub(super) fn of(raw: &super::RawOpen) -> Option<RawFile> {
        match (&raw.bytes, &raw.path) {
            (Some(b), _) => Some(RawFile::Bytes(b.clone())),
            #[cfg(not(target_arch = "wasm32"))]
            (None, Some(p)) => Some(RawFile::Path(p.clone())),
            _ => None,
        }
    }

    fn read(self) -> Option<std::sync::Arc<Vec<u8>>> {
        match self {
            RawFile::Bytes(b) => Some(b),
            #[cfg(not(target_arch = "wasm32"))]
            RawFile::Path(p) => std::fs::read(p).ok().map(std::sync::Arc::new),
        }
    }
}

impl CameraRawDialog {
    /// Starts building the raw source for `file` on a worker; [`Self::poll_raw_source`] attaches it.
    /// `demosaic` is the method the document was developed with, `to_document` the conversion of
    /// its ProPhoto pixels to the document's colours ([`convert::raw_to_document`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn load_raw_source(
        &mut self,
        ctx: &egui::Context,
        file: RawFile,
        demosaic: photocraft_io::raw::Demosaic,
        to_document: Option<std::sync::Arc<photocraft_cms::Transform>>,
    ) {
        let (w, h, pw, ph) = (self.full_w, self.full_h, self.pw, self.ph);
        let factor = w.max(h).div_ceil(PROXY_SIDE).max(1);
        let (surf, area) = self.detail.source();
        let surf = surf.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Option<RawLoaded>>();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new().name("camera-raw-source".into()).spawn(move || {
            let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let bytes = file.read()?;
                let source = photocraft_io::raw::RawSource::new(&bytes, factor).ok()?;
                let detail = RawDetail { source: std::sync::Arc::new(source), demosaic, to_document };
                let as_opened = photocraft_io::raw::RawTuning { demosaic, ..Default::default() };
                let proxy = convert::raw_proxy(&detail, &as_opened, pw, ph).ok().flatten().map(|(px, _)| px);
                // Auto reads the default develop at the preview's size ([`CameraRawDialog::auto_pixels`]).
                let auto = proxy.is_some().then(|| convert::box_proxy(&surf, area, w, h).0);
                Some(RawLoaded { detail, proxy, auto })
            }))
            .ok()
            .flatten();
            // The dialog may have closed meanwhile; then nobody listens.
            if tx.send(loaded).is_ok() {
                repaint.request_repaint();
            }
        });
        if spawned.is_ok() {
            self.raw_loading = Some(rx);
        }
    }

    /// Attaches the worker's raw source once it is done. Until then (or if the raw can't be read
    /// again) the preview adjusts the default develop, as for a raw it can't read.
    pub(super) fn poll_raw_source(&mut self) {
        let Some(rx) = &self.raw_loading else { return };
        let loaded = match rx.try_recv() {
            Ok(loaded) => loaded,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        };
        self.raw_loading = None;
        if let Some(loaded) = loaded {
            self.attach_raw_source(loaded);
        }
    }

    pub(crate) fn raw_source_pending(&self) -> bool {
        self.raw_loading.is_some()
    }

    fn attach_raw_source(&mut self, loaded: RawLoaded) {
        let RawLoaded { detail, proxy, auto } = loaded;
        self.raw_curve = Some(std::sync::Arc::new(detail.source.tone_curve().to_vec()));
        if let Some(px) = proxy {
            self.before_histogram = photocraft_algo::histogram::RgbHistogram::from_rgba(&px);
            self.proxy = std::sync::Arc::new(px);
            self.raw_proxy = true;
            // A new proxy: a new GPU upload and before image.
            self.source_key = crate::camera_raw_gpu::source_key();
            self.before_display_key = None;
            self.auto_proxy = Default::default();
            if let Some(auto) = auto {
                let _ = self.auto_proxy.set(auto);
            }
        }
        self.detail.set_raw(self.raw_curve.clone(), Some(detail.clone()));
        self.raw_source = Some(detail);
        self.dirty = true;
    }
}
