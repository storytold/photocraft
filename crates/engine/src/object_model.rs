//! The seam between Object Selection and a learned segmentation model.
//!
//! The engine knows nothing about how the model runs: the frontend supplies an [`ObjectModel`]
//! (the desktop app runs the shared `select-subject` tool, SAM 2.1). Pictures cross the seam as
//! raw pixels, so this module is free of codecs, network and platform code and builds on wasm.

/// A segmentation model provided by the frontend (the desktop app runs the shared `select-subject` tool).
pub trait ObjectModel: Send + Sync {
    /// `None` when it can run now; otherwise why not, in words for a person. Cheap (the
    /// implementation caches).
    fn unavailable(&self) -> Option<String>;

    /// Segments `req`; `cancelled` is polled while waiting. Never panics.
    fn segment(&self, req: &SegmentRequest, cancelled: &(dyn Fn() -> bool + Sync)) -> std::result::Result<SegmentAnswer, String>;
}

/// One picture and what to find in it.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentRequest {
    /// Straight 8-bit RGB, row-major: `width * height * 3` bytes.
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// A box around the object, as fractions `0..1` of the picture: `[x0, y0, x1, y1]`.
    pub area: Option<[f32; 4]>,
    /// Clicks as `(x, y, positive)`, with `x` and `y` fractions `0..1` of the picture.
    pub clicks: Vec<(f32, f32, bool)>,
}

/// What the model found.
#[derive(Clone, Debug, PartialEq)]
pub struct SegmentAnswer {
    /// `width * height` bytes of the request's picture, `0..=255`, 255 = selected.
    pub mask: Vec<u8>,
    /// Whether the model found an object at all.
    pub matched: bool,
}
