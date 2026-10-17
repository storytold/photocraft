//! Free Transform previews a styled layer with its style (#3653), through the real session and
//! canvas: at every bit depth and colour mode, for pixel layers and groups, following edits made
//! while the box is open, and matching what the commit leaves.

use photocraft_color::BlendMode;
use photocraft_compose::Buffer;
use photocraft_doc::{Layer, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::transform_fx::Plan;

/// A 200 × 160 document with a 40 × 20 blue block at (60, 60) on its own layer, wearing a 4 px
/// red outside Stroke.
fn app(mode: &str, depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 160, "mode": mode, "depth": depth})).unwrap();
    app.run("layer.new.layer", json!({"name": "styled"})).unwrap();
    app.run("select.rect", json!({"x": 60, "y": 60, "width": 40, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.layerStyle.stroke", json!({"size": 4, "color": "#ff0000", "position": "outside"})).unwrap();
    app.sync_views();
    app
}

fn begin(app: &mut PhotocraftApp) {
    crate::transform_tool::begin(app, &egui::Context::default()).unwrap();
}

/// What the box draws for the styled layer at `zoom`: `None` when it draws the bare pixels.
fn composite(app: &PhotocraftApp, zoom: f32) -> Option<(Buffer, Plan)> {
    let (_, frame) = crate::transform_tool::styled_frame(app, zoom, false)?;
    let (plan, work, level) = frame.plan()?;
    let backdrop = match frame.under {
        Some(under) => Some(under_box(under, &plan)?),
        None => None,
    };
    let rest = match frame.rest {
        Some(_) => Some(frame.rest_at(&plan)?),
        None => None,
    };
    Some((frame.compose_over(&plan, &work, level, backdrop.as_ref(), rest.as_ref())?, plan))
}

/// The document under the box at the plan's scale, rendered as the preview renders it (from a
/// proxy when the scale allows one).
fn under_box(under: &photocraft_doc::Document, plan: &Plan) -> Option<Buffer> {
    let k = crate::transform_fx::proxy_factor(under, plan.scale);
    let proxy = (k > 1).then(|| photocraft_compose::proxy::proxy_document(under, k));
    crate::transform_fx::backdrop(proxy.as_ref().unwrap_or(under), k, plan)
}

/// The composite at a document point.
fn at(c: &(Buffer, Plan), x: f64, y: f64) -> [f32; 4] {
    let (x, y) = ((x * c.1.scale).floor() as i32, (y * c.1.scale).floor() as i32);
    if c.0.rect.contains(x, y) { c.0.get(x, y) } else { [0.0; 4] }
}

fn set_quad(app: &mut PhotocraftApp, quad: [[f64; 2]; 4]) {
    app.ui.transform.as_mut().unwrap().quad = quad;
}

fn active(app: &PhotocraftApp) -> LayerId {
    app.session.active().unwrap().active_layer.unwrap()
}

fn edit_layer(app: &mut PhotocraftApp, f: impl FnOnce(&mut Layer)) {
    let id = active(app);
    app.session
        .edit("edit", |doc, _| {
            f(doc.layer_mut(id).unwrap());
            Ok(())
        })
        .unwrap();
}

#[test]
fn the_style_shows_while_the_box_is_open_at_every_depth_and_mode() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("gray", 16), ("cmyk", 8), ("cmyk", 16), ("lab", 8)] {
        let mut app = app(mode, depth);
        begin(&mut app);
        let c = composite(&app, 1.0).unwrap_or_else(|| panic!("{mode}/{depth}: styled"));
        let (fill, stroke) = (at(&c, 80.0, 70.0), at(&c, 57.0, 70.0));
        assert!(fill[3] > 0.99 && stroke[3] > 0.99, "{mode}/{depth}: {fill:?} {stroke:?}");
        assert!(fill.iter().zip(stroke).any(|(a, b)| (a - b).abs() > 0.1), "{mode}/{depth}: the stroke has its own colour: {fill:?} {stroke:?}");
        if mode == "rgb" {
            assert!(stroke[0] > 0.9 && stroke[2] < 0.1 && fill[2] > 0.9 && fill[0] < 0.1, "{depth}: {fill:?} {stroke:?}");
        }
        assert_eq!(at(&c, 53.0, 70.0)[3], 0.0, "{mode}/{depth}: nothing past the stroke");
        // Twice the size about the middle: the block is 40..120 × 50..90 and the stroke is
        // still 4 px, drawn round the new edge.
        set_quad(&mut app, [[40.0, 50.0], [120.0, 50.0], [120.0, 90.0], [40.0, 90.0]]);
        let c = composite(&app, 1.0).unwrap();
        assert!(at(&c, 80.0, 70.0)[3] > 0.99 && at(&c, 37.0, 70.0)[3] > 0.99 && at(&c, 122.0, 70.0)[3] > 0.99, "{mode}/{depth}");
        assert!(at(&c, 37.0, 70.0).iter().zip(stroke).all(|(a, b)| (a - b).abs() < 0.02), "{mode}/{depth}: the same stroke");
        assert_eq!(at(&c, 33.0, 70.0)[3], 0.0, "{mode}/{depth}: the stroke did not grow with the box");
        assert_eq!(at(&c, 126.0, 70.0)[3], 0.0, "{mode}/{depth}");
    }
}

/// The preview is the layer as the commit leaves it: same stroke, same shadow, same place.
#[test]
fn the_preview_matches_the_committed_layer() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8)] {
        let mut app = app(mode, depth);
        app.run("layer.layerStyle.dropShadow", json!({"distance": 9, "size": 4, "opacity": 80, "add": true})).unwrap();
        begin(&mut app);
        // One and a half times the size, turned 20° about the middle of the block (80, 70).
        let (sin, cos) = 20f64.to_radians().sin_cos();
        let quad = [[-30.0, -15.0], [30.0, -15.0], [30.0, 15.0], [-30.0, 15.0]].map(|[x, y]: [f64; 2]| [80.0 + x * cos - y * sin, 70.0 + x * sin + y * cos]);
        set_quad(&mut app, quad);
        let (shown, plan) = composite(&app, 1.0).unwrap();
        crate::transform_tool::commit(&mut app);
        assert!(app.ui.transform.is_none(), "{mode}/{depth}: committed");
        // The committed layer alone, over transparency.
        let st = app.session.active().unwrap();
        let mut solo = (*st.doc).clone();
        let id = st.active_layer.unwrap();
        for l in solo.layers.iter_mut() {
            l.visible = l.id == id;
        }
        let committed = photocraft_compose::render(&solo, plan.shown);
        let premultiplied = |p: &[f32; 4]| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
        let (mut sum, mut off) = (0.0f64, 0usize);
        for (a, b) in shown.px.iter().zip(&committed.px) {
            let d = premultiplied(a).iter().zip(premultiplied(b)).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
            sum += f64::from(d);
            off += usize::from(d > 0.25);
        }
        let n = shown.px.len();
        assert!(n > 4000, "{mode}/{depth}: {n}");
        // The commit resamples bicubic and the preview bilinear, so soft edges differ a little
        // (a mean of 0.002 here); a stroke of another width or a shadow falling elsewhere would
        // differ over whole bands.
        assert!(sum / n as f64 <= 0.006, "{mode}/{depth}: mean difference {}", sum / n as f64);
        assert!(off * 500 <= n, "{mode}/{depth}: {off} of {n} pixels differ");
        // Something is there to compare: fill, stroke and shadow.
        assert!(committed.px.iter().filter(|p| p[3] > 0.5).count() > 2000, "{mode}/{depth}");
    }
}

/// Groups (like type, shapes and smart objects) preview from a flattened render of the layer,
/// which is read without the layer's own style and opacity: the box applies them itself.
#[test]
fn a_styled_group_previews_with_its_style_once() {
    let mut app = app("rgb", 8);
    app.session
        .edit("group", |doc, active| {
            let id = active.unwrap();
            let at = doc.layers.iter().position(|l| l.id == id).unwrap();
            let mut child = doc.layers.remove(at);
            let style = std::mem::take(&mut child.effects.items);
            let mut group = Layer::group("group", vec![child]);
            group.effects.items = style;
            group.opacity = 0.5;
            *active = Some(group.id);
            doc.layers.insert(at, group);
            Ok(())
        })
        .unwrap();
    begin(&mut app);
    let pv = app.transform_preview.as_ref().unwrap();
    // The texels are the bare pixels over their own bounds: no stroke, no opacity yet.
    assert_eq!(pv.texture.size(), [40, 20]);
    let (texels, _) = pv.texture.level_image(0).unwrap();
    assert!(texels.pixels.iter().all(|p| *p == egui::Color32::from_rgb(0, 0, 255)), "{:?}", texels.pixels.first());
    // The composite has the stroke round them, all of it at the group's 50 %.
    let c = composite(&app, 1.0).unwrap();
    let (fill, stroke) = (at(&c, 80.0, 70.0), at(&c, 57.0, 70.0));
    assert!((fill[3] - 0.5).abs() < 0.01 && fill[2] > 0.9, "{fill:?}");
    assert!((stroke[3] - 0.5).abs() < 0.01 && stroke[0] > 0.9, "{stroke:?}");
    assert_eq!(at(&c, 53.0, 70.0)[3], 0.0);
}

#[test]
fn edits_made_while_the_box_is_open_show_in_the_styled_preview() {
    let mut app = app("rgb", 8);
    begin(&mut app);
    assert!(composite(&app, 1.0).is_some());
    // Opacity.
    edit_layer(&mut app, |l| l.opacity = 0.3);
    let c = composite(&app, 1.0).unwrap();
    assert!((at(&c, 57.0, 70.0)[3] - 0.3).abs() < 0.01 && (at(&c, 80.0, 70.0)[3] - 0.3).abs() < 0.01);
    // Fill 0 %: the pixels go and the stroke stays (the outline-only look).
    edit_layer(&mut app, |l| {
        l.opacity = 1.0;
        l.fill_opacity = 0.0;
    });
    let c = composite(&app, 1.0).unwrap();
    assert_eq!(at(&c, 80.0, 70.0)[3], 0.0);
    assert!(at(&c, 57.0, 70.0)[3] > 0.99);
    // A wider stroke.
    app.run("layer.layerStyle.stroke", json!({"size": 10, "color": "#ff0000", "position": "outside"})).unwrap();
    assert!(at(&composite(&app, 1.0).unwrap(), 52.0, 70.0)[3] > 0.99);
    // The style switched off, then the layer hidden: the box draws the bare pixels (or nothing).
    edit_layer(&mut app, |l| l.effects.enabled = false);
    assert!(composite(&app, 1.0).is_none());
    edit_layer(&mut app, |l| l.effects.enabled = true);
    assert!(composite(&app, 1.0).is_some());
    edit_layer(&mut app, |l| l.visible = false);
    assert!(composite(&app, 1.0).is_none());
}

#[test]
fn layers_without_a_style_and_other_boxes_keep_the_plain_preview() {
    // No style: nothing to composite, until one is added while the box is open (a whole layer's
    // texels are always read without one).
    let mut plain = app("rgb", 8);
    edit_layer(&mut plain, |l| l.effects.items.clear());
    begin(&mut plain);
    assert!(composite(&plain, 1.0).is_none());
    plain.run("layer.layerStyle.stroke", json!({"size": 4, "color": "#ff0000"})).unwrap();
    assert!(composite(&plain, 1.0).is_some());
    // A selected part of a layer without a style or mode: the bare moving pixels, the rest left
    // in the document under the box.
    let mut part = app("rgb", 8);
    edit_layer(&mut part, |l| l.effects.items.clear());
    part.run("select.rect", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
    begin(&mut part);
    assert_eq!(part.ui.transform.as_ref().unwrap().rect, [60.0, 60.0, 80.0, 80.0]);
    assert!(composite(&part, 1.0).is_none());
    let pv = part.transform_preview.as_ref().unwrap();
    assert!(pv.doc.layer(active(&part)).unwrap().visible, "the rest stays in the document under the box");
    // Transform Selection moves an outline, not the layer.
    let mut outline = app("rgb", 8);
    outline.run("select.rect", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
    crate::transform_tool::begin_selection(&mut outline, &egui::Context::default()).unwrap();
    assert!(composite(&outline, 1.0).is_none());
    // Warp mode draws its own mesh.
    let mut warp = app("rgb", 8);
    crate::transform_tool::begin_warp(&mut warp, &egui::Context::default()).unwrap();
    assert!(composite(&warp, 1.0).is_none());
}

#[test]
fn the_work_follows_the_zoom_and_never_the_document_size() {
    let mut app = app("rgb", 8);
    begin(&mut app);
    // Zoomed out to 25 %: a sixteenth of the pixels.
    let (full, half) = (composite(&app, 1.0).unwrap(), composite(&app, 0.25).unwrap());
    assert_eq!(full.1.scale, 1.0);
    assert_eq!(half.1.scale, 0.25);
    assert!(half.0.px.len() * 8 < full.0.px.len(), "{} vs {}", half.0.px.len(), full.0.px.len());
    assert!(at(&half, 80.0, 70.0)[3] > 0.99 && at(&half, 57.0, 70.0)[3] > 0.99 && at(&half, 50.0, 70.0)[3] == 0.0);
    // Zoomed in: the document's own pixels, no finer.
    assert_eq!(composite(&app, 16.0).unwrap().1.scale, 1.0);
    // A box dragged out to an absurd size stays within the budget.
    set_quad(&mut app, [[-4.0e6, -4.0e6], [4.0e6, -4.0e6], [4.0e6, 4.0e6], [-4.0e6, 4.0e6]]);
    if let Some((buf, _)) = composite(&app, 1.0) {
        assert!(buf.px.len() as f64 <= crate::transform_fx::MAX_PIXELS);
    }
    // A box that isn't a box: the bare pixels, no panic.
    set_quad(&mut app, [[f64::NAN; 2]; 4]);
    assert!(composite(&app, 1.0).is_none());
    set_quad(&mut app, [[70.0, 70.0]; 4]);
    assert!(composite(&app, 1.0).is_none());
}

/// Through the real canvas: the composite is made once per change, not once per frame, the box
/// still drags, undoes and commits as before, and the committed layer keeps its style.
#[test]
fn the_canvas_draws_the_styled_preview_and_reuses_it_between_frames() {
    let mut h = crate::transform_undo_tests::harness();
    h.state_mut().run("layer.layerStyle.stroke", json!({"size": 4, "color": "#00ff00", "position": "outside"})).unwrap();
    h.run_steps(2);
    let steps = h.state().session.active().unwrap().history.entries().len();
    crate::transform_undo_tests::begin(&mut h);
    let builds = |h: &egui_kittest::Harness<'static, PhotocraftApp>| crate::transform_tool::styled_frame(h.state(), 1.0, false).unwrap().0.builds();
    assert_eq!(builds(&h), 1, "composited when the box opened");
    h.run_steps(5);
    assert_eq!(builds(&h), 1, "and kept while nothing changes");
    // Drag the bottom-right corner out: the composite follows the box.
    crate::transform_undo_tests::drag(&mut h, [200.0, 160.0], [260.0, 196.0]);
    let after_drag = builds(&h);
    assert!(after_drag > 1, "{after_drag}");
    let quad = h.state().ui.transform.as_ref().unwrap().quad;
    assert!(quad[2][0] > 250.0, "{quad:?}");
    h.run_steps(5);
    assert_eq!(builds(&h), after_drag, "at rest again");
    // Zooming rebuilds it for the new scale.
    h.state_mut().ui.views[0].zoom = 0.5;
    h.run_steps(2);
    assert_eq!(builds(&h), after_drag + 1);
    // Undo steps the box back (not the document), and the composite follows.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(2);
    assert_eq!(h.state().ui.transform.as_ref().unwrap().quad[2], [200.0, 160.0]);
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps);
    assert_eq!(builds(&h), after_drag + 2);
    // Enter commits: one history step, the style still on the layer.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let app = h.state();
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    let st = app.session.active().unwrap();
    assert!(photocraft_compose::effects::has_effects(st.doc.layer(st.active_layer.unwrap()).unwrap()));
}

/// A 200 × 160 document on an orange-brown background with a 40 × 20 blue block at (60, 60) on
/// its own layer in `blend` mode, without a style (#5513).
fn blended(mode: &str, depth: u32, blend: BlendMode) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 160, "mode": mode, "depth": depth})).unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#c08040"})).unwrap();
    app.run("layer.new.layer", json!({"name": "blended"})).unwrap();
    app.run("select.rect", json!({"x": 60, "y": 60, "width": 40, "height": 20})).unwrap();
    app.run("edit.fill", json!({"color": "#4080ff"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    edit_layer(&mut app, |l| l.blend = blend);
    app.sync_views();
    app
}

/// Straight-alpha `top` over `under`.
fn over(top: [f32; 4], under: [f32; 4]) -> [f32; 4] {
    let a = top[3] + under[3] * (1.0 - top[3]);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let c = |k: usize| (top[k] * top[3] + under[k] * under[3] * (1.0 - top[3])) / a;
    [c(0), c(1), c(2), a]
}

/// What the canvas shows over the plan's area while the box is open at zoom 1: the document under
/// the box with the composite drawn over it.
fn on_canvas(app: &PhotocraftApp) -> (Buffer, Plan) {
    let (c, plan) = composite(app, 1.0).expect("composited");
    let under = photocraft_compose::render(&app.transform_preview.as_ref().unwrap().doc, plan.shown);
    (Buffer { rect: plan.shown, px: c.px.iter().zip(&under.px).map(|(p, u)| over(*p, *u)).collect() }, plan)
}

/// The box turned 20° and one and a half times the size, about the middle of the block (80, 70).
fn turned() -> [[f64; 2]; 4] {
    let (sin, cos) = 20f64.to_radians().sin_cos();
    [[-30.0, -15.0], [30.0, -15.0], [30.0, 15.0], [-30.0, 15.0]].map(|[x, y]: [f64; 2]| [80.0 + x * cos - y * sin, 70.0 + x * sin + y * cos])
}

/// Mean and count of the pixels of `a` and `b` that differ by more than a quarter.
fn difference(a: &Buffer, b: &Buffer) -> (f64, usize) {
    let premultiplied = |p: &[f32; 4]| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
    let (mut sum, mut off) = (0.0f64, 0usize);
    for (x, y) in a.px.iter().zip(&b.px) {
        let d = premultiplied(x).iter().zip(premultiplied(y)).map(|(p, q)| (p - q).abs()).fold(0.0f32, f32::max);
        sum += f64::from(d);
        off += usize::from(d > 0.25);
    }
    (sum / a.px.len().max(1) as f64, off)
}

/// The commit of the box as it is, then the document's composite over `rect`.
fn committed(app: &mut PhotocraftApp, rect: photocraft_geom::Rect) -> Buffer {
    crate::transform_tool::commit(app);
    assert!(app.ui.transform.is_none(), "committed");
    photocraft_compose::render(&app.session.active().unwrap().doc, rect)
}

#[test]
fn a_blend_mode_shows_while_the_box_is_open_at_every_depth_and_mode() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("gray", 16), ("cmyk", 8), ("cmyk", 16), ("lab", 8)] {
        let mut app = blended(mode, depth, BlendMode::Multiply);
        begin(&mut app);
        let (shown, plan) = on_canvas(&app);
        let block = at(&(shown, plan), 80.0, 70.0);
        let normal = {
            let mut n = blended(mode, depth, BlendMode::Normal);
            begin(&mut n);
            assert!(composite(&n, 1.0).is_none(), "{mode}/{depth}: a Normal layer keeps the plain preview");
            photocraft_compose::render(&n.session.active().unwrap().doc, photocraft_geom::Rect::new(80, 70, 81, 71)).px[0]
        };
        assert!(block.iter().zip(normal).take(3).any(|(a, b)| (a - b).abs() > 0.1), "{mode}/{depth}: multiplied, not drawn in Normal: {block:?} vs {normal:?}");
        if mode == "rgb" {
            // #c08040 × #4080ff (3/4 × 1/4, 1/2 × 1/2, 1/4 × 1), whatever the depth.
            let want = [0.75 * 0.25, 0.5 * 0.5, 0.25];
            assert!(block.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.02), "{depth}: {block:?}");
        }
        set_quad(&mut app, turned());
        // Bilinear like the preview: a bicubic commit rings at the box's corners, and in Lab that
        // tints them (a corner pixel of this blue block commits as [0.75, 0.53, 1.0]).
        app.ui.transform.as_mut().unwrap().interpolation = "bilinear".into();
        let (shown, plan) = on_canvas(&app);
        // The canvas under the box, where the layer leaves it alone, is not drawn over.
        let (c, _) = composite(&app, 1.0).unwrap();
        assert!(c.px.iter().filter(|p| p[3] == 0.0).count() * 3 > c.px.len(), "{mode}/{depth}: only the layer's pixels are drawn");
        let after = committed(&mut app, plan.shown);
        let (mean, off) = difference(&shown, &after);
        assert!(mean <= 0.006 && off * 500 <= shown.px.len(), "{mode}/{depth}: mean {mean}, {off} of {} pixels differ", shown.px.len());
    }
}

#[test]
fn every_blend_mode_previews_as_the_commit_draws_it() {
    use BlendMode::*;
    for blend in [
        Darken,
        Multiply,
        ColorBurn,
        LinearBurn,
        DarkerColor,
        Lighten,
        Screen,
        ColorDodge,
        LinearDodge,
        LighterColor,
        Overlay,
        SoftLight,
        HardLight,
        VividLight,
        LinearLight,
        PinLight,
        HardMix,
        Difference,
        Exclusion,
        Subtract,
        Divide,
        Hue,
        Saturation,
        Color,
        Luminosity,
    ] {
        let mut app = blended("rgb", 8, blend);
        // At half opacity too, which mixes the blended colour with what is under it.
        edit_layer(&mut app, |l| l.opacity = 0.5);
        begin(&mut app);
        set_quad(&mut app, turned());
        let (shown, plan) = on_canvas(&app);
        let after = committed(&mut app, plan.shown);
        let (mean, off) = difference(&shown, &after);
        assert!(mean <= 0.006 && off * 500 <= shown.px.len(), "{blend:?}: mean {mean}, {off} of {} pixels differ", shown.px.len());
    }
}

/// A group in a blend mode (like type, shapes and smart objects) previews from a flattened render
/// of it, read at full opacity: the box applies its mode and opacity once.
#[test]
fn a_blending_group_previews_its_mode_and_opacity_once() {
    let mut app = blended("rgb", 8, BlendMode::Normal);
    app.session
        .edit("group", |doc, active| {
            let id = active.unwrap();
            let at = doc.layers.iter().position(|l| l.id == id).unwrap();
            let child = doc.layers.remove(at);
            let mut group = Layer::group("group", vec![child]);
            group.blend = BlendMode::Screen;
            group.opacity = 0.5;
            *active = Some(group.id);
            doc.layers.insert(at, group);
            Ok(())
        })
        .unwrap();
    begin(&mut app);
    let (texels, _) = app.transform_preview.as_ref().unwrap().texture.level_image(0).unwrap();
    assert!(texels.pixels.iter().all(|p| p.a() == 255), "the texels are read at full opacity");
    set_quad(&mut app, turned());
    let (shown, plan) = on_canvas(&app);
    let after = committed(&mut app, plan.shown);
    let (mean, off) = difference(&shown, &after);
    assert!(mean <= 0.006 && off * 500 <= shown.px.len(), "mean {mean}, {off} differ");
}

/// A group, type or shape layer without a style or mode is still drawn as the bare pixels, tinted
/// by its opacity once: its texels are read at full opacity.
#[test]
fn a_translucent_group_is_drawn_at_its_opacity_once() {
    let mut app = blended("rgb", 8, BlendMode::Normal);
    app.session
        .edit("group", |doc, active| {
            let id = active.unwrap();
            let at = doc.layers.iter().position(|l| l.id == id).unwrap();
            let child = doc.layers.remove(at);
            let mut group = Layer::group("group", vec![child]);
            group.opacity = 0.5;
            *active = Some(group.id);
            doc.layers.insert(at, group);
            Ok(())
        })
        .unwrap();
    begin(&mut app);
    assert!(composite(&app, 1.0).is_none());
    let pv = app.transform_preview.as_ref().unwrap();
    let (texels, _) = pv.texture.level_image(0).unwrap();
    assert!(texels.pixels.iter().all(|p| p.a() == 255), "{:?}", texels.pixels.first());
    assert!((pv.opacity - 0.5).abs() < 1e-6);
}

/// Where the layers under the canvas aren't what the layer blends with, the box draws it in
/// Normal as before rather than blending it with the wrong pixels.
#[test]
fn a_blend_mode_previews_only_against_what_it_blends_with() {
    let blends = |app: &mut PhotocraftApp| {
        begin(app);
        let shown = composite(app, 1.0).is_some();
        crate::transform_tool::cancel(app);
        shown
    };
    let grouped = |blend: BlendMode, f: &dyn Fn(&mut Layer)| {
        let mut app = blended("rgb", 8, BlendMode::Multiply);
        app.session
            .edit("group", |doc, active| {
                let id = active.unwrap();
                let at = doc.layers.iter().position(|l| l.id == id).unwrap();
                let child = doc.layers.remove(at);
                let mut group = Layer::group("group", vec![child]);
                group.blend = blend;
                f(&mut group);
                doc.layers.insert(at, group);
                Ok(())
            })
            .unwrap();
        app
    };
    assert!(blends(&mut blended("rgb", 8, BlendMode::Multiply)));
    // In a Pass Through group, the layer still blends with the layers under the group.
    assert!(blends(&mut grouped(BlendMode::PassThrough, &|_| {})));
    // In a Normal group it blends with the group's own layers only; a group at 50 % or with a
    // mask changes what it blends to.
    assert!(!blends(&mut grouped(BlendMode::Normal, &|_| {})));
    assert!(!blends(&mut grouped(BlendMode::PassThrough, &|g| g.opacity = 0.5)));
    // A Normal layer over it covers it (drawn over the box), so it still blends.
    let mut covered = blended("rgb", 8, BlendMode::Multiply);
    let id = active(&covered);
    covered.run("layer.new.layer", json!({"name": "over"})).unwrap();
    covered.run("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    covered.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    covered.run("select.deselect", json!({})).unwrap();
    covered.session.active_mut().unwrap().active_layer = Some(id);
    assert!(blends(&mut covered));
    // A Screen layer over it stays in the canvas under the box: it would apply to the moving
    // pixels too, so they are drawn in Normal.
    let over = covered.session.active().unwrap().doc.layers.last().unwrap().id;
    covered
        .session
        .edit("screen", |doc, _| {
            doc.layer_mut(over).unwrap().blend = BlendMode::Screen;
            Ok(())
        })
        .unwrap();
    assert!(!blends(&mut covered));
    // A layer clipped to the one under it blends within that clipping group.
    let mut clipped = blended("rgb", 8, BlendMode::Multiply);
    edit_layer(&mut clipped, |l| l.clipped = true);
    assert!(!blends(&mut clipped));
    // The selected part of a layer blends as one with the rest of it, composited over it (#6635).
    let mut part = blended("rgb", 8, BlendMode::Multiply);
    part.run("select.rect", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
    assert!(blends(&mut part));
}

#[test]
fn a_blend_mode_set_while_the_box_is_open_shows_at_once() {
    let mut app = blended("rgb", 8, BlendMode::Normal);
    begin(&mut app);
    assert!(composite(&app, 1.0).is_none());
    edit_layer(&mut app, |l| l.blend = BlendMode::Multiply);
    let c = composite(&app, 1.0).expect("multiplied");
    assert!((at(&c, 80.0, 70.0)[0] - 0.75 * 0.25).abs() < 0.02, "{:?}", at(&c, 80.0, 70.0));
    edit_layer(&mut app, |l| l.blend = BlendMode::Normal);
    assert!(composite(&app, 1.0).is_none());
    edit_layer(&mut app, |l| {
        l.blend = BlendMode::Screen;
        l.visible = false;
    });
    assert!(composite(&app, 1.0).is_none(), "a hidden layer draws nothing");
}

/// Through the real canvas: the document under the box is rendered once and reused while the box
/// is dragged nearby; a new zoom renders it again at the new scale; the commit keeps the mode.
#[test]
fn the_canvas_renders_the_layers_under_a_blending_layer_once() {
    let mut h = crate::transform_undo_tests::harness();
    let id = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut()
        .session
        .edit("multiply", |doc, _| {
            doc.layer_mut(id).unwrap().blend = BlendMode::Multiply;
            Ok(())
        })
        .unwrap();
    h.run_steps(2);
    crate::transform_undo_tests::begin(&mut h);
    let counts = |h: &egui_kittest::Harness<'static, PhotocraftApp>| {
        let (s, _) = crate::transform_tool::styled_frame(h.state(), 1.0, false).unwrap();
        (s.builds(), s.backdrops())
    };
    assert_eq!(counts(&h), (1, 1), "composited onto the canvas when the box opened");
    h.run_steps(5);
    assert_eq!(counts(&h), (1, 1));
    crate::transform_undo_tests::drag(&mut h, [200.0, 160.0], [230.0, 180.0]);
    let (builds, backdrops) = counts(&h);
    assert!(builds > 1, "{builds}");
    assert_eq!(backdrops, 1, "a box dragged nearby reuses the layers under it");
    h.state_mut().ui.views[0].zoom = 0.5;
    h.run_steps(2);
    assert_eq!(counts(&h), (builds + 1, 2));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    assert!(h.state().ui.transform.is_none());
    assert_eq!(st.doc.layer(id).unwrap().blend, BlendMode::Multiply);
}

/// The box moved 30 px right.
fn moved(r: [f64; 4]) -> [[f64; 2]; 4] {
    [[r[0] + 30.0, r[1]], [r[2] + 30.0, r[1]], [r[2] + 30.0, r[3]], [r[0] + 30.0, r[3]]]
}

/// The selected part of a styled layer moves with the style drawn round it and the rest of the
/// layer together, as the commit draws it (#6635): the stroke closes round the rest's new edge and
/// round the moved part, and the rest isn't drawn twice (the layer leaves the canvas under the box).
#[test]
fn the_selected_part_of_a_styled_layer_previews_with_the_style_round_both() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("gray", 8), ("cmyk", 8), ("lab", 8)] {
        let mut app = app(mode, depth);
        // The left half of the blue block, 60..80 × 60..80.
        app.run("select.rect", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
        begin(&mut app);
        let rect = app.ui.transform.as_ref().unwrap().rect;
        let pv = app.transform_preview.as_ref().unwrap();
        assert!(!pv.doc.layer(active(&app)).unwrap().visible, "{mode}/{depth}: the whole layer leaves the canvas under the box");
        set_quad(&mut app, moved(rect));
        app.ui.transform.as_mut().unwrap().interpolation = "bilinear".into();
        let (shown, plan) = composite(&app, 1.0).unwrap_or_else(|| panic!("{mode}/{depth}: composited"));
        // The stroke now runs down the rest's cut edge (x 80..84) and round the moved part.
        assert!(at(&(shown.clone(), plan), 82.0, 70.0)[3] > 0.9, "{mode}/{depth}: stroke along the cut");
        assert!(at(&(shown.clone(), plan), 92.0, 70.0)[3] > 0.9, "{mode}/{depth}: the moved part");
        crate::transform_tool::commit(&mut app);
        assert!(app.ui.transform.is_none(), "{mode}/{depth}");
        let st = app.session.active().unwrap();
        let mut solo = (*st.doc).clone();
        let id = st.active_layer.unwrap();
        for l in solo.layers.iter_mut() {
            l.visible = l.id == id;
        }
        let after = photocraft_compose::render(&solo, plan.shown);
        let (mean, off) = difference(&shown, &after);
        assert!(mean <= 0.006 && off * 500 <= shown.px.len(), "{mode}/{depth}: mean {mean}, {off} of {} pixels differ", shown.px.len());
    }
}

/// The selected part of a Multiply layer blends with the layers under it, together with the rest
/// of the layer, as the commit draws it.
#[test]
fn the_selected_part_of_a_blending_layer_previews_its_mode() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("gray", 8), ("cmyk", 8)] {
        let mut app = blended(mode, depth, BlendMode::Multiply);
        app.run("select.rect", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
        begin(&mut app);
        let rect = app.ui.transform.as_ref().unwrap().rect;
        set_quad(&mut app, moved(rect));
        app.ui.transform.as_mut().unwrap().interpolation = "bilinear".into();
        let (shown, plan) = on_canvas(&app);
        let after = committed(&mut app, plan.shown);
        let (mean, off) = difference(&shown, &after);
        assert!(mean <= 0.006 && off * 500 <= shown.px.len(), "{mode}/{depth}: mean {mean}, {off} of {} pixels differ", shown.px.len());
    }
}

/// Through the real canvas: the rest of the layer is rendered once for a scale and reused while
/// the part is dragged; the commit keeps the style.
#[test]
fn the_rest_of_a_moving_part_is_rendered_once_per_scale() {
    let mut h = crate::transform_undo_tests::harness();
    h.state_mut().run("layer.layerStyle.stroke", json!({"size": 4, "color": "#00ff00", "position": "outside"})).unwrap();
    h.state_mut().run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 60})).unwrap();
    h.run_steps(2);
    crate::transform_undo_tests::begin(&mut h);
    let builds = |h: &egui_kittest::Harness<'static, PhotocraftApp>| crate::transform_tool::styled_frame(h.state(), 1.0, false).unwrap().0.builds();
    assert_eq!(builds(&h), 1);
    let rect = h.state().ui.transform.as_ref().unwrap().rect;
    // The bottom-right corner out: the part grows, the rest stays.
    crate::transform_undo_tests::drag(&mut h, [rect[2], rect[3]], [rect[2] + 40.0, rect[3] + 30.0]);
    assert!(h.state().ui.transform.as_ref().unwrap().quad[2][0] > rect[2] + 30.0);
    let after_drag = builds(&h);
    assert!(after_drag > 1, "{after_drag}");
    let rests = |h: &egui_kittest::Harness<'static, PhotocraftApp>| crate::transform_tool::styled_frame(h.state(), 1.0, false).unwrap().0.rests();
    assert_eq!(rests(&h), 1, "the rest is rendered once while the part moves");
    h.run_steps(5);
    assert_eq!(builds(&h), after_drag, "at rest again");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    assert!(h.state().ui.transform.is_none());
    assert!(photocraft_compose::effects::has_effects(st.doc.layer(st.active_layer.unwrap()).unwrap()));
}
