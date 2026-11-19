//! One-click MCP connector configuration for Claude Desktop, Claude Code, and Gemini CLI.
//!
//! Writes or merges the `photocraft` server entry (`photocraft-cli mcp`) into each client's
//! configuration file without clobbering existing MCP servers or settings.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// Supported external AI subscription clients that connect to PhotoCraft via MCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectorTarget {
    ClaudeDesktop,
    GeminiCli,
    ClaudeCodeCli,
}

impl ConnectorTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "Claude Desktop",
            Self::GeminiCli => "Gemini CLI",
            Self::ClaudeCodeCli => "Claude Code",
        }
    }
}

/// The MCP server entry written into client configs.
pub fn server_entry(cli_path: &Path) -> Value {
    json!({
        "command": cli_path.to_string_lossy(),
        "args": ["mcp"]
    })
}

/// A copyable CLI command or JSON snippet for `target`.
pub fn snippet(target: ConnectorTarget, cli_path: &Path) -> String {
    let cli = cli_path.to_string_lossy();
    match target {
        ConnectorTarget::ClaudeCodeCli => format!("claude mcp add photocraft -- \"{cli}\" mcp"),
        ConnectorTarget::ClaudeDesktop | ConnectorTarget::GeminiCli => {
            let doc = json!({
                "mcpServers": {
                    "photocraft": server_entry(cli_path)
                }
            });
            serde_json::to_string_pretty(&doc).unwrap_or_default()
        }
    }
}

/// Path to Claude Desktop's `claude_desktop_config.json` on the current platform.
pub fn claude_desktop_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Claude").join("claude_desktop_config.json"))
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home_dir().map(|h| h.join(".config")));
        base.map(|b| b.join("Claude/claude_desktop_config.json"))
    }
}

/// Path to Gemini CLI's `~/.gemini/settings.json`.
pub fn gemini_settings_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".gemini/settings.json"))
}

/// Merge `photocraft` into `target`'s JSON config file on disk.
pub fn install(target: ConnectorTarget, cli_path: &Path) -> Result<PathBuf, String> {
    let path = match target {
        ConnectorTarget::ClaudeDesktop => {
            claude_desktop_config_path().ok_or_else(|| "Could not locate home directory for Claude Desktop config.".to_owned())?
        }
        ConnectorTarget::GeminiCli => gemini_settings_path().ok_or_else(|| "Could not locate home directory for Gemini CLI settings.".to_owned())?,
        ConnectorTarget::ClaudeCodeCli => {
            return Err("Use the Copy Command button and run it in your terminal for Claude Code.".to_owned());
        }
    };
    merge_mcp_server_file(&path, "photocraft", server_entry(cli_path))?;
    Ok(path)
}

/// Safely read `path` (if present), merge `mcpServers[name] = entry`, and write it back.
pub fn merge_mcp_server_file(path: &Path, name: &str, entry: Value) -> Result<(), String> {
    let existing = if path.exists() {
        let raw = fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str::<Value>(&raw).map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?
        }
    } else {
        Value::Object(Map::new())
    };
    let merged = merge_mcp_server_value(existing, name, entry)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    let pretty = serde_json::to_string_pretty(&merged).map_err(|e| format!("Failed to serialize config: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, format!("{pretty}\n")).map_err(|e| format!("Failed to write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to replace {}: {e}", path.display()))?;
    Ok(())
}

/// Pure JSON merge helper: preserves all unrelated keys and existing `mcpServers`.
pub fn merge_mcp_server_value(mut root: Value, name: &str, entry: Value) -> Result<Value, String> {
    let obj = root.as_object_mut().ok_or_else(|| "Config root must be a JSON object.".to_owned())?;
    let servers = obj.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let servers_obj = servers.as_object_mut().ok_or_else(|| "`mcpServers` must be a JSON object.".to_owned())?;
    servers_obj.insert(name.to_owned(), entry);
    Ok(root)
}

pub(super) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_existing_servers_and_keys() {
        let existing = json!({
            "theme": "dark",
            "mcpServers": {
                "other": { "command": "other-cli", "args": [] }
            }
        });
        let entry = server_entry(Path::new("/usr/local/bin/photocraft-cli"));
        let merged = merge_mcp_server_value(existing, "photocraft", entry).unwrap();
        assert_eq!(merged["theme"], "dark");
        assert_eq!(merged["mcpServers"]["other"]["command"], "other-cli");
        assert_eq!(merged["mcpServers"]["photocraft"]["command"], "/usr/local/bin/photocraft-cli");
        assert_eq!(merged["mcpServers"]["photocraft"]["args"], json!(["mcp"]));
    }

    #[test]
    fn merge_creates_mcp_servers_when_missing() {
        let merged = merge_mcp_server_value(json!({}), "photocraft", server_entry(Path::new("photocraft-cli"))).unwrap();
        assert_eq!(merged["mcpServers"]["photocraft"]["args"], json!(["mcp"]));
    }

    #[test]
    fn snippet_formats_claude_code_command() {
        let s = snippet(ConnectorTarget::ClaudeCodeCli, Path::new("/opt/photocraft-cli"));
        assert_eq!(s, "claude mcp add photocraft -- \"/opt/photocraft-cli\" mcp");
    }
}
