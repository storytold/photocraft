//! Reference images on `generate.edit` / `generate.image`, image variations and the time
//! estimate, against the fake server.

use photocraft_doc::LayerId;
use photocraft_genai::fake::FakeNative;
use serde_json::{Value, json};

use super::*;
use crate::Session;
use crate::generate_cmds::{EDIT, FILL, IMAGE, SIMILAR, estimate_seconds, generative_info};

/// A 64×48 document with a textured layer, a second small layer (the reference) and a
/// selection, talking to `url`.
fn session(url: &str) -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let paint = |s: &mut Session, rect: photocraft_geom::Rect, rgba: [f32; 4]| {
        s.edit("paint", |doc, active| {
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let n = surf.channels();
            let mut tmp = [0.0f32; 8];
            let used = photocraft_raster::from_rgba_into(&surf.format(), rgba, &mut tmp).min(n);
            let px: Vec<f32> = (0..rect.width() * rect.height()).flat_map(|_| tmp[..used].to_vec()).collect();
            surf.write_region(rect, &px);
            Ok(())
        })
        .unwrap();
    };
    s.execute("layer.new.layer", json!({})).unwrap();
    paint(&mut s, photocraft_geom::Rect::new(0, 0, 64, 48), [0.2, 0.6, 0.9, 1.0]);
    let base = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Hat"})).unwrap();
    paint(&mut s, photocraft_geom::Rect::new(10, 10, 30, 22), [0.9, 0.1, 0.1, 1.0]);
    let hat = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.select", json!({"layer": base.0})).unwrap();
    s.execute("select.rect", json!({"x": 10, "y": 8, "width": 20, "height": 16})).unwrap();
    s.edit_prefs(|p| {
        p.integrations.models_path = url.to_string();
        p.integrations.runtime_path = "fake".into();
        p.integrations.allow_research_models = false;
    });
    (s, base, hat)
}

fn uploads(fake: &FakeNative) -> Vec<(String, (u32, u32))> {
    fake.state().pictures.iter().enumerate().map(|(n, p)| (n.to_string(), (p.width, p.height))).collect()
}

#[test]
fn an_edit_sends_its_layer_references_and_similar_sends_them_again() {
    let fake = FakeNative::start().unwrap();
    let (mut s, base, hat) = session(&fake.url);
    let r = s.execute(EDIT, json!({"prompt": "put the hat from image 2 on the sky", "seed": 3, "references": [{"layer": hat.0}, {"layer": base.0}]})).unwrap();
    let up = uploads(&fake);
    assert_eq!(up.len(), 3, "the picture and two references: {up:?}");
    assert_eq!(up[1].1, (20, 12), "the hat layer over its content bounds");
    {
        let st = fake.state();
        assert_eq!(st.runs[0].run.references.len(), 2);
    }
    let layer = LayerId(r["layer"].as_u64().unwrap());
    let info = generative_info(s.active().unwrap().doc.layer(layer).unwrap()).unwrap();
    assert_eq!(info.references, vec![json!({"layer": hat.0}), json!({"layer": base.0})]);
    // Similar resolves them again.
    s.execute(SIMILAR, json!({"layer": layer.0, "seed": 4})).unwrap();
    let st = fake.state();
    assert_eq!(st.runs.len(), 2);
    assert!(st.runs[1].request["references"].as_array().is_some_and(|r| r.len() == 2), "both references again");
}

#[test]
fn reference_counts_and_specs_are_checked_before_anything_runs() {
    let fake = FakeNative::start().unwrap();
    let (mut s, base, hat) = session(&fake.url);
    let three = json!([{"layer": hat.0}, {"layer": base.0}, {"layer": hat.0}]);
    let e = s.execute(EDIT, json!({"prompt": "x", "references": three})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg.contains("at most 2")), "{e}");
    // A fill with references switches to the reference template, which takes two; a plain
    // fill template named outright takes none.
    let e = s.execute(FILL, json!({"prompt": "x", "references": three})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { msg, .. } if msg.contains("at most 2")), "{e}");
    let e = s.execute(FILL, json!({"prompt": "x", "template": "qwen-edit-2511/fill", "references": [{"layer": hat.0}]})).unwrap_err();
    assert!(e.to_string().contains("takes no reference images"), "{e}");
    for bad in [json!("x"), json!([5]), json!([{}]), json!([{"layer": 1, "path": "a"}]), json!([{"job": "x"}]), json!([{"path": ""}])] {
        assert!(matches!(s.execute(EDIT, json!({"prompt": "x", "references": bad.clone()})), Err(EngineError::BadParams { .. })), "{bad}");
    }
    let e = s.execute(EDIT, json!({"prompt": "x", "references": [{"layer": 99_999}]})).unwrap_err();
    assert!(e.to_string().contains("reference 1"), "{e}");
    let e = s.execute(EDIT, json!({"prompt": "x", "references": [{"job": 42}]})).unwrap_err();
    assert!(e.to_string().contains("no stored result"), "{e}");
    let e = s.execute(IMAGE, json!({"prompt": "x", "template": "krea2-turbo/image", "references": [{"layer": hat.0}]})).unwrap_err();
    assert!(e.to_string().contains("takes no reference images"), "{e}");
    let e = s.execute(IMAGE, json!({"prompt": "x", "template": "qwen-edit-2511/image-ref"})).unwrap_err();
    assert!(e.to_string().contains("at least 1"), "{e}");
    assert!(fake.state().runs.is_empty(), "nothing queued");
}

/// A fill given a reference image (no template named) runs the fill-from-reference template:
/// the selection goes out flattened to grey, the reference on image 2, the prompt names the hole;
/// the layer remembers it all and Generate Similar sends the reference again.
#[test]
fn a_fill_with_a_reference_erases_the_selection_and_runs_the_reference_template() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, hat) = session(&fake.url);
    let want = crate::generate_cmds::fill_ref_template(crate::generate_cmds::quality(&s));
    let r = s.execute(FILL, json!({"prompt": "the red hat", "seed": 3, "edge": "hard", "references": [{"layer": hat.0}]})).unwrap();
    assert_eq!(r["template"], want);
    let up = uploads(&fake);
    assert_eq!(up.len(), 3, "the picture, its mask and the reference: {up:?}");
    assert_eq!(up[2].1, (20, 12), "the hat layer over its content bounds");
    {
        let st = fake.state();
        let g = &st.runs[0].request;
        let prompt = g["prompt"].as_str().unwrap_or_default();
        assert!(prompt.contains("flat grey area") && prompt.contains("the red hat"), "{prompt}");
        // Selection 20×16 at (10, 8) with its margin: a 32×32 request from (5, 3), sent at
        // 512²; document (x, y) is request ((x − 5) × 16, (y − 3) × 16).
        let sent = st.pictures[0].clone();
        assert_eq!(sent.get(200, 200), Some([128, 128, 128, 255]), "inside the selection: flat grey");
        assert_ne!(sent.get(8, 8).map(|p| [p[0], p[1], p[2]]), Some([128, 128, 128]), "the margin keeps its pixels");
    }
    let layer = LayerId(r["layer"].as_u64().unwrap());
    let info = generative_info(s.active().unwrap().doc.layer(layer).unwrap()).unwrap();
    assert_eq!((info.template.as_str(), info.erase, info.grow), (want, true, 0));
    assert_eq!(info.references, vec![json!({"layer": hat.0})]);
    s.execute(SIMILAR, json!({"layer": layer.0, "seed": 4})).unwrap();
    let st = fake.state();
    assert_eq!(st.runs.len(), 2);
    assert!(st.runs[1].request["references"].as_array().is_some_and(|r| r.len() == 1), "the reference again");
    let sent = st.pictures[3].clone();
    assert_eq!(sent.get(200, 200), Some([128, 128, 128, 255]), "erased again");
}

/// `grow` widens the selection before the request (and the result's mask with it); `erase`
/// flattens it on any fill template, and off keeps the pixels on the reference template.
#[test]
fn grow_widens_the_selection_and_erase_is_a_choice() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, hat) = session(&fake.url);
    let r = s.execute(FILL, json!({"prompt": "x", "seed": 1, "edge": "hard", "grow": 4, "erase": true})).unwrap();
    assert_eq!(r["template"], crate::generate_cmds::likely_auto(FILL, crate::generate_cmds::quality(&s)), "a plain fill still");
    let layer = LayerId(r["layer"].as_u64().unwrap());
    let d = s.active().unwrap();
    let mask = &d.doc.layer(layer).unwrap().mask.as_ref().unwrap().surface;
    assert_eq!(mask.read_region(photocraft_geom::Rect::new(7, 8, 8, 9)), vec![1.0], "3 px left of the selection: grown into");
    assert_eq!(mask.read_region(photocraft_geom::Rect::new(10, 2, 11, 3)), vec![0.0], "6 px above: not");
    let info = generative_info(d.doc.layer(layer).unwrap()).unwrap();
    assert_eq!((info.grow, info.erase), (4, true));
    {
        let st = fake.state();
        let sent = st.pictures[0].clone();
        assert_eq!(sent.get(72, 120), Some([128, 128, 128, 255]), "document (9, 10), left of the selection: grown and erased");
    }
    // The reference template with `erase: false`: the pixels stay.
    s.execute(FILL, json!({"prompt": "x", "seed": 1, "erase": false, "references": [{"layer": hat.0}]})).unwrap();
    let st = fake.state();
    let sent = st.pictures[3].clone();
    assert_ne!(sent.get(200, 200).map(|p| [p[0], p[1], p[2]]), Some([128, 128, 128]));
    assert!(matches!(s.execute(FILL, json!({"prompt": "x", "grow": 1.5})), Err(EngineError::BadParams { .. })));
    assert!(matches!(s.execute(FILL, json!({"prompt": "x", "grow": 9999})), Err(EngineError::BadParams { .. })));
}

/// A removal through a Qwen-Image-2.1 template shows the model the picture as it is; through
/// 2511 the hole is smoothed over first (`prefill`). The red hat sits inside the selection with
/// blue all round it, so the smoothed hole is blue where the real picture is red.
#[test]
fn a_qwen_21_removal_sees_the_picture_as_it_is_and_a_2511_one_the_hole_smoothed() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, _) = session(&fake.url);
    // The picture is a run's first upload (a fill's mask follows it).
    let centre_of_hole = |fake: &FakeNative, first: usize| -> [u8; 4] {
        let st = fake.state();
        let pic = st.pictures[first].clone();
        // The request is the whole 64×48 canvas resampled; the hole's centre is (20, 16).
        pic.get(pic.width * 20 / 64, pic.height * 16 / 48).unwrap()
    };
    s.edit_prefs(|p| {
        p.integrations.allow_research_models = true;
        p.integrations.default_fill_template = "qwen-2.1/edit".into();
    });
    let r = s.execute("edit.contentAwareFill", json!({"generative": true, "variations": 1, "enhance": false})).unwrap();
    assert_eq!(r["template"], "qwen-2.1/edit", "{r}");
    let [rd, _, b, _] = centre_of_hole(&fake, 0);
    assert!(rd > 180 && b < 80, "2.1 sees the red hat: {rd} {b}");
    s.edit_prefs(|p| {
        p.integrations.default_fill_template = "auto".into();
        p.integrations.allow_research_models = false;
    });
    let before = fake.state().pictures.len();
    let r = s.execute("edit.contentAwareFill", json!({"generative": true, "variations": 1, "enhance": false})).unwrap();
    assert!(r["template"].as_str().unwrap_or_default().starts_with("qwen-edit-2511/"), "{r}");
    let [rd, _, b, _] = centre_of_hole(&fake, before);
    assert!(b > rd, "2511 sees the hole smoothed from the blue around it: {rd} {b}");
}

/// Content-Aware Fill, the Remove tool and Spot Healing go through the fill model when asked
/// (`generative`), with the removal instruction and the hole flattened to grey, each as a masked
/// layer; without a backend configured they fall back to their classical fills, in place.
#[test]
fn the_classical_tools_go_through_the_model_when_asked_and_fall_back_without_a_server() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, _) = session(&fake.url);
    let layers = s.active().unwrap().doc.layers.len();
    let r = s.execute("edit.contentAwareFill", json!({"generative": true, "variations": 1})).unwrap();
    assert!(r["layer"].is_number(), "a generative layer: {r}");
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 1);
    // Under Quality best (the default) a removal still runs the 8-step tier: the 40-step one
    // was no better on removals and four times the time.
    assert_eq!(crate::generate_cmds::quality(&s), "best");
    assert_eq!(r["template"], "qwen-edit-2511/fill-lightning-8", "{r}");
    {
        let st = fake.state();
        // The rewriter names the object first (the fake echoes the instruction), then the fill.
        assert_eq!(st.runs.len(), 2, "the rewriter's ask, then the fill");
        let prompt = st.runs[1].request["prompt"].as_str().unwrap_or_default();
        assert!(prompt.contains("Remove the object in the selected area") && prompt.to_lowercase().contains("change nothing else"), "{prompt}");
        // Selection 20×16 at (10, 8): the 32×32 request from (5, 3) at 512²; (200, 200) is inside
        // and keeps the picture's pixels (a grey hole is painted back as a panel: measured).
        let sent = st.pictures[0].clone();
        assert_ne!(sent.get(200, 200).map(|p| [p[0], p[1], p[2]]), Some([128, 128, 128]), "the pixels stay");
    }
    let made = LayerId(r["layer"].as_u64().unwrap());
    let info = generative_info(s.active().unwrap().doc.layer(made).unwrap()).unwrap();
    assert_eq!((info.command.as_str(), info.erase, info.name.as_str()), (FILL, false, "Content-Aware Fill"));
    // The fake's rewrite ("Enhanced: …") names nothing, so the fill keeps the generic wording
    // and the layer records no rewrite (a named removal records the wording with the name).
    assert!(info.enhanced.is_empty(), "{info:?}");
    // `keepObject`: what was removed stays as its own masked layer above the fill (AusBoss's
    // Cut Out + Remove), named after the object or "object" when nothing was named.
    let before = s.active().unwrap().doc.layers.len();
    let r = s.execute("edit.contentAwareFill", json!({"generative": true, "variations": 1, "keepObject": true})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), before + 2, "the fill and the object: {r}");
    let kept = LayerId(r["objectLayer"].as_u64().unwrap_or_else(|| panic!("{r}")));
    let layer = s.active().unwrap().doc.layer(kept).unwrap();
    assert_eq!(layer.name, "Content-Aware Fill: object");
    assert!(layer.mask.as_ref().is_some_and(|m| m.enabled), "masked to the object's silhouette");
    assert!(matches!(s.execute("edit.contentAwareFill", json!({"generative": true, "keepObject": "yes"})), Err(EngineError::BadParams { .. })));
    // The Remove tool and Spot Healing: a stroke each, a layer each; each asks the detector
    // for the object under the stroke first (the fake finds nothing: the stroke alone), then
    // the rewriter, then the fill.
    s.execute("paint.remove", json!({"points": [[20, 16], [26, 16]], "size": 6, "generative": true, "variations": 1})).unwrap();
    s.execute("paint.spotHealing", json!({"points": [[22, 18]], "size": 6, "type": "generative", "variations": 1})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), layers + 5, "the first fill, the kept pair, a layer each for the two tools");
    assert_eq!(fake.state().runs.len(), 10, "two detector asks, two rewrites, two fills more");
    // No backend configured: the classical fills, in place, no layer.
    s.edit_prefs(|p| {
        p.integrations.models_path.clear();
        p.integrations.runtime_path.clear();
        p.integrations.models_path.clear();
    });
    let n = s.active().unwrap().doc.layers.len();
    let r = s.execute("edit.contentAwareFill", json!({"generative": true, "variations": 1})).unwrap();
    assert!(r.get("filled").is_some() && r.get("runId").is_none(), "the classical fill: {r}");
    s.execute("paint.remove", json!({"points": [[20, 16], [26, 16]], "size": 6, "generative": true, "variations": 1})).unwrap();
    s.execute("paint.spotHealing", json!({"points": [[22, 18]], "size": 6, "type": "generative", "variations": 1})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), n);
    assert_eq!(fake.state().runs.len(), 10, "nothing more was asked of the server");
}

#[test]
fn a_fills_denoise_reaches_the_2511_sampler_and_is_one_unless_given() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, _) = session(&fake.url);
    s.execute("file.new", json!({"width": 256, "height": 256})).unwrap();
    s.execute("select.rect", json!({"x": 64, "y": 64, "width": 96, "height": 96})).unwrap();
    for denoise in [None, Some(0.6)] {
        let mut p = json!({"prompt": "a red hat", "template": "qwen-edit-2511/fill-lightning-8", "seed": 2, "enhance": false});
        if let Some(d) = denoise {
            p["denoise"] = json!(d);
        }
        s.execute("generate.fill", p).unwrap();
        let st = fake.state();
        let g = &st.runs.last().unwrap().request;
        assert_eq!(g["params"]["denoise"].as_f64(), denoise, "omitted means the worker default of 1");
    }
    assert!(crate::generate_cmds::takes_denoise("qwen-edit-2511/fill-lightning-8"));
    assert!(!crate::generate_cmds::takes_denoise("qwen-2.1/fill"), "the 2.1 fill redraws from noise");
    assert!(s.execute("generate.fill", json!({"prompt": "x", "denoise": 0.0})).is_err(), "0.05 to 1");
}

#[test]
fn name_shadow_asks_a_whole_picture_removal_for_the_objects_shadow_too() {
    use photocraft_genai::fake::Options;
    let fake = FakeNative::start_with(Options {
        segments_for: vec![("red hat".into(), vec![[0.25, 1.0 / 3.0, 0.75, 170.0 / 300.0]])],
        enhanced_text: Some("Remove the red hat.".into()),
        ..Options::default()
    })
    .unwrap();
    let (mut s, _, _) = session(&fake.url);
    s.edit_prefs(|p| {
        p.integrations.default_fill_template = "qwen-2.1/edit".into();
        p.integrations.allow_research_models = true;
    });
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let stroke = |extra: serde_json::Value| {
        let mut p = json!({"points": [[110, 135], [210, 135]], "size": 30, "hardness": 1, "type": "generative", "variations": 1, "seed": 3});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        p
    };
    let mut said = Vec::new();
    for extra in [json!({}), json!({"nameShadow": true})] {
        let r = s.execute("paint.spotHealing", stroke(extra)).unwrap();
        let info = generative_info(s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap()).unwrap();
        said.push(info.enhanced);
    }
    assert!(said[0].starts_with("Remove the red hat and fill its place"), "off by default: {said:?}");
    assert!(said[1].starts_with("Remove the red hat and its shadow and fill its place"), "{said:?}");
}

#[test]
fn a_generative_stroke_takes_the_object_under_it_whole_when_it_covers_enough_of_it() {
    use photocraft_genai::fake::Options;
    // A 400×300 picture; the detector "finds" an object at (100, 100)–(220, 170) under the
    // stroke's spots (8400 pixels), and, asked by the name the rewriter gives it, the whole
    // "red hat" at (100, 100)–(300, 170) (14000 pixels: a thin object's point prompt returns
    // only the brushed part).
    let fake = FakeNative::start_with(Options {
        segments: vec![[0.25, 1.0 / 3.0, 0.55, 170.0 / 300.0]],
        segments_for: vec![("red hat".into(), vec![[0.25, 1.0 / 3.0, 0.75, 170.0 / 300.0]])],
        enhanced_text: Some("Remove the red hat.".into()),
        ..Options::default()
    })
    .unwrap();
    let (mut s, _, _) = session(&fake.url);
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    // A stroke across the object (its footprint 150×51, from x 85 to 235): the rewriter names
    // "the red hat", the detector finds the whole of it by that name and the stroke covers
    // over a tenth of it, so it goes whole, the hole grown by 15 % of its larger side (the
    // stroke and the whole hat: x 85 to 300 → 215 → 32). The point prompt is not asked.
    let r = s
        .execute("paint.spotHealing", json!({"points": [[110, 135], [210, 135]], "size": 30, "hardness": 1, "type": "generative", "variations": 1, "seed": 3}))
        .unwrap();
    let made = LayerId(r["layer"].as_u64().unwrap());
    let info = generative_info(s.active().unwrap().doc.layer(made).unwrap()).unwrap();
    assert_eq!(info.grow, 32, "{info:?}");
    // The rewriter gives the name; the fill keeps the generic wording around it.
    assert_eq!(
        info.enhanced,
        "Remove the red hat and fill its place with the surrounding scene continued seamlessly (its background, surfaces, textures, patterns and lines), as if nothing had ever been there",
        "{info:?}"
    );
    let rect = photocraft_geom::Rect::new(info.rect[0], info.rect[1], info.rect[0] + info.rect[2], info.rect[1] + info.rect[3]);
    assert!(rect.contains(70, 70) && rect.contains(330, 200), "the request takes in the whole hat, its growth and margin: {rect:?}");
    {
        let st = fake.state();
        assert_eq!(st.runs.len(), 3, "the rewriter, the detector by name, the fill");
        assert!(st.runs[1].run.prompt == "red hat", "asked by the rewriter's name: {}", st.runs[1].request);
    }
    // A dab on one spot of it (a blemish, not the object): the named hat is 14000 pixels, the
    // dab covers under a tenth of it, and so of the "hat" the shorter name finds (the fake's
    // default object, 8400 pixels); the point prompt's object (the same) is far bigger than
    // the dab's box. The hat stays, the dab is healed with the least growth.
    let r =
        s.execute("paint.spotHealing", json!({"points": [[120, 130]], "size": 8, "hardness": 1, "type": "generative", "variations": 1, "seed": 3})).unwrap();
    let made = LayerId(r["layer"].as_u64().unwrap());
    let info = generative_info(s.active().unwrap().doc.layer(made).unwrap()).unwrap();
    assert_eq!(info.grow, 16, "{info:?}");
    let rect = photocraft_geom::Rect::new(info.rect[0], info.rect[1], info.rect[0] + info.rect[2], info.rect[1] + info.rect[3]);
    // The request is widened to at least 384 px a side (the canvas is 300 tall) so the model
    // sees context around the dab, but no further.
    assert!(rect.width() == 384 && rect.height() == 300 && !rect.contains(399, 10), "a dab's request: {rect:?}");
    assert_eq!(fake.state().runs.len(), 8, "the rewriter, the detector by \"red hat\" then \"hat\", the detector by point, the fill");
    // `wholeObject: false` never asks the detector; `enhance: false` never asks the rewriter.
    s.execute(
        "paint.remove",
        json!({"points": [[110, 135], [210, 135]], "size": 30, "generative": true, "wholeObject": false, "enhance": false, "variations": 1}),
    )
    .unwrap();
    assert_eq!(fake.state().runs.len(), 9, "the fill alone for the Remove stroke");
    // With no rewriter (`enhance: false`) the point prompt's object is taken when it fits
    // the stroke's box: the stroke's box is 150×51, the object 120×70 (under 1.5× each way).
    let r = s
        .execute(
            "paint.spotHealing",
            json!({"points": [[110, 135], [210, 135]], "size": 30, "hardness": 1, "type": "generative", "enhance": false, "variations": 1, "seed": 3}),
        )
        .unwrap();
    let made = LayerId(r["layer"].as_u64().unwrap());
    let info = generative_info(s.active().unwrap().doc.layer(made).unwrap()).unwrap();
    assert_eq!((info.grow, info.enhanced.as_str()), (23, ""), "the stroke and the point object, x 85 to 235: {info:?}");
    assert_eq!(fake.state().runs.len(), 11, "the detector by point, the fill");
}

#[test]
fn a_removal_matches_the_tone_of_its_surroundings_at_the_hole() {
    // The fake paints its result a flat colour; the document is another. A removal's result
    // comes back matched to the document at the hole's edge, so the layer's pixels inside the
    // hole are the document's tone, not the fake's; a plain fill keeps the fake's colour.
    use photocraft_genai::fake::Options;
    let fake = FakeNative::start_with(Options { color: [200, 120, 40, 255], ..Options::default() }).unwrap();
    let (mut s, _, _) = session(&fake.url);
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("edit.fill", json!({"color": "#2060a0"})).unwrap();
    let sample = |s: &Session, layer: LayerId, x: i32, y: i32| -> [u8; 4] {
        let l = s.active().unwrap().doc.layer(layer).unwrap();
        let mut px = [[0u8; 4]; 1];
        l.surface().unwrap().read_rgba8_into(photocraft_geom::Rect::new(x, y, x + 1, y + 1), &mut px);
        px[0]
    };
    let before = s.active().unwrap().doc.layers.len();
    let r = s
        .execute(
            "paint.spotHealing",
            json!({"points": [[200, 150]], "size": 40, "hardness": 1, "type": "generative", "enhance": false, "wholeObject": false, "seed": 3}),
        )
        .unwrap();
    let removed = sample(&s, LayerId(r["layer"].as_u64().unwrap()), 200, 150);
    assert!(removed[0] < 80 && removed[2] > 120, "the document's blue, not the fake's orange: {removed:?}");
    // Two seeds by default: two layers, the shown one first (the fake paints both alike).
    let ids: Vec<u64> = r["layers"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
    assert_eq!((ids.len(), s.active().unwrap().doc.layers.len()), (2, before + 2), "{r}");
    let d = s.active().unwrap();
    assert!(d.doc.layer(LayerId(ids[0])).unwrap().visible && !d.doc.layer(LayerId(ids[1])).unwrap().visible);
    s.execute("select.rect", json!({"x": 180, "y": 130, "width": 40, "height": 40})).unwrap();
    let r = s.execute(FILL, json!({"prompt": "a mark", "enhance": false, "seed": 3})).unwrap();
    let filled = sample(&s, LayerId(r["layer"].as_u64().unwrap()), 200, 150);
    assert!(filled[0] > 150 && filled[2] < 80, "a fill keeps what the model painted: {filled:?}");
}

#[test]
fn a_rewritten_removal_names_its_object() {
    use crate::generate_cmds::named_object;
    assert_eq!(
        named_object("Remove the gold necklace and replace it with the surrounding skin, collarbone, and dark grey fabric."),
        Some("gold necklace".into())
    );
    assert_eq!(named_object("Remove the gold pendant and chain, replacing them with skin."), Some("gold pendant and chain".into()));
    assert_eq!(named_object("Remove the red boat from the water so the sea continues."), Some("red boat".into()));
    assert_eq!(named_object("remove the small bird on the wire."), Some("small bird".into()));
    assert_eq!(named_object("Fill the area with sea."), None, "not a removal");
    assert_eq!(named_object("Remove the very long and winding description of something that goes on and on."), None, "a sentence, not a name");
    assert_eq!(named_object("Remove the black crow perched on the wire and fill its place with the sky."), Some("black crow".into()));
    assert_eq!(named_object("Remove the painted-over area."), None, "the selection, not a thing");
    assert_eq!(named_object("Remove the selected region of the cup."), None);
    assert_eq!(named_object("Remove the selected object."), None);
    assert_eq!(named_object("Remove the black frame."), None, "a photograph's frame or border is never what a stroke on the picture means");
    assert_eq!(named_object("Remove the bicycle leaning against the wall."), Some("bicycle".into()), "not the wall it leans on");
    assert_eq!(
        named_object("Remove the black silhouette of the standing person."),
        Some("black silhouette of the standing person".into()),
        "\"standing\" after an article is part of the name, not where it ends"
    );
    use crate::generate_cmds::wearer_candidates;
    assert_eq!(
        wearer_candidates("man wearing the cap and mask"),
        Some(vec!["cap".to_string(), "mask".into(), "cap and mask".into(), "man".into()]),
        "each worn thing, then all of them, then the wearer: the stroke decides"
    );
    assert_eq!(wearer_candidates("woman holding a red umbrella"), Some(vec!["red umbrella".to_string(), "woman".into()]));
    assert_eq!(wearer_candidates("white face mask"), None, "a plain name keeps the plain candidates");
    use crate::generate_cmds::{name_candidates, removal_prompt};
    assert_eq!(name_candidates("white ceramic coffee cup"), vec!["white ceramic coffee cup", "coffee cup", "cup"]);
    assert_eq!(name_candidates("red boat"), vec!["red boat", "boat"]);
    assert_eq!(name_candidates("dog"), vec!["dog"]);
    assert!(removal_prompt("silver watch").starts_with("Remove the silver watch and fill its place with the surrounding scene continued seamlessly"));
}

#[test]
fn the_worker_names_a_removal_from_a_tighter_close_up_and_a_wide_one_keeps_its_shape() {
    use crate::generate_cmds::{naming_views, side_by_side};
    use photocraft_genai::Rgba8;
    use photocraft_geom::Rect;
    // A 200×160 mask on a 2000×1500 photo: the close-up the 4B sees has a quarter of the grown
    // stroke's side around it, the one the worker's 8B sees a tenth.
    let canvas = Rect::new(0, 0, 2000, 1500);
    let hole = Rect::new(900, 700, 1100, 860);
    let (wide, close) = naming_views(hole, canvas, false).unwrap();
    let (wide_w, close_w) = naming_views(hole, canvas, true).unwrap();
    assert!(close_w.width() < close.width() && close_w.height() < close.height(), "{close_w:?} inside {close:?}");
    for (w, c) in [(wide, close), (wide_w, close_w)] {
        assert!(c.x0 <= hole.x0 && c.y0 <= hole.y0 && c.x1 >= hole.x1 && c.y1 >= hole.y1, "the close-up holds the stroke: {c:?}");
        assert!(w.width() >= 384 && w.height() >= 384 && w.x0 <= c.x0 && w.x1 >= c.x1, "the surroundings hold the close-up: {w:?}");
    }
    // A close-up three times as wide as it is high beside square surroundings keeps its shape.
    let left = Rgba8::new(40, 40, vec![0; 40 * 40 * 4]).unwrap();
    let right = Rgba8::new(90, 30, vec![255; 90 * 30 * 4]).unwrap();
    let seen = side_by_side(&left, &right).unwrap();
    assert_eq!((seen.width, seen.height), (40 + 120, 40), "the right half at 120×40, not squeezed to 40×40");
}

#[test]
fn a_removal_takes_the_shadow_that_went_with_its_object_and_nothing_else() {
    use crate::generate_cmds::{SHADOW_CHANGE, SHADOW_MAX_SHARE, change_band, feather_outward};
    use photocraft_genai::{Gray8, Rgba8};
    let (w, h) = (128u32, 96u32);
    let paint = |f: &dyn Fn(f64, f64) -> u8| -> Rgba8 {
        let mut d = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = f(f64::from(x), f64::from(y));
                d.extend([v, v, v, 255]);
            }
        }
        Rgba8::new(w, h, d).unwrap()
    };
    let disc = |x: f64, y: f64| (x - 40.0).powi(2) + (y - 48.0).powi(2) <= 100.0;
    let shadow = |x: f64, y: f64| ((x - 58.0) / 10.0).powi(2) + ((y - 50.0) / 6.0).powi(2) <= 1.0;
    let far = |x: f64, y: f64| x >= 112.0 && y < 12.0;
    // The photo: a dark disc and its cast shadow on grey; the model's result: the disc and the
    // shadow gone, and (unconnected) a corner it changed for no reason.
    let original = paint(&|x, y| {
        if disc(x, y) {
            20
        } else if shadow(x, y) {
            70
        } else {
            128
        }
    });
    let result = paint(&|x, y| if far(x, y) { 200 } else { 128 });
    let hole = Gray8::new(
        w,
        h,
        (0..w * h)
            .map(|i| if disc(f64::from(i % w), f64::from(i / w)) || (f64::from(i % w) - 40.0).hypot(f64::from(i / w) - 48.0) <= 13.0 { 255 } else { 0 })
            .collect(),
    )
    .unwrap();
    let mask = feather_outward(&hole, 6).unwrap();
    let band = change_band(&result, &original, &mask, SHADOW_MAX_SHARE, SHADOW_CHANGE).expect("the shadow changed with the disc");
    let at = |x: u32, y: u32| band.data[(y * w + x) as usize];
    assert_eq!(at(58, 50), 255, "the shadow's middle is taken");
    assert_eq!(at(66, 50), 255, "and its far end");
    assert_eq!(at(120, 4), 0, "a change not connected to the hole is not");
    assert_eq!(at(40, 48), 0, "the hole itself is the hole's, not the band's");
    // Nothing changed beside the hole, or everything did: no band.
    let clean = paint(&|x, y| if shadow(x, y) { 70 } else { 128 });
    assert!(change_band(&clean, &original, &mask, SHADOW_MAX_SHARE, SHADOW_CHANGE).is_none(), "only the disc went");
    let brighter = paint(&|_, _| 180);
    assert!(change_band(&brighter, &original, &mask, SHADOW_MAX_SHARE, SHADOW_CHANGE).is_none(), "a whole-picture tone shift is not a shadow");
    // A shadow that runs out of the request: not cut off at its edge.
    let long = |x: f64, y: f64| x >= 45.0 && (y - 50.0).abs() <= 5.0;
    let original = paint(&|x, y| {
        if disc(x, y) {
            20
        } else if long(x, y) {
            70
        } else {
            128
        }
    });
    assert!(change_band(&paint(&|_, _| 128), &original, &mask, SHADOW_MAX_SHARE, SHADOW_CHANGE).is_none(), "the shadow goes on past the request");
}

/// An edit's swap: a coat longer than the jacket it replaces runs past the selection; that
/// part is taken for the layer mask within the edit's share, and a change as large as the
/// selection again is not (a second subject, not a hem).
#[test]
fn an_edit_takes_the_garment_that_runs_past_its_selection_within_its_share() {
    use crate::generate_cmds::{EDIT_BAND_CHANGE, EDIT_BAND_MAX_SHARE, change_band, feather_outward};
    use photocraft_genai::{Gray8, Rgba8};
    let (w, h) = (96u32, 128u32);
    let paint = |f: &dyn Fn(u32, u32) -> u8| -> Rgba8 {
        let mut d = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = f(x, y);
                d.extend([v, v, v, 255]);
            }
        }
        Rgba8::new(w, h, d).unwrap()
    };
    let jacket = |x: u32, y: u32| (30..66).contains(&x) && (30..70).contains(&y);
    let original = paint(&|x, y| if jacket(x, y) { 60 } else { 140 });
    // The coat: the jacket's box and 12 rows further down.
    let coat = paint(&|x, y| if (30..66).contains(&x) && (30..82).contains(&y) { 230 } else { 140 });
    let hole = Gray8::new(w, h, (0..w * h).map(|i| if jacket(i % w, i / w) { 255 } else { 0 }).collect()).unwrap();
    let mask = feather_outward(&hole, 4).unwrap();
    let band = change_band(&coat, &original, &mask, EDIT_BAND_MAX_SHARE, EDIT_BAND_CHANGE).expect("the hem below the selection");
    assert_eq!(band.data[(78 * w + 48) as usize], 255, "the coat's hem is taken");
    assert_eq!(band.data[(110 * w + 48) as usize], 0, "below it is not");
    // A change as large again as the selection: refused under the edit's share.
    let big = paint(&|x, y| if (30..66).contains(&x) && (30..118).contains(&y) { 230 } else { 140 });
    assert!(change_band(&big, &original, &mask, EDIT_BAND_MAX_SHARE, EDIT_BAND_CHANGE).is_none());
}

#[test]
fn an_image_from_a_reference_switches_template_and_variations_fill_one_document() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, hat) = session(&fake.url);
    let r = s
        .execute(IMAGE, json!({"prompt": "a portrait wearing the hat from image 1", "target": "document", "width": 96, "height": 64, "variations": 2, "seed": 9, "references": [{"layer": hat.0}]}))
        .unwrap();
    assert_eq!(r["template"], "qwen-edit-2511/image-ref", "the default image template takes no references");
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(96), Some(64)), "the requested size, not the reference's");
    assert_eq!(r["seeds"], json!([9, 10]));
    let d = s.active().unwrap();
    assert_eq!(d.doc.layers.len(), 2);
    assert!(d.doc.layers[0].visible && !d.doc.layers[1].visible, "only the first visible");
    assert_eq!(d.doc.layers[1].name, "Generated (2/2)");
    assert_eq!(r["layers"].as_array().map(Vec::len), Some(2));
    let info = generative_info(&d.doc.layers[0]).unwrap();
    assert_eq!(info.template, "qwen-edit-2511/image-ref");
    assert_eq!(info.references, vec![json!({"layer": hat.0})]);
    let st = fake.state();
    assert_eq!(st.runs.len(), 2);
    let g = &st.runs[0].request;
    assert_eq!(g["references"].as_array().map(Vec::len), Some(1), "only the supplied reference is sent");
    assert_eq!(g["width"], 96);
}

#[test]
fn image_variations_on_a_layer_and_references_from_files_and_generations() {
    let fake = FakeNative::start().unwrap();
    let (mut s, _, hat) = session(&fake.url);
    s.execute("select.deselect", json!({})).unwrap();
    let r = s.execute(IMAGE, json!({"prompt": "a lighthouse", "target": "layer", "variations": 3, "seed": 1})).unwrap();
    let ids: Vec<u64> = r["layers"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
    assert_eq!(ids.len(), 3);
    let d = s.active().unwrap();
    let vis: Vec<bool> = ids.iter().map(|i| d.doc.layer(LayerId(*i)).unwrap().visible).collect();
    assert_eq!(vis, [true, false, false]);
    assert_eq!(d.active_layer, Some(LayerId(ids[0])), "the visible result is active");
    // A file reference and a stored generation (the library in a temp folder).
    let dir = std::env::temp_dir().join(format!("photocraft-refs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    s.set_generations_dir(dir.join("generations"));
    let png = photocraft_genai::png::encode_rgba8(&photocraft_genai::Rgba8::solid(30, 20, [9, 9, 9, 255]).unwrap()).unwrap();
    let file = dir.join("ref.png");
    std::fs::write(&file, png).unwrap();
    s.execute(IMAGE, json!({"prompt": "a boat", "target": "layer", "references": [{"layer": hat.0}]})).unwrap();
    let job = s.generations.records().last().unwrap().id;
    let before = fake.state().pictures.len();
    s.execute(EDIT, json!({"prompt": "x", "references": [{"path": file.to_string_lossy()}, {"job": job}]})).unwrap();
    let up = uploads(&fake);
    assert_eq!(up.len(), before + 3);
    assert_eq!(up[before + 1].1, (30, 20), "the file");
    assert_eq!(up[before + 2].1, (64, 48), "the stored result (the layer image over the canvas)");
    let e = s.execute(EDIT, json!({"prompt": "x", "references": [{"path": dir.join("nope.png").to_string_lossy()}]})).unwrap_err();
    assert!(e.to_string().contains("nope.png"), "{e}");
    s.flush_generations();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_time_estimate_is_one_formula_everywhere() {
    // Lightning-8 fill: 7.5 s + 5.5 s per megapixel, at the size it is sent (≤ 0.75 MP).
    let est = estimate_seconds("qwen-edit-2511/fill-lightning-8", 1000, 1000, 2, 0).unwrap();
    let (w, h) = crate::generate_cmds::request_size(1000, 1000);
    let mp = f64::from(w) * f64::from(h) / 1e6;
    assert!((f64::from(est) - (7.5 + 5.5 * mp) * 2.0).abs() < 0.01, "{est}");
    // References: 9 s each on the Lightning edit.
    let two = estimate_seconds("qwen-edit-2511/edit-lightning-8", 1000, 1000, 1, 2).unwrap();
    assert!((f64::from(two) - (7.5 + 5.5 * mp + 18.0)).abs() < 0.01, "{two}");
    // An image template counts the size asked for.
    assert!((estimate_seconds("krea2-turbo/image", 1000, 1000, 1, 0).unwrap() - 7.0).abs() < 0.01);
    assert!(estimate_seconds("nope/x", 1, 1, 1, 0).is_none());
    // The native Qwen-Image-2.1 edit (Edit's `auto`): three variations of a 1024² photo took
    // 17.7 s in the app on the 5090 (2026-11-03); the ComfyUI-era numbers said about 2 min.
    let edit = estimate_seconds("qwen-2.1/edit", 1024, 1024, 3, 0).unwrap();
    assert!((12.0..30.0).contains(&edit), "{edit}");
    // generate.models reports it per template for an area and count.
    let mut s = Session::new();
    let v = s.execute("generate.models", json!({"probe": false, "width": 1000, "height": 1000, "count": 1})).unwrap();
    let krea = v["templates"].as_array().unwrap().iter().find(|t| t["id"] == "krea2-turbo/image").unwrap().clone();
    assert!((krea["eta"].as_f64().unwrap() - 7.0).abs() < 0.01);
    assert_eq!(krea["pages"], json!(["image"]));
    assert_eq!(krea["creator"], "Krea");
    let edit = v["templates"].as_array().unwrap().iter().find(|t| t["id"] == "qwen-edit-2511/edit-lightning-8").unwrap().clone();
    assert_eq!(edit["references"]["max"], 2);
    let v = s.execute("generate.models", json!({"probe": false})).unwrap();
    assert!(v["templates"][0]["eta"].is_null(), "no area, no estimate");
    assert!(matches!(s.execute("generate.models", json!({"probe": false, "width": 10})), Err(EngineError::BadParams { .. })));
}
