//! An in-process S3 stand-in: path-style GET, PUT (with `If-None-Match: *` and `If-Match`) and
//! DELETE over HTTP/1.1 keep-alive, recording every request.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use photocraft_s3::{Credentials, S3Config};

pub const BUCKET: &str = "designs";

#[derive(Debug, Clone)]
pub struct Logged {
    pub method: String,
    pub key: String,
    /// `if-none-match` or `if-match`, as sent.
    pub condition: Option<(String, String)>,
    pub authorization: String,
}

#[derive(Default)]
pub struct State {
    /// Key → (bytes, ETag).
    pub objects: BTreeMap<String, (Vec<u8>, String)>,
    pub log: Vec<Logged>,
    next_etag: u64,
    /// PUTs to keys starting with this answer 500.
    pub fail_puts_under: Option<String>,
}

pub struct FakeS3 {
    pub endpoint: String,
    pub state: Arc<Mutex<State>>,
}

impl FakeS3 {
    pub fn start() -> FakeS3 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                std::thread::spawn(move || serve(stream, &state));
            }
        });
        FakeS3 { endpoint, state }
    }

    pub fn config(&self) -> S3Config {
        S3Config {
            endpoint: self.endpoint.clone(),
            region: "us-east-1".into(),
            bucket: BUCKET.into(),
            credentials: Credentials { access_key_id: "test-key".into(), secret_access_key: "test-secret".into(), session_token: None },
            virtual_hosted: false,
        }
    }

    pub fn keys(&self) -> Vec<String> {
        self.state.lock().unwrap().objects.keys().cloned().collect()
    }

    pub fn object(&self, key: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().objects.get(key).map(|(b, _)| b.clone())
    }

    /// Requests since the last call.
    pub fn take_log(&self) -> Vec<Logged> {
        std::mem::take(&mut self.state.lock().unwrap().log)
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>) {
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        let length: usize = headers.get("content-length").map_or(0, |v| v.parse().unwrap());
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let mut parts = request_line.split_whitespace();
        let (method, path) = (parts.next().unwrap().to_string(), parts.next().unwrap());
        let key = path.strip_prefix(&format!("/{BUCKET}/")).unwrap().to_string();
        let (status, etag, reply) = handle(state, &method, &key, &headers, body);
        let mut head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\n", reply.len());
        if let Some(etag) = etag {
            head.push_str(&format!("ETag: {etag}\r\n"));
        }
        head.push_str("\r\n");
        writer.write_all(head.as_bytes()).unwrap();
        writer.write_all(&reply).unwrap();
    }
}

fn error(code: &str) -> Vec<u8> {
    format!("<?xml version=\"1.0\"?><Error><Code>{code}</Code><Message>fake S3</Message></Error>").into_bytes()
}

fn handle(state: &Mutex<State>, method: &str, key: &str, headers: &HashMap<String, String>, body: Vec<u8>) -> (u16, Option<String>, Vec<u8>) {
    let mut s = state.lock().unwrap();
    let condition = ["if-none-match", "if-match"].into_iter().find_map(|h| headers.get(h).map(|v| (h.to_string(), v.clone())));
    let authorization = headers.get("authorization").cloned().unwrap_or_default();
    s.log.push(Logged { method: method.into(), key: key.into(), condition: condition.clone(), authorization });
    match method {
        "GET" => match s.objects.get(key) {
            Some((bytes, etag)) => (200, Some(etag.clone()), bytes.clone()),
            None => (404, None, error("NoSuchKey")),
        },
        "DELETE" => {
            s.objects.remove(key);
            (204, None, Vec::new())
        }
        "PUT" => {
            if s.fail_puts_under.as_deref().is_some_and(|prefix| key.starts_with(prefix)) {
                return (500, None, error("InternalError"));
            }
            let current = s.objects.get(key).map(|(_, etag)| etag.clone());
            let holds = match &condition {
                Some((h, v)) if h == "if-none-match" && v == "*" => current.is_none(),
                Some((h, v)) if h == "if-match" => current.as_deref() == Some(v.as_str()),
                _ => true,
            };
            if !holds {
                return (412, None, error("PreconditionFailed"));
            }
            s.next_etag += 1;
            let etag = format!("\"{}\"", s.next_etag);
            s.objects.insert(key.to_string(), (body, etag.clone()));
            (200, Some(etag), Vec::new())
        }
        _ => (405, None, error("MethodNotAllowed")),
    }
}
