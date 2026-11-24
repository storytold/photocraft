use super::*;
use craft::imaging::ui::adjust::TonePicker;
use craft::ui::dialog::DialogId;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::adjust_editors::values;

fn app_with_image() -> Box<PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#b04020"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

fn app_in_mode(mode: &str) -> Box<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 32, "height": 24, "mode": mode})).unwrap();
    PhotocraftApp::new(session, crate::Services::default())
}

/// The hosted dialog `id` names.
fn hosted(app: &mut PhotocraftApp, id: DialogId) -> &mut HostedAdjustDialog {
    app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id).expect("the hosted adjustment dialog is open")
}

/// The top dialog of the stack.
fn top(app: &PhotocraftApp) -> DialogId {
    app.framework.dialogs.stack().top().expect("a dialog is open")
}

/// The adjustment kind (`levels`, `curves`, ..) of the open dialog `id`.
fn kind_of(app: &PhotocraftApp, id: DialogId) -> String {
    open_value(app).filter(|open| open.id == id).expect("the adjustment dialog is open").kind.clone()
}

/// The engine parameters of the dialog's value: the keys the tests and recorded actions use.
fn params(app: &mut PhotocraftApp, id: DialogId) -> Value {
    let dialog = hosted(app, id);
    dialog.value().params(dialog.edited())
}

/// Changes the dialog's value through its engine parameters, as automation edits it.
fn edit_params(app: &mut PhotocraftApp, id: DialogId, edit: impl FnOnce(&mut Map<String, Value>)) {
    let kind = kind_of(app, id);
    let mut current = params(app, id);
    edit(current.as_object_mut().expect("object parameters"));
    let dialog = hosted(app, id);
    let value = values::from_params(&kind, &current, &dialog.value().cx).expect("the edited parameters read back");
    dialog.value_mut().value = value;
}

/// Points the dialog `id` at `address` (`target`, `layer`, `document`), replacing the address it was opened with,
/// as a retarget through [`apply_params`] does.
fn retarget(app: &mut PhotocraftApp, id: DialogId, address: &Value) {
    let open = app.adjust_open.as_mut().filter(|open| open.id == id).expect("the dialog is the open adjustment");
    let dialog = app.framework.dialogs.get_mut_by_id::<HostedAdjustDialog>(id).expect("the hosted adjustment dialog is open");
    for held in [&mut dialog.value_mut().cx.base, &mut open.recorded] {
        let held = held.as_object_mut().expect("object parameters");
        for key in ["target", "layer", "document"] {
            held.remove(key);
        }
        held.extend(address.as_object().expect("object address").clone());
    }
}

/// Writes the keys of `extra` over the dialog's parameters.
fn set_params(app: &mut PhotocraftApp, id: DialogId, extra: &Value) {
    edit_params(app, id, |current| current.extend(extra.as_object().expect("object parameters").clone()));
}

fn field(app: &mut PhotocraftApp, id: DialogId, key: &str) -> Value {
    params(app, id).get(key).cloned().unwrap_or(Value::Null)
}

fn set_field(app: &mut PhotocraftApp, id: DialogId, key: &str, value: Value) {
    edit_params(app, id, |current| {
        current.insert(key.into(), value);
    });
}

/// Arms `picker` on the dialog `id`.
fn arm(app: &mut PhotocraftApp, id: DialogId, picker: TonePicker) {
    hosted(app, id).set_picker(Some(picker));
}

fn picker_armed(app: &PhotocraftApp) -> bool {
    crate::dialogs::canvas_input::armed(app).is_some()
}

/// A canvas press at document point `(x, y)` with the armed tone picker.
fn sample_at(app: &mut PhotocraftApp, x: f64, y: f64) -> bool {
    crate::dialogs::canvas_input::pick_tone(app, [x, y])
}

/// The points of the top dialog's curve `key`.
fn channel_points(app: &mut PhotocraftApp, key: &str) -> Vec<[f64; 2]> {
    let id = top(app);
    field(app, id, key)
        .as_array()
        .map(|points| {
            points
                .iter()
                .filter_map(|point| {
                    let point = point.as_array()?;
                    Some([point.first()?.as_f64()?, point.get(1)?.as_f64()?])
                })
                .collect()
        })
        .unwrap_or_default()
}

/// OK on the dialog `id` through the host.
fn ok(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) -> Result<(), String> {
    crate::dialogs::confirm(app, ctx, id).map(|_| ())
}

/// Cancel on the dialog `id` through the host.
fn cancel(app: &mut PhotocraftApp, ctx: &egui::Context, id: DialogId) {
    crate::dialogs::cancel(app, ctx, id).expect("the dialog cancels");
}

fn dialog_harness(app: Box<PhotocraftApp>, size: egui::Vec2) -> Harness<'static, Box<PhotocraftApp>> {
    // Real frame times: a double-click's two clicks must land within the double-click time.
    let h =
        Harness::builder().with_context(app.ctx().clone()).with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h
}

fn input(h: &mut Harness<'static, Box<PhotocraftApp>>, mods: egui::Modifiers, events: &[egui::Event]) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.run_steps(1);
    for e in events {
        h.event(e.clone());
        h.run_steps(1);
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

/// Holds Alt, which turns Cancel into Reset, and presses the Reset button.
fn press_reset(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    h.run_steps(2);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    h.run_steps(2);
    let reset = h.get_by_label("Reset").rect();
    let click = |pressed| egui::Event::PointerButton { pos: reset.center(), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::ALT };
    input(h, egui::Modifiers::ALT, &[egui::Event::PointerMoved(reset.center()), click(true), click(false)]);
}

/// Opens `kind` in a harness over `app`.
fn open_in_harness(app: Box<PhotocraftApp>, command: &str) -> (Harness<'static, Box<PhotocraftApp>>, DialogId) {
    let mut h = dialog_harness(app, egui::vec2(1200.0, 900.0));
    let id = open(h.state_mut(), command).unwrap();
    h.run_steps(3);
    (h, id)
}

fn sample_params(kind: &str) -> Value {
    match kind {
        "brightnessContrast" => json!({"brightness": 40}),
        "levels" => json!({"inBlack": 30, "gamma": 1.3}),
        "curves" => json!({"points": [[0, 0], [100, 150], [255, 255]]}),
        "exposure" => json!({"exposure": 1}),
        "vibrance" => json!({"vibrance": 50}),
        "hueSaturation" => json!({"reds": {"hue": 40}}),
        "colorBalance" => json!({"midtones": [40, 0, -20]}),
        "blackWhite" => json!({"reds": 150, "tint": true}),
        "photoFilter" => json!({"color": "#006dff", "density": 60}),
        "channelMixer" => json!({"red": [50, 50, 0, 0]}),
        "posterize" => json!({"levels": 3}),
        "threshold" => json!({"level": 90}),
        "gradientMap" => json!({"stops": [[0, "#200040"], [1, "#ffd080"]]}),
        "selectiveColor" => json!({"method": "absolute", "blues": [10, 0, -20, 0]}),
        _ => json!({}),
    }
}

/// Opens each dialog, changes a value, then cancels (document untouched) or confirms (one history step).
#[test]
fn dialogs_preview_cancel_and_commit() {
    for kind in crate::adjust_editors::KINDS {
        let cmd = format!("image.adjustments.{kind}");
        let mut app = app_with_image();
        let ctx = app.ctx().clone();
        let before = app.session.active().unwrap().revision;
        let doc = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        // Cancel after an edit leaves the document as it was.
        let id = open(&mut app, &cmd).unwrap();
        set_params(&mut app, id, &sample_params(kind));
        cancel(&mut app, &ctx, id);
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc), "{kind}: Cancel leaves the document");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{kind}: Cancel adds no step");
        // OK after an edit is one step; the preview before it never moves the revision.
        let id = open(&mut app, &cmd).unwrap();
        set_params(&mut app, id, &sample_params(kind));
        assert!(crate::adjust_preview::display_doc(&mut app, 0).is_some(), "{kind}: preview");
        assert_eq!(app.session.active().unwrap().revision, before, "{kind}: previewing leaves the document alone");
        let r = ok(&mut app, &ctx, id);
        assert!(r.is_ok(), "{kind}: {r:?}");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "{kind}: one history step");
    }
}

/// A pencil curve (a 256-entry map) set on the value reaches the engine as that map with `pencil` when OK is pressed.
#[test]
fn a_pencil_map_reaches_the_journal_on_ok() {
    let mut app = app_with_image();
    let ctx = app.ctx().clone();
    let steps = app.session.active().unwrap().history.past_len();
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    let mut map: Vec<f64> = (0..256).map(f64::from).collect();
    for level in map.iter_mut().take(193).skip(64) {
        *level = 200.0;
    }
    set_params(&mut app, id, &json!({"pencil": true, "points": map}));
    let shown = field(&mut app, id, "points");
    assert_eq!(shown.as_array().map(Vec::len), Some(256));
    assert_eq!(shown[20].as_f64(), Some(20.0), "outside the stroke the map is untouched");
    ok(&mut app, &ctx, id).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
    let (_, journaled) = app.session.journal.last().unwrap();
    assert_eq!(journaled["pencil"], json!(true));
    let committed = journaled["points"].as_array().unwrap();
    assert_eq!(committed.len(), 256);
    assert!((70..185).all(|k| (committed[k].as_f64().unwrap() - 200.0).abs() < 3.0), "{committed:?}");
}

#[test]
fn curves_eyedroppers_sample_repeatedly_without_editing_the_document() {
    let mut app = app_with_image();
    let original = app.session.active().unwrap().doc.clone();
    let revision = app.session.active().unwrap().revision;
    let steps = app.session.active().unwrap().history.past_len();
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    arm(&mut app, id, TonePicker::Black);
    assert!(picker_armed(&app));
    assert!(sample_at(&mut app, 8.0, 8.0));
    let first = channel_points(&mut app, "red");
    assert!((first[0][0] - 176.0).abs() < 0.6 && first[0][1] == 0.0, "{first:?}");
    assert!(sample_at(&mut app, 8.0, 8.0));
    assert_eq!(channel_points(&mut app, "red"), first, "the same repeated sample is deterministic");
    assert!(sample_at(&mut app, 48.0, 8.0));
    assert_ne!(channel_points(&mut app, "red"), first);
    assert!(sample_at(&mut app, 8.0, 8.0));
    assert_eq!(channel_points(&mut app, "red"), first, "repeated samples resolve from the armed baseline");
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &original));
    assert_eq!(st.revision, revision);
    assert_eq!(st.history.past_len(), steps);
}

/// #3116: the Levels dialog's Set White Point moves each channel's input white to the sample
/// (#b04020) and leaves the document alone until OK.
#[test]
fn levels_white_point_eyedropper_sets_the_channel_input_whites() {
    let mut app = app_with_image();
    let original = app.session.active().unwrap().doc.clone();
    let id = open(&mut app, "image.adjustments.levels").unwrap();
    arm(&mut app, id, TonePicker::White);
    assert!(picker_armed(&app));
    assert!(sample_at(&mut app, 8.0, 8.0));
    let shown = params(&mut app, id);
    let in_white = |key: &str| shown.get(key).and_then(|c| c.get("inWhite")).and_then(Value::as_f64);
    assert_eq!([in_white("red"), in_white("green"), in_white("blue")], [Some(176.0), Some(64.0), Some(32.0)]);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
}

#[test]
fn curves_picker_rejects_transparent_samples() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24, "background": "transparent"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    arm(&mut app, id, TonePicker::White);
    let before = params(&mut app, id);
    assert!(sample_at(&mut app, 4.0, 4.0));
    assert_eq!(params(&mut app, id), before);
    assert!(picker_armed(&app), "an invalid sample does not disarm the picker");
}

#[test]
fn curves_picker_modes_and_depths_are_explicit() {
    for depth in [8, 16, 32] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        s.execute("edit.fill", json!({"color": [0.3142, 0.4821, 0.7137]})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let sampled = crate::canvas::composite_color(&mut app, 4.0, 4.0).unwrap();
        let id = open(&mut app, "image.adjustments.curves").unwrap();
        arm(&mut app, id, TonePicker::Gray);
        assert!(sample_at(&mut app, 4.0, 4.0), "RGB @{depth}");
        let red = channel_points(&mut app, "red");
        let target_level = (f64::from(sampled[0]) * 0.2126 + f64::from(sampled[1]) * 0.7152 + f64::from(sampled[2]) * 0.0722) * 255.0;
        let sample_level = f64::from(sampled[0]) * 255.0;
        let serialized_sample_level = (sample_level * 100.0).round() / 100.0;
        let neutral_anchor = red.iter().find(|point| (point[0] - serialized_sample_level).abs() < 1e-5).unwrap();
        let serialized_target_level = (target_level * 100.0).round() / 100.0;
        assert!((neutral_anchor[1] - serialized_target_level).abs() < 1e-5, "depth={depth} anchor={neutral_anchor:?} sampled={sampled:?}");
        assert!((neutral_anchor[0] - sample_level).abs() <= 0.0051, "depth={depth} anchor={neutral_anchor:?} sampled={}", sampled[0]);
        if depth == 16 || depth == 32 {
            assert!((sampled[0] * 255.0 - (sampled[0] * 255.0).round()).abs() > 0.01, "depth={depth} sample was quantized to an 8-bit level: {}", sampled[0]);
        }
    }
}

/// A canvas pointer event at document point `(x, y)` with `mods` held, through the tool host as a mouse takes it.
fn pointer(app: &mut PhotocraftApp, event: crate::canvas::ToolEvent, mods: egui::Modifiers) {
    crate::canvas::testing::shown(app);
    crate::canvas::tool_event(app, event, mods);
}

/// A click at document point `(x, y)` with `mods` held.
fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    pointer(app, crate::canvas::ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    pointer(app, crate::canvas::ToolEvent::Up { x, y }, mods);
}

/// The Curves curve `pts` (0-255 levels) as a 256-entry 0-255 map.
fn curve_levels(pts: &[[f64; 2]]) -> Vec<f32> {
    let cp: Vec<_> = pts.iter().map(|q| photocraft_doc::adjust::CurvePoint { input: q[0] as f32 / 255.0, output: q[1] as f32 / 255.0 }).collect();
    let lut = photocraft_compose::adjust::curve_lut(&cp);
    (0..256).map(|k| lut[(k as f32 / 255.0 * (lut.len() - 1) as f32).round() as usize] * 255.0).collect()
}

/// #7325: ⌘/Ctrl-click on the image puts a point on the shown curve at the pixel's tone
/// (#b04020: Rec. 709 luma 85.5 → level 86) and at the curve's own output there, so the
/// image does not change; the new point is on the composite curve, and the document waits for OK.
#[test]
fn command_click_on_the_image_adds_a_point_on_the_composite_curve() {
    let mut app = app_with_image();
    let ctx = app.ctx().clone();
    let original = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    let base = [[0.0, 0.0], [128.0, 200.0], [255.0, 255.0]];
    set_params(&mut app, id, &json!({"points": base}));
    let sample = crate::canvas::composite_color(&mut app, 8.0, 8.0).unwrap();
    let level = ((sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722) * 255.0).round();
    assert_eq!(level, 86.0);
    let output = f64::from(curve_levels(&base)[86].round());
    assert_ne!(output, 86.0, "the base curve is not the identity at the sampled level");

    let _ = crate::dialogs::canvas_input::add_curve_point(&mut app, [8.0, 8.0], false);
    let points = channel_points(&mut app, "points");
    assert_eq!(points, vec![base[0], [86.0, output], base[1], base[2]]);
    let (before, after) = (curve_levels(&base), curve_levels(&points));
    assert!(before.iter().zip(&after).all(|(a, b)| (a - b).abs() <= 0.5), "the curve keeps its shape:\n{before:?}\n{after:?}");
    assert_eq!(hosted(&mut app, id).channel(), 0, "the composite curve is shown");
    for key in ["red", "green", "blue"] {
        assert_eq!(channel_points(&mut app, key), vec![[0.0, 0.0], [255.0, 255.0]], "{key} untouched");
    }
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    ok(&mut app, &ctx, id).unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
}

/// #7325: a plain click, a full curve and an armed eyedropper add nothing.
#[test]
fn command_click_adds_nothing_without_the_modifier_on_a_full_curve_or_with_an_eyedropper() {
    let mut app = app_with_image();
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    let before = params(&mut app, id);
    crate::canvas::testing::clear_of_dialogs(&mut app, [8.0, 8.0]);
    click(&mut app, 8.0, 8.0, egui::Modifiers::NONE);
    assert_eq!(params(&mut app, id), before, "no modifier, no point");

    // Sixteen points (the most a curve holds), none within reach of level 86.
    let full: Vec<[f64; 2]> = [0, 10, 20, 30, 40, 50, 60, 70, 80, 100, 120, 140, 160, 180, 200, 255].iter().map(|&i| [f64::from(i), f64::from(i)]).collect();
    set_params(&mut app, id, &json!({"points": full}));
    let _ = crate::dialogs::canvas_input::add_curve_point(&mut app, [8.0, 8.0], false);
    assert_eq!(channel_points(&mut app, "points"), full, "a full curve takes no more points and moves none");

    let mut app = app_with_image();
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    arm(&mut app, id, TonePicker::Black);
    crate::canvas::testing::clear_of_dialogs(&mut app, [8.0, 8.0]);
    click(&mut app, 8.0, 8.0, egui::Modifiers::COMMAND);
    assert_eq!(channel_points(&mut app, "points").len(), 2, "the armed eyedropper samples instead");
    assert_eq!(channel_points(&mut app, "red")[0], [176.0, 0.0]);
}

/// #7325: Lab and CMYK curves, a targeted mask or filter mask, a dialog whose recorded target is a saved
/// channel, and a mask captured by the dialog and retargeted afterwards take no canvas point.
/// A recorded `pixels` target still takes it.
#[test]
fn command_click_adds_nothing_in_lab_or_on_a_targeted_mask() {
    let mut lab = app_in_mode("lab");
    lab.run("edit.fill", json!({"color": "#b04020"})).unwrap();
    let id = open(&mut lab, "image.adjustments.curves").unwrap();
    let before = params(&mut lab, id);
    let _ = crate::dialogs::canvas_input::add_curve_point(&mut lab, [4.0, 4.0], false);
    assert_eq!(params(&mut lab, id), before);

    let mut masked = app_with_image();
    masked.run("layer.layerMask.revealAll", json!({})).unwrap();
    masked.run("channel.target", json!({"channel": "mask"})).unwrap();
    let id = open(&mut masked, "image.adjustments.curves").unwrap();
    let before = params(&mut masked, id);
    let _ = crate::dialogs::canvas_input::add_curve_point(&mut masked, [8.0, 8.0], false);
    assert_eq!(params(&mut masked, id), before);

    // A targeted filter mask is a gray plane too.
    let mut filtered = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    filtered.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    filtered.run("layer.new.layer", json!({})).unwrap();
    filtered.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    filtered.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    filtered.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
    let layer = filtered.session.active().unwrap().active_layer.unwrap();
    filtered.select_filter_mask_target(Some(layer.0));
    assert!(filtered.filter_mask_targeted());
    let id = open(&mut filtered, "image.adjustments.curves").unwrap();
    let before = params(&mut filtered, id);
    let _ = crate::dialogs::canvas_input::add_curve_point(&mut filtered, [8.0, 8.0], false);
    assert_eq!(params(&mut filtered, id), before);

    // The dialog's explicit target wins over the Channels panel's.
    let mut app = app_with_image();
    let ctx = app.ctx().clone();
    app.run("channel.new", json!({"name": "Saved sky", "fill": "white"})).unwrap();
    app.run("channel.target", json!({"channel": {"color": 0}})).unwrap();
    let id = open_with_params(&mut app, "image.adjustments.curves", &json!({"target": {"channel": 0}})).unwrap();
    let before = params(&mut app, id);
    let doc = app.session.active().unwrap().doc.clone();
    let _ = crate::dialogs::canvas_input::add_curve_point(&mut app, [8.0, 8.0], false);
    assert_eq!(params(&mut app, id), before, "a saved alpha cannot sample the colour composite");
    assert_eq!(app.session.active().unwrap().doc, doc);
    cancel(&mut app, &ctx, id);

    app.run("channel.target", json!({"channel": 0})).unwrap();
    let id = open_with_params(&mut app, "image.adjustments.curves", &json!({"target": "pixels"})).unwrap();
    let _ = crate::dialogs::canvas_input::add_curve_point(&mut app, [8.0, 8.0], false);
    assert_eq!(channel_points(&mut app, "points"), vec![[0.0, 0.0], [86.0, 86.0], [255.0, 255.0]]);
    assert_eq!(field(&mut app, id, "target"), "pixels");
    for key in ["red", "green", "blue"] {
        assert_eq!(channel_points(&mut app, key), vec![[0.0, 0.0], [255.0, 255.0]], "{key} untouched");
    }
    assert_eq!(app.session.active().unwrap().doc, doc, "adding a preview point leaves committed pixels alone");

    // A mask the dialog captured stays its target after the panel points elsewhere.
    for target in [json!({"channel": "composite"}), json!({"colors": [0, 2]})] {
        for every_channel in [false, true] {
            let mut app = app_with_image();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("channel.target", json!({"colors": [0, 2]})).unwrap();
            app.run("view.layerMask", json!({"mode": "gray"})).unwrap();
            let id = open(&mut app, "image.adjustments.curves").unwrap();
            let before = params(&mut app, id);
            app.run("channel.target", target.clone()).unwrap();
            let _ = crate::dialogs::canvas_input::add_curve_point(&mut app, [8.0, 8.0], every_channel);
            assert_eq!(params(&mut app, id), before, "{target} every_channel={every_channel}");
        }
    }
}

/// `ui.pointer` drives an armed Curves picker as the mouse does: a hover samples nothing, the press samples and the
/// release samples nothing more; the picker stays armed.
#[test]
fn ui_pointer_drives_an_armed_curves_picker() {
    let mut app = app_with_image();
    let id = open(&mut app, "image.adjustments.curves").unwrap();
    arm(&mut app, id, TonePicker::Black);
    let before = channel_points(&mut app, "red");
    let pointer = |app: &mut PhotocraftApp, kind: &str, x: f64, y: f64| {
        crate::canvas::testing::control(app, "ui.pointer", json!({"events": [{"kind": kind, "x": x, "y": y}]}));
    };
    crate::canvas::testing::clear_of_dialogs(&mut app, [8.0, 8.0]);
    pointer(&mut app, "move", 12.0, 8.0);
    assert_eq!(channel_points(&mut app, "red"), before, "pointer move does not sample");

    pointer(&mut app, "down", 8.0, 8.0);
    let red = channel_points(&mut app, "red");
    assert!((red[0][0] - 176.0).abs() < 0.6 && red[0][1] == 0.0, "{red:?}");
    assert_ne!(red, before, "pointer down samples");

    // Back to the curve before the press: a release that sampled would set it again.
    set_params(&mut app, id, &json!({"red": before}));
    pointer(&mut app, "up", 8.0, 8.0);
    assert_eq!(channel_points(&mut app, "red"), before, "pointer up does not sample again");
    assert!(picker_armed(&app));
}

/// #5335: in a Lab document, Curves over a gray target keeps neutral fields: no Lab records
/// to overwrite the mask's own curve or pick the wrong tone space.
#[test]
fn curves_on_a_gray_target_seeds_neutral_fields_in_a_lab_document() {
    for via_layers_panel in [false, true] {
        let mut app = app_in_mode("lab");
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        if via_layers_panel {
            app.ui.mask_target = true;
        } else {
            app.run("channel.target", json!({"channel": "mask"})).unwrap();
        }
        let id = open(&mut app, "image.adjustments.curves").expect("opens");
        let values = params(&mut app, id);
        assert!(values.get("points").is_some(), "{values:?}");
        for key in ["lightness", "a", "b", "cyan", "magenta", "yellow", "black"] {
            assert!(values.get(key).is_none(), "{key} in a gray target's curves: {values:?}");
        }
    }
}

/// A Lab OK followed by ⌥-reopen (the last settings): the combined record and the native lightness
/// record both come back.
#[test]
fn lab_subset_recall_preserves_combined_record_beside_native_lightness() {
    for kind in ["levels", "curves"] {
        let mut app = app_with_image();
        let ctx = app.ctx().clone();
        app.run("file.new", json!({"width":32,"height":24,"mode":"lab","depth":16})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        let command = format!("image.adjustments.{kind}");
        let id = open(&mut app, &command).unwrap();
        let (key, combined) = if kind == "curves" { ("points", json!([[0.0, 0.0], [100.0, 170.0], [255.0, 255.0]])) } else { ("gamma", json!(1.7)) };
        set_field(&mut app, id, key, combined.clone());
        let lightness = field(&mut app, id, "lightness");
        ok(&mut app, &ctx, id).unwrap();
        let id = open_last(&mut app, &command).unwrap();
        assert_eq!(field(&mut app, id, key), combined);
        assert_eq!(field(&mut app, id, "lightness"), lightness);
        assert_eq!(field(&mut app, id, "target"), json!({"colors":[0,2]}));
    }
}

/// A dialog keeps the target it captured when it opened, so Reset does not follow a later
/// retarget: a mask stays gray, a native Lab target stays Lab.
#[test]
fn curves_reset_keeps_the_captured_mask_or_native_space_after_retargeting() {
    for mask in [false, true] {
        let mut app = app_with_image();
        app.run("file.new", json!({"width":32,"height":24,"mode":"lab","depth":16})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        if mask {
            app.run("view.layerMask", json!({"mode":"gray"})).unwrap();
        }
        let (mut h, id) = open_in_harness(app, "image.adjustments.curves");
        let target = field(h.state_mut(), id, "target");
        if mask {
            h.state_mut().run("channel.target", json!({"colors":[0,2]})).unwrap();
        } else {
            h.state_mut().run("view.layerMask", json!({"mode":"gray"})).unwrap();
        }
        press_reset(&mut h);
        assert!(h.state().framework.dialogs.stack().ids().any(|open| open == id), "Reset keeps the dialog open");
        let after = params(h.state_mut(), id);
        assert_eq!(after["target"], target);
        assert_eq!(after.get("a").is_some(), !mask, "mask curves stay gray; native curves stay Lab");
    }
}

/// #9879: the reference application's Photo Filter reopens at Warming Filter (85) and 25 % after
/// any OK, but keeps Preserve Luminosity as last confirmed (measured 2026-11-08); ⌥ reopens at all
/// the last settings; Levels still opens at its neutral settings.
#[test]
fn photo_filter_keeps_only_preserve_luminosity_from_its_last_ok() {
    let mut app = app_with_image();
    let ctx = app.ctx().clone();
    let cmd = "image.adjustments.photoFilter";
    let first = open(&mut app, cmd).unwrap();
    let neutral = params(&mut app, first);
    cancel(&mut app, &ctx, first);
    let id = open(&mut app, cmd).unwrap();
    // Preserve Luminosity is on by default; the sample turns it off with the other settings.
    assert_eq!(neutral["preserveLuminosity"], json!(true));
    let mut changed = sample_params("photoFilter");
    changed["preserveLuminosity"] = json!(false);
    set_params(&mut app, id, &changed);
    let sample = params(&mut app, id);
    assert_ne!(sample["color"], neutral["color"], "the sample differs from the neutral settings");
    assert_ne!(sample["density"], neutral["density"]);
    ok(&mut app, &ctx, id).unwrap();

    let again = open(&mut app, cmd).unwrap();
    let reopened = params(&mut app, again);
    assert_eq!(reopened["preserveLuminosity"], json!(false), "Preserve Luminosity off is kept");
    assert_eq!(reopened["color"], neutral["color"], "the colour is the neutral one again");
    assert_eq!(reopened["density"], neutral["density"], "and so is the density");
    // Turned on again and confirmed, it reopens on.
    set_field(&mut app, again, "preserveLuminosity", json!(true));
    ok(&mut app, &ctx, again).unwrap();
    let again = open(&mut app, cmd).unwrap();
    assert_eq!(params(&mut app, again), neutral, "all neutral again");
    cancel(&mut app, &ctx, again);
    // ⌥ reopens at every last setting (#8900).
    let id = open(&mut app, cmd).unwrap();
    set_params(&mut app, id, &changed);
    ok(&mut app, &ctx, id).unwrap();
    let alt = open_last(&mut app, cmd).unwrap();
    assert_eq!(params(&mut app, alt), sample, "⌥ reopens at the last settings");
    cancel(&mut app, &ctx, alt);

    let levels = "image.adjustments.levels";
    let id = open(&mut app, levels).unwrap();
    set_field(&mut app, id, "inBlack", json!(30));
    ok(&mut app, &ctx, id).unwrap();
    let again = open(&mut app, levels).unwrap();
    let defaults =
        photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for("levels", photocraft_doc::ColorMode::Rgb).unwrap());
    assert_eq!(field(&mut app, again, "inBlack"), defaults["inBlack"], "Levels opens neutral without ⌥");
}

/// #9879 review: a recorded Photo Filter that names only its preset completes to that preset's
/// colour, so editing it as a smart filter doesn't swap in the dialog's neutral colour. Recorded
/// values are kept as they were, and nothing else is rewritten.
#[test]
fn recorded_photo_filter_settings_complete_to_their_preset_colour() {
    let app = app_with_image();
    let pf = "image.adjustments.photoFilter";
    let done = complete_params(&app, pf, &json!({"filter": "cooling80", "density": 60, "extra": [1]})).unwrap();
    assert_eq!(done, json!({"filter": "cooling80", "density": 60, "extra": [1], "color": "#006dff"}), "only the colour is added");
    // A recorded colour (here a float triple the engine would rewrite as hex) is left alone.
    assert!(complete_params(&app, pf, &json!({"filter": "cooling80", "color": [0.5, 0.0, 0.0]})).is_none());
    let null = complete_params(&app, pf, &json!({"filter": "cooling80", "color": null})).unwrap();
    assert_eq!(null["color"], json!("#006dff"), "a null colour completes from the preset, as the engine reads it");
    assert!(complete_params(&app, pf, &json!({"density": 60})).is_none(), "no preset to take a colour from");
    assert!(complete_params(&app, pf, &json!({"filter": "nope"})).is_none(), "rejected params");
    // Other adjustments keep their recorded values exactly (a pencil curve isn't smoothed again).
    let curves = json!({"pencil": true, "smooth": 2, "rgb": [0, 255]});
    assert!(complete_params(&app, "image.adjustments.curves", &curves).is_none());
    assert!(complete_params(&app, "filter.blur.gaussian", &json!({"radius": 2})).is_none(), "not an adjustment");
}

/// Retargeting an armed picker uses the current address without losing its adjustment baseline.
#[test]
fn rabbit_picker_uses_current_address_after_retargeting() {
    use photocraft_engine::channel_cmds::ChannelTarget;
    for kind in ["levels", "curves"] {
        for case in 0..4 {
            let mut app = app_with_image();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("edit.fill", json!({"target":"mask","contents":"color","color":"#404040"})).unwrap();
            let current = app.session.active().unwrap().active_layer.unwrap();
            app.run("layer.new.layer", json!({"name":"Receiver"})).unwrap();
            app.run("layer.layerMask.revealAll", json!({})).unwrap();
            app.run("edit.fill", json!({"target":"mask","contents":"color","color":"#bfbfbf"})).unwrap();
            let receiver = app.session.active().unwrap().active_layer.unwrap();
            app.run("channel.new", json!({"name":"Old sample","fill":"black"})).unwrap();
            app.session.select_layer(current).unwrap();
            app.session.active_mut().unwrap().channel_view.target = ChannelTarget::LayerMask;
            let id = open(&mut app, &format!("image.adjustments.{kind}")).unwrap();
            let old = match case {
                0 => json!({"target":"mask","layer":current.0,"document":0}),
                1 => json!({"target":{"channel":0},"layer":receiver.0,"document":0}),
                _ => json!({"target":"mask","layer":receiver.0,"document":0}),
            };
            let address = match case {
                0 | 1 => json!({"target":"mask","layer":receiver.0}),
                2 => json!({"target":"mask"}),
                _ => json!({}),
            };
            retarget(&mut app, id, &old);
            edit_params(&mut app, id, |fields| {
                if kind == "levels" {
                    fields.insert("gamma".into(), json!(1.4));
                } else {
                    fields.insert("points".into(), json!([[0, 10], [128, 180], [255, 240]]));
                }
            });
            arm(&mut app, id, TonePicker::White);
            let cached = params(&mut app, id);
            retarget(&mut app, id, &address);
            let before = app.session.active().unwrap().doc.clone();
            let layer = if case < 2 { receiver } else { current };
            let sample = before.layer(layer).unwrap().mask.as_ref().unwrap().surface.sample_channel(8, 8, 0);
            let expected = photocraft_engine::adjust_params::adjustment_eyedropper(
                kind,
                &cached,
                photocraft_engine::adjust_params::CurvesEyedropper::White,
                [sample; 3],
                photocraft_doc::ColorMode::Grayscale,
            )
            .unwrap();
            assert!(sample_at(&mut app, 8.0, 8.0));
            let shown = params(&mut app, id);
            let actual = photocraft_engine::adjust_params::from_params(kind, &shown, None, photocraft_doc::ColorMode::Grayscale).unwrap();
            assert_eq!(actual, expected, "{kind} address case {case}");
            assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        }
    }
}

/// Levels' own implicit mask and Quick Mask targets reach the live canvas preview and commit.
#[test]
fn rabbit_levels_live_mask_preview_matches_commit() {
    use photocraft_engine::channel_cmds::ChannelTarget;
    // A fixed sample of the mode, depth and Quick Mask combinations: each runs a preview and an OK.
    for (mode, depth, quick) in [("rgb", 8, false), ("lab", 16, true), ("cmyk", 8, true)] {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":32,"height":24,"mode":mode,"depth":depth})).unwrap();
        if quick {
            session.execute("select.editInQuickMaskMode", json!({})).unwrap();
        } else {
            session.execute("layer.layerMask.revealAll", json!({})).unwrap();
            session.active_mut().unwrap().channel_view.target = ChannelTarget::LayerMask;
        }
        let target = if quick { "quickMask" } else { "mask" };
        session.execute("edit.fill", json!({"target":target,"contents":"color","color":"#404040"})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.background_jobs = false;
        let ctx = app.ctx().clone();
        let id = open(&mut app, "image.adjustments.levels").unwrap();
        set_field(&mut app, id, "gamma", json!(2));
        let before = app.session.active().unwrap().doc.clone();
        let idx = app.session.active_index().unwrap();
        crate::canvas::ensure_filter_preview(&mut app, idx, &ctx).expect("the dialog previews on the canvas");
        let preview = app.filter_preview.as_ref().and_then(|p| p.result.clone()).expect("the CPU proxy preview");
        assert!(std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
        ok(&mut app, &ctx, id).unwrap();
        let after = &app.session.active().unwrap().doc;
        let plane = |doc: &photocraft_doc::Document| {
            if quick { doc.quick_mask.as_ref().unwrap().surface.clone() } else { doc.layers[0].mask.as_ref().unwrap().surface.clone() }
        };
        assert_eq!(plane(&preview), plane(after), "{mode} {depth} {target}: live preview matches commit");
        assert!(plane(after).sample_channel(8, 8, 0) > plane(&before).sample_channel(8, 8, 0) + 0.1);
        assert_eq!(after.layers[0].content, before.layers[0].content);
        assert_eq!(after.channels, before.channels);
    }
}

const SELECTIVE: &str = "image.adjustments.selectiveColor";

/// A red left half and a blue right half, so each colour range has pixels to change.
fn selective_app() -> Box<PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#c03020"})).unwrap();
    s.execute("select.rect", json!({"x": 32, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#2030c0"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

/// Recorded parameters the editor cannot read never reach the engine: the dialog refuses to open or OK refuses.
#[test]
fn selective_color_ok_on_malformed_params_reports_and_commits_nothing() {
    let cases = [
        json!({"reds": "oops"}),
        json!({"blues": [1, "x", 0, 0]}),
        json!({"colors": "blues", "cyan": "not a number"}),
        json!({"colors": "purples"}),
        json!({"method": "sideways"}),
    ];
    for case in cases {
        let mut app = selective_app();
        let ctx = app.ctx().clone();
        let doc = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        let journal = app.session.journal.len();
        if let Some(id) = open_with_params(&mut app, SELECTIVE, &case) {
            assert!(ok(&mut app, &ctx, id).is_err(), "{case}: OK reports an error");
        }
        assert_eq!(app.session.active().unwrap().history.past_len(), steps, "{case}: no history step");
        assert_eq!(app.session.journal.len(), journal, "{case}: nothing journaled");
        assert!(std::sync::Arc::ptr_eq(&doc, &app.session.active().unwrap().doc), "{case}: document untouched");
    }
}

/// The legacy single-colour form (`colors` with `cyan`..`black`) folds into the colour's row, and
/// `relative` into the method, as the engine reads them (`relative` overrides `method`).
#[test]
fn selective_color_dialog_folds_the_single_colour_form_into_the_rows() {
    let mut app = selective_app();
    let ctx = app.ctx().clone();
    let id = open_with_params(&mut app, SELECTIVE, &json!({"colors": "blues", "cyan": 30})).unwrap();
    let shown = params(&mut app, id);
    for key in ["cyan", "magenta", "yellow", "black"] {
        assert!(shown.get(key).is_none(), "{key} folded: {shown}");
    }
    assert_eq!(shown["blues"][0].as_f64(), Some(30.0));
    cancel(&mut app, &ctx, id);

    let id = open_with_params(&mut app, SELECTIVE, &json!({"reds": [10.0, 0.0, 0.0, 0.0], "method": "relative", "relative": false})).unwrap();
    let shown = params(&mut app, id);
    assert!(shown.get("relative").is_none(), "folded: {shown}");
    assert_eq!(shown["method"], json!("absolute"));
    assert_eq!(shown["reds"][0].as_f64(), Some(10.0));
}
