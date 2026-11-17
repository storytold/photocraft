//! Naming policy captured before an edit borrows the session. Only generated text goes through
//! this service; source layer names, file stems, preset names and text content remain literal.

use photocraft_doc::Document;

#[derive(Clone, Default)]
pub struct DefaultNames {
    pub(crate) language: String,
    pub(crate) translate: Option<fn(&str, &str, &str) -> String>,
    pub(crate) variants: Option<fn(&str, &str) -> Vec<String>>,
}

impl DefaultNames {
    pub fn base(&self, english: &str) -> String {
        self.base_in("layerName", english)
    }

    pub fn base_in(&self, context: &str, english: &str) -> String {
        self.translate.map(|tr| tr(&self.language, context, english)).filter(|name| !name.is_empty()).unwrap_or_else(|| english.into())
    }

    pub fn numbered(&self, english: &str, number: usize) -> String {
        format!("{} {number}", self.base(english))
    }

    pub fn numbered_in(&self, context: &str, english: &str, number: usize) -> String {
        format!("{} {number}", self.base_in(context, english))
    }

    pub fn copy(&self, doc: &Document, source: &str) -> String {
        doc.copy_name_with_suffix(source, &self.base("copy"))
    }

    pub(crate) fn is_type_copy(&self, name: &str, auto: &str, empty: bool) -> bool {
        let suffixes = self.bases("copy");
        let mut candidate = name;
        let mut previous = None;
        // Languages can append different copy markers. Identical consecutive markers are not
        // produced by the generator; keep such literal names. Bound scans of handcrafted chains.
        for _ in 0..32 {
            let Some((index, marker, root)) = suffixes.iter().enumerate().find_map(|(index, suffix)| {
                if previous == Some(index) {
                    return None;
                }
                copy_root(candidate, suffix).map(|root| (index, suffix.as_str(), root))
            }) else {
                return false;
            };
            if root == copy_root(auto, marker).unwrap_or(auto) || (empty && self.is_empty_text_name(root)) {
                return true;
            }
            previous = Some(index);
            candidate = root;
        }
        false
    }

    fn bases(&self, english: &str) -> Vec<String> {
        let mut values = self.variants.map(|variants| variants("layerName", english)).unwrap_or_default();
        values.push(self.base(english));
        values.push(english.into());
        values.retain(|value| !value.is_empty());
        values.sort_unstable();
        values.dedup();
        values
    }

    pub fn is_empty_text_name(&self, name: &str) -> bool {
        self.bases("Type Layer").iter().any(|value| value == name)
    }

    pub fn is_numbered(&self, name: &str, english: &str) -> bool {
        let prefix = format!("{} ", self.base(english));
        name.strip_prefix(&prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    }

    pub fn text(&self, text: &str) -> String {
        if text.trim().is_empty() { self.base("Type Layer") } else { crate::type_cmds::layer_name(text) }
    }

    /// Substitute in the translated template once. A user's name containing `{effect}` (for
    /// example) must not be reinterpreted as another placeholder after it is inserted.
    pub fn format(&self, english: &str, values: &[(&str, &str)]) -> String {
        let template = self.base(english);
        let mut rest = template.as_str();
        let mut out = String::new();
        while let Some((before, after)) = rest.split_once('{') {
            out.push_str(before);
            let Some((key, tail)) = after.split_once('}') else {
                out.push('{');
                out.push_str(after);
                return out;
            };
            if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
                out.push_str(value);
            } else {
                out.push('{');
                out.push_str(key);
                out.push('}');
            }
            rest = tail;
        }
        out.push_str(rest);
        out
    }
}

fn copy_root<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let (root, rest) = name.rsplit_once(&format!(" {suffix}"))?;
    (rest.is_empty() || rest.strip_prefix(' ').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))).then_some(root)
}
