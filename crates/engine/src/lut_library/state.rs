//! Favourite and recently used LUTs, kept in `<library>/.state.json`. A LUT is identified by
//! `Pack/path/in/pack.cube`. The file is small and read once; a missing, oversized or corrupt
//! file is an empty state, and every write is atomic.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{LutLibrary, clean_component, is_lut, write_atomic};

const STATE_FILE: &str = ".state.json";
const MAX_STATE_BYTES: u64 = 1 << 20;
/// Most favourites kept.
pub const MAX_FAVORITES: usize = 1000;
/// Most recently used LUTs kept.
pub const MAX_RECENT: usize = 12;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct State {
    #[serde(default)]
    pub favorites: Vec<String>,
    #[serde(default)]
    pub recent: Vec<String>,
}

impl State {
    pub(super) fn load(root: &Path) -> Self {
        let path = root.join(STATE_FILE);
        if fs::metadata(&path).map_or(true, |m| m.len() > MAX_STATE_BYTES) {
            return State::default();
        }
        fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub(super) fn save(&self, root: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            let _ = fs::create_dir_all(root);
            let _ = write_atomic(&root.join(STATE_FILE), &bytes);
        }
    }

    pub(super) fn forget_pack(&mut self, pack: &str) {
        let prefix = format!("{pack}/");
        self.favorites.retain(|id| !id.starts_with(&prefix));
        self.recent.retain(|id| !id.starts_with(&prefix));
    }
}

impl LutLibrary {
    /// Whether `id` names an installed LUT (`Pack/path.cube`, no way out of the library).
    pub fn is_installed(&self, id: &str) -> bool {
        let Some((pack, file)) = id.split_once('/') else { return false };
        if pack.is_empty() || clean_component(pack) != pack || pack.starts_with('.') {
            return false;
        }
        let parts: Vec<&str> = file.split('/').collect();
        if parts.iter().any(|p| p.is_empty() || *p == "." || *p == ".." || p.starts_with('.') || p.contains(['\\', ':'])) || !is_lut(Path::new(file)) {
            return false;
        }
        self.path_of(pack, file).is_file()
    }

    /// Favourite LUT ids, in the order they were added.
    pub fn favorites(&self) -> &[String] {
        &self.state.favorites
    }

    /// Recently used LUT ids, newest first.
    pub fn recent(&self) -> &[String] {
        &self.state.recent
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.state.favorites.iter().any(|f| f == id)
    }

    /// Add or remove a favourite.
    pub fn set_favorite(&mut self, id: &str, on: bool) -> Result<(), String> {
        if on && !self.is_installed(id) {
            return Err(format!("`{id}` is not an installed LUT"));
        }
        self.state.favorites.retain(|f| f != id);
        if on {
            if self.state.favorites.len() >= MAX_FAVORITES {
                return Err(format!("at most {MAX_FAVORITES} favourites"));
            }
            self.state.favorites.push(id.to_string());
        }
        self.state.save(&self.root);
        self.rev += 1;
        Ok(())
    }

    /// Record that a LUT was used (moves it to the front of the recent list).
    pub fn note_used(&mut self, id: &str) -> Result<(), String> {
        if !self.is_installed(id) {
            return Err(format!("`{id}` is not an installed LUT"));
        }
        if self.state.recent.first().is_some_and(|r| r == id) {
            return Ok(());
        }
        self.state.recent.retain(|r| r != id);
        self.state.recent.insert(0, id.to_string());
        self.state.recent.truncate(MAX_RECENT);
        self.state.save(&self.root);
        self.rev += 1;
        Ok(())
    }

    /// Drop favourites and recent entries whose file is gone (after hand edits).
    pub fn prune_state(&mut self) {
        let (f, r) = (self.state.favorites.len(), self.state.recent.len());
        let keep: Vec<String> = self.state.favorites.iter().filter(|id| self.is_installed(id)).cloned().collect();
        let recent: Vec<String> = self.state.recent.iter().filter(|id| self.is_installed(id)).cloned().collect();
        self.state.favorites = keep;
        self.state.recent = recent;
        if f != self.state.favorites.len() || r != self.state.recent.len() {
            self.state.save(&self.root);
            self.rev += 1;
        }
    }
}
