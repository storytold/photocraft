//! The canvas Brush Preset picker runs only what changes: open with nothing changing, the whole picker replays (with the
//! document area around it), its popover and the focused search field included, and none of its components runs; a Size drag runs the head and replays the list on every move; a preset rename or a new preset
//! builds the rows once, and the lists settle.

use craft::ui::runtime::measure::{MemoRun, last_frame_derived_builds, last_frame_memo_runs, last_pass_memo_runs, last_pass_runs};
use egui::accesskit::Role;
use egui::{Event, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app with a document and the Brush tool, its canvas picker open and settled.
fn opened() -> H {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), session, crate::Services::default());
        app.ui.tool = Tool::Brush;
        app
    });
    h.run_steps(8);
    let at = h.state().last_canvas_rect.center();
    crate::brush_picker::open(h.state_mut(), [at.x, at.y]);
    h.run_steps(8);
    h
}

/// The memoized views whose props type is named `props` that took part in the last frame as `how`, by id.
fn views(h: &H, props: &str, how: MemoRun) -> Vec<egui::Id> {
    last_frame_memo_runs(&h.ctx).into_iter().filter(|(name, _, run)| name.contains(props) && *run == how).map(|(_, id, _)| id).collect()
}

/// The list rows that ran in the last pass.
fn list_rows(h: &H) -> u32 {
    last_pass_runs(&h.ctx).iter().filter(|(ty, _)| ty.ends_with("::ListRow")).map(|(_, runs)| runs).sum()
}

/// The picker's Size slider.
fn size_slider(h: &H) -> egui::Rect {
    let slider = h.query_all_by_role(Role::Slider).find(|n| n.accesskit_node().label().is_some_and(|label| label.starts_with("Size")));
    slider.expect("the picker's Size slider shows").rect()
}

fn press(h: &mut H, at: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
}

/// The components of the picker's popover chrome and search field that ran in the last pass.
fn chrome_runs(h: &H) -> u32 {
    let picker = |ty: &str| ["::Popover", "::TextField", "::Stack", "::ListRow", "::Bar"].iter().any(|end| ty.ends_with(end));
    last_pass_runs(&h.ctx).iter().filter(|(ty, _)| picker(ty)).map(|(_, runs)| runs).sum()
}

#[test]
fn an_idle_open_picker_runs_no_component() {
    let mut h = opened();
    assert!(crate::brush_picker::opened_at(h.state()).is_some(), "the picker is open");
    for frame in 0..3 {
        h.run_steps(1);
        // The picker replays in its own view, or whole in the document area's view or the docked tree around it.
        let holders = ["brush_picker::CanvasProps<", "region_versions::CanvasSlot<", "area::TreeView<"];
        assert!(holders.iter().all(|props| views(&h, props, MemoRun::Ran).is_empty()), "idle frame {frame} runs no picker");
        assert!(holders.iter().any(|props| !views(&h, props, MemoRun::Replayed).is_empty()), "idle frame {frame} replays the picker");
        assert_eq!(chrome_runs(&h), 0, "idle frame {frame} runs none of the picker's components: {:?}", last_pass_runs(&h.ctx));
    }
    assert!(h.query_all_by_role(Role::Slider).any(|n| n.accesskit_node().label().is_some_and(|label| label.starts_with("Size"))));
}

#[test]
fn a_picker_size_drag_runs_the_head_and_replays_the_preset_list_on_every_move() {
    let mut h = opened();
    let size = size_slider(&h);
    let from = egui::pos2(size.left() + size.width() * 0.2, size.center().y);
    let before = h.state().session.tools.brush.size;
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    press(&mut h, from, true);
    h.run_steps(1);
    let mut at = from;
    for step in 1..=5 {
        at += vec2(3.0, 0.0);
        h.event(Event::PointerMoved(at));
        h.run_steps(1);
        assert!(!views(&h, "HeadProps", MemoRun::Ran).is_empty(), "move {step} runs the head");
        assert!(views(&h, "ListProps", MemoRun::Ran).is_empty(), "move {step} runs no preset list");
        assert!(!views(&h, "ListProps", MemoRun::Replayed).is_empty(), "move {step} replays the preset list");
        assert_eq!(list_rows(&h), 0, "move {step} runs no list row");
    }
    press(&mut h, at, false);
    h.run_steps(1);
    assert_ne!(h.state().session.tools.brush.size, before, "the drag set the brush size");
}

#[test]
fn renaming_or_adding_a_brush_preset_builds_the_rows_once_and_settles() {
    let mut h = opened();
    let name = h.state().session.tools.presets.first().expect("a preset").name.clone();
    // The frames each edit takes to settle: an added row grows the popover in the frame after it, and the views whose
    // room that changes run once more.
    let edits = [("brush.presets.rename", json!({"name": name, "newName": "Renamed Brush"}), 1), ("brush.presets.save", json!({"name": "Added Brush"}), 2)];
    for (command, params, settle) in edits {
        h.state_mut().run(command, params).unwrap_or_else(|e| panic!("{command}: {e}"));
        h.run_steps(1);
        assert!(!views(&h, "ListProps", MemoRun::Ran).is_empty(), "{command}: the frame after it runs the picker's list");
        // The real runs of the last pass: a popover's sizing calls its list's view again when the rows change.
        let ran: Vec<egui::Id> =
            last_pass_memo_runs(&h.ctx).into_iter().filter(|(name, _, run)| name.contains("ListProps") && *run == MemoRun::Ran).map(|(_, id, _)| id).collect();
        let mut once = ran.clone();
        once.sort_by_key(|id| id.value());
        once.dedup();
        assert_eq!(once.len(), ran.len(), "{command}: each list runs once in the pass");
        let built = last_frame_derived_builds(&h.ctx);
        assert!(built.iter().all(|(_, n)| *n == 1), "{command}: each derived value is built once in the frame: {built:?}");
        for frame in 1..=settle {
            h.run_steps(1);
            let again: Vec<_> = last_frame_derived_builds(&h.ctx).into_iter().filter(|(id, _)| built.iter().any(|(b, _)| b == id)).collect();
            assert!(again.is_empty(), "{command}: frame {frame} after it builds none of them again: {again:?}");
        }
        assert!(views(&h, "ListProps", MemoRun::Ran).is_empty(), "{command}: {settle} frames after it every list replays");
    }
}
