use super::*;
use crate::xcf::tests::{TestImage, TestLayer, W};
use photocraft_doc::import_conversion::{ConversionReason, ImportConversion};

fn visits() -> usize {
    MARKUP_ATTRIBUTE_VISITS_DONE.with(std::cell::Cell::get)
}

fn reset() {
    MARKUP_ATTRIBUTE_VISITS_DONE.with(|count| count.set(0));
}

fn adversarial_markup() -> String {
    let attributes = (0..128).map(|index| format!("unknown{index}='inert' ")).collect::<String>();
    format!("<markup><span {attributes}>{}</span></markup>", "<b>x</b>".repeat(64))
}

fn with_visit_limit<T>(limit: usize, call: impl FnOnce() -> T) -> T {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|limit| limit.set(self.0));
        }
    }
    let restore = Restore(MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|slot| slot.replace(Some(limit))));
    let result = call();
    drop(restore);
    result
}

#[test]
fn inherited_unknown_attributes_spend_visits_before_repeated_diagnostics() {
    let markup = adversarial_markup();
    let mut notes = Vec::new();
    let mut allowance = 256;
    reset();
    let result = markup_runs_with_attribute_allowance(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut notes, &mut allowance);
    assert!(matches!(result, Err(crate::IoError::Unsupported(message)) if message.contains("attribute-visit allowance")));
    assert_eq!(allowance, 0);
    assert_eq!(visits(), 256, "one shared ledger covers inspection and application across descendants");
    assert_eq!(notes.len(), 128, "the first leaf reaches diagnostic storage's cap; later work must still be stopped independently");
    assert!(notes.iter().all(|note| note.len() < 128));
    assert!(markup.len() < 4096, "the negative control never runs a huge quadratic fixture");
}

#[test]
fn exact_visit_boundary_preserves_inherited_known_styles_and_font_precedence() {
    let markup = "<markup><span weight='bold' foreground='#336699'><i>A</i><b>é</b></span></markup>";
    let mut allowance = 8; // Two attributes, two visits each, across two text descendants.
    reset();
    let (text, runs) = markup_runs_with_attribute_allowance(markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new(), &mut allowance).unwrap();
    assert_eq!((allowance, visits()), (0, 8));
    assert_eq!(text, "Aé");
    assert_eq!(runs.iter().map(|run| run.len).collect::<Vec<_>>(), [1, 2]);
    assert!(runs.iter().all(|run| run.style.weight == 700));
    assert!(runs[0].style.italic);
    assert!(!runs[1].style.italic);
    assert!(runs.iter().all(|run| run.style.color == Color::rgba(0.2, 0.4, 0.6, 1.)));
    let mut short = 7;
    assert!(markup_runs_with_attribute_allowance(markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new(), &mut short).is_err());

    let requested = CharStyle { font_family: "Chosen family".into(), postscript_name: Some("ChosenExactFace".into()), ..Default::default() };
    let fonts = HashMap::from([("Chosen".to_owned(), requested)]);
    let mut allowance = 6;
    let mut notes = Vec::new();
    let (_, runs) = markup_runs_with_attribute_allowance(
        "<markup><span font='Chosen' font_desc='Ignored fallback' weight='bold'>x</span></markup>",
        &CharStyle::default(),
        &fonts,
        72.,
        &mut notes,
        &mut allowance,
    )
    .unwrap();
    assert_eq!(allowance, 0);
    assert_eq!(runs[0].style.font_family, "Chosen family");
    assert_eq!(runs[0].style.weight, 700);
    assert!(runs[0].style.postscript_name.is_none(), "a weight override still clears the inherited exact face");
    assert!(notes.is_empty());
}

fn source(config: &[u8]) -> Vec<u8> {
    let mut parasite = W { out: Vec::new(), wide: false };
    parasite.string("gimp-text-layer");
    parasite.u32(0);
    parasite.u32(config.len() as u32);
    parasite.out.extend_from_slice(config);
    let mut layer = TestLayer::rgba("Attribute work source", 1, 1, vec![17, 31, 57, 255]);
    layer.props.push((26, 0u32.to_be_bytes().to_vec()));
    layer.props.push((21, parasite.out));
    let mut image = TestImage::rgb8(1, 1);
    image.version = 26;
    image.layers.push(layer);
    image.write()
}

#[test]
fn attribute_work_fallback_keeps_pixels_source_report_and_native_roundtrip() {
    let config = format!("(markup \"{}\") (font \"Sans\") (font-size 12)", adversarial_markup()).into_bytes();
    let bytes = source(&config);
    reset();
    let imported = with_visit_limit(256, || crate::import("attribute-work.xcf", &bytes)).unwrap();
    assert_eq!(visits(), 256);
    assert!(MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|limit| limit.get().is_none()));
    let document = imported.document;
    assert!(matches!(document.layers[0].content, photocraft_doc::LayerContent::Raster(_)));
    assert_eq!(document.layers[0].surface().unwrap().pixel(0, 0), vec![17. / 255., 31. / 255., 57. / 255., 1.]);
    let ImportConversion::Report(report) = document.metadata.import_conversion.as_deref().unwrap() else { panic!("typed report") };
    assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.text"
        && entry.reason == ConversionReason::MissingMapping
        && entry.source_description.contains("attribute-visit allowance")));
    assert!(report.attachments.iter().any(|attachment| attachment.bytes.windows(config.len()).any(|window| window == config.as_slice())));
    assert!(imported.warnings.iter().any(|warning| warning.contains("opens as pixels")));
    let native = photocraft_format::save_to_bytes(&document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.layers, document.layers);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
}

#[test]
fn immediate_cancellation_is_not_optional_attribute_work_fallback() {
    let config = format!("(markup \"{}\")", adversarial_markup()).into_bytes();
    let bytes = source(&config);
    reset();
    let cancelled = photocraft_raster::Interrupt::cancel_only(&|| true);
    assert!(matches!(with_visit_limit(0, || crate::xcf::import("cancel.xcf", &bytes, &cancelled)), Err(crate::IoError::Cancelled)));
    assert_eq!(visits(), 0);
    assert!(MARKUP_ATTRIBUTE_VISIT_LIMIT.with(|limit| limit.get().is_none()), "RAII restores an unconsumed one-shot override");
}
