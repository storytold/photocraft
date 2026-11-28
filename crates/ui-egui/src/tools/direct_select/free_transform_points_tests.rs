//! #13209: ⌘T / Edit ▸ Free Transform with anchors selected in Direct Selection frames the box over
//! those anchors and transforms only them (Free Transform Points). Measured on the epic's document:
//! the anchors (300,300) and (100,500) of the closed square selected, the box spans 100,300 to
//! 300,500; W 50 % (linked) and Enter moves them to (250,350) and (150,450) about the centre of the
//! selection, the two unselected anchors of the square stay, and History holds one step "Transform
//! Points". With nothing selected, and in the neighbouring cases the ticket leaves unmeasured, the
//! whole path transforms as it does today.

use craft::behavior::geometry::quad::BoxState;
use craft::behavior::kurbo::{Point, Vec2};
use egui::Modifiers;
use photocraft_doc::vector::Path;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::Tool;

/// The epic's document: 800 x 600 on white, a layer "Red", and saved path Path 1 holding the closed
/// corner square (100,300) (300,300) (300,500) (100,500) and the open line (400,300)-(700,300),
/// targeted in the Paths panel, Direct Selection on.
fn saved_path_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("layer.new.layer", json!({"name": "Red"})).unwrap();
    let path = json!({"subpaths": [
        {"closed": true, "knots": [[100, 300], [300, 300], [300, 500], [100, 500]]},
        {"closed": false, "knots": [[400, 300], [700, 300]]},
    ]});
    app.run("path.set", json!({"name": "Path 1", "path": path})).unwrap();
    app.ui.selected_path = Some("Path 1".into());
    app.ui.tool = Tool::DirectSelection;
    app
}

/// A square work path whose knot 1 at (120,40) is smooth, with handles at (100,40) and (140,40).
fn work_path_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::DirectSelection;
    let path = json!({"subpaths": [{"closed": true, "knots": [
        [40, 40],
        {"anchor": [120, 40], "in": [100, 40], "out": [140, 40], "smooth": true},
        [120, 120],
        [40, 120],
    ]}]});
    app.run("path.set", json!({"name": "work", "path": path})).unwrap();
    app
}

/// A live rectangle shape layer "Rectangle 1" at 550,50 to 750,200, Direct Selection on.
fn shape_app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    app.sync_views();
    app.run("shape.create", json!({"kind": "rect", "rect": [550, 50, 200, 150]})).unwrap();
    app.ui.tool = Tool::DirectSelection;
    app
}

fn click(app: &mut PhotocraftApp, at: [f64; 2], mods: Modifiers) {
    testing::click(app, at, mods);
}

/// The measured two-anchor selection: (300,300) and (100,500) of the square.
fn select_two_corners(app: &mut PhotocraftApp) {
    click(app, [300.0, 300.0], Modifiers::NONE);
    click(app, [100.0, 500.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1], [0, 3]], "both corners are selected");
}

/// ⌘T / Edit ▸ Free Transform.
fn open_box(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "edit.freeTransform", json!({})).expect("the path is transformable");
    assert!(app.ui.transform.is_some(), "the box opened");
}

/// The box's source frame.
fn box_rect(app: &PhotocraftApp) -> [f64; 4] {
    app.ui.transform.as_ref().expect("a Free Transform is open").rect_array()
}

/// What the box will send to `path.transform`.
fn box_params(app: &PhotocraftApp) -> serde_json::Value {
    app.ui.transform.as_ref().expect("a Free Transform is open").path.clone().unwrap_or(serde_json::Value::Null)
}

/// Puts the box on `quad`, as the transform controls do.
fn set_box(app: &mut PhotocraftApp, quad: [[f64; 2]; 4]) {
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = BoxState::new(quad.map(|[x, y]| Point::new(x, y)), b.pivot));
}

/// Moves the box and its reference point, as a drag inside the box does.
fn move_box(app: &mut PhotocraftApp, dx: f64, dy: f64) {
    app.ui.transform.as_mut().unwrap().edit_box(|b| *b = b.moved(Vec2::new(dx, dy)));
}

/// OK: commit the open box.
fn commit_box(app: &mut PhotocraftApp) {
    crate::transform_tool::commit(app);
    assert!(app.ui.transform.is_none(), "the box closed: {}", app.ui.status);
}

fn path1(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.paths.iter().find(|p| p.name == "Path 1").map(|p| p.path.clone()).expect("Path 1 is there")
}

fn work_path(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().expect("the work path is there")
}

fn anchors(path: &Path, sub: usize) -> Vec<[f64; 2]> {
    path.subpaths[sub].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

fn knot(path: &Path, sub: usize, knot: usize) -> (photocraft_geom::Point, photocraft_geom::Point, photocraft_geom::Point) {
    let k = &path.subpaths[sub].knots[knot];
    (k.anchor, k.in_ctrl, k.out_ctrl)
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn last_step(app: &PhotocraftApp) -> Option<String> {
    app.session.active().unwrap().history.undo_label().map(str::to_string)
}

/// The measured row: the box frames the two selected anchors, not all of Path 1.
#[test]
fn the_box_frames_the_selected_anchors() {
    let mut app = saved_path_app();
    select_two_corners(&mut app);
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 300.0, 500.0], "the box spans 100,300 to 300,500");
    assert_eq!(box_params(&app), json!({"name": "Path 1", "anchors": [[0, 1], [0, 3]]}), "the box names the selected anchors");
}

/// The measured row: W 50 % (linked) and Enter moves the selected anchors about the centre of the
/// selection; the unselected anchors of the square and the other subpath stay where they are.
#[test]
fn a_half_wide_box_moves_only_the_selected_anchors() {
    let mut app = saved_path_app();
    select_two_corners(&mut app);
    open_box(&mut app);
    // Half the width and half the height about the centre (200,400) of the selected anchors.
    set_box(&mut app, [[150.0, 350.0], [250.0, 350.0], [250.0, 450.0], [150.0, 450.0]]);
    commit_box(&mut app);
    let p = path1(&app);
    assert_eq!(anchors(&p, 0), vec![[100.0, 300.0], [250.0, 350.0], [300.0, 500.0], [150.0, 450.0]], "only the selected corners moved");
    assert_eq!(anchors(&p, 1), vec![[400.0, 300.0], [700.0, 300.0]], "the other subpath is untouched");
}

/// The measured row: one History step, named "Transform Points".
#[test]
fn the_point_transform_is_one_history_step_named_transform_points() {
    let mut app = saved_path_app();
    let before = steps(&app);
    select_two_corners(&mut app);
    assert_eq!(steps(&app), before, "selecting anchors changes nothing");
    open_box(&mut app);
    set_box(&mut app, [[150.0, 350.0], [250.0, 350.0], [250.0, 450.0], [150.0, 450.0]]);
    commit_box(&mut app);
    assert_eq!((steps(&app), last_step(&app).as_deref()), (before + 1, Some("Transform Points")), "{}", app.ui.status);
    assert!(app.session.undo(), "the step undoes");
    assert_eq!(anchors(&path1(&app), 0), vec![[100.0, 300.0], [300.0, 300.0], [300.0, 500.0], [100.0, 500.0]], "Undo restores the square");
}

/// The Done-when row "with their own handles": a selected smooth anchor's direction handles follow
/// the box; the anchors and knots outside the selection do not move.
#[test]
fn a_selected_anchor_moves_its_own_handles() {
    let mut app = work_path_app();
    click(&mut app, [120.0, 40.0], Modifiers::NONE);
    click(&mut app, [120.0, 120.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1], [0, 2]]);
    open_box(&mut app);
    // The box covers the two anchors and the selected knot's handles: 100,40 to 140,120.
    assert_eq!(box_rect(&app), [100.0, 40.0, 140.0, 120.0]);
    // Twice as wide: the anchors sit on the box's centre line, so only the handles move in x.
    set_box(&mut app, [[80.0, 40.0], [160.0, 40.0], [160.0, 120.0], [80.0, 120.0]]);
    commit_box(&mut app);
    let p = work_path(&app);
    let (anchor, handle_in, handle_out) = knot(&p, 0, 1);
    assert_eq!([anchor.x, anchor.y], [120.0, 40.0]);
    assert_eq!([handle_in.x, handle_in.y], [80.0, 40.0], "the in handle followed the box");
    assert_eq!([handle_out.x, handle_out.y], [160.0, 40.0], "the out handle followed the box");
    assert_eq!(anchors(&p, 0), vec![[40.0, 40.0], [120.0, 40.0], [120.0, 120.0], [40.0, 120.0]], "the unselected anchors stayed");
    let (corner, corner_in, corner_out) = knot(&p, 0, 0);
    assert_eq!([(corner.x, corner.y), (corner_in.x, corner_in.y), (corner_out.x, corner_out.y)], [(40.0, 40.0), (40.0, 40.0), (40.0, 40.0)]);
    assert!(app.session.undo(), "one Undo takes the point transform back");
    let (anchor, handle_in, handle_out) = knot(&work_path(&app), 0, 1);
    assert_eq!(
        [(anchor.x, anchor.y), (handle_in.x, handle_in.y), (handle_out.x, handle_out.y)],
        [(120.0, 40.0), (100.0, 40.0), (140.0, 40.0)],
        "Undo restores the anchor and both handles"
    );
}

/// Neighbour: with nothing selected, ⌘T keeps today's whole-path transform and today's step name.
#[test]
fn nothing_selected_transforms_the_whole_path() {
    let mut app = saved_path_app();
    let before = steps(&app);
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0], "the box frames all of Path 1");
    assert_eq!(box_params(&app), json!({"name": "Path 1"}), "no anchors are named");
    set_box(&mut app, [[250.0, 300.0], [550.0, 300.0], [550.0, 500.0], [250.0, 500.0]]);
    commit_box(&mut app);
    assert_eq!((steps(&app), last_step(&app).as_deref()), (before + 1, Some("Transform Path")));
    let p = path1(&app);
    assert_eq!(anchors(&p, 0), vec![[250.0, 300.0], [350.0, 300.0], [350.0, 500.0], [250.0, 500.0]], "the whole square moved");
    assert_eq!(anchors(&p, 1), vec![[400.0, 300.0], [550.0, 300.0]], "and the line with it");
}

/// Neighbour: Path Selection is untouched by the point decision, even with a path targeted.
#[test]
fn path_selection_transforms_the_whole_path() {
    let mut app = saved_path_app();
    app.ui.tool = Tool::PathSelection;
    let before = steps(&app);
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0]);
    assert_eq!(box_params(&app), json!({"name": "Path 1"}));
    set_box(&mut app, [[250.0, 300.0], [550.0, 300.0], [550.0, 500.0], [250.0, 500.0]]);
    commit_box(&mut app);
    assert_eq!((steps(&app), last_step(&app).as_deref()), (before + 1, Some("Transform Path")));
    assert_eq!(anchors(&path1(&app), 0), vec![[250.0, 300.0], [350.0, 300.0], [350.0, 500.0], [250.0, 500.0]]);
}

/// Neighbour the ticket leaves unmeasured: anchors on several subpaths keep today's whole-path
/// transform.
#[test]
fn anchors_on_two_subpaths_keep_the_whole_path() {
    let mut app = saved_path_app();
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    click(&mut app, [400.0, 300.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1], [1, 0]], "one anchor in each subpath");
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0], "the box still frames the whole path");
    assert_eq!(box_params(&app), json!({"name": "Path 1"}), "no anchors are named");
    set_box(&mut app, [[250.0, 300.0], [550.0, 300.0], [550.0, 500.0], [250.0, 500.0]]);
    commit_box(&mut app);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"));
    assert_eq!(anchors(&path1(&app), 0), vec![[250.0, 300.0], [350.0, 300.0], [350.0, 500.0], [250.0, 500.0]]);
    assert_eq!(anchors(&path1(&app), 1), vec![[400.0, 300.0], [550.0, 300.0]]);
}

/// Neighbour the ticket leaves unmeasured: a segment selected on its own keeps today's whole-path
/// transform.
#[test]
fn a_segment_selection_keeps_the_whole_path() {
    let mut app = saved_path_app();
    click(&mut app, [200.0, 300.0], Modifiers::NONE);
    assert!(app.ui.direct_selection.anchors.is_empty(), "a segment click selects no anchor");
    assert_eq!(app.ui.direct_selection.segment, Some([0, 0]), "the segment between knots 0 and 1");
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0]);
    assert_eq!(box_params(&app), json!({"name": "Path 1"}));
    set_box(&mut app, [[250.0, 300.0], [550.0, 300.0], [550.0, 500.0], [250.0, 500.0]]);
    commit_box(&mut app);
    assert_eq!(anchors(&path1(&app), 0), vec![[250.0, 300.0], [350.0, 300.0], [350.0, 500.0], [250.0, 500.0]]);
}

/// Neighbour: the box keeps the anchor selection while it is open and leaves it, as a path transform
/// does today, when it commits. Same for the whole-path transform of the unmeasured multi-subpath
/// selection, so the point decision changes nothing there.
#[test]
fn the_transform_leaves_the_anchor_selection_as_a_path_transform_does() {
    let mut app = saved_path_app();
    select_two_corners(&mut app);
    open_box(&mut app);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1], [0, 3]], "the selection stands while the box is open");
    set_box(&mut app, [[150.0, 350.0], [250.0, 350.0], [250.0, 450.0], [150.0, 450.0]]);
    commit_box(&mut app);
    assert_eq!(app.ui.direct_selection.anchors, Vec::<[usize; 2]>::new(), "the box ends the selection, as a path transform does");

    let mut app = saved_path_app();
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    click(&mut app, [400.0, 300.0], Modifiers::SHIFT);
    open_box(&mut app);
    set_box(&mut app, [[250.0, 300.0], [550.0, 300.0], [550.0, 500.0], [250.0, 500.0]]);
    commit_box(&mut app);
    assert_eq!(app.ui.direct_selection.anchors, Vec::<[usize; 2]>::new(), "and the same with the whole-path transform");
}

/// The command on its own: `path.transform` with `anchors` pivots numeric scaling on the centre of
/// the anchors it names, which is how the measured box move comes out.
#[test]
fn the_command_transforms_named_anchors_about_their_own_centre() {
    let mut app = saved_path_app();
    app.run("path.transform", json!({"name": "Path 1", "anchors": [[0, 1], [0, 3]], "scaleX": 0.5, "scaleY": 0.5})).expect("the anchors scale");
    let p = path1(&app);
    assert_eq!(anchors(&p, 0), vec![[100.0, 300.0], [250.0, 350.0], [300.0, 500.0], [150.0, 450.0]]);
    assert_eq!(anchors(&p, 1), vec![[400.0, 300.0], [700.0, 300.0]], "the unlisted subpath is untouched");
    assert_eq!(last_step(&app).as_deref(), Some("Transform Points"));
}

/// Hostile params: an anchor list the path does not have is an error, never a whole-path transform
/// and never a panic.
#[test]
fn anchors_off_the_path_are_an_error() {
    let mut app = saved_path_app();
    let (before, n) = (path1(&app), steps(&app));
    for bad in [
        json!({"name": "Path 1", "anchors": [[9, 0]], "matrix": [1, 0, 0, 1, 5, 0]}),
        json!({"name": "Path 1", "anchors": [[0, 9]], "matrix": [1, 0, 0, 1, 5, 0]}),
        json!({"name": "Path 1", "anchors": [], "matrix": [1, 0, 0, 1, 5, 0]}),
        json!({"name": "Path 1", "anchors": [[0]], "matrix": [1, 0, 0, 1, 5, 0]}),
        json!({"name": "Path 1", "anchors": "0,1", "matrix": [1, 0, 0, 1, 5, 0]}),
    ] {
        assert!(app.run("path.transform", bad.clone()).is_err(), "{bad} is refused");
    }
    assert_eq!(path1(&app), before, "nothing moved");
    assert_eq!(steps(&app), n, "and no step was added");
}

/// The box needs an area to work in: one straight anchor spans none, so that selection keeps today's
/// whole-path transform. The reference application was not measured with a frame-less selection.
#[test]
fn one_anchor_selected_keeps_the_whole_path() {
    let mut app = saved_path_app();
    click(&mut app, [300.0, 300.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "one straight anchor is selected");
    let before = steps(&app);
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0], "the box frames the whole path");
    assert_eq!(box_params(&app), json!({"name": "Path 1"}), "no anchors are named");
    // Drag the box 20 right and 20 down, OK: every anchor of Path 1 follows.
    move_box(&mut app, 20.0, 20.0);
    commit_box(&mut app);
    assert_eq!((steps(&app), last_step(&app).as_deref()), (before + 1, Some("Transform Path")));
    assert_eq!(anchors(&path1(&app), 0), vec![[120.0, 320.0], [320.0, 320.0], [320.0, 520.0], [120.0, 520.0]]);
    assert_eq!(anchors(&path1(&app), 1), vec![[420.0, 320.0], [720.0, 320.0]], "the line moved with it");
}

/// Two anchors on one line span an area of nothing either: the same whole-path transform.
#[test]
fn collinear_anchors_keep_the_whole_path() {
    let mut app = saved_path_app();
    click(&mut app, [100.0, 300.0], Modifiers::NONE);
    click(&mut app, [300.0, 300.0], Modifiers::SHIFT);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 0], [0, 1]], "both top corners");
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0], "the box frames the whole path");
    assert_eq!(box_params(&app), json!({"name": "Path 1"}), "no anchors are named");
    set_box(&mut app, [[250.0, 300.0], [550.0, 300.0], [550.0, 500.0], [250.0, 500.0]]);
    commit_box(&mut app);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"));
    assert_eq!(anchors(&path1(&app), 0), vec![[250.0, 300.0], [350.0, 300.0], [350.0, 500.0], [250.0, 500.0]]);
    assert_eq!(anchors(&path1(&app), 1), vec![[400.0, 300.0], [550.0, 300.0]]);
}

/// Path Selection's component pick fills the same anchor list; the tool is checked first, so Ctrl+T
/// keeps the whole-path transform.
#[test]
fn a_path_selection_component_pick_keeps_the_whole_path() {
    let mut app = saved_path_app();
    app.ui.tool = Tool::PathSelection;
    click(&mut app, [200.0, 400.0], Modifiers::NONE);
    assert!(!app.ui.direct_selection.anchors.is_empty(), "the component pick writes the anchors it covers");
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0], "the box frames the whole path");
    assert_eq!(box_params(&app), json!({"name": "Path 1"}), "no anchors are named");
    move_box(&mut app, 20.0, 20.0);
    commit_box(&mut app);
    assert_eq!(last_step(&app).as_deref(), Some("Transform Path"));
    assert_eq!(anchors(&path1(&app), 0), vec![[120.0, 320.0], [320.0, 320.0], [320.0, 520.0], [120.0, 520.0]]);
    assert_eq!(anchors(&path1(&app), 1), vec![[420.0, 320.0], [720.0, 320.0]]);
}

/// #13284 makes Path Selection with Ctrl held act as Direct Selection, and the anchor it presses ends
/// up in that same list. The free-transform entry reads the tool in hand, so this keeps today's
/// whole-path transform; asked on the issue whether the held tool should decide instead.
#[test]
fn path_selection_with_ctrl_held_keeps_the_whole_path() {
    let mut app = saved_path_app();
    app.ui.tool = Tool::PathSelection;
    click(&mut app, [300.0, 300.0], Modifiers::COMMAND);
    assert_eq!(app.ui.direct_selection.anchors, vec![[0, 1]], "the Ctrl press selected the anchor");
    open_box(&mut app);
    assert_eq!(box_rect(&app), [100.0, 300.0, 700.0, 500.0], "today: the box frames the whole path");
    assert_eq!(box_params(&app), json!({"name": "Path 1"}), "today: no anchors are named");
}

/// Neighbour the ticket leaves unmeasured: anchors selected on a shape layer's path keep today's
/// whole-path box, params and step name.
#[test]
fn anchors_on_a_shape_layer_keep_the_whole_path() {
    let mut app = shape_app();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    click(&mut app, [750.0, 50.0], Modifiers::NONE);
    assert_eq!(app.ui.direct_selection.target, Some(crate::direct_select::PathRef::Layer(layer.0)), "the shape path is targeted");
    let selected = app.ui.direct_selection.anchors.clone();
    assert_eq!(selected.len(), 1, "the corner anchor is selected");
    let before = steps(&app);
    open_box(&mut app);
    assert_eq!(box_rect(&app), [550.0, 50.0, 750.0, 200.0], "the box frames the whole shape path");
    assert_eq!(box_params(&app), json!({"name": "layer", "layer": layer.0}), "no anchors are named");
    move_box(&mut app, 20.0, 20.0);
    commit_box(&mut app);
    assert_eq!((steps(&app), last_step(&app).as_deref()), (before + 1, Some("Transform Path")), "{}", app.ui.status);
    let shape = app.session.active().unwrap().doc.layer(layer).and_then(|l| match &l.content {
        photocraft_doc::LayerContent::Shape(sh) => Some(sh.path.clone()),
        _ => None,
    });
    let p = shape.expect("the layer is still a shape");
    let mut moved: Vec<[f64; 2]> = p.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    moved.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap().then(a[1].partial_cmp(&b[1]).unwrap()));
    assert_eq!(moved, vec![[570.0, 70.0], [570.0, 220.0], [770.0, 70.0], [770.0, 220.0]], "the whole rect moved");
}
