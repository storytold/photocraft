//! #14095: when images cannot be aligned, Photomerge warns "Some images could not be automatically aligned" and still
//! builds the result with every source as a layer. With none aligned it uses the unaligned layout: the canvas as wide
//! as the sources side by side with a 10 px gap between each two, as tall as the tallest, every layer at 0,0. With some
//! aligned, the ones that failed still get a layer. The result keeps listing the failed images in `failed`.

#![cfg(not(target_arch = "wasm32"))]

use super::*;

/// An RGB document `name` of `w`x`h` cut at `x0` from a `sw`-wide image `sc`.
fn tile(name: &str, sc: &[[f32; 4]], sw: usize, x0: usize, w: usize, h: usize) -> Document {
    let mut d = Document::new(name, Size::new(w as u32, h as u32), ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; w * h * n];
    for (i, o) in data.chunks_exact_mut(n).enumerate() {
        photocraft_raster::from_rgba_into(&fmt, sc[(i / w) * sw + x0 + i % w], o);
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    d.layers.push(Layer::new("Background", LayerContent::Raster(surf)));
    d
}

/// A deterministic pseudo-random sequence in 0..1.
fn rng(seed: u64) -> impl FnMut() -> f32 {
    let mut z = seed;
    move || {
        z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((z >> 33) as f64 / (1u64 << 31) as f64) as f32
    }
}

/// A textured scene: a gradient with scattered squares, so overlapping cuts of it align.
fn scene(w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut rnd = rng(7);
    let mut px: Vec<[f32; 4]> = (0..w * h).map(|i| [0.3 + 0.2 * (i % w) as f32 / w as f32, 0.35, 0.4, 1.0]).collect();
    for _ in 0..(w * h / 600) {
        let (cx, cy, r) = ((rnd() * w as f32) as usize, (rnd() * h as f32) as usize, 2 + (rnd() * 8.0) as usize);
        let col = [rnd(), rnd(), rnd(), 1.0];
        for y in cy.saturating_sub(r)..(cy + r).min(h) {
            for x in cx.saturating_sub(r)..(cx + r).min(w) {
                px[y * w + x] = col;
            }
        }
    }
    px
}

/// Per-pixel noise that matches nothing.
fn noise(w: usize, h: usize, seed: u64) -> Vec<[f32; 4]> {
    let mut rnd = rng(seed);
    (0..w * h).map(|_| [rnd(), rnd(), rnd(), 1.0]).collect()
}

/// A session with `docs` open.
fn session(docs: Vec<Document>) -> Session {
    let mut s = Session::new();
    for d in docs {
        s.add_document(d, None);
    }
    s
}

/// The result document's layers, top first: (name, pixel at 0,0 of the layer's surface).
fn layers(s: &Session) -> Vec<(String, [f32; 4])> {
    let doc = &s.active().unwrap().doc;
    doc.layers
        .iter()
        .rev()
        .map(|l| {
            let LayerContent::Raster(px) = &l.content else { panic!("{} is a pixel layer", l.name) };
            (l.name.clone(), px.rgba(0, 0))
        })
        .collect()
}

fn near(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0 / 255.0 + 1e-6)
}

#[test]
fn with_no_image_aligned_the_merge_uses_the_unaligned_layout_and_warns() {
    let (a, b) = (noise(300, 300, 11), noise(300, 300, 23));
    let mut s = session(vec![tile("t1", &a, 300, 0, 300, 300), tile("far", &b, 300, 0, 300, 300)]);
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true})).unwrap();
    assert_eq!(r["warning"], json!(PHOTOMERGE_UNALIGNED), "{r}");
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(610), Some(300)), "300 + 300 + 10, the tallest: {r}");
    let mut failed: Vec<&str> = r["failed"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    failed.sort_unstable();
    assert_eq!(failed, ["far", "t1"]);
    let size = s.active().unwrap().doc.size;
    assert_eq!((size.width, size.height), (610, 300));
    let got = layers(&s);
    assert_eq!(got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["t1", "far"], "every source a layer, the first on top");
    // Each layer is its source unmoved: its top-left pixel is the source's.
    assert!(near(got[0].1, a[0]) && near(got[1].1, b[0]), "{got:?}");
}

#[test]
fn three_tiles_side_by_side_count_two_gaps() {
    let (a, b, c) = (noise(300, 300, 3), noise(300, 280, 5), noise(300, 260, 9));
    let mut s = session(vec![tile("a", &a, 300, 0, 300, 300), tile("b", &b, 300, 0, 300, 280), tile("c", &c, 300, 0, 300, 260)]);
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "blend": false})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(920), Some(300)), "{r}");
    assert_eq!(layers(&s).len(), 3);
}

#[test]
fn an_image_that_fails_beside_aligned_ones_still_gets_its_layer() {
    let (sw, h) = (700usize, 300usize);
    let sc = scene(sw, h);
    let far = noise(300, 300, 41);
    let docs = vec![tile("t1", &sc, sw, 0, 300, h), tile("t2", &sc, sw, 200, 300, h), tile("t3", &sc, sw, 400, 300, h), tile("far", &far, 300, 0, 300, 300)];
    let mut s = session(docs);
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition"})).unwrap();
    assert_eq!(r["failed"], json!(["far"]), "{r}");
    assert_eq!(r["warning"], json!(PHOTOMERGE_UNALIGNED));
    let got = layers(&s);
    let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["t1", "t2", "t3", "far"], "no source is missing, the first on top");
    assert!(near(got[3].1, far[0]), "the failed image is its source at 0,0: {:?}", got[3].1);
}

#[test]
fn a_merge_that_aligns_everything_has_no_warning() {
    let (sw, h) = (500usize, 300usize);
    let sc = scene(sw, h);
    let mut s = session(vec![tile("t1", &sc, sw, 0, 300, h), tile("t2", &sc, sw, 200, 300, h)]);
    let r = s.execute("file.automate.photomerge", json!({"useOpenDocuments": true, "layout": "reposition"})).unwrap();
    assert!(r["failed"].as_array().unwrap().is_empty() && r["warning"].is_null(), "{r}");
    assert_eq!(layers(&s).len(), 2);
}

#[test]
fn the_blend_allowance_leaves_room_for_the_unplaced_sources() {
    // Without unplaced sources the allowance is as before: the budget less the fixed buffers.
    assert_eq!(blend_band_budget(1000, 300, 0), 700);
    // Retained unplaced sources stay allocated during the blend, so they come off the allowance too.
    assert_eq!(blend_band_budget(1000, 300, 200), 500);
    // Never negative.
    assert_eq!(blend_band_budget(1000, 900, 200), 0);
}
