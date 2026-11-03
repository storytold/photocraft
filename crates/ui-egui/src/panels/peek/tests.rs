//! Frame-level tests for the #4096 rework: the peek through `PhotocraftApp`, with the
//! canvas drawn first (as `ui_body` does) and the peek hook after it. Presence is read from
//! the strip the peek recorded — `compute_peek` clears a strip the frame its peek ends,
//! so a strip is present exactly when the peek drew — and geometry from the peek
//! `DockArea`'s probe rects.

use egui::{Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::PhotocraftApp;
use crate::panel_docking::probe;

/// The canvas plus the peek hook, in the order `ui_body` runs them: the canvas sets
/// `last_canvas_rect` and the scrollbar flags, then the peek computes and draws.
fn harness(doc_width: f32, doc_height: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": doc_width as i64, "height": doc_height as i64})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                crate::canvas::document_area(app, ui);
            });
            let peek = compute_peek(app, ui.ctx(), true, false);
            if peek.toolbar || peek.dock {
                draw_peek(app, ui.ctx(), &peek);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

fn canvas_rect(h: &Harness<'static, PhotocraftApp>) -> Rect {
    let rect = h.state().last_canvas_rect;
    assert!(rect.is_positive(), "the canvas drew: {rect:?}");
    rect
}

fn dock_strip(h: &Harness<'static, PhotocraftApp>) -> Option<Rect> {
    h.ctx.data(|d| d.get_temp::<Option<Rect>>(dock_strip_id())).flatten()
}

fn toolbar_strip(h: &Harness<'static, PhotocraftApp>) -> Option<Rect> {
    h.ctx.data(|d| d.get_temp::<Option<Rect>>(toolbar_strip_id())).flatten()
}

fn hover(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2) {
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
}

fn press(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2) {
    h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn release(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2) {
    h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// Window commands (`window.togglePanels`, `window.toggle.dock`) dispatch through the
/// menus, not the engine: the dock tests' path, not `app.run`.
fn run_window(h: &mut Harness<'static, PhotocraftApp>, id: &str) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap();
}

/// The dock column rect the peek should draw: the peek layout's column width, clamped to
/// the dock width range and the canvas, anchored at the canvas rect's end edge.
fn expected_dock_rect(h: &Harness<'static, PhotocraftApp>, rtl: bool) -> Rect {
    let canvas = canvas_rect(h);
    let (_, col_width) = peek_layout(h.state()).expect("a dock column to peek");
    let range = crate::panels::dock_width_range(h.ctx.content_rect().width());
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
    run_window(&mut h, "window.toggle.dock");
    h.run_steps(1);
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
    let group = probe::peek_group_rect(h.state(), &h.ctx, "layers").expect("the peeked layers group drew");
    // The column is anchored at the canvas edge; the layers group is its bottom group, so
    // it shares the column's sides and bottom and starts below the groups above it.
    assert_close(group.left(), expected.left(), "group left");
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
    let mut h = harness(1000.0, 1500.0);
    let canvas = canvas_rect(&h);
    let (h_drawn, v_drawn) = crate::canvas::scrollbars::last_drawn(&h.ctx);
    assert!(h_drawn && v_drawn, "both bars draw: h={h_drawn} v={v_drawn}");
    run_window(&mut h, "window.toggle.dock");
    h.run_steps(1);

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
    run_window(&mut h, "window.togglePanels");
    h.run_steps(1);
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
    run_window(&mut h, "window.togglePanels");
    h.run_steps(1);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_some(), "Tab-hidden dock peeks");
    hover(&mut h, canvas.center());

    // Tab shows again, Shift+Tab hides: the dock still peeks (its own hide source).
    run_window(&mut h, "window.togglePanels");
    run_window(&mut h, "window.toggle.dock");
    h.run_steps(1);
    assert!(h.state().ui.panels.dock_hidden_by_shift_tab, "Shift+Tab records its hide source");
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_some(), "Shift+Tab-hidden dock peeks");
    hover(&mut h, canvas.center());

    // Shift+Tab shows again: no peek, and the flag cleared.
    run_window(&mut h, "window.toggle.dock");
    h.run_steps(1);
    assert!(!h.state().ui.panels.dock_hidden_by_shift_tab, "showing clears the Shift+Tab flag");
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_none(), "a shown dock does not peek");

    // A dock hidden with neither source (direct state, as a workspace preset leaves it)
    // does not peek: only Tab and Shift+Tab hides come back on hover.
    h.state_mut().ui.panels.dock = false;
    h.state_mut().ui.panels.hidden_by_tab = None;
    h.state_mut().ui.panels.dock_hidden_by_shift_tab = false;
    h.run_steps(1);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_none(), "a dock hidden by neither Tab nor Shift+Tab does not peek");

    // Tab-hide, Shift+Tab (shows the dock), Tab-hide again: the dock peeks, because the
    // second Tab-hide is the hide source again.
    h.state_mut().ui.panels.dock = true;
    run_window(&mut h, "window.togglePanels");
    run_window(&mut h, "window.toggle.dock");
    run_window(&mut h, "window.togglePanels");
    h.run_steps(1);
    hover(&mut h, edge);
    assert!(dock_strip(&h).is_some(), "Tab-hide after Shift+Tab peeks");
}

#[test]
fn toolbar_peek_strip_anchors_at_canvas_start_edge() {
    let mut h = harness(400.0, 300.0);
    run_window(&mut h, "window.togglePanels");
    h.run_steps(1);
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
    run_window(&mut h, "window.toggle.dock");
    h.run_steps(1);
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
    press(&mut h, strip.center());
    hover(&mut h, canvas.center());
    assert!(dock_strip(&h).is_some(), "a drag started in the peek holds it");
    release(&mut h, canvas.center());
    assert!(dock_strip(&h).is_none(), "releasing over the canvas ends the peek");
}

#[test]
fn rtl_peek_anchors_at_canvas_start_edge() {
    let _rtl = crate::direction::direction_scope(crate::direction::Dir::Rtl);
    let mut h = harness(400.0, 300.0);
    run_window(&mut h, "window.toggle.dock");
    h.run_steps(1);
    let canvas = canvas_rect(&h);
    hover(&mut h, pos2(canvas.left() + 2.0, canvas.center().y));
    let strip = dock_strip(&h).expect("the RTL edge hover peeks the dock");
    let expected = expected_dock_rect(&h, true);
    assert_close(strip.left(), expected.left(), "RTL peek left");
    assert_close(strip.right(), expected.right(), "RTL peek right");
    assert_close(strip.top(), expected.top(), "RTL peek top");
    assert_close(strip.bottom(), expected.bottom(), "RTL peek bottom");
}
