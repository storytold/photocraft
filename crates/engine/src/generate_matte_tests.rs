//! `generate.removeBackground` (the model's RGBA matte as a layer mask or selection) and the
//! transparent option of `generate.image`, against the in-process fake server.

use photocraft_doc::LayerId;
use photocraft_genai::fake::{FakeNative, Options};
use photocraft_geom::Rect;
use serde_json::json;

use super::*;

/// A 64×48 document with a textured layer, talking to `url`, research models allowed.
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
    s.edit_prefs(|p| {
        {
            p.integrations.models_path = url.to_string();
            p.integrations.runtime_path = "fake".into();
            p.integrations.allow_research_models = false;
        };
        p.integrations.allow_research_models = true;
    });
    s
}

/// The fake answers with alpha 255 inside the middle half of the image (x 16..48, y 12..36 on
/// the canvas), 0 outside.
/// The fake without ViTMatte's weights (with them, Remove Background's `auto` goes to the
/// detector and ViTMatte first).
fn no_vitmatte() -> Vec<String> {
    vec!["model.safetensors".into()]
}

fn matte_server() -> FakeNative {
    FakeNative::start_with(Options { matte: Some([0.25, 0.25, 0.75, 0.75]), missing_files: no_vitmatte(), ..Options::default() }).unwrap()
}

fn mask_at(s: &Session, id: LayerId, x: i32, y: i32) -> f32 {
    let d = s.active().unwrap();
    d.doc.layer(id).unwrap().mask.as_ref().unwrap().surface.read_region(Rect::new(x, y, x + 1, y + 1))[0]
}

#[test]
fn remove_background_masks_the_layer_with_the_models_alpha_and_keeps_its_pixels() {
    let fake = matte_server();
    let mut s = session(&fake.url);
    let (id, before, steps) = {
        let d = s.active().unwrap();
        let id = d.active_layer.unwrap();
        (id, d.doc.layer(id).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 64, 48)), d.history.past_len())
    };
    let r = s.execute(REMOVE_BG, json!({"seed": 4})).unwrap();
    assert_eq!(r["layer"].as_u64(), Some(id.0));
    assert_eq!(r["selection"], false);
    assert_eq!(r["template"], DEFAULT_MATTE_TEMPLATE);
    // 64×48 goes out at 64×32 (sides in multiples of 32) and the matte comes back resampled.
    assert_eq!((r["requestWidth"].as_u64(), r["requestHeight"].as_u64()), (Some(64), Some(32)));
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(64), Some(48)));
    assert!(r["pixels"].as_u64().unwrap() > 500, "{}", r["pixels"]);
    let d = s.active().unwrap();
    assert_eq!(d.history.past_len(), steps + 1, "one undo step");
    let l = d.doc.layer(id).unwrap();
    assert_eq!(l.surface().unwrap().read_region(Rect::new(0, 0, 64, 48)), before, "the pixels are never changed");
    let m = l.mask.as_ref().expect("a layer mask");
    assert!(m.enabled);
    assert_eq!(m.surface.read_region(Rect::new(32, 24, 33, 25))[0], 1.0, "the subject is shown");
    assert_eq!(m.surface.read_region(Rect::new(2, 2, 3, 3))[0], 0.0, "the background is hidden");
    assert_eq!(m.surface.read_region(Rect::new(32, 44, 33, 45))[0], 0.0);
    assert!(d.doc.selection.is_none(), "like the Quick Action, the selection is dropped");
    // What the server got: first the question naming the subject (no prompt given; the fake
    // answers "Enhanced: <question>"), then the 2.1 edit graph with the picture as image_1 at
    // its own size and that name wrapped into the instruction.
    let st = fake.state();
    assert!(st.runs.len() >= 2, "the naming question, then the matte");
    let g = &st.runs.last().unwrap().request;
    assert!(g["mask"].is_null());
    assert!(g["references"].as_array().unwrap().is_empty());
    let instruction = g["prompt"].as_str().unwrap_or("");
    assert!(instruction.starts_with("Remove the background, keeping only Enhanced:") && instruction != MATTE_DEFAULT_PROMPT, "{instruction}");
    assert_eq!(g["seed"], 4);
    assert_eq!(g["steps"], 25);
    let sent = st.pictures[0].clone();
    assert_eq!((sent.width, sent.height), (64, 32));
    drop(st);
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).unwrap().mask.is_none(), "undo removes the mask");
}

#[test]
fn naming_the_subject_wraps_it_into_the_instruction_and_the_background_becomes_a_layer() {
    let fake = matte_server();
    let mut s = session(&fake.url);
    // The Background layer (index 0) cannot carry a mask: it becomes a normal layer first.
    let bg = s.active().unwrap().doc.layers[0].id;
    let r = s.execute(REMOVE_BG, json!({"layer": bg.0, "prompt": "the lighthouse"})).unwrap();
    assert_eq!(r["layer"].as_u64(), Some(bg.0));
    let d = s.active().unwrap();
    let l = d.doc.layer(bg).unwrap();
    assert!(l.mask.is_some());
    assert_ne!(l.name, "Background");
    assert_eq!(mask_at(&s, bg, 32, 24), 1.0);
    let g = &fake.state().runs[0].request;
    assert_eq!(g["prompt"], "Remove the background, keeping only the lighthouse, and output a PNG image");
}

#[test]
fn as_selection_loads_the_matte_as_the_selection_and_leaves_the_layer_alone() {
    let fake = matte_server();
    let mut s = session(&fake.url);
    let id = s.active().unwrap().active_layer.unwrap();
    let r = s.execute(REMOVE_BG, json!({"asSelection": true})).unwrap();
    assert_eq!(r["selection"], true);
    let d = s.active().unwrap();
    assert!(d.doc.layer(id).unwrap().mask.is_none(), "no mask");
    let sel = d.doc.selection.as_ref().expect("a selection");
    assert_eq!(sel.read_region(Rect::new(32, 24, 33, 25))[0], 1.0);
    assert_eq!(sel.read_region(Rect::new(2, 2, 3, 3))[0], 0.0);
    // The matte's bounds on the canvas: exact across (64 → 64), a row short down (the fake's
    // half-covered border rows blur under 0.5 when 32 rows become 48).
    let b = r["bounds"].as_array().unwrap();
    assert_eq!((b[0].as_i64(), b[2].as_i64()), (Some(16), Some(32)), "{b:?}");
    assert!((12..=13).contains(&b[1].as_i64().unwrap()) && (22..=24).contains(&b[3].as_i64().unwrap()), "{b:?}");
    // `add` keeps what was selected before.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    s.execute(REMOVE_BG, json!({"asSelection": true, "mode": "add"})).unwrap();
    let sel = s.active().unwrap().doc.selection.clone().unwrap();
    assert_eq!(sel.read_region(Rect::new(2, 2, 3, 3))[0], 1.0, "the earlier rectangle stays");
    assert_eq!(sel.read_region(Rect::new(32, 24, 33, 25))[0], 1.0);
}

#[test]
fn remove_background_validates_and_runs_the_research_matte_by_name() {
    let fake = matte_server();
    let mut s = session(&fake.url);
    for p in [json!({"asSelection": "yes"}), json!({"mode": "xor"}), json!({"sampleAllLayers": 1}), json!({"template": "qwen-edit-2511/fill"})] {
        assert!(matches!(s.execute(REMOVE_BG, p.clone()), Err(EngineError::BadParams { .. })), "{p}");
    }
    // The research preference off only steers Auto: the matte model asked for by name runs.
    s.edit_prefs(|p| p.integrations.allow_research_models = false);
    assert_eq!(s.execute(REMOVE_BG, json!({"template": "qwen-2.1/matte"})).unwrap()["template"], "qwen-2.1/matte");
    s.edit_prefs(|p| p.integrations.allow_research_models = true);
    // A fully transparent answer: the model "kept nothing", and nothing changes.
    let empty = FakeNative::start_with(Options { matte: Some([0.0, 0.0, 0.0, 0.0]), missing_files: no_vitmatte(), ..Options::default() }).unwrap();
    let mut s = session(&empty.url);
    let e = s.execute(REMOVE_BG, json!({})).unwrap_err().to_string();
    assert!(e.contains("kept nothing"), "{e}");
    assert!(s.active().unwrap().doc.layers.last().unwrap().mask.is_none(), "an error leaves no half-done mask");
}

#[test]
fn without_the_research_opt_in_remove_background_uses_the_permissive_detector() {
    // The fake detector "finds" the centre quarter: x 16..48, y 12..36 on 64×48.
    let fake = FakeNative::start_with(Options { missing_files: no_vitmatte(), ..Options::default() }).unwrap();
    let mut s = session(&fake.url);
    s.edit_prefs(|p| p.integrations.allow_research_models = false);
    let id = s.active().unwrap().active_layer.unwrap();
    let r = s.execute(REMOVE_BG, json!({"prompt": "the boat", "refine": false})).unwrap();
    assert_eq!(r["template"], "sam3.1/segment");
    assert_eq!(r["bounds"], json!([16, 12, 32, 24]), "unrefined: the detector's rectangle exactly");
    assert_eq!(mask_at(&s, id, 32, 24), 1.0);
    assert_eq!(mask_at(&s, id, 2, 2), 0.0);
    assert_eq!(mask_at(&s, id, 16, 12), 1.0, "a hard corner");
    // The default refines the hard mask against the pixels: still the subject, softer edges.
    let r = s.execute(REMOVE_BG, json!({"prompt": "the boat"})).unwrap();
    let b = r["bounds"].as_array().unwrap();
    assert!((13..=19).contains(&b[0].as_i64().unwrap()) && (9..=15).contains(&b[1].as_i64().unwrap()), "{b:?}");
    assert_eq!(mask_at(&s, id, 32, 24), 1.0);
    assert_eq!(mask_at(&s, id, 2, 2), 0.0);
    let edge: Vec<f32> = (12..20).map(|x| mask_at(&s, id, x, 24)).collect();
    assert!(edge.iter().any(|v| *v > 0.0 && *v < 1.0), "partial coverage across the edge: {edge:?}");
    {
        let st = fake.state();
        let g = &st.runs[0].request;
        assert_eq!(g["prompt"], "the boat");
        assert_eq!(g["params"]["threshold"], 0.5);
    }
    // No subject named: the detector is asked for the main subject. As a selection, too.
    let r = s.execute(REMOVE_BG, json!({"asSelection": true})).unwrap();
    assert_eq!(r["selection"], true);
    assert!(s.active().unwrap().doc.selection.is_some());
    assert_eq!(fake.state().runs[2].request["prompt"], "the main subject");
    // Research allowed but the 2.1 files missing: `auto` still falls back to the detector.
    let bare =
        FakeNative::start_with(Options { missing_files: vec!["qwen_image_2.1_bf16.safetensors".into(), "model.safetensors".into()], ..Options::default() })
            .unwrap();
    let mut s = session(&bare.url);
    let r = s.execute(REMOVE_BG, json!({"prompt": "the boat"})).unwrap();
    assert_eq!(r["template"], "sam3.1/segment");
    let m = s.execute(MODELS, json!({})).unwrap();
    assert_eq!(m["autoMatte"], "sam3.1/segment");
    s.edit_prefs(|p| p.integrations.allow_research_models = false);
    assert_eq!(s.execute(MODELS, json!({})).unwrap()["autoMatte"], "sam3.1/segment");
    // With everything installed and allowed, auto is the matte model.
    let full = matte_server();
    let mut s = session(&full.url);
    assert_eq!(s.execute(MODELS, json!({})).unwrap()["autoMatte"], DEFAULT_MATTE_TEMPLATE);
}

#[test]
fn a_transparent_generate_image_asks_the_template_for_it_and_keeps_the_alpha() {
    let fake = matte_server();
    let mut s = session(&fake.url);
    let r = s.execute(IMAGE, json!({"prompt": "a red rowing boat", "transparent": true, "template": "qwen-2.1/image", "seed": 2})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let d = s.active().unwrap();
    let l = d.doc.layer(id).unwrap();
    assert_eq!(l.name, "Generated: a red rowing boat", "the name is the prompt as typed");
    let surf = l.surface().unwrap();
    assert_eq!(surf.read_region(Rect::new(32, 24, 33, 25))[3], 1.0, "the subject is opaque");
    assert_eq!(surf.read_region(Rect::new(2, 2, 3, 3))[3], 0.0, "the background is transparent in the pixels themselves");
    let g = &fake.state().runs[0].request;
    assert_eq!(g["prompt"], "a red rowing boat, isolated on a transparent background, output a PNG image");
    // A template whose model cannot output transparency refuses.
    let e = s.execute(IMAGE, json!({"prompt": "x", "transparent": true})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }) && e.to_string().contains("transparency"), "{e}");
    assert!(matches!(s.execute(IMAGE, json!({"prompt": "x", "transparent": "yes"})), Err(EngineError::BadParams { .. })));
}

#[test]
fn with_vitmatte_remove_background_auto_goes_to_the_detector_and_vitmatte() {
    // The fake detector "finds" the centre quarter; ViTMatte (the fake) answers opaque, so the
    // mask is the detector's grown by the trimap's band, solid inside, nothing far out.
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    let id = s.active().unwrap().active_layer.unwrap();
    let r = s.execute(REMOVE_BG, json!({})).unwrap();
    assert_eq!(r["template"], "sam3.1/segment", "{r}");
    assert_eq!(r["vitmatte"], true, "{r}");
    assert_eq!(mask_at(&s, id, 32, 24), 1.0);
    assert_eq!(mask_at(&s, id, 0, 0), 0.0);
    let st = fake.state();
    assert!(st.runs.iter().any(|r| r.request["model"] == "vitmatte/refine" && r.request["mask"].is_object()), "ViTMatte with a trimap");
}
