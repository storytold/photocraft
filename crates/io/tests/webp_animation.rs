use photocraft_codecs::{self as codecs, AnimationFrame, ChannelLayout, DecodeOptions, EncodeOptions, Image, SampleType as CSample};
use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Layer, LayerContent};
use photocraft_io::{ExportOptions, export, import};

fn fixture(loops: u16) -> Vec<u8> {
    let frames: Vec<_> = [([255, 0, 0, 255], 0), ([0, 255, 0, 128], 75), ([0, 0, 255, 255], 123)]
        .into_iter()
        .map(|(c, ms)| AnimationFrame { image: Image::from_raw(4, 3, ChannelLayout::Rgba, CSample::U8, c.repeat(12)).unwrap(), delay_ms: ms })
        .collect();
    codecs::encode_webp_animation(&frames, loops, &EncodeOptions::default()).unwrap()
}

#[test]
fn open_edit_native_save_then_webp_save_preserves_every_frame_and_loop_count() {
    let original = fixture(7);
    let imported = import("animation.webp", &original).unwrap();
    assert!(imported.warnings.is_empty());
    let mut doc = imported.document;
    assert_eq!(doc.file_encoding.webp_animation_loops, Some(7));
    assert_eq!(doc.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Frame1 (0 ms)", "Frame2 (75 ms)", "Frame3 (123 ms)"]);
    assert_eq!(doc.layers.iter().map(|l| l.visible).collect::<Vec<_>>(), [true, false, false]);
    assert!(doc.layers.iter().all(|l| !l.locks.position && !l.locks.transparency));
    // Edit a hidden frame, rename its timing, and reorder the layers. All must affect save.
    let bounds = doc.bounds();
    doc.layers[1].surface_mut().unwrap().fill_rect(bounds, &[0.0, 0.0, 1.0, 0.5]);
    doc.layers[1].name = "Edited (91 ms)".into();
    doc.layers.swap(0, 2);
    let native = export(&doc, "saved.pcraft", &ExportOptions::default()).unwrap();
    let back = import("saved.pcraft", &native.bytes).unwrap().document;
    assert_eq!(back.file_encoding, doc.file_encoding);
    assert_eq!(back.layers, doc.layers);
    let saved = export(&back, "saved.webp", &ExportOptions::default()).unwrap();
    assert!(!saved.warnings.iter().any(|w| w.contains("layers flattened")));
    let decoded = codecs::decode_webp_frames(&saved.bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.loop_count, Some(7));
    assert_eq!(decoded.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [123, 91, 0]);
    assert_eq!(&decoded.frames[1].image.to_rgba8()[..4], &[0, 0, 255, 128]);
    assert_eq!(&decoded.frames[0].image.to_rgba8()[..4], &[0, 0, 255, 255]);
    // A non-WebP export is still supported, with an honest animation-metadata loss warning.
    assert!(export(&back, "saved.psd", &ExportOptions::default()).unwrap().warnings.iter().any(|w| w.contains("loop count")));
}

#[test]
fn deletion_duplication_and_duration_validation_follow_the_layer_stack() {
    let mut doc = import("a.webp", &fixture(0)).unwrap().document;
    doc.layers.remove(1);
    let mut dup = doc.layers[0].clone();
    dup.name = "Duplicate (9 ms)".into();
    doc.layers.push(dup);
    let saved = export(&doc, "a.webp", &ExportOptions::default()).unwrap();
    let decoded = codecs::decode_webp_frames(&saved.bytes, &DecodeOptions::default()).unwrap();
    assert_eq!(decoded.loop_count, Some(0));
    assert_eq!(decoded.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(), [0, 123, 9]);
    for bad in ["No timing", "Bad (-1 ms)", "Bad (16777216 ms)", "Bad (1.5 ms)", "Bad (+10 ms)"] {
        doc.layers[0].name = bad.into();
        assert!(export(&doc, "a.webp", &ExportOptions::default()).is_err(), "{bad}");
    }
    doc.layers[0].name = "Frame1 (10 ms)".into();
    doc.layers[0].blend = BlendMode::Multiply;
    assert!(export(&doc, "a.webp", &ExportOptions::default()).is_err());
    doc.layers[0].blend = BlendMode::Normal;
    doc.layers[0].clipped = true;
    assert!(export(&doc, "a.webp", &ExportOptions::default()).is_err());
    doc.layers[0].clipped = false;
    doc.layers.push(Layer::new("Group (50 ms)", LayerContent::Group(photocraft_doc::Group { children: Vec::new(), expanded: true, artboard: None })));
    assert!(export(&doc, "a.webp", &ExportOptions::default()).is_err());
}

#[test]
fn frame_adapter_keeps_timing_and_stops_on_callback_error() {
    let mut calls = Vec::new();
    assert_eq!(
        photocraft_io::import_webp_frames("a.webp", &fixture(0), &mut |doc, ms| {
            calls.push((doc.layers.len(), ms));
            Ok(())
        })
        .unwrap(),
        3
    );
    assert_eq!(calls, [(1, 0), (1, 75), (1, 123)]);
    let mut count = 0;
    assert!(
        photocraft_io::import_webp_frames("a.webp", &fixture(0), &mut |_, _| {
            count += 1;
            Err(photocraft_io::IoError::Cancelled)
        })
        .is_err()
    );
    assert_eq!(count, 1);
}

#[test]
fn still_webp_and_old_native_documents_remain_stills() {
    let image = Image::from_raw(1, 1, ChannelLayout::Rgba, CSample::U8, vec![20, 30, 40, 255]).unwrap();
    let bytes = codecs::encode(&image, codecs::Format::WebP, &EncodeOptions::default()).unwrap();
    let doc = import("still.webp", &bytes).unwrap().document;
    assert_eq!(doc.layers.len(), 1);
    assert_eq!(doc.file_encoding.webp_animation_loops, None);
    let saved = export(&doc, "still.webp", &ExportOptions::default()).unwrap();
    assert_eq!(codecs::webp_animation_info(&saved.bytes).unwrap(), None);
    assert!(serde_json::from_str::<photocraft_doc::FileEncoding>("{}").unwrap().is_empty());
}

#[test]
fn masks_opacity_and_multiple_document_depths_are_rendered_per_frame() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut doc = photocraft_doc::Document::new("Frames", photocraft_geom::Size::new(3, 2), mode, depth);
            doc.file_encoding.webp_animation_loops = Some(1);
            for i in 0..2 {
                let mut l = Layer::raster(format!("Frame{} (45 ms)", i + 1), doc.pixel_format());
                let color = photocraft_raster::from_rgba(&doc.pixel_format(), [0.2, 0.4, 0.6, 1.0]);
                l.surface_mut().unwrap().fill_rect(doc.bounds(), &color);
                l.opacity = 0.5;
                let mut mask = photocraft_doc::LayerMask::reveal_all();
                mask.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 1, 2), &[0.0]);
                l.mask = Some(mask);
                l.visible = i == 0;
                doc.layers.push(l);
            }
            let saved = export(&doc, "frames.webp", &ExportOptions::default()).unwrap();
            let decoded = codecs::decode_webp_frames(&saved.bytes, &DecodeOptions::default()).unwrap();
            assert_eq!(decoded.frames.len(), 2, "{mode:?} {depth:?}");
            let pixels = decoded.frames[1].image.to_rgba8();
            assert_eq!(pixels[3], 0, "hidden frame mask: {mode:?} {depth:?}");
            assert_eq!(pixels[7], 128, "hidden frame opacity: {mode:?} {depth:?}");
        }
    }
}
