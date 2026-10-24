//! The plug-in manifest (`pc_manifest`) and parameter schema.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{Error, Result};

/// Longest manifest the host reads.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Most parameters a plug-in may declare.
pub const MAX_PARAMS: usize = 64;
/// Range of a `point` coordinate: percent of the canvas width or height.
pub const POINT_RANGE: (f64, f64) = (0.0, 100.0);
/// Largest neighbourhood (`overlap`) a plug-in may ask for, in pixels.
pub const MAX_OVERLAP: u32 = 256;
/// Longest parameter `group` (a dialog tab label), in characters.
pub const MAX_GROUP_CHARS: usize = 32;

/// What a plug-in is. ABI v1 has filters only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Filter,
}

/// Which pixels a filter is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Area {
    /// The layer's non-empty pixels (like Invert or Gaussian Blur).
    #[default]
    Content,
    /// The whole canvas, including transparent pixels (like Clouds).
    Canvas,
}

/// What a `number` or `int` parameter measures (`"unit"` in the manifest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Unit {
    /// No unit (the default): a level, factor, count or percentage, the same at any image size.
    #[default]
    None,
    /// A length in document pixels (a radius, distance, shift or size). On a reduced live preview
    /// the host divides it by the reduction, so the preview matches what OK applies.
    Px,
}

impl Unit {
    fn is_none(&self) -> bool {
        *self == Unit::None
    }
}

/// Reads `"unit"` leniently: `"px"` is [`Unit::Px`]; anything else (an unknown string such as
/// `"pt"`, a number, `null`, an array…) is unitless, as when the field is absent. Manifests from
/// before `unit` existed could carry any value there (unknown fields were ignored), and they must
/// keep installing.
fn lenient_unit<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Unit, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) if s == "px" => Unit::Px,
        _ => Unit::None,
    })
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ParamSpec {
    /// A real number in `min..=max` (a slider in the dialog).
    Number {
        min: f64,
        max: f64,
        default: f64,
        #[serde(default, deserialize_with = "lenient_unit", skip_serializing_if = "Unit::is_none")]
        unit: Unit,
    },
    /// A whole number in `min..=max`.
    Int {
        #[serde(default = "int_min")]
        min: i64,
        #[serde(default = "int_max")]
        max: i64,
        #[serde(default)]
        default: i64,
        #[serde(default, deserialize_with = "lenient_unit", skip_serializing_if = "Unit::is_none")]
        unit: Unit,
    },
    /// A checkbox.
    Bool {
        #[serde(default)]
        default: bool,
    },
    /// One of `options` (a dropdown).
    Choice {
        options: Vec<String>,
        #[serde(default)]
        default: Option<String>,
    },
    /// A point on the canvas, `[x, y]` in percent of the canvas width and height (0-100): a
    /// draggable handle on the canvas while the dialog is open.
    Point { default: [f64; 2] },
    /// An opaque colour, `"#rrggbb"` (a swatch that opens a colour picker). The plug-in receives
    /// the lowercase hex string.
    Color { default: String },
    /// A direction in degrees, `0..=360` (a dial in the dialog): 0 points right (east) and the
    /// angle grows clockwise on screen. The plug-in receives a number in `0..=360`.
    Angle {
        #[serde(default)]
        default: f64,
    },
}

fn int_min() -> i64 {
    -1_000_000
}
fn int_max() -> i64 {
    1_000_000
}

/// The manifest a plug-in returns from `pc_manifest` (UTF-8 JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Stable id, `[A-Za-z0-9._-]{1,64}` (reverse-DNS style is recommended: `org.example.invert`).
    pub id: String,
    /// Menu label.
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub kind: Kind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// Parameters in declaration (dialog) order; a JSON object `{"name": spec, …}` in the manifest.
    #[serde(default, deserialize_with = "ordered_params", serialize_with = "params_as_object")]
    pub params: Vec<(String, ParamSpec)>,
    /// Pixels of context the filter needs around each output pixel (0 = point filter).
    #[serde(default)]
    pub overlap: u32,
    #[serde(default)]
    pub area: Area,
    /// Optional dotted guide line the dialog draws on the canvas between two `point` params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guide: Option<Guide>,
    /// Each parameter's optional `"group"` (the dialog tab it sits on), in `params` order; `None`
    /// keeps the parameter above the tabs, always visible. Read by [`Manifest::parse`].
    #[serde(skip)]
    pub groups: Vec<Option<String>>,
}

/// A dotted line between two `point` parameters (`"guide": {"from": "start", "to": "end"}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Guide {
    pub from: String,
    pub to: String,
}

/// A `"#rrggbb"` colour (either case), lowercased; `None` for anything else (no alpha, no `#rgb`).
pub fn normalize_color(s: &str) -> Option<String> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| format!("#{}", hex.to_ascii_lowercase()))
}

/// Range of an `angle` parameter, in degrees.
pub const ANGLE_RANGE: (f64, f64) = (0.0, 360.0);

/// An angle in degrees brought into [`ANGLE_RANGE`]: values already in `0..=360` are kept (360
/// stays 360), others wrap around the circle (370 → 10, -90 → 270, 720 → 0). `None` when not finite.
pub fn wrap_angle(deg: f64) -> Option<f64> {
    let (lo, hi) = ANGLE_RANGE;
    deg.is_finite().then(|| if (lo..=hi).contains(&deg) { deg } else { deg.rem_euclid(hi) })
}

/// Clamps a point to the canvas: each coordinate into [`POINT_RANGE`] (non-finite values are `None`).
pub fn clamp_point(x: f64, y: f64) -> Option<[f64; 2]> {
    let (lo, hi) = POINT_RANGE;
    (x.is_finite() && y.is_finite()).then(|| [x.clamp(lo, hi), y.clamp(lo, hi)])
}

impl Manifest {
    /// Parses and validates manifest JSON.
    pub fn parse(bytes: &[u8]) -> Result<Manifest> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::Manifest(format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", bytes.len())));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::Manifest("manifest is not UTF-8".into()))?;
        let mut man: Manifest = serde_json::from_str(text).map_err(|e| Error::Manifest(format!("manifest: {e}")))?;
        let groups: ParamGroups = serde_json::from_str(text).map_err(|e| Error::Manifest(format!("manifest: {e}")))?;
        man.groups = groups.params.into_iter().map(|g| g.map(|g| g.trim().to_string())).collect();
        man.validate()?;
        Ok(man)
    }

    fn validate(&self) -> Result<()> {
        if !valid_id(&self.id) {
            return Err(Error::Manifest(format!("invalid id {:?}: use 1-64 characters from A-Z a-z 0-9 . _ -", self.id)));
        }
        if !printable(self.name.trim(), 64) {
            return Err(Error::Manifest("`name` must be 1-64 printable characters".into()));
        }
        if self.version.len() > 32 || self.description.len() > 1024 || self.author.len() > 128 {
            return Err(Error::Manifest("`version`, `description` or `author` is too long".into()));
        }
        if self.overlap > MAX_OVERLAP {
            return Err(Error::Manifest(format!("`overlap` {} exceeds {MAX_OVERLAP}", self.overlap)));
        }
        for (k, spec) in &self.params {
            if !valid_key(k) {
                return Err(Error::Manifest(format!("invalid parameter name {k:?}")));
            }
            if let Some(g) = self.group(k)
                && !printable(g, MAX_GROUP_CHARS)
            {
                return Err(Error::Manifest(format!("parameter `{k}`: `group` must be 1-{MAX_GROUP_CHARS} printable characters")));
            }
            match spec {
                ParamSpec::Number { min, max, default, .. } if !(min.is_finite() && max.is_finite() && default.is_finite()) || min > max => {
                    return Err(Error::Manifest(format!("parameter `{k}`: bad range")));
                }
                ParamSpec::Int { min, max, .. } if min > max => return Err(Error::Manifest(format!("parameter `{k}`: bad range"))),
                ParamSpec::Choice { options, default } => {
                    if options.is_empty() || options.len() > 64 || options.iter().any(|o| o.is_empty() || o.contains(['|', '"'])) {
                        return Err(Error::Manifest(format!("parameter `{k}`: choices must be 1-64 non-empty strings without | or \"")));
                    }
                    if default.as_ref().is_some_and(|d| !options.contains(d)) {
                        return Err(Error::Manifest(format!("parameter `{k}`: default is not one of the options")));
                    }
                }
                ParamSpec::Point { default } => {
                    let (lo, hi) = POINT_RANGE;
                    if !default.iter().all(|v| v.is_finite() && (lo..=hi).contains(v)) {
                        return Err(Error::Manifest(format!("parameter `{k}`: point default must be two numbers in {lo}-{hi}")));
                    }
                }
                ParamSpec::Angle { default } if !(default.is_finite() && (ANGLE_RANGE.0..=ANGLE_RANGE.1).contains(default)) => {
                    return Err(Error::Manifest(format!("parameter `{k}`: angle default must be a number of degrees in 0-360, not {default}")));
                }
                ParamSpec::Color { default } if normalize_color(default).is_none() => {
                    return Err(Error::Manifest(format!("parameter `{k}`: color default must be a \"#rrggbb\" hex string, not {default:?}")));
                }
                _ => {}
            }
        }
        if let Some(g) = &self.guide {
            let is_point = |name: &str| self.params.iter().any(|(k, s)| k == name && matches!(s, ParamSpec::Point { .. }));
            if g.from == g.to || !is_point(&g.from) || !is_point(&g.to) {
                return Err(Error::Manifest("`guide` must name two different `point` parameters".into()));
            }
        }
        Ok(())
    }

    /// The `group` of parameter `key`, if it has one.
    pub fn group(&self, key: &str) -> Option<&str> {
        let i = self.params.iter().position(|(k, _)| k == key)?;
        self.groups.get(i)?.as_deref()
    }

    /// The dialog's tabs: each distinct `group`, in the order it first appears in `params`.
    pub fn tabs(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for g in self.groups.iter().flatten() {
            if !out.contains(&g.as_str()) {
                out.push(g);
            }
        }
        out
    }

    /// Parameters without a `group`: shown above the tabs, always visible.
    pub fn always_visible(&self) -> Vec<&str> {
        self.params.iter().map(|(k, _)| k.as_str()).filter(|k| self.group(k).is_none()).collect()
    }

    /// The parameter notation the command registry and the generated dialogs use
    /// (`{"amount":0..100=50,"mode":"a|b","mono":bool=false}`).
    pub fn params_notation(&self) -> String {
        let parts: Vec<String> = self
            .params
            .iter()
            .map(|(k, spec)| match spec {
                ParamSpec::Number { min, max, default, .. } => format!("\"{k}\":{min}..{max}={default}"),
                ParamSpec::Int { default, .. } => format!("\"{k}\":int={default}"),
                ParamSpec::Bool { default } => format!("\"{k}\":bool={default}"),
                ParamSpec::Choice { options, default } => {
                    // The dialog's default is the first option, so list the default first.
                    let mut opts = options.clone();
                    if let Some(d) = default
                        && let Some(i) = opts.iter().position(|o| o == d)
                    {
                        let d = opts.remove(i);
                        opts.insert(0, d);
                    }
                    format!("\"{k}\":\"{}\"", opts.join("|"))
                }
                ParamSpec::Point { default: [x, y] } => format!("\"{k}\":point=[{x},{y}]"),
                ParamSpec::Color { default } => format!("\"{k}\":color={}", normalize_color(default).unwrap_or_default()),
                ParamSpec::Angle { default } => format!("\"{k}\":angle={default}"),
            })
            .collect();
        format!("{{{}}}", parts.join(","))
    }

    /// Validates user `params` against the schema: missing keys take their defaults, numbers (defaults
    /// included) are clamped into range, unknown keys are dropped; a value of the wrong type is an error.
    pub fn resolve_params(&self, params: &Value) -> Result<Map<String, Value>> {
        let given = match params {
            Value::Null => &Map::new(),
            Value::Object(m) => m,
            _ => return Err(Error::Params("plug-in parameters must be a JSON object".into())),
        };
        let mut out = Map::new();
        for (k, spec) in &self.params {
            let v = given.get(k).filter(|v| !v.is_null());
            let wrong = |what: &str| Error::Params(format!("`{k}` must be {what}"));
            let resolved = match spec {
                ParamSpec::Number { min, max, default, .. } => match v {
                    None => json!((*default).clamp(*min, *max)),
                    Some(v) => json!(v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?.clamp(*min, *max)),
                },
                ParamSpec::Int { min, max, default, .. } => match v {
                    None => json!((*default).clamp(*min, *max)),
                    Some(v) => {
                        let f = v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| wrong("a number"))?;
                        json!((f.round().clamp(*min as f64, *max as f64)) as i64)
                    }
                },
                ParamSpec::Bool { default } => match v {
                    None => json!(default),
                    Some(v) => json!(v.as_bool().ok_or_else(|| wrong("true or false"))?),
                },
                ParamSpec::Choice { options, default } => match v {
                    None => json!(default.clone().or_else(|| options.first().cloned()).unwrap_or_default()),
                    Some(v) => {
                        let s = v.as_str().ok_or_else(|| wrong("a string"))?;
                        if !options.iter().any(|o| o == s) {
                            return Err(Error::Params(format!("`{k}` must be one of {}", options.join(", "))));
                        }
                        json!(s)
                    }
                },
                ParamSpec::Point { default: [x, y] } => {
                    let [x, y] = match v {
                        None => [*x, *y],
                        Some(v) => match v.as_array().map(Vec::as_slice) {
                            Some([x, y]) => [x.as_f64().ok_or_else(|| wrong("[x, y]"))?, y.as_f64().ok_or_else(|| wrong("[x, y]"))?],
                            _ => return Err(wrong("an array [x, y]")),
                        },
                    };
                    json!(clamp_point(x, y).ok_or_else(|| wrong("two finite numbers"))?)
                }
                ParamSpec::Color { default } => match v {
                    None => json!(normalize_color(default).unwrap_or_default()),
                    Some(v) => json!(v.as_str().and_then(normalize_color).ok_or_else(|| wrong("a \"#rrggbb\" colour"))?),
                },
                ParamSpec::Angle { default } => match v {
                    None => json!(wrap_angle(*default).unwrap_or(0.0)),
                    Some(v) => json!(v.as_f64().and_then(wrap_angle).ok_or_else(|| wrong("a number of degrees"))?),
                },
            };
            out.insert(k.clone(), resolved);
        }
        Ok(out)
    }

    /// Whether parameter `key` is a length in pixels (`"unit": "px"`).
    pub fn is_pixel_param(&self, key: &str) -> bool {
        self.pixel_params().any(|(k, ..)| k == key)
    }

    /// `(key, min, max, default, is_int)` of each parameter declared `"unit": "px"`.
    fn pixel_params(&self) -> impl Iterator<Item = (&str, f64, f64, f64, bool)> {
        self.params.iter().filter_map(|(k, spec)| match spec {
            ParamSpec::Number { min, max, default, unit: Unit::Px } => Some((k.as_str(), *min, *max, *default, false)),
            ParamSpec::Int { min, max, default, unit: Unit::Px } => Some((k.as_str(), *min as f64, *max as f64, *default as f64, true)),
            _ => None,
        })
    }

    /// The value [`Manifest::resolve_params`] would use for a pixel parameter: the given number, or
    /// the default when missing, within `min..=max`. `None` for a value of the wrong type (which
    /// `resolve_params` rejects) or one that isn't finite.
    fn pixel_value(params: &Map<String, Value>, key: &str, min: f64, max: f64, default: f64) -> Option<f64> {
        let v = match params.get(key).filter(|v| !v.is_null()) {
            None => default,
            Some(v) => v.as_f64()?,
        };
        // `max`/`min` rather than `clamp`, which panics on a reversed or NaN range.
        v.is_finite().then(|| v.max(min).min(max))
    }

    /// Proxy factor for a live preview with `params` on a document reduced `k` times: the largest
    /// factor up to `k` at which no pixel parameter with a positive minimum, divided by it, drops
    /// below that minimum. A clamped value would change the feature size (as for built-in filters,
    /// #2063), so such a preview runs less reduced instead. Always at least 1.
    pub fn preview_factor(&self, params: &Map<String, Value>, k: u32) -> u32 {
        self.pixel_params().fold(k.max(1), |k, (key, min, max, default, _)| match Self::pixel_value(params, key, min, max, default) {
            Some(v) if min > 0.0 => k.min((v / min).floor().max(1.0).min(f64::from(k)) as u32),
            Some(v) if max < 0.0 => k.min((v / max).floor().max(1.0).min(f64::from(k)) as u32),
            _ => k,
        })
    }

    /// Rewrites the plug-in's `params` for a live preview on a copy of the document reduced `k`
    /// times: each pixel parameter (the given value, or its default when missing) is divided by
    /// `k`, rounded for `int`s and kept within its range, so the preview shows the same lengths in
    /// document pixels as OK applies (#7524). Unitless parameters, unknown keys and values of the
    /// wrong type (which [`Manifest::resolve_params`] rejects) are left as they are.
    pub fn scale_for_preview(&self, params: &mut Map<String, Value>, k: u32) {
        if k <= 1 {
            return;
        }
        let k = f64::from(k);
        for (key, min, max, default, int) in self.pixel_params() {
            let Some(v) = Self::pixel_value(params, key, min, max, default) else { continue };
            let x = (v / k).max(min).min(max);
            params.insert(key.to_string(), if int { json!(x.round().max(min).min(max) as i64) } else { json!(x) });
        }
    }
}

/// Reads the `params` object keeping its key order (serde_json's `Map` would sort it).
fn ordered_params<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<(String, ParamSpec)>, D::Error> {
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<(String, ParamSpec)>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of parameter specs")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some((k, v)) = map.next_entry::<String, ParamSpec>()? {
                if out.len() >= MAX_PARAMS {
                    return Err(serde::de::Error::custom(format!("too many parameters (limit {MAX_PARAMS})")));
                }
                out.push((k, v));
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

/// The `group` of each parameter, in manifest order (a second, lenient read of the manifest).
#[derive(Deserialize)]
struct ParamGroups {
    #[serde(default, deserialize_with = "ordered_groups")]
    params: Vec<Option<String>>,
}

fn ordered_groups<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Vec<Option<String>>, D::Error> {
    #[derive(Deserialize)]
    struct Entry {
        #[serde(default)]
        group: Option<String>,
    }
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = Vec<Option<String>>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object of parameter specs")
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
            Ok(Vec::new())
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some((_, e)) = map.next_entry::<String, Entry>()? {
                out.push(e.group);
            }
            Ok(out)
        }
    }
    d.deserialize_any(V)
}

fn params_as_object<S: serde::Serializer>(p: &[(String, ParamSpec)], s: S) -> std::result::Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut m = s.serialize_map(Some(p.len()))?;
    for (k, v) in p {
        m.serialize_entry(k, v)?;
    }
    m.end()
}

/// `[A-Za-z0-9._-]{1,64}`.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The Unicode format characters (general category Cf, Unicode 15.1). Most draw nothing, and the bidi
/// controls among them (U+061C, U+200E-200F, U+202A-202E, U+2066-2069) reorder the text around them.
const FORMAT: &[(char, char)] = &[
    ('\u{AD}', '\u{AD}'),
    ('\u{600}', '\u{605}'),
    ('\u{61C}', '\u{61C}'),
    ('\u{6DD}', '\u{6DD}'),
    ('\u{70F}', '\u{70F}'),
    ('\u{890}', '\u{891}'),
    ('\u{8E2}', '\u{8E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];

/// Default-ignorable characters outside Cf (fillers, variation selectors): allowed in a name, but they
/// draw nothing on their own.
const BLANK: &[(char, char)] = &[
    ('\u{34F}', '\u{34F}'),
    ('\u{115F}', '\u{1160}'),
    ('\u{17B4}', '\u{17B5}'),
    ('\u{180B}', '\u{180D}'),
    ('\u{180F}', '\u{180F}'),
    ('\u{3164}', '\u{3164}'),
    ('\u{FE00}', '\u{FE0F}'),
    ('\u{FFA0}', '\u{FFA0}'),
    ('\u{E0100}', '\u{E01EF}'),
];

fn in_ranges(c: char, ranges: &[(char, char)]) -> bool {
    ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// A format character a menu label must not carry: it hides in the label or reorders it. The zero-width
/// non-joiner and joiner stay allowed, since Persian, Indic scripts and emoji sequences need them and they
/// cannot reorder text.
pub(crate) fn hidden_format(c: char) -> bool {
    !matches!(c, '\u{200C}' | '\u{200D}') && in_ranges(c, FORMAT)
}

/// 1-`max` characters, none of them control or hidden format characters, at least one visible
/// (the menu label's rules; `s` is already trimmed).
fn printable(s: &str, max: usize) -> bool {
    s.chars().all(|c| !c.is_control() && !hidden_format(c)) && s.chars().any(visible) && s.chars().count() <= max
}

/// A character that draws something on its own.
pub(crate) fn visible(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !in_ranges(c, FORMAT) && !in_ranges(c, BLANK)
}

/// Names the host's `plugin.run` command takes for itself: a parameter of the same name would be
/// read as the command's routing (which layer or channel is filtered), not as the plug-in's.
const HOST_KEYS: [&str; 4] = ["id", "layer", "target", "channel"];

fn valid_key(k: &str) -> bool {
    (1..=64).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !k.starts_with('_') && !HOST_KEYS.contains(&k)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{"id":"org.example.tone","name":"Tone","version":"1.2","kind":"filter",
        "params":{"amount":{"type":"number","min":0,"max":100,"default":50},
                  "mode":{"type":"choice","options":["soft","hard"],"default":"hard"},
                  "mono":{"type":"bool"},"seed":{"type":"int","min":0,"max":9}},
        "overlap":2,"area":"canvas"}"#;

    #[test]
    fn parses_and_keeps_param_order() {
        let m = Manifest::parse(FULL.as_bytes()).unwrap();
        assert_eq!(m.id, "org.example.tone");
        assert_eq!(m.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["amount", "mode", "mono", "seed"]);
        assert_eq!(m.overlap, 2);
        assert_eq!(m.area, Area::Canvas);
        assert_eq!(m.params_notation(), r#"{"amount":0..100=50,"mode":"hard|soft","mono":bool=false,"seed":int=0}"#);
    }

    #[test]
    fn resolves_defaults_clamps_and_rejects_wrong_types() {
        let m = Manifest::parse(FULL.as_bytes()).unwrap();
        let p = m.resolve_params(&json!({"amount": 500, "seed": 3.6, "junk": 1})).unwrap();
        assert_eq!(Value::Object(p), json!({"amount": 100.0, "mode": "hard", "mono": false, "seed": 4}));
        assert!(m.resolve_params(&json!({"amount": "lots"})).is_err());
        assert!(m.resolve_params(&json!({"mode": "medium"})).is_err());
        assert!(m.resolve_params(&json!({"mono": 1})).is_err());
        assert!(m.resolve_params(&json!([1, 2])).is_err());
        assert!(m.resolve_params(&Value::Null).is_ok());
    }

    #[test]
    fn out_of_range_defaults_are_clamped() {
        // Issue #1012: a missing value fell back to a default outside the declared range unclamped.
        let m = Manifest::parse(
            br#"{"id":"org.example.tone","name":"Tone","kind":"filter",
                "params":{"amount":{"type":"number","min":0,"max":100,"default":1000},
                          "steps":{"type":"int","min":1,"max":16,"default":1000},
                          "low":{"type":"number","min":-1,"max":1,"default":-5},
                          "count":{"type":"int","min":3,"max":9}}}"#,
        )
        .unwrap();
        let resolved = json!({"amount": 100.0, "steps": 16, "low": -1.0, "count": 3});
        assert_eq!(Value::Object(m.resolve_params(&json!({})).unwrap()), resolved);
        assert_eq!(Value::Object(m.resolve_params(&Value::Null).unwrap()), resolved);
        assert_eq!(Value::Object(m.resolve_params(&json!({"amount": null, "steps": null})).unwrap()), resolved);
        let given = m.resolve_params(&json!({"amount": 500, "steps": 500, "low": 0.5, "count": 5})).unwrap();
        assert_eq!(Value::Object(given), json!({"amount": 100.0, "steps": 16, "low": 0.5, "count": 5}));
        // In-range defaults are unchanged.
        let full = Manifest::parse(FULL.as_bytes()).unwrap();
        assert_eq!(Value::Object(full.resolve_params(&json!({})).unwrap()), json!({"amount": 50.0, "mode": "hard", "mono": false, "seed": 0}));
    }

    #[test]
    fn rejects_bad_manifests() {
        for bad in [
            "",
            "[]",
            "not json",
            r#"{"id":"","name":"x","kind":"filter"}"#,
            r#"{"id":"a b","name":"x","kind":"filter"}"#,
            r#"{"id":"a","name":"","kind":"filter"}"#,
            r#"{"id":"a","name":"x","kind":"panel"}"#,
            r#"{"id":"a","name":"x"}"#,
            r#"{"id":"a","name":"x","kind":"filter","overlap":100000}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"r":{"type":"number","min":5,"max":1,"default":1}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"c":{"type":"choice","options":[]}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"__x":{"type":"bool"}}}"#,
            // Names the host command routes by: a plug-in default must not steer the filter.
            r#"{"id":"a","name":"x","kind":"filter","params":{"target":{"type":"bool","default":false}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":{"channel":{"type":"int","min":0,"max":3,"default":1}}}"#,
            r#"{"id":"a","name":"x","kind":"filter","params":[1]}"#,
        ] {
            assert!(Manifest::parse(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(Manifest::parse(&[0xff, 0xfe]).is_err());
        assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    }

    fn with_points(params: Value, guide: Value) -> Result<Manifest> {
        let mut m = json!({"id": "org.example.bolt", "name": "Bolt", "kind": "filter", "params": params});
        if !guide.is_null() {
            m["guide"] = guide;
        }
        Manifest::parse(m.to_string().as_bytes())
    }

    #[test]
    fn point_params_parse_with_a_guide() {
        // A raw string: `json!` would sort the keys, and the dialog order is the manifest's.
        let m = Manifest::parse(
            br#"{"id":"org.example.bolt","name":"Bolt","kind":"filter",
                "params":{"start":{"type":"point","default":[12,12]},"end":{"type":"point","default":[88,62.5]},
                          "glow":{"type":"number","min":0,"max":1,"default":0.5}},
                "guide":{"from":"start","to":"end"}}"#,
        )
        .unwrap();
        assert_eq!(m.params[0].1, ParamSpec::Point { default: [12.0, 12.0] });
        assert_eq!(m.guide, Some(Guide { from: "start".into(), to: "end".into() }));
        assert_eq!(m.params_notation(), r#"{"start":point=[12,12],"end":point=[88,62.5],"glow":0..1=0.5}"#);
        // Points without a guide are fine too.
        let params = json!({"start": {"type": "point", "default": [12, 12]}, "end": {"type": "point", "default": [88, 62.5]}});
        assert_eq!(with_points(params, Value::Null).unwrap().guide, None);
    }

    #[test]
    fn rejects_bad_point_params_and_guides() {
        let pt = |d: Value| json!({"start": {"type": "point", "default": d}, "end": {"type": "point", "default": [1, 1]}});
        // Wrong length, wrong type, out of range.
        for d in [json!([1]), json!([1, 2, 3]), json!([]), json!(["a", 1]), json!(5), json!([-1, 50]), json!([50, 100.5]), json!([101, 0])] {
            assert!(with_points(pt(d.clone()), Value::Null).is_err(), "{d}");
        }
        assert!(with_points(json!({"p": {"type": "point"}}), Value::Null).is_err(), "a point needs a default");
        // Edges of the range are allowed.
        assert!(with_points(pt(json!([0, 100])), Value::Null).is_ok());
        let params = json!({"start": {"type": "point", "default": [0, 0]}, "end": {"type": "point", "default": [1, 1]}, "n": {"type": "number", "min": 0, "max": 1, "default": 0}});
        for g in [
            json!({"from": "start", "to": "nowhere"}),
            json!({"from": "nowhere", "to": "end"}),
            json!({"from": "start", "to": "n"}),
            json!({"from": "start", "to": "start"}),
            json!({"from": "start"}),
            json!([1, 2]),
        ] {
            assert!(with_points(params.clone(), g.clone()).is_err(), "{g}");
        }
    }

    /// Chromatic Aberration-like: a pixel shift, a pixel int, a signed pixel offset and a unitless
    /// `size` (a name the built-in filters treat as pixels).
    const PIXELS: &str = r#"{"id":"org.example.fringe","name":"Fringe","kind":"filter",
        "params":{"amount":{"type":"number","min":0,"max":100,"default":12,"unit":"px"},
                  "radius":{"type":"int","min":2,"max":50,"default":10,"unit":"px"},
                  "shift":{"type":"number","min":-40,"max":40,"default":0,"unit":"px"},
                  "size":{"type":"number","min":0,"max":100,"default":50},
                  "mix":{"type":"number","min":0,"max":1,"default":1,"unit":"none"}}}"#;

    #[test]
    fn parses_units_and_keeps_old_manifests_unitless() {
        let m = Manifest::parse(PIXELS.as_bytes()).unwrap();
        assert!(m.is_pixel_param("amount") && m.is_pixel_param("radius") && m.is_pixel_param("shift"));
        assert!(!m.is_pixel_param("size") && !m.is_pixel_param("mix") && !m.is_pixel_param("missing"));
        assert!(matches!(m.params[0].1, ParamSpec::Number { unit: Unit::Px, .. }));
        assert!(matches!(m.params[1].1, ParamSpec::Int { unit: Unit::Px, .. }));
        // No `unit` is unitless, and unitless params serialize as before (no `unit` key).
        let old = Manifest::parse(FULL.as_bytes()).unwrap();
        assert!(old.params.iter().all(|(k, _)| !old.is_pixel_param(k)));
        let schema = serde_json::to_value(&m).unwrap();
        assert_eq!(schema["params"]["amount"]["unit"], "px");
        assert!(schema["params"]["size"].get("unit").is_none() && schema["params"]["mix"].get("unit").is_none());
        assert_eq!(Manifest::parse(serde_json::to_string(&m).unwrap().as_bytes()).unwrap(), m);
        // A unit is a `number`/`int` property; the dialog notation is unchanged by it.
        assert_eq!(m.params_notation(), r#"{"amount":0..100=12,"radius":int=10,"shift":-40..40=0,"size":0..100=50,"mix":0..1=1}"#);
    }

    #[test]
    fn unknown_units_are_unitless() {
        // Unknown fields were always ignored, so a manifest may already carry any `unit`: it must
        // keep installing, unscaled, exactly as when the field is absent.
        for unit in [r#""pt""#, r#""PX""#, r#""""#, "5", "true", "null", r#"["px"]"#, r#"{"px":1}"#] {
            let text = format!(r#"{{"id":"a","name":"x","kind":"filter","params":{{"r":{{"type":"number","min":0,"max":9,"default":1,"unit":{unit}}}}}}}"#);
            for text in [text.clone(), text.replace(r#""type":"number""#, r#""type":"int""#)] {
                let m = Manifest::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{text}: {e}"));
                assert!(!m.is_pixel_param("r"), "{text}");
                let mut p = json!({"r": 8}).as_object().unwrap().clone();
                m.scale_for_preview(&mut p, 4);
                assert_eq!(Value::Object(p), json!({"r": 8}), "{text}");
                assert!(serde_json::to_value(&m).unwrap()["params"]["r"].get("unit").is_none(), "{text}");
            }
        }
    }

    #[test]
    fn point_values_are_clamped_and_type_checked() {
        let m = with_points(json!({"start": {"type": "point", "default": [12, 12]}}), Value::Null).unwrap();
        let get = |v: Value| m.resolve_params(&json!({"start": v})).map(|p| p["start"].clone());
        assert_eq!(get(Value::Null).unwrap(), json!([12.0, 12.0]));
        assert_eq!(get(json!([33.25, 70])).unwrap(), json!([33.25, 70.0]), "in range: passed through unchanged");
        assert_eq!(get(json!([-5, 250])).unwrap(), json!([0.0, 100.0]));
        for bad in [json!([1]), json!([1, 2, 3]), json!(["1", 2]), json!(5), json!({"x": 1, "y": 2})] {
            assert!(get(bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(clamp_point(150.0, -0.5), Some([100.0, 0.0]));
        assert_eq!(clamp_point(f64::NAN, 1.0), None);
        assert_eq!(clamp_point(1.0, f64::INFINITY), None);
    }

    fn with_color(default: Value) -> Result<Manifest> {
        Manifest::parse(
            json!({"id": "org.example.bolt", "name": "Bolt", "kind": "filter", "params": {"inner": {"type": "color", "default": default}}})
                .to_string()
                .as_bytes(),
        )
    }

    #[test]
    fn color_params_validate_their_default() {
        let m = with_color(json!("#CFEAff")).unwrap();
        assert_eq!(m.params[0].1, ParamSpec::Color { default: "#CFEAff".into() });
        assert_eq!(m.params_notation(), r##"{"inner":color=#cfeaff}"##);
        assert!(with_color(json!("#000000")).is_ok());
        for bad in [
            json!("#cfeafz"),
            json!("cfeaff"),
            json!("#cfeaf"),
            json!("#cfeaff0"),
            json!("#cfeaffcc"),
            json!("#fff"),
            json!(""),
            json!("#"),
            json!(" #cfeaff"),
            json!("#cféaf"),
        ] {
            let e = with_color(bad.clone()).unwrap_err().to_string();
            assert!(e.contains("inner") && e.contains("#rrggbb"), "{bad}: {e}");
        }
        for bad in [json!(0xcfeaff), json!(null), json!([207, 234, 255]), json!(true), json!({"r": 1})] {
            assert!(with_color(bad.clone()).is_err(), "{bad}");
        }
        assert!(Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","params":{"c":{"type":"color"}}}"#).is_err(), "a colour needs a default");
    }

    #[test]
    fn color_values_pass_through_as_lowercase_hex() {
        let m = with_color(json!("#CFEAFF")).unwrap();
        let get = |v: Value| m.resolve_params(&json!({"inner": v})).map(|p| p["inner"].clone());
        assert_eq!(get(Value::Null).unwrap(), json!("#cfeaff"), "the default, lowercased");
        assert_eq!(get(json!("#ff8800")).unwrap(), json!("#ff8800"), "passed through unchanged");
        assert_eq!(get(json!("#FF8800")).unwrap(), json!("#ff8800"));
        for bad in [json!("ff8800"), json!("#ff880"), json!("#ff8800ff"), json!("#gg8800"), json!(16746496), json!([255, 136, 0])] {
            assert!(get(bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(normalize_color("#AbCdEf").as_deref(), Some("#abcdef"));
        assert_eq!(normalize_color("#ab"), None);
    }

    fn with_angle(default: Value) -> Result<Manifest> {
        Manifest::parse(
            json!({"id": "org.example.bolt", "name": "Bolt", "kind": "filter", "params": {"direction": {"type": "angle", "default": default, "group": "Bolt"}}})
                .to_string()
                .as_bytes(),
        )
    }

    #[test]
    fn angle_params_validate_their_default() {
        let m = with_angle(json!(45)).unwrap();
        assert_eq!(m.params[0].1, ParamSpec::Angle { default: 45.0 });
        assert_eq!(m.group("direction"), Some("Bolt"));
        assert_eq!(m.params_notation(), r#"{"direction":angle=45}"#);
        for ok in [json!(0), json!(360), json!(12.5)] {
            assert!(with_angle(ok.clone()).is_ok(), "{ok}");
        }
        for bad in [json!(-0.5), json!(360.5), json!(720), json!(-90)] {
            let e = with_angle(bad.clone()).unwrap_err().to_string();
            assert!(e.contains("direction") && e.contains("0-360"), "{bad}: {e}");
        }
        for bad in [json!("45"), json!(null), json!([45]), json!(true), json!({"deg": 45})] {
            assert!(with_angle(bad.clone()).is_err(), "{bad}");
        }
        let no_default = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","params":{"a":{"type":"angle"}}}"#).unwrap();
        assert_eq!(no_default.params[0].1, ParamSpec::Angle { default: 0.0 }, "the default defaults to 0");
    }

    #[test]
    fn angles_wrap_into_0_to_360() {
        for (given, want) in [(0.0, 0.0), (90.0, 90.0), (360.0, 360.0), (370.0, 10.0), (-90.0, 270.0), (720.0, 0.0), (-360.0, 0.0), (45.5, 45.5)] {
            assert_eq!(wrap_angle(given), Some(want), "{given}");
        }
        assert_eq!(wrap_angle(f64::NAN), None);
        assert_eq!(wrap_angle(f64::INFINITY), None);
    }

    #[test]
    fn angle_values_pass_through_params() {
        let m = with_angle(json!(45)).unwrap();
        let get = |v: Value| m.resolve_params(&json!({"direction": v})).map(|p| p["direction"].clone());
        assert_eq!(get(Value::Null).unwrap(), json!(45.0), "the default");
        for a in [0.0, 90.0, 137.5, 270.0, 360.0] {
            assert_eq!(get(json!(a)).unwrap(), json!(a), "passed through unchanged");
        }
        assert_eq!(get(json!(370)).unwrap(), json!(10.0));
        assert_eq!(get(json!(-90)).unwrap(), json!(270.0));
        for bad in [json!("90"), json!([90]), json!(true), json!({"deg": 90})] {
            let e = get(bad.clone()).unwrap_err().to_string();
            assert!(e.contains("direction"), "{bad}: {e}");
        }
    }

    #[test]
    fn preview_scales_pixel_params_only() {
        // #7524: at a reduction of 4, a 12 px shift must be 3 proxy pixels (12 document pixels).
        let m = Manifest::parse(PIXELS.as_bytes()).unwrap();
        let given = json!({"id": "org.example.fringe", "amount": 12, "radius": 10, "shift": -20, "size": 40, "mix": 0.5});
        let mut p = given.as_object().unwrap().clone();
        m.scale_for_preview(&mut p, 4);
        assert_eq!(
            Value::Object(p.clone()),
            json!({"id": "org.example.fringe", "amount": 3.0, "radius": 3, "shift": -5.0, "size": 40, "mix": 0.5}),
            "px params divided (ints rounded, signs kept); unitless and other keys untouched"
        );
        // The scaled values are what the plug-in receives.
        assert_eq!(m.resolve_params(&Value::Object(p)).unwrap()["amount"], json!(3.0));
        // Missing px params get their scaled default, so the default isn't previewed unscaled.
        let mut p = Map::new();
        m.scale_for_preview(&mut p, 2);
        assert_eq!(Value::Object(p), json!({"amount": 6.0, "radius": 5, "shift": 0.0}));
        // Never below the minimum, and values are clamped into range first, as `resolve_params` does.
        let mut p = json!({"radius": 3, "amount": 1000}).as_object().unwrap().clone();
        m.scale_for_preview(&mut p, 4);
        assert_eq!(Value::Object(p), json!({"radius": 2, "amount": 25.0, "shift": 0.0}));
        // A full-resolution preview changes nothing.
        for k in [0, 1] {
            let mut p = given.as_object().unwrap().clone();
            m.scale_for_preview(&mut p, k);
            assert_eq!(Value::Object(p), given);
        }
        // Wrong types and non-numbers are left for `resolve_params` to reject, never a panic.
        let mut p = json!({"amount": "lots", "radius": true, "shift": [1]}).as_object().unwrap().clone();
        m.scale_for_preview(&mut p, 4);
        assert_eq!(Value::Object(p.clone()), json!({"amount": "lots", "radius": true, "shift": [1]}));
        assert!(m.resolve_params(&Value::Object(p)).is_err());
        // A plug-in without px params is never touched, even with built-in pixel names.
        let old = Manifest::parse(br#"{"id":"a","name":"x","kind":"filter","params":{"radius":{"type":"number","min":0,"max":9,"default":4}}}"#).unwrap();
        let mut p = json!({"radius": 8}).as_object().unwrap().clone();
        old.scale_for_preview(&mut p, 4);
        assert_eq!(Value::Object(p), json!({"radius": 8}));
    }

    #[test]
    fn preview_factor_keeps_pixel_params_above_their_minimum() {
        let m = Manifest::parse(PIXELS.as_bytes()).unwrap();
        let p = |v: Value| v.as_object().unwrap().clone();
        // radius min 2: 10 px allows 4, 5 px allows 2, 3 px only full resolution.
        assert_eq!(m.preview_factor(&p(json!({"radius": 10})), 4), 4);
        assert_eq!(m.preview_factor(&p(json!({"radius": 5})), 4), 2);
        assert_eq!(m.preview_factor(&p(json!({"radius": 3})), 4), 1);
        // Missing: the default (10). Params with a zero or negative minimum don't limit it.
        assert_eq!(m.preview_factor(&Map::new(), 8), 5);
        assert_eq!(m.preview_factor(&p(json!({"radius": 50, "amount": 0, "shift": -40})), 8), 8);
        // Never more than the document's factor, never 0; bad values don't limit it.
        assert_eq!(m.preview_factor(&p(json!({"radius": 50})), 2), 2);
        assert_eq!(m.preview_factor(&p(json!({"radius": 50})), 0), 1);
        assert_eq!(m.preview_factor(&p(json!({"radius": "x"})), 4), 4);
        let unitless = Manifest::parse(FULL.as_bytes()).unwrap();
        assert_eq!(unitless.preview_factor(&p(json!({"amount": 0.01})), 4), 4);
    }

    fn with_name(name: &str) -> Result<Manifest> {
        Manifest::parse(json!({"id": "org.example.tone", "name": name, "kind": "filter"}).to_string().as_bytes())
    }

    #[test]
    fn name_must_be_printable_unicode() {
        for bad in [
            "Tone\u{202E}evil",
            "\u{2067}Tone\u{2069}",
            "Tone\u{200F}",
            "\u{200B}\u{200B}\u{200B}",
            "Tone\u{00AD}",
            "\u{FEFF}Tone",
            "\u{200D}",
            "\u{3164}\u{FE0F}",
            " \u{3000} ",
            "Tone\u{7}",
        ] {
            assert!(with_name(bad).is_err(), "{bad:?}");
        }
        // Unicode names still install, with joiners where the script or emoji needs them.
        for good in ["Tone…", "Kontrast ±", "می\u{200C}خواهم", "\u{1F469}\u{200D}\u{1F4BB} Code", "\u{2764}\u{FE0F} Love", "色调"] {
            assert_eq!(with_name(good).map(|m| m.name).ok().as_deref(), Some(good));
        }
        // The documented limit counts characters, not bytes.
        assert!(with_name(&"é".repeat(64)).is_ok());
        assert!(with_name(&"é".repeat(65)).is_err());
    }

    fn with_group(group: Value) -> Result<Manifest> {
        let params = json!({"amount": {"type": "number", "min": 0, "max": 1, "default": 0.5, "group": group}});
        Manifest::parse(json!({"id": "org.example.tone", "name": "Tone", "kind": "filter", "params": params}).to_string().as_bytes())
    }

    #[test]
    fn param_groups_follow_the_menu_label_rules() {
        assert_eq!(with_group(json!("Glow")).unwrap().group("amount"), Some("Glow"));
        assert_eq!(with_group(json!("  Glow ")).unwrap().group("amount"), Some("Glow"), "trimmed like the name");
        assert_eq!(with_group(json!("色调 ±")).unwrap().group("amount"), Some("色调 ±"));
        assert!(with_group(json!("é".repeat(MAX_GROUP_CHARS))).is_ok());
        assert!(with_group(Value::Null).unwrap().group("amount").is_none(), "null is no group");
        for bad in [
            json!(""),
            json!("   "),
            json!("é".repeat(MAX_GROUP_CHARS + 1)),
            json!("Glow\u{7}"),
            json!("Glow\n2"),
            json!("Gl\tow"),
            json!("\u{202E}Glow"),
            json!("\u{200B}"),
        ] {
            let e = with_group(bad.clone()).unwrap_err().to_string();
            assert!(e.contains("amount") && e.contains("group"), "{bad}: {e}");
        }
        for bad in [json!(3), json!(true), json!(["Glow"]), json!({"name": "Glow"})] {
            assert!(with_group(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn tabs_follow_first_appearance_and_ungrouped_params_stay_visible() {
        let m = Manifest::parse(
            br##"{"id":"org.example.bolt","name":"Bolt","kind":"filter","params":{
                "effect":{"type":"choice","options":["a","b"]},
                "glow":{"type":"number","min":0,"max":1,"default":0.5,"group":"Glow"},
                "start":{"type":"point","default":[12,12],"group":"Bolt"},
                "seed":{"type":"int","min":0,"max":9,"group":"Bolt"},
                "inner":{"type":"color","default":"#ffffff","group":"Colour"},
                "width":{"type":"number","min":0,"max":1,"default":0.5,"group":"Glow"},
                "mono":{"type":"bool"}}}"##,
        )
        .unwrap();
        assert_eq!(m.tabs(), ["Glow", "Bolt", "Colour"]);
        assert_eq!(m.always_visible(), ["effect", "mono"]);
        assert_eq!((m.group("width"), m.group("effect"), m.group("missing")), (Some("Glow"), None, None));
        // Groups don't change the values or the notation.
        assert_eq!(
            m.params_notation(),
            r##"{"effect":"a|b","glow":0..1=0.5,"start":point=[12,12],"seed":int=0,"inner":color=#ffffff,"width":0..1=0.5,"mono":bool=false}"##
        );
        let plain = Manifest::parse(FULL.as_bytes()).unwrap();
        assert!(plain.tabs().is_empty());
        assert_eq!(plain.always_visible(), ["amount", "mode", "mono", "seed"]);
    }
}
