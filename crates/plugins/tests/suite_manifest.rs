//! Effect suites need room for a selector and several controls per effect.
use photocraft_plugins::Manifest;
use serde_json::{Map, json};

#[test]
fn effect_suites_accept_256_parameters_but_reject_257() {
    let mut params = Map::new();
    for i in 0..256 {
        params.insert(format!("p{i}"), json!({"type":"number","min":0,"max":100,"default":50}));
    }
    let mut value = json!({"id":"org.example.suite","name":"Suite","kind":"filter","params":params});
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() < 64 * 1024);
    let manifest = Manifest::parse(&bytes).unwrap();
    assert_eq!(manifest.params.len(), 256);
    assert_eq!(manifest.resolve_params(&json!({})).unwrap().len(), 256);
    value["params"]["overflow"] = json!({"type":"bool"});
    let error = Manifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(error.to_string().contains("too many parameters"));
}

#[test]
fn conditional_dialog_metadata_keeps_param_order_and_legacy_groups() {
    let manifest = Manifest::parse(
        br#"{"id":"org.example.suite","name":"Suite","kind":"filter",
            "presetSelector":"effect","params":{
                "effect":{"type":"choice","options":["glow","blur"],"label":"Effect"},
                "radius":{"type":"number","min":0,"max":32,"default":8,"group":"Glow",
                    "label":"Radius","visibleWhen":{"effect":"glow","enabled":true}},
                "enabled":{"type":"bool","default":true},
                "blur":{"type":"number","min":0,"max":64,"default":4,"group":"Blur",
                    "visibleWhen":{"effect":"blur"}}
            }}"#,
    )
    .unwrap();

    assert_eq!(manifest.preset_selector.as_deref(), Some("effect"));
    assert_eq!(manifest.label("effect"), Some("Effect"));
    assert_eq!(manifest.label("radius"), Some("Radius"));
    assert_eq!(manifest.label("enabled"), None);
    assert_eq!(manifest.visible_when("radius").unwrap()["effect"], json!("glow"));
    assert_eq!(manifest.visible_when("radius").unwrap()["enabled"], json!(true));
    assert_eq!(manifest.visible_when("blur").unwrap()["effect"], json!("blur"));
    assert_eq!(manifest.visible_when("effect"), None);
    assert_eq!(manifest.params.iter().map(|(key, _)| key.as_str()).collect::<Vec<_>>(), ["effect", "radius", "enabled", "blur"]);
    assert_eq!(manifest.tabs(), ["Glow", "Blur"]);
    assert_eq!(manifest.group("radius"), Some("Glow"));
}

#[test]
fn old_manifests_keep_group_behavior_without_conditional_metadata() {
    let manifest = Manifest::parse(
        br#"{"id":"org.example.old","name":"Old","kind":"filter","params":{
            "amount":{"type":"number","min":0,"max":1,"default":0.5,"group":"Main"},
            "mode":{"type":"choice","options":["a","b"]}
        }}"#,
    )
    .unwrap();

    assert_eq!(manifest.preset_selector, None);
    assert_eq!(manifest.label("amount"), None);
    assert_eq!(manifest.visible_when("amount"), None);
    assert_eq!(manifest.group("amount"), Some("Main"));
    assert_eq!(manifest.tabs(), ["Main"]);
    assert_eq!(manifest.always_visible(), ["mode"]);
    assert!(manifest.preset_preserve.is_empty());
}

#[test]
fn builtin_preset_preserve_accepts_known_unique_keys_and_explicit_overrides() {
    let mut value = suite(json!({
        "effect":{"type":"choice","options":["fur","fire"]},
        "coat":{"type":"choice","options":["plain","tiger"]},
        "length":{"type":"number","min":1,"max":60,"default":12}
    }));
    value["presetSelector"] = json!("effect");
    value["presetPreserve"] = json!(["coat"]);
    value["presets"] = json!({"Long":{"effect":"fur","length":40},"Tiger":{"effect":"fur","coat":"tiger"}});
    let manifest = parse(&value).unwrap();
    assert_eq!(manifest.preset_preserve, ["coat"]);
    assert!(!manifest.presets["Long"].contains_key("coat"));
    assert_eq!(manifest.presets["Tiger"]["coat"], json!("tiger"));
    let encoded = serde_json::to_value(&manifest).unwrap();
    assert_eq!(encoded["presetPreserve"], json!(["coat"]));
    for keys in [json!(["missing"]), json!(["coat", "coat"]), json!(["effect"]), json!("coat"), json!([null])] {
        value["presetPreserve"] = keys.clone();
        assert!(parse(&value).is_err(), "invalid preserve keys: {keys}");
    }
}

fn suite(params: serde_json::Value) -> serde_json::Value {
    json!({"id":"org.example.suite","name":"Suite","kind":"filter","params":params})
}

fn parse(value: &serde_json::Value) -> photocraft_plugins::Result<Manifest> {
    Manifest::parse(&serde_json::to_vec(value).unwrap())
}

#[test]
fn conditional_metadata_rejects_invalid_references_and_values() {
    let cases = [
        ("missing reference", json!({"missing":"x"}), "references missing parameter"),
        ("numeric target", json!({"amount":1}), "must be a choice or boolean"),
        ("choice value", json!({"effect":"other"}), "must match one of its options"),
        ("choice type", json!({"effect":true}), "must match one of its options"),
        ("bool value", json!({"enabled":"yes"}), "must be true or false"),
        ("bool number", json!({"enabled":1}), "must be true or false"),
        ("self reference", json!({"mode":"a"}), "cannot reference itself"),
        ("empty condition", json!({}), "must contain 1-8 predicates"),
    ];
    for (name, conditions, message) in cases {
        let value = suite(json!({
            "effect":{"type":"choice","options":["a","b"]},
            "enabled":{"type":"bool"},
            "amount":{"type":"number","min":0,"max":1,"default":0.5},
            "mode":{"type":"choice","options":["a","b"],"visibleWhen":conditions}
        }));
        let error = parse(&value).unwrap_err().to_string();
        assert!(error.contains(message), "{name}: {error}");
    }
}

#[test]
fn conditional_controllers_require_every_parent_predicate_with_the_same_value() {
    let valid = suite(json!({
        "effect":{"type":"choice","options":["swirl","glow"]},
        "enabled":{"type":"bool","default":true},
        "swirl_style":{"type":"choice","options":["distort","rays"],"visibleWhen":{"effect":"swirl","enabled":true}},
        "amount":{"type":"number","min":0,"max":1,"default":0.5,
            "visibleWhen":{"effect":"swirl","enabled":true,"swirl_style":"distort"}}
    }));
    let manifest = parse(&valid).unwrap();
    assert_eq!(manifest.visible_when("amount").unwrap().len(), 3);
    // Hidden controls retain their values in the plug-in's complete parameter object.
    assert_eq!(manifest.resolve_params(&json!({"effect":"glow","amount":0.75})).unwrap()["amount"], json!(0.75));
    for conditions in [
        json!({"swirl_style":"distort"}),
        json!({"effect":"swirl","swirl_style":"distort"}),
        json!({"effect":"glow","enabled":true,"swirl_style":"distort"}),
        json!({"effect":"swirl","enabled":false,"swirl_style":"distort"}),
    ] {
        let mut value = valid.clone();
        value["params"]["amount"]["visibleWhen"] = conditions;
        let error = parse(&value).unwrap_err().to_string();
        assert!(error.contains("must include all predicates"), "{error}");
    }
}

#[test]
fn visibility_supports_multiple_nested_controllers() {
    let manifest = parse(&suite(json!({
        "effect":{"type":"choice","options":["swirl","glow"]},
        "style":{"type":"choice","options":["distort","rays"],"visibleWhen":{"effect":"swirl"}},
        "detail":{"type":"bool","visibleWhen":{"effect":"swirl","style":"distort"}},
        "strength":{"type":"number","min":0,"max":10,"default":1,
            "visibleWhen":{"effect":"swirl","style":"distort","detail":true}}
    })))
    .unwrap();
    assert_eq!(manifest.visible_when("strength").unwrap().len(), 3);
}

#[test]
fn visibility_rejects_cycles() {
    for params in [
        json!({
            "a":{"type":"bool","visibleWhen":{"b":true}},
            "b":{"type":"bool","visibleWhen":{"a":true}}
        }),
        json!({
            "a":{"type":"bool","visibleWhen":{"b":true}},
            "b":{"type":"bool","visibleWhen":{"c":true}},
            "c":{"type":"bool","visibleWhen":{"a":true}}
        }),
    ] {
        assert!(parse(&suite(params)).is_err());
    }
}

#[test]
fn visibility_accepts_eight_predicates_and_rejects_nine() {
    let mut params = Map::new();
    let mut conditions = Map::new();
    for i in 0..8 {
        let key = format!("flag{i}");
        params.insert(key.clone(), json!({"type":"bool"}));
        conditions.insert(key, json!(true));
    }
    params.insert("amount".into(), json!({"type":"number","min":0,"max":1,"default":0.5,"visibleWhen":conditions}));
    let mut value = suite(json!(params));
    assert_eq!(parse(&value).unwrap().visible_when("amount").unwrap().len(), 8);
    value["params"]["flag8"] = json!({"type":"bool"});
    value["params"]["amount"]["visibleWhen"]["flag8"] = json!(true);
    assert!(parse(&value).unwrap_err().to_string().contains("must contain 1-8 predicates"));
}

#[test]
fn preset_selector_must_name_an_unconditional_choice() {
    for (selector, params, message) in [
        ("missing", json!({"mode":{"type":"choice","options":["a"]}}), "references missing parameter"),
        ("amount", json!({"amount":{"type":"number","min":0,"max":1,"default":0.5}}), "must name an unconditional choice"),
        ("flag", json!({"flag":{"type":"bool"}}), "must name an unconditional choice"),
        ("mode", json!({"mode":{"type":"choice","options":["a"],"visibleWhen":{"flag":true}},"flag":{"type":"bool"}}), "must name an unconditional choice"),
    ] {
        let mut value = suite(params);
        value["presetSelector"] = json!(selector);
        let error = parse(&value).unwrap_err().to_string();
        assert!(error.contains(message), "{error}");
    }
}

#[test]
fn labels_are_printable_and_limited_to_64_characters() {
    for label in ["", "   ", "Bad\nlabel", "Label\n", "\tLabel", "Hidden\u{202e}label", "\u{200d}", "\u{fe0f}"] {
        let error = parse(&suite(json!({"mode":{"type":"choice","options":["a"],"label":label}}))).unwrap_err().to_string();
        assert!(error.contains("`label` must be 1-64 printable characters"), "{label:?}: {error}");
    }
    let too_long = "x".repeat(65);
    assert!(parse(&suite(json!({"mode":{"type":"choice","options":["a"],"label":too_long}}))).is_err());
    let label = "界".repeat(64);
    let manifest = parse(&suite(json!({"mode":{"type":"choice","options":["a"],"label":label}}))).unwrap();
    assert_eq!(manifest.label("mode"), Some(label.as_str()));
    assert_eq!(manifest.label("missing"), None);
    assert_eq!(manifest.visible_when("missing"), None);
}

#[test]
fn metadata_rejects_wrong_json_types() {
    for metadata in [json!({"label":123}), json!({"visibleWhen":true}), json!({"visibleWhen":[]})] {
        let mut spec = json!({"type":"bool"});
        spec.as_object_mut().unwrap().extend(metadata.as_object().unwrap().clone());
        assert!(parse(&suite(json!({"flag":spec}))).is_err());
    }
    let mut value = suite(json!({"flag":{"type":"bool"}}));
    value["presetSelector"] = json!(123);
    assert!(parse(&value).is_err());
}
