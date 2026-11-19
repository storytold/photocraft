//! Re-exports [`craft_ipp`] for backward compatibility with `photocraft-ipp`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_ipp::*;

#[cfg(test)]
mod job_ids {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{Endpoint, Error, Job, Request, TAG_JOB_GROUP, print_job};
    use std::io::{BufRead, BufReader, Read, Write};

    #[test]
    fn the_shared_client_requires_positive_job_ids() {
        for id in [i32::MIN, -1, 0, 1, i32::MAX] {
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
                // IPP replies share the request header layout: status zero means success.
                let mut reply = Request::new(0, 2);
                reply.group(TAG_JOB_GROUP);
                reply.integer("job-id", id);
                let body = reply.end();
                let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
                response.extend_from_slice(&body);
                reader.into_inner().write_all(&response).unwrap();
            });
            let result = print_job(
                &Endpoint::Tcp("127.0.0.1".into(), port),
                &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" },
            );
            server.join().unwrap();
            if id > 0 {
                assert_eq!(result.unwrap(), u32::try_from(id).unwrap());
            } else {
                assert!(matches!(result, Err(Error::Protocol(_))), "job-id {id}: {result:?}");
            }
        }
    }
}
