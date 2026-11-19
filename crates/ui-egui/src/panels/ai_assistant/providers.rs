//! Subscription-backed CLI providers (`claude` and `gemini`) with live MCP tool access.
//!
//! Launches the user's installed `claude` (Claude Pro/Max subscription) or `gemini` (Google/Gemini
//! subscription) CLI on a background thread, attaching PhotoCraft's MCP server (`photocraft-cli mcp`)
//! so the model can inspect and edit the open document directly.

use std::fs;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::AiProvider;
use super::connector;

/// Auto-detected paths to local AI CLIs and `photocraft-cli`.
#[derive(Clone, Debug, Default)]
pub struct DetectedClis {
    pub claude: Option<PathBuf>,
    pub gemini: Option<PathBuf>,
    pub photocraft_cli: Option<PathBuf>,
}

impl DetectedClis {
    /// Scan `PATH` and common macOS/Linux/Windows binary directories.
    pub fn detect() -> Self {
        Self { claude: find_binary("claude", &extra_cli_dirs()), gemini: find_binary("gemini", &extra_cli_dirs()), photocraft_cli: find_photocraft_cli() }
    }

    pub fn path_for(&self, provider: AiProvider) -> Option<&Path> {
        match provider {
            AiProvider::Claude => self.claude.as_deref(),
            AiProvider::Gemini => self.gemini.as_deref(),
        }
    }
}

/// Incremental events streamed back from the worker thread to the `egui` UI loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    TextDelta(String),
    ToolUse { name: String, summary: String },
    SessionId(String),
    Error(String),
    Done,
}

/// A running AI subprocess that can be polled non-blockingly and cancelled cleanly.
pub struct ActiveRun {
    pub rx: Receiver<StreamEvent>,
    cancelled: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    kill_tree: bool,
}

impl ActiveRun {
    /// Request cancellation and kill the child process (and process tree for Windows batch shims) if still alive.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let mut guard = self.child.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(child) = guard.as_mut() {
            terminate_child(child, self.kill_tree);
        }
    }
}

impl Drop for ActiveRun {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Parameters required to start a single assistant turn.
pub struct RunRequest {
    pub provider: AiProvider,
    pub model: Option<String>,
    pub cli_bin: PathBuf,
    pub photocraft_cli: Option<PathBuf>,
    /// Live in-app bridge `(port, token_file)` when the app's agent link is active.
    pub live_bridge: Option<(u16, PathBuf)>,
    pub prompt: String,
    pub session_id: Option<String>,
    pub gemini_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub ctx: egui::Context,
}

/// Spawn a background worker that runs `claude` or `gemini` with PhotoCraft's MCP server attached.
pub fn spawn(req: RunRequest) -> ActiveRun {
    let (tx, rx) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
    let kill_tree = uses_windows_batch_shim(&req.cli_bin);

    let worker_cancelled = Arc::clone(&cancelled);
    let worker_child = Arc::clone(&child_slot);
    let _ = std::thread::Builder::new().name("ai-assistant".into()).spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_worker(req, &tx, &worker_cancelled, &worker_child, kill_tree);
        }));
        if outcome.is_err() {
            let _ = tx.send(StreamEvent::Error("AI worker exited unexpectedly.".to_owned()));
            let _ = tx.send(StreamEvent::Done);
        }
    });

    ActiveRun { rx, cancelled, child: child_slot, kill_tree }
}

fn run_worker(req: RunRequest, tx: &Sender<StreamEvent>, cancelled: &AtomicBool, child_slot: &Mutex<Option<Child>>, kill_tree: bool) {
    let temp_dir = make_temp_run_dir();
    let cmd_res = build_command(&req, temp_dir.as_deref());
    let (mut cmd, stdin_prompt) = match cmd_res {
        Ok(pair) => pair,
        Err(e) => {
            send_wake(tx, &req.ctx, StreamEvent::Error(e));
            send_wake(tx, &req.ctx, StreamEvent::Done);
            cleanup_temp_dir(temp_dir.as_deref());
            return;
        }
    };

    let stdin_cfg = if stdin_prompt.is_some() { Stdio::piped() } else { Stdio::null() };
    let mut child = match cmd.stdin(stdin_cfg).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            send_wake(tx, &req.ctx, StreamEvent::Error(format!("Failed to launch {}: {e}", req.cli_bin.display())));
            send_wake(tx, &req.ctx, StreamEvent::Done);
            cleanup_temp_dir(temp_dir.as_deref());
            return;
        }
    };

    let stdin_pipe = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    {
        let mut guard = child_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cancelled.load(Ordering::SeqCst) {
            terminate_child(&mut child, kill_tree);
        }
        *guard = Some(child);
    }

    let stderr_handle = stderr.map(|err_pipe| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let mut reader = BufReader::new(err_pipe);
            let _ = std::io::Read::read_to_string(&mut reader, &mut buf);
            buf
        })
    });

    // Write piped prompt on a dedicated thread after publishing `child` and starting readers so
    // large prompts cannot deadlock the pipe buffer and cancellation closes the pipe cleanly.
    let stdin_handle = match (stdin_prompt, stdin_pipe) {
        (Some(prompt_text), Some(mut stdin)) => Some(std::thread::spawn(move || {
            let _ = stdin.write_all(prompt_text.as_bytes());
        })),
        _ => None,
    };

    let mut emitted_text = false;
    if let Some(out_pipe) = stdout {
        let reader = BufReader::new(out_pipe);
        for line in reader.lines().map_while(Result::ok) {
            if cancelled.load(Ordering::SeqCst) {
                break;
            }
            for ev in parse_stream_line(req.provider, &line) {
                if matches!(ev, StreamEvent::TextDelta(_)) {
                    emitted_text = true;
                }
                send_wake(tx, &req.ctx, ev);
            }
        }
    }

    let exit_status = wait_child_cancellable(child_slot, cancelled, kill_tree);
    if let Some(h) = stdin_handle {
        let _ = h.join();
    }
    let stderr_text = stderr_handle.and_then(|h| h.join().ok()).unwrap_or_default();

    if !cancelled.load(Ordering::SeqCst)
        && let Some(status) = exit_status
        && !status.success()
        && !emitted_text
    {
        let detail = clean_cli_error(&stderr_text);
        let msg = if detail.is_empty() {
            format!("{} exited with status {status}.", req.provider.label())
        } else {
            format!("{} error: {detail}", req.provider.label())
        };
        send_wake(tx, &req.ctx, StreamEvent::Error(msg));
    }

    cleanup_temp_dir(temp_dir.as_deref());
    send_wake(tx, &req.ctx, StreamEvent::Done);
}

fn send_wake(tx: &Sender<StreamEvent>, ctx: &egui::Context, ev: StreamEvent) {
    let _ = tx.send(ev);
    ctx.request_repaint();
}

/// Terminate `child`, and on Windows when `kill_tree` is set (`.cmd`/`.bat` shims),
/// terminate the entire process tree via `taskkill /T /F` so descendant Node/MCP processes
/// do not keep inherited stdout/stderr pipe handles open.
fn terminate_child(child: &mut Child, kill_tree: bool) {
    #[cfg(windows)]
    if kill_tree {
        let pid = child.id().to_string();
        let _ = Command::new("taskkill").args(["/PID", &pid, "/T", "/F"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    let _ = kill_tree;
    let _ = child.kill();
}

/// Poll `child_slot` with short mutex acquisitions so `ActiveRun::cancel` and `Drop` can acquire
/// the lock promptly and kill the subprocess without deadlocking on a blocking `wait()`.
fn wait_child_cancellable(child_slot: &Mutex<Option<Child>>, cancelled: &AtomicBool, kill_tree: bool) -> Option<ExitStatus> {
    loop {
        let mut guard = child_slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let child = guard.as_mut()?;
        if cancelled.load(Ordering::SeqCst) {
            terminate_child(child, kill_tree);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                *guard = None;
                return Some(status);
            }
            Ok(None) => {
                drop(guard);
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                *guard = None;
                return None;
            }
        }
    }
}

/// Whether `path` is a Windows batch shim (`.cmd`/`.bat`), where `cmd.exe` splits arguments
/// containing newlines and prompts must be delivered via stdin instead.
fn uses_windows_batch_shim(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
}

fn build_command(req: &RunRequest, temp_dir: Option<&Path>) -> Result<(Command, Option<String>), String> {
    let mut cmd = Command::new(&req.cli_bin);
    let via_stdin = uses_windows_batch_shim(&req.cli_bin);
    let mut stdin_prompt = None;
    let system_prompt = "You are embedded inside PhotoCraft, a desktop photo editor. \
        Use the `photocraft` MCP tools (`doc_inspect`, `doc_place_svg`, `command_list`, `command_run`, `command_batch`, etc.) \
        to read the open document and apply requested edits directly. \
        For detailed drawings, illustrations, scenes, icons, or complex vector art, generate rich multi-path `<svg>` markup \
        (with Bezier curves, gradients, shading, and highlights) and place it cleanly in one call using `doc_place_svg`. Keep replies concise.";

    match req.provider {
        AiProvider::Claude => {
            cmd.arg("-p");
            if via_stdin {
                stdin_prompt = Some(req.prompt.clone());
            } else {
                cmd.arg(&req.prompt);
            }
            cmd.arg("--output-format").arg("stream-json").arg("--verbose").arg("--append-system-prompt").arg(system_prompt);
            if let Some(model) = req.model.as_deref().filter(|m| !m.is_empty()) {
                cmd.arg("--model").arg(model);
            }
            if let Some(sid) = req.session_id.as_deref().filter(|s| !s.is_empty()) {
                cmd.arg("--resume").arg(sid);
            }
            if let Some(key) = req.anthropic_api_key.as_deref().filter(|k| !k.is_empty()) {
                cmd.env("ANTHROPIC_API_KEY", key);
            }
            if let (Some(cli), Some(dir)) = (req.photocraft_cli.as_deref(), temp_dir) {
                let mcp_path = dir.join("mcp.json");
                let bridge = req.live_bridge.as_ref().map(|(port, token)| (*port, token.as_path()));
                let cfg = json!({
                    "mcpServers": {
                        "photocraft": connector::server_entry_with_bridge(cli, bridge)
                    }
                });
                let bytes = serde_json::to_vec_pretty(&cfg).map_err(|e| format!("Failed to encode MCP config: {e}"))?;
                fs::write(&mcp_path, bytes).map_err(|e| format!("Failed to write MCP config: {e}"))?;
                cmd.arg("--mcp-config").arg(&mcp_path).arg("--allowedTools").arg("mcp__photocraft__*");
            }
        }
        AiProvider::Gemini => {
            let combined_prompt = format!("{system_prompt}\n\nUser request: {}", req.prompt);
            if via_stdin {
                cmd.arg("-p").arg("");
                stdin_prompt = Some(combined_prompt);
            } else {
                cmd.arg("-p").arg(combined_prompt);
            }
            cmd.arg("--output-format").arg("stream-json").env("GEMINI_CLI_TRUST_WORKSPACE", "true");
            if let Some(model) = req.model.as_deref().filter(|m| !m.is_empty()) {
                cmd.arg("-m").arg(model);
            }
            if req.photocraft_cli.is_some() {
                cmd.arg("--allowed-mcp-server-names").arg("photocraft");
            }
            let has_api_key = req.gemini_api_key.as_deref().is_some_and(|k| !k.is_empty());
            if let Some(key) = req.gemini_api_key.as_deref().filter(|k| !k.is_empty()) {
                cmd.env("GEMINI_API_KEY", key);
            }
            if let Some(dir) = temp_dir {
                let gemini_dir = dir.join(".gemini");
                fs::create_dir_all(&gemini_dir).map_err(|e| format!("Failed to create temp .gemini dir: {e}"))?;
                let settings_path = gemini_dir.join("settings.json");
                let mut cfg = serde_json::Map::new();
                if let Some(cli) = req.photocraft_cli.as_deref() {
                    let bridge = req.live_bridge.as_ref().map(|(port, token)| (*port, token.as_path()));
                    let mut entry = connector::server_entry_with_bridge(cli, bridge);
                    if let Some(obj) = entry.as_object_mut() {
                        // Trust only the PhotoCraft MCP server so `-p` mode can invoke PhotoCraft
                        // tools without `--yolo` auto-approving arbitrary shell/file actions.
                        obj.insert("trust".to_owned(), Value::Bool(true));
                    }
                    cfg.insert("mcpServers".to_owned(), json!({ "photocraft": entry }));
                }
                cfg.insert(
                    "tools".to_owned(),
                    json!({
                        "exclude": ["run_shell_command", "write_file", "replace", "edit", "delete_file", "save_memory"]
                    }),
                );
                if has_api_key {
                    // Override any global `oauth-personal` setting so GEMINI_API_KEY is honored.
                    cfg.insert("security".to_owned(), json!({ "auth": { "selectedType": "gemini-api-key" } }));
                }
                let bytes = serde_json::to_vec_pretty(&Value::Object(cfg)).map_err(|e| format!("Failed to encode Gemini MCP settings: {e}"))?;
                fs::write(&settings_path, bytes).map_err(|e| format!("Failed to write Gemini MCP settings: {e}"))?;
                cmd.current_dir(dir);
            }
        }
    }
    Ok((cmd, stdin_prompt))
}

/// Parse one NDJSON line from `claude --output-format stream-json` or `gemini --output-format stream-json`.
pub fn parse_stream_line(provider: AiProvider, line: &str) -> Vec<StreamEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
        return vec![StreamEvent::TextDelta(format!("{trimmed}\n"))];
    };
    match provider {
        AiProvider::Claude => parse_claude_event(&val),
        AiProvider::Gemini => parse_gemini_event(&val),
    }
}

fn parse_claude_event(val: &Value) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    if let Some(sid) = val.get("session_id").and_then(Value::as_str) {
        out.push(StreamEvent::SessionId(sid.to_owned()));
    }
    match val.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            if let Some(blocks) = val.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(Value::as_str)
                                && !t.is_empty()
                            {
                                out.push(StreamEvent::TextDelta(t.to_owned()));
                            }
                        }
                        Some("tool_use") => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool").to_owned();
                            let summary = summarize_tool_input(block.get("input"));
                            out.push(StreamEvent::ToolUse { name, summary });
                        }
                        _ => {}
                    }
                }
            }
        }
        Some("result") => {
            if val.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                && let Some(err) = val.get("result").and_then(Value::as_str)
            {
                out.push(StreamEvent::Error(err.to_owned()));
            }
        }
        Some("error") => {
            let msg = val.get("error").and_then(Value::as_str).or_else(|| val.get("message").and_then(Value::as_str)).unwrap_or("Unknown Claude error");
            out.push(StreamEvent::Error(msg.to_owned()));
        }
        _ => {}
    }
    out
}

fn parse_gemini_event(val: &Value) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    match val.get("type").and_then(Value::as_str) {
        Some("message") => {
            let role = val.get("role").and_then(Value::as_str).unwrap_or("");
            if role == "assistant"
                && let Some(text) = val.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                out.push(StreamEvent::TextDelta(text.to_owned()));
            }
        }
        Some("tool_use") => {
            let name = val.get("tool_name").or_else(|| val.get("name")).and_then(Value::as_str).unwrap_or("tool").to_owned();
            if name != "update_topic" {
                let summary = summarize_tool_input(val.get("parameters").or_else(|| val.get("input")));
                out.push(StreamEvent::ToolUse { name, summary });
            }
        }
        Some("error") => {
            let msg = val.get("message").or_else(|| val.get("error")).and_then(Value::as_str).unwrap_or("Unknown Gemini error");
            out.push(StreamEvent::Error(msg.to_owned()));
        }
        _ => {}
    }
    out
}

fn summarize_tool_input(input: Option<&Value>) -> String {
    let Some(val) = input else { return String::new() };
    let compact = serde_json::to_string(val).unwrap_or_default();
    if compact == "{}" || compact == "null" {
        return String::new();
    }
    if compact.len() > 96 {
        let end = compact.floor_char_boundary(96);
        format!("{}…", &compact[..end])
    } else {
        compact
    }
}

/// Strip ANSI escape sequences and noisy CLI startup boilerplate from stderr output.
pub(crate) fn clean_cli_error(stderr: &str) -> String {
    let stripped = strip_ansi(stderr);
    let mut lines = Vec::new();
    for line in stripped.lines().map(str::trim) {
        if line.is_empty()
            || line.starts_with("Warning: Basic terminal")
            || line.starts_with("Warning: 256-color")
            || line.starts_with("YOLO mode is enabled")
            || line.starts_with("Ripgrep is not available")
            || line.starts_with("[STARTUP]")
            || line.starts_with("at ")
        {
            continue;
        }
        lines.push(line);
        if lines.len() >= 6 {
            break;
        }
    }
    lines.join("\n")
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            let _ = chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn make_temp_run_dir() -> Option<PathBuf> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("photocraft-ai-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn cleanup_temp_dir(dir: Option<&Path>) {
    if let Some(d) = dir {
        let _ = fs::remove_dir_all(d);
    }
}

fn find_photocraft_cli() -> Option<PathBuf> {
    let exe_name = if cfg!(windows) { "photocraft-cli.exe" } else { "photocraft-cli" };
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let sibling = dir.join(exe_name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    find_binary("photocraft-cli", &extra_cli_dirs())
}

fn extra_cli_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];
    if let Some(home) = connector::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join(".cargo/bin"));
    }
    #[cfg(target_os = "windows")]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    dirs
}

fn find_binary(name: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    let candidates: Vec<String> = if cfg!(windows) { vec![format!("{name}.exe"), format!("{name}.cmd")] } else { vec![name.to_owned()] };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for bin in &candidates {
                let candidate = dir.join(bin);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    for dir in extra_dirs {
        for bin in &candidates {
            let candidate = dir.join(bin);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_assistant_text_and_tool_use() {
        let line = r#"{"type":"assistant","session_id":"sess-42","message":{"content":[{"type":"tool_use","name":"mcp__photocraft__doc_inspect","input":{"doc_id":1}},{"type":"text","text":"Checked the canvas."}]}}"#;
        let events = parse_stream_line(AiProvider::Claude, line);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], StreamEvent::SessionId("sess-42".to_owned()));
        assert!(matches!(&events[1], StreamEvent::ToolUse { name, .. } if name == "mcp__photocraft__doc_inspect"));
        assert_eq!(events[2], StreamEvent::TextDelta("Checked the canvas.".to_owned()));
    }

    #[test]
    fn parses_gemini_message_and_tool_use() {
        let msg = r#"{"type":"message","role":"assistant","content":"Adjusting exposure now."}"#;
        assert_eq!(parse_stream_line(AiProvider::Gemini, msg), vec![StreamEvent::TextDelta("Adjusting exposure now.".to_owned())]);

        let tool = r#"{"type":"tool_use","tool_name":"command_run","parameters":{"command":"image.autoTone"}}"#;
        let events = parse_stream_line(AiProvider::Gemini, tool);
        assert!(matches!(events.first(), Some(StreamEvent::ToolUse { name, .. }) if name == "command_run"));

        let internal = r#"{"type":"tool_use","tool_name":"update_topic","parameters":{"strategic_intent":"x"}}"#;
        assert!(parse_stream_line(AiProvider::Gemini, internal).is_empty());
    }

    #[test]
    fn batch_shim_delivers_multiline_prompt_via_stdin() {
        let ctx = egui::Context::default();
        for (provider, bin) in [(AiProvider::Claude, "C:\\npm\\claude.cmd"), (AiProvider::Gemini, "C:\\npm\\gemini.cmd")] {
            let req = RunRequest {
                provider,
                model: None,
                cli_bin: PathBuf::from(bin),
                photocraft_cli: None,
                live_bridge: None,
                prompt: "line one\nline two".to_owned(),
                session_id: None,
                gemini_api_key: None,
                anthropic_api_key: None,
                ctx: ctx.clone(),
            };
            let (cmd, stdin_prompt) = build_command(&req, None).unwrap();
            for arg in cmd.get_args() {
                let s = arg.to_string_lossy();
                assert!(!s.contains('\n') && !s.contains('\r'), "arg for {bin} must not contain newlines: {s:?}");
            }
            let piped = stdin_prompt.expect("batch shim must pipe prompt via stdin");
            assert!(piped.contains("line one\nline two"));
        }

        let exe_req = RunRequest {
            provider: AiProvider::Claude,
            model: None,
            cli_bin: PathBuf::from("C:\\bin\\claude.exe"),
            photocraft_cli: None,
            live_bridge: None,
            prompt: "hello".to_owned(),
            session_id: None,
            gemini_api_key: None,
            anthropic_api_key: None,
            ctx,
        };
        let (_cmd, stdin_prompt) = build_command(&exe_req, None).unwrap();
        assert!(stdin_prompt.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_shim_receives_multiline_prompt_via_stdin() {
        let dir = make_temp_run_dir().unwrap();
        let shim = dir.join("echo-stdin.cmd");
        fs::write(&shim, "@echo off\r\nmore\r\n").unwrap();
        let req = RunRequest {
            provider: AiProvider::Gemini,
            model: None,
            cli_bin: shim,
            photocraft_cli: None,
            live_bridge: None,
            prompt: "first line\nsecond line".to_owned(),
            session_id: None,
            gemini_api_key: None,
            anthropic_api_key: None,
            ctx: egui::Context::default(),
        };
        let (mut cmd, stdin_prompt) = build_command(&req, Some(&dir)).unwrap();
        let piped = stdin_prompt.unwrap();
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(piped.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        cleanup_temp_dir(Some(&dir));
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("first line") && stdout.contains("second line"), "unexpected stdout: {stdout:?}");
    }

    #[test]
    fn summarizes_multibyte_utf8_tool_input_without_panicking() {
        let long_turkish = json!({"prompt": "şğüöçıİ".repeat(20)});
        let summary = summarize_tool_input(Some(&long_turkish));
        assert!(summary.ends_with('…'));
        assert!(summary.len() <= 96 + '…'.len_utf8());
    }

    #[test]
    fn cleans_ansi_and_startup_noise_from_stderr() {
        let raw = "Warning: Basic terminal detected (TERM=dumb).\nYOLO mode is enabled.\n\x1b[31mRate limit exceeded\x1b[0m\n    at foo.js:12";
        assert_eq!(clean_cli_error(raw), "Rate limit exceeded");
    }
}
