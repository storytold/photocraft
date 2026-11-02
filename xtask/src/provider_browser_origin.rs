//! Explicit, bounded test-only artifact relay. Never started by the application or ordinary CI.
#[cfg(unix)]
pub const MARKER: &str = "file_provider_browser_tests::page_local_provider_boundary_checks_default_off_inputs_limits_and_opaque_frames";

#[cfg(not(unix))]
pub fn run(_args: &[&str]) -> Result<(), String> {
    Err("browser-test-provider-origin requires a Unix managed validation unit".into())
}

#[cfg(unix)]
pub fn run(args: &[&str]) -> Result<(), String> {
    support::run(args)
}

#[cfg(unix)]
mod support {
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    use std::os::unix::process::CommandExt;
    use std::path::{Component, Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread;
    use std::time::{Duration, Instant};

    use super::MARKER;
    use serde_json::{Value, json};

    const MAX_FILE: u64 = 512 * 1024 * 1024;
    const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
    const MAX_CONNECTIONS: usize = 96;
    const MAX_HEADER: usize = 16 * 1024;
    const MAX_INDEX: usize = 128 * 1024;
    const MAX_LOG: u64 = 32 * 1024 * 1024;

    #[derive(Default)]
    struct State {
        upstream: Option<SocketAddr>,
        generated: Option<PathBuf>,
        artifacts: PathBuf,
        receipts: Vec<Value>,
        bytes: u64,
        error: Option<String>,
        proof: Option<Value>,
        passed: usize,
        summary: bool,
        summaries: usize,
        log: Option<Value>,
    }

    fn locked(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
        state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    struct Config {
        evidence: PathBuf,
        temporary: PathBuf,
        timeout: Duration,
    }

    fn config(args: &[&str]) -> Result<Config, String> {
        let mut evidence = None;
        let mut temporary = None;
        let mut seconds = 1800_u64;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            let value = iter.next().ok_or_else(|| format!("missing value for {arg}"))?;
            match *arg {
                "--evidence" => evidence = Some(PathBuf::from(*value)),
                "--tmp-root" => temporary = Some(PathBuf::from(*value)),
                "--timeout-seconds" => seconds = value.parse().map_err(|_| "invalid timeout")?,
                _ => return Err(format!("unknown browser-test-provider-origin option {arg}")),
            }
        }
        if !(600..=3600).contains(&seconds) {
            return Err("timeout must be between 600 and 3600 seconds".into());
        }
        let temporary = temporary.ok_or("--tmp-root is required")?.canonicalize().map_err(|e| e.to_string())?;
        if !temporary.is_dir() || temporary.parent().is_none() || temporary == Path::new("/tmp") {
            return Err("--tmp-root must be an existing task-owned private directory, not / or /tmp".into());
        }
        let evidence = evidence.ok_or("--evidence is required")?;
        let parent = evidence.parent().ok_or("evidence needs an existing parent")?.canonicalize().map_err(|e| e.to_string())?;
        let filename = evidence.file_name().ok_or("evidence needs a filename")?;
        let evidence = parent.join(filename);
        if evidence.exists() {
            return Err("refusing to overwrite existing browser-origin evidence".into());
        }
        Ok(Config { evidence, temporary, timeout: Duration::from_secs(seconds) })
    }

    pub fn run(args: &[&str]) -> Result<(), String> {
        let cfg = config(args)?;
        // Caller supplies the existing D-backed TMP/target/cache and the sole managed heavy lane.
        let mut evidence = OpenOptions::new().write(true).create_new(true).open(&cfg.evidence).map_err(|e| e.to_string())?;
        evidence.write_all(b"{\"status\":\"starting\",\"success\":false}\n").map_err(|e| e.to_string())?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let mut random = [0_u8; 32];
        File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut random)).map_err(|e| format!("test nonce: {e}"))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let endpoint = format!("http://{address}/{token}/");
        let artifacts = cfg.evidence.with_extension("artifacts");
        std::fs::create_dir(&artifacts).map_err(|e| format!("fresh artifact snapshot directory: {e}"))?;
        let state = Arc::new(Mutex::new(State { artifacts, ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + cfg.timeout;
        let server_state = state.clone();
        let server_stop = stop.clone();
        let server = thread::Builder::new()
            .name("photocraft-origin-relay".into())
            .spawn(move || {
                let result = serve(listener, &token, &server_state, &server_stop, deadline);
                if let Err(error) = &result {
                    locked(&server_state).error.get_or_insert(error.clone());
                }
                result
            })
            .map_err(|e| format!("artifact relay thread: {e}"))?;
        let outcome = execute(&cfg, &endpoint, &state, deadline);
        stop.store(true, Ordering::SeqCst);
        let server_result = server.join().map_err(|_| "artifact relay thread panicked".to_string()).and_then(|r| r);
        let s = locked(&state);
        let checks = s.error.clone().map_or(Ok(()), Err).and_then(|()| {
            if s.passed != 1
                || s.summaries != 1
                || !s.summary
                || !valid_proof(&s, address)
                || !s.receipts.iter().any(|r| r["path"] == "/wasm-bindgen-test_bg.wasm")
                || !s.receipts.iter().any(|r| r["path"] == "/")
            {
                Err("missing exact parent test result, real-child proof, generated index or WASM receipt".into())
            } else {
                Ok(())
            }
        });
        let success = outcome.is_ok() && server_result.is_ok() && checks.is_ok();
        let result = json!({
            "schema":1, "success":success, "test":MARKER,
            "cargo": outcome.as_ref().err(), "relay":server_result.as_ref().err(), "proofError":checks.as_ref().err(),
            "upstream":s.upstream.map(|a| format!("http://{a}")), "generatedArtifacts":s.generated,
            "relayOrigin":format!("http://{address}"), "artifactSnapshots":s.artifacts, "cors":"null", "sandbox":"allow-scripts",
            "isolation":{"coop":"same-origin","coep":"require-corp"}, "artifactReceipts":s.receipts,
            "childProof":s.proof, "cargoLog":s.log, "parentPositiveCount":s.passed, "exactOnePassSummary":s.summary,
            "totalBytes":s.bytes, "limits":{"maxArtifactBytes":MAX_FILE,"maxTotalBytes":MAX_TOTAL,"maxConnections":MAX_CONNECTIONS,"timeoutSeconds":cfg.timeout.as_secs()},
            "scope":"test-only artifact relay, not backend durability or editor rendering"
        });
        let primary = outcome.and(server_result).and(checks);
        let saved = serde_json::to_vec_pretty(&result).map_err(|e| format!("origin receipt JSON: {e}")).and_then(|encoded| {
            use std::io::{Seek, SeekFrom};
            evidence
                .set_len(0)
                .and_then(|()| {
                    evidence.seek(SeekFrom::Start(0))?;
                    evidence.write_all(&encoded)?;
                    evidence.write_all(b"\n")?;
                    evidence.sync_all()
                })
                .map_err(|e| format!("origin evidence: {e}"))
        });
        match (primary, saved) {
            (Err(primary), Err(secondary)) => Err(format!("{primary}; secondary evidence error: {secondary}")),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(()), result) => result,
        }
    }

    fn valid_proof(state: &State, relay: SocketAddr) -> bool {
        let Some(proof) = &state.proof else {
            return false;
        };
        let Some(upstream) = state.upstream else {
            return false;
        };
        proof["parentOrigin"] == format!("http://{upstream}")
            && proof["relayOrigin"] == format!("http://{relay}")
            && proof["marker"] == "photocraft-provider-opaque-probe"
            && proof["sandbox"] == "allow-scripts"
            && proof["nonce"].as_str().is_some_and(|nonce| nonce.len() == 32 && nonce.bytes().all(|b| b.is_ascii_hexdigit()))
    }

    struct Output {
        file: File,
        path: PathBuf,
        total: u64,
        line: Vec<u8>,
        overlong: bool,
        hash: Digest,
    }

    impl Output {
        fn drain(&mut self, temporary: &Path, state: &Mutex<State>) -> Result<bool, String> {
            let mut buffer = [0_u8; 65536];
            for _ in 0..16 {
                let n = self.file.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Ok(true);
                }
                self.total = self.total.checked_add(n as u64).ok_or("log length overflow")?;
                if self.total > MAX_LOG {
                    return Err("browser stdout exceeded 32 MiB".into());
                }
                self.hash.update(&buffer[..n])?;
                std::io::stdout().write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                for &byte in &buffer[..n] {
                    if byte == b'\n' {
                        if !self.overlong {
                            observe_line(String::from_utf8_lossy(&self.line).trim(), temporary, state);
                        }
                        self.line.clear();
                        self.overlong = false;
                    } else if self.line.len() < 8192 {
                        self.line.push(byte);
                    } else {
                        self.overlong = true;
                    }
                }
            }
            Ok(false)
        }
        fn finish(mut self, temporary: &Path, state: &Mutex<State>) -> Result<(), String> {
            while !self.drain(temporary, state)? {}
            if !self.line.is_empty() && !self.overlong {
                observe_line(String::from_utf8_lossy(&self.line).trim(), temporary, state);
            }
            let mut s = locked(state);
            s.log = Some(json!({"path":self.path,"bytes":self.total,"sha256":self.hash.finish()?}));
            Ok(())
        }
    }

    fn execute(cfg: &Config, endpoint: &str, state: &Arc<Mutex<State>>, deadline: Instant) -> Result<(), String> {
        let runner = std::env::var_os("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER").ok_or("bind the exact pinned WASM runner path first")?;
        if !Path::new(&runner).is_absolute() || !Path::new(&runner).is_file() {
            return Err("the WASM runner must be an existing absolute bound executable path".into());
        }
        let path = cfg.evidence.with_extension("cargo.log");
        let writer = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        let file = File::open(&path).map_err(|e| e.to_string())?;
        let mut output = Output { file, path, total: 0, line: Vec::new(), overlong: false, hash: Digest::new() };
        let mut command = crate::cargo();
        command.args([
            "test",
            "--locked",
            "--offline",
            "-p",
            "photocraft-web",
            "--bin",
            "photocraft-web",
            "--target",
            "wasm32-unknown-unknown",
            "-j1",
            MARKER,
            "--",
            "--nocapture",
        ]);
        command
            .env("PHOTOCRAFT_TEST_ORIGIN_RELAY", endpoint)
            .env("WASM_BINDGEN_KEEP_TEST_BUILD", "1")
            .env("WASM_BINDGEN_TEST_ADDRESS", "127.0.0.1:0")
            .env("WASM_BINDGEN_TEST_TIMEOUT", "900")
            .env("TMPDIR", &cfg.temporary)
            .env("CARGO_BUILD_JOBS", "1")
            .env("RUST_TEST_THREADS", "1")
            .env("RAYON_NUM_THREADS", "1")
            .env_remove("NO_HEADLESS")
            .env_remove("WASM_BINDGEN_USE_NO_MODULE")
            .env_remove("WASM_BINDGEN_TEST_NO_ORIGIN_ISOLATION")
            .env_remove("WASM_BINDGEN_TEST_ONLY_NODE")
            .stdin(Stdio::null())
            .stdout(Stdio::from(writer))
            .stderr(Stdio::inherit())
            .process_group(0);
        let mut child = OwnedCargo(command.spawn().map_err(|e| format!("cargo browser test: {e}"))?);
        let outcome = loop {
            if let Err(error) = output.drain(&cfg.temporary, state) {
                let stopped = terminate(&mut child.0);
                break Err(format!("{error}; cargo termination: {stopped:?}"));
            }
            if let Some(status) = child.0.try_wait().map_err(|e| e.to_string())? {
                break if status.success() { Ok(()) } else { Err(format!("cargo browser test exited {status}")) };
            }
            if Instant::now() >= deadline || locked(state).error.is_some() {
                let stopped = terminate(&mut child.0);
                break Err(format!("browser helper deadline/relay failure; cargo termination: {stopped:?}"));
            }
            thread::sleep(Duration::from_millis(100));
        };
        // A regular owned log file avoids an unbounded pipe-reader join when descendants inherit
        // stdout. The outer managed unit must still prove all compiler/driver/Chrome scope absent.
        let recorded = output.finish(&cfg.temporary, state);
        match (outcome, recorded) {
            (Err(primary), Err(secondary)) => Err(format!("{primary}; secondary log evidence error: {secondary}")),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(()), recorded) => recorded,
        }
    }

    struct OwnedCargo(Child);
    impl Drop for OwnedCargo {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = terminate(&mut self.0);
            }
        }
    }

    fn terminate(child: &mut Child) -> Result<(), String> {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Ok(());
        }
        let group = format!("-{}", child.id());
        let signal = Command::new("/bin/kill").args(["-TERM", "--", &group]).status();
        if !signal.is_ok_and(|status| status.success()) {
            child.kill().map_err(|e| e.to_string())?;
        }
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Ok(());
            }
            if Instant::now() >= until {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let signal = Command::new("/bin/kill").args(["-KILL", "--", &group]).status();
        if !signal.is_ok_and(|status| status.success()) {
            child.kill().map_err(|e| e.to_string())?;
        }
        child.wait().map(|_| ()).map_err(|e| e.to_string())
    }

    fn observe_line(line: &str, temporary: &Path, state: &Mutex<State>) {
        let result = observe(line, temporary, &mut locked(state));
        if let Err(error) = result {
            locked(state).error.get_or_insert(error);
        }
    }

    fn observe(line: &str, temporary: &Path, state: &mut State) -> Result<(), String> {
        if let Some(path) = line.strip_prefix("Retaining temporary build output folder: ") {
            let path = Path::new(path).canonicalize().map_err(|e| e.to_string())?;
            if !path.starts_with(temporary) || path == temporary || state.generated.is_some() {
                return Err("unexpected or repeated generated artifact directory".into());
            }
            state.generated = Some(path);
        } else if let Some(origin) = line.strip_prefix("Tests are now available at http://") {
            let address: SocketAddr = origin.parse().map_err(|_| "invalid owned runner origin")?;
            if address.ip() != Ipv4Addr::LOCALHOST || address.port() == 0 || state.upstream.is_some() {
                return Err("runner must announce one exact IPv4 loopback origin".into());
            }
            state.upstream = Some(address);
        } else if let Some(proof) = line.strip_prefix("PHOTOCRAFT_OPAQUE_CHILD_PROOF ") {
            let proof: Value = serde_json::from_str(proof).map_err(|e| e.to_string())?;
            if state.proof.is_some()
                || proof["denied"] != true
                || proof["errorName"] != "SecurityError"
                || proof["senderOrigin"] != "null"
                || proof["forgedMessages"] != 1
                || proof["realSourceMatched"] != true
                || proof["trustedMessage"] != true
            {
                return Err("invalid or repeated opaque-child proof".into());
            }
            state.proof = Some(proof);
        } else if line == format!("test {MARKER} ... ok") {
            state.passed = state.passed.saturating_add(1);
        } else if line.starts_with("test result:") {
            state.summaries = state.summaries.saturating_add(1);
            state.summary = line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;");
            if state.summaries != 1 || !state.summary {
                return Err("zero, failed, ignored or repeated browser test summary".into());
            }
        }
        Ok(())
    }

    fn artifact_path<'a>(target: &'a str, token: &str) -> Result<&'a str, String> {
        let prefix = format!("/{token}/");
        let relative = target.strip_prefix(&prefix).ok_or("unknown relay capability path")?;
        if relative.len() > 1024 || relative.contains(['%', '?', '#', '\\', ':']) {
            return Err("encoded, queried or non-relative artifact path rejected".into());
        }
        if relative.is_empty() {
            return Ok("/");
        }
        if relative == "run.js" || relative == "wasm-bindgen-test.js" || relative == "wasm-bindgen-test" || relative == "wasm-bindgen-test_bg.wasm" {
            return Ok(relative);
        }
        if !relative.starts_with("snippets/")
            || !relative.ends_with(".js")
            || relative.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || !part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
        {
            return Err("artifact is outside the generated module/snippets allowlist".into());
        }
        Ok(relative)
    }

    struct Request {
        target: String,
        origin: Option<String>,
    }

    fn request(socket: &mut TcpStream, stop: &AtomicBool, deadline: Instant) -> Result<Request, String> {
        let mut header = Vec::new();
        loop {
            if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return Err("request stopped".into());
            }
            let mut byte = [0_u8; 1];
            socket.read_exact(&mut byte).map_err(|e| e.to_string())?;
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
            if header.len() >= MAX_HEADER {
                return Err("request header too large".into());
            }
        }
        let text = std::str::from_utf8(&header).map_err(|_| "non-UTF8 request header")?;
        let mut lines = text.split("\r\n");
        let mut first = lines.next().ok_or("missing request line")?.split(' ');
        if first.next() != Some("GET") {
            return Err("only GET artifact requests are supported".into());
        }
        let target = first.next().ok_or("missing request target")?.to_string();
        if first.next() != Some("HTTP/1.1") || first.next().is_some() {
            return Err("invalid HTTP request line".into());
        }
        let mut origin = None;
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').ok_or("invalid request header")?;
            if name.eq_ignore_ascii_case("origin") {
                if origin.replace(value.trim().to_string()).is_some() {
                    return Err("duplicate Origin".into());
                }
            }
            if name.eq_ignore_ascii_case("transfer-encoding") || (name.eq_ignore_ascii_case("content-length") && value.trim() != "0") {
                return Err("request body is not permitted".into());
            }
        }
        Ok(Request { target, origin })
    }

    fn serve(listener: TcpListener, token: &str, state: &Mutex<State>, stop: &AtomicBool, deadline: Instant) -> Result<(), String> {
        let mut count = 0;
        while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut socket, peer)) => {
                    count += 1;
                    if !peer.ip().is_loopback() || count > MAX_CONNECTIONS {
                        return Err("relay connection bound exceeded".into());
                    }
                    socket.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
                    socket.set_write_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
                    if let Err(error) = handle(&mut socket, token, state, stop, deadline) {
                        let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
                        // Unauthorised browser favicon requests do not invalidate a genuine run.
                        if !error.starts_with("unknown relay capability path") {
                            locked(state).error.get_or_insert(error);
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(20)),
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    fn handle(socket: &mut TcpStream, token: &str, state: &Mutex<State>, stop: &AtomicBool, deadline: Instant) -> Result<(), String> {
        let req = request(socket, stop, deadline)?;
        let path = artifact_path(&req.target, token)?;
        if req.origin.as_deref() != Some("null") && !(path == "/" && req.origin.is_none()) {
            return Err("only opaque-origin assets and the initial iframe navigation are permitted".into());
        }
        let (upstream, generated) = {
            let s = locked(state);
            (s.upstream.ok_or("runner origin not announced")?, s.generated.clone().ok_or("generated directory not announced")?)
        };
        if path == "/" {
            let bytes = index(upstream, stop, deadline)?;
            ensure_budget(state, bytes.len() as u64)?;
            let (mut snapshot, saved) = snapshot_file(state)?;
            snapshot.write_all(&bytes).and_then(|()| snapshot.sync_all()).map_err(|e| e.to_string())?;
            response_headers(socket, "text/html; charset=utf-8", bytes.len() as u64)?;
            socket.write_all(&bytes).map_err(|e| e.to_string())?;
            receipt(state, "/", bytes.len() as u64, &crate::sha256::hex(&bytes), &saved)?;
        } else {
            let name = if path == "wasm-bindgen-test" { "wasm-bindgen-test.js" } else { path };
            let source = generated_file(&generated, name)?;
            let mut file = File::open(&source).map_err(|e| e.to_string())?;
            let before = file.metadata().map_err(|e| e.to_string())?;
            if !before.is_file() || before.len() > MAX_FILE {
                return Err("generated artifact exceeds file bound".into());
            }
            let mime = if path.ends_with(".wasm") { "application/wasm" } else { "text/javascript; charset=utf-8" };
            ensure_budget(state, before.len())?;
            let (mut snapshot, saved) = snapshot_file(state)?;
            response_headers(socket, mime, before.len())?;
            let digest = stream_file(socket, &mut file, &mut snapshot, before.len(), stop, deadline)?;
            snapshot.sync_all().map_err(|e| e.to_string())?;
            let after = file.metadata().map_err(|e| e.to_string())?;
            if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
                return Err("generated artifact changed while served".into());
            }
            receipt(state, &format!("/{name}"), before.len(), &digest, &saved)?;
        }
        Ok(())
    }

    fn generated_file(root: &Path, name: &str) -> Result<PathBuf, String> {
        if Path::new(name).components().any(|part| !matches!(part, Component::Normal(_))) {
            return Err("non-normal generated artifact path".into());
        }
        let path = root.join(name).canonicalize().map_err(|e| e.to_string())?;
        if !path.starts_with(root) || path == root {
            return Err("generated artifact escapes its retained directory".into());
        }
        Ok(path)
    }

    fn response_headers(socket: &mut TcpStream, mime: &str, length: u64) -> Result<(), String> {
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {length}\r\nConnection: close\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: null\r\nVary: Origin\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nX-Content-Type-Options: nosniff\r\n\r\n").map_err(|e| e.to_string())
    }

    fn stream_file(socket: &mut TcpStream, file: &mut File, snapshot: &mut File, length: u64, stop: &AtomicBool, deadline: Instant) -> Result<String, String> {
        let mut buffer = [0_u8; 65536];
        let mut left = length;
        let mut hash = Digest::new();
        while left != 0 {
            if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return Err("artifact transfer stopped".into());
            }
            let size = usize::try_from(left.min(buffer.len() as u64)).map_err(|e| e.to_string())?;
            file.read_exact(&mut buffer[..size]).map_err(|e| e.to_string())?;
            hash.update(&buffer[..size])?;
            snapshot.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
            socket.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
            left -= size as u64;
        }
        hash.finish()
    }

    fn ensure_budget(state: &Mutex<State>, bytes: u64) -> Result<(), String> {
        if bytes > MAX_TOTAL.saturating_sub(locked(state).bytes) { Err("relay total byte budget exhausted".into()) } else { Ok(()) }
    }

    fn snapshot_file(state: &Mutex<State>) -> Result<(File, PathBuf), String> {
        let s = locked(state);
        let path = s.artifacts.join(format!("artifact-{:03}.bin", s.receipts.len()));
        let file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        Ok((file, path))
    }

    fn receipt(state: &Mutex<State>, path: &str, bytes: u64, sha256: &str, snapshot: &Path) -> Result<(), String> {
        let mut s = locked(state);
        s.bytes = s.bytes.checked_add(bytes).ok_or("relay byte counter overflow")?;
        if s.bytes > MAX_TOTAL || s.receipts.len() >= MAX_CONNECTIONS {
            return Err("relay total bound exceeded".into());
        }
        if s.receipts.iter().any(|row| row["path"] == path && (row["sha256"] != sha256 || row["bytes"] != bytes)) {
            return Err("a repeated generated artifact changed identity".into());
        }
        s.receipts.push(json!({"path":path,"snapshot":snapshot,"bytes":bytes,"sha256":sha256,"cors":"null","coop":"same-origin","coep":"require-corp"}));
        Ok(())
    }

    fn index(upstream: SocketAddr, stop: &AtomicBool, deadline: Instant) -> Result<Vec<u8>, String> {
        let mut socket = TcpStream::connect_timeout(&upstream, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        socket.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
        socket.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
        write!(socket, "GET / HTTP/1.1\r\nHost: {upstream}\r\nConnection: close\r\nAccept-Encoding: identity\r\n\r\n").map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return Err("index transfer stopped".into());
            }
            let n = socket.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if n > (MAX_HEADER + MAX_INDEX).saturating_sub(bytes.len()) {
                return Err("generated index response too large".into());
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
        let split = bytes.windows(4).position(|part| part == b"\r\n\r\n").ok_or("index response has no header terminator")?;
        if split > MAX_HEADER {
            return Err("index response header too large".into());
        }
        let header = std::str::from_utf8(&bytes[..split]).map_err(|_| "invalid index HTTP header")?;
        let mut lines = header.split("\r\n");
        if !matches!(lines.next(), Some("HTTP/1.1 200 OK" | "HTTP/1.0 200 OK")) {
            return Err("generated index was not HTTP 200".into());
        }
        let mut fields = std::collections::BTreeMap::new();
        for line in lines {
            let (name, value) = line.split_once(':').ok_or("invalid index header")?;
            if fields.insert(name.to_ascii_lowercase(), value.trim()).is_some() {
                return Err("duplicate index header".into());
            }
        }
        if fields.get("cross-origin-opener-policy") != Some(&"same-origin")
            || fields.get("cross-origin-embedder-policy") != Some(&"require-corp")
            || !fields.get("content-type").is_some_and(|value| value.starts_with("text/html"))
            || fields.contains_key("content-encoding")
        {
            return Err("generated index isolation/type/encoding mismatch".into());
        }
        let body = bytes.get(split + 4..).ok_or("index body missing")?;
        let body = if fields.get("transfer-encoding") == Some(&"chunked") {
            if fields.contains_key("content-length") {
                return Err("ambiguous index transfer framing".into());
            }
            unchunk(body)?
        } else {
            if fields.contains_key("transfer-encoding") {
                return Err("unsupported index transfer encoding".into());
            }
            let length: usize = fields.get("content-length").ok_or("index length missing")?.parse().map_err(|_| "bad index length")?;
            if length != body.len() {
                return Err("generated index length mismatch".into());
            }
            body.to_vec()
        };
        if body.len() > MAX_INDEX {
            return Err("generated index body too large".into());
        }
        Ok(body)
    }

    fn unchunk(mut input: &[u8]) -> Result<Vec<u8>, String> {
        let mut result = Vec::new();
        loop {
            let line = input.windows(2).position(|p| p == b"\r\n").ok_or("missing chunk length")?;
            if line > 16 {
                return Err("chunk length too long".into());
            }
            let count = usize::from_str_radix(std::str::from_utf8(&input[..line]).map_err(|_| "bad chunk text")?, 16).map_err(|_| "bad chunk length")?;
            input = input.get(line + 2..).ok_or("bad chunk boundary")?;
            if count == 0 {
                if input != b"\r\n" {
                    return Err("unexpected chunk trailers".into());
                }
                return Ok(result);
            }
            if count > MAX_INDEX.saturating_sub(result.len()) {
                return Err("chunked index too large".into());
            }
            result.extend_from_slice(input.get(..count).ok_or("short chunk")?);
            if input.get(count..count + 2) != Some(b"\r\n") {
                return Err("missing chunk terminator".into());
            }
            input = input.get(count + 2..).ok_or("missing next chunk")?;
        }
    }

    // Streaming adaptation of this repository's xtask/src/sha256.rs, MIT OR Apache-2.0.
    // Hashes every served WASM byte with a 64-byte state instead of loading the artifact.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    struct Digest {
        state: [u32; 8],
        buffer: [u8; 64],
        used: usize,
        length: u64,
    }
    impl Digest {
        fn new() -> Self {
            Self {
                state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
                buffer: [0; 64],
                used: 0,
                length: 0,
            }
        }
        fn update(&mut self, mut bytes: &[u8]) -> Result<(), String> {
            self.length = self.length.checked_add(bytes.len() as u64).ok_or("SHA length overflow")?;
            while !bytes.is_empty() {
                let count = (64 - self.used).min(bytes.len());
                self.buffer.get_mut(self.used..self.used + count).ok_or("SHA buffer bounds")?.copy_from_slice(bytes.get(..count).ok_or("SHA input bounds")?);
                bytes = bytes.get(count..).ok_or("SHA remaining bounds")?;
                self.used += count;
                if self.used == 64 {
                    self.compress();
                    self.used = 0;
                }
            }
            Ok(())
        }
        fn compress(&mut self) {
            let mut w = [0_u32; 64];
            for (i, word) in self.buffer.as_chunks::<4>().0.iter().enumerate() {
                w[i] = u32::from_be_bytes(*word);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let t1 = h.wrapping_add(s1).wrapping_add((e & f) ^ (!e & g)).wrapping_add(K[i]).wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            for (word, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
                *word = word.wrapping_add(value);
            }
        }
        fn finish(mut self) -> Result<String, String> {
            let bits = self.length.checked_mul(8).ok_or("SHA bit length overflow")?;
            self.update(&[0x80])?;
            while self.used != 56 {
                self.update(&[0])?;
            }
            self.update(&bits.to_be_bytes())?;
            Ok(self.state.iter().map(|word| format!("{word:08x}")).collect())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{Digest, artifact_path, unchunk};
        #[test]
        fn artifact_routes_reject_non_generated_and_escaping_inputs() {
            assert_eq!(artifact_path("/token/run.js", "token").ok(), Some("run.js"));
            assert_eq!(artifact_path("/token/", "token").ok(), Some("/"));
            for path in
                ["/wrong/run.js", "/token/../Cargo.toml", "/token/%2e%2e/run.js", "/token/snippets/../run.js", "/token/run.js?x=1", "/token/snippets/source.rs"]
            {
                assert!(artifact_path(path, "token").is_err(), "{path}");
            }
        }
        #[test]
        fn chunked_index_requires_complete_bounded_framing() {
            assert_eq!(unchunk(b"3\r\nabc\r\n0\r\n\r\n").ok(), Some(b"abc".to_vec()));
            for bytes in [b"3\r\nab".as_slice(), b"0\r\nx", b"ffffff\r\n", b"1;extension\r\na\r\n0\r\n\r\n"] {
                assert!(unchunk(bytes).is_err());
            }
        }
        #[test]
        fn streaming_artifact_sha_matches_existing_whole_input_vectors() -> Result<(), String> {
            for length in [0, 1, 55, 56, 63, 64, 65, 1000, 131079] {
                let bytes = vec![b'a'; length];
                let mut digest = Digest::new();
                for part in bytes.chunks(37) {
                    digest.update(part)?;
                }
                assert_eq!(digest.finish()?, crate::sha256::hex(&bytes));
            }
            Ok(())
        }
    }
}
