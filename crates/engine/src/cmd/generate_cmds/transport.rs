//! Bounded v3 wire adapter; no SDK source or private build dependency.
use super::{Output, Request, bad};
use crate::{EngineError, Result, jobs::JobCtx};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::{Duration, Instant},
};

const FRAME_LIMIT: usize = 8 * 1024 * 1024;
const IDLE: Duration = Duration::from_secs(30 * 60);
const WALL: Duration = Duration::from_secs(4 * 60 * 60);
fn io(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Enki: {error}"))
}

struct Connection {
    reader: BufReader<UnixStream>,
    started: Instant,
}
impl Connection {
    fn connect(path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(path).map_err(io)?;
        if rustix::net::sockopt::socket_peercred(&stream).map_err(io)?.uid != rustix::process::geteuid() {
            return Err(io("runtime belongs to another user"));
        }
        stream.set_read_timeout(Some(Duration::from_millis(250))).map_err(io)?;
        stream.set_write_timeout(Some(Duration::from_secs(1))).map_err(io)?;
        Ok(Self { reader: BufReader::new(stream), started: Instant::now() })
    }
    fn send(&mut self, request: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(request).map_err(io)?;
        if bytes.len() >= FRAME_LIMIT {
            return Err(io("request exceeds frame limit"));
        }
        bytes.push(b'\n');
        self.reader.get_mut().write_all(&bytes).map_err(io)
    }
    fn receive(&mut self, ctx: &JobCtx) -> Result<Value> {
        let mut frame = Vec::new();
        let mut last = Instant::now();
        loop {
            if ctx.cancelled() {
                let _ = self.send(&json!({"type":"cancel"}));
                let _ = self.reader.get_ref().shutdown(std::net::Shutdown::Both);
                return Err(EngineError::Cancelled);
            }
            if self.started.elapsed() > WALL || last.elapsed() > IDLE {
                return Err(io("runtime response timed out"));
            }
            let buffer = match self.reader.fill_buf() {
                Ok([]) => return Err(io("runtime closed the connection")),
                Ok(b) => b,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => continue,
                Err(e) => return Err(io(e)),
            };
            let end = buffer.iter().position(|b| *b == b'\n');
            let take = end.map_or(buffer.len(), |n| n + 1);
            if frame.len().saturating_add(take) > FRAME_LIMIT {
                return Err(io("runtime frame exceeds limit"));
            }
            frame.extend_from_slice(buffer.get(..take).ok_or_else(|| io("invalid runtime frame"))?);
            self.reader.consume(take);
            last = Instant::now();
            if end.is_some() {
                return serde_json::from_slice(&frame).map_err(io);
            }
        }
    }
    fn handshake(&mut self, ctx: &JobCtx) -> Result<()> {
        self.send(&json!({"type":"negotiate","versions":[3],"app":"lumafoundry"}))?;
        let reply = self.receive(ctx)?;
        if reply.get("type").and_then(Value::as_str) != Some("negotiated") || reply.get("version").and_then(Value::as_u64) != Some(3) {
            return Err(io("runtime does not support image protocol v3"));
        }
        Ok(())
    }
}

pub(super) fn generate(request: &Request, ctx: &JobCtx) -> Result<Output> {
    let path = std::env::var_os("SHAREDAI_SOCKET")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()).map(|s| PathBuf::from(s).join("sharedai/ipc.sock")))
        .ok_or_else(|| io("start the runtime and set XDG_RUNTIME_DIR or SHAREDAI_SOCKET"))?;
    generate_at(&path, request, ctx)
}

fn generate_at(path: &std::path::Path, request: &Request, ctx: &JobCtx) -> Result<Output> {
    ctx.progress(0.0, "Connecting to Enki");
    let mut model = request.model.clone();
    let mut steps = request.steps;
    let mut cfg = request.cfg;
    if model.is_empty() {
        let mut connection = Connection::connect(path)?;
        connection.handshake(ctx)?;
        connection.send(&json!({"type":if request.edit.is_some(){"image_edit_defaults"}else{"image_defaults"}}))?;
        let defaults = connection.receive(ctx)?;
        let profile = defaults
            .get("profile")
            .filter(|_| defaults["type"] == "image_defaults")
            .ok_or_else(|| io("runtime did not supply a model recommendation; update Enki for editing support"))?;
        model =
            profile.get("model").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 1024).ok_or_else(|| io("invalid recommended model"))?.into();
        if steps.is_none() {
            steps = profile.get("steps").and_then(Value::as_u64).filter(|n| (1..=50).contains(n)).map(|n| n as u32);
        }
        if cfg.is_none() {
            cfg = profile.get("cfg").and_then(Value::as_f64).filter(|n| n.is_finite() && (1.0..=20.0).contains(n));
        }
        if steps.is_none() || cfg.is_none() {
            return Err(io("invalid recommended sampling settings"));
        }
    }
    let mut connection = Connection::connect(path)?;
    connection.handshake(ctx)?;
    if request.edit.is_some() {
        connection.send(&json!({"type":"image_capabilities"}))?;
        let caps = connection.receive(ctx)?;
        if caps.get("type").and_then(Value::as_str) != Some("image_capabilities")
            || caps.get("masked_edit_revision").and_then(Value::as_u64) != Some(1)
            || caps.get("max_side").and_then(Value::as_u64).is_none_or(|n| n < u64::from(request.width.max(request.height)))
        {
            return Err(io("this runtime does not support protected masked editing; update Enki before using this tool"));
        }
    }
    let mut params = json!({"width":request.width,"height":request.height,"seed":request.seed});
    if let Some(n) = steps {
        params["steps"] = json!(n);
    }
    if let Some(n) = cfg {
        params["cfg"] = json!(n);
    }
    if let Some(n) = request.strength {
        params["strength"] = json!(n);
    }
    let mut message = json!({"type":"imagine","task":"image","model":model,"prompt":request.prompt,"negative":request.negative,"params":params});
    if let Some(edit) = &request.edit {
        message["task"] = json!("fill");
        message["source_png"] = json!(edit.source_png);
        message["params"]["mask_png"] = json!(edit.mask_png);
    }
    connection.send(&message)?;
    ctx.progress(0.0, "Waiting for image model");
    let mut sequence = 0u64;
    loop {
        let reply = connection.receive(ctx)?;
        match reply.get("type").and_then(Value::as_str) {
            Some("image_step") => {
                let step = reply.get("step").and_then(Value::as_u64).ok_or_else(|| io("invalid sampling step"))?;
                let total = reply.get("total").and_then(Value::as_u64).filter(|n| (1..=50).contains(n)).ok_or_else(|| io("invalid sampling total"))?;
                if reply.get("seq").and_then(Value::as_u64) != Some(sequence) || step > total {
                    return Err(io("invalid sampling sequence"));
                }
                sequence = sequence.saturating_add(1);
                ctx.progress(step as f32 / total as f32 * 0.95, "Generating image");
            }
            Some("image") => {
                let dimension = |key: &str, expected: u32| -> Result<u32> {
                    if reply.get(key).and_then(Value::as_u64) != Some(u64::from(expected)) {
                        return Err(io("result dimensions differ from request"));
                    }
                    Ok(expected)
                };
                let width = dimension("width", request.width)?;
                let height = dimension("height", request.height)?;
                let encoded = reply
                    .get("png")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= (5 * 1024 * 1024_usize).div_ceil(3) * 4)
                    .ok_or_else(|| bad("invalid runtime PNG payload"))?;
                let png = photocraft_paint::tile::b64_decode(encoded).ok_or_else(|| bad("invalid runtime PNG encoding"))?;
                super::validate_png(&png, width, height)?;
                let seed = reply.get("seed").and_then(Value::as_i64).ok_or_else(|| io("missing result seed"))?;
                ctx.progress(0.98, "Importing image");
                return Ok(Output { png, width, height, seed, model });
            }
            Some("busy") => return Err(io("runtime is busy; try again when its queue clears")),
            Some("error" | "inference_error") => {
                return Err(io(reply.get("message").and_then(Value::as_str).unwrap_or("image generation failed").chars().take(1024).collect::<String>()));
            }
            _ => return Err(io("unexpected image response")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_and_truncated_frames_fail_without_panicking() {
        for bytes in [b"{\"type\":\"image\"}".to_vec(), vec![b'x'; FRAME_LIMIT + 1]] {
            let (client, mut server) = UnixStream::pair().unwrap();
            let thread = std::thread::spawn(move || {
                let _ = server.write_all(&bytes);
            });
            let mut connection = Connection { reader: BufReader::new(client), started: Instant::now() };
            assert!(connection.receive(&JobCtx::new()).is_err());
            drop(connection);
            thread.join().unwrap();
        }
    }
    #[test]
    fn cancellation_interrupts_a_partial_frame() {
        let (client, mut server) = UnixStream::pair().unwrap();
        client.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        server.write_all(b"{\"type\":").unwrap();
        let ctx = JobCtx::new();
        ctx.cancel();
        let mut connection = Connection { reader: BufReader::new(client), started: Instant::now() };
        assert!(matches!(connection.receive(&ctx), Err(EngineError::Cancelled)));
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    #[test]
    fn authenticated_runtime_receives_source_request_and_reports_seed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut r = BufReader::new(stream);
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let hello: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(hello["versions"], json!([3]));
            assert_eq!(hello["app"], "lumafoundry");
            writeln!(r.get_mut(), "{}", json!({"type":"negotiated","version":3,"process_id":1})).unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["task"], "image");
            assert_eq!(request["params"]["width"], 64);
            assert_eq!(request["model"], "test-model");
            let image = photocraft_codecs::Image::new(64, 64, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).unwrap();
            let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
            for reply in [
                json!({"type":"image_step","seq":0,"step":1,"total":1}),
                json!({"type":"image","width":64,"height":64,"seed":123,"png":photocraft_paint::tile::b64_encode(&png)}),
            ] {
                writeln!(r.get_mut(), "{reply}").unwrap();
            }
        });
        let request = Request::parse(&json!({"prompt":"test landscape","width":64,"height":64,"model":"test-model"})).unwrap();
        let ctx = JobCtx::new();
        let result = generate_at(&path, &request, &ctx).unwrap();
        assert_eq!((result.width, result.height, result.seed), (64, 64, 123));
        assert_eq!(result.model, "test-model");
        assert!(ctx.fraction() >= 0.95);
        worker.join().unwrap();
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    #[test]
    fn old_runtime_is_refused_before_sending_any_image_or_mask() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut r = BufReader::new(stream);
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            writeln!(r.get_mut(), "{}", json!({"type":"negotiated","version":3})).unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            let message: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(message["type"], "image_capabilities");
            writeln!(r.get_mut(), "{}", json!({"type":"error","message":"unknown request"})).unwrap();
            line.clear();
            assert_eq!(r.read_line(&mut line).unwrap(), 0, "must not send image data to an unsupported runtime");
        });
        let mut request = Request::parse(&json!({"prompt":"test","width":64,"height":64,"model":"test"})).unwrap();
        request.edit = Some(super::super::EditInput { source_png: "must-not-send".into(), mask_png: "must-not-send".into() });
        let error = generate_at(&path, &request, &JobCtx::new()).err().unwrap();
        assert!(error.to_string().contains("update Enki"));
        worker.join().unwrap();
    }
}

pub(super) fn subject_mask(source_png: String, width: u32, height: u32, ctx: &JobCtx) -> Result<(Vec<u8>, String)> {
    let path = std::env::var_os("SHAREDAI_SOCKET")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(|s| PathBuf::from(s).join("sharedai/ipc.sock")))
        .ok_or_else(|| io("start Enki before selecting a subject"))?;
    let mut connection = Connection::connect(&path)?;
    connection.handshake(ctx)?;
    connection.send(&json!({"type":"segmentation_capabilities"}))?;
    let caps = connection.receive(ctx)?;
    if caps.get("type").and_then(Value::as_str) != Some("segmentation_capabilities") || caps.get("available").and_then(Value::as_bool) != Some(true) {
        return Err(io("install the Enki subject-mask worker and verified model before using learned selection"));
    }
    connection.send(&json!({"type":"segment_image","source_png":source_png}))?;
    let reply = connection.receive(ctx)?;
    if reply.get("type").and_then(Value::as_str) != Some("subject_mask") {
        return Err(io(reply.get("message").and_then(Value::as_str).unwrap_or("subject selection failed")));
    }
    if reply.get("width").and_then(Value::as_u64) != Some(width as u64) || reply.get("height").and_then(Value::as_u64) != Some(height as u64) {
        return Err(io("subject mask dimensions differ from the document"));
    }
    let encoded = reply.get("png").and_then(Value::as_str).filter(|s| s.len() <= 7 * 1024 * 1024).ok_or_else(|| io("invalid subject mask"))?;
    let png = photocraft_paint::tile::b64_decode(encoded).ok_or_else(|| io("invalid subject mask encoding"))?;
    super::validate_png(&png, width, height)?;
    let model = reply.get("model").and_then(Value::as_str).filter(|s| s.len() <= 128).ok_or_else(|| io("invalid subject model identity"))?.to_string();
    Ok((png, model))
}
