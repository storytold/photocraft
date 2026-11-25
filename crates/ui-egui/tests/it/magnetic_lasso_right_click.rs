//! Magnetic Lasso: a right-click while a border is being drawn opens no menu (#11689).
//!
//! Measured in Photoshop: with a Magnetic Lasso border in progress, a right-click on the canvas
//! opens nothing and changes nothing; the border stays live, and ↩ still closes it. With no border
//! in progress the right-click opens the selection tools' menu as before. PhotoCraft opened that
//! menu over the live border, from the canvas and from the control channel alike.
//!
//! The border is drawn with ⌥-clicks (straight segments), so its points don't depend on edges.
//! Driven through the control channel in an egui_kittest harness (no GPU needed): the canvas takes
//! each `ui.pointer` call's input on the frames that follow it.

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

/// The border's corners: a plain click starts it, ⌥-clicks add the others.
const CORNERS: [[f64; 2]; 3] = [[50.0, 50.0], [250.0, 50.0], [150.0, 200.0]];

/// Where the right-click goes: inside the border, on the image.
const INSIDE: [f64; 2] = [150.0, 100.0];

fn app() -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width": 400, "height": 300})).expect("new document");
        app
    });
    h.run_steps(4);
    h
}

/// Runs one control-channel request, which must succeed, then frames so the canvas takes any
/// queued input. Returns the reply, `null` for input answered after the frames.
fn call(h: &mut Harness, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    let v = match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => {
            assert_eq!(v["ok"], true, "{method} {params}: {v}");
            v
        }
        Reply::AfterInput => Value::Null,
        _ => panic!("{method} {params}: not done"),
    };
    h.run_steps(6);
    v
}

/// Clicks at `p` through `ui.pointer`; `tool` names the tool the call selects, if any.
fn click(h: &mut Harness, tool: Option<&str>, p: [f64; 2], button: &str, alt: bool) {
    let events = json!([{"kind": "down", "x": p[0], "y": p[1]}, {"kind": "up", "x": p[0], "y": p[1]}]);
    let mut params = json!({"button": button, "alt": alt, "events": events});
    if let Some(tool) = tool {
        params["tool"] = json!(tool);
    }
    call(h, "ui.pointer", params);
}

/// `ui.inspect`'s Magnetic Lasso border and canvas tool menu.
fn inspect(h: &mut Harness) -> (Value, Value) {
    let v = call(h, "ui.inspect", json!({}));
    let r = &v["result"];
    (r["magnetic"].clone(), r["canvasToolMenu"].clone())
}

/// Whether the selection menu is open: it lists Select All.
fn selection_menu_open(menu: &Value) -> bool {
    menu["entries"].as_array().is_some_and(|entries| entries.iter().any(|e| e["id"] == "select.all"))
}

/// A border is in progress: it has fastening points.
fn border_live(magnetic: &Value) -> bool {
    magnetic["anchors"].as_array().is_some_and(|a| !a.is_empty())
}

#[test]
fn agent_right_click_on_another_document_still_opens_the_selection_menu() {
    // A border left on the first document doesn't hold the menu back on the second: the gate is
    // per document. The tool drops the first document's border on its next pass.
    let mut h = app();
    click(&mut h, Some("MagneticLasso"), CORNERS[0], "left", false);
    click(&mut h, Some("MagneticLasso"), CORNERS[1], "left", true);
    h.state_mut().run("file.new", json!({"width": 400, "height": 300})).expect("second document");
    h.run_steps(6);

    click(&mut h, Some("MagneticLasso"), INSIDE, "right", false);
    let (_, menu) = inspect(&mut h);
    assert!(selection_menu_open(&menu), "{menu}");
}

#[test]
fn canvas_right_click_while_drawing_opens_no_menu_and_enter_still_closes_the_border() {
    let mut h = app();
    // The tool is the current one, as when the user picked it; the secondary press is the canvas's.
    call(&mut h, "ui.set", json!({"tool": "MagneticLasso"}));
    click(&mut h, None, CORNERS[0], "left", false);
    click(&mut h, None, CORNERS[1], "left", true);
    click(&mut h, None, CORNERS[2], "left", true);
    // The pointer is already over the click point: moving it there extends the live border (the lasso fastens points
    // along the pointer's path), which the right-click itself must not do.
    let hover = json!({"events": [{"kind": "move", "x": INSIDE[0], "y": INSIDE[1]}]});
    call(&mut h, "ui.pointer", hover);
    let (before, _) = inspect(&mut h);
    assert!(border_live(&before), "a border is in progress: {before}");

    click(&mut h, None, INSIDE, "secondary", false);
    let (after, menu) = inspect(&mut h);
    assert!(menu.is_null(), "a right-click over a live border opens no menu: {menu}");
    assert_eq!((&after["path"], &after["anchors"]), (&before["path"], &before["anchors"]), "the border is unchanged");

    call(&mut h, "ui.key", json!({"key": "Enter"}));
    let (closed, _) = inspect(&mut h);
    assert!(!border_live(&closed), "↩ closed the border: {closed}");
    let sel = h.state().session.active().expect("a document").doc.selection.as_ref().expect("the border closed into a selection");
    assert!(sel.sample_channel(150, 100, 0) > 0.9, "the inside is selected");
}
