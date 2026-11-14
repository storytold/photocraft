//! #11884: the Brush Preset picker's row of recently used brushes under the search field. The
//! current preset goes to the front (moving there if it is already in the row), the row is capped,
//! a deleted preset leaves it, a click on a tile picks that brush, and the gear's Show Recent
//! Brushes shows and hides it, remembered with the picker's other view settings.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = Tool::Brush;
    app.sync_views();
    app
}

fn names(app: &PhotocraftApp) -> Vec<String> {
    app.session.tools.presets.iter().map(|p| p.name.clone()).collect()
}

fn pick(app: &mut PhotocraftApp, name: &str) {
    app.run("tools.setBrush", json!({ "preset": name })).unwrap();
    let ctx = egui::Context::default();
    persist(app, &ctx);
}

fn recent(app: &PhotocraftApp) -> Vec<String> {
    app.ui.brush_picker_list.recent.clone()
}

#[test]
fn the_current_preset_goes_to_the_front_once() {
    let mut app = app();
    let n = names(&app);
    assert!(n.len() >= 4, "{n:?}");
    for name in [&n[0], &n[1], &n[2]] {
        pick(&mut app, name);
    }
    assert_eq!(recent(&app), [n[2].clone(), n[1].clone(), n[0].clone()], "newest first");
    pick(&mut app, &n[0]);
    assert_eq!(recent(&app), [n[0].clone(), n[2].clone(), n[1].clone()], "moved to the front, not twice");
    // The same preset again changes nothing.
    pick(&mut app, &n[0]);
    assert_eq!(recent(&app).len(), 3);
}

#[test]
fn the_row_keeps_at_most_recent_max() {
    let mut app = app();
    let n = names(&app);
    assert!(n.len() > RECENT_MAX, "enough built-in presets: {}", n.len());
    for name in n.iter().take(RECENT_MAX + 3) {
        pick(&mut app, name);
    }
    let r = recent(&app);
    assert_eq!(r.len(), RECENT_MAX);
    assert_eq!(r[0], n[RECENT_MAX + 2], "the newest stays");
    assert!(!r.contains(&n[0]), "the oldest dropped");
}

#[test]
fn a_preset_changed_anywhere_is_noted_by_the_frame_loop() {
    // Not through the picker: the frame loop's `persist` notes it all the same.
    let mut app = app();
    let n = names(&app);
    app.run("tools.setBrush", json!({ "preset": n[1] })).unwrap();
    assert!(recent(&app).is_empty(), "nothing until a frame runs");
    persist(&mut app, &egui::Context::default());
    assert_eq!(recent(&app), [n[1].clone()]);
}

#[test]
fn a_deleted_preset_leaves_the_row() {
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[0]);
    pick(&mut app, &n[1]);
    app.run("brush.presets.delete", json!({ "name": n[0] })).unwrap();
    persist(&mut app, &egui::Context::default());
    assert_eq!(recent(&app), [n[1].clone()], "no stale entry");
    // A row holding a name no preset has (a stale saved value) drops it too.
    app.ui.brush_picker_list.recent.push("No Such Brush".into());
    note_recent(&mut app.ui.brush_picker_list, Some(&n[1]), &app.session.tools.presets);
    assert_eq!(recent(&app), [n[1].clone()]);
    // No current preset (a hand-edited brush) adds nothing.
    note_recent(&mut app.ui.brush_picker_list, None, &app.session.tools.presets);
    assert_eq!(recent(&app), [n[1].clone()]);
}

/// The picker as the canvas opens it.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            persist(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let c = h.state().last_canvas_rect.center();
    let ctx = h.ctx.clone();
    open(&mut h.state_mut().ui, &ctx, [c.x, c.y]);
    h.run_steps(3);
    h
}

fn tile(name: &str) -> String {
    format!("Recent brush: {name}")
}

#[test]
fn clicking_a_recent_tile_picks_that_brush() {
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[3]);
    pick(&mut app, &n[1]);
    let mut h = harness(app);
    assert!(h.query_by_label(&tile(&n[3])).is_some() && h.query_by_label(&tile(&n[1])).is_some(), "both tiles show");
    h.get_by_label(&tile(&n[3])).click();
    h.run_steps(2);
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some(n[3].as_str()), "picked as from the groups");
    let (brush, preset) = (&h.state().session.tools.brush, &h.state().session.tools.presets[3].brush);
    assert_eq!((brush.size, brush.hardness, &brush.tip), (preset.size, preset.hardness, &preset.tip), "the preset's tip and size");
    assert!(h.state().ui.brush_picker.is_some(), "a click keeps the picker open");
    // It moves to the front once the pointer is off the row.
    let below = h.get_by_label("Brush Preset Options").rect().center() + egui::vec2(0.0, 200.0);
    h.hover_at(below);
    h.run_steps(2);
    assert_eq!(h.state().ui.brush_picker_list.recent[0], n[3], "and it moves to the front");
}

#[test]
fn show_recent_brushes_hides_the_row_and_is_remembered() {
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[2]);
    let mut h = harness(app);
    assert!(h.state().ui.brush_picker_list.show_recent, "checked by default");
    assert!(h.query_by_label(&tile(&n[2])).is_some());
    h.get_by_label("Brush Preset Options").click();
    h.run_steps(2);
    h.get_by_label("Show Recent Brushes").click();
    h.run_steps(3);
    assert!(!h.state().ui.brush_picker_list.show_recent);
    assert!(h.query_by_label(&tile(&n[2])).is_none(), "the row is hidden");
    assert_eq!(h.state().session.prefs().brush_picker["showRecent"], json!(false), "remembered with the picker's view");
}

#[test]
fn the_saved_toggle_is_restored_and_a_bad_value_keeps_the_default() {
    let mut app = app();
    app.session.prefs.edit(|p| p.brush_picker = json!({"showRecent": false}));
    restore(&mut app);
    assert!(!app.ui.brush_picker_list.show_recent);
    for bad in [json!({"showRecent": "no"}), json!({"showRecent": null}), json!(42), json!({})] {
        let mut app = self::app();
        app.session.prefs.edit(|p| p.brush_picker = bad.clone());
        restore(&mut app);
        assert!(app.ui.brush_picker_list.show_recent, "{bad}");
    }
}

#[test]
fn a_preset_picked_in_another_case_is_noted_by_its_own_name() {
    // `tools.setBrush` finds a preset whatever the case of its name and keeps the name as given.
    let mut app = app();
    let n = names(&app);
    let shouted = n[1].to_ascii_uppercase();
    assert_ne!(shouted, n[1], "the preset's name has lower-case letters");
    pick(&mut app, &shouted);
    assert_eq!(recent(&app), [n[1].clone()], "noted under the preset's own name");
    let h = harness(app);
    assert_eq!(h.get_by_label(&tile(&n[1])).accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "and shown as the current brush");
}

#[test]
fn double_clicking_a_recent_tile_chooses_that_brush() {
    // The first click moves the tile to the front; the second lands on whatever moved under the
    // pointer, and must still choose the brush the double-click began on.
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[3]);
    pick(&mut app, &n[1]);
    let mut h = harness(app);
    assert_eq!(h.state().ui.brush_picker_list.recent, [n[1].clone(), n[3].clone()]);
    let p = h.get_by_label(&tile(&n[3])).rect().center();
    h.hover_at(p);
    h.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(2);
    }
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some(n[3].as_str()), "the brush the double-click began on");
    assert!(h.state().ui.brush_picker.is_none(), "a double-click closes the picker");
}

#[test]
fn a_full_recent_row_fits_the_narrowest_picker() {
    let mut app = app();
    let n = names(&app);
    for name in n.iter().take(RECENT_MAX) {
        pick(&mut app, name);
    }
    app.ui.brush_picker_size = Some(MIN_SIZE);
    let mut h = harness(app);
    h.run_steps(3);
    let right = h.get_by_label("Brush Preset Options").rect().max.x;
    for name in n.iter().take(RECENT_MAX) {
        let r = h.get_by_label(&tile(name)).rect();
        assert!(r.max.x <= right + 0.5, "`{name}`'s tile ends at {} past the picker's right edge {right}", r.max.x);
    }
}

#[test]
fn a_clicked_recent_tile_stays_under_the_pointer_until_it_leaves_the_row() {
    // The click picks the brush, but the row keeps its order while the pointer is on it: the tile
    // that was clicked stays where the pointer is, and moves to the front once the pointer leaves.
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[3]);
    pick(&mut app, &n[1]);
    let mut h = harness(app);
    let p = h.get_by_label(&tile(&n[3])).rect().center();
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(3);
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some(n[3].as_str()), "the click picks the brush");
    assert_eq!(recent(h.state()), [n[1].clone(), n[3].clone()], "the row holds its order under the pointer");
    assert!(h.get_by_label(&tile(&n[3])).rect().contains(p), "the clicked tile is still under the pointer");
    let below = h.get_by_label("Brush Preset Options").rect().center() + egui::vec2(0.0, 200.0);
    h.hover_at(below);
    h.run_steps(2);
    assert_eq!(recent(h.state()), [n[3].clone(), n[1].clone()], "off the row, the picked brush goes to the front");
    assert!(h.state().ui.brush_picker.is_some(), "the picker stays open");
}

#[test]
fn every_brush_picked_on_the_row_counts_once_the_pointer_leaves() {
    // Two brushes picked one after the other with the pointer on the row: the row holds still,
    // and once the pointer leaves, the last one is at the front and the one before it second.
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[5]);
    pick(&mut app, &n[3]);
    pick(&mut app, &n[1]);
    let mut h = harness(app);
    assert_eq!(recent(h.state()), [n[1].clone(), n[3].clone(), n[5].clone()]);
    for name in [&n[5], &n[3]] {
        let p = h.get_by_label(&tile(name)).rect().center();
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(3);
        assert_eq!(h.state().session.tools.current_preset.as_deref(), Some(name.as_str()), "the click picks `{name}`");
    }
    assert_eq!(recent(h.state()), [n[1].clone(), n[3].clone(), n[5].clone()], "the row holds its order under the pointer");
    let below = h.get_by_label("Brush Preset Options").rect().center() + egui::vec2(0.0, 200.0);
    h.hover_at(below);
    h.run_steps(2);
    assert_eq!(recent(h.state()), [n[3].clone(), n[5].clone(), n[1].clone()], "both picks count, the last one first");
}

#[test]
fn quick_clicks_on_two_neighbouring_tiles_pick_the_second() {
    // Two clicks a few pixels apart across the gap between two tiles count as a double-click for
    // egui, but they are on different brushes: the second click picks its own brush and the
    // picker stays open.
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[3]);
    pick(&mut app, &n[1]);
    let mut h = harness(app);
    assert_eq!(recent(h.state()), [n[1].clone(), n[3].clone()]);
    let (a, b) = (h.get_by_label(&tile(&n[1])).rect(), h.get_by_label(&tile(&n[3])).rect());
    let (p, q) = (egui::pos2(a.max.x - 1.0, a.center().y), egui::pos2(b.min.x + 1.0, b.center().y));
    assert!(p.distance(q) < 6.0, "the clicks are close enough for a double-click");
    for at in [p, q] {
        h.hover_at(at);
        h.run_steps(1);
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(2);
    }
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some(n[3].as_str()), "the second click picks its own brush");
    assert!(h.state().ui.brush_picker.is_some(), "clicks on two brushes keep the picker open");
}

#[test]
fn a_double_click_begun_beside_a_tile_chooses_that_tile_not_an_older_click() {
    // A tile clicked earlier moves to the front once the pointer leaves, so another brush is now
    // where it was. A double-click whose first press lands in the gap beside that brush is a
    // double-click on it for egui: it chooses the brush under the pointer, not the one the
    // earlier click picked.
    let mut app = app();
    let n = names(&app);
    pick(&mut app, &n[3]);
    pick(&mut app, &n[1]);
    let mut h = harness(app);
    let p = h.get_by_label(&tile(&n[3])).rect().center();
    h.hover_at(p);
    h.run_steps(1);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
    }
    h.run_steps(2);
    let below = h.get_by_label("Brush Preset Options").rect().center() + egui::vec2(0.0, 200.0);
    h.hover_at(below);
    h.run_steps(40);
    assert_eq!(recent(h.state()), [n[3].clone(), n[1].clone()], "the clicked brush went to the front");
    let r = h.get_by_label(&tile(&n[1])).rect();
    assert!(r.contains(p), "the other brush is now where the first click was");
    let (gap, on) = (egui::pos2(r.min.x - 1.5, r.center().y), egui::pos2(r.min.x + 1.0, r.center().y));
    for at in [gap, on] {
        h.hover_at(at);
        h.run_steps(1);
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(2);
    }
    assert_eq!(h.state().session.tools.current_preset.as_deref(), Some(n[1].as_str()), "the brush under the double-click");
    assert!(h.state().ui.brush_picker.is_none(), "a double-click closes the picker");
}
