//! Native tile storage configuration, kept out of the preference data model.

use crate::prefs::Preferences;

pub(crate) fn configure(prefs: &Preferences) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = prefs
            .scratch_disks
            .disks
            .iter()
            .find(|d| d.enabled && !d.path.trim().is_empty())
            .map(|d| if d.path.trim() == "(system temp)" { std::env::temp_dir() } else { std::path::PathBuf::from(d.path.trim()) });
        // An unusable directory disables eviction without dropping resident pixels.
        // The reason is exposed in ui.inspect and the application's status bar.
        let _ = photocraft_raster::spill::configure(dir.as_deref(), prefs.performance.tile_budget_bytes());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = prefs;
}
