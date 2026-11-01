use super::*;
use base64::Engine as _;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

type Seen = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 80, "depth": 8, "background": "white"})).unwrap();
    s
}

fn solid_png(w: u32, h: u32, colour: [u8; 3]) -> Vec<u8> {
    png(w, h, photocraft_codecs::ChannelLayout::Rgb, (0..w * h).flat_map(|_| colour).collect()).unwrap()
}

/// A fake model server. `handler(method, path, body)` answers each request with a status and
/// body; the server stops after a request whose path starts with `last` (or after one error).
/// Replies are chunked, to exercise the decoder. Returns the host and the requests it saw.
fn fake_server(last: &'static str, handler: impl Fn(&str, &str, &[u8]) -> (u16, Vec<u8>) + Send + 'static) -> (String, std::thread::JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let h = std::thread::spawn(move || {
        let seen: Seen = Arc::default();
        loop {
            let Ok((mut sock, _)) = listener.accept() else { break };
            let mut raw = Vec::new();
            let mut buf = [0u8; 16384];
            let (head_end, len) = loop {
                let n = sock.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..n]);
                if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..i]).to_ascii_lowercase();
                    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).map_or(0, |v| v.trim().parse().unwrap());
                    break (i + 4, len);
                }
            };
            while raw.len() < head_end + len {
                let n = sock.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..n]);
            }
            let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
            let mut first = head.lines().next().unwrap().split_whitespace();
            let (method, path) = (first.next().unwrap().to_string(), first.next().unwrap().to_string());
            let body = raw[head_end..head_end + len].to_vec();
            let (status, payload) = handler(&method, &path, &body);
            seen.lock().unwrap().push((method, path.clone(), body));
            let (a, b) = payload.split_at(payload.len() / 2);
            let mut resp = format!("HTTP/1.1 {status} X\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", a.len()).into_bytes();
            resp.extend_from_slice(a);
            resp.extend_from_slice(format!("\r\n{:x}\r\n", b.len()).as_bytes());
            resp.extend_from_slice(b);
            resp.extend_from_slice(b"\r\n0\r\n\r\n");
            sock.write_all(&resp).unwrap();
            if path.starts_with(last) {
                break;
            }
        }
        seen
    });
    (host, h)
}

fn a1111_server(colour: [u8; 3]) -> (String, std::thread::JoinHandle<Seen>) {
    fake_server("/sdapi/v1/img2img", move |_, _, body| {
        let req: Value = serde_json::from_slice(body).unwrap();
        let init = base64::engine::general_purpose::STANDARD.decode(req["init_images"][0].as_str().unwrap()).unwrap();
        let (w, h) = photocraft_codecs::decode(&init).unwrap().dimensions();
        (200, json!({"images": [base64::engine::general_purpose::STANDARD.encode(solid_png(w, h, colour))]}).to_string().into_bytes())
    })
}

/// A fake ComfyUI for the 96x80 test document: the context window is the whole canvas, which is
/// sent at 768x640 with the default resolution.
fn comfy_server(colour: [u8; 3]) -> (String, std::thread::JoinHandle<Seen>) {
    let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    fake_server("/view", move |method, path, _| match (method, path) {
        ("GET", p) if p.starts_with("/object_info/CheckpointLoaderSimple") => (
            200,
            json!({"CheckpointLoaderSimple": {"input": {"required": {"ckpt_name": [["sd15.safetensors", "sd15-inpainting.safetensors"]]}}}})
                .to_string()
                .into_bytes(),
        ),
        ("POST", "/upload/image") => (200, json!({"name": "up.png", "subfolder": "", "type": "input"}).to_string().into_bytes()),
        ("POST", "/prompt") => (200, json!({"prompt_id": "abc", "node_errors": {}}).to_string().into_bytes()),
        ("GET", "/history/abc") => {
            // Still running on the first poll.
            if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                (200, b"{}".to_vec())
            } else {
                (200, json!({"abc": {"status": {"status_str": "success"}, "outputs": {"9": {"images": [{"filename": "f.png", "subfolder": "", "type": "temp"}]}}}}).to_string().into_bytes())
            }
        }
        ("GET", p) if p.starts_with("/view?filename=f.png") => (200, solid_png(768, 640, colour)),
        _ => (404, b"unexpected".to_vec()),
    })
}

fn rect_selection(s: &mut Session) {
    s.execute("select.rect", json!({"x": 30, "y": 25, "width": 20, "height": 20})).unwrap();
}

fn px(s: &Session, x: i32, y: i32) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().pixel(x, y)
}

/// What a rejected command must leave alone: the layers and the history.
fn state(s: &Session) -> (usize, usize, Option<String>) {
    let st = s.active().unwrap();
    (layer_count(s), st.history.past_len(), st.history.undo_label().map(str::to_string))
}

fn layer_count(s: &Session) -> usize {
    s.active().unwrap().doc.walk().len()
}

#[test]
fn needs_a_selection() {
    let mut s = session();
    assert!(!s.is_enabled("edit.generativeFill"));
    rect_selection(&mut s);
    assert!(s.is_enabled("edit.generativeFill"));
}

#[test]
fn a1111_fills_into_a_new_layer_and_undoes() {
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = a1111_server([255, 0, 0]);
    let before = layer_count(&s);
    let r = s.execute("edit.generativeFill", json!({"backend": "a1111", "prompt": "a red door", "host": host, "steps": 4, "colorMatch": 0})).unwrap();
    assert_eq!(r["generated"].as_u64().unwrap(), 400, "{r}");
    let seen = server.join().unwrap();
    let req: Value = serde_json::from_slice(&seen.lock().unwrap()[0].2).unwrap();
    assert_eq!(req["prompt"], "a red door");
    assert_eq!(req["steps"], 4);
    assert_eq!(req["inpaint_full_res"], true);
    assert_eq!(layer_count(&s), before + 1);
    let inside = px(&s, 40, 35);
    assert!(inside[0] > 0.95 && inside[1] < 0.05, "{inside:?}");
    // Outside the selection the new layer is transparent.
    assert_eq!(px(&s, 5, 5).last().copied().unwrap_or(0.0), 0.0);
    s.undo();
    assert_eq!(layer_count(&s), before);
}

#[test]
fn comfyui_runs_the_whole_queue_flow() {
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = comfy_server([0, 255, 0]);
    let before = layer_count(&s);
    let r = s
        .execute(
            "edit.generativeFill",
            json!({"backend": "comfyui", "prompt": "grass", "host": format!("http://{host}"), "steps": 5, "seed": 7, "colorMatch": 0}),
        )
        .unwrap();
    assert_eq!(r["generated"].as_u64().unwrap(), 400, "{r}");
    let seen = server.join().unwrap();
    let seen = seen.lock().unwrap();
    let paths: Vec<&str> = seen.iter().map(|(_, p, _)| p.as_str()).collect();
    assert_eq!(paths[0], "/object_info/CheckpointLoaderSimple");
    assert_eq!(paths.iter().filter(|p| **p == "/upload/image").count(), 2, "image and mask: {paths:?}");
    assert_eq!(paths.iter().filter(|p| p.starts_with("/history/abc")).count(), 2, "polled until done: {paths:?}");
    let queued: Value = serde_json::from_slice(&seen.iter().find(|(_, p, _)| p == "/prompt").unwrap().2).unwrap();
    let wf = &queued["prompt"];
    assert_eq!(wf["1"]["inputs"]["ckpt_name"], "sd15-inpainting.safetensors", "prefers an inpainting checkpoint");
    assert_eq!(wf["4"]["inputs"]["text"], "grass");
    assert_eq!(wf["7"]["inputs"]["steps"], 5);
    assert_eq!(wf["7"]["inputs"]["seed"], 7);
    assert_eq!(layer_count(&s), before + 1);
    let inside = px(&s, 40, 35);
    assert!(inside[1] > 0.95 && inside[0] < 0.05, "{inside:?}");
    assert_eq!(px(&s, 5, 5).last().copied().unwrap_or(0.0), 0.0);
}

#[test]
fn current_output_only_changes_selected_pixels() {
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = a1111_server([0, 0, 255]);
    s.execute("edit.generativeFill", json!({"backend": "a1111", "host": host, "output": "current", "colorMatch": 0})).unwrap();
    server.join().unwrap();
    let inside = px(&s, 40, 35);
    assert!(inside[2] > 0.95 && inside[0] < 0.05, "{inside:?}");
    assert!(px(&s, 5, 5)[0] > 0.99, "outside must stay white");
}

#[test]
fn only_local_network_hosts_are_allowed() {
    let mut s = session();
    rect_selection(&mut s);
    let before = state(&s);
    for host in
        ["http://8.8.8.8:8188", "https://127.0.0.1:8188", "ftp://127.0.0.1", "http://user@127.0.0.1", "http://", "http://[::1", "1.1.1.1", "127.0.0.1:0"]
    {
        assert!(s.execute("edit.generativeFill", json!({"host": host})).is_err(), "{host}");
        assert_eq!(state(&s), before, "{host}: a refused host changes nothing");
    }
    let err = parse_endpoint("http://8.8.8.8:8188", Backend::Comfy).unwrap_err().to_string();
    assert!(err.contains("local network"), "{err}");
    for ok in [
        "192.168.1.20:8188",
        "http://10.0.0.5",
        "172.16.3.4:9000",
        "http://169.254.1.1",
        "100.100.1.1:8188",
        "localhost",
        "http://[::1]:8188",
        "http://[fd00::1]:8188",
        "http://127.0.0.1:8188/sub/",
    ] {
        assert!(parse_endpoint(ok, Backend::Comfy).is_ok(), "{ok}");
    }
    // The port defaults to the backend's; a mount path is kept.
    assert_eq!(parse_endpoint("192.168.1.20", Backend::Comfy).unwrap().addr.port(), 8188);
    assert_eq!(parse_endpoint("192.168.1.20", Backend::A1111).unwrap().addr.port(), 7860);
    assert_eq!(parse_endpoint("127.0.0.1:8188/sub/", Backend::Comfy).unwrap().base, "/sub");
    assert!(Backend::parse("forge").is_ok() && Backend::parse("ComfyUI").is_ok() && Backend::parse("dalle").is_err());
}

#[test]
fn reports_server_errors_and_bad_params() {
    let mut s = session();
    rect_selection(&mut s);
    let before = state(&s);
    let (host, server) = fake_server("/", |_, _, _| (500, b"{\"error\":\"out of memory\"}".to_vec()));
    let err = s.execute("edit.generativeFill", json!({"backend": "a1111", "host": host})).unwrap_err().to_string();
    server.join().unwrap();
    assert!(err.contains("500") && err.contains("out of memory"), "{err}");
    assert_eq!(state(&s), before, "a server error changes nothing");
    // Nothing listening.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let host = closed.local_addr().unwrap().to_string();
    drop(closed);
    let err = s.execute("edit.generativeFill", json!({"host": host})).unwrap_err().to_string();
    assert!(err.contains("reach"), "{err}");
    assert_eq!(state(&s), before, "an unreachable server changes nothing");
    for p in [json!({"steps": 0}), json!({"steps": "x"}), json!({"output": 3}), json!({"prompt": 5}), json!({"resolution": 1e30}), json!({"backend": "dalle"})]
    {
        assert!(s.execute("edit.generativeFill", p.clone()).is_err(), "{p}");
        assert_eq!(state(&s), before, "{p}: rejected parameters change nothing");
    }
}

#[test]
fn http_parsing_survives_garbage() {
    for raw in [
        &b""[..],
        b"HTTP/1.1",
        b"HTTP/1.1 200 OK\r\n\r\n",
        b"HTTP/1.1 abc\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffff\r\nab",
    ] {
        let _ = parse_response(raw);
    }
    assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\n\r\nhi").unwrap(), b"hi");
    assert!(decode_rgba(b"not a png", 4, 4).is_err());
    assert!(resize(&[], (0, 0), (4, 4), 3).iter().all(|v| *v == 0));
    assert_eq!(resize(&[10, 20, 30, 40], (2, 2), (4, 4), 1).len(), 16);
    assert_eq!(url_encode("a b/c?"), "a%20b%2Fc%3F");
}

#[test]
fn saved_preferences_supply_the_backend_host_and_model() {
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = comfy_server([0, 0, 255]);
    s.execute("prefs.set", json!({"path": "integrations.generativeFillBackend", "value": "comfyui"})).unwrap();
    s.execute("prefs.set", json!({"path": "integrations.generativeFillHost", "value": host})).unwrap();
    s.execute("prefs.set", json!({"path": "integrations.generativeFillModel", "value": "mine.safetensors"})).unwrap();
    assert!(s.execute("prefs.set", json!({"path": "integrations.generativeFillBackend", "value": "dalle"})).is_err());
    // No host, backend or model in the command: the saved ones are used.
    s.execute("edit.generativeFill", json!({"prompt": "sky"})).unwrap();
    let seen = server.join().unwrap();
    let seen = seen.lock().unwrap();
    let queued: Value = serde_json::from_slice(&seen.iter().find(|(_, p, _)| p == "/prompt").unwrap().2).unwrap();
    assert_eq!(queued["prompt"]["1"]["inputs"]["ckpt_name"], "mine.safetensors");
    assert!(seen.iter().all(|(_, p, _)| !p.starts_with("/object_info")), "a saved model needs no lookup");
}

#[test]
fn lists_the_servers_models_for_both_backends() {
    let mut s = session();
    let (host, server) = fake_server("/object_info", |_, _, _| {
        (
            200,
            json!({"CheckpointLoaderSimple": {"input": {"required": {"ckpt_name": [["a.safetensors", "b-inpainting.safetensors"]]}}}}).to_string().into_bytes(),
        )
    });
    let r = s.execute("edit.generativeFill.models", json!({"backend": "comfyui", "host": host})).unwrap();
    server.join().unwrap();
    assert_eq!(r["models"], json!(["a.safetensors", "b-inpainting.safetensors"]));
    assert_eq!(r["default"], "b-inpainting.safetensors");

    let (host, server) = fake_server("/sdapi/v1/sd-models", |_, _, _| {
        (200, json!([{"title": "sd15.safetensors [abc]", "model_name": "sd15"}, {"model_name": "only-name"}]).to_string().into_bytes())
    });
    let r = s.execute("edit.generativeFill.models", json!({"backend": "a1111", "host": host})).unwrap();
    server.join().unwrap();
    assert_eq!(r["models"], json!(["sd15.safetensors [abc]", "only-name"]));
    assert_eq!(r["default"], "sd15.safetensors [abc]");
    // Not a command the history records, and it works with nothing selected.
    assert!(s.active().unwrap().history.undo_label().is_none_or(|l| l != "Generative Fill Models"));
}

#[test]
fn listing_models_fails_fast_and_clearly() {
    let mut s = session();
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let host = closed.local_addr().unwrap().to_string();
    drop(closed);
    let started = std::time::Instant::now();
    let err = s.execute("edit.generativeFill.models", json!({"host": host})).unwrap_err().to_string();
    assert!(err.contains("reach") && started.elapsed().as_secs() < 5, "{err}");
    assert!(s.execute("edit.generativeFill.models", json!({"host": "8.8.8.8"})).is_err());
}

#[test]
fn the_dialog_spec_has_text_boxes_for_text_params() {
    // `text` params must be strings (a number there is the "must be text" error); the dialog
    // reads this notation, so keep it free of anything that parses as a number or a menu.
    let spec = FILL_PARAMS;
    for key in ["prompt", "negativePrompt", "host", "model"] {
        assert!(spec.contains(&format!("\"{key}\":text")), "{key}");
    }
    assert!(!spec.contains("text=\"\""));
    let r = Session::new().execute("edit.generativeFill", json!({"prompt": 0}));
    assert!(r.is_err());
}

#[test]
fn box_blur_averages_and_keeps_flat_planes_flat() {
    let mut flat = vec![0.7f32; 9 * 7];
    box_blur(&mut flat, 9, 7, 3);
    assert!(flat.iter().all(|v| (v - 0.7).abs() < 1e-5));
    let mut dot = vec![0.0f32; 9 * 9];
    dot[4 * 9 + 4] = 9.0 * 9.0;
    box_blur(&mut dot, 9, 9, 1);
    assert!((dot[4 * 9 + 4] - 9.0).abs() < 1e-4 && (dot[3 * 9 + 3] - 9.0).abs() < 1e-4 && dot[0] == 0.0, "{:?}", &dot[..20]);
    // Degenerate sizes and radii are no-ops, not panics.
    box_blur(&mut [], 0, 0, 5);
    box_blur(&mut [1.0], 4, 4, 5);
}

/// A 20x20 selection in a 96x80 window.
fn square_cover() -> Vec<f32> {
    (0..96 * 80).map(|i| if (30..50).contains(&(i % 96)) && (25..45).contains(&(i / 96)) { 1.0 } else { 0.0 }).collect()
}

#[test]
fn feathering_grows_the_mask_and_fades_the_edge() {
    let cover = square_cover();
    let (grown, alpha) = feather(&cover, 96, 80, 6);
    let at = |x: usize, y: usize| y * 96 + x;
    assert!(grown[at(40, 35)] && grown[at(25, 35)], "grown past the edge");
    assert!(!grown[at(10, 35)]);
    assert!(alpha[at(40, 35)] > 0.99 && alpha[at(30, 35)] > 0.99, "full strength over the selection");
    let band = alpha[at(27, 35)];
    assert!(band > 0.0 && band < 1.0, "soft outside the edge: {band}");
    assert_eq!(alpha[at(5, 5)], 0.0);
    assert!((0..96 * 80).all(|i| alpha[i] >= cover[i] - 1e-6), "never less than the selection's own coverage");
    // No feather: the selection exactly.
    let (grown, alpha) = feather(&cover, 96, 80, 0);
    assert_eq!(alpha, cover);
    assert_eq!(grown.iter().filter(|g| **g).count(), 400);
}

#[test]
fn colour_match_removes_the_drift_seen_at_the_rim_but_is_bounded() {
    let (w, h) = (96usize, 80usize);
    let cover = square_cover();
    let (grown, _) = feather(&cover, w, h, 6);
    let original = vec![100u8; w * h * 3];
    // The model drifted +30 in red everywhere, and painted a very different interior.
    let mut rgba: Vec<u8> = (0..w * h).flat_map(|i| if cover[i] > 0.0 { [230, 20, 20, 255] } else { [130, 100, 100, 255] }).collect();
    let untouched = rgba.clone();
    match_colors(&mut rgba, &original, &cover, &grown, (w, h), 0.0);
    assert_eq!(rgba, untouched, "strength 0 changes nothing");
    match_colors(&mut rgba, &original, &cover, &grown, (w, h), 1.0);
    let i = (35 * w + 40) * 4;
    assert!((rgba[i] as i32 - 200).abs() <= 1, "red drift of 30 removed: {}", rgba[i]);
    assert_eq!((rgba[i + 1], rgba[i + 2]), (20, 20), "no drift in green or blue, so nothing to remove");
    // A huge difference is capped, so intentional changes survive.
    let mut rgba: Vec<u8> = (0..w * h).flat_map(|_| [255, 100, 100, 255]).collect();
    match_colors(&mut rgba, &original, &cover, &grown, (w, h), 1.0);
    assert!(rgba[i] >= 255 - 42, "{}", rgba[i]);
    // Outside the grown area nothing moves; and with no band (no feather) nothing happens.
    assert_eq!(rgba[(5 * w + 5) * 4], 255);
    let (g0, _) = feather(&cover, w, h, 0);
    let mut rgba: Vec<u8> = (0..w * h).flat_map(|_| [200, 100, 100, 255]).collect();
    match_colors(&mut rgba, &original, &cover, &g0, (w, h), 1.0);
    assert!(rgba.chunks(4).all(|p| p[0] == 200));
    // Short buffers are ignored.
    match_colors(&mut [0u8; 4], &[], &cover, &grown, (w, h), 1.0);
}

#[test]
fn the_fill_fades_out_past_the_selection_edge() {
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = a1111_server([255, 0, 0]);
    s.execute("edit.generativeFill", json!({"backend": "a1111", "host": host, "feather": 6, "colorMatch": 0})).unwrap();
    let seen = server.join().unwrap();
    let req: Value = serde_json::from_slice(&seen.lock().unwrap()[0].2).unwrap();
    // The mask the model sees is the selection grown by the feather radius: 20 + 2*6 square.
    let mask = base64::engine::general_purpose::STANDARD.decode(req["mask"].as_str().unwrap()).unwrap();
    let mask = photocraft_codecs::decode(&mask).unwrap();
    let white = mask.data().iter().filter(|v| **v == 255).count();
    assert_eq!(white, 32 * 32, "{white}");
    // Fully red inside, partly red just outside, none far away.
    assert!(px(&s, 40, 35)[0] > 0.95);
    let rim = px(&s, 27, 35);
    assert!(*rim.last().unwrap() > 0.0 && *rim.last().unwrap() < 1.0, "{rim:?}");
    assert_eq!(*px(&s, 5, 5).last().unwrap(), 0.0);
    // Feather 0 keeps the old hard edge.
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = a1111_server([255, 0, 0]);
    s.execute("edit.generativeFill", json!({"backend": "a1111", "host": host, "feather": 0})).unwrap();
    server.join().unwrap();
    assert_eq!(*px(&s, 27, 35).last().unwrap(), 0.0);
    assert!(*px(&s, 31, 35).last().unwrap() > 0.99);
}

/// A fake ComfyUI that has a Flux Fill GGUF, the Flux text encoders and VAE, and also video
/// models in its unet folder, which must not be offered for filling.
fn flux_server(colour: [u8; 3]) -> (String, std::thread::JoinHandle<Seen>) {
    let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    fake_server("/view", move |method, path, _| {
        let list = |node: &str, input: &str, names: &[&str]| (200, json!({ node: {"input": {"required": {input: [names]}}} }).to_string().into_bytes());
        match (method, path) {
            ("GET", "/object_info/CheckpointLoaderSimple") => list("CheckpointLoaderSimple", "ckpt_name", &["sdxl.safetensors"]),
            ("GET", "/object_info/UnetLoaderGGUF") => {
                list("UnetLoaderGGUF", "unet_name", &["flux1-dev-Q4_K_S.gguf", "flux1-fill-dev-Q4_K_S.gguf", "Wan2.2-Animate-14B-Q4_K_M.gguf"])
            }
            ("GET", "/object_info/UNETLoader") => list("UNETLoader", "unet_name", &["wan2.1_t2v_1.3B_fp16.safetensors", "my-sdxl-inpainting-unet.safetensors"]),
            ("GET", "/object_info/DualCLIPLoader") => (
                200,
                json!({"DualCLIPLoader": {"input": {"required": {
                    "clip_name1": [["clip_l.safetensors", "t5xxl_fp16.safetensors", "umt5_xxl_fp8_e4m3fn_scaled.safetensors"]],
                    "clip_name2": [["clip_l.safetensors", "t5xxl_fp16.safetensors", "umt5_xxl_fp8_e4m3fn_scaled.safetensors"]]}}}})
                .to_string()
                .into_bytes(),
            ),
            ("GET", "/object_info/VAELoader") => list("VAELoader", "vae_name", &["sdxl_vae.safetensors", "wan_2.1_vae.safetensors", "flux_vae.safetensors"]),
            ("POST", "/upload/image") => (200, json!({"name": "up.png", "subfolder": "", "type": "input"}).to_string().into_bytes()),
            ("POST", "/prompt") => (200, json!({"prompt_id": "abc", "node_errors": {}}).to_string().into_bytes()),
            ("GET", "/history/abc") => {
                if polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    (200, b"{}".to_vec())
                } else {
                    (200, json!({"abc": {"status": {"status_str": "success"}, "outputs": {"13": {"images": [{"filename": "f.png", "subfolder": "", "type": "temp"}]}}}}).to_string().into_bytes())
                }
            }
            ("GET", p) if p.starts_with("/view?filename=f.png") => (200, solid_png(768, 640, colour)),
            _ => (404, b"unexpected".to_vec()),
        }
    })
}

#[test]
fn comfyui_prefers_flux_fill_and_builds_its_workflow() {
    let mut s = session();
    rect_selection(&mut s);
    let (host, server) = flux_server([10, 200, 10]);
    // No model given: Flux Fill is picked over the checkpoint, and sized in multiples of 16.
    let r = s.execute("edit.generativeFill", json!({"backend": "comfyui", "host": host, "prompt": "bushes", "steps": 20, "seed": 3, "colorMatch": 0})).unwrap();
    assert_eq!(r["generated"].as_u64().unwrap(), 400, "{r}");
    let seen = server.join().unwrap();
    let seen = seen.lock().unwrap();
    let queued: Value = serde_json::from_slice(&seen.iter().find(|(_, p, _)| p == "/prompt").unwrap().2).unwrap();
    let wf = &queued["prompt"];
    assert_eq!(wf["1"]["class_type"], "UnetLoaderGGUF");
    assert_eq!(wf["1"]["inputs"]["unet_name"], "flux1-fill-dev-Q4_K_S.gguf");
    assert_eq!(wf["2"]["inputs"]["clip_name1"], "clip_l.safetensors");
    assert_eq!(wf["2"]["inputs"]["clip_name2"], "t5xxl_fp16.safetensors", "t5xxl, not the video model's umt5");
    assert_eq!(wf["2"]["inputs"]["type"], "flux");
    assert_eq!(wf["3"]["inputs"]["vae_name"], "flux_vae.safetensors");
    assert_eq!(wf["6"]["inputs"]["text"], "bushes");
    assert_eq!(wf["7"]["class_type"], "FluxGuidance");
    assert_eq!(wf["7"]["inputs"]["guidance"], 30.0, "automatic guidance for Flux");
    assert_eq!(wf["8"]["class_type"], "ConditioningZeroOut");
    assert_eq!(wf["11"]["inputs"]["cfg"], 1.0);
    assert_eq!(wf["11"]["inputs"]["steps"], 20);
    assert_eq!(wf["11"]["inputs"]["seed"], 3);
    // Everything the sampler uses comes from the Flux nodes.
    assert_eq!(wf["11"]["inputs"]["model"], json!(["10", 0]));
    assert_eq!(wf["10"]["inputs"]["model"], json!(["1", 0]));
    let inside = px(&s, 40, 35);
    assert!(inside[1] > 0.7 && inside[0] < 0.1, "{inside:?}");
}

#[test]
fn flux_guidance_can_be_set_and_safetensors_unets_use_the_plain_loader() {
    let t = Task {
        prompt: String::new(),
        negative: String::new(),
        model: String::new(),
        steps: 20,
        guidance: 12.5,
        strength: 1.0,
        seed: 1,
        size: (64, 64),
        init_png: vec![],
        mask_png: vec![],
        sent: (64, 64),
    };
    let parts = FluxParts { clip_l: "l".into(), t5: "t".into(), vae: "v".into() };
    let wf = flux_workflow("fill.safetensors", false, &parts, "i.png", "m.png", &t);
    assert_eq!(wf["1"]["class_type"], "UNETLoader");
    assert_eq!(wf["7"]["inputs"]["guidance"], 12.5);
    assert_eq!(comfy_model_named("flux1-fill-dev.safetensors"), ComfyModel::FluxFill { unet: "flux1-fill-dev.safetensors".into(), gguf: false });
    assert_eq!(comfy_model_named("Flux1-Fill-dev-Q4.GGUF"), ComfyModel::FluxFill { unet: "Flux1-Fill-dev-Q4.GGUF".into(), gguf: true });
    assert_eq!(comfy_model_named("flux1-dev-Q4_K_S.gguf"), ComfyModel::Checkpoint("flux1-dev-Q4_K_S.gguf".into()), "plain Flux dev can't inpaint");
    assert_eq!(comfy_model_named("sdxl.safetensors"), ComfyModel::Checkpoint("sdxl.safetensors".into()));
}

#[test]
fn the_model_list_offers_fill_models_and_hides_unrelated_unets() {
    let mut s = session();
    let (host, server) = flux_server([0, 0, 0]);
    let r = s.execute("edit.generativeFill.models", json!({"backend": "comfyui", "host": host})).unwrap();
    // The fake's /view never comes, so end the server by asking for it.
    let _ = std::net::TcpStream::connect(&host).and_then(|mut c| c.write_all(b"GET /view?filename=f.png HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
    server.join().unwrap();
    let names: Vec<&str> = r["models"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(names, ["sdxl.safetensors", "flux1-fill-dev-Q4_K_S.gguf", "my-sdxl-inpainting-unet.safetensors"], "{names:?}");
    assert_eq!(r["default"], "flux1-fill-dev-Q4_K_S.gguf");
}

#[test]
fn flux_fill_names_missing_files() {
    let (host, server) = fake_server("/object_info/VAELoader", |_, path, _| match path {
        "/object_info/DualCLIPLoader" => (
            200,
            json!({"DualCLIPLoader": {"input": {"required": {"clip_name1": [["clip_l.safetensors"]], "clip_name2": [["clip_l.safetensors"]]}}}})
                .to_string()
                .into_bytes(),
        ),
        _ => (200, json!({"VAELoader": {"input": {"required": {"vae_name": [["sdxl_vae.safetensors"]]}}}}).to_string().into_bytes()),
    });
    let ep = parse_endpoint(&host, Backend::Comfy).unwrap();
    let err = flux_parts(&ep, &|| false).unwrap_err().to_string();
    server.join().unwrap();
    assert!(err.contains("t5xxl") && err.contains("Flux VAE") && !err.contains("clip_l"), "{err}");
}

#[test]
fn generative_remove_is_a_one_click_fill_with_removal_defaults() {
    let mut s = session();
    assert!(!s.is_enabled("edit.generativeRemove"), "needs a selection");
    rect_selection(&mut s);
    let (host, server) = a1111_server([200, 180, 140]);
    let before = layer_count(&s);
    // No prompt, no dialog: only where the server is.
    let r = s.execute("edit.generativeRemove", json!({"backend": "a1111", "host": host, "colorMatch": 0})).unwrap();
    assert_eq!(r["generated"].as_u64().unwrap(), 400, "{r}");
    let seen = server.join().unwrap();
    let req: Value = serde_json::from_slice(&seen.lock().unwrap()[0].2).unwrap();
    assert!(req["prompt"].as_str().unwrap().contains("seamless continuation"), "{}", req["prompt"]);
    assert!(req["negative_prompt"].as_str().unwrap().contains("watermark"));
    assert_eq!(layer_count(&s), before + 1);
    assert_eq!(s.active().unwrap().history.undo_label(), Some("Generative Remove"));
    let name = {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone()
    };
    assert!(name.starts_with("Generative Remove"), "{name}");
    s.undo();
    assert_eq!(layer_count(&s), before);

    // Anything the fill takes can still be given, and overrides the defaults.
    let (host, server) = a1111_server([0, 0, 0]);
    s.execute("edit.generativeRemove", json!({"backend": "a1111", "host": host, "prompt": "plain sand", "feather": 0, "colorMatch": 0})).unwrap();
    let seen = server.join().unwrap();
    let req: Value = serde_json::from_slice(&seen.lock().unwrap()[0].2).unwrap();
    assert_eq!(req["prompt"], "plain sand");
    let before = state(&s);
    assert!(s.execute("edit.generativeRemove", json!({"feather": "wide"})).is_err());
    assert_eq!(state(&s), before, "a rejected parameter changes nothing");
}

/// The host and its path go straight into the HTTP request, so anything that could end a line or
/// start a query is refused: a CR or LF there would inject headers.
#[test]
fn the_host_and_path_cannot_inject_into_the_request() {
    let mut s = session();
    rect_selection(&mut s);
    let before = state(&s);
    for bad in [
        "127.0.0.1:8188/a\r\nX-Injected: 1",
        "127.0.0.1:8188/a\nX: y",
        "127.0.0.1:8188/with space",
        "127.0.0.1:8188/path?query=1",
        "127.0.0.1:8188/path#frag",
        "127.0.0.1:8188/%0d%0aX: y",
        "127.0.0.1\r\nX: y:8188",
        "127.0.0.1 :8188",
        "127.0.0.1:8188/a\"b",
    ] {
        let err = parse_endpoint(bad, Backend::Comfy).unwrap_err().to_string();
        assert!(err.contains("may contain only"), "{bad:?}: {err}");
        assert!(s.execute("edit.generativeFill", json!({"host": bad})).is_err(), "{bad:?}");
        assert!(s.execute("edit.generativeFill.models", json!({"host": bad})).is_err(), "{bad:?}");
        assert_eq!(state(&s), before, "{bad:?}");
    }
    // Ordinary mount paths and hosts still work.
    for ok in ["127.0.0.1:8188/sub/path-1_x.y~z/", "localhost:8188", "[::1]:8188", "http://127.0.0.1:8188/comfy"] {
        assert!(parse_endpoint(ok, Backend::Comfy).is_ok(), "{ok}");
    }
}

/// The model lookup shares one short limit across its requests (it runs on the UI thread): a
/// server that answers the first and then goes quiet costs about 4 s in all, not 4 s per request.
#[test]
fn listing_models_has_one_time_limit_for_all_its_requests() {
    use std::time::{Duration, Instant};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        // The first request is answered, and its connection closed, as a real server does.
        let (mut first, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = first.read(&mut buf).unwrap();
        let body = json!({"CheckpointLoaderSimple": {"input": {"required": {"ckpt_name": [["a.safetensors"]]}}}}).to_string();
        write!(first, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        drop(first);
        // The two lookups after it connect and are never answered.
        let mut held = Vec::new();
        for _ in 0..2 {
            if let Ok((sock, _)) = listener.accept() {
                held.push(sock);
            }
        }
        std::thread::sleep(Duration::from_secs(6));
        drop(held);
    });
    let mut s = session();
    let started = Instant::now();
    let r = s.execute("edit.generativeFill.models", json!({"backend": "comfyui", "host": host})).unwrap();
    let took = started.elapsed();
    assert!(took < Duration::from_secs(6), "one shared limit, not one per request (two silent lookups would take 8 s): {took:?}");
    assert!(took >= Duration::from_secs(3), "it did wait for the silent lookups up to the limit: {took:?}");
    assert_eq!(r["models"], json!(["a.safetensors"]), "what was answered is still returned");
    server.join().unwrap();
}

/// The dialogs show, and then submit, the server a fill would use: parameters first, then
/// Preferences, then this machine on the backend's usual port.
#[test]
fn the_effective_server_is_the_one_a_fill_would_use() {
    let mut s = session();
    // Nothing saved: this machine, the backend's port (a developer's own environment aside).
    if std::env::var(HOST_ENV).is_err() && std::env::var(BACKEND_ENV).is_err() {
        assert_eq!(effective_server(&s, &json!({})).unwrap(), ("comfyui".to_string(), "127.0.0.1:8188".to_string()));
        assert_eq!(effective_server(&s, &json!({"backend": "a1111"})).unwrap().1, "127.0.0.1:7860");
        assert_eq!(default_host_for("a1111").unwrap(), "127.0.0.1:7860");
    }
    s.execute("prefs.set", json!({"path": "integrations.generativeFillBackend", "value": "a1111"})).unwrap();
    s.execute("prefs.set", json!({"path": "integrations.generativeFillHost", "value": "192.168.1.20:7000"})).unwrap();
    if std::env::var(HOST_ENV).is_err() && std::env::var(BACKEND_ENV).is_err() {
        assert_eq!(effective_server(&s, &json!({})).unwrap(), ("a1111".to_string(), "192.168.1.20:7000".to_string()));
        assert_eq!(effective_server(&s, &json!({"host": "  "})).unwrap().1, "192.168.1.20:7000", "a blank parameter is no parameter");
    }
    // Parameters win over what is saved.
    assert_eq!(effective_server(&s, &json!({"backend": "comfyui", "host": "10.0.0.5:9"})).unwrap(), ("comfyui".to_string(), "10.0.0.5:9".to_string()));
    assert!(effective_server(&s, &json!({"backend": "dalle"})).is_err());
    assert!(default_host_for("dalle").is_err());
}

/// The lookup the dialogs run on a worker thread needs no session, echoes the server it asked,
/// and refuses a bad backend or a public host without making a request.
#[test]
fn the_session_free_lookup_echoes_the_server_it_asked() {
    let (host, server) =
        fake_server("/sdapi/v1/sd-models", |_, _, _| (200, json!([{"title": "m1 [abc]"}, {"model_name": "m2-inpainting"}]).to_string().into_bytes()));
    let r = lookup_models("a1111", &host).unwrap();
    server.join().unwrap();
    assert_eq!((r["backend"].as_str(), r["host"].as_str(), r["server"].as_str()), (Some("a1111"), Some(host.as_str()), Some(host.as_str())));
    assert_eq!(r["models"], json!(["m1 [abc]", "m2-inpainting"]));
    assert_eq!(r["default"], "m2-inpainting");
    assert!(lookup_models("dalle", &host).is_err());
    assert!(lookup_models("a1111", "8.8.8.8:7860").unwrap_err().to_string().contains("local network"));
    assert!(lookup_models("a1111", "127.0.0.1:7860/a b").unwrap_err().to_string().contains("may contain only"));
}

/// Menus offer the fill only once a server is set up; a saved host (or the environment's) does it,
/// blank or whitespace does not, and a command with its own `host` runs regardless.
#[test]
fn a_server_counts_as_configured_once_a_host_is_saved() {
    let mut s = session();
    if std::env::var(HOST_ENV).is_err() {
        assert!(!is_configured(&s));
        s.execute("prefs.set", json!({"path": "integrations.generativeFillHost", "value": "   "})).unwrap();
        assert!(!is_configured(&s), "whitespace is no host");
    }
    s.execute("prefs.set", json!({"path": "integrations.generativeFillHost", "value": "192.168.1.20:8188"})).unwrap();
    assert!(is_configured(&s));
}
