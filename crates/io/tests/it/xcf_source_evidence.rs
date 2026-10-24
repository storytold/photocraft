//! Independently saved GIMP 3.2.6 sources awaiting public corpus publication/pinning.
//! Expected values come from original authoring inputs, not importer-produced snapshots.
//! PNGs are source previews, never a native-rendering/HDR equivalence oracle.
#![cfg(feature = "corpus")]

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, SmartFilter, SmartSource};
use photocraft_geom::{Affine, Point, Rect};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; requires expanded public upstream XCF corpus pin (T081)", path.display()))
}
fn open(stem: &str) -> Document {
    let name = format!("{stem}.xcf");
    let result = photocraft_io::import(&name, &fixture(&name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(result.source_read_only);
    let preview = format!("{stem}.png");
    let preview = photocraft_io::import(&preview, &fixture(&preview)).expect("saved source PNG preview").document;
    assert_eq!(preview.size, result.document.size, "source preview dimensions: {stem}");
    result.document
}
fn layer<'a>(doc: &'a Document, name: &str) -> &'a Layer {
    doc.layers.iter().find(|layer| layer.name == name).unwrap_or_else(|| panic!("missing {name}"))
}
fn raster(layer: &Layer) -> &photocraft_raster::Surface {
    let LayerContent::Raster(surface) = &layer.content else { panic!("{} must be editable native pixels", layer.name) };
    surface
}
fn roundtrip(doc: &Document) {
    // No source pathname is supplied during native reopen.
    let bytes = photocraft_format::save_to_bytes(doc, &Default::default()).expect("native save");
    let reopened = photocraft_format::load_from_bytes(&bytes).expect("source-independent native reopen");
    assert!(reopened.layers == doc.layers, "native layer data changed on reopen");
    assert_eq!(reopened.paths, doc.paths);
    assert!(reopened.channels == doc.channels, "native channels changed on reopen");
    assert!(reopened.selection == doc.selection, "native selection changed on reopen");
    assert_eq!(reopened.icc_profile, doc.icc_profile);
    assert!(reopened.metadata == doc.metadata, "historical metadata changed on reopen");
}
fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!((actual - expected).abs() <= tolerance, "actual {actual}, expected {expected}");
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_precision_samples_preserve_signed_hdr_rounding_offsets_and_native_reopen() {
    let cases: [(&str, usize, usize, &[f64], bool); 4] = [
        ("precision-u32-nonlinear", 17, 5, &[0., 1., 2., 16777215., 16777217., 2147483647., 2147483648., 4294967294., 4294967295.], true),
        ("precision-half-hdr", 9, 7, &[0., -0., 2f64.powi(-24), 2f64.powi(-14), -0.25, 0.5, 1., 1.5, 65504.], false),
        ("precision-float-hdr", 11, 3, &[0., -0., 2f64.powi(-149), 2f64.powi(-126), -0.125, 0.25, 1., 2., 4.], false),
        (
            "precision-double-gray-boundaries",
            13,
            5,
            &[0., -0., f64::from_bits(1), 2f64.powi(-150), 2f64.powi(-149), 1. + 2f64.powi(-24), 1. + 3. * 2f64.powi(-24), 1.5, -0.25, 2.],
            false,
        ),
    ];
    for (name, w, h, colors, integer) in cases {
        let doc = open(name);
        let gray = name.contains("gray");
        assert_eq!(doc.mode, if gray { ColorMode::Grayscale } else { ColorMode::Rgb });
        assert_eq!(doc.depth, SampleType::F32);
        assert_eq!((doc.size.width, doc.size.height), (w as u32 + 4, h as u32 + 3));
        near(f64::from(doc.resolution_dpi), 96., 0.001);
        let surface = raster(layer(&doc, "Stored precision samples"));
        let n = if gray { 2 } else { 4 };
        let samples = surface.read_region(Rect::new(-2, 1, w as i32 - 2, h as i32 + 1));
        for y in 0..h {
            for x in 0..w {
                for c in 0..n {
                    let expected = if c == n - 1 {
                        if integer { [0., 1073741824., 2147483648., 4294967295.][(x + y) % 4] / 4294967295. } else { [0., 0.25, 0.5, 1.][(x + y) % 4] }
                    } else {
                        let value = colors[(x + y * 3 + c) % colors.len()];
                        if integer { value / 4294967295. } else { value }
                    } as f32;
                    let actual = samples[(y * w + x) * n + c];
                    assert_eq!(actual.to_bits(), expected.to_bits(), "{name} ({x},{y}) c{c}");
                }
            }
        }
        roundtrip(&doc);
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_standard_wrappers_open_the_same_native_precision_data() {
    let raw = open("precision-u32-nonlinear");
    for suffix in ["gz", "bz2", "xz"] {
        let name = format!("precision-u32-nonlinear.xcf.{suffix}");
        let result = photocraft_io::import(&name, &fixture(&name)).expect("independently encoded standard wrapper");
        assert!(result.source_read_only);
        assert_eq!(result.document.layers.len(), raw.layers.len());
        for (actual, expected) in result.document.layers.iter().zip(&raw.layers) {
            assert_eq!(actual.name, expected.name);
            assert!(actual.content == expected.content, "wrapper altered native pixels");
            assert_eq!((actual.opacity, actual.blend, actual.visible), (expected.opacity, expected.blend, expected.visible));
        }
        assert_eq!(result.document.icc_profile, raw.icc_profile);
        roundtrip(&result.document);
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_original_profile_colorants_and_literal_samples_follow_native_transfer_policy() {
    for model in ["rgb", "gray"] {
        for encoding in ["linear", "perceptual"] {
            let name = format!("profile-original-{model}-{encoding}");
            let doc = open(&name);
            assert_eq!(doc.depth, SampleType::F32);
            let profile = photocraft_cms::Profile::parse(doc.icc_profile.as_ref().expect("native ICC")).expect("usable native ICC");
            assert!(profile.description.contains("Original procedural"));
            near(profile.white_point[0], 0.9642, 1. / 65536.);
            near(profile.white_point[1], 1., 1. / 65536.);
            near(profile.white_point[2], 0.8249, 1. / 65536.);
            if model == "rgb" {
                let matrix = profile.matrix.expect("assigned original colorants");
                for (actual, expected) in matrix.iter().flatten().zip([0.6, 0.25, 0.1142, 0.2, 0.7, 0.1, 0.01, 0.1, 0.7149]) {
                    near(*actual, expected, 1. / 65536.);
                }
            }
            let curve = if model == "gray" { profile.gray_trc.as_ref().expect("Gray curve") } else { &profile.trc.as_ref().expect("RGB curves")[0] };
            let expected = if encoding == "linear" { 0.5 } else { ((0.5f64 + 0.055) / 1.055).powf(2.4) };
            near(curve.eval64(0.5), expected, 0.0001);
            let surface = raster(layer(&doc, "Literal assigned-profile samples"));
            let n = if model == "gray" { 2 } else { 4 };
            let values = surface.read_region(Rect::new(2, -1, 11, 4));
            for y in 0..5 {
                for x in 0..9 {
                    for c in 0..n {
                        let expected: f32 = if c == n - 1 { 1. } else { [0.125, 0.25, 0.5, 0.75][(x + y + c) % 4] };
                        assert_eq!(values[(y * 9 + x) * n + c].to_bits(), expected.to_bits(), "literal storage {name}");
                    }
                }
            }
            roundtrip(&doc);
        }
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_unicode_fixed_markup_is_native_type_and_modified_cache_stays_pixels() {
    let doc = open("text-unicode-fixed-markup");
    let native = doc.layers.iter().find_map(|l| if let LayerContent::Text(t) = &l.content { Some(t) } else { None }).expect("native editable text");
    assert_eq!(native.text, "héλlo\n漢字");
    assert_eq!(native.transform, Affine::translate(-3., 7.));
    assert_eq!(native.shape, photocraft_doc::text::TextShape::Box { x: 0., y: 0., width: 120., height: 60. });
    assert!(native.runs.len() > 1, "styled source becomes ordinary native runs");
    assert!(native.runs.iter().any(|r| r.style.weight == 700));
    assert!(native.cache.is_none(), "saved GIMP cache is not native editable type rendering");
    roundtrip(&doc);
    let modified = open("text-unicode-modified-cache");
    assert!(modified.layers.iter().all(|l| !matches!(l.content, LayerContent::Text(_))));
    let pixels = raster(modified.layers.last().expect("modified text pixels"));
    let p = pixels.read_region(Rect::new(2, 10, 3, 11)); // local(5,3), offset(-3,7)
    for (c, value) in p.iter().take(3).enumerate() {
        near(f64::from(*value) * 255., ((5 * 17 + 3 * 5 + c * 61) % 256) as f64, 0.51);
    }
    assert_eq!(p[3], 1.);
    roundtrip(&modified);
}

#[test]
#[ignore = "Requires locally authored named-color source or expanded upstream XCF corpus pin"]
fn saved_named_markup_colors_become_native_editable_runs() {
    let doc = open("text-named-markup-colors");
    let native = doc.layers.iter().find_map(|l| if let LayerContent::Text(t) = &l.content { Some(t) } else { None }).expect("native editable named text");
    assert_eq!(native.text, "Green Gray Gold 42 Steel\nY M P R L C B");
    assert_eq!(native.transform, Affine::translate(3., 4.));
    assert!(native.cache.is_none());
    let expected: [(&str, [u8; 3], f32); 12] = [
        ("Green ", [0, 128, 0], 1.),
        ("Gray ", [128, 128, 128], 1.),
        ("Gold ", [250, 250, 210], 32768. / 65535.),
        ("42 ", [107, 107, 107], 1.),
        ("Steel\n", [54, 100, 139], 1.),
        ("Y ", [128, 128, 128], 1.),
        ("M ", [128, 0, 0], 1.),
        ("P ", [128, 0, 128], 1.),
        ("R ", [255, 0, 0], 1.),
        ("L ", [250, 250, 210], 1.),
        ("C ", [250, 250, 210], 1.),
        ("B", [102, 51, 153], 1.),
    ];
    // Original authoring literals, converted independently through native CMS.
    let profile = photocraft_cms::Profile::parse(doc.icc_profile.as_deref().expect("saved source profile")).unwrap();
    let transform =
        photocraft_cms::Transform::new(photocraft_cms::Builtin::Srgb.profile(), &profile, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    for (word, rgb, alpha) in expected {
        let at = native.text.find(word).expect("original text span");
        let mut offset = 0;
        let run = native
            .runs
            .iter()
            .find(|run| {
                let contains = (offset..offset + run.len).contains(&at);
                offset += run.len;
                contains
            })
            .unwrap();
        let source = rgb.map(|channel| f32::from(channel) / 255.);
        let mut converted = [0.; 3];
        transform.convert_f32(&source, 3, &mut converted, 3, false);
        for (actual, expected) in run.style.color.c.iter().take(3).zip(converted) {
            near(f64::from(*actual), f64::from(expected), 0.00001);
        }
        near(f64::from(run.style.color.alpha), f64::from(alpha), 0.00001);
    }
    roundtrip(&doc);
}

fn check_paths(doc: &Document, legacy_open_handles: bool) {
    assert_eq!(doc.paths.len(), 2);
    for (name, closed) in [("Open fractional cubic", false), ("Closed fractional cubic", true)] {
        let path = &doc.paths.iter().find(|p| p.name == name).expect("named native path").path;
        assert_eq!(path.subpaths.len(), 1);
        let sub = &path.subpaths[0];
        assert_eq!(sub.closed, closed);
        assert_eq!(sub.knots.len(), 3);
        for (index, (k, (incoming, anchor, outgoing))) in sub
            .knots
            .iter()
            .zip([((-8.5, 2.25), (-4.5, 4.25), (2.5, 8.25)), ((18.125, -6.5), (24.125, 10.5), (32.125, 18.5)), ((72.5, 36.25), (68.5, 52.25), (64.5, 58.25))])
            .enumerate()
        {
            // Stock legacy open-path serialization omits the unused first incoming handle.
            let incoming = if legacy_open_handles && !closed && index == 0 { anchor } else { incoming };
            assert_eq!(k.in_ctrl, Point::new(incoming.0, incoming.1));
            assert_eq!(k.anchor, Point::new(anchor.0, anchor.1));
            assert_eq!(k.out_ctrl, Point::new(outgoing.0, outgoing.1));
        }
    }
}
#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_fractional_paths_vector_stroke_and_modified_pixels_stay_native() {
    let paths = open("paths-fractional-open-closed");
    check_paths(&paths, true);
    roundtrip(&paths);
    let vector = open("vector-solid-dashed-stroke");
    check_paths(&vector, false);
    let shape = vector.layers.iter().find_map(|l| if let LayerContent::Shape(s) = &l.content { Some(s) } else { None }).expect("native editable vector");
    assert_eq!(shape.path, vector.paths.iter().find(|p| p.name == "Closed fractional cubic").unwrap().path);
    assert!(shape.fill.is_some());
    let stroke = shape.stroke.as_ref().expect("native vector stroke");
    assert_eq!(stroke.width, 3.5);
    assert_eq!(stroke.dashes, [2., 1., 4., 1.]);
    assert_eq!(stroke.cap, photocraft_doc::vector::LineCap::Round);
    assert_eq!(stroke.join, photocraft_doc::vector::LineJoin::Bevel);
    roundtrip(&vector);
    let modified = open("vector-modified-cache");
    check_paths(&modified, false);
    assert!(modified.layers.iter().all(|l| !matches!(l.content, LayerContent::Shape(_))));
    let cache = raster(layer(&modified, "Closed fractional cubic"));
    // The saved modified raster PROP_OFFSETS is(-8,-1), unlike GIMP's public
    // VectorLayer path-bounds offset(-6,0). Pixels must follow their saved raster frame.
    assert_eq!(cache.content_bounds(), Rect::new(-8, -1, 72, 56));
    let pixel = cache.read_region(Rect::new(2, 4, 3, 5)); // stored-cache local(10,5)
    for (c, value) in pixel.iter().take(3).enumerate() {
        near(f64::from(*value) * 255., ((10 * 7 + 5 * 19 + c * 53) % 256) as f64, 0.51);
    }
    assert_eq!(pixel[3], 1.);
    roundtrip(&modified);
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_floating_pixels_remain_separate_editable_layers_without_anchoring() {
    for kind in ["rgb", "channel"] {
        let doc = open(&format!("floating-{kind}-negative-offset"));
        let floating = layer(&doc, "Floating original pixels");
        near(f64::from(floating.opacity), 0.75, 0.0001);
        let pixels = raster(floating).read_region(Rect::new(-2, 3, 7, 10));
        for y in 0..7 {
            for x in 0..9 {
                for c in 0..3 {
                    let source_c = if kind == "channel" { 0 } else { c };
                    near(f64::from(pixels[(y * 9 + x) * 4 + c]) * 255., ((x * 19 + y * 23 + source_c * 37) % 256) as f64, 0.51);
                }
                near(f64::from(pixels[(y * 9 + x) * 4 + 3]) * 255., 192., 0.51);
            }
        }
        let base = raster(layer(&doc, "Attachment layer")).read_region(Rect::new(3, 7, 4, 8));
        // Inside the floating overlap: anchoring must not modify the independently stored base.
        for (c, value) in base.iter().take(3).enumerate() {
            near(f64::from(*value) * 255., (((5 * 31 + 7 * 17 + c * 101 + 7) * 13) % 256) as f64, 0.51);
        }
        near(f64::from(base[3]) * 255., (10 * 255 / 52) as f64, 0.51);
        if kind == "channel" {
            let channel = doc.channels.iter().find(|c| c.name == "Attachment channel").expect("attachment channel preserved");
            near(f64::from(channel.surface.read_region(Rect::new(5, 7, 6, 8))[0]) * 255., (5 * 13 + 7 * 7) as f64, 0.51);
        }
        roundtrip(&doc);
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_relative_link_uses_stored_pixels_and_metadata_channel_stays_native() {
    let doc = open("link-relative-stored-pixels");
    let cached = raster(layer(&doc, "Relative original resource"));
    let resource = photocraft_io::import("link-source.png", &fixture("link-source.png")).expect("original independently saved resource").document;
    assert_eq!((resource.size.width, resource.size.height), (12, 9));
    let expected = raster(&resource.layers[0]).read_region(Rect::new(5, 4, 6, 5));
    let actual = cached.read_region(Rect::new(3, 7, 4, 8));
    for (a, b) in actual.iter().zip(expected) {
        near(f64::from(*a), f64::from(b), 1. / 255.);
    }
    let mut linked_layers = Vec::new();
    native_layers(&doc.layers, 0, &mut linked_layers); // also rejects any external SmartSource::Linked
    roundtrip(&doc);
    let metadata = open("metadata-channel-view");
    near(f64::from(metadata.resolution_dpi), 144., 0.001);
    assert_eq!(metadata.guides.horizontal, [7.]);
    assert_eq!(metadata.guides.vertical, [19.]);
    let channel = metadata.channels.iter().find(|c| c.name == "Overlay channel").expect("native saved channel");
    near(f64::from(channel.opacity), 0.375, 0.001);
    near(f64::from(channel.surface.read_region(Rect::new(5, 7, 6, 8))[0]) * 255., (5 * 5 + 7 * 11) as f64, 0.51);
    assert!(metadata.metadata.xmp.as_deref().is_some_and(|s| s.contains("Nonpersonal fixture description")));
    roundtrip(&metadata);
}

fn native_layers(layers: &[Layer], depth: usize, out: &mut Vec<Layer>) {
    assert!(depth < 16, "bounded ordinary native source nesting");
    for layer in layers {
        out.push(layer.clone());
        match &layer.content {
            LayerContent::Group(group) => native_layers(&group.children, depth + 1, out),
            LayerContent::Smart(s) => {
                if let SmartSource::Embedded { file_name, bytes } = &s.source {
                    if file_name.ends_with(".pcraft") {
                        let embedded = photocraft_format::load_from_bytes(bytes).expect("ordinary embedded native document");
                        native_layers(&embedded.layers, depth + 1, out);
                    }
                } else {
                    panic!("effect source must not depend on an external resource");
                }
            }
            _ => {}
        }
    }
}
fn native_filters(layers: &[Layer]) -> Vec<SmartFilter> {
    layers.iter().filter_map(|l| if let LayerContent::Smart(s) = &l.content { Some(s.smart_filters.iter()) } else { None }).flatten().cloned().collect()
}
#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_effect_owner_parameters_scope_masks_and_disabled_stages_are_native_data() {
    let mut sibling = None;
    for (name, id, key, expected) in [
        ("brightness-contrast", "image.adjustments.brightnessContrast", "brightness", 25.),
        ("exposure", "image.adjustments.exposure", "exposure", 0.75),
        ("levels", "image.adjustments.levels", "", 0.),
        ("gaussian-anisotropic", "filter.blur.gaussianBlur", "radius", ((4. + 0.5625f64) / 2.).sqrt()),
    ] {
        let doc = open(&format!("effect-{name}-owner"));
        let plain = raster(layer(&doc, "Unaffected sibling"));
        if let Some(expected) = &sibling {
            assert!(plain == expected, "effect altered unaffected sibling pixels");
        } else {
            sibling = Some(plain.clone());
        }
        let mut stages = Vec::new();
        native_layers(&doc.layers, 0, &mut stages);
        let native = native_filters(&stages);
        let filter = native.iter().find(|f| f.command == id).unwrap_or_else(|| panic!("missing native {id}"));
        if !key.is_empty() {
            near(filter.params[key].as_f64().expect("native numeric parameter"), expected, 0.0001);
        }
        if name == "brightness-contrast" {
            near(filter.params["contrast"].as_f64().unwrap(), -30., 0.0001);
            assert_eq!(filter.params["legacy"], false);
        }
        if name == "levels" {
            for (key, value) in [("inBlack", 25.5), ("inWhite", 229.5), ("gamma", 1.4), ("outBlack", 12.75), ("outWhite", 242.25)] {
                near(filter.params["gray"][key].as_f64().unwrap(), value, 0.0001);
            }
        }
        if name == "exposure" {
            near(filter.params["offset"].as_f64().unwrap(), -0.03 * 0.75f64.exp2(), 0.0001);
            assert_eq!(filter.params["gamma"], 1.0);
        }
        assert_eq!(native.len(), 1, "one ordinary native filter for this single-stage source");
        near(f64::from(filter.opacity), 0.65, 0.001);
        assert!(filter.visible);
        let stage = stages
            .iter()
            .find_map(|layer| if let LayerContent::Smart(s) = &layer.content { s.smart_filters.iter().any(|f| f.command == id).then_some(s) } else { None })
            .expect("native owner stage");
        let mask = stage.filter_mask.as_ref().expect("active stock owner mask");
        assert!(mask.enabled);
        assert!(
            mask.surface.read_region(Rect::new(0, 0, 19, 13)).iter().all(|v| *v == 1.),
            "full-white source mask must admit the effect throughout the owner"
        );
        let LayerContent::Smart(owner) = &layer(&doc, "Effect owner").content else { panic!("native editable effect owner") };
        assert_eq!(owner.transform, Affine::translate(7., 5.));
        roundtrip(&doc);
    }
    for stem in ["effect-stack-common-mask", "effect-stack-distinct-masks-disabled"] {
        let doc = open(stem);
        let mut stages = Vec::new();
        native_layers(&doc.layers, 0, &mut stages);
        let native = native_filters(&stages);
        let active: Vec<_> =
            native.iter().filter(|f| ["image.adjustments.exposure", "image.adjustments.brightnessContrast"].contains(&f.command.as_str())).collect();
        // The last applied stage encloses the earlier input document: outer-to-inner traversal.
        assert_eq!(active.iter().map(|f| f.command.as_str()).collect::<Vec<_>>(), ["image.adjustments.exposure", "image.adjustments.brightnessContrast"]);
        let distinct = stem.contains("distinct");
        for (id, opacity, visible, rectangle) in [
            ("image.adjustments.brightnessContrast", 0.5, true, Rect::new(0, 0, 8, 6)),
            ("image.adjustments.exposure", 0.8, !distinct, if distinct { Rect::new(5, 3, 13, 9) } else { Rect::new(0, 0, 8, 6) }),
        ] {
            let smart = stages
                .iter()
                .find_map(|layer| if let LayerContent::Smart(s) = &layer.content { s.smart_filters.iter().any(|f| f.command == id).then_some(s) } else { None })
                .expect("native masked stage");
            assert_eq!(smart.smart_filters.len(), 1, "each source tone stage applies once");
            let filter = &smart.smart_filters[0];
            near(f64::from(filter.opacity), opacity, 0.001);
            assert_eq!(filter.visible, visible, "disabled exposure stays disabled");
            let mask = smart.filter_mask.as_ref().expect("source selection becomes native filter mask");
            assert!(mask.enabled);
            let coverage = mask.surface.read_region(Rect::new(0, 0, 19, 13));
            for y in 0..13 {
                for x in 0..19 {
                    let expected = if x >= rectangle.x0 && x < rectangle.x1 && y >= rectangle.y0 && y < rectangle.y1 { 1. } else { 0. };
                    assert_eq!(coverage[(y * 19 + x) as usize], expected, "{stem} {id} owner-local mask({x},{y})");
                }
            }
        }
        roundtrip(&doc);
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn saved_active_per_channel_curve_anchors_and_native_alpha_table_survive_native_save() {
    use photocraft_doc::adjust::{CurvePoint, ToneSpace};
    let doc = open("effect-curves-per-channel-owner");
    let mut stages = Vec::new();
    native_layers(&doc.layers, 0, &mut stages);
    let stage = stages
        .iter()
        .find_map(|l| if let LayerContent::Smart(s) = &l.content { (s.smart_filters.len() == 4).then_some(s) } else { None })
        .expect("ordinary four-filter native Curves stage");
    assert_eq!(
        stage.smart_filters.iter().map(|f| f.command.as_str()).collect::<Vec<_>>(),
        ["image.adjustments.curves", "image.adjustments.curves", "filter.other.alphaCurve", "image.adjustments.curves"]
    );
    for (index, key, expected) in [
        (0, "red", [[0., 12.75], [127.5, 89.25], [255., 242.25]]),
        (1, "blue", [[0., 0.], [165.75, 216.75], [255., 255.]]),
        (3, "points", [[0., 0.], [89.25, 127.5], [255., 255.]]),
    ] {
        let points = stage.smart_filters[index].params[key].as_array().expect("native editable control points");
        assert_eq!(points.len(), 3);
        for (point, expected) in points.iter().zip(expected) {
            near(point[0].as_f64().unwrap(), expected[0], 0.0001);
            near(point[1].as_f64().unwrap(), expected[1], 0.0001);
        }
    }
    // Independently construct the ordinary native alpha-curve counterpart from original
    // authoring anchors; source spline samples are not an expected native-render oracle.
    let alpha = photocraft_doc::Adjustment::Curves {
        master: vec![CurvePoint { input: 0., output: 0. }, CurvePoint { input: 0.5, output: 0.65 }, CurvePoint { input: 1., output: 1. }],
        per_channel: std::array::from_fn(|_| Vec::new()),
        space: ToneSpace::Rgb,
        black: Vec::new(),
        pencil: false,
    };
    let mut expected =
        photocraft_compose::Buffer { rect: Rect::new(0, 0, 256, 1), px: (0..256).map(|x| [x as f32 / 255., x as f32 / 255., x as f32 / 255., 1.]).collect() };
    photocraft_compose::adjust::apply_in(&alpha, &mut expected, photocraft_compose::adjust::Transfer::Gamma(1.), Some(SampleType::F32), ColorMode::Grayscale);
    let actual = stage.smart_filters[2].params["values"].as_array().expect("ordinary native alpha table");
    assert_eq!(actual.len(), 256);
    for (actual, expected) in actual.iter().zip(&expected.px) {
        near(actual.as_f64().unwrap(), f64::from(expected[0]), 0.000001);
    }
    assert!(stage.smart_filters.iter().all(|f| f.opacity == 1. && f.visible), "compound source envelope applied once, not per filter");
    let mask = stage.filter_mask.as_ref().expect("native compound-envelope coverage");
    assert!(mask.enabled);
    for value in mask.surface.read_region(Rect::new(0, 0, 19, 13)) {
        near(f64::from(value), 0.65, 0.000001);
    }
    let sibling = raster(layer(&doc, "Unaffected sibling")).read_region(Rect::new(14, 11, 15, 12));
    for (c, value) in sibling.iter().take(3).enumerate() {
        near(f64::from(*value) * 255., (((22 * 31 + 11 * 17 + c * 101 + 7) * 13) % 256) as f64, 0.51);
    }
    roundtrip(&doc);
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn historical_v19_font_objects_named_cubic_paths_and_precision_are_native_and_persist() {
    for (suffix, depth) in [("u8", SampleType::U8), ("float", SampleType::F32)] {
        let stem = format!("historical-2-99-18-text-path-{suffix}");
        assert_eq!(&fixture(&format!("{stem}.xcf"))[..14], b"gimp xcf v019\0");
        let doc = open(&stem);
        assert_eq!((doc.size.width, doc.size.height, doc.mode, doc.depth), (64, 32, ColorMode::Rgb, depth));
        let text =
            doc.layers.iter().find_map(|l| if let LayerContent::Text(t) = &l.content { Some(t) } else { None }).expect("ordinary editable historical text");
        assert_eq!(text.text, "Historical XCF");
        assert_eq!(text.transform, Affine::IDENTITY);
        // Native requested-font policy prefers the stored full face over its generic alias.
        assert_eq!(text.runs[0].style.font_family, "Cascadia Code Regular");
        assert_eq!(text.runs[0].style.postscript_name.as_deref(), Some("CascadiaCode-Roman"));
        near(f64::from(text.size_pt) * f64::from(doc.resolution_dpi) / 72., 12., 0.0001);
        assert!(text.cache.is_none());
        let path = doc.paths.iter().find(|p| p.name == "Original historical Bezier").expect("native v19 named path");
        assert_eq!(path.path.subpaths.len(), 1);
        let sub = &path.path.subpaths[0];
        assert!(sub.closed);
        assert_eq!(sub.knots.len(), 2);
        for (k, expected) in sub.knots.iter().zip([[(2., 4.), (4., 4.), (8., 2.)], [(40., 24.), (44., 24.), (48., 28.)]]) {
            assert_eq!(k.in_ctrl, Point::new(expected[0].0, expected[0].1));
            assert_eq!(k.anchor, Point::new(expected[1].0, expected[1].1));
            assert_eq!(k.out_ctrl, Point::new(expected[2].0, expected[2].1));
        }
        // This is GimpFont-object evidence and v18+ standalone path framing;
        // it is not an old string-only font or old image-property path witness.
        roundtrip(&doc);
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn historical_stock_resaved_bootstrap_graphs_preserve_original_pixels_as_native_contents() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    // Disclosed documented-wire seeds were loaded, saved to NEW XCF files, deleted,
    // reopened and merged by unmodified stock2.99.18. They were not GUI authored.
    // Stock PNG behavior is source observation, not expected PhotoCraft compositing.
    let expected = (0..6)
        .flat_map(|y| {
            (0..8).flat_map(move |x| {
                [
                    ((x * 23 + 17) % 256) as f32 / 255.,
                    ((y * 37 + 13) % 256) as f32 / 255.,
                    ((x * 11 + y * 19 + 29) % 256) as f32 / 255.,
                    (64 + ((x + y) % 4) * 48) as f32 / 255.,
                ]
            })
        })
        .collect::<Vec<_>>();
    for suffix in ["invert-linear", "original-over-inverted"] {
        let stem = format!("historical-2-99-18-graph-{suffix}");
        assert_eq!(&fixture(&format!("{stem}.xcf"))[..14], b"gimp xcf v020\0");
        let doc = open(&stem);
        assert_eq!((doc.size.width, doc.size.height, doc.mode, doc.depth), (8, 6, ColorMode::Rgb, SampleType::U8));
        let mut stages = Vec::new();
        native_layers(&doc.layers, 0, &mut stages);
        let filters = native_filters(&stages);
        assert!(!filters.is_empty());
        assert!(filters.iter().all(|f| f.command == "image.adjustments.invert"));
        assert!(
            stages.iter().any(|layer| if let LayerContent::Raster(surface) = &layer.content {
                surface.format().mode == ColorMode::Rgb && surface.channels() == 4 && surface.read_region(Rect::new(0, 0, 8, 6)) == expected
            } else {
                false
            }),
            "original procedural owner pixels must remain ordinary native editable contents"
        );
        let masks = stages
            .iter()
            .flat_map(|layer| layer.mask.iter().chain(if let LayerContent::Smart(s) = &layer.content { s.filter_mask.as_ref() } else { None }))
            .collect::<Vec<_>>();
        // A linear native filter uses filter_mask; a constructed fork/join uses an
        // ordinary enclosing layer mask. Both apply the one source envelope once.
        assert_eq!(masks.len(), 1, "one whole-graph coverage envelope");
        let mask = masks[0];
        assert!(mask.enabled);
        assert!(mask.surface.read_region(Rect::new(0, 0, 8, 6)).iter().all(|v| *v == 1.), "stock full-white graph mask remains native coverage");
        let Some(ImportConversion::Report(report)) = &doc.metadata.import_conversion else { panic!("typed source conversion report") };
        assert!(report.entries.iter().any(|e| e.feature_code == "xcf.layer_effect"
            && e.source_description.contains("gegl:gegl")
            && e.disposition == ConversionDisposition::NativeApproximation));
        roundtrip(&doc);
    }
}

#[test]
#[ignore = "Requires expanded upstream XCF corpus pin (T081)"]
fn genuine_v0_string_font_and_legacy_property_path_are_editable_native_data() {
    // Independently authored, saved, deleted, reopened and queried in stock2.10.32.
    // This producer stores the string font and PROP_PATHS23, unlike v019 objects/list framing.
    let stem = "historical-2-10-32-legacy-text-path";
    assert_eq!(&fixture(&format!("{stem}.xcf"))[..14], b"gimp xcf file\0");
    let doc = open(stem);
    assert_eq!((doc.size.width, doc.size.height, doc.mode, doc.depth), (64, 32, ColorMode::Rgb, SampleType::U8));
    let text = doc
        .layers
        .iter()
        .find_map(|layer| if let LayerContent::Text(text) = &layer.content { Some(text) } else { None })
        .expect("editable historical string-font text");
    assert_eq!(text.text, "Legacy XCF");
    assert_eq!(text.transform, Affine::IDENTITY);
    assert_eq!(text.runs[0].style.font_family, "Sans-serif");
    near(f64::from(text.size_pt) * f64::from(doc.resolution_dpi) / 72., 12., 0.0001);
    assert!(text.cache.is_none());
    let path = doc.paths.iter().find(|path| path.name == "Original legacy Bezier").expect("named legacy property path");
    assert_eq!(path.path.subpaths.len(), 1);
    let sub = &path.path.subpaths[0];
    assert!(sub.closed);
    assert_eq!(sub.knots.len(), 2);
    for (knot, expected) in sub.knots.iter().zip([[(2., 4.), (4., 4.), (8., 2.)], [(40., 24.), (44., 24.), (48., 28.)]]) {
        assert_eq!(knot.in_ctrl, Point::new(expected[0].0, expected[0].1));
        assert_eq!(knot.anchor, Point::new(expected[1].0, expected[1].1));
        assert_eq!(knot.out_ctrl, Point::new(expected[2].0, expected[2].1));
    }
    roundtrip(&doc);
}
