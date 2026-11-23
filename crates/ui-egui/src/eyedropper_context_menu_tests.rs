//! #11747: a right-click on the image with the Eyedropper opens its menu: the seven Sample Sizes
//! (the current one checked), then Copy Color as HTML and Copy Color's Hex Code for the colour
//! under the pointer, read from all layers with the Sample Size, whatever the Sample setting.

use craft::color_ui::ui::{SampleSize, SampleSource};
use craft::keymap::Mods;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas_tool_menu::{CanvasToolMenu, EYEDROPPER_COPY_HEX, EYEDROPPER_COPY_HTML, available, choose, open_at};
use crate::control::{ControlRequest, Reply, handle};
use crate::state::Tool;

/// 400 × 300 white with three layers: a red (220,30,30) block at 100,100–300,250 ("Red", active),
/// a blue (30,30,220) block at 320,100–380,160 above it ("Blue"), and a green (30,160,30) block at
/// 20,20–80,80 on a 50 % opacity layer ("Green"). The Eyedropper is the tool.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    let block = |app: &mut PhotocraftApp, name: &str, rect: [i32; 4], color: &str, opacity: f64| {
        app.run("layer.new.layer", json!({"name": name})).unwrap();
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": color})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        if opacity < 100.0 {
            app.run("layer.layerStyle.blendingOptions", json!({"opacity": opacity})).unwrap();
        }
    };
    block(&mut app, "Green", [20, 20, 60, 60], "#1ea01e", 50.0);
    block(&mut app, "Red", [100, 100, 200, 150], "#dc1e1e", 100.0);
    block(&mut app, "Blue", [320, 100, 60, 60], "#1e1edc", 100.0);
    let red = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == "Red").unwrap().2.id;
    app.run("layer.select", json!({"layer": red.0})).unwrap();
    app.ui.tool = Tool::Eyedropper;
    app
}

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
    let (req, _rx) = ControlRequest::new(method, params);
    match handle(app, ctx, &req) {
        Reply::Done(v) => v,
        _ => panic!("{method}: expected an immediate reply"),
    }
}

/// Right-click at document point `at` (the shell's path) and return the open menu.
fn right_click(app: &mut PhotocraftApp, at: [f64; 2]) -> CanvasToolMenu {
    assert!(open_at(app, Tool::Eyedropper, Mods::NONE, egui::pos2(50.0, 50.0), at, None), "a menu opens");
    app.ui.canvas_tool_menu.clone().unwrap()
}

/// What picking `command` in a freshly opened menu at `at` puts on the clipboard.
fn copied(app: &mut PhotocraftApp, at: [f64; 2], command: &str) -> Vec<String> {
    right_click(app, at);
    let ctx = app.ctx().clone();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| choose(app, ui.ctx(), command));
    out.textures_delta.clear();
    out.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect()
}

fn fg_bg(app: &PhotocraftApp) -> ([f32; 4], [f32; 4]) {
    (app.session.tools.foreground, app.session.tools.background)
}

#[test]
fn copy_rows_copy_the_colour_under_the_right_click_in_lower_case() {
    let mut app = app();
    assert_eq!(copied(&mut app, [200.0, 200.0], EYEDROPPER_COPY_HTML), ["color=\"#dc1e1e\""]);
    assert_eq!(copied(&mut app, [200.0, 200.0], EYEDROPPER_COPY_HEX), ["dc1e1e"]);
}

#[test]
fn the_copy_reads_all_layers_whatever_the_sample_setting() {
    let mut app = app();
    app.ui.tool_options.eyedropper.source = SampleSource::Current;
    // The blue is on the layer above the active Red layer.
    assert_eq!(copied(&mut app, [350.0, 130.0], EYEDROPPER_COPY_HEX), ["1e1edc"]);
    // The half-transparent green reads as the composite over white, not the layer's own green.
    let green = copied(&mut app, [50.0, 50.0], EYEDROPPER_COPY_HEX);
    let rgb = u32::from_str_radix(&green[0], 16).unwrap();
    let (r, g, b) = (rgb >> 16, (rgb >> 8) & 0xff, rgb & 0xff);
    assert!((141..=143).contains(&r) && (206..=208).contains(&g) && (141..=143).contains(&b), "composite of 50 % green over white: {green:?}");
}

#[test]
fn the_copy_honours_the_sample_size() {
    // A 3 × 3 box on the red block's corner: four of nine pixels red, the rest white.
    let mut app = app();
    app.ui.tool_options.eyedropper.size = SampleSize::from_pixels(3).unwrap();
    let corner = copied(&mut app, [100.5, 100.5], EYEDROPPER_COPY_HEX);
    app.ui.tool_options.eyedropper.size = SampleSize::from_pixels(1).unwrap();
    let point = copied(&mut app, [100.5, 100.5], EYEDROPPER_COPY_HEX);
    assert_eq!(point, ["dc1e1e"]);
    assert_ne!(corner, point, "the 3 by 3 average mixes in the white around the corner: {corner:?}");
}

#[test]
fn the_right_click_and_the_picks_change_no_colours_and_add_no_history() {
    let mut app = app();
    let before = fg_bg(&app);
    let steps = app.session.active().unwrap().history.past_len();
    for command in [EYEDROPPER_COPY_HTML, EYEDROPPER_COPY_HEX, "canvasMenu.eyedropper.size.5"] {
        copied(&mut app, [200.0, 200.0], command);
    }
    assert_eq!(fg_bg(&app), before, "foreground and background unchanged");
    assert_eq!(app.session.active().unwrap().history.past_len(), steps, "no history step");
}

#[test]
fn over_transparency_the_copy_rows_are_greyed() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    app.ui.tool = Tool::Eyedropper;
    let menu = right_click(&mut app, [10.0, 10.0]);
    assert!(!available(&app, &menu, EYEDROPPER_COPY_HTML) && !available(&app, &menu, EYEDROPPER_COPY_HEX));
    assert!(available(&app, &menu, "canvasMenu.eyedropper.size.3"), "the sizes stay available");
}

#[test]
fn over_the_control_channel_the_menu_is_listed_and_choose_picks_a_row() {
    let mut app = app();
    app.sync_views();
    let ctx = crate::canvas::testing::ctx(&app).clone();
    let r = crate::canvas::testing::control(
        &mut app,
        "ui.pointer",
        json!({"button": "secondary", "events": [{"kind": "down", "x": 200, "y": 200}, {"kind": "up", "x": 200, "y": 200}]}),
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.ui.canvas_tool_menu.is_some(), "the secondary press opened the menu");
    let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
    let ids: Vec<&str> =
        inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap().iter().filter_map(|e| e["id"].as_str()).collect();
    assert_eq!(ids.len(), 9, "{ids:?}");
    assert_eq!(ids[3], "canvasMenu.eyedropper.size.11");
    assert_eq!(ids[8], EYEDROPPER_COPY_HEX);
    assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id": "canvasMenu.eyedropper.size.31"}))["ok"], true);
    assert_eq!(app.ui.tool_options.eyedropper.size.pixels(), 31);
}
