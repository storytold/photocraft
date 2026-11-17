//! Pushing an edge in leaves transparency (or a fill colour) behind, not the edge pixel stretched.
use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool, PastBounds, ProxyImage, apply_liquify, apply_liquify_past};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

const CANVAS: Rect = Rect { x0: 0, y0: 0, x1: 96, y1: 64 };

/// An opaque red layer covering the canvas, with or without an alpha channel.
fn red(alpha: bool) -> Surface {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, alpha);
    let mut s = Surface::new(fmt);
    let px: &[f32] = if alpha { &[1.0, 0.0, 0.0, 1.0] } else { &[1.0, 0.0, 0.0] };
    s.fill_rect(CANVAS, px);
    s
}

/// Forward Warp from the left edge into the canvas: the left columns are pushed right.
fn push_in_from_left() -> LiquifyField {
    let mut warp = LiquifyStroke::new(LiquifyTool::ForwardWarp, 60.0);
    warp.density = 100.0;
    warp.points = vec![vec![0.0, 32.0], vec![24.0, 32.0]];
    LiquifyField::from_strokes(CANVAS, 2.0, &[warp])
}

#[test]
fn a_layer_with_transparency_is_left_transparent_where_the_edge_moved_in() {
    let field = push_in_from_left();
    assert!(field.max_displacement() > 8.0, "the stroke moved the edge");
    let out = apply_liquify(&red(true), &field);
    let edge = out.pixel(0, 32);
    assert!(edge[3] < 0.05, "the pushed-in edge is transparent, not the edge pixel repeated: {edge:?}");
    // Away from the stroke the layer is untouched.
    assert_eq!(out.pixel(0, 0), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(out.pixel(80, 32), vec![1.0, 0.0, 0.0, 1.0]);
    // The image itself moved in, still red and opaque.
    let inside = out.pixel(30, 32);
    assert!(inside[3] > 0.99 && inside[0] > 0.99, "{inside:?}");
}

#[test]
fn a_layer_without_transparency_is_filled_with_the_given_colour() {
    let field = push_in_from_left();
    let out = apply_liquify_past(&red(false), &field, PastBounds::Pixel(&[0.0, 0.0, 1.0]));
    let edge = out.pixel(0, 32);
    assert!(edge[2] > 0.95 && edge[0] < 0.05, "the pushed-in edge is the fill colour: {edge:?}");
    assert_eq!(out.pixel(0, 0), vec![1.0, 0.0, 0.0]);
    // Edge still repeats the edge pixel, and a source without alpha defaults to it.
    for out in [apply_liquify_past(&red(false), &field, PastBounds::Edge), apply_liquify(&red(false), &field)] {
        assert_eq!(out.pixel(0, 32), vec![1.0, 0.0, 0.0]);
    }
}

#[test]
fn a_tile_pulled_wholly_from_past_the_canvas_is_filled() {
    // A strong push moves the first tile's whole footprint off the canvas.
    let mut warp = LiquifyStroke::new(LiquifyTool::ForwardWarp, 200.0);
    warp.density = 100.0;
    warp.pressure = 100.0;
    warp.points = vec![vec![0.0, 32.0], vec![90.0, 32.0]];
    let field = LiquifyField::from_strokes(CANVAS, 2.0, &std::iter::repeat_n(warp, 6).collect::<Vec<_>>());
    let out = apply_liquify(&red(true), &field);
    let edge = out.pixel(0, 32);
    assert!(edge[3] < 0.05, "{edge:?}");
}

#[test]
fn the_previews_show_transparency_where_the_edge_moved_in() {
    let field = push_in_from_left();
    let src = red(true);
    let proxy = ProxyImage::new(&src, CANVAS, 1600);
    let mut out = vec![[0u8; 4]; proxy.w * proxy.h];
    proxy.render(&field, [0, 0, proxy.w, proxy.h], &mut out);
    assert!(out[32 * proxy.w][3] < 13, "proxy: {:?}", out[32 * proxy.w]);
    assert_eq!(out[0], [255, 0, 0, 255]);
    let mut full = vec![[0u8; 4]; (CANVAS.width() * CANVAS.height()) as usize];
    let mut scratch = Vec::new();
    assert!(ProxyImage::render_full_resolution(&src, &field, CANVAS, CANVAS, &mut full, &mut scratch));
    let w = CANVAS.width() as usize;
    assert!(full[32 * w][3] < 13, "full resolution: {:?}", full[32 * w]);
    assert_eq!(full[0], [255, 0, 0, 255]);
    // A layer without alpha previews transparent too; the fill comes when the filter is applied.
    let proxy = ProxyImage::new(&red(false), CANVAS, 1600);
    proxy.render(&field, [0, 0, proxy.w, proxy.h], &mut out);
    assert!(out[32 * proxy.w][3] < 13, "opaque proxy: {:?}", out[32 * proxy.w]);
}
