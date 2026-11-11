use super::*;
use crate::presets::clone_source::Mapping;

fn setup(depth: u64, mode: &str) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": depth, "mode": mode})).unwrap();
    let source = s.active().unwrap().active_layer.unwrap();
    s.edit("source", |doc, _| {
        let rect = doc.bounds();
        let surf = doc.layer_mut(source).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in 0..64 {
            for x in 0..64 {
                data.extend(photocraft_raster::from_rgba(&fmt, [x as f32 / 64.0, y as f32 / 64.0, 0.25, 1.0]));
            }
        }
        surf.write_region(rect, &data);
        Ok(())
    })
    .unwrap();
    s.execute("cloneSource.set", json!({"source": [16, 16]})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let target = s.active().unwrap().active_layer.unwrap();
    (s, source, target)
}

fn pixel(s: &Session, id: LayerId, x: i32, y: i32) -> [f32; 4] {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(x, y)
}

fn params(wrap: bool) -> Value {
    json!({"points": [[48, 48]], "size": 12, "hardness": 100, "opacity": 50, "flow": 40, "wrap": wrap})
}

#[test]
fn clone_current_layer_keeps_the_picked_source_for_preview_live_commit_and_undo() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            for wrap in [false, true] {
                let (mut s, source, target) = setup(depth, mode);
                let want = pixel(&s, source, 16, 16);
                let map = Mapping { source: (16.0, 16.0), anchor: (48.0, 48.0), m: [1.0, 0.0, 0.0, 1.0] };
                let preview = clone_preview_periodic(&s, Rect::new(48, 48, 49, 49), &map, "current", wrap).unwrap();
                assert_eq!(preview.px[0], want);
                let p = params(wrap);
                let live = LiveRetouch::begin(&s, "paint.cloneStamp", &p).unwrap();
                let live_pixels = live.doc.layer(target).unwrap().surface().unwrap().read_region(Rect::new(40, 40, 56, 56));
                s.offer_live_retouch(live).unwrap();
                let result = s.execute("paint.cloneStamp", p.clone()).unwrap();
                assert_eq!(result["fromLive"], true);
                let got = pixel(&s, target, 48, 48);
                assert!((got[3] - 0.2).abs() < 0.01, "{depth}/{mode}/{wrap}: {got:?}");
                for c in 0..3 {
                    assert!((got[c] - want[c]).abs() < 0.02, "{depth}/{mode}/{wrap}: {got:?} vs {want:?}");
                }
                assert_eq!(pixel(&s, source, 16, 16), want, "source remains untouched");
                assert_eq!(s.active().unwrap().doc.layer(target).unwrap().surface().unwrap().read_region(Rect::new(40, 40, 56, 56)), live_pixels);
                assert!(s.undo());
                assert_eq!(pixel(&s, target, 48, 48)[3], 0.0);
                assert!(s.redo());
                assert_eq!(pixel(&s, target, 48, 48), got);
                let (mut replay, _, replay_target) = setup(depth, mode);
                replay.execute("paint.cloneStamp", p).unwrap();
                assert_eq!(pixel(&replay, replay_target, 48, 48), got, "ordinary command matches the live commit");
            }
        }
    }
}

#[test]
fn clone_deleted_source_and_invalid_source_layer_fail_without_edits() {
    let (mut s, source, target) = setup(8, "rgb");
    s.execute("layer.delete", json!({"layer": source.0})).unwrap();
    let revision = s.active().unwrap().revision;
    assert!(s.execute("paint.cloneStamp", params(false)).is_err());
    assert!(LiveRetouch::begin(&s, "paint.cloneStamp", &params(false)).is_err());
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(pixel(&s, target, 48, 48)[3], 0.0);
    for value in [json!("bad"), json!(-1), json!(999999)] {
        let mut p = params(false);
        p["sourceLayer"] = value;
        assert!(s.execute("paint.cloneStamp", p).is_err());
        assert_eq!(s.active().unwrap().revision, revision);
    }
}

#[test]
fn clone_transparent_source_and_explicit_source_layer_are_respected() {
    let (mut s, source, target) = setup(16, "rgb");
    s.execute("cloneSource.set", json!({"source": [16, 16]})).unwrap();
    s.execute("paint.cloneStamp", params(false)).unwrap();
    assert_eq!(pixel(&s, target, 48, 48)[3], 0.0);
    let mut p = params(false);
    p["sourceLayer"] = json!(source.0);
    s.execute("paint.cloneStamp", p).unwrap();
    assert!((pixel(&s, target, 48, 48)[3] - 0.2).abs() < 0.01);
}

#[test]
fn clone_source_layer_does_not_change_composite_sampling_modes() {
    for which in ["currentAndBelow", "all"] {
        let (mut s, source, target) = setup(8, "rgb");
        // A removed picked layer is irrelevant when sampling the visible stack.
        s.execute("layer.delete", json!({"layer": source.0})).unwrap();
        let mut p = params(false);
        p["sampleLayer"] = json!(which);
        s.execute("paint.cloneStamp", p).unwrap();
        assert_eq!(pixel(&s, target, 48, 48)[3], 0.0);
    }
}

#[test]
fn clone_live_commit_rejects_a_changed_source_layer_with_the_same_coordinates() {
    let (mut s, _, target) = setup(8, "rgb");
    let live = LiveRetouch::begin(&s, "paint.cloneStamp", &params(false)).unwrap();
    s.execute("cloneSource.set", json!({"source": [16, 16], "layer": target.0})).unwrap();
    assert!(s.offer_live_retouch(live).is_err());
}

#[test]
fn clone_repicking_on_the_original_layer_updates_both_layers() {
    for depth in [8, 16, 32] {
        let (mut s, source, target) = setup(depth, "rgb");
        let p = json!({"points": [[48, 48]], "size": 8, "hardness": 100});
        s.execute("paint.cloneStamp", p.clone()).unwrap();
        let first = pixel(&s, source, 16, 16);
        assert_eq!(pixel(&s, target, 48, 48), first);
        s.execute("layer.select", json!({"layer": source.0})).unwrap();
        s.execute("cloneSource.set", json!({"source": [8, 8]})).unwrap();
        let replacement = pixel(&s, source, 8, 8);
        assert_ne!(replacement, first);
        s.execute("paint.cloneStamp", p.clone()).unwrap();
        assert_eq!(pixel(&s, source, 48, 48), replacement, "same-layer painting uses the new source");
        s.execute("layer.select", json!({"layer": target.0})).unwrap();
        let map = Mapping { source: (8.0, 8.0), anchor: (48.0, 48.0), m: [1.0, 0.0, 0.0, 1.0] };
        assert_eq!(clone_preview(&s, Rect::new(48, 48, 49, 49), &map, "current").unwrap().px[0], replacement);
        s.execute("paint.cloneStamp", p).unwrap();
        assert_eq!(pixel(&s, target, 48, 48), replacement, "returning to the other layer uses the new source too");
    }
}

#[test]
fn clone_sources_work_between_documents_with_preview_live_commit_and_undo() {
    for depth in [8, 16, 32] {
        for which in ["current", "currentAndBelow", "all"] {
            for wrap in [false, true] {
                let (mut s, source, _) = setup(depth, "rgb");
                let want = pixel(&s, source, 16, 16);
                let source_doc = s.active().unwrap().doc.clone();
                s.execute("file.new", json!({"width": 80, "height": 80, "depth": depth})).unwrap();
                s.execute("layer.new.layer", json!({})).unwrap();
                let target = s.active().unwrap().active_layer.unwrap();
                let mut p = params(wrap);
                p["sampleLayer"] = json!(which);
                let map = Mapping { source: (16.0, 16.0), anchor: (48.0, 48.0), m: [1.0, 0.0, 0.0, 1.0] };
                let preview = clone_preview_periodic(&s, Rect::new(48, 48, 49, 49), &map, which, wrap).unwrap();
                for (got, expected) in preview.px[0].iter().zip(want) {
                    assert!((got - expected).abs() < 0.01, "{depth}/{which}/{wrap}");
                }
                let live = LiveRetouch::begin(&s, "paint.cloneStamp", &p).unwrap();
                let shown = live.doc.layer(target).unwrap().surface().unwrap().rgba(48, 48);
                s.offer_live_retouch(live).unwrap();
                assert_eq!(s.execute("paint.cloneStamp", p.clone()).unwrap()["fromLive"], true);
                assert_eq!(pixel(&s, target, 48, 48), shown);
                assert!((shown[3] - 0.2).abs() < 0.01);
                assert_eq!(s.documents()[0].doc, source_doc, "the source document is read-only");
                assert!(s.undo());
                assert_eq!(pixel(&s, target, 48, 48)[3], 0.0);
                s.execute("paint.cloneStamp", p).unwrap();
                assert_eq!(pixel(&s, target, 48, 48), shown, "ordinary rendering matches the live commit");
            }
        }
    }
}

#[test]
fn clone_cross_document_color_profiles_and_modes_use_cms() {
    use photocraft_cms::{Builtin, TransformOptions};
    for (from, to) in [("rgb", "rgb"), ("gray", "rgb"), ("rgb", "cmyk"), ("lab", "rgb")] {
        let (mut s, source, _) = setup(32, from);
        if from == "rgb" {
            s.edit("profile", |doc, _| {
                doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
                Ok(())
            })
            .unwrap();
        }
        let doc = s.active().unwrap().doc.clone();
        let native = doc.layer(source).unwrap().surface().unwrap().read_region(Rect::new(16, 16, 17, 17));
        s.execute("file.new", json!({"width": 64, "height": 64, "mode": to, "depth": 16})).unwrap();
        let dst = s.active().unwrap().doc.clone();
        let transform = photocraft_cms::transform::cached(
            &crate::color_cmds::document_profile(&doc),
            &crate::color_cmds::document_profile(&dst),
            TransformOptions { intent: s.color.settings.intent(), bpc: s.color.settings.bpc, ..Default::default() },
        )
        .unwrap();
        let mut want = [0.0; 16];
        transform.eval(&native, &mut want);
        s.execute("paint.cloneStamp", json!({"points": [[48,48]], "size": 8, "hardness": 100})).unwrap();
        let st = s.active().unwrap();
        let got = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(48, 48, 49, 49));
        for (got, want) in got.iter().zip(want).take(dst.mode.color_channels()) {
            assert!((got - want).abs() < 0.005, "{from} -> {to}: {got} vs {want}");
        }
    }
}

#[test]
fn clone_closed_source_document_and_stale_live_pixels_fail_gracefully() {
    let (mut s, source, _) = setup(8, "rgb");
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let target = s.active().unwrap().active_layer.unwrap();
    let p = params(false);
    let live = LiveRetouch::begin(&s, "paint.cloneStamp", &p).unwrap();
    assert!(s.set_active(0));
    s.edit("source changed", |doc, _| {
        doc.layer_mut(source).unwrap().surface_mut().unwrap().fill_rect(Rect::new(16, 16, 17, 17), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    assert!(s.set_active(1));
    assert!(s.offer_live_retouch(live).is_err(), "a changed source revision invalidates offered pixels");
    s.execute("paint.cloneStamp", p.clone()).unwrap();
    let updated = pixel(&s, target, 48, 48);
    assert_eq!(&updated[..3], &[1.0, 0.0, 0.0], "ordinary replay reads the edited source after rejecting stale live pixels");
    assert!((updated[3] - 0.2).abs() < 0.01);
    s.close(0).unwrap();
    let revision = s.active().unwrap().revision;
    let anchor = s.presets.clone.active().anchor;
    assert!(s.execute("paint.cloneStamp", p).is_err());
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.presets.clone.active().anchor, anchor);
    for value in [json!(-1), json!("bad"), json!(999999)] {
        assert!(s.execute("cloneSource.set", json!({"source": [8,8], "document": value})).is_err());
    }
}

#[test]
fn clone_closed_source_cannot_be_revived_by_reopening_a_saved_copy() {
    let (mut s, source, _) = setup(8, "rgb");
    let saved_copy = (*s.active().unwrap().doc).clone();
    let expected = pixel(&s, source, 16, 16);
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let target = s.active().unwrap().active_layer.unwrap();
    s.close(0).unwrap();
    let reopened = s.add_document(saved_copy.clone(), None);
    assert_ne!(s.documents()[reopened].doc.id, saved_copy.id, "a new tab must not revive a closed clone source's identity");
    assert!(s.set_active(0));
    let before = s.active().unwrap().doc.clone();
    let p = params(false);
    assert!(s.execute("paint.cloneStamp", p.clone()).is_err());
    assert!(LiveRetouch::begin(&s, "paint.cloneStamp", &p).is_err());
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("cloneSource.set", json!({"source": [16,16], "document": reopened, "layer": source.0})).unwrap();
    s.execute("paint.cloneStamp", p).unwrap();
    let got = pixel(&s, target, 48, 48);
    for c in 0..3 {
        assert!((got[c] - expected[c]).abs() < 0.01, "explicitly re-picking the reopened document works");
    }
    assert!((got[3] - 0.2).abs() < 0.01);
}

#[test]
fn clone_source_document_json_and_legacy_slots_are_compatible() {
    let (mut s, _, _) = setup(8, "rgb");
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let slot = s.execute("cloneSource.set", json!({"source": [8,8], "document": 0})).unwrap();
    assert_eq!(slot["document"], 0);
    let mut legacy = serde_json::to_value(s.presets.clone.active()).unwrap();
    legacy.as_object_mut().unwrap().remove("document");
    let old: crate::presets::clone_source::CloneSource = serde_json::from_value(legacy).unwrap();
    assert!(old.document.is_none());
}

#[test]
fn clone_source_replies_keep_document_identity_when_resetting_transforms() {
    let (mut s, _, _) = setup(8, "rgb");
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let document_id = s.active().unwrap().doc.id;
    assert!(s.set_active(0));
    s.execute("cloneSource.set", json!({"index": 2, "source": [8,8], "document": 1, "width": 200, "rotation": 45})).unwrap();
    let replies = [
        s.execute("cloneSource.list", json!({})).unwrap()["sources"][2].clone(),
        s.execute("cloneSource.select", json!({"index": 2})).unwrap(),
        s.execute("cloneSource.set", json!({"index": 2, "width": 150})).unwrap(),
        s.execute("cloneSource.resetTransform", json!({"index": 2})).unwrap(),
    ];
    for reply in replies {
        assert_eq!(reply["document"], 1, "all slot replies expose the same source-document index: {reply}");
        assert_eq!(reply["documentId"], json!(document_id));
        assert_eq!(reply["source"], json!([8.0, 8.0]));
    }
    assert_eq!(s.presets.clone.active().scale, [100.0, 100.0]);
    assert_eq!(s.presets.clone.active().rotation, 0.0);
    s.close(0).unwrap();
    let reset = s.execute("cloneSource.resetTransform", json!({"index": 2})).unwrap();
    assert_eq!(reset["document"], 0, "closing an earlier tab changes the index, not the source identity");
    assert_eq!(reset["documentId"], json!(document_id));
}

#[test]
fn clone_between_documents_preserves_float_hdr_and_source_alpha() {
    for alpha in [false, true] {
        let (mut s, source, _) = setup(32, "rgb");
        s.edit("source pixels", |doc, _| {
            let fmt = PixelFormat::new(photocraft_color::ColorMode::Rgb, SampleType::F32, alpha);
            let values = if alpha { vec![1.5, 0.25, 0.75, 0.25] } else { vec![1.5, 0.25, 0.75] };
            *doc.layer_mut(source).unwrap().surface_mut().unwrap() = Surface::with_default(fmt, &values);
            Ok(())
        })
        .unwrap();
        s.execute("file.new", json!({"width": 64, "height": 64, "depth": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let target = s.active().unwrap().active_layer.unwrap();
        s.execute("paint.cloneStamp", json!({"points": [[48,48]], "size": 8, "hardness": 100})).unwrap();
        assert_eq!(pixel(&s, target, 48, 48), [1.5, 0.25, 0.75, if alpha { 0.25 } else { 1.0 }]);
    }
}

#[test]
fn clone_cross_document_mask_targets_refuse_instead_of_sampling_the_wrong_document() {
    let (mut s, _, _) = setup(8, "rgb");
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let mut p = params(false);
    p["target"] = json!("mask");
    assert!(s.execute("paint.cloneStamp", p.clone()).is_err());
    assert!(LiveRetouch::begin(&s, "paint.cloneStamp", &p).is_err());
    assert_eq!(s.active().unwrap().doc, before);
}

#[test]
fn clone_cross_document_mismatched_surface_models_fail_without_panicking_or_editing() {
    for change_source in [true, false] {
        let (mut s, source, _) = setup(32, "rgb");
        s.execute("file.new", json!({"width": 64, "height": 64, "depth": 32})).unwrap();
        let target = s.active().unwrap().active_layer.unwrap();
        assert!(s.set_active(if change_source { 0 } else { 1 }));
        s.edit("malformed layer", |doc, _| {
            let id = if change_source { source } else { target };
            *doc.layer_mut(id).unwrap().surface_mut().unwrap() =
                Surface::with_default(PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F32, false), &[0.5]);
            Ok(())
        })
        .unwrap();
        assert!(s.set_active(1));
        let before = s.active().unwrap().doc.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute("paint.cloneStamp", params(false))));
        assert!(result.unwrap().is_err());
        assert_eq!(s.active().unwrap().doc, before);
    }
}

#[test]
fn clone_source_edges_clip_preview_live_and_committed_stamps() {
    for depth in [8, 16, 32] {
        for external in [false, true] {
            for which in ["current", "currentAndBelow", "all"] {
                for (sx, sy) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
                    let (mut s, source, mut target) = setup(depth, "rgb");
                    s.edit("finite source", |doc, _| {
                        let bounds = doc.bounds();
                        doc.layer_mut(source).unwrap().surface_mut().unwrap().fill_rect(bounds, &[1.0, 0.0, 0.0, 1.0]);
                        Ok(())
                    })
                    .unwrap();
                    s.execute("layer.select", json!({"layer": source.0})).unwrap();
                    s.execute("cloneSource.set", json!({"source": [sx, sy]})).unwrap();
                    if external {
                        s.execute("file.new", json!({"width": 80, "height": 80, "depth": depth})).unwrap();
                        s.execute("layer.new.layer", json!({})).unwrap();
                        target = s.active().unwrap().active_layer.unwrap();
                    } else {
                        s.execute("layer.select", json!({"layer": target.0})).unwrap();
                    }
                    let (dx, dy) = (if sx == 0 { 1 } else { -1 }, if sy == 0 { 1 } else { -1 });
                    let map = Mapping { source: (sx as f64, sy as f64), anchor: (32.0, 32.0), m: [1.0, 0.0, 0.0, 1.0] };
                    let preview = clone_preview(&s, Rect::new(30, 30, 35, 35), &map, which).unwrap();
                    let checks = [(32, 32 + dy, 1.0), (32 + dx, 32, 1.0), (32 - dx, 32 + dy, 0.0), (32 + dx, 32 - dy, 0.0)];
                    for (x, y, alpha) in checks {
                        let index = ((y - 30) * 5 + x - 30) as usize;
                        assert_eq!(preview.px[index][3], alpha, "preview {depth}/{external}/{which}/{sx},{sy} at {x},{y}");
                    }
                    let p = json!({"points": [[32,32]], "size": 32, "hardness": 100, "sampleLayer": which});
                    let live = LiveRetouch::begin(&s, "paint.cloneStamp", &p).unwrap();
                    for (x, y, alpha) in checks {
                        assert_eq!(live.doc.layer(target).unwrap().surface().unwrap().rgba(x, y)[3], alpha);
                    }
                    s.offer_live_retouch(live).unwrap();
                    assert_eq!(s.execute("paint.cloneStamp", p.clone()).unwrap()["fromLive"], true);
                    for (x, y, alpha) in checks {
                        assert_eq!(pixel(&s, target, x, y)[3], alpha, "commit {depth}/{external}/{which}/{sx},{sy} at {x},{y}");
                    }
                    assert!(s.undo());
                    s.execute("paint.cloneStamp", p).unwrap();
                    for (x, y, alpha) in checks {
                        assert_eq!(pixel(&s, target, x, y)[3], alpha, "ordinary rendering matches the live stamp");
                    }
                }
            }
        }
    }
}

#[test]
fn clone_clipped_source_preserves_existing_destination_pixels_on_the_original_layer() {
    let (mut s, source, _) = setup(16, "rgb");
    s.edit("two colors", |doc, _| {
        let bounds = doc.bounds();
        let surf = doc.layer_mut(source).unwrap().surface_mut().unwrap();
        surf.fill_rect(bounds, &[0.0, 0.0, 1.0, 1.0]);
        surf.fill_rect(Rect::new(0, 0, 16, 16), &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": source.0})).unwrap();
    s.execute("cloneSource.set", json!({"source": [0, 0]})).unwrap();
    s.execute("paint.cloneStamp", json!({"points": [[32,32]], "size": 32, "hardness": 100})).unwrap();
    assert_eq!(pixel(&s, source, 33, 33), [1.0, 0.0, 0.0, 1.0]);
    for (x, y) in [(31, 33), (33, 31), (31, 31)] {
        assert_eq!(pixel(&s, source, x, y), [0.0, 0.0, 1.0, 1.0], "missing source pixels leave the blue destination intact");
    }
}

#[test]
fn clone_pattern_preview_wraps_at_the_source_document_edges() {
    let (mut s, source, _) = setup(16, "rgb");
    let want = pixel(&s, source, 1, 16);
    s.execute("cloneSource.set", json!({"source": [63,16], "layer": source.0})).unwrap();
    s.execute("file.new", json!({"width": 80, "height": 80, "depth": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let target = s.active().unwrap().active_layer.unwrap();
    let map = Mapping { source: (63.0, 16.0), anchor: (48.0, 48.0), m: [1.0, 0.0, 0.0, 1.0] };
    let preview = clone_preview_periodic(&s, Rect::new(50, 48, 51, 49), &map, "current", true).unwrap();
    assert_eq!(preview.px[0], want);
    s.execute("paint.cloneStamp", json!({"points": [[48,48]], "size": 12, "hardness": 100, "wrap": true})).unwrap();
    assert_eq!(pixel(&s, target, 50, 48), want, "wrapping uses the 64 px source canvas, not the 80 px destination");
}

#[test]
#[ignore = "allocates full 5,000 px clone coverage and pixel buffers; run explicitly with one test thread"]
fn clone_maximum_size_stamp_keeps_straight_source_edges() {
    let (mut s, source, _) = setup(8, "rgb");
    s.edit("finite red source", |doc, _| {
        let bounds = doc.bounds();
        doc.layer_mut(source).unwrap().surface_mut().unwrap().fill_rect(bounds, &[1.0, 0.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("cloneSource.set", json!({"source": [0,0], "layer": source.0})).unwrap();
    s.execute("file.new", json!({"width": 6000, "height": 4000, "background": "transparent"})).unwrap();
    let target = s.active().unwrap().active_layer.unwrap();
    let start = std::time::Instant::now();
    s.execute("paint.cloneStamp", json!({"points": [[3000,2000]], "size": 5000, "hardness": 100})).unwrap();
    eprintln!("5,000 px hard clone stamp on 24 MP: {:?}", start.elapsed());
    assert_eq!(pixel(&s, target, 3000, 2000), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(pixel(&s, target, 3063, 2063), [1.0, 0.0, 0.0, 1.0]);
    for (x, y) in [(2999, 2001), (3001, 1999), (3064, 2001), (3001, 2064), (4000, 2000)] {
        assert_eq!(pixel(&s, target, x, y)[3], 0.0, "source clips at {x},{y} within the 5,000 px tip");
    }
}

#[test]
fn clone_offsets_report_each_unaligned_stroke_without_persisting_its_alignment() {
    for depth in [8, 16, 32] {
        let (mut s, source, target) = setup(depth, "rgb");
        let want = pixel(&s, source, 16, 16);
        for (x, y) in [(40, 30), (50, 40)] {
            s.execute("paint.cloneStamp", json!({"points": [[x,y]], "size": 8, "hardness": 100, "aligned": false})).unwrap();
            assert_eq!(pixel(&s, target, x, y), want, "each unaligned stroke restarts at the sample");
            let listed = s.execute("cloneSource.list", json!({})).unwrap();
            assert_eq!(listed["sources"][0]["offset"], json!([16.0 - x as f64, 16.0 - y as f64]));
            assert!(s.presets.clone.active().anchor.is_none(), "reporting an offset must not establish aligned sampling");
            assert_eq!(s.presets.clone.active().source, Some([16.0, 16.0]));
        }
    }
}

#[test]
fn clone_offsets_follow_mode_toggles_and_manual_edits_use_the_reported_pairing() {
    for depth in [8, 16, 32] {
        let (mut s, source, target) = setup(depth, "rgb");
        let stamp = |s: &mut Session, x, y, aligned| {
            s.execute("paint.cloneStamp", json!({"points": [[x,y]], "size": 8, "hardness": 100, "aligned": aligned})).unwrap();
        };
        stamp(&mut s, 40, 30, true);
        stamp(&mut s, 50, 40, false);
        assert_eq!(s.presets.clone.active().anchor, Some([40.0, 30.0]));
        assert_eq!(s.execute("cloneSource.list", json!({})).unwrap()["sources"][0]["offset"], json!([-34.0, -24.0]));
        stamp(&mut s, 52, 42, true);
        assert_eq!(pixel(&s, target, 52, 42), pixel(&s, source, 28, 28), "switching back retains the aligned pairing");
        stamp(&mut s, 50, 40, false);
        s.execute("cloneSource.set", json!({"offset": [-30,-20]})).unwrap();
        assert_eq!(s.presets.clone.active().source, Some([20.0, 20.0]), "editing the displayed offset uses its actual destination reference");
        assert_eq!(s.presets.clone.active().anchor, Some([50.0, 40.0]));
        stamp(&mut s, 54, 44, false);
        assert_eq!(pixel(&s, target, 54, 44), pixel(&s, source, 20, 20));
        stamp(&mut s, 54, 44, true);
        assert_eq!(pixel(&s, target, 54, 44), pixel(&s, source, 24, 24), "a manual edit establishes the newly requested pairing");
    }
}

#[test]
fn clone_offsets_accept_a_preview_reference_and_reject_invalid_points_atomically() {
    let (mut s, _, _) = setup(16, "rgb");
    let reply = s.execute("cloneSource.set", json!({"offset": [-20,-10], "anchor": [40,30]})).unwrap();
    assert_eq!(reply["source"], json!([20.0, 20.0]));
    assert_eq!(reply["offset"], json!([-20.0, -10.0]));
    for p in [
        json!({"offset": [-20,-10], "anchor": ["bad",30]}),
        json!({"offset": [-20,-10], "anchor": [1.0e300,30]}),
        json!({"anchor": [40,30]}),
        json!({"source": [1.0e300,0]}),
        json!({"source": [20,20], "offset": ["bad",0]}),
        json!({"source": [20,20], "anchor": [999999,999999], "offset": [10,20]}),
    ] {
        let before = s.presets.clone.clone();
        assert!(s.execute("cloneSource.set", p).is_err());
        assert_eq!(s.presets.clone, before, "a rejected edit leaves all source slots unchanged");
    }
}

#[test]
fn clone_offsets_do_not_record_previews_failed_strokes_or_explicit_command_sources() {
    for cmd in ["paint.cloneStamp", "paint.healingBrush"] {
        let (mut s, _, target) = setup(8, "rgb");
        let p = json!({"points": [[40,30]], "size": 8, "hardness": 100});
        let before = s.presets.clone.clone();
        let _live = LiveRetouch::begin(&s, cmd, &p).unwrap();
        assert_eq!(s.presets.clone, before, "a live preview is read-only");
        s.edit("lock target", |doc, _| {
            doc.layer_mut(target).unwrap().locks.all = true;
            Ok(())
        })
        .unwrap();
        assert!(s.execute(cmd, p.clone()).is_err());
        assert_eq!(s.presets.clone, before, "failed painting cannot anchor or report a successful pairing");
        s.edit("unlock target", |doc, _| {
            doc.layer_mut(target).unwrap().locks.all = false;
            Ok(())
        })
        .unwrap();
        let mut explicit = p.clone();
        explicit["offset"] = json!([-20, -10]);
        s.execute(cmd, explicit).unwrap();
        assert_eq!(s.presets.clone, before, "an explicit command source does not overwrite the picked slot");
        let mut unaligned = p;
        unaligned["aligned"] = json!(false);
        s.execute(cmd, unaligned).unwrap();
        assert_eq!(s.execute("cloneSource.list", json!({})).unwrap()["sources"][0]["offset"], json!([-24.0, -14.0]));
        assert!(s.presets.clone.active().anchor.is_none());
    }
}

#[test]
fn clone_offsets_keep_legacy_pairings_and_clear_readouts_when_a_source_is_repicked() {
    let (mut s, source, _) = setup(16, "rgb");
    s.execute("paint.cloneStamp", json!({"points": [[40,30]], "size": 8, "hardness": 100})).unwrap();
    let mut old = serde_json::to_value(s.presets.clone.active()).unwrap();
    old.as_object_mut().unwrap().remove("last_anchor");
    let restored: crate::presets::clone_source::CloneSource = serde_json::from_value(old).unwrap();
    assert_eq!(restored.offset(), Some([-24.0, -14.0]));
    let picked = s.execute("cloneSource.set", json!({"source": [20,20], "layer": source.0})).unwrap();
    assert!(picked["offset"].is_null());
    assert!(picked["anchor"].is_null());
    assert!(picked["lastAnchor"].is_null());
    s.execute("cloneSource.set", json!({"source": null, "offset": null, "anchor": null, "index": null, "width": 150})).unwrap();
    assert_eq!(s.presets.clone.active().source, Some([20.0, 20.0]), "null optional coordinates remain omitted for compatibility");
    for value in [json!(u64::MAX), json!(-1), json!("bad")] {
        assert!(s.execute("cloneSource.set", json!({"index": value})).is_err());
        assert!(s.execute("cloneSource.select", json!({"index": value})).is_err());
    }
    s.presets.clone.active = usize::MAX;
    assert!(
        matches!(s.execute("cloneSource.set", json!({})), Err(EngineError::BadParams { .. })),
        "corrupt saved selection must be a parameter error, not an internal panic"
    );
    assert!(matches!(s.execute("cloneSource.resetTransform", json!({})), Err(EngineError::BadParams { .. })));
}

#[test]
fn clone_offsets_scaled_source_reads_are_bounded_before_allocation() {
    for cmd in ["paint.cloneStamp", "paint.healingBrush"] {
        let (mut s, _, _) = setup(8, "rgb");
        let before = s.presets.clone.clone();
        let document = s.active().unwrap().doc.clone();
        let result = s.execute(cmd, json!({"points": [[32,32]], "scale": [0.00001,0.00001], "size": 8, "hardness": 100}));
        assert!(result.is_err(), "a small dab cannot allocate an unbounded transformed source");
        assert_eq!(s.active().unwrap().doc, document);
        assert_eq!(s.presets.clone, before);
    }
}

#[test]
fn clone_offsets_null_optional_coordinates_keep_the_picked_source_document() {
    for depth in [8, 16, 32] {
        let (mut s, source, _) = setup(depth, "rgb");
        let want = pixel(&s, source, 16, 16);
        s.execute("file.new", json!({"width": 64, "height": 64, "depth": depth})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let target = s.active().unwrap().active_layer.unwrap();
        s.execute("paint.cloneStamp", json!({"points": [[40,30]], "size": 8, "hardness": 100, "source": null, "offset": null, "anchor": null})).unwrap();
        assert_eq!(pixel(&s, target, 40, 30), want, "null optional coordinates must not silently switch sampling to the destination document");
        assert_eq!(s.execute("cloneSource.list", json!({})).unwrap()["sources"][0]["offset"], json!([-24.0, -14.0]));
        s.execute("cloneSource.set", json!({"width": 200, "rotation": 90})).unwrap();
        let (map, _) =
            crate::presets::clone_source::resolve(&s, &json!({"scale": null, "rotation": null, "flipH": null}), (40.0, 30.0), "paint.cloneStamp").unwrap();
        assert_eq!(map.m, crate::presets::clone_source::transform_matrix([200.0, 100.0], 90.0, false, false));
    }
}

#[test]
fn clone_and_heal_transformed_far_sources_are_finite_and_preserve_the_destination() {
    for depth in [8, 16, 32] {
        for cmd in ["paint.cloneStamp", "paint.healingBrush"] {
            let (mut s, _, target) = setup(depth, "rgb");
            s.edit("green destination", |doc, _| {
                let bounds = doc.bounds();
                doc.layer_mut(target).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.0, 1.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
            for source in [[3.0e9, 0.0], [0.0, 3.0e9], [-3.0e9, 0.0], [0.0, -3.0e9], [f64::MAX, f64::MAX]] {
                s.execute(cmd, json!({"points": [[32,32]], "source": source, "rotation": 45, "size": 8, "hardness": 100})).unwrap();
                let got = pixel(&s, target, 32, 32);
                for (got, want) in got.into_iter().zip([0.0, 1.0, 0.0, 1.0]) {
                    assert!(got.is_finite() && (got - want).abs() < 1e-5, "{cmd} at {depth}-bit, source {source:?}: {got}");
                }
            }
        }
    }
}
