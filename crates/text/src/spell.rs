//! Spelling checker for Edit › Check Spelling.
//!
//! The dictionary is SCOWL size 50 (American English, ~72k words; see
//! `assets/dict/LICENSE-SCOWL.txt`), gzip-compressed into the binary and decoded on first use.
//! Words are grouped by SCOWL level (10 = most common … 50), which ranks suggestions.
//!
//! Case rules follow common spell checkers: a lowercase dictionary word may be written
//! capitalised or in all caps; a capitalised entry (proper noun) must keep its capital. Trailing
//! possessive `'s` is ignored, typographic apostrophes count as `'`, and tokens containing digits
//! are skipped.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::OnceLock;

static WORDS_GZ: &[u8] = include_bytes!("../../../assets/dict/en_US-scowl-50.txt.gz");

/// A character language with a bundled spelling dictionary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

impl Language {
    /// Untagged text uses American English; unsupported or malformed tags are not checked.
    pub fn from_tag(tag: Option<&str>) -> Option<Self> {
        let Some(tag) = tag else { return Some(Self::English) };
        if tag.len() > 255 || !tag.is_ascii() {
            return None;
        }
        let parts: Vec<&str> = tag.split('-').collect();
        let primary = parts[0];
        let language = if primary.eq_ignore_ascii_case("en") {
            Self::English
        } else if primary.eq_ignore_ascii_case("de") {
            Self::German
        } else {
            return None;
        };
        if parts.iter().any(|p| p.is_empty() || p.len() > 8 || !p.bytes().all(|b| b.is_ascii_alphanumeric())) {
            return None;
        }
        let alpha = |p: &str| p.bytes().all(|b| b.is_ascii_alphabetic());
        let mut i = 1;
        while i <= 3 && parts.get(i).is_some_and(|p| p.len() == 3 && alpha(p)) {
            i += 1;
        }
        // Optional script and region, then variants, extensions and private-use subtags.
        if parts.get(i).is_some_and(|p| p.len() == 4 && alpha(p)) {
            i += 1;
        }
        if parts.get(i).is_some_and(|p| (p.len() == 2 && alpha(p)) || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))) {
            i += 1;
        }
        let variants = i;
        while let Some(p) = parts.get(i).filter(|p| p.len() >= 5 || (p.len() == 4 && p.as_bytes()[0].is_ascii_digit())) {
            if parts[variants..i].iter().any(|v| v.eq_ignore_ascii_case(p)) {
                return None;
            }
            i += 1;
        }
        let mut extensions = 0u64;
        while i < parts.len() {
            let p = parts[i];
            if p.len() != 1 {
                return None;
            }
            let key = p.as_bytes()[0].to_ascii_lowercase();
            i += 1;
            if key == b'x' {
                return (i < parts.len()).then_some(language);
            }
            let bit = if key.is_ascii_digit() { key - b'0' } else { key - b'a' + 10 };
            if extensions & (1u64 << bit) != 0 {
                return None;
            }
            extensions |= 1u64 << bit;
            let start = i;
            while parts.get(i).is_some_and(|p| p.len() >= 2) {
                i += 1;
            }
            if i == start {
                return None;
            }
        }
        Some(language)
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::German => "de",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English: USA",
            Self::German => "German: 2006 Reform",
        }
    }

    pub fn dictionary(self) -> &'static Dictionary {
        match self {
            Self::English => Dictionary::english(),
            Self::German => Dictionary::german(),
        }
    }
}

/// A whole-word misspelling and the dictionary used for its suggestions.
pub struct LocalizedMisspelling {
    pub misspelling: Misspelling,
    pub language: Language,
    pub suggestions: Vec<String>,
}

/// Checks whole words using the language of their first character. Style boundaries never
/// split a word; normalized character runs and returned ranges both use UTF-8 byte offsets.
pub fn misspellings_for_layer(layer: &photocraft_doc::TextLayer, user: &HashSet<String>, max: usize) -> Vec<LocalizedMisspelling> {
    let runs = layer.char_runs();
    let mut run = 0;
    let mut end = runs[0].len;
    let mut out = Vec::new();
    for (a, b) in words(&layer.text) {
        while a >= end && run + 1 < runs.len() {
            run += 1;
            end += runs[run].len;
        }
        let Some(language) = Language::from_tag(runs[run].style.language.as_deref()) else { continue };
        let word = &layer.text[a..b];
        let dictionary = language.dictionary();
        if !dictionary.check(word, user) {
            out.push(LocalizedMisspelling {
                misspelling: Misspelling { start: a, end: b, word: word.to_string() },
                language,
                suggestions: dictionary.suggest(word, max),
            });
        }
    }
    out
}

/// A word list with frequency levels.
pub struct Dictionary {
    /// Word (as listed) → SCOWL level.
    words: HashMap<String, u8>,
    /// Lowercase form → listed forms (for case-insensitive lookups and suggestions).
    by_lower: HashMap<String, Vec<String>>,
    /// Lowercase words grouped by char length, for suggestions.
    by_len: Vec<Vec<(String, Vec<char>, u8)>>,
}

/// A misspelled word in a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Byte range in the checked text.
    pub start: usize,
    pub end: usize,
    pub word: String,
}

impl Dictionary {
    /// The bundled German dictionary (decoded independently on first use).
    pub fn german() -> &'static Dictionary {
        static D: OnceLock<Dictionary> = OnceLock::new();
        D.get_or_init(|| {
            let mut s = String::new();
            let _ = flate2::read::GzDecoder::new(&include_bytes!("../../../assets/dict/de_DE-morphy-1.2.4.txt.gz")[..]).read_to_string(&mut s);
            Dictionary::from_ranked_literals(&s)
        })
    }

    /// German's `priority<TAB>surface form` format keeps literal `#` and whitespace in forms.
    fn from_ranked_literals(list: &str) -> Dictionary {
        let mut words = HashMap::new();
        let mut by_lower: HashMap<String, Vec<String>> = HashMap::new();
        for line in list.lines() {
            let Some((priority, word)) = line.split_once('\t') else { continue };
            let level = match priority {
                "10" => 10,
                "50" => 50,
                _ => continue,
            };
            if word.is_empty() || words.contains_key(word) {
                continue;
            }
            words.insert(word.to_string(), level);
            by_lower.entry(word.to_lowercase()).or_default().push(word.to_string());
        }
        let mut by_len: Vec<Vec<(String, Vec<char>, u8)>> = Vec::new();
        for (lw, forms) in &by_lower {
            let n = lw.chars().count();
            if by_len.len() <= n {
                by_len.resize(n + 1, Vec::new());
            }
            let lvl = forms.iter().filter_map(|f| words.get(f)).min().copied().unwrap_or(50);
            by_len[n].push((lw.clone(), lw.chars().collect(), lvl));
        }
        for v in &mut by_len {
            v.sort();
        }
        Dictionary { words, by_lower, by_len }
    }

    /// The bundled English dictionary (decoded once).
    pub fn english() -> &'static Dictionary {
        static D: OnceLock<Dictionary> = OnceLock::new();
        D.get_or_init(|| {
            let mut s = String::new();
            let _ = flate2::read::GzDecoder::new(WORDS_GZ).read_to_string(&mut s);
            Dictionary::from_list(&s)
        })
    }

    /// Builds a dictionary from one word per line; `#NN` lines set the level of the words after.
    pub fn from_list(list: &str) -> Dictionary {
        let mut words = HashMap::new();
        let mut by_lower: HashMap<String, Vec<String>> = HashMap::new();
        let mut level = 50u8;
        for line in list.lines() {
            let w = line.trim();
            if let Some(l) = w.strip_prefix('#') {
                level = l.parse().unwrap_or(level);
                continue;
            }
            if w.is_empty() || words.contains_key(w) {
                continue;
            }
            words.insert(w.to_string(), level);
            by_lower.entry(w.to_lowercase()).or_default().push(w.to_string());
        }
        let mut by_len: Vec<Vec<(String, Vec<char>, u8)>> = Vec::new();
        for (lw, forms) in &by_lower {
            let n = lw.chars().count();
            if by_len.len() <= n {
                by_len.resize(n + 1, Vec::new());
            }
            let lvl = forms.iter().filter_map(|f| words.get(f)).min().copied().unwrap_or(50);
            by_len[n].push((lw.clone(), lw.chars().collect(), lvl));
        }
        for v in &mut by_len {
            v.sort();
        }
        Dictionary { words, by_lower, by_len }
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether `word` is spelled correctly (also accepting `user` words, compared without case).
    pub fn check(&self, word: &str, user: &HashSet<String>) -> bool {
        let w = normalize(word);
        let w = w.strip_suffix("'s").or_else(|| w.strip_suffix("'S")).filter(|b| !b.is_empty()).unwrap_or(&w);
        let w = w.trim_matches('\'');
        if w.is_empty() || w.chars().any(|c| c.is_numeric()) || !w.chars().any(char::is_alphabetic) {
            return true;
        }
        let lower = w.to_lowercase();
        if user.contains(&lower) || self.words.contains_key(w) {
            return true;
        }
        let Some(forms) = self.by_lower.get(&lower) else {
            return false;
        };
        let all_caps = w.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
        let first_cap = w.chars().next().is_some_and(char::is_uppercase) && w.chars().skip(1).all(|c| !c.is_uppercase());
        forms.iter().any(|f| {
            let f_lower = f.chars().all(|c| !c.is_uppercase());
            // "the" → "The"/"THE"; "Paris" → "PARIS"; "NASA" stays as listed (exact match above).
            all_caps || (f_lower && first_cap) || (first_cap && capitalize(f) == w)
        })
    }

    /// Up to `max` suggestions for a misspelled word, nearest first (then most common), with the
    /// word's case pattern applied.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let w = normalize(word);
        let lower = w.to_lowercase();
        let lc: Vec<char> = lower.chars().collect();
        let n = lc.len();
        let max_d = if n <= 4 { 1 } else { 2 };
        // Same letters in another order (transposed typing) ranks first among equal distances.
        let mut sorted = lc.clone();
        sorted.sort_unstable();
        let anagram = |c: &str| {
            let mut v: Vec<char> = c.chars().collect();
            v.sort_unstable();
            v == sorted
        };
        let mut found: Vec<(usize, bool, u8, &str)> = Vec::new();
        let mut rows = Rows::default();
        for len in n.saturating_sub(max_d)..=(n + max_d) {
            let Some(bucket) = self.by_len.get(len) else {
                continue;
            };
            for (cand, cc, lvl) in bucket {
                if let Some(d) = distance(&lc, cc, max_d, &mut rows) {
                    found.push((d, !anagram(cand), *lvl, cand));
                }
            }
        }
        found.sort();
        let all_caps = w.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase) && w.chars().filter(|c| c.is_alphabetic()).count() > 1;
        let first_cap = w.chars().next().is_some_and(char::is_uppercase);
        let mut out: Vec<String> = Vec::new();
        for (_, _, _, cand) in found {
            // Common (lowercase) forms before proper nouns: "brwon" → "brown", then "Brown".
            let mut forms: Vec<&String> = self.by_lower.get(cand).into_iter().flatten().collect();
            forms.sort_by_key(|f| f.chars().any(char::is_uppercase));
            for form in forms {
                let s = if all_caps {
                    form.to_uppercase()
                } else if first_cap {
                    capitalize(form)
                } else {
                    form.clone()
                };
                if !out.contains(&s) && s != w {
                    out.push(s);
                }
            }
            if out.len() >= max {
                break;
            }
        }
        out.truncate(max);
        out
    }

    /// Misspelled words of `text` in order.
    pub fn misspellings(&self, text: &str, user: &HashSet<String>) -> Vec<Misspelling> {
        words(text)
            .into_iter()
            .filter(|(a, b)| !self.check(&text[*a..*b], user))
            .map(|(a, b)| Misspelling { start: a, end: b, word: text[a..b].to_string() })
            .collect()
    }
}

/// Typographic apostrophes → `'`.
fn normalize(w: &str) -> String {
    w.replace(['\u{2019}', '\u{2018}', '\u{02BC}'], "'")
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Byte ranges of the words in `text`: letters/digits with inner apostrophes (hyphens split).
pub fn words(text: &str) -> Vec<(usize, usize)> {
    let is_apos = |c: char| matches!(c, '\'' | '\u{2019}' | '\u{02BC}');
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(b, c)) in chars.iter().enumerate() {
        let inner_apos = is_apos(c) && start.is_some() && chars.get(i + 1).is_some_and(|(_, n)| n.is_alphabetic());
        if c.is_alphanumeric() || inner_apos {
            start.get_or_insert(b);
        } else if let Some(s) = start.take() {
            out.push((s, b));
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// Scratch rows for [`distance`].
#[derive(Default)]
struct Rows {
    prev2: Vec<usize>,
    prev: Vec<usize>,
    cur: Vec<usize>,
}

/// Damerau–Levenshtein (optimal string alignment) distance between `a` and `b` in chars, or None
/// when above `max`. Exposed for "did you mean" hints elsewhere (engine command ids, #3674).
pub fn edit_distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    distance(&a, &b, max, &mut Rows::default())
}

/// Damerau–Levenshtein (optimal string alignment) distance, or None when above `max`.
fn distance(a: &[char], b: &[char], max: usize, rows: &mut Rows) -> Option<usize> {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return None;
    }
    let Rows { prev2, prev, cur } = rows;
    prev2.clear();
    prev2.resize(m + 1, 0);
    prev.clear();
    prev.extend(0..=m);
    cur.clear();
    cur.resize(m + 1, 0);
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
            row_min = row_min.min(v);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(prev2, prev);
        std::mem::swap(prev, cur);
    }
    (prev[m] <= max).then_some(prev[m])
}

#[cfg(test)]
#[path = "spell_language_tests.rs"]
mod language_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn all_caps_possessive_is_accepted() {
        let d = Dictionary::english();
        for w in ["MOTHER'S", "MONDAY'S", "Mother's", "mother's", "MOTHER’S"] {
            assert!(d.check(w, &none()), "{w}");
        }
        assert!(!d.check("MOTHER'SX", &none()));
    }

    #[test]
    fn bundled_dictionary_loads() {
        let d = Dictionary::english();
        assert!(d.len() > 70_000, "{}", d.len());
        for w in ["the", "The", "THE", "color", "colors", "running", "don't", "don’t", "Paris", "PARIS", "photographer's", "NASA"] {
            assert!(d.check(w, &none()), "{w}");
        }
        for w in ["teh", "recieve", "paris", "Photograpy", "zxqv"] {
            assert!(!d.check(w, &none()), "{w}");
        }
        // Numbers and symbols are never flagged.
        assert!(d.check("2026", &none()) && d.check("B2B", &none()));
    }

    #[test]
    fn suggestions_rank_by_distance_then_frequency() {
        let d = Dictionary::english();
        assert_eq!(d.suggest("teh", 3)[0], "the");
        assert!(d.suggest("recieve", 5).contains(&"receive".to_string()));
        assert_eq!(d.suggest("Wrold", 1), vec!["World".to_string()]);
        assert_eq!(d.suggest("brwon", 1), vec!["brown".to_string()]);
        assert_eq!(d.suggest("HELO", 5).first().map(|s| s.chars().all(|c| c.is_uppercase())), Some(true));
    }

    #[test]
    fn misspellings_and_user_words() {
        let d = Dictionary::from_list("#10\nhello\nworld\nParis\n");
        let text = "Hello wrld, Paris paris photocraft 42x";
        let m: Vec<String> = d.misspellings(text, &none()).into_iter().map(|m| m.word).collect();
        assert_eq!(m, vec!["wrld", "paris", "photocraft"]);
        let user: HashSet<String> = ["photocraft".to_string()].into();
        assert_eq!(d.misspellings(text, &user).len(), 2);
        let mm = &d.misspellings(text, &none())[0];
        assert_eq!(&text[mm.start..mm.end], "wrld");
    }

    /// `cargo test --release -p photocraft-text -- --ignored --nocapture spell_timing`
    #[test]
    #[ignore]
    fn spell_timing() {
        let t = std::time::Instant::now();
        let d = Dictionary::english();
        let load = t.elapsed();
        let t = std::time::Instant::now();
        for w in ["recieve", "teh", "photograpy", "definately", "seperate", "wierd", "acommodate", "begining", "occurence", "untill"] {
            let _ = d.suggest(w, 8);
        }
        eprintln!("load {load:?}, suggest {:?}/word", t.elapsed() / 10);
    }

    #[test]
    fn tokenizer_and_distance() {
        let t = "it's well-known 'quoted' café";
        let w: Vec<&str> = words(t).into_iter().map(|(a, b)| &t[a..b]).collect();
        assert_eq!(w, vec!["it's", "well", "known", "quoted", "café"]);
        let c = |s: &str| s.chars().collect::<Vec<char>>();
        let mut r = Rows::default();
        assert_eq!(distance(&c("teh"), &c("the"), 2, &mut r), Some(1));
        assert_eq!(distance(&c("teh"), &c("toe"), 2, &mut r), Some(2));
        assert_eq!(distance(&c("teh"), &c("banana"), 2, &mut r), None);
        assert_eq!(edit_distance("edit.undoo", "edit.undo", 2), Some(1));
        assert_eq!(edit_distance("teh", "banana", 2), None);
    }
}
