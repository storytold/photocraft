//! The UI's pictures and derived values of document pixels follow the scratch read-error generation: a tile that
//! fails to read shows zeros and its real pixels again once it reads, without moving its surface's generation or its
//! document's revision (`crate::scratch_reads`). Each test simulates one such transition and checks that the key a
//! cache is held by moves with it, and stays put without it.

use craft::imaging::trace::MaskSource;
use craft::layers::ui::thumbs::{LayerThumb, ThumbContents};
use craft::ui::primitives::textures::{ThumbRequest, ThumbSource};
use serde_json::json;

use crate::PhotocraftApp;
use crate::channels_panel::Row;
use crate::scratch_reads::tests::simulate_read_error_transition;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    app.run("paint.stroke", json!({"points": [[4, 4], [40, 30]], "size": 8, "color": "#2060c0"})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.run("channel.new", json!({"name": "Alpha", "fill": "white"})).unwrap();
    app.run("select.rect", json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
    app
}

fn request() -> ThumbRequest {
    ThumbRequest::for_cell(32.0, 1.0)
}

/// Calls `revision` before, again without a change, and after one transition: the key must hold, then move.
fn holds_then_moves<T: PartialEq + std::fmt::Debug>(what: &str, mut revision: impl FnMut() -> T) {
    let before = revision();
    assert_eq!(revision(), before, "{what}: nothing changed, the same key");
    simulate_read_error_transition();
    assert_ne!(revision(), before, "{what}: a read error or its recovery moves the key");
}

#[test]
fn layer_thumbnails_follow_a_scratch_read_error_transition() {
    let app = app();
    let st = app.session.active().unwrap();
    let layer = st.active_layer.unwrap();
    let keys = [LayerThumb::pixels(layer, ThumbContents::EntireDocument), LayerThumb::mask(layer)];
    holds_then_moves("layer thumbnails", || {
        let thumbs = crate::panels::layers_tree::PhotoThumbs::new(&app, &st.doc);
        keys.iter().map(|key| thumbs.revision(key, &request()).get()).collect::<Vec<_>>()
    });
}

#[test]
fn the_rule_facts_follow_a_scratch_read_error_transition() {
    let app = app();
    holds_then_moves("rule facts", || crate::menus::facts::key(&app));
}

#[test]
fn channel_thumbnails_follow_a_scratch_read_error_transition() {
    let app = app();
    let st = app.session.active().unwrap();
    holds_then_moves("channel thumbnails", || {
        let thumbs = crate::app::thumbs::ChannelThumbs::new(&app, st);
        [Row::Composite, Row::Color(0), Row::Alpha(0), Row::LayerMask].iter().map(|row| thumbs.revision(row, &request()).get()).collect::<Vec<_>>()
    });
}

#[test]
fn snapshot_thumbnails_follow_a_scratch_read_error_transition() {
    let app = app();
    let st = app.session.active().unwrap();
    let snapshot = st.snapshots().first().map(|s| s.id).expect("a new document has its first snapshot");
    holds_then_moves("snapshot thumbnails", || crate::app::thumbs::SnapshotThumbs::new(&app, st.doc.id).revision(&snapshot, &request()).get());
}

/// A selection whose tile read as zeros scans as empty: once it reads again, the cached bounds are scanned anew.
#[test]
fn cached_bounds_are_scanned_again_after_a_scratch_read_error_transition() {
    let mut app = app();
    let (doc, selection) = {
        let st = app.session.active().unwrap();
        (st.doc.id, st.doc.selection.clone().unwrap())
    };
    let key = u64::MAX - doc.0;
    let bounds = app.cached_bounds(key, &selection);
    assert!(!bounds.is_empty());
    // The box a scan during the read error would have cached.
    let fingerprint = crate::app::thumbs::bounds_revision(&selection);
    app.bounds_cache.insert(key, (fingerprint, photocraft_geom::Rect::EMPTY));
    assert!(app.known_bounds(key, &selection).is_empty(), "without a transition the cached box is believed");
    simulate_read_error_transition();
    assert_eq!(app.known_bounds(key, &selection), bounds, "after it, the bounds are scanned again");
    assert_eq!(app.cached_bounds(key, &selection), bounds);
}

#[test]
fn the_selection_outline_follows_a_scratch_read_error_transition() {
    let app = app();
    let st = app.session.active().unwrap();
    let selection = st.doc.selection.as_ref().unwrap();
    let size = [st.doc.size.width, st.doc.size.height];
    holds_then_moves("selection outline", || crate::canvas::outline::SurfaceMask::new(selection, selection.generation(), size).revision());
}

#[test]
fn the_navigator_picture_draws_again_after_a_scratch_read_error_transition() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let doc = app.session.active().unwrap().doc.id;
    crate::canvas::navigator_texture(&mut app, &ctx, 0).unwrap();
    let held = |app: &PhotocraftApp| app.navigator_textures.get(&doc).map(|(_, key, _, texture)| (*key, texture.id())).unwrap();
    let before = held(&app);
    crate::canvas::navigator_texture(&mut app, &ctx, 0).unwrap();
    assert_eq!(held(&app), before, "nothing changed: the same picture");
    simulate_read_error_transition();
    crate::canvas::navigator_texture(&mut app, &ctx, 0).unwrap();
    assert_ne!(held(&app).0, before.0, "the picture is drawn again for the new generation");
}
