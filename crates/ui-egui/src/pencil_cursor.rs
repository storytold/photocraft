//! The Pencil's cursor, and the Pencil- and Block-mode Eraser's: the outline of the whole pixels
//! one dab at the pointer fills, on the pixel grid (#5262). From 4 px up a round aliased dab is a
//! stair-stepped disc, which the smooth circle that stood for it cut through.

use std::sync::Arc;

use egui::{CursorIcon, Painter, Pos2, Shape, Stroke, pos2, vec2};
use photocraft_paint::brush::{BrushSettings, TipShape};
use photocraft_paint::{grid_center, grid_square};

use crate::canvas::ViewXform;

/// Closed rings of pixel corners, as `photocraft_paint::outline::aliased_outline` traces them.
type Rings = Vec<Vec<[i32; 2]>>;

/// Under this many points across, the cursor is too small to show where it is, so the hotspot
/// crosshair joins it.
const SMALL: f32 = 6.0;

#[derive(Clone)]
struct Cached {
    key: Key,
    rings: Option<Arc<Rings>>,
}

/// The brush fields an aliased outline depends on.
#[derive(Clone)]
struct Key {
    tip: TipShape,
    size: f32,
    angle: f32,
    roundness: f32,
    flip_x: bool,
    flip_y: bool,
    square: bool,
}

impl Key {
    fn of(b: &BrushSettings) -> Self {
        Self { tip: b.tip.clone(), size: b.size, angle: b.angle, roundness: b.roundness, flip_x: b.flip_x, flip_y: b.flip_y, square: b.square }
    }
    fn matches(&self, b: &BrushSettings) -> bool {
        crate::brush_cursor::same_tip(&self.tip, &b.tip)
            && self.size == b.size
            && self.angle == b.angle
            && self.roundness == b.roundness
            && self.flip_x == b.flip_x
            && self.flip_y == b.flip_y
            && self.square == b.square
    }
}

/// The outline's two lines on screen, through the corners of each ring: the dark one half a
/// physical pixel outside the filled pixels, the light one half a pixel inside.
struct Lines {
    dark: Vec<Vec<Pos2>>,
    light: Vec<Vec<Pos2>>,
    /// The rings' larger side on screen, in points.
    size: f32,
}

/// The Pencil's cursor with the pointer at `at` (screen): the outline of the pixels a dab there
/// fills where [`draw`] can show them, else the tip's smooth outline. From 100 % up that is
/// centred where the dab lands, as the outline is; below 100 % it is on the pointer, which the
/// dab lands within half a physical pixel of. `block` is the Block-mode Eraser's square in
/// document pixels; `centre` is Preferences › Cursors › Show Crosshair in Brush Tip.
pub(crate) fn cursor(painter: &Painter, xf: &ViewXform, at: Pos2, brush: &BrushSettings, block: Option<f32>, centre: bool) -> CursorIcon {
    let doc = xf.to_doc(at);
    if let Some(size) = draw(painter, xf, doc, brush, block) {
        // Too small to see where it is: the hotspot as well.
        if centre || size < SMALL {
            crate::tool_cursor::crosshair(painter, at, 4.0, 0.0);
        }
        return CursorIcon::None;
    }
    // Where a document pixel is smaller than a physical one, the tip's smooth outline is as
    // exact: all of a round tip, or a sampled tip's 50 % contour, where the aliased dab cuts them.
    // A tip too big to trace is off by under a pixel at its edge.
    let radius = (brush.size / 2.0 * xf.zoom).max(1.0);
    let axes = crate::brush_cursor::tip_axes(xf, brush.size);
    let (normal, mark) = (!matches!(brush.tip, TipShape::Round), centre || 2.0 * radius < SMALL);
    // From 100 % up the dab, on the pixel grid, can be half a document pixel each way from the
    // pointer, half a physical pixel or more (4 points at 800 %): the outline goes where it lands,
    // as the square's circle did. It is painted, since an OS cursor bitmap can only sit on the
    // pointer; the hotspot crosshair stays there.
    let (cx, cy) = grid_center(doc[0], doc[1], brush.size);
    let dab = xf.to_screen(cx, cy);
    if whole_pixels(xf.zoom, pixels_per_point(painter)) && dab.is_finite() {
        crate::brush_cursor::paint(painter, dab, brush, radius, axes, normal);
        if mark {
            crate::tool_cursor::crosshair(painter, at, 4.0, 0.0);
        }
        return CursorIcon::None;
    }
    crate::brush_cursor::cursor(painter, at, brush, radius, axes, normal, mark)
}

/// Physical pixels per point, or 1 when the context's figure is unusable.
fn pixels_per_point(painter: &Painter) -> f32 {
    let ppp = painter.ctx().pixels_per_point();
    if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 }
}

/// Whether a document pixel `zoom` points across spans at least one physical pixel (`ppp` per
/// point), as from 100 % up. At 100 % `zoom` is `1 / ppp`, and `(1 / ppp) · ppp` rounds to just
/// under 1 in f32 at many UI scales (85 % on a 1× or 2× display, 110 % on a 2.25× one).
fn whole_pixels(zoom: f32, ppp: f32) -> bool {
    zoom * ppp >= 1.0 - 1e-4
}

/// Draws the outline of the pixels a dab centred at document point `at` fills: `brush`'s, or with
/// `block` the Block-mode Eraser's square of that many pixels (16 screen pixels at any zoom,
/// #2770). Returns its larger side on screen; `None`, drawing nothing, where a document pixel is
/// smaller than a physical one either way (below 100 %, or at 100 % with a Pixel Aspect Ratio
/// below 1), for a tip too big or ragged to trace, or off the map.
fn draw(painter: &Painter, xf: &ViewXform, at: [f64; 2], brush: &BrushSettings, block: Option<f32>) -> Option<f32> {
    let ppp = pixels_per_point(painter);
    let rings: Vec<Vec<[f32; 2]>> = match block {
        Some(size) => {
            let [x0, y0, x1, y1] = grid_square(at[0], at[1], size).map(|v| v as f32);
            vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]]
        }
        None => {
            if !whole_pixels(xf.zoom.min(xf.zoom_x()), ppp) {
                return None;
            }
            // Traced once per brush for a dab at the origin: one at `at` fills the same pixels
            // moved by the whole pixels between their grid centres.
            let rings = outline(painter.ctx(), brush)?;
            let (cx, cy) = grid_center(at[0], at[1], brush.size);
            let (ox, oy) = grid_center(0.0, 0.0, brush.size);
            let (dx, dy) = (cx - ox, cy - oy);
            rings.iter().map(|r| r.iter().map(|&[x, y]| [x as f32 + dx, y as f32 + dy]).collect()).collect()
        }
    };
    let lines = screen_lines(xf, &rings, ppp)?;
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    let px = 1.0 / ppp;
    // The dark lines first: where two filled pixels touch at a corner, the light ones stay on top.
    painter.extend(lines.dark.into_iter().map(|l| Shape::closed_line(l, Stroke::new(px, dark))));
    painter.extend(lines.light.into_iter().map(|l| Shape::closed_line(l, Stroke::new(px, light))));
    Some(lines.size)
}

/// `aliased_outline` of `brush`, kept across frames: it changes only with the brush, and a sampled
/// tip builds its mipmaps to rasterise.
fn outline(ctx: &egui::Context, brush: &BrushSettings) -> Option<Arc<Rings>> {
    let id = egui::Id::unique("photocraft-pencil-outline");
    if let Some(c) = ctx.data(|d| d.get_temp::<Cached>(id))
        && c.key.matches(brush)
    {
        return c.rings;
    }
    let rings = photocraft_paint::outline::aliased_outline(brush).map(Arc::new);
    ctx.data_mut(|d| d.insert_temp(id, Cached { key: Key::of(brush), rings: rings.clone() }));
    rings
}

/// Where the outline of `rings` (pixel corners in document pixels, the filled pixels on the right
/// of each edge) goes on screen, as the Pencil's square always went: each corner moves half a
/// physical pixel out of the pixels along both its edges for the dark line, and half a pixel in
/// for the light one, so 1-pixel strokes hug the pixels from either side. On a view that isn't
/// turned the corners first snap to physical pixels (`ppp` per point), for crisp lines. `None`
/// when a corner doesn't land on a finite point.
fn screen_lines(xf: &ViewXform, rings: &[Vec<[f32; 2]>], ppp: f32) -> Option<Lines> {
    let aligned = xf.rotation.rem_euclid(90.0) == 0.0;
    let snap = |p: Pos2| if aligned { pos2((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp) } else { p };
    // Half a physical pixel out of the edge from `a` to `b`: its filled pixels are on its right,
    // so out is to its left, turned, flipped and stretched with the view.
    let out = |a: [f32; 2], b: [f32; 2]| {
        let sign = |v: f32| f32::from(v > 0.0) - f32::from(v < 0.0);
        xf.map_vec(vec2(sign(b[1] - a[1]), -sign(b[0] - a[0]))).normalized() * (0.5 / ppp)
    };
    let (mut dark, mut light) = (Vec::with_capacity(rings.len()), Vec::with_capacity(rings.len()));
    let (mut min, mut max) = (pos2(f32::INFINITY, f32::INFINITY), pos2(f32::NEG_INFINITY, f32::NEG_INFINITY));
    for ring in rings {
        let n = ring.len();
        let (mut outer, mut inner) = (Vec::with_capacity(n), Vec::with_capacity(n));
        for (i, &c) in ring.iter().enumerate() {
            let (Some(&prev), Some(&next)) = (ring.get((i + n - 1) % n), ring.get((i + 1) % n)) else { continue };
            let p = snap(xf.to_screen(c[0], c[1]));
            let o = out(prev, c) + out(c, next);
            (min, max) = (min.min(p), max.max(p));
            outer.push(p + o);
            inner.push(p - o);
        }
        if !outer.iter().chain(&inner).all(|p| p.is_finite()) {
            return None;
        }
        dark.push(outer);
        light.push(inner);
    }
    let size = (max - min).max_elem();
    size.is_finite().then_some(Lines { dark, light, size })
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2};

    use super::*;

    fn view(zoom: f32, rotation: f32, flip: bool, aspect: f32) -> ViewXform {
        ViewXform { rect: Rect::from_min_size(pos2(0.3, 0.7), egui::vec2(800.0, 600.0)), zoom, center: [50.2, 37.9], flip, rotation, aspect }
    }

    /// A square of pixels is drawn where the old square cursor was (`rect_stroke` outside and
    /// inside): a dark physical pixel just outside its edges and a light one just inside, with its
    /// corners on physical pixels at any zoom and density. A turned view turns the outline with it.
    #[test]
    fn the_outline_hugs_the_pixels_on_physical_pixels_and_turns_with_the_view() {
        let square = vec![vec![[8.0, 4.0], [12.0, 4.0], [12.0, 8.0], [8.0, 8.0]]];
        for (zoom, ppp) in [(8.0, 1.0), (3.3, 2.0), (1.0, 1.0), (13.7, 1.5)] {
            let xf = view(zoom, 0.0, false, 1.0);
            let lines = screen_lines(&xf, &square, ppp).unwrap();
            let snap = |v: f32| (v * ppp).round() / ppp;
            let (a, b) = (xf.to_screen(8.0, 4.0), xf.to_screen(12.0, 8.0));
            let (x0, y0, x1, y1, h) = (snap(a.x), snap(a.y), snap(b.x), snap(b.y), 0.5 / ppp);
            assert_eq!(lines.dark, [[pos2(x0 - h, y0 - h), pos2(x1 + h, y0 - h), pos2(x1 + h, y1 + h), pos2(x0 - h, y1 + h)]], "zoom {zoom}, ppp {ppp}");
            assert_eq!(lines.light, [[pos2(x0 + h, y0 + h), pos2(x1 - h, y0 + h), pos2(x1 - h, y1 - h), pos2(x0 + h, y1 - h)]], "zoom {zoom}, ppp {ppp}");
            assert!(((x1 - x0) - 4.0 * zoom).abs() <= 1.0 / ppp + 1e-3, "zoom {zoom}, ppp {ppp}");
            assert_eq!(lines.size, (x1 - x0).max(y1 - y0));
        }
        // Turned 30° and flipped: each corner of the dark line is half a pixel out along both
        // edges (√½ pixel diagonally), outside the square; the light line's inside it.
        let xf = view(8.0, 30.0, true, 1.0);
        let lines = screen_lines(&xf, &square, 1.0).unwrap();
        let centre = xf.to_screen(10.0, 6.0);
        for (i, [x, y]) in square[0].iter().copied().enumerate() {
            let p = xf.to_screen(x, y);
            let (d, l): (Vec2, Vec2) = (lines.dark[0][i] - p, lines.light[0][i] - p);
            assert!((d.length() - 0.5f32.hypot(0.5)).abs() < 1e-4 && (d + l).length() < 1e-4, "corner {i}: {d:?} {l:?}");
            assert!((lines.dark[0][i] - centre).length() > (p - centre).length(), "corner {i}");
        }
        // Hostile views and corners draw nothing.
        for xf in [view(f32::NAN, 0.0, false, 1.0), view(f32::INFINITY, 0.0, false, 1.0), view(8.0, f32::NAN, false, 1.0)] {
            assert!(screen_lines(&xf, &square, 1.0).is_none());
        }
        assert!(screen_lines(&view(8.0, 0.0, false, 1.0), &[vec![[f32::NAN, 4.0], [12.0, 4.0], [12.0, 8.0], [8.0, 8.0]]], 1.0).is_none());
        assert!(screen_lines(&view(8.0, 0.0, false, 1.0), &[], 1.0).is_none());
    }

    /// At every corner of every ring the dark line's point lands in the empty pixel outside it and
    /// the light line's in the filled pixel inside, on views turned by quarter turns (where the
    /// corners snap to physical pixels) and in between, mirrored, stretched by Pixel Aspect Ratio
    /// Correction, at several zooms and display densities.
    #[test]
    fn the_two_lines_straddle_every_corner_on_any_view() {
        // At document (30, 20): an L of pixels, with a concave corner, and a 3×3 square with a
        // hole.
        let pixels = [(0, 0), (0, 1), (0, 2), (1, 2), (2, 2), (5, 0), (6, 0), (7, 0), (5, 1), (7, 1), (5, 2), (6, 2), (7, 2)];
        let filled = |[x, y]: [f64; 2]| pixels.contains(&(x.floor() as i64 - 30, y.floor() as i64 - 20));
        // Their outline: clockwise outer rings, the hole counter-clockwise.
        let rings: Vec<Vec<[f32; 2]>> = vec![
            vec![[30.0, 20.0], [31.0, 20.0], [31.0, 22.0], [33.0, 22.0], [33.0, 23.0], [30.0, 23.0]],
            vec![[35.0, 20.0], [38.0, 20.0], [38.0, 23.0], [35.0, 23.0]],
            vec![[36.0, 21.0], [36.0, 22.0], [37.0, 22.0], [37.0, 21.0]],
        ];
        let mut fails = Vec::new();
        for zoom in [2.0f32, 8.0, 13.7] {
            for ppp in [1.0f32, 1.5, 2.0] {
                for rotation in [0.0f32, 90.0, 180.0, 270.0, -90.0, 30.0, 45.0] {
                    for flip in [false, true] {
                        for aspect in [1.0f32, 2.0, 0.75] {
                            let what = format!("zoom {zoom}, ppp {ppp}, {rotation}°, flipped {flip}, aspect {aspect}");
                            let xf = view(zoom, rotation, flip, aspect);
                            let Some(lines) = screen_lines(&xf, &rings, ppp) else {
                                fails.push(format!("{what}: nothing drawn"));
                                continue;
                            };
                            for (r, ring) in rings.iter().enumerate() {
                                for (i, c) in ring.iter().enumerate() {
                                    let (d, l) = (lines.dark[r][i], lines.light[r][i]);
                                    // Snapping to physical pixels can move the corner by half a
                                    // pixel either way, so judge the two points from where it went.
                                    let mid = xf.to_doc(pos2((d.x + l.x) / 2.0, (d.y + l.y) / 2.0));
                                    let shift = [f64::from(c[0]) - mid[0], f64::from(c[1]) - mid[1]];
                                    let slack = f64::from(0.75 / ppp / (zoom * aspect.min(1.0)));
                                    let filled_at = |p: Pos2| {
                                        let [x, y] = xf.to_doc(p);
                                        filled([x + shift[0], y + shift[1]])
                                    };
                                    let (moved, dark, light) = (shift[0].hypot(shift[1]), filled_at(d), filled_at(l));
                                    if moved > slack || dark || !light {
                                        let kind = |f: bool| if f { "filled" } else { "empty" };
                                        fails.push(format!(
                                            "{what}: corner {c:?} moved {moved:.3} px, dark point in a {} pixel, light point in a {} pixel",
                                            kind(dark),
                                            kind(light),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(fails.is_empty(), "{} corners:\n{}", fails.len(), fails.join("\n"));
    }

    /// From 100 % up the outline is drawn (the Block square at any zoom); below it, for a tip
    /// too big to trace or a pointer off the map the caller keeps the smooth outline. The traced
    /// outline is kept until the brush changes.
    #[test]
    fn below_100_percent_or_for_huge_tips_the_smooth_outline_stays() {
        let ctx = egui::Context::default();
        let brush = BrushSettings { size: 9.0, ..Default::default() };
        let huge = BrushSettings { size: 5000.0, ..Default::default() };
        let mut drawn = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let p = ui.painter();
            for zoom in [0.5, 0.99, 1.0, 8.0] {
                drawn.push(draw(p, &view(zoom, 0.0, false, 1.0), [20.3, 15.6], &brush, None).is_some());
            }
            drawn.push(draw(p, &view(0.25, 0.0, false, 1.0), [20.3, 15.6], &brush, Some(64.0)).is_some());
            drawn.push(draw(p, &view(8.0, 0.0, false, 1.0), [20.3, 15.6], &huge, None).is_some());
            drawn.push(draw(p, &view(8.0, 0.0, false, 1.0), [f64::NAN, 15.6], &brush, None).is_some());
        });
        // No renderer takes the font atlas here.
        output.textures_delta.clear();
        assert_eq!(drawn, [false, false, true, true, true, false, false]);
        let first = outline(&ctx, &brush).unwrap();
        assert!(Arc::ptr_eq(&first, &outline(&ctx, &brush).unwrap()));
        assert!(!Arc::ptr_eq(&first, &outline(&ctx, &BrushSettings { size: 10.0, ..brush }).unwrap()));
    }

    /// Below 100 % a Pencil under 6 points across still shows its hotspot, as its cursor always
    /// has when too small to see where it is: the smooth circle with the crosshair in it. A
    /// bigger one is the plain circle unless Show Crosshair in Brush Tip asks for the crosshair.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn below_100_percent_a_tiny_pencil_still_shows_its_hotspot() {
        // The OS cursor bitmap a pass that runs `f` shows.
        let bitmap = |f: &dyn Fn(&Painter)| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| f(ui.painter()));
            // No renderer takes the font atlas here.
            output.textures_delta.clear();
            output.platform_output.cursor_image.map(|i| (i.size, i.rgba))
        };
        let (xf, at) = (view(0.5, 0.0, false, 1.0), pos2(100.0, 100.0));
        // At 50 % an 11 px tip is 5.5 points across and a 12 px one 6.
        for (size, centre, crosshair) in [(1.0, false, true), (11.0, false, true), (12.0, false, false), (40.0, false, false), (40.0, true, true)] {
            let brush = BrushSettings { size, ..Default::default() };
            let radius = (size / 2.0 * 0.5).max(1.0);
            let want = bitmap(&|p| {
                crate::tool_cursor::circle(p, at, radius, crosshair);
            });
            assert!(want.is_some());
            let got = bitmap(&|p| {
                cursor(p, &xf, at, &brush, None, centre);
            });
            assert_eq!(got, want, "{size} px, Show Crosshair in Brush Tip {centre}");
        }
    }

    /// At 100 % a document pixel is one physical pixel at any UI scale, so the outline is drawn.
    /// The zoom in points is `1 / ppp` there, and `(1 / ppp) · ppp` rounds to just under 1 in f32
    /// at many scales (85 % on a 1× or 2× display, 110 % on a 2.25× one): those showed the smooth
    /// circle, as below 100 %.
    #[test]
    fn at_100_percent_the_outline_is_drawn_at_any_ui_scale() {
        let brush = BrushSettings { size: 20.0, ..Default::default() };
        let mut fails = Vec::new();
        for ppp in [0.85f32, 1.7, 2.25 * 1.1] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            // The new scale applies from the next pass.
            ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
            let mut scale = 0.0;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                scale = ui.ctx().pixels_per_point();
                let xf = view(1.0 / ppp, 0.0, false, 1.0);
                cursor(ui.painter(), &xf, xf.to_screen(20.3, 15.6), &brush, None, false);
            });
            // No renderer takes the font atlas here.
            output.textures_delta.clear();
            assert_eq!(scale, ppp);
            let lines = output.shapes.iter().filter(|s| matches!(&s.shape, Shape::Path(p) if p.closed)).count();
            let bitmap = output.platform_output.cursor_image.is_some();
            if lines != 2 || bitmap {
                fails.push(format!("ppp {ppp}: {lines} outline lines drawn, the smooth outline's OS cursor bitmap {bitmap}"));
            }
        }
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

    /// From 100 % up the dab lands on the pixel grid, up to half a pixel each way from the pointer
    /// (4 points at 800 %). A tip too big to trace keeps its smooth outline, centred where the dab
    /// lands, as the old square's circle was: two pointers over one dab draw what a pointer on its
    /// centre draws, a round tip's circle around that centre. None of it is an OS cursor bitmap,
    /// which can only sit on the pointer; the hotspot crosshair still does.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn from_100_percent_an_untraceable_tip_is_centred_on_its_dab() {
        let disc = photocraft_paint::GrayTile::from_fn(64, 64, |x, y| if (x as f32 - 31.5).hypot(y as f32 - 31.5) < 30.0 { 1.0 } else { 0.0 });
        // Round tips over the 508 px cap (at 300 % and 100 % their circle fits an OS cursor
        // bitmap), a squashed and turned one, and a sampled tip over about 359 px.
        let cases = [
            (BrushSettings { size: 600.0, ..Default::default() }, 8.0),
            (BrushSettings { size: 520.0, ..Default::default() }, 3.0),
            (BrushSettings { size: 600.0, ..Default::default() }, 1.0),
            (BrushSettings { size: 600.0, roundness: 0.5, angle: 30.0, ..Default::default() }, 8.0),
            (BrushSettings { size: 400.0, tip: TipShape::Sampled(disc), ..Default::default() }, 2.0),
        ];
        // What a pass drawing the cursor with the pointer at `at` paints, and its OS cursor bitmap.
        let pass = |xf: &ViewXform, at: Pos2, brush: &BrushSettings, centre: bool| {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                cursor(ui.painter(), xf, at, brush, None, centre);
            });
            // No renderer takes the font atlas here.
            output.textures_delta.clear();
            (output.shapes.into_iter().map(|s| s.shape).collect::<Vec<_>>(), output.platform_output.cursor_image.map(|i| (i.size, i.rgba)))
        };
        for (brush, zoom) in &cases {
            let what = format!("{} px, roundness {}, sampled {}, at {} %", brush.size, brush.roundness, brush.tip != TipShape::Round, zoom * 100.0);
            let xf = view(*zoom, 0.0, false, 1.0);
            // An even size: the dab over both points is centred on the pixel corner (20, 16).
            let dab = xf.to_screen(20.0, 16.0);
            let (want, _) = pass(&xf, dab, brush, false);
            for [x, y] in [[20.1, 15.6], [20.4, 15.9]] {
                let (got, bitmap) = pass(&xf, xf.to_screen(x, y), brush, false);
                for s in &got {
                    if let Shape::Circle(c) = s {
                        assert!((c.center - dab).length() < 1e-3, "{what}, pointer at ({x}, {y}): a circle around {:?}, the dab's centre is {dab:?}", c.center);
                    }
                }
                assert!(
                    !got.is_empty() && got == want,
                    "{what}, pointer at ({x}, {y}): {} shapes painted, {} with the pointer on the dab",
                    got.len(),
                    want.len()
                );
                assert!(bitmap.is_none(), "{what}, pointer at ({x}, {y}): an OS cursor bitmap");
            }
        }
        // Show Crosshair in Brush Tip: the outline is the dab's, the crosshair the pointer's.
        let (brush, zoom) = &cases[0];
        let xf = view(*zoom, 0.0, false, 1.0);
        let at = xf.to_screen(20.3, 15.6);
        let crosshair = {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                crate::tool_cursor::crosshair(ui.painter(), at, 4.0, 0.0);
            });
            output.textures_delta.clear();
            output.platform_output.cursor_image.map(|i| (i.size, i.rgba))
        };
        let (got, bitmap) = pass(&xf, at, brush, true);
        assert!(crosshair.is_some() && bitmap == crosshair, "the hotspot crosshair");
        assert_eq!(got, pass(&xf, xf.to_screen(20.0, 16.0), brush, false).0, "the outline with the crosshair");
    }

    /// The Block-mode Eraser's square is the engine's `16 / zoom` pixels at every zoom (#2770),
    /// including those where the Pencil keeps the smooth outline.
    #[test]
    fn the_block_square_is_drawn_at_every_zoom() {
        let ctx = egui::Context::default();
        let brush = BrushSettings { size: 9.0, ..Default::default() };
        let mut drawn = Vec::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for zoom in [0.01f32, 0.25, 0.33, 0.5, 1.0, 3.0, 16.0, 64.0] {
                let size = photocraft_engine::brush_cmds::block_size(f64::from(zoom));
                drawn.push((zoom, size, draw(ui.painter(), &view(zoom, 0.0, false, 1.0), [20.3, 15.6], &brush, Some(size))));
            }
        });
        // No renderer takes the font atlas here.
        output.textures_delta.clear();
        for (zoom, size, side) in drawn {
            let side = side.unwrap_or_else(|| panic!("zoom {zoom}: no Block square"));
            assert!((side - size * zoom).abs() <= 1.0, "zoom {zoom}: {side} points for a {size} px square");
        }
    }

    /// A stored tip (#1843) paints its preview until the preset store loads the full bitmap,
    /// which can be after the cursor first traced it: the outline is traced again then.
    #[test]
    fn a_stored_tip_is_traced_again_once_its_full_bitmap_loads() {
        let preview = photocraft_paint::GrayTile::from_fn(12, 12, |_, _| 1.0);
        let ring = photocraft_paint::GrayTile::from_fn(48, 48, |x, y| {
            let d = (x as f32 - 23.5).hypot(y as f32 - 23.5) / 24.0;
            if (0.5..0.95).contains(&d) { 1.0 } else { 0.0 }
        });
        let unloaded = photocraft_paint::StoredTile { key: "ring".into(), width: 48, height: 48, preview: Arc::new(preview), full: None };
        let loaded = photocraft_paint::StoredTile { full: Some(Arc::new(ring)), ..unloaded.clone() };
        let before = BrushSettings { size: 24.0, tip: TipShape::Stored(unloaded), ..Default::default() };
        let after = BrushSettings { tip: TipShape::Stored(loaded), ..before.clone() };
        assert_eq!(before.tip, after.tip, "stored tips compare by key");
        let traced = photocraft_paint::outline::aliased_outline(&after);
        assert!(traced.is_some() && photocraft_paint::outline::aliased_outline(&before) != traced);
        let ctx = egui::Context::default();
        let _ = outline(&ctx, &before);
        assert_eq!(outline(&ctx, &after).map(|r| (*r).clone()), traced, "the outline is still the preview's");
    }
}
