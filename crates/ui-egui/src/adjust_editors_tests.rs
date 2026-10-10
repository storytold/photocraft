//! UI tests for the adjustment editors: the whole app (eframe harness, CPU canvas) with an
//! adjustment layer on a new document, edited through the Properties panel with real pointer and
//! key input (issue #12: "add an adjustment layer to a new document, edit it, crash"), and the
//! Curves point interactions (issue #43).

use egui::{Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

fn app_harness(kind: &str, mode: &str, depth: u32) -> Harness<'static, PhotocraftApp> {
    app_harness_stepped(kind, mode, depth, 0.25)
}

/// [`app_harness`] advancing `step_dt` seconds per frame (double-clicks need real frame times).
fn app_harness_stepped(kind: &str, mode: &str, depth: u32, step_dt: f32) -> Harness<'static, PhotocraftApp> {
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
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        // The default Properties group gives way to Layers (#147) and scrolls taller editors;
        // these tests drive every control without scrolling, so they size it like a user would.
        if let Some(s) = app.ui.dock.pane_of("properties").and_then(|i| app.ui.dock.panes.get_mut(i)) {
            s.height = Some(560.0);
        }
        app
    });
    h.run_steps(8);
    h
}

fn layer(h: &Harness<'_, PhotocraftApp>) -> (LayerId, Adjustment) {
    let st = h.state().session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Adjustment(a) => (id, a.clone()),
        other => panic!("{other:?}"),
    }
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, from: Pos2, to: Pos2) {
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

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
    h.run_steps(3);
}

fn label_rect(h: &Harness<'_, PhotocraftApp>, text: &str) -> Option<Rect> {
    // The last match: the Properties header repeats the kind's name above its controls.
    h.query_all_by_label(text).last().map(|n| n.rect())
}

fn curves_graph(h: &Harness<'_, PhotocraftApp>, id: LayerId) -> Rect {
    h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("adjust-layer", id.0)).with("curves-graph"))).expect("curves graph drawn")
}

/// Issue #12: every adjustment kind, on a new document at 8/16/32 bits and in RGB, Grayscale,
/// CMYK and Lab, opens its Properties editor and takes an edit through real input without
/// panicking; slider kinds commit exactly one history step per gesture.
#[test]
fn every_adjustment_layer_edits_through_properties() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("cmyk", 8), ("lab", 16)] {
        for kind in ALL_KINDS {
            let mut h = app_harness(kind, mode, depth);
            let (id, before) = layer(&h);
            let steps = h.state().session.active().unwrap().history.past_len();
            // A probe label per editor; the control sits just below it.
            let probe = match kind {
                "brightnessContrast" => Some("Brightness"),
                "exposure" => Some("Exposure"),
                "vibrance" => Some("Vibrance"),
                "hueSaturation" => Some("Hue"),
                "colorBalance" => Some("Cyan  ·  Red"),
                "blackWhite" => Some("Reds"),
                "photoFilter" => Some("Density"),
                "channelMixer" => Some("Constant"),
                "posterize" => Some("Levels"),
                "threshold" => Some("Threshold Level"),
                "selectiveColor" => Some("Cyan"),
                _ => None,
            };
            match kind {
                "curves" => {
                    let g = curves_graph(&h, id);
                    drag(&mut h, g.center(), g.center() + vec2(0.0, -40.0));
                }
                "levels" => {
                    // The output black handle sits below the output gradient bar.
                    let r = label_rect(&h, "Output Levels:").unwrap_or_else(|| panic!("{kind} {mode}: Output Levels"));
                    let y = r.bottom() + 17.0;
                    drag(&mut h, Pos2::new(r.left() + 6.0, y), Pos2::new(r.left() + 60.0, y));
                }
                "gradientMap" => {
                    let r = label_rect(&h, "Reverse").unwrap_or_else(|| panic!("{kind} {mode}: Reverse"));
                    click(&mut h, r.center());
                }
                _ => {
                    if let Some(p) = probe {
                        let r = label_rect(&h, p).unwrap_or_else(|| panic!("{kind} {mode}: no `{p}` label"));
                        let y = r.bottom() + 14.0;
                        drag(&mut h, Pos2::new(r.left() + 40.0, y), Pos2::new(r.left() + 120.0, y));
                    }
                }
            }
            h.run_steps(4);
            let (_, after) = layer(&h);
            assert!(h.state().live_adjust.is_none(), "{kind} {mode}: preview ended with the gesture");
            if !matches!(kind, "invert" | "colorLookup") {
                assert_ne!(after, before, "{kind} {mode}@{depth}: the edit reached the layer");
                assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "{kind} {mode}: one history step");
            }
        }
    }
}

fn curve_of(a: &Adjustment) -> Vec<[f32; 2]> {
    match a {
        Adjustment::Curves { master, .. } => master.iter().map(|p| [(p.input * 255.0).round(), (p.output * 255.0).round()]).collect(),
        other => panic!("{other:?}"),
    }
}

/// Issue #43: pressing on a point selects it (no new point), Delete/Backspace and ⌘/Ctrl-click
/// remove points, dragging off the graph removes one, endpoints stay.
#[test]
fn curves_points_are_added_selected_and_deleted() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let g = curves_graph(&h, id);
    let at = |v: [f32; 2]| Pos2::new(g.left() + v[0] / 255.0 * g.width(), g.bottom() - v[1] / 255.0 * g.height());
    // Add a point.
    click(&mut h, at([128.0, 128.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    // Press 4 px off it and drag: moves that point, adds none.
    drag(&mut h, at([128.0, 128.0]) + vec2(4.0, 3.0), at([128.0, 180.0]));
    let c = curve_of(&layer(&h).1);
    assert_eq!(c.len(), 3, "{c:?}");
    assert!(c[1][1] > 165.0, "{c:?}");
    // Delete key removes the selected point (the graph has focus).
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // Backspace too.
    click(&mut h, at([64.0, 100.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    h.key_press(egui::Key::Backspace);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // ⌘/Ctrl-click removes.
    click(&mut h, at([190.0, 120.0]));
    assert_eq!(curve_of(&layer(&h).1).len(), 3);
    let p = at([190.0, 120.0]);
    h.event_modifiers(egui::Event::PointerMoved(p), egui::Modifiers::COMMAND);
    h.event_modifiers(
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::COMMAND },
        egui::Modifiers::COMMAND,
    );
    h.run_steps(1);
    h.event_modifiers(
        egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::COMMAND },
        egui::Modifiers::COMMAND,
    );
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    // Dragging a point off the graph removes it; endpoints survive both.
    click(&mut h, at([100.0, 60.0]));
    drag(&mut h, at([100.0, 60.0]), g.right_center() + vec2(80.0, 0.0));
    assert_eq!(curve_of(&layer(&h).1).len(), 2);
    drag(&mut h, at([0.0, 0.0]), g.left_top() + vec2(-80.0, -80.0));
    click(&mut h, at([0.0, 0.0]));
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(curve_of(&layer(&h).1).len(), 2, "endpoints can't be deleted");
}

/// Capture and history are shared with Camera Raw, but Properties commits on release.
#[test]
fn curves_fast_release_commits_one_edit_and_undo_restores_the_curve() {
    let mut h = app_harness("curves", "rgb", 8);
    let (id, _) = layer(&h);
    let graph = curves_graph(&h, id);
    click(&mut h, graph.center());
    let (_, before) = layer(&h);
    let steps = h.state().session.active().unwrap().history.past_len();
    let at = graph.center() + vec2(4.0, 3.0);
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    // Only the release frame supplies the destination, as a very fast physical gesture can.
    h.drop_at(at + vec2(40.0, -32.0));
    h.run_steps(3);
    let after = curve_of(&layer(&h).1);
    assert_eq!(after.len(), 3);
    assert!(after[1][0] > 150.0 && after[1][1] > 150.0, "{after:?}");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
    assert!(h.state().live_adjust.is_none());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(layer(&h).1, before);
}

/// Real pointer input: `n` clicks at `at` with `mods` held (2 = a double-click).
fn clicks_with(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, n: usize, mods: egui::Modifiers) {
    // Well past the double-click time since the last gesture.
    h.run_steps(40);
    h.event(egui::Event::ModifiersChanged(mods));
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..n {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: mods });
            h.run_steps(1);
        }
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn wheel_at(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, notches: f32, mods: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, notches), phase: egui::TouchPhase::Move, modifiers: mods });
    h.run_steps(1);
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn key_with(h: &mut Harness<'_, PhotocraftApp>, key: egui::Key, mods: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(mods));
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods });
    }
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

fn color_balance_of(a: &Adjustment) -> ([f32; 3], [f32; 3], [f32; 3], bool) {
    match a {
        Adjustment::ColorBalance { shadows, midtones, highlights, preserve_luminosity } => (*shadows, *midtones, *highlights, *preserve_luminosity),
        other => panic!("{other:?}"),
    }
}

/// Photoshop 25.4, Color Balance in the Properties panel (measured on the real application by
/// scripted clicks and reading the layer back): a double-click on a slider's knob or track sets that
/// slider of the shown tone to 0, whatever modifier is held; the label doesn't; a click jumps; the
/// wheel does nothing; Up/Down in a field step 1, with Shift 10.
#[test]
fn color_balance_sliders_reset_and_step_like_photoshop() {
    let mut h = app_harness_stepped("colorBalance", "rgb", 8, 1.0 / 60.0);
    let (id, _) = layer(&h);
    let start = json!({"layer": id.0, "shadows": [10, 0, 0], "midtones": [40, -30, 20], "highlights": [0, 0, -15], "preserveLuminosity": false});
    h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
    h.run_steps(4);
    let label = label_rect(&h, "Cyan  ·  Red").expect("Cyan · Red row");
    let on_track = Pos2::new(label.left() + 60.0, label.bottom() + 14.0);
    let cb = |h: &Harness<'_, PhotocraftApp>| color_balance_of(&layer(h).1);

    // A click jumps to the pointer, with or without modifiers.
    clicks_with(&mut h, on_track, 1, egui::Modifiers::NONE);
    let jumped = cb(&h).1[0];
    assert!(jumped < 0.0 && jumped > -100.0, "click jumps to the pointer: {jumped}");
    for mods in [egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::ALT] {
        h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
        h.run_steps(3);
        clicks_with(&mut h, on_track, 1, mods);
        assert_eq!(cb(&h).1[0], jumped, "{mods:?}-click is a plain click");
    }
    // A double-click anywhere on the slider zeroes it, with any modifier; nothing else changes.
    for mods in [egui::Modifiers::NONE, egui::Modifiers::SHIFT, egui::Modifiers::CTRL, egui::Modifiers::ALT] {
        h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
        h.run_steps(3);
        clicks_with(&mut h, on_track, 2, mods);
        assert_eq!(cb(&h), ([10.0, 0.0, 0.0], [0.0, -30.0, 20.0], [0.0, 0.0, -15.0], false), "{mods:?} double-click");
    }
    // The label is not a reset target.
    h.state_mut().run("layer.setAdjustment", start.clone()).unwrap();
    h.run_steps(3);
    clicks_with(&mut h, label.center(), 2, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1, [40.0, -30.0, 20.0], "double-click on the label");
    // The panel's sliders ignore the wheel.
    wheel_at(&mut h, on_track, 3.0, egui::Modifiers::NONE);
    wheel_at(&mut h, on_track, 3.0, egui::Modifiers::SHIFT);
    assert_eq!(cb(&h).1, [40.0, -30.0, 20.0], "wheel over a Properties slider");
    // Up/Down in the field: 1, Shift: 10 (also below zero, where a half step used to round away).
    let field = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .map(|n| n.rect())
        .find(|r| (r.center().y - label.center().y).abs() < 8.0)
        .expect("Cyan · Red field");
    clicks_with(&mut h, field.center(), 1, egui::Modifiers::NONE);
    key_with(&mut h, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1[0], 41.0, "Up");
    key_with(&mut h, egui::Key::ArrowUp, egui::Modifiers::SHIFT);
    assert_eq!(cb(&h).1[0], 51.0, "Shift+Up");
    for _ in 0..8 {
        key_with(&mut h, egui::Key::ArrowDown, egui::Modifiers::SHIFT);
    }
    key_with(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1[0], -30.0, "Shift+Down ×8, Down");
    key_with(&mut h, egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1[0], -29.0, "Up below zero");
}

/// Photoshop 25.4, Color Balance in the Properties panel: edits that follow each other make one
/// history step (a double-click, whose first click moved the knob, undoes in one go); Reset first
/// returns to the settings the layer had when the panel started showing it (Tone choice kept),
/// then to the defaults (every tone 0, Midtones, Preserve Luminosity kept), within that step.
#[test]
fn color_balance_properties_history_and_reset_like_photoshop() {
    let mut h = app_harness_stepped("colorBalance", "rgb", 8, 1.0 / 60.0);
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
    let cb = |h: &Harness<'_, PhotocraftApp>| color_balance_of(&layer(h).1);
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    let label = label_rect(&h, "Cyan  ·  Red").expect("Cyan · Red row");
    let on_track = Pos2::new(label.left() + 60.0, label.bottom() + 14.0);
    let before = steps(&h);
    clicks_with(&mut h, on_track, 1, egui::Modifiers::NONE);
    clicks_with(&mut h, Pos2::new(label.left() + 90.0, on_track.y), 2, egui::Modifiers::NONE);
    assert_eq!(cb(&h).1, [0.0, -30.0, 20.0]);
    assert_eq!(steps(&h), before + 1, "a click and a double-click: one history step");
    assert!(h.state_mut().session.undo());
    h.run_steps(3);
    assert_eq!(cb(&h), start, "one Undo returns to the settings before them");
    // Edit with Highlights shown; Reset returns to the settings the panel started with.
    let hl = label_rect(&h, "Highlights").expect("Highlights radio");
    clicks_with(&mut h, hl.center(), 1, egui::Modifiers::NONE);
    clicks_with(&mut h, on_track, 1, egui::Modifiers::NONE);
    assert_ne!(cb(&h), start);
    let tone = crate::adjust_editors::layer_mem(id).with("cb-tone");
    let reset = h.get_by_label("Reset to defaults").rect();
    let before = steps(&h);
    clicks_with(&mut h, reset.center(), 1, egui::Modifiers::NONE);
    assert_eq!(cb(&h), start, "first Reset: the settings the panel started with");
    assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(tone)), Some(2), "Tone choice kept");
    clicks_with(&mut h, reset.center(), 1, egui::Modifiers::NONE);
    assert_eq!(cb(&h), ([0.0; 3], [0.0; 3], [0.0; 3], false), "second Reset: defaults, Preserve Luminosity kept");
    assert_eq!(h.ctx.data(|d| d.get_temp::<usize>(tone)), Some(1), "defaults show Midtones");
    clicks_with(&mut h, reset.center(), 1, egui::Modifiers::NONE);
    assert_eq!(cb(&h), ([0.0; 3], [0.0; 3], [0.0; 3], false), "third Reset: still the defaults");
    assert_eq!(steps(&h), before, "the edit and the Resets share one history step");
}

fn map_stops(a: &Adjustment) -> Vec<(f32, [u8; 3])> {
    let q = |v: f32| (v * 255.0).round() as u8;
    match a {
        Adjustment::GradientMap { stops, .. } => stops.iter().map(|(t, c)| (*t, [q(c[0]), q(c[1]), q(c[2])])).collect(),
        other => panic!("{other:?}"),
    }
}

/// Issue #1588: the Gradient Map preset picker offers the session's gradient library (built-in
/// groups beyond Basics and the user's own saved gradients), and picking one writes its stops as
/// one history step.
#[test]
fn gradient_map_picker_offers_the_gradient_library() {
    use photocraft_engine::presets::{Group, gradients};
    let mut h = app_harness("gradientMap", "rgb", 8);
    // A short library so every row fits the popup: Basics, Blues and a user-saved group.
    let mut lib: Vec<_> = gradients::builtin().into_iter().take(2).collect();
    lib.push(Group::new("Mine", vec![gradients::GradientPreset::new("My Teal", &["#008080", "#ffee00"])]));
    h.state_mut().session.presets.gradients = lib;
    h.run_steps(2);
    let steps = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().history.past_len();
    for (name, want) in
        [("Blue 03", vec![(0.0, [0x00, 0xb4, 0xd8]), (1.0, [0x03, 0x04, 0x5e])]), ("My Teal", vec![(0.0, [0x00, 0x80, 0x80]), (1.0, [0xff, 0xee, 0x00])])]
    {
        let before = steps(&h);
        // The picker sits right of its "Preset:" label.
        let preset = label_rect(&h, "Preset:").expect("the preset picker");
        click(&mut h, Pos2::new(preset.right() + 60.0, preset.center().y));
        let item = h.query_by_label(name).unwrap_or_else(|| panic!("`{name}` in the picker")).rect();
        click(&mut h, item.center());
        h.run_steps(4);
        assert_eq!(map_stops(&layer(&h).1), want, "{name}");
        assert_eq!(steps(&h), before + 1, "{name}: one history step");
    }
}
