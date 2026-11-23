use super::{Document, Rect, Session, json, rotate_arbitrary_with};
use photocraft_doc::Size;

const ARBITRARY: &str = "image.rotation.arbitrary";

fn session(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": depth, "background": "white"})).unwrap();
    s.execute("tools.setColors", json!({"background": "#ff00ff"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.edit("Fixture", |doc, active| {
        let background = doc.background_mut().unwrap().surface_mut().unwrap();
        let blue = photocraft_raster::from_rgba(&background.format(), [0.0, 0.0, 1.0, 1.0]);
        background.fill_rect(Rect::new(396, 297, 400, 300), &blue);
        let top = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let green = photocraft_raster::from_rgba(&top.format(), [0.0, 1.0, 0.0, 1.0]);
        top.fill_rect(Rect::new(175, 125, 225, 175), &green);
        let translucent = photocraft_raster::from_rgba(&top.format(), [0.15432, 0.891, 1.0, 0.63654]);
        top.fill_rect(Rect::new(175, 125, 182, 130), &translucent);
        Ok(())
    })
    .unwrap();
    s
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn arbitrary_right_angle_turns_guides_clears_selection_and_keeps_count_points() {
    let mut s = session(8);
    s.execute("view.newGuide", json!({"orientation":"vertical","position":20})).unwrap();
    s.execute("select.rect", json!({"x":175,"y":125,"width":50,"height":50})).unwrap();
    s.execute("count.add", json!({"x":10,"y":20})).unwrap();
    let before = doc(&s).clone();

    s.execute(ARBITRARY, json!({"angle":90})).unwrap();
    assert!(doc(&s).selection.is_none());
    assert_eq!(doc(&s).guides.horizontal, vec![20.0]);
    assert!(doc(&s).guides.vertical.is_empty());
    assert_eq!(doc(&s).measurement.count_groups, before.measurement.count_groups);
    assert!(s.undo());
    assert_eq!(doc(&s).guides, before.guides);
    assert_eq!(doc(&s).selection, before.selection);
    assert_eq!(doc(&s).size, before.size);
}

#[test]
fn right_angle_crop_still_drops_guides() {
    let mut s = session(8);
    s.execute("view.newGuide", json!({"orientation":"vertical","position":150})).unwrap();
    s.execute("image.crop", json!({"x":50,"y":40,"width":200,"height":150,"angle":90})).unwrap();
    assert_eq!(doc(&s).size, Size::new(200, 150));
    assert!(doc(&s).guides.horizontal.is_empty() && doc(&s).guides.vertical.is_empty());
}

#[test]
fn artboard_right_angle_keeps_the_resampling_path() {
    let mut arbitrary = session(8);
    let mut legacy = session(8);
    for s in [&mut arbitrary, &mut legacy] {
        s.execute("layer.new.artboard", json!({"rect":[0,0,100,80]})).unwrap();
        assert!(doc(s).has_artboards());
    }
    let params = json!({"angle":90});
    let actual = arbitrary.execute(ARBITRARY, params.clone()).unwrap();
    // The shared path used by Straighten still runs the existing resampler.
    assert_eq!(actual, rotate_arbitrary_with(&mut legacy, &params, false, false).unwrap());
    let (actual, legacy) = (doc(&arbitrary), doc(&legacy));
    assert_eq!(actual.size, legacy.size);
    let (actual_layers, legacy_layers) = (actual.walk(), legacy.walk());
    assert_eq!(actual_layers.len(), legacy_layers.len());
    for ((actual_path, _, actual), (legacy_path, _, legacy)) in actual_layers.into_iter().zip(legacy_layers) {
        assert_eq!(actual_path, legacy_path);
        assert_eq!(actual.artboard().map(|board| board.rect), legacy.artboard().map(|board| board.rect));
        assert!(actual.surface() == legacy.surface(), "artboard fallback changed the pixels of {}", actual.name);
    }
}
