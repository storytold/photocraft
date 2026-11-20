//! Count trials for value drags in the whole app (#15859): a drag runs the regions that show the dragged value and
//! leaves every other settled region replaying, each move is one pass, the value follows the pointer in the frame it
//! moves, and the drag is one command.

use std::collections::HashSet;

use craft::ui::runtime::measure::{Settle, last_pass_settles};
use egui::accesskit::Role;
use egui::{Event, Id, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The whole app on a new document with the Brush, and an adjustment layer of `kind` when one is asked for.
fn harness(kind: Option<&'static str>) -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        if let Some(kind) = kind {
            s.execute(&format!("layer.newAdjustmentLayer.{kind}"), json!({})).unwrap();
        }
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    h
}

fn node_rect(h: &H, role: Role, label: &str) -> Option<Rect> {
    h.query_all_by_role(role).find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label)).map(|n| n.rect())
}

/// The value the spin field `label` inside `within` shows.
fn field_value(h: &H, label: &str, within: Rect) -> Option<f64> {
    h.query_all_by_role(Role::SpinButton)
        .find(|n| n.accesskit_node().label().is_some_and(|l| l.trim_end_matches(':') == label) && within.contains(n.rect().center()))
        .and_then(|n| n.accesskit_node().numeric_value().or_else(|| n.accesskit_node().value().and_then(|v| v.trim_end_matches(" px").parse().ok())))
}

/// The settling regions the last pass replayed.
fn replayed(h: &H) -> HashSet<Id> {
    last_pass_settles(&h.ctx).into_iter().filter(|(_, s)| *s == Settle::Replayed).map(|(id, _)| id).collect()
}

/// What one drag move did: the regions it replayed, the passes it ran and `read` after it.
struct Move<T> {
    replayed: HashSet<Id>,
    passes: u64,
    read: T,
}

/// The regions a hover frame over `at` replays, then each drag move from `at` by `dx`.
fn drag<T>(h: &mut H, at: Pos2, dx: f32, moves: usize, read: impl Fn(&H) -> T) -> (HashSet<Id>, Vec<Move<T>>) {
    h.event(Event::PointerMoved(at));
    h.run_steps(4);
    h.event(Event::PointerMoved(at + vec2(0.0, 1.0)));
    h.run_steps(1);
    let hover = replayed(h);
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    let mut moves_made = Vec::new();
    let mut to = at;
    for _ in 0..moves {
        to += vec2(dx, 0.0);
        h.event(Event::PointerMoved(to));
        let before = h.ctx.cumulative_pass_nr();
        h.run_steps(1);
        moves_made.push(Move { replayed: replayed(h), passes: h.ctx.cumulative_pass_nr() - before, read: read(h) });
    }
    h.event(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
    (hover, moves_made)
}

/// The canvas picker's Size slider: the brush size follows each move in that frame, each move is one pass (the
/// options bar's Brush Preset chip holds its width as its caption gains digits), and only the options bar, which
/// shows the size, leaves the regions a hover replays (before #15859 every region ran on every move). The drag is
/// one `tools.setBrush` and no history step.
#[test]
fn a_picker_size_drag_runs_only_the_regions_that_show_the_brush() {
    let mut h = harness(None);
    let c = h.state().last_canvas_rect.center();
    crate::brush_picker::open(h.state_mut(), [c.x, c.y]);
    h.run_steps(4);
    let picker = node_rect(&h, Role::Slider, "Size").expect("the picker's Size slider");
    let (journal, steps) = (h.state().session.journal.len(), h.state().session.active().unwrap().history.past_len());
    let from = egui::pos2(picker.left() + picker.width() * 0.2, picker.center().y);
    // From a size of one digit to three: the chip's caption gains digits on the way.
    let (hover, moves) = drag(&mut h, from, 3.0, 30, |h| h.state().session.tools.brush.size);
    assert!(!hover.is_empty(), "a hover frame replays the settled regions");
    let sizes: Vec<f32> = moves.iter().map(|m| m.read).collect();
    assert!(sizes.iter().any(|&s| s < 10.0) && sizes.iter().any(|&s| s >= 100.0), "the drag spans one to three digits: {sizes:?}");
    // The first moves pass egui's drag threshold.
    for (i, m) in moves.iter().enumerate().skip(3) {
        let left: Vec<&Id> = hover.difference(&m.replayed).collect();
        assert!(left.len() <= 1, "move {i}: only the options bar (it shows the brush size) leaves the regions a hover replays: {left:?}");
        assert_eq!(m.passes, 1, "move {i}: the move is one pass: {sizes:?}");
        assert_ne!(m.read, moves[i - 1].read, "move {i}: the size follows the pointer every move: {sizes:?}");
    }
    let state = h.state();
    let ids: Vec<&str> = state.session.journal[journal..].iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["tools.setBrush"], "the drag is one command");
    assert_eq!(state.session.active().unwrap().history.past_len(), steps, "a brush edit is no history step");
}

/// A Properties slider of an adjustment layer: its field shows the slider's value each move, each move is one pass,
/// and no region other than Properties and the canvas runs for it (before #15859 every region ran on every move).
#[test]
fn a_properties_slider_drag_leaves_the_other_regions_settled() {
    let mut h = harness(Some("brightnessContrast"));
    let slider = node_rect(&h, Role::Slider, "Brightness").expect("the Brightness slider");
    let row = Rect::from_min_max(egui::pos2(slider.left() - 400.0, slider.top() - 40.0), egui::pos2(slider.right() + 400.0, slider.bottom()));
    let (hover, moves) = drag(&mut h, slider.center(), 3.0, 12, |h| field_value(h, "Brightness", row));
    assert!(!hover.is_empty(), "a hover frame replays the settled regions");
    let shown: Vec<Option<f64>> = moves.iter().map(|m| m.read).collect();
    for (i, m) in moves.iter().enumerate().skip(3) {
        let left: Vec<&Id> = hover.difference(&m.replayed).collect();
        assert!(left.len() <= 1, "move {i}: only the canvas leaves the regions a hover replays: {left:?}");
        assert_eq!(m.passes, 1, "move {i}: the move is one pass");
        assert!(m.read.is_some_and(|v| v > 0.0), "move {i}: the Brightness field follows the slider: {shown:?}");
    }
    assert!(shown.windows(2).skip(3).all(|w| w[0] != w[1]), "the field changes every move: {shown:?}");
}
