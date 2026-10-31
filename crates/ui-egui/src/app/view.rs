use super::PhotocraftApp;
use crate::state::WindowToggles;
use photocraft_doc::DocId;

/// The window whose View menu settings are the live ones: the main window showing a document,
/// or an extra window ("New Window for Document"), which keeps its own even for the same document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToggleOwner {
    Doc(DocId),
    Window(u64),
}

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

    /// The per-window settings a canvas of document `index` in `viewport` is drawn with: the
    /// live ones in the window that owns them, else the ones its window kept (the main window's
    /// tile of each document, or an extra window).
    pub(crate) fn window_toggles_in(&self, viewport: egui::ViewportId, index: usize) -> WindowToggles {
        let live = self.window_toggles();
        let window = self.ui.windows.iter().find(|w| crate::canvas::doc_window_viewport(w.id) == viewport).map(|w| ToggleOwner::Window(w.id));
        match window.or_else(|| self.session.documents().get(index).map(|d| ToggleOwner::Doc(d.doc.id))) {
            Some(key) if Some(key) != self.toggles_owner => self.stored_toggles(key).unwrap_or(live),
            _ => live,
        }
    }

    /// [`Self::window_toggles_in`] for the main window.
    pub(crate) fn window_toggles_of(&self, index: usize) -> WindowToggles {
        self.window_toggles_in(egui::ViewportId::ROOT, index)
    }

    /// The settings a new window for document `index` starts with: the live ones when the window
    /// that has them shows that document (the window it was opened from), else the document's
    /// own main-window ones.
    pub(crate) fn new_window_toggles(&self, index: usize) -> WindowToggles {
        let shows = match self.toggles_owner {
            Some(ToggleOwner::Window(id)) => self.ui.windows.iter().any(|w| w.id == id && w.document == index),
            Some(ToggleOwner::Doc(id)) => self.session.documents().get(index).is_some_and(|d| d.doc.id == id),
            None => true,
        };
        if shows { self.window_toggles() } else { self.window_toggles_of(index) }
    }

    fn stored_toggles(&self, key: ToggleOwner) -> Option<WindowToggles> {
        match key {
            ToggleOwner::Doc(id) => self.session.documents().iter().position(|d| d.doc.id == id).and_then(|i| self.ui.views.get(i)).and_then(|v| v.toggles),
            ToggleOwner::Window(id) => self.ui.windows.iter().find(|w| w.id == id).and_then(|w| w.view.toggles),
        }
    }

    fn toggles_slot(&mut self, key: ToggleOwner) -> Option<&mut Option<WindowToggles>> {
        match key {
            ToggleOwner::Doc(id) => {
                let i = self.session.documents().iter().position(|d| d.doc.id == id)?;
                self.ui.views.get_mut(i).map(|v| &mut v.toggles)
            }
            ToggleOwner::Window(id) => self.ui.windows.iter_mut().find(|w| w.id == id).map(|w| &mut w.view.toggles),
        }
    }

    /// Part of `sync_views`: when another window takes focus (the main window's active document,
    /// or an extra window), the one left keeps the live per-window settings and the new one
    /// brings back its own (one that has none yet keeps those in use, as a new document does).
    pub(super) fn sync_window_toggles(&mut self) {
        let focused = self.ui.windows.iter().find(|w| w.open && w.focused).map(|w| ToggleOwner::Window(w.id));
        let owner = focused.or_else(|| self.session.active().map(|d| ToggleOwner::Doc(d.doc.id)));
        if owner == self.toggles_owner {
            return;
        }
        let live = self.window_toggles();
        if let Some(slot) = self.toggles_owner.and_then(|o| self.toggles_slot(o)) {
            *slot = Some(live);
        }
        if let Some(t) = owner.and_then(|o| self.stored_toggles(o)) {
            self.set_window_toggles(t);
        }
        self.toggles_owner = owner;
    }
}
