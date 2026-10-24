//! Public import/native-save boundaries, independently authored XCF wire data.
//! These tests need no private corpus, source runtime, profiles or assets on disk.
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, import_conversion::ImportConversion};
use photocraft_geom::Rect;
use photocraft_io::import;

#[derive(Default)]
struct Wire(Vec<u8>);
impl Wire {
    fn word(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }
    fn pointer(&mut self, value: usize) {
        self.0.extend_from_slice(&(value as u64).to_be_bytes());
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.pointer(0);
        at
    }
    fn set(&mut self, slot: usize) {
        let address = self.0.len() as u64;
        self.0[slot..slot + 8].copy_from_slice(&address.to_be_bytes());
    }
    fn string(&mut self, value: &str) {
        self.word(value.len() as u32 + 1);
        self.0.extend_from_slice(value.as_bytes());
        self.0.push(0);
    }
    fn property(&mut self, kind: u32, data: &[u8]) {
        self.word(kind);
        self.word(data.len() as u32);
        self.0.extend_from_slice(data);
    }
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    fn pixels(&mut self, width: u32, height: u32, bpp: u32, data: &[u8]) {
        assert!(width <= 64 && height <= 64);
        assert_eq!(data.len(), (width * height * bpp) as usize);
        self.word(width);
        self.word(height);
        self.word(bpp);
        let level = self.slot();
        self.pointer(0);
        self.set(level);
        self.word(width);
        self.word(height);
        let tile = self.slot();
        self.pointer(0);
        self.set(tile);
        self.0.extend_from_slice(data);
    }
}
struct Drawable {
    name: &'static str,
    width: u32,
    height: u32,
    kind: u32,
    offset: (i32, i32),
    data: Vec<u8>,
    mask: Option<Vec<u8>>,
    inert_effect: bool,
}
fn drawable(name: &'static str, width: u32, height: u32, kind: u32, data: Vec<u8>) -> Drawable {
    Drawable { name, width, height, kind, offset: (0, 0), data, mask: None, inert_effect: false }
}
struct Image {
    width: u32,
    height: u32,
    base: u32,
    precision: u32,
    bytes: u32,
    layers: Vec<Drawable>,
    profile: Option<Vec<u8>>,
    palette: Vec<[u8; 3]>,
    channel: Option<(Vec<u8>, bool)>,
}
impl Image {
    fn new(base: u32, precision: u32, bytes: u32) -> Self {
        Self { width: 3, height: 2, base, precision, bytes, layers: Vec::new(), profile: None, palette: Vec::new(), channel: None }
    }
    fn bytes(&self) -> Vec<u8> {
        let mut w = Wire(b"gimp xcf v026\0".to_vec());
        for value in [self.width, self.height, self.base, self.precision] {
            w.word(value);
        }
        w.property(17, &[0]); // Independently framed uncompressed tiles.
        if let Some(profile) = &self.profile {
            let mut parasite = Wire::default();
            parasite.string("icc-profile");
            parasite.word(0);
            parasite.word(profile.len() as u32);
            parasite.0.extend_from_slice(profile);
            w.property(21, &parasite.0);
        }
        if !self.palette.is_empty() {
            let mut colors = Wire::default();
            colors.word(self.palette.len() as u32);
            for color in &self.palette {
                colors.0.extend_from_slice(color);
            }
            w.property(1, &colors.0);
        }
        w.end();
        let layers: Vec<_> = self.layers.iter().map(|_| w.slot()).collect();
        w.pointer(0);
        let channel = self.channel.as_ref().map(|_| w.slot());
        w.pointer(0);
        w.pointer(0); // channels, then paths.
        for (source, slot) in self.layers.iter().zip(layers) {
            w.set(slot);
            for value in [source.width, source.height, source.kind] {
                w.word(value);
            }
            w.string(source.name);
            w.property(7, &28u32.to_be_bytes());
            w.property(8, &1u32.to_be_bytes());
            w.property(33, &1f32.to_be_bytes());
            let mut offset = source.offset.0.to_be_bytes().to_vec();
            offset.extend_from_slice(&source.offset.1.to_be_bytes());
            w.property(15, &offset);
            if source.mask.is_some() {
                w.property(11, &1u32.to_be_bytes());
            }
            w.end();
            let raster = w.slot();
            let mask = w.slot();
            let effect = source.inert_effect.then(|| w.slot());
            w.pointer(0);
            w.set(raster);
            let channels = match source.kind {
                0 => 3,
                1 => 4,
                2 | 4 => 1,
                3 | 5 => 2,
                _ => panic!("test kind"),
            };
            w.pixels(source.width, source.height, channels * self.bytes, &source.data);
            if let Some(data) = &source.mask {
                w.set(mask);
                w.word(source.width);
                w.word(source.height);
                w.string("Independent mask");
                w.end();
                let raster = w.slot();
                w.pointer(0);
                w.set(raster);
                w.pixels(source.width, source.height, self.bytes, data);
            }
            if let Some(slot) = effect {
                w.set(slot);
                for value in ["Retained source configuration", "", "example:inert-source-configuration", ""] {
                    w.string(value);
                }
                w.end();
                w.pointer(0);
            }
        }
        if let (Some(slot), Some((data, selection))) = (channel, &self.channel) {
            w.set(slot);
            w.word(self.width);
            w.word(self.height);
            w.string("Independent coverage");
            if *selection {
                w.property(4, &[]);
            }
            w.end();
            let raster = w.slot();
            w.pointer(0);
            w.set(raster);
            w.pixels(self.width, self.height, self.bytes, data);
        }
        w.0
    }
}
fn doubles(values: impl IntoIterator<Item = f64>) -> Vec<u8> {
    values.into_iter().flat_map(f64::to_be_bytes).collect()
}
fn words(values: impl IntoIterator<Item = u32>) -> Vec<u8> {
    values.into_iter().flat_map(u32::to_be_bytes).collect()
}
fn layer<'a>(document: &'a Document, name: &str) -> &'a Layer {
    document.layers.iter().find(|layer| layer.name == name).unwrap()
}
fn samples(layer: &Layer, rect: Rect) -> Vec<f32> {
    let LayerContent::Raster(surface) = &layer.content else { panic!("editable native raster") };
    surface.read_region(rect)
}
fn reopen(document: &Document) -> Document {
    photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(document, &Default::default()).unwrap()).unwrap()
}
fn assert_native_equal(before: &Document, after: &Document) {
    assert_eq!((after.size, after.mode, after.depth), (before.size, before.mode, before.depth));
    assert_eq!(after.icc_profile, before.icc_profile);
    assert_eq!(after.metadata.import_conversion, before.metadata.import_conversion);
    assert_eq!(after.channels, before.channels);
    for original in &before.layers {
        let restored = layer(after, &original.name);
        let bounds = original.surface().unwrap().tile_bounds();
        assert_eq!(restored.surface(), original.surface());
        assert_eq!(restored.mask, original.mask);
        assert_eq!(
            samples(restored, bounds).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            samples(original, bounds).iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn rgb_double_offcanvas_layers_masks_and_channel_keep_native_bits_profile_and_owner_notices() {
    let mut source = Image::new(0, 750, 8);
    source.profile = Some(photocraft_cms::Builtin::DisplayP3.profile().to_bytes().as_ref().clone());
    let mut front = drawable("Double foreground", 2, 1, 1, doubles([1. + 2f64.powi(-24), -2., 2f64.powi(-150), 0.25, 1. + 3. * 2f64.powi(-24), 4., -0., 0.75]));
    front.offset = (-1, 1);
    front.mask = Some(doubles([2f64.powi(-150), 0.5]));
    front.inert_effect = true;
    let mut offcanvas = drawable("Outside canvas", 1, 1, 1, doubles([-0.5, 2., f64::from(f32::from_bits(1)), 1.]));
    offcanvas.offset = (4, -2);
    source.layers = vec![front, offcanvas];
    source.channel = Some((doubles([2f64.powi(-150), 0., 0.25, 0.5, 0.75, 1.]), false));
    let imported = import("independent-rgb-double.xcf", &source.bytes()).unwrap();
    let document = &imported.document;
    assert!(imported.source_read_only);
    assert_eq!((document.mode, document.depth), (ColorMode::Rgb, SampleType::F32));
    assert_eq!(document.icc_profile.as_deref(), source.profile.as_ref());
    assert_eq!(
        samples(layer(document, "Double foreground"), Rect::new(-1, 1, 1, 2)).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        [0x3f800000, 0xc0000000, 0, 0x3e800000, 0x3f800002, 0x40800000, 0x80000000, 0x3f400000]
    );
    assert_eq!(
        samples(layer(document, "Outside canvas"), Rect::new(4, -2, 5, -1)).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        [0xbf000000, 0x40000000, 1, 0x3f800000]
    );
    let Some(ImportConversion::Report(report)) = &document.metadata.import_conversion else { panic!("conversion report") };
    let underflows: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.precision_underflow").collect();
    assert_eq!(underflows.len(), 3);
    assert!(
        report.attachments.iter().any(|attachment| attachment
            .bytes
            .windows(b"example:inert-source-configuration".len())
            .any(|value| value == b"example:inert-source-configuration"))
    );
    assert_native_equal(document, &reopen(document));
}

#[test]
fn gray_u32_color_alpha_mask_and_channel_normalize_to_native_float_without_u16_reduction() {
    let mut source = Image::new(1, 350, 4);
    source.width = 2;
    source.height = 2;
    let mut raster = drawable("Gray integer", 2, 1, 3, words([1, 0x80000000, u32::MAX, 0x40000000]));
    raster.offset = (0, -1);
    raster.mask = Some(words([0x01000001, u32::MAX]));
    source.layers.push(raster);
    source.channel = Some((words([0, 1, u32::MAX - 1, u32::MAX]), false));
    let imported = import("independent-gray-u32.xcf", &source.bytes()).unwrap();
    let document = &imported.document;
    assert_eq!((document.mode, document.depth), (ColorMode::Grayscale, SampleType::F32));
    assert_eq!(
        samples(layer(document, "Gray integer"), Rect::new(0, -1, 2, 0)).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        [0x2f800000, 0x3f000000, 0x3f800000, 0x3e800000]
    );
    let mask = layer(document, "Gray integer").mask.as_ref().unwrap();
    assert_eq!(mask.surface.read_region(Rect::new(0, -1, 2, 0)).iter().map(|v| v.to_bits()).collect::<Vec<_>>(), [0x3b800001, 0x3f800000]);
    assert_eq!(
        document.channels[0].surface.read_region(Rect::new(0, 0, 2, 2)).iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        [0, 0x2f800000, 0x3f800000, 0x3f800000]
    );
    let profile = photocraft_cms::Profile::parse(document.icc_profile.as_ref().unwrap()).unwrap();
    assert_eq!(profile.color_space, photocraft_cms::ColorSpace::Gray);
    assert_native_equal(document, &reopen(document));
}

#[test]
fn indexed_palette_alpha_and_opaque_offcanvas_layer_become_native_rgb_data() {
    let mut source = Image::new(2, 150, 1);
    source.palette = vec![[250, 7, 21], [14, 239, 35], [26, 48, 227]];
    let mut rgba = drawable("Palette alpha", 3, 1, 5, vec![2, 255, 0, 128, 1, 0]);
    rgba.offset = (-1, 1);
    let mut opaque = drawable("Palette opaque", 1, 1, 4, vec![1]);
    opaque.offset = (3, -1);
    source.layers = vec![rgba, opaque];
    let imported = import("independent-indexed.xcf", &source.bytes()).unwrap();
    let document = &imported.document;
    assert_eq!((document.mode, document.depth), (ColorMode::Rgb, SampleType::U8));
    let expected: Vec<_> = [26u8, 48, 227, 255, 250, 7, 21, 128, 14, 239, 35, 0].into_iter().map(|v| f32::from(v) / 255.).collect();
    assert_eq!(samples(layer(document, "Palette alpha"), Rect::new(-1, 1, 2, 2)), expected);
    assert_eq!(samples(layer(document, "Palette opaque"), Rect::new(3, -1, 4, 0)), vec![14. / 255., 239. / 255., 35. / 255., 1.]);
    assert_native_equal(document, &reopen(document));
}

#[test]
fn invalid_later_offcanvas_color_cannot_succeed_as_a_partially_imported_document() {
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::from_bits(f64::from(f32::MAX).to_bits() + 1)] {
        let mut source = Image::new(0, 750, 8);
        source.layers.push(drawable("Valid first", 1, 1, 1, doubles([0.25, 0.5, 0.75, 1.])));
        let mut invalid_layer = drawable("Invalid outside", 1, 1, 1, doubles([invalid, 0., 0., 1.]));
        invalid_layer.offset = (-100, -100);
        source.layers.push(invalid_layer);
        let error = import("invalid-later-double.xcf", &source.bytes()).unwrap_err().to_string();
        assert!(error.contains("Invalid outside") && error.contains("color"), "{error}");
    }
}

#[test]
fn coverage_outside_storage_domain_is_rejected_before_narrowing_even_when_offcanvas() {
    for value in [-2f64.powi(-150), 1. + 2f64.powi(-24)] {
        for owner in ["alpha", "mask", "channel", "selection"] {
            let mut source = Image::new(0, 750, 8);
            let mut raster = drawable("Coverage owner", 1, 1, 1, doubles([2., -1., 0.5, 1.]));
            raster.offset = (-5, 3);
            match owner {
                "alpha" => raster.data = doubles([2., -1., 0.5, value]),
                "mask" => raster.mask = Some(doubles([value])),
                "channel" | "selection" => source.channel = Some((doubles([value, 0., 0., 0., 0., 0.]), owner == "selection")),
                _ => panic!("test owner"),
            };
            source.layers.push(raster);
            let error = import("invalid-stored-coverage.xcf", &source.bytes()).unwrap_err().to_string();
            assert!(error.contains("coverage") && error.contains("plane"), "{owner}: {error}");
        }
    }
}

#[test]
fn retained_source_configuration_stays_inert_when_native_pixels_are_edited_and_saved_again() {
    let mut source = Image::new(0, 750, 8);
    let mut raster = drawable("Native editing", 1, 1, 1, doubles([2., -1., 0.5, 1.]));
    raster.inert_effect = true;
    source.layers.push(raster);
    let imported = import("inert-source.xcf", &source.bytes()).unwrap();
    let mut native = reopen(&imported.document);
    let retained = native.metadata.import_conversion.clone();
    let LayerContent::Raster(surface) = &mut native.layers[0].content else { panic!("ordinary native raster") };
    surface.write_region(Rect::new(0, 0, 1, 1), &[8., -4., 1.25, 0.5]);
    let after = reopen(&native);
    assert_eq!(samples(&after.layers[0], Rect::new(0, 0, 1, 1)), [8., -4., 1.25, 0.5]);
    assert_eq!(after.metadata.import_conversion, retained);
    assert!(matches!(after.layers[0].content, LayerContent::Raster(_)));
}

#[cfg(feature = "corpus")]
#[test]
fn published_float_linear_import_retains_native_profile_depth_and_samples_on_native_reopen() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/gimp/float-linear.xcf");
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}; run cargo xtask corpus --gimp", path.display()));
    let imported = import("float-linear.xcf", &bytes).unwrap();
    let document = &imported.document;
    assert_eq!(document.depth, SampleType::F32);
    assert_native_equal(document, &reopen(document));
    let profile = photocraft_cms::Profile::parse(document.icc_profile.as_ref().unwrap()).unwrap();
    let target = photocraft_cms::Builtin::LinearSrgb.profile();
    let transform = photocraft_cms::Transform::new(&profile, target, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    let mut converted = [0.; 3];
    transform.convert_f32(&[0.125, 0.25, 0.5], 3, &mut converted, 3, false);
    for (actual, expected) in converted.into_iter().zip([0.125, 0.25, 0.5]) {
        assert!((actual - expected).abs() < 0.0001);
    }
}
