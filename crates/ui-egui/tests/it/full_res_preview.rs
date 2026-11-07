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

// #11560 R3: a finite inline full-app U8 commit/cancel/history route selection.
// This does not qualify native worker lifetime, device input, or the remaining route matrix.
#[cfg(test)]
mod issue_11560_commit_routes {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use photocraft_engine::{DocState, Session};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const WIDTH: u32 = 64;
    const HEIGHT: u32 = 48;

    fn parameters(command: &str, amount: u32) -> Value {
        match command {
            "filter.sharpen.unsharpMask" => json!({"amount":amount,"radius":2,"threshold":0}),
            "filter.sharpen.smartSharpen" => json!({"amount":amount,"radius":2,"reduceNoise":0}),
            _ => panic!("unexpected R3 sharpening command"),
        }
    }

    fn menu(h: &mut Harness, command: &str) -> Value {
        assert!(photocraft_ui_egui::menus::is_enabled(h.state(), command), "production menu item must be enabled: {command}");
        let ctx = h.ctx.clone();
        photocraft_ui_egui::menus::invoke(h.state_mut(), &ctx, command, json!({})).expect("production menu invocation")
    }

    fn fixture() -> Harness {
        let mut h = harness().expect("R3 requires the real full-eframe offscreen renderer; no skipped qualification");
        h.set_render_cursor(false);
        let app = h.state_mut();
        app.background_jobs = false; // Bounded deterministic inline path, not native job acceptance.
        app.session.color.settings.monitor_profile = "srgb".into();
        app.session.color.settings.working_rgb = "srgb".into();
        app.run("file.new", json!({"width":WIDTH,"height":HEIGHT,"depth":8})).unwrap();
        app.run("edit.assignProfile", json!({"profile":"srgb"})).unwrap();
        app.session
            .edit("R3 opaque RGB edge fixture", |doc, active| {
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
        app.ui.extras.rulers = false;
        app.sync_views();
        h.run_steps(6);
        // Place the complete image in the canvas's upper-left, clear of the centred modal.
        let xf = photocraft_ui_egui::canvas::ViewXform::active(h.state()).unwrap();
        let offset = (xf.rect.min + egui::vec2(96.0, 96.0) - xf.rect.center()) / 2.0;
        control(&mut h, "ui.set", json!({"zoom":2.0,"center":[32.0-offset.x,24.0-offset.y]}));
        h.hover_at(egui::pos2(8.0, 8.0));
        h.run_steps(6);
        h
    }

    fn history(st: &DocState) -> Value {
        // Preserve every exposed row, including redo identity and its actual document pixels;
        // a matching past_len alone would miss altered/replaced history or a lost redo branch.
        let rows: Vec<_> = st
            .history
            .row_ids()
            .enumerate()
            .map(|(i, id)| {
                let doc = st.history.row_doc(i, &st.doc).unwrap();
                let pixels = photocraft_compose::flatten(&doc);
                json!({"id":id,"pixels":&pixels.px})
            })
            .collect();
        json!({"entries":st.history.entries(),"redo":st.history.redo_labels().collect::<Vec<_>>(),
            "past":st.history.past_len(),"rows":rows,"undo_label":st.history.undo_label(),
            "redo_label":st.history.redo_label(),"can_undo":st.history.can_undo(),"can_redo":st.history.can_redo(),
            "brush_source":st.history.brush_source(),"visibility":st.history.current_visibility()})
    }

    fn write_json(root: &Path, name: &str, value: &Value) {
        std::fs::write(root.join(format!("{name}.json")), serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn record_state(h: &Harness, root: &Path, phase: &str) {
        let st = h.state().session.active().unwrap();
        write_json(
            root,
            &format!("{phase}-source-history"),
            &json!({"revision":st.revision,"history":history(st),
            "active_layer":st.active_layer,"selected_layers":st.selected_layers,"doc_id":st.doc.id}),
        );
    }

    fn assert_state(h: &Harness, expected: &DocState, phase: &str) {
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc, expected.doc, "{phase}: exact independently command-dispatched document");
        assert_eq!(photocraft_compose::flatten(&st.doc).px, photocraft_compose::flatten(&expected.doc).px, "{phase}: exact composite pixels");
        assert_eq!(st.revision, expected.revision, "{phase}: revision");
        assert_eq!(history(st), history(expected), "{phase}: full exposed history including redo rows/pixels");
        assert_eq!((st.active_layer, &st.selected_layers), (expected.active_layer, &expected.selected_layers));
        assert!(!h.state().ui.status_error, "{phase}: {}", h.state().ui.status);
        assert!(h.state().session.active_job().is_none(), "inline route must have completed, not queued a job");
    }

    fn oracle(h: &Harness) -> Session {
        let mut engine = Session::new();
        engine.color.settings = h.state().session.color.settings.clone();
        let st = h.state().session.active().unwrap();
        engine.add_document((*st.doc).clone(), None);
        // Clone actual editing state, not a replacement Session/model or preview cache. The
        // independent real engine command starts with exactly the same source and history IDs.
        *engine.active_mut().unwrap() = st.clone();
        engine
    }

    fn display_texture(engine: &Session) -> Vec<[u8; 4]> {
        let doc = &engine.active().unwrap().doc;
        let composite = photocraft_compose::flatten(doc);
        assert!(composite.px.iter().all(|p| p[3] == 1.0));
        let display = engine.color.canvas_display_for(doc, engine.color.main_display).unwrap();
        display.to_rgba8(&composite).pixels.as_chunks::<4>().0.to_vec()
    }

    fn image_meshes(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Mesh(mesh) => {
                let rect = mesh.vertices.iter().fold(egui::Rect::NOTHING, |r, v| r.union(egui::Rect::from_min_max(v.pos, v.pos)));
                if (rect.width() - WIDTH as f32).abs() < 0.01 && (rect.height() - HEIGHT as f32).abs() < 0.01 && clip.contains_rect(rect) {
                    assert_eq!((mesh.vertices.len(), mesh.indices.len()), (4, 6));
                    for v in &mesh.vertices {
                        assert_eq!(v.color, egui::Color32::WHITE);
                        let uv = egui::pos2((v.pos.x - rect.left()) / rect.width(), (v.pos.y - rect.top()) / rect.height());
                        assert!(v.uv.distance(uv) < 1e-6, "mini's default full-image UVs");
                    }
                    out.push(rect);
                }
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| image_meshes(s, clip, out)),
            _ => {}
        }
    }

    fn capture(h: &mut Harness, root: &Path, phase: &str, mini: Option<&[[u8; 4]]>) -> Vec<[u8; 4]> {
        let xf = photocraft_ui_egui::canvas::ViewXform::active(h.state()).unwrap();
        assert_eq!((xf.zoom, xf.rotation, xf.aspect, xf.flip), (2.0, 0.0, 1.0, false));
        assert_eq!(h.ctx.pixels_per_point(), 1.0);
        let canvas = xf.image_bounds([WIDTH, HEIGHT]);
        assert!(xf.rect.contains_rect(canvas), "complete fixture is in the canvas viewport");
        // Read the production dialog paint/hit-guard rectangles through public Context data.
        let dialogs: Vec<egui::Rect> = h.ctx.data(|m| m.get_temp(egui::Id::unique("pc-dialog-rects"))).unwrap_or_default();
        assert_eq!(dialogs.len(), h.state().ui.dialogs.len(), "production dialog-rectangle observation is complete");
        assert!(dialogs.iter().all(|r| !r.intersects(canvas.expand(8.0))), "sampled canvas must not be under a dialog");
        let image = h.render().expect("actual full-eframe screenshot");
        image.save(root.join(format!("{phase}.png"))).unwrap();
        assert!(h.state().perf.gpu, "R3 canvas observation requires the production GPU route");
        let sample_rect = canvas.shrink(4.0);
        let mut samples = Vec::new();
        for y in sample_rect.top().ceil() as u32..sample_rect.bottom().floor() as u32 {
            for x in sample_rect.left().ceil() as u32..sample_rect.right().floor() as u32 {
                samples.push(image.get_pixel(x, y).0);
            }
        }
        std::fs::write(root.join(format!("{phase}-canvas.rgba")), samples.as_flattened()).unwrap();
        write_json(
            root,
            &format!("{phase}-geometry"),
            &json!({"canvas":[canvas.left(),canvas.top(),canvas.right(),canvas.bottom()],
            "samples":samples.len(),"pixels_per_point":1,"zoom":2,"gpu":h.state().perf.gpu}),
        );
        assert!(!samples.is_empty());
        if let Some(texture) = mini {
            let checkbox = h.get_by_label("Preview").rect();
            let mut rects = Vec::new();
            for s in &h.output().shapes {
                image_meshes(&s.shape, s.clip_rect, &mut rects);
            }
            rects.retain(|r| r.top() > checkbox.bottom() && r.bottom() < checkbox.bottom() + 180.0);
            assert_eq!(rects.len(), 1, "one actual painted full-image mini below Preview");
            let rect = rects[0];
            let mut actual = Vec::new();
            let mut wanted = Vec::new();
            for y in (rect.top() + 2.0).ceil() as u32..(rect.bottom() - 2.0).floor() as u32 {
                for x in (rect.left() + 2.0).ceil() as u32..(rect.right() - 2.0).floor() as u32 {
                    // Same pinned gamma-space four-tap sampler contract as the retained R1
                    // oracle, derived from physical centres and independently asserted UVs.
                    let sx = x as f32 - rect.left();
                    let sy = y as f32 - rect.top();
                    let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
                    let (fx, fy) = (sx - sx.floor(), sy - sy.floor());
                    let at = |dx: i32, dy: i32, c: usize| {
                        let tx = (ix + dx).clamp(0, WIDTH as i32 - 1) as usize;
                        let ty = (iy + dy).clamp(0, HEIGHT as i32 - 1) as usize;
                        f32::from(texture[ty * WIDTH as usize + tx][c])
                    };
                    let q: [u8; 4] = std::array::from_fn(|c| {
                        let top = at(0, 0, c) * (1.0 - fx) + at(1, 0, c) * fx;
                        let bottom = at(0, 1, c) * (1.0 - fx) + at(1, 1, c) * fx;
                        (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
                    });
                    actual.push(image.get_pixel(x, y).0);
                    wanted.push(q);
                }
            }
            std::fs::write(root.join(format!("{phase}-mini-actual.rgba")), actual.as_flattened()).unwrap();
            std::fs::write(root.join(format!("{phase}-mini-expected.rgba")), wanted.as_flattened()).unwrap();
            assert!(!actual.is_empty());
            assert!(
                actual.iter().zip(&wanted).all(|(p, q)| p.iter().zip(q).all(|(a, b)| a.abs_diff(*b) <= 1)),
                "{phase}: independently command-dispatched mini pixels"
            );
        }
        samples
    }

    fn open_unchecked(h: &mut Harness, command: &str, amount: u32) -> u64 {
        let result = menu(h, command);
        let id = result["dialog"].as_u64().expect("empty-params production menu must open the schema dialog");
        assert_eq!(h.state().ui.dialogs.len(), 1);
        let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        assert_eq!(fields["__command"], json!(command));
        assert_eq!(fields["__filter"], json!(true));
        // Off before the first frame, not after warming a checked preview.
        fields.insert("__preview".into(), json!(false));
        for (key, value) in parameters(command, amount).as_object().unwrap() {
            fields.insert(key.clone(), value.clone());
        }
        id
    }

    fn route(command: &str) {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut h = fixture();
        let root: PathBuf =
            std::env::var_os("PHOTOCRAFT_11560_ARTIFACT_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("photocraft-11560-r3"));
        let root = root.join(format!("r3-{}-{command}-u8", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        eprintln!("#11560 R3 captures: {}", root.display());
        let original = h.state().session.active().unwrap().clone();
        record_state(&h, &root, "before-first-preview");
        let canvas_original = capture(&mut h, &root, "before-first-preview", None);
        let mut expected = oracle(&h);
        expected.execute(command, parameters(command, 180)).unwrap();
        let filtered = display_texture(&expected);
        assert_ne!(
            photocraft_compose::flatten(&original.doc).px,
            photocraft_compose::flatten(&expected.active().unwrap().doc).px,
            "nonidentity command oracle"
        );
        let id = open_unchecked(&mut h, command, 180);
        assert_state(&h, &original, "opened-before-first-preview");
        record_state(&h, &root, "opened-before-first-preview");
        h.run_steps(12);
        assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"], json!(false));
        assert_eq!(
            capture(&mut h, &root, "unchecked-before-confirm", Some(&filtered)),
            canvas_original,
            "unchecked canvas stays original while mini is filtered"
        );
        assert_state(&h, &original, "unchecked-before-confirm");
        // Positive control: this exact visible crop must detect the real checked canvas,
        // otherwise a background/occluded/static crop could give a vacuous equality.
        h.get_by_label("Preview").click();
        h.run_steps(6);
        let canvas_filtered = capture(&mut h, &root, "checked-canvas-control", Some(&filtered));
        assert!(
            canvas_filtered.iter().zip(&canvas_original).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) >= 8)).count() >= 16,
            "the actual canvas crop distinguishes checked filtered pixels from original"
        );
        assert_state(&h, &original, "checked-canvas-control");
        h.get_by_label("Preview").click();
        h.run_steps(6);
        assert_eq!(capture(&mut h, &root, "unchecked-confirm-ready", Some(&filtered)), canvas_original);
        assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__preview"], json!(false));
        record_state(&h, &root, "unchecked-confirm-ready");
        assert_state(&h, &original, "unchecked-confirm-ready");
        h.get_by_label("OK").click();
        h.run_steps(6);
        assert!(h.state().ui.dialogs.is_empty(), "actual OK must close the filter dialog without error dialogs");
        record_state(&h, &root, "confirmed");
        assert_state(&h, expected.active().unwrap(), "confirmed");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), original.history.past_len() + 1, "Confirm is exactly one undoable step");
        let committed_history = history(h.state().session.active().unwrap());
        capture(&mut h, &root, "confirmed", None);

        menu(&mut h, "edit.undo");
        h.run_steps(6);
        expected.execute("edit.undo", json!({})).unwrap();
        record_state(&h, &root, "menu-undo");
        assert_state(&h, expected.active().unwrap(), "menu-undo");
        assert!(Arc::ptr_eq(&original.doc, &h.state().session.active().unwrap().doc), "Undo restores the original source allocation");
        assert_eq!(capture(&mut h, &root, "menu-undo", None), canvas_original);
        assert!(h.state().session.active().unwrap().history.can_redo());
        // Cancel on the restored original while a redo branch exists: it must preserve both.
        let undone = h.state().session.active().unwrap().clone();
        let mut cancel_oracle = oracle(&h);
        cancel_oracle.execute(command, parameters(command, 60)).unwrap();
        let cancel_filtered = display_texture(&cancel_oracle);
        open_unchecked(&mut h, command, 60);
        assert_state(&h, &undone, "cancel-opened-before-preview");
        record_state(&h, &root, "cancel-opened-before-preview");
        h.run_steps(12);
        assert_eq!(capture(&mut h, &root, "unchecked-before-cancel", Some(&cancel_filtered)), canvas_original);
        assert_state(&h, &undone, "unchecked-before-cancel");
        record_state(&h, &root, "unchecked-before-cancel");
        h.get_by_label("Cancel").click();
        h.run_steps(6);
        assert!(h.state().ui.dialogs.is_empty());
        record_state(&h, &root, "cancelled");
        assert_state(&h, &undone, "cancelled");
        assert!(Arc::ptr_eq(&undone.doc, &h.state().session.active().unwrap().doc), "Cancel must retain original source identity");
        assert_eq!(capture(&mut h, &root, "cancelled", None), canvas_original);

        menu(&mut h, "edit.redo");
        h.run_steps(6);
        expected.execute("edit.redo", json!({})).unwrap();
        record_state(&h, &root, "menu-redo-after-cancel");
        assert_state(&h, expected.active().unwrap(), "menu-redo-after-cancel");
        assert_eq!(history(h.state().session.active().unwrap()), committed_history, "Redo restores exact committed history rows and pixels");
        capture(&mut h, &root, "menu-redo-after-cancel", None);
    }

    #[test]
    fn unsharp_u8_unchecked_mini_confirm_cancel_menu_undo_redo() {
        route("filter.sharpen.unsharpMask");
    }

    #[test]
    fn smart_sharpen_u8_unchecked_mini_confirm_cancel_menu_undo_redo() {
        route("filter.sharpen.smartSharpen");
    }
}
