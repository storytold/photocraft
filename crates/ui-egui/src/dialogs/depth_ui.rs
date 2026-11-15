//! Estimate Depth: the Surface popover's button.
//!
//! The estimate runs `image.estimateDepth`, which uses the shared Craft AI layer's `estimate-depth`
//! tool. When the tool is not set up, [`native`] draws the shared layer's own screens over its
//! state (the missing-model popup, then `craft_ai_egui::native_model_setup` over a `SetupSession`),
//! so setup, the download and the consent to it are the shared flow, not a second one here.
//! Everything that reads or writes disk or the network runs on a worker thread. The web build has
//! no AI layer: the button is disabled there.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod web {
    use serde_json::Value;

    use crate::PhotocraftApp;

    /// Nothing to keep: there is no setup flow on the web.
    #[derive(Default)]
    pub struct DepthAi;

    pub fn start(app: &mut PhotocraftApp) {
        app.ui.status = tl!("Not available in this build").into();
        app.ui.status_error = true;
    }

    pub fn show(_app: &mut PhotocraftApp, _ctx: &egui::Context) {}

    /// There is no setup window to close.
    pub fn cancel_stale(_app: &mut PhotocraftApp) {}

    pub fn inspect(_app: &PhotocraftApp) -> Value {
        Value::Null
    }

    /// Why the button is off, for its tooltip.
    pub fn unavailable_reason(_app: &PhotocraftApp) -> Option<String> {
        Some(tl!("Not available in this build").into())
    }
}
#[cfg(target_arch = "wasm32")]
pub use web::*;
