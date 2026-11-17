//! Synthetic input around the app's passes: input queued for one active document is withdrawn
//! when another document becomes active between passes, not on an edit nor on a switch the
//! injected step makes itself, and a hidden window keeps painting while input or a capture waits.
//! The queue itself is the window host's (craft-automation's `synthetic` trials).

use crate::{PhotocraftApp, Services};
use craft::ui::runtime::windows::WindowHost;
use eframe::App as _;
use egui::{Context, RawInput};
use photocraft_engine::Session;
use serde_json::json;

fn app_with_doc() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).expect("a document");
    app
}

/// Sends `ui.key V` through a control channel and runs it: the reply waits on the queued input.
fn queue_key(app: &mut PhotocraftApp, ctx: &Context) -> std::sync::mpsc::Receiver<serde_json::Value> {
    let (transport, rx) = std::sync::mpsc::channel();
    app.control = Some(craft::automation::ControlChannel::new(rx));
    let (req, reply) = crate::ControlRequest::new("ui.key", json!({"key": "V"}));
    transport.send(req).expect("the channel is open");
    app.drain_control(ctx);
    assert!(!WindowHost::of(ctx).injected_idle(), "the key is queued");
    reply
}

/// Hidden logic ticks still receive the current viewport's native geometry. The default ROOT
/// entry has no geometry, so it cannot represent a window with a capturable size.
fn host_input(viewport: egui::ViewportId) -> RawInput {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let mut raw = RawInput { viewport_id: viewport, screen_rect: Some(rect), focused: true, ..Default::default() };
    raw.viewports
        .insert(viewport, egui::ViewportInfo { inner_rect: Some(rect), native_pixels_per_point: Some(1.0), minimized: Some(false), ..Default::default() });
    raw
}

/// A switch of the active document before the queued input ran withdraws it and fails the
/// waiting reply; an edit in the same document (a new revision, the same activation) does not.
#[test]
fn queued_input_is_withdrawn_only_when_the_active_document_changes() {
    for (command, params, withdrawn) in [("file.new", json!({"width": 16, "height": 16}), true), ("layer.new.layer", json!({"name": "queued"}), false)] {
        let mut app = app_with_doc();
        let ctx = app.ctx().clone();
        let reply = queue_key(&mut app, &ctx);
        app.run(command, params).expect(command);
        app.withdraw_stale_input(&ctx);
        if withdrawn {
            // The window host carries the withdrawal out in the next pass's input hook.
            ctx.run_ui(RawInput::default(), |_| {}).textures_delta.clear();
        }
        app.poll_input(&ctx);
        let host = WindowHost::of(&ctx);
        if withdrawn {
            let reply = reply.try_recv().expect("the waiting request is answered");
            assert_eq!(reply["ok"], false, "{command}");
            assert!(reply["error"].as_str().is_some_and(|e| e.contains("document or focus changed")), "{reply}");
            assert!(host.injected_idle(), "nothing is left for the new document");
        } else {
            assert!(reply.try_recv().is_err(), "{command}: no cancellation reply");
            assert!(!host.injected_idle(), "{command}: the input stays queued");
        }
    }
}

/// A hidden host runs `logic` without a UI pass. Both a capture that is waiting for its first
/// geometry and queued synthetic input must ask the host for a paint, or the window stays
/// hidden forever and the control request times out.
#[test]
fn a_logic_tick_requests_a_hidden_paint_for_waiting_captures_and_queued_input() {
    let paints = |out: &egui::LogicOutput| {
        out.viewport_commands.get(&egui::ViewportId::ROOT).is_some_and(|commands| commands.contains(&egui::ViewportCommand::RequestPaintWhileHidden))
    };
    let mut frame = eframe::Frame::_new_kittest();

    let mut app = app_with_doc();
    let ctx = app.ctx().clone();
    let (tx, _rx) = std::sync::mpsc::channel();
    let document = app.document_identity();
    let (_transport, requests) = std::sync::mpsc::channel();
    let mut channel = craft::automation::ControlChannel::new(requests);
    channel
        .captures
        .admit(
            {
                let mut request = craft::automation::screenshot::ScreenshotRequest::new(90, egui::ViewportId::ROOT, tx);
                request.document = document;
                request
            },
            craft::ui::runtime::motion::now(),
        )
        .expect("parked before the first pass");
    app.control = Some(channel);
    let out = ctx.run_logic(&host_input(egui::ViewportId::ROOT), |ctx| app.logic(ctx, &mut frame));
    assert!(paints(&out), "a capture waiting for geometry keeps the hidden window painting");

    let ctx = app.ctx().clone();
    let mut app = app_with_doc();
    let _reply = queue_key(&mut app, &ctx);
    let out = ctx.run_logic(&host_input(egui::ViewportId::ROOT), |ctx| app.logic(ctx, &mut frame));
    assert!(paints(&out), "queued synthetic input keeps the hidden window painting");
}

/// The whole app in an eframe harness with `documents` open and a control channel; the channel's
/// sending end.
pub(crate) fn app_harness(
    services: Services,
    documents: usize,
) -> (egui_kittest::Harness<'static, PhotocraftApp>, std::sync::mpsc::Sender<crate::ControlRequest>) {
    let (transport, requests) = std::sync::mpsc::channel();
    let requests = std::cell::Cell::new(Some(requests));
    let services = std::cell::Cell::new(Some(services));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), Session::new(), services.take().expect("one app"));
        for _ in 0..documents {
            app.run("file.new", json!({"width": 32, "height": 32})).expect("a document");
        }
        app.control = Some(craft::automation::ControlChannel::new(requests.take().expect("one channel")));
        app
    });
    h.run_steps(8);
    (h, transport)
}

/// Sends `method` over the harness's control channel and runs passes until it is answered.
pub(crate) fn answered(
    h: &mut egui_kittest::Harness<'static, PhotocraftApp>,
    transport: &std::sync::mpsc::Sender<crate::ControlRequest>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let (req, reply) = crate::ControlRequest::new(method, params);
    transport.send(req).expect("the channel is open");
    for _ in 0..64 {
        h.run_steps(1);
        if let Ok(answer) = reply.try_recv() {
            return answer;
        }
    }
    panic!("{method} was not answered");
}

/// A step that switches the active document itself (Ctrl+Tab on its press pass) delivers the rest
/// of its request: the reply is ok, and the key and the modifier are released.
#[test]
fn a_step_that_switches_the_document_delivers_the_rest_of_its_request() {
    let (mut h, transport) = app_harness(Services::default(), 2);
    let before = h.state().session.active_generation();
    let reply = answered(&mut h, &transport, "ui.key", json!({"key": "Tab", "ctrl": true}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_ne!(h.state().session.active_generation(), before, "Ctrl+Tab switched the document");
    h.run_steps(1);
    assert!(h.ctx.input(|i| i.keys_down.is_empty() && i.modifiers.is_none()), "nothing stays down");
}
