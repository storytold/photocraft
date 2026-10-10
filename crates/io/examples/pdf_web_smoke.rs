//! Run with wasm-bindgen's Node.js target; exercises the actual no-unwind build.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn pdf_web_smoke() -> Result<String, wasm_bindgen::JsValue> {
    use photocraft_doc::{ColorMode, Document, SampleType, Size};
    use photocraft_io::{ExportOptions, export, pdf};
    use std::sync::Arc;
    let document = Document::new("existing work", Size::new(8, 8), ColorMode::Rgb, SampleType::U8);
    let before = document.clone();
    let valid = export(&document, "pdf", &ExportOptions::default()).map_err(|e| e.to_string())?.bytes;
    let bomb = b"%PDF-1.4\n1 0 obj << /Type /XObject /Subtype /Image /Width 2147483647 /Height 2147483647 /Filter /JBIG2Decode >> stream\n\x97JB2\nendstream\nendobj\n".to_vec();
    for bytes in [valid, bomb, b"%PDF-broken".to_vec()] {
        let ctl = photocraft_raster::Interrupt::default();
        let errors = [
            pdf::page_sizes(&bytes).err(),
            pdf::page_preview(Arc::new(bytes.clone()), 0).err(),
            pdf::import_pdf("web.pdf", &bytes, &ctl).err(),
            pdf::import_pdf_pages_with("web.pdf", &bytes, Some(&[0]), pdf::ImportOptions { resolution: 300.0 }, &ctl).err(),
        ];
        for error in errors {
            if !error.is_some_and(|e| e.to_string().contains("desktop app")) {
                return Err("PDF import did not return the safe web fallback".into());
            }
        }
    }
    if document != before {
        return Err("Existing document changed".into());
    }
    let output = export(&document, "pdf", &ExportOptions::default()).map_err(|e| e.to_string())?;
    if !output.bytes.starts_with(b"%PDF-") {
        return Err("PDF export stopped working after rejected imports".into());
    }
    Ok("PASS: all PDF import/preview entry points return an error without trapping; existing work and PDF export survive".into())
}
