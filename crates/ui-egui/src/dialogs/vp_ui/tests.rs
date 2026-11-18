use craft::behavior::PixelSize;
use egui::{Color32, ColorImage};
use egui_kittest::Harness;
use photocraft_geom::Rect;

use super::*;

fn app_with_layer() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 240, "height": 160, "depth": 16})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#4080c0"})).unwrap();
    app.sync_views();
    app
}

/// The dialog host's pass; it runs the `TAKERS`, which include this module's `take`.
fn draw(ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>) {
    crate::dialogs::run(app, ui);
}

/// Vanishing Point opened on the active layer of `app`, as the window shows it.
fn open(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let app_ctx = app.ctx().clone();
    let mut h = Harness::builder().with_context(app_ctx).with_size(egui::vec2(1200.0, 800.0)).build_ui_state(draw, app);
    let ctx = h.ctx.clone();
    begin(h.state_mut(), &ctx).unwrap();
    h.run_steps(4);
    h
}

/// Sets workspace fields as `ui.dialog.set` would, then runs the passes that report them and waits for the preview job the
/// revision schedules (the stamp preview is painted by that job, as `ui.waitPreview` waits for it).
fn set(h: &mut Harness<'static, Box<PhotocraftApp>>, ui: Value) {
    let ctx = h.ctx.clone();
    crate::control::dialogs::workspace_ui(h.state_mut(), &ctx, VP, &ui).unwrap();
    h.run_steps(4);
    crate::control::dialogs::workspace_ui(h.state_mut(), &ctx, VP, &json!({"waitPreview": true})).unwrap();
    h.run_steps(2);
}

fn source(h: &Harness<'static, Box<PhotocraftApp>>) -> Arc<PcVanishingSource> {
    h.state().vanishing.as_ref().expect("Vanishing Point is open").source.clone()
}

/// The stamp preview's pixel at `(x, y)`.
fn shown(h: &Harness<'static, Box<PhotocraftApp>>, x: i32, y: i32) -> [u8; 4] {
    let source = source(h);
    let picture = source.stamp_preview().expect("a stamp preview");
    let mut out = ColorImage::new([1, 1], vec![Color32::TRANSPARENT]);
    picture.read(PixelRect::new(x, y, x + 1, y + 1), 1, &mut out);
    out.pixels[0].to_array()
}

fn close(a: [u8; 4], b: [f32; 4]) -> bool {
    (0..4).all(|i| (f32::from(a[i]) - b[i] * 255.0).abs() <= 3.0)
}

/// OK through the automation surface, with the passes that answer it and close the workspace.
fn ok(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let ctx = h.ctx.clone();
    crate::control::dialogs::workspace_ui(h.state_mut(), &ctx, VP, &json!({"commit": true})).unwrap();
    h.run_steps(4);
    assert!(h.state().vanishing.is_none(), "OK closed the workspace");
}

/// What the committed command left: the layer's pixels, or the visible image when the result went to a new layer.
fn committed(app: &PhotocraftApp, layer: LayerId, new_layer: bool) -> Surface {
    let st = app.session.active().unwrap();
    if new_layer { visible_image(&st.doc) } else { st.doc.layer(layer).unwrap().surface().unwrap().clone() }
}

fn floor() -> Value {
    json!({"corners": [[70, 60], [170, 60], [220, 150], [20, 150]]})
}

fn full_plane() -> Value {
    json!({"corners": [[10, 10], [230, 10], [230, 150], [10, 150]]})
}

/// A red square on the clipboard (copied from the layer's top-left corner).
fn copy_red_square(app: &mut PhotocraftApp) {
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("edit.copy", json!({})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
}

#[test]
fn a_plane_and_ok_paste_in_one_step() {
    let mut app = app_with_layer();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("edit.copy", json!({})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = open(app);
    set(&mut h, json!({"planes": [floor()]}));
    assert!(source(&h).stamp_preview().is_none(), "no strokes, no stamp preview");
    let before = h.state().session.active().unwrap().history.past_len();
    ok(&mut h);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    // The paste landed on the plane: red at its middle, blue outside it.
    let l = committed(app, layer, false);
    let hm = VpPlane { corners: [[70.0, 60.0], [170.0, 60.0], [220.0, 150.0], [20.0, 150.0]] }.homography().unwrap();
    let (x, y) = hm.apply(0.5, 0.35);
    assert!(l.rgba(x as i32, y as i32)[0] > 0.9, "{:?}", l.rgba(x as i32, y as i32));
    assert!(l.rgba(230, 20)[0] < 0.5, "outside the plane");
}

#[test]
fn a_torn_wall_and_clone_strokes_are_one_step_and_preview_on_the_source() {
    let mut app = app_with_layer();
    // A green mark near the floor's front edge to clone.
    app.session.execute("select.rect", json!({"x": 100, "y": 135, "width": 40, "height": 8})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = open(app);
    let stroke = json!({"source": [120, 139], "points": [[118, 100], [122, 100], [126, 100]], "size": 12, "hardness": 80});
    set(&mut h, json!({"planes": [floor(), {"from": 0, "edge": "top", "angle": 90, "depth": 0.5}], "tool": "stamp", "clone": [stroke]}));
    // The command carries the torn plane as such.
    let id = h.state().vanishing.as_ref().unwrap().dialog;
    let command = h.state().framework.dialogs.get_by_id::<Hosted>(id).unwrap().workspace().command(false);
    let planes = serde_json::to_value(&command).unwrap()["planes"].clone();
    assert_eq!((planes[1]["from"].clone(), planes[1]["edge"].clone()), (json!(0), json!("top")), "{planes}");
    // The source's raster shows the stroke; the document is untouched until OK.
    assert!(shown(&h, 122, 100)[1] > 200, "{:?}", shown(&h, 122, 100));
    let st = h.state().session.active().unwrap();
    assert!(st.doc.layer(layer).unwrap().surface().unwrap().rgba(122, 100)[1] < 0.6);
    // A second stroke repaints only its own area.
    let revision = source(&h).stamp_preview().unwrap().revision();
    let second = json!({"source": [120, 139], "points": [[60, 100]], "size": 12});
    set(&mut h, json!({"clone": [stroke, second]}));
    let src = source(&h);
    let area = src.stamp_preview().unwrap().damage(revision).expect("damage since the first stroke");
    assert!(area.x0 <= 60 && 60 < area.x1 && area.x1 < 122 && area.y0 > 20, "only the new stroke: {area:?}");

    let before = h.state().session.active().unwrap().history.past_len();
    ok(&mut h);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.history.past_len(), before + 1, "one history step for the whole session");
    assert!(committed(h.state(), layer, false).rgba(122, 100)[1] > 0.8);
}

#[test]
fn overlapping_stamp_preview_matches_commit_across_batches() {
    for new_layer in [false, true] {
        let mut app = app_with_layer();
        app.session
            .edit("pattern", |doc, active| {
                let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for x in 0..240 {
                    surf.fill_rect(Rect::new(x, 0, x + 1, 160), &[x as f32 / 240.0, (x % 7) as f32 / 6.0, 0.3, 1.0]);
                }
                Ok(())
            })
            .unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let original = committed(&app, layer, false);
        let mut h = open(app);
        set(&mut h, json!({"planes": [full_plane()], "newLayer": new_layer}));
        let points = [[80.0, 80.0], [84.0, 80.0], [88.0, 80.0], [92.0, 80.0]];
        let stroke = |n: usize| json!({"source": [76.0, 80.0], "points": &points[..n], "size": 12, "hardness": 80});
        set(&mut h, json!({"clone": [stroke(2)]}));
        set(&mut h, json!({"clone": [stroke(4)]}));
        let at: Vec<(i32, i32)> = (70..100).step_by(2).flat_map(|x| [76, 80, 84].map(|y| (x, y))).collect();
        let preview: Vec<[u8; 4]> = at.iter().map(|&(x, y)| shown(&h, x, y)).collect();
        assert!(at.iter().zip(&preview).any(|(&(x, y), p)| !close(*p, original.rgba(x, y))), "the strokes changed pixels");
        ok(&mut h);
        let done = committed(h.state(), layer, new_layer);
        for (&(x, y), p) in at.iter().zip(&preview) {
            assert!(close(*p, done.rgba(x, y)), "({x}, {y}) newLayer={new_layer}: {p:?} vs {:?}", done.rgba(x, y));
        }
    }
}

#[test]
fn stamp_preview_paints_over_the_paste_like_ok() {
    for new_layer in [false, true] {
        let mut app = app_with_layer();
        copy_red_square(&mut app);
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let mut h = open(app);
        set(&mut h, json!({"planes": [full_plane()], "newLayer": new_layer}));
        // The paste covers x 65..175 from y 45 down. One stroke clones it onto the base layer, the other paints across
        // its right edge.
        set(
            &mut h,
            json!({"clone": [
                {"source": [100, 60], "points": [[40, 120], [48, 120], [56, 120]], "size": 16, "hardness": 80},
                {"source": [40, 130], "points": [[150, 60], [170, 60], [190, 60]], "size": 16, "hardness": 80},
            ]}),
        );
        let at: Vec<(i32, i32)> = (30..200).step_by(10).flat_map(|x| (40..130).step_by(10).map(move |y| (x, y))).collect();
        let preview: Vec<[u8; 4]> = at.iter().map(|&(x, y)| shown(&h, x, y)).collect();
        assert!(shown(&h, 100, 60)[0] > 230, "the preview shows the paste");
        ok(&mut h);
        let done = committed(h.state(), layer, new_layer);
        for (&(x, y), p) in at.iter().zip(&preview) {
            assert!(close(*p, done.rgba(x, y)), "({x}, {y}) newLayer={new_layer}: {p:?} vs {:?}", done.rgba(x, y));
        }
    }
}

#[test]
fn moving_the_paste_repaints_the_stamp_preview() {
    let mut app = app_with_layer();
    copy_red_square(&mut app);
    let mut h = open(app);
    set(&mut h, json!({"planes": [full_plane()]}));
    set(&mut h, json!({"clone": [{"source": [200, 130], "points": [[30, 130]], "size": 8}]}));
    assert!(shown(&h, 100, 60)[0] > 230);
    set(&mut h, json!({"pasteAt": [0.0, 0.5]}));
    assert!(shown(&h, 100, 60)[0] < 128, "the paste left its old place");
    assert!(shown(&h, 30, 100)[0] > 230, "and shows at the new one");
    set(&mut h, json!({"paste": false}));
    assert!(shown(&h, 30, 100)[0] < 128, "paste off: gone from the preview");
}

#[test]
fn new_layer_stamp_preview_stays_on_the_canvas() {
    let mut app = app_with_layer();
    // A red stripe where the first dab samples, so cloning it changes pixels.
    app.session.execute("select.rect", json!({"x": 130, "y": 70, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let mut h = open(app);
    set(&mut h, json!({"planes": [full_plane()], "newLayer": true}));
    // The second dab lies left of the canvas and samples the canvas 100 px to its right.
    set(&mut h, json!({"clone": [{"source": [140, 80], "points": [[40, 80], [-30, 80]], "size": 16}]}));
    let src = source(&h);
    let canvas = src.preview.inputs.canvas;
    let out = locked(&src.preview.state).as_ref().unwrap().out.content_bounds();
    assert!(!out.is_empty() && canvas.contains_rect(&out), "{out:?}");
    assert_eq!(src.stamp_preview().unwrap().size(), PixelSize::new(240, 160));
}

#[test]
fn deleting_the_last_plane_drops_the_stamp_preview() {
    let mut h = open(app_with_layer());
    set(&mut h, json!({"planes": [floor()]}));
    set(&mut h, json!({"clone": [{"source": [120, 139], "points": [[118, 100], [126, 100]], "size": 12}]}));
    assert!(source(&h).stamp_preview().is_some());
    set(&mut h, json!({"planes": []}));
    assert!(source(&h).stamp_preview().is_none(), "no strokes shown without a plane");
}

#[test]
fn ok_paints_the_layer_the_workspace_was_opened_on() {
    for new_layer in [false, true] {
        let mut app = app_with_layer();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("stripe", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(50, 70, 70, 90), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        let mut h = open(app);
        set(&mut h, json!({"planes": [full_plane()], "newLayer": new_layer}));
        set(&mut h, json!({"clone": [{"source": [60, 80], "points": [[120, 80], [126, 80]], "size": 12}]}));
        // Another layer is picked while the workspace is open.
        h.state_mut().session.execute("layer.new.layer", json!({"name": "other"})).unwrap();
        let other = h.state().session.active().unwrap().active_layer.unwrap();
        h.run_steps(2);
        let at = [(120, 80), (124, 80), (60, 80), (200, 30)];
        let preview: Vec<[u8; 4]> = at.iter().map(|&(x, y)| shown(&h, x, y)).collect();
        assert!(preview[0][0] > 230, "the stroke shows in the preview");
        ok(&mut h);
        let app = h.state();
        let d = &app.session.active().unwrap().doc;
        assert!(d.layer(other).unwrap().surface().unwrap().content_bounds().is_empty(), "newLayer={new_layer}");
        if !new_layer {
            assert!(d.layer(layer).unwrap().surface().unwrap().rgba(120, 80)[0] > 0.9, "the stroke lands on the workspace's layer");
            // And it becomes active, so Fade and a colour-channel target follow it.
            assert_eq!(app.session.active().unwrap().active_layer, Some(layer));
        }
        let done = committed(app, layer, new_layer);
        for (&(x, y), p) in at.iter().zip(&preview) {
            assert!(close(*p, done.rgba(x, y)), "({x}, {y}) newLayer={new_layer}");
        }
    }
}
