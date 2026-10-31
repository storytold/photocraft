//! The G'MIC filter catalogue: the filters G'MIC-Qt lists, as `gmic parse_gui json` prints them.
//!
//! That JSON (`"format_version": "gmic_json_1.0"`) groups filters by category; each filter has a
//! name, the G'MIC command that runs it and a parameter list. Every value is a string. Parameters
//! with a `pos` are command arguments, in `pos` order (a `point` takes two, a `color` three or
//! four); `note`, `separator` and `link` are only decoration. [`Filter::notation`] turns a filter
//! into the parameter notation of PhotoCraft's generated dialogs, and [`Filter::command_line`]
//! turns dialog values back into `command arg1,arg2,…`.
//!
//! The catalogue comes from the installed G'MIC at run time; none of it is bundled.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// Largest catalogue accepted (the full G'MIC 4.0 catalogue is about 3 MB).
pub const MAX_BYTES: usize = 64 << 20;
const MAX_FILTERS: usize = 20_000;
const MAX_PARAMS: usize = 256;
/// Parameter keys the engine commands use themselves.
const RESERVED: &[&str] = &["id", "layer", "target", "channel", "params", "command"];

/// A parsed catalogue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    /// The G'MIC version that produced it (`"4.0.5"`).
    pub gmic_version: String,
    /// Filters in G'MIC's order (by category).
    pub filters: Vec<Filter>,
}

/// One G'MIC-Qt filter.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    /// `category/name`, unique in the catalogue.
    pub id: String,
    /// The category (G'MIC-Qt's folder), e.g. `"Deformations"`.
    pub category: String,
    pub name: String,
    /// The G'MIC command, e.g. `"fx_kaleidoscope"`.
    pub command: String,
    /// The command G'MIC-Qt previews with (often the same).
    pub preview_command: String,
    /// Parameters that are command arguments, in argument order.
    pub params: Vec<Param>,
}

/// One filter parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// Dialog key (camelCase, unique in the filter).
    pub key: String,
    /// G'MIC's label, e.g. `"Angular Tiles"`.
    pub label: String,
    pub kind: ParamKind,
}

/// A parameter type and its default.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Float { min: f64, max: f64, default: f64 },
    Int { min: i64, max: i64, default: i64 },
    Bool(bool),
    /// Passed as the option's index.
    Choice { options: Vec<String>, default: usize },
    /// 3 (RGB) or 4 (RGBA) components, 0–255.
    Color(Vec<f64>),
    /// A position in percent (two arguments).
    Point { x: f64, y: f64 },
    /// Free text, passed quoted.
    Text(String),
    /// A file or folder path, passed quoted.
    Path(String),
    /// A fixed argument (`value`), passed as is; not shown.
    Value(String),
    /// A push button: `0` (not pressed); not shown.
    Button,
}

impl ParamKind {
    fn shown(&self) -> bool {
        !matches!(self, ParamKind::Value(_) | ParamKind::Button)
    }
}

#[derive(Deserialize)]
struct RawCatalog {
    #[serde(default)]
    format_version: String,
    #[serde(default)]
    gmic_version: String,
    #[serde(default)]
    categories: Vec<RawCategory>,
}

#[derive(Deserialize)]
struct RawCategory {
    #[serde(default)]
    name: String,
    #[serde(default)]
    filters: Vec<RawFilter>,
}

#[derive(Deserialize)]
struct RawFilter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    command: String,
    #[serde(default)]
    command_preview: String,
    #[serde(default)]
    parameters: Vec<Map<String, Value>>,
}

fn s<'a>(m: &'a Map<String, Value>, k: &str) -> &'a str {
    m.get(k).and_then(Value::as_str).unwrap_or("")
}

fn num(m: &Map<String, Value>, k: &str) -> Option<f64> {
    s(m, k).trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Labels may carry G'MIC-Qt markup (`<b>`, `<i>`): plain text, single line.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag && !c.is_control() => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `"X-Offset (%)"` → `"xOffset"`: the label's words, camelCased, ASCII alphanumerics only.
fn camel(label: &str) -> String {
    let mut out = String::new();
    for word in label.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let mut cs = word.chars();
        if let Some(first) = cs.next() {
            if out.is_empty() {
                out.extend(first.to_lowercase());
                out.push_str(&cs.as_str().to_lowercase());
            } else {
                out.extend(first.to_uppercase());
                out.push_str(&cs.as_str().to_lowercase());
            }
        }
        if out.len() > 40 {
            break;
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, 'p');
    }
    out
}

/// Choice options for the dialog notation: no `|` or `"`, not empty, unique.
fn option_label(text: &str, taken: &[String]) -> String {
    let mut o = plain(text).replace('|', "/").replace('"', "'");
    if o.is_empty() {
        o = "-".into();
    }
    let base = o.clone();
    let mut n = 2;
    while taken.contains(&o) {
        o = format!("{base} ({n})");
        n += 1;
    }
    o
}

fn parse_param(m: &Map<String, Value>) -> Option<(usize, String, ParamKind)> {
    let pos: usize = s(m, "pos").trim().parse().ok()?;
    let label = plain(s(m, "name"));
    let kind = match s(m, "type") {
        "float" => {
            let (a, b) = (num(m, "min").unwrap_or(0.0), num(m, "max").unwrap_or(1.0));
            let (min, max) = if a <= b { (a, b) } else { (b, a) };
            ParamKind::Float { min, max, default: num(m, "default").unwrap_or(min).clamp(min, max) }
        }
        "int" => {
            let (a, b) = (num(m, "min").unwrap_or(0.0).round() as i64, num(m, "max").unwrap_or(100.0).round() as i64);
            let (min, max) = if a <= b { (a, b) } else { (b, a) };
            ParamKind::Int { min, max, default: (num(m, "default").unwrap_or(0.0).round() as i64).clamp(min, max) }
        }
        "bool" => ParamKind::Bool(num(m, "default").is_some_and(|v| v != 0.0)),
        "choice" => {
            let raw = m.get("choices").and_then(Value::as_object)?;
            // Keys are "0", "1", …: order by index (JSON objects don't promise order).
            let mut indexed: Vec<(usize, &str)> = raw.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?))).collect();
            indexed.sort_by_key(|(i, _)| *i);
            let mut options: Vec<String> = Vec::with_capacity(indexed.len());
            for (_, text) in indexed.iter().take(256) {
                let o = option_label(text, &options);
                options.push(o);
            }
            if options.is_empty() {
                return None;
            }
            let default = num(m, "default").map_or(0, |d| d.max(0.0) as usize).min(options.len() - 1);
            ParamKind::Choice { options, default }
        }
        "color" => {
            let mut c: Vec<f64> = s(m, "default").split(',').filter_map(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 255.0)).collect();
            if c.len() < 3 {
                c.resize(3, 0.0);
            }
            c.truncate(4);
            ParamKind::Color(c)
        }
        "point" => {
            let mut xy = s(m, "position").split(',').map(|v| v.trim().parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(50.0));
            ParamKind::Point { x: xy.next().unwrap_or(50.0), y: xy.next().unwrap_or(50.0) }
        }
        "text" => ParamKind::Text(s(m, "default").to_string()),
        "file" | "folder" => ParamKind::Path(s(m, "default").to_string()),
        "value" => ParamKind::Value(s(m, "value").to_string()),
        "button" => ParamKind::Button,
        _ => return None,
    };
    Some((pos, label, kind))
}

impl Catalog {
    /// Parses `gmic parse_gui json` output. Filters without a runnable command (G'MIC-Qt's
    /// information pages use `_none_`) are left out.
    pub fn parse(bytes: &[u8]) -> Result<Catalog> {
        if bytes.len() > MAX_BYTES {
            return Err(Error::Catalog(format!("{} bytes is larger than the limit ({MAX_BYTES})", bytes.len())));
        }
        let raw: RawCatalog = serde_json::from_slice(bytes).map_err(|e| Error::Catalog(format!("not valid JSON: {e}")))?;
        if !raw.format_version.is_empty() && !raw.format_version.starts_with("gmic_json_1") {
            return Err(Error::Catalog(format!("unsupported format {:?}", raw.format_version)));
        }
        let mut filters: Vec<Filter> = Vec::new();
        let mut ids = std::collections::HashSet::new();
        for cat in raw.categories {
            let category = plain(&cat.name);
            for f in cat.filters {
                if filters.len() >= MAX_FILTERS {
                    return Err(Error::Catalog(format!("more than {MAX_FILTERS} filters")));
                }
                let command = f.command.trim().to_string();
                let name = plain(&f.name);
                if name.is_empty() || !valid_command(&command) || command == "_none_" {
                    continue;
                }
                let mut id = format!("{category}/{name}");
                let mut n = 2;
                while ids.contains(&id) {
                    id = format!("{category}/{name} ({n})");
                    n += 1;
                }
                ids.insert(id.clone());
                let mut params: Vec<(usize, String, ParamKind)> = f.parameters.iter().take(MAX_PARAMS).filter_map(parse_param).collect();
                params.sort_by_key(|(pos, ..)| *pos);
                let mut keys: Vec<String> = Vec::new();
                let params = params
                    .into_iter()
                    .map(|(pos, label, kind)| {
                        let mut key = camel(&label);
                        if key.is_empty() {
                            key = format!("p{pos}");
                        }
                        let base = key.clone();
                        let mut n = 2;
                        // Points add `X`/`Y` keys of their own: keep those unique too.
                        while keys.iter().any(|k| *k == key || *k == format!("{key}X") || *k == format!("{key}Y")) || RESERVED.contains(&key.as_str()) {
                            key = format!("{base}{n}");
                            n += 1;
                        }
                        keys.push(key.clone());
                        if matches!(kind, ParamKind::Point { .. }) {
                            keys.push(format!("{key}X"));
                            keys.push(format!("{key}Y"));
                        }
                        Param { key, label, kind }
                    })
                    .collect();
                let preview = f.command_preview.trim();
                let preview_command = if valid_command(preview) { preview.to_string() } else { command.clone() };
                filters.push(Filter { id, category: category.clone(), name, command, preview_command, params });
            }
        }
        Ok(Catalog { gmic_version: raw.gmic_version, filters })
    }

    /// The filter with `id`.
    pub fn get(&self, id: &str) -> Option<&Filter> {
        self.filters.iter().find(|f| f.id == id)
    }

    /// Category names in catalogue order.
    pub fn categories(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for f in &self.filters {
            if out.last() != Some(&f.category.as_str()) && !out.contains(&f.category.as_str()) {
                out.push(&f.category);
            }
        }
        out
    }
}

/// A G'MIC command name: letters, digits and `_`, not starting with a digit.
pub fn valid_command(c: &str) -> bool {
    !c.is_empty() && c.len() <= 128 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') && !c.starts_with(|ch: char| ch.is_ascii_digit())
}

/// A number as G'MIC reads it: no exponent, no trailing zeros.
fn fmt_num(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `text` as a quoted G'MIC string: `"`, `\`, `$`, `{` and `}` escaped (so a value can't end
/// the string or trigger substitution), newlines as `\n`, other control characters dropped.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' | '\\' | '$' | '{' | '}' => {
                out.push('\\');
                out.push(ch);
            }
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `#rrggbb` for colour components 0–255.
fn hex(c: &[f64]) -> String {
    let b = |i: usize| c.get(i).copied().unwrap_or(0.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", b(0), b(1), b(2))
}

fn parse_hex(s: &str) -> Option<[f64; 3]> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(f64::from);
    Some([c(0)?, c(2)?, c(4)?])
}

impl Filter {
    /// The parameter notation of PhotoCraft's generated dialogs
    /// (`{"angularTiles":1..72=10,"boundary":"Mirror|Transparent|…",…}`): numbers as sliders,
    /// a point as two percentage sliders, a colour as a swatch. The default choice is listed
    /// first (the dialog starts on the first option).
    pub fn notation(&self) -> String {
        let mut parts = Vec::new();
        for p in self.params.iter().filter(|p| p.kind.shown()) {
            let k = &p.key;
            match &p.kind {
                ParamKind::Float { min, max, default } => parts.push(format!("\"{k}\":{}..{}={}", fmt_num(*min), fmt_num(*max), fmt_num(*default))),
                ParamKind::Int { min, max, default } => parts.push(format!("\"{k}\":{min}..{max}={default}")),
                ParamKind::Bool(d) => parts.push(format!("\"{k}\":bool={d}")),
                ParamKind::Choice { options, default } => {
                    let mut o = options.clone();
                    if *default < o.len() {
                        let d = o.remove(*default);
                        o.insert(0, d);
                    }
                    parts.push(format!("\"{k}\":\"{}\"", o.join("|")));
                }
                ParamKind::Color(c) => parts.push(format!("\"{k}\":color={}", hex(c))),
                ParamKind::Point { x, y } => {
                    parts.push(format!("\"{k}X\":-100..200={}", fmt_num(*x)));
                    parts.push(format!("\"{k}Y\":-100..200={}", fmt_num(*y)));
                }
                ParamKind::Text(_) | ParamKind::Path(_) => parts.push(format!("\"{k}\":text")),
                ParamKind::Value(_) | ParamKind::Button => {}
            }
        }
        format!("{{{}}}", parts.join(","))
    }

    /// Dialog field values for the defaults (what a dialog opens with), including text defaults
    /// the notation can't carry.
    pub fn defaults(&self) -> Map<String, Value> {
        let mut m = Map::new();
        for p in &self.params {
            match &p.kind {
                ParamKind::Float { default, .. } => drop(m.insert(p.key.clone(), json!(default))),
                ParamKind::Int { default, .. } => drop(m.insert(p.key.clone(), json!(default))),
                ParamKind::Bool(d) => drop(m.insert(p.key.clone(), json!(d))),
                ParamKind::Choice { options, default } => drop(m.insert(p.key.clone(), json!(options.get(*default).cloned().unwrap_or_default()))),
                ParamKind::Color(c) => drop(m.insert(p.key.clone(), json!(hex(c)))),
                ParamKind::Point { x, y } => {
                    m.insert(format!("{}X", p.key), json!(x));
                    m.insert(format!("{}Y", p.key), json!(y));
                }
                ParamKind::Text(t) | ParamKind::Path(t) => drop(m.insert(p.key.clone(), json!(t))),
                ParamKind::Value(_) | ParamKind::Button => {}
            }
        }
        m
    }

    /// The command arguments for `values` (dialog keys; missing ones take their defaults). Numbers
    /// are clamped into range; a value of the wrong type is an error.
    pub fn args(&self, values: &Value) -> Result<Vec<String>> {
        let given = match values {
            Value::Null => &Map::new(),
            Value::Object(m) => m,
            _ => return Err(Error::Params("parameters must be a JSON object".into())),
        };
        let get = |k: &str| given.get(k).filter(|v| !v.is_null());
        let number = |k: &str| -> Result<Option<f64>> {
            match get(k) {
                None => Ok(None),
                Some(v) => {
                    let n = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|f: &f64| f.is_finite());
                    n.map(Some).ok_or_else(|| Error::Params(format!("`{k}` must be a number")))
                }
            }
        };
        let mut out = Vec::new();
        for p in &self.params {
            let k = p.key.as_str();
            match &p.kind {
                ParamKind::Float { min, max, default } => out.push(fmt_num(number(k)?.unwrap_or(*default).clamp(*min, *max))),
                ParamKind::Int { min, max, default } => out.push(format!("{}", (number(k)?.unwrap_or(*default as f64).round() as i64).clamp(*min, *max))),
                ParamKind::Bool(d) => {
                    let b = match get(k) {
                        None => *d,
                        Some(v) => v.as_bool().or_else(|| v.as_f64().map(|f| f != 0.0)).ok_or_else(|| Error::Params(format!("`{k}` must be true or false")))?,
                    };
                    out.push(if b { "1" } else { "0" }.into());
                }
                ParamKind::Choice { options, default } => {
                    let i = match get(k) {
                        None => *default,
                        Some(Value::String(s)) => options.iter().position(|o| o == s).ok_or_else(|| Error::Params(format!("`{k}` must be one of: {}", options.join(", "))))?,
                        Some(v) => v.as_u64().map(|i| i as usize).filter(|i| *i < options.len()).ok_or_else(|| Error::Params(format!("`{k}` must be one of: {}", options.join(", "))))?,
                    };
                    out.push(i.to_string());
                }
                ParamKind::Color(def) => {
                    let mut c = def.clone();
                    match get(k) {
                        None => {}
                        Some(Value::String(s)) => {
                            let rgb = parse_hex(s).ok_or_else(|| Error::Params(format!("`{k}` must be a colour like \"#ff8800\"")))?;
                            c.splice(..3, rgb);
                        }
                        Some(Value::Array(a)) => {
                            let v: Vec<f64> = a.iter().filter_map(Value::as_f64).filter(|f| f.is_finite()).collect();
                            if v.len() != a.len() || !(3..=4).contains(&v.len()) {
                                return Err(Error::Params(format!("`{k}` must be 3 or 4 numbers (0-255)")));
                            }
                            for (i, x) in v.into_iter().enumerate().take(c.len()) {
                                if let Some(slot) = c.get_mut(i) {
                                    *slot = x.clamp(0.0, 255.0);
                                }
                            }
                        }
                        Some(_) => return Err(Error::Params(format!("`{k}` must be a colour like \"#ff8800\""))),
                    }
                    out.extend(c.iter().map(|v| fmt_num(v.round())));
                }
                ParamKind::Point { x, y } => {
                    out.push(fmt_num(number(&format!("{k}X"))?.unwrap_or(*x).clamp(-1e6, 1e6)));
                    out.push(fmt_num(number(&format!("{k}Y"))?.unwrap_or(*y).clamp(-1e6, 1e6)));
                }
                ParamKind::Text(d) | ParamKind::Path(d) => {
                    let t = match get(k) {
                        None => d.as_str(),
                        Some(v) => v.as_str().ok_or_else(|| Error::Params(format!("`{k}` must be text")))?,
                    };
                    if t.len() > 64 * 1024 {
                        return Err(Error::Params(format!("`{k}` is longer than 64 KiB")));
                    }
                    out.push(quote(t));
                }
                ParamKind::Value(v) => out.push(v.clone()),
                ParamKind::Button => out.push("0".into()),
            }
        }
        Ok(out)
    }

    /// The G'MIC pipeline for `values`: `command arg1,arg2,…`.
    pub fn command_line(&self, values: &Value) -> Result<String> {
        let args = self.args(values)?;
        Ok(if args.is_empty() { self.command.clone() } else { format!("{} {}", self.command, args.join(",")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-written catalogue in G'MIC's `parse_gui json` shape (not G'MIC's own filters).
    const SAMPLE: &str = r##"{
      "format_version": "gmic_json_1.0", "gmic_version": "4.0.5",
      "categories": [
        { "name": "About", "filters": [
          { "name": "About G'MIC", "lang": "en", "command": "_none_", "command_preview": "_none_", "parameters": [] } ] },
        { "name": "Test <b>Shapes</b>", "filters": [
          { "name": "Swirl", "lang": "en", "command": "fx_test_swirl", "command_preview": "fx_test_swirl_preview", "parameters": [
            { "type": "note", "text": "A note" },
            { "type": "separator" },
            { "type": "point", "name": "Center (%)", "position": "40,60", "pos": "1" },
            { "type": "int", "name": "Turns", "default": "3", "min": "1", "max": "12", "pos": "3" },
            { "type": "float", "name": "Amount", "default": "0.5", "min": "0", "max": "2", "pos": "4" },
            { "type": "choice", "name": "Boundary", "default": "2", "pos": "5", "choices": { "0": "None", "1": "A|B", "2": "Mirror \"x\"" } },
            { "type": "bool", "name": "Invert?", "default": "1", "pos": "6" },
            { "type": "color", "name": "Tint", "default": "255,128,0,200", "pos": "7" },
            { "type": "text", "name": "Label", "default": "hi", "pos": "11" },
            { "type": "value", "value": "42", "pos": "12" },
            { "type": "button", "name": "Reset", "pos": "13" },
            { "type": "link", "name": "Docs", "url": "https://example.org", "align": "left" }
          ] },
          { "name": "Swirl", "lang": "en", "command": "fx_other", "command_preview": "", "parameters": [] },
          { "name": "Bad", "lang": "en", "command": "rm -rf; x", "parameters": [] }
        ] }
      ]
    }"##;

    fn swirl() -> Filter {
        Catalog::parse(SAMPLE.as_bytes()).unwrap().filters.remove(0)
    }

    #[test]
    fn parses_filters_and_skips_info_pages_and_bad_commands() {
        let c = Catalog::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(c.gmic_version, "4.0.5");
        let ids: Vec<&str> = c.filters.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["Test Shapes/Swirl", "Test Shapes/Swirl (2)"]);
        assert_eq!(c.categories(), ["Test Shapes"]);
        let f = &c.filters[0];
        assert_eq!(f.preview_command, "fx_test_swirl_preview");
        assert_eq!(c.filters[1].preview_command, "fx_other", "an empty preview command falls back to the command");
        let keys: Vec<&str> = f.params.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["center", "turns", "amount", "boundary", "invert", "tint", "label", "p12", "reset"]);
    }

    #[test]
    fn notation_and_defaults() {
        let f = swirl();
        assert_eq!(
            f.notation(),
            r##"{"centerX":-100..200=40,"centerY":-100..200=60,"turns":1..12=3,"amount":0..2=0.5,"boundary":"Mirror 'x'|None|A/B","invert":bool=true,"tint":color=#ff8000,"label":text}"##
        );
        let d = f.defaults();
        assert_eq!(d["boundary"], json!("Mirror 'x'"));
        assert_eq!(d["label"], json!("hi"));
        assert_eq!(d["centerX"], json!(40.0));
    }

    #[test]
    fn default_command_line() {
        assert_eq!(swirl().command_line(&Value::Null).unwrap(), r#"fx_test_swirl 40,60,3,0.5,2,1,255,128,0,200,"hi",42,0"#);
    }

    #[test]
    fn values_are_clamped_converted_and_quoted() {
        let f = swirl();
        let v = json!({"centerX": 10.25, "turns": 99, "amount": -1, "boundary": "A/B", "invert": false, "tint": "#0a0b0c", "label": "a\"b${x}{1}\\\nz"});
        assert_eq!(f.command_line(&v).unwrap(), r#"fx_test_swirl 10.25,60,12,0,1,0,10,11,12,200,"a\"b\$\{x\}\{1\}\\\nz",42,0"#);
        // Choice by index, colour as an array, numbers as strings (form fields).
        let v = json!({"boundary": 0, "tint": [1, 2, 3, 4], "turns": "5"});
        assert_eq!(f.args(&v).unwrap()[2..10], ["5", "0.5", "0", "1", "1", "2", "3", "4"]);
    }

    #[test]
    fn bad_values_are_errors() {
        let f = swirl();
        for v in [
            json!([1, 2]),
            json!({"turns": "many"}),
            json!({"amount": {}}),
            json!({"boundary": "Nope"}),
            json!({"boundary": 9}),
            json!({"invert": "yes"}),
            json!({"tint": "orange"}),
            json!({"tint": [1, 2]}),
            json!({"tint": [1, "x", 3]}),
            json!({"label": 5}),
            json!({"centerX": "left"}),
        ] {
            assert!(f.args(&v).is_err(), "{v}");
        }
        assert!(f.args(&json!({"label": "x".repeat(70_000)})).is_err());
    }

    #[test]
    fn malformed_catalogues_are_errors_or_skipped() {
        assert!(Catalog::parse(b"").is_err());
        assert!(Catalog::parse(b"[1,2]").is_err());
        assert!(Catalog::parse(br#"{"format_version":"gmic_json_9","categories":[]}"#).is_err());
        // Wrong types inside a parameter skip just that parameter.
        let c = Catalog::parse(
            br#"{"categories":[{"name":"C","filters":[{"name":"F","command":"fx_f","parameters":[
                {"type":"float","name":"A","pos":"x"},{"type":"choice","name":"B","pos":"1","choices":{}},
                {"type":"float","name":"C","pos":"2","min":"5","max":"1","default":"9"},{"type":"mystery","pos":"3"}]}]}]}"#,
        )
        .unwrap();
        let f = &c.filters[0];
        assert_eq!(f.params.len(), 1);
        assert_eq!(f.params[0].kind, ParamKind::Float { min: 1.0, max: 5.0, default: 5.0 });
        assert!(Catalog::parse(&vec![b' '; MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn helpers() {
        assert_eq!(camel("X-Offset (%)"), "xOffset");
        assert_eq!(camel("3D Depth"), "p3dDepth");
        assert_eq!(plain("<b>Bold</b>\n  text"), "Bold text");
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
        assert_eq!(fmt_num(-1.5), "-1.5");
        assert!(valid_command("fx_smooth_anisotropic"));
        assert!(!valid_command("fx;rm"));
        assert!(!valid_command("1abc"));
        assert_eq!(quote("plain"), "\"plain\"");
    }
}
