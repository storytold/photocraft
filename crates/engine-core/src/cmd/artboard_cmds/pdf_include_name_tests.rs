//! Artboards to PDF › Include Artboard Name (#13654), on the measured document: each page grows
//! by a border of the Canvas Extension colour, the board's name in the top one; bad Name Options
//! fail the export before anything is written; without the option the PDF is today's.

use super::pdf_page_order_tests::session;
use super::tests::assert_rejected;
use super::*;

const RED: [f32; 4] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
const BLUE: [f32; 4] = [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, 1.0];

/// The measured document with its pixel layers: `Red` at 20,20-120,100 in Red Board and `Blue`
/// at 130,40-230,140 in Blue Board.
fn measured() -> Session {
    let (mut s, [red, blue, _]) = session();
    for (board, name, r, rgba) in [(red, "Red", Rect::new(20, 20, 120, 100), RED), (blue, "Blue", Rect::new(130, 40, 230, 140), BLUE)] {
        s.edit("px", |doc, _| {
            let fmt = doc.pixel_format();
            let mut l = Layer::raster(name, fmt);
            l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&fmt, rgba));
            doc.layer_mut(LayerId(board)).and_then(Layer::children_mut).unwrap().push(l);
            Ok(())
        })
        .unwrap();
    }
    s
}

/// A PDF path in an empty folder that is removed when dropped.
fn pdf_path() -> (tempfile::TempDir, String) {
    let dir = tempfile::Builder::new().prefix("photocraft-pdf-name-").tempdir().unwrap();
    let path = join(&dir.path().to_string_lossy(), "boards.pdf");
    (dir, path)
}

/// Runs the export with `extra` params and returns the PDF's bytes.
fn export(s: &mut Session, extra: Value) -> Vec<u8> {
    let (_dir, path) = pdf_path();
    let mut p = extra;
    p["path"] = json!(path);
    s.execute("file.export.artboardsToPdf", p).unwrap();
    std::fs::read(&path).unwrap()
}

/// Each page's size in points, in page order.
fn page_sizes(pdf: &[u8]) -> Vec<(u32, u32)> {
    String::from_utf8_lossy(pdf)
        .split("/MediaBox [0 0 ")
        .skip(1)
        .map(|rest| {
            let mut n = rest.split([' ', ']']).map(|v| v.parse::<f64>().unwrap().round() as u32);
            (n.next().unwrap(), n.next().unwrap())
        })
        .collect()
}

/// Each page's image, decoded and flattened, in page order.
fn page_images(pdf: &[u8]) -> Vec<photocraft_compose::Buffer> {
    let marker = b"/Filter /DCTDecode /Length ";
    let mut images = Vec::new();
    let mut at = 0;
    while let Some(found) = pdf[at..].windows(marker.len()).position(|w| w == marker) {
        let start = at + found + marker.len();
        let digits = pdf[start..].iter().take_while(|b| b.is_ascii_digit()).count();
        let len: usize = std::str::from_utf8(&pdf[start..start + digits]).unwrap().parse().unwrap();
        let data = start + pdf[start..].windows(7).position(|w| w == b"stream\n").unwrap() + 7;
        let jpeg = &pdf[data..data + len];
        images.push(photocraft_compose::flatten(&photocraft_io::import("page.jpg", jpeg).unwrap().document));
        at = data + len;
    }
    images
}

fn close(a: [f32; 4], b: [f32; 4], tolerance: f32) -> bool {
    a.iter().zip(b).take(3).all(|(x, y)| (x - y).abs() < tolerance)
}

#[test]
fn the_pages_grow_by_the_measured_borders() {
    let mut s = measured();
    assert_eq!(page_sizes(&export(&mut s, json!({}))), [(200, 150), (200, 200), (100, 100)]);
    assert_eq!(page_sizes(&export(&mut s, json!({"includeName": true}))), [(236, 186), (240, 240), (120, 120)]);
}

#[test]
fn the_board_sits_unchanged_inside_a_border_of_the_extension_colour() {
    let mut s = measured();
    let plain = page_images(&export(&mut s, json!({})));
    for (extension, colour) in [(json!("white"), [1.0; 4]), (json!("black"), [0.0, 0.0, 0.0, 1.0]), (json!([0.0, 1.0, 0.0]), [0.0, 1.0, 0.0, 1.0])] {
        let named = page_images(&export(&mut s, json!({"includeName": true, "extension": extension, "textColor": "#ff0000"})));
        for ((before, after), b) in plain.iter().zip(&named).zip([18, 20]) {
            // Inside the board: the same pixels shifted by the border (JPEG blocks fall
            // differently, so compare away from colour edges).
            for (x, y) in [(70, 60), (160, 30), (40, 130), (170, 130)] {
                assert!(
                    close(before.get(x, y), after.get(x + b, y + b), 0.05),
                    "{extension} at {x},{y}: {:?} vs {:?}",
                    before.get(x, y),
                    after.get(x + b, y + b)
                );
            }
            // The left, right and bottom borders and the top border's corners are the colour.
            let (w, h) = (after.rect.width() as i32, after.rect.height() as i32);
            for (x, y) in [(2, 2), (w - 3, 2), (2, h / 2), (w - 3, h / 2), (w / 2, h - 3), (b / 2, h - b / 2)] {
                assert!(close(after.get(x, y), colour, 0.05), "{extension} at {x},{y}: {:?}", after.get(x, y));
            }
        }
        assert!(close(named[0].get(18 + 70, 18 + 60), RED, 0.05), "{extension}: Red keeps its colour");
        assert!(close(named[1].get(20 + 60, 20 + 60), BLUE, 0.05), "{extension}: Blue keeps its colour");
    }
}

#[test]
fn the_name_is_in_the_top_border_from_the_boards_left_edge() {
    let mut s = measured();
    let named = page_images(&export(&mut s, json!({"includeName": true, "textColor": [1.0, 0.0, 0.0]})));
    for (page, b) in named.iter().zip([18, 20, 10]) {
        let w = page.rect.width() as i32;
        let h = page.rect.height() as i32;
        // Ink is where the page is clearly not white.
        let ink: Vec<(i32, i32)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| page.get(x, y)[1] < 0.5).collect();
        let inked = |x: i32, y: i32| ink.contains(&(x, y));
        let top: Vec<_> = ink.iter().filter(|p| p.1 < b).collect();
        assert!(!top.is_empty(), "border {b}: the name is drawn in the top border");
        let x0 = top.iter().map(|p| p.0).min().unwrap();
        assert!((x0 - b).abs() <= 1, "border {b}: ink starts at the board's left edge, got {x0}");
        // Nothing outside the board and the top border.
        assert!((0..h).all(|y| (0..b).all(|x| !inked(x, y))), "border {b}: no ink in the left border");
        assert!((b..h).all(|y| (w - b..w).all(|x| !inked(x, y))), "border {b}: no ink in the right border");
        assert!((h - b..h).all(|y| (0..w).all(|x| !inked(x, y))), "border {b}: no ink in the bottom border");
        let darkest = top.iter().map(|&&(x, y)| page.get(x, y)).min_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
        // Thin anti-aliased type loses some saturation to JPEG's colour subsampling.
        assert!(darkest[0] > 0.7 && darkest[1] < 0.3 && darkest[2] < 0.3, "border {b}: red text, got {darkest:?}");
    }
}

#[test]
fn a_name_taller_than_the_border_stays_in_it() {
    let mut s = measured();
    let plain = page_images(&export(&mut s, json!({})));
    let named = page_images(&export(&mut s, json!({"includeName": true, "fontSize": "200"})));
    let (before, after) = (&plain[0], &named[0]);
    for (x, y) in [(70, 60), (160, 30), (40, 130), (170, 130)] {
        assert!(close(before.get(x, y), after.get(x + 18, y + 18), 0.05), "at {x},{y}");
    }
    // The left, right and bottom borders are those of the default size: the overlapping Blue
    // board's edge leaves JPEG ringing there either way. Rows from 16 px below the top border, past
    // JPEG's colour blocks along it.
    let small = page_images(&export(&mut s, json!({"includeName": true})));
    let (w, h, b) = (236, 186, 18);
    let sides = (b + 16..h).flat_map(|y| (0..b).chain(w - b..w).map(move |x| (x, y)));
    for (x, y) in sides.chain((h - b..h).flat_map(|y| (0..w).map(move |x| (x, y)))) {
        assert!(close(after.get(x, y), small[0].get(x, y), 0.05), "at {x},{y}: {:?} vs {:?}", after.get(x, y), small[0].get(x, y));
    }
}

/// With a transparent Canvas Extension nothing covers the name, so only its cut to the top border
/// keeps it out of the other borders: the side and bottom borders don't change with its colour.
#[test]
fn a_name_stays_in_the_top_border_over_a_transparent_extension() {
    let mut s = measured();
    let with = |s: &mut Session, text: &str| {
        page_images(&export(s, json!({"includeName": true, "fontSize": "200", "extension": "#00000000", "textColor": text}))).remove(0)
    };
    let (red, green) = (with(&mut s, "#ff0000"), with(&mut s, "#00ff00"));
    // Rows from 16 px below the top border, past JPEG's colour blocks along it.
    let (w, h, b) = (236, 186, 18);
    let sides = (b + 16..h).flat_map(|y| (0..b).chain(w - b..w).map(move |x| (x, y)));
    for (x, y) in sides.chain((h - b..h).flat_map(|y| (0..w).map(move |x| (x, y)))) {
        assert!(close(red.get(x, y), green.get(x, y), 0.1), "at {x},{y}: {:?} vs {:?}", red.get(x, y), green.get(x, y));
    }
}

#[test]
fn without_the_option_the_pdf_is_todays() {
    let mut s = measured();
    let today = export(&mut s, json!({}));
    assert_eq!(export(&mut s, json!({"includeName": false, "fontSize": "abc", "extension": "nope"})), today);
}

#[test]
fn bad_name_options_fail_and_write_nothing() {
    let mut s = measured();
    let (dir, path) = pdf_path();
    for bad in [
        json!({"fontSize": "abc"}),
        json!({"fontSize": ""}),
        json!({"fontSize": 0}),
        json!({"fontSize": -12}),
        json!({"fontSize": "-1"}),
        json!({"fontSize": "NaN"}),
        json!({"fontSize": true}),
        json!({"extension": "purple"}),
        json!({"extension": 3}),
        json!({"extension": [0.5]}),
        json!({"extension": [0.1, "x", 0.2]}),
        json!({"extension": "other", "extensionColor": "not-a-colour"}),
        json!({"extension": "other", "extensionColor": [0.5]}),
        json!({"extension": "other", "extensionColor": [0.1, "x", 0.2]}),
    ] {
        let mut p = bad.clone();
        p["includeName"] = json!(true);
        p["path"] = json!(path);
        assert_rejected(&mut s, "file.export.artboardsToPdf", p);
        assert!(!std::path::Path::new(&path).exists(), "{bad}: nothing is written");
    }
    drop(dir);
}

#[test]
fn the_export_leaves_the_document_and_its_history_alone() {
    let mut s = measured();
    let d = s.active().unwrap();
    let (before, revision, past) = (d.doc.clone(), d.revision, d.history.past_len());
    export(&mut s, json!({"includeName": true, "extension": "other", "extensionColor": [0.2, 0.4, 0.6], "fontSize": 14, "fontStyle": "Bold"}));
    let d = s.active().unwrap();
    assert!(d.doc == before, "the document is unchanged");
    assert_eq!((d.revision, d.history.past_len()), (revision, past), "no history step");
}
