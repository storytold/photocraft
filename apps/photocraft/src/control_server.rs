//! Localhost JSON-lines control server (one request per line, one reply per line).
//! Loopback only. This is the transport the MCP server will wrap.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use photocraft_automation::budgets::write_reply;
use photocraft_automation::security::{
    AUTH_DEADLINE, AuthDeadlineStream, ConnectionLimiter, LineRead, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply, configure_stream,
    read_bounded_line, repeat_auth_reply,
};
use photocraft_ui_egui::ControlRequest;
use photocraft_ui_egui::control::REPLY_DEADLINE;
use serde_json::{Value, json};

/// Start serving a listener bound before the window is opened.
pub fn start(listener: TcpListener, token: String, ctx: egui::Context) -> Receiver<ControlRequest> {
    let (tx, rx) = channel::<ControlRequest>();
    let port = listener.local_addr().map_or(0, |a| a.port());
    eprintln!("photocraft: control server listening on {}", listening_addr(&listener, port));
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
            std::thread::spawn(move || {
                let _permit = permit;
                serve(stream, &token, tx, ctx);
            });
        }
    });
    rx
}

/// Address clients can use, including the OS-selected port when binding port zero (#6427).
fn listening_addr(listener: &TcpListener, requested_port: u16) -> String {
    listener.local_addr().map_or_else(|_| format!("127.0.0.1:{requested_port}"), |addr| addr.to_string())
}

/// How long `events.poll {"waitMs"}` may hold a reply for events at most.
const MAX_WAIT: Duration = Duration::from_secs(25);
/// How often a waiting poll asks the app again.
const WAIT_STEP: Duration = Duration::from_millis(250);
/// How often a subscribed connection checks for pushed events between requests.
const PUSH_STEP: Duration = Duration::from_millis(100);

/// Ask the app once; `None` when it went away.
fn ask(
    tx: &Sender<ControlRequest>,
    ctx: &egui::Context,
    mut req: ControlRequest,
    rrx: Receiver<photocraft_ui_egui::control::ControlResponse>,
    deadline: Instant,
) -> Option<Value> {
    req.deadline = Some(deadline);
    let dispatch = req.dispatch.clone();
    if tx.send(req).is_err() {
        return None;
    }
    ctx.request_repaint();
    Some(rrx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| json!({"ok": false, "error": dispatch.give_up()})))
}

/// One writer drains a subscriber queue while the reader waits for complete request lines.
/// All outbound lines share the same lock so a push cannot split a control reply.
struct PushWriter {
    stop: Sender<()>,
    ready: Sender<()>,
    done: Receiver<()>,
    interrupt: TcpStream,
    handle: std::thread::JoinHandle<()>,
}

impl PushWriter {
    fn start(out: Arc<Mutex<TcpStream>>, push: Receiver<String>) -> std::io::Result<Self> {
        let (stop_tx, stop_rx) = channel();
        let (ready_tx, ready_rx) = channel();
        let (done_tx, done_rx) = channel();
        let interrupt = out.lock().unwrap_or_else(PoisonError::into_inner).try_clone()?;
        let handle = std::thread::Builder::new().name("photocraft-event-writer".into()).spawn(move || {
            // The subscribe reply must be the first line before any pushed event.
            if ready_rx.recv().is_err() {
                let _ = done_tx.send(());
                return;
            }
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                match push.recv_timeout(PUSH_STEP) {
                    Ok(line) => {
                        if stop_rx.try_recv().is_ok() {
                            break;
                        }
                        let mut stream = out.lock().unwrap_or_else(PoisonError::into_inner);
                        if writeln!(stream, "{line}").is_err() || stream.flush().is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = done_tx.send(());
        })?;
        Ok(Self { stop: stop_tx, ready: ready_tx, done: done_rx, interrupt, handle })
    }

    fn begin(&self) {
        let _ = self.ready.send(());
    }

    fn stop(self) {
        let _ = self.stop.send(());
        // Also release a writer that has not yet sent its subscribe reply.
        let _ = self.ready.send(());
        // A healthy subscriber keeps its connection for the unsubscribe reply. Only close a
        // stalled writer's socket after its normal receive interval has had time to finish.
        if self.done.recv_timeout(PUSH_STEP * 2).is_err() {
            let _ = self.interrupt.shutdown(std::net::Shutdown::Both);
        }
        if self.handle.join().is_err() {
            eprintln!("photocraft: event writer stopped unexpectedly");
        }
    }
}

fn stop_push_writer(push: &mut Option<PushWriter>) {
    if let Some(writer) = push.take() {
        writer.stop();
    }
}

fn send_reply(out: &Mutex<TcpStream>, reply: &Value) -> bool {
    let mut stream = out.lock().unwrap_or_else(PoisonError::into_inner);
    write_reply(&mut *stream, reply).is_ok() && stream.flush().is_ok()
}

fn serve(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context) {
    serve_with_auth_deadline(stream, token, tx, ctx, AUTH_DEADLINE);
}

/// [`serve`] for a connection that must complete `auth` within `auth_deadline` of connecting,
/// however much it sends meanwhile (#4965); after that only the idle timeout applies.
fn serve_with_auth_deadline(stream: TcpStream, token: &str, tx: Sender<ControlRequest>, ctx: egui::Context, auth_deadline: Duration) {
    if configure_stream(&stream).is_err() {
        return;
    }
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(AuthDeadlineStream::new(read, auth_deadline));
    let out = Arc::new(Mutex::new(stream));
    let mut line = String::new();
    let mut authenticated = false;
    let mut push: Option<PushWriter> = None;
    'conn: loop {
        match read_bounded_line(&mut reader, &mut line) {
            Ok(LineRead::Eof) | Err(_) => break,
            Ok(LineRead::TooLong) => {
                let reply = json!({
                    "id": null,
                    "ok": false,
                    "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                });
                let _ = send_reply(&out, &reply);
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
            if !send_reply(&out, &reply) || !authenticated {
                break;
            }
            continue;
        }
        if let Some(reply) = repeat_auth_reply(&line) {
            if !send_reply(&out, &reply) {
                break;
            }
            continue;
        }
        let mut pending_push: Option<PushWriter> = None;
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                let method = msg.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = msg.get("params").cloned().unwrap_or(json!({}));
                let deadline = Instant::now() + REPLY_DEADLINE;
                let mut r = match method.as_str() {
                    "events.subscribe" => {
                        let (ptx, prx) = sync_channel::<String>(photocraft_ui_egui::control::PUSH_QUEUE);
                        let (mut req, rrx) = ControlRequest::new(method, params);
                        req.push = Some(ptx);
                        let Some(mut r) = ask(&tx, &ctx, req, rrx, deadline) else { break };
                        if r.get("ok").and_then(Value::as_bool) == Some(true) {
                            stop_push_writer(&mut push);
                            if reader.get_mut().subscription_idle(false).is_err() {
                                break 'conn;
                            }
                            match PushWriter::start(Arc::clone(&out), prx) {
                                Ok(writer) => pending_push = Some(writer),
                                Err(e) => r = json!({"ok": false, "error": format!("could not start event writer: {e}")}),
                            }
                        }
                        r
                    }
                    "events.unsubscribe" => {
                        // Dropping the receiver ends the app's subscription. The reader has
                        // retained its normal timeout, so there is no socket reset to undo.
                        stop_push_writer(&mut push);
                        if reader.get_mut().subscription_idle(false).is_err() {
                            break 'conn;
                        }
                        let (req, rrx) = ControlRequest::new(method, params);
                        let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break };
                        r
                    }
                    "events.poll" => {
                        // `waitMs`: hold the reply until an event arrives or the wait is over,
                        // asking the app again every few frames (long poll).
                        let wait = if params.get("since").is_some_and(|v| !v.is_null()) {
                            params.get("waitMs").and_then(Value::as_u64).map(|ms| Duration::from_millis(ms).min(MAX_WAIT)).unwrap_or(Duration::ZERO)
                        } else {
                            Duration::ZERO
                        };
                        let until = Instant::now() + wait;
                        let mut p = params.clone();
                        if let Some(o) = p.as_object_mut() {
                            o.remove("waitMs");
                        }
                        loop {
                            let (req, rrx) = ControlRequest::new(method.clone(), p.clone());
                            let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break 'conn };
                            let empty = r.get("ok").and_then(Value::as_bool) == Some(true)
                                && r["result"]["events"].as_array().is_some_and(Vec::is_empty)
                                && r["result"]["gap"].as_bool() != Some(true);
                            if !empty || Instant::now() >= until {
                                break r;
                            }
                            std::thread::sleep(WAIT_STEP);
                        }
                    }
                    _ => {
                        let (req, rrx) = ControlRequest::new(method, params);
                        let Some(r) = ask(&tx, &ctx, req, rrx, deadline) else { break };
                        r
                    }
                };
                if let Some(o) = r.as_object_mut() {
                    o.insert("id".into(), id);
                }
                r
            }
            Err(e) => json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
        };
        if !send_reply(&out, &reply) {
            if let Some(writer) = pending_push {
                writer.stop();
            }
            break;
        }
        if let Some(writer) = pending_push {
            if reader.get_mut().subscription_idle(true).is_err() {
                writer.stop();
                break;
            }
            writer.begin();
            push = Some(writer);
        }
    }
    stop_push_writer(&mut push);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    #[test]
    fn stopping_a_stalled_push_writer_does_not_wait_for_socket_timeout() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        server.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        let out = Arc::new(Mutex::new(server));
        let (tx, rx) = sync_channel(1);
        let writer = PushWriter::start(Arc::clone(&out), rx).unwrap();
        writer.begin();
        tx.send("x".repeat(8 * 1024 * 1024)).unwrap();
        let start = Instant::now();
        while out.try_lock().is_ok() {
            assert!(start.elapsed() < Duration::from_secs(5), "writer never began sending");
            std::thread::sleep(Duration::from_millis(1));
        }
        let stop = Instant::now();
        writer.stop();
        assert!(stop.elapsed() < Duration::from_secs(1), "stop waited for a blocked socket write: {:?}", stop.elapsed());
    }

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

    #[test]
    fn a_prebound_listener_authenticates_and_dispatches_requests() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let requests = start(listener, TOKEN.into(), egui::Context::default());
        let mut stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut replies = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        let mut line = String::new();
        replies.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["result"]["authenticated"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "test.bound"})).unwrap();
        let request = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.method, "test.bound");
        request.reply.send(json!({"ok": true, "result": "served"})).unwrap();
        line.clear();
        replies.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], 2);
        assert_eq!(reply["result"], "served");
    }

    /// #4965: a connection that keeps sending without authenticating is closed at the deadline
    /// (releasing its connection permit), and nothing it sent is dispatched.
    #[test]
    fn an_unauthenticated_connection_is_closed_at_the_deadline_whatever_it_sends() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let started = Instant::now();
            serve_with_auth_deadline(stream, "unused", tx, egui::Context::default(), Duration::from_millis(300));
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

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A subscribed connection receives the lines the app pushes between its requests, and
    /// `events.poll` with `waitMs` holds its reply until there is something to return (#4939).
    #[test]
    fn subscribed_connections_receive_pushed_events_and_polls_can_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        // A stand-in for the app: answers requests, keeps the subscriber's queue, pushes into it.
        let handler = std::thread::spawn(move || {
            let mut push: Option<std::sync::mpsc::SyncSender<String>> = None;
            let mut polls = 0;
            while let Ok(req) = rx.recv_timeout(Duration::from_secs(10)) {
                match req.method.as_str() {
                    "events.subscribe" => {
                        push = req.push.clone();
                        assert!(push.is_some(), "the transport hands the app a queue");
                        req.reply.send(json!({"ok": true, "result": {"subscribed": true, "next": 0}})).unwrap();
                        // Pushed after the reply, as the app does on later frames.
                        for i in 1..=3 {
                            push.as_ref().unwrap().try_send(json!({"seq": i, "event": "document.changed"}).to_string()).unwrap();
                        }
                    }
                    "events.poll" => {
                        polls += 1;
                        assert!(req.params.get("waitMs").is_none(), "the wait stays in the transport");
                        // Empty twice, then an event: the transport keeps asking meanwhile.
                        let gap = req.params.get("since") == Some(&json!(1000));
                        let events =
                            if gap || req.params.get("since").is_none() || polls < 3 { json!([]) } else { json!([{"seq": 9, "event": "tool.changed"}]) };
                        req.reply.send(json!({"ok": true, "result": {"events": events, "next": 9, "gap": gap}})).unwrap();
                    }
                    "events.unsubscribe" => {
                        req.reply.send(json!({"ok": true, "result": {"subscribed": false}})).unwrap();
                        assert!(push.take().is_some());
                    }
                    other => req.reply.send(json!({"ok": true, "result": other})).unwrap(),
                }
            }
            polls
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        let mut read = |reader: &mut BufReader<TcpStream>| -> Value {
            line.clear();
            reader.read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        };
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        assert_eq!(read(&mut reader)["ok"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "events.subscribe", "params": {}})).unwrap();
        let r = read(&mut reader);
        assert_eq!((r["id"].as_i64(), r["ok"].as_bool()), (Some(2), Some(true)), "{r}");
        // The three pushed lines arrive without a request, each a JSON object without an id.
        for i in 1..=3 {
            let e = read(&mut reader);
            assert_eq!(e["seq"], i, "{e}");
            assert!(e.get("id").is_none());
        }
        // A long poll: the transport re-asks the app until there is an event.
        let t = Instant::now();
        writeln!(stream, "{}", json!({"id": 3, "method": "events.poll", "params": {"since": 0, "waitMs": 5000}})).unwrap();
        let r = read(&mut reader);
        assert_eq!(r["id"], 3);
        assert_eq!(r["result"]["events"][0]["seq"], 9, "{r}");
        assert!(t.elapsed() >= WAIT_STEP * 2 && t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
        // Without a cursor, polling only asks for the current position: waiting cannot reveal
        // any events, so it must answer after one app dispatch even with waitMs set.
        let t = Instant::now();
        writeln!(stream, "{}", json!({"id": 4, "method": "events.poll", "params": {"waitMs": 700}})).unwrap();
        let r = read(&mut reader);
        assert_eq!(r["result"]["events"], json!([]));
        assert!(t.elapsed() < Duration::from_millis(400), "cursorless poll waited {:?}", t.elapsed());
        let t = Instant::now();
        writeln!(stream, "{}", json!({"id": 7, "method": "events.poll", "params": {"since": 1000, "waitMs": 700}})).unwrap();
        let r = read(&mut reader);
        assert_eq!(r["result"]["gap"], true);
        assert!(t.elapsed() < Duration::from_millis(400), "gap reply waited {:?}", t.elapsed());
        writeln!(stream, "{}", json!({"id": 5, "method": "events.unsubscribe"})).unwrap();
        assert_eq!(read(&mut reader)["result"]["subscribed"], false);
        writeln!(stream, "{}", json!({"id": 6, "method": "test.after"})).unwrap();
        assert_eq!(read(&mut reader)["result"], "test.after", "the connection keeps serving requests");
        drop(stream);
        drop(reader);
        server.join().unwrap();
        assert_eq!(handler.join().unwrap(), 5, "two empty polls, an event, one cursorless poll and one gap poll");
    }

    /// #5154: pushes continue while a subscribed client sends one request in pieces.
    #[test]
    fn subscribed_slow_request_keeps_its_prefix_across_push_tick() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
        });
        let handler = std::thread::spawn(move || {
            let sub = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(sub.method, "events.subscribe");
            let push = sub.push.as_ref().unwrap().clone();
            sub.reply.send(json!({"ok": true, "result": {"subscribed": true}})).unwrap();
            std::thread::sleep(PUSH_STEP * 2);
            push.try_send(json!({"seq": 1, "event": "document.changed"}).to_string()).unwrap();
            let req = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(req.method, "test.split");
            req.reply.send(json!({"ok": true, "result": "whole request"})).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        writeln!(stream, "{}", json!({"id": 1, "method": "auth", "params": {"token": TOKEN}})).unwrap();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["ok"], true);
        writeln!(stream, "{}", json!({"id": 2, "method": "events.subscribe", "params": {}})).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], 2);
        stream.write_all(b"{\"id\":3,\"method\":\"test.").unwrap();
        // The push must arrive while the request is incomplete and after the old read tick.
        line.clear();
        reader.read_line(&mut line).unwrap();
        let pushed: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(pushed["seq"], 1, "{pushed}");
        stream.write_all(b"split\"}\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], 3, "{reply}");
        assert_eq!(reply["result"], "whole request");
        drop(stream);
        drop(reader);
        server.join().unwrap();
        handler.join().unwrap();
    }

    #[test]
    fn oversized_reply_preserves_framing_id_and_the_next_control_request() {
        use photocraft_automation::budgets::MAX_RESPONSE_BYTES;
        const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = channel::<ControlRequest>();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
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
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
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
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve(stream, TOKEN, tx, egui::Context::default());
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
}
