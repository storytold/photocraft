//! The Generative Fill model rows' lookups, against one-request servers on this machine, and the dialog's rows.

use std::time::{Duration, Instant};

use serde_json::json;

use super::*;

/// A one-request server for the model lookups: answers the ComfyUI checkpoint list after `delay_ms`, then closes.
fn checkpoint_list(names: &'static [&'static str], delay_ms: u64) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf).unwrap();
        std::thread::sleep(Duration::from_millis(delay_ms));
        let body = json!({"CheckpointLoaderSimple": {"input": {"required": {"ckpt_name": [names]}}}}).to_string();
        let _ = write!(sock, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    });
    (host, server)
}

/// An address nothing listens on.
fn nobody_home() -> String {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    closed.local_addr().unwrap().to_string()
}

/// Follows `host` from row time `t` until its list has come back (the lookup runs on a worker thread); the passes
/// after the first are past the typing pause.
fn follow_until_listed(list: &mut ModelList, host: &str, t: f64) {
    let started = Instant::now();
    let mut now = t;
    while list.follow("comfyui", host, now, false) {
        now = t + TYPING_PAUSE + 0.01;
        assert!(started.elapsed() < Duration::from_secs(8), "timed out waiting for {host}: {} {:?}", list.note, list.models);
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_model_row_lists_the_servers_models() {
    let (host, server) = checkpoint_list(&["sd15.safetensors", "sd15-inpainting.safetensors"], 0);
    let mut list = ModelList::default();
    assert!(list.follow("comfyui", &host, 0.0, false), "the first pass asks at once");
    assert!(list.note.contains("Looking for models"), "{}", list.note);
    follow_until_listed(&mut list, &host, 0.0);
    server.join().unwrap();
    assert_eq!(list.models, ["sd15.safetensors", "sd15-inpainting.safetensors"]);
    assert!(list.note.contains("2 models"), "{}", list.note);
    // Nothing chosen yet: the server's choice (an inpainting checkpoint) is filled in.
    let mut model = String::new();
    list.settle(&mut model, "");
    assert_eq!(model, "sd15-inpainting.safetensors");
}

#[test]
fn a_silent_server_leaves_a_text_box_and_says_why() {
    let host = nobody_home();
    let mut list = ModelList::default();
    follow_until_listed(&mut list, &host, 0.0);
    assert!(list.models.is_empty());
    assert!(list.note.starts_with("Couldn't list models"), "{}", list.note);
    let mut model = "typed.safetensors".to_string();
    list.settle(&mut model, "");
    assert_eq!(model, "typed.safetensors", "a typed model stays");
}

#[test]
fn changing_the_server_waits_for_typing_and_ignores_the_old_answer() {
    let (slow, old_server) = checkpoint_list(&["old.safetensors"], 400);
    let (fresh, new_server) = checkpoint_list(&["new.safetensors"], 0);
    let mut list = ModelList::default();
    assert!(list.follow("comfyui", &slow, 0.0, false));
    // The host changes while the slow server is still answering: within the typing pause nothing is asked yet.
    assert!(list.follow("comfyui", &fresh, 1.0, false));
    assert!(list.note.contains(slow.as_str()), "still the old lookup's note: {}", list.note);
    follow_until_listed(&mut list, &fresh, 1.0 + TYPING_PAUSE);
    assert_eq!(list.models, ["new.safetensors"], "the old server's answer never shows");
    old_server.join().unwrap();
    new_server.join().unwrap();
    assert!(!list.follow("comfyui", &fresh, 5.0, false), "a list for the server shown asks nothing more");
}

#[test]
fn an_automatic_model_follows_the_server_but_a_typed_one_stays() {
    let (first, a) = checkpoint_list(&["a.safetensors", "b.safetensors"], 0);
    let (second, b) = checkpoint_list(&["c.safetensors"], 0);
    let mut list = ModelList::default();
    follow_until_listed(&mut list, &first, 0.0);
    let mut model = String::new();
    list.settle(&mut model, "b.safetensors");
    assert_eq!(model, "b.safetensors", "the saved model when the server has it");
    follow_until_listed(&mut list, &second, 10.0);
    list.settle(&mut model, "b.safetensors");
    assert_eq!(model, "c.safetensors", "a model the row filled in yields to the new server's");
    a.join().unwrap();
    b.join().unwrap();

    let (third, c) = checkpoint_list(&["d.safetensors"], 0);
    let mut typed = "mine.safetensors".to_string();
    follow_until_listed(&mut list, &third, 20.0);
    list.settle(&mut typed, "");
    assert_eq!(typed, "mine.safetensors", "a model the user typed is kept");
    assert!(list.note.contains("isn't one of them"), "{}", list.note);
    c.join().unwrap();
}

#[test]
fn refresh_asks_the_same_server_again() {
    let (host, server) = checkpoint_list(&["a.safetensors"], 0);
    let mut list = ModelList::default();
    follow_until_listed(&mut list, &host, 0.0);
    server.join().unwrap();
    let (again, server) = checkpoint_list(&["a.safetensors", "b.safetensors"], 0);
    // Same server key as `again` from now on; a refresh asks at once rather than after the pause.
    assert!(list.follow("comfyui", &again, 1.0, true));
    follow_until_listed(&mut list, &again, 1.0);
    assert_eq!(list.models.len(), 2);
    server.join().unwrap();
}

#[test]
fn the_dialog_keeps_the_sampler_controls_behind_advanced_options() {
    let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for command in COMMANDS {
        let spec = photocraft_engine::commands::find(command).expect("command").params;
        let mut rows = Param::parse_list(spec);
        dialog_rows(&app, command, &mut rows);
        let keys: Vec<&str> = rows.iter().map(|p| p.key.as_str()).collect();
        let toggle = keys.iter().position(|k| *k == SHOW_ADVANCED).expect("the Advanced Options box");
        for key in ADVANCED {
            let at = keys.iter().position(|k| *k == key).unwrap_or_else(|| panic!("{key}"));
            assert!(at > toggle, "{key} follows the box");
        }
        for key in ["prompt", "model", "feather", "colorMatch", "output"] {
            let at = keys.iter().position(|k| *k == key).unwrap_or_else(|| panic!("{key}"));
            assert!(at < toggle, "{key} stays on the dialog itself");
        }
        assert!(matches!(rows[keys.iter().position(|k| *k == "model").unwrap()].kind, ParamKind::Custom(_)), "the model row asks the server");
    }
    assert!(!is_advanced_param("filter.blur.gaussian", "steps"), "other dialogs keep every row visible");
}
