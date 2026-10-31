//! The Layers panel rows mirror in a right-to-left UI: the eye column and the indicators swap
//! ends, the name hangs from the reading start, and a drag stays physical.
//!
//! The twin switches the language through the real preference path, so only the direction
//! changes: the same document, the same rows and the same data in both directions.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, control, dock::Group, layer_row_ui, mask_thumbs_ui};
use serde_json::json;

/// A document with a locked layer, a plain one and a group holding a child.
fn session() -> (photocraft_engine::Session, Vec<u64>) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let locked = s.execute("layer.new.layer", json!({"name": "Locked"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": locked, "locks": {"all": true}})).unwrap();
    let plain = s.execute("layer.new.layer", json!({"name": "Plain"})).unwrap()["layer"].as_u64().unwrap();
    let child = s.execute("layer.new.layer", json!({"name": "Child"})).unwrap()["layer"].as_u64().unwrap();
    let group = s.execute("layer.groupLayers", json!({"layer": child, "name": "Group"})).unwrap()["layer"].as_u64().unwrap();
    // Group Layers creates the group collapsed; open it so the panel lists its child.
    s.execute("layer.setExpanded", json!({"layer": group, "expanded": true})).unwrap();
    (s, vec![locked, plain, child, group])
}

/// The app with the Layers panel alone in the dock, so its rows have room.
fn harness(session: photocraft_engine::Session) -> Harness<'static, PhotocraftApp> {
    // 60 fps steps, so two clicks a frame apart count as a double click.
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, Default::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = control::ControlRequest::new("ui.set", json!({"theme": "promedium", "dockWidth": 290.0}));
    control::handle(h.state_mut(), &ctx, &req);
    // Every other dock group collapses to its tab strip, so the Layers rows have the height.
    for group in [Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History, Group::Brushes] {
        h.state_mut().ui.dock.set_collapsed(group, true);
    }
    h.run_steps(8);
    h
}

/// The app with its interface language set to `code`.
fn in_language(h: &mut Harness<'static, PhotocraftApp>, code: &str) {
    h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("the language is registered");
    h.run_steps(4);
}

/// Where a layer's row was drawn.
fn row_of(h: &Harness<'_, PhotocraftApp>, id: u64) -> layer_row_ui::RowRects {
    layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id).unwrap_or_else(|| panic!("layer {id} drawn"))
}

/// The rect of the lock icon on a row.
fn lock_of(r: &layer_row_ui::RowRects) -> egui::Rect {
    r.indicators.iter().find(|(k, _)| *k == layer_row_ui::Indicator::Lock).unwrap_or_else(|| panic!("the lock icon: {:?}", r.indicators)).1
}

/// In an Arabic UI the row's indicators pack from the left and its name hangs from the right;
/// switching back to English puts them where they were.
#[test]
fn an_arabic_layer_row_swaps_its_ends() {
    let (s, ids) = session();
    let locked = ids[0];
    let mut h = harness(s);
    in_language(&mut h, "ar");
    let r = row_of(&h, locked);
    let lock = lock_of(&r);
    let name = r.name.expect("the name is drawn");
    assert!(lock.left() < r.row.center().x, "the indicators pack from the left: {lock:?} in {:?}", r.row);
    assert!((lock.left() - (r.row.left() + layer_row_ui::RIGHT_PAD)).abs() < 0.01, "the first indicator clears the scrollbar side: {lock:?}");
    assert!(name.right() > r.row.center().x, "the name hangs from the reading start: {name:?} in {:?}", r.row);
    assert!(name.right() <= r.row.right(), "the name stays in its row: {name:?}");

    in_language(&mut h, "en");
    let r = row_of(&h, locked);
    let lock = lock_of(&r);
    let name = r.name.expect("the name is drawn");
    assert!(lock.right() > r.row.center().x, "English keeps the indicators on the right: {lock:?} in {:?}", r.row);
    assert!(name.left() < r.row.center().x, "English keeps the name on the left: {name:?} in {:?}", r.row);
}

/// A layer inside a group indents towards the reading end: in an Arabic UI that is the left.
#[test]
fn an_arabic_layers_child_indents_towards_the_left() {
    let (s, ids) = session();
    let (plain, child) = (ids[1], ids[2]);
    let mut h = harness(s);
    in_language(&mut h, "ar");
    let plain = row_of(&h, plain).name.expect("the plain layer's name");
    let child = row_of(&h, child).name.expect("the child's name");
    assert!(child.right() < plain.right(), "the child sits left of its top-level neighbour: {child:?} vs {plain:?}");
}

/// The eye column moves to the row's right in an Arabic UI: the eye is there, and clicking it
/// toggles the layer.
#[test]
fn an_arabic_layers_eye_column_sits_on_the_right() {
    let (s, ids) = session();
    let plain = ids[1];
    let mut h = harness(s);
    in_language(&mut h, "ar");
    let r = row_of(&h, plain);
    click(&mut h, pos2(r.row.right() - 15.0, r.row.center().y));
    assert!(!visible(&h, plain), "the eye in the right-hand column toggles the layer");

    in_language(&mut h, "en");
    let r = row_of(&h, plain);
    click(&mut h, pos2(r.row.left() + 15.0, r.row.center().y));
    assert!(visible(&h, plain), "English keeps the eye in the left-hand column: it toggles back");
}

/// Is the layer visible?
fn visible(h: &Harness<'_, PhotocraftApp>, id: u64) -> bool {
    let doc = &h.state().session.active().expect("the document").doc;
    doc.layers.iter().find(|l| l.id.0 == id).expect("the layer").visible
}

/// A click at `at`.
fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// A double click at `at`: two clicks, a frame apart.
fn double_click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..2 {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
}

/// The rename field opens on the mirrored name and runs from the reading start.
#[test]
fn an_arabic_rename_field_hangs_from_the_reading_start() {
    let (s, ids) = session();
    let plain = ids[1];
    let mut h = harness(s);
    in_language(&mut h, "ar");
    let name = row_of(&h, plain).name.expect("the name is drawn");
    double_click(&mut h, name.center());
    assert_eq!(layer_row_ui::renaming(&h.ctx), Some(plain), "the double click starts renaming");
    let field = h.ctx.read_response(egui::Id::unique(("layer-rename-field", plain))).expect("the rename field").rect;
    assert!((field.right() - (name.right() + 3.0)).abs() < 0.5, "the field starts at the reading start: {field:?} vs {name:?}");
    assert!(field.left() <= name.left(), "the field covers the name: {field:?} vs {name:?}");
}

/// Reordering a row is pointer-driven: it stays physical in an Arabic UI.
#[test]
fn an_arabic_drag_reorders_the_layers() {
    let (s, ids) = session();
    let (locked, plain) = (ids[0], ids[1]);
    let mut h = harness(s);
    in_language(&mut h, "ar");
    // The rows read top to bottom: Group, Plain, Locked, Background. Dragging Locked onto the
    // upper quarter of Plain drops it above Plain, in the stack too.
    let from = row_of(&h, locked).row.center();
    let plain_row = row_of(&h, plain).row;
    let to = pos2(plain_row.center().x, plain_row.top() + plain_row.height() * 0.25);
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for k in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some("Reorder Layers"));
    let names: Vec<String> = h.state().session.active().unwrap().doc.layers.iter().map(|l| l.name.clone()).collect();
    let at = |name: &str| names.iter().position(|n| n == name).unwrap();
    assert_eq!(at("Locked"), at("Plain") + 1, "the row landed above the other one: {names:?}");
}

/// A mask thumbnail follows its layer thumbnail in reading order: in an Arabic UI that is to
/// the left.
#[test]
fn an_arabic_mask_thumb_sits_left_of_the_layer_thumbnail() {
    let (mut s, ids) = session();
    let plain = ids[1];
    // The layer's alpha becomes its layer mask, so the row draws a mask thumbnail.
    s.execute("layer.layerMask.fromTransparency", json!({"layer": plain})).unwrap();
    let mut h = harness(s);
    in_language(&mut h, "ar");
    let thumb = row_of(&h, plain).thumbnail.expect("the layer thumbnail");
    let (thumbs, _chains) = mask_thumbs_ui::recorded(&h.ctx, plain).expect("the mask thumbnail");
    let mask = thumbs.first().expect("a mask").1;
    assert!(mask.right() <= thumb.left(), "the mask thumbnail sits left of the layer thumbnail: {mask:?} vs {thumb:?}");
}

/// A closed group's disclosure triangle points towards the reading end: left in an Arabic UI,
/// while an open one still points down.
#[test]
fn an_arabic_closed_group_points_left() {
    let (mut s, ids) = session();
    let group = ids[3];
    s.execute("layer.setExpanded", json!({"layer": group, "expanded": false})).unwrap();
    let mut h = harness(s);
    in_language(&mut h, "ar");
    let row = row_of(&h, group).row;
    let triangle = triangle_in(&h, row).expect("the disclosure triangle");
    let points = sorted_by_y(triangle);
    // Closed: the apex is the middle point, and it is the leftmost one.
    let apex = points[1];
    assert!(apex.x <= points.iter().map(|p| p.x).fold(f32::INFINITY, f32::min) + 0.01, "the closed group points left: {points:?}");

    h.state_mut().run("layer.setExpanded", json!({"layer": group, "expanded": true})).expect("the group opens");
    h.run_steps(4);
    let row = row_of(&h, group).row;
    let points = sorted_by_y(triangle_in(&h, row).expect("the disclosure triangle"));
    // Open: the apex is the bottom point, between the two top ones.
    let bottom = points[2];
    let (top1, top2) = (points[0], points[1]);
    assert!(bottom.y > top1.y + 0.5, "the open group points down: {points:?}");
    assert!(bottom.x > top1.x.min(top2.x) && bottom.x < top1.x.max(top2.x), "the open group points down: {points:?}");
}

/// The triangle's points, top to bottom.
fn sorted_by_y(mut points: Vec<Pos2>) -> Vec<Pos2> {
    points.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
    points
}

/// The three points of the triangle painted in `row`, if any.
fn triangle_in(h: &Harness<'_, PhotocraftApp>, row: egui::Rect) -> Option<Vec<Pos2>> {
    let mut found = None;
    for clipped in &h.output().shapes {
        walk(&clipped.shape, row, &mut found);
    }
    found
}

/// The three-point path in `row`, if there is one.
fn walk(shape: &egui::Shape, row: egui::Rect, found: &mut Option<Vec<Pos2>>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, row, found)),
        egui::Shape::Path(p) if p.closed && p.points.len() == 3 && p.points.iter().all(|q| row.contains(*q)) => {
            *found = Some(p.points.clone());
        }
        _ => {}
    }
}
