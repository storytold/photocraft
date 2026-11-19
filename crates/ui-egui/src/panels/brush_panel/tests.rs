use crate::app::framework::BuildApp;
use craft::behavior::{EditEvent, GestureId, Mixed};
use craft::keymap::Mods;
use craft::ui::contract::ReorderMove;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::presets::{PresetEvent, PresetRow, PresetView};
use craft::ui::runtime::commands::{CommandRef, CommandState};
use craft::ui::runtime::env::Env;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::paint::{Control, GrayTile, MaskMode, Pattern, PatternStyle, TipShape};

use super::brushes::{BrushKey, apply_events, parts_of, preset_rows, run_picks};
use super::*;

fn app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn last_journal(app: &PhotocraftApp) -> Option<(String, Value)> {
    app.session.journal.last().cloned()
}

/// The visible options caption, not the numeric field it names.
fn caption_rect(h: &Harness<'_, Box<PhotocraftApp>>, text: &str) -> egui::Rect {
    h.query_all(egui_kittest::kittest::by().label(text).include_labels())
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        .unwrap_or_else(|| panic!("the `{text}` caption"))
        .rect()
}

#[test]
fn sections_with_boxes_have_flags() {
    let mut b = BrushSettings::default();
    for (i, (_, has_box)) in SECTIONS.iter().enumerate() {
        assert_eq!(section_flag(&mut b, i).is_some(), *has_box, "{i}");
    }
    *section_flag(&mut b, 3).unwrap() = true;
    assert!(b.texture.enabled);
}

/// What each section's controls edit, set to non-default values.
fn edit_section(i: usize, b: &mut BrushSettings) {
    let dynamic = |jitter: f32, control: Control, minimum: f32| paint::Dynamic { jitter, control, fade_steps: 40, minimum, ..Default::default() };
    match i {
        0 => {
            b.size = 87.0;
            b.flip_x = true;
            b.flip_y = true;
            b.angle = -35.0;
            b.roundness = 0.4;
            b.hardness = 0.25;
            b.spacing = 0.6;
            b.spacing_enabled = false;
        }
        1 => {
            let sd = &mut b.shape_dynamics;
            sd.enabled = true;
            sd.size = dynamic(0.3, Control::Fade, 0.2);
            sd.angle = dynamic(0.1, Control::Direction, 0.0);
            sd.roundness = dynamic(0.5, Control::PenTilt, 0.25);
            sd.flip_x_jitter = true;
            sd.flip_y_jitter = true;
            sd.tilt_scale = 1.25;
            sd.brush_projection = true;
        }
        2 => {
            let sc = &mut b.scattering;
            sc.enabled = true;
            sc.scatter = dynamic(2.5, Control::PenPressure, 0.0);
            sc.both_axes = true;
            sc.count = 4;
            sc.count_jitter = dynamic(0.4, Control::StylusWheel, 0.0);
        }
        3 => {
            let tx = &mut b.texture;
            tx.enabled = true;
            tx.pattern = Pattern::Procedural { style: PatternStyle::Canvas, size: 200, seed: 1 };
            tx.invert = true;
            tx.scale = 1.5;
            tx.brightness = -0.2;
            tx.contrast = 0.6;
            tx.each_tip = true;
            tx.mode = MaskMode::HardMix;
            tx.depth = 0.7;
            tx.depth_jitter = dynamic(0.3, Control::Rotation, 0.1);
        }
        4 => {
            let d = &mut b.dual_brush;
            d.enabled = true;
            d.mode = MaskMode::ColorBurn;
            d.flip = true;
            d.tip = TipShape::Sampled(GrayTile::from_fn(8, 6, |x, y| ((x + y) % 2) as f32));
            d.size = 33.0;
            d.hardness = 0.5;
            d.spacing = 0.8;
            d.scatter = 1.2;
            d.both_axes = true;
            d.count = 3;
        }
        5 => {
            let c = &mut b.color_dynamics;
            c.enabled = true;
            c.per_tip = false;
            c.fg_bg = dynamic(0.6, Control::Fade, 0.0);
            c.hue_jitter = 0.1;
            c.saturation_jitter = 0.2;
            c.brightness_jitter = 0.3;
            c.purity = -0.4;
        }
        6 => {
            b.transfer.enabled = true;
            b.transfer.opacity = dynamic(0.5, Control::PenPressure, 0.1);
            b.transfer.flow = dynamic(0.25, Control::Fade, 0.3);
            b.transfer.wetness = dynamic(0.35, Control::PenPressure, 0.1);
            b.transfer.mix = dynamic(0.45, Control::Fade, 0.2);
            b.mixer = paint::MixerSettings {
                wet: 0.9,
                load: 0.15,
                mix: 0.7,
                flow: 0.8,
                sample_all_layers: true,
                load_after_stroke: false,
                clean_after_stroke: false,
                solid_colors_only: true,
            };
        }
        7 => {
            let p = &mut b.pose;
            p.enabled = true;
            p.tilt_x = 45.0;
            p.tilt_y = -27.0;
            p.override_tilt_x = true;
            p.override_tilt_y = true;
            p.rotation = 120.0;
            p.override_rotation = true;
            p.pressure = 0.4;
            p.override_pressure = true;
        }
        8 => b.noise = true,
        9 => b.wet_edges = true,
        10 => {
            b.build_up = true;
            b.build_up_rate = 55.0;
        }
        11 => {
            let s = &mut b.smoothing;
            s.amount = 0.45;
            s.pulled_string = true;
            s.catch_up = false;
            s.catch_up_on_end = false;
            s.adjust_for_zoom = false;
        }
        _ => b.protect_texture = true,
    }
}

#[test]
fn every_section_round_trips_through_set_brush() {
    for (i, (name, _)) in SECTIONS.iter().enumerate() {
        let mut app = app();
        let before = app.session.tools.brush.clone();
        let mut after = before.clone();
        edit_section(i, &mut after);
        assert_ne!(before, after, "section {i} edits nothing");
        commit(&mut app, &before, &after);
        assert_eq!(app.session.tools.brush, after, "section {i} ({name})");
        // Through the command, with only the changed fields.
        let (id, p) = last_journal(&app).expect("journaled");
        assert_eq!(id, "tools.setBrush");
        let patch = &p["brush"];
        assert!(patch.as_object().is_some_and(|o| !o.is_empty() && o.len() <= 8), "section {i}: {patch}");
        // And the engine's view (`brush.get`) reads back the same settings.
        let got = app.session.execute("brush.get", json!({})).unwrap();
        assert_eq!(serde_json::from_value::<BrushSettings>(got).unwrap(), after, "section {i}");
        // Replaying the journaled call onto a fresh session gives the same brush (drivable).
        let mut s = photocraft_engine::Session::new();
        s.execute(&id, p).unwrap();
        assert_eq!(s.tools.brush, after, "replay of section {i}");
    }
}

#[test]
fn patches_are_minimal_and_skip_unchanged_bitmaps() {
    let tip = TipShape::Sampled(GrayTile::from_fn(400, 300, |x, y| ((x * y) % 7) as f32 / 7.0));
    let old = BrushSettings {
        tip: tip.clone(),
        texture: paint::Texture { pattern: Pattern::Tile(GrayTile::from_fn(64, 64, |x, _| x as f32 / 64.0)), ..Default::default() },
        ..Default::default()
    };
    let new = BrushSettings { size: 44.0, ..old.clone() };
    assert_eq!(brush_patch(&old, &new), json!({"size": 44.0}));
    let new = BrushSettings { shape_dynamics: paint::ShapeDynamics { enabled: true, ..Default::default() }, ..old.clone() };
    assert_eq!(brush_patch(&old, &new), json!({"shapeDynamics": {"enabled": true}}));
    assert_eq!(brush_patch(&old, &old), json!({}));
    // A changed tip is sent whole.
    let new = BrushSettings { tip: TipShape::Round, ..old.clone() };
    assert_eq!(brush_patch(&old, &new), json!({"tip": "round"}));
    // Applying any of these through the engine gives back `new`.
    let mut app = app();
    app.session.tools.brush = old.clone();
    let new = BrushSettings { size: 9.0, roundness: 0.5, ..old.clone() };
    commit(&mut app, &old, &new);
    assert_eq!(app.session.tools.brush, new);
    // No change, no command.
    let n = app.session.journal.len();
    commit(&mut app, &new, &new);
    assert_eq!(app.session.journal.len(), n);
}

#[test]
fn bad_patches_fail_without_touching_the_brush() {
    let mut app = app();
    let before = app.session.tools.brush.clone();
    for bad in [
        json!({"brush": {"size": "huge"}}),
        json!({"brush": {"shapeDynamics": {"size": {"control": "telepathy"}}}}),
        json!({"brush": {"tip": {"sampled": {"width": 0}}}}),
    ] {
        assert!(app.run("tools.setBrush", bad).is_err());
        assert_eq!(app.session.tools.brush, before);
    }
}

#[test]
fn section_locks_round_trip_and_survive_preset_picks() {
    let mut b = BrushSettings::default();
    for (i, (name, _)) in SECTIONS.iter().enumerate() {
        assert_eq!(section_lock(&mut b, i).is_some(), i > 0, "{name} has a lock");
    }
    let mut app = app();
    let before = app.session.tools.brush.clone();
    let mut after = before.clone();
    edit_section(2, &mut after);
    for i in [2, 6, 8] {
        *section_lock(&mut after, i).unwrap() = true;
    }
    commit(&mut app, &before, &after);
    let (_, p) = last_journal(&app).unwrap();
    assert_eq!(p["brush"]["locks"], json!({"scattering": true, "transfer": true, "noise": true}));
    // Picking a preset keeps the locked sections (and the locks), and the preset still reads as current.
    let chalk = paint::presets::find(&app.session.tools.presets, "Chalk").unwrap().brush.clone();
    app.run("tools.setBrush", json!({"preset": "Chalk"})).unwrap();
    let b = &app.session.tools.brush;
    assert_eq!(b.scattering, after.scattering);
    assert!(b.locks.scattering && b.locks.noise);
    assert_eq!(b.tip, chalk.tip);
}

/// A harness that shows the options bar with `tool` in the Studio (non-pro) theme.
fn options_bar_harness(tool: crate::state::Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = app();
    app.ui.tool = tool;
    // As tall as a window, so the bar's menus open in full below it.
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(2400.0, 600.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

#[test]
fn options_bar_edits_are_one_set_brush_per_gesture() {
    let mut h = options_bar_harness(crate::state::Tool::Brush);
    let n = h.state().session.journal.len();
    h.run_steps(3);
    assert_eq!(h.state().session.journal.len(), n, "showing the bar issues nothing");
    // Drag the Opacity field left: many frames, one journal entry.
    let label = caption_rect(&h, "Opacity");
    let start = egui::pos2(label.right() + 40.0, label.center().y);
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.run();
    for k in 1..=8 {
        h.hover_at(start - vec2(5.0 * k as f32, 0.0));
        h.run();
    }
    h.drop_at(start - vec2(40.0, 0.0));
    h.run_steps(2);
    let opacity = h.state().session.tools.brush.opacity;
    assert!(opacity < 0.95, "the drag lowered the opacity: {opacity}");
    let j = &h.state().session.journal;
    assert_eq!(j.len(), n + 1, "{:?}", &j[n..]);
    let (id, p) = j.last().unwrap();
    assert_eq!(id, "tools.setBrush");
    assert_eq!(p["brush"]["opacity"].as_f64().map(|v| v as f32), Some(opacity));
    // A click is its own gesture: the Pressure controls size switch sits left of its label.
    let label = h.get_by_label("Pressure controls size").rect();
    let was = h.state().session.tools.brush.pressure_size;
    let at = egui::pos2(label.left() + 6.0, label.center().y);
    h.hover_at(at);
    h.run();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run_steps(2);
    assert_eq!(h.state().session.tools.brush.pressure_size, !was);
    assert_eq!(h.state().session.journal.len(), n + 2);
    assert_eq!(h.state().session.journal.last().unwrap().1["brush"], json!({"pressureSize": !was}));
    // Replaying the journal gives the same brush.
    let mut s = photocraft_engine::Session::new();
    for (id, p) in &h.state().session.journal {
        s.execute(id, p.clone()).unwrap();
    }
    assert_eq!(s.tools.brush, h.state().session.tools.brush);
}

#[test]
fn brush_and_eraser_size_fields_keep_sizes_above_2500_px() {
    // #5708: outside the Pro themes the Size field capped at 2500 px and clamped a bigger brush
    // (set from the picker or `]`) on the next frame; Photoshop and the engine allow 5000 px.
    for tool in [crate::state::Tool::Brush, crate::state::Tool::Eraser] {
        let mut h = options_bar_harness(tool);
        let n = h.state().session.journal.len();
        h.state_mut().session.tools.brush.size = 4000.0;
        h.run_steps(4);
        assert_eq!(h.state().session.tools.brush.size, 4000.0, "{tool:?}");
        let j = &h.state().session.journal;
        assert_eq!(j.len(), n, "showing the bar issues nothing: {:?}", &j[n..]);
    }
}

/// Runs the menu row `label` from the keyboard: focus it, then Enter.
#[test]
fn mixer_brush_clean_brush_and_the_automatic_load_keep_the_foreground_out() {
    let mut h = options_bar_harness(crate::state::Tool::MixerBrush);
    // A red image, an empty layer above it, a blue foreground, and a brush that samples all layers.
    for (id, p) in [
        ("file.new", json!({"width": 200, "height": 40, "background": "transparent"})),
        ("edit.fill", json!({"color": "#ff0000"})),
        ("layer.new.layer", json!({})),
        ("tools.setColors", json!({"foreground": "#0000ff"})),
        ("tools.setBrush", json!({"brush": {"size": 8, "pressureSize": false, "mixer": {"sampleAllLayers": true}}})),
    ] {
        h.state_mut().run(id, p).unwrap();
    }
    h.run_steps(3);
    for label in ["Current brush load", "Load the brush after each stroke", "Clean the brush after each stroke"] {
        assert!(h.get_by_label(label).rect().is_positive(), "missing Mixer Brush option {label}");
    }

    // Turn the automatic Load off: one tools.setBrush call.
    h.get_by_label("Load the brush after each stroke").click();
    h.run_steps(3);
    assert!(!h.state().session.tools.brush.mixer.load_after_stroke);
    let (id, params) = last_journal(h.state()).unwrap();
    assert_eq!((id.as_str(), &params["brush"]["mixer"]["loadAfterStroke"]), ("tools.setBrush", &json!(false)));

    // Current Brush Load › Clean Brush empties the brush.
    h.get_by_label("Current brush load").click();
    h.run_steps(3);
    h.get_by_label("Clean Brush").click();
    h.run_steps(3);
    assert_eq!(last_journal(h.state()).map(|(id, _)| id), Some("brush.mixer.clean".to_string()));
    assert_eq!(h.state().session.tools.mixer.reservoir, None);

    // The stroke the canvas commits on release lays down only the red it picks up.
    crate::retouch_ui::finish_stroke(h.state_mut(), crate::state::Tool::MixerBrush, &[[5.0, 20.0, 1.0], [195.0, 20.0, 1.0]], Mods::NONE, false);
    let d = h.state().session.active().unwrap();
    let c = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(100, 20);
    assert!(c[3] > 0.1 && c[0] > 0.99 && c[2] < 0.01, "the stroke carries no foreground: {c:?}");
    // With the automatic Load off, the stroke did not refill the brush either.
    assert_eq!(h.state().session.tools.mixer.reservoir, None);

    // Load Brush fills it with the foreground again.
    h.get_by_label("Current brush load").click();
    h.run_steps(3);
    h.get_by_label("Load Brush").click();
    h.run_steps(3);
    assert_eq!(h.state().session.tools.mixer.reservoir, Some([0.0, 0.0, 1.0, 1.0]));
}

/// A harness that draws the registry body of `panel` (`"brushSettings"` or `"brushes"`) with
/// `section` selected.
fn harness(panel: &'static str, section: usize) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = app();
    app.ui.brush_section = section;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1200.0, 900.0)).build_app(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let entry = crate::panels::registry::entry(panel).expect("a registered panel");
            (entry.body)(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

#[test]
fn checkbox_only_rows_cannot_restore_an_empty_properties_pane() {
    for (section, &(name, _)) in SECTIONS.iter().enumerate().skip(8) {
        let h = harness("brushSettings", section);
        assert_eq!(h.state().ui.brush_section, 0, "an old selection of {name} returns to Brush Tip Shape");
    }
}

#[test]
fn editing_the_options_bar_amount_reenables_smoothing() {
    let mut h = options_bar_harness(crate::state::Tool::Brush);
    h.state_mut().run("tools.setBrush", json!({"brush": {"smoothing": {"enabled": false, "amount": 0.4}}})).unwrap();
    h.run_steps(3);
    let n = h.state().session.journal.len();
    let label = caption_rect(&h, "Smoothing");
    let at = egui::pos2(label.right() + 25.0, label.center().y);
    h.hover_at(at);
    h.run_steps(2);
    h.drag_at(at);
    h.run_steps(2);
    h.hover_at(at + vec2(20.0, 0.0));
    h.run_steps(2);
    h.drop_at(at + vec2(20.0, 0.0));
    h.run_steps(3);
    let smoothing = &h.state().session.tools.brush.smoothing;
    assert!(smoothing.enabled && smoothing.amount != 0.4);
    assert_eq!(h.state().session.journal.len(), n + 1, "one options-bar gesture");
    assert_eq!(last_journal(h.state()).unwrap().1["brush"]["smoothing"]["enabled"], true);
}

#[test]
fn every_section_draws_without_issuing_commands() {
    // Brush Settings draws the selected section's controls, but only up to `CHECKBOX_ONLY_START`: a later section is
    // reset to 0 before drawing (`brush_settings.rs`, tested by `checkbox_only_rows_cannot_restore_an_empty_properties_pane`).
    // The Brushes panel does not read the section at all, so one run per view covers it (#15652).
    for (panel, view, sections) in
        [("brushSettings", None, 0..CHECKBOX_ONLY_START), ("brushes", Some(PresetView::List), 0..1), ("brushes", Some(PresetView::Grid), 0..1)]
    {
        for section in sections {
            let mut h = harness(panel, section);
            if let Some(view) = view {
                h.state_mut().ui.brushes_panel.view = view;
            }
            // A brush with every section on and a sampled tip.
            let mut b = BrushSettings::default();
            for i in 0..SECTIONS.len() {
                edit_section(i, &mut b);
            }
            b.tip = TipShape::Sampled(GrayTile::from_fn(50, 40, |x, y| ((x ^ y) & 1) as f32));
            h.state_mut().session.tools.brush = b.clone();
            let n = h.state().session.journal.len();
            h.run_steps(4);
            // Merely showing the panel must not rewrite the brush (no rounding drift, no commands).
            assert_eq!(h.state().session.journal.len(), n, "{panel} {view:?} section {section}");
            assert_eq!(h.state().session.tools.brush, b, "{panel} {view:?} section {section}");
        }
    }
}

fn kit(name: &str, group: &str, folder: &[&str]) -> paint::BrushPreset {
    paint::BrushPreset {
        name: name.into(),
        brush: Default::default(),
        builtin: false,
        group: group.into(),
        folder: folder.iter().map(|s| s.to_string()).collect(),
        captures: Default::default(),
    }
}

/// The rows `preset_rows` gives for `app`, drawn in a frame (the rows read the frame's environment).
fn rows_of(app: Box<PhotocraftApp>, view: PresetView) -> Vec<PresetRow<BrushKey>> {
    let mut h = Harness::builder().with_size(vec2(200.0, 200.0)).with_context(app.ctx().clone()).build_ui_state(
        move |ui, st: &mut (Box<PhotocraftApp>, Vec<PresetRow<BrushKey>>)| {
            st.1 = preset_rows(&st.0, &Env::of(ui), parts_of(view));
        },
        (app, Vec::new()),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(2);
    std::mem::take(&mut h.state_mut().1)
}

#[test]
fn preset_rows_nest_presets_in_panel_order() {
    let max = photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;
    let deep: Vec<&str> = vec!["d"; max + 5];
    let mut app = app();
    app.session.tools.presets = vec![
        kit("Top", "Kit", &[]),
        kit("Pen", "Kit", &["Inks"]),
        kit("Chalk", "Kit", &["Dry"]),
        kit("Nib", "Kit", &["Inks", "Fine"]),
        kit("Quill", "Kit", &["Inks"]),
        kit("Deep", "Kit", &deep),
    ];
    let out: Vec<String> = rows_of(app, PresetView::List)
        .iter()
        .map(|r| match r.count {
            Some(n) => format!("{}:{}#{n}", r.depth, r.label),
            None => format!("{}:{}", r.depth, r.label),
        })
        .collect();
    assert_eq!(out[..10], ["0:Kit#6", "1:Top", "1:Inks#3", "2:Pen", "2:Fine#1", "3:Nib", "2:Quill", "1:Dry#1", "2:Chalk", "1:d#1"]);
    assert_eq!(out.iter().filter(|s| s.ends_with(":d#1")).count(), max, "paths are cut at the folder depth");
    assert_eq!(out.last().map(String::as_str), Some(format!("{}:Deep", max + 1).as_str()));
}

fn fixture() -> Box<PhotocraftApp> {
    let mut app = app();
    app.session.tools.presets = vec![
        kit("A1", "Alpha", &[]),
        kit("A2", "Alpha", &[]),
        kit("A3", "Alpha", &[]),
        kit("B1", "Beta", &[]),
        kit("Kit Top", "Kit", &[]),
        kit("Kit Pen", "Kit", &["Inks"]),
        kit("Kit Nib", "Kit", &["Inks", "Fine"]),
        kit("Kit Chalk", "Kit", &["Dry"]),
    ];
    app
}

fn folder(group: &str, path: &[&str]) -> BrushKey {
    BrushKey::Folder { group: group.into(), folder: path.iter().map(|s| s.to_string()).collect() }
}

fn preset(name: &str) -> BrushKey {
    BrushKey::Preset(name.into())
}

/// A finished reorder gesture of `moved` to `to`, landing in `parent`.
fn reorder(moved: BrushKey, to: usize, parent: Option<BrushKey>) -> PresetEvent<BrushKey> {
    let mv = ReorderMove::new(moved.clone(), vec![moved], to, false).in_group(parent);
    PresetEvent::Reorder(EditEvent::commit(GestureId::new(1), Mixed::Same(mv.clone()), mv))
}

#[test]
fn brushes_panel_events_run_their_commands() {
    let keys: Vec<BrushKey> = rows_of(fixture(), PresetView::List).into_iter().map(|r| r.key).collect();
    assert_eq!(keys.len(), 14, "{keys:?}");
    let table: Vec<(&str, PresetEvent<BrushKey>, (&str, Value))> = vec![
        ("select", PresetEvent::select(preset("A2"), Mods::default()), ("tools.setBrush", json!({"preset": "A2"}))),
        ("rename a preset", PresetEvent::rename(preset("A3"), "Third"), ("brush.presets.rename", json!({"name": "A3", "newName": "Third"}))),
        (
            "rename a nested folder",
            PresetEvent::rename(folder("Kit", &["Inks"]), " Pens "),
            ("brush.presets.renameGroup", json!({"group": "Kit", "folder": ["Inks"], "newName": "Pens"})),
        ),
        ("delete a preset", PresetEvent::Delete(preset("B1")), ("brush.presets.delete", json!({"name": "B1"}))),
        ("delete a folder", PresetEvent::Delete(folder("Kit", &["Dry"])), ("brush.presets.deleteGroup", json!({"group": "Kit", "folder": ["Dry"]}))),
        (
            "move to a group",
            PresetEvent::move_to(preset("A2"), folder("Beta", &[])),
            ("brush.presets.move", json!({"name": "A2", "group": "Beta", "folder": []})),
        ),
        (
            "reorder a preset into a folder",
            reorder(preset("A1"), 9, Some(folder("Kit", &["Inks"]))),
            ("brush.presets.move", json!({"name": "A1", "group": "Kit", "folder": ["Inks"], "index": 1})),
        ),
        ("reorder a group before another", reorder(folder("Kit", &[]), 0, None), ("brush.presets.moveGroup", json!({"group": "Kit", "before": "Alpha"}))),
        (
            "reorder a folder into a folder",
            reorder(folder("Kit", &["Dry"]), 99, Some(folder("Kit", &["Inks", "Fine"]))),
            ("brush.presets.moveGroup", json!({"group": "Kit", "folder": ["Dry"], "into": {"group": "Kit", "folder": ["Inks", "Fine"]}})),
        ),
        (
            "reorder a folder out to the top level",
            reorder(folder("Kit", &["Inks", "Fine"]), 6, None),
            ("brush.presets.moveGroup", json!({"group": "Kit", "folder": ["Inks", "Fine"], "into": {}, "before": "Kit"})),
        ),
    ];
    for (what, event, (id, params)) in table {
        let mut app = fixture();
        apply_events(&mut app, vec![event], &keys);
        assert_eq!(last_journal(&app), Some((id.to_string(), params)), "{what}");
    }
    // Nothing runs for the current preset's activation, an unchanged name or a drag still running.
    let mut app = fixture();
    app.run("tools.setBrush", json!({"preset": "A2"})).unwrap();
    let n = app.session.journal.len();
    let mv = ReorderMove::new(preset("A1"), vec![preset("A1")], 9, false).in_group(Some(folder("Kit", &["Inks"])));
    apply_events(
        &mut app,
        vec![
            PresetEvent::activate(preset("A2"), Mods::default()),
            PresetEvent::rename(preset("A3"), "A3"),
            PresetEvent::Reorder(EditEvent::update(GestureId::new(2), mv)),
        ],
        &keys,
    );
    assert_eq!(app.session.journal.len(), n);
    // The panel's picks go through the same mapping.
    run_picks(&mut app, vec![craft::draw::ui::brush_presets::BrushPick::Select(preset("A1"))], &keys);
    assert_eq!(last_journal(&app), Some(("tools.setBrush".to_string(), json!({"preset": "A1"}))));
}

/// A fresh session with the Brush tool: shortcuts, the options bar and the Brush Settings body,
/// in the default theme (#258).
fn app_harness() -> Harness<'static, Box<PhotocraftApp>> {
    app_harness_with_language(crate::i18n::Lang::EN)
}

fn app_harness_with_language(lang: crate::i18n::Lang) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = app();
    app.ui.tool = crate::state::Tool::Brush;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1400.0, 900.0)).build_app(
        move |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let previous = crate::i18n::current();
            crate::i18n::set_current(lang);
            crate::shortcuts::handle(app, &ctx);
            crate::panels::options_bar(app, ui);
            if crate::panels::panel_docking::is_shown(app, "brushSettings") {
                super::super::brush_settings::brush_settings(app, ui);
            }
            crate::brush_picker::show_at(ui, app);
            crate::i18n::set_current(previous);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
    h.run_steps(4);
    h
}

fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_text(s, out)),
        _ => {}
    }
}

#[test]
fn brush_sections_paint_simplified_chinese_labels() {
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let zh = crate::i18n::Lang::from_code("zh-hans").unwrap();
    let mut h = app_harness_with_language(zh);
    h.key_press(egui::Key::F5);
    h.run_steps(3);
    let mut painted = Vec::new();
    for shape in &h.output().shapes {
        collect_text(&shape.shape, &mut painted);
    }
    for (name, _) in SECTIONS {
        let translated = crate::i18n::tr(zh, name);
        assert!(painted.iter().any(|s| s == translated), "{name} must paint as {translated}");
    }
}

/// #3477: the options pane has no title of its own. The selected row in the section list is the
/// only place the section's name is painted.
#[test]
fn options_pane_does_not_repeat_the_selected_sections_name() {
    for (section, (name, _)) in SECTIONS.iter().enumerate().take(CHECKBOX_ONLY_START) {
        let h = harness("brushSettings", section);
        let mut painted = Vec::new();
        for shape in &h.output().shapes {
            collect_text(&shape.shape, &mut painted);
        }
        assert_eq!(painted.iter().filter(|s| s.as_str() == *name).count(), 1, "{name} is painted once, in the section list");
    }
}

#[test]
fn f5_edits_the_default_brush_with_every_section() {
    let mut h = app_harness();
    assert!(!crate::panels::panel_docking::is_shown(h.state(), "brushSettings"));
    let fresh = h.state().session.tools.brush.clone();
    assert!(!fresh.shape_dynamics.enabled && !fresh.transfer.enabled, "the default round brush has no dynamics yet");
    h.key_press(egui::Key::F5);
    h.run_steps(3);
    assert!(crate::panels::panel_docking::is_shown(h.state(), "brushSettings"), "F5 shows Brush Settings");
    // Every section is there for the built-in round brush, not only for sampled or imported ones.
    for (name, _) in SECTIONS {
        assert!(h.query_all_by_label(name).next().is_some(), "{name} listed");
    }
    // Clicking a section's name shows it and turns it on: one tools.setBrush.
    let n = h.state().session.journal.len();
    h.get_all_by_label("Shape Dynamics").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().ui.brush_section, 1);
    assert!(h.state().session.tools.brush.shape_dynamics.enabled);
    assert_eq!(h.state().session.journal.len(), n + 1);
    let (id, p) = last_journal(h.state()).unwrap();
    assert_eq!(id, "tools.setBrush");
    assert_eq!(p["brush"]["shapeDynamics"]["enabled"], json!(true));
    // A section's box toggles it without showing it.
    h.get_all_by_label("Enable Transfer").next().unwrap().click();
    h.run_steps(3);
    assert!(h.state().session.tools.brush.transfer.enabled);
    assert_eq!(h.state().ui.brush_section, 1);
    assert_eq!(h.state().session.journal.len(), n + 2);
    // The rest of the brush is untouched.
    let b = &h.state().session.tools.brush;
    assert_eq!((b.size, b.hardness, &b.tip), (fresh.size, fresh.hardness, &fresh.tip));
    // F5 again hides the panel.
    h.key_press(egui::Key::F5);
    h.run_steps(2);
    assert!(!crate::panels::panel_docking::is_shown(h.state(), "brushSettings"));
}

#[test]
fn update_current_brush_button_overwrites_the_picked_preset_in_place() {
    let mut h = harness("brushSettings", 0);
    // Nothing picked yet: the button is disabled and clicking it does nothing.
    assert!(h.state().session.tools.current_preset.is_none());
    h.get_by_label("Update the current brush with these settings").click();
    h.run_steps(3);
    assert!(!h.state().session.journal.iter().any(|(id, _)| id == "brush.presets.update"));
    // Pick a preset, then turn a section on.
    h.state_mut().run("tools.setBrush", json!({ "preset": "Chalk" })).unwrap();
    h.run_steps(2);
    let n = h.state().session.tools.presets.len();
    let at = h.state().session.tools.presets.iter().position(|p| p.name == "Chalk").unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::Switch, "Enable Scattering").click();
    h.run_steps(3);
    assert!(h.state().session.tools.brush.scattering.enabled);
    // The update button overwrites the picked preset in place: no new preset, position kept.
    h.get_by_label("Update the current brush with these settings").click();
    h.run_steps(3);
    assert!(h.state().session.journal.iter().any(|(id, p)| id == "brush.presets.update" && p["brush"]["scattering"]["enabled"] == json!(true)));
    assert_eq!(h.state().session.tools.presets.len(), n);
    let chalk = h.state().session.tools.presets.get(at).unwrap();
    assert_eq!(chalk.name, "Chalk");
    assert!(chalk.brush.scattering.enabled);
    assert!(!chalk.builtin, "an updated built-in becomes the user's preset");
}

/// Each painting tool keeps its own brush (#218): the preset picked for one tool is not the
/// current preset of the next, so Update can't overwrite it with the other tool's brush.
#[test]
fn switching_tools_forgets_the_picked_preset() {
    let mut app = app();
    app.ui.tool = crate::state::Tool::Brush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({ "preset": "Chalk" })).unwrap();
    assert_eq!(app.session.tools.current_preset.as_deref(), Some("Chalk"));
    app.ui.tool = crate::state::Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    assert_eq!(app.session.tools.current_preset, None);
    assert!(app.run("brush.presets.update", json!({})).is_err());
}

#[test]
fn fixed_size_shows_and_takes_the_unit_picked_on_its_fields() {
    use egui::accesskit::Role;

    // 254 ppi is 10 px a millimetre.
    let mut app = app();
    app.run("file.new", json!({"width": 400, "height": 200, "resolution": 254})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::RectMarquee;
    let o = &mut app.ui.tool_options;
    (o.marquee_style, o.marquee_width, o.marquee_height) = ("fixedSize".into(), 100.0, 50.0);
    assert_eq!(o.marquee_unit, "pixels");
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1400.0, 360.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    let has = |h: &Harness<'static, Box<PhotocraftApp>>, v: f64| h.query_all_by_role(Role::SpinButton).any(|n| n.accesskit_node().numeric_value() == Some(v));
    assert!(has(&h, 100.0) && has(&h, 50.0));

    // A right-click on Width lists the units; Millimeters shows both sides in mm.
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(100.0)).unwrap().click_secondary();
    h.run_steps(3);
    h.get_by_label("Millimeters").click();
    h.run_steps(4);
    let o = &h.state().ui.tool_options;
    assert_eq!(o.marquee_unit, "mm");
    assert_eq!((o.marquee_width, o.marquee_height), (100.0, 50.0), "the size itself stays in pixels");
    assert!(has(&h, 10.0) && has(&h, 5.0), "100 x 50 px at 254 ppi is 10 x 5 mm");

    // A number typed now is in mm.
    let field = h.query_all_by_role(Role::SpinButton).position(|n| n.accesskit_node().numeric_value() == Some(10.0)).unwrap();
    h.query_all_by_role(Role::SpinButton).nth(field).unwrap().click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text("20".into()));
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.marquee_width, 200.0);

    // Entering and leaving a field without an edit keeps the pixels, even where the unit rounds
    // them (50 px at 254 ppi is 0.197 in, which reads back as 50.038 px).
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().numeric_value() == Some(5.0)).unwrap().click_secondary();
    h.run_steps(3);
    h.get_by_label("Inches").click();
    h.run_steps(4);
    let field = h.query_all_by_role(Role::SpinButton).position(|n| n.accesskit_node().numeric_value().is_some_and(|v| (v - 0.19685).abs() < 1e-4)).unwrap();
    h.query_all_by_role(Role::SpinButton).nth(field).unwrap().click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.tool_options.marquee_unit, "inches");
    assert_eq!(h.state().ui.tool_options.marquee_height, 50.0);
}

#[test]
fn brush_settings_hamburger_menu_actions() {
    let command = |c: BrushSettingsCommand| CommandRef::from(c);
    let state = |app: &PhotocraftApp, c: BrushSettingsCommand| command_states(app).into_iter().find(|(r, _)| *r == command(c)).map(|(_, s)| s).unwrap();

    // 1. The menu offers Photoshop's four actions in order.
    let mut app = app();
    let ctx = app.ctx().clone();
    let offered: Vec<BrushSettingsCommand> =
        panel_rows(&app).iter().filter_map(|row| if let PanelRow::Command(c) = row { BrushSettingsCommand::from_ref(c) } else { None }).collect();
    assert_eq!(
        offered,
        [BrushSettingsCommand::NewPreset, BrushSettingsCommand::ClearControls, BrushSettingsCommand::ResetLocks, BrushSettingsCommand::CopyTexture]
    );
    assert!(!run_fired(&mut app, &ctx, &CommandRef::new("file.save")), "another panel's command is not run");

    // 2. New Brush Preset… opens the New Brush Preset dialog and saves nothing yet.
    let (dialogs, presets) = (app.dialog_count(), app.session.tools.presets.len());
    assert!(run_fired(&mut app, &ctx, &command(BrushSettingsCommand::NewPreset)));
    assert_eq!(app.dialog_count(), dialogs + 1);
    assert_eq!(app.session.tools.presets.len(), presets);

    // 3. Clear Brush Controls resets dynamics, keeps locked sections and smoothing.
    let mut app = self::app();
    app.session.tools.brush.size = 45.0;
    app.session.tools.brush.scattering.enabled = true;
    app.session.tools.brush.texture.enabled = true;
    app.session.tools.brush.locks.texture = true;
    app.session.tools.brush.smoothing.amount = 0.5;
    assert!(run_fired(&mut app, &ctx, &command(BrushSettingsCommand::ClearControls)));
    let b = &app.session.tools.brush;
    assert!(!b.scattering.enabled, "unlocked scattering cleared");
    assert!(b.texture.enabled, "locked texture preserved");
    assert_eq!(b.smoothing.amount, 0.5, "smoothing preserved");
    assert_eq!(b.size, 45.0, "tip size preserved");

    // 4. Reset All Locked Settings is disabled without a lock and clears them with one.
    assert_eq!(state(&self::app(), BrushSettingsCommand::ResetLocks), CommandState::new().enabled(false));
    assert!(app.session.tools.brush.locks.texture);
    assert_eq!(state(&app, BrushSettingsCommand::ResetLocks), CommandState::new().enabled(true));
    assert!(run_fired(&mut app, &ctx, &command(BrushSettingsCommand::ResetLocks)));
    assert_eq!(app.session.tools.brush.locks, photocraft_paint::brush::SectionLocks::default());

    // 5. Copy Texture to Other Tools is disabled with Texture off and copies only to tools with Texture on.
    let mut app = self::app();
    assert_eq!(state(&app, BrushSettingsCommand::CopyTexture), CommandState::new().enabled(false));
    app.session.tools.brush.texture.enabled = true;
    app.session.tools.brush.texture.scale = 2.5;
    assert_eq!(state(&app, BrushSettingsCommand::CopyTexture), CommandState::new().enabled(true));
    let mut pencil = BrushSettings::default();
    pencil.texture.enabled = true;
    pencil.texture.scale = 1.0;
    let mut eraser = BrushSettings::default();
    eraser.texture.scale = 1.0;
    app.ui.tool_brushes = vec![(crate::state::Tool::Pencil, pencil), (crate::state::Tool::Eraser, eraser)];
    assert!(run_fired(&mut app, &ctx, &command(BrushSettingsCommand::CopyTexture)));
    let scale = |app: &PhotocraftApp, tool| app.ui.tool_brushes.iter().find(|(t, _)| *t == tool).unwrap().1.texture.scale;
    assert_eq!(scale(&app, crate::state::Tool::Pencil), 2.5);
    assert_eq!(scale(&app, crate::state::Tool::Eraser), 1.0, "a tool with Texture off keeps its own");
}

/// Each event family of the section list, applied through the panel's own dispatch, runs its command
/// and edits its state.
#[test]
fn panel_events_map_to_their_commands() {
    use super::super::brush_settings_events::{apply_action, apply_section_events};
    use craft::draw::ui::brush_settings::{BrushPanelAction, BrushSectionEvent};

    // A section's flag and lock edit only their own field and leave as one `tools.setBrush` of that field;
    // a switch keeps its options (Smoothing keeps its amount).
    let mut app = app();
    app.run("tools.setBrush", json!({"brush": {"smoothing": {"amount": 0.65, "pulledString": true}}})).unwrap();
    let cases: [(BrushSectionEvent<usize>, Value); 4] = [
        (BrushSectionEvent::enable(1, true), json!({"shapeDynamics": {"enabled": true}})),
        (BrushSectionEvent::lock(3, true), json!({"locks": {"texture": true}})),
        (BrushSectionEvent::switch(8, true), json!({"noise": true})),
        (BrushSectionEvent::switch(11, false), json!({"smoothing": {"enabled": false}})),
    ];
    for (event, patch) in cases {
        let before = app.session.tools.brush.clone();
        let mut after = before.clone();
        let mut shown = 0;
        assert!(!apply_section_events(&mut after, &mut shown, vec![event]));
        assert_eq!(shown, 0);
        commit(&mut app, &before, &after);
        let (command, params) = last_journal(&app).unwrap();
        assert_eq!((command.as_str(), &params["brush"]), ("tools.setBrush", &patch));
    }
    assert_eq!(app.session.tools.brush.smoothing.amount, 0.65);

    // Select shows the pane and runs no command; the Brushes link asks to open the Brushes panel.
    let journal = app.session.journal.len();
    let mut brush = app.session.tools.brush.clone();
    let mut shown = 0;
    assert!(!apply_section_events(&mut brush, &mut shown, vec![BrushSectionEvent::Select(2)]));
    assert_eq!((shown, &brush), (2, &app.session.tools.brush));
    assert!(apply_section_events(&mut brush, &mut shown, vec![BrushSectionEvent::OpenLink]));
    assert_eq!((shown, app.session.journal.len()), (2, journal));

    // Reset keeps the colours, smoothing and locks.
    let mut brush = app.session.tools.brush.clone();
    brush.size = 99.0;
    brush.locks.texture = true;
    apply_action(&mut app, &mut brush, BrushPanelAction::Reset);
    assert_eq!((brush.size, brush.locks.texture, brush.smoothing.amount), (BrushSettings::default().size, true, 0.65));
}
