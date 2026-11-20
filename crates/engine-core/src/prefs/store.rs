use super::{Deserialize, Map, Preferences, Serialize, Value};

// ------------------------------------------------------------------ store

/// The session's preferences plus a revision counter that moves on every change (frontends
/// persist when it moves).
#[derive(Clone, Debug, Default)]
pub struct PrefsStore {
    values: Preferences,
    rev: u64,
    catalog_rev: u64,
    section_revs: std::collections::BTreeMap<String, u64>,
}

impl PrefsStore {
    pub fn get(&self) -> &Preferences {
        &self.values
    }
    /// Change preferences; bumps the revision.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Preferences) -> R) -> R {
        let before = (&self.values.shortcuts, &self.values.alternate_shortcuts, &self.values.toolbar);
        let before = (before.0.clone(), before.1.clone(), before.2.clone());
        let sections = self.values.sections.clone();
        self.rev += 1;
        let result = f(&mut self.values);
        if (&before.0, &before.1, &before.2) != (&self.values.shortcuts, &self.values.alternate_shortcuts, &self.values.toolbar) {
            self.catalog_rev = self.catalog_rev.wrapping_add(1);
        }
        for key in sections.keys().chain(self.values.sections.keys()).collect::<std::collections::BTreeSet<_>>() {
            if sections.get(key) != self.values.sections.get(key) {
                let revision = self.section_revs.entry(key.clone()).or_default();
                *revision = revision.wrapping_add(1);
            }
        }
        result
    }
    /// Revision of catalog-affecting preferences.
    pub fn catalog_rev(&self) -> u64 {
        self.catalog_rev
    }
    /// Revision of a framework preference section.
    pub fn section_rev(&self, key: &str) -> u64 {
        self.section_revs.get(key).copied().unwrap_or_default()
    }
    pub fn rev(&self) -> u64 {
        self.rev
    }
}

/// The preferences file format written by [`SavedPrefsExt::prefs_value`](crate::prefs::SavedPrefsExt::prefs_value). 2: Use Shift Key for Tool
/// Switch stored as chosen (before, a stored `false` was the old default; see `load_prefs_json`).
pub const PREFS_VERSION: u64 = 2;

/// A saved preference [`SavedPrefsExt::load_prefs_json`](crate::prefs::SavedPrefsExt::load_prefs_json) couldn't read (a wrong type, an unknown
/// choice) and left at its default.
#[derive(Clone, Debug, PartialEq)]
pub struct InvalidPref {
    /// JSON keys from the top of the preferences file (`["fileHandling", "recentFileCount"]`),
    /// kept apart because keys such as command ids contain dots.
    pub keys: Vec<String>,
    /// The value the file held there.
    pub value: Value,
}

impl InvalidPref {
    /// The dotted path, as `prefs.get` and `prefs.set` take it.
    pub fn path(&self) -> String {
        self.keys.join(".")
    }
}

/// How deep [`read_lenient`] looks for the readable parts of an unreadable value.
const LENIENT_DEPTH: usize = 32;

/// Read `v` as a `T`. When that fails, start from `T::default()` and lay the file's values over
/// it one at a time, keeping each one the whole still reads with and descending into objects
/// that don't, so one bad value resets only itself. The values left out are added to
/// `invalid`, their keys after `prefix`.
pub(super) fn read_lenient<T>(v: Value, prefix: &[&str], invalid: &mut Vec<InvalidPref>) -> T
where
    T: Default + Serialize + for<'de> Deserialize<'de>,
{
    if let Ok(t) = T::deserialize(&v) {
        return t;
    }
    let prefix: Vec<String> = prefix.iter().map(|k| (*k).to_string()).collect();
    let mut acc = serde_json::to_value(T::default()).unwrap_or(Value::Null);
    let fits = |root: &Value| T::deserialize(root).is_ok();
    match v {
        Value::Object(file) if acc.is_object() && fits(&acc) => {
            lay_over(&mut acc, &mut Vec::new(), file, &fits, &prefix, invalid);
            T::deserialize(&acc).unwrap_or_default()
        }
        value => {
            invalid.push(InvalidPref { keys: prefix, value });
            T::default()
        }
    }
}

/// One section of a preferences file (`performance`, …) read as [`SavedPrefsExt::load_prefs_json`](crate::prefs::SavedPrefsExt::load_prefs_json)
/// reads it: a value of the wrong type keeps its default and the rest still loads (#6611).
/// Startup reads the graphics settings with it before a session exists.
pub fn read_section<T>(v: Value) -> T
where
    T: Default + Serialize + for<'de> Deserialize<'de>,
{
    read_lenient(v, &[], &mut Vec::new())
}

/// [`read_lenient`]'s walk: lay `file` over the object at `at` inside `root`, key by key, so that
/// `root` always reads (`fits`).
fn lay_over(
    root: &mut Value,
    at: &mut Vec<String>,
    file: Map<String, Value>,
    fits: &dyn Fn(&Value) -> bool,
    prefix: &[String],
    invalid: &mut Vec<InvalidPref>,
) {
    for (k, fv) in file {
        let Some(obj) = object_at(root, at) else { return };
        let old = obj.insert(k.clone(), fv.clone());
        if fits(root) {
            continue;
        }
        // Back to the default. An object keeps its parts that read, and so does a new map entry
        // (a shortcut set, a workspace) when it reads from an empty object.
        let Some(obj) = object_at(root, at) else { return };
        let mut descend = fv.is_object() && at.len() < LENIENT_DEPTH && matches!(old, None | Some(Value::Object(_)));
        match old {
            Some(o) => obj.insert(k.clone(), o),
            None if descend => obj.insert(k.clone(), Value::Object(Map::new())),
            None => obj.remove(&k),
        };
        at.push(k);
        if descend && !fits(root) {
            descend = false;
            if let Some((key, parent)) = at.split_last()
                && let Some(obj) = object_at(root, parent)
            {
                obj.remove(key);
            }
        }
        match fv {
            Value::Object(m) if descend => lay_over(root, at, m, fits, prefix, invalid),
            value => invalid.push(InvalidPref { keys: prefix.iter().chain(at.iter()).cloned().collect(), value }),
        }
        at.pop();
    }
}

/// The JSON object at `path` inside `root`.
fn object_at<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Map<String, Value>> {
    path.iter().try_fold(root, |cur, k| cur.get_mut(k.as_str()))?.as_object_mut()
}

#[cfg(test)]
mod revision_tests {
    use super::PrefsStore;
    use serde_json::json;

    #[test]
    fn unrelated_preference_edits_do_not_invalidate_the_catalog() {
        let mut prefs = PrefsStore::default();
        let before = prefs.catalog_rev();
        prefs.edit(|p| p.selected_tool = json!("brush"));
        assert_eq!(prefs.catalog_rev(), before);
        prefs.edit(|p| {
            p.shortcuts.insert("example".into(), "A".into());
        });
        assert_eq!(prefs.catalog_rev(), before.wrapping_add(1));
    }

    #[test]
    fn section_revisions_change_independently() {
        let mut prefs = PrefsStore::default();
        prefs.edit(|p| {
            p.sections.insert("one".into(), json!({"value": 1}));
        });
        let one = prefs.section_rev("one");
        let two = prefs.section_rev("two");
        prefs.edit(|p| {
            p.sections.insert("two".into(), json!({"value": 2}));
        });
        assert_eq!(prefs.section_rev("one"), one);
        assert_eq!(prefs.section_rev("two"), two.wrapping_add(1));
        prefs.edit(|p| {
            p.sections.remove("one");
        });
        assert_eq!(prefs.section_rev("one"), one.wrapping_add(1));
    }
}
