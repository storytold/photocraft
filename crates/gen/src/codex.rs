//! Newline-delimited JSON-RPC over the installed Codex runtime's stdio transport.
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{EditRequest, EditResult, Error, ImageProvider};

mod account;
mod runtime;
pub use account::{AccountInfo, AccountService, valid_login_url};

#[derive(Clone)]
pub struct CodexProvider {
    /// Empty discovers an installed runtime on each connection. An explicit path must point
    /// inside the COMPLETE desktop runtime; do not copy the executable out.
    pub executable: PathBuf,
    pub timeout: Duration,
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self { executable: PathBuf::new(), timeout: Duration::from_secs(600) }
    }
}

struct Rpc {
    _server: Server,
    input: ChildStdin,
    output: Receiver<Result<Value, Error>>,
    deadline: Instant,
    pending: VecDeque<Value>,
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        // Closing a job also closes its private server, including on timeout/error/cancel.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Rpc {
    fn start(provider: &CodexProvider, workspace: &Path, chatgpt_only: bool) -> Result<Self, Error> {
        let deadline = Instant::now().checked_add(provider.timeout).ok_or_else(|| Error::Provider("Invalid Codex timeout".into()))?;
        // Resolve in the worker, before changing cwd to the private image workspace. This also
        // refreshes versioned desktop paths after Codex updates while PhotoCraft stays open.
        let executable = runtime::resolve(&provider.executable)?;
        let mut cmd = Command::new(executable);
        cmd.args(["app-server", "--stdio", "-c", "model_provider=\"openai\"", "-c", "features.shell_tool=false", "-c", "web_search=\"disabled\""])
            .current_dir(workspace)
            .env_remove("OPENAI_API_KEY")
            .env_remove("OPENAI_BASE_URL")
            .env_remove("CODEX_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if chatgpt_only {
            cmd.args(["-c", "forced_login_method=\"chatgpt\""]);
        }
        // An explicit profile is shared by authentication and edits, and never changes the
        // desktop app's normal login. File storage prevents an isolated test using its keyring.
        if let Some(home) = std::env::var_os("PHOTOCRAFT_CODEX_HOME") {
            cmd.env("CODEX_HOME", home).args(["-c", "cli_auth_credentials_store=\"file\""]);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().map_err(|e| {
            Error::Provider(format!("Could not start the complete Codex runtime: {e}. Set PHOTOCRAFT_CODEX_PATH to its original executable location."))
        })?;
        let mut server = Server(child);
        let input = server.0.stdin.take().ok_or_else(|| Error::Provider("Codex stdin unavailable".into()))?;
        let stdout = server.0.stdout.take().ok_or_else(|| Error::Provider("Codex stdout unavailable".into()))?;
        let (tx, output) = mpsc::sync_channel(32);
        std::thread::Builder::new().name("photocraft-codex-rpc".into()).spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                // Bound a single protocol message. Images arrive by savedPath, not base64.
                let read = reader.by_ref().take(8 * 1024 * 1024 + 1).read_until(b'\n', &mut line);
                let value = match read {
                    Ok(0) => break,
                    Ok(n) if n <= 8 * 1024 * 1024 => serde_json::from_slice(&line).map_err(|_| Error::Provider("Invalid Codex JSON-RPC message".into())),
                    Ok(_) => Err(Error::Provider("Codex protocol message exceeds 8 MiB".into())),
                    Err(e) => Err(Error::Io(e)),
                };
                let failed = value.is_err();
                if tx.send(value).is_err() || failed {
                    break;
                }
            }
        })?;
        Ok(Self { _server: server, input, output, deadline, pending: VecDeque::new() })
    }

    fn send(&mut self, value: Value) -> Result<(), Error> {
        serde_json::to_writer(&mut self.input, &value).map_err(|_| Error::Provider("Could not encode Codex request".into()))?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }

    fn next(&mut self, cancelled: &dyn Fn() -> bool) -> Result<Value, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        if let Some(v) = self.pending.pop_front() {
            return Ok(v);
        }
        self.receive(cancelled)
    }

    fn receive(&mut self, cancelled: &dyn Fn() -> bool) -> Result<Value, Error> {
        loop {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            if Instant::now() >= self.deadline {
                return Err(Error::Timeout);
            }
            match self.output.recv_timeout(Duration::from_millis(100)) {
                Ok(v) => return v,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(Error::Provider("Codex exited before returning an image. Verify the complete runtime and ChatGPT sign-in.".into())),
            }
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value, cancel: &dyn Fn() -> bool) -> Result<Value, Error> {
        self.send(json!({"id": id, "method": method, "params": params}))?;
        loop {
            let v = self.receive(cancel)?;
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                if v.get("error").is_some() {
                    return Err(Error::Provider(format!("Codex rejected {method}; check runtime version and account configuration")));
                }
                return v.get("result").cloned().ok_or_else(|| Error::Provider("Missing RPC result".into()));
            }
            self.reject_request(&v)?;
            if v.get("id").is_none() {
                if self.pending.len() >= 1024 {
                    return Err(Error::Provider("Too many Codex notifications before RPC response".into()));
                }
                self.pending.push_back(v);
            }
        }
    }

    fn reject_request(&mut self, v: &Value) -> Result<(), Error> {
        if v.get("method").is_some()
            && let Some(id) = v.get("id")
        {
            // This image-only integration never authorizes shell commands, file writes or external tools.
            self.send(
                json!({"id":id,"error":{"code":-32601,"message":"PhotoCraft supports image generation only; interactive tool requests are unavailable"}}),
            )?;
        }
        Ok(())
    }
}

fn chatgpt_account(value: &Value) -> Result<(), Error> {
    if value.pointer("/account/type").and_then(Value::as_str) != Some("chatgpt") {
        return Err(Error::Provider("Sign in with ChatGPT from AI Edit or the installed Codex app. PhotoCraft will not use API-key billing.".into()));
    }
    Ok(())
}

fn completed_image(v: &Value, thread: &str) -> Option<PathBuf> {
    if v.get("method")?.as_str()? != "item/completed" || v.pointer("/params/threadId")?.as_str()? != thread {
        return None;
    }
    let item = v.pointer("/params/item")?;
    if item.get("type")?.as_str()? != "imageGeneration" || item.get("status")?.as_str()? != "completed" {
        return None;
    }
    item.get("savedPath")?.as_str().filter(|p| !p.is_empty()).map(PathBuf::from)
}

impl ImageProvider for CodexProvider {
    fn edit(&self, request: &EditRequest<'_>, cancelled: &dyn Fn() -> bool) -> Result<EditResult, Error> {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let mut rpc = Rpc::start(self, request.workspace, true)?;
        rpc.request(
            1,
            "initialize",
            json!({"clientInfo":{"name":"photocraft_ai","title":"PhotoCraft AI","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}),
            cancelled,
        )?;
        rpc.send(json!({"method":"initialized"}))?;
        let account = rpc.request(2, "account/read", json!({"refreshToken":true}), cancelled).map_err(|error| match error {
            Error::Cancelled | Error::Timeout => error,
            _ => Error::Provider("Could not verify ChatGPT sign-in. Check the connection or sign in again from AI Edit; no layer was added.".into()),
        })?;
        chatgpt_account(&account)?;
        let start = rpc.request(3, "thread/start", json!({
            "cwd":request.workspace,"ephemeral":true,"modelProvider":"openai","approvalPolicy":"untrusted","sandbox":"workspace-write",
            "baseInstructions":"You are PhotoCraft's image editor. Use the native image generation tool to EDIT the supplied image. Do not use shell, code, web, MCP, skills or other tools. Preserve canvas dimensions. Return one edited image. If image generation is unavailable, report failure; never use an API or substitute an explanation.",
        }), cancelled)?;
        let thread = start.pointer("/thread/id").and_then(Value::as_str).ok_or_else(|| Error::Provider("Codex returned no thread id".into()))?.to_owned();
        rpc.request(
            4,
            "turn/start",
            json!({"threadId":thread,"input":[
                {"type":"text","text":format!("Edit the attached current canvas using image generation. Keep its dimensions. Instruction:\n{}",request.prompt)},
                {"type":"localImage","path":request.image}
            ]}),
            cancelled,
        )?;
        let mut saved = None;
        loop {
            let v = rpc.next(cancelled)?;
            rpc.reject_request(&v)?;
            if let Some(path) = completed_image(&v, &thread) {
                saved = Some(path);
            }
            if v.get("method").and_then(Value::as_str) == Some("turn/completed") && v.pointer("/params/threadId").and_then(Value::as_str) == Some(&thread) {
                if v.pointer("/params/turn/status").and_then(Value::as_str) != Some("completed") {
                    return Err(Error::Provider("Codex image edit failed or was interrupted; no layer was added".into()));
                }
                let path = saved.ok_or_else(|| {
                    Error::Provider(
                        "Codex completed without an imageGeneration savedPath. Check that this is the complete desktop runtime with image editing available."
                            .into(),
                    )
                })?;
                if !path.is_absolute() || !path.is_file() {
                    return Err(Error::Provider("Codex returned an unavailable image path".into()));
                }
                return Ok(EditResult { saved_path: path });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_api_key_and_missing_login() {
        for v in [json!({"account":null}), json!({"account":{"type":"apiKey"}})] {
            assert!(chatgpt_account(&v).is_err());
        }
        assert!(chatgpt_account(&json!({"account":{"type":"chatgpt"}})).is_ok());
    }
    #[test]
    fn requires_completed_image_event_for_our_thread() {
        let mut v =
            json!({"method":"item/completed","params":{"threadId":"ours","item":{"type":"imageGeneration","status":"completed","savedPath":"/image.png"}}});
        assert_eq!(completed_image(&v, "ours"), Some(PathBuf::from("/image.png")));
        assert!(completed_image(&v, "other").is_none());
        v["params"]["item"]["status"] = json!("in_progress");
        assert!(completed_image(&v, "ours").is_none());
    }
}
