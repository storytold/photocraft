//! #11796: a document's notes are saved into its PDF as text annotations (ISO 32000
//! `/Subtype /Text`, each with a `/Popup`), and [`read_notes`] reads them back with their text,
//! author, colour, open state and date; positions come back in page points, so they scale onto a
//! canvas by canvas height / page height. Hostile or truncated input never panics.

use super::*;
use lopdf::Object;
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{Layer, LayerContent, Note, Size};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

/// A white 600 × 400 RGB document at `dpi` with `notes`.
fn doc_with(notes: Vec<Note>, dpi: f32) -> Document {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut d = Document::new("notes.pdf", Size { width: 600, height: 400 }, ColorMode::Rgb, SampleType::U8);
    let mut s = Surface::new(fmt);
    let px: Vec<f32> = [1.0, 1.0, 1.0, 1.0].iter().copied().cycle().take(600 * 400 * 4).collect();
    s.write_region(Rect::from_xywh(0, 0, 600, 400), &px);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(s))];
    d.resolution_dpi = dpi;
    d.notes = notes;
    d
}

/// Three invented notes: plain, non-Latin text with an author and colour, and one near the corner.
fn three() -> Vec<Note> {
    vec![
        Note {
            text: "first".into(),
            author: "Ana".into(),
            position: [92.0, 90.0],
            popup: [112.0, 90.0, 352.0, 230.0],
            modified: "D:20260102030405Z".into(),
            ..Default::default()
        },
        Note {
            text: "línea 2 ✓\rsecond line".into(),
            author: "Bo Ström".into(),
            color: photocraft_color::Color::rgb(0.2, 0.6, 1.0),
            position: [292.0, 190.0],
            popup: [312.0, 190.0, 552.0, 330.0],
            open: true,
            modified: "D:20251231235959+01'00'".into(),
        },
        Note { text: String::new(), position: [582.0, 370.0], ..Default::default() },
    ]
}

fn pdf_of(d: &Document) -> Vec<u8> {
    export(d, false, &Interrupt::NONE).unwrap().bytes
}

#[test]
fn notes_are_written_as_text_annotations_with_popups() {
    let bytes = pdf_of(&doc_with(three(), 72.0));
    let pdf = lopdf::Document::load_mem(&bytes).expect("another reader opens the saved PDF");
    let (_, page) = pdf.get_pages().into_iter().next().expect("one page");
    let page = pdf.get_dictionary(page).unwrap();
    let annots = page.get(b"Annots").and_then(Object::as_array).expect("the page lists its annotations");
    assert_eq!(annots.len(), 6, "a text annotation and its popup per note");
    let dict = |o: &Object| pdf.get_dictionary(o.as_reference().unwrap()).unwrap();
    let text: Vec<_> = annots.iter().map(dict).filter(|a| a.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Text".as_slice())).collect();
    assert_eq!(text.len(), 3);
    // Note 1 at (92, 90) px on a 400 pt page at 72 ppi: its icon's top-left is (92, 310) in PDF space.
    let rect: Vec<f32> = text[0].get(b"Rect").and_then(Object::as_array).unwrap().iter().map(|v| v.as_float().unwrap()).collect();
    assert_eq!(rect, vec![92.0, 290.0, 112.0, 310.0]);
    let popup = dict(text[1].get(b"Popup").unwrap());
    assert_eq!(popup.get(b"Subtype").and_then(Object::as_name).unwrap(), b"Popup");
    let color: Vec<f32> = text[1].get(b"C").and_then(Object::as_array).unwrap().iter().map(|v| v.as_float().unwrap()).collect();
    assert_eq!(color, vec![0.2, 0.6, 1.0]);
}

#[test]
fn the_saved_notes_read_back_unchanged() {
    for dpi in [72.0, 300.0] {
        let want = three();
        let bytes = pdf_of(&doc_with(want.clone(), dpi));
        let (got, page_height) = read_notes(&bytes).unwrap();
        let s = f64::from(dpi) / 72.0;
        assert!((page_height * s - 400.0).abs() < 1e-6, "{dpi} ppi: the page is the canvas, {page_height} pt");
        assert_eq!(got.len(), 3, "{dpi} ppi");
        for (g, w) in got.iter().zip(&want) {
            // Back on the 400 px canvas: canvas height / page height.
            for k in 0..2 {
                assert!((g.position[k] * s - w.position[k]).abs() < 1.0, "{dpi} ppi: {:?} vs {:?}", g.position, w.position);
            }
            assert_eq!((g.text.as_str(), g.author.as_str(), g.open, g.modified.as_str()), (w.text.as_str(), w.author.as_str(), w.open, w.modified.as_str()));
            let (gc, wc) = (g.color.to_rgb(), w.color.to_rgb());
            assert!(gc.iter().zip(wc).all(|(a, b)| (a - b).abs() < 1.0 / 512.0), "{gc:?} vs {wc:?}");
        }
        // The popup rectangle comes back too.
        let p = got[1].popup.map(|v| v * s);
        assert!(p.iter().zip([312.0, 190.0, 552.0, 330.0]).all(|(a, b)| (a - b).abs() < 1.0), "{p:?}");
    }
}

#[test]
fn notes_are_added_as_an_incremental_update() {
    let mut d = doc_with(Vec::new(), 72.0);
    let plain = pdf_of(&d);
    assert!(!plain.windows(7).any(|w| w == b"/Annots"), "no notes, no annotations");
    d.notes = three();
    let with = pdf_of(&d);
    assert!(with.starts_with(&plain), "the writer's own bytes stay as they were");
    assert!(with[plain.len()..].windows(5).any(|w| w == b"/Prev"));
    // Another reader opens it and still sees one page.
    assert_eq!(lopdf::Document::load_mem(&with).unwrap().get_pages().len(), 1);
}

#[test]
fn a_pdf_without_text_annotations_gives_no_notes() {
    let (notes, height) = read_notes(&pdf_of(&doc_with(Vec::new(), 72.0))).unwrap();
    assert!(notes.is_empty());
    assert_eq!(height, 400.0);
    // A link annotation is not a note.
    let pdf = hand_made(b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] >>");
    assert!(read_notes(&pdf).unwrap().0.is_empty());
}

#[test]
fn another_programs_text_annotation_is_read_from_the_crop_box() {
    // A literal (PDFDocEncoding) string, no popup, no colour, a crop box offset from the origin.
    let pdf = hand_made(b"<< /Type /Annot /Subtype /Text /Rect [150 650 170 670] /Contents (Check this) /T (Rev) >>");
    let (notes, height) = read_notes(&pdf).unwrap();
    assert_eq!(height, 700.0);
    assert_eq!(notes.len(), 1);
    let n = &notes[0];
    assert_eq!((n.text.as_str(), n.author.as_str()), ("Check this", "Rev"));
    // Crop box [50 50 550 750]: x from its left, y down from its top.
    assert_eq!(n.position, [100.0, 80.0]);
    assert_eq!(n.color, Note::default().color);
    assert!(n.modified.is_empty());
}

#[test]
fn hostile_or_truncated_input_never_panics() {
    let good = pdf_of(&doc_with(three(), 72.0));
    for cut in (0..good.len()).step_by(97).chain([good.len() - 1, good.len() - 30]) {
        let _ = read_notes(&good[..cut]);
    }
    for junk in [&b""[..], b"%PDF-1.7", b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 9 0 R >> endobj\ntrailer << /Root 1 0 R >>", b"not a pdf at all"] {
        let _ = read_notes(junk);
    }
    // Broken annotation entries are skipped.
    let pdf = hand_made(b"<< /Type /Annot /Subtype /Text /Rect [1 2] /Contents 7 /C [1 (x) 2] >>");
    assert!(read_notes(&pdf).unwrap().0.is_empty(), "a text annotation without a usable /Rect is no note");
    let pdf = hand_made(b"<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents 7 /C [1 (x) 2] /Popup 99 0 R >>");
    let n = &read_notes(&pdf).unwrap().0[0];
    assert!(n.text.is_empty());
    assert_eq!(n.color, Note::default().color);
}

/// A one-page PDF (crop box [50 50 550 750] on a 600 × 800 page) whose page has `annot` as its
/// only annotation, with a correct cross-reference table.
fn hand_made(annot: &[u8]) -> Vec<u8> {
    let objects: [&[u8]; 4] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /CropBox [50 50 550 750] /Annots [4 0 R] >>",
        annot,
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}
