//! #14069: an EXR with only luminance or single data channels opens as an RGB 32-bit document
//! tagged linear sRGB, R = G = B = the file's value (above 1 kept), alpha kept; RGB files are
//! unchanged.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_io::{ImportOptions, ImportResult, exr_parts, import, import_file};
use photocraft_raster::Interrupt;

const W: u32 = 32;
const H: u32 = 16;

/// A minimal uncompressed scanline EXR, `W` x `H`, FLOAT channels; each channel is `left` in
/// columns 0..16 and `right` in 16..32. Hand-written so channel names like `Z` are possible.
fn exr(channels: &[(&str, f32, f32)]) -> Vec<u8> {
    let mut chans: Vec<&(&str, f32, f32)> = channels.iter().collect();
    chans.sort_by_key(|c| c.0);
    let mut out = vec![0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0];
    let mut attr = |name: &str, kind: &str, value: &[u8]| {
        out.extend(name.bytes().chain([0]).chain(kind.bytes()).chain([0]));
        out.extend((value.len() as i32).to_le_bytes());
        out.extend(value);
    };
    let mut chlist = Vec::new();
    for (name, _, _) in &chans {
        chlist.extend(name.bytes().chain([0]));
        chlist.extend(2i32.to_le_bytes()); // FLOAT
        chlist.extend([0, 0, 0, 0]); // pLinear + reserved
        chlist.extend(1i32.to_le_bytes());
        chlist.extend(1i32.to_le_bytes());
    }
    chlist.push(0);
    let window: Vec<u8> = [0i32, 0, W as i32 - 1, H as i32 - 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    attr("channels", "chlist", &chlist);
    attr("compression", "compression", &[0]);
    attr("dataWindow", "box2i", &window);
    attr("displayWindow", "box2i", &window);
    attr("lineOrder", "lineOrder", &[0]);
    attr("pixelAspectRatio", "float", &1f32.to_le_bytes());
    attr("screenWindowCenter", "v2f", &[0; 8]);
    attr("screenWindowWidth", "float", &1f32.to_le_bytes());
    out.push(0);
    let line_bytes = 4 * W as usize * chans.len();
    let table_end = out.len() + 8 * H as usize;
    for y in 0..H as usize {
        out.extend(((table_end + y * (8 + line_bytes)) as u64).to_le_bytes());
    }
    for y in 0..H as i32 {
        out.extend(y.to_le_bytes());
        out.extend((line_bytes as i32).to_le_bytes());
        for (_, left, right) in &chans {
            for x in 0..W {
                out.extend(if x < 16 { left } else { right }.to_le_bytes());
            }
        }
    }
    out
}

fn y_only() -> Vec<u8> {
    exr(&[("Y", 0.7, 0.2)])
}
fn z_only() -> Vec<u8> {
    exr(&[("Z", 5.0, 0.25)])
}
fn y_a() -> Vec<u8> {
    exr(&[("Y", 0.7, 0.2), ("A", 1.0, 0.5)])
}
fn rgba() -> Vec<u8> {
    exr(&[("R", 0.8, 0.1), ("G", 0.4, 0.1), ("B", 0.2, 0.1), ("A", 1.0, 0.5)])
}

/// File > Open: the file from disk.
fn open_file(tag: &str, bytes: &[u8]) -> ImportResult {
    let path = std::env::temp_dir().join(format!("photocraft-14069-{tag}-{}.exr", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let result = import_file("f.exr", &path, &ImportOptions::default(), &Interrupt::NONE);
    let _ = std::fs::remove_file(&path);
    result.unwrap()
}

/// File > Open EXR Parts... with every listed entry chosen.
fn open_parts(bytes: &[u8]) -> ImportResult {
    let entries = exr_parts::entries(bytes).unwrap();
    let selection = exr_parts::Selection { parts: entries.iter().map(|e| e.name.clone()).collect(), groups: vec![], view: "both".into(), precomp: false };
    exr_parts::import("p", bytes, &selection).unwrap()
}

fn px(doc: &Document, x: i32, y: i32) -> Vec<f32> {
    doc.layers[0].surface().unwrap().read_region(Rect::from_xywh(x, y, 1, 1))
}

/// RGB 32-bit, linear sRGB, (4,4) = `left`, (24,4) = `right` as RGBA.
fn assert_rgb(route: &str, r: &ImportResult, left: [f32; 4], right: [f32; 4]) {
    let d = &r.document;
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::F32), "{route}: mode and depth");
    assert_eq!(d.icc_profile, Some(craft_cms::Builtin::LinearSrgb.profile().to_bytes()), "{route}: linear sRGB, not linear gray");
    assert_eq!(px(d, 4, 4), left, "{route}: left half");
    assert_eq!(px(d, 24, 4), right, "{route}: right half");
}

#[test]
fn luminance_exr_opens_as_rgb_through_file_open() {
    assert_rgb("Y File > Open", &open_file("y", &y_only()), [0.7, 0.7, 0.7, 1.0], [0.2, 0.2, 0.2, 1.0]);
    assert_rgb("Z File > Open", &open_file("z", &z_only()), [5.0, 5.0, 5.0, 1.0], [0.25, 0.25, 0.25, 1.0]);
    assert_rgb("Y+A File > Open", &open_file("ya", &y_a()), [0.7, 0.7, 0.7, 1.0], [0.2, 0.2, 0.2, 0.5]);
}

#[test]
fn luminance_and_data_exr_open_as_rgb_through_open_exr_parts() {
    assert_rgb("Y parts", &open_parts(&y_only()), [0.7, 0.7, 0.7, 1.0], [0.2, 0.2, 0.2, 1.0]);
    assert_rgb("Z parts", &open_parts(&z_only()), [5.0, 5.0, 5.0, 1.0], [0.25, 0.25, 0.25, 1.0]);
    assert_rgb("Y+A parts", &open_parts(&y_a()), [0.7, 0.7, 0.7, 1.0], [0.2, 0.2, 0.2, 0.5]);
}

#[test]
fn rgba_exr_opens_as_before() {
    let left = [0.8, 0.4, 0.2, 1.0];
    let right = [0.1, 0.1, 0.1, 0.5];
    assert_rgb("RGBA File > Open", &open_file("rgba", &rgba()), left, right);
    assert_rgb("RGBA parts", &open_parts(&rgba()), left, right);
    assert_rgb("RGBA bytes", &import("f.exr", &rgba()).unwrap(), left, right);
}

/// Open point: the whole-file decoder in `flat.rs` (the bytes import, and File > Open's
/// fallback for headers the band decoder refuses) still maps a gray EXR to Grayscale. That file
/// is outside this item's scope until the epic owner agrees.
#[test]
#[ignore = "flat.rs still opens gray EXRs as Grayscale (#14069 open point)"]
fn luminance_exr_opens_as_rgb_through_whole_file_decoder() {
    assert_rgb("Y bytes", &import("f.exr", &y_only()).unwrap(), [0.7, 0.7, 0.7, 1.0], [0.2, 0.2, 0.2, 1.0]);
    assert_rgb("Z bytes", &import("f.exr", &z_only()).unwrap(), [5.0, 5.0, 5.0, 1.0], [0.25, 0.25, 0.25, 1.0]);
    assert_rgb("Y+A bytes", &import("f.exr", &y_a()).unwrap(), [0.7, 0.7, 0.7, 1.0], [0.2, 0.2, 0.2, 0.5]);
}
