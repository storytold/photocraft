//! `photocraft-cli mcp`: tools go to the open desktop app when the person lets agents use it,
//! found through its connection file, and otherwise to a session without a window, as `mcp`
//! always did (`--headless` forces that). The file is looked up again on every tool call, so the
//! app may start, quit or move to another port while the MCP client stays connected, and the MCP
//! client's own start never opens a window.
//!
//! While agents may use the app but it is closed, When PhotoCraft Is Closed (or `--when-closed`)
//! decides: start the app and wait for it, serve a headless session with the folders chosen in
//! the app, or report an error that says what to do. A session that went headless stays headless
//! for the rest of the MCP session, so documents don't move under the agent.
//!
//! The lookup itself is the framework's, shared by every Craft app
//! (`craft::automation::agents::Live`); this module supplies PhotoCraft's parts: its saved
//! preferences, its window client ([`BridgeClient`]) and its headless session ([`Headless`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft::automation::agents::{self, Consent, HeadlessFolders, Route, Window};
use photocraft_engine::prefs::WhenAppClosed;

use crate::AuthorizedWorkspace;
use crate::agent_link;
use crate::bridge::BridgeClient;
use crate::headless::Headless;

pub use craft::automation::agents::{LAUNCH_TIMEOUT, LaunchFn};

/// Prepares a headless session before its first call (see [`LiveConfig::prepare_headless`]).
pub type PrepareFn = Box<dyn Fn(&mut Headless) + Send + Sync>;

/// The app's name in the lookup's messages.
const APP_NAME: &str = "PhotoCraft";
/// The desktop app's file name beside the CLI (without `.exe`).
const APP_EXE: &str = "photocraft";

pub struct LiveConfig {
    /// The app's settings folder: connection file, token and preferences.
    pub config_dir: PathBuf,
    /// Overrides the app's preference (`--when-closed`).
    pub when_closed: Option<WhenAppClosed>,
    /// The folders a headless session may use (`--automation-read-root`, repeatable, and
    /// `--automation-write-root`); while agents may use the app, they override its folders.
    pub read_roots: Vec<PathBuf>,
    pub write_root: Option<PathBuf>,
    /// Starts the app; `None` when this install can't (Flatpak sandbox, no app beside the CLI).
    pub launch: Option<LaunchFn>,
    pub launch_timeout: Duration,
    /// Whether the person lets agents use the open app, given its saved preferences
    /// ([`agent_link::agents_allowed_now`]: the ArtCraft Launcher's switch); tests replace it.
    pub allowed: fn(&photocraft_engine::prefs::Integrations) -> bool,
    /// Prepares a headless session before its first call (the CLI's `--gpu` / `--no-gpu`).
    pub prepare_headless: Option<PrepareFn>,
}

impl LiveConfig {
    pub fn new(config_dir: PathBuf) -> Self {
        LiveConfig {
            config_dir,
            when_closed: None,
            read_roots: Vec::new(),
            write_root: None,
            launch: None,
            launch_timeout: LAUNCH_TIMEOUT,
            allowed: agent_link::agents_allowed_now,
            prepare_headless: None,
        }
    }
}

/// Where a tool call goes once [`Live::resolve`] has run.
#[derive(Clone)]
pub(crate) enum Target {
    Bridge(Arc<BridgeClient>),
    Headless(Arc<Mutex<Headless>>),
}

impl From<Route<Arc<BridgeClient>, Arc<Mutex<Headless>>>> for Target {
    fn from(route: Route<Arc<BridgeClient>, Arc<Mutex<Headless>>>) -> Self {
        match route {
            Route::Window(b) => Target::Bridge(b),
            Route::Headless(h) => Target::Headless(h),
        }
    }
}

type Shared = agents::Live<Arc<BridgeClient>, Arc<Mutex<Headless>>>;

pub struct Live {
    shared: Arc<Shared>,
}

impl Live {
    pub fn new(cfg: LiveConfig) -> Self {
        let LiveConfig { config_dir, when_closed, read_roots, write_root, launch, launch_timeout, allowed, prepare_headless } = cfg;
        let consent = move |dir: &Path| {
            let integrations = agent_link::read_integrations(dir);
            Consent::new(allowed(&integrations), integrations.agents)
        };
        let connect = |w: &Window| BridgeClient::new(w.addr(), w.token.clone()).map(Arc::new).map_err(|e| e.to_string());
        let headless = move |folders: &HeadlessFolders| {
            let workspace =
                AuthorizedWorkspace::with_read_roots(folders.read.iter().map(PathBuf::as_path), folders.write.as_deref()).map_err(|e| e.to_string())?;
            let mut session = Headless::with_workspace(workspace);
            if let Some(prepare) = &prepare_headless {
                prepare(&mut session);
            }
            Ok(Arc::new(Mutex::new(session)))
        };
        let mut shared = agents::LiveConfig::new(APP_NAME, config_dir, consent, connect, headless);
        shared.when_closed = when_closed;
        shared.read_roots = read_roots;
        shared.write_root = write_root;
        shared.launch = launch;
        shared.launch_timeout = launch_timeout;
        Live { shared: Arc::new(agents::Live::new(shared)) }
    }

    pub(crate) fn current(&self) -> Option<Target> {
        self.shared.current().map(Target::from)
    }

    /// Find the app (or apply the closed-app choice) before a tool call. The error is the tool's
    /// result: it tells the agent, and through it the user, what to do. The lookup blocks (file
    /// reads, a loopback probe, polling while the app starts), so it runs off the async runtime.
    pub(crate) async fn resolve(&self) -> Result<(), String> {
        let shared = self.shared.clone();
        tokio::task::spawn_blocking(move || shared.resolve().map(|_| ())).await.map_err(|e| format!("task failed: {e}"))?
    }
}

/// How to start the desktop app that ships with the CLI at `cli` (detached, with no standard
/// streams: the CLI's carry the MCP protocol, and the app must outlive the MCP client).
pub fn launcher(cli: &Path) -> Option<LaunchFn> {
    agents::launcher(cli, APP_EXE)
}
