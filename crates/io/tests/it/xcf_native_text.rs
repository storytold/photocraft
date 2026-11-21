//! Public corpus typography counterpart and independent synthetic source schemas.
//! The synthetic cases never depend on unpublished research fixtures.

/// The source offset names the logical top-left, while native point text is baseline-anchored.
/// Check the original affine coefficients and its origin independently of any imported translation.
fn assert_source_origin(layer: &photocraft_doc::TextLayer, dpi: f32, source: photocraft_geom::Affine) {
    assert_eq!(&layer.transform.m[..4], &source.m[..4]);
    let mut engine = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let [left, top, _, _] = engine.layout(layer, dpi).bounds().expect("native logical text bounds");
    let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(left), f64::from(top)));
    assert!((origin.x - source.m[4]).abs() < 0.0001 && (origin.y - source.m[5]).abs() < 0.0001, "{origin:?} versus {source:?}");
}

#[cfg(feature = "corpus")]
mod published {
    use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};

    #[test]
    fn published_text_with_ambiguous_float_paint_keeps_pixels_and_report_after_reopen() {
        for name in ["text.xcf", "text-perceptual.xcf"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp").join(name);
            let bytes = std::fs::read(path).expect("run cargo xtask corpus --gimp");
            let imported = photocraft_io::import(name, &bytes).expect("import");
            let document = imported.document;
            let layer = &document.layers[1];
            assert_eq!(layer.name, "XCF");
            assert!(!layer.surface().expect("saved text pixels").content_bounds().is_empty());
            assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
            let Some(ImportConversion::Report(report)) = document.metadata.import_conversion.as_deref() else { panic!("conversion report") };
            let text = report.entries.iter().find(|entry| entry.feature_code == "xcf.text").expect("text fallback");
            assert_eq!(text.reason, ConversionReason::MissingMapping);
            assert!(text.source_description.contains("source byte-order evidence"));
            let native = photocraft_format::save_to_bytes(&document, &Default::default()).expect("save");
            let reopened = photocraft_format::load_from_bytes(&native).expect("reopen");
            assert_eq!(reopened.layers[1].surface(), layer.surface());
            assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
        }
    }
}

mod synthetic {
    use photocraft_doc::LayerContent;
    struct Wire(Vec<u8>);
    impl Wire {
        /// Writes a big-endian framing word for independent historical text fixtures.
        fn word(&mut self, n: u32) {
            self.0.extend(n.to_be_bytes());
        }
        /// Writes a fixture pointer using the selected text fixture's 64-bit pointer representation.
        fn pointer(&mut self, n: usize) {
            self.0.extend((n as u64).to_be_bytes());
        }
        /// Reserves a pointer for a layer or hierarchy record written later in the text fixture.
        fn slot(&mut self) -> usize {
            let at = self.0.len();
            self.pointer(0);
            at
        }
        /// Patches a reserved pointer to the current fixture record without using importer internals.
        fn patch(&mut self, at: usize) {
            let offset = self.0.len() as u64;
            self.0[at..at + 8].copy_from_slice(&offset.to_be_bytes());
        }
        /// Writes a length-prefixed source string and its required terminating NUL.
        fn text(&mut self, text: &str) {
            self.word(text.len() as u32 + 1);
            self.0.extend(text.as_bytes());
            self.0.push(0);
        }
        /// Frames a controlled source property or parasite payload for text precedence tests.
        fn property(&mut self, kind: u32, data: &[u8]) {
            self.word(kind);
            self.word(data.len() as u32);
            self.0.extend(data);
        }
        /// Terminates the fixture property list before layer or hierarchy pointers.
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
    }
    /// Builds modern and legacy text records with controlled modification flags and parasite order.
    fn source(modified: bool, modern: bool, reverse: bool) -> Vec<u8> {
        source_config(modified, modern, reverse, br#"(text "Modern wins") (font "Native requested family") (font-size 12)"#)
    }
    /// Builds the text fixture using an explicitly supplied modern configuration alongside legacy data.
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
    /// Checks a full historical GDynText record imports as editable native type and survives native reopen.
    fn historical_gdyntext_full_import_is_native_editable_and_persists() {
        let source = source(false, false, false);
        let imported = photocraft_io::import("legacy.xcf", &source).unwrap();
        let doc = imported.document;
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("native legacy text") };
        assert_eq!(text.text, "héλ");
        assert_eq!(text.font_family, "fixed");
        assert_eq!(text.size_pt, 20.);
        assert_eq!(text.cache.as_ref().unwrap().pixel(0, 0), [7. / 255., 19. / 255., 31. / 255., 1.]);
        let png = photocraft_io::export(&doc, "editable-text.png", &Default::default()).unwrap();
        let pixels = photocraft_codecs::decode(&png.bytes).unwrap();
        assert_eq!(&pixels.to_rgba8()[..4], &[7, 19, 31, 255]);
        assert!(imported.source_read_only && !imported.preview_only);
        assert!(doc.metadata.import_conversion.is_some());
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(reopened.layers, doc.layers);
        assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
    }
    #[test]
    /// Verifies modern text precedence and the raster fallback for destructively modified legacy text.
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
    /// Checks observed inactive legacy rotation does not invent a native text transform during import or
    /// persistence.
    fn observed_inactive_legacy_rotation_imports_native_text_and_persists() {
        let mut bytes = source(false, false, false);
        let marker = b"}{0}{2}{c04020}";
        let at = bytes.windows(marker.len()).position(|value| value == marker).unwrap();
        bytes[at + 2] = b'1';
        let doc = photocraft_io::import("legacy-rotation.xcf", &bytes).unwrap().document;
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("editable legacy text") };
        assert_eq!(text.text, "héλ");
        super::assert_source_origin(text, doc.resolution_dpi, photocraft_geom::Affine::IDENTITY);
        let photocraft_doc::import_conversion::ImportConversion::Report(report) = doc.metadata.import_conversion.as_deref().unwrap() else {
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
    /// Verifies modern affine text placement excludes source offsets independently observed to be inactive.
    fn modern_affine_placement_does_not_add_observed_inactive_source_offsets() {
        let bytes =
            source_config(false, true, false, br#"(text "Position") (font-size 20) (transformation (matrix 1.2 0.3 -0.2 0.9)) (offset-x 20) (offset-y 10)"#);
        let doc = photocraft_io::import("text-position.xcf", &bytes).unwrap().document;
        let LayerContent::Text(text) = &doc.layers[0].content else { panic!("native affine text") };
        super::assert_source_origin(text, doc.resolution_dpi, photocraft_geom::Affine { m: [1.2, -0.2, 0.3, 0.9, 0., 0.] });
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().layers, doc.layers);
    }
    #[test]
    /// Import named Pango paints through full XCF framing, keeping X11 numbered colors and CSS gray precedence as editable native text after reopen.
    fn named_markup_colors_import_as_editable_native_runs_and_persist() {
        use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};
        let config = br#"(markup "<markup><span foreground=\"steelblue4\">S</span><span fgcolor=\"gray42\">N</span><span color=\"gray\">C</span></markup>") (font "Sans") (font-size 12)"#;
        let bytes = source_config(false, true, false, config);
        let imported = photocraft_io::import("named-markup.xcf", &bytes).unwrap();
        let doc = imported.document;
        let LayerContent::Text(text) = &doc.layers[0].content else {
            panic!("named paints must preserve editable text instead of falling back to saved raster pixels")
        };
        assert_eq!(text.text, "SNC");
        assert_eq!(text.runs.len(), 3);
        assert!(text.cache.is_some(), "editable type retains its saved appearance without installed fonts");
        // Independent X11 rgb.txt facts for the numbered names; unnumbered
        // Pango gray uses CSS128, not the X11 table's190 value.
        let expected = [[54., 100., 139.], [107., 107., 107.], [128., 128., 128.]];
        for (run, expected) in text.runs.iter().zip(expected) {
            assert_eq!(run.len, 1);
            assert_eq!(run.style.color.mode, photocraft_color::ColorMode::Rgb);
            assert_eq!(run.style.color.alpha, 1.);
            for (actual, byte) in run.style.color.c.iter().take(3).zip(expected) {
                assert!((*actual - byte / 255.).abs() < 1e-5, "named paint must match the independent RGB literal after native sRGB CMS");
            }
        }
        let Some(ImportConversion::Report(report)) = doc.metadata.import_conversion.as_deref() else { panic!("typed conversion report") };
        let converted = report.entries.iter().find(|entry| entry.feature_code == "xcf.text").unwrap();
        assert_eq!(converted.disposition, ConversionDisposition::NativeApproximation);
        assert_eq!(converted.reason, ConversionReason::NativePolicy);
        assert!(converted.native_description.contains("Native editable text"));
        assert!(imported.source_read_only && !imported.preview_only);
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        let LayerContent::Text(reopened_text) = &reopened.layers[0].content else { panic!("native reopen must retain editable type") };
        assert_eq!(reopened_text.text, text.text);
        assert_eq!(reopened_text.runs, text.runs);
        assert_eq!(reopened.metadata.import_conversion, doc.metadata.import_conversion);
    }
}
