//! What a font menu shows for a family besides its English name: the names the font gives
//! itself in other languages, the CJK writing systems it serves (the menu lists the interface
//! language's own fonts above the rest, as Photoshop's does) and whether it covers the sample
//! each of those writing systems shows in place of "AaBbCc".
//!
//! Everything comes from the family's representative face, read once per family by
//! [`crate::FontDb::menu_index`] — in the background font scan, so drawing the menu never reads
//! a font file.

use skrifa::raw::types::Tag;
use skrifa::raw::{FontRef, TableProvider};
use skrifa::string::StringId;
use skrifa::MetadataProvider;

use crate::cjk::{self, CjkScript};

/// The CJK writing systems a family serves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CjkScripts {
    pub hans: bool,
    pub hant: bool,
    pub jpan: bool,
    pub kore: bool,
}

impl CjkScripts {
    pub const NONE: Self = Self { hans: false, hant: false, jpan: false, kore: false };

    /// The scripts of a set, in [`CjkScript`] order.
    pub fn has(self, script: CjkScript) -> bool {
        match script {
            CjkScript::SimplifiedChinese => self.hans,
            CjkScript::TraditionalChinese => self.hant,
            CjkScript::Japanese => self.jpan,
            CjkScript::Korean => self.kore,
        }
    }

    pub fn any(self) -> bool {
        self.hans || self.hant || self.jpan || self.kore
    }

    pub(crate) fn set(&mut self, script: CjkScript) {
        match script {
            CjkScript::SimplifiedChinese => self.hans = true,
            CjkScript::TraditionalChinese => self.hant = true,
            CjkScript::Japanese => self.jpan = true,
            CjkScript::Korean => self.kore = true,
        }
    }
}

/// A family's entry in the font menu.
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyMenuInfo {
    /// The family's own names, `(language tag, name)`: what its `name` table writes for the
    /// typographic family (ID 16), or the family (ID 1) when it has none. The tag is skrifa's
    /// BCP 47 form of the record's language (`zh-CN`, `zh-TW`, `ja-JP`, `ko-KR`; empty for a
    /// record that names none). This is the list a label, the script classification and a
    /// single-face row read.
    pub names: Vec<(String, String)>,
    /// Every family name the family's faces write, under both name IDs (typographic and legacy),
    /// for search: a font whose ID 1 records differ from its ID 16 ones ("HYQiHei-45S") is found by
    /// either, and an extended family's other faces by their own names too, while the labels and
    /// grouping above keep the representative face's ID 16 names.
    pub search_names: Vec<String>,
    /// The representative face's full name (ID 4) in every language: "SimSun-ExtB Regular" where
    /// the family is "SimSun-ExtB". A menu shows it for a family with a single face whose style
    /// is plain Regular, as Photoshop lists such a family (see [`single_face_label`]).
    pub full_names: Vec<(String, String)>,
    /// The representative face's subfamilies: "常规", "Medium", "Wide ExtraLight". A single-face
    /// family is listed as "family + style"; several faces get a style menu.
    pub subfamilies: Vec<(String, String)>,
    /// How many faces the family has. A family of one face is listed by its full name, one of
    /// several by its family name and a style menu.
    pub faces: usize,
    /// The writing systems the family serves: the OS/2 code page bits Photoshop groups by when
    /// the font states them, else the languages it names itself in, else the characters it
    /// covers (see [`scripts_of`]).
    pub scripts: CjkScripts,
    /// The writing systems whose [`SAMPLES`] text the family's cmap maps to real glyphs. A menu
    /// shows a writing system's sample only where this says the font can draw it.
    pub samples: CjkScripts,
    /// OS/2 `ulCodePageRange1` (`0` when the table is too old to have one), kept for the traits
    /// the menu filters by and for diagnostics.
    pub code_pages: u32,
}

/// The sample the font menu shows for each writing system in place of "AaBbCc" (Photoshop shows
/// a Chinese font as "字体样式"). Kept short: the menu's sample slot is 100 px wide at 18 pt.
pub const SAMPLES: [(CjkScript, &str); 4] =
    [(CjkScript::SimplifiedChinese, "字体样式"), (CjkScript::TraditionalChinese, "字體樣式"), (CjkScript::Japanese, "あア亜"), (CjkScript::Korean, "가나다")];

/// Characters only a Simplified Chinese font has: all sixteen are in GB2312 and in none of
/// JIS X 0208, Big5 or KS X 1001, so a Japanese, Korean or Traditional Chinese font never
/// covers them however broad its character set.
const HANS_PROBE: &str = "这们为说时对过发经现动还进样开实";
/// How many of [`HANS_PROBE`] make a Simplified Chinese font; a broader font may also cover
/// characters outside GB2312, a partial one none.
const HANS_PROBE_MIN: usize = 14;
/// Kana, which every Japanese font has and no Chinese or Korean one does.
const JPAN_KANA: &str = "あいうアイウ";
/// Kanji only JIS X 0208 has (国·図 style simplifications and kana-font kanji).
const JPAN_PROBE: &str = "図駅広楽歩込働気";
/// Hangul syllables, which only a Korean font has.
const KORE_PROBE: &str = "가나다라한글국어";
/// Traditional forms that the Japanese and Korean sets also have (their fonts cover these
/// too), so [`HANT_BOPO`] is what tells a Traditional Chinese font apart.
const HANT_PROBE: &str = "們這個來為國說時會對學與關點";
/// Bopomofo, which only Traditional Chinese fonts have.
const HANT_BOPO: &str = "ㄅㄆㄇㄈ";

/// The most records one name ID is examined from, and the most text they may hold. A font can
/// point many records at one large string, or fill the table with ones that name nothing, so the
/// blob's size alone bounds neither the decoding work nor the memory (CWE-770); a menu lists a
/// handful of languages and no family name is longer than a sentence.
const MAX_NAME_RECORDS: usize = 64;
const MAX_NAME_BYTES: usize = 8 << 10;
/// The most searchable names a family keeps, all its faces together.
const MAX_SEARCH_NAMES: usize = 256;

/// The writing systems a family serves, from what it says about itself, in the order that names
/// the fewest fonts wrongly:
///
/// 1. The OS/2 code page bits, which is what Photoshop groups by: 18 (GB2312) Simplified
///    Chinese, 20 (Big5) Traditional, 17 (Shift-JIS) Japanese, 19 and 21 (Wansung/Johab) Korean.
///    The writing systems' fonts cover almost the same characters (Glow Sans SC, TC and J differ
///    only in preferred forms), so coverage cannot tell them apart; the bits can.
/// 2. The languages the font names itself in (a font without code page bits), for example
///    "Glow Sans SC" naming itself "未来荧黑" in zh-CN.
/// 3. The characters it covers, for a font that says neither. Han characters are shared, so
///    each script is probed with characters only its own standard set has.
pub fn scripts_of(names: &[(String, String)], code_pages: u32, covers: impl Fn(char) -> bool) -> CjkScripts {
    let bits = CjkScripts {
        hans: code_pages & (1 << 18) != 0,
        hant: code_pages & (1 << 20) != 0,
        jpan: code_pages & (1 << 17) != 0,
        kore: code_pages & ((1 << 19) | (1 << 21)) != 0,
    };
    if bits.any() {
        return bits;
    }
    let mut named = CjkScripts::NONE;
    for (lang, _) in names {
        if let Some(script) = cjk::locale_script(Some(lang)) {
            named.set(script);
        }
    }
    if named.any() {
        return named;
    }
    let count = |probe: &str| probe.chars().filter(|c| covers(*c)).count();
    let mut out = CjkScripts::NONE;
    if count(HANS_PROBE) >= HANS_PROBE_MIN {
        out.hans = true;
    }
    let jpan = count(JPAN_KANA) == JPAN_KANA.chars().count() && count(JPAN_PROBE) >= 7;
    let kore = count(KORE_PROBE) >= 7;
    if jpan {
        out.jpan = true;
    }
    if kore {
        out.kore = true;
    }
    // A Japanese or Korean font covers most Traditional forms too (JIS X 0208 and KS X 1001
    // carry them), so a Traditional Chinese font is one that also has Bopomofo.
    if !jpan && !kore && count(HANT_BOPO) >= 3 && count(HANT_PROBE) >= 13 {
        out.hant = true;
    }
    out
}

/// The name a font menu shows for a family: its own name for `prefer` — the interface
/// language's writing system, so a Simplified Chinese interface lists "未来荧黑" where a
/// Traditional one lists "未来熒黑" and a Japanese one "ヒカリ角ゴ" — else its first non-English
/// name (Photoshop's Show Font Names in English off lists an Asian font by its own name even in
/// an English interface), else `None` and the English family name stands.
pub fn display_name(english: &str, names: &[(String, String)], prefer: Option<CjkScript>) -> Option<String> {
    menu_name(english, names, prefer, false)
}

/// The name a menu shows out of a face's `(language tag, name)` records: with `english_names`, the
/// English one; else the record for `prefer`'s writing system, else the first non-English one. A
/// record without a language (a Mac platform name, usually the English one) is never a label, nor
/// is a blank name or one that repeats `english` (`""` when any record will do).
pub fn menu_name(english: &str, names: &[(String, String)], prefer: Option<CjkScript>, english_names: bool) -> Option<String> {
    let usable = |(lang, name): &(String, String)| {
        !lang.is_empty()
            && !name.trim().is_empty()
            && name != english
            && if english_names {
                is_english(lang)
            } else {
                !is_english(lang)
            }
    };
    if let Some(want) = prefer.filter(|_| !english_names)
        && let Some((_, name)) = names.iter().find(|n| cjk::locale_script(Some(&n.0)) == Some(want) && usable(n))
    {
        return Some(name.clone());
    }
    names.iter().find(|n| usable(n)).map(|(_, name)| name.clone())
}

/// Is a name-table language tag English (`en`, `en-US`)? English names are what the menus store,
/// so they are never a localized label.
fn is_english(lang: &str) -> bool {
    let primary = lang.split(['-', '_']).next().unwrap_or(lang);
    primary.eq_ignore_ascii_case("en")
}

/// The menu information of one face of a family with `faces` faces (1 for a single-style family).
pub fn info_of(font: &FontRef, faces: usize) -> FamilyMenuInfo {
    let names = names_of(font);
    let search_names = search_names_of(font);
    let full_names = records_of(font, StringId::FULL_NAME);
    let subfamilies = localized_subfamilies(font);
    let code_pages = font.os2().ok().and_then(|t| t.ul_code_page_range_1()).unwrap_or(0);
    let charmap = font.charmap();
    let covers = |c: char| charmap.map(c).is_some_and(|g| g.to_u32() != 0);
    let scripts = scripts_of(&names, code_pages, covers);
    let mut samples = CjkScripts::NONE;
    for (script, text) in SAMPLES {
        if text.chars().all(covers) {
            samples.set(script);
        }
    }
    FamilyMenuInfo { names, search_names, full_names, subfamilies, faces, scripts, samples, code_pages }
}

/// Every family name a face writes, under both name IDs, deduplicated: what a search matches.
fn search_names_of(font: &FontRef) -> Vec<String> {
    let mut names: Vec<String> = records_of(font, StringId::TYPOGRAPHIC_FAMILY_NAME)
        .into_iter()
        .chain(records_of(font, StringId::FAMILY_NAME))
        .map(|(_, name)| name)
        .collect();
    let mut seen: Vec<String> = Vec::with_capacity(names.len());
    names.retain(|name| {
        if seen.contains(name) {
            return false;
        }
        seen.push(name.clone());
        true
    });
    names.truncate(MAX_SEARCH_NAMES);
    names
}

/// Adds another face's family names to `info`'s search names: in an extended family (one ID 16,
/// several ID 1s) only the representative face reaches [`FamilyMenuInfo::names`], but every face's
/// names are searchable.
pub fn add_face_search_names(info: &mut FamilyMenuInfo, font: &FontRef) {
    for name in search_names_of(font) {
        if info.search_names.len() >= MAX_SEARCH_NAMES {
            return;
        }
        if !info.search_names.contains(&name) {
            info.search_names.push(name);
        }
    }
}

/// The family's own names: every record of the typographic family name (ID 16), or of the
/// family name (ID 1) when it has none.
fn names_of(font: &FontRef) -> Vec<(String, String)> {
    let typographic = records_of(font, StringId::TYPOGRAPHIC_FAMILY_NAME);
    if typographic.is_empty() { records_of(font, StringId::FAMILY_NAME) } else { typographic }
}

/// A face's subfamily names: the typographic subfamily (ID 17), or the legacy one (ID 2) when it
/// has none. A style menu shows the interface language's ("常规") where the font writes one.
pub fn localized_subfamilies(font: &FontRef) -> Vec<(String, String)> {
    let typographic = records_of(font, StringId::TYPOGRAPHIC_SUBFAMILY_NAME);
    if typographic.is_empty() { records_of(font, StringId::SUBFAMILY_NAME) } else { typographic }
}

/// A face's name records of one ID, without blanks or duplicates: the Unicode ones the font stack
/// reads, then names kept only in a legacy CJK encoding ([`legacy_records`]). Bounded by
/// [`MAX_NAME_RECORDS`] records examined — blank ones count, so a font whose records mostly name
/// nothing cannot make the menu walk the whole table — and by [`MAX_NAME_BYTES`] of text: the
/// name table is untrusted input.
fn records_of(font: &FontRef, id: StringId) -> Vec<(String, String)> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut bytes = 0usize;
    let mut examined = 0usize;
    for record in font.localized_strings(id) {
        if examined >= MAX_NAME_RECORDS {
            break;
        }
        examined += 1;
        let language = record.language().unwrap_or("").to_string();
        let name = record.to_string();
        if name.trim().is_empty() {
            continue;
        }
        if bytes + name.len() > MAX_NAME_BYTES {
            break;
        }
        bytes += name.len();
        names.push((language, name));
    }
    // The legacy records get what the Unicode ones left of both budgets.
    names.extend(legacy_records(font, id, MAX_NAME_BYTES.saturating_sub(bytes), MAX_NAME_RECORDS.saturating_sub(examined)));
    let mut seen: Vec<(String, String)> = Vec::with_capacity(names.len());
    names.retain(|name| {
        if seen.contains(name) {
            return false;
        }
        seen.push(name.clone());
        true
    });
    names
}

/// Name records the font stack can't read, from fonts that keep a localized name only in a legacy
/// CJK encoding: Mac platform records (Japanese Shift-JIS, Traditional Chinese Big5, Korean EUC-KR,
/// Simplified Chinese GB2312) and Windows ones (encodings 2–5). Decoded here so an old Chinese,
/// Japanese or Korean font still lists its own name; the tag comes from the encoding, which is
/// what these records are written in. Nothing on the web, which has no system fonts and must stay
/// under its size gate.
#[cfg(not(target_arch = "wasm32"))]
fn legacy_records(font: &FontRef, id: StringId, bytes_left: usize, records_left: usize) -> Vec<(String, String)> {
    /// The encoding a name record may use besides Unicode, and the language its text is in.
    fn codec(platform: u16, encoding: u16) -> Option<(&'static encoding_rs::Encoding, &'static str)> {
        use encoding_rs::{BIG5, EUC_KR, GBK, SHIFT_JIS};
        match (platform, encoding) {
            (1, 1) | (3, 2) => Some((SHIFT_JIS, "ja")),
            (1, 2) | (3, 4) => Some((BIG5, "zh-Hant")),
            (1, 3) | (3, 5) => Some((EUC_KR, "ko")),
            (1, 25) | (3, 3) => Some((GBK, "zh-Hans")),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let Some(data) = font.table_data(Tag::new(b"name")) else {
        return out;
    };
    let bytes = data.as_bytes();
    let be16 = |at: usize| -> Option<u16> { Some(u16::from_be_bytes(<[u8; 2]>::try_from(bytes.get(at..at.checked_add(2)?)?).ok()?)) };
    let (Some(count), Some(strings)) = (be16(2), be16(4).map(usize::from)) else {
        return out;
    };
    let mut budget = bytes_left;
    let mut examined = 0usize;
    for i in 0..usize::from(count) {
        if examined >= records_left {
            break;
        }
        let record = 6 + i * 12;
        let (Some(platform), Some(encoding), Some(name), Some(length), Some(offset)) =
            (be16(record), be16(record + 2), be16(record + 6), be16(record + 8), be16(record + 10))
        else {
            continue;
        };
        if name != id.to_u16() {
            continue;
        }
        let Some((codec, language)) = codec(platform, encoding) else {
            continue;
        };
        // The allowance pays for decoding, so only a record this reader can decode uses it up.
        examined += 1;
        let Some(start) = strings.checked_add(usize::from(offset)) else {
            continue;
        };
        let Some(raw) = start.checked_add(usize::from(length)).and_then(|end| bytes.get(start..end)) else {
            continue;
        };
        let (text, _, had_errors) = codec.decode(raw);
        if had_errors || text.trim().is_empty() {
            continue;
        }
        let text = text.into_owned();
        if text.len() > budget {
            break;
        }
        budget -= text.len();
        out.push((language.to_string(), text));
    }
    out
}

#[cfg(target_arch = "wasm32")]
fn legacy_records(_font: &FontRef, _id: StringId, _bytes_left: usize, _records_left: usize) -> Vec<(String, String)> {
    Vec::new()
}

/// The name `names` gives for `script` — the interface's writing system — when the font has
/// one in that language: a style label uses it and no other language's, so a Chinese menu shows
/// a font's own "Bold" rather than borrowing its Japanese subfamily.
pub fn name_for_script(names: &[(String, String)], script: CjkScript) -> Option<String> {
    names.iter().find(|(lang, name)| !name.trim().is_empty() && cjk::locale_script(Some(lang)) == Some(script)).map(|(_, name)| name.clone())
}

/// What a font menu calls a family with a single face, as Photoshop lists such a family by
/// "family + style": the style the font writes in the interface's language — "Aa厚底黑 常规",
/// "寒蝉宽黑体 Medium" — or, for a plain Regular, the font's full name when it names one beyond
/// the family ("SimSun-ExtB Regular", "LeeFont蒙黑体 Regular"; "Arial"'s is just "Arial").
/// `None` when the row needs nothing added: a Regular style on a font whose full name is its
/// family name, a style the family name already ends with, or a full name that names something
/// else entirely (a few fonts carry a junk record such as "0"). Several faces are listed under
/// the family name with a style menu instead. `english_names` takes the English names, as the
/// menu then shows English family names.
pub fn single_face_label(family: &str, info: &FamilyMenuInfo, prefer: Option<CjkScript>, english_names: bool) -> Option<String> {
    let family_label = menu_name(family, &info.names, prefer, english_names).unwrap_or_else(|| family.to_string());
    let style = menu_name("", &info.subfamilies, prefer, english_names)?;
    let (family_label, style) = (family_label.trim(), style.trim());
    if style.is_empty() {
        return None;
    }
    if style.eq_ignore_ascii_case("Regular") {
        let full = menu_name("", &info.full_names, prefer, english_names)?;
        // The style follows the family's name; a record that doesn't start with it would replace
        // the family with something else on the row.
        let full = full.trim();
        return (!full.is_empty() && full != family_label && full.starts_with(family_label)).then(|| full.to_string());
    }
    // A family that already ends with its style ("Noto Emoji Light" as a family name) says it once.
    if family_label.to_lowercase().ends_with(&style.to_lowercase()) {
        return None;
    }
    Some(format!("{family_label} {style}"))
}

#[cfg(test)]
mod tests;
