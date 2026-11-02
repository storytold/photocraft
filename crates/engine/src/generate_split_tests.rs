//! `generate.splitLayers` (a picture decomposed into RGBA layers) against the fake server, which
//! answers a layers request with one image per layer, opaque in its own horizontal band.

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
fn split_lays_the_models_layers_above_the_active_one_background_first() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let (before, active, steps) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.active_layer.unwrap(), d.history.past_len())
    };
    let r = s.execute(SPLIT, json!({"layers": 3, "seed": 5, "tries": 1})).unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(r["template"], DEFAULT_LAYERS_TEMPLATE);
    assert_eq!((r["requestWidth"].as_u64(), r["requestHeight"].as_u64()), (Some(64), Some(48)), "small pictures go as they are");
    let ids: Vec<LayerId> = r["layers"].as_array().unwrap().iter().map(|v| LayerId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 3);
    let d = s.active().unwrap();
    assert_eq!(d.doc.layers.len(), before + 3);
    assert_eq!(d.history.past_len(), steps + 1, "one undo step");
    let pos = |l: LayerId| d.doc.layers.iter().position(|x| x.id == l).unwrap();
    assert_eq!(pos(ids[0]), pos(active) + 1, "the first (background) layer right above the active one");
    assert_eq!(pos(ids[2]), pos(ids[1]) + 1, "then each next one above the last");
    assert_eq!(d.active_layer, Some(ids[2]), "the top one is active");
    assert_eq!(d.doc.layer(ids[0]).unwrap().name, "Split Layer 1/3");
    assert_eq!(d.doc.layer(ids[2]).unwrap().name, "Split Layer 3/3");
    // The fake's bands: layer i is opaque in the i-th third of the height.
    assert_eq!(alpha_at(&s, ids[0], 10, 5), 1.0);
    assert_eq!(alpha_at(&s, ids[0], 10, 40), 0.0, "transparent where another layer's content is");
    assert_eq!(alpha_at(&s, ids[2], 10, 40), 1.0);
    assert_eq!(alpha_at(&s, ids[2], 10, 5), 0.0);
    let info = generative_info(d.doc.layer(ids[1]).unwrap()).unwrap();
    assert_eq!(info.command, SPLIT);
    assert_eq!(info.prompt, SPLIT_DEFAULT_PROMPT, "no description: the generic instruction");
    assert_eq!(info.seed, 5);
    // What the server got.
    let st = fake.state();
    let g = &st.runs[0].request;
    assert_eq!(g["params"]["layers"], 3);
    assert_eq!((g["width"].as_u64(), g["height"].as_u64()), (Some(64), Some(48)));
    assert_eq!(g["task"], "layers");
    assert_eq!(g["prompt"], SPLIT_DEFAULT_PROMPT);
    assert_eq!(g["steps"], 20);
    assert_eq!(g["guidance"], 2.5);
    assert_eq!(g["params"]["models"]["unet"], "qwen_image_layered_fp8mixed.safetensors");
    assert_eq!(st.pictures.len(), 1);
    drop(st);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), before);
}

#[test]
fn split_sizes_validates_and_has_no_similar() {
    assert_eq!(layered_request_size(1024, 1024, None), (640, 640));
    assert_eq!(layered_request_size(1920, 1080, None), (640, 352), "aspect kept, multiples of 16");
    assert_eq!(layered_request_size(300, 200, None), (288, 192), "small pictures only snap to the grid");
    assert_eq!(layered_request_size(832, 1109, Some(1_000_000)), (864, 1152), "1 MP at 3:4");
    assert_eq!(layered_request_size(3000, 4000, Some(1_000_000)), (864, 1152), "down to 1 MP");
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    for p in [json!({"layers": 0}), json!({"layers": 9}), json!({"layers": 2.5}), json!({"sampleAllLayers": "no"}), json!({"template": "qwen-edit-2511/fill"})]
    {
        assert!(matches!(s.execute(SPLIT, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    let r = s.execute(SPLIT, json!({"layers": 2, "prompt": "a lighthouse on rocks", "name": "Scene"})).unwrap();
    let id = LayerId(r["layers"][0].as_u64().unwrap());
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().name, "Scene 1/2");
    assert_eq!(fake.state().runs[0].request["prompt"], "a lighthouse on rocks");
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
fn fast_draft_sets_the_splits_step_cache() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute(SPLIT, json!({"layers": 2, "seed": 5, "tries": 1})).unwrap();
    s.edit_prefs(|p| p.integrations.fast_draft = true);
    s.execute(SPLIT, json!({"layers": 2, "seed": 5, "tries": 1})).unwrap();
    let st = fake.state();
    let cache = |i: usize| st.runs[i].run.params.get("step_cache").cloned();
    assert_eq!(cache(0), None, "off by default");
    assert_eq!(cache(1), Some(json!(FAST_DRAFT_SPLIT_STEP_CACHE)));
}

#[test]
fn split_tries_runs_that_many_seeds_and_keeps_the_best_separated() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(SPLIT, json!({"layers": 3, "seed": 10, "tries": 3})).unwrap();
    assert_eq!(r["separation"].as_array().map(Vec::len), Some(3), "{r}");
    let st = fake.state();
    assert_eq!(st.runs.len(), 3, "one run per try");
    let seeds: Vec<u64> = st.runs.iter().filter_map(|run| run.request["seed"].as_u64()).collect();
    assert_eq!(seeds, vec![10, 11, 12], "consecutive seeds");
    assert!(r["seed"].as_u64().is_some_and(|seed| (10..13).contains(&seed)), "{r}");
    assert_eq!(r["count"], 3, "one split's layers placed, not every try's");
    drop(st);
    // One try is the default (the owner's choice); more than four, or a fraction, is refused.
    let one = s.execute(SPLIT, json!({"seed": 1})).unwrap();
    assert_eq!(one["separation"].as_array().map(Vec::len), Some(1));
    assert!(s.execute(SPLIT, json!({"tries": 5})).is_err());
    assert!(s.execute(SPLIT, json!({"tries": 1.5})).is_err());
}
