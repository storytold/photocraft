//! One window per user on Linux and the BSDs (#1540, #1266).
//!
//! A file manager opens a file by launching `photocraft <path>`, so every Open With started a
//! second app in its own window. Now the first instance listens on a Unix socket; a later launch
//! sends its paths there, waits for the answer and exits, and the running window opens them as
//! tabs (through [`OsEvent::Open`], as macOS's Apple events do) and comes forward.
//!
//! - **Where:** `$XDG_RUNTIME_DIR` (per user, mode 0700; shared by all instances of a Flatpak
//!   app), else the settings directory. The name carries a hash of the settings directory, so
//!   separate `PHOTOCRAFT_CONFIG_DIR`s (tests, agents, portable copies) stay separate apps.
//! - **Protocol:** the client writes [`MAGIC`], then each absolute path's bytes followed by a NUL,
//!   then shuts down its write half; the server answers `ok\n` once the paths are queued. A
//!   connection without the header (a liveness probe, anything else) is ignored. Input is capped
//!   ([`MAX_BYTES`], [`MAX_PATHS`]).
//! - **Never in the way:** a socket nobody accepts on is stale and replaced; no answer within
//!   [`ANSWER_TIMEOUT`] (a hung instance) or any error starts this launch normally.
//!   `--new-instance` or `PHOTOCRAFT_NEW_INSTANCE=1` always starts a new window, and so does
//!   `--control` (automation drives its own instance).
//!
//! Only std: no new dependency and no `unsafe`.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use photocraft_ui_egui::{OsEvent, OsEventsFn};

/// How long a launch waits for the running instance to take its paths.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
/// Largest request a server reads (paths and separators).
pub const MAX_BYTES: usize = 1 << 20;
/// Most paths a server takes from one request.
pub const MAX_PATHS: usize = 4096;
const ANSWER: &[u8] = b"ok\n";
/// Starts every request (protocol version 1).
pub const MAGIC: &[u8] = b"photocraft-open-1\0";

/// The socket for the settings directory `config_dir`: in `runtime_dir` when there is one, else
/// in `config_dir` itself. `None` without either.
pub fn socket_path(runtime_dir: Option<&Path>, config_dir: Option<&Path>) -> Option<PathBuf> {
    let config = config_dir?;
    // FNV-1a: stable across builds and platforms (std's hasher is seeded per process).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in config.as_os_str().as_bytes() {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    let name = format!("photocraft-{h:016x}.sock");
    Some(match runtime_dir.filter(|d| d.is_absolute()) {
        Some(run) => run.join(name),
        None => config.join(name),
    })
}

/// Whether this launch should hand its files to a running instance.
pub fn should_forward(new_instance_flag: bool, env: Option<OsString>, control: bool) -> bool {
    let env_on = env.is_some_and(|v| !v.is_empty() && v != "0");
    !(new_instance_flag || env_on || control)
}

/// A request's bytes for `paths`.
pub fn encode(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    for p in paths {
        out.extend_from_slice(p.as_os_str().as_bytes());
        out.push(0);
    }
    out
}

/// The paths in a request: NUL-terminated, empty entries skipped, at most [`MAX_PATHS`]. `None`
/// when it doesn't start with [`MAGIC`].
pub fn decode(bytes: &[u8]) -> Option<Vec<PathBuf>> {
    let bytes = bytes.strip_prefix(MAGIC)?;
    Some(bytes.split(|b| *b == 0).filter(|p| !p.is_empty()).take(MAX_PATHS).map(|p| PathBuf::from(OsString::from_vec(p.to_vec()))).collect())
}

/// What happened to a launch's attempt to forward.
#[derive(Debug, PartialEq, Eq)]
pub enum Forward {
    /// The running instance took the paths: exit.
    Delivered,
    /// Nobody is listening (or it didn't answer): start normally.
    NoInstance,
}

/// Send `paths` (absolute) to the instance listening on `socket`.
pub fn forward(socket: &Path, paths: &[PathBuf]) -> Forward {
    let Ok(mut stream) = UnixStream::connect(socket) else { return Forward::NoInstance };
    let sent = stream.set_write_timeout(Some(ANSWER_TIMEOUT)).is_ok()
        && stream.set_read_timeout(Some(ANSWER_TIMEOUT)).is_ok()
        && stream.write_all(&encode(paths)).is_ok()
        && stream.shutdown(std::net::Shutdown::Write).is_ok();
    if !sent {
        return Forward::NoInstance;
    }
    let mut answer = Vec::new();
    match stream.take(ANSWER.len() as u64).read_to_end(&mut answer) {
        Ok(_) if answer == ANSWER => Forward::Delivered,
        _ => Forward::NoInstance,
    }
}

/// The paths received from later launches, until the UI drains them.
#[derive(Clone, Default)]
pub struct Inbox {
    inner: Arc<Mutex<InboxState>>,
}

#[derive(Default)]
struct InboxState {
    queue: Vec<OsEvent>,
    ctx: Option<egui::Context>,
}

impl Inbox {
    fn push(&self, paths: Vec<PathBuf>) {
        let mut s = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        s.queue.push(OsEvent::Open(paths.iter().map(|p| p.to_string_lossy().into_owned()).collect()));
        if let Some(ctx) = &s.ctx {
            ctx.request_repaint();
        }
    }

    /// The queue the UI drains every frame (`Services::os_events`); new requests wake `ctx`.
    pub fn connect(&self, ctx: &egui::Context) -> OsEventsFn {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner).ctx = Some(ctx.clone());
        let inner = self.inner.clone();
        Box::new(move || std::mem::take(&mut inner.lock().unwrap_or_else(PoisonError::into_inner).queue))
    }
}

/// The listening socket; removes its file when dropped (hold it until the event loop returns).
pub struct Server {
    path: PathBuf,
    pub inbox: Inbox,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Listen on `socket` for later launches. `None` when another instance already listens there or
/// the socket can't be made (this instance then just runs on its own).
pub fn listen(socket: &Path) -> Option<Server> {
    let listener = match UnixListener::bind(socket) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // A live instance answers a connect; a crashed one left a file nobody accepts on.
            if UnixStream::connect(socket).is_ok() {
                return None;
            }
            std::fs::remove_file(socket).ok()?;
            UnixListener::bind(socket).ok()?
        }
        Err(e) => {
            log::info!("single instance: can't listen on {}: {e}", socket.display());
            return None;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Owner only, also when it lands in the settings directory.
        if let Err(e) = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)) {
            log::warn!("single instance: {e}");
        }
    }
    let inbox = Inbox::default();
    let queue = inbox.clone();
    let spawned = std::thread::Builder::new().name("single instance".into()).spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => serve(s, &queue),
                Err(e) => log::warn!("single instance: {e}"),
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("single instance: couldn't start the listener: {e}");
        let _ = std::fs::remove_file(socket);
        return None;
    }
    Some(Server { path: socket.to_path_buf(), inbox })
}

/// Read one request and queue its paths. A slow or broken client can't hold the listener.
fn serve(mut stream: UnixStream, inbox: &Inbox) {
    if stream.set_read_timeout(Some(ANSWER_TIMEOUT)).is_err() || stream.set_write_timeout(Some(ANSWER_TIMEOUT)).is_err() {
        return;
    }
    let mut bytes = Vec::new();
    if (&mut stream).take(MAX_BYTES as u64).read_to_end(&mut bytes).is_err() {
        return;
    }
    // Only absolute paths: a relative one would resolve against this process's directory.
    let Some(paths) = decode(&bytes) else { return };
    let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.is_absolute()).collect();
    // An empty request (launched without files) still brings the window forward.
    inbox.push(paths);
    let _ = stream.write_all(ANSWER);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-si-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn drain(inbox: &Inbox) -> Vec<OsEvent> {
        let ctx = egui::Context::default();
        let mut f = inbox.connect(&ctx);
        // The listener thread queues before it answers, so the queue is complete once `forward`
        // returned; poll briefly anyway in case of scheduling.
        for _ in 0..100 {
            let v = f();
            if !v.is_empty() {
                return v;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Vec::new()
    }

    #[test]
    fn socket_path_is_per_settings_directory() {
        let run = Path::new("/run/user/1000");
        let a = socket_path(Some(run), Some(Path::new("/home/u/.config/photocraft"))).unwrap();
        let b = socket_path(Some(run), Some(Path::new("/tmp/test-config"))).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(run));
        assert_eq!(a, socket_path(Some(run), Some(Path::new("/home/u/.config/photocraft"))).unwrap(), "stable");
        // No runtime directory (or a relative one): beside the settings.
        let c = socket_path(None, Some(Path::new("/tmp/test-config"))).unwrap();
        assert_eq!(c.parent(), Some(Path::new("/tmp/test-config")));
        assert_eq!(socket_path(Some(Path::new("rel")), Some(Path::new("/c"))).unwrap().parent(), Some(Path::new("/c")));
        assert_eq!(socket_path(Some(run), None), None);
    }

    #[test]
    fn opt_outs_start_a_new_window() {
        assert!(should_forward(false, None, false));
        assert!(should_forward(false, Some("0".into()), false));
        assert!(should_forward(false, Some("".into()), false));
        assert!(!should_forward(true, None, false));
        assert!(!should_forward(false, Some("1".into()), false));
        assert!(!should_forward(false, None, true), "--control drives its own instance");
    }

    #[test]
    fn requests_round_trip_any_path_bytes() {
        let paths = vec![PathBuf::from("/a/b.psd"), PathBuf::from("/c d/é.png"), PathBuf::from(OsString::from_vec(b"/latin1-\xe9.jpg".to_vec()))];
        assert_eq!(decode(&encode(&paths)), Some(paths));
        assert_eq!(decode(&encode(&[])), Some(vec![]));
        assert_eq!(decode(&[MAGIC, b"\0\0/x\0\0"].concat()), Some(vec![PathBuf::from("/x")]));
        let many = [MAGIC, &[b'a', 0].repeat(MAX_PATHS + 10)].concat();
        assert_eq!(decode(&many).map(|v| v.len()), Some(MAX_PATHS));
        // No header: a probe or a stranger, not a request.
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"/etc/passwd\0"), None);
    }

    #[test]
    fn a_second_launch_hands_its_files_to_the_first() {
        let dir = temp_dir("handoff");
        let sock = dir.join("s.sock");
        assert_eq!(forward(&sock, &[PathBuf::from("/x.png")]), Forward::NoInstance, "nobody listening yet");
        let server = listen(&sock).expect("first instance listens");
        assert!(listen(&sock).is_none(), "a second listener defers to the live one");
        let paths = vec![PathBuf::from("/photos/a.psd"), PathBuf::from("/photos/b c.png")];
        assert_eq!(forward(&sock, &paths), Forward::Delivered);
        let got = drain(&server.inbox);
        assert!(matches!(&got[..], [OsEvent::Open(p)] if p == &["/photos/a.psd".to_string(), "/photos/b c.png".to_string()]), "{got:?}");
        // No files: still answered (the window comes forward), with nothing to open.
        assert_eq!(forward(&sock, &[]), Forward::Delivered);
        assert!(matches!(&drain(&server.inbox)[..], [OsEvent::Open(p)] if p.is_empty()));
        drop(server);
        assert!(!sock.exists(), "the socket goes away with the instance");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_paths_from_a_client_are_dropped() {
        let dir = temp_dir("relative");
        let sock = dir.join("s.sock");
        let server = listen(&sock).unwrap();
        assert_eq!(forward(&sock, &[PathBuf::from("rel.png"), PathBuf::from("/abs.png")]), Forward::Delivered);
        assert!(matches!(&drain(&server.inbox)[..], [OsEvent::Open(p)] if p == &["/abs.png".to_string()]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let dir = temp_dir("stale");
        let sock = dir.join("s.sock");
        // A crashed instance: the file exists, nobody accepts.
        drop(UnixListener::bind(&sock).unwrap());
        assert!(sock.exists());
        assert_eq!(forward(&sock, &[PathBuf::from("/x.png")]), Forward::NoInstance);
        let server = listen(&sock).expect("takes over the stale socket");
        assert_eq!(forward(&sock, &[PathBuf::from("/x.png")]), Forward::Delivered);
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_instance_that_never_answers_lets_the_launch_start() {
        let dir = temp_dir("hung");
        let sock = dir.join("s.sock");
        // Accepts connections (the kernel backlog) but never reads or answers.
        let _hung = UnixListener::bind(&sock).unwrap();
        let t = std::time::Instant::now();
        assert_eq!(forward(&sock, &[PathBuf::from("/x.png")]), Forward::NoInstance);
        assert!(t.elapsed() < ANSWER_TIMEOUT + Duration::from_secs(2), "{:?}", t.elapsed());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_socket_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let sock = dir.join("s.sock");
        let _server = listen(&sock).unwrap();
        assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
