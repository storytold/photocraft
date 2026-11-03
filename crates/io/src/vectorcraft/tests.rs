//! Synthetic VectorCraft files (JSON written here, a literal PDF with a red rectangle) for the
//! reader: the drawn artboard, compression, the missing PDF, several artboards and large pages.

use std::io::Write as _;

use super::*;

/// A PDF of `pages` pages, each `w` × `h` points with its left half filled red.
fn pdf(w: u32, h: u32, pages: usize) -> Vec<u8> {
    let content = format!("1 0 0 rg 0 0 {} {h} re f", w / 2);
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 4 + i)).collect();
    let mut out = format!(
        "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [{}] /Count {pages} >>\nendobj\n3 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
        kids.join(" "),
        content.len()
    );
    for i in 0..pages {
        out.push_str(&format!("{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents 3 0 R >>\nendobj\n", 4 + i));
    }
    out.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    out.into_bytes()
}

/// A version 4 `.vectorcraft` file whose `pdf` member (when given) follows the JSON as a blob.
fn file(pdf: Option<&[u8]>) -> Vec<u8> {
    let member = match pdf {
        Some(p) => format!(r#""pdf":{{"mime":"application/pdf","blob":[0,{}]}},"#, p.len()),
        None => String::new(),
    };
    let mut out =
        format!(r#"{{"format":"vectorcraft","version":4,"generator":"VectorCraft 0.1.0",{member}"document":{{"artboards":[],"layers":[]}}}}"#).into_bytes();
    if let Some(p) = pdf {
        out.extend_from_slice(b"\n\0VCBLOBS\0");
        out.extend_from_slice(p);
    }
    out
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn pixel(r: &ImportResult, x: i32, y: i32) -> Vec<f32> {
    r.document.layers[0].surface().unwrap().pixel(x, y)
}

#[test]
fn the_first_artboard_is_drawn_from_the_carried_pdf() {
    let r = crate::import("Logo.vectorcraft", &file(Some(&pdf(80, 40, 1)))).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (80, 40));
    assert_eq!(d.resolution_dpi, 72.0);
    assert_eq!(d.icc_profile, Some(photocraft_cms::Builtin::Srgb.profile().to_bytes()));
    assert_eq!(d.layers.len(), 1);
    // Red where the rectangle is, transparent beside it.
    let red = pixel(&r, 10, 20);
    assert!(red[0] > 0.99 && red[1] < 0.01 && red[2] < 0.01 && red[3] > 0.99, "{red:?}");
    assert!(pixel(&r, 70, 20)[3] < 0.01);
    assert!(r.source_read_only, "save never writes back over the .vectorcraft file");
    assert!(!r.preview_only);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn compressed_files_and_misleading_names_are_recognized_by_content() {
    let r = crate::import("Untitled", &gzip(&file(Some(&pdf(20, 10, 1))))).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (20, 10));
    assert!(pixel(&r, 2, 5)[0] > 0.99);
}

#[test]
fn a_file_without_its_pdf_names_create_pdf_compatible_file() {
    let e = crate::import("Logo.vectorcraft", &file(None)).unwrap_err();
    assert!(matches!(e, IoError::VectorCraft(_)), "{e:?}");
    assert!(e.to_string().contains("Create PDF-Compatible File"), "{e}");
}

#[test]
fn a_damaged_file_named_vectorcraft_keeps_the_vectorcraft_error() {
    for bytes in [&b"{\"format\":\"vectorcraft\""[..], b"not a document"] {
        let e = crate::import("Logo.vectorcraft", bytes).unwrap_err();
        assert!(matches!(e, IoError::VectorCraft(_)), "{e:?}");
    }
}

#[test]
fn only_the_first_of_several_artboards_is_imported() {
    let r = crate::import("Pages.vectorcraft", &file(Some(&pdf(30, 30, 3)))).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (30, 30));
    assert!(r.warnings.iter().any(|w| w.contains("first of the file's 3 artboards")), "{:?}", r.warnings);
}

#[test]
fn a_page_past_the_raster_limits_is_drawn_smaller() {
    let r = crate::import("Banner.vectorcraft", &file(Some(&pdf(20_000, 100, 1)))).unwrap();
    let d = &r.document;
    assert!(d.size.width <= raster::MAX_SIDE && d.size.width > raster::MAX_SIDE - 2, "{}", d.size.width);
    assert!(d.resolution_dpi < 72.0 && d.resolution_dpi > 29.0, "{}", d.resolution_dpi);
    assert!(r.warnings.iter().any(|w| w.contains("pixels per inch")), "{:?}", r.warnings);
    for (w, h) in [(20_000.0, 100.0), (9_000.0, 9_000.0), (100.0, 1e6), (8192.0, 8192.0), (612.0, 792.0)] {
        let k = max_scale(w, h);
        let (pw, ph) = ((w * k).floor() as u64, (h * k).floor() as u64);
        assert!(pw <= u64::from(raster::MAX_SIDE) && ph <= u64::from(raster::MAX_SIDE) && pw * ph <= raster::MAX_PIXELS, "{w}x{h}: {pw}x{ph}");
    }
    assert!(max_scale(612.0, 792.0) > 10.0);
}

#[test]
fn import_scaled_draws_more_pixels_per_point() {
    let r = import_scaled("Logo.vectorcraft", &file(Some(&pdf(80, 40, 1))), 2.5, &Interrupt::NONE).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (200, 100));
    assert_eq!(r.document.resolution_dpi, 180.0);
    assert!(pixel(&r, 95, 50)[0] > 0.99 && pixel(&r, 105, 50)[3] < 0.01);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    // Past the raster limits, the largest scale that fits.
    let r = import_scaled("Logo.vectorcraft", &file(Some(&pdf(80, 40, 1))), 1000.0, &Interrupt::NONE).unwrap();
    assert!(r.document.size.width <= raster::MAX_SIDE && r.document.size.width > raster::MAX_SIDE - 2);
    assert!(r.warnings.iter().any(|w| w.contains("pixels per inch")), "{:?}", r.warnings);
    assert!(matches!(import_scaled("Logo.vectorcraft", &file(Some(&pdf(80, 40, 1))), 0.0, &Interrupt::NONE), Err(IoError::VectorCraft(_))));
    // Relative to the size it opens at: a page that opens smaller than one pixel per point
    // can't be drawn any larger.
    let limit = scale_limit(&file(Some(&pdf(80, 40, 1)))).unwrap();
    assert!((limit - max_scale(80.0, 40.0)).abs() < 1e-3, "{limit}");
    let banner_limit = scale_limit(&file(Some(&pdf(20_000, 2_000, 1)))).unwrap();
    assert!((1.0..1.001).contains(&banner_limit), "{banner_limit}");
    assert!(matches!(scale_limit(&file(None)), Err(IoError::VectorCraft(_))));
    // The size Open draws at, without drawing.
    assert_eq!(drawn_size(&file(Some(&pdf(80, 40, 1)))).unwrap(), (80, 40));
    let banner = file(Some(&pdf(20_000, 2_000, 1)));
    let opened = crate::import("Banner.vectorcraft", &banner).unwrap().document.size;
    assert_eq!(drawn_size(&banner).unwrap(), (opened.width, opened.height));
}

#[test]
fn a_fractional_page_keeps_its_opened_size_at_every_scale() {
    let page = String::from_utf8(pdf(20, 10, 1)).unwrap().replace("/MediaBox [0 0 20 10]", "/MediaBox [0 0 20.5 10.5]");
    let bytes = file(Some(page.as_bytes()));
    let opened = crate::import("Mark.vectorcraft", &bytes).unwrap().document.size;
    assert_eq!((opened.width, opened.height), (20, 10));
    for (scale, size) in [(2.0, (40, 20)), (3.0, (60, 30)), (0.5, (10, 5))] {
        let d = import_scaled("Mark.vectorcraft", &bytes, scale, &Interrupt::NONE).unwrap().document;
        assert_eq!((d.size.width, d.size.height), size, "at {scale}");
    }
    // A page under one point a side opens at one pixel and grows with the scale.
    let tiny = String::from_utf8(pdf(20, 10, 1)).unwrap().replace("/MediaBox [0 0 20 10]", "/MediaBox [0 0 0.5 0.5]");
    let bytes = file(Some(tiny.as_bytes()));
    assert_eq!(drawn_size(&bytes).unwrap(), (1, 1));
    let d = import_scaled("Dot.vectorcraft", &bytes, 4.0, &Interrupt::NONE).unwrap().document;
    assert_eq!((d.size.width, d.size.height), (4, 4));
    // Past the raster limits it stays at the largest size, relative to the one it opens at.
    let limit = scale_limit(&bytes).unwrap();
    assert!(limit > 7999.0, "{limit}");
    let at_limit = import_scaled("Dot.vectorcraft", &bytes, limit, &Interrupt::NONE).unwrap().document.size;
    let past = import_scaled("Dot.vectorcraft", &bytes, limit * 2.0, &Interrupt::NONE).unwrap().document.size;
    assert_eq!(past, at_limit);
    // A square at the 64,000,000-pixel limit.
    assert_eq!((past.width, past.height), (8000, 8000));
}

#[test]
fn a_canceled_open_stops_drawing() {
    let cancel = || true;
    let e = crate::import_with("Logo.vectorcraft", &file(Some(&pdf(80, 40, 1))), &Interrupt::cancel_only(&cancel)).unwrap_err();
    assert!(matches!(e, IoError::Cancelled), "{e:?}");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn cancellation_after_the_draw_callback_starts_reaches_the_pdf_renderer() {
    let pdf = carried_pdf(&file(Some(&pdf(80, 40, 1)))).unwrap();
    let pages = pdf.pages();
    let (page, _, _) = first_page(pages).unwrap();
    let options = Options { scale: [1.0; 2], size: Some([80, 40]), ..Options::default() };
    let requested = AtomicBool::new(false);
    let cancel = || requested.load(Ordering::Relaxed);
    let result = with_render_cancellation(&Interrupt::cancel_only(&cancel), |settings| {
        let flag = settings.cancelled.as_ref().unwrap();
        assert!(!flag.load(Ordering::Relaxed), "the draw starts before cancellation");
        requested.store(true, Ordering::Relaxed);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !flag.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        raster::render(page, &RenderCache::new(), settings, &options)
    });
    assert!(matches!(result, Err(raster::Error::Cancelled)));
}

#[test]
fn scale_limits_reuse_the_same_bytes_and_recheck_changed_or_invalid_sources() {
    let a = file(Some(&pdf(83, 47, 1)));
    let b = file(Some(&pdf(83, 48, 1)));
    let before = SCALE_LIMIT_READS.with(|n| n.get());
    let limit = scale_limit(&a).unwrap();
    assert_eq!(scale_limit(&a.clone()).unwrap(), limit);
    assert_eq!(SCALE_LIMIT_READS.with(|n| n.get()), before + 1);
    scale_limit(&b).unwrap();
    assert_eq!(SCALE_LIMIT_READS.with(|n| n.get()), before + 2);
    for _ in 0..2 {
        assert!(scale_limit(b"not a VectorCraft file").is_err());
    }
    assert_eq!(SCALE_LIMIT_READS.with(|n| n.get()), before + 4, "errors are not cached");
}

#[test]
fn premultiplied_pixels_become_straight_alpha() {
    let mut px = [128, 64, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0];
    unpremultiply(&mut px);
    assert_eq!(px, [255, 128, 0, 128, 10, 20, 30, 255, 0, 0, 0, 0]);
}
