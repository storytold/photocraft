//! Per-character language selection keeps the existing English spelling results intact.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use photocraft_doc::TextLayer;
use photocraft_doc::text::{CharStyle, TextRun};

fn layer(text: &str, spans: &[(usize, Option<&str>)]) -> TextLayer {
    TextLayer {
        text: text.into(),
        runs: spans
            .iter()
            .map(|(len, language)| TextRun { len: *len, style: CharStyle { language: language.map(str::to_string), ..Default::default() } })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn supported_tags_choose_the_dictionary_and_display_name() {
    for tag in [None, Some("en"), Some("en-US"), Some("EN-us"), Some("en-Latn-US"), Some("en-US-u-co-phonebk"), Some("en-x-private")] {
        assert_eq!(Language::from_tag(tag), Some(Language::English), "{tag:?}");
    }
    for tag in ["de", "de-DE", "DE-de", "de-Latn-DE", "de-CH-1901", "de-DE-1996", "de-u-co-phonebk", "de-x-spelling"] {
        assert_eq!(Language::from_tag(Some(tag)), Some(Language::German), "{tag}");
    }
    assert_eq!((Language::English.tag(), Language::English.name()), ("en-US", "English: USA"));
    assert_eq!((Language::German.tag(), Language::German.name()), ("de", "German: 2006 Reform"));
    assert!(std::ptr::eq(Language::English.dictionary(), Dictionary::english()));
    assert!(std::ptr::eq(Language::German.dictionary(), Dictionary::german()));
}

#[test]
fn german_ranked_format_preserves_literal_forms_and_priority() {
    let d = Dictionary::from_ranked_literals("10\tWelt\n50\tWelten\n10\t#bleich\n50\t#preis\n10\t leading\n50\ttrailing \ninvalid\n20\tignored\n10\t\n");
    assert_eq!(d.len(), 6);
    for (word, priority) in [("Welt", 10), ("Welten", 50), ("#bleich", 10), ("#preis", 50), (" leading", 10), ("trailing ", 50)] {
        assert_eq!(d.words.get(word), Some(&priority), "{word:?}");
    }
    assert!(!d.words.contains_key("leading") && !d.words.contains_key("trailing"));
    assert_eq!(d.suggest("Weltt", 1), ["Welt"]);
}

#[test]
fn german_bundle_keeps_the_full_export_including_historic_spellings() {
    let d = Dictionary::german();
    assert_eq!(d.len(), 552_751);
    for word in ["Hallo", "Welt", "daß", "muß", "#bleich", "#preis", "#scher", "#vertu"] {
        assert!(d.words.contains_key(word), "{word}");
    }
    assert!(d.check("Hallo", &HashSet::new()));
    assert_eq!(d.suggest("Weltt", 1), ["Welt"]);
}

#[test]
fn unsupported_and_malformed_tags_are_not_checked() {
    let long = format!("de-x-{}", "a".repeat(4096));
    for tag in [
        "fr",
        "zh-Hans",
        "",
        "!!@@",
        "de_ DE",
        "de_DE",
        " de",
        "de ",
        "de-",
        "de--DE",
        "de-abcdefghi",
        "de-ü",
        "de-u",
        "de-x",
        "de-u-co-u-ca",
        "de-1996-1996",
        "de-DE-DE",
        long.as_str(),
    ] {
        assert_eq!(Language::from_tag(Some(tag)), None, "{tag}");
        let l = layer("zxqv", &[(4, Some(tag))]);
        assert!(misspellings_for_layer(&l, &HashSet::new(), 5).is_empty(), "{tag}");
    }
}

#[test]
fn untagged_and_english_runs_keep_all_existing_results_and_suggestions() {
    let text = "Teh quick brwon fox, recieve Wrold HELO Paris paris NASA MOTHER'S 42x don't cafè zxqv";
    let user = HashSet::new();
    let expected = Dictionary::english().misspellings(text, &user);
    for tag in [None, Some("en-US"), Some("EN")] {
        let l = layer(text, &[(text.len(), tag)]);
        let found = misspellings_for_layer(&l, &user, 8);
        assert_eq!(found.len(), expected.len());
        for (actual, old) in found.iter().zip(&expected) {
            assert_eq!(&actual.misspelling, old);
            assert_eq!(actual.language, Language::English);
            assert_eq!(actual.suggestions, Dictionary::english().suggest(&old.word, 8));
        }
    }
    let no_runs = TextLayer { text: text.into(), ..Default::default() };
    let actual = misspellings_for_layer(&no_runs, &user, 8);
    assert_eq!(actual.iter().map(|m| m.misspelling.clone()).collect::<Vec<_>>(), expected);
}

#[test]
fn each_word_uses_its_character_language_in_one_layer() {
    let text = "Teh Hallo Weltt brwon";
    let l = layer(text, &[(4, None), (12, Some("de")), (5, Some("en-US"))]);
    let found = misspellings_for_layer(&l, &HashSet::new(), 8);
    assert_eq!(
        found.iter().map(|m| (m.misspelling.word.as_str(), m.language)).collect::<Vec<_>>(),
        [("Teh", Language::English), ("Weltt", Language::German), ("brwon", Language::English),]
    );
    let german = &found[1];
    assert_eq!(german.suggestions.first().map(String::as_str), Some("Welt"));
    assert!(Dictionary::german().check("Hallo", &HashSet::new()));
    assert_eq!(found[0].suggestions, Dictionary::english().suggest("Teh", 8));
    assert_eq!(found[2].suggestions, Dictionary::english().suggest("brwon", 8));
}

#[test]
fn style_boundaries_do_not_fragment_words_or_change_suggestions() {
    let mut l = layer("recieve Weltt", &[(2, None), (6, Some("en-US")), (2, Some("de")), (3, Some("DE-de"))]);
    l.runs[1].style.size_pt = 24.0;
    l.runs[3].style.size_pt = 30.0;
    let found = misspellings_for_layer(&l, &HashSet::new(), 8);
    assert_eq!(found.iter().map(|m| m.misspelling.word.as_str()).collect::<Vec<_>>(), ["recieve", "Weltt"]);
    assert_eq!(found[0].suggestions, Dictionary::english().suggest("recieve", 8));
    assert_eq!(found[1].suggestions, Dictionary::german().suggest("Weltt", 8));
}

#[test]
fn word_language_comes_from_its_first_character() {
    let l = layer("Weltt Teh", &[(2, Some("de")), (4, Some("fr")), (3, None)]);
    let found = misspellings_for_layer(&l, &HashSet::new(), 5);
    assert_eq!(found.iter().map(|m| (m.misspelling.word.as_str(), m.language)).collect::<Vec<_>>(), [("Weltt", Language::German), ("Teh", Language::English),]);
}

#[test]
fn user_words_remain_shared_between_languages() {
    let l = layer("zxqv zxqv", &[(5, None), (4, Some("de"))]);
    assert_eq!(misspellings_for_layer(&l, &HashSet::new(), 5).len(), 2);
    let user = HashSet::from(["zxqv".to_string()]);
    assert!(misspellings_for_layer(&l, &user, 5).is_empty());
}

#[test]
fn normalized_byte_runs_keep_unicode_ranges_and_skip_unsupported_words() {
    let text = "🦊 café Weltt Teh";
    let german_start = text.find("Weltt").unwrap();
    let english_start = text.find("Teh").unwrap();
    // The zero-length run is discarded, and the final short run stretches to the end.
    let l = layer(text, &[(0, Some("de")), (german_start, Some("fr")), (english_start - german_start, Some("de")), (1, None)]);
    let found = misspellings_for_layer(&l, &HashSet::new(), 5);
    assert_eq!(found.len(), 2);
    for (m, (a, b)) in found.iter().zip([(german_start, german_start + 5), (english_start, text.len())]) {
        assert_eq!((m.misspelling.start, m.misspelling.end), (a, b));
        assert_eq!(&text[a..b], m.misspelling.word);
    }
    assert_eq!(found[0].language, Language::German);
    assert_eq!(found[1].language, Language::English);
    assert!(misspellings_for_layer(&TextLayer::default(), &HashSet::new(), 5).is_empty());
}
