//! Color Table › Load… with an `.act` trailer count of 0 (#14374): one white entry, as Photoshop
//! loads it, instead of an empty table that OK can't apply.
use super::*;

/// The measured test file: 256 triplets with entry 0 = (10, 20, 30), the rest black, then the
/// trailer (`count`, transparent 0xFFFF) unless `count` is `None`.
fn act(count: Option<u16>) -> Vec<u8> {
    let mut b = vec![0u8; ACT_COLORS * 3];
    b[..3].copy_from_slice(&[10, 20, 30]);
    if let Some(n) = count {
        b.extend_from_slice(&n.to_be_bytes());
        b.extend_from_slice(&0xFFFFu16.to_be_bytes());
    }
    b
}

#[test]
fn count_zero_loads_one_white_entry() {
    let bytes = act(Some(0));
    assert_eq!(bytes.len(), 772);
    let (colors, transparent) = act_table(&bytes).unwrap();
    assert_eq!(colors, vec!["#ffffff".to_string()], "one white entry, not the file's first colour");
    assert_eq!(transparent, None);
    // Not measured: a count-0 file that also names a transparent index keeps today's handling
    // (the index is outside the file's zero used colours, so there is no transparent entry).
    let mut bytes = act(Some(0));
    bytes[770..772].copy_from_slice(&0u16.to_be_bytes());
    assert_eq!(act_table(&bytes).unwrap().1, None);
}

#[test]
fn the_loaded_table_applies_and_saves_with_count_one() {
    let (colors, transparent) = act_table(&act(Some(0))).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": "uniform", "colors": 8, "dither": "none", "forced": "none"})).unwrap();
    // What the dialog's OK sends after Load….
    s.execute("image.mode.colorTable", json!({"table": "custom", "colors": colors, "transparent": transparent})).unwrap();
    let table = s.active().unwrap().doc.color_table.clone().unwrap();
    assert_eq!(table.colors, vec![[255u8, 255, 255]]);
    // Save… writes the table with count 1 in the trailer.
    let saved = act_bytes(&table.colors, None).unwrap();
    assert_eq!(saved.len(), 772);
    assert_eq!(&saved[..3], &[255, 255, 255]);
    assert_eq!(&saved[768..770], &1u16.to_be_bytes());
}

#[test]
fn other_counts_and_a_missing_trailer_load_as_before() {
    for (count, len) in [(Some(1), 1), (Some(40), 40), (Some(256), 256), (Some(300), 256), (None, 256)] {
        let (colors, transparent) = act_table(&act(count)).unwrap();
        assert_eq!(colors.len(), len, "count {count:?}");
        assert_eq!(colors[0], "#0a141e", "count {count:?}");
        assert!(colors[1..].iter().all(|c| c == "#000000"), "count {count:?}");
        assert_eq!(transparent, None, "count {count:?}");
    }
    assert!(act_table(&[0u8; 10]).is_err(), "junk bytes still fail");
}
