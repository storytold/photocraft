//! Artboards to PDF's Include Background in Export (#13648): with `"includeBackground": false`
//! a white board's fill is left out and its page image gets a soft mask (`/SMask`, DeviceGray,
//! 8 bpc, FlateDecode, `/Matte [1 1 1]`), alpha 0 where the board has no pixels. Ticked (the
//! default) keeps today's pages, and so do the cases the reference application was not measured
//! with.
//!
//! The document is the issue's: 800 x 400, RGB, 8 bit, transparent, with, bottom to top, `Red
//! Board` 0,0-200,150 (white, pixel layer `Red` 220,30,30 at 20,20-120,100), `Blue Board`
//! 120,30-320,230 (white, pixel layer `Blue` 30,30,220 at 130,40-230,140) and `Empty Board`
//! 600,0-700,100 (white, no layers).

use super::*;

/// The issue's document at `depth` bits. Returns the session and the ids of Red, Blue and Empty
/// Board.
fn session(depth: u32) -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"name": "abpdf-test", "width": 800, "height": 400, "depth": depth, "background": "transparent"})).unwrap();
    let mut board = |name: &str, rect: [u32; 4], layer: Option<(&str, [u32; 4], &str)>| {
        let id = s.execute("layer.new.artboard", json!({"name": name, "rect": rect, "background": "white"})).unwrap()["layer"].as_u64().unwrap();
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

/// Runs the export into a fresh temp file and returns the PDF's bytes.
fn export(s: &mut Session, tag: &str, params: Value) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!("photocraft-abpdf-bg-{}-{tag}.pdf", std::process::id()));
    let mut p = params;
    p["path"] = json!(path.to_string_lossy());
    s.execute("file.export.artboardsToPdf", p).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    bytes
}

/// One image XObject of a PDF: its object number, dictionary text and stream bytes.
struct Image {
    num: usize,
    dict: String,
    data: Vec<u8>,
}

/// The first index at or after `from` where `needle` starts in `hay`.
fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Every image XObject in `pdf`, in file order. Searches bytes: the JPEG streams are binary, so
/// offsets in a lossy text copy would drift.
fn images(pdf: &[u8]) -> Vec<Image> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = find(pdf, b"/Subtype /Image", from) {
        let head = pdf[..at].windows(7).rposition(|w| w == b" 0 obj\n").unwrap();
        let line = pdf[..head].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let num: usize = String::from_utf8_lossy(&pdf[line..head]).parse().unwrap();
        let start = find(pdf, b">>\nstream\n", at).unwrap();
        let dict = String::from_utf8_lossy(&pdf[head..start + 2]).into_owned();
        let len: usize = dict.split("/Length ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        let data_at = start + b">>\nstream\n".len();
        out.push(Image { num, dict, data: pdf[data_at..data_at + len].to_vec() });
        from = data_at + len;
    }
    out
}

/// A page's colour image decoded to 8-bit RGBA, row by row.
fn colours(img: &Image) -> (u32, u32, Vec<[u8; 4]>) {
    let doc = photocraft_io::import("page.jpg", &img.data).unwrap().document;
    let px = photocraft_compose::flatten(&doc).px.iter().map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
    (doc.size.width, doc.size.height, px)
}

/// A soft mask's Flate stream decoded to its alpha bytes.
fn alpha(mask: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(mask.data.as_slice()), &mut out).unwrap();
    out
}

/// The image the colour image's `/SMask N 0 R` names.
fn mask_of<'a>(all: &'a [Image], colour: &Image) -> &'a Image {
    let n: usize = colour.dict.split("/SMask ").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
    all.iter().find(|i| i.num == n).unwrap()
}

fn close(a: [u8; 4], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

#[test]
fn ticked_by_default_keeps_todays_opaque_pages() {
    let (mut s, [red, blue, empty]) = session(8);
    let default = export(&mut s, "default", json!({"artboards": [red, blue, empty]}));
    let ticked = export(&mut s, "ticked", json!({"artboards": [red, blue, empty], "includeBackground": true}));
    assert_eq!(default, ticked);
    let text = String::from_utf8_lossy(&default);
    assert!(!text.contains("/SMask") && !text.contains("/FlateDecode"));
    let all = images(&default);
    assert_eq!(all.len(), 3);
    // The Red Board page: white where the board has no pixels, red inside the layer.
    let (w, h, px) = colours(all.iter().find(|i| i.dict.contains("/Width 200 /Height 150")).unwrap());
    assert_eq!((w, h), (200, 150));
    assert!(close(px[(60 * w + 150) as usize], [255, 255, 255], 3));
    assert!(close(px[(60 * w + 70) as usize], [220, 30, 30], 12));
}

#[test]
fn unticked_the_red_board_page_gets_a_soft_mask() {
    let (mut s, [red, ..]) = session(8);
    let pdf = export(&mut s, "unticked", json!({"artboards": [red], "includeBackground": false}));
    let all = images(&pdf);
    assert_eq!(all.len(), 2, "the colour image and its soft mask");
    let colour = all.iter().find(|i| i.dict.contains("/DeviceRGB")).unwrap();
    let mask = mask_of(&all, colour);
    for key in ["/Width 200", "/Height 150", "/ColorSpace /DeviceGray", "/BitsPerComponent 8", "/Matte [1 1 1]", "/Filter /FlateDecode"] {
        assert!(mask.dict.contains(key), "{key} in {}", mask.dict);
    }
    let a = alpha(mask);
    assert_eq!(a.len(), 200 * 150);
    for y in 0..150u32 {
        for x in 0..200u32 {
            let inside = (20..120).contains(&x) && (20..100).contains(&y);
            assert_eq!(a[(y * 200 + x) as usize], if inside { 255 } else { 0 }, "({x}, {y})");
        }
    }
    // The measured sample: alpha 0 at page pixel 150,60.
    assert_eq!(a[60 * 200 + 150], 0);
    // The colours inside the red layer stay red (within JPEG tolerance).
    let (w, _, px) = colours(colour);
    for (x, y) in [(30, 30), (70, 60), (110, 90)] {
        assert!(close(px[(y * w + x) as usize], [220, 30, 30], 12), "({x}, {y}): {:?}", px[(y * w + x) as usize]);
    }
}

#[test]
fn unticked_every_page_with_layers_is_masked_and_the_xref_still_points_at_objects() {
    let (mut s, [red, blue, empty]) = session(8);
    let pdf = export(&mut s, "all", json!({"artboards": [red, blue, empty], "includeBackground": false}));
    let text = String::from_utf8_lossy(&pdf);
    assert_eq!(text.matches("/Type /Page ").count(), 3);
    // Red and Blue get a mask; the empty board was not measured and keeps today's white page.
    assert_eq!(text.matches("/SMask ").count(), 2);
    assert_eq!(text.matches("/Matte [1 1 1]").count(), 2);
    let all = images(&pdf);
    let empty_page = all.iter().find(|i| i.dict.contains("/Width 100 /Height 100") && i.dict.contains("/DeviceRGB")).unwrap();
    assert!(!empty_page.dict.contains("/SMask"));
    let (_, _, px) = colours(empty_page);
    assert!(px.iter().all(|p| close(*p, [255, 255, 255], 3)));
    // Blue Board: transparent outside its layer (board-local 10,10-110,110), opaque inside.
    let blue_page = all.iter().find(|i| i.dict.contains("/Width 200 /Height 200") && i.dict.contains("/DeviceRGB")).unwrap();
    let a = alpha(mask_of(&all, blue_page));
    assert_eq!((a[5 * 200 + 5], a[50 * 200 + 50], a[150 * 200 + 150]), (0, 255, 0));
    // Every xref entry points at its own object.
    let at = find(&pdf, b"startxref\n", 0).unwrap() + b"startxref\n".len();
    let xref: usize = String::from_utf8_lossy(&pdf[at..]).lines().next().unwrap().parse().unwrap();
    let table = String::from_utf8_lossy(&pdf[xref..]).into_owned();
    let entries: Vec<usize> = table.lines().skip(3).take_while(|l| l.ends_with(" n ")).map(|l| l[..10].parse().unwrap()).collect();
    assert_eq!(entries.len(), 2 + 3 * 3 + 2);
    for (i, off) in entries.iter().enumerate() {
        assert!(pdf[*off..].starts_with(format!("{} 0 obj\n", i + 1).as_bytes()), "object {}", i + 1);
    }
}

#[test]
fn unticked_a_board_its_layer_covers_entirely_stays_opaque() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 200, "background": "transparent"})).unwrap();
    let id = s.execute("layer.new.artboard", json!({"name": "Full", "rect": [0, 0, 120, 80]})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": id})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Cover"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 120, "height": 80, "antiAlias": false})).unwrap();
    s.execute("edit.fill", json!({"color": "#DC1E1E"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let pdf = export(&mut s, "cover", json!({"includeBackground": false}));
    let all = images(&pdf);
    assert_eq!(all.len(), 1, "no mask when every pixel is opaque");
    assert!(!all[0].dict.contains("/SMask"));
    let (w, h, px) = colours(&all[0]);
    assert_eq!((w, h), (120, 80));
    assert!(px.iter().all(|p| close(*p, [220, 30, 30], 12)));
}

#[test]
fn the_cases_not_measured_keep_todays_page() {
    // 16 bit and 32 bit: no mask, white where the board has no pixels.
    for depth in [16, 32] {
        let (mut s, [red, ..]) = session(depth);
        let pdf = export(&mut s, &format!("depth{depth}"), json!({"artboards": [red], "includeBackground": false}));
        assert!(!String::from_utf8_lossy(&pdf).contains("/SMask"), "{depth} bit");
        let all = images(&pdf);
        let (w, _, px) = colours(&all[0]);
        assert!(close(px[(60 * w + 150) as usize], [255, 255, 255], 3), "{depth} bit");
    }
    // A black board keeps its fill.
    let (mut s, [red, ..]) = session(8);
    s.execute("layer.artboard.set", json!({"layer": red, "background": "black"})).unwrap();
    let pdf = export(&mut s, "black", json!({"artboards": [red], "includeBackground": false}));
    assert!(!String::from_utf8_lossy(&pdf).contains("/SMask"));
    let all = images(&pdf);
    let (w, _, px) = colours(&all[0]);
    assert!(close(px[(60 * w + 150) as usize], [0, 0, 0], 3));
}

#[test]
fn the_source_document_is_unchanged_and_gets_no_history_entry() {
    for background in [true, false] {
        let (mut s, [red, blue, empty]) = session(8);
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.entries();
        export(&mut s, &format!("src{background}"), json!({"artboards": [red, blue, empty], "includeBackground": background}));
        let d = s.active().unwrap();
        assert_eq!(d.history.entries(), steps, "includeBackground {background}");
        assert!(d.doc.artboards().iter().all(|b| b.2.background == ArtboardBackground::White), "includeBackground {background}");
        assert_eq!(d.doc.layers.len(), before.layers.len());
    }
}

#[test]
fn a_param_that_is_not_a_bool_does_not_panic() {
    let (mut s, [red, ..]) = session(8);
    for bad in [json!("no"), json!(0), json!(null), json!([false]), json!({"x": 1})] {
        let pdf = export(&mut s, "bad", json!({"artboards": [red], "includeBackground": bad}));
        assert!(!String::from_utf8_lossy(&pdf).contains("/SMask"), "{bad}: treated as the default");
    }
    assert!(s.execute("file.export.artboardsToPdf", json!({"includeBackground": false})).is_err(), "no path");
}
