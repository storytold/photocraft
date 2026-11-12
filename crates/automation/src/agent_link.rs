//! Finding the desktop app's control server without being told (Preferences › Integrations).
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

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::AutomationError;

/// The connection file in the settings folder, present while the app accepts agents.
pub const CONNECTION_FILE: &str = "agent-connection.json";
/// The bearer-token file the app keeps in its settings folder for agent connections.
pub const TOKEN_FILE: &str = "agent-control.token";
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-agent-link-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_published_connection_reads_back_in_the_documented_shape() {
        let dir = temp();
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
        let dir = temp();
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
        let dir = temp();
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
        let dir = temp();
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
