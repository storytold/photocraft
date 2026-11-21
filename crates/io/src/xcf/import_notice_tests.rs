//! Native-history notices do not repeat successful first-import policy disclosures.
use super::tests::{TestImage, TestLayer, W};
use super::*;
use photocraft_doc::import_conversion::{ConversionReason as R, ImportConversion};

fn parasite(name: &str, data: &[u8]) -> Vec<u8> {
    let mut wire = W { out: vec![], wide: false };
    wire.string(name);
    wire.u32(0);
    wire.u32(data.len() as u32);
    wire.out.extend(data);
    wire.out
}

fn text_source(config: &[u8]) -> Vec<u8> {
    let mut source = TestImage::rgb8(1, 1);
    source.version = 26;
    let mut layer = TestLayer::rgba("Editable text", 1, 1, vec![17, 31, 57, 255]);
    layer.props.push((26, 0u32.to_be_bytes().to_vec()));
    layer.props.push((21, parasite("gimp-text-layer", config)));
    source.layers.push(layer);
    source.write()
}

fn vector_source() -> Vec<u8> {
    let mut source = TestImage::rgb8(1, 1);
    source.version = 26;
    let mut path = W { out: vec![], wide: false };
    for value in [1, 0, 1] {
        path.u32(value);
    }
    path.string("Native path");
    for value in [7, 0, 0, 0, 1, 1, 1, 2, 3] {
        path.u32(value);
    }
    for (tag, x, y) in [(1, 0f32, 0f32), (0, 1., 1.), (1, 2., 2.)] {
        path.u32(tag);
        path.u32(x.to_bits());
        path.u32(y.to_bits());
    }
    source.image_props.push((25, path.out));
    let mut style = W { out: vec![], wide: false };
    for value in [0, 7, 1, 0] {
        style.u32(value);
    }
    for _ in 0..2 {
        style.u32(0);
        style.u32(1);
        style.string("R'G'B'A u8");
        style.u32(4);
        style.out.extend([17, 31, 57, 255]);
        style.u32(0);
        style.string("");
    }
    for value in [1f32.to_bits(), 0, 0, 4f32.to_bits(), 0] {
        style.u32(value);
    }
    let mut layer = TestLayer::rgba("Editable vector", 1, 1, vec![17, 31, 57, 255]);
    layer.props.push((47, style.out));
    source.layers.push(layer);
    source.write()
}

fn float_source(precision: u32, red: f64, alpha: f64) -> Vec<u8> {
    let mut source = TestImage::rgb8(1, 1);
    source.version = 26;
    source.precision = precision;
    let data = [red, 0.5, 0.75, alpha]
        .into_iter()
        .flat_map(|value| if matches!(precision, 700 | 750) { value.to_be_bytes().to_vec() } else { (value as f32).to_be_bytes().to_vec() })
        .collect();
    source.layers.push(TestLayer::rgba("Native float pixels", 1, 1, data));
    source.write()
}

fn assert_persisted_notices(imported: &crate::ImportResult, warn: bool) {
    let report = imported.document.metadata.import_conversion.clone();
    assert_eq!(crate::open_warnings(&imported.document).len(), usize::from(warn));
    let saved = crate::export(&imported.document, "notice.pcraft", &Default::default()).unwrap();
    assert!(!saved.warnings.iter().any(|warning| warning.contains("import conversion report") && warning.contains("not exported")));
    let reopened = crate::import("notice.pcraft", &saved.bytes).unwrap();
    assert_eq!(reopened.document.metadata.import_conversion, report, "history remains inspectable without recurring informational notices");
    assert_eq!(reopened.warnings.iter().filter(|warning| warning.contains("historical import conversion report")).count(), usize::from(warn));
    let exported = crate::export(&reopened.document, "png", &Default::default()).unwrap();
    assert_eq!(
        exported.warnings.iter().filter(|warning| warning.contains("import conversion report") && warning.contains("not exported")).count(),
        usize::from(warn)
    );
}

#[test]
fn successful_text_and_vector_keep_first_import_disclosures_but_native_history_is_quiet() {
    let text = crate::import("notice.xcf", &text_source(br#"(text "Ag") (font "Sans") (font-size 12)"#)).unwrap();
    assert!(matches!(text.document.layers[0].content, LayerContent::Text(_)));
    assert!(text.warnings.iter().any(|warning| warning.contains("editable native text")));
    assert_persisted_notices(&text, false);
    let vector = crate::import("notice.xcf", &vector_source()).unwrap();
    assert!(matches!(vector.document.layers[0].content, LayerContent::Shape(_)));
    assert!(vector.warnings.iter().any(|warning| warning.contains("editable native shape")));
    assert_persisted_notices(&vector, false);
}

#[test]
fn ordinary_f32_f64_storage_conversion_is_quiet_but_underflow_and_clamping_still_warn() {
    for precision in [650, 700, 750] {
        let imported = crate::import("notice.xcf", &float_source(precision, 0.25, 1.)).unwrap();
        assert_eq!(imported.document.depth, SampleType::F32);
        let pixels = imported.document.layers[0].surface().unwrap().read_region(Rect::new(0, 0, 1, 1));
        assert_eq!(pixels, [0.25, 0.5, 0.75, 1.]);
        let Some(ImportConversion::Report(report)) = imported.document.metadata.import_conversion.as_deref() else { panic!("typed report") };
        assert_eq!(
            report.entries.iter().any(|entry| entry.feature_code == "xcf.precision" && entry.reason == R::NumericalConversion),
            matches!(precision, 700 | 750)
        );
        let encoding = report.entries.iter().find(|entry| entry.feature_code == "xcf.color_encoding").unwrap();
        if precision == 700 {
            assert!(encoding.native_description.contains("transfer curve"));
        }
        assert!(!encoding.native_description.contains("precision"));
        assert_persisted_notices(&imported, false);
        let clamped = crate::import("clamped.xcf", &float_source(precision, 0.25, 1.5)).unwrap();
        assert!(clamped.warnings.iter().any(|warning| warning.contains("coverage") && warning.contains("clamped")));
        assert_persisted_notices(&clamped, true);
    }
    let underflow = crate::import("tiny.xcf", &float_source(750, f64::from_bits(1), 1.)).unwrap();
    assert!(underflow.warnings.iter().any(|warning| warning.contains("rounded to zero")));
    assert_persisted_notices(&underflow, true);
}

#[test]
fn unsupported_text_keeps_initial_and_native_history_notices() {
    let imported = crate::import("unmapped.xcf", &text_source(br#"(text "Ag") (font-size 12) (base-direction ttb-ltr)"#)).unwrap();
    assert!(matches!(imported.document.layers[0].content, LayerContent::Raster(_)));
    assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
    assert_persisted_notices(&imported, true);
}
