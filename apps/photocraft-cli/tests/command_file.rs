//! Remote command-file behavior, including preflight and uncertain mutation outcomes.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(script: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pc-command-file-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("script.jsonl"), script).unwrap();
        std::fs::write(path.join("token"), "a".repeat(64)).unwrap();
        Self(path)
    }
    fn run(&self, port: u16) -> Output {
        Command::new(env!("CARGO_BIN_EXE_photocraft-cli"))
            .env_remove("PHOTOCRAFT_CONTROL_TOKEN")
            .env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE")
            .args(["run", "--control", &port.to_string(), "--control-token-file"])
            .arg(self.0.join("token"))
            .arg(self.0.join("script.jsonl"))
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn read(reader: &mut BufReader<TcpStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
fn reply(reader: &mut BufReader<TcpStream>, request: &Value, result: Value) {
    writeln!(reader.get_mut(), "{}", json!({"id":request["id"],"ok":true,"result":result})).unwrap();
}
fn auth(listener: &TcpListener) -> BufReader<TcpStream> {
    let (stream, _) = listener.accept().unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = BufReader::new(stream);
    let request = read(&mut reader);
    assert_eq!(request["method"], "auth");
    reply(&mut reader, &request, json!({}));
    reader
}

#[test]
fn sequential_commands_capture_typed_values_and_escape_dollars() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut reader = auth(&listener);
        let first = read(&mut reader);
        assert_eq!(first["method"], "engine.execute");
        assert_eq!(first["params"]["command"], "layer.new.layer");
        reply(&mut reader, &first, json!({"id":42,"visible":true,"a/b":{"~x":[null,7]}}));
        let second = read(&mut reader);
        assert_eq!(
            second["params"]["params"],
            json!({"layer":42,"nested":[true,{"literal":"$ink","value":7}],"whole":{"id":42,"visible":true,"a/b":{"~x":[null,7]}}})
        );
        reply(&mut reader, &second, json!({"done":true}));
    });
    let fixture = Fixture::new(
        "{\"command\":\"layer.new.layer\",\"saveAs\":\"ink\"}\n{\"command\":\"layer.rename\",\"params\":{\"layer\":\"$ink/id\",\"nested\":[\"$ink/visible\",{\"literal\":\"$$ink\",\"value\":\"$ink/a~1b/~0x/1\"}],\"whole\":\"$ink\"}}\n",
    );
    let output = fixture.run(port);
    server.join().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let results: Vec<Value> = String::from_utf8(output.stdout).unwrap().lines().map(|s| serde_json::from_str(s).unwrap()).collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["result"]["id"], 42);
    assert_eq!(results[1]["result"]["done"], true);
}

#[test]
fn invalid_scripts_are_rejected_before_connecting_or_mutating() {
    let cases = [
        ("", "empty"),
        ("[]", "command object"),
        ("{", "invalid JSON"),
        ("{\"command\":4}", "nonempty string"),
        ("{\"command\":\"x\",\"params\":[]}", "params must be an object"),
        ("{\"command\":\"x\",\"extra\":1}", "only command"),
        ("{\"command\":\"x\",\"params\":{\"layer\":\"$later\"},\"saveAs\":\"later\"}", "forward capture"),
        ("{\"command\":\"x\",\"saveAs\":\"a\"}\n{\"command\":\"y\",\"saveAs\":\"a\"}", "duplicate capture"),
        ("{\"command\":\"x\",\"saveAs\":\"a\"}\n{\"command\":\"y\",\"params\":{\"x\":\"$a/~2\"}}", "pointer escapes"),
        ("{\"command\":\"x\"}\n\n", "line 2"),
    ];
    for (script, expected) in cases {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let output = Fixture::new(script).run(listener.local_addr().unwrap().port());
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains(expected), "{error}");
        assert!(listener.accept().is_err(), "preflight opened a connection");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn lost_reply_stops_without_resending_mutation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut reader = auth(&listener);
        let request = read(&mut reader);
        assert_eq!(request["params"]["command"], "first");
        drop(reader);
        listener
    });
    let output = Fixture::new("{\"command\":\"first\"}\n{\"command\":\"second\"}").run(port);
    let listener = server.join().unwrap();
    listener.set_nonblocking(true).unwrap();
    assert!(listener.accept().is_err());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("line 1") && error.contains("may have completed"), "{error}");
}

#[test]
fn auth_refusal_does_not_send_commands_or_echo_remote_secrets() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let request = read(&mut reader);
        writeln!(reader.get_mut(), "{}", json!({"id":request["id"],"ok":false,"error":"synthetic-sensitive-message"})).unwrap();
        let mut line = String::new();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
    });
    let output = Fixture::new("{\"command\":\"first\"}").run(port);
    server.join().unwrap();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(error.contains("server rejected"));
    assert!(!error.contains("synthetic-sensitive-message"));
}

#[test]
fn missing_result_pointer_stops_before_next_command() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let mut reader = auth(&listener);
        let request = read(&mut reader);
        reply(&mut reader, &request, json!({}));
        let mut line = String::new();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
    });
    let output = Fixture::new("{\"command\":\"first\",\"saveAs\":\"a\"}\n{\"command\":\"next\",\"params\":{\"layer\":\"$a/missing\"}}").run(port);
    server.join().unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("line 2: JSON pointer does not exist"));
}

#[test]
fn connection_failure_and_unknown_flags_remain_actionable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let output = Fixture::new("{\"command\":\"first\"}").run(port);
    assert!(String::from_utf8_lossy(&output.stderr).contains("request was not sent"));
    let output = Command::new(env!("CARGO_BIN_EXE_photocraft-cli")).args(["run", "--control", "1", "--typo"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown flag --typo"));
}

#[test]
fn remote_flags_do_not_silently_change_local_run_behavior() {
    for flags in [vec!["--control", "1", "--new", "{}"], vec!["--token-file", "missing"], vec!["--control", "0"], vec!["--control", "1", "--control", "2"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_photocraft-cli")).arg("run").args(flags).arg("missing.jsonl").output().unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("cannot open"));
    }
}

#[test]
fn scripts_and_lines_have_bounded_read_budgets() {
    for script in [" ".repeat(8 * 1024 * 1024 + 1), format!("{{\"command\":\"x\",\"params\":{{\"text\":\"{}\"}}}}", "x".repeat(256 * 1024))] {
        let output = Fixture::new(&script).run(1);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("budget"));
    }
}
