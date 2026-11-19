//! #11765: the Count tool's options bar shows every control from the start, in Photoshop's order,
//! with its tooltips, and greys what can't act: the group dropdown until there is a group, the
//! trash with fewer than two groups, Clear while the current group is empty or hidden.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const EYE: &str = "Toggle visibility of count group";
const FOLDER: &str = "Create a new count group";
const TRASH: &str = "Delete the currently selected count group";

/// A 100 × 100 document with the Count tool and `groups` count groups (the first with a marker).
fn app(groups: usize) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.ui.tool = Tool::Count;
    if groups > 0 {
        app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
    }
    for _ in 1..groups {
        app.run("count.newGroup", json!({})).unwrap();
    }
    if groups > 0 {
        app.run("count.setGroup", json!({"group": 0, "active": true})).unwrap();
    }
    app
}

fn harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(vec2(1400.0, 120.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h.run_steps(4);
    h
}

fn disabled(h: &Harness<'static, Box<PhotocraftApp>>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

fn combo_disabled(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.get_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the group dropdown").accesskit_node().is_disabled()
}

/// What the options bar greys, by the groups the document has: the group dropdown until there is a
/// group, the trash with fewer than two groups, Clear while the current group is hidden or empty.
/// The eye and the folder stay live in every row.
#[test]
fn count_bar_greys_what_cannot_act() {
    struct Row {
        groups: usize,
        hide_current: bool,
        /// The group made current after the setup, when not the first.
        current: Option<u64>,
        combo_disabled: bool,
        trash_disabled: bool,
        clear_disabled: bool,
    }
    let rows = [
        Row { groups: 0, hide_current: false, current: None, combo_disabled: true, trash_disabled: true, clear_disabled: true },
        Row { groups: 1, hide_current: false, current: None, combo_disabled: false, trash_disabled: true, clear_disabled: false },
        Row { groups: 1, hide_current: true, current: None, combo_disabled: false, trash_disabled: true, clear_disabled: true },
        Row { groups: 2, hide_current: false, current: None, combo_disabled: false, trash_disabled: false, clear_disabled: false },
        Row { groups: 2, hide_current: false, current: Some(1), combo_disabled: false, trash_disabled: false, clear_disabled: true },
    ];
    for (n, row) in rows.iter().enumerate() {
        let mut a = app(row.groups);
        if row.hide_current {
            a.run("count.setGroup", json!({"visible": false})).unwrap();
        }
        if let Some(group) = row.current {
            a.run("count.setGroup", json!({"group": group, "active": true})).unwrap();
        }
        let h = harness(a);
        let at = |what: &str| format!("row {n} ({} groups, hidden {}, current {:?}): {what}", row.groups, row.hide_current, row.current);
        assert_eq!(combo_disabled(&h), row.combo_disabled, "{}", at("group dropdown"));
        assert_eq!(disabled(&h, TRASH), row.trash_disabled, "{}", at("trash"));
        assert_eq!(disabled(&h, "Clear"), row.clear_disabled, "{}", at("Clear"));
        assert!(!disabled(&h, EYE) && !disabled(&h, FOLDER), "{}", at("eye and folder"));
    }
}

/// Drags the visible "Marker Size" caption `dx` points right in four moves.
fn scrub_marker_size(h: &mut Harness<'static, Box<PhotocraftApp>>, dx: f32) {
    let from = h
        .query_all(egui_kittest::kittest::by().label("Marker Size").include_labels())
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        .expect("the Marker Size caption")
        .rect()
        .center();
    h.hover_at(from);
    h.run();
    h.drag_at(from);
    h.run();
    for i in 1..=4 {
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.hover_at(from + vec2(dx / 4.0 * i as f32, 0.0));
        h.run();
    }
    h.drop_at(from + vec2(dx, 0.0));
    h.run_steps(2);
}

#[test]
fn one_size_scrub_is_one_history_step() {
    let mut h = harness(app(1));
    let steps = |h: &Harness<'static, Box<PhotocraftApp>>| h.state().session.active().unwrap().history.past_len();
    let before = steps(&h);
    scrub_marker_size(&mut h, 24.0);
    let size = h.state().session.active().unwrap().doc.measurement.count_groups[0].marker_size;
    assert!(size > 2, "the scrub moved Marker Size several steps: {size}");
    assert_eq!(steps(&h), before + 1, "one scrub, one Undo step");
    // Back left, as the first may have reached the top of the range.
    scrub_marker_size(&mut h, -24.0);
    let back = h.state().session.active().unwrap().doc.measurement.count_groups[0].marker_size;
    assert!(back < size, "the second scrub moved it back: {size} -> {back}");
    assert_eq!(steps(&h), before + 2, "a second scrub is a step of its own");
}
