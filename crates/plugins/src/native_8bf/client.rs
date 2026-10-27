//! Process transport: no native library is ever loaded into PhotoCraft.
use super::wire::{Request, Response};
use std::{collections::BTreeMap, ffi::OsString, io, path::PathBuf, time::Duration};
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub environment: BTreeMap<OsString, OsString>,
    pub timeout: Duration,
}
impl Launch {
    pub fn from_environment(architecture: super::pe::Architecture) -> io::Result<Self> {
        let worker = super::launch::worker_path(architecture, &std::env::current_exe()?, cfg!(target_os = "macos"), |key| std::env::var_os(key))?;
        #[cfg(target_os = "windows")]
        let (program, args, environment) = (worker, Vec::new(), BTreeMap::new());
        #[cfg(not(target_os = "windows"))]
        let (program, args, environment) = {
            let mut environment = BTreeMap::new();
            let prefix = super::launch::wine_prefix(cfg!(target_os = "macos"), |key| std::env::var_os(key))?;
            environment.insert("WINEPREFIX".into(), prefix.into_os_string());
            environment.insert("WINEDEBUG".into(), "-all".into());
            let wine = super::launch::wine_program(cfg!(target_os = "macos"), |key| std::env::var_os(key), super::launch::is_executable);
            (wine, vec![worker.into_os_string()], environment)
        };
        Ok(Self { program, args, environment, timeout: Duration::from_secs(600) })
    }
}
/// Worker runs on Windows; Wine's Z: drive maps the Unix absolute filesystem.
pub fn windows_path(path: &std::path::Path) -> io::Result<String> {
    let path = path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "8BF path must be valid UTF-8"))?;
    #[cfg(target_os = "windows")]
    {
        Ok(path.into())
    }
    #[cfg(not(target_os = "windows"))]
    {
        if !path.starts_with('/') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "8BF path must be absolute"));
        }
        Ok(format!("Z:{}", path.replace('/', "\\")))
    }
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
struct Diagnostics {
    retained: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    finished: std::sync::mpsc::Receiver<()>,
}
struct Process {
    child: std::process::Child,
    terminated: bool,
}
impl Process {
    fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        // Wine can spawn descendants inheriting the pipes: kill the private process group,
        // then reap the direct child. This never touches a user's other Wine sessions.
        #[cfg(unix)]
        {
            let _ = std::process::Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn failure(&mut self, error: io::Error, diagnostics: &Diagnostics) -> io::Error {
        // Pipe EOF can arrive just before the OS publishes the exit status. Only report
        // a status observed before our cleanup kill, and never wait indefinitely for it.
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Err(_) => break None,
                Ok(None) if std::time::Instant::now() >= deadline => break None,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        self.terminate();
        // An escaped Wine descendant may retain stderr after group cleanup. The reader
        // publishes its bounded prefix as it goes, so EOF is not required to retain it.
        let _ = diagnostics.finished.recv_timeout(Duration::from_millis(100));
        let retained = diagnostics.retained.lock().unwrap_or_else(|e| e.into_inner());
        let detail = String::from_utf8_lossy(&retained);
        let mut message = match status {
            Some(status) => format!("8BF worker exited {status}; communication failed: {error}"),
            None => format!("8BF worker communication failed: {error}"),
        };
        if !detail.trim().is_empty() {
            message.push_str("\n8BF worker output: ");
            message.push_str(detail.trim());
        }
        io::Error::new(error.kind(), message)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.terminate();
    }
}
pub fn exchange(launch: &Launch, request: Request, pixels: Vec<u8>, cancelled: impl Fn() -> bool) -> io::Result<Vec<u8>> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::mpsc,
        time::Instant,
    };
    let expected = request.pixel_len()?;
    if expected != pixels.len() {
        return Err(invalid("8BF request pixel length does not match its format"));
    }
    if cancelled() {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "8BF filter cancelled"));
    }
    let mut command = Command::new(&launch.program);
    command.env_remove("WINEARCH").args(&launch.args).envs(&launch.environment).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Hide the helper's console; the filter's own native GUI remains available.
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let child = command.spawn().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("Could not start 8BF worker {}: {e}. Check the installed 8BF workers or PHOTOCRAFT_8BF_WORKER32/64; Linux and macOS also require Wine (PHOTOCRAFT_8BF_WINE).", launch.program.display()),
        )
    })?;
    let mut process = Process { child, terminated: false };
    let input = process.child.stdin.take().ok_or_else(|| io::Error::other("8BF worker stdin unavailable"))?;
    let output = process.child.stdout.take().ok_or_else(|| io::Error::other("8BF worker stdout unavailable"))?;
    let mut error = process.child.stderr.take().ok_or_else(|| io::Error::other("8BF worker stderr unavailable"))?;
    let (write_tx, write_rx) = mpsc::sync_channel(1);
    let outgoing = request.clone();
    std::thread::Builder::new().name("8bf-send".into()).spawn(move || {
        let _ = write_tx.send(super::wire::write_frame(input, &outgoing, &pixels));
    })?;
    let (read_tx, read_rx) = mpsc::sync_channel(1);
    std::thread::Builder::new().name("8bf-receive".into()).spawn(move || {
        let _ = read_tx.send(super::wire::read_frame::<Response>(output));
    })?;
    let (error_tx, error_rx) = mpsc::sync_channel(1);
    let retained = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let diagnostics = Diagnostics { retained: retained.clone(), finished: error_rx };
    std::thread::Builder::new().name("8bf-stderr".into()).spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match error.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut retained = retained.lock().unwrap_or_else(|e| e.into_inner());
                    let room = 4096usize.saturating_sub(retained.len());
                    retained.extend_from_slice(buffer.get(..n.min(room)).unwrap_or_default());
                }
            }
        }
        let _ = error_tx.send(());
    })?;
    let start = Instant::now();
    let mut sent = false;
    let mut received = None;
    loop {
        if cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "8BF filter cancelled"));
        }
        if start.elapsed() > launch.timeout {
            return Err(process.failure(io::Error::new(io::ErrorKind::TimedOut, "8BF worker timed out; no pixels were applied"), &diagnostics));
        }
        if !sent {
            match write_rx.try_recv() {
                Ok(r) => {
                    if let Err(error) = r {
                        return Err(process.failure(error, &diagnostics));
                    }
                    sent = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => return Err(process.failure(io::Error::other("8BF input writer disconnected"), &diagnostics)),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if received.is_none() {
            match read_rx.try_recv() {
                Ok(Ok(response)) => received = Some(response),
                Ok(Err(error)) => return Err(process.failure(error, &diagnostics)),
                Err(mpsc::TryRecvError::Disconnected) => return Err(process.failure(io::Error::other("8BF response reader disconnected"), &diagnostics)),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(status) = process.child.try_wait()? {
            if !status.success() {
                return Err(process.failure(io::Error::other(format!("8BF worker exited {status}")), &diagnostics));
            }
            if sent && let Some((response, bytes)) = received.take() {
                match response.status.as_str() {
                    "cancelled" => return Err(io::Error::new(io::ErrorKind::Interrupted, "8BF filter cancelled in plug-in dialog")),
                    "error" => return Err(io::Error::other(format!("8BF filter: {}", response.message))),
                    "ok" => {}
                    _ => return Err(invalid("unknown 8BF response status")),
                }
                if response.width != request.width
                    || response.height != request.height
                    || response.channels != request.channels
                    || response.depth != request.depth
                    || bytes.len() != expected
                {
                    return Err(invalid("8BF worker returned inconsistent pixel dimensions or format"));
                }
                return Ok(bytes);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn request() -> Request {
        Request { path: "ignored.8bf".into(), entry: "PluginMain".into(), width: 2, height: 1, channels: 3, depth: 8, show_ui: false }
    }
    fn launch(script: &str) -> Launch {
        Launch { program: "python3".into(), args: vec!["-c".into(), script.into()], environment: BTreeMap::new(), timeout: Duration::from_secs(3) }
    }
    #[test]
    fn child_transport_roundtrips_and_rejects_malformed_or_crashed_worker() {
        let script = r#"import sys,struct,json
r=sys.stdin.buffer; w=sys.stdout.buffer
assert r.read(8)==b'PC8BF001'
n,p=struct.unpack('<IQ',r.read(12)); m=json.loads(r.read(n)); pixels=r.read(p)
m={'status':'ok','message':'','width':m['width'],'height':m['height'],'channels':3,'depth':8}
h=json.dumps(m).encode(); w.write(b'PC8BF001'+struct.pack('<IQ',len(h),len(pixels))+h+bytes(255-v for v in pixels)); w.flush()
"#;
        assert_eq!(exchange(&launch(script), request(), vec![1, 2, 3, 4, 5, 6], || false).unwrap(), [254, 253, 252, 251, 250, 249]);
        for script in ["import sys; sys.exit(17)", "import sys; sys.stdout.buffer.write(b'bad reply')"] {
            assert!(exchange(&launch(script), request(), vec![0; 6], || false).is_err());
        }
    }
    #[test]
    fn blocked_worker_cancels_and_times_out_without_hanging() {
        let mut l = launch("import time; time.sleep(60)");
        l.timeout = Duration::from_millis(60);
        let start = std::time::Instant::now();
        assert_eq!(exchange(&l, request(), vec![0; 6], || false).unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        let start = std::time::Instant::now();
        assert_eq!(exchange(&l, request(), vec![0; 6], || start.elapsed() > Duration::from_millis(20)).unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn failed_transport_retains_bounded_worker_diagnostics_without_hanging() {
        let script = r#"import os,sys,time
sys.stderr.write('Missing required plug-in runtime: example.dll\n' + 'x' * 8192)
sys.stderr.flush()
os.close(1)
time.sleep(60)
"#;
        let start = std::time::Instant::now();
        let error = exchange(&launch(script), request(), vec![0; 6], || false).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert!(error.to_string().contains("example.dll"), "worker diagnosis was lost: {error}");
        assert!(!error.to_string().contains("8BF worker exited"), "cleanup kill must not be reported as the original exit: {error}");
        assert!(error.to_string().len() < 4600, "diagnostics must stay bounded");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn crashing_worker_retains_diagnostics_when_descendants_hold_stderr_open() {
        let script = r#"import os,sys,time
sys.stderr.write('Native runtime initialization failed\n')
sys.stderr.flush()
if os.fork() == 0:
    time.sleep(60)
    os._exit(0)
os._exit(17)
"#;
        let start = std::time::Instant::now();
        let error = exchange(&launch(script), request(), vec![0; 6], || false).unwrap_err();
        assert!(error.to_string().contains("Native runtime initialization failed"), "worker diagnosis was lost: {error}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn silent_worker_exit_retains_original_status() {
        let error = exchange(&launch("import os,sys; sys.stdin.buffer.read(); os._exit(17)"), request(), vec![0; 6], || false).unwrap_err();
        assert!(error.to_string().contains("8BF worker exited exit status: 17"), "original status was lost: {error}");
    }

    #[test]
    fn escaped_descendant_cannot_hide_already_written_diagnostics() {
        let script = r#"import os,sys,time
sys.stdin.buffer.read()
ready, signal = os.pipe()
if os.fork() == 0:
    os.close(ready)
    os.setsid()
    os.close(0)
    os.close(1)
    os.write(signal, b'1')
    os.close(signal)
    time.sleep(0.6)
    os._exit(0)
os.close(signal)
os.read(ready, 1)
os.close(ready)
sys.stderr.write('Escaped runtime diagnosis: example.dll\n' + 'x' * 8192)
sys.stderr.flush()
os._exit(17)
"#;
        let start = std::time::Instant::now();
        let error = exchange(&launch(script), request(), vec![0; 6], || false).unwrap_err();
        assert!(error.to_string().contains("Escaped runtime diagnosis: example.dll"), "already read diagnostics were lost: {error}");
        assert!(error.to_string().len() < 4600, "diagnostics must stay bounded");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn truncated_worker_reply_identifies_the_frame_stage() {
        for (reply, stage) in [(b"PC8".as_slice(), "frame magic"), (b"PC8BF001\x02".as_slice(), "metadata length")] {
            let error = super::super::wire::read_frame::<Response>(reply).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
            assert!(error.to_string().contains(stage), "frame stage was lost: {error}");
        }
    }

    #[test]
    fn all_supported_wire_formats_preserve_full_range_samples() {
        let script = r#"import sys,struct,json
r=sys.stdin.buffer;w=sys.stdout.buffer
assert r.read(8)==b'PC8BF001'
n,p=struct.unpack('<IQ',r.read(12));m=json.loads(r.read(n));d=r.read(p)
m.update(status='ok',message='')
h=json.dumps(m).encode();w.write(b'PC8BF001'+struct.pack('<IQ',len(h),len(d))+h+d)
"#;
        for (channels, depth) in [(1, 8), (1, 16), (3, 8), (3, 16)] {
            let mut request = request();
            request.channels = channels;
            request.depth = depth;
            let input: Vec<u8> = if depth == 16 {
                [1u16, 255, 256, 32769, 65534, 65535].into_iter().cycle().take(2 * usize::from(channels)).flat_map(u16::to_le_bytes).collect()
            } else {
                vec![7; 2 * usize::from(channels)]
            };
            assert_eq!(exchange(&launch(script), request, input.clone(), || false).unwrap(), input);
        }
    }
    #[test]
    fn inconsistent_response_format_and_invalid_requests_are_rejected() {
        let script = r#"import sys,struct,json
r=sys.stdin.buffer;w=sys.stdout.buffer
r.read(8);n,p=struct.unpack('<IQ',r.read(12));m=json.loads(r.read(n));d=r.read(p)
m.update(status='ok',message='',depth=8)
h=json.dumps(m).encode();w.write(b'PC8BF001'+struct.pack('<IQ',len(h),len(d))+h+d)
"#;
        let mut sixteen = request();
        sixteen.depth = 16;
        assert!(exchange(&launch(script), sixteen, vec![0; 12], || false).is_err());
        let mut absent = launch("");
        absent.program = "/nonexistent/photocraft-worker-must-not-run".into();
        for (channels, depth, width, height, bytes) in [(2, 8, 2, 1, 4), (1, 32, 2, 1, 8), (1, 16, 0, 1, 0), (1, 16, 32768, 1, 65536), (3, 16, 2, 1, 11)] {
            let mut request = request();
            request.channels = channels;
            request.depth = depth;
            request.width = width;
            request.height = height;
            assert_eq!(exchange(&absent, request, vec![0; bytes], || false).unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
    }
}
