use super::*;

#[test]
fn inherited_long_multibyte_diagnostics_stay_bounded_across_many_descendants() {
    // The cutoff falls inside a multibyte scalar. A single large source value
    // must not be copied in full for each otherwise-small native text run.
    let value = format!("aa{}", "界".repeat(170_000));
    let children = "<b>x</b>".repeat(1_000);
    for attribute in ["variant", "font_variant", "text_transform"] {
        let markup = format!("<markup><span {attribute}='{value}'>{children}</span></markup>");
        let mut notes = Vec::new();
        let (text, runs) = markup_runs(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut notes).unwrap();
        assert_eq!(text, "x".repeat(1_000));
        assert_eq!(runs.len(), 1_000);
        assert!(runs.iter().all(|run| run.style.weight == 700 && run.style.caps == Caps::Normal));
        assert_eq!(notes.len(), 1, "one inherited disclosure: {attribute}");
        assert!(notes[0].len() < 320, "diagnostic must be bounded before descendant formatting");
        assert!(notes[0].contains(&format!("aa{}…", "界".repeat(84))));
        assert!(notes[0].ends_with(" remains inert"));
    }
}

#[test]
fn inherited_long_element_and_attribute_names_have_bounded_disclosures() {
    let name = format!("x{}", "界".repeat(2_000));
    let children = "<i>x</i>".repeat(100);
    for markup in [format!("<markup><{name}>{children}</{name}></markup>"), format!("<markup><span {name}='inert'>{children}</span></markup>")] {
        let mut notes = Vec::new();
        let (text, runs) = markup_runs(&markup, &CharStyle::default(), &HashMap::new(), 72., &mut notes).unwrap();
        assert_eq!(text, "x".repeat(100));
        assert!(runs.iter().all(|run| run.style.italic));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].len() < 340);
        assert!(notes[0].contains('…'));
        assert!(notes[0].ends_with(" is retained without native formatting"));
    }
}

#[test]
fn ordinary_inherited_diagnostics_and_supported_caps_are_unchanged() {
    let markup = "<markup><unknown variant='petite-caps' text_transform='capitalize' custom='v'><b>A</b><i>B</i></unknown><span variant='small-caps'>C</span><span variant='normal' text_transform='uppercase'>D</span></markup>";
    let mut notes = Vec::new();
    let (text, runs) = markup_runs(markup, &CharStyle::default(), &HashMap::new(), 72., &mut notes).unwrap();
    assert_eq!(text, "ABCD");
    assert_eq!(
        notes,
        [
            "Source markup element unknown is retained without native formatting",
            "Source font variant petite-caps remains inert",
            "Source text transform capitalize remains inert",
            "Source markup attribute custom is retained without native formatting",
        ]
    );
    assert_eq!(runs[2].style.caps, Caps::SmallCaps);
    assert_eq!(runs[3].style.caps, Caps::AllCaps);
}
