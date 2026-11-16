//! Where a linked Pattern Overlay starts tiling (#14960).
//!
//! The measured reference application these tests read: a 400 x 300 px, 72 ppi, RGB 8 bit
//! document with a white Background and a pixel layer "Shape" holding an orange (255,128,0)
//! rectangle at x 100..299, y 75..224, and a 20 x 20 pattern whose quadrants are red (top left),
//! green (top right), blue (bottom left) and black (bottom right), with one white pixel at tile
//! (0, 0) marking the tile origin. The Pattern Overlay is Normal, 100 %, angle 0, scale 100 %,
//! phase 0 / 0.
//!
//! Expected rows and columns read the way the measurement writes them: one letter per pixel (W
//! white, R red, G green, B blue, K black), equal runs counted after their letter and the count
//! left out for a single pixel. `B×10 K×10 B×6` is therefore ten blue, ten black, six blue.

use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Effect, FxCommon, FxPaint, Layer, Pattern, StrokeFx, StrokePosition};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

use crate::{Buffer, render};

/// The measured orange of the "Shape" layer as a document sample.
const ORANGE: [f32; 4] = [1.0, 128.0 / 255.0, 0.0, 1.0];
/// The measured rectangle of the never-moved "Shape" layer: x 100..299, y 75..224.
const SHAPE: Rect = Rect::new(100, 75, 300, 225);

/// The measured 20 x 20 pattern: four quadrants and the white marker pixel at tile (0, 0).
fn quad_pattern() -> Pattern {
    let mut tile = Surface::new(PixelFormat::RGBA8);
    tile.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    tile.fill_rect(Rect::new(10, 0, 20, 10), &[0.0, 1.0, 0.0, 1.0]);
    tile.fill_rect(Rect::new(0, 10, 10, 20), &[0.0, 0.0, 1.0, 1.0]);
    tile.fill_rect(Rect::new(10, 10, 20, 20), &[0.0, 0.0, 0.0, 1.0]);
    tile.fill_rect(Rect::new(0, 0, 1, 1), &[1.0, 1.0, 1.0, 1.0]);
    Pattern::new("quadrants", tile, 20, 20)
}

/// A Pattern Overlay of [`quad_pattern`] with the measured settings, `link` and `phase` as asked.
fn overlay(pat: &Pattern, link: bool, phase: (f32, f32)) -> Effect {
    Effect::PatternOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), name: pat.name.clone(), id: pat.id.clone(), scale: 1.0, angle: 0.0, link, phase }
}

/// The "Shape" layer: a raster layer holding the orange rectangle at `rect`.
fn shape(rect: Rect) -> Layer {
    let mut l = Layer::raster("Shape", PixelFormat::RGBA8);
    l.surface_mut().unwrap().fill_rect(rect, &ORANGE);
    l
}

/// The measured document with `layer` (its effects set to one linked Pattern Overlay with
/// `reference` as its stored effects reference point) above the white Background.
fn overlay_doc(mut layer: Layer, reference: Option<(f64, f64)>, link: bool, phase: (f32, f32)) -> Document {
    let pat = quad_pattern();
    layer.effects.items = vec![overlay(&pat, link, phase)];
    layer.effects.reference = reference;
    let mut d = Document::with_background("Pattern overlay origin", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    d.patterns.push(pat);
    d.layers.push(layer);
    d
}

/// The flattened picture of the whole document.
fn flat(doc: &Document) -> Buffer {
    render(doc, doc.bounds())
}

fn pixel(buf: &Buffer, x: i32, y: i32) -> [f32; 4] {
    let i = (y - buf.rect.y0) as usize * buf.rect.width() as usize + (x - buf.rect.x0) as usize;
    *buf.px.get(i).expect("the point is inside the rendered region")
}

/// The colour of a document pixel as its letter; anything else as `?`, so a wrong expectation
/// shows what the compositor actually painted.
fn letter(p: [f32; 4]) -> char {
    let q = |v: f32| v > 0.5;
    match (q(p[0]), q(p[1]), q(p[2])) {
        (true, false, false) => 'R',
        (false, true, false) => 'G',
        (false, false, true) => 'B',
        (false, false, false) => 'K',
        (true, true, true) => 'W',
        _ => '?',
    }
}

/// A line of letters as the measurement writes it: each run counted after its letter, the count
/// left out for a single pixel.
fn runs(line: &[char]) -> String {
    let mut out = String::new();
    for (i, c) in line.iter().enumerate() {
        if i + 1 < line.len() && line[i + 1] == *c {
            continue;
        }
        let start = line[..=i].iter().rposition(|p| *p != *c).map_or(0, |p| p + 1);
        if !out.is_empty() {
            out.push(' ');
        }
        out.push(*c);
        if i > start {
            out.push('×');
            out.push_str(&(i + 1 - start).to_string());
        }
    }
    out
}

/// The letters along row `y` from `from` to `to`, both inside the document.
fn row(buf: &Buffer, y: i32, from: i32, to: i32) -> String {
    runs(&(from..=to).map(|x| letter(pixel(buf, x, y))).collect::<Vec<char>>())
}

/// The letters down column `x` from `from` to `to`, both inside the document.
fn column(buf: &Buffer, x: i32, from: i32, to: i32) -> String {
    runs(&(from..=to).map(|y| letter(pixel(buf, x, y))).collect::<Vec<char>>())
}

/// Done when 1: on the test document a new linked overlay (no stored reference point, the layer
/// never moved) tiles from the document origin, not from the layer's corner at (100, 75).
#[test]
fn a_linked_overlay_without_a_reference_point_tiles_from_the_document_origin() {
    let d = overlay_doc(shape(SHAPE), None, true, (0.0, 0.0));
    let b = flat(&d);
    assert_eq!(letter(pixel(&b, 100, 80)), 'W', "tile marker at (100, 80): {:?}", pixel(&b, 100, 80));
    assert_eq!(letter(pixel(&b, 120, 100)), 'W', "tile marker at (120, 100): {:?}", pixel(&b, 120, 100));
    // y 70..74 are outside the layer and show the white backdrop; y 75..79 are the lower half of
    // the tile that began at y 60, and a new tile starts at y 80.
    assert_eq!(column(&b, 110, 70, 100), "W×5 K×5 G×10 K×10 G", "column x 110 from y 70");
    assert_eq!(row(&b, 150, 100, 125), "B×10 K×10 B×6", "row y 150 from x 100");
}

/// The layer of `d` the test placed above the Background, with `phase` as the move phase its Move
/// commands recorded (`Effects::move_phase`).
fn with_move_phase(mut d: Document, phase: (f64, f64)) -> Document {
    d.layers.last_mut().expect("the test layer").effects.move_phase = Some(phase);
    d
}

/// Done when 2 (the compositor half): the state a Move leaves behind, a move phase holding the
/// layer's +5,+3 and the layer's rectangle moved with it. The grid follows the layer.
#[test]
fn the_grid_anchors_at_the_move_phase_a_layer_move_records() {
    let d = with_move_phase(overlay_doc(shape(Rect::new(105, 78, 305, 228)), None, true, (0.0, 0.0)), (5.0, 3.0));
    let b = flat(&d);
    // x 100..104 are outside the moved layer and show the white backdrop.
    assert_eq!(row(&b, 150, 100, 130), "W×5 R×10 G×10 R×6", "row y 150 after a +5,+3 move");
    assert_eq!(letter(pixel(&b, 105, 83)), 'W', "tile marker after the move: {:?}", pixel(&b, 105, 83));
    assert_eq!(letter(pixel(&b, 125, 103)), 'W', "the next marker: {:?}", pixel(&b, 125, 103));
}

/// Done when 3 (the compositor half): a layer moved +7,+4 before the overlay was added leaves the
/// grid on that phase, so its tile edges fall on x 107, 127, ... and not on the layer's corner.
#[test]
fn a_move_phase_of_the_layer_moves_tiles_from_the_document_origin_plus_that_move() {
    let d = with_move_phase(overlay_doc(shape(Rect::new(107, 79, 307, 229)), None, true, (0.0, 0.0)), (7.0, 4.0));
    let b = flat(&d);
    assert_eq!(row(&b, 150, 107, 132), "R×10 G×10 R×6", "row y 150 from the moved layer's left edge");
    assert_eq!(letter(pixel(&b, 107, 144)), 'W', "tile marker at (107, 144): {:?}", pixel(&b, 107, 144));
    assert_eq!(letter(pixel(&b, 127, 164)), 'W', "tile marker at (127, 164): {:?}", pixel(&b, 127, 164));
}

/// Done when 4: with the overlay on a group, the pattern stays on the document grid while a child
/// layer moves: the group's own grid does not follow its content's bounds.
#[test]
fn a_group_overlay_stays_on_the_document_grid_when_a_child_moves() {
    let pat = quad_pattern();
    let mut group = Layer::group("Group", vec![shape(Rect::new(107, 79, 307, 229))]);
    group.effects.items = vec![overlay(&pat, true, (0.0, 0.0))];
    let mut d = Document::with_background("Pattern overlay origin", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    d.patterns.push(pat);
    d.layers.push(group);
    let b = flat(&d);
    assert_eq!(row(&b, 150, 107, 130), "B×3 K×10 B×10 K", "row y 150 from x 107 on the group");
    assert_eq!(letter(pixel(&b, 120, 100)), 'W', "the grid's marker stays on the document grid: {:?}", pixel(&b, 120, 100));
}

/// Done when 5 (the compositor half): an overlay read from a file keeps its stored reference point
/// and its phase, and the two add up to where the grid starts. Not the document origin.
#[test]
fn a_stored_reference_point_and_its_phase_keep_their_anchor() {
    let d = overlay_doc(shape(SHAPE), Some((5.0, 3.0)), true, (5.0, 3.0));
    let b = flat(&d);
    assert_eq!(row(&b, 150, 100, 125), "G×10 R×10 G×6", "row y 150 with reference (5, 3) and phase (5, 3)");
    assert_eq!(letter(pixel(&b, 110, 106)), 'W', "tile marker at (110, 106): {:?}", pixel(&b, 110, 106));
}

/// Link with Layer off already matched the reference application: the grid sits on the document
/// origin and neither a stored reference point nor the layer's own position moves it.
#[test]
fn an_unlinked_overlay_tiles_from_the_document_origin_whatever_the_layer_holds() {
    let d = overlay_doc(shape(Rect::new(105, 78, 305, 228)), Some((5.0, 3.0)), false, (0.0, 0.0));
    let b = flat(&d);
    assert_eq!(row(&b, 150, 105, 130), "B×5 K×10 B×10 K", "row y 150 of an unlinked overlay");
    assert_eq!(letter(pixel(&b, 120, 100)), 'W', "tile marker at (120, 100): {:?}", pixel(&b, 120, 100));
}

/// Every other pin above measures Scale 100 % and Angle 0, where the anchor only offsets the grid.
/// At Scale 200 % the anchor is also the centre the pattern is scaled about, so taking the anchor
/// from the layer's corner and putting it on the document origin moves the whole placement. The
/// second document below puts that corner in as a stored reference point, which is where
/// `upstream/main` anchored this layer: at this scale the two anchors give different pixels, so the
/// pin really measures the placement centre (which it also is for a non-zero Angle, not measured
/// here).
#[test]
fn a_linked_overlay_at_scale_200_is_scaled_about_its_anchor() {
    let with_anchor = |reference: Option<(f64, f64)>| {
        let pat = quad_pattern();
        let mut l = shape(SHAPE);
        l.effects.items = vec![Effect::PatternOverlay {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            name: pat.name.clone(),
            id: pat.id.clone(),
            scale: 2.0,
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        }];
        l.effects.reference = reference;
        let mut d = Document::with_background("Pattern overlay origin", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        d.patterns.push(pat);
        d.layers.push(l);
        flat(&d)
    };
    let doc_origin = with_anchor(None);
    let corner = with_anchor(Some((100.0, 75.0)));
    // Doubled tiles: on the document grid a tile corner falls every 40 px, at (120, 80) among them,
    // while the corner anchor has plain green there.
    assert_eq!(letter(pixel(&doc_origin, 120, 80)), 'W', "tile marker of the scaled document grid at (120, 80): {:?}", pixel(&doc_origin, 120, 80));
    assert_eq!(letter(pixel(&corner, 120, 80)), 'G', "the same point on the corner-anchored placement: {:?}", pixel(&corner, 120, 80));
    // Down at y 223 the scaled document grid is in its black quadrant where the corner-anchored
    // placement has its blue one.
    assert_eq!(letter(pixel(&doc_origin, 100, 223)), 'K', "(100, 223) on the scaled document grid: {:?}", pixel(&doc_origin, 100, 223));
    assert_eq!(letter(pixel(&corner, 100, 223)), 'B', "the same point on the corner-anchored placement: {:?}", pixel(&corner, 100, 223));
}

/// The other linked patterns are not part of #14960: a pattern stroke with no stored reference
/// point still tiles from the layer's corner. y 76 is the top half of the tile that starts at the
/// corner y 75 and the lower half of a tile of the document grid, so the two anchors give this
/// pixel different colours.
#[test]
fn a_pattern_stroke_without_a_reference_point_still_anchors_at_the_layer_corner() {
    let pat = quad_pattern();
    let mut l = shape(SHAPE);
    l.effects.items = vec![Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: 3.0,
        position: StrokePosition::Outside,
        paint: FxPaint::Pattern { name: pat.name.clone(), id: pat.id.clone(), scale: 1.0 },
        overprint: false,
    })];
    let mut d = Document::with_background("Pattern overlay origin", Size::new(400, 300), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    d.patterns.push(pat);
    d.layers.push(l);
    let b = flat(&d);
    // (99, 76) and (99, 96) are the outside stroke just left of the layer's edge: green on a grid
    // starting at the layer's corner y 75, black on one starting at the document origin.
    assert_eq!(letter(pixel(&b, 99, 76)), 'G', "pattern stroke at (99, 76): {:?}", pixel(&b, 99, 76));
    assert_eq!(letter(pixel(&b, 99, 96)), 'G', "pattern stroke at (99, 96): {:?}", pixel(&b, 99, 96));
    // A move phase is the overlay's alone: the stroke reads the same with one recorded.
    let moved = flat(&with_move_phase(d, (5.0, 3.0)));
    assert_eq!(moved.px, b.px, "a recorded move phase does not move the pattern stroke's grid");
}
