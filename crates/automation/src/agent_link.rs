//! Connecting AI agents to the desktop app without a terminal (Edit › Preferences › AI
//! Integrations): finding the app's control server without being told, and adding PhotoCraft
//! to MCP clients.
//!
//! When the app starts its control server from the preferences (Allow Agent Control on and a
//! Control Port, no `--control`), it publishes where in its settings folder ([`CONNECTION_FILE`]):
//! the port, its process id and the bearer-token file ([`TOKEN_FILE`] unless a token file was
//! given at launch; created `0600` on Unix, see [`crate::security::server_token`]). A local
//! client reads that file, then the token from the file it names, so nothing has to pass a port
//! or a token to an app started from the Dock, the Start menu or a file manager.
//!
//! The file holds no secret. A crashed (or, on macOS, quit) app can leave it behind; nothing then
//! listens on its port ([`probe`]), and the app's next launch replaces it or removes it
//! ([`remove_stale_connection`]). Until then another program could take that port, so a client
//! checks that [`Connection::pid`] is still running before it sends the token. That narrows the
//! gap without closing it (a pid can be reused); `docs/control-protocol.md` says when a client
//! should not send the token on the strength of this file.
//!
//! While *Let AI agents connect* is on, the app starts and stops that server at once when the
//! setting changes. `photocraft-cli mcp --live` ([`crate::live`]) reads the connection file on
//! every tool call, so MCP client configurations never hold a port or a token, and keep working
//! when the app restarts on another port. [`install`] adds that command to an MCP client's own
//! configuration (Claude Desktop, Claude Code, Cursor) or returns the JSON to paste into any
//! other client; the app's buttons and `photocraft-cli mcp install` share it.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use photocraft_engine::prefs::Integrations;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::AutomationError;

/// The connection file in the settings folder, present while the app accepts agents.
pub const CONNECTION_FILE: &str = "agent-connection.json";
/// The bearer-token file the app keeps in its settings folder for agent connections.
pub const TOKEN_FILE: &str = "agent-control.token";
/// The server name MCP clients list PhotoCraft under.
pub const SERVER_NAME: &str = "photocraft";
/// How long [`probe`] waits for the app to accept a connection.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);
/// Version of the [`Connection`] file format.
const CONNECTION_VERSION: u32 = 1;
/// Largest connection file read; anything bigger is not ours.
const MAX_CONNECTION_BYTES: u64 = 1 << 20;

/// What the app publishes in [`CONNECTION_FILE`]. Holds no secret: the token stays in its own
/// user-only file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub version: u32,
    /// The app's process id: only that process removes the file.
    pub pid: u32,
    /// Loopback port of the control server.
    pub port: u16,
    pub token_file: PathBuf,
    /// The app's version, for diagnostics.
    pub app: String,
}

impl Connection {
    pub fn new(port: u16, token_file: PathBuf) -> Self {
        Connection { version: CONNECTION_VERSION, pid: std::process::id(), port, token_file, app: photocraft_engine::build_info::long_version() }
    }

    /// `127.0.0.1:<port>`.
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

fn io_error(path: &Path, e: impl std::fmt::Display) -> AutomationError {
    AutomationError::Io(format!("{}: {e}", path.display()))
}

/// Publish `c` in `dir` (written atomically, so a reader never sees half a file).
pub fn write_connection(dir: &Path, c: &Connection) -> Result<(), AutomationError> {
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, e))?;
    let path = dir.join(CONNECTION_FILE);
    let text = serde_json::to_string_pretty(c).map_err(|e| AutomationError::Other(e.to_string()))?;
    photocraft_format::atomic_write(&path, text.as_bytes()).map_err(|e| io_error(&path, e))
}

/// The connection published in `dir`, if any. A file from another version or one that doesn't
/// parse (invalid UTF-8 included) counts as none: the app rewrites it when it next starts
/// listening. Anything but a regular file (a FIFO would block the read) is an error, and left
/// alone.
pub fn read_connection(dir: &Path) -> Result<Option<Connection>, AutomationError> {
    let path = dir.join(CONNECTION_FILE);
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(&path, e)),
    };
    if !meta.is_file() {
        return Err(io_error(&path, "not a regular file"));
    }
    if meta.len() > MAX_CONNECTION_BYTES {
        return Err(io_error(&path, format!("larger than {MAX_CONNECTION_BYTES} bytes")));
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(&path, e)),
    };
    Ok(serde_json::from_slice::<Connection>(&bytes).ok().filter(|c| c.version == CONNECTION_VERSION && c.port != 0))
}

/// Remove the connection file in `dir` if process `pid` wrote it (another instance may have
/// published its own since). Errors are ignored: a stale file only fails [`probe`].
pub fn remove_connection(dir: &Path, pid: u32) {
    if let Ok(Some(c)) = read_connection(dir)
        && c.pid == pid
    {
        let _ = std::fs::remove_file(dir.join(CONNECTION_FILE));
    }
}

/// Remove the connection file in `dir` when nothing listens on its port any more (an app that
/// crashed or quit without removing it), or when it doesn't read as a connection. A live
/// instance's file is kept. Errors are ignored.
pub fn remove_stale_connection(dir: &Path) {
    let path = dir.join(CONNECTION_FILE);
    let stale = match read_connection(dir) {
        Ok(Some(c)) => !probe(c.port, PROBE_TIMEOUT),
        Ok(None) => path.is_file(),
        Err(_) => false,
    };
    if stale {
        let _ = std::fs::remove_file(path);
    }
}

/// Does something accept connections on loopback `port`? (A crashed app leaves its connection
/// file behind; nothing listens on its port any more.)
pub fn probe(port: u16, timeout: Duration) -> bool {
    port != 0 && TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeout).is_ok()
}

/// A folder preference as a path (`None` when empty or blank).
pub fn folder(pref: &str) -> Option<PathBuf> {
    let pref = pref.trim();
    (!pref.is_empty()).then(|| PathBuf::from(pref))
}

/// Largest client configuration file read; anything bigger is not ours.
const MAX_CONFIG_BYTES: u64 = 16 << 20;

/// Read a small text file, refusing anything larger than [`MAX_CONFIG_BYTES`] or anything but a
/// regular file (a FIFO would block the read).
fn read_small(path: &Path) -> Result<Option<String>, AutomationError> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(path, e)),
    };
    if !meta.is_file() {
        return Err(io_error(path, "not a regular file"));
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(io_error(path, format!("larger than {MAX_CONFIG_BYTES} bytes")));
    }
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(path, e)),
    }
}

/// The MCP preferences saved in `dir/preferences.json` (defaults when the file or
/// the section is missing or unreadable: the CLI must not fail over the app's settings).
pub fn read_integrations(dir: &Path) -> Integrations {
    read_small(&dir.join("preferences.json"))
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("integrations").cloned())
        .and_then(|mut v| {
            // As the app reads it: a Control Port from before *Let AI agents connect* keeps it on.
            photocraft_engine::prefs::migrate_integrations(&mut v);
            serde_json::from_value(v).ok()
        })
        .unwrap_or_default()
}

// ---- MCP clients --------------------------------------------------------------------------------

/// How an MCP client starts PhotoCraft's MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpLaunch {
    pub command: String,
    pub args: Vec<String>,
}

impl McpLaunch {
    /// `<cli> mcp --live`.
    pub fn cli(cli: &Path) -> Self {
        McpLaunch { command: cli.to_string_lossy().into_owned(), args: vec!["mcp".into(), "--live".into()] }
    }

    /// How to start the CLI that ships beside the executable in `exe_dir`. `appimage` is the
    /// AppImage this process runs from (`$APPIMAGE`): its mount point disappears when the app
    /// quits, so the AppImage itself starts the CLI (`--cli`, see `packaging/linux/AppRun`).
    pub fn beside(exe_dir: &Path, appimage: Option<&Path>) -> Self {
        match appimage {
            Some(image) => McpLaunch { command: image.to_string_lossy().into_owned(), args: vec!["--cli".into(), "mcp".into(), "--live".into()] },
            None => Self::cli(&exe_dir.join(cli_file_name())),
        }
    }

    /// The `mcpServers` entry (`{"command", "args"}`).
    pub fn entry(&self) -> Value {
        json!({"command": self.command, "args": self.args})
    }

    /// A complete configuration to paste into an MCP client.
    pub fn config_json(&self) -> String {
        let v = json!({"mcpServers": {SERVER_NAME: self.entry()}});
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// A prompt to paste into an agent that has a terminal (Claude Code, Codex, Cursor's agent):
    /// it registers PhotoCraft as its own MCP server, then checks the connection. Claude Desktop
    /// has no terminal, so it is set up by [`install`] instead. Holds no port and no token.
    pub fn agent_prompt(&self) -> String {
        let line = self.shell_line();
        let config = self.config_json();
        format!(
            "Please connect yourself to PhotoCraft, the image editor running on this computer, as an MCP server, then check that it works.
             
             1. Register a stdio MCP server named \"{SERVER_NAME}\" that runs this command:
                {line}
                - Claude Code: claude mcp add --scope user {SERVER_NAME} -- {line}
                - Codex: codex mcp add {SERVER_NAME} -- {line}
                - Any other client: add this to its MCP configuration:
             {config}
             2. Reload your MCP servers if your client needs that (or tell me to restart you).
             3. Call the {SERVER_NAME} tool `ui_inspect` and tell me which document is open.
             
             PhotoCraft must be running with Edit › Preferences › AI Integrations › Let AI agents connect turned on.              The command needs no port or token: it finds the app by itself on every call.
"
        )
    }

    /// The command line, quoted for a POSIX shell (Windows paths are double-quoted).
    pub fn shell_line(&self) -> String {
        std::iter::once(&self.command).chain(&self.args).map(|s| shell_quote(s)).collect::<Vec<_>>().join(" ")
    }
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._-+:,@=".contains(&b)) {
        s.to_string()
    } else if cfg!(windows) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// The CLI's file name on this platform.
pub fn cli_file_name() -> &'static str {
    if cfg!(windows) { "photocraft-cli.exe" } else { "photocraft-cli" }
}

/// MCP clients PhotoCraft can add itself to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Client {
    ClaudeDesktop,
    ClaudeCode,
    Cursor,
    /// No client: the JSON to paste into any MCP client.
    Json,
}

impl Client {
    pub const ALL: [Client; 4] = [Client::ClaudeDesktop, Client::ClaudeCode, Client::Cursor, Client::Json];

    pub fn id(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "claude-desktop",
            Client::ClaudeCode => "claude-code",
            Client::Cursor => "cursor",
            Client::Json => "json",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == s)
    }

    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "Claude Desktop",
            Client::ClaudeCode => "Claude Code",
            Client::Cursor => "Cursor",
            Client::Json => "MCP client",
        }
    }
}

/// The environment [`install`] looks at, so tests can point it at a temporary home.
pub struct ClientEnv {
    pub home: Option<PathBuf>,
    /// `%APPDATA%` (Windows).
    pub appdata: Option<PathBuf>,
    /// `%LOCALAPPDATA%` (Windows): Microsoft Store (MSIX) installs keep their settings in
    /// `Packages` there.
    pub local_appdata: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME` (Linux).
    pub xdg_config: Option<PathBuf>,
    /// `$PATH`, searched for the `claude` command.
    pub path: Option<std::ffi::OsString>,
}

/// The platform whose settings locations apply: a parameter, so tests cover all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Mac,
    Windows,
    Other,
}

impl Platform {
    const CURRENT: Platform = if cfg!(target_os = "macos") {
        Platform::Mac
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Other
    };
}

/// Where one install of a client keeps its MCP servers: `file`, in `folder`, which the client
/// creates when it first starts (so a missing folder means the client isn't set up there).
struct Location {
    folder: PathBuf,
    file: PathBuf,
}

const CLAUDE_DESKTOP_CONFIG: &str = "claude_desktop_config.json";

/// Microsoft Store (MSIX) packages of Claude Desktop, by folder-name prefix in
/// `%LOCALAPPDATA%\Packages`. Windows redirects a packaged app's `%APPDATA%` into its package
/// (`LocalCache\Roaming`), so such an install reads its configuration there and never sees
/// `%APPDATA%\Claude`.
const CLAUDE_PACKAGE_PREFIXES: [&str; 2] = ["Claude_", "Anthropic.Claude"];

impl ClientEnv {
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        ClientEnv {
            // Windows apps (Cursor, Claude) use the profile folder; `HOME` there is often a Unix
            // shell's (Git Bash, MSYS) and can point elsewhere.
            home: if cfg!(windows) { var("USERPROFILE").or_else(|| var("HOME")) } else { var("HOME").or_else(|| var("USERPROFILE")) }.map(PathBuf::from),
            appdata: var("APPDATA").map(PathBuf::from),
            local_appdata: var("LOCALAPPDATA").map(PathBuf::from),
            xdg_config: var("XDG_CONFIG_HOME").map(PathBuf::from),
            path: var("PATH"),
        }
    }

    /// The JSON files `client`'s installs on this computer read their MCP servers from (empty for
    /// Claude Code, which is configured through its own command, for [`Client::Json`], and when
    /// the client isn't set up here).
    pub fn config_files(&self, client: Client) -> Vec<PathBuf> {
        self.config_files_on(client, Platform::CURRENT)
    }

    fn config_files_on(&self, client: Client, platform: Platform) -> Vec<PathBuf> {
        self.locations(client, platform).into_iter().filter(|l| l.folder.is_dir()).map(|l| l.file).collect()
    }

    fn locations(&self, client: Client, platform: Platform) -> Vec<Location> {
        let at = |folder: PathBuf, name: &str| Location { file: folder.join(name), folder };
        match (client, platform) {
            (Client::ClaudeDesktop, Platform::Mac) => {
                self.home.iter().map(|h| at(h.join("Library").join("Application Support").join("Claude"), CLAUDE_DESKTOP_CONFIG)).collect()
            }
            (Client::ClaudeDesktop, Platform::Windows) => {
                let packaged = self
                    .claude_packages()
                    .into_iter()
                    .map(|package| Location { file: package.join("LocalCache").join("Roaming").join("Claude").join(CLAUDE_DESKTOP_CONFIG), folder: package });
                packaged.chain(self.appdata.iter().map(|a| at(a.join("Claude"), CLAUDE_DESKTOP_CONFIG))).collect()
            }
            (Client::ClaudeDesktop, Platform::Other) => self
                .xdg_config
                .clone()
                .or_else(|| self.home.as_ref().map(|h| h.join(".config")))
                .into_iter()
                .map(|c| at(c.join("Claude"), CLAUDE_DESKTOP_CONFIG))
                .collect(),
            (Client::Cursor, _) => self.home.iter().map(|h| at(h.join(".cursor"), "mcp.json")).collect(),
            (Client::ClaudeCode | Client::Json, _) => Vec::new(),
        }
    }

    /// Claude Desktop's Microsoft Store packages ([`CLAUDE_PACKAGE_PREFIXES`]).
    fn claude_packages(&self) -> Vec<PathBuf> {
        let Some(packages) = self.local_appdata.as_ref().map(|l| l.join("Packages")) else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(packages) else { return Vec::new() };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| CLAUDE_PACKAGE_PREFIXES.iter().any(|p| e.file_name().to_string_lossy().starts_with(p)))
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        found.sort();
        found
    }

    /// The `claude` command: on `PATH`, else where its installers put it (an app started from
    /// the Dock or Start menu has a minimal `PATH`).
    pub fn claude_command(&self) -> Option<PathBuf> {
        self.claude_command_on(Platform::CURRENT)
    }

    fn claude_command_on(&self, platform: Platform) -> Option<PathBuf> {
        // Windows: the native installer's `claude.exe`, or npm's `claude.cmd` script.
        let names: &[&str] = if platform == Platform::Windows { &["claude.exe", "claude.cmd"] } else { &["claude"] };
        let on_path: Vec<PathBuf> = self.path.iter().flat_map(std::env::split_paths).flat_map(|d| names.iter().map(move |n| d.join(n))).collect();
        let mut known: Vec<PathBuf> = Vec::new();
        if let Some(h) = &self.home {
            let native = h.join(".local").join("bin");
            known.extend(names.iter().map(|n| native.join(n)));
            known.push(h.join(".claude").join("local").join(names[0]));
        }
        if platform == Platform::Windows {
            known.extend(self.appdata.iter().map(|a| a.join("npm").join("claude.cmd")));
        } else {
            known.extend(["/opt/homebrew/bin/claude", "/usr/local/bin/claude"].map(PathBuf::from));
        }
        on_path.into_iter().chain(known).find(|p| p.is_file())
    }
}

/// What [`install`] did, for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// Wrote the configuration of each install of the client found (Claude Desktop on Windows
    /// can have a Microsoft Store one and a classic one).
    Files(Vec<Written>),
    /// Ran `claude mcp add`.
    ClaudeCode,
    /// The configuration to paste.
    Json(String),
}

/// One configuration file [`install`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub file: PathBuf,
    /// The previous version, kept beside it.
    pub backup: Option<PathBuf>,
    /// It already started PhotoCraft this way, so it was left alone.
    pub unchanged: bool,
}

impl Installed {
    /// One sentence for a status line or the terminal.
    pub fn message(&self, client: Client) -> String {
        let restart = match client {
            Client::ClaudeDesktop => " Quit Claude Desktop completely (not just its window) and open it again to connect.",
            Client::Cursor => " Restart Cursor to connect; PhotoCraft then shows under Customize › MCPs.",
            _ => "",
        };
        match self {
            Installed::Files(files) => {
                let list = files.iter().map(|w| w.file.display().to_string()).collect::<Vec<_>>().join("; ");
                if files.iter().all(|w| w.unchanged) {
                    format!("{} already starts PhotoCraft ({list}).{restart}", client.name())
                } else {
                    format!("Added PhotoCraft to {} ({list}).{restart}", client.name())
                }
            }
            Installed::ClaudeCode => "Added PhotoCraft to Claude Code (user scope). Start a new Claude Code session to connect.".into(),
            Installed::Json(text) => text.clone(),
        }
    }
}

/// Add PhotoCraft (started by `launch`) to `client`'s MCP servers, replacing an older
/// `photocraft` entry and keeping everything else in its configuration.
pub fn install(client: Client, launch: &McpLaunch, env: &ClientEnv) -> Result<Installed, AutomationError> {
    install_on(client, launch, env, Platform::CURRENT)
}

fn install_on(client: Client, launch: &McpLaunch, env: &ClientEnv, platform: Platform) -> Result<Installed, AutomationError> {
    match client {
        Client::Json => Ok(Installed::Json(launch.config_json())),
        Client::ClaudeCode => install_claude_code(launch, env),
        Client::ClaudeDesktop | Client::Cursor => {
            let files = env.config_files_on(client, platform);
            if files.is_empty() {
                return Err(not_set_up(client, &env.locations(client, platform)));
            }
            files.iter().map(|file| merge_into_file(file, launch)).collect::<Result<_, _>>().map(Installed::Files)
        }
    }
}

/// No install of `client` found: writing a configuration nobody reads would look like success.
fn not_set_up(client: Client, looked: &[Location]) -> AutomationError {
    let name = client.name();
    if looked.is_empty() {
        return AutomationError::Other(format!("can't find {name}'s settings folder (no home folder)"));
    }
    let folders = looked.iter().map(|l| l.folder.display().to_string()).collect::<Vec<_>>().join(" or ");
    AutomationError::Other(format!("{name} isn't set up on this computer (no {folders} folder): install it and open it once, then try again"))
}

/// Merge the `photocraft` entry into a JSON configuration with an `mcpServers` object.
fn merge_into_file(file: &Path, launch: &McpLaunch) -> Result<Written, AutomationError> {
    let existing = read_small(file)?;
    let mut root = match existing.as_deref().map(str::trim) {
        None | Some("") => Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(m)) => m,
            Ok(_) => return Err(io_error(file, "is not a JSON object; fix or remove it, then try again")),
            Err(e) => return Err(io_error(file, format!("is not valid JSON ({e}); fix or remove it, then try again"))),
        },
    };
    let servers = root.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(io_error(file, "`mcpServers` is not an object; fix it, then try again"));
    };
    let entry = launch.entry();
    if servers.get(SERVER_NAME) == Some(&entry) {
        return Ok(Written { file: file.to_path_buf(), backup: None, unchanged: true });
    }
    servers.insert(SERVER_NAME.into(), entry);
    let text = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| AutomationError::Other(e.to_string()))?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    // Keys are rewritten in sorted order, so keep the user's file as it was.
    let backup = match &existing {
        Some(old) => {
            let mut name = file.file_name().unwrap_or_default().to_os_string();
            name.push(".photocraft-backup");
            let backup = file.with_file_name(name);
            photocraft_format::atomic_write(&backup, old.as_bytes()).map_err(|e| io_error(&backup, e))?;
            Some(backup)
        }
        None => None,
    };
    photocraft_format::atomic_write(file, format!("{text}\n").as_bytes()).map_err(|e| io_error(file, e))?;
    Ok(Written { file: file.to_path_buf(), backup, unchanged: false })
}

/// Longest wait for one `claude` command: the Preferences button waits on it.
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(20);

/// Run `c` and collect its output, killing it after `timeout` (an error then).
fn output_within(mut c: std::process::Command, timeout: Duration) -> std::io::Result<std::process::Output> {
    let mut child = c.spawn()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("no answer within {} s", timeout.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn install_claude_code(launch: &McpLaunch, env: &ClientEnv) -> Result<Installed, AutomationError> {
    let by_hand = format!("claude mcp add --scope user {SERVER_NAME} -- {}", launch.shell_line());
    let claude =
        env.claude_command().ok_or_else(|| AutomationError::Other(format!("the `claude` command wasn't found; run this in a terminal instead: {by_hand}")))?;
    // npm installs `claude` as a Node script, and Node sits beside it (nvm, Homebrew): put that
    // folder first on `PATH`, which is minimal for an app started from the Dock or Start menu.
    let path = std::env::join_paths(claude.parent().map(Path::to_path_buf).into_iter().chain(env.path.iter().flat_map(std::env::split_paths))).ok();
    let run = |args: &[&str]| {
        let mut c = std::process::Command::new(&claude);
        c.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        if let Some(p) = &path {
            c.env("PATH", p);
        }
        output_within(c, CLAUDE_TIMEOUT)
    };
    let mut add = vec!["mcp", "add", "--scope", "user", SERVER_NAME, "--", launch.command.as_str()];
    add.extend(launch.args.iter().map(String::as_str));
    // Add first, so an older entry stays when adding can't work at all (no Node, a broken
    // install). `add` refuses a name that exists: only then replace that entry (ours, by name).
    let mut result = run(&add);
    let mut replaced = false;
    let exists = || run(&["mcp", "get", SERVER_NAME]).is_ok_and(|out| out.status.success());
    if result.as_ref().is_ok_and(|out| !out.status.success()) && exists() {
        // Say so when the older entry could not be removed: adding again would only fail with
        // "already exists", and the message must not claim the entry is gone.
        let why_not = match run(&["mcp", "remove", "--scope", "user", SERVER_NAME]) {
            Ok(out) if out.status.success() => None,
            Ok(out) => Some(String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string()),
            Err(e) => Some(format!("{}: {e}", claude.display())),
        };
        if let Some(why) = why_not {
            return Err(AutomationError::Other(format!(
                "Claude Code already has a `{SERVER_NAME}` entry and removing it failed ({why}); run this in a terminal to replace it: claude mcp remove --scope user {SERVER_NAME} && {by_hand}"
            )));
        }
        replaced = true;
        result = run(&add);
    }
    let why = match result {
        Ok(out) if out.status.success() => return Ok(Installed::ClaudeCode),
        Ok(out) => String::from_utf8_lossy(if out.stderr.is_empty() { &out.stdout } else { &out.stderr }).trim().to_string(),
        Err(e) => format!("{}: {e}", claude.display()),
    };
    if replaced {
        return Err(AutomationError::Other(format!(
            "adding PhotoCraft to Claude Code failed ({why}) after its older `{SERVER_NAME}` entry was removed; run this in a terminal to add it again: {by_hand}"
        )));
    }
    Err(AutomationError::Other(format!("adding PhotoCraft to Claude Code failed ({why}); run this in a terminal instead: {by_hand}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-agent-link-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn env_at(home: &Path) -> ClientEnv {
        ClientEnv {
            home: Some(home.to_path_buf()),
            appdata: Some(home.join("AppData").join("Roaming")),
            local_appdata: Some(home.join("AppData").join("Local")),
            xdg_config: None,
            path: None,
        }
    }

    #[test]
    fn connection_round_trips_and_only_its_writer_removes_it() {
        let dir = temp("conn");
        assert_eq!(read_connection(&dir).unwrap(), None);
        let c = Connection::new(4242, dir.join(TOKEN_FILE));
        write_connection(&dir, &c).unwrap();
        assert_eq!(read_connection(&dir).unwrap(), Some(c.clone()));
        assert_eq!(c.addr(), "127.0.0.1:4242");
        remove_connection(&dir, c.pid.wrapping_add(1));
        assert!(dir.join(CONNECTION_FILE).exists(), "another instance's file stays");
        remove_connection(&dir, c.pid);
        assert!(!dir.join(CONNECTION_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_foreign_connection_file_counts_as_none() {
        let dir = temp("bad-conn");
        for text in
            ["", "{", "[]", r#"{"version":99,"pid":1,"port":1,"tokenFile":"t","app":"x"}"#, r#"{"version":1,"pid":1,"port":0,"tokenFile":"t","app":"x"}"#]
        {
            std::fs::write(dir.join(CONNECTION_FILE), text).unwrap();
            assert_eq!(read_connection(&dir).unwrap(), None, "{text}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_sees_a_listener_and_not_a_closed_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(probe(port, PROBE_TIMEOUT));
        drop(listener);
        assert!(!probe(port, PROBE_TIMEOUT));
        assert!(!probe(0, PROBE_TIMEOUT));
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
        assert_eq!(i.when_app_closed, photocraft_engine::prefs::WhenAppClosed::Headless);
        assert_eq!(folder(&i.read_root), Some(PathBuf::from("/in")));
        assert_eq!(folder(&i.write_root), None);
        assert_eq!(folder("  "), None);
        // A Control Port saved before *Let AI agents connect* reads as on, as the app reads it.
        std::fs::write(dir.join("preferences.json"), r#"{"integrations": {"controlPort": 7878}}"#).unwrap();
        assert!(read_integrations(&dir).connect_agents);
        std::fs::write(dir.join("preferences.json"), r#"{"integrations": {"controlPort": 7878, "connectAgents": false}}"#).unwrap();
        assert!(!read_integrations(&dir).connect_agents);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_commands_and_config_json() {
        let l = McpLaunch::cli(Path::new("/opt/PhotoCraft/photocraft-cli"));
        assert_eq!(l.args, ["mcp", "--live"]);
        let v: Value = serde_json::from_str(&l.config_json()).unwrap();
        assert_eq!(v["mcpServers"]["photocraft"]["command"], "/opt/PhotoCraft/photocraft-cli");
        assert_eq!(v["mcpServers"]["photocraft"]["args"], json!(["mcp", "--live"]));
        let beside = McpLaunch::beside(Path::new("/opt/PhotoCraft"), None);
        assert!(beside.command.ends_with(cli_file_name()), "{}", beside.command);
        let image = McpLaunch::beside(Path::new("/tmp/.mount_x/usr/bin"), Some(Path::new("/home/u/PhotoCraft.AppImage")));
        assert_eq!(image.command, "/home/u/PhotoCraft.AppImage");
        assert_eq!(image.args, ["--cli", "mcp", "--live"]);
        if !cfg!(windows) {
            assert_eq!(McpLaunch::cli(Path::new("/a b/it's")).shell_line(), r"'/a b/it'\''s' mcp --live");
        }
        for c in Client::ALL {
            assert_eq!(Client::parse(c.id()), Some(c));
        }
        assert_eq!(Client::parse("vscode"), None);
    }

    #[test]
    fn install_adds_the_entry_and_keeps_other_servers_and_settings() {
        let launch = McpLaunch::cli(Path::new("/opt/PhotoCraft/photocraft-cli"));
        for platform in [Platform::Mac, Platform::Windows, Platform::Other] {
            for client in [Client::ClaudeDesktop, Client::Cursor] {
                let home = temp("install");
                let env = env_at(&home);
                let [location] = &env.locations(client, platform)[..] else { panic!("{client:?} on {platform:?}") };
                let file = location.file.clone();
                assert!(file.starts_with(&home), "{}", file.display());
                // The client created its folder when it first started; a fresh install creates the file.
                std::fs::create_dir_all(&location.folder).unwrap();
                let first = install_on(client, &launch, &env, platform).unwrap();
                assert_eq!(first, Installed::Files(vec![Written { file: file.clone(), backup: None, unchanged: false }]));
                let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
                assert_eq!(v["mcpServers"]["photocraft"], launch.entry());
                // Other servers and settings survive; an old photocraft entry is replaced.
                std::fs::write(&file, r#"{"theme": "dark", "mcpServers": {"other": {"command": "x"}, "photocraft": {"command": "old"}}}"#).unwrap();
                let again = install_on(client, &launch, &env, platform).unwrap();
                let Installed::Files(written) = &again else { panic!("{again:?}") };
                let [Written { backup: Some(backup), unchanged: false, .. }] = &written[..] else { panic!("{again:?}") };
                assert!(std::fs::read_to_string(backup).unwrap().contains("\"old\""), "the previous file is kept");
                let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
                assert_eq!(v["theme"], "dark");
                assert_eq!(v["mcpServers"]["other"]["command"], "x");
                assert_eq!(v["mcpServers"]["photocraft"], launch.entry());
                // Installing again changes nothing.
                let same = install_on(client, &launch, &env, platform).unwrap();
                assert!(matches!(&same, Installed::Files(w) if w.iter().all(|w| w.unchanged)), "{same:?}");
                assert!(again.message(client).contains(client.name()));
                assert!(same.message(client).contains("already"));
                let _ = std::fs::remove_dir_all(&home);
            }
        }
    }

    #[test]
    fn a_client_that_was_never_opened_gets_no_file_and_an_explanation() {
        let launch = McpLaunch::cli(Path::new("/x/photocraft-cli"));
        for platform in [Platform::Mac, Platform::Windows, Platform::Other] {
            for client in [Client::ClaudeDesktop, Client::Cursor] {
                let home = temp("not-set-up");
                let env = env_at(&home);
                let e = install_on(client, &launch, &env, platform).unwrap_err().to_string();
                assert!(e.contains(client.name()) && e.contains("open it once"), "{e}");
                assert!(env.locations(client, platform).iter().all(|l| e.contains(&l.folder.display().to_string()) && !l.file.exists()), "{e}");
                let _ = std::fs::remove_dir_all(&home);
            }
        }
    }

    /// Claude Desktop from the Microsoft Store reads `%APPDATA%` inside its package
    /// (anthropics/claude-code#26073), so the entry goes there, and to a classic install too.
    #[test]
    fn claude_desktop_from_the_microsoft_store_gets_the_entry_in_its_package() {
        let home = temp("msix");
        let env = env_at(&home);
        let launch = McpLaunch::cli(Path::new("C:/PhotoCraft/photocraft-cli.exe"));
        let packages = home.join("AppData").join("Local").join("Packages");
        let store = packages.join("Claude_pzs8sxrjxfjjc");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(packages.join("Microsoft.WindowsCalculator_8wekyb3d8bbwe")).unwrap();
        let packaged = store.join("LocalCache").join("Roaming").join("Claude").join(CLAUDE_DESKTOP_CONFIG);
        let classic = home.join("AppData").join("Roaming").join("Claude").join(CLAUDE_DESKTOP_CONFIG);
        // Installed from the Store and never opened: the package is enough to know where it reads.
        assert_eq!(env.config_files_on(Client::ClaudeDesktop, Platform::Windows), std::slice::from_ref(&packaged));
        let done = install_on(Client::ClaudeDesktop, &launch, &env, Platform::Windows).unwrap();
        assert_eq!(done, Installed::Files(vec![Written { file: packaged.clone(), backup: None, unchanged: false }]));
        assert!(!classic.exists(), "no file where this install never looks");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&packaged).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["photocraft"], launch.entry());
        // A classic install as well (or Claude's own Edit Config file): both get the entry.
        std::fs::create_dir_all(classic.parent().unwrap()).unwrap();
        let both = install_on(Client::ClaudeDesktop, &launch, &env, Platform::Windows).unwrap();
        let Installed::Files(written) = &both else { panic!("{both:?}") };
        assert_eq!(written.iter().map(|w| (w.file.clone(), w.unchanged)).collect::<Vec<_>>(), [(packaged.clone(), true), (classic.clone(), false)]);
        let message = both.message(Client::ClaudeDesktop);
        assert!(message.contains(&packaged.display().to_string()) && message.contains(&classic.display().to_string()), "{message}");
        // Other platforms never look in Packages.
        assert!(env.config_files_on(Client::ClaudeDesktop, Platform::Other).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn install_refuses_to_overwrite_a_file_it_cannot_read_as_json() {
        let home = temp("install-bad");
        let env = env_at(&home);
        let launch = McpLaunch::cli(Path::new("/x/photocraft-cli"));
        let file = home.join(".cursor").join("mcp.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        for text in ["{ broken", "[1, 2]", r#"{"mcpServers": []}"#] {
            std::fs::write(&file, text).unwrap();
            let e = install(Client::Cursor, &launch, &env).unwrap_err().to_string();
            assert!(e.contains("try again"), "{e}");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "left untouched");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn claude_code_without_the_claude_command_names_the_command_to_run() {
        let home = temp("claude-code");
        let env = ClientEnv { path: Some(home.join("empty").into_os_string()), ..env_at(&home) };
        if env.claude_command().is_some() {
            return; // A system-wide `claude` (Homebrew, /usr/local) is installed here.
        }
        let e = install(Client::ClaudeCode, &McpLaunch::cli(Path::new("/x/photocraft-cli")), &env).unwrap_err().to_string();
        assert!(e.contains("claude mcp add --scope user photocraft -- /x/photocraft-cli mcp --live"), "{e}");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The prompt an agent pastes names the exact command, the Claude Code and Codex lines and a
    /// configuration for any other client, and no token.
    #[test]
    fn the_agent_prompt_registers_the_live_command() {
        let launch = McpLaunch::cli(Path::new("/x/photocraft-cli"));
        let prompt = launch.agent_prompt();
        assert!(prompt.contains("claude mcp add --scope user photocraft -- /x/photocraft-cli mcp --live"), "{prompt}");
        assert!(prompt.contains("codex mcp add photocraft -- /x/photocraft-cli mcp --live"), "{prompt}");
        assert!(prompt.contains(&launch.config_json()), "{prompt}");
        assert!(prompt.contains("Edit › Preferences › AI Integrations › Let AI agents connect"), "{prompt}");
        assert!(!prompt.contains(TOKEN_FILE) && !prompt.contains("token\":"), "{prompt}");
    }

    #[test]
    fn claude_command_is_found_where_its_installers_put_it() {
        let home = temp("claude-find");
        let env = ClientEnv { path: Some(home.join("empty").into_os_string()), ..env_at(&home) };
        let place = |p: PathBuf| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "").unwrap();
            p
        };
        assert_eq!(env.claude_command_on(Platform::Windows), None);
        // npm on Windows: a `claude.cmd` script in %APPDATA%\npm.
        let npm = place(home.join("AppData").join("Roaming").join("npm").join("claude.cmd"));
        assert_eq!(env.claude_command_on(Platform::Windows), Some(npm));
        // The native installer wins over npm, on every platform.
        let native = place(home.join(".local").join("bin").join("claude.exe"));
        assert_eq!(env.claude_command_on(Platform::Windows), Some(native));
        let unix = place(home.join(".local").join("bin").join("claude"));
        assert_eq!(env.claude_command_on(Platform::Mac), Some(unix));
        // `PATH` first.
        let on_path = place(home.join("tools").join("claude.cmd"));
        let env = ClientEnv { path: Some(home.join("tools").into_os_string()), ..env_at(&home) };
        assert_eq!(env.claude_command_on(Platform::Windows), Some(on_path));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A fake `claude` records what it was run with: `mcp add` with the launch command after
    /// `--`, and its own folder first on `PATH` (where npm's Node is).
    #[cfg(unix)]
    #[test]
    fn claude_code_install_runs_claude_mcp_add() {
        use std::os::unix::fs::PermissionsExt;
        let home = temp("claude-run");
        let bin = home.join(".local").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = home.join("calls.txt");
        let fail = home.join("fail");
        let exists = home.join("exists");
        let keep = home.join("keep");
        // `add` fails while an entry exists (as claude's does) or while `fail` is there; `remove`
        // removes the entry, or fails while `keep` is there. `PATH` holds only claude's folder and
        // an empty one, so anything that isn't a shell builtin needs its full path.
        let script = format!(
            "#!/bin/sh\nprintf 'PATH=%s\\n' \"$PATH\" >> '{log}'\nprintf '%s ' \"$@\" >> '{log}'\nprintf '\\n' >> '{log}'\n\
             if [ \"$2\" = remove ] && [ -e '{keep}' ]; then echo 'locked' >&2; exit 1; fi\n\
             if [ \"$2\" = remove ]; then /bin/rm -f '{exists}'; exit 0; fi\n\
             if [ \"$2\" = get ]; then [ -e '{exists}' ]; exit $?; fi\n\
             if [ \"$2\" = add ] && [ -e '{exists}' ]; then echo 'already exists' >&2; exit 1; fi\n\
             if [ \"$2\" = add ] && [ -e '{fail}' ]; then echo 'boom' >&2; exit 1; fi\n\
             if [ \"$2\" = add ]; then : > '{exists}'; fi\n",
            log = log.display(),
            fail = fail.display(),
            exists = exists.display(),
            keep = keep.display()
        );
        let claude = bin.join("claude");
        std::fs::write(&claude, script).unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = ClientEnv { path: Some(home.join("empty").into_os_string()), ..env_at(&home) };
        let launch = McpLaunch::cli(Path::new("/x/photocraft-cli"));
        // Writing an executable while other test threads fork can briefly leave it busy (ETXTBSY).
        let run = || {
            for _ in 0..50 {
                match install(Client::ClaudeCode, &launch, &env) {
                    Err(e) if e.to_string().contains("busy") => std::thread::sleep(std::time::Duration::from_millis(20)),
                    r => return r,
                }
            }
            install(Client::ClaudeCode, &launch, &env)
        };
        // No entry yet: added, nothing removed.
        assert_eq!(run().unwrap(), Installed::ClaudeCode);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(!calls.contains("mcp remove"), "{calls}");
        assert!(calls.contains("mcp add --scope user photocraft -- /x/photocraft-cli mcp --live \n"), "{calls}");
        assert!(calls.contains(&format!("PATH={}:", bin.display())), "claude's folder comes first: {calls}");
        // An entry exists: replaced.
        assert_eq!(run().unwrap(), Installed::ClaudeCode);
        assert!(std::fs::read_to_string(&log).unwrap().contains("mcp remove --scope user photocraft \n"));
        assert!(exists.exists());
        // An entry exists and adding can't work: the error says the older entry went, and how to add it.
        std::fs::write(&fail, "").unwrap();
        let e = run().unwrap_err().to_string();
        assert!(e.contains("was removed") && e.contains("claude mcp add --scope user photocraft -- /x/photocraft-cli mcp --live"), "{e}");
        // No entry and adding can't work: nothing is removed.
        std::fs::write(&log, "").unwrap();
        let e = run().unwrap_err().to_string();
        assert!(e.contains("boom") && !e.contains("was removed"), "{e}");
        assert!(!std::fs::read_to_string(&log).unwrap().contains("mcp remove"));
        // An entry exists and removing it fails: the error says so, and does not claim it went.
        std::fs::remove_file(&fail).unwrap();
        std::fs::write(&exists, "").unwrap();
        std::fs::write(&keep, "").unwrap();
        let e = run().unwrap_err().to_string();
        assert!(e.contains("removing it failed (locked)") && !e.contains("was removed"), "{e}");
        assert!(e.contains("claude mcp remove --scope user photocraft && claude mcp add"), "{e}");
        assert!(exists.exists(), "the older entry is still there");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A `claude` that never answers can't hang the Preferences button.
    #[cfg(unix)]
    #[test]
    fn a_command_that_hangs_is_stopped_after_the_timeout() {
        let mut c = std::process::Command::new("sleep");
        c.arg("30").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let t = std::time::Instant::now();
        let e = output_within(c, Duration::from_millis(200)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn json_client_returns_the_configuration() {
        let launch = McpLaunch::cli(Path::new("/x/photocraft-cli"));
        let env = ClientEnv { home: None, appdata: None, local_appdata: None, xdg_config: None, path: None };
        let Installed::Json(text) = install(Client::Json, &launch, &env).unwrap() else { panic!() };
        assert_eq!(text, launch.config_json());
        assert!(install(Client::Cursor, &launch, &env).is_err(), "no home folder: an error, not a panic");
    }

    // ---- connection file (Preferences › AI Integrations) ----

    #[test]
    fn a_published_connection_reads_back_in_the_documented_shape() {
        let dir = temp("conn");
        let c = Connection::new(7878, dir.join(TOKEN_FILE));
        write_connection(&dir, &c).unwrap();
        assert_eq!(read_connection(&dir).unwrap(), Some(c.clone()));
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(CONNECTION_FILE)).unwrap()).unwrap();
        let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["app", "pid", "port", "tokenFile", "version"]);
        assert_eq!((v["version"].as_u64(), v["port"].as_u64()), (Some(1), Some(7878)));
        assert_eq!(c.addr(), "127.0.0.1:7878");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn other_versions_garbage_and_port_zero_read_as_none() {
        let dir = temp("conn");
        assert_eq!(read_connection(&dir).unwrap(), None, "no file");
        for text in [
            &b"{not json"[..],
            br#"{"version":2,"pid":1,"port":7878,"tokenFile":"t","app":"x"}"#,
            br#"{"version":1,"pid":1,"port":0,"tokenFile":"t","app":"x"}"#,
            b"\xff\xfe not utf-8",
        ] {
            std::fs::write(dir.join(CONNECTION_FILE), text).unwrap();
            assert_eq!(read_connection(&dir).unwrap(), None, "{}", String::from_utf8_lossy(text));
        }
        std::fs::remove_file(dir.join(CONNECTION_FILE)).unwrap();
        // Not a regular file: an error (and `remove_stale_connection` leaves it), never a read
        // that could block on a FIFO.
        std::fs::create_dir(dir.join(CONNECTION_FILE)).unwrap();
        assert!(read_connection(&dir).is_err());
        remove_stale_connection(&dir);
        assert!(dir.join(CONNECTION_FILE).is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_publishing_process_removes_its_file() {
        let dir = temp("conn");
        let c = Connection { pid: 1, ..Connection::new(7878, dir.join(TOKEN_FILE)) };
        write_connection(&dir, &c).unwrap();
        remove_connection(&dir, 2);
        assert!(dir.join(CONNECTION_FILE).is_file(), "another process's file stays");
        remove_connection(&dir, 1);
        assert!(!dir.join(CONNECTION_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_file_is_removed_and_a_live_one_kept() {
        let dir = temp("conn");
        let live = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let live_port = live.local_addr().unwrap().port();
        write_connection(&dir, &Connection { pid: 1, ..Connection::new(live_port, dir.join(TOKEN_FILE)) }).unwrap();
        remove_stale_connection(&dir);
        assert!(dir.join(CONNECTION_FILE).is_file(), "something listens on {live_port}");
        drop(live);
        // A port nothing listens on: bind one, then close it.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        write_connection(&dir, &Connection { pid: 1, ..Connection::new(closed, dir.join(TOKEN_FILE)) }).unwrap();
        remove_stale_connection(&dir);
        assert!(!dir.join(CONNECTION_FILE).exists(), "nothing listens on {closed}");
        for text in [&b"{not json"[..], b"\xff\xfe not utf-8"] {
            std::fs::write(dir.join(CONNECTION_FILE), text).unwrap();
            remove_stale_connection(&dir);
            assert!(!dir.join(CONNECTION_FILE).exists(), "an unreadable file is not a connection");
        }
        remove_stale_connection(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_preferences_are_trimmed_and_empty_means_none() {
        assert_eq!(folder(""), None);
        assert_eq!(folder("   "), None);
        assert_eq!(folder(" /work/project "), Some(PathBuf::from("/work/project")));
    }
}
