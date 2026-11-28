//! #17099: the GPU picture's texels start at a whole device pixel (crisp at 100 %), but the canvas draws the
//! checkerboard under it and the edge rule around it at the document's exact place on screen. The picture must cover
//! exactly the pixels that exact rect covers: a pixel it leaves out shows a line of checkerboard along the image's
//! edge, and one past it paints over the edge rule. Drawn through the production callback and shader on a real
//! device, with synthetic pixels.

use super::*;
use photocraft_compose::Buffer;
use photocraft_doc::SampleType;
use photocraft_geom::Rect;

/// Side of the test document, in document pixels.
const SIDE: u32 = 8;
/// Side of each drawn cell, in device pixels; the view centre is its centre.
const CELL: usize = 80;
/// The document's colour (opaque): nothing else drawn is this red.
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// One view of the document: device pixels per document pixel and the document point at the cell centre.
#[derive(Clone, Copy, Debug)]
struct Case {
    zoom: f32,
    center: [f32; 2],
    /// Drawn from viewport pages (their cores bound what they draw) rather than the document's texture set.
    pages: bool,
}

impl Case {
    /// The pixels whose centre lies in the document's exact rect on screen, along each axis: from the first to one
    /// past the last.
    fn covered(&self, axis: usize) -> (usize, usize) {
        let lo = CELL as f32 / 2.0 - self.center[axis] * self.zoom;
        let hi = lo + SIDE as f32 * self.zoom;
        ((lo - 0.5).ceil() as usize, (hi - 0.5).ceil() as usize)
    }
}

#[test]
fn the_picture_covers_exactly_the_documents_rect_on_screen() {
    let _gpu = photocraft_testkit::gpu_lock();
    let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default()));
    let Ok(rs) = rs else {
        eprintln!("skipping canvas edge readback: no GPU adapter");
        return;
    };
    let gpu = GpuCanvas::new(&rs);
    // Edges on a half pixel (the tie the whole-pixel origin rounds away from), just either side of one, and widths
    // that are not a whole number of device pixels, in each filter mode (zoomed out, between 1× and 2×, sharp).
    let mut cases = Vec::new();
    for zoom in [0.75f32, 1.5, 2.5, 2.6, 4.0] {
        for c in [4.0f32, 4.1, 4.2, 4.3] {
            // Centre `c` puts the left edge at 40 - c × zoom; nudge it to a half pixel for the tie.
            let tie = (CELL as f32 / 2.0 - (CELL as f32 / 2.0 - c * zoom).floor() - 0.5) / zoom;
            for center in [c, tie] {
                for pages in [false, true] {
                    cases.push(Case { zoom, center: [center, center], pages });
                }
            }
        }
    }
    let buf = Buffer::filled(Rect::from_xywh(0, 0, SIDE, SIDE), RED);
    let mut draws = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let key = i as u64 + 1;
        if c.pages {
            let core = Rect::new(0, 0, SIDE as i32, SIDE as i32);
            assert!(gpu.upload_page(PageUpload { key, page: 1, size: [SIDE, SIDE], depth: SampleType::U8, factor: 1, core, buf: &buf }), "page {key}");
        } else {
            gpu.upload_buffer_full(key, &buf, SampleType::U8);
        }
        draws.push(ViewParams {
            doc: key,
            doc_size: [SIDE, SIDE],
            zoom: c.zoom,
            center: c.center,
            rotation: 0.0,
            view_key: key,
            display: 0,
            output: 0,
            hdr: None,
        });
    }
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let cell = CELL as f32;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(cell, cell * cases.len() as f32)).with_pixels_per_point(1.0).renderer(renderer).build_ui(
        move |ui| {
            let o = ui.ctx().content_rect().min;
            for (i, params) in draws.iter().enumerate() {
                let at = egui::Rect::from_min_size(o + egui::vec2(0.0, i as f32 * cell), egui::vec2(cell, cell));
                GpuCanvas::paint(ui.painter(), at, *params);
            }
        },
    );
    h.run_steps(2);
    let img = h.render().expect("render");
    assert!(gpu.health.is_ok(), "{:?}", gpu.fault());
    for (i, c) in cases.iter().enumerate() {
        let red = |x: usize, y: usize| {
            let p = img.get_pixel(x as u32, (i * CELL + y) as u32).0;
            p[0] > 200 && p[1] < 60 && p[2] < 60
        };
        let (x0, x1) = c.covered(0);
        let (y0, y1) = c.covered(1);
        // Across the middle row and down the middle column, one pixel either side of each edge.
        let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
        for x in x0 - 1..=x1 {
            assert_eq!(red(x, my), (x0..x1).contains(&x), "{c:?}: column {x} (the document covers {x0}..{x1})");
        }
        for y in y0 - 1..=y1 {
            assert_eq!(red(mx, y), (y0..y1).contains(&y), "{c:?}: row {y} (the document covers {y0}..{y1})");
        }
    }
}
