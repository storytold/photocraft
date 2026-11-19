use photocraft_doc::{Document, Path};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A saved row is addressed by position, independently of its display name. Strings keep
/// old serialized UI state readable and represent the temporary work/layer paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathSelection {
    Saved {
        #[serde(rename = "pathIndex")]
        index: usize,
    },
    Name(String),
}

impl From<String> for PathSelection {
    fn from(name: String) -> Self {
        Self::Name(name)
    }
}

impl From<&str> for PathSelection {
    fn from(name: &str) -> Self {
        Self::Name(name.into())
    }
}

impl PathSelection {
    /// Only legacy names and temporary paths have a string key; a saved path named "work"
    /// must never be treated as the Work Path.
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Name(n) => Some(n),
            Self::Saved { .. } => None,
        }
    }

    pub fn saved_index(&self, doc: &Document) -> Option<usize> {
        match self {
            Self::Saved { index } => doc.paths.get(*index).map(|_| *index),
            Self::Name(n) if n != "work" && n != "layer" => {
                let mut matches = doc.paths.iter().enumerate().filter(|(_, p)| &p.name == n);
                let (index, _) = matches.next()?;
                // Old UI state cannot disambiguate duplicate names. Require a fresh row click.
                matches.next().is_none().then_some(index)
            }
            _ => None,
        }
    }

    pub fn path<'a>(&self, doc: &'a Document) -> Option<&'a Path> {
        if self.name() == Some("work") { doc.work_path.as_ref() } else { doc.paths.get(self.saved_index(doc)?).map(|p| &p.path) }
    }

    pub fn params(&self) -> Value {
        match self {
            Self::Saved { index } => json!({"pathIndex": index}),
            Self::Name(n) => json!({"name": n}),
        }
    }

    pub fn with_params(&self, extra: Value) -> Value {
        let mut params = extra.as_object().cloned().unwrap_or_default();
        params.remove("name");
        params.remove("pathIndex");
        if let Some(target) = self.params().as_object() {
            params.extend(target.clone());
        }
        Value::Object(params)
    }
}

impl PartialEq<&str> for PathSelection {
    fn eq(&self, other: &&str) -> bool {
        self.name() == Some(*other)
    }
}

impl From<&PathSelection> for PathSelection {
    fn from(target: &PathSelection) -> Self {
        target.clone()
    }
}

/// Runtime ownership of an open path menu. Reopened files can reuse IDs and revision numbers.
/// Keep a weak snapshot so an old allocation cannot be reused, without retaining document pixels.
#[derive(Clone, Debug)]
pub struct PathMenuSource {
    document: std::sync::Weak<Document>,
    revision: u64,
    saved_paths_revision: u64,
}

impl PartialEq for PathMenuSource {
    fn eq(&self, other: &Self) -> bool {
        self.document.ptr_eq(&other.document) && self.revision == other.revision && self.saved_paths_revision == other.saved_paths_revision
    }
}

impl PathMenuSource {
    pub(crate) fn capture(st: &photocraft_engine::DocState) -> Self {
        Self { document: std::sync::Arc::downgrade(&st.doc), revision: st.revision, saved_paths_revision: st.saved_paths_revision() }
    }
}
