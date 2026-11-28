//! The PDF image budget as a wasm32 run (no browser, no bindings), built as a binary so it needs
//! none of the crate's dev-dependencies: a PDF whose image declares far more pixels than it holds,
//! or whose JPX codestream, CCITT fax parameters or JBIG2 page information make the decoder build
//! far more than its dictionary declares, or whose inline images pass the page's budget together,
//! or whose content or Form XObject goes through an image codec, is refused before any decoder
//! runs. On wasm32-unknown-unknown
//! an attempt to decode would abort the module (an allocation past linear memory), so a normal
//! return is the proof. Natively it is the same check. Build and run:
//!
//! ```text
//! cargo build -p photocraft-io --bin pdf_budget_wasm --features wasm-budget-check --target wasm32-unknown-unknown
//! node crates/io/wasm/run_wasm_test.js target/wasm32-unknown-unknown/debug/pdf_budget_wasm.wasm
//! ```

use photocraft_io::pdf_import::{self, PdfOpenOptions};
use photocraft_raster::Interrupt;

/// A one-page PDF with `content`, and `xobject` (dictionary and data) as `/X0` when given.
fn pdf_of(xobject: Option<(&str, &[u8])>, content: &[u8]) -> Vec<u8> {
    let resources = if xobject.is_some() { "/XObject << /X0 5 0 R >>" } else { "" };
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << {resources} >> /Contents 4 0 R >>").into_bytes(),
        stream("", content),
    ];
    if let Some((dict, data)) = xobject {
        objs.push(stream(dict, data));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn stream(extra: &str, data: &[u8]) -> Vec<u8> {
    let mut o = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    o.extend_from_slice(data);
    o.extend_from_slice(b"\nendstream");
    o
}

/// A one-page PDF whose content stream has `contents_dict` entries, with `resources`, and objects
/// 5.. `more` (dictionary and data each, as streams).
fn pdf_with(resources: &str, contents_dict: &str, content: &[u8], more: &[(&str, &[u8])]) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << {resources} >> /Contents 4 0 R >>").into_bytes(),
        stream(contents_dict, content),
    ];
    for (dict, data) in more {
        objs.push(stream(dict, data));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A JBIG2 stream (embedded organisation) of one page information segment for `w × h`.
fn jbig2_page_only(w: u32, h: u32) -> Vec<u8> {
    let mut d = vec![0, 0, 0, 0, 48, 0, 1, 0, 0, 0, 19];
    d.extend_from_slice(&w.to_be_bytes());
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[0; 11]);
    d
}

/// Content drawing one inline image with `dict` and `data`.
fn inline(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut c = format!("q 200 0 0 200 0 0 cm BI {dict} ID ").into_bytes();
    c.extend_from_slice(data);
    c.extend_from_slice(b" EI Q\n");
    c
}

/// A JPEG 2000 codestream header alone: SOC, then a SIZ marker segment for a `w × h` grey image.
fn jpx_header_only(w: u32, h: u32) -> Vec<u8> {
    let mut d = vec![0xFF, 0x4F, 0xFF, 0x51, 0x00, 0x29, 0x00, 0x00];
    for v in [w, h, 0, 0, w, h, 0, 0] {
        d.extend_from_slice(&v.to_be_bytes());
    }
    d.extend_from_slice(&[0x00, 0x01, 0x07, 0x01, 0x01]);
    d
}

/// A 2 × 2 grey JPEG 2000 codestream (OpenJPEG 2.5, lossless 5/3): pixels 10, 200, 90, 30.
const J2K_GRAY_2X2: [u8; 143] = [
    0xff, 0x4f, 0xff, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x01, 0xff, 0x52, 0x00, 0x0c, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xff, 0x5c, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xff, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61,
    0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4f, 0x70, 0x65, 0x6e, 0x4a, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x32, 0x2e,
    0x35, 0x2e, 0x30, 0xff, 0x90, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x01, 0xff, 0x93, 0xc7, 0xd4, 0x04, 0x05, 0xdf, 0xc7, 0xda, 0x05, 0x0f,
    0xa8, 0x0a, 0x3f, 0x00, 0x20, 0x04, 0xaf, 0x05, 0xdf, 0x09, 0xd3, 0xff, 0xd9,
];

/// Every way of opening `bytes` is refused with an error naming `needle`.
fn refused(bytes: &[u8], needle: &str) {
    let e = pdf_import::import_pdf("big.pdf", bytes, &PdfOpenOptions::default(), &Interrupt::NONE).expect_err("the page is refused").to_string();
    assert!(e.contains(needle), "{e}");
    assert!(pdf_import::thumbnail(bytes, 1, 32, None).is_err(), "the thumbnail is refused");
    assert!(pdf_import::import_image("big.pdf", bytes, 1, None, None, &Interrupt::NONE).is_err(), "Images mode is refused");
    assert!(pdf_import::image_thumbnail(bytes, 1, 32, None).is_err(), "the image thumbnail is refused");
}

/// `bytes` opens as a document of `size`.
fn opens(bytes: &[u8], size: (u32, u32)) -> photocraft_io::ImportResult {
    let r = pdf_import::import_pdf("small.pdf", bytes, &PdfOpenOptions::default(), &Interrupt::NONE).expect("opens");
    assert_eq!((r.document.size.width, r.document.size.height), size);
    assert!(pdf_import::thumbnail(bytes, 1, 32, None).is_ok());
    r
}

/// Every check that must hold; a failed one panics (on wasm32: traps), so the run says so.
fn main() {
    let draw = b"q 200 0 0 200 0 0 cm /X0 Do Q";
    // 40000 × 40000 px declared (1.6 GP, 4.8 GB of RGB), 4 bytes of data.
    refused(
        &pdf_of(Some(("/Type /XObject /Subtype /Image /Width 40000 /Height 40000 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[0, 0, 0, 0])), draw),
        "40000 × 40000",
    );
    // A tiny image with the same shape opens as that image.
    opens(&pdf_of(Some(("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", &[10, 20, 30, 40])), draw), (2, 2));
    // 10 × 10 px declared, a JPX codestream whose SIZ encodes 20000 × 20000: refused from the
    // header, before the decoder runs; as an XObject, an inline image and an inline stencil.
    let jpx = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode";
    refused(&pdf_of(Some((jpx, &jpx_header_only(20000, 20000))), draw), "20000 × 20000");
    refused(&pdf_of(None, &inline("/W 10 /H 10 /CS /G /BPC 8 /F /JPXDecode", &jpx_header_only(20000, 20000))), "20000 × 20000");
    refused(&pdf_of(None, &inline("/W 10 /H 10 /IM true /F /JPXDecode", &jpx_header_only(20000, 20000))), "20000 × 20000");
    // A real 2 × 2 JPEG 2000 image passes the budget and decodes.
    let jpx = "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode";
    let r = opens(&pdf_of(Some((jpx, &J2K_GRAY_2X2)), draw), (2, 2));
    let px = r.document.layers[0].surface().expect("a raster layer").pixel(1, 0);
    assert!((px[0] * 255.0 - 200.0).abs() < 1.5, "{px:?}");
    // Five inline images of 64 MP each, within the per-image limit, 320 MP together.
    let one = b"q 10 0 0 10 0 0 cm BI /W 8000 /H 8000 /CS /G /BPC 8 /F /Fl ID x EI Q\n";
    refused(&pdf_of(None, &one.repeat(5)), "together");
    // CCITT fax: 10 × 10 px declared, /Columns × /Rows 20000 × 20000 (the decoder's buffer); as an
    // XObject and an inline stencil.
    let ccitt = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 20000 /Rows 20000 >>";
    refused(&pdf_of(Some((ccitt, &[0; 8])), draw), "20000 × 20000");
    refused(&pdf_of(None, &inline("/W 10 /H 10 /IM true /F /CCF /DP << /K -1 /Columns 20000 /Rows 20000 >>", &[0; 8])), "20000 × 20000");
    // JBIG2: 10 × 10 px declared, a 20000 × 20000 page information segment.
    let jbig2 = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode";
    refused(&pdf_of(Some((jbig2, &jbig2_page_only(20000, 20000))), draw), "20000 × 20000");
    // A JPX codestream as the page's content, and as a Form XObject the page never draws: no image
    // codec outside an image.
    refused(&pdf_with("", "/Filter /JPXDecode", &jpx_header_only(20000, 20000), &[]), "JPXDecode");
    refused(
        &pdf_with(
            "/XObject << /F 5 0 R >>",
            "",
            b"",
            &[("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Filter /JPXDecode", &jpx_header_only(20000, 20000))],
        ),
        "JPXDecode",
    );
    println!("pdf image budget: ok");
}
