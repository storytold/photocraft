use super::*;
use photocraft_doc::LayerId;

fn text_of(s: &Session, id: LayerId) -> String {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn change_all(text: &str, params: Value) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8})).unwrap();
    let id = LayerId(s.execute("type.create", json!({"text": text, "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap());
    let mut p = json!({"replace": "tea", "action": "changeAll"});
    p.as_object_mut().unwrap().extend(params.as_object().unwrap().clone());
    s.execute("edit.findAndReplaceText", p).unwrap();
    text_of(&s, id)
}

#[test]
fn ignore_accents_matches_both_spellings_and_off_or_missing_stays_exact() {
    for find in ["cafe", "café"] {
        assert_eq!(change_all("café cafe", json!({"find": find, "ignoreAccents": true})), "tea tea", "on, {find}");
    }
    assert_eq!(change_all("café cafe", json!({"find": "cafe", "ignoreAccents": false})), "café tea");
    assert_eq!(change_all("café cafe", json!({"find": "café", "ignoreAccents": false})), "tea cafe");
    // Old actions and scripts don't send the param: matching stays exact.
    assert_eq!(change_all("café cafe", json!({"find": "cafe"})), "café tea");
}

#[test]
fn ignore_accents_works_with_case_and_whole_word() {
    let on = |find: &str, extra: Value| {
        let mut p = json!({"find": find, "ignoreAccents": true});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        change_all("CAFÉ Café cafés", p)
    };
    assert_eq!(on("cafe", json!({"caseSensitive": true})), "CAFÉ Café teas");
    assert_eq!(on("Cafe", json!({"caseSensitive": true})), "CAFÉ tea cafés");
    assert_eq!(on("cafe", json!({"wholeWord": true})), "tea tea cafés");
    // A word ending inside "écafé": composed and decomposed, the mark belongs to the "e" before it.
    for text in ["\u{e9}caf\u{e9}", "e\u{301}cafe\u{301}"] {
        assert!(find_matches_with(text, "cafe", false, true, true).is_empty(), "{text:?}");
        assert_eq!(find_matches_with(text, "cafe", false, false, true).len(), 1, "{text:?}");
    }
}

#[test]
fn ignored_accent_marks_stay_with_their_letter() {
    // "cafe" + COMBINING ACUTE ACCENT: the mark is part of the match, so no stray accent is left.
    let text = "cafe\u{301} x\u{301}cafe";
    let ms = find_matches_with(text, "cafe", false, false, true);
    assert_eq!(ms.len(), 2, "{ms:?}");
    for (a, b) in &ms {
        assert!(text.is_char_boundary(*a) && text.is_char_boundary(*b));
        let after = text[*b..].chars().next();
        assert!(!after.is_some_and(unicode_normalization::char::is_combining_mark), "match {a}..{b} leaves a mark behind");
    }
    assert_eq!(change_all(text, json!({"find": "cafe", "ignoreAccents": true})), "tea x\u{301}tea");
    // A find of only a combining mark matches nothing rather than every letter.
    assert!(find_matches_with(text, "\u{301}", false, false, true).is_empty());
}

#[test]
fn changing_ignore_accents_starts_a_new_search() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "depth": 8})).unwrap();
    let id = LayerId(s.execute("type.create", json!({"text": "cafe", "x": 2, "y": 12})).unwrap()["layer"].as_u64().unwrap());
    let f = s.execute("edit.findAndReplaceText", json!({"find": "cafe", "action": "find"})).unwrap();
    assert_eq!(f["found"]["text"], "cafe");
    // The found text matches either way, but a changed option is a new query: Change finds first.
    let c = s.execute("edit.findAndReplaceText", json!({"find": "cafe", "replace": "tea", "action": "change", "ignoreAccents": true})).unwrap();
    assert!(c["changed"].is_null(), "{c}");
    assert_eq!(text_of(&s, id), "cafe");
}
