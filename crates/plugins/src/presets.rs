//! Named parameter presets for a plug-in: the file format and its validation.
//!
//! Each plug-in gets one presets file, `plugins/<plug-in id>.json` in the app's settings
//! directory. The host reads and writes it on the plug-in's behalf; the plug-in itself never sees
//! a path and has no file access (see `docs/plugins.md`). This module is pure: it turns bytes into
//! presets and back and checks names, but never touches the file system. The desktop app does the
//! reads and writes (`apps/photocraft/src/plugin_presets.rs`).
//!
//! ```json
//! {"version": 1, "presets": {"Soft": {"amount": 25, "mode": "screen"}}}
//! ```

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::manifest::{Manifest, ParamSpec, clamp_point, normalize_color, valid_id, wrap_angle};

/// The presets file format this build reads and writes.
pub const VERSION: u64 = 1;
/// Longest preset name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Most presets one plug-in keeps.
pub const MAX_PRESETS: usize = 256;
/// Largest presets file the host reads.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Preset name → parameter values (a JSON object, as the plug-in's `params`).
pub type Presets = BTreeMap<String, Map<String, Value>>;

/// What a presets file holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// A version-1 file. Presets that aren't objects or have invalid names are left out.
    Presets(Presets),
    /// A file from another (newer) format version: read nothing and never overwrite it.
    UnknownVersion(Value),
    /// Not a presets file at all (bad JSON, wrong shape).
    Corrupt(String),
}

/// The file name for `plugin_id`'s presets (`<id>.json`, capitals escaped as `%XX`), or `None` if the id could name anything
/// other than one plain file in the plug-in folder. Ids are already `[A-Za-z0-9._-]{1,64}`; this
/// also refuses `.`/`..` sequences and leading dots, and checks for separators again.
pub fn file_name(plugin_id: &str) -> Option<String> {
    let safe = valid_id(plugin_id) && !plugin_id.contains("..") && !plugin_id.starts_with('.') && !plugin_id.contains(['/', '\\', ':', '\0']);
    // Ids are case-sensitive but Windows and macOS file names usually aren't: each capital becomes
    // `%XX` (never part of an id), so `Foo` and `foo` can't share a file. Lowercase ids are unchanged.
    safe.then(|| {
        let mut name = String::with_capacity(plugin_id.len() + 5);
        for b in plugin_id.bytes() {
            if b.is_ascii_uppercase() {
                name.push_str(&format!("%{b:02X}"));
            } else {
                name.push(char::from(b));
            }
        }
        name.push_str(".json");
        name
    })
}

/// A preset name the dialog may store: 1-64 printable characters, no path separators and no `..`
/// (names never become paths, but they are refused anyway so a file stays safe to hand-edit or
/// export), and no leading or trailing spaces.
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=MAX_NAME_CHARS).contains(&n)
        && name.trim() == name
        && !name.contains(['/', '\\'])
        && !name.contains("..")
        && name.chars().all(|c| !c.is_control() && !crate::manifest::hidden_format(c))
        && name.chars().any(crate::manifest::visible)
}

/// Reads a presets file. Never fails: a damaged file is [`Parsed::Corrupt`].
pub fn parse(bytes: &[u8]) -> Parsed {
    let root: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return Parsed::Corrupt(e.to_string()),
    };
    let Some(obj) = root.as_object() else { return Parsed::Corrupt("not a JSON object".into()) };
    match obj.get("version") {
        Some(v) if v.as_u64() == Some(VERSION) => {}
        Some(v) => return Parsed::UnknownVersion(v.clone()),
        None => return Parsed::Corrupt("no `version`".into()),
    }
    let Some(presets) = obj.get("presets").and_then(Value::as_object) else { return Parsed::Corrupt("no `presets` object".into()) };
    let out = presets
        .iter()
        .filter(|(name, _)| valid_name(name))
        .filter_map(|(name, v)| v.as_object().map(|m| (name.clone(), m.clone())))
        .take(MAX_PRESETS)
        .collect();
    Parsed::Presets(out)
}

/// The file text for `presets` (pretty JSON, like `preferences.json`).
pub fn to_json(presets: &Presets) -> String {
    let presets: Map<String, Value> = presets.iter().map(|(k, v)| (k.clone(), Value::Object(v.clone()))).collect();
    serde_json::to_string_pretty(&json!({"version": VERSION, "presets": presets})).unwrap_or_default()
}

impl Manifest {
    /// The values of a stored preset that still fit this manifest: unknown keys and values of the
    /// wrong type are dropped, numbers and points are clamped into range, and choices the
    /// plug-in no longer offers and colours that aren't `"#rrggbb"` are dropped; angles are wrapped
    /// into 0-360 and non-numbers dropped. `None` when nothing usable is left (the preset no
    /// longer matches the plug-in). Missing parameters keep their defaults when the result goes
    /// through [`Manifest::resolve_params`].
    pub fn preset_values(&self, stored: &Map<String, Value>) -> Option<Map<String, Value>> {
        let mut out = Map::new();
        for (k, spec) in &self.params {
            let Some(v) = stored.get(k) else { continue };
            let finite = v.as_f64().filter(|f| f.is_finite());
            let kept = match spec {
                ParamSpec::Number { min, max, .. } => finite.map(|f| json!(f.clamp(*min, *max))),
                ParamSpec::Int { min, max, .. } => finite.map(|f| json!(f.round().clamp(*min as f64, *max as f64) as i64)),
                ParamSpec::Bool { .. } => v.as_bool().map(Value::Bool),
                ParamSpec::Choice { options, .. } => v.as_str().filter(|s| options.iter().any(|o| o == s)).map(|s| json!(s)),
                ParamSpec::Point { .. } => match v.as_array().map(Vec::as_slice) {
                    Some([x, y]) => x.as_f64().zip(y.as_f64()).and_then(|(x, y)| clamp_point(x, y)).map(|p| json!(p)),
                    _ => None,
                },
                ParamSpec::Color { .. } => v.as_str().and_then(normalize_color).map(Value::String),
                ParamSpec::Angle { .. } => finite.and_then(wrap_angle).map(|a| json!(a)),
            };
            if let Some(kept) = kept {
                out.insert(k.clone(), kept);
            }
        }
        (!out.is_empty()).then_some(out)
    }

    /// Every preset in `presets` that still fits this manifest ([`Manifest::preset_values`]);
    /// presets that no longer match are skipped, the rest still load.
    pub fn valid_presets(&self, presets: &Presets) -> Presets {
        presets.iter().filter_map(|(name, values)| self.preset_values(values).map(|v| (name.clone(), v))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest::parse(
            br##"{"id":"org.example.t","name":"T","kind":"filter","params":{
                "amount":{"type":"number","min":0,"max":100,"default":50},
                "steps":{"type":"int","min":1,"max":8,"default":2},
                "mono":{"type":"bool"},
                "mode":{"type":"choice","options":["a","b"]},
                "at":{"type":"point","default":[50,50]},
                "inner":{"type":"color","default":"#cfeaff"},
                "direction":{"type":"angle","default":45}}}"##,
        )
        .unwrap()
    }

    #[test]
    fn file_names_come_only_from_safe_ids() {
        assert_eq!(file_name("com.stack-tech.freecandy7").as_deref(), Some("com.stack-tech.freecandy7.json"));
        assert_eq!(file_name("a_b-1").as_deref(), Some("a_b-1.json"));
        // Ids differing only in case get different files on case-insensitive file systems.
        assert_eq!(file_name("org.example.Foo").as_deref(), Some("org.example.%46oo.json"));
        assert_ne!(file_name("org.example.Foo").map(|n| n.to_lowercase()), file_name("org.example.foo").map(|n| n.to_lowercase()));
        for bad in ["", ".", "..", "a..b", "../x", "a/b", "a\\b", ".hidden", "a:b", "a\0b", "é", &"x".repeat(65)] {
            assert_eq!(file_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn preset_names_refuse_separators_and_dot_dot() {
        assert!(valid_name("Soft glow"));
        assert!(valid_name("Été 2"));
        for bad in ["", " lead", "trail ", "a/b", "a\\b", "..", "x..y", "../etc", "tab\there", "bidi\u{202E}", "\u{3164}", "\u{FE0F}", &"n".repeat(65)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn round_trip() {
        let mut p = Presets::new();
        p.insert("Soft".into(), json!({"amount": 25.0, "mode": "b"}).as_object().unwrap().clone());
        p.insert("Hard".into(), json!({"amount": 90.0, "mono": true, "at": [10.0, 20.0], "inner": "#ff8800"}).as_object().unwrap().clone());
        let text = to_json(&p);
        assert!(text.contains("\"version\": 1"), "{text}");
        assert_eq!(parse(text.as_bytes()), Parsed::Presets(p));
    }

    #[test]
    fn corrupt_and_unknown_versions_load_nothing() {
        for bad in [&b""[..], b"{", b"[]", b"{\"presets\":{}}", b"{\"version\":1}", b"{\"version\":1,\"presets\":[]}"] {
            assert!(matches!(parse(bad), Parsed::Corrupt(_)), "{:?}", String::from_utf8_lossy(bad));
        }
        assert_eq!(parse(br#"{"version":2,"presets":{"x":{"amount":1}}}"#), Parsed::UnknownVersion(json!(2)));
        assert_eq!(parse(br#"{"version":"1","presets":{}}"#), Parsed::UnknownVersion(json!("1")));
    }

    #[test]
    fn bad_entries_are_skipped_and_the_rest_load() {
        let Parsed::Presets(p) = parse(br#"{"version":1,"presets":{"ok":{"amount":1},"a/b":{"amount":2},"num":5}}"#) else { panic!() };
        assert_eq!(p.keys().collect::<Vec<_>>(), ["ok"]);
    }

    #[test]
    fn stored_values_are_checked_against_the_manifest() {
        let m = manifest();
        let stored = json!({
            "amount": 250, "steps": 3.6, "mono": "yes", "mode": "gone", "at": [-5, 140], "removed": 1, "id": "other"
        });
        let v = m.preset_values(stored.as_object().unwrap()).unwrap();
        assert_eq!(Value::Object(v), json!({"amount": 100.0, "steps": 4, "at": [0.0, 100.0]}));
        // Nothing usable left: the preset no longer matches.
        assert_eq!(m.preset_values(json!({"mode": "gone", "old": 3}).as_object().unwrap()), None);
        let mut all = Presets::new();
        all.insert("stale".into(), json!({"old": 1}).as_object().unwrap().clone());
        all.insert("fine".into(), json!({"mode": "b", "amount": f64::NAN}).as_object().unwrap().clone());
        let kept = m.valid_presets(&all);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["fine"]);
        assert_eq!(Value::Object(kept["fine"].clone()), json!({"mode": "b"}));
        // Applied values fill the rest from the defaults.
        let full = m.resolve_params(&Value::Object(kept["fine"].clone())).unwrap();
        assert_eq!(full["amount"], json!(50.0));
    }

    #[test]
    fn colours_round_trip_and_invalid_ones_are_dropped() {
        let m = manifest();
        let mut p = Presets::new();
        p.insert("Warm".into(), json!({"inner": "#ff8800", "amount": 10.0}).as_object().unwrap().clone());
        let Parsed::Presets(back) = parse(to_json(&p).as_bytes()) else { panic!() };
        assert_eq!(back, p);
        assert_eq!(Value::Object(m.preset_values(&back["Warm"]).unwrap()), json!({"inner": "#ff8800", "amount": 10.0}));
        assert_eq!(Value::Object(m.preset_values(json!({"inner": "#FF8800"}).as_object().unwrap()).unwrap()), json!({"inner": "#ff8800"}));
        for bad in [json!("ff8800"), json!("#ff880"), json!("#ff8800aa"), json!("#xyz123"), json!(0xff8800), json!([255, 136, 0]), json!(null)] {
            let stored = json!({"inner": bad, "amount": 10.0});
            assert_eq!(Value::Object(m.preset_values(stored.as_object().unwrap()).unwrap()), json!({"amount": 10.0}), "{bad}");
            assert_eq!(m.preset_values(json!({"inner": bad}).as_object().unwrap()), None, "{bad}");
        }
        // The kept colour fills in through the defaults.
        let full = m.resolve_params(&json!({})).unwrap();
        assert_eq!(full["inner"], json!("#cfeaff"));
    }

    #[test]
    fn angles_round_trip_wrap_and_invalid_ones_are_dropped() {
        let m = manifest();
        let mut p = Presets::new();
        p.insert("Steep".into(), json!({"direction": 137.5, "amount": 10.0}).as_object().unwrap().clone());
        let Parsed::Presets(back) = parse(to_json(&p).as_bytes()) else { panic!() };
        assert_eq!(back, p);
        assert_eq!(Value::Object(m.preset_values(&back["Steep"]).unwrap()), json!({"direction": 137.5, "amount": 10.0}));
        for (stored, want) in [(json!(370), 10.0), (json!(-90), 270.0), (json!(720), 0.0), (json!(360), 360.0)] {
            assert_eq!(Value::Object(m.preset_values(json!({"direction": stored}).as_object().unwrap()).unwrap()), json!({"direction": want}), "{stored}");
        }
        for bad in [json!("90"), json!([90]), json!(true), json!(null), json!({"deg": 90})] {
            let stored = json!({"direction": bad, "amount": 10.0});
            assert_eq!(Value::Object(m.preset_values(stored.as_object().unwrap()).unwrap()), json!({"amount": 10.0}), "{bad}");
            assert_eq!(m.preset_values(json!({"direction": bad}).as_object().unwrap()), None, "{bad}");
        }
        assert_eq!(m.resolve_params(&json!({})).unwrap()["direction"], json!(45.0));
    }
}
