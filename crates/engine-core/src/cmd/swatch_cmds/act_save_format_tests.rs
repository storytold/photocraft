//! #14375: Color Table Save… writes `.act` files as Photoshop does. The 256 RGB triplets are
//! always there (unused entries black); the 4-byte trailer (big-endian colour count, big-endian
//! transparent index, `FFFF` for none) follows only for a table shorter than 256 colours or one
//! with a transparent index. Byte layouts below are the ones measured in Photoshop.

use super::*;

/// `n` distinct, non-black colours, so padding and the trailer can't be mistaken for them.
fn table(n: usize) -> Vec<[u8; 3]> {
    (0..n).map(|i| [i as u8, 255 - i as u8, 0x80 | (i as u8 & 0x7f)]).collect()
}

/// Asserts that `bytes` starts with `colors` as RGB triplets and pads the rest of the 768-byte
/// palette with black.
fn assert_palette(bytes: &[u8], colors: &[[u8; 3]]) {
    assert!(bytes.len() >= ACT_COLORS * 3, "a palette is 768 bytes, got {}", bytes.len());
    let (used, unused) = bytes[..ACT_COLORS * 3].split_at(colors.len() * 3);
    assert_eq!(used, colors.concat(), "the table's colours, in order");
    assert!(unused.iter().all(|&b| b == 0), "unused entries are 0,0,0");
}

#[test]
fn a_full_table_without_transparency_is_the_bare_768_byte_palette() {
    let colors = table(256);
    let bytes = act_bytes(&colors, None).unwrap();
    assert_eq!(bytes.len(), 768, "no trailer");
    assert_palette(&bytes, &colors);
    // It loads back as the whole table with no transparent colour.
    let (hex, transparent) = act_table(&bytes).unwrap();
    assert_eq!(hex.len(), 256);
    assert_eq!(hex[255], "#ff00ff");
    assert_eq!(transparent, None);
}

#[test]
fn a_short_table_keeps_its_count_in_the_trailer() {
    let colors = table(6);
    let bytes = act_bytes(&colors, None).unwrap();
    assert_eq!(bytes.len(), 772);
    assert_palette(&bytes, &colors);
    assert_eq!(&bytes[768..], &[0x00, 0x06, 0xFF, 0xFF], "6 colours, no transparent index");
    let (hex, transparent) = act_table(&bytes).unwrap();
    assert_eq!((hex.len(), transparent), (6, None));
}

#[test]
fn a_transparent_index_keeps_the_trailer() {
    let colors = table(3);
    let bytes = act_bytes(&colors, Some(2)).unwrap();
    assert_eq!(bytes.len(), 772);
    assert_palette(&bytes, &colors);
    assert_eq!(&bytes[768..], &[0x00, 0x03, 0x00, 0x02]);
    assert_eq!(act_table(&bytes).unwrap(), (vec!["#00ff80".to_string(), "#01fe81".into(), "#02fd82".into()], Some(2)));

    // Not measured, by the same rule: a full table with a transparent index keeps the trailer,
    // count 256.
    let colors = table(256);
    let bytes = act_bytes(&colors, Some(5)).unwrap();
    assert_eq!(bytes.len(), 772);
    assert_palette(&bytes, &colors);
    assert_eq!(&bytes[768..], &[0x01, 0x00, 0x00, 0x05]);
    let (hex, transparent) = act_table(&bytes).unwrap();
    assert_eq!((hex.len(), transparent), (256, Some(5)));
}

#[test]
fn one_colour_short_of_full_still_has_the_trailer() {
    let colors = table(255);
    let bytes = act_bytes(&colors, None).unwrap();
    assert_eq!(bytes.len(), 772);
    assert_palette(&bytes, &colors);
    assert_eq!(&bytes[768..], &[0x00, 0xFF, 0xFF, 0xFF]);
}
