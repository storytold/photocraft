//! The client against an in-process fake CUPS: TCP and Unix socket, content-length and chunked
//! replies, every operation, error statuses and broken replies.

use super::*;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;

/// What the fake received: op, attributes, document.
type Seen = (u16, Vec<Attribute>, Vec<u8>);

/// A fake server answering one request with `reply_body` (IPP bytes), chunked or not, under an
/// HTTP status. Returns the endpoint and a channel with what it received.
fn fake(reply: Vec<u8>, chunked: bool, http_status: u16) -> (Endpoint, mpsc::Receiver<Seen>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        serve(Stream::Tcp(stream), reply, chunked, http_status, &tx);
    });
    (Endpoint::Tcp("127.0.0.1".into(), port), rx)
}

fn serve(stream: Stream, reply: Vec<u8>, chunked: bool, http_status: u16, tx: &mpsc::Sender<Seen>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut content_length = 0usize;
    let mut first = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if first.is_empty() {
            first = line.clone();
        }
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap();
        }
    }
    if first.is_empty() {
        return; // a reachability probe: connect and close
    }
    assert!(first.starts_with("POST "), "{first}");
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).unwrap();
    let _ = tx.send(parse_request(&body).unwrap());
    let mut out = reader.into_inner();
    let status_text = if http_status == 200 { "OK" } else { "Nope" };
    if chunked {
        let mut resp = format!("HTTP/1.1 {http_status} {status_text}\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n").into_bytes();
        for chunk in reply.chunks(7) {
            resp.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            resp.extend_from_slice(chunk);
            resp.extend_from_slice(b"\r\n");
        }
        resp.extend_from_slice(b"0\r\n\r\n");
        out.write_all(&resp).unwrap();
    } else {
        out.write_all(format!("HTTP/1.1 {http_status} {status_text}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", reply.len()).as_bytes())
            .unwrap();
        out.write_all(&reply).unwrap();
    }
    out.flush().unwrap();
}

/// An attribute to write: tag, name, values.
type Attr<'a> = (u8, &'a str, &'a [&'a [u8]]);
/// A group to write: tag and attributes.
type Group<'a> = (u8, &'a [Attr<'a>]);

/// An IPP reply body.
fn ipp(status: u16, groups: &[Group<'_>]) -> Vec<u8> {
    let mut b = vec![2, 0];
    b.extend_from_slice(&status.to_be_bytes());
    b.extend_from_slice(&1u32.to_be_bytes());
    for (group, attrs) in groups {
        b.push(*group);
        for (tag, name, values) in *attrs {
            for (i, v) in values.iter().enumerate() {
                b.push(*tag);
                let n = if i == 0 { name.as_bytes() } else { b"" };
                b.extend_from_slice(&(n.len() as u16).to_be_bytes());
                b.extend_from_slice(n);
                b.extend_from_slice(&(v.len() as u16).to_be_bytes());
                b.extend_from_slice(v);
            }
        }
    }
    b.push(TAG_END);
    b
}

fn ok_header() -> Group<'static> {
    (TAG_OPERATION_GROUP, &[(TAG_CHARSET, "attributes-charset", &[b"utf-8"]), (TAG_LANGUAGE, "attributes-natural-language", &[b"en"])])
}

#[test]
fn printers_are_listed_with_state_and_default() {
    let three = ipp(
        0,
        &[
            ok_header(),
            (
                TAG_PRINTER_GROUP,
                &[
                    (TAG_NAME, "printer-name", &[b"Office"]),
                    (TAG_TEXT, "printer-info", &[b"HP in the office"]),
                    (TAG_ENUM, "printer-state", &[&3u32.to_be_bytes()]),
                    (TAG_BOOLEAN, "printer-is-accepting-jobs", &[&[1]]),
                ],
            ),
            (
                TAG_PRINTER_GROUP,
                &[
                    (TAG_NAME, "printer-name", &[b"Stopped"]),
                    (TAG_ENUM, "printer-state", &[&5u32.to_be_bytes()]),
                    (TAG_BOOLEAN, "printer-is-accepting-jobs", &[&[0]]),
                ],
            ),
        ],
    );
    for chunked in [false, true] {
        let (ep, rx) = fake(three.clone(), chunked, 200);
        // `printers` asks for the default too; that second request reaches a server that has
        // closed, so the default is simply unknown here.
        let list = printers(&ep).unwrap();
        let (op, attrs, doc) = rx.recv().unwrap();
        assert_eq!(op, OP_CUPS_GET_PRINTERS);
        assert!(attrs.iter().any(|a| a.name == "requested-attributes" && a.values.len() == 4 && a.tag == TAG_KEYWORD));
        assert!(doc.is_empty());
        assert_eq!(list.len(), 2, "chunked {chunked}");
        assert_eq!((list[0].name.as_str(), list[0].info.as_str(), list[0].state, list[0].accepting_jobs), ("Office", "HP in the office", 3, true));
        assert_eq!((list[1].name.as_str(), list[1].state, list[1].accepting_jobs), ("Stopped", 5, false));
        assert!(!list[0].is_default && !list[1].is_default);
    }
}

#[test]
fn default_printer_is_read_and_absence_is_not_an_error() {
    let (ep, rx) = fake(ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]), false, 200);
    assert_eq!(default_printer(&ep).unwrap(), Some("Office".into()));
    assert_eq!(rx.recv().unwrap().0, OP_CUPS_GET_DEFAULT);
    let (ep, _rx) = fake(ipp(0x0406, &[ok_header(), (TAG_OPERATION_GROUP, &[(TAG_TEXT, "status-message", &[b"No default printer"])])]), false, 200);
    assert_eq!(default_printer(&ep).unwrap(), None);
}

#[test]
fn a_print_job_carries_the_document_and_returns_its_id() {
    let (ep, rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(TAG_INTEGER, "job-id", &[&42u32.to_be_bytes()])])]), true, 200);
    let pdf = b"%PDF-1.4 fake".to_vec();
    let id =
        print_job(&ep, &Job { printer: "Office", is_class: false, title: "photo.psd", user: "diego", copies: 2, document: &pdf, format: "application/pdf" })
            .unwrap();
    assert_eq!(id, 42);
    let (op, attrs, doc) = rx.recv().unwrap();
    assert_eq!(op, OP_PRINT_JOB);
    assert_eq!(doc, pdf);
    let get = |n: &str| attrs.iter().find(|a| a.name == n).cloned().unwrap_or_else(|| panic!("{n}"));
    assert_eq!(get("printer-uri").values[0], format!("ipp://{}/printers/Office", ep.host()).as_bytes());
    assert_eq!(get("job-name").values[0], b"photo.psd");
    assert_eq!(get("requesting-user-name").values[0], b"diego");
    assert_eq!(get("document-format").values[0], b"application/pdf");
    let copies = get("copies");
    assert_eq!((copies.group, copies.tag, copies.int()), (TAG_JOB_GROUP, TAG_INTEGER, Some(2)));
    // One copy: no job attributes group at all.
    let (ep, rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(TAG_INTEGER, "job-id", &[&7u32.to_be_bytes()])])]), false, 200);
    print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" }).unwrap();
    assert!(!rx.recv().unwrap().1.iter().any(|a| a.name == "copies"));
}

#[test]
fn error_statuses_and_broken_replies_are_errors() {
    let (ep, _rx) = fake(ipp(0x0506, &[ok_header(), (TAG_OPERATION_GROUP, &[(TAG_TEXT, "status-message", &[b"Printer is not accepting jobs"])])]), false, 200);
    let e = print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" }).unwrap_err();
    assert_eq!(e, Error::Status(0x0506, "Printer is not accepting jobs".into()));
    assert!(e.to_string().contains("0x0506"));
    // A CUPS with no printer at all answers "not found": that is an empty list, not an error.
    let (ep, _rx) = fake(ipp(0x0406, &[ok_header(), (TAG_OPERATION_GROUP, &[(TAG_TEXT, "status-message", &[b"No destinations added."])])]), false, 200);
    assert_eq!(printers(&ep).unwrap(), Vec::<Printer>::new());
    // HTTP 401, a reply cut short, garbage.
    let (ep, _rx) = fake(ipp(0, &[ok_header()]), false, 401);
    assert!(matches!(printers(&ep), Err(Error::Protocol(m)) if m.contains("401")));
    let (ep, _rx) = fake(vec![2, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_GROUP, TAG_NAME, 0, 5], true, 200);
    assert!(matches!(printers(&ep), Err(Error::Protocol(_))));
    let (ep, _rx) = fake(b"not ipp at all".to_vec(), false, 200);
    assert!(matches!(printers(&ep), Err(Error::Protocol(_))));
    // Nobody listening.
    let e = printers(&Endpoint::Tcp("127.0.0.1".into(), 1)).unwrap_err();
    assert!(matches!(e, Error::Unreachable(_)), "{e}");
}

#[test]
fn reply_parsing_is_bounded() {
    assert!(Reply::parse(&[]).is_err());
    assert!(Reply::parse(&[2, 0, 0, 0, 0, 0, 0, 1]).is_err(), "no end tag");
    let mut many = vec![2, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_GROUP];
    for _ in 0..MAX_ATTRIBUTES + 1 {
        many.extend_from_slice(&[TAG_NAME, 0, 1, b'a', 0, 0]);
    }
    many.push(TAG_END);
    assert!(matches!(Reply::parse(&many), Err(Error::Protocol(m)) if m.contains("too many")));
    // An additional value before any attribute.
    assert!(Reply::parse(&[2, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_GROUP, TAG_NAME, 0, 0, 0, 1, b'x', TAG_END]).is_err());
    assert!(dechunk(b"zz\r\n").is_err() && dechunk(b"5\r\nab").is_err());
    assert_eq!(dechunk(b"3\r\nabc\r\n0\r\n\r\n").unwrap(), b"abc");
}

#[cfg(unix)]
#[test]
fn the_unix_socket_endpoint_works_and_detect_honours_cups_server() {
    let dir = std::env::temp_dir().join(format!("pc-ipp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("cups.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    let reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Sock"])])]);
    std::thread::spawn(move || {
        for _ in 0..3 {
            let (stream, _) = listener.accept().unwrap();
            serve(Stream::Unix(stream), reply.clone(), false, 200, &tx);
        }
    });
    let ep = Endpoint::Unix(sock.clone());
    assert!(ep.reachable());
    assert_eq!(default_printer(&ep).unwrap(), Some("Sock".into()));
    assert_eq!(rx.recv().unwrap().0, OP_CUPS_GET_DEFAULT);
    assert!(!Endpoint::Unix(dir.join("missing.sock")).reachable());
    assert_eq!(ep.to_string(), sock.display().to_string());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn requests_start_with_the_mandatory_attributes() {
    let r = Request::new(OP_PRINT_JOB, 9).end();
    let (op, attrs, doc) = parse_request(&r).unwrap();
    assert_eq!(op, OP_PRINT_JOB);
    assert_eq!(Reply::parse(&r).unwrap().request_id, 9);
    assert_eq!(attrs[0].name, "attributes-charset");
    assert_eq!(attrs[1].name, "attributes-natural-language");
    assert!(doc.is_empty());
}

/// A server that keeps the connection open after its reply (a printer reached directly, a proxy)
/// must not hold the client until the read times out: the reply ends at Content-Length, or at
/// the last chunk.
#[test]
fn replies_end_at_content_length_or_the_last_chunk_without_waiting_for_close() {
    for chunked in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let one = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
        let none = ipp(0x0406, &[ok_header()]);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut reader = BufReader::new(stream);
                let (mut line, mut len) = (String::new(), 0usize);
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                if len == 0 {
                    continue;
                }
                let mut body = vec![0u8; len];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let op = parse_request(&body).map(|r| r.0).unwrap_or(0);
                let reply = if op == OP_CUPS_GET_PRINTERS { one.clone() } else { none.clone() };
                let mut out = reader.into_inner();
                let mut resp = Vec::new();
                if chunked {
                    resp.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n");
                    for chunk in reply.chunks(5) {
                        resp.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                        resp.extend_from_slice(chunk);
                        resp.extend_from_slice(b"\r\n");
                    }
                    resp.extend_from_slice(b"0\r\n\r\n");
                } else {
                    resp.extend_from_slice(format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", reply.len()).as_bytes());
                    resp.extend_from_slice(&reply);
                }
                if out.write_all(&resp).and_then(|()| out.flush()).is_err() {
                    continue;
                }
                // Keep this connection open for a while, on its own thread, and go on serving.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(10));
                    drop(out);
                });
            }
        });
        let ep = Endpoint::Tcp("127.0.0.1".into(), port);
        let started = std::time::Instant::now();
        let list = printers(&ep).unwrap();
        assert_eq!(list.len(), 1, "chunked {chunked}");
        assert!(started.elapsed() < Duration::from_secs(5), "waited for the server to close (chunked {chunked}): {:?}", started.elapsed());
    }
}

/// A chunked reply whose payload contains the bytes of a last chunk (`\r\n0\r\n\r\n`, here inside
/// a printer name) and arrives split right after them: the reader must wait for the real last
/// chunk instead of returning a cut reply.
#[test]
fn a_last_chunk_pattern_inside_a_value_does_not_end_the_reply() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Of\r\n0\r\n\r\nfice"])])]);
    let cut = reply.windows(7).position(|w| w == b"\r\n0\r\n\r\n").unwrap() + 7;
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let (mut line, mut len) = (String::new(), 0usize);
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).unwrap();
        let mut out = reader.into_inner();
        let (first, second) = reply.split_at(cut);
        // Part one ends exactly on the pattern, inside the first chunk's payload.
        let mut a = b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        a.extend_from_slice(format!("{:x}\r\n", first.len()).as_bytes());
        a.extend_from_slice(first);
        out.write_all(&a).and_then(|()| out.flush()).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let mut b = b"\r\n".to_vec();
        b.extend_from_slice(format!("{:x}\r\n", second.len()).as_bytes());
        b.extend_from_slice(second);
        b.extend_from_slice(b"\r\n0\r\n\r\n");
        out.write_all(&b).and_then(|()| out.flush()).unwrap();
    });
    let ep = Endpoint::Tcp("127.0.0.1".into(), port);
    let list = printers(&ep).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Of\r\n0\r\n\r\nfice");
    // The boundary walk itself: complete only at a zero-size chunk, never on payload bytes.
    assert!(!chunked_complete(b"5\r\nab\r\n0"));
    assert!(!chunked_complete(b"7\r\n\r\n0\r\n\r\nx\r\n"));
    assert!(chunked_complete(b"7\r\n\r\n0\r\n\r\nx\r\n0\r\n\r\n"));
    assert!(chunked_complete(b"zz\r\n") && dechunk(b"zz\r\n").is_err());
}

/// A chunk size that cannot be satisfied (`usize::MAX`, `usize::MAX - 1`, anything past the
/// reply cap) is a protocol error, through the real reader, never an overflow panic.
#[test]
fn an_impossible_chunk_size_is_a_protocol_error_not_a_panic() {
    for size_line in ["ffffffffffffffff", "fffffffffffffffe", "1000001"] {
        assert!(chunked_complete(format!("{size_line}\r\n").as_bytes()));
        assert!(dechunk(format!("{size_line}\r\nab\r\n").as_bytes()).is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let line = size_line.to_string();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (mut l, mut len) = (String::new(), 0usize);
            loop {
                l.clear();
                if reader.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                    break;
                }
                if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let mut out = reader.into_inner();
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n{line}\r\n");
            let _ = out.write_all(resp.as_bytes()).and_then(|()| out.flush());
        });
        let ep = Endpoint::Tcp("127.0.0.1".into(), port);
        match printers(&ep) {
            Err(Error::Protocol(_)) => {}
            other => panic!("{size_line}: expected a protocol error, got {other:?}"),
        }
    }
}
