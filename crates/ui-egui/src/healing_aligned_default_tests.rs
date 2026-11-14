//! Healing's Aligned default is separate from the Clone Stamp's (#12100).
use photocraft_engine::Session;
use serde_json::{Value, json};

use crate::state::{Tool, ToolOptions};
use crate::{PhotocraftApp, Services};

#[test]
fn old_healing_settings_start_unaligned_and_round_trip_independently() {
    let defaults = ToolOptions::default();
    assert!(!defaults.healing_aligned);
    assert!(defaults.clone_aligned);
    for clone_aligned in [false, true] {
        let mut options: ToolOptions = serde_json::from_value(json!({"clone_aligned": clone_aligned})).unwrap();
        assert!(!options.healing_aligned, "the missing new field must not inherit Clone Stamp's value");
        assert_eq!(options.clone_aligned, clone_aligned);
        options.healing_aligned = true;
        assert_eq!(options.clone_aligned, clone_aligned);
        let saved = serde_json::to_value(&options).unwrap();
        let mut restored: ToolOptions = serde_json::from_value(saved).unwrap();
        assert!(restored.healing_aligned);
        assert_eq!(restored.clone_aligned, clone_aligned);
        restored.clone_aligned = !clone_aligned;
        assert!(restored.healing_aligned, "changing Clone Stamp must not toggle Healing");
    }
}

fn pick_tool(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(app);
}

#[test]
fn healing_and_clone_checkboxes_feed_only_their_own_stroke_setting() {
    use egui_kittest::{Harness, kittest::Queryable};

    let mut session = Session::new();
    session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    session.execute("cloneSource.set", json!({"source": [2, 2]})).unwrap();
    let mut app = PhotocraftApp::new(session, Services::default());
    pick_tool(&mut app, Tool::Healing);
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 100.0)).build_ui_state(
        |ui, app| {
            crate::paint_mouse::sync_tool_brush(app);
            let tool = app.ui.tool;
            super::options_bar(app, ui, tool);
        },
        app,
    );
    h.run_steps(3);
    assert_eq!(super::clone_params(h.state()).unwrap()["aligned"], false);
    for (tool, healing, clone) in [(Tool::Healing, true, true), (Tool::CloneStamp, true, false), (Tool::Healing, false, false), (Tool::CloneStamp, false, true)]
    {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        h.get_by_label("Aligned").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.tool_options.healing_aligned, healing);
        assert_eq!(h.state().ui.tool_options.clone_aligned, clone);
        assert_eq!(super::clone_params(h.state()).unwrap()["aligned"], if tool == Tool::Healing { healing } else { clone });
        assert!(h.state().ui.tool_options.pattern_stamp_aligned, "Pattern Stamp's unmeasured setting is untouched");
    }
}

#[test]
fn reset_healing_leaves_clone_alignment_and_other_tool_brushes_alone() {
    for clone_aligned in [false, true] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let mut app = PhotocraftApp::new(session, Services::default());
        pick_tool(&mut app, Tool::Brush);
        app.session.tools.brush.angle = 31.0;
        app.session.tools.brush.pressure_size = true;
        pick_tool(&mut app, Tool::CloneStamp);
        app.session.tools.brush.angle = -47.0;
        app.session.tools.brush.pressure_size = true;
        app.ui.tool_options.clone_aligned = clone_aligned;
        pick_tool(&mut app, Tool::Healing);
        app.ui.tool_options.healing_aligned = true;
        app.session.tools.brush.angle = 86.0;
        app.session.tools.brush.pressure_size = true;

        crate::menus::invoke(&mut app, &egui::Context::default(), crate::tool_reset::RESET, json!({})).unwrap();
        assert!(!app.ui.tool_options.healing_aligned);
        assert_eq!(app.ui.tool_options.clone_aligned, clone_aligned);
        assert_eq!(app.session.tools.brush.angle, 0.0);
        assert!(!app.session.tools.brush.pressure_size);
        pick_tool(&mut app, Tool::CloneStamp);
        assert_eq!(app.session.tools.brush.angle, -47.0);
        assert!(app.session.tools.brush.pressure_size);
        assert_eq!(app.ui.tool_options.clone_aligned, clone_aligned);
        pick_tool(&mut app, Tool::Brush);
        assert_eq!(app.session.tools.brush.angle, 31.0);
        assert!(app.session.tools.brush.pressure_size);
        pick_tool(&mut app, Tool::Healing);
        assert!(!app.ui.tool_options.healing_aligned);
        assert_eq!(app.session.tools.brush.angle, 0.0);
        assert!(!app.session.tools.brush.pressure_size);
    }
}

fn sampled_session(depth: u64) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 40, "depth": depth, "mode": "rgb"})).unwrap();
    s.edit("source texture", |doc, active| {
        let bounds = doc.bounds();
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let mut data = Vec::new();
        for y in bounds.y0..bounds.y1 {
            for x in bounds.x0..bounds.x1 {
                let pixel = if (4..12).contains(&x) && (4..12).contains(&y) {
                    let value = if ((x / 2) + (y / 2)) % 2 == 0 { 0.2 } else { 0.8 };
                    [value, value, value, 1.0]
                } else {
                    [0.2, 0.6, 0.3, 1.0]
                };
                data.extend(photocraft_raster::from_rgba(&format, pixel));
            }
        }
        surface.write_region(bounds, &data);
        Ok(())
    })
    .unwrap();
    s.execute("cloneSource.set", json!({"source": [8, 8]})).unwrap();
    s
}

fn dab(s: &mut Session, command: &str, at: [i32; 2], aligned: Option<bool>) -> Value {
    let mut params = json!({"points": [at], "size": 8, "hardness": 100});
    if let Some(aligned) = aligned {
        params["aligned"] = json!(aligned);
    }
    s.execute(command, params).unwrap()
}

fn patch(s: &Session, x: i32, y: i32) -> Vec<[f32; 4]> {
    let state = s.active().unwrap();
    let surface = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap();
    (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| surface.rgba(x + dx, y + dy))).collect()
}

#[test]
fn omitted_healing_alignment_restarts_the_sample_without_reusing_or_replacing_an_anchor() {
    for depth in [8, 16, 32] {
        let (mut default, mut off, mut on) = (sampled_session(depth), sampled_session(depth), sampled_session(depth));
        for at in [[24, 24], [44, 24]] {
            let implicit = dab(&mut default, "paint.healingBrush", at, None);
            let explicit = dab(&mut off, "paint.healingBrush", at, Some(false));
            let aligned = dab(&mut on, "paint.healingBrush", at, Some(true));
            assert_eq!(implicit, explicit, "depth {depth}, stroke {at:?}");
            assert_eq!(implicit["aligned"], false);
            assert_eq!(implicit["offset"], json!([8 - at[0], 8 - at[1]]));
            assert_eq!(implicit["nextSource"], json!([8.0, 8.0]));
            assert_eq!(aligned["aligned"], true);
            assert_eq!(aligned["offset"], json!([-16, -16]));
            assert!(aligned["nextSource"].is_null());
        }
        assert_eq!(default.presets.clone.active().anchor, None);
        assert_eq!(default.presets.clone.active().last_anchor, Some([44.0, 24.0]));
        assert_eq!(on.presets.clone.active().anchor, Some([24.0, 24.0]));
        assert_eq!(patch(&default, 24, 24), patch(&default, 44, 24), "depth {depth}: source texture restarts at the same phase");
        assert_eq!(patch(&default, 44, 24), patch(&off, 44, 24));
        assert_ne!(patch(&default, 44, 24), patch(&on, 44, 24), "the aligned control samples past the checker");

        let resumed = dab(&mut on, "paint.healingBrush", [44, 30], None);
        assert_eq!(resumed["offset"], json!([-36, -22]), "default off must ignore an existing aligned anchor");
        assert_eq!(on.presets.clone.active().anchor, Some([24.0, 24.0]), "unaligned bookkeeping must preserve the aligned pairing");
        let mut stamp = sampled_session(depth);
        for at in [[24, 24], [44, 24]] {
            let result = dab(&mut stamp, "paint.cloneStamp", at, None);
            assert_eq!(result["aligned"], true, "Clone Stamp's omitted default is unchanged");
            assert_eq!(result["offset"], json!([-16, -16]));
        }
    }
}

#[test]
fn pending_retouch_stroke_keeps_its_alignment_after_selection_changes() {
    for tool in [Tool::Healing, Tool::CloneStamp] {
        for switched in [false, true] {
            let mut app = PhotocraftApp::new(sampled_session(8), Services::default());
            app.ui.tool = if switched { if tool == Tool::Healing { Tool::CloneStamp } else { Tool::Healing } } else { tool };
            app.ui.tool_options.healing_aligned = false;
            app.ui.tool_options.clone_aligned = true;
            app.session.tools.brush.size = 8.0;
            let drag = crate::canvas::Drag::new(tool, [24.0, 24.0], vec![[24.0, 24.0, 1.0]], egui::Modifiers::NONE, false);
            crate::canvas::finish_gesture(&mut app, drag);
            assert!(!app.ui.status_error, "{}", app.ui.status);
            let (command, params) = app.session.journal.last().unwrap();
            assert_eq!(command, if tool == Tool::Healing { "paint.healingBrush" } else { "paint.cloneStamp" });
            assert_eq!(params["aligned"], tool == Tool::CloneStamp, "Aligned follows the stroke tool, including after a switch");
            assert_eq!(app.ui.clone_offset.is_some(), tool == Tool::CloneStamp);
        }
    }
}

#[test]
fn healing_source_mapping_uses_its_own_alignment_without_changing_the_anchor() {
    let mut app = PhotocraftApp::new(sampled_session(8), Services::default());
    app.session.presets.clone.active_mut().anchor = Some([24.0, 24.0]);
    for (healing, clone) in [(false, true), (true, false), (false, false), (true, true)] {
        app.ui.tool_options.healing_aligned = healing;
        app.ui.tool_options.clone_aligned = clone;
        for tool in [Tool::Healing, Tool::CloneStamp] {
            app.ui.tool = tool;
            let sample = crate::preset_panels::clone_sample_point(&app, Some([44.0, 24.0])).unwrap();
            let aligned = if tool == Tool::Healing { healing } else { clone };
            assert_eq!(sample, if aligned { [28.0, 8.0] } else { [8.0, 8.0] });
            assert_eq!(app.session.presets.clone.active().anchor, Some([24.0, 24.0]), "preview must not store a new pairing");
        }
    }
}
