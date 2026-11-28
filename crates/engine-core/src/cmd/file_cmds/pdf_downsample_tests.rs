//! Save a Copy › PDF downsamples its image to `pdfDownsamplePpi` (#9642), as Save Adobe PDF ›
//! Compression › Bicubic Downsampling To does: the page keeps its size, the image has fewer
//! pixels and the file gets smaller. Unset, the image keeps every pixel, as before.

use super::tests::tmp;
use super::*;

/// A noisy 600 × 300 px document at 300 ppi: two by one inches, and pixels Flate can't shrink
/// much, so the file size follows the pixel count.
fn noisy_300_ppi() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 600, "height": 300, "resolution": 300, "depth": 8, "background": "#808080"})).unwrap();
    s.execute("filter.noise.addNoise", json!({"amount": 40, "seed": 2})).unwrap();
    s
}

/// The bytes of a Save a Copy to `name` in `dir` with `extra` params.
fn save_pdf(s: &mut Session, dir: &str, name: &str, extra: Value) -> std::result::Result<Vec<u8>, EngineError> {
    let out = join(dir, name);
    let mut p = extra;
    p["path"] = json!(out);
    s.execute("file.saveACopy", p)?;
    Ok(std::fs::read(&out).unwrap())
}

fn has(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn pdf_downsample_ppi_resamples_the_image_and_keeps_the_page() {
    let dir = tmp("pdf-downsample");
    let mut s = noisy_300_ppi();
    let full = save_pdf(&mut s, &dir, "full.pdf", json!({})).unwrap();
    assert!(has(&full, "/Width 600 /Height 300"), "unset: every pixel, as before");
    let half = save_pdf(&mut s, &dir, "half.pdf", json!({"pdfDownsamplePpi": 150})).unwrap();
    let quarter = save_pdf(&mut s, &dir, "quarter.pdf", json!({"pdfDownsamplePpi": 75})).unwrap();
    assert!(has(&half, "/Width 300 /Height 150"), "150 ppi over two by one inches");
    assert!(has(&quarter, "/Width 150 /Height 75"));
    for pdf in [&full, &half, &quarter] {
        assert!(has(pdf, "/MediaBox [0 0 144.00 72.00]"), "the page stays two by one inches");
    }
    assert!(half.len() * 3 < full.len(), "a quarter of the pixels: {} vs {} bytes", half.len(), full.len());
    assert!(quarter.len() * 3 < half.len(), "{} vs {} bytes", quarter.len(), half.len());
}

#[test]
fn pdf_downsample_ppi_never_upsamples() {
    let dir = tmp("pdf-downsample-above");
    let mut s = noisy_300_ppi();
    let full = save_pdf(&mut s, &dir, "full.pdf", json!({})).unwrap();
    for ppi in [json!(300), json!(600), json!(null)] {
        let pdf = save_pdf(&mut s, &dir, "same.pdf", json!({"pdfDownsamplePpi": ppi})).unwrap();
        assert!(has(&pdf, "/Width 600 /Height 300"), "{ppi}: the image keeps its pixels");
        assert_eq!(pdf.len(), full.len(), "{ppi}: the same file as without it");
    }
}

#[test]
fn a_bad_pdf_downsample_ppi_writes_nothing() {
    let dir = tmp("pdf-downsample-bad");
    let mut s = noisy_300_ppi();
    for bad in [json!(0), json!(-72), json!(30001), json!("150"), json!(true), json!([150])] {
        let r = save_pdf(&mut s, &dir, "bad.pdf", json!({"pdfDownsamplePpi": bad}));
        assert!(matches!(r, Err(EngineError::BadParams { .. })), "{bad}: {r:?}");
        assert!(!std::path::Path::new(&join(&dir, "bad.pdf")).exists(), "{bad}: nothing written");
    }
}
