//! A blocking client for the app's JSON-lines control channel (`docs/control-protocol.md`): one request
//! per line, one reply per line. The app closes a connection that sits idle, so callers reconnect after
//! any error and keep a long-lived connection busy.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{Value, json};

pub struct Control {
    reader: BufReader<TcpStream>,
    next_id: u64,
}

impl Control {
    /// Connect to the app's loopback control port and authenticate. `timeout` bounds the connect and every
    /// later read and write, so a hung app is an error here rather than a hung test.
    pub fn connect(port: u16, token: &str, timeout: Duration) -> Result<Self, String> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect to {addr}: {e}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .and_then(|()| stream.set_nodelay(true))
            .map_err(|e| format!("socket setup: {e}"))?;
        let mut control = Control { reader: BufReader::new(stream), next_id: 0 };
        control.exchange(json!("auth"), "auth", json!({"token": token}))?;
        Ok(control)
    }

    /// Call a control method and return its `result`, or the app's error text.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = json!(self.next_id);
        self.exchange(id, method, params)
    }

    /// One request and its reply. Replies are matched by `id`, so one that answers a different request (a late
    /// reply after a timeout, say) is an error and not mistaken for this one's answer.
    fn exchange(&mut self, id: Value, method: &str, params: Value) -> Result<Value, String> {
        let request = json!({"id": id, "method": method, "params": params}).to_string();
        let stream = self.reader.get_mut();
        stream.write_all(request.as_bytes()).and_then(|()| stream.write_all(b"\n")).map_err(|e| format!("{method}: send: {e}"))?;
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).map_err(|e| match e.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => format!("{method}: no reply within the timeout"),
            _ => format!("{method}: {e}"),
        })?;
        if read == 0 {
            return Err(format!("{method}: the app closed the connection"));
        }
        let reply: Value = serde_json::from_str(&line).map_err(|e| format!("{method}: unreadable reply: {e}"))?;
        if reply["ok"] != json!(true) {
            let error = reply["error"].as_str().map_or_else(|| reply["error"].to_string(), str::to_owned);
            return Err(format!("{method}: {error}"));
        }
        if reply["id"] != id {
            return Err(format!("{method}: the reply answers request {}, not {id}", reply["id"]));
        }
        Ok(reply["result"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    /// A loopback server that reads one line per `replies` entry and answers with it (`None`: stays silent).
    fn serve(replies: Vec<Option<&'static str>>) -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for reply in replies {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                match reply {
                    Some(text) => writeln!(stream, "{text}").unwrap(),
                    None => {
                        // Never answers: wait for the client to give up and hang up.
                        let mut rest = Vec::new();
                        let _ = stream.read_to_end(&mut rest);
                        return;
                    }
                }
            }
        });
        port
    }

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn calls_return_results_and_errors_and_a_closed_connection() {
        let port = serve(vec![
            Some(r#"{"id":"auth","ok":true,"result":{}}"#),
            Some(r#"{"id":1,"ok":true,"result":{"frame":7}}"#),
            Some(r#"{"id":2,"ok":false,"error":"unknown tool `nope`"}"#),
            Some(r#"{"id":99,"ok":true,"result":{}}"#),
        ]);
        let mut control = Control::connect(port, "token", SECOND).unwrap();
        assert_eq!(control.call("ui.inspect", json!({})).unwrap()["frame"], 7);
        assert_eq!(control.call("nope", json!({})).unwrap_err(), "nope: unknown tool `nope`");
        let mismatch = control.call("ui.inspect", json!({})).unwrap_err();
        assert!(mismatch.contains("answers request 99, not 3"), "{mismatch}");
        // The server has hung up: a clean close or a reset, depending on the platform, but an error naming the call.
        let closed = control.call("ui.inspect", json!({})).unwrap_err();
        assert!(closed.starts_with("ui.inspect:"), "{closed}");
    }

    #[test]
    fn a_failed_authentication_is_an_error() {
        let port = serve(vec![Some(r#"{"id":"auth","ok":false,"error":"bad token"}"#)]);
        let error = Control::connect(port, "wrong", SECOND).err().unwrap();
        assert!(error.contains("bad token"), "{error}");
    }

    #[test]
    fn a_silent_app_times_out_instead_of_hanging_the_test() {
        let port = serve(vec![Some(r#"{"id":"auth","ok":true,"result":{}}"#), None]);
        let mut control = Control::connect(port, "token", Duration::from_millis(300)).unwrap();
        let started = std::time::Instant::now();
        let error = control.call("ui.inspect", json!({})).unwrap_err();
        assert!(error.contains("no reply within the timeout"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn nothing_listening_is_an_error() {
        // A port that was just free: bind, note it, drop.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        assert!(Control::connect(port, "token", Duration::from_millis(300)).is_err());
    }
}
