//! Re-exports [`craft_ipp`] for backward compatibility with `photocraft-ipp`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_ipp::*;

#[cfg(test)]
mod incomplete_responses {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Endpoint, Error, Request, TAG_NAME, TAG_PRINTER_GROUP, printers};
    use std::io::{BufRead, BufReader, Read, Write};

    #[test]
    fn the_shared_client_rejects_incomplete_content_length_bodies() {
        let mut reply = Request::new(0, 2);
        reply.group(TAG_PRINTER_GROUP);
        reply.attr(TAG_NAME, "printer-name", b"Office");
        let complete = reply.end();
        let mut accepted = Vec::new();
        for (probe, body) in (0..3).flat_map(|probe| [Vec::new(), b"o".to_vec(), complete.clone()].map(|body| (probe, body))) {
            let complete = complete.clone();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                for index in 0..=probe {
                    let (stream, _) = listener.accept().unwrap();
                    let mut reader = BufReader::new(stream);
                    let (mut line, mut length) = (String::new(), 0);
                    loop {
                        line.clear();
                        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    reader.read_exact(&mut vec![0; length]).unwrap();
                    let response_body = if index == probe { body.as_slice() } else { complete.as_slice() };
                    let declared = response_body.len() + usize::from(index == probe);
                    let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\n\r\n").into_bytes();
                    response.extend_from_slice(response_body);
                    reader.into_inner().write_all(&response).unwrap();
                }
            });
            let result = printers(&Endpoint::Tcp("127.0.0.1".into(), port));
            server.join().unwrap();
            if !matches!(result, Err(Error::Protocol(_))) {
                accepted.push((probe, result));
            }
        }
        assert!(accepted.is_empty(), "accepted incomplete HTTP bodies at probes: {accepted:?}");
    }
}
