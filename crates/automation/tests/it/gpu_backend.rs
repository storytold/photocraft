//! Tests for the headless GPU backend, capability isolation, cache mutation, and CPU/GPU parity.

#[cfg(feature = "gpu")]
mod tests {
    use std::fs;

    use photocraft_codecs::decode;
    use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
    use photocraft_doc::{Document, Layer, LayerContent};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;

    use photocraft_automation::workspace::AuthorizedWorkspace;
    use photocraft_automation::{GpuRenderer, Headless};

    fn make_test_doc(w: u32, h: u32, depth: SampleType) -> Document {
        let mut doc = Document::with_background("test_doc", Size::new(w, h), ColorMode::Rgb, depth, Color::WHITE);
        let mut s = Surface::new(doc.pixel_format());
        s.fill_rect(Rect::new(10, 10, w as i32 - 10, h as i32 - 10), &[0.2, 0.5, 0.8, 0.8]);
        let mut top = Layer::new("top", LayerContent::Raster(s));
        top.opacity = 0.75;
        top.blend = BlendMode::Multiply;
        doc.layers.push(top);
        doc
    }

    #[test]
    fn headless_with_gpu_renders_png_full_and_scaled() {
        let mut h = Headless::trusted_local().with_gpu();
        let doc = make_test_doc(120, 80, SampleType::U8);
        h.session.open_document(doc);

        // Full size render (max_side = 0)
        let full_png = h.render_png(None, 0).expect("full render_png");
        let full_img = decode(&full_png).expect("decode full");
        assert_eq!((full_img.width(), full_img.height()), (120, 80));

        // Scaled preview (max_side = 40)
        let thumb_png = h.render_png(None, 40).expect("thumbnail render_png");
        let thumb_img = decode(&thumb_png).expect("decode thumbnail");
        assert_eq!((thumb_img.width(), thumb_img.height()), (40, 27));
    }

    #[test]
    fn capability_scoped_writes_preserved_with_gpu() {
        let dir = std::env::temp_dir().join(format!("photocraft-cap-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let workspace = AuthorizedWorkspace::new(dir.clone(), vec![dir.clone()]).expect("authorized workspace");

        let mut h = Headless::with_workspace(workspace).with_gpu();
        let doc = make_test_doc(64, 64, SampleType::U8);
        h.session.open_document(doc);

        let valid_target = dir.join("out.png");
        let res = h.save(None, Some(&valid_target), Some("png"), &Default::default());
        assert!(res.is_ok(), "authorized write must succeed: {:?}", res);
        assert!(valid_target.exists());

        // Attempt write outside authorized workspace
        let invalid_target = std::env::temp_dir().join("outside_unauthorized.png");
        let denied = h.save(None, Some(&invalid_target), Some("png"), &Default::default());
        assert!(denied.is_err(), "write outside authorized workspace must be denied");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_invalidation_across_mutations_and_distinct_docs() {
        let mut h = Headless::trusted_local().with_gpu();

        // 1. Render Doc 1
        let doc1 = make_test_doc(64, 64, SampleType::U8);
        h.session.open_document(doc1);
        let png1_initial = h.render_png(Some(0), 0).expect("initial render");
        let img1_initial = decode(&png1_initial).expect("decode initial");

        // 2. Mutate Doc 1 in place (change top layer opacity to 0.1)
        h.session.documents_mut()[0].doc.layers[1].opacity = 0.1;
        let png1_mutated = h.render_png(Some(0), 0).expect("mutated render");
        let img1_mutated = decode(&png1_mutated).expect("decode mutated");

        assert_ne!(img1_initial.data(), img1_mutated.data(), "mutating document must invalidate cache and produce fresh pixels");

        // 3. Open Doc 2 with distinct contents
        let mut doc2 = Document::with_background("doc2", Size::new(64, 64), ColorMode::Rgb, SampleType::U8, Color::BLACK);
        let mut s2 = Surface::new(doc2.pixel_format());
        s2.fill_rect(Rect::new(0, 0, 64, 64), &[1.0, 0.0, 0.0, 1.0]);
        doc2.layers.push(Layer::new("red", LayerContent::Raster(s2)));
        h.session.open_document(doc2);

        let png2 = h.render_png(Some(1), 0).expect("doc2 render");
        let img2 = decode(&png2).expect("decode doc2");

        assert_ne!(img2.data(), img1_mutated.data(), "distinct document must not return stale pixels from previous doc");
    }

    #[test]
    fn cpu_gpu_parity_over_synthetic_documents() {
        if GpuRenderer::new().is_err() {
            eprintln!("skipping cpu_gpu_parity test: no GPU adapter available");
            return;
        }

        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let doc = make_test_doc(96, 96, depth);

            // Render via CPU
            let mut cpu_h = Headless::trusted_local();
            cpu_h.session.open_document(doc.clone());
            let cpu_png = cpu_h.render_png(None, 0).expect("cpu render");
            let cpu_img = decode(&cpu_png).expect("decode cpu");

            // Render via GPU
            let mut gpu_h = Headless::trusted_local().with_gpu();
            gpu_h.session.open_document(doc);
            let gpu_png = gpu_h.render_png(None, 0).expect("gpu render");
            let gpu_img = decode(&gpu_png).expect("decode gpu");

            assert_eq!(cpu_img.width(), gpu_img.width());
            assert_eq!(cpu_img.height(), gpu_img.height());
            assert_eq!(cpu_img.layout(), gpu_img.layout());
            assert_eq!(cpu_img.sample_type(), gpu_img.sample_type());

            let cpu_data = cpu_img.data();
            let gpu_data = gpu_img.data();
            assert_eq!(cpu_data.len(), gpu_data.len());

            let tol = 2u8; // Within 2/255 tolerance
            for (i, (&c, &g)) in cpu_data.iter().zip(gpu_data).enumerate() {
                let diff = (c as i16 - g as i16).abs();
                assert!(diff <= tol as i16, "depth {:?} pixel byte {} differs: cpu={}, gpu={}, diff={}", depth, i, c, g, diff);
            }
        }
    }
}
