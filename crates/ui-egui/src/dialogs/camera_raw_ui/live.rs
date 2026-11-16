//! What the develop view's preview shows while a mask is painted, and how up to date it is.

/// A finished overlay alpha: its mask, the edit it is of, the alpha (`None`: it failed).
pub(crate) type OverlayResult = (u32, u64, Option<Vec<u8>>);

/// Which edit each layer of the preview shows, and what the preview drew this frame (the control
/// channel's `live`).
#[derive(Default)]
pub(crate) struct Live {
    /// Counts the settings edits made on the preview (a brush dab, a handle moved).
    pub edit_seq: u64,
    /// The edit the proxy's texture shows.
    pub tex_seq: u64,
    /// The edit the mask overlay shows.
    pub overlay_seq: u64,
    /// What the preview drew this frame: `"window"` (the shared engine's render of what is on
    /// screen), `"windowDraft"`, `"proxy"` (the proxy at or above the screen's resolution) or
    /// `"proxyEnlarged"` (the proxy stretched: softer than the screen).
    pub shown: &'static str,
    /// The edit the photo on screen shows.
    pub photo_seq: u64,
    /// The preview's busy indicator was drawn this frame.
    pub spinner: bool,
    /// The overlay's mask alpha being rendered: (mask, edit, alpha).
    pub overlay_pending: Option<std::sync::mpsc::Receiver<OverlayResult>>,
    /// The mask and settings the overlay's alpha was last asked for.
    pub overlay_for: Option<(u32, craft_develop_model::DevelopSettings)>,
    /// Counts the overlay alphas taken in.
    pub overlay_generation: u64,
    /// The proxy in black and white (Color Overlay on B&W over the proxy) and the preview
    /// revision it is of.
    pub gray_proxy: Option<(egui::TextureHandle, u64)>,
}

impl Live {
    pub(crate) fn describe(&self) -> serde_json::Value {
        serde_json::json!({"editSeq": self.edit_seq, "photoSeq": self.photo_seq, "overlaySeq": self.overlay_seq, "shown": self.shown, "spinner": self.spinner})
    }
}
