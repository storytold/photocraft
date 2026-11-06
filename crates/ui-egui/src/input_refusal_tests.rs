//! Synthetic-input control replies report the refusals that input caused (#11579).
//!
//! These drive the whole app through its control channel: `ui.key` and friends queue events, the
//! harness injects each step the way `raw_input_hook` does, and the request's reply arrives once
//! the queue has settled.

use std::sync::mpsc::{Receiver, Sender};

use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::{ControlRequest, ControlResponse, PhotocraftApp, Services};

/// The policy a sandboxed agent might run under: no file system commands. The app can export,
/// so File › Save is enabled and only the policy refuses it.
fn deny_files() -> Services {
    Services {
        automation_command: Some(Box::new(|id, _| if id.starts_with("file.") && id != "file.new" { Err("filesystem command denied".into()) } else { Ok(()) })),
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((Vec::new(), Vec::new())))),
        ..Default::default()
    }
}

fn harness(services: Services, document: bool) -> (Harness<'static, PhotocraftApp>, Sender<ControlRequest>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        if document {
            app.session.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
        }
        app.with_control(rx)
    });
    h.run_steps(4);
    (h, tx)
}

fn send(tx: &Sender<ControlRequest>, method: &str, params: Value) -> Receiver<ControlResponse> {
    let (mut req, reply) = ControlRequest::new(method, params);
    req.deadline = None;
    tx.send(req).unwrap();
    reply
}

/// Run frames, feeding queued synthetic input in as the native host's `raw_input_hook` does.
fn pump(h: &mut Harness<'static, PhotocraftApp>, frames: usize) {
    for _ in 0..frames {
        let ctx = h.ctx.clone();
        let mut raw = egui::RawInput::default();
        h.state_mut().inject_synthetic(&ctx, &mut raw);
        h.input_mut().events.extend(raw.events);
        h.step();
    }
}

fn reply(h: &mut Harness<'static, PhotocraftApp>, rx: &Receiver<ControlResponse>) -> ControlResponse {
    for _ in 0..16 {
        if let Ok(r) = rx.try_recv() {
            return r;
        }
        pump(h, 1);
    }
    panic!("no reply after the input settled");
}

#[test]
fn a_refused_ctrl_s_replies_with_the_refusal_and_keeps_the_status_bar() {
    let (mut h, tx) = harness(deny_files(), true);
    let rx = send(&tx, "ui.key", json!({"key": "S", "command": true}));
    let r = reply(&mut h, &rx);
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"], "filesystem command denied", "{r}");
    assert_eq!(r["errors"], json!(["filesystem command denied"]), "reported once, not by every layer: {r}");
    assert_eq!(h.state().ui.status, "filesystem command denied", "the status bar still says why");
    assert!(h.state().ui.status_error);
}

#[test]
fn a_repeated_refusal_is_reported_again() {
    // The status bar already shows the refusal: the second Ctrl+S must still be reported.
    let (mut h, tx) = harness(deny_files(), true);
    for _ in 0..2 {
        let rx = send(&tx, "ui.key", json!({"key": "S", "command": true}));
        let r = reply(&mut h, &rx);
        assert_eq!(r["error"], "filesystem command denied", "{r}");
    }
}

#[test]
fn a_disabled_menu_shortcut_replies_with_why() {
    // No document: File › Save is disabled, and its shortcut says so.
    let (mut h, tx) = harness(Services::default(), false);
    let rx = send(&tx, "ui.key", json!({"key": "S", "command": true}));
    let r = reply(&mut h, &rx);
    assert_eq!(r["ok"], false, "{r}");
    let error = r["error"].as_str().unwrap_or_default();
    assert!(error.contains("not available"), "{r}");
    assert_eq!(h.state().ui.status, error);
}

#[test]
fn a_refused_dialog_ok_replies_with_the_refusal() {
    let (mut h, tx) = harness(deny_files(), true);
    // A dialog whose OK exports a file: the policy refuses it in `confirm_preflight`.
    let dialog = {
        let app = h.state_mut();
        let mut fixed = serde_json::Map::new();
        fixed.insert("__export".into(), json!(true));
        crate::filter_dialog::open_with_spec(app, "file.export.exportAs", "Export", r#"{"width":1..300000=100}"#, fixed)
    };
    pump(&mut h, 4);
    let rx = send(&tx, "ui.key", json!({"key": "Enter"}));
    let r = reply(&mut h, &rx);
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"], "filesystem command denied", "{r}");
    assert!(h.state().ui.dialogs.iter().any(|d| d.id == dialog), "a refused OK keeps the dialog open");
}

#[test]
fn input_that_succeeds_or_does_nothing_still_replies_ok() {
    let (mut h, tx) = harness(deny_files(), true);
    for (method, params) in
        [("ui.key", json!({"key": "V"})), ("ui.move", json!({"x": 5.0, "y": 5.0})), ("ui.key", json!({"key": "F20"})), ("ui.type", json!({"text": "x"}))]
    {
        let rx = send(&tx, method, params.clone());
        assert_eq!(reply(&mut h, &rx), json!({"ok": true, "result": null}), "{method} {params}");
    }
    assert_eq!(h.state().ui.tool, crate::Tool::Move, "the V press was handled");
}

#[test]
fn concurrent_requests_each_get_only_their_own_refusals() {
    let (mut h, tx) = harness(deny_files(), true);
    // Queued in the same frame: a refused Ctrl+S, then a harmless V, then another refused Ctrl+S.
    let first = send(&tx, "ui.key", json!({"key": "S", "command": true}));
    let second = send(&tx, "ui.key", json!({"key": "V"}));
    let third = send(&tx, "ui.key", json!({"key": "S", "command": true, "shift": true}));
    let first = reply(&mut h, &first);
    let second = reply(&mut h, &second);
    let third = reply(&mut h, &third);
    assert_eq!(first["error"], "filesystem command denied", "{first}");
    assert_eq!(second, json!({"ok": true, "result": null}), "the V press caused no refusal");
    assert_eq!(third["error"], "filesystem command denied", "{third}");
    assert_eq!(first["errors"].as_array().map(Vec::len), Some(1), "{first}");
}

#[test]
fn cancelled_input_still_says_so() {
    let (mut h, tx) = harness(deny_files(), true);
    let rx = send(&tx, "ui.key", json!({"key": "S", "command": true}));
    // The request is queued (no step injected yet); the document changes before it is delivered.
    h.step();
    let ctx = h.ctx.clone();
    let mut raw = egui::RawInput::default();
    h.state_mut().inject_synthetic(&ctx, &mut raw);
    h.state_mut().run("file.new", json!({"width": 8, "height": 8})).unwrap();
    h.state_mut().inject_synthetic(&ctx, &mut egui::RawInput::default());
    let r = rx.try_recv().expect("the cancelled request is answered");
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().is_some_and(|e| e.starts_with("input cancelled: ")), "{r}");
    assert!(r.get("errors").is_none(), "{r}");
}

/// A refusal a direct control call caused is that call's own error, never a later input's.
#[test]
fn a_refusal_outside_synthetic_input_is_not_reported_to_input() {
    let (mut h, tx) = harness(deny_files(), true);
    let direct = send(&tx, "engine.execute", json!({"command": "layer.delete", "params": {"layer": 999_999}}));
    pump(&mut h, 2);
    assert_eq!(direct.try_recv().unwrap()["ok"], false);
    let rx = send(&tx, "ui.key", json!({"key": "V"}));
    assert_eq!(reply(&mut h, &rx), json!({"ok": true, "result": null}));
}
