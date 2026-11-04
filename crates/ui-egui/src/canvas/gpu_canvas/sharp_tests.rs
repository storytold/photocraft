//! #11122: the sharp filter (crisp texels with a one-device-pixel edge) depends only on where and
//! how large the stored texels are drawn, not on how much a viewport page was reduced. Drawn
//! through the production callback and shader on a real device, with synthetic pixels.

use super::*;
use photocraft_compose::Buffer;
use photocraft_doc::SampleType;
use photocraft_geom::Rect;

/// An 8 × 4 texture, columns 0–3 black and 4–7 white.
fn edge_texture() -> Buffer {
    let mut buf = Buffer::filled(Rect::from_xywh(0, 0, 8, 4), [0.0, 0.0, 0.0, 1.0]);
    for (i, px) in buf.px.iter_mut().enumerate() {
        if i % 8 >= 4 {
            *px = [1.0, 1.0, 1.0, 1.0];
        }
    }
    buf
}

fn view(doc: u64, doc_size: [u32; 2], zoom: f32) -> ViewParams {
    let center = [doc_size[0] as f32 / 2.0, doc_size[1] as f32 / 2.0];
    ViewParams {
        doc,
        doc_size,
        zoom,
        center,
        rotation: 0.0,
        shadow: false,
        pixel_grid: false,
        view_key: doc,
        display: 0,
        output: 0,
        hdr: None,
        offset: [0.0, 0.0],
    }
}

#[test]
fn sharp_edges_match_at_every_page_reduction() {
    let _gpu = photocraft_testkit::gpu_lock();
    let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default()));
    let Ok(rs) = rs else {
        eprintln!("skipping sharp filter readback: no GPU adapter");
        return;
    };
    let gpu = GpuCanvas::new(&rs);
    let tex = edge_texture();
    // The same stored texels as a page reduced by 2 (a 16 × 8 document drawn at 2×) and at full
    // resolution (an 8 × 4 document drawn at 4×): four device pixels per texel either way, at the
    // same origin (the view centre).
    let (half, full) = (1, 2);
    for (key, size, factor) in [(half, [16, 8], 2), (full, [8, 4], 1)] {
        let core = Rect::new(0, 0, size[0] as i32, size[1] as i32);
        assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth: SampleType::U8, factor, core, buf: &tex }), "page {key}");
    }
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    // Side by side: the reduced page in the left 48 × 32 rect, the full one in the right.
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(96.0, 32.0)).with_pixels_per_point(1.0).renderer(renderer).build_ui(move |ui| {
        let r = ui.ctx().content_rect();
        let (left, right) = r.split_left_right_at_x(r.min.x + 48.0);
        GpuCanvas::paint(ui.painter(), left, view(half, [16, 8], 2.0));
        GpuCanvas::paint(ui.painter(), right, view(full, [8, 4], 4.0));
    });
    h.run_steps(2);
    let img = h.render().expect("render");
    assert!(gpu.health.is_ok(), "{:?}", gpu.fault());
    // In each rect the 32 × 16 px image sits at x 8..40; the edge between texel columns 3 and 4
    // is at x = 8 + 4 × 4 = 24.
    let row = 16;
    let (a, b): (Vec<u8>, Vec<u8>) = (16..32).map(|x| (img.get_pixel(x, row)[0], img.get_pixel(48 + x, row)[0])).unzip();
    assert!(b.first().is_some_and(|&v| v < 16) && b.last().is_some_and(|&v| v > 239), "the edge is drawn: {b:?}");
    assert_eq!(a, b, "the same texels at the same place and size filter the same at either reduction");
}
