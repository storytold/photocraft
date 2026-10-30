//! Generative Fill (`edit.generativeFill`): replace the selection with AI-generated content that
//! follows a text prompt and blends with its surroundings.
//!
//! The model never runs inside PhotoCraft and never leaves your network. The command sends the
//! selection (plus a margin of context) and a mask to an inpainting server you host yourself,
//! either on this machine or on a home server. Two backends are supported (`backend`):
//!
//! - `comfyui`: ComfyUI's HTTP API. PhotoCraft uploads the image and mask, queues a small
//!   inpainting workflow (`InpaintModelConditioning` + `KSampler`, which works with regular and
//!   dedicated inpainting checkpoints), polls the queue and downloads the result.
//! - `a1111`: the AUTOMATIC1111 `/sdapi/v1/img2img` protocol, also served by Forge and SD.Next.
//!
//! "Locally hosted" is enforced, not assumed: the server must be plain `http://` and every
//! address its host resolves to must be on this machine or a private network (loopback, RFC 1918,
//! link-local, unique-local IPv6, or Tailscale's 100.64.0.0/10), so a mistyped or malicious host
//! can't ship the user's pixels to the public internet.
//!
//! The request runs as a cancellable background job. The result lands in a new layer (default,
//! non-destructive, like Photoshop's generative layers) or in the active layer, as one undo step.

use photocraft_color::PixelFormat;
use photocraft_doc::{Layer, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::{from_rgba_into, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Environment variable that supplies the server when the command has no `host`.
pub const HOST_ENV: &str = "PHOTOCRAFT_GENFILL_URL";
/// Environment variable that supplies the backend when the command has no `backend`.
pub const BACKEND_ENV: &str = "PHOTOCRAFT_GENFILL_BACKEND";
/// Largest side of the window sent to the model, in pixels (the window is the selection plus context).
const MAX_WINDOW_SIDE: u32 = 2048;
/// Largest response accepted from the server.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_RESPONSE_BYTES: usize = 256 << 20;
/// How long to wait for the model before giving up.
const TIMEOUT_SECS: u64 = 30 * 60;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "edit.generativeFill".into(), msg: msg.into() }
}

fn other(msg: impl Into<String>) -> EngineError {
    EngineError::Other(msg.into())
}

fn num(p: &Value, key: &str, default: f64, lo: f64, hi: f64) -> Result<f64> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(default),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && (lo..=hi).contains(&x) => Ok(x),
            _ => Err(bad(format!("`{key}` must be a number from {lo} to {hi}"))),
        },
    }
}

fn text(p: &Value, key: &str, max: usize) -> Result<String> {
    match p.get(key).filter(|v| !v.is_null()) {
        None => Ok(String::new()),
        Some(Value::String(s)) if s.chars().count() <= max => Ok(s.clone()),
        _ => Err(bad(format!("`{key}` must be text of at most {max} characters"))),
    }
}

// ------------------------------------------------------------------ backend and host

/// Which kind of server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    A1111,
    Comfy,
}

impl Backend {
    fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "comfyui" | "comfy" => Ok(Backend::Comfy),
            "a1111" | "automatic1111" | "forge" | "sdnext" => Ok(Backend::A1111),
            _ => Err(bad("`backend` must be comfyui|a1111")),
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Backend::A1111 => 7860,
            Backend::Comfy => 8188,
        }
    }

    /// The name `backend` parameters and Preferences use.
    fn name(self) -> &'static str {
        match self {
            Backend::A1111 => "a1111",
            Backend::Comfy => "comfyui",
        }
    }
}

/// A validated server address.
#[derive(Debug, Clone, PartialEq)]
struct Endpoint {
    addr: std::net::SocketAddr,
    /// The `Host` header value.
    host: String,
    /// The path prefix, without a trailing slash (a reverse proxy mount point).
    base: String,
    /// Seconds to wait for the connection and for the whole exchange.
    connect_secs: u64,
    total_secs: u64,
}

/// Whether `ip` is on this machine or a private network.
fn is_local(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            a.is_loopback() || a.is_private() || a.is_link_local() || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            a.is_loopback() || (s[0] & 0xfe00) == 0xfc00 || (s[0] & 0xffc0) == 0xfe80 || a.to_ipv4_mapped().is_some_and(|v| is_local(IpAddr::V4(v)))
        }
    }
}

/// Parse `[http://]host[:port][/prefix]` and resolve it, refusing anything outside the local
/// network. The port defaults to the backend's.
fn parse_endpoint(url: &str, backend: Backend) -> Result<Endpoint> {
    use std::net::ToSocketAddrs;
    let url = url.trim();
    if url.starts_with("https://") {
        return Err(bad("the host must be plain http:// on your local network (a home server needs no TLS)"));
    }
    let rest = url.strip_prefix("http://").unwrap_or(url);
    if rest.contains("://") {
        return Err(bad("the host must look like 192.168.1.20:8188 or http://myserver.local:8188"));
    }
    let (authority, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    if authority.is_empty() || authority.contains('@') {
        return Err(bad("the host must look like 192.168.1.20:8188 or http://myserver.local:8188"));
    }
    // Both parts go straight into the HTTP request (the request line and the `Host` header), so
    // anything that could end a line or start a query is refused rather than escaped: a CR or LF
    // there would inject headers.
    if !authority.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')) {
        return Err(bad("the host may contain only letters, digits and . - _ : [ ]"));
    }
    if !path.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~')) {
        return Err(bad("the host's path may contain only letters, digits and / - _ . ~"));
    }
    let port_of = |p: &str| p.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| bad("the host's port must be 1..65535"));
    let (host, port) = match authority.strip_prefix('[') {
        // [::1]:8188
        Some(v6) => {
            let (h, tail) = v6.split_once(']').ok_or_else(|| bad("unterminated IPv6 address in the host"))?;
            let port = match tail.strip_prefix(':') {
                Some(p) => port_of(p)?,
                None if tail.is_empty() => backend.default_port(),
                None => return Err(bad("the host must look like 192.168.1.20:8188")),
            };
            (h.to_string(), port)
        }
        None => match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), port_of(p)?),
            None => (authority.to_string(), backend.default_port()),
        },
    };
    let addrs: Vec<_> = (host.as_str(), port).to_socket_addrs().map_err(|e| bad(format!("can't resolve `{host}`: {e}")))?.collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_local(a.ip())) {
        return Err(bad(format!("`{host}` isn't on your local network: Generative Fill only talks to a model server you host yourself")));
    }
    let addr = addrs.iter().find(|a| a.is_ipv4()).or(addrs.first()).copied().ok_or_else(|| bad("no address for the host"))?;
    let shown = if authority.contains(':') && !authority.starts_with('[') || authority.starts_with('[') && authority.contains("]:") {
        authority.to_string()
    } else {
        format!("{authority}:{port}")
    };
    Ok(Endpoint {
        addr,
        host: shown,
        base: format!("/{}", path.trim_matches('/')).trim_end_matches('/').to_string(),
        connect_secs: 5,
        total_secs: TIMEOUT_SECS,
    })
}

// ------------------------------------------------------------------ HTTP (HTTP/1.1, Connection: close)

/// Send one request and return the response body. Cancellable: the socket is polled in short
/// slices so a cancelled job stops waiting at once.
#[cfg(not(target_arch = "wasm32"))]
fn request(ep: &Endpoint, method: &str, path: &str, content_type: Option<&str>, body: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    let unreachable = |e: std::io::Error| {
        other(format!(
            "Generative Fill couldn't reach the model server at {} ({e}). Check the host and that ComfyUI or an AUTOMATIC1111-compatible server (started with --api) is running and listening on the network",
            ep.host
        ))
    };
    let mut sock = TcpStream::connect_timeout(&ep.addr, Duration::from_secs(ep.connect_secs)).map_err(unreachable)?;
    sock.set_write_timeout(Some(Duration::from_secs(60))).ok();
    sock.set_read_timeout(Some(Duration::from_millis(250))).ok();
    let mut head = format!("{method} {}{path} HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\nConnection: close\r\n", ep.base, ep.host);
    if let Some(ct) = content_type {
        head.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    if method != "GET" {
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    sock.write_all(head.as_bytes()).and_then(|()| sock.write_all(body)).map_err(unreachable)?;

    let deadline = Instant::now() + Duration::from_secs(ep.total_secs);
    let mut raw: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err(EngineError::Cancelled);
        }
        if Instant::now() > deadline {
            return Err(other("the model server took too long to respond"));
        }
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(buf.get(..n).unwrap_or_default());
                if raw.len() > MAX_RESPONSE_BYTES {
                    return Err(other("the model server's response is too large"));
                }
            }
            // The read timeout surfaces as WouldBlock (Unix) or TimedOut (Windows): poll again.
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            // The server closed mid-body after sending everything it had.
            Err(e) if e.kind() == ErrorKind::ConnectionReset && !raw.is_empty() => break,
            Err(e) => return Err(unreachable(e)),
        }
    }
    parse_response(&raw)
}

#[cfg(target_arch = "wasm32")]
fn request(_: &Endpoint, _: &str, _: &str, _: Option<&str>, _: &[u8], _: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    Err(other("Generative Fill needs a model server and isn't available in the browser build"))
}

fn post_json(ep: &Endpoint, path: &str, body: &Value, cancelled: &dyn Fn() -> bool) -> Result<Value> {
    let body = serde_json::to_vec(body).map_err(|e| other(e.to_string()))?;
    let reply = request(ep, "POST", path, Some("application/json"), &body, cancelled)?;
    serde_json::from_slice(&reply).map_err(|e| other(format!("the model server sent unreadable JSON: {e}")))
}

fn get_json(ep: &Endpoint, path: &str, cancelled: &dyn Fn() -> bool) -> Result<Value> {
    let reply = request(ep, "GET", path, None, &[], cancelled)?;
    serde_json::from_slice(&reply).map_err(|e| other(format!("the model server sent unreadable JSON: {e}")))
}

/// The body of a complete `Connection: close` HTTP/1.1 response, or an error for a non-2xx status.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn parse_response(raw: &[u8]) -> Result<Vec<u8>> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| other("the model server sent a malformed response"))?;
    let head = String::from_utf8_lossy(raw.get(..split).unwrap_or_default()).into_owned();
    let body = raw.get(split + 4..).unwrap_or_default();
    let mut lines = head.split("\r\n");
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| other("the model server sent a malformed status line"))?;
    let chunked = lines.any(|l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("transfer-encoding:") && l.contains("chunked")
    });
    let body = if chunked { dechunk(body)? } else { body.to_vec() };
    if !(200..300).contains(&status) {
        let detail: String = String::from_utf8_lossy(&body).chars().take(400).collect();
        return Err(other(format!("the model server returned HTTP {status}: {detail}")));
    }
    Ok(body)
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn dechunk(mut data: &[u8]) -> Result<Vec<u8>> {
    let malformed = || other("the model server sent a malformed chunked response");
    let mut out = Vec::new();
    loop {
        let eol = data.windows(2).position(|w| w == b"\r\n").ok_or_else(malformed)?;
        let size_text = String::from_utf8_lossy(data.get(..eol).unwrap_or_default()).into_owned();
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16).map_err(|_| malformed())?;
        data = data.get(eol + 2..).ok_or_else(malformed)?;
        if size == 0 {
            return Ok(out);
        }
        out.extend_from_slice(data.get(..size).ok_or_else(malformed)?);
        data = data.get(size.checked_add(2).ok_or_else(malformed)?..).ok_or_else(malformed)?;
    }
}

/// Percent-encode a query value.
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ------------------------------------------------------------------ images

fn png(w: u32, h: u32, layout: photocraft_codecs::ChannelLayout, data: Vec<u8>) -> Result<Vec<u8>> {
    let img = photocraft_codecs::Image::from_u8(w, h, layout, data).map_err(|e| other(e.to_string()))?;
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).map_err(|e| other(e.to_string()))
}

/// Decode the model's PNG into RGBA bytes, checking that it is the size that was asked for.
fn decode_rgba(bytes: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    use photocraft_codecs::ChannelLayout as L;
    use photocraft_codecs::SampleType as T;
    let img = photocraft_codecs::decode(bytes).map_err(|e| other(format!("the model server returned an unreadable image: {e}")))?;
    if img.dimensions() != (w, h) {
        let (iw, ih) = img.dimensions();
        return Err(other(format!("the model server returned a {iw}x{ih} image, expected {w}x{h}")));
    }
    let ch = img.layout().channels();
    let sample = |i: usize| -> Option<u8> {
        match img.sample_type() {
            T::U8 => img.data().get(i).copied(),
            T::U16 => {
                img.data().get(i * 2..i * 2 + 2).and_then(|b| b.try_into().ok()).map(|b: [u8; 2]| (f32::from(u16::from_ne_bytes(b)) / 257.0).round() as u8)
            }
            T::F32 => {
                img.data().get(i * 4..i * 4 + 4).and_then(|b| b.try_into().ok()).map(|b: [u8; 4]| (f32::from_ne_bytes(b).clamp(0.0, 1.0) * 255.0).round() as u8)
            }
            _ => None,
        }
    };
    let n = img.pixel_count();
    let mut out = Vec::with_capacity(n.saturating_mul(4));
    for p in 0..n {
        let at = |c: usize| sample(p * ch + c).ok_or_else(|| other("the model server's image is truncated"));
        let px = match img.layout() {
            L::Gray => [at(0)?, at(0)?, at(0)?, 255],
            L::GrayA => [at(0)?, at(0)?, at(0)?, at(1)?],
            L::Rgb => [at(0)?, at(1)?, at(2)?, 255],
            L::Rgba => [at(0)?, at(1)?, at(2)?, at(3)?],
            L::Cmyk | L::CmykA | L::Lab | L::LabA => return Err(other("the model server returned a CMYK or Lab image")),
        };
        out.extend_from_slice(&px);
    }
    Ok(out)
}

/// Bilinear resize of an interleaved 8-bit image with `ch` channels.
fn resize(src: &[u8], (sw, sh): (usize, usize), (dw, dh): (usize, usize), ch: usize) -> Vec<u8> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 || src.len() < sw * sh * ch {
        return vec![0; dw * dh * ch];
    }
    let mut out = Vec::with_capacity(dw * dh * ch);
    let at = |x: usize, y: usize, c: usize| f32::from(src.get((y * sw + x) * ch + c).copied().unwrap_or(0));
    for y in 0..dh {
        let fy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, ty) = (fy as usize, fy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..dw {
            let fx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, tx) = (fx as usize, fx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            for c in 0..ch {
                let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                let bot = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                out.push((top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    out
}

/// Round to a multiple of `step` (8 for the SD VAE; 16 for Flux's 2x2 patches), at least `step`.
fn mult(v: f64, step: usize) -> usize {
    ((v / step as f64).round() as usize).max(1) * step
}

// ------------------------------------------------------------------ backends

/// What the model is asked to do.
struct Task {
    prompt: String,
    negative: String,
    model: String,
    steps: u32,
    guidance: f64,
    strength: f64,
    seed: i64,
    /// Size the model works at.
    size: (usize, usize),
    /// PNG of the context window (RGB) and its mask (white = generate), both at `gen` size for
    /// ComfyUI and at window size for A1111 (which resizes itself).
    init_png: Vec<u8>,
    mask_png: Vec<u8>,
    /// Size of `init_png`.
    sent: (u32, u32),
}

/// AUTOMATIC1111 / Forge / SD.Next: one blocking `img2img` call. Returns the result PNG.
fn generate_a1111(ep: &Endpoint, t: &Task, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>> {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    let mut req = json!({
        "init_images": [B64.encode(&t.init_png)],
        "mask": B64.encode(&t.mask_png),
        "prompt": t.prompt,
        "negative_prompt": t.negative,
        "steps": t.steps,
        "cfg_scale": if t.guidance > 0.0 { t.guidance } else { 7.0 },
        "denoising_strength": t.strength,
        "seed": t.seed,
        "width": t.size.0,
        "height": t.size.1,
        "batch_size": 1,
        "n_iter": 1,
        "resize_mode": 0,
        "mask_blur": 4,
        "inpainting_fill": 1,
        "inpaint_full_res": true,
        "inpaint_full_res_padding": 32,
        "inpainting_mask_invert": 0,
        "send_images": true,
        "save_images": false,
    });
    if !t.model.is_empty() {
        req["override_settings"] = json!({"sd_model_checkpoint": t.model});
    }
    let reply = post_json(ep, "/sdapi/v1/img2img", &req, cancelled)?;
    let first = reply
        .get("images")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .ok_or_else(|| other("the model server returned no image"))?;
    // Some servers prefix a data URI.
    let first = first.rsplit(',').next().unwrap_or(first);
    B64.decode(first.trim()).map_err(|e| other(format!("the model's image is not valid base64: {e}")))
}

/// A `multipart/form-data` body uploading `png` as input image `name`.
fn multipart_upload(name: &str, png: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----photocraftgenfill7d3a91";
    let mut body = Vec::new();
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{name}\"\r\nContent-Type: image/png\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(png);
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// A short unique token for upload names and seeds.
fn token() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
}

/// The checkpoints ComfyUI has installed.
fn comfy_checkpoints(ep: &Endpoint, cancelled: &dyn Fn() -> bool) -> Result<Vec<String>> {
    let info = get_json(ep, "/object_info/CheckpointLoaderSimple", cancelled)?;
    Ok(info
        .pointer("/CheckpointLoaderSimple/input/required/ckpt_name/0")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default())
}

/// What ComfyUI loads for a fill.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ComfyModel {
    /// A checkpoint (SD 1.5 or SDXL, ideally an inpainting one).
    Checkpoint(String),
    /// FLUX.1 Fill: a UNet from `models/unet` (GGUF or safetensors); the text encoders and VAE are
    /// separate files ([`FluxParts`]).
    FluxFill { unet: String, gguf: bool },
}

/// Whether `name` is a FLUX.1 Fill model file.
fn is_flux_fill(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("flux") && n.contains("fill")
}

fn comfy_model_named(name: &str) -> ComfyModel {
    if is_flux_fill(name) {
        ComfyModel::FluxFill { unet: name.to_string(), gguf: name.to_ascii_lowercase().ends_with(".gguf") }
    } else {
        ComfyModel::Checkpoint(name.to_string())
    }
}

/// The names a node's required input offers (its first list).
fn input_list(info: &Value, node: &str, input: &str) -> Vec<String> {
    info.pointer(&format!("/{node}/input/required/{input}/0"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// Every model a fill can use: the checkpoints, plus Flux Fill and inpainting UNets. The UNet
/// folders also hold video and other models, which would only mislead, so only those that are
/// meant for filling are listed. A server without GGUF support just has none of those.
fn comfy_models(ep: &Endpoint, cancelled: &dyn Fn() -> bool) -> Result<Vec<String>> {
    let mut names = comfy_checkpoints(ep, cancelled)?;
    for node in ["UnetLoaderGGUF", "UNETLoader"] {
        let Ok(info) = get_json(ep, &format!("/object_info/{node}"), cancelled) else { continue };
        for n in input_list(&info, node, "unet_name") {
            if (is_flux_fill(&n) || n.to_ascii_lowercase().contains("inpaint")) && !names.contains(&n) {
                names.push(n);
            }
        }
    }
    Ok(names)
}

/// What a fill with no `model` would use: Flux Fill, else an inpainting model, else the first.
fn preferred_model(names: &[String]) -> Option<&String> {
    names.iter().find(|n| is_flux_fill(n)).or_else(|| names.iter().find(|n| n.to_ascii_lowercase().contains("inpaint"))).or(names.first())
}

/// The model to use: the requested one, else [`preferred_model`].
fn comfy_pick_model(ep: &Endpoint, wanted: &str, cancelled: &dyn Fn() -> bool) -> Result<ComfyModel> {
    if !wanted.is_empty() {
        return Ok(comfy_model_named(wanted));
    }
    let names = comfy_models(ep, cancelled)?;
    preferred_model(&names).map(|n| comfy_model_named(n)).ok_or_else(|| other("ComfyUI has no models installed; put one in models/checkpoints or pass `model`"))
}

/// The files FLUX.1 Fill needs besides its UNet.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FluxParts {
    clip_l: String,
    t5: String,
    vae: String,
}

/// Find the Flux text encoders and VAE among what ComfyUI has installed.
fn flux_parts(ep: &Endpoint, cancelled: &dyn Fn() -> bool) -> Result<FluxParts> {
    let clips = get_json(ep, "/object_info/DualCLIPLoader", cancelled)?;
    let mut encoders = input_list(&clips, "DualCLIPLoader", "clip_name1");
    encoders.extend(input_list(&clips, "DualCLIPLoader", "clip_name2"));
    let lower = |s: &String| s.to_ascii_lowercase();
    let clip_l = encoders.iter().find(|n| lower(n).contains("clip_l")).cloned();
    // fp8 is much lighter on a small GPU.
    let t5 = encoders
        .iter()
        .find(|n| lower(n).contains("t5xxl") && lower(n).contains("fp8"))
        .or_else(|| encoders.iter().find(|n| lower(n).contains("t5xxl")))
        .cloned();
    let vaes = input_list(&get_json(ep, "/object_info/VAELoader", cancelled)?, "VAELoader", "vae_name");
    let vae = vaes.iter().find(|n| lower(n).contains("flux") || lower(n) == "ae.safetensors").cloned();
    match (clip_l, t5, vae) {
        (Some(clip_l), Some(t5), Some(vae)) => Ok(FluxParts { clip_l, t5, vae }),
        (clip_l, t5, vae) => {
            let missing: Vec<&str> = [
                (clip_l.is_none(), "clip_l.safetensors"),
                (t5.is_none(), "t5xxl_fp8 or t5xxl_fp16 (text_encoders)"),
                (vae.is_none(), "the Flux VAE, flux_vae.safetensors or ae.safetensors (vae)"),
            ]
            .into_iter()
            .filter_map(|(m, n)| m.then_some(n))
            .collect();
            Err(other(format!("FLUX.1 Fill needs files ComfyUI doesn't have: {}", missing.join(", "))))
        }
    }
}

/// The FLUX.1 Fill workflow in API format: the model sees the image and mask directly, takes its
/// guidance as a conditioning value (there is no negative prompt) and samples at `cfg` 1.
fn flux_workflow(unet: &str, gguf: bool, parts: &FluxParts, image: &str, mask: &str, t: &Task) -> Value {
    let guidance = if t.guidance > 0.0 { t.guidance } else { 30.0 };
    let loader = if gguf {
        json!({"class_type": "UnetLoaderGGUF", "inputs": {"unet_name": unet}})
    } else {
        json!({"class_type": "UNETLoader", "inputs": {"unet_name": unet, "weight_dtype": "default"}})
    };
    json!({
        "1": loader,
        "2": {"class_type": "DualCLIPLoader", "inputs": {"clip_name1": parts.clip_l, "clip_name2": parts.t5, "type": "flux"}},
        "3": {"class_type": "VAELoader", "inputs": {"vae_name": parts.vae}},
        "4": {"class_type": "LoadImage", "inputs": {"image": image}},
        "5": {"class_type": "LoadImageMask", "inputs": {"image": mask, "channel": "red"}},
        "6": {"class_type": "CLIPTextEncode", "inputs": {"text": t.prompt, "clip": ["2", 0]}},
        "7": {"class_type": "FluxGuidance", "inputs": {"conditioning": ["6", 0], "guidance": guidance}},
        "8": {"class_type": "ConditioningZeroOut", "inputs": {"conditioning": ["6", 0]}},
        "9": {"class_type": "InpaintModelConditioning", "inputs": {
            "positive": ["7", 0], "negative": ["8", 0], "vae": ["3", 0], "pixels": ["4", 0], "mask": ["5", 0], "noise_mask": true
        }},
        // Blends by the mask's gradation, so a feathered edge is honoured rather than thresholded.
        "10": {"class_type": "DifferentialDiffusion", "inputs": {"model": ["1", 0]}},
        "11": {"class_type": "KSampler", "inputs": {
            "model": ["10", 0], "seed": t.seed, "steps": t.steps, "cfg": 1.0, "sampler_name": "euler", "scheduler": "simple",
            "positive": ["9", 0], "negative": ["9", 1], "latent_image": ["9", 2], "denoise": t.strength
        }},
        "12": {"class_type": "VAEDecode", "inputs": {"samples": ["11", 0], "vae": ["3", 0]}},
        "13": {"class_type": "PreviewImage", "inputs": {"images": ["12", 0]}},
    })
}

/// The ComfyUI inpainting workflow in API format.
fn comfy_workflow(ckpt: &str, image: &str, mask: &str, t: &Task) -> Value {
    json!({
        "1": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": ckpt}},
        "2": {"class_type": "LoadImage", "inputs": {"image": image}},
        "3": {"class_type": "LoadImageMask", "inputs": {"image": mask, "channel": "red"}},
        "4": {"class_type": "CLIPTextEncode", "inputs": {"text": t.prompt, "clip": ["1", 1]}},
        "5": {"class_type": "CLIPTextEncode", "inputs": {"text": t.negative, "clip": ["1", 1]}},
        "6": {"class_type": "InpaintModelConditioning", "inputs": {
            "positive": ["4", 0], "negative": ["5", 0], "vae": ["1", 2], "pixels": ["2", 0], "mask": ["3", 0], "noise_mask": true
        }},
        "7": {"class_type": "KSampler", "inputs": {
            "model": ["1", 0], "seed": t.seed, "steps": t.steps, "cfg": if t.guidance > 0.0 { t.guidance } else { 7.0 }, "sampler_name": "dpmpp_2m", "scheduler": "karras",
            "positive": ["6", 0], "negative": ["6", 1], "latent_image": ["6", 2], "denoise": t.strength
        }},
        "8": {"class_type": "VAEDecode", "inputs": {"samples": ["7", 0], "vae": ["1", 2]}},
        "9": {"class_type": "PreviewImage", "inputs": {"images": ["8", 0]}},
    })
}

/// ComfyUI: upload, queue the workflow, poll the history until it finishes, download the image.
fn generate_comfy(ep: &Endpoint, t: &Task, ctx: &crate::jobs::JobCtx) -> Result<Vec<u8>> {
    let cancelled = &|| ctx.cancelled();
    let model = comfy_pick_model(ep, &t.model, cancelled)?;
    let parts = match &model {
        ComfyModel::FluxFill { .. } => Some(flux_parts(ep, cancelled)?),
        ComfyModel::Checkpoint(_) => None,
    };
    let id = token();
    let upload = |label: &str, png: &[u8]| -> Result<String> {
        let name = format!("photocraft_genfill_{id:x}_{label}.png");
        let (ct, body) = multipart_upload(&name, png);
        let reply = request(ep, "POST", "/upload/image", Some(&ct), &body, cancelled)?;
        let reply: Value = serde_json::from_slice(&reply).map_err(|e| other(format!("ComfyUI sent an unreadable upload reply: {e}")))?;
        let stored = reply.get("name").and_then(Value::as_str).ok_or_else(|| other("ComfyUI didn't confirm the upload"))?;
        Ok(match reply.get("subfolder").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            Some(sub) => format!("{sub}/{stored}"),
            None => stored.to_string(),
        })
    };
    let image = upload("image", &t.init_png)?;
    let mask = upload("mask", &t.mask_png)?;
    ctx.progress(0.1, "Queued in ComfyUI");
    let workflow = match (&model, &parts) {
        (ComfyModel::FluxFill { unet, gguf }, Some(parts)) => flux_workflow(unet, *gguf, parts, &image, &mask, t),
        (ComfyModel::Checkpoint(ckpt), _) => comfy_workflow(ckpt, &image, &mask, t),
        (ComfyModel::FluxFill { .. }, None) => return Err(other("FLUX.1 Fill files weren't looked up")),
    };
    let queued = post_json(ep, "/prompt", &json!({"prompt": workflow, "client_id": "photocraft"}), cancelled)?;
    let prompt_id = queued.get("prompt_id").and_then(Value::as_str).ok_or_else(|| other("ComfyUI didn't accept the workflow"))?.to_string();
    if let Some(errs) = queued.get("node_errors").and_then(Value::as_object).filter(|o| !o.is_empty()) {
        return Err(other(format!("ComfyUI rejected the workflow: {}", Value::Object(errs.clone()).to_string().chars().take(400).collect::<String>())));
    }

    let started = std::time::Instant::now();
    let entry = loop {
        if ctx.cancelled() {
            // Best effort: stop the server working on it.
            let _ = request(ep, "POST", "/interrupt", Some("application/json"), b"{}", &|| false);
            return Err(EngineError::Cancelled);
        }
        if started.elapsed().as_secs() > TIMEOUT_SECS {
            return Err(other("ComfyUI took too long to finish"));
        }
        let hist = get_json(ep, &format!("/history/{}", url_encode(&prompt_id)), cancelled)?;
        if let Some(e) = hist.get(&prompt_id) {
            break e.clone();
        }
        ctx.progress((0.1 + started.elapsed().as_secs_f32() / 120.0).min(0.9), "Generating in ComfyUI");
        for _ in 0..4 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if ctx.cancelled() {
                break;
            }
        }
    };
    if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
        let detail = entry.pointer("/status/messages").map(|m| m.to_string()).unwrap_or_default();
        return Err(other(format!("ComfyUI reported an error: {}", detail.chars().take(400).collect::<String>())));
    }
    let img = entry
        .get("outputs")
        .and_then(Value::as_object)
        .and_then(|o| o.values().find_map(|n| n.get("images").and_then(Value::as_array).and_then(|a| a.first())))
        .ok_or_else(|| other("ComfyUI finished without an image"))?;
    let field = |k: &str| img.get(k).and_then(Value::as_str).unwrap_or("");
    let path = format!(
        "/view?filename={}&subfolder={}&type={}",
        url_encode(field("filename")),
        url_encode(field("subfolder")),
        url_encode(if field("type").is_empty() { "temp" } else { field("type") })
    );
    request(ep, "GET", &path, None, &[], cancelled)
}

// ------------------------------------------------------------------ command

/// What the worker hands to the apply step.
struct Generated {
    /// RGBA, 8 bits, window-sized.
    rgba: Vec<u8>,
    /// How much of the result to keep at each window pixel: the selection's coverage, grown and
    /// feathered at its edge.
    alpha: Vec<f32>,
    /// Pixels the selection itself covers.
    selected: usize,
}

/// In-place mean filter of radius `r` over a `w`×`h` plane (a shrinking window at the edges).
fn box_blur(buf: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 || h == 0 || buf.len() < w * h {
        return;
    }
    let mut prefix = vec![0.0f64; w.max(h) + 1];
    let mut line = vec![0.0f32; w.max(h)];
    // Rows, then columns.
    for pass in 0..2 {
        let (len, lines) = if pass == 0 { (w, h) } else { (h, w) };
        for l in 0..lines {
            let at = |i: usize| if pass == 0 { l * w + i } else { i * w + l };
            for i in 0..len {
                prefix[i + 1] = prefix[i] + f64::from(buf.get(at(i)).copied().unwrap_or(0.0));
            }
            for (i, v) in line.iter_mut().take(len).enumerate() {
                let (lo, hi) = (i.saturating_sub(r), (i + r + 1).min(len));
                *v = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
                if let Some(dst) = buf.get_mut(at(i)) {
                    *dst = *v;
                }
            }
        }
    }
}

/// The selection grown by `r` pixels, and the weight at which the result is blended back: 1 over
/// the selection, falling to 0 over about `r` pixels outside it. With `r == 0` they are the
/// selection and its own coverage. The model fills the grown area, so the fill can blend across
/// the old edge instead of stopping at it.
fn feather(cover: &[f32], w: usize, h: usize, r: usize) -> (Vec<bool>, Vec<f32>) {
    if r == 0 {
        return (cover.iter().map(|c| *c > 0.0).collect(), cover.to_vec());
    }
    let mut grown: Vec<f32> = cover.iter().map(|c| if *c > 0.0 { 1.0 } else { 0.0 }).collect();
    box_blur(&mut grown, w, h, r);
    let mut alpha: Vec<f32> = grown.iter().map(|g| if *g > 1e-4 { 1.0 } else { 0.0 }).collect();
    let is_grown: Vec<bool> = alpha.iter().map(|a| *a > 0.0).collect();
    box_blur(&mut alpha, w, h, r);
    for (a, c) in alpha.iter_mut().zip(cover) {
        *a = a.max(*c).clamp(0.0, 1.0);
    }
    (is_grown, alpha)
}

/// Pull the generated pixels' colour toward what is around them. In the grown band outside the
/// selection the original is still there, so `generated - original` there is the model's colour
/// drift; spread it inward smoothly (a normalised blur of the band's differences) and subtract
/// `strength` of it. A bounded correction keeps a deliberate change, such as a green bush on sand,
/// from being washed out. Needs a band (`feather > 0`).
fn match_colors(rgba: &mut [u8], original_rgb: &[u8], cover: &[f32], grown: &[bool], (w, h): (usize, usize), strength: f32) {
    const MAX_SHIFT: f32 = 0.16;
    let n = w * h;
    if strength <= 0.0 || rgba.len() < n * 4 || original_rgb.len() < n * 3 || cover.len() < n || grown.len() < n {
        return;
    }
    let band: Vec<bool> = (0..n).map(|i| grown[i] && cover[i] <= 0.0).collect();
    if band.iter().filter(|b| **b).count() < 16 {
        return;
    }
    let mut weight: Vec<f32> = band.iter().map(|b| if *b { 1.0 } else { 0.0 }).collect();
    let mut diff: [Vec<f32>; 3] = std::array::from_fn(|c| {
        (0..n).map(|i| if band[i] { (f32::from(rgba[i * 4 + c]) - f32::from(original_rgb[i * 3 + c])) / 255.0 } else { 0.0 }).collect()
    });
    // Wide enough to reach across the whole fill from its rim.
    let radius = (w.min(h) / 3).max(8);
    box_blur(&mut weight, w, h, radius);
    for d in &mut diff {
        box_blur(d, w, h, radius);
    }
    for i in 0..n {
        if !grown[i] || weight[i] < 1e-4 {
            continue;
        }
        for (c, d) in diff.iter().enumerate() {
            let shift = (d[i] / weight[i]).clamp(-MAX_SHIFT, MAX_SHIFT) * strength;
            let v = f32::from(rgba[i * 4 + c]) - shift * 255.0;
            rgba[i * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn can_generate(s: &Session) -> std::result::Result<(), String> {
    crate::edit_menu_cmds::can_caf(s).map_err(|_| "make a selection on a pixel layer to generate into".to_string())
}

/// The backend and the server text to talk to, before the text is checked or resolved: the
/// command parameters, then the environment, then Preferences › Integrations.
fn settings(s: &Session, p: &Value) -> Result<(Backend, String)> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let pick = |key: &str, env_key: &str| p.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty()).map(str::to_string).or_else(|| env(env_key));
    let saved = &s.prefs().integrations;
    let backend = Backend::parse(&pick("backend", BACKEND_ENV).unwrap_or_else(|| saved.generative_fill_backend.name().to_string()))?;
    let host =
        pick("host", HOST_ENV).or_else(|| Some(saved.generative_fill_host.clone()).filter(|h| !h.trim().is_empty())).unwrap_or_else(|| default_host(backend));
    Ok((backend, host))
}

fn server(s: &Session, p: &Value) -> Result<(Backend, Endpoint)> {
    let (backend, host) = settings(s, p)?;
    Ok((backend, parse_endpoint(&host, backend)?))
}

/// Where a fill goes when no host is set: this machine, on the backend's usual port.
fn default_host(backend: Backend) -> String {
    format!("127.0.0.1:{}", backend.default_port())
}

/// Whether a Generative Fill server has been set up: a host in Preferences › Integrations or in
/// the environment. Menus offer the fill only then (it stays listed, greyed, with a note); a
/// command given an explicit `host` still runs without it.
pub fn is_configured(s: &Session) -> bool {
    let set = |v: &str| !v.trim().is_empty();
    std::env::var(HOST_ENV).is_ok_and(|v| set(&v)) || set(&s.prefs().integrations.generative_fill_host)
}

/// The backend (`comfyui` or `a1111`) and host text a fill with these parameters would use, so a
/// dialog can show, and then submit, the very server it asked about (parameters, then the
/// environment, then Preferences). Nothing is resolved or contacted.
pub fn effective_server(s: &Session, p: &Value) -> Result<(String, String)> {
    let (backend, host) = settings(s, p)?;
    Ok((backend.name().to_string(), host))
}

/// The host a blank host field stands for with `backend`: the environment's, else this machine.
pub fn default_host_for(backend: &str) -> Result<String> {
    let backend = Backend::parse(backend)?;
    Ok(std::env::var(HOST_ENV).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| default_host(backend)))
}

/// Ask a server which models it has, with no session: the dialogs run this on a worker thread
/// (it waits on the network), so it takes only the backend and host text. The whole lookup
/// (ComfyUI makes up to three requests) shares one short limit, and it also resolves the host
/// name, so nothing in it blocks the interface. Returns `{backend, host, server, models,
/// default}`: `backend` and `host` echo what was asked, `server` is the address shown to people.
pub fn lookup_models(backend: &str, host: &str) -> Result<Value> {
    const LIMIT: std::time::Duration = std::time::Duration::from_secs(4);
    if cfg!(target_arch = "wasm32") {
        return Err(other("Generative Fill needs a model server and isn't available in the browser build"));
    }
    let backend = Backend::parse(backend)?;
    let mut ep = parse_endpoint(host, backend)?;
    ep.connect_secs = 1;
    ep.total_secs = LIMIT.as_secs();
    let started = std::time::Instant::now();
    let late = || started.elapsed() >= LIMIT;
    let slow =
        |e: EngineError| if matches!(e, EngineError::Cancelled) { other(format!("{} didn't answer within {} seconds", ep.host, LIMIT.as_secs())) } else { e };
    let names = match backend {
        Backend::Comfy => comfy_models(&ep, &late).map_err(slow)?,
        Backend::A1111 => {
            let list = get_json(&ep, "/sdapi/v1/sd-models", &late).map_err(slow)?;
            list.as_array()
                .map(|a| a.iter().filter_map(|m| m.get("title").or_else(|| m.get("model_name")).and_then(Value::as_str).map(str::to_string)).collect())
                .unwrap_or_default()
        }
    };
    // What a fill with no `model` would pick.
    let default = preferred_model(&names).cloned();
    Ok(json!({"backend": backend.name(), "host": host, "server": ep.host, "models": names, "default": default}))
}

/// Edit › Generative Fill › models: the command form of [`lookup_models`], for the server a fill
/// with these parameters would use.
fn list_models(s: &mut Session, p: &Value) -> Result<Value> {
    let (backend, host) = settings(s, p)?;
    lookup_models(backend.name(), &host)
}

fn generative_fill(s: &mut Session, p: &Value) -> Result<Value> {
    fill_as(s, p, "Generative Fill")
}

/// Generative Remove: take the selection out and let the model continue the surroundings. A
/// Generative Fill with a prompt that asks for exactly that, so one click does it; every
/// parameter of the fill can still be given and overrides these defaults.
fn generative_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let mut q = json!({
        "prompt": "empty background, seamless continuation of the surrounding scene",
        "negativePrompt": "object, person, text, logo, watermark, artifacts",
        "feather": 8,
    });
    if let (Some(q), Some(given)) = (q.as_object_mut(), p.as_object()) {
        q.extend(given.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())));
    }
    fill_as(s, &q, "Generative Remove")
}

fn fill_as(s: &mut Session, p: &Value, label: &'static str) -> Result<Value> {
    use crate::retouch_cmds::{SampleLayers, composite_region};
    let id = crate::edit_menu_cmds::pixel_layer(s).map_err(EngineError::Other)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let sel = doc.selection.clone().ok_or_else(|| bad("no selection"))?;
    let hb = sel.content_bounds().intersect(&doc.bounds());
    if hb.is_empty() {
        return Err(bad("the selection is outside the canvas"));
    }

    let prompt = text(p, "prompt", 2000)?;
    let negative = text(p, "negativePrompt", 2000)?;
    let model = text(p, "model", 200)?;
    let steps = num(p, "steps", 25.0, 1.0, 150.0)? as u32;
    let guidance = num(p, "guidance", 0.0, 0.0, 30.0)?;
    let strength = num(p, "strength", 1.0, 0.1, 1.0)?;
    let seed_in = num(p, "seed", -1.0, -1.0, 4_294_967_295.0)? as i64;
    let resolution = num(p, "resolution", 768.0, 256.0, 2048.0)?;
    let feather_px = num(p, "feather", 6.0, 0.0, 32.0)?.round() as usize;
    let color_match = (num(p, "colorMatch", 60.0, 0.0, 100.0)? / 100.0) as f32;
    let output = p.get("output").and_then(Value::as_str).unwrap_or("new").to_string();
    if !matches!(output.as_str(), "new" | "current") {
        return Err(bad("`output` must be new|current"));
    }
    let (backend, ep) = server(s, p)?;
    let model = if model.is_empty() { s.prefs().integrations.generative_fill_model.trim().to_string() } else { model };
    // A random seed, fixed here so a rerun with the same params is reproducible from the result.
    let seed = if seed_in < 0 { (token() % 4_294_967_295) as i64 } else { seed_in };

    // The selection plus context to match lighting and texture, clamped to the canvas.
    let context = (hb.width().max(hb.height()) / 2).max(64) as i32;
    let window = hb.inflate(context).intersect(&doc.bounds());
    let (ww, wh) = (window.width(), window.height());
    if ww > MAX_WINDOW_SIDE || wh > MAX_WINDOW_SIDE {
        return Err(bad(format!("the selection and its surroundings are larger than {MAX_WINDOW_SIDE}px; select a smaller area")));
    }
    let surf = doc.layer(id).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(id))?;
    let fmt = surf.format();
    // ComfyUI works on exactly what it is sent, so the window is scaled to `resolution` on its
    // long side. A1111 crops around the mask and rescales itself, so it gets the window as is,
    // plus the size to generate the masked area at.
    let (wu, hu) = (ww as usize, wh as usize);
    let (size, sent) = match backend {
        Backend::Comfy => {
            let f = resolution / f64::from(ww.max(wh)).max(1.0);
            let g = (mult(f64::from(ww) * f, 16), mult(f64::from(wh) * f, 16));
            (g, g)
        }
        Backend::A1111 => {
            let f = resolution / f64::from(hb.width().max(hb.height())).max(1.0);
            ((mult(f64::from(hb.width()) * f, 8), mult(f64::from(hb.height()) * f, 8)), (wu, hu))
        }
    };

    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            use photocraft_codecs::ChannelLayout;

            use crate::fill_cmds::read_window_mask;
            ctx.progress(0.0, "Preparing the selection");
            let cancel = || ctx.cancelled();
            let n = fmt.channels();
            let region = composite_region(&doc, None, SampleLayers::All, window, fmt);
            ctx.check()?;
            let cover = read_window_mask(&sel, window, &cancel).ok_or(EngineError::Cancelled)?;
            let count = wu * hu;
            if region.data.len() < count * n || cover.len() < count {
                return Err(other("couldn't read the selection's surroundings"));
            }
            let mut rgb = Vec::with_capacity(count * 3);
            for px in region.data.chunks_exact(n).take(count) {
                let c = to_rgba(&fmt, px);
                rgb.extend(c.iter().take(3).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
            let selected = cover.iter().take(count).filter(|c| **c > 0.0).count();
            let (grown, alpha) = feather(cover.get(..count).unwrap_or_default(), wu, hu, feather_px);
            // What the model regenerates is the selection grown by the feather radius.
            let mut mask: Vec<u8> = grown.iter().map(|g| if *g { 255 } else { 0 }).collect();
            let original_rgb = if color_match > 0.0 && feather_px > 0 { rgb.clone() } else { Vec::new() };
            let (rgb, mask) = if sent == (wu, hu) {
                (rgb, mask)
            } else {
                mask = resize(&mask, (wu, hu), sent, 1);
                (resize(&rgb, (wu, hu), sent, 3), mask)
            };
            let (sw, sh) = (sent.0 as u32, sent.1 as u32);
            let mask_rgb: Vec<u8> = mask.iter().flat_map(|m| [*m, *m, *m]).collect();
            let task = Task {
                prompt,
                negative,
                model,
                steps,
                guidance,
                strength,
                seed,
                size,
                init_png: png(sw, sh, ChannelLayout::Rgb, rgb)?,
                mask_png: match backend {
                    Backend::Comfy => png(sw, sh, ChannelLayout::Rgb, mask_rgb)?,
                    Backend::A1111 => png(sw, sh, ChannelLayout::Gray, mask)?,
                },
                sent: (sw, sh),
            };
            ctx.progress(0.05, "Waiting for the model server");
            let bytes = match backend {
                Backend::A1111 => generate_a1111(&ep, &task, &cancel)?,
                Backend::Comfy => generate_comfy(&ep, &task, ctx)?,
            };
            ctx.check()?;
            let rgba = decode_rgba(&bytes, task.sent.0, task.sent.1)?;
            let mut rgba = if sent == (wu, hu) { rgba } else { resize(&rgba, sent, (wu, hu), 4) };
            match_colors(&mut rgba, &original_rgb, &cover, &grown, (wu, hu), color_match);
            ctx.progress(1.0, label);
            Ok(Generated { rgba, alpha, selected })
        },
        move |s, done| apply(s, label, id, &output, fmt, window, done),
    )
}

/// Write the generated pixels over the selection (weighted by its coverage) as one undo step.
fn apply(s: &mut Session, label: &str, id: LayerId, output: &str, fmt: PixelFormat, window: Rect, done: Generated) -> Result<Value> {
    let n = fmt.channels();
    let Generated { rgba, alpha, selected } = done;
    let generated = |i: usize| -> [f32; 4] {
        let b = |k: usize| f32::from(rgba.get(i * 4 + k).copied().unwrap_or(0)) / 255.0;
        [b(0), b(1), b(2), b(3)]
    };
    let layer = s.edit(label, |doc, active| {
        if output == "new" {
            let mut l = Layer::raster(doc.next_layer_name(label), PixelFormat { alpha: true, ..fmt });
            let lf = l.surface().map(|s| s.format()).unwrap_or(fmt);
            let ln = lf.channels();
            let mut px = vec![0.0f32; alpha.len() * ln];
            let mut enc = [0.0f32; 8];
            for (i, c) in alpha.iter().enumerate().filter(|(_, c)| **c > 0.001) {
                let g = generated(i);
                from_rgba_into(&lf, [g[0], g[1], g[2], g[3] * c], &mut enc);
                if let Some(dst) = px.get_mut(i * ln..(i + 1) * ln) {
                    dst.copy_from_slice(enc.get(..ln).unwrap_or_default());
                }
            }
            if let Some(sf) = l.surface_mut() {
                sf.write_region(window, &px);
                sf.prune();
            }
            let nid = doc.insert_above(Some(id), l);
            *active = Some(nid);
            return Ok(nid);
        }
        let surf = crate::edit_menu_cmds::writable_surface(doc, id)?;
        let mut px = Vec::new();
        surf.read_region_into(window, &mut px);
        if px.len() != alpha.len() * n {
            return Err(other("the layer changed while the model was running"));
        }
        let mut enc = [0.0f32; 8];
        for (i, c) in alpha.iter().enumerate().filter(|(_, c)| **c > 0.001) {
            let g = generated(i);
            let old = to_rgba(&fmt, px.get(i * n..(i + 1) * n).unwrap_or_default());
            let mix = |k: usize| old[k] + (g[k] - old[k]) * c;
            from_rgba_into(&fmt, [mix(0), mix(1), mix(2), mix(3).max(*c)], &mut enc);
            if let Some(dst) = px.get_mut(i * n..(i + 1) * n) {
                dst.copy_from_slice(enc.get(..n).unwrap_or_default());
            }
        }
        surf.write_region(window, &px);
        Ok(id)
    })?;
    Ok(json!({"layer": layer.0, "generated": selected, "window": [window.x0, window.y0, window.width(), window.height()]}))
}

/// Parameters of `edit.generativeFill`. The leading object drives the dialog (`text` fields are
/// text boxes); what follows is documentation. `backend`, `host` and `model` default to
/// Preferences › Integrations, then the environment (`PHOTOCRAFT_GENFILL_BACKEND`,
/// `PHOTOCRAFT_GENFILL_URL`). `host` is `192.168.1.20:8188` or `http://myserver.local:8188`, on
/// this machine or a private network only. An empty `model` is the server's first checkpoint,
/// preferring an inpainting one.
const FILL_PARAMS: &str = r##"{"prompt":text,"negativePrompt":text,"backend":"comfyui|a1111","host":text,"model":text,"steps":1..150=25,"guidance":0..30=0,"strength":0.1..1=1,"resolution":256..2048=768,"seed":-1..4294967295=-1,"feather":0..32=6,"colorMatch":0..100=60,"output":"new|current"}"##;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "edit.generativeFill",
            label: "Generative Fill…",
            menu: &[],
            shortcut: None,
            params: FILL_PARAMS,
            enabled: can_generate,
            run: generative_fill,
            journal: true,
        },
        CommandSpec {
            id: "edit.generativeRemove",
            label: "Generative Remove",
            menu: &[],
            shortcut: None,
            // The same parameters as the fill; the prompt, negative prompt and feather default to
            // removal values.
            params: FILL_PARAMS,
            enabled: can_generate,
            run: generative_remove,
            journal: true,
        },
        CommandSpec {
            id: "edit.generativeFill.models",
            label: "Generative Fill Models",
            menu: &[],
            shortcut: None,
            params: r##"{"backend":"comfyui|a1111","host":text} → {"host","models":[name],"default":name?}"##,
            enabled: |_| Ok(()),
            run: list_models,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
