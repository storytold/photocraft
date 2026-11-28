//! #12905: with fresh settings or after Reset Tool, the Pen in Path mode has Exclude Overlapping
//! Shapes checked and a second subpath drawn into the Work Path is added as Exclude; the Pen in
//! Shape mode still has New Layer. Each mode keeps its own operation, and the Path-mode one is a
//! single value the Pen, the Freeform Pen, the Curvature Pen, Path Selection and Direct Selection
//! share. A settings file written by an older build, which had one `pen_path_op` for both modes,
//! still loads and its picked operation still counts.

use egui::accesskit::Toggled;
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, Path, PathOp};
use serde_json::json;

use super::pen::pen_path_op;
use super::pen::tool_path_op;
use crate::PhotocraftApp;
use crate::canvas::testing;
use crate::state::{Tool, ToolOptions};
use craft::draw::ui::VectorMode;

/// A 200 × 200 document, the Pen in Path mode: the issue's measured configuration.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Pen;
    app
}

/// Clicks a closed square with the Pen, from `(x, y)` with side `side`.
fn square(app: &mut PhotocraftApp, x: f64, y: f64, side: f64) {
    for (dx, dy) in [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)] {
        testing::click(app, [x + dx, y + dy], Modifiers::NONE);
    }
    testing::click(app, [x + 0.5, y + 0.5], Modifiers::NONE); // close
}

fn work(app: &PhotocraftApp) -> Path {
    app.session.active().unwrap().doc.work_path.clone().expect("a work path")
}

fn ops(path: &Path) -> Vec<PathOp> {
    path.subpaths.iter().map(|s| s.op).collect()
}

/// How much of the pixel at `(x, y)` the path's filled area covers: the composite of its
/// subpaths' operations, as the rasterizer applies them.
fn covered(path: &Path, x: i32, y: i32) -> f32 {
    photocraft_vector::fill_rasterizer(path, photocraft_vector::DEFAULT_TOLERANCE).render(photocraft_geom::Rect::from_xywh(x, y, 1, 1))[0]
}

/// The Pen's options bar in a harness wide enough for the whole bar.
fn pen_bar(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(ctx).with_size(vec2(1400.0, 400.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| super::options_bar(app, ui, Tool::Pen));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

fn open_menu(h: &mut Harness<'_, Box<PhotocraftApp>>) {
    h.get_by_label("Path operations").click();
    h.run_steps(3);
}

fn checked(h: &Harness<'_, Box<PhotocraftApp>>, entry: &str) -> bool {
    h.get_by_label(entry).accesskit_node().toggled() == Some(Toggled::True)
}

/// Fresh settings: the Pen in Path mode resolves to Exclude Overlapping Shapes, in Shape mode to a
/// new shape layer (New Layer).
#[test]
fn fresh_settings_give_the_pen_in_path_mode_exclude_overlapping_shapes() {
    let o = ToolOptions::default();
    assert_eq!(o.vector_mode, VectorMode::Path, "the Pen starts in Path mode");
    assert_eq!(pen_path_op(&o), Some("exclude"), "Path mode: Exclude Overlapping Shapes");
    let shape_options = ToolOptions { vector_mode: VectorMode::Shape, ..ToolOptions::default() };
    assert_eq!(pen_path_op(&shape_options), None, "Shape mode: a new shape layer");
}

/// Reset Tool on the Pen puts the Path-mode operation back to Exclude, also after a pick.
#[test]
fn reset_tool_puts_the_path_mode_operation_back_to_exclude() {
    let mut app = app();
    crate::path_components::set_op(&mut app, "combine").unwrap();
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("combine"));
    crate::tool_reset::reset_options(&mut app.ui.tool_options, Tool::Pen);
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("exclude"), "Reset Tool: Exclude again");
}

/// The options bar's menu: Exclude checked in Path mode, New Layer checked in Shape mode.
#[test]
fn the_menu_checks_exclude_in_path_mode_and_new_layer_in_shape_mode() {
    let mut h = pen_bar(app());
    open_menu(&mut h);
    assert!(checked(&h, "Exclude Overlapping Shapes"), "Path mode: Exclude checked");
    assert!(!checked(&h, "Combine Shapes"), "not Combine, as it was before #12905");
    assert!(h.get_by_label("New Layer").accesskit_node().is_disabled(), "Path mode has no New Layer");
    // A second bar, the Pen in Shape mode: the menu stays open, so each half gets its own harness.
    let mut app = app();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    let mut h = pen_bar(app);
    open_menu(&mut h);
    assert!(checked(&h, "New Layer"), "Shape mode: New Layer checked");
    assert!(!h.get_by_label("New Layer").accesskit_node().is_disabled());
    assert!(!checked(&h, "Exclude Overlapping Shapes"));
}

/// The menu keeps the entries in the reference's order, the separator and Merge Shape Components
/// last: only the checked entry changes.
#[test]
fn the_menu_entries_keep_their_order() {
    let mut h = pen_bar(app());
    open_menu(&mut h);
    let top = |name: &str| h.get_by_label(name).rect().min.y;
    let order = ["New Layer", "Combine Shapes", "Subtract Front Shape", "Intersect Shape Areas", "Exclude Overlapping Shapes", "Merge Shape Components"];
    for pair in order.windows(2) {
        assert!(top(pair[0]) < top(pair[1]), "{} before {}: {} vs {}", pair[0], pair[1], top(pair[0]), top(pair[1]));
    }
}

/// The composite effect, relative: two overlapping subpaths drawn with the default operation paint
/// the union without the intersection, exactly as the same outlines stored with Exclude applied
/// explicitly, and not as they paint when both are Combined.
#[test]
fn the_default_paints_two_overlapping_subpaths_as_exclude() {
    let mut app = app();
    square(&mut app, 20.0, 20.0, 100.0);
    square(&mut app, 50.0, 50.0, 100.0);
    let drawn = work(&app);
    assert_eq!(ops(&drawn), [PathOp::Combine, PathOp::Exclude], "the second subpath takes the Path-mode default");
    // The same outlines, with the second subpath's operation set by hand.
    let mut explicit = drawn.clone();
    explicit.subpaths[1].op = PathOp::Exclude;
    let mut combined = drawn.clone();
    combined.subpaths[1].op = PathOp::Combine;
    for x in (0..=180).step_by(10) {
        for y in (0..=180).step_by(10) {
            assert_eq!(covered(&drawn, x, y), covered(&explicit, x, y), "coverage at ({x}, {y}) differs from the explicit Exclude");
        }
    }
    // Where only the two overlap Exclude and Combine differ, so the comparison above is not vacuous.
    assert_eq!((covered(&drawn, 70, 70), covered(&combined, 70, 70)), (0.0, 1.0), "the overlap paints away");
    assert_eq!(covered(&drawn, 30, 30), 1.0, "the first square alone still paints");
    assert_eq!(covered(&drawn, 140, 140), 1.0, "the second square alone still paints");
}

/// The Pen in Shape mode after Reset Tool still makes a new shape layer.
#[test]
fn shape_mode_after_reset_still_makes_a_new_shape_layer() {
    let mut app = app();
    crate::tool_reset::reset_options(&mut app.ui.tool_options, Tool::Pen);
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    assert_eq!(pen_path_op(&app.ui.tool_options), None);
    square(&mut app, 20.0, 20.0, 60.0);
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    assert!(matches!(st.doc.layer(id).unwrap().content, LayerContent::Shape(_)), "a shape layer");
    let count = st.doc.layers.len();
    square(&mut app, 100.0, 20.0, 60.0);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), count + 1, "the next path makes another layer");
}

/// The two modes keep their own operation: a pick in one does not move the other, in either
/// direction and for the first pick as much as for a later one.
#[test]
fn each_mode_keeps_the_operation_picked_in_the_other() {
    let mut app = app();
    // Pick in Path mode, look at Shape mode, come back.
    crate::path_components::set_op(&mut app, "combine").unwrap();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    assert_eq!(pen_path_op(&app.ui.tool_options), None, "Shape mode: New Layer, not the Path-mode pick");
    app.ui.tool_options.vector_mode = VectorMode::Path;
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("combine"), "Path mode keeps its own pick");
    // Pick in Shape mode first: Path mode keeps what it had (the default Exclude).
    app.ui.tool_options = ToolOptions::default();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    crate::path_components::set_op(&mut app, "intersect").unwrap();
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("intersect"), "Shape mode keeps its pick");
    app.ui.tool_options.vector_mode = VectorMode::Path;
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("exclude"), "Path mode keeps the default, not the Shape-mode pick");
    crate::path_components::set_op(&mut app, "subtract").unwrap();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("intersect"), "and it keeps its own pick too");
}

/// One Path-mode value for the three Pens: picked on one, the other two show it.
#[test]
fn the_three_pens_share_one_path_mode_operation() {
    let mut app = app();
    for (first, others) in [
        (Tool::Pen, [Tool::FreeformPen, Tool::CurvaturePen]),
        (Tool::FreeformPen, [Tool::Pen, Tool::CurvaturePen]),
        (Tool::CurvaturePen, [Tool::Pen, Tool::FreeformPen]),
    ] {
        app.ui.tool = first;
        crate::path_components::set_op(&mut app, "subtract").unwrap();
        for tool in others {
            app.ui.tool = tool;
            assert_eq!(tool_path_op(&app), Some("subtract"), "{tool:?} shows the pick made on {first:?}");
        }
    }
    // And after Reset Tool on the Pen all three start at Exclude again.
    crate::tool_reset::reset_options(&mut app.ui.tool_options, Tool::Pen);
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("exclude"), "{tool:?}");
    }
}

/// Path Selection and Direct Selection read and write that same Path-mode value, even while the
/// Pen happens to be in Shape mode; their own menu shows it with nothing selected.
#[test]
fn the_two_selection_tools_share_the_path_mode_operation() {
    let mut app = app();
    // A pick on the Pen shows on both selection tools, also while the Pen is in Shape mode.
    crate::path_components::set_op(&mut app, "intersect").unwrap();
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("intersect"), "{tool:?} shows the Pen's Path-mode pick with the Pen in Shape mode");
    }
    // And a pick on either selection tool shows on the Pen, each tool starting from fresh settings.
    for (tool, picked) in [(Tool::PathSelection, "subtract"), (Tool::DirectSelection, "combine")] {
        app.ui.tool_options = ToolOptions::default();
        app.ui.tool = tool;
        crate::path_components::set_op(&mut app, picked).unwrap();
        app.ui.tool_options.vector_mode = VectorMode::Path;
        app.ui.tool = Tool::Pen;
        assert_eq!(pen_path_op(&app.ui.tool_options), Some(picked), "the Pen shows the {tool:?} pick in Path mode");
    }
}

/// A settings file written by an older build names only `pen_path_op`; it still loads, and the
/// operation it picked counts in both modes until one of them is picked again.
#[test]
fn settings_saved_by_older_builds_still_load() {
    for (saved, path_mode, shape_mode) in [("subtract", Some("subtract"), Some("subtract")), ("combine", Some("combine"), Some("combine"))] {
        let mut app = app();
        app.session.prefs.edit(|p| p.tool_options = json!({"pen_path_op": saved}));
        crate::tool_memory::restore(&mut app);
        app.ui.tool = Tool::Pen;
        assert_eq!(pen_path_op(&app.ui.tool_options), path_mode, "Path mode keeps the old file's {saved}");
        app.ui.tool_options.vector_mode = VectorMode::Shape;
        assert_eq!(pen_path_op(&app.ui.tool_options), shape_mode, "Shape mode keeps the old file's {saved}");
        // Picking in one mode does not lose the other mode's carried-over value.
        crate::path_components::set_op(&mut app, "exclude").unwrap();
        app.ui.tool_options.vector_mode = VectorMode::Path;
        assert_eq!(pen_path_op(&app.ui.tool_options), Some(saved), "Path mode keeps {saved}");
    }
    // An older file that never picked anything lands on the new default.
    let mut app = app();
    app.session.prefs.edit(|p| p.tool_options = json!({"pen_path_op": "newLayer"}));
    crate::tool_memory::restore(&mut app);
    app.ui.tool = Tool::Pen;
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("exclude"), "the old New Layer default reads as Exclude in Path mode");
    app.ui.tool_options.vector_mode = VectorMode::Shape;
    assert_eq!(pen_path_op(&app.ui.tool_options), None, "and New Layer in Shape mode");
}

/// Which Path Operations draw the Work Path live (#12890, measured after Reset Tool): Combine and
/// the new Exclude default. The operations #12890 did not measure keep today's route: with Subtract
/// or Intersect the first click is no step of its own and the Work Path appears only at the finish.
#[test]
fn an_unmeasured_operation_keeps_the_draft_until_the_path_is_finished() {
    let steps = |app: &PhotocraftApp| app.session.active().unwrap().history.past_len();
    for op in ["subtract", "intersect"] {
        let mut app = app();
        crate::path_components::set_op(&mut app, op).unwrap();
        let before = steps(&app);
        testing::click(&mut app, [20.0, 20.0], Modifiers::NONE);
        assert_eq!(steps(&app), before, "{op}: the first click is no History step of its own");
        assert!(app.session.active().unwrap().doc.work_path.is_none(), "{op}: no Work Path yet");
    }
    // The new Exclude default takes the live route that Combine had.
    let mut app = app();
    let before = steps(&app);
    testing::click(&mut app, [20.0, 20.0], Modifiers::NONE);
    assert_eq!(steps(&app), before + 1, "the Exclude default: the first click is its own step");
    assert!(app.session.active().unwrap().doc.work_path.is_some(), "and the Work Path exists at once");
}

/// The other live gate of #12890, the press on an open Work Path's endpoint: it runs for Exclude as
/// it ran for Combine (the step "Pick-up Path"), and Subtract keeps today's route.
#[test]
fn the_pick_up_step_follows_the_same_two_operations() {
    let names = |app: &PhotocraftApp| app.session.active().unwrap().history.entries();
    let mut exclude = app();
    let before = names(&exclude).len();
    testing::click(&mut exclude, [20.0, 20.0], Modifiers::NONE);
    testing::click(&mut exclude, [120.0, 20.0], Modifiers::NONE);
    testing::key(&mut exclude, Key::Escape, Modifiers::NONE);
    testing::click(&mut exclude, [120.0, 20.0], Modifiers::NONE);
    assert_eq!(&names(&exclude)[before..], ["New Work Path", "New Anchor Point", "Pick-up Path"], "the Exclude default draws live and picks the endpoint up");
    let mut subtract = app();
    crate::path_components::set_op(&mut subtract, "subtract").unwrap();
    let before = names(&subtract).len();
    testing::click(&mut subtract, [20.0, 20.0], Modifiers::NONE);
    testing::click(&mut subtract, [120.0, 20.0], Modifiers::NONE);
    testing::key(&mut subtract, Key::Escape, Modifiers::NONE);
    testing::click(&mut subtract, [120.0, 20.0], Modifiers::NONE);
    assert_eq!(&names(&subtract)[before..], ["Work Path"], "Subtract: one step at the finish, and the endpoint press is no step of its own");
}

/// Reset Tool and a tool preset carry the shared Path-mode value for the Pen and the Curvature Pen,
/// so after Reset Tool on either of them all three Pens read Exclude again. The Freeform Pen's line
/// of `option_keys` is #17087's to change (it rewrites that exact row), so the key is not in it yet.
#[test]
fn the_path_mode_value_resets_for_the_pen_and_the_curvature_pen() {
    for tool in [Tool::Pen, Tool::CurvaturePen] {
        assert!(crate::tool_reset::option_keys(tool).contains(&"pen_path_mode_op"), "{tool:?} resets and stores the Path-mode value");
    }
    let mut app = app();
    app.ui.tool = Tool::FreeformPen;
    crate::path_components::set_op(&mut app, "combine").unwrap();
    for reset_on in [Tool::Pen, Tool::CurvaturePen] {
        crate::tool_reset::reset_options(&mut app.ui.tool_options, reset_on);
        for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen] {
            app.ui.tool = tool;
            assert_eq!(tool_path_op(&app), Some("exclude"), "Reset Tool on {reset_on:?} gives {tool:?} Exclude again");
        }
    }
}

/// An unknown value in the new Path-mode slot (an edited settings file) reads as the default and
/// does not move the Shape-mode value.
#[test]
fn an_unknown_path_mode_value_reads_as_the_default() {
    let o = ToolOptions { pen_path_mode_op: Some("melt".into()), ..ToolOptions::default() };
    assert_eq!(pen_path_op(&o), Some("exclude"), "Path mode: the default, never a panic");
    let o = ToolOptions { pen_path_mode_op: Some("melt".into()), vector_mode: VectorMode::Shape, ..ToolOptions::default() };
    assert_eq!(pen_path_op(&o), None, "Shape mode still reads its own New Layer");
}

/// The new default reaches the other two places a Path-mode subpath is added: a targeted vector
/// mask, and Make Shape of the path just drawn.
#[test]
fn the_new_default_reaches_a_targeted_vector_mask_and_make_shape() {
    let mut masked = app();
    let id = masked.session.active().unwrap().active_layer.unwrap().0;
    masked
        .run("layer.vectorMask.add", json!({"layer": id, "path": {"subpaths": [{"closed": true, "knots": [[0, 0], [190, 0], [190, 190], [0, 190]]}]}}))
        .unwrap();
    masked.ui.vector_mask_target = true;
    // Clear of the mask's own segments, which the Pen's Auto Add/Delete would edit.
    square(&mut masked, 60.0, 60.0, 60.0);
    let mask = masked.session.active().unwrap().doc.layer(LayerId(id)).unwrap().vector_mask.clone().unwrap();
    assert_eq!(ops(&mask.path), [PathOp::Combine, PathOp::Exclude], "the mask takes the Path-mode default for the new subpath");
    let mut drawn = app();
    square(&mut drawn, 20.0, 20.0, 100.0);
    square(&mut drawn, 50.0, 50.0, 100.0);
    super::make_shape(&mut drawn).unwrap();
    let made = shape_path(&drawn);
    assert_eq!(ops(&made), [PathOp::Combine, PathOp::Exclude], "Make Shape keeps the operations of the drawn path");
    assert_eq!(covered(&made, 70, 70), 0.0, "and the shape it makes shows the exclusion");
}

/// The active layer's shape path.
fn shape_path(app: &PhotocraftApp) -> Path {
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    match &st.doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.path.clone(),
        _ => panic!("the active layer is not a shape layer"),
    }
}

/// The control channel's `penPathOp` (the reference's `shapeOperation`) picks that same one
/// Path-mode value: set by script, it shows on all five path tools. With a Shape tool active the
/// setter keeps the route it had before #12905 and writes the Pens' Shape-mode value.
#[test]
fn the_control_channel_picks_the_same_path_mode_value() {
    use crate::control::{ControlRequest, Reply, handle};
    let mut app = app();
    let ctx = app.ctx().clone();
    let (req, _rx) = ControlRequest::new("ui.set", json!({"penMode": "path", "penPathOp": "subtract"}));
    assert!(matches!(handle(&mut app, &ctx, &req), Reply::Done(_)), "{}", app.ui.status);
    for tool in [Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::PathSelection, Tool::DirectSelection] {
        app.ui.tool = tool;
        assert_eq!(tool_path_op(&app), Some("subtract"), "{tool:?} shows the pick made by script");
    }
    // `ui.inspect` reports the new slot, which is what the protocol documents.
    let (req, _rx) = ControlRequest::new("ui.inspect", json!({}));
    let Reply::Done(seen) = handle(&mut app, &ctx, &req) else { panic!("ui.inspect replies with the state") };
    assert_eq!(seen["result"]["toolOptions"]["pen_path_mode_op"].as_str(), Some("subtract"), "ui.inspect reports pen_path_mode_op");
    // A Shape tool active: the setter keeps the route it had before #12905.
    app.ui.tool = Tool::Rectangle;
    let (req, _rx) = ControlRequest::new("ui.set", json!({"penPathOp": "combine"}));
    assert!(matches!(handle(&mut app, &ctx, &req), Reply::Done(_)), "{}", app.ui.status);
    assert_eq!(app.ui.tool_options.pen_path_op, "combine", "the Shape tool route writes the Pens' Shape-mode value");
    assert_eq!(app.ui.tool_options.shape_path_op, "newLayer", "the Shape tools' own value stays at New Layer");
    assert_eq!(app.ui.tool_options.pen_path_mode_op.as_deref(), Some("subtract"), "the Path-mode pick is untouched");
    // The control channel accepts "newLayer", which the bar greys out in Path mode: it is stored as
    // given and read as the Path-mode default, so nothing here decides that case on its own.
    app.ui.tool = Tool::Pen;
    let (req, _rx) = ControlRequest::new("ui.set", json!({"penMode": "path", "penPathOp": "newLayer"}));
    assert!(matches!(handle(&mut app, &ctx, &req), Reply::Done(_)), "{}", app.ui.status);
    assert_eq!(app.ui.tool_options.pen_path_mode_op.as_deref(), Some("newLayer"), "stored as sent");
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("exclude"), "and read as Exclude Overlapping Shapes");
}

/// The Shape tools keep their own Path operations setting: the Pen's new default does not move it.
#[test]
fn a_shape_tool_keeps_its_own_operations_setting() {
    let mut app = app();
    app.ui.tool = Tool::Rectangle;
    assert_eq!(super::pen::shape_path_op(&app.ui.tool_options), None, "the Shape tools start at New Layer");
    crate::path_components::set_op(&mut app, "combine").unwrap();
    assert_eq!(super::pen::shape_path_op(&app.ui.tool_options), Some("combine"));
    app.ui.tool = Tool::Pen;
    assert_eq!(pen_path_op(&app.ui.tool_options), Some("exclude"), "the Pen's Path-mode value is untouched");
}
