//! TOOL-214-12: a live rectangle's corner radius widgets, as in Photoshop.

use egui::Modifiers;
use photocraft_doc::LayerContent;
use photocraft_doc::vector::LiveShape;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// A 200 × 200 document at 100 % with a live rectangle at (40, 40), 120 × 80, radii 10.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "roundedRect", "rect": [40, 40, 120, 80], "radii": 10})).unwrap();
    app.ui.tool = tool;
    app
}

fn radii(app: &PhotocraftApp) -> Option<[f64; 4]> {
    let st = app.session.active()?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id))?;
    match &l.content {
        LayerContent::Shape(sh) => match sh.live {
            Some(LiveShape::Rect { radii, .. }) => Some(radii),
            _ => None,
        },
        _ => None,
    }
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.entries().len()
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
}

#[test]
fn dragging_a_widget_rounds_every_corner_in_one_step() {
    let mut app = app(Tool::PathSelection);
    let n = steps(&app);
    // The top-left widget sits at the arc centre (50, 50); dragging it 15 px along the diagonal
    // adds 15 to the radius.
    drag(&mut app, [50.0, 50.0], [65.0, 65.0], Modifiers::NONE);
    assert_eq!(radii(&app), Some([25.0; 4]));
    assert_eq!(steps(&app), n + 1, "one history step");
    assert!(app.ui.corner_radius.is_none());
    assert!(app.session.undo());
    assert_eq!(radii(&app), Some([10.0; 4]));
}

#[test]
fn alt_drags_one_corner_and_radii_stay_within_half_the_short_side() {
    let mut app = app(Tool::Rectangle);
    // The bottom-right widget, with ⌥: only that corner, capped at 40 (half of the height 80).
    drag(&mut app, [150.0, 110.0], [20.0, 20.0], Modifiers::ALT);
    assert_eq!(radii(&app), Some([10.0, 10.0, 40.0, 10.0]));
    // Dragging out past the corner makes it sharp.
    drag(&mut app, [150.0, 50.0], [190.0, 10.0], Modifiers::ALT);
    assert_eq!(radii(&app), Some([10.0, 0.0, 40.0, 10.0]));
    // The Rectangle tool drew no new shape.
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn a_sharp_corner_keeps_a_grabbable_widget_and_a_click_changes_nothing() {
    let mut app = app(Tool::PathSelection);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("shape.edit", json!({"layer": id, "radii": 0})).unwrap();
    let n = steps(&app);
    // At 100 % the widget of a sharp corner is 10 px inside it.
    drag(&mut app, [50.0, 50.0], [50.0, 50.0], Modifiers::NONE);
    assert_eq!((radii(&app), steps(&app)), (Some([0.0; 4]), n), "a click commits nothing");
    drag(&mut app, [50.0, 50.0], [60.0, 60.0], Modifiers::NONE);
    assert_eq!(radii(&app), Some([10.0; 4]));
}

#[test]
fn only_live_rectangles_with_the_right_tools_take_the_press() {
    for tool in [Tool::Move, Tool::DirectSelection, Tool::Brush] {
        let mut a = app(tool);
        assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE), "{tool:?}");
    }
    // ⌘/Ctrl leaves the press to the tool.
    let mut a = app(Tool::PathSelection);
    assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::COMMAND));
    // An ellipse has no corners.
    let mut a = app_with(json!({"kind": "ellipse", "rect": [40, 40, 120, 80]}));
    assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    // A press off the widgets is the tool's.
    let mut a = app(Tool::PathSelection);
    assert!(!super::pointer(&mut a, ToolEvent::Down { x: 100.0, y: 80.0, pressure: 1.0 }, Modifiers::NONE));
    // Shape drawing's Path/Pixels mode does not disable the main selection tools.
    for tool in [Tool::PathSelection, Tool::Move] {
        let mut a = app(tool);
        a.ui.tool_options.shape.mode = "pixels".into();
        if tool == Tool::Move {
            a.ui.tool_options.move_show_transform = true;
        }
        assert!(super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE), "{tool:?}");
        super::pointer(&mut a, ToolEvent::Up { x: 50.0, y: 50.0 }, Modifiers::NONE);
    }
    for lock in ["all", "pixels", "position"] {
        let mut a = app(Tool::PathSelection);
        let id = a.session.active().unwrap().active_layer.unwrap().0;
        a.run("layer.setProps", json!({"layer":id,"locks":{lock:true}})).unwrap();
        assert!(!super::pointer(&mut a, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE), "{lock}");
    }
}

fn app_with(shape: serde_json::Value) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.run("shape.create", shape).unwrap();
    app.ui.tool = Tool::PathSelection;
    app
}

/// Three live rectangles: (20, 20) 60 × 40 radii 5, (110, 20) 70 × 50 radii 0, and (20, 120)
/// 80 × 60 radii 0, the last one left unselected. Returns the ids, bottom first.
fn three(tool: Tool) -> (PhotocraftApp, Vec<photocraft_doc::LayerId>) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    let mut ids = Vec::new();
    for (rect, r) in [([20, 20, 60, 40], 5), ([110, 20, 70, 50], 0), ([20, 120, 80, 60], 0)] {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "radii": r})).unwrap();
        ids.push(app.session.active().unwrap().active_layer.unwrap());
    }
    app.run("layer.select", json!({"layer": ids[0].0})).unwrap();
    app.run("layer.select", json!({"layer": ids[1].0, "mode": "add"})).unwrap();
    app.ui.tool = tool;
    (app, ids)
}

fn radii_of(app: &PhotocraftApp, id: photocraft_doc::LayerId) -> [f64; 4] {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => match sh.live {
            Some(LiveShape::Rect { radii, .. }) => radii,
            _ => panic!("live rect"),
        },
        _ => panic!("shape"),
    }
}

#[test]
fn one_widget_rounds_every_selected_rectangle_alike_in_one_step() {
    let (mut app, ids) = three(Tool::PathSelection);
    let n = steps(&app);
    // The second rectangle's top-left widget (sharp: 10 px inside, at (120, 30)), dragged 15 px
    // in each direction: radius 15 on every corner of both selected rectangles.
    drag(&mut app, [120.0, 30.0], [135.0, 45.0], Modifiers::NONE);
    assert_eq!(radii_of(&app, ids[0]), [15.0; 4]);
    assert_eq!(radii_of(&app, ids[1]), [15.0; 4]);
    assert_eq!(radii_of(&app, ids[2]), [0.0; 4], "the unselected one is untouched");
    assert_eq!(steps(&app), n + 1, "one history step for both layers");
    assert!(app.session.undo());
    assert_eq!((radii_of(&app, ids[0]), radii_of(&app, ids[1])), ([5.0; 4], [0.0; 4]));
}

#[test]
fn escape_cancels_a_radius_preview_without_discarding_the_existing_redo_branch() {
    let mut app = app(Tool::PathSelection);
    let id = app.session.active().unwrap().active_layer.unwrap().0;
    app.run("layer.setProps", json!({"layer":id,"name":"Renamed"})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().history.can_redo());
    let before = app.session.active().unwrap().doc.clone();

    assert!(super::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    super::pointer(&mut app, ToolEvent::Move { x: 58.0, y: 58.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(app.ui.corner_radius.is_some());
    assert_eq!(app.session.active().unwrap().doc, before, "drag changes preview only");

    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
            ..Default::default()
        },
        |ui| crate::shortcuts::handle(&mut app, ui.ctx()),
    );
    output.textures_delta.clear();

    assert!(app.ui.corner_radius.is_none(), "Escape ends the preview");
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(app.session.active().unwrap().history.can_redo(), "Escape keeps the pre-existing redo branch");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().name, "Renamed");
}

#[test]
fn a_tool_change_ends_a_radius_gesture_without_committing_its_preview() {
    let mut app = app(Tool::PathSelection);
    let before_radii = radii(&app);
    let history = app.session.active().unwrap().history.past_len();
    assert!(super::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    super::pointer(&mut app, ToolEvent::Move { x: 58.0, y: 58.0, pressure: 1.0 }, Modifiers::NONE);

    app.ui.tool = Tool::Move;
    super::pointer(&mut app, ToolEvent::Up { x: 58.0, y: 58.0 }, Modifiers::NONE);

    assert!(app.ui.corner_radius.is_none(), "the old gesture is discarded");
    assert_eq!(radii(&app), before_radii, "tool change must discard the preview");
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

#[test]
fn document_and_target_changes_discard_a_radius_preview() {
    let mut first_app = app(Tool::PathSelection);
    assert!(super::pointer(&mut first_app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    super::pointer(&mut first_app, ToolEvent::Move { x: 58.0, y: 58.0, pressure: 1.0 }, Modifiers::NONE);
    first_app.run("file.new", json!({"width":100,"height":100})).unwrap();
    let new_document = first_app.session.active().unwrap().doc.clone();
    super::pointer(&mut first_app, ToolEvent::Up { x: 58.0, y: 58.0 }, Modifiers::NONE);
    assert!(first_app.ui.corner_radius.is_none(), "document change ends the stale gesture");
    assert_eq!(first_app.session.active().unwrap().doc, new_document, "old target is not edited in the new document");

    let mut target_app = app(Tool::PathSelection);
    let first = target_app.session.active().unwrap().active_layer.unwrap();
    target_app.run("shape.create", json!({"kind":"roundedRect","rect":[130,40,50,50],"radii":10})).unwrap();
    let second = target_app.session.active().unwrap().active_layer.unwrap();
    target_app.run("layer.select", json!({"layer":first.0})).unwrap();
    let before_radii = radii(&target_app);
    assert!(super::pointer(&mut target_app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    super::pointer(&mut target_app, ToolEvent::Move { x: 58.0, y: 58.0, pressure: 1.0 }, Modifiers::NONE);
    target_app.run("layer.select", json!({"layer":second.0})).unwrap();
    super::pointer(&mut target_app, ToolEvent::Up { x: 58.0, y: 58.0 }, Modifiers::NONE);
    assert!(target_app.ui.corner_radius.is_none(), "target change ends the stale gesture");
    assert_eq!(radii_of(&target_app, first), before_radii.unwrap());

    let mut mask_app = app(Tool::PathSelection);
    let before_radii = radii(&mask_app);
    assert!(super::pointer(&mut mask_app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
    super::pointer(&mut mask_app, ToolEvent::Move { x: 58.0, y: 58.0, pressure: 1.0 }, Modifiers::NONE);
    mask_app.ui.mask_target = true;
    super::pointer(&mut mask_app, ToolEvent::Up { x: 58.0, y: 58.0 }, Modifiers::NONE);
    assert!(mask_app.ui.corner_radius.is_none(), "mask target change ends the stale gesture");
    assert_eq!(radii(&mask_app), before_radii);
}

#[test]
fn rectangle_path_modes_do_not_start_or_commit_radius_gestures() {
    for mode in ["path", "pixels"] {
        let mut app = app(Tool::Rectangle);
        app.ui.tool_options.shape.mode = mode.into();
        assert!(!super::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE), "{mode} mode belongs to drawing options");

        app.ui.tool_options.shape.mode = "shape".into();
        assert!(super::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE));
        super::pointer(&mut app, ToolEvent::Move { x: 58.0, y: 58.0, pressure: 1.0 }, Modifiers::NONE);
        app.ui.tool_options.shape.mode = mode.into();
        super::pointer(&mut app, ToolEvent::Up { x: 58.0, y: 58.0 }, Modifiers::NONE);
        assert!(app.ui.corner_radius.is_none(), "switch to {mode} ends the shape-tool gesture");
        assert_eq!(radii(&app), Some([10.0; 4]));
    }
}

#[test]
fn alt_changes_that_corner_on_each_and_each_stays_within_its_own_size() {
    let (mut app, ids) = three(Tool::Rectangle);
    // ⌥ on the first rectangle's top-left widget (radius 5, so drawn 10 px inside at (30, 30)),
    // dragged far in: each rectangle's top-left corner only, capped at half of its own short side.
    drag(&mut app, [30.0, 30.0], [100.0, 100.0], Modifiers::ALT);
    assert_eq!(radii_of(&app, ids[0]), [20.0, 5.0, 5.0, 5.0]);
    assert_eq!(radii_of(&app, ids[1]), [25.0, 0.0, 0.0, 0.0]);
}

#[test]
fn the_move_tool_shows_and_drags_the_widgets_with_its_transform_controls() {
    let (mut app, ids) = three(Tool::Move);
    assert!(!super::pointer(&mut app, ToolEvent::Down { x: 120.0, y: 30.0, pressure: 1.0 }, Modifiers::NONE), "not without the controls");
    app.ui.tool_options.move_show_transform = true;
    let before = app.session.active().unwrap().selected_layers();
    drag(&mut app, [120.0, 30.0], [130.0, 40.0], Modifiers::NONE);
    assert_eq!((radii_of(&app, ids[0]), radii_of(&app, ids[1])), ([10.0; 4], [10.0; 4]));
    assert_eq!(app.session.active().unwrap().selected_layers(), before, "the press neither moved nor reselected layers");
}

#[test]
fn the_cursor_and_readout_follow_the_widgets() {
    let mut app = app(Tool::PathSelection);
    // The corner radius cursor shows over a widget, not elsewhere.
    assert!(super::hovers(&app, [50.0, 50.0]));
    assert!(!super::hovers(&app, [100.0, 80.0]));
    // While dragging it stays, and the readout shows the dragged corner's radius.
    tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 62.0, y: 62.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(super::hovers(&app, [100.0, 80.0]));
    assert_eq!(app.ui.corner_radius.as_ref().and_then(super::dragged_radius), Some(22.0));
    tool_event(&mut app, ToolEvent::Up { x: 62.0, y: 62.0 }, Modifiers::NONE);
    assert!(!super::hovers(&app, [100.0, 80.0]));
}

#[test]
fn clicking_an_unlinked_radius_widget_without_motion_preserves_all_radii() {
    let mut app = app_with(json!({"kind":"roundedRect", "rect":[40,40,120,80], "radii":[5,10,15,20]}));
    let before = radii(&app);
    let history = steps(&app);
    super::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    super::pointer(&mut app, ToolEvent::Up { x: 50.0, y: 50.0 }, Modifiers::NONE);
    assert_eq!(radii(&app), before);
    assert_eq!(steps(&app), history);

    let mut app = app_with(json!({"kind":"roundedRect", "rect":[40,40,120,80], "radii":[5,10,15,20]}));
    let before = radii(&app);
    let history = steps(&app);
    super::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    super::pointer(&mut app, ToolEvent::Move { x: 60.0, y: 60.0, pressure: 1.0 }, Modifiers::NONE);
    super::pointer(&mut app, ToolEvent::Move { x: 50.0, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    super::pointer(&mut app, ToolEvent::Up { x: 50.0, y: 50.0 }, Modifiers::NONE);
    assert_eq!(radii(&app), before, "returning to the press point restores the unlinked radii");
    assert_eq!(steps(&app), history);
}

#[test]
fn widget_hit_distance_tracks_corrected_pixel_aspect() {
    let mut app = app(Tool::PathSelection);
    app.ui.view.pixel_aspect = "custom:0.5:test".into();
    assert!(super::hit(&app, &super::targets(&app), [58.0, 50.0]).is_some(), "4 screen points from the grip at 0.5 aspect");
    app.ui.view.pixel_aspect = "custom:2:test".into();
    assert!(super::hit(&app, &super::targets(&app), [54.0, 50.0]).is_none(), "8 screen points from the grip at 2.0 aspect");
}

fn canvas_frame_for_radius(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0));
    app.last_canvas_rect = rect;
    let view = app.ui.views[0].clone();
    let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(rect), events, ..Default::default() }, |ui| {
        app.ui.views[0] = crate::canvas::canvas_view(app, ui, 0, rect, view.clone(), true);
    });
    out.textures_delta.clear();
    out.shapes
}

fn canvas_radius_app() -> (PhotocraftApp, egui::Context) {
    let mut app = app(Tool::Rectangle);
    app.ui.views[0].zoom = 1.0;
    app.ui.views[0].center = [100.0, 80.0];
    app.ui.views[0].fit_pending = false;
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
    canvas_frame_for_radius(&mut app, &ctx, vec![]);
    (app, ctx)
}

fn space_key(pressed: bool) -> egui::Event {
    egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE }
}

fn primary_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn native_space_navigation_cancels_radius_preview_before_release() {
    let (mut app, ctx) = canvas_radius_app();
    let before = app.session.active().unwrap().doc.clone();
    let history = steps(&app);
    let xf = crate::canvas::ViewXform::active(&app).unwrap();
    let t = super::targets(&app)[0];
    let doc = super::widget(t.rect, t.radii, 0, f64::from(app.point_zoom()));
    let screen = xf.to_screen(doc[0] as f32, doc[1] as f32);
    canvas_frame_for_radius(&mut app, &ctx, vec![egui::Event::PointerMoved(screen), primary_button(screen, true)]);
    assert!(app.ui.corner_radius.is_some(), "radius drag starts on the widget");
    canvas_frame_for_radius(&mut app, &ctx, vec![egui::Event::PointerMoved(screen + egui::vec2(2.0, 2.0))]);
    assert!(app.ui.corner_radius.is_some());
    canvas_frame_for_radius(&mut app, &ctx, vec![space_key(true)]);
    assert!(app.ui.corner_radius.is_none(), "temporary Hand navigation cancels the preview");
    let moved = screen + egui::vec2(30.0, 25.0);
    canvas_frame_for_radius(&mut app, &ctx, vec![egui::Event::PointerMoved(moved), primary_button(moved, false)]);
    canvas_frame_for_radius(&mut app, &ctx, vec![space_key(false)]);
    assert_eq!(app.session.active().unwrap().doc, before);
    assert_eq!(steps(&app), history);
}

#[test]
fn canvas_draws_one_visible_custom_colored_radius_grip_set() {
    let (mut app, ctx) = canvas_radius_app();
    let color = egui::Color32::from_rgb(230, 40, 70);
    app.ui.tool_options.shape.path_color = Some([230, 40, 70]);
    let xf = crate::canvas::ViewXform::active(&app).unwrap();
    let center = xf.to_screen(50.0, 50.0);
    let circles_at_center = |shapes: &[egui::epaint::ClippedShape]| {
        shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Circle(c) if c.center.distance(center) < 0.01 => Some(c),
                _ => None,
            })
            .cloned()
            .collect::<Vec<_>>()
    };
    let shapes = canvas_frame_for_radius(&mut app, &ctx, vec![]);
    let circles = circles_at_center(&shapes);
    assert_eq!(circles.len(), 2, "one grip uses its outer and inner circles");
    assert!(circles.iter().any(|c| c.fill == color), "grip uses the configured path color");

    app.ui.tool_options.shape.show_path = false;
    let shapes = canvas_frame_for_radius(&mut app, &ctx, vec![]);
    assert!(circles_at_center(&shapes).is_empty(), "hidden paths hide the radius grips");
}

#[test]
fn raw_pointer_press_does_not_become_a_radius_hit_after_pixel_rounding() {
    let mut app = app(Tool::Rectangle);
    app.ui.extras.snap = false;
    assert!(app.session.prefs().tools.snap_vector_tools_and_transforms_to_pixel_grid);
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 57.2, y: 50.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(app.ui.corner_radius.is_none(), "raw press is 7.2 points from the grip; snapped x=57 must not start it");
}
