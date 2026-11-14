//! Artboards to PDF › Include Overlapping Areas / Artboard Content Only (#13646): a board's page
//! also shows the boards stacked above it where they overlap it, boards below never show, and the
//! source document is left alone.

use super::*;
use photocraft_geom::Rect;

const RED: [u8; 3] = [220, 30, 30];
const BLUE: [u8; 3] = [30, 30, 220];
const WHITE: [u8; 3] = [255, 255, 255];
const CMD: &str = "file.export.artboardsToPdf";

/// The measured test document: 800 x 400, transparent, bottom to top `Red Board` 0,0-200,150
/// (pixel layer `Red` at 20,20-120,100), `Blue Board` 120,30-320,230 (pixel layer `Blue` at
/// 130,40-230,140) and `Empty Board` 600,0-700,100, all with white backgrounds.
fn session(depth: u32) -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400, "depth": depth, "background": "transparent"})).unwrap();
    let board =
        |s: &mut Session, name: &str, rect: [i64; 4]| s.execute("layer.new.artboard", json!({"rect": rect, "name": name})).unwrap()["layer"].as_u64().unwrap();
    let red = board(&mut s, "Red Board", [0, 0, 200, 150]);
    let blue = board(&mut s, "Blue Board", [120, 30, 200, 200]);
    let empty = board(&mut s, "Empty Board", [600, 0, 100, 100]);
    paint(&mut s, red, "Red", Rect::new(20, 20, 120, 100), RED);
    paint(&mut s, blue, "Blue", Rect::new(130, 40, 230, 140), BLUE);
    (s, [red, blue, empty])
}

/// A pixel layer `name` of `color` over `rect`, on top of board `board`'s layers.
fn paint(s: &mut Session, board: u64, name: &str, rect: Rect, color: [u8; 3]) {
    s.edit(name, |doc, _| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster(name, fmt);
        let [r, g, b] = color.map(|c| f32::from(c) / 255.0);
        l.surface_mut().unwrap().fill_rect(rect, &photocraft_raster::from_rgba(&fmt, [r, g, b, 1.0]));
        doc.layer_mut(LayerId(board)).unwrap().children_mut().unwrap().push(l);
        Ok(())
    })
    .unwrap();
}

struct Out(std::path::PathBuf);

impl Out {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("photocraft-pdf-overlap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Out(dir)
    }
    fn pdf(&self) -> String {
        self.0.join("boards.pdf").to_string_lossy().into_owned()
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The JPEG image of every page in a PDF written by [`raster_pdf`], decoded.
fn pages(pdf: &[u8]) -> Vec<Document> {
    let key = b"/DCTDecode /Length ";
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = pdf[at..].windows(key.len()).position(|w| w == key) {
        let start = at + i + key.len();
        let digits = pdf[start..].iter().take_while(|b| b.is_ascii_digit()).count();
        let len: usize = std::str::from_utf8(&pdf[start..start + digits]).unwrap().parse().unwrap();
        let data = start + digits + b" >>\nstream\n".len();
        out.push(photocraft_io::import("page.jpg", &pdf[data..data + len]).unwrap().document);
        at = data + len;
    }
    out
}

/// Exports board `board` with `extra` params and reads its one page back.
fn export(s: &mut Session, out: &Out, board: u64, extra: Value) -> Document {
    let mut p = json!({"path": out.pdf(), "artboards": [board], "quality": 12});
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    let r = s.execute(CMD, p).unwrap();
    assert_eq!(r["pages"], 1, "{r}");
    let mut pages = pages(&std::fs::read(out.pdf()).unwrap());
    assert_eq!(pages.len(), 1);
    pages.remove(0)
}

/// The page's pixel at (x, y) as 8-bit RGB.
fn px(d: &Document, x: i32, y: i32) -> [u8; 3] {
    let flat = photocraft_compose::flatten(d);
    let w = d.size.width as usize;
    let p = flat.px[y as usize * w + x as usize].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    [p[0], p[1], p[2]]
}

/// Within JPEG tolerance.
fn near(got: [u8; 3], want: [u8; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 8)
}

#[test]
fn include_overlapping_areas_is_the_default_and_shows_the_board_above() {
    for depth in [8, 16] {
        let (mut s, [red, _, _]) = session(depth);
        let out = Out::new(&format!("default-{depth}"));
        for extra in [json!({}), json!({"content": "overlapping"}), json!({"content": null})] {
            let page = export(&mut s, &out, red, extra.clone());
            assert_eq!((page.size.width, page.size.height), (200, 150), "{depth} bit {extra}");
            assert!(near(px(&page, 150, 60), BLUE), "{depth} bit {extra}: Blue's pixels, got {:?}", px(&page, 150, 60));
            assert!(near(px(&page, 125, 35), WHITE), "{depth} bit {extra}: Blue Board's background, got {:?}", px(&page, 125, 35));
            assert!(near(px(&page, 110, 50), RED), "{depth} bit {extra}: Red stays red");
        }
    }
}

#[test]
fn artboard_content_only_keeps_todays_page() {
    let (mut s, [red, _, _]) = session(8);
    let out = Out::new("content-only");
    let page = export(&mut s, &out, red, json!({"content": "artboard"}));
    assert!(near(px(&page, 150, 60), WHITE), "nothing of Blue Board, got {:?}", px(&page, 150, 60));
    assert!(near(px(&page, 110, 50), RED));
}

#[test]
fn boards_below_do_not_show_in_either_mode() {
    let (mut s, [red, blue, _]) = session(8);
    let out = Out::new("below");
    for content in ["overlapping", "artboard"] {
        let page = export(&mut s, &out, blue, json!({"content": content}));
        assert_eq!((page.size.width, page.size.height), (200, 200), "{content}");
        // Page (0,0) and (110,50): source (120,30) and (230,80), over Red Board's area or past it.
        assert!(near(px(&page, 0, 0), WHITE), "{content}: got {:?}", px(&page, 0, 0));
        assert!(near(px(&page, 110, 50), WHITE), "{content}: got {:?}", px(&page, 110, 50));
        assert!(near(px(&page, 30, 30), BLUE), "{content}");
    }
    // A transparent Blue Board would show Red Board's red through it if boards below leaked in.
    s.execute("layer.artboard.set", json!({"layer": blue, "background": "transparent"})).unwrap();
    paint(&mut s, red, "Red in overlap", Rect::new(120, 30, 200, 40), RED);
    for content in ["overlapping", "artboard"] {
        let page = export(&mut s, &out, blue, json!({"content": content}));
        // Source (150,35): Red's pixels under Blue Board; the page stays white there.
        assert!(near(px(&page, 30, 5), WHITE), "{content}: got {:?}", px(&page, 30, 5));
    }
}

#[test]
fn every_board_gets_its_page_and_only_the_overlapped_one_changes() {
    let (mut s, _) = session(8);
    let out = Out::new("all");
    for content in ["overlapping", "artboard"] {
        let r = s.execute(CMD, json!({"path": out.pdf(), "content": content, "quality": 12})).unwrap();
        assert_eq!(r["pages"], 3, "{content}");
        let pages = pages(&std::fs::read(out.pdf()).unwrap());
        let red = pages.iter().find(|p| (p.size.width, p.size.height) == (200, 150)).unwrap();
        let empty = pages.iter().find(|p| (p.size.width, p.size.height) == (100, 100)).unwrap();
        let want = if content == "overlapping" { BLUE } else { WHITE };
        assert!(near(px(red, 150, 60), want), "{content}: got {:?}", px(red, 150, 60));
        assert!(near(px(empty, 50, 50), WHITE), "{content}");
    }
}

#[test]
fn the_source_document_is_unchanged_and_gets_no_history_entry() {
    let (mut s, [red, blue, _]) = session(8);
    let out = Out::new("source");
    for content in ["overlapping", "artboard"] {
        for board in [red, blue] {
            let d = s.active().unwrap();
            let (before, rev, past) = (d.doc.clone(), d.revision, d.history.past_len());
            export(&mut s, &out, board, json!({"content": content}));
            let d = s.active().unwrap();
            assert!(d.doc == before, "{content} changed the document");
            assert_eq!((d.revision, d.history.past_len()), (rev, past), "{content}");
        }
    }
}

#[test]
fn a_bad_content_value_is_an_error_and_writes_nothing() {
    let (mut s, _) = session(8);
    let out = Out::new("bad");
    for bad in [json!("both"), json!(""), json!(1), json!(true), json!(["overlapping"]), json!({})] {
        assert!(s.execute(CMD, json!({"path": out.pdf(), "content": bad})).is_err(), "{bad}");
        assert!(!std::path::Path::new(&out.pdf()).exists(), "{bad}");
    }
}
