//! #12545: the Remove options bar always ends with Reset all strokes and Apply, greyed until strokes
//! wait; the Remove after each stroke box is greyed while strokes wait; and so is the Layers panel,
//! which cannot change the active layer until Apply or Reset all strokes answers the strokes.

use egui::accesskit::Role;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::Session;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::layers_panel_nodes::{click, layer_id, name_point};
use crate::state::Tool;

/// The bar's two answer cells, in the reference order.
const RESET: &str = "Reset all strokes";
const APPLY: &str = "Apply to current strokes";

/// A settled options bar over an 80 x 80 document with `tool`.
fn bar_for(tool: Tool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
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
    h
}

/// The Remove tool's settled options bar.
fn remove_bar() -> Harness<'static, Box<PhotocraftApp>> {
    bar_for(Tool::Remove)
}

/// The whole app over a document with "L0" and "L1" over the Background.
fn full_app() -> Harness<'static, Box<PhotocraftApp>> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for name in ["L0", "L1"] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
    }
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    h
}

/// Remove after each stroke off, one stroke: it waits instead of being removed at once.
fn wait_one_stroke(app: &mut PhotocraftApp) {
    app.ui.tool_options.remove_after_each_stroke = false;
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 30.0, y: 18.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 30.0, y: 42.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 30.0, y: 42.0 }, m);
}

/// Two strokes waiting, the second beside the first.
fn wait_two_strokes(app: &mut PhotocraftApp) {
    wait_one_stroke(app);
    let m = Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: 50.0, y: 18.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: 50.0, y: 42.0, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: 50.0, y: 42.0 }, m);
}

/// The bar's button named `label`; panics when the bar shows no such button.
fn button<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(Role::Button)
        .find(|n| n.accesskit_node().label().as_deref() == Some(label))
        .unwrap_or_else(|| panic!("the Remove bar shows no {label:?} button"))
}

/// Whether the bar drew its button named `label` greyed.
fn greyed(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> bool {
    button(h, label).accesskit_node().is_disabled()
}

/// The x of the bar's button named `label`.
fn button_left(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> f32 {
    button(h, label).rect().left()
}

fn steps(h: &PhotocraftApp) -> usize {
    h.session.active().expect("a document").history.past_len()
}

/// The whole app's active layer.
fn active_layer(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<photocraft_doc::LayerId> {
    h.state().session.active().expect("a document").active_layer
}

/// Nothing is waiting, so both cells stand on the bar greyed.
#[test]
fn the_bar_shows_both_answer_cells_greyed_before_any_stroke() {
    let h = remove_bar();
    assert!(greyed(&h, RESET), "Reset all strokes is drawn greyed with nothing waiting");
    assert!(greyed(&h, APPLY), "Apply is drawn greyed with nothing waiting");
}

/// One waiting stroke enables both; the answer greys them again.
#[test]
fn a_waiting_stroke_enables_both_cells_and_the_answer_greys_them_again() {
    let mut h = remove_bar();
    wait_one_stroke(h.state_mut());
    h.run_steps(2);
    assert!(!greyed(&h, RESET) && !greyed(&h, APPLY), "both cells answer while a stroke waits");
    button(&h, RESET).click();
    h.run_steps(2);
    assert!(greyed(&h, RESET) && greyed(&h, APPLY), "and go grey again once nothing waits");
}

/// The two cells end the bar: after every checkbox, Reset all strokes before Apply.
#[test]
fn the_pair_ends_the_bar_with_reset_before_apply() {
    let h = remove_bar();
    let last_box = h.query_all_by_role(Role::CheckBox).map(|n| n.rect().right()).fold(f32::MIN, f32::max);
    let (reset, apply) = (button_left(&h, RESET), button_left(&h, APPLY));
    assert!(reset > last_box, "the answer cells come after the bar's boxes ({reset} vs {last_box})");
    assert!(reset < apply, "{RESET} stands before {APPLY}");
    let mut named: Vec<(f32, String)> = h.query_all_by_role(Role::Button).map(|n| (n.rect().left(), n.accesskit_node().label().unwrap_or_default())).collect();
    named.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut ends: Vec<&str> = named.iter().rev().take(2).map(|(_, l)| l.as_str()).collect();
    ends.reverse();
    assert_eq!(ends, [RESET, APPLY], "they are the bar's last two members");
}

/// Apply takes the waiting strokes as one removal, one history step; the bar goes grey again.
#[test]
fn apply_click_removes_the_waiting_strokes_as_one_history_step() {
    let mut h = remove_bar();
    wait_two_strokes(h.state_mut());
    h.run_steps(2);
    assert_eq!(h.state().remove_pending.len(), 2, "two strokes wait");
    let before = steps(h.state());
    button(&h, APPLY).click();
    h.run_steps(3);
    let app = h.state();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(app.remove_pending.is_empty(), "nothing waits after Apply");
    assert_eq!(steps(app), before + 1, "one history step for both strokes");
    let (id, p) = app.session.journal.last().cloned().expect("a journaled removal");
    assert_eq!(id.as_str(), "paint.remove", "the removal command runs once");
    assert_eq!(p["strokes"].as_array().map(Vec::len), Some(2), "carrying both strokes");
    assert!(greyed(&h, APPLY) && greyed(&h, RESET), "the cells go grey again");
}

/// Reset all strokes drops the waiting strokes and adds no history step.
#[test]
fn reset_click_drops_the_waiting_strokes_without_a_history_step() {
    let mut h = remove_bar();
    wait_two_strokes(h.state_mut());
    h.run_steps(2);
    let before = steps(h.state());
    let journal = h.state().session.journal.len();
    button(&h, RESET).click();
    h.run_steps(3);
    let app = h.state();
    assert!(app.remove_pending.is_empty() && app.remove_trail.is_none(), "the strokes and their tint are gone");
    assert_eq!(steps(app), before, "no history step");
    assert_eq!(app.session.journal.len(), journal, "and no command at all");
    assert!(greyed(&h, APPLY) && greyed(&h, RESET), "the cells go grey again");
}

/// Greyed cells answer nothing, by key or by pointer: neither the commit chord, the Delete key nor
/// the cancel chord removes or drops anything while the pair stands there greyed.
#[test]
fn the_greyed_pair_answers_nothing_when_nothing_waits() {
    let mut h = remove_bar();
    let before = steps(h.state());
    h.key_press(egui::Key::Enter);
    h.key_press(egui::Key::Delete);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(steps(h.state()), before, "none of Enter, Delete or Escape removes anything");
    assert!(h.state().remove_pending.is_empty(), "and none of them queues or drops a stroke");
    button(&h, APPLY).click();
    h.run_steps(2);
    button(&h, RESET).click();
    h.run_steps(2);
    assert_eq!(steps(h.state()), before, "and neither does a press on either greyed cell");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
}

/// Remove after each stroke is greyed while strokes wait and free again after the answer; the
/// other boxes keep today's behaviour.
#[test]
fn remove_after_each_stroke_is_greyed_while_strokes_wait() {
    let mut h = remove_bar();
    let box_named = |h: &Harness<'static, Box<PhotocraftApp>>, name: &str| {
        h.query_all_by_role(Role::CheckBox)
            .find(|n| n.accesskit_node().label().as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no {name:?} box"))
            .accesskit_node()
            .is_disabled()
    };
    assert!(!box_named(&h, "Remove after each stroke"), "the box is free before any stroke waits");
    wait_one_stroke(h.state_mut());
    h.run_steps(2);
    assert!(box_named(&h, "Remove after each stroke"), "greyed while a stroke waits");
    for name in ["Sample all layers", "Create new layer"] {
        assert!(!box_named(&h, name), "{name} keeps today's behaviour while strokes wait");
    }
    button(&h, APPLY).click();
    h.run_steps(2);
    assert!(!box_named(&h, "Remove after each stroke"), "free again after Apply");
}

/// Whether the Layers panel reports its row named `name` as disabled.
fn row_greyed(h: &Harness<'static, Box<PhotocraftApp>>, name: &str) -> bool {
    h.query_all_by_label(name)
        .find(|n| n.accesskit_node().role() == Role::TreeItem)
        .map(|n| n.accesskit_node().is_disabled())
        .unwrap_or_else(|| panic!("no Layers panel row named {name:?}"))
}

/// The Layers panel is shut while strokes wait and takes no pick of another layer; the bar's answer
/// makes it usable again.
#[test]
fn the_layers_panel_takes_no_pick_while_strokes_wait() {
    let mut h = full_app();
    let (l0, l1) = {
        let app = h.state();
        (layer_id(app, "L0"), layer_id(app, "L1"))
    };
    assert_eq!(active_layer(&h), Some(l1), "the newest layer is active");
    assert!(!row_greyed(&h, "L0"), "the panel is free before strokes wait");
    h.state_mut().ui.tool = Tool::Remove;
    wait_one_stroke(h.state_mut());
    h.run_steps(3);

    let at = name_point(&h, "L0");
    assert!(row_greyed(&h, "L0"), "the panel says its rows are disabled while strokes wait");
    click(&mut h, at, Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(active_layer(&h), Some(l1), "a pick on the shut panel changes nothing");
    let before = steps(h.state());

    // Reset all strokes runs no command and leaves the document's revision alone: the panel body
    // re-runs only because its version holds the waiting state.
    button(&h, RESET).click();
    h.run_steps(3);
    assert_eq!(h.state().remove_pending.len(), 0, "Reset all strokes answered the strokes");
    assert_eq!(steps(h.state()), before, "and ran no command");
    assert!(!row_greyed(&h, "L0"), "and the panel says its rows are free again");
    click(&mut h, at, Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(active_layer(&h), Some(l0), "and the panel takes the pick again");
}

/// Neighbour case: Content-Aware Move keeps today's bar - its answer pair appears only while its
/// transform box is up, and the Remove pair does not leak into it.
#[test]
fn content_aware_move_shows_its_pair_only_while_the_box_is_up() {
    let h = bar_for(Tool::ContentAwareMove);
    let shown: Vec<String> = h.query_all_by_role(Role::Button).map(|n| n.accesskit_node().label().unwrap_or_default()).collect();
    for label in ["Commit transform", "Cancel transform", RESET, APPLY] {
        assert!(!shown.iter().any(|l| l == label), "{label} is not on the Content-Aware Move bar before its box is up: {shown:?}");
    }
}
