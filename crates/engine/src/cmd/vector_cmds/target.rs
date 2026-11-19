use super::{Document, EngineError, LayerId, NamedPath, Path, Result, Session, Value, bad, json, path_json, paths};

/// An index is a position in the current document's saved paths, not a persistent identity.
/// Keep it separate from the display name, including names such as "work" and "layer".
#[derive(Clone, Copy)]
pub(crate) enum PathTarget<'a> {
    Name(Option<&'a str>),
    Saved(usize),
}

impl<'a> PathTarget<'a> {
    pub(crate) fn parse(s: &Session, p: &'a Value, cmd: &str) -> Result<Self> {
        Self::parse_with_destination(s, p, cmd, false)
    }

    /// Fill, stroke and vector-mask creation use `layer` as the destination, not the path source.
    pub(super) fn source(s: &Session, p: &'a Value, cmd: &str) -> Result<Self> {
        Self::parse_with_destination(s, p, cmd, true)
    }

    fn parse_with_destination(s: &Session, p: &'a Value, cmd: &str, destination_layer: bool) -> Result<Self> {
        let Some(value) = p.get("pathIndex") else { return Ok(Self::Name(p.get("name").and_then(Value::as_str))) };
        for key in ["name", "typePath", "vectorMask"].into_iter().chain((!destination_layer).then_some("layer")) {
            if p.get(key).is_some() {
                return Err(bad(cmd, format!("give `pathIndex` or `{key}`, not both")));
            }
        }
        let index = value.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "`pathIndex` must be a non-negative integer"))?;
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        if doc.paths.get(index).is_none() {
            return Err(bad(cmd, format!("`pathIndex` {index} is out of range for {} saved paths", doc.paths.len())));
        }
        Ok(Self::Saved(index))
    }

    pub(crate) fn is_work(self) -> bool {
        matches!(self, Self::Name(name) if paths::is_work(name))
    }

    pub(crate) fn is_layer(self) -> bool {
        matches!(self, Self::Name(Some("layer")))
    }

    pub(crate) fn index(self) -> Option<usize> {
        match self {
            Self::Saved(i) => Some(i),
            Self::Name(_) => None,
        }
    }

    pub(crate) fn name(self, s: &Session) -> Result<String> {
        match self {
            Self::Name(name) => Ok(name.unwrap_or("work").to_owned()),
            Self::Saved(i) => {
                let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
                Ok(doc.paths.get(i).ok_or_else(|| missing(i))?.name.clone())
            }
        }
    }

    pub(crate) fn resolve(self, s: &Session) -> Result<Path> {
        match self {
            Self::Name(name) => paths::resolve_path(s, name),
            Self::Saved(i) => {
                let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
                Ok(doc.paths.get(i).ok_or_else(|| missing(i))?.path.clone())
            }
        }
    }

    pub(crate) fn path_mut(self, doc: &mut Document, layer: Option<LayerId>) -> Result<&mut Path> {
        match self {
            Self::Name(name) => paths::path_mut(doc, name.unwrap_or("work"), layer),
            Self::Saved(i) => Ok(&mut saved_mut(doc, i)?.path),
        }
    }

    pub(crate) fn with_index(self, mut result: Value) -> Value {
        if let Self::Saved(i) = self
            && let Some(object) = result.as_object_mut()
        {
            object.insert("pathIndex".into(), json!(i));
        }
        result
    }

    pub(super) fn info(self, s: &Session) -> Result<Value> {
        Ok(self.with_index(json!({"name": self.name(s)?, "path": path_json(&self.resolve(s)?)})))
    }

    /// Clipping paths still refer to a name. Do not guess which duplicate a rename/delete
    /// should update: the caller can clear that designation before editing the saved path.
    pub(super) fn check_clipping_reference(self, s: &Session, cmd: &str) -> Result<()> {
        if let Self::Saved(i) = self {
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            let name = &doc.paths.get(i).ok_or_else(|| missing(i))?.name;
            if doc.clipping_path.as_ref().is_some_and(|c| &c.name == name) && doc.paths.iter().filter(|p| &p.name == name).take(2).count() > 1 {
                return Err(bad(
                    cmd,
                    "the clipping path name matches multiple saved paths; clear the clipping path designation before renaming or deleting one",
                ));
            }
        }
        Ok(())
    }
}

fn missing(i: usize) -> EngineError {
    EngineError::Other(format!("no saved path at pathIndex {i}"))
}

pub(super) fn saved_mut(doc: &mut Document, i: usize) -> Result<&mut NamedPath> {
    doc.paths.get_mut(i).ok_or_else(|| missing(i))
}
