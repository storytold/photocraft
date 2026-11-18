use super::*;

fn assert_settings_limit<T>(result: Result<T>) {
    match result {
        Err(error) => assert!(error.to_string().contains("too many native font features"), "{error}"),
        Ok(_) => panic!("duplicate or empty source settings must count toward the parsing allowance"),
    }
}

#[test]
fn inherited_duplicate_feature_lists_are_bounded_before_descendant_amplification() {
    let settings = vec!["kern=1"; 60_000].join(",");
    assert_settings_limit(features(&settings));
    let markup = format!("<markup><span font_features='{settings}'>{}</span></markup>", "<b>x</b>".repeat(1_000));
    assert_settings_limit(markup_runs(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new()));
}

#[test]
fn feature_work_limit_counts_empty_slots_and_stops_before_later_invalid_settings() {
    assert_settings_limit(features(&",".repeat(60_000)));
    let settings = format!("{},invalid-tag=1", vec!["kern=1"; MAX_FEATURE_SETTINGS].join(","));
    assert_settings_limit(features(&settings));
}

#[test]
fn feature_settings_at_the_limit_keep_order_and_last_value_semantics() {
    let settings = format!("{},kern=0", vec!["kern=1"; MAX_FEATURE_SETTINGS - 1].join(","));
    assert_eq!(features(&settings).unwrap(), [FontFeature { tag: "kern".into(), value: 0 }]);
    let distinct = (0..MAX_FEATURE_SETTINGS).map(|i| format!("a{i:03}=1")).collect::<Vec<_>>().join(",");
    let parsed = features(&distinct).unwrap();
    assert_eq!(parsed.len(), MAX_FEATURE_SETTINGS);
    assert_eq!(parsed[0].tag, "a000");
    assert_eq!(parsed[MAX_FEATURE_SETTINGS - 1].tag, "a063");
    assert_eq!(
        features(", kern=0,, +liga, -dlig, ").unwrap().iter().map(|f| (f.tag.as_str(), f.value)).collect::<Vec<_>>(),
        [("kern", 0), ("liga", 1), ("dlig", 0)]
    );
    let markup = format!("<markup><span font_features='{settings}'><b>A</b><i>B</i></span></markup>");
    let (text, runs) = markup_runs(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut Vec::new()).unwrap();
    assert_eq!(text, "AB");
    assert!(runs.iter().all(|run| run.style.features == [FontFeature { tag: "kern".into(), value: 0 }]));
    assert_eq!(runs[0].style.weight, 700);
    assert!(runs[1].style.italic);
}
