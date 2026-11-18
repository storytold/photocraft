//! #1118: Paint Bucket's All Layers option must sample the visible image while painting only the
//! active layer. Driven through the control channel (`ui.set`, then a `ui.pointer` click in
//! document px) in an egui_kittest harness; no GPU needed.

use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

fn harness() -> Harness {
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 720.0)).with_pixels_per_point(1.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app
    });
    h.run_steps(4);
    h
}

/// Runs one control-channel request, which must succeed, then frames so the canvas takes any
/// queued input.
fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params.clone());
    match handle(h.state_mut(), &ctx, &req) {
        Reply::Done(v) => assert_eq!(v["ok"], true, "{method} {params}: {v}"),
        Reply::AfterInput => {}
        _ => panic!("{method} {params}: not done"),
    }
    h.run_steps(6);
}

/// A white 400 × 300 document with its left half black and an empty active layer above it, a red
/// foreground and the Paint Bucket set to fill the contiguous exact colour. Returns the background
/// layer.
fn setup(h: &mut Harness, all_layers: bool) -> LayerId {
    let background = {
        let app = h.state_mut();
        while app.session.active().is_some() {
            app.run("file.close", json!({"discard": true})).expect("close");
        }
        app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).expect("new");
        let background = app.session.active().expect("document").active_layer.expect("background");
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300})).expect("select left half");
        app.run("edit.fill", json!({"contents": "black"})).expect("black left half");
        app.run("select.deselect", json!({})).expect("deselect");
        app.run("layer.new.layer", json!({"name": "Bucket fill"})).expect("empty active layer");
        app.run("tools.setColors", json!({"foreground": [1.0, 0.0, 0.0, 1.0]})).expect("red foreground");
        app.ui.extras.rulers = false;
        let o = &mut app.ui.tool_options;
        o.tolerance = 0.0;
        o.anti_alias = false;
        o.contiguous = true;
        o.sample_all.paint_bucket = all_layers;
        o.bucket_fill_pattern = false;
        o.fill_opacity = 100.0;
        background
    };
    // `ui.set` has no key for Paint Bucket's All Layers; the option is set on the public
    // `tool_options` field above, as the options bar's checkbox does.
    control(h, "ui.set", json!({"tool": "paintBucket", "zoom": 1.0, "center": [200, 150]}));
    background
}

fn pixels(h: &Harness, layer: LayerId) -> Vec<[f32; 4]> {
    let st = h.state().session.active().expect("document");
    let surface = st.doc.layer(layer).expect("layer").surface().expect("pixel layer");
    let mut pixels = vec![[0.0; 4]; 400 * 300];
    surface.read_rgba_into(Rect::new(0, 0, 400, 300), &mut pixels);
    pixels
}

#[test]
fn all_layers_checkbox_limits_bucket_fill_to_the_visible_color() {
    let mut h = harness();
    for all_layers in [false, true] {
        let background = setup(&mut h, all_layers);
        let before = pixels(&h, background);
        assert_eq!(h.state().ui.tool_options.sample_all.paint_bucket, all_layers, "the option is set");

        let events = json!([{"kind": "down", "x": 300.0, "y": 150.0}, {"kind": "up", "x": 300.0, "y": 150.0}]);
        control(&mut h, "ui.pointer", json!({"events": events}));

        assert_eq!(pixels(&h, background), before, "bucket must leave the sampled background unchanged");
        let active = h.state().session.active().expect("document").active_layer.expect("active layer");
        for (i, pixel) in pixels(&h, active).into_iter().enumerate() {
            let x = i % 400;
            let expected = if all_layers && x < 200 { [0.0; 4] } else { [1.0, 0.0, 0.0, 1.0] };
            assert_eq!(pixel, expected, "All Layers={all_layers}, active-layer pixel ({x}, {})", i / 400);
        }
    }
}
