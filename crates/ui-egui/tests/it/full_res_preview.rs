//! Preferences › Performance › Low Resolution Previews: on (the default) live previews of a large
//! document render on a reduced copy, off they render at full resolution.
//!
//! A 2502 × 2000 document of 1 px black and white columns: the reduced copy (every 2nd pixel at
//! this size) is all black, so its preview shows a flat colour where the full-size one shows the
//! columns at 100 % (and their mipmapped grey zoomed out). Drives the real app offscreen on wgpu;
//! skips when no GPU adapter exists (like `drag_preview_canvas.rs`).

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use photocraft_geom::Rect;
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

// Just above the 5 MP proxy threshold; keep the same factor-two reduction.
const W: i32 = 2502;
const H: i32 = 2000;

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app
        })
    });
    match built {
        Ok(h) => Some(h),
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            None
        }
    }
}

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

fn columns_document() -> Document {
    let mut doc = Document::with_background("columns", Size::new(W as u32, H as u32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    assert_eq!(photocraft_ui_egui::proxy::factor(&doc), 2, "the fixture must exercise a reduced preview");
    let s = doc.layers[0].surface_mut().expect("background pixels");
    let row: Vec<f32> = (0..W)
        .flat_map(|x| {
            let v = if x % 2 == 0 { 0.0 } else { 1.0 };
            [v, v, v, 1.0]
        })
        .collect();
    for y in (0..H).step_by(64) {
        let end = (y + 64).min(H);
        s.write_region(Rect::new(0, y, W, end), &row.repeat((end - y) as usize));
    }
    doc
}

fn setup(h: &mut Harness, zoom: f32) {
    {
        let app = h.state_mut();
        app.session.add_document(columns_document(), None);
        app.sync_views();
        app.ui.extras.rulers = false;
    }
    control(h, "ui.set", json!({"zoom": zoom, "center": [W / 2, H / 2]}));
    h.run_steps(4);
}

/// Whether the canvas draws on the GPU (the previews under test are the GPU canvas's).
fn on_gpu(h: &mut Harness) -> bool {
    let _ = h.render();
    let gpu = h.state().perf.gpu;
    if !gpu {
        eprintln!("skipping: the canvas isn't on the GPU");
    }
    gpu
}

fn low_res(h: &mut Harness, on: bool) {
    h.state_mut().run("prefs.set", json!({"path": "performance.lowResolutionPreviews", "value": on})).expect("prefs.set");
    h.run_steps(4);
}

/// Luma along a 300 px row near the canvas's top-left corner (clear of centred dialogs).
fn row(h: &mut Harness) -> Vec<i32> {
    h.run_steps(4);
    let canvas = {
        let app = h.state();
        photocraft_ui_egui::rulers::content_rect(app, app.last_canvas_rect)
    };
    // egui_kittest waits 10 s for the GPU. Windows CI renders on WARP, a software rasterizer,
    // where the first full-resolution composite of this 5 MP document can take longer; the
    // submitted work still finishes, so wait for it again instead of failing.
    let img = (0..3).find_map(|_| h.render().ok()).expect("render");
    let (x0, y) = (canvas.min.x as u32 + 20, canvas.min.y as u32 + 40);
    (x0..x0 + 300)
        .map(|x| {
            let p = img.get_pixel(x, y).0;
            (i32::from(p[0]) * 3 + i32::from(p[1]) * 6 + i32::from(p[2])) / 10
        })
        .collect()
}

/// Fraction of neighbouring screen pixels that differ like black and white.
fn columns(r: &[i32]) -> f32 {
    r.windows(2).filter(|w| (w[0] - w[1]).abs() > 100).count() as f32 / (r.len() - 1) as f32
}

fn mean(r: &[i32]) -> f32 {
    r.iter().sum::<i32>() as f32 / r.len() as f32
}

/// Dragging an adjustment layer's control (Properties panel) previews through the reduced copy
/// only as coarse as the screen (#5764): at 100 % every column shows even with low-resolution
/// previews on; zoomed out to 50 % the default drag runs on the reduced copy (only the black columns, inverted), and with
/// them off it matches the committed view (the columns' mipmapped grey).
#[test]
fn adjustment_layer_drag_previews_at_full_resolution_when_low_res_is_off() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    setup(&mut h, 1.0);
    if !on_gpu(&mut h) {
        return;
    }
    assert!(columns(&row(&mut h)) > 0.9, "the canvas shows the 1 px columns at 100 %");
    let layer = {
        let app = h.state_mut();
        app.run("layer.newAdjustmentLayer.invert", json!({})).expect("adjustment layer");
        let id = app.session.active().and_then(|d| d.active_layer).expect("active layer");
        // What the Properties panel sets while a slider drags.
        app.live_adjust = Some((id, json!({})));
        id
    };
    let at_100 = row(&mut h);
    assert!(columns(&at_100) > 0.9, "default at 100 %: the drag is never coarser than the screen ({})", columns(&at_100));
    control(&mut h, "ui.set", json!({"zoom": 0.5, "center": [W / 2, H / 2]}));
    h.state_mut().live_adjust = None;
    let committed = mean(&row(&mut h));
    assert!(committed > 60.0, "zoomed out the inverted columns average to grey ({committed})");
    h.state_mut().live_adjust = Some((layer, json!({})));
    let reduced = mean(&row(&mut h));
    assert!(reduced > 225.0, "default at 50 %: the reduced copy keeps only the (inverted) black columns ({reduced})");
    low_res(&mut h, false);
    h.state_mut().live_adjust = Some((layer, json!({})));
    let full = mean(&row(&mut h));
    assert!((full - committed).abs() < 8.0, "low-res previews off: the drag matches the committed view ({full} vs {committed})");
}

/// A filter dialog's live preview is never coarser than the screen either (#6326): at 100 % every
/// column shows with low-resolution previews on; zoomed out to 50 % the default preview runs on
/// the reduced copy (only the black columns), and with them off it matches the committed view.
#[test]
fn filter_dialog_previews_at_full_resolution_when_low_res_is_off() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    setup(&mut h, 0.5);
    if !on_gpu(&mut h) {
        return;
    }
    let committed = mean(&row(&mut h));
    assert!(committed > 60.0, "zoomed out the columns average to grey ({committed})");
    control(&mut h, "ui.set", json!({"zoom": 1.0, "center": [W / 2, H / 2]}));
    // Offset 0, 0 leaves the pixels as they are: any change is the preview's resolution.
    photocraft_ui_egui::filter_dialog::open(h.state_mut(), "filter.other.offset").expect("Offset dialog");
    let at_100 = row(&mut h);
    assert!(columns(&at_100) > 0.9, "default at 100 %: the preview is never coarser than the screen ({})", columns(&at_100));
    control(&mut h, "ui.set", json!({"zoom": 0.5, "center": [W / 2, H / 2]}));
    let reduced = mean(&row(&mut h));
    assert!(reduced < 30.0, "default at 50 %: the reduced copy keeps only the black columns ({reduced})");
    low_res(&mut h, false);
    let full = mean(&row(&mut h));
    assert!((full - committed).abs() < 8.0, "low-res previews off: the preview matches the committed view ({full} vs {committed})");
}

#[test]
fn adjustment_dialog_zoomed_out_skips_the_reduced_copy_when_low_res_is_off() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    setup(&mut h, 0.5);
    if !on_gpu(&mut h) {
        return;
    }
    let committed = mean(&row(&mut h));
    assert!(committed > 60.0, "zoomed out the columns average to grey ({committed})");
    photocraft_ui_egui::adjust_dialog::open(h.state_mut(), "image.adjustments.levels").expect("Levels dialog");
    h.run_steps(4);
    assert!(photocraft_ui_egui::adjust_preview::gpu_proxy(h.state_mut(), 0, 0.5).is_some(), "default: zoomed out on the reduced copy");
    let reduced = mean(&row(&mut h));
    assert!(reduced < 30.0, "the reduced copy keeps only the black columns ({reduced})");
    low_res(&mut h, false);
    assert!(photocraft_ui_egui::adjust_preview::gpu_proxy(h.state_mut(), 0, 0.5).is_none(), "off: no reduced copy");
    let full = mean(&row(&mut h));
    assert!((full - committed).abs() < 8.0, "off: the preview matches the committed view ({full} vs {committed})");
}

// #11560 uses actual production dialog input and offscreen mini pixels. Unlike the older
// resolution characterizations above, absence of a renderer is a failure, never a skipped pass.
mod issue_11560 {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn setup_sharpen(command: &str) -> (Harness, u64) {
        let mut h = harness().expect("#11560 requires an offscreen renderer; unavailable is not qualified");
        {
            let app = h.state_mut();
            app.background_jobs = false;
            app.run("file.new", json!({"width":64,"height":48})).unwrap();
            app.session
                .edit("Midtone edge", |doc, active| {
                    let s = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                    for x in 0..64 {
                        let v = 0.25 + x as f32 / 256.0 + if x >= 32 { 0.15 } else { 0.0 } + if x % 3 == 0 { 0.04 } else { 0.0 };
                        s.fill_rect(Rect::new(x, 0, x + 1, 48), &[v, v, v, 1.0]);
                    }
                    Ok(())
                })
                .unwrap();
            app.sync_views();
        }
        let id = photocraft_ui_egui::filter_dialog::open(h.state_mut(), command).unwrap();
        let f = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        f.insert("amount".into(), json!(180));
        f.insert("radius".into(), json!(2));
        let detail = match command {
            "filter.sharpen.unsharpMask" => "threshold",
            "filter.sharpen.smartSharpen" => "reduceNoise",
            _ => panic!("unexpected sharpening fixture command"),
        };
        f.insert(detail.into(), json!(0));
        h.run_steps(5);
        (h, id)
    }

    fn collect(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Mesh(m) => {
                let rect = m.vertices.iter().fold(egui::Rect::NOTHING, |r, v| r.union(egui::Rect::from_min_max(v.pos, v.pos)));
                if (rect.width() - 64.0).abs() < 0.1 && (rect.height() - 48.0).abs() < 0.1 && clip.contains_rect(rect) {
                    out.push(rect);
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, clip, out)),
            _ => {}
        }
    }

    fn mini(h: &Harness) -> egui::Rect {
        let checkbox = h.get_by_label("Preview").rect();
        let mut images = Vec::new();
        for s in &h.output().shapes {
            collect(&s.shape, s.clip_rect, &mut images);
        }
        let candidates: Vec<_> = images.into_iter().filter(|r| r.center().y > checkbox.bottom() && r.center().y < checkbox.bottom() + 180.0).collect();
        assert_eq!(candidates.len(), 1, "one visible 100% mini image below the actual Preview checkbox, not a canvas or icon");
        candidates[0]
    }

    fn pixels(h: &mut Harness, rect: egui::Rect) -> Vec<[u8; 4]> {
        let image = h.render().expect("actual offscreen renderer");
        // Avoid the frame/checker border and take the complete interior sharpening row.
        // Stay away from the pointer at the image centre during the comparison gesture.
        let y = (rect.top() + 5.0).floor() as u32;
        ((rect.left() + 2.0).ceil() as u32..(rect.right() - 2.0).floor() as u32).map(|x| image.get_pixel(x, y).0).collect()
    }

    fn button(h: &mut Harness, at: egui::Pos2, pressed: bool) {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(2);
    }

    fn checkbox_hold_release(command: &str) {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut h, id) = setup_sharpen(command);
        let before = h.state().session.active().unwrap().doc.clone();
        let revision = h.state().session.active().unwrap().revision;
        let history = h.state().session.active().unwrap().history.past_len();
        let rect = mini(&h);
        let filtered = pixels(&mut h, rect);
        let mut engine = photocraft_engine::Session::new();
        engine.color.settings = h.state().session.color.settings.clone();
        engine.add_document(before.as_ref().clone(), None);
        if let Some(active) = h.state().session.active().unwrap().active_layer {
            engine.select_layer(active).unwrap();
        }
        let params = match command {
            "filter.sharpen.unsharpMask" => json!({"amount":180,"radius":2,"threshold":0}),
            "filter.sharpen.smartSharpen" => json!({"amount":180,"radius":2,"reduceNoise":0}),
            _ => panic!("unexpected sharpening fixture command"),
        };
        engine.execute(command, params).unwrap();
        let expected = engine.active().unwrap().doc.clone();
        assert_ne!(photocraft_compose::flatten(&expected).px, photocraft_compose::flatten(&before).px, "nonzero real sharpening oracle");
        let at = rect.center();
        let mut original = None;
        for on in [true, false, true] {
            if h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"] != json!(on) {
                h.get_by_label("Preview").click();
                h.run_steps(5);
            }
            assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"], json!(on), "actual checkbox input reached production");
            let rect = mini(&h);
            assert_eq!(pixels(&mut h, rect), filtered, "{command}: unchecked canvas must not turn the mini original");
            h.hover_at(at);
            h.run_steps(1);
            button(&mut h, at, true);
            let held = pixels(&mut h, rect);
            assert_ne!(held, filtered, "actual mini press shows visibly different original pixels");
            if let Some(ref first) = original {
                assert_eq!(&held, first);
            } else {
                original = Some(held.clone());
            }
            h.run_steps(3);
            assert_eq!(pixels(&mut h, rect), held, "held comparison survives idle frames");
            button(&mut h, at, false);
            assert_eq!(pixels(&mut h, rect), filtered, "real release restores filtered pixels with either checkbox state");
            assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"], json!(on));
        }
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        photocraft_ui_egui::dialogs::cancel(h.state_mut(), id).unwrap();
        h.run_steps(3);
        assert!(std::sync::Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
    }

    #[test]
    fn unsharp_checkbox_and_mini_hold_release_render_actual_pixels() {
        checkbox_hold_release("filter.sharpen.unsharpMask");
    }

    #[test]
    fn smart_sharpen_checkbox_and_mini_hold_release_render_actual_pixels() {
        checkbox_hold_release("filter.sharpen.smartSharpen");
    }
}

// Supplemental #11560 R1: absolute engine -> monitor texture -> offscreen mini pixels.
// Keep this separate from the frozen relational, target-policy and worker-lifecycle modules.
mod issue_11560_absolute {
    use super::*;
    use egui_kittest::kittest::Queryable;

    const WIDTH: usize = 64;
    const HEIGHT: usize = 48;
    // Only final framebuffer quantization/float interpolation may differ by one code. This
    // is not a tolerance on the engine result, a percentage match, or an adaptive threshold.
    const SCREEN_TOLERANCE: u8 = 1;

    fn parameters(command: &str, amount: u32) -> Value {
        match command {
            "filter.sharpen.unsharpMask" => json!({"amount":amount,"radius":2,"threshold":0}),
            "filter.sharpen.smartSharpen" => json!({"amount":amount,"radius":2,"reduceNoise":0}),
            _ => panic!("unexpected absolute-oracle command"),
        }
    }

    fn fixture(command: &str, depth: u32) -> (Harness, u64) {
        let mut h = harness().expect("R1 needs the actual offscreen wgpu renderer; absence is not a pass");
        // Disable only kittest's synthetic cursor paint, never production pointer events.
        h.set_render_cursor(false);
        let app = h.state_mut();
        app.background_jobs = false; // Inline route only; no claim about native worker lifetime.
        app.session.color.settings.monitor_profile = "srgb".into();
        app.session.color.settings.working_rgb = "srgb".into();
        app.run("file.new", json!({"width":WIDTH,"height":HEIGHT,"depth":depth})).unwrap();
        app.run("edit.assignProfile", json!({"profile":"srgb"})).unwrap();
        app.session
            .edit("R1 opaque two-dimensional RGB fixture", |doc, active| {
                let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for y in 0..HEIGHT as i32 {
                    for x in 0..WIDTH as i32 {
                        let r = 0.20 + x as f32 / 256.0 + if x >= 32 { 0.13 } else { 0.0 };
                        let g = 0.24 + y as f32 / 192.0 + if y % 7 < 3 { 0.09 } else { 0.0 };
                        let b = 0.62 - x as f32 / 384.0 + if (x + 2 * y) % 9 < 4 { 0.08 } else { 0.0 };
                        surface.fill_rect(Rect::new(x, y, x + 1, y + 1), &[r, g, b, 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
        app.sync_views();
        let id = photocraft_ui_egui::filter_dialog::open(app, command).unwrap();
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        // Off before the first producer pass: no warmed filtered frame can satisfy this cell.
        fields.insert("__preview".into(), json!(false));
        for (key, value) in parameters(command, 180).as_object().unwrap() {
            fields.insert(key.clone(), value.clone());
        }
        h.run_steps(12);
        (h, id)
    }

    fn expected_texture(h: &Harness, source: &Document, command: &str, amount: Option<u32>) -> Vec<[u8; 4]> {
        // Independent real command dispatch on the original source, never filter_preview.result
        // or a cached painted frame. The engine CPU compositor is the project pixel oracle.
        let mut engine = photocraft_engine::Session::new();
        engine.color.settings = h.state().session.color.settings.clone();
        engine.add_document(source.clone(), None);
        engine.select_layer(h.state().session.active().unwrap().active_layer.unwrap()).unwrap();
        if let Some(amount) = amount {
            engine.execute(command, parameters(command, amount)).unwrap();
        }
        let doc = &engine.active().unwrap().doc;
        let composite = photocraft_compose::flatten(doc);
        assert_eq!(composite.px.len(), WIDTH * HEIGHT);
        assert!(composite.px.iter().all(|p| p[3] == 1.0), "opaque fixture excludes checker/alpha blending ambiguity");
        // The public engine display contract includes linear encoding and ICC conversion.
        // Do not assume that document floats are screenshot bytes (especially at F32).
        let display = engine.color.canvas_display_for(doc, engine.color.main_display).unwrap();
        let rgba = display.to_rgba8(&composite);
        assert_eq!((rgba.width, rgba.height), (WIDTH as u32, HEIGHT as u32));
        rgba.pixels.as_chunks::<4>().0.to_vec()
    }

    fn collect_meshes(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<egui::epaint::Mesh>) {
        match shape {
            egui::Shape::Mesh(mesh) => {
                let bounds = mesh.vertices.iter().fold(egui::Rect::NOTHING, |r, v| r.union(egui::Rect::from_min_max(v.pos, v.pos)));
                if (bounds.width() - WIDTH as f32).abs() < 0.01 && (bounds.height() - HEIGHT as f32).abs() < 0.01 && clip.contains_rect(bounds) {
                    out.push(mesh.as_ref().clone());
                }
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_meshes(shape, clip, out)),
            _ => {}
        }
    }

    fn mini_geometry(h: &Harness) -> egui::Rect {
        assert_eq!(h.ctx.pixels_per_point(), 1.0, "explicit physical screenshot scale");
        let checkbox = h.get_by_label("Preview").rect();
        let mut meshes = Vec::new();
        for shape in &h.output().shapes {
            collect_meshes(&shape.shape, shape.clip_rect, &mut meshes);
        }
        meshes.retain(|m| m.vertices.iter().all(|v| v.pos.y > checkbox.bottom() && v.pos.y < checkbox.bottom() + 180.0));
        assert_eq!(meshes.len(), 1, "unique fully clipped-in mini, not a canvas/icon/checkbox");
        let mesh = &meshes[0];
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), (4, 6));
        let rect = mesh.vertices.iter().fold(egui::Rect::NOTHING, |r, v| r.union(egui::Rect::from_min_max(v.pos, v.pos)));
        // Assert the default geometry independently instead of deriving an oracle from an
        // arbitrary (possibly wrong) production UV mapping. Fractional placement is allowed.
        for vertex in &mesh.vertices {
            assert_eq!(vertex.color, egui::Color32::WHITE, "no tint or transient window opacity");
            let expected_uv = egui::vec2((vertex.pos.x - rect.left()) / rect.width(), (vertex.pos.y - rect.top()) / rect.height());
            assert!((vertex.uv.x - expected_uv.x).abs() < 1e-6 && (vertex.uv.y - expected_uv.y).abs() < 1e-6, "100% full-image UV contract");
        }
        rect
    }

    fn sample(texture: &[[u8; 4]], rect: egui::Rect, x: u32, y: u32) -> [u8; 4] {
        // PREDICTABLE kittest uses gamma-space Rgba8Unorm four-tap bilinear sampling,
        // no dither/MSAA. UVs map texel centres, not integer screen corners:
        // texel = ((physical_pixel + .5) / pixels_per_point - dest.min) - .5.
        let sx = x as f32 + 0.5 - rect.left() - 0.5;
        let sy = y as f32 + 0.5 - rect.top() - 0.5;
        let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
        let (fx, fy) = (sx - sx.floor(), sy - sy.floor());
        let at = |dx: i32, dy: i32, c: usize| {
            let tx = (ix + dx).clamp(0, WIDTH as i32 - 1) as usize;
            let ty = (iy + dy).clamp(0, HEIGHT as i32 - 1) as usize;
            f32::from(texture[ty * WIDTH + tx][c])
        };
        std::array::from_fn(|c| {
            let top = at(0, 0, c) * (1.0 - fx) + at(1, 0, c) * fx;
            let bottom = at(0, 1, c) * (1.0 - fx) + at(1, 1, c) * fx;
            (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
        })
    }

    fn assert_pixels(h: &mut Harness, expected: &[[u8; 4]], rejected: &[[u8; 4]], root: &std::path::Path, phase: &str) {
        let rect = mini_geometry(h);
        let image = h.render().expect("real app offscreen screenshot, not a cached preview-document probe");
        // Save evidence before the pixel assertions, including a mismatching screenshot.
        std::fs::create_dir_all(root).unwrap();
        image.save(root.join(format!("{phase}.png"))).unwrap();
        std::fs::write(root.join(format!("{phase}-expected-texture.rgba")), expected.as_flattened()).unwrap();
        let (x0, x1) = ((rect.left() + 2.0).ceil() as u32, (rect.right() - 2.0).floor() as u32);
        let (y0, y1) = ((rect.top() + 2.0).ceil() as u32, (rect.bottom() - 2.0).floor() as u32);
        assert!(x1 > x0 && y1 > y0 && x1 <= image.width() && y1 <= image.height());
        let mut actual = Vec::new();
        let mut wanted = Vec::new();
        let mut max_error = 0;
        let mut discriminating = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = image.get_pixel(x, y).0;
                let q = sample(expected, rect, x, y);
                let wrong = sample(rejected, rect, x, y);
                if (0..3).any(|c| q[c].abs_diff(wrong[c]) >= 8) {
                    discriminating += 1;
                }
                for c in 0..4 {
                    max_error = max_error.max(p[c].abs_diff(q[c]));
                }
                actual.push(p);
                wanted.push(q);
            }
        }
        std::fs::write(root.join(format!("{phase}-actual-samples.rgba")), actual.as_flattened()).unwrap();
        std::fs::write(root.join(format!("{phase}-expected-samples.rgba")), wanted.as_flattened()).unwrap();
        let geometry = json!({"phase":phase,"texture":[WIDTH,HEIGHT],"dest":[rect.left(),rect.top(),rect.right(),rect.bottom()],"uv":[0,0,1,1],"pixels_per_point":1,"sample_bounds":[x0,y0,x1,y1],"samples":actual.len(),"max_error":max_error,"tolerance":SCREEN_TOLERANCE,"discriminating_pixels":discriminating});
        std::fs::write(root.join(format!("{phase}.json")), serde_json::to_vec_pretty(&geometry).unwrap()).unwrap();
        assert!(discriminating >= 16, "{phase}: oracle must reject the other original/filtered state by at least eight codes at sixteen pixels");
        assert!(actual.iter().all(|p| p[3] == 255), "{phase}: screenshot must be opaque");
        assert!(
            max_error <= SCREEN_TOLERANCE,
            "{phase}: absolute engine/monitor/bilinear screenshot mismatch: max error {max_error}; captures {}",
            root.display()
        );
    }

    fn absolute_route(command: &str, depth: u32) {
        let _gpu = photocraft_testkit::gpu_lock();
        let (mut h, id) = fixture(command, depth);
        let before = h.state().session.active().unwrap().doc.clone();
        let original_pixels = photocraft_compose::flatten(&before).px.clone();
        let revision = h.state().session.active().unwrap().revision;
        let history = h.state().session.active().unwrap().history.past_len();
        let original = expected_texture(&h, &before, command, None);
        // A caller can collect these without changing CI or the production renderer. Default
        // outputs remain in the test process's temp area, separated by process/command/depth.
        let root =
            std::env::var_os("PHOTOCRAFT_11560_ARTIFACT_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("photocraft-11560-r1"));
        let root = root.join(format!("{}-{command}-{depth}", std::process::id()));
        eprintln!("#11560 R1 captures: {}", root.display());
        let mut previous: Option<(u32, Vec<[u8; 4]>)> = None;
        for (amount, on) in [(180, false), (180, true), (60, false), (60, true)] {
            if h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"] != json!(on) {
                h.get_by_label("Preview").click();
                h.run_steps(3);
            }
            // Change the real dialog's schema fields while unchecked; let the normal app
            // frame/producer/presenter run. No injected FilterPreview or worker result.
            for (key, value) in parameters(command, amount).as_object().unwrap() {
                h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(key.clone(), value.clone());
            }
            h.run_steps(12);
            let filtered = expected_texture(&h, &before, command, Some(amount));
            let phase = format!("amount-{amount}-preview-{on}");
            if let Some((prior_amount, prior)) = &previous
                && *prior_amount != amount
            {
                // This second absolute check must also distinguish a stale prior result,
                // not merely distinguish any sharpened frame from the original.
                assert_pixels(&mut h, &filtered, prior, &root, &format!("{phase}-parameter-refresh"));
            }
            assert_pixels(&mut h, &filtered, &original, &root, &format!("{phase}-filtered"));
            previous = Some((amount, filtered.clone()));
            let at = mini_geometry(&h).center();
            h.hover_at(at);
            h.run_steps(1);
            let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            h.event(button(true));
            h.run_steps(2);
            assert_pixels(&mut h, &original, &filtered, &root, &format!("{phase}-pressed"));
            h.run_steps(3);
            assert_pixels(&mut h, &original, &filtered, &root, &format!("{phase}-held"));
            h.event(button(false));
            h.run_steps(2);
            assert_pixels(&mut h, &filtered, &original, &root, &format!("{phase}-released"));
            let st = h.state().session.active().unwrap();
            assert!(std::sync::Arc::ptr_eq(&before, &st.doc));
            assert_eq!(photocraft_compose::flatten(&st.doc).px, original_pixels);
            assert_eq!(st.revision, revision);
            assert_eq!(st.history.past_len(), history);
            assert_eq!(
                h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"],
                json!(on),
                "real pointer comparison preserves checkbox state"
            );
        }
        photocraft_ui_egui::dialogs::cancel(h.state_mut(), id).unwrap();
        h.run_steps(3);
        let st = h.state().session.active().unwrap();
        assert!(std::sync::Arc::ptr_eq(&before, &st.doc));
        assert_eq!(photocraft_compose::flatten(&st.doc).px, original_pixels);
        assert_eq!((st.revision, st.history.past_len()), (revision, history));
    }

    #[test]
    fn unsharp_u8_absolute_engine_mini_pixels() {
        absolute_route("filter.sharpen.unsharpMask", 8);
    }
    #[test]
    fn unsharp_u16_absolute_engine_mini_pixels() {
        absolute_route("filter.sharpen.unsharpMask", 16);
    }
    #[test]
    fn unsharp_f32_absolute_engine_mini_pixels() {
        absolute_route("filter.sharpen.unsharpMask", 32);
    }
    #[test]
    fn smart_sharpen_u8_absolute_engine_mini_pixels() {
        absolute_route("filter.sharpen.smartSharpen", 8);
    }
    #[test]
    fn smart_sharpen_u16_absolute_engine_mini_pixels() {
        absolute_route("filter.sharpen.smartSharpen", 16);
    }
    #[test]
    fn smart_sharpen_f32_absolute_engine_mini_pixels() {
        absolute_route("filter.sharpen.smartSharpen", 32);
    }
}
