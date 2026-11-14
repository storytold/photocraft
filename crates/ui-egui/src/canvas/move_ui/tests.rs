use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

#[test]
fn dragging_a_layer_larger_than_the_canvas_redraws_what_comes_in_from_beyond_the_edge() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(-32, -32, 96, 96), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, m);
    let mut keys = Vec::new();
    for x in [42.0, 52.0] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
        keys.push(super::display_doc(&mut app, 0).unwrap().1);
    }
    let st = app.session.active().unwrap();
    // From +10 to +20 the strip x 0..10 shows pixels that were off the canvas a frame ago.
    let r = super::damage(&app, st.doc.id, st.revision, keys[0], keys[1]).unwrap();
    assert_eq!(r, Rect::new(0, 0, 64, 64));
}

/// The Move tool shows how far it has moved the layer, ΔX/ΔY in whole pixels, as Photoshop
/// and Free Transform do (#5727): once the pointer moves, until the release.
#[test]
fn a_move_drag_reads_out_the_offset_it_will_commit() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(16, 16, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    assert_eq!(super::offset_readout(&app), None, "no drag");
    // Pressed on the red square (16..32): with Auto-Select (on by default) an empty spot would
    // start the layer marquee instead of a move.
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 24.0, y: 24.0, pressure: 1.0 }, m);
    assert_eq!(super::offset_readout(&app), None, "pressed, not moved yet");
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 34.4, y: 20.8, pressure: 1.0 }, m);
    assert_eq!(super::offset_readout(&app), Some(["10 px".to_string(), "-3 px".to_string()]));
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 34.4, y: 20.8 }, m);
    assert_eq!(super::offset_readout(&app), None, "released");
}

/// A release past the ±10,000,000 px the preview and the readout show commits the offset they
/// showed, not the raw pointer distance.
#[test]
fn a_release_commits_the_offset_the_readout_caps_at() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    super::finish(&mut app, 2.5e7, -2.5e7);
    let st = app.session.active().unwrap();
    let s = st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| l.surface()).unwrap();
    assert!(s.pixel(16 + 10_000_000, 16 - 10_000_000)[3] > 0.5, "moved by the capped offset");
}

/// A shape reaching past the canvas edge renders cut at the canvas, but its path goes on: a
/// drag frame must redraw where the rest of it was, or the cut edge stays behind (#3946).
#[test]
fn dragging_a_shape_past_the_canvas_edge_leaves_no_trail() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.sync_views();
    let id = app.session.execute("shape.create", json!({"kind": "rect", "rect": [40, 16, 50, 32], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let id = photocraft_doc::LayerId(id);
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 32.0, pressure: 1.0 }, m);
    let mut frames = Vec::new();
    for x in [40.0, 10.0] {
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
        frames.push(super::display_doc(&mut app, 0).unwrap());
    }
    let st = app.session.active().unwrap();
    let r = super::damage(&app, st.doc.id, st.revision, frames[0].1, frames[1].1).unwrap();
    let alpha = |d: &photocraft_doc::Document, x, y| d.layer(id).and_then(|l| l.surface()).map_or(0.0, |s| s.pixel(x, y)[3]);
    for y in 0..64 {
        for x in 0..64 {
            if alpha(&frames[0].0, x, y) != alpha(&frames[1].0, x, y) {
                assert!(r.contains(x, y), "({x}, {y}) changed outside the redrawn {r:?}");
            }
        }
    }
}

/// #9329: an adjustment's mask slides over layers that stay put, so a Move drag of it changes
/// pixels away from where its bound in place sits. Every pixel that differs between two drag
/// frames' full composites must be inside the preview's damage (`None` redraws everything).
#[test]
fn dragging_a_masked_adjustment_redraws_the_layers_its_mask_slides_over() {
    for depth in [8, 32] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent", "depth": depth})).unwrap();
        app.sync_views();
        // A small static layer at (8..16)², and an Invert masked to x 24..56, clear of it.
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.rect", json!({"x": 24, "y": 0, "width": 32, "height": 64})).unwrap();
        app.session.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 40.0, y: 32.0, pressure: 1.0 }, m);
        let mut frames = Vec::new();
        // At -2 the mask (22..54) misses the small layer; at -20 (4..36) it covers it.
        for x in [38.0, 20.0] {
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y: 32.0, pressure: 1.0 }, m);
            frames.push(super::display_doc(&mut app, 0).unwrap());
        }
        let st = app.session.active().unwrap();
        let r = super::damage(&app, st.doc.id, st.revision, frames[0].1, frames[1].1);
        let canvas = st.doc.bounds();
        let (a, b) = (photocraft_compose::render(&frames[0].0, canvas), photocraft_compose::render(&frames[1].0, canvas));
        let w = canvas.width() as usize;
        let mut changed = 0;
        for (i, (pa, pb)) in a.px.iter().zip(&b.px).enumerate() {
            if pa != pb {
                changed += 1;
                let (x, y) = (canvas.x0 + (i % w) as i32, canvas.y0 + (i / w) as i32);
                assert!(r.is_none_or(|r| r.contains(x, y)), "{depth}-bit: ({x}, {y}) changed outside the redrawn {r:?}");
            }
        }
        assert!(changed > 0, "{depth}-bit: the mask sliding over the small layer changes pixels");
    }
}

/// Dragging with an unlinked mask targeted moves the mask alone, shown live and committed;
/// with the mask linked (or the pixels targeted) the layer moves.
#[test]
fn dragging_with_an_unlinked_mask_targeted_moves_the_mask_alone() {
    for (linked, mask_target) in [(false, true), (false, false), (true, true)] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
        app.sync_views();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.session.execute("select.deselect", json!({})).unwrap();
        app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        app.session.execute("layer.layerMask.linked", json!({"linked": linked})).unwrap();
        app.ui.mask_target = mask_target;
        app.ui.tool = Tool::Move;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let m = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 16.0, pressure: 1.0 }, m);
        let shown = super::display_doc(&mut app, 0).unwrap().0;
        let id = app.session.active().unwrap().active_layer.unwrap();
        let layer_moved = |d: &photocraft_doc::Document| d.layer(id).and_then(|l| l.surface()).is_some_and(|s| s.pixel(28, 16)[3] > 0.5);
        let alone = !linked && mask_target;
        assert_eq!(layer_moved(&shown), !alone, "preview, linked {linked} mask target {mask_target}");
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 16.0 }, m);
        let d = &app.session.active().unwrap().doc;
        assert_eq!(layer_moved(d), !alone, "commit, linked {linked} mask target {mask_target}");
    }
}

/// Mid-drag on a painted layer: is the layer shown at the pointer?
fn live(outline: bool) -> bool {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.edit_prefs(|p| p.interface.show_bounding_box_when_dragging_layer = outline);
    app.ui.tool = Tool::Move;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 30.0, y: 20.0, pressure: 1.0 }, m);
    let shown = super::display_doc(&mut app, 0).is_some();
    assert_eq!(shown, super::showing(&app));
    shown
}

/// A layer past the pixel-memory limit isn't copied for each frame of a drag (#6114, #6163).
#[test]
fn layers_over_the_limit_are_not_followed_live() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let one_tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2) * 4;
    assert!(super::exceeds(&st.doc, &[id], one_tile - 1, false), "a tile of pixels is over a limit below its size");
    assert!(super::exceeds(&st.doc, &[id], one_tile * 4 - 1, false), "a shift can write four tiles for each stored one");
    assert!(!super::exceeds(&st.doc, &[id], u64::MAX, false), "nothing is over an unlimited drag");
    assert!(!super::exceeds(&st.doc, &[], 0, false), "no layers hold no pixels");
    assert!(!super::exceeds(&st.doc, &[id], super::LIVE_LIMIT, false), "an ordinary layer follows the pointer");
}

/// A drag whose layers are past the limit shows no live copy (the outline and arrow stay), and
/// the release still moves the layer once (#6114, #6163).
#[test]
fn a_drag_over_the_limit_shows_no_live_copy_and_still_commits() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    // Enough stored tiles, all sharing one allocation, to be past the limit before the drag starts.
    let step = photocraft_geom::TILE_SIZE;
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile_bytes = u64::from(step.unsigned_abs()).pow(2) * surface.format().bytes_per_pixel() as u64;
            let count = (super::LIVE_LIMIT / (tile_bytes * 2) + 2) as i32; // a row writes about (n + 1) x 2 tiles
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..count).map(|x| (photocraft_geom::TileCoord::new(x, 0), tile.clone())));
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    let to = 16.0 + f64::from(step); // a whole tile, so the release shares the tiles too
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: to, y: 16.0, pressure: 1.0 }, m);
    assert!(super::display_doc(&mut app, 0).is_none(), "a layer over the limit isn't copied for the frame");
    assert!(!super::showing(&app));
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: to, y: 16.0 }, m);
    let d = &app.session.active().unwrap().doc;
    assert!(d.layer(id).and_then(|l| l.surface()).is_some_and(|s| s.pixel(to as i32, 16)[3] > 0.5), "the release moved the layer");
}

/// What a live drag frame writes is the tiles the content can reach, not four per stored tile for
/// every layer: a dense photo stays live, isolated tiles (each straddles four) don't (#6114).
#[test]
fn the_estimate_counts_reachable_target_tiles_and_what_moved_target_copies() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let step = photocraft_geom::TILE_SIZE;
    let tile_bytes = u64::from(step.unsigned_abs()).pow(2) * 4;
    let tiles_over_limit = (super::LIVE_LIMIT / tile_bytes + 1) as i32;
    // A dense block of the same number of stored tiles as the sparse one below.
    let side = (f64::from(tiles_over_limit / 4)).sqrt() as i32;
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..side * side).map(|i| (photocraft_geom::TileCoord::new(i % side, i / side), tile.clone())));
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    assert!(!super::exceeds(&st.doc, &[id], super::LIVE_LIMIT, false), "a dense layer writes about as many tiles as it stores");
    // The same count of isolated tiles: each straddles four target tiles.
    let sparse = tiles_over_limit / 4 + 1;
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..sparse).map(|i| (photocraft_geom::TileCoord::new(i * 10, i * 10 + 1000), tile.clone())));
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    assert!(super::exceeds(&st.doc, &[id], super::LIVE_LIMIT, false), "isolated tiles each write up to four");
    // Only the mask is copied when an unlinked mask is dragged alone: the huge content isn't charged.
    app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    app.session.execute("layer.layerMask.linked", json!({"linked": false})).unwrap();
    let st = app.session.active().unwrap();
    assert!(super::exceeds(&st.doc, &[id], 1, false), "dragging the layer copies its content");
    assert!(!super::exceeds(&st.doc, &[id], super::LIVE_LIMIT / 4, true), "dragging the unlinked mask alone copies the mask only");
}

/// A group moves its contents whole; a group's unlinked mask dragged alone leaves them where they are.
#[test]
fn the_estimate_follows_a_group_into_its_contents() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let inner = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("paint", |doc, a| {
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let tile = surface.solid_tile(&[1.0, 0.0, 0.0, 1.0]);
            surface.put_tiles((0..600).map(|i| (photocraft_geom::TileCoord::new(i * 10, i * 10), tile.clone())));
            Ok(())
        })
        .unwrap();
    let group = app.session.execute("layer.new.group", json!({})).unwrap()["layer"].as_u64().unwrap();
    app.session.execute("layer.moveTo", json!({"layer": inner.0, "target": group, "position": "into"})).unwrap();
    let group = photocraft_doc::LayerId(group);
    let st = app.session.active().unwrap();
    assert!(super::exceeds(&st.doc, &[group], super::LIVE_LIMIT, false), "the group's contents are shifted with it");
    app.session.execute("layer.layerMask.revealAll", json!({"layer": group.0})).unwrap();
    app.session.execute("layer.layerMask.linked", json!({"layer": group.0, "linked": false})).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.doc.layer(group).is_some_and(|l| l.mask.as_ref().is_some_and(|m| !m.linked)), "the group has an unlinked mask");
    assert!(!super::exceeds(&st.doc, &[group], super::LIVE_LIMIT, true), "an unlinked group mask dragged alone leaves the contents");
}

/// A `width` × `height` `depth`-bit document whose new layer covers the canvas, one fill per tile
/// with its own colour (a photo: no two tiles share pixels), and the Move tool on.
fn photo(width: u32, height: u32, depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": width, "height": height, "depth": depth})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            let canvas = Rect::new(0, 0, width as i32, height as i32);
            let surface = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            for (i, c) in canvas.tiles().enumerate() {
                let v = (i % 251) as f32 / 251.0;
                surface.fill_rect(c.rect().intersect(&canvas), &[v, 1.0 - v, 0.5, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

/// Mid-drag by (37, 11): is the layer shown at the pointer?
fn follows_a_drag(app: &mut PhotocraftApp) -> bool {
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(app, ToolEvent::Down { x: 100.0, y: 100.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: 137.0, y: 111.0, pressure: 1.0 }, m);
    let shown = super::display_doc(app, 0).is_some();
    crate::canvas::tool_event(app, ToolEvent::Up { x: 137.0, y: 111.0 }, m);
    shown
}

/// Ordinary photos keep the live drag: a 36 MP 8-bit layer (580 stored tiles) and a 24 MP 16-bit
/// one write their tiles plus one edge row and column, not four tiles for each stored one.
#[test]
fn ordinary_photos_follow_the_pointer() {
    for (w, h, depth) in [(7360, 4912, 8), (6000, 4000, 16)] {
        let mut app = photo(w, h, depth);
        assert!(follows_a_drag(&mut app), "{w} × {h} {depth}-bit");
    }
}

/// A solid fill stores one tile shared by every covered position, and a shift shares it too (only
/// the edge is written), so a large filled layer drags live as it does without the limit.
#[test]
fn a_large_solid_layer_follows_the_pointer() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 12000, "height": 12000, "depth": 16})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.all", json!({})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    assert!(follows_a_drag(&mut app), "a filled 12000 × 12000 16-bit layer");
}

/// Content off the tile grid: the 2 × 2 blocks round its stored tiles reach a column and a row
/// past what a shift writes, which is at most the tiles the content can reach.
#[test]
fn the_estimate_is_capped_by_what_the_content_reaches() {
    let format = photocraft_color::PixelFormat::RGBA8;
    let mut s = photocraft_raster::Surface::new(format);
    s.fill_rect(Rect::new(128, 128, 384, 384), &[1.0, 0.0, 0.0, 1.0]);
    let tile = u64::from(photocraft_geom::TILE_SIZE.unsigned_abs()).pow(2) * format.bytes_per_pixel() as u64;
    assert_eq!(s.tile_count(), 4, "a tile-sized square straddles four tiles");
    assert_eq!(super::shift_bytes(&s), 4 * tile, "it reaches (1 + 1) × (1 + 1) target tiles, not the nine blocks round its tiles");
}

/// `shift_smart` shifts a smart object's filter mask with it: the estimate counts it.
#[test]
fn the_estimate_counts_a_smart_filter_mask() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent"})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    assert!(!super::exceeds(&app.session.active().unwrap().doc, &[id], super::LIVE_LIMIT, false), "a small smart object");
    app.session
        .edit("paint", |doc, a| {
            let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layer_mut(a.unwrap()).unwrap().content else { panic!("not a smart object") };
            let mut mask = photocraft_doc::LayerMask::hide_all();
            // Isolated tiles, each written as four by a shift: past the limit at one byte a pixel.
            let tile = mask.surface.solid_tile(&[1.0]);
            let count = (super::LIVE_LIMIT / (1 << 16) / 4 + 1) as i32;
            mask.surface.put_tiles((0..count).map(|i| (photocraft_geom::TileCoord::new(i * 10, 0), tile.clone())));
            sm.filter_mask = Some(mask);
            Ok(())
        })
        .unwrap();
    assert!(super::exceeds(&app.session.active().unwrap().doc, &[id], super::LIVE_LIMIT, false), "the filter mask moves with it");
}

#[test]
fn bounding_box_preference_turns_the_live_drag_off() {
    assert!(live(false), "by default the layer follows the pointer");
    assert!(!live(true), "with the preference on, only the outline and arrow move");
}

/// A `depth`-bit transparent document whose layer holds a red square (8..24)² and a blue one
/// (40..56)×(8..24), the red one selected; the Move tool with Auto-Select on.
fn selected_pixels(depth: u32) -> (PhotocraftApp, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64, "background": "transparent", "depth": depth})).unwrap();
    app.sync_views();
    for (x, color) in [(40, "#0000ff"), (8, "#ff0000")] {
        app.session.execute("select.rect", json!({"x": x, "y": 8, "width": 16, "height": 16})).unwrap();
        app.session.execute("edit.fill", json!({"color": color})).unwrap();
    }
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let id = app.session.active().unwrap().active_layer.unwrap();
    (app, id)
}

fn alpha(app: &PhotocraftApp, id: photocraft_doc::LayerId, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(x, y)[3]
}

fn offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    photocraft_engine::float_cmds::floating(app.session.active().unwrap()).map(|f| f.offset)
}

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// ↩ through the app's key handling.
fn enter(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let input = egui::RawInput { events: vec![key], ..Default::default() };
    ctx.run_ui(input, |ui| crate::shortcuts::handle(app, ui.ctx())).textures_delta.clear();
}

/// #578: with a selection the Move tool drags the selected pixels (from anywhere, without an
/// Auto-Select pick), not the layer; ↩ drops them as one "Move" step that one undo takes back.
#[test]
fn with_a_selection_the_move_tool_moves_only_the_selected_pixels() {
    for depth in [8, 16, 32] {
        let (mut app, id) = selected_pixels(depth);
        let steps = app.session.active().unwrap().history.past_len();
        drag(&mut app, [50.0, 40.0], [60.0, 45.0], egui::Modifiers::NONE);
        assert_eq!(offset(&app), Some((10, 5)), "{depth}-bit: floating");
        let shown = super::display_doc(&mut app, 0).unwrap().0;
        let shown_alpha = |x, y| shown.layer(id).unwrap().surface().unwrap().rgba(x, y)[3];
        assert!(shown_alpha(10, 10) == 0.0 && shown_alpha(30, 25) == 1.0, "{depth}-bit: shown moved");
        enter(&mut app);
        assert_eq!(offset(&app), None, "{depth}-bit: ↩ dropped it");
        assert_eq!(alpha(&app, id, 10, 10), 0.0, "{depth}-bit: the hole is transparent");
        assert_eq!(alpha(&app, id, 30, 25), 1.0, "{depth}-bit: the red square moved");
        assert_eq!(alpha(&app, id, 48, 16), 1.0, "{depth}-bit: the unselected blue one stayed");
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(18, 13, 34, 29), "{depth}-bit");
        assert_eq!((st.history.past_len(), st.history.undo_label()), (steps + 1, Some("Move")), "{depth}-bit");
        assert!(app.session.undo());
        assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 30, 25) == 0.0, "{depth}-bit: undone");
    }
}

/// ⌥ copies the selected pixels (no layer copy), ⇧ locks the drag to multiples of 45°.
#[test]
fn alt_copies_and_shift_constrains_the_selected_pixels() {
    for depth in [8, 16, 32] {
        let (mut app, id) = selected_pixels(depth);
        let layers = app.session.active().unwrap().doc.layers.len();
        drag(&mut app, [16.0, 16.0], [30.0, 28.0], egui::Modifiers::ALT | egui::Modifiers::SHIFT);
        assert_eq!(offset(&app), Some((13, 13)), "{depth}-bit: 45°");
        app.run("select.drop", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "{depth}-bit: no layer copy");
        assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 35, 35) == 1.0, "{depth}-bit: original and copy");
        assert!(app.session.undo());
        assert_eq!(alpha(&app, id, 35, 35), 0.0, "{depth}-bit: undone");
    }
}

/// #7538: a loose marquee round the red square shrinks to it as the drag starts: the canvas
/// outlines the moved pixels while they float, and the dropped selection is theirs. Alt-drag
/// (a copy) too.
#[test]
fn dragging_selected_pixels_shrinks_the_ants_to_them() {
    for depth in [8, 16, 32] {
        for m in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
            let (mut app, _) = selected_pixels(depth);
            app.session.execute("select.rect", json!({"x": 2, "y": 2, "width": 30, "height": 30})).unwrap();
            drag(&mut app, [16.0, 16.0], [26.0, 21.0], m);
            assert_eq!(offset(&app), Some((10, 5)), "{depth}-bit, {m:?}: floating");
            let ants = crate::canvas::floating_outline(&app, 0).map(|s| s.content_bounds());
            assert_eq!(ants, Some(Rect::new(8, 8, 24, 24)), "{depth}-bit, {m:?}: the ants hug the red square");
            assert!(crate::canvas::floating_outline(&app, 1).is_none(), "only the active document's");
            app.run("select.drop", json!({})).unwrap();
            let sel = app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
            assert_eq!(sel, Some(Rect::new(18, 13, 34, 29)), "{depth}-bit, {m:?}: dropped");
            assert!(crate::canvas::floating_outline(&app, 0).is_none(), "nothing floats");
        }
    }
}

/// ⌘-drag with a painting tool drags the selected pixels as the Move tool does, also when ⌘
/// comes with the event (control channel, MCP) rather than as the held-key override.
#[test]
fn command_drag_with_a_painting_tool_moves_only_the_selected_pixels() {
    let (mut app, id) = selected_pixels(8);
    app.ui.tool = Tool::Brush;
    app.ui.tool_options.move_auto_select = false;
    let cmd = egui::Modifiers { mac_cmd: true, command: true, ..Default::default() };
    drag(&mut app, [50.0, 40.0], [60.0, 45.0], cmd);
    assert_eq!(offset(&app), Some((10, 5)), "the selected pixels float");
    drag(&mut app, [50.0, 40.0], [60.0, 45.0], cmd);
    assert_eq!(offset(&app), Some((20, 10)), "a second drag moves the same floating piece");
    app.run("select.drop", json!({})).unwrap();
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 40, 30) == 1.0, "the red square moved");
    assert_eq!(alpha(&app, id, 48, 16), 1.0, "the unselected blue one stayed");
}

/// A click moves nothing and leaves nothing floating (Undo isn't spent putting it back).
#[test]
fn a_click_with_a_selection_moves_nothing() {
    let (mut app, id) = selected_pixels(8);
    let steps = app.session.active().unwrap().history.past_len();
    drag(&mut app, [16.0, 16.0], [16.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(offset(&app), None);
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 48, 16) == 1.0);
}

#[test]
fn arrow_keys_nudge_the_selected_pixels() {
    let (mut app, id) = selected_pixels(16);
    crate::move_mods::nudge(&mut app, 10.0, 0.0, false);
    crate::move_mods::nudge(&mut app, 0.0, 1.0, false);
    assert_eq!(offset(&app), Some((10, 1)));
    // ⌥ leaves the piece where it is and nudges a copy of it.
    crate::move_mods::nudge(&mut app, 1.0, 0.0, true);
    assert_eq!(offset(&app), Some((1, 0)));
    app.run("select.drop", json!({})).unwrap();
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 18, 20) == 1.0 && alpha(&app, id, 34, 20) == 1.0);
    assert_eq!(alpha(&app, id, 48, 16), 1.0, "the layer itself didn't move");
}

/// Without floatable pixels (a position-locked layer) the Move tool behaves as without a
/// selection: it moves the layer, which the lock refuses.
#[test]
fn a_layer_that_cannot_float_moves_whole_as_before() {
    let (mut app, id) = selected_pixels(8);
    app.session.execute("layer.lockLayers", json!({"position": true})).unwrap();
    drag(&mut app, [16.0, 16.0], [26.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(offset(&app), None);
    assert!(alpha(&app, id, 10, 10) == 1.0 && alpha(&app, id, 30, 16) == 0.0, "nothing moved");
    // Without a selection the layer moves whole.
    app.session.execute("layer.lockLayers", json!({"position": false})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    drag(&mut app, [16.0, 16.0], [26.0, 16.0], egui::Modifiers::NONE);
    assert!(alpha(&app, id, 10, 10) == 0.0 && alpha(&app, id, 58, 16) == 1.0, "the whole layer moved");
}

#[test]
fn floating_selection_does_not_snap_to_its_own_starting_outline_or_preview_at_zero() {
    let (mut app, _id) = selected_pixels(8);
    app.ui.extras.snap = true;
    app.ui.view.snap_to.document_bounds = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    assert!(super::display_doc(&mut app, 0).is_none(), "no preview composite at offset (0, 0) before the pointer moves");
    // A small 2 px move within the snap radius must move immediately instead of snapping back to (0, 0).
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 18.0, y: 17.0, pressure: 1.0 }, m);
    assert_eq!(super::floating_offset(&app), Some((2, 1)), "small drag moves without snapping to own selection bounds");
    assert!(super::display_doc(&mut app, 0).is_some(), "preview is shown once offset is non-zero");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 18.0, y: 17.0 }, m);
    assert_eq!(offset(&app), Some((2, 1)));
}

/// Board "A" at (10, 10, 50, 50) holding a red square at (10..30)², and an empty board "B" at
/// (200, 20, 240, 60); the red layer selected, no selection, the Move tool without Auto-Select.
fn two_boards() -> (PhotocraftApp, photocraft_doc::LayerId, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 300, "height": 100, "background": "transparent"})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    let red = app.session.active().unwrap().active_layer.unwrap();
    let id = |v: serde_json::Value| photocraft_doc::LayerId(v["layer"].as_u64().unwrap());
    let a = id(app.session.execute("layer.new.artboardFromLayers", json!({"name": "A"})).unwrap());
    app.session.execute("layer.artboard.set", json!({"width": 40, "height": 40})).unwrap();
    let b = id(app.session.execute("layer.new.artboard", json!({"rect": [200, 20, 40, 40], "name": "B"})).unwrap());
    app.session.execute("layer.select", json!({"layer": red.0})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    (app, red, a, b)
}

/// #10218: a layer dragged with the Move tool from one artboard onto another joins that board
/// (Photoshop's auto-nesting), so it shows there instead of staying in its old board, clipped away.
/// ⌥-dragging does the same for the copy and leaves the original where it was.
#[test]
fn move_tool_drag_onto_another_artboard_nests_the_layer_there() {
    let (mut app, red, a, b) = two_boards();
    let board = |app: &PhotocraftApp, id| app.session.active().unwrap().doc.artboard_of(id);
    drag(&mut app, [20.0, 20.0], [210.0, 30.0], egui::Modifiers::NONE);
    assert_eq!(board(&app, red), Some(b), "the layer joined board B");
    // ⌥-drag it back onto A: the copy joins A, the original stays on B.
    drag(&mut app, [210.0, 30.0], [20.0, 20.0], egui::Modifiers::ALT);
    let st = app.session.active().unwrap();
    let copy = st.active_layer.unwrap();
    assert_ne!(copy, red, "the copy is active");
    assert_eq!(board(&app, copy), Some(a), "the copy joined board A");
    assert_eq!(board(&app, red), Some(b), "the original stayed on board B");
}

/// A document with at least [`super::MANY_LAYERS`] layers, `side` px square.
fn many_layers(side: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": side, "height": side})).unwrap();
    app.sync_views();
    while app.session.active().unwrap().doc.walk().len() < super::MANY_LAYERS {
        app.session.execute("layer.new.layer", json!({})).unwrap();
    }
    app
}

fn costly(app: &PhotocraftApp, bounds: Option<Rect>) -> bool {
    super::costly_to_redraw(&app.session.active().unwrap().doc, bounds)
}

/// A document costly to drag in, the Move tool picked: many layers, and on the active one two
/// bands, top and bottom, between which the white background shows unless it's hidden.
fn banded() -> PhotocraftApp {
    let mut app = many_layers(4096);
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            s.fill_rect(Rect::new(0, 0, 4096, 128), &[1.0, 0.0, 0.0, 1.0]);
            s.fill_rect(Rect::new(0, 3968, 4096, 4096), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app
}

/// Starts a costly drag of the band layer.
fn start_costly_drag(app: &mut PhotocraftApp) {
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(app, ToolEvent::Down { x: 2048.0, y: 64.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Move { x: 2148.0, y: 94.0, pressure: 1.0 }, m);
    let _ = super::display_doc(app, 0);
}

/// Pattern Preview tiles the canvas, which one picture can't follow: a costly drag there keeps its
/// outline and arrow, and the layers stay in every copy.
#[test]
fn a_costly_drag_under_pattern_preview_keeps_its_outline() {
    let mut app = banded();
    app.ui.view.pattern_preview = true;
    start_costly_drag(&mut app);
    assert!(app.move_preview.as_ref().is_some_and(|p| p.too_big), "too costly to follow live");
    assert!(!super::snapshot_drag(&app), "no snapshot: the outline and arrow");
}

/// One snapshot renders at a time: a costly drag started while another's render still runs keeps
/// its outline, and once that render is done the next drag gets its snapshot again.
// Native only: the web build keeps the outline (no thread to render a snapshot on).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_costly_drag_while_a_snapshot_renders_keeps_its_outline() {
    let mut app = banded();
    let held = super::SnapshotSlot::take().expect("no render running");
    start_costly_drag(&mut app);
    assert!(!super::snapshot_drag(&app), "another render runs: the outline and arrow");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 2148.0, y: 94.0 }, egui::Modifiers::NONE);
    drop(held);
    app.session.execute("edit.undo", json!({})).unwrap();
    start_costly_drag(&mut app);
    assert!(super::snapshot_drag(&app), "free again: a snapshot");
}

/// The picture arrives as straight alpha, converted for the canvas display like the canvas itself:
/// a half-opaque red texel uploads as half-bright premultiplied red, not full red over half alpha.
// Native only: the web build keeps the outline (no thread to render a snapshot on).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_snapshot_uploads_straight_alpha_the_way_the_canvas_does() {
    let mut app = banded();
    start_costly_drag(&mut app);
    let (tx, rx) = std::sync::mpsc::channel();
    app.move_preview.as_mut().and_then(|p| p.snapshot.as_mut()).expect("a snapshot").rx = Some(rx);
    let mut px = photocraft_raster::scratch::take(1, [0.0; 4]);
    px[0] = [1.0, 0.0, 0.0, 0.5];
    tx.send(photocraft_compose::Buffer { rect: Rect::new(0, 0, 1, 1), px }).unwrap();
    let ctx = egui::Context::default();
    let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 800.0)),
        zoom: 0.1,
        center: [2048.0, 2048.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    super::draw_snapshot(&mut app, &painter, &xf);
    let mut delta = ctx.tex_manager().write().take_delta();
    let texel = delta
        .set
        .values()
        .flatten()
        .find_map(|d| {
            let egui::ImageData::Color(img) = &d.image;
            (img.size == [1, 1]).then(|| img.pixels[0])
        })
        .expect("the snapshot uploaded");
    // Handled here, as a renderer would.
    delta.clear();
    assert!(texel.a().abs_diff(128) <= 1 && texel.r().abs_diff(128) <= 1, "premultiplied half red: {texel:?}");
}

/// Only a drag over a large area of a document with many layers is too costly to redraw live:
/// a storyboard's photo (#12841), not an ordinary photo or a small layer.
#[test]
fn only_large_moves_in_documents_with_many_layers_are_too_costly_to_redraw_live() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 4096, "height": 4096})).unwrap();
    app.sync_views();
    let full = Rect::new(0, 0, 4096, 4096);
    assert!(!costly(&app, Some(full)), "few layers");
    let app = many_layers(4096);
    assert!(costly(&app, Some(full)), "16 MP under many layers");
    assert!(!costly(&app, Some(Rect::new(0, 0, 4096, 4095))), "under 16 MP");
    assert!(!costly(&app, Some(Rect::new(-8192, 0, 1, 4096))), "the area on the canvas counts");
    assert!(costly(&app, None), "an unbounded layer");
}

/// A costly drag shows a reduced picture of the moving layers alone at the pointer, over the
/// canvas with them hidden; the drop redraws only where they landed, not where they were too.
// Native only: the web build keeps the outline (no thread to render a snapshot on).
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_costly_drag_shows_a_snapshot_and_its_drop_redraws_where_the_layers_land() {
    let mut app = banded();
    let moving = app.session.active().unwrap().active_layer.unwrap();
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 2048.0, y: 64.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 2148.0, y: 94.0, pressure: 1.0 }, m);
    assert!(super::display_doc(&mut app, 0).is_none(), "the canvas is unchanged until the snapshot arrives");
    assert!(super::snapshot_drag(&app), "no arrow: a snapshot is on its way");

    let s = app.move_preview.as_mut().and_then(|p| p.snapshot.as_mut()).expect("a snapshot");
    let rect = s.rect;
    let buf = s.rx.take().unwrap().recv_timeout(std::time::Duration::from_secs(120)).expect("the snapshot render");
    let (w, h) = (buf.rect.width() as usize, buf.rect.height() as usize);
    assert!(w <= super::SNAPSHOT_SIDE as usize && h <= super::SNAPSHOT_SIDE as usize, "reduced: {w}×{h}");
    let at = |x: i32, y: i32| {
        let bx = (x - rect.x0) as usize * w / rect.width() as usize;
        let by = (y - rect.y0) as usize * h / rect.height() as usize;
        buf.px[by * w + bx]
    };
    assert!(at(2048, 32)[3] > 0.99, "the moving layer's band");
    assert!(at(2048, 2048)[3] < 0.01, "the background and the other layers aren't in it");
    // Arrived: the next frame hides the layers under it.
    s.picture = Some(buf);

    let (shown, key) = super::display_doc(&mut app, 0).expect("the canvas with the moving layers hidden");
    assert!(super::is_preview_key(key));
    assert!(!shown.layer(moving).unwrap().visible, "hidden while its snapshot follows the pointer");

    let before = app.session.active().unwrap().revision;
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 2148.0, y: 94.0 }, m);
    let st = app.session.active().unwrap();
    assert_eq!(st.revision, before + 1, "one move");
    let (hidden, landed) = app.move_drop.as_ref().and_then(|d| d.landed).expect("a snapshot drop");
    assert_eq!(hidden, key);
    let canvas = Rect::new(0, 0, 4096, 4096);
    assert_eq!(landed, rect.translate(100, 30).inflate(1).intersect(&canvas), "where the layers landed");
    // The move's own damage: where the layers were and are.
    let both = Some(canvas);
    let r = crate::canvas::textures::damage_since(&app, 0, (before, key), (st.revision, 0), 0, (before, both));
    let reach = crate::canvas::textures::effect_reach(&st.doc.layers);
    assert_eq!(r, Some(landed.inflate(reach)));
}

/// A release after the canvas showed one of the drag's previews, but not its final offset,
/// redraws the move's own damage and where that preview drew the layers, not the whole canvas:
/// the pointer went past both places before coming back.
#[test]
fn a_release_after_an_overshooting_preview_redraws_the_move_and_that_preview() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 128, "height": 128})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    app.session.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.session.execute("select.deselect", json!({})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 16.0, y: 16.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 96.0, y: 96.0, pressure: 1.0 }, m);
    let (_, key) = super::display_doc(&mut app, 0).expect("a live drag");
    assert!(super::is_preview_key(key));
    let before = app.session.active().unwrap().revision;
    // Released at (20, 9), far from the (80, 80) the canvas showed.
    super::finish(&mut app, 20.0, 9.0);
    let st = app.session.active().unwrap();
    // Where the layer was and is.
    let moved = Rect::new(8, 8, 44, 33);
    let r = crate::canvas::textures::damage_since(&app, 0, (before, key), (st.revision, 0), 0, (before, Some(moved))).expect("bounded");
    assert_eq!(r.union(&moved), r, "where the layer was and is");
    assert_eq!(r.union(&Rect::new(90, 90, 102, 102)), r, "where the preview drew it");
    assert_ne!(r, Rect::new(0, 0, 128, 128), "not the whole canvas");
}

/// A snapshot shows only what lies on the canvas, sampling the matching part of its texture.
#[test]
fn a_snapshot_is_cut_at_the_canvas() {
    let canvas = Rect::new(0, 0, 50, 50);
    let (r, uv) = super::snapshot_quad(Rect::new(-100, 0, 100, 100), canvas).unwrap();
    assert_eq!(r, canvas);
    assert_eq!(uv, [egui::pos2(0.5, 0.0), egui::pos2(0.75, 0.0), egui::pos2(0.75, 0.5), egui::pos2(0.5, 0.5)]);
    assert!(super::snapshot_quad(Rect::new(60, 0, 80, 20), canvas).is_none(), "off the canvas");
}

/// View › Pattern Preview: dragging a block off the right edge shows it coming in on the left,
/// and the release commits that wrap (#10293).
#[test]
fn pattern_preview_drag_wraps_pixels_across_the_canvas_edge() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 32, "height": 16, "background": "transparent"})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(28, 4, 32, 12), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.view.pattern_preview = true;
    app.ui.tool = Tool::Move;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    let m = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 8.0, pressure: 1.0 }, m);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 36.0, y: 8.0, pressure: 1.0 }, m);
    let id = app.session.active().unwrap().active_layer.unwrap();
    let blue = |d: &photocraft_doc::Document, x: i32| d.layer(id).and_then(|l| l.surface()).is_some_and(|s| s.pixel(x, 6)[2] > 0.9);
    let (shown, wrapped_key) = super::display_doc(&mut app, 0).expect("a wrapped preview");
    let st = app.session.active().unwrap();
    let damage = super::damage(&app, st.doc.id, st.revision, 0, wrapped_key);
    assert!(damage.is_none_or(|r| r.contains(2, 6) && r.contains(30, 6)), "damage covers both changed edges");
    assert!(blue(&shown, 2), "the preview brings the block in on the left");
    assert!(!blue(&shown, 30), "the preview clears where the block was");
    app.ui.view.pattern_preview = false;
    let (shown, raw_key) = super::display_doc(&mut app, 0).expect("a rebuilt preview");
    assert_ne!(wrapped_key, raw_key, "the rebuilt preview does not reuse the wrapped cache key");
    let st = app.session.active().unwrap();
    assert!(super::damage(&app, st.doc.id, st.revision, wrapped_key, raw_key).is_none(), "old-mode keys force a full redraw");
    assert!(!blue(&shown, 2), "turning Pattern Preview off mid-drag stops wrapping");
    assert!(blue(&shown, 34), "the block sits at the raw offset");
    app.ui.view.pattern_preview = true;
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 36.0, y: 8.0 }, m);
    let committed = &app.session.active().unwrap().doc;
    assert!(blue(committed, 2) && !blue(committed, 30), "release commits the wrap");
}

/// Two views on monitors with different profiles each show the snapshot in their own colours,
/// and a profile changed mid-drag converts it again: the picture is kept, and each display gets a
/// texture of its own.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn each_display_converts_the_snapshot_itself() {
    let mut app = banded();
    start_costly_drag(&mut app);
    let (tx, rx) = std::sync::mpsc::channel();
    app.move_preview.as_mut().and_then(|p| p.snapshot.as_mut()).expect("a snapshot").rx = Some(rx);
    let mut px = photocraft_raster::scratch::take(1, [0.0; 4]);
    px[0] = [1.0, 0.0, 0.0, 1.0];
    tx.send(photocraft_compose::Buffer { rect: Rect::new(0, 0, 1, 1), px }).unwrap();
    let ctx = egui::Context::default();
    let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 800.0)),
        zoom: 0.1,
        center: [2048.0, 2048.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let keys = |app: &PhotocraftApp| {
        app.move_preview.as_ref().and_then(|p| p.snapshot.as_ref()).map(|s| s.textures.iter().map(|(k, _)| *k).collect::<Vec<_>>()).unwrap()
    };
    super::draw_snapshot(&mut app, &painter, &xf);
    let first = keys(&app);
    assert_eq!(first.len(), 1, "the first display's texture");
    app.session.color.settings.monitor_profile = "display-p3".into();
    super::draw_snapshot(&mut app, &painter, &xf);
    let both = keys(&app);
    assert!(both.len() == 2 && both[1] != first[0], "the P3 monitor converts its own: {both:?}");
    assert!(app.move_preview.as_ref().and_then(|p| p.snapshot.as_ref()).is_some_and(|s| s.picture.is_some()), "the picture is kept for the next display");
    ctx.tex_manager().write().take_delta().clear();
}
