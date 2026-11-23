use serde_json::json;

use super::*;

fn specs() -> &'static [crate::commands::CommandSpec] {
    crate::install().unwrap();
    crate::commands::command_specs()
}

fn registered(id: &str) -> &'static crate::commands::CommandSpec {
    crate::install().unwrap();
    crate::commands::find(id).expect("registered")
}

fn parse(doc: &str) -> Params {
    Params::parse(doc).unwrap_or_else(|| panic!("{doc}"))
}

fn keys(doc: &str) -> Vec<String> {
    parse(doc).keys().into_iter().map(str::to_string).collect()
}

/// The registry docs written in another notation (unquoted keys): not checked for unknown keys
/// until rewritten in the grammar. The list may only shrink.
const UNPARSED: &[&str] = &[
    "edit.presets.migratePresets",
    "file.export.dataSetsAsFiles",
    "file.import.variableDataSets",
    "image.applyDataSet",
    "image.trap",
    "image.variables.dataSets",
    "image.variables.define",
    "timeline.create",
    "timeline.setFrame",
    "timeline.setProps",
];

#[test]
fn every_registry_doc_parses_and_renders_back_byte_for_byte() {
    let mut unparsed = Vec::new();
    for c in specs() {
        match Params::parse(c.params) {
            Some(p) => assert_eq!(p.render(), c.params, "{}: the schema must reproduce the doc", c.id),
            None => unparsed.push(c.id),
        }
    }
    unparsed.sort_unstable();
    unparsed.dedup();
    assert_eq!(unparsed, UNPARSED);
}

#[test]
fn no_registry_range_is_left_as_prose() {
    // A bound the parser didn't read would be prose a reader can't check (#7598).
    fn walk(id: &str, o: &Object) {
        for p in o.params() {
            for k in &p.kinds {
                match k {
                    Kind::Object(inner) => walk(id, inner),
                    Kind::Free(t) => {
                        let b = t.as_bytes();
                        let range = (1..b.len().saturating_sub(2)).any(|i| b[i - 1].is_ascii_digit() && &b[i..i + 2] == b".." && b[i + 2].is_ascii_digit());
                        assert!(!range || t.starts_with('{'), "{id}.{}: range in prose {t:?}", p.key());
                    }
                    _ => {}
                }
            }
        }
    }
    for c in specs() {
        if let Some(p) = Params::parse(c.params) {
            for (_, o) in &p.objects {
                walk(c.id, o);
            }
        }
    }
}

#[test]
fn ranges_defaults_units_and_optional_markers() {
    let p = parse(r#"{"radius":0.1..1000=1,"size":0.5..5000 px?,"scale":1..1000 %=100,"tracking":-1000..,"gain":1e-3..2e2}"#);
    let radius = p.param("radius").unwrap();
    assert_eq!(radius.range(), Some((0.1, 1000.0)));
    assert_eq!(radius.default(), Some(&DefaultValue::Num(1.0)));
    // #7598: a unit after the bound keeps the bound.
    let size = p.param("size").unwrap();
    assert_eq!((size.range(), size.unit(), size.optional()), (Some((0.5, 5000.0)), Some("px"), true));
    assert_eq!(p.param("scale").unwrap().unit(), Some("%"));
    assert_eq!(p.param("tracking").unwrap().range(), Some((-1000.0, f64::INFINITY)));
    assert_eq!(p.param("gain").unwrap().range(), Some((0.001, 200.0)));
}

#[test]
fn type_words_alternatives_and_choices_keep_every_member() {
    let p = parse(
        r#"{"name":str,"path":string?,"x":number=0.5,"seed":u32=0,"kernel":int[25],"target":"pixels"|"mask"|{"channel":i}=Channels panel target,"depth":8|16|32=8 (bitmap: 1),"q":"draft|good|best"="good","mode":blend mode name="normal" (not Pass Through)}"#,
    );
    let kinds = |k: &str| p.param(k).unwrap().kinds.clone();
    assert!(matches!(kinds("name")[..], [Kind::Type { ty: Ty::Text, .. }]));
    assert!(matches!(kinds("path")[..], [Kind::Type { ty: Ty::Text, .. }]));
    assert!(matches!(kinds("x")[..], [Kind::Type { ty: Ty::Number, .. }]));
    assert!(matches!(kinds("seed")[..], [Kind::Type { ty: Ty::U32, .. }]));
    assert!(matches!(kinds("kernel")[..], [Kind::Sized { ty: Ty::Int, len: 25, .. }]));
    // #7598: every alternative is kept, not just the first.
    let target = kinds("target");
    assert_eq!(target.len(), 3, "{target:?}");
    assert!(matches!(&target[2], Kind::Object(o) if o.params().next().is_some_and(|c| c.key() == "channel")));
    assert_eq!(p.param("target").unwrap().default(), Some(&DefaultValue::Symbol("Channels panel target".into())));
    assert_eq!(kinds("depth").len(), 3);
    assert_eq!(p.param("depth").unwrap().suffixes.last(), Some(&Suffix::Note(" (bitmap: 1)".into())));
    assert_eq!(kinds("q"), [Kind::Choice(vec!["draft".into(), "good".into(), "best".into()])]);
    assert_eq!(p.param("q").unwrap().default(), Some(&DefaultValue::Str("good".into())));
    assert!(matches!(&kinds("mode")[..], [Kind::Free(t)] if t == "blend mode name"));
}

#[test]
fn input_keys_are_the_top_level_ones_of_the_input_objects() {
    // Nested keys, `→ {result}` keys and keys in trailing prose are not input parameters.
    assert_eq!(keys(r#"{"a":{"b":0..1},"c":bool} (per channel {"hue","saturation"}) → {"d":id}"#), ["a", "c"]);
    // A composed object (` + {…}`) adds its keys.
    assert_eq!(keys(r#"{"x":bool} + {"opacity":0..1=1,"name":str?}"#), ["x", "opacity", "name"]);
    // `"all","pixels":bool`, `"quad"|"corners":…`, `"key"?:…`, a bare `"key"?` and ` or "key":` alternatives.
    assert_eq!(
        keys(r#"{"locks":{"all","pixels":bool}?,"all","pixels":bool,"quad"|"corners":json?,"historyName"?:text,"stops"?,"layer":id? or "layers":[id,…]?}"#),
        ["locks", "all", "pixels", "quad", "corners", "historyName", "stops", "layer", "layers"]
    );
    // A nested `…` leaves that object open, not the command's keys.
    let p = parse(r#"{"curve":[[in,out],…],"opts":{"a":bool,…}}"#);
    assert!(!p.open());
    assert!(parse(r#"{"layer":id?, …params of that kind}"#).open());
    assert!(parse("{…}").open());
    // A key family.
    let exr = parse(r#"{"part/<name>":bool (import this part)}"#);
    assert!(exr.has_key("part/beauty") && !exr.has_key("part/") && !exr.has_key("parts"));
}

#[test]
fn values_are_checked_against_what_the_doc_states() {
    let p = parse(r#"{"r":0.1..10=1,"c":"a|b","open":"normal|multiply|…","b":bool,"s":u32=0,"t":str?,"j":json,"alt":"x"|{"channel":i}|null?}"#);
    let ok = |k: &str, v: serde_json::Value| p.param(k).unwrap().accepts(&v);
    assert!(ok("r", json!(0.1)) && ok("r", json!(10)) && !ok("r", json!(10.01)) && !ok("r", json!("5")) && !ok("r", json!(null)));
    assert!(ok("c", json!("b")) && !ok("c", json!("z")) && ok("open", json!("screen")));
    assert!(ok("b", json!(false)) && !ok("b", json!(0)));
    assert!(ok("s", json!(u32::MAX)) && !ok("s", json!(-1)) && !ok("s", json!(0.5)));
    assert!(ok("t", json!("x")) && ok("t", json!(null)) && !ok("t", json!(1)));
    assert!(ok("j", json!({"any": [1]})));
    assert!(ok("alt", json!("x")) && ok("alt", json!({"channel": 2})) && ok("alt", json!(null)) && !ok("alt", json!("y")));
    assert_eq!(p.param("r").unwrap().expected(), "a number in 0.1..10");
}

#[test]
fn universal_keys_are_the_ones_every_command_honours() {
    // `coalesce` is a string key; `layer` and `channel` are not universal (#7599 review).
    assert_eq!(UNIVERSAL, ["coalesce", "target"]);
}

#[test]
fn the_registry_schema_is_read_once_and_shared() {
    let spec = registered("filter.blur.gaussianBlur");
    let a = of(spec).unwrap();
    assert!(std::ptr::eq(a, of(spec).unwrap()));
    assert_eq!(a.param("radius").unwrap().range(), Some((0.1, 1000.0)));
}

/// `layer.newAdjustmentLayer.*` docs put ` + {path, opacity, …}` after the adjustment object and
/// before any trailing notes, so composed keys stay documented (#15255).
#[test]
fn new_adjustment_layer_docs_keep_composed_options_in_front_of_notes() {
    let spec = registered("layer.newAdjustmentLayer.gradientMap");
    let schema = parse(spec.params);
    assert!(schema.has_key("opacity"), "{}", spec.params);
    assert!(schema.has_key("name"), "{}", spec.params);
    assert!(schema.has_key("clipToPrevious"), "{}", spec.params);
    assert!(schema.has_key("stops"), "{}", spec.params);
    let plus = spec.params.find(" + {").expect("composed object");
    let notes = spec.params.find("(stops").expect("gradientMap notes");
    assert!(plus < notes, "composition must precede notes: {}", spec.params);
}

/// `gradient.presets.stop` documents `stops` and `transparency` as real keys, not as prose after
/// `opacity`, so `action: "set"` is accepted and no range is left as prose (#15255).
#[test]
fn gradient_presets_stop_documents_the_set_keys() {
    let spec = registered("gradient.presets.stop");
    let schema = parse(spec.params);
    assert!(schema.has_key("stops"), "{}", spec.params);
    assert!(schema.has_key("transparency"), "{}", spec.params);
    assert!(schema.has_key("opacity"), "{}", spec.params);
}

/// `shape.presets.importCsh` documents `path` and `data` as alternative keys, so a base64
/// `data` payload is accepted and not rejected as unknown (#15255).
#[test]
fn import_csh_documents_path_and_data() {
    let spec = registered("shape.presets.importCsh");
    let schema = parse(spec.params);
    assert!(schema.has_key("path"), "{}", spec.params);
    assert!(schema.has_key("data"), "{}", spec.params);
    assert!(schema.has_key("group"), "{}", spec.params);
}
