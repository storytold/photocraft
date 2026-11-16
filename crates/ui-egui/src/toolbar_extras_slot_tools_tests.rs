//! #14620: the toolbar's "…" slot reaches the Extra Tools (the tools hidden in Edit › Toolbar,
//! in toolbar order). Its flyout lists "Edit Toolbar…" and then every extra tool; the slot shows
//! the current tool's icon, highlighted, while that tool is an extra tool; a left or right click
//! selects the last-used extra tool, and without extra tools a left click opens Edit Toolbar.

use egui::accesskit::{Role, Toggled};
use egui::{Event, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use crate::PhotocraftApp;
use crate::panels::toolbar::{extra_tools, extras_slot_tool, toolbar};
use crate::state::Tool;

const EDIT: &str = "Edit Toolbar…";

fn harness(hidden: &[&str]) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.edit_prefs(|p| p.toolbar.hidden = hidden.iter().map(|h| (*h).to_string()).collect());
    harness_for(app, vec2(400.0, 1800.0))
}

fn harness_for(mut app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    app.ui.tool = Tool::Brush;
    let mut h = Harness::builder().with_size(size).build_ui_state(|ui, app| toolbar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    h
}

/// The tooltip a tool's toolbar button carries: its name and letter.
fn tool_tip(tool: Tool) -> String {
    let key = tool.key();
    if key == '\0' { tool.label().to_string() } else { format!("{}  ({key})", tool.label()) }
}

/// Where the "…" slot is: the button named after an extra tool while one is current, else "•••".
fn slot(h: &Harness<'static, PhotocraftApp>, label: &str) -> Pos2 {
    h.query_all_by_role(Role::Button).find(|n| n.accesskit_node().label().as_deref() == Some(label)).map(|n| n.rect().center()).expect(label)
}

fn press(h: &mut Harness<'static, PhotocraftApp>, at: Pos2, button: PointerButton) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(Event::PointerButton { pos: at, button, pressed: false, modifiers: Default::default() });
    // A flyout that just opened is laid out invisibly first; its rows take clicks a frame later.
    h.run_steps(3);
}

fn flyout_open(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("toolbar-more-flyout-open"))).unwrap_or(false)
}

/// The flyout's rows, top to bottom: the centres of its clickable 26 point rows (its rows are
/// painted, not separate accessibility nodes).
fn rows(h: &Harness<'static, PhotocraftApp>) -> Vec<Pos2> {
    let Some(area) = h.ctx.memory(|m| m.area_rect(egui::Id::unique("toolbar-more-flyout"))) else { return Vec::new() };
    let mut rows: Vec<Pos2> = h.ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && (w.rect.height() - 26.0).abs() < 0.5 && area.contains_rect(w.rect))
            .map(|w| w.rect.center())
            .collect()
    });
    rows.sort_by(|a, b| a.y.total_cmp(&b.y));
    rows.dedup();
    rows
}

#[test]
fn extra_tools_are_the_hidden_tools_in_toolbar_order() {
    let hidden = ["count", "Frame", "not a tool", "HorizontalTypeMask"].map(String::from);
    let custom = |hidden: &[String]| photocraft_engine::prefs::ToolbarCustomization { hidden: hidden.to_vec(), ..Default::default() };
    assert_eq!(extra_tools(&custom(&hidden)), vec![Tool::Frame, Tool::Count, Tool::HorizontalTypeMask]);
    assert!(extra_tools(&custom(&[])).is_empty());
    let extras = [Tool::Frame, Tool::Count];
    assert_eq!(extras_slot_tool(Tool::Brush, &[], &extras), Some(Tool::Frame), "the first extra tool when none was used yet");
    assert_eq!(extras_slot_tool(Tool::Brush, &[Tool::Brush, Tool::Count, Tool::Frame], &extras), Some(Tool::Count), "the one used last");
    assert_eq!(extras_slot_tool(Tool::Brush, &[], &[]), None);
}

#[test]
fn the_slot_shows_the_current_extra_tool_highlighted() {
    let mut h = harness(&["Frame"]);
    let more = h.get_by_label(EDIT);
    assert_eq!(more.accesskit_node().toggled(), Some(Toggled::False), "\"•••\" while the tool is not an extra tool");
    // K picks Frame, which only the "…" slot shows now.
    h.state_mut().ui.tool = Tool::Frame;
    h.run_steps(2);
    let frame = tool_tip(Tool::Frame);
    let more = h.get_by_label(&frame);
    assert_eq!(more.accesskit_node().toggled(), Some(Toggled::True), "the Frame icon, highlighted");
    assert!(h.query_by_label(EDIT).is_none(), "no \"•••\" while Frame is current");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    assert!(h.query_by_label(EDIT).is_some(), "back to \"•••\"");
}

#[test]
fn a_left_click_selects_the_last_used_extra_tool() {
    let mut h = harness(&["Count", "Frame"]);
    let at = slot(&h, EDIT);
    press(&mut h, at, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Frame, "the first extra tool, in toolbar order");
    assert!(h.state().ui.dialogs.is_empty(), "Edit Toolbar doesn't open");
    assert!(!flyout_open(&h), "a left click opens no flyout");
    h.state_mut().ui.tool = Tool::Brush;
    h.state_mut().ui.group_tools = vec![Tool::Brush, Tool::Count, Tool::Frame];
    h.run_steps(2);
    press(&mut h, at, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Count, "the extra tool used last");
}

#[test]
fn a_left_click_without_extra_tools_still_opens_edit_toolbar() {
    let mut h = harness(&[]);
    let at = slot(&h, EDIT);
    press(&mut h, at, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert_eq!(h.state().ui.dialogs.len(), 1, "edit.toolbar opened its dialog");
}

#[test]
fn a_right_click_selects_the_extra_tool_and_lists_every_extra_tool() {
    let mut h = harness(&["Count", "Frame"]);
    let at = slot(&h, EDIT);
    press(&mut h, at, PointerButton::Secondary);
    assert_eq!(h.state().ui.tool, Tool::Frame, "the right click also selects the slot's tool");
    assert!(flyout_open(&h));
    assert_eq!(rows(&h).len(), 3, "Edit Toolbar…, then Frame and Count");
    // Picking a row selects that tool and closes the flyout: the rows are in toolbar order.
    let count = rows(&h)[2];
    press(&mut h, count, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Count);
    assert!(!flyout_open(&h));
    assert!(h.state().ui.dialogs.is_empty());
    // Count is current now, so the slot shows it; the second row is Frame, the first Edit Toolbar.
    let at = slot(&h, &tool_tip(Tool::Count));
    press(&mut h, at, PointerButton::Secondary);
    assert_eq!(h.state().ui.tool, Tool::Count, "the current extra tool stays");
    let frame = rows(&h)[1];
    press(&mut h, frame, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Frame);
    let at = slot(&h, &tool_tip(Tool::Frame));
    press(&mut h, at, PointerButton::Secondary);
    let edit = rows(&h)[0];
    press(&mut h, edit, PointerButton::Primary);
    assert_eq!(h.state().ui.dialogs.len(), 1, "Edit Toolbar… opened its dialog");
    assert!(!flyout_open(&h));
}

#[test]
fn without_extra_tools_the_flyout_has_only_edit_toolbar() {
    let mut h = harness(&[]);
    let at = slot(&h, EDIT);
    press(&mut h, at, PointerButton::Secondary);
    assert!(flyout_open(&h));
    assert_eq!(rows(&h).len(), 1, "only Edit Toolbar…");
    assert_eq!(h.state().ui.tool, Tool::Brush);
    // Escape closes it without running anything.
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    assert!(!flyout_open(&h));
    assert!(h.state().ui.dialogs.is_empty());
}

/// CodeRabbit on #14653: the slot reads the toolbar's resolved layout. `edit.toolbar` with `extras`
/// moves Brush out of the toolbar into the stored extras and clears `hidden`, so a slot that read only
/// `hidden` found no extra tool and Brush was in neither place.
#[test]
fn a_tool_edit_toolbar_stored_in_the_extras_reaches_the_slot() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("edit.toolbar", serde_json::json!({"extras": [["Brush"]]})).unwrap();
    assert!(app.session.prefs().toolbar.hidden.is_empty(), "the writer clears `hidden`");
    assert_eq!(extra_tools(&app.session.prefs().toolbar), vec![Tool::Brush]);
    let mut h = harness_for(app, vec2(400.0, 1800.0));
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    let tip = tool_tip(Tool::Brush);
    let more = h.get_by_label(&tip);
    assert_eq!(more.accesskit_node().toggled(), Some(Toggled::True), "the slot shows Brush, the current extra tool");
}

/// CodeRabbit on #14653: with 40 extra tools the flyout (41 rows of 26 points) is taller than a 600 point
/// window. It stays inside the window and scrolls, and its last tool can still be picked.
#[test]
fn a_flyout_taller_than_the_window_scrolls_and_reaches_the_last_tool() {
    let tools: Vec<Tool> = crate::panels::toolbar::layout_order(&Default::default()).into_iter().take(40).collect();
    assert_eq!(tools.len(), 40);
    let hidden: Vec<String> = tools.iter().map(|t| format!("{t:?}")).collect();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.edit_prefs(|p| p.toolbar.hidden = hidden.clone());
    let extras = extra_tools(&app.session.prefs().toolbar);
    assert_eq!(extras.len(), 40, "every hidden tool is an extra tool");
    let mut h = harness_for(app, vec2(400.0, 600.0));
    // Brush is current: when it is one of the extras the slot shows it, else "•••".
    let label = if extras.contains(&Tool::Brush) { tool_tip(Tool::Brush) } else { EDIT.to_string() };
    let at = slot(&h, &label);
    press(&mut h, at, PointerButton::Secondary);
    let area = h.ctx.memory(|m| m.area_rect(egui::Id::unique("toolbar-more-flyout"))).expect("flyout is open");
    assert!(h.ctx.content_rect().contains_rect(area), "the flyout fits the window: {area:?}");
    // Scroll to the end, then pick the bottom row.
    h.event(Event::PointerMoved(area.center()));
    h.run_steps(2);
    h.event(Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -5000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    h.run_steps(12);
    let last = rows(&h).pop().expect("a visible row");
    press(&mut h, last, PointerButton::Primary);
    // A 600 point window shows about 21 rows unscrolled; a tool from the end of the 40 was out of reach before.
    let picked = extras.iter().position(|t| *t == h.state().ui.tool).expect("an extra tool was picked");
    assert!(picked >= 25, "a tool from the end of the list was picked: row {picked} of {}", extras.len());
}
