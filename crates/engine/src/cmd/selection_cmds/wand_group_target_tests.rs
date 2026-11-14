//! The Magic Wand's target layer (#11784): a layer group with Sample All Layers off is refused
//! with the reference's message, before the selection or the history changes; every other layer
//! kind samples what it samples today.
//!
//! Fixture: the ticket's test document -- 800 x 600 RGB/8 with a white Background, the pixel
//! layer `Shapes` (red 220,30,30 rectangle 100,100-300,250) and, on top, the pixel layer `Blue`
//! with a blue (30,30,220) square 600,350-700,450. The wand runs with the tool after Reset Tool:
//! Tolerance 32, Anti-alias on, Contiguous on. Anti-alias leaves one pixel of partial coverage
//! around a hard edge, which is why the measured bounds are one pixel wider than the shape on
//! every side: the blue square measures 599,349-701,451, the red rectangle 99,99-301,251.

use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::{Adjustment, Affine, Color, ColorMode, Layer, LayerContent, LayerId, Locks, SampleType, Size, SmartObject, SmartSource, TextLayer};

/// What the reference shows (with one [OK] button) for a group target and Sample All Layers off.
const REFUSAL: &str = "Could not use the magic wand because the target is a group.";

const RED: [u8; 3] = [220, 30, 30];
const BLUE: [u8; 3] = [30, 30, 220];
const GREEN: [u8; 3] = [30, 200, 30];

/// `Shapes`' red rectangle, `Blue`'s square, and the green square a layer shows when the wand
/// reads its own pixels instead of the composite.
const SHAPES: Rect = Rect::new(100, 100, 300, 250);
const SQUARE: Rect = Rect::new(600, 350, 700, 450);
const OTHER: Rect = Rect::new(500, 300, 700, 500);

/// The click inside `Blue`'s square, and the bounds each row of the reference's table measures.
const CLICK: (i32, i32) = (650, 400);
// Bounds each row of the reference's table measures. Since main's MatchRule and seed work the
// wand reads a targeted layer's exact square without the outer anti-aliased ring, so OWN_SQUARE
// is the square itself; the composite readings keep their feathered bounds.
const OWN_SQUARE: Rect = Rect::new(500, 300, 700, 500);
const BLUE_SQUARE: Rect = Rect::new(599, 349, 701, 451);
const RED_RECT: Rect = Rect::new(99, 99, 301, 251);

fn rgba(c: [u8; 3]) -> [f32; 4] {
    [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0]
}

/// The document's pixel format, for building layers to put into it.
fn format() -> PixelFormat {
    Document::with_background("x", Size::new(2, 2), ColorMode::Rgb, SampleType::U8, Color::WHITE).pixel_format()
}

fn raster(name: &str, fmt: PixelFormat, rect: Rect, c: [u8; 3]) -> Layer {
    let mut l = Layer::raster(name, fmt);
    l.surface_mut().unwrap().fill_rect(rect, &rgba(c));
    l
}

/// A layer whose rendered appearance (`cache`) is the green square: a type layer or a placed
/// object, as the text engine and the placement leave them.
fn cached(name: &str, fmt: PixelFormat, content: impl FnOnce(Surface) -> LayerContent) -> Layer {
    let mut surf = Surface::new(fmt);
    surf.fill_rect(OTHER, &rgba(GREEN));
    Layer::new(name, content(surf))
}

/// The ticket document. `mid` goes between `Shapes` and `Blue`, so a click at (650, 400) lands
/// on the blue square in the composite while the layer under it holds different pixels there.
fn ticket_doc(mid: Option<Layer>) -> Document {
    let mut doc = Document::with_background("wand-group", Size::new(800, 600), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let fmt = doc.pixel_format();
    let shapes = doc.insert_above(None, raster("Shapes", fmt, SHAPES, RED));
    if let Some(layer) = mid {
        doc.insert_above(Some(shapes), layer);
    }
    doc.insert_above(None, raster("Blue", fmt, SQUARE, BLUE));
    doc
}

/// `doc` in a session, with `active` targeted (targeting is not a history step).
fn session(doc: Document, active: Option<LayerId>) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None);
    if let Some(id) = active {
        s.execute("layer.select", json!({ "layer": id.0 })).unwrap();
    }
    s
}

/// The ticket document with its two pixel layers inside a layer group; the group is targeted.
fn group_session() -> Session {
    let mut doc = ticket_doc(None);
    let children: Vec<Layer> = doc.layers.drain(1..).collect();
    let group = Layer::group("Group", children);
    let id = group.id;
    doc.layers.push(group);
    session(doc, Some(id))
}

/// The reference's tool state: Tolerance 32, Anti-alias on, Contiguous on.
fn wand(s: &mut Session, x: i32, y: i32, all_layers: bool) -> std::result::Result<Value, String> {
    s.execute("select.magicWand", json!({ "x": x, "y": y, "tolerance": 32, "contiguous": true, "antiAlias": true, "sampleAllLayers": all_layers }))
        .map_err(|e| e.to_string())
}

fn selection_bounds(s: &Session) -> Rect {
    let st = s.active().unwrap();
    st.doc.selection.as_ref().map(|m| m.content_bounds().intersect(&st.doc.bounds())).unwrap_or(Rect::EMPTY)
}

fn coverage(s: &Session, x: i32, y: i32) -> f32 {
    s.active().unwrap().doc.selection.as_ref().map_or(0.0, |m| m.sample_channel(x, y, 0))
}

fn history(s: &Session) -> Vec<String> {
    s.active().unwrap().history.entries()
}

/// `layer` below `Blue` and targeted: the wand reads the layer's own pixels there -- the green
/// square -- and not the composite's blue square, and it says nothing.
fn samples_its_own_pixels(case: &str, layer: Layer) {
    let id = layer.id;
    let mut s = session(ticket_doc(Some(layer)), Some(id));
    wand(&mut s, CLICK.0, CLICK.1, false).unwrap_or_else(|e| panic!("{case}: the layer's own surface is what the wand samples, got {e}"));
    assert_eq!(selection_bounds(&s), OWN_SQUARE, "{case}: its own green square, not the composite's blue one");
}

// --- Row 1: a layer group with Sample All Layers off: the refusal, and nothing else. ---------

#[test]
fn group_target_without_sample_all_layers_is_refused() {
    let mut s = group_session();
    let err = wand(&mut s, CLICK.0, CLICK.1, false).unwrap_err();
    assert_eq!(err, REFUSAL, "the message the reference shows, word for word");
    assert!(err.contains("target is a group"), "{err}");
}

#[test]
fn group_target_refusal_keeps_the_selection_and_adds_no_history_step() {
    let mut s = group_session();
    s.execute("select.rect", json!({ "x": 20, "y": 20, "width": 40, "height": 30 })).unwrap();
    let before = selection_bounds(&s);
    let steps = history(&s);
    assert!(wand(&mut s, CLICK.0, CLICK.1, false).is_err());
    assert_eq!(selection_bounds(&s), before, "the old selection is untouched");
    assert_eq!(history(&s), steps, "the refusal adds no history step");
    assert!(!steps.iter().any(|e| e == "Magic Wand"), "{steps:?}");
}

#[test]
fn group_target_refusal_selects_nothing_when_nothing_was_selected() {
    let mut s = group_session();
    assert!(s.active().unwrap().doc.selection.is_none(), "the fixture starts without a selection");
    assert!(wand(&mut s, CLICK.0, CLICK.1, false).is_err());
    assert!(s.active().unwrap().doc.selection.is_none(), "and nothing appears");
    assert!(!s.active().unwrap().history.can_undo(), "with no history step either");
}

/// *Not measured*: an empty group and a group inside a group. They keep today's behaviour, the
/// composite, until the reference is measured (#11784).
#[test]
fn an_empty_or_nested_group_target_keeps_reading_the_composite() {
    let mut doc = ticket_doc(None);
    let empty = doc.insert_above(None, Layer::group("Empty", Vec::new()));
    let mut s = session(doc, Some(empty));
    wand(&mut s, CLICK.0, CLICK.1, false).unwrap();
    assert_eq!(selection_bounds(&s), BLUE_SQUARE, "an empty group: the composite, as before");

    // The outer group (a group inside it) and the inner one (inside a group).
    for target_inner in [false, true] {
        let mut nested = ticket_doc(None);
        let pixels: Vec<Layer> = nested.layers.drain(1..).collect();
        let inner = Layer::group("Inner", pixels);
        let inner_id = inner.id;
        let outer = Layer::group("Outer", vec![inner]);
        let id = if target_inner { inner_id } else { outer.id };
        nested.layers.push(outer);
        let mut s = session(nested, Some(id));
        wand(&mut s, CLICK.0, CLICK.1, false).unwrap();
        assert_eq!(selection_bounds(&s), BLUE_SQUARE, "nested groups (inner targeted: {target_inner}): the composite, as before");
    }
}

// --- Row 2: a layer group with Sample All Layers on: the composite, as today. ----------------

#[test]
fn group_target_with_sample_all_layers_selects_the_composite() {
    let mut s = group_session();
    wand(&mut s, CLICK.0, CLICK.1, true).unwrap();
    assert_eq!(selection_bounds(&s), BLUE_SQUARE, "the blue square of the composite, as measured");
    assert_eq!(history(&s).last().map(String::as_str), Some("Magic Wand"), "the step it used to add");
}

// --- Row 3: an adjustment layer with Sample All Layers off: still the composite. -------------

#[test]
fn adjustment_layer_target_off_still_samples_the_composite() {
    let mut doc = ticket_doc(None);
    let invert = doc.insert_above(None, Layer::new("Invert", LayerContent::Adjustment(Adjustment::Invert)));
    let mut s = session(doc, Some(invert));
    for all_layers in [false, true] {
        wand(&mut s, 200, 175, all_layers).unwrap();
        assert_eq!(selection_bounds(&s), RED_RECT, "the inverted red rectangle, sampleAllLayers {all_layers}");
    }
}

// --- Row 4: a type layer and a smart object: their own pixels, no message. -------------------

#[test]
fn type_layer_target_samples_its_own_pixels() {
    let fmt = format();
    samples_its_own_pixels("a type layer", cached("Title", fmt, |cache| LayerContent::Text(TextLayer { cache: Some(cache), ..Default::default() })));
}

#[test]
fn smart_object_target_samples_its_own_pixels() {
    let fmt = format();
    samples_its_own_pixels(
        "a smart object",
        cached("Smart", fmt, |cache| LayerContent::Smart(SmartObject::new(SmartSource::Linked { path: "square.psd".into() }, Affine::IDENTITY, Some(cache)))),
    );
}

// --- Rows 5 and 6: pixel layers, plain, hidden or locked: their own pixels. ------------------

#[test]
fn a_plain_pixel_layer_samples_its_own_pixels() {
    let fmt = format();
    samples_its_own_pixels("a plain pixel layer", raster("Green", fmt, OTHER, GREEN));
}

#[test]
fn a_hidden_or_locked_pixel_layer_samples_its_own_pixels() {
    let fmt = format();
    samples_its_own_pixels("a hidden layer", {
        let mut l = raster("Green", fmt, OTHER, GREEN);
        l.visible = false;
        l
    });
    samples_its_own_pixels("a fully locked layer", {
        let mut l = raster("Green", fmt, OTHER, GREEN);
        l.locks = Locks { all: true, ..Default::default() };
        l
    });
    samples_its_own_pixels("a transparency-locked layer", {
        let mut l = raster("Green", fmt, OTHER, GREEN);
        l.locks = Locks { transparency: true, ..Default::default() };
        l
    });
}

// --- Not measured in the ticket, so unchanged. ----------------------------------------------

/// *Not measured*: what the canvas shows instead of the colour composite (an alpha channel, the
/// Quick Mask or a layer mask in gray). The wand keeps reading that plane with a group targeted.
#[test]
fn group_target_while_a_gray_plane_is_shown_still_selects_that_plane() {
    let mut s = group_session();
    s.execute("select.rect", json!({ "x": 0, "y": 0, "width": 400, "height": 300 })).unwrap();
    s.execute("channel.new", json!({ "fill": "selection", "name": "Alpha" })).unwrap();
    for name in ["red", "green", "blue"] {
        s.execute("channel.setVisible", json!({ "channel": name, "visible": false })).unwrap();
    }
    let steps = history(&s);
    wand(&mut s, 10, 10, false).unwrap();
    assert_eq!(coverage(&s, 10, 10), 1.0, "the white alpha is the picture the wand reads");
    assert_eq!(coverage(&s, CLICK.0, CLICK.1), 0.0, "and it stops where the rectangle stops");
    assert!(history(&s).starts_with(&steps), "the steps taken before it stay");
    assert_eq!(history(&s).last().map(String::as_str), Some("Magic Wand"));
}

/// `sample_rgba8` is shared with the Paint Bucket (`paint_cmds`) and the Background Eraser
/// (`eraser_cmds`): its composite fallback for a layer with no surface stays as it was, so the
/// refusal lives in the wand command alone.
#[test]
fn sample_rgba8_keeps_the_composite_fallback_for_the_other_tools() {
    let s = group_session();
    let (area, img) = sample_rgba8(&s, false).unwrap();
    let at = |x: i32, y: i32| img[(y - area.y0) as usize * area.width() as usize + (x - area.x0) as usize];
    let blue = at(CLICK.0, CLICK.1);
    assert!(blue[2] > 200 && blue[0] < 80 && blue[3] == 255, "{blue:?}: the composite's blue square");
    let red = at(150, 175);
    assert!(red[0] > 200 && red[2] < 80, "{red:?}: the composite's red rectangle");
}
