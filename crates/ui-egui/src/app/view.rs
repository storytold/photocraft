use super::PhotocraftApp;

impl PhotocraftApp {
    /// Zoom of the active document's main view: screen (device) pixels per document pixel, the
    /// user-facing factor (`100%` is `1.0`).
    pub fn current_zoom(&self) -> f32 {
        self.session.active_index().and_then(|i| self.ui.views.get(i)).map_or(1.0, |v| v.zoom)
    }

    /// Screen (device) pixels per egui point of the canvas (`ctx.pixels_per_point`), as of the
    /// last canvas frame. `1.0` before the first frame.
    pub fn canvas_ppp(&self) -> f32 {
        if self.ppp.is_finite() && self.ppp > 0.0 { self.ppp } else { 1.0 }
    }

    /// The active view's zoom in egui points per document pixel: the unit every screen-space
    /// distance, tolerance and texture-level choice on the canvas is measured in.
    pub fn point_zoom(&self) -> f32 {
        self.current_zoom() / self.canvas_ppp()
    }

    /// How much wider than tall the active document's pixels are drawn (see
    /// [`crate::view_cmds::ViewOptions::display_aspect`]); 1 with no document.
    pub fn display_aspect(&self) -> f32 {
        self.display_aspect_of(self.session.active_index())
    }

    /// [`Self::display_aspect`] for the document at `index`: each keeps its own pixel aspect ratio.
    pub fn display_aspect_of(&self, index: Option<usize>) -> f32 {
        self.ui.view.display_aspect(index.and_then(|i| self.session.documents().get(i)).map_or(1.0, |d| d.doc.pixel_aspect))
    }
}
