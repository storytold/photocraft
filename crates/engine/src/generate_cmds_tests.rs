use std::time::{Duration, Instant};

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LayerId, Size};
use photocraft_genai::fake::{FakeNative, Options};
use photocraft_geom::Rect;
use serde_json::json;

use super::*;
use crate::jobs::{JobOutcome, Started};

const FAKE_COLOR: [f32; 4] = [200.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0];

/// A 64×48 document with a textured layer and a 20×16 selection at (10, 8), talking to `url`.
fn session(url: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    texture(&mut s);
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 20, "height": 16})).unwrap();
    s.edit_prefs(|p| {
        {
            p.integrations.models_path = url.to_string();
            p.integrations.runtime_path = "fake".into();
            p.integrations.allow_research_models = false;
        };
        // These tests pin the Lightning `auto` tier: Quality `balanced` (the default is `best`).
        p.integrations.quality = "balanced".into();
    });
    s
}

fn texture(s: &mut Session) {
    s.edit("texture", |doc, active| {
        let r = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let n = surf.channels();
        let mut px = Vec::new();
        for y in 0..r.height() {
            for x in 0..r.width() {
                let v = ((x * 7 + y * 13) % 31) as f32 / 30.0;
                let rgba = [v, 1.0 - v, 0.5, 1.0];
                let mut tmp = [0.0f32; 8];
                let used = photocraft_raster::from_rgba_into(&surf.format(), rgba, &mut tmp).min(n);
                px.extend_from_slice(&tmp[..used]);
            }
        }
        surf.write_region(r, &px);
        Ok(())
    })
    .unwrap();
}

fn composite(s: &Session) -> photocraft_compose::Buffer {
    let d = s.active().unwrap();
    photocraft_compose::render(&d.doc, d.doc.bounds())
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 2.5 / 255.0)
}

fn wait(s: &mut Session, id: crate::jobs::JobId) -> crate::jobs::JobEvent {
    let t = Instant::now();
    loop {
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            return e;
        }
        assert!(t.elapsed() < Duration::from_secs(30), "job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn fill_adds_a_masked_layer_above_the_active_one() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let before = composite(&s);
    let (layers, steps, journal) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.history.past_len(), s.journal.len())
    };
    // A hard edge: the layer mask is the selection, exactly (the default soft edge is tested below).
    let r = s.execute(FILL, json!({"prompt": "a red bicycle", "seed": 5, "edge": "hard"})).unwrap();
    assert_eq!(r["seed"], 5);
    // The preference is `auto`, and the fake "has" the Lightning LoRA: the 8-step tier runs.
    assert_eq!(r["template"], AUTO_FILL_ORDER[0]);
    // Selection 20×16 at (10, 8), margin 25 % of 20 = 5 → a 30×26 rect at (5, 3), grown to the
    // 16-px grid: 32×32.
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(32), Some(32)));
    let id = LayerId(r["layer"].as_u64().unwrap());

    let d = s.active().unwrap();
    assert_eq!(d.doc.layers.len(), layers + 1);
    assert_eq!(d.active_layer, Some(id), "the new layer is active");
    assert_eq!(d.history.past_len(), steps + 1, "one undo step");
    assert_eq!(s.journal.len(), journal + 1);
    let l = d.doc.layer(id).unwrap();
    assert_eq!(l.name, "Generative Fill: a red bicycle");
    let surf = l.surface().unwrap();
    assert_eq!(surf.content_bounds(), Rect::new(5, 3, 37, 35), "the result covers the request rectangle (grown to the 16-px grid)");
    let px = surf.read_region(Rect::new(15, 12, 16, 13));
    assert!(close([px[0], px[1], px[2], px[3]], FAKE_COLOR), "{px:?}");
    let mask = l.mask.as_ref().expect("a layer mask cut from the selection");
    assert!(mask.enabled);
    assert_eq!(mask.surface.read_region(Rect::new(15, 12, 16, 13)), vec![1.0], "inside the selection");
    assert_eq!(mask.surface.read_region(Rect::new(6, 4, 7, 5)), vec![0.0], "in the margin, outside the selection");
    assert_eq!(mask.surface.read_region(Rect::new(60, 40, 61, 41)), vec![0.0], "outside the request");

    // The composite changes only inside the selection.
    let after = composite(&s);
    assert!(close(after.get(15, 12), FAKE_COLOR), "{:?}", after.get(15, 12));
    assert_eq!(after.get(6, 4), before.get(6, 4), "margin pixels are untouched");
    assert_eq!(after.get(2, 2), before.get(2, 2));
    assert_eq!(after.get(50, 40), before.get(50, 40));

    // What the server received: the composite crop and a mask that is white only in the selection.
    let st = fake.state();
    assert_eq!(st.pictures.len(), 2);
    let sent = st.pictures[0].clone();
    // The 30×26 rect (selection + 5 px margin) is grown to the 16-px grid, 32×32 from (5, 3),
    // and a request that small goes out upscaled to 512 px (the model samples at that size).
    assert_eq!((sent.width, sent.height), (512, 512));
    assert_eq!((r["requestWidth"].as_u64(), r["requestHeight"].as_u64()), (Some(512), Some(512)));
    let mask_png = st.pictures[1].clone();
    // Request pixel (x, y) is document (5 + x / 16, 3 + y / 16): (88, 88) is inside the selection.
    assert_eq!(mask_png.get(88, 88).map(|p| p[0]), Some(255), "document (10,8) is selected");
    assert!(mask_png.get(8, 8).map(|p| p[0]) < Some(128), "document (5,3) is margin: at most the outward feather's tail");
    let prompt = st.runs[0].request["prompt"].as_str().unwrap_or_default().to_string();
    assert!(prompt.starts_with("Add a red bicycle to this image"), "the description is wrapped as an edit instruction: {prompt}");
    drop(st);

    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
    assert_eq!(composite(&s).px, before.px);
}

#[test]
fn fill_runs_as_a_background_job_with_the_same_result() {
    let fake = FakeNative::start().unwrap();
    let mut inline = session(&fake.url);
    inline.execute(FILL, json!({"prompt": "moss", "seed": 9})).unwrap();

    let mut s = session(&fake.url);
    let layers = s.active().unwrap().doc.layers.len();
    let id = match s.start(FILL, json!({"prompt": "moss", "seed": 9})).unwrap() {
        Started::Job(id) => id,
        Started::Done(v) => panic!("ran inline: {v}"),
    };
    assert_eq!(s.active().unwrap().doc.layers.len(), layers, "nothing lands before the job is applied");
    assert!(s.job(id).is_some());
    let e = wait(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert_eq!(v["seed"], 9);
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 1);
    assert_eq!(composite(&s).px, composite(&inline).px);
    let info = s.jobs_with_recent().into_iter().find(|j| j.id == id).unwrap();
    assert_eq!(info.state, "done");
    assert_eq!(info.progress, 1.0);
}

#[test]
fn cancelling_a_fill_leaves_the_document_alone_and_interrupts_the_server() {
    let fake = FakeNative::start_with(Options { delay_ms: 20_000, progress_steps: 2, ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    let before = composite(&s);
    let layers = s.active().unwrap().doc.layers.len();
    let id = match s.start(FILL, json!({"prompt": "slow"})).unwrap() {
        Started::Job(id) => id,
        Started::Done(v) => panic!("ran inline: {v}"),
    };
    // Let it reach the waiting loop.
    let t = Instant::now();
    while fake.state().runs.is_empty() {
        assert!(t.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(10));
    }
    let t = Instant::now();
    assert!(s.cancel_job(id));
    s.join_cancelled_jobs();
    assert!(t.elapsed() < Duration::from_secs(5), "cancel took {:?}", t.elapsed());
    assert_eq!(s.active().unwrap().doc.layers.len(), layers);
    assert_eq!(composite(&s).px, before.px);
    assert_eq!(fake.state().cancellations, 1, "the server was told to stop");
    assert!(s.jobs().is_empty());
}

#[test]
fn a_server_failure_is_an_error_and_changes_nothing() {
    let fake = FakeNative::start_with(Options { fail: Some("CUDA out of memory".into()), ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    let before = composite(&s);
    let steps = s.active().unwrap().history.past_len();
    let e = s.execute(FILL, json!({"prompt": "x"})).unwrap_err();
    assert!(e.to_string().contains("CUDA out of memory"), "{e}");
    assert_eq!(s.active().unwrap().history.past_len(), steps);
    assert_eq!(composite(&s).px, before.px);
}

#[test]
fn an_unreachable_server_is_a_clear_error() {
    let mut s = session("http://127.0.0.1:1");
    let e = s.execute(FILL, json!({"prompt": "x"})).unwrap_err();
    assert!(e.to_string().contains("native test worker"), "{e}");
    let h = s.execute(HEALTH, json!({})).unwrap();
    assert_eq!(h["ok"], false);
    assert!(h["error"].as_str().is_some_and(|m| m.contains("127.0.0.1:1")));
    let m = s.execute(MODELS, json!({})).unwrap();
    assert!(m["templates"].as_array().is_some_and(|t| !t.is_empty()));
    assert!(m["templates"][0]["models"][0]["installed"].is_null(), "unknown while offline");
}

#[test]
fn health_and_models_report_the_server() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let h = s.execute(HEALTH, json!({})).unwrap();
    assert_eq!(h["ok"], true);
    assert_eq!(h["backend"], "native/fake");
    assert!(h["vramTotal"].is_null());
    let m = s.execute(MODELS, json!({})).unwrap();
    assert_eq!(m["server"]["ok"], true);
    let t = &m["templates"][0];
    assert_eq!(t["id"], DEFAULT_FILL_TEMPLATE);
    assert_eq!(t["license"], "permissive");
    assert_eq!(t["allowed"], true);
    assert_eq!(t["models"][0]["installed"], true);
    assert_eq!(s.journal.iter().filter(|(id, _)| id.starts_with("generate.")).count(), 0, "queries are not journaled");
}

#[test]
fn a_worker_that_lists_only_templates_marks_their_files_installed() {
    // gn-diffusion's index names the templates it serves, not files: a served template's
    // slots are installed and the others' are not, so the picker's badges follow the worker.
    let missing = "qwen_image_layered_fp8mixed.safetensors".to_string();
    let fake = FakeNative::start_with(Options { templates_only: true, missing_files: vec![missing.clone()], ..Default::default() }).unwrap();
    let mut s = session(&fake.url);
    let m = s.execute(MODELS, json!({})).unwrap();
    let templates = m["templates"].as_array().unwrap();
    let slot_installed = |t: &Value| t["models"].as_array().unwrap().iter().all(|s| s["installed"] == true);
    for t in templates {
        let lacks = t["models"].as_array().unwrap().iter().any(|s| s["file"] == missing.as_str());
        assert_eq!(slot_installed(t), !lacks, "{}", t["id"]);
    }
}

#[test]
fn parameters_are_validated_before_anything_runs() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    for p in [
        json!({}),
        json!({"prompt": ""}),
        json!({"prompt": "   "}),
        json!({"prompt": 5}),
        json!({"prompt": "x", "steps": -1}),
        json!({"prompt": "x", "steps": 1000}),
        json!({"prompt": "x", "margin": 2}),
        json!({"prompt": "x", "margin": -0.1}),
        json!({"prompt": "x", "variations": 0}),
        json!({"prompt": "x", "variations": 5}),
        json!({"prompt": "x", "variations": 1.5}),
        json!({"prompt": "x", "seed": -1}),
        json!({"prompt": "x", "seed": 1.5}),
        json!({"prompt": "x", "seed": "7"}),
        json!({"prompt": "x", "guidance": 99}),
        json!({"prompt": "x", "template": "nope/fill"}),
        json!({"prompt": "x", "model": "../evil.safetensors"}),
        json!({"prompt": "x", "name": "n".repeat(300)}),
    ] {
        let e = s.execute(FILL, p.clone()).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }), "{p}: {e}");
    }
    assert!(fake.state().runs.is_empty(), "no request reached the server");
}

#[test]
fn fill_needs_a_selection_and_a_document() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute("select.deselect", json!({})).unwrap();
    assert!(matches!(s.execute(FILL, json!({"prompt": "x"})), Err(EngineError::Disabled(..))));
    assert!(!s.is_enabled(FILL));
    let mut empty = Session::new();
    assert!(matches!(empty.execute(FILL, json!({"prompt": "x"})), Err(EngineError::Disabled(..))));
    assert!(empty.is_enabled(HEALTH));
}

#[test]
fn the_model_preference_and_param_override_the_template() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.edit_prefs(|p| p.integrations.default_edit_model = "qwen_image_edit_2511_bf16.safetensors".into());
    s.execute(FILL, json!({"prompt": "x", "seed": 1})).unwrap();
    s.execute(FILL, json!({"prompt": "x", "seed": 1, "model": "custom.safetensors", "steps": 4, "guidance": 1, "negative": "blurry", "name": "Bike"})).unwrap();
    let st = fake.state();
    assert_eq!(st.runs[0].request["params"]["models"]["unet"], "qwen_image_edit_2511_bf16.safetensors", "the preference");
    assert_eq!(st.runs[1].request["params"]["models"]["unet"], "custom.safetensors", "the param wins");
    assert_eq!(st.runs[1].request["steps"], 4);
    assert_eq!(st.runs[1].request["guidance"], 1.0);
    assert_eq!(st.runs[1].request["negative"], "blurry");
    drop(st);
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(d.active_layer.unwrap()).unwrap().name, "Bike");
}

#[test]
fn a_sixteen_bit_document_gets_a_sixteen_bit_layer() {
    let fake = FakeNative::start().unwrap();
    let mut s = Session::new();
    let doc = Document::new("deep", Size::new(32, 32), ColorMode::Rgb, SampleType::U16);
    s.add_document(doc, None);
    s.execute("layer.new.layer", json!({})).unwrap();
    texture(&mut s);
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 8})).unwrap();
    s.edit_prefs(|p| {
        {
            p.integrations.models_path = fake.url.clone();
            p.integrations.runtime_path = "fake".into();
            p.integrations.allow_research_models = false;
        };
        p.integrations.quality = "balanced".into();
    });
    let r = s.execute(FILL, json!({"prompt": "x", "margin": 0})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(16), Some(16)), "an 8×8 selection goes out on the 16-px grid");
    let d = s.active().unwrap();
    let l = d.doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(l.surface().unwrap().format().sample, SampleType::U16);
    let px = l.surface().unwrap().read_region(Rect::new(10, 10, 11, 11));
    assert!(close([px[0], px[1], px[2], px[3]], FAKE_COLOR), "{px:?}");
}

#[test]
fn results_of_another_size_are_resampled_to_the_request() {
    let big = Rgba8::solid(60, 52, [10, 20, 30, 255]).unwrap();
    let r = resize_rgba8(&big, 30, 26).unwrap();
    assert_eq!((r.width, r.height), (30, 26));
    assert_eq!(r.get(0, 0), Some([10, 20, 30, 255]));
    assert_eq!(r.get(29, 25), Some([10, 20, 30, 255]));
    // A two-tone image keeps its halves.
    let mut data = Vec::new();
    for y in 0..4u32 {
        for _ in 0..4u32 {
            data.extend_from_slice(if y < 2 { &[0, 0, 0, 255] } else { &[255, 255, 255, 255] });
        }
    }
    let two = Rgba8::new(4, 4, data).unwrap();
    let r = resize_rgba8(&two, 2, 2).unwrap();
    // Lanczos averages across the edge a little; the halves stay dark and light.
    assert!(r.get(0, 0).unwrap()[0] < 48 && r.get(0, 0).unwrap()[3] == 255, "{:?}", r.get(0, 0));
    assert!(r.get(1, 1).unwrap()[0] > 207, "{:?}", r.get(1, 1));
    assert!(resize_rgba8(&two, 0, 2).is_err());
}

#[test]
fn the_quality_preference_decides_what_auto_picks() {
    // The default is `best`: the full-step base templates, no 4-bit weights under `auto`.
    let fresh = Session::new();
    assert_eq!(quality(&fresh), "best");
    assert_eq!(auto_order(FILL, "best"), &[DEFAULT_FILL_TEMPLATE]);
    assert_eq!(auto_order(FILL, "balanced"), AUTO_FILL_ORDER);
    assert_eq!(auto_order(FILL, "fast")[0], "qwen-edit-2511/fill-lightning-4");
    assert_eq!(auto_order(EDIT, "best"), &[DEFAULT_EDIT_TEMPLATE]);
    assert_eq!(auto_order(EXPAND, "fast"), AUTO_EXPAND_ORDER, "no 4-step expand: fast is balanced");
    assert_eq!(likely_auto(FILL, "best"), DEFAULT_FILL_TEMPLATE);
    assert_eq!(likely_auto(FILL, "nonsense"), "qwen-edit-2511/fill-lightning-8", "unknown reads as balanced in the order");
    let fake = FakeNative::start_with(Options::default()).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 20, "height": 16})).unwrap();
    s.edit_prefs(|p| {
        p.integrations.models_path = fake.url.clone();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    let r = s.execute(FILL, json!({"prompt": "a kite", "seed": 1})).unwrap();
    assert_eq!(r["template"], DEFAULT_FILL_TEMPLATE, "best: the 40-step base template");
    assert_eq!(s.execute("generate.models", json!({})).unwrap()["quality"], "best");
    s.execute("prefs.set", json!({"path": "integrations.quality", "value": "balanced"})).unwrap();
    let r = s.execute(FILL, json!({"prompt": "a kite", "seed": 1})).unwrap();
    assert_eq!(r["template"], "qwen-edit-2511/fill-lightning-8");
    assert!(s.execute("prefs.set", json!({"path": "integrations.quality", "value": "ultra"})).is_err());
    let v = s.execute("generate.models", json!({})).unwrap();
    assert_eq!((v["quality"].as_str(), v["autoFill"].as_str()), (Some("balanced"), Some("qwen-edit-2511/fill-lightning-8")));
}

#[test]
fn variations_are_hidden_siblings_and_the_variation_command_switches_them() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let (layers, steps) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.history.past_len())
    };
    let r = s.execute(FILL, json!({"prompt": "a kite", "seed": 40, "variations": 3})).unwrap();
    let ids: Vec<LayerId> = r["layers"].as_array().unwrap().iter().map(|v| LayerId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(r["layer"], ids[0].0, "the visible result is the primary one");
    assert_eq!(r["seeds"], json!([40, 41, 42]), "consecutive seeds");
    assert_eq!(fake.state().runs.len(), 3, "one backend run per variation");
    {
        let d = s.active().unwrap();
        assert_eq!(d.doc.layers.len(), layers + 3);
        assert_eq!(d.history.past_len(), steps + 1, "one undo step");
        assert_eq!(d.active_layer, Some(ids[0]));
        let visible: Vec<bool> = ids.iter().map(|id| d.doc.layer(*id).unwrap().visible).collect();
        assert_eq!(visible, [true, false, false]);
        assert_eq!(d.doc.layer(ids[1]).unwrap().name, "Generative Fill: a kite (2/3)");
        assert!(ids.iter().all(|id| d.doc.layer(*id).unwrap().mask.is_some()), "every variation is masked to the selection");
    }
    // Switching shows exactly one of them and activates it, in one undo step.
    let layers_json: Vec<u64> = ids.iter().map(|l| l.0).collect();
    let v = s.execute(VARIATION, json!({"layers": layers_json, "index": 3})).unwrap();
    assert_eq!((v["shown"].as_u64(), v["count"].as_u64()), (Some(ids[2].0), Some(3)));
    {
        let d = s.active().unwrap();
        let visible: Vec<bool> = ids.iter().map(|id| d.doc.layer(*id).unwrap().visible).collect();
        assert_eq!(visible, [false, false, true]);
        assert_eq!(d.active_layer, Some(ids[2]));
        assert_eq!(d.history.past_len(), steps + 2);
    }
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(ids[0]).unwrap().visible);
    for p in [
        json!({}),
        json!({"layers": [], "index": 1}),
        json!({"layers": layers_json, "index": 4}),
        json!({"layers": layers_json, "index": 0}),
        json!({"layers": layers_json}),
    ] {
        assert!(matches!(s.execute(VARIATION, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    // (`Session::execute` explains a missing layer with the document's listing.)
    assert!(matches!(s.execute(VARIATION, json!({"layers": [999_999], "index": 1})), Err(EngineError::NoLayer(_) | EngineError::UnknownLayer { .. })));
    let twice = json!({"layers": [ids[0].0, ids[0].0], "index": 1});
    assert!(matches!(s.execute(VARIATION, twice), Err(EngineError::BadParams { .. })));
}

#[test]
fn dropping_and_keeping_variations_are_one_undo_step_each() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(FILL, json!({"prompt": "a kite", "seed": 7, "variations": 4})).unwrap();
    let ids: Vec<u64> = r["layers"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
    let (n, steps) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.history.past_len())
    };
    let visible = |s: &Session| -> Vec<bool> {
        let d = s.active().unwrap();
        ids.iter().filter_map(|id| d.doc.layer(LayerId(*id)).map(|l| l.visible)).collect()
    };
    // Dropping the shown first one shows the next and makes it active.
    let v = s.execute(DROP_VARIATION, json!({"layers": ids, "index": 1})).unwrap();
    assert_eq!((v["dropped"].as_u64(), v["shown"].as_u64()), (Some(ids[0]), Some(ids[1])));
    assert_eq!(v["layers"], json!([ids[1], ids[2], ids[3]]));
    {
        let d = s.active().unwrap();
        assert_eq!((d.doc.layers.len(), d.history.past_len(), d.active_layer), (n - 1, steps + 1, Some(LayerId(ids[1]))));
    }
    assert_eq!(visible(&s), [true, false, false]);
    // Dropping a hidden one leaves the shown one alone.
    let rest = [ids[1], ids[2], ids[3]];
    let v = s.execute(DROP_VARIATION, json!({"layers": rest, "index": 3})).unwrap();
    assert_eq!((v["dropped"].as_u64(), v["shown"].as_u64()), (Some(ids[3]), Some(ids[1])));
    assert!(s.undo(), "the drop comes back");
    assert_eq!(s.active().unwrap().doc.layers.len(), n - 1);
    // Keep the third: the others go, it is shown and active, in one step.
    let v = s.execute(KEEP_VARIATION, json!({"layers": rest, "index": 2})).unwrap();
    assert_eq!((v["kept"].as_u64(), v["removed"].clone()), (Some(ids[2]), json!([ids[1], ids[3]])));
    {
        let d = s.active().unwrap();
        assert_eq!(d.doc.layers.len(), n - 3);
        assert!(d.doc.layer(LayerId(ids[2])).unwrap().visible);
        assert_eq!(d.active_layer, Some(LayerId(ids[2])));
        assert_eq!(d.history.past_len(), steps + 2);
    }
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers.len(), n - 1, "keep undoes in one step");
    for p in [json!({"layers": rest}), json!({"layers": rest, "index": 4}), json!({"layers": [], "index": 1})] {
        assert!(matches!(s.execute(KEEP_VARIATION, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
        assert!(matches!(s.execute(DROP_VARIATION, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    // The only layer of a document cannot be dropped.
    let mut lone = Session::new();
    lone.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let only = lone.active().unwrap().doc.layers[0].id.0;
    assert!(lone.execute(DROP_VARIATION, json!({"layers": [only], "index": 1})).is_err());
    assert_eq!(lone.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn auto_takes_the_lightning_tier_when_its_lora_is_installed_and_the_base_otherwise() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    assert_eq!(s.prefs().integrations.default_fill_template, AUTO_TEMPLATE, "the shipped default");
    let r = s.execute(FILL, json!({"prompt": "x", "seed": 1, "template": "auto"})).unwrap();
    assert_eq!(r["template"], "qwen-edit-2511/fill-lightning-8");
    let st = fake.state();
    let g = &st.runs[0].request;
    assert_eq!(g["params"]["models"]["lora"], "Qwen-Image-Edit-2511-Lightning-8steps-V1.0-bf16.safetensors");
    assert_eq!(g["steps"], 8, "the tier's default steps");
    assert_eq!(g["guidance"], 1.0);
    drop(st);
    let m = s.execute(MODELS, json!({})).unwrap();
    assert_eq!(m["autoFill"], "qwen-edit-2511/fill-lightning-8");
    assert!(s.execute(MODELS, json!({"probe": false})).unwrap()["autoFill"].is_null(), "no server contact, no answer");

    // Without the LoRA on the server, auto falls back to the 40-step base template.
    let bare =
        FakeNative::start_with(Options { missing_files: vec!["Qwen-Image-Edit-2511-Lightning-8steps-V1.0-bf16.safetensors".into()], ..Options::default() })
            .unwrap();
    let mut s = session(&bare.url);
    let r = s.execute(FILL, json!({"prompt": "x", "seed": 1})).unwrap();
    assert_eq!(r["template"], DEFAULT_FILL_TEMPLATE);
    let st = bare.state();
    assert_eq!(st.runs[0].request["steps"], 40);
    drop(st);
    let m = s.execute(MODELS, json!({})).unwrap();
    assert_eq!(m["autoFill"], DEFAULT_FILL_TEMPLATE);
    let lightning = m["templates"].as_array().unwrap().iter().find(|t| t["id"] == "qwen-edit-2511/fill-lightning-8").unwrap();
    assert_eq!(lightning["models"][3]["installed"], false);
    assert_eq!(lightning["license"], "permissive");
}

#[test]
fn expand_grows_the_canvas_and_paints_the_added_area_in_one_step() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute("select.deselect", json!({})).unwrap();
    let before = composite(&s);
    let (layers, steps) = {
        let d = s.active().unwrap();
        (d.doc.layers.len(), d.history.past_len())
    };
    let r = s.execute(EXPAND, json!({"right": 32, "bottom": 16, "seed": 9})).unwrap();
    assert_eq!(r["canvas"], json!([96, 64]));
    assert_eq!(r["offset"], json!([0, 0]));
    assert_eq!(r["template"], AUTO_EXPAND_ORDER[0], "expand has its own Lightning tier");
    let d = s.active().unwrap();
    assert_eq!((d.doc.size.width, d.doc.size.height), (96, 64));
    assert_eq!(d.doc.layers.len(), layers + 1);
    assert_eq!(d.history.past_len(), steps + 1, "one undo step");
    let l = d.doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(l.name, "Generative Expand");
    let mask = &l.mask.as_ref().unwrap().surface;
    assert_eq!(mask.read_region(Rect::new(80, 10, 81, 11))[0], 1.0, "the added strip on the right is shown");
    assert_eq!(mask.read_region(Rect::new(30, 56, 31, 57))[0], 1.0, "so is the strip at the bottom");
    assert_eq!(mask.read_region(Rect::new(10, 10, 11, 11))[0], 0.0, "the old picture is not");
    let band = mask.read_region(Rect::new(62, 10, 63, 11))[0];
    assert!(band > 0.0 && band < 1.0, "the picture's edge band fades into the result: {band}");
    let after = composite(&s);
    assert_eq!(after.get(10, 10), before.get(10, 10), "old pixels untouched");
    assert!(close(after.get(80, 10), FAKE_COLOR) && close(after.get(30, 56), FAKE_COLOR), "the added area is painted");
    // What the server got: a request covering the whole new canvas (both strips meet at the
    // corner), nothing transparent in it, and the default prompt as an instruction.
    let st = fake.state();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(96), Some(64)));
    let g = &st.runs[0].request;
    let prompt = g["prompt"].as_str().unwrap_or_default().to_string();
    assert!(prompt.starts_with("extend the scene beyond its original edges"), "{prompt}");
    // The model's reference is the picture alone: the 64×48 picture at (0, 0) of the 96×64
    // request, in the request's upscaled pixels (512 wide → ×5.33).
    assert_eq!((g["params"]["ref_x"].as_u64(), g["params"]["ref_y"].as_u64()), (Some(0), Some(0)));
    let (cw, ch) = (g["params"]["ref_w"].as_u64().unwrap(), g["params"]["ref_h"].as_u64().unwrap());
    assert!((330..=352).contains(&cw) && (240..=264).contains(&ch), "{cw}×{ch}");
    let sent = st.pictures[0].clone();
    assert!(sent.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "no transparent pixels reach the model");
    // The added canvas reaches the model continuing the picture's edge pixels (the 96×64 rect
    // goes out at 512 px wide), not as a wall of grey.
    let y = sent.height / 4;
    let (far, edge) = (sent.get(sent.width - 2, y).unwrap(), sent.get(cw as u32 - 3, y).unwrap());
    assert!(
        far.iter().zip(edge.iter()).take(3).all(|(a, b)| (i16::from(*a) - i16::from(*b)).abs() <= 6),
        "the pre-fill continues the edge: {far:?} vs {edge:?}"
    );
    assert!(far.iter().take(3).any(|c| (i16::from(*c) - 128).abs() > 20), "and is not grey: {far:?}");
    drop(st);
    assert!(s.undo());
    let d = s.active().unwrap();
    assert_eq!((d.doc.size.width, d.doc.size.height), (64, 48));
    assert_eq!(d.doc.layers.len(), layers);
}

#[test]
fn expand_takes_a_target_size_with_an_anchor_and_validates() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(EXPAND, json!({"width": 96, "anchor": "right", "prompt": "more sea", "seed": 2})).unwrap();
    assert_eq!(r["canvas"], json!([96, 48]));
    assert_eq!(r["offset"], json!([32, 0]), "anchored right: the picture moved by the left pad");
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().name, "Generative Expand: more sea");
    assert_eq!(d.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(42, 8, 62, 24), "the selection moved with the picture");
    // Only the left strip plus a margin of the picture was requested: x 0..(32 + 24), aligned.
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(64), Some(48)));
    // (One lock on the fake's state at a time: a second `fake.state()` while a guard lives
    // would deadlock the test.)
    let st = fake.state();
    let g = &st.runs[0].request;
    let prompt = g["prompt"].as_str().unwrap_or_default().to_string();
    assert!(prompt.starts_with("Extend this image beyond its current edges with more sea"), "{prompt}");
    // The picture sits at x 32..96 of the 96×48 canvas; the request is x 0..64 (left pad + a
    // 24 px margin, aligned) at 512 px wide, so the reference crop starts at 32 × 8 = 256.
    assert_eq!(g["params"]["ref_x"], json!(256));
    assert_eq!(g["params"]["ref_w"], json!(256));
    drop(st);
    for p in [
        json!({}),
        json!({"left": 0, "right": 0}),
        json!({"width": 32}),
        json!({"left": -5}),
        json!({"right": 2.5}),
        json!({"width": 200, "anchor": "middle"}),
        json!({"right": 20000}),
    ] {
        assert!(matches!(s.execute(EXPAND, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    let mut empty = Session::new();
    assert!(!empty.is_enabled(EXPAND));
    assert!(matches!(empty.execute(EXPAND, json!({"right": 8})), Err(EngineError::Disabled(..))));
}

#[test]
fn purge_generative_models_asks_the_server_to_free() {
    let fake = FakeNative::start().unwrap();
    let mut s = Session::new();
    s.edit_prefs(|p| {
        {
            p.integrations.models_path = fake.url.clone();
            p.integrations.runtime_path = "fake".into();
            p.integrations.allow_research_models = false;
        };
        p.integrations.quality = "balanced".into();
    });
    assert!(s.is_enabled(FREE), "no document needed");
    let r = s.execute(FREE, json!({})).unwrap();
    assert_eq!(r["freed"], true);
    assert_eq!(fake.state().frees, 1);
    assert_eq!(s.journal.iter().filter(|(id, _)| id.as_str() == FREE).count(), 0, "not journaled");
    // No server configured: a clear error.
    s.edit_prefs(|p| {
        p.integrations.models_path = String::new();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    assert!(s.execute(FREE, json!({})).unwrap_err().to_string().contains("choose your Models folder"));
}

#[test]
fn request_rects_grow_to_the_sixteen_pixel_grid_inside_the_canvas() {
    let canvas = Rect::new(0, 0, 64, 48);
    assert_eq!(align_to_grid(Rect::new(5, 3, 35, 29), canvas), Rect::new(5, 3, 37, 35), "right and down first");
    assert_eq!(align_to_grid(Rect::new(50, 40, 64, 48), canvas), Rect::new(48, 32, 64, 48), "left and up at the canvas edge");
    assert_eq!(align_to_grid(Rect::new(0, 0, 64, 48), canvas), Rect::new(0, 0, 64, 48), "aligned already");
    assert_eq!(align_to_grid(Rect::new(2, 2, 8, 8), Rect::new(0, 0, 10, 10)), Rect::new(2, 2, 8, 8), "a canvas too small keeps the rect");
    assert_eq!(align_to_grid(Rect::new(0, 0, 40, 34), canvas), Rect::new(0, 0, 48, 48));
}

#[test]
fn large_areas_are_sent_at_one_megapixel_and_come_back_full_size() {
    assert_eq!(request_size(1024, 1024), (1024, 1024), "exactly the 1 MP cap: untouched");
    assert_eq!(request_size(1024, 768), (1024, 768), "under the cap: untouched");
    assert_eq!(request_size(1280, 1280), (1024, 1024));
    assert_eq!(request_size(4000, 100), (4000, 100), "under the cap, long enough: untouched");
    assert_eq!(request_size(2048, 1536), (1168, 880));
    assert_eq!(request_size(200, 150), (512, 384), "small requests go out at 512 px on the longer side");
    assert_eq!(request_size(100, 100), (512, 512));
    assert_eq!(request_size(512, 300), (512, 300), "exactly the floor: untouched");
    let (w, h) = request_size(8000, 8000);
    assert!(w % 16 == 0 && h % 16 == 0 && u64::from(w) * u64::from(h) <= 1024 * 1024, "{w}×{h}");
    let (w, h) = request_size(20_000, 64);
    assert!(h == 64 && w < 20_000 && w % 16 == 0, "the short side never drops below 64: {w}×{h}");

    let fake = FakeNative::start().unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1600, "height": 1200})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 100, "y": 100, "width": 1400, "height": 1000})).unwrap();
    s.edit_prefs(|p| {
        {
            p.integrations.models_path = fake.url.clone();
            p.integrations.runtime_path = "fake".into();
            p.integrations.allow_research_models = false;
        };
        p.integrations.quality = "balanced".into();
    });
    let r = s.execute(FILL, json!({"prompt": "x", "margin": 0})).unwrap();
    // 1400×1000 grown to the grid: 1408×1008 from (100, 100).
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1408), Some(1008)));
    let (rw, rh) = (r["requestWidth"].as_u64().unwrap(), r["requestHeight"].as_u64().unwrap());
    assert!(rw * rh <= 1024 * 1024 && rw % 16 == 0 && rh % 16 == 0, "{rw}×{rh}");
    let st = fake.state();
    let sent = st.pictures[0].clone();
    assert_eq!((u64::from(sent.width), u64::from(sent.height)), (rw, rh), "the upload is the downscaled request");
    let mask = st.pictures[1].clone();
    assert_eq!((mask.width, mask.height), (sent.width, sent.height));
    drop(st);
    let d = s.active().unwrap();
    let l = d.doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(l.surface().unwrap().content_bounds(), Rect::new(100, 100, 1508, 1108), "placed at full size");
    let px = l.surface().unwrap().read_region(Rect::new(800, 600, 801, 601));
    assert!(close([px[0], px[1], px[2], px[3]], FAKE_COLOR), "{px:?}");
    assert!(r["timings"][0]["encodeMs"].is_u64() && r["timings"][0]["runMs"].is_u64(), "{}", r["timings"]);
}

#[test]
fn the_request_mask_is_feathered_outward_and_the_layer_mask_is_not() {
    // A 20×20 square at (20, 20) on 64×64, feathered by 4.
    let mut data = vec![0u8; 64 * 64];
    for y in 20..40 {
        for x in 20..40 {
            data[y * 64 + x] = 255;
        }
    }
    let mask = Gray8::new(64, 64, data).unwrap();
    let soft = feather_outward(&mask, 4).unwrap();
    let at = |x: usize, y: usize| soft.data[y * 64 + x];
    assert_eq!(at(30, 30), 255, "the inside keeps full coverage");
    assert_eq!(at(20, 30), 255, "so does the edge pixel");
    assert!(at(18, 30) > 0 && at(18, 30) < 255, "a falloff just outside: {}", at(18, 30));
    assert!(at(18, 30) > at(14, 30), "monotonic falloff");
    assert_eq!(at(5, 5), 0, "far away stays empty");
    assert_eq!(feather_outward(&mask, 0).unwrap(), mask);
    assert_eq!((feather_radius(576, 512), feather_radius(100, 100), feather_radius(4000, 4000)), (23, 6, 48));

    // End to end: the uploaded mask is soft outside the selection; the layer mask is the same
    // soft band, dithered, or the selection exactly for a hard edge.
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let r = s.execute(FILL, json!({"prompt": "x", "margin": 0.5, "seed": 3})).unwrap();
    let st = fake.state();
    let sent = st.pictures[1].clone();
    // The selection is x 10..30, y 8..24 on the canvas; a 50 % margin of the longer side (10 px)
    // makes the request rect (0, 0)..(40, 34), grown to the grid as (0, 0)..(48, 48) and sent
    // upscaled to 512×512 (scale 10.67), so the selection sits at x 107..320, y 85..256 of it.
    // The feather is the 6 px minimum here, a 12 px tail: nearly gone at the margin's far edge.
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(48), Some(48)));
    assert_eq!((sent.width, sent.height), (512, 512));
    assert_eq!(sent.get(200, 170).map(|p| p[0]), Some(255), "the selection itself");
    let (near, far) = (sent.get(90, 170).unwrap()[0], sent.get(5, 170).unwrap()[0]);
    assert!(near > 128 && far < 40 && near > far, "a falloff from full at the selection's edge: {near} -> {far}");
    assert_eq!(sent.get(5, 500).map(|p| p[0]), Some(0), "the far corner is empty");
    drop(st);
    // The default soft edge: the layer mask fades (with grain) across the band outside the
    // selection, is full inside and empty far away; a hard edge is the selection exactly.
    let mask_of = |s: &Session, r: &Value| {
        let d = s.active().unwrap();
        d.doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().mask.as_ref().unwrap().surface.clone()
    };
    let m = mask_of(&s, &r);
    assert_eq!(m.read_region(Rect::new(20, 16, 21, 17))[0], 1.0, "inside the selection");
    assert_eq!(m.read_region(Rect::new(10, 16, 11, 17))[0], 1.0, "and on its edge pixel");
    let band = m.read_region(Rect::new(8, 16, 9, 17))[0];
    assert!(band > 0.3 && band < 1.0, "2 px outside the selection is high in the soft band: {band}");
    let outer = m.read_region(Rect::new(2, 16, 3, 17))[0];
    assert!(outer < band, "and the band falls off outward: {outer} < {band}");
    assert_eq!(m.read_region(Rect::new(0, 47, 1, 48))[0], 0.0, "the far corner is outside the band");
    // Grain: neighbouring band pixels are not all equal (a plain ramp would be).
    let row: Vec<f32> = (0..10).map(|dx| m.read_region(Rect::new(dx, 16, dx + 1, 17))[0]).collect();
    assert!(row.windows(2).any(|p| (p[0] - p[1]).abs() > 0.02), "{row:?}");
    let r2 = s.execute(FILL, json!({"prompt": "x", "margin": 0.5, "edge": "hard"})).unwrap();
    let m2 = mask_of(&s, &r2);
    assert_eq!(m2.read_region(Rect::new(8, 16, 9, 17))[0], 0.0, "a hard edge is the selection itself");
    assert_eq!(m2.read_region(Rect::new(9, 16, 10, 17))[0], 0.0);
    assert_eq!(m2.read_region(Rect::new(10, 16, 11, 17))[0], 1.0);
    assert!(matches!(s.execute(FILL, json!({"prompt": "x", "edge": "fuzzy"})), Err(EngineError::BadParams { .. })));
}

#[test]
fn the_dithered_edge_is_a_noisy_ramp_that_stays_clean_inside_and_outside() {
    let mask: Vec<u8> = (0..=255u8).collect();
    let a = dither_edge(&mask, 7);
    assert_eq!((a[0], a[255]), (0.0, 1.0), "fully in and out are untouched");
    assert!(a.iter().all(|v| (0.0..=1.0).contains(v)));
    // Mid-ramp the noise is strongest; the ramp still rises on average.
    let mid: Vec<f32> = a[100..156].to_vec();
    assert!(mid.windows(2).any(|p| p[1] < p[0]), "noise breaks the monotonic ramp: {mid:?}");
    let low: f32 = a[1..64].iter().sum::<f32>() / 63.0;
    let high: f32 = a[192..255].iter().sum::<f32>() / 63.0;
    assert!(low < 0.35 && high > 0.65, "{low} {high}");
    assert_eq!(dither_edge(&mask, 7), a, "deterministic for a seed");
    assert_ne!(dither_edge(&mask, 8), a, "another seed, another grain");
}

#[test]
fn resampling_is_lanczos_for_pixels_and_tent_for_masks() {
    // Flat stays flat both ways (normalised taps, no ringing on a constant).
    let flat = Rgba8::solid(37, 23, [10, 200, 30, 255]).unwrap();
    for (w, h) in [(74, 46), (19, 12), (100, 7)] {
        let r = resize_rgba8(&flat, w, h).unwrap();
        assert!(r.data.as_chunks::<4>().0.iter().all(|p| *p == [10, 200, 30, 255]), "{w}×{h}");
    }
    // A two-tone image keeps its halves after a 4× upscale; the edge is sharp within a few px.
    let mut data = Vec::new();
    for _y in 0..8u32 {
        for x in 0..8u32 {
            data.extend_from_slice(if x < 4 { &[0, 0, 0, 255] } else { &[255, 255, 255, 255] });
        }
    }
    let two = Rgba8::new(8, 8, data).unwrap();
    let up = resize_rgba8(&two, 32, 32).unwrap();
    assert_eq!(up.get(2, 16).map(|p| p[0]), Some(0));
    assert_eq!(up.get(29, 16).map(|p| p[0]), Some(255));
    assert!(up.get(13, 16).unwrap()[0] < 60 && up.get(18, 16).unwrap()[0] > 195, "{:?} {:?}", up.get(13, 16), up.get(18, 16));
    let down = resize_rgba8(&two, 4, 4).unwrap();
    assert!(down.get(0, 0).unwrap()[0] < 10 && down.get(3, 3).unwrap()[0] > 245);
    // Masks: a hard edge stays monotonic and in range.
    let mask = Gray8::new(8, 1, vec![0, 0, 0, 0, 255, 255, 255, 255]).unwrap();
    let m = resize_gray8(&mask, 16, 1).unwrap();
    assert!(m.data.windows(2).all(|w| w[0] <= w[1]), "{:?}", m.data);
    assert_eq!((m.data[0], m.data[15]), (0, 255));
    assert!(resize_rgba8(&two, 0, 2).is_err() && resize_gray8(&mask, 3, 0).is_err());
}

#[test]
fn the_default_fill_template_is_permissive() {
    let fake = FakeNative::start().unwrap();
    let s = session(&fake.url);
    assert!(!s.prefs().integrations.allow_research_models);
    let t = template::find(DEFAULT_FILL_TEMPLATE).unwrap();
    assert_eq!(t.meta.license, photocraft_genai::template::License::Permissive);
}

/// The job keeps a backend's latest picture over the place the work named, drops one with no
/// place, a wrong byte count or after a cancel, and clears it on request.
#[test]
fn job_progress_keeps_the_latest_preview_with_its_place() {
    let ctx = JobCtx::new();
    let p = JobProgress::span(&ctx, 0.1, 0.9);
    let picture =
        |level: u8, step: u32| photocraft_genai::Preview { image: Rgba8::solid(4, 2, [level, level, level, 255]).unwrap(), step, steps: 8, output: 0 };
    photocraft_genai::Progress::preview(&p, &picture(10, 1));
    assert_eq!(ctx.preview(), None, "no place named: nothing to show");
    ctx.preview_at(Rect::new(5, 3, 37, 35));
    photocraft_genai::Progress::preview(&p, &picture(20, 2));
    let first = ctx.preview().unwrap();
    assert_eq!((first.rect, first.width, first.height, first.step, first.steps, first.output), (Rect::new(5, 3, 37, 35), 4, 2, 2, 8, 0));
    assert_eq!(first.rgba[..4], [20, 20, 20, 255]);
    // A later picture replaces it, with a new version; a later place applies to later pictures.
    ctx.preview_at(Rect::new(0, 0, 64, 48));
    photocraft_genai::Progress::preview(&p, &photocraft_genai::Preview { output: 2, ..picture(30, 3) });
    let second = ctx.preview().unwrap();
    assert!(second.version > first.version);
    assert_eq!((second.rect, second.step, second.output), (Rect::new(0, 0, 64, 48), 3, 2));
    ctx.show_preview(4, 2, vec![0; 5], 4, 8, 0);
    assert_eq!(ctx.preview().map(|p| p.step), Some(3), "a picture of the wrong size is dropped");
    ctx.clear_preview();
    assert_eq!(ctx.preview(), None);
    ctx.cancel();
    photocraft_genai::Progress::preview(&p, &picture(40, 4));
    assert_eq!(ctx.preview(), None, "a cancelled job shows no more pictures");
}

/// One request making several tiles (Upscale's batch): output `i`'s pictures land on tile `i`,
/// each output keeps its latest, an output past the list is dropped; a single place takes every
/// output (a split's layers) and keeps only the latest.
#[test]
fn batched_previews_land_on_their_own_tiles() {
    let ctx = JobCtx::new();
    let tiles = vec![Rect::new(0, 0, 32, 32), Rect::new(32, 0, 64, 32), Rect::new(0, 32, 32, 64)];
    ctx.preview_places(tiles.clone());
    for (step, output) in [(1, 0), (1, 1), (1, 2), (2, 1), (1, 7)] {
        ctx.show_preview(2, 2, vec![9; 16], step, 4, output);
    }
    let shown = ctx.previews();
    assert_eq!(shown.iter().map(|p| (p.output, p.rect, p.step)).collect::<Vec<_>>(), [(0, tiles[0], 1), (1, tiles[1], 2), (2, tiles[2], 1)]);
    assert_eq!(ctx.preview().map(|p| p.output), Some(1), "the latest");
    ctx.preview_at(Rect::new(0, 0, 64, 64));
    assert!(ctx.previews().is_empty(), "a new place drops the old pictures");
    ctx.show_preview(2, 2, vec![9; 16], 1, 4, 0);
    ctx.show_preview(2, 2, vec![9; 16], 1, 4, 1);
    let shown = ctx.previews();
    assert_eq!(shown.iter().map(|p| (p.output, p.rect)).collect::<Vec<_>>(), [(1, Rect::new(0, 0, 64, 64))]);
}

/// A fill in the background shows the worker's pictures over its request area while it runs
/// (`Session::job_preview`), and the result replaces them when it lands.
#[test]
fn a_running_fill_shows_its_preview_over_its_area_until_the_result_lands() {
    let fake = FakeNative::start_with(Options { delay_ms: 1500, progress_steps: 6, ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    assert_eq!(s.prefs().integrations.preview_while_generating, "fast", "on by default");
    let doc = s.active().unwrap().doc.id;
    let id = match s.start(FILL, json!({"prompt": "moss", "seed": 3, "enhance": false})).unwrap() {
        Started::Job(id) => id,
        Started::Done(v) => panic!("ran inline: {v}"),
    };
    let t = Instant::now();
    let shown = loop {
        if let Some(p) = s.job_preview(doc) {
            break p;
        }
        assert!(t.elapsed() < Duration::from_secs(20), "no preview arrived");
        std::thread::sleep(Duration::from_millis(5));
    };
    // The request rectangle (see `fill_adds_a_masked_layer_above_the_active_one`), at the
    // worker's eighth of the 512-px request.
    assert_eq!(shown.rect, Rect::new(5, 3, 37, 35));
    assert_eq!((shown.width, shown.height, shown.steps), (64, 64, 6));
    assert_eq!(shown.rgba.len(), 64 * 64 * 4);
    assert_eq!(s.job_preview_of(id).map(|p| p.rect), Some(shown.rect));
    assert_eq!(fake.state().runs[0].run.preview.as_str(), "fast");
    let e = wait(&mut s, id);
    assert!(matches!(e.outcome, JobOutcome::Done(_)), "{e:?}");
    assert_eq!(s.job_preview(doc), None, "the result replaced the preview");
    assert_eq!(s.job_preview_of(id), None);
}

/// `integrations.previewWhileGenerating` reaches the worker at the next run; `off` asks for no
/// pictures and the job shows none.
#[test]
fn the_preview_preference_reaches_the_next_run() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.execute("prefs.set", json!({"path": "integrations.previewWhileGenerating", "value": "off"})).unwrap();
    s.execute(FILL, json!({"prompt": "moss", "seed": 3, "enhance": false})).unwrap();
    s.execute("prefs.set", json!({"path": "integrations.previewWhileGenerating", "value": "sharp"})).unwrap();
    s.execute(FILL, json!({"prompt": "moss", "seed": 3, "enhance": false})).unwrap();
    let st = fake.state();
    let asked: Vec<_> = st.runs.iter().map(|r| r.run.preview.as_str()).collect();
    assert_eq!(asked, ["off", "sharp"]);
    assert!(st.runs[0].request.get("preview").is_none(), "off is not sent: a worker from before previews reads the same request");
}

/// The preference: `fast` by default, one of off / fast / sharp, kept across a restart; a
/// hand-edited unknown value reads as `fast`.
#[test]
fn the_preview_preference_defaults_to_fast_and_round_trips() {
    let mut s = Session::new();
    assert_eq!(s.execute("prefs.get", json!({"path": "integrations.previewWhileGenerating"})).unwrap(), json!("fast"));
    assert_eq!(preview_mode(&s.prefs().integrations), photocraft_genai::PreviewMode::Fast);
    assert!(s.execute("prefs.set", json!({"path": "integrations.previewWhileGenerating", "value": "blurry"})).is_err());
    s.execute("prefs.set", json!({"path": "integrations.previewWhileGenerating", "value": "sharp"})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"path": "integrations.previewWhileGenerating"})).unwrap(), json!("sharp"));
    let mut restored = Session::new();
    restored.load_prefs_json(&s.prefs_to_json()).unwrap();
    assert_eq!(preview_mode(&restored.prefs().integrations), photocraft_genai::PreviewMode::Sharp);
    restored.edit_prefs(|p| p.integrations.preview_while_generating = "blurry".into());
    assert_eq!(preview_mode(&restored.prefs().integrations), photocraft_genai::PreviewMode::Fast);
}

/// Fast draft: a 2511 edit of 20 steps or more asks the worker for its step cache at
/// `FAST_DRAFT_STEP_CACHE`; off, or on a Lightning tier, it does not.
#[test]
fn fast_draft_sets_the_step_cache_on_a_2511_full_edit_only() {
    assert!(fast_draft_applies("qwen-edit-2511/edit", 0), "the template's own 40 steps");
    assert!(!fast_draft_applies("qwen-edit-2511/edit-lightning-8", 0));
    assert!(!fast_draft_applies("qwen-2.1/edit", 25), "only 2511");
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let edit = |s: &mut Session, template: &str| {
        s.execute(EDIT, json!({"prompt": "make it winter", "template": template, "wholePicture": true, "seed": 3, "enhance": false})).unwrap();
    };
    edit(&mut s, "qwen-edit-2511/edit");
    s.edit_prefs(|p| p.integrations.fast_draft = true);
    edit(&mut s, "qwen-edit-2511/edit");
    edit(&mut s, "qwen-edit-2511/edit-lightning-8");
    let st = fake.state();
    let cache = |i: usize| st.runs[i].run.params.get("step_cache").cloned();
    assert_eq!(cache(0), None, "off by default");
    assert_eq!(cache(1), Some(json!(FAST_DRAFT_STEP_CACHE)));
    assert_eq!(cache(2), None, "not on the 8-step tier");
}

#[test]
fn the_split_kept_is_the_one_whose_layers_hold_the_picture() {
    // A 10x10 layer with `on` of its 100 pixels opaque.
    let layer = |on: usize| Rgba8::new(10, 10, (0..100).flat_map(|i| [9, 9, 9, if i < on { 255 } else { 0 }]).collect()).unwrap();
    let full = || layer(100);
    // The background (first) never counts; an empty or a whole-picture layer above it neither.
    assert_eq!(split_separation(&[full(), layer(0), full()]), 0.0);
    assert_eq!(split_separation(&[layer(0), layer(1), layer(98)]), 0.0);
    // A layer holding part of the picture counts 1 plus its share up to a half.
    assert!((split_separation(&[full(), layer(30), layer(0)]) - 1.3).abs() < 1e-9);
    assert!((split_separation(&[full(), layer(80), layer(0)]) - 1.5).abs() < 1e-9);
    // Two such layers beat one, and of two with one each the fuller wins.
    assert!(split_separation(&[full(), layer(10), layer(20)]) > split_separation(&[full(), layer(50), layer(0)]));
    assert!(split_separation(&[full(), layer(40), layer(0)]) > split_separation(&[full(), layer(5), layer(0)]));
}
