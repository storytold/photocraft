//! Menu information from synthetic fonts: names in several languages, OS/2 code page bits and
//! the script probes, without redistributing a real CJK font.

use super::*;

/// The `name`-table language a test record uses.
fn lang_id(lang: &str) -> u16 {
    match lang {
        "en-US" => 0x0409,
        // skrifa reports Windows' Chinese language IDs as "zh-Hans" and "zh-TW".
        "zh-Hans" | "zh-CN" => 0x0804,
        "zh-Hant" | "zh-TW" => 0x0404,
        "ja-JP" => 0x0411,
        "ko-KR" => 0x0412,
        other => panic!("no language id for {other}"),
    }
}

/// `source` with its `name` table rebuilt from `records` — `(language, name ID, text)` — and, when
/// given, an OS/2 `ulCodePageRange1` (`None` leaves the table as it is).
fn rebuilt(source: &[u8], records: &[(&str, u16, &str)], code_pages: Option<u32>) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut name_record = 0;
    for i in 0..count {
        let r = 12 + i * 16;
        let offset = u32::from_be_bytes(bytes[r + 8..r + 12].try_into().unwrap()) as usize;
        match &bytes[r..r + 4] {
            b"name" => name_record = r,
            b"OS/2" => {
                // Version 1 tables carry the code page ranges at offset 78.
                if let Some(pages) = code_pages {
                    // `version` at 0, `ulCodePageRange1` at 78 in the version 1 layout.
                    bytes[offset..offset + 2].copy_from_slice(&1u16.to_be_bytes());
                    bytes[offset + 78..offset + 82].copy_from_slice(&pages.to_be_bytes());
                }
            }
            _ => {}
        }
    }
    let mut table = Vec::new();
    let mut strings = Vec::new();
    for value in [0u16, records.len() as u16, (6 + records.len() * 12) as u16] {
        table.extend(value.to_be_bytes());
    }
    for (lang, id, text) in records {
        let encoded: Vec<_> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3, 1, lang_id(lang), *id, encoded.len() as u16, strings.len() as u16] {
            table.extend(n.to_be_bytes());
        }
        strings.extend(encoded);
    }
    table.extend(strings);
    let offset = bytes.len() as u32;
    bytes[name_record + 8..name_record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[name_record + 12..name_record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    bytes.extend(table);
    bytes
}

fn info(records: &[(&str, u16, &str)], code_pages: Option<u32>) -> FamilyMenuInfo {
    let bytes = rebuilt(crate::fonts::INTER_REGULAR.as_slice(), records, code_pages);
    let font = FontRef::new(&bytes).expect("synthetic font parses");
    info_of(&font, 1)
}

/// OS/2 code page bits are what Photoshop groups by: 18 is GB2312, 20 Big5, 17 Shift-JIS, 19/21
/// Korean — the only signal that tells Glow Sans SC, TC and J apart, since their coverage is the
/// same.
#[test]
fn code_page_bits_name_the_writing_system() {
    let scripts = |pages: u32| scripts_of(&[], pages, |_| false);
    assert_eq!(scripts(1 << 18), CjkScripts { hans: true, ..CjkScripts::NONE });
    assert_eq!(scripts(1 << 20), CjkScripts { hant: true, ..CjkScripts::NONE });
    assert_eq!(scripts(1 << 17), CjkScripts { jpan: true, ..CjkScripts::NONE });
    assert_eq!(scripts(1 << 19), CjkScripts { kore: true, ..CjkScripts::NONE });
    assert_eq!(scripts(1 << 21), CjkScripts { kore: true, ..CjkScripts::NONE });
    assert_eq!(scripts(0), CjkScripts::NONE, "no bits: nothing claimed without names or coverage");
}

/// A font without code page bits is read from the languages it names itself in.
#[test]
fn name_languages_decide_without_code_page_bits() {
    let scripts = |langs: &[(&str, &str)]| {
        let names: Vec<(String, String)> = langs.iter().map(|(l, n)| (l.to_string(), n.to_string())).collect();
        scripts_of(&names, 0, |_| false)
    };
    assert_eq!(scripts(&[("en-US", "Glow Sans SC"), ("zh-CN", "未来荧黑")]), CjkScripts { hans: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&[("zh-TW", "未来熒黑")]), CjkScripts { hant: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&[("ja-JP", "ヒカリ角ゴ")]), CjkScripts { jpan: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&[("ko-KR", "맑은 고딕")]), CjkScripts { kore: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&[("en-US", "Arial")]), CjkScripts::NONE, "an English-only font is no CJK font");
}

/// A font that says nothing (no code page bits, English names only) is read from its coverage,
/// with characters only one standard set has.
#[test]
fn coverage_decides_when_the_font_says_nothing() {
    let covering = |text: &'static str| {
        move |c: char| text.contains(c)
    };
    let scripts = |covers: &dyn Fn(char) -> bool| scripts_of(&[], 0, covers);
    assert_eq!(scripts(&covering(HANS_PROBE)), CjkScripts { hans: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&covering("あいうアイウ図駅広楽歩込働気")), CjkScripts { jpan: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&covering("가나다라한글국어")), CjkScripts { kore: true, ..CjkScripts::NONE });
    assert_eq!(scripts(&covering("們這個來為國說時會對學與關點ㄅㄆㄇㄈ")), CjkScripts { hant: true, ..CjkScripts::NONE });
    // A Japanese or Korean font covers most Traditional forms too: without Bopomofo it is no
    // Traditional Chinese font.
    assert_eq!(scripts(&covering("們這個來為國說時會對學與關點")), CjkScripts::NONE);
    assert_eq!(scripts(&covering("あいうアイウ図駅広楽歩込働気們這個來為國說時會對學與關點")), CjkScripts { jpan: true, ..CjkScripts::NONE });
    // Few characters is no evidence.
    assert_eq!(scripts(&covering("这们为")), CjkScripts::NONE);
}

/// The menu shows a family by its own name in the interface's language, whatever order the
/// `name` table records come in: a font with a Traditional record first and a Simplified one
/// after reads Simplified on a Simplified Chinese interface, and "未来荧黑" not "Glow Sans SC".
#[test]
fn the_interface_language_picks_the_name() {
    let names: Vec<(String, String)> = [
        ("en-US", "Glow Sans SC"),
        ("zh-TW", "未来熒黑"),
        ("zh-CN", "未来荧黑"),
    ]
    .iter()
    .map(|(l, n)| (l.to_string(), n.to_string()))
    .collect();
    let pick = |script| display_name("Glow Sans SC", &names, script);
    assert_eq!(pick(Some(CjkScript::SimplifiedChinese)).as_deref(), Some("未来荧黑"));
    assert_eq!(pick(Some(CjkScript::TraditionalChinese)).as_deref(), Some("未来熒黑"));
    assert_eq!(pick(Some(CjkScript::Japanese)).as_deref(), Some("未来熒黑"), "no Japanese name: the first non-English one");
    assert_eq!(pick(None).as_deref(), Some("未来熒黑"), "English interface, names off: Photoshop's behaviour");
    // A family named only in English has no label to show.
    let english: Vec<(String, String)> = vec![("en-US".into(), "Arial".into())];
    assert_eq!(display_name("Arial", &english, Some(CjkScript::SimplifiedChinese)), None);
    // A name equal to the English one (a Mac record of the same family) is no label either.
    let same: Vec<(String, String)> = vec![(String::new(), "Arial".into()), ("ja-JP".into(), "アリアル".into())];
    assert_eq!(display_name("Arial", &same, Some(CjkScript::SimplifiedChinese)).as_deref(), Some("アリアル"));
}

/// End to end: a font whose `name` table has an English and a Simplified Chinese name reports
/// both, classified by its code page bit, and can draw the Simplified sample only if its cmap
/// maps it (Inter has no CJK glyphs, so it cannot).
#[test]
fn info_reads_names_classification_and_sample_coverage() {
    let named = info(
        &[("en-US", 16, "Gothic Test"), ("zh-CN", 16, "未来荧黑")],
        Some(1 << 18),
    );
    // skrifa names language 0x0804 "zh-Hans" (Windows lists it as Chinese, `zh-CN`).
    assert_eq!(named.names, [("en-US".to_string(), "Gothic Test".to_string()), ("zh-Hans".to_string(), "未来荧黑".to_string())]);
    assert_eq!(named.scripts, CjkScripts { hans: true, ..CjkScripts::NONE });
    assert_eq!(named.samples, CjkScripts::NONE, "Inter has no CJK glyphs");
    assert_eq!(named.code_pages, 1 << 18);
    let plain = info(&[("en-US", 16, "Gothic Test")], None);
    assert_eq!(plain.scripts, CjkScripts::NONE, "Inter states Latin code pages only, so no CJK ones");
    assert_eq!(plain.code_pages & (0b11111 << 17), 0, "no CJK code page bits");
}

/// A font may point many name records at large strings; the reader keeps a bounded handful, so a
/// small file cannot make the menu allocate without limit (CWE-770).
#[test]
fn name_records_are_read_within_bounds() {
    // 100 records of 200 characters: more text than the reader may keep. (Shorter than 65535 bytes
    // in all, since a name record's string offset is a `u16`.)
    let big = "N".repeat(200);
    let text: Vec<u8> = big.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let records: Vec<(u16, u16, u16, u16, &[u8])> = (0..100).map(|_| (3, 1, 0x0409, 16, text.as_slice())).collect();
    let table = rebuilt_legacy(crate::fonts::INTER_REGULAR.as_slice(), &records);
    let font = FontRef::new(&table).expect("synthetic font parses");
    let names = records_of(&font, StringId::TYPOGRAPHIC_FAMILY_NAME);
    assert!(!names.is_empty(), "the first records are read");
    assert!(names.len() <= 64, "{} records", names.len());
    let total: usize = names.iter().map(|(_, name)| name.len()).sum();
    assert!(total <= 8 * 1024, "{total} bytes of name text");
}

/// A font whose records mostly name nothing is bounded the same way: blank records count as
/// examined, so the menu cannot be made to walk a table of them, and a handful before the real
/// names changes nothing.
#[test]
fn blank_records_count_toward_the_allowance() {
    let blank: &[u8] = &[];
    let mut records: Vec<(u16, u16, u16, u16, &[u8])> = vec![(3, 1, 0x0409, 16, blank); 70];
    let late: Vec<u8> = "Late Name".encode_utf16().flat_map(u16::to_be_bytes).collect();
    records.push((3, 1, 0x0409, 16, &late));
    let table = rebuilt_legacy(crate::fonts::INTER_REGULAR.as_slice(), &records);
    let font = FontRef::new(&table).expect("synthetic font parses");
    assert!(records_of(&font, StringId::TYPOGRAPHIC_FAMILY_NAME).is_empty(), "seventy blanks use the allowance up");
    // A few blanks first are fine: the names after them still list.
    let mut records: Vec<(u16, u16, u16, u16, &[u8])> = vec![(3, 1, 0x0409, 16, blank); 3];
    records.push((3, 1, 0x0409, 16, &late));
    let table = rebuilt_legacy(crate::fonts::INTER_REGULAR.as_slice(), &records);
    let font = FontRef::new(&table).expect("synthetic font parses");
    assert_eq!(records_of(&font, StringId::TYPOGRAPHIC_FAMILY_NAME), [("en-US".to_string(), "Late Name".to_string())]);
}

/// Records of another name ID never use up the legacy allowance: a font that lists its legacy
/// family name (ID 1) beside the typographic one (ID 16) still has the ID 16 one read.
#[test]
fn legacy_records_of_other_ids_do_not_use_up_the_allowance() {
    let gb2312 = [0xCE_u8, 0xB4, 0xC0, 0xB4, 0xD3, 0xAB, 0xBA, 0xDA];
    let mut records: Vec<(u16, u16, u16, u16, &[u8])> = (0..100).map(|_| (1, 25, 33, 1, gb2312.as_slice())).collect();
    records.push((1, 25, 33, 16, &gb2312));
    let table = rebuilt_legacy(crate::fonts::INTER_REGULAR.as_slice(), &records);
    let font = FontRef::new(&table).expect("synthetic font parses");
    assert_eq!(records_of(&font, StringId::TYPOGRAPHIC_FAMILY_NAME), [("zh-Hans".to_string(), "未来荧黑".to_string())]);
}

/// A font whose legacy family name (ID 1) differs from its typographic one (ID 16) is found by
/// either, while its label keeps the typographic name.
#[test]
fn search_covers_both_family_name_ids() {
    let named = info(
        &[
            ("en-US", 16, "HYQiHei"),
            ("en-US", 1, "HYQiHei-45S ExtraLight"),
            ("en-US", 4, "HYQiHei-45S ExtraLight"),
            ("en-US", 17, "45S"),
        ],
        None,
    );
    assert_eq!(named.names, [("en-US".to_string(), "HYQiHei".to_string())], "the menu label stays the ID 16 name");
    assert_eq!(
        named.search_names,
        ["HYQiHei".to_string(), "HYQiHei-45S ExtraLight".to_string()],
        "and the search knows the ID 1 name too"
    );
    // A font with no typographic name uses its ID 1 names for both.
    let legacy = info(&[("en-US", 1, "ChillDINGothic Bold"), ("en-US", 17, "Bold")], None);
    assert_eq!(legacy.names, legacy.search_names.iter().map(|name| ("en-US".to_string(), name.clone())).collect::<Vec<_>>());
}

/// A family with a single face is listed by "family + style", as Photoshop's Chinese menu lists
/// these fonts: "Aa厚底黑 常规" (the style the font writes in that language), "寒蝉宽黑体 Medium".
/// A plain Regular adds the font's full name when that says more than the family name
/// ("SimSun-ExtB Regular") and nothing when it doesn't ("Arial").
#[test]
fn a_single_face_family_shows_its_style() {
    let named = info(
        &[
            ("en-US", 16, "Aa HouDiHei"),
            ("zh-Hans", 16, "Aa厚底黑"),
            ("en-US", 4, "AaHouDiHei"),
            ("zh-Hans", 4, "Aa厚底黑"),
            ("en-US", 17, "Regular"),
            ("zh-Hans", 17, "常规"),
        ],
        None,
    );
    assert_eq!(named.faces, 1);
    let label = |script, english| single_face_label("Aa HouDiHei", &named, script, english);
    assert_eq!(label(Some(CjkScript::SimplifiedChinese), false).as_deref(), Some("Aa厚底黑 常规"), "Photoshop's row");
    assert_eq!(label(Some(CjkScript::SimplifiedChinese), true).as_deref(), None, "English names: the family alone, as in an English menu");
    // A Regular style: the full name when it continues the family's name, nothing when it is the
    // family name.
    let simsun = info(&[("en-US", 16, "SimSun-ExtB"), ("en-US", 4, "SimSun-ExtB Regular"), ("en-US", 17, "Regular")], None);
    assert_eq!(single_face_label("SimSun-ExtB", &simsun, None, true).as_deref(), Some("SimSun-ExtB Regular"));
    let arial = info(&[("en-US", 16, "Arial"), ("en-US", 4, "Arial"), ("en-US", 17, "Regular")], None);
    assert_eq!(single_face_label("Arial", &arial, None, true), None);
    // A junk full name record is ignored rather than replacing the family name on the row.
    let junk = info(&[("en-US", 16, "PangMenZhengDao"), ("zh-Hans", 16, "庞门正道粗书体"), ("zh-Hans", 4, "0"), ("zh-Hans", 17, "Regular")], None);
    assert_eq!(single_face_label("PangMenZhengDao", &junk, Some(CjkScript::SimplifiedChinese), false), None);
    // A family name that already ends with the style says it once.
    let light = info(&[("en-US", 16, "Noto Emoji Light"), ("en-US", 17, "Light")], None);
    assert_eq!(single_face_label("Noto Emoji Light", &light, None, true), None);
    // No subfamily at all: nothing to add.
    let nameless = info(&[("en-US", 16, "Hanchan")], None);
    assert_eq!(single_face_label("Hanchan", &nameless, None, true), None);
}

/// `source` with its `name` table rebuilt from raw records — `(platform, encoding, language,
/// name ID, bytes)` — for the legacy encodings the font stack can't decode.
fn rebuilt_legacy(source: &[u8], records: &[(u16, u16, u16, u16, &[u8])]) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut name_record = 0;
    for i in 0..count {
        let r = 12 + i * 16;
        if &bytes[r..r + 4] == b"name" {
            name_record = r;
        }
    }
    let mut table = Vec::new();
    let mut strings = Vec::new();
    for value in [0u16, records.len() as u16, (6 + records.len() * 12) as u16] {
        table.extend(value.to_be_bytes());
    }
    for (platform, encoding, language, id, text) in records {
        for n in [*platform, *encoding, *language, *id, text.len() as u16, strings.len() as u16] {
            table.extend(n.to_be_bytes());
        }
        strings.extend_from_slice(text);
    }
    table.extend(strings);
    let offset = bytes.len() as u32;
    bytes[name_record + 8..name_record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[name_record + 12..name_record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    bytes.extend(table);
    bytes
}

/// Old fonts keep a localized name only in a legacy CJK encoding — Mac GB2312, Big5, EUC-KR or
/// Windows Shift-JIS — which the font stack can't decode; the menu reads it itself, tagged with
/// the language the encoding is for. Undecodable bytes are dropped rather than shown as mojibake.
#[test]
fn legacy_encoded_names_are_read() {
    let read = |platform: u16, encoding: u16, language: u16, text: &[u8]| {
        let bytes = rebuilt_legacy(crate::fonts::INTER_REGULAR.as_slice(), &[(platform, encoding, language, 16, text)]);
        let font = FontRef::new(&bytes).expect("synthetic font parses");
        info_of(&font, 1).names
    };
    // 未来荧黑, 未來熒黑, ヒカリ角ゴ and 맑은 고딕 in the encodings an old font would use.
    let gb2312 = [0xCE_u8, 0xB4, 0xC0, 0xB4, 0xD3, 0xAB, 0xBA, 0xDA];
    let big5 = [0xA5_u8, 0xBC, 0xA8, 0xD3, 0xBA, 0xB7, 0xB6, 0xC2];
    let shift_jis = [0x83_u8, 0x71, 0x83, 0x4A, 0x83, 0x8A, 0x8A, 0x70, 0x83, 0x53];
    let euc_kr = [0xB8_u8, 0xBC, 0xC0, 0xBA, 0x20, 0xB0, 0xED, 0xB5, 0xF1];
    assert_eq!(read(1, 25, 33, &gb2312), [("zh-Hans".to_string(), "未来荧黑".to_string())], "Mac Simplified Chinese");
    assert_eq!(read(1, 2, 19, &big5), [("zh-Hant".to_string(), "未來熒黑".to_string())], "Mac Traditional Chinese");
    assert_eq!(read(3, 2, 0x0411, &shift_jis), [("ja".to_string(), "ヒカリ角ゴ".to_string())], "Windows Shift-JIS");
    assert_eq!(read(1, 3, 23, &euc_kr), [("ko".to_string(), "맑은 고딕".to_string())], "Mac Korean");
    assert_eq!(read(1, 25, 33, &[0xFF, 0xFF]), Vec::new(), "undecodable bytes are no name");
    // The menu shows it, and a Unicode record of the same ID would still win by coming first.
    let bytes = rebuilt_legacy(crate::fonts::INTER_REGULAR.as_slice(), &[(1, 25, 33, 16, &gb2312)]);
    let font = FontRef::new(&bytes).expect("synthetic font parses");
    let info = info_of(&font, 1);
    assert_eq!(display_name("Gothic Test", &info.names, Some(CjkScript::SimplifiedChinese)).as_deref(), Some("未来荧黑"));
    assert_eq!(display_name("Gothic Test", &info.names, Some(CjkScript::TraditionalChinese)).as_deref(), Some("未来荧黑"), "no Traditional name: the first other one");
}

/// A face's subfamilies: the typographic one (ID 17) when it has it, else the legacy one (ID 2),
/// in every language, so a style menu can show "常规" where the font writes it.
#[test]
fn subfamilies_come_in_every_language() {
    let bytes = rebuilt(
        crate::fonts::INTER_REGULAR.as_slice(),
        &[("en-US", 17, "Wide ExtraLight"), ("zh-Hans", 17, "宽 极细"), ("en-US", 2, "Regular"), ("zh-Hans", 2, "常规")],
        None,
    );
    let font = FontRef::new(&bytes).expect("synthetic font parses");
    let styles = localized_subfamilies(&font);
    assert_eq!(styles, [("en-US".to_string(), "Wide ExtraLight".to_string()), ("zh-Hans".to_string(), "宽 极细".to_string())]);
    // A font with only the legacy subfamily (ID 2) is read from it.
    let legacy = rebuilt(crate::fonts::INTER_REGULAR.as_slice(), &[("en-US", 2, "Regular"), ("zh-Hans", 2, "常规")], None);
    let font = FontRef::new(&legacy).expect("synthetic font parses");
    assert_eq!(localized_subfamilies(&font), [("en-US".to_string(), "Regular".to_string()), ("zh-Hans".to_string(), "常规".to_string())]);
}

/// The sample texts are short enough for the menu's 100 px sample slot at 18 pt and show each
/// writing system's own characters.
#[test]
fn samples_are_the_writing_systems_own_text() {
    let sample = |script: CjkScript| SAMPLES.iter().find(|(s, _)| *s == script).map(|(_, t)| *t);
    assert_eq!(sample(CjkScript::SimplifiedChinese), Some("字体样式"));
    assert_eq!(sample(CjkScript::TraditionalChinese), Some("字體樣式"));
    for (_, text) in SAMPLES {
        assert!(text.chars().count() <= 4, "{text} fits the sample slot");
        assert!(text.chars().all(|c| crate::cjk::classify(c).is_some()), "{text} is CJK");
    }
}

/// A machine with the Glow Sans family installed (a font that names itself in Chinese, Taiwanese
/// and Japanese): the menu shows each face under its own name for the interface's language, and
/// the Chinese list reads in pinyin order as Photoshop's does. Run it on such a machine with
/// `cargo test -p photocraft-text --lib installed_glow -- --ignored --nocapture`.
#[test]
#[ignore = "requires the installed Glow Sans family; no font files are distributed"]
fn installed_glow_sans_names_and_orders_its_faces() {
    use crate::collate::menu_sort_key;
    let mut db = crate::FontDb::with_system_fonts();
    let index = db.menu_index();
    let label = |family: &str, script| display_name(family, &index[family].names, Some(script));
    let hans = label("Glow Sans SC", CjkScript::SimplifiedChinese).unwrap_or_default();
    let hant = label("Glow Sans TC", CjkScript::TraditionalChinese).unwrap_or_default();
    let jpan = label("Glow Sans J", CjkScript::Japanese).unwrap_or_default();
    println!("Glow Sans SC → {hans}, TC → {hant}, J → {jpan}");
    assert!(hans.ends_with("荧黑"), "the Simplified name: {hans}");
    assert!(hant.ends_with("熒黑"), "the Traditional name: {hant}");
    assert!(jpan.ends_with("角ゴ"), "the Japanese name: {jpan}");
    // The Chinese families' labels, in the menu's order: pinyin for Simplified Chinese.
    let mut labels: Vec<String> = index
        .iter()
        .filter(|(_, info)| info.scripts.hans)
        .filter_map(|(family, info)| display_name(family, &info.names, Some(CjkScript::SimplifiedChinese)))
        .collect();
    labels.sort_by_cached_key(|l| (menu_sort_key(l, Some(CjkScript::SimplifiedChinese)), l.to_lowercase()));
    let first = labels.iter().take(8).cloned().collect::<Vec<_>>().join(" · ");
    println!("Simplified Chinese, in menu order: {first}");
    assert!(labels.len() >= 5, "the machine has Chinese fonts: {labels:?}");
    let ordered = labels.windows(2).all(|w| menu_sort_key(&w[0], Some(CjkScript::SimplifiedChinese)) <= menu_sort_key(&w[1], Some(CjkScript::SimplifiedChinese)));
    assert!(ordered, "the sorted list stays in menu order");
}

/// A machine with Chinese fonts installed: a family of one face is listed by its full name, the
/// way Photoshop's Chinese menu lists these fonts (图 2 of the bug report read "快看世界体 标准"
/// where the family is "快看世界体"). Run with
/// `cargo test -p photocraft-text --lib installed_chinese_single -- --ignored --nocapture`.
#[test]
#[ignore = "requires installed Chinese fonts; no font files are distributed"]
fn installed_chinese_single_face_fonts_show_their_full_names() {
    let mut db = crate::FontDb::with_system_fonts();
    let index = db.menu_index();
    let label = |family: &str| {
        index
            .get(family)
            .filter(|info| info.faces == 1)
            .and_then(|info| single_face_label(family, info, Some(CjkScript::SimplifiedChinese), false))
    };
    for family in ["AaHouDiHei", "HYQiHei", "HYZhongHei", "PangMenZhengDao-Cu6.0", "Inter"] {
        println!("{family} → {:?}", label(family));
    }
    assert_eq!(label("AaHouDiHei").as_deref(), Some("Aa厚底黑 常规"), "Photoshop's own row for this font");
    for family in ["HYQiHei", "HYZhongHei"] {
        let row = label(family).unwrap_or_default();
        assert!(row.starts_with("汉仪") && row.contains(' '), "{family} reads as family + style: {row}");
    }
    assert_eq!(label("PangMenZhengDao-Cu6.0"), None, "a junk full name record is ignored");
    assert_eq!(label("Inter"), None, "an English-only family adds nothing");
}
