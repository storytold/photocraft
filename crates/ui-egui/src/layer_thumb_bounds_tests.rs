//! #4281: Clip Thumbnails to Layer Bounds. A small object in a large document fills its Layers
//! panel thumbnail instead of being a speck in an empty canvas-sized one.

use craft::layers::ui::thumbs::{LayerThumb, ThumbContents};
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::panels::layers_tree::PhotoThumbs;

/// A 2000 by 1000 document with a 20 by 10 red speck on layer "Speck".
fn app() -> (Box<PhotocraftApp>, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 2000, "height": 1000})).unwrap();
    let id = s.execute("layer.new.layer", json!({"name": "Speck"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 200, "width": 20, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    (PhotocraftApp::new(s, crate::Services::default()), LayerId(id))
}

/// The contents the panel asks the speck's thumbnail for, the frame it shows and its revision.
fn thumb(app: &PhotocraftApp, id: LayerId) -> (ThumbContents, Rect, u64) {
    let contents = crate::layers_panel_ui::thumbnail_contents(app);
    let doc = &app.session.active().unwrap().doc;
    let frame = crate::layer_thumb_frame(doc.layer(id).unwrap(), doc.bounds(), contents == ThumbContents::LayerBounds);
    let revision = PhotoThumbs::new(app, doc).revision(&LayerThumb::pixels(id, contents), &ThumbRequest::for_cell(32.0, 1.0));
    (contents, frame, revision.get())
}

#[test]
fn layer_thumbnails_frame_the_canvas_until_clip_to_layer_bounds_is_on() {
    let (mut app, id) = app();
    assert!(!app.session.prefs().interface.clip_thumbnails_to_layer_bounds);
    let (contents, frame, before) = thumb(&app, id);
    assert_eq!((contents, frame), (ThumbContents::EntireDocument, Rect::new(0, 0, 2000, 1000)));

    app.run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": true})).unwrap();
    let (contents, frame, after) = thumb(&app, id);
    assert_eq!((contents, frame), (ThumbContents::LayerBounds, Rect::new(100, 200, 120, 210)), "framed on the filled rect");
    assert_ne!(before, after, "toggling the preference rebuilds the thumbnail");

    app.run("prefs.set", json!({"path": "interface.clipThumbnailsToLayerBounds", "value": false})).unwrap();
    assert_eq!(thumb(&app, id).1, Rect::new(0, 0, 2000, 1000), "and back to the canvas");
}

#[test]
fn empty_and_off_canvas_layers_keep_the_canvas_frame() {
    let canvas = Rect::new(0, 0, 50, 40);
    let raster = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    let mut empty = photocraft_doc::Layer::new("Empty", photocraft_doc::LayerContent::Raster(raster));
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, true), canvas);
    empty.surface_mut().unwrap().write_pixel(-10, -10, &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, true), canvas, "pixels only off the canvas");
    empty.surface_mut().unwrap().write_pixel(45, 5, &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, true), Rect::new(0, 0, 46, 6), "the layer bounds, cut to the canvas");
    assert_eq!(crate::layer_thumb_frame(&empty, canvas, false), canvas);
}

#[test]
fn exr_emission_thumbnail_preserves_light_and_coverage() {
    use craft::ui::primitives::textures::ThumbRender;
    use egui::Color32;
    use photocraft_codecs::{ChannelLayout, EncodeOptions, ExrCompression, Format, Image, encode};
    use photocraft_engine::jobs::{OpenSource, Started};

    // Four equally wide strips: emission, transparency, ordinary alpha, and opaque HDR.
    let strips = [[0.5, 0.25, 0.0, 0.0], [0.0; 4], [0.25, 0.0, 0.0, 0.5], [2.0, 0.25, 0.0, 1.0]];
    let pixels: Vec<_> = (0..16 * 4).flat_map(|i| strips[i % 16 / 4]).collect();
    let input = Image::from_f32(16, 4, ChannelLayout::Rgba, &pixels).unwrap();
    let bytes = encode(&input, Format::OpenExr, &EncodeOptions { exr_compression: ExrCompression::Zip16, ..Default::default() }).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("emission-thumbnail.exr");
    std::fs::write(&path, bytes).unwrap();

    let mut session = photocraft_engine::Session::new();
    let source = OpenSource::Path(path.to_string_lossy().into_owned());
    assert!(matches!(session.start_open_for_user_with_background("emission-thumbnail.exr", source, false).unwrap(), Started::Done(_)));
    let snapshot = session.execute("history.newSnapshot", json!({})).unwrap()["snapshot"].as_u64().unwrap();
    let app = PhotocraftApp::new(session, crate::Services::default());
    let state = app.session.active().unwrap();
    let source = PhotoThumbs::new(&app, &state.doc);
    let check = |image: egui::ColorImage| {
        assert_eq!(image.size, [16, 4]);
        for row in image.pixels.as_chunks::<16>().0 {
            assert_eq!(row[1], Color32::from_rgba_premultiplied(128, 64, 0, 0));
            assert_eq!(row[5], Color32::TRANSPARENT);
            assert_eq!(row[9], Color32::from_rgba_unmultiplied(128, 0, 0, 128));
            assert_eq!(row[13], Color32::from_rgb(255, 64, 0));
        }
    };
    let request = ThumbRequest::for_cell(16.0, 1.0);
    for contents in [ThumbContents::EntireDocument, ThumbContents::LayerBounds] {
        let key = LayerThumb::pixels(state.active_layer.unwrap(), contents);
        let ThumbRender::Ready(image) = source.render(&key, &request) else {
            panic!("expected a raster thumbnail");
        };
        check(image);
    }
    let history = crate::app::thumbs::SnapshotThumbs::new(&app, state.doc.id);
    let ThumbRender::Ready(image) = history.render(&snapshot, &request) else {
        panic!("expected a history thumbnail");
    };
    check(image);
}
