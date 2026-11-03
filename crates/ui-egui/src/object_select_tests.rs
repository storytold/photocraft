use egui::vec2;
use egui_kittest::Harness;
use photocraft_genai::fake::{FakeNative, Options};
use serde_json::json;

use super::*;
use crate::canvas::{ToolEvent, tool_event};

/// A 100×60 document with a layer, talking to `url`, the Object Selection tool chosen.
fn app(url: &str) -> PhotocraftApp {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    a.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    a.run("layer.new.layer", json!({})).unwrap();
    a.session.edit_prefs(|p| {
        p.integrations.models_path = url.to_string();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    a.ui.tool = Tool::ObjectSelection;
    // The new, empty layer is active: the classical rectangle wants pixels under it.
    a.ui.tool_options.sample_all.object_selection = true;
    a.sync_views();
    a
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn selection_bounds(app: &PhotocraftApp) -> Option<photocraft_geom::Rect> {
    app.session.active().unwrap().doc.selection.as_ref().map(|m| m.content_bounds())
}

fn last_params(app: &PhotocraftApp) -> (String, serde_json::Value) {
    let (id, p) = app.session.journal.last().unwrap();
    (id.clone(), p.clone())
}

#[test]
fn a_drag_is_a_box_prompt_and_ctrl_clicks_refine_the_last_object() {
    let fake = FakeNative::start_with(Options { missing_files: vec!["model.safetensors".into()], ..Options::default() }).unwrap();
    let mut a = app(&fake.url);
    let none = egui::Modifiers::NONE;
    // A box around the object: the box template, refine on by default.
    tool_event(&mut a, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
    tool_event(&mut a, ToolEvent::Move { x: 60.0, y: 50.0, pressure: 1.0 }, none);
    tool_event(&mut a, ToolEvent::Up { x: 60.0, y: 50.0 }, none);
    let (id, p) = last_params(&a);
    assert_eq!((id.as_str(), p["box"].clone(), p["refine"].clone()), ("select.byPoint", json!([10.0, 10.0, 50.0, 40.0]), json!(true)));
    assert!(selection_bounds(&a).is_some());
    let steps = a.session.active().unwrap().history.past_len();
    // Refine mode, ⌥-click: the same query again with the spot excluded, replacing its result
    // (one undo step out, one in).
    a.ui.tool_options.object_refine = true;
    click(&mut a, 20.0, 20.0, egui::Modifiers::ALT);
    let (id, p) = last_params(&a);
    assert_eq!(id, "select.byPoint");
    assert_eq!(p["negativePoints"], json!([[20.0, 20.0]]));
    assert_eq!(p["box"], json!([10.0, 10.0, 50.0, 40.0]), "the box stays");
    assert_eq!(a.session.active().unwrap().history.past_len(), steps, "undo + redo = the same depth");
    assert_eq!(fake.state().runs.len(), 2);
    assert_eq!(fake.state().runs[1].request["params"]["negatives"], r#"[{"x":20,"y":20}]"#);
    // A plain click in Refine mode adds a spot the object must include.
    click(&mut a, 40.0, 30.0, none);
    let (_, p) = last_params(&a);
    assert_eq!(p["points"], json!([[40.0, 30.0]]));
    assert_eq!(p["negativePoints"], json!([[20.0, 20.0]]));
    assert_eq!(a.ui.object_select.last.as_ref().map(|l| (l.positives.len(), l.negatives.len())), Some((1, 1)));
    // Another edit in between ends the refinement: the next click is a new object.
    a.run("select.rect", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    click(&mut a, 50.0, 30.0, none);
    let (_, p) = last_params(&a);
    assert!(p.get("negativePoints").is_none());
    assert_eq!(p["points"], json!([[50.0, 30.0]]));
    // Back in New mode: ⇧-click adds a new object, ⌥-click subtracts one, fresh detections.
    a.ui.tool_options.object_refine = false;
    click(&mut a, 70.0, 30.0, egui::Modifiers::SHIFT);
    assert_eq!(last_params(&a).1["mode"], "add");
    click(&mut a, 70.0, 30.0, egui::Modifiers::ALT);
    assert_eq!(last_params(&a).1["mode"], "subtract");
    // Refine edge off: the command is told.
    a.ui.tool_options.refine_edge = false;
    click(&mut a, 30.0, 30.0, none);
    let p = last_params(&a).1;
    assert_eq!(p["refine"], false);
    assert!(p.get("grow").is_none() && p.get("fillHoles").is_none() && p.get("soften").is_none(), "the mask stack goes along only when set: {p}");
    // The options bar's Widen, Fill Holes and Soften travel as the command's mask stack.
    let plain = selection_bounds(&a).unwrap();
    a.ui.tool_options.object_widen = 6.0;
    a.ui.tool_options.object_fill_holes = true;
    click(&mut a, 30.0, 30.0, none);
    let p = last_params(&a).1;
    assert_eq!((p["grow"].as_f64(), p["fillHoles"].as_bool(), p.get("soften")), (Some(6.0), Some(true), None));
    assert_eq!(selection_bounds(&a).map(|b| b.x0), Some(plain.x0 - 6), "the same object, widened");
    // Soften feathers beyond the widened edge (a Gaussian tail), so only the parameter is checked.
    a.ui.tool_options.object_soften = 2.5;
    click(&mut a, 30.0, 30.0, none);
    assert_eq!(last_params(&a).1["soften"].as_f64(), Some(2.5));
}

#[test]
fn the_finder_probes_a_resting_pointer_and_the_click_applies_the_probe() {
    let fake = FakeNative::start().unwrap();
    let mut a = app(&fake.url);
    let mut h = Harness::builder().with_size(vec2(700.0, 500.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::canvas::document_area(app, ui);
        },
        std::mem::replace(&mut a, PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())),
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.state_mut().ui.tool_options.refine_edge = false;
    h.run_steps(4);
    // The pointer rests over the picture: after the delay the finder asks (inline here) and the
    // probe is kept, with its tint texture; the status bar shows nothing of it.
    let xf = ViewXform::active(h.state()).unwrap();
    let at = xf.to_screen(30.0, 20.0);
    h.hover_at(at);
    h.run_steps(2);
    std::thread::sleep(std::time::Duration::from_millis((PROBE_DELAY * 1000.0) as u64 + 60));
    h.hover_at(at);
    h.run_steps(3);
    let v = inspect(h.state());
    assert_eq!(v["probesAsked"], 1, "{v}");
    assert_eq!(v["probe"]["bounds"], json!([25, 15, 50, 30]), "the fake's centre quarter of 100×60");
    assert_eq!(fake.state().runs.len(), 1);
    assert!(crate::jobs_ui::job_in_view(h.state()).is_none());
    // Resting on: no second ask. A nudge inside the same cell: none either.
    h.run_steps(3);
    h.hover_at(at + vec2(1.0, 1.0));
    h.run_steps(3);
    assert_eq!(h.state().ui.object_select.probes_asked, 1);
    // The click applies the probe: no new detection, the selection is the probe's mask.
    let before = fake.state().runs.len();
    h.drag_at(at);
    h.run();
    h.drop_at(at);
    h.run_steps(3);
    assert_eq!(fake.state().runs.len(), before, "the kept probe, no round trip");
    assert_eq!(selection_bounds(h.state()), Some(photocraft_geom::Rect::new(25, 15, 75, 45)));
    assert!(last_params(h.state()).1["probe"].as_u64().is_some(), "the click named the probe");
    assert!(h.state().ui.object_select.probe.is_none(), "forgotten with the click");
    // Switching tools drops the finder's state.
    h.state_mut().ui.tool = Tool::RectMarquee;
    h.run_steps(2);
    assert!(h.state().ui.object_select.hover.is_none());
    // Nothing under the pointer: the finder remembers the spot and asks again only after a move.
    let empty = FakeNative::start_with(Options { segments: vec![], ..Options::default() }).unwrap();
    h.state_mut().session.edit_prefs(|p| {
        p.integrations.models_path = empty.url.clone();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    h.state_mut().ui.object_select = Default::default();
    h.state_mut().ui.tool = Tool::ObjectSelection;
    let far = xf.to_screen(80.0, 50.0);
    h.hover_at(far);
    h.run_steps(2);
    std::thread::sleep(std::time::Duration::from_millis((PROBE_DELAY * 1000.0) as u64 + 60));
    h.hover_at(far);
    h.run_steps(3);
    assert_eq!(h.state().ui.object_select.probes_asked, 1);
    assert!(h.state().ui.object_select.probe.is_none());
    h.run_steps(3);
    assert_eq!(h.state().ui.object_select.probes_asked, 1, "no retry without a move");
    // Off: no probes at all.
    h.state_mut().ui.tool_options.object_finder = false;
    h.state_mut().ui.object_select = Default::default();
    h.hover_at(at);
    std::thread::sleep(std::time::Duration::from_millis((PROBE_DELAY * 1000.0) as u64 + 60));
    h.hover_at(at);
    h.run_steps(3);
    assert_eq!(h.state().ui.object_select.probes_asked, 0);
}

#[test]
fn without_a_server_the_tool_is_the_classical_rectangle_and_select_subject_falls_back() {
    let mut a = app("");
    let none = egui::Modifiers::NONE;
    click(&mut a, 30.0, 20.0, none);
    assert!(a.session.journal.last().is_none_or(|(id, _)| id != "select.byPoint"));
    tool_event(&mut a, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, none);
    tool_event(&mut a, ToolEvent::Move { x: 60.0, y: 50.0, pressure: 1.0 }, none);
    tool_event(&mut a, ToolEvent::Up { x: 60.0, y: 50.0 }, none);
    assert_eq!(last_params(&a).0, "select.object");
    // The finder's health check is keyed by the server, so changing it asks again.
    assert!(!online(&mut a, 1.0));
    // Select › Subject from the menu: the classical one here, the detector with a server.
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut a, &ctx, "select.subject", json!({})).unwrap();
    assert_eq!(last_params(&a).0, "select.subject");
    let fake = FakeNative::start().unwrap();
    a.session.edit_prefs(|p| {
        p.integrations.models_path = fake.url.clone();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    crate::menus::invoke(&mut a, &ctx, "select.subject", json!({})).unwrap();
    let (id, p) = last_params(&a);
    assert_eq!((id.as_str(), p["refine"].as_bool()), ("select.subjectML", Some(true)));
    assert!(online(&mut a, 2.0), "a new server: checked afresh");
    a.session.edit_prefs(|p| {
        p.integrations.models_path = "http://127.0.0.1:9".into();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    assert!(!online(&mut a, 3.0), "and a dead one is seen as such at once, not from the cache");
}
