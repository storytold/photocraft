//! The Camera Raw dialog laid out as Photoshop's Camera Raw 16.1.1: panel eyes, value boxes,
//! knob resets, presets, the More menu's resets, the persisted layout and the Before/After
//! views, through the real shell and its control channel.
use egui::{Event, Modifiers, PointerButton, Pos2, accesskit::Role, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, p: Value) -> Result<Value, String> {
    camera_raw_ui::menu(app, ctx, "filter.cameraRaw", &p).ok_or("missing handler")?
}
fn ui(h: &mut Harness<'_, PhotocraftApp>, request: Value) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    let r = menu(h.state_mut(), &ctx, json!({ "ui": request }));
    h.run_steps(3);
    r
}
fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}
fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1632.0, 1046.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::ProMedium);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.set_theme(&cc.egui_ctx, ThemeKind::ProMedium);
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#708090"})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.run_steps(4);
    h
}
fn click(h: &mut Harness<'_, PhotocraftApp>, p: Pos2, times: usize) {
    h.event(Event::PointerMoved(p));
    for _ in 0..times {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn a_panel_eye_hides_its_edits_in_the_preview_only() {
    let mut h = fixture();
    let unedited = inspect(&h)["histogram"].clone();
    ui(&mut h, json!({"set":{"exposure":1.0}})).unwrap();
    let edited = inspect(&h)["histogram"].clone();
    assert_ne!(edited["green"], unedited["green"], "exposure changes the preview");
    // The Light panel's eye: the preview shows the image without Light's settings.
    let eye = h.get_by_label("Toggle visibility of Light settings").rect().center();
    click(&mut h, eye, 1);
    let hidden = inspect(&h);
    assert_eq!(hidden["hidden"], json!(["light"]));
    assert_eq!(hidden["histogram"]["green"], unedited["green"], "hidden panel left out of the preview");
    assert_eq!(hidden["params"]["exposure"], 1.0, "hiding never edits the settings");
    click(&mut h, eye, 1);
    assert_eq!(inspect(&h)["hidden"], json!([]));
    assert_eq!(inspect(&h)["histogram"]["green"], edited["green"]);
    // OK applies the settings even while a panel is hidden.
    ui(&mut h, json!({"hidden":["light"]})).unwrap();
    let history = h.state().session.active().unwrap().history.past_len();
    ui(&mut h, json!({"commit":true})).unwrap();
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
}

#[test]
fn value_boxes_take_typed_numbers_and_the_knob_resets_on_double_click() {
    let mut h = fixture();
    let slider = h.get_by_role_and_label(Role::Slider, "Contrast").rect();
    // The value box sits above the track's right end.
    let field = pos2(slider.right() - 24.0, slider.top() - 5.0);
    click(&mut h, field, 1);
    h.event(Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.event(Event::Text("35".into()));
    h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    assert_eq!(inspect(&h)["params"]["contrast"], 35.0, "typed value");
    // A double-click on the track resets to the default.
    let track = h.get_by_role_and_label(Role::Slider, "Contrast").rect();
    click(&mut h, track.center(), 2);
    assert_eq!(inspect(&h)["params"]["contrast"], 0.0, "double-click resets");
}

#[test]
fn presets_apply_with_an_amount_and_reset_to_open_restores_the_settings() {
    let mut h = fixture();
    ui(&mut h, json!({"layout":{"tool":"presets"}})).unwrap();
    h.get_by_label("B&W High Contrast").click();
    h.run_steps(4);
    let p = inspect(&h)["params"].clone();
    assert_eq!(p["monochrome"], true);
    assert_eq!(p["contrast"], 40.0);
    // Back to Edit: More › Reset to Open.
    ui(&mut h, json!({"layout":{"tool":"edit"}})).unwrap();
    let more = h.get_by_label("More image settings").rect().center();
    click(&mut h, more, 1);
    h.get_by_label("Reset to Open").click();
    h.run_steps(4);
    let p = inspect(&h)["params"].clone();
    assert_eq!((p["monochrome"].clone(), p["contrast"].clone()), (json!(false), json!(0.0)));
}

#[test]
fn layout_persists_between_openings_and_rejects_unknown_ids() {
    let mut h = fixture();
    ui(&mut h, json!({"layout":{"open":["optics","detail"],"groups":["defringe"],"curve":"point"}})).unwrap();
    assert!(h.query_by_label("Defringe").is_some(), "the open group is shown");
    assert!(h.query_by_label("Exposure").is_none(), "Light was closed");
    ui(&mut h, json!({"cancel":true})).unwrap();
    let ctx = h.ctx.clone();
    camera_raw_ui::open(h.state_mut(), &ctx).unwrap();
    h.run_steps(4);
    let layout = &h.state().ui.camera_raw_layout;
    assert_eq!(layout.open, ["optics", "detail"]);
    assert_eq!(layout.curve, "point");
    for bad in [
        json!({"layout":{"tool":"crop"}}),
        json!({"layout":{"open":["basic"]}}),
        json!({"layout":{"groups":["nope"]}}),
        json!({"layout":{"unknown":1}}),
        json!({"hidden":["light","nope"]}),
        json!({"beforeAfter":"diagonal"}),
        json!({"profileBrowser":"yes"}),
        json!({"set":{"upright":"sideways"}}),
    ] {
        assert!(ui(&mut h, bad.clone()).is_err(), "rejects {bad}");
    }
}

#[test]
fn before_after_views_cycle_and_copy_settings() {
    let mut h = fixture();
    ui(&mut h, json!({"set":{"exposure":1.0}})).unwrap();
    for (key, want) in [("leftRight", "leftRight"), ("topBottomSplit", "topBottomSplit"), ("off", "off")] {
        ui(&mut h, json!({"beforeAfter": key})).unwrap();
        assert_eq!(inspect(&h)["beforeAfter"], want);
    }
    // Q cycles the views.
    h.key_press(egui::Key::Q);
    h.run_steps(3);
    assert_eq!(inspect(&h)["beforeAfter"], "leftRight");
    // Copy After settings to Before, then Swap: the current settings become the Before ones.
    h.get_by_label("Copy After settings to Before (Ctrl+Alt+P)").click();
    h.run_steps(3);
    h.get_by_label("Swap Before and After settings (Ctrl+Alt+Shift+P)").click();
    h.run_steps(3);
    assert_eq!(inspect(&h)["params"]["exposure"], 1.0, "swapping equal settings keeps them");
    h.key_press(egui::Key::Q);
    h.run_steps(3);
    assert_eq!(inspect(&h)["beforeAfter"], "leftRightSplit");
}

#[test]
fn every_panel_and_tool_draws_without_editing_the_document() {
    let mut h = fixture();
    let doc = h.state().session.active().unwrap().doc.clone();
    let before = inspect(&h)["params"].clone();
    let groups = ["vignette", "grain", "sharpening", "noiseReduction", "colorNoiseReduction", "lensVignette", "defringe", "manualTransformations"];
    for panel in ["light", "color", "effects", "curve", "mixer", "grading", "detail", "optics", "geometry", "calibration"] {
        ui(&mut h, json!({"layout":{"tool":"edit","open":[panel],"groups":groups}})).unwrap();
        h.run_steps(3);
    }
    for (curve, mixer, adjust, tab, grading) in
        [("point", "pointColor", "hsl", "all", "shadows"), ("red", "mixer", "color", "hue", "global"), ("blue", "mixer", "hsl", "all", "threeWay")]
    {
        ui(&mut h, json!({"layout":{"open":["curve","mixer","grading"],"curve":curve,"mixer":mixer,"mixerAdjust":adjust,"mixerTab":tab,"grading":grading}}))
            .unwrap();
    }
    for tool in ["remove", "masking", "redEye", "presets", "edit"] {
        ui(&mut h, json!({"layout":{"tool":tool}})).unwrap();
    }
    ui(&mut h, json!({"profileBrowser":true})).unwrap();
    ui(&mut h, json!({"profileBrowser":false,"set":{"monochrome":true}})).unwrap();
    ui(&mut h, json!({"layout":{"open":["mixer"]}})).unwrap();
    assert!(h.query_by_label("B&W Mix").is_some(), "Monochrome turns the mixer into the B&W Mix");
    ui(&mut h, json!({"set":{"monochrome":false}})).unwrap();
    assert_eq!(inspect(&h)["params"], before);
    assert!(std::sync::Arc::ptr_eq(&doc, &h.state().session.active().unwrap().doc));
}
