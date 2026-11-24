//! `ui.screenshot` through the app: craft-automation's capture queue driven by PhotoCraft's logic
//! and UI phases, its document identity, the geometry the runtime reports, and the automation
//! write authority. The queue's own rules (budgets, settling, deadlines, image checks) are
//! craft-automation's and tested there.

use craft::automation::screenshot::{CaptureGeometry, Captures, DocumentId, SETTLE_PASSES, ScreenshotRequest};
use craft::ui::runtime::motion;
use craft::ui::runtime::windows::WindowHost;
use egui::{Color32, ColorImage, Context, ViewportId, vec2};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn captures(app: &crate::PhotocraftApp) -> &Captures {
    &app.control.as_ref().expect("a control channel").captures
}

fn captures_mut(app: &mut crate::PhotocraftApp) -> &mut Captures {
    &mut app.control.as_mut().expect("a control channel").captures
}

/// Execute the same logic→UI order as eframe, with genuinely new host geometry before logic.
fn actual_capture_frame(app: &mut crate::PhotocraftApp, ctx: &Context, size: egui::Vec2, scale: f32, minimized: bool) -> egui::FullOutput {
    use eframe::App as _;
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut viewports = egui::ViewportIdMap::default();
    viewports.insert(
        ViewportId::ROOT,
        egui::ViewportInfo { inner_rect: Some(rect), native_pixels_per_point: Some(scale), minimized: Some(minimized), ..Default::default() },
    );
    let mut input = egui::RawInput { screen_rect: Some(rect), viewports, focused: true, ..Default::default() };
    app.raw_input_hook(ctx, &mut input);
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    });
    output.textures_delta.clear();
    output
}

fn actual_callback(output: &egui::FullOutput) -> Option<egui::ScreenshotCallback> {
    output.viewport_output.values().flat_map(|viewport| &viewport.commands).find_map(|command| match command {
        egui::ViewportCommand::Screenshot(callback) => Some(callback.clone()),
        _ => None,
    })
}

type ScreenshotWrites = std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<u8>)>>>;

struct ActualCallbackFixture {
    app: Box<crate::PhotocraftApp>,
    ctx: Context,
    writes: ScreenshotWrites,
    reply: std::sync::mpsc::Receiver<serde_json::Value>,
    document: Option<DocumentId>,
    old_geometry: CaptureGeometry,
    old_pixels: std::sync::Weak<ColorImage>,
}

fn actual_callback_fixture() -> ActualCallbackFixture {
    use std::cell::RefCell;
    use std::rc::Rc;

    let writes = Rc::new(RefCell::new(Vec::new()));
    let written = writes.clone();
    let services = crate::Services {
        automation_write: Some(Box::new(move |path, bytes| {
            written.borrow_mut().push((path.to_owned(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), services).with_control(std::sync::mpsc::channel().1);
    app.run("file.new", serde_json::json!({"width": 32, "height": 32})).unwrap();
    let ctx = app.ctx().clone();
    // `raw_input_hook` hands the craft runtime its frame input, as on the app's context.
    crate::PhotocraftApp::setup_context(&ctx, Default::default());
    for _ in 0..4 {
        actual_capture_frame(&mut app, &ctx, vec2(600.0, 400.0), 1.0, false);
    }
    let old_geometry = captures(&app).last_geometry(ViewportId::ROOT).expect("the old real UI geometry");
    let document = app.document_identity();
    let (tx, reply) = std::sync::mpsc::channel();
    captures_mut(&mut app)
        .admit(
            {
                let mut request = ScreenshotRequest::new(91, ViewportId::ROOT, tx);
                request.path = Some("fixture/current.png".into());
                request.document = document;
                request
            },
            motion::now(),
        )
        .unwrap();
    let mut callback = None;
    for _ in 0..8 {
        let output = actual_capture_frame(&mut app, &ctx, vec2(600.0, 400.0), 1.0, false);
        if let Some(found) = actual_callback(&output) {
            callback = Some(found);
            break;
        }
    }
    let old_image = Arc::new(ColorImage::filled(old_geometry.expected_pixels().unwrap(), Color32::RED));
    let old_pixels = Arc::downgrade(&old_image);
    callback.expect("actual app issued one screenshot callback").complete(old_image);
    assert!(writes.borrow().is_empty(), "the real callback only stores pixels");
    ActualCallbackFixture { app, ctx, writes, reply, document, old_geometry, old_pixels }
}

#[test]
fn completed_callback_revalidates_next_raw_input_before_actual_app_writes() {
    for (size, scale, minimized) in
        [(vec2(600.0, 400.0), 1.0, false), (vec2(640.0, 400.0), 1.0, false), (vec2(600.0, 400.0), 2.0, false), (vec2(600.0, 400.0), 1.0, true)]
    {
        let mut fixture = actual_callback_fixture();
        if size == vec2(600.0, 400.0) && scale == 1.0 && !minimized {
            verify_unchanged_capture(&mut fixture);
        } else {
            verify_changed_capture(&mut fixture, size, scale, minimized);
        }
    }
}

fn verify_unchanged_capture(fixture: &mut ActualCallbackFixture) {
    actual_capture_frame(&mut fixture.app, &fixture.ctx, vec2(600.0, 400.0), 1.0, false);
    assert_successful_capture(fixture, vec2(600.0, 400.0), 1.0, false);
}

fn verify_changed_capture(fixture: &mut ActualCallbackFixture, size: egui::Vec2, scale: f32, minimized: bool) {
    let output = actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, minimized);
    assert!(fixture.writes.borrow().is_empty(), "no stale write on the first changed input");
    assert!(fixture.old_pixels.upgrade().is_none(), "old pixels are released on geometry rejection");
    let current = captures(&fixture.app).last_geometry(ViewportId::ROOT).unwrap();
    assert!(
        current.size_points != fixture.old_geometry.size_points
            || current.pixels_per_point != fixture.old_geometry.pixels_per_point
            || current.minimized != fixture.old_geometry.minimized
    );
    if minimized {
        verify_minimized_capture(fixture, size, scale, &output);
        return;
    }
    assert!(fixture.reply.try_recv().is_err(), "bounded retry has not delivered a stale success");
    let mut retry = actual_callback(&output);
    for _ in 0..8 {
        if retry.is_some() {
            break;
        }
        retry = actual_callback(&actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, minimized));
    }
    retry
        .expect("the actual app reissues at bounded current geometry")
        .complete(Arc::new(ColorImage::filled(current.expected_pixels().unwrap(), Color32::BLUE)));
    actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, minimized);
    assert_successful_capture(fixture, size, scale, minimized);
}

fn verify_minimized_capture(fixture: &mut ActualCallbackFixture, size: egui::Vec2, scale: f32, output: &egui::FullOutput) {
    let reply = fixture.reply.try_recv().expect("one minimized error");
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(reply["error"].as_str().unwrap().contains("minimized"));
    assert_eq!(captures(&fixture.app).pending(), 0);
    assert_eq!(captures(&fixture.app).reserved_bytes(), 0);
    assert!(actual_callback(output).is_none());
    actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, false);
    let (tx, restored_rx) = std::sync::mpsc::channel();
    captures_mut(&mut fixture.app)
        .admit(
            {
                let mut request = ScreenshotRequest::new(92, ViewportId::ROOT, tx);
                request.path = Some("fixture/restored.png".into());
                request.document = fixture.document;
                request
            },
            motion::now(),
        )
        .unwrap();
    for _ in 0..8 {
        let output = actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, false);
        if let Some(callback) = actual_callback(&output) {
            callback.complete(Arc::new(ColorImage::filled(
                captures(&fixture.app).last_geometry(ViewportId::ROOT).unwrap().expected_pixels().unwrap(),
                Color32::BLUE,
            )));
            break;
        }
    }
    actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, false);
    let reply = restored_rx.try_recv().expect("the restored window accepts a new request");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(fixture.writes.borrow().len(), 1);
    assert!(fixture.writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(captures(&fixture.app).pending(), 0);
    assert_eq!(captures(&fixture.app).reserved_bytes(), 0);
    assert!(fixture.reply.try_recv().is_err() && restored_rx.try_recv().is_err());
}

fn assert_successful_capture(fixture: &mut ActualCallbackFixture, size: egui::Vec2, scale: f32, minimized: bool) {
    let reply = fixture.reply.try_recv().expect("one response after the valid callback");
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(fixture.writes.borrow().len(), 1);
    assert!(fixture.writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"), "real PNG encoding and automation_write ran");
    assert_eq!(captures(&fixture.app).pending(), 0);
    assert_eq!(captures(&fixture.app).reserved_bytes(), 0);
    actual_capture_frame(&mut fixture.app, &fixture.ctx, size, scale, minimized);
    assert_eq!(fixture.writes.borrow().len(), 1, "no duplicate output");
    assert!(fixture.reply.try_recv().is_err());
}

/// A `ui.screenshot` that arrives before the first frame (a launch with a capture request) is
/// answered with an image once the UI has drawn and settled, not rejected.
#[test]
fn a_launch_and_capture_request_waits_for_the_first_ui_pass() {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()).with_control(rx);
    let (request, reply) = crate::control::ControlRequest::new("ui.screenshot", json!({"focus": false}));
    tx.send(request).unwrap();
    let ctx = app.ctx().clone();
    crate::PhotocraftApp::setup_context(&ctx, Default::default());
    // The settle delay runs on the wall clock.
    let give_up = Instant::now() + Duration::from_secs(5);
    let callback = loop {
        let output = actual_capture_frame(&mut app, &ctx, vec2(600.0, 400.0), 1.0, false);
        assert!(reply.try_recv().is_err(), "no answer before the image");
        if let Some(found) = actual_callback(&output) {
            break found;
        }
        assert!(Instant::now() < give_up, "the request was never issued");
        std::thread::sleep(Duration::from_millis(20));
    };
    let geometry = captures(&app).last_geometry(ViewportId::ROOT).expect("the first pass recorded the geometry");
    callback.complete(Arc::new(ColorImage::filled(geometry.expected_pixels().unwrap(), Color32::RED)));
    actual_capture_frame(&mut app, &ctx, vec2(600.0, 400.0), 1.0, false);
    let reply = reply.try_recv().expect("one answer after the image");
    assert_eq!(reply["ok"], true, "{reply}");
}

/// PhotoCraft's document identity against a capture in flight: switching to another document, or
/// away and back, fails it; an edit in the same document does not.
#[test]
fn a_capture_in_flight_follows_the_app_document_identity() {
    let activate = |index: u64| ("document.activate", json!({"document": index}));
    type Case<'a> = (&'a str, Vec<(&'a str, Value)>, bool);
    let cases: [Case<'_>; 3] = [
        ("switch to another document", vec![activate(0)], false),
        ("switch away and back", vec![activate(0), activate(1)], false),
        ("edit in the same document", vec![("layer.new.layer", json!({}))], true),
    ];
    for (name, between, captured) in cases {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let ctx = app.ctx().clone();
        let now = motion::now();
        let geometry = |document: Option<DocumentId>| {
            let mut g = CaptureGeometry::new([600.0, 400.0], 8192);
            g.document = document;
            g
        };
        let admitted = app.document_identity();
        let mut queue = Captures::default();
        queue.note_geometry(ViewportId::ROOT, geometry(admitted));
        let (tx, _reply) = std::sync::mpsc::channel();
        queue
            .admit(
                {
                    let mut request = ScreenshotRequest::new(1, ViewportId::ROOT, tx);
                    request.document = admitted;
                    request
                },
                now,
            )
            .unwrap();
        for _ in 0..SETTLE_PASSES {
            queue.note_pass();
        }
        assert!(queue.issue(&WindowHost::of(&ctx), now, admitted).is_empty());
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        let callback = actual_callback(&output).expect("issued");
        callback.complete(Arc::new(ColorImage::filled([600, 400], Color32::RED)));

        for (command, params) in between {
            app.run(command, params).unwrap();
        }
        let current = app.document_identity();
        assert_ne!(current, admitted, "{name}: the identity moved");
        queue.note_geometry(ViewportId::ROOT, geometry(current));
        let finished = queue.drain(now, current);
        assert_eq!(finished.len(), 1, "{name}");
        assert_eq!(finished[0].result.is_ok(), captured, "{name}: {:?}", finished[0].result.as_ref().err());
    }
}
