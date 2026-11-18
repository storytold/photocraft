use super::*;
use craft::behavior::{EditEvent, Mixed};
use craft::keymap::Mods;
use craft::ui::contract::ReorderMove;
use std::sync::Arc;

fn group(name: &str) -> PresetKey {
    PresetKey::Group(name.into())
}

fn item(key: &str) -> PresetKey {
    PresetKey::Item(key.into())
}

fn reorder(key: PresetKey, to: usize) -> PresetEvent<PresetKey> {
    let mv = ReorderMove::new(key.clone(), vec![key], to, false);
    PresetEvent::Reorder(EditEvent::set(Mixed::Same(mv.clone()), mv, Arc::from("Reorder"), None))
}

fn run(command: &str, params: Value) -> Option<Act> {
    Some(Act::Run(command.into(), params))
}

/// Every browser event of the shared mapping, for each engine prefix that takes `{prefix}.edit` and `{prefix}.new`:
/// the command and params it runs, or the folder move it makes.
#[test]
fn each_preset_event_runs_its_edit_command_for_every_prefix() {
    // Folder A holds a1 and a2, folder B holds b1.
    let rows = [group("A"), item("a1"), item("a2"), group("B"), item("b1")];
    for prefix in ["gradient.presets", "pattern.presets", "style.presets", "shape.presets", "tool.presets"] {
        let panel = PresetPanel { id: "panel", label: "Panel", prefix, new_params: || json!({"name": "New"}), rename_new: true };
        let edit = format!("{prefix}.edit");
        let cases: Vec<(PresetEvent<PresetKey>, Option<Act>)> = vec![
            (PresetEvent::rename(item("a1"), "b"), run(&edit, json!({"action": "rename", "preset": "a1", "name": "b"}))),
            (PresetEvent::rename(group("A"), "C"), run(&edit, json!({"action": "renameGroup", "group": "A", "name": "C"}))),
            (PresetEvent::Delete(item("a1")), run(&edit, json!({"action": "delete", "preset": "a1"}))),
            (PresetEvent::Delete(group("A")), run(&edit, json!({"action": "deleteGroup", "group": "A"}))),
            (PresetEvent::move_to(item("a1"), group("B")), run(&edit, json!({"action": "move", "preset": "a1", "to": "B"}))),
            (PresetEvent::Footer(PresetFooterEvent::NewGroup), run(&edit, json!({"action": "newGroup", "name": "Group"}))),
            (PresetEvent::Footer(PresetFooterEvent::New), run(&format!("{prefix}.new"), json!({"name": "New"}))),
            (PresetEvent::Footer(PresetFooterEvent::Delete), run(&edit, json!({"action": "delete", "preset": "a2"}))),
            // a1 lands after b1: the end of B.
            (reorder(item("a1"), 4), run(&edit, json!({"action": "move", "preset": "a1", "to": "B", "index": 1}))),
            // a2 lands first in A.
            (reorder(item("a2"), 1), run(&edit, json!({"action": "move", "preset": "a2", "to": "A", "index": 0}))),
            // B goes before A; A after B is the last place.
            (reorder(group("B"), 0), Some(Act::MoveFolder { from: "B".into(), before: Some("A".into()) })),
            (reorder(group("A"), 2), Some(Act::MoveFolder { from: "A".into(), before: None })),
            (PresetEvent::Action(0), None),
        ];
        for (event, expected) in cases {
            assert_eq!(act(panel, &event, &rows, Some("a2")), expected, "{prefix}: {event:?}");
        }
    }
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

/// The events that use a preset: Patterns select on a click and fill on a double click or a drop; Styles apply,
/// Shift adding the effects, and a drop styles the active layer; Gradients select and apply; Shapes pick, place and
/// place at a drop; Tool Presets apply.
#[test]
fn each_panel_uses_a_preset_its_own_way() {
    let mut app = app();
    crate::canvas::testing::frame(&mut app);
    let shift = Mods { shift: true, ..Mods::default() };
    let none = Mods::default();
    let at = pos2(10.0, 20.0);
    let cases: Vec<(PresetPanel, PresetEvent<PresetKey>, Option<Act>)> = vec![
        (PATTERNS, PresetEvent::select(item("p"), none), run("pattern.presets.select", json!({"pattern": "p"}))),
        (PATTERNS, PresetEvent::activate(item("p"), none), run("pattern.presets.apply", json!({"pattern": "p"}))),
        (PATTERNS, PresetEvent::drop_at(item("p"), at, none), run("pattern.presets.apply", json!({"pattern": "p"}))),
        (STYLES, PresetEvent::select(item("s"), none), run("style.presets.apply", json!({"preset": "s", "add": false}))),
        (STYLES, PresetEvent::select(item("s"), shift), run("style.presets.apply", json!({"preset": "s", "add": true}))),
        (STYLES, PresetEvent::drop_at(item("s"), at, shift), run("style.presets.apply", json!({"preset": "s"}))),
        (STYLES, PresetEvent::select(group("G"), none), None),
        (GRADIENTS, PresetEvent::select(item("g"), none), run("gradient.presets.select", json!({"preset": "g"}))),
        (GRADIENTS, PresetEvent::activate(item("g"), none), run("gradient.presets.apply", json!({"preset": "g"}))),
        (GRADIENTS, PresetEvent::drop_at(item("g"), at, none), run("gradient.presets.apply", json!({"preset": "g"}))),
        (SHAPES, PresetEvent::select(item("Star"), none), Some(Act::CustomShapeTool)),
        (SHAPES, PresetEvent::activate(item("Star"), none), run("shape.presets.place", json!({"preset": "Star", "fill": shape_fill(&app)}))),
        (TOOL_PRESETS, PresetEvent::select(item("T"), none), Some(Act::ToolPreset("T".into()))),
        (TOOL_PRESETS, PresetEvent::activate(item("T"), none), Some(Act::ToolPreset("T".into()))),
        (TOOL_PRESETS, PresetEvent::drop_at(item("T"), at, none), None),
    ];
    for (panel, event, expected) in cases {
        assert_eq!(panel_act(&app, panel, &event), expected, "{}: {event:?}", panel.id);
    }
    // A shape dropped on the canvas lands in a square centred on the drop, of 30 % of the shorter side.
    let drop = canvas_drop_rect(&app).center();
    let c = crate::canvas::document_display::doc_point_at(&app, drop).expect("the drop is on the canvas");
    let side = (48.0_f64 * 0.3).max(8.0);
    let expected = json!({"preset": "Star", "fill": shape_fill(&app), "rect": [c.x - side / 2.0, c.y - side / 2.0, side, side]});
    assert_eq!(panel_act(&app, SHAPES, &PresetEvent::drop_at(item("Star"), drop, none)), run("shape.presets.place", expected));
}

/// Regression: preset drops mapped the pointer through the raw widget rect, ignoring the rulers' inset — a dropped
/// shape landed off the pointer, and a drop on the ruler strip counted as a canvas drop. Drops map through, and
/// target, the same content rect the canvas draws with.
#[test]
fn preset_drops_map_through_the_ruler_inset() {
    let mut app = app();
    app.ui.extras.rulers = true;
    crate::canvas::testing::frame(&mut app);
    let rect = canvas_drop_rect(&app);
    let centre = rect.center();
    assert!(rect.contains(centre));
    assert!(!rect.contains(pos2(rect.left() - 4.0, centre.y)), "the ruler strip is no drop target");
    let doc = crate::canvas::document_display::doc_point_at(&app, centre).expect("the canvas centre is on the document");
    assert!((doc.x - 32.0).abs() < 1.0 && (doc.y - 24.0).abs() < 1.0, "the canvas centre is the document centre: {doc:?}");
}
