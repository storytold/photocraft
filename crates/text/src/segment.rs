//! Text segmentation for layout: grapheme clusters (the caret stops) and cursive words, the runs
//! of joining-script letters that letter spacing and kerning must not pull apart.

use std::ops::Range;

use icu_properties::CodePointMapData;
use icu_properties::props::{BidiClass, GeneralCategory, GeneralCategoryGroup, Script};
use icu_properties::script::ScriptWithExtensions;
use icu_segmenter::GraphemeClusterSegmenter;

/// Scripts whose letters join (CSS Text 3 §7.2.1, "cursive scripts").
const CURSIVE_SCRIPTS: [Script; 7] = [Script::Arabic, Script::HanifiRohingya, Script::Mandaic, Script::Mongolian, Script::Nko, Script::PhagsPa, Script::Syriac];

/// Byte offsets of the grapheme cluster boundaries of `text`, ascending, including 0 and
/// `text.len()`.
pub(crate) fn grapheme_boundaries(text: &str) -> Vec<usize> {
    GraphemeClusterSegmenter::new().segment_str(text).collect()
}

/// The character's Script_Extensions include a cursive script.
fn in_cursive_script(c: char) -> bool {
    ScriptWithExtensions::new().get_script_extensions_val(c).iter().any(|s| CURSIVE_SCRIPTS.contains(&s))
}

/// A letter of a cursive script (Script_Extensions, so tatweel, which is Common, counts).
pub(crate) fn is_cursive_letter(c: char) -> bool {
    GeneralCategoryGroup::Letter.contains(CodePointMapData::<GeneralCategory>::new().get(c)) && in_cursive_script(c)
}

/// A character of the Latin script (Script_Extensions), the bundled spelling dictionary's.
pub(crate) fn is_latin(c: char) -> bool {
    ScriptWithExtensions::new().has_script(c, Script::Latin)
}

/// ZWNJ, ZWJ and the Mongolian vowel separator: they sit inside cursive words.
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}' | '\u{180E}')
}

/// Byte ranges of the cursive words of `text`, ascending: maximal runs of cursive-script
/// letters with the marks and joiners inside them. A mark never starts a word, and a run of only
/// marks and joiners is not a word, so marks shared with other scripts (U+0308 lists Syriac) and
/// ZWJ in emoji or Indic text stay non-cursive. Any mark after an open run extends it, so a
/// word never ends mid-grapheme.
pub(crate) fn cursive_words(text: &str) -> Vec<Range<usize>> {
    let categories = CodePointMapData::<GeneralCategory>::new();
    let mut words = Vec::new();
    let mut run: Option<(usize, bool)> = None; // (start, contains a cursive letter)
    for (i, c) in text.char_indices() {
        let letter = is_cursive_letter(c);
        let mark = GeneralCategoryGroup::Mark.contains(categories.get(c));
        match run {
            Some((start, has)) if letter || mark || is_joiner(c) => {
                run = Some((start, has || letter));
            }
            None if letter || is_joiner(c) => run = Some((i, letter)),
            _ => {
                if let Some((start, true)) = run.take() {
                    words.push(start..i);
                }
            }
        }
    }
    if let Some((start, true)) = run {
        words.push(start..text.len());
    }
    words
}

/// Index of the word in `words` (ascending, as from [`cursive_words`]) containing byte `offset`.
pub(crate) fn word_at(words: &[Range<usize>], offset: usize) -> Option<usize> {
    let i = words.partition_point(|w| w.end <= offset);
    words.get(i).filter(|w| w.start <= offset).map(|_| i)
}

/// The direction of the first strong character (UAX #9 P2–P3): `Some(true)` for R or AL,
/// `Some(false)` for L, `None` when there is none. Text inside an isolate (LRI/RLI/FSI … PDI,
/// or to the end when unclosed) is skipped, as the rule requires. Digits are weak.
pub fn first_strong_rtl(text: &str) -> Option<bool> {
    let classes = CodePointMapData::<BidiClass>::new();
    let mut depth = 0usize;
    for c in text.chars() {
        let bc = classes.get(c);
        if bc == BidiClass::LeftToRightIsolate || bc == BidiClass::RightToLeftIsolate || bc == BidiClass::FirstStrongIsolate {
            depth = depth.saturating_add(1);
        } else if bc == BidiClass::PopDirectionalIsolate {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && bc == BidiClass::LeftToRight {
            return Some(false);
        } else if depth == 0 && (bc == BidiClass::RightToLeft || bc == BidiClass::ArabicLetter) {
            return Some(true);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes_keep_harakat_with_their_letter() {
        // ك + fatha, ت + fatha, ب + fatha: three graphemes of four bytes each.
        assert_eq!(grapheme_boundaries("كَتَبَ"), vec![0, 4, 8, 12]);
        // لا is two graphemes (UAX #29), though fonts draw it as one ligature.
        assert_eq!(grapheme_boundaries("لا"), vec![0, 2, 4]);
        // Latin combining marks too.
        assert_eq!(grapheme_boundaries("e\u{301}x"), vec![0, 3, 4]);
    }

    #[test]
    fn cursive_letters_are_letters_of_joining_scripts() {
        assert!(is_cursive_letter('س'));
        assert!(is_cursive_letter('\u{0640}')); // tatweel: Script=Common, Script_Extensions has Arabic
        assert!(is_cursive_letter('ܐ')); // Syriac alaph
        assert!(is_cursive_letter('ߊ')); // N'Ko letter A
        assert!(!is_cursive_letter('\u{064E}')); // fatha is a mark, not a letter
        assert!(!is_cursive_letter('\u{200D}'));
        assert!(!is_cursive_letter('١')); // digits don't join
        assert!(!is_cursive_letter('،'));
        assert!(!is_cursive_letter('a'));
        assert!(!is_cursive_letter(' '));
        assert!(!is_cursive_letter('א')); // Hebrew is RTL but not cursive
    }

    #[test]
    fn cursive_words_are_maximal_runs() {
        let t = "سلام abc مرحبا١";
        let w = cursive_words(t);
        assert_eq!(w.len(), 2);
        assert_eq!(&t[w[0].clone()], "سلام");
        assert_eq!(&t[w[1].clone()], "مرحبا");
        assert_eq!(word_at(&w, 2), Some(0));
        assert_eq!(word_at(&w, w[0].end), None); // the space after the word
        assert_eq!(word_at(&w, w[1].start), Some(1));
        assert_eq!(word_at(&w, t.len()), None);
        assert!(cursive_words("").is_empty());
        assert_eq!(cursive_words("مرحبا"), vec![0.."مرحبا".len()]);
        assert_eq!(cursive_words("كَتَبَ"), vec![0..12]);
        let persian = "می\u{200C}خواهم";
        assert_eq!(cursive_words(persian), vec![0..persian.len()]);
        assert_eq!(cursive_words("ب\u{0301}ت"), vec![0..6]);
        assert_eq!(cursive_words("ب\u{034F}\u{064E}ت"), vec![0..8]);
        assert_eq!(cursive_words("\u{200D}ب"), vec![0..5]);
        assert_eq!(cursive_words("ب\u{200D}"), vec![0..5]);
        let mongolian = "ᠭᠠᠷᠢᠭ\u{180E}ᠠ";
        assert_eq!(cursive_words(mongolian), vec![0..mongolian.len()]);
    }

    #[test]
    fn marks_and_joiners_of_other_scripts_never_make_a_word() {
        for t in ["u\u{0308}ber", "Vi\u{0323}\u{0302}t", "👨\u{200D}👩\u{200D}👧", "क्\u{200D}ष", "a\u{200D}"] {
            assert!(cursive_words(t).is_empty(), "{t:?}");
        }
        // A mark never opens a word: the word starts at ب, not inside the ü grapheme.
        assert_eq!(cursive_words("u\u{0308}ب"), vec![3..5]);
    }

    #[test]
    fn first_strong_direction_skips_weak_characters_and_isolates() {
        assert_eq!(first_strong_rtl("مرحبا"), Some(true));
        assert_eq!(first_strong_rtl("١٢٣ مرحبا"), Some(true), "Arabic-Indic digits are weak");
        assert_eq!(first_strong_rtl("123 abc"), Some(false));
        assert_eq!(first_strong_rtl("Galaxy S24 شاشة"), Some(false));
        assert_eq!(first_strong_rtl("שלום"), Some(true));
        assert_eq!(first_strong_rtl("\u{2066}abc\u{2069} مرحبا"), Some(true), "isolated text is skipped (UAX #9 P2)");
        assert_eq!(first_strong_rtl("\u{2067}abc"), None, "an unclosed isolate runs to the end");
        assert_eq!(first_strong_rtl("١٢٣"), None);
        assert_eq!(first_strong_rtl(""), None);
    }
}
