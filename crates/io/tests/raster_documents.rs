use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::{export, import};
use photocraft_raster::Surface;
fn doc(depth: SampleType, mode: ColorMode) -> Document {
    let mut doc = Document::new("delivery", Size::new(16, 16), mode, depth);
    doc.resolution_dpi = 144.0;
    let mut surface = Surface::new(PixelFormat::new(mode, depth, true));
    let px = if mode == ColorMode::Grayscale { vec![0.4, 0.5] } else { vec![0.2, 0.4, 0.6, 0.5] };
    surface.write_region(Rect::new(0, 0, 16, 16), &px.repeat(256));
    doc.layers.push(Layer::new("Raster", LayerContent::Raster(surface)));
    doc
}
#[test]
fn svg_and_svgz_embed_the_actual_raster_and_reopen() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let original = doc(depth, ColorMode::Rgb);
        for ext in ["svg", "svgz"] {
            let result = export(&original, ext, &Default::default()).unwrap();
            assert!(result.warnings.iter().any(|w| w.contains("raster image")));
            let reopened = import(&format!("delivery.{ext}"), &result.bytes).unwrap().document;
            assert_eq!(reopened.size, original.size);
            let a = photocraft_compose::flatten(&original).to_rgba8();
            let b = photocraft_compose::flatten(&reopened).to_rgba8();
            assert!(a.pixels.iter().zip(&b.pixels).all(|(a, b)| a.abs_diff(*b) <= 1));
        }
    }
}
#[test]
fn pdf_pages_encode_gray_rgb_depth_transparency_profile_and_real_resolution() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut original = doc(depth, mode);
            original.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
            if mode == ColorMode::Grayscale {
                original.icc_profile = None;
            }
            let result = export(&original, "pdf", &Default::default()).unwrap();
            let source = String::from_utf8_lossy(&result.bytes);
            assert!(source.contains("/MediaBox [0 0 8 8]"));
            assert!(source.contains("/SMask 6 0 R"));
            assert!(source.contains(if depth == SampleType::U8 { "/BitsPerComponent 8" } else { "/BitsPerComponent 16" }));
            if mode == ColorMode::Rgb {
                assert!(source.contains("/ICCBased"));
            }
            // Cross-reference offsets are checked against the serialized object boundaries.
            let xref = source.rfind("xref\n0 ").unwrap();
            let rows = source[xref..].lines().skip(3).take(if mode == ColorMode::Rgb { 7 } else { 6 });
            for (i, row) in rows.enumerate() {
                let offset = row.split_whitespace().next().unwrap().parse::<usize>().unwrap();
                assert!(result.bytes[offset..].starts_with(format!("{} 0 obj", i + 1).as_bytes()));
            }
            if let Ok(dir) = std::env::var("PHOTOCRAFT_EXPORT_ORACLE_DIR") {
                std::fs::write(std::path::Path::new(&dir).join(format!("delivery-{mode:?}-{depth:?}.pdf")), result.bytes).unwrap();
            }
        }
    }
    let mut bad = doc(SampleType::U8, ColorMode::Rgb);
    bad.resolution_dpi = f32::NAN;
    assert!(export(&bad, "pdf", &Default::default()).is_err());
}
