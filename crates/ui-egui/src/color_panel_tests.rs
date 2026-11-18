//! The Color panel's host (#6000, #2569, #9592): its state contract, its saved options, its edits
//! (`tools.setColors`, one history step per gesture, and the foreground recolour of what is
//! selected) and its commands. How the panel draws and edits a colour is `ColorPanel`'s, covered
//! in craft-color.

use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use craft::color_ui::ui::{ColorPanelCommand, ColorPanelMode, ColorPanelRamp};

use crate::PhotocraftApp;
use crate::theme::AppTheme;

fn new_app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

/// The Color tab of `kind` for `app`, alone in a 300 x 400 pt group that it fills.
fn color_tab_of(kind: AppTheme, app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(300.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::panel_docking::probe::body(app, ui, "color");
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(3);
    h
}

fn bytes(c: [f32; 4]) -> [u8; 3] {
    [c[0], c[1], c[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// `ui.set colorPanel` merges, so setting the mode keeps the edited well; bad values change nothing.
#[test]
fn control_sets_the_mode_ramp_and_web_safe_ramp() {
    let mut app = new_app();
    let ctx = app.ctx().clone();
    let call = |app: &mut PhotocraftApp, params: serde_json::Value| {
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", params);
        match crate::control::handle(app, &ctx, &req) {
            crate::control::Reply::Done(v) => v["ok"] == true,
            _ => panic!("expected an immediate reply"),
        }
    };
    assert!(call(&mut app, json!({"colorPanel": {"background": true}})));
    assert!(call(&mut app, json!({"colorPanel": {"mode": "labSliders", "ramp": "grayscaleRamp", "webSafeRamp": true}})));
    let cp = app.ui.color_panel;
    assert!(cp.panel.background, "the well is kept");
    assert_eq!((cp.panel.mode, cp.panel.ramp, cp.panel.web_safe_ramp), (ColorPanelMode::LabSliders, ColorPanelRamp::GrayscaleRamp, true));
    for bad in [json!({"colorPanel": {"mode": "hueWheel"}}), json!({"colorPanel": {"ramp": 3}}), json!({"colorPanel": {"webSafeRamp": "yes"}})] {
        assert!(!call(&mut app, bad.clone()), "{bad}");
        assert_eq!(app.ui.color_panel, cp, "{bad}: nothing changes");
    }
}

/// The saved state keeps its shape: the panel's fields flat at the top level.
#[test]
fn saved_state_keeps_its_keys() {
    let old: crate::state::ColorPanelState = serde_json::from_value(json!({"background": true, "mode": "rgbSliders"})).unwrap();
    assert!(old.panel.background);
    assert_eq!(old.panel.mode, ColorPanelMode::RgbSliders);
    let new = serde_json::to_value(old).unwrap();
    assert_eq!((new.get("mode"), new.get("background")), (Some(&json!("rgbSliders")), Some(&json!(true))));
}

/// #9592: the options survive a restart (they ride in the preferences), as in Photoshop;
/// preferences without them keep the defaults. CodeRabbit on #9606: a change made outside the
/// panel (`prefs.set`, a reset) reaches the live panel instead of being written over.
#[test]
fn options_persist_restore_and_follow_outside_changes() {
    let mut first = new_app();
    first.ui.color_panel.panel.mode = ColorPanelMode::HsbSliders;
    first.ui.color_panel.panel.ramp = ColorPanelRamp::GrayscaleRamp;
    crate::color_panel_ui::persist(&mut first);
    let saved = first.session.prefs().clone();
    let mut next = new_app();
    next.session.prefs.edit(|p| *p = saved);
    crate::color_panel_ui::restore(&mut next);
    assert_eq!((next.ui.color_panel.panel.mode, next.ui.color_panel.panel.ramp), (ColorPanelMode::HsbSliders, ColorPanelRamp::GrayscaleRamp));
    let mut fresh = new_app();
    crate::color_panel_ui::restore(&mut fresh);
    assert_eq!(fresh.ui.color_panel.panel.mode, ColorPanelMode::HueCube, "no saved options: the default");

    let mut app = new_app();
    app.ui.color_panel.panel.mode = ColorPanelMode::HsbSliders;
    crate::color_panel_ui::persist(&mut app);
    let mut other = app.ui.color_panel;
    other.panel.mode = ColorPanelMode::GrayscaleSlider;
    app.session.prefs.edit(|p| p.color_panel = serde_json::to_value(other).unwrap());
    crate::color_panel_ui::persist(&mut app);
    assert_eq!(app.ui.color_panel.panel.mode, ColorPanelMode::GrayscaleSlider, "the live panel follows the preference");
    let stored: crate::state::ColorPanelState = serde_json::from_value(app.session.prefs().color_panel.clone()).unwrap();
    assert_eq!(stored.panel.mode, ColorPanelMode::GrayscaleSlider, "and the preference isn't overwritten");
    app.session.prefs.edit(|p| p.color_panel = serde_json::Value::Null);
    for pass in ["reset", "and it stays reset"] {
        crate::color_panel_ui::persist(&mut app);
        assert_eq!(app.ui.color_panel.panel.mode, ColorPanelMode::HueCube, "{pass}: the default mode");
    }
}

/// The Color tab in `mode` with a document holding one selected black rectangle, the Rectangle tool
/// in hand, foreground blue and background red, editing the background well if `background`.
fn color_tab_with_a_shape(mode: ColorPanelMode, background: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = new_app();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "rect", "rect": [20, 20, 50, 50], "fill": "#000000"})).unwrap();
    app.ui.tool = crate::state::Tool::Rectangle;
    app.session.tools.foreground = [0.0, 0.0, 1.0, 1.0];
    app.session.tools.background = [1.0, 0.0, 0.0, 1.0];
    app.ui.color_panel.panel.mode = mode;
    app.ui.color_panel.panel.background = background;
    color_tab_of(AppTheme::studio(), app)
}

/// Types `text` into the first channel's number field (Hue, Red) and commits it with Enter.
fn type_first_channel(h: &mut Harness<'_, PhotocraftApp>, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
}

fn shape_fill(app: &PhotocraftApp) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let photocraft_doc::LayerContent::Shape(sh) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("shape layer") };
    let Some(photocraft_doc::Fill::Solid(c)) = &sh.fill else { panic!("solid fill") };
    bytes([c.c[0], c.c[1], c.c[2], 1.0])
}

/// #5842 and G3: an edit changes the well the panel edits through `tools.setColors`; a foreground
/// edit recolours the selected shape (the Rectangle tool is in hand) in one history step, a
/// background edit recolours nothing.
#[test]
fn edits_set_the_edited_well_and_only_the_foreground_recolours() {
    // (mode, edits the background, typed into the first channel, foreground, background, shape fill)
    let cases = [
        (ColorPanelMode::HsbSliders, false, "120", [0, 255, 0], [255, 0, 0], [0, 255, 0]),
        (ColorPanelMode::HsbSliders, true, "120", [0, 0, 255], [0, 255, 0], [0, 0, 0]),
        (ColorPanelMode::RgbSliders, false, "255", [255, 0, 255], [255, 0, 0], [255, 0, 255]),
        (ColorPanelMode::RgbSliders, true, "0", [0, 0, 255], [0, 0, 0], [0, 0, 0]),
    ];
    for (mode, background, typed, fg, bg, fill) in cases {
        let case = format!("{mode:?}, background {background}, {typed}");
        let mut h = color_tab_with_a_shape(mode, background);
        let steps = h.state().session.active().unwrap().history.entries().len();
        type_first_channel(&mut h, typed);
        let app = h.state();
        assert_eq!((bytes(app.session.tools.foreground), bytes(app.session.tools.background)), (fg, bg), "{case}");
        assert_eq!(shape_fill(app), fill, "{case}");
        let added = app.session.active().unwrap().history.entries().len() - steps;
        assert_eq!(added, usize::from(!background), "{case}: one step for the recolour, none for a tool colour");
    }
}

/// The commands the panel hands its host: the menu's Copy Color as HTML and Copy Color's Hex Code
/// give the edited colour's text, and Add to Swatches (beside the Color Wheel) adds it as a swatch.
#[test]
fn copy_commands_and_add_to_swatches() {
    let mut app = new_app();
    app.session.tools.foreground = [0.2, 0.6, 0.8, 1.0];
    app.session.tools.background = [1.0, 1.0, 1.0, 1.0];
    let copy = |app: &mut PhotocraftApp, command| crate::panels::color::run_menu_command(app, command);
    assert_eq!(copy(&mut app, ColorPanelCommand::CopyHtml).as_deref(), Some("color=\"#3399cc\""));
    assert_eq!(copy(&mut app, ColorPanelCommand::CopyHex).as_deref(), Some("3399cc"));
    app.ui.color_panel.panel.background = true;
    assert_eq!(copy(&mut app, ColorPanelCommand::CopyHex).as_deref(), Some("ffffff"), "the edited well's colour");

    app.ui.color_panel.panel.background = false;
    app.ui.color_panel.panel.mode = ColorPanelMode::ColorWheel;
    let count = |app: &PhotocraftApp| app.session.presets.swatches.iter().map(|group| group.entries.len()).sum::<usize>();
    let before = count(&app);
    let mut h = color_tab_of(AppTheme::pro(), app);
    h.get_by_label("Add to Swatches").click();
    h.run_steps(3);
    assert_eq!(count(h.state()), before + 1, "one swatch of the foreground");
}
