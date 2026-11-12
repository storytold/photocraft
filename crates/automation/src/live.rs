//! `photocraft-cli mcp --live`: tools go to the desktop app found through its connection file
//! ([`crate::agent_link`]). The file is looked up again on every tool call, so the app may start,
//! quit or move to another port while the MCP client stays connected, and the MCP client's own
//! start never opens a window.
//!
//! While the app is closed, MCP › When PhotoCraft Is Closed (or `--when-closed`)
//! decides: start the app and wait for it, serve a headless session with the folders chosen in
//! the app (then for the rest of the MCP session, so documents don't move under the agent), or
//! report an error that says what to do.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use photocraft_engine::prefs::{Integrations, WhenAppClosed};

use crate::agent_link::{self, Connection, PROBE_TIMEOUT};
use crate::bridge::BridgeClient;
use crate::headless::Headless;
use crate::{AuthorizedWorkspace, security};

/// Starts the desktop app (see [`launcher`]).
pub type LaunchFn = Box<dyn Fn() -> Result<(), String> + Send + Sync>;

/// How long a started app gets to accept agents. A first start can take a while (GPU driver
/// shader caches, font scan), but MCP clients give a tool call about a minute.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(250);

pub struct LiveConfig {
    /// The app's settings folder: connection file, token and preferences.
    pub config_dir: PathBuf,
    /// Overrides the app's preference (`--when-closed`).
    pub when_closed: Option<WhenAppClosed>,
    /// Override the app's folders for a headless session (`--automation-read-root`,
    /// `--automation-write-root`).
    pub read_root: Option<PathBuf>,
    pub write_root: Option<PathBuf>,
    /// Starts the app; `None` when this install can't (Flatpak sandbox, no app beside the CLI).
    pub launch: Option<LaunchFn>,
    pub launch_timeout: Duration,
}

impl LiveConfig {
    pub fn new(config_dir: PathBuf) -> Self {
        LiveConfig { config_dir, when_closed: None, read_root: None, write_root: None, launch: None, launch_timeout: LAUNCH_TIMEOUT }
    }
}

/// Where a tool call goes once [`Live::resolve`] has run.
#[derive(Clone)]
pub(crate) enum Target {
    Bridge(Arc<BridgeClient>),
    Headless(Arc<Mutex<Headless>>),
}

/// The port and token a bridge client was made for: when the app restarts, either changes.
type BridgeKey = (u16, String, u32);

pub struct Live {
    cfg: LiveConfig,
    /// Serialises lookups, so concurrent calls start the app once.
    lookup: tokio::sync::Mutex<()>,
    /// The current target, and for the bridge the port and token it was made for.
    current: Mutex<Option<(Target, Option<BridgeKey>)>>,
}

impl Live {
    pub fn new(cfg: LiveConfig) -> Self {
        Live { cfg, lookup: tokio::sync::Mutex::new(()), current: Mutex::new(None) }
    }

    pub(crate) fn current(&self) -> Option<Target> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|(t, _)| t.clone())
    }

    fn set(&self, target: Target, key: Option<BridgeKey>) {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = Some((target, key));
    }

    /// Find the app (or apply the closed-app choice) before a tool call. The error is the tool's
    /// result: it tells the agent, and through it the user, what to do.
    pub(crate) async fn resolve(&self) -> Result<(), String> {
        let _one_at_a_time = self.lookup.lock().await;
        if matches!(self.current(), Some(Target::Headless(_))) {
            return Ok(());
        }
        let dir = self.cfg.config_dir.clone();
        let (found, integrations) = blocking(move || (find_app(&dir), agent_link::read_integrations(&dir))).await?;
        if let Some((c, token)) = found? {
            self.use_app(&c, token)?;
            return Ok(());
        }
        match self.cfg.when_closed.unwrap_or(integrations.when_app_closed) {
            WhenAppClosed::Error => Err(closed(&integrations)),
            WhenAppClosed::Headless => {
                let read = self.cfg.read_root.clone().or_else(|| agent_link::folder(&integrations.read_root));
                let write = self.cfg.write_root.clone().or_else(|| agent_link::folder(&integrations.write_root));
                let workspace = AuthorizedWorkspace::new(read.as_deref(), write.as_deref())
                    .map_err(|e| format!("PhotoCraft isn't running and its agent folders can't be used: {e}"))?;
                self.set(Target::Headless(Arc::new(Mutex::new(Headless::with_workspace(workspace)))), None);
                Ok(())
            }
            WhenAppClosed::Launch => self.launch_and_wait(&integrations).await,
        }
    }

    async fn launch_and_wait(&self, integrations: &Integrations) -> Result<(), String> {
        if !integrations.connect_agents {
            return Err(closed(integrations));
        }
        let Some(launch) = &self.cfg.launch else {
            return Err("PhotoCraft isn't running, and this install can't start it from here: open PhotoCraft, then try again.".into());
        };
        launch().map_err(|e| format!("couldn't start PhotoCraft: {e}"))?;
        let deadline = Instant::now() + self.cfg.launch_timeout;
        loop {
            tokio::time::sleep(POLL).await;
            let dir = self.cfg.config_dir.clone();
            if let Some((c, token)) = blocking(move || find_app(&dir)).await?? {
                return self.use_app(&c, token);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "PhotoCraft didn't accept agents within {} s of starting. If it's still opening, try again in a moment; otherwise check Edit › Preferences › AI Integrations.",
                    self.cfg.launch_timeout.as_secs()
                ));
            }
        }
    }

    fn use_app(&self, c: &Connection, token: String) -> Result<(), String> {
        // The pid too: a restarted app keeps its port and token, but the old socket is dead.
        let key: BridgeKey = (c.port, token, c.pid);
        let same = self.current.lock().unwrap_or_else(PoisonError::into_inner).as_ref().is_some_and(|(_, k)| k.as_ref() == Some(&key));
        if !same {
            let client = BridgeClient::new(c.addr(), key.1.clone()).map_err(|e| e.to_string())?;
            self.set(Target::Bridge(Arc::new(client)), Some(key));
        }
        Ok(())
    }
}

/// Run blocking file and socket work off the async runtime.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("task failed: {e}"))
}

/// The running app's connection and token: `None` when no app accepts agents (no connection
/// file, or one left by an app that crashed).
fn find_app(dir: &Path) -> Result<Option<(Connection, String)>, String> {
    let Some(c) = agent_link::read_connection(dir).map_err(|e| e.to_string())? else { return Ok(None) };
    if !agent_link::probe(c.port, PROBE_TIMEOUT) {
        return Ok(None);
    }
    let token = security::client_token(None, Some(&c.token_file)).map_err(|e| format!("PhotoCraft's agent token: {e}"))?;
    Ok(Some((c, token)))
}

fn closed(integrations: &Integrations) -> String {
    if integrations.connect_agents {
        "PhotoCraft isn't running: open it, then try again.".into()
    } else {
        "PhotoCraft doesn't accept AI agents: open it, turn on Edit › Preferences › AI Integrations › Let AI agents connect, then try again.".into()
    }
}

/// How to start the desktop app that ships with the CLI at `cli` (detached, with no standard
/// streams: the CLI's carry the MCP protocol, and the app must outlive the MCP client).
pub fn launcher(cli: &Path) -> Option<LaunchFn> {
    use std::process::{Command, Stdio};
    if std::env::var_os("FLATPAK_ID").is_some() {
        return None;
    }
    let command: Command = if let Some(image) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        // The AppImage's mount point goes away with the CLI; the image itself stays.
        Command::new(image)
    } else {
        let dir = cli.parent()?;
        let bundle = dir.parent().filter(|_| dir.ends_with("Contents/MacOS")).and_then(Path::parent);
        match bundle {
            // A macOS app bundle: LaunchServices starts (or brings forward) the app.
            Some(app) if cfg!(target_os = "macos") => {
                let mut c = Command::new("/usr/bin/open");
                c.arg("-a").arg(app);
                c
            }
            _ => {
                let app = dir.join(if cfg!(windows) { "photocraft.exe" } else { "photocraft" });
                if !app.is_file() {
                    return None;
                }
                if cfg!(windows) {
                    let (program, args) = windows_launch(&app, std::env::var_os("SystemRoot").as_deref());
                    let mut c = Command::new(program);
                    c.args(args);
                    c
                } else {
                    Command::new(app)
                }
            }
        }
    };
    let command = Mutex::new(command);
    Some(Box::new(move || {
        let mut c = command.lock().unwrap_or_else(PoisonError::into_inner);
        c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        detach(&mut c);
        c.spawn().map(|_| ()).map_err(|e| e.to_string())
    }))
}

/// How Windows starts the app at `app`: through Explorer, as a double-click would. The app is then
/// no child of the CLI, so it outlives the MCP client ending the CLI's process tree (Claude Code
/// does on Windows), and it runs outside the container of a Microsoft Store client (Claude
/// Desktop): there, new files under `%APPDATA%` land in the client's package, where the app
/// started from the Start menu would never see the settings saved meanwhile. Explorer reads a
/// comma as a separator, so such a path is started directly. (`%WINDIR%` can be empty under a
/// Store client, hence `%SystemRoot%`.)
fn windows_launch(app: &Path, system_root: Option<&std::ffi::OsStr>) -> (PathBuf, Vec<std::ffi::OsString>) {
    if app.to_string_lossy().contains(',') {
        return (app.to_path_buf(), Vec::new());
    }
    let root = system_root.filter(|r| !r.is_empty()).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    (root.join("explorer.exe"), vec![app.as_os_str().to_os_string()])
}

/// Keep the app running when the MCP client stops the CLI (and its process group).
fn detach(c: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = c;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn windows_starts_the_app_through_explorer() {
        let app = Path::new(r"C:\Program Files\PhotoCraft\photocraft.exe");
        let (program, args) = windows_launch(app, Some(OsStr::new(r"D:\Win")));
        assert_eq!(program, Path::new(r"D:\Win").join("explorer.exe"));
        assert_eq!(args, [app.as_os_str()]);
        // No usable %SystemRoot%: the default Windows folder.
        let default = Path::new(r"C:\Windows").join("explorer.exe");
        assert_eq!(windows_launch(app, Some(OsStr::new(""))).0, default);
        assert_eq!(windows_launch(app, None).0, default);
        // Explorer would split this path at the comma.
        let comma = Path::new(r"C:\Apps, tools\photocraft.exe");
        assert_eq!(windows_launch(comma, None), (comma.to_path_buf(), Vec::new()));
    }
}
