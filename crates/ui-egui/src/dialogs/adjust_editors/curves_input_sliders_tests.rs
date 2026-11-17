//! The Curves dialog's black and white input sliders under the graph.
use super::*;
use egui::{Event, Modifiers, PointerButton};

fn cx(dialog: bool) -> EditorCx {
    EditorCx {
        mem: egui::Id::unique("curves-input-sliders-test"),
        hist: None,
        gray: false,
        swatches: [[0.0; 3], [1.0; 3]],
        gradients: Vec::new(),
        dialog,
        dynamic_color_sliders: false,
        isolated_curve: None,
    }
}

fn frame(ctx: &egui::Context, v: &mut Value, cx: &EditorCx, events: Vec<Event>) -> Edit {
    let mut edit = Edit::default();
    let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 800.0));
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| {
        edit = curves(ui, v, cx);
    });
    out.textures_delta.clear();
    edit
}

fn track(ctx: &egui::Context, cx: &EditorCx) -> Option<Rect> {
    ctx.data(|d| d.get_temp(cx.mem.with("curves-input-slider")))
}

fn at(track: Rect, level: f32) -> egui::Pos2 {
    pos2(track.left() + level / 255.0 * track.width(), track.center().y)
}

fn moved(p: egui::Pos2) -> Event {
    Event::PointerMoved(p)
}

fn button(p: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::default() }
}

struct Rig {
    ctx: egui::Context,
    cx: EditorCx,
    v: Value,
    track: Option<Rect>,
}

impl Rig {
    fn new(dialog: bool, v: Value) -> Self {
        let ctx = egui::Context::default();
        let cx = cx(dialog);
        let mut rig = Rig { ctx, cx, v, track: None };
        for _ in 0..2 {
            frame(&rig.ctx, &mut rig.v, &rig.cx, vec![]);
        }
        rig.track = track(&rig.ctx, &rig.cx);
        rig
    }
    /// Where `level` sits on the strip (or the spot below the graph when no strip exists).
    fn pos(&self, level: f32) -> egui::Pos2 {
        let graph: Rect = self.ctx.data(|d| d.get_temp(self.cx.mem.with("curves-graph"))).unwrap_or(Rect::ZERO);
        let tr = self.track.unwrap_or_else(|| Rect::from_min_max(pos2(graph.left(), graph.bottom() + 13.0), pos2(graph.right(), graph.bottom() + 24.0)));
        at(tr, level)
    }
    fn step(&mut self, events: Vec<Event>) -> Edit {
        frame(&self.ctx, &mut self.v, &self.cx, events)
    }
    /// Presses at `from`, drags to `to`; the Edit's `changed` is true if any frame changed.
    fn drag_to(&mut self, from: f32, to: f32) -> Edit {
        let (a, b) = (self.pos(from), self.pos(to));
        let mut changed = false;
        for events in [vec![moved(a)], vec![moved(a)], vec![button(a, true)], vec![], vec![moved(b)], vec![moved(b)]] {
            changed |= self.step(events).changed;
        }
        Edit { changed, ..Edit::default() }
    }
    fn release(&mut self, at_level: f32) -> Edit {
        let p = self.pos(at_level);
        let mut commit = false;
        let mut last = self.step(vec![button(p, false)]);
        commit |= last.commit;
        for _ in 0..2 {
            last = self.step(vec![]);
            commit |= last.commit;
        }
        Edit { commit, ..last }
    }
    fn points(&self) -> Vec<[f64; 2]> {
        self.v["points"].as_array().map(|a| a.iter().filter_map(|q| Some([q.get(0)?.as_f64()?, q.get(1)?.as_f64()?])).collect()).unwrap_or_default()
    }
}

#[test]
fn black_triangle_sets_first_input_and_keeps_output() {
    for (pts, out_b, out_w) in [(json!([[0, 0], [255, 255]]), 0.0, 255.0), (json!([[0, 30], [255, 220]]), 30.0, 220.0)] {
        let mut rig = Rig::new(true, json!({"points": pts}));
        assert!(rig.track.is_some(), "slider strip is shown in the dialog");
        rig.drag_to(0.0, 40.0);
        rig.release(40.0);
        assert_eq!(rig.points(), vec![[40.0, out_b], [255.0, out_w]]);
    }
}

#[test]
fn white_triangle_sets_last_input_and_keeps_output() {
    for (pts, out_b, out_w) in [(json!([[0, 0], [255, 255]]), 0.0, 255.0), (json!([[0, 30], [255, 220]]), 30.0, 220.0)] {
        let mut rig = Rig::new(true, json!({"points": pts}));
        rig.drag_to(255.0, 200.0);
        rig.release(200.0);
        assert_eq!(rig.points(), vec![[0.0, out_b], [200.0, out_w]]);
    }
}

#[test]
fn drag_is_limited_by_the_neighbouring_point() {
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [100, 120], [255, 255]]}));
    rig.drag_to(0.0, 200.0);
    rig.release(200.0);
    assert_eq!(rig.points(), vec![[96.0, 0.0], [100.0, 120.0], [255.0, 255.0]]);
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [100, 120], [255, 255]]}));
    rig.drag_to(255.0, 10.0);
    rig.release(10.0);
    assert_eq!(rig.points(), vec![[0.0, 0.0], [100.0, 120.0], [104.0, 255.0]]);
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [255, 255]]}));
    rig.drag_to(0.0, 255.0);
    rig.release(255.0);
    assert_eq!(rig.points(), vec![[251.0, 0.0], [255.0, 255.0]]);
}

#[test]
fn drag_selects_the_end_point_and_previews_then_commits_on_release() {
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [100, 120], [255, 255]]}));
    let edit = rig.drag_to(0.0, 40.0);
    assert!(edit.changed, "live preview while dragging");
    let st: CurveUi = rig.ctx.data(|d| d.get_temp(rig.cx.mem.with("curves"))).unwrap_or_default();
    assert_eq!(st.gesture.selected, Some(0));
    assert_eq!(rig.points()[0], [40.0, 0.0]);
    let edit = rig.release(40.0);
    assert!(edit.commit, "release commits");
    // The white triangle selects the last point.
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [100, 120], [255, 255]]}));
    rig.drag_to(255.0, 200.0);
    let st: CurveUi = rig.ctx.data(|d| d.get_temp(rig.cx.mem.with("curves"))).unwrap_or_default();
    assert_eq!(st.gesture.selected, Some(2));
}

#[test]
fn properties_panel_has_no_sliders() {
    let mut rig = Rig::new(false, json!({"points": [[0, 0], [255, 255]]}));
    assert!(rig.track.is_none());
    rig.drag_to(0.0, 40.0);
    rig.release(40.0);
    assert_ne!(rig.points().first().map(|q| q[0]), Some(40.0));
    assert_eq!(rig.points().first().map(|q| q[0]), Some(0.0));
}

#[test]
fn pencil_mode_has_no_sliders() {
    let mut rig = Rig::new(true, json!({"pencil": true, "points": [[0, 0], [255, 255]]}));
    assert!(rig.track.is_none());
    rig.drag_to(0.0, 40.0);
    rig.release(40.0);
    assert!(rig.points().first().is_none_or(|q| q[0] == 0.0));
}

#[test]
fn neighbour_closer_than_the_gap_does_not_panic() {
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [2, 10], [255, 255]]}));
    rig.drag_to(0.0, 200.0);
    rig.release(200.0);
    assert_eq!(rig.points().first().map(|q| q[0]), Some(0.0));
    assert!(input_slider_value(&[[0.0, 0.0], [2.0, 10.0], [255.0, 255.0]], 1, 0.0).is_some());
    assert_eq!(input_slider_value(&[[0.0, 0.0], [255.0, 255.0]], 1, f32::NAN), Some(4.0));
    assert_eq!(input_slider_value(&[[0.0, 0.0]], 0, 5.0), None);
    assert_eq!(input_slider_value(&[[0.0, 0.0], [254.0, 255.0], [255.0, 255.0]], 1, 0.0), Some(255.0));
}

#[test]
fn a_release_in_the_same_frame_as_the_last_move_commits_where_the_button_came_up() {
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [255, 255]]}));
    rig.drag_to(0.0, 40.0);
    assert_eq!(rig.points()[0], [40.0, 0.0]);
    let p = rig.pos(80.0);
    let edit = rig.step(vec![moved(p), button(p, false)]);
    assert!(edit.commit, "release commits");
    assert_eq!(rig.points(), vec![[80.0, 0.0], [255.0, 255.0]]);
}

#[test]
fn switching_channels_during_a_held_drag_leaves_the_new_channel_alone() {
    let mut rig = Rig::new(true, json!({"points": [[0, 0], [255, 255]]}));
    rig.drag_to(0.0, 40.0);
    assert_eq!(rig.points()[0], [40.0, 0.0]);
    // Alt+3 selects Red while the button is still down.
    let alt3 = Event::Key { key: egui::Key::Num3, physical_key: Some(egui::Key::Num3), pressed: true, repeat: false, modifiers: Modifiers::ALT };
    let edit = rig.step(vec![alt3]);
    assert!(edit.commit, "the composite drag ends when the channel changes");
    assert_eq!(rig.ctx.data(|d| d.get_temp::<usize>(rig.cx.mem.with("curves-ch"))), Some(1));
    for level in [60.0, 90.0] {
        let p = rig.pos(level);
        rig.step(vec![moved(p)]);
        rig.step(vec![]);
    }
    let p = rig.pos(90.0);
    rig.step(vec![button(p, false)]);
    rig.step(vec![]);
    let red = read_curve(&rig.v, "red");
    assert_eq!(red.first().map(|q| q[0]), Some(0.0), "Red's black input moves only after a new press");
    assert_eq!(rig.points()[0], [40.0, 0.0], "the composite keeps where its drag ended");
}
