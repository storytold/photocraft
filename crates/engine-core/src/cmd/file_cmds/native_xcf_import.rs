//! Session-backed Open and raster Place Embedded explicitly admit native XCF
//! tone preparation. Smart/frame/linked and sessionless IO helpers stay inert.
use crate::{EngineError, Result};
use serde_json::Value;

type Gate = fn(&str, &Value) -> Result<()>;

pub(crate) fn with_authority<T>(gate: Option<Gate>, import: impl FnOnce() -> T) -> T {
    // None is the existing trusted-local Session policy only because this
    // function is called from an actual Session-backed import entrypoint.
    // Existing nested/action restrictions remain installed when gate is None.
    let _request = crate::jobs::render_authorization_scope(gate);
    let _import = photocraft_io::native_xcf_tones::enter_import(authorize);
    import()
}

impl crate::Session {
    /// Scope one synchronous import with this session's native XCF command gate.
    /// Acquire source bytes through the caller's filesystem capability first:
    /// this scope grants no file access and does not admit or recolor a document.
    /// Ordinary sessionless IO remains inert, and nested restrictions are preserved.
    pub fn with_native_xcf_import_authority<T>(&self, import: impl FnOnce() -> T) -> T {
        with_authority(self.authorize, import)
    }
}

fn authorize(command: &str, params: &Value) -> std::result::Result<(), photocraft_io::native_xcf_tones::AuthorizationError> {
    match crate::jobs::authorize_render_command(command, params) {
        Ok(()) => Ok(()),
        Err(EngineError::Cancelled) => Err(photocraft_io::native_xcf_tones::AuthorizationError::Cancelled),
        Err(_) => Err(photocraft_io::native_xcf_tones::AuthorizationError::Denied),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mask_tests;

#[cfg(test)]
mod levels_tests;

#[cfg(test)]
mod point_color_tests;

#[cfg(test)]
mod spatial_tests;
