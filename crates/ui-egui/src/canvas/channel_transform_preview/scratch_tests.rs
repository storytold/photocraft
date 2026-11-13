//! Scratch configuration is process-wide; isolate these failure/recovery scenarios from
//! other unit tests, including when libtest runs those tests on several threads.
use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_geom::TileCoord;
use photocraft_raster::spill;
use std::io::{Read, Seek, SeekFrom, Write};

struct Damaged {
    file: std::fs::File,
    original: Vec<u8>,
    dir: std::path::PathBuf,
}

impl Damaged {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("photocraft-channel-scratch-{name}-{}", std::process::id()));
        spill::configure(Some(&dir), 1).unwrap();
        // Pinned canvas/source tiles deliberately exceed this tiny budget. Use bounded
        // maintenance instead of asking settle() to reach an unattainable resident limit.
        spill::maintain_budget();
        let mut file = spill::scratch_handle_for_tests().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut original = Vec::new();
        file.read_to_end(&mut original).unwrap();
        assert!(!original.is_empty(), "a real tile must have spilled");
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&vec![0; original.len()]).unwrap();
        file.sync_all().unwrap();
        Self { file, original, dir }
    }

    fn restore(&mut self) {
        self.file.seek(SeekFrom::Start(0)).unwrap();
        self.file.write_all(&self.original).unwrap();
        self.file.sync_all().unwrap();
    }
}

impl Drop for Damaged {
    fn drop(&mut self) {
        spill::configure(None, 0).unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(size: Size, sample: SampleType, x: i32) -> Document {
    let mut doc = Document::new("channel scratch recovery", size, ColorMode::Rgb, sample);
    let mut layer = Layer::raster("pixels", PixelFormat::new(ColorMode::Rgb, sample, true));
    layer.surface_mut().unwrap().write_pixel(x, 8, &[0.0, 1.0, 0.0, 1.0]);
    doc.layers.push(layer);
    doc
}

fn proxy_and_bounds_retry(sample: SampleType) {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    a.session.add_document(fixture(Size::new(3000, 2000), sample, 700), None);
    a.run("channel.target", json!({"channel":"green"})).unwrap();
    let st = a.session.active().unwrap();
    let (doc, layer) = (st.doc.clone(), st.active_layer);
    let target = json!({"target":{"color":1}});
    let mut bounds = photocraft_engine::channel_transform::AffinePreviewBounds::new(doc.clone(), layer, &target, [0.0, 0.0, 0.0, 1.0]).unwrap();
    let h = photocraft_algo::transform::Homography([2000.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut a, &ctx).unwrap();
    let good = a.ui.transform.as_ref().unwrap().clone();
    crate::transform_tool::cancel(&mut a);
    let mut captured = Preview::new(&doc, layer, [0.0, 0.0, 0.0, 1.0], true).unwrap();
    let mut stretched = good.clone();
    stretched.quad = stretched.quad.map(|[x, y]| [x * 2000.0, y]);
    let mut damaged = Damaged::new(&format!("proxy-{sample:?}"));
    let error = bounds.check(&h).unwrap_err();
    assert!(!matches!(error, photocraft_engine::EngineError::BadParams { .. }), "scratch errors must remain retryable");
    assert!(captured.check_native(&stretched).is_err());
    assert!(Preview::new(&doc, layer, [0.0, 0.0, 0.0, 1.0], true).is_err(), "a placeholder proxy must never be admitted");
    assert!(photocraft_engine::channel_transform::source_bounds(&doc, layer, &target).is_err());
    let mut moved = (*doc).clone();
    let colors = photocraft_engine::color_channels::ColorChannels::from_indices([1], 3).unwrap();
    assert!(photocraft_engine::channel_cmds::move_color_channels(&mut moved, layer.unwrap(), colors, 5, 0, [0.0; 4]).is_err());
    assert!(crate::transform_tool::begin(&mut a, &egui::Context::default()).is_err());
    assert!(a.ui.transform.is_none() && a.transform_preview.is_none());
    assert!(!photocraft_engine::channel_transform::locked(&a.session));
    damaged.restore();
    // Advance the preview's retry gate without sleeping; no test-only native reads
    // clear the failure. The active-preview path itself must discover recovery.
    captured.retry_after = 0.0;
    assert!(captured.check_native(&stretched).is_err());
    assert!(captured.check_native(&stretched).unwrap_err().contains("reach"));
    assert!(captured.check_native(&good).is_ok());
    assert!(bounds.check(&h).is_err(), "a recovery transition cannot admit a result yet");
    assert!(bounds.check(&h).unwrap_err().to_string().contains("reach"), "recovery must recompute the real component bounds");
    let preview = Preview::new(&doc, layer, [0.0, 0.0, 0.0, 1.0], true).unwrap();
    assert_eq!(preview.factor, 2);
    assert_eq!(preview.base.layer(layer.unwrap()).unwrap().surface().unwrap().sample_channel(350, 4, 1), 1.0);
    crate::transform_tool::begin(&mut a, &egui::Context::default()).unwrap();
    crate::transform_tool::cancel(&mut a);
    assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));
}

fn cpu_texture_retry() {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    a.session.add_document(fixture(Size::new(64, 64), SampleType::U8, 8), None);
    a.run("channel.target", json!({"channel":"green"})).unwrap();
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut a, &ctx).unwrap();
    // Refuse geometry before the first CPU resample, retaining the original native tiles.
    let t = a.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + f64::from(photocraft_algo::transform::WARP_REACH) + 64.0, y]);
    let doc = a.session.active().unwrap().doc.clone();
    let mut damaged = Damaged::new("cpu-texture");
    texture(&mut a, &ctx, 0, None).unwrap();
    assert!(a.ui.status_error);
    assert!(a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().textures[&None].view.is_none(), "failed display reads cannot be memoized");
    ctx.tex_manager().write().take_delta().clear();
    for _ in 0..3 {
        texture(&mut a, &ctx, 0, None).unwrap();
        let mut delta = ctx.tex_manager().write().take_delta();
        assert!(delta.set.is_empty(), "failed display pixels cannot be rebuilt on every pointer frame");
        delta.clear();
    }
    damaged.restore();
    a.transform_preview.as_mut().unwrap().channel.as_mut().unwrap().retry_after = 0.0;
    texture(&mut a, &ctx, 0, None).unwrap();
    texture(&mut a, &ctx, 0, None).unwrap();
    assert!(a.transform_preview.as_ref().unwrap().channel.as_ref().unwrap().textures[&None].view.is_some());
    let mut delta = ctx.tex_manager().write().take_delta();
    assert!(!delta.set.is_empty(), "identical geometry and eyes must upload recovered pixels");
    assert!(delta.set.values().flat_map(|deltas| deltas.iter()).any(|d| match &d.image {
        egui::ImageData::Color(image) => image.pixels[8 * 64 + 8].g() > 0,
    }));
    delta.clear();
    crate::transform_tool::cancel(&mut a);
    ctx.tex_manager().write().take_delta().clear();
    assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));
}

fn scalar_proxy_retry() {
    use photocraft_doc::AlphaChannel;
    let mut doc = Document::new("scalar proxy recovery", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("pixels", PixelFormat::RGBA8));
    let mut mask = photocraft_raster::Surface::new(PixelFormat::GRAY8);
    mask.fill_rect(photocraft_geom::Rect::new(400, 400, 5600, 3600), &[0.5]);
    doc.channels.push(AlphaChannel::new("saved", mask));
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    a.session.add_document(doc, None);
    a.run("channel.target", json!({"channel":0})).unwrap();
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut a, &ctx).unwrap();
    let mut t = a.ui.transform.as_ref().unwrap().clone();
    t.quad = t.quad.map(|[x, y]| [x + 4.0, y]);
    let mut p = Preview::new(&a.session.active().unwrap().doc, a.session.active().unwrap().active_layer, [0.0; 4], true).unwrap();
    p.update(&t).unwrap();
    let native = p.native.clone();
    let base = p.base.clone();
    let shown = p.shown.clone();
    let bad = TileCoord::new(2, 1);
    assert!(!Arc::ptr_eq(base.channels[0].surface.tile(bad).unwrap(), shown.channels[0].surface.tile(bad).unwrap()));
    let mut pins = Vec::new();
    for doc in [&native, &base, &shown] {
        for (coord, tile) in doc.channels[0].surface.tiles() {
            if Arc::ptr_eq(doc, &base) && *coord == bad {
                continue;
            }
            pins.push(tile.try_bytes().unwrap());
        }
    }
    let mut damaged = Damaged::new("scalar-proxy");
    assert!(base.channels[0].surface.tile(bad).unwrap().try_bytes().is_err());
    assert!(native.scratch_health().is_ok() && shown.scratch_health().is_ok());
    t.quad = t.quad.map(|[x, y]| [x + 4.0, y]);
    assert!(p.update(&t).is_err());
    assert!(p.check_native(&t).is_err());
    damaged.restore();
    p.retry_after = 0.0;
    assert!(p.check_native(&t).is_err(), "the recovery transition is refused once");
    p.check_native(&t).unwrap();
    p.update(&t).unwrap();
    assert_eq!(p.shown.channels[0].surface.sample_channel(550, 400, 0), 128.0 / 255.0);
    drop(pins);
    crate::transform_tool::cancel(&mut a);
}

fn cpu_move_bounds_retry() {
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut doc = fixture(Size::new(600, 64), SampleType::U8, 8);
    doc.layers[0].surface_mut().unwrap().write_pixel(550, 8, &[0.0, 1.0, 0.0, 1.0]);
    let id = doc.layers[0].id;
    a.session.add_document(doc, None);
    a.run("channel.target", json!({"channel":"green"})).unwrap();
    a.ui.tool = crate::state::Tool::Move;
    a.ui.extras.snap = false;
    a.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut a, crate::canvas::ToolEvent::Down { x: 8.0, y: 8.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut a, crate::canvas::ToolEvent::Move { x: 13.0, y: 8.0, pressure: 1.0 }, m);
    a.session
        .edit("cold native identities", |doc, _| {
            let s = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            s.write_pixel(8, 8, &[0.0, 1.0, 0.0, 1.0]);
            s.write_pixel(550, 8, &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let st = a.session.active().unwrap();
    let (doc, revision) = (st.doc.clone(), st.revision);
    let pin = doc.layer(id).unwrap().surface().unwrap().tile(TileCoord::new(0, 0)).unwrap().try_bytes().unwrap();
    a.move_preview = None;
    let mut sentinel = photocraft_raster::Surface::new(PixelFormat::GRAY8);
    sentinel.write_pixel(0, 0, &[0.5]);
    let mut damaged = Damaged::new("move-bounds");
    assert!(sentinel.tile(TileCoord::new(0, 0)).unwrap().try_bytes().is_err());
    photocraft_compose::discard_unreadable_pixel_caches();
    assert!(doc.scratch_health().is_ok(), "the first active-document read must happen inside the bounds scan");
    assert!(crate::move_ui::display_doc(&mut a, 0).is_none());
    assert!(a.move_preview.is_none(), "a first failed bounds read cannot enter the damage cache");
    damaged.restore();
    assert!(crate::move_ui::display_doc(&mut a, 0).is_none());
    let (shown, key) = crate::move_ui::display_doc(&mut a, 0).unwrap();
    assert_eq!(shown.layer(id).unwrap().surface().unwrap().sample_channel(555, 8, 1), 1.0);
    crate::canvas::tool_event(&mut a, crate::canvas::ToolEvent::Move { x: 15.0, y: 8.0, pressure: 1.0 }, m);
    let (_, next) = crate::move_ui::display_doc(&mut a, 0).unwrap();
    assert!(crate::move_ui::damage(&a, doc.id, revision, key, next).unwrap().contains(557, 8), "recovered far samples must be redrawn at later offsets");
    assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));
    drop(pin);
}

fn gpu_off_canvas_retry() {
    let Ok(rs) =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(crate::gpu_canvas::wgpu_setup(), Default::default())))
    else {
        return;
    };
    if !crate::channel_transform_gpu::test_supported(&rs) {
        return;
    }
    let gpu = crate::gpu_canvas::GpuCanvas::new(&rs);
    {
        let warm = fixture(Size::new(64, 64), SampleType::U8, 8);
        let layer = Some(warm.layers[0].id);
        crate::channel_transform_gpu::test_wait(&rs, || crate::channel_transform_gpu::Preview::new(&gpu, &warm, layer, [0.0; 4])).unwrap();
    }
    let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut doc = fixture(Size::new(64, 64), SampleType::U8, 8);
    let id = doc.layers[0].id;
    doc.layers[0].surface_mut().unwrap().write_pixel(300, 8, &[0.0, 0.5, 0.0, 1.0]);
    a.session.add_document(doc, None);
    a.run("channel.target", json!({"channel":"green"})).unwrap();
    a.set_wgpu(rs.clone());
    a.ui.tool = crate::state::Tool::Move;
    a.ui.extras.snap = false;
    a.ui.view.show.smart_guides = false;
    let doc = a.session.active().unwrap().doc.clone();
    let surface = doc.layer(id).unwrap().surface().unwrap();
    // Keep the uploadable canvas tile resident. Only off-canvas bytes are damaged.
    let pin = surface.tile(TileCoord::new(0, 0)).unwrap().try_bytes().unwrap();
    let mut damaged = Damaged::new("gpu-off-canvas");
    assert!(surface.tile(TileCoord::new(1, 0)).unwrap().try_bytes().is_err());
    for factor in [1, 2] {
        assert!(crate::channel_transform_gpu::Preview::new_move(&gpu, &doc, Some(id), [0.0; 4], factor).is_none());
    }
    assert!(crate::channel_transform_gpu::Preview::new(&gpu, &doc, Some(id), [0.0; 4]).is_none());
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut a, crate::canvas::ToolEvent::Down { x: 8.0, y: 8.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut a, crate::canvas::ToolEvent::Move { x: 13.0, y: 8.0, pressure: 1.0 }, m);
    assert!(crate::move_ui::channel_texture(&mut a, 0, None).is_none());
    assert!(crate::move_ui::display_doc(&mut a, 0).is_none());
    damaged.restore();
    assert!(crate::move_ui::channel_texture(&mut a, 0, None).is_none(), "recovered off-canvas pixels still require the CPU source");
    let shown = crate::move_ui::display_doc(&mut a, 0).unwrap().0;
    assert_eq!(shown.layer(id).unwrap().surface().unwrap().sample_channel(305, 8, 1), surface.sample_channel(300, 8, 1));
    assert_eq!(shown.layer(id).unwrap().surface().unwrap().sample_channel(13, 8, 1), 1.0);
    assert!(Arc::ptr_eq(&doc, &a.session.active().unwrap().doc));
    drop(pin);
}

#[test]
fn channel_previews_refuse_scratch_placeholders_and_recover() {
    const CHILD: &str = "PHOTOCRAFT_CHANNEL_SCRATCH_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "canvas::channel_transform_preview::scratch_tests::channel_previews_refuse_scratch_placeholders_and_recover",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "child failed: {}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        proxy_and_bounds_retry(sample);
    }
    cpu_texture_retry();
    scalar_proxy_retry();
    cpu_move_bounds_retry();
    gpu_off_canvas_retry();
}
