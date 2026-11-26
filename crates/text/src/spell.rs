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
    /// word's case pattern and possessive ending applied.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let normalized = normalize(word);
        let (w, suffix) = normalized
            .strip_suffix("'s")
            .filter(|w| !w.is_empty())
            .map(|w| (w, "'s"))
            .or_else(|| normalized.strip_suffix("'S").filter(|w| !w.is_empty()).map(|w| (w, "'S")))
            .unwrap_or((&normalized, ""));
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
                // The input's ending is restored below; do not append it to another possessive.
                if !suffix.is_empty() && cand.ends_with("'s") {
                    continue;
                }
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
                let mut s = if all_caps {
                    form.to_uppercase()
                } else if first_cap {
                    capitalize(form)
                } else {
                    form.clone()
                };
                s.push_str(suffix);
                if !out.contains(&s) && s != normalized {
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
    let is_apos = |c: char| matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '\u{02BC}');
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
    fn possessive_suggestions_keep_the_ending_and_case() {
        let d = Dictionary::from_list("#10\nworld\nworlds\nMonday\n");
        for (word, want) in [("wrold's", "world's"), ("Wrold's", "World's"), ("WROLD'S", "WORLD'S"), ("WROLD's", "WORLD's"), ("Mnday's", "Monday's")] {
            assert_eq!(d.suggest(word, 1), vec![want.to_string()], "{word}");
        }
        assert!(d.suggest("wrold's", 0).is_empty());
        assert_eq!(d.suggest("wrold", 1), vec!["world".to_string()]);
    }

    #[test]
    fn possessive_suggestions_normalize_apostrophes_without_doubling_the_ending() {
        let d = Dictionary::from_list("world\nworld's\n");
        for apostrophe in ['\'', '\u{2019}', '\u{2018}', '\u{02BC}'] {
            assert_eq!(d.suggest(&format!("wrold{apostrophe}s"), 1), vec!["world's".to_string()]);
        }
        assert!(d.suggest("world's", 8).is_empty(), "a listed possessive must not gain a second ending");
    }

    #[test]
    fn bundled_dictionary_suggests_a_correction_for_a_misspelled_possessive() {
        let d = Dictionary::english();
        let text = "The wrold's best photo.";
        let found = d.misspellings(text, &none());
        assert_eq!(found, vec![Misspelling { start: 4, end: 11, word: "wrold's".into() }]);
        assert_eq!(d.suggest(&found[0].word, 1), vec!["world's".to_string()]);
        assert!(d.check("world's", &none()));
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

    #[test]
    fn normalized_apostrophes_stay_inside_words() {
        let d = Dictionary::english();
        for word in ["doesn't", "doesn\u{2019}t", "doesn\u{2018}t", "doesn\u{02BC}t"] {
            let text = format!("({word})");
            assert!(d.check(word, &none()), "{word}");
            assert_eq!(words(&text), vec![(1, 1 + word.len())], "{word}");
        }
    }

    #[test]
    fn normalized_contractions_are_not_reported_as_misspellings() {
        let d = Dictionary::english();
        for word in ["doesn't", "doesn\u{2019}t", "doesn\u{2018}t", "doesn\u{02BC}t"] {
            let text = format!("({word})");
            let found = d.misspellings(&text, &none());
            assert!(found.is_empty(), "{word}: {found:?}");
        }
        let typo = "doesn\u{2018}tt";
        let found = d.misspellings(typo, &none());
        assert_eq!(found, vec![Misspelling { start: 0, end: typo.len(), word: typo.into() }]);
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
