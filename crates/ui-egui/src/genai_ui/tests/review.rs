//! The shell after review (#10903): placement notes, the count and cost before starting, consent
//! and setup fixes, and Save Fill Diagnostics.

use super::*;

#[test]
fn skipped_colour_matching_and_another_framing_are_said() {
    let note = |colour, framing| VariationNote { colour, framing };
    let plain = note(ColourMatch::Matched { drift: 0.01, changed: 0.0, shadows: 0.0, midtones: 0.0, highlights: 0.0 }, Framing::Same);
    assert!(note_lines(&plain).is_empty(), "nothing to say about a plain one");
    let skipped = note_lines(&note(ColourMatch::TooDifferent { drift: 0.4, limit: 0.25 }, Framing::Centre { returned: [1536, 1024], sent: [1024, 1024] }));
    assert_eq!(skipped.len(), 2);
    assert!(skipped[0].contains("Colour matching was skipped"), "{skipped:?}");
    assert!(skipped[1].contains("1536×1024") && skipped[1].contains("1024×1024") && skipped[1].contains("centre"), "{skipped:?}");
    assert!(note_lines(&note(ColourMatch::TooLittleOutside, Framing::Same))[0].contains("too little"));
    // The status line after a fill lands counts the skipped ones.
    let matching = json!([{"colour": {"state": "matched"}}, {"colour": {"state": "tooDifferent"}}, {"colour": {"state": "tooLittleOutside"}}]);
    assert_eq!(skipped_note(Some(&matching)).as_deref(), Some("Colour matching was skipped for 2 variations."));
    assert_eq!(skipped_note(Some(&json!([{"colour": {"state": "matched"}}]))), None);
    assert_eq!(skipped_note(None), None);
}
