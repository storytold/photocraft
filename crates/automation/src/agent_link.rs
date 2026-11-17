//! PhotoCraft's side of agents in the open desktop app: its agent settings as `photocraft-cli mcp`
//! reads them from the saved preferences, and whether the person lets agents in now. The switch,
//! the connection file and the link host are the framework's (`craft::automation::agents`): the
//! app serves its control channel on loopback while agents may connect and publishes where in its
//! settings folder, and `photocraft-cli mcp` ([`crate::live`]) reads that file on every tool call,
//! so MCP client configurations never hold a port or a token. Adding PhotoCraft to MCP clients is
//! the ArtCraft Launcher's job (one entry for every Craft app).

use std::path::Path;

use craft::automation::agents;
use photocraft_engine::prefs::Integrations;

/// The MCP preferences saved in `dir`, read as the app's settings store loads them
/// (`photocraft_engine::prefs::read_saved`: `config.toml`, or before it the older
/// `preferences.json`). Defaults when nothing is saved or the file is unreadable: the CLI must not
/// fail over the app's settings.
pub fn read_integrations(dir: &Path) -> Integrations {
    photocraft_engine::prefs::read_saved(Some(dir)).prefs.integrations
}

// ---- The agents switch -------------------------------------------------------------------------

/// The id the ArtCraft Launcher's app list names PhotoCraft by.
pub const LAUNCHER_APP_ID: &str = "photocraft";

/// Whether agents may use the open window now ([`agents::agents_allowed`]): with the suite's
/// `AI-MCP` flag the launcher's switch, without it a Control Port saved in the preferences
/// (`connect_agents`).
pub fn agents_allowed_now(integrations: &Integrations) -> bool {
    agents::agents_allowed(agents::mcp_flag(), agents::launcher_allows(LAUNCHER_APP_ID), integrations.connect_agents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-agent-link-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn integrations_come_from_the_saved_preferences_or_default() {
        let dir = temp("prefs");
        assert_eq!(read_integrations(&dir), Integrations::default());
        std::fs::write(dir.join("preferences.json"), "not json").unwrap();
        assert_eq!(read_integrations(&dir), Integrations::default());
        // A section of the wrong type doesn't fail the CLI either.
        std::fs::write(dir.join("preferences.json"), r#"{"integrations": {"connectAgents": "yes"}}"#).unwrap();
        assert_eq!(read_integrations(&dir), Integrations::default());
        std::fs::write(
            dir.join("preferences.json"),
            r#"{"general": {}, "integrations": {"connectAgents": true, "whenAppClosed": "headless", "readRoot": "/in"}}"#,
        )
        .unwrap();
        let i = read_integrations(&dir);
        assert!(i.connect_agents);
        assert_eq!(i.agents.when_app_closed, photocraft_engine::prefs::WhenAppClosed::Headless);
        assert_eq!(i.agents.read_folder(), Some(PathBuf::from("/in")));
        assert_eq!(i.agents.write_folder(), None);
        // A Control Port saved before the agents switch reads as on, as the app reads it.
        std::fs::write(dir.join("preferences.json"), r#"{"integrations": {"controlPort": 7878}}"#).unwrap();
        assert!(read_integrations(&dir).connect_agents);
        std::fs::write(dir.join("preferences.json"), r#"{"integrations": {"controlPort": 7878, "connectAgents": false}}"#).unwrap();
        assert!(!read_integrations(&dir).connect_agents);
        // One bad value resets only itself, as in the app: the saved closed-app choice stays.
        std::fs::write(dir.join("preferences.json"), r#"{"integrations": {"whenAppClosed": "error", "readRoot": 5}}"#).unwrap();
        assert_eq!(read_integrations(&dir).agents.when_app_closed, photocraft_engine::prefs::WhenAppClosed::Error);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
