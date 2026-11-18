use egui::Modifiers;

use super::host::{LayerHost, type_text as insert};
use super::*;
use crate::canvas::{ToolEvent, tool_event};
use craft::text::ui::editor::TextEditorHost as _;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

/// A plain click with the Type tool, through the canvas's own input path.
fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

/// A drag from `from` to `to` in one move, with the button released at `to`.
fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, Modifiers::NONE);
}

fn bar_pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
    let _ = crate::testing::run_ui(&ctx, input, |ui| crate::type_tool::options_bar(app, ui));
}

/// A lower Recent Fonts cap trims the existing list in the next pass, without waiting for a
/// pick; 0 clears it.
#[test]
fn lowering_the_recent_fonts_cap_applies_at_once() {
    let mut app = app();
    app.ui.recent_fonts = ["C", "B", "A"].map(String::from).to_vec();
    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 2})).unwrap();
    // One pass of the options bar, whose pickers hold the list.
    bar_pass(&mut app);
    assert_eq!(app.ui.recent_fonts, ["C", "B"], "the smaller cap trims the tail without a pick");
    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 0})).unwrap();
    bar_pass(&mut app);
    assert!(app.ui.recent_fonts.is_empty(), "0 clears the list");
}

fn layer_text(app: &PhotocraftApp) -> String {
    let ed = app.ui.text_edit.as_ref().unwrap();
    current_text(app, LayerId(ed.layer)).unwrap()
}

/// A committed type layer reading `s` at the same spot as the others (stacked variants, #4320).
fn stacked_variant(app: &mut PhotocraftApp, s: &str) -> LayerId {
    // A click on the stack's spot would edit the text under it, so the layer is made where the click would.
    app.ui.text_edit = None;
    let mut p = default_style_params(app);
    p["text"] = json!(s);
    p["orientation"] = json!("horizontal");
    p["x"] = json!(50.0);
    p["y"] = json!(100.0);
    LayerId(app.run("type.create", p).unwrap()["layer"].as_u64().unwrap())
}

fn hide(app: &mut PhotocraftApp, id: LayerId) {
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
}

/// Wraps `id` in a new group and returns the group.
fn group_of(app: &mut PhotocraftApp, id: LayerId) -> LayerId {
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let path = doc.path_of(id).unwrap();
    doc.layer_at(&path[..path.len() - 1]).unwrap().id
}

/// Three type variants at one spot, bottom to top: A, B, C.
fn variants() -> (PhotocraftApp, [LayerId; 3]) {
    let mut app = app();
    let ids = ["Variant A", "Variant B", "Variant C"].map(|s| stacked_variant(&mut app, s));
    (app, ids)
}

const ON_TEXT: (f64, f64) = (60.0, 95.0);

/// #4320: variants hidden through their groups above the shown one are not picked.
#[test]
fn type_click_skips_text_in_hidden_groups_above_the_shown_variant() {
    let (mut app, [a, b, c]) = variants();
    for id in [b, c] {
        let g = group_of(&mut app, id);
        hide(&mut app, g);
    }
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a));
    app.ui.text_edit = None;
    click(&mut app, ON_TEXT.0, ON_TEXT.1, Modifiers::NONE);
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(a.0), "the click edits the shown variant");
}

#[test]
fn type_click_skips_own_hidden_variants_above_and_below() {
    let (mut app, [a, b, c]) = variants();
    hide(&mut app, b);
    hide(&mut app, c);
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a), "hidden variants above");
    let (mut app, [a, b, c]) = variants();
    hide(&mut app, a);
    hide(&mut app, b);
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(c), "hidden variants below");
}

/// Photoshop: a click on existing type edits it; a ⇧-click there starts a new type layer.
#[test]
fn shift_click_on_existing_type_starts_new_text() {
    let mut app = app();
    click(&mut app, 50.0, 100.0, Modifiers::NONE);
    insert(&mut app, "Existing words");
    commit(&mut app);
    let existing = app.session.active().unwrap().doc.layers.last().unwrap().id;
    let layers = app.session.active().unwrap().doc.layers.len();
    let (x, y) = (60.0, 95.0);
    assert_eq!(hit_layer(&mut app, x, y), Some(existing), "the click lands on the existing text");
    click(&mut app, x, y, Modifiers::SHIFT);
    let ed = app.ui.text_edit.clone().expect("⇧-click starts a session");
    assert_ne!(ed.layer, existing.0, "⇧: a new type layer, not the existing one");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
    // Cancel takes the new layer back; committed, its placeholder would sit over the existing text.
    cancel(&mut app);
    click(&mut app, x, y, Modifiers::NONE);
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(existing.0), "a plain click edits the existing text");
}

#[test]
fn click_creates_placeholder_selected_and_typing_replaces_it() {
    let mut app = app();
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 100.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(app.ui.text_edit.is_none(), "a press alone starts nothing");
    tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 100.0 }, Modifiers::NONE);
    let ed = app.ui.text_edit.clone().unwrap();
    assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()));
    insert(&mut app, "Héllo");
    insert(&mut app, " world");
    assert_eq!(layer_text(&app), "Héllo world");
    // Creating and typing share the session key: one step after the initial snapshot.
    let steps = app.session.active().unwrap().history.entries();
    assert_eq!(steps, ["New", "New Type Layer"]);
    commit(&mut app);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.last().unwrap().name, "Héllo world");
    assert!(app.ui.text_edit.is_none());
}

// Independent perimeter oracle for the rectangle used by the endpoint hit tests.
fn closed_path_endpoint_test_point(s: f64) -> (f64, f64) {
    if s < 200.0 {
        (20.0 + s, 20.0)
    } else if s < 300.0 {
        (220.0, 20.0 + s - 200.0)
    } else if s < 500.0 {
        (220.0 - (s - 300.0), 120.0)
    } else {
        (20.0, 120.0 - (s - 500.0))
    }
}

/// On a closed path, a point 2 px past the last caret maps to the last offset and a point 2 px
/// before the first caret maps to offset 0, for every alignment.
#[test]
fn closed_path_forward_hit_after_last_caret() {
    for align in ["left", "center", "right"] {
        let mut app = app();
        app.ui.tool_options.type_align = align.into();
        let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
        app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
        click(&mut app, 20.0, 45.0, Modifiers::NONE);
        insert(&mut app, "ssssssssssss");
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let (l, _, _) = layout(&mut app, id).unwrap();
        // The rectangle runs clockwise; the text origin is perimeter distance 575.
        let (last, _, _) = l.caret(12);
        let s = (575.0 + f64::from(last) + 2.0).rem_euclid(600.0);
        let (x, y) = closed_path_endpoint_test_point(s);
        assert_eq!(offset_at(&mut app, id, x, y), 12, "{align}: after the last caret");
        let (first, _, _) = l.caret(0);
        let s = (575.0 + f64::from(first) - 2.0).rem_euclid(600.0);
        let (x, y) = closed_path_endpoint_test_point(s);
        assert_eq!(offset_at(&mut app, id, x, y), 0, "{align}: before the first caret");
    }
}

#[test]
fn open_path_hits_keep_endpoint_clamping() {
    let mut app = app();
    let path = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (380.0, 100.0)])]);
    app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
    click(&mut app, 200.0, 100.0, Modifiers::NONE);
    insert(&mut app, "ssss");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert_eq!(offset_at(&mut app, id, 198.0, 100.0), 0);
    assert_eq!(offset_at(&mut app, id, 20.0, 100.0), 0);
    assert_eq!(offset_at(&mut app, id, 380.0, 100.0), 4);
}

#[test]
fn closed_path_upstream_caret_rectangle_left_edge_roundtrip() {
    let mut app = app();
    let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
    app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
    click(&mut app, 20.0, 45.0, Modifiers::NONE);
    insert(&mut app, "ssssssssssss");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let (layout, _, _) = layout(&mut app, id).unwrap();
    let shape = text_layer(&app.session.active().unwrap().doc, id).unwrap().shape.clone();
    let photocraft_doc::text::TextShape::Path { start, .. } = shape else {
        assert!(matches!(shape, photocraft_doc::text::TextShape::Path { .. }), "outline click must attach path text");
        return;
    };
    // Independent rectangle perimeter oracle: never use the warp under test to
    // generate expected caret coordinates. Start is 25 px before the top-left seam.
    assert!((f64::from(start) - 575.0).abs() < 0.5);
    let mut tested_after_seam = false;
    for index in [1, 4, 7, 10] {
        let (x, _, _) = layout.caret(index);
        let distance = f64::from(start) + f64::from(x);
        tested_after_seam |= distance > 600.0;
        let s = distance.rem_euclid(600.0);
        let point = if s < 200.0 {
            Point::new(20.0 + s, 20.0)
        } else if s < 300.0 {
            Point::new(220.0, 20.0 + s - 200.0)
        } else if s < 500.0 {
            Point::new(220.0 - (s - 300.0), 120.0)
        } else {
            Point::new(20.0, 120.0 - (s - 500.0))
        };
        // The rectangle oracle already returns document coordinates.
        let doc = point;
        assert_eq!(offset_at(&mut app, id, doc.x, doc.y), index, "caret round trip at {index}");
    }
    assert!(tested_after_seam, "fixture must cross the seam");
}

/// A click in type on a path puts the caret at the glyph there along the curve, not where the
/// straight line would have it.
#[test]
fn caret_hits_type_on_a_path_along_the_curve() {
    let mut app = app();
    app.session
        .edit("path", |doc, _| {
            doc.work_path = Some(photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(300.0, 10.0), (300.0, 195.0)])]));
            Ok(())
        })
        .unwrap();
    click(&mut app, 300.0, 20.0, Modifiers::NONE);
    insert(&mut app, "ABCDEFGH");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert!(matches!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, photocraft_doc::text::TextShape::Path { .. }));
    // The type runs down the path: near its start the caret is at the beginning, far down at the end.
    assert_eq!(offset_at(&mut app, id, 300.0, 18.0), 0);
    // (The straight line would put both of these at the start: they are left of its first glyph.)
    let (mid, far) = (offset_at(&mut app, id, 296.0, 60.0), offset_at(&mut app, id, 296.0, 190.0));
    assert!(0 < mid && mid < far, "{mid} {far}");
}

#[test]
fn a_name_from_text_after_blank_lines_keeps_following_the_text() {
    let mut app = app();
    click(&mut app, 50.0, 100.0, Modifiers::NONE);
    insert(&mut app, "\n\nTitle");
    let id = app.ui.text_edit.as_ref().unwrap().layer;
    commit(&mut app);
    let name = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.last().unwrap().name.clone();
    assert_eq!(name(&app), "Title");
    // Edited later, outside the session that created it (#483).
    app.run("type.edit", json!({"layer": id, "text": "\nSubtitle"})).unwrap();
    assert_eq!(name(&app), "Subtitle");
}

/// Dragging the text frame's corner in a full app frame resizes the paragraph box in one history
/// step, and the edit session survives it.
#[test]
fn dragging_a_box_handle_resizes_the_paragraph_box() {
    let mut app = app();
    drag(&mut app, [10.0, 10.0], [110.0, 60.0]);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let before = box_shape(&app, id).expect("a drag makes paragraph text");
    let steps = app.session.active().unwrap().history.entries().len();
    // Top-left corner: the box keeps its bottom-right corner.
    tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 20.0, y: 25.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 30.0 }, Modifiers::NONE);
    let after = box_shape(&app, id).expect("still paragraph text");
    assert!(after.2 < before.2 && after.3 < before.3, "the box shrank: {before:?} -> {after:?}");
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps, "the whole drag is one history step");
    assert!(app.ui.text_edit.is_some(), "the edit session survives the drag");
}

#[test]
fn empty_new_layer_is_deleted_on_commit() {
    let mut app = app();
    click(&mut app, 10.0, 50.0, Modifiers::NONE);
    let n = app.session.active().unwrap().doc.layers.len();
    insert(&mut app, "");
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.anchor = 0;
    }
    insert(&mut app, "");
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), n - 1);
}

#[test]
fn double_click_from_move_tool_restores_move_tool_on_commit_and_cancel() {
    let (mut app, _) = variants();
    let ctx = app.ctx().clone();
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, &ctx, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    assert_eq!(app.ui.text_edit_return_tool, Some(crate::state::Tool::Move));
    commit(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Move, "commit restores the Move tool");
    assert_eq!(app.ui.text_edit_return_tool, None);

    // Also test cancel
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, &ctx, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    cancel(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Move, "cancel restores the Move tool");
    assert_eq!(app.ui.text_edit_return_tool, None);

    // Switching tools mid-edit commits without overriding the newly chosen tool
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, &ctx, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    assert_eq!(app.ui.text_edit_return_tool, Some(crate::state::Tool::Move));
    app.ui.tool = crate::state::Tool::Brush;
    commit(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Brush, "switching tools keeps the chosen tool");
    assert_eq!(app.ui.text_edit_return_tool, None);

    // Layers-thumbnail edit stays on Type tool (matches Photoshop)
    app.ui.tool = crate::state::Tool::Move;
    edit_active(&mut app, &ctx).unwrap();
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    assert_eq!(app.ui.text_edit_return_tool, None);
    commit(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Type, "Layers-thumbnail edit stays on Type tool");
}

/// On an aligned closed path, text reaches negative text coordinates and every cluster still
/// lands on the path, so a selection of all of it highlights every cluster.
#[test]
fn closed_path_aligned_selection_highlights_every_visible_cluster() {
    for align in ["center", "right"] {
        let mut app = app();
        app.ui.tool_options.type_align = align.into();
        let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
        app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
        click(&mut app, 20.0, 45.0, Modifiers::NONE);
        insert(&mut app, "ssssssssssss");
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let (l, _, _) = layout(&mut app, id).unwrap();
        assert!(l.bounds().unwrap()[0] < 0.0, "{align} must exercise negative text coordinates");
        let text_layer = text_layer(&app.session.active().unwrap().doc, id).unwrap();
        let rendered = photocraft_text::render::layout_deform(&l, text_layer).unwrap();
        let visible = l.clusters.iter().filter(|c| rendered.place_glyph(f64::from(c.x + c.advance / 2.0)) != photocraft_text::warp::Placement::Hidden).count();
        assert_eq!(visible, l.clusters.len(), "{align}: every cluster is visible on the path");
        let quads = LayerHost::new(&mut app, id).selection_quads(0..l.clusters.len()).len();
        assert_eq!(quads, visible, "{align}: the selection highlights every visible cluster");
    }
}
