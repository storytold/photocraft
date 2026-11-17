//! Frontend-provided text for layers generated during import. Saved layer names never
//! pass through this policy; PSD and layered-TIFF importers retain their file's names.

#[derive(Clone, Debug, PartialEq)]
pub struct GeneratedLayerNames {
    pub background: String,
    pub layer_zero: String,
    /// Defaults for unnamed format objects, such as PDF pages and paths.
    pub labels: std::collections::BTreeMap<String, String>,
}

impl Default for GeneratedLayerNames {
    fn default() -> Self {
        Self { background: "Background".into(), layer_zero: "Layer 0".into(), labels: Default::default() }
    }
}

impl GeneratedLayerNames {
    pub(crate) fn base<'a>(&'a self, english: &'a str) -> &'a str {
        self.labels.get(english).map(String::as_str).unwrap_or(english)
    }

    pub(crate) fn numbered(&self, english: &str, number: impl std::fmt::Display) -> String {
        format!("{} {number}", self.base(english))
    }
    /// Only call at a decoder branch that just constructed one unnamed raster layer. The
    /// layer's role determines its new name; no English-name guessing or replacement is used.
    pub(crate) fn apply_to_flat(&self, doc: &mut photocraft_doc::Document) {
        let background = doc.background().is_some();
        if let [layer] = doc.layers.as_mut_slice() {
            layer.name.clone_from(if background { &self.background } else { &self.layer_zero });
        }
    }
}
