//! Artboards to Files' Include Background in Export (#13631): with `"includeBackground": false`
//! a board's white fill is left out of a PNG, TIFF or WebP, so the file is transparent wherever
//! the board's layers have no pixels. Ticked (the default) keeps today's files, and so do the
//! cases the reference application was not measured with.
//!
//! The document is the issue's: 800 x 400, RGB, 8 bit, transparent, with, bottom to top, `Red
//! Board` 0,0-200,150 (white, pixel layer `Red` 220,30,30 at 20,20-120,100), `Blue Board`
//! 120,30-320,230 (white, pixel layer `Blue` 30,30,220 at 130,40-230,140) and `Empty Board`
//! 600,0-700,100 (white, no layers).

use super::*;

const RED: [u8; 4] = [220, 30, 30, 255];
const BLUE: [u8; 4] = [30, 30, 220, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// The issue's document at `depth` bits, its boards' fill `background` (`white`, `black`, …).
/// Returns the session and the ids of Red, Blue and Empty Board.
fn session(depth: u32, background: Value) -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "abfiles-test", "width": 800, "height": 400, "depth": depth, "background": "transparent"})).unwrap();
    let mut board = |name: &str, rect: [u32; 4], layer: Option<(&str, [u32; 4], &str)>| {
        let mut p = json!({"name": name, "rect": rect, "background": background});
        if background == "custom" {
            p["color"] = json!("#3366CC");
        }
        let id = s.execute("layer.new.artboard", p).unwrap()["layer"].as_u64().unwrap();
        if let Some((layer, [x, y, width, height], color)) = layer {
            s.execute("layer.select", json!({"layer": id})).unwrap();
            s.execute("layer.new.layer", json!({"name": layer})).unwrap();
            s.execute("select.rect", json!({"x": x, "y": y, "width": width, "height": height, "antiAlias": false})).unwrap();
            s.execute("edit.fill", json!({"color": color})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
        }
        id
    };
    let red = board("Red Board", [0, 0, 200, 150], Some(("Red", [20, 20, 100, 80], "#DC1E1E")));
    let blue = board("Blue Board", [120, 30, 200, 200], Some(("Blue", [130, 40, 100, 100], "#1E1EDC")));
    let empty = board("Empty Board", [600, 0, 100, 100], None);
    (s, [red, blue, empty])
}

/// One written file read back: its bytes, size and every pixel as 8-bit RGBA, row by row.
struct Written {
    bytes: Vec<u8>,
    size: (u32, u32),
    px: Vec<[u8; 4]>,
}

impl Written {
    /// A PNG's IHDR colour type: 2 is RGB, 6 is RGBA.
    fn png_colour_type(&self) -> u8 {
        assert_eq!(&self.bytes[12..16], b"IHDR");
        self.bytes[25]
    }

    /// The pixel at (`x`, `y`).
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        self.px[(y * self.size.0 + x) as usize]
    }

    /// Asserts every pixel: `inside` the rect ([x0, y0, x1, y1]) exactly `ink`, elsewhere
    /// exactly `outside`, or only fully transparent when `outside` is `None`.
    fn assert_pixels(&self, inside: [u32; 4], ink: [u8; 4], outside: Option<[u8; 4]>) {
        let [x0, y0, x1, y1] = inside;
        for y in 0..self.size.1 {
            for x in 0..self.size.0 {
                let px = self.at(x, y);
                if (x0..x1).contains(&x) && (y0..y1).contains(&y) {
                    assert_eq!(px, ink, "({x}, {y})");
                } else if let Some(outside) = outside {
                    assert_eq!(px, outside, "({x}, {y})");
                } else {
                    assert_eq!(px[3], 0, "({x}, {y}) is transparent: {px:?}");
                }
            }
        }
    }
}

/// Runs Artboards to Files on `board` alone with `params` and reads the one file back. Checks
/// that the document and its History are left alone.
fn export(s: &mut Session, board: u64, mut params: Value) -> Written {
    let dir = tempfile::tempdir().unwrap();
    params["dir"] = json!(dir.path());
    params["artboards"] = json!([board]);
    let before = (s.active().unwrap().revision, s.active().unwrap().history.entries());
    let r = s.execute("file.export.artboardsToFiles", params).unwrap();
    assert_eq!((s.active().unwrap().revision, s.active().unwrap().history.entries()), before, "the document and its History are unchanged");
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
    let path = files[0].as_str().unwrap();
    let bytes = std::fs::read(path).unwrap();
    let doc = photocraft_io::import(path, &bytes).unwrap().document;
    let px = photocraft_compose::flatten(&doc).px.iter().map(|px| px.map(|c| (c * 255.0).round() as u8)).collect();
    Written { bytes, size: (doc.size.width, doc.size.height), px }
}

#[test]
fn ticked_keeps_the_white_fill_in_an_rgb_png() {
    let (mut s, [red, ..]) = session(8, json!("white"));
    for params in [json!({}), json!({"includeBackground": true}), json!({"includeBackground": null})] {
        let png = export(&mut s, red, params.clone());
        assert_eq!((png.size, png.png_colour_type()), ((200, 150), 2), "{params}: RGB, no alpha");
        assert_eq!((png.at(0, 0), png.at(30, 30)), (WHITE, RED), "{params}");
        png.assert_pixels([20, 20, 120, 100], RED, Some(WHITE));
    }
}

#[test]
fn unticked_leaves_the_fill_out_of_a_png() {
    let (mut s, [red, blue, _]) = session(8, json!("white"));
    let png = export(&mut s, red, json!({"includeBackground": false}));
    assert_eq!((png.size, png.png_colour_type()), ((200, 150), 6), "RGBA");
    assert_eq!((png.at(0, 0)[3], png.at(30, 30)), (0, RED));
    png.assert_pixels([20, 20, 120, 100], RED, None);
    // Blue Board's layer, 130,40 on the canvas, is at 10,10 in the board.
    let png = export(&mut s, blue, json!({"includeBackground": false, "format": ".PNG"}));
    assert_eq!((png.size, png.png_colour_type()), ((200, 200), 6));
    png.assert_pixels([10, 10, 110, 110], BLUE, None);
}

#[test]
fn unticked_tiff_and_webp_are_transparent_too() {
    let (mut s, [red, ..]) = session(8, json!("white"));
    for format in ["tif", "tiff", "webp"] {
        let ticked = export(&mut s, red, json!({"format": format}));
        ticked.assert_pixels([20, 20, 120, 100], RED, Some(WHITE));
        let unticked = export(&mut s, red, json!({"format": format, "includeBackground": false}));
        assert_eq!(unticked.size, (200, 150), "{format}");
        unticked.assert_pixels([20, 20, 120, 100], RED, None);
    }
}

#[test]
fn psd_and_the_formats_without_alpha_keep_todays_file() {
    let (mut s, [red, ..]) = session(8, json!("white"));
    for format in ["jpg", "jpeg", "bmp", "psd"] {
        let ticked = export(&mut s, red, json!({"format": format}));
        let unticked = export(&mut s, red, json!({"format": format, "includeBackground": false}));
        assert_eq!(unticked.size, (200, 150), "{format}");
        // JPEG is lossy, so the red layer may bleed a little into the corner's block; BMP and
        // PSD must be exactly white there.
        let corner = unticked.at(0, 0);
        let flattened = if matches!(format, "jpg" | "jpeg") { corner[3] == 255 && corner.iter().all(|c| *c >= 240) } else { corner == WHITE };
        assert!(flattened, "{format}: flattened on the board's white, not transparent: {corner:?}");
        assert!(unticked.bytes == ticked.bytes, "{format}: unticked writes the same file as ticked");
    }
    // Lossless, so the whole board can be checked.
    export(&mut s, red, json!({"format": "bmp", "includeBackground": false})).assert_pixels([20, 20, 120, 100], RED, Some(WHITE));
}

#[test]
fn an_empty_board_keeps_its_fill() {
    let (mut s, [.., empty]) = session(8, json!("white"));
    let png = export(&mut s, empty, json!({"includeBackground": false}));
    assert_eq!((png.size, png.png_colour_type()), ((100, 100), 2), "today's 100 x 100 white RGB file");
    assert!(png.px.iter().all(|px| *px == WHITE));
}

#[test]
fn fills_other_than_white_and_other_depths_keep_todays_file() {
    for (depth, background) in [(8, json!("black")), (8, json!("custom")), (8, json!("transparent")), (16, json!("white"))] {
        let (mut s, [red, ..]) = session(depth, background.clone());
        let ticked = export(&mut s, red, json!({}));
        let unticked = export(&mut s, red, json!({"includeBackground": false}));
        assert!(unticked.bytes == ticked.bytes, "{depth} bit, {background}: unticked writes the same file as ticked");
    }
}

#[test]
fn every_board_of_an_unticked_export() {
    let (mut s, [red, blue, empty]) = session(8, json!("white"));
    let dir = tempfile::tempdir().unwrap();
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": dir.path(), "prefix": "out", "includeBackground": false})).unwrap();
    let alpha: Vec<(u32, u32, bool)> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let doc = photocraft_io::import("x.png", &std::fs::read(f.as_str().unwrap()).unwrap()).unwrap().document;
            (doc.size.width, doc.size.height, photocraft_compose::flatten(&doc).px[0][3] == 0.0)
        })
        .collect();
    // Top of the Layers panel first: Empty Board keeps its white, the other two lose theirs.
    assert_eq!(alpha, [(100, 100, false), (200, 200, true), (200, 150, true)]);
    // Board by board gives the same as all at once.
    assert_eq!(export(&mut s, empty, json!({"includeBackground": false})).at(0, 0), WHITE);
    assert_eq!(export(&mut s, blue, json!({"includeBackground": false})).at(0, 0)[3], 0);
    assert_eq!(export(&mut s, red, json!({"includeBackground": false})).at(0, 0)[3], 0);
}

#[test]
fn only_files_with_alpha_drop_the_fill() {
    let without = |format: &str| !includes_background(&json!({"format": format, "includeBackground": false}));
    assert!(["png", ".PNG", "tif", "TIFF", "webp"].into_iter().all(without));
    assert!(!["jpg", "jpeg", "bmp", "psd", "gif"].into_iter().any(without));
    assert!(!includes_background(&json!({"includeBackground": false})), "PNG is the default format");
    assert!(includes_background(&json!({})) && includes_background(&json!({"includeBackground": "no"})), "anything but false keeps the fill");
}
