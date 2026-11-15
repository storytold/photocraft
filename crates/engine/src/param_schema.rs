//! The parameter schema: a typed description of a command's input parameters, read from the
//! registry's `params` doc string (`{"radius":0.1..1000=1}`, the format `docs/control-protocol.md`
//! documents) by one parser, and rendered back to exactly that string.
//!
//! Dispatch (unknown keys, #7596), filter and adjustment value validation (#7597) and the
//! filter dialogs (#7598) all read this one description instead of scanning the string
//! themselves. A value the grammar can't type is kept as [`Kind::Free`] prose, which every
//! reader treats as "anything": the parser never guesses a number or a bound. A test holds
//! [`Params::render`] to reproduce every registry doc byte for byte, so nothing in a doc is
//! dropped or reinterpreted on the way in.
//!
//! Design after #7599 by @SckraT.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

use serde_json::Value;

use crate::commands::CommandSpec;

/// Keys every command accepts whatever its doc says: the history-coalescing key
/// (`Session::execute`, a string) and the Channels-panel `"target"` pixel commands follow.
/// Command-specific keys such as `layer` and `channel` belong in the command's own doc.
pub const UNIVERSAL: &[&str] = &["coalesce", "target"];

/// The parsed doc of a registered command, read once per process. `None` for a doc that does
/// not follow the grammar (a test lists those).
pub fn of(spec: &CommandSpec) -> Option<&'static Params> {
    static SCHEMAS: OnceLock<HashMap<&'static str, Option<Params>>> = OnceLock::new();
    let schemas = SCHEMAS.get_or_init(|| {
        let mut m = HashMap::new();
        for c in crate::commands::command_specs() {
            m.entry(c.id).or_insert_with(|| Params::parse(c.params));
        }
        m
    });
    match schemas.get(spec.id) {
        // The registered spec (the common case), or another spec with the same id and doc.
        Some(p) if crate::commands::find(spec.id).is_some_and(|c| c.params == spec.params) => p.as_ref(),
        _ => None,
    }
}

/// A type word's meaning (the word itself is kept as written: `str`, `string`, `text`…).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Bool,
    Text,
    /// Any finite number (`number`, `f64`, or a bare unit like `px`).
    Number,
    /// An integer (`int`, `i32`, `index`…).
    Int,
    /// An unsigned 32-bit integer (`u32`, a seed).
    U32,
    /// A layer, document or other object reference (`id`).
    Id,
    Color,
    Json,
    /// One of the open documents (`doc`).
    Doc,
    /// `null` (as one alternative: "reset to the default").
    Null,
}

impl Ty {
    fn of(word: &str) -> Option<Ty> {
        Some(match word {
            "bool" | "boolean" | "true" | "false" => Ty::Bool,
            "text" | "str" | "string" | "name" | "path" | "folder" | "hex" | "base64" => Ty::Text,
            "number" | "num" | "float" | "f32" | "f64" | "px" | "pt" | "deg" | "degrees" | "%" | "ppi" | "mm" | "in" => Ty::Number,
            "int" | "integer" | "i32" | "i64" | "u8" | "u16" | "u64" | "usize" | "uint" | "index" | "i" | "j" | "n" | "char" | "row" | "len" => Ty::Int,
            "u32" => Ty::U32,
            "id" => Ty::Id,
            "color" | "colour" => Ty::Color,
            "json" => Ty::Json,
            "doc" => Ty::Doc,
            "null" => Ty::Null,
            _ => return None,
        })
    }
}

/// One alternative of a value (`a|b|c` lists several).
#[derive(Clone, PartialEq, Debug)]
pub enum Kind {
    /// `min..max`, inclusive (`min..` has no maximum). `src` is the bounds as written.
    Range { min: f64, max: f64, src: String },
    /// A literal number (`8` in `8|16|32`).
    Number { value: f64, src: String },
    /// `"a|b|c"`: one of these strings. A `…` member leaves the list open.
    Choice(Vec<String>),
    /// A type word.
    Type { ty: Ty, word: String },
    /// `int[25]`: an array of exactly `len` values.
    Sized { ty: Ty, word: String, len: usize },
    /// A nested object.
    Object(Object),
    /// `[…]`, kept as written.
    Array(String),
    /// Prose the grammar does not type (`blend mode name`, `built-in id`): any value.
    Free(String),
}

/// What follows a value's alternatives, in the order written.
#[derive(Clone, PartialEq, Debug)]
pub enum Suffix {
    /// ` px`, ` %`: the unit of a range or number (leading space included).
    Unit(String),
    /// `?`: the key may be left out.
    Optional,
    /// `=…`: the default, as written and as read.
    Default(String, DefaultValue),
    /// Prose after the value (` (pixel counts are rounded)`), leading space included.
    Note(String),
}

/// A declared default. `Symbol` names UI state (`foreground`, `tool size`) only a dialog
/// resolves; the engine never interprets it.
#[derive(Clone, PartialEq, Debug)]
pub enum DefaultValue {
    Num(f64),
    Bool(bool),
    Str(String),
    Symbol(String),
}

/// One documented parameter. `keys` is usually one key; `"all","pixels":bool` names two that
/// share a value and `"quad"|"corners":…` two spellings of one. A key with no `:value` (`"stops"?`)
/// takes any value.
#[derive(Clone, PartialEq, Debug)]
pub struct Param {
    /// The text before it in its object: `""`, `","`, `", "` or ` or ` (an alternative spelling).
    pub sep: String,
    pub keys: Vec<String>,
    /// The keys are spellings of one key (`"a"|"b"`) rather than a list (`"a","b"`).
    pub keys_alt: bool,
    /// `"key"?:value`: the optional marker written before the colon.
    pub key_optional: bool,
    /// The alternatives the value takes; empty for a key written without `:value`.
    pub kinds: Vec<Kind>,
    pub suffixes: Vec<Suffix>,
}

impl Param {
    pub fn key(&self) -> &str {
        self.keys.last().map_or("", String::as_str)
    }

    pub fn optional(&self) -> bool {
        self.key_optional || self.suffixes.contains(&Suffix::Optional)
    }

    pub fn unit(&self) -> Option<&str> {
        self.suffixes.iter().find_map(|s| if let Suffix::Unit(u) = s { Some(u.trim()) } else { None })
    }

    pub fn default(&self) -> Option<&DefaultValue> {
        self.suffixes.iter().find_map(|s| if let Suffix::Default(_, d) = s { Some(d) } else { None })
    }

    /// Whether this param is an alternative spelling of the one before it (`"layer":id? or "layers":[id,…]?`).
    pub fn is_alternative(&self) -> bool {
        self.sep.trim() == "or"
    }

    /// The single numeric range this param documents, if that is all it accepts.
    pub fn range(&self) -> Option<(f64, f64)> {
        match self.kinds.as_slice() {
            [Kind::Range { min, max, .. }] => Some((*min, *max)),
            _ => None,
        }
    }
}

/// One item of an object: a parameter, or the `…` that leaves the object open.
#[derive(Clone, PartialEq, Debug)]
pub enum Item {
    Param(Param),
    /// `…` / `...` with any prose after it (`…params of that kind`), separator included.
    Open(String),
}

/// A `{…}` of parameters.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Object {
    pub items: Vec<Item>,
}

impl Object {
    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.items.iter().filter_map(|i| if let Item::Param(p) = i { Some(p) } else { None })
    }

    /// Whether keys beyond the listed ones are accepted (the object has a `…`).
    pub fn open(&self) -> bool {
        self.items.iter().any(|i| matches!(i, Item::Open(_)))
    }
}

/// A command's whole `params` doc: its input object, objects composed onto it with ` + {…}`
/// (a new adjustment layer's options), and the notes after them (prose, `→ {result}`).
#[derive(Clone, PartialEq, Debug)]
pub struct Params {
    /// The input objects with the text before each (`""` for the first, ` + ` after).
    pub objects: Vec<(String, Object)>,
    /// Everything after the input objects, verbatim. Not parameters.
    pub notes: String,
}

impl Params {
    /// Read a registry `params` doc. `None` when its input object doesn't follow the grammar.
    pub fn parse(doc: &str) -> Option<Params> {
        let (first, mut rest) = object_at(doc)?;
        let mut objects = vec![(String::new(), first)];
        loop {
            let t = rest.trim_start();
            let Some(after) = t.strip_prefix('+') else { break };
            let lead_len = rest.len() - after.len();
            let ws = after.len() - after.trim_start().len();
            let Some((obj, r)) = object_at(&after[ws..]) else { break };
            objects.push((rest[..lead_len + ws].to_string(), obj));
            rest = r;
        }
        Some(Params { objects, notes: rest.to_string() })
    }

    /// Every documented input parameter, composed objects included (top level only).
    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.objects.iter().flat_map(|(_, o)| o.params())
    }

    /// The parameters of the command's own object (without composed ones).
    pub fn own(&self) -> &Object {
        &self.objects[0].1
    }

    /// Whether the doc leaves its keys open (a top-level `…`).
    pub fn open(&self) -> bool {
        self.objects.iter().any(|(_, o)| o.open())
    }

    /// Whether `key` is a documented input key.
    pub fn has_key(&self, key: &str) -> bool {
        self.param(key).is_some()
    }

    /// The documented param named `key` (`"part/<name>"` names every `part/…` key).
    pub fn param(&self, key: &str) -> Option<&Param> {
        self.params().find(|p| p.keys.iter().any(|k| key_matches(k, key)))
    }

    /// The documented input keys, for messages.
    pub fn keys(&self) -> Vec<&str> {
        self.params().flat_map(|p| p.keys.iter().map(String::as_str)).collect()
    }

    /// The doc string: the format agents and `command.list` read.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (lead, o) in &self.objects {
            out.push_str(lead);
            render_object(&mut out, o);
        }
        out.push_str(&self.notes);
        out
    }
}

fn key_matches(documented: &str, key: &str) -> bool {
    match documented.split_once("/<") {
        Some((family, _)) => key.strip_prefix(family).and_then(|r| r.strip_prefix('/')).is_some_and(|r| !r.is_empty()),
        None => documented == key,
    }
}

impl Kind {
    /// Whether `v` is a value of this alternative. Shapes the grammar leaves untyped (objects,
    /// arrays, references, prose) accept any value: only what the doc states is checked.
    pub fn accepts(&self, v: &Value) -> bool {
        match self {
            Kind::Range { min, max, .. } => v.as_f64().is_some_and(|n| n.is_finite() && *min <= n && n <= *max),
            Kind::Number { value, .. } => v.as_f64() == Some(*value),
            Kind::Choice(options) => v.as_str().is_some_and(|s| options.iter().any(|o| o == s || pattern_accepts(o, s))),
            Kind::Type { ty, .. } => match ty {
                Ty::Bool => v.is_boolean(),
                Ty::Text => v.is_string(),
                Ty::Number => v.as_f64().is_some_and(f64::is_finite),
                Ty::Int => v.as_i64().is_some() || v.as_f64().is_some_and(|n| n.is_finite() && n.fract() == 0.0),
                Ty::U32 => v.as_f64().is_some_and(|n| n.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&n)),
                Ty::Null => v.is_null(),
                Ty::Id | Ty::Color | Ty::Json | Ty::Doc => true,
            },
            Kind::Sized { .. } | Kind::Array(_) => v.is_array(),
            Kind::Object(_) => v.is_object(),
            Kind::Free(_) => true,
        }
    }

    fn expected(&self) -> String {
        match self {
            Kind::Range { min, max, .. } if max.is_infinite() => format!("a number of at least {min}"),
            Kind::Range { min, max, .. } => format!("a number in {min}..{max}"),
            Kind::Number { src, .. } => src.clone(),
            Kind::Choice(options) => format!("one of {}", options.join("|")),
            Kind::Type { ty: Ty::Bool, .. } => "a boolean".into(),
            Kind::Type { ty: Ty::Text, .. } => "a string".into(),
            Kind::Type { ty: Ty::Number, .. } => "a finite number".into(),
            Kind::Type { ty: Ty::Int, .. } => "an integer".into(),
            Kind::Type { ty: Ty::U32, .. } => format!("an integer in 0..{}", u32::MAX),
            Kind::Type { ty: Ty::Null, .. } => "null".into(),
            other => {
                let mut s = String::new();
                render_kind(&mut s, other);
                s
            }
        }
    }
}

/// A quoted option that stands for a family of strings: `#rrggbb` (a hex colour), `…` (any
/// further name) or a `<placeholder>`.
fn pattern_accepts(option: &str, s: &str) -> bool {
    match option {
        "…" => true,
        "#rrggbb" => s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit()),
        o => o.contains('<'),
    }
}

impl Param {
    /// Whether `v` is a value this param documents: one of its alternatives, or `null` for an
    /// optional one. A key written without a value takes anything.
    pub fn accepts(&self, v: &Value) -> bool {
        self.kinds.is_empty() || (v.is_null() && self.optional()) || self.kinds.iter().any(|k| k.accepts(v))
    }

    /// What [`Self::accepts`] takes, for an error message.
    pub fn expected(&self) -> String {
        self.kinds.iter().map(Kind::expected).collect::<Vec<_>>().join(" or ")
    }
}

fn render_object(out: &mut String, o: &Object) {
    out.push('{');
    for item in &o.items {
        match item {
            Item::Open(t) => out.push_str(t),
            Item::Param(p) => {
                out.push_str(&p.sep);
                for (i, k) in p.keys.iter().enumerate() {
                    if i > 0 {
                        out.push(if p.keys_alt { '|' } else { ',' });
                    }
                    let _ = write!(out, "\"{k}\"");
                }
                if p.key_optional {
                    out.push('?');
                }
                if !p.kinds.is_empty() {
                    out.push(':');
                }
                for (i, k) in p.kinds.iter().enumerate() {
                    if i > 0 {
                        out.push('|');
                    }
                    render_kind(out, k);
                }
                for s in &p.suffixes {
                    match s {
                        Suffix::Unit(u) | Suffix::Note(u) => out.push_str(u),
                        Suffix::Optional => out.push('?'),
                        Suffix::Default(d, _) => {
                            out.push('=');
                            out.push_str(d);
                        }
                    }
                }
            }
        }
    }
    out.push('}');
}

fn render_kind(out: &mut String, k: &Kind) {
    match k {
        Kind::Range { src, .. } | Kind::Number { src, .. } | Kind::Free(src) => out.push_str(src),
        Kind::Choice(options) => {
            out.push('"');
            out.push_str(&options.join("|"));
            out.push('"');
        }
        Kind::Type { word, .. } => out.push_str(word),
        Kind::Sized { word, len, .. } => {
            let _ = write!(out, "{word}[{len}]");
        }
        Kind::Object(o) => render_object(out, o),
        Kind::Array(inner) => {
            out.push('[');
            out.push_str(inner);
            out.push(']');
        }
    }
}

/// The length of the balanced `{…}` / `[…]` / `(…)` group at the start of `s` (quotes respected).
fn group_len(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_str = false;
    for (i, c) in s.char_indices() {
        if in_str {
            in_str = c != '"';
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The object at the start of `s` and the text after it.
fn object_at(s: &str) -> Option<(Object, &str)> {
    if !s.starts_with('{') {
        return None;
    }
    let n = group_len(s)?;
    Some((parse_object(&s[1..n - 1])?, &s[n..]))
}

fn starts_open(s: &str) -> bool {
    s.starts_with('…') || s.starts_with("...")
}

/// Byte offsets where an item separator starts: a `,` or ` or ` outside quotes and groups that
/// is followed by a key (`"`) or the open marker.
fn item_starts(inner: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_str = false;
    for (i, c) in inner.char_indices() {
        if in_str {
            in_str = c != '"';
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth != 0 || in_str {
            continue;
        }
        let rest = &inner[i..];
        let (after, or) = if let Some(r) = rest.strip_prefix(',') {
            (r, false)
        } else if let Some(r) = rest.strip_prefix(" or ") {
            (r, true)
        } else {
            continue;
        };
        let next = after.trim_start();
        let keyed = quoted_key(next).is_some_and(|n| if or { next[n..].starts_with(':') || next[n..].starts_with("?:") } else { true });
        if keyed || (!or && starts_open(next)) {
            out.push(i);
        }
    }
    out
}

fn parse_object(inner: &str) -> Option<Object> {
    let mut items = Vec::new();
    if inner.is_empty() {
        return Some(Object { items });
    }
    let mut bounds = item_starts(inner);
    bounds.insert(0, 0);
    bounds.push(inner.len());
    for w in bounds.windows(2) {
        let text = &inner[w[0]..w[1]];
        // The separator: `,`/` or ` and the spaces after it.
        let body_at = if w[0] == 0 {
            0
        } else {
            let t = text.strip_prefix(',').or_else(|| text.strip_prefix(" or "))?;
            text.len() - t.trim_start().len()
        };
        let (sep, body) = text.split_at(body_at);
        if starts_open(body) {
            items.push(Item::Open(text.to_string()));
            continue;
        }
        items.push(Item::Param(parse_param(sep, body)?));
    }
    Some(Object { items })
}

/// The length of the quoted key at the start of `s`: word characters, and `/<name>` for a key
/// family (`"part/<name>"`).
fn quoted_key(s: &str) -> Option<usize> {
    let r = s.strip_prefix('"')?;
    let end = r.find('"')?;
    let key = &r[..end];
    let word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let ok = match key.split_once('/') {
        None => word(key),
        Some((a, b)) => word(a) && b.strip_prefix('<').and_then(|b| b.strip_suffix('>')).is_some_and(word),
    };
    ok.then_some(end + 2)
}

/// `"key":value`, `"a","b":value`, `"a"|"b":value`, `"key"?:value` or a bare `"key"`.
fn parse_param(sep: &str, body: &str) -> Option<Param> {
    let mut p = Param { sep: sep.to_string(), keys: Vec::new(), keys_alt: false, key_optional: false, kinds: Vec::new(), suffixes: Vec::new() };
    let mut rest = body;
    loop {
        let n = quoted_key(rest)?;
        p.keys.push(rest[1..n - 1].to_string());
        rest = &rest[n..];
        let joiner = if p.keys_alt { '|' } else { ',' };
        if let Some(r) = rest.strip_prefix(joiner).filter(|r| quoted_key(r).is_some()) {
            rest = r;
        } else if p.keys.len() == 1
            && let Some(r) = rest.strip_prefix('|').filter(|r| quoted_key(r).is_some())
        {
            p.keys_alt = true;
            rest = r;
        } else {
            break;
        }
    }
    if rest.starts_with("?:") {
        p.key_optional = true;
        rest = &rest[1..];
    }
    if let Some(v) = rest.strip_prefix(':') {
        (p.kinds, p.suffixes) = parse_value(v);
    } else {
        // A key with no value: only an optional marker and notes may follow.
        p.suffixes = parse_suffixes(rest);
        if p.suffixes.iter().any(|s| matches!(s, Suffix::Unit(_) | Suffix::Default(..))) {
            return None;
        }
    }
    Some(p)
}

const UNITS: &[&str] = &["px", "pt", "%", "deg", "°", "em", "ms", "s", "ppi", "mm", "in", "cm"];

/// Whether `s` (after a token) starts at a token boundary.
fn boundary(s: &str) -> bool {
    s.is_empty() || s.starts_with([' ', '|', '?', '=', ')', '('])
}

/// A number token at the start of `s` (`-1.5`, `1e-3`), its length.
fn number_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    if b.first() == Some(&b'-') || b.first() == Some(&b'+') {
        i += 1;
    }
    let digits_from = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if i == digits_from {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'-' | b'+')) {
            j += 1;
        }
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            i = j;
        }
    }
    s[..i].parse::<f64>().ok().filter(|v| v.is_finite()).map(|_| i)
}

/// One alternative at the start of `s`, and its length.
fn parse_kind(s: &str) -> (Kind, usize) {
    if let Some(r) = s.strip_prefix('"')
        && let Some(end) = r.find('"')
    {
        return (Kind::Choice(r[..end].split('|').map(str::to_string).collect()), end + 2);
    }
    if (s.starts_with('{') || s.starts_with('['))
        && let Some(n) = group_len(s)
    {
        let kind = if s.starts_with('[') {
            Kind::Array(s[1..n - 1].to_string())
        } else {
            parse_object(&s[1..n - 1]).map_or_else(|| Kind::Free(s[..n].to_string()), Kind::Object)
        };
        return (kind, n);
    }
    if let Some(a) = number_len(s) {
        if let Some(r) = s[a..].strip_prefix("..") {
            let min = s[..a].parse().unwrap_or(0.0);
            if let Some(b) = number_len(r)
                && boundary(&r[b..])
            {
                let n = a + 2 + b;
                return (Kind::Range { min, max: r[..b].parse().unwrap_or(0.0), src: s[..n].to_string() }, n);
            }
            if boundary(r) {
                return (Kind::Range { min, max: f64::INFINITY, src: s[..a + 2].to_string() }, a + 2);
            }
        }
        if boundary(&s[a..]) {
            return (Kind::Number { value: s[..a].parse().unwrap_or(0.0), src: s[..a].to_string() }, a);
        }
    }
    let word = if s.starts_with('%') { 1 } else { s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(s.len()) };
    if word > 0
        && let Some(ty) = Ty::of(&s[..word])
    {
        let r = &s[word..];
        if let Some(inner) = r.strip_prefix('[')
            && let Some(end) = inner.find(']')
            && let Ok(len) = inner[..end].parse::<usize>()
            && boundary(&inner[end + 1..])
        {
            return (Kind::Sized { ty, word: s[..word].to_string(), len }, word + end + 2);
        }
        if boundary(r) && !(r.starts_with(' ') && prose_continues(r)) {
            return (Kind::Type { ty, word: s[..word].to_string() }, word);
        }
    }
    // Prose: up to the next `|`, `?`, `=` or `(` outside quotes and groups.
    let mut n = 0;
    let mut in_str = false;
    for (i, c) in s.char_indices() {
        n = i;
        if in_str {
            in_str = c != '"';
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if matches!(c, '|' | '?' | '=' | '(') {
            break;
        }
        n = i + c.len_utf8();
    }
    let n = s[..n].trim_end().len().max(1).min(s.len());
    (Kind::Free(s[..n].to_string()), n)
}

/// After a type word and a space: more words of prose (`blend mode name`), not a unit or a note.
fn prose_continues(r: &str) -> bool {
    let t = r.trim_start();
    !(t.is_empty() || t.starts_with('(') || t.starts_with('?') || t.starts_with('='))
}

fn parse_value(v: &str) -> (Vec<Kind>, Vec<Suffix>) {
    let mut kinds = Vec::new();
    let mut i = 0;
    loop {
        let (k, n) = parse_kind(&v[i..]);
        kinds.push(k);
        i += n;
        if v[i..].starts_with('|') && i + 1 < v.len() {
            i += 1;
        } else {
            break;
        }
    }
    let mut suffixes = Vec::new();
    // A unit right after a numeric value.
    if matches!(kinds.last(), Some(Kind::Range { .. } | Kind::Number { .. })) && v[i..].starts_with(' ') {
        let t = &v[i + 1..];
        if let Some(u) = UNITS.iter().find(|u| t.starts_with(**u) && boundary(&t[u.len()..])) {
            suffixes.push(Suffix::Unit(v[i..i + 1 + u.len()].to_string()));
            i += 1 + u.len();
        }
    }
    suffixes.extend(parse_suffixes(&v[i..]));
    (kinds, suffixes)
}

/// `?`, `=default` and notes, in the order written.
fn parse_suffixes(v: &str) -> Vec<Suffix> {
    let mut suffixes = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let r = &v[i..];
        if r.starts_with('?') {
            suffixes.push(Suffix::Optional);
            i += 1;
        } else if let Some(d) = r.strip_prefix('=') {
            let n = default_len(d);
            suffixes.push(Suffix::Default(d[..n].to_string(), read_default(&d[..n])));
            i += 1 + n;
        } else {
            let ws = r.len() - r.trim_start().len();
            let n = if r[ws..].starts_with('(') { group_len(&r[ws..]).map_or(r.len(), |g| ws + g) } else { r.len() };
            suffixes.push(Suffix::Note(r[..n].to_string()));
            i += n;
        }
    }
    suffixes
}

/// A default runs to a note (` (`) or the end.
fn default_len(d: &str) -> usize {
    if let Some(r) = d.strip_prefix('"')
        && let Some(end) = r.find('"')
    {
        return end + 2;
    }
    let mut in_str = false;
    for (i, c) in d.char_indices() {
        if in_str {
            in_str = c != '"';
            continue;
        }
        if c == '"' {
            in_str = true;
        } else if c == '(' && d[..i].ends_with(' ') {
            return d[..i].trim_end().len();
        }
    }
    d.len()
}

fn read_default(d: &str) -> DefaultValue {
    if let Some(s) = d.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        return DefaultValue::Str(s.to_string());
    }
    match d {
        "true" => DefaultValue::Bool(true),
        "false" => DefaultValue::Bool(false),
        _ => d.parse::<f64>().ok().filter(|v| v.is_finite()).map_or_else(|| DefaultValue::Symbol(d.to_string()), DefaultValue::Num),
    }
}

#[cfg(test)]
mod tests;
