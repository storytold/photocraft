//! UI tests for the adjustment editors: the whole app (eframe harness, CPU canvas) with an
//! adjustment layer on a new document, edited through the Properties panel with real pointer and
//! key input (issue #12: "add an adjustment layer to a new document, edit it, crash"). The
//! editors' own gestures are the framework's tests; these cover what the app wires around them.

use craft::behavior::dock::Action;
use craft::color_ui::ui::{ColorPickerDialog, PickedColor};
use craft::theme::Srgba;
use egui::accesskit::{Role, Toggled};
use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{Adjustment, LayerContent, LayerId};
use serde_json::json;

use crate::PhotocraftApp;

const ALL_KINDS: [&str; 16] = [
    "brightnessContrast",
    "levels",
    "curves",
    "exposure",
    "vibrance",
    "hueSaturation",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "invert",
    "posterize",
    "threshold",
    "gradientMap",
    "selectiveColor",
    "colorLookup",
];

/// Kinds whose Properties editor has a slider the first drag of `every_adjustment_layer_edits_through_properties` moves.
const SLIDER_KINDS: [&str; 11] = [
    "brightnessContrast",
    "exposure",
    "vibrance",
    "hueSaturation",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "posterize",
    "threshold",
    "selectiveColor",
];

fn app_harness(kind: &str, mode: &str, depth: u32) -> Harness<'static, Box<PhotocraftApp>> {
    app_harness_with(kind, mode, depth, 1.0 / 60.0, crate::Services::default())
}

/// [`app_harness`] with the app's `services` (a file source the test scripts).
fn app_harness_with(kind: &str, mode: &str, depth: u32, step_dt: f32, services: crate::Services) -> Harness<'static, Box<PhotocraftApp>> {
    let (kind, mode) = (kind.to_string(), mode.to_string());
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(step_dt).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 96, "height": 64, "mode": mode, "depth": depth})).unwrap();
        // Two tones so histograms (and Levels' Auto) have something to work with.
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 48, "height": 64})).unwrap();
        s.execute("edit.fill", json!({"color": "#6a3020"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, services);
        // The default Properties group gives way to Layers (#147) and scrolls taller editors;
        // these tests drive every control without scrolling, so they size it like a user would.
        crate::panel_docking::apply(&mut app, vec![Action::resize_group("properties".to_owned(), Some(560.0))]).unwrap();
        app
    });
    // The Color panel above Properties has sliders of its own; these tests drive the Properties editor's.
    let ctx = h.ctx.clone();
    let (request, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color"]}}));
    crate::control::handle(h.state_mut(), &ctx, &request);
    h.run_steps(8);
    h
}

fn layer(h: &Harness<'_, Box<PhotocraftApp>>) -> (LayerId, Adjustment) {
    let st = h.state().session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Adjustment(a) => (id, a.clone()),
        other => panic!("{other:?}"),
    }
}

fn history_len(h: &Harness<'_, Box<PhotocraftApp>>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

fn drag(h: &mut Harness<'_, Box<PhotocraftApp>>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

fn label_rect(h: &Harness<'_, Box<PhotocraftApp>>, text: &str) -> Option<Rect> {
    // The last caption: the Properties header repeats the kind's name above its controls.
    // `get_by_label` hides a caption that names a control, so ask for the label node itself;
    // a checkbox paints its caption as the control, so fall back to the last matching widget.
    h.query_all(egui_kittest::kittest::by().label(text).include_labels())
        .rfind(|n| n.accesskit_node().role() == Role::Label)
        .map(|n| n.rect())
        .or_else(|| h.query_all_by_label(text).last().map(|n| n.rect()))
}

/// The rectangles of the editor's sliders, top to bottom.
fn sliders(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<Rect> {
    let mut rects: Vec<Rect> = h.query_all_by_role(Role::Slider).map(|n| n.rect()).collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

/// Issue #12: every adjustment kind, on a new document at 8/16/32 bits and in RGB, Grayscale,
/// CMYK and Lab, opens its Properties editor and takes an edit through real input without
/// panicking; slider kinds commit exactly one history step per gesture and end the preview with it.
#[test]
fn every_adjustment_layer_edits_through_properties() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("cmyk", 8), ("lab", 16)] {
        for kind in ALL_KINDS {
            let mut h = app_harness(kind, mode, depth);
            let (_, before) = layer(&h);
            let steps = history_len(&h);
            let slider = sliders(&h).first().copied();
            if SLIDER_KINDS.contains(&kind) {
                let r = slider.unwrap_or_else(|| panic!("{kind} {mode}: no slider in the Properties editor"));
                drag(&mut h, r.center() - vec2(r.width() * 0.2, 0.0), r.center() + vec2(r.width() * 0.2, 0.0));
            } else if kind == "gradientMap" {
                let r = label_rect(&h, "Reverse").unwrap_or_else(|| panic!("{kind} {mode}: Reverse"));
                click(&mut h, r.center());
            }
            h.run_steps(4);
            let (_, after) = layer(&h);
            assert!(h.state().live_adjust.is_none(), "{kind} {mode}: preview ended with the gesture");
            if SLIDER_KINDS.contains(&kind) || kind == "gradientMap" {
                assert_ne!(after, before, "{kind} {mode}@{depth}: the edit reached the layer");
                assert_eq!(history_len(&h), steps + 1, "{kind} {mode}: one history step");
            }
        }
    }
}

/// #3264: wheel browsing previews without recording one edit per selected LUT.
#[test]
fn color_lookup_picker_cycles_luts_with_the_mouse_wheel() {
    let mut h = app_harness("colorLookup", "rgb", 8);
    let (_, before) = layer(&h);
    let Adjustment::ColorLookup { name: before_name, .. } = before else { panic!("Color Lookup layer") };
    let label = label_rect(&h, "3D LUT File").expect("Color Lookup file label");
    let at = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .min_by(|a, b| a.rect().center().distance(label.center()).total_cmp(&b.rect().center().distance(label.center())))
        .expect("Color Lookup picker")
        .rect()
        .center();
    let steps = h.state().session.active().unwrap().history.past_len();
    h.hover_at(at);
    h.run_steps(1);
    h.query_all_by_role(egui::accesskit::Role::ComboBox)
        .min_by(|a, b| a.rect().center().distance(label.center()).total_cmp(&b.rect().center().distance(label.center())))
        .expect("Color Lookup picker")
        .click();
    h.run_steps(1);
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(1);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "arrow navigation previews");
    assert!(h.state().live_adjust.is_some(), "arrow navigation selects a preview LUT");
    for _ in 0..3 {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "cycling remains a preview");
    assert!(h.state().live_adjust.is_some(), "the selected LUT is previewed");
    // The wheel run commits when the wheel gap passes without a notch (the harness steps 1/60 s).
    h.run_steps((craft::behavior::GESTURE_GAP * 60.0).ceil() as usize + 4);
    let (_, after) = layer(&h);
    let Adjustment::ColorLookup { name: after_name, .. } = after else { panic!("Color Lookup layer") };
    assert_ne!(after_name, before_name, "wheel browsing selects a different LUT");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one wheel gesture makes one adjustment edit");
    assert!(h.state().live_adjust.is_none(), "the completed wheel gesture is committed");
}

fn color_balance_of(a: &Adjustment) -> ([f32; 3], [f32; 3], [f32; 3], bool) {
    match a {
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => (*shadows, *midtones, *highlights, *preserve_luminosity),
        other => panic!("{other:?}"),
    }
}

/// The tone radio Color Balance shows as chosen.
fn shown_tone(h: &Harness<'_, Box<PhotocraftApp>>) -> Option<&'static str> {
    ["Shadows", "Midtones", "Highlights"].into_iter().find(|name| h.query_all_by_label(name).any(|n| n.accesskit_node().toggled() == Some(Toggled::True)))
}

/// Photoshop 25.4, Color Balance in the Properties panel: edits that follow each other make one
/// history step; Reset first returns to the settings the layer had when the panel started showing
/// it (Tone choice kept), then to the defaults (every tone 0, Midtones, Preserve Luminosity kept),
/// within that step.
#[test]
fn color_balance_properties_history_and_reset_like_photoshop() {
    let mut h = app_harness("colorBalance", "rgb", 8);
    let (id, _) = layer(&h);
    let start = ([10.0, 0.0, 0.0], [40.0, -30.0, 20.0], [0.0, 0.0, -15.0], false);
    h.state_mut()
        .run(
            "layer.setAdjustment",
            json!({"layer": id.0, "shadows": [10, 0, 0], "midtones": [40, -30, 20], "highlights": [0, 0, -15], "preserveLuminosity": false}),
        )
        .unwrap();
    // Show another layer, then this one again: the panel starts showing it with these settings.
    let bg = h.state().session.active().unwrap().doc.layers[0].id;
    h.state_mut().session.select_layer(bg).unwrap();
    h.run_steps(4);
    h.state_mut().session.select_layer(id).unwrap();
    h.run_steps(4);
    let cb = |h: &Harness<'_, Box<PhotocraftApp>>| color_balance_of(&layer(h).1);
    let track = *sliders(&h).first().expect("the Cyan · Red slider");
    let before = history_len(&h);
    // Two clicks far apart (not a double-click): the second follows the first within one step.
    click(&mut h, Pos2::new(track.left() + track.width() * 0.25, track.center().y));
    let first = cb(&h).1[0];
    click(&mut h, Pos2::new(track.left() + track.width() * 0.75, track.center().y));
    let second = cb(&h).1[0];
    assert!(first < 0.0 && second > 0.0, "the clicks jump to the pointer: {first} then {second}");
    assert_eq!(history_len(&h), before + 1, "two edits in a row: one history step");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    assert_eq!(cb(&h), start, "one Undo returns to the settings before them");
    // Edit with Highlights shown; Reset returns to the settings the panel started with.
    let hl = label_rect(&h, "Highlights").expect("Highlights radio");
    click(&mut h, hl.center());
    assert_eq!(shown_tone(&h), Some("Highlights"));
    click(&mut h, Pos2::new(track.left() + track.width() * 0.25, track.center().y));
    assert_ne!(cb(&h), start);
    let reset = h.get_by_label("Reset to defaults").rect();
    let before = history_len(&h);
    click(&mut h, reset.center());
    assert_eq!(cb(&h), start, "first Reset: the settings the panel started with");
    assert_eq!(shown_tone(&h), Some("Highlights"), "Tone choice kept");
    click(&mut h, reset.center());
    assert_eq!(cb(&h), ([0.0; 3], [0.0; 3], [0.0; 3], false), "second Reset: defaults, Preserve Luminosity kept");
    assert_eq!(shown_tone(&h), Some("Midtones"), "defaults show Midtones");
    click(&mut h, reset.center());
    assert_eq!(cb(&h), ([0.0; 3], [0.0; 3], [0.0; 3], false), "third Reset: still the defaults");
    assert_eq!(history_len(&h), before, "the edit and the Resets share one history step");
}

/// UI-217-15 (#3110, #3116): the Properties panel of a Curves or Levels layer has the Set
/// Black/Neutral Gray/White Point eyedroppers. With Set White Point armed, a click on the image
/// makes that pixel white as one history step (sampled below the layer), the picker stays armed for
/// another click, and Esc disarms it.
#[test]
fn properties_white_point_eyedropper_edits_the_layer() {
    for kind in ["curves", "levels"] {
        let mut h = app_harness(kind, "rgb", 8);
        let (_, before) = layer(&h);
        let steps = history_len(&h);
        h.get_by_label("Set White Point").click();
        h.run_steps(2);
        assert!(crate::layer_picker::armed(h.state()).is_some(), "{kind}: armed from the Properties panel");
        // The left half is #6a3020: the white point moves to it, so it composites white.
        let view = crate::canvas::testing::view(h.state()).expect("the canvas has a view");
        let at = view.to_screen(craft::behavior::kurbo::Point::new(10.0, 10.0));
        click(&mut h, at);
        let px: Vec<f32> = serde_json::from_value(h.state_mut().run("document.pixel", json!({"x": 10, "y": 10})).unwrap()).unwrap();
        assert!(px.iter().take(3).all(|v| *v > 0.99), "{kind}: {px:?}");
        assert_ne!(layer(&h).1, before, "{kind}: the layer changed");
        assert_eq!(history_len(&h), steps + 1, "{kind}: one history step");
        assert!(crate::layer_picker::armed(h.state()).is_some(), "{kind}: stays armed for another click");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().ui.layer_picker.is_none(), "{kind}: Esc disarms");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(layer(&h).1, before, "{kind}: undo restores the layer");
    }
}

fn photo_filter_color(a: &Adjustment) -> [f32; 3] {
    match a {
        Adjustment::PhotoFilter { color, .. } => *color,
        other => panic!("{other:?}"),
    }
}

/// Clicks the colour well labelled `label` and returns the Color Picker it opened.
fn open_picker(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str) -> craft::ui::dialog::DialogId {
    // The panel scrolls: bring the well into view before the pointer goes to it.
    h.get_by_role_and_label(Role::ColorWell, label).scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::ColorWell, label).click();
    h.run_steps(3);
    crate::color_picker_ui::top(h.state()).expect("the colour well opens the Color Picker")
}

fn show_in_picker(h: &mut Harness<'_, Box<PhotocraftApp>>, picker: craft::ui::dialog::DialogId, hex: &str) {
    let color = Srgba::from_hex(hex).expect("a hex colour");
    h.state_mut().framework.dialogs.get_mut_by_id::<ColorPickerDialog>(picker).expect("the picker is open").set_color(PickedColor::rgb(color));
    h.run_steps(3);
}

#[test]
fn photo_filter_colour_picker_previews_live_cancel_restores_and_ok_is_one_step() {
    let mut h = app_harness("photoFilter", "rgb", 8);
    let (id, _) = layer(&h);
    h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "color": "#336699"})).unwrap();
    h.run_steps(4);
    h.get_by_role_and_label(Role::RadioButton, "Color").click();
    h.run_steps(3);
    let start = photo_filter_color(&layer(&h).1);
    let before = history_len(&h);

    // While the picker is open, the canvas previews its colour; nothing is committed yet.
    let picker = open_picker(&mut h, "Filter Color");
    show_in_picker(&mut h, picker, "#ff0000");
    let live = h.state().live_adjust.clone();
    assert!(matches!(&live, Some((l, v)) if *l == id && v["color"] == "#ff0000"), "live preview while the picker is open: {live:?}");
    assert_eq!(photo_filter_color(&layer(&h).1), start);
    assert_eq!(history_len(&h), before);

    // Cancel: back to the opening colour, no history step.
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.cancel_by_id(&ctx, picker);
    h.run_steps(4);
    assert!(h.state().live_adjust.is_none(), "Cancel ends the preview");
    assert_eq!(photo_filter_color(&layer(&h).1), start);
    assert_eq!(history_len(&h), before);

    // OK: the colour, as one history step however long it previewed.
    let picker = open_picker(&mut h, "Filter Color");
    show_in_picker(&mut h, picker, "#00ff80");
    show_in_picker(&mut h, picker, "#00ff00");
    assert_eq!(history_len(&h), before);
    crate::dialogs::confirm(h.state_mut(), &ctx, picker).expect("OK is enabled");
    h.run_steps(4);
    assert!(h.state().live_adjust.is_none());
    assert_eq!(photo_filter_color(&layer(&h).1), [0.0, 1.0, 0.0]);
    assert_eq!(history_len(&h), before + 1, "OK: one history step");
}

/// Opening a Gradient Map stop's Color Picker and closing it without a new colour (Cancel, or OK
/// unchanged) edits nothing: a stop at a location finer than a 0.001 grid (a PSD's 30 % is
/// 1229/4096) stays as it is, with no history step.
#[test]
fn gradient_map_stop_picker_closed_without_a_colour_changes_nothing() {
    let mut h = app_harness("gradientMap", "rgb", 16);
    let (id, _) = layer(&h);
    let fine = 1229.0 / 4096.0;
    h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "stops": [[0, "#000000"], [fine, "#ff0000"], [1, "#ffffff"]]})).unwrap();
    h.run_steps(4);
    let before = layer(&h).1;
    let history = history_len(&h);
    let ctx = h.ctx.clone();
    for button in ["Cancel", "OK"] {
        let picker = open_picker(&mut h, "Color");
        if button == "OK" {
            crate::dialogs::confirm(h.state_mut(), &ctx, picker).expect("OK is enabled");
        } else {
            h.state_mut().framework.dialogs.cancel_by_id(&ctx, picker);
        }
        h.run_steps(4);
        assert!(crate::color_picker_ui::top(h.state()).is_none(), "{button}: the picker closed");
        assert_eq!(layer(&h).1, before, "{button}: the stops are untouched");
        assert_eq!(history_len(&h), history, "{button}: no history step");
        assert!(h.state().live_adjust.is_none());
    }
}

/// #9269: Black & White's Auto button sets the layer's weights from the image under it, as one
/// history step.
#[test]
fn black_white_auto_button_sets_the_weights() {
    let mut h = app_harness("blackWhite", "rgb", 8);
    // A red and a blue the default mix turns into the same grey, under the adjustment layer.
    h.state_mut()
        .session
        .edit("setup", |doc, _| {
            let b = doc.bounds();
            let bg = doc.layers.first_mut().and_then(|l| l.surface_mut()).unwrap();
            let fmt = bg.format();
            let mut data = Vec::new();
            for _ in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    let c = if x < b.x1 / 2 { [0.776, 0.2, 0.2, 1.0] } else { [0.29, 0.29, 1.0, 1.0] };
                    data.extend(photocraft_raster::from_rgba(&fmt, c));
                }
            }
            bg.write_region(b, &data);
            Ok(())
        })
        .unwrap();
    h.run_steps(2);
    let (_, before) = layer(&h);
    let steps = history_len(&h);
    let auto = h.query_all_by_label("Auto").last().map(|n| n.rect().center()).expect("an Auto button");
    click(&mut h, auto);
    let (_, after) = layer(&h);
    let Adjustment::BlackWhite { weights, .. } = &after else { panic!("{after:?}") };
    let id = layer(&h).0;
    let expected = h.state_mut().run("image.adjustments.blackWhite.auto", json!({"layer": id.0})).unwrap();
    let want: Vec<f32> = photocraft_engine::adjust_params::HUE_RANGES.iter().map(|k| expected[*k].as_f64().unwrap() as f32).collect();
    assert_eq!(weights.to_vec(), want);
    assert_ne!(after, before, "the red and blue are told apart");
    assert_eq!(history_len(&h), steps + 1);
}

/// The screen x of input `level` (0..=255) on the Curves graph, from its end points' nodes.
fn curve_x(h: &Harness<'_, Box<PhotocraftApp>>, points: &[[f32; 2]], level: f32) -> f32 {
    let n = points.len();
    let first = h.get_by_label(&format!("Point 1 of {n}")).rect().center();
    let last = h.get_by_label(&format!("Point {n} of {n}")).rect().center();
    let (a, b) = (points[0][0], points[n - 1][0]);
    first.x + (level - a) / (b - a) * (last.x - first.x)
}

/// The master curve of the Curves layer, in levels.
fn master_points(h: &Harness<'_, Box<PhotocraftApp>>) -> Vec<[f32; 2]> {
    match layer(h).1 {
        Adjustment::Curves { master, .. } => master.iter().map(|p| [(p.input * 255.0).round(), (p.output * 255.0).round()]).collect(),
        other => panic!("{other:?}"),
    }
}

const BLACK_SLIDER: &str = "Input levels, Black input";
const WHITE_SLIDER: &str = "Input levels, White input";

/// #14384 in the Properties panel: the black and white input sliders under a Curves layer's graph move the end points'
/// inputs and keep their outputs, an end point keeps 4 levels from its neighbour, and each drag is one history step.
#[test]
fn properties_curves_input_sliders_move_the_end_points() {
    let mut h = app_harness("curves", "rgb", 8);
    assert_eq!(master_points(&h), vec![[0.0, 0.0], [255.0, 255.0]]);
    let steps = history_len(&h);
    let black = h.get_by_label(BLACK_SLIDER).rect().center();
    let to = curve_x(&h, &[[0.0, 0.0], [255.0, 255.0]], 40.0);
    drag(&mut h, black, Pos2::new(to, black.y));
    assert_eq!(master_points(&h), vec![[40.0, 0.0], [255.0, 255.0]], "the black end's input moves, its output stays");
    assert_eq!(history_len(&h), steps + 1, "one history step");
    // The panel's scroll bar takes presses along its right edge, over the right half of the white thumb.
    let white = h.get_by_label(WHITE_SLIDER).rect();
    let from = Pos2::new(white.left() + 1.0, white.center().y);
    let to = curve_x(&h, &[[40.0, 0.0], [255.0, 255.0]], 0.0);
    drag(&mut h, from, Pos2::new(to, from.y));
    assert_eq!(master_points(&h), vec![[40.0, 0.0], [44.0, 255.0]], "the white end stops 4 levels past the black one");
    assert_eq!(history_len(&h), steps + 2);
}

/// #10110: in the Properties panel the Curves graph takes the panel's width and the height left above the controls
/// under it, so it isn't square, and the controls under it stay in view. Its points still map levels on each axis.
#[test]
fn properties_curves_graph_fills_the_panel() {
    let mut h = app_harness("curves", "rgb", 8);
    let black = h.get_by_label("Point 1 of 2").rect().center();
    let white = h.get_by_label("Point 2 of 2").rect().center();
    let (wide, tall) = (white.x - black.x, black.y - white.y);
    assert!((wide - tall).abs() > 20.0, "not a square: {wide} × {tall}");
    let reset = h.get_by_label("Reset to defaults").rect();
    assert!(reset.bottom() <= h.ctx.content_rect().bottom(), "the controls under the editor stay in view: {reset:?}");
    let input = h.get_by_label(BLACK_SLIDER).rect().center();
    assert!((input.x - black.x).abs() < 0.5, "the black slider sits under the black end point");
    // A click at the levels (64, 192) of this graph adds a point there.
    let at = Pos2::new(black.x + wide * 64.0 / 255.0, black.y - tall * 192.0 / 255.0);
    click(&mut h, at);
    assert_eq!(master_points(&h), vec![[0.0, 0.0], [64.0, 192.0], [255.0, 255.0]]);
}

/// #14383 in the Properties panel: Alt on the drag of the black input slider shows the clipping display on the canvas,
/// and the display ends when Alt or the mouse is released.
#[test]
fn properties_curves_alt_drag_shows_clipping_until_alt_or_the_mouse_ends() {
    use craft::imaging::ui::adjust::LevelsClip;
    let mut h = app_harness("curves", "rgb", 8);
    let clipping = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().adjust_session.as_ref().and_then(|r| r.clipping);
    let black = h.get_by_label(BLACK_SLIDER).rect().center();
    // As a pointer reports it: the button event carries the modifiers held.
    let button =
        |pos: Pos2, pressed: bool, modifiers: egui::Modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let hold = |h: &mut Harness<'_, Box<PhotocraftApp>>, modifiers: egui::Modifiers| h.event(egui::Event::ModifiersChanged(modifiers));
    hold(&mut h, egui::Modifiers::ALT);
    h.hover_at(black);
    h.run_steps(1);
    h.event(button(black, true, egui::Modifiers::ALT));
    h.run_steps(1);
    h.hover_at(black + vec2(12.0, 0.0));
    h.run_steps(3);
    assert_eq!(clipping(&h), Some(LevelsClip::Shadows), "Alt-drag of the black slider");
    assert!(h.state().levels_clipping.is_some(), "the canvas draws the display");
    hold(&mut h, egui::Modifiers::NONE);
    h.hover_at(black + vec2(14.0, 0.0));
    h.run_steps(3);
    assert_eq!(clipping(&h), None, "Alt released");
    hold(&mut h, egui::Modifiers::ALT);
    h.hover_at(black + vec2(16.0, 0.0));
    h.run_steps(3);
    assert_eq!(clipping(&h), Some(LevelsClip::Shadows), "Alt again during the drag");
    h.event(button(black + vec2(16.0, 0.0), false, egui::Modifiers::ALT));
    h.run_steps(3);
    assert_eq!(clipping(&h), None, "mouse released");
    hold(&mut h, egui::Modifiers::NONE);
    h.run_steps(2);
    assert!(h.state().levels_clipping.is_none(), "the display's cache goes once nothing asks for it");
}

/// Alt (Option) held on the press of an input slider shows the clipping display at once, before the pointer moves,
/// and keeps it while the button stays down; the release ends it. An Alt-click on a triangle shows what it clips.
#[test]
fn properties_curves_alt_press_on_a_slider_shows_clipping_before_it_moves() {
    use craft::imaging::ui::adjust::LevelsClip;
    let mut h = app_harness("curves", "rgb", 8);
    let clipping = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().adjust_session.as_ref().and_then(|r| r.clipping);
    let button =
        |pos: Pos2, pressed: bool, modifiers: egui::Modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    for (name, end) in [(BLACK_SLIDER, LevelsClip::Shadows), (WHITE_SLIDER, LevelsClip::Highlights)] {
        let at = h.get_by_label(name).rect().center();
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.hover_at(at);
        h.run_steps(1);
        h.event(button(at, true, egui::Modifiers::ALT));
        h.run_steps(3);
        assert_eq!(clipping(&h), Some(end), "Alt-press of the {name}, the pointer still");
        assert!(h.state().levels_clipping.is_some(), "the canvas draws the display for the {name}");
        h.event(button(at, false, egui::Modifiers::ALT));
        h.run_steps(3);
        assert_eq!(clipping(&h), None, "the {name} released");
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(2);
    }
    assert_eq!(master_points(&h), vec![[0.0, 0.0], [255.0, 255.0]], "a press that does not move changes nothing");
}

/// The Show Clipping check box shows the display on the press of an end without Alt; unchecked, the press shows
/// nothing.
#[test]
fn properties_curves_show_clipping_shows_the_display_without_alt() {
    use craft::imaging::ui::adjust::LevelsClip;
    let mut h = app_harness("curves", "rgb", 8);
    let clipping = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().adjust_session.as_ref().and_then(|r| r.clipping);
    let press = |h: &mut Harness<'_, Box<PhotocraftApp>>, at: Pos2, pressed: bool| {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
    };
    let white = h.get_by_label(WHITE_SLIDER).rect().center();
    h.hover_at(white);
    h.run_steps(1);
    press(&mut h, white, true);
    assert_eq!(clipping(&h), None, "no Alt, Show Clipping off");
    press(&mut h, white, false);
    let check = label_rect(&h, "Show Clipping").expect("the Show Clipping check box shows").center();
    click(&mut h, check);
    let white = h.get_by_label(WHITE_SLIDER).rect().center();
    h.hover_at(white);
    h.run_steps(1);
    press(&mut h, white, true);
    assert_eq!(clipping(&h), Some(LevelsClip::Highlights), "Show Clipping on: the white slider's press");
    assert!(h.state().levels_clipping.is_some(), "the canvas draws the display");
    press(&mut h, white, false);
    assert_eq!(clipping(&h), None, "released");
}

/// Photoshop's Alt+2 to Alt+5 pick the Channel select's entry of a Levels or Curves layer in Properties: the
/// composite, Red, Green and Blue; Alt+6, past an RGB document's list, changes nothing. The key matches by its position,
/// so the character Option types with it on a Mac (£ for Option+3) doesn't matter; they work with the panel docked and
/// floating. While the Input field has the keys, Alt+3 stays with its text. The select shows each key.
#[test]
fn properties_alt_digits_pick_the_channel() {
    use egui::{Key, Modifiers};
    for kind in ["curves", "levels"] {
        let mut h = app_harness(kind, "rgb", 8);
        let channel = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().adjust_session.as_ref().map(|r| r.channel);
        let alt_digit = |h: &mut Harness<'_, Box<PhotocraftApp>>, key: Key, typed: &str| {
            h.event(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::ALT });
            h.event(egui::Event::Text(typed.into()));
            h.event(egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: Modifiers::ALT });
            h.run_steps(3);
        };
        assert_eq!(channel(&h), Some(0), "{kind}: the composite to begin with");
        let version = |h: &Harness<'_, Box<PhotocraftApp>>| crate::panels::region_versions::panel(h.state(), "properties");
        for (key, typed, want) in [(Key::Num3, "£", 1), (Key::Num4, "¢", 2), (Key::Num5, "∞", 3), (Key::Num6, "§", 3), (Key::Num2, "™", 0)] {
            let before = (channel(&h), version(&h));
            alt_digit(&mut h, key, typed);
            assert_eq!(channel(&h), Some(want), "{kind}: Alt+{key:?}");
            // An idle panel replays its last run until its content version changes.
            assert_eq!(version(&h) != before.1, channel(&h) != before.0, "{kind}: Alt+{key:?} renews the panel's content version");
        }
        let ctx = h.ctx.clone();
        let shown: Vec<String> = super::channel_keys::commands()
            .iter()
            .take(4)
            .map(|command| craft::ui::runtime::commands::chord(&ctx, command).map(|chord| chord.display(craft::host::target::Os::Linux)).unwrap_or_default())
            .collect();
        assert_eq!(shown, ["Alt+2", "Alt+3", "Alt+4", "Alt+5"], "{kind}: the keys the select shows");
        // Properties floating in a window of its own takes the keys too.
        crate::panel_docking::apply(
            h.state_mut(),
            vec![Action::float("properties".to_owned(), Rect::from_min_size(Pos2::new(400.0, 200.0), vec2(320.0, 560.0)))],
        )
        .unwrap();
        h.run_steps(4);
        alt_digit(&mut h, Key::Num4, "¢");
        assert_eq!(channel(&h), Some(2), "{kind}: Alt+4 with Properties floating");
    }
    // A text field with the keys keeps Alt+3: the Curves Input field.
    let mut h = app_harness("curves", "rgb", 8);
    let white = h.get_by_label(WHITE_SLIDER).rect().center();
    click(&mut h, white);
    let input = h.get_by_label("Input").rect().center();
    click(&mut h, input);
    h.run_steps(2);
    h.event(egui::Event::Key { key: Key::Num3, physical_key: Some(Key::Num3), pressed: true, repeat: false, modifiers: Modifiers::ALT });
    h.event(egui::Event::Text("£".into()));
    h.run_steps(3);
    assert_eq!(h.state().adjust_session.as_ref().map(|r| r.channel), Some(0), "typing in Input: the channel stays");
}
