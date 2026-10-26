use super::*;
use photocraft_ai::{PromptPreparation, WorkflowId};

pub(super) fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    let mut d = Document::new("Synthetic AI test", photocraft_geom::Size::new(8, 6), mode, depth);
    let mut layer = Layer::raster("Original", d.pixel_format());
    let n = d.pixel_format().channels();
    layer.surface_mut().unwrap().write_region(d.bounds(), &vec![0.25; 8 * 6 * n]);
    d.layers.push(layer);
    s.add_document(d, None);
    s
}

pub(super) fn prepare(s: &Session, source: Source, operation: Operation, margin: u32) -> PreparedInput {
    let snapshot = Snapshot::capture(s.active().unwrap(), source, margin).unwrap();
    let request = AiRequest {
        operation,
        workflow: WorkflowId { name: "synthetic".into(), revision: 1 },
        prompt: "a blue square".into(),
        negative_prompt: None,
        size: size(snapshot.roi).unwrap(),
        variants: 1,
        seed: 0,
        source: None,
        edit_mask: None,
        references: vec![],
        prompt_preparation: PromptPreparation::Manual,
    };
    snapshot.prepare(request).unwrap()
}

pub(super) fn image_result(input: &PreparedInput, colour: [f32; 4]) -> AiResult {
    AiResult {
        provider_job: None,
        prepared_prompt: None,
        artifacts: vec![Artifact::Image(RgbaImage::new(input.request.size, colour.repeat(input.request.size.pixels().unwrap())).unwrap())],
    }
}

fn soft_selection(s: &mut Session) {
    s.edit("Select", |doc, _| {
        let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        mask.write_region(Rect::new(2, 2, 4, 3), &[0.25, 1.0]);
        doc.selection = Some(mask);
        Ok(())
    })
    .unwrap();
}

#[test]
fn explicit_apply_soft_mask_and_undo_preserve_original_at_both_depths() {
    for depth in [SampleType::U8, SampleType::U16] {
        let mut s = session(ColorMode::Rgb, depth);
        soft_selection(&mut s);
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let input = prepare(&s, Source::Composite, Operation::Fill, 1);
        assert_eq!(input.target.roi, Rect::new(1, 1, 5, 4));
        assert_eq!(input.request.edit_mask.as_ref().unwrap().values().iter().filter(|v| **v > 0.0).count(), 2);
        let result = image_result(&input, [0.0, 0.0, 1.0, 1.0]);
        let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().revision, revision);
        s.execute("ai.result.apply", json!({"result":id,"coalesce":"unrelated"})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.depth, depth);
        assert_eq!(st.doc.layers[0], before.layers[0]);
        let layer = st.doc.layers.last().unwrap();
        assert_eq!(layer.surface().unwrap().format().sample, depth);
        assert_eq!(layer.surface().unwrap().sample_channel(2, 2, 3), 1.0); // mask not baked into alpha
        assert_eq!(layer.mask.as_ref().unwrap().value(2, 2), 0.25);
        assert_eq!(layer.mask.as_ref().unwrap().value(1, 2), 0.0);
        let old = photocraft_compose::render(&before, before.bounds());
        let new = photocraft_compose::render(&st.doc, st.doc.bounds());
        for (i, (a, b)) in old.px.iter().zip(&new.px).enumerate() {
            if ![18, 19].contains(&i) {
                assert_eq!(a, b, "outside selection pixel {i}");
            }
        }
        assert!(s.undo());
        assert_eq!(*s.active().unwrap().doc, *before);
        assert!(s.redo());
        assert_eq!(s.active().unwrap().doc.layers.len(), 2);
    }
}

#[test]
fn applies_to_captured_document_without_switching_tabs() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [0.0, 1.0, 0.0, 1.0]);
    let document = input.target.document;
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    s.add_document(Document::new("Other", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::U16), None);
    let other = s.active().unwrap().doc.clone();
    let applied = s.execute("ai.result.apply", json!({"result":id})).unwrap();
    assert_eq!(applied["document"], document.0);
    assert_eq!(s.active_index(), Some(1));
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &other));
    assert_eq!(s.documents()[0].doc.layers.len(), 2);
    s.set_active(0);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn saving_and_group_disclosure_do_not_stale_an_unchanged_ai_target() {
    for after_completion in [false, true] {
        let mut s = session(ColorMode::Rgb, SampleType::U16);
        let group = Layer::group("Folder", vec![]);
        let group_id = group.id;
        Arc::make_mut(&mut s.active_mut().unwrap().doc).layers.push(group);
        let input = prepare(&s, Source::Composite, Operation::Generate, 0);
        let result = image_result(&input, [0.2, 0.4, 0.8, 1.0]);
        let mut ready = Some(input.finish(result).unwrap());
        let id = if after_completion { Some(s.retain_ai_result(ready.take().unwrap()).unwrap()) } else { None };
        s.active_mut().unwrap().saved_to("renamed.pcraft".into());
        s.execute("layer.setExpanded", json!({"layer":group_id.0,"expanded":false})).unwrap();
        let id = match id {
            Some(id) => id,
            None => s.retain_ai_result(ready.take().unwrap()).unwrap(),
        };
        for _ in 0..2 {
            assert_eq!(s.execute("ai.result.list", json!({})).unwrap()["results"][0]["stale"], false);
        }
        let original = s.active().unwrap().doc.clone();
        s.execute("ai.result.apply", json!({"result":id})).unwrap();
        assert_eq!(s.active().unwrap().doc.name, "renamed.pcraft");
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc, original);
    }
}

#[test]
fn same_revision_pixel_mutation_still_blocks_ai_application_after_a_cached_view_comparison() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    s.active_mut().unwrap().saved_to("renamed.pcraft".into());
    assert_eq!(s.execute("ai.result.list", json!({})).unwrap()["results"][0]["stale"], false);
    Arc::make_mut(&mut s.active_mut().unwrap().doc).layers[0].surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(s.execute("ai.result.list", json!({})).unwrap()["results"][0]["stale"], true);
    assert!(s.execute("ai.result.apply", json!({"result":id})).is_err());
}

#[test]
fn stale_ai_result_can_open_its_captured_composition_without_changing_the_edited_original() {
    let mut s = session(ColorMode::Rgb, SampleType::U16);
    let captured = s.active().unwrap().doc.clone();
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [0.2, 0.4, 0.8, 1.0]);
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    s.execute("layer.new.layer", json!({"name":"Later edit"})).unwrap();
    let edited = s.active().unwrap().doc.clone();
    assert!(s.execute("ai.result.apply", json!({"result":id})).is_err());
    let response = s.execute("ai.result.openDocument", json!({"result":id})).unwrap();
    let opened = s.active().unwrap();
    assert_eq!(response["document"], opened.doc.id.0);
    assert_ne!(opened.doc.id, edited.id);
    assert_eq!(opened.doc.size, captured.size);
    assert_eq!(opened.doc.depth, captured.depth);
    assert_eq!(opened.doc.layers.len(), captured.layers.len() + 1);
    assert_eq!(opened.doc.layers[0], captured.layers[0]);
    assert!(opened.is_dirty());
    assert_eq!(s.documents()[0].doc, edited);
    assert!(s.documents()[0].history.can_undo());
    for params in [json!({}), json!({"result":id,"variant":999}), json!({"result":id,"name":null})] {
        let count = s.documents().len();
        assert!(s.execute("ai.result.openDocument", params).is_err());
        assert_eq!(s.documents().len(), count);
    }
}

#[test]
fn stale_result_remains_inspectable_but_cannot_mutate_document() {
    let mut s = session(ColorMode::Rgb, SampleType::U16);
    let input = prepare(&s, Source::ActiveRaster, Operation::Edit, 0);
    let result = image_result(&input, [1.0; 4]);
    s.edit("Unrelated change", |doc, _| {
        doc.name = "Changed".into();
        Ok(())
    })
    .unwrap();
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute("ai.result.list", json!({})).unwrap()["results"][0]["stale"].as_bool().unwrap());
    assert!(s.execute("ai.result.apply", json!({"result":id})).unwrap_err().to_string().contains("changed"));
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(s.execute("ai.result.discard", json!({"result":id})).is_ok());
    assert_eq!(s.execute("ai.result.list", json!({})).unwrap()["bytes"], 0);
    assert!(s.undo());
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn icc_conversion_and_gray_are_used_without_changing_profile_or_depth() {
    for (mode, profile) in [(ColorMode::Rgb, Builtin::DisplayP3.profile()), (ColorMode::Grayscale, Builtin::GrayGamma22.profile())] {
        let mut s = session(mode, SampleType::U16);
        s.edit("Tag profile", |doc, _| {
            doc.icc_profile = Some(profile.to_bytes());
            Ok(())
        })
        .unwrap();
        let input = prepare(&s, Source::ActiveRaster, Operation::Edit, 0);
        let original = s.active().unwrap().doc.clone();
        let sf = original.layers[0].surface().unwrap();
        let src = sf.read_region(original.bounds());
        let t = transform::cached(profile, Builtin::Srgb.profile(), Default::default()).unwrap();
        let mut expected = vec![0.0; 8 * 6 * 4];
        t.convert_f32(&src, sf.channels(), &mut expected, 4, true);
        assert_eq!(input.request.source.as_ref().unwrap().samples(), expected);
        let result = image_result(&input, [0.3, 0.6, 0.4, 0.75]);
        let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
        s.execute("ai.result.apply", json!({"result":id})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.icc_profile, original.icc_profile);
        assert_eq!(doc.depth, SampleType::U16);
        assert_eq!(doc.mode, mode);
        let t = transform::cached(Builtin::Srgb.profile(), profile, Default::default()).unwrap();
        let mut want = vec![0.0; doc.pixel_format().channels()];
        t.convert_f32(&[0.3, 0.6, 0.4, 0.75], 4, &mut want, doc.pixel_format().channels(), true);
        let got = doc.layers.last().unwrap().surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
        for (a, b) in got.iter().zip(want) {
            assert!((*a - b).abs() < 2.0 / 65535.0);
        }
    }
}

#[test]
fn segmentation_keeps_native_pixels_and_editable_foreground_mask() {
    let mut s = session(ColorMode::Rgb, SampleType::U16);
    let before = s.active().unwrap().doc.clone();
    let input = prepare(&s, Source::ActiveRaster, Operation::SegmentSubject, 0);
    assert!(input.request.edit_mask.is_none());
    let result = AiResult {
        provider_job: None,
        prepared_prompt: None,
        artifacts: vec![Artifact::ForegroundMask(Mask::new(input.request.size, vec![0.5; 48]).unwrap())],
    };
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    s.execute("ai.result.apply", json!({"result":id})).unwrap();
    let d = &s.active().unwrap().doc;
    assert_eq!(d.layers[0], before.layers[0]);
    assert_eq!(d.layers[1].surface().unwrap().read_region(d.bounds()), before.layers[0].surface().unwrap().read_region(d.bounds()));
    assert_eq!(d.layers[1].mask.as_ref().unwrap().value(0, 0), 0.5);
    assert_eq!(d.layers[1].mask.as_ref().unwrap().value(-1, 0), 0.0);
    assert!(s.undo());
    assert_eq!(*s.active().unwrap().doc, *before);
}

#[test]
fn invalid_params_results_and_closed_documents_fail_atomically() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let bad = AiResult { provider_job: None, prepared_prompt: None, artifacts: vec![] };
    assert!(input.finish(bad).is_err());
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [0.0; 4]);
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    let before = s.active().unwrap().doc.clone();
    for p in [
        json!({}),
        json!({"result":-1}),
        json!({"result":"1"}),
        json!({"result":id,"variant":-1}),
        json!({"result":id,"variant":u64::MAX}),
        json!({"result":id,"variant":1}),
        json!({"result":999}),
    ] {
        assert!(s.execute("ai.result.apply", p).is_err());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    }
    assert!(!s.active().unwrap().history.can_undo());
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [0.0; 4]);
    s.close(0);
    assert_eq!(s.execute("ai.result.list", json!({})).unwrap()["bytes"], 0);
    assert!(s.retain_ai_result(input.finish(result).unwrap()).is_err());
    assert!(s.execute("ai.result.apply", json!({"result":id})).is_err());
}

#[test]
fn cache_limits_and_id_exhaustion_do_not_evict_existing_results() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    for _ in 0..16 {
        let input = prepare(&s, Source::Composite, Operation::Generate, 0);
        let result = image_result(&input, [1.0; 4]);
        s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    }
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    assert!(s.retain_ai_result(input.finish(result).unwrap()).is_err());
    assert_eq!(s.ai.targets.len(), 16);
    s.ai.discard(1);
    s.ai.next = u64::MAX;
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    assert!(s.retain_ai_result(input.finish(result).unwrap()).is_err());
    assert_eq!(s.ai.targets.len(), 15);
}

#[test]
fn unsupported_formats_invalid_profiles_and_empty_masks_fail_before_allocation() {
    for (mode, depth) in [(ColorMode::Rgb, SampleType::F32), (ColorMode::Cmyk, SampleType::U16), (ColorMode::Lab, SampleType::U8)] {
        let s = session(mode, depth);
        assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, 0).is_err());
    }
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, u32::MAX).is_err());
    s.edit("Bad profile", |d, _| {
        d.icc_profile = Some(Arc::new(vec![0; 4]));
        Ok(())
    })
    .unwrap();
    assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, 0).is_err());
    s.undo();
    s.edit("Empty selection", |d, _| {
        d.selection = Some(Surface::new(PixelFormat::GRAY8));
        Ok(())
    })
    .unwrap();
    assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, 0).is_err());
}

#[test]
fn external_job_completion_retains_preview_without_edit_and_cancel_discards_late_reply() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    let document = input.target.document;
    let before = s.active().unwrap().doc.clone();
    let job = s
        .start_external_job("ai.generate", "Synthetic generation", Some(document), |s, prepared| s.retain_ai_result(prepared).map(|id| json!({"result":id})))
        .unwrap();
    assert!(s.active_job().is_none());
    assert!(job.complete(Ok(input.finish(result).unwrap())));
    s.poll_jobs();
    assert_eq!(s.ai.targets.len(), 1);
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    let job =
        s.start_external_job("ai.generate", "Late reply", Some(document), |s, prepared| s.retain_ai_result(prepared).map(|id| json!({"result":id}))).unwrap();
    s.cancel_job(job.id);
    assert!(!job.complete(Ok(input.finish(result).unwrap())));
    s.poll_jobs();
    assert_eq!(s.ai.targets.len(), 1);
}

#[test]
fn automatic_prompt_pipeline_finishes_as_preview_then_applies() {
    use std::{
        future::Future,
        task::{Context, Poll, Wake, Waker},
    };
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    let mut s = session(ColorMode::Rgb, SampleType::U16);
    let mut input = prepare(&s, Source::Composite, Operation::Generate, 0);
    input.request.prompt_preparation = PromptPreparation::OpenAiText;
    let fake = photocraft_ai::fake::FakeProvider::default();
    let control = photocraft_ai::Control::default();
    let result = {
        let mut future = Box::pin(photocraft_ai::generate(input.request(), &fake, Some(&fake), &control));
        let waker = Waker::from(Arc::new(Noop));
        match future.as_mut().poll(&mut Context::from_waker(&waker)) {
            Poll::Ready(result) => result.unwrap(),
            Poll::Pending => panic!("deterministic fake unexpectedly pending"),
        }
    };
    assert_eq!(fake.preparations(), 1);
    assert_eq!(fake.submissions(), 1);
    let before = s.active().unwrap().doc.clone();
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert_eq!(s.ai_result(id).unwrap().prepared_prompt.as_ref().unwrap().text_model.as_deref(), Some("fake-text"));
    s.execute("ai.result.apply", json!({"result":id})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), 2);
    assert!(s.undo());
    assert_eq!(*s.active().unwrap().doc, *before);
}

#[test]
fn result_commands_do_not_drop_a_floating_selection_in_another_tab() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    let other = session(ColorMode::Rgb, SampleType::U8);
    s.add_document((*other.active().unwrap().doc).clone(), None);
    s.execute("select.rect", json!({"x":1,"y":1,"width":3,"height":2})).unwrap();
    s.execute("select.float", json!({"dx":2})).unwrap();
    let before = s.active().unwrap().doc.clone();
    s.execute("ai.result.list", json!({})).unwrap();
    s.execute("ai.result.apply", json!({"result":id})).unwrap();
    assert!(s.active().unwrap().floating.is_some());
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert_eq!(s.documents()[0].doc.layers.len(), 2);
}

#[test]
fn variants_wrong_provenance_and_memory_limits_are_checked() {
    let mut s = session(ColorMode::Rgb, SampleType::U16);
    let mut input = prepare(&s, Source::Composite, Operation::Generate, 0);
    input.request.variants = 2;
    let a = Artifact::Image(RgbaImage::new(input.request.size, [1.0, 0.0, 0.0, 1.0].repeat(48)).unwrap());
    let b = Artifact::Image(RgbaImage::new(input.request.size, [0.0, 1.0, 0.0, 1.0].repeat(48)).unwrap());
    let result = AiResult { provider_job: None, prepared_prompt: None, artifacts: vec![a, b] };
    let id = s.retain_ai_result(input.finish(result).unwrap()).unwrap();
    s.execute("ai.result.apply", json!({"result":id,"variant":1})).unwrap();
    let pixel = s.active().unwrap().doc.layers[1].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
    assert!(pixel[1] > 0.99 && pixel[0] < 0.01);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let mut result = image_result(&input, [1.0; 4]);
    result.prepared_prompt = Some(photocraft_ai::PreparedPrompt {
        original: "different request".into(),
        positive: "blue".into(),
        negative: None,
        workflow: input.request.workflow.clone(),
        text_model: None,
    });
    assert!(input.finish(result).is_err());
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    let ready = input.finish(result).unwrap();
    s.ai.target_bytes = CACHE_BYTES;
    let count = s.ai.targets.len();
    assert!(s.retain_ai_result(ready).is_err());
    assert_eq!(s.ai.targets.len(), count);
}

#[test]
fn oversized_roi_missing_selection_and_unmapped_expansion_fail_gracefully() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    for operation in [Operation::Fill, Operation::Remove, Operation::Expand] {
        let input = prepare(&s, Source::Composite, Operation::Generate, 0);
        let mut request = input.request;
        request.operation = operation;
        assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap().prepare(request).is_err());
    }
    s.add_document(Document::new("Large sparse image", photocraft_geom::Size::new(300_000, 300_000), ColorMode::Rgb, SampleType::U16), None);
    assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, 0).is_err());
    s.add_document(Document::new("Large sparse image", photocraft_geom::Size::new(8192, 8192), ColorMode::Rgb, SampleType::U16), None);
    let snapshot = Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap();
    let request = AiRequest {
        operation: Operation::Generate,
        workflow: WorkflowId { name: "large".into(), revision: 1 },
        prompt: "blue".into(),
        negative_prompt: None,
        size: snapshot.size(),
        variants: 1,
        seed: 0,
        source: None,
        edit_mask: None,
        references: vec![],
        prompt_preparation: PromptPreparation::Manual,
    };
    assert!(snapshot.prepare(request).is_err());
}

#[test]
fn hostile_selection_coordinates_are_rejected_without_scanning_them() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    s.edit("Hostile selection", |doc, _| {
        let mut mask = Surface::new(PixelFormat::GRAY8);
        let tile = mask.solid_tile(&[1.0]);
        mask.put_tiles([(photocraft_geom::TileCoord::new(i32::MIN, i32::MAX), tile)]);
        doc.selection = Some(mask);
        Ok(())
    })
    .unwrap();
    assert!(Snapshot::capture(s.active().unwrap(), Source::Composite, 0).is_err());
}

#[test]
fn reopened_persisted_document_id_cannot_receive_an_old_completion() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::Generate, 0);
    let result = image_result(&input, [1.0; 4]);
    let ready = input.finish(result).unwrap();
    let original = (*s.active().unwrap().doc).clone();
    s.close(0);
    s.add_document(original.clone(), None);
    assert_eq!(s.active().unwrap().doc.id, original.id);
    assert_eq!(s.active().unwrap().revision, 1);
    assert!(s.retain_ai_result(ready).is_err());
    assert_eq!(s.ai.targets.len(), 0);
    assert!(!s.active().unwrap().history.can_undo());
}

#[test]
fn expansion_shifts_original_once_preserves_pixels_and_undo_restores_geometry() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":32,"background":"#cc2200","depth":16})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let snapshot = Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap();
    let edges = Expansion { left: 8, top: 16, right: 8, bottom: 0 };
    let size = snapshot.expansion_size(edges).unwrap();
    let request = photocraft_ai::AiRequest {
        operation: Operation::Expand,
        workflow: photocraft_ai::WorkflowId { name: "qwen-image21".into(), revision: 1 },
        prompt: "extend the plain background".into(),
        negative_prompt: None,
        size,
        variants: 1,
        seed: 0,
        source: None,
        edit_mask: None,
        references: vec![],
        prompt_preparation: photocraft_ai::PromptPreparation::Manual,
    };
    let input = snapshot.prepare_expansion(request, edges).unwrap();
    assert_eq!(s.active().unwrap().doc.size, before.size);
    let result = AiResult {
        provider_job: None,
        prepared_prompt: None,
        artifacts: vec![Artifact::Image(RgbaImage::new(size, vec![1.0; size.pixels().unwrap() * 4]).unwrap())],
    };
    let prepared = input.finish(result).unwrap();
    let id = s.retain_ai_result(prepared).unwrap();
    s.execute("ai.result.apply", json!({"result":id})).unwrap();
    let after = &s.active().unwrap().doc;
    assert_eq!(after.size, photocraft_geom::Size::new(48, 48));
    assert_eq!(after.layers.len(), before.layers.len() + 1);
    let old = photocraft_compose::render(&before, before.bounds());
    let shifted = photocraft_compose::render(after, Rect::from_xywh(8, 16, 32, 32));
    assert_eq!(old.px, shifted.px);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.size, before.size);
    assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
    assert_eq!(photocraft_compose::render(&s.active().unwrap().doc, before.bounds()).px, old.px);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.size, photocraft_geom::Size::new(48, 48));
}

#[test]
fn foreground_apply_can_hide_originals_in_the_same_undo_step() {
    let mut s = session(ColorMode::Rgb, SampleType::U8);
    let input = prepare(&s, Source::Composite, Operation::SegmentSubject, 0);
    let size = input.request.size;
    let result = AiResult {
        provider_job: None,
        prepared_prompt: None,
        artifacts: vec![Artifact::ForegroundMask(Mask::new(size, vec![0.5; size.pixels().unwrap()]).unwrap())],
    };
    let prepared = input.finish(result).unwrap();
    let id = s.retain_ai_result(prepared).unwrap();
    s.execute("ai.result.apply", json!({"result":id,"hideOriginals":true})).unwrap();
    let doc = &s.active().unwrap().doc;
    assert!(!doc.layers.first().unwrap().visible);
    assert!(doc.layers.last().unwrap().visible);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.active().unwrap().doc.layers.first().unwrap().visible);
}

#[test]
fn background_swap_uses_inverse_foreground_and_previews_the_actual_masked_composite() {
    let s = session(ColorMode::Rgb, SampleType::U16);
    let snapshot = Snapshot::capture(s.active().unwrap(), Source::Composite, 0).unwrap();
    let size = snapshot.canvas_size();
    let request = AiRequest {
        operation: Operation::Edit,
        workflow: WorkflowId { name: "qwen-image21".into(), revision: 1 },
        prompt: "a blue background".into(),
        negative_prompt: None,
        size,
        variants: 1,
        seed: 0,
        source: None,
        edit_mask: None,
        references: vec![],
        prompt_preparation: PromptPreparation::Manual,
    };
    let mut input = snapshot.prepare_background(request).unwrap();
    let mut values = vec![1.0; size.pixels().unwrap()];
    values[0] = 0.0;
    input.use_background_mask(&Mask::new(size, values).unwrap()).unwrap();
    assert_eq!(input.request.operation, Operation::Fill);
    assert_eq!(input.request.edit_mask.as_ref().unwrap().values()[0], 1.0);
    assert_eq!(input.request.edit_mask.as_ref().unwrap().values()[1], 0.0);
    let result = image_result(&input, [0.0, 0.0, 1.0, 1.0]);
    let prepared = input.finish(result).unwrap();
    let preview = prepared.target.previews.first().unwrap();
    assert_eq!(preview.rgba.get(0..4), Some([0, 0, 255, 255].as_slice()));
    assert_ne!(preview.rgba.get(4..8), Some([0, 0, 255, 255].as_slice()));
}

#[test]
fn vision_input_is_bounded_and_keeps_document_and_history_unchanged() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16] {
            let s = session(mode, depth);
            let original = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let input = Snapshot::capture_canvas(s.active().unwrap(), Source::Composite).unwrap().analysis_image().unwrap();
            assert_eq!(input.size(), ImageSize { width: 8, height: 6 });
            assert!(input.samples().iter().all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
            assert!(Arc::ptr_eq(&original, &s.active().unwrap().doc));
            assert_eq!(s.active().unwrap().revision, revision);
        }
    }
    let mut s = Session::new();
    s.execute("file.new", json!({"width":2400,"height":1200})).unwrap();
    let input = Snapshot::capture_canvas(s.active().unwrap(), Source::Composite).unwrap().analysis_image().unwrap();
    assert_eq!(input.size(), ImageSize { width: 1024, height: 512 });
}
