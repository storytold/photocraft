use super::PhotocraftApp;
use crate::state::WindowToggles;

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

    /// The live per-window View menu settings: the active document window's.
    pub(crate) fn window_toggles(&self) -> WindowToggles {
        let (v, e) = (&self.ui.view, &self.ui.extras);
        WindowToggles { flip_horizontal: v.flip_horizontal, extras: v.extras, rulers: e.rulers, grid: e.grid, guides: e.guides }
    }

    fn set_window_toggles(&mut self, t: WindowToggles) {
        self.ui.view.flip_horizontal = t.flip_horizontal;
        self.ui.view.extras = t.extras;
        self.ui.extras.rulers = t.rulers;
        self.ui.extras.grid = t.grid;
        self.ui.extras.guides = t.guides;
    }

    /// The per-window settings document `index` is drawn with: the live ones while it is active,
    /// else the ones its window kept.
    pub(crate) fn window_toggles_of(&self, index: usize) -> WindowToggles {
        let live = self.window_toggles();
        if self.session.active_index() == Some(index) {
            return live;
        }
        self.ui.views.get(index).and_then(|v| v.toggles).unwrap_or(live)
    }

    /// Part of `sync_views`, before the views follow the documents: the window being left keeps
    /// the live per-window settings.
    pub(super) fn store_window_toggles(&mut self) {
        let active = self.session.active().map(|d| d.doc.id);
        if active == self.toggles_doc {
            return;
        }
        let live = self.window_toggles();
        if let Some(i) = self.toggles_doc.and_then(|old| self.view_docs.iter().position(|d| *d == old))
            && let Some(v) = self.ui.views.get_mut(i)
        {
            v.toggles = Some(live);
        }
    }

    /// Part of `sync_views`, after the views follow the documents: a newly active document's
    /// window brings back its own settings (a new one keeps those in use).
    pub(super) fn restore_window_toggles(&mut self) {
        let active = self.session.active().map(|d| d.doc.id);
        if active == self.toggles_doc {
            return;
        }
        self.toggles_doc = active;
        if let Some(t) = self.session.active_index().and_then(|i| self.ui.views.get(i)).and_then(|v| v.toggles) {
            self.set_window_toggles(t);
        }
    }
}
