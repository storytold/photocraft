//! Frame-level tests for the #4096 rework: the peek through `PhotocraftApp`, with the
//! canvas drawn first (as `ui_body` does) and the peek hook after it. Presence is read from
//! the strip the peek recorded — `compute_peek` clears a strip the frame its peek ends,
//! so a strip is present exactly when the peek drew — and geometry from the dock's probe
//! rects. The hide state is set directly ([`set_hide`]): the shell owns the Tab and
//! Shift+Tab toggles, and the peek only reads the state the shell pass recorded.

use craft::behavior::dock::Action;
use egui::{Modifiers, PointerButton, Pos2, Rect, accesskit::Role, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use super::*;
use crate::PhotocraftApp;
use crate::panel_docking::probe;

/// The canvas plus the peek hook, in the order `ui_body` runs them: the canvas sets
/// `last_canvas_rect`, then the peek computes and draws.
fn harness(doc_width: f32, doc_height: f32) -> Harness<'static, Box<PhotocraftApp>> {
    build(doc_width, doc_height, false)
}

/// A spot on the canvas, left of any peeked dock, with a context menu of its own.
const CANVAS_MENU: Rect = Rect { min: Pos2 { x: 60.0, y: 300.0 }, max: Pos2 { x: 140.0, y: 380.0 } };

/// [`harness`] plus a context menu on the canvas at [`CANVAS_MENU`], drawn with it.
fn harness_with_canvas_menu(doc_width: f32, doc_height: f32) -> Harness<'static, Box<PhotocraftApp>> {
    build(doc_width, doc_height, true)
}

fn build(doc_width: f32, doc_height: f32, canvas_menu: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": doc_width as i64, "height": doc_height as i64})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        move |ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                crate::canvas::document_area(app, ui);
                if canvas_menu {
                    ui.interact(CANVAS_MENU, egui::Id::unique("test-canvas-menu"), egui::Sense::click()).context_menu(|ui| {
                        ui.label("canvas menu");
                    });
                }
            });
            let peek = compute_peek(app, ui.ctx(), true);
            if peek.toolbar || peek.dock {
                draw_peek(app, ui.ctx(), &peek);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

fn canvas_rect(h: &Harness<'static, Box<PhotocraftApp>>) -> Rect {
    let rect = h.state().last_canvas_rect;
    assert!(rect.is_positive(), "the canvas drew: {rect:?}");
    rect
}

fn dock_strip(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<Rect> {
    h.ctx.data(|d| d.get_temp::<Option<Rect>>(dock_strip_id())).flatten()
}

fn toolbar_strip(h: &Harness<'static, Box<PhotocraftApp>>) -> Option<Rect> {
    h.ctx.data(|d| d.get_temp::<Option<Rect>>(toolbar_strip_id())).flatten()
}

fn hover(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
}

fn press(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2) {
    h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn release(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2) {
    h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// Sets the hide state the shell pass would have recorded: Tab hid everything, Shift+Tab
/// hid the panels, or nothing is hidden. The peek reads it on the next frame.
fn set_hide(h: &mut Harness<'static, Box<PhotocraftApp>>, hide: HideState) {
    record_hide_state(&h.ctx, hide);
    h.run_steps(1);
}

/// Tab hid everything.
fn hide_tab(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    set_hide(h, HideState { tab: true, shift: false, revealed: false });
}

/// Shift+Tab hid the panels, keeping the tools.
fn hide_shift(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    set_hide(h, HideState { tab: false, shift: true, revealed: false });
}

/// Nothing is hidden: the panels show.
fn hide_none(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    set_hide(h, HideState::default());
}

/// The dock column rect the peek should draw: the peek layout's column width, clamped to
/// the dock width range and the canvas, anchored at the canvas rect's end edge.
fn expected_dock_rect(h: &Harness<'static, Box<PhotocraftApp>>, rtl: bool) -> Rect {
    let canvas = canvas_rect(h);
    let (_, col_width) = peek_layout(h.state()).expect("a dock column to peek");
    let range = dock_width_range(h.ctx.content_rect().width());
    let width = col_width.clamp(*range.start(), *range.end()).min(canvas.width());
    if rtl {
        Rect::from_min_max(pos2(canvas.left(), canvas.top()), pos2(canvas.left() + width, canvas.bottom()))
    } else {
        Rect::from_min_max(pos2(canvas.right() - width, canvas.top()), pos2(canvas.right(), canvas.bottom()))
    }
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!((actual - expected).abs() <= 2.0, "{what}: {actual} vs {expected}");
}

#[test]
fn edge_trigger_draws_dock_column_at_canvas_edge_without_moving_canvas() {
    let mut h = harness(400.0, 300.0);
    hide_shift(&mut h);
    let before = canvas_rect(&h);
    assert!(dock_strip(&h).is_none(), "nothing peeks before the pointer reaches the edge");

    hover(&mut h, pos2(before.right() - 2.0, before.center().y));
    // An Area's first frame lays its content out before its positioned rect is known; the
    // second frame lands it. Settle before asserting geometry.
    h.run_steps(3);
    let strip = dock_strip(&h).expect("the edge hover peeks the dock");
    let expected = expected_dock_rect(&h, false);
    assert_close(strip.left(), expected.left(), "peek left");
    assert_close(strip.right(), expected.right(), "peek right");
    assert_close(strip.top(), expected.top(), "peek top");
    assert_close(strip.bottom(), expected.bottom(), "peek bottom");
    let group = probe::group_rect(&h, "layers").expect("the peeked layers group drew");
    // The column is anchored at the canvas edge; the layers group is the bottom group of
    // the dock's expanded column, so it shares the column's right and bottom and starts
    // below the groups above it. The dock has a second column beside the canvas (the
    // iconic column), so the group's left sits inside the dock, past that column.
    assert!(group.left() > expected.left(), "group left inside the dock: {group:?} in {expected:?}");
    assert_close(group.right(), expected.right(), "group right");
    assert_close(group.bottom(), expected.bottom(), "group bottom");
    assert!(group.top() > expected.top() && group.top() < expected.bottom(), "group stacked inside the column: {group:?} in {expected:?}");

    // The canvas itself never moves: the peek is an overlay, not a layout change.
    assert_eq!(canvas_rect(&h), before, "the canvas rect is unchanged by the peek");

    // The middle reveals nothing.
    hover(&mut h, before.center());
    assert!(dock_strip(&h).is_none(), "the middle ends the peek");
}

#[test]
fn scrollbar_and_ruler_rules() {
    // This app's bars always draw while a document exists (even when it fits), so the
    // vertical bar is present at the right edge here; the horizontal bar does not touch it.
    // The peek computes the bar rect from the canvas itself, so no scrollbar flags are
    // asserted: the geometry below is the bar rule.
    let mut h = harness(1000.0, 1500.0);
    let canvas = canvas_rect(&h);
    hide_shift(&mut h);

    // The outermost points always trigger, even over the bar.
    hover(&mut h, pos2(canvas.right() - 2.0, canvas.center().y));
    assert!(dock_strip(&h).is_some(), "right-2 triggers over the bar");
    // End the peek first: a pointer over the strip it drew holds it, so the block rule
    // can only be probed from a cold start.
    hover(&mut h, canvas.center());
    assert!(dock_strip(&h).is_none(), "the middle ends the peek");
    // Inside the 12-point bar, past the 3-point always-edge, the bar blocks.
    hover(&mut h, pos2(canvas.right() - 6.0, canvas.center().y));
    assert!(dock_strip(&h).is_none(), "right-6 over the bar is blocked");
    // Just past the bar, still in the 18-point band, triggers again.
    hover(&mut h, pos2(canvas.right() - 13.0, canvas.center().y));
    assert!(dock_strip(&h).is_some(), "right-13 past the bar triggers");
    hover(&mut h, canvas.center());

    // Rulers on: the left band blocks the toolbar edge the same way.
    h.state_mut().ui.extras.rulers = true;
    h.run_steps(1);
    hide_tab(&mut h);
    let canvas = canvas_rect(&h);
    hover(&mut h, pos2(canvas.left() + 2.0, canvas.center().y));
    assert!(toolbar_strip(&h).is_some(), "left+2 triggers over the ruler band");
    hover(&mut h, canvas.center());
    assert!(toolbar_strip(&h).is_none(), "the middle ends the toolbar peek");
    hover(&mut h, pos2(canvas.left() + 8.0, canvas.center().y));
    assert!(toolbar_strip(&h).is_none(), "left+8 over the ruler band is blocked");
    hover(&mut h, pos2(canvas.left() + 17.0, canvas.center().y));
    assert!(toolbar_strip(&h).is_some(), "left+17 past the ruler band triggers");
}

#[test]
fn shift_tab_and_tab_hide_sources_peek_and_clear_distinctly() {
    let mut h = harness(400.0, 300.0);
    let canvas = canvas_rect(&h);
    let edge = pos2(canvas.right() - 2.0, canvas.center().y);

    // Tab hides: the dock peeks.
    hide_tab(&mut h);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_some(), "Tab-hidden dock peeks");
    hover(&mut h, canvas.center());

    // Tab shows again, Shift+Tab hides: the dock still peeks (its own hide source).
    hide_none(&mut h);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_none(), "a shown dock does not peek");
    hide_shift(&mut h);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_some(), "Shift+Tab-hidden dock peeks");
    hover(&mut h, canvas.center());

    // Both hide sources at once: the dock still peeks.
    set_hide(&mut h, HideState { tab: true, shift: true, revealed: false });
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_some(), "Tab- and Shift+Tab-hidden dock peeks");
    hover(&mut h, canvas.center());

    // Shift+Tab shows again: no peek.
    hide_none(&mut h);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_none(), "a shown dock does not peek");

    // A dock closed any other way (its panels closed, no dock column left) does not peek,
    // even while Tab hides the rest: there is nothing to bring back.
    close_dock_column(&mut h);
    assert!(peek_layout(h.state()).is_none(), "the dock column is gone");
    hide_tab(&mut h);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_none(), "a closed dock does not peek");
}

/// Closes every panel in the end dock column, so there is no dock column to peek.
fn close_dock_column(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let (layout, _) = peek_layout(h.state()).expect("a dock column to close");
    let panels: Vec<String> = layout.panels().into_iter().cloned().collect();
    for panel in panels {
        let _ = crate::panel_docking::apply(h.state_mut(), vec![Action::close(panel)]);
    }
    h.run_steps(1);
}

#[test]
fn toolbar_peek_strip_anchors_at_canvas_start_edge() {
    let mut h = harness(400.0, 300.0);
    hide_tab(&mut h);
    let canvas = canvas_rect(&h);
    hover(&mut h, pos2(canvas.left() + 2.0, canvas.center().y));
    let strip = toolbar_strip(&h).expect("the edge hover peeks the toolbar");
    assert_close(strip.left(), canvas.left(), "toolbar strip left");
    assert_close(strip.top(), canvas.top(), "toolbar strip top");
    hover(&mut h, canvas.center());
    assert!(toolbar_strip(&h).is_none(), "the middle ends the toolbar peek");
}

#[test]
fn peek_holds_while_pointer_drag_or_strip() {
    let mut h = harness(400.0, 300.0);
    hide_shift(&mut h);
    let canvas = canvas_rect(&h);
    let edge = pos2(canvas.right() - 2.0, canvas.center().y);

    // A drag never starts a peek: pressing in the canvas and crossing the edge mid-stroke
    // must not move the image under the pointer.
    press(&mut h, canvas.center());
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_none(), "a drag crossing the edge does not start a peek");
    release(&mut h, edge);

    // The peek holds while the pointer is over the strip it drew.
    hover(&mut h, edge);
    let strip = dock_strip(&h).expect("the edge hover peeks the dock");
    hover(&mut h, strip.center());
    assert!(dock_strip(&h).is_some(), "the peek holds over its own strip");

    // And while a drag started in the strip is still down, even over the canvas.
    // Press on a layer row: a row press selects (no popup), so the drag hold is
    // isolated from the popup hold. The strip centre lands on a Properties
    // dropdown, which opens a popup that would hold the peek past the release.
    let layers = probe::group_rect(&h, "layers").expect("the peeked layers group drew");
    press(&mut h, layers.center());
    // The press may focus a widget in the peek; the focus hold is a separate rule
    // (tested below), so surrender it here to isolate the drag hold.
    if let Some(focused) = h.ctx.memory(|m| m.focused()) {
        h.ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    h.run_steps(1);
    hover(&mut h, canvas.center());
    assert!(dock_strip(&h).is_some(), "a drag started in the peek holds it");
    release(&mut h, canvas.center());
    assert!(dock_strip(&h).is_none(), "releasing over the canvas ends the peek");
}

#[test]
fn rtl_peek_anchors_at_canvas_start_edge() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut h = harness(400.0, 300.0);
    hide_shift(&mut h);
    let canvas = canvas_rect(&h);
    hover(&mut h, pos2(canvas.left() + 2.0, canvas.center().y));
    let strip = dock_strip(&h).expect("the RTL edge hover peeks the dock");
    let expected = expected_dock_rect(&h, true);
    assert_close(strip.left(), expected.left(), "RTL peek left");
    assert_close(strip.right(), expected.right(), "RTL peek right");
    assert_close(strip.top(), expected.top(), "RTL peek top");
    assert_close(strip.bottom(), expected.bottom(), "RTL peek bottom");
}

fn secondary_click(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.event(egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, Box<PhotocraftApp>>, pos: Pos2) {
    hover(h, pos);
    press(h, pos);
    release(h, pos);
}

/// The dock peeked from its edge and settled, so its widgets are where they are drawn.
fn peek_dock(h: &mut Harness<'static, Box<PhotocraftApp>>) -> Rect {
    hide_shift(h);
    let canvas = canvas_rect(h);
    hover(h, pos2(canvas.right() - 2.0, canvas.center().y));
    h.run_steps(3);
    dock_strip(h).expect("the edge hover peeks the dock")
}

/// A text field the peeked dock drew, inside its strip.
fn text_field_in(h: &Harness<'static, Box<PhotocraftApp>>, strip: Rect) -> Rect {
    h.query_all_by_role(egui::accesskit::Role::TextInput).map(|n| n.rect()).find(|r| strip.contains(r.center())).expect("a text field in the peeked dock")
}

/// The dock peeked again from its edge (it is already hidden).
fn peek_dock_again(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let canvas = canvas_rect(h);
    hover(h, canvas.center());
    hover(h, pos2(canvas.right() - 2.0, canvas.center().y));
    h.run_steps(3);
}

/// Any popup is open: an egui popup, or a framework overlay popup (a Select's
/// listbox, a menu) — the peek's popup hold counts both.
fn any_popup_open(ctx: &egui::Context) -> bool {
    egui::Popup::is_any_open(ctx) || craft::ui::contract::overlay::any_open(ctx, craft::ui::contract::overlay::OpenFilter::Keys)
}

#[test]
fn a_popup_opened_from_the_peek_holds_it_until_it_closes() {
    let mut h = harness(400.0, 300.0);
    let strip = peek_dock(&mut h);
    let canvas = canvas_rect(&h);
    // The Layers panel's filter-type select, in the peeked dock: the framework panel
    // filters by kind with toggle buttons, so the dropdown is the filter-type select.
    let combo = h.get_by_label("Filter type").rect();
    assert!(strip.contains(combo.center()), "the dropdown {combo:?} is in the peek {strip:?}");
    click(&mut h, combo.center());
    h.run_steps(1);
    assert!(any_popup_open(&h.ctx), "the dropdown opened");
    // The click focused the select too; surrender that focus so only the popup holds
    // the peek.
    if let Some(focused) = h.ctx.memory(|m| m.focused()) {
        h.ctx.memory_mut(|m| m.surrender_focus(focused));
    }
    h.run_steps(1);

    // Away from the strip and the list: the popup the peek opened keeps it.
    let away = pos2(canvas.left() + 40.0, canvas.center().y);
    hover(&mut h, away);
    h.run_steps(1);
    assert!(dock_strip(&h).is_some(), "a popup opened from the peek holds it");

    // A press outside closes the popup, and with it the peek.
    click(&mut h, away);
    h.run_steps(2);
    assert!(!any_popup_open(&h.ctx), "the press outside closed the dropdown");
    assert!(dock_strip(&h).is_none(), "the peek ends with its popup");
}

#[test]
fn a_canvas_popup_does_not_hold_the_peek() {
    let mut h = harness_with_canvas_menu(400.0, 300.0);
    let strip = peek_dock(&mut h);
    // Held by a field focused in it, with the pointer over the canvas, the canvas opens a
    // context menu of its own. When focus leaves, that menu does not keep the peek: it is
    // not the peek's.
    let field = text_field_in(&h, strip);
    click(&mut h, field.center());
    hover(&mut h, CANVAS_MENU.center());
    assert!(dock_strip(&h).is_some(), "the focused field holds the peek");
    secondary_click(&mut h, CANVAS_MENU.center());
    h.run_steps(1);
    assert!(egui::Popup::is_any_open(&h.ctx), "the canvas menu opened");
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.run_steps(2);
    assert!(egui::Popup::is_any_open(&h.ctx), "the canvas menu is still open");
    assert!(dock_strip(&h).is_none(), "a popup the canvas opened does not hold the peek");
}

#[test]
fn a_widget_focused_in_the_peek_holds_it_until_focus_leaves() {
    let mut h = harness(400.0, 300.0);
    let strip = peek_dock(&mut h);
    let canvas = canvas_rect(&h);
    let field = text_field_in(&h, strip);
    click(&mut h, field.center());
    h.run_steps(1);
    assert!(h.ctx.memory(|m| m.focused()).is_some(), "the field took focus");

    // Typing goes on with the pointer over the canvas: the focused field holds the peek.
    let away = pos2(canvas.left() + 40.0, canvas.center().y);
    hover(&mut h, away);
    h.run_steps(1);
    assert!(dock_strip(&h).is_some(), "the focused field holds the peek");

    // Focus leaving the field ends it.
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.run_steps(2);
    assert!(dock_strip(&h).is_none(), "the peek ends once focus leaves");
}

#[test]
fn the_tool_flyout_holds_the_toolbar_peek() {
    let mut h = harness(400.0, 300.0);
    hide_tab(&mut h);
    let canvas = canvas_rect(&h);
    hover(&mut h, pos2(canvas.left() + 2.0, canvas.center().y));
    h.run_steps(3);
    let strip = toolbar_strip(&h).expect("the edge hover peeks the toolbar");
    // The first tool button with a flyout.
    let index = crate::panels::toolbar::TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.len() > 1).expect("a slot with a flyout");
    let bx = if crate::theme::Tokens::get(&h.ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = h.ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| strip.contains_rect(w.rect) && w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let button = *buttons.get(index).expect("the tool buttons in the peeked toolbar");
    secondary_click(&mut h, button.center());
    h.run_steps(1);
    assert!(crate::panels::toolbar::tool_flyout_open(&h.ctx), "the flyout opened");

    // Over the flyout, beside the strip: the toolbar stays. The flyout opens on the
    // canvas side of the toolbar at the button's row, so just past the strip's edge at
    // that height is inside it.
    let flyout_point = pos2(strip.right() + 30.0, button.center().y);
    assert!(!strip.expand(2.0).contains(flyout_point), "the flyout point lies outside the strip");
    hover(&mut h, flyout_point);
    h.run_steps(1);
    assert!(toolbar_strip(&h).is_some(), "the open flyout holds the toolbar peek");

    // A press on the canvas closes the flyout and ends the peek.
    click(&mut h, canvas.center());
    h.run_steps(2);
    assert!(!crate::panels::toolbar::tool_flyout_open(&h.ctx), "the press outside closed the flyout");
    assert!(toolbar_strip(&h).is_none(), "the peek ends with its flyout");
}

#[test]
fn the_toolbar_peek_fits_the_canvas_rect() {
    let mut h = harness(400.0, 300.0);
    hide_tab(&mut h);
    let canvas = canvas_rect(&h);
    hover(&mut h, pos2(canvas.left() + 2.0, canvas.center().y));
    h.run_steps(3);
    let strip = toolbar_strip(&h).expect("the edge hover peeks the toolbar");
    // Sized to the toolbar, not egui's default 600 x 400 area, and inside the canvas.
    assert!(canvas.expand(1.0).contains_rect(strip), "the toolbar peek {strip:?} lies in the canvas {canvas:?}");
    assert!(strip.width() < 120.0, "as wide as the toolbar: {strip:?}");
}

/// The tab of `panel` in the peeked dock: the tab button carrying the panel's title, in
/// the tab strip at the top of the panel's group. Tabs are buttons; the framework's tab
/// role may also read as a tab, so both are tried.
fn tab_rect(h: &Harness<'static, Box<PhotocraftApp>>, panel: &str) -> Rect {
    let group = probe::group_rect(h, panel).expect("the peeked panel's group");
    let title = crate::panels::registry::entry(panel).map(|entry| (entry.label)()).expect("the panel's title");
    let by_role = |role: Role| {
        h.query_all_by_role(role)
            .find(|n| n.accesskit_node().label().is_some_and(|label| label == title) && n.rect().top() >= group.top() && n.rect().top() < group.top() + 40.0)
            .map(|n| n.rect())
    };
    by_role(Role::Button).or_else(|| by_role(Role::Tab)).unwrap_or_else(|| panic!("the peeked {panel} tab"))
}

#[test]
fn a_tab_dragged_in_the_peek_does_not_rearrange_the_layout() {
    let mut h = harness(400.0, 300.0);
    peek_dock(&mut h);
    let before = h.state().ui.docking.layout().clone();
    // The column alone cannot say where a drop would land in the full layout, so a tab
    // dragged out of the peek neither floats nor moves.
    // Layers dragged onto the Color group's tabs, then out over the canvas.
    let (from, onto) = (tab_rect(&h, "layers").center(), tab_rect(&h, "color").center());
    for target in [onto, pos2(from.x - 300.0, from.y)] {
        press(&mut h, from);
        for k in 1..=10 {
            hover(&mut h, from + (target - from) * (k as f32 / 10.0));
        }
        release(&mut h, target);
        h.run_steps(2);
        assert_eq!(h.state().ui.docking.layout().clone(), before, "the layout is unchanged after a drop at {target:?}");
        peek_dock_again(&mut h);
    }
}
