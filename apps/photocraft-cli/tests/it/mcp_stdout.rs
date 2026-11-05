//! `photocraft-cli mcp` over real stdio: with the most verbose logging on, stdout carries only
//! JSON-RPC messages and the logs go to stderr.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn stdout_carries_only_protocol_messages_even_with_trace_logging() {
    let root = std::env::temp_dir().join(format!("pc-cli-mcp-stdout-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut child = Command::new(crate::binary())
        .args(["mcp", "--automation-read-root"])
        .arg(&root)
        .arg("--automation-write-root")
        .arg(&root)
        .env("PHOTOCRAFT_LOG", "trace")
        .env_remove("PHOTOCRAFT_CONTROL_PORT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start photocraft-cli mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    // Drain stderr on its own thread so trace logs can never fill the pipe and stall the server.
    let mut stderr = child.stderr.take().unwrap();
    let logs = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let mut seen = Vec::new();
    let mut next_id = 0;
    let mut send = |stdin: &mut std::process::ChildStdin, method: &str, params: Value, notify: bool| -> Option<i64> {
        let mut msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let id = (!notify).then(|| {
            next_id += 1;
            msg["id"] = json!(next_id);
            next_id
        });
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        id
    };
    let reply = |stdout: &mut BufReader<std::process::ChildStdout>, id: i64, seen: &mut Vec<Value>| -> Value {
        loop {
            let mut line = String::new();
            assert!(stdout.read_line(&mut line).unwrap() > 0, "stdout closed early");
            // Every stdout line must be a JSON-RPC 2.0 message: nothing else may be printed there.
            let v: Value = serde_json::from_str(line.trim_end()).unwrap_or_else(|e| panic!("non-protocol stdout line ({e}): {line:?}"));
            assert_eq!(v["jsonrpc"], "2.0", "{v}");
            assert!(v.get("id").is_some() || v.get("method").is_some(), "{v}");
            seen.push(v.clone());
            if v["id"] == json!(id) {
                return v;
            }
        }
    };

    let id = send(
        &mut stdin,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "stdout-test", "version": "1"}}),
        false,
    )
    .unwrap();
    assert!(reply(&mut stdout, id, &mut seen)["result"]["serverInfo"].is_object());
    send(&mut stdin, "notifications/initialized", json!({}), true);
    let id = send(&mut stdin, "tools/list", json!({}), false).unwrap();
    assert!(reply(&mut stdout, id, &mut seen)["result"]["tools"].as_array().is_some_and(|t| t.len() > 30));
    // Successful calls, tool errors and a protocol error: none may leak text onto stdout.
    for (name, args) in [
        ("doc_new", json!({"width": 32, "height": 16})),
        ("add_text", json!({"text": "hi", "x": 2, "y": 12, "size": 10})),
        ("resize_image", json!({"percent": 50})),
        ("export_image", json!({"path": "out/a.png"})),
        ("export_image", json!({"path": "out/a.png"})),
        ("open_image", json!({"path": "../outside.png"})),
        ("batch_process", json!({"input_dir": "out", "output_dir": "conv", "format": "jpg"})),
        ("resize_image", json!({"bogus": 1})),
    ] {
        let id = send(&mut stdin, "tools/call", json!({"name": name, "arguments": args}), false).unwrap();
        let r = reply(&mut stdout, id, &mut seen);
        assert!(r.get("result").is_some() || r.get("error").is_some(), "{r}");
    }
    drop(stdin);
    let status = child.wait().unwrap();
    // Anything left on stdout after the last reply must be protocol too.
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    for line in rest.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("non-protocol stdout line ({e}): {line:?}"));
        assert_eq!(v["jsonrpc"], "2.0");
    }
    let logs = logs.join().unwrap();
    assert!(status.success(), "exit {status:?}: {logs}");
    assert!(logs.contains("task tool ok") && logs.contains("task tool failed"), "the logs went to stderr: {logs}");
    assert!(seen.len() >= 10, "{seen:?}");
    let _ = std::fs::remove_dir_all(&root);
}
