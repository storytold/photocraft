//! Native tile storage configuration, kept out of the preference data model.

use crate::prefs::Preferences;

pub(crate) fn configure(prefs: &Preferences) {
    let budget = photocraft_raster::memory::configure(photocraft_raster::memory::Policy {
        percent: prefs.performance.memory_usage_percent,
        absolute_bytes: u64::from(prefs.performance.memory_usage_mb).saturating_mul(1 << 20),
    });
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
        let tiles = if std::env::var_os("PHOTOCRAFT_TILE_BUDGET_MB").is_some() {
            prefs.performance.tile_budget_bytes()
        } else {
            usize::try_from(budget.tile_bytes).unwrap_or(usize::MAX)
        };
        let _ = photocraft_raster::spill::configure(dir.as_deref(), tiles);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (prefs, budget);
}
