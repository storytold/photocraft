//! #16883: Window › Arrange opens a floating document window over the app window's document area. The document
//! area is the app window's own rect, in the app window's own points; the place a window opens at is a rect on the
//! desktop. Every test here stands the app window away from the screen's origin, so the two differ, and builds the
//! expected places from the app's own geometry: the window's rect as the host reports it and the document area the
//! app drew in its last pass.

use egui::{Context, Pos2, Rect, ViewportId, ViewportIdMap, ViewportInfo};
use serde_json::json;

use crate::PhotocraftApp;

/// The app window as the host reports it: 1000 x 720 desktop points, its top-left far from the screen's origin, as
/// the reference measurement of #16883 had it (the app window at 816, 414).
fn app_window() -> Rect {
    Rect::from_min_size(Pos2::new(760.0, 300.0), egui::vec2(1000.0, 720.0))
}

/// One pass in the order eframe runs them (`logic` before `ui`), with the app window reported at `window` on the
/// desktop, or with no rect reported at all when `window` is `None`, and `scale` pixels per point.
fn pass(app: &mut PhotocraftApp, ctx: &Context, window: Option<Rect>, scale: f32) {
    use eframe::App as _;
    let local = Rect::from_min_size(Pos2::ZERO, window.unwrap_or_else(app_window).size());
    let mut viewports = ViewportIdMap::default();
    // egui needs the ROOT entry either way; without a rect the host has told the app nothing about where its window
    // stands, which is the case before the first pass and on the fallback layer.
    viewports.insert(
        ViewportId::ROOT,
        match window {
            Some(window) => ViewportInfo {
                inner_rect: Some(window),
                outer_rect: Some(window),
                focused: Some(true),
                native_pixels_per_point: Some(scale),
                minimized: Some(false),
                ..Default::default()
            },
            None => ViewportInfo { focused: Some(true), native_pixels_per_point: Some(scale), ..Default::default() },
        },
    );
    let mut input = egui::RawInput { viewport_id: ViewportId::ROOT, screen_rect: Some(local), viewports, focused: true, ..Default::default() };
    app.raw_input_hook(ctx, &mut input);
    let mut frame = eframe::Frame::_new_kittest();
    let mut output = ctx.run_ui(input, |ui| {
        app.logic(ui.ctx(), &mut frame);
        app.ui(ui, &mut frame);
    });
    output.textures_delta.clear();
}

/// The app with `documents` documents, its window standing at `window` on the desktop (nowhere reported when
/// `window` is `None`) at `scale`, through the passes that lay the document area out. Document windows open as OS
/// windows of their own, as the desktop integration runs them.
fn desktop(documents: usize, window: Option<Rect>, scale: f32) -> (Box<PhotocraftApp>, Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for i in 0..documents {
        app.run("file.new", json!({ "width": 300 + 40 * i as u32, "height": 220, "name": format!("doc {i}") })).unwrap();
    }
    app.sync_views();
    let ctx = app.ctx().clone();
    PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.set_embed_viewports(false);
    for _ in 0..3 {
        pass(&mut app, &ctx, window, scale);
    }
    (app, ctx)
}

/// Runs an Arrange command and returns the floating windows it left, in order.
fn arranged(app: &mut PhotocraftApp, ctx: &Context, command: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    crate::menus::invoke(app, ctx, &format!("window.arrange.{command}"), params)
}

/// The windows' rects on the desktop, in the order the app lists its floating windows.
fn placed(app: &PhotocraftApp) -> Vec<Rect> {
    app.ui.document_windows.floating.iter().map(|(_, rect)| *rect).collect()
}

/// The document area where the app window stands: the rect the app drew its canvas in, moved from the app window's
/// own points into desktop points.
fn area_on_desktop(app: &PhotocraftApp, window: Rect) -> Rect {
    Rect::from_min_size(app.last_canvas_rect.min + window.min.to_vec2(), app.last_canvas_rect.size())
}

/// The passes placed a document area of their own inside the app window: the tests compare against that rect, not a
/// literal one, so they read what the app really has.
fn assert_real_document_area(app: &PhotocraftApp, window: Rect) {
    let canvas = app.last_canvas_rect;
    assert!(canvas.is_positive(), "the passes laid out a document area: {canvas:?}");
    let local = Rect::from_min_size(Pos2::ZERO, window.size());
    assert!(local.contains(canvas.min) && local.contains(canvas.max), "the document area is in the app window's own points: {canvas:?} in {local:?}");
    assert!(canvas.width() < window.width() && canvas.height() < window.height(), "the document area is part of the window, not all of it: {canvas:?}");
}

#[test]
fn float_in_window_opens_its_window_over_the_document_area() {
    let window = app_window();
    let (mut app, ctx) = desktop(1, Some(window), 1.0);
    assert_real_document_area(&app, window);
    let area = area_on_desktop(&app, window);
    arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("the active document floats");
    let windows = placed(&app);
    let first = windows.last().copied().expect("one floating window");
    assert_eq!(windows.len(), 1, "one window for the one document");
    assert!((first.min - area.min).length() < 0.5, "the window opens at the document area's top-left on the desktop: {first:?}, the area is {area:?}");
    assert!(area.contains(first.min), "so its top-left lies over the document area: {first:?} in {area:?}");
    assert!((first.size() - area.size() * 0.5).length() < 0.5, "the window still takes half the document area: {first:?} of {area:?}");
    assert!(
        (first.min - app.last_canvas_rect.min).length() > 100.0,
        "the document area's own points are not a desktop place: {first:?} against the window's own {area:?}"
    );
}

#[test]
fn float_all_in_windows_cascades_from_the_document_area() {
    let window = app_window();
    let (mut app, ctx) = desktop(3, Some(window), 1.0);
    assert_real_document_area(&app, window);
    let area = area_on_desktop(&app, window);
    arranged(&mut app, &ctx, "floatAllInWindows", json!({})).expect("every document gets a window");
    let windows = placed(&app);
    assert_eq!(windows.len(), 3, "one window per document");
    assert!((windows[0].min - area.min).length() < 0.5, "the first window at the document area's top-left: {windows:?}, the area is {area:?}");
    for pair in windows.windows(2) {
        let (before, next) = (pair[0], pair[1]);
        assert!(next.min.x > before.min.x && next.min.y > before.min.y, "each next window steps down and across: {before:?} then {next:?}");
        assert!(area.contains(next.min), "and opens over the document area: {next:?} in {area:?}");
    }
    assert!(windows.iter().all(|rect| (rect.size() - area.size() * 0.5).length() < 0.5), "every window takes half the document area: {windows:?}");
}

#[test]
fn new_window_for_document_opens_over_the_document_area() {
    let window = app_window();
    let (mut app, ctx) = desktop(1, Some(window), 1.0);
    let area = area_on_desktop(&app, window);
    arranged(&mut app, &ctx, "newWindowForDocument", json!({})).expect("a second view opens");
    let windows = placed(&app);
    assert_eq!(windows.len(), 1, "one new window");
    assert!((windows[0].min - area.min).length() < 0.5, "the second view opens at the document area's top-left: {windows:?}, the area is {area:?}");
    assert!(area.contains(windows[0].min), "so it lies over the document area: {windows:?} in {area:?}");
}

/// The point a torn-off tab was released at is a point of the app window; the window it opens stands on the desktop
/// under that same point.
#[test]
fn a_tab_dropped_off_the_strip_opens_its_window_where_it_was_dropped() {
    let window = app_window();
    let (mut app, ctx) = desktop(2, Some(window), 1.0);
    assert_real_document_area(&app, window);
    let document = app.session.documents()[1].doc.id;
    let dropped = app.last_canvas_rect.min + egui::vec2(30.0, 12.0);
    arranged(&mut app, &ctx, "floatInWindow", json!({ "document": document.0, "position": [dropped.x, dropped.y] })).expect("the torn-off tab floats");
    let windows = placed(&app);
    assert_eq!(windows.len(), 1, "the drop opened one window");
    let want = dropped + window.min.to_vec2();
    assert!((windows[0].min - want).length() < 0.5, "the window opens under the point the tab was dropped at: {:?}, want {want:?}", windows[0]);
}

#[test]
fn cascade_moves_every_window_into_the_document_area() {
    let window = app_window();
    let (mut app, ctx) = desktop(3, Some(window), 1.0);
    let area = area_on_desktop(&app, window);
    arranged(&mut app, &ctx, "floatAllInWindows", json!({})).expect("every document gets a window");
    // One window the user dragged off the document area: Cascade brings it back, as it did before.
    let size = app.ui.document_windows.floating[0].1.size();
    app.ui.document_windows.floating[0].1 = Rect::from_min_size(Pos2::new(2400.0, 1800.0), size);
    arranged(&mut app, &ctx, "cascade", json!({})).expect("cascade runs");
    let windows = placed(&app);
    assert!((windows[0].min - area.min).length() < 0.5, "the cascade starts at the document area's top-left: {windows:?}, the area is {area:?}");
    assert!(windows.iter().all(|rect| area.contains(rect.min)), "and every window lands over the document area: {windows:?} in {area:?}");
}

/// The docked layouts work in the app window's own points: a floating window's desktop place must not move the
/// document area the app draws in.
#[test]
fn the_docked_layouts_keep_the_window_s_own_document_area() {
    let window = app_window();
    let (mut app, ctx) = desktop(2, Some(window), 1.0);
    assert_real_document_area(&app, window);
    let canvas = app.last_canvas_rect;
    arranged(&mut app, &ctx, "twoUpVertical", json!({})).expect("2-up Vertical arranges the panes");
    assert_eq!(app.last_canvas_rect, canvas, "the layout leaves the app window's own document rect alone");
    let panes = crate::canvas::arranged_cells(&app, app.last_canvas_rect).expect("two panes");
    assert_eq!(panes.len(), 2, "one pane per document");
    for (document, pane) in &panes {
        assert!(canvas.contains(pane.min) && canvas.contains(pane.max), "pane {document} stays inside the window's own points: {pane:?} in {canvas:?}");
    }
    arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("the active document floats");
    let windows = placed(&app);
    let first = windows.last().copied().expect("the floated window");
    assert!(area_on_desktop(&app, window).contains(first.min), "the floated window still opens over the document area: {first:?}");
    assert_eq!(app.last_canvas_rect, canvas, "and the document area did not move");
}

/// With no document open the app shows its empty desktop: no Arrange command opens a window there, and the desktop's
/// own layout is left as it is.
#[test]
fn an_empty_desktop_opens_no_window_and_keeps_its_layout() {
    let window = app_window();
    let (mut app, ctx) = desktop(0, Some(window), 1.0);
    let canvas = app.last_canvas_rect;
    arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("Float in Window over an empty desktop runs the framework arrangement");
    assert!(arranged(&mut app, &ctx, "floatInWindow", json!({ "position": [300.0, 200.0] })).is_err(), "a drop with no document behind it is refused");
    for command in ["floatAllInWindows", "newWindowForDocument", "cascade", "tile", "tileAllVertically"] {
        let _ = arranged(&mut app, &ctx, command, json!({}));
    }
    assert!(app.ui.document_windows.floating.is_empty(), "no window opened over the empty desktop: {:?}", placed(&app));
    assert!(app.ui.document_windows.docked.is_empty(), "and no document group appeared");
    pass(&mut app, &ctx, Some(window), 1.0);
    assert_eq!(app.last_canvas_rect, canvas, "the empty desktop keeps the layout it had");
}

/// Where the app window stands at the screen's origin its own points are desktop points: placement is what it was.
#[test]
fn a_window_at_the_screen_origin_places_where_it_always_did() {
    let window = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 720.0));
    let (mut app, ctx) = desktop(1, Some(window), 1.0);
    assert_real_document_area(&app, window);
    arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("the active document floats");
    let first = placed(&app).pop().expect("one floating window");
    assert!((first.min - app.last_canvas_rect.min).length() < 0.5, "at the origin the window opens at the document area's own rect: {first:?}");
}

/// The desktop place is known once the host reports the app window's rect. Before that, and on the fallback layer
/// with no OS windows, the document area's own points are used as they are today.
#[test]
fn an_app_that_has_not_reported_its_place_keeps_todays_placement() {
    let (mut app, ctx) = desktop(1, None, 1.0);
    assert_real_document_area(&app, app_window());
    arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("the active document floats");
    let first = placed(&app).pop().expect("one floating window");
    assert!((first.min - app.last_canvas_rect.min).length() < 0.5, "with no rect reported the window opens where the document area is: {first:?}");
}

/// Where the host opens no OS window (egui embeds the viewports, as in the web build), the document window shows on
/// the framework's fallback layer inside the app window: its rect stays one of the app window's own points, and the
/// app window's desktop place is not added.
#[test]
fn a_document_window_on_the_fallback_layer_keeps_the_app_window_s_own_points() {
    let window = app_window();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({ "width": 300, "height": 220, "name": "fallback.png" })).unwrap();
    app.sync_views();
    let ctx = app.ctx().clone();
    PhotocraftApp::setup_context(&ctx, Default::default());
    assert!(!craft::ui::runtime::windows::WindowHost::of(&ctx).viewports_available(), "this host opens no OS window of its own");
    for _ in 0..3 {
        pass(&mut app, &ctx, Some(window), 1.0);
    }
    assert_real_document_area(&app, window);
    arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("the active document floats");
    let first = placed(&app).pop().expect("one floating window");
    assert!(
        (first.min - app.last_canvas_rect.min).length() < 0.5,
        "the fallback layer keeps the app window's own points: {first:?} in {:?}",
        app.last_canvas_rect
    );
}

/// The rule is in points, the unit the app window's rect and the document area are both reported in: the same place
/// at 100 %, 125 % and 200 % display scale.
#[test]
fn the_document_area_rule_holds_at_100_125_and_200_percent_scale() {
    let window = app_window();
    let mut places: Vec<Rect> = Vec::new();
    for scale in [1.0_f32, 1.25, 2.0] {
        let (mut app, ctx) = desktop(1, Some(window), scale);
        let area = area_on_desktop(&app, window);
        assert!((area.min.x - (app.last_canvas_rect.min.x + window.min.x)).abs() < 0.5, "the area maps at {scale}: {area:?}");
        arranged(&mut app, &ctx, "floatInWindow", json!({})).expect("the active document floats");
        let first = placed(&app).pop().expect("one floating window");
        assert!((first.min - area.min).length() < 0.5, "at {scale} the window opens at the document area's top-left: {first:?}, the area is {area:?}");
        assert!((first.size() - area.size() * 0.5).length() < 0.5, "at {scale} the window's size is the same in points: {first:?} of {area:?}");
        places.push(first);
    }
    assert_eq!(places.len(), 3, "one place per scale");
    for pair in places.windows(2) {
        assert!((pair[0].min - pair[1].min).length() < 0.5, "the scale does not move the window's place in points: {pair:?}");
    }
}
