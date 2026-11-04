//! #11122: the sharp filter (crisp texels with a one-device-pixel edge) depends only on where and
//! how large the stored texels are drawn, not on how much a viewport page was reduced. Drawn
//! through the production callback and shader on a real device, with synthetic pixels.

use super::*;
use photocraft_compose::Buffer;
use photocraft_doc::SampleType;
use photocraft_geom::Rect;

/// An 8 × 4 texture, black left of column `edge` and white from it.
fn edge_texture(edge: usize) -> Buffer {
    let mut buf = Buffer::filled(Rect::from_xywh(0, 0, 8, 4), [0.0, 0.0, 0.0, 1.0]);
    for (i, px) in buf.px.iter_mut().enumerate() {
        if i % 8 >= edge {
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
    // The same stored texels as a page reduced by 2 (a 16 × 8 document) and at full resolution
    // (an 8 × 4 document), drawn at the same origin (the view centre) and texel size. Aligned:
    // the edge after column 4 at 4 device px per texel lands on a pixel boundary. Fractional:
    // the edge after column 3 at 4.5 px per texel lands at x = 6 + 13.5, inside a pixel, where
    // the one-device-pixel ramp must show (nearest sampling would not).
    let (half, full, half_frac, full_frac) = (1, 2, 3, 4);
    for (key, size, factor, edge) in [(half, [16, 8], 2, 4), (full, [8, 4], 1, 4), (half_frac, [16, 8], 2, 3), (full_frac, [8, 4], 1, 3)] {
        let core = Rect::new(0, 0, size[0] as i32, size[1] as i32);
        let tex = edge_texture(edge);
        assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth: SampleType::U8, factor, core, buf: &tex }), "page {key}");
    }
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    // Side by side: the reduced page in the left 48 × 32 rect, the full one in the right; the
    // aligned pair on top, the fractional pair below.
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(96.0, 64.0)).with_pixels_per_point(1.0).renderer(renderer).build_ui(move |ui| {
        let r = ui.ctx().content_rect();
        let (top, bottom) = r.split_top_bottom_at_y(r.min.y + 32.0);
        let (left, right) = top.split_left_right_at_x(r.min.x + 48.0);
        GpuCanvas::paint(ui.painter(), left, view(half, [16, 8], 2.0));
        GpuCanvas::paint(ui.painter(), right, view(full, [8, 4], 4.0));
        let (left, right) = bottom.split_left_right_at_x(r.min.x + 48.0);
        GpuCanvas::paint(ui.painter(), left, view(half_frac, [16, 8], 2.25));
        GpuCanvas::paint(ui.painter(), right, view(full_frac, [8, 4], 4.5));
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
    // Fractional: the image spans x 6..42 and y 39..57 in each lower rect; the edge is at 19.5.
    let row = 48;
    let (a, b): (Vec<u8>, Vec<u8>) = (10..30).map(|x| (img.get_pixel(x, row)[0], img.get_pixel(48 + x, row)[0])).unzip();
    assert!(b.first().is_some_and(|&v| v < 16) && b.last().is_some_and(|&v| v > 239), "the edge is drawn: {b:?}");
    assert!(b.iter().any(|&v| (16..240).contains(&v)), "a fractional edge shows the one-pixel ramp: {b:?}");
    assert_eq!(a, b, "the fractional edge filters the same at either reduction");
}
