//! #11122: the sharp filter (crisp texels with a one-device-pixel edge) depends only on where and
//! how large the stored texels are drawn, not on how much a viewport page was reduced. Drawn
//! through the production callback and shader on a real device, with synthetic pixels.

use super::*;
use photocraft_compose::Buffer;
use photocraft_doc::SampleType;
use photocraft_geom::Rect;

/// Texels per side of the test texture.
const TEXELS: usize = 4;
/// Side of each drawn rect, in device pixels; its centre is the view centre.
const CELL: usize = 80;

/// A 4 × 4 texture, black before texel `edge` and white from it, across columns (`vertical`) or
/// rows.
fn edge_texture(edge: usize, vertical: bool) -> Buffer {
    let mut buf = Buffer::filled(Rect::from_xywh(0, 0, TEXELS as u32, TEXELS as u32), [0.0, 0.0, 0.0, 1.0]);
    for (i, px) in buf.px.iter_mut().enumerate() {
        let at = if vertical { i % TEXELS } else { i / TEXELS };
        if at >= edge {
            *px = [1.0, 1.0, 1.0, 1.0];
        }
    }
    buf
}

fn view(doc: u64, doc_size: [u32; 2], zoom: f32) -> ViewParams {
    let center = [doc_size[0] as f32 / 2.0, doc_size[1] as f32 / 2.0];
    ViewParams { doc, doc_size, zoom, center, rotation: 0.0, view_key: doc, display: 0, output: 0, hdr: None }
}

/// One comparison: the same texels as a page reduced by `factor` and at full resolution, both
/// drawn at `per_texel` device pixels per stored texel.
struct Case {
    factor: u32,
    vertical: bool,
    per_texel: f32,
    edge: usize,
}

impl Case {
    /// Where the black-to-white edge falls along the scanned axis, in cell pixels.
    fn edge_at(&self) -> f32 {
        CELL as f32 / 2.0 - (TEXELS as f32 / 2.0 - self.edge as f32) * self.per_texel
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
    // Every reduction a viewport page uses up to 8, for an edge across columns and across rows.
    // The document scale is 2 (aligned) or 2 + 0.5 / factor (fractional), so both draws take the
    // sharp path (`filter_mode`). Aligned: the edge at texel 2 lands on the cell centre, a pixel
    // boundary. Fractional: 2 × factor + 0.5 px per texel puts the edge at texel 1 inside a pixel,
    // where exactly one pixel shows the ramp (nearest sampling would show none).
    let mut cases = Vec::new();
    for factor in [2u32, 4, 8] {
        for vertical in [true, false] {
            cases.push(Case { factor, vertical, per_texel: 2.0 * factor as f32, edge: 2 });
            cases.push(Case { factor, vertical, per_texel: 2.0 * factor as f32 + 0.5, edge: 1 });
        }
    }
    let side = TEXELS as u32;
    let mut draws = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let tex = edge_texture(c.edge, c.vertical);
        let (reduced, full) = (2 * i as u64 + 1, 2 * i as u64 + 2);
        let reduced_size = [side * c.factor, side * c.factor];
        for (key, size, factor) in [(reduced, reduced_size, c.factor), (full, [side, side], 1)] {
            let core = Rect::new(0, 0, size[0] as i32, size[1] as i32);
            assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth: SampleType::U8, factor, core, buf: &tex }), "page {key}");
        }
        draws.push((view(reduced, reduced_size, c.per_texel / c.factor as f32), view(full, [side, side], c.per_texel)));
    }
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    // One row per case: the reduced page in the left cell, the full one in the right.
    let cell = CELL as f32;
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(2.0 * cell, cell * cases.len() as f32)).with_pixels_per_point(1.0).renderer(renderer).build_ui(
            move |ui| {
                let o = ui.ctx().content_rect().min;
                for (i, (reduced, full)) in draws.iter().enumerate() {
                    let at = |col: f32| egui::Rect::from_min_size(o + egui::vec2(col * cell, i as f32 * cell), egui::vec2(cell, cell));
                    GpuCanvas::paint(ui.painter(), at(0.0), *reduced);
                    GpuCanvas::paint(ui.painter(), at(1.0), *full);
                }
            },
        );
    h.run_steps(2);
    let img = h.render().expect("render");
    assert!(gpu.health.is_ok(), "{:?}", gpu.fault());
    for (i, c) in cases.iter().enumerate() {
        let what = format!("factor {}, {} edge, {} px per texel", c.factor, if c.vertical { "vertical" } else { "horizontal" }, c.per_texel);
        // The image spans the cell centre ± 2 texels; scan across the edge inside it, on lines
        // through and either side of the centre.
        let half = (TEXELS as f32 / 2.0 * c.per_texel) as usize;
        let (lo, hi) = (CELL / 2 - half + 1, CELL / 2 + half - 1);
        let edge = c.edge_at();
        for line in [CELL / 2 - 6, CELL / 2, CELL / 2 + 5] {
            let px = |col: usize, along: usize| {
                let (x, y) = if c.vertical { (along, line) } else { (line, along) };
                img.get_pixel((col * CELL + x) as u32, (i * CELL + y) as u32).0
            };
            for along in lo..hi {
                let (reduced, full) = (px(0, along), px(1, along));
                for ch in 0..3 {
                    // Identical texels at an identical place and size: equal up to rounding.
                    assert!(reduced[ch].abs_diff(full[ch]) <= 1, "{what}, line {line}, at {along}: reduced {reduced:?}, full {full:?}");
                }
                // Exactly one device pixel of ramp, where the edge falls: none when it lands on
                // a pixel boundary, the pixel it crosses when it lands inside one.
                let centre = along as f32 + 0.5;
                let v = full[0];
                if centre < edge {
                    assert!(v < 16, "{what}, line {line}: dark before the edge at {edge}, {along} is {v}");
                } else if centre > edge {
                    assert!(v > 239, "{what}, line {line}: light after the edge at {edge}, {along} is {v}");
                } else {
                    assert!((16..240).contains(&v), "{what}, line {line}: the pixel the edge crosses at {edge} shows the ramp, {v}");
                }
            }
        }
    }
}

#[test]
fn sharp_footprints_exclude_zero_weight_neighbours() {
    let hit = |x, scale, column| sampled_doc_rect(x, 10.5, scale).contains(column, 10);
    assert!(hit(100.625, 4.0, 100));
    assert!(!hit(100.625, 4.0, 101));
    assert!(!hit(100.875, 4.0, 101)); // The ramp endpoint still has exactly zero weight.
    assert!(hit(100.9375, 4.0, 100));
    assert!(hit(100.9375, 4.0, 101));
    assert!(!hit(101.125, 4.0, 100));
    assert!(hit(101.125, 4.0, 101));
    assert!(hit(-0.375, 4.0, -1));
    assert!(!hit(-0.375, 4.0, 0));
    assert!(hit(-0.0625, 4.0, -1));
    assert!(hit(-0.0625, 4.0, 0));
    assert!(!hit(100.5, 1.5, 101));
    assert!(hit(100.625, 1.5, 101)); // Non-sharp bilinear keeps its positive neighbour.
}

#[test]
fn sampling_tracks_mip_boundaries_and_preview_origin() {
    let grid = callback::SamplingGrid { rect: [0, 0, 16, 16], core: Rect::from_xywh(0, 0, 16, 16), texels: [16, 16], levels: 5, opaque: None };
    let mut sample = GpuSampling { grids: vec![grid].into(), scale: 0.5, factor: 1, growth: [0.0; 2] };
    let hit = |sample: &GpuSampling, x, y| sample.any_region(8.0, 8.0, |r| r.contains(x, y));
    assert!(hit(&sample, 6, 6));
    assert!(!hit(&sample, 5, 6));
    assert!(!hit(&sample, 10, 6));
    sample.scale = 0.499;
    assert!(hit(&sample, 4, 4)); // The next mip has acquired a positive trilinear weight.
    sample.scale = 0.501;
    assert!(!hit(&sample, 4, 4));
    sample.scale = 0.25;
    assert!(hit(&sample, 11, 11));
    assert!(!hit(&sample, 12, 11));
    // A factor-four preview grown around the old image centre starts at negative document
    // coordinates. Its first stored texel, not the requested document grid, is sampled here.
    sample.scale = 4.0;
    sample.factor = 4;
    sample.growth = [0.5, 0.5];
    assert!(sample.any_region(-1.5, -1.5, |r| r.contains(-2, -2)));
    assert!(!sample.any_region(-1.5, -1.5, |r| r.contains(2, 2)));
    for (x, y, scale) in [(f64::NAN, 0.0, 1.0), (0.0, f64::INFINITY, 1.0), (0.0, 0.0, 0.0), (0.0, 0.0, f32::NAN)] {
        sample.scale = scale;
        assert!(!sample.any_region(x, y, |_| panic!("invalid query must not reach the ink consumer")));
    }
}

#[test]
fn fractional_lod_visits_all_mips_before_testing_occlusion() {
    let core = Rect::from_xywh(0, 0, 16, 16);
    let coarse = callback::SamplingGrid { rect: [0, 0, 16, 16], core, texels: [2, 2], levels: 1, opaque: None };
    // At (8,8), level1 covers base [6,10)² and is opaque; level2 also contributes
    // [4,12)² and includes transparent texels. Neither its ink nor the earlier page may
    // disappear merely because level1 was visited first.
    let opaque = OpaqueTexels::new(16, 16, (0..256).map(|i| (6..10).contains(&(i % 16)) && (6..10).contains(&(i / 16)))).unwrap();
    let fine = callback::SamplingGrid { rect: [0, 0, 16, 16], core, texels: [16, 16], levels: 5, opaque: Some(opaque) };
    let sampling = GpuSampling { grids: vec![coarse.clone(), fine.clone()].into(), scale: 0.4, factor: 1, growth: [0.0; 2] };
    assert!(sampling.any_region(8.0, 8.0, |r| r.contains(4, 4)), "second mip's own ink remains hittable");
    assert!(sampling.any_region(8.0, 8.0, |r| r.contains(1, 1)), "translucent trilinear sample leaves the coarse page visible");
    let fine = callback::SamplingGrid { opaque: OpaqueTexels::new(16, 16, [true; 256]), ..fine };
    let sampling = GpuSampling { grids: vec![coarse, fine].into(), ..sampling };
    assert!(!sampling.any_region(8.0, 8.0, |r| r.contains(1, 1)), "only a fully opaque trilinear sample hides the coarse page");
    assert!(sampling.any_region(8.0, 8.0, |r| r.contains(4, 4)), "opaque grids still test all their own mip support");
}

#[test]
fn odd_mip_bounds_preserve_all_positive_source_taps() {
    let grid = callback::SamplingGrid { rect: [0, 0, 5, 5], core: Rect::from_xywh(0, 0, 5, 5), texels: [5, 5], levels: 3, opaque: None };
    let sampling = GpuSampling { grids: vec![grid].into(), scale: 0.5, factor: 1, growth: [0.0; 2] };
    // The 5→2 downsample taps columns {0,1} and {3,4}, not a five-pixel box.
    // Bounds preserve every positive tap without claiming the interior gap is exact.
    for y in [0, 1, 3, 4] {
        for x in [0, 1, 3, 4] {
            assert!(sampling.any_region(2.5, 2.5, |r| r.contains(x, y)));
        }
    }
    assert!(!sampling.any_region(2.5, 2.5, |r| r.contains(5, 4)));
}

#[test]
fn opacity_patches_preserve_painted_snapshots_and_bounds() {
    let mut mask = OpaqueTexels::new(3, 2, [true; 6]).unwrap();
    let painted = mask.clone();
    mask.update([1, 1], 1, [false]);
    assert!(painted.all(0, 0, 3, 2));
    assert!(!mask.all(0, 0, 3, 2));
    assert!(mask.all(0, 0, 3, 1));
    assert!(!mask.all(3, 0, 4, 1));
    assert!(!mask.all(0, 2, 1, 3));
    assert!(!mask.all(0, 0, 0, 1));
    assert!(OpaqueTexels::new(u32::MAX, u32::MAX, []).is_none());
}

#[test]
fn resident_pages_and_sharp_footprints_match_rendered_ink() {
    let _gpu = photocraft_testkit::gpu_lock();
    let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default()));
    let Ok(rs) = rs else {
        eprintln!("skipping sampling readback: no GPU adapter");
        return;
    };
    let gpu = GpuCanvas::new(&rs);
    let mut draws = Vec::new();
    let mut probes = Vec::new();
    let mut key = 500;
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        // Resident factor-eight pixels remain visible at 1x while a finer page is pending.
        let size = [128, 16];
        let coarse = Buffer::filled(Rect::from_xywh(12, 0, 4, 2), [1.0, 0.0, 0.0, 1.0]);
        assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth, factor: 8, core: Rect::from_xywh(100, 0, 20, 16), buf: &coarse }));
        let old = gpu.sampling(key, size, 1.0, 1, [0.0; 2]);
        assert!(old.any_region(101.5, 8.5, |r| r.contains(97, 8)));
        assert!(!old.any_region(99.5, 8.5, |_| true)); // The apron is sampled, never drawn.
        draws.push(ViewParams { center: [112.0, 8.0], ..view(key, size, 1.0) });
        probes.push((old, 101.5, 8.5, 97, true, [255, 0, 0]));
        key += 1;
        // Transparent fine detail leaves that coarse ink visible; opaque blue detail hides
        // it, so upper imported ink solely in column97 cannot steal the lower blue press.
        for opaque in [false, true] {
            assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth, factor: 8, core: Rect::from_xywh(96, 0, 32, 16), buf: &coarse }));
            let fine = Buffer::filled(Rect::from_xywh(96, 0, 32, 16), if opaque { [0.0, 0.0, 1.0, 1.0] } else { [0.0; 4] });
            assert!(gpu.upload_page(PageUpload { key, page: 2, size, depth, factor: 1, core: fine.rect, buf: &fine }));
            let sampling = gpu.sampling(key, size, 1.0, 1, [0.0; 2]);
            draws.push(ViewParams { center: [112.0, 8.0], ..view(key, size, 1.0) });
            probes.push((sampling, 101.5, 8.5, 97, !opaque, if opaque { [0, 0, 255] } else { [255, 0, 0] }));
            if opaque {
                let before = gpu.sampling(key, size, 1.0, 1, [0.0; 2]);
                let patch = Buffer::filled(Rect::from_xywh(101, 8, 1, 1), [0.0; 4]);
                assert!(gpu.update_page(key, 2, &patch));
                let after = gpu.sampling(key, size, 1.0, 1, [0.0; 2]);
                assert!(!before.any_region(101.5, 8.5, |r| r.contains(97, 8)));
                assert!(after.any_region(101.5, 8.5, |r| r.contains(97, 8)));
                let restore = Buffer::filled(patch.rect, [0.0, 0.0, 1.0, 1.0]);
                assert!(gpu.update_page(key, 2, &restore));
            }
            key += 1;
        }
        // Replacing the same resident page really switches to the full-resolution grid.
        assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth, factor: 8, core: Rect::from_xywh(96, 0, 32, 16), buf: &coarse }));
        let stale = gpu.sampling(key, size, 1.0, 1, [0.0; 2]);
        let mut fine = Buffer::filled(Rect::from_xywh(96, 0, 32, 16), [0.0, 0.0, 1.0, 1.0]);
        for row in fine.px.as_chunks_mut::<32>().0 {
            row[1] = [1.0, 0.0, 0.0, 1.0];
        }
        assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth, factor: 1, core: fine.rect, buf: &fine }));
        assert!(stale.any_region(101.5, 8.5, |r| r.contains(97, 8)));
        let sampling = gpu.sampling(key, size, 1.0, 1, [0.0; 2]);
        draws.push(ViewParams { center: [112.0, 8.0], ..view(key, size, 1.0) });
        probes.push((sampling, 101.5, 8.5, 97, false, [0, 0, 255]));
        key += 1;
        // Exact 4x sample x100.625 has zero weight for red column101. At 4.5x the
        // device pixel centred on the edge has positive weights for both columns.
        let mut tex = Buffer::filled(Rect::from_xywh(0, 0, 128, 16), [0.0, 0.0, 1.0, 1.0]);
        for row in tex.px.as_chunks_mut::<128>().0 {
            row[101] = [1.0, 0.0, 0.0, 1.0];
        }
        for scale in [4.0, 4.5] {
            gpu.upload_buffer_full(key, &tex, depth);
            let x = if scale == 4.0 { 100.625 } else { 101.0 };
            let sampling = gpu.sampling(key, size, scale, 1, [0.0; 2]);
            draws.push(view(key, size, scale));
            probes.push((sampling, x, 8.5, 101, scale != 4.0, if scale == 4.0 { [0, 0, 255] } else { [128, 0, 128] }));
            key += 1;
        }
    }
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(640.0, draws.len() as f32 * 80.0)).with_pixels_per_point(1.0).renderer(renderer).build_ui(
            move |ui| {
                let origin = ui.ctx().content_rect().min;
                for (i, params) in draws.iter().enumerate() {
                    let rect = egui::Rect::from_min_size(origin + egui::vec2(0.0, i as f32 * 80.0), egui::vec2(640.0, 80.0));
                    GpuCanvas::paint(ui.painter(), rect, *params);
                }
            },
        );
    h.run_steps(2);
    let image = h.render().expect("render");
    assert!(gpu.health.is_ok(), "{:?}", gpu.fault());
    for (i, (sample, x, y, column, expected_hit, expected_rgb)) in probes.iter().enumerate() {
        assert_eq!(sample.any_region(*x, *y, |r| r.contains(*column, 8)), *expected_hit, "probe {i}");
        // The first four cases pan to x112 at1x. Sharp texture sets use centre64,
        // and the callback rounds the image origin just as this device-pixel lookup does.
        let case = i % 6;
        let (scale, center): (f64, f64) = if case < 4 {
            (1.0, 112.0)
        } else if case == 4 {
            (4.0, 64.0)
        } else {
            (4.5, 64.0)
        };
        let px = ((320.0 - center * scale).round() + *x * scale).floor() as u32;
        let py = (i * 80) as u32 + ((40.0_f64 - 8.0 * scale).round() + *y * scale).floor() as u32;
        let shown = image.get_pixel(px, py).0;
        for ch in 0..3 {
            assert!(shown[ch].abs_diff(expected_rgb[ch]) <= 2, "probe {i}: shown {shown:?}, expected {expected_rgb:?}");
        }
    }
}
