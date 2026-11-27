//! #10894: Crop's Fill goes back to its default entry when another document that was already open
//! becomes active, and keeps Content-Aware Fill otherwise (tool switches, new documents, a
//! committed crop, a cancelled crop).

use craft::draw::ui::crop::CropFill;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::crop_ui;
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The app with one 64 × 48 document and the Crop tool in Classic Mode, so a drag draws a frame.
fn harness() -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        // Snapping would pull the drawn frames' corners to the canvas edges and guides.
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool_options.crop.shield.classic_mode = true;
        app
    });
    h.run_steps(4);
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    h
}

/// A control request as the control channel sends it, which must succeed, then frames so the
/// canvas takes any queued input.
fn control(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let v = match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => {
            assert_eq!(v["ok"], true, "{method} {params}: {v}");
            v
        }
        // Input the canvas takes on its next pass: the frames below deliver it.
        Reply::AfterInput => Value::Null,
        _ => panic!("{method} {params}: not done"),
    };
    h.run_steps(6);
    v
}

/// Draws a crop frame from `a` to `b` (document px) with pointer events.
fn draw_frame(h: &mut Harness, a: [f64; 2], b: [f64; 2]) {
    let events = json!([
        {"kind": "down", "x": a[0], "y": a[1]},
        {"kind": "move", "x": b[0], "y": b[1]},
        {"kind": "up", "x": b[0], "y": b[1]},
    ]);
    control(h, "ui.pointer", json!({"events": events}));
}

/// Picks a document by its Window menu row, as the tab bar does.
fn activate(h: &mut Harness, index: usize) {
    control(h, "ui.menu.invoke", json!({"id": format!("window.document.{index}")}));
    assert_eq!(h.state().session.active_index(), Some(index), "document {index} is active");
}

fn fill(h: &Harness) -> CropFill {
    h.state().ui.tool_options.crop.fill
}

fn size(h: &Harness) -> (u32, u32) {
    let s = h.state().session.active().expect("a document").doc.size;
    (s.width, s.height)
}

#[test]
fn fill_goes_back_to_the_default_entry_when_another_open_document_becomes_active() {
    let mut h = harness();
    h.state_mut().ui.tool_options.crop.fill = CropFill::ContentAware;

    control(&mut h, "ui.set", json!({"tool": "brush"}));
    control(&mut h, "ui.set", json!({"tool": "crop"}));
    assert_eq!(fill(&h), CropFill::ContentAware, "a tool switch and back keeps it");

    h.state_mut().run("file.new", json!({"width": 32, "height": 32})).unwrap();
    h.run_steps(6);
    assert_eq!(h.state().session.active_index(), Some(1), "the new document is active");
    assert_eq!(fill(&h), CropFill::ContentAware, "a new document keeps it");

    // A committed crop keeps it.
    // Each press starts clear of the untouched default frame's handles and edges (their reach is at most a quarter
    // of the frame's shortest side on screen), so it draws a frame instead of resizing the default one.
    draw_frame(&mut h, [10.0, 10.0], [26.0, 22.0]);
    assert!(crop_ui::pending(h.state()), "the drawn frame is pending");
    control(&mut h, "ui.menu.invoke", json!({"id": "image.crop"}));
    assert_eq!(size(&h), (16, 12), "the crop was committed");
    assert_eq!(fill(&h), CropFill::ContentAware, "a committed crop keeps it");

    // A cancelled crop keeps it.
    draw_frame(&mut h, [5.0, 4.0], [12.0, 9.0]);
    assert!(crop_ui::pending(h.state()));
    control(&mut h, "ui.key", json!({"key": "Escape"}));
    assert!(!crop_ui::pending(h.state()), "Escape cancelled the crop");
    assert_eq!(size(&h), (16, 12), "the cancelled crop changed nothing");
    assert_eq!(fill(&h), CropFill::ContentAware, "a cancelled crop keeps it");

    // Another document that was already open becomes active.
    activate(&mut h, 0);
    assert_eq!(fill(&h), CropFill::Background, "another open document: back to the default");

    // Switching back does not bring it back.
    activate(&mut h, 1);
    assert_eq!(fill(&h), CropFill::Background, "switching back: stays at the default");

    // Closing the active document makes another open one active. The crop above left it modified, so the close
    // discards its changes instead of asking to save them.
    h.state_mut().ui.tool_options.crop.fill = CropFill::ContentAware;
    h.state_mut().run("file.close", json!({"save": "discard"})).unwrap();
    h.run_steps(6);
    assert_eq!(h.state().session.documents().len(), 1, "one document closed");
    assert_eq!(fill(&h), CropFill::Background, "closing the active document: back to the default");
}
