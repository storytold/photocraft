//! #2117: the Layers panel's mask thumbnail follows whole-mask edits such as Image › Adjustments ›
//! Invert, not only brush strokes. A fresh Reveal All mask has no tiles, so inverting it only
//! changes the surface's default pixel; the thumbnail cache key must see that.

use craft::layers::ui::thumbs::LayerThumb;
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use photocraft_doc::LayerId;
use photocraft_raster::Surface;
use serde_json::json;

use crate::panels::layers_tree::PhotoThumbs;
use crate::{PhotocraftApp, surface_fingerprint};

#[test]
fn fingerprint_sees_the_default_pixel() {
    let fmt = photocraft_color::PixelFormat::GRAY8;
    let white = Surface::with_default(fmt, &[1.0]);
    let black = Surface::with_default(fmt, &[0.0]);
    assert_eq!(white.tile_count(), 0);
    assert_ne!(surface_fingerprint(&white), surface_fingerprint(&black));
    assert_eq!(surface_fingerprint(&white), surface_fingerprint(&white.clone()));
}

/// The mask thumbnail's revision follows Invert on a tile-less mask.
#[test]
fn inverting_a_tileless_mask_refreshes_its_thumbnail() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id = LayerId(app.run("layer.new.layer", json!({"name": "Masked"})).unwrap()["layer"].as_u64().unwrap());
    app.run("layer.layerMask.revealAll", json!({"layer": id.0})).unwrap();
    let revision = |app: &PhotocraftApp| {
        let doc = &app.session.active().unwrap().doc;
        PhotoThumbs::new(app, doc).revision(&LayerThumb::mask(id), &ThumbRequest::for_cell(32.0, 1.0))
    };
    let before = revision(&app);
    app.run("image.adjustments.invert", json!({"target": "mask"})).unwrap();
    let mask = app.session.active().unwrap().doc.layer(id).unwrap().mask.clone().unwrap();
    assert_eq!(mask.surface.tile_count(), 0, "the inverted Reveal All mask is still tile-less");
    assert_eq!(mask.surface.default_pixel(), vec![0.0], "and now hides everything");
    assert_ne!(revision(&app), before, "the thumbnail is rebuilt after Invert");
}
