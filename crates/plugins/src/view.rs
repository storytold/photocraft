//! The declarative widget tree a panel draws (`ui.view`).
//!
//! A plug-in describes widgets; the host draws them with egui and `theme::Tokens`. A plug-in cannot
//! choose a colour, font or size, so every panel is on-theme in light and dark for free, and the
//! tree stays readable through `ui.inspect`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Most widgets in one view.
pub const MAX_WIDGETS: usize = 512;
/// Deepest nesting in one view.
pub const MAX_DEPTH: usize = 16;
/// Most options in one `choice`.
pub const MAX_OPTIONS: usize = 256;
/// Longest text a `text` widget may hold.
///
/// A whole chapter of a lettering script goes in one `text` field, so this is generous — a page of
/// dialogue is a few kilobytes and the ABI already bounds one reply to 1 MiB (`ops::MAX_OPS_BYTES`),
/// which is what keeps a panel from filling memory with widgets rather than this.
pub const MAX_TEXT_LENGTH: usize = 65_536;
/// Longest label, title or placeholder.
pub const MAX_LABEL: usize = 256;

/// How a `label` is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Muted,
    Info,
    Warning,
    Error,
}

/// One option of a `choice`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
}

/// One node of the tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Widget {
    /// Children stacked vertically.
    Column {
        children: Vec<Widget>,
    },
    /// Children laid out in a line.
    Row {
        children: Vec<Widget>,
    },
    /// A collapsible group; its collapsed state is kept per plug-in by the host.
    Section {
        title: String,
        #[serde(default)]
        collapsed: bool,
        children: Vec<Widget>,
    },
    /// Plain text, no markup.
    Label {
        text: String,
        #[serde(default)]
        level: Option<Level>,
    },
    /// A prompt field. `multiline` is the script box: a whole chapter lives in one.
    Text {
        id: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        multiline: bool,
        #[serde(default)]
        max_length: Option<usize>,
    },
    /// A number, as a field or (with `slider`) a slider. `step` is honoured.
    Number {
        id: String,
        value: f64,
        min: f64,
        max: f64,
        #[serde(default)]
        step: Option<f64>,
        #[serde(default)]
        slider: bool,
    },
    Toggle {
        id: String,
        value: bool,
        label: String,
    },
    Choice {
        id: String,
        value: String,
        options: Vec<ChoiceOption>,
    },
    /// Sends `ui.click` when pressed.
    Button {
        id: String,
        label: String,
        #[serde(default)]
        primary: bool,
        #[serde(default)]
        enabled: Option<bool>,
    },
    /// A bar inside the panel; `ui.progress` drives the status bar instead.
    Progress {
        fraction: Option<f64>,
    },
    /// A PNG or JPEG blob, decoded off the UI thread.
    Image {
        blob: String,
        #[serde(default)]
        max_height: Option<f64>,
    },
    Separator,
}

impl Widget {
    /// The widget's id, for the widgets that have one.
    pub fn id(&self) -> Option<&str> {
        match self {
            Widget::Text { id, .. } | Widget::Number { id, .. } | Widget::Toggle { id, .. } | Widget::Choice { id, .. } | Widget::Button { id, .. } => Some(id),
            _ => None,
        }
    }

    fn children(&self) -> &[Widget] {
        match self {
            Widget::Column { children } | Widget::Row { children } | Widget::Section { children, .. } => children,
            _ => &[],
        }
    }
}

/// A widget id: `[A-Za-z0-9_.-]{1,64}`, unique within the view.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// The values a view declares for its inputs, in id order.
///
/// `ui.click` and `ui.change` promise "every input's current value"; a panel that declares a field's
/// value in `ui.view` rather than setting it with `ui.set` has still said what that value is.
pub fn input_values(root: &Widget) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    collect_values(root, 0, &mut out);
    out
}

fn collect_values(w: &Widget, depth: usize, out: &mut BTreeMap<String, serde_json::Value>) {
    if depth > MAX_DEPTH {
        return;
    }
    match w {
        Widget::Text { id, value, .. } => {
            out.insert(id.clone(), serde_json::Value::String(value.clone()));
        }
        Widget::Number { id, value, .. } => {
            if let Some(n) = serde_json::Number::from_f64(*value) {
                out.insert(id.clone(), serde_json::Value::Number(n));
            }
        }
        Widget::Toggle { id, value, .. } => {
            out.insert(id.clone(), serde_json::Value::Bool(*value));
        }
        Widget::Choice { id, value, .. } => {
            out.insert(id.clone(), serde_json::Value::String(value.clone()));
        }
        _ => {}
    }
    for child in w.children() {
        collect_values(child, depth + 1, out);
    }
}

/// Checks a whole tree: widget count, depth, ids, and the ranges the types cannot express. A bad
/// view is rejected whole: the reply carrying it is refused before any of it is applied, so the
/// view already on screen stays as it was and the panel stops with the reason.
pub fn check(root: &Widget) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    walk(root, 1, &mut count, &mut seen)
}

fn walk(w: &Widget, depth: usize, count: &mut usize, seen: &mut BTreeSet<String>) -> Result<()> {
    *count += 1;
    if *count > MAX_WIDGETS {
        return Err(Error::Ops(format!("the view has more than {MAX_WIDGETS} widgets")));
    }
    if depth > MAX_DEPTH {
        return Err(Error::Ops(format!("the view nests deeper than {MAX_DEPTH}")));
    }
    if let Some(id) = w.id() {
        if !valid_id(id) {
            return Err(Error::Ops(format!("invalid widget id {id:?}: use 1-64 characters from A-Z a-z 0-9 . _ -")));
        }
        if !seen.insert(id.to_string()) {
            return Err(Error::Ops(format!("duplicate widget id {id:?}")));
        }
    }
    match w {
        Widget::Section { title, .. } => short_text("section title", Some(title))?,
        Widget::Label { text, .. } => short_text("label text", Some(text))?,
        Widget::Text { value, placeholder, max_length, .. } => {
            let cap = max_length.unwrap_or(MAX_TEXT_LENGTH);
            if cap > MAX_TEXT_LENGTH {
                return Err(Error::Ops(format!("`maxLength` {cap} exceeds {MAX_TEXT_LENGTH}")));
            }
            if value.chars().count() > cap {
                return Err(Error::Ops(format!("a `text` value is longer ({}) than its `maxLength` ({cap})", value.chars().count())));
            }
            short_text("placeholder", placeholder.as_deref())?;
        }
        Widget::Number { value, min, max, step, .. } => {
            if !(value.is_finite() && min.is_finite() && max.is_finite()) || min > max {
                return Err(Error::Ops("a `number` has a bad range".into()));
            }
            if step.is_some_and(|s| !s.is_finite() || s <= 0.0) {
                return Err(Error::Ops("a `number` has a non-positive `step`".into()));
            }
        }
        Widget::Toggle { label, .. } => short_text("toggle label", Some(label))?,
        Widget::Button { label, .. } => short_text("button label", Some(label))?,
        Widget::Choice { value, options, .. } => {
            if options.is_empty() || options.len() > MAX_OPTIONS {
                return Err(Error::Ops(format!("a `choice` must offer 1-{MAX_OPTIONS} options")));
            }
            for o in options {
                if o.value.is_empty() || o.value.len() > MAX_LABEL {
                    return Err(Error::Ops("a `choice` option needs a value of 1-256 bytes".into()));
                }
                short_text("choice label", Some(&o.label))?;
            }
            if !options.iter().any(|o| o.value == *value) {
                return Err(Error::Ops(format!("a `choice` value {value:?} is not one of its options")));
            }
        }
        Widget::Progress { fraction } => {
            if fraction.is_some_and(|f| !f.is_finite() || !(0.0..=1.0).contains(&f)) {
                return Err(Error::Ops("a `progress` fraction must be between 0 and 1".into()));
            }
        }
        Widget::Image { blob, max_height } => {
            if !crate::blobs::valid_id(blob) {
                return Err(Error::Ops(format!("`image` names {blob:?}, which is not a blob id")));
            }
            if max_height.is_some_and(|h| !h.is_finite() || h <= 0.0) {
                return Err(Error::Ops("an `image` `maxHeight` must be positive".into()));
            }
        }
        _ => {}
    }
    for child in w.children() {
        walk(child, depth + 1, count, seen)?;
    }
    Ok(())
}

fn short_text(what: &str, text: Option<&str>) -> Result<()> {
    if text.is_some_and(|t| t.chars().count() > MAX_LABEL) {
        return Err(Error::Ops(format!("a {what} is longer than {MAX_LABEL} characters")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<Widget> {
        serde_json::from_value(v).map_err(|e| Error::Ops(e.to_string()))
    }

    #[test]
    fn parses_the_spec_example() {
        // The `ui.view` example from `docs/plugins-v2.md`.
        let w = parse(json!({"type": "column", "children": [
            {"type": "choice", "id": "tool", "value": "txt2img",
             "options": [{"value": "txt2img", "label": "Text to Image"}, {"value": "inpaint", "label": "Inpaint"}]},
            {"type": "text", "id": "prompt", "value": "", "multiline": true, "placeholder": "Describe the image"},
            {"type": "section", "title": "Settings", "collapsed": true, "children": [
                {"type": "number", "id": "steps", "value": 4, "min": 1, "max": 50, "step": 1, "slider": true},
                {"type": "number", "id": "seed", "value": 0, "min": 0, "max": 4294967295.0, "step": 1}]},
            {"type": "button", "id": "generate", "label": "Generate", "primary": true},
            {"type": "label", "id": "status", "text": "ComfyUI: connected", "level": "muted"}]}))
        .unwrap();
        check(&w).unwrap();
        let Widget::Column { children } = &w else { return };
        assert_eq!(children.len(), 5);
        assert_eq!(children[2].id(), None, "a section has no id of its own");
        assert!(matches!(&children[3], Widget::Button { primary: true, .. }));
    }

    #[test]
    fn rejects_bad_trees() {
        // Not a widget type at all.
        assert!(parse(json!({"type": "canvas"})).is_err());
        let cases = [
            // Bad id.
            json!({"type": "button", "id": "no spaces", "label": "x"}),
            json!({"type": "button", "id": "", "label": "x"}),
            // Duplicate id.
            json!({"type": "column", "children": [
                {"type": "toggle", "id": "a", "value": true, "label": "x"},
                {"type": "toggle", "id": "a", "value": false, "label": "y"}]}),
            // Bad ranges.
            json!({"type": "number", "id": "n", "value": 1, "min": 5, "max": 1}),
            json!({"type": "number", "id": "n", "value": 1, "min": 0, "max": 2, "step": 0}),
            json!({"type": "progress", "fraction": 1.5}),
            // A choice whose value is not offered, or offers nothing.
            json!({"type": "choice", "id": "c", "value": "z", "options": [{"value": "a", "label": "A"}]}),
            json!({"type": "choice", "id": "c", "value": "a", "options": []}),
            // Not a blob id.
            json!({"type": "image", "blob": "17"}),
            json!({"type": "image", "blob": "b:1", "maxHeight": -1}),
            // Value longer than its own cap.
            json!({"type": "text", "id": "t", "value": "abcd", "maxLength": 2}),
        ];
        for case in cases {
            let w = parse(case.clone()).unwrap();
            assert!(check(&w).is_err(), "{case}");
        }
        // An indeterminate bar is valid, not a mistake.
        let w = parse(json!({"type": "progress", "fraction": null})).unwrap();
        check(&w).unwrap();
    }

    #[test]
    fn rejects_trees_that_are_too_big_or_too_deep() {
        let leaf = json!({"type": "separator"});
        let column = |n: usize| Widget::Column { children: (0..n).map(|_| parse(leaf.clone()).unwrap()).collect() };
        assert!(check(&column(MAX_WIDGETS - 1)).is_ok());
        assert!(check(&column(MAX_WIDGETS)).is_err());

        let mut deep = leaf;
        for _ in 0..MAX_DEPTH {
            deep = json!({"type": "row", "children": [deep]});
        }
        assert!(check(&parse(deep).unwrap()).is_err());
    }

    #[test]
    fn a_text_value_may_fill_its_cap() {
        let w = parse(json!({"type": "text", "id": "t", "value": "ab", "maxLength": 2})).unwrap();
        check(&w).unwrap();
        // The default cap applies when none is given.
        let long = "x".repeat(MAX_TEXT_LENGTH + 1);
        let w = parse(json!({"type": "text", "id": "t", "value": long})).unwrap();
        assert!(check(&w).is_err());
        assert!(check(&parse(json!({"type": "text", "id": "t", "value": "hi"})).unwrap()).is_ok());
    }

    #[test]
    fn input_values_are_the_values_the_view_declares() {
        let w = parse(json!({"type": "column", "children": [
            {"type": "text", "id": "script", "value": "line one"},
            {"type": "section", "title": "Style", "children": [
                {"type": "number", "id": "size", "value": 12.5, "min": 1, "max": 100},
                {"type": "toggle", "id": "bold", "value": true, "label": "Bold"},
                {"type": "choice", "id": "font", "value": "Serif",
                 "options": [{"value": "Serif", "label": "Serif"}]}]},
            {"type": "button", "id": "go", "label": "Go"}]}))
        .unwrap();
        check(&w).unwrap();
        let values = input_values(&w);
        assert_eq!(values.get("script"), Some(&json!("line one")));
        assert_eq!(values.get("size"), Some(&json!(12.5)));
        assert_eq!(values.get("bold"), Some(&json!(true)));
        assert_eq!(values.get("font"), Some(&json!("Serif")));
        assert_eq!(values.get("go"), None, "a button has no value to report");
    }
}
