//! The Arabic draft is validated without making it a selectable UI language. UI bidi, caret
//! behaviour and web font delivery must be verified before it is added to the language registry.

use super::catalog::{Catalog, parse_entries, placeholders};
use super::*;

const DRAFT: &str = include_str!("ar.tsv");

#[test]
fn arabic_draft_preserves_catalog_contracts() {
    let (entries, errors) = parse_entries(DRAFT);
    assert!(errors.is_empty(), "Arabic draft: {errors:?}");
    assert!(!entries.is_empty(), "the Arabic draft must contain translations");
    let mut seen = std::collections::HashSet::new();
    for (context, source, translation) in &entries {
        assert!(seen.insert((context, source)), "duplicate Arabic key: {context:?} {source:?}");
        if context == "@plural" {
            let sources: Vec<_> = source.split('|').collect();
            assert_eq!(sources.len(), 2, "plural source must be one|other: {source:?}");
            assert_eq!(translation.split('|').count(), 6, "Arabic needs zero/one/two/few/many/other: {source:?}");
            let mut want = placeholders(sources[1]);
            want.sort_unstable();
            for form in translation.split('|') {
                let mut got = placeholders(form);
                got.sort_unstable();
                assert_eq!(want, got, "Arabic plural placeholders: {source:?}");
            }
            continue;
        }
        let mut want = placeholders(source);
        let mut got = placeholders(translation);
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(want, got, "Arabic placeholders: {context:?} {source:?}");
        if context.is_empty() {
            assert_eq!(source.ends_with('…'), translation.ends_with('…'), "Arabic dialog ellipsis: {source:?}");
        }
        if context == "@id" {
            assert!(crate::menus::is_live(source) || crate::menu_catalog::CATALOG.iter().any(|m| m.3 == source), "unknown Arabic command id: {source:?}");
        }
    }
}

#[test]
fn packed_arabic_draft_matches_its_source() {
    assert_eq!(unpack(catalog!("ar")).expect("packed Arabic draft"), DRAFT);
}

#[test]
fn arabic_draft_tracks_live_menu_tool_and_document_category_labels() {
    let catalog = Catalog::parse(DRAFT);
    let menus: std::collections::BTreeSet<_> = crate::menu_catalog::CATALOG
        .iter()
        .filter_map(|(path, _, _, _)| path.first().copied())
        .chain(crate::menus::TOP_MENUS)
        .collect();
    for source in menus {
        assert!(catalog.plain(source).is_some(), "untranslated Arabic main menu: {source}");
    }
    for tool in crate::state::Tool::ALL {
        assert!(catalog.plain(tool.label()).is_some(), "untranslated Arabic tool: {}", tool.label());
    }
    for &(category, _) in crate::new_doc_ui::CATEGORIES {
        assert!(catalog.plain(category).is_some(), "untranslated Arabic New Document category: {category}");
    }
}

#[test]
fn arabic_draft_covers_the_main_menus_and_new_document_workflow() {
    let catalog = Catalog::parse(DRAFT);
    for (source, translation) in [
        ("File", "ملف"),
        ("Layer", "طبقة"),
        ("New Document", "مستند جديد"),
        ("Width", "العرض"),
        ("Height", "الارتفاع"),
        ("Create", "إنشاء"),
        ("Cancel", "إلغاء"),
    ] {
        assert_eq!(catalog.plain(source), Some(translation), "Arabic workflow label: {source}");
    }
    assert_eq!(catalog.contextual("document", "Orientation"), Some("اتجاه الصفحة"));
    assert_eq!(catalog.contextual("unrelated", "Orientation"), None, "a document-specific meaning stays scoped");
    let presets = catalog.plain("BLANK DOCUMENT PRESETS ({n})").expect("New Document preset count");
    assert_eq!(fmt(presets, &[("n", "12")]), "إعدادات مسبقة لمستندات فارغة (12)");
    let cancelled = catalog.plain("Cancelled {label}").expect("cancellation label");
    assert_eq!(fmt(cancelled, &[("label", "خلفية {Original} 2026.psd")]), "تم إلغاء خلفية {Original} 2026.psd", "user-provided names are preserved");
}

#[test]
fn arabic_draft_does_not_activate_an_unverified_interface() {
    assert!(!Lang::all().any(|lang| lang.code() == "ar"), "Arabic stays staged until rendering is verified");
    for code in ["ar", "AR"] {
        assert_eq!(Lang::from_code(code), None, "staged code: {code}");
    }
    for tag in ["ar", "ar-LB", "ar-SA", "ar_EG.UTF-8"] {
        assert_eq!(lang_from_tag(tag), None, "staged locale: {tag}");
    }
    system::with_system_tags(&["ar-LB"], || {
        assert_eq!(system_lang(), Lang::EN);
        assert_eq!(Lang::from_pref("auto"), Lang::EN);
        assert_eq!(Lang::from_pref("ar"), Lang::EN);
    });
    system::with_system_tags(&["ar-LB", "fr-FR"], || {
        assert_eq!(system_lang().code(), "fr", "Auto skips an unregistered Arabic draft");
        assert_eq!(Lang::from_pref("en"), Lang::EN, "saved manual choices take precedence");
    });
    assert_eq!(dir_for_code("ar"), Dir::Rtl, "the existing direction rule remains available");
}
