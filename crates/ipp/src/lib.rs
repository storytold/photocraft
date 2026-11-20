//! Re-exports [`craft_ipp`] for backward compatibility with `photocraft-ipp`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_ipp::*;

#[cfg(test)]
mod http_status {
    use super::{Endpoint, Error, default_printer};
    use std::io::{BufRead, BufReader, Read, Write};

    #[test]
    fn malformed_status_lines_cannot_report_a_default_printer() {
        for (status_line, valid) in [
            ("garbage 200 OK", false),
            ("HTTP/1.1 +200 OK", false),
            ("HTTP/1.1 0200 OK", false),
            ("http/1.1 200 OK", false),
            ("HTTP/1.a 200 OK", false),
            ("HTTP/1.10 200 OK", false),
            ("HTTP/1.0 200 OK", true),
            ("HTTP/1.1 200 ", true),
            ("HTTP/1.9 200 OK", true),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let (mut line, mut length) = (String::new(), 0usize);
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
                // A successful IPP reply containing the default printer "P".
                let body = b"\x02\x00\x00\x00\x00\x00\x00\x02\x04\x42\x00\x0cprinter-name\x00\x01P\x03";
                let mut response = format!("{status_line}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
                response.extend_from_slice(body);
                reader.into_inner().write_all(&response).unwrap();
            });
            let result = default_printer(&Endpoint::Tcp("127.0.0.1".into(), port));
            server.join().unwrap();
            if valid {
                assert_eq!(result, Ok(Some("P".into())), "{status_line}");
            } else {
                assert!(matches!(&result, Err(Error::Protocol(message)) if message.starts_with("bad status line")), "{status_line}: {result:?}");
            }
        }
    }
}
