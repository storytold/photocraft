//! #14425: a 3D `.cube` with a DOMAIN_MAX line (DOMAIN_MIN at 0) loads in Color Lookup and the
//! domain is ignored, as in the reference application: the table applies over 0..1. A raised
//! DOMAIN_MIN, a `LUT_3D_INPUT_RANGE` line and a 1D `.cube` with a domain keep today's behaviour.

use serde_json::{Value, json};

use crate::Session;

const LOOKUP: &str = "image.adjustments.colorLookup";

/// A 2x2x2 `.cube` identity table (red fastest) with `header` lines before the data.
fn identity_cube(header: &str) -> String {
    let mut text = format!("TITLE \"dom\"\nLUT_3D_SIZE 2\n{header}\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                text.push_str(&format!("{r} {g} {b}\n"));
            }
        }
    }
    text
}

/// The issue's test strips: an 8-bit RGB document whose Background holds gray levels 0, 16, …,
/// 240, 255 in its first row and a few colours in its second.
fn session() -> (Session, Vec<[f32; 4]>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 18, "height": 2, "depth": 8, "background": "white"})).unwrap();
    let mut levels: Vec<f32> = (0..16).map(|i| (i * 16) as f32 / 255.0).collect();
    levels.push(1.0);
    let colours = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.8, 0.4, 0.2], [0.2, 0.6, 0.9]];
    let mut strips = Vec::new();
    for (x, v) in levels.iter().enumerate() {
        strips.push((x as i64, 0, [*v, *v, *v]));
    }
    for (x, c) in colours.iter().enumerate() {
        strips.push((x as i64, 1, *c));
    }
    for (x, y, [r, g, b]) in strips {
        s.execute("select.rect", json!({"x": x, "y": y, "width": 1, "height": 1})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": [r, g, b, 1.0]})).unwrap();
    }
    s.execute("select.deselect", json!({})).unwrap();
    let before = pixels(&s);
    (s, before)
}

fn pixels(s: &Session) -> Vec<[f32; 4]> {
    let st = s.active().unwrap();
    let surface = st.doc.layers[0].surface().unwrap();
    (0..2).flat_map(|y| (0..18).map(move |x| (x, y))).map(|(x, y)| surface.rgba(x, y)).collect()
}

fn lookup(s: &mut Session, text: &str) -> crate::Result<Value> {
    s.execute(LOOKUP, json!({"data": text, "fileName": "dom2.cube"}))
}

fn newest_step(s: &Session) -> Option<String> {
    s.active().unwrap().history.entries().last().cloned()
}

#[test]
fn a_3d_cube_with_domain_lines_loads_and_applies_over_0_to_1() {
    for header in ["DOMAIN_MAX 2 2 2", "DOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2", "DOMAIN_MAX 4 3 2"] {
        let (mut s, before) = session();
        lookup(&mut s, &identity_cube(header)).unwrap_or_else(|e| panic!("{header}: {e}"));
        assert_eq!(pixels(&s), before, "{header}: the identity table leaves every strip unchanged (not in/2)");
        assert_eq!(newest_step(&s).as_deref(), Some("Color Lookup"), "{header}");
    }
}

#[test]
fn a_3d_input_range_line_keeps_todays_refusal() {
    let (mut s, before) = session();
    let steps = s.active().unwrap().history.past_len();
    let err = lookup(&mut s, &identity_cube("LUT_3D_INPUT_RANGE 0 2")).expect_err("an input range is still refused");
    assert!(err.to_string().contains("DOMAIN_MAX"), "{err}");
    assert_eq!(pixels(&s), before);
    assert_eq!(s.active().unwrap().history.past_len(), steps, "no history state");
}

#[test]
fn a_raised_domain_min_keeps_todays_refusal() {
    for header in ["DOMAIN_MIN 0.5 0.5 0.5", "DOMAIN_MIN 0.5 0.5 0.5\nDOMAIN_MAX 2 2 2"] {
        let (mut s, before) = session();
        assert!(lookup(&mut s, &identity_cube(header)).is_err(), "{header}: not measured, still refused");
        assert_eq!(pixels(&s), before);
    }
}

#[test]
fn a_1d_cube_with_a_domain_keeps_todays_baked_domain() {
    // A 1D identity ramp over 0..2 bakes the domain into the table, so a level reads at half.
    let text = "LUT_1D_SIZE 2\nDOMAIN_MAX 2 2 2\n0 0 0\n1 1 1\n";
    let (mut s, before) = session();
    lookup(&mut s, text).expect("a 1D .cube with a domain loads as before");
    let after = pixels(&s);
    let white = before.iter().position(|p| p[0] == 1.0 && p[1] == 1.0 && p[2] == 1.0).unwrap();
    assert!((after[white][0] - 0.5).abs() < 2.0 / 255.0, "white reads the ramp at 0.5: {:?}", after[white]);
}

#[test]
fn a_layer_with_domain_lines_reopens_from_psd_and_psb() {
    use photocraft_doc::{Adjustment, LayerContent};
    // The layer embeds its file in a PSD, and a PSD read still refuses a custom domain, so the
    // save writes the same table as a plain 0..1 `.cube` and the layer reopens as Color Lookup.
    // An inverting table, so a dropped or identity table would not pass.
    let mut invert = String::from("TITLE \"inv\"\nLUT_3D_SIZE 2\nDOMAIN_MAX 2 2 2\n");
    for b in 0..2 {
        for g in 0..2 {
            for r in 0..2 {
                invert.push_str(&format!("{} {} {}\n", 1 - r, 1 - g, 1 - b));
            }
        }
    }
    let (mut s, _) = session();
    let r = s.execute("layer.newAdjustmentLayer.colorLookup", json!({"data": invert, "fileName": "dom2.cube"})).unwrap();
    let id = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    let doc = &s.active().unwrap().doc;
    let Some(LayerContent::Adjustment(Adjustment::ColorLookup { lut: Some(want), size: 2, .. })) = doc.layer(id).map(|l| &l.content) else {
        panic!("not a 2³ Color Lookup layer")
    };
    assert_eq!(want.get(..3), Some(&[1.0, 1.0, 1.0][..]), "the table inverts");
    for (file, psb) in [("x.psd", false), ("x.psb", true)] {
        let opts = photocraft_io::ExportOptions { force_psb: psb, ..Default::default() };
        let bytes = photocraft_io::export(doc, file, &opts).unwrap().bytes;
        let back = photocraft_io::import(file, &bytes).unwrap().document;
        let layer = back.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == doc.layer(id).unwrap().name).unwrap();
        let LayerContent::Adjustment(Adjustment::ColorLookup { name, lut, size, .. }) = &layer.content else {
            panic!("{file}: reopened as {:?}", layer.content)
        };
        assert_eq!((name.as_str(), *size), ("dom2.cube", 2), "{file}");
        assert_eq!(lut.as_deref(), Some(&**want), "{file}: the same table");
    }
}

#[test]
fn a_cube_without_domain_lines_is_unchanged() {
    let (mut s, before) = session();
    lookup(&mut s, &identity_cube("")).unwrap();
    assert_eq!(pixels(&s), before);
}
