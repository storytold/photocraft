use super::tests::{image_result, prepare, session};
use super::*;

#[test]
fn four_k_and_eight_k_generation_change_canvas_only_on_apply_and_undo() {
    for (width, height, mode, depth) in [(3840, 2160, ColorMode::Rgb, SampleType::U8), (7680, 4320, ColorMode::Grayscale, SampleType::U16)] {
        let mut s = session(mode, depth);
        let before = s.active().unwrap().doc.clone();
        let mut request = prepare(&s, Source::Composite, Operation::Generate, 0).request;
        request.size = ImageSize { width: 64, height: 36 };
        let snapshot = Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap();
        let input = snapshot.prepare_generation(request, ImageSize { width, height }).unwrap();
        let result = image_result(&input, [0.3, 0.6, 0.8, 1.0]);
        let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
        let preview = s.ai_preview(id, 0).unwrap();
        assert_eq!(preview.size, ImageSize { width: 1024, height: 576 });
        assert!(preview.rgba.len() <= 4 * 1024 * 1024);
        let list = s.execute("ai.result.list", json!({})).unwrap();
        assert_eq!(list["results"][0]["workingSize"]["width"], 64);
        assert_eq!(list["results"][0]["outputSize"]["width"], width);
        assert_eq!(list["results"][0]["resampled"], true);
        s.execute("ai.result.apply", json!({"result":id})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.size, photocraft_geom::Size::new(width, height));
        assert_eq!(doc.layers[0], before.layers[0]);
        let surface = doc.layers.last().unwrap().surface().unwrap();
        assert_eq!(surface.format().sample, depth);
        for (x, y) in [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)] {
            assert!(surface.pixel(x as i32, y as i32).last().copied().unwrap() > 0.99);
        }
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, *before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(width, height));
        s.execute("ai.result.discard", json!({"result":id})).unwrap();
    }
}

#[test]
fn scaled_selection_keeps_native_soft_edges_and_offset_pixels_exact() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut s = session(ColorMode::Rgb, depth);
        s.edit("Size and mask", |doc, _| {
            doc.size = photocraft_geom::Size::new(4096, 2048);
            let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            mask.fill_rect(Rect::new(513, 257, 3585, 1793), &[1.0]);
            mask.write_pixel(513, 257, &[0.25]);
            mask.write_pixel(1001, 800, &[0.0]);
            doc.selection = Some(mask);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        let snapshot = Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap();
        let mut request = prepare(&s, Source::Composite, Operation::Generate, 0).request;
        request.operation = Operation::Fill;
        request.size = ImageSize { width: 256, height: 128 };
        let input = snapshot.prepare(request).unwrap();
        assert_eq!(input.request.source.as_ref().unwrap().size(), ImageSize { width: 256, height: 128 });
        let result = image_result(&input, [0.0, 0.0, 1.0, 1.0]);
        let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
        s.execute("ai.result.apply", json!({"result":id})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.layers[0], before.layers[0]);
        let mask = doc.layers.last().unwrap().mask.as_ref().unwrap();
        assert_eq!(mask.value(513, 257), 0.25);
        assert_eq!(mask.value(1001, 800), 0.0);
        assert_eq!(mask.value(512, 257), 0.0);
        let region = Rect::new(998, 798, 1004, 804);
        let a = photocraft_compose::render(&before, region);
        let b = photocraft_compose::render(doc, region);
        assert_eq!(a.px[15], b.px[15]);
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, *before);
        assert!(s.redo());
    }
}

#[test]
fn working_size_retains_portrait_and_destination_budget_rejects_absurd_sizes() {
    assert_eq!(Snapshot::working_size(ImageSize { width: 4320, height: 7680 }, 1024).unwrap(), ImageSize { width: 576, height: 1024 });
    let s = session(ColorMode::Rgb, SampleType::U16);
    for output in [ImageSize { width: 0, height: 4320 }, ImageSize { width: u32::MAX, height: 1 }, ImageSize { width: 300_000, height: 300_000 }] {
        let snapshot = Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap();
        let request = prepare(&s, Source::Composite, Operation::Generate, 0).request;
        assert!(snapshot.prepare_generation(request, output).is_err());
    }
}

#[test]
fn detail_upscale_maps_overlapping_patches_without_gaps_and_checks_each_result() {
    let image = RgbaImage::new(ImageSize { width: 64, height: 32 }, [0.2, 0.4, 0.6, 1.0].repeat(64 * 32)).unwrap();
    let roi = Rect::from_xywh(513, 257, 4097, 1027);
    let mut calls = 0;
    let surface = scaling::detail_surface(PixelFormat::RGBA8, Builtin::Srgb.profile(), roi, &image, &mut |patch| {
        calls += 1;
        assert!(patch.size().width <= 640 && patch.size().height <= 640);
        let output = ImageSize { width: patch.size().width * 4, height: patch.size().height * 4 };
        Ok(RgbaImage::new(output, [0.2, 0.4, 0.6, 1.0].repeat(output.pixels().unwrap())).unwrap())
    })
    .unwrap();
    assert_eq!(calls, 3);
    for x in [0, 2047, 2048, 4095, 4096] {
        for y in [0, 512, 1026] {
            assert_eq!(surface.pixel(roi.x0 + x, roi.y0 + y), vec![0.2, 0.4, 0.6, 1.0]);
        }
    }
    assert_eq!(surface.pixel(roi.x0 - 1, roi.y0), vec![0.0; 4]);
    assert!(scaling::detail_surface(PixelFormat::RGBA8, Builtin::Srgb.profile(), roi, &image, &mut |_| Ok(image.clone())).is_err());
}

#[test]
fn native_selection_rejects_invalid_coverage_before_provider_submission() {
    let roi = Rect::new(0, 0, 4, 4);
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        mask.write_pixel(1, 1, &[value]);
        assert!(scaling::native_selection(&mask, roi).is_err());
    }
}

#[test]
fn standalone_upscale_uses_original_native_detail_and_preserves_alpha_undo_and_source_layers() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut s = Session::new();
        let mut doc = Document::new("Native upscale", photocraft_geom::Size::new(2048, 16), ColorMode::Rgb, depth);
        let mut layer = Layer::raster("Original", doc.pixel_format());
        for x in 0..2048 {
            layer
                .surface_mut()
                .unwrap()
                .fill_rect(Rect::from_xywh(x, 0, 1, 16), &[if x % 2 == 0 { 1.0 } else { 0.0 }, 0.0, 0.0, if x < 1024 { 0.25 } else { 0.0 }]);
        }
        doc.layers.push(layer);
        s.add_document(doc, None);
        let original = s.active().unwrap().doc.clone();
        let snapshot = Snapshot::capture_canvas(s.active().unwrap(), Source::Composite).unwrap();
        let mut witnessed_detail = false;
        let ready = snapshot
            .finish_upscale(ImageSize { width: 8192, height: 64 }, |patch| {
                witnessed_detail |= patch.samples().chunks_exact(4).any(|p| p[0] > 0.9) && patch.samples().chunks_exact(4).any(|p| p[0] < 0.1);
                let out = ImageSize { width: patch.size().width * 4, height: patch.size().height * 4 };
                let mut pixels = Vec::new();
                for y in 0..out.height {
                    for x in 0..out.width {
                        let pos = ((y / 4) * patch.size().width + x / 4) as usize * 4;
                        let source = patch.samples().get(pos..pos + 4).unwrap();
                        pixels.extend_from_slice(&[source[0], source[1], source[2], 1.0]);
                    }
                }
                RgbaImage::new(out, pixels).map_err(error)
            })
            .unwrap();
        assert!(witnessed_detail, "native alternating one-pixel detail was lost before the model");
        let id = s.retain_ai_result(ready).unwrap();
        let row = s.execute("ai.result.list", json!({})).unwrap();
        assert_eq!(row["results"][0]["operation"], "upscale");
        assert_eq!(row["results"][0]["workingSize"]["width"], 2048);
        s.execute("ai.result.apply", json!({"result":id,"hideOriginals":true})).unwrap();
        let current = &s.active().unwrap().doc;
        assert_eq!(current.size, photocraft_geom::Size::new(8192, 64));
        assert!(!current.layers[0].visible);
        assert_eq!(current.layers[0].surface(), original.layers[0].surface());
        let surface = current.layers.last().unwrap().surface().unwrap();
        let opaque = surface.read_region(Rect::from_xywh(100, 8, 1, 1));
        let transparent = surface.read_region(Rect::from_xywh(7000, 8, 1, 1));
        assert!((opaque[3] - 0.25).abs() < 0.005);
        assert_eq!(transparent[3], 0.0, "upscaler alpha must not make transparent source pixels opaque");
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, original);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(8192, 64));
    }
}

#[test]
fn native_eight_k_preparation_is_bounded_before_provider_submission() {
    let s = session(ColorMode::Rgb, SampleType::U16);
    let snapshot = Snapshot::capture_canvas(s.active().unwrap(), Source::Composite).unwrap();
    let output = ImageSize { width: 7680, height: 4320 };
    let request = AiRequest {
        operation: Operation::Generate,
        workflow: photocraft_ai::WorkflowId { name: "qwen-image21".into(), revision: 1 },
        prompt: "test".into(),
        negative_prompt: None,
        size: output,
        variants: 1,
        seed: 0,
        source: None,
        edit_mask: None,
        references: vec![],
        prompt_preparation: photocraft_ai::PromptPreparation::Manual,
    };
    assert_eq!(snapshot.prepare_generation(request.clone(), output).unwrap().request().size, output);
    let mut oversized = request;
    oversized.size = ImageSize { width: 8192, height: 8192 };
    let snapshot = Snapshot::capture_canvas(s.active().unwrap(), Source::Composite).unwrap();
    assert!(snapshot.prepare_generation(oversized.clone(), oversized.size).is_err());
}
