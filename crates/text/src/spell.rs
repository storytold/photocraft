//! Spelling checker for Edit › Check Spelling.
//!
//! Words are checked and suggested by craft-proof against SCOWL's American English dictionary,
//! which craft-proof embeds from craft-i18n's Language data at build time (`CRAFT_I18N_DIR`, the
//! commit in `craft-i18n.rev`; RFC-0011). Without that checkout craft-proof falls back to an
//! installed Hunspell dictionary or its own English word list.
//!
//! Words in capitals are checked too (`TEH` is flagged). A trailing possessive `'s` is checked and
//! suggested on its stem, typographic apostrophes count as `'`, and tokens containing digits are
//! skipped.

use std::collections::HashSet;

/// craft-proof's English spelling, with what PhotoCraft skips.
pub struct Dictionary(());

/// Words in capitals are checked; words with digits and web addresses are not.
const OPTIONS: craft::proof::Options = craft::proof::Options { ignore_uppercase: false, ignore_numbers: true, ignore_internet: true };

/// A misspelled word in a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Byte range in the checked text.
    pub start: usize,
    pub end: usize,
    pub word: String,
}

impl Dictionary {
    /// The English dictionary.
    pub fn english() -> &'static Dictionary {
        static D: Dictionary = Dictionary(());
        &D
    }

    /// Whether `word` is spelled correctly (also accepting `user` words, compared without case).
    pub fn check(&self, word: &str, user: &HashSet<String>) -> bool {
        let (w, _) = stem(word);
        let w = w.trim_matches('\'');
        if w.is_empty() || w.chars().any(|c| c.is_numeric()) || !w.chars().any(char::is_alphabetic) {
            return true;
        }
        user.contains(&w.to_lowercase()) || craft::proof::is_correct_in_with(w, craft::proof::ProofLang::En, &OPTIONS)
    }

    /// Up to `max` suggestions for a misspelled word, best first, with its possessive ending kept.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let (w, suffix) = stem(word);
        let normalized = normalize(word);
        let mut out: Vec<String> = Vec::new();
        for s in craft::proof::suggest(&w, max) {
            // The input's ending is restored here; never on a candidate that already has one.
            if !suffix.is_empty() && (s.ends_with("'s") || s.ends_with("'S")) {
                continue;
            }
            let s = format!("{s}{suffix}");
            if !out.contains(&s) && s != normalized {
                out.push(s);
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

/// `word` with typographic apostrophes as `'` and without a trailing possessive (`'s`/`'S`), and
/// that ending.
fn stem(word: &str) -> (String, &'static str) {
    let w = normalize(word);
    for suffix in ["'s", "'S"] {
        if let Some(b) = w.strip_suffix(suffix).filter(|b| !b.is_empty()) {
            return (b.to_string(), suffix);
        }
    }
    (w, "")
}

/// Typographic apostrophes → `'`.
fn normalize(w: &str) -> String {
    w.replace(['\u{2019}', '\u{2018}', '\u{02BC}'], "'")
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
    fn english_dictionary_checks_words() {
        let d = Dictionary::english();
        for w in ["the", "The", "THE", "color", "colors", "running", "don't", "don’t", "Paris", "PARIS", "photographer's", "NASA"] {
            assert!(d.check(w, &none()), "{w}");
        }
        // American English: British-only spellings are flagged, as with the old en_US list.
        for w in ["teh", "recieve", "paris", "Photograpy", "zxqv", "colour", "centre"] {
            assert!(!d.check(w, &none()), "{w}");
        }
        // Numbers and symbols are never flagged.
        assert!(d.check("2026", &none()) && d.check("B2B", &none()));
    }

    /// The dictionary is craft-i18n's SCOWL (rel-2026.02.25), not the old in-tree SCOWL 50 list,
    /// which had neither word.
    #[test]
    fn english_dictionary_is_craft_i18ns() {
        let d = Dictionary::english();
        for w in ["livestream", "vlog"] {
            assert!(d.check(w, &none()), "{w}");
        }
    }

    #[test]
    fn suggestions_offer_the_correction() {
        let d = Dictionary::english();
        let has = |word: &str, want: &str| d.suggest(word, 5).contains(&want.to_string());
        assert!(has("teh", "the") && has("recieve", "receive") && has("brwon", "brown"));
        assert!(has("Wrold", "World"), "{:?}", d.suggest("Wrold", 5));
        assert_eq!(d.suggest("HELO", 5).first().map(|s| s.chars().all(|c| c.is_uppercase())), Some(true));
    }

    #[test]
    fn possessive_suggestions_keep_the_ending() {
        let d = Dictionary::english();
        for (word, want) in [("wrold's", "world's"), ("Wrold's", "World's"), ("Mnday's", "Monday's")] {
            assert!(d.suggest(word, 5).contains(&want.to_string()), "{word}: {:?}", d.suggest(word, 5));
        }
        assert!(d.suggest("wrold's", 0).is_empty());
        assert!(!d.suggest("churchs's", 20).iter().any(|s| s.ends_with("'s's")), "{:?}", d.suggest("churchs's", 20));
        for apostrophe in ['\'', '\u{2019}', '\u{2018}', '\u{02BC}'] {
            let found = d.suggest(&format!("wrold{apostrophe}s"), 5);
            assert!(found.contains(&"world's".to_string()) && !found.iter().any(|s| s.ends_with("'s's")), "{found:?}");
        }
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
        let d = Dictionary::english();
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
