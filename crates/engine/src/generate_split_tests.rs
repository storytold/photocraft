//! `generate.splitLayers` (SAM 3.1 subjects over a repainted background) against the fake
//! worker, whose detector returns the rectangles it is given and whose fill paints its colour.

use photocraft_doc::LayerId;
use photocraft_genai::fake::FakeNative;
use photocraft_geom::Rect;
use serde_json::json;

use super::*;

/// A 64×48 document with one empty layer, talking to `url`.
fn session(url: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit_prefs(|p| {
        p.integrations.models_path = url.to_string();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    s
}

fn alpha_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(id).unwrap().surface().unwrap().read_region(Rect::new(x, y, x + 1, y + 1))[3]
}

#[test]
fn split_by_subjects_cuts_each_instance_out_over_a_repainted_background() {
    // The fake detector finds two subjects; the fill paints its colour.
    let fake = FakeNative::start_with(photocraft_genai::fake::Options {
        segments: vec![[0.0, 0.0, 0.5, 0.5], [0.5, 0.5, 1.0, 1.0]],
        color: [10, 200, 10, 255],
        ..Default::default()
    })
    .unwrap();
    let mut s = session(&fake.url);
    s.execute("edit.fill", json!({"use": "color", "color": [200, 40, 40]})).ok();
    let (before, steps) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.history.past_len())
    };
    let r = s.execute(SPLIT, json!({"seed": 3})).unwrap();
    assert_eq!(r["count"], 3, "the background and the two subjects: {r}");
    // ViTMatte (the fake's answer is opaque): each subject is its mask grown by the band.
    assert_eq!(r["edges"], "vitmatte", "{r}");
    let ids: Vec<LayerId> = r["layers"].as_array().unwrap().iter().map(|v| LayerId(v.as_u64().unwrap())).collect();
    let d = s.active().unwrap();
    assert_eq!(d.doc.layers.len(), before + 3, "the fill's own layer is folded into the background");
    assert_eq!(d.history.past_len(), steps + 1, "one undo step");
    assert_eq!(d.doc.layer(ids[0]).unwrap().name, "Split Layer 1/3");
    // The background is whole and opaque; each subject is opaque on its instance only.
    assert_eq!(alpha_at(&s, ids[0], 5, 5), 1.0);
    assert_eq!(alpha_at(&s, ids[0], 60, 45), 1.0);
    assert!(alpha_at(&s, ids[1], 5, 5) > 0.5 || alpha_at(&s, ids[2], 5, 5) > 0.5, "a subject holds the top-left quarter");
    assert_eq!(alpha_at(&s, ids[1], 60, 5), 0.0, "nothing outside the instances");
    assert_eq!(alpha_at(&s, ids[2], 60, 5), 0.0);
    let st = fake.state();
    assert!(st.runs.iter().any(|r| r.request["task"] == "segment"), "SAM asked");
    assert!(st.runs.iter().any(|r| r.request["task"] != "segment" && r.request["task"] != "layers"), "the fill ran");
    assert!(st.runs.iter().all(|r| r.request["task"] != "layers"), "not the layered model");
    // The removal names what it removes (an edit model told only "the object" changed nothing).
    assert!(
        st.runs.iter().any(|r| r.request["task"] != "segment" && r.request["prompt"].as_str().is_some_and(|p| p.contains("the person"))),
        "{:?}",
        st.runs.iter().map(|r| r.request["prompt"].clone()).collect::<Vec<_>>()
    );
    drop(st);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), before);
    // `layers` caps the subjects: the largest kept.
    let r = s.execute(SPLIT, json!({"layers": 2})).unwrap();
    assert_eq!(r["count"], 2, "{r}");
}

#[test]
fn split_validates_its_parameters_and_has_no_similar() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    for p in [json!({"layers": 1}), json!({"layers": 9}), json!({"layers": 2.5}), json!({"seed": "x"})] {
        assert!(matches!(s.execute(SPLIT, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    let r = s.execute(SPLIT, json!({"layers": 2, "prompt": "the lighthouse", "name": "Scene"})).unwrap();
    assert_eq!(r["foundBy"], "the lighthouse", "the prompt is what to cut out: {r}");
    let id = LayerId(r["layers"][0].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().name, "Scene 1/2");
    let st = fake.state();
    assert!(st.runs.iter().any(|r| r.request["task"] == "segment" && r.request["prompt"] == "the lighthouse"));
    assert!(
        st.runs.iter().any(|r| r.request["prompt"].as_str().is_some_and(|p| p.contains("the lighthouse") && r.request["task"] != "segment")),
        "the fill removes the lighthouse by name"
    );
    drop(st);
    let e = s.execute(SIMILAR, json!({"layer": id.0})).unwrap_err().to_string();
    assert!(e.contains("split"), "{e}");
    let mut empty = Session::new();
    empty.edit_prefs(|p| {
        p.integrations.models_path = fake.url.clone();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    assert!(!empty.is_enabled(SPLIT));
}

#[test]
fn split_takes_hair_from_the_matte_around_each_subject() {
    // The detector finds the top-left quarter; the matte keeps a little more (x < 38, y < 28):
    // the "hair" just past the mask is kept, nothing far from it.
    let fake = FakeNative::start_with(photocraft_genai::fake::Options {
        segments: vec![[0.0, 0.0, 0.5, 0.5]],
        matte: Some([0.0, 0.0, 0.6, 0.6]),
        ..Default::default()
    })
    .unwrap();
    let mut s = session(&fake.url);
    s.execute("edit.fill", json!({"use": "color", "color": [200, 40, 40]})).ok();
    let r = s.execute(SPLIT, json!({"layers": 2, "seed": 1, "edges": "matte"})).unwrap();
    assert_eq!(r["edges"], "matte", "{r}");
    let subject = LayerId(r["layers"][1].as_u64().unwrap());
    assert_eq!(alpha_at(&s, subject, 5, 5), 1.0, "inside the mask");
    assert!(alpha_at(&s, subject, 34, 5) > 0.9, "the matte's fringe past the mask");
    assert_eq!(alpha_at(&s, subject, 60, 40), 0.0, "nothing far away");
    let st = fake.state();
    assert!(st.runs.iter().any(|r| r.request["model"] == DEFAULT_MATTE_TEMPLATE || r.request["template"] == DEFAULT_MATTE_TEMPLATE), "the matte ran");
    drop(st);
    // The other edge sources, by name.
    let r = s.execute(SPLIT, json!({"layers": 2, "edges": "refine"})).unwrap();
    assert_eq!(r["edges"], "refine", "{r}");
    let r = s.execute(SPLIT, json!({"layers": 2, "edges": "vitmatte"})).unwrap();
    assert_eq!(r["edges"], "vitmatte", "{r}");
    let subject = LayerId(r["layers"][1].as_u64().unwrap());
    assert_eq!(alpha_at(&s, subject, 5, 5), 1.0, "solid inside the mask");
    assert_eq!(alpha_at(&s, subject, 60, 40), 0.0, "nothing past the band");
    let st = fake.state();
    let run = st.runs.iter().rev().find(|r| r.request["model"] == "vitmatte/refine").expect("ViTMatte asked");
    assert!(run.request["mask"].is_object(), "with a trimap: {}", run.request);
    drop(st);
    for bad in ["soft", "hair", "bogus"] {
        assert!(matches!(s.execute(SPLIT, json!({"edges": bad})), Err(EngineError::BadParams { .. })), "{bad}");
    }
}

#[test]
fn split_fails_with_nothing_changed_when_every_fill_paints_the_subject_back() {
    // The fake's fill hands the picture back (echo), so the detector still finds the subject
    // in the hole after each try.
    let fake = FakeNative::start_with(photocraft_genai::fake::Options { echo: true, ..Default::default() }).unwrap();
    let mut s = session(&fake.url);
    s.execute("edit.fill", json!({"use": "color", "color": [30, 90, 160]})).ok();
    let (before, steps) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.history.past_len())
    };
    let e = s.execute(SPLIT, json!({"seed": 3})).unwrap_err().to_string();
    assert!(e.contains("could not clear"), "{e}");
    let d = s.active().unwrap();
    assert_eq!((d.doc.layers.len(), d.history.past_len()), (before, steps), "nothing changed");
    let fills = fake.state().runs.iter().filter(|r| r.request["task"] != "segment" && r.request["task"] != "matte").count();
    assert!(fills >= 4, "four fills tried: {fills}");
    // After the removal, the backdrop fill: the hole's box flattened, the subject never named.
    let st = fake.state();
    let backdrop: Vec<_> = st.runs.iter().filter(|r| r.request["prompt"].as_str().is_some_and(|p| p.contains("with nobody there"))).collect();
    assert_eq!(backdrop.len(), 3, "three backdrop fills after the removal");
    assert!(backdrop.iter().all(|r| !r.request["prompt"].as_str().unwrap_or("").contains("person")), "the subject is not named");
}
