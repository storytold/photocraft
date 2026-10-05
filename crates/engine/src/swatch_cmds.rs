//! Saved colour swatches: Color Picker › Add to Swatches and the Swatches panel. They live in the
//! preferences ([`crate::prefs::Preferences::swatches`]), so they persist like any other setting,
//! in the browser build too, and agents can add, list and delete them by command.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::prefs::Swatch;
use crate::{EngineError, Result, Session};

/// Most swatches kept; adding beyond this is refused rather than growing the preferences file.
pub const MAX_SWATCHES: usize = 1000;
/// Longest swatch name, in characters; longer names are cut.
pub const MAX_NAME: usize = 64;

fn bad(cmd: &str, msg: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

/// `#rrggbb` from `"#rrggbb"` (six hex digits) or `[r, g, b]` with components in 0..1.
fn color_of(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => {
            let h = s.trim().strip_prefix('#').unwrap_or(s.trim());
            (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", h.to_ascii_lowercase()))
        }
        Value::Array(a) if a.len() == 3 => {
            let mut c = [0u8; 3];
            for (out, x) in c.iter_mut().zip(a) {
                let f = x.as_f64().filter(|f| f.is_finite() && (0.0..=1.0).contains(f))?;
                *out = (f * 255.0).round() as u8;
            }
            Some(format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
        }
        _ => None,
    }
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.add";
    let color = color_of(p.get("color")).ok_or_else(|| bad(CMD, "`color` must be \"#rrggbb\" or [r, g, b] with components in 0..1"))?;
    let name = match p.get("name") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(n)) => n.trim().chars().take(MAX_NAME).collect(),
        Some(_) => return Err(bad(CMD, "`name` must be a string")),
    };
    let count = s.prefs().swatches.len();
    if count >= MAX_SWATCHES {
        return Err(EngineError::Other(format!("the swatch list is full ({MAX_SWATCHES} swatches); delete some first")));
    }
    // Photoshop names an unnamed swatch "Swatch N".
    let name = if name.is_empty() { format!("Swatch {}", count + 1) } else { name };
    let swatch = Swatch { name, color };
    s.edit_prefs(|pr| pr.swatches.push(swatch.clone()));
    Ok(json!({"index": count, "name": swatch.name, "color": swatch.color}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "swatches.delete";
    let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(CMD, "`index` must be a swatch index (0 or more)"))?;
    let count = s.prefs().swatches.len();
    let i = usize::try_from(index).ok().filter(|i| *i < count).ok_or_else(|| bad(CMD, &format!("no swatch {index} ({count} saved)")))?;
    let removed = s.edit_prefs(|pr| pr.swatches.remove(i));
    Ok(json!({"deleted": i, "name": removed.name, "color": removed.color}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "swatches.add",
            label: "Add to Swatches",
            menu: &[],
            shortcut: None,
            params: r##"{"color":"#rrggbb"|[r,g,b],"name":string?}"##,
            enabled: always,
            run: add,
            journal: true,
        },
        CommandSpec {
            id: "swatches.delete",
            label: "Delete Swatch",
            menu: &[],
            shortcut: None,
            params: r#"{"index":n}"#,
            enabled: always,
            run: delete,
            journal: true,
        },
        CommandSpec {
            id: "swatches.list",
            label: "List Swatches",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(json!({"swatches": s.prefs().swatches})),
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_list_and_delete() {
        let mut s = Session::new();
        let a = s.execute("swatches.add", json!({"color": "#3399CC", "name": "Sky"})).unwrap();
        assert_eq!(a, json!({"index": 0, "name": "Sky", "color": "#3399cc"}));
        // Unnamed swatches get Photoshop's "Swatch N"; a colour can also be [r, g, b] in 0..1.
        let b = s.execute("swatches.add", json!({"color": [1.0, 0.5, 0.0]})).unwrap();
        assert_eq!(b, json!({"index": 1, "name": "Swatch 2", "color": "#ff8000"}));
        let list = s.execute("swatches.list", json!({})).unwrap();
        assert_eq!(list["swatches"].as_array().map(Vec::len), Some(2));
        let d = s.execute("swatches.delete", json!({"index": 0})).unwrap();
        assert_eq!(d["name"], "Sky");
        assert_eq!(s.prefs().swatches, vec![Swatch { name: "Swatch 2".into(), color: "#ff8000".into() }]);
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = Session::new();
        for p in [
            json!({}),
            json!({"color": "#12345"}),
            json!({"color": "#12345g"}),
            json!({"color": "#+12345"}),
            json!({"color": "#ééé"}),
            json!({"color": [1.0, 2.0, 0.0]}),
            json!({"color": [f64::MAX, 0.0, 0.0]}),
            json!({"color": [0.1, 0.2]}),
            json!({"color": "#000000", "name": 5}),
            json!(null),
            json!("x"),
        ] {
            assert!(s.execute("swatches.add", p.clone()).is_err(), "{p}");
        }
        for p in [json!({}), json!({"index": -1}), json!({"index": 0}), json!({"index": u64::MAX}), json!({"index": "0"})] {
            assert!(s.execute("swatches.delete", p.clone()).is_err(), "{p}");
        }
        assert!(s.prefs().swatches.is_empty());
    }

    #[test]
    fn long_names_are_cut_and_the_list_is_capped() {
        let mut s = Session::new();
        let r = s.execute("swatches.add", json!({"color": "#000000", "name": "é".repeat(500)})).unwrap();
        assert_eq!(r["name"].as_str().map(|n| n.chars().count()), Some(MAX_NAME));
        s.edit_prefs(|pr| pr.swatches = vec![Swatch::default(); MAX_SWATCHES]);
        assert!(s.execute("swatches.add", json!({"color": "#000000"})).is_err());
        assert_eq!(s.prefs().swatches.len(), MAX_SWATCHES);
    }
}
