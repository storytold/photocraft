use std::io::Write as _;

use photocraft_color::{ColorMode, SampleType};
use photocraft_raster::Interrupt;

use super::*;
use crate::IoError;

/// A page of a hand-written test PDF.
struct TestPage<'a> {
    w: f32,
    h: f32,
    content: &'a str,
    /// Extra page dictionary entries (`/Rotate 90 /CropBox [...]`).
    extra: &'a str,
    /// Extra resource dictionary entries (`/ExtGState << ... >>`).
    resources: &'a str,
}

fn page<'a>(w: f32, h: f32, content: &'a str) -> TestPage<'a> {
    TestPage { w, h, content, extra: "", resources: "" }
}

/// Objects 1–3 are the catalog, the page tree and Helvetica; each page adds its content stream
/// and page dictionary.
fn pdf(pages: &[TestPage]) -> Vec<u8> {
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    for (i, p) in pages.iter().enumerate() {
        objs.push(stream("", p.content.as_bytes()));
        objs.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] {} /Resources << /Font << /F1 3 0 R >> {} >> /Contents {} 0 R >>",
                p.w,
                p.h,
                p.extra,
                p.resources,
                4 + 2 * i
            )
            .into_bytes(),
        );
    }
    write(objs)
}

fn stream(extra: &str, data: &[u8]) -> Vec<u8> {
    let mut o = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    o.extend_from_slice(data);
    o.extend_from_slice(b"\nendstream");
    o
}

fn write(objs: Vec<Vec<u8>>) -> Vec<u8> {
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

/// A page drawing Flate-compressed images: `(width, height, gray, matrix)` each, plus `extra`
/// content after them.
fn image_pdf(w: f32, h: f32, images: &[(u32, u32, bool, &str)], extra: &str) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()];
    // 3 = page, 4 = content, 5.. = images
    let mut xobjects = String::new();
    let mut content = String::new();
    for (i, (iw, ih, gray, m)) in images.iter().enumerate() {
        xobjects.push_str(&format!("/Im{i} {} 0 R ", 5 + i));
        content.push_str(&format!("q {m} cm /Im{i} Do Q\n"));
        let n = if *gray { 1 } else { 3 };
        let px: Vec<u8> = (0..ih * iw * n).map(|k| (k * 7 % 251) as u8).collect();
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&px).unwrap();
        let data = enc.finish().unwrap();
        objs.push(Vec::new()); // placeholder, filled below in order
        let cs = if *gray { "/DeviceGray" } else { "/DeviceRGB" };
        let idx = objs.len() - 1;
        objs[idx] =
            stream(&format!("/Type /XObject /Subtype /Image /Width {iw} /Height {ih} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode"), &data);
    }
    content.push_str(extra);
    let page = format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Resources << /XObject << {xobjects} >> >> /Contents 4 0 R >>").into_bytes();
    objs.insert(2, page);
    objs.insert(3, stream("", content.as_bytes()));
    write(objs)
}

fn rect(x: f32, y: f32, w: f32, h: f32, rgb: (f32, f32, f32)) -> String {
    format!("{} {} {} rg {x} {y} {w} {h} re f\n", rgb.0, rgb.1, rgb.2)
}

fn open(bytes: &[u8], opts: PdfOpenOptions) -> ImportResult {
    import_pdf("test.pdf", bytes, &opts, &Interrupt::NONE).expect("import")
}

fn at72(crop: CropTo) -> PdfOpenOptions {
    PdfOpenOptions { crop, resolution: 72.0, ..Default::default() }
}

fn pixel(r: &ImportResult, x: i32, y: i32) -> Vec<f32> {
    r.document.layers[0].surface().expect("raster layer").pixel(x, y)
}

#[test]
fn detects_pdf_by_its_header() {
    assert!(is_pdf(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n1 0 obj"));
    assert!(is_pdf(b"\xef\xbb\xbfjunk before the header\n%PDF-1.4\n"));
    assert!(!is_pdf(b"\x89PNG\r\n\x1a\n"));
    assert!(!is_pdf(b""));
    assert!(!is_pdf(b"<svg/>"));
}

#[test]
fn pixel_size_rounds_half_up_as_photoshop() {
    assert_eq!(pixel_size((595.276, 841.89), 300.0), (2480, 3508));
    assert_eq!(pixel_size((595.276, 841.89), 72.0), (595, 842));
    assert_eq!(pixel_size((595.276, 841.89), 600.0), (4961, 7016));
    assert_eq!(pixel_size((100.5, 200.25), 72.0), (101, 200));
    assert_eq!(pixel_size((100.5, 200.25), 300.0), (419, 834));
    assert_eq!(pixel_size((123.4, 77.7), 300.0), (514, 324));
    assert_eq!(pixel_size((0.0, -5.0), 300.0), (1, 1));
}

#[test]
fn document_names_follow_photoshop() {
    assert_eq!(page_name("single.pdf", None, 1, 1), "single");
    assert_eq!(page_name("/tmp/multi.pdf", None, 2, 3), "multi-2.pdf");
    assert_eq!(page_name("multi.pdf", Some("Foo"), 2, 3), "Foo-2.pdf");
    assert_eq!(page_name("single.pdf", Some("Foo"), 1, 1), "Foo");
    assert_eq!(page_name("no-extension", None, 1, 1), "no-extension");
    assert_eq!(page_name("a.b.pdf", Some("  "), 1, 1), "a.b");
}

#[test]
fn crop_choices_round_trip_their_ids() {
    for c in CropTo::ALL {
        assert_eq!(CropTo::from_id(c.id()), Some(c));
        assert_eq!(CropTo::from_id(c.label()), Some(c));
    }
    assert_eq!(CropTo::from_id("TRIMBOX"), Some(CropTo::TrimBox));
    assert_eq!(CropTo::from_id("fold"), None);
}

#[test]
fn page_boxes_fall_back_to_the_crop_and_media_boxes() {
    let boxed = pdf(&[TestPage {
        w: 612.0,
        h: 792.0,
        content: "",
        extra: "/CropBox [36 36 576 756] /BleedBox [50 50 560 740] /TrimBox [72 72 540 720] /ArtBox [100 100 500 700]",
        resources: "",
    }]);
    let i = info(&boxed, None).unwrap();
    let p = &i.pages[0];
    assert_eq!(p.media, [0.0, 0.0, 612.0, 792.0]);
    assert_eq!(p.crop, [36.0, 36.0, 576.0, 756.0]);
    assert_eq!(p.bleed, [50.0, 50.0, 560.0, 740.0]);
    assert_eq!(p.trim, [72.0, 72.0, 540.0, 720.0]);
    assert_eq!(p.art, [100.0, 100.0, 500.0, 700.0]);
    assert_eq!(p.displayed_size(p.box_for(CropTo::TrimBox)), (468.0, 648.0));

    let plain = pdf(&[page(400.0, 300.0, "")]);
    let p = &info(&plain, None).unwrap().pages[0];
    for c in [CropTo::CropBox, CropTo::BleedBox, CropTo::TrimBox, CropTo::ArtBox] {
        assert_eq!(p.box_for(c), [0.0, 0.0, 400.0, 300.0], "{c:?} falls back to the media box");
    }

    // A crop box larger than the media box, and a media box away from the origin.
    let offset = pdf(&[TestPage { w: 1.0, h: 1.0, content: "", extra: "/MediaBox [100 100 500 400] /CropBox [50 50 600 500]", resources: "" }]);
    let p = &info(&offset, None).unwrap().pages[0];
    assert_eq!(p.media, [100.0, 100.0, 500.0, 400.0]);
    assert_eq!(p.crop, [100.0, 100.0, 500.0, 400.0]);
}

#[test]
fn bounding_box_is_the_geometry_of_the_marks() {
    let letter = |content: &str, resources: &str| pdf(&[TestPage { w: 612.0, h: 792.0, content, extra: "", resources }]);
    let bb = |bytes: &[u8]| bounding_box(bytes, 1, None).unwrap();
    // A fill: its exact bounds; the same under a 2× CTM.
    assert_eq!(bb(&letter(&rect(100.0, 292.0, 300.0, 350.0, (1.0, 0.0, 0.0)), "")), [100.0, 292.0, 400.0, 642.0]);
    assert_eq!(bb(&letter("q 2 0 0 2 0 0 cm 1 0 0 rg 50 146 150 175 re f Q\n", "")), [100.0, 292.0, 400.0, 642.0]);
    // Strokes grow by max(lw / 2, 4) + 2 points.
    assert_eq!(bb(&letter("0 0 1 RG 20 w 100 292 300 350 re S\n", "")), [88.0, 280.0, 412.0, 654.0]);
    assert_eq!(bb(&letter("0 0 1 RG 1 w 100 292 300 350 re S\n", "")), [94.0, 286.0, 406.0, 648.0]);
    assert_eq!(bb(&letter("0 0 1 RG 20 w 1 J 100 400 m 400 400 l S\n", "")), [88.0, 388.0, 412.0, 412.0]);
    // Clipped content counts within its clip; content past the media box is cut to it.
    assert_eq!(bb(&letter("q 200 200 100 100 re W n 1 0 0 rg 0 0 612 792 re f Q\n", "")), [200.0, 200.0, 300.0, 300.0]);
    assert_eq!(bb(&letter(&rect(-100.0, -100.0, 300.0, 300.0, (0.0, 0.5, 0.0)), "")), [0.0, 0.0, 200.0, 200.0]);
    // White and fully transparent fills count; an empty page is its crop box.
    assert_eq!(bb(&letter(&rect(0.0, 0.0, 612.0, 792.0, (1.0, 1.0, 1.0)), "")), [0.0, 0.0, 612.0, 792.0]);
    let alpha0 = letter(
        &format!("q /G0 gs {}Q {}", rect(50.0, 50.0, 500.0, 700.0, (1.0, 0.0, 0.0)), rect(200.0, 200.0, 100.0, 100.0, (0.0, 0.0, 0.0))),
        "/ExtGState << /G0 << /ca 0 /CA 0 >> >>",
    );
    assert_eq!(bb(&alpha0), [50.0, 50.0, 550.0, 750.0]);
    assert_eq!(bb(&letter("", "")), [0.0, 0.0, 612.0, 792.0]);
    // Text: Photoshop's generous per-glyph box ("H" at 48 pt measured 166 × 85).
    let [x0, y0, x1, y1] = bb(&letter("BT /F1 48 Tf 100 400 Td (H) Tj ET\n", ""));
    assert!((x1 - x0 - 166.1).abs() < 1.0 && (y1 - y0 - 85.0).abs() < 1.0, "{x0} {y0} {x1} {y1}");
    assert!((x0 - 49.6).abs() < 0.6 && (y0 - 374.6).abs() < 0.6, "{x0} {y0}");
    // Invisible text counts too.
    let [x0, _, x1, _] = bb(&letter("BT 3 Tr /F1 40 Tf 100 600 Td (hidden) Tj ET\n", ""));
    assert!(x0 < 60.0 && x1 > 200.0, "{x0} {x1}");
    // An image: its placement.
    let img = image_pdf(612.0, 792.0, &[(100, 50, false, "200 0 0 100 100 100")], &rect(10.0, 10.0, 20.0, 20.0, (0.0, 0.0, 0.0)));
    assert_eq!(bb(&img), [10.0, 10.0, 300.0, 200.0]);
}

#[test]
fn a_page_opens_as_a_transparent_layer_1() {
    let bytes = pdf(&[page(400.0, 400.0, &rect(100.0, 100.0, 200.0, 200.0, (1.0, 0.0, 0.0)))]);
    let r = import_pdf("square.pdf", &bytes, &at72(CropTo::MediaBox), &Interrupt::NONE).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (400, 400));
    assert_eq!(d.name, "square");
    assert_eq!(d.resolution_dpi, 72.0);
    assert_eq!((d.mode, d.depth), (ColorMode::Rgb, SampleType::U8));
    assert!(d.icc_profile.is_none(), "the working space applies");
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.layers[0].name, "Layer 1");
    assert!(!d.layers[0].locks.transparency, "not a Background layer");
    assert!(r.source_read_only, "Save must not write back into the PDF");
    assert_eq!(pixel(&r, 5, 5)[3], 0.0, "nothing painted: transparent");
    let p = pixel(&r, 200, 200);
    assert!(p[0] > 0.99 && p[1] < 0.01 && p[3] > 0.99, "{p:?}");
    // The bounding box crop keeps only the square.
    let r = open(&bytes, at72(CropTo::BoundingBox));
    assert_eq!((r.document.size.width, r.document.size.height), (200, 200));
    // 300 ppi by default.
    let r = open(&bytes, PdfOpenOptions { crop: CropTo::MediaBox, ..Default::default() });
    assert_eq!((r.document.size.width, r.document.size.height, r.document.resolution_dpi), (1667, 1667, 300.0));
}

#[test]
fn pages_of_a_multi_page_file_are_named_and_counted() {
    let bytes = pdf(&[page(612.0, 792.0, ""), page(595.0, 420.0, ""), page(300.0, 300.0, "")]);
    let r = import_pdf("multi.pdf", &bytes, &PdfOpenOptions { page: 2, ..at72(CropTo::MediaBox) }, &Interrupt::NONE).unwrap();
    assert_eq!(r.document.name, "multi-2.pdf");
    assert_eq!((r.document.size.width, r.document.size.height), (595, 420));
    let r = import_pdf("multi.pdf", &bytes, &PdfOpenOptions { page: 3, name: Some("Foo".into()), ..at72(CropTo::MediaBox) }, &Interrupt::NONE).unwrap();
    assert_eq!(r.document.name, "Foo-3.pdf");
    for bad in [0, 4, usize::MAX] {
        let e = import_pdf("multi.pdf", &bytes, &PdfOpenOptions { page: bad, ..Default::default() }, &Interrupt::NONE).expect_err("no such page");
        assert!(matches!(e, IoError::Pdf(_)), "{e}");
    }
    assert_eq!(info(&bytes, None).unwrap().pages.len(), 3);
}

#[test]
fn rotation_turns_the_page_like_hayro_and_photoshop() {
    for (rot, (w, h)) in [(0, (612, 792)), (90, (792, 612)), (180, (612, 792)), (270, (792, 612))] {
        // A mark in the top-left corner of the unrotated page.
        let bytes =
            pdf(&[TestPage { w: 612.0, h: 792.0, content: &rect(0.0, 772.0, 20.0, 20.0, (0.0, 0.0, 0.0)), extra: &format!("/Rotate {rot}"), resources: "" }]);
        let r = open(&bytes, at72(CropTo::MediaBox));
        assert_eq!((r.document.size.width, r.document.size.height), (w, h), "rotate {rot}");
        let (x, y) = match rot {
            90 => (w as i32 - 5, 5),
            180 => (w as i32 - 5, h as i32 - 5),
            270 => (5, h as i32 - 5),
            _ => (5, 5),
        };
        assert_eq!(pixel(&r, x, y)[3], 1.0, "rotate {rot}: the mark moved with the page");
        assert_eq!(pixel(&r, w as i32 / 2, h as i32 / 2)[3], 0.0);
    }
}

#[test]
fn crop_choices_use_the_page_boxes() {
    let bytes = pdf(&[TestPage {
        w: 612.0,
        h: 792.0,
        content: &rect(20.0, 20.0, 572.0, 752.0, (0.8, 0.9, 1.0)),
        extra: "/CropBox [36 36 576 756] /BleedBox [50 50 560 740] /TrimBox [72 72 540 720] /ArtBox [100 100 500 700]",
        resources: "",
    }]);
    let size = |c| {
        let d = open(&bytes, at72(c)).document;
        (d.size.width, d.size.height)
    };
    assert_eq!(size(CropTo::MediaBox), (612, 792));
    assert_eq!(size(CropTo::CropBox), (540, 720));
    assert_eq!(size(CropTo::BleedBox), (510, 690));
    assert_eq!(size(CropTo::TrimBox), (468, 648));
    assert_eq!(size(CropTo::ArtBox), (400, 600));
    // The content reaches past the crop box: the bounding box is cut to the crop box.
    assert_eq!(size(CropTo::BoundingBox), (540, 720));
    // The trim box's top-left pixel is painted (the fill starts at 20 pt, the trim box at 72).
    let r = open(&bytes, at72(CropTo::TrimBox));
    assert_eq!(pixel(&r, 0, 0)[3], 1.0);
}

#[test]
fn modes_depths_and_device_colours() {
    let cmyk_red = pdf(&[page(100.0, 100.0, "0 1 1 0 k 0 0 100 100 re f\n")]);
    // DeviceCMYK red opens as Photoshop's SWOP conversion (237, 28, 36), within a few levels.
    let r = open(&cmyk_red, at72(CropTo::MediaBox));
    let p = pixel(&r, 50, 50);
    let want = [237.0 / 255.0, 28.0 / 255.0, 36.0 / 255.0];
    for (got, want) in p.iter().zip(want) {
        assert!((got - want).abs() < 3.0 / 255.0, "{p:?}");
    }
    // CMYK mode: four inks plus alpha, red is magenta + yellow.
    let r = open(&cmyk_red, PdfOpenOptions { mode: ColorMode::Cmyk, ..at72(CropTo::MediaBox) });
    assert_eq!(r.document.mode, ColorMode::Cmyk);
    let p = pixel(&r, 50, 50);
    assert_eq!(p.len(), 5);
    assert!(p[0] < 0.1 && p[1] > 0.8 && p[2] > 0.8 && p[3] < 0.1 && p[4] > 0.99, "{p:?}");
    // Grayscale and Lab modes carry their own channels; 16 bits keep the depth.
    let r = open(&cmyk_red, PdfOpenOptions { mode: ColorMode::Grayscale, depth: SampleType::U16, ..at72(CropTo::MediaBox) });
    assert_eq!((r.document.mode, r.document.depth), (ColorMode::Grayscale, SampleType::U16));
    let p = pixel(&r, 50, 50);
    assert_eq!(p.len(), 2);
    assert!(p[0] > 0.2 && p[0] < 0.7, "{p:?}");
    let r = open(&cmyk_red, PdfOpenOptions { mode: ColorMode::Lab, ..at72(CropTo::MediaBox) });
    assert_eq!(r.document.mode, ColorMode::Lab);
    assert_eq!(pixel(&r, 50, 50).len(), 4);
    // Modes a PDF can't open in fall back to RGB; odd depths to 8 bits.
    let r = open(&cmyk_red, PdfOpenOptions { mode: ColorMode::Indexed, depth: SampleType::F32, ..at72(CropTo::MediaBox) });
    assert_eq!((r.document.mode, r.document.depth), (ColorMode::Rgb, SampleType::U8));
}

#[test]
fn anti_aliasing_can_be_turned_off() {
    let triangle = pdf(&[page(100.0, 100.0, "0 0 0 rg 10 10 m 90 15 l 40 90 l h f\n")]);
    let smooth = open(&triangle, at72(CropTo::MediaBox));
    let hard = open(&triangle, PdfOpenOptions { anti_alias: false, ..at72(CropTo::MediaBox) });
    let alphas = |r: &ImportResult| (0..100).flat_map(|y| (0..100).map(move |x| (x, y))).map(|(x, y)| pixel(r, x, y)[3]).collect::<Vec<_>>();
    assert!(alphas(&smooth).iter().any(|a| *a > 0.0 && *a < 1.0), "anti-aliased edges have partial coverage");
    assert!(alphas(&hard).iter().all(|a| *a == 0.0 || *a == 1.0), "without anti-aliasing every pixel is in or out");
    assert!(alphas(&hard).iter().filter(|a| **a == 1.0).count() > 1000, "the triangle is still there");
}

#[test]
fn a_page_that_is_one_image_opens_as_that_image() {
    let bytes = image_pdf(612.0, 792.0, &[(100, 50, false, "200 0 0 100 100 100")], "");
    let r = import_pdf("scan.pdf", &bytes, &PdfOpenOptions::default(), &Interrupt::NONE).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (100, 50), "the image's own pixels");
    assert_eq!(d.name, "scan.pdf", "named like an image file, extension kept");
    assert_eq!(d.resolution_dpi, 36.0, "100 px over 200 pt");
    assert_eq!(d.mode, ColorMode::Rgb);
    assert_eq!(d.layers[0].name, "Background");
    assert!(d.layers[0].locks.transparency && d.layers[0].locks.position);
    assert!(!r.source_read_only, "it is the file's image: Save As defaults to it like any image");
    assert_eq!(pixel(&r, 0, 0)[3], 1.0);
    // A gray image gives a Grayscale document; the matrix's rotation is ignored.
    let gray = image_pdf(300.0, 300.0, &[(64, 64, true, "0 64 -64 0 100 50")], "");
    let d = import_pdf("g.pdf", &gray, &PdfOpenOptions::default(), &Interrupt::NONE).unwrap().document;
    assert_eq!((d.size.width, d.size.height, d.mode, d.resolution_dpi), (64, 64, ColorMode::Grayscale, 72.0));
    // Anything else on the page, or a second image, and the page rasterises as usual.
    let with_rect = image_pdf(400.0, 300.0, &[(100, 50, false, "200 0 0 100 100 100")], &rect(10.0, 10.0, 50.0, 50.0, (1.0, 0.0, 0.0)));
    let d = open(&with_rect, at72(CropTo::MediaBox)).document;
    assert_eq!((d.size.width, d.size.height), (400, 300));
    assert_eq!(d.layers[0].name, "Layer 1");
    let two = image_pdf(400.0, 300.0, &[(100, 50, false, "200 0 0 100 20 150"), (64, 64, true, "64 0 0 64 300 50")], "");
    assert_eq!(open(&two, at72(CropTo::MediaBox)).document.layers[0].name, "Layer 1");
    assert!(opens_as_image(&bytes, None) && !opens_as_image(&two, None) && !opens_as_image(&with_rect, None));
}

#[test]
fn images_mode_lists_and_opens_the_embedded_images() {
    let two =
        image_pdf(400.0, 300.0, &[(100, 50, false, "200 0 0 100 20 150"), (64, 64, true, "64 0 0 64 300 50")], &rect(0.0, 0.0, 10.0, 10.0, (1.0, 0.0, 0.0)));
    let list = images(&two, None).unwrap();
    assert_eq!(list, vec![PdfImage { index: 1, page: 1, width: 100, height: 50 }, PdfImage { index: 2, page: 1, width: 64, height: 64 }]);
    let r = import_pdf("pair.pdf", &two, &PdfOpenOptions { image: Some(2), ..Default::default() }, &Interrupt::NONE).unwrap();
    let d = &r.document;
    assert_eq!((d.name.as_str(), d.size.width, d.size.height, d.mode), ("pair-2", 64, 64, ColorMode::Grayscale));
    assert_eq!(d.layers[0].name, "Background");
    assert!(r.source_read_only, "untitled, as in Photoshop: not tied to the PDF");
    let r = import_pdf("pair.pdf", &two, &PdfOpenOptions { image: Some(1), name: Some("Foo.pdf".into()), ..Default::default() }, &Interrupt::NONE).unwrap();
    assert_eq!((r.document.name.as_str(), r.document.size.width, r.document.resolution_dpi), ("Foo-1", 100, 36.0));
    assert!(import_pdf("pair.pdf", &two, &PdfOpenOptions { image: Some(3), ..Default::default() }, &Interrupt::NONE).is_err());
    let t = image_thumbnail(&two, 1, 50, None).unwrap();
    assert_eq!((t.width, t.height), (50, 25));
    // Suppress Warnings drops the import notes.
    let big = pdf(&[page(14400.0, 100.0, "")]);
    let r = open(&big, PdfOpenOptions { crop: CropTo::MediaBox, suppress_warnings: true, ..Default::default() });
    assert!(r.warnings.is_empty());
}

#[test]
fn oversized_pages_are_scaled_down_with_a_warning() {
    let bytes = pdf(&[page(14400.0, 100.0, &rect(0.0, 0.0, 14400.0, 100.0, (0.0, 0.0, 1.0)))]);
    let r = open(&bytes, PdfOpenOptions { crop: CropTo::MediaBox, resolution: 300.0, ..Default::default() });
    assert!(r.document.size.width <= MAX_SIDE && r.document.size.height >= 1);
    assert_eq!(r.document.size.width, MAX_SIDE);
    assert!(r.warnings.iter().any(|w| w.contains("scaled down")), "{:?}", r.warnings);
    // An explicit pixel size is honoured, within the same limits.
    let r = open(&bytes, PdfOpenOptions { crop: CropTo::MediaBox, pixel_size: Some((1440, 10)), ..Default::default() });
    assert_eq!((r.document.size.width, r.document.size.height), (1440, 10));
    assert!(r.warnings.is_empty());
}

#[test]
fn damaged_files_are_errors_not_panics() {
    let good = pdf(&[page(100.0, 100.0, "")]);
    for bad in [&good[..good.len() / 2], b"%PDF-1.7 nothing else", b"", b"\x89PNG"] {
        let e = import_pdf("x.pdf", bad, &PdfOpenOptions::default(), &Interrupt::NONE).expect_err("an error");
        assert!(matches!(e, IoError::Pdf(_)), "{e}");
        // hayro recovers what it can: a page list, possibly empty, or an error.
        assert!(info(bad, None).map(|i| i.pages.is_empty()).unwrap_or(true));
        assert!(!needs_password(bad));
    }
    // A hostile resolution is clamped, not trusted.
    let r = open(&good, PdfOpenOptions { crop: CropTo::MediaBox, resolution: f32::NAN, ..Default::default() });
    assert_eq!(r.document.resolution_dpi, DEFAULT_RESOLUTION);
    let r = open(&good, PdfOpenOptions { crop: CropTo::MediaBox, resolution: -5.0, ..Default::default() });
    assert_eq!((r.document.size.width, r.document.resolution_dpi), (1, MIN_RESOLUTION));
}

#[test]
fn thumbnails_fit_the_asked_side() {
    let bytes = pdf(&[TestPage { w: 612.0, h: 792.0, content: "", extra: "/Rotate 90", resources: "" }]);
    let t = thumbnail(&bytes, 1, 128, None).unwrap();
    assert_eq!((t.width, t.height), (128, 99), "the rotated page is landscape");
    assert_eq!(t.rgba.len(), 128 * 99 * 4);
    assert!(thumbnail(&bytes, 2, 128, None).is_err());
}

/// Without Import PDF answers every page opens as an artboard (#11647); with them, the page the
/// dialog chose opens on its own.
#[test]
fn plain_import_opens_pages_as_artboards_and_answers_open_one_page() {
    let bytes = pdf(&[page(72.0, 72.0, &rect(0.0, 0.0, 72.0, 72.0, (0.0, 0.0, 0.0))), page(36.0, 36.0, "")]);
    let r = crate::import("two.pdf", &bytes).unwrap();
    let boards: Vec<_> = r
        .document
        .layers
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Group(g) => g.artboard.as_ref().map(|a| (a.rect.width(), a.rect.height())),
            _ => None,
        })
        .collect();
    assert_eq!(boards, vec![(300, 300), (150, 150)], "one artboard a page, at 300 ppi");
    assert_eq!(r.document.resolution_dpi, 300.0);
    let opts =
        crate::ImportOptions { pdf: Some(PdfOpenOptions { page: 2, crop: CropTo::MediaBox, resolution: 72.0, ..Default::default() }), ..Default::default() };
    let r = crate::import_with_options("two.pdf", &bytes, &opts).unwrap();
    assert_eq!((r.document.name.as_str(), r.document.size.width), ("two-2.pdf", 36));
}

// ---- Image budgets: refused from the file's declarations, before anything is decoded. ----

/// A one-page PDF whose objects 5.. are `xobjects` (dictionary text and stream data each); the
/// first `named` of them are `/X0`, `/X1`… in the page's resources (the rest are reached only
/// through the others, as masks or a form's resources), drawn by `content`.
fn xobject_pdf(xobjects: &[(&str, &[u8])], named: usize, content: &[u8]) -> Vec<u8> {
    let names: String = (0..xobjects.len().min(named)).map(|i| format!("/X{i} {} 0 R ", 5 + i)).collect();
    let mut objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /XObject << {names} >> >> /Contents 4 0 R >>").into_bytes(),
        stream("", content),
    ];
    for (dict, data) in xobjects {
        objs.push(stream(dict, data));
    }
    write(objs)
}

/// [`xobject_pdf`] with a tiny Flate stream behind every dictionary.
fn declared_pdf(xobjects: &[&str], named: usize, content: &str) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&[0u8; 16]).unwrap();
    let data = enc.finish().unwrap();
    let with_data: Vec<(&str, &[u8])> = xobjects.iter().map(|x| (*x, data.as_slice())).collect();
    xobject_pdf(&with_data, named, content.as_bytes())
}

const HUGE: &str = "/Type /XObject /Subtype /Image /Width 40000 /Height 40000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode";

fn refused(bytes: &[u8], what: &str) -> String {
    let e = import_pdf("big.pdf", bytes, &PdfOpenOptions::default(), &Interrupt::NONE).err().unwrap_or_else(|| panic!("{what}: opened")).to_string();
    assert!(e.contains("can't be opened") || e.contains("can't be bounded"), "{what}: {e}");
    e
}

#[test]
fn an_oversized_image_refuses_the_page_before_decoding() {
    // 40000 × 40000 px declared, 16 bytes of data: the decoders would build 1.6 GP.
    let bytes = declared_pdf(&[HUGE], 1, "q 200 0 0 200 0 0 cm /X0 Do Q");
    let e = refused(&bytes, "page");
    assert!(e.contains("40000 × 40000"), "{e}");
    assert!(thumbnail(&bytes, 1, 64, None).is_err(), "the thumbnail decodes the same image");
    assert!(opens_as_image(&bytes, None), "it is a one-image page as far as the file says");
    assert_eq!(images(&bytes, None).unwrap().len(), 1, "listing images decodes nothing");
    assert!(import_image("big.pdf", &bytes, 1, None, None, &Interrupt::NONE).is_err(), "Images mode refuses it too");
    assert!(image_thumbnail(&bytes, 1, 64, None).is_err());
    // At the limit it is allowed by the declaration (the data then decides).
    let edge = "/Type /XObject /Subtype /Image /Width 8000 /Height 8000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode";
    assert!(preflight(page_of(&load(&declared_pdf(&[edge], 1, "q 200 0 0 200 0 0 cm /X0 Do Q"), None).unwrap(), 1).unwrap()).is_ok());
}

#[test]
fn an_oversized_mask_or_a_nested_or_unsized_image_refuses_too() {
    let small =
        |mask: &str| format!("/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode {mask}");
    // A soft mask of its own size (object 6, reached only through the image).
    let e = refused(
        &declared_pdf(
            &[
                &small("/SMask 6 0 R"),
                "/Type /XObject /Subtype /Image /Width 40000 /Height 40000 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode",
            ],
            1,
            "q 200 0 0 200 0 0 cm /X0 Do Q",
        ),
        "soft mask",
    );
    assert!(e.contains("soft mask"), "{e}");
    // An explicit mask.
    let e = refused(
        &declared_pdf(
            &[&small("/Mask 6 0 R"), "/Type /XObject /Subtype /Image /Width 40000 /Height 40000 /ImageMask true /BitsPerComponent 1 /Filter /FlateDecode"],
            1,
            "q 200 0 0 200 0 0 cm /X0 Do Q",
        ),
        "mask",
    );
    assert!(e.contains("a mask"), "{e}");
    // Inside a Form XObject's resources, even when the content draws nothing at all.
    let form = "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Im 6 0 R >> >>";
    let e = refused(&declared_pdf(&[form, HUGE], 1, ""), "form");
    assert!(e.contains("40000 × 40000"), "{e}");
    // An image without its size can't be bounded.
    let e = refused(
        &declared_pdf(&["/Type /XObject /Subtype /Image /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode"], 1, "q 200 0 0 200 0 0 cm /X0 Do Q"),
        "unsized",
    );
    assert!(e.contains("doesn't declare its size"), "{e}");
    // Too many images for one page together: five distinct 64 MP images, one of them drawn.
    let per: Vec<String> = (0..5)
        .map(|i| format!("/Type /XObject /Subtype /Image /Width {} /Height 8000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode", 8000 - i))
        .collect();
    let five: Vec<&str> = per.iter().map(String::as_str).collect();
    let e = refused(&declared_pdf(&five, 5, "q 200 0 0 200 0 0 cm /X0 Do Q"), "total");
    assert!(e.contains("together"), "{e}");
    // An inline image: not in the resources, but placed.
    let inline = "q 200 0 0 200 0 0 cm BI /W 40000 /H 40000 /CS /RGB /BPC 8 /F /Fl ID x EI Q";
    let e = refused(&declared_pdf(&[], 0, inline), "inline");
    assert!(e.contains("40000 × 40000"), "{e}");
    // A normal image still opens, image-only.
    let ok = declared_pdf(
        &["/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode"],
        1,
        "q 200 0 0 200 0 0 cm /X0 Do Q",
    );
    let r = import_pdf("ok.pdf", &ok, &PdfOpenOptions::default(), &Interrupt::NONE).expect("opens");
    assert_eq!((r.document.size.width, r.document.size.height), (2, 2));
}

// ---- JPX: bounded from the codestream header, not only the dictionary; inline images count. ----

/// A 2 × 2 grey JPEG 2000 codestream (OpenJPEG 2.5, lossless 5/3): pixels 10, 200, 90, 30.
const J2K_GRAY_2X2: [u8; 143] = [
    0xff, 0x4f, 0xff, 0x51, 0x00, 0x29, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x01, 0xff, 0x52, 0x00, 0x0c, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x01, 0x04, 0x04, 0x00, 0x01, 0xff, 0x5c, 0x00, 0x07, 0x40, 0x40, 0x48, 0x48, 0x50, 0xff, 0x64, 0x00, 0x25, 0x00, 0x01, 0x43, 0x72, 0x65, 0x61,
    0x74, 0x65, 0x64, 0x20, 0x62, 0x79, 0x20, 0x4f, 0x70, 0x65, 0x6e, 0x4a, 0x50, 0x45, 0x47, 0x20, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x20, 0x32, 0x2e,
    0x35, 0x2e, 0x30, 0xff, 0x90, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x00, 0x01, 0xff, 0x93, 0xc7, 0xd4, 0x04, 0x05, 0xdf, 0xc7, 0xda, 0x05, 0x0f,
    0xa8, 0x0a, 0x3f, 0x00, 0x20, 0x04, 0xaf, 0x05, 0xdf, 0x09, 0xd3, 0xff, 0xd9,
];

/// A JPEG 2000 codestream header alone: SOC, then a SIZ marker segment for a `w × h` image of
/// `components` 8-bit components in one tile. Nothing decodable follows: the budget decides before
/// any decoder runs.
fn jpx_header_only(w: u32, h: u32, components: u16) -> Vec<u8> {
    let mut d = vec![0xFF, 0x4F, 0xFF, 0x51];
    d.extend_from_slice(&(38 + 3 * components).to_be_bytes());
    d.extend_from_slice(&[0, 0]);
    for v in [w, h, 0, 0, w, h, 0, 0] {
        d.extend_from_slice(&v.to_be_bytes());
    }
    d.extend_from_slice(&components.to_be_bytes());
    for _ in 0..components {
        d.extend_from_slice(&[7, 1, 1]);
    }
    d
}

/// `codestream` in a JP2 file: the signature and file type boxes, a `jp2h` with an `ihdr` of
/// `ihdr_size` and a greyscale `colr`, then the `jp2c` box.
fn jp2_file(ihdr_size: (u32, u32), codestream: &[u8]) -> Vec<u8> {
    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(kind);
        b.extend_from_slice(body);
        b
    }
    let mut ihdr = ihdr_size.1.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&ihdr_size.0.to_be_bytes());
    ihdr.extend_from_slice(&[0, 1, 7, 7, 0, 0]);
    let mut jp2h = bx(b"ihdr", &ihdr);
    jp2h.extend(bx(b"colr", &[1, 0, 0, 0, 0, 0, 17]));
    let mut f = bx(b"jP  ", &[0x0D, 0x0A, 0x87, 0x0A]);
    f.extend(bx(b"ftyp", b"jp2 \0\0\0\0jp2 "));
    f.extend(bx(b"jp2h", &jp2h));
    f.extend(bx(b"jp2c", codestream));
    f
}

/// Content drawing one inline image with `dict` and `data`.
fn inline(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut c = format!("q 200 0 0 200 0 0 cm BI {dict} ID ").into_bytes();
    c.extend_from_slice(data);
    c.extend_from_slice(b" EI Q\n");
    c
}

const JPX_10: &str = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode";
const DRAW_X0: &[u8] = b"q 200 0 0 200 0 0 cm /X0 Do Q";

fn passes(bytes: &[u8]) -> bool {
    preflight(page_of(&load(bytes, None).unwrap(), 1).unwrap()).is_ok()
}

#[test]
fn a_jpx_codestream_larger_than_its_dictionary_refuses_the_page() {
    // 10 × 10 px declared, 20000 × 20000 encoded: the decoder would build the codestream's size.
    let big = xobject_pdf(&[(JPX_10, &jpx_header_only(20000, 20000, 1))], 1, DRAW_X0);
    let e = refused(&big, "codestream");
    assert!(e.contains("20000 × 20000") && e.contains("JPX"), "{e}");
    assert!(thumbnail(&big, 1, 64, None).is_err(), "the thumbnail decodes the same image");
    assert!(import_image("big.pdf", &big, 1, None, None, &Interrupt::NONE).is_err(), "Images mode too");
    assert!(image_thumbnail(&big, 1, 64, None).is_err());
    // In a JP2 file either header counts: the codestream's SIZ or the image header.
    let e = refused(&xobject_pdf(&[(JPX_10, &jp2_file((10, 10), &jpx_header_only(20000, 20000, 1)))], 1, DRAW_X0), "jp2c");
    assert!(e.contains("20000 × 20000"), "{e}");
    let e = refused(&xobject_pdf(&[(JPX_10, &jp2_file((20000, 20000), &jpx_header_only(10, 10, 1)))], 1, DRAW_X0), "ihdr");
    assert!(e.contains("20000 × 20000"), "{e}");
    // Components multiply the pixels: 4000 × 4000 px of 64 components.
    let e = refused(&xobject_pdf(&[(JPX_10, &jpx_header_only(4000, 4000, 64))], 1, DRAW_X0), "components");
    assert!(e.contains("64 component"), "{e}");
    assert!(passes(&xobject_pdf(&[(JPX_10, &jpx_header_only(4000, 4000, 4))], 1, DRAW_X0)), "four components are one image");
    // A header that can't be read is refused, not decoded: behind another filter, not a
    // codestream at all, or tiled into millions of tiles.
    let chained = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter [/FlateDecode /JPXDecode]";
    let e = refused(&xobject_pdf(&[(chained, &jpx_header_only(10, 10, 1))], 1, DRAW_X0), "chained");
    assert!(e.contains("can't be checked"), "{e}");
    let e = refused(&xobject_pdf(&[(JPX_10, b"not a codestream")], 1, DRAW_X0), "garbage");
    assert!(e.contains("can't be checked"), "{e}");
    let mut tiled = jpx_header_only(8000, 8000, 1);
    tiled[24..32].copy_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1]); // XTsiz = YTsiz = 1: 64 million tiles
    let e = refused(&xobject_pdf(&[(JPX_10, &tiled)], 1, DRAW_X0), "tiles");
    assert!(e.contains("tiles"), "{e}");
    // A JPX soft mask is bounded the same.
    let masked = "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /SMask 6 0 R";
    let e = refused(&xobject_pdf(&[(masked, &[0u8; 16]), (JPX_10, &jpx_header_only(20000, 20000, 1))], 1, DRAW_X0), "soft mask");
    assert!(e.contains("soft mask") && e.contains("20000 × 20000"), "{e}");
    // Inline images: a raster one is sized from its stream by the content walk, a stencil (whose
    // stream the interpreter doesn't show) by the content scan; in a Form XObject the content
    // never draws, too.
    let e = refused(&xobject_pdf(&[], 0, &inline("/W 10 /H 10 /CS /G /BPC 8 /F /JPXDecode", &jpx_header_only(20000, 20000, 1))), "inline");
    assert!(e.contains("20000 × 20000") && e.contains("inline"), "{e}");
    let e = refused(&xobject_pdf(&[], 0, &inline("/W 10 /H 10 /IM true /F /JPXDecode", &jpx_header_only(20000, 20000, 1))), "inline stencil");
    assert!(e.contains("20000 × 20000"), "{e}");
    let form = "/Type /XObject /Subtype /Form /BBox [0 0 10 10]";
    let e = refused(&xobject_pdf(&[(form, &inline("/W 10 /H 10 /IM true /F /JPXDecode", &jpx_header_only(20000, 20000, 1)))], 1, b""), "form inline");
    assert!(e.contains("20000 × 20000"), "{e}");
}

#[test]
fn a_jpx_image_within_its_dictionary_opens() {
    // A real 2 × 2 JPEG 2000 image, raw and in a JP2 file: the budget passes it, hayro decodes it.
    let dict = "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode";
    for data in [J2K_GRAY_2X2.to_vec(), jp2_file((2, 2), &J2K_GRAY_2X2)] {
        let bytes = xobject_pdf(&[(dict, &data)], 1, DRAW_X0);
        let r = import_pdf("jpx.pdf", &bytes, &PdfOpenOptions::default(), &Interrupt::NONE).expect("opens");
        assert_eq!((r.document.size.width, r.document.size.height, r.document.mode), (2, 2, ColorMode::Grayscale));
        assert_eq!(r.document.layers[0].name, "Background", "a one-image page opens as the image");
        for (x, y, want) in [(0, 0, 10.0), (1, 0, 200.0), (0, 1, 90.0), (1, 1, 30.0)] {
            assert!((pixel(&r, x, y)[0] * 255.0 - want).abs() < 1.5, "({x}, {y}): {:?}", pixel(&r, x, y));
        }
        assert!(thumbnail(&bytes, 1, 32, None).is_ok());
    }
    // A header that agrees with the dictionary passes the budget; the data then decides (hayro
    // can't decode a header alone, draws nothing, and the page opens as a page).
    let dict = "/Type /XObject /Subtype /Image /Width 1000 /Height 1000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode";
    let bytes = xobject_pdf(&[(dict, &jpx_header_only(1000, 1000, 3))], 1, DRAW_X0);
    assert!(passes(&bytes));
    let r = import_pdf("jpx.pdf", &bytes, &PdfOpenOptions::default(), &Interrupt::NONE).expect("not refused");
    assert_eq!(r.document.layers[0].name, "Layer 1");
    // An inline JPX image of the right size opens too.
    let bytes = xobject_pdf(&[], 0, &inline("/W 2 /H 2 /CS /G /BPC 8 /F /JPXDecode", &J2K_GRAY_2X2));
    let r = import_pdf("jpx.pdf", &bytes, &PdfOpenOptions::default(), &Interrupt::NONE).expect("opens");
    assert_eq!((r.document.size.width, r.document.size.height), (2, 2));
}

#[test]
fn inline_images_count_toward_the_page_total() {
    let one = |w: u32| format!("q 10 0 0 10 0 0 cm BI /W {w} /H 8000 /CS /G /BPC 8 /F /Fl ID x EI Q\n");
    // Five inline images of 64 MP, each within the per-image limit, 320 MP together.
    let five: String = (0..5).map(|_| one(8000)).collect();
    let e = refused(&declared_pdf(&[], 0, &five), "five inline");
    assert!(e.contains("together"), "{e}");
    // Three (192 MP) pass.
    let three: String = (0..3).map(|_| one(8000)).collect();
    assert!(passes(&declared_pdf(&[], 0, &three)));
    // An image XObject is decoded every time it is drawn: the same 64 MP image five times.
    let per = "/Type /XObject /Subtype /Image /Width 8000 /Height 8000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode";
    let e = refused(&declared_pdf(&[per], 1, &"q 10 0 0 10 0 0 cm /X0 Do Q\n".repeat(5)), "five drawings");
    assert!(e.contains("together"), "{e}");
    // Drawn once with three inline images: 256 MP, the limit; one more inline image is past it.
    let mixed = format!("q 10 0 0 10 0 0 cm /X0 Do Q\n{three}");
    assert!(passes(&declared_pdf(&[per], 1, &mixed)));
    let e = refused(&declared_pdf(&[per], 1, &format!("{mixed}{}", one(8000))), "mixed");
    assert!(e.contains("together"), "{e}");
    // Inline images in a Form XObject the content never draws count once each; drawn twice, the
    // form's images are decoded twice.
    let form = "/Type /XObject /Subtype /Form /BBox [0 0 10 10]";
    let e = refused(&xobject_pdf(&[(form, five.as_bytes())], 1, b""), "form");
    assert!(e.contains("together"), "{e}");
    assert!(passes(&xobject_pdf(&[(form, three.as_bytes())], 1, b"")));
    let e = refused(&xobject_pdf(&[(form, three.as_bytes())], 1, b"q /X0 Do Q q /X0 Do Q"), "form twice");
    assert!(e.contains("together"), "{e}");
}

/// A chain of `levels` Form XObjects (objects 5..), each drawing the next `fan` times; the page
/// draws the first once.
/// Two direct Type 3 fonts (no object numbers) share glyph 5, which draws `/X`: form 6 in both,
/// with no `/Resources`, so its `/Y` resolves in the font's. `/Y` is leaf 7 in font A and in font
/// B `b_y`: 7 again, or 8, the head of a chain that draws 2^18 forms.
fn shared_glyph_fonts(a_y: usize, b_y: usize) -> Vec<u8> {
    let font = |y: usize| {
        format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [0.1 0 0 0.1 0 0] /CharProcs << /a 5 0 R >> /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [10] /Resources << /XObject << /X 6 0 R /Y {y} 0 R >> >> >>"
        )
    };
    let form = |res: String, content: &str| stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 10 10] {res}"), content.as_bytes());
    let mut objs = vec![stream("", b"10 0 d0 /X Do"), form(String::new(), "/Y Do"), form(String::new(), "0 0 10 10 re f")];
    let levels = 18;
    for i in 0..levels {
        if i + 1 < levels {
            objs.push(form(format!("/Resources << /XObject << /C {} 0 R >> >>", 9 + i), "/C Do\n/C Do\n"));
        } else {
            objs.push(form(String::new(), ""));
        }
    }
    objects_pdf(&format!("/Font << /A {} /B {} >>", font(a_y), font(b_y)), "", "", b"", objs)
}

fn form_chain(levels: usize, fan: usize) -> Vec<u8> {
    let forms = (0..levels)
        .map(|i| {
            let (res, content) = if i + 1 < levels {
                (
                    format!("/Resources << /XObject << /C {} 0 R >> >>", 6 + i),
                    "/C Do
"
                    .repeat(fan),
                )
            } else {
                (String::new(), "0 0 1 rg 0 0 10 10 re f".into())
            };
            stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 10 10] {res}"), content.as_bytes())
        })
        .collect();
    objects_pdf("/XObject << /F 5 0 R >>", "", "", b"/F Do", forms)
}

/// A form without `/Resources` drawn under two direct Type 3 fonts resolves its names in each
/// font's resources, so its children are cached per font: whichever font comes first, the font
/// whose `/Y` is the 2^18 chain is counted with the chain, not with the other font's leaf.
#[test]
fn direct_type3_fonts_keep_their_own_form_children() {
    for (a, b) in [(7, 8), (8, 7)] {
        let e = refused(&shared_glyph_fonts(a, b), "chain behind one of two direct fonts");
        assert!(e.contains("Form XObjects more than"), "{e}");
    }
    assert!(import_pdf("fonts.pdf", &shared_glyph_fonts(7, 7), &PdfOpenOptions::default(), &Interrupt::NONE).is_ok());
}

/// The interpreter bounds how deep forms nest, not how often they're drawn: a form drawing its
/// child twice, 40 levels down, is 2^40 draws, which never finished interpreting. It's refused
/// before anything interprets the page. Ordinary reuse opens.
#[test]
fn form_xobject_fan_out_is_bounded_before_the_page_is_interpreted() {
    let e = refused(&form_chain(30, 2), "2^30 form draws");
    assert!(e.contains("Form XObjects more than"), "{e}");
    // Two forms drawing each other twice: hayro doesn't resolve the reference back into a form
    // being drawn, so the cycle ends at once and the page opens (the walk refuses a loop it can
    // follow).
    let cycle = objects_pdf(
        "/XObject << /F 5 0 R >>",
        "",
        "",
        b"/F Do",
        vec![
            stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /B 6 0 R >> >>", b"/B Do /B Do"),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /A 5 0 R >> >>", b"/A Do /A Do"),
        ],
    );
    assert!(import_pdf("cycle.pdf", &cycle, &PdfOpenOptions::default(), &Interrupt::NONE).is_ok());
    // 2^10 draws in all, well within the cap, open.
    let r = import_pdf("forms.pdf", &form_chain(10, 2), &PdfOpenOptions::default(), &Interrupt::NONE).expect("opens");
    assert_eq!(r.document.layers.len(), 1);
    // Every entry point that interprets the page checks it first, not only the page import.
    let deep = form_chain(30, 2);
    assert!(!opens_as_image(&deep, None));
    assert!(bounding_box(&deep, 1, None).is_err());
    assert!(images(&deep, None).is_err());
    assert!(import_image("deep.pdf", &deep, 1, None, None, &Interrupt::NONE).is_err());
    assert!(thumbnail(&deep, 1, 32, None).is_err());
    // Draws that start outside the page's content count too: here an annotation's appearance
    // draws the chain (objects 5..34), and the page content draws nothing.
    let mut forms: Vec<Vec<u8>> = (0..30)
        .map(|i| {
            let (res, content) = if i + 1 < 30 {
                (format!("/Resources << /XObject << /C {} 0 R >> >>", 6 + i), "/C Do\n/C Do\n".to_string())
            } else {
                (String::new(), String::new())
            };
            stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 10 10] {res}"), content.as_bytes())
        })
        .collect();
    forms.push(stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /F 5 0 R >> >>", b"/F Do"));
    forms.push(b"<< /Type /Annot /Subtype /Square /Rect [0 0 10 10] /AP << /N 35 0 R >> >>".to_vec());
    let annotated = objects_pdf("", "/Annots [36 0 R]", "", b"", forms);
    let e = refused(&annotated, "fan-out in an appearance");
    assert!(e.contains("Form XObjects more than"), "{e}");
    // A tiling pattern in a form's resources, its cell drawing a 2^11 chain: the renderer would
    // interpret it once per fill. Refused per cell, and through the form that holds it.
    let chain = |first: usize, levels: usize| -> Vec<Vec<u8>> {
        (0..levels)
            .map(|i| {
                let (res, content) = if i + 1 < levels {
                    (format!("/Resources << /XObject << /C {} 0 R >> >>", first + 1 + i), "/C Do\n/C Do\n".to_string())
                } else {
                    (String::new(), String::new())
                };
                stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 10 10] {res}"), content.as_bytes())
            })
            .collect()
    };
    // 5: the form the page draws, holding pattern 6; 6: the pattern, drawing chain 7..17.
    let mut objs = vec![
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Pattern << /P 6 0 R >> >>", b"/Pattern cs /P scn 0 0 10 10 re f"),
        stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /XObject << /C 7 0 R >> >>",
            b"/C Do",
        ),
    ];
    objs.extend(chain(7, 11));
    let e = refused(&objects_pdf("/XObject << /F 5 0 R >>", "", "", b"/F Do", objs), "pattern in a form");
    assert!(e.contains("tiling pattern"), "{e}");
    // A form without /Resources draws `/C` from its parent's: the same leaf form under two parents
    // that map /C differently. Parent A (drawn first) maps it to an empty form, parent B to a 2^30
    // chain. Caching the leaf's children by object alone would reuse A's and miss B's chain.
    // Objects: 5 A, 6 B, 7 the shared leaf (no /Resources, draws /C), 8 empty, 9.. the chain.
    let mut objs = vec![
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /L 7 0 R /C 8 0 R >> >>", b"/L Do"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /L 7 0 R /C 9 0 R >> >>", b"/L Do"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", b"/C Do"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", b""),
    ];
    objs.extend(chain(9, 30));
    let e = refused(&objects_pdf("/XObject << /A 5 0 R /B 6 0 R >>", "", "", b"/A Do /B Do", objs), "scoped leaf");
    assert!(e.contains("Form XObjects more than"), "{e}");
    // A pattern cell drawing a few forms opens.
    let mut objs = vec![
        stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Pattern << /P 6 0 R >> >>", b"/Pattern cs /P scn 0 0 10 10 re f"),
        stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /XObject << /C 7 0 R >> >>",
            b"/C Do",
        ),
    ];
    objs.extend(chain(7, 4));
    assert!(import_pdf("pattern.pdf", &objects_pdf("/XObject << /F 5 0 R >>", "", "", b"/F Do", objs), &PdfOpenOptions::default(), &Interrupt::NONE).is_ok());
}

/// `data` in ASCII85 (no `z` shorthand), ended by `~>`.
fn ascii85_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 * 5 + 8);
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        let mut v = u32::from_be_bytes(word);
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = (v % 85) as u8 + b'!';
            v /= 85;
        }
        out.extend_from_slice(&digits[..chunk.len() + 1]);
    }
    out.extend_from_slice(b"~>");
    out
}

#[test]
fn the_ascii_decoders_read_what_the_spec_writes() {
    assert_eq!(super::ascii_hex(b"48 65 6c 6C 6f>ignored").as_deref(), Some(b"Hello".as_slice()));
    assert_eq!(super::ascii_hex(b"7").as_deref(), Some([0x70].as_slice()), "an odd last digit is followed by 0");
    assert_eq!(super::ascii_hex(b"zz"), None);
    for data in [b"".as_slice(), b"a", b"ab", b"abc", b"abcd", b"Hello, ASCII85!", &[0, 0, 0, 0, 255, 255, 255]] {
        assert_eq!(super::ascii85(&ascii85_encode(data)).as_deref(), Some(data), "{data:?}");
    }
    assert_eq!(super::ascii85(b"z~>").as_deref(), Some([0u8; 4].as_slice()));
    assert_eq!(super::ascii85(b"uuuuu~>"), None, "a group past 2^32 - 1");
}

/// `n` bytes of spaces, deflated (zlib): a few hundred KB that inflate to `n`.
fn deflated_spaces(n: usize) -> Vec<u8> {
    use std::io::Write;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    let chunk = vec![b' '; 1 << 20];
    for _ in 0..n / chunk.len() {
        z.write_all(&chunk).unwrap();
    }
    z.finish().unwrap()
}

/// A content stream that inflates past 256 MB is refused before it is decoded (hayro has no
/// decode limit): as the page's content and as a form the page never draws. Ordinary deflated
/// content opens.
#[test]
fn a_content_stream_that_inflates_too_far_is_refused_before_decoding() {
    let bomb = deflated_spaces(300 << 20);
    assert!(bomb.len() < 2 << 20, "the test file stays small: {}", bomb.len());
    let e = refused(&objects_pdf("", "", "/Filter /FlateDecode", &bomb, Vec::new()), "deflate bomb content");
    assert!(e.contains("inflates past 256 MB"), "{e}");
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Filter /FlateDecode", &bomb);
    let e = refused(&objects_pdf("/XObject << /F 5 0 R >>", "", "", b"", vec![form]), "deflate bomb form");
    assert!(e.contains("inflates past"), "{e}");
    // FlateDecode behind ASCII85Decode is measured too; behind LZW it can't be, so it's refused.
    let wrapped = ascii85_encode(&bomb);
    let e = refused(&objects_pdf("", "", "/Filter [/ASCII85Decode /FlateDecode]", &wrapped, Vec::new()), "ASCII85 bomb");
    assert!(e.contains("inflates past"), "{e}");
    let hexed: String = bomb.iter().map(|b| format!("{b:02x}")).collect::<String>() + ">";
    let e = refused(&objects_pdf("", "", "/Filter [/ASCIIHexDecode /FlateDecode]", hexed.as_bytes(), Vec::new()), "ASCIIHex bomb");
    assert!(e.contains("inflates past"), "{e}");
    let e = refused(&objects_pdf("", "", "/Filter [/LZWDecode /FlateDecode]", b"x", Vec::new()), "LZW then Flate");
    assert!(e.contains("decompressed twice"), "{e}");
    // ASCII data the measurement can't read is refused rather than left to a lenient decoder.
    let e = refused(&objects_pdf("", "", "/Filter [/ASCII85Decode /FlateDecode]", b"\x80\x81 not ascii85", Vec::new()), "bad ASCII85");
    assert!(e.contains("can't be read"), "{e}");
    // Deflate twice: the first stage stays under the limit, the second would expand it unmeasured.
    let e = refused(&objects_pdf("", "", "/Filter [/FlateDecode /FlateDecode]", &deflated_spaces(1 << 20), Vec::new()), "Flate then Flate");
    assert!(e.contains("decompressed twice"), "{e}");
    let fine = {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(b"0 0 1 rg 10 10 50 50 re f").unwrap();
        z.finish().unwrap()
    };
    assert!(import_pdf("ok.pdf", &objects_pdf("", "", "/Filter /FlateDecode", &fine, Vec::new()), &PdfOpenOptions::default(), &Interrupt::NONE).is_ok());
}

// ---- CCITT and JBIG2: sized from their parameters and segment headers, not the dictionary. ----

/// A one-page PDF: objects 5.. are `more`, the page has `resources` and `page_extra` (which may
/// give its own `/Contents`), and its content stream (object 4) has `content_dict` entries.
fn objects_pdf(resources: &str, page_extra: &str, content_dict: &str, content: &[u8], more: Vec<Vec<u8>>) -> Vec<u8> {
    let contents = if page_extra.contains("/Contents") { "" } else { "/Contents 4 0 R" };
    let mut objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << {resources} >> {contents} {page_extra} >>").into_bytes(),
        stream(content_dict, content),
    ];
    objs.extend(more);
    write(objs)
}

const CCITT_10: &str = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode";

#[test]
fn a_ccitt_image_is_sized_by_its_decode_parameters() {
    // 10 × 10 px declared, /Columns × /Rows 20000 × 20000: the decoder allocates that up front.
    let parms = |cols: u32, rows: u32| format!("{CCITT_10} /DecodeParms << /K -1 /Columns {cols} /Rows {rows} >>");
    let big = xobject_pdf(&[(&parms(20000, 20000), &[0u8; 8])], 1, DRAW_X0);
    let e = refused(&big, "ccitt");
    assert!(e.contains("CCITT") && e.contains("20000 × 20000"), "{e}");
    assert!(thumbnail(&big, 1, 64, None).is_err() && import_image("big.pdf", &big, 1, None, None, &Interrupt::NONE).is_err());
    // /Rows smaller than /Height: the height counts; no /Columns: 1728.
    let tall = "/Type /XObject /Subtype /Image /Width 10 /Height 9000 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8000 /Rows 1 >>";
    let e = refused(&xobject_pdf(&[(tall, &[0u8; 8])], 1, DRAW_X0), "height");
    assert!(e.contains("8000 × 9000"), "{e}");
    assert!(passes(&xobject_pdf(&[(CCITT_10, &[0u8; 8])], 1, DRAW_X0)), "1728 × 10 by default");
    assert!(passes(&xobject_pdf(&[(&parms(10, 10), &[0u8; 8])], 1, DRAW_X0)));
    // /Columns or /Rows past u32 keep u32::MAX and are refused; `as u32` wrapped 2^32 + 5 to 5.
    let wide = format!("{CCITT_10} /DecodeParms << /K -1 /Columns {} /Rows 10 >>", (1u64 << 32) + 5);
    let e = refused(&xobject_pdf(&[(&wide, &[0u8; 8])], 1, DRAW_X0), "columns past u32");
    assert!(e.contains(&format!("{} ×", u32::MAX)), "{e}");
    let tall_rows = format!("{CCITT_10} /DecodeParms << /K -1 /Columns 10 /Rows {} >>", (1u64 << 32) + 5);
    let e = refused(&xobject_pdf(&[(&tall_rows, &[0u8; 8])], 1, DRAW_X0), "rows past u32");
    assert!(e.contains(&format!("× {}", u32::MAX)), "{e}");
    // Behind another filter, its own parameters in the array.
    let chained = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter [/FlateDecode /CCITTFaxDecode] /DecodeParms [null << /K -1 /Columns 20000 /Rows 20000 >>]";
    let e = refused(&xobject_pdf(&[(chained, &[0u8; 8])], 1, DRAW_X0), "chained");
    assert!(e.contains("20000 × 20000"), "{e}");
    // An inline image and an inline stencil, abbreviated.
    let e = refused(&xobject_pdf(&[], 0, &inline("/W 10 /H 10 /CS /G /BPC 1 /F /CCF /DP << /K -1 /Columns 20000 /Rows 20000 >>", &[0u8; 8])), "inline");
    assert!(e.contains("20000 × 20000"), "{e}");
    let e = refused(&xobject_pdf(&[], 0, &inline("/W 10 /H 10 /IM true /F /CCF /DP << /K -1 /Columns 20000 /Rows 20000 >>", &[0u8; 8])), "stencil");
    assert!(e.contains("20000 × 20000"), "{e}");
    // A CCITT soft mask.
    let masked = "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 6 0 R";
    let e = refused(&xobject_pdf(&[(masked, &[0u8; 48]), (&parms(20000, 20000), &[0u8; 8])], 1, DRAW_X0), "mask");
    assert!(e.contains("soft mask") && e.contains("20000 × 20000"), "{e}");
}

/// One JBIG2 segment (embedded organisation): `number`, `kind`, no referred-to segments, page 1,
/// `data`.
fn seg(number: u32, kind: u8, data: &[u8]) -> Vec<u8> {
    let mut s = number.to_be_bytes().to_vec();
    s.extend_from_slice(&[kind, 0, 1]);
    s.extend_from_slice(&(data.len() as u32).to_be_bytes());
    s.extend_from_slice(data);
    s
}

/// A page information segment's data for `w × h`.
fn jbig2_page_info(w: u32, h: u32) -> Vec<u8> {
    let mut d = w.to_be_bytes().to_vec();
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[0; 8]);
    d.extend_from_slice(&[0, 0, 0]);
    d
}

/// Region segment information for a `w × h` bitmap at the origin, then a generic region's flags
/// (MMR off, template 0) and its four adaptive pixels, then `coded`.
fn region(w: u32, h: u32, coded: &[u8]) -> Vec<u8> {
    let mut d = w.to_be_bytes().to_vec();
    d.extend_from_slice(&h.to_be_bytes());
    d.extend_from_slice(&[0; 8]);
    d.push(0);
    d.push(0);
    d.extend_from_slice(&[3, 0xFF, 0xFD, 0xFF, 2, 0xFE, 0xFE, 0xFE]);
    d.extend_from_slice(coded);
    d
}

const JBIG2_10: &str = "/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode";

#[test]
fn a_jbig2_image_is_sized_by_its_segments() {
    let jbig2 = |segments: &[Vec<u8>]| xobject_pdf(&[(JBIG2_10, &segments.concat())], 1, DRAW_X0);
    // The page information gives the size the decoder builds, whatever the dictionary says.
    let big = jbig2(&[seg(0, 48, &jbig2_page_info(20000, 20000)), seg(1, 49, &[])]);
    let e = refused(&big, "page");
    assert!(e.contains("JBIG2") && e.contains("20000 × 20000"), "{e}");
    assert!(thumbnail(&big, 1, 64, None).is_err() && import_image("big.pdf", &big, 1, None, None, &Interrupt::NONE).is_err());
    // Within the dictionary's size it passes; the data then decides.
    let small = jbig2(&[seg(0, 48, &jbig2_page_info(10, 10)), seg(1, 38, &region(10, 10, &[0; 4])), seg(2, 49, &[])]);
    assert!(passes(&small));
    assert!(import_pdf("small.pdf", &small, &PdfOpenOptions::default(), &Interrupt::NONE).is_ok());
    // A region bigger than the limits, an unknown height without stripes, stripes past it.
    let e = refused(&jbig2(&[seg(0, 48, &jbig2_page_info(10, 10)), seg(1, 38, &region(20000, 20000, &[0; 4]))]), "region");
    assert!(e.contains("region segment"), "{e}");
    let e = refused(&jbig2(&[seg(0, 48, &jbig2_page_info(20000, u32::MAX))]), "unknown height");
    assert!(e.contains("height is unknown"), "{e}");
    let e = refused(&jbig2(&[seg(0, 48, &jbig2_page_info(20000, u32::MAX)), seg(1, 50, &19_999u32.to_be_bytes())]), "stripes");
    assert!(e.contains("20000 × 20000"), "{e}");
    // Symbol and halftone coding take their bitmaps' sizes from the coded data: refused.
    for kind in [0, 6, 16, 22] {
        let e = refused(&jbig2(&[seg(0, 48, &jbig2_page_info(10, 10)), seg(1, kind, &[0; 32])]), "symbols");
        assert!(e.contains("symbol or halftone"), "{kind}: {e}");
    }
    // Headers that can't be read: refused, not decoded.
    let e = refused(&jbig2(&[seg(0, 48, &jbig2_page_info(10, 10))[..6].to_vec()]), "truncated");
    assert!(e.contains("can't be checked"), "{e}");
    let e = refused(&jbig2(&[seg(0, 48, &jbig2_page_info(10, 10)), seg(1, 55, &[])]), "unknown type");
    assert!(e.contains("unknown type"), "{e}");
    let e = refused(
        &xobject_pdf(&[(&JBIG2_10.replace("/JBIG2Decode", "[/FlateDecode /JBIG2Decode]"), &seg(0, 48, &jbig2_page_info(10, 10)))], 1, DRAW_X0),
        "chained",
    );
    assert!(e.contains("JBIG2Decode isn't its only filter"), "{e}");
    // The walk follows the header forms: a 2-byte referred-to number and a 4-byte page
    // association (segment 300), and a region of unknown length that ends at its marker; the big
    // region after each is found.
    let mut long = 300u32.to_be_bytes().to_vec();
    long.extend_from_slice(&[0x40 | 62, 0x20, 0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2, 0xAA, 0xBB]);
    let e = refused(&jbig2(&[seg(1, 48, &jbig2_page_info(10, 10)), long, seg(301, 38, &region(20000, 20000, &[]))]), "long header");
    assert!(e.contains("region segment"), "{e}");
    let mut unknown = 2u32.to_be_bytes().to_vec();
    unknown.extend_from_slice(&[38, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF]);
    unknown.extend(region(10, 10, &[1, 2, 3, 0xFF, 0xAC, 0, 0, 0, 10]));
    let e = refused(&jbig2(&[seg(1, 48, &jbig2_page_info(10, 10)), unknown, seg(3, 38, &region(20000, 20000, &[]))]), "unknown length");
    assert!(e.contains("region segment"), "{e}");
    // The globals count: a page information there, and they must not go through an image codec.
    let globals = |dict: &str| {
        xobject_pdf(
            &[(&format!("{JBIG2_10} /DecodeParms << /JBIG2Globals 6 0 R >>"), &seg(2, 49, &[])), (dict, &seg(0, 48, &jbig2_page_info(20000, 20000)))],
            1,
            DRAW_X0,
        )
    };
    let e = refused(&globals(""), "globals");
    assert!(e.contains("20000 × 20000"), "{e}");
    let e = refused(&globals("/Filter /JPXDecode"), "globals codec");
    assert!(e.contains("JBIG2Globals") || e.contains("JPXDecode"), "{e}");
}

// ---- Streams that aren't images may not go through an image codec. ----

#[test]
fn a_stream_that_isnt_an_image_may_not_use_an_image_codec() {
    let jpx = jpx_header_only(9000, 9000, 1);
    let check = |bytes: &[u8], what: &str, needle: &str| {
        let e = refused(bytes, what);
        assert!(e.contains(needle) && e.contains("isn't an image"), "{what}: {e}");
        assert!(thumbnail(bytes, 1, 64, None).is_err(), "{what}: thumbnail");
        assert!(bounding_box(bytes, 1, None).is_err(), "{what}: bounding box");
        assert!(images(bytes, None).is_err(), "{what}: Images mode list");
        assert!(!opens_as_image(bytes, None), "{what}: image-only check");
    };
    // The page's content, alone or in an array.
    check(&objects_pdf("", "", "/Filter /JPXDecode", &jpx, vec![]), "contents", "JPXDecode");
    let arr = objects_pdf("", "/Contents [4 0 R 5 0 R]", "", b"0 0 1 rg 0 0 10 10 re f", vec![stream("/Filter /DCTDecode", &[0xFF, 0xD8])]);
    check(&arr, "contents array", "DCTDecode");
    // A Form XObject the page never draws.
    check(
        &objects_pdf("/XObject << /F 5 0 R >>", "", "", b"", vec![stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Filter /JPXDecode", &jpx)]),
        "form",
        "JPXDecode",
    );
    // A tiling pattern, a Type 3 glyph, a soft-mask group, a font program, an ICC profile.
    check(
        &objects_pdf(
            "/Pattern << /P 5 0 R >>",
            "",
            "",
            b"",
            vec![stream("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Filter /CCITTFaxDecode", &[0; 8])],
        ),
        "pattern",
        "CCITTFaxDecode",
    );
    check(
        &objects_pdf(
            "/Font << /T3 5 0 R >>",
            "",
            "",
            b"",
            vec![
                b"<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [0.1 0 0 0.1 0 0] /CharProcs << /a 6 0 R >> /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [10] >>".to_vec(),
                stream("/Filter /JBIG2Decode", &[0; 8]),
            ],
        ),
        "type 3",
        "JBIG2Decode",
    );
    check(
        &objects_pdf(
            "/ExtGState << /G0 << /SMask << /S /Luminosity /G 5 0 R >> >> >>",
            "",
            "",
            b"",
            vec![stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Filter /JPXDecode", &jpx)],
        ),
        "soft mask group",
        "JPXDecode",
    );
    check(
        &objects_pdf(
            "/Font << /F 5 0 R >>",
            "",
            "",
            b"",
            vec![
                b"<< /Type /Font /Subtype /TrueType /BaseFont /X /FontDescriptor 6 0 R >>".to_vec(),
                b"<< /Type /FontDescriptor /FontName /X /Flags 32 /FontFile2 7 0 R >>".to_vec(),
                stream("/Filter /JPXDecode", &jpx),
            ],
        ),
        "font file",
        "/FontFile2",
    );
    check(&objects_pdf("/ColorSpace << /CS [/ICCBased 5 0 R] >>", "", "", b"", vec![stream("/N 3 /Filter /DCTDecode", &[0xFF, 0xD8])]), "icc", "DCTDecode");
    // An annotation's appearance.
    check(
        &objects_pdf(
            "",
            "/Annots [<< /Type /Annot /Subtype /Square /Rect [0 0 10 10] /AP << /N 5 0 R >> >>]",
            "",
            b"",
            vec![stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Filter /JPXDecode", &jpx)],
        ),
        "appearance",
        "JPXDecode",
    );
    // A stream that calls itself an image is one only where the interpreter decodes an image: as
    // a pattern it is content.
    check(
        &objects_pdf("/Pattern << /P 5 0 R >>", "", "", b"", vec![stream("/Type /XObject /Subtype /Image /PatternType 1 /Filter /JPXDecode", &jpx)]),
        "image as pattern",
        "JPXDecode",
    );
    // Images may: a JPX image beside an ICC profile in Flate passes.
    let ok = objects_pdf(
        "/XObject << /X0 5 0 R >> /ColorSpace << /CS [/ICCBased 6 0 R] >>",
        "",
        "",
        DRAW_X0,
        vec![stream(JPX_10, &J2K_GRAY_2X2), stream("/N 1 /Filter /FlateDecode", &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01])],
    );
    assert!(passes(&ok));
    assert!(opens_as_image(&ok, None));
}

#[test]
fn a_walk_past_its_limits_refuses_the_page() {
    // An array that holds itself ends the walk (the reader cuts the cycle).
    assert!(passes(&objects_pdf("/Properties << /P 5 0 R >>", "", "", b"", vec![b"[5 0 R 5 0 R]".to_vec()])));
    // 25 arrays each holding the next twice: 2^25 paths, refused at the object limit instead of
    // walked.
    let more: Vec<Vec<u8>> = (5..30).map(|k| format!("[{} 0 R {} 0 R]", k + 1, k + 1).into_bytes()).chain([b"[]".to_vec()]).collect();
    let e = refused(&objects_pdf("/Properties << /P 5 0 R >>", "", "", b"", more), "doubling");
    assert!(e.contains("too deep or hold too many objects"), "{e}");
}

// ---- What opening a PDF ties to the file, and which files open as PDFs. ----

#[test]
fn only_a_one_image_pdf_opened_directly_keeps_the_file() {
    let one = image_pdf(200.0, 100.0, &[(100, 50, false, "200 0 0 100 0 0")], "");
    let two = image_pdf(400.0, 300.0, &[(100, 50, false, "200 0 0 100 20 150"), (64, 64, true, "64 0 0 64 300 50")], "");
    let plain = |b: &[u8]| crate::import("f.pdf", b).unwrap().source_read_only;
    let chosen = |b: &[u8], o: PdfOpenOptions| {
        crate::import_with_options("f.pdf", b, &crate::ImportOptions { pdf: Some(o), ..Default::default() }).unwrap().source_read_only
    };
    assert!(!plain(&one), "a one-image PDF opens like a picture");
    assert!(plain(&two), "a page is rasterised");
    assert!(chosen(&two, PdfOpenOptions { image: Some(1), ..Default::default() }), "Images mode");
    assert!(chosen(&two, PdfOpenOptions::default()), "a page chosen in Import PDF");
    assert!(chosen(&one, PdfOpenOptions::default()), "anything Import PDF opens");
}

#[test]
fn a_file_opens_as_a_pdf_when_it_starts_like_one_or_nothing_else_takes_it() {
    let svg = b"<?xml version=\"1.0\"?>\n<!-- exported from %PDF-1.4 source -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"4\"><rect width=\"8\" height=\"4\" fill=\"red\"/></svg>";
    assert!(is_pdf(svg) && !crate::is_pdf_input("drawing.svg", svg) && !crate::is_pdf_input("drawing", svg));
    let r = crate::import("drawing.svg", svg).expect("opens as an SVG, as before PDF import existed");
    assert_eq!((r.document.size.width, r.document.size.height), (8, 4));
    // A PDF after a byte-order mark or a shell line still opens as one.
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(pdf(&[page(72.0, 72.0, &rect(0.0, 0.0, 72.0, 72.0, (0.0, 0.0, 0.0)))]));
    assert!(crate::is_pdf_input("x.pdf", &bom));
    assert!(crate::import("x.pdf", &bom).is_ok());
    let r = crate::import("scan.bin", &bom).expect("a leading-byte PDF needs no .pdf extension");
    assert_eq!(r.document.artboards().len(), 1);
    let mut illustrator = pdf(&[page(72.0, 72.0, &rect(0.0, 0.0, 72.0, 72.0, (0.0, 0.0, 0.0)))]);
    illustrator.extend_from_slice(b"\n% /AIPrivateData\n");
    let r = crate::import("illustration.ai", &illustrator).expect("PDF-based Illustrator data still uses PDF import");
    assert_eq!(r.document.artboards().len(), 1);
    assert!(crate::is_pdf_input("x", b"%PDF-1.7\n"));
}
