//! Writes a starting corpus for this crate's fuzz targets: small valid files built in code (the
//! repository commits no binary seeds), so libFuzzer starts from real XML/PSD/.pcraft structure
//! instead of having to guess it (`seeds.py` next to this file seeds the KRA and ORA targets).
//! Each seed must open, or this fails: a seed the importer rejects only teaches the fuzzer the
//! error path.
//!
//! `cargo run --example seeds -- [target ...]` (no target: all) writes `corpus/<target>/seed-*`.

use std::path::Path;
use std::sync::Arc;

use photocraft_algo::camera_raw::CameraRaw;
use photocraft_doc::{Adjustment, BlendMode, Color, ColorMode, Document, Fill, Layer, LayerContent, LayerMask, PixelFormat, Rect, SampleType, Size, TextLayer};
use photocraft_io::{ExportOptions, export, import};
use photocraft_raster::Surface;

const XMP: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Seed</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta>"#;

/// A raster layer holding a `w`×`h` colour ramp at (`x`, `y`).
fn block(name: &str, fmt: PixelFormat, x: i32, y: i32, w: u32, h: u32, seed: u32) -> Layer {
    let mut s = Surface::new(fmt);
    let mut data = Vec::new();
    let mut px = vec![0; fmt.bytes_per_pixel()];
    for j in 0..h {
        for i in 0..w {
            let v = |k: u32| ((i * 7 + j * 13 + k * 29 + seed * 31) % 256) as f32 / 255.0;
            photocraft_raster::encode_pixel(&fmt, &[v(0), v(1), v(2), 0.25 + 0.75 * v(3)], &mut px);
            data.extend_from_slice(&px);
        }
    }
    s.write_interleaved(Rect::from_xywh(x, y, w, h), &data);
    Layer::new(name, LayerContent::Raster(s))
}

/// A small document with the layer kinds every format has to map: pixels at each offset, a
/// hidden layer, blend modes, a mask, nested groups, a fill, an adjustment and type.
fn document(depth: SampleType) -> Document {
    let fmt = PixelFormat::new(ColorMode::Rgb, depth, true);
    let mut d = Document::new("seed", Size { width: 24, height: 16 }, ColorMode::Rgb, depth);
    d.metadata.xmp = Some(Arc::from(XMP));
    let mut base = block("Base", fmt, 0, 0, 24, 16, 1);
    base.xmp = Some(Arc::from(XMP));
    let mut off = block("Off canvas", fmt, -4, 10, 12, 8, 2);
    off.blend = BlendMode::Multiply;
    off.opacity = 0.5;
    let mut hidden = block("Hidden", fmt, 3, 3, 4, 4, 3);
    hidden.visible = false;
    let mut masked = block("Masked", fmt, 2, 2, 10, 10, 4);
    let mut mask = LayerMask::hide_all();
    mask.surface.fill_rect(Rect::from_xywh(2, 2, 5, 10), &[1.0]);
    masked.mask = Some(mask);
    let mut inner = Layer::group("Inner", vec![block("Deep", fmt, 6, 1, 6, 6, 5)]);
    inner.blend = BlendMode::Screen;
    let mut group = Layer::group("Group <&> \"quoted\"", vec![masked, inner]);
    group.opacity = 0.75;
    let text = TextLayer { text: "Seed".into(), font_family: "Inter".into(), size_pt: 8.0, ..Default::default() };
    d.layers = vec![
        base,
        off,
        hidden,
        group,
        Layer::new("Solid", LayerContent::Fill(Fill::Solid(Color::rgb(0.2, 0.4, 0.6)))),
        Layer::new("Invert", LayerContent::Adjustment(Adjustment::Invert)),
        Layer::new("Type", LayerContent::Text(text)),
    ];
    d
}

/// `doc` written as `ext`; `None` when this build can't write that format (import-only formats).
fn written(doc: &Document, ext: &str) -> Option<Vec<u8>> {
    let opts = ExportOptions { read_linked_files: false, ..ExportOptions::default() };
    export(doc, ext, &opts).ok().map(|r| r.bytes)
}

fn documents_as(ext: &str) -> Vec<Vec<u8>> {
    [SampleType::U8, SampleType::U16, SampleType::F32].into_iter().filter_map(|depth| written(&document(depth), ext)).collect()
}

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="24" height="16" viewBox="0 0 24 16">
<defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="blue" stop-opacity=".5"/></linearGradient>
<clipPath id="c"><circle cx="12" cy="8" r="6"/></clipPath><symbol id="s"><rect width="4" height="4" fill="#0f0"/></symbol></defs>
<g id="layer" opacity=".8" transform="translate(1 1) rotate(5)">
<rect x="0" y="0" width="20" height="12" rx="2" fill="url(#g)" stroke="#000" stroke-width=".5"/>
<path d="M2 2 L10 2 Q14 6 10 10 C8 12 4 12 2 10 Z" fill="#ff0" fill-rule="evenodd" clip-path="url(#c)"/>
<use xlink:href="#s" x="16" y="8"/><text x="2" y="14" font-size="4">Seed</text></g></svg>"##;

const CAMERA_RAW_XMP: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Version="15.0" crs:WhiteBalance="Custom"
 crs:IncrementalTemperature="+12" crs:IncrementalTint="-8" crs:Exposure2012="+0.65" crs:Contrast2012="-12" crs:Highlights2012="-40" crs:Shadows2012="+35"
 crs:Clarity2012="+10" crs:Vibrance="+15" crs:Saturation="-5" crs:PostCropVignetteStyle="Highlight Priority" crs:PostCropVignetteAmount="-20">
<crs:ToneCurvePV2012><rdf:Seq><rdf:li>0, 0</rdf:li><rdf:li>128, 140</rdf:li><rdf:li>255, 255</rdf:li></rdf:Seq></crs:ToneCurvePV2012>
</rdf:Description></rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#;

/// The seeds of one target, each with the file name it must open as.
fn seeds(target: &str) -> Vec<(String, Vec<u8>)> {
    let named = |ext: &str, files: Vec<Vec<u8>>| files.into_iter().map(|b| (format!("seed.{ext}"), b)).collect::<Vec<_>>();
    match target {
        // `import_kra` and `import_ora` are seeded by `seeds.py` next to this file; XCF and PDN
        // have no seeds yet (PhotoCraft writes neither, and their test writers are private).
        "import_svg" => named("svg", vec![SVG.as_bytes().to_vec()]),
        "camera_raw_xmp" => {
            let mut edited = CameraRaw::default();
            if photocraft_io::camera_raw_xmp::load(&mut edited, CAMERA_RAW_XMP.as_bytes()).is_err() {
                panic!("the handwritten Camera Raw seed does not load");
            }
            let saved = photocraft_io::camera_raw_xmp::save(&edited).into_bytes();
            vec![("seed.xmp".into(), CAMERA_RAW_XMP.as_bytes().to_vec()), ("seed.xmp".into(), saved)]
        }
        "export_roundtrip" => {
            // The target picks its writer by the input's length (mod 4): pad PSD seeds (which
            // ignore trailing bytes, unlike a ZIP) with zeros so that they reach every writer.
            let psd = documents_as("psd");
            let mut v: Vec<(String, Vec<u8>)> = psd
                .iter()
                .chain(psd.first())
                .enumerate()
                .map(|(i, b)| {
                    let mut b = b.clone();
                    b.resize(b.len() + (i + 4 - b.len() % 4) % 4, 0);
                    ("seed.psd".to_string(), b)
                })
                .collect();
            v.extend(named("pcraft", documents_as("pcraft")));
            v
        }
        _ => Vec::new(),
    }
}

const TARGETS: &[&str] = &["import_svg", "camera_raw_xmp", "export_roundtrip"];

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
        for (i, (name, bytes)) in seeds.iter().enumerate() {
            // The Camera Raw target has its own loader; every other one opens through File › Open.
            if t != "camera_raw_xmp"
                && let Err(e) = import(name, bytes)
            {
                panic!("{t}: seed {i} ({name}) does not open: {e}");
            }
            std::fs::write(dir.join(format!("seed-{i}")), bytes).expect("write a seed");
        }
        println!("{t}: {} seeds", seeds.len());
    }
}
