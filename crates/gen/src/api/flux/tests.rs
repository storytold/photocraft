use super::*;
use crate::api::{EditRequest, ImageProvider, Profile, STANDARD, cancellable};
use base64::Engine as _;
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

fn mock(responses: Vec<(u16, Vec<u8>)>) -> (String, std::thread::JoinHandle<Vec<String>>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let address = base.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let thread = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (code, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < deadline, "missing expected FLUX request");
                std::thread::sleep(Duration::from_millis(2));
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|n| n.trim().parse::<usize>().ok()))
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            seen.fetch_add(1, Ordering::Release);
            let body = if body.starts_with(b"{") { String::from_utf8(body).unwrap().replace("@BASE@", &address).into_bytes() } else { body };
            write!(stream, "HTTP/1.1 {code} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            let _ = stream.write_all(&body);
        }
        requests
    });
    (base, thread, count)
}

fn task() -> (u16, Vec<u8>) {
    (200, br#"{"id":"test","polling_url":"@BASE@/poll?id=test"}"#.to_vec())
}
fn pending() -> (u16, Vec<u8>) {
    (200, br#"{"status":"Pending"}"#.to_vec())
}
fn provider(base: &str) -> ApiProvider {
    ApiProvider::new(Profile { endpoint: format!("{base}/v1/{{model}}"), ..crate::api_config::preset("flux").unwrap() }, "private-flux-key".into()).unwrap()
}

#[test]
fn submits_once_polls_then_downloads_without_credentials() {
    let image = super::super::tests::png();
    let (base, server, _) =
        mock(vec![task(), pending(), (200, br#"{"status":"Ready","result":{"sample":"@BASE@/image?signature=private"}}"#.to_vec()), (200, image.clone())]);
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("input.png");
    std::fs::write(&input, &image).unwrap();
    let result = provider(&base).edit(&EditRequest { image: &input, prompt: "make yellow", workspace: work.path() }, &|| false).unwrap();
    assert_eq!(std::fs::read(result.saved_path).unwrap(), image);
    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("POST /v1/flux-2-pro "));
    let body: Value = serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["input_image"], STANDARD.encode(image));
    assert_eq!(body["output_format"], "png");
    assert!(body["prompt"].as_str().unwrap().contains("make yellow"));
    for request in requests.iter().take(3) {
        assert!(request.contains("x-key: private-flux-key"));
    }
    assert!(!requests[3].contains("private-flux-key"));
    assert!(!requests[3].contains("x-key:"));
    assert_eq!(requests.iter().filter(|r| r.starts_with("POST ")).count(), 1);
}

#[test]
fn failure_moderation_unknown_status_and_http_error_are_secret_free() {
    for status in ["Failed", "Error", "Request Moderated", "Content Moderated", "Unknown"] {
        let (base, server, _) = mock(vec![task(), (200, json!({"status":status,"error":"private-flux-key"}).to_string().into_bytes())]);
        let result = cancellable(provider(&base).request(json!({})), &|| false).unwrap_err().to_string();
        assert!(!result.contains("private-flux-key"));
        assert_eq!(server.join().unwrap().len(), 2);
    }
    let (base, server, _) = mock(vec![(429, b"private-flux-key".to_vec())]);
    assert!(!cancellable(provider(&base).request(json!({})), &|| false).unwrap_err().to_string().contains("private-flux-key"));
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn cancellation_and_total_deadline_stop_polling() {
    let (base, server, count) = mock(vec![task(), pending()]);
    assert!(matches!(cancellable(provider(&base).request(json!({})), &|| count.load(Ordering::Acquire) >= 2), Err(Error::Cancelled)));
    server.join().unwrap();
    let (base, server, _) = mock(vec![task()]);
    let mut service = provider(&base);
    service.timeout = Duration::from_millis(150);
    assert!(matches!(cancellable(service.request(json!({})), &|| false), Err(Error::Timeout)));
    server.join().unwrap();
}

#[test]
fn redirects_and_oversized_metadata_are_rejected_without_download_or_resubmit() {
    for response in [(302, b"private-flux-key".to_vec()), (200, vec![b'x'; MAX_METADATA + 1])] {
        let (base, server, _) = mock(vec![response]);
        assert!(cancellable(provider(&base).request(json!({})), &|| false).is_err());
        assert_eq!(server.join().unwrap().len(), 1);
    }
    let (base, server, _) =
        mock(vec![task(), (200, br#"{"status":"Ready","result":{"sample":"@BASE@/image"}}"#.to_vec()), (302, b"private-flux-key".to_vec())]);
    assert!(cancellable(provider(&base).request(json!({})), &|| false).is_err());
    assert_eq!(server.join().unwrap().len(), 3);
}

#[test]
fn result_addresses_cannot_leak_key_or_redirect_to_unrelated_hosts() {
    let origin = Url::parse("https://api.bfl.ai/v1/flux-2-pro").unwrap();
    assert!(checked_url("https://api.eu.bfl.ai/v1/get_result?id=1", &origin, true).is_ok());
    assert!(checked_url("https://delivery-eu1.bfl.ai/image?signature=1", &origin, false).is_ok());
    for value in [
        "https://evil.example/poll",
        "http://api.bfl.ai/poll",
        "https://user:pass@api.bfl.ai/poll",
        "http://127.0.0.1/private",
        "file:///private",
        "https://delivery-eu1.bfl.ai/image",
    ] {
        assert!(checked_url(value, &origin, true).is_err(), "{value}");
    }
    let custom = Url::parse("https://custom.example/v1/model").unwrap();
    assert!(checked_url("https://api.bfl.ai/poll", &custom, true).is_err());
    assert!(checked_url("https://custom.example/image?sig=1", &custom, false).is_ok());
}
