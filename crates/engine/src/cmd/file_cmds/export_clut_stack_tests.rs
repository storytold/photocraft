//! #13742: an exported colour lookup table holds the whole visible stack above the Background,
//! pixel layers and masks included, with the lattice laid out over the canvas in place of the
//! Background's pixels. Values from the issue's measurements, within 1/4096.

use super::*;
use photocraft_doc::LayerMask;
use serde_json::json;

const RED: [f32; 4] = [200.0 / 255.0, 30.0 / 255.0, 30.0 / 255.0, 1.0];
const GREEN: [f32; 4] = [0.0, 160.0 / 255.0, 0.0, 1.0];

/// The issue's document: 200 x 150 RGB/8, Background red on the left half and white on the
/// right, an Invert adjustment layer on top. Returns the session and the Invert's id.
fn session() -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 150})).unwrap();
    {
        let st = s.active_mut().unwrap();
        let doc = std::sync::Arc::make_mut(&mut st.doc);
        let surface = doc.layers[0].surface_mut().unwrap();
        let red = photocraft_raster::from_rgba(&surface.format(), RED);
        surface.fill_rect(Rect::new(0, 0, 100, 150), &red);
    }
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let invert = s.active().unwrap().active_layer.unwrap();
    (s, invert)
}

fn edit(s: &mut Session, f: impl FnOnce(&mut Document)) {
    let st = s.active_mut().unwrap();
    f(std::sync::Arc::make_mut(&mut st.doc));
}

/// A filled 200 x 150 green pixel layer at `opacity`.
fn green(doc: &Document, opacity: f32) -> Layer {
    let fmt = doc.pixel_format();
    let mut l = Layer::raster("Green", fmt);
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 150), &photocraft_raster::from_rgba(&fmt, GREEN));
    l.opacity = opacity;
    l
}

/// The table's entries, indexed `b·n² + g·n + r`.
fn entries(s: &mut Session) -> Vec<[f32; 3]> {
    let r = s.execute("file.export.colorLookupTables", json!({"size": 7})).unwrap();
    parse(r["cube"].as_str().unwrap())
}

fn parse(cube: &str) -> Vec<[f32; 3]> {
    cube.lines()
        .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|l| {
            let v: Vec<f32> = l.split(' ').map(|x| x.parse().unwrap()).collect();
            [v[0], v[1], v[2]]
        })
        .collect()
}

fn at(e: &[[f32; 3]], r: usize, g: usize, b: usize) -> [f32; 3] {
    e[b * 49 + g * 7 + r]
}

fn near(got: [f32; 3], want: [f32; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.0 / 4096.0)
}

fn input(r: usize, g: usize, b: usize) -> [f32; 3] {
    [r as f32 / 6.0, g as f32 / 6.0, b as f32 / 6.0]
}

#[test]
fn an_invert_over_the_background_is_the_exact_inverse_whatever_the_backgrounds_pixels() {
    let (mut s, _) = session();
    let e = entries(&mut s);
    assert_eq!(e.len(), 343);
    assert!(near(at(&e, 0, 0, 0), [1.0; 3]) && near(at(&e, 6, 6, 6), [0.0; 3]));
    for b in 0..7 {
        for g in 0..7 {
            for r in 0..7 {
                let i = input(r, g, b);
                assert!(near(at(&e, r, g, b), i.map(|v| 1.0 - v)), "({r},{g},{b})");
            }
        }
    }
}

#[test]
fn a_hidden_invert_gives_the_identity_with_no_error() {
    let (mut s, invert) = session();
    edit(&mut s, |d| d.layer_mut(invert).unwrap().visible = false);
    // The command itself stays disabled without a visible adjustment layer (its menu rules are
    // #13743's); the table it bakes is the identity, not an error.
    let doc = s.active().unwrap().doc.clone();
    let e = parse(&bake_cube_stack(7, "t", &doc).unwrap());
    assert!(near(at(&e, 0, 0, 0), [0.0; 3]) && near(at(&e, 3, 3, 3), [0.5; 3]) && near(at(&e, 6, 6, 6), [1.0; 3]));
}

/// A PSD's 50 % opacity is 128/255.
#[test]
fn pixel_layers_above_and_below_the_invert_count() {
    let (mut s, _) = session();
    edit(&mut s, |d| {
        let l = green(d, 128.0 / 255.0);
        d.layers.push(l);
    });
    let e = entries(&mut s);
    assert!(near(at(&e, 0, 0, 0), [0.498047, 0.812988, 0.498047]), "{:?}", at(&e, 0, 0, 0));
    assert!(near(at(&e, 6, 6, 6), [0.0, 0.314941, 0.0]), "{:?}", at(&e, 6, 6, 6));

    let (mut s, _) = session();
    edit(&mut s, |d| {
        let l = green(d, 1.0);
        d.layers.push(l);
    });
    assert!(entries(&mut s).iter().all(|v| near(*v, [0.0, 0.627441, 0.0])), "opaque green on top: a constant");

    let (mut s, _) = session();
    edit(&mut s, |d| {
        let l = green(d, 1.0);
        d.layers.insert(1, l);
    });
    assert!(entries(&mut s).iter().all(|v| near(*v, [1.0, 0.372559, 1.0])), "opaque green under the Invert: a constant");
}

#[test]
fn a_half_masked_invert_inverts_the_entries_laid_on_its_revealed_half() {
    let (mut s, invert) = session();
    edit(&mut s, |d| {
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(0, 0, 100, 150), &[0.0]);
        d.layer_mut(invert).unwrap().mask = Some(mask);
    });
    let e = entries(&mut s);
    let (mut inverted, mut identity) = (0, 0);
    for b in 0..7 {
        for g in 0..7 {
            for r in 0..7 {
                let (v, i) = (at(&e, r, g, b), input(r, g, b));
                if near(v, i.map(|x| 1.0 - x)) {
                    inverted += 1;
                } else if near(v, i) {
                    identity += 1;
                }
            }
        }
    }
    assert_eq!((inverted, identity), (169, 174), "as measured (the grey centre counts as inverted)");
}

#[test]
fn explicit_layers_and_the_selection_keep_the_adjustment_only_table() {
    let (mut s, invert) = session();
    edit(&mut s, |d| {
        let l = green(d, 1.0);
        d.layers.push(l);
    });
    let r = s.execute("file.export.colorLookupTables", json!({"size": 3, "layers": [invert.0]})).unwrap();
    assert_eq!(r["layerCount"], 1);
    assert!(r["cube"].as_str().unwrap().contains("1.000000 1.000000 1.000000"), "the Invert alone, as before");
    let r = s.execute("file.export.colorLookupTables", json!({"size": 3, "scope": "selected"}));
    assert!(r.is_ok() || r.unwrap_err().to_string().contains("adjustment"), "the selection path is unchanged");
}

#[test]
fn small_and_large_canvases_never_panic() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1, "height": 1})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let err = s.execute("file.export.colorLookupTables", json!({"size": 129})).unwrap_err();
    assert!(err.to_string().contains("too small"), "{err}");
    let e = entries(&mut s);
    assert!(near(at(&e, 2, 4, 5), input(2, 4, 5).map(|v| 1.0 - v)), "a 1 x 1 canvas takes one pass per entry");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 12000, "height": 9000})).unwrap();
    s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let e = entries(&mut s);
    assert!(near(at(&e, 0, 0, 0), [1.0; 3]), "only the lattice rows are rendered");
}
