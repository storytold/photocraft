//! Re-exports [`craft_ipp`] for backward compatibility with `photocraft-ipp`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_ipp::*;

#[cfg(test)]
mod chunk_framing {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    #[test]
    fn the_shared_client_rejects_malformed_chunk_separators() {
        for separator in [b"XX", b"\n\r", b"\r\n"] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
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
                // A successful IPP reply containing the default printer "P".
                let body = b"\x02\x00\x00\x00\x00\x00\x00\x02\x04\x42\x00\x0cprinter-name\x00\x01P\x03";
                let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
                response.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
                response.extend_from_slice(body);
                response.extend_from_slice(separator);
                response.extend_from_slice(b"0\r\n\r\n");
                reader.into_inner().write_all(&response).unwrap();
            });
            let result = default_printer(&Endpoint::Tcp("127.0.0.1".into(), port));
            server.join().unwrap();
            if separator == b"\r\n" {
                assert_eq!(result.unwrap().as_deref(), Some("P"));
            } else {
                assert!(matches!(result, Err(Error::Protocol(message)) if message == "invalid chunk end"));
            }
        }
    }
}
