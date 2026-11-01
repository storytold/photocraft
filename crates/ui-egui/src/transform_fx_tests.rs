//! Free Transform previews a styled layer with its style (#3653), through the real session and
//! canvas: at every bit depth and colour mode, for pixel layers and groups, following edits made
//! while the box is open, and matching what the commit leaves.

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

#[test]
fn styled_texture_tracks_monitor_changes_without_recoloring_effect_inputs() {
    let mut app = app("rgb", 16);
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    ctx.tex_manager().write().take_delta().clear();
    let raw = composite(&app, 1.0).unwrap().0;
    let mut previous = None;
    for monitor in ["srgb", "display-p3", "srgb"] {
        app.session.color.settings.monitor_profile = monitor.into();
        let display = app.session.color.canvas_display(&app.session.active().unwrap().doc).unwrap();
        let expected = crate::canvas::display_image(Some(&display), &raw);
        let (styled, frame) = crate::transform_tool::styled_frame(&app, 1.0, false).unwrap();
        let (id, _) = styled.texture(&ctx, &frame, Some(&display)).unwrap();
        assert_ne!(Some(id), previous, "monitor change invalidates the texture");
        let builds = styled.builds();
        assert_eq!(styled.texture(&ctx, &frame, Some(&display)).unwrap().0, id);
        assert_eq!(styled.builds(), builds, "unchanged display reuses the composite");
        let mut delta = ctx.tex_manager().write().take_delta();
        let egui::ImageData::Color(shown) = &delta.set.get(&id).unwrap().last().unwrap().image;
        assert_eq!(shown.pixels, expected.pixels, "monitor={monitor}");
        delta.clear();
        assert_eq!(composite(&app, 1.0).unwrap().0.px, raw.px, "effects still use document colors");
        previous = Some(id);
    }
}

/// What the box draws for the styled layer at `zoom`: `None` when it draws the bare pixels.
fn composite(app: &PhotocraftApp, zoom: f32) -> Option<(Buffer, Plan)> {
    let (_, frame) = crate::transform_tool::styled_frame(app, zoom, false)?;
    let (plan, work, level) = frame.plan()?;
    Some((frame.compose(&plan, &work, level)?, plan))
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
    // No style: nothing to composite, also if one is added once the box is open (its texels
    // were not read for it).
    let mut plain = app("rgb", 8);
    edit_layer(&mut plain, |l| l.effects.items.clear());
    begin(&mut plain);
    assert!(composite(&plain, 1.0).is_none());
    plain.run("layer.layerStyle.stroke", json!({"size": 4, "color": "#ff0000"})).unwrap();
    assert!(composite(&plain, 1.0).is_none());
    // The selected part of a styled layer: its style belongs to that part and the rest together.
    let mut part = app("rgb", 8);
    part.run("select.rect", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
    begin(&mut part);
    assert_eq!(part.ui.transform.as_ref().unwrap().rect, [60.0, 60.0, 80.0, 80.0]);
    assert!(composite(&part, 1.0).is_none());
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

/// A 400 × 300 document with a 200 × 100 red block at (100, 100) on layer "red", inside group
/// "G" whose mask reveals only the left half of the canvas (x < 200). "red" is active.
fn masked_group_app(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "depth": depth})).unwrap();
    app.run("layer.new.layer", json!({"name": "red"})).unwrap();
    let red = active(&app).0;
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.run("layer.new.groupFromLayers", json!({"name": "G"})).unwrap();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap();
    app.run("layer.layerMask.revealSelection", json!({})).unwrap();
    app.run("layer.select", json!({"layer": red})).unwrap();
    app.sync_views();
    app
}

/// Free Transform moving a layer inside a masked group (#9941): the group mask stays where it is
/// and hides the part of the moving layer that leaves it, as the commit does, instead of the
/// moving pixels showing through it.
#[test]
fn a_group_mask_cuts_the_moving_layer_while_the_box_is_open() {
    for depth in [8, 16, 32] {
        let mut app = masked_group_app(depth);
        begin(&mut app);
        // Moved 50 px right: the block is 150..350 × 100..200; the mask shows x < 200 only.
        set_quad(&mut app, [[150.0, 100.0], [350.0, 100.0], [350.0, 200.0], [150.0, 200.0]]);
        let c = composite(&app, 1.0).unwrap_or_else(|| panic!("{depth}: the box composites the layer through the group mask"));
        assert!(at(&c, 175.0, 150.0)[3] > 0.99, "{depth}: inside the mask: {:?}", at(&c, 175.0, 150.0));
        for x in [205.0, 250.0, 340.0] {
            assert_eq!(at(&c, x, 150.0)[3], 0.0, "{depth}: hidden by the group mask at x {x}");
        }
        // The same as the commit leaves it.
        crate::transform_tool::commit(&mut app);
        let doc = &app.session.active().unwrap().doc;
        let shown = photocraft_compose::render(doc, photocraft_geom::Rect::new(0, 0, 400, 300));
        assert!(shown.get(175, 150)[1] < 0.05 && shown.get(250, 150)[1] > 0.95, "{depth}: committed");
    }
}

/// A styled layer in a masked group: the style and the pixels are both cut by the mask.
#[test]
fn a_group_mask_cuts_a_styled_moving_layer_too() {
    let mut app = masked_group_app(8);
    app.run("layer.layerStyle.stroke", json!({"size": 4, "color": "#0000ff", "position": "outside"})).unwrap();
    begin(&mut app);
    set_quad(&mut app, [[150.0, 100.0], [350.0, 100.0], [350.0, 200.0], [150.0, 200.0]]);
    let c = composite(&app, 1.0).unwrap();
    assert!(at(&c, 147.0, 150.0)[3] > 0.99 && at(&c, 175.0, 150.0)[3] > 0.99, "stroke and pixels inside the mask");
    assert_eq!(at(&c, 250.0, 150.0)[3], 0.0, "pixels outside it");
    assert_eq!(at(&c, 352.0, 150.0)[3], 0.0, "stroke outside it");
}

/// Without a masked group nor a style the box keeps drawing the bare texels (no composite).
#[test]
fn an_unmasked_unstyled_layer_keeps_the_texel_mesh() {
    let mut app = masked_group_app(8);
    app.session
        .edit("unmask", |doc, _| {
            doc.layers.iter_mut().for_each(|l| l.mask = None);
            Ok(())
        })
        .unwrap();
    begin(&mut app);
    assert!(crate::transform_tool::styled_frame(&app, 1.0, false).is_none());
}

#[test]
fn clipping_follows_the_work_scale_and_refuses_hostile_scales() {
    use photocraft_compose::Buffer;
    use photocraft_geom::Rect;
    let mut surface = photocraft_raster::Surface::with_default(photocraft_color::PixelFormat::GRAY8, &[0.0]);
    surface.fill_rect(Rect::new(0, 0, 100, 100), &[1.0]);
    let masks = [crate::transform_tex::PreviewMask { surface, density: 1.0 }];
    // Half scale: work pixel 40 is document pixel 80 (shown), work pixel 60 is 120 (hidden).
    let mut b = Buffer::transparent(Rect::new(0, 0, 100, 10));
    b.px.iter_mut().for_each(|p| *p = [1.0; 4]);
    crate::transform_fx::clip(&mut b, 0.5, &masks);
    assert_eq!(b.get(40, 5)[3], 1.0);
    assert_eq!(b.get(60, 5)[3], 0.0);
    // Density 50 %: hidden pixels keep half their alpha.
    let half = [crate::transform_tex::PreviewMask { surface: masks[0].surface.clone(), density: 0.5 }];
    let mut b = Buffer::transparent(Rect::new(0, 0, 200, 1));
    b.px.iter_mut().for_each(|p| *p = [1.0; 4]);
    crate::transform_fx::clip(&mut b, 1.0, &half);
    assert!((b.get(150, 0)[3] - 0.5).abs() < 1e-6 && b.get(50, 0)[3] == 1.0);
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-300] {
        let mut b = Buffer::transparent(Rect::new(-5, -5, 5, 5));
        crate::transform_fx::clip(&mut b, scale, &masks);
    }
}
