//! Archived GIMP-authored source evidence, not executable effect-stage qualification.
//! Fixtures and original procedural generators are pinned in photocraft-corpus;
//! their provenance is recorded in ATTRIBUTION.md. Missing corpus inputs fail.

#![cfg(feature = "corpus")]

use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, ConversionReport, ImportConversion};
use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Point;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}: run `cargo xtask corpus --gimp`", path.display()))
}

fn report(document: &Document) -> &ConversionReport {
    match document.metadata.import_conversion.as_deref() {
        Some(ImportConversion::Report(report)) => report,
        _ => panic!("expected durable source conversion evidence"),
    }
}

fn reopen(document: &Document) -> Document {
    let bytes = photocraft_format::save_to_bytes(document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
    // Includes layer IDs, saved raster/type caches and their appearance policies.
    assert_eq!(reopened.layers, document.layers);
    assert_eq!(reopened.paths, document.paths);
    assert_eq!(reopened.metadata.import_conversion, document.metadata.import_conversion);
    reopened
}

fn assert_attachment_source(document: &Document, source: &[u8]) {
    let report = report(document);
    assert!(!report.attachments.is_empty());
    for entry in &report.entries {
        for &index in &entry.attachments {
            let attachment = &report.attachments[index as usize];
            assert_eq!(attachment.owner, entry.source);
            let start = attachment.owner.record_offset.unwrap() as usize;
            let end = start.checked_add(attachment.bytes.len()).unwrap();
            assert!(!attachment.bytes.is_empty());
            assert_eq!(attachment.bytes.as_slice(), source.get(start..end).unwrap(), "source record bytes must not be regenerated");
        }
    }
}

#[test]
fn archived_fractional_paths_keep_names_order_handles_and_off_canvas_geometry() {
    let name = "paths-fractional-open-closed.xcf";
    let imported = photocraft_io::import(name, &fixture(name)).unwrap();
    assert!(imported.source_read_only && !imported.preview_only);
    let document = &imported.document;
    assert_eq!(document.paths.iter().map(|path| path.name.as_str()).collect::<Vec<_>>(), ["Closed fractional cubic", "Open fractional cubic"]);
    for (path, closed) in document.paths.iter().zip([true, false]) {
        assert_eq!(path.path.subpaths.len(), 1);
        let stroke = &path.path.subpaths[0];
        assert_eq!(stroke.closed, closed);
        assert_eq!(stroke.knots.len(), 3);
        // Original producer coordinates: negative/fractional handles and an
        // anchor outside the 64x48 canvas must survive without clipping.
        assert_eq!(stroke.knots[0].anchor, Point::new(-4.5, 4.25));
        // This v11 archive uses legacy PROP_PATHS. Its open record has
        // eight points rather than the closed record's nine: GIMP does not
        // serialize the unused incoming handle of the first open anchor.
        // The shared reader represents that absent handle at the anchor.
        let incoming = if closed { Point::new(-8.5, 2.25) } else { stroke.knots[0].anchor };
        assert_eq!(stroke.knots[0].in_ctrl, incoming);
        assert_eq!(stroke.knots[0].out_ctrl, Point::new(2.5, 8.25));
        assert!(stroke.knots[2].anchor.x > f64::from(document.size.width));
        assert!(stroke.knots[2].anchor.y > f64::from(document.size.height));
        assert_eq!(stroke.knots[1].anchor, Point::new(24.125, 10.5));
        assert_eq!(stroke.knots[1].in_ctrl, Point::new(18.125, -6.5));
        assert_eq!(stroke.knots[1].out_ctrl, Point::new(32.125, 18.5));
    }
    reopen(document);
}

#[test]
fn archived_unicode_text_retains_ambiguous_colors_and_independently_modified_pixels() {
    let name = "text-unicode-fixed-markup.xcf";
    let source = fixture(name);
    let imported = photocraft_io::import(name, &source).unwrap();
    assert!(imported.source_read_only && !imported.preview_only);
    let layer = imported.document.layers.iter().find(|layer| layer.name == "héλlo ...").unwrap();
    // Only the lambda/lo span has portable foreground markup. The initial hé and
    // trailing CJK runs use a GEGL native-float base color whose author byte
    // order is not recorded; drawable byte order cannot establish that color.
    let LayerContent::Raster(surface) = &layer.content else { panic!("ambiguous base color must keep the saved appearance") };
    assert!(surface.tiles().next().is_some());
    let entry = report(&imported.document).entries.iter().find(|entry| entry.feature_code == "xcf.text").unwrap();
    assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied);
    assert_eq!(entry.reason, ConversionReason::NativeLimitation);
    assert!(entry.native_description.contains("byte-order evidence"));
    assert_eq!(entry.targets[0].layer_id, Some(layer.id));
    assert!(!entry.attachments.is_empty());
    assert!(imported.warnings.iter().any(|warning| warning.contains("byte-order evidence")));
    assert_attachment_source(&imported.document, &source);
    let reopened = reopen(&imported.document);
    assert_attachment_source(&reopened, &source);

    let modified = "text-unicode-modified-cache.xcf";
    let modified_source = fixture(modified);
    let imported = photocraft_io::import(modified, &modified_source).unwrap();
    let layer = imported.document.layers.iter().find(|layer| layer.name == "héλlo ...").unwrap();
    let LayerContent::Raster(surface) = &layer.content else { panic!("independent pixel edits must not be replaced by editable text") };
    // The producer overwrote every saved type-cache pixel with this original
    // opaque pattern after its destructive edit. Sample in image coordinates.
    for (x, y) in [(0, 0), (11, 9), (63, 41)] {
        for channel in 0..4 {
            let byte = if channel == 3 { 255 } else { (x * 17 + y * 5 + channel as i32 * 61) % 256 };
            assert_eq!(surface.sample_channel(x - 3, y + 7, channel), byte as f32 / 255.);
        }
    }
    let entry = report(&imported.document).entries.iter().find(|entry| entry.feature_code == "xcf.text").unwrap();
    assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied);
    assert_eq!(entry.targets[0].layer_id, Some(layer.id));
    assert!(imported.warnings.iter().any(|warning| warning.contains("modified independently")));
    assert_attachment_source(&imported.document, &modified_source);
    assert_attachment_source(&reopen(&imported.document), &modified_source);
}

#[test]
fn archived_legacy_numeric_color_text_stays_editable_with_its_saved_appearance() {
    let name = "historical-2-10-32-legacy-text-path.xcf";
    let source = fixture(name);
    let imported = photocraft_io::import(name, &source).unwrap();
    assert!(imported.source_read_only && !imported.preview_only);
    let layer = imported
        .document
        .layers
        .iter()
        .find(|layer| matches!(&layer.content, LayerContent::Text(text) if text.text == "Legacy XCF"))
        .expect("portable producer text remains editable");
    let LayerContent::Text(text) = &layer.content else { panic!("editable legacy text") };
    // This historical file has no fixed-name flag.
    assert!(!text.keep_layer_name);
    assert!(text.cache_uses_raster_blending);
    assert!(text.cache.as_ref().is_some_and(|cache| cache.tiles().next().is_some()));
    assert_eq!(text.color.alpha, 1.);
    assert!(text.color.c.iter().take(3).all(|sample| sample.abs() < 0.001));
    assert_eq!(text.runs.iter().map(|run| run.len).sum::<usize>(), text.text.len());
    assert_eq!(text.paragraphs[0].style.direction, photocraft_doc::text::TextDirection::Ltr);
    let entry = report(&imported.document).entries.iter().find(|entry| entry.feature_code == "xcf.text").unwrap();
    assert_eq!(entry.disposition, ConversionDisposition::NativeApproximation);
    assert_eq!(entry.targets[0].layer_id, Some(layer.id));
    assert!(!entry.attachments.is_empty());
    assert_attachment_source(&imported.document, &source);
    let reopened = reopen(&imported.document);
    let LayerContent::Text(reopened_text) = &reopened.layers.iter().find(|candidate| candidate.id == layer.id).unwrap().content else {
        panic!("editable after save")
    };
    assert_eq!(reopened_text.text, "Legacy XCF");
    assert_attachment_source(&reopened, &source);
}

#[test]
fn archived_effect_stack_retains_exact_ordered_owner_evidence_without_claiming_application() {
    let name = "effect-stack-distinct-masks-disabled.xcf";
    let source = fixture(name);
    let imported = photocraft_io::import(name, &source).unwrap();
    assert!(imported.source_read_only && !imported.preview_only);
    let document = &imported.document;
    assert_eq!(document.layers.iter().map(|layer| layer.name.as_str()).collect::<Vec<_>>(), ["Unaffected sibling", "Effect stack owner"]);
    let sibling = document.layers[0].surface().unwrap();
    for (x, y) in [(9, 7), (31, 26)] {
        let rgb = [0, 1, 2].map(|channel| (((x + 8) * 31 + y * 17 + channel * 101 + 7) * 13 % 256) as f32 / 255.);
        let alpha = ((x + y) * 255 / 76).min(255) as f32 / 255.;
        assert_eq!(sibling.rgba(x, y), [rgb[0], rgb[1], rgb[2], alpha]);
    }
    let owner = &document.layers[1];
    let LayerContent::Raster(surface) = &owner.content else { panic!("inventory-only effects must preserve saved owner pixels") };
    assert!(owner.effects.items.is_empty(), "source inventory is not a native effect stack");
    // Independent source generator oracle, before its non-destructive filters.
    for (x, y) in [(0, 0), (3, 3), (18, 12)] {
        for channel in 0..4 {
            let byte = if channel == 3 { ((x + y) * 255 / 28).min(255) } else { ((x + 2) * 31 + y * 17 + channel as i32 * 101 + 7) * 13 % 256 };
            assert_eq!(surface.sample_channel(x + 7, y + 5, channel), byte as f32 / 255.);
        }
    }
    let report = report(document);
    let effects = report.entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect").collect::<Vec<_>>();
    // Serialized order is the reverse of filter creation: disabled exposure
    // remains present, preceding brightness/contrast with its distinct mask.
    assert_eq!(effects.len(), 2);
    for (ordinal, (entry, (name, operation, reason))) in effects
        .iter()
        .zip([
            ("Stack stage 2", "gegl:exposure", ConversionReason::NativeLimitation),
            ("Stack stage 1", "gimp:brightness-contrast", ConversionReason::MissingMapping),
        ])
        .enumerate()
    {
        assert_eq!(entry.source.kind, "layer_effect");
        assert_eq!(entry.source.ordinal, ordinal as u32);
        assert_eq!(entry.source.item_path, [0]);
        assert_eq!(entry.source.name, name);
        assert_eq!(entry.source_description, operation);
        assert_eq!(entry.targets.len(), 1);
        assert_eq!(entry.targets[0].layer_id, Some(owner.id));
        assert_eq!(entry.targets[0].label, owner.name);
        assert_eq!(entry.disposition, ConversionDisposition::RetainedUnapplied);
        assert_eq!(entry.reason, reason);
        assert_eq!(entry.attachments.len(), 1);
        let attachment = &report.attachments[entry.attachments[0] as usize];
        assert_eq!(attachment.kind, "xcf.layer_effect");
        assert_eq!(attachment.encoding, "xcf_effect_record");
    }
    assert_ne!(effects[0].source.record_offset, effects[1].source.record_offset);
    assert_archived_stage_masks(&source, &effects);
    assert!(imported.warnings.iter().any(|warning| warning.contains("2 layer effect(s)") && warning.contains("not applied")));
    assert_attachment_source(document, &source);
    let reopened = reopen(document);
    assert_attachment_source(&reopened, &source);
}

fn assert_archived_stage_masks(source: &[u8], effects: &[&ConversionEntry]) {
    // The producer sets distinct selections before adding each stage. The shared reader
    // exposes their exact stored coverage independently of display opacity and visibility.
    let xcf = craft_xcf::Xcf::parse(source).unwrap();
    let mut effect_budget = craft_xcf::effects::Budget::default();
    let mut metadata = craft_xcf::records::Budget::default();
    let keep_going = || true;
    let inventory = xcf.layer_effects(0, &mut effect_budget, &keep_going).unwrap();
    let reader = craft_xcf::effects::Reader::new(source, xcf.image().version);
    let mut mask_offsets = Vec::new();
    for (ordinal, (offset, (left, top))) in inventory.offsets.into_iter().zip([(12, 8), (7, 5)]).enumerate() {
        assert_eq!(Some(offset), effects[ordinal].source.record_offset);
        let owned = reader.read(inventory.owner, offset, &mut effect_budget, &keep_going).unwrap();
        assert_eq!(owned.record.visible, ordinal == 1);
        let mask = xcf.effect_mask_header(&owned.record, &mut metadata, &keep_going).unwrap().unwrap();
        assert_eq!((mask.width(), mask.height()), (48, 32));
        mask_offsets.push(mask.source_offset());
        let mut bytes = 48 * 32;
        xcf.visit_effect_mask_tiles(&mask, Default::default(), &mut bytes, &keep_going, |tile| {
            assert_eq!(tile.drawable.channels, 1);
            for (i, sample) in tile.data.iter().enumerate() {
                let x = tile.x + i as u32 % tile.width;
                let y = tile.y + i as u32 / tile.width;
                let expected = if (left..left + 8).contains(&x) && (top..top + 6).contains(&y) { 255 } else { 0 };
                assert_eq!(*sample, expected, "stage {ordinal}, pixel ({x}, {y})");
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(bytes, 0);
    }
    assert_eq!(mask_offsets.len(), 2);
    assert_ne!(mask_offsets[0], mask_offsets[1]);
}

#[test]
fn archived_binary_graph_source_only_route_keeps_pixels_and_exact_native_evidence() {
    let source = fixture("graph-original-over-inverted.xcf");
    let imported = photocraft_io::import("graph-original-over-inverted.xcf", &source).unwrap();
    assert!(imported.source_read_only && !imported.preview_only);
    assert_eq!(imported.document.layers.len(), 1);
    let owner = &imported.document.layers[0];
    let surface = owner.surface().expect("unverified graph envelope must not replace the original owner");
    // Original procedural producer input, not the saved filtered preview.
    assert_eq!(surface.rgba(0, 0), [17. / 255., 13. / 255., 29. / 255., 64. / 255.]);
    let effects: Vec<_> = report(&imported.document).entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect").collect();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].source_description, "gegl:gegl");
    assert_eq!(effects[0].disposition, ConversionDisposition::RetainedUnapplied);
    assert_eq!(effects[0].reason, ConversionReason::NativeLimitation);
    // This source has unverified blend/composite-space words, an auxiliary mask
    // and an extra `error` argument. Its filename is not native admission proof.
    assert_attachment_source(&imported.document, &source);
    let reopened = reopen(&imported.document);
    assert_attachment_source(&reopened, &source);
}
