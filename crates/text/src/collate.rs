//! Font menu ordering: the order a font menu lists names in, which follows the writing system of
//! the interface, as Photoshop's does. Latin names compare letter by letter with case and
//! full-width forms folded; Hanzi, kana and Hangul compare in the order of the writing system's
//! standard character set — GB2312's pinyin rows for Simplified Chinese, Big5's stroke order for
//! Traditional Chinese, JIS X 0208's gojūon order for Japanese and KS X 1001's for Korean — so a
//! Chinese list reads like a pinyin index ("阿里巴巴" before "阿里健康", "未来" near "W").
//!
//! Sorting by code point is not that order at all ("优设标题黑" first, "阿里巴巴" last), and a
//! plain locale collation would need the ICU data; the standard sets' own order is what
//! Photoshop, Finder and Windows dialogs use for these scripts.

use crate::cjk::{self, CjkScript};

/// The generated tables ([`build.rs`](../../build.rs)): each writing system's standard set, its
/// characters sorted by code point and the rank each has in the set's own order.
mod tables {
    include!(concat!(env!("OUT_DIR"), "/collation.rs"));
}

/// One writing system's characters and their order.
struct Set {
    chars: &'static [char],
    ranks: &'static [u16],
}

impl Set {
    /// The character's rank in the set's order; `None` for a character the set doesn't have.
    fn rank(&self, c: char) -> Option<u32> {
        self.chars.binary_search(&c).ok().and_then(|i| self.ranks.get(i)).map(|r| u32::from(*r))
    }
}

fn set(script: CjkScript) -> Set {
    match script {
        CjkScript::SimplifiedChinese => Set { chars: &tables::HANS_CHARS, ranks: &tables::HANS_RANKS },
        CjkScript::TraditionalChinese => Set { chars: &tables::HANT_CHARS, ranks: &tables::HANT_RANKS },
        CjkScript::Japanese => Set { chars: &tables::JPAN_CHARS, ranks: &tables::JPAN_RANKS },
        CjkScript::Korean => Set { chars: &tables::KORE_CHARS, ranks: &tables::KORE_RANKS },
    }
}

/// ASCII first (letters, digits, spaces and punctuation alike, in code point order), then the
/// script's own characters in its set's order, then the other scripts' characters, then
/// everything else, each group by code point.
const SET: u32 = 1 << 24;
const OTHER_SCRIPT: u32 = 2 << 24;
const OTHER: u32 = 3 << 24;

/// The sort key of a menu label for `script`: compare keys as sequences, then the English family
/// names, and the rows come out in menu order.
pub fn menu_sort_key(label: &str, script: Option<CjkScript>) -> Vec<u32> {
    let set = script.map(set);
    label.chars().map(|c| rank_of(c, set.as_ref())).collect()
}

fn rank_of(c: char, set: Option<&Set>) -> u32 {
    let c = fold(c);
    // All ASCII, a space or a comma included: those sort with the letters, so "Open Sans" reads
    // before "OpenSymbol" and "Aa 厚底" before "Aa厚底" instead of after every Han character.
    if c.is_ascii() {
        return u32::from(c);
    }
    if let Some(rank) = set.and_then(|s| s.rank(c)) {
        return SET | rank;
    }
    if cjk::classify(c).is_some() { OTHER_SCRIPT | u32::from(c) } else { OTHER | u32::from(c) }
}

/// The form of a character the menu sorts by: full-width ASCII forms are the letters they stand
/// for ("ＭＳ ゴシック" sorts with "MS"), katakana fold onto the hiragana they share the gojūon
/// order with ("ヒカリ" among "ひかり"), and capitals onto lower case.
fn fold(c: char) -> char {
    let c = if ('\u{FF01}'..='\u{FF5E}').contains(&c) { char::from_u32(c as u32 - 0xFEE0).unwrap_or(c) } else { c };
    let c = if ('\u{30A1}'..='\u{30F6}').contains(&c) { char::from_u32(c as u32 - 0x60).unwrap_or(c) } else { c };
    c.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(label: &str, script: CjkScript) -> Vec<u32> {
        menu_sort_key(label, Some(script))
    }

    /// The pinyin order Photoshop lists Chinese fonts in: "阿里巴巴" (ba) before "阿里健康"
    /// (jian), which code point order reverses.
    #[test]
    fn simplified_chinese_sorts_by_pinyin() {
        let mut names = ["未来荧黑", "阿里巴巴普惠体", "阿里健康体", "寒蝉德黑体", "优设标题黑", "站酷高端黑"];
        names.sort_by_cached_key(|n| key(n, CjkScript::SimplifiedChinese));
        assert_eq!(names, ["阿里巴巴普惠体", "阿里健康体", "寒蝉德黑体", "未来荧黑", "优设标题黑", "站酷高端黑"]);
    }

    /// Japanese sorts in gojūon order, with katakana folded onto hiragana; a name that starts
    /// with Latin letters still leads, and full-width letters are the letters they stand for.
    #[test]
    fn japanese_sorts_by_gojuon() {
        let mut names = ["游ゴシック", "ヒカリ角ゴ", "ＭＳ ゴシック", "あおぞら"];
        names.sort_by_cached_key(|n| key(n, CjkScript::Japanese));
        // "MS" < あおぞら (a) < ヒカリ (hi) < 游 (a kanji, after the kana block)
        assert_eq!(names, ["ＭＳ ゴシック", "あおぞら", "ヒカリ角ゴ", "游ゴシック"]);
    }

    /// Latin names come first, whatever the script, and compare letter by letter: "Aa厚底黑"
    /// before "Aa剑豪体" (hou before jian) as in Photoshop's Chinese menu.
    #[test]
    fn latin_names_lead_and_compare_letter_by_letter() {
        let mut names = ["阿里巴巴普惠体", "Aa新华霜剑体", "Aa剑豪体", "Aa厚底黑"];
        names.sort_by_cached_key(|n| key(n, CjkScript::SimplifiedChinese));
        assert_eq!(names, ["Aa厚底黑", "Aa剑豪体", "Aa新华霜剑体", "阿里巴巴普惠体"]);
        let folded = |a: &str, b: &str| key(a, CjkScript::SimplifiedChinese) < key(b, CjkScript::SimplifiedChinese);
        assert_eq!(key("adobe", CjkScript::SimplifiedChinese), key("Adobe", CjkScript::SimplifiedChinese), "case is ignored");
        assert!(folded("ＡＣＭＥ", "adobe"), "full-width forms are the same letters: acme < adobe");
        assert!(folded("07NikumaruFont", "Arial"), "digits before letters");
        assert!(folded("Open Sans", "OpenSymbol"), "a space sorts before a letter, as in a dictionary");
        assert!(folded("Aa 厚底", "Aa厚底"), "and before a Han character, so the Latin name still leads");
    }

    /// A character the writing system's set doesn't have sorts after its own characters, by
    /// code point, and a label with no script at all still has a stable key.
    #[test]
    fn uncovered_characters_sort_by_code_point_after_the_script() {
        let mut names = ["漢字", "Arial", "字"];
        names.sort_by_cached_key(|n| key(n, CjkScript::SimplifiedChinese));
        assert_eq!(names, ["Arial", "字", "漢字"], "漢 is not in GB2312, 字 is");
        assert!(key("漢", CjkScript::SimplifiedChinese) > key("字", CjkScript::SimplifiedChinese));
        assert_eq!(menu_sort_key("字", None), vec![OTHER_SCRIPT | u32::from('字')], "no script: still a stable key");
    }

    /// Every writing system has a table with its own characters: Simplified Chinese has the
    /// GB2312 pinyin rows, Japanese the kana, Korean the Hangul syllables.
    #[test]
    fn every_writing_system_has_its_characters() {
        for (script, c) in [
            (CjkScript::SimplifiedChinese, '这'),
            (CjkScript::SimplifiedChinese, '说'),
            (CjkScript::TraditionalChinese, '們'),
            (CjkScript::TraditionalChinese, '體'),
            (CjkScript::Japanese, 'あ'),
            (CjkScript::Japanese, '図'),
            (CjkScript::Korean, '가'),
            (CjkScript::Korean, '한'),
        ] {
            assert!(set(script).rank(c).is_some(), "{script:?} has {c}");
        }
        assert!(set(CjkScript::SimplifiedChinese).rank('體').is_none(), "GB2312 has no Traditional forms");
        assert!(set(CjkScript::Japanese).rank('가').is_none(), "JIS X 0208 has no Hangul");
    }
}
