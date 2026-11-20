//! #10881: the Layers panel's Attribute filter, against the cases measured in Photoshop.

use std::collections::BTreeSet;

use photocraft_doc::{BlendRange, Document, Group, Knockout, Layer, LayerContent, LayerMask, VectorMask};
use serde_json::json;

use craft::layers::ui::filter::{FilterBy, LayerAttribute};

use super::tests::filter;
use crate::PhotocraftApp;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// One layer per measured case. Every pixel layer has pixels except "empty"; the adjustment, fill
/// and shape layers come from their commands, so they carry the masks those give them.
fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let pixel_layers = [
        "pixels",
        "hidden",
        "position lock",
        "all locked",
        "empty",
        "linked a",
        "linked b",
        "base",
        "masked",
        "vector masked",
        "fx",
        "fx off",
        "fill 50",
        "adv channels",
        "adv knockout",
        "adv interior",
        "adv clipped",
        "adv shapes",
        "adv layer mask",
        "adv vector mask",
        "adv blend if",
    ];
    for name in pixel_layers {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        match name {
            "base" => {
                s.execute("layer.newAdjustmentLayer.levels", json!({})).unwrap();
                rename_active(&mut s, "clipped");
                s.execute("layer.createClippingMask", json!({})).unwrap();
            }
            "fx" => {
                s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
            }
            "fx off" => {
                s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": "dropShadow", "params": {"enabled": false}}]})).unwrap();
            }
            _ => {}
        }
    }
    s.execute("layer.newFillLayer.solidColor", json!({"color": "#00ff00"})).unwrap();
    rename_active(&mut s, "fill");
    s.execute("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
    rename_active(&mut s, "shape");

    let doc = std::sync::Arc::make_mut(&mut s.active_mut().unwrap().doc);
    let full = photocraft_geom::Rect::new(0, 0, 64, 48);
    for name in pixel_layers.into_iter().filter(|n| *n != "empty") {
        let l = layer(doc, name);
        let LayerContent::Raster(surface) = &mut l.content else { panic!("{name} is a pixel layer") };
        surface.fill_rect(full, &RED);
    }
    layer(doc, "hidden").visible = false;
    layer(doc, "position lock").locks.position = true;
    layer(doc, "all locked").locks.all = true;
    layer(doc, "linked a").link_group = Some(7);
    layer(doc, "linked b").link_group = Some(7);
    layer(doc, "masked").mask = Some(LayerMask::reveal_all());
    layer(doc, "vector masked").vector_mask = Some(VectorMask { path: Default::default(), enabled: true, linked: true, density: 1.0, feather: 0.0 });
    layer(doc, "fill 50").fill_opacity = 0.5;
    layer(doc, "adv channels").excluded_channels = 0b010;
    layer(doc, "adv knockout").advanced.knockout = Knockout::Shallow;
    layer(doc, "adv interior").advanced.blend_interior = true;
    layer(doc, "adv clipped").advanced.blend_clipped = false;
    layer(doc, "adv shapes").advanced.transparency_shapes = false;
    layer(doc, "adv layer mask").advanced.layer_mask_hides_effects = true;
    layer(doc, "adv vector mask").advanced.vector_mask_hides_effects = true;
    layer(doc, "adv blend if").blend_if.ranges = vec![[BlendRange { black: [40, 40], white: [255, 255] }, BlendRange::FULL]];
    // A visible group holding a pixel layer, and an empty group.
    let mut inside = Layer::new("inside", LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA8)));
    if let LayerContent::Raster(surface) = &mut inside.content {
        surface.fill_rect(full, &RED);
    }
    doc.layers.push(Layer::new("group", LayerContent::Group(Group { children: vec![inside], expanded: true, artboard: None, frame_stroke: None })));
    doc.layers.push(Layer::new("empty group", LayerContent::Group(Group { children: Vec::new(), expanded: true, artboard: None, frame_stroke: None })));
    // A hidden group whose child is visible: Visible goes by the layer's own flag.
    let mut child = Layer::new("in hidden group", LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA8)));
    if let LayerContent::Raster(surface) = &mut child.content {
        surface.fill_rect(full, &RED);
    }
    let mut hidden_group = Layer::new("hidden group", LayerContent::Group(Group { children: vec![child], expanded: true, artboard: None, frame_stroke: None }));
    hidden_group.visible = false;
    doc.layers.push(hidden_group);
    s
}

fn layer<'a>(doc: &'a mut Document, name: &str) -> &'a mut Layer {
    let id = doc.walk().into_iter().find(|(_, _, l)| l.name == name).map(|(_, _, l)| l.id).unwrap_or_else(|| panic!("no layer {name}"));
    doc.layer_mut(id).unwrap()
}

fn rename_active(s: &mut photocraft_engine::Session, name: &str) {
    let d = s.active_mut().unwrap();
    let id = d.active_layer.unwrap();
    std::sync::Arc::make_mut(&mut d.doc).layer_mut(id).unwrap().name = name.into();
}

/// The names `attribute` lists.
fn listed(doc: &Document, attribute: LayerAttribute) -> BTreeSet<String> {
    super::tests::listed(doc, &filter(FilterBy::Attribute, |f| f.attribute = attribute)).into_iter().collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|n| n.to_string()).collect()
}

fn all(doc: &Document) -> BTreeSet<String> {
    doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
}

#[test]
fn each_attribute_lists_the_measured_layers() {
    let s = session();
    let doc = &s.active().unwrap().doc;
    let everything = all(doc);
    // A pixel layer without pixels; the empty group isn't listed.
    assert_eq!(listed(doc, LayerAttribute::Empty), names(&["empty"]));
    // Every other layer and group, the empty group too.
    assert_eq!(listed(doc, LayerAttribute::NotEmpty), everything.difference(&names(&["empty"])).cloned().collect::<BTreeSet<_>>());
}

#[test]
fn a_layer_erased_completely_is_empty() {
    // The Eraser only clears alpha, so the erased pixels keep their colour bytes.
    for depth in [8, 16, 32] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": "transparent"})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let name = s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).next().unwrap();
        let erase = |s: &mut photocraft_engine::Session, x: i32| {
            s.execute("paint.stroke", json!({"points": [[x, -10], [x, 40]], "size": 30, "erase": true, "brush": {"hardness": 1.0}})).unwrap();
        };
        erase(&mut s, 10);
        let doc = &s.active().unwrap().doc;
        assert_eq!(listed(doc, LayerAttribute::Empty), names(&[]), "{depth}: partly erased");
        assert_eq!(listed(doc, LayerAttribute::NotEmpty), names(&[&name]), "{depth}");
        erase(&mut s, 30);
        let doc = &s.active().unwrap().doc;
        assert_eq!(listed(doc, LayerAttribute::Empty), names(&[&name]), "{depth}: erased completely");
        assert_eq!(listed(doc, LayerAttribute::NotEmpty), names(&[]), "{depth}");
    }
}

#[test]
fn a_layer_with_an_opaque_default_pixel_is_not_empty() {
    // No stored tiles, but the default pixel covers the whole canvas.
    let mut doc = Document::new("Opaque default", photocraft_geom::Size::new(40, 30), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let opaque = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::RGBA8, &RED);
    doc.layers.push(Layer::new("opaque", LayerContent::Raster(opaque)));
    doc.layers.push(Layer::new("clear", LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA8))));
    assert_eq!(listed(&doc, LayerAttribute::Empty), names(&["clear"]));
    assert_eq!(listed(&doc, LayerAttribute::NotEmpty), names(&["opaque"]));
}

#[test]
fn an_opaque_default_layer_erased_on_the_whole_canvas_is_empty() {
    // 300 px wide: the second column of tiles reaches past the canvas, where the default stays.
    let mut doc = Document::new("Opaque erased", photocraft_geom::Size::new(300, 30), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let clear = [1.0, 0.0, 0.0, 0.0];
    for (name, erased) in [("erased", photocraft_geom::Rect::new(0, 0, 300, 30)), ("half erased", photocraft_geom::Rect::new(0, 0, 200, 30))] {
        let mut s = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::RGBA8, &RED);
        s.fill_rect(erased, &clear);
        doc.layers.push(Layer::new(name, LayerContent::Raster(s)));
    }
    let mut dot = photocraft_raster::Surface::with_default(photocraft_doc::PixelFormat::RGBA8, &RED);
    dot.fill_rect(photocraft_geom::Rect::new(0, 0, 300, 30), &clear);
    dot.fill_rect(photocraft_geom::Rect::new(299, 29, 300, 30), &RED);
    doc.layers.push(Layer::new("one pixel", LayerContent::Raster(dot)));
    // The half erased layer has no tile over x 256..300, so the default shows there.
    assert_eq!(listed(&doc, LayerAttribute::Empty), names(&["erased"]));
    assert_eq!(listed(&doc, LayerAttribute::NotEmpty), names(&["half erased", "one pixel"]));
}

#[test]
fn the_empty_layers_are_found_once_per_document() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "background": "transparent"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    app.ui.layer_filter = filter(FilterBy::Attribute, |f| f.attribute = LayerAttribute::Empty);
    let empty = |app: &PhotocraftApp| super::attribute::empty_layers(app);
    let first = empty(&app).expect("found for Empty");
    assert!(first.is_empty(), "the filled layer has pixels");
    let again = empty(&app).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again), "the same document reuses the result");
    // Selecting a layer moves the revision but changes no pixels.
    let id = app.session.active().unwrap().doc.layers[0].id;
    app.session.select_layer(id).unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &empty(&app).unwrap()), "a selection reuses the result");
    app.session.execute("paint.stroke", json!({"points": [[20, -10], [20, 40]], "size": 60, "erase": true, "brush": {"hardness": 1.0}})).unwrap();
    assert_eq!(empty(&app).map(|set| set.len()), Some(1), "an edit finds them again");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(super::tests::listed_app(&app).len(), doc.walk().len(), "the erased layer is listed");
    app.ui.layer_filter.attribute = LayerAttribute::NotEmpty;
    assert!(super::tests::listed_app(&app).is_empty(), "Not Empty lists none of them");
    // Another document, with no empty layer, gets its own result.
    app.session.execute("file.new", json!({"width": 40, "height": 30})).unwrap();
    assert_eq!(empty(&app).map(|set| set.len()), Some(0), "the new document's layers");
}
