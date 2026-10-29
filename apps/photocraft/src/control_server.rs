//! Localhost JSON-lines control server (one request per line, one reply per line).
//! Loopback only. This is the transport the MCP server will wrap.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use photocraft_automation::budgets::write_reply;
use photocraft_automation::security::{
    AUTH_DEADLINE, AuthDeadlineStream, ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream,
    read_bounded_line, repeat_auth_reply,
};
use photocraft_ui_egui::control::{ControlWatch, Dispatch, REPLY_DEADLINE, STALL_CHECK, Stalled, not_drawing_frames_error, stalled_action};
use photocraft_ui_egui::{ControlRequest, ControlResponse};
use serde_json::{Value, json};

pub fn start(port: u16, token: String, ctx: egui::Context) -> (Receiver<ControlRequest>, Arc<ControlWatch>) {
    let (tx, rx) = channel::<ControlRequest>();
    let watch = Arc::new(ControlWatch::new());
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("photocraft: control server failed to bind 127.0.0.1:{port}: {e}");
            return (rx, watch);
        }
    };
    eprintln!("photocraft: control server listening on {}", listening_addr(&listener, port));
    let accept_watch = Arc::clone(&watch);
    std::thread::spawn(move || {
        let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
        let token = Arc::new(token);
        for mut stream in listener.incoming().flatten() {
            let Some(permit) = limiter.try_acquire() else {
                let _ = configure_stream(&stream);
                let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                continue;
            };
            let tx = tx.clone();
            let ctx = ctx.clone();
            let token = Arc::clone(&token);
            let watch = Arc::clone(&accept_watch);
            std::thread::spawn(move || {
                let _permit = permit;
                serve(stream, &token, tx, ctx, &watch);
            });
        }
    });
    (rx, watch)
}

/// The address `listener` is bound to: with `--control 0` the OS picks the port, and a client
/// needs that one, not the requested 0 (#6427). Falls back to the requested port if the OS
/// can't report it.
fn listening_addr(listener: &TcpListener, requested_port: u16) -> String {
    match listener.local_addr() {
        Ok(addr) => addr.to_string(),
        Err(_) => format!("127.0.0.1:{requested_port}"),
    }
}

fn serve(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context, watch: &ControlWatch) {
    serve_with_auth_deadline(stream, token, tx, ctx, AUTH_DEADLINE, watch);
}

/// [`serve`] for a connection that must complete `auth` within `auth_deadline` of connecting,
/// however much it sends meanwhile (#4965); after that only the idle timeout applies.
fn serve_with_auth_deadline(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context, auth_deadline: Duration, watch: &ControlWatch) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(AuthDeadlineStream::new(read, auth_deadline));
    let mut out = stream;
    let mut line = String::new();
    let mut authenticated = false;
    loop {
        match read_bounded_line(&mut reader, &mut line) {
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let reply = json!({
                    "id": null,
                    "ok": false,
                    "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                });
                let _ = write_reply(&mut out, &reply);
                let _ = out.flush();
                break;
            }
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => {}
        }
        if !authenticated {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            if ok && reader.get_mut().authenticated().is_err() {
                break;
            }
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() || !authenticated {
                break;
            }
            continue;
        }
        if let Some(reply) = repeat_auth_reply(&line) {
            if write_reply(&mut out, &reply).is_err() || out.flush().is_err() {
                break;
            }
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                // `health` is answered from the watch alone, before the queue: it must not
                // need a frame, which is the state it exists to report on (#3667).
                let mut r = if method == "health" {
                    json!({"ok": true, "result": watch.health()})
                } else {
                    let deadline = Instant::now() + REPLY_DEADLINE;
                    let (mut req, rrx) = ControlRequest::new(method.clone(), params.clone());
                    req.deadline = Some(deadline);
                    let dispatch = req.dispatch.clone();
                    // Read before the send: a later read could see a logic pass the request
                    // itself woke, and wrongly serve a snapshot for a live loop.
                    let logic_before = watch.logic_count();
                    if tx.send(req).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                    wait_reply(&rrx, &dispatch, watch, &method, &params, logic_before, deadline)
                };
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
        };
        if write_reply(&mut out, &reply).is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }
    }
}

/// Wait for the frame thread's reply, falling back when the frame loop is wedged (#2493):
/// after [`STALL_CHECK`] without a reply, a live loop keeps waiting while a wedged one serves
/// the getter's last snapshot or fails fast. A dropped reply sender can never be answered; it
/// gets the deadline error. Giving up settles the request as abandoned (#3662); a served
/// snapshot leaves it queued, so the frame thread's eventual reply refreshes it.
fn wait_reply(
    rrx: &Receiver<ControlResponse>,
    dispatch: &Dispatch,
    watch: &ControlWatch,
    method: &str,
    params: &Value,
    logic_before: u64,
    deadline: Instant,
) -> Value {
    let until = deadline.saturating_duration_since(Instant::now()).min(STALL_CHECK);
    match rrx.recv_timeout(until) {
        Ok(r) => return r,
        Err(RecvTimeoutError::Disconnected) => return json!({"ok": false, "error": dispatch.give_up()}),
        Err(RecvTimeoutError::Timeout) => {}
    }
    if Instant::now() >= deadline {
        return json!({"ok": false, "error": dispatch.give_up()});
    }
    match stalled_action(watch, method, params, logic_before) {
        Stalled::KeepWaiting => match rrx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(r) => r,
            Err(_) => json!({"ok": false, "error": dispatch.give_up()}),
        },
        Stalled::Snapshot { age_ms, value } => {
            let mut value = (*value).clone();
            // Stamp into `result` when it is an object; a non-object result still gets the
            // flags, into the reply itself.
            if let Some(obj) = value.get_mut("result").and_then(Value::as_object_mut) {
                obj.insert("frameStalled".into(), json!(true));
                obj.insert("snapshotAgeMs".into(), json!(age_ms));
            } else if let Some(obj) = value.as_object_mut() {
                obj.insert("frameStalled".into(), json!(true));
                obj.insert("snapshotAgeMs".into(), json!(age_ms));
            }
            value
        }
        Stalled::NotDrawingFrames => json!({"ok": false, "error": not_drawing_frames_error()}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    /// #6427: with port 0 the log names the port the OS bound, which a client can connect to.
    #[test]
    fn port_zero_logs_the_bound_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let bound = listener.local_addr().unwrap();
        assert_ne!(bound.port(), 0);
        let logged = listening_addr(&listener, 0);
        assert_eq!(logged, format!("127.0.0.1:{}", bound.port()));
        TcpStream::connect(&logged).unwrap();
    }

    /// #4965: a connection that keeps sending without authenticating is closed at the deadline
    /// (releasing its connection permit), and nothing it sent is dispatched.
    #[test]
    fn an_unauthenticated_connection_is_closed_at_the_deadline_whatever_it_sends() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = ControlWatch::new();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let started = Instant::now();
            serve_with_auth_deadline(stream, "unused", tx, egui::Context::default(), Duration::from_millis(300), &watch);
            started.elapsed()
        });
        // Blank lines and partial frames, one byte every 20 ms: each read succeeds long before
        // the idle timeout.
        let mut stream = TcpStream::connect(addr).unwrap();
        for i in 0..150 {
            let byte: &[u8] = if i % 5 == 0 { b"\n" } else { b" " };
            if stream.write_all(byte).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let served = server.join().unwrap();
        assert!(served < Duration::from_secs(3), "the trickle did not extend the deadline: {served:?}");
        assert!(rx.try_recv().is_err(), "nothing was dispatched");
    }

    #[test]
    fn oversized_reply_preserves_framing_id_and_the_next_control_request() {
        use photocraft_automation::budgets::MAX_RESPONSE_BYTES;
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = ControlWatch::new();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &watch);
        });
        let handler = std::thread::spawn(move || {
            let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(first.method, "test.large");
            assert!(first.deadline.is_some_and(|deadline| deadline > std::time::Instant::now()), "native requests carry their reply deadline");
            first.reply.send(json!({"ok": true, "result": "x".repeat(MAX_RESPONSE_BYTES)})).unwrap();
            let second = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(second.method, "test.small");
            second.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "test.large"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let rejected: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(rejected["id"], 2);
        assert_eq!(rejected["ok"], false);
        assert!(rejected["error"].as_str().unwrap().contains("operation may have completed"));
        writeln!(stream, "{}", json!({"id": 3, "method": "test.small"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let accepted: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(accepted["id"], 3);
        assert_eq!(accepted["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn bad_json_reply_carries_a_null_id_and_the_connection_keeps_serving() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = ControlWatch::new();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &watch);
        });
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.after");
            req.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{{not json}}").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let bad: Value = serde_json::from_str(&line).unwrap();
        let bad = bad.as_object().unwrap();
        assert_eq!(bad.get("id"), Some(&Value::Null), "bad JSON reply must carry `id: null`: {line}");
        assert_eq!(bad.get("ok"), Some(&Value::Bool(false)));
        assert!(bad.get("error").and_then(Value::as_str).unwrap().starts_with("bad JSON: "));
        writeln!(stream, "{}", json!({"id": 7, "method": "test.after"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let after: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(after["id"], 7);
        assert_eq!(after["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn repeat_auth_reports_already_authenticated_without_dispatching() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = ControlWatch::new();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &watch);
        });
        let handler = std::thread::spawn(move || {
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.after", "a repeat auth must not reach the UI dispatcher");
            req.reply.send(json!({"ok": true, "result": "still serving"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        for id in ["auth", "again"] {
            writeln!(stream, "{}", json!({"id": id, "method": "auth", "params": {"token": TOKEN}})).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
        }
        let again: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(again["id"], "again");
        assert_eq!(again["ok"], false);
        assert_eq!(again["error"], "already authenticated");
        writeln!(stream, "{}", json!({"id": 9, "method": "test.after"})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let after: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(after["id"], 9);
        assert_eq!(after["result"], "still serving");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Generous against the stall checkpoint (1s) and the reply deadline (60s): a healthy
    /// reply lands within a second or two, a broken one never.
    const READ_TIMEOUT: Duration = Duration::from_secs(30);

    fn authed_client(addr: std::net::SocketAddr) -> (TcpStream, BufReader<TcpStream>) {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(READ_TIMEOUT)).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": "auth", "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true, "auth failed: {line}");
        (stream, reader)
    }

    fn send_and_read(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>, request: Value) -> Value {
        writeln!(stream, "{request}").unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"))
    }

    #[test]
    fn health_is_answered_without_queueing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = Arc::new(ControlWatch::new());
        watch.note_logic();
        let server_watch = Arc::clone(&watch);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &server_watch);
        });
        let (mut stream, mut reader) = authed_client(addr);
        let reply = send_and_read(&mut stream, &mut reader, json!({"id": 2, "method": "health", "params": {"extra": true}}));
        assert_eq!(reply["id"], 2);
        assert_eq!(reply["ok"], true);
        for key in ["lastLogicAgoMs", "lastDrawnAgoMs", "logicCount", "drawnCount"] {
            assert!(reply["result"].as_object().is_some_and(|o| o.contains_key(key)), "health result lacks `{key}`: {reply}");
        }
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "health must never reach the control channel");
        drop(reader);
        drop(stream);
        server.join().unwrap();
    }

    #[test]
    fn wedged_snapshot_getter_serves_the_last_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = Arc::new(ControlWatch::new());
        watch.note_snapshot("ui.inspect", &json!({"ok": true, "result": {"tool": "Move"}}));
        let server_watch = Arc::clone(&watch);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &server_watch);
        });
        // A wedged frame loop never answers: hold the request until the client has its reply,
        // so the reply sender is not dropped while the transport is still waiting.
        let (release_tx, release_rx) = channel::<()>();
        let handler = std::thread::spawn(move || {
            let Ok(req) = rx.recv_timeout(Duration::from_secs(5)) else { return };
            assert_eq!(req.method, "ui.inspect");
            let _ = release_rx.recv_timeout(READ_TIMEOUT);
        });
        let (mut stream, mut reader) = authed_client(addr);
        let sent = Instant::now();
        let reply = send_and_read(&mut stream, &mut reader, json!({"id": 3, "method": "ui.inspect"}));
        let elapsed = sent.elapsed();
        assert!(elapsed < READ_TIMEOUT, "served after {elapsed:?}, not from the snapshot");
        assert_eq!(reply["id"], 3);
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["result"]["tool"], "Move");
        assert_eq!(reply["result"]["frameStalled"], true);
        assert!(reply["result"]["snapshotAgeMs"].as_u64().is_some(), "no snapshot age: {reply}");
        drop(reader);
        drop(stream);
        let _ = release_tx.send(());
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn wedged_getter_without_a_snapshot_fails_fast() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = Arc::new(ControlWatch::new());
        let server_watch = Arc::clone(&watch);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &server_watch);
        });
        let (release_tx, release_rx) = channel::<()>();
        let handler = std::thread::spawn(move || {
            let Ok(req) = rx.recv_timeout(Duration::from_secs(5)) else { return };
            assert_eq!(req.method, "jobs.list");
            let _ = release_rx.recv_timeout(READ_TIMEOUT);
        });
        let (mut stream, mut reader) = authed_client(addr);
        let sent = Instant::now();
        let reply = send_and_read(&mut stream, &mut reader, json!({"id": 4, "method": "jobs.list"}));
        let elapsed = sent.elapsed();
        assert!(elapsed < READ_TIMEOUT, "failed after {elapsed:?}, not fast");
        assert_eq!(reply["id"], 4);
        assert_eq!(reply["ok"], false);
        assert!(reply["error"].as_str().is_some_and(|e| e.contains("not drawing frames")), "{reply}");
        drop(reader);
        drop(stream);
        let _ = release_tx.send(());
        handler.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn a_live_frame_loop_keeps_waiting_for_a_slow_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let watch = Arc::new(ControlWatch::new());
        // The snapshot exists, but the live counter must win: the real reply, not the stale one.
        watch.note_snapshot("ui.inspect", &json!({"ok": true, "result": {"tool": "Stale"}}));
        let server_watch = Arc::clone(&watch);
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default(), &server_watch);
        });
        let handler_watch = Arc::clone(&watch);
        let handler = std::thread::spawn(move || {
            let Ok(req) = rx.recv_timeout(Duration::from_secs(5)) else { return };
            assert_eq!(req.method, "ui.inspect");
            // The frame loop ran after the request was queued; its reply is only slow.
            handler_watch.note_logic();
            std::thread::sleep(STALL_CHECK + Duration::from_secs(1));
            let _ = req.reply.send(json!({"ok": true, "result": "slow but live"}));
        });
        let (mut stream, mut reader) = authed_client(addr);
        let reply = send_and_read(&mut stream, &mut reader, json!({"id": 5, "method": "ui.inspect"}));
        assert_eq!(reply["id"], 5);
        assert_eq!(reply["result"], "slow but live");
        assert!(reply.get("frameStalled").is_none(), "a live loop must not be reported stalled: {reply}");
        drop(reader);
        drop(stream);
        handler.join().unwrap();
        server.join().unwrap();
    }
}
