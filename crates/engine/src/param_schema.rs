//! The Parameter schema: a typed description of a Command's input parameters — key, kind,
//! bounds, unit, default — as data that the dialog, the agent docs and dispatch-time
//! validation can share instead of re-deriving from the params string each.
//!
//! `render()` produces the human/agent-readable Display string documented in
//! `docs/control-protocol.md` (`{"radius":0.1..1000=1}`) from a schema, so the published
//! string is derived from one description rather than maintained beside it. `Kind::Free`
//! renders a verbatim type word (`id`, `index`) where a command's doc names one without
//! bounds; `UNIVERSAL` holds the keys every command accepts regardless of schema.

use std::fmt::Write as _;

/// One unit-suffixed numeric or string parameter's unit. Only the four suffixes that
/// appear as value types in registry docs; anything else is prose or `Json`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Px,
    Pt,
    Percent,
    Deg,
}

impl Unit {
    fn word(self) -> &'static str {
        match self {
            Unit::Px => "px",
            Unit::Pt => "pt",
            Unit::Percent => "%",
            Unit::Deg => "deg",
        }
    }
}

/// A parameter's declared default. `Symbol` names UI state (`foreground`, `bottom selected`);
/// the engine never interprets it, only the dialog resolves it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Default {
    Num(f64),
    Bool(bool),
    Str(&'static str),
    Choice(&'static str),
    Symbol(&'static str),
}

/// The shape of one accepted value. `Object`/`Array` nest to exactly the depth the command's
/// doc describes; structurally unclear payloads stay `Json`. `Alt` accepts any member.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Range {
        int: bool,
        min: f64,
        max: f64,
    },
    Bool,
    Text,
    Choice(&'static [&'static str]),
    Color,
    Json,
    Object(&'static [Param]),
    Array(&'static Kind),
    Alt(&'static [Kind]),
    /// Transitional verbatim type word (`id`, `index`, `u32`, `[x,y]`): renders as written.
    Free(&'static str),
}

/// One accepted parameter: key, shape, optionality, unit, default.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Param {
    pub key: &'static str,
    pub kind: Kind,
    pub optional: bool,
    pub unit: Option<Unit>,
    pub default: Option<Default>,
}

impl Param {
    pub const fn required(key: &'static str, kind: Kind) -> Param {
        Param { key, kind, optional: false, unit: None, default: None }
    }

    pub const fn optional(key: &'static str, kind: Kind) -> Param {
        Param { key, kind, optional: true, unit: None, default: None }
    }

    pub const fn with_default(mut self, default: Default) -> Param {
        self.default = Some(default);
        self
    }

    pub const fn with_unit(mut self, unit: Unit) -> Param {
        self.unit = Some(unit);
        self
    }

    pub const fn as_optional(mut self) -> Param {
        self.optional = true;
        self
    }

    pub const fn range(key: &'static str, int: bool, min: f64, max: f64) -> Param {
        Param { key, kind: Kind::Range { int, min, max }, optional: false, unit: None, default: None }
    }
}

/// A command's full input description: entries, verbatim notes (prose and `→ {result}`
/// annotations, outside the checked set), and whether unknown keys are accepted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Params {
    pub entries: &'static [Param],
    pub notes: &'static str,
    pub open: bool,
}

/// Keys every command accepts regardless of its schema (target shims included).
/// Specs never redeclare these; the renderer and the checker share this one list.
pub const UNIVERSAL: &[Param] = &[
    Param::optional("coalesce", Kind::Bool),
    Param::optional("target", Kind::Text),
    Param::optional("layer", Kind::Text),
    Param::optional("channel", Kind::Text),
];

impl Params {
    pub const fn closed(entries: &'static [Param]) -> Params {
        Params { entries, notes: "", open: false }
    }

    pub const fn with_notes(mut self, notes: &'static str) -> Params {
        self.notes = notes;
        self
    }

    pub const fn opened(mut self) -> Params {
        self.open = true;
        self
    }

    /// Composition: this command's entries extended by a shared static slice, at spec-build.
    /// Leaked once per composed command, like the registry's other built `'static` data.
    pub fn concat(base: &'static [Param], extra: &'static [Param], notes: &'static str) -> Params {
        let mut all = Vec::with_capacity(base.len() + extra.len());
        all.extend_from_slice(base);
        all.extend_from_slice(extra);
        Params { entries: Box::leak(all.into_boxed_slice()), notes, open: false }
    }

    /// The Display string: the locked agent/human format. Conformance target.
    pub fn render(&self) -> String {
        let mut out = String::new();
        render_into(&mut out, self.entries, self.open, self.notes);
        out
    }

    /// The Display string with the Universal keys appended. Deduplicated: a spec declaring
    /// one of them (`layer`, `target`) keeps its own form.
    pub fn render_with_universal(&self) -> String {
        let mut all: Vec<Param> = self.entries.to_vec();
        for u in UNIVERSAL {
            if !all.iter().any(|p| p.key == u.key) {
                all.push(*u);
            }
        }
        let mut out = String::new();
        render_into(&mut out, &all, self.open, self.notes);
        out
    }
}

fn render_into(out: &mut String, entries: &[Param], open: bool, notes: &str) {
    out.push('{');
    render_entries(out, entries);
    if open {
        if !entries.is_empty() {
            out.push_str(", …");
        } else {
            out.push('…');
        }
    }
    out.push('}');
    out.push_str(notes);
}

fn render_entries(out: &mut String, entries: &[Param]) {
    for (i, p) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{}\":", p.key);
        render_kind(out, &p.kind);
        if let Some(u) = p.unit {
            let _ = write!(out, " {}", u.word());
        }
        if p.optional {
            out.push('?');
        }
        render_default(out, p.default.as_ref());
    }
}

fn render_kind(out: &mut String, kind: &Kind) {
    match kind {
        Kind::Range { min, max, .. } => {
            let _ = write!(out, "{min}..{max}");
        }
        Kind::Bool => out.push_str("bool"),
        Kind::Text => out.push_str("text"),
        Kind::Color => out.push_str("color"),
        Kind::Json => out.push_str("json"),
        Kind::Choice(options) => {
            out.push('"');
            for (i, o) in options.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                out.push_str(o);
            }
            out.push('"');
        }
        Kind::Object(entries) => {
            out.push('{');
            render_entries(out, entries);
            out.push('}');
        }
        Kind::Array(elem) => {
            out.push('[');
            render_kind(out, elem);
            out.push(']');
        }
        Kind::Alt(members) => {
            for (i, m) in members.iter().enumerate() {
                if i > 0 {
                    out.push('|');
                }
                render_kind(out, m);
            }
        }
        Kind::Free(w) => out.push_str(w),
    }
}

fn render_default(out: &mut String, default: Option<&Default>) {
    match default {
        Some(Default::Num(v)) => {
            let _ = write!(out, "={v}");
        }
        Some(Default::Bool(v)) => out.push_str(if *v { "=true" } else { "=false" }),
        Some(Default::Str(s)) | Some(Default::Choice(s)) => {
            let _ = write!(out, "=\"{s}\"");
        }
        Some(Default::Symbol(s)) => {
            let _ = write!(out, "={s}");
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_with_default_renders_the_established_format() {
        static P: [Param; 1] = [Param::range("radius", false, 0.1, 1000.0).with_default(Default::Num(1.0))];
        assert_eq!(Params::closed(&P).render(), r##"{"radius":0.1..1000=1}"##);
    }

    #[test]
    fn choice_renders_quoted_alternatives_and_default() {
        static M: [Param; 1] = [Param::required("method", Kind::Choice(&["spin", "zoom"]))];
        static Q: [Param; 1] = [Param::required("quality", Kind::Choice(&["draft", "good", "best"])).with_default(Default::Choice("good"))];
        assert_eq!(Params::closed(&M).render(), r##"{"method":"spin|zoom"}"##);
        assert_eq!(Params::closed(&Q).render(), r##"{"quality":"draft|good|best"="good"}"##);
    }

    #[test]
    fn unit_and_optional_markers_render_after_the_value() {
        static P: [Param; 3] = [
            Param::range("size", false, 0.5, 5000.0).with_unit(Unit::Px).as_optional(),
            Param::optional("layer", Kind::Free("id")),
            Param::required("legacy", Kind::Bool).with_default(Default::Bool(false)),
        ];
        assert_eq!(Params::closed(&P).render(), r##"{"size":0.5..5000 px?,"layer":id?,"legacy":bool=false}"##);
    }

    #[test]
    fn nested_object_notes_and_open_marker_render() {
        static INNER: [Param; 2] = [Param::range("in", true, 0.0, 255.0), Param::range("out", true, 0.0, 255.0)];
        static CHANNEL: Param = Param::required("channel", Kind::Object(&INNER));
        static OUTER: [Param; 1] = [CHANNEL];
        assert_eq!(Params::closed(&OUTER).render(), r##"{"channel":{"in":0..255,"out":0..255}}"##);
        static G: [Param; 1] = [Param::range("gamma", false, 0.01, 9.99).with_default(Default::Num(1.0))];
        let with_notes = Params::closed(&G).with_notes(" (per channel)");
        assert_eq!(with_notes.render(), r##"{"gamma":0.01..9.99=1} (per channel)"##);
        let opened = Params::closed(&G).opened();
        assert_eq!(opened.render(), r##"{"gamma":0.01..9.99=1, …}"##);
    }

    #[test]
    fn array_and_alt_render_members_by_kind() {
        static A: [Param; 1] = [Param::required("values", Kind::Array(&Kind::Range { int: true, min: 0.0, max: 255.0 }))];
        assert_eq!(Params::closed(&A).render(), r##"{"values":[0..255]}"##);
        static SUB: [Param; 1] = [Param::required("channel", Kind::Free("i"))];
        static ALT: [Kind; 3] = [Kind::Choice(&["pixels"]), Kind::Choice(&["mask"]), Kind::Object(&SUB)];
        static T: [Param; 1] = [Param::required("target", Kind::Alt(&ALT))];
        assert_eq!(Params::closed(&T).render(), r##"{"target":"pixels"|"mask"|{"channel":i}}"##);
    }

    #[test]
    fn concat_merges_static_slices_at_build() {
        static BASE: [Param; 1] = [Param::required("x", Kind::Free("i"))];
        static PATH: [Param; 1] = [Param::optional("path", Kind::Choice(&["work", "saved"]))];
        let merged = Params::concat(&BASE, &PATH, " (new layer)");
        assert_eq!(merged.render(), r##"{"x":i,"path":"work|saved"?} (new layer)"##);
    }

    #[test]
    fn symbol_default_renders_unquoted() {
        static P: [Param; 1] = [Param::required("fill", Kind::Choice(&["foreground", "background"])).with_default(Default::Symbol("foreground"))];
        assert_eq!(Params::closed(&P).render(), r##"{"fill":"foreground|background"=foreground}"##);
    }

    #[test]
    fn universal_tail_merges_without_duplicates() {
        static A: [Param; 1] = [Param::required("invert", Kind::Bool)];
        assert_eq!(Params::closed(&A).render_with_universal(), r##"{"invert":bool,"coalesce":bool?,"target":text?,"layer":text?,"channel":text?}"##);
        static WITH_LAYER: [Param; 1] = [Param::optional("layer", Kind::Free("id"))];
        assert_eq!(Params::closed(&WITH_LAYER).render_with_universal(), r##"{"layer":id?,"coalesce":bool?,"target":text?,"channel":text?}"##);
    }
}
