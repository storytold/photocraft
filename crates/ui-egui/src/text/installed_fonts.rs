//! Fonts installed while the app runs (a family the launcher puts in the suite's font store, or
//! one installed in the system's font folders): the catalog notices them (its store poll, and a
//! check on window activation), and each frame registers what is new for documents and re-renders
//! the type layers that use those families.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use photocraft_engine::TypeLayerRef;

use crate::PhotocraftApp;

/// The context the catalog's worker wakes; set by the first frame once [`start`] ran.
static WAKE: OnceLock<egui::Context> = OnceLock::new();
/// [`start`] ran: activations check the font folders.
static STARTED: OnceLock<()> = OnceLock::new();
/// The catalog generation [`tick`] last compared the shared engine's fonts with.
static CHECKED: AtomicU64 = AtomicU64::new(u64::MAX);
/// Layers whose document was busy with a background job when their font arrived.
static PENDING: Mutex<Vec<TypeLayerRef>> = Mutex::new(Vec::new());

/// Starts the font catalog (desktop start-up, before the first window). Until then [`tick`] and
/// [`on_input`] do nothing, so tests and the web build keep a catalog nothing polls.
pub(crate) fn start() {
    if STARTED.set(()).is_ok() {
        photocraft_text::fonts::start_catalog(|| {
            if let Some(ctx) = WAKE.get() {
                ctx.request_repaint();
            }
        });
    }
}

/// For the frame's raw input: a window activation checks the font folders.
pub(crate) fn on_input(raw_input: &egui::RawInput) {
    if STARTED.get().is_some() && raw_input.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true))) {
        photocraft_text::fonts::catalog_activated();
    }
}

/// Registers fonts the installed catalog gained since the last frame and re-renders the type layers that
/// use their families (a missing font that now resolves, a style just installed). Also retries
/// the layers of a document that was busy.
pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if STARTED.get().is_none() {
        return;
    }
    WAKE.get_or_init(|| ctx.clone());
    follow_catalog(app);
}

/// [`tick`]'s work once the catalog runs.
fn follow_catalog(app: &mut PhotocraftApp) {
    let mut layers = std::mem::take(&mut *PENDING.lock().unwrap_or_else(PoisonError::into_inner));
    let generation = photocraft_text::fonts::installed_catalog().generation();
    if generation != CHECKED.load(Ordering::Relaxed)
        && let Some(engine) = photocraft_text::try_shared()
    {
        CHECKED.store(generation, Ordering::Relaxed);
        let update = engine.lock().unwrap_or_else(PoisonError::into_inner).fonts.installed_update();
        if let Some(update) = update {
            // Picked before the fonts are in: whether a layer shows our own layout compares its
            // pixels with what the engine draws without them, and that takes the engine's lock.
            if !update.families.is_empty() {
                super::served_fonts::pick(&app.session, &update.families, &mut layers);
            }
            engine.lock().unwrap_or_else(PoisonError::into_inner).fonts.apply_installed(update);
        }
    }
    if !layers.is_empty() {
        let busy = app.session.refresh_type_layers(&layers);
        PENDING.lock().unwrap_or_else(PoisonError::into_inner).extend(busy);
    }
}

#[cfg(test)]
#[path = "installed_fonts_tests.rs"]
mod tests;
