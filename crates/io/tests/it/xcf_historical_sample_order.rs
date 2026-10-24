//! Independent documented-wire sample-order tests, not historical producer witnesses.
//! Genuine stock-writer qualification lives separately in xcf_historical_records.
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent};
use photocraft_io::{HistoricalXcfSampleOrder as Order, ImportOptions};
use photocraft_raster::Interrupt;

struct Wire {
    bytes: Vec<u8>,
    wide: bool,
}
impl Wire {
    fn word(&mut self, value: u32) {
        self.bytes.extend(value.to_be_bytes());
    }
    fn pointer(&mut self, value: usize) {
        if self.wide {
            self.bytes.extend((value as u64).to_be_bytes());
        } else {
            self.word(value as u32);
        }
    }
    fn slot(&mut self) -> usize {
        let at = self.bytes.len();
        self.pointer(0);
        at
    }
    fn set(&mut self, slot: usize) {
        let at = self.bytes.len();
        if self.wide {
            self.bytes[slot..slot + 8].copy_from_slice(&(at as u64).to_be_bytes());
        } else {
            self.bytes[slot..slot + 4].copy_from_slice(&(at as u32).to_be_bytes());
        }
    }
    fn string(&mut self, value: &str) {
        self.word(value.len() as u32 + 1);
        self.bytes.extend(value.as_bytes());
        self.bytes.push(0);
    }
    fn property(&mut self, kind: u32, bytes: &[u8]) {
        self.word(kind);
        self.word(bytes.len() as u32);
        self.bytes.extend(bytes);
    }
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    fn pixels(&mut self, sample: Sample, order: Order, channels: usize, phase: usize, compression: u8) {
        self.word(65);
        self.word(2);
        self.word((sample.width() * channels) as u32);
        let level = self.slot();
        self.pointer(0);
        self.set(level);
        self.word(65);
        self.word(2);
        let tiles = [self.slot(), self.slot()];
        self.pointer(0);
        for (column, slot) in tiles.into_iter().enumerate() {
            self.set(slot);
            let width = if column == 0 { 64 } else { 1 };
            let mut raw = Vec::new();
            for y in 0..2 {
                for dx in 0..width {
                    for component in 0..channels {
                        raw.extend(sample.bytes((column * 64 + dx + y + component + phase) % 4, order));
                    }
                }
            }
            match compression {
                0 => self.bytes.extend(raw),
                1 => {
                    let bpp = sample.width() * channels;
                    // Independent literal-only planar RLE, split into <=127 runs.
                    for byte in 0..bpp {
                        let plane = raw.iter().skip(byte).step_by(bpp).copied().collect::<Vec<_>>();
                        for run in plane.chunks(127) {
                            self.bytes.push((256 - run.len()) as u8);
                            self.bytes.extend(run);
                        }
                    }
                }
                2 => {
                    use std::io::Write;
                    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(&raw).unwrap();
                    self.bytes.extend(encoder.finish().unwrap());
                }
                _ => panic!("test compression"),
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Sample {
    U16,
    U32,
    Half,
    Float,
    Double,
}
impl Sample {
    fn precision(self) -> u32 {
        match self {
            Self::U16 => 250,
            Self::U32 => 350,
            Self::Half => 550,
            Self::Float => 650,
            Self::Double => 750,
        }
    }
    fn width(self) -> usize {
        match self {
            Self::U16 | Self::Half => 2,
            Self::U32 | Self::Float => 4,
            Self::Double => 8,
        }
    }
    fn native_depth(self) -> SampleType {
        if matches!(self, Self::U16) { SampleType::U16 } else { SampleType::F32 }
    }
    fn expected(self, index: usize) -> f32 {
        match self {
            Self::U16 => f32::from([0x1234u16, 0x4567, 0x89ab, 0xcdef][index]) / 65535.,
            Self::U32 => (f64::from([0x12345678u32, 0x456789ab, 0x89abcdef, 0xcdef0123][index]) / f64::from(u32::MAX)) as f32,
            Self::Half | Self::Float | Self::Double => [0.125, 0.25, 0.5, 0.75][index],
        }
    }
    fn bytes(self, index: usize, order: Order) -> Vec<u8> {
        let big = match self {
            Self::U16 => [0x1234u16, 0x4567, 0x89ab, 0xcdef][index].to_be_bytes().to_vec(),
            Self::U32 => [0x12345678u32, 0x456789ab, 0x89abcdef, 0xcdef0123][index].to_be_bytes().to_vec(),
            Self::Half => [0x3000u16, 0x3400, 0x3800, 0x3a00][index].to_be_bytes().to_vec(),
            Self::Float => self.expected(index).to_be_bytes().to_vec(),
            Self::Double => f64::from(self.expected(index)).to_be_bytes().to_vec(),
        };
        if order == Order::LittleEndian { big.into_iter().rev().collect() } else { big }
    }
}
fn image(version: u32, sample: Sample, order: Order, gray: bool, compression: u8) -> Vec<u8> {
    let mut w = Wire { bytes: format!("gimp xcf v{version:03}\0").into_bytes(), wide: version >= 11 };
    for value in [65, 2, u32::from(gray), sample.precision()] {
        w.word(value);
    }
    w.property(17, &[compression]);
    w.end();
    let layer = w.slot();
    w.pointer(0);
    let channel = w.slot();
    let selection = w.slot();
    w.pointer(0);
    w.set(layer);
    for value in [65, 2, if gray { 3 } else { 1 }] {
        w.word(value);
    }
    w.string("Original historical samples");
    w.property(8, &1u32.to_be_bytes());
    w.property(11, &1u32.to_be_bytes());
    w.property(15, &[(-2i32).to_be_bytes(), 1i32.to_be_bytes()].concat());
    w.end();
    let raster = w.slot();
    let mask = w.slot();
    w.set(raster);
    w.pixels(sample, order, if gray { 2 } else { 4 }, 0, compression);
    w.set(mask);
    w.word(65);
    w.word(2);
    w.string("Original source mask");
    w.end();
    let raster = w.slot();
    w.set(raster);
    w.pixels(sample, order, 1, 1, compression);
    w.set(channel);
    w.word(65);
    w.word(2);
    w.string("Original source channel");
    w.end();
    let raster = w.slot();
    w.set(raster);
    w.pixels(sample, order, 1, 2, compression);
    w.set(selection);
    w.word(65);
    w.word(2);
    w.string("Original source selection");
    w.property(4, &[]);
    w.end();
    let raster = w.slot();
    w.set(raster);
    w.pixels(sample, order, 1, 3, compression);
    w.bytes
}
fn opts(order: Order) -> ImportOptions {
    ImportOptions { historical_xcf_sample_order: Some(order), ..Default::default() }
}
fn open(bytes: &[u8], order: Order) -> Document {
    photocraft_io::import_with_options("original-wire.xcf", bytes, &opts(order)).unwrap().document
}
fn assert_pixels(document: &Document, sample: Sample, gray: bool) {
    assert_eq!(document.mode, if gray { ColorMode::Grayscale } else { ColorMode::Rgb });
    assert_eq!(document.depth, sample.native_depth());
    let layer = &document.layers[0];
    let LayerContent::Raster(surface) = &layer.content else { panic!("native raster") };
    let mask = &layer.mask.as_ref().unwrap().surface;
    let channel = &document.channels[0].surface;
    for (x, y) in [(0, 0), (1, 0), (63, 0), (64, 0), (0, 1), (64, 1)] {
        let pixel = surface.pixel(x - 2, y + 1);
        for (component, actual) in pixel.iter().enumerate() {
            let expected = sample.expected((x as usize + y as usize + component) % 4);
            assert!((actual - expected).abs() <= 1e-7, "{sample:?}/{gray}:({x},{y}) component{component} {actual}!={expected}");
        }
        assert!((mask.pixel(x - 2, y + 1)[0] - sample.expected((x as usize + y as usize + 1) % 4)).abs() <= 1e-7);
        assert!((channel.pixel(x, y)[0] - sample.expected((x as usize + y as usize + 2) % 4)).abs() <= 1e-7);
        assert!((document.selection.as_ref().unwrap().pixel(x, y)[0] - sample.expected((x as usize + y as usize + 3) % 4)).abs() <= 1e-7);
    }
}
#[test]
fn explicit_sample_order_covers_all_native_widths_roles_compressions_and_tile_edges() {
    for version in [7, 8, 9, 10, 11] {
        for sample in [Sample::U16, Sample::U32, Sample::Half, Sample::Float, Sample::Double] {
            for gray in [false, true] {
                for compression in [0, 1, 2] {
                    let little = open(&image(version, sample, Order::LittleEndian, gray, compression), Order::LittleEndian);
                    let big = open(&image(version, sample, Order::BigEndian, gray, compression), Order::BigEndian);
                    assert_pixels(&little, sample, gray);
                    assert_pixels(&big, sample, gray);
                    assert!(little.layers[0].surface() == big.layers[0].surface(), "explicit orders changed native raster");
                    assert!(little.layers[0].mask == big.layers[0].mask, "explicit orders changed native mask");
                    assert!(little.channels == big.channels, "explicit orders changed native channels");
                    assert!(little.selection == big.selection, "explicit orders changed native selection");
                }
            }
        }
    }
}

// One uncompressed LZMA2 block, CRC32 check and index/footer, independently
// framed as in the existing native transport test (no encoder feature needed).
fn xz(bytes: &[u8]) -> Vec<u8> {
    fn vli(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            out.push(if value == 0 { byte } else { byte | 128 });
            if value == 0 {
                break;
            }
        }
    }
    assert!(!bytes.is_empty() && bytes.len() <= 65536);
    let mut out = b"\xfd7zXZ\0\0\x01".to_vec();
    out.extend(crc32fast::hash(&[0, 1]).to_le_bytes());
    let mut header = vec![2, 0, 0x21, 1, 0, 0, 0, 0];
    header.extend(crc32fast::hash(&header).to_le_bytes());
    out.extend(&header);
    let mut data = vec![1];
    data.extend(((bytes.len() - 1) as u16).to_be_bytes());
    data.extend(bytes);
    data.push(0);
    out.extend(&data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    out.extend(crc32fast::hash(bytes).to_le_bytes());
    let mut index = vec![0, 1];
    vli(header.len() + data.len() + 4, &mut index);
    vli(bytes.len(), &mut index);
    while !index.len().is_multiple_of(4) {
        index.push(0);
    }
    index.extend(crc32fast::hash(&index).to_le_bytes());
    out.extend(&index);
    let mut footer = ((index.len() / 4 - 1) as u32).to_le_bytes().to_vec();
    footer.extend([0, 1]);
    out.extend(crc32fast::hash(&footer).to_le_bytes());
    out.extend(footer);
    out.extend(b"YZ");
    out
}

#[test]
fn wrapped_historical_sources_require_choice_and_decode_to_independent_native_samples() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::{AlphaChannel, Layer, LayerMask};
    use photocraft_geom::{Rect, Size};
    use photocraft_raster::Surface;
    use std::io::Write;

    let bytes = image(9, Sample::Float, Order::LittleEndian, false, 1);
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&bytes).unwrap();
    let wrappers = [
        ("historical.xcf.gz", gzip.finish().unwrap()),
        ("historical.xcf.bz2", crabz2::compress(&bytes, crabz2::Level::default())),
        ("historical.xcf.xz", xz(&bytes)),
    ];
    // Author the complete native counterpart directly from specified sample
    // values, rather than obtaining expected pixels from another XCF import.
    let mut expected = Document::new("Independent native sample planes", Size::new(65, 2), ColorMode::Rgb, SampleType::F32);
    let mut pixels = Vec::new();
    let mut mask_values = Vec::new();
    let mut channel_values = Vec::new();
    let mut selection_values = Vec::new();
    for y in 0..2 {
        for x in 0..65 {
            for component in 0..4 {
                pixels.push(Sample::Float.expected((x + y + component) % 4));
            }
            mask_values.push(Sample::Float.expected((x + y + 1) % 4));
            channel_values.push(Sample::Float.expected((x + y + 2) % 4));
            selection_values.push(Sample::Float.expected((x + y + 3) % 4));
        }
    }
    let layer_bounds = Rect::new(-2, 1, 63, 3);
    let mut raster = Surface::new(PixelFormat::RGBA32F);
    raster.write_region(layer_bounds, &pixels);
    let mut layer = Layer::new("Native samples", LayerContent::Raster(raster));
    let scalar = PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false);
    let mut mask = Surface::new(scalar);
    mask.write_region(layer_bounds, &mask_values);
    layer.mask = Some(LayerMask { surface: mask, ..LayerMask::reveal_all() });
    expected.layers.push(layer);
    let mut channel = Surface::new(scalar);
    channel.write_region(expected.bounds(), &channel_values);
    expected.channels.push(AlphaChannel::new("Native channel", channel));
    let mut selection = Surface::new(scalar);
    selection.write_region(expected.bounds(), &selection_values);
    expected.selection = Some(selection);
    for (name, wrapped) in wrappers {
        let request = photocraft_io::historical_xcf_sample_order_request(name, &wrapped).unwrap().unwrap();
        assert_eq!((request.version, request.bits_per_sample), (9, 32), "{name}");
        let Err(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(required)) = photocraft_io::import(name, &wrapped) else {
            panic!("{name}: missing typed default refusal")
        };
        assert_eq!(required, request);
        let result = photocraft_io::import_with_options(name, &wrapped, &opts(Order::LittleEndian)).unwrap();
        assert!(result.source_read_only && !result.preview_only, "{name}");
        assert_pixels(&result.document, Sample::Float, false);
        assert!(result.document.layers[0].surface() == expected.layers[0].surface(), "{name}: complete native raster");
        assert!(result.document.layers[0].mask.as_ref().unwrap().surface == expected.layers[0].mask.as_ref().unwrap().surface, "{name}: complete native mask");
        assert!(result.document.channels[0].surface == expected.channels[0].surface, "{name}: complete native channel");
        assert!(result.document.selection == expected.selection, "{name}: complete native selection");
        let saved = photocraft_format::save_to_bytes(&result.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert!(reopened.layers[0].surface() == expected.layers[0].surface(), "{name}: native persistence");
        assert!(reopened.metadata.import_conversion == result.document.metadata.import_conversion, "{name}: choice report persistence");
    }
}
#[test]
fn default_requires_explicit_order_before_decoding_and_cancellation_still_wins() {
    for version in [7, 8, 9, 10, 11] {
        for sample in [Sample::U16, Sample::U32, Sample::Half, Sample::Float, Sample::Double] {
            let bytes = image(version, sample, Order::LittleEndian, false, 0);
            let request = photocraft_io::historical_xcf_sample_order_request("wire.xcf", &bytes).unwrap().unwrap();
            assert_eq!((request.version, request.bits_per_sample), (version, (sample.width() * 8) as u32));
            let result = photocraft_io::import("wire.xcf", &bytes);
            assert!(matches!(result, Err(photocraft_io::IoError::HistoricalXcfSampleOrderRequired(_))));
            let cancelled = || true;
            let ctl = Interrupt::cancel_only(&cancelled);
            assert!(matches!(photocraft_io::historical_xcf_sample_order_request_with("wire.xcf", &bytes, &ctl), Err(photocraft_io::IoError::Cancelled)));
            assert!(matches!(
                photocraft_io::import_with_options_and_interrupt("wire.xcf", &bytes, &opts(Order::LittleEndian), &ctl),
                Err(photocraft_io::IoError::Cancelled)
            ));
        }
    }
    // Choice is irrelevant for canonical modern sample framing, which remains BE.
    for version in [12, 17] {
        let bytes = image(version, Sample::Float, Order::BigEndian, false, 2);
        assert!(photocraft_io::historical_xcf_sample_order_request("modern.xcf", &bytes).unwrap().is_none());
        let default = photocraft_io::import("modern.xcf", &bytes).unwrap().document;
        let choice = open(&bytes, Order::LittleEndian);
        assert_pixels(&default, Sample::Float, false);
        assert_pixels(&choice, Sample::Float, false);
        // Independent imports allocate fresh LayerIds. Compare actual native
        // data and styles rather than unrelated import identities or filenames.
        let before = &default.layers[0];
        let after = &choice.layers[0];
        assert!(before.surface() == after.surface(), "modern sample-order override changed native pixels");
        assert!(before.mask == after.mask, "modern sample-order override changed native mask");
        assert!(default.channels == choice.channels, "modern sample-order override changed native channels");
        assert!(default.selection == choice.selection, "modern sample-order override changed native selection");
        assert!(default.icc_profile == choice.icc_profile, "modern sample-order override changed native profile");
        assert_eq!((before.blend, before.opacity, before.fill_opacity, before.visible), (after.blend, after.opacity, after.fill_opacity, after.visible));
        assert!(before.locks == after.locks && before.effects == after.effects, "modern sample-order override changed native style");
    }
}
