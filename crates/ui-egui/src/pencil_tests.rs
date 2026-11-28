//! The Pencil tool (#213): aliased strokes through the live-stroke path, Auto Erase, the
//! pixel-grid cursor, ⇧-click lines, Control+Alt resizing and the B group.

use std::collections::BTreeSet;

use craft::behavior::CanvasView;
use craft::behavior::kurbo::Point;
use egui::{Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_paint::brush::TipShape;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::cursor_policy::pencil_tip_is_round;
use crate::canvas::{ToolEvent, active_view, tool_event};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    // Each tool keeps its own brush (#218): switch first, then set the Pencil's options, as the
    // options bar does.
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 3, "hardness": 0.0, "smoothing": {"amount": 0.0}}})).unwrap();
    app
}

fn layer(app: &PhotocraftApp) -> photocraft_engine::doc::Layer {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
}

fn rgba(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    layer(app).surface().unwrap().rgba(x, y)
}

fn drag(app: &mut PhotocraftApp, pts: &[(f64, f64)], mods: Modifiers) {
    let (x, y) = pts[0];
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    for &(x, y) in &pts[1..] {
        tool_event(app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
    }
    let &(x, y) = pts.last().unwrap();
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn last(app: &PhotocraftApp) -> (String, serde_json::Value) {
    app.session.journal.last().cloned().unwrap()
}

#[test]
fn pencil_strokes_are_aliased_live_and_one_undo_step() {
    let mut app = app();
    let pts = [(10.0, 10.0), (40.3, 22.7), (70.0, 50.0), (100.0, 30.0)];
    let (x, y) = pts[0];
    tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    for &(x, y) in &pts[1..] {
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    // The canvas shows the engine's real dabs while dragging.
    assert!(app.live_stroke.is_some(), "the Pencil strokes live");
    tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 30.0 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.pencil");
    assert_eq!(p["autoErase"], json!(false));
    assert_eq!(app.session.active().unwrap().history.past_len(), 1);
    // 8 bits, a soft session brush: still no partial alpha anywhere.
    let s = layer(&app).surface().unwrap().clone();
    let mut painted = 0;
    for y in 0..80 {
        for x in 0..120 {
            let a = s.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y})");
            painted += usize::from(a == 1.0);
        }
    }
    assert!(painted > 200, "{painted}");
    assert_eq!(rgba(&app, 70, 50), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn auto_erase_paints_the_background_colour_from_foreground_pixels() {
    let mut app = app();
    drag(&mut app, &[(10.0, 40.0), (110.0, 40.0)], Modifiers::NONE);
    app.ui.tool_options.pencil_auto_erase = true;
    // Starting on the black line: white.
    drag(&mut app, &[(50.0, 40.0), (50.0, 70.0)], Modifiers::NONE);
    assert_eq!(rgba(&app, 50, 60), [1.0; 4]);
    assert_eq!(last(&app).1["autoErase"], json!(true));
    // Starting off it: black.
    drag(&mut app, &[(80.0, 10.0), (80.0, 30.0)], Modifiers::NONE);
    assert_eq!(rgba(&app, 80, 20), [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn shift_click_draws_a_pencil_line() {
    let mut app = app();
    drag(&mut app, &[(10.0, 10.0), (20.0, 10.0)], Modifiers::NONE);
    drag(&mut app, &[(100.0, 60.0)], Modifiers::SHIFT);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.pencil");
    assert_eq!(p["points"].as_array().unwrap().len(), 2, "{p}");
    assert_eq!(rgba(&app, 60, 35)[3], 1.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), 2);
}

#[test]
fn shift_clicked_one_pixel_line_uses_the_clicked_pixels_not_subpixel_positions() {
    let fractions = [("TL", (0.05, 0.05)), ("TR", (0.95, 0.05)), ("BL", (0.05, 0.95)), ("BR", (0.95, 0.95)), ("C", (0.5, 0.5))];
    let starts = [&fractions[0], &fractions[1], &fractions[2], &fractions[3]];

    for (start_name, start_fraction) in starts {
        for (end_name, end_fraction) in fractions {
            let paint = |raw: bool| {
                let mut app = app();
                app.run("tools.setBrush", json!({"brush": {"size": 1.0, "hardness": 1.0}})).unwrap();
                let at = |origin: f64, fraction: f64| origin + if raw { fraction } else { 0.5 };
                let start = (at(10.0, start_fraction.0), at(10.0, start_fraction.1));
                let end = (at(60.0, end_fraction.0), at(45.0, end_fraction.1));
                for (point, mods) in [(start, Modifiers::NONE), (end, Modifiers::SHIFT)] {
                    tool_event(&mut app, ToolEvent::Down { x: point.0, y: point.1, pressure: 1.0 }, mods);
                    tool_event(&mut app, ToolEvent::Up { x: point.0, y: point.1 }, mods);
                }
                (0..80).flat_map(|y| (0..120).map(move |x| (x, y))).filter(|&(x, y)| rgba(&app, x, y)[3] > 0.0).collect::<Vec<_>>()
            };

            assert_eq!(paint(true), paint(false), "{start_name}->{end_name}");
        }
    }
}

#[test]
fn shift_clicked_pencil_does_not_split_a_click_near_a_pixel_boundary() {
    let mut app = app();
    app.run("tools.setBrush", json!({"brush": {"size": 1.0, "hardness": 1.0}})).unwrap();
    let click = |app: &mut PhotocraftApp, x: f64, y: f64, modifiers| {
        tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, modifiers);
        tool_event(app, ToolEvent::Up { x, y }, modifiers);
    };

    click(&mut app, 30.5, 30.5, Modifiers::NONE);
    // The pointer is just below the grid corner; numerical rounding must not make the Pencil
    // snap to one pixel and the line walker rasterize the neighbouring one.
    let just_below_corner = 30.0 - 5e-8;
    click(&mut app, just_below_corner, just_below_corner, Modifiers::SHIFT);

    assert_eq!(rgba(&app, 29, 29)[3], 0.0, "the click should remain in the pixel chosen by the line walker");
    assert_eq!(rgba(&app, 30, 30)[3], 1.0);
}

#[test]
fn ctrl_alt_drag_resizes_the_pencil_without_painting() {
    let mut app = app();
    // ⌃⌥ is its own chord on a Mac only: elsewhere the window host reports Ctrl as ⌘.
    crate::canvas::testing::on_a_mac(&app);
    let ctrl_alt = Modifiers { alt: true, ctrl: true, ..Default::default() };
    drag(&mut app, &[(50.0, 40.0), (60.0, 40.0)], ctrl_alt);
    assert_eq!(app.session.tools.brush.size, 23.0);
    assert_eq!(app.session.active().unwrap().history.past_len(), 0);
}

/// Unit edges of the pixel grid: `(x, y, true)` runs from corner (x, y) to (x + 1, y),
/// `(x, y, false)` from (x, y) to (x, y + 1).
type GridEdges = BTreeSet<(i64, i64, bool)>;

/// The canvas alone at 800 % around document point (20, 15), on a display of `ppp` physical
/// pixels per point, without the rulers or the pixel grid (which strokes every pixel edge itself).
fn cursor_canvas(ppp: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = app();
    app.ui.extras.rulers = false;
    app.ui.view.show.pixel_grid = false;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).with_pixels_per_point(ppp).build_ui_state(
        |ui: &mut egui::Ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        *app,
    );
    h.state_mut().ctx = h.ctx.clone();
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center) = (8.0, [20.0, 15.0]);
    v.pending.clear();
    h.run_steps(2);
    h
}

/// What the canvas paints for the cursor with the pointer at document point `at`: the shapes of a
/// frame with the pointer there that a frame without the pointer lacks.
fn cursor_shapes(h: &mut Harness<'_, PhotocraftApp>, at: [f64; 2]) -> (CanvasView, Vec<egui::Shape>) {
    h.event(egui::Event::PointerGone);
    h.run_steps(2);
    let mut idle = h.output().shapes.clone();
    let xf = active_view(h.state()).unwrap();
    h.event(egui::Event::PointerMoved(xf.to_screen(Point::new(at[0], at[1]))));
    h.run_steps(2);
    let mut cursor = Vec::new();
    for c in &h.output().shapes {
        match idle.iter().position(|i| i == c) {
            Some(k) => {
                idle.swap_remove(k);
            }
            None => cursor.push(c.shape.clone()),
        }
    }
    (xf, cursor)
}

/// The pixel-grid edges `shapes` stroke, read back into document pixels through the view. What is
/// not a straight stroke between two pixel corners goes to `other`.
fn drawn_grid_edges(shapes: &[egui::Shape], xf: &CanvasView, edges: &mut GridEdges, other: &mut Vec<&'static str>) {
    use egui::Shape;
    // A pixel corner, give or take a quarter pixel: the outline's strokes sit just either side
    // of the pixel edges, on physical pixels.
    let corner = |p: Pos2| {
        let pt = xf.to_doc(p);
        let (cx, cy) = (pt.x.round(), pt.y.round());
        ((pt.x - cx).abs() < 0.25 && (pt.y - cy).abs() < 0.25).then_some((cx as i64, cy as i64))
    };
    for s in shapes {
        let segments: Vec<[Pos2; 2]> = match s {
            Shape::Vec(v) => {
                drawn_grid_edges(v, xf, edges, other);
                continue;
            }
            Shape::LineSegment { points, .. } => vec![*points],
            Shape::Rect(r) if r.stroke.width > 0.0 => {
                let c = [r.rect.left_top(), r.rect.right_top(), r.rect.right_bottom(), r.rect.left_bottom()];
                (0..4).map(|i| [c[i], c[(i + 1) % 4]]).collect()
            }
            Shape::Path(p) if p.stroke.width > 0.0 => {
                let n = p.points.len();
                let m = if p.closed { n } else { n.saturating_sub(1) };
                (0..m).map(|i| [p.points[i], p.points[(i + 1) % n]]).collect()
            }
            Shape::Circle(_) | Shape::Ellipse(_) => {
                other.push("circle");
                continue;
            }
            _ => {
                other.push("other shape");
                continue;
            }
        };
        for [a, b] in segments {
            match (corner(a), corner(b)) {
                (Some((x0, y0)), Some((x1, y1))) if x0 == x1 => edges.extend((y0.min(y1)..y0.max(y1)).map(|y| (x0, y, false))),
                (Some((x0, y0)), Some((x1, y1))) if y0 == y1 => edges.extend((x0.min(x1)..x0.max(x1)).map(|x| (x, y0, true))),
                _ => other.push("off-grid stroke"),
            }
        }
    }
}

/// The grid edges around the pixels of the 120 × 80 document where `inside` holds, and how many
/// pixels that is.
fn boundary(inside: impl Fn(i64, i64) -> bool) -> (usize, GridEdges) {
    let inside = |x: i64, y: i64| (0..120).contains(&x) && (0..80).contains(&y) && inside(x, y);
    let (mut pixels, mut edges) = (0, GridEdges::new());
    for y in 0..80 {
        for x in 0..120 {
            if !inside(x, y) {
                continue;
            }
            pixels += 1;
            for (nx, ny, edge) in [(x, y - 1, (x, y, true)), (x, y + 1, (x, y + 1, true)), (x - 1, y, (x, y, false)), (x + 1, y, (x + 1, y, false))] {
                if !inside(nx, ny) {
                    edges.insert(edge);
                }
            }
        }
    }
    (pixels, edges)
}

/// The pixels one `paint.pencil` dab at `at` fills (the session brush, plus `extra` params), as
/// the grid edges around them, and how many there are: painted, read back and undone.
fn dab_grid_edges(app: &mut PhotocraftApp, at: [f64; 2], extra: serde_json::Value) -> (usize, GridEdges) {
    let mut p = json!({"points": [at]});
    if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(extra.clone());
    }
    app.run("paint.pencil", p).unwrap();
    let s = layer(app).surface().unwrap().clone();
    app.run("edit.undo", json!({})).unwrap();
    boundary(|x, y| s.rgba(x as i32, y as i32)[3] == 1.0)
}

#[test]
fn the_pencil_cursor_outlines_the_pixels_its_dab_fills() {
    // #5262: the Pencil's cursor, and the Pencil- and Block-mode Eraser's, outlines exactly the
    // whole pixels one dab at the pointer fills, along the pixel grid. From 4 px up it was a
    // smooth circle cut through them. Roundness and angle shape the outline as they shape the
    // dab, and it turns and flips with the view (the 1-3 px square and the Block square did not).
    // The tool, the Eraser's mode, the brush, the view's rotation and flip, the pointer.
    type Case = (Tool, &'static str, serde_json::Value, f32, bool, [f64; 2]);
    let round = |size: f64| json!({"size": size, "roundness": 1.0, "angle": 0.0});
    let mut cases: Vec<Case> = Vec::new();
    for n in 1..=12 {
        for at in [[20.3, 15.6], [20.8, 15.2]] {
            cases.push((Tool::Pencil, "brush", round(f64::from(n)), 0.0, false, at));
        }
    }
    // The dab's radius is half the unrounded size: a 2.6 px dab is a plus, a 4.4 px one a square.
    for size in [2.6, 4.4] {
        cases.push((Tool::Pencil, "brush", round(size), 0.0, false, [20.3, 15.6]));
    }
    for angle in [0.0, 30.0] {
        cases.push((Tool::Pencil, "brush", json!({"size": 9, "roundness": 0.5, "angle": angle}), 0.0, false, [20.3, 15.6]));
    }
    for n in [2.0, 5.0, 8.0] {
        cases.push((Tool::Pencil, "brush", round(n), 30.0, true, [20.3, 15.6]));
    }
    // The report: the Eraser in Pencil mode. The Block square is 16 screen pixels (2 px here).
    cases.push((Tool::Eraser, "pencil", round(9.0), 0.0, false, [20.3, 15.6]));
    cases.push((Tool::Eraser, "pencil", round(6.0), 30.0, false, [20.8, 15.2]));
    cases.push((Tool::Eraser, "block", round(9.0), 0.0, false, [20.3, 15.6]));
    cases.push((Tool::Eraser, "block", round(9.0), 30.0, true, [20.3, 15.6]));
    let mut h = cursor_canvas(1.0);
    let mut fails = Vec::new();
    for (tool, mode, brush, rotation, flip, at) in &cases {
        let app = h.state_mut();
        app.ui.tool = *tool;
        crate::paint_mouse::sync_tool_brush(app);
        app.ui.tool_options.eraser_mode = (*mode).into();
        app.run("tools.setBrush", json!({"brush": brush})).unwrap();
        app.ui.views[0].rotation = *rotation;
        app.ui.view.flip_horizontal = *flip;
        let (xf, shapes) = cursor_shapes(&mut h, *at);
        let (mut drawn, mut other) = (GridEdges::new(), Vec::new());
        drawn_grid_edges(&shapes, &xf, &mut drawn, &mut other);
        let extra = if *mode == "block" { json!({"block": true, "zoom": xf.zoom}) } else { json!({}) };
        let (pixels, want) = dab_grid_edges(h.state_mut(), *at, extra);
        if pixels == 0 || drawn != want || !other.is_empty() {
            fails.push(format!(
                "{tool:?} ({mode}) {brush} at {at:?}, view {rotation}° flipped {flip}: the dab fills {pixels} px ({} grid edges); the \
                 cursor drew {} of those edges, {} other grid edges, and {other:?}",
                want.len(),
                drawn.intersection(&want).count(),
                drawn.difference(&want).count(),
            ));
        }
    }
    // Hostile brush values (a damaged preset, a script) draw something or nothing, never panic.
    let app = h.state_mut();
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(app);
    for size in [f32::NAN, f32::INFINITY, -4.0, 0.0, 1e9] {
        h.state_mut().session.tools.brush.size = size;
        h.run_steps(1);
    }
    h.state_mut().session.tools.brush.size = 9.0;
    for v in [f32::NAN, f32::INFINITY, -1.0] {
        let b = &mut h.state_mut().session.tools.brush;
        (b.roundness, b.angle) = (v, v);
        h.run_steps(1);
    }
    assert!(fails.is_empty(), "the cursor is not the dab's footprint:\n{}", fails.join("\n"));
}

/// A plain click (press and release, no movement) on the canvas at document point `at`.
fn click(h: &mut Harness<'static, PhotocraftApp>, at: [f64; 2]) {
    let pos = active_view(h.state()).unwrap().to_screen(Point::new(at[0], at[1]));
    h.event(egui::Event::PointerMoved(pos));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// An L-shaped sampled tip: not symmetric about either axis or a diagonal.
fn l_tip() -> TipShape {
    TipShape::Sampled(photocraft_paint::GrayTile::from_fn(20, 14, |x, y| if x < 6 || y >= 9 { 1.0 } else { 0.0 }))
}

/// A click on the canvas, to compare with the cursor drawn just before it.
struct Click {
    tool: Tool,
    /// The Eraser's mode.
    mode: &'static str,
    brush: serde_json::Value,
    tip: TipShape,
    /// The view's rotation and flip, and View › Pixel Aspect Ratio.
    rotation: f32,
    flip: bool,
    pixel_aspect: &'static str,
    at: [f64; 2],
}

impl Click {
    /// The Pencil with a round tip and `brush` at `at`, on a plain view.
    fn pencil(brush: serde_json::Value, at: [f64; 2]) -> Self {
        Self { tool: Tool::Pencil, mode: "brush", brush, tip: TipShape::Round, rotation: 0.0, flip: false, pixel_aspect: "square", at }
    }
}

/// Draws the cursor for `c`, clicks there and undoes the click: `None` when the click changed
/// exactly the pixels the cursor outlined, in one undo step, else what differs.
fn check_click(h: &mut Harness<'static, PhotocraftApp>, c: &Click) -> Option<String> {
    let app = h.state_mut();
    app.ui.tool = c.tool;
    crate::paint_mouse::sync_tool_brush(app);
    app.ui.tool_options.eraser_mode = c.mode.into();
    app.run("tools.setBrush", json!({"brush": c.brush})).unwrap();
    app.session.tools.brush.tip = c.tip.clone().into();
    app.ui.views[0].rotation = c.rotation;
    app.ui.view.flip_horizontal = c.flip;
    app.ui.view.pixel_aspect = c.pixel_aspect.into();
    let (xf, shapes) = cursor_shapes(h, c.at);
    if (xf.aspect - f64::from(crate::view_cmds::pixel_aspect_ratio(c.pixel_aspect))).abs() > 1e-6 {
        return Some(format!("pixel aspect {}: the view's aspect is {}", c.pixel_aspect, xf.aspect));
    }
    let (mut drawn, mut other) = (GridEdges::new(), Vec::new());
    drawn_grid_edges(&shapes, &xf, &mut drawn, &mut other);
    let before = layer(h.state()).surface().unwrap().clone();
    let past = h.state().session.active().unwrap().history.past_len();
    click(h, c.at);
    let after = layer(h.state()).surface().unwrap().clone();
    let steps = h.state().session.active().unwrap().history.past_len().saturating_sub(past);
    for _ in 0..steps {
        h.state_mut().run("edit.undo", json!({})).unwrap();
    }
    let (pixels, want) = boundary(|x, y| before.rgba(x as i32, y as i32) != after.rgba(x as i32, y as i32));
    if pixels > 0 && drawn == want && other.is_empty() && steps == 1 {
        return None;
    }
    Some(format!(
        "{:?} ({}) {} with a {} tip at {:?}, view {}° flipped {}, pixel aspect {}: the click ({steps} undo steps) changed {pixels} px \
         ({} grid edges); the cursor drew {} of those edges, {} other grid edges, and {other:?}",
        c.tool,
        c.mode,
        c.brush,
        if c.tip == TipShape::Round { "round" } else { "sampled" },
        c.at,
        c.rotation,
        c.flip,
        c.pixel_aspect,
        want.len(),
        drawn.intersection(&want).count(),
        drawn.difference(&want).count(),
    ))
}

#[test]
fn a_real_click_paints_exactly_what_the_pencil_cursor_outlined() {
    // #5262 end to end: a click on the canvas, through the live-stroke path, the pixel-grid snap
    // and the commit, changes exactly the pixels the cursor outlined just before.
    let round = |size: f64| json!({"size": size, "roundness": 1.0, "angle": 0.0});
    let at = [20.3, 15.6];
    let mut clicks: Vec<Click> = [1.0, 2.0, 3.0, 4.0, 5.0, 8.0, 9.0, 12.0].map(|n| Click::pencil(round(n), at)).into();
    // Even and odd tips on a pixel corner, a pixel centre, half-way along an edge and just off a
    // corner.
    for p in [[20.0, 15.0], [20.5, 15.5], [20.5, 15.0], [19.999, 15.001]] {
        clicks.extend([4.0, 5.0, 8.0].map(|n| Click::pencil(round(n), p)));
    }
    // A squashed, turned tip on every quarter turn of the view, mirrored or not, and in between.
    let ellipse = json!({"size": 9, "roundness": 0.4, "angle": 30});
    for rotation in [0.0, 90.0, 180.0, 270.0, 30.0] {
        for flip in [false, true] {
            clicks.push(Click { rotation, flip, ..Click::pencil(ellipse.clone(), at) });
        }
    }
    // An asymmetric sampled tip, turned and flipped, on plain, quarter-turned and mirrored views.
    let l = json!({"size": 12, "roundness": 1.0, "angle": 20, "flipX": true});
    for (rotation, flip) in [(0.0, false), (90.0, true), (180.0, false), (30.0, true)] {
        clicks.push(Click { tip: l_tip(), rotation, flip, ..Click::pencil(l.clone(), at) });
    }
    // Pixel Aspect Ratio Correction stretches the view, and the outline with it.
    for (pixel_aspect, rotation) in [("anamorphic2To1", 0.0), ("anamorphic2To1", 90.0), ("d1DvNtsc", 30.0)] {
        clicks.push(Click { pixel_aspect, rotation, ..Click::pencil(ellipse.clone(), at) });
    }
    let mut h = cursor_canvas(1.0);
    let fails: Vec<String> = clicks.iter().filter_map(|c| check_click(&mut h, c)).collect();
    assert!(fails.is_empty(), "{} of {} clicks:\n{}", fails.len(), clicks.len(), fails.join("\n"));
}

#[test]
fn a_real_eraser_click_erases_exactly_what_its_cursor_outlined() {
    // The report's tool: the Eraser in Pencil mode, and in Block mode, on an opaque layer (one
    // 400 px dab covers the 120 × 80 document), with the view turned and mirrored.
    let mut h = cursor_canvas(1.0);
    h.state_mut().run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    let eraser = |mode, brush| Click { tool: Tool::Eraser, mode, ..Click::pencil(brush, [20.3, 15.6]) };
    let ellipse = json!({"size": 9, "roundness": 0.5, "angle": 45});
    let mut clicks = Vec::new();
    for (rotation, flip) in [(0.0, false), (90.0, true), (30.0, false), (270.0, true)] {
        for (mode, brush) in [("pencil", ellipse.clone()), ("pencil", json!({"size": 6, "roundness": 1.0, "angle": 0})), ("block", ellipse.clone())] {
            clicks.push(Click { rotation, flip, ..eraser(mode, brush) });
        }
    }
    let l = json!({"size": 12, "angle": 20, "flipX": true});
    clicks.push(Click { tip: l_tip(), rotation: 90.0, flip: true, at: [20.8, 15.2], ..eraser("pencil", l) });
    let fails: Vec<String> = clicks.iter().filter_map(|c| check_click(&mut h, c)).collect();
    assert!(fails.is_empty(), "{} of {} clicks:\n{}", fails.len(), clicks.len(), fails.join("\n"));
}

#[test]
fn a_real_click_matches_the_cursor_on_high_density_displays() {
    // At 2×, 1.5× and 1.25× the outline's corners snap to physical pixels, still on the
    // document's pixel edges.
    let at = [20.3, 15.6];
    for ppp in [2.0, 1.5, 1.25] {
        let mut h = cursor_canvas(ppp);
        let clicks = [
            Click::pencil(json!({"size": 9, "roundness": 1.0, "angle": 0}), at),
            Click::pencil(json!({"size": 4, "roundness": 1.0, "angle": 0}), [20.5, 15.5]),
            Click { rotation: 90.0, flip: true, ..Click::pencil(json!({"size": 9, "roundness": 0.4, "angle": 30}), at) },
            Click { tip: l_tip(), flip: true, ..Click::pencil(json!({"size": 12, "roundness": 1.0, "angle": 20, "flipX": true}), [20.8, 15.2]) },
        ];
        let fails: Vec<String> = clicks.iter().filter_map(|c| check_click(&mut h, c)).collect();
        assert!(fails.is_empty(), "ppp {ppp}: {} of {} clicks:\n{}", fails.len(), clicks.len(), fails.join("\n"));
    }
}

#[test]
fn the_cursor_is_round_exactly_when_the_pencil_dab_is() {
    // #2662: one dab per size; the cursor is the square only while the dab fills its corners.
    let mut app = app();
    for n in 1..=12u32 {
        let (x, y) = (8.0 + f64::from((n - 1) % 6) * 18.0 + 0.3, 15.0 + f64::from((n - 1) / 6) * 30.0 + 0.6);
        app.run("paint.pencil", json!({"points": [[x, y]], "size": n})).unwrap();
        let [x0, y0, ..] = photocraft_engine::paint::grid_square(x, y, n as f32);
        let corner = rgba(&app, x0 as i32, y0 as i32)[3] == 1.0;
        assert_eq!(pencil_tip_is_round(n as f32), !corner, "{n} px");
    }
}

#[test]
fn the_eraser_in_pencil_mode_erases_aliased_pixels() {
    // #2662: an opaque layer, then a soft Eraser in Pencil mode erases with hard edges.
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 0.0, "smoothing": {"amount": 0.0}}})).unwrap();
    app.ui.tool_options.eraser_mode = "pencil".into();
    drag(&mut app, &[(10.0, 40.0), (60.3, 25.7), (110.0, 40.0)], Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!((id.as_str(), &p["erase"]), ("paint.pencil", &json!(true)));
    let s = layer(&app).surface().unwrap().clone();
    let mut erased = 0;
    for y in 0..80 {
        for x in 0..120 {
            let a = s.rgba(x, y)[3];
            assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y})");
            erased += usize::from(a == 0.0);
        }
    }
    assert!(erased > 500, "{erased}");
}

/// Erase to History paints the source state in each Eraser mode, even on a layer made since.
#[test]
fn the_eraser_with_erase_to_history_paints_the_source_state_in_every_mode_even_on_a_layer_made_since() {
    for mode in ["brush", "pencil", "block"] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        let before = layer(&app).surface().unwrap().rgba(60, 40);
        assert!(before[3] > 0.99, "{before:?}");
        // State 1 = after the stroke; the layer made next did not exist then.
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01);
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        // Without the option the Eraser erases: the empty layer stays empty.
        drag(&mut app, &[(55.0, 40.0), (65.0, 40.0)], Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        app.ui.tool_options.erase_to_history = true;
        drag(&mut app, &[(55.0, 40.0), (65.0, 40.0)], Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}");
        assert_eq!((p["pencil"] == json!(true), p["block"] == json!(true)), (mode == "pencil", mode == "block"), "{mode}");
        let s = layer(&app).surface().unwrap().clone();
        let after = s.rgba(60, 40);
        assert!(after[3] > 0.99 && (0..3).all(|i| (after[i] - before[i]).abs() < 0.01), "{mode}: {after:?} vs {before:?}");
        // The hard tips leave only whole pixels: painted back in full or untouched.
        if mode != "brush" {
            for y in 0..80 {
                for x in 0..120 {
                    let a = s.rgba(x, y)[3];
                    assert!(a == 0.0 || a == 1.0, "{mode}: partial alpha {a} at ({x},{y})");
                }
            }
        }
    }
}

#[test]
fn erase_to_history_in_pencil_mode_ignores_the_brush_modes_flow() {
    let mut app = app();
    app.ui.tool = Tool::Eraser;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"flow": 0.5}})).unwrap();
    app.ui.tool_options.erase_to_history = true;
    app.ui.tool_options.eraser_mode = "brush".into();
    assert_eq!(crate::retouch_ui::history_brush_params(&app)["flow"], json!(50.0));
    app.ui.tool_options.eraser_mode = "pencil".into();
    assert_eq!(crate::retouch_ui::history_brush_params(&app)["flow"], json!(100.0));
}

/// ⌥ at the press reverses Erase to History for that stroke, and the option itself stays as it was.
#[test]
fn alt_at_the_press_reverses_erase_to_history_for_one_eraser_stroke() {
    for (mode, pencil) in [("brush", false), ("pencil", true)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        let painted = layer(&app).surface().unwrap().rgba(60, 40);
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        let stroke = |app: &mut PhotocraftApp, down: Modifiers, later: Modifiers| {
            tool_event(app, ToolEvent::Down { x: 55.0, y: 40.0, pressure: 1.0 }, down);
            tool_event(app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, later);
            tool_event(app, ToolEvent::Up { x: 65.0, y: 40.0 }, later);
        };
        // A plain stroke erases, and ⌥ first pressed during it changes nothing.
        stroke(&mut app, Modifiers::NONE, Modifiers::ALT);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        // ⌥ at the press paints the source back, and letting go of it during the stroke doesn't stop it.
        stroke(&mut app, Modifiers::ALT, Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}: {id} {p}");
        assert_eq!(p["pencil"] == json!(true), pencil, "{mode}: {p}");
        let after = layer(&app).surface().unwrap().rgba(60, 40);
        assert!(after[3] > 0.99 && (0..3).all(|i| (after[i] - painted[i]).abs() < 0.01), "{mode}: {after:?} vs {painted:?}");
        // The option itself stays off, so the next plain stroke erases again.
        assert!(!app.ui.tool_options.erase_to_history, "{mode}");
        stroke(&mut app, Modifiers::NONE, Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        // With the option on, a plain stroke paints the source back and ⌥ at the press erases.
        app.ui.tool_options.erase_to_history = true;
        stroke(&mut app, Modifiers::NONE, Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] > 0.99, "{mode}");
        stroke(&mut app, Modifiers::ALT, Modifiers::NONE);
        assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}");
        assert!(app.ui.tool_options.erase_to_history, "{mode}");
    }
}

/// A ⇧-click Erase to History line isn't smoothed (#11887) and keeps the Eraser's brush from the
/// press: the commit sends `smoothing: 0` beside the `brush` param, as its live preview does, so
/// the release takes the pixels the preview drew.
#[test]
fn a_shift_click_erase_to_history_line_commits_its_live_pixels_unsmoothed() {
    for mode in ["brush", "pencil", "block"] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 9, "hardness": 1.0, "smoothing": {"amount": 0.6, "catchUpOnEnd": false}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        app.ui.tool_options.erase_to_history = true;
        drag(&mut app, &[(20.0, 20.0), (40.0, 30.0)], Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 }, Modifiers::SHIFT);
        assert!(app.live_stroke.is_some(), "{mode}: a live line");
        app.retouch_from_live = false;
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 60.0 }, Modifiers::SHIFT);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}: {id} {p}");
        let ends: Vec<[f64; 2]> = p["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap()]).collect();
        // The Pencil tip's line runs between the clicked pixels' centres.
        let c = if mode == "pencil" { 0.5 } else { 0.0 };
        assert_eq!(ends, vec![[40.0 + c, 30.0 + c], [100.0 + c, 60.0 + c]], "{mode}");
        assert_eq!(p["smoothing"], json!(0.0), "{mode}: {p}");
        assert!(p["brush"].is_object(), "{mode}: the brush from the press: {p}");
        assert!(app.retouch_from_live, "{mode}: the release took the live pixels instead of rendering the line again");
        // The line reaches the click: the source is painted back at its far end.
        assert!(rgba(&app, 99, 60)[3] > 0.99, "{mode}");
        assert_eq!(app.session.tools.brush.smoothing.amount, 0.6, "{mode}: the Eraser keeps its Smoothing");
    }
}

/// A tool key pressed mid-stroke changes neither the stroke's options nor its tip.
#[test]
fn a_tool_key_during_an_erase_to_history_stroke_keeps_the_stroke_as_it_began() {
    for (mode, pencil) in [("brush", false), ("pencil", true)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        let before = layer(&app).surface().unwrap().rgba(60, 40);
        app.run("history.setSource", json!({"state": 1})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = mode.into();
        app.ui.tool_options.erase_to_history = true;
        tool_event(&mut app, ToolEvent::Down { x: 55.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        // What pressing a tool's letter does (`shortcuts`): the drag is not cancelled.
        app.ui.tool = Tool::Brush;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 90}})).unwrap();
        tool_event(&mut app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 65.0, y: 40.0 }, Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["eraseToHistory"]), ("paint.historyBrush", &json!(true)), "{mode}: {id} {p}");
        assert_eq!(p["pencil"] == json!(true), pencil, "{mode}: {p}");
        assert_eq!(p["size"], json!(15.0), "{mode}: the Eraser's size, not the new tool's");
        let after = layer(&app).surface().unwrap().rgba(60, 40);
        assert!(after[3] > 0.99 && (0..3).all(|i| (after[i] - before[i]).abs() < 0.01), "{mode}: {after:?} vs {before:?}");
    }
}

#[test]
fn a_tool_key_during_a_history_brush_stroke_keeps_the_brush_it_began_with() {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    app.run("history.setSource", json!({"state": 1})).unwrap();
    app.ui.tool = Tool::HistoryBrush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 0.5, "spacing": 0.3, "smoothing": {"amount": 0.0}}})).unwrap();
    tool_event(&mut app, ToolEvent::Down { x: 55.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    // What pressing a tool's letter does (`shortcuts`): the drag is not cancelled.
    app.ui.tool = Tool::Brush;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 90, "hardness": 1.0, "spacing": 0.1}})).unwrap();
    tool_event(&mut app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Up { x: 65.0, y: 40.0 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.historyBrush", "{p}");
    assert_eq!(p["size"], json!(15.0), "{p}");
    assert_eq!(p["hardness"].as_f64().map(f64::round), Some(50.0), "{p}");
    assert_eq!(p["spacing"].as_f64().map(f64::round), Some(30.0), "{p}");
    assert_eq!(p.get("eraseToHistory"), None, "{p}");
}

/// The layer after a History Brush (`mode` "") or Erase to History (`mode` "brush", "pencil",
/// "block") stroke whose tool brush is given `pressed`; with `switched`, the Brush tool is picked
/// halfway and its saved brush given `switched`, as a tool key does mid-stroke (the drag goes on).
fn history_stroke_pixels(mode: &str, pressed: &serde_json::Value, switched: Option<&serde_json::Value>) -> Vec<f32> {
    let mut app = app();
    app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
    app.run("history.setSource", json!({"state": 1})).unwrap();
    let tool = if mode.is_empty() { Tool::HistoryBrush } else { Tool::Eraser };
    if tool == Tool::Eraser {
        // A layer made since the source state: Erase to History paints that state's one layer.
        app.run("layer.new.layer", json!({})).unwrap();
    } else {
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400, "erase": true})).unwrap();
    }
    assert!(layer(&app).surface().unwrap().rgba(60, 40)[3] < 0.01, "{mode}: nothing there to start with");
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 15, "hardness": 1.0, "spacing": 0.1, "smoothing": {"amount": 0.0}}})).unwrap();
    app.run("tools.setBrush", json!({ "brush": pressed })).unwrap();
    if tool == Tool::Eraser {
        app.ui.tool_options.eraser_mode = mode.into();
        app.ui.tool_options.erase_to_history = true;
    }
    let pts = [(30.0, 40.0), (45.0, 44.0), (60.0, 37.0), (75.0, 43.0), (90.0, 40.0)];
    tool_event(&mut app, ToolEvent::Down { x: pts[0].0, y: pts[0].1, pressure: 1.0 }, Modifiers::NONE);
    for (i, &(x, y)) in pts.iter().enumerate().skip(1) {
        if i == 3
            && let Some(sw) = switched
        {
            // What pressing a tool's letter does (`shortcuts`): the drag is not cancelled.
            app.ui.tool = Tool::Brush;
            crate::paint_mouse::sync_tool_brush(&mut app);
            app.run("tools.setBrush", json!({ "brush": sw })).unwrap();
        }
        tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(&mut app, ToolEvent::Up { x: pts[4].0, y: pts[4].1 }, Modifiers::NONE);
    let (id, p) = last(&app);
    assert_eq!(id, "paint.historyBrush", "{mode}: {p}");
    // The mode's own flags: the pixel comparisons can't see one go, as both strokes would lose it.
    for flag in ["pencil", "block"] {
        assert_eq!(p.get(flag) == Some(&json!(true)), mode == flag, "{mode}: {flag} in {p}");
    }
    let st = app.session.active().unwrap();
    layer(&app).surface().unwrap().read_region(st.doc.bounds())
}

/// A tool key pressed mid-stroke leaves a History Brush or Erase to History stroke exactly as the
/// same stroke without it, with the brush it was pressed with: tip shape, sampled tip, dynamics
/// and smoothing, not only size, hardness and spacing (FredApps' review of #10692).
#[test]
fn a_tool_key_mid_stroke_paints_what_the_stroke_paints_without_it() {
    let round = json!({"tip": "round", "roundness": 1.0, "angle": 0.0});
    let cross = json!({"sampled": {"width": 3, "height": 3, "data": [0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0]}});
    let cases = [
        ("an elliptical tip, then a round one", json!({"roundness": 0.2, "angle": 30.0}), round.clone()),
        ("a round tip, then a sampled one", round.clone(), json!({"tip": cross})),
        ("a sampled tip, then a round one", json!({"tip": cross}), round.clone()),
        ("no dynamics, then size jitter", round.clone(), json!({"shapeDynamics": {"enabled": true, "size": {"jitter": 1.0}}})),
        ("no smoothing, then 90 %", round.clone(), json!({"smoothing": {"enabled": true, "amount": 0.9}})),
    ];
    // Each choice, not every mode with every case (#16041): the brush is captured once, at the
    // press, whatever the mode, so History Brush takes every case and each Erase to History mode
    // one, for its own flags: Brush and Pencil with the sampled tip, Block with smoothing (the only
    // brush setting Block keeps).
    let runs = cases.iter().map(|c| ("", c)).chain([("brush", &cases[2]), ("pencil", &cases[2]), ("block", &cases[4])]);
    for (mode, (what, pressed, switched)) in runs {
        let plain = history_stroke_pixels(mode, pressed, None);
        assert!(history_stroke_pixels(mode, pressed, Some(switched)) == plain, "{mode} {what}: the switch changed the stroke");
        // The switched-to brush paints differently, so the comparison means something.
        assert!(history_stroke_pixels(mode, switched, None) != plain, "{mode} {what}: both brushes paint alike");
    }
}
