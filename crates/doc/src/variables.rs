//! Data-driven graphics: Image › Variables and Data Sets.
//!
//! A *variable* is a named data slot (a CSV column). A *binding* ([`VariableDef`]) drives one
//! layer's visibility, text or pixel content, either straight from a variable or through a *rule*:
//! an expression over any of the variables (`Type == "gear"`, `"{Name} - Lv {Level}"`). One
//! variable may drive any number of layers. A *data set* is a named row of values, one per
//! variable. Applying a data set mutates the document so a single template renders many
//! variations — ideal for batch generation (`file.export.dataSetsAsFiles`) and agents driving the
//! CLI/MCP.
//!
//! Pure data (the doc crate takes no I/O): loading a replacement image for a Pixel Replacement
//! variable is the engine's job; here a data value just names the file.

use serde::{Deserialize, Serialize};

use crate::LayerId;

/// All variable definitions and data sets for a document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Variables {
    /// Layer bindings. Several may read the same variable; a layer has at most one per kind.
    #[serde(default)]
    pub defs: Vec<VariableDef>,
    /// Declared variables (data columns) beyond those a binding reads directly: CSV headers and
    /// the names rules refer to. [`Variables::variable_names`] is the full list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Named data sets (rows of values).
    #[serde(default)]
    pub data_sets: Vec<DataSet>,
    /// Index into `data_sets` of the data set applied last, if any.
    #[serde(default)]
    pub active: Option<usize>,
}

/// One layer property bound to a variable, or to a rule over variables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariableDef {
    /// The variable this binding reads (matched by data-set values and CSV headers). Ignored, and
    /// usually empty, when `rule` is set.
    #[serde(default)]
    pub name: String,
    /// The layer this binding drives.
    pub layer: LayerId,
    /// What it controls.
    pub kind: VarKind,
    /// An expression computing the value from the data set (the engine's rule language), used
    /// instead of reading `name` directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

impl VariableDef {
    /// A binding that reads variable `name` directly.
    pub fn direct(name: impl Into<String>, layer: LayerId, kind: VarKind) -> Self {
        Self { name: name.into(), layer, kind, rule: None }
    }

    /// A binding whose value comes from `rule`.
    pub fn ruled(rule: impl Into<String>, layer: LayerId, kind: VarKind) -> Self {
        Self { name: String::new(), layer, kind, rule: Some(rule.into()) }
    }

    /// The variable read directly, or None for a rule binding.
    pub fn direct_name(&self) -> Option<&str> {
        self.rule.is_none().then_some(self.name.as_str()).filter(|n| !n.is_empty())
    }
}

/// The aspect of a layer a variable controls (Photoshop's three variable types).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum VarKind {
    /// Show or hide the layer.
    Visibility,
    /// Replace a text layer's string.
    TextReplacement,
    /// Replace a pixel layer's content with an image file, scaled and aligned.
    PixelReplacement {
        #[serde(default)]
        method: PixelMethod,
        #[serde(default)]
        align: PixelAlign,
        /// Clip the replacement to the original layer's bounds.
        #[serde(default)]
        clip: bool,
    },
}

/// How a replacement image is scaled into the layer bounds (Photoshop's "Method").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelMethod {
    /// Scale to fit inside the bounds, keeping aspect (default).
    #[default]
    Fit,
    /// Scale to cover the bounds, keeping aspect.
    Fill,
    /// Place at original size.
    AsIs,
    /// Stretch to the bounds, ignoring aspect.
    Conform,
}

/// Where a replacement image sits within the layer bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelAlign {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    #[default]
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// A named row of values, one per variable (keyed by variable name).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DataSet {
    pub name: String,
    /// Variable name → value. A variable with no entry keeps the document's current value.
    #[serde(default)]
    pub values: Vec<DataValue>,
}

/// One variable's value within a data set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataValue {
    /// Which variable (by name).
    pub variable: String,
    #[serde(flatten)]
    pub value: Value,
}

/// The payload of a [`DataValue`], matched to the variable's [`VarKind`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Value {
    /// For a Visibility variable.
    Visibility(bool),
    /// For a Text Replacement variable.
    Text(String),
    /// For a Pixel Replacement variable: a path to the replacement image.
    Pixels(String),
}

impl Variables {
    /// True when there are no bindings, declared variables or data sets.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty() && self.names.is_empty() && self.data_sets.is_empty()
    }

    /// Find the first binding that reads variable `name` directly.
    pub fn def(&self, name: &str) -> Option<&VariableDef> {
        self.defs.iter().find(|d| d.direct_name() == Some(name))
    }

    /// Every binding that reads variable `name` directly.
    pub fn defs_reading<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a VariableDef> + 'a {
        self.defs.iter().filter(move |d| d.direct_name() == Some(name))
    }

    /// Every variable, without duplicates: declared names first, then those bindings read directly.
    pub fn variable_names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in self.names.iter().map(String::as_str).chain(self.defs.iter().filter_map(VariableDef::direct_name)) {
            if !n.is_empty() && !out.iter().any(|o| o == n) {
                out.push(n.to_string());
            }
        }
        out
    }

    /// Drop the bindings of layers `exists` rejects (deleted, merged away). A variable that loses
    /// its last binding stays declared, so data-set values for it are kept. Returns how many
    /// bindings were dropped.
    pub fn retain_layers(&mut self, exists: impl Fn(LayerId) -> bool) -> usize {
        let before = self.defs.len();
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.defs).into_iter().partition(|d| exists(d.layer));
        self.defs = keep;
        for name in gone.iter().filter_map(VariableDef::direct_name) {
            if !self.names.iter().any(|n| n == name) && !self.defs.iter().any(|d| d.direct_name() == Some(name)) {
                self.names.push(name.to_string());
            }
        }
        before - self.defs.len()
    }

    /// Find a data set by name.
    pub fn data_set(&self, name: &str) -> Option<&DataSet> {
        self.data_sets.iter().find(|s| s.name == name)
    }

    /// Index of a data set by name.
    pub fn data_set_index(&self, name: &str) -> Option<usize> {
        self.data_sets.iter().position(|s| s.name == name)
    }
}

impl DataSet {
    /// The value for a variable in this data set, if present.
    pub fn get(&self, variable: &str) -> Option<&Value> {
        self.values.iter().find(|v| v.variable == variable).map(|v| &v.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let v = Variables {
            defs: vec![
                VariableDef::direct("show", LayerId(1), VarKind::Visibility),
                VariableDef::direct("title", LayerId(2), VarKind::TextReplacement),
                VariableDef::direct("photo", LayerId(3), VarKind::PixelReplacement { method: PixelMethod::Fill, align: PixelAlign::TopLeft, clip: true }),
                VariableDef::ruled("Type == \"gear\"", LayerId(4), VarKind::Visibility),
            ],
            names: vec!["Type".into()],
            data_sets: vec![DataSet {
                name: "row1".into(),
                values: vec![
                    DataValue { variable: "show".into(), value: Value::Visibility(false) },
                    DataValue { variable: "title".into(), value: Value::Text("Hello".into()) },
                    DataValue { variable: "photo".into(), value: Value::Pixels("/tmp/a.png".into()) },
                ],
            }],
            active: Some(0),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<Variables>(&json).unwrap(), v);
    }

    #[test]
    fn lookups() {
        let v = Variables {
            defs: vec![VariableDef::direct("t", LayerId(9), VarKind::TextReplacement)],
            names: vec![],
            data_sets: vec![DataSet { name: "s".into(), values: vec![DataValue { variable: "t".into(), value: Value::Text("x".into()) }] }],
            active: None,
        };
        assert_eq!(v.def("t").unwrap().layer, LayerId(9));
        assert_eq!(v.data_set_index("s"), Some(0));
        assert!(matches!(v.data_set("s").unwrap().get("t"), Some(Value::Text(t)) if t == "x"));
        assert!(v.def("missing").is_none());
    }

    #[test]
    fn older_files_without_rules_or_names_still_load() {
        let json = r#"{"defs":[{"name":"t","layer":9,"kind":{"type":"textReplacement"}}],"dataSets":[]}"#;
        let v: Variables = serde_json::from_str(json).unwrap();
        assert_eq!(v.defs, vec![VariableDef::direct("t", LayerId(9), VarKind::TextReplacement)]);
        assert!(v.names.is_empty());
    }

    #[test]
    fn retain_layers_drops_bindings_of_missing_layers_and_keeps_their_variables() {
        let mut v = Variables {
            defs: vec![
                VariableDef::direct("title", LayerId(1), VarKind::TextReplacement),
                VariableDef::direct("title", LayerId(2), VarKind::TextReplacement),
                VariableDef::direct("show", LayerId(2), VarKind::Visibility),
                VariableDef::ruled("Type == \"gear\"", LayerId(2), VarKind::Visibility),
            ],
            names: vec!["Type".into()],
            ..Default::default()
        };
        assert_eq!(v.retain_layers(|id| id != LayerId(2)), 3);
        assert_eq!(v.defs, vec![VariableDef::direct("title", LayerId(1), VarKind::TextReplacement)]);
        assert_eq!(v.names, vec!["Type".to_string(), "show".to_string()], "`title` is still bound; `show` stays declared");
        assert_eq!(v.retain_layers(|_| true), 0);
    }

    #[test]
    fn one_variable_drives_several_layers() {
        let v = Variables {
            defs: vec![
                VariableDef::direct("title", LayerId(1), VarKind::TextReplacement),
                VariableDef::direct("title", LayerId(2), VarKind::TextReplacement),
                VariableDef::ruled("Type == \"gear\"", LayerId(3), VarKind::Visibility),
            ],
            names: vec!["Type".into(), "title".into()],
            ..Default::default()
        };
        assert_eq!(v.defs_reading("title").map(|d| d.layer).collect::<Vec<_>>(), vec![LayerId(1), LayerId(2)]);
        assert_eq!(v.variable_names(), vec!["Type".to_string(), "title".to_string()]);
        assert_eq!(v.defs[2].direct_name(), None);
    }
}
