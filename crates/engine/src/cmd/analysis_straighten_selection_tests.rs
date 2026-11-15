//! #11993: with a selection active, Straighten Layer turns the whole active layer, exactly as
//! without one, and the selection is the same afterwards. One undo brings both back, and no
//! channel is left behind (the reference saves the selection to a channel and deletes it again).

use photocraft_doc::LayerId;

use super::*;

const CMD: &str = "image.analysis.straightenLayer";

/// An 800 × 600 document with a pixel layer "Colors" painted at 100,100 to 400,300, active, and a
/// ruler that tilts it by a few degrees.
fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600, "background": "white"})).unwrap();
    let l = LayerId(s.execute("layer.new.layer", json!({"name": "Colors"})).unwrap()["layer"].as_u64().unwrap());
    s.edit("paint", |doc, _| {
        let surf = doc.layer_mut(l).unwrap().surface_mut().unwrap();
        surf.fill_rect(Rect::new(100, 100, 400, 300), &[0.86, 0.12, 0.12, 1.0]);
        surf.fill_rect(Rect::new(250, 180, 310, 220), &[0.1, 0.3, 0.9, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("image.analysis.rulerTool", json!({"start": [100, 300], "end": [500, 270]})).unwrap();
    (s, l)
}

fn select_square(s: &mut Session) {
    s.execute("select.rect", json!({"x": 150, "y": 150, "width": 50, "height": 50})).unwrap();
}

fn layer(s: &Session, l: LayerId) -> photocraft_doc::Surface {
    s.active().unwrap().doc.layer(l).unwrap().surface().unwrap().clone()
}

fn selection(s: &Session) -> Option<photocraft_doc::Surface> {
    s.active().unwrap().doc.selection.clone()
}

#[test]
fn the_whole_layer_turns_as_without_a_selection() {
    let (mut plain, l) = session();
    plain.execute(CMD, json!({})).unwrap();
    let (mut with, l2) = session();
    select_square(&mut with);
    with.execute(CMD, json!({})).unwrap();
    let (a, b) = (layer(&plain, l), layer(&with, l2));
    assert_eq!(a.content_bounds(), b.content_bounds(), "same bounds");
    assert!(a.content_bounds().width() > 300, "the whole layer turned, not a 50 px piece: {:?}", b.content_bounds());
    assert!(a == b, "same pixels");
}

#[test]
fn the_selection_is_the_same_afterwards() {
    let (mut s, _) = session();
    select_square(&mut s);
    let before = selection(&s);
    assert!(before.is_some());
    s.execute(CMD, json!({})).unwrap();
    assert!(selection(&s) == before, "same bounds and mask");
}

#[test]
fn one_undo_brings_back_the_layer_and_the_selection() {
    let (mut s, l) = session();
    select_square(&mut s);
    let (layer_before, sel_before) = (layer(&s, l), selection(&s));
    let ruler_before = s.active().unwrap().doc.measurement.ruler;
    s.execute(CMD, json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    assert!(layer(&s, l) == layer_before, "the layer as it was");
    assert!(selection(&s) == sel_before, "and the selection");
    assert_eq!(s.active().unwrap().doc.measurement.ruler, ruler_before);
}

#[test]
fn no_channel_is_left_behind() {
    let (mut s, _) = session();
    select_square(&mut s);
    let channels = s.active().unwrap().doc.channels.len();
    s.execute(CMD, json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.channels.len(), channels);
}

/// CodeRabbit on #14986: only a plain rectangle (every pixel fully selected) gets the whole-layer
/// turn. An elliptical or feathered selection was not measured and keeps today's behaviour: only the
/// selected pixels turn.
#[test]
fn an_elliptical_or_feathered_selection_keeps_turning_only_its_pixels() {
    for kind in ["ellipse", "feathered"] {
        let (mut s, l) = session();
        match kind {
            "ellipse" => {
                s.execute("select.rect", json!({"x": 150, "y": 150, "width": 50, "height": 50, "ellipse": true})).unwrap();
            }
            _ => {
                select_square(&mut s);
                s.execute("select.modify.feather", json!({"radius": 8})).unwrap();
            }
        }
        s.execute(CMD, json!({})).unwrap();
        let (mut plain, pl) = session();
        plain.execute(CMD, json!({})).unwrap();
        assert!(layer(&s, l) != layer(&plain, pl), "{kind}: not the whole-layer turn a plain rectangle gets");
    }
}

/// CodeRabbit on #14986: a floating-point selection (the XCF importer makes F32 ones) can hold 0.9995,
/// which is not fully selected: only a plain rectangle of exactly 1.0 gets the whole-layer turn. The old
/// 0.999 threshold took this one for a plain rectangle.
#[test]
fn an_f32_selection_with_a_barely_partial_pixel_is_not_a_plain_rectangle() {
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    let f32_selection = |edge: f32| {
        let mut sel = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        sel.fill_rect(Rect::new(150, 150, 200, 200), &[1.0]);
        sel.write_pixel(150, 150, &[edge]);
        sel
    };
    let turned = |edge: f32| {
        let (mut s, l) = session();
        s.edit("selection", |doc, _| {
            doc.selection = Some(f32_selection(edge));
            Ok(())
        })
        .unwrap();
        s.execute(CMD, json!({})).unwrap();
        layer(&s, l)
    };
    let (mut plain, pl) = session();
    plain.execute(CMD, json!({})).unwrap();
    let whole = layer(&plain, pl);
    assert!(turned(1.0) == whole, "exactly 1.0 everywhere is a plain rectangle: the whole layer turns");
    assert!(turned(0.9995) != whole, "0.9995 is not: only the selected pixels turn");
}
