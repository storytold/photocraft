//! #12543: the Remove tool's options bar starts with the Add to / Subtract from brushed area pair
//! (Add on by default and after Reset Tool), and a stroke with Subtract on - or an Alt-drag - takes
//! its footprint out of the pending area instead of adding to it. ↩ then applies the strokes as one
//! History state and leaves the pixels under the subtracted part untouched. A subtract stroke with
//! nothing pending changes nothing.

use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

/// The pair's words, as the reference application writes them (its tooltips).
const ADD: &str = "Add to brushed area";
const SUBTRACT: &str = "Subtract from brushed area";

/// The red (220,30,30) square of the issue's test document: x 300-360, y 250-310.
const RED: [f32; 3] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0];

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    app
}

/// The Remove tool on the issue's test document (800 x 600 RGB 8 bit): vertical 20 px stripes of
/// (60,120,200) and (200,200,80), and the red square at x 300-360, y 250-310. Remove after each
/// stroke is off, so the strokes wait, and the brush is 30 px and hard-edged.
fn measured() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    for x in (20..800).step_by(40) {
        app.run("paint.pencil", json!({"points": [[x, 0], [x, 600]], "size": 40, "color": "#3c78c8"})).unwrap();
    }
    for x in (30..800).step_by(40) {
        app.run("paint.pencil", json!({"points": [[x, 0], [x, 600]], "size": 20, "color": "#c8c850"})).unwrap();
    }
    app.run("paint.pencil", json!({"points": [[330, 280]], "size": 60, "color": "#dc1e1e"})).unwrap();
    app.ui.tool = Tool::Remove;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.ui.tool_options.remove_after_each_stroke = false;
    app.run("tools.setBrush", json!({"brush": {"size": 30.0, "hardness": 1.0}})).unwrap();
    app
}

fn active(app: &PhotocraftApp) -> &photocraft_doc::Layer {
    let st = app.session.active().unwrap();
    st.active_layer.and_then(|id| st.doc.layer(id)).unwrap()
}

/// The active layer's colour at `x`, `y`, without the alpha.
fn px(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 3] {
    let c = active(app).surface().unwrap().rgba(x, y);
    [c[0], c[1], c[2]]
}

fn is_red(c: [f32; 3]) -> bool {
    c.iter().zip(RED).all(|(a, b)| (a - b).abs() < 0.02)
}

/// Down at the first point, a move through each next one, up at the last, with `mods` held.
fn drag(app: &mut PhotocraftApp, points: &[[f64; 2]], mods: egui::Modifiers) {
    let [x, y] = points[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    for &[x, y] in &points[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
    }
    let [x, y] = points[points.len() - 1];
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

/// The three strokes across the square of [`measured`], held to the pending area.
fn stroke_the_square(app: &mut PhotocraftApp) {
    for y in [250.0, 280.0, 310.0] {
        drag(app, &[[290.0, y], [330.0, y], [370.0, y]], egui::Modifiers::NONE);
    }
}

/// A settled options bar with the Remove tool.
fn remove_bar() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = app();
    app.ui.tool = Tool::Remove;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

/// The bar's control named `label`, by a role that is no text node.
fn control<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, label: &'a str) -> egui_kittest::Node<'a> {
    h.query_all_by_label(label).find(|n| n.accesskit_node().role() != Role::Label).unwrap_or_else(|| panic!("no control named {label:?}"))
}

/// Whether the bar shows `label` as the picked one of its pair.
fn picked(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> bool {
    let node = control(h, label).accesskit_node();
    node.toggled() == Some(Toggled::True) || node.is_selected() == Some(true)
}

/// The 100 x 60 document of [`app`] with a grey field and two dark marks, at x 30 and x 60.
fn stripe_marks(app: &mut PhotocraftApp) {
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.run("paint.pencil", json!({"points": [[30, 15], [30, 45]], "size": 4, "color": "#000000"})).unwrap();
    app.run("paint.pencil", json!({"points": [[60, 15], [60, 45]], "size": 4, "color": "#000000"})).unwrap();
}

#[test]
fn the_bar_shows_the_add_and_subtract_pair_first() {
    let h = remove_bar();
    let (add, subtract) = (control(&h, ADD), control(&h, SUBTRACT));
    let first_box = h.query_all_by_role(Role::CheckBox).map(|n| n.rect().left()).fold(f32::MAX, f32::min);
    assert!(add.rect().left() < first_box && subtract.rect().left() > add.rect().left(), "the pair comes before the tool's checks, Add first");
    assert!(picked(&h, ADD), "Add to brushed area is the one on");
    assert!(!picked(&h, SUBTRACT), "Subtract from brushed area is off");
}

#[test]
fn clicking_subtract_picks_it_and_reset_tool_brings_back_add() {
    let mut h = remove_bar();
    control(&h, SUBTRACT).click();
    h.run_steps(2);
    assert!(picked(&h, SUBTRACT) && !picked(&h, ADD), "the pair is exclusive, Subtract now picked");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(2);
    assert!(picked(&h, ADD) && !picked(&h, SUBTRACT), "Reset Tool restores Add");
}

/// The issue's measured case: the strokes over the whole square wait, the Alt-drag over part of it
/// leaves that part out, and ↩ fills the rest while the Alt-drag's area keeps its red.
#[test]
fn an_alt_drag_takes_its_footprint_out_of_the_pending_area() {
    let mut app = measured();
    stroke_the_square(&mut app);
    assert_eq!(app.remove_pending.len(), 3, "the strokes wait");
    drag(&mut app, &[[345.0, 280.0], [375.0, 280.0]], egui::Modifiers::ALT);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(app.remove_pending.len(), 4, "the Alt-drag joins what waits");
    assert!(crate::retouch_ui::remove_pending_commit(&mut app));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(is_red(px(&app, 350, 280)), "the Alt-drag's area keeps the red: {:?}", px(&app, 350, 280));
    for at in [(310, 280), (350, 255), (310, 255)] {
        assert!(!is_red(px(&app, at.0, at.1)), "{at:?} no longer holds the red: {:?}", px(&app, at.0, at.1));
    }
}

/// The hole's two-pixel growth stops at a Subtract footprint: pixels inside the Alt-drag's
/// footprint next to its edge keep their red. The map of the applied document before the engine
/// order fix had these two filled (no longer red); they are the fringe of the growth into the
/// subtracted footprint. The hole mask itself is pinned in the engine crate
/// (`retouch_cmds/remove.rs`, `remove_subtract_grow_tests`).
#[test]
fn the_grown_hole_edge_stops_at_the_subtracted_footprint() {
    let mut app = measured();
    stroke_the_square(&mut app);
    drag(&mut app, &[[345.0, 280.0], [375.0, 280.0]], egui::Modifiers::ALT);
    assert!(crate::retouch_ui::remove_pending_commit(&mut app));
    assert!(!app.ui.status_error, "{}", app.ui.status);
    for at in [(350, 264), (350, 296)] {
        assert!(
            is_red(px(&app, at.0, at.1)),
            "{at:?} sits inside the Alt-drag's footprint, within the grown edge, and keeps the red: {:?}",
            px(&app, at.0, at.1)
        );
    }
}

/// The same with the options bar's Subtract picked instead of Alt held.
#[test]
fn subtract_on_the_bar_takes_its_footprint_out_too() {
    let mut h = remove_bar();
    let app = h.state_mut();
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.run("paint.pencil", json!({"points": [[30, 15], [30, 45]], "size": 4, "color": "#000000"})).unwrap();
    app.run("paint.pencil", json!({"points": [[60, 15], [60, 45]], "size": 4, "color": "#000000"})).unwrap();
    app.ui.tool_options.remove_after_each_stroke = false;
    app.run("tools.setBrush", json!({"brush": {"size": 12.0, "hardness": 1.0}})).unwrap();
    let dark = |app: &PhotocraftApp, x: i32| px(app, x, 30).iter().all(|v| *v < 0.05);
    assert!(dark(h.state(), 30) && dark(h.state(), 60), "both marks are dark to start with");
    // One Add stroke across both marks; the bar's Subtract then takes the second one back out.
    drag(h.state_mut(), &[[24.0, 30.0], [66.0, 30.0]], egui::Modifiers::NONE);
    control(&h, SUBTRACT).click();
    h.run_steps(2);
    drag(h.state_mut(), &[[60.0, 18.0], [60.0, 42.0]], egui::Modifiers::NONE);
    assert_eq!(h.state().remove_pending.len(), 2, "both strokes wait");
    assert!(crate::retouch_ui::remove_pending_commit(h.state_mut()));
    let app = h.state();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(!dark(app, 30), "the Add stroke's mark was filled: {:?}", px(app, 30, 30));
    assert!(dark(app, 60), "the Subtract stroke's mark stays: {:?}", px(app, 60, 30));
}

/// The apply is one History state for the whole pending area, and the journal sees every stroke
/// with the subtract one named, as `paint.remove` documents it.
#[test]
fn the_pending_strokes_apply_as_one_history_state() {
    let mut app = measured();
    let (journal, steps) = (app.session.journal.len(), app.session.active().unwrap().history.past_len());
    stroke_the_square(&mut app);
    drag(&mut app, &[[345.0, 280.0], [375.0, 280.0]], egui::Modifiers::ALT);
    assert_eq!((app.session.journal.len(), app.session.active().unwrap().history.past_len()), (journal, steps), "nothing runs before ↩");
    assert!(crate::retouch_ui::remove_pending_commit(&mut app));
    let steps_after = app.session.active().unwrap().history.past_len();
    assert_eq!((app.session.journal.len() - journal, steps_after - steps), (1, 1), "one command, one History state");
    let (id, p) = app.session.journal.last().cloned().expect("a journaled command");
    assert_eq!(id, "paint.remove");
    let strokes = p["strokes"].as_array().expect("the strokes went as one param");
    assert_eq!(strokes.len(), 4);
    let subtracts: Vec<bool> = strokes.iter().map(|e| e["subtract"].as_bool().unwrap_or(false)).collect();
    assert_eq!(subtracts, [false, false, false, true], "the Alt-drag is the documented subtract entry");
}

#[test]
fn a_subtract_stroke_with_nothing_pending_changes_nothing() {
    for by_alt in [true, false] {
        let mut app = measured();
        let (journal, steps) = (app.session.journal.len(), app.session.active().unwrap().history.past_len());
        if by_alt {
            drag(&mut app, &[[345.0, 280.0], [375.0, 280.0]], egui::Modifiers::ALT);
        } else {
            app.ui.tool_options.remove_subtract = true;
            drag(&mut app, &[[345.0, 280.0], [375.0, 280.0]], egui::Modifiers::NONE);
        }
        assert_eq!(
            (app.remove_pending.len(), app.remove_trail.is_some(), app.session.journal.len(), app.session.active().unwrap().history.past_len()),
            (0, false, journal, steps),
            "by_alt {by_alt}: nothing pending, nothing changes and no History state"
        );
        // The square's red is still there for the check below, and strokes drawn after it wait as
        // they did, back in the bar's Add mode.
        assert!(is_red(px(&app, 350, 280)), "by_alt {by_alt}: the pixels are untouched");
        app.ui.tool_options.remove_subtract = false;
        stroke_the_square(&mut app);
        assert_eq!(app.remove_pending.len(), 3, "by_alt {by_alt}: the next strokes wait as they did");
    }
}

/// What waits on the canvas: an Alt-drag tints nothing of its own, and the strokes that wait keep
/// their tint. Where a Subtract cuts into an area an earlier Add already tinted the tint stays,
/// because the framework's coverage mask can only grow; `paint.remove` still leaves those pixels
/// alone, which the measured test above checks.
#[test]
fn the_waiting_tint_leaves_the_subtract_stroke_out() {
    let mut app = measured();
    stroke_the_square(&mut app);
    let tinted = |app: &PhotocraftApp| {
        let trail = app.remove_trail.as_ref().expect("the waiting strokes are tinted");
        (230..330).flat_map(|y| (260..450).map(move |x| (x, y))).filter(|(x, y)| trail.coverage(*x, *y) > 0).count()
    };
    let before = tinted(&app);
    assert!(before > 0, "the Add strokes tint what they cover");
    drag(&mut app, &[[345.0, 280.0], [410.0, 280.0]], egui::Modifiers::ALT);
    assert_eq!(app.remove_pending.len(), 4, "the Alt-drag joins what waits");
    assert_eq!(tinted(&app), before, "its footprint adds nothing to the tint");
}

/// Not measured: Subtract with Remove after each stroke on keeps today's per-stroke removal.
#[test]
fn subtract_with_remove_after_each_stroke_on_still_removes_every_stroke() {
    let mut app = app();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_after_each_stroke = true;
    app.ui.tool_options.remove_subtract = true;
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.run("paint.pencil", json!({"points": [[30, 15], [30, 45]], "size": 4, "color": "#000000"})).unwrap();
    let (journal, steps) = (app.session.journal.len(), app.session.active().unwrap().history.past_len());
    drag(&mut app, &[[30.0, 18.0], [30.0, 42.0]], egui::Modifiers::ALT);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.remove_pending.is_empty() && app.remove_trail.is_none(), "nothing waits");
    assert_eq!((app.session.journal.len() - journal, app.session.active().unwrap().history.past_len() - steps), (1, 1), "the stroke was removed on its own");
    let (id, p) = app.session.journal.last().cloned().expect("a journaled command");
    assert_eq!(
        (id.as_str(), p.get("strokes").is_some(), p["points"].as_array().map(Vec::len)),
        ("paint.remove", false, Some(2)),
        "as a plain one-stroke removal"
    );
}

/// `paint.remove` takes the subtract strokes as a `strokes` entry param: the subtracted stroke's
/// footprint comes out of the hole, and the other one is filled.
#[test]
fn paint_remove_honours_a_subtract_entry() {
    let mut app = app();
    stripe_marks(&mut app);
    let (at, kept) = (px(&app, 30, 30), px(&app, 60, 30));
    assert!(at.iter().all(|v| *v < 0.05) && at == kept, "both marks are dark to start with: {at:?} {kept:?}");
    let r = app.run(
        "paint.remove",
        json!({
            "strokes": [
                {"points": [[30, 18], [30, 42]]},
                {"points": [[60, 18], [60, 42]], "subtract": true},
            ],
            "size": 12.0,
        }),
    );
    assert!(r.is_ok(), "the removal runs: {:?}", r.err());
    let grey = 128.0 / 255.0;
    let filled = px(&app, 30, 30);
    assert!(filled.iter().all(|v| (v - grey).abs() < 0.02), "the add entry's mark was filled with the grey field: {filled:?}");
    assert!(px(&app, 60, 30).iter().all(|v| *v < 0.05), "the subtract entry left its mark: {:?}", px(&app, 60, 30));
}

/// The subtraction is a set operation on the pending area, not a sequence: where in `strokes` its
/// entry sits makes no difference.
#[test]
fn the_subtract_entry_works_whatever_its_place_in_the_list() {
    let mut app = app();
    stripe_marks(&mut app);
    let r = app.run(
        "paint.remove",
        json!({
            "strokes": [
                {"points": [[60, 18], [60, 42]], "subtract": true},
                {"points": [[30, 18], [30, 42]]},
            ],
            "size": 12.0,
        }),
    );
    assert!(r.is_ok(), "the removal runs: {:?}", r.err());
    let grey = 128.0 / 255.0;
    assert!(px(&app, 30, 30).iter().all(|v| (v - grey).abs() < 0.02), "the add entry's mark was filled: {:?}", px(&app, 30, 30));
    assert!(px(&app, 60, 30).iter().all(|v| *v < 0.05), "the subtract entry left its mark: {:?}", px(&app, 60, 30));
}

/// A `subtract` that is no boolean is refused with an error, never a panic; so is an entry with the
/// flag and no points.
#[test]
fn paint_remove_refuses_a_malformed_subtract_entry() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[0, 30], [100, 30]], "size": 200, "color": "#808080"})).unwrap();
    app.run("paint.pencil", json!({"points": [[30, 15], [30, 45]], "size": 4, "color": "#000000"})).unwrap();
    for bad in [
        json!({"strokes": [{"points": [[30, 18], [30, 42]], "subtract": "yes"}], "size": 12.0}),
        json!({"strokes": [{"points": [[30, 18], [30, 42]], "subtract": 1}], "size": 12.0}),
        json!({"strokes": [{"subtract": true}], "size": 12.0}),
        json!({"strokes": [true], "size": 12.0}),
    ] {
        let before = (app.session.journal.len(), px(&app, 30, 30));
        let r = app.run("paint.remove", bad.clone());
        assert!(r.is_err(), "{bad}: a malformed entry is refused, not run");
        assert!(r.as_ref().err().unwrap().contains("paint.remove"), "{bad}: the error names the command: {:?}", r.err());
        assert_eq!((app.session.journal.len(), px(&app, 30, 30)), (before.0, before.1), "{bad}: nothing changed");
    }
}

/// The other retouch tools' bars are untouched: no pair anywhere else, and Patch keeps its own arm
/// (another open PR edits that arm).
#[test]
fn the_other_retouch_bars_keep_their_own_cells() {
    for tool in [Tool::Patch, Tool::SpotHealing, Tool::Healing] {
        let mut app = app();
        app.ui.tool = tool;
        let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut Box<PhotocraftApp>| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::options_bar(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
        h.run_steps(4);
        for label in [ADD, SUBTRACT] {
            assert!(h.query_all_by_label(label).next().is_none(), "{tool:?}: no {label:?} cell");
        }
    }
    let mut app = app();
    app.ui.tool = Tool::Patch;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1600.0, 300.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    assert!(h.query_all_by_label("Lasso around an area, then drag the selection").next().is_some(), "the Patch arm's hint is where it was");
    assert!(h.query_all_by_label("Patch").next().is_some(), "the Patch arm's own picker is where it was");
    assert!(h.query_all_by_label("Remove after each stroke").next().is_none(), "no Remove cell leaked into the Patch bar");
}
