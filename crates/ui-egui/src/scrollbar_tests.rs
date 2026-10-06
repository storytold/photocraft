//! Canvas scrollbars through the real canvas (#300): they appear only when the document overflows
//! the viewport, the thumb pans by the expected amount, a track click pages, and the bars never
//! reach the tool underneath. Degenerate sizes must not panic.

use egui::{Modifiers, PointerButton, Pos2, Vec2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::scrollbars::{Bars, layout};
use crate::state::Tool;

fn harness_sized(size: Vec2, w: u32, h: u32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": w, "height": h})).unwrap();
    app.sync_views();
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            // Fonts set up after the first frame only apply from the next one.
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn harness() -> Harness<'static, PhotocraftApp> {
    harness_sized(vec2(1200.0, 800.0), 400, 300)
}

/// The bars the canvas lays out for the current view.
fn bars(h: &Harness<'static, PhotocraftApp>) -> Bars {
    let app = h.state();
    let rect = crate::rulers::content_rect(app, app.last_canvas_rect);
    let view = &app.ui.views[0];
    let doc = &app.session.active().unwrap().doc;
    let xf = ViewXform { rect, zoom: view.zoom, center: view.center, flip: app.ui.view.flip_horizontal };
    layout(rect, xf.doc_rect(doc.bounds()), app.session.prefs().tools.overscroll)
}

fn set_view(h: &mut Harness<'static, PhotocraftApp>, zoom: f32, center: [f32; 2]) {
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = zoom;
    v.center = center;
    v.fit_pending = false;
    h.run_steps(2);
}

fn center(h: &Harness<'static, PhotocraftApp>) -> [f32; 2] {
    h.state().ui.views[0].center
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
}

/// Press at `from`, move to `to` in steps, release.
fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    h.run_steps(1);
    button(h, from, true);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    button(h, to, false);
    h.run_steps(2);
}

fn history_len(h: &Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn bars_appear_only_when_the_document_overflows() {
    let mut h = harness();
    let b = bars(&h);
    assert!(b.h.is_none() && b.v.is_none(), "a fitted document needs no scrollbars: {b:?}");

    set_view(&mut h, 4.0, [200.0, 150.0]);
    let b = bars(&h);
    assert!(b.h.is_some() && b.v.is_some(), "1600×1200 on screen overflows both ways: {b:?}");
    let rect = crate::rulers::content_rect(h.state(), h.state().last_canvas_rect);
    let v = b.v.unwrap();
    assert!((v.track.right() - rect.right()).abs() < 0.01 && v.track.width() <= crate::scrollbars::THICKNESS);
    // The thumb is the visible fraction of the scrollable range (document + overscroll margin).
    let visible = rect.height() / (1200.0 + rect.height());
    assert!((v.thumb.height() / v.track.height() - visible).abs() < 0.01, "{v:?}");

    // 100%, panned so the document hangs off the right edge only: a horizontal bar alone.
    set_view(&mut h, 1.0, [-400.0, 150.0]);
    let b = bars(&h);
    assert!(b.h.is_some() && b.v.is_none(), "{b:?}");

    // Full Screen modes have no scrollbars (Photoshop).
    set_view(&mut h, 4.0, [200.0, 150.0]);
    h.state_mut().ui.view.screen_mode = "fullScreen".into();
    h.state_mut().ui.tool = Tool::Hand;
    h.run_steps(2);
    let v = bars(&h).v.unwrap();
    let c0 = center(&h);
    // Where the thumb would be, the Hand drags the canvas the other way instead.
    drag(&mut h, v.thumb.center(), v.thumb.center() + vec2(0.0, 30.0));
    assert!(center(&h)[1] < c0[1], "no bar in Full Screen: the Hand pans up: {c0:?} -> {:?}", center(&h));
}

#[test]
fn dragging_the_thumb_pans_by_the_expected_amount() {
    let mut h = harness();
    h.state_mut().ui.tool = Tool::Brush;
    set_view(&mut h, 4.0, [200.0, 150.0]);
    let v = bars(&h).v.unwrap();
    let c0 = center(&h);
    let expected = v.span.shift_for_thumb(v.track.top(), v.track.bottom(), v.thumb.top() + 30.0) / 4.0;
    assert!(expected > 30.0 / 4.0, "the range is longer than the track: {expected}");
    drag(&mut h, v.thumb.center(), v.thumb.center() + vec2(0.0, 30.0));
    let c1 = center(&h);
    assert!((c1[1] - (c0[1] + expected)).abs() < 0.05, "thumb down 30 pt pans by {expected} doc px: {c0:?} -> {c1:?}");
    assert_eq!(c1[0], c0[0], "a vertical bar never pans sideways");
    assert_eq!(history_len(&h), 0, "the drag must not reach the Brush underneath");

    // Past the end: the document's bottom edge stops at the viewport centre (Overscroll on).
    let v = bars(&h).v.unwrap();
    drag(&mut h, v.thumb.center(), v.thumb.center() + vec2(0.0, 2000.0));
    assert!((center(&h)[1] - 300.0).abs() < 0.05, "{:?}", center(&h));

    // The horizontal bar, back to the start: the left edge at the centre.
    let b = bars(&h).h.unwrap();
    drag(&mut h, b.thumb.center(), b.thumb.center() - vec2(2000.0, 0.0));
    assert!(center(&h)[0].abs() < 0.05, "{:?}", center(&h));
    assert_eq!(history_len(&h), 0);

    // In sync with other panning: after a Hand-style pan the thumb follows.
    let before = bars(&h).v.unwrap().thumb;
    set_view(&mut h, 4.0, [200.0, 150.0]);
    assert!(bars(&h).v.unwrap().thumb.top() < before.top());
}

#[test]
fn a_track_click_pages_and_the_wheel_scrolls_over_a_bar() {
    let mut h = harness();
    set_view(&mut h, 4.0, [200.0, 150.0]);
    let b = bars(&h).h.unwrap();
    let c0 = center(&h);
    let page = b.span.page(true) / 4.0;
    let p = egui::pos2(b.track.right() - 3.0, b.track.center().y);
    assert!(p.x > b.thumb.right());
    h.hover_at(p);
    h.run_steps(1);
    button(&mut h, p, true);
    h.run_steps(1);
    button(&mut h, p, false);
    h.run_steps(2);
    let c1 = center(&h);
    assert!(page > 0.0 && (c1[0] - (c0[0] + page)).abs() < 0.05, "a click right of the thumb pages right by {page}: {c0:?} -> {c1:?}");

    let v = bars(&h).v.unwrap();
    h.hover_at(v.thumb.center());
    h.run_steps(1);
    h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -40.0), phase: egui::TouchPhase::Move, modifiers: Modifiers::NONE });
    h.run_steps(8);
    assert!(center(&h)[1] > c1[1], "the wheel over a bar scrolls the canvas");
}

#[test]
fn without_overscroll_the_view_stays_on_the_document() {
    let mut h = harness();
    h.state_mut().session.edit_prefs(|p| p.tools.overscroll = false);
    set_view(&mut h, 4.0, [-1000.0, 5000.0]);
    let rect = crate::rulers::content_rect(h.state(), h.state().last_canvas_rect);
    let c = center(&h);
    assert!((c[0] - rect.width() / 8.0).abs() < 0.05 && (c[1] - (300.0 - rect.height() / 8.0)).abs() < 0.05, "clamped to the edges: {c:?}");
    // The thumb stops at the document's edge.
    let v = bars(&h).v.unwrap();
    drag(&mut h, v.thumb.center(), v.thumb.center() - vec2(0.0, 2000.0));
    assert!((center(&h)[1] - rect.height() / 8.0).abs() < 0.05, "{:?}", center(&h));
    // Smaller than the viewport: centred, no bars.
    set_view(&mut h, 0.5, [-1000.0, 5000.0]);
    assert_eq!(center(&h), [200.0, 150.0]);
    let b = bars(&h);
    assert!(b.h.is_none() && b.v.is_none());
}

#[test]
fn degenerate_sizes_do_not_panic() {
    // A 1×1 document at a huge zoom, a zero-size window, and a canvas narrower than the bars.
    for (size, zoom) in [(vec2(1200.0, 800.0), 1e6), (vec2(1200.0, 800.0), 64.0), (vec2(0.0, 0.0), 64.0), (vec2(24.0, 24.0), 1e6), (vec2(60.0, 2000.0), 1e-6)] {
        for overscroll in [true, false] {
            let mut h = harness_sized(size, 1, 1);
            h.state_mut().session.edit_prefs(|p| p.tools.overscroll = overscroll);
            set_view(&mut h, zoom, [0.5, 0.5]);
            let b = bars(&h);
            for bar in [b.h, b.v].into_iter().flatten() {
                drag(&mut h, bar.thumb.center(), bar.thumb.center() + vec2(500.0, 500.0));
                button(&mut h, bar.track.min, true);
                button(&mut h, bar.track.min, false);
                h.run_steps(2);
            }
            assert!(center(&h).iter().all(|c| c.is_finite()), "{size:?} @ {zoom}: {:?}", center(&h));
        }
    }
}
