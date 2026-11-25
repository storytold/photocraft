//! Conformance cases for the first adapter increment: tile deltas and per-actor undo.

use photocraft_collab_adapter::{Actor, ApplyError, ChangeSet, Part, TileAddr, TileDelta, UndoLog, UndoRefusal, apply, diff};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerId, LayerMask};
use photocraft_geom::{Rect, Size, TileCoord};

fn document() -> Document {
    let mut doc = Document::new("collab", Size::new(600, 600), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Paint", doc.pixel_format()));
    doc.layers.push(Layer::raster("Other", doc.pixel_format()));
    doc
}

fn layer(doc: &Document, index: usize) -> LayerId {
    doc.layers[index].id
}

fn paint(doc: &mut Document, layer: LayerId, x: i32, y: i32, rgba: [f32; 4]) {
    let l = doc.layer_mut(layer).unwrap();
    let surface = l.surface_mut().unwrap();
    let px = photocraft_raster::from_rgba(&surface.format(), rgba);
    surface.fill_rect(Rect::from_xywh(x, y, 8, 8), &px);
}

fn paint_mask(doc: &mut Document, layer: LayerId, x: i32, y: i32) {
    let l = doc.layer_mut(layer).unwrap();
    let mask = l.mask.get_or_insert_with(LayerMask::reveal_all);
    let px = [0.0f32];
    let surface = &mut mask.surface;
    surface.fill_rect(Rect::from_xywh(x, y, 8, 8), &px);
}

fn tile_addrs(set: &ChangeSet) -> Vec<TileAddr> {
    set.tiles.iter().map(TileDelta::addr).collect()
}

/// A stroke in one corner names one tile of one layer; a stroke in another layer names another.
#[test]
fn diff_names_only_the_tiles_a_stroke_touched() {
    let doc = document();
    let (a, b) = (layer(&doc, 0), layer(&doc, 1));
    let before = doc.clone();
    let mut after = doc.clone();
    paint(&mut after, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    paint(&mut after, b, 300, 300, [0.0, 1.0, 0.0, 1.0]);

    let set = diff(&before, &after).unwrap();
    assert_eq!(tile_addrs(&set), vec![TileAddr::new(a, Part::Pixels, TileCoord::new(0, 0)), TileAddr::new(b, Part::Pixels, TileCoord::new(1, 1)),]);

    // Applying the delta to the before document reproduces the after pixels, and the second
    // replica has nothing left to send.
    let mut replica = before.clone();
    apply(&mut replica, &set).unwrap();
    assert!(diff(&replica, &after).unwrap().tiles.is_empty());
}

/// The patch carries the changed pixels only: two 8×8 dabs are 128 pixels, not a whole tile.
#[test]
fn a_patch_carries_only_the_pixels_that_changed() {
    let doc = document();
    let a = layer(&doc, 0);
    let before = doc.clone();
    let mut after = doc.clone();
    paint(&mut after, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    paint(&mut after, a, 100, 100, [0.0, 0.0, 1.0, 1.0]);

    let set = diff(&before, &after).unwrap();
    let [TileDelta::Patch { patch, .. }] = set.tiles.as_slice() else {
        panic!("expected one patch, got {:?}", set.tiles);
    };
    assert_eq!(patch.indices.len(), 128);
    assert_eq!(patch.pixels.len(), 128 * 4);
    assert!(set.tiles.iter().all(|d| matches!(d, TileDelta::Patch { .. })));
}

/// Two actors painting different pixels of one tile both keep their pixels; the server applies
/// each patch over whatever the tile holds.
#[test]
fn two_actors_in_one_tile_keep_each_others_pixels() {
    let doc = document();
    let a = layer(&doc, 0);
    let before = doc.clone();
    let mut after = doc.clone();
    paint(&mut after, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    let first = diff(&before, &after).unwrap();

    let mut later = after.clone();
    paint(&mut later, a, 100, 100, [0.0, 1.0, 0.0, 1.0]);
    let second = diff(&after, &later).unwrap();

    // The server saw the second edit first, then the first one arrives.
    let mut server = later.clone();
    apply(&mut server, &second).unwrap();
    apply(&mut server, &first).unwrap();

    // The opposite order gives the same bytes: the two patches commute inside the tile.
    let mut other_order = after.clone();
    apply(&mut other_order, &first).unwrap();
    apply(&mut other_order, &second).unwrap();
    assert!(diff(&server, &other_order).unwrap().tiles.is_empty());
    // Both dabs are readable: neither patch wrote default pixels over the other.
    let surface = server.layer(a).unwrap().surface().unwrap();
    let bytes = surface.tile(TileCoord::new(0, 0)).unwrap().bytes().to_vec();
    let red = &bytes[(10 * 256 + 10) * 4..(10 * 256 + 10) * 4 + 4];
    let green = &bytes[(100 * 256 + 100) * 4..(100 * 256 + 100) * 4 + 4];
    assert_eq!(red, &[255, 0, 0, 255]);
    assert_eq!(green, &[0, 255, 0, 255]);
}

/// Undo is per actor: B's pixels in another tile stay when A undoes.
#[test]
fn undo_restores_only_the_actors_tiles() {
    let doc = document();
    let a = layer(&doc, 0);
    let b = layer(&doc, 1);
    let mut log = UndoLog::default();
    let actor_a = Actor(1);
    let actor_b = Actor(2);

    let mut doc = doc;
    let before_a = doc.clone();
    paint(&mut doc, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    let change_a = diff(&before_a, &doc).unwrap();
    log.record(actor_a, 1, &before_a, &change_a, &doc).unwrap();

    let before_b = doc.clone();
    paint(&mut doc, b, 500, 500, [0.0, 1.0, 0.0, 1.0]);
    let change_b = diff(&before_b, &doc).unwrap();
    log.record(actor_b, 2, &before_b, &change_b, &doc).unwrap();

    log.undo(actor_a, &mut doc).unwrap();

    // A's tile is back to nothing, B's dab stays.
    let mut expected = before_a.clone();
    paint(&mut expected, b, 500, 500, [0.0, 1.0, 0.0, 1.0]);
    assert!(diff(&doc, &expected).unwrap().tiles.is_empty());
    assert!(log.history(actor_a).unwrap().can_redo());
    assert!(log.history(actor_b).unwrap().can_undo());
}

/// Undo refuses when another actor changed a pixel of the step, and leaves the document alone.
#[test]
fn undo_refuses_a_pixel_another_actor_changed() {
    let doc = document();
    let a = layer(&doc, 0);
    let mut doc = doc;
    let mut log = UndoLog::default();
    let actor_a = Actor(1);
    let actor_b = Actor(2);

    let before_a = doc.clone();
    paint(&mut doc, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    let change_a = diff(&before_a, &doc).unwrap();
    log.record(actor_a, 1, &before_a, &change_a, &doc).unwrap();

    // B paints over A's pixels in the same tile.
    let before_b = doc.clone();
    paint(&mut doc, a, 10, 10, [0.0, 0.0, 1.0, 1.0]);
    let change_b = diff(&before_b, &doc).unwrap();
    log.record(actor_b, 2, &before_b, &change_b, &doc).unwrap();

    let snapshot = doc.clone();
    let refused = log.undo(actor_a, &mut doc).unwrap_err();
    let UndoRefusal::Conflict { addr } = refused else {
        panic!("expected a conflict, got {refused:?}");
    };
    assert_eq!(addr, TileAddr::new(a, Part::Pixels, TileCoord::new(0, 0)));
    assert!(diff(&doc, &snapshot).unwrap().tiles.is_empty());
    assert!(log.history(actor_a).unwrap().can_undo());
}

/// Redo puts the step's own pixels back after undo, and still ignores the other actor's.
#[test]
fn redo_reapplies_the_after_image() {
    let doc = document();
    let a = layer(&doc, 0);
    let mut doc = doc;
    let mut log = UndoLog::default();
    let actor = Actor(7);

    let before = doc.clone();
    paint(&mut doc, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    let change = diff(&before, &doc).unwrap();
    log.record(actor, 1, &before, &change, &doc).unwrap();
    let after = doc.clone();

    log.undo(actor, &mut doc).unwrap();
    assert!(diff(&doc, &before).unwrap().tiles.is_empty());

    log.redo(actor, &mut doc).unwrap();
    assert!(diff(&doc, &after).unwrap().tiles.is_empty());
}

/// A mask edit is a tile of its own part and applies without touching the layer's pixels.
#[test]
fn a_mask_delta_is_separate_from_pixels() {
    let mut doc = document();
    let a = layer(&doc, 0);
    doc.layer_mut(a).unwrap().mask = Some(LayerMask::reveal_all());
    let before = doc.clone();
    let mut after = doc.clone();
    paint(&mut after, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    paint_mask(&mut after, a, 200, 200);

    let set = diff(&before, &after).unwrap();
    assert_eq!(tile_addrs(&set), vec![TileAddr::new(a, Part::Pixels, TileCoord::new(0, 0)), TileAddr::new(a, Part::Mask, TileCoord::new(0, 0)),]);

    let mut pixels_only = before.clone();
    let pixels: ChangeSet = ChangeSet { tiles: vec![set.tiles[0].clone()], ..Default::default() };
    apply(&mut pixels_only, &pixels).unwrap();
    let l = pixels_only.layer(a).unwrap();
    assert!(l.mask.as_ref().unwrap().surface.tile(TileCoord::new(0, 0)).is_none());
    assert!(l.surface().unwrap().tile(TileCoord::new(0, 0)).is_some());
}

/// Applying a delta to a document whose layer is gone is a typed refusal, not a panic.
#[test]
fn apply_refuses_a_missing_layer() {
    let doc = document();
    let a = layer(&doc, 0);
    let before = doc.clone();
    let mut after = doc.clone();
    paint(&mut after, a, 10, 10, [1.0, 0.0, 0.0, 1.0]);
    let set = diff(&before, &after).unwrap();

    let mut target = document();
    target.layers.retain(|l| l.id != a);
    assert_eq!(apply(&mut target, &set), Err(ApplyError::NoLayer(a)));
}
