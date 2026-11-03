//! Generative layer metadata (`generate.info`) and `generate.similar`, against the fake server.

use photocraft_doc::LayerId;
use photocraft_genai::fake::FakeNative;
use photocraft_geom::Rect;
use serde_json::json;

use super::*;

/// A 64×48 document with a textured layer and a 20×16 selection at (10, 8), talking to `url`.
fn session(url: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("texture", |doc, active| {
        let r = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let n = surf.channels();
        let mut px = Vec::new();
        for y in 0..r.height() {
            for x in 0..r.width() {
                let v = ((x * 7 + y * 13) % 31) as f32 / 30.0;
                let mut tmp = [0.0f32; 8];
                let used = photocraft_raster::from_rgba_into(&surf.format(), [v, 1.0 - v, 0.5, 1.0], &mut tmp).min(n);
                px.extend_from_slice(&tmp[..used]);
            }
        }
        surf.write_region(r, &px);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 20, "height": 16})).unwrap();
    s.edit_prefs(|p| {
        {
            p.integrations.models_path = url.to_string();
            p.integrations.runtime_path = "fake".into();
            p.integrations.allow_research_models = false;
        };
        p.integrations.quality = "balanced".into();
    });
    s
}

fn mask_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(id).unwrap().mask.as_ref().unwrap().surface.read_region(Rect::new(x, y, x + 1, y + 1))[0]
}

#[test]
fn fill_layers_remember_their_run_and_generate_similar_re_rolls_them_in_place() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    assert!(!s.is_enabled(SIMILAR), "a textured layer was not generated");
    let r = s.execute(FILL, json!({"prompt": "a kite", "seed": 5})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let info = s.execute(INFO, json!({})).unwrap();
    assert_eq!(info["layer"].as_u64(), Some(id.0));
    let g = &info["generative"];
    assert_eq!(g["command"], FILL);
    assert_eq!(g["prompt"], "a kite", "the prompt as typed, not the wrapper");
    assert_eq!(g["template"], AUTO_FILL_ORDER[0], "auto resolved");
    assert_eq!(g["seed"], 5);
    assert_eq!(g["edge"], "soft");
    assert_eq!((g["rect"][2].as_u64(), g["rect"][3].as_u64()), (r["width"].as_u64(), r["height"].as_u64()));
    // The record is a plain PSD block, so it survives duplication.
    let d = s.active().unwrap();
    let dup = d.doc.layer(id).unwrap().duplicate();
    assert_eq!(generative_info(&dup).map(|i| i.prompt), Some("a kite".to_string()));
    assert!(s.is_enabled(SIMILAR));

    // Similar: no selection needed; the layer's mask is the area, the result sits above it.
    s.execute("select.deselect", json!({})).unwrap();
    let steps = s.active().unwrap().history.past_len();
    let r2 = s.execute(SIMILAR, json!({"seed": 6})).unwrap();
    let id2 = LayerId(r2["layer"].as_u64().unwrap());
    assert_ne!(id2, id);
    assert_eq!(r2["template"], AUTO_FILL_ORDER[0]);
    assert_eq!(r2["seed"], 6);
    assert_eq!((r2["width"].as_u64(), r2["height"].as_u64()), (r["width"].as_u64(), r["height"].as_u64()), "the same area");
    let d = s.active().unwrap();
    assert_eq!(d.history.past_len(), steps + 1, "one undo step");
    let pos = |l: LayerId| d.doc.layers.iter().position(|x| x.id == l).unwrap();
    assert_eq!(pos(id2), pos(id) + 1, "above the layer it is similar to");
    assert_eq!(d.doc.layer(id2).unwrap().name, "Generative Fill: a kite");
    assert_eq!(mask_at(&s, id2, 20, 16), 1.0, "inside the original selection");
    assert_eq!(mask_at(&s, id2, 62, 46), 0.0, "far outside");
    let g2 = s.execute(INFO, json!({"layer": id2.0})).unwrap();
    assert_eq!(g2["generative"]["seed"], 6);
    assert_eq!(g2["generative"]["prompt"], "a kite");
    let st = fake.state();
    assert_eq!(st.runs.len(), 2);
    assert_eq!(st.runs[1].request["seed"], 6);
    assert_eq!(st.runs[1].request["prompt"], st.runs[0].request["prompt"], "the same wrapped prompt");
    drop(st);
    // Not a generative layer: a clear error. A bad seed: a parameter error.
    let bg = s.active().unwrap().doc.layers[0].id;
    let e = s.execute(SIMILAR, json!({"layer": bg.0})).unwrap_err().to_string();
    assert!(e.contains("not made by a generative command"), "{e}");
    assert!(matches!(s.execute(SIMILAR, json!({"seed": -1})), Err(EngineError::BadParams { .. })));
}

#[test]
fn generated_images_and_expanded_areas_have_their_own_similar() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute(IMAGE, json!({"prompt": "a lighthouse", "seed": 2})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let g = s.execute(INFO, json!({})).unwrap();
    assert_eq!(g["generative"]["command"], IMAGE);
    assert_eq!((g["generative"]["width"].as_u64(), g["generative"]["height"].as_u64()), (Some(64), Some(64)));
    assert_eq!(g["generative"]["transparent"], false);
    let r2 = s.execute(SIMILAR, json!({"seed": 3})).unwrap();
    let id2 = LayerId(r2["layer"].as_u64().unwrap());
    let d = s.active().unwrap();
    let pos = |l: LayerId| d.doc.layers.iter().position(|x| x.id == l).unwrap();
    assert_eq!(pos(id2), pos(id) + 1);
    assert_eq!(d.doc.layer(id2).unwrap().name, "Generated: a lighthouse");
    let st = fake.state();
    assert_eq!(st.runs[1].request["prompt"], "a lighthouse");
    assert_eq!(st.runs[1].request["seed"], 3);
    drop(st);

    // An expand: its canvas is already there, so "similar" fills the added area again with the
    // expand's prompt through a fill template (no ImageCrop, a noise mask).
    let r = s.execute(EXPAND, json!({"right": 32, "seed": 9})).unwrap();
    let eid = LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.execute(INFO, json!({})).unwrap()["generative"]["command"], EXPAND);
    let r3 = s.execute(SIMILAR, json!({"layer": eid.0, "seed": 10})).unwrap();
    assert_eq!(r3["template"], AUTO_FILL_ORDER[0]);
    let st = fake.state();
    let g = &st.runs.last().unwrap().request;
    assert!(g["task"] == "fill" && !g["mask"].is_null(), "a fill request");
    assert!(g["prompt"].as_str().unwrap().contains("continuing to the right, empty, with no people"), "{}", g["prompt"]);
    drop(st);
    let d = s.active().unwrap();
    let nid = LayerId(r3["layer"].as_u64().unwrap());
    assert_eq!(mask_at(&s, nid, 80, 10), 1.0, "the added strip is the area");
    assert_eq!(mask_at(&s, nid, 10, 10), 0.0);
    let _ = d;
}
