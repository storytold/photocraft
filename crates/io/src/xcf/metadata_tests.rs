use super::tests::{TestImage, TestLayer, W, hierarchy};
use super::*;
use crate::initial_view::NativeImportTarget;

fn open(bytes: &[u8]) -> ImportResult {
    import("metadata.xcf", bytes, &Interrupt::NONE).expect("valid independent synthetic XCF")
}

fn parasite(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = W { out: Vec::new(), wide: true };
    out.string(name);
    out.u32(1);
    out.u32(payload.len() as u32);
    out.out.extend_from_slice(payload);
    out.out
}

#[test]
fn metadata_comments_exif_and_serialized_tags_are_native_and_round_trip() {
    let mut image = TestImage::rgb8(3, 2);
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\0\0\0\0\0\0";
    let xml = br#"<metadata><tag name="Exif.Image.Artist">Procedural test</tag><tag name="Xmp.dc.subject">one &amp; two</tag><tag name="Xmp.dc.subject">three</tag><tag name="Exif.Photo.MakerNote" encoding="base64">AA==</tag></metadata>"#;
    let mut data = parasite("gimp-comment", "Unicode λ\0".as_bytes());
    data.extend(parasite("exif-data", exif));
    data.extend(parasite("gimp-image-metadata", xml));
    image.image_props.push((21, data));
    let result = open(&image.write());
    assert_eq!(result.document.metadata.exif.as_deref().map(Vec::as_slice), Some(&exif[6..]));
    assert_eq!(
        result.document.metadata.text,
        vec![
            ("Comment".into(), "Unicode λ".into()),
            ("Exif.Image.Artist".into(), "Procedural test".into()),
            ("Xmp.dc.subject".into(), "one & two".into()),
            ("Xmp.dc.subject".into(), "three".into()),
        ]
    );
    let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).expect("native save");
    let reopened = photocraft_format::load_from_bytes(&native).expect("native reopen");
    assert_eq!(reopened.metadata.text, result.document.metadata.text);
    assert_eq!(reopened.metadata.exif, result.document.metadata.exif);
    assert!(reopened.metadata.import_conversion.is_some());
}

#[test]
fn optional_invalid_xml_and_comment_remain_inert() {
    let mut image = TestImage::rgb8(2, 2);
    let mut data = parasite(
        "gimp-image-metadata",
        br#"<!DOCTYPE metadata [<!ENTITY secret SYSTEM "file:///never-read">]><metadata><tag name="Xmp.dc.title">&secret;</tag></metadata>"#,
    );
    data.extend(parasite("gimp-comment", &[255, 254]));
    data.extend(parasite("exif-data", b"not EXIF"));
    image.image_props.push((21, data));
    let result = open(&image.write());
    assert!(result.document.metadata.text.is_empty());
    assert!(result.document.metadata.exif.is_none());
    let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = result.document.metadata.import_conversion else { panic!("report") };
    assert_eq!(report.attachments.len(), 3);
}

#[test]
fn guides_resolution_and_units_follow_native_limits_and_documented_versions() {
    for version in [11, 15] {
        let mut image = TestImage::rgb8(5, 4);
        image.version = version;
        let mut guides = Vec::new();
        for (position, orientation) in [(-2i32, 1u8), (7, 2), (2, 1), (3, 9)] {
            guides.extend(position.to_be_bytes());
            guides.push(orientation);
        }
        guides.push(1); // optional incomplete tail must not become a guide
        image.image_props.extend([(18, guides), (19, [300f32.to_be_bytes(), 150f32.to_be_bytes()].concat()), (22, 2u32.to_be_bytes().to_vec())]);
        let result = open(&image.write());
        assert_eq!(result.document.resolution_dpi, 300.0);
        assert_eq!(result.document.guides.horizontal, if version == 11 { vec![2.0] } else { vec![-2.0, 2.0] });
        assert_eq!(result.document.guides.vertical, if version == 11 { vec![] } else { vec![7.0] });
        let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = result.document.metadata.import_conversion else { panic!("report") };
        assert!(report.entries.iter().any(|e| e.feature_code == "xcf.resolution"));
        assert!(report.entries.iter().any(|e| e.feature_code == "xcf.unit"));
    }
    for y in [0.0f32, f32::NAN, f32::INFINITY] {
        let mut image = TestImage::rgb8(2, 2);
        image.image_props.push((19, [300f32.to_be_bytes(), y.to_be_bytes()].concat()));
        assert_eq!(open(&image.write()).document.resolution_dpi, 72.0);
    }
}

#[test]
fn modern_samplers_win_and_native_capacity_bounds_and_inert_fields_are_preserved() {
    let mut image = TestImage::rgb8(20, 20);
    let mut points = Vec::new();
    for x in -1i32..13 {
        points.extend(x.to_be_bytes());
        points.extend(4i32.to_be_bytes());
        points.extend(99u32.to_be_bytes());
        points.extend([0u8; 8]);
    }
    image.image_props.push((39, points));
    image.image_props.push((27, [18i32.to_be_bytes(), 18i32.to_be_bytes()].concat()));
    let result = open(&image.write());
    let view = result.initial_view.expect("sampler handoff");
    assert_eq!(view.samplers, (0..10).map(|x| [f64::from(x), 4.0]).collect::<Vec<_>>());
    view.validate(&result.document).expect("native view valid");
    let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).expect("save");
    assert!(crate::import("reopened.pcraft", &native).expect("reopen").initial_view.is_none());
}

#[test]
fn active_layer_mask_is_a_one_time_native_target() {
    let mut image = TestImage::rgb8(3, 2);
    let mut layer = TestLayer::rgba("masked", 2, 1, vec![0, 255, 0, 255, 0, 0, 255, 255]);
    layer.x = -1;
    layer.y = 1;
    layer.mask = Some(vec![64, 255]);
    layer.props.push((2, vec![]));
    layer.props.push((12, 1u32.to_be_bytes().to_vec()));
    layer.props.push((13, 1u32.to_be_bytes().to_vec()));
    image.layers.push(layer);
    let result = open(&image.write());
    let view = result.initial_view.expect("source markers");
    assert_eq!(view.selected_layers, vec![result.document.layers[0].id]);
    assert_eq!(view.active_layer, Some(result.document.layers[0].id));
    assert_eq!(view.target, NativeImportTarget::LayerMask);
    assert!(view.show_layer_mask);
    let mask = result.document.layers[0].mask.as_ref().expect("native mask");
    assert_eq!(mask.surface.read_region(Rect::new(-1, 1, 1, 2)), vec![64.0 / 255.0, 1.0]);
}

fn channel_file(props: &[(u32, Vec<u8>)], declared_size: (u32, u32)) -> Vec<u8> {
    let mut w = W::new(11);
    w.u32(5);
    w.u32(4);
    w.u32(0);
    w.u32(150);
    w.prop(17, &[0]);
    w.end();
    w.ptr(0);
    let channel = w.slot();
    w.ptr(0);
    let at = w.out.len();
    w.patch(channel, at);
    w.u32(declared_size.0);
    w.u32(declared_size.1);
    w.string("overlay");
    for (kind, bytes) in props {
        w.prop(*kind, bytes);
    }
    w.end();
    let pixels = w.slot();
    let at = hierarchy(&mut w, 2, 1, 1, &[64, 255], Compression::None);
    w.patch(pixels, at);
    w.out
}

#[test]
fn channel_overlay_and_active_target_use_native_channel_data() {
    let props = vec![
        (3, vec![]),
        (8, 1u32.to_be_bytes().to_vec()),
        (16, vec![255, 0, 0]),
        (38, [0.25f32.to_be_bytes(), 0.5f32.to_be_bytes(), 0.75f32.to_be_bytes()].concat()),
        (33, 0.3f32.to_be_bytes().to_vec()),
        (6, 255u32.to_be_bytes().to_vec()),
        (14, 0u32.to_be_bytes().to_vec()),
    ];
    let result = open(&channel_file(&props, (2, 1)));
    let channel = &result.document.channels[0];
    assert_eq!(channel.color, photocraft_color::Color::rgb(0.25, 0.5, 0.75));
    assert_eq!(channel.opacity, 0.3);
    assert_eq!(channel.indicates, photocraft_doc::ColorIndicates::SelectedAreas);
    assert_eq!(channel.surface.read_region(Rect::new(0, 0, 3, 1)), vec![64.0 / 255.0, 1.0, 0.0]);
    let view = result.initial_view.expect("active channel");
    assert_eq!(view.target, NativeImportTarget::AlphaChannel(0));
    assert_eq!(view.visible_alpha_channels, vec![0]);
    assert!(import("bad.xcf", &channel_file(&props, (3, 1)), &Interrupt::NONE).is_err());
}

#[test]
fn finite_extended_overlay_colors_are_preserved_and_nonfinite_optional_values_stay_inert() {
    let props = vec![(16, vec![64, 128, 255]), (38, [(-0.25f32).to_be_bytes(), 1.5f32.to_be_bytes(), f32::MAX.to_be_bytes()].concat())];
    let result = open(&channel_file(&props, (2, 1)));
    assert_eq!(result.document.channels[0].color, photocraft_color::Color::rgb(-0.25, 1.5, f32::MAX));
    let bad = vec![
        (16, vec![64, 128, 255]),
        (38, [f32::NAN.to_be_bytes(), 0f32.to_be_bytes(), 0f32.to_be_bytes()].concat()),
        (33, f32::INFINITY.to_be_bytes().to_vec()),
    ];
    let result = open(&channel_file(&bad, (2, 1)));
    assert_eq!(result.document.channels[0].color, photocraft_color::Color::rgb(64.0 / 255.0, 128.0 / 255.0, 1.0));
    assert_eq!(result.document.channels[0].opacity, 0.5);
    let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = result.document.metadata.import_conversion else { panic!("report") };
    assert!(report.attachments.iter().any(|a| a.kind == "xcf.property.38"));
    assert!(report.attachments.iter().any(|a| a.kind == "xcf.property.33"));
}

#[test]
fn differing_mask_extents_are_positioned_in_native_document_coordinates() {
    // Source layer 3x1 at (-2, 2), independently stored mask 2x3. A native surface has
    // document coordinates and does not need a matching-extents exception or evaluator.
    let mut w = W::new(11);
    w.u32(4);
    w.u32(5);
    w.u32(0);
    w.u32(150);
    w.prop(17, &[0]);
    w.end();
    let layer_slot = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = w.out.len();
    w.patch(layer_slot, at);
    w.u32(3);
    w.u32(1);
    w.u32(1);
    w.string("unequal mask");
    w.prop(15, &[(-2i32).to_be_bytes(), 2i32.to_be_bytes()].concat());
    w.prop_u32(11, 0);
    w.end();
    let layer_pixels = w.slot();
    let mask_slot = w.slot();
    let at = hierarchy(&mut w, 3, 1, 4, &[255; 12], Compression::None);
    w.patch(layer_pixels, at);
    let at = w.out.len();
    w.patch(mask_slot, at);
    w.u32(2);
    w.u32(3);
    w.string("mask");
    w.prop_u32(20, 123);
    w.end();
    let mask_pixels = w.slot();
    let values = [0, 64, 128, 192, 255, 32];
    let at = hierarchy(&mut w, 2, 3, 1, &values, Compression::None);
    w.patch(mask_pixels, at);
    let result = open(&w.out);
    let mask = result.document.layers[0].mask.as_ref().expect("native mask");
    assert!(!mask.enabled);
    assert!(mask.linked);
    assert_eq!(mask.surface.read_region(Rect::new(-2, 2, 0, 5)), values.map(|v| f32::from(v) / 255.0));
    let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).expect("native save");
    let reopened = photocraft_format::load_from_bytes(&native).expect("native reopen");
    assert_eq!(reopened.layers[0].mask, result.document.layers[0].mask);
}

#[test]
fn multiple_selection_records_preserve_all_coverage_without_overwriting_the_first() {
    let mut image = TestImage::rgb8(2, 1);
    image.channels.push(("first", vec![255, 0], true));
    image.channels.push(("second", vec![0, 128], true));
    let result = open(&image.write());
    assert_eq!(result.document.selection.expect("selection").read_region(Rect::new(0, 0, 2, 1)), vec![1.0, 0.0]);
    assert_eq!(result.document.channels.len(), 1);
    assert_eq!(result.document.channels[0].surface.read_region(Rect::new(0, 0, 2, 1)), vec![0.0, 128.0 / 255.0]);
}

#[test]
fn native_view_overlay_hints_reject_ambiguous_or_absent_targets() {
    let mut document = Document::new("native", Size::new(3, 2), ColorMode::Rgb, SampleType::U8);
    document.channels.push(AlphaChannel::new("alpha", Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false))));
    for visible_alpha_channels in [vec![1], vec![0, 0]] {
        let view = crate::initial_view::NativeInitialView { visible_alpha_channels, ..Default::default() };
        assert!(view.validate(&document).is_err());
    }
    let view = crate::initial_view::NativeInitialView { show_layer_mask: true, ..Default::default() };
    assert!(view.validate(&document).is_err());
    let view = crate::initial_view::NativeInitialView { visible_alpha_channels: vec![0], ..Default::default() };
    view.validate(&document).expect("existing native alpha overlay");
}

#[test]
fn source_xmp_schemas_become_native_rdf_arrays_and_language_alternatives() {
    let mut image = TestImage::rgb8(2, 2);
    let xml = br#"<metadata><namespace prefix="dc" url="http://purl.org/dc/elements/1.1/"/>
      <tag name="Xmp.dc.subject">first &amp; second</tag><tag name="Xmp.dc.subject">third</tag>
      <tag name="Xmp.dc.creator">Author one</tag><tag name="Xmp.dc.creator">Author two</tag>
      <tag name="Xmp.dc.title">lang=&quot;x-default&quot; A &lt; B</tag><tag name="Xmp.dc.title">lang=&quot;de-DE&quot; Titel</tag>
      <tag name="Xmp.xmp.CreatorTool">Source editor</tag><tag name="Xmp.unknown.structure">never interpreted</tag>
    </metadata>"#;
    image.image_props.push((21, parasite("gimp-image-metadata", xml)));
    let result = open(&image.write());
    let packet = result.document.metadata.xmp.as_ref().unwrap();
    let rdf = roxmltree::Document::parse(packet).unwrap();
    let subject = rdf.descendants().find(|node| node.has_tag_name(("http://purl.org/dc/elements/1.1/", "subject"))).unwrap();
    assert!(subject.children().any(|node| node.has_tag_name(("http://www.w3.org/1999/02/22-rdf-syntax-ns#", "Bag"))));
    assert_eq!(
        subject
            .descendants()
            .filter(|node| node.has_tag_name(("http://www.w3.org/1999/02/22-rdf-syntax-ns#", "li")))
            .filter_map(|node| node.text())
            .collect::<Vec<_>>(),
        vec!["first & second", "third"]
    );
    let title = rdf.descendants().find(|node| node.has_tag_name(("http://purl.org/dc/elements/1.1/", "title"))).unwrap();
    assert!(title.descendants().any(|node| node.attribute(("http://www.w3.org/XML/1998/namespace", "lang")) == Some("de-DE") && node.text() == Some("Titel")));
    assert!(!packet.contains("never interpreted"));
    let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).unwrap();
    let reopened = photocraft_format::load_from_bytes(&native).unwrap();
    assert_eq!(reopened.metadata.xmp, result.document.metadata.xmp);
    assert_eq!(reopened.metadata.text, result.document.metadata.text);
}

#[test]
fn source_ascii_exif_fields_become_a_native_tiff_packet() {
    let mut image = TestImage::rgb8(2, 2);
    let xml=br#"<metadata><tag name="Exif.Image.Artist">Synthetic Artist</tag><tag name="Exif.Image.Software">Native metadata test</tag><tag name="Exif.Photo.MakerNote" encoding="base64">AA==</tag><tag name="Exif.Image.Orientation">8</tag></metadata>"#;
    image.image_props.push((21, parasite("gimp-image-metadata", xml)));
    let result = open(&image.write());
    let exif = result.document.metadata.exif.as_deref().unwrap();
    assert_eq!(&exif[..8], b"II\x2a\0\x08\0\0\0");
    assert_eq!(u16::from_le_bytes(exif[8..10].try_into().unwrap()), 2);
    let fields: Vec<_> = (0..2)
        .map(|i| {
            let start = 10 + i * 12;
            let id = u16::from_le_bytes(exif[start..start + 2].try_into().unwrap());
            let len = u32::from_le_bytes(exif[start + 4..start + 8].try_into().unwrap()) as usize;
            let offset = u32::from_le_bytes(exif[start + 8..start + 12].try_into().unwrap()) as usize;
            (id, &exif[offset..offset + len])
        })
        .collect();
    assert_eq!(fields, vec![(305, b"Native metadata test\0".as_slice()), (315, b"Synthetic Artist\0".as_slice())]);
    assert_eq!(photocraft_codecs::exif_orientation(exif), 1);
    let native = photocraft_format::save_to_bytes(&result.document, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&native).unwrap().metadata.exif, result.document.metadata.exif);
}

#[test]
fn ambiguous_xmp_types_namespaces_and_duplicate_scalars_remain_inert() {
    let mut image = TestImage::rgb8(2, 2);
    let xml=br#"<metadata><namespace prefix="dc" url="https://invalid.example/not-dc"/><tag name="Xmp.dc.subject">wrong namespace</tag><tag name="Xmp.xmp.Label">one</tag><tag name="Xmp.xmp.Label">two</tag><tag name="Xmp.photoshop.Unknown">untyped</tag><tag name="Exif.Image.Artist">one</tag><tag name="Exif.Image.Artist">two</tag></metadata>"#;
    image.image_props.push((21, parasite("gimp-image-metadata", xml)));
    let result = open(&image.write());
    assert!(result.document.metadata.xmp.is_none());
    assert!(result.document.metadata.exif.is_none());
    assert_eq!(result.document.metadata.text.len(), 6);
}
