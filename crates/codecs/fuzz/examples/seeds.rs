//! Writes a starting corpus for this crate's fuzz targets: small valid files built in code (the
//! repository commits no binary seeds), so libFuzzer starts from real structure instead of
//! having to guess a header. Each seed must decode, or this fails: a seed the decoder rejects
//! only teaches the fuzzer the error path.
//!
//! `cargo run --example seeds -- [target ...]` (no target: all) writes `corpus/<target>/seed-*`.

use std::path::Path;

use photocraft_codecs::{ChannelLayout, DdsCompression, DdsOptions, DpxBits, EncodeOptions, Format, Image, decode_as, encode};

/// A 3×2 test image: distinct values per pixel and channel, so a seed exercises the sample paths.
fn image(layout: ChannelLayout, sixteen: bool) -> Image {
    let n = 3 * 2 * layout.channels();
    if sixteen {
        let v: Vec<u16> = (0..n).map(|i| (i as u16).wrapping_mul(9973)).collect();
        Image::from_u16(3, 2, layout, &v).expect("3x2 fits")
    } else {
        let v: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37)).collect();
        Image::from_u8(3, 2, layout, v).expect("3x2 fits")
    }
}

fn encoded(format: Format, layout: ChannelLayout, sixteen: bool, opts: &EncodeOptions) -> Vec<u8> {
    encode(&image(layout, sixteen), format, opts).unwrap_or_else(|e| panic!("encoding a {format:?} seed failed: {e}"))
}

fn xpm() -> Vec<u8> {
    b"/* XPM */\nstatic char *seed[] = {\n\"3 2 3 1\",\n\"  c None\",\n\". c #FF0000\",\n\"+ c blue\",\n\". +\",\n\"+. \",\n};\n".to_vec()
}

/// ZSoft PCX header (128 bytes) for a 3×2 image.
fn pcx_header(bits: u8, planes: u8, stride: u16, palette_type: u16) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(&[0x0a, 5, 1, bits]);
    b[8..10].copy_from_slice(&2u16.to_le_bytes()); // x1
    b[10..12].copy_from_slice(&1u16.to_le_bytes()); // y1
    b[12..16].copy_from_slice(&[72, 0, 72, 0]);
    for (i, c) in b[16..64].iter_mut().enumerate() {
        *c = (i as u8).wrapping_mul(17);
    }
    b[65] = planes;
    b[66..68].copy_from_slice(&stride.to_le_bytes());
    b[68..70].copy_from_slice(&palette_type.to_le_bytes());
    b
}

fn pcx() -> Vec<Vec<u8>> {
    // 24-bit: three 8-bit planes, rows of 4 bytes (even stride), one RLE run and literals.
    let mut rgb = pcx_header(8, 3, 4, 1);
    for _ in 0..2 {
        rgb.extend_from_slice(&[0xC4, 0x10, 0x20, 0x30, 0x40, 0x00, 0x50, 0x60, 0x70, 0x00, 0x80, 0x90, 0xA0]);
    }
    // 8-bit indexed with the VGA palette after the pixels.
    let mut indexed = pcx_header(8, 1, 4, 1);
    for _ in 0..2 {
        indexed.extend_from_slice(&[1, 2, 3, 0]);
    }
    indexed.push(0x0c);
    indexed.extend((0..768).map(|i| (i % 256) as u8));
    // 4-bit EGA: one plane, palette from the header.
    let mut ega = pcx_header(4, 1, 2, 1);
    ega.extend_from_slice(&[0x12, 0x30, 0x45, 0x60]);
    vec![rgb, indexed, ega]
}

/// A `FORM ILBM` with `BMHD`, `CMAP` and `BODY`, 4×2 pixels in 2 bitplanes.
fn ilbm(compression: u8) -> Vec<u8> {
    let mut b = b"FORM\0\0\0\0ILBMBMHD".to_vec();
    b.extend_from_slice(&20u32.to_be_bytes());
    b.extend_from_slice(&4u16.to_be_bytes());
    b.extend_from_slice(&2u16.to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 0, 2, 0, compression, 0, 0, 0, 1, 1, 0, 0, 0, 0]);
    b.extend_from_slice(b"CMAP");
    b.extend_from_slice(&12u32.to_be_bytes());
    b.extend_from_slice(&[0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
    // Each row: plane 0 then plane 1, two bytes each (rows are padded to 16 pixels).
    let rows: [[u8; 2]; 4] = [[0b1010_0000, 0], [0b0110_0000, 0], [0b0101_0000, 0], [0b1001_0000, 0]];
    let body: Vec<u8> = if compression == 1 { rows.iter().flat_map(|r| [1, r[0], r[1]]).collect() } else { rows.iter().flatten().copied().collect() };
    b.extend_from_slice(b"BODY");
    b.extend_from_slice(&(body.len() as u32).to_be_bytes());
    b.extend_from_slice(&body);
    if body.len() % 2 == 1 {
        b.push(0);
    }
    let size = (b.len() - 8) as u32;
    b[4..8].copy_from_slice(&size.to_be_bytes());
    b
}

/// A DICOM file: 128-byte preamble, `DICM`, a group-0002 transfer syntax, then the image pixel
/// module in explicit or implicit little-endian VR.
fn dicom(explicit: bool, samples: u16, bits: u16, photometric: &str) -> Vec<u8> {
    fn element(b: &mut Vec<u8>, g: u16, e: u16, vr: &[u8; 2], value: &[u8], explicit: bool) {
        let mut v = value.to_vec();
        if v.len() % 2 == 1 {
            v.push(if vr == b"UI" { 0 } else { b' ' });
        }
        b.extend_from_slice(&g.to_le_bytes());
        b.extend_from_slice(&e.to_le_bytes());
        if explicit {
            b.extend_from_slice(vr);
            if vr == b"OB" || vr == b"OW" {
                b.extend_from_slice(&[0, 0]);
                b.extend_from_slice(&(v.len() as u32).to_le_bytes());
            } else {
                b.extend_from_slice(&(v.len() as u16).to_le_bytes());
            }
        } else {
            b.extend_from_slice(&(v.len() as u32).to_le_bytes());
        }
        b.extend_from_slice(&v);
    }
    let ts: &[u8] = if explicit { b"1.2.840.10008.1.2.1" } else { b"1.2.840.10008.1.2" };
    let mut b = vec![0u8; 128];
    b.extend_from_slice(b"DICM");
    element(&mut b, 0x0002, 0x0010, b"UI", ts, true);
    let us = |v: u16| v.to_le_bytes();
    element(&mut b, 0x0028, 0x0002, b"US", &us(samples), explicit);
    element(&mut b, 0x0028, 0x0004, b"CS", photometric.as_bytes(), explicit);
    element(&mut b, 0x0028, 0x0006, b"US", &us(0), explicit);
    element(&mut b, 0x0028, 0x0010, b"US", &us(2), explicit);
    element(&mut b, 0x0028, 0x0011, b"US", &us(3), explicit);
    element(&mut b, 0x0028, 0x0100, b"US", &us(bits), explicit);
    element(&mut b, 0x0028, 0x0101, b"US", &us(bits), explicit);
    element(&mut b, 0x0028, 0x0102, b"US", &us(bits - 1), explicit);
    element(&mut b, 0x0028, 0x0103, b"US", &us(0), explicit);
    let len = 3 * 2 * usize::from(samples) * usize::from(bits / 8);
    let pixels: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(41)).collect();
    element(&mut b, 0x7FE0, 0x0010, if bits > 8 { b"OW" } else { b"OB" }, &pixels, explicit);
    b
}

/// The seeds of one target, each with the format it must decode as.
fn seeds(target: &str) -> Vec<(Format, Vec<u8>)> {
    use ChannelLayout::{Gray, GrayA, Rgb, Rgba};
    let opts = EncodeOptions::default();
    let dds = |compression, mipmaps| {
        let o = EncodeOptions { dds: DdsOptions { compression, mipmaps }, ..EncodeOptions::default() };
        (Format::Dds, encoded(Format::Dds, Rgba, false, &o))
    };
    let dpx = |format, bits, layout, sixteen| {
        let o = EncodeOptions { dpx_bits: bits, ..EncodeOptions::default() };
        (format, encoded(format, layout, sixteen, &o))
    };
    match target {
        "decode_dds" => vec![
            dds(DdsCompression::None, false),
            dds(DdsCompression::None, true),
            dds(DdsCompression::Bc1, false),
            dds(DdsCompression::Bc2, false),
            dds(DdsCompression::Bc3, true),
            (Format::Dds, encoded(Format::Dds, Gray, false, &opts)),
        ],
        "decode_dicom" => vec![
            (Format::Dicom, dicom(true, 1, 8, "MONOCHROME2")),
            (Format::Dicom, dicom(false, 1, 16, "MONOCHROME1")),
            (Format::Dicom, dicom(true, 3, 8, "RGB")),
        ],
        "decode_dpx" => vec![
            dpx(Format::Dpx, None, Rgb, false),
            dpx(Format::Dpx, Some(DpxBits::Bits10), Rgb, true),
            dpx(Format::Dpx, Some(DpxBits::Bits12), Rgba, true),
            dpx(Format::Dpx, Some(DpxBits::Bits16), Gray, true),
            dpx(Format::Cineon, None, Rgb, true),
        ],
        "decode_fits" => {
            vec![(Format::Fits, encoded(Format::Fits, Gray, false, &opts)), (Format::Fits, encoded(Format::Fits, Rgb, true, &opts))]
        }
        "decode_iff" => vec![(Format::Iff, ilbm(0)), (Format::Iff, ilbm(1))],
        "decode_pcx" => pcx().into_iter().map(|b| (Format::Pcx, b)).collect(),
        "decode_pnm" => vec![
            (Format::Pnm, encoded(Format::Pnm, Gray, false, &opts)),
            (Format::Pnm, encoded(Format::Pnm, Rgb, false, &opts)),
            (Format::Pnm, encoded(Format::Pnm, Rgb, true, &opts)),
            (Format::Pnm, encoded(Format::Pnm, Rgba, false, &opts)),
            (Format::Pnm, b"P1\n# bitmap\n3 2\n1 0 1\n0 1 0\n".to_vec()),
            (Format::Pnm, b"P3\n3 2\n255\n255 0 0 0 255 0 0 0 255\n1 2 3 4 5 6 7 8 9\n".to_vec()),
        ],
        "decode_tga" => vec![
            (Format::Tga, encoded(Format::Tga, Rgb, false, &opts)),
            (Format::Tga, encoded(Format::Tga, Rgba, false, &opts)),
            (Format::Tga, encoded(Format::Tga, Gray, false, &opts)),
        ],
        "decode_xpm" => vec![(Format::Xpm, xpm())],
        "decode_any" => ["decode_dds", "decode_dicom", "decode_dpx", "decode_fits", "decode_iff", "decode_pcx", "decode_pnm", "decode_tga", "decode_xpm"]
            .iter()
            .flat_map(|t| seeds(t))
            .chain([Format::Png, Format::Tiff, Format::OpenExr, Format::Bmp, Format::Qoi].into_iter().map(|f| (f, encoded(f, GrayA, false, &opts))))
            .collect(),
        _ => Vec::new(),
    }
}

const TARGETS: &[&str] =
    &["decode_any", "decode_dds", "decode_dicom", "decode_dpx", "decode_fits", "decode_iff", "decode_pcx", "decode_pnm", "decode_tga", "decode_xpm"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let targets: Vec<&str> = if args.is_empty() { TARGETS.to_vec() } else { args.iter().map(String::as_str).collect() };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    for t in targets {
        let seeds = seeds(t);
        if seeds.is_empty() {
            println!("{t}: no generated seeds");
            continue;
        }
        let dir = root.join(t);
        std::fs::create_dir_all(&dir).expect("create the corpus folder");
        for (i, (format, bytes)) in seeds.iter().enumerate() {
            if let Err(e) = decode_as(*format, bytes) {
                panic!("{t}: seed {i} ({format:?}) does not decode: {e}");
            }
            // `decode_any` finds the format itself, so its seeds must be ones detection reaches.
            if t == "decode_any" && photocraft_codecs::detect(bytes) != Some(*format) {
                panic!("{t}: seed {i} ({format:?}) is detected as {:?}", photocraft_codecs::detect(bytes));
            }
            std::fs::write(dir.join(format!("seed-{i}")), bytes).expect("write a seed");
        }
        println!("{t}: {} seeds", seeds.len());
    }
}
