use super::*;
use crate::xcf::tests::{TestImage, TestLayer, W};
use photocraft_doc::LayerContent;
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};
use photocraft_raster::Interrupt;

struct Limits(Option<usize>, Option<usize>);
impl Drop for Limits {
    fn drop(&mut self) {
        MARKUP_FORMATTING_LIMIT.with(|slot| slot.set(self.0));
        FILE_STYLE_COPY_BYTE_LIMIT.with(|slot| slot.set(self.1));
    }
}
fn limits(local: Option<usize>, file: Option<usize>) -> Limits {
    MARKUP_STYLE_COPY_BYTES_DONE.with(|slot| slot.set(0));
    Limits(MARKUP_FORMATTING_LIMIT.with(|slot| slot.replace(local)), FILE_STYLE_COPY_BYTE_LIMIT.with(|slot| slot.replace(file)))
}
fn copied() -> usize {
    MARKUP_STYLE_COPY_BYTES_DONE.with(std::cell::Cell::get)
}
fn parse(markup: &str, base: &CharStyle, fonts: &HashMap<String, CharStyle>) -> Result<(String, Vec<TextRun>)> {
    markup_runs(markup, base, fonts, 72., &mut Vec::new())
}

#[test]
fn base_style_copy_is_charged_before_an_overriding_leaf_at_the_exact_boundary() {
    let base = CharStyle { font_family: "A".repeat(1024), postscript_name: Some("ExactBase".into()), ..Default::default() };
    let markup = "<markup><span face='Tiny'>é</span></markup>";
    let mut expected = base.clone();
    expected.font_family = "Tiny".into();
    expected.postscript_name = None;
    let base_cost = style_allocation(&base);
    let total = base_cost + "Tiny".len() + std::mem::size_of::<TextRun>() + "é".len() + style_allocation(&expected);
    {
        let _limits = limits(Some(total), None);
        let (text, runs) = parse(markup, &base, &HashMap::new()).unwrap();
        assert_eq!(text, "é");
        assert_eq!(runs[0].style, expected);
        assert_eq!(copied(), base_cost, "the discarded inherited family still costs a copy");
    }
    {
        let _limits = limits(Some(base_cost - 1), None);
        let error = parse(markup, &base, &HashMap::new()).unwrap_err();
        assert!(matches!(error, crate::IoError::Unsupported(message) if message.contains("style copies")));
        assert_eq!(copied(), 0, "refusal happens before the base clone");
    }
    let _limits = limits(Some(total - 1), None);
    assert!(parse(markup, &base, &HashMap::new()).is_err(), "one byte below total formatting work cannot admit the run");
}

#[test]
fn font_resource_family_style_and_postscript_copies_are_each_charged_before_override() {
    let base = CharStyle::default();
    for field in 0..3 {
        let mut font =
            CharStyle { font_family: "Requested".into(), font_style: "Book".into(), postscript_name: Some("ExactResource".into()), ..Default::default() };
        match field {
            0 => font.font_family = "F".repeat(1024),
            1 => font.font_style = "S".repeat(1024),
            _ => font.postscript_name = Some("P".repeat(1024)),
        }
        let resource_cost = font.font_family.capacity() + font.font_style.capacity() + font.postscript_name.as_ref().unwrap().capacity();
        assert_eq!(font_resource_copy_bytes(&font), resource_cost);
        let mut expected = base.clone();
        expected.font_family = "Tiny".into();
        expected.font_style = font.font_style.clone();
        expected.postscript_name = None;
        let markup = "<markup><span font='F'><span font_family='Tiny'>x</span></span></markup>";
        let total = style_allocation(&base) + resource_cost + 4 + std::mem::size_of::<TextRun>() + 1 + style_allocation(&expected);
        let fonts = HashMap::from([("F".into(), font)]);
        {
            let _limits = limits(Some(total), None);
            let (text, runs) = parse(markup, &base, &fonts).unwrap();
            assert_eq!(text, "x");
            assert_eq!(runs[0].style, expected);
            assert_eq!(copied(), style_allocation(&base) + resource_cost);
        }
        let _limits = limits(Some(style_allocation(&base) + resource_cost - 1), None);
        let error = parse(markup, &base, &fonts).unwrap_err();
        assert!(matches!(error, crate::IoError::Unsupported(message) if message.contains("style copies")));
        assert_eq!(copied(), style_allocation(&base), "no resource strings are copied after refusal");
    }
}

#[test]
fn resource_construction_cannot_discard_an_uncharged_base_style() {
    let config = format!(
        "(font \"{}\") (font-size 12) (box-mode fixed) (box-width 16) (box-height 16) (markup \"<markup/>\" (markupfont \"F\" (font (family \"Tiny\"))))",
        "B".repeat(1024)
    );
    let _limits = limits(None, Some(1));
    let mut work = DecodeWork::default();
    let mut layout = POINT_LAYOUT_WORK_UNITS;
    let error = decode_with_work(config.as_bytes(), 72., (0, 0), &Interrupt::NONE, &mut layout, &mut work).err().unwrap();
    assert!(matches!(error, crate::IoError::Unsupported(message) if message.contains("document text style-copy")));
    assert_eq!(copied(), 0);
    assert_eq!(work.attributes.copies_left, 0, "a refused shared charge saturates and is never refunded");
}

fn config(resource: bool) -> Vec<u8> {
    let family = "A".repeat(16_384);
    if resource {
        format!("(font \"Sans\") (font-size 12) (box-mode fixed) (box-width 16) (box-height 16) (markup \"<markup><span font='F'><span face='Tiny'>x</span></span></markup>\" (markupfont \"F\" (font (family \"{family}\") (style \"Book\") (psname \"ExactResource\"))))").into_bytes()
    } else {
        format!("(font \"{family}\") (font-size 12) (box-mode fixed) (box-width 16) (box-height 16) (markup \"<markup><span face='Tiny'>x</span></markup>\")")
            .into_bytes()
    }
}
fn source(config: &[u8], owners: usize, aliases: bool) -> Vec<u8> {
    let mut parasite = W { out: Vec::new(), wide: false };
    parasite.string("gimp-text-layer");
    parasite.u32(0);
    parasite.u32(config.len() as u32);
    parasite.out.extend_from_slice(config);
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    for _ in 0..owners {
        let mut layer = TestLayer::rgba("Style copy work", 1, 1, vec![17, 31, 57, 255]);
        layer.props.push((26, 0u32.to_be_bytes().to_vec()));
        layer.props.push((21, parasite.out.clone()));
        image.layers.push(layer);
    }
    let mut bytes = image.write();
    if aliases {
        let mut cursor = crate::xcf::wire::Cursor { b: &bytes, pos: 30, wide: true };
        crate::xcf::wire::props_version(&mut cursor, Some(26)).unwrap();
        let start = cursor.pos;
        let pointer = cursor.ptr().unwrap();
        for owner in 1..owners {
            let slot = start + owner * 8;
            bytes[slot..slot + 8].copy_from_slice(&(pointer as u64).to_be_bytes());
        }
    }
    bytes
}
fn assert_persisted(imported: crate::ImportResult, config: &[u8], expected_fallbacks: usize) {
    let doc = imported.document;
    let raster: Vec<_> = doc.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Raster(_))).collect();
    assert_eq!(raster.len(), expected_fallbacks);
    for layer in raster {
        assert_eq!(layer.surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    }
    assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
    let ImportConversion::Report(report) = doc.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
    assert_eq!(
        report.entries.iter().filter(|entry| entry.feature_code == "xcf.text" && entry.reason == ConversionReason::MissingMapping).count(),
        expected_fallbacks
    );
    assert!(report.attachments.iter().any(|attachment| attachment.bytes.windows(config.len()).any(|bytes| bytes == config)));
    let native = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
    let restored = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(restored.layers, doc.layers);
    assert_eq!(restored.metadata.import_conversion, doc.metadata.import_conversion);
}

#[test]
fn discarded_base_and_resource_copy_limits_preserve_saved_pixels_source_and_native_report() {
    for resource in [false, true] {
        let config = config(resource);
        let _limits = limits(Some(4096), None);
        let imported = crate::import("style-copy.xcf", &source(&config, 1, false)).unwrap();
        assert_persisted(imported, &config, 1);
    }
}

#[test]
fn shared_copy_work_is_nonrefundable_across_alias_and_distinct_layer_owners() {
    let config = config(false);
    let plain = format!("(font \"{}\") (font-size 12) (box-mode fixed) (box-width 16) (box-height 16) (text \"x\")", "A".repeat(16_384));
    let base = decode(plain.as_bytes(), 72., (0, 0)).unwrap().layer.runs.remove(0).style;
    let cost = style_allocation(&base);
    for aliases in [false, true] {
        let _limits = limits(None, Some(cost * 2));
        let imported = crate::import("owners.xcf", &source(&config, 4, aliases)).unwrap();
        assert_eq!(copied(), cost * 2);
        assert_eq!(imported.document.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Text(_))).count(), 2);
        assert_persisted(imported, &config, 2);
    }
}

#[test]
fn cancellation_precedes_style_copy_work_with_exhausted_allowances() {
    let _limits = limits(Some(0), Some(0));
    let mut work = DecodeWork::default();
    let mut layout = POINT_LAYOUT_WORK_UNITS;
    let cancel = || true;
    let result = decode_with_work(&config(true), 72., (0, 0), &Interrupt::cancel_only(&cancel), &mut layout, &mut work);
    assert!(matches!(result, Err(crate::IoError::Cancelled)));
    assert_eq!(copied(), 0);
    assert_eq!(work.attributes.copies_left, 0);
}
