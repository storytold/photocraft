use super::*;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::LayerMask;
use std::sync::Arc;

fn masked(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Masked colour"})).unwrap();
    let id = s.active().unwrap().active_layer.unwrap();
    s.edit("fixture", |doc, _| {
        let l = doc.layer_mut(id).unwrap();
        l.surface_mut().unwrap().fill_rect(Rect::new(10, 8, 30, 24), &[0.8, 0.2, 0.4, 1.0]);
        let mut mask = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, doc_depth(depth), false), &[1.0]);
        mask.fill_rect(Rect::new(6, 4, 20, 28), &[0.0]);
        mask.fill_rect(Rect::new(20, 4, 24, 28), &[0.5]);
        l.mask = Some(LayerMask { surface: mask, enabled: true, linked: false, density: 0.75, feather: 1.25 });
        Ok(())
    })
    .unwrap();
    (s, id)
}

fn doc_depth(depth: u32) -> SampleType {
    match depth {
        16 => SampleType::U16,
        32 => SampleType::F32,
        _ => SampleType::U8,
    }
}

fn layer(s: &Session, id: LayerId) -> &Layer {
    s.active().unwrap().doc.layer(id).unwrap()
}

fn pasted(s: &mut Session, params: Value) -> LayerId {
    LayerId(s.execute("edit.paste", params).unwrap()["layer"].as_u64().unwrap())
}

#[test]
fn copy_then_paste_keeps_an_editable_nonuniform_mask() {
    for depth in [8, 16, 32] {
        let (mut s, src) = masked(depth);
        let original = layer(&s, src).clone();
        s.execute("edit.copy", json!({})).unwrap();
        let id = pasted(&mut s, json!({}));
        assert!(layer(&s, id).mask == original.mask, "{depth}-bit editable mask and metadata");
        assert!(layer(&s, id).surface() == original.surface(), "independent colour data");
        for (a, b) in original.surface().unwrap().tiles().zip(layer(&s, id).surface().unwrap().tiles()) {
            assert_eq!(a.0, b.0);
            assert!(Arc::ptr_eq(a.1, b.1), "matching colour tiles remain COW-shared");
        }
        for (a, b) in original.mask.as_ref().unwrap().surface.tiles().zip(layer(&s, id).mask.as_ref().unwrap().surface.tiles()) {
            assert_eq!(a.0, b.0);
            assert!(Arc::ptr_eq(a.1, b.1), "matching mask tiles remain COW-shared");
        }
        assert_ne!(id, src);
        let area = Rect::new(10, 8, 30, 24);
        let a = photocraft_compose::render_layer(&original, area);
        let b = photocraft_compose::render_layer(layer(&s, id), area);
        assert_eq!(a.px, b.px, "the composited appearance matches");
        let steps = s.active().unwrap().history.past_len();
        s.undo();
        assert!(s.active().unwrap().doc.layer(id).is_none());
        s.redo();
        assert!(layer(&s, id).mask == original.mask);
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        s.edit("edit pasted mask and colour", |doc, _| {
            let l = doc.layer_mut(id).unwrap();
            l.mask.as_mut().unwrap().surface.write_pixel(12, 12, &[1.0]);
            l.surface_mut().unwrap().write_pixel(25, 12, &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        assert!(layer(&s, src).mask == original.mask, "editing the copy leaves the source mask alone");
        assert!(layer(&s, src).surface() == original.surface());
        let next = pasted(&mut s, json!({}));
        assert_ne!(next, id);
        assert!(layer(&s, next).mask == original.mask, "the clipboard is an independent snapshot");
        assert!(layer(&s, next).surface() == original.surface());
    }
}

#[test]
fn source_edits_after_copy_do_not_change_the_clipboard() {
    let (mut s, src) = masked(32);
    let original = layer(&s, src).clone();
    s.execute("edit.copy", json!({})).unwrap();
    s.edit("edit source", |doc, _| {
        let l = doc.layer_mut(src).unwrap();
        l.mask.as_mut().unwrap().surface.write_pixel(12, 12, &[1.0]);
        l.surface_mut().unwrap().write_pixel(25, 12, &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let id = pasted(&mut s, json!({}));
    assert!(layer(&s, id).mask == original.mask);
    assert!(layer(&s, id).surface() == original.surface());
}

#[test]
fn disabled_masks_inverted_samples_and_link_flags_survive_whole_layer_paste() {
    for enabled in [true, false] {
        for linked in [true, false] {
            let (mut s, src) = masked(32);
            s.edit("inverted fixture", |doc, _| {
                let m = doc.layer_mut(src).unwrap().mask.as_mut().unwrap();
                m.enabled = enabled;
                m.linked = linked;
                m.surface = Surface::with_default(m.surface.format(), &[0.0]);
                m.surface.fill_rect(Rect::new(10, 8, 20, 24), &[1.0]);
                Ok(())
            })
            .unwrap();
            let original = layer(&s, src).mask.as_ref().unwrap().clone();
            s.execute("edit.copy", json!({})).unwrap();
            let id = pasted(&mut s, json!({"center": [40, 36]}));
            let m = layer(&s, id).mask.as_ref().unwrap();
            assert_eq!((m.enabled, m.linked, m.density, m.feather), (original.enabled, original.linked, original.density, original.feather));
            assert_eq!(m.surface.sample_channel(32, 32, 0), 1.0);
            assert_eq!(m.surface.sample_channel(45, 32, 0), 0.0);
            assert_eq!(m.surface.default_pixel(), vec![0.0]);
        }
    }
}

#[test]
fn cross_document_paste_moves_both_masks_even_when_unlinked_and_converts_colour() {
    use photocraft_doc::vector::{Path, Subpath, VectorMask};
    for source_depth in [8, 16, 32] {
        for (mode, dest_depth) in [("grayscale", 16), ("cmyk", 32), ("lab", 8), ("rgb", 32)] {
            let (mut s, src) = masked(source_depth);
            s.edit("vector fixture", |doc, _| {
                doc.layer_mut(src).unwrap().vector_mask = Some(VectorMask {
                    path: Path::new(vec![Subpath::polygon(&[(11.0, 9.0), (29.0, 9.0), (29.0, 23.0), (11.0, 23.0)])]),
                    enabled: false,
                    linked: false,
                    density: 0.6,
                    feather: 2.0,
                });
                Ok(())
            })
            .unwrap();
            let original = layer(&s, src).clone();
            s.execute("edit.copy", json!({})).unwrap();
            s.execute("file.new", json!({"width": 100, "height": 80, "depth": dest_depth})).unwrap();
            if mode != "rgb" {
                s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
            }
            let id = pasted(&mut s, json!({"center": [50, 40]}));
            // Source colour extent is (10,8)-(30,24), so centre (20,16) moves by (30,24).
            let l = layer(&s, id);
            assert_eq!(l.surface().unwrap().content_bounds(), Rect::new(40, 32, 60, 48));
            let m = l.mask.as_ref().unwrap();
            let old = original.mask.as_ref().unwrap();
            assert_eq!((m.enabled, m.linked, m.density, m.feather), (old.enabled, old.linked, old.density, old.feather));
            assert!((m.surface.sample_channel(51, 36, 0) - old.surface.sample_channel(21, 12, 0)).abs() < 0.005);
            assert_eq!(m.surface.sample_channel(42, 36, 0), 0.0);
            assert_eq!(m.surface.default_pixel(), vec![1.0]);
            assert_eq!(m.surface.format().sample, doc_depth(dest_depth));
            let vm = l.vector_mask.as_ref().unwrap();
            let old = original.vector_mask.as_ref().unwrap();
            assert_eq!(vm.path, old.path.transform(&photocraft_geom::Affine::translate(30.0, 24.0)));
            assert_eq!((vm.enabled, vm.linked, vm.density, vm.feather), (old.enabled, old.linked, old.density, old.feather));
            assert_eq!(l.surface().unwrap().format().sample, doc_depth(dest_depth));
            let pixel = l.surface().unwrap().pixel(55, 36);
            assert_eq!(pixel.last().copied(), Some(1.0));
            // Validate the converted colour against the CMS transform, independent of paste.
            let source_doc = &s.documents()[0].doc;
            let dest_doc = &s.active().unwrap().doc;
            let t = photocraft_cms::Transform::new(
                &crate::color_cmds::document_profile(source_doc),
                &crate::color_cmds::document_profile(dest_doc),
                s.color.settings.intent(),
                s.color.settings.bpc,
            )
            .unwrap();
            let source_pixel = original.surface().unwrap().pixel(25, 12);
            let mut expected = vec![0.0; dest_doc.mode.color_channels()];
            t.eval(&source_pixel[..3], &mut expected);
            for (actual, expected) in pixel.iter().zip(expected) {
                assert!((actual - expected).abs() < 0.015, "{source_depth} -> {mode}/{dest_depth}: {pixel:?}");
            }
            assert!(s.documents()[0].doc.layer(src).unwrap().mask == original.mask);
            s.undo();
            assert!(s.active().unwrap().doc.layer(id).is_none());
            s.redo();
            assert_eq!(layer(&s, id).mask.as_ref().unwrap().surface.sample_channel(42, 36, 0), 0.0);
        }
    }
}

#[test]
fn selections_copy_merged_and_explicit_pixels_do_not_carry_layer_masks() {
    let (mut s, src) = masked(16);
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 6, "height": 8})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().layers.is_none());
    let id = pasted(&mut s, json!({}));
    assert!(layer(&s, id).mask.is_none());
    assert_eq!(layer(&s, id).surface().unwrap().content_bounds(), Rect::new(10, 8, 16, 16));
    s.select_layer(src).unwrap();
    s.execute("edit.copy", json!({"pixels": true})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().layers.is_none());
    s.execute("edit.copyMerged", json!({})).unwrap();
    let clip = s.clipboard.as_ref().unwrap();
    assert!(clip.layers.is_none());
    let expected = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(25, 12, 26, 13)).px[0];
    for (a, b) in clip.surface.rgba(25, 12).iter().zip(expected) {
        assert!((a - b).abs() < 2.0 / 65535.0);
    }
}

#[test]
fn copying_an_active_mask_channel_or_quick_mask_copies_the_plane_only() {
    for depth in [8, 16, 32] {
        let (mut s, src) = masked(depth);
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.copy", json!({"target": "mask"})).unwrap();
        let clip = s.clipboard.as_ref().unwrap();
        assert!(clip.layers.is_none());
        assert_eq!(clip.surface.pixel(12, 12), vec![0.0, 1.0]);
        assert!((clip.surface.pixel(21, 12)[0] - 0.5).abs() < 0.005);
        let id = pasted(&mut s, json!({"target": "pixels"}));
        assert!(layer(&s, id).mask.is_none());
        assert_eq!(layer(&s, id).surface().unwrap().rgba(12, 12), [0.0, 0.0, 0.0, 1.0]);
        s.select_layer(src).unwrap();
        s.execute("channel.target", json!({"channel": "green"})).unwrap();
        s.execute("edit.copy", json!({})).unwrap();
        assert!((s.clipboard.as_ref().unwrap().surface.pixel(25, 12)[0] - 0.2).abs() < 0.005);
        s.execute("channel.target", json!({"channel": "composite"})).unwrap();
        s.execute("select.editInQuickMaskMode", json!({})).unwrap();
        s.execute("edit.paste", json!({})).unwrap();
        assert!((s.active().unwrap().doc.quick_mask.as_ref().unwrap().surface.sample_channel(25, 12, 0) - 0.2).abs() < 0.005);
    }
}

#[test]
fn invalid_copy_and_paste_leave_document_and_clipboard_unchanged() {
    let (mut s, _) = masked(8);
    s.execute("edit.copy", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let bounds = s.clipboard.as_ref().unwrap().bounds;
    for p in [json!({"pixels": "yes"}), json!({"target": "bogus"}), json!({"target": {"channel": -1}}), json!({"target": "mask", "pixels": 3})] {
        assert!(s.execute("edit.copy", p.clone()).is_err());
        assert!(s.execute("edit.paste", p).is_err());
    }
    for center in [json!([]), json!([1]), json!(["x", 1]), json!([1e30, 0]), json!([1, 2, 3])] {
        assert!(s.execute("edit.paste", json!({"center": center})).is_err());
    }
    assert!(*s.active().unwrap().doc == *before);
    assert_eq!(s.clipboard.as_ref().unwrap().bounds, bounds);
    assert!(s.clipboard.as_ref().unwrap().layers.is_some());
    s.clipboard.as_mut().unwrap().bounds = Rect::new(0, 0, 1_000_000, 1_000_000);
    assert!(s.execute("edit.paste", json!({})).is_err());
    assert!(*s.active().unwrap().doc == *before);
    s.clipboard.as_mut().unwrap().bounds = Rect::EMPTY;
    assert!(s.execute("edit.paste", json!({})).is_err());
    assert!(*s.active().unwrap().doc == *before);
    s.clipboard.as_mut().unwrap().bounds = bounds;
    s.clipboard.as_mut().unwrap().layers.as_mut().unwrap().layers.clear();
    assert!(s.execute("edit.paste", json!({})).is_err());
    assert!(*s.active().unwrap().doc == *before);
}

#[test]
fn clipboard_to_new_documents_keeps_masks_offsets_and_unique_document_ids() {
    let (mut s, src) = masked(16);
    let original = layer(&s, src).clone();
    s.execute("edit.copy", json!({})).unwrap();
    let r = s.execute("file.newFromClipboard", json!({})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(20), Some(16)));
    let first = s.active().unwrap().doc.id;
    let l = &s.active().unwrap().doc.layers[0];
    assert_eq!(l.surface().unwrap().content_bounds(), Rect::new(0, 0, 20, 16));
    assert_eq!(l.mask.as_ref().unwrap().surface.sample_channel(2, 4, 0), 0.0);
    assert_eq!(l.mask.as_ref().unwrap().density, original.mask.as_ref().unwrap().density);
    s.execute("file.newFromClipboard", json!({})).unwrap();
    assert_ne!(s.active().unwrap().doc.id, first);
}

#[test]
fn group_pastes_assign_new_ids_to_every_child_and_keep_masks_styles_and_psd_metadata() {
    let (mut s, src) = masked(8);
    s.edit("fixture", |doc, _| {
        let l = doc.layer_mut(src).unwrap();
        l.psd_id = Some(123);
        l.psd_blocks.push((*b"test", std::sync::Arc::new(vec![1, 2, 3])));
        l.opacity = 0.6;
        l.fill_opacity = 0.7;
        l.effects.items.push(photocraft_doc::Effect::ColorOverlay {
            color: photocraft_color::Color::BLACK,
            common: photocraft_doc::FxCommon::new(photocraft_color::BlendMode::Normal, 0.2),
        });
        Ok(())
    })
    .unwrap();
    let original = layer(&s, src).clone();
    s.execute("layer.groupLayers", json!({})).unwrap();
    let source_group = s.active().unwrap().active_layer.unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let first = pasted(&mut s, json!({}));
    let second = pasted(&mut s, json!({}));
    let a = &layer(&s, first).children().unwrap()[0];
    let b = &layer(&s, second).children().unwrap()[0];
    assert_ne!(first, source_group);
    assert_ne!(a.id, src);
    assert_ne!(a.id, b.id);
    assert!(a.mask == original.mask);
    assert_eq!(a.psd_id, None);
    assert_eq!(a.psd_blocks, original.psd_blocks);
    assert_eq!(a.effects, original.effects);
    assert_eq!((a.opacity, a.fill_opacity), (0.6, 0.7));
}

#[test]
fn same_mode_profile_conversion_changes_colour_but_keeps_mask_coverage() {
    use photocraft_cms::{Builtin, Transform};
    let (mut s, src) = masked(16);
    s.edit("source profile", |doc, _| {
        doc.icc_profile = Some(Builtin::AdobeRgbCompat.profile().to_bytes());
        Ok(())
    })
    .unwrap();
    let original = layer(&s, src).clone();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 16})).unwrap();
    s.edit("destination profile", |doc, _| {
        doc.icc_profile = Some(Builtin::DisplayP3.profile().to_bytes());
        Ok(())
    })
    .unwrap();
    let id = pasted(&mut s, json!({}));
    let t = Transform::new(Builtin::AdobeRgbCompat.profile(), Builtin::DisplayP3.profile(), s.color.settings.intent(), s.color.settings.bpc).unwrap();
    let mut expected = [0.0; 3];
    t.eval(&original.surface().unwrap().pixel(25, 12)[..3], &mut expected);
    let actual = layer(&s, id).surface().unwrap().pixel(25, 12);
    for (a, b) in actual.iter().zip(expected) {
        assert!((a - b).abs() < 0.001);
    }
    assert!(layer(&s, id).mask == original.mask);
    assert_ne!(actual, original.surface().unwrap().pixel(25, 12));
    s.execute("document.activate", json!({"document": 0})).unwrap();
    s.execute("channel.target", json!({"channel": "Green"})).unwrap();
    s.execute("edit.copyMerged", json!({})).unwrap();
    assert_eq!(s.clipboard.as_ref().unwrap().profile.as_ref().unwrap().content_hash(), photocraft_cms::Builtin::AdobeRgbCompat.profile().content_hash());
}

#[test]
fn explicit_pixel_paste_without_a_document_creates_a_bitmap_document() {
    let (mut s, _) = masked(8);
    let profile = photocraft_cms::Builtin::AdobeRgbCompat.profile().to_bytes();
    s.edit("source profile", |doc, _| {
        doc.icc_profile = Some(profile.clone());
        Ok(())
    })
    .unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    while s.active().is_some() {
        s.execute("file.close", json!({"discard": true})).unwrap();
    }
    s.execute("edit.paste", json!({"pixels": true})).unwrap();
    assert!(s.active().unwrap().doc.layers[0].mask.is_none());
    assert_eq!(s.active().unwrap().doc.icc_profile, Some(profile));
    assert!(s.clipboard.as_ref().unwrap().layers.is_some());
}

#[test]
fn copy_merged_encodes_composited_colour_in_the_document_profile_at_every_depth() {
    for depth in [8, 16, 32] {
        for mode in ["grayscale", "cmyk", "lab"] {
            let (mut s, _) = masked(depth);
            s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
            let doc = &s.active().unwrap().doc;
            let composite = photocraft_compose::render(doc, Rect::new(25, 12, 26, 13));
            let transform = photocraft_cms::Transform::new(
                &crate::color_cmds::composite_profile(doc),
                &crate::color_cmds::document_profile(doc),
                s.color.settings.intent(),
                s.color.settings.bpc,
            )
            .unwrap();
            let mut expected = vec![0.0; doc.mode.color_channels()];
            transform.eval(&composite.px[0][..3], &mut expected);
            s.execute("edit.copyMerged", json!({})).unwrap();
            let clip = s.clipboard.as_ref().unwrap();
            assert!(clip.layers.is_none());
            assert_eq!(clip.surface.format().sample, doc_depth(depth));
            let pixel = clip.surface.pixel(25, 12);
            for (a, b) in pixel.iter().zip(expected) {
                assert!((a - b).abs() < 0.01, "{mode}/{depth}: {pixel:?}");
            }
        }
    }
}

#[test]
fn masks_with_no_tiles_copy_black_and_white_defaults_without_a_selection() {
    for depth in [8, 16, 32] {
        for value in [0.0, 1.0] {
            let (mut s, id) = masked(depth);
            s.edit("uniform mask", |doc, _| {
                doc.layer_mut(id).unwrap().mask.as_mut().unwrap().surface =
                    Surface::with_default(PixelFormat::new(ColorMode::Grayscale, doc_depth(depth), false), &[value]);
                Ok(())
            })
            .unwrap();
            s.execute("edit.copy", json!({"target": "mask"})).unwrap();
            let clip = s.clipboard.as_ref().unwrap();
            assert!(clip.layers.is_none());
            assert_eq!(clip.bounds, Rect::new(0, 0, 64, 48));
            assert_eq!(clip.surface.pixel(40, 30), vec![value, 1.0]);
        }
    }
}
