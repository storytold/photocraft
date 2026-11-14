//! Artboards to Files › Include Artboard Name (#13637): a border of the Canvas Extension colour
//! on all four sides, the board's name in the top one, at 8 and 16 bit; bad Name Options fail the
//! export before any file is written; without the option the output is today's.

use super::*;

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const RED: [f32; 4] = [220.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];

/// The spec's test document: 800 x 400, transparent, `Red Board` 0,0-200,150 (white, a red
/// layer at 20,20-120,100) and `Blue Board` 120,30-320,230 (white, a blue layer at 130,40-230,140).
fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400, "depth": depth, "background": "transparent"})).unwrap();
    board(&mut s, "Red Board", [0, 0, 200, 150], Rect::new(20, 20, 120, 100), RED);
    board(&mut s, "Blue Board", [120, 30, 200, 200], Rect::new(130, 40, 230, 140), [30.0 / 255.0, 30.0 / 255.0, 220.0 / 255.0, 1.0]);
    s
}

fn board(s: &mut Session, name: &str, rect: [i32; 4], px: Rect, rgba: [f32; 4]) {
    let r = s.execute("layer.new.artboard", json!({"rect": rect, "name": name, "background": "white"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    s.edit("px", |doc, _| {
        let fmt = doc.pixel_format();
        let mut l = Layer::raster(name.replace(" Board", ""), fmt);
        l.surface_mut().unwrap().fill_rect(px, &photocraft_raster::from_rgba(&fmt, rgba));
        doc.layer_mut(id).and_then(Layer::children_mut).unwrap().push(l);
        Ok(())
    })
    .unwrap();
}

/// An empty export folder, removed when dropped (also when an assertion fails).
struct Tmp {
    _dir: tempfile::TempDir,
    path: String,
}

impl std::ops::Deref for Tmp {
    type Target = str;
    fn deref(&self) -> &str {
        &self.path
    }
}

fn tmp(tag: &str) -> Tmp {
    let dir = tempfile::Builder::new().prefix(&format!("photocraft-abname-{tag}-")).tempdir().unwrap();
    let path = dir.path().to_string_lossy().into_owned();
    Tmp { _dir: dir, path }
}

/// Runs the export and reads every written PNG back, keyed by the board name in its file name.
fn export(s: &mut Session, dir: &str, extra: Value) -> Vec<(String, Document)> {
    let mut p = json!({"dir": dir, "prefix": "t", "format": "png"});
    if let (Some(p), Value::Object(extra)) = (p.as_object_mut(), extra) {
        p.extend(extra);
    }
    let r = s.execute("file.export.artboardsToFiles", p).unwrap();
    r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let f = f.as_str().unwrap();
            let name = if f.contains("Red Board") { "Red" } else { "Blue" };
            (name.to_string(), photocraft_io::import("x.png", &std::fs::read(f).unwrap()).unwrap().document)
        })
        .collect()
}

fn find<'a>(files: &'a [(String, Document)], name: &str) -> &'a Document {
    &files.iter().find(|(n, _)| n == name).unwrap().1
}

fn px(d: &Document, x: i32, y: i32) -> [f32; 4] {
    photocraft_compose::flatten(d).get(x, y)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01)
}

fn on() -> Value {
    json!({"includeName": true})
}

#[test]
fn the_border_width_is_a_twentieth_of_width_plus_height() {
    // The three borders measured in the reference application.
    assert_eq!(name_border_width(200, 150), 18);
    assert_eq!(name_border_width(200, 200), 20);
    assert_eq!(name_border_width(800, 400), 60);
}

#[test]
fn the_boards_get_the_measured_sizes_with_their_content_unchanged_inside() {
    for depth in [8, 16] {
        let mut s = session(depth);
        let plain = export(&mut s, &tmp(&format!("plain{depth}")), json!({}));
        let named = export(&mut s, &tmp(&format!("named{depth}")), on());
        let (red, blue) = (find(&named, "Red"), find(&named, "Blue"));
        assert_eq!((red.size.width, red.size.height), (236, 186), "{depth} bit");
        assert_eq!((blue.size.width, blue.size.height), (240, 240), "{depth} bit");
        // Content at 18..218 x 18..168, pixel for pixel the file without the option.
        let (before, after) = (photocraft_compose::flatten(find(&plain, "Red")), photocraft_compose::flatten(red));
        for y in (0..150).step_by(7) {
            for x in (0..200).step_by(7) {
                assert!(close(before.get(x, y), after.get(x + 18, y + 18)), "{depth} bit at {x},{y}");
            }
        }
        assert!(close(px(red, 18 + 50, 18 + 50), RED), "{depth} bit");
        assert!(close(px(red, 18, 18), WHITE), "{depth} bit: the board's own white");
        // The border is opaque white on every side.
        for (x, y) in [(0, 0), (235, 185), (5, 100), (230, 100), (100, 180), (17, 17)] {
            assert!(close(px(red, x, y), WHITE), "{depth} bit at {x},{y}");
        }
    }
}

#[test]
fn the_name_sits_in_the_top_border_from_the_boards_left_edge() {
    let mut s = session(8);
    let files = export(&mut s, &tmp("text"), on());
    for (name, border) in [("Red", 18), ("Blue", 20)] {
        let img = photocraft_compose::flatten(find(&files, name));
        let w = find(&files, name).size.width as i32;
        let dark: Vec<(i32, i32)> = (0..border).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| img.get(x, y)[0] < 0.5).collect();
        assert!(!dark.is_empty(), "{name}: the name is drawn in the top border");
        let x0 = dark.iter().map(|p| p.0).min().unwrap();
        assert!((x0 - border).abs() <= 1, "{name}: ink starts at the board's left edge, got {x0}");
        let (y0, y1) = (dark.iter().map(|p| p.1).min().unwrap(), dark.iter().map(|p| p.1).max().unwrap());
        assert!(y0 > 0 && y1 < border - 1, "{name}: ink inside the border, rows {y0}..{y1}");
        // Black text by default: the darkest ink is near black and neutral (small anti-aliased
        // glyphs may not reach pure black with every font).
        let ink = dark.iter().map(|&(x, y)| img.get(x, y)).min_by(|a, b| a[0].total_cmp(&b[0])).unwrap();
        assert!(ink[0] < 0.3 && (ink[0] - ink[1]).abs() < 0.02 && (ink[0] - ink[2]).abs() < 0.02, "{name}: {ink:?}");
    }
}

#[test]
fn a_name_taller_than_the_border_never_draws_over_the_board() {
    let mut s = session(8);
    let plain = export(&mut s, &tmp("plain-big"), json!({}));
    for size in [72, 400, 1296] {
        let named = export(&mut s, &tmp("big"), json!({"includeName": true, "fontSize": size}));
        let (before, after) = (photocraft_compose::flatten(find(&plain, "Red")), photocraft_compose::flatten(find(&named, "Red")));
        for y in 0..150 {
            for x in (0..200).step_by(3) {
                assert!(close(before.get(x, y), after.get(x + 18, y + 18)), "{size} pt at {x},{y}");
            }
        }
        // The cut name still shows in the border.
        assert!((0..18).any(|y| (18..236).any(|x| after.get(x, y)[0] < 0.5)), "{size} pt");
    }
}

#[test]
fn canvas_extension_black_fills_the_border_black() {
    let mut s = session(8);
    let files = export(&mut s, &tmp("black"), json!({"includeName": true, "extension": "black", "textColor": [1.0, 1.0, 1.0]}));
    let red = find(&files, "Red");
    for (x, y) in [(0, 0), (235, 185), (5, 100), (230, 100), (100, 180)] {
        assert!(close(px(red, x, y), BLACK), "at {x},{y}");
    }
    assert!(close(px(red, 18, 18), WHITE), "the board keeps its own white");
    // An "#rrggbb" extension colour too.
    let files = export(&mut s, &tmp("hex"), json!({"includeName": true, "extension": "#00ff00"}));
    assert!(close(px(find(&files, "Red"), 2, 100), [0.0, 1.0, 0.0, 1.0]));
    // The dialog's Other... sends the swatch colour along.
    let files = export(&mut s, &tmp("other"), json!({"includeName": true, "extension": "other", "extensionColor": [0.0, 0.0, 1.0]}));
    assert!(close(px(find(&files, "Red"), 2, 100), [0.0, 0.0, 1.0, 1.0]));
}

#[test]
fn without_the_option_the_output_is_todays() {
    let mut s = session(8);
    for extra in [json!({}), json!({"includeName": false, "fontSize": "abc"})] {
        let files = export(&mut s, &tmp("off"), extra);
        let red = find(&files, "Red");
        assert_eq!((red.size.width, red.size.height), (200, 150));
        assert!(close(px(red, 50, 50), RED));
    }
    // PSD keeps its layers: the option leaves it as it is today.
    let d = tmp("psd");
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": &*d, "format": "psd", "includeName": true})).unwrap();
    for f in r["files"].as_array().unwrap() {
        let psd = photocraft_io::import("x.psd", &std::fs::read(f.as_str().unwrap()).unwrap()).unwrap().document;
        assert!(matches!((psd.size.width, psd.size.height), (200, 150) | (200, 200)), "{:?}", psd.size);
    }
}

#[test]
fn a_size_that_is_not_a_positive_number_fails_and_writes_nothing() {
    let mut s = session(8);
    for size in [json!("abc"), json!(""), json!("0"), json!(0), json!(-3), json!("-1"), json!("inf"), json!({"x": 1}), json!([12])] {
        let d = tmp("bad");
        let before = s.active().unwrap().doc.clone();
        let err = s.execute("file.export.artboardsToFiles", json!({"dir": &*d, "includeName": true, "fontSize": size})).unwrap_err().to_string();
        assert!(err.contains("size"), "{size}: {err}");
        assert_eq!(std::fs::read_dir(&*d).unwrap().count(), 0, "{size}: no file written");
        assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len(), "{size}: source unchanged");
    }
    // A numeric string is its number, as the dialog's free text field sends it.
    let files = export(&mut s, &tmp("str"), json!({"includeName": true, "fontSize": " 9 "}));
    assert_eq!(find(&files, "Red").size.width, 236);
}

#[test]
fn bad_name_options_never_panic() {
    let mut s = session(8);
    let d = tmp("junk");
    for p in [
        json!({"extension": "purple"}),
        json!({"extension": [f64::MAX, -1.0, 2.0]}),
        json!({"extension": {"r": 1}}),
        json!({"extension": "#12"}),
        json!({"textColor": "zz"}),
        json!({"textColor": [1, 2]}),
        json!({"font": 7, "fontStyle": null}),
        json!({"font": "", "fontStyle": "No Such Style"}),
        json!({"font": "No Such Font Family", "fontSize": 1e300}),
        json!({"fontSize": 1e-30}),
        json!({"includeName": "yes"}),
    ] {
        let mut p = p;
        p["dir"] = json!(&*d);
        if p.get("includeName").is_none() {
            p["includeName"] = json!(true);
        }
        let _ = s.execute("file.export.artboardsToFiles", p);
    }
    assert!(s.execute("file.export.artboardsToFiles", json!({"dir": &*d, "includeName": true, "extension": "purple"})).is_err());
}
