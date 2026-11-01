//! #9674: a type layer converted to another profile (Convert to Profile, or copied into a
//! document with another profile) keeps its converted colour through a PSD save, reopen and
//! edit. The conversion changed the layer's colour and pixels but left its `TySh` holding the
//! old colour, and the reopened text took its colour from that block.

use photocraft_doc::{Document, LayerContent, LayerId, TextLayer};
use photocraft_engine::Session;
use serde_json::{Value, json};

fn text(doc: &Document, id: LayerId) -> &TextLayer {
    let LayerContent::Text(t) = &doc.layer(id).unwrap().content else { panic!("type layer") };
    t
}

fn only_text(doc: &Document) -> LayerId {
    doc.walk().into_iter().find_map(|(_, _, l)| matches!(l.content, LayerContent::Text(_)).then_some(l.id)).unwrap()
}

/// A white 256×128 sRGB document at `depth` with an "H" in #c04080, point or paragraph type.
fn source(s: &mut Session, depth: u32, paragraph: bool) -> LayerId {
    s.execute("file.new", json!({"width": 256, "height": 128, "depth": depth, "background": "white"})).unwrap();
    s.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "relative", "bpc": true})).unwrap();
    let mut p = json!({"x": 50, "y": 80, "text": "H", "size": 48, "color": "#c04080"});
    if paragraph {
        p["box"] = json!([40, 20, 120, 80]);
    }
    LayerId(s.execute("type.create", p).unwrap()["layer"].as_u64().unwrap())
}

/// The colour channels of the layer's most opaque pixel and that pixel's position.
fn ink(t: &TextLayer) -> ((u32, u32), Vec<f32>) {
    let c = t.cache.as_ref().unwrap();
    let r = c.content_bounds();
    let px = c.read_region(r);
    let n = c.format().channels();
    let i = (0..px.len() / n).max_by(|a, b| px[a * n + n - 1].total_cmp(&px[b * n + n - 1])).unwrap();
    let at = (r.x0 as u32 + (i as u32 % r.width()), r.y0 as u32 + (i as u32 / r.width()));
    (at, px[i * n..i * n + n - 1].to_vec())
}

fn ink_at(t: &TextLayer, at: (u32, u32)) -> Vec<f32> {
    let c = t.cache.as_ref().unwrap();
    let px = c.read_region(photocraft_geom::Rect::new(at.0 as i32, at.1 as i32, at.0 as i32 + 1, at.1 as i32 + 1));
    px[..px.len() - 1].to_vec()
}

fn close(a: &[f32], b: &[f32], what: &str) {
    assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.5 / 255.0), "{what}: {a:?} vs {b:?}");
}

/// Saves `doc` as PSD, reopens it, edits H → M → H and returns the reopened session and layer.
fn reopen_and_edit(doc: &Document) -> (Session, LayerId) {
    let out = photocraft_io::export(doc, "psd", &Default::default()).unwrap();
    let back = photocraft_io::import("converted.psd", &out.bytes).unwrap().document;
    let id = only_text(&back);
    let mut s = Session::new();
    s.add_document(back, None);
    for t in ["M", "H"] {
        s.execute("type.edit", json!({"layer": id.0, "text": t})).unwrap();
    }
    (s, id)
}

#[test]
fn converted_type_keeps_its_colour_through_a_psd_reopen_and_edit() {
    for depth in [8, 16] {
        for paragraph in [false, true] {
            for copy in [false, true] {
                let case = format!("depth {depth}, paragraph {paragraph}, copy {copy}");
                let mut s = Session::new();
                let mut id = source(&mut s, depth, paragraph);
                let original = text(&s.active().unwrap().doc, id).color;
                if copy {
                    s.execute("file.new", json!({"width": 256, "height": 128, "depth": depth, "background": "white"})).unwrap();
                    s.execute("edit.convertToProfile", json!({"profile": "adobe-rgb-compat", "intent": "relative", "bpc": true})).unwrap();
                    let r: Value = s.execute("layer.copyToDocument", json!({"source": 0, "document": 1, "layers": [id.0]})).unwrap();
                    id = LayerId(r["layers"][0].as_u64().unwrap());
                } else {
                    s.execute("edit.convertToProfile", json!({"profile": "adobe-rgb-compat", "intent": "relative", "bpc": true})).unwrap();
                }
                let converted = s.active().unwrap().doc.clone();
                let t = text(&converted, id);
                let moved = t.color.c[..3].iter().zip(&original.c[..3]).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
                assert!(moved > 10.0 / 255.0, "{case}: the conversion changed the colour ({original:?} -> {:?})", t.color);
                let (at, expected) = ink(t);
                let (reopened, rid) = reopen_and_edit(&converted);
                let back = text(&reopened.active().unwrap().doc, rid);
                close(&back.color.c[..3], &t.color.c[..3], &format!("{case}: reopened colour"));
                close(&ink_at(back, at), &expected, &format!("{case}: reopened and edited pixels"));
            }
        }
    }
}

/// An unconverted control: the same steps without a profile change keep the colour too.
#[test]
fn unconverted_type_round_trips_unchanged() {
    let mut s = Session::new();
    let id = source(&mut s, 8, false);
    let doc = s.active().unwrap().doc.clone();
    let (at, expected) = ink(text(&doc, id));
    let (reopened, rid) = reopen_and_edit(&doc);
    close(&ink_at(text(&reopened.active().unwrap().doc, rid), at), &expected, "control");
}
