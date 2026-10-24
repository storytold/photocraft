//! Public corpus typography counterpart and independent synthetic source schemas.
//! The synthetic cases never depend on unpublished research fixtures.

#[cfg(feature = "corpus")]
mod published {
    use photocraft_color::Color;
    use photocraft_doc::text::{CharStyle, Kerning, ParagraphRun, ParagraphStyle, TextDirection, TextRun};
    use photocraft_doc::{LayerContent, TextLayer};
    use photocraft_geom::Affine;
    #[test]
    fn published_text_matches_independently_constructed_native_typography_and_reopens() {
        for name in ["text.xcf", "text-perceptual.xcf"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp").join(name);
            let bytes = std::fs::read(path).expect("run cargo xtask corpus --gimp");
            let doc = photocraft_io::import(name, &bytes).expect("import").document;
            let LayerContent::Text(actual) = &doc.layers[1].content else { panic!("native text") };
            // Public generator:24pixel DejaVu Sans at(10,10),300DPI.
            let style = CharStyle {
                font_family: "DejaVu Sans".into(),
                font_style: "Book".into(),
                postscript_name: Some("DejaVuSans".into()),
                size_pt: 24. * 72. / 300.,
                kerning: Kerning::Off,
                language: Some("en-US".into()),
                ..Default::default()
            };
            let mut expected = TextLayer {
                text: "XCF".into(),
                transform: Affine::translate(10., 10.),
                runs: vec![TextRun { len: 3, style }],
                paragraphs: vec![ParagraphRun { len: 3, style: ParagraphStyle { direction: TextDirection::Ltr, ..Default::default() } }],
                ..Default::default()
            };
            expected.sync_summary();
            assert_eq!(actual, &expected, "native source mapping for {name}");
            assert_eq!(actual.color, Color::BLACK);
            let mut engine = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
            let actual_layout = engine.layout(actual, doc.resolution_dpi);
            let expected_layout = engine.layout(&expected, doc.resolution_dpi);
            let actual_pixels = photocraft_text::render::rasterize_warped(&actual_layout, &actual.transform, doc.pixel_format(), actual.antialias, None);
            let expected_pixels =
                photocraft_text::render::rasterize_warped(&expected_layout, &expected.transform, doc.pixel_format(), expected.antialias, None);
            assert_eq!(actual_pixels.surface, expected_pixels.surface, "normal native text renderer");
            drop(engine);
            let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).expect("save");
            let reopened = photocraft_format::load_from_bytes(&saved).expect("reopen");
            let LayerContent::Text(text) = &reopened.layers[1].content else { panic!("native text on reopen") };
            assert_eq!(text, &expected);
        }
    }
}

mod synthetic {
    use photocraft_doc::LayerContent;
    struct Wire(Vec<u8>);
    impl Wire {
        fn word(&mut self, n: u32) {
            self.0.extend(n.to_be_bytes());
        }
        fn pointer(&mut self, n: usize) {
            self.0.extend((n as u64).to_be_bytes());
        }
        fn slot(&mut self) -> usize {
            let at = self.0.len();
            self.pointer(0);
            at
        }
        fn patch(&mut self, at: usize) {
            let offset = self.0.len() as u64;
            self.0[at..at + 8].copy_from_slice(&offset.to_be_bytes());
        }
        fn text(&mut self, text: &str) {
            self.word(text.len() as u32 + 1);
            self.0.extend(text.as_bytes());
            self.0.push(0);
        }
        fn property(&mut self, kind: u32, data: &[u8]) {
            self.word(kind);
            self.word(data.len() as u32);
            self.0.extend(data);
        }
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
    }
    fn source(modified: bool, modern: bool, reverse: bool) -> Vec<u8> {
        source_config(modified, modern, reverse, br#"(text "Modern wins") (font "Native requested family") (font-size 12)"#)
    }
    fn source_config(modified: bool, modern: bool, reverse: bool, current: &[u8]) -> Vec<u8> {
        let mut w = Wire(b"gimp xcf v011\0".to_vec());
        for n in [1, 1, 0, 150] {
            w.word(n);
        }
        w.property(17, &[0]);
        w.end();
        let layer = w.slot();
        w.pointer(0);
        w.pointer(0);
        w.patch(layer);
        for n in [1, 1, 1] {
            w.word(n);
        }
        w.text("Historical text");
        w.property(26, &(if modified { 2u32 } else { 0 }).to_be_bytes());
        let legacy = br"GDT10{h\303\251\316\273}{1}{1}{0}{2}{c04020}{0}{-misc-fixed-bold-i-normal--20-120-75-75-p-100-iso8859-1}";
        let mut entries = vec![("plug_in_gdyntext/data", legacy.as_slice())];
        if modern {
            entries.push(("gimp-text-layer", current));
        }
        if reverse {
            entries.reverse();
        }
        let mut parasites = Wire(Vec::new());
        for (name, data) in entries {
            parasites.text(name);
            parasites.word(1);
            parasites.word(data.len() as u32);
            parasites.0.extend(data);
        }
        w.property(21, &parasites.0);
        w.end();
        let hierarchy = w.slot();
        w.pointer(0);
        w.patch(hierarchy);
        for n in [1, 1, 4] {
            w.word(n);
        }
        let level = w.slot();
        w.pointer(0);
        w.patch(level);
        w.word(1);
        w.word(1);
        let tile = w.slot();
        w.pointer(0);
        w.patch(tile);
        w.0.extend([7, 19, 31, 255]);
        w.0
    }
    #[test]
    fn historical_gdyntext_full_import_is_native_editable_and_persists() {
        let source = source(false, false, false);
        let imported = photocraft_io::import("legacy.xcf", &source).unwrap();
        let doc = imported.document;
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("native legacy text") };
        assert_eq!(text.text, "héλ");
        assert_eq!(text.font_family, "fixed");
        assert_eq!(text.size_pt, 20.);
        assert!(text.cache.is_none());
        assert!(imported.source_read_only && !imported.preview_only);
        assert!(doc.metadata.import_conversion.is_some());
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers, doc.layers);
        assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
    }
    #[test]
    fn modern_text_takes_priority_and_modified_legacy_keeps_its_own_pixels() {
        for reverse in [false, true] {
            let doc = photocraft_io::import("both.xcf", &source(false, true, reverse)).unwrap().document;
            let LayerContent::Text(text) = &doc.layers[0].content else { panic!("native modern text") };
            assert_eq!(text.text, "Modern wins");
        }
        let imported = photocraft_io::import("modified-legacy.xcf", &source(true, false, false)).unwrap();
        let LayerContent::Raster(pixels) = &imported.document.layers[0].content else { panic!("modified source raster") };
        assert_eq!(pixels.read_region(photocraft_geom::Rect::new(0, 0, 1, 1)), vec![7. / 255., 19. / 255., 31. / 255., 1.]);
        assert!(imported.warnings.iter().any(|warning| warning.contains("modified text pixels")));
    }
    #[test]
    fn observed_inactive_legacy_rotation_imports_native_text_and_persists() {
        let mut bytes = source(false, false, false);
        let marker = b"}{0}{2}{c04020}";
        let at = bytes.windows(marker.len()).position(|value| value == marker).unwrap();
        bytes[at + 2] = b'1';
        let doc = photocraft_io::import("legacy-rotation.xcf", &bytes).unwrap().document;
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("editable legacy text") };
        assert_eq!(text.text, "héλ");
        assert_eq!(text.transform, photocraft_geom::Affine::IDENTITY);
        let photocraft_doc::import_conversion::ImportConversion::Report(report) = doc.metadata.import_conversion.as_ref().unwrap() else {
            panic!("known report")
        };
        let entry = report.entries.iter().find(|entry| entry.feature_code == "xcf.text").unwrap();
        assert_ne!(entry.reason, photocraft_doc::import_conversion::ConversionReason::MissingMapping);
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.text.formatting"
            && entry.reason == photocraft_doc::import_conversion::ConversionReason::NativePolicy
            && entry.source_description.contains("rotation control")));
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers, doc.layers);
    }
    #[test]
    fn modern_affine_placement_does_not_add_observed_inactive_source_offsets() {
        let bytes =
            source_config(false, true, false, br#"(text "Position") (font-size 20) (transformation (matrix 1.2 0.3 -0.2 0.9)) (offset-x 20) (offset-y 10)"#);
        let doc = photocraft_io::import("text-position.xcf", &bytes).unwrap().document;
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("native affine text") };
        assert_eq!(text.transform.m, [1.2, -0.2, 0.3, 0.9, 0., 0.]);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().layers, doc.layers);
    }
}
