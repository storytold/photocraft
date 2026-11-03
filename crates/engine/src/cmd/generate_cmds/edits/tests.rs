use super::*;

fn photo(depth: u32, mode: &str, w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":w,"height":h,"depth":depth,"mode":mode})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Original"})).unwrap();
    s.execute("edit.fill", json!({"contents":"color","color":"#b86b35"})).unwrap();
    s
}
fn result_surface(s: &Session, w: u32, h: u32) -> Surface {
    let fmt = s.active().unwrap().doc.pixel_format();
    let mut out = Surface::new(fmt);
    let mut p = vec![0.2; fmt.channels()];
    if fmt.alpha {
        *p.last_mut().unwrap() = 1.0;
    }
    out.fill_rect(Rect::from_xywh(0, 0, w, h), &p);
    out
}

#[test]
fn edits_never_change_protected_pixels_or_original_layers_at_any_working_depth() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            let mut s = photo(depth, mode, 64, 64);
            s.execute("select.rect", json!({"x":16,"y":16,"width":24,"height":24})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let past = s.active().unwrap().history.past_len();
            let expected = photocraft_compose::render(&before, before.bounds());
            let edit = selection_edit(&s).unwrap();
            let dims = (edit.region.width(), edit.region.height());
            let encoded = source(&before, &edit, &crate::jobs::JobCtx::new()).unwrap();
            let bytes = photocraft_paint::tile::b64_decode(&encoded).unwrap();
            validate_png(&bytes, dims.0, dims.1).unwrap();
            let pixels = result_surface(&s, dims.0, dims.1);
            apply_edit(&mut s, pixels, edit, "Fill Selection", 42, "mock").unwrap();
            let st = s.active().unwrap();
            assert_eq!(st.doc.selection, before.selection);
            assert_eq!(st.doc.icc_profile, before.icc_profile);
            assert_eq!(st.history.past_len(), past + 1);
            for layer in &before.layers {
                assert_eq!(st.doc.layer(layer.id), Some(layer));
            }
            let actual = photocraft_compose::render(&st.doc, st.doc.bounds());
            for y in 0..64 {
                for x in 0..64 {
                    if !(16..40).contains(&x) || !(16..40).contains(&y) {
                        assert_eq!(actual.px[y * 64 + x], expected.px[y * 64 + x], "{mode}/{depth} at {x},{y}");
                    }
                }
            }
            assert_ne!(actual.px[24 * 64 + 24], expected.px[24 * 64 + 24]);
            let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
            assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().layers, st.doc.layers);
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
            assert!(s.redo());
        }
    }
}

#[test]
fn large_documents_use_bounded_selection_context_without_downsampling() {
    let mut s = photo(8, "rgb", 2400, 1800);
    s.execute("select.rect", json!({"x":1400,"y":1100,"width":128,"height":96})).unwrap();
    let edit = selection_edit(&s).unwrap();
    assert!(edit.region.width() <= 1024);
    assert!(edit.region.x0 > 0);
    assert_eq!(edit.mask.sample_channel(1401, 1101, 0), 1.0);
    assert_eq!(edit.mask.sample_channel(edit.region.x0, edit.region.y0, 0), 0.0);
    let png = source(&s.active().unwrap().doc, &edit, &crate::jobs::JobCtx::new()).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(&png).unwrap();
    validate_png(&bytes, edit.region.width(), edit.region.height()).unwrap();
    let image = photocraft_codecs::decode(&bytes).unwrap();
    assert_eq!(image.width(), edit.region.width());
    let mask = photocraft_paint::tile::b64_decode(&mask_png(&edit).unwrap()).unwrap();
    let m = photocraft_codecs::decode(&mask).unwrap();
    assert_eq!((m.width(), m.height()), (edit.region.width(), edit.region.height()));
    s.execute("select.all", json!({})).unwrap();
    assert!(selection_edit(&s).is_err());
}

#[test]
fn expansion_preserves_original_region_including_transparency_and_undo() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            let mut s = photo(depth, mode, 64, 64);
            let background = s.active().unwrap().doc.layers[0].id;
            s.edit("transparent document", |doc, _| {
                doc.remove(background);
                Ok(())
            })
            .unwrap();
            s.execute("select.rect", json!({"x":8,"y":8,"width":16,"height":16})).unwrap();
            s.execute("edit.clear", json!({})).unwrap();
            let before = s.active().unwrap().doc.clone();
            let expected = photocraft_compose::render(&before, before.bounds());
            let edit = expansion(&s, &json!({"left":8,"right":8,"top":8,"bottom":8})).unwrap();
            let surface = result_surface(&s, 80, 80);
            apply_edit(&mut s, surface, edit, "Expand Canvas", 42, "mock").unwrap();
            let after = s.active().unwrap();
            assert_eq!(after.doc.size, Size::new(80, 80));
            let actual = photocraft_compose::render(&after.doc, after.doc.bounds());
            for y in 0..64 {
                for x in 0..64 {
                    assert_eq!(actual.px[(y + 8) * 80 + x + 8], expected.px[y * 64 + x], "{mode}/{depth} at {x},{y}");
                }
            }
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.size, before.size);
            assert_eq!(s.active().unwrap().doc.layers, before.layers);
        }
    }
}

#[test]
fn invalid_edits_fail_before_runtime_access_and_preserve_history() {
    let mut s = photo(8, "rgb", 64, 64);
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    for (id, p) in [
        (FILL, json!({"prompt":"blue"})),
        (EXPAND, json!({"prompt":"landscape"})),
        (EXPAND, json!({"prompt":"landscape","left":u64::MAX})),
        (EXPAND, json!({"prompt":"landscape","left":1})),
        (EXPAND, json!({"prompt":"landscape","left":512,"right":512})),
    ] {
        assert!(s.execute(id, p).is_err());
    }
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    assert_eq!(s.active().unwrap().history.past_len(), past);
    s.execute("select.rect", json!({"x":8,"y":8,"width":8,"height":8})).unwrap();
    assert!(s.execute(FILL, json!({"prompt":"blue","document":u64::MAX})).is_err());
    s.edit("responsive fill", |doc, _| {
        doc.layers.push(Layer::new("Fill", LayerContent::Fill(photocraft_doc::Fill::Solid(photocraft_color::Color::rgb(0.2, 0.3, 0.4)))));
        Ok(())
    })
    .unwrap();
    assert!(expandable(&s).is_err());
}

#[test]
fn feather_weights_are_kept_in_the_editable_mask() {
    let mut s = photo(16, "rgb", 64, 64);
    s.edit("selection", |doc, _| {
        let mut mask = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F32, false));
        mask.fill_rect(Rect::new(8, 8, 24, 24), &[0.12345]);
        doc.selection = Some(mask);
        Ok(())
    })
    .unwrap();
    let edit = selection_edit(&s).unwrap();
    assert!((edit.mask.sample_channel(10, 10, 0) - 0.12345).abs() < 1e-6);
}

#[test]
fn dry_run_validates_operations_without_inference_or_history_changes() {
    let mut s = photo(8, "rgb", 64, 64);
    s.execute("select.rect", json!({"x":8,"y":8,"width":16,"height":16})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    for (id, params) in
        [(FILL, json!({"prompt":"blue sky","dryRun":true})), (REMOVE, json!({"dryRun":true})), (EXPAND, json!({"prompt":"sky","left":8,"dryRun":true}))]
    {
        assert_eq!(s.execute(id, params).unwrap()["dryRun"], true);
    }
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    assert_eq!(s.active().unwrap().doc.size, before.size);
    assert_eq!(s.active().unwrap().history.past_len(), past);
}

#[test]
fn transparent_model_input_uses_a_neutral_matte_without_changing_document_alpha() {
    let mut s = photo(8, "rgb", 64, 64);
    s.edit("transparent source", |doc, _| {
        doc.layers.clear();
        Ok(())
    })
    .unwrap();
    let before = s.active().unwrap().doc.clone();
    let edit = expansion(&s, &json!({"left":8})).unwrap();
    let bytes = photocraft_paint::tile::b64_decode(&source(&before, &edit, &crate::jobs::JobCtx::new()).unwrap()).unwrap();
    let imported = crate::file_cmds::import("source.png", &bytes).unwrap();
    let image = photocraft_compose::render(&imported, imported.bounds());
    assert!((image.px[0][0] - 0.5).abs() < 0.01);
    assert_eq!(image.px[0][3], 1.0);
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
}

#[test]
fn invalid_params_identify_the_edit_command() {
    let mut s = photo(8, "rgb", 64, 64);
    s.execute("select.rect", json!({"x":8,"y":8,"width":16,"height":16})).unwrap();
    for (id, p) in [(FILL, json!({"prompt":""})), (REMOVE, json!({"prompt":""})), (EXPAND, json!({"left":-1}))] {
        let error = s.execute(id, p).unwrap_err();
        assert!(matches!(error,EngineError::BadParams {ref cmd,..} if cmd==id));
    }
}

#[test]
fn background_replacement_protects_subject_and_keeps_its_soft_weights() {
    let mut s = photo(16, "rgb", 64, 64);
    s.edit("subject", |doc, _| {
        let mut m = Surface::new(PixelFormat::new(photocraft_color::ColorMode::Grayscale, SampleType::F32, false));
        m.fill_rect(Rect::from_xywh(16, 16, 24, 24), &[1.0]);
        m.fill_rect(Rect::from_xywh(15, 16, 1, 24), &[0.12345]);
        doc.selection = Some(m);
        Ok(())
    })
    .unwrap();
    let plan = s.execute(BACKGROUND, json!({"prompt":"new sky","dryRun":true})).unwrap();
    assert_eq!(plan["harmonize"], false);
    let before = s.active().unwrap().doc.clone();
    let edit = background_edit(&s).unwrap();
    assert_eq!(edit.mask.sample_channel(20, 20, 0), 0.0);
    assert!((edit.mask.sample_channel(15, 20, 0) - 0.87655).abs() < 0.00001);
    let original = photocraft_compose::render(&before, before.bounds());
    let pixels = result_surface(&s, 64, 64);
    apply_edit(&mut s, pixels, edit, "Replace Background", 1, "mock").unwrap();
    let st = s.active().unwrap();
    let after = photocraft_compose::render(&st.doc, st.doc.bounds());
    for y in 16..40 {
        for x in 16..40 {
            assert_eq!(after.px[y * 64 + x], original.px[y * 64 + x]);
        }
    }
    assert_eq!(st.doc.selection, before.selection);
    assert_ne!(after.px[0], original.px[0]);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    assert!(s.execute(EXPAND, json!({"left":8,"prompt":"sea","harmonize":12,"dryRun":true})).is_err());
}
