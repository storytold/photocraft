use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{
    Document, Layer, LayerContent,
    sprite::{SpriteCel, SpriteFrame, SpriteLayer},
};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;

fn add_to_session(doc: Document) -> Session {
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

fn session() -> Session {
    let mut doc = Document::new("Animation", Size::new(2, 1), ColorMode::Rgb, SampleType::U8);
    let mut surface = Surface::new(PixelFormat::RGBA8);
    surface.write_interleaved(Rect::from_xywh(0, 0, 1, 1), &[255, 0, 0, 255]);
    let layer = Layer::new("Ink", LayerContent::Raster(surface.clone()));
    let id = layer.id;
    doc.layers.push(layer);
    let first =
        Arc::new(SpriteCel { rect: Rect::from_xywh(0, 0, 1, 1), pixels: surface, indices: None, opacity: 255, z_index: 0, linked_frame: None, precise: None });
    let second = Arc::new(SpriteCel { linked_frame: Some(0), ..first.as_ref().clone() });
    doc.sprite = Some(Sprite {
        frames: vec![
            Arc::new(SpriteFrame { duration_ms: 17, palette: Arc::new(vec![]), cels: [(id, first)].into(), metadata: vec![] }),
            Arc::new(SpriteFrame { duration_ms: 83, palette: Arc::new(vec![]), cels: [(id, second)].into(), metadata: vec![] }),
        ],
        current: 0,
        layers: vec![SpriteLayer { id, flags: 3, opacity: 255, uuid: None }],
        indexed: false,
        transparent_index: 0,
        pixel_aspect: [1, 1],
        grid: [0; 4],
        tags: vec![],
        profile: None,
    });
    doc.timeline = Some(photocraft_doc::Timeline::new(2, 10.0));
    add_to_session(doc)
}

fn in_group(mut doc: Document, all: bool, position: bool) -> Document {
    let children = std::mem::take(&mut doc.layers);
    let mut group = Layer::group("Parent", children);
    group.locks.all = all;
    group.locks.position = position;
    doc.sprite.as_mut().unwrap().layers.insert(0, SpriteLayer { id: group.id, flags: 3, opacity: 255, uuid: None });
    doc.layers.push(group);
    doc
}

fn assert_refused_unchanged(s: &mut Session, command: &str, params: Value) {
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let (history, revision, active) = (state.history.past_len(), state.revision, state.active_layer);
    assert!(s.execute(command, params).is_err(), "{command}");
    let state = s.active().unwrap();
    assert_eq!(state.doc, before, "{command}: document");
    assert_eq!(state.history.past_len(), history, "{command}: history");
    assert_eq!(state.revision, revision, "{command}: revision");
    assert_eq!(state.active_layer, active, "{command}: target");
}

#[test]
fn sprite_commands_are_disabled_without_a_sprite_or_with_unsupported_samples() {
    let mut empty = Session::new();
    let mut ordinary = add_to_session(Document::new("Ordinary", Size::new(2, 1), ColorMode::Rgb, SampleType::U8));
    for spec in specs() {
        for s in [&mut empty, &mut ordinary] {
            assert!(s.disabled_reason(spec.id).is_some(), "{}", spec.id);
            assert!(s.execute(spec.id, json!({})).is_err(), "{}", spec.id);
        }
    }
    assert_eq!(ordinary.active().unwrap().history.past_len(), 0);
    for depth in [SampleType::U16, SampleType::F32] {
        let mut s = session();
        Arc::make_mut(&mut s.active_mut().unwrap().doc).depth = depth;
        for spec in specs() {
            assert!(s.disabled_reason(spec.id).unwrap().contains("8-bit"), "{}", spec.id);
            assert_refused_unchanged(&mut s, spec.id, json!({}));
        }
    }
}

#[test]
fn every_sprite_edit_adds_one_history_step_and_roundtrips_undo_redo() {
    for command in [
        "sprite.setDuration",
        "sprite.setPaletteEntry",
        "sprite.setCel",
        "sprite.unlinkCel",
        "sprite.setTags",
        "sprite.duplicateFrame",
        "sprite.deleteFrame",
        "sprite.detach",
        "sprite.rasterizeTilemap",
    ] {
        let mut s = if command == "sprite.rasterizeTilemap" {
            add_to_session(photocraft_io::import("tiles.aseprite", include_bytes!("../../../../io/tests/fixtures/aseprite/tiles.aseprite")).unwrap().document)
        } else {
            session()
        };
        if command == "sprite.unlinkCel" {
            s.execute("timeline.setFrame", json!({"frame":1})).unwrap();
        }
        let id = s.active().unwrap().doc.layers[0].id.0;
        let params = match command {
            "sprite.setDuration" => json!({"durationMs":47}),
            "sprite.setPaletteEntry" => json!({"index":0,"rgba":[12,34,56,255],"name":"Original"}),
            "sprite.setCel" => json!({"layer":id,"x":-2,"opacity":127,"zIndex":-1}),
            "sprite.unlinkCel" | "sprite.rasterizeTilemap" => json!({"layer":id}),
            "sprite.setTags" => json!({"tags":[{"name":"Walk","from":0,"to":1,"direction":2,"repeat":2,"color":[1,2,3]}]}),
            _ => json!({}),
        };
        let before = s.active().unwrap().doc.clone();
        let n = s.active().unwrap().history.past_len();
        s.execute(command, params).unwrap();
        let after = s.active().unwrap().doc.clone();
        assert_ne!(before, after, "{command}");
        assert_eq!(s.active().unwrap().history.past_len(), n + 1, "{command}");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before, "{command}: undo");
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, after, "{command}: redo");
    }
}

#[test]
fn frame_delay_gestures_coalesce_separately_and_queries_add_no_history() {
    let mut s = session();
    for (gesture, values) in [("first", [47, 63, 80]), ("second", [90, 100, 110])] {
        for ms in values {
            s.execute("sprite.setDuration", json!({"durationMs":ms,"coalesce":gesture})).unwrap();
        }
    }
    assert_eq!(s.active().unwrap().history.past_len(), 2);
    s.execute("sprite.info", json!({})).unwrap();
    s.execute("sprite.playback", json!({"elapsedMs":12})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), 2);
    for (command, ms) in [("edit.undo", 80), ("edit.undo", 17), ("edit.redo", 80), ("edit.redo", 110)] {
        s.execute(command, json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.sprite.as_ref().unwrap().frame_duration_ms(), ms);
    }
}

#[test]
fn cel_edits_respect_target_and_inherited_locks_without_mutating_history() {
    for inherited in [false, true] {
        let original = session().active().unwrap().doc.as_ref().clone();
        let id = original.layers[0].id;
        let mut locked = original.clone();
        if inherited {
            locked = in_group(locked, true, false);
        } else {
            locked.layers[0].locks.all = true;
        }
        let mut s = add_to_session(locked);
        for command in ["sprite.setCel", "sprite.unlinkCel"] {
            assert!(s.disabled_reason_with(command, &json!({"layer":id.0})).is_some());
            assert_refused_unchanged(&mut s, command, json!({"layer":id.0,"x":2,"opacity":127}));
        }
        let mut position = original;
        if inherited {
            position = in_group(position, false, true);
        } else {
            position.layers[0].locks.position = true;
        }
        let mut s = add_to_session(position);
        for field in ["x", "y", "zIndex"] {
            let mut params = json!({"layer":id.0});
            params[field] = json!(2);
            assert_refused_unchanged(&mut s, "sprite.setCel", params);
        }
        // Position locks do not prevent opacity edits. The explicit child target works
        // even when the group's row, rather than that child, is active.
        s.execute("sprite.setCel", json!({"layer":id.0,"opacity":127})).unwrap();
        assert_eq!(s.active().unwrap().doc.sprite.as_ref().unwrap().frames[0].cels[&id].opacity, 127);
    }
}

#[test]
fn rasterization_is_disabled_in_locked_groups_but_explicitly_unlocks_its_own_cache() {
    let doc = photocraft_io::import("tiles.aseprite", include_bytes!("../../../../io/tests/fixtures/aseprite/tiles.aseprite")).unwrap().document;
    let id = doc.layers[0].id;
    let mut s = add_to_session(in_group(doc, true, false));
    assert!(s.disabled_reason_with("sprite.rasterizeTilemap", &json!({"layer":id.0})).is_some());
    assert_refused_unchanged(&mut s, "sprite.rasterizeTilemap", json!({"layer":id.0}));
    s.edit("Unlock Group", |doc, _| {
        doc.layers[0].locks.all = false;
        Ok(())
    })
    .unwrap();
    s.execute("sprite.rasterizeTilemap", json!({"layer":id.0})).unwrap();
    assert!(!s.active().unwrap().doc.layer(id).unwrap().locks.all);
}

#[test]
fn sprite_edits_and_frame_navigation_keep_cels_and_undo() {
    let mut s = session();
    assert_eq!(s.execute("sprite.info", json!({})).unwrap()["durationsMs"], json!([17, 83]));
    s.execute("sprite.setDuration", json!({"durationMs":47})).unwrap();
    let n = s.active().unwrap().history.past_len();
    s.execute("timeline.setFrame", json!({"frame":1})).unwrap();
    assert_eq!(s.active().unwrap().history.past_len(), n, "navigation is not a history step");
    let id = s.active().unwrap().doc.layers[0].id.0;
    s.execute("sprite.setCel", json!({"layer":id,"x":-2,"y":1,"opacity":127,"zIndex":-3})).unwrap();
    let frame = &s.active().unwrap().doc.sprite.as_ref().unwrap().frames[1];
    assert_eq!(frame.cels[&LayerId(id)].rect, Rect::from_xywh(-2, 1, 1, 1));
    s.execute("sprite.unlinkCel", json!({"layer":id})).unwrap();
    assert!(s.active().unwrap().doc.sprite.as_ref().unwrap().frames[1].cels[&LayerId(id)].linked_frame.is_none());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.sprite.as_ref().unwrap().frames[1].cels[&LayerId(id)].linked_frame, Some(0));
    s.execute("sprite.duplicateFrame", json!({})).unwrap();
    assert_eq!(s.execute("sprite.info", json!({})).unwrap()["frames"], 3);
    s.execute("sprite.setTags", json!({"tags":[{"name":"Walk","from":0,"to":2,"direction":2,"repeat":3,"color":[1,2,3]}]})).unwrap();
    assert_eq!(s.execute("sprite.info", json!({})).unwrap()["tags"][0]["repeat"], 3);
    s.execute("sprite.deleteFrame", json!({})).unwrap();
    assert_eq!(s.execute("sprite.info", json!({})).unwrap()["frames"], 2);
    s.execute("sprite.detach", json!({})).unwrap();
    assert!(s.active().unwrap().doc.sprite.is_none());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.sprite.is_some());
}

#[test]
fn malformed_sprite_commands_never_modify_the_document() {
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    for (cmd, p) in [
        ("sprite.setDuration", json!({"durationMs":65536})),
        ("sprite.setDuration", json!({"durationMs":17,"frame":u64::MAX})),
        ("sprite.setPaletteEntry", json!({"index":u64::MAX,"rgba":[0,0,0,0]})),
        ("sprite.setPaletteEntry", json!({"index":0,"rgba":[0,-1,0,0]})),
        ("sprite.setCel", json!({"layer":0,"x":u64::MAX})),
        ("sprite.setTags", json!({"tags":[{"name":"Bad","from":3,"to":0,"direction":9,"repeat":1,"color":[1,2,3]}]})),
    ] {
        assert!(s.execute(cmd, p).is_err(), "{cmd}");
        assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
    }
}

#[test]
fn reference_layers_cannot_be_unlocked_into_a_discarded_display_cache_edit() {
    let mut s = session();
    s.edit("Reference", |doc, _| {
        doc.sprite.as_mut().unwrap().layers[0].flags |= 64;
        doc.layers[0].locks.all = true;
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(
        s.edit("Unlock", |doc, _| {
            doc.layers[0].locks.all = false;
            Ok(())
        })
        .is_err()
    );
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn tilemap_pixels_require_explicit_rasterization_while_other_edits_preserve_tile_records() {
    let doc = photocraft_io::import("tiles.aseprite", include_bytes!("../../../../io/tests/fixtures/aseprite/tiles.aseprite")).unwrap().document;
    let id = doc.layers[0].id;
    let mut s = Session::new();
    s.add_document(doc, None);
    let before = s.active().unwrap().doc.clone();
    let change = |doc: &mut Document, _: &mut Option<LayerId>| {
        doc.layer_mut(id).unwrap().surface_mut().unwrap().write_pixel(0, 0, &[0.0, 0.0, 0.0, 1.0]);
        Ok(())
    };
    assert!(s.edit("Paint", change).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("sprite.duplicateFrame", json!({})).unwrap();
    assert!(photocraft_io::export(&s.active().unwrap().doc, "aseprite", &Default::default()).is_ok());
    s.execute("timeline.setFrame", json!({"frame":0})).unwrap();
    s.execute("sprite.deleteFrame", json!({})).unwrap();
    assert!(photocraft_io::export(&s.active().unwrap().doc, "aseprite", &Default::default()).is_ok());
    s.execute("sprite.rasterizeTilemap", json!({"layer":id.0})).unwrap();
    s.edit("Paint", change).unwrap();
    assert!(s.active().unwrap().doc.sprite.as_ref().unwrap().tilemap_descriptor(id).is_none());
    let out = photocraft_io::export(&s.active().unwrap().doc, "aseprite", &Default::default()).unwrap();
    assert!(photocraft_io::import("pixels.aseprite", &out.bytes).unwrap().document.sprite.as_ref().unwrap().tilemap_descriptor(id).is_none());
}

#[test]
fn shrinking_an_indexed_palette_cannot_leave_locked_reference_cels_out_of_range() {
    let mut doc = photocraft_io::import("animation.aseprite", include_bytes!("../../../../io/tests/fixtures/aseprite/animation.aseprite")).unwrap().document;
    doc.sprite.as_mut().unwrap().layers[0].flags |= 64;
    doc.layers[0].locks.all = true;
    let mut s = add_to_session(doc);
    assert_refused_unchanged(&mut s, "image.mode.colorTable", json!({"colors": ["#000000", "#ff0000"], "transparent": 0}));
    let out = photocraft_io::export(&s.active().unwrap().doc, "aseprite", &Default::default()).unwrap();
    photocraft_io::import("reference.aseprite", &out.bytes).unwrap();
}

#[test]
fn turns_refuse_retained_slice_coordinates_before_pixels_or_history_change() {
    use photocraft_doc::sprite::{MetadataTarget, SpriteChunk};
    let mut s = session();
    s.edit("Slice", |doc, _| {
        Arc::make_mut(&mut doc.sprite.as_mut().unwrap().frames[0]).metadata.push(SpriteChunk {
            kind: 0x2022,
            target: MetadataTarget::Frame,
            bytes: Arc::new(vec![]),
        });
        Ok(())
    })
    .unwrap();
    for command in ["image.imageRotation.flipCanvasHorizontal", "image.imageRotation.flipCanvasVertical", "image.imageRotation.180", "image.imageRotation.90cw"]
    {
        assert_refused_unchanged(&mut s, command, json!({}));
    }
    let mut ordinary = session();
    ordinary.execute("image.imageRotation.flipCanvasHorizontal", json!({})).unwrap();
    assert_eq!(ordinary.active().unwrap().history.past_len(), 1);
    ordinary.execute("edit.undo", json!({})).unwrap();
}

#[test]
fn crops_with_retained_slices_cannot_move_the_canvas_origin_even_at_the_same_size() {
    use photocraft_doc::sprite::{MetadataTarget, SpriteChunk};
    let mut s = session();
    s.edit("Slice", |doc, _| {
        Arc::make_mut(&mut doc.sprite.as_mut().unwrap().frames[0]).metadata.push(SpriteChunk {
            kind: 0x2022,
            target: MetadataTarget::Frame,
            bytes: Arc::new(vec![]),
        });
        Ok(())
    })
    .unwrap();
    for (x, y) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        assert_refused_unchanged(&mut s, "image.crop", json!({"x":x,"y":y,"width":2,"height":1}));
    }
    assert_refused_unchanged(&mut s, "image.crop", json!({"x":0,"y":0,"width":2,"height":1,"angle":180}));
    let mut ordinary = session();
    ordinary.execute("image.crop", json!({"x":1,"y":0,"width":2,"height":1})).unwrap();
    assert_eq!(ordinary.active().unwrap().history.past_len(), 1);
    ordinary.execute("edit.undo", json!({})).unwrap();
}

#[test]
fn slice_coordinates_prevent_crop_resampling_even_when_final_canvas_size_is_unchanged() {
    use photocraft_doc::sprite::{MetadataTarget, SpriteChunk};
    let mut s = session();
    s.edit("Slice", |doc, _| {
        Arc::make_mut(&mut doc.sprite.as_mut().unwrap().frames[0]).metadata.push(SpriteChunk {
            kind: 0x2022,
            target: MetadataTarget::Frame,
            bytes: Arc::new(vec![]),
        });
        Ok(())
    })
    .unwrap();
    for params in
        [json!({"x":0,"y":0,"width":1,"height":1,"targetWidth":2,"targetHeight":1}), json!({"x":0,"y":0,"width":2,"height":2,"targetWidth":2,"targetHeight":1})]
    {
        assert_refused_unchanged(&mut s, "image.crop", params);
    }
    s.execute("image.crop", json!({"x":0,"y":0,"width":2,"height":1,"targetWidth":2,"targetHeight":1})).unwrap();
    let mut ordinary = session();
    let before = ordinary.active().unwrap().doc.clone();
    ordinary.execute("image.crop", json!({"x":0,"y":0,"width":1,"height":1,"targetWidth":2,"targetHeight":1})).unwrap();
    assert_eq!(ordinary.active().unwrap().doc.size, before.size);
    assert_eq!(ordinary.active().unwrap().history.past_len(), 1);
    ordinary.execute("edit.undo", json!({})).unwrap();
    assert_eq!(ordinary.active().unwrap().doc, before);
}

#[test]
fn arbitrary_canvas_rotation_refuses_slices_for_same_size_turns_and_allows_a_noop() {
    use photocraft_doc::sprite::{MetadataTarget, SpriteChunk};
    let mut doc = (*session().active().unwrap().doc).clone();
    doc.size = Size::new(2, 2);
    Arc::make_mut(&mut doc.sprite.as_mut().unwrap().frames[0]).metadata.push(SpriteChunk {
        kind: 0x2022,
        target: MetadataTarget::Frame,
        bytes: Arc::new(vec![]),
    });
    let mut s = add_to_session(doc);
    for angle in [90, 180, 270, -90, 45] {
        assert_refused_unchanged(&mut s, "image.rotation.arbitrary", json!({"angle":angle}));
    }
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let history = state.history.past_len();
    s.execute("image.rotation.arbitrary", json!({"angle":360})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
    let mut ordinary = session();
    ordinary.execute("image.rotation.arbitrary", json!({"angle":180})).unwrap();
    assert_eq!(ordinary.active().unwrap().history.past_len(), 1);
    ordinary.execute("edit.undo", json!({})).unwrap();
}

#[test]
fn rgb_and_gray_layer_from_background_stays_unlocked_after_aseprite_export() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let doc = Document::with_background("Background", Size::new(2, 1), mode, SampleType::U8, photocraft_color::Color::WHITE);
        let out = photocraft_io::export(&doc, "aseprite", &Default::default()).unwrap();
        let doc = photocraft_io::import("background.aseprite", &out.bytes).unwrap().document;
        let mut s = add_to_session(doc);
        assert!(s.active().unwrap().doc.background().is_some());
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        assert!(s.active().unwrap().doc.background().is_none());
        let out = photocraft_io::export(&s.active().unwrap().doc, "aseprite", &Default::default()).unwrap();
        let back = photocraft_io::import("unlocked.aseprite", &out.bytes).unwrap().document;
        assert!(back.background().is_none());
        assert!(!back.layers[0].locks.transparency);
        assert!(!back.layers[0].locks.position);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.background().is_some());
    }
}
