//! The Pencil tool (#213): aliased strokes through the live-stroke path, Auto Erase, the
//! pixel-grid cursor, ⇧-click lines, Control+Alt resizing and the B group.

use std::collections::BTreeSet;

use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_paint::brush::TipShape;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform, tool_event};
use crate::state::Tool;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 80, "background": "transparent"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000", "background": "#ffffff"})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 3, "hardness": 0.0}})).unwrap();
    // Each tool keeps its own brush (#218): switch first, then set the Pencil's options, as the
    // options bar does (its first frame takes the Brush's tip at Photoshop's 10 % smoothing).
    app.ui.tool = Tool::Pencil;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"smoothing": {"amount": 0.0}}})).unwrap();
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
    let ctrl_alt = Modifiers { alt: true, ctrl: true, ..Default::default() };
    assert!(crate::brush_resize::applies(Tool::Pencil));
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
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center, v.fit_pending) = (8.0, [20.0, 15.0], false);
    h.run_steps(2);
    h
}

/// What the canvas paints for the cursor with the pointer at document point `at`: the shapes of a
/// frame with the pointer there that a frame without the pointer lacks.
fn cursor_shapes(h: &mut Harness<'_, PhotocraftApp>, at: [f64; 2]) -> (ViewXform, Vec<egui::Shape>) {
    h.event(egui::Event::PointerGone);
    h.run_steps(2);
    let mut idle = h.output().shapes.clone();
    let xf = ViewXform::active(h.state()).unwrap();
    h.event(egui::Event::PointerMoved(xf.to_screen(at[0] as f32, at[1] as f32)));
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
fn drawn_grid_edges(shapes: &[egui::Shape], xf: &ViewXform, edges: &mut GridEdges, other: &mut Vec<&'static str>) {
    use egui::Shape;
    // A pixel corner, give or take a quarter pixel: the outline's strokes sit just either side
    // of the pixel edges, on physical pixels.
    let corner = |p: Pos2| {
        let [x, y] = xf.to_doc(p);
        let (cx, cy) = (x.round(), y.round());
        ((x - cx).abs() < 0.25 && (y - cy).abs() < 0.25).then_some((cx as i64, cy as i64))
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
    let pos = ViewXform::active(h.state()).unwrap().to_screen(at[0] as f32, at[1] as f32);
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
    app.session.tools.brush.tip = c.tip.clone();
    app.ui.views[0].rotation = c.rotation;
    app.ui.view.flip_horizontal = c.flip;
    app.ui.view.pixel_aspect = c.pixel_aspect.into();
    let (xf, shapes) = cursor_shapes(h, c.at);
    if xf.aspect != crate::view_cmds::pixel_aspect_ratio(c.pixel_aspect) {
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

#[test]
fn the_eraser_in_block_mode_erases_a_16_screen_px_square() {
    // #2770: a soft, small, half-opacity Eraser in Block mode still clears a hard square of
    // 16 screen pixels: 16 document pixels at 100 %, 8 at 200 %.
    for (zoom, n) in [(1.0, 16usize), (2.0, 8)] {
        let mut app = app();
        app.run("paint.pencil", json!({"points": [[60.0, 40.0]], "size": 400})).unwrap();
        app.ui.tool = Tool::Eraser;
        crate::paint_mouse::sync_tool_brush(&mut app);
        app.run("tools.setBrush", json!({"brush": {"size": 5, "hardness": 0.0, "opacity": 0.5, "flow": 0.3, "smoothing": {"amount": 0.0}}})).unwrap();
        app.ui.tool_options.eraser_mode = "block".into();
        app.ui.views[0].zoom = zoom;
        drag(&mut app, &[(60.3, 40.6)], Modifiers::NONE);
        let (id, p) = last(&app);
        assert_eq!((id.as_str(), &p["block"]), ("paint.pencil", &json!(true)));
        let s = layer(&app).surface().unwrap().clone();
        let (mut erased, mut x0, mut y0, mut x1, mut y1) = (0, i32::MAX, i32::MAX, 0, 0);
        for y in 0..80 {
            for x in 0..120 {
                let a = s.rgba(x, y)[3];
                assert!(a == 0.0 || a == 1.0, "partial alpha {a} at ({x},{y}), zoom {zoom}");
                if a == 0.0 {
                    erased += 1;
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        assert_eq!((erased, x1 - x0 + 1, y1 - y0 + 1), (n * n, n as i32, n as i32), "zoom {zoom}");
    }
}

#[test]
fn b_cycles_brush_pencil_mixer_brush_and_color_replacement() {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        app
    });
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = false);
    h.run_steps(4);
    let press = |h: &mut Harness<'_, PhotocraftApp>, m: Modifiers| {
        h.event(egui::Event::Key { key: Key::B, physical_key: None, pressed: true, repeat: false, modifiers: m });
        h.event(egui::Event::Key { key: Key::B, physical_key: None, pressed: false, repeat: false, modifiers: m });
        h.run_steps(2);
    };
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement);
    // Use Shift Key for Tool Switch: ⇧B cycles, B keeps the group's tool.
    h.state_mut().session.edit_prefs(|p| p.tools.use_shift_key_for_tool_switch = true);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::Pencil);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::MixerBrush);
    press(&mut h, Modifiers::SHIFT);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement);
    press(&mut h, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::ColorReplacement);
    assert_eq!(Tool::from_name("pencil"), Some(Tool::Pencil));
    assert_eq!(Tool::from_name("mixerBrush"), Some(Tool::MixerBrush));
}
