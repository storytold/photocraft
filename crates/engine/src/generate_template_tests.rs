//! Template selection: research-licence templates (never refused; the Qwen-Image-2.1 ones as the
//! examples) and the default-template preferences.

use photocraft_genai::fake::FakeNative;
use serde_json::json;

use super::*;

fn session(url: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 16, "width": 32, "height": 32})).unwrap();
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

#[test]
fn research_templates_run_whatever_the_preference_says() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    // The preference off only steers Auto: a research template asked for by name runs, and the
    // listing allows every template, the licence a note.
    let r = s.execute(IMAGE, json!({"prompt": "x", "template": "qwen-2.1/image"})).unwrap();
    assert_eq!(r["template"], "qwen-2.1/image");
    let m = s.execute(MODELS, json!({})).unwrap();
    for t in m["templates"].as_array().unwrap() {
        let research = t["license"] == "research";
        assert_eq!(t["allowed"], true, "{}", t["id"]);
        assert!(!research || t["licenseNote"].as_str().unwrap().contains("Qwen Research License"), "{}", t["id"]);
    }
    let r = s.execute(FILL, json!({"prompt": "a red bicycle", "template": "qwen-2.1/fill", "seed": 1})).unwrap();
    assert_eq!(r["template"], "qwen-2.1/fill");
    let st = fake.state();
    let g = &st.runs[1].request;
    let prompt = g["prompt"].as_str().unwrap();
    assert!(prompt.contains("a red bicycle") && prompt.contains("image 2"), "wrapped as a local-edit instruction: {prompt}");
    assert_eq!(g["steps"], 25, "2.1 default");
    assert_eq!(g["guidance"], 1.0);
    assert_eq!(g["params"]["models"]["unet"], "qwen_image_2.1_bf16.safetensors");
    assert_eq!(g["params"]["models"]["vae"], "qwen_image_2.1_vae_bf16.safetensors");
}

#[test]
fn the_default_template_preferences_pick_the_template() {
    let fake = FakeNative::start().unwrap();
    let mut s = session(&fake.url);
    s.edit_prefs(|p| {
        p.integrations.allow_research_models = true;
        p.integrations.default_fill_template = "qwen-2.1/fill".into();
        p.integrations.default_image_template = "qwen-2.1/image".into();
    });
    let r = s.execute(FILL, json!({"prompt": "x"})).unwrap();
    assert_eq!(r["template"], "qwen-2.1/fill");
    let r = s.execute(IMAGE, json!({"prompt": "x"})).unwrap();
    assert_eq!(r["template"], "qwen-2.1/image");
    let st = fake.state();
    drop(st);

    // An explicit param still wins, and an empty preference means `auto` (the Lightning tier,
    // whose LoRA the fake lists).
    let r = s.execute(FILL, json!({"prompt": "x", "template": "qwen-edit-2511/fill"})).unwrap();
    assert_eq!(r["template"], "qwen-edit-2511/fill");
    s.edit_prefs(|p| p.integrations.default_fill_template = String::new());
    let r = s.execute(FILL, json!({"prompt": "x"})).unwrap();
    assert_eq!(r["template"], "qwen-2.1/fill", "native auto uses 2.1 when research is allowed");

    // A preference naming an unknown or wrong-task template is a clear error, not a panic.
    s.edit_prefs(|p| p.integrations.default_fill_template = "nope/fill".into());
    let e = s.execute(FILL, json!({"prompt": "x"})).unwrap_err();
    assert!(matches!(e, EngineError::BadParams { .. }) && e.to_string().contains("nope/fill"), "{e}");
    s.edit_prefs(|p| p.integrations.default_fill_template = "krea2-turbo/image".into());
    assert!(matches!(s.execute(FILL, json!({"prompt": "x"})), Err(EngineError::BadParams { .. })));
    // A research template named by the preference runs with the research preference off too.
    s.edit_prefs(|p| {
        p.integrations.default_fill_template = "qwen-2.1/fill".into();
        p.integrations.allow_research_models = false;
    });
    assert_eq!(s.execute(FILL, json!({"prompt": "x"})).unwrap()["template"], "qwen-2.1/fill");
}

#[test]
fn template_preferences_are_validated_when_set() {
    let mut p = crate::prefs::Preferences::default();
    assert!(p.set("integrations.defaultFillTemplate", json!("qwen-2.1/fill")).is_ok());
    assert!(p.set("integrations.defaultFillTemplate", json!("../x")).is_err());
    assert!(p.set("integrations.defaultImageTemplate", json!("Has Spaces")).is_err());
    assert!(p.set("integrations.generativeTimeoutSecs", json!(1)).is_err());
    assert!(p.set("integrations.defaultEditModel", json!("a/b.safetensors")).is_err());
    assert_eq!(p.integrations.default_fill_template, "qwen-2.1/fill");
}

/// Krea 2 AnyPaint: its template is a Krea 2 fill that takes `denoise`, and its prompt writer is
/// shown the crop with the selection flattened to mid-grey (AusBoss's "Write the prompt").
#[test]
fn the_krea2_fill_takes_denoise_and_its_writer_sees_a_grey_hole() {
    assert!(krea2_fill("krea2-turbo/fill") && !krea2_fill("qwen-2.1/fill") && !krea2_fill("krea2-turbo/edit"));
    assert!(takes_denoise("krea2-turbo/fill"));
    let image = Rgba8 { width: 2, height: 1, data: vec![10, 20, 30, 255, 40, 50, 60, 255] };
    let mask = Gray8 { width: 2, height: 1, data: vec![200, 50] };
    assert_eq!(grey_where(&image, &mask).data, vec![128, 128, 128, 255, 40, 50, 60, 255]);
}
